//! Tests of `place_search`: the generation pool, selection, the launch rule, the
//! routing phase (with fake `RouteCall`s on a paused clock), the references, rerun and
//! the table. Nothing here starts fastroute or KiCad.

use super::*;
use crate::place_generators::{ForceParams, Params, PerturbParams, SchematicParams};
use crate::place_held::tests::{board, board_with_groups, pad, parse, Fp, Shape};
use crate::place_route::export_options;
use crate::route_judge::{Gates, Measures};
use crate::tools::pcb_components::footprint_moved_in;
use konnect_ipc::types::IpcFootprintPlacement;

const OUTLINE: Shape = Shape::Rect(0.0, 0.0, 60.0, 40.0);

// ─── Fixtures ────────────────────────────────────────────────────────────────

/// A roomy board: U1, four capacitors, three resistors, a held connector and mounting
/// hole, and two pad-less logos that share one reference.
fn parts() -> Vec<Fp> {
    let mut parts = vec![
        Fp::new("U1", 30.0, 20.0)
            .lib("Test:U")
            .court(3.0, 3.0)
            .pad(pad("1", -2.0, -1.0, Some("VCC")))
            .pad(pad("2", -2.0, 1.0, Some("GND")))
            .pad(pad("3", 2.0, 0.0, Some("SIG0"))),
        Fp::new("J1", 58.0, 20.0)
            .lib("Test:J")
            .court(1.5, 3.0)
            .pad(pad("1", 0.0, -1.0, Some("SIG0")))
            .pad(pad("2", 0.0, 1.0, Some("GND"))),
        Fp::new("H1", 4.0, 4.0)
            .lib("Test:H")
            .court(2.0, 2.0)
            .pad(pad("1", 0.0, 0.0, Some("GND"))),
        Fp::new("LOGO", 50.0, 6.0).lib("Test:Logo").court(2.0, 2.0),
        Fp::new("LOGO", 50.0, 34.0).lib("Test:Logo").court(2.0, 2.0),
    ];
    for (k, (x, y)) in [(24.0, 14.0), (24.0, 26.0), (36.0, 14.0), (36.0, 26.0)]
        .into_iter()
        .enumerate()
    {
        parts.push(
            Fp::new(&format!("C{}", k + 1), x, y)
                .lib("Test:C")
                .value("100nF")
                .two_pad("VCC", "GND"),
        );
    }
    for k in 0..3 {
        parts.push(
            Fp::new(&format!("R{}", k + 1), 10.0, 10.0 + 6.0 * k as f64)
                .lib("Test:R")
                .two_pad(&format!("SIG{k}"), &format!("SIG{}", k + 1)),
        );
    }
    parts
}

fn text() -> String {
    board(OUTLINE, &parts())
}

/// The model of `text`, built as `search` builds it.
fn model_of(text: &str) -> BoardModel {
    let set = held_set(text, &BTreeSet::new(), &HoldRules::default()).expect("the held set");
    let tree = parse(text);
    let edge = Allowlist::new(real_held_references(&set));
    let evaluated = evaluate(&tree, &edge, MAX_NET_REFERENCES).expect("evaluates");
    let pairs = decoupling_pairs(&evaluated).unwrap();
    let intent = Intent::new(groups(&tree, &set, &[]), pairs);
    board_model(text, set, intent).expect("the model")
}

fn model() -> BoardModel {
    model_of(&text())
}

fn force_params() -> ForceParams {
    ForceParams {
        rail_weight: 0.5,
        fanout_limit: 8,
        pair_weight: 10.0,
        group_weight: 0.5,
        iterations: 80,
    }
}

/// 8 + 8 + 8 recipes.
fn small_matrix() -> Matrix {
    Matrix {
        force_seeds: (1..=8).collect(),
        force: vec![force_params()],
        schematic_seeds: (1..=8).collect(),
        schematic: vec![SchematicParams {
            fanout_limit: 8,
            polish: None,
        }],
        perturb_seeds: (1..=8).collect(),
        perturb: vec![PerturbParams {
            moves: 4,
            radius_mm: 3.0,
            swaps: true,
            turns: true,
        }],
    }
}

fn perturb_recipe(seed: u64) -> Recipe {
    Recipe {
        family: Family::Perturb,
        seed,
        params: Params::Perturb(PerturbParams {
            moves: 1,
            radius_mm: 1.0,
            swaps: false,
            turns: false,
        }),
    }
}

fn placement(reference: &str, x: f64, y: f64, rotation: f64) -> IpcFootprintPlacement {
    IpcFootprintPlacement {
        reference: reference.to_string(),
        x,
        y,
        rotation,
    }
}

fn candidate(placements: Vec<IpcFootprintPlacement>) -> Result<Candidate, GenerateFailure> {
    Ok(Candidate {
        placements,
        converged: None,
        iterations_run: None,
    })
}

fn measures(unconnected: usize, signal_mm: f64) -> Measures {
    Measures {
        unconnected,
        violations: BTreeMap::new(),
        routed_share: Some(1.0),
        audit_complete: Some(true),
        acute: Some(0.0),
        right_angles: Some(0.0),
        copper_quality: Some(90.0),
        nets_undersized: None,
        narrow_paths: None,
        signal_mm,
        power_mm: 0.0,
        pour_track_mm: 0.0,
        vias_off_pour: 0,
        below_floor: 0,
        width_rows: Vec::new(),
        longest_below_mm: BTreeMap::new(),
        stitching_vias: 0,
        route_s: 10.0,
        lengths: None,
    }
}

fn routed(unconnected: usize, signal_mm: f64) -> RouteResult {
    RouteResult::Routed {
        gates: Gates {
            g1: Vec::new(),
            g2: Vec::new(),
            g3: Vec::new(),
            g4: Vec::new(),
        },
        measures: measures(unconnected, signal_mm),
        unrouted: BTreeMap::new(),
        status: "ok".into(),
        call: json!({"engine": {"name": "fake"}, "effort": "m"}),
    }
}

/// What the fake router does on its n-th call (from 0).
#[derive(Clone)]
enum Step {
    Routed {
        unconnected: usize,
        signal_mm: f64,
    },
    NoWinner,
    Export,
    Call,
    /// Routed, with a G1 line that names a net and an unrouted check that has a
    /// `clearance` entry.
    Diagnosed,
}

struct Shared {
    steps: Box<dyn Fn(usize, usize) -> Step + Send + Sync>,
    /// How long the n-th call lasts, on tokio's clock.
    wall: Box<dyn Fn(usize) -> Duration + Send + Sync>,
    calls: AtomicUsize,
    active: AtomicUsize,
    max_active: AtomicUsize,
    /// `(id, start)` of every call, on tokio's clock.
    starts: Mutex<Vec<(usize, Instant)>>,
    unrouted: Result<BTreeMap<String, usize>, String>,
}

struct Fake(Arc<Shared>);

fn fake(wall_s: u64, steps: impl Fn(usize, usize) -> Step + Send + Sync + 'static) -> Fake {
    fake_walls(move |_| wall_s, steps)
}

/// A fake router whose n-th call lasts `wall_s(n)` seconds.
fn fake_walls(
    wall_s: impl Fn(usize) -> u64 + Send + Sync + 'static,
    steps: impl Fn(usize, usize) -> Step + Send + Sync + 'static,
) -> Fake {
    Fake(Arc::new(Shared {
        steps: Box::new(steps),
        wall: Box::new(move |n| Duration::from_secs(wall_s(n))),
        calls: AtomicUsize::new(0),
        active: AtomicUsize::new(0),
        max_active: AtomicUsize::new(0),
        starts: Mutex::new(Vec::new()),
        unrouted: Ok(BTreeMap::new()),
    }))
}

impl RouteCall for Fake {
    fn route(
        &self,
        id: usize,
        _board_text: String,
        _folder: PathBuf,
    ) -> BoxFuture<(RouteResult, Timings)> {
        let shared = Arc::clone(&self.0);
        Box::pin(async move {
            let n = shared.calls.fetch_add(1, Ordering::SeqCst);
            shared.starts.lock().unwrap().push((id, Instant::now()));
            let now = shared.active.fetch_add(1, Ordering::SeqCst) + 1;
            shared.max_active.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep((shared.wall)(n)).await;
            shared.active.fetch_sub(1, Ordering::SeqCst);
            let result = match (shared.steps)(n, id) {
                Step::Routed {
                    unconnected,
                    signal_mm,
                } => routed(unconnected, signal_mm),
                Step::Diagnosed => RouteResult::Routed {
                    gates: Gates {
                        g1: vec!["net SECRETNET is not fully routed".to_string()],
                        g2: Vec::new(),
                        g3: Vec::new(),
                        g4: Vec::new(),
                    },
                    measures: measures(0, 100.0),
                    unrouted: BTreeMap::from([("clearance".to_string(), 3)]),
                    status: "ok".into(),
                    call: json!({"engine": {"name": "fake"}, "effort": "m"}),
                },
                Step::NoWinner => RouteResult::NoWinner {
                    unrouted: Some(BTreeMap::new()),
                    reason: "no run was routed and judged".into(),
                },
                Step::Export => RouteResult::Failed {
                    stage: "export",
                    reason: "the export refused".into(),
                },
                Step::Call => RouteResult::Failed {
                    stage: "call",
                    reason: "the call erred".into(),
                },
            };
            (result, Timings::default())
        })
    }

    fn unrouted(
        &self,
        _board_text: String,
        _folder: PathBuf,
    ) -> BoxFuture<anyhow::Result<BTreeMap<String, usize>>> {
        let answer = self.0.unrouted.clone();
        Box::pin(async move { answer.map_err(|why| anyhow::anyhow!(why)) })
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    root: PathBuf,
    plan: Plan,
}

fn setup(time_limit_s: u64) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let input = root.join("input");
    std::fs::create_dir_all(&input).unwrap();
    let board_path = input.join("board.kicad_pcb");
    std::fs::write(&board_path, text()).unwrap();
    let plan = Plan {
        name: "T9".into(),
        folder: root.join("run"),
        matrix: small_matrix(),
        threads: 2,
        reference_calls: 3,
        best_per_family: 3,
        drawn_per_family: 1,
        draw_seed: 1,
        time_limit: Duration::from_secs(time_limit_s),
        route: RouteInputs {
            rules: BTreeMap::new(),
            options: export_options(&board_path),
            effort: Effort::M,
            fastroute: root.join("no-fastroute"),
            kicad_cli: "no-kicad-cli".into(),
            kicad_config_home: root.join("cfg"),
            board: board_path,
        },
        ai_board: None,
        caller_held: BTreeSet::new(),
        hold: HoldRules::default(),
    };
    std::fs::create_dir_all(&plan.folder).unwrap();
    Setup {
        _dir: dir,
        root,
        plan,
    }
}

/// Records that were selected for routing, whatever became of them.
fn selected(outcome: &Outcome) -> usize {
    outcome
        .records
        .iter()
        .filter(|r| {
            [
                "routed",
                "no_winner",
                "export_refused",
                "route_failed",
                "not_routed_time_limit",
                "not_routed_stopped",
            ]
            .contains(&r.outcome)
        })
        .count()
}

fn count(outcome: &Outcome, name: &str) -> usize {
    outcome.records.iter().filter(|r| r.outcome == name).count()
}

fn good(_n: usize, _id: usize) -> Step {
    Step::Routed {
        unconnected: 0,
        signal_mm: 100.0,
    }
}

/// A bare record for the selection tests.
fn rec(id: usize, origin: Origin, outcome: &'static str, key: Option<ScreenKey>) -> Record {
    Record {
        id,
        origin,
        board_sha256: None,
        input_sha256: "i".into(),
        held_sha256: "h".into(),
        intent_sha256: "t".into(),
        outcome,
        reason: None,
        converged: None,
        screen: None,
        screen_key: key,
        key: None,
        rank: None,
        timings: Timings::default(),
        route: None,
        placement_gates: None,
        unrouted: None,
        route_gates: None,
        schematic_neighbours: None,
        schematic_neighbours_reason: None,
        board_text: None,
        judged: None,
    }
}

fn generated_origin(family: Family, seed: u64) -> Origin {
    Origin::Generated {
        recipe: Recipe {
            family,
            seed,
            params: Params::Perturb(PerturbParams {
                moves: 1,
                radius_mm: 1.0,
                swaps: false,
                turns: false,
            }),
        },
        generator_version: GENERATOR_VERSION,
    }
}

fn key_with_crossings(crossings: i64) -> ScreenKey {
    [0, 0, 0, 0, crossings, 0]
}

// ─── Phase 1 ─────────────────────────────────────────────────────────────────

#[test]
fn a_panicking_generator_is_a_record_not_the_end() {
    let model = model();
    let recipes = recipes(&small_matrix());
    let boom = |model: &BoardModel, recipe: &Recipe| {
        if recipe.family == Family::Force && recipe.seed == 2 {
            panic!("boom in seed two");
        }
        generate(model, recipe)
    };
    let records = generate_all_with(&model, &recipes, 3, 7, &boom);
    assert_eq!(records.len(), recipes.len());
    let failed: Vec<&Record> = records
        .iter()
        .filter(|r| r.outcome == "generate_failed")
        .collect();
    assert_eq!(failed.len(), 1, "only the panicking recipe fails");
    assert!(failed[0]
        .reason
        .as_deref()
        .unwrap()
        .contains("boom in seed two"));
    assert_eq!(failed[0].id, 7 + 1, "ids are first_id + slot");
    assert!(
        records
            .iter()
            .filter(|r| r.outcome != "generate_failed")
            .all(|r| r.screen.is_some() || r.outcome == "duplicate"),
        "the others are generated"
    );
}

#[test]
fn the_pool_never_runs_more_jobs_than_threads() {
    let model = model();
    let recipes: Vec<Recipe> = (1..=24).map(perturb_recipe).collect();
    let active = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    let slow = |model: &BoardModel, recipe: &Recipe| {
        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
        peak.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(15));
        active.fetch_sub(1, Ordering::SeqCst);
        generate(model, recipe)
    };
    let records = generate_all_with(&model, &recipes, 3, 0, &slow);
    assert_eq!(records.len(), 24);
    let peak = peak.load(Ordering::SeqCst);
    assert!(peak <= 3, "at most 3 jobs at once, saw {peak}");
    assert!(peak >= 2, "the pool does run jobs side by side, saw {peak}");
}

#[test]
fn the_records_do_not_depend_on_the_thread_count() {
    let model = model();
    let recipes = recipes(&small_matrix());
    let strip = |records: Vec<Record>| -> Vec<serde_json::Value> {
        records
            .into_iter()
            .map(|r| {
                let mut value = serde_json::to_value(&r).unwrap();
                value["timings"] = serde_json::Value::Null;
                value
            })
            .collect()
    };
    let one = strip(generate_all(&model, &recipes, 1, 4));
    let eight = strip(generate_all(&model, &recipes, 8, 4));
    assert_eq!(one, eight);
}

#[test]
fn a_candidate_that_moves_a_held_part_is_a_held_moved_record() {
    let model = model();
    let moves_connector =
        |_: &BoardModel, _: &Recipe| candidate(vec![placement("J1", 50.0, 20.0, 0.0)]);
    let records = generate_all_with(&model, &[perturb_recipe(1)], 1, 0, &moves_connector);
    assert_eq!(records[0].outcome, "held_moved");
    assert!(records[0].reason.as_deref().unwrap().contains("J1"));
    assert!(records[0].board_sha256.is_some());
}

#[test]
fn a_candidate_that_does_not_apply_is_a_write_failed_record() {
    let model = model();
    let unknown = |_: &BoardModel, _: &Recipe| candidate(vec![placement("Z9", 1.0, 1.0, 0.0)]);
    let records = generate_all_with(&model, &[perturb_recipe(1)], 1, 0, &unknown);
    assert_eq!(records[0].outcome, "write_failed");
    assert!(records[0].board_sha256.is_none());
}

#[test]
fn a_repeated_board_is_a_duplicate_and_never_routed() {
    let model = model();
    let by_seed = |_: &BoardModel, recipe: &Recipe| match recipe.seed {
        1 | 2 => candidate(vec![placement("R1", 12.0, 12.0, 0.0)]),
        3 => candidate(vec![placement("R1", 12.0, 12.001, 0.0)]),
        _ => candidate(Vec::new()),
    };
    let recipes: Vec<Recipe> = (1..=4).map(perturb_recipe).collect();
    let mut records = generate_all_with(&model, &recipes, 2, 10, &by_seed);
    assert_eq!(records[0].outcome, "generated");
    assert_eq!(records[1].outcome, "duplicate");
    assert_eq!(
        records[1].reason.as_deref(),
        Some("same board as candidate 10")
    );
    assert_eq!(
        records[2].outcome, "generated",
        "a board 1 um away is a different board"
    );
    assert_eq!(records[3].outcome, "duplicate");
    assert_eq!(
        records[3].reason.as_deref(),
        Some("same board as the input board")
    );
    // A's board is compared too, by `search`, through the same function.
    let a_sha = records[2].board_sha256.clone().unwrap();
    for record in &mut records {
        if record.outcome == "duplicate" {
            continue;
        }
        record.outcome = "generated";
    }
    mark_duplicates(&mut records, &[(Some(3), a_sha)]);
    assert_eq!(records[2].outcome, "duplicate");
    assert_eq!(
        records[2].reason.as_deref(),
        Some("same board as candidate 3")
    );
    // No duplicate is ever selected.
    let order = routing_order(&records, 3, 1, 1);
    for record in &records {
        if record.outcome == "duplicate" {
            assert!(!order.contains(&record.id));
        }
    }
}

#[test]
fn a_perturbation_that_ends_where_it_began_is_a_duplicate_of_the_input() {
    // One movable part, at 180 degrees with a text at 100 degrees (a set would re-spell
    // it). Two edits that undo each other (two half turns) leave it where it was: the
    // candidate lists nothing, and its board is the input board byte for byte.
    let all = vec![
        Fp::new("J1", 58.0, 20.0)
            .court(1.5, 3.0)
            .pad(pad("1", 0.0, 0.0, Some("SIG0"))),
        Fp::new("R1", 20.0, 20.0).two_pad("SIG0", "SIG1").rot(180.0),
    ];
    let input = board(OUTLINE, &all).replacen(
        "(property \"Reference\" \"R1\" (at 0 -1.5 0)",
        "(property \"Reference\" \"R1\" (at 0 -1.5 100)",
        1,
    );
    let model = model_of(&input);
    let recipe = |seed: u64| Recipe {
        family: Family::Perturb,
        seed,
        params: Params::Perturb(PerturbParams {
            moves: 2,
            radius_mm: 1.0,
            swaps: false,
            turns: true,
        }),
    };
    let unchanged: Vec<Recipe> = (1..=300)
        .map(recipe)
        .filter(|r| generate(&model, r).is_ok_and(|c| c.placements.is_empty()))
        .collect();
    assert!(!unchanged.is_empty(), "no seed of 300 undoes itself");
    let records = generate_all(&model, &unchanged, 2, 0);
    for record in &records {
        assert_eq!(record.outcome, "duplicate", "{record:?}");
        assert_eq!(
            record.reason.as_deref(),
            Some("same board as the input board")
        );
        assert_eq!(
            record.board_sha256.as_deref(),
            Some(model.held.board_sha256.as_str())
        );
    }
    // Where it did move, the board differs from the input and is kept.
    let moved_recipe = (1..=300)
        .map(recipe)
        .find(|r| generate(&model, r).is_ok_and(|c| !c.placements.is_empty()))
        .expect("some seed moves the part");
    let moved = generate_all(&model, &[moved_recipe], 1, 0);
    assert_eq!(moved[0].outcome, "generated");
}

#[test]
fn the_edge_mount_list_holds_real_references_only() {
    let model = model();
    assert!(
        model.held.held.keys().any(|key| key.contains('#')),
        "the fixture's logos are held under shared keys"
    );
    let real = real_held_references(&model.held);
    assert_eq!(real, vec!["H1".to_string(), "J1".to_string()]);
}

#[test]
fn a_reference_with_a_hash_in_it_is_still_a_real_reference() {
    // Only `<reference>#<digits>` is a shared key; a real reference that happens to hold a
    // `#` (`TP#a`, `TP#`) names a footprint and is an edge mount like any other.
    let mut set = model().held;
    let part = set.held["J1"].clone();
    for key in ["TP#a", "TP#", "TP#1x"] {
        set.held.insert(key.to_string(), part.clone());
    }
    let real = real_held_references(&set);
    for key in ["TP#a", "TP#", "TP#1x", "J1", "H1"] {
        assert!(real.contains(&key.to_string()), "{key} in {real:?}");
    }
    assert!(!real.iter().any(|key| key.starts_with("LOGO#")), "{real:?}");
}

#[test]
fn the_group_spread_follows_the_groups() {
    let text = board_with_groups(OUTLINE, &parts(), &[("power", &["C1", "C2", "C3"])]);
    let grouped = model_of(&text);
    let spread = group_spread(&grouped, &parse(&text)).expect("a group of 3");
    assert!(spread > 0.0 && spread < 1.0, "{spread}");
    let none = model();
    assert_eq!(group_spread(&none, &parse(&none.text)), None);
}

// ─── Phase 2 ─────────────────────────────────────────────────────────────────

/// O: 0..3, A: 3..6, F: 10..16, S: 20..26, P: 30..36.
fn selection_records() -> Vec<Record> {
    let mut records = Vec::new();
    for call in 1..=3 {
        records.push(rec(
            call - 1,
            Origin::Owner { call },
            "generated",
            Some([0; 6]),
        ));
    }
    for call in 1..=3 {
        records.push(rec(
            2 + call,
            Origin::Ai {
                board: PathBuf::from("ai.kicad_pcb"),
                call,
            },
            "generated",
            Some([0; 6]),
        ));
    }
    // Crossings 5, 1, 3, 0, 2, 4: the best three are the 4th, 2nd and 5th.
    for (base, family) in [
        (10, Family::Force),
        (20, Family::Schematic),
        (30, Family::Perturb),
    ] {
        for (k, crossings) in [5, 1, 3, 0, 2, 4].into_iter().enumerate() {
            records.push(rec(
                base + k,
                generated_origin(family, k as u64),
                "generated",
                Some(key_with_crossings(crossings)),
            ));
        }
    }
    records
}

#[test]
fn routing_order_takes_the_references_then_the_families_interleaved() {
    let records = selection_records();
    let order = routing_order(&records, 3, 1, 7);
    // The rest (not among the best) per family, in id order: base+0, +2, +5.
    let mut rng = SplitMix64(7);
    let mut drawn = Vec::new();
    for base in [10usize, 20, 30] {
        let rest = [base, base + 2, base + 5];
        drawn.push(rest[rng.below(3) as usize]);
    }
    // Round r starts at family r mod 3: F, S, P; then S, P, F; then P, F, S; then F, S, P.
    let expected = vec![
        0, 1, 2, 3, 4, 5, // O three times, A three times
        13, 23, 33, // each family's best
        drawn[1], drawn[2], drawn[0], // each family's drawn
        31, 11, 21, // second best
        14, 24, 34, // third best
    ];
    assert_eq!(order, expected);
}

#[test]
fn the_round_robin_starts_each_round_at_another_family() {
    // One best and one drawn per family: two rounds. Without the rotation P's drawn
    // candidate would always be the last call, and the first one a time limit cuts.
    let records = selection_records();
    let order = routing_order(&records, 1, 1, 7);
    let routed: Vec<usize> = order.into_iter().skip(6).collect();
    assert_eq!(routed.len(), 6);
    assert_eq!(routed[..3], [13, 23, 33], "round 0 is F, S, P");
    let family_of = |id: usize| id / 10;
    assert_eq!(
        routed[3..]
            .iter()
            .map(|&id| family_of(id))
            .collect::<Vec<_>>(),
        vec![2, 3, 1],
        "round 1 is S, P, F"
    );
    assert_eq!(
        family_of(*routed.last().unwrap()),
        1,
        "F's drawn candidate is last"
    );
}

#[test]
fn routing_order_never_selects_a_candidate_that_has_no_screen() {
    let mut records = selection_records();
    // The best of F is a duplicate, the second best failed before its screen.
    records[6 + 3].outcome = "duplicate";
    records[6 + 1].outcome = "generate_failed";
    records[6 + 1].screen_key = None;
    records[6 + 4].outcome = "screened_out";
    let order = routing_order(&records, 3, 1, 7);
    for id in [13, 11, 14] {
        assert!(!order.contains(&id), "{id} must not be selected: {order:?}");
    }
    // O and A that did not get through their screen are not called either.
    records[0].outcome = "screen_failed";
    records[3].outcome = "ai_refused";
    let order = routing_order(&records, 3, 1, 7);
    assert!(!order.contains(&0) && !order.contains(&3));
}

#[test]
fn screen_ties_go_to_the_lower_id() {
    // Six candidates of one family with the same key: the best two are the two lowest ids.
    let records: Vec<Record> = (0..6)
        .rev()
        .map(|k| {
            rec(
                20 + k,
                generated_origin(Family::Force, k as u64),
                "generated",
                Some(key_with_crossings(1)),
            )
        })
        .collect();
    let order = routing_order(&records, 2, 0, 1);
    assert_eq!(order, vec![20, 21]);
}

#[test]
fn the_draw_is_fixed_and_never_draws_a_p1_failure() {
    let mut records = selection_records();
    let first = routing_order(&records, 3, 1, 7);
    assert_eq!(
        first,
        routing_order(&records, 3, 1, 7),
        "the same seed, the same draw"
    );
    // Make every candidate outside F's best three fail P1: nothing may be drawn there.
    for k in [0usize, 2, 5] {
        records[6 + k].screen_key = Some([1, 0, 0, 0, 9, 0]);
    }
    let order = routing_order(&records, 3, 1, 7);
    for id in [10, 12, 15] {
        assert!(
            !order.contains(&id),
            "{id} fails P1 and is not drawn: {order:?}"
        );
    }
    // The best three of F are routed as before.
    for id in [13, 11, 14] {
        assert!(order.contains(&id));
    }
    // A family with fewer candidates than slots gives what it has: of F's ids 10 and 11,
    // 10 fails P1 here, so only 11 is routed.
    let few: Vec<Record> = records.into_iter().filter(|r| r.id < 12).collect();
    let order = routing_order(&few, 3, 1, 7);
    assert_eq!(order.iter().filter(|id| **id >= 10).count(), 1);
    assert!(order.contains(&11));
}

#[test]
fn a_candidate_that_fails_p1_is_never_the_best_of_its_family() {
    // F's best by the screen key is a P1 failure: its key's first slot is 1, but the
    // other slots are the lowest of the family, so a sort that ignored P1 would take it.
    let mut records = selection_records();
    records[6 + 3].screen_key = Some([1, 0, 0, 0, 0, 0]);
    records[6].screen_key = Some([1, 0, 0, 0, 0, 0]);
    let order = routing_order(&records, 3, 1, 7);
    assert!(
        !order.contains(&13) && !order.contains(&10),
        "a P1 failure is not routed: {order:?}"
    );
    // The best three are the best three that pass P1: crossings 1, 2, 3 are ids 11, 14, 12;
    // the rest of the pool that passes P1 is crossings 4: id 15, which the draw takes.
    let f: Vec<usize> = order
        .iter()
        .copied()
        .filter(|id| (10..16).contains(id))
        .collect();
    assert_eq!(f.len(), 4, "{f:?}");
    for id in [11, 14, 12, 15] {
        assert!(f.contains(&id), "{id} in {f:?}");
    }
    // Where every candidate of a family fails P1, the family gets no call at all.
    for k in 0..6 {
        records[6 + k].screen_key = Some([1, 0, 0, 0, 0, 0]);
    }
    let order = routing_order(&records, 3, 1, 7);
    assert!(order.iter().all(|id| !(10..16).contains(id)), "{order:?}");
}

// ─── The launch rule ─────────────────────────────────────────────────────────

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[test]
fn the_first_estimate_is_the_budget_plus_a_minute() {
    assert_eq!(first_call_estimate(Effort::M), secs(300));
    assert_eq!(first_call_estimate(Effort::H), secs(780));
    assert_eq!(first_call_estimate(Effort::Xh), secs(2460));
    assert_eq!(first_call_estimate(Effort::Fast), HANG_CAP + secs(60));
}

#[test]
fn no_call_starts_that_cannot_end_by_the_limit_table() {
    let m = Effort::M;
    // Before any call: the estimate (300 s).
    assert!(may_route(secs(0), secs(300), &[], m));
    assert!(!may_route(secs(1), secs(300), &[], m));
    // After calls: their median (the lower middle of an even count).
    assert!(may_route(secs(900), secs(1000), &[secs(100)], m));
    assert!(!may_route(secs(901), secs(1000), &[secs(100)], m));
    assert!(may_route(
        secs(800),
        secs(1000),
        &[secs(100), secs(900), secs(200)],
        m
    ));
    assert!(may_route(secs(900), secs(1000), &[secs(100), secs(900)], m));
    assert!(!may_route(
        secs(901),
        secs(1000),
        &[secs(100), secs(900)],
        m
    ));
}

#[tokio::test(start_paused = true)]
async fn no_call_starts_that_cannot_end_by_the_limit() {
    let setup = setup(1000);
    let router = Arc::new(fake(100, good));
    let shared = Arc::clone(&router.0);
    let outcome = search(&setup.plan, Arc::new(Fake(Arc::clone(&shared))))
        .await
        .unwrap();
    // O's 3 calls and up to 4 per family are selected; calls last 100 s each. The first
    // starts at 0 (estimate 300 <= 1000), then the median is 100: starts at 0..=900.
    assert!(selected(&outcome) > 10, "the fixture selects more than fit");
    assert_eq!(count(&outcome, "routed"), 10);
    assert_eq!(
        count(&outcome, "not_routed_time_limit"),
        selected(&outcome) - 10
    );
    assert!(outcome.stopped_routing.is_some());
    assert!(outcome.elapsed <= secs(1000), "{:?}", outcome.elapsed);
    // One call at a time.
    assert_eq!(shared.max_active.load(Ordering::SeqCst), 1);
    // Every call started where the rule allows it.
    let starts = shared.starts.lock().unwrap();
    let t0 = starts[0].1;
    for (k, (_, at)) in starts.iter().enumerate() {
        assert_eq!(at.duration_since(t0), secs(100 * k as u64));
    }
}

#[tokio::test(start_paused = true)]
async fn a_started_call_is_never_cut_by_the_limit() {
    // The first call runs 2000 s against a 1000 s limit: it is let finish, nothing
    // else starts.
    let setup = setup(1000);
    let router = fake(2000, good);
    let shared = Arc::clone(&router.0);
    let outcome = search(&setup.plan, Arc::new(router)).await.unwrap();
    assert_eq!(shared.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        outcome
            .records
            .iter()
            .filter(|r| r.outcome == "routed")
            .count(),
        1
    );
    assert!(outcome.elapsed >= secs(2000));
}

// ─── The routing phase ───────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn routing_failures_are_records_and_three_in_a_row_stop() {
    // Every call fails: three are tried, the rest are stopped.
    let setup = setup(100_000);
    let router = fake(
        10,
        |n, _| if n % 2 == 0 { Step::Export } else { Step::Call },
    );
    let shared = Arc::clone(&router.0);
    let outcome = search(&setup.plan, Arc::new(router)).await.unwrap();
    assert_eq!(
        shared.calls.load(Ordering::SeqCst),
        ROUTE_FAILURES_BEFORE_STOP
    );
    assert_eq!(count(&outcome, "export_refused"), 2);
    assert_eq!(count(&outcome, "route_failed"), 1);
    assert_eq!(
        count(&outcome, "not_routed_stopped"),
        selected(&outcome) - 3
    );
    assert!(outcome
        .stopped_routing
        .as_deref()
        .unwrap()
        .contains("in a row"));
    assert_eq!(count(&outcome, "routed"), 0);
    let refused = outcome
        .records
        .iter()
        .find(|r| r.outcome == "export_refused")
        .unwrap();
    assert_eq!(refused.reason.as_deref(), Some("the export refused"));
}

#[tokio::test(start_paused = true)]
async fn a_success_between_the_failures_keeps_the_phase_going() {
    // F F R F F: four failures in all, never three in a row.
    let setup = setup(100_000);
    let router = fake(10, |n, _| match n {
        0 | 1 | 3 | 4 => Step::Export,
        2 => Step::Routed {
            unconnected: 0,
            signal_mm: 50.0,
        },
        _ => Step::Routed {
            unconnected: 0,
            signal_mm: 50.0,
        },
    });
    let shared = Arc::clone(&router.0);
    let outcome = search(&setup.plan, Arc::new(router)).await.unwrap();
    assert_eq!(
        shared.calls.load(Ordering::SeqCst),
        selected(&outcome),
        "every selected candidate is called"
    );
    assert!(outcome.stopped_routing.is_none());
    assert_eq!(count(&outcome, "export_refused"), 4);
    assert_eq!(count(&outcome, "routed"), selected(&outcome) - 4);
}

#[tokio::test(start_paused = true)]
async fn calls_with_no_winner_count_toward_the_stop_like_failures() {
    // No winner, a refused export, no winner: three misses in a row, no routed
    // candidate yet. The first call may be any of them, and a mix counts.
    let setup = setup(100_000);
    let router = fake(10, |n, _| match n {
        0 | 2 => Step::NoWinner,
        1 => Step::Export,
        _ => Step::Routed {
            unconnected: 0,
            signal_mm: 50.0,
        },
    });
    let shared = Arc::clone(&router.0);
    let outcome = search(&setup.plan, Arc::new(router)).await.unwrap();
    assert_eq!(
        shared.calls.load(Ordering::SeqCst),
        ROUTE_FAILURES_BEFORE_STOP
    );
    assert_eq!(count(&outcome, "no_winner"), 2);
    assert_eq!(count(&outcome, "export_refused"), 1);
    assert_eq!(count(&outcome, "routed"), 0);
    assert_eq!(
        count(&outcome, "not_routed_stopped"),
        selected(&outcome) - 3
    );
    assert!(outcome
        .stopped_routing
        .as_deref()
        .unwrap()
        .contains("in a row"));

    // A routed call between them resets the count, and after one the phase never stops
    // this way: no winner, no winner, routed, then no winner after no winner after no winner.
    let setup = self::setup(100_000);
    let router = fake(10, |n, _| match n {
        0 | 1 | 3 | 4 | 5 => Step::NoWinner,
        _ => Step::Routed {
            unconnected: 0,
            signal_mm: 50.0,
        },
    });
    let shared = Arc::clone(&router.0);
    let outcome = search(&setup.plan, Arc::new(router)).await.unwrap();
    assert_eq!(shared.calls.load(Ordering::SeqCst), selected(&outcome));
    assert!(outcome.stopped_routing.is_none());
    assert_eq!(count(&outcome, "no_winner"), 5);
}

#[tokio::test(start_paused = true)]
async fn a_call_with_no_winner_does_not_set_the_launch_rules_median() {
    // The first call judges nothing and lasts 600 s; the others route in 100 s. Limit 1000 s.
    // The median of the routed calls only: before one has routed, the estimate is 300 s,
    // and 600 + 300 fits, so the second call starts at 600 s. With the no-winner call in
    // the median it would be 600 s, 600 + 600 does not fit, and the search would end there.
    let setup = setup(1000);
    let router = fake_walls(
        |n| if n == 0 { 600 } else { 100 },
        |n, _| {
            if n == 0 {
                Step::NoWinner
            } else {
                Step::Routed {
                    unconnected: 0,
                    signal_mm: 50.0,
                }
            }
        },
    );
    let shared = Arc::clone(&router.0);
    let outcome = search(&setup.plan, Arc::new(router)).await.unwrap();
    let starts = shared.starts.lock().unwrap();
    assert!(starts.len() >= 2, "{} call(s) started", starts.len());
    assert_eq!(starts[1].1.duration_since(starts[0].1), secs(600));
    // After one routed call (100 s) the median is 100 s: the next starts at 700 s, and the
    // calls go on until 900 s.
    assert_eq!(
        starts.last().unwrap().1.duration_since(starts[0].1),
        secs(900)
    );
    assert_eq!(count(&outcome, "no_winner"), 1);
    assert_eq!(count(&outcome, "routed"), 4);
}

#[tokio::test(start_paused = true)]
async fn failures_after_a_routed_candidate_do_not_stop_the_phase() {
    let setup = setup(100_000);
    let router = fake(10, |n, _| match n {
        0 => Step::Routed {
            unconnected: 0,
            signal_mm: 50.0,
        },
        1..=5 => Step::Call,
        _ => Step::Routed {
            unconnected: 0,
            signal_mm: 50.0,
        },
    });
    let shared = Arc::clone(&router.0);
    let outcome = search(&setup.plan, Arc::new(router)).await.unwrap();
    assert_eq!(shared.calls.load(Ordering::SeqCst), selected(&outcome));
    assert!(outcome.stopped_routing.is_none());
    assert_eq!(count(&outcome, "route_failed"), 5);
}

#[tokio::test(start_paused = true)]
async fn a_failed_input_check_stops_the_routing_before_any_call() {
    let setup = setup(100_000);
    let mut shared = fake(10, good);
    Arc::get_mut(&mut shared.0).unwrap().unrouted = Err("kicad-cli is missing".into());
    let calls = Arc::clone(&shared.0);
    let outcome = search(&setup.plan, Arc::new(shared)).await.unwrap();
    assert_eq!(calls.calls.load(Ordering::SeqCst), 0);
    assert!(outcome
        .stopped_routing
        .as_deref()
        .unwrap()
        .contains("unrouted check failed"));
    assert_eq!(count(&outcome, "not_routed_stopped"), selected(&outcome));
    assert!(selected(&outcome) > 10);
}

#[tokio::test(start_paused = true)]
async fn search_numbers_o_then_a_then_the_recipes_and_ranks_them() {
    let mut setup = setup(100_000);
    // A: S0 with every movable part moved a little and one turned.
    let model = model();
    let s0 = s0_text(&model).unwrap();
    let mut placements = placements_of(&parse(&s0), &model.held.movable).unwrap();
    placements[0].x += 1.0;
    placements[1].rotation = 90.0;
    let ai_text = placements_applied_to(&s0, &placements).unwrap();
    let ai_dir = setup.root.join("ai");
    std::fs::create_dir_all(&ai_dir).unwrap();
    let ai_path = ai_dir.join("board.kicad_pcb");
    std::fs::write(&ai_path, &ai_text).unwrap();
    setup.plan.ai_board = Some(ai_path);
    // Every call the same, so ties decide: O's calls rank by id.
    let router = fake(10, good);
    let outcome = search(&setup.plan, Arc::new(router)).await.unwrap();
    let records = &outcome.records;
    for (id, record) in records.iter().enumerate() {
        assert_eq!(record.id, id, "ids are positions");
    }
    for (id, record) in records.iter().enumerate().take(3) {
        assert!(matches!(record.origin, Origin::Owner { call } if call == id + 1));
    }
    for (id, record) in records.iter().enumerate().take(6).skip(3) {
        assert!(matches!(&record.origin, Origin::Ai { call, .. } if *call == id - 2));
    }
    for record in &records[6..] {
        assert!(matches!(record.origin, Origin::Generated { .. }));
    }
    assert_eq!(records.len(), 6 + 24);
    // Ties in the key go to the lower id: O's identical calls rank 1, 2, 3 in order
    // unless a better candidate sits above them.
    let o_ranks: Vec<usize> = records[..3].iter().map(|r| r.rank.unwrap()).collect();
    assert!(o_ranks.windows(2).all(|w| w[0] < w[1]), "{o_ranks:?}");
    // Ranks are 1..=n over the judged candidates.
    let mut ranks: Vec<usize> = records.iter().filter_map(|r| r.rank).collect();
    ranks.sort_unstable();
    assert_eq!(ranks, (1..=ranks.len()).collect::<Vec<_>>());
    // search.json and table.md are written below the plan's folder.
    assert!(setup.plan.folder.join("search.json").is_file());
    assert!(setup.plan.folder.join("table.md").is_file());
    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(setup.plan.folder.join("search.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(json["records"].as_array().unwrap().len(), records.len());
    assert_eq!(json["plan"]["effort"], "m");
}

#[tokio::test(start_paused = true)]
async fn a_refused_ai_board_is_one_record_after_the_owners() {
    let mut setup = setup(100_000);
    let model = model();
    let s0 = s0_text(&model).unwrap();
    // A changed net: not a placement.
    let ai_text = s0.replace("(net \"SIG1\")", "(net \"SIGX\")");
    let ai_path = setup.root.join("ai-board.kicad_pcb");
    std::fs::write(&ai_path, ai_text).unwrap();
    setup.plan.ai_board = Some(ai_path);
    let outcome = search(&setup.plan, Arc::new(fake(10, good))).await.unwrap();
    let refused: Vec<&Record> = outcome
        .records
        .iter()
        .filter(|r| matches!(r.origin, Origin::Ai { .. }))
        .collect();
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].id, 3);
    assert_eq!(refused[0].outcome, "ai_refused");
    assert!(refused[0].reason.as_deref().unwrap().contains("differs"));
    assert_eq!(outcome.records[4].id, 4, "the recipes follow at once");
    assert!(matches!(
        outcome.records[4].origin,
        Origin::Generated { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn the_search_holds_what_the_plans_rules_say() {
    // The fixture's J1 is an unlocked connector with a netted pad: `HoldRules::default()`
    // holds it, `locks_only()` does not.
    let text = text();
    let none = BTreeSet::new();
    let locks = held_set(&text, &none, &HoldRules::locks_only()).unwrap();
    let rules = held_set(&text, &none, &HoldRules::default()).unwrap();
    assert!(locks.movable.contains("J1") && rules.held.contains_key("J1"));
    assert_ne!(locks.held_sha256, rules.held_sha256);

    let mut setup = setup(100_000);
    setup.plan.hold = HoldRules::locks_only();
    setup.plan.ai_board = None;
    let outcome = search(&setup.plan, Arc::new(fake(10, good))).await.unwrap();
    // Every record was made against the locks-only held list, and J1 moved in a routed
    // candidate (the routed ones keep their board text).
    assert!(outcome
        .records
        .iter()
        .all(|r| r.held_sha256 == locks.held_sha256));
    let mut j1_moved = false;
    for record in outcome.records.iter().filter(|r| r.board_text.is_some()) {
        let tree = parse(record.board_text.as_deref().unwrap());
        let placements = placements_of(&tree, &locks.movable).unwrap();
        let j1 = placements.iter().find(|p| p.reference == "J1").unwrap();
        j1_moved |= (j1.x, j1.y) != (58.0, 20.0);
    }
    assert!(j1_moved, "no candidate moved the unlocked connector");
    // search.json records the rules the search ran with.
    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(setup.plan.folder.join("search.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        json["plan"]["hold_rules"],
        serde_json::to_value(HoldRules::locks_only()).unwrap()
    );

    // Negative control: with the default rules J1 is held, and the list is the other one.
    let mut setup = self::setup(100_000);
    setup.plan.ai_board = None;
    let outcome = search(&setup.plan, Arc::new(fake(10, good))).await.unwrap();
    assert!(outcome
        .records
        .iter()
        .all(|r| r.held_sha256 == rules.held_sha256));
    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(setup.plan.folder.join("search.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        json["plan"]["hold_rules"],
        serde_json::to_value(HoldRules::default()).unwrap()
    );
}

#[tokio::test(start_paused = true)]
async fn a_family_the_matrix_leaves_out_has_no_row_and_no_verdict() {
    let mut setup = setup(100_000);
    setup.plan.ai_board = None;
    setup.plan.matrix = setup
        .plan
        .matrix
        .clone()
        .only(&[Family::Force, Family::Schematic]);
    let outcome = search(&setup.plan, Arc::new(fake(10, good))).await.unwrap();
    let table_text = std::fs::read_to_string(setup.plan.folder.join("table.md")).unwrap();
    assert_eq!(table_text, table(&setup.plan, &outcome));
    assert!(table_text.contains("(families F, S)"), "{table_text}");
    for left_in in ["| F |", "| S |", "| O |", "| A |"] {
        assert!(table_text.contains(left_in), "{left_in}: {table_text}");
    }
    assert!(!table_text.contains("| P |"), "{table_text}");
    // Negative control: with P in the matrix the same table has its row and verdict.
    let mut setup = self::setup(100_000);
    setup.plan.ai_board = None;
    let outcome = search(&setup.plan, Arc::new(fake(10, good))).await.unwrap();
    let table_text = table(&setup.plan, &outcome);
    assert!(table_text.contains("(families F, S, P)"), "{table_text}");
    assert_eq!(table_text.matches("| P |").count(), 2, "{table_text}");
}

#[tokio::test(start_paused = true)]
async fn a_held_list_that_cannot_be_proved_is_an_error_before_phase_one() {
    let mut setup = setup(100_000);
    setup.plan.caller_held.insert("NOPE".into());
    let error = search(&setup.plan, Arc::new(fake(10, good)))
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("NOPE"), "{error:#}");
    assert!(!setup.plan.folder.join("search.json").exists());
}

#[tokio::test(start_paused = true)]
async fn the_table_names_no_net_and_no_part() {
    let mut setup = setup(100_000);
    // The generated candidates route completely, O leaves 5 open: every family beats O.
    let router = fake(10, |_, id| Step::Routed {
        unconnected: if id < 3 { 5 } else { 0 },
        signal_mm: 100.0,
    });
    setup.plan.ai_board = None;
    let outcome = search(&setup.plan, Arc::new(router)).await.unwrap();
    let table_text = std::fs::read_to_string(setup.plan.folder.join("table.md")).unwrap();
    assert_eq!(table_text, table(&setup.plan, &outcome));
    let model = model();
    let mut names: Vec<String> = model.references.clone();
    names.extend(model.net_fanout.keys().cloned());
    names.extend(["LOGO".to_string(), "Test:".to_string()]);
    for name in names {
        assert!(!table_text.contains(&name), "`{name}` appears in table.md");
    }
    // Counts and verdicts are there.
    assert!(table_text.contains("| rank | origin |"));
    assert!(
        table_text.contains("| F | beats | unknown |"),
        "{table_text}"
    );
    assert!(table_text.contains("| P | beats | unknown |"));
    assert!(table_text.contains("| O call 1 |") || table_text.contains("O call 1"));
}

#[tokio::test(start_paused = true)]
async fn a_reference_call_with_no_winner_is_the_references_worst_rank() {
    // O's calls: routed, no winner, routed. Every generated candidate routes but leaves one
    // connection open, so each ranks below O's two routed calls and above its no-winner
    // call. A family's best is then inside O's own spread (it matches); counting O's
    // routed calls only would put it below all of them.
    let setup = setup(100_000);
    let router = fake(10, |_, id| match id {
        1 => Step::NoWinner,
        0 | 2 => Step::Routed {
            unconnected: 0,
            signal_mm: 100.0,
        },
        _ => Step::Routed {
            unconnected: 1,
            signal_mm: 100.0,
        },
    });
    let outcome = search(&setup.plan, Arc::new(router)).await.unwrap();
    let ranks: Vec<usize> = outcome.records[..3]
        .iter()
        .map(|r| r.rank.unwrap())
        .collect();
    let judged = outcome.records.iter().filter(|r| r.rank.is_some()).count();
    assert_eq!(ranks, vec![1, judged, 2], "the no-winner call ranks last");
    let table_text = table(&setup.plan, &outcome);
    for family in ["F", "S", "P"] {
        assert!(
            table_text.contains(&format!("| {family} | matches | unknown |")),
            "{table_text}"
        );
    }
    assert!(table_text.contains("no-winner call ranks as the reference's worst"));
}

#[tokio::test(start_paused = true)]
async fn the_table_prints_a_code_for_a_stop_never_the_reason() {
    // The input check fails with a message that names a path and a tool's output.
    let setup = setup(100_000);
    let mut router = fake(10, good);
    Arc::get_mut(&mut router.0).unwrap().unrouted =
        Err("/Users/owner/boards/secret.kicad_pcb: kicad-cli said: boom".into());
    let outcome = search(&setup.plan, Arc::new(router)).await.unwrap();
    let table_text = std::fs::read_to_string(setup.plan.folder.join("table.md")).unwrap();
    assert!(
        table_text.contains("Routing stopped early: input_check_failed."),
        "{table_text}"
    );
    for leaked in ["secret", "kicad-cli", "/Users", "boom"] {
        assert!(!table_text.contains(leaked), "`{leaked}` in table.md");
    }
    // The full reason is kept in search.json.
    let json = std::fs::read_to_string(setup.plan.folder.join("search.json")).unwrap();
    assert!(json.contains("secret.kicad_pcb") && json.contains("input_check_failed"));
    assert!(outcome.stopped_routing.as_deref().unwrap().contains("boom"));

    // The other two stops have their own codes.
    let setup = self::setup(100_000);
    let router = fake(10, |_, _| Step::Export);
    search(&setup.plan, Arc::new(router)).await.unwrap();
    let table_text = std::fs::read_to_string(setup.plan.folder.join("table.md")).unwrap();
    assert!(table_text.contains("Routing stopped early: calls_failed_in_a_row."));
    assert!(!table_text.contains("the export refused"));
    let setup = self::setup(1000);
    search(&setup.plan, Arc::new(fake(100, good)))
        .await
        .unwrap();
    let table_text = std::fs::read_to_string(setup.plan.folder.join("table.md")).unwrap();
    assert!(table_text.contains("Routing stopped early: time_limit."));
    // No stop, no line.
    let setup = self::setup(100_000);
    search(&setup.plan, Arc::new(fake(10, good))).await.unwrap();
    let table_text = std::fs::read_to_string(setup.plan.folder.join("table.md")).unwrap();
    assert!(!table_text.contains("Routing stopped"));
}

#[tokio::test(start_paused = true)]
async fn records_carry_the_gate_lines_and_the_unrouted_counts_in_search_json_only() {
    let setup = setup(100_000);
    let outcome = search(&setup.plan, Arc::new(fake(10, |_, _| Step::Diagnosed)))
        .await
        .unwrap();
    let routed: Vec<&Record> = outcome
        .records
        .iter()
        .filter(|r| r.outcome == "routed")
        .collect();
    assert!(!routed.is_empty());
    for record in &routed {
        // The unrouted board has 3 `clearance` entries and the input none: P2 says so.
        let gates = record.placement_gates.as_ref().expect("the P gates");
        assert!(
            gates.p2.iter().any(|line| line.starts_with("clearance: 3")),
            "{gates:?}"
        );
        assert_eq!(
            record.unrouted,
            Some(BTreeMap::from([("clearance".to_string(), 3)]))
        );
        assert_eq!(
            record.route_gates.as_ref().unwrap().g1,
            vec!["net SECRETNET is not fully routed".to_string()]
        );
    }
    // A record that was not routed has none of the three.
    let screened = outcome
        .records
        .iter()
        .find(|r| r.outcome == "screened_out")
        .expect("a screened-out record");
    assert!(screened.placement_gates.is_none() && screened.unrouted.is_none());
    assert!(screened.route_gates.is_none());

    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(setup.plan.folder.join("search.json")).unwrap(),
    )
    .unwrap();
    let first = json["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["outcome"] == "routed")
        .unwrap();
    assert_eq!(first["unrouted"]["clearance"], 3);
    assert_eq!(
        first["route_gates"]["g1"][0],
        "net SECRETNET is not fully routed"
    );
    assert!(first["placement_gates"]["p2"][0]
        .as_str()
        .unwrap()
        .starts_with("clearance: 3"));
    assert!(json["plan"]["clearances"]["part_gap_mm"].is_number());
    assert!(json["plan"]["clearances"]["edge_mm"].is_number());
    // Never in the table: the lines can name nets.
    let table_text = std::fs::read_to_string(setup.plan.folder.join("table.md")).unwrap();
    assert!(!table_text.contains("SECRETNET") && !table_text.contains("clearance"));
}

#[tokio::test(start_paused = true)]
async fn the_search_sets_the_clearances_from_its_routing_inputs() {
    let mut setup = setup(100_000);
    setup.plan.route.options.copper_edge_clearance_um = Some(500);
    setup.plan.route.rules = BTreeMap::from([(
        "N".to_string(),
        konnect_ipc::types::IpcRoutingRules {
            class_name: "Default".into(),
            constituents: Vec::new(),
            track_width_mm: None,
            clearance_mm: Some(0.25),
            via_diameter_mm: None,
            via_drill_mm: None,
        },
    )]);
    let outcome = search(&setup.plan, Arc::new(fake(10, good))).await.unwrap();
    assert!((outcome.clearances.part_gap_mm - 0.35).abs() < 1e-12);
    assert!((outcome.clearances.edge_mm - 0.5).abs() < 1e-12);
    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(setup.plan.folder.join("search.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(json["plan"]["clearances"]["edge_mm"], 0.5);
}

#[test]
fn the_clearances_come_from_the_rules_and_the_export_options() {
    let rule = |clearance: Option<f64>| konnect_ipc::types::IpcRoutingRules {
        class_name: "Default".into(),
        constituents: Vec::new(),
        track_width_mm: None,
        clearance_mm: clearance,
        via_diameter_mm: None,
        via_drill_mm: None,
    };
    let setup = setup(100);
    let mut options = setup.plan.route.options.clone();
    options.copper_edge_clearance_um = Some(500);
    let rules = BTreeMap::from([
        ("A".to_string(), rule(Some(0.2))),
        ("B".to_string(), rule(Some(0.3))),
        ("C".to_string(), rule(None)),
    ]);
    let found = Clearances::from_inputs(&rules, &options);
    assert!((found.part_gap_mm - 0.4).abs() < 1e-12, "{found:?}");
    assert!((found.edge_mm - 0.5).abs() < 1e-12);
    options.copper_edge_clearance_um = None;
    let found = Clearances::from_inputs(&BTreeMap::new(), &options);
    assert_eq!(found, Clearances::default());
}

#[tokio::test(start_paused = true)]
async fn only_the_boards_that_are_routed_keep_their_text() {
    // 288 copies of a 2 MB board would be held to the end: after the selection, only the
    // candidates in the routing order keep theirs.
    let setup = setup(100_000);
    let outcome = search(&setup.plan, Arc::new(fake(10, good))).await.unwrap();
    let with_text = outcome
        .records
        .iter()
        .filter(|r| r.board_text.is_some())
        .count();
    assert!(
        count(&outcome, "screened_out") > 0,
        "the fixture screens some out"
    );
    assert_eq!(with_text, selected(&outcome));
    for record in &outcome.records {
        if matches!(record.outcome, "screened_out" | "duplicate") {
            assert!(record.board_text.is_none(), "{}", record.id);
        }
    }
}

// ─── The references ──────────────────────────────────────────────────────────

fn edit_footprint(text: &str, reference: &str, edit: impl Fn(&str) -> String) -> String {
    let at = text
        .find(&format!("(property \"Reference\" \"{reference}\""))
        .expect("the reference is on the board");
    let start = text[..at].rfind("(footprint \"").unwrap();
    let end = start + text[start..].find("\n)\n").unwrap() + 3;
    format!(
        "{}{}{}",
        &text[..start],
        edit(&text[start..end]),
        &text[end..]
    )
}

#[test]
fn admit_ai_refuses_any_edit_but_placement() {
    let model = model();
    let s0 = s0_text(&model).unwrap();
    let movable = &model.held.movable;
    // Movable parts moved and turned part by part, as an AI session writes them: a move
    // by the move tool's own writer, a turn by a set (what the move and rotate tools
    // write together). Admitted, and what is judged is the rebuilt text.
    let placements = placements_of(&parse(&s0), movable).unwrap();
    let mut ai = s0.clone();
    for (k, p) in placements.iter().enumerate() {
        let (x, y) = (10.0 + 3.0 * k as f64, 15.0 + k as f64);
        ai = if k % 2 == 0 {
            placements_applied_to(&ai, &[placement(&p.reference, x, y, 90.0)]).unwrap()
        } else {
            footprint_moved_in(&ai, &p.reference, x, y).unwrap()
        };
    }
    let rebuilt = admit_ai(&s0, &ai, movable).expect("a placement is admitted");
    assert_eq!(same_board(&parse(&rebuilt), &parse(&ai)), Ok(()));
    // S0 itself is admitted too.
    assert!(admit_ai(&s0, &s0, movable).is_ok());

    let refuse = |what: &str, edited: String| {
        let refusal =
            admit_ai(&s0, &edited, movable).expect_err(&format!("{what} must be refused"));
        assert!(refusal.contains("differs"), "{what}: {refusal}");
    };
    refuse(
        "a flip",
        edit_footprint(&ai, "C1", |fp| {
            fp.replacen("(layer \"F.Cu\")", "(layer \"B.Cu\")", 1)
        }),
    );
    refuse(
        "a moved held part",
        placements_applied_to(&ai, &[placement("J1", 50.0, 20.0, 0.0)]).unwrap(),
    );
    refuse(
        "a changed net",
        ai.replace("(net \"SIG1\")", "(net \"SIGX\")"),
    );
    let zone =
        "(zone (net \"GND\") (layer \"F.Cu\") (polygon (pts (xy 0 0) (xy 5 0) (xy 5 5))))\n)\n";
    refuse(
        "a zone",
        ai.trim_end().strip_suffix(')').unwrap().to_string() + zone,
    );
    refuse(
        "a changed outline",
        ai.replacen("(end 60 0)", "(end 61 0)", 1),
    );
    refuse(
        "a changed rule",
        ai.replace(
            "(pad_to_mask_clearance 0.05)",
            "(pad_to_mask_clearance 0.06)",
        ),
    );
    // Unreadable or incomplete boards are refused with a reason, not a panic.
    assert!(admit_ai(&s0, "(kicad_pcb", movable).is_err());
    let missing = edit_footprint(&ai, "R3", |_| String::new());
    assert!(admit_ai(&s0, &missing, movable).is_err());
}

/// The board's text with R9 at 180 degrees and its texts at angles a set does not keep:
/// 100 degrees becomes -80, which a second set makes 280.
fn text_with_a_turned_part_and_angled_text() -> String {
    let mut all = parts();
    all.push(
        Fp::new("R9", 40.0, 8.0)
            .lib("Test:R")
            .two_pad("SIG8", "SIG9")
            .rot(180.0),
    );
    edit_footprint(&board(OUTLINE, &all), "R9", |fp| {
        fp.replace("(at 0 -1.5 0)", "(at 0 -1.5 100)")
            .replace("(at 0 1.5 0)", "(at 0 1.5 180)")
    })
}

#[test]
fn a_board_written_by_the_move_writer_is_admitted() {
    // An AI session moves R9 (at 180 degrees, with texts) and C1 with the move tool's
    // writer: both keep their children's angles as spelled.
    let model = model_of(&text_with_a_turned_part_and_angled_text());
    let s0 = s0_text(&model).unwrap();
    let mut ai = footprint_moved_in(&s0, "R9", 44.0, 9.0).unwrap();
    ai = footprint_moved_in(&ai, "C1", 20.0, 12.0).unwrap();
    // Setting every part the AI left turned would re-spell R9's texts: the board the AI
    // wrote is then no longer the board rebuilt from its anchors.
    let placements = placements_of(&parse(&ai), &model.held.movable).unwrap();
    let set_everything = placements_applied_to(&s0, &placements).unwrap();
    assert!(
        same_board(&parse(&set_everything), &parse(&ai)).is_err(),
        "the fixture must show that a set changes what a move leaves"
    );
    let rebuilt = admit_ai(&s0, &ai, &model.held.movable).expect("the move writer's board");
    assert_eq!(same_board(&parse(&rebuilt), &parse(&ai)), Ok(()));
    // A turn is still a set: R9 turned to 270 degrees by a set is admitted too.
    let turned = placements_applied_to(&ai, &[placement("R9", 44.0, 9.0, 270.0)]).unwrap();
    assert!(admit_ai(&s0, &turned, &model.held.movable).is_ok());
}

#[test]
fn verdict_needs_to_clear_every_reference_call() {
    let calls = [4, 6, 9];
    assert_eq!(verdict(Some(1), &calls), Verdict::Beats);
    assert_eq!(verdict(Some(3), &calls), Verdict::Beats);
    assert_eq!(
        verdict(Some(5), &calls),
        Verdict::Matches,
        "between two calls"
    );
    assert_eq!(verdict(Some(4), &calls), Verdict::Matches);
    assert_eq!(verdict(Some(9), &calls), Verdict::Matches);
    assert_eq!(verdict(Some(10), &calls), Verdict::Below);
    assert_eq!(verdict(None, &calls), Verdict::Unknown);
    assert_eq!(verdict(Some(1), &[]), Verdict::Unknown);
    assert_eq!(verdict(Some(2), &[3]), Verdict::Beats);
    assert_eq!(verdict(Some(4), &[3]), Verdict::Below);
}

// ─── Rerun ───────────────────────────────────────────────────────────────────

#[test]
fn rerun_rebuilds_the_recorded_board() {
    let model = model();
    let records = generate_all(&model, &recipes(&small_matrix()), 2, 6);
    let record = records
        .iter()
        .find(|r| r.outcome == "generated")
        .expect("a generated candidate");
    let rebuilt = rerun(&model, record).expect("the same build rebuilds it");
    assert_eq!(Some(&rebuilt), record.board_text.as_ref());

    let mut other_version = record.clone();
    if let Origin::Generated {
        generator_version, ..
    } = &mut other_version.origin
    {
        *generator_version += 1;
    }
    assert!(rerun(&model, &other_version)
        .unwrap_err()
        .to_string()
        .contains("generator version"));
    for edit in [
        |r: &mut Record| r.input_sha256 = "x".into(),
        |r: &mut Record| r.held_sha256 = "x".into(),
        |r: &mut Record| r.intent_sha256 = "x".into(),
        |r: &mut Record| r.board_sha256 = Some("x".into()),
        |r: &mut Record| r.origin = Origin::Owner { call: 1 },
    ] {
        let mut changed = record.clone();
        edit(&mut changed);
        assert!(rerun(&model, &changed).is_err());
    }
}

// ─── The production RouteCall ────────────────────────────────────────────────

#[tokio::test]
async fn the_production_call_reports_an_export_refusal_as_a_stage() {
    let setup = setup(100);
    let isolation = setup.root.join("isolation");
    let call = PlaceRouteCall {
        inputs: setup.plan.route.clone(),
        ctx: crate::place_route::isolated_context(&isolation, "no-kicad-cli").unwrap(),
    };
    // The candidate's folder exists already: the export refuses before any routing.
    let (result, timings) = call.route(1, text(), setup.root.join("input")).await;
    assert!(
        matches!(
            &result,
            RouteResult::Failed {
                stage: "export",
                ..
            }
        ),
        "{result:?}"
    );
    assert!(timings.route_s.is_none());
}

#[tokio::test]
async fn the_production_input_check_refuses_a_text_that_is_not_the_input() {
    let setup = setup(100);
    let isolation = setup.root.join("isolation");
    let call = PlaceRouteCall {
        inputs: setup.plan.route.clone(),
        ctx: crate::place_route::isolated_context(&isolation, "no-kicad-cli").unwrap(),
    };
    let error = call
        .unrouted("(kicad_pcb)".into(), setup.root.join("check"))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("not the input board"),
        "{error:#}"
    );
}

#[test]
fn the_search_never_writes_the_input_board() {
    // `search` is given the stripped input by path; the fake run leaves it as it was.
    let setup = setup(100_000);
    let before = std::fs::read(&setup.plan.route.board).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    runtime
        .block_on(search(&setup.plan, Arc::new(fake(10, good))))
        .unwrap();
    assert_eq!(before, std::fs::read(&setup.plan.route.board).unwrap());
}

// ─── The schematic-neighbour measure (D23) ───────────────────────────────────

/// Writes the schematic of the fixture board next to it (`input/board.kicad_sch`): a flat
/// sheet, U1 with its capacitors close together, the resistors in a chain, J1 and H1 apart.
/// The fixture's footprints carry no path, so they match by reference.
fn write_schematic(setup: &Setup) {
    let at = [
        ("U1", 0.0, 0.0),
        ("C1", 5.0, 0.0),
        ("C2", -5.0, 0.0),
        ("C3", 0.0, 5.0),
        ("C4", 0.0, -5.0),
        ("R1", 30.0, 0.0),
        ("R2", 35.0, 0.0),
        ("R3", 40.0, 0.0),
        ("J1", 60.0, 0.0),
        ("H1", 90.0, 0.0),
    ];
    let mut text = String::from(
        "(kicad_sch (version 20260101) (generator \"test\") (uuid \"root\") (paper \"A4\")\n(lib_symbols)\n",
    );
    for (n, (reference, x, y)) in at.iter().enumerate() {
        text.push_str(&format!(
            "(symbol (lib_id \"Dev:Part\") (at {x} {y} 0) (unit 1) (uuid \"u{n}\") (property \"Reference\" \"{reference}\" (at 0 0 0)))\n"
        ));
    }
    text.push_str(")\n");
    std::fs::write(setup.root.join("input").join("board.kicad_sch"), text).unwrap();
}

/// Ids, keys and ranks of every record, the three things the rank is.
fn ranking(outcome: &Outcome) -> Vec<(usize, Option<Key>, Option<usize>)> {
    outcome
        .records
        .iter()
        .map(|r| (r.id, r.key, r.rank))
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_missing_schematic_is_a_reason_on_every_board_and_the_search_goes_on() {
    // `setup` writes no schematic.
    let setup = setup(100_000);
    let outcome = search(&setup.plan, Arc::new(fake(10, good))).await.unwrap();
    assert!(count(&outcome, "routed") > 0, "the search completed");
    let with_board: Vec<&Record> = outcome
        .records
        .iter()
        .filter(|r| r.board_sha256.is_some() && r.outcome != "write_failed")
        .collect();
    assert!(!with_board.is_empty());
    for record in &with_board {
        assert!(record.schematic_neighbours.is_none());
        let reason = record.schematic_neighbours_reason.as_deref().unwrap();
        assert!(reason.contains("board.kicad_sch"), "{reason}");
    }
    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(setup.plan.folder.join("search.json")).unwrap(),
    )
    .unwrap();
    let first = json["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["outcome"] == "routed")
        .unwrap();
    assert!(first["schematic_neighbours"].is_null());
    assert!(first["schematic_neighbours_reason"].is_string());
    // The table prints a dash in both columns.
    let table_text = std::fs::read_to_string(setup.plan.folder.join("table.md")).unwrap();
    assert!(table_text.contains("| - | - |\n"), "{table_text}");
}

#[tokio::test(start_paused = true)]
async fn the_measure_reaches_search_json_and_the_table_and_the_input_measures_one() {
    let setup = setup(100_000);
    write_schematic(&setup);
    let outcome = search(&setup.plan, Arc::new(fake(10, good))).await.unwrap();
    let routed: Vec<&Record> = outcome
        .records
        .iter()
        .filter(|r| r.outcome == "routed")
        .collect();
    assert!(routed.len() > 10);
    for record in &routed {
        assert!(record.schematic_neighbours_reason.is_none());
        let measure = record.schematic_neighbours.as_ref().expect("a measure");
        assert_eq!((measure.parts_measured, measure.parts_unmatched), (10, 0));
        // The two pad-less logos are skipped, not unmatched.
        assert_eq!(measure.parts_alone, 0);
    }
    // O's calls are the input board: ratio 1, nobody pulled.
    for record in outcome.records.iter().filter(|r| r.group() == "O") {
        let measure = record.schematic_neighbours.as_ref().unwrap();
        assert_eq!(measure.ratio_to_input, Some(1.0));
        assert_eq!(measure.parts_pulled_away, 0);
    }
    // Generated placements move parts, so some candidate measures differently.
    assert!(
        routed
            .iter()
            .any(|r| r.schematic_neighbours.as_ref().unwrap().ratio_to_input != Some(1.0)),
        "no generated candidate differs from the input"
    );
    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(setup.plan.folder.join("search.json")).unwrap(),
    )
    .unwrap();
    let generated = json["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["outcome"] == "routed" && r["origin"]["kind"] == "generated")
        .unwrap();
    let measure = &generated["schematic_neighbours"];
    for field in [
        "mean_mm",
        "ratio_to_input",
        "parts_pulled_away",
        "parts_measured",
        "parts_unmatched",
    ] {
        assert!(!measure[field].is_null(), "{field}: {measure}");
    }
    let table_text = std::fs::read_to_string(setup.plan.folder.join("table.md")).unwrap();
    assert!(
        table_text.contains("| score | sch. nbr mm | pulled away |"),
        "{table_text}"
    );
    assert!(table_text.contains("are reported only; they do not rank."));
    // A routed row ends in the two numbers, not dashes.
    let row = table_text
        .lines()
        .find(|line| line.starts_with("| 1 |"))
        .unwrap();
    let cells: Vec<&str> = row.trim_end_matches('|').rsplit('|').take(2).collect();
    assert!(
        cells[0].trim().parse::<usize>().is_ok(),
        "pulled away: {row}"
    );
    assert!(cells[1].trim().parse::<f64>().is_ok(), "sch. nbr mm: {row}");
    // Counts only: the table names no part.
    for name in model().references {
        assert!(!table_text.contains(&format!(" {name} ")), "`{name}`");
    }
}

#[tokio::test(start_paused = true)]
async fn the_measure_does_not_change_a_key_or_a_rank() {
    // The same search, with the schematic and without: every key and rank is the same,
    // although the records differ in the measure.
    let with = setup(100_000);
    write_schematic(&with);
    let without = setup(100_000);
    let router = |_: usize, id: usize| Step::Routed {
        unconnected: id % 3,
        signal_mm: 100.0 + id as f64,
    };
    let a = search(&with.plan, Arc::new(fake(10, router)))
        .await
        .unwrap();
    let b = search(&without.plan, Arc::new(fake(10, router)))
        .await
        .unwrap();
    assert!(a.records.iter().any(|r| r.schematic_neighbours.is_some()));
    assert!(b.records.iter().all(|r| r.schematic_neighbours.is_none()));
    assert_eq!(ranking(&a), ranking(&b));
    assert!(a.records.iter().any(|r| r.rank.is_some()));
    // The verdicts are the table's rows after the routed ones.
    let verdicts = |text: String| {
        text.split("## Per family and reference")
            .nth(1)
            .unwrap()
            .to_string()
    };
    assert_eq!(
        verdicts(table(&with.plan, &a)),
        verdicts(table(&without.plan, &b))
    );
}
