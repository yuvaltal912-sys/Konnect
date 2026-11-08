use super::*;
use serde_json::json;

fn board(extra_setup: &str, items: &str) -> String {
    format!(
        r#"(kicad_pcb (version 20260206) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
  (setup {extra_setup})
  (gr_rect (start 0 0) (end 60 40) (stroke (width 0.05) (type solid)) (fill no) (layer "Edge.Cuts") (uuid "e1"))
  (footprint "Fixture:J" (layer "F.Cu") (uuid "fj") (at 5 5 0)
    (property "Reference" "J1" (at 0 -3 0) (layer "F.SilkS") (uuid "fj-r") (effects (font (size 1 1))))
    (pad "1" smd rect (at 0 0 0) (size 2 2) (layers "F.Cu" "F.Mask") (net "PWR") (uuid "pj1"))
    (pad "2" smd rect (at 3 0 0) (size 2 2) (layers "F.Cu" "F.Mask") (net "SIG") (uuid "pj2")))
  {items}
)"#
    )
}

const TRACKS: &str = r#"
  (segment (start 10 10) (end 20 10) (width 0.25) (layer "F.Cu") (net "PWR") (uuid "t-thin"))
  (segment (start 20 10) (end 30 10) (width 1.0) (layer "F.Cu") (net "PWR") (uuid "t-wide"))
  (segment (start 10 20) (end 20 20) (width 0.15) (layer "F.Cu") (net "SIG") (uuid "t-sig"))
  (via (at 30 10) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net "PWR") (uuid "v1"))"#;

struct Fixture {
    _dir: tempfile::TempDir,
    board: std::path::PathBuf,
}

fn fixture(board_text: &str, intent: Option<Value>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("demo.kicad_pcb");
    std::fs::write(&board, board_text).unwrap();
    std::fs::write(dir.path().join("demo.kicad_pro"), "{}").unwrap();
    if let Some(intent) = intent {
        let path = design_intent_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_string_pretty(&intent).unwrap()).unwrap();
    }
    Fixture { _dir: dir, board }
}

fn run(f: &Fixture, default_rise: Option<f64>) -> Value {
    let result = audit(&f.board, default_rise).unwrap();
    assert!(!result.is_error, "{result:?}");
    match result.content.first() {
        Some(crate::mcp::protocol::ToolContent::Text { text }) => {
            serde_json::from_str(text).unwrap()
        }
        other => panic!("expected text, got {other:?}"),
    }
}

fn net<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["nets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["net"] == name)
        .unwrap_or_else(|| panic!("no net {name}: {report}"))
}

fn findings_for<'a>(report: &'a Value, name: &str) -> Vec<&'a Value> {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["net"] == name)
        .collect()
}

#[test]
fn ipc_2221_matches_the_published_chart() {
    // 1 A, 10 °C rise, 1 oz outer: about 12 mil (0.30 mm).
    let outer = required_area_mm2(1.0, 10.0, true) / MM_PER_OZ;
    assert!((outer - 0.30).abs() < 0.01, "{outer}");
    // Inner layers need 2^(1/0.725) ≈ 2.6 times the section.
    let inner = required_area_mm2(1.0, 10.0, false) / MM_PER_OZ;
    assert!((inner / outer - 2.6).abs() < 0.02, "{inner}");
    // The two directions of the formula agree.
    let a = required_area_mm2(3.0, 20.0, true);
    assert!((capacity_a(a, 20.0, true) - 3.0).abs() < 1e-9);
}

#[test]
fn no_record_says_there_is_nothing_to_check() {
    let f = fixture(&board("", TRACKS), None);
    let report = run(&f, None);
    assert_eq!(report["status"], "no_design_intent");
    assert_eq!(report["findings"], json!([]));
}

#[test]
fn a_record_without_currents_lists_the_rails_that_lack_one() {
    let f = fixture(
        &board("", TRACKS),
        Some(
            json!({ "schema_version": 1, "rails": [{ "name": "MAIN", "nets": ["PWR"], "voltage_v": 5.0 }] }),
        ),
    );
    let report = run(&f, None);
    assert_eq!(report["status"], "no_currents_recorded");
    assert_eq!(report["rails_without_current"], json!(["MAIN"]));
}

#[test]
fn an_undersized_segment_is_an_error_named_by_uuid() {
    let f = fixture(
        &board("", TRACKS),
        Some(json!({ "schema_version": 1, "currents": [{ "net": "PWR", "max_current_a": 2.0 }] })),
    );
    let report = run(&f, None);
    assert_eq!(report["status"], "checked");
    let pwr = net(&report, "PWR");
    assert_eq!(pwr["tracks_checked"], 2);
    assert_eq!(pwr["undersized_count"], 1);
    assert_eq!(pwr["undersized"][0]["uuid"], "t-thin");
    assert_eq!(pwr["narrowest"]["uuid"], "t-thin");
    assert_eq!(pwr["temp_rise_source"], "built_in_default");
    // 2 A at 10 °C on 1 oz outer copper needs about 0.78 mm.
    let required = pwr["undersized"][0]["required_mm"].as_f64().unwrap();
    assert!((required - 0.78).abs() < 0.02, "{required}");
    let errors: Vec<_> = findings_for(&report, "PWR")
        .into_iter()
        .filter(|f| f["severity"] == "error")
        .collect();
    assert_eq!(errors.len(), 1, "{report}");
    assert!(errors[0]["issue"]
        .as_str()
        .unwrap()
        .contains("narrowest 0.25 mm on F.Cu"));
    // The signal net has no recorded current and is not audited.
    assert!(report["nets"]
        .as_array()
        .unwrap()
        .iter()
        .all(|n| n["net"] != "SIG"));
}

#[test]
fn a_zone_of_the_net_on_the_same_layer_softens_the_finding() {
    let zone = r#"
  (zone (net "PWR") (layer "F.Cu") (uuid "z1") (hatch edge 0.5)
    (polygon (pts (xy 8 8) (xy 32 8) (xy 32 12) (xy 8 12))))"#;
    let f = fixture(
        &board("", &format!("{TRACKS}{zone}")),
        Some(json!({ "schema_version": 1, "currents": [{ "net": "PWR", "max_current_a": 2.0 }] })),
    );
    let report = run(&f, None);
    assert_eq!(net(&report, "PWR")["zone_layers"], json!(["F.Cu"]));
    let track_finding = findings_for(&report, "PWR")
        .into_iter()
        .find(|f| f["issue"].as_str().unwrap().contains("track segment"))
        .unwrap();
    assert_eq!(track_finding["severity"], "warning");
    assert!(track_finding["issue"]
        .as_str()
        .unwrap()
        .contains("may share the load"));
}

#[test]
fn a_recorded_net_missing_from_the_board_is_reported() {
    let f = fixture(
        &board("", TRACKS),
        Some(json!({ "schema_version": 1, "currents": [{ "net": "/PWR", "max_current_a": 1.0 }] })),
    );
    let report = run(&f, None);
    assert_eq!(report["nets_not_on_board"], json!(["/PWR"]));
    assert_eq!(findings_for(&report, "/PWR")[0]["severity"], "warning");
}

#[test]
fn rail_currents_apply_to_each_rail_net_and_heavier_copper_needs_less_width() {
    let intent = |oz: Option<f64>| {
        let mut v = json!({
            "schema_version": 1,
            "rails": [
                { "name": "MAIN", "nets": ["PWR"], "voltage_v": 5.0, "max_current_a": 1.0, "max_temp_rise_c": 20.0 },
                { "name": "IO", "nets": ["SIG"], "voltage_v": 3.3 }
            ]
        });
        if let Some(oz) = oz {
            v["fab_target"] = json!({ "outer_copper_oz": oz });
        }
        v
    };
    let one = run(&fixture(&board("", TRACKS), Some(intent(None))), None);
    let two = run(&fixture(&board("", TRACKS), Some(intent(Some(2.0)))), None);
    let pwr1 = net(&one, "PWR");
    assert_eq!(pwr1["source"], "rail:MAIN");
    assert_eq!(pwr1["temp_rise_c"], 20.0);
    assert_eq!(pwr1["temp_rise_source"], "design_intent");
    assert_eq!(one["rails_without_current"], json!(["IO"]));
    let w1 = pwr1["narrowest"]["required_mm"].as_f64().unwrap();
    let w2 = net(&two, "PWR")["narrowest"]["required_mm"]
        .as_f64()
        .unwrap();
    assert!((w1 / w2 - 2.0).abs() < 0.02, "{w1} {w2}");
    assert_eq!(two["copper"]["F.Cu"]["source"], "design_intent");
    assert_eq!(one["copper"]["F.Cu"]["source"], "default_1oz");
}

#[test]
fn the_board_stackup_supplies_copper_thickness_when_the_record_does_not() {
    let stackup = r#"(stackup (layer "F.Cu" (type "copper") (thickness 0.07)) (layer "B.Cu" (type "copper") (thickness 0.035)))"#;
    let f = fixture(
        &board(stackup, TRACKS),
        Some(json!({ "schema_version": 1, "currents": [{ "net": "PWR", "max_current_a": 2.0 }] })),
    );
    let report = run(&f, None);
    assert_eq!(report["copper"]["F.Cu"]["source"], "board_stackup");
    assert_eq!(report["copper"]["F.Cu"]["thickness_mm"], 0.07);
}

#[test]
fn one_via_cannot_carry_a_heavy_net_alone() {
    let f = fixture(
        &board("", TRACKS),
        Some(json!({ "schema_version": 1, "currents": [{ "net": "PWR", "max_current_a": 3.0 }] })),
    );
    let report = run(&f, None);
    let vias = &net(&report, "PWR")["vias"];
    assert_eq!(vias["count"], 1);
    assert!(vias["needed_in_parallel"].as_u64().unwrap() >= 3, "{vias}");
    assert!(findings_for(&report, "PWR")
        .iter()
        .any(|f| f["issue"].as_str().unwrap().contains("via(s)")));
}

#[test]
fn the_call_default_rise_is_used_and_named() {
    let f = fixture(
        &board("", TRACKS),
        Some(json!({ "schema_version": 1, "currents": [{ "net": "PWR", "max_current_a": 2.0 }] })),
    );
    let report = run(&f, Some(40.0));
    let pwr = net(&report, "PWR");
    assert_eq!(pwr["temp_rise_c"], 40.0);
    assert_eq!(pwr["temp_rise_source"], "call_default");
}

#[test]
fn an_unusable_record_is_refused_not_ignored() {
    let f = fixture(
        &board("", TRACKS),
        Some(json!({ "schema_version": 1, "currents": "nope" })),
    );
    let result = audit(&f.board, None).unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn the_served_tool_refuses_a_bad_default_rise() {
    let f = fixture(&board("", TRACKS), None);
    let ctx = ToolContext::new(
        crate::tools::ServerConfig::default(),
        std::sync::Arc::new(crate::router::ToolRouter::new()),
    );
    let result = handle_audit_trace_current(
        &json!({ "board": f.board.to_str().unwrap(), "default_temp_rise_c": 0 }),
        &ctx,
    )
    .await
    .unwrap();
    assert!(result.is_error);
}
