//! Tests of `pour_check`: one test and one negative control for each guard in
//! docs/POUR_PLAN_DESIGN.md, "Tests, each guard with a negative control".

use super::*;
use crate::pour_plan::{project_rules, settings_for};
use crate::tools::cli::{DrcViolation, ReportItem};
use std::sync::Mutex;

fn proposal(net: &str, layer: &str) -> ProposedZone {
    ProposedZone {
        net: net.into(),
        layer: layer.into(),
        points_mm: vec![(0.0, 0.0), (50.0, 0.0), (50.0, 40.0), (0.0, 40.0)],
        settings: settings_for(&[], &project_rules(None).unwrap(), None),
        reason: "by rule".into(),
        asked: false,
    }
}

fn violation(rule: &str, severity: &str, items: &[&str]) -> DrcViolation {
    DrcViolation {
        severity: severity.to_string(),
        description: "a violation".to_string(),
        rule: rule.to_string(),
        pos: None,
        items: items
            .iter()
            .map(|description| ReportItem {
                description: description.to_string(),
                ..Default::default()
            })
            .collect(),
    }
}

fn open_on(net: &str) -> DrcViolation {
    violation(
        "unconnected_items",
        "error",
        &[
            &format!("Pad 1 [{net}] of U1"),
            &format!("Pad 2 [{net}] of U2"),
        ],
    )
}

fn report(violations: Vec<DrcViolation>, unconnected: Vec<DrcViolation>) -> DrcReport {
    DrcReport {
        violations,
        unconnected_items: Some(unconnected),
        schematic_parity: None,
        ownership_diagnostic: None,
        schematic_parity_diagnostic: None,
    }
}

fn many(rule: &str, severity: &str, count: usize) -> Vec<DrcViolation> {
    (0..count).map(|_| violation(rule, severity, &[])).collect()
}

fn square(x: f64, y: f64, side: f64) -> String {
    format!(
        "(pts (xy {x} {y}) (xy {} {y}) (xy {} {}) (xy {x} {}))",
        x + side,
        x + side,
        y + side,
        y + side
    )
}

/// A zone of `net` on `layer` with one `filled_polygon` per square.
fn zone(net: &str, layer: &str, uuid: &str, squares: &[(f64, f64, f64)]) -> String {
    let fills: String = squares
        .iter()
        .map(|&(x, y, side)| {
            format!(
                "\n    (filled_polygon (layer \"{layer}\") {})",
                square(x, y, side)
            )
        })
        .collect();
    format!(
        "  (zone (net \"{net}\") (layers \"{layer}\") (uuid \"{uuid}\") (hatch edge 0.5)\n    (connect_pads (clearance 0.2)) (min_thickness 0.25)\n    (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.5))\n    (polygon {}){fills})\n",
        square(0.0, 0.0, 100.0)
    )
}

fn board(zones: &[String]) -> String {
    format!(
        "(kicad_pcb (version 20260206) (generator \"pcbnew\")\n{})",
        zones.concat()
    )
}

/// The refilled board of one GND proposal on F.Cu, in one piece.
fn one_piece() -> String {
    board(&[zone("GND", "F.Cu", "z1", &[(0.0, 0.0, 10.0)])])
}

// ─── An error that rises is not clean, a warning is ─────────────────────────────

#[test]
fn an_error_that_rises_is_not_clean_and_a_warning_is() {
    let zones = [proposal("GND", "F.Cu")];
    let base = report(vec![], vec![]);
    let check = verdict(
        &zones,
        &base,
        &report(many("clearance", "error", 1), vec![]),
        &one_piece(),
    )
    .unwrap();
    assert!(!check.clean);
    assert_eq!(
        check.risen,
        [Risen {
            rule: "clearance".into(),
            severity: "error".into(),
            before: 0,
            after: 1
        }]
    );

    let check = verdict(
        &zones,
        &base,
        &report(many("isolated_copper", "warning", 1), vec![]),
        &one_piece(),
    )
    .unwrap();
    assert!(check.clean, "a warning that rises is listed, not unclean");
    assert_eq!(check.risen.len(), 1);
    assert_eq!(check.risen[0].rule, "isolated_copper");
    assert_eq!(check.risen[0].severity, "warning");
}

// ─── A starved thermal that rises is not clean at any severity ─────────────────

#[test]
fn a_starved_thermal_that_rises_is_not_clean_at_any_severity() {
    let zones = [proposal("GND", "F.Cu")];
    let check = verdict(
        &zones,
        &report(vec![], vec![]),
        &report(
            many(crate::zone_fill::STARVED_THERMAL, "warning", 1),
            vec![],
        ),
        &one_piece(),
    )
    .unwrap();
    assert!(!check.clean);
    assert_eq!(
        (check.starved_thermals_before, check.starved_thermals),
        (0, 1)
    );

    let check = verdict(
        &zones,
        &report(
            many(crate::zone_fill::STARVED_THERMAL, "warning", 1),
            vec![],
        ),
        &report(
            many(crate::zone_fill::STARVED_THERMAL, "warning", 1),
            vec![],
        ),
        &one_piece(),
    )
    .unwrap();
    assert!(check.clean);
    assert_eq!(
        (check.starved_thermals_before, check.starved_thermals),
        (1, 1)
    );
}

// ─── Only a count that rises is listed ─────────────────────────────────────────

#[test]
fn only_a_count_that_rises_is_listed() {
    let zones = [proposal("GND", "F.Cu")];
    let check = verdict(
        &zones,
        &report(many("silk_overlap", "warning", 3), vec![]),
        &report(many("silk_overlap", "warning", 3), vec![]),
        &one_piece(),
    )
    .unwrap();
    assert!(check.risen.is_empty(), "{:?}", check.risen);
    let check = verdict(
        &zones,
        &report(many("silk_overlap", "warning", 3), vec![]),
        &report(many("silk_overlap", "warning", 4), vec![]),
        &one_piece(),
    )
    .unwrap();
    assert_eq!(check.risen.len(), 1);
    assert_eq!((check.risen[0].before, check.risen[0].after), (3, 4));
    // A count that falls is not listed either.
    let check = verdict(
        &zones,
        &report(many("silk_overlap", "warning", 3), vec![]),
        &report(many("silk_overlap", "warning", 1), vec![]),
        &one_piece(),
    )
    .unwrap();
    assert!(check.risen.is_empty());
}

#[test]
fn open_counts_each_proposed_net_before_and_after() {
    let zones = [proposal("GND", "F.Cu"), proposal("GND", "B.Cu")];
    let refilled = board(&[
        zone("GND", "F.Cu", "z1", &[(0.0, 0.0, 10.0)]),
        zone("GND", "B.Cu", "z2", &[(0.0, 0.0, 10.0)]),
    ]);
    let check = verdict(
        &zones,
        &report(vec![], vec![open_on("GND"), open_on("GND"), open_on("VCC")]),
        &report(vec![], vec![open_on("VCC")]),
        &refilled,
    )
    .unwrap();
    assert_eq!(
        check.open,
        [NetOpen {
            net: "GND".into(),
            before: 2,
            after: 0
        }]
    );
    assert!(check.clean);
}

// ─── A fill row is its own proposal's ──────────────────────────────────────────

#[test]
fn a_fill_row_is_its_own_proposals() {
    let refilled = board(&[
        zone(
            "GND",
            "B.Cu",
            "old",
            &[(0.0, 0.0, 10.0), (20.0, 0.0, 1.0), (30.0, 0.0, 1.0)],
        ),
        zone("GND", "F.Cu", "new", &[(0.0, 0.0, 10.0)]),
    ]);
    let check = verdict(
        &[proposal("GND", "F.Cu")],
        &report(vec![], vec![]),
        &report(vec![], vec![]),
        &refilled,
    )
    .unwrap();
    assert_eq!(check.fills.len(), 1);
    assert_eq!(check.fills[0].layer, "F.Cu");
    assert_eq!(check.fills[0].pieces, 1);
    assert_eq!(check.fills[0].area_mm2, 100.0);
    assert_eq!(check.fills[0].largest_piece_share, Some(1.0));
}

#[test]
fn a_proposal_kicad_fills_nothing_of_has_no_piece() {
    let refilled = board(&[zone("GND", "F.Cu", "new", &[])]);
    let check = verdict(
        &[proposal("GND", "F.Cu")],
        &report(vec![], vec![]),
        &report(vec![], vec![]),
        &refilled,
    )
    .unwrap();
    assert_eq!(check.fills[0].pieces, 0);
    assert_eq!(check.fills[0].largest_piece_share, None);
}

// ─── A proposal with no fill row is an error ───────────────────────────────────

#[test]
fn a_proposal_with_no_fill_row_is_an_error() {
    let refilled = board(&[zone("GND", "B.Cu", "other", &[(0.0, 0.0, 10.0)])]);
    let error = verdict(
        &[proposal("GND", "F.Cu")],
        &report(vec![], vec![]),
        &report(vec![], vec![]),
        &refilled,
    )
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("no zone of GND on F.Cu"),
        "{error:#}"
    );
}

// ─── No unconnected category is an error, not zero ─────────────────────────────

#[test]
fn no_unconnected_category_is_an_error_not_zero() {
    let mut without = report(vec![], vec![]);
    without.unconnected_items = None;
    assert!(verdict(
        &[proposal("GND", "F.Cu")],
        &without,
        &report(vec![], vec![]),
        &one_piece()
    )
    .is_err());
    let mut with = report(vec![], vec![]);
    with.unconnected_items = None;
    assert!(verdict(
        &[proposal("GND", "F.Cu")],
        &report(vec![], vec![]),
        &with,
        &one_piece()
    )
    .is_err());
}

// ─── The scratch projects and the check ────────────────────────────────────────

/// A refill that writes a fill into every zone of its scratch board (one square on the
/// zone's own layer, as `zone_sexp` names it) and answers with a canned report; it
/// records the boards it was given.
struct Scripted {
    without: DrcReport,
    with: DrcReport,
    seen: Mutex<Vec<PathBuf>>,
}

impl Scripted {
    fn new(without: DrcReport, with: DrcReport) -> Self {
        Self {
            without,
            with,
            seen: Mutex::new(Vec::new()),
        }
    }
}

/// `text` with one `filled_polygon` on its own layer added to every `(zone …)` that
/// `zone_sexp` wrote (`(layers "X")` after its net).
fn filled(text: &str) -> String {
    let mut out = String::new();
    for (i, part) in text.split("\n  (zone ").enumerate() {
        if i == 0 {
            out.push_str(part);
            continue;
        }
        out.push_str("\n  (zone ");
        let layer = part
            .split("(layers \"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap_or("F.Cu");
        out.push_str(&part.replacen(
            "\n    (polygon (pts",
            &format!(
                "\n    (filled_polygon (layer \"{layer}\") (pts (xy 0 0) (xy 5 0) (xy 5 5) (xy 0 5)))\n    (polygon (pts"
            ),
            1,
        ));
    }
    out
}

impl Refill for Scripted {
    fn refill<'a>(&'a self, board: &'a Path) -> RefillCall<'a> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(board.to_path_buf());
            let text = std::fs::read_to_string(board)?;
            std::fs::write(board, filled(&text))?;
            let with =
                board.parent().and_then(|p| p.file_name()) == Some(std::ffi::OsStr::new(WITH));
            Ok(if with {
                self.with.clone()
            } else {
                self.without.clone()
            })
        })
    }
}

/// A user's project: the board, its project file with its own severities, and a
/// design-intent record.
fn project(dir: &Path) -> PathBuf {
    let board = dir.join("board.kicad_pcb");
    std::fs::write(&board, crate::pour_plan::tests::pours_without_pours()).unwrap();
    std::fs::write(
        dir.join("board.kicad_pro"),
        serde_json::json!({
            "board": { "design_settings": { "rule_severities": {
                "starved_thermal": "ignore", "unconnected_items": "ignore",
                "isolated_copper": "ignore"
            } } }
        })
        .to_string(),
    )
    .unwrap();
    std::fs::create_dir_all(dir.join(".konnect")).unwrap();
    std::fs::write(
        dir.join(".konnect").join("design_intent.json"),
        "{\"schema_version\": 1}\n",
    )
    .unwrap();
    board
}

/// Every file below `dir` with its bytes.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(folder) = stack.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[tokio::test]
async fn the_owners_files_are_never_written() {
    let user = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let board = project(user.path());
    let before = snapshot(user.path());
    let text = std::fs::read_to_string(&board).unwrap();
    let folder = run_folder(state.path(), &text, "1").unwrap();
    let zones = [proposal("GND", "F.Cu"), proposal("GND", "B.Cu")];
    let refill = Scripted::new(report(vec![], vec![open_on("GND")]), report(vec![], vec![]));
    let check = check(&folder, &board, &text, &zones, &refill)
        .await
        .unwrap();
    assert_eq!(
        snapshot(user.path()),
        before,
        "the user's folder is unchanged"
    );
    assert_eq!(check.fills.len(), 2);
    assert!(
        check.fills.iter().all(|f| f.pieces == 1),
        "{:?}",
        check.fills
    );
    assert_eq!(
        check.open,
        [NetOpen {
            net: "GND".into(),
            before: 1,
            after: 0
        }]
    );
    let seen = refill.seen.lock().unwrap().clone();
    assert_eq!(
        seen,
        [
            folder.join(WITHOUT).join("board.kicad_pcb"),
            folder.join(WITH).join("board.kicad_pcb")
        ]
    );
    // The scratch board without the proposals is the text as read, refilled.
    let without = std::fs::read_to_string(folder.join(WITHOUT).join("board.kicad_pcb")).unwrap();
    assert_eq!(without, filled(&text));
}

// ─── The scratch project cannot hide a pour's faults ───────────────────────────

#[tokio::test]
async fn the_scratch_project_cannot_hide_a_pours_faults() {
    let user = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let board = project(user.path());
    let text = std::fs::read_to_string(&board).unwrap();
    let folder = run_folder(state.path(), &text, "2").unwrap();
    let refill = Scripted::new(report(vec![], vec![]), report(vec![], vec![]));
    check(&folder, &board, &text, &[proposal("GND", "F.Cu")], &refill)
        .await
        .unwrap();
    for name in [WITHOUT, WITH] {
        let project: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(folder.join(name).join("board.kicad_pro")).unwrap(),
        )
        .unwrap();
        let severities = &project["board"]["design_settings"]["rule_severities"];
        assert_eq!(severities["starved_thermal"], "warning", "{name}");
        assert_eq!(severities["isolated_copper"], "warning", "{name}");
        assert_eq!(severities["unconnected_items"], "error", "{name}");
    }
    // The user's own project still ignores them.
    let own = std::fs::read_to_string(user.path().join("board.kicad_pro")).unwrap();
    assert!(own.contains("\"starved_thermal\":\"ignore\""), "{own}");
}

#[tokio::test]
async fn a_step_that_fails_names_the_step() {
    struct Failing;
    impl Refill for Failing {
        fn refill<'a>(&'a self, _board: &'a Path) -> RefillCall<'a> {
            Box::pin(async { anyhow::bail!("kicad-cli is not there") })
        }
    }
    let user = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let board = project(user.path());
    let text = std::fs::read_to_string(&board).unwrap();
    let folder = run_folder(state.path(), &text, "3").unwrap();
    let error = check(&folder, &board, &text, &[proposal("GND", "F.Cu")], &Failing)
        .await
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("step 2"), "{message}");
    assert!(message.contains("kicad-cli is not there"), "{message}");
    // The folder is kept as the record of what KiCad was shown.
    assert!(folder.join(WITHOUT).join("board.kicad_pcb").is_file());
}

// ─── Two calls never share a folder ────────────────────────────────────────────

#[test]
fn two_calls_never_share_a_folder() {
    let state = tempfile::tempdir().unwrap();
    let first = run_folder(state.path(), "(kicad_pcb)", "42").unwrap();
    assert!(first.is_dir());
    assert!(first.starts_with(state.path().join(PLANS_FOLDER)));
    let name = first.file_name().unwrap().to_str().unwrap().to_string();
    assert!(
        name.starts_with("42-") && name.len() == "42-".len() + 8,
        "{name}"
    );
    assert!(run_folder(state.path(), "(kicad_pcb)", "42").is_err());
    // Another text is another folder.
    assert!(run_folder(state.path(), "(kicad_pcb )", "42").is_ok());
}

// ─── kicad-cli runs on private settings ────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn kicad_cli_runs_on_private_settings() {
    let dir = tempfile::tempdir().unwrap();
    let script = r#"#!/bin/sh
dir="$(cd "$(dirname "$0")" && pwd)"
echo "KICAD_CONFIG_HOME=${KICAD_CONFIG_HOME-unset}" >> "$dir/calls.log"
out=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--output" ]; then out="$a"; fi
  prev="$a"
done
printf '{"violations": [], "unconnected_items": []}' > "$out"
exit 0
"#;
    let cli = crate::tools::cli::test_support::write_script(dir.path(), "kicad-cli", script, "");
    let config = dir.path().join("private-config");
    std::fs::create_dir_all(&config).unwrap();
    let board = dir.path().join("board.kicad_pcb");
    std::fs::write(&board, "(kicad_pcb)").unwrap();
    let refill = KicadCli {
        kicad_cli: cli.display().to_string(),
        config_home: config.clone(),
    };
    let report = refill.refill(&board).await.unwrap();
    assert!(report.violations.is_empty());
    let calls = std::fs::read_to_string(dir.path().join("calls.log")).unwrap();
    assert_eq!(
        calls.trim(),
        format!("KICAD_CONFIG_HOME={}", config.display())
    );
}
