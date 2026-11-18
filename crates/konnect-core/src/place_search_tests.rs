//! Tests of `place_search`: the generation pool, selection, the launch rule, the
//! routing phase (with fake `RouteCall`s on a paused clock), the references, rerun and
//! the table. Nothing here starts fastroute or KiCad.

use super::*;
use crate::place_generators::{ForceParams, Params, PerturbParams, SchematicParams};
use crate::place_held::tests::{board, board_with_groups, pad, parse, Fp, Shape};
use crate::place_route::export_options;
use crate::route_judge::{Gates, Measures};
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
    }
}

fn routed(unconnected: usize, signal_mm: f64) -> RouteResult {
    RouteResult::Routed {
        gates: Gates {
            g1: Vec::new(),
            g2: Vec::new(),
            g3: Vec::new(),
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
    Routed { unconnected: usize, signal_mm: f64 },
    NoWinner,
    Export,
    Call,
}

struct Shared {
    steps: Box<dyn Fn(usize, usize) -> Step + Send + Sync>,
    wall: Duration,
    calls: AtomicUsize,
    active: AtomicUsize,
    max_active: AtomicUsize,
    /// `(id, start)` of every call, on tokio's clock.
    starts: Mutex<Vec<(usize, Instant)>>,
    unrouted: Result<BTreeMap<String, usize>, String>,
}

struct Fake(Arc<Shared>);

fn fake(wall_s: u64, steps: impl Fn(usize, usize) -> Step + Send + Sync + 'static) -> Fake {
    Fake(Arc::new(Shared {
        steps: Box::new(steps),
        wall: Duration::from_secs(wall_s),
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
            tokio::time::sleep(shared.wall).await;
            shared.active.fetch_sub(1, Ordering::SeqCst);
            let result = match (shared.steps)(n, id) {
                Step::Routed {
                    unconnected,
                    signal_mm,
                } => routed(unconnected, signal_mm),
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
    let expected = vec![
        0, 1, 2, 3, 4, 5, // O three times, A three times
        13, 23, 33, // each family's best
        drawn[0], drawn[1], drawn[2], // each family's drawn
        11, 21, 31, // second best
        14, 24, 34, // third best
    ];
    assert_eq!(order, expected);
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
    // A family with fewer candidates than slots gives what it has.
    let few: Vec<Record> = records.into_iter().filter(|r| r.id < 12).collect();
    let order = routing_order(&few, 3, 1, 7);
    assert_eq!(order.iter().filter(|id| **id >= 10).count(), 2);
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
    // F F N F F: five failures in all, never three in a row.
    let setup = setup(100_000);
    let router = fake(10, |n, _| match n {
        0 | 1 | 3 | 4 => Step::Export,
        2 => Step::NoWinner,
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
    assert_eq!(count(&outcome, "no_winner"), 1);
    assert_eq!(count(&outcome, "routed"), selected(&outcome) - 5);
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
    // Movable parts moved and turned: admitted, and what is judged is the rebuilt text.
    let mut placements = placements_of(&parse(&s0), movable).unwrap();
    for (k, p) in placements.iter_mut().enumerate() {
        p.x = 10.0 + 3.0 * k as f64;
        p.y = 15.0 + k as f64;
        p.rotation = if k % 2 == 0 { 90.0 } else { 0.0 };
    }
    let ai = placements_applied_to(&s0, &placements).unwrap();
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
