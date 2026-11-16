//! Tests of the best-of-N judge: preflight, gates, rank key, measures. The two
//! functions that need kicad-cli (`baseline` and `judge`) have one ignored test.

use super::*;
use crate::intent_width_report::WidthReport;
use crate::specctra_ses::{SesArc, SesImportPlan, SesTrack, SesVia};
use crate::tools::cli::{DrcReport, DrcViolation, ReportItem};

// ------------------------------------------------------------------ prepare

const BOARD: &str = "(kicad_pcb (version 20260206))\n";

/// A DSN, its manifest, the saved board and a fake kicad-cli, in a temp folder.
struct Setup {
    _dir: tempfile::TempDir,
    dsn: PathBuf,
    board: PathBuf,
    manifest: PathBuf,
    cli: String,
    root: PathBuf,
}

fn manifest_for(board: &Path, text: &str) -> Value {
    json!({
        "schema_version": 1,
        "board_path": board.display().to_string(),
        "source_sha256": format!("{:x}", Sha256::digest(text.as_bytes())),
        "pours": [
            {"net": "GND", "layer": "F.Cu", "policy": "fixed", "chosen_by": "default"},
            {"net": "GND", "layer": "B.Cu", "policy": "omit", "chosen_by": "default"}
        ],
        "intent_widths": [
            {"net": "VBUS", "from_mm": 0.25, "to_mm": 1.0, "status": "widened"},
            {"net": "VAUX", "width_mm": 0.5, "needed_mm": 0.4, "status": "already_wide_enough"},
            // A net that is also a pour net is a pour net.
            {"net": "GND", "from_mm": 0.25, "to_mm": 0.8, "status": "widened"},
            {"net": "PWR2", "status": "not_widened"},
            {"net": "GHOST", "status": "not_routed"}
        ],
        "classes": [
            {"name": "Default", "track_width_um": 250, "clearance_um": 200},
            {"name": "Power", "track_width_um": 500, "clearance_um": 200}
        ],
        "nets": [
            {"name": "S1", "class_name": "Default"},
            {"name": "VBUS", "class_name": "Power", "min_track_width_um": 300},
            {"name": "VAUX", "class_name": "Power"},
            {"name": "GND", "class_name": "Default"},
            {"name": "NOCLASS", "class_name": "Missing"}
        ]
    })
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn setup_with(dsn_name: &str) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.kicad_pcb");
    std::fs::write(&board, BOARD).unwrap();
    let dsn = dir.path().join(dsn_name);
    std::fs::write(&dsn, "(pcb board)").unwrap();
    let mut manifest_name = dsn.as_os_str().to_os_string();
    manifest_name.push(".konnect.json");
    let manifest = PathBuf::from(manifest_name);
    std::fs::write(&manifest, manifest_for(&board, BOARD).to_string()).unwrap();
    let cli = dir.path().join("kicad-cli");
    std::fs::write(&cli, "#!/bin/sh\nexit 0\n").unwrap();
    make_executable(&cli);
    let root = dir.path().join("search").join("judge");
    Setup {
        dsn,
        board,
        manifest,
        cli: cli.display().to_string(),
        root,
        _dir: dir,
    }
}

fn setup() -> Setup {
    setup_with("board.dsn")
}

fn refusal(result: std::result::Result<JudgeContext, Refusal>) -> Refusal {
    result.expect_err("the preflight must refuse")
}

#[test]
fn prepare_accepts_the_matching_case_and_reads_what_the_judge_needs() {
    let s = setup();
    let ctx = prepare(&s.dsn, &s.root, &s.cli).expect("a matching board, manifest and kicad-cli");
    assert_eq!(ctx.board, std::fs::canonicalize(&s.board).unwrap());
    assert_eq!(ctx.board_text, BOARD);
    assert_eq!(
        ctx.manifest_text,
        std::fs::read_to_string(&s.manifest).unwrap()
    );
    assert_eq!(ctx.manifest["schema_version"], 1);
    assert_eq!(ctx.kicad_cli, s.cli);
    assert_eq!(ctx.root, s.root);
    assert!(
        !s.root.exists(),
        "the caller creates the root, not the preflight"
    );

    // Pour nets: the manifest's pours rows. Power nets: widened or wide enough, and
    // not a pour net (GND is both widened and a pour).
    assert_eq!(
        ctx.pour_nets,
        ["GND"]
            .iter()
            .map(|n| n.to_string())
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(
        ctx.power_nets,
        ["VAUX", "VBUS"]
            .iter()
            .map(|n| n.to_string())
            .collect::<BTreeSet<_>>()
    );
    // Floors: a net's own minimum when it has one, else its class width; a net of
    // an unknown class has none.
    assert_eq!(ctx.floors_mm.get("S1"), Some(&0.25));
    assert_eq!(ctx.floors_mm.get("VBUS"), Some(&0.3));
    assert_eq!(ctx.floors_mm.get("VAUX"), Some(&0.5));
    assert_eq!(ctx.floors_mm.get("GND"), Some(&0.25));
    assert_eq!(ctx.floors_mm.get("NOCLASS"), None);
}

#[test]
fn prepare_refuses_without_a_manifest_beside_the_dsn() {
    let s = setup();
    std::fs::remove_file(&s.manifest).unwrap();
    let refused = refusal(prepare(&s.dsn, &s.root, &s.cli));
    assert!(refused.reason.contains("manifest"), "{refused:?}");
    assert!(
        refused.next_step.contains("export_specctra_dsn"),
        "{refused:?}"
    );
}

#[test]
fn the_manifest_name_is_the_whole_dsn_file_name_with_the_suffix_appended() {
    // A dotted name: appending, not replacing the extension.
    let s = setup_with("my.board v2.dsn");
    assert!(s.manifest.ends_with("my.board v2.dsn.konnect.json"));
    assert!(prepare(&s.dsn, &s.root, &s.cli).is_ok());
    // The manifest under an extension-replaced name is not the one.
    let wrong = s.dsn.with_extension("konnect.json");
    std::fs::rename(&s.manifest, &wrong).unwrap();
    assert!(prepare(&s.dsn, &s.root, &s.cli).is_err());
}

#[test]
fn prepare_refuses_a_manifest_that_is_not_json_or_names_no_board() {
    let s = setup();
    std::fs::write(&s.manifest, "{ not json").unwrap();
    assert!(refusal(prepare(&s.dsn, &s.root, &s.cli))
        .reason
        .contains("not JSON"));

    for broken in [
        json!({"source_sha256": "x"}),
        json!({"board_path": "", "source_sha256": "x"}),
    ] {
        std::fs::write(&s.manifest, broken.to_string()).unwrap();
        let refused = refusal(prepare(&s.dsn, &s.root, &s.cli));
        assert!(refused.reason.contains("no board"), "{refused:?}");
    }
}

#[test]
fn prepare_refuses_a_missing_board() {
    let s = setup();
    std::fs::remove_file(&s.board).unwrap();
    let refused = refusal(prepare(&s.dsn, &s.root, &s.cli));
    assert!(refused.reason.contains("missing"), "{refused:?}");
}

#[test]
fn prepare_refuses_a_board_saved_after_the_export() {
    let s = setup();
    std::fs::write(&s.board, format!("{BOARD}; edited after the export\n")).unwrap();
    let refused = refusal(prepare(&s.dsn, &s.root, &s.cli));
    assert!(refused.reason.contains("SHA-256"), "{refused:?}");
    assert!(
        refused.next_step.contains("save the board in KiCad"),
        "{refused:?}"
    );
    assert!(
        refused.next_step.contains("export the DSN again"),
        "{refused:?}"
    );

    // A manifest with no hash at all is no proof either.
    std::fs::write(&s.board, BOARD).unwrap();
    let mut manifest = manifest_for(&s.board, BOARD);
    manifest.as_object_mut().unwrap().remove("source_sha256");
    std::fs::write(&s.manifest, manifest.to_string()).unwrap();
    assert!(refusal(prepare(&s.dsn, &s.root, &s.cli))
        .reason
        .contains("SHA-256"));

    // The unchanged board is accepted again (the pair).
    std::fs::write(&s.manifest, manifest_for(&s.board, BOARD).to_string()).unwrap();
    assert!(prepare(&s.dsn, &s.root, &s.cli).is_ok());
}

#[test]
fn prepare_reads_the_hash_in_either_letter_case() {
    let s = setup();
    let mut manifest = manifest_for(&s.board, BOARD);
    let upper = manifest["source_sha256"].as_str().unwrap().to_uppercase();
    manifest["source_sha256"] = json!(upper);
    std::fs::write(&s.manifest, manifest.to_string()).unwrap();
    assert!(prepare(&s.dsn, &s.root, &s.cli).is_ok());
}

/// The configured default is the bare name `kicad-cli`, which KiCad's installer does not
/// put on PATH: the judge must find it as every other kicad-cli tool does
/// (`resolve_cli_executable`), not refuse it (the first end-to-end call did, 2026-11-16).
#[test]
fn prepare_resolves_a_bare_kicad_cli_name_as_the_other_tools_do() {
    let s = setup();
    let resolved = crate::tools::cli::resolve_cli_executable("kicad-cli");
    let found = is_executable_file(&resolved);
    let result = prepare(&s.dsn, &s.root, "kicad-cli");
    if found {
        let ctx = result.expect("a kicad-cli the other tools find is accepted");
        assert_eq!(ctx.kicad_cli, resolved.display().to_string());
    } else {
        assert!(result.is_err(), "no kicad-cli on this machine: refused");
    }
}

#[cfg(unix)]
#[test]
fn prepare_refuses_a_kicad_cli_that_is_not_an_executable_file() {
    use std::os::unix::fs::PermissionsExt;
    let s = setup();

    // Not executable.
    std::fs::set_permissions(&s.cli, std::fs::Permissions::from_mode(0o644)).unwrap();
    let refused = refusal(prepare(&s.dsn, &s.root, &s.cli));
    assert!(refused.reason.contains("kicad-cli"), "{refused:?}");
    assert!(!refused.next_step.is_empty());

    // Not there, a folder, and empty.
    let missing = s.root.join("no-such-kicad-cli").display().to_string();
    assert!(prepare(&s.dsn, &s.root, &missing).is_err());
    let folder = s._dir.path().display().to_string();
    assert!(prepare(&s.dsn, &s.root, &folder).is_err());
    assert!(prepare(&s.dsn, &s.root, "").is_err());

    // Any execute bit is enough (the pair).
    std::fs::set_permissions(&s.cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(prepare(&s.dsn, &s.root, &s.cli).is_ok());
}

// -------------------------------------------------------------------- gates

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
        power_mm: 10.0,
        pour_track_mm: 0.0,
        vias_off_pour: 0,
        below_floor: 0,
        width_rows: vec![],
        longest_below_mm: BTreeMap::new(),
        stitching_vias: 0,
        route_s: 10.0,
    }
}

fn base(violations: &[(&str, usize)]) -> Baseline {
    Baseline {
        violations: violations
            .iter()
            .map(|(k, n)| (k.to_string(), *n))
            .collect(),
        unconnected: 3,
    }
}

fn rules(counts: &[(&str, usize)]) -> BTreeMap<String, usize> {
    counts.iter().map(|(k, n)| (k.to_string(), *n)).collect()
}

/// An `intent_width_report` row, as the report writes it.
fn width_row(net: &str, routed: f64, below: f64, share: f64, longest: f64) -> Value {
    json!({
        "net": net,
        "intent_width_mm": 1.0,
        "basis": "widened",
        "routed_mm": routed,
        "below_intent_mm": below,
        "below_intent_share": share,
        "longest_below_intent_mm": longest,
    })
}

#[test]
fn a_clean_run_passes_every_gate() {
    let mut m = measures();
    m.violations = rules(&[("clearance", 2)]);
    m.width_rows = vec![width_row("VBUS", 40.0, 1.0, 0.025, 0.5)];
    let g = gates(&m, &base(&[("clearance", 2), ("silk_overlap", 1)]));
    assert_eq!(
        g,
        Gates {
            g1: vec![],
            g2: vec![],
            g3: vec![]
        }
    );
    assert!(g.all_pass());
}

#[test]
fn all_pass_needs_all_three_lists_empty() {
    let pass = Gates {
        g1: vec![],
        g2: vec![],
        g3: vec![],
    };
    assert!(pass.all_pass());
    for failed in [
        Gates {
            g1: vec!["x".into()],
            ..pass.clone()
        },
        Gates {
            g2: vec!["x".into()],
            ..pass.clone()
        },
        Gates {
            g3: vec!["x".into()],
            ..pass.clone()
        },
    ] {
        assert!(!failed.all_pass());
    }
}

/// The gate lists of `m` when exactly one gate is expected to fail with one line.
fn only(g: &Gates, which: usize) {
    let lists = [&g.g1, &g.g2, &g.g3];
    for (index, list) in lists.iter().enumerate() {
        if index == which {
            assert_eq!(list.len(), 1, "G{} lines: {list:?}", index + 1);
        } else {
            assert!(list.is_empty(), "G{} lines: {list:?}", index + 1);
        }
    }
}

#[test]
fn g1_each_condition_fails_alone() {
    let b = base(&[]);

    let mut m = measures();
    m.unconnected = 2;
    only(&gates(&m, &b), 0);

    let mut m = measures();
    m.routed_share = Some(0.99);
    only(&gates(&m, &b), 0);

    let mut m = measures();
    m.routed_share = None;
    only(&gates(&m, &b), 0);

    let mut m = measures();
    m.audit_complete = Some(false);
    only(&gates(&m, &b), 0);

    let mut m = measures();
    m.audit_complete = None;
    only(&gates(&m, &b), 0);

    // All three at once: three lines.
    let mut m = measures();
    m.unconnected = 1;
    m.routed_share = Some(0.5);
    m.audit_complete = Some(false);
    assert_eq!(gates(&m, &b).g1.len(), 3);
}

#[test]
fn g1_routed_share_of_one_within_1e_9_passes() {
    let b = base(&[]);
    let mut m = measures();
    m.routed_share = Some(1.0 - 5e-10);
    assert!(gates(&m, &b).all_pass());
    m.routed_share = Some(1.0 - 5e-9);
    only(&gates(&m, &b), 0);
}

#[test]
fn g2_equal_passes_one_more_fails_a_new_type_fails() {
    let baseline = base(&[("clearance", 2), ("silk_overlap", 1)]);

    let mut m = measures();
    m.violations = rules(&[("clearance", 2), ("silk_overlap", 1)]);
    assert!(
        gates(&m, &baseline).all_pass(),
        "equal to the baseline passes"
    );

    m.violations = rules(&[("clearance", 1)]);
    assert!(
        gates(&m, &baseline).all_pass(),
        "fewer than the baseline passes"
    );

    m.violations = rules(&[("clearance", 3), ("silk_overlap", 1)]);
    let g = gates(&m, &baseline);
    only(&g, 1);
    assert!(g.g2[0].contains("clearance"), "{:?}", g.g2);

    // A type the unrouted board does not have.
    m.violations = rules(&[("clearance", 2), ("shorting_items", 1)]);
    let g = gates(&m, &baseline);
    only(&g, 1);
    assert!(g.g2[0].contains("shorting_items"), "{:?}", g.g2);

    // Two types over: two lines.
    m.violations = rules(&[("clearance", 9), ("shorting_items", 1)]);
    assert_eq!(gates(&m, &baseline).g2.len(), 2);
}

#[test]
fn g3_a_track_below_its_floor_fails() {
    let mut m = measures();
    m.below_floor = 1;
    only(&gates(&m, &base(&[])), 2);
    m.below_floor = 0;
    assert!(gates(&m, &base(&[])).all_pass());
}

#[test]
fn g3_track_width_violations_fail_even_when_the_baseline_has_them() {
    let mut m = measures();
    m.violations = rules(&[("track_width", 1)]);
    // Against a baseline with one: G2 is quiet, G3 is absolute.
    only(&gates(&m, &base(&[("track_width", 1)])), 2);
    // Against a baseline with none: both.
    let g = gates(&m, &base(&[]));
    assert_eq!((g.g2.len(), g.g3.len()), (1, 1));
}

#[test]
fn g3_share_below_intent_width_is_at_most_five_percent() {
    let b = base(&[]);
    let mut m = measures();

    m.width_rows = vec![width_row("VBUS", 100.0, 5.0, BELOW_INTENT_SHARE, 0.5)];
    assert!(gates(&m, &b).all_pass(), "exactly 5 % passes");

    m.width_rows = vec![width_row("VBUS", 100.0, 5.1, 0.051, 0.5)];
    let g = gates(&m, &b);
    only(&g, 2);
    assert!(g.g3[0].contains("VBUS"), "{:?}", g.g3);

    // A row with no share figure: the share is below over routed.
    let mut row = width_row("VBUS", 10.0, 0.6, 0.0, 0.5);
    row.as_object_mut().unwrap().remove("below_intent_share");
    m.width_rows = vec![row];
    only(&gates(&m, &b), 2);
}

#[test]
fn g3_no_stretch_below_intent_width_longer_than_two_mm() {
    let b = base(&[]);
    let mut m = measures();

    m.width_rows = vec![width_row("VBUS", 100.0, 1.0, 0.01, BELOW_INTENT_SECTION_MM)];
    assert!(gates(&m, &b).all_pass(), "exactly 2.0 mm passes");

    m.width_rows = vec![width_row("VBUS", 100.0, 1.0, 0.01, 2.001)];
    only(&gates(&m, &b), 2);

    // The longest stretch is also given per net; the larger of the two counts.
    m.width_rows = vec![width_row("VBUS", 100.0, 1.0, 0.01, 0.5)];
    m.longest_below_mm = [("VBUS".to_string(), 2.5)].into_iter().collect();
    only(&gates(&m, &b), 2);

    // Each net is judged: one bad net among good ones is one line.
    m.longest_below_mm.clear();
    m.width_rows = vec![
        width_row("VBUS", 100.0, 1.0, 0.01, 0.5),
        width_row("VAUX", 100.0, 1.0, 0.01, 3.0),
        width_row("VOTHER", 100.0, 1.0, 0.01, 0.1),
    ];
    let g = gates(&m, &b);
    only(&g, 2);
    assert!(g.g3[0].contains("VAUX"), "{:?}", g.g3);
}

#[test]
fn g3_audit_4_2_and_4_3_fail_when_nonzero_and_pass_when_absent() {
    let b = base(&[]);

    let mut m = measures();
    m.nets_undersized = Some(1.0);
    only(&gates(&m, &b), 2);

    let mut m = measures();
    m.narrow_paths = Some(2.0);
    only(&gates(&m, &b), 2);

    // The audit did not run those checks: nothing to fail.
    let mut m = measures();
    m.nets_undersized = None;
    m.narrow_paths = None;
    assert!(gates(&m, &b).all_pass());
}

#[test]
fn the_three_gates_are_judged_independently() {
    let mut m = measures();
    m.unconnected = 1;
    m.violations = rules(&[("clearance", 1)]);
    m.below_floor = 1;
    let g = gates(&m, &base(&[]));
    assert_eq!((g.g1.len(), g.g2.len(), g.g3.len()), (1, 1, 1));
}

// --------------------------------------------------------------------- rank

#[derive(Clone)]
struct Spec {
    failed: [bool; 3],
    acute: Option<f64>,
    signal: f64,
    power: f64,
    right: Option<f64>,
    vias: usize,
    quality: Option<f64>,
    seconds: f64,
}

fn spec() -> Spec {
    Spec {
        failed: [false; 3],
        acute: Some(0.0),
        signal: 100.0,
        power: 10.0,
        right: Some(0.0),
        vias: 0,
        quality: Some(90.0),
        seconds: 10.0,
    }
}

fn run(s: &Spec) -> Judged {
    let mut m = measures();
    m.acute = s.acute;
    m.signal_mm = s.signal;
    m.power_mm = s.power;
    m.right_angles = s.right;
    m.vias_off_pour = s.vias;
    m.copper_quality = s.quality;
    m.route_s = s.seconds;
    let line = |failed: bool| {
        if failed {
            vec!["failed".to_string()]
        } else {
            vec![]
        }
    };
    Judged {
        measures: m,
        gates: Gates {
            g1: line(s.failed[0]),
            g2: line(s.failed[1]),
            g3: line(s.failed[2]),
        },
        board: PathBuf::new(),
        rounds: vec![],
    }
}

fn keys(specs: &[Spec]) -> Vec<Key> {
    let judged: Vec<Judged> = specs.iter().map(run).collect();
    let refs: Vec<&Judged> = judged.iter().collect();
    rank(&refs)
}

#[test]
fn the_key_of_a_lone_run_follows_the_documented_order() {
    let mut s = spec();
    s.acute = Some(2.0);
    s.right = Some(3.0);
    s.vias = 4;
    s.quality = Some(80.4);
    s.seconds = 12.9;
    // [G1, G2, G3, acute, signal step, power step, right angles, vias off pour,
    //  -copper quality, route seconds]: a lone run is its own shortest (ratio 1, step 50).
    assert_eq!(
        keys(&[s.clone()]),
        vec![[0, 0, 0, 2, 50, 50, 3, 4, -80, 12]]
    );

    s.failed = [true, false, true];
    assert_eq!(keys(&[s])[0][..3], [1, 0, 1]);
}

#[test]
fn the_rank_orders_by_g1_then_g2_then_g3_then_acute_then_the_rest() {
    let with = |change: &dyn Fn(&mut Spec)| {
        let mut s = spec();
        change(&mut s);
        s
    };
    // Labelled runs, in a scrambled order.
    let runs: Vec<(char, Spec)> = vec![
        ('d', with(&|s| s.failed[0] = true)),
        ('f', with(&|s| s.signal = 110.0)),
        ('a', spec()),
        ('j', with(&|s| s.quality = Some(80.0))),
        ('c', with(&|s| s.failed[1] = true)),
        ('h', with(&|s| s.right = Some(1.0))),
        ('e', with(&|s| s.acute = Some(1.0))),
        ('k', with(&|s| s.seconds = 20.0)),
        ('b', with(&|s| s.failed[2] = true)),
        ('i', with(&|s| s.vias = 1)),
        ('g', with(&|s| s.power = 12.0)),
    ];
    let specs: Vec<Spec> = runs.iter().map(|(_, s)| s.clone()).collect();
    let keys = keys(&specs);
    assert_eq!(keys.len(), runs.len());
    let mut order: Vec<usize> = (0..runs.len()).collect();
    order.sort_by_key(|&i| keys[i]);
    let sorted: String = order.iter().map(|&i| runs[i].0).collect();
    // Perfect first; seconds, quality, vias, right angles, power, signal and acute
    // corners each push a run back, the later in the key the less; a failed G3, G2
    // and G1 are worse than any of those, in that order.
    assert_eq!(sorted, "akjihgfebcd");
}

#[test]
fn lengths_are_compared_in_steps_against_the_shortest_judged_run() {
    let mut a = spec();
    a.signal = 100.0;
    a.power = 6.0;
    // 100.5 / 100 = 1.005: inside the same 0.02 step as 100, so power decides.
    let mut b = spec();
    b.signal = 100.5;
    b.power = 5.0;
    let ks = keys(&[a.clone(), b.clone()]);
    assert_eq!(ks[0][4], ks[1][4], "same signal step");
    assert_eq!(ks[0][4], 50);
    assert_eq!(ks[1][5], 50);
    assert_eq!(ks[0][5], 60);
    assert!(ks[1] < ks[0], "the shorter power wins when signal ties");

    // 102 / 100: one step is 2 %: a different step.
    let mut c = spec();
    c.signal = 102.0;
    let ks = keys(&[spec(), c]);
    assert_eq!(ks[0][4], 50);
    assert_eq!(ks[1][4], 51);
}

#[test]
fn when_every_length_of_a_category_is_zero_its_step_is_zero() {
    let mut a = spec();
    a.signal = 0.0;
    a.power = 0.0;
    let mut b = a.clone();
    b.acute = Some(3.0);
    for key in keys(&[a, b]) {
        assert_eq!(key[4], 0, "signal step");
        assert_eq!(key[5], 0, "power step");
    }
}

#[test]
fn a_run_with_none_of_a_category_among_runs_that_have_some_has_step_zero() {
    let mut none = spec();
    none.power = 0.0;
    let ks = keys(&[none, spec()]);
    assert_eq!(ks[0][5], 0);
    assert_eq!(
        ks[1][5], 50,
        "the shortest run that has any is the reference"
    );
}

#[test]
fn a_measure_the_audit_did_not_give_sorts_last() {
    let mut missing = spec();
    missing.acute = None;
    missing.right = None;
    missing.quality = None;
    let k = keys(&[missing])[0];
    assert_eq!((k[3], k[6], k[8]), (i64::MAX, i64::MAX, i64::MAX));

    // Even a run with many acute corners sorts before a run with none known.
    let mut many = spec();
    many.acute = Some(1000.0);
    let mut unknown = spec();
    unknown.acute = None;
    let ks = keys(&[unknown, many]);
    assert!(ks[1] < ks[0]);

    // Same for the copper quality and the right angles.
    let mut no_quality = spec();
    no_quality.quality = None;
    let mut low_quality = spec();
    low_quality.quality = Some(1.0);
    let ks = keys(&[no_quality, low_quality]);
    assert!(ks[1] < ks[0]);
}

#[test]
fn copper_quality_counts_in_whole_points_and_seconds_round_down() {
    let mut a = spec();
    a.quality = Some(90.4);
    a.seconds = 10.99;
    let mut b = spec();
    b.quality = Some(89.6);
    b.seconds = 10.0;
    let ks = keys(&[a, b]);
    assert_eq!(ks[0][8], -90);
    assert_eq!(ks[1][8], -90, "89.6 rounds to 90 as well");
    assert_eq!(ks[0][9], 10);
    assert_eq!(ks[1][9], 10);
}

#[test]
fn rank_returns_one_key_per_run_in_input_order() {
    assert!(rank(&[]).is_empty());
    let mut worse = spec();
    worse.acute = Some(9.0);
    let ks = keys(&[worse, spec(), spec()]);
    assert_eq!(ks.len(), 3);
    assert_eq!(ks[0][3], 9);
    assert_eq!(ks[1], ks[2]);
}

// ------------------------------------------------------------------ measure

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn context() -> JudgeContext {
    JudgeContext {
        board: PathBuf::from("/b/board.kicad_pcb"),
        board_text: String::new(),
        manifest: json!({}),
        manifest_text: "{}".to_string(),
        kicad_cli: String::new(),
        pour_nets: set(&["GND"]),
        // GND is also listed as a power net: a pour net wins.
        power_nets: set(&["VBUS", "GND"]),
        floors_mm: [("S1", 0.25), ("VBUS", 0.6), ("GND", 0.2)]
            .iter()
            .map(|(net, mm)| (net.to_string(), *mm))
            .collect(),
        root: PathBuf::from("/r"),
    }
}

fn session_track(net: &str, width: f64, from: (f64, f64), to: (f64, f64)) -> SesTrack {
    SesTrack {
        net_name: net.to_string(),
        layer: "F.Cu".to_string(),
        width_mm: width,
        x1_mm: from.0,
        y1_mm: from.1,
        x2_mm: to.0,
        y2_mm: to.1,
    }
}

fn session_arc(net: &str, width: f64) -> SesArc {
    let half = std::f64::consts::FRAC_1_SQRT_2;
    SesArc {
        net_name: net.to_string(),
        layer: "F.Cu".to_string(),
        width_mm: width,
        start_x_mm: 1.0,
        start_y_mm: 0.0,
        mid_x_mm: half,
        mid_y_mm: half,
        end_x_mm: 0.0,
        end_y_mm: 1.0,
    }
}

fn session_via(net: &str) -> SesVia {
    SesVia {
        net_name: net.to_string(),
        padstack_name: "Via".to_string(),
        x_mm: 1.0,
        y_mm: 1.0,
        drill_mm: 0.3,
        size_mm: 0.6,
    }
}

fn plan(tracks: Vec<SesTrack>, arcs: Vec<SesArc>, vias: Vec<SesVia>) -> SesImportPlan {
    SesImportPlan {
        routing_context: None,
        board_path: String::new(),
        source_sha256: String::new(),
        session_id: "s".to_string(),
        locked_track_count: 0,
        locked_via_count: 0,
        tracks,
        arcs,
        vias,
        duplicates_skipped: 0,
    }
}

fn violation(rule: &str, descriptions: &[&str]) -> DrcViolation {
    DrcViolation {
        severity: "error".to_string(),
        description: "a description".to_string(),
        rule: rule.to_string(),
        pos: None,
        items: descriptions
            .iter()
            .map(|description| ReportItem {
                description: description.to_string(),
                ..Default::default()
            })
            .collect(),
    }
}

fn report(violations: Vec<DrcViolation>, unconnected: Option<Vec<DrcViolation>>) -> DrcReport {
    DrcReport {
        violations,
        unconnected_items: unconnected,
        schematic_parity: None,
        ownership_diagnostic: None,
        schematic_parity_diagnostic: None,
    }
}

fn measured(plan: &SesImportPlan) -> Measures {
    measure(
        &context(),
        &report(vec![], Some(vec![])),
        &json!({}),
        plan,
        &WidthReport::default(),
        0,
        1.0,
    )
    .unwrap()
}

#[test]
fn lengths_are_summed_by_category_and_a_pour_net_beats_power() {
    let plan = plan(
        vec![
            session_track("S1", 0.25, (0.0, 0.0), (3.0, 4.0)),
            session_track("S1", 0.25, (3.0, 4.0), (3.0, 5.0)),
            session_track("VBUS", 0.6, (0.0, 0.0), (10.0, 0.0)),
            // GND is a pour net and also listed as a power net.
            session_track("GND", 0.2, (0.0, 0.0), (0.0, 2.0)),
            // A net that is in no list is signal.
            session_track("OTHER", 0.25, (0.0, 0.0), (0.0, 1.0)),
        ],
        vec![session_arc("S1", 0.25)],
        vec![],
    );
    let m = measured(&plan);
    let quarter_circle = std::f64::consts::FRAC_PI_2;
    assert!(
        (m.signal_mm - (5.0 + 1.0 + 1.0 + quarter_circle)).abs() < 1e-9,
        "{}",
        m.signal_mm
    );
    assert!((m.power_mm - 10.0).abs() < 1e-9, "{}", m.power_mm);
    assert!((m.pour_track_mm - 2.0).abs() < 1e-9, "{}", m.pour_track_mm);
}

#[test]
fn vias_on_pour_nets_do_not_count_as_vias_off_pour() {
    let plan = plan(
        vec![],
        vec![],
        vec![
            session_via("S1"),
            session_via("GND"),
            session_via("VBUS"),
            session_via("GND"),
        ],
    );
    assert_eq!(measured(&plan).vias_off_pour, 2);
    assert_eq!(
        measured(&self::plan(vec![], vec![], vec![session_via("GND")])).vias_off_pour,
        0
    );
}

#[test]
fn below_floor_counts_tracks_and_arcs_beyond_the_half_micrometre_tolerance() {
    let one = |width: f64| {
        measured(&plan(
            vec![session_track("S1", width, (0.0, 0.0), (1.0, 0.0))],
            vec![],
            vec![],
        ))
        .below_floor
    };
    assert_eq!(one(0.25), 0, "at the floor");
    assert_eq!(one(0.3), 0, "above the floor");
    assert_eq!(
        one(0.2495),
        0,
        "exactly the tolerance below is at the floor"
    );
    assert_eq!(one(0.2494), 1, "a little beyond the tolerance is below");
    assert_eq!(one(0.2), 1);

    // An arc counts like a track; a net with no floor never counts.
    let m = measured(&plan(
        vec![
            session_track("S1", 0.2, (0.0, 0.0), (1.0, 0.0)),
            session_track("VBUS", 0.5, (0.0, 0.0), (1.0, 0.0)),
            session_track("UNKNOWN", 0.01, (0.0, 0.0), (1.0, 0.0)),
        ],
        vec![
            session_arc("S1", 0.1),
            session_arc("S1", 0.25),
            session_arc("UNKNOWN", 0.01),
        ],
        vec![],
    ));
    assert_eq!(m.below_floor, 3, "S1 track, VBUS track, S1 arc");
}

#[test]
fn arc_length_follows_the_arc_through_the_mid_point() {
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let half = std::f64::consts::FRAC_1_SQRT_2;
    // A quarter circle of radius 1, either way round.
    assert!(near(
        arc_length_mm([1.0, 0.0], [half, half], [0.0, 1.0]),
        std::f64::consts::FRAC_PI_2
    ));
    assert!(near(
        arc_length_mm([0.0, 1.0], [half, half], [1.0, 0.0]),
        std::f64::consts::FRAC_PI_2
    ));
    // A half circle.
    assert!(near(
        arc_length_mm([1.0, 0.0], [0.0, 1.0], [-1.0, 0.0]),
        std::f64::consts::PI
    ));
    // Three quarters: from (1,0) through (0,-1) to (0,1), the long way.
    assert!(near(
        arc_length_mm([1.0, 0.0], [0.0, -1.0], [0.0, 1.0]),
        1.5 * std::f64::consts::PI
    ));
    // A radius of 5: a half circle is 5 pi.
    assert!(near(
        arc_length_mm([5.0, 0.0], [0.0, 5.0], [-5.0, 0.0]),
        5.0 * std::f64::consts::PI
    ));
    // Three points on a line have no circle: the chord.
    assert!(near(arc_length_mm([0.0, 0.0], [1.0, 0.0], [3.0, 0.0]), 3.0));
    // Start and end alike: nothing to measure.
    assert!(near(arc_length_mm([1.0, 1.0], [2.0, 2.0], [1.0, 1.0]), 0.0));
}

#[test]
fn measure_reads_kicads_report_and_the_audit() {
    let drc = report(
        vec![
            violation("clearance", &["Track [S1] on F.Cu"]),
            violation("clearance", &["Track [S1] on F.Cu"]),
            violation("shorting_items", &[]),
        ],
        Some(vec![
            violation("unconnected_items", &["Pad 1 [S1] of U1"]),
            violation("unconnected_items", &[]),
        ]),
    );
    let audit = json!({
        "audit": "layout_quality",
        "quality": { "routed_share": 0.998, "copper_quality": 87.5 },
        "completion": { "complete": true },
        "metrics": {
            "5.3.acute_corners": 3,
            "5.3.right_angle_corners": 4,
            "4.2.nets_undersized": 0,
            "4.3.narrow_paths": 1
        }
    });
    let widths = WidthReport {
        nets: vec![
            width_row("VBUS", 40.0, 1.0, 0.025, 0.7),
            width_row("VAUX", 5.0, 0.0, 0.0, 0.0),
        ],
        ..Default::default()
    };
    let m = measure(
        &context(),
        &drc,
        &audit,
        &plan(vec![], vec![], vec![]),
        &widths,
        5,
        12.5,
    )
    .unwrap();
    assert_eq!(m.unconnected, 2);
    assert_eq!(
        m.violations,
        rules(&[("clearance", 2), ("shorting_items", 1)])
    );
    assert_eq!(m.routed_share, Some(0.998));
    assert_eq!(m.audit_complete, Some(true));
    assert_eq!(m.acute, Some(3.0));
    assert_eq!(m.right_angles, Some(4.0));
    assert_eq!(m.copper_quality, Some(87.5));
    assert_eq!(m.nets_undersized, Some(0.0));
    assert_eq!(m.narrow_paths, Some(1.0));
    assert_eq!(m.stitching_vias, 5);
    assert_eq!(m.route_s, 12.5);
    assert_eq!(m.width_rows.len(), 2);
    assert_eq!(m.longest_below_mm.get("VBUS"), Some(&0.7));
    assert_eq!(m.longest_below_mm.get("VAUX"), Some(&0.0));
}

#[test]
fn a_value_the_audit_does_not_give_is_none_never_zero() {
    let m = measured(&plan(vec![], vec![], vec![]));
    assert_eq!(m.routed_share, None);
    assert_eq!(m.audit_complete, None);
    assert_eq!(m.acute, None);
    assert_eq!(m.right_angles, None);
    assert_eq!(m.copper_quality, None);
    assert_eq!(m.nets_undersized, None);
    assert_eq!(m.narrow_paths, None);
    // And such a run fails G1 (nothing says it is complete).
    let g = gates(&m, &base(&[]));
    assert_eq!(g.g1.len(), 2, "{:?}", g.g1);
}

#[test]
fn a_report_with_no_unconnected_category_is_an_error_never_zero() {
    let result = measure(
        &context(),
        &report(vec![], None),
        &json!({}),
        &plan(vec![], vec![], vec![]),
        &WidthReport::default(),
        0,
        1.0,
    );
    let error = result.unwrap_err();
    assert!(format!("{error:#}").contains("unconnected"), "{error:#}");
}

// ------------------------------------------------------------ unconnected_on

#[test]
fn unconnected_on_matches_the_net_in_brackets_exactly() {
    let drc = report(
        // Violations are not unconnected items, whatever they name.
        vec![violation("clearance", &["Track [GND] on F.Cu"])],
        Some(vec![
            violation(
                "unconnected_items",
                &["Pad 1 [GND] of U1 on F.Cu", "Pad 2 [GND] of U2 on F.Cu"],
            ),
            violation(
                "unconnected_items",
                &["Pad 3 [GND2] of U3 on F.Cu", "Track [VCC] on B.Cu"],
            ),
            violation("unconnected_items", &["Zone [GND_TOP] of F.Cu"]),
            violation("unconnected_items", &[]),
        ]),
    );
    let on = |nets: &[&str]| {
        unconnected_on(
            &drc,
            &nets.iter().map(|n| n.to_string()).collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        on(&["GND"]),
        1,
        "[GND] is not [GND2] or [GND_TOP]; two items count once"
    );
    assert_eq!(on(&["GND2"]), 1);
    assert_eq!(on(&["GND_TOP"]), 1);
    assert_eq!(on(&["GND", "GND2"]), 2);
    assert_eq!(on(&["GND", "VCC"]), 2, "each violation counts once");
    assert_eq!(on(&["VCC"]), 1);
    assert_eq!(on(&["GN"]), 0, "a prefix is not a match");
    assert_eq!(on(&["ND"]), 0);
    assert_eq!(on(&["gnd"]), 0, "net names are case-sensitive");
    assert_eq!(on(&[]), 0);
}

#[test]
fn unconnected_on_counts_nothing_when_the_report_has_no_unconnected_category() {
    let drc = report(vec![violation("clearance", &["Track [GND] on F.Cu"])], None);
    assert_eq!(unconnected_on(&drc, &["GND".to_string()]), 0);
}

// ------------------------------------------------------------------ judge

/// Judges the recorded session of the rounded two-resistor board on scratch copies,
/// with the real kicad-cli. Not run in the ordinary gate.
///
/// LEAD: run it by hand, after it compiles:
///
/// ```text
/// KICAD_CONFIG_HOME=<scratch folder> KONNECT_TEST_KICAD_CLI=<path to kicad-cli> \
///   cargo test -p konnect-core --lib judges_a_recorded_session -- --ignored --nocapture
/// ```
///
/// `KONNECT_TEST_KICAD_CLI` defaults to KiCad 10's path on macOS. The test builds its
/// own DSN manifest from `specctra_pours_policy.kicad_pcb` (named as the recorded
/// session's board), writes the recorded session beside it and has no router. Its
/// pours are all left out, so there are no planes and no stitching rounds; judging a
/// run with planes needs the pour-routing fixture and a planner-ready board and is
/// left to the real measurement. It prints the measures and gates: the gates of a
/// recorded session of this fixture are not asserted, only that the judge ran end to
/// end, wrote its boards where documented and measured the session's copper.
#[tokio::test]
#[ignore = "needs kicad-cli (KICAD_CONFIG_HOME must name a scratch folder)"]
async fn judges_a_recorded_session_on_scratch_copies() {
    use crate::specctra::{export_dsn_with_options, ExportOptions, PourPolicy};
    use crate::tools::ServerConfig;
    use konnect_ipc::{IpcEffectiveRoutingRules, IpcRoutingRules};

    assert!(
        std::env::var_os("KICAD_CONFIG_HOME").is_some(),
        "LEAD: set KICAD_CONFIG_HOME to a scratch folder, or KiCad re-saves the real settings"
    );
    let cli = std::env::var("KONNECT_TEST_KICAD_CLI")
        .unwrap_or_else(|_| "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli".to_string());

    let board_text = include_str!("../tests/fixtures/specctra_pours_policy.kicad_pcb");
    let ses_text = include_str!("../tests/fixtures/specctra_rounded_board.freerouting-2.3.0.ses");
    let dir = tempfile::tempdir().unwrap();
    // The recorded session names `specctra_rounded_board` as its design.
    let board = dir.path().join("specctra_rounded_board.kicad_pcb");
    std::fs::write(&board, board_text).unwrap();

    let rules: IpcEffectiveRoutingRules = ["GND", "VCC"]
        .into_iter()
        .map(|net| {
            (
                net.to_string(),
                IpcRoutingRules {
                    class_name: "Default".into(),
                    constituents: vec!["Default".into()],
                    track_width_mm: Some(0.2),
                    clearance_mm: Some(0.2),
                    via_diameter_mm: Some(0.6),
                    via_drill_mm: Some(0.3),
                },
            )
        })
        .collect();
    let bundle = export_dsn_with_options(
        &board,
        board_text,
        &rules,
        ExportOptions {
            pours: PourPolicy::from_argument(Some(&json!({
                "zones": [{ "name": "GND_TOP", "policy": "omit" }]
            })))
            .unwrap(),
            ..Default::default()
        },
    )
    .unwrap();
    let dsn = dir.path().join("specctra_rounded_board.dsn");
    std::fs::write(&dsn, &bundle.dsn).unwrap();
    std::fs::write(
        dir.path().join("specctra_rounded_board.dsn.konnect.json"),
        &bundle.manifest,
    )
    .unwrap();
    let ses = dir.path().join("specctra_rounded_board.ses");
    std::fs::write(&ses, ses_text).unwrap();

    let root = dir.path().join("search").join("judge");
    let ctx = prepare(&dsn, &root, &cli).expect("preflight");
    std::fs::create_dir_all(&root).unwrap();
    let base = baseline(&ctx).await.expect("the unrouted board's baseline");
    eprintln!("baseline: {base:?}");

    let routed = Routed {
        index: 0,
        shape: crate::route_search::Shape::AsExported,
        seed: 0,
        dsn: dsn.clone(),
        held: crate::fastroute::Held {
            ses,
            unrouted: 0,
            kept: "full",
            passes: vec![],
            seconds: 1.5,
        },
        planes: vec![],
        folder: dir.path().to_path_buf(),
    };
    let tools = Arc::new(crate::tools::ToolContext::new(
        ServerConfig {
            kicad_cli: cli.clone(),
            kicad_binary: String::new(),
            ipc_address: String::new(),
            project_dir: None,
            jlcpcb_db_path: None,
            auto_load_toolsets: false,
            eager_toolsets: false,
        },
        Arc::new(crate::router::ToolRouter::new()),
    ));
    let judged = judge(&ctx, &routed, &base, tools).await.expect("judge");
    eprintln!(
        "measures: {:#?}\ngates: {:#?}",
        judged.measures, judged.gates
    );

    assert!(
        judged.board.starts_with(root.join("run-0")),
        "{}",
        judged.board.display()
    );
    assert!(judged.board.is_file());
    assert!(judged.rounds.is_empty(), "no planes, no stitching rounds");
    let copper =
        judged.measures.signal_mm + judged.measures.power_mm + judged.measures.pour_track_mm;
    assert!(copper > 0.0, "the session's copper was measured");
    assert!(judged.measures.routed_share.is_some(), "the audit ran");
    assert_eq!(judged.measures.route_s, 1.5);
}
