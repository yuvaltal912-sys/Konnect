//! The public fixture has two opposite, asymmetric copper pads of the same anchor size.
use super::tests::{parse_dsn, rules};
use super::*;

const BOARD: &str = include_str!("../tests/fixtures/specctra_custom_pad.kicad_pcb");
const TWO_RESISTORS: &str = include_str!("../tests/fixtures/specctra_two_resistors.kicad_pcb");

fn export(source: &str) -> Result<ExportBundle> {
    export_dsn(Path::new("board.kicad_pcb"), source, &rules())
}

fn pin_stack<'a>(pcb: &'a dsn::Pcb, image: &str, id: &str) -> &'a dsn::Padstack {
    let image = pcb.library.images.iter().find(|i| i.name == image).unwrap();
    let pin = image.pins.iter().find(|p| p.id == id).unwrap();
    pcb.library
        .padstacks
        .iter()
        .find(|s| s.name == pin.name)
        .unwrap()
}

#[test]
fn custom_pad_copper_in_the_dsn_matches_both_fixture_pads() {
    let bundle = export(BOARD).unwrap();
    let pcb = parse_dsn(&bundle.dsn).pcb;
    let stack1 = pin_stack(&pcb, "konnect_image_R1", "1");
    let stack2 = pin_stack(&pcb, "konnect_image_R2", "2");
    assert_ne!(stack1.name, stack2.name);
    let manifest: serde_json::Value = serde_json::from_str(&bundle.manifest).unwrap();
    assert_eq!(
        manifest["padstacks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["shape"] == "custom")
            .count(),
        2
    );

    for (stack, direction) in [(stack1, -1.0), (stack2, 1.0)] {
        let [dsn::Shape::Polygon(poly)] = stack.shapes.as_slice() else {
            panic!("one copper polygon per layer required: {stack:?}");
        };
        assert_eq!((poly.layer.as_str(), poly.width), ("F.Cu", 0.0));
        let points: Vec<_> = poly.coords.iter().map(|p| (p.x, p.y)).collect();
        // In KiCad's local frame these parts make a 400 µm-radius capsule from
        // y = -600 to 1200. Test its continuous analytical shape against the
        // parsed DSN away from the 5 µm chord error and the 1 µm grid.
        for x in (-450..=450).step_by(25) {
            for y in (-1650..=1650).step_by(25) {
                let local_y = f64::from(y) * direction;
                let distance = f64::from(x).hypot(local_y - local_y.clamp(-600.0, 1200.0));
                if (distance - 400.0).abs() > 6.0 {
                    assert_eq!(
                        konnect_sexp::planar::point_in_ring((f64::from(x), f64::from(y)), &points),
                        distance < 400.0,
                        "wrong copper at ({x}, {y}) for {}",
                        stack.name
                    );
                }
            }
        }
    }
}

fn with_custom_pad(anchor: &str, primitives: &str) -> String {
    TWO_RESISTORS
        .replace("\r\n", "\n")
        .replacen("(pad \"1\" smd rect", "(pad \"1\" smd custom", 1)
        .replacen(
            "(size 0.6 0.5)",
            &format!("(size 1 1) (options (anchor {anchor})) (primitives {primitives})"),
            1,
        )
}

#[test]
fn a_custom_pad_must_have_one_convex_copper_region_without_holes() {
    let rect = |x1, y1, x2, y2| {
        format!("(gr_rect (start {x1} {y1}) (end {x2} {y2}) (width 0) (fill yes))")
    };
    let disconnected = rect(2.0, 0.0, 3.0, 1.0);
    let concave = rect(0.2, 0.2, 1.0, 1.0);
    // Four overlapping bars enclose a void to the right of the anchor.
    let hole = [
        rect(0.0, -0.5, 2.0, -0.25),
        rect(0.0, 0.25, 2.0, 0.5),
        rect(1.75, -0.5, 2.0, 0.5),
    ]
    .join(" ");
    for (geometry, reason) in [
        (disconnected, "disconnected"),
        (concave, "concave"),
        (hole, "hole"),
    ] {
        let error = format!(
            "{:#}",
            export(&with_custom_pad("rect", &geometry)).unwrap_err()
        );
        assert!(error.contains("R1-1") && error.contains(reason), "{error}");
    }
    // Filling the void gives a plain convex contact.
    export(&with_custom_pad("rect", &rect(0.0, -0.5, 2.0, 0.5))).unwrap();
}

#[test]
fn a_custom_pad_never_drops_an_unmodelled_copper_primitive() {
    let shape = "(gr_curve (pts (xy 0 0) (xy 1 0) (xy 1 1) (xy 2 1)) (width 0.2))";
    let error = format!("{:#}", export(&with_custom_pad("rect", shape)).unwrap_err());
    assert!(
        error.contains("R1-1") && error.contains("gr_curve"),
        "{error}"
    );
}

#[test]
fn circle_anchor_and_graphical_guides_have_kicads_copper() {
    // gr_bbox and gr_vector are editing guides in KiCad's primitive list.
    let guides = "(gr_bbox (start 0 0) (end 9 9)) (gr_vector (start 0 0) (end 8 8))";
    let board = with_custom_pad("circle", guides);
    assert_eq!(
        export(&board).unwrap().dsn,
        export(&with_custom_pad("circle", "")).unwrap().dsn
    );
    let pcb = parse_dsn(&export(&board).unwrap().dsn).pcb;
    let [dsn::Shape::Polygon(poly)] = pin_stack(&pcb, "konnect_image_R1", "1").shapes.as_slice()
    else {
        panic!("expected the circle anchor polygon")
    };
    assert!(poly
        .coords
        .iter()
        .all(|p| (p.x.hypot(p.y) - 500.0).abs() < 1.0));
}

#[test]
fn custom_pads_with_unknown_anchor_stroke_or_layer_geometry_are_refused() {
    let bad_anchor = with_custom_pad("oval", "");
    let unknown_width = with_custom_pad(
        "rect",
        "(gr_circle (center 0 0) (end 1 0) (width wide) (fill yes))",
    );
    let layers = "(padstack (mode custom) (layer \"F.Cu\" (shape custom) (size 2 2)))";
    let layered = with_custom_pad("rect", "").replace("(primitives )", layers);
    for (source, reason) in [
        (bad_anchor, "anchor"),
        (unknown_width, "width"),
        (layered, "layer"),
    ] {
        let error = format!("{:#}", export(&source).unwrap_err());
        assert!(error.contains("R1-1") && error.contains(reason), "{error}");
    }
}

#[test]
fn custom_pad_straight_strokes_include_their_width_and_round_caps() {
    let line = "(gr_line (start 0 0) (end 2 0) (width 1))";
    let pcb = parse_dsn(&export(&with_custom_pad("circle", line)).unwrap().dsn).pcb;
    let [dsn::Shape::Polygon(poly)] = pin_stack(&pcb, "konnect_image_R1", "1").shapes.as_slice()
    else {
        panic!("expected a polygon")
    };
    let ring: Vec<_> = poly.coords.iter().map(|p| (p.x, p.y)).collect();
    for (p, inside) in [
        ((2450.0, 0.0), true),
        ((1000.0, 450.0), true),
        ((2450.0, 450.0), false),
    ] {
        assert_eq!(
            konnect_sexp::planar::point_in_ring(p, &ring),
            inside,
            "{p:?}"
        );
    }
}

#[test]
fn ambiguous_custom_pad_drawings_are_refused_before_export() {
    for (prim, reason) in [
        (
            "(gr_circle (center 0 0) (end 2 0) (width 0.2) (fill no))",
            "unfilled",
        ),
        (
            "(gr_rect (start 0 0) (end 1 1) (radius 0.3) (width 0) (fill yes))",
            "rounded",
        ),
        (
            "(gr_poly (pts (xy 0 0) (xy 1 1) (xy 0 1) (xy 1 0)) (width 0) (fill yes))",
            "invalid custom pad polygon",
        ),
        (
            "(gr_poly (pts (xy 0 0) (xy 1 0) (xy 0 1)) (width 0.2) (fill yes))",
            "stroked",
        ),
        (
            "(gr_line (start 0 0) (end 1 1) (width 0))",
            "positive width",
        ),
        (
            "(gr_rect (start 0 0) (end 2000000 1) (width 0) (fill yes))",
            "coordinate range",
        ),
    ] {
        let err = format!("{:#}", export(&with_custom_pad("rect", prim)).unwrap_err());
        assert!(err.contains("R1-1") && err.contains(reason), "{err}");
    }
}

#[test]
fn a_plated_custom_pads_offset_and_copper_follow_each_layer() {
    let board = with_custom_pad(
        "rect",
        "(gr_rect (start -0.5 -0.5) (end 1.5 0.5) (fill yes))",
    )
    .replacen("smd custom", "thru_hole custom", 1)
    .replacen(
        "(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")",
        "(drill 0.3 (offset 0.1 0.05)) (layers \"*.Cu\" \"*.Mask\")",
        1,
    );
    let pcb = parse_dsn(&export(&board).unwrap().dsn).pcb;
    let stack = pin_stack(&pcb, "konnect_image_R1", "1");
    let layers: Vec<_> = stack
        .shapes
        .iter()
        .map(|shape| {
            let dsn::Shape::Polygon(p) = shape else {
                panic!("expected a pad polygon")
            };
            let vertices: BTreeSet<_> = p.coords.iter().map(|p| (p.x as i32, p.y as i32)).collect();
            assert_eq!(
                vertices,
                BTreeSet::from([(-400, -550), (1600, -550), (1600, 450), (-400, 450)])
            );
            p.layer.as_str()
        })
        .collect();
    assert_eq!(layers, ["F.Cu", "B.Cu"]);
}

#[tokio::test]
#[ignore = "requires KONNECT_FASTROUTE and KiCad CLI"]
async fn fastroute_connects_the_custom_pad_fixture_after_kicad_refill() {
    use crate::place_judge::RouteResult;
    use crate::place_route::{
        export_candidate, export_options, isolated_context, route_candidate, RouteInputs,
    };
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("input/board.kicad_pcb");
    std::fs::create_dir(source.parent().unwrap()).unwrap();
    std::fs::write(&source, BOARD).unwrap();
    let cli = crate::kicad_install::find_cli("")
        .expect("KiCad CLI")
        .display()
        .to_string();
    let inputs = RouteInputs {
        options: export_options(&source),
        board: source,
        rules: rules(),
        effort: crate::route_search::Effort::Fast,
        fastroute: std::env::var_os("KONNECT_FASTROUTE")
            .expect("set KONNECT_FASTROUTE")
            .into(),
        kicad_config_home: temp.path().join("settings"),
        kicad_cli: cli.clone(),
    };
    let candidate = temp.path().join("candidate");
    let dsn = export_candidate(&inputs, &candidate, BOARD).unwrap();
    let ctx = isolated_context(&temp.path().join("isolation"), &cli).unwrap();
    let route = route_candidate(ctx, &inputs, &dsn, &candidate).await;
    let RouteResult::Routed {
        gates, measures, ..
    } = route
    else {
        panic!("{route:?}")
    };
    assert!(gates.g1.is_empty(), "{:?}", gates.g1);
    assert_eq!(measures.unconnected, 0);
    let manifest = std::fs::read_to_string(dsn.with_extension("dsn.konnect.json")).unwrap();
    let ses = std::fs::read_to_string(candidate.join("route.ses")).unwrap();
    let plan = crate::specctra_ses::parse_import_plan(
        &dsn.with_extension("kicad_pcb"),
        BOARD,
        &manifest,
        &ses,
    )
    .unwrap();
    assert!(!plan.tracks.is_empty());
}

#[test]
fn a_rotated_back_custom_pad_uses_its_copper_layer_and_local_frame() {
    let board = with_custom_pad(
        "rect",
        "(gr_rect (start -0.5 -0.5) (end 1.5 0.5) (width 0))",
    )
    .replacen("(layer \"F.Cu\")", "(layer \"B.Cu\")", 1)
    .replacen("(at 100 50)", "(at 100 50 30)", 1)
    .replacen("(at -0.5 0)", "(at -0.5 0 120)", 1)
    .replacen(
        "(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")",
        "(layers \"B.Cu\" \"B.Mask\" \"B.Paste\")",
        1,
    );
    let pcb = parse_dsn(&export(&board).unwrap().dsn).pcb;
    let image = pcb
        .library
        .images
        .iter()
        .find(|i| i.name == "konnect_image_R1")
        .unwrap();
    let pin = image.pins.iter().find(|p| p.id == "1").unwrap();
    assert_eq!(pin.rotate, Some(90.0));
    let [dsn::Shape::Polygon(p)] = pin_stack(&pcb, &image.name, "1").shapes.as_slice() else {
        panic!("expected pad polygon")
    };
    assert_eq!(p.layer, "B.Cu");
    let corners: BTreeSet<_> = p.coords.iter().map(|v| (v.x as i32, v.y as i32)).collect();
    assert_eq!(
        corners,
        BTreeSet::from([(-500, -500), (-500, 500), (1500, -500), (1500, 500)])
    );
}

#[test]
fn a_submicrometre_gap_does_not_connect_a_custom_pad_island() {
    let gap = "(gr_rect (start 0.5004 -0.5) (end 2 0.5) (width 0) (fill yes))";
    let error = format!("{:#}", export(&with_custom_pad("rect", gap)).unwrap_err());
    assert!(error.contains("disconnected"), "{error}");
    export(&with_custom_pad("rect", &gap.replace("0.5004", "0.5"))).unwrap();
}

#[test]
fn a_stroke_and_its_circular_anchor_agree_at_every_cap_angle() {
    for end in ["2 0", "1.2 1.6"] {
        let source = with_custom_pad(
            "circle",
            &format!("(gr_line (start 0 0) (end {end}) (width 0.6))"),
        )
        .replacen("(size 1 1) (options", "(size 0.6 0.6) (options", 1);
        let _ = export(&source).unwrap();
    }
}

#[test]
fn a_surface_custom_offset_can_be_written_without_a_drill_diameter() {
    let shape = "(gr_rect (start -0.5 -0.5) (end 1.5 0.5) (fill yes))";
    let source = with_custom_pad("rect", shape).replacen(
        "(size 1 1) (options",
        "(size 1 1) (drill (offset 0.1004 0.0503)) (options",
        1,
    );
    let pcb = parse_dsn(&export(&source).unwrap().dsn).pcb;
    let [dsn::Shape::Polygon(p)] = pin_stack(&pcb, "konnect_image_R1", "1").shapes.as_slice()
    else {
        panic!("expected polygon")
    };
    assert!(p.coords.iter().any(|p| p.x == -399.6 && p.y == 449.7));
    assert!(p.coords.iter().any(|p| p.x == 1600.4 && p.y == -550.3));
    let oval = source.replace("(drill (offset", "(drill oval (offset");
    assert_eq!(export(&source).unwrap().dsn, export(&oval).unwrap().dsn);
}

#[test]
fn a_custom_plated_slot_fits_inside_the_offset_copper() {
    let board = with_custom_pad(
        "circle",
        "(gr_rect (start -0.7 -0.7) (end 0.7 0.7) (fill yes))",
    )
    .replacen("smd custom", "thru_hole custom", 1)
    .replacen(
        "(size 1 1) (options",
        "(size 0.2 0.2) (drill oval 0.8 0.5 (offset 0.2 0)) (options",
        1,
    )
    .replacen(
        "(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")",
        "(layers \"*.Cu\" \"*.Mask\")",
        1,
    );
    export(&board).unwrap();
    let bad = board.replace("(offset 0.2 0)", "(offset 0.5 0)");
    assert!(format!("{:#}", export(&bad).unwrap_err()).contains("positive ring"));
}

#[test]
fn a_custom_pad_with_layer_specific_drilling_or_unflashed_layers_is_refused() {
    let base = with_custom_pad("rect", "")
        .replacen("smd custom", "thru_hole custom", 1)
        .replacen(
            "(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")",
            "(drill 0.3) (layers \"*.Cu\")",
            1,
        )
        .replacen(
            "(2 \"B.Cu\" signal)",
            "(4 \"In1.Cu\" signal) (6 \"In2.Cu\" signal) (2 \"B.Cu\" signal)",
            1,
        );
    export(&base).unwrap();
    for option in [
        "(backdrill (size 0.8) (layers \"F.Cu\" \"In1.Cu\"))",
        "(tertiary_drill (size 0.8) (layers \"In2.Cu\" \"B.Cu\"))",
        "(front_post_machining counterbore (size 0.8))",
        "(back_post_machining countersink (size 0.8))",
        "(remove_unused_layers)",
        "(remove_unused_layers true)",
    ] {
        let error = format!(
            "{:#}",
            export(&base.replace("(primitives )", option)).unwrap_err()
        );
        assert!(
            error.contains("R1-1")
                && (error.contains("machined") || error.contains("unused layers")),
            "{error}"
        );
    }
    let explicit = base.replace(
        "(layers \"*.Cu\")",
        "(layers \"F.Cu\" \"In1.Cu\" \"In2.Cu\" \"B.Cu\")",
    );
    assert_eq!(export(&base).unwrap().dsn, export(&explicit).unwrap().dsn);
    let partial = base.replace("(layers \"*.Cu\")", "(layers \"F.Cu\" \"B.Cu\")");
    assert!(format!("{:#}", export(&partial).unwrap_err()).contains("partial copper stack"));
}

#[test]
fn each_primitive_list_contributes_and_native_adoption_checks_custom_identity() {
    let shape = "(gr_rect (start -0.5 -0.5) (end 1.5 0.5) (fill yes))";
    let source = with_custom_pad("rect", shape);
    let ordinary = export(&source).unwrap();
    let repeated = export(&source.replace("(primitives ", "(primitives) (primitives ")).unwrap();
    assert_eq!(ordinary.dsn, repeated.dsn);
    assert_eq!(
        ordinary.rust_only_bridge_reasons(),
        ["the board has custom pad polygons"]
    );
    assert!(export(TWO_RESISTORS)
        .unwrap()
        .rust_only_bridge_reasons()
        .is_empty());
    assert!(format!(
        "{:#}",
        adopt_native_dsn(ordinary, repeated.dsn).unwrap_err()
    )
    .contains("custom pad polygon identity"));
}

#[test]
fn the_projected_custom_plated_pad_still_has_a_positive_ring() {
    let source = with_custom_pad(
        "circle",
        "(gr_rect (start -0.50004 -0.50004) (end 0.50004 0.50004) (fill yes))",
    )
    .replacen("smd custom", "thru_hole custom", 1)
    .replacen(
        "(size 1 1) (options",
        "(size 0.2 0.2) (drill 1) (options",
        1,
    )
    .replacen(
        "(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")",
        "(layers \"*.Cu\")",
        1,
    );
    let error = format!("{:#}", export(&source).unwrap_err());
    assert!(error.contains("positive ring"), "{error}");
    export(&source.replace("0.50004", "0.50006")).unwrap();
}

#[test]
fn odd_nanometres_do_not_extend_the_negative_side_of_the_anchor() {
    let rectangle = "(gr_rect (start -2 -0.5) (end -0.500002 0.5) (width 0) (fill yes))";
    let source = with_custom_pad("rect", rectangle).replacen(
        "(size 1 1) (options",
        "(size 1.000003 1) (options",
        1,
    );
    let error = format!("{:#}", export(&source).unwrap_err());
    assert!(error.contains("disconnected"), "{error}");
    export(&source.replace("-0.500002", "-0.500001")).unwrap();
}
