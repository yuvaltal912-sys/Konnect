//! PR 15 follow-up: `run_design_review` reads a stored DRC baseline. The
//! review and the baseline tools run over a scratch project against a
//! stand-in `kicad-cli` that serves whatever DRC report the test put there.

use super::*;
use crate::mcp::protocol::ToolContent;
use crate::tools::check_baselines::handle_save_check_baseline;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const SCHEMATIC: &str = r#"(kicad_sch
  (version 20250610)
  (generator "konnect")
  (generator_version "10.0")
  (uuid "11111111-1111-4111-8111-111111111111")
  (paper "A4")
  (lib_symbols
    (symbol "Device:R"
      (property "Reference" "R" (at 0 0 0))
      (property "Value" "R" (at 0 0 0))
      (symbol "R_1_1"
        (pin passive line (at 0 0 0) (length 2.54) (name "~") (number "1"))
      )
    )
  )
  (label "SIG" (at 20 20 0) (uuid "22222222-2222-4222-8222-222222222222"))
  (symbol
    (lib_id "Device:R")
    (at 20 20 0)
    (unit 1)
    (in_bom yes)
    (on_board yes)
    (dnp no)
    (uuid "33333333-3333-4333-8333-333333333333")
    (property "Reference" "R1" (at 20 20 0))
    (property "Value" "10k" (at 20 20 0))
    (property "Footprint" "Resistor_SMD:R_0603_1608Metric" (at 20 20 0))
    (pin "1" (uuid "44444444-4444-4444-8444-444444444444"))
  )
)
"#;

/// A board the file audits pass: an outline, one footprint whose pads carry
/// UUIDs, two fab-sized tracks and a via, all with UUIDs DRC items can name.
const BOARD: &str = r#"(kicad_pcb (version 20260206) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 60 40) (stroke (width 0.05) (type solid)) (fill no) (layer "Edge.Cuts") (uuid "edge"))
  (footprint "Resistor_SMD:R_0603_1608Metric" (layer "F.Cu") (uuid "fp-r1") (at 10 10 0)
    (property "Reference" "R1" (at 0 -3 0) (layer "F.SilkS") (uuid "fp-r1-ref") (effects (font (size 1 1))))
    (pad "1" smd rect (at -0.8 0 0) (size 0.9 0.95) (layers "F.Cu" "F.Mask" "F.Paste") (net "N1") (uuid "pad-r1-1"))
    (pad "2" smd rect (at 0.8 0 0) (size 0.9 0.95) (layers "F.Cu" "F.Mask" "F.Paste") (net "N2") (uuid "pad-r1-2")))
  (segment (start 20 5) (end 30 5) (width 0.25) (layer "F.Cu") (net "N1") (uuid "seg-a"))
  (segment (start 20 6) (end 30 6) (width 0.25) (layer "F.Cu") (net "N2") (uuid "seg-b"))
  (via (at 40 20) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net "N1") (uuid "via-a"))
)
"#;

struct Scene {
    dir: tempfile::TempDir,
    ctx: ToolContext,
}

impl Scene {
    /// The stand-in copies `report.json` to the report path and logs each run;
    /// while a `fail` file exists it exits 1, as a broken kicad-cli would.
    fn new() -> Scene {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("top.kicad_sch"), SCHEMATIC).unwrap();
        std::fs::write(dir.path().join("top.kicad_pcb"), BOARD).unwrap();
        let (report, runs, fail) = (
            dir.path().join("report.json"),
            dir.path().join("runs.log"),
            dir.path().join("fail"),
        );
        let script = crate::tools::cli::test_support::write_script(
            dir.path(),
            "fake-kicad-cli",
            &format!(
                "#!/bin/sh\necho run >> '{}'\n[ -e '{}' ] && exit 1\ncp '{}' \"$4\"\nexit 0\n",
                runs.display(),
                fail.display(),
                report.display()
            ),
            "",
        );
        let ctx = ToolContext::new(
            crate::tools::ServerConfig {
                kicad_cli: script.to_str().unwrap().to_owned(),
                ..Default::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        Scene { dir, ctx }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
    fn serve(&self, report: &Value) {
        std::fs::write(self.path("report.json"), report.to_string()).unwrap();
    }
    fn runs(&self) -> usize {
        std::fs::read_to_string(self.path("runs.log"))
            .map(|log| log.lines().count())
            .unwrap_or(0)
    }
    fn board(&self) -> String {
        self.path("top.kicad_pcb").display().to_string()
    }
    async fn save(&self, name: &str) {
        let saved =
            handle_save_check_baseline(&json!({ "board": self.board(), "name": name }), &self.ctx)
                .await
                .unwrap();
        assert!(!saved.is_error, "{}", text(&saved));
    }
    async fn review(&self, baseline: Option<&str>) -> CallToolResult {
        let mut args = json!({
            "schematic": self.path("top.kicad_sch").display().to_string(),
            "board": self.board(),
        });
        if let Some(name) = baseline {
            args["baseline"] = json!(name);
        }
        handle_run_design_review(&args, &self.ctx).await.unwrap()
    }
}

fn text(result: &CallToolResult) -> &str {
    match &result.content[0] {
        ToolContent::Text { text } => text,
        _ => panic!("expected text"),
    }
}

fn review_body(result: &CallToolResult) -> Value {
    assert!(!result.is_error, "{}", text(result));
    serde_json::from_str::<Value>(text(result)).unwrap()["design_review"].clone()
}

fn item(description: &str, uuid: &str, x: f64) -> Value {
    json!({ "description": description, "uuid": uuid, "pos": { "x": x, "y": 1.0 } })
}

fn violation(rule: &str, severity: &str, description: &str, items: Vec<Value>) -> Value {
    json!({ "type": rule, "severity": severity, "description": description, "items": items })
}

/// The tracks too close; the measured value and the position change between
/// runs, the identity does not.
fn clearance(actual: &str, x: f64, severity: &str) -> Value {
    violation(
        "clearance",
        severity,
        &format!(
            "Clearance violation (netclass 'Default' clearance 0.2000 mm; actual {actual} mm)"
        ),
        vec![
            item("Track [N1] on F.Cu, length 10.0000 mm", "seg-a", x),
            item("Track [N2] on F.Cu, length 10.0000 mm", "seg-b", x),
        ],
    )
}

fn unrouted() -> Value {
    violation(
        "unconnected_items",
        "error",
        "Missing connection between items",
        vec![
            item("Pad 1 [N1] of R1 on F.Cu", "pad-r1-1", 9.2),
            item("Via [N1] on F.Cu - B.Cu", "via-a", 40.0),
        ],
    )
}

fn hole_clearance() -> Value {
    violation(
        "hole_clearance",
        "error",
        "Hole clearance violation (clearance 0.2500 mm; actual 0.1000 mm)",
        vec![
            item("Via [N1] on F.Cu - B.Cu", "via-a", 40.0),
            item("Pad 2 [N2] of R1 on F.Cu", "pad-r1-2", 10.8),
        ],
    )
}

fn report(violations: Vec<Value>, unconnected: Option<Vec<Value>>) -> Value {
    let mut report = json!({ "violations": violations, "schematic_parity": [] });
    if let Some(unconnected) = unconnected {
        report["unconnected_items"] = json!(unconnected);
    }
    report
}

fn drc_findings(review: &Value) -> Vec<Value> {
    review["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["category"] == "drc")
        .cloned()
        .collect()
}

fn sha256(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}

#[cfg(unix)]
#[tokio::test]
async fn only_new_drc_findings_count_against_the_verdict() {
    let scene = Scene::new();
    scene.serve(&report(
        vec![clearance("0.1500", 25.0, "error")],
        Some(vec![unrouted()]),
    ));
    scene.save("pre-route").await;
    // Routing fixed the connection, the clearance was re-measured and moved,
    // and the route added a hole clearance error.
    scene.serve(&report(
        vec![clearance("0.0500", 26.0, "error"), hole_clearance()],
        Some(vec![]),
    ));

    let absolute = review_body(&scene.review(None).await);
    assert_eq!(absolute["errors"], 2, "{absolute}");
    assert!(absolute["baseline"].is_null());

    let review = review_body(&scene.review(Some("pre-route")).await);
    assert_eq!(review["status"], "complete", "{review}");
    assert_eq!(review["errors"], 1, "only the new error counts");
    let counted = drc_findings(&review);
    assert_eq!(counted.len(), 1, "{review}");
    assert_eq!(counted[0]["rule"], "hole_clearance");
    assert_eq!(counted[0]["baseline_change"], "new");
    assert!(
        counted[0]["location"].is_object(),
        "the finding keeps its position"
    );
    assert_eq!(review["drc"]["errors"], 2, "the DRC summary stays absolute");

    let baseline = &review["baseline"];
    let stored = scene.path(".konnect/baselines/drc/pre-route.json");
    assert_eq!(baseline["status"], "compared");
    assert_eq!(baseline["name"], "pre-route");
    assert_eq!(baseline["check"], "drc");
    assert_eq!(baseline["path"], stored.display().to_string());
    assert_eq!(baseline["revision"], sha256(&stored));
    assert_eq!(baseline["new_count"], 1);
    assert_eq!(baseline["unchanged_count"], 1);
    assert_eq!(baseline["resolved_count"], 1);
    assert_eq!(baseline["resolved"][0]["category"], "unconnected_items");
    assert_eq!(baseline["source_changed_since_baseline"], false);
    assert_eq!(baseline["keying"]["items_by_uuid"], 4);
    assert_eq!(baseline["warnings"], json!([]));
    let verdict = review["verdict"].as_str().unwrap();
    assert!(verdict.starts_with("NOT READY"), "{verdict}");
    assert!(
        verdict.contains("baseline 'pre-route': 1 new, 1 unchanged, 1 resolved"),
        "{verdict}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_board_unchanged_since_its_baseline_can_look_good() {
    let scene = Scene::new();
    scene.serve(&report(
        vec![clearance("0.1500", 25.0, "error")],
        Some(vec![unrouted()]),
    ));
    scene.save("accepted").await;
    // The same two violations, re-measured and listed in another order.
    scene.serve(&report(
        vec![clearance("0.1499", 30.0, "error")],
        Some(vec![unrouted()]),
    ));

    let absolute = review_body(&scene.review(None).await);
    assert!(absolute["verdict"]
        .as_str()
        .unwrap()
        .starts_with("NOT READY"));

    let review = review_body(&scene.review(Some("accepted")).await);
    assert_eq!(review["status"], "complete", "{review}");
    assert!(drc_findings(&review).is_empty(), "{review}");
    assert_eq!(review["baseline"]["unchanged_count"], 2);
    assert_eq!(review["baseline"]["new_count"], 0);
    let verdict = review["verdict"].as_str().unwrap();
    assert!(verdict.starts_with("LOOKS GOOD"), "{verdict}");
    assert!(verdict.contains("baseline 'accepted'"), "{verdict}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_severity_increase_counts_and_a_decrease_does_not() {
    let scene = Scene::new();
    scene.serve(&report(
        vec![clearance("0.1500", 25.0, "warning"), hole_clearance()],
        Some(vec![]),
    ));
    scene.save("before").await;
    let mut lowered = hole_clearance();
    lowered["severity"] = json!("warning");
    scene.serve(&report(
        vec![clearance("0.1500", 25.0, "error"), lowered],
        Some(vec![]),
    ));

    let review = review_body(&scene.review(Some("before")).await);
    let counted = drc_findings(&review);
    assert_eq!(counted.len(), 1, "{review}");
    assert_eq!(counted[0]["rule"], "clearance");
    assert_eq!(counted[0]["baseline_change"], "severity_raised");
    assert_eq!(counted[0]["baseline_severity"], "warning");
    assert_eq!(review["errors"], 1);
    assert_eq!(
        review["warnings"], 0,
        "the lowered one is known, not counted"
    );
    assert_eq!(review["baseline"]["severity_raised_count"], 1);
    assert_eq!(review["baseline"]["severity_lowered_count"], 1);
    assert!(review["verdict"].as_str().unwrap().starts_with("NOT READY"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_category_the_baseline_did_not_report_is_counted_not_trusted() {
    let scene = Scene::new();
    // An older kicad-cli reported no unconnected items at all.
    scene.serve(&report(vec![], None));
    scene.save("old-cli").await;
    scene.serve(&report(vec![], Some(vec![unrouted()])));

    let review = review_body(&scene.review(Some("old-cli")).await);
    let counted = drc_findings(&review);
    assert_eq!(counted.len(), 1, "{review}");
    assert_eq!(counted[0]["baseline_change"], "not_compared");
    let baseline = &review["baseline"];
    assert_eq!(
        baseline["categories_not_compared"],
        json!(["unconnected_items"])
    );
    assert_eq!(baseline["not_compared_count"], 1);
    assert_eq!(baseline["resolved_count"], 0);
    assert!(baseline["warnings"][0]
        .as_str()
        .unwrap()
        .starts_with("categories_not_compared"));
    assert!(review["verdict"].as_str().unwrap().starts_with("NOT READY"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_missing_or_unusable_baseline_is_refused_before_anything_runs() {
    let scene = Scene::new();
    let refused = |result: CallToolResult, needle: &str| {
        assert!(result.is_error, "{}", text(&result));
        assert!(text(&result).contains(needle), "{}", text(&result));
    };

    refused(scene.review(Some("nope")).await, "Stored: none");
    assert_eq!(scene.runs(), 0, "a missing baseline runs nothing");

    scene.serve(&report(vec![], Some(vec![])));
    scene.save("good").await;
    let runs = scene.runs();
    refused(
        scene.review(Some("nope")).await,
        "No drc baseline named 'nope' for this project. Stored: good.",
    );
    refused(scene.review(Some("../good")).await, "baseline");

    // A baseline needs the board it was taken on.
    let without_board = json!({
        "schematic": scene.path("top.kicad_sch").display().to_string(),
        "baseline": "good",
    });
    refused(
        handle_run_design_review(&without_board, &scene.ctx)
            .await
            .unwrap(),
        "needs board",
    );
    std::fs::write(scene.path("other.kicad_pcb"), BOARD).unwrap();
    let other_board = json!({
        "schematic": scene.path("top.kicad_sch").display().to_string(),
        "board": scene.path("other.kicad_pcb").display().to_string(),
        "baseline": "good",
    });
    refused(
        handle_run_design_review(&other_board, &scene.ctx)
            .await
            .unwrap(),
        "was taken on top.kicad_pcb, not other.kicad_pcb",
    );

    // A damaged file is refused and left as it is.
    let damaged = scene.path(".konnect/baselines/drc/damaged.json");
    std::fs::write(&damaged, "{ not json").unwrap();
    refused(scene.review(Some("damaged")).await, "cannot be used");
    assert_eq!(std::fs::read_to_string(&damaged).unwrap(), "{ not json");

    assert_eq!(scene.runs(), runs, "no refusal ran DRC");
}

#[cfg(unix)]
#[tokio::test]
async fn drc_that_cannot_run_leaves_the_baseline_uncompared_and_the_review_incomplete() {
    let scene = Scene::new();
    scene.serve(&report(
        vec![clearance("0.1500", 25.0, "error")],
        Some(vec![]),
    ));
    scene.save("before").await;
    std::fs::write(scene.path("fail"), "").unwrap();

    let review = review_body(&scene.review(Some("before")).await);
    assert_eq!(review["status"], "partial", "{review}");
    assert_eq!(
        review["verdict"],
        "INCOMPLETE — review could not evaluate the full design"
    );
    let baseline = &review["baseline"];
    assert_eq!(baseline["status"], "not_compared");
    assert_eq!(baseline["reason"], "drc_unavailable");
    assert_eq!(baseline["name"], "before");
    assert_eq!(
        baseline["revision"],
        sha256(&scene.path(".konnect/baselines/drc/before.json"))
    );
    assert!(baseline["new_count"].is_null(), "nothing was compared");
}
