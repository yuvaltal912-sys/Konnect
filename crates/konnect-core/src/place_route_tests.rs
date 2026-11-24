//! Tests of `place_route`: the strip, the rules, a candidate's project and DSN, the
//! routing call's arguments and its result read back, the unrouted check and the
//! isolated context. Nothing here starts fastroute or a live KiCad: the results the
//! reader is given are the ones `route_fastroute` builds (from a constructed search
//! outcome, and from the call's own refusals), and kicad-cli is a fake script.

use super::*;
use crate::fastroute::{Binary, Held};
use crate::place_held::tests::{board, pad, Fp, Shape as PartShape};
use crate::route_judge::{Baseline, Judged};
use crate::route_search::{
    EffortSource, Lanes, Order, Routed, RunRecord, SearchOutcome, SearchPlan, Shape,
};
use crate::tools::route_fastroute::{no_winner_result, prepare_call, result_json};
use sha2::{Digest, Sha256};
use std::time::Duration;

const POURS_BOARD: &str = include_str!("../tests/fixtures/specctra_pours_policy.kicad_pcb");
const BOARD_FILE: &str = "specctra_pours_policy.kicad_pcb";

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

// ─── strip_routing ───────────────────────────────────────────────────────────

/// A small board with every kind of top-level item the strip decides about.
fn routed_board() -> String {
    [
        "(kicad_pcb\n",
        "\t(version 20260206)\n",
        "\t(footprint \"A:B\"\n",
        // Quoted text that looks like a block, and a nested segment-like head: neither
        // is top level, so neither is touched.
        "\t\t(property \"Value\" \"(segment (start 0 0) \\\" (via\")\n",
        "\t\t(segment_like 1)\n",
        "\t)\n",
        "\t(gr_arc (start 1 1) (mid 2 2) (end 3 3) (layer \"Edge.Cuts\"))\n",
        "\t(segment (start 0 0) (end 1 1) (width 0.2) (layer \"F.Cu\") (net \"GND\"))\n",
        "\t(arc (start 0 0) (mid 1 1) (end 2 0) (width 0.2) (layer \"F.Cu\") (net \"GND\"))\n",
        "\t(via (at 5 5) (size 0.6) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") (net \"GND\"))\n",
        "\t(zone (net \"GND\") (layer \"F.Cu\") (name \"GND_TOP\") (polygon (pts (xy 0 0) (xy 9 0) (xy 9 9))))\n",
        "\t(zone (net \"GND\") (layer \"F.Cu\") (attr (teardrop (type padvia))) (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1))))\n",
        "\t(segment (start 3 3) (end 4 4) (width 0.2) (layer \"B.Cu\") (net \"VCC\"))\n",
        "\t(gr_line (start 0 0) (end 9 0) (layer \"Edge.Cuts\"))\n",
        ")\n",
    ]
    .concat()
}

#[test]
fn strip_routing_removes_tracks_vias_and_teardrops_only() {
    let stripped = strip_routing(&routed_board()).unwrap();
    // Exactly what remains, byte for byte: each removed block took the whitespace
    // before it, every other byte stayed.
    let expected = [
        "(kicad_pcb\n",
        "\t(version 20260206)\n",
        "\t(footprint \"A:B\"\n",
        "\t\t(property \"Value\" \"(segment (start 0 0) \\\" (via\")\n",
        "\t\t(segment_like 1)\n",
        "\t)\n",
        "\t(gr_arc (start 1 1) (mid 2 2) (end 3 3) (layer \"Edge.Cuts\"))",
        "\n\t(zone (net \"GND\") (layer \"F.Cu\") (name \"GND_TOP\") (polygon (pts (xy 0 0) (xy 9 0) (xy 9 9))))",
        "\n\t(gr_line (start 0 0) (end 9 0) (layer \"Edge.Cuts\"))\n",
        ")\n",
    ]
    .concat();
    assert_eq!(stripped, expected);
    // The result is a board, and stripping it again changes nothing.
    assert!(konnect_sexp::parse_sexp(&stripped).is_ok());
    assert_eq!(strip_routing(&stripped).unwrap(), stripped);
    // A board with nothing to strip comes back byte for byte.
    assert_eq!(strip_routing(POURS_BOARD).unwrap(), POURS_BOARD);
}

#[test]
fn strip_routing_errs_on_text_that_is_not_a_board() {
    assert!(strip_routing("(kicad_pcb (version 1)").is_err());
    assert!(strip_routing("(kicad_sch (version 1))").is_err());
    assert!(strip_routing("").is_err());
}

// ─── rules_from_json ─────────────────────────────────────────────────────────

fn rules_json(nets: &[&str]) -> String {
    // As `xtask/placement-bench/net_rules.py` writes it.
    let rule = |net: &str| {
        json!({
            "class_name": "Default",
            "constituents": ["Default"],
            "track_width_mm": 0.2,
            "clearance_mm": 0.2,
            "via_diameter_mm": 0.6,
            "via_drill_mm": 0.3,
            "net": net,
        })
    };
    let object: serde_json::Map<String, Value> = nets
        .iter()
        .map(|net| (net.to_string(), rule(net)))
        .collect();
    Value::Object(object).to_string()
}

#[test]
fn rules_from_json_refuses_a_net_it_lacks() {
    // The complete file is accepted (an extra key in an entry is ignored).
    let rules = rules_from_json(&rules_json(&["GND", "VCC"]), POURS_BOARD).unwrap();
    assert_eq!(rules.len(), 2);
    assert_eq!(rules["GND"].track_width_mm, Some(0.2));
    assert_eq!(rules["VCC"].constituents, vec!["Default".to_string()]);
    // A net that is not on the board is harmless.
    assert!(rules_from_json(&rules_json(&["GND", "VCC", "UNUSED"]), POURS_BOARD).is_ok());

    // A net with a pad and no entry is refused, and the refusal names it.
    let error = rules_from_json(&rules_json(&["GND"]), POURS_BOARD).unwrap_err();
    let error = format!("{error:#}");
    assert!(error.contains("VCC") && error.contains("1 net"), "{error}");
    // A file that is not the rules form is refused too.
    assert!(rules_from_json("[]", POURS_BOARD).is_err());
    assert!(rules_from_json("{\"GND\": {\"class_name\": \"Default\"}}", POURS_BOARD).is_err());
    assert!(rules_from_json(&rules_json(&["GND", "VCC"]), "(kicad_pcb").is_err());
}

// ─── export_options ──────────────────────────────────────────────────────────

#[test]
fn export_options_are_the_handlers_defaults_with_the_projects_clearances() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("b.kicad_pcb");
    std::fs::write(&board, POURS_BOARD).unwrap();

    // No project beside the board: no clearance.
    let options = export_options(&board);
    assert_eq!(options.copper_edge_clearance_um, None);
    assert_eq!(options.hole_clearance_um, None);
    assert!(options.widths_from_intent);
    assert!(!options.allow_duplicate_references);
    // Pours as the handler reads them with no `pours` argument: the policy default.
    let handler_default = crate::specctra::PourPolicy::from_argument(None).unwrap();
    assert_eq!(
        format!("{:?}", options.pours),
        format!("{handler_default:?}")
    );

    std::fs::write(
        dir.path().join("b.kicad_pro"),
        r#"{"board":{"design_settings":{"rules":{"min_copper_edge_clearance":0.3,"min_hole_clearance":0.25}}}}"#,
    )
    .unwrap();
    let options = export_options(&board);
    assert_eq!(options.copper_edge_clearance_um, Some(300));
    assert_eq!(options.hole_clearance_um, Some(250));
    // A value that is not above zero is none.
    std::fs::write(
        dir.path().join("b.kicad_pro"),
        r#"{"board":{"design_settings":{"rules":{"min_copper_edge_clearance":0,"min_hole_clearance":-1}}}}"#,
    )
    .unwrap();
    let options = export_options(&board);
    assert_eq!(options.copper_edge_clearance_um, None);
    assert_eq!(options.hole_clearance_um, None);
}

// ─── export_candidate ────────────────────────────────────────────────────────

/// The bench folder of one board: `bench/input/` holds the stripped project.
struct Bench {
    _dir: tempfile::TempDir,
    root: PathBuf,
    inputs: RouteInputs,
}

fn fake_kicad_cli(dir: &Path) -> String {
    crate::tools::cli::test_support::noop_cli(dir)
        .display()
        .to_string()
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let input = root.join("input");
    std::fs::create_dir_all(&input).unwrap();
    let board_path = input.join(BOARD_FILE);
    std::fs::write(&board_path, strip_routing(POURS_BOARD).unwrap()).unwrap();
    std::fs::write(
        input.join(BOARD_FILE.replace(".kicad_pcb", ".kicad_pro")),
        r#"{"board":{"design_settings":{"rules":{"min_copper_edge_clearance":0.3,"min_hole_clearance":0.25}}}}"#,
    )
    .unwrap();
    let tools = root.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    let inputs = RouteInputs {
        rules: rules_from_json(&rules_json(&["GND", "VCC"]), POURS_BOARD).unwrap(),
        options: export_options(&board_path),
        effort: Effort::M,
        fastroute: root.join("tools/no-such-fastroute"),
        kicad_cli: fake_kicad_cli(&tools),
        kicad_config_home: root.join("kicad-config"),
        board: board_path,
    };
    Bench {
        _dir: dir,
        root,
        inputs,
    }
}

/// Every file below `root` with its SHA-256, relative paths.
fn tree_hashes(root: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, at: &Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().display().to_string(),
                    sha(&std::fs::read(&path).unwrap()),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn an_exported_candidate_passes_the_judge_preflight() {
    let bench = bench();
    let text = std::fs::read_to_string(&bench.inputs.board).unwrap();
    let folder = bench.root.join("candidates").join("c001");
    let dsn = export_candidate(&bench.inputs, &folder, &text).unwrap();

    // The files: the project copy, the board's own name, the DSN and its manifest.
    assert_eq!(
        dsn,
        folder
            .canonicalize()
            .unwrap()
            .join("specctra_pours_policy.dsn")
    );
    let manifest_path = PathBuf::from(format!("{}.konnect.json", dsn.display()));
    assert!(dsn.is_file() && manifest_path.is_file());
    let copy = folder.join(BOARD_FILE);
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), text);
    assert!(folder.join("specctra_pours_policy.kicad_pro").is_file());
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    assert_eq!(
        manifest["board_path"].as_str().unwrap(),
        copy.canonicalize().unwrap().display().to_string()
    );
    assert_eq!(manifest["source_sha256"], sha(text.as_bytes()));

    // The judge's preflight accepts it, and finds the candidate's own board.
    let judge_root = bench.root.join("judge");
    let ctx = crate::route_judge::prepare(&dsn, &judge_root, &bench.inputs.kicad_cli)
        .expect("the preflight accepts the exported candidate");
    assert_eq!(ctx.board, copy.canonicalize().unwrap());
    assert_eq!(ctx.board_text, text);
    // The project's clearances reached the manifest through the options.
    assert_eq!(
        manifest["supported_profile"]["copper_edge_clearance_um"],
        300
    );

    // Only the copy's layout changed after the export: the same board as a tree, which
    // the preflight accepts since Row RJ (`board_identity::canonical_sha256`).
    std::fs::write(&copy, format!("{text} ")).unwrap();
    crate::route_judge::prepare(
        &dsn,
        &bench.root.join("judge-layout"),
        &bench.inputs.kicad_cli,
    )
    .expect("a change of layout alone is the same board");

    // One real change to the copy's board after the export: the preflight refuses.
    let changed = text.replacen("(title \"Test Board\")", "(title \"Test Board 2\")", 1);
    assert_ne!(
        changed, text,
        "the fixture's title is where the test expects it"
    );
    std::fs::write(&copy, changed).unwrap();
    let refusal = crate::route_judge::prepare(&dsn, &judge_root, &bench.inputs.kicad_cli)
        .expect_err("a board that is not the exported one is refused");
    assert!(refusal.reason.contains("SHA-256"), "{refusal:?}");
}

#[test]
fn export_candidate_writes_only_inside_its_folder() {
    let bench = bench();
    let text = std::fs::read_to_string(&bench.inputs.board).unwrap();
    let before = tree_hashes(&bench.root);
    let folder = bench.root.join("candidates").join("c002");
    export_candidate(&bench.inputs, &folder, &text).unwrap();
    let after = tree_hashes(&bench.root);

    // Nothing that was there changed; everything new is below the candidate's folder.
    for (path, hash) in &before {
        assert_eq!(after.get(path), Some(hash), "{path} changed");
    }
    for path in after.keys().filter(|path| !before.contains_key(*path)) {
        assert!(path.starts_with("candidates/c002/"), "{path} is outside");
    }
    assert!(after.len() > before.len());

    // A folder that exists is refused, and nothing in it is touched.
    let again = export_candidate(&bench.inputs, &folder, &text).unwrap_err();
    assert!(format!("{again:#}").contains("exists already"), "{again:#}");
    assert_eq!(tree_hashes(&bench.root), after);
}

#[test]
fn an_export_the_exporter_refuses_is_an_error_and_its_folder_stays() {
    let bench = bench();
    // A rules file with no entry for VCC: the exporter refuses to invent one.
    let mut inputs = bench.inputs.clone();
    inputs.rules.remove("VCC");
    let text = std::fs::read_to_string(&inputs.board).unwrap();
    let folder = bench.root.join("candidates").join("c003");
    assert!(export_candidate(&inputs, &folder, &text).is_err());
    // The folder stays as the failed export left it, for the record; no DSN is in it.
    assert!(folder.join(BOARD_FILE).is_file());
    assert!(!folder.join("specctra_pours_policy.dsn").exists());
}

// ─── the routing call ────────────────────────────────────────────────────────

#[test]
fn the_routing_call_names_the_effort_in_lower_case() {
    for (effort, name) in [
        (Effort::Fast, "fast"),
        (Effort::M, "m"),
        (Effort::H, "h"),
        (Effort::Xh, "xh"),
    ] {
        let args = routing_arguments("/x/a.dsn", "/x/route.ses", effort, "/x/fastroute");
        assert_eq!(args["effort"], name);
        assert_eq!(args["engine"], "fastroute");
        assert_eq!(args["dsn_path"], "/x/a.dsn");
        assert_eq!(args["ses_output_path"], "/x/route.ses");
        assert_eq!(args["fastroute_path"], "/x/fastroute");
        // The call reads exactly that name back as the same level.
        let read =
            crate::route_search::resolve_effort(Some(&args["effort"]), None).expect("a level");
        assert_eq!(read.0, effort);
        assert_eq!(read.1, EffortSource::Argument);
    }
}

#[tokio::test]
async fn a_call_that_cannot_find_its_binary_is_a_failed_call_not_a_panic() {
    let bench = bench();
    let text = std::fs::read_to_string(&bench.inputs.board).unwrap();
    let folder = bench.root.join("candidates").join("c004");
    let dsn = export_candidate(&bench.inputs, &folder, &text).unwrap();
    let ctx = isolated_context(&bench.root.join("isolation"), &bench.inputs.kicad_cli).unwrap();
    let result = route_candidate(ctx, &bench.inputs, &dsn, &folder).await;
    let RouteResult::Failed { stage, reason } = result else {
        panic!("expected a failed call, got {result:?}");
    };
    assert_eq!(stage, "call");
    assert!(!reason.is_empty());
    // The call ran no search: nothing is left in the candidate's folder but its files.
    assert!(!folder.join("route.ses").exists());
    assert!(!folder.join("route.search").exists());
}

// ─── read_route_result ───────────────────────────────────────────────────────

fn measures() -> Measures {
    Measures {
        unconnected: 0,
        violations: BTreeMap::new(),
        routed_share: Some(1.0),
        audit_complete: Some(true),
        acute: Some(0.0),
        right_angles: Some(0.0),
        copper_quality: Some(90.0),
        nets_undersized: Some(0.0),
        narrow_paths: Some(0.0),
        signal_mm: 100.0,
        power_mm: 20.0,
        pour_track_mm: 0.0,
        vias_off_pour: 3,
        below_floor: 0,
        width_rows: vec![],
        longest_below_mm: BTreeMap::new(),
        stitching_vias: 0,
        route_s: 12.34,
        lengths: None,
        pours: None,
        pours_error: None,
        reference_vias: 0,
        checklist: Default::default(),
    }
}

fn candidate_binary() -> Binary {
    Binary {
        path: PathBuf::from("/x/fastroute"),
        sha256: "ab".repeat(32),
        version: "fastroute 0.1.14".to_string(),
        source: "argument",
    }
}

fn plan() -> SearchPlan {
    SearchPlan {
        limits: Effort::M.limits(),
        lanes: Lanes {
            routers: 2,
            judges: 1,
        },
        order: Order::Fixed(vec![(Shape::AsExported, 1), (Shape::Omitted, 1)]),
        hang_cap: crate::route_search::HANG_CAP,
    }
}

fn record(
    index: usize,
    outcome: &'static str,
    routed: Option<Routed>,
    judged: Option<Judged>,
) -> RunRecord {
    RunRecord {
        index,
        shape: Shape::AsExported,
        seed: 7,
        launched_s: 0.04,
        routed_s: routed.as_ref().map(|_| 12.34),
        judged_s: judged.as_ref().map(|_| 20.06),
        route_s: routed.as_ref().map(|_| 12.3),
        judge_s: judged.as_ref().map(|_| 7.71),
        outcome,
        reason: (outcome == "route_failed").then(|| "the router crashed".to_string()),
        routed,
        judged,
    }
}

fn baseline() -> Baseline {
    Baseline {
        violations: [
            ("clearance".to_string(), 2),
            ("hole_clearance".to_string(), 1),
        ]
        .into(),
        unconnected: 41,
        pour_fills: None,
        pour_fills_error: None,
    }
}

fn outcome(runs: Vec<RunRecord>, winner: Option<usize>) -> SearchOutcome {
    let keys = runs
        .iter()
        .map(|record| record.judged.as_ref().map(|_| [0; 13]))
        .collect();
    let pour_keys = runs
        .iter()
        .map(|record| record.judged.as_ref().map(|_| [0; 17]))
        .collect();
    SearchOutcome {
        runs,
        winner,
        keys,
        pour_keys,
        rank: crate::pour_rank::RankMode::Routing,
        elapsed: Duration::from_millis(61_234),
        budget_overrun: None,
        stopped_launching: Some("the launch rule".to_string()),
        baseline: Ok(baseline()),
        pour_prune: None,
        reference_stitch: None,
    }
}

/// A finished search folder with one judged run (a winner), as the product leaves it.
struct Finished {
    _dir: tempfile::TempDir,
    root: PathBuf,
    folder: PathBuf,
    dsn: PathBuf,
    ses: PathBuf,
    outcome: SearchOutcome,
}

fn finished(gates: Gates) -> Finished {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let folder = root.join("route.search");
    let run = folder.join("run-000-as_exported-s7");
    std::fs::create_dir_all(run.join("full")).unwrap();
    std::fs::write(run.join("variant.dsn"), "(pcb board)").unwrap();
    std::fs::write(run.join("full/full.ses"), "(session board)").unwrap();
    let board = folder.join("judge/run-0/r0/board.kicad_pcb");
    std::fs::create_dir_all(board.parent().unwrap()).unwrap();
    std::fs::write(&board, "(kicad_pcb)").unwrap();
    let routed = Routed {
        index: 0,
        shape: Shape::AsExported,
        seed: 7,
        dsn: run.join("variant.dsn"),
        held: Held {
            ses: run.join("full/full.ses"),
            unrouted: 0,
            kept: "full",
            passes: vec![
                json!({ "name": "full", "outcome": "kept", "unrouted": 0, "seconds": 12.3 }),
            ],
            seconds: 12.3,
            untuned: None,
            tune: Default::default(),
            group_search: None,
            planes: None,
        },
        planes: vec![],
        folder: run,
    };
    let judged = Judged {
        measures: measures(),
        gates,
        board,
        rounds: vec![json!({ "round": 1, "open_before": 2, "open_after": 0 })],
        tune: None,
        reference_stitch: None,
        reference_checked: None,
    };
    let dsn = root.join("candidate.dsn");
    let ses = root.join("route.ses");
    std::fs::write(&dsn, "(pcb board)").unwrap();
    std::fs::write(&ses, "(session board)").unwrap();
    Finished {
        outcome: outcome(
            vec![record(0, "judged", Some(routed), Some(judged))],
            Some(0),
        ),
        _dir: dir,
        root,
        folder,
        dsn,
        ses,
    }
}

fn no_gate_failures() -> Gates {
    Gates {
        g1: vec![],
        g2: vec![],
        g3: vec![],
        g4: vec![],
    }
}

/// The success result as `search_and_write` returns it.
fn success_result(finished: &Finished) -> CallToolResult {
    let result = result_json(
        &finished.dsn,
        &finished.ses,
        &finished.folder,
        &candidate_binary(),
        (Effort::M, EffortSource::Argument),
        &plan(),
        &[Shape::AsExported, Shape::Omitted],
        &finished.outcome,
        &crate::tools::route_pairs::PairsPlan::default(),
        &crate::tools::route_lengths::LengthPlan::default(),
    );
    assert_eq!(result["success"], true, "{result}");
    CallToolResult::json(&result)
}

/// A call whose outcome judged nothing, with the error result the product builds for
/// it (`no_winner_result`), carrying its `search`.
fn no_winner(baseline: std::result::Result<Baseline, String>) -> CallToolResult {
    let dir = tempfile::tempdir().unwrap();
    let dsn = dir.path().join("board.dsn");
    std::fs::write(&dsn, "(pcb board)").unwrap();
    let args = json!({
        "dsn_path": dsn,
        "ses_output_path": dir.path().join("out.ses"),
        "effort": "m",
    });
    let call = prepare_call(&args)
        .unwrap()
        .expect("the arguments are accepted");
    let mut outcome = outcome(
        vec![
            record(0, "route_failed", None, None),
            record(1, "route_failed", None, None),
        ],
        None,
    );
    outcome.baseline = baseline;
    // The `search` of the result, as `result_json` of the same outcome reports it.
    let search = result_json(
        &dsn,
        &dir.path().join("out.ses"),
        &dir.path().join("out.search"),
        &candidate_binary(),
        (Effort::M, EffortSource::Argument),
        &plan(),
        &[Shape::AsExported, Shape::Omitted],
        &outcome,
        &crate::tools::route_pairs::PairsPlan::default(),
        &crate::tools::route_lengths::LengthPlan::default(),
    )["search"]
        .clone();
    assert!(search["winner"].is_null());
    let result = no_winner_result(
        &call,
        &outcome,
        search,
        &crate::tools::route_pairs::PairsPlan::default(),
        &crate::tools::route_lengths::LengthPlan::default(),
    );
    assert!(result.is_error);
    result
}

#[tokio::test]
async fn the_routing_record_carries_the_calls_differential_pairs() {
    // A call without pairs: both lists are there, empty, so a run shows no pair was passed.
    let done = finished(no_gate_failures());
    let RouteResult::Routed { call, .. } = read_route_result(&success_result(&done)) else {
        panic!("a winner is routed");
    };
    assert_eq!(call["differential_pairs"], json!([]));
    assert_eq!(call["differential_pairs_unresolved"], json!([]));
    assert!(call.get("differential_pairs_note").is_none(), "{call}");

    // A call that passed a pair and could not resolve another, with a note.
    let mut body: Value = serde_json::from_str(&text_of(&success_result(&done))).unwrap();
    body["differential_pairs"] = json!([{
        "positive": "CAN_H", "negative": "CAN_L",
        "dsn_positive": "CAN_H", "dsn_negative": "CAN_L", "gap_mm": 0.2,
    }]);
    body["differential_pairs_unresolved"] = json!([
        { "positive": "USB_P", "negative": "USB_N", "reason": "no such net" }
    ]);
    body["differential_pairs_note"] = json!("one pair used");
    let RouteResult::Routed { call, .. } = read_route_result(&CallToolResult::json(&body)) else {
        panic!("a winner is routed");
    };
    assert_eq!(call["differential_pairs"][0]["positive"], "CAN_H");
    assert_eq!(call["differential_pairs"][0]["gap_mm"], 0.2);
    assert_eq!(
        call["differential_pairs_unresolved"][0]["reason"],
        "no such net"
    );
    assert_eq!(call["differential_pairs_note"], "one pair used");
}

#[tokio::test]
async fn read_route_result_reads_the_winner_the_baseline_and_failures() {
    // A winner: its gates and measures, the baseline's violations, the status, the summary.
    let done = finished(no_gate_failures());
    let routed = read_route_result(&success_result(&done));
    let RouteResult::Routed {
        gates,
        measures: read,
        unrouted,
        status,
        call,
    } = routed
    else {
        panic!("expected a routed result, got {routed:?}");
    };
    assert!(gates.all_pass());
    assert_eq!(read, measures());
    assert_eq!(unrouted, baseline().violations);
    assert!(!status.is_empty());
    assert_eq!(call["winner_shape"], "as_exported");
    assert_eq!(call["winner_seed"], 7);
    assert_eq!(call["effort"]["level"], "m");
    assert_eq!(call["engine"]["name"], "fastroute");
    assert_eq!(call["lanes"]["routers"], 2);
    assert_eq!(call["outcomes"]["judged"], 1);
    assert_eq!(call["elapsed_s"], 61.2);
    assert_eq!(
        call["search_folder"].as_str().unwrap(),
        done.folder.display().to_string()
    );

    // Gate failures of the winner are read as they are.
    let failing = finished(Gates {
        g1: vec!["3 connections open".into()],
        g2: vec![],
        g3: vec!["a width".into()],
        g4: vec![],
    });
    let RouteResult::Routed { gates, .. } = read_route_result(&success_result(&failing)) else {
        panic!("a winner with failed gates is still routed");
    };
    assert_eq!(gates.g1, vec!["3 connections open".to_string()]);
    assert_eq!(gates.g3, vec!["a width".to_string()]);

    // "No run was routed and judged": an error result that still carries `search`.
    // It is `NoWinner` with its baseline, never `Failed`.
    let result = no_winner(Ok(baseline()));
    match read_route_result(&result) {
        RouteResult::NoWinner { unrouted, reason } => {
            assert_eq!(unrouted, Some(baseline().violations));
            assert!(reason.contains("No run was routed and judged"), "{reason}");
        }
        other => panic!("expected NoWinner, got {other:?}"),
    }
    // Without a usable baseline there is none to give.
    match read_route_result(&no_winner(Err("kicad-cli failed".into()))) {
        RouteResult::NoWinner { unrouted, .. } => assert_eq!(unrouted, None),
        other => panic!("expected NoWinner, got {other:?}"),
    }

    // A refusal before the search has no `search`: a failed call, with its words and
    // where the call put its folder.
    let args = json!({
        "dsn_path": done.dsn,
        "ses_output_path": done.root.join("refused.ses"),
        "effort": "ludicrous",
    });
    let ctx = isolated_context(&done.root.join("iso"), "kicad-cli").unwrap();
    let refused = crate::tools::route_fastroute::route_with_fastroute(
        &args,
        ctx.clone(),
        crate::tools::route_fastroute::PourPrune::Off,
        crate::place_route::CANDIDATE_RANK,
        crate::place_route::CANDIDATE_STITCH,
    )
    .await
    .unwrap();
    assert!(refused.is_error);
    match read_route_result(&refused) {
        RouteResult::Failed { stage, reason } => {
            assert_eq!(stage, "call");
            assert!(reason.contains("ludicrous"), "{reason}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    // A binary that is not there is refused after the folder was made, and the call's
    // note on where its files went is kept.
    let args = json!({
        "dsn_path": done.dsn,
        "ses_output_path": done.root.join("missing.ses"),
        "effort": "m",
        "fastroute_path": done.root.join("no-such-fastroute"),
    });
    let missing = crate::tools::route_fastroute::route_with_fastroute(
        &args,
        ctx,
        crate::tools::route_fastroute::PourPrune::Off,
        crate::place_route::CANDIDATE_RANK,
        crate::place_route::CANDIDATE_STITCH,
    )
    .await
    .unwrap();
    assert!(missing.is_error);
    match read_route_result(&missing) {
        RouteResult::Failed { stage, reason } => {
            assert_eq!(stage, "call");
            assert!(reason.contains("fastroute"), "{reason}");
            assert!(reason.contains("failed call"), "{reason}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }

    // Text that is no result at all is a failed call, not a panic.
    assert!(matches!(
        read_route_result(&CallToolResult::error("boom")),
        RouteResult::Failed { stage: "call", .. }
    ));
    assert!(matches!(
        read_route_result(&CallToolResult::json(&json!({"success": true}))),
        RouteResult::Failed { stage: "call", .. }
    ));
    // A winner whose measures cannot be read is a failed call too: no guessing.
    let mut broken = serde_json::from_str::<Value>(&text_of(&success_result(&done))).unwrap();
    broken["search"]["winner"]["measures"] = json!({"unconnected": "many"});
    assert!(matches!(
        read_route_result(&CallToolResult::json(&broken)),
        RouteResult::Failed { stage: "call", .. }
    ));
}

// ─── unrouted_check ──────────────────────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn the_unrouted_check_counts_violations_by_type_with_its_own_settings() {
    let bench = bench();
    let fake = bench.root.join("fake");
    std::fs::create_dir_all(&fake).unwrap();
    // Stands in for `kicad-cli pcb drc --output <json> ... --save-board <board>`:
    // records `KICAD_CONFIG_HOME` and the board it was given, writes a canned report.
    let script = r#"#!/bin/sh
dir="$(cd "$(dirname "$0")" && pwd)"
echo "KICAD_CONFIG_HOME=${KICAD_CONFIG_HOME-unset} $*" >> "$dir/calls.log"
out=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--output" ]; then out="$a"; fi
  prev="$a"
done
cp "$dir/drc.json" "$out"
exit 0
"#;
    let cli = crate::tools::cli::test_support::write_script(&fake, "kicad-cli", script, "");
    let violation =
        |rule: &str| json!({"type": rule, "severity": "error", "description": "x", "items": []});
    std::fs::write(
        fake.join("drc.json"),
        json!({
            "violations": [
                violation("clearance"),
                violation("clearance"),
                violation("courtyards_overlap"),
                violation("silk_overlap"),
            ],
            "unconnected_items": [violation("unconnected_items")],
        })
        .to_string(),
    )
    .unwrap();

    let mut inputs = bench.inputs.clone();
    inputs.kicad_cli = cli.display().to_string();
    let before = std::fs::read(&inputs.board).unwrap();
    let counts = unrouted_check(&inputs, &bench.root.join("checks"))
        .await
        .unwrap();
    assert_eq!(
        counts,
        BTreeMap::from([
            ("clearance".to_string(), 2),
            ("courtyards_overlap".to_string(), 1),
            ("silk_overlap".to_string(), 1),
        ])
    );
    // kicad-cli ran on a scratch copy, with the bench's own settings folder.
    let calls = std::fs::read_to_string(fake.join("calls.log")).unwrap();
    assert!(
        calls.contains(&format!(
            "KICAD_CONFIG_HOME={}",
            inputs.kicad_config_home.display()
        )),
        "{calls}"
    );
    assert!(
        calls.contains(&bench.root.join("checks/unrouted").display().to_string()),
        "{calls}"
    );
    assert_eq!(std::fs::read(&inputs.board).unwrap(), before);
    // A second check into the same folder is refused, not merged.
    assert!(unrouted_check(&inputs, &bench.root.join("checks"))
        .await
        .is_err());
    // A report whose JSON parses but has no unconnected-items category is refused, not
    // counted as a board with no such entries (`unrouted_counts`).
    std::fs::write(
        fake.join("drc.json"),
        json!({ "violations": [violation("silk_overlap")] }).to_string(),
    )
    .unwrap();
    let error = unrouted_check(&inputs, &bench.root.join("incomplete"))
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("no unconnected-items category"),
        "{error:#}"
    );
}

/// A report with these violation types and, when `unconnected` is `Some`, that many
/// unconnected items.
fn drc_report(rules: &[&str], unconnected: Option<usize>) -> crate::tools::cli::DrcReport {
    let violation = |rule: &str| crate::tools::cli::DrcViolation {
        severity: "error".into(),
        description: "x".into(),
        rule: rule.into(),
        pos: None,
        items: Vec::new(),
    };
    crate::tools::cli::DrcReport {
        violations: rules.iter().map(|rule| violation(rule)).collect(),
        unconnected_items: unconnected
            .map(|n| (0..n).map(|_| violation("unconnected_items")).collect()),
        schematic_parity: None,
        ownership_diagnostic: None,
        schematic_parity_diagnostic: None,
    }
}

#[test]
fn an_unrouted_check_refuses_a_report_without_the_unconnected_items_category() {
    // The bench's own per-candidate check (`route_judge::baseline`) turns such a report
    // into an error, never a zero; the silk check, which reads the same counts, must not
    // take it for a board with no entries.
    let rules = ["silk_overlap", "clearance", "silk_overlap"];
    let counts = unrouted_counts(&drc_report(&rules, Some(0))).unwrap();
    assert_eq!(
        counts,
        BTreeMap::from([
            ("clearance".to_string(), 1),
            ("silk_overlap".to_string(), 2)
        ])
    );
    // A report that lists none and has the category is a checked zero.
    assert!(unrouted_counts(&drc_report(&[], Some(3)))
        .unwrap()
        .is_empty());
    // The same violations without the category: refused, and the error says why.
    let error = unrouted_counts(&drc_report(&rules, None)).unwrap_err();
    assert!(
        format!("{error:#}").contains("no unconnected-items category"),
        "{error:#}"
    );
    // The judge's requirement and this one are one.
    assert!(crate::route_judge::unconnected_total(&drc_report(&rules, None)).is_err());
}

// ─── isolated_context ────────────────────────────────────────────────────────

#[tokio::test]
async fn the_isolated_context_reaches_no_kicad() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let folder = root.join("isolation");
    let ctx = isolated_context(&folder, "kicad-cli").unwrap();

    // The address lies inside the folder, and nothing is there.
    let address = ctx.config.ipc_address.clone();
    let socket = address.strip_prefix("ipc://").expect("an ipc address");
    assert!(Path::new(socket).starts_with(&folder), "{address}");
    assert!(!Path::new(socket).exists());
    assert_eq!(ctx.config.kicad_cli, "kicad-cli");

    // `score_placement` through it falls back to the saved file, and says so.
    let parts = vec![
        Fp::new("U1", 20.0, 20.0)
            .court(3.0, 3.0)
            .pad(pad("1", -2.0, 0.0, Some("VCC")))
            .pad(pad("2", 2.0, 0.0, Some("GND"))),
        Fp::new("C1", 12.0, 20.0).two_pad("VCC", "GND"),
    ];
    let board_path = root.join("scored.kicad_pcb");
    std::fs::write(
        &board_path,
        board(PartShape::Rect(0.0, 0.0, 60.0, 40.0), &parts),
    )
    .unwrap();
    let tool = crate::router::registry::ALL_TOOLSETS
        .iter()
        .find_map(|toolset| {
            crate::router::registry::tools_for(toolset.name)?
                .into_iter()
                .find(|tool| tool.name == "score_placement")
        })
        .expect("score_placement is in the registry");
    let result = (tool.handler)(&json!({ "board": board_path }), ctx.clone())
        .await
        .unwrap();
    assert!(!result.is_error, "{result:?}");
    let report: Value = serde_json::from_str(&text_of(&result)).unwrap();
    assert_eq!(report["source"], "saved_file");

    // The folder is now not empty, relative folders are refused, and so is a folder
    // that is a file.
    assert!(
        isolated_context(&folder, "kicad-cli").is_ok(),
        "an empty folder is fine"
    );
    std::fs::write(folder.join("x"), "x").unwrap();
    assert!(isolated_context(&folder, "kicad-cli").is_err());
    assert!(isolated_context(Path::new("relative/isolation"), "kicad-cli").is_err());
    assert!(isolated_context(&board_path, "kicad-cli").is_err());
}
