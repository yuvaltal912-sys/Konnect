//! PR 16b follow-up: unplated holes keep the project's hole clearance, and
//! geometry the DSN cannot carry yet is refused rather than dropped.
//!
//! Unplated holes are image keepouts with no rule of their own, so
//! Freerouting kept only its copper clearance (0.2 mm) from them while the
//! project asked for 0.25 mm. Each keepout now grows by the difference.

use super::*;
use konnect_ipc::IpcRoutingRules;

const FOUR_LAYER: &str = include_str!("../tests/fixtures/specctra_four_layer_back.kicad_pcb");
const HOOK: &str = "\t\t(duplicate_pad_numbers_are_jumpers no)";

fn rules() -> IpcEffectiveRoutingRules {
    ["GND", "VCC"]
        .into_iter()
        .map(|net| {
            (
                net.to_string(),
                IpcRoutingRules {
                    class_name: "Default".to_string(),
                    constituents: vec!["Default".to_string()],
                    track_width_mm: Some(0.25),
                    clearance_mm: Some(0.2),
                    via_diameter_mm: Some(0.6),
                    via_drill_mm: Some(0.3),
                },
            )
        })
        .collect()
}

/// R1 with a 3.0 mm round unplated hole and a 1 x 2 mm unplated slot.
fn board_with_holes() -> String {
    let board = FOUR_LAYER.replacen(
        HOOK,
        "\t\t(pad \"\" np_thru_hole circle (at 0 3) (size 3 3) (drill 3) (layers \"*.Cu\" \"*.Mask\") (uuid \"h1\"))\n\t\t(pad \"\" np_thru_hole oval (at 4 3) (size 1 2) (drill oval 1 2) (layers \"*.Cu\" \"*.Mask\") (uuid \"h2\"))\n\t\t(duplicate_pad_numbers_are_jumpers no)",
        1,
    );
    assert_ne!(board, FOUR_LAYER, "fixture shape changed");
    board
}

fn export_with(board: &str, hole_clearance_um: Option<i64>) -> ExportBundle {
    let options = ExportOptions {
        hole_clearance_um,
        ..Default::default()
    };
    export_dsn_with_options(Path::new("board.kicad_pcb"), board, &rules(), options).unwrap()
}

/// R1's hole keepouts on F.Cu: the circles' diameters and the slot paths'
/// widths and lengths, in micrometres.
fn r1_hole_keepouts(bundle: &ExportBundle) -> (Vec<f64>, Vec<(f64, f64)>) {
    let tree = parse_dsn_sexp(&bundle.dsn).unwrap();
    let image = tree
        .find("library")
        .unwrap()
        .find_all("image")
        .into_iter()
        .find(|i| i.get(1).and_then(SexpNode::as_str) == Some("konnect_image_R1"))
        .unwrap();
    let mut circles = Vec::new();
    let mut slots = Vec::new();
    for keepout in image.find_all("keepout") {
        if let Some(circle) = keepout.find("circle") {
            if circle.get(1).and_then(SexpNode::as_str) == Some("F.Cu") {
                circles.push(circle.get_f64(2).unwrap());
            }
        }
        if let Some(path) = keepout.find("path") {
            if path.get(1).and_then(SexpNode::as_str) == Some("F.Cu") {
                let width = path.get_f64(2).unwrap();
                let (x1, y1) = (path.get_f64(3).unwrap(), path.get_f64(4).unwrap());
                let (x2, y2) = (path.get_f64(5).unwrap(), path.get_f64(6).unwrap());
                slots.push((width, ((x2 - x1).powi(2) + (y2 - y1).powi(2)).sqrt()));
            }
        }
    }
    (circles, slots)
}

#[test]
fn unplated_holes_grow_by_the_hole_clearance_less_the_class_clearance() {
    let board = board_with_holes();
    let plain = export_with(&board, None);
    let (circles, slots) = r1_hole_keepouts(&plain);
    assert_eq!(circles, vec![3000.0]);
    assert_eq!(slots, vec![(1000.0, 1000.0)]);

    // 0.25 mm hole clearance over a 0.2 mm class clearance: 50 um a side.
    let grown = export_with(&board, Some(250));
    let (circles, slots) = r1_hole_keepouts(&grown);
    assert_eq!(
        circles,
        vec![3100.0],
        "the 3.0 mm peg keepout becomes 3.1 mm"
    );
    assert_eq!(
        slots,
        vec![(1100.0, 1000.0)],
        "the slot widens by 0.1 mm; its straight run is unchanged"
    );
    let m: serde_json::Value = serde_json::from_str(&grown.manifest).unwrap();
    assert_eq!(m["supported_profile"]["hole_clearance_um"], 250);
    assert_eq!(m["supported_profile"]["hole_keepout_growth_um"], 50);
}

#[test]
fn a_hole_clearance_at_or_below_the_class_clearance_or_unset_changes_nothing() {
    let board = board_with_holes();
    for hole_clearance_um in [None, Some(0), Some(150), Some(200)] {
        let bundle = export_with(&board, hole_clearance_um);
        let (circles, slots) = r1_hole_keepouts(&bundle);
        assert_eq!(circles, vec![3000.0], "{hole_clearance_um:?}");
        assert_eq!(slots, vec![(1000.0, 1000.0)], "{hole_clearance_um:?}");
    }
    let m: serde_json::Value = serde_json::from_str(&export_with(&board, None).manifest).unwrap();
    assert!(m["supported_profile"]
        .get("hole_keepout_growth_um")
        .is_none());
}

#[test]
fn the_strict_importer_accepts_a_manifest_with_the_hole_clearance() {
    let board = board_with_holes();
    let temp = tempfile::tempdir().unwrap();
    let board_path = temp.path().join("board.kicad_pcb");
    std::fs::write(&board_path, &board).unwrap();
    let options = ExportOptions {
        hole_clearance_um: Some(250),
        ..Default::default()
    };
    let bundle = export_dsn_with_options(&board_path, &board, &rules(), options).unwrap();
    let m: serde_json::Value = serde_json::from_str(&bundle.manifest).unwrap();
    let ses = super::layers_tests::ses_for(&m);
    crate::specctra_ses::parse_import_plan(&board_path, &board, &bundle.manifest, &ses).unwrap();
}

#[test]
fn a_rule_area_inside_a_footprint_is_refused_with_the_footprint_named() {
    let board = FOUR_LAYER.replacen(
        HOOK,
        "\t\t(zone (net 0) (net_name \"\") (layers \"F.Cu\") (uuid \"fz1\") (name \"antenna\") (hatch edge 0.5) (connect_pads (clearance 0)) (min_thickness 0.25) (keepout (tracks not_allowed) (vias not_allowed) (pads allowed) (copperpour not_allowed) (footprints allowed)) (fill (thermal_gap 0.5) (thermal_bridge_width 0.5)) (polygon (pts (xy -1 -1) (xy 1 -1) (xy 1 1) (xy -1 1))))\n\t\t(duplicate_pad_numbers_are_jumpers no)",
        1,
    );
    assert_ne!(board, FOUR_LAYER, "fixture shape changed");
    let error = export_dsn(Path::new("board.kicad_pcb"), &board, &rules())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("footprint 'R1'") && error.contains("rule area"),
        "{error}"
    );
}

#[test]
fn an_edge_cuts_slot_inside_a_footprint_is_refused_and_silkscreen_is_not() {
    let line = |layer: &str| {
        FOUR_LAYER.replacen(
            HOOK,
            &format!("\t\t(fp_line (start -0.5 2) (end 0.5 2) (stroke (width 0.1) (type solid)) (layer \"{layer}\") (uuid \"fe1\"))\n{HOOK}"),
            1,
        )
    };
    let error = export_dsn(Path::new("board.kicad_pcb"), &line("Edge.Cuts"), &rules())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("footprint 'R1'") && error.contains("Edge.Cuts"),
        "{error}"
    );
    export_dsn(Path::new("board.kicad_pcb"), &line("F.SilkS"), &rules()).unwrap();
}
