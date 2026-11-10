//! S6 first half: pour ownership suppresses intent widening, not class rules.
use super::*;
use konnect_ipc::IpcRoutingRules;
use serde_json::json;

const TWO: &str = include_str!("../tests/fixtures/specctra_rounded_board.kicad_pcb");
const FOUR: &str = include_str!("../tests/fixtures/specctra_four_layer_back.kicad_pcb");

fn rules() -> IpcEffectiveRoutingRules {
    ["GND", "VCC"]
        .into_iter()
        .map(|net| {
            (
                net.to_string(),
                IpcRoutingRules {
                    class_name: "Default".into(),
                    constituents: vec!["Default".into()],
                    track_width_mm: Some(0.25),
                    clearance_mm: Some(0.2),
                    via_diameter_mm: Some(0.6),
                    via_drill_mm: Some(0.3),
                },
            )
        })
        .collect()
}

fn with_intent(source: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.kicad_pcb");
    std::fs::write(&board, source).unwrap();
    let intent = crate::tools::design_intent::design_intent_path(dir.path());
    std::fs::create_dir_all(intent.parent().unwrap()).unwrap();
    std::fs::write(
        intent,
        json!({"schema_version": 1, "currents": [
            {"net": "GND", "max_current_a": 3.0}, {"net": "VCC", "max_current_a": 3.0}
        ]})
        .to_string(),
    )
    .unwrap();
    (dir, board)
}

#[test]
fn pour_nets_keep_their_class_width_even_when_pours_are_omitted() {
    for source in [TWO, FOUR] {
        let (_dir, board) = with_intent(source);
        let snapshot = crate::routing_context::Snapshot::read(&board).unwrap();
        for omit_pours in [false, true] {
            let bundle = export_dsn_with_context(
                &board,
                source,
                &rules(),
                &snapshot,
                ExportOptions {
                    widths_from_intent: true,
                    omit_pours,
                    ..Default::default()
                },
            )
            .unwrap();
            let report = &bundle.intent_widths;
            let pour = report.iter().find(|r| r["net"] == "GND").unwrap();
            assert_eq!(pour["status"], "not_widened");
            assert_eq!(pour["from_mm"], 0.25);
            assert_eq!(pour["to_mm"], 0.25);
            assert!(pour["reason"].as_str().unwrap().contains("copper pour"));
            let power = report.iter().find(|r| r["net"] == "VCC").unwrap();
            assert_eq!(power["status"], "widened");
            assert!(power["to_mm"].as_f64().unwrap() > 0.25);
            let manifest: serde_json::Value = serde_json::from_str(&bundle.manifest).unwrap();
            assert_eq!(manifest["intent_widths"], json!(report));
            let net = manifest["nets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["name"] == "GND")
                .unwrap();
            let class = manifest["classes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["name"] == net["class_name"])
                .unwrap();
            assert_eq!(class["track_width_um"], 250);
            assert!(net["min_track_width_um"].is_null());
        }
    }
}

#[test]
fn pour_ownership_uses_resolved_net_identity_not_the_ground_name() {
    // KiCad 8/9 numbered nets and a supply pour, rather than a name heuristic.
    let source = FOUR
        .replace(
            "(net \"GND\")\n\t\t(layer \"In2.Cu\")",
            "(net 7)\n\t\t(net_name \"VCC\")\n\t\t(layer \"In2.Cu\")",
        )
        .replacen("(kicad_pcb", "(kicad_pcb (net 7 \"VCC\")", 1);
    let (_dir, board) = with_intent(&source);
    let (out, report) = widen_for_intent(&board, &source, &rules()).unwrap();
    assert_eq!(out["VCC"].track_width_mm, Some(0.25));
    assert!(out["GND"].track_width_mm.unwrap() > 0.25);
    assert_eq!(
        report.iter().find(|r| r["net"] == "VCC").unwrap()["status"],
        "not_widened"
    );
}

#[test]
fn rule_areas_and_non_copper_zones_do_not_suppress_widening() {
    let hook = "(net \"GND\")\n\t\t(layer \"In2.Cu\")";
    for replacement in [
        "(net \"GND\")\n\t\t(layer \"In2.Cu\") (keepout (tracks not_allowed) (vias not_allowed))",
        "(net \"GND\")\n\t\t(layer \"Dwgs.User\")",
    ] {
        let source = FOUR.replace(hook, replacement);
        assert_ne!(source, FOUR);
        let (_dir, board) = with_intent(&source);
        let (out, _) = widen_for_intent(&board, &source, &rules()).unwrap();
        assert!(out["GND"].track_width_mm.unwrap() > 0.25);
    }
}

#[test]
fn pour_ownership_does_not_fill_in_a_missing_class_width() {
    let (_dir, board) = with_intent(FOUR);
    let mut incomplete = rules();
    incomplete.get_mut("GND").unwrap().track_width_mm = None;
    let (out, _) = widen_for_intent(&board, FOUR, &incomplete).unwrap();
    assert_eq!(out["GND"].track_width_mm, None);
    assert!(export_dsn(&board, FOUR, &out).is_err());
}

#[test]
fn no_intent_and_explicit_opt_out_keep_the_dsn_identical() {
    for source in [TWO, FOUR] {
        let (dir, board) = with_intent(source);
        let snapshot = crate::routing_context::Snapshot::read(&board).unwrap();
        let options = ExportOptions::default();
        let before = export_dsn_with_options(&board, source, &rules(), options).unwrap();
        let opted_out =
            export_dsn_with_context(&board, source, &rules(), &snapshot, options).unwrap();
        assert_eq!(before.dsn, opted_out.dsn);
        assert!(opted_out.intent_widths.is_empty());
        std::fs::remove_file(crate::tools::design_intent::design_intent_path(dir.path())).unwrap();
        let snapshot = crate::routing_context::Snapshot::read(&board).unwrap();
        let absent = export_dsn_with_context(
            &board,
            source,
            &rules(),
            &snapshot,
            ExportOptions {
                widths_from_intent: true,
                ..options
            },
        )
        .unwrap();
        assert_eq!(before.dsn, absent.dsn);
        assert!(absent.intent_widths.is_empty());
    }
}
