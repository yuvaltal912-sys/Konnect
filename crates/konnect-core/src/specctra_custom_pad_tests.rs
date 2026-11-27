//! The public fixture has two opposite, asymmetric copper pads of the same anchor size.
use super::custom_pad::{area2, area2_outside, grown, side, split_concave, Contour};
use super::tests::{parse_dsn, rules};
use super::*;
use i_overlay::core::{fill_rule::FillRule, overlay::Overlay, overlay_rule::OverlayRule};
use i_overlay::i_float::int::point::IntPoint;
use i_overlay::i_shape::int::path::ContourExtension;

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
fn a_custom_pad_must_be_one_copper_region_without_holes() {
    let rect = |x1, y1, x2, y2| {
        format!("(gr_rect (start {x1} {y1}) (end {x2} {y2}) (width 0) (fill yes))")
    };
    let disconnected = rect(2.0, 0.0, 3.0, 1.0);
    // Four overlapping bars enclose a void to the right of the anchor.
    let hole = [
        rect(0.0, -0.5, 2.0, -0.25),
        rect(0.0, 0.25, 2.0, 0.5),
        rect(1.75, -0.5, 2.0, 0.5),
    ]
    .join(" ");
    for (geometry, reason) in [(disconnected, "disconnected"), (hole, "hole")] {
        let error = format!(
            "{:#}",
            export(&with_custom_pad("rect", &geometry)).unwrap_err()
        );
        assert!(error.contains("R1-1") && error.contains(reason), "{error}");
    }
    // Filling the void gives a plain convex contact.
    let convex = export(&with_custom_pad("rect", &rect(0.0, -0.5, 2.0, 0.5))).unwrap();
    assert!(convex.concave_custom_pads.is_empty());
    // A concave region is exported: a convex pin and a keepout for the rest.
    let concave = export(&with_custom_pad("rect", &rect(0.2, 0.2, 1.0, 1.0))).unwrap();
    assert_eq!(concave.concave_custom_pads, ["R1-1"]);
    let pcb = parse_dsn(&concave.dsn).pcb;
    let image = pcb
        .library
        .images
        .iter()
        .find(|i| i.name == "konnect_image_R1")
        .unwrap();
    assert_eq!(image.keepouts.0.len(), 1, "{:?}", image.keepouts);
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
    route_after_kicad_refill(BOARD).await;
}

#[tokio::test]
#[ignore = "requires KONNECT_FASTROUTE and KiCad CLI"]
async fn fastroute_connects_a_concave_custom_pad_after_kicad_refill() {
    // The owner's capacitor pad at full size, moved clear of the other pad. At a
    // fifth of the size the tab ends sit 0.25 mm from the pad's centre, too close
    // for a 0.25 mm track with 0.2 mm clearance to reach the pin between them.
    let full = CROSS
        .replace("0.13", "0.65")
        .replace("0.47", "2.35")
        .replace("0.25", "1.25")
        .replace("0.51", "2.55")
        .replace("0.37", "1.85");
    let board = with_custom_pad("rect", &full)
        .replacen("(size 1 1) (options", "(size 2 2) (options", 1)
        .replacen("(at -0.5 0)", "(at -4 0)", 1);
    route_after_kicad_refill(&board).await;
}

/// Route a board with fastroute at effort fast, judge it after KiCad's refill,
/// and import the session: nothing open, and real tracks.
async fn route_after_kicad_refill(board: &str) {
    use crate::place_judge::RouteResult;
    use crate::place_route::{
        export_candidate, export_options, isolated_context, route_candidate, RouteInputs,
    };
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("input/board.kicad_pcb");
    std::fs::create_dir(source.parent().unwrap()).unwrap();
    std::fs::write(&source, board).unwrap();
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
    let dsn = export_candidate(&inputs, &candidate, board).unwrap();
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
        board,
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

// ─── Concave custom pads: a convex pin plus keepouts ────────────────────────

/// A plus-shaped pad drawn as one concave polygon, the shape of the owner's
/// electrolytic capacitor pad, at a fifth of its size: a bar from x = -0.51 to
/// 0.37 and y = -0.25 to 0.25, crossed by a tab from y = -0.47 to 0.47 that is
/// 0.26 wide. The anchor is a 0.4 mm square inside the bar.
const CROSS: &str = "(gr_poly (pts (xy -0.13 0.47) (xy -0.13 0.25) (xy -0.51 0.25) \
     (xy -0.51 -0.25) (xy -0.13 -0.25) (xy -0.13 -0.47) (xy 0.13 -0.47) (xy 0.13 -0.25) \
     (xy 0.37 -0.25) (xy 0.37 0.25) (xy 0.13 0.25) (xy 0.13 0.47)) (width 0))";

fn with_cross_pad() -> String {
    with_custom_pad("rect", CROSS).replacen("(size 1 1) (options", "(size 0.4 0.4) (options", 1)
}

/// The cross in the pad's own frame with Y up, in micrometres.
fn in_cross((x, y): (f64, f64)) -> bool {
    let bar = (-510.0..=370.0).contains(&x) && (-250.0..=250.0).contains(&y);
    let tab = (-130.0..=130.0).contains(&x) && (-470.0..=470.0).contains(&y);
    bar || tab
}

/// Distance in micrometres from a point to the cross's outline, for sampling
/// clear of the grid, the 5 µm keepout growth and rounding.
fn near_cross_edge((x, y): (f64, f64), within: f64) -> bool {
    let near = |v: f64, edges: &[f64]| edges.iter().any(|e| (v - e).abs() <= within);
    near(x, &[-510.0, 370.0, -130.0, 130.0]) || near(y, &[-250.0, 250.0, -470.0, 470.0])
}

fn polygon_points(shape: &dsn::Shape) -> (&str, Vec<(f64, f64)>) {
    let dsn::Shape::Polygon(p) = shape else {
        panic!("expected a polygon: {shape:?}")
    };
    (
        p.layer.as_str(),
        p.coords.iter().map(|c| (c.x, c.y)).collect(),
    )
}

/// The pin's polygon and the image's keepouts, both in the image frame.
fn router_copper(pcb: &dsn::Pcb, image: &str, id: &str) -> (Vec<(f64, f64)>, Vec<Vec<(f64, f64)>>) {
    let image = pcb.library.images.iter().find(|i| i.name == image).unwrap();
    let pin = image.pins.iter().find(|p| p.id == id).unwrap();
    let stack = pin_stack(pcb, &image.name, id);
    let [shape] = stack.shapes.as_slice() else {
        panic!("one pin polygon per layer: {stack:?}")
    };
    let (sin, cos) = pin.rotate.unwrap_or(0.0).to_radians().sin_cos();
    let placed = polygon_points(shape)
        .1
        .into_iter()
        .map(|(x, y)| (x * cos - y * sin + pin.x, x * sin + y * cos + pin.y))
        .collect();
    let keepouts = image
        .keepouts
        .0
        .iter()
        .map(|k| polygon_points(&k.shape).1)
        .collect();
    (placed, keepouts)
}

#[test]
fn a_concave_custom_pad_is_a_convex_pin_and_keepouts_of_its_other_copper() {
    let bundle = export(&with_cross_pad()).unwrap();
    assert_eq!(bundle.concave_custom_pads, ["R1-1"]);
    let pcb = parse_dsn(&bundle.dsn).pcb;
    // The pin is the bar, the largest convex part around the pad's centre.
    let [shape] = pin_stack(&pcb, "konnect_image_R1", "1").shapes.as_slice() else {
        panic!("expected one pin polygon")
    };
    let (layer, pin) = polygon_points(shape);
    assert_eq!(layer, "F.Cu");
    let corners: BTreeSet<_> = pin.iter().map(|&(x, y)| (x as i32, y as i32)).collect();
    assert_eq!(
        corners,
        BTreeSet::from([(-510, -250), (370, -250), (370, 250), (-510, 250)])
    );
    // The two ends of the tab are keepouts on the pad's layer, at the pad.
    let image = pcb
        .library
        .images
        .iter()
        .find(|i| i.name == "konnect_image_R1")
        .unwrap();
    assert_eq!(image.keepouts.0.len(), 2, "{:?}", image.keepouts);
    for keepout in &image.keepouts.0 {
        let dsn::Shape::Polygon(p) = &keepout.shape else {
            panic!("{keepout:?}")
        };
        assert_eq!((p.layer.as_str(), p.width), ("F.Cu", 10.0));
    }
    // Together the router's pin and keepouts are the pad's copper, and the pin
    // takes no copper the pad does not have.
    let (placed, keepouts) = router_copper(&pcb, "konnect_image_R1", "1");
    for x in (-700..=500).step_by(10) {
        for y in (-600..=600).step_by(10) {
            let local = (f64::from(x), f64::from(y));
            if near_cross_edge(local, 8.0) {
                continue;
            }
            let at = (local.0 - 500.0, local.1);
            let in_pin = konnect_sexp::planar::point_in_ring(at, &placed);
            let in_keepout = keepouts
                .iter()
                .any(|k| konnect_sexp::planar::point_in_ring(at, k));
            assert_eq!(in_pin || in_keepout, in_cross(local), "at {local:?}");
            assert!(!in_pin || in_cross(local), "pin copper at {local:?}");
            assert!(!(in_pin && in_keepout), "pin under a keepout at {local:?}");
        }
    }
    // The manifest counts them with the other copper keepouts.
    let manifest: serde_json::Value = serde_json::from_str(&bundle.manifest).unwrap();
    assert_eq!(
        manifest["supported_profile"]["copper_graphic_keepout_count"],
        2
    );
}

#[test]
fn a_turned_concave_pad_keeps_its_keepouts_on_its_copper() {
    // Turned 90 degrees in KiCad (counter-clockwise on screen) on a footprint at
    // 30 degrees. The bar's long end (local x = -0.51) then points down on screen,
    // which is -Y in the DSN, and its short end (0.37) up.
    let board = with_cross_pad()
        .replacen("(at 100 50)", "(at 100 50 30)", 1)
        .replacen("(at -0.5 0)", "(at -0.5 0 120)", 1);
    let pcb = parse_dsn(&export(&board).unwrap().dsn).pcb;
    let (placed, keepouts) = router_copper(&pcb, "konnect_image_R1", "1");
    let copper = |p: (f64, f64)| {
        konnect_sexp::planar::point_in_ring(p, &placed)
            || keepouts
                .iter()
                .any(|k| konnect_sexp::planar::point_in_ring(p, k))
    };
    // Image frame: the pad sits at (-500, 0).
    for (p, expected) in [
        ((-500.0, -450.0), true),         // the long end of the bar
        ((-500.0, 450.0), false),         // past the short end
        ((-500.0, 330.0), true),          // inside the short end
        ((-500.0 - 400.0, 0.0), true),    // one end of the tab, a keepout
        ((-500.0 + 400.0, 0.0), true),    // the other end
        ((-500.0 + 400.0, 200.0), false), // beside the tab, past the bar
    ] {
        assert_eq!(copper(p), expected, "at {p:?}");
    }
    assert!(keepouts.iter().all(|k| k
        .iter()
        .all(|&(x, y)| (x + 500.0).abs() <= 480.0 && y.abs() <= 140.0)));
}

#[test]
fn a_back_side_concave_pad_keeps_its_keepouts_on_its_copper_layer() {
    let board = with_cross_pad()
        .replacen("(layer \"F.Cu\")", "(layer \"B.Cu\")", 1)
        .replacen("(at 100 50)", "(at 100 50 30)", 1)
        .replacen("(at -0.5 0)", "(at -0.5 0 210)", 1)
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
    assert_eq!(pin.rotate, Some(180.0));
    assert!(image
        .keepouts
        .0
        .iter()
        .all(|k| polygon_points(&k.shape).0 == "B.Cu"));
    // The router's copper is the cross of the file, turned half a turn at the pad.
    let (placed, keepouts) = router_copper(&pcb, "konnect_image_R1", "1");
    for x in (-700..=500).step_by(10) {
        for y in (-600..=600).step_by(10) {
            let local = (f64::from(x), f64::from(y));
            if near_cross_edge(local, 8.0) {
                continue;
            }
            let at = (-local.0 - 500.0, -local.1);
            let copper = konnect_sexp::planar::point_in_ring(at, &placed)
                || keepouts
                    .iter()
                    .any(|k| konnect_sexp::planar::point_in_ring(at, k));
            assert_eq!(copper, in_cross(local), "at {local:?}");
        }
    }
}

#[test]
fn a_plated_concave_pad_keeps_its_other_copper_out_on_every_layer() {
    let board = with_cross_pad()
        .replacen("smd custom", "thru_hole custom", 1)
        .replacen(
            "(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")",
            "(drill 0.2) (layers \"*.Cu\" \"*.Mask\")",
            1,
        );
    let pcb = parse_dsn(&export(&board).unwrap().dsn).pcb;
    let image = pcb
        .library
        .images
        .iter()
        .find(|i| i.name == "konnect_image_R1")
        .unwrap();
    let mut layers: Vec<_> = image
        .keepouts
        .0
        .iter()
        .map(|k| polygon_points(&k.shape).0.to_string())
        .collect();
    layers.sort();
    assert_eq!(layers, ["B.Cu", "B.Cu", "F.Cu", "F.Cu"]);
    // The offset moves the copper so that the hole, at the pad's origin, is in
    // a tab: the split keeps the origin in the pin, so the pin is the tab's bar.
    let in_tab = board.replacen("(drill 0.2)", "(drill 0.1 (offset 0 0.36))", 1);
    let pcb = parse_dsn(&export(&in_tab).unwrap().dsn).pcb;
    let [shape, ..] = pin_stack(&pcb, "konnect_image_R1", "1").shapes.as_slice() else {
        panic!("expected pin polygons")
    };
    let corners: BTreeSet<_> = polygon_points(shape)
        .1
        .iter()
        .map(|&(x, y)| (x as i32, y as i32))
        .collect();
    assert_eq!(
        corners,
        BTreeSet::from([(-130, -830), (130, -830), (130, 110), (-130, 110)])
    );
    // A hole wider than that bar is refused: the pin must hold the drill.
    let wide = board.replacen("(drill 0.2)", "(drill 0.3 (offset 0 0.36))", 1);
    let error = format!("{:#}", export(&wide).unwrap_err());
    assert!(
        error.contains("R1-1") && error.contains("enclose the drill"),
        "{error}"
    );
}

fn square(x1: i64, y1: i64, x2: i64, y2: i64) -> Contour {
    vec![
        IntPoint::new(x1, y1),
        IntPoint::new(x2, y1),
        IntPoint::new(x2, y2),
        IntPoint::new(x1, y2),
    ]
}

fn union(rings: &[Contour]) -> Contour {
    let shapes = Overlay::from_subj(rings).overlay(OverlayRule::Subject, FillRule::NonZero);
    let mut ring = shapes[0][0].clone();
    if area2(&ring) < 0 {
        ring.reverse();
    }
    ring
}

#[test]
fn the_split_covers_the_copper_exactly_with_a_convex_pin_around_the_centre() {
    let mm = 1_000_000;
    let shapes = [
        // An L, a T, a U and a notched square, all around the origin, each
        // with the area in mm^2 of its largest convex pin a cut can leave: the
        // L's upright, the T's stem (its bar's lower edge runs through the
        // centre), the U's base, and the square below the notch's left edge.
        (
            union(&[
                square(-mm, -mm, 2 * mm, mm / 2),
                square(-mm, -mm, mm / 2, 3 * mm),
            ]),
            6.0,
        ),
        (
            union(&[
                square(-2 * mm, 0, 2 * mm, mm),
                square(-mm / 2, -2 * mm, mm / 2, mm),
            ]),
            3.0,
        ),
        (
            union(&[
                square(-2 * mm, -mm, 2 * mm, mm / 2),
                square(-2 * mm, -mm, -mm, 2 * mm),
                square(mm, -mm, 2 * mm, 2 * mm),
            ]),
            6.0,
        ),
        (
            vec![
                IntPoint::new(-mm, -mm),
                IntPoint::new(mm, -mm),
                IntPoint::new(mm, mm),
                IntPoint::new(mm / 3, mm / 2),
                IntPoint::new(-mm, mm),
            ],
            3.25,
        ),
    ];
    for (copper, best_mm2) in shapes {
        let (pin, rest) = split_concave(&copper, IntPoint::new(0, 0)).unwrap();
        assert!(pin.is_convex() && area2(&pin) > 0, "{pin:?}");
        let area_mm2 = area2(&pin) as f64 / 2.0 / 1e12;
        assert!(
            (area_mm2 - best_mm2).abs() < 1e-3,
            "{area_mm2} mm2, not {best_mm2}: {pin:?}"
        );
        assert!(pin.as_slice().contains_point(IntPoint::new(0, 0)));
        assert!(
            area2_outside(&pin, &copper) <= i128::from(40 * mm),
            "{pin:?}"
        );
        let total: i128 = area2(&pin) + rest.iter().map(|r| area2(r)).sum::<i128>();
        let slack = i128::from(80 * mm);
        assert!((total - area2(&copper)).abs() <= slack, "{copper:?}");
        assert!(!rest.is_empty());
    }
}

#[test]
fn no_cut_within_ten_micrometres_of_the_pads_centre_is_made() {
    // An L whose inner corner is 5 um from the pad's centre: every cut that would
    // free the pin from the notch passes that close, so the pad is refused.
    let mm = 1_000_000;
    let copper = union(&[square(-mm, -mm, mm, 5_000), square(-mm, -mm, 5_000, mm)]);
    let error = format!(
        "{:#}",
        split_concave(&copper, IntPoint::new(0, 0)).unwrap_err()
    );
    assert!(error.contains("holds the pad's centre"), "{error}");
    // 15 um away, the same L splits.
    let copper = union(&[square(-mm, -mm, mm, 15_000), square(-mm, -mm, 15_000, mm)]);
    split_concave(&copper, IntPoint::new(0, 0)).unwrap();
}

#[test]
fn a_slanted_split_leaves_no_slivers_along_its_cuts() {
    // The cross turned to every whole degree: each slanted cut crosses the grid
    // at a rounded point, which must not leave thin copper along the pin as
    // extra keepouts.
    let cross = [
        (-130, 470),
        (-130, 250),
        (-510, 250),
        (-510, -250),
        (-130, -250),
        (-130, -470),
        (130, -470),
        (130, -250),
        (370, -250),
        (370, 250),
        (130, 250),
        (130, 470),
    ];
    for degrees in 1..90 {
        let (sin, cos) = f64::from(degrees).to_radians().sin_cos();
        let mut copper: Contour = cross
            .iter()
            .map(|&(x, y)| {
                let (x, y) = (f64::from(x) * 1000.0, f64::from(y) * 1000.0);
                IntPoint::new(
                    (x * cos - y * sin).round() as i64,
                    (x * sin + y * cos).round() as i64,
                )
            })
            .collect();
        if area2(&copper) < 0 {
            copper.reverse();
        }
        let (pin, rest) = split_concave(&copper, IntPoint::new(0, 0)).unwrap();
        // The pin is the bar, 0.88 x 0.5 mm (its cut points may add vertices on
        // its edges), and the rest the tab's two ends, 0.26 x 0.22 mm each.
        let bar = 2 * 880_000_i128 * 500_000;
        assert!(
            pin.is_convex() && (area2(&pin) - bar).abs() < bar / 1000,
            "{degrees} degrees: {pin:?}"
        );
        assert_eq!(rest.len(), 2, "{degrees} degrees: {rest:?}");
        for ring in &rest {
            let tab = 2 * 260_000_i128 * 220_000;
            assert!(
                (area2(ring) - tab).abs() < tab / 1000,
                "{degrees} degrees: {ring:?}"
            );
        }
    }
}

#[test]
fn growing_a_pin_moves_every_edge_outwards() {
    // A 2 x 1 mm rectangle grown by 2 nm, and the same rectangle turned 30
    // degrees: every edge lies 2 nm further out, so the rest of a split never
    // keeps a sliver along the pin.
    let rect = square(-1_000_000, -500_000, 1_000_000, 500_000);
    assert_eq!(
        grown(&rect, 2.0),
        square(-1_000_002, -500_002, 1_000_002, 500_002)
    );
    let (sin, cos) = 30f64.to_radians().sin_cos();
    let turned: Contour = rect
        .iter()
        .map(|p| {
            let (x, y) = (p.x as f64, p.y as f64);
            IntPoint::new(
                (x * cos - y * sin).round() as i64,
                (x * sin + y * cos).round() as i64,
            )
        })
        .collect();
    let bigger = grown(&turned, 2.0);
    for (&p, &q) in turned.iter().zip(turned.iter().cycle().skip(1)) {
        let offset = bigger
            .iter()
            .map(|&v| -side(p, q, v) as f64 / ((q.x - p.x) as f64).hypot((q.y - p.y) as f64))
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            (offset - 2.0).abs() < 1.0,
            "edge {p:?}-{q:?} moved {offset} nm"
        );
    }
}
