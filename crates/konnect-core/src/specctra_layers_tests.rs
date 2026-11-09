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
    // Read straight from the DSN tree, independent of the exporter's own
    // identity parser: placements, image pins and padstack layers.
    let tree = parse_dsn_sexp(dsn).unwrap();
    let num = |n: &SexpNode, i: usize| -> f64 {
        n.get(i)
            .and_then(SexpNode::as_str)
            .unwrap()
            .parse()
            .unwrap()
    };
    let library = tree.find("library").unwrap();
    let mut padstack_layers: BTreeMap<String, String> = BTreeMap::new();
    for padstack in library.find_all("padstack") {
        let name = padstack
            .get(1)
            .and_then(SexpNode::as_str)
            .unwrap()
            .to_string();
        let layers: Vec<String> = padstack
            .find_all("shape")
            .into_iter()
            .filter_map(|shape| {
                shape
                    .children()?
                    .get(1)?
                    .get(1)?
                    .as_str()
                    .map(str::to_string)
            })
            .collect();
        padstack_layers.insert(name, layers.join(","));
    }
    let mut images: BTreeMap<String, Vec<(String, String, f64, f64)>> = BTreeMap::new();
    for image in library.find_all("image") {
        let name = image.get(1).and_then(SexpNode::as_str).unwrap().to_string();
        let pins = image
            .find_all("pin")
            .into_iter()
            .map(|pin| {
                let data: Vec<&SexpNode> = pin
                    .children()
                    .unwrap()
                    .iter()
                    .skip(1)
                    .filter(|c| c.head() != Some("rotate"))
                    .collect();
                (
                    data[0].as_str().unwrap().to_string(),
                    data[1].as_str().unwrap().to_string(),
                    data[2].as_str().unwrap().parse().unwrap(),
                    data[3].as_str().unwrap().parse().unwrap(),
                )
            })
            .collect();
        images.insert(name, pins);
    }
    let mut out = BTreeMap::new();
    for component in tree.find("placement").unwrap().find_all("component") {
        let image = component.get(1).and_then(SexpNode::as_str).unwrap();
        for place in component.find_all("place") {
            let reference = place.get(1).and_then(SexpNode::as_str).unwrap();
            assert_eq!(
                place.get(4).and_then(SexpNode::as_str),
                Some("front"),
                "{reference}: the DSN never mirrors a part"
            );
            let (px, py, theta) = (num(place, 2), num(place, 3), num(place, 5).to_radians());
            for (padstack, pin, lx, ly) in &images[image] {
                let x = px + lx * theta.cos() - ly * theta.sin();
                let y = py + lx * theta.sin() + ly * theta.cos();
                out.insert(
                    format!("{reference}-{pin}"),
                    (x, y, padstack_layers[padstack].clone()),
                );
            }
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
pub(super) fn ses_for(m: &serde_json::Value) -> String {
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
    // The text keepout is a rectangle at the text's 30°: it covers a
    // 4-character, 1 mm line (true glyphs reach about ±1.8 mm along it and
    // ±0.5 mm across) and stays near that size instead of a square for every
    // rotation (the old form was 67 mm wide on a demo board and blocked it).
    let text = keepouts
        .iter()
        .find_map(|k| k.find("polygon"))
        .expect("text keepout is a polygon");
    let v: Vec<f64> = text
        .children()
        .unwrap()
        .iter()
        .skip(3)
        .filter_map(|n| n.as_str()?.parse().ok())
        .collect();
    let corners: Vec<(f64, f64)> = v
        .chunks(2)
        .map(|p| (p[0] / 1000.0, -p[1] / 1000.0))
        .collect();
    // Back into the text's own frame (Y down, 30° CCW on screen).
    let (c, s) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
    let local: Vec<(f64, f64)> = corners
        .iter()
        .map(|(x, y)| {
            let (dx, dy) = (x - 104.0, y - 47.0);
            (dx * c - dy * s, dx * s + dy * c)
        })
        .collect();
    let along = local
        .iter()
        .map(|p| p.0)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| {
            (a.min(x), b.max(x))
        });
    let across = local
        .iter()
        .map(|p| p.1)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), y| {
            (a.min(y), b.max(y))
        });
    assert!(along.0 <= -1.8 && along.1 >= 1.8, "{along:?}");
    assert!(across.0 <= -0.5 && across.1 >= 0.5, "{across:?}");
    assert!(
        along.1 - along.0 < 6.0 && across.1 - across.0 < 4.0,
        "{along:?} {across:?}"
    );
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

/// The retry width `route_specctra_dsn` would pass, from manifest text.
fn manifest_retry_width(manifest_text: &str) -> Option<i64> {
    let temp = tempfile::tempdir().ok()?;
    let dsn = temp.path().join("x.dsn");
    std::fs::write(temp.path().join("x.dsn.konnect.json"), manifest_text).ok()?;
    manifest_routing_hints(&dsn).retry_width_um
}

/// Live acceptance: export each board in a folder, route it with
/// Freerouting, and run the SES through the strict import planner.
/// `KONNECT_DSN_PROBE_DIR=<dir> FREEROUTING_JAR=<jar> FREEROUTING_JAVA=<java>
/// cargo test -p konnect-core freerouting_routes_a_folder -- --ignored --nocapture`.
/// Each Freerouting run is capped at `FREEROUTING_SECONDS` (default 600).
#[test]
#[ignore = "needs KONNECT_DSN_PROBE_DIR, FREEROUTING_JAR and Java 21"]
fn freerouting_routes_a_folder() {
    let dir = std::path::PathBuf::from(std::env::var("KONNECT_DSN_PROBE_DIR").expect("dir"));
    let jar = std::env::var("FREEROUTING_JAR").expect("FREEROUTING_JAR");
    let java = std::env::var("FREEROUTING_JAVA").unwrap_or_else(|_| "java".to_string());
    let cap: u64 = std::env::var("FREEROUTING_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "kicad_pcb"))
        .collect();
    paths.sort();
    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
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
        // As export_specctra_dsn does: the project's rules and revision,
        // intent widths, and the copper-to-edge and hole clearances.
        let project_um = |key: &str| {
            std::fs::read_to_string(path.with_extension("kicad_pro"))
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .and_then(|p| p["board"]["design_settings"]["rules"][key].as_f64())
                .filter(|mm| mm.is_finite() && *mm > 0.0)
                .map(|mm| (mm * 1000.0).round() as i64)
        };
        let options = ExportOptions {
            omit_pours: std::env::var_os("KONNECT_PROBE_OMIT_POURS").is_some(),
            copper_edge_clearance_um: project_um("min_copper_edge_clearance"),
            widths_from_intent: true,
            hole_clearance_um: project_um("min_hole_clearance"),
        };
        let Ok(snapshot) = crate::routing_context::Snapshot::read(&path) else {
            println!("ROUTE {name}: project rules could not be read");
            continue;
        };
        let exported = export_dsn_with_context(&path, &source, &rules, &snapshot, options);
        if let Ok(bundle) = &exported {
            for w in bundle
                .intent_widths
                .iter()
                .filter(|w| w["status"] == "widened")
            {
                println!(
                    "WIDEN {name}: {} {} -> {} mm",
                    w["net"], w["from_mm"], w["to_mm"]
                );
            }
        }
        let Ok(bundle) = exported else {
            continue; // the export probe reports refusals
        };
        let work = tempfile::tempdir().unwrap();
        // `KONNECT_PROBE_KEEP=<dir>` keeps each DSN and SES for inspection.
        let out_dir = std::env::var_os("KONNECT_PROBE_KEEP")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| work.path().to_path_buf());
        let dsn_path = out_dir.join(dsn_design_name(&name).replace(".kicad_pcb", ".dsn"));
        let ses_path = dsn_path.with_extension("ses");
        std::fs::write(&dsn_path, &bundle.dsn).unwrap();
        let started = std::time::Instant::now();
        let mut child = std::process::Command::new(&java)
            .arg("-jar")
            .arg(&jar)
            .arg("-de")
            .arg(&dsn_path)
            .arg("-do")
            .arg(&ses_path)
            .arg("-mp")
            .arg("20")
            .arg("--router.automatic_neckdown=false")
            .args(
                manifest_retry_width(&bundle.manifest)
                    .map(|um| format!("--router.neck_width_um={um}")),
            )
            .args(
                options
                    .copper_edge_clearance_um
                    .map(|um| format!("--router.copper_to_edge_clearance_um={um}")),
            )
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("launch Freerouting");
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status);
            }
            if started.elapsed().as_secs() > cap {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        };
        let output = child.wait_with_output().ok();
        let log = output
            .map(|o| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                )
            })
            .unwrap_or_default();
        let unrouted = log
            .lines()
            .rev()
            .find(|l| {
                l.to_lowercase().contains("unrouted") || l.to_lowercase().contains("incomplete")
            })
            .unwrap_or("")
            .trim()
            .to_string();
        let secs = started.elapsed().as_secs();
        let Some(status) = status else {
            println!("ROUTE {name}: Freerouting stopped after the {cap}s cap");
            continue;
        };
        let Ok(ses) = std::fs::read_to_string(&ses_path) else {
            println!(
                "ROUTE {name}: Freerouting exited {status} in {secs}s without an SES; last log: {}",
                log.lines().last().unwrap_or("")
            );
            continue;
        };
        match crate::specctra_ses::parse_import_plan(&path, &source, &bundle.manifest, &ses) {
            Ok(plan) => println!(
                "ROUTE {name}: routed in {secs}s, import plan OK: {} tracks, {} arcs, {} vias; {unrouted}",
                plan.tracks.len(), plan.arcs.len(), plan.vias.len()
            ),
            Err(error) => println!("ROUTE {name}: SES refused by the import planner: {error:#}"),
        }
    }
}

/// Freerouting 2.5.0 echoed KiCad's `-90` placement as `270`, and the
/// importer refused every such board as moved. The export now writes angles
/// in [0, 360), and the importer compares directions, so a manifest written
/// with `-90` still matches an SES that says `270`.
#[test]
fn a_negative_placement_angle_survives_freeroutings_normalisation() {
    let temp = tempfile::tempdir().unwrap();
    let board_path = temp.path().join("board.kicad_pcb");
    std::fs::write(&board_path, FOUR_LAYER).unwrap();
    let bundle = export_dsn(&board_path, FOUR_LAYER, &rules()).unwrap();
    let mut m = manifest(&bundle);
    let r2 = m["components"]
        .as_array()
        .unwrap()
        .iter()
        .position(|c| c["reference"] == "R2")
        .unwrap();
    // The fixture stores R2 at -90.
    assert_eq!(m["components"][r2]["rotation_degrees"], 270.0);
    assert!(
        bundle.dsn.contains("(place R2 110000 -50000 front 270)"),
        "{}",
        bundle.dsn
    );
    let ses = ses_for(&m);
    crate::specctra_ses::parse_import_plan(&board_path, FOUR_LAYER, &bundle.manifest, &ses)
        .unwrap();
    // An older manifest that recorded -90 still accepts Freerouting's 270.
    m["components"][r2]["rotation_degrees"] = json!(-90.0);
    crate::specctra_ses::parse_import_plan(&board_path, FOUR_LAYER, &m.to_string(), &ses).unwrap();
    // A real turn is still refused.
    let turned = ses.replace(
        "(place R2 1100000 -500000 front 270)",
        "(place R2 1100000 -500000 front 180)",
    );
    assert_ne!(turned, ses);
    let error =
        crate::specctra_ses::parse_import_plan(&board_path, FOUR_LAYER, &bundle.manifest, &turned)
            .unwrap_err()
            .to_string();
    assert!(error.contains("changes placement"), "{error}");
}

/// Diagnostic: every DSN pin against the pad position the shared board model
/// computes. `KONNECT_DSN_PROBE_FILE=<board> cargo test -p konnect-core
/// probe_pins_match_the_board_model -- --ignored --nocapture`.
#[test]
#[ignore = "needs KONNECT_DSN_PROBE_FILE"]
fn probe_pins_match_the_board_model() {
    let path = std::path::PathBuf::from(std::env::var("KONNECT_DSN_PROBE_FILE").unwrap());
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
    let bundle = export_dsn(&path, &source, &rules).unwrap();
    let pins = router_pin_positions(&bundle.dsn);
    let geometry = konnect_sexp::board_geometry::BoardGeometry::from_tree(&tree);
    let m = manifest(&bundle);
    let (mut worst, mut off, mut total) = (0.0f64, 0, 0);
    for c in m["components"].as_array().unwrap() {
        let reference = c["reference"].as_str().unwrap();
        let fp = geometry.footprint(reference).unwrap();
        for p in c["pads"].as_array().unwrap() {
            let dsn_pin = p["dsn_pin"].as_str().unwrap();
            let (x, y, _) = &pins[dsn_pin];
            let number = p["pad_number"].as_str().unwrap();
            let best = fp
                .pads
                .iter()
                .filter(|g| g.number == number)
                .map(|g| ((x / 1000.0 - g.at.0).powi(2) + (-y / 1000.0 - g.at.1).powi(2)).sqrt())
                .fold(f64::INFINITY, f64::min);
            total += 1;
            worst = worst.max(best);
            if best > 0.001 {
                off += 1;
                if off <= 5 {
                    println!("OFF {dsn_pin}: {best:.4} mm");
                }
            }
        }
    }
    println!("PINS {total} checked, {off} off by more than 1 um, worst {worst:.4} mm");
}

/// `pours: "omit"` leaves the In2.Cu pour out of the DSN so the router can
/// cross its area, while the manifest still counts it for the importer.
#[test]
fn omitted_pours_leave_the_dsn_but_stay_in_the_inventory() {
    let temp = tempfile::tempdir().unwrap();
    let board_path = temp.path().join("board.kicad_pcb");
    std::fs::write(&board_path, FOUR_LAYER).unwrap();
    let options = ExportOptions {
        omit_pours: true,
        ..Default::default()
    };
    let bundle = export_dsn_with_options(&board_path, FOUR_LAYER, &rules(), options).unwrap();
    assert!(!bundle.dsn.contains("(plane"), "{}", bundle.dsn);
    let m = manifest(&bundle);
    assert_eq!(m["supported_profile"]["pours_omitted"], true);
    assert_eq!(m["supported_profile"]["zone_plane_count"], 1);
    // The import plan still binds the board, pour included.
    let ses = ses_for(&m);
    crate::specctra_ses::parse_import_plan(&board_path, FOUR_LAYER, &bundle.manifest, &ses)
        .unwrap();
    // The default keeps it.
    assert!(export().dsn.contains("(plane GND"));
    // KiCad's exporter always writes pours, so it is not adopted.
    let native = include_str!("../tests/fixtures/specctra_two_resistors.native-kicad-10.dsn");
    let error = adopt_native_dsn(bundle, native.to_string())
        .unwrap_err()
        .to_string();
    assert!(error.contains("pours were left out"), "{error}");
}

/// Freerouting 2.5.0 wrote one segment twice on two demo boards, and the
/// importer refused the whole SES. An exact repeat is now applied once and
/// counted; a segment that differs in width is still its own track.
#[test]
fn an_exact_repeat_in_the_ses_is_applied_once_and_counted() {
    let temp = tempfile::tempdir().unwrap();
    let board_path = temp.path().join("board.kicad_pcb");
    std::fs::write(&board_path, FOUR_LAYER).unwrap();
    let bundle = export_dsn(&board_path, FOUR_LAYER, &rules()).unwrap();
    let ses = ses_for(&manifest(&bundle));
    let wire = "        (wire\n          (path In1.Cu 2500\n            1037300 -560000\n            1037300 -530000\n          )\n        )\n";
    assert!(ses.contains(wire));
    let repeated = ses.replacen(wire, &format!("{wire}{wire}"), 1);
    let plan = crate::specctra_ses::parse_import_plan(
        &board_path,
        FOUR_LAYER,
        &bundle.manifest,
        &repeated,
    )
    .unwrap();
    assert_eq!(plan.duplicates_skipped, 1);
    assert_eq!(
        plan.tracks.iter().filter(|t| t.layer == "In1.Cu").count(),
        1
    );
    let wider = wire.replace("2500", "3000");
    let distinct = ses.replacen(wire, &format!("{wire}{wider}"), 1);
    let plan = crate::specctra_ses::parse_import_plan(
        &board_path,
        FOUR_LAYER,
        &bundle.manifest,
        &distinct,
    )
    .unwrap();
    assert_eq!(plan.duplicates_skipped, 0);
    assert_eq!(
        plan.tracks.iter().filter(|t| t.layer == "In1.Cu").count(),
        2
    );
}

// ─── PR 16b: route at the project's rules ────────────────────────────────────

/// On the owner's reference board Freerouting's neck-down wrote 0.15 mm
/// tracks into a 0.2 mm class, and the importer took them. It now refuses a
/// wire narrower than its net's class, and names the worst one.
#[test]
fn a_wire_narrower_than_its_class_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let board_path = temp.path().join("board.kicad_pcb");
    std::fs::write(&board_path, FOUR_LAYER).unwrap();
    let bundle = export_dsn(&board_path, FOUR_LAYER, &rules()).unwrap();
    let m = manifest(&bundle);
    assert_eq!(m["classes"][0]["track_width_um"], 250);
    let ses = ses_for(&m);
    // The class is 0.25 mm; neck the In1.Cu wire down to 0.15 mm.
    let necked = ses.replacen("(path In1.Cu 2500", "(path In1.Cu 1500", 1);
    assert_ne!(necked, ses);
    let error =
        crate::specctra_ses::parse_import_plan(&board_path, FOUR_LAYER, &bundle.manifest, &necked)
            .unwrap_err()
            .to_string();
    assert!(error.contains("narrower than their net allows"), "{error}");
    assert!(
        error.contains("0.150 mm on 'GND'") && error.contains("0.250 mm"),
        "{error}"
    );
    // A manifest from before PR 16b has no classes, so nothing is checked.
    let mut old = m.clone();
    old.as_object_mut().unwrap().remove("classes");
    crate::specctra_ses::parse_import_plan(&board_path, FOUR_LAYER, &old.to_string(), &necked)
        .unwrap();
}

/// A net with a design-intent current is widened to the IPC-2221 width on the
/// stack's worst layer (an inner one here), never narrowed, and the export
/// says so; nets with no current keep their class.
#[test]
fn design_intent_currents_widen_their_nets() {
    let temp = tempfile::tempdir().unwrap();
    let board_path = temp.path().join("board.kicad_pcb");
    std::fs::write(&board_path, FOUR_LAYER).unwrap();
    let intent_path = crate::tools::design_intent::design_intent_path(temp.path());
    std::fs::create_dir_all(intent_path.parent().unwrap()).unwrap();
    std::fs::write(
        &intent_path,
        json!({ "schema_version": 1, "currents": [
            { "net": "GND", "max_current_a": 3.0 },
            { "net": "VCC", "max_current_a": 0.05 },
            { "net": "NOPE", "max_current_a": 1.0 }
        ]})
        .to_string(),
    )
    .unwrap();
    let (widened, report) = widen_for_intent(&board_path, FOUR_LAYER, &rules()).unwrap();
    let gnd = widened["GND"].track_width_mm.unwrap();
    // 3 A at 10 °C on 0.5 oz inner copper (the limiting layer): about 4.8 mm.
    let expected =
        crate::tools::trace_current::required_area_mm2(3.0, 10.0, false) / (0.5 * 0.0348);
    assert!((gnd - expected).abs() < 0.001, "{gnd} vs {expected}");
    assert_eq!(widened["VCC"].track_width_mm, Some(0.25), "never narrowed");
    let status = |net: &str| report.iter().find(|r| r["net"] == net).unwrap()["status"].clone();
    assert_eq!(status("GND"), "widened");
    assert_eq!(status("VCC"), "already_wide_enough");
    assert_eq!(status("NOPE"), "not_routed");
    let limiting = report.iter().find(|r| r["net"] == "GND").unwrap()["limiting_layer"].clone();
    assert!(limiting.as_str().unwrap().starts_with("In"), "{limiting}");
    // The widened rule reaches the DSN as its own class.
    let bundle = export_dsn(&board_path, FOUR_LAYER, &widened).unwrap();
    let m = manifest(&bundle);
    let widths: BTreeSet<i64> = m["classes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["track_width_um"].as_i64().unwrap())
        .collect();
    assert!(
        widths.contains(&250) && widths.iter().any(|w| *w > 4000),
        "{widths:?}"
    );
    // No record: nothing changes. A broken record refuses.
    std::fs::write(&intent_path, "{ broken").unwrap();
    assert!(widen_for_intent(&board_path, FOUR_LAYER, &rules()).is_err());
    std::fs::remove_file(&intent_path).unwrap();
    let (same, report) = widen_for_intent(&board_path, FOUR_LAYER, &rules()).unwrap();
    assert_eq!(same["GND"].track_width_mm, Some(0.25));
    assert!(report.is_empty());
}

/// The project's copper-to-edge clearance travels in the manifest beside the
/// DSN, where `route_specctra_dsn` reads it for Freerouting.
#[test]
fn the_edge_clearance_travels_beside_the_dsn() {
    let temp = tempfile::tempdir().unwrap();
    let options = ExportOptions {
        copper_edge_clearance_um: Some(500),
        ..Default::default()
    };
    let bundle =
        export_dsn_with_options(Path::new("board.kicad_pcb"), FOUR_LAYER, &rules(), options)
            .unwrap();
    let dsn = temp.path().join("job.dsn");
    std::fs::write(&dsn, &bundle.dsn).unwrap();
    assert_eq!(
        manifest_routing_hints(&dsn),
        RoutingHints::default(),
        "no manifest yet"
    );
    std::fs::write(temp.path().join("job.dsn.konnect.json"), &bundle.manifest).unwrap();
    let hints = manifest_routing_hints(&dsn);
    assert_eq!(hints.copper_edge_clearance_um, Some(500));
    assert_eq!(hints.retry_width_um, Some(250), "the narrowest class");
}

/// A net widened to its current keeps its base width as the floor: the
/// router retries a connection it cannot reach wide at that width, and the
/// importer takes it, while anything narrower is still refused.
#[test]
fn a_widened_net_may_fall_back_to_its_base_width_and_no_lower() {
    let temp = tempfile::tempdir().unwrap();
    let board_path = temp.path().join("board.kicad_pcb");
    std::fs::write(&board_path, FOUR_LAYER).unwrap();
    let intent_path = crate::tools::design_intent::design_intent_path(temp.path());
    std::fs::create_dir_all(intent_path.parent().unwrap()).unwrap();
    std::fs::write(
        &intent_path,
        json!({ "schema_version": 1, "currents": [{ "net": "GND", "max_current_a": 3.0 }] })
            .to_string(),
    )
    .unwrap();
    let snapshot = crate::routing_context::Snapshot::read(&board_path).unwrap();
    let options = ExportOptions {
        widths_from_intent: true,
        ..Default::default()
    };
    let bundle =
        export_dsn_with_context(&board_path, FOUR_LAYER, &rules(), &snapshot, options).unwrap();
    let m = manifest(&bundle);
    let gnd = m["nets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["name"] == "GND")
        .unwrap();
    assert_eq!(gnd["min_track_width_um"], 250, "{gnd}");
    assert!(bundle
        .intent_widths
        .iter()
        .any(|w| w["net"] == "GND" && w["status"] == "widened"));
    let ses = ses_for(&m);
    // The fixture SES routes GND at 0.25 mm, its base width: accepted.
    crate::specctra_ses::parse_import_plan(&board_path, FOUR_LAYER, &bundle.manifest, &ses)
        .unwrap();
    // Below the base width: refused.
    let necked = ses.replacen("(path In1.Cu 2500", "(path In1.Cu 1500", 1);
    let error =
        crate::specctra_ses::parse_import_plan(&board_path, FOUR_LAYER, &bundle.manifest, &necked)
            .unwrap_err()
            .to_string();
    assert!(error.contains("floor 0.250 mm"), "{error}");
}

// ─── PR 16b review follow-up: pad-less footprints, retry width, no-width rules ─

const PADLESS_HOOK: &str = "\t(via\n";

/// A footprint with no pads at (104, 44) turned 90°, holding `body`.
fn with_padless(reference: &str, body: &str) -> String {
    let footprint = format!(
        "\t(footprint \"Pkg:Padless\" (layer \"F.Cu\") (uuid \"padless-uuid\") (at 104 44 90) (property \"Reference\" \"{reference}\" (at 0 0 0) (layer \"F.SilkS\") (uuid \"pl\") (effects (font (size 1 1))))\n{body}\n\t)\n{PADLESS_HOOK}"
    );
    let board = FOUR_LAYER.replacen(PADLESS_HOOK, &footprint, 1);
    assert_ne!(board, FOUR_LAYER, "fixture shape changed");
    board
}

fn export_error(board: &str) -> String {
    export_dsn(Path::new("board.kicad_pcb"), board, &rules())
        .unwrap_err()
        .to_string()
}

const ANTENNA_ZONE: &str = "(zone (net 0) (net_name \"\") (layers \"F.Cu\") (uuid \"fz1\") (name \"antenna\") (hatch edge 0.5) (connect_pads (clearance 0)) (min_thickness 0.25) (keepout (tracks not_allowed) (vias not_allowed) (pads allowed) (copperpour not_allowed) (footprints allowed)) (fill (thermal_gap 0.5) (thermal_bridge_width 0.5)) (polygon (pts (xy -1 -1) (xy 1 -1) (xy 1 1) (xy -1 1))))";

/// An antenna keepout or a slot is often a footprint with no pads. The pad-less
/// skip used to run first, so the board exported with the keepout missing.
#[test]
fn a_padless_footprint_with_a_zone_or_edge_cut_is_refused_and_named() {
    let error = export_error(&with_padless("ANT1", ANTENNA_ZONE));
    assert!(
        error.contains("footprint 'ANT1'") && error.contains("rule area"),
        "{error}"
    );
    // No Reference text: the footprint is named by its UUID.
    let error = export_error(&with_padless("", ANTENNA_ZONE));
    assert!(error.contains("footprint 'padless-uuid'"), "{error}");
    let slot = "(fp_rect (start -1 -1) (end 1 1) (stroke (width 0.1) (type solid)) (fill no) (layer \"Edge.Cuts\") (uuid \"fe\"))";
    let error = export_error(&with_padless("SLOT1", slot));
    assert!(
        error.contains("footprint 'SLOT1'") && error.contains("Edge.Cuts"),
        "{error}"
    );
    // The same footprint with silkscreen only is still accepted and counted.
    let silk = "(fp_rect (start -1 -1) (end 1 1) (stroke (width 0.1) (type solid)) (fill no) (layer \"F.SilkS\") (uuid \"fs\"))";
    let bundle = export_dsn(
        Path::new("board.kicad_pcb"),
        &with_padless("LOGO1", silk),
        &rules(),
    )
    .unwrap();
    assert_eq!(
        manifest(&bundle)["supported_profile"]["footprints_without_pads"],
        1
    );
}

/// Copper graphics on a footprint with no pads (a copper logo) are in the
/// router's way: they become board keepouts, turned and moved with the part.
#[test]
fn copper_graphics_on_a_padless_footprint_become_keepouts_in_board_space() {
    // A 2 mm line along the part's X; the part is at (104, 44) turned 90°
    // counter-clockwise, so it runs from (104, 44) up to (104, 42) on the board.
    let line = "(fp_line (start 0 0) (end 2 0) (stroke (width 0.2) (type solid)) (layer \"F.Cu\") (uuid \"fl\"))";
    let base = export_dsn(Path::new("board.kicad_pcb"), FOUR_LAYER, &rules()).unwrap();
    let base_count = manifest(&base)["supported_profile"]["copper_graphic_keepout_count"]
        .as_u64()
        .unwrap();
    let bundle = export_dsn(
        Path::new("board.kicad_pcb"),
        &with_padless("LOGO1", line),
        &rules(),
    )
    .unwrap();
    let m = manifest(&bundle);
    assert_eq!(
        m["supported_profile"]["copper_graphic_keepout_count"]
            .as_u64()
            .unwrap(),
        base_count + 1
    );
    assert_eq!(m["supported_profile"]["footprints_without_pads"], 1);
    // It is not a placed part.
    assert!(m["components"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["reference"] != "LOGO1"));
    let tree = parse_dsn_sexp(&bundle.dsn).unwrap();
    let structure = tree.find("structure").unwrap();
    let path = structure
        .find_all("keepout")
        .into_iter()
        .filter_map(|k| k.find("path"))
        .find(|p| p.get(1).and_then(SexpNode::as_str) == Some("F.Cu"))
        .expect("the logo line is a structure keepout on F.Cu");
    let numbers: Vec<f64> = path
        .children()
        .unwrap()
        .iter()
        .skip(3)
        .filter_map(|n| n.as_str()?.parse().ok())
        .collect();
    assert_eq!(numbers, vec![104000.0, -44000.0, 104000.0, -42000.0]);
    // Stroke 0.2 mm plus the chord tolerance on each side.
    assert_eq!(path.get(2).and_then(SexpNode::as_str), Some("210"));
}

fn two_class_rules(gnd_width: f64, vcc_width: f64) -> IpcEffectiveRoutingRules {
    let mut rules = rules();
    rules.get_mut("GND").unwrap().track_width_mm = Some(gnd_width);
    rules.get_mut("VCC").unwrap().track_width_mm = Some(vcc_width);
    rules
}

fn hints_of(bundle: &ExportBundle) -> RoutingHints {
    let temp = tempfile::tempdir().unwrap();
    let dsn = temp.path().join("job.dsn");
    std::fs::write(temp.path().join("job.dsn.konnect.json"), &bundle.manifest).unwrap();
    manifest_routing_hints(&dsn)
}

/// Freerouting retries any failed connection at its one retry width, so it is
/// sent only when no net's floor is above it.
#[test]
fn the_retry_width_is_off_when_net_classes_have_different_floors() {
    let board = Path::new("board.kicad_pcb");
    // One class: the retry width is that class's width.
    let same = export_dsn(board, FOUR_LAYER, &rules()).unwrap();
    let hints = hints_of(&same);
    assert_eq!(hints.retry_width_um, Some(250));
    assert_eq!(hints.retry_off_reason, None);
    // GND 0.5 mm, VCC 0.25 mm: a 0.25 mm retry would take GND below its class.
    let two = export_dsn(board, FOUR_LAYER, &two_class_rules(0.5, 0.25)).unwrap();
    let hints = hints_of(&two);
    assert_eq!(hints.retry_width_um, None);
    assert_eq!(hints.retry_off_reason, Some(RETRY_OFF_DIFFERENT_FLOORS));
    assert!(hints
        .retry_off_reason
        .unwrap()
        .contains("different minimum widths"));
}

/// A widened net's floor is its base width, so widening alone keeps the retry
/// on, at that width.
#[test]
fn a_widened_net_keeps_the_retry_on_at_the_shared_base_width() {
    let temp = tempfile::tempdir().unwrap();
    let board_path = temp.path().join("board.kicad_pcb");
    std::fs::write(&board_path, FOUR_LAYER).unwrap();
    let intent_path = crate::tools::design_intent::design_intent_path(temp.path());
    std::fs::create_dir_all(intent_path.parent().unwrap()).unwrap();
    std::fs::write(
        &intent_path,
        json!({ "schema_version": 1, "currents": [{ "net": "GND", "max_current_a": 3.0 }] })
            .to_string(),
    )
    .unwrap();
    let snapshot = crate::routing_context::Snapshot::read(&board_path).unwrap();
    let options = ExportOptions {
        widths_from_intent: true,
        ..Default::default()
    };
    let bundle =
        export_dsn_with_context(&board_path, FOUR_LAYER, &rules(), &snapshot, options).unwrap();
    let hints = hints_of(&bundle);
    assert_eq!(hints.retry_width_um, Some(250));
    assert_eq!(hints.retry_off_reason, None);
}

/// A run that sent the narrowest floor as Freerouting's retry width narrows a
/// wider net to exactly that width; the importer names that cause. A track
/// narrower than every floor is still put down to fanout and neck-down.
#[test]
fn a_wire_at_the_retry_width_gets_the_retry_width_message() {
    let temp = tempfile::tempdir().unwrap();
    let board_path = temp.path().join("board.kicad_pcb");
    std::fs::write(&board_path, FOUR_LAYER).unwrap();
    let bundle = export_dsn(&board_path, FOUR_LAYER, &two_class_rules(0.5, 0.25)).unwrap();
    let m = manifest(&bundle);
    let ses = ses_for(&m);
    // `ses_for` routes GND at 0.25 mm: the narrowest floor on the board
    // (VCC's), below GND's 0.5 mm class.
    let error =
        crate::specctra_ses::parse_import_plan(&board_path, FOUR_LAYER, &bundle.manifest, &ses)
            .unwrap_err()
            .to_string();
    assert!(
        error.contains("retry width (neck_width_um): it is one value for every net")
            && error.contains("'GND'")
            && error.contains("0.250 mm")
            && error.contains("0.500 mm"),
        "{error}"
    );
    // Narrower than any floor: not the retry width, so the old advice stands.
    let necked = ses
        .replace("(path In1.Cu 2500", "(path In1.Cu 1500")
        .replace("(path B.Cu 2500", "(path B.Cu 1500");
    let error =
        crate::specctra_ses::parse_import_plan(&board_path, FOUR_LAYER, &bundle.manifest, &necked)
            .unwrap_err()
            .to_string();
    assert!(
        !error.contains("retry width") && error.contains("fanout"),
        "{error}"
    );
}

/// A rule with no track width stays incomplete: intent widening must not
/// fill it from the current, or the export would accept it.
#[test]
fn a_net_with_no_track_width_is_not_widened_and_is_still_refused() {
    let temp = tempfile::tempdir().unwrap();
    let board_path = temp.path().join("board.kicad_pcb");
    std::fs::write(&board_path, FOUR_LAYER).unwrap();
    let intent_path = crate::tools::design_intent::design_intent_path(temp.path());
    std::fs::create_dir_all(intent_path.parent().unwrap()).unwrap();
    std::fs::write(
        &intent_path,
        json!({ "schema_version": 1, "currents": [{ "net": "VCC", "max_current_a": 2.0 }] })
            .to_string(),
    )
    .unwrap();
    let mut incomplete = rules();
    incomplete.get_mut("VCC").unwrap().track_width_mm = None;
    let (widened, report) = widen_for_intent(&board_path, FOUR_LAYER, &incomplete).unwrap();
    assert_eq!(widened["VCC"].track_width_mm, None, "left as it was");
    let vcc = report.iter().find(|r| r["net"] == "VCC").unwrap();
    assert_eq!(vcc["status"], "not_widened");
    let error = export_dsn(&board_path, FOUR_LAYER, &widened)
        .unwrap_err()
        .to_string();
    assert!(error.contains("track width"), "{error}");
    // Through the whole export, with intent widening on.
    let snapshot = crate::routing_context::Snapshot::read(&board_path).unwrap();
    let options = ExportOptions {
        widths_from_intent: true,
        ..Default::default()
    };
    let error = export_dsn_with_context(&board_path, FOUR_LAYER, &incomplete, &snapshot, options)
        .map(|_| ())
        .unwrap_err()
        .to_string();
    assert!(!error.is_empty());
}
