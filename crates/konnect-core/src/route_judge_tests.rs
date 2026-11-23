//! Tests of the best-of-N judge: preflight, gates, rank key, measures. The two
//! functions that need kicad-cli (`baseline` and `judge`) have one ignored test.

use super::*;
use crate::intent_width_report::WidthReport;
use crate::specctra_ses::{SesArc, SesImportPlan, SesTrack, SesVia};
use crate::tools::cli::{DrcReport, DrcViolation, ReportItem};
use sha2::{Digest, Sha256};

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

/// The export hashes KiCad's in-memory text of a board (IPC); KiCad saves the
/// same board in another layout and without each footprint's library metadata.
/// The judge refused every such board until it compared the canonical identity
/// too (found on a live board, 2026-11-18). A board that really changed is still
/// refused, and so is one whose manifest predates the canonical identity.
#[test]
fn prepare_accepts_the_saved_file_of_a_board_exported_from_kicads_memory() {
    use crate::board_identity::tests::{ipc_form, SAVED};
    let live = ipc_form(SAVED);
    let s = setup();
    std::fs::write(&s.board, SAVED).unwrap();

    // A manifest from before the canonical identity: bytes only, refused.
    let mut manifest = manifest_for(&s.board, &live);
    std::fs::write(&s.manifest, manifest.to_string()).unwrap();
    let refused = refusal(prepare(&s.dsn, &s.root, &s.cli));
    assert!(refused.reason.contains("SHA-256"), "{refused:?}");

    // With it, the saved file of the exported board is accepted, and the judge
    // reads the saved text.
    manifest["source_canonical_sha256"] =
        json!(crate::board_identity::canonical_sha256(&live).unwrap());
    std::fs::write(&s.manifest, manifest.to_string()).unwrap();
    let ctx = prepare(&s.dsn, &s.root, &s.cli).expect("the same board, saved");
    assert_eq!(ctx.board_text, SAVED);

    // A board edited after the export is refused, with the same next step.
    std::fs::write(&s.board, SAVED.replacen("(at ", "(at 1", 1)).unwrap();
    let refused = refusal(prepare(&s.dsn, &s.root, &s.cli));
    assert!(
        refused.next_step.contains("save the board in KiCad"),
        "{refused:?}"
    );

    // So is a saved file that does not parse.
    std::fs::write(&s.board, "(kicad_pcb (unclosed").unwrap();
    assert!(prepare(&s.dsn, &s.root, &s.cli).is_err());
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

pub(crate) fn measures() -> Measures {
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
        lengths: None,
        pours: None,
        pours_error: None,
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
            g3: vec![],
            g4: vec![]
        }
    );
    assert!(g.all_pass());
}

/// A row with no share figure and no routed length has share 0, whatever its
/// below-intent figure: nothing routed is below its width.
#[test]
fn a_row_with_no_routed_length_and_no_share_passes_g3() {
    let mut m = measures();
    let mut row = width_row("VBUS", 0.0, 1.0, 0.0, 0.5);
    row.as_object_mut().unwrap().remove("below_intent_share");
    m.width_rows = vec![row];
    assert!(gates(&m, &base(&[])).g3.is_empty());
    // Control: the same row with 1 mm routed is all below its width.
    let mut row = width_row("VBUS", 1.0, 1.0, 0.0, 0.5);
    row.as_object_mut().unwrap().remove("below_intent_share");
    m.width_rows = vec![row];
    assert_eq!(gates(&m, &base(&[])).g3.len(), 1);
}

#[test]
fn all_pass_needs_all_three_lists_empty() {
    let pass = Gates {
        g1: vec![],
        g2: vec![],
        g3: vec![],
        g4: vec![],
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
        Gates {
            g4: vec!["x".into()],
            ..pass.clone()
        },
    ] {
        assert!(!failed.all_pass());
    }
}

/// The gate lists of `m` when exactly one gate is expected to fail with one line.
fn only(g: &Gates, which: usize) {
    let lists = [&g.g1, &g.g2, &g.g3, &g.g4];
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

/// A run's length measures: `miss_mm` outside a target window on one net.
fn length_measures(miss_mm: f64, unmeasured: &[&str]) -> crate::length_probe::LengthMeasures {
    use crate::length_probe::{GroupLengths, LengthMeasures, NetLength};
    let (mm, miss) = if miss_mm > 0.0 {
        (Some(41.5 - miss_mm), miss_mm)
    } else {
        (Some(42.0), 0.0)
    };
    LengthMeasures {
        groups: vec![GroupLengths {
            interface: "ram".into(),
            name: "dq".into(),
            longest_mm: mm,
            nets: vec![NetLength {
                net: "/DQ0".into(),
                kicad_mm: mm,
                low_mm: 41.5,
                high_mm: 42.5,
                miss_mm: miss,
            }],
        }],
        pairs: vec![],
        unmeasured: unmeasured.iter().map(|net| net.to_string()).collect(),
        paths: Default::default(),
        miss_mm,
        probe_s: 0.1,
    }
}

/// G2 stopped counting KiCad's own length and skew violations: against the unrouted
/// board, a run with fewer of them than the baseline used to pass every gate, though
/// the board still broke a rule; G4 counts them, absolute.
#[test]
fn g2_and_g4_split_kicads_length_violations() {
    let b = base(&[("length_out_of_range", 3), ("clearance", 1)]);

    // Before Row LT this run passed G2 (2 is not more than the baseline's 3), and
    // nothing else failed: the board was called clean while breaking two rules.
    let mut m = measures();
    m.violations = rules(&[("length_out_of_range", 2), ("clearance", 1)]);
    let g = gates(&m, &b);
    assert!(g.g2.is_empty(), "G2 does not count the length keys: {g:?}");
    assert_eq!(
        g.g4,
        vec!["KiCad: 2 length_out_of_range violation(s) of the board's own rules"]
    );
    assert!(!g.all_pass());

    // More than the baseline has is no longer a G2 failure either: one gate, one breach.
    m.violations = rules(&[("length_out_of_range", 4), ("skew_out_of_range", 1)]);
    let g = gates(&m, &b);
    assert!(g.g2.is_empty(), "{g:?}");
    assert_eq!(g.g4.len(), 2);

    // The other keys are G2's as before: a clearance over the baseline still fails G2.
    m.violations = rules(&[("clearance", 2)]);
    let g = gates(&m, &b);
    assert_eq!(g.g2.len(), 1, "{g:?}");
    assert!(g.g4.is_empty());

    // Negative control: a run with none of them passes G4.
    m.violations = rules(&[("clearance", 1)]);
    assert!(gates(&m, &b).all_pass());
}

#[test]
fn g4_fails_on_the_intents_lengths_and_names_the_net() {
    let b = base(&[]);
    let mut m = measures();
    m.lengths = Some(length_measures(0.62, &[]));
    let g = gates(&m, &b);
    only(&g, 3);
    assert_eq!(
        g.g4[0],
        "group 'dq' (ram): /DQ0 is 40.88 mm, 0.62 mm below its window 41.50\u{2013}42.50 mm"
    );

    // Everything inside its window: G4 passes.
    m.lengths = Some(length_measures(0.0, &[]));
    assert!(gates(&m, &b).all_pass());

    // A checked net with no length fails G4 on its own.
    m.lengths = Some(length_measures(0.0, &["/DQ0"]));
    only(&gates(&m, &b), 3);

    // Without length measures and without KiCad's own violations G4 is empty.
    m.lengths = None;
    assert!(gates(&m, &b).g4.is_empty());
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

    // A row with no share figure: the share is below over routed. 0.6 of 10 mm
    // fails; 1 of 100 mm passes (the pair pins the division, not just the order).
    let without_share = |routed: f64, below: f64| {
        let mut row = width_row("VBUS", routed, below, 0.0, 0.5);
        row.as_object_mut().unwrap().remove("below_intent_share");
        row
    };
    m.width_rows = vec![without_share(10.0, 0.6)];
    only(&gates(&m, &b), 2);
    m.width_rows = vec![without_share(100.0, 1.0)];
    assert!(gates(&m, &b).all_pass(), "1 % passes");
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

/// The key's positions (L10): G1, G2, G3, G4, intent miss, KiCad's violations, then
/// `KEY_QUALITY` and the route seconds.
const G4: usize = 3;
const MISS: usize = 4;
const KICAD: usize = 5;
const ACUTE: usize = 6;
const SIGNAL: usize = 7;
const POWER: usize = 8;
const RIGHT: usize = 9;
const VIAS: usize = 10;
const QUALITY: usize = 11;
const SECONDS: usize = 12;

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
            g4: vec![],
        },
        board: PathBuf::new(),
        rounds: vec![],
        tune: None,
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
    // [G1, G2, G3, G4, intent miss, KiCad length violations, acute, signal step,
    //  power step, right angles, vias off pour, -copper quality, route seconds]: a lone
    //  run is its own shortest (ratio 1, step 50).
    assert_eq!(
        keys(&[s.clone()]),
        vec![[0, 0, 0, 0, 0, 0, 2, 50, 50, 3, 4, -80, 12]]
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
    assert_eq!(ks[0][SIGNAL], ks[1][SIGNAL], "same signal step");
    assert_eq!(ks[0][SIGNAL], 50);
    assert_eq!(ks[1][POWER], 50);
    assert_eq!(ks[0][POWER], 60);
    assert!(ks[1] < ks[0], "the shorter power wins when signal ties");

    // 102 / 100: one step is 2 %: a different step.
    let mut c = spec();
    c.signal = 102.0;
    let ks = keys(&[spec(), c]);
    assert_eq!(ks[0][SIGNAL], 50);
    assert_eq!(ks[1][SIGNAL], 51);
}

#[test]
fn when_every_length_of_a_category_is_zero_its_step_is_zero() {
    let mut a = spec();
    a.signal = 0.0;
    a.power = 0.0;
    let mut b = a.clone();
    b.acute = Some(3.0);
    for key in keys(&[a, b]) {
        assert_eq!(key[SIGNAL], 0, "signal step");
        assert_eq!(key[POWER], 0, "power step");
    }
}

#[test]
fn a_run_with_none_of_a_category_among_runs_that_have_some_has_step_zero() {
    let mut none = spec();
    none.power = 0.0;
    let ks = keys(&[none, spec()]);
    assert_eq!(ks[0][POWER], 0);
    assert_eq!(
        ks[1][POWER], 50,
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
    assert_eq!(
        (k[ACUTE], k[RIGHT], k[QUALITY]),
        (i64::MAX, i64::MAX, i64::MAX)
    );

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
    assert_eq!(ks[0][QUALITY], -90);
    assert_eq!(ks[1][QUALITY], -90, "89.6 rounds to 90 as well");
    assert_eq!(ks[0][SECONDS], 10);
    assert_eq!(ks[1][SECONDS], 10);
}

#[test]
fn rank_returns_one_key_per_run_in_input_order() {
    assert!(rank(&[]).is_empty());
    let mut worse = spec();
    worse.acute = Some(9.0);
    let ks = keys(&[worse, spec(), spec()]);
    assert_eq!(ks.len(), 3);
    assert_eq!(ks[0][ACUTE], 9);
    assert_eq!(ks[1], ks[2]);
}

/// A judged run from `spec`, with Row LT's length parts set.
fn run_with_lengths(
    s: &Spec,
    g4_failed: bool,
    lengths: Option<crate::length_probe::LengthMeasures>,
    violations: &[(&str, usize)],
) -> Judged {
    let mut judged = run(s);
    if g4_failed {
        judged.gates.g4 = vec!["failed".to_string()];
    }
    judged.measures.lengths = lengths;
    judged.measures.violations = rules(violations);
    judged
}

fn keys_of(runs: &[Judged]) -> Vec<Key> {
    let refs: Vec<&Judged> = runs.iter().collect();
    rank(&refs)
}

#[test]
fn the_key_has_thirteen_places_and_the_quality_measures_are_named() {
    assert_eq!(KEY_LEN, 13);
    assert_eq!(keys(&[spec()])[0].len(), KEY_LEN);
    // The six quality measures: acute corners to minus copper quality.
    assert_eq!(KEY_QUALITY, ACUTE..QUALITY + 1);
    let mut s = spec();
    s.acute = Some(2.0);
    s.right = Some(3.0);
    s.vias = 4;
    s.quality = Some(80.0);
    let key = keys(&[s])[0];
    assert_eq!(key[KEY_QUALITY], [2, 50, 50, 3, 4, -80]);
    assert_eq!(key[VIAS], 4);
    assert_eq!(key[SECONDS], 10);
}

/// Without a length group, a pair skew or a KiCad length rule, positions 3 to 5 are 0
/// for every run, so the order is the one the key had before Row LT (the oracle
/// `rank_before_the_pull_out` has those three places, 0, and nothing else).
#[test]
fn without_lengths_positions_three_to_five_are_zero_and_the_order_is_todays() {
    let with = |change: &dyn Fn(&mut Spec)| {
        let mut s = spec();
        change(&mut s);
        s
    };
    let specs = vec![
        with(&|s| s.failed[2] = true),
        spec(),
        with(&|s| s.acute = Some(3.0)),
        with(&|s| s.signal = 120.0),
        with(&|s| s.vias = 2),
        with(&|s| s.seconds = 99.0),
        with(&|s| s.failed[0] = true),
        with(&|s| s.quality = Some(10.0)),
    ];
    let ks = keys(&specs);
    for key in &ks {
        assert_eq!(key[G4..ACUTE], [0, 0, 0], "{key:?}");
    }
    let judged: Vec<Judged> = specs.iter().map(run).collect();
    let refs: Vec<&Judged> = judged.iter().collect();
    assert_eq!(ks, rank_before_the_pull_out(&refs));
    // Dropping the three places leaves the old ten-entry key, in the old order.
    let mut by_new: Vec<usize> = (0..ks.len()).collect();
    by_new.sort_by_key(|&i| ks[i]);
    let old = |key: &Key| -> Vec<i64> { key[..3].iter().chain(&key[ACUTE..]).copied().collect() };
    let mut by_old: Vec<usize> = (0..ks.len()).collect();
    by_old.sort_by_key(|&i| old(&ks[i]));
    assert_eq!(by_new, by_old);
}

#[test]
fn g4_ranks_after_g3_and_before_every_quality_measure() {
    let clean = spec();
    let mut few_corners = spec();
    few_corners.acute = Some(0.0);
    let mut many_corners = spec();
    many_corners.acute = Some(50.0);
    many_corners.signal = 300.0;
    many_corners.seconds = 500.0;
    let mut g3_failed = spec();
    g3_failed.failed[2] = true;

    let runs = vec![
        run_with_lengths(&g3_failed, false, None, &[]),
        // Fails G4, with the best quality measures.
        run_with_lengths(&few_corners, true, Some(length_measures(0.3, &[])), &[]),
        // Passes G4, with the worst.
        run_with_lengths(&many_corners, false, Some(length_measures(0.0, &[])), &[]),
        run_with_lengths(&clean, false, Some(length_measures(0.0, &[])), &[]),
    ];
    let ks = keys_of(&runs);
    let mut order: Vec<usize> = (0..runs.len()).collect();
    order.sort_by_key(|&i| ks[i]);
    // clean, the long run that meets every length, the one that misses by 0.3 mm, and
    // last the run that failed G3.
    assert_eq!(order, vec![3, 2, 1, 0]);
    assert_eq!(ks[1][G4], 1);
    assert_eq!(ks[2][G4], 0);
    assert_eq!(ks[1][MISS], 30);
    // G3 weighs more than G4.
    assert_eq!((ks[0][2], ks[0][G4]), (1, 0));
}

#[test]
fn among_runs_that_miss_the_smaller_miss_ranks_first_then_fewer_kicad_violations() {
    let s = spec();
    let runs = vec![
        run_with_lengths(&s, true, Some(length_measures(0.50, &[])), &[]),
        run_with_lengths(&s, true, Some(length_measures(0.20, &[])), &[]),
        // The same miss as the second, with two of KiCad's own violations.
        run_with_lengths(
            &s,
            true,
            Some(length_measures(0.20, &[])),
            &[("length_out_of_range", 1), ("skew_out_of_range", 1)],
        ),
        // A checked net with no length: last of the runs that fail G4.
        run_with_lengths(&s, true, Some(length_measures(0.0, &["/DQ0"])), &[]),
        // Only KiCad's own rules, no intent: one violation.
        run_with_lengths(&s, true, None, &[("length_out_of_range", 1)]),
    ];
    let ks = keys_of(&runs);
    assert_eq!(ks[1][MISS], 20);
    assert_eq!(ks[2][KICAD], 2);
    assert_eq!(ks[3][MISS], i64::MAX);
    assert_eq!(ks[4][KICAD], 1);
    let mut order: Vec<usize> = (0..runs.len()).collect();
    order.sort_by_key(|&i| ks[i]);
    // No intent miss (4, with one KiCad violation) < 0.20 mm (1) < 0.20 mm with two
    // KiCad violations (2) < 0.50 mm (0) < a net unmeasured (3).
    assert_eq!(order, vec![4, 1, 2, 0, 3]);

    // KiCad's violations count by key: the clearance key is not among them.
    let run = run_with_lengths(&s, false, None, &[("clearance", 7)]);
    assert_eq!(keys_of(&[run])[0][KICAD], 0);
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
        kicad_config_home: None,
        lengths: None,
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

/// The files of a recorded export: a DSN with its manifest beside it, the saved
/// board (named as the recorded session's design) and the session.
pub(crate) struct Exported {
    pub(crate) dsn: PathBuf,
    pub(crate) ses: PathBuf,
}

/// Exports `specctra_pours_policy.kicad_pcb` (written as `specctra_rounded_board`,
/// the design the recorded session names) with the top pour left out, and writes the
/// recorded Freerouting session of the rounded board beside it.
pub(crate) fn exported(dir: &Path) -> Exported {
    exported_as(dir, false)
}

/// [`exported`]; with `over_ipc` the DSN and its manifest come from the board as
/// KiCad's memory writes it (`board_identity::tests::ipc_form`), while the file
/// beside them stays the saved text: the live board of a real export.
fn exported_as(dir: &Path, over_ipc: bool) -> Exported {
    use crate::specctra::{export_dsn_with_options, ExportOptions, PourPolicy};
    use konnect_ipc::{IpcEffectiveRoutingRules, IpcRoutingRules};

    let board_text = include_str!("../tests/fixtures/specctra_pours_policy.kicad_pcb");
    let ses_text = include_str!("../tests/fixtures/specctra_rounded_board.freerouting-2.3.0.ses");
    let board = dir.join("specctra_rounded_board.kicad_pcb");
    std::fs::write(&board, board_text).unwrap();
    let exported_text = if over_ipc {
        crate::board_identity::tests::ipc_form(board_text)
    } else {
        board_text.to_string()
    };

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
        &exported_text,
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
    let dsn = dir.join("specctra_rounded_board.dsn");
    std::fs::write(&dsn, &bundle.dsn).unwrap();
    std::fs::write(
        dir.join("specctra_rounded_board.dsn.konnect.json"),
        &bundle.manifest,
    )
    .unwrap();
    let ses = dir.join("specctra_rounded_board.ses");
    std::fs::write(&ses, ses_text).unwrap();
    Exported { dsn, ses }
}

/// The tool context the judge's registry calls run in.
fn tool_context(cli: &str) -> Arc<crate::tools::ToolContext> {
    Arc::new(crate::tools::ToolContext::new(
        crate::tools::ServerConfig {
            kicad_cli: cli.to_string(),
            kicad_binary: String::new(),
            ipc_address: String::new(),
            project_dir: None,
            jlcpcb_db_path: None,
            auto_load_toolsets: false,
            eager_toolsets: false,
        },
        Arc::new(crate::router::ToolRouter::new()),
    ))
}

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
    assert!(
        std::env::var_os("KICAD_CONFIG_HOME").is_some(),
        "LEAD: set KICAD_CONFIG_HOME to a scratch folder, or KiCad re-saves the real settings"
    );
    let cli = std::env::var("KONNECT_TEST_KICAD_CLI")
        .unwrap_or_else(|_| "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli".to_string());
    let dir = tempfile::tempdir().unwrap();
    let Exported { dsn, ses } = exported(dir.path());

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
            untuned: None,
            tune: Default::default(),
            group_search: None,
        },
        planes: vec![],
        folder: dir.path().to_path_buf(),
    };
    let tools = tool_context(&cli);
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

// ------------------------------------------------- rank reference (G1-passing runs)

#[test]
fn a_run_that_fails_g1_does_not_set_the_length_reference() {
    // Two complete runs whose signal lengths are inside one 2 % step: the vias decide.
    let mut a = spec();
    a.signal = 100.0;
    a.vias = 1;
    let mut b = spec();
    b.signal = 100.5;
    b.vias = 0;
    // A degenerate run that is not complete, with almost no signal track. Were it the
    // reference, every step would be fifty times finer and length would decide first.
    let mut degenerate = spec();
    degenerate.signal = 1.0;
    degenerate.failed[0] = true;

    let ks = keys(&[a.clone(), b.clone(), degenerate]);
    assert_eq!(
        ks[0][SIGNAL], 50,
        "stepped against the complete runs' shortest"
    );
    assert_eq!(ks[1][SIGNAL], 50);
    assert!(ks[1] < ks[0], "so the vias off the pour decide: {ks:?}");
    // The two complete runs rank as they do without the degenerate one.
    let alone = keys(&[a, b]);
    assert_eq!(alone[0][SIGNAL..], ks[0][SIGNAL..]);
    assert_eq!(alone[1][SIGNAL..], ks[1][SIGNAL..]);
    // And the degenerate run is last, by G1.
    assert!(ks[2] > ks[0] && ks[2] > ks[1]);
}

#[test]
fn when_no_run_passes_g1_the_reference_is_the_shortest_of_all() {
    let mut a = spec();
    a.signal = 100.0;
    a.failed[0] = true;
    let mut b = spec();
    b.signal = 200.0;
    b.failed[0] = true;
    let ks = keys(&[a, b]);
    assert_eq!(ks[0][SIGNAL], 50);
    assert_eq!(ks[1][SIGNAL], 100);
}

// ------------------------------------------------- rank_measures (the pool)

/// `rank_measures` over specs, each with its pool flag.
fn pooled(specs: &[(Spec, bool)]) -> Vec<Key> {
    let judged: Vec<Judged> = specs.iter().map(|(s, _)| run(s)).collect();
    let runs: Vec<(&Measures, &Gates, bool)> = judged
        .iter()
        .zip(specs)
        .map(|(j, (_, in_pool))| (&j.measures, &j.gates, *in_pool))
        .collect();
    rank_measures(&runs)
}

/// `rank` exactly as it was before `rank_measures` was pulled out of it: the oracle
/// the pulled-out version is held to, since `rank` now calls `rank_measures` and a
/// change inside it would otherwise change both sides of a comparison alike.
fn rank_before_the_pull_out(judged: &[&Judged]) -> Vec<Key> {
    let complete: Vec<&Judged> = judged
        .iter()
        .copied()
        .filter(|run| run.gates.g1.is_empty())
        .collect();
    let pool: &[&Judged] = if complete.is_empty() {
        judged
    } else {
        &complete
    };
    let shortest = |length: fn(&Measures) -> f64| -> Option<f64> {
        pool.iter()
            .map(|run| length(&run.measures))
            .filter(|mm| *mm > 0.0)
            .fold(None, |least: Option<f64>, mm| {
                Some(least.map_or(mm, |least| least.min(mm)))
            })
    };
    let step = |mm: f64, shortest: Option<f64>| -> i64 {
        match shortest {
            Some(shortest) if mm > 0.0 => (mm / shortest / LENGTH_STEP).round() as i64,
            _ => 0,
        }
    };
    let shortest_signal = shortest(|m| m.signal_mm);
    let shortest_power = shortest(|m| m.power_mm);
    let whole = |value: Option<f64>| value.map_or(i64::MAX, |value| value.round() as i64);
    judged
        .iter()
        .map(|run| {
            let (m, g) = (&run.measures, &run.gates);
            [
                i64::from(!g.g1.is_empty()),
                i64::from(!g.g2.is_empty()),
                i64::from(!g.g3.is_empty()),
                // Row LT: G4, the intent miss and KiCad's length violations are 0 for
                // every run that checks no length.
                0,
                0,
                0,
                whole(m.acute),
                step(m.signal_mm, shortest_signal),
                step(m.power_mm, shortest_power),
                whole(m.right_angles),
                m.vias_off_pour as i64,
                m.copper_quality
                    .map_or(i64::MAX, |quality| -(quality.round() as i64)),
                m.route_s.max(0.0).floor() as i64,
            ]
        })
        .collect()
}

#[test]
fn rank_is_rank_measures_with_every_run_in_the_pool() {
    let mut degenerate = spec();
    degenerate.signal = 1.0;
    degenerate.power = 0.5;
    degenerate.failed[0] = true;
    let mut long = spec();
    long.signal = 150.0;
    long.vias = 2;
    let mut far = spec();
    far.signal = 102.0;
    far.acute = Some(4.0);
    far.seconds = 33.7;
    let mut failed_g2 = spec();
    failed_g2.failed[1] = true;
    failed_g2.signal = 80.0;
    let mut failed_g1_short = spec();
    failed_g1_short.failed[0] = true;
    failed_g1_short.signal = 40.0;
    let mut unknown = spec();
    unknown.acute = None;
    unknown.quality = None;
    let mut no_power = spec();
    no_power.power = 0.0;

    let sets: Vec<Vec<Spec>> = vec![
        vec![],
        vec![spec()],
        // A short run that fails G1 beside complete runs: it must not set the steps.
        vec![spec(), long.clone(), degenerate.clone(), far.clone()],
        // No run passes G1: the steps are against the shortest of all.
        vec![degenerate.clone(), failed_g1_short.clone()],
        vec![
            spec(),
            long,
            far,
            failed_g2,
            degenerate,
            failed_g1_short,
            unknown,
            no_power,
        ],
    ];
    for specs in sets {
        let marked: Vec<(Spec, bool)> = specs.iter().cloned().map(|s| (s, true)).collect();
        let judged: Vec<Judged> = specs.iter().map(run).collect();
        let refs: Vec<&Judged> = judged.iter().collect();
        let before = rank_before_the_pull_out(&refs);
        assert_eq!(keys(&specs), before, "{} run(s): rank", specs.len());
        assert_eq!(
            pooled(&marked),
            before,
            "{} run(s): rank_measures",
            specs.len()
        );
    }
}

#[test]
fn a_short_run_that_fails_g1_does_not_set_the_steps_of_rank_measures() {
    let mut short_failed = spec();
    short_failed.signal = 1.0;
    short_failed.failed[0] = true;
    let ks = pooled(&[(spec(), true), (short_failed, true)]);
    assert_eq!(
        ks[0][SIGNAL], 50,
        "stepped against the complete run's own length"
    );
    assert_eq!(
        ks[1][SIGNAL], 1,
        "1 / 100 is a step of 0.5, which rounds to 1"
    );
    assert_eq!(ks[1][0], 1, "and it fails G1");
}

#[test]
fn a_short_run_outside_the_pool_does_not_set_the_length_steps() {
    // The short run passes every gate, so only the pool flag keeps it from the steps.
    let mut short = spec();
    short.signal = 10.0;
    short.power = 1.0;
    let outside = pooled(&[(spec(), true), (short.clone(), false)]);
    let inside = pooled(&[(spec(), true), (short, true)]);
    assert_eq!(
        outside[0][SIGNAL..POWER + 1],
        [50, 50],
        "the pool's run is its own reference"
    );
    assert_eq!(
        inside[0][SIGNAL..POWER + 1],
        [500, 500],
        "in the pool, the short run is the reference"
    );
    // The run outside the pool still has a key, stepped against the pool: below 50.
    assert_eq!(outside[1][SIGNAL..POWER + 1], [5, 5]);
    assert!(
        outside[1] < outside[0],
        "shorter, so it ranks above, and sets nothing"
    );
}

#[test]
fn rank_measures_falls_back_by_the_pool_then_to_every_run() {
    let mut short_ok = spec();
    short_ok.signal = 50.0;
    let mut mid_failed = spec();
    mid_failed.signal = 100.0;
    mid_failed.failed[0] = true;
    let mut long_failed = spec();
    long_failed.signal = 200.0;
    long_failed.failed[0] = true;

    // The pool holds only runs that fail G1: the shortest of the pool (100), though a
    // run outside it passes G1 with 50.
    let ks = pooled(&[
        (short_ok.clone(), false),
        (mid_failed.clone(), true),
        (long_failed.clone(), true),
    ]);
    assert_eq!(ks[1][SIGNAL], 50);
    assert_eq!(ks[2][SIGNAL], 100);
    assert_eq!(ks[0][SIGNAL], 25, "50 / 100 / 0.02");

    // Nothing is in the pool: the shortest of every run (50).
    let ks = pooled(&[(short_ok, false), (mid_failed, false), (long_failed, false)]);
    assert_eq!(ks[0][SIGNAL], 50);
    assert_eq!(ks[1][SIGNAL], 100);
    assert_eq!(ks[2][SIGNAL], 200);

    assert!(rank_measures(&[]).is_empty());
}

#[test]
fn measures_and_gates_survive_a_round_trip() {
    let mut m = measures();
    m.unconnected = 3;
    m.violations = BTreeMap::from([
        ("clearance".to_string(), 2),
        ("shorting_items".to_string(), 1),
    ]);
    m.routed_share = Some(0.987_654_321);
    m.audit_complete = Some(false);
    m.acute = None;
    m.right_angles = Some(12.0);
    m.copper_quality = None;
    m.nets_undersized = Some(1.0);
    m.narrow_paths = None;
    m.signal_mm = 1_234.567_890_123;
    m.power_mm = 0.0;
    m.pour_track_mm = 55.5;
    m.vias_off_pour = 7;
    m.below_floor = 2;
    m.width_rows = vec![
        width_row("VCC", 40.0, 2.0, 0.05, 1.5),
        width_row("/SIG", 10.0, 0.0, 0.0, 0.0),
    ];
    m.longest_below_mm = BTreeMap::from([("VCC".to_string(), 1.5)]);
    m.stitching_vias = 4;
    m.route_s = 12.25;
    m.lengths = Some(length_measures(0.62, &["/DQ9"]));
    let g = Gates {
        g1: vec!["3 unconnected item(s)".to_string()],
        g2: vec![],
        g3: vec!["VCC: a 1.50 mm stretch".to_string(), "another".to_string()],
        g4: vec!["group 'dq' (ram): /DQ0 is 40.88 mm".to_string()],
    };

    let text = serde_json::to_string(&json!({ "measures": m, "gates": g })).unwrap();
    let back: Value = serde_json::from_str(&text).unwrap();
    let m_back: Measures = serde_json::from_value(back["measures"].clone()).unwrap();
    let g_back: Gates = serde_json::from_value(back["gates"].clone()).unwrap();
    assert_eq!(m_back, m);
    assert_eq!(g_back, g);
    // An unset measure stays unset, and does not become 0.
    assert_eq!(m_back.acute, None);
    assert_eq!(m_back.copper_quality, None);
    assert_eq!(m_back.narrow_paths, None);
}

/// A record written before Row LT has neither `lengths` nor `g4`: it still reads, and
/// a run with no length measures does not write the field.
#[test]
fn a_record_from_before_row_lt_still_reads() {
    let m = measures();
    let mut value = serde_json::to_value(&m).unwrap();
    assert!(value.get("lengths").is_none(), "{value}");
    value.as_object_mut().unwrap().remove("lengths");
    let back: Measures = serde_json::from_value(value).unwrap();
    assert_eq!(back.lengths, None);

    let old: Gates = serde_json::from_value(json!({ "g1": [], "g2": ["x"], "g3": [] })).unwrap();
    assert_eq!(old.g4, Vec::<String>::new());
    assert!(!old.all_pass());
}

// ------------------------------------------------------- the private KiCad settings

fn put(folder: &Path, relative: &str, text: &str) {
    let path = folder.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn the_settings_version_under_kicad_config_home_is_the_highest_present() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(config_source_in(dir.path()), None);
    // A file is not a settings folder.
    put(dir.path(), "10.0", "not a folder");
    assert_eq!(config_source_in(dir.path()), None);
    std::fs::remove_file(dir.path().join("10.0")).unwrap();
    for (version, expected) in [("8.0", "8.0"), ("9.0", "9.0"), ("10.0", "10.0")] {
        std::fs::create_dir(dir.path().join(version)).unwrap();
        assert_eq!(
            config_source_in(dir.path()),
            Some((dir.path().join(expected), expected.to_string()))
        );
    }
}

#[test]
fn the_private_settings_copy_holds_the_version_folder_and_leaves_the_source_alone() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("real").join("10.0");
    put(&source, "kicad_common.json", "{\"a\": 1}");
    put(&source, "colors/user.json", "{}");
    let into = dir.path().join("private");

    let config = copy_kicad_config(Some((source.clone(), "10.0".to_string())), &into).unwrap();
    assert_eq!(config.home, into, "the value for KICAD_CONFIG_HOME");
    assert!(config.skipped_links.is_empty());
    assert_eq!(
        std::fs::read_to_string(into.join("10.0/kicad_common.json")).unwrap(),
        "{\"a\": 1}"
    );
    assert!(into.join("10.0/colors/user.json").is_file());
    // Nothing else is in the copy.
    assert_eq!(std::fs::read_dir(&into).unwrap().count(), 1);
    // Writing to the copy does not reach the source.
    std::fs::write(into.join("10.0/kicad_common.json"), "changed").unwrap();
    assert_eq!(
        std::fs::read_to_string(source.join("kicad_common.json")).unwrap(),
        "{\"a\": 1}"
    );
}

#[test]
fn with_no_settings_to_copy_the_private_folder_is_empty_and_ready() {
    let dir = tempfile::tempdir().unwrap();
    let into = dir.path().join("private");
    let config = copy_kicad_config(None, &into).unwrap();
    assert_eq!(config.home, into);
    assert!(config.skipped_links.is_empty());
    assert!(into.is_dir());
    assert_eq!(std::fs::read_dir(&into).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn a_link_in_the_settings_is_left_out_and_reported_and_a_linked_version_folder_is_read() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside");
    put(&outside, "secret.json", "{}");

    // A link inside the version folder is left out of the copy and named; the
    // rest is copied, and the call goes on.
    let source = dir.path().join("real/10.0");
    put(&source, "kicad_common.json", "{}");
    symlink(outside.join("secret.json"), source.join("linked.json")).unwrap();
    symlink(&outside, source.join("linked-folder")).unwrap();
    let p1 = dir.path().join("p1");
    let config = copy_kicad_config(Some((source.clone(), "10.0".into())), &p1).unwrap();
    // The copy reads the resolved folder (on macOS the temp folder sits behind /var).
    let read_from = std::fs::canonicalize(&source).unwrap();
    assert_eq!(
        config.skipped_links,
        vec![
            read_from.join("linked-folder").display().to_string(),
            read_from.join("linked.json").display().to_string()
        ]
    );
    assert!(p1.join("10.0/kicad_common.json").is_file());
    for name in ["linked.json", "linked-folder"] {
        assert!(
            std::fs::symlink_metadata(p1.join("10.0").join(name)).is_err(),
            "{name}"
        );
    }

    // The version folder itself a link: read through it, copied as a real folder.
    let real = dir.path().join("elsewhere");
    put(&real, "kicad_common.json", "{\"b\": 2}");
    std::fs::create_dir_all(dir.path().join("root2")).unwrap();
    symlink(&real, dir.path().join("root2/10.0")).unwrap();
    let p2 = dir.path().join("p2");
    let config =
        copy_kicad_config(Some((dir.path().join("root2/10.0"), "10.0".into())), &p2).unwrap();
    assert!(config.skipped_links.is_empty());
    let copied = std::fs::symlink_metadata(p2.join("10.0")).unwrap();
    assert!(copied.is_dir() && !copied.file_type().is_symlink());
    assert_eq!(
        std::fs::read_to_string(p2.join("10.0/kicad_common.json")).unwrap(),
        "{\"b\": 2}"
    );
}

// ------------------------------------------------------- removing an earlier round

#[test]
fn an_earlier_round_folder_is_removed_whole() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("judge");
    let round = root.join("run-3/r1");
    put(&round, "board.kicad_pcb", "(kicad_pcb)");
    put(&round, "board.drc.json", "{}");
    put(&round, "lib.pretty/a.kicad_mod", "x");
    put(&round, ".konnect/design_intent.json", "{}");
    put(&root, "run-3/r2/board.kicad_pcb", "(kicad_pcb)");

    remove_round_folder(&root, &round.join("board.kicad_pcb")).unwrap();
    assert!(!round.exists());
    assert!(
        root.join("run-3/r2/board.kicad_pcb").is_file(),
        "the other rounds stay"
    );
    // A folder that is already gone is fine.
    remove_round_folder(&root, &round.join("board.kicad_pcb")).unwrap();
}

#[test]
fn only_a_round_folder_below_the_root_is_removed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("judge");
    put(&root, "run-0/r0/board.kicad_pcb", "x");
    put(&root, "run-0/other/board.kicad_pcb", "x");
    put(dir.path(), "outside/r0/board.kicad_pcb", "x");
    put(&root, "board.kicad_pcb", "x");

    // Not named r<number>, outside the root, or the root itself.
    assert!(remove_round_folder(&root, &root.join("run-0/other/board.kicad_pcb")).is_err());
    assert!(remove_round_folder(&root, &root.join("run-0/board.kicad_pcb")).is_err());
    assert!(remove_round_folder(&root, &dir.path().join("outside/r0/board.kicad_pcb")).is_err());
    assert!(remove_round_folder(&root, &root.join("board.kicad_pcb")).is_err());
    assert!(root.join("run-0/other/board.kicad_pcb").is_file());
    assert!(dir.path().join("outside/r0/board.kicad_pcb").is_file());
    // The honest round folder is removed (the pair).
    remove_round_folder(&root, &root.join("run-0/r0/board.kicad_pcb")).unwrap();
    assert!(!root.join("run-0/r0").exists());
}

#[cfg(unix)]
#[test]
fn a_round_folder_that_is_a_link_is_not_followed_or_removed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("judge");
    let outside = dir.path().join("outside");
    put(&outside, "board.kicad_pcb", "precious");
    std::fs::create_dir_all(root.join("run-0")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("run-0/r1")).unwrap();

    let error = remove_round_folder(&root, &root.join("run-0/r1/board.kicad_pcb")).unwrap_err();
    assert!(format!("{error:#}").contains("link"), "{error:#}");
    assert_eq!(
        std::fs::read_to_string(outside.join("board.kicad_pcb")).unwrap(),
        "precious"
    );
    assert!(std::fs::symlink_metadata(root.join("run-0/r1")).is_ok());
}

// ------------------------------------------------ Row LT, L22 and L23: G4(d), tune_harm

/// A judged run to compare, from the plain run, with its counts set. The plain run has no
/// undersized net and no narrow path, a routed share of 1 and a complete audit.
fn counted(unconnected: usize, violations: &[(&str, usize)], below: usize) -> Judged {
    let mut judged = run(&spec());
    judged.measures.unconnected = unconnected;
    judged.measures.violations = rules(violations);
    judged.measures.below_floor = below;
    judged
}

#[test]
fn tune_harm_names_each_measure_that_tuning_made_worse_with_both_values() {
    let untuned = counted(21, &[("clearance", 2)], 0);
    // One measure worse at a time.
    let worse: Vec<(Judged, &str)> = vec![
        (
            counted(25, &[("clearance", 2)], 0),
            "unconnected items 21 \u{2192} 25",
        ),
        (
            counted(21, &[("clearance", 3)], 0),
            "clearance violations 2 \u{2192} 3",
        ),
        (
            counted(21, &[("clearance", 2), ("shorting_items", 1)], 0),
            "shorting_items violations 0 \u{2192} 1",
        ),
        (
            counted(21, &[("clearance", 2)], 4),
            "tracks below their floor 0 \u{2192} 4",
        ),
    ];
    for (tuned, line) in worse {
        assert_eq!(
            tune_harm(&untuned, &tuned),
            vec![line.to_string()],
            "{line}"
        );
    }
    // The audit's measures, by the numbers: a count that stays one gate line is still worse.
    let audit = |change: &dyn Fn(&mut Measures)| {
        let mut tuned = counted(21, &[("clearance", 2)], 0);
        change(&mut tuned.measures);
        tune_harm(&untuned, &tuned)
    };
    let mut narrow = untuned.clone();
    narrow.measures.narrow_paths = Some(1.0);
    let mut worse_narrow = narrow.clone();
    worse_narrow.measures.narrow_paths = Some(30.0);
    assert_eq!(
        tune_harm(&narrow, &worse_narrow),
        vec!["narrow paths 1 \u{2192} 30".to_string()]
    );
    assert_eq!(
        audit(&|m| m.nets_undersized = Some(2.0)),
        vec!["nets undersized 0 \u{2192} 2".to_string()]
    );
    assert_eq!(
        audit(&|m| m.routed_share = Some(0.99)),
        vec!["routed share 1 \u{2192} 0.99".to_string()]
    );
    assert_eq!(
        audit(&|m| m.routed_share = None),
        vec!["routed share 1 \u{2192} none".to_string()]
    );
    assert_eq!(
        audit(&|m| m.audit_complete = Some(false)),
        vec!["audit completion true \u{2192} false".to_string()]
    );
    assert_eq!(
        audit(&|m| m.audit_complete = None),
        vec!["audit completion true \u{2192} none".to_string()]
    );
    // Everything worse: one line each, in this order.
    let mut all = counted(22, &[("clearance", 5), ("silk_overlap", 1)], 2);
    all.measures.nets_undersized = Some(1.0);
    all.measures.narrow_paths = Some(1.0);
    all.measures.routed_share = Some(0.5);
    all.measures.audit_complete = Some(false);
    let lines = tune_harm(&untuned, &all);
    assert_eq!(lines.len(), 8, "{lines:?}");
    let starts = [
        "unconnected items",
        "clearance",
        "silk_overlap",
        "nets undersized",
        "narrow paths",
        "tracks below",
        "routed share",
        "audit completion",
    ];
    for (line, start) in lines.iter().zip(starts) {
        assert!(line.starts_with(start), "{lines:?}");
    }
}

#[test]
fn tune_harm_gives_nothing_for_equal_or_better_measures() {
    let mut untuned = counted(21, &[("clearance", 2), ("track_dangling", 3)], 1);
    untuned.measures.narrow_paths = Some(4.0);
    untuned.measures.nets_undersized = Some(2.0);
    untuned.measures.routed_share = Some(0.9);
    untuned.measures.audit_complete = Some(false);
    // Equal.
    assert!(tune_harm(&untuned, &untuned.clone()).is_empty());
    // Better in every measure, and a type that vanished.
    let mut better = counted(18, &[("clearance", 1)], 0);
    better.measures.narrow_paths = Some(0.0);
    better.measures.nets_undersized = Some(0.0);
    better.measures.routed_share = Some(1.0);
    better.measures.audit_complete = Some(true);
    assert!(tune_harm(&untuned, &better).is_empty());
    // Not harm: a failing gate whose lines differ, the judge's stitching vias, and a
    // measure the audit gave nowhere.
    let mut other = untuned.clone();
    other.gates.g1 = vec!["still open".to_string()];
    other.gates.g3 = vec!["other text".to_string(), "more text".to_string()];
    other.measures.stitching_vias = 9;
    assert!(tune_harm(&untuned, &other).is_empty());
    let mut unknown = untuned.clone();
    unknown.measures.nets_undersized = None;
    unknown.measures.narrow_paths = None;
    assert!(tune_harm(&untuned, &unknown).is_empty());
    let mut was_none = untuned.clone();
    was_none.measures.routed_share = None;
    was_none.measures.audit_complete = None;
    assert!(tune_harm(&was_none, &untuned).is_empty());
}

#[test]
fn tune_harm_leaves_kicads_length_and_skew_violations_to_g4() {
    let untuned = counted(0, &[], 0);
    let tuned = counted(
        0,
        &[("length_out_of_range", 9), ("skew_out_of_range", 1)],
        0,
    );
    assert!(tune_harm(&untuned, &tuned).is_empty());
    // Control: any other new type is harm.
    let tuned = counted(0, &[("length_out_of_range", 9), ("clearance", 1)], 0);
    assert_eq!(
        tune_harm(&untuned, &tuned),
        vec!["clearance violations 0 \u{2192} 1"]
    );
}

#[test]
fn g4_holds_the_contact_lines_of_a_faulted_net_and_a_clean_run_has_none() {
    let mut m = measures();
    let mut lengths = length_measures(0.0, &[]);
    lengths.paths = BTreeMap::from([(
        "/DQ0".to_string(),
        crate::length_probe::PathFault {
            contacts: 3,
            dangling: 1,
        },
    )]);
    m.lengths = Some(lengths);
    let g = gates(&m, &base(&[]));
    assert_eq!(g.g4.len(), 2, "{:?}", g.g4);
    assert!(g.g4[0].starts_with("/DQ0's copper crosses or overlaps itself 3 time(s)"));
    assert!(g.g4[1].starts_with("/DQ0 has 1 dangling or unconnected item(s)"));
    assert!(g.g1.is_empty() && g.g2.is_empty() && g.g3.is_empty());
    // Control: the same measures with no fault pass.
    m.lengths = Some(length_measures(0.0, &[]));
    assert!(gates(&m, &base(&[])).g4.is_empty());
}

fn held_with_candidates(variants: &[crate::fastroute::TuneVariant]) -> crate::fastroute::Held {
    use crate::fastroute::{Held, TuneCandidate, TuneOutcome};
    let row = |variant: crate::fastroute::TuneVariant, outcome: &str| json!({ "name": "tune", "outcome": outcome, "unrouted": 0, "seconds": 2.0, "variant": variant, "round": 1 });
    Held {
        ses: PathBuf::from("/run/full/full.ses"),
        unrouted: 4,
        kept: "polish",
        passes: vec![
            json!({ "name": "full", "outcome": "kept", "unrouted": 4, "seconds": 9.0 }),
            row(crate::fastroute::TuneVariant::Free, "candidate"),
        ],
        seconds: 20.0,
        untuned: Some(PathBuf::from("/run/full/full.ses")),
        tune: TuneOutcome {
            candidates: variants
                .iter()
                .enumerate()
                .map(|(at, variant)| TuneCandidate {
                    variant: *variant,
                    ses: PathBuf::from(format!("/run/{}/tune.ses", variant.folder_name(1))),
                    unrouted: at,
                    dropped: vec![],
                    passes: 1,
                })
                .collect(),
            rows: variants
                .iter()
                .map(|variant| row(*variant, "candidate"))
                .collect(),
            seconds: 4.0,
        },
        group_search: None,
    }
}

#[test]
fn a_chosen_candidate_rewrites_the_held_run_to_its_board() {
    use crate::fastroute::TuneVariant::{Free, PourSafe};
    let mut held = held_with_candidates(&[Free, PourSafe]);
    let choice = TuneChoice {
        chosen: Some(PourSafe),
        rows: vec![],
        untuned: measures(),
    };
    choice.apply(&mut held).unwrap();
    assert_eq!(held.ses, PathBuf::from("/run/tune-pour-safe/tune.ses"));
    assert_eq!(held.kept, "tune");
    assert_eq!(held.unrouted, 1, "the pour-safe candidate's own count");
    // The run's Tune row is the chosen candidate's, and says kept.
    assert_eq!(held.passes.len(), 2);
    assert_eq!(held.passes[1]["variant"], json!("pour_safe"));
    assert_eq!(held.passes[1]["outcome"], json!("kept"));
    // In `tune.rows` the chosen candidate's pass says kept and the other not_chosen.
    let outcomes: Vec<&str> = held
        .tune
        .rows
        .iter()
        .map(|row| row["outcome"].as_str().unwrap())
        .collect();
    assert_eq!(outcomes, ["not_chosen", "kept"]);
    // The untuned session stays on record.
    assert_eq!(held.untuned, Some(PathBuf::from("/run/full/full.ses")));
    // A run the judge never saw says `candidate` for each of its rows.
    let unjudged = held_with_candidates(&[Free, PourSafe]);
    assert!(unjudged
        .tune
        .rows
        .iter()
        .all(|row| row["outcome"] == json!("candidate")));
}

/// Row GS: a chosen Tune candidate (whose report has no group search) leaves the
/// group search of the routing pass it started from, and so does no choice.
#[test]
fn a_tune_choice_keeps_the_routing_passs_group_search() {
    use crate::fastroute::TuneVariant::{Free, PourSafe};
    let object = json!({ "ended_by": "converged", "passes": [] });
    for chosen in [Some(PourSafe), Some(Free), None] {
        let mut held = held_with_candidates(&[Free, PourSafe]);
        held.group_search = Some(object.clone());
        TuneChoice {
            chosen,
            rows: vec![],
            untuned: measures(),
        }
        .apply(&mut held)
        .unwrap();
        assert_eq!(held.group_search, Some(object.clone()), "{chosen:?}");
    }
}

#[test]
fn no_candidate_chosen_leaves_the_untuned_board_and_says_why_in_the_tune_row() {
    use crate::fastroute::TuneVariant::{Free, PourSafe};
    let mut held = held_with_candidates(&[Free, PourSafe]);
    let choice = TuneChoice {
        chosen: None,
        rows: vec![
            json!({ "variant": "free", "rejected": ["unconnected items 21 \u{2192} 25"] }),
            json!({ "variant": "pour_safe", "rejected": [], "reason": "ranked no better than the untuned board" }),
        ],
        untuned: measures(),
    };
    choice.apply(&mut held).unwrap();
    assert_eq!(held.ses, PathBuf::from("/run/full/full.ses"));
    assert_eq!((held.kept, held.unrouted), ("polish", 4));
    let row = &held.passes[1];
    assert_eq!(row["outcome"], json!("discarded"));
    let reason = row["reason"].as_str().unwrap();
    assert!(
        reason.starts_with("the judge kept the untuned board: "),
        "{reason}"
    );
    assert!(
        reason.contains("free: unconnected items 21 \u{2192} 25"),
        "{reason}"
    );
    assert!(
        reason.contains("pour_safe: ranked no better than the untuned board"),
        "{reason}"
    );
    assert!(held
        .tune
        .rows
        .iter()
        .all(|row| row["outcome"] == json!("not_chosen")));
}

#[test]
fn a_choice_the_run_cannot_follow_is_an_error_and_changes_nothing() {
    use crate::fastroute::TuneVariant::{Free, PourSafe};
    let mut held = held_with_candidates(&[Free]);
    let before = held.clone();
    let error = TuneChoice {
        chosen: Some(PourSafe),
        rows: vec![],
        untuned: measures(),
    }
    .apply(&mut held)
    .unwrap_err();
    assert!(format!("{error:#}").contains("PourSafe"), "{error:#}");
    assert_eq!(held.ses, before.ses);
    assert_eq!(held.kept, before.kept);
    assert_eq!(held.tune.rows, before.tune.rows);
    // Control: a candidate the run holds is followed.
    TuneChoice {
        chosen: Some(Free),
        rows: vec![],
        untuned: measures(),
    }
    .apply(&mut held)
    .unwrap();
    assert_eq!(held.kept, "tune");
}

#[test]
fn the_untuned_reason_names_each_candidates_lines_or_reason() {
    let choice = TuneChoice {
        chosen: None,
        rows: vec![
            json!({ "variant": "free", "rejected": ["a", "b"] }),
            json!({ "variant": "pour_safe", "rejected": [], "reason": "not judged" }),
            json!({ "variant": "free", "rejected": [] }),
        ],
        untuned: measures(),
    };
    assert_eq!(
        choice.untuned_reason(),
        "free: a; b / pour_safe: not judged"
    );
}

// -------------------------------- the round logic, with a fake kicad-cli (no KiCad)

#[cfg(unix)]
mod with_a_fake_kicad_cli {
    use super::*;
    use std::sync::Mutex;

    /// Stands in for `kicad-cli pcb drc --output <json> --format json --refill-zones
    /// --save-board <board>`: records `KICAD_CONFIG_HOME` and its arguments in
    /// `calls.log`, counts its calls in `counter`, copies the call's canned report
    /// (`drc.<n>.json`, else `drc.default.json`) to the `--output` path, and leaves
    /// the board as it is, or, when the call has a `board.<n>` file, saves that over the
    /// board (as `--save-board` saves the refilled board).
    const FAKE_CLI: &str = r#"#!/bin/sh
dir="$(cd "$(dirname "$0")" && pwd)"
echo "KICAD_CONFIG_HOME=${KICAD_CONFIG_HOME-unset} $*" >> "$dir/calls.log"
out=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--output" ]; then out="$a"; fi
  prev="$a"
done
n=$(cat "$dir/counter" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" > "$dir/counter"
if [ -f "$dir/drc.$n.json" ]; then cp "$dir/drc.$n.json" "$out"; else cp "$dir/drc.default.json" "$out"; fi
if [ -f "$dir/board.$n" ]; then cp "$dir/board.$n" "$prev"; fi
exit 0
"#;

    /// A KiCad DRC report: one unconnected item per net in `open` (named as KiCad names
    /// a pad item), and one violation per rule in `violations`.
    fn drc_json(open: &[&str], violations: &[&str]) -> String {
        json!({
            "violations": violations
                .iter()
                .map(|rule| json!({
                    "type": rule, "severity": "error", "description": "a violation", "items": []
                }))
                .collect::<Vec<_>>(),
            "unconnected_items": open
                .iter()
                .map(|net| json!({
                    "type": "unconnected_items",
                    "severity": "error",
                    "description": "Missing connection between items",
                    "items": [{
                        "description": format!("Pad 1 [{net}] of R1 on F.Cu"),
                        "pos": {"x": 1.0, "y": 2.0}
                    }]
                }))
                .collect::<Vec<_>>(),
        })
        .to_string()
    }

    struct Harness {
        dir: tempfile::TempDir,
        ctx: JudgeContext,
        routed: Routed,
        fake: PathBuf,
        config: PathBuf,
    }

    impl Harness {
        fn calls(&self) -> Vec<String> {
            std::fs::read_to_string(self.fake.join("calls.log"))
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect()
        }

        fn run_dir(&self) -> PathBuf {
            self.ctx.root.join("run-0")
        }

        /// The settings folder one kicad-cli user gets (`baseline`, `run-<index>`).
        fn own_config(&self, name: &str) -> String {
            self.ctx
                .root
                .join("kicad-config")
                .join(name)
                .display()
                .to_string()
        }

        /// The names in the run folder: the round folders left.
        fn rounds_left(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(self.run_dir())
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
                .collect();
            names.sort();
            names
        }
    }

    /// A judge context over the exported rounded board, with the fake kicad-cli, a
    /// private settings copy, a library folder beside the board, and `planes` as the
    /// nets the run's router got as planes (the fake report decides what is open).
    fn harness(planes: &[&str], default: String, canned: &[(usize, String)]) -> Harness {
        harness_as(planes, default, canned, false)
    }

    /// [`harness`]; with `over_ipc` the export is of the IPC text of the board and
    /// the saved file differs in bytes (see [`exported_as`]).
    fn harness_as(
        planes: &[&str],
        default: String,
        canned: &[(usize, String)],
        over_ipc: bool,
    ) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let Exported { dsn, ses } = exported_as(dir.path(), over_ipc);
        put(dir.path(), "lib.pretty/a.kicad_mod", "(footprint \"a\")");

        let fake = dir.path().join("fake");
        std::fs::create_dir_all(&fake).unwrap();
        let cli = crate::tools::cli::test_support::write_script(&fake, "kicad-cli", FAKE_CLI, "");
        std::fs::write(fake.join("drc.default.json"), default).unwrap();
        for (call, report) in canned {
            std::fs::write(fake.join(format!("drc.{call}.json")), report).unwrap();
        }

        let settings = dir.path().join("settings/10.0");
        put(&settings, "kicad_common.json", "{}");
        let config = copy_kicad_config(
            Some((settings, "10.0".to_string())),
            &dir.path().join("kicad-config"),
        )
        .unwrap()
        .home;

        let root = dir.path().join("search").join("judge");
        let mut ctx = prepare(&dsn, &root, &cli.display().to_string()).expect("preflight");
        std::fs::create_dir_all(&root).unwrap();
        ctx.kicad_config_home = Some(config.clone());
        let routed = Routed {
            index: 0,
            shape: crate::route_search::Shape::AsExported,
            seed: 0,
            dsn,
            held: crate::fastroute::Held {
                ses,
                unrouted: 0,
                kept: "full",
                passes: vec![],
                seconds: 2.5,
                untuned: None,
                tune: Default::default(),
                group_search: None,
            },
            planes: planes.iter().map(|net| net.to_string()).collect(),
            folder: dir.path().to_path_buf(),
        };
        Harness {
            dir,
            ctx,
            routed,
            fake,
            config,
        }
    }

    /// The registry's tools, except that `plan_route_refinement` answers from a
    /// script: `plan(n)` for the n-th call (from 0). Every planner call's arguments
    /// are kept.
    struct Scripted {
        real: RegistryTools,
        plan: Box<dyn Fn(usize) -> (bool, String) + Send + Sync>,
        seen: Mutex<Vec<Value>>,
    }

    impl Scripted {
        fn new(
            h: &Harness,
            plan: impl Fn(usize) -> (bool, String) + Send + Sync + 'static,
        ) -> Self {
            Scripted {
                real: RegistryTools(tool_context(&h.ctx.kicad_cli)),
                plan: Box::new(plan),
                seen: Mutex::new(Vec::new()),
            }
        }

        fn planner_calls(&self) -> Vec<Value> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl ToolCaller for Scripted {
        fn call<'a>(&'a self, name: &'a str, args: &'a Value) -> ToolCall<'a> {
            Box::pin(async move {
                if name == "plan_route_refinement" {
                    let number = {
                        let mut seen = self.seen.lock().unwrap();
                        seen.push(args.clone());
                        seen.len() - 1
                    };
                    return Ok((self.plan)(number));
                }
                self.real.call(name, args).await
            })
        }
    }

    /// A planner result that proposes one stitching via on `net`.
    fn one_via(net: &str) -> (bool, String) {
        (
            false,
            json!({
                "proposals": [{
                    "id": "p",
                    "kind": "stitching",
                    "operations": [{
                        "tool": "route_batch",
                        "arguments": {
                            "board": "b",
                            "tracks": [],
                            "vias": [{"net_name": net, "x": 5.0, "y": 5.0, "drill": 0.3, "pad_size": 0.6}]
                        }
                    }]
                }]
            })
            .to_string(),
        )
    }

    fn board_name() -> &'static str {
        "specctra_rounded_board.kicad_pcb"
    }

    #[tokio::test]
    async fn baseline_counts_the_report_and_runs_on_the_private_settings() {
        let h = harness(
            &[],
            drc_json(&["GND", "VCC"], &["clearance", "clearance", "silk_overlap"]),
            &[],
        );
        let base = baseline(&h.ctx).await.unwrap();
        assert_eq!(
            base.violations,
            rules(&[("clearance", 2), ("silk_overlap", 1)])
        );
        assert_eq!(base.unconnected, 2);

        let calls = h.calls();
        assert_eq!(calls.len(), 1, "{calls:?}");
        let own = h.own_config("baseline");
        assert!(
            calls[0].starts_with(&format!("KICAD_CONFIG_HOME={own} pcb drc --output ")),
            "the child sees the baseline's own copy: {}",
            calls[0]
        );
        // That copy is gone once the baseline is done; the call's copy is untouched.
        assert!(!h.ctx.root.join("kicad-config/baseline").exists());
        assert!(
            calls[0].contains(" --format json --refill-zones --save-board "),
            "{}",
            calls[0]
        );
        assert!(
            calls[0].ends_with(&format!("baseline/{}", board_name())),
            "{}",
            calls[0]
        );
        // The saved board, unchanged, in a scratch project.
        let written = h.ctx.root.join("baseline").join(board_name());
        assert_eq!(std::fs::read_to_string(&written).unwrap(), h.ctx.board_text);
        assert!(h.ctx.root.join("baseline/lib.pretty/a.kicad_mod").is_file());
        // The settings copy is the one made from the source.
        assert!(h.config.join("10.0/kicad_common.json").is_file());

        // A second call clears the first one's folder.
        assert_eq!(baseline(&h.ctx).await.unwrap(), base);
    }

    #[tokio::test]
    async fn baseline_refuses_a_report_with_no_unconnected_category() {
        let h = harness(&[], json!({"violations": []}).to_string(), &[]);
        let error = baseline(&h.ctx).await.unwrap_err();
        assert!(format!("{error:#}").contains("unconnected"), "{error:#}");
    }

    /// F1 (Row LT's verification): the preflight took the saved file of a board
    /// exported over IPC by its tree, then the run's import-plan call refused it by
    /// bytes, so every run of a live board was `judge_failed`. The run now takes the
    /// same identity. Controls: a board that changed since the export, and a
    /// manifest without the tree hash, still fail the run at that step.
    #[tokio::test]
    async fn a_run_is_judged_on_the_saved_file_of_a_board_exported_over_ipc() {
        let mut h = harness_as(&[], drc_json(&["GND"], &["clearance"]), &[], true);
        let recorded = h.ctx.manifest["source_sha256"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(
            recorded,
            format!("{:x}", Sha256::digest(h.ctx.board_text.as_bytes())),
            "the byte hash is of the IPC text, not the saved file"
        );
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| panic!("the planner must not be called"));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
        assert!(judged.board.is_file(), "the session was written and judged");

        // A board that changed since the export: the run fails at the import step.
        let saved = h.ctx.board_text.clone();
        h.ctx.board_text = saved.replacen("(at ", "(at 1", 1);
        assert_ne!(h.ctx.board_text, saved);
        let error = judge_with(&h.ctx, &h.routed, &base, &tools)
            .await
            .unwrap_err();
        let text = format!("{error:#}");
        assert!(
            text.contains("the importer refuses the run's session"),
            "{text}"
        );
        assert!(
            text.contains("saved board does not match routing manifest"),
            "{text}"
        );
        h.ctx.board_text = saved.clone();

        // A manifest from before the tree hash: bytes only, and the bytes differ.
        let mut body = h.ctx.manifest.clone();
        body.as_object_mut()
            .unwrap()
            .remove("source_canonical_sha256");
        h.ctx.manifest_text = body.to_string();
        let error = judge_with(&h.ctx, &h.routed, &base, &tools)
            .await
            .unwrap_err();
        let text = format!("{error:#}");
        assert!(
            text.contains("the importer refuses the run's session"),
            "{text}"
        );
        assert!(
            text.contains("saved board does not match routing manifest"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn no_planes_no_rounds_whatever_is_open() {
        let h = harness(&[], drc_json(&["GND"], &["clearance"]), &[]);
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| panic!("the planner must not be called"));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();

        assert!(judged.rounds.is_empty());
        assert!(tools.planner_calls().is_empty());
        assert_eq!(judged.board, h.run_dir().join("r0").join(board_name()));
        assert!(judged.board.is_file());
        assert_eq!(h.rounds_left(), vec!["r0"]);
        // Baseline and the run's one refill, each on its own copy of the private
        // settings (never the call's copy, and never the same folder).
        let calls = h.calls();
        assert_eq!(calls.len(), 2, "{calls:?}");
        for (call, name) in calls.iter().zip(["baseline", "run-0"]) {
            let own = h.own_config(name);
            assert!(
                call.starts_with(&format!("KICAD_CONFIG_HOME={own} pcb drc ")),
                "{calls:?}"
            );
        }
        assert!(
            !h.ctx.root.join("kicad-config/run-0").exists(),
            "removed after the judge"
        );
        // KiCad's count is the last report's; the open item is still counted.
        assert_eq!(judged.measures.unconnected, 1);
        assert_eq!(judged.measures.stitching_vias, 0);
        assert_eq!(judged.measures.route_s, 2.5);
        assert!(
            judged.measures.signal_mm + judged.measures.power_mm + judged.measures.pour_track_mm
                > 0.0,
            "the session's copper is measured"
        );
        // Same violations as the baseline: G2 is quiet.
        assert!(judged.gates.g2.is_empty(), "{:?}", judged.gates);
    }

    /// The fake kicad-cli's report of the length probe: one `length_out_of_range` entry
    /// for each `(net, mm)`, rule `n` for the n-th, its item naming the net.
    fn probe_json(entries: &[(&str, f64)]) -> String {
        json!({
            "violations": entries
                .iter()
                .enumerate()
                .map(|(index, (net, mm))| json!({
                    "type": "length_out_of_range",
                    "severity": "error",
                    "description": format!(
                        "Track length out of range (rule '__konnect_len_{:04}' min length 2000.0000 mm; actual {mm:.4} mm)",
                        index + 1
                    ),
                    "items": [{ "description": format!("Track [{net}] on F.Cu, length 3 mm") }]
                }))
                .collect::<Vec<_>>(),
            "unconnected_items": [],
        })
        .to_string()
    }

    fn dq_checks(nets: &[&str]) -> Arc<crate::length_probe::LengthChecks> {
        use crate::length_probe::{CheckedGroup, LengthChecks};
        Arc::new(LengthChecks {
            groups: vec![CheckedGroup {
                interface: "ram".into(),
                name: "dq".into(),
                nets: nets.iter().map(|net| net.to_string()).collect(),
                window: crate::tools::route_lengths::Window::Target {
                    target_mm: 40.0,
                    tolerance_mm: 0.5,
                },
            }],
            pairs: vec![],
            board_rules: false,
        })
    }

    #[tokio::test]
    async fn the_judge_probes_the_checked_nets_on_the_last_board_and_gates_on_them() {
        // Call 1 is the baseline, 2 the routed board's refill, 3 the length probe.
        let mut h = harness(
            &[],
            drc_json(&[], &[]),
            &[(3, probe_json(&[("GND", 40.2), ("VCC", 39.0)]))],
        );
        h.ctx.lengths = Some(dq_checks(&["GND", "VCC"]));
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| panic!("no planes, no planner"));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();

        let lengths = judged.measures.lengths.as_ref().expect("the probe ran");
        assert_eq!(lengths.groups[0].nets[0].kicad_mm, Some(40.2));
        assert_eq!(lengths.groups[0].nets[0].miss_mm, 0.0);
        assert!(
            (lengths.groups[0].nets[1].miss_mm - 0.5).abs() < 1e-9,
            "39.0 against 39.5"
        );
        assert!((lengths.miss_mm - 0.5).abs() < 1e-9);
        // G4 names the short net; the other gates are quiet.
        assert!(judged.gates.g2.is_empty(), "{:?}", judged.gates);
        assert_eq!(judged.gates.g4.len(), 1, "{:?}", judged.gates);
        assert!(
            judged.gates.g4[0].contains("VCC is 39.00 mm"),
            "{:?}",
            judged.gates
        );
        assert!(!judged.gates.all_pass());

        // The probe's copy, in the run's folder next to the judged board's folders.
        let folder = h.run_dir().join("lengths");
        assert_eq!(
            std::fs::read_to_string(folder.join(board_name())).unwrap(),
            std::fs::read_to_string(&judged.board).unwrap()
        );
        let rules =
            std::fs::read_to_string(folder.join("specctra_rounded_board.kicad_dru")).unwrap();
        assert_eq!(
            rules,
            crate::length_probe::probe_rules_text(&lengths_nets(&["GND", "VCC"]))
        );

        // Three kicad-cli runs; the probe is the third, on the run's own settings
        // copy, with no refill and no save.
        let calls = h.calls();
        assert_eq!(calls.len(), 3, "{calls:?}");
        let own = h.own_config("run-0");
        assert_eq!(
            calls[2],
            format!(
                "KICAD_CONFIG_HOME={own} pcb drc --output {} --format json --units mm {}",
                folder.join("specctra_rounded_board.length.json").display(),
                folder.join(board_name()).display()
            )
        );
        // The own copy is gone once the judge is done.
        assert!(!h.ctx.root.join("kicad-config/run-0").exists());
    }

    fn lengths_nets(nets: &[&str]) -> Vec<String> {
        nets.iter().map(|net| net.to_string()).collect()
    }

    #[tokio::test]
    async fn a_judge_with_nothing_to_check_runs_no_probe() {
        for checks in [
            None,
            Some(Arc::new(crate::length_probe::LengthChecks::default())),
        ] {
            let mut h = harness(&[], drc_json(&[], &[]), &[]);
            h.ctx.lengths = checks;
            let base = baseline(&h.ctx).await.unwrap();
            let tools = Scripted::new(&h, |_| panic!("no planes, no planner"));
            let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
            assert_eq!(judged.measures.lengths, None);
            assert!(judged.gates.g4.is_empty());
            assert_eq!(h.calls().len(), 2, "the baseline and the routed board only");
            assert!(!h.run_dir().join("lengths").exists());
        }
    }

    /// The scratch copy's project a judged run used.
    fn judged_project(h: &Harness) -> Value {
        let text = std::fs::read_to_string(h.run_dir().join("r0/specctra_rounded_board.kicad_pro"))
            .unwrap();
        serde_json::from_str(&text).unwrap()
    }

    #[tokio::test]
    async fn a_board_with_length_rules_is_judged_on_a_project_that_reports_them() {
        const OWN: &str = r#"{"board": {"design_settings": {"rule_severities": {"length_out_of_range": "ignore", "skew_out_of_range": "ignore", "clearance": "ignore"}}}}"#;
        for (board_rules, wanted) in [(true, "error"), (false, "ignore")] {
            let mut h = harness(&[], drc_json(&[], &[]), &[]);
            let source = h.ctx.board.with_extension("kicad_pro");
            std::fs::write(&source, OWN).unwrap();
            // The export of a board with a project file records its hash.
            let mut manifest: Value = serde_json::from_str(&h.ctx.manifest_text).unwrap();
            manifest["routing_context"] = json!({
                "project_sha256": format!("{:x}", Sha256::digest(OWN.as_bytes())),
                "custom_rules_sha256": null,
                "effective_rules_sha256": "0",
            });
            h.ctx.manifest_text = manifest.to_string();
            // No group or pair: the board's rules alone make the checks.
            h.ctx.lengths = Some(Arc::new(crate::length_probe::LengthChecks {
                board_rules,
                ..Default::default()
            }));
            let base = baseline(&h.ctx).await.unwrap();
            let tools = Scripted::new(&h, |_| panic!("no planes, no planner"));
            let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
            let severities = &judged_project(&h)["board"]["design_settings"]["rule_severities"];
            assert_eq!(severities["length_out_of_range"], wanted, "{board_rules}");
            assert_eq!(severities["skew_out_of_range"], wanted, "{board_rules}");
            // Another check keeps the project's own setting; G2 and the probe are not asked.
            assert_eq!(severities["clearance"], "ignore");
            assert_eq!(h.calls().len(), 2, "the baseline and the routed board only");
            assert!(judged.measures.lengths.is_none() && judged.gates.g4.is_empty());
            // The owner's project is never written.
            assert_eq!(std::fs::read_to_string(&source).unwrap(), OWN);
        }
    }

    /// Writes `own` as the board's project, with the hash the export records.
    fn with_project(h: &mut Harness, own: &str) -> PathBuf {
        let source = h.ctx.board.with_extension("kicad_pro");
        std::fs::write(&source, own).unwrap();
        let mut manifest: Value = serde_json::from_str(&h.ctx.manifest_text).unwrap();
        manifest["routing_context"] = json!({
            "project_sha256": format!("{:x}", Sha256::digest(own.as_bytes())),
            "custom_rules_sha256": null,
            "effective_rules_sha256": "0",
        });
        h.ctx.manifest_text = manifest.to_string();
        source
    }

    fn project_with(track_dangling: &str, unconnected_items: &str) -> String {
        json!({"board": {"design_settings": {"rule_severities": {
            "track_dangling": track_dangling,
            "unconnected_items": unconnected_items,
            "clearance": "ignore"
        }}}})
        .to_string()
    }

    fn severities_of(path: &Path) -> Value {
        let text = std::fs::read_to_string(path).unwrap();
        serde_json::from_str::<Value>(&text).unwrap()["board"]["design_settings"]["rule_severities"]
            .clone()
    }

    #[tokio::test]
    async fn a_call_with_checks_sees_dangling_and_unconnected_items_whatever_the_project_says() {
        // (the project's severities, what the judge's copies get)
        for (own, wanted) in [
            (("ignore", "ignore"), ("warning", "error")),
            (("warning", "warning"), ("warning", "error")),
            // At that level or higher: left as it is.
            (("error", "error"), ("error", "error")),
        ] {
            let mut h = harness(
                &[],
                dangling_json("GND"),
                &[(3, probe_json(&[("GND", 40.0), ("VCC", 40.0)]))],
            );
            let source = with_project(&mut h, &project_with(own.0, own.1));
            h.ctx.lengths = Some(dq_checks(&["GND", "VCC"]));
            let base = baseline(&h.ctx).await.unwrap();
            let tools = Scripted::new(&h, |_| panic!("no planes, no planner"));
            let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
            let baseline_copy = severities_of(
                &h.ctx
                    .root
                    .join("baseline")
                    .join("specctra_rounded_board.kicad_pro"),
            );
            let judged_copy = &judged_project(&h)["board"]["design_settings"]["rule_severities"];
            for copy in [&baseline_copy, judged_copy] {
                assert_eq!(copy["track_dangling"], wanted.0, "{own:?}");
                assert_eq!(copy["unconnected_items"], wanted.1, "{own:?}");
                // Another check keeps the project's own setting.
                assert_eq!(copy["clearance"], "ignore");
            }
            // The dangling checked net fails G4(d).
            assert!(
                judged.gates.g4[0].starts_with("GND has 1 dangling or unconnected item(s)"),
                "{:?}",
                judged.gates.g4
            );
            // The owner's project is never written.
            assert_eq!(severities_of(&source)["track_dangling"], own.0, "{own:?}");
        }
    }

    #[tokio::test]
    async fn a_call_without_checks_keeps_the_projects_severities() {
        let mut h = harness(&[], drc_json(&[], &[]), &[]);
        with_project(&mut h, &project_with("ignore", "ignore"));
        for lengths in [
            None,
            Some(Arc::new(crate::length_probe::LengthChecks {
                board_rules: true,
                ..Default::default()
            })),
        ] {
            h.ctx.lengths = lengths;
            let base = baseline(&h.ctx).await.unwrap();
            let tools = Scripted::new(&h, |_| panic!("no planes, no planner"));
            judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
            let baseline_copy = severities_of(
                &h.ctx
                    .root
                    .join("baseline")
                    .join("specctra_rounded_board.kicad_pro"),
            );
            let judged_copy = &judged_project(&h)["board"]["design_settings"]["rule_severities"];
            for copy in [&baseline_copy, judged_copy] {
                assert_eq!(copy["track_dangling"], "ignore");
                assert_eq!(copy["unconnected_items"], "ignore");
            }
        }
    }

    #[tokio::test]
    async fn a_candidate_that_overshoots_a_window_loses_to_the_untuned_board() {
        // VCC is 0.5 mm short untuned (miss 0.5, window 39.5 to 40.5) and 1.0 mm over after
        // tuning (miss 1.0), with no DRC change: not harm, and not a gain.
        let (h, judged) = judged_with_candidates(
            &[TuneVariant::Free],
            &[(refill_ok(), vcc(39.0)), (refill_ok(), vcc(41.5))],
        )
        .await;
        let choice = judged.tune.as_ref().unwrap();
        assert_eq!(choice.chosen, None);
        assert_eq!(choice.rows[0]["rejected"], json!([]));
        assert_eq!(choice.rows[0]["judged"], json!(true));
        assert_eq!(
            choice.rows[0]["reason"],
            json!("ranked no better than the untuned board")
        );
        assert_eq!(folder_of(&h, &judged), "run-0-untuned");
        assert!(
            judged.gates.g4[0].contains("VCC is 39.00 mm"),
            "{:?}",
            judged.gates.g4
        );
        assert_eq!(
            choice.untuned_reason(),
            "free: ranked no better than the untuned board"
        );
        // Control: a candidate that shrinks the miss wins.
        let (h, judged) = judged_with_candidates(
            &[TuneVariant::Free],
            &[(refill_ok(), vcc(39.0)), (refill_ok(), vcc(39.4))],
        )
        .await;
        let choice = judged.tune.as_ref().unwrap();
        assert_eq!(choice.chosen, Some(TuneVariant::Free));
        assert_eq!(folder_of(&h, &judged), "run-0");
    }

    #[tokio::test]
    async fn an_exact_tie_with_the_untuned_board_goes_to_the_untuned_board() {
        // The candidate measures exactly what the untuned board does.
        let (h, judged) = judged_with_candidates(
            &[TuneVariant::Free],
            &[(refill_ok(), vcc(39.0)), (refill_ok(), vcc(39.0))],
        )
        .await;
        assert_eq!(judged.tune.as_ref().unwrap().chosen, None);
        assert_eq!(folder_of(&h, &judged), "run-0-untuned");
        // Control: one hundredth of a millimetre nearer is a gain.
        let (_h, judged) = judged_with_candidates(
            &[TuneVariant::Free],
            &[(refill_ok(), vcc(39.0)), (refill_ok(), vcc(39.01))],
        )
        .await;
        assert_eq!(
            judged.tune.as_ref().unwrap().chosen,
            Some(TuneVariant::Free)
        );
    }

    #[tokio::test]
    async fn a_probe_that_cannot_be_trusted_fails_the_judge() {
        // The same net reported twice.
        let mut h = harness(
            &[],
            drc_json(&[], &[]),
            &[(3, probe_json(&[("GND", 40.0), ("GND", 40.0)]))],
        );
        h.ctx.lengths = Some(dq_checks(&["GND"]));
        // One rule was written; the second entry names rule 2, which was not.
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| panic!("no planner"));
        let error = judge_with(&h.ctx, &h.routed, &base, &tools)
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("length probe"), "{error:#}");
    }

    /// The call's own KiCad rules are G4's: a run with fewer `length_out_of_range`
    /// violations than the unrouted board has used to pass G2 and be called complete.
    #[tokio::test]
    async fn kicads_own_length_violations_fail_g4_and_not_g2_in_the_judge() {
        let mut h = harness(
            &[],
            drc_json(
                &[],
                &[
                    "length_out_of_range",
                    "length_out_of_range",
                    "length_out_of_range",
                ],
            ),
            &[(
                2,
                drc_json(&[], &["length_out_of_range", "length_out_of_range"]),
            )],
        );
        h.ctx.lengths = None;
        let base = baseline(&h.ctx).await.unwrap();
        assert_eq!(base.violations, rules(&[("length_out_of_range", 3)]));
        let tools = Scripted::new(&h, |_| panic!("no planner"));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
        assert!(judged.gates.g2.is_empty(), "{:?}", judged.gates);
        assert_eq!(
            judged.gates.g4,
            vec!["KiCad: 2 length_out_of_range violation(s) of the board's own rules"]
        );
        // And no probe ran: nothing from the intent is checked.
        assert_eq!(h.calls().len(), 2);
    }

    /// Row ZR, ZR0: the pours' open count is the first refill's, before the stitching
    /// round, and the rest is read from the last board and its report. Control: a call
    /// with no pour nets records no `pours` at all.
    #[tokio::test]
    async fn the_judge_records_the_pours_before_and_after_stitching() {
        // Call 1 is the baseline, 2 the routed board (GND open, a starved thermal on the
        // stitched one), 3 the stitched one.
        let h = harness(
            &["GND"],
            drc_json(&["GND"], &[]),
            &[(3, drc_json(&[], &["starved_thermal"]))],
        );
        assert!(h.ctx.pour_nets.contains("GND"), "{:?}", h.ctx.pour_nets);
        // Only the stitched board (call 3) is saved with a fill: one piece on one of the
        // GND zone layers, so the pieces can only come from the last board.
        let layers = crate::zone_fill::pour_fills(&h.ctx.board_text, &h.ctx.pour_nets)
            .unwrap()
            .len();
        assert!(layers >= 1, "the fixture has a GND pour");
        let filled = h.ctx.board_text.replacen(
            "(name \"GND_BOTTOM\")",
            "(name \"GND_BOTTOM\") (filled_polygon (layer \"B.Cu\") (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10)))",
            1,
        );
        assert_ne!(filled, h.ctx.board_text, "the fixture names GND_BOTTOM");
        std::fs::write(h.fake.join("board.3"), &filled).unwrap();
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| one_via("GND"));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
        let pours = judged.measures.pours.as_ref().expect("the call has pours");
        assert_eq!(judged.measures.pours_error, None);
        assert_eq!(pours.open_before_stitching, 1, "the routed board's report");
        assert_eq!(pours.open_after_stitching, 0, "the stitched board's report");
        assert_eq!(pours.starved_thermals, 1);
        assert_eq!(pours.pieces, 1, "the stitched board's fill");
        assert_eq!(pours.unfilled, layers - 1);
        assert_eq!(pours.largest_piece_share, Some(1.0));
        assert_eq!((pours.false_connected, pours.false_open), (None, None));
        // Recorded only: the gates and the key are those of the same measures without it.
        let mut without = judged.measures.clone();
        without.pours = None;
        assert_eq!(gates(&without, &base), judged.gates);
        assert_eq!(
            rank_measures(&[(&without, &judged.gates, true)]),
            rank_measures(&[(&judged.measures, &judged.gates, true)])
        );

        // Control: no pour nets, no pour measures.
        let mut h = harness(
            &["GND"],
            drc_json(&["GND"], &[]),
            &[(3, drc_json(&[], &[]))],
        );
        h.ctx.pour_nets.clear();
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| one_via("GND"));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
        assert_eq!(judged.measures.pours, None);
        assert!(serde_json::to_value(&judged.measures)
            .unwrap()
            .get("pours")
            .is_none());
    }

    #[tokio::test]
    async fn an_open_plane_net_runs_a_round_and_records_its_row() {
        // Call 1 is the baseline, 2 the routed board (GND open), 3 the stitched one.
        let h = harness(
            &["GND"],
            drc_json(&["GND"], &[]),
            &[(3, drc_json(&[], &[]))],
        );
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| one_via("GND"));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();

        assert_eq!(
            judged.rounds,
            vec![json!({
                "round": 1, "open_before": 1, "proposals": 1, "written_vias": 1, "open_after": 0
            })]
        );
        assert_eq!(judged.measures.stitching_vias, 1);
        assert_eq!(
            judged.measures.unconnected, 0,
            "the last report is round 1's"
        );

        // The planner was asked as documented, of the routed board.
        let seen = tools.planner_calls();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0]["board"],
            json!(h
                .run_dir()
                .join("r0")
                .join(board_name())
                .display()
                .to_string())
        );
        assert_eq!(seen[0]["kinds"], json!(["stitching"]));
        assert_eq!(seen[0]["nets"], json!(["GND"]));
        assert_eq!(seen[0]["search_radius_mm"], json!(STITCH_RADIUS_MM));
        assert_eq!(seen[0]["effort"], json!("standard"));

        // The earlier round's folder is gone whole; the last is kept, with its copied
        // project, and is the judged board.
        assert_eq!(h.rounds_left(), vec!["r1"]);
        assert_eq!(judged.board, h.run_dir().join("r1").join(board_name()));
        assert!(h.run_dir().join("r1/lib.pretty/a.kicad_mod").is_file());
        let text = std::fs::read_to_string(&judged.board).unwrap();
        assert!(
            text.contains("(at 5 5)") && text.contains("(net \"GND\")"),
            "the via is written"
        );

        // Baseline, routed board, round 1: three runs of kicad-cli, all private.
        let calls = h.calls();
        assert_eq!(calls.len(), 3, "{calls:?}");
        assert!(calls[0].starts_with(&format!("KICAD_CONFIG_HOME={} ", h.own_config("baseline"))));
        let own = h.own_config("run-0");
        assert!(calls[1..]
            .iter()
            .all(|call| call.starts_with(&format!("KICAD_CONFIG_HOME={own} "))));
        assert!(
            calls[2].ends_with(&format!("run-0/r1/{}", board_name())),
            "{}",
            calls[2]
        );
    }

    #[tokio::test]
    async fn the_rounds_stop_at_stitch_rounds_when_the_item_never_closes() {
        // The default report keeps GND open for ever; the planner always proposes a via.
        let h = harness(&["GND"], drc_json(&["GND"], &[]), &[]);
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| one_via("GND"));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();

        assert_eq!(judged.rounds.len(), STITCH_ROUNDS);
        for (index, row) in judged.rounds.iter().enumerate() {
            assert_eq!(row["round"], json!(index + 1));
            assert_eq!(row["open_before"], json!(1));
            assert_eq!(row["written_vias"], json!(1));
            assert_eq!(row["open_after"], json!(1));
        }
        assert_eq!(tools.planner_calls().len(), STITCH_ROUNDS);
        assert_eq!(judged.measures.stitching_vias, STITCH_ROUNDS);
        // Baseline, the routed board and each round: nothing after the last round.
        assert_eq!(h.calls().len(), 2 + STITCH_ROUNDS);
        // Only the last round's folder is left.
        let last = format!("r{STITCH_ROUNDS}");
        assert_eq!(h.rounds_left(), vec![last.clone()]);
        assert_eq!(judged.board, h.run_dir().join(&last).join(board_name()));
        assert_eq!(judged.measures.unconnected, 1, "still open: G1 fails");
        assert!(!judged.gates.g1.is_empty());
    }

    #[tokio::test]
    async fn a_planner_refusal_stops_the_rounds_and_keeps_the_routed_board() {
        let h = harness(&["GND"], drc_json(&["GND"], &[]), &[]);
        let base = baseline(&h.ctx).await.unwrap();
        let refusal = json!({"status": "blocked", "proposals": [], "blocked": [{"reason": "cannot plan here"}]});
        let tools = Scripted::new(&h, move |_| (true, refusal.to_string()));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();

        assert_eq!(
            judged.rounds,
            vec![json!({"round": 1, "open_before": 1, "planner_refused": "cannot plan here"})]
        );
        assert_eq!(h.rounds_left(), vec!["r0"]);
        assert_eq!(judged.board, h.run_dir().join("r0").join(board_name()));
        assert_eq!(h.calls().len(), 2, "no refill after a refusal");
    }

    #[tokio::test]
    async fn no_proposal_stops_the_rounds_and_is_recorded() {
        let h = harness(&["GND"], drc_json(&["GND"], &[]), &[]);
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| (false, json!({"proposals": []}).to_string()));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
        assert_eq!(
            judged.rounds,
            vec![json!({"round": 1, "open_before": 1, "proposals": 0})]
        );
        assert_eq!(h.rounds_left(), vec!["r0"]);
        assert_eq!(h.calls().len(), 2);
    }

    #[tokio::test]
    async fn a_proposal_that_modifies_copper_stops_the_rounds_without_a_write() {
        let h = harness(&["GND"], drc_json(&["GND"], &[]), &[]);
        let base = baseline(&h.ctx).await.unwrap();
        let modify = json!({"proposals": [{"id": "p", "operations": [
            {"tool": "modify_trace", "arguments": {"board": "b", "uuid": "u", "x1": 0, "y1": 0, "x2": 1, "y2": 1}}
        ]}]});
        let tools = Scripted::new(&h, move |_| (false, modify.to_string()));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
        assert_eq!(judged.rounds.len(), 1);
        let reason = judged.rounds[0]["planner_refused"].as_str().unwrap();
        assert!(reason.contains("modify_trace"), "{reason}");
        assert_eq!(h.rounds_left(), vec!["r0"]);
        assert_eq!(h.calls().len(), 2);
    }

    #[tokio::test]
    async fn a_net_that_is_not_a_plane_is_not_stitched_even_when_open() {
        // GND is open but only VCC was given as a plane.
        let h = harness(&["VCC"], drc_json(&["GND"], &[]), &[]);
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| panic!("the planner must not be called"));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
        assert!(judged.rounds.is_empty());
    }

    /// The registry's own planner and audit, in-process, on the routed scratch board.
    /// What the planner proposes for this small board is not fixed here: the row it
    /// leaves must be one of the documented shapes, and whatever the outcome only the
    /// last round's folder is left.
    #[tokio::test]
    async fn the_real_planner_and_audit_run_in_the_loop() {
        let h = harness(&["GND"], drc_json(&["GND"], &[]), &[]);
        let base = baseline(&h.ctx).await.unwrap();
        let tools = tool_context(&h.ctx.kicad_cli);
        let judged = judge(&h.ctx, &h.routed, &base, tools).await.unwrap();
        eprintln!("rounds: {:#?}", judged.rounds);

        assert!(
            !judged.rounds.is_empty(),
            "GND is open on a plane net: a round is tried"
        );
        assert!(judged.rounds.len() <= STITCH_ROUNDS);
        let first = &judged.rounds[0];
        assert_eq!(first["round"], json!(1));
        assert_eq!(first["open_before"], json!(1));
        assert!(
            first.get("planner_refused").is_some() || first.get("proposals").is_some(),
            "{first}"
        );
        assert_eq!(
            h.rounds_left().len(),
            1,
            "only the last round's folder is left"
        );
        assert_eq!(
            judged.board.parent().unwrap().parent().unwrap(),
            h.run_dir()
        );
        assert!(judged.board.is_file());
        assert!(
            judged.measures.routed_share.is_some(),
            "the audit ran for real"
        );
        let _keep_dir_alive = &h.dir;
    }
    // ------------------------------------------------ Row LT, L22: G4(d) in the judge

    /// A KiCad report whose one `track_dangling` entry names `net`.
    fn dangling_json(net: &str) -> String {
        json!({
            "violations": [{
                "type": "track_dangling",
                "severity": "warning",
                "description": "Track has unconnected end",
                "items": [{ "description": format!("Track [{net}] on F.Cu, length 3 mm") }]
            }],
            "unconnected_items": [],
        })
        .to_string()
    }

    #[tokio::test]
    async fn a_checked_net_named_in_a_dangling_entry_fails_g4_with_its_windows_lower_bound() {
        // Calls: 1 the baseline, 2 the routed board's refill, 3 the probe.
        let mut h = harness(
            &[],
            drc_json(&[], &[]),
            &[
                (2, dangling_json("GND")),
                (3, probe_json(&[("GND", 40.0), ("VCC", 40.0)])),
            ],
        );
        h.ctx.lengths = Some(dq_checks(&["GND", "VCC"]));
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| panic!("no planes, no planner"));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
        let lengths = judged.measures.lengths.as_ref().unwrap();
        assert_eq!(
            lengths.paths["GND"],
            crate::length_probe::PathFault {
                contacts: 0,
                dangling: 1
            }
        );
        assert_eq!(lengths.paths.len(), 1);
        // Both nets are inside the window by KiCad's length; GND's miss is the lower
        // bound, and G4 names it.
        assert!((lengths.groups[0].nets[0].miss_mm - 39.5).abs() < 1e-9);
        assert_eq!(lengths.groups[0].nets[1].miss_mm, 0.0);
        assert_eq!(judged.gates.g4.len(), 1, "{:?}", judged.gates.g4);
        assert!(
            judged.gates.g4[0].starts_with("GND has 1 dangling or unconnected item(s)"),
            "{:?}",
            judged.gates.g4
        );
    }

    #[tokio::test]
    async fn a_session_whose_checked_net_crosses_itself_fails_g4_in_the_judge() {
        let mut h = harness(
            &[],
            drc_json(&[], &[]),
            &[(3, probe_json(&[("GND", 40.0), ("VCC", 40.0)]))],
        );
        // VCC's recorded wire runs along y = -506062 (units of 0.1 um); a wire straight
        // down across it makes the net cross itself.
        let ses = std::fs::read_to_string(&h.routed.held.ses).unwrap();
        let crossing = ses.replacen(
            "(net VCC\n",
            "(net VCC\n        (wire\n          (path F.Cu 2000\n            1050000 -510000\n            1050000 -503000\n          )\n        )\n",
            1,
        );
        assert_ne!(crossing, ses, "the recorded session has a net VCC");
        let crossed = h.dir.path().join("crossed.ses");
        std::fs::write(&crossed, crossing).unwrap();
        h.routed.held.ses = crossed;
        h.ctx.lengths = Some(dq_checks(&["GND", "VCC"]));
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| panic!("no planes, no planner"));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
        let lengths = judged.measures.lengths.as_ref().unwrap();
        assert_eq!(lengths.paths.keys().collect::<Vec<_>>(), ["VCC"]);
        assert_eq!(lengths.paths["VCC"].contacts, 1);
        assert!(judged.gates.g4[0].starts_with("VCC's copper crosses or overlaps itself 1 time(s)"));
        // Control: the recorded session itself is clean.
        let mut h = harness(
            &[],
            drc_json(&[], &[]),
            &[(3, probe_json(&[("GND", 40.0), ("VCC", 40.0)]))],
        );
        h.ctx.lengths = Some(dq_checks(&["GND", "VCC"]));
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| panic!("no planes, no planner"));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
        assert!(judged.measures.lengths.unwrap().paths.is_empty());
        assert!(judged.gates.g4.is_empty(), "{:?}", judged.gates.g4);
    }

    // ----------------------------------------- Row LT, L23 and L24: judge_candidates

    use crate::fastroute::{TuneCandidate, TuneVariant};

    /// Gives the harness's run its untuned session and the `variants` as candidates,
    /// each a copy of the session in `<name>.ses` (a candidate's `ses` may be pointed
    /// elsewhere by the test), and their kept rows.
    fn with_candidates(h: &mut Harness, variants: &[TuneVariant]) {
        let untuned = h.routed.held.ses.clone();
        h.routed.held.untuned = Some(untuned.clone());
        for variant in variants {
            let ses = h.dir.path().join(format!("{}.ses", variant.folder_name(1)));
            std::fs::copy(&untuned, &ses).unwrap();
            h.routed.held.tune.candidates.push(TuneCandidate {
                variant: *variant,
                ses,
                unrouted: 0,
                dropped: vec![(
                    "GND".to_string(),
                    "its copper touches itself: 2 crossings".to_string(),
                )],
                passes: 2,
            });
            h.routed.held.tune.rows.push(json!({
                "name": "tune", "outcome": "candidate", "unrouted": 0, "seconds": 1.0,
                "variant": variant, "round": 1
            }));
        }
        h.routed
            .held
            .passes
            .push(h.routed.held.tune.rows[0].clone());
    }

    /// The canned reports of judges in order: each judge is two kicad-cli runs (its
    /// refill, then its probe), the first after the baseline's call 1.
    fn canned_judges(judges: &[(String, String)]) -> Vec<(usize, String)> {
        judges
            .iter()
            .enumerate()
            .flat_map(|(at, (refill, probe))| {
                [(2 + 2 * at, refill.clone()), (3 + 2 * at, probe.clone())]
            })
            .collect()
    }

    fn refill_ok() -> String {
        drc_json(&[], &[])
    }

    /// A probe report with VCC at `vcc` mm (the window is 39.5 to 40.5 mm).
    fn vcc(mm: f64) -> String {
        probe_json(&[("GND", 40.0), ("VCC", mm)])
    }

    async fn judged_with_candidates(
        variants: &[TuneVariant],
        judges: &[(String, String)],
    ) -> (Harness, Judged) {
        let mut h = harness(&[], drc_json(&[], &[]), &canned_judges(judges));
        h.ctx.lengths = Some(dq_checks(&["GND", "VCC"]));
        with_candidates(&mut h, variants);
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| panic!("no planes, no planner"));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
        (h, judged)
    }

    fn folder_of(h: &Harness, judged: &Judged) -> String {
        judged
            .board
            .strip_prefix(&h.ctx.root)
            .unwrap()
            .components()
            .next()
            .unwrap()
            .as_os_str()
            .to_string_lossy()
            .into_owned()
    }

    #[tokio::test]
    async fn a_harmful_free_candidate_loses_to_a_harmless_pour_safe_one() {
        let (h, judged) = judged_with_candidates(
            &[TuneVariant::Free, TuneVariant::PourSafe],
            &[
                (refill_ok(), vcc(39.0)),
                // Free adds a clearance violation the untuned board does not have.
                (drc_json(&[], &["clearance"]), vcc(40.0)),
                (refill_ok(), vcc(39.4)),
            ],
        )
        .await;
        let choice = judged
            .tune
            .as_ref()
            .expect("a run with candidates records its choice");
        assert_eq!(choice.chosen, Some(TuneVariant::PourSafe));
        assert_eq!(
            choice.rows[0]["rejected"],
            json!(["clearance violations 0 \u{2192} 1"])
        );
        assert_eq!(choice.rows[0]["judged"], json!(true));
        assert_eq!(choice.rows[0]["chosen"], json!(false));
        assert_eq!(choice.rows[1]["rejected"], json!([]));
        assert_eq!(choice.rows[1]["chosen"], json!(true));
        assert_eq!(choice.rows[1]["variant"], json!("pour_safe"));
        assert_eq!(choice.rows[1]["passes"], json!(2));
        assert_eq!(
            choice.rows[1]["dropped"],
            json!([{ "net": "GND", "reason": "its copper touches itself: 2 crossings" }])
        );
        // The judged board is the pour-safe candidate's, in its own folder; the three
        // judges each ran their refill and their probe after the baseline.
        assert_eq!(folder_of(&h, &judged), "run-0-pour-safe");
        assert_eq!(h.calls().len(), 7);
        assert!(h.ctx.root.join("run-0-untuned").is_dir());
        assert!(h.ctx.root.join("run-0").is_dir());
        // Its measures are the candidate's: VCC at 39.4 is 0.1 mm short.
        assert!((judged.measures.lengths.as_ref().unwrap().miss_mm - 0.1).abs() < 1e-9);
        assert_eq!(choice.untuned.violations.get("clearance"), None);
    }

    #[tokio::test]
    async fn when_both_candidates_are_harmless_the_better_rank_wins() {
        // Free fails G4 by 0.1 mm; the pour-safe candidate passes it.
        let (h, judged) = judged_with_candidates(
            &[TuneVariant::Free, TuneVariant::PourSafe],
            &[
                (refill_ok(), vcc(39.0)),
                (refill_ok(), vcc(39.4)),
                (refill_ok(), vcc(40.0)),
            ],
        )
        .await;
        let choice = judged.tune.as_ref().unwrap();
        assert_eq!(choice.chosen, Some(TuneVariant::PourSafe));
        assert_eq!(choice.rows[0]["g4_failed"], json!(true));
        assert_eq!(choice.rows[1]["g4_failed"], json!(false));
        assert!(judged.gates.g4.is_empty(), "{:?}", judged.gates.g4);
        assert_eq!(folder_of(&h, &judged), "run-0-pour-safe");
        // A tie goes to the first candidate, Free.
        let (h, judged) = judged_with_candidates(
            &[TuneVariant::Free, TuneVariant::PourSafe],
            &[
                (refill_ok(), vcc(39.0)),
                (refill_ok(), vcc(39.4)),
                (refill_ok(), vcc(39.4)),
            ],
        )
        .await;
        assert_eq!(
            judged.tune.as_ref().unwrap().chosen,
            Some(TuneVariant::Free)
        );
        assert_eq!(folder_of(&h, &judged), "run-0");
    }

    #[tokio::test]
    async fn a_harmless_free_candidate_that_passes_g4_leaves_the_pour_safe_one_unjudged() {
        let (h, judged) = judged_with_candidates(
            &[TuneVariant::Free, TuneVariant::PourSafe],
            &[
                (refill_ok(), vcc(39.0)),
                (refill_ok(), vcc(40.0)),
                (refill_ok(), vcc(40.0)),
            ],
        )
        .await;
        let choice = judged.tune.as_ref().unwrap();
        assert_eq!(choice.chosen, Some(TuneVariant::Free));
        assert_eq!(
            h.calls().len(),
            5,
            "the baseline and two judges: pour-safe is not judged"
        );
        assert!(!h.ctx.root.join("run-0-pour-safe").exists());
        assert_eq!(choice.rows[1]["variant"], json!("pour_safe"));
        assert_eq!(choice.rows[1]["judged"], json!(false));
        assert_eq!(choice.rows[1]["chosen"], json!(false));
        assert!(choice.rows[1]["reason"]
            .as_str()
            .unwrap()
            .contains("free candidate"));
        assert_eq!(folder_of(&h, &judged), "run-0");
        assert!(judged.gates.g4.is_empty());
    }

    #[tokio::test]
    async fn a_pour_safe_candidate_alone_is_judged_in_its_own_folder() {
        // The free pass was discarded: only the pour-safe candidate exists.
        let (h, judged) = judged_with_candidates(
            &[TuneVariant::PourSafe],
            &[(refill_ok(), vcc(39.0)), (refill_ok(), vcc(40.0))],
        )
        .await;
        assert_eq!(
            judged.tune.as_ref().unwrap().chosen,
            Some(TuneVariant::PourSafe)
        );
        assert_eq!(folder_of(&h, &judged), "run-0-pour-safe");
        assert!(!h.ctx.root.join("run-0").exists());
    }

    #[tokio::test]
    async fn when_every_candidate_is_harmful_the_untuned_board_stands_with_its_own_g4() {
        let (h, judged) = judged_with_candidates(
            &[TuneVariant::Free, TuneVariant::PourSafe],
            &[
                (refill_ok(), vcc(39.0)),
                // Free opens an item; pour-safe adds a violation. Both pass G4.
                (drc_json(&["GND"], &[]), vcc(40.0)),
                (drc_json(&[], &["clearance", "clearance"]), vcc(40.0)),
            ],
        )
        .await;
        let choice = judged.tune.as_ref().unwrap();
        assert_eq!(choice.chosen, None);
        assert_eq!(
            choice.rows[0]["rejected"],
            json!(["unconnected items 0 \u{2192} 1"])
        );
        assert_eq!(
            choice.rows[1]["rejected"],
            json!(["clearance violations 0 \u{2192} 2"])
        );
        assert!(choice.rows.iter().all(|row| row["chosen"] == json!(false)));
        // The board, the measures and G4 are the untuned board's: VCC at 39.0 is short.
        assert_eq!(folder_of(&h, &judged), "run-0-untuned");
        assert_eq!(judged.measures, choice.untuned);
        assert_eq!(judged.gates.g4.len(), 1, "{:?}", judged.gates.g4);
        assert!(
            judged.gates.g4[0].contains("VCC is 39.00 mm"),
            "{:?}",
            judged.gates.g4
        );
    }

    #[tokio::test]
    async fn a_candidate_the_judge_cannot_read_is_rejected_and_the_untuned_board_stands() {
        let mut h = harness(
            &[],
            drc_json(&[], &[]),
            &canned_judges(&[(refill_ok(), vcc(39.0))]),
        );
        h.ctx.lengths = Some(dq_checks(&["GND", "VCC"]));
        with_candidates(&mut h, &[TuneVariant::Free]);
        h.routed.held.tune.candidates[0].ses = h.dir.path().join("absent.ses");
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| panic!("no planes, no planner"));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
        let choice = judged.tune.as_ref().unwrap();
        assert_eq!(choice.chosen, None);
        assert_eq!(choice.rows[0]["judged"], json!(false));
        let line = choice.rows[0]["rejected"][0].as_str().unwrap();
        assert!(line.starts_with("the judge failed on it:"), "{line}");
        assert_eq!(folder_of(&h, &judged), "run-0-untuned");
        // Control: a failure of the untuned session is the judge's failure.
        let mut h = harness(&[], drc_json(&[], &[]), &[]);
        with_candidates(&mut h, &[TuneVariant::Free]);
        h.routed.held.untuned = Some(h.dir.path().join("absent.ses"));
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| panic!("no planes, no planner"));
        let error = judge_with(&h.ctx, &h.routed, &base, &tools)
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("untuned session"),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn a_run_without_candidates_is_judged_once_in_its_own_folder_with_no_choice() {
        let mut h = harness(&[], drc_json(&[], &[]), &[]);
        h.routed.held.untuned = Some(h.routed.held.ses.clone());
        let base = baseline(&h.ctx).await.unwrap();
        let tools = Scripted::new(&h, |_| panic!("no planes, no planner"));
        let judged = judge_with(&h.ctx, &h.routed, &base, &tools).await.unwrap();
        assert!(judged.tune.is_none());
        assert_eq!(folder_of(&h, &judged), "run-0");
        assert!(!h.ctx.root.join("run-0-untuned").exists());
        assert_eq!(h.calls().len(), 2);
    }

    #[tokio::test]
    async fn the_choice_rewrites_the_held_run_to_the_judged_board() {
        let (mut h, judged) = judged_with_candidates(
            &[TuneVariant::Free, TuneVariant::PourSafe],
            &[
                (refill_ok(), vcc(39.0)),
                (drc_json(&[], &["clearance"]), vcc(40.0)),
                (refill_ok(), vcc(40.0)),
            ],
        )
        .await;
        let pour_safe = h.routed.held.tune.candidates[1].ses.clone();
        judged
            .tune
            .as_ref()
            .unwrap()
            .apply(&mut h.routed.held)
            .unwrap();
        // The session the call would write, and the pass it would report, are those of
        // the judged board.
        assert_eq!(h.routed.held.ses, pour_safe);
        assert_eq!(h.routed.held.kept, "tune");
        assert_eq!(folder_of(&h, &judged), "run-0-pour-safe");
    }
}
