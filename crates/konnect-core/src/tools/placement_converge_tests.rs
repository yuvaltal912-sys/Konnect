//! Tests of `plan_convergent_placement` (Row CT). The search runs for real on the
//! placement tests' fixture board at family C's reduced stage length; the router is the
//! search tests' fake, so nothing here starts fastroute, KiCad or kicad-cli.

use super::*;
use crate::place_generators::Family;
use crate::place_held::tests::{board, pad, Fp, Shape};
use crate::place_search::tests::{fake, good, quick, setup, small_matrix, Fake, Setup, Step};
use crate::place_search::{search, search_model};
use crate::tools::pcb_components::placements_applied_to;
use konnect_ipc::types::IpcRoutingRules;
use std::collections::BTreeMap;

// ─── Arguments (T1) ──────────────────────────────────────────────────────────

#[test]
fn the_arguments_default_to_effort_m_and_refuse_what_they_cannot_use() {
    let call = read_arguments(&json!({"board": "/x/board.kicad_pcb"})).unwrap();
    assert_eq!(call.effort, Effort::M);
    assert_eq!(call.fastroute_path, None);
    let call = read_arguments(
        &json!({"board": "/x/board.kicad_pcb", "effort": "xh", "fastroute_path": "/bin/fr"}),
    )
    .unwrap();
    assert_eq!(call.effort, Effort::Xh);
    assert_eq!(call.fastroute_path, Some(PathBuf::from("/bin/fr")));
    for args in [
        json!({"board": "/x/board.kicad_pcb", "effort": "max"}),
        json!({"board": "/x/board.kicad_pcb", "effort": 2}),
        json!({"board": "/x/board.kicad_pcb", "fastroute_path": ""}),
        json!({"board": "/x/board.kicad_sch"}),
        json!({"board": "/x/board"}),
        json!({}),
    ] {
        let refusal = read_arguments(&args).unwrap_err();
        assert!(refusal.is_error, "{args}");
    }
}

#[test]
fn the_tool_is_registered_live_only_in_the_placement_toolset_with_a_closed_schema() {
    let tools = crate::router::registry::tools_for("placement").unwrap();
    let tool = tools
        .iter()
        .find(|tool| tool.name == "plan_convergent_placement")
        .expect("registered in placement");
    assert_eq!(tool.board_access, crate::tools::BoardAccess::LiveOnly);
    assert_eq!(tool.input_schema["additionalProperties"], json!(false));
    let properties: BTreeSet<&str> = tool.input_schema["properties"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        properties,
        BTreeSet::from(["board", "effort", "fastroute_path", "run_id", "wait_s"])
    );
    // Neither is required: a call gives board to start a run or run_id to report one.
    assert!(tool.input_schema.get("required").is_none());
}

// ─── The live read (T3) ──────────────────────────────────────────────────────

fn rules(clearance_mm: f64) -> IpcEffectiveRoutingRules {
    BTreeMap::from([(
        "GND".to_string(),
        IpcRoutingRules {
            class_name: "Default".to_string(),
            constituents: Vec::new(),
            track_width_mm: Some(0.25),
            clearance_mm: Some(clearance_mm),
            via_diameter_mm: Some(0.6),
            via_drill_mm: Some(0.3),
        },
    )])
}

#[test]
fn a_stable_read_gives_the_text_and_the_rules() {
    let live = stable_read("A".into(), "A".into(), rules(0.2), rules(0.2)).unwrap();
    assert_eq!(live.text, "A");
    assert_eq!(live.rules, rules(0.2));
}

#[test]
fn a_board_or_rules_that_change_while_read_are_refused() {
    let text = stable_read("A".into(), "B".into(), rules(0.2), rules(0.2)).unwrap_err();
    assert!(text.contains("board changed"), "{text}");
    let rule = stable_read("A".into(), "A".into(), rules(0.2), rules(0.3)).unwrap_err();
    assert!(rule.contains("routing rules changed"), "{rule}");
}

// ─── The run folder and the scratch project (T5) ─────────────────────────────

#[test]
fn a_run_folder_is_new_and_named_by_the_input() {
    let state = tempfile::tempdir().unwrap();
    let folder = run_folder(state.path(), "board text", "1700000000000").unwrap();
    let digest = sha256_hex(b"board text");
    assert_eq!(
        folder,
        state
            .path()
            .join(PLANS_FOLDER)
            .join(format!("1700000000000-{}", &digest[..12]))
    );
    assert!(folder.is_dir());
    assert!(
        run_folder(state.path(), "board text", "1700000000000").is_err(),
        "no run writes into another's folder"
    );
}

/// A small routed board with a track and a via, beside its project, rules and intent.
fn project_folder() -> (tempfile::TempDir, PathBuf, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let parts = [
        Fp::new("R1", 10.0, 10.0).lib("Test:R").two_pad("A", "B"),
        Fp::new("R2", 20.0, 10.0).lib("Test:R").two_pad("B", "C"),
    ];
    let mut text = board(Shape::Rect(0.0, 0.0, 30.0, 20.0), &parts);
    let close = text.rfind(')').unwrap();
    text.insert_str(
        close,
        "(segment (start 10 10) (end 20 10) (width 0.25) (layer \"F.Cu\") (net 2) (uuid \"t1\"))\n(via (at 15 10) (size 0.6) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") (net 2) (uuid \"v1\"))\n(zone (net 2) (net_name \"B\") (layer \"F.Cu\") (uuid \"z1\") (attr (teardrop (type padvia))) (polygon (pts (xy 10 9) (xy 11 10) (xy 10 11))))\n",
    );
    let board_path = root.join("board.kicad_pcb");
    std::fs::write(&board_path, &text).unwrap();
    std::fs::write(root.join("board.kicad_pro"), "{\"board\":{}}").unwrap();
    std::fs::write(root.join("board.kicad_dru"), "(version 1)\n").unwrap();
    std::fs::create_dir(root.join(".konnect")).unwrap();
    std::fs::write(root.join(".konnect/design_intent.json"), "{}").unwrap();
    (dir, board_path, text)
}

/// Every file below `root` with its bytes.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    files
}

#[test]
fn the_scratch_project_is_stripped_named_like_the_board_and_beside_its_project_files() {
    let (_dir, board_path, text) = project_folder();
    let original = board_path.parent().unwrap().to_path_buf();
    let before = snapshot(&original);
    let state = tempfile::tempdir().unwrap();
    let folder = run_folder(state.path(), &text, "1").unwrap();
    let copy = scratch_project(&folder, &board_path, &text).unwrap();
    assert_eq!(copy, folder.join("project").join("board.kicad_pcb"));
    let written = std::fs::read_to_string(&copy).unwrap();
    assert_eq!(written, strip_routing(&text).unwrap());
    assert!(text.contains("(segment") && text.contains("(via"));
    assert!(!written.contains("(segment") && !written.contains("(via"));
    assert!(text.contains("(teardrop") && !written.contains("(teardrop"));
    for name in [
        "board.kicad_pro",
        "board.kicad_dru",
        ".konnect/design_intent.json",
    ] {
        assert!(folder.join("project").join(name).is_file(), "{name}");
    }
    assert_eq!(
        snapshot(&original),
        before,
        "the original folder is never written"
    );
}

// ─── The plan (T4, T7, T8) ───────────────────────────────────────────────────

#[test]
fn the_plan_is_the_bench_search_with_the_locks_held_and_only_c_routed() {
    let setup = setup(1);
    let root = setup.root.join("root.kicad_sch");
    let plan = tool_plan(
        &setup.root.join("run"),
        setup.plan.route.clone(),
        Some(root.clone()),
        6,
    );
    let mut bench = Matrix::bench().only(&[Family::Force, Family::Schematic, Family::Converge]);
    bench.converge = Some((ConvergeParams::measured(0), OuterRule::measured()));
    assert_eq!(plan.matrix, bench);
    assert_eq!(plan.hold, HoldRules::locks_only());
    assert!(plan.caller_held.is_empty());
    assert_eq!(plan.reference_calls, 0);
    assert_eq!(plan.ai_board, None);
    assert_eq!(plan.best_per_family, ROUTED_CANDIDATES);
    assert_eq!(plan.drawn_per_family, 0);
    assert_eq!(plan.routed, Routed::ConvergeOnly);
    assert_eq!(plan.schematic, Some(root));
    assert_eq!(plan.threads, 6);
    assert_eq!(plan.time_limit, TIME_GUARD);
}

// ─── The whole call with a fake router (T8, T9, T10, T11) ────────────────────

/// The tool's plan on the search tests' fixture, at family C's reduced stage length and
/// with a small matrix, so a run takes seconds.
fn fast_plan(setup: &Setup) -> Plan {
    let mut plan = tool_plan(&setup.plan.folder, setup.plan.route.clone(), None, 2);
    plan.matrix = small_matrix().only(&[Family::Force, Family::Schematic, Family::Converge]);
    plan.matrix.converge = Some((quick(0), OuterRule::measured()));
    plan
}

/// What the live read of an unrouted board gives.
fn unrouted_board() -> Planned {
    Planned {
        sha256: "0".repeat(64),
        routing_items: 0,
    }
}

async fn call(plan: &Plan, router: Arc<Fake>) -> Value {
    plan_and_judge(
        plan,
        router,
        Path::new("/boards/board.kicad_pcb"),
        &unrouted_board(),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plan_routes_only_family_c_and_returns_the_rank_one_winner_as_apply_arguments() {
    let setup = setup(1);
    let plan = fast_plan(&setup);
    let router = Arc::new(fake(0, good));
    let body = call(&plan, router.clone()).await;
    assert_eq!(body["status"], "planned", "{body:#}");
    let called = router.called_ids();
    // At least two calls, so "the best rank" is a choice the test can see.
    assert!(
        called.len() >= 2 && called.len() <= ROUTED_CANDIDATES,
        "{called:?}"
    );
    assert_eq!(body["search"]["routing_calls"], called.len());

    // The arguments are set_component_placements' own, for the user's board, and they
    // make exactly the judged board from the input.
    assert_eq!(body["apply_with"]["tool"], "set_component_placements");
    let arguments = &body["apply_with"]["arguments"];
    assert_eq!(arguments["board"], "/boards/board.kicad_pcb");
    let placements: Vec<IpcFootprintPlacement> = arguments["placements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| IpcFootprintPlacement {
            reference: p["reference"].as_str().unwrap().to_string(),
            x: p["x"].as_f64().unwrap(),
            y: p["y"].as_f64().unwrap(),
            rotation: p["rotation"].as_f64().unwrap(),
        })
        .collect();
    assert!(!placements.is_empty());
    assert_eq!(body["winner"]["moved_parts"], placements.len());
    let (model, _) = search_model(&plan).unwrap();
    // What moved, against the board as read: the same count and verdict the
    // response gives.
    let moves = moves_of(&model, &placements);
    assert_eq!(body["winner"]["moves_total"], moves.len());
    assert_eq!(body["winner"]["change"], change_of(&moves).name());
    assert_eq!(
        body["winner"]["moves"].as_array().unwrap().len(),
        moves.len().min(MAX_LINES)
    );
    let applied = placements_applied_to(&model.text, &placements).unwrap();
    let file: Value =
        serde_json::from_str(&std::fs::read_to_string(plan.folder.join("search.json")).unwrap())
            .unwrap();
    let id: usize = body["winner"]["candidate"].as_str().unwrap()[1..]
        .parse()
        .unwrap();
    let judged = file["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap();
    assert_eq!(judged["rank"], 1);
    assert_eq!(judged["board_sha256"], sha256_hex(applied.as_bytes()));
    assert!(called.contains(&id));

    // The response's numbers and records.
    assert_eq!(body["winner"]["rank"], 1);
    assert_eq!(body["winner"]["gates_pass"], true);
    assert!(body["winner"]["cost"]["end"].is_number());
    assert!(body["winner"]["screen"]["airwire_mm"].is_number());
    assert_eq!(body["candidates"].as_array().unwrap().len(), called.len());
    assert_eq!(body["records"], plan.folder.display().to_string());
    assert!(body["search"]["starts_taken"].as_u64().unwrap() >= 1);
    assert!(body.get("reasons").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_winner_that_fails_a_gate_is_planned_with_each_failure_named() {
    let setup = setup(1);
    let plan = fast_plan(&setup);
    let body = call(&plan, Arc::new(fake(0, |_, _| Step::Diagnosed))).await;
    assert_eq!(body["status"], "planned_with_gate_failures", "{body:#}");
    let lines: Vec<&str> = body["winner"]["gate_failures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap())
        .collect();
    assert!(
        lines.iter().any(|line| line.starts_with("G1: ")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.starts_with("P2: clearance")),
        "{lines:?}"
    );
    assert_eq!(body["winner"]["gates_pass"], false);
    assert!(body["apply_with"]["arguments"]["placements"].is_array());
    let steps = body["next_steps"].to_string();
    assert!(steps.contains("fails a gate"), "{steps}");
}

#[tokio::test(flavor = "multi_thread")]
async fn no_routed_candidate_gives_no_placements_and_the_reasons() {
    let setup = setup(1);
    let plan = fast_plan(&setup);
    let body = call(&plan, Arc::new(fake(0, |_, _| Step::Export))).await;
    assert_eq!(body["status"], "not_routed", "{body:#}");
    assert!(body.get("apply_with").is_none());
    assert!(body.get("winner").is_none());
    let reasons = body["reasons"].to_string();
    assert!(reasons.contains("the export refused"), "{reasons}");
}

#[tokio::test(flavor = "multi_thread")]
async fn no_start_is_no_candidate_and_nothing_is_routed() {
    let setup = setup(1);
    let mut plan = fast_plan(&setup);
    plan.matrix.force_seeds.clear();
    plan.matrix.schematic_seeds.clear();
    let router = Arc::new(fake(0, good));
    let body = call(&plan, router.clone()).await;
    assert_eq!(body["status"], "no_candidate", "{body:#}");
    assert!(router.called_ids().is_empty());
    assert!(body.get("apply_with").is_none());
    assert!(body["reasons"].to_string().contains("no start"), "{body:#}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_winner_whose_board_is_not_the_judged_one_is_refused() {
    let setup = setup(1);
    let plan = fast_plan(&setup);
    let mut outcome = search(&plan, Arc::new(fake(0, good))).await.unwrap();
    let model = outcome.model.clone();
    assert!(verdict_of(&model, &outcome).unwrap().winner.is_some());
    let winner = outcome
        .records
        .iter_mut()
        .filter(|r| r.group() == "C" && r.outcome == "routed")
        .min_by_key(|r| r.rank)
        .unwrap();
    winner.board_sha256 = Some("0".repeat(64));
    let error = verdict_of(&model, &outcome).unwrap_err();
    assert!(
        format!("{error:#}").contains("not the recorded one"),
        "{error:#}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_not_routed_verdict_gives_the_called_records_reasons_only() {
    // Every call refused: family C's other records were only not among the first three,
    // so none of them may lend the verdict a reason. One of them is made a plain
    // screened-out record that passes P1 (on this small board they are often duplicates,
    // which carry their own reason).
    let setup = setup(1);
    let plan = fast_plan(&setup);
    let mut outcome = search(&plan, Arc::new(fake(0, |_, _| Step::Export)))
        .await
        .unwrap();
    let other = outcome
        .records
        .iter_mut()
        .find(|r| r.group() == "C" && !was_sent(r.outcome))
        .expect("family C has more records than the three it routes");
    other.outcome = "screened_out";
    other.reason = None;
    let mut key = other.screen_key.expect("a screened record");
    key[0] = 0;
    other.screen_key = Some(key);
    let verdict = verdict_of(&outcome.model.clone(), &outcome).unwrap();
    assert_eq!(verdict.status, "not_routed");
    let reasons = verdict.reasons.join(" | ");
    assert!(reasons.contains("the export refused"), "{reasons}");
    assert!(!reasons.contains("fails P1"), "{reasons}");
}

#[test]
fn every_tool_the_next_steps_name_is_registered() {
    let names = [
        "create_checkpoint",
        "set_component_placements",
        "score_placement",
        "export_specctra_dsn",
        "route_specctra_dsn",
    ];
    let registered: BTreeSet<&str> = crate::router::registry::ALL_TOOLSETS
        .iter()
        .flat_map(|toolset| crate::router::registry::tools_for(toolset.name).unwrap())
        .map(|tool| tool.name)
        .collect();
    let verdict = Verdict {
        status: "planned_with_gate_failures",
        winner: Some((0, Vec::new())),
        gate_failures: vec!["G1: x".to_string()],
        reasons: Vec::new(),
    };
    let setup = setup(1);
    let steps = next_steps(&verdict, &setup.plan, &unrouted_board()).join(" ");
    for name in names {
        assert!(steps.contains(name), "{name} is named");
        assert!(registered.contains(name), "{name} is registered");
    }
}

#[test]
fn a_board_without_decoupling_pairs_gets_the_note_and_one_with_pairs_does_not() {
    // Two resistors: no capacitor, so no pair.
    let parts = [
        Fp::new("R1", 10.0, 10.0).lib("Test:R").two_pad("A", "B"),
        Fp::new("R2", 20.0, 10.0)
            .lib("Test:R")
            .pad(pad("1", -1.0, 0.0, Some("B")))
            .pad(pad("2", 1.0, 0.0, Some("C"))),
    ];
    let mut bare = setup(1);
    std::fs::write(
        &bare.plan.route.board,
        board(Shape::Rect(0.0, 0.0, 30.0, 20.0), &parts),
    )
    .unwrap();
    bare.plan.hold = HoldRules::locks_only();
    let (model, schematic) = search_model(&bare.plan).unwrap();
    assert!(model.intent.pairs.is_empty());
    let reason = schematic.err();
    let joined = notes(&model, reason.as_deref(), &bare.plan, &unrouted_board()).join(" ");
    assert!(joined.contains("No decoupling pair"), "{joined}");
    assert!(joined.contains("schematic could not be read"), "{joined}");
    assert!(!joined.contains("tracks, arcs, vias"), "{joined}");

    // An IC and its capacitor on VCC (the pair of `place_held`'s own test), and a routed
    // board.
    let paired_parts = [
        Fp::new("U1", 20.0, 20.0)
            .court(3.0, 3.0)
            .pad(pad("1", -2.0, 0.0, Some("VCC")))
            .pad(pad("2", 2.0, 0.0, Some("GND"))),
        Fp::new("C1", 14.0, 20.0)
            .value("100nF")
            .two_pad("VCC", "GND"),
    ];
    let mut paired = setup(1);
    std::fs::write(
        &paired.plan.route.board,
        board(Shape::Rect(0.0, 0.0, 60.0, 40.0), &paired_parts),
    )
    .unwrap();
    paired.plan.hold = HoldRules::locks_only();
    let (model, _) = search_model(&paired.plan).unwrap();
    assert!(!model.intent.pairs.is_empty());
    let routed = Planned {
        sha256: "0".repeat(64),
        routing_items: 12,
    };
    let joined = notes(&model, None, &paired.plan, &routed).join(" ");
    assert!(!joined.contains("No decoupling pair"), "{joined}");
    assert!(!joined.contains("schematic could not be read"), "{joined}");
    assert!(joined.contains("12 tracks, arcs, vias"), "{joined}");
}

#[test]
fn a_routed_board_is_counted_and_its_old_routing_named_before_the_apply() {
    let (_dir, _board, text) = project_folder();
    let planned = Planned::of(&text).unwrap();
    assert_eq!(
        planned.routing_items, 3,
        "one segment, one via and one teardrop"
    );
    assert_eq!(planned.sha256, sha256_hex(text.as_bytes()));
    let verdict = Verdict {
        status: "planned",
        winner: Some((0, Vec::new())),
        gate_failures: Vec::new(),
        reasons: Vec::new(),
    };
    let setup = setup(1);
    let routed = next_steps(&verdict, &setup.plan, &planned).join(" ");
    assert!(routed.contains("remove the old tracks"), "{routed}");
    let bare = next_steps(&verdict, &setup.plan, &unrouted_board()).join(" ");
    assert!(!bare.contains("remove the old tracks"), "{bare}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_input_check_that_fails_routes_nothing_and_says_so() {
    let setup = setup(1);
    let plan = fast_plan(&setup);
    let router = Arc::new(fake(0, good).with_failing_input_check());
    let body = call(&plan, router.clone()).await;
    assert_eq!(body["status"], "not_routed", "{body:#}");
    assert!(router.called_ids().is_empty());
    assert_eq!(body["search"]["routing_calls"], 0);
    assert!(body["candidates"].as_array().unwrap().is_empty());
    let reasons = body["reasons"].to_string();
    assert!(reasons.contains("the routing phase stopped"), "{reasons}");
    assert!(!reasons.contains("fails P1"), "{reasons}");
    assert!(body["search"]["routing_stopped"].is_string());
}

// ─── End to end on a public fixture (ignored) ────────────────────────────────

/// The whole plan with real fastroute, kicad-cli and judge on the public placement
/// fixture (`konnect-sexp/tests/fixtures/placement`), at effort m. Only the live read is
/// replaced: the rules are the fixture project's Default class for every net with a pad,
/// which is what KiCad resolves for it (it names no other class). Needs
/// `KONNECT_FASTROUTE`, and scratch `KICAD_CONFIG_HOME` and `KONNECT_STATE_DIR`. Writes the
/// response to `<run folder>/e2e.json`.
#[test]
#[ignore = "starts fastroute and kicad-cli: needs KONNECT_FASTROUTE and scratch KICAD_CONFIG_HOME and KONNECT_STATE_DIR"]
fn the_whole_plan_on_the_public_placement_fixture() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../konnect-sexp/tests/fixtures/placement")
        .canonicalize()
        .unwrap();
    let user = tempfile::tempdir().unwrap();
    let user = user.path().canonicalize().unwrap();
    for name in [
        "placement_fixture.kicad_pcb",
        "placement_fixture.kicad_pro",
        "placement_fixture.kicad_sch",
    ] {
        std::fs::copy(fixture.join(name), user.join(name)).unwrap();
    }
    let board_path = user.join("placement_fixture.kicad_pcb");
    let text = std::fs::read_to_string(&board_path).unwrap();
    let tree = konnect_sexp::parse_sexp(&text).unwrap();
    let index = konnect_sexp::board::PcbConnectivityIndex::build(&tree);
    let rules: IpcEffectiveRoutingRules = index
        .nets()
        .into_iter()
        .filter(|net| !net.is_empty() && !index.pads_of_net(net).is_empty())
        .map(|net| {
            (
                net.to_string(),
                IpcRoutingRules {
                    class_name: "Default".to_string(),
                    constituents: Vec::new(),
                    track_width_mm: Some(0.2),
                    clearance_mm: Some(0.2),
                    via_diameter_mm: Some(0.6),
                    via_drill_mm: Some(0.3),
                },
            )
        })
        .collect();
    assert!(!rules.is_empty());
    let state =
        PathBuf::from(std::env::var("KONNECT_STATE_DIR").expect("a scratch KONNECT_STATE_DIR"));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let folder = run_folder(&state, &text, &stamp()).unwrap();
    let binary = runtime
        .block_on(locate_fastroute(None, &folder))
        .unwrap()
        .unwrap_or_else(|refusal| panic!("fastroute: {refusal:?}"));
    let project_board = scratch_project(&folder, &board_path, &text).unwrap();
    let kicad_cli = crate::kicad_install::find_cli("")
        .expect("kicad-cli is installed")
        .display()
        .to_string();
    let kicad_config_home = folder.join("kicad-config");
    std::fs::create_dir(&kicad_config_home).unwrap();
    let inputs = RouteInputs {
        options: export_options(&project_board),
        board: project_board,
        rules,
        effort: Effort::M,
        fastroute: binary.path.clone(),
        kicad_cli: kicad_cli.clone(),
        kicad_config_home,
    };
    let plan = tool_plan(
        &folder,
        inputs.clone(),
        Some(board_path.with_extension("kicad_sch")),
        std::thread::available_parallelism().map_or(1, |n| n.get()),
    );
    let ctx = isolated_context(&folder.join("isolation"), &kicad_cli).unwrap();
    let before = snapshot(&user);
    let planned = Planned::of(&text).unwrap();
    let body = runtime
        .block_on(plan_and_judge(
            &plan,
            Arc::new(PlaceRouteCall { inputs, ctx }),
            &board_path,
            &planned,
        ))
        .unwrap();
    std::fs::write(
        folder.join("e2e.json"),
        serde_json::to_string_pretty(&body).unwrap(),
    )
    .unwrap();
    assert_eq!(
        snapshot(&user),
        before,
        "the user's folder is never written"
    );
    let status = body["status"].as_str().unwrap();
    assert!(
        matches!(status, "planned" | "planned_with_gate_failures"),
        "{body:#}"
    );
    assert!(
        body["search"]["routing_calls"].as_u64().unwrap() >= 1,
        "{body:#}"
    );
    assert!(!body["apply_with"]["arguments"]["placements"]
        .as_array()
        .unwrap()
        .is_empty());
}

// ─── What the winner changes (the honest summary) ────────────────────────────

/// The demo model's part `reference` at its own position, moved by `dx` and turned by
/// `turn` degrees.
fn nudged(model: &BoardModel, reference: &str, dx: f64, turn: f64) -> IpcFootprintPlacement {
    let part = model
        .parts
        .iter()
        .find(|part| part.reference.as_deref() == Some(reference))
        .unwrap();
    IpcFootprintPlacement {
        reference: reference.to_string(),
        x: part.at.0 + dx,
        y: part.at.1,
        rotation: part.rotation_deg + turn,
    }
}

#[test]
fn the_moves_list_only_parts_that_move_or_turn_largest_first() {
    let model =
        crate::place_generators::tests::model_of(&crate::place_generators::tests::demo_text(), &[]);
    let placements = vec![
        nudged(&model, "U1", 0.0, 0.0),
        nudged(&model, "R1", 0.25, 0.0),
        nudged(&model, "R2", -2.0, 0.0),
        nudged(&model, "R3", 0.0, 90.0),
        // A full turn is no turn, and float noise below a full turn is none either.
        nudged(&model, "R4", 0.0, 360.0),
        nudged(&model, "R5", 0.0, -1e-9),
    ];
    let moves = moves_of(&model, &placements);
    let names: Vec<&str> = moves.iter().map(|m| m.reference.as_str()).collect();
    assert_eq!(names, ["R2", "R1", "R3"], "{moves:?}");
    assert!((moves[0].distance_mm - 2.0).abs() < 1e-9);
    assert!((moves[0].dx_mm + 2.0).abs() < 1e-9);
    assert_eq!(moves[2].rotation_deg, 90.0);
    assert_eq!(moves[2].distance_mm, 0.0);
    assert_eq!(change_of(&moves), Change::Placement);
}

#[test]
fn a_plan_of_nudges_is_a_small_change_and_says_so_first() {
    let model =
        crate::place_generators::tests::model_of(&crate::place_generators::tests::demo_text(), &[]);
    // Two parts by 0.25 mm: the run the owner reported.
    let small = moves_of(
        &model,
        &[
            nudged(&model, "R1", 0.25, 0.0),
            nudged(&model, "R2", -0.25, 0.0),
        ],
    );
    assert_eq!(change_of(&small), Change::Small);
    // At the bound it is still small; just past it, or with a turn, it is not.
    let at_bound = moves_of(&model, &[nudged(&model, "R1", SMALL_MOVE_MM, 0.0)]);
    assert_eq!(change_of(&at_bound), Change::Small);
    let past = moves_of(&model, &[nudged(&model, "R1", SMALL_MOVE_MM + 0.01, 0.0)]);
    assert_eq!(change_of(&past), Change::Placement);
    let turned = moves_of(&model, &[nudged(&model, "R1", 0.1, 180.0)]);
    assert_eq!(change_of(&turned), Change::Placement);
    // Nothing moves: none, and its own line.
    let none = moves_of(&model, &[nudged(&model, "U1", 0.0, 0.0)]);
    assert_eq!(change_of(&none), Change::None);
    assert!(Change::None
        .line(&none)
        .unwrap()
        .contains("nothing to apply"));
    assert_eq!(Change::Placement.line(&past), None);

    let line = Change::Small.line(&small).unwrap();
    assert!(
        line.contains("2 part(s)") && line.contains("0.25 mm"),
        "{line}"
    );
    let mut notes = vec!["old note".to_string()];
    let mut steps = vec!["old step".to_string()];
    add_change_note(&mut notes, &mut steps, Some(line.clone()));
    assert_eq!(notes[0], line);
    assert!(
        steps[0].starts_with(&line) && steps[0].contains("decide"),
        "{steps:?}"
    );
    assert_eq!(notes[1], "old note");
    assert_eq!(steps[1], "old step");
    // A real placement adds nothing.
    add_change_note(&mut notes, &mut steps, None);
    assert_eq!((notes.len(), steps.len()), (2, 2));
}

// ─── Runs in the background (Row AP2) ────────────────────────────────────────

#[test]
fn a_call_starts_a_run_with_board_or_reports_one_with_run_id_never_both() {
    let start = read_request(&json!({"board": "/x/board.kicad_pcb", "effort": "h"})).unwrap();
    assert!(matches!(
        start,
        Request::Start(Call {
            effort: Effort::H,
            ..
        })
    ));
    let report = read_request(&json!({"run_id": "1-0123456789ab", "wait_s": 600})).unwrap();
    assert_eq!(
        report,
        Request::Report {
            run_id: "1-0123456789ab".into(),
            wait: Duration::from_secs(600)
        }
    );
    let report = read_request(&json!({"run_id": "1-0123456789ab"})).unwrap();
    assert_eq!(
        report,
        Request::Report {
            run_id: "1-0123456789ab".into(),
            wait: Duration::ZERO
        }
    );
    for args in [
        json!({"run_id": "1-0123456789ab", "board": "/x/board.kicad_pcb"}),
        json!({"run_id": "1-0123456789ab", "effort": "m"}),
        json!({"run_id": "1-0123456789ab", "fastroute_path": "/bin/fr"}),
        json!({"run_id": "1-0123456789ab", "wait_s": 601}),
        json!({"run_id": "1-0123456789ab", "wait_s": -1}),
        json!({"run_id": "1-0123456789ab", "wait_s": 1.5}),
        json!({"run_id": ""}),
        json!({"run_id": 7}),
        json!({"board": "/x/board.kicad_pcb", "wait_s": 10}),
        json!({}),
    ] {
        let refusal = read_request(&args).unwrap_err();
        assert!(refusal.is_error, "{args}");
    }
}

fn text_of(result: &CallToolResult) -> String {
    let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
        panic!("a text result");
    };
    text.clone()
}

fn body_of(result: &CallToolResult) -> Value {
    serde_json::from_str(&text_of(result)).expect("a JSON result")
}

/// A router whose calls each wait for a permit, so a test sees the run mid-routing.
struct Gated {
    inner: Fake,
    gate: Arc<tokio::sync::Semaphore>,
}

impl RouteCall for Gated {
    fn route(
        &self,
        id: usize,
        board_text: String,
        folder: PathBuf,
    ) -> crate::route_search::BoxFuture<(RouteResult, crate::place_search::Timings)> {
        let gate = Arc::clone(&self.gate);
        let call = self.inner.route(id, board_text, folder);
        Box::pin(async move {
            let _permit = gate.acquire().await.unwrap();
            call.await
        })
    }

    fn unrouted(
        &self,
        board_text: String,
        folder: PathBuf,
    ) -> crate::route_search::BoxFuture<anyhow::Result<BTreeMap<String, usize>>> {
        self.inner.unrouted(board_text, folder)
    }
}

/// A run of `fast_plan` started in its own state folder, as the handler starts it.
struct Started {
    _state: tempfile::TempDir,
    state: PathBuf,
    jobs: Jobs,
    run_id: String,
    body: Value,
}

fn start_fast_run(setup: &Setup, routing: Arc<dyn RouteCall>, text: &str) -> Started {
    let dir = tempfile::tempdir().unwrap();
    let state = std::fs::canonicalize(dir.path()).unwrap();
    let board = Path::new("/boards/board.kicad_pcb");
    let stamp = "1700000000000";
    let run_id = run_name(text, stamp);
    let claim = claim_board(&state, &PLACEMENT_RUNS, board, &run_id)
        .unwrap()
        .unwrap();
    let mut plan = fast_plan(setup);
    plan.folder = run_folder(&state, text, stamp).unwrap();
    let jobs = Jobs::default();
    let body = start_run(
        &jobs,
        claim,
        Background {
            plan,
            routing,
            board: board.to_path_buf(),
            planned: unrouted_board(),
            fastroute: json!({ "path": "/fake/fastroute" }),
        },
    )
    .unwrap();
    Started {
        _state: dir,
        state,
        jobs,
        run_id,
        body,
    }
}

/// `body` without what differs between two runs of one plan: the run's id and binary,
/// its wall times and its folder.
fn without_run(mut body: Value) -> Value {
    let object = body.as_object_mut().unwrap();
    object.remove("run_id");
    object.remove("fastroute");
    object.remove("records");
    body["search"]["elapsed_s"] = json!(0);
    body["search"]["converge_s"] = json!(0);
    body
}

#[tokio::test(flavor = "multi_thread")]
async fn a_background_run_gives_the_synchronous_result_and_keeps_it_for_a_restart() {
    let setup = setup(1);
    let synchronous = call(&fast_plan(&setup), Arc::new(fake(0, good))).await;
    assert_eq!(synchronous["status"], "planned", "{synchronous:#}");

    let run = start_fast_run(&setup, Arc::new(fake(0, good)), "board text");
    assert_eq!(run.body["status"], "started");
    assert_eq!(run.body["run_id"], run.run_id.as_str());
    let folder = run_folder_of(&run.state, &PLACEMENT_RUNS, &run.run_id);
    assert_eq!(run.body["records"], folder.display().to_string());
    assert!(run.body["next_steps"][0]
        .as_str()
        .unwrap()
        .contains(&format!("\"run_id\": \"{}\"", run.run_id)));

    let done = report_result(&run.jobs, &run.state, &run.run_id, crate::jobs::MAX_WAIT).await;
    assert!(!done.is_error);
    let body = body_of(&done);
    assert_eq!(body["run_id"], run.run_id.as_str());
    assert_eq!(body["fastroute"], json!({ "path": "/fake/fastroute" }));
    assert_eq!(body["records"], folder.display().to_string());
    assert!(body["apply_with"]["arguments"]["placements"].is_array());
    assert!(body["winner"]["moves"].is_array() && body["winner"]["change"].is_string());
    assert_eq!(without_run(body.clone()), without_run(synchronous));

    // A restarted server has no run in memory: the result is read from the folder.
    let again = report_result(&Jobs::default(), &run.state, &run.run_id, Duration::ZERO).await;
    assert_eq!(body_of(&again), body);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_run_reports_its_phase_counts_and_effort() {
    let setup = setup(1);
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let router = Gated {
        inner: fake(0, good),
        gate: Arc::clone(&gate),
    };
    let run = start_fast_run(&setup, Arc::new(router), "board text");
    // Every route call waits at the gate, so the run stops in its routing phase.
    let started = std::time::Instant::now();
    let running = loop {
        let report =
            body_of(&report_result(&run.jobs, &run.state, &run.run_id, Duration::ZERO).await);
        assert_eq!(report["status"], "running", "{report:#}");
        if report["phase"] == "routing" {
            break report;
        }
        assert!(started.elapsed() < Duration::from_secs(120), "{report:#}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let candidates = &running["candidates"];
    let total = candidates["generate_total"].as_u64().unwrap();
    assert!(total > 0, "{running:#}");
    assert_eq!(candidates["generated"], total, "phase 1 is over");
    assert!(candidates["converge_runs"].as_u64().unwrap() >= 1);
    let to_route = candidates["to_route"].as_u64().unwrap();
    assert!(
        (1..=ROUTED_CANDIDATES as u64).contains(&to_route),
        "{running:#}"
    );
    assert_eq!(candidates["judged"], 0);
    assert_eq!(candidates["routed"], 0);
    assert!(running["effort_level_reached"].is_u64());
    assert_eq!(running["routing_effort"], "m");
    assert!(running["elapsed_s"].as_f64().unwrap() >= 0.0);
    assert_eq!(running["run_id"], run.run_id.as_str());
    assert_eq!(running["board"], "/boards/board.kicad_pcb");
    assert!(running.get("served_by_pid").is_none());

    // A second start on the same board is refused while it runs, and names it.
    let busy = claim_board(
        &run.state,
        &PLACEMENT_RUNS,
        Path::new("/boards/board.kicad_pcb"),
        "1700000000001-0123456789ab",
    )
    .unwrap()
    .unwrap_err();
    let refusal = busy_result(&busy);
    assert!(refusal.is_error);
    assert!(text_of(&refusal).contains(&format!("run {}", run.run_id)));

    gate.add_permits(64);
    let done =
        body_of(&report_result(&run.jobs, &run.state, &run.run_id, crate::jobs::MAX_WAIT).await);
    assert_eq!(done["status"], "planned", "{done:#}");
    // The last progress is kept with the run: every call judged, as the result counts.
    let folder = run_folder_of(&run.state, &PLACEMENT_RUNS, &run.run_id);
    let record: Value =
        serde_json::from_str(&std::fs::read_to_string(folder.join("run.json")).unwrap()).unwrap();
    assert_eq!(record["state"], "done");
    assert_eq!(record["progress"]["phase"], "writing");
    assert_eq!(
        record["progress"]["candidates"]["judged"],
        done["search"]["routing_calls"]
    );
    assert_eq!(
        record["progress"]["candidates"]["routed"],
        done["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|line| line["outcome"] == "routed")
            .count()
    );
    assert_eq!(
        record["progress"]["effort_level_reached"],
        done["search"]["effort_level_reached"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_report_names_what_it_cannot_read() {
    let state = tempfile::tempdir().unwrap();
    for id in ["../etc", "1700000000000-0123456789ab"] {
        let report = report_result(&Jobs::default(), state.path(), id, Duration::ZERO).await;
        assert!(report.is_error, "{id}");
        let text = text_of(&report);
        assert!(text.contains("run_id"), "{text}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_or_interrupted_run_is_an_error_result_with_its_status() {
    let state = tempfile::tempdir().unwrap();
    let state = std::fs::canonicalize(state.path()).unwrap();
    let id = "1700000000000-0123456789ab";
    let folder = run_folder_of(&state, &PLACEMENT_RUNS, id);
    std::fs::create_dir_all(&folder).unwrap();
    let mut record = json!({
        "run_id": id, "tool": "plan_convergent_placement", "board": "/boards/board.kicad_pcb",
        "state": "running", "pid": 1, "started_unix_ms": 1, "finished_unix_ms": null,
        "progress": {}, "error": null,
    });
    std::fs::write(folder.join("run.json"), record.to_string()).unwrap();
    let interrupted = report_result(&Jobs::default(), &state, id, Duration::ZERO).await;
    assert!(interrupted.is_error);
    let body = body_of(&interrupted);
    assert_eq!(body["status"], "interrupted", "{body:#}");
    assert_eq!(body["records"], folder.display().to_string());

    record["state"] = json!("failed");
    record["error"] = json!("the input board cannot be read");
    std::fs::write(folder.join("run.json"), record.to_string()).unwrap();
    let failed = report_result(&Jobs::default(), &state, id, Duration::ZERO).await;
    assert!(failed.is_error);
    let body = body_of(&failed);
    assert_eq!(body["status"], "failed", "{body:#}");
    assert_eq!(body["error"], "the input board cannot be read");
}
