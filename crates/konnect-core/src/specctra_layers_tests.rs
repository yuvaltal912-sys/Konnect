//! PR 16: more than two copper layers and back-side parts.
//!
//! The fixture was written by KiCad 10.0.8's own `pcbnew` module from
//! `specctra_two_resistors.kicad_pcb`: four copper layers (In2.Cu typed
//! `power`), R2 flipped to the back at 90°, a through-hole J1, a locked GND
//! track on In1.Cu, a locked through via and a GND pour on In2.Cu.

use super::*;
use konnect_ipc::IpcRoutingRules;
use serde_json::json;

const FOUR_LAYER: &str = include_str!("../tests/fixtures/specctra_four_layer_back.kicad_pcb");

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

fn export() -> ExportBundle {
    export_dsn(Path::new("board.kicad_pcb"), FOUR_LAYER, &rules()).unwrap()
}

fn manifest(bundle: &ExportBundle) -> serde_json::Value {
    serde_json::from_str(&bundle.manifest).unwrap()
}

/// Where the router puts each pin: the image pin rotated counter-clockwise
/// (Specctra's Y-up frame) by the place rotation, plus the place position,
/// in DSN micrometres. Computed from the DSN alone.
fn router_pin_positions(dsn: &str) -> BTreeMap<String, (f64, f64, String)> {
    let identity = dsn_identity(&parse_dsn_sexp(dsn).unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for (reference, place) in &identity.components {
        assert_eq!(
            place.side, "front",
            "{reference}: the DSN never mirrors a part"
        );
        let theta = f64::from_bits(place.rotation.0).to_radians();
        for (pin, geometry) in &place.pins {
            let (lx, ly) = (geometry.x as f64, geometry.y as f64);
            let x = place.x as f64 + lx * theta.cos() - ly * theta.sin();
            let y = place.y as f64 + lx * theta.sin() + ly * theta.cos();
            let layers = identity.padstacks[&geometry.padstack]
                .iter()
                .map(|shape| match shape {
                    DsnShapeIdentity::Circle { layer, .. }
                    | DsnShapeIdentity::Rect { layer, .. } => layer.clone(),
                })
                .collect::<Vec<_>>()
                .join(",");
            out.insert(format!("{reference}-{pin}"), (x, y, layers));
        }
    }
    out
}

#[test]
fn a_four_layer_board_keeps_its_stack_order_and_layer_types() {
    let bundle = export();
    let m = manifest(&bundle);
    assert_eq!(m["supported_profile"]["copper_layers"], 4);
    let layers: Vec<(String, String)> = m["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["kicad_name"].as_str().unwrap().to_string(),
                l["dsn_type"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        layers,
        [
            ("F.Cu", "signal"),
            ("In1.Cu", "signal"),
            ("In2.Cu", "power"),
            ("B.Cu", "signal")
        ]
        .map(|(a, b)| (a.to_string(), b.to_string()))
    );
    // The file lists B.Cu second; the DSN indexes the stack top to bottom.
    let identity = dsn_identity(&parse_dsn_sexp(&bundle.dsn).unwrap()).unwrap();
    assert_eq!(
        identity.layers.values().cloned().collect::<Vec<_>>(),
        ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]
    );
    assert!(bundle.dsn.contains("(type power)"), "{}", bundle.dsn);
}

#[test]
fn a_back_side_part_reaches_the_router_at_kicads_own_pad_positions() {
    let bundle = export();
    let pins = router_pin_positions(&bundle.dsn);
    // KiCad 10.0.8 pcbnew, PAD::GetPosition() on the saved fixture (mm, Y down).
    let kicad: [(&str, f64, f64, &str); 6] = [
        ("R1-1", 99.5, 50.0, "F.Cu"),
        ("R1-2", 100.5, 50.0, "F.Cu"),
        ("R2-1", 110.0, 49.5, "B.Cu"),
        ("R2-2", 110.0, 50.5, "B.Cu"),
        ("J1-1", 103.73, 56.0, "F.Cu,In1.Cu,In2.Cu,B.Cu"),
        ("J1-2", 106.27, 56.0, "F.Cu,In1.Cu,In2.Cu,B.Cu"),
    ];
    for (pin, x_mm, y_mm, layers) in kicad {
        let (x, y, got_layers) = &pins[pin];
        assert!(
            (x - x_mm * 1000.0).abs() < 0.5 && (y + y_mm * 1000.0).abs() < 0.5,
            "{pin}: router at ({x}, {y}) um, KiCad at ({x_mm}, {y_mm}) mm"
        );
        assert_eq!(got_layers, layers, "{pin}");
    }
    let m = manifest(&bundle);
    assert_eq!(m["supported_profile"]["component_side"], "front_and_back");
    let r2 = m["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["reference"] == "R2")
        .unwrap();
    assert_eq!(r2["kicad_side"], "back");
    assert_eq!(r2["side"], "front");
}

#[test]
fn locked_inner_copper_and_an_inner_pour_are_fixed_routing() {
    let bundle = export();
    let m = manifest(&bundle);
    assert_eq!(m["supported_profile"]["locked_track_count"], 1);
    assert_eq!(m["supported_profile"]["locked_via_count"], 1);
    assert_eq!(m["supported_profile"]["zone_plane_count"], 1);
    assert!(bundle.dsn.contains("(path In1.Cu"), "{}", bundle.dsn);
    // The via padstack covers the whole stack.
    let via = m["padstacks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["purpose"] == "via")
        .unwrap();
    assert_eq!(via["layers"], json!(["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]));
}

#[test]
fn two_layer_boards_export_exactly_as_before() {
    // The DSN of the original fixture is unchanged by this PR; only the
    // manifest gained `dsn_type` and `kicad_side`.
    let source = include_str!("../tests/fixtures/specctra_two_resistors.kicad_pcb");
    let bundle = export_dsn(Path::new("board.kicad_pcb"), source, &rules()).unwrap();
    let identity = dsn_identity(&parse_dsn_sexp(&bundle.dsn).unwrap()).unwrap();
    assert_eq!(
        identity.layers.values().cloned().collect::<Vec<_>>(),
        ["F.Cu", "B.Cu"]
    );
    assert!(!bundle.dsn.contains("(type power)"));
    let m = manifest(&bundle);
    assert_eq!(m["supported_profile"]["component_side"], "front");
    assert_eq!(m["supported_profile"]["copper_layers"], 2);
}

/// An SES as Freerouting writes one: the placement echoed, and routes on an
/// inner layer and on B.Cu to the back-side part, with one through via.
fn ses_for(m: &serde_json::Value) -> String {
    let mut placement = String::new();
    for c in m["components"].as_array().unwrap() {
        placement.push_str(&format!(
            "    (component {}\n      (place {} {} {} front {})\n    )\n",
            c["image_name"].as_str().unwrap(),
            c["reference"].as_str().unwrap(),
            c["x_um"].as_i64().unwrap() * 10,
            c["y_um"].as_i64().unwrap() * 10,
            c["rotation_degrees"].as_f64().unwrap()
        ));
    }
    let via = m["padstacks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["purpose"] == "via")
        .unwrap();
    let mut shapes = String::new();
    for layer in via["layers"].as_array().unwrap() {
        shapes.push_str(&format!(
            "        (shape\n          (circle {} 6000 0 0)\n        )\n",
            layer.as_str().unwrap()
        ));
    }
    format!(
        "(session \"board\"\n  (base_design \"board\")\n  (placement\n    (resolution um 10)\n{placement}  )\n  (was_is\n  )\n  (routes \n    (resolution um 10)\n    (parser\n      (host_cad \"KiCad's Pcbnew\")\n      (host_version \"10.0.8\")\n    )\n    (library_out \n      (padstack \"{via}\"\n{shapes}        (attach off)\n      )\n    )\n    (network_out \n      (net GND\n        (wire\n          (path In1.Cu 2500\n            1037300 -560000\n            1037300 -530000\n          )\n        )\n        (wire\n          (path B.Cu 2500\n            1037300 -530000\n            1100000 -530000\n            1100000 -505000\n          )\n        )\n        (via \"{via}\" 1037300 -530000\n        )\n      )\n    )\n  )\n)\n",
        via = via["name"].as_str().unwrap()
    )
}

#[test]
fn an_ses_with_inner_layer_and_back_side_routes_passes_the_strict_planner() {
    let temp = tempfile::tempdir().unwrap();
    let board_path = temp.path().join("board.kicad_pcb");
    std::fs::write(&board_path, FOUR_LAYER).unwrap();
    let bundle = export_dsn(&board_path, FOUR_LAYER, &rules()).unwrap();
    let m = manifest(&bundle);
    let ses = ses_for(&m);
    let plan =
        crate::specctra_ses::parse_import_plan(&board_path, FOUR_LAYER, &bundle.manifest, &ses)
            .unwrap();
    let layers: BTreeSet<&str> = plan.tracks.iter().map(|t| t.layer.as_str()).collect();
    assert_eq!(layers, BTreeSet::from(["B.Cu", "In1.Cu"]));
    assert_eq!(plan.vias.len(), 1);

    // A route on a layer the board does not have is refused.
    let bad = ses.replace("(path In1.Cu", "(path In3.Cu");
    let error =
        crate::specctra_ses::parse_import_plan(&board_path, FOUR_LAYER, &bundle.manifest, &bad)
            .unwrap_err()
            .to_string();
    assert!(error.contains("unknown layer"), "{error}");
}

#[test]
fn a_manifest_that_misstates_the_stack_or_sides_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let board_path = temp.path().join("board.kicad_pcb");
    std::fs::write(&board_path, FOUR_LAYER).unwrap();
    let bundle = export_dsn(&board_path, FOUR_LAYER, &rules()).unwrap();
    let ses = ses_for(&manifest(&bundle));
    let refuse = |edit: &dyn Fn(&mut serde_json::Value), needle: &str| {
        let mut m = manifest(&bundle);
        edit(&mut m);
        let error =
            crate::specctra_ses::parse_import_plan(&board_path, FOUR_LAYER, &m.to_string(), &ses)
                .unwrap_err()
                .to_string();
        assert!(error.contains(needle), "{needle}: {error}");
    };
    refuse(
        &|m| m["supported_profile"]["copper_layers"] = json!(2),
        "one mapping per copper layer",
    );
    refuse(
        &|m| m["supported_profile"]["copper_layers"] = json!(3),
        "supported routing profile",
    );
    refuse(
        &|m| m["supported_profile"]["component_side"] = json!("front"),
        "component sides",
    );
    refuse(&|m| m["layers"][2]["index"] = json!(7), "contiguous");
    refuse(
        &|m| {
            for c in m["components"].as_array_mut().unwrap() {
                if c["reference"] == "R2" {
                    c["kicad_side"] = json!("sideways");
                }
            }
        },
        "component placement metadata",
    );
}

#[test]
fn kicads_native_export_is_not_adopted_for_a_back_side_part() {
    // Any native text will do: adoption refuses before correlating it.
    let native = include_str!("../tests/fixtures/specctra_two_resistors.native-kicad-10.dsn");
    let error = adopt_native_dsn(export(), native.to_string())
        .unwrap_err()
        .to_string();
    assert!(error.contains("back-side part"), "{error}");
}

#[test]
fn stacks_the_router_cannot_take_are_still_refused() {
    let refuse = |board: String, needle: &str| {
        let error = export_dsn(Path::new("board.kicad_pcb"), &board, &rules())
            .unwrap_err()
            .to_string();
        assert!(error.contains(needle), "{needle}: {error}");
    };
    // A gap in the inner numbering.
    refuse(
        FOUR_LAYER.replace("(6 \"In2.Cu\" power)", "(8 \"In3.Cu\" power)"),
        "without gaps",
    );
    // An odd count.
    refuse(
        FOUR_LAYER.replace("\t\t(6 \"In2.Cu\" power)\n", ""),
        "even count",
    );
    // A jumper layer.
    refuse(
        FOUR_LAYER.replace("(6 \"In2.Cu\" power)", "(6 \"In2.Cu\" jumper)"),
        "type 'jumper'",
    );
    // A through via KiCad would write as a blind span.
    refuse(
        FOUR_LAYER.replace(
            "(layers \"F.Cu\" \"B.Cu\")\n\t\t(locked yes)",
            "(layers \"F.Cu\" \"In1.Cu\")\n\t\t(locked yes)",
        ),
        "through span",
    );
    // An SMD pad on two copper layers.
    let two_layer_smd = FOUR_LAYER.replacen(
        "(layers \"B.Cu\" \"B.Mask\" \"B.Paste\")",
        "(layers \"F.Cu\" \"B.Cu\" \"B.Mask\")",
        1,
    );
    refuse(two_layer_smd, "not one outer layer");
    // A copper dimension has no keepout form yet.
    let dimension = FOUR_LAYER.replacen(
        "\t(via\n",
        "\t(dimension (type aligned) (layer \"F.Cu\") (uuid \"d1\") (pts (xy 98 47) (xy 104 47)))\n\t(via\n",
        1,
    );
    refuse(dimension, "'dimension' on a copper layer");
}

/// Every pad's orientation as the router sees it (place rotation plus pin
/// rotation) against KiCad's `PAD::GetOrientationDegrees()`. Before PR 16 the
/// pin rotation was the board-space angle, so a part turned 90° had its
/// rectangular pads turned twice.
#[test]
fn pad_orientation_reaches_the_router_as_kicad_has_it() {
    let bundle = export();
    let identity = dsn_identity(&parse_dsn_sexp(&bundle.dsn).unwrap()).unwrap();
    let kicad: [(&str, &str, f64); 4] = [
        ("R1", "1", 0.0),
        ("R2", "1", 270.0),
        ("R2", "2", 270.0),
        ("J1", "1", 0.0),
    ];
    for (reference, pin, expected) in kicad {
        let place = &identity.components[reference];
        let pin_rotation = place.pins[pin]
            .rotation
            .map_or(0.0, |r| f64::from_bits(r.0));
        let total = normalized_degrees(f64::from_bits(place.rotation.0) + pin_rotation);
        // A rectangle is the same at θ and θ+180.
        assert!(
            (total - expected)
                .rem_euclid(180.0)
                .min(180.0 - (total - expected).rem_euclid(180.0))
                < 1e-6,
            "{reference}-{pin}: router {total}°, KiCad {expected}°"
        );
    }
}

/// R1 turned 90° in place: its pads' board angle is 90, their angle in the
/// image is 0, so the DSN writes no pin rotation.
#[test]
fn a_rotated_part_writes_pad_angles_relative_to_the_part() {
    let rotated = FOUR_LAYER
        .replacen("(at 100 50)\n", "(at 100 50 90)\n", 1)
        .replacen(
            "(at -0.5 0)\n\t\t\t(size 0.6 0.5)",
            "(at -0.5 0 90)\n\t\t\t(size 0.6 0.5)",
            1,
        )
        .replacen(
            "(at 0.5 0)\n\t\t\t(size 0.6 0.5)",
            "(at 0.5 0 90)\n\t\t\t(size 0.6 0.5)",
            1,
        );
    assert_ne!(rotated, FOUR_LAYER);
    let bundle = export_dsn(Path::new("board.kicad_pcb"), &rotated, &rules()).unwrap();
    let identity = dsn_identity(&parse_dsn_sexp(&bundle.dsn).unwrap()).unwrap();
    let r1 = &identity.components["R1"];
    assert_eq!(f64::from_bits(r1.rotation.0), 90.0);
    assert!(r1.pins.values().all(|p| p.rotation.is_none()), "{r1:?}");
}

#[test]
fn repeated_and_unnumbered_pads_get_unique_pins_and_keep_their_kicad_number() {
    // J1's two pads both become "MP"; add an unnumbered copper pad too.
    let board = FOUR_LAYER
        .replacen("(pad \"1\" thru_hole circle", "(pad \"MP\" thru_hole circle", 1)
        .replacen("(pad \"2\" thru_hole circle", "(pad \"MP\" thru_hole circle", 1)
        .replacen(
            "(pad \"MP\" thru_hole circle",
            "(pad \"\" smd rect (at 0 2) (size 1 1) (layers \"F.Cu\") (uuid \"fid\"))\n\t\t(pad \"MP\" thru_hole circle",
            1,
        );
    let bundle = export_dsn(Path::new("board.kicad_pcb"), &board, &rules()).unwrap();
    let m = manifest(&bundle);
    let j1 = m["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["reference"] == "J1")
        .unwrap();
    let pins: Vec<(String, String)> = j1["pads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["pad_number"].as_str().unwrap().to_string(),
                p["dsn_pin"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        pins,
        [("", "J1-@1"), ("MP", "J1-MP@1"), ("MP", "J1-MP@2")]
            .map(|(a, b)| (a.to_string(), b.to_string()))
    );
    // Both mounting pads stay on their nets; the unnumbered one is on none.
    let nets = &m["nets"];
    let gnd = nets
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["name"] == "GND")
        .unwrap();
    assert!(gnd["pins"].as_array().unwrap().contains(&json!("J1-MP@1")));
    // The strict importer accepts the manifest.
    let temp = tempfile::tempdir().unwrap();
    let board_path = temp.path().join("board.kicad_pcb");
    std::fs::write(&board_path, &board).unwrap();
    let bundle = export_dsn(&board_path, &board, &rules()).unwrap();
    let ses = ses_for(&manifest(&bundle));
    crate::specctra_ses::parse_import_plan(&board_path, &board, &bundle.manifest, &ses).unwrap();
}

#[test]
fn an_unplated_hole_is_a_keepout_on_every_copper_layer_and_a_slot_is_a_stadium() {
    let board = FOUR_LAYER.replacen(
        "\t\t(duplicate_pad_numbers_are_jumpers no)",
        "\t\t(pad \"\" np_thru_hole circle (at 0 3) (size 3.2 3.2) (drill 3.2) (layers \"*.Cu\" \"*.Mask\") (uuid \"h1\"))\n\t\t(pad \"\" np_thru_hole oval (at 4 3) (size 1 2) (drill oval 1 2) (layers \"*.Cu\" \"*.Mask\") (uuid \"h2\"))\n\t\t(duplicate_pad_numbers_are_jumpers no)",
        1,
    );
    let bundle = export_dsn(Path::new("board.kicad_pcb"), &board, &rules()).unwrap();
    let tree = parse_dsn_sexp(&bundle.dsn).unwrap();
    let image = tree
        .find("library")
        .unwrap()
        .find_all("image")
        .into_iter()
        .find(|i| i.get(1).and_then(SexpNode::as_str) == Some("konnect_image_R1"))
        .unwrap();
    let keepouts = image.find_all("keepout");
    assert_eq!(keepouts.len(), 8, "two holes on four layers: {image:?}");
    let circles = keepouts
        .iter()
        .filter(|k| k.find("circle").is_some())
        .count();
    let paths = keepouts.iter().filter(|k| k.find("path").is_some()).count();
    assert_eq!((circles, paths), (4, 4));
    // A hole is not a pin.
    let m = manifest(&bundle);
    let r1 = m["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["reference"] == "R1")
        .unwrap();
    assert_eq!(r1["pads"].as_array().unwrap().len(), 2);
}

#[test]
fn slots_connect_pads_and_padless_footprints_are_accepted() {
    // A plated slot on J1 pad 1, a `connect` pad (an edge finger) on R1,
    // and a padless logo footprint.
    let board = FOUR_LAYER
        .replacen("(drill 1)", "(drill oval 1 1.2)", 1)
        .replacen("(pad \"1\" smd rect", "(pad \"1\" connect rect", 1)
        .replacen(
            "\t(via\n",
            "\t(footprint \"Logo\" (layer \"F.Cu\") (uuid \"logo\") (at 104 44) (property \"Reference\" \"G1\" (at 0 0 0) (layer \"F.SilkS\") (uuid \"lg\") (effects (font (size 1 1)))))\n\t(via\n",
            1,
        );
    assert!(
        board.contains("(drill oval 1 1.2)"),
        "fixture shape changed"
    );
    let bundle = export_dsn(Path::new("board.kicad_pcb"), &board, &rules()).unwrap();
    let m = manifest(&bundle);
    assert_eq!(m["supported_profile"]["footprints_without_pads"], 1);
    assert!(m["components"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["reference"] != "G1"));
    // A slot wider than its copper is still refused.
    let error = export_dsn(
        Path::new("board.kicad_pcb"),
        &FOUR_LAYER.replacen("(drill 1)", "(drill oval 1 1.8)", 1),
        &rules(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("not smaller than its copper"), "{error}");
}

/// Acceptance probe over a folder of boards (KiCad's demos with their routing
/// stripped): `KONNECT_DSN_PROBE_DIR=<dir> cargo test -p konnect-core
/// probe_dsn_export_over_a_folder -- --ignored --nocapture`. Prints one line
/// per board; asserts nothing, since which boards fit is the finding.
#[test]
#[ignore = "needs KONNECT_DSN_PROBE_DIR"]
fn probe_dsn_export_over_a_folder() {
    let dir = std::env::var("KONNECT_DSN_PROBE_DIR").expect("set KONNECT_DSN_PROBE_DIR");
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "kicad_pcb"))
        .collect();
    paths.sort();
    for path in paths {
        let source = std::fs::read_to_string(&path).unwrap();
        let tree = parse_sexp(&source).unwrap();
        let table = top_level_net_table(&tree);
        let mut rules = IpcEffectiveRoutingRules::new();
        for fp in tree.find_all("footprint") {
            for pad in fp.find_all("pad") {
                if let Some(net) = pad.find("net").and_then(|n| resolve_net(n, &table)) {
                    rules.insert(
                        net,
                        IpcRoutingRules {
                            class_name: "Default".to_string(),
                            constituents: vec!["Default".to_string()],
                            track_width_mm: Some(0.2),
                            clearance_mm: Some(0.2),
                            via_diameter_mm: Some(0.6),
                            via_drill_mm: Some(0.3),
                        },
                    );
                }
            }
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        match export_dsn(&path, &source, &rules) {
            Ok(bundle) => {
                let m = manifest(&bundle);
                let back = m["components"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|c| c["kicad_side"] == "back")
                    .count();
                println!(
                    "PROBE {name}: exported, {} copper layers, {} parts ({back} on the back), {} nets",
                    m["supported_profile"]["copper_layers"], bundle.component_count, bundle.net_count
                );
            }
            Err(error) => {
                let reason = format!("{error:#}");
                println!(
                    "PROBE {name}: refused: {}",
                    reason.lines().next().unwrap_or("")
                );
            }
        }
    }
}

/// Copper text and graphics on the board and in footprints reach the router
/// as keepouts that cover them, instead of being invisible to it.
#[test]
fn copper_text_and_graphics_become_keepouts_that_cover_them() {
    let board = FOUR_LAYER
        .replacen(
            "\t(via\n",
            "\t(gr_text \"LOGO\" (at 104 47 30) (layer \"F.Cu\") (uuid \"t1\") (effects (font (size 1 1) (thickness 0.15))))\n\t(gr_line (start 96 47) (end 99 47) (stroke (width 0.3) (type solid)) (layer \"B.Cu\") (uuid \"g1\"))\n\t(via\n",
            1,
        )
        .replacen(
            "\t\t(duplicate_pad_numbers_are_jumpers no)",
            "\t\t(fp_poly (pts (xy -1 -1) (xy 1 -1) (xy 1 1)) (stroke (width 0.1) (type solid)) (fill yes) (layer \"F.Cu\") (uuid \"p9\"))\n\t\t(duplicate_pad_numbers_are_jumpers no)",
            1,
        );
    let bundle = export_dsn(Path::new("board.kicad_pcb"), &board, &rules()).unwrap();
    let m = manifest(&bundle);
    // Text and line on the board, polygon in R1: three keepouts, one layer each.
    assert_eq!(m["supported_profile"]["copper_graphic_keepout_count"], 3);
    let tree = parse_dsn_sexp(&bundle.dsn).unwrap();
    let structure = tree.find("structure").unwrap();
    let keepouts = structure.find_all("keepout");
    // The In2.Cu pour is a plane, not a keepout: these two are the graphics.
    assert_eq!(keepouts.len(), 2, "{structure:?}");
    // The text square covers a 4-character, 1 mm glyph box at any angle: it
    // reaches past 104 ± 5 mm and 47 ± 6 mm (half-width 4·1.25+0.15).
    let text = keepouts
        .iter()
        .find_map(|k| k.find("polygon"))
        .expect("text keepout is a polygon");
    let xs: Vec<f64> = text
        .children()
        .unwrap()
        .iter()
        .skip(3)
        .filter_map(|n| n.as_str()?.parse().ok())
        .step_by(2)
        .collect();
    let (min_x, max_x) = xs
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| {
            (a.min(*x), b.max(*x))
        });
    assert!(min_x <= 99_000.0 && max_x >= 109_000.0, "{text:?}");
    // The line keeps its stroke plus the chord tolerance.
    let line = keepouts.iter().find_map(|k| k.find("path")).unwrap();
    assert_eq!(line.get(1).and_then(SexpNode::as_str), Some("B.Cu"));
    assert_eq!(line.get(2).and_then(SexpNode::as_str), Some("310"));
    // The footprint polygon is an image keepout of R1.
    let image = tree
        .find("library")
        .unwrap()
        .find_all("image")
        .into_iter()
        .find(|i| i.get(1).and_then(SexpNode::as_str) == Some("konnect_image_R1"))
        .unwrap();
    assert_eq!(image.find_all("keepout").len(), 1);
    // Hidden copper text draws nothing and is not a keepout.
    let hidden = board.replacen(
        "(effects (font (size 1 1) (thickness 0.15))))",
        "(effects (font (size 1 1) (thickness 0.15)) (hide yes)))",
        1,
    );
    let m = manifest(&export_dsn(Path::new("board.kicad_pcb"), &hidden, &rules()).unwrap());
    assert_eq!(m["supported_profile"]["copper_graphic_keepout_count"], 2);
}

/// KiCad's `(drill d (offset x y))` moves a pad's copper off its hole. The
/// padstack carries it, so the router sees the copper where it is.
#[test]
fn a_drill_offset_moves_the_padstack_copper() {
    let board = FOUR_LAYER.replacen("(drill 1)", "(drill 1 (offset 0.3 -0.2))", 1);
    assert_ne!(board, FOUR_LAYER);
    let bundle = export_dsn(Path::new("board.kicad_pcb"), &board, &rules()).unwrap();
    let identity = dsn_identity(&parse_dsn_sexp(&bundle.dsn).unwrap()).unwrap();
    let pin = &identity.components["J1"].pins["1"];
    // The pin stays on the hole; its circle is offset (Y up, so -0.2 mm is +200).
    assert_eq!((pin.x, pin.y), (-1_270, 0));
    let tree = parse_dsn_sexp(&bundle.dsn).unwrap();
    let padstack = tree
        .find("library")
        .unwrap()
        .find_all("padstack")
        .into_iter()
        .find(|p| p.get(1).and_then(SexpNode::as_str) == Some(pin.padstack.as_str()))
        .unwrap();
    let circle = padstack.find("shape").unwrap().find("circle").unwrap();
    let values: Vec<&str> = circle
        .children()
        .unwrap()
        .iter()
        .skip(1)
        .filter_map(SexpNode::as_str)
        .collect();
    assert_eq!(values, ["F.Cu", "1700", "300", "200"], "{padstack:?}");
}

#[test]
fn copperless_pads_are_skipped_and_ringless_unconnected_plated_holes_are_holes() {
    let board = FOUR_LAYER.replacen(
        "\t\t(duplicate_pad_numbers_are_jumpers no)",
        "\t\t(pad \"\" smd rect (at 0 2) (size 1 1) (layers \"F.Paste\") (uuid \"ap\"))\n\t\t(pad \"\" thru_hole circle (at 0 4) (size 2 2) (drill 2) (layers \"*.Cu\" \"*.Mask\") (uuid \"mh\"))\n\t\t(duplicate_pad_numbers_are_jumpers no)",
        1,
    );
    let bundle = export_dsn(Path::new("board.kicad_pcb"), &board, &rules()).unwrap();
    let m = manifest(&bundle);
    let r1 = m["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["reference"] == "R1")
        .unwrap();
    assert_eq!(r1["pads"].as_array().unwrap().len(), 2, "{r1}");
    let tree = parse_dsn_sexp(&bundle.dsn).unwrap();
    let image = tree
        .find("library")
        .unwrap()
        .find_all("image")
        .into_iter()
        .find(|i| i.get(1).and_then(SexpNode::as_str) == Some("konnect_image_R1"))
        .unwrap();
    assert_eq!(
        image.find_all("keepout").len(),
        4,
        "the hole on four layers"
    );
}

#[test]
fn a_board_name_with_a_space_is_a_valid_dsn_and_its_ses_imports() {
    let temp = tempfile::tempdir().unwrap();
    let board_path = temp.path().join("sonde xilinx.kicad_pcb");
    std::fs::write(&board_path, FOUR_LAYER).unwrap();
    let bundle = export_dsn(&board_path, FOUR_LAYER, &rules()).unwrap();
    assert!(
        bundle.dsn.starts_with("(pcb sonde_xilinx.kicad_pcb"),
        "{}",
        &bundle.dsn[..60]
    );
    // Freerouting echoes the DSN name as the session's base design.
    let ses = ses_for(&manifest(&bundle))
        .replace("(session \"board\"", "(session \"sonde_xilinx\"")
        .replace("(base_design \"board\")", "(base_design \"sonde_xilinx\")");
    crate::specctra_ses::parse_import_plan(&board_path, FOUR_LAYER, &bundle.manifest, &ses)
        .unwrap();
}
