//! Synthetic geometry and public-contract checks for the read-only S7 planner.
use serde_json::json;
use serde_json::Value;
use std::sync::Arc;

fn uid(n: usize) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

fn track(n: usize, a: (f64, f64), b: (f64, f64), net: usize) -> String {
    format!(
        "(segment (start {} {}) (end {} {}) (width 0.2) (layer \"F.Cu\") (net {net}) (uuid {}))",
        a.0,
        a.1,
        b.0,
        b.1,
        uid(n)
    )
}

fn pad(n: usize, at: (f64, f64), net: usize, layer: &str) -> String {
    format!("(footprint \"Test:Pad\" (layer \"F.Cu\") (at 0 0) (uuid {}) (property \"Reference\" \"P{n}\" (at 0 0) (layer \"F.SilkS\")) (pad \"1\" smd circle (at {} {}) (size 0.8 0.8) (layers \"{layer}\") (net {net} \"{}\") (uuid {})))", uid(n + 1000), at.0, at.1, ["", "GND", "SIG", "OTHER"][net], uid(n))
}

fn fill(n: usize, layer: &str, net: usize, points: &[(f64, f64)]) -> String {
    let xy = points
        .iter()
        .map(|p| format!("(xy {} {})", p.0, p.1))
        .collect::<String>();
    format!("(zone (net {net}) (net_name \"{}\") (layer \"{layer}\") (uuid {}) (hatch edge 0.5) (connect_pads (clearance 0.2)) (min_thickness 0.1) (fill yes (thermal_gap 0.3) (thermal_bridge_width 0.3)) (polygon (pts {xy})) (filled_polygon (layer \"{layer}\") (pts {xy})))", ["", "GND", "SIG", "OTHER"][net], uid(n))
}

pub(super) struct Fixture {
    dir: tempfile::TempDir,
    pub(super) board: std::path::PathBuf,
}

impl Fixture {
    pub(super) fn new(items: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let board = dir.path().join("test.kicad_pcb");
        std::fs::write(&board, format!("(kicad_pcb (version 20241229) (generator pcbnew) (general (thickness 1.6)) (layers (0 \"F.Cu\" signal) (31 \"B.Cu\" signal) (44 \"Edge.Cuts\" user)) (setup (pad_to_mask_clearance 0)) (net 0 \"\") (net 1 \"GND\") (net 2 \"SIG\") (net 3 \"OTHER\") (gr_rect (start 0 0) (end 30 30) (stroke (width 0.05) (type default)) (fill none) (layer \"Edge.Cuts\") (uuid {})) {items})",uid(9999))).unwrap();
        std::fs::write(board.with_extension("kicad_pro"), json!({"board":{"design_settings":{"rules":{"min_clearance":0.2,"min_track_width":0.1,"min_copper_edge_clearance":0.2,"min_hole_clearance":0.2,"min_hole_to_hole":0.2,"min_via_annular_width":0.1,"min_via_diameter":0.6,"min_through_hole_diameter":0.3}}},"net_settings":{"classes":[{"name":"Default","clearance":0.2,"track_width":0.2,"via_diameter":0.6,"via_drill":0.3}]}}).to_string()).unwrap();
        Self { dir, board }
    }

    async fn run(&self, mut args: Value) -> Value {
        args["board"] = json!(self.board);
        let tool = crate::router::registry::tools_for("pcb_routing")
            .unwrap()
            .into_iter()
            .find(|t| t.name == "plan_route_refinement")
            .expect("missing S7 planner");
        let file_count = std::fs::read_dir(self.dir.path()).unwrap().count();
        let before: Vec<_> = [&self.board, &self.board.with_extension("kicad_pro")]
            .into_iter()
            .map(|p| {
                (
                    std::fs::read(p).unwrap(),
                    std::fs::metadata(p).unwrap().modified().unwrap(),
                )
            })
            .collect();
        let ctx = Arc::new(super::ToolContext::new(
            super::ServerConfig::default(),
            Arc::new(crate::router::ToolRouter::new()),
        ));
        let r = (tool.handler)(&args, ctx).await.unwrap();
        let after: Vec<_> = [&self.board, &self.board.with_extension("kicad_pro")]
            .into_iter()
            .map(|p| {
                (
                    std::fs::read(p).unwrap(),
                    std::fs::metadata(p).unwrap().modified().unwrap(),
                )
            })
            .collect();
        assert_eq!(before, after, "planning changed source bytes or metadata");
        assert_eq!(
            std::fs::read_dir(self.dir.path()).unwrap().count(),
            file_count,
            "planning wrote files"
        );
        let crate::mcp::protocol::ToolContent::Text { text } = &r.content[0] else {
            panic!("not text")
        };
        let body: Value = serde_json::from_str(text).unwrap();
        assert_eq!(body["mutated"], false, "{body}");
        body
    }
}

pub(super) fn dogleg() -> String {
    [
        pad(1, (4., 6.), 2, "F.Cu"),
        pad(2, (6., 10.), 2, "F.Cu"),
        track(11, (4., 6.), (6., 6.), 2),
        track(12, (6., 6.), (6., 10.), 2),
    ]
    .concat()
}

/// A plan without its wall-clock readings; everything else must repeat exactly.
fn stable(mut value: Value) -> Value {
    value.as_object_mut().unwrap().remove("elapsed_ms");
    let used = value["effort"]["used"].as_object_mut().unwrap();
    assert!(used.remove("elapsed_ms").is_some() && used.remove("preparation_ms").is_some());
    value
}

fn custom_budget(candidates: usize) -> Value {
    json!({"max_candidates":candidates,"max_pair_checks":2_000_000,"max_geometry_work":100_000_000,"max_elapsed_ms":30_000})
}

#[tokio::test]
async fn route_refine_effort_is_explicit_and_levels_change_only_resource_limits() {
    let f = Fixture::new(&dogleg());
    let mut limits = Vec::new();
    for level in ["quick", "standard", "deep"] {
        let r = f.run(json!({"kinds":["chamfer"],"effort":level})).await;
        assert_eq!(r["effort"]["effective_level"], level, "{r}");
        assert_eq!(r["effort"]["requested"], level);
        assert_eq!(r["proposals"].as_array().unwrap().len(), 1);
        assert_eq!(r["parameters"]["geometry_margin_mm"], 0.01);
        limits.push(r["effort"]["limits"]["max_geometry_work"].as_u64().unwrap());
        assert!(r["elapsed_ms"].is_u64());
    }
    assert!(limits[0] < limits[1] && limits[1] < limits[2]);
}

#[tokio::test]
async fn route_refine_custom_budget_has_no_ambiguous_tier_precedence() {
    let f = Fixture::new(&dogleg());
    let budget = custom_budget(1);
    let r = f.run(json!({"kinds":["chamfer"],"budget":budget})).await;
    assert_eq!(r["effort"]["effective_level"], "custom", "{r}");
    assert_eq!(r["effort"]["limits"], budget);
    assert!(r["coverage"]["candidates_considered"].as_u64().unwrap() <= 1);
    for args in [
        json!({"effort":"quick","budget":budget}),
        json!({"effort":"unbounded"}),
        json!({"budget":{}}),
    ] {
        assert_eq!(f.run(args).await["status"], "blocked");
    }
}

#[tokio::test]
async fn route_refine_chamfer_preserves_track_ids_width_and_pad_contact() {
    let f = Fixture::new(&dogleg());
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    let p = &r["proposals"][0];
    assert_eq!(p["kind"], "chamfer");
    let ops = p["operations"].as_array().unwrap();
    assert_eq!(
        ops.iter().filter(|x| x["tool"] == "modify_trace").count(),
        2
    );
    let batch = ops.iter().find(|x| x["tool"] == "route_batch").unwrap();
    assert_eq!(
        batch["arguments"]["tracks"][0]["points"],
        json!([{"x":5.5,"y":6.0},{"x":6.0,"y":6.5}])
    );
    assert_eq!(batch["arguments"]["tracks"][0]["width"], 0.2);
    assert_eq!(
        stable(f.run(json!({"kinds":["chamfer"]})).await),
        stable(r),
        "same snapshot must yield the same plan"
    );
}

#[tokio::test]
async fn route_refine_blocks_an_obstacle_in_the_chamfer_envelope() {
    let f = Fixture::new(&(dogleg() + &pad(3, (5.75, 6.25), 3, "F.Cu")));
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert!(!r["blocked"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn route_refine_does_not_shorten_over_a_same_net_branch_or_locked_track() {
    for items in [
        dogleg() + &track(13, (5.8, 6.), (5.8, 4.), 2),
        dogleg().replace("(segment (start 4 6)", "(segment (locked yes) (start 4 6)"),
    ] {
        let f = Fixture::new(&items);
        let r = f.run(json!({"kinds":["chamfer"]})).await;
        if items.contains("locked yes") {
            assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
            assert_eq!(r["coverage"]["blocked_total"], 1, "{r}");
        } else {
            // The corner is still repaired, with a setback that keeps the branch.
            assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
        }
        for p in r["proposals"].as_array().unwrap() {
            for op in p["operations"].as_array().unwrap() {
                if op["arguments"]["uuid"] == uid(11) {
                    // The 0.2-mm-wide branch may touch the new diagonal before its
                    // centreline reaches the old endpoint. For this hand-drawn 45°
                    // geometry, endpoint-to-diagonal distance is dx / sqrt(2),
                    // and the two track half-widths sum to 0.2 mm.
                    let dx = (5.8 - op["arguments"]["x2"].as_f64().unwrap_or(6.)).max(0.);
                    assert!(dx / 2f64.sqrt() <= 0.2, "lost copper attachment: {r}");
                    assert!(!items.contains("locked yes"), "modified locked track: {r}");
                }
            }
        }
    }
}

#[tokio::test]
async fn route_refine_stitches_disconnected_same_net_fills_without_writing() {
    let square = [(2., 2.), (18., 2.), (18., 18.), (2., 18.)];
    let f = Fixture::new(
        &[
            fill(30, "F.Cu", 1, &square),
            fill(31, "B.Cu", 1, &square),
            pad(1, (4., 4.), 1, "F.Cu"),
            pad(2, (16., 16.), 1, "B.Cu"),
        ]
        .concat(),
    );
    let r = f.run(json!({"kinds":["stitching"]})).await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    let v = &r["proposals"][0]["operations"][0]["arguments"]["vias"][0];
    assert_eq!(v["net_name"], "GND");
    assert_eq!(v["pad_size"], 0.6);
    assert_eq!(v["drill"], 0.3);
    assert!(v["x"].as_f64().unwrap() > 2.31);
    assert!(v["x"].as_f64().unwrap() < 17.69);
}

#[tokio::test]
async fn route_refine_reterminates_the_acute_leg_perpendicularly() {
    let f = Fixture::new(
        &[
            pad(1, (4., 10.), 2, "F.Cu"),
            pad(2, (16., 10.), 2, "F.Cu"),
            pad(3, (6., 6.), 2, "F.Cu"),
            track(11, (4., 10.), (10., 10.), 2),
            track(12, (10., 10.), (16., 10.), 2),
            track(13, (6., 6.), (10., 10.), 2),
        ]
        .concat(),
    );
    let r = f.run(json!({"kinds":["t_junction"]})).await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    let p = &r["proposals"][0];
    let op = p["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["tool"] == "route_batch")
        .unwrap();
    let points = op["arguments"]["tracks"][0]["points"].as_array().unwrap();
    assert_eq!(
        points[0]["x"], points[1]["x"],
        "branch into horizontal trunk must be perpendicular: {r}"
    );
    assert_ne!(points[0]["y"], points[1]["y"]);
    assert_eq!(points[1]["y"], 10.0);
}

#[tokio::test]
async fn route_refine_refuses_unsupported_rules_and_invalid_options_read_only() {
    for args in [
        json!({"apply":true}),
        json!({"grid_step_mm":0}),
        json!({"max_candidates":0}),
        json!({"budget":{"max_candidates":0,"max_pair_checks":2_000_000,"max_geometry_work":100_000_000,"max_elapsed_ms":30_000}}),
        json!({"max_proposals":0}),
        json!({"max_proposals":1.5}),
        json!({"kinds":["anything"]}),
        json!({"nets":["MISSING"]}),
    ] {
        let f = Fixture::new(&dogleg());
        let r = f.run(args).await;
        assert_eq!(r["status"], "blocked", "{r}");
        assert!(r["proposals"].as_array().unwrap().is_empty());
    }
    let f = Fixture::new(&dogleg());
    let path = f.board.with_extension("kicad_pro");
    let mut p: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    p["net_settings"]["classes"][0]["clearance"] = json!(-0.2);
    std::fs::write(path, p.to_string()).unwrap();
    let r = f.run(json!({})).await;
    assert_eq!(r["status"], "blocked", "{r}");
}

/// KiCad saves `null` for a project with no explicit class assignments or
/// patterns. That means none; any other malformed value still refuses the plan.
#[tokio::test]
async fn route_refine_reads_a_project_whose_class_assignments_are_null() {
    let edit = |f: &Fixture, key: &str, value: Value| {
        let path = f.board.with_extension("kicad_pro");
        let mut p: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        p["net_settings"][key] = value;
        std::fs::write(path, p.to_string()).unwrap();
    };
    let f = Fixture::new(&dogleg());
    edit(&f, "netclass_assignments", Value::Null);
    edit(&f, "netclass_patterns", Value::Null);
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    for (key, bad) in [
        ("netclass_assignments", json!([])),
        ("netclass_assignments", json!("Default")),
        ("netclass_patterns", json!({})),
        ("netclass_patterns", json!("GND")),
    ] {
        let f = Fixture::new(&dogleg());
        edit(&f, key, bad);
        let r = f.run(json!({"kinds":["chamfer"]})).await;
        assert_eq!(r["status"], "blocked", "{key}: {r}");
        assert!(r["proposals"].as_array().unwrap().is_empty());
    }
}

#[tokio::test]
async fn route_refine_never_joins_different_reference_nets_or_two_floating_islands() {
    let square = [(2., 2.), (18., 2.), (18., 18.), (2., 18.)];
    for items in [
        [
            fill(30, "F.Cu", 1, &square),
            fill(31, "B.Cu", 3, &square),
            pad(1, (4., 4.), 1, "F.Cu"),
            pad(2, (16., 16.), 3, "B.Cu"),
        ]
        .concat(),
        [fill(30, "F.Cu", 1, &square), fill(31, "B.Cu", 1, &square)].concat(),
    ] {
        let f = Fixture::new(&items);
        let r = f.run(json!({"kinds":["stitching"]})).await;
        assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    }
}

#[tokio::test]
async fn route_refine_checks_foreign_copper_on_intermediate_via_layers() {
    let square = [(2., 2.), (18., 2.), (18., 18.), (2., 18.)];
    let f = Fixture::new(
        &[
            fill(30, "F.Cu", 1, &square),
            fill(31, "B.Cu", 1, &square),
            fill(32, "In1.Cu", 3, &square),
            pad(1, (4., 4.), 1, "F.Cu"),
            pad(2, (16., 16.), 1, "B.Cu"),
        ]
        .concat(),
    );
    let board = std::fs::read_to_string(&f.board).unwrap().replace(
        "(31 \"B.Cu\" signal)",
        "(2 \"In1.Cu\" signal) (4 \"In2.Cu\" signal) (31 \"B.Cu\" signal)",
    );
    std::fs::write(&f.board, board).unwrap();
    let r = f.run(json!({"kinds":["stitching"]})).await;
    assert!(
        r["proposals"].as_array().unwrap().is_empty(),
        "via crosses foreign inner-plane copper: {r}"
    );
}

#[tokio::test]
async fn route_refine_multiple_candidates_have_stable_order_and_ids() {
    let f = Fixture::new(
        &(dogleg()
            + &[
                pad(3, (14., 6.), 2, "F.Cu"),
                pad(4, (16., 10.), 2, "F.Cu"),
                track(21, (14., 6.), (16., 6.), 2),
                track(22, (16., 6.), (16., 10.), 2),
            ]
            .concat()),
    );
    let first = f.run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(first["proposals"].as_array().unwrap().len(), 2);
    for _ in 0..12 {
        assert_eq!(
            stable(f.run(json!({"kinds":["chamfer"]})).await),
            stable(first.clone())
        );
    }
}

/// The pad-anchored fill pair of the stitching tests, with the hole test's
/// bridged B.Cu fill: the planner must stitch them without using the hole.
#[tokio::test]
async fn route_refine_stitch_avoids_a_fill_hole_and_every_pad() {
    let square = [(2., 2.), (18., 2.), (18., 18.), (2., 18.)];
    let hole = [
        (2., 2.),
        (18., 2.),
        (18., 18.),
        (2., 18.),
        (2., 2.),
        (9., 9.),
        (9., 11.),
        (11., 11.),
        (11., 9.),
        (9., 9.),
        (2., 2.),
    ];
    for at in [(10., 10.), (8.8, 10.), (8.6, 10.)] {
        let f = Fixture::new(
            &[
                fill(30, "F.Cu", 1, &square),
                fill(31, "B.Cu", 1, &hole),
                pad(1, at, 1, "F.Cu"),
                pad(2, (16., 16.), 1, "B.Cu"),
            ]
            .concat(),
        );
        let r = f.run(json!({"kinds":["stitching"]})).await;
        assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{at:?}: {r}");
        let v = &r["proposals"][0]["operations"][0]["arguments"]["vias"][0];
        let (x, y) = (v["x"].as_f64().unwrap(), v["y"].as_f64().unwrap());
        let radius = v["pad_size"].as_f64().unwrap() / 2.;
        assert!(
            (x - 10.).abs().max((y - 10.).abs()) >= 1. + radius + 0.01,
            "annulus over the fill hole at {x},{y}: {r}"
        );
        // The round pads are 0.8 mm across; the stitch must not overlap either.
        for centre in [at, (16., 16.)] {
            assert!(
                (x - centre.0).hypot(y - centre.1) > 0.4 + radius,
                "stitch at {x},{y} lands on the pad at {centre:?}: {r}"
            );
        }
        assert_eq!(r["status"], "planned", "{r}");
    }
}

#[tokio::test]
async fn route_refine_cannot_claim_complete_geometry_after_ignoring_a_copper_primitive() {
    for extra in [
        format!("(unknown_copper (layer \"F.Cu\") (uuid {}))", uid(800)),
        fill(30, "F.Cu", 1, &[(2., 2.), (18., 2.), (18., 18.), (2., 18.)])
            .replace("(filled_polygon", "(unreadable_polygon"),
        format!(
            "(gr_text \"COPPER\" (at 20 20) (layer \"F.Cu\") (uuid {}))",
            uid(801)
        ),
    ] {
        let f = Fixture::new(&(dogleg() + &extra));
        let r = f.run(json!({"kinds":["chamfer"]})).await;
        assert_eq!(r["status"], "blocked", "{r}");
        assert!(r["proposals"].as_array().unwrap().is_empty());
    }
}

#[tokio::test]
async fn route_refine_reports_candidate_exhaustion_as_partial() {
    let f = Fixture::new(&(dogleg() + &pad(3, (5.75, 6.25), 3, "F.Cu")));
    let r = f
        .run(json!({"kinds":["chamfer"],"budget":custom_budget(1)}))
        .await;
    assert_eq!(r["status"], "partial");
    assert_eq!(r["truncated"], true);
    assert_eq!(r["coverage"]["complete"], false);
}

#[tokio::test]
async fn route_refine_reports_ground_components_with_no_overlap_as_unresolved() {
    let f = Fixture::new(
        &[
            fill(30, "F.Cu", 1, &[(2., 2.), (8., 2.), (8., 8.), (2., 8.)]),
            fill(
                31,
                "B.Cu",
                1,
                &[(12., 12.), (18., 12.), (18., 18.), (12., 18.)],
            ),
            pad(1, (4., 4.), 1, "F.Cu"),
            pad(2, (16., 16.), 1, "B.Cu"),
        ]
        .concat(),
    );
    let r = f.run(json!({"kinds":["stitching"]})).await;
    assert!(r["proposals"].as_array().unwrap().is_empty());
    assert_eq!(
        r["status"], "partial",
        "missing ground repair must remain visible: {r}"
    );
    assert!(!r["blocked"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn route_refine_preserves_legal_track_contact_to_its_own_plated_pad() {
    let items = dogleg()
        .replace("smd circle", "thru_hole circle")
        .replace("(size 0.8 0.8)", "(size 0.8 0.8) (drill 0.3)")
        .replace("(layers \"F.Cu\")", "(layers \"*.Cu\")");
    let f = Fixture::new(&items);
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(
        r["proposals"].as_array().unwrap().len(),
        1,
        "legal same-net plated pad contact was rejected: {r}"
    );
}

#[tokio::test]
async fn route_refine_rejects_ambiguous_net_codes_before_planning() {
    let f = Fixture::new(&(dogleg() + "(net 1 \"OTHER\")"));
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(r["status"], "blocked", "{r}");
}

#[tokio::test]
async fn route_refine_does_not_silently_ignore_conditional_custom_rules() {
    let f = Fixture::new(&dogleg());
    std::fs::write(f.board.with_extension("kicad_dru"),"(version 1) (rule \"Special\" (condition \"A.NetName == 'SIG'\") (constraint clearance (min 0.8mm)))").unwrap();
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(r["status"], "blocked", "{r}");
}

#[tokio::test]
async fn route_refine_respects_zone_specific_clearance() {
    let zone = fill(30, "F.Cu", 3, &[(5.5, 5.), (6., 5.), (6., 5.5), (5.5, 5.5)])
        .replace("(clearance 0.2)", "(clearance 0.6)");
    let f = Fixture::new(&(dogleg() + &zone));
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert!(
        r["proposals"].as_array().unwrap().is_empty(),
        "ignored zone clearance: {r}"
    );
}

#[tokio::test]
async fn route_refine_refuses_curves_exceeding_the_claimed_approximation_bound() {
    let circle=format!("(gr_circle (center 500 500) (end 1000 500) (stroke (width 0.2) (type default)) (fill none) (layer \"B.Cu\") (uuid {}))",uid(500));
    let f = Fixture::new(&(dogleg() + &circle));
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(
        r["status"], "blocked",
        "radius500 chord error exceeds0.01mm: {r}"
    );
}

#[tokio::test]
async fn route_refine_uses_the_actual_junction_not_its_rounded_audit_location() {
    let f = Fixture::new(
        &[
            pad(1, (4.0004, 6.0004), 2, "F.Cu"),
            pad(2, (6.0004, 10.0004), 2, "F.Cu"),
            track(11, (4.0004, 6.0004), (6.0004, 6.0004), 2),
            track(12, (6.0004, 6.0004), (6.0004, 10.0004), 2),
        ]
        .concat(),
    );
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(
        r["proposals"].as_array().unwrap().len(),
        1,
        "audit rounds locations to0.001mm; original copper is an exact right angle: {r}"
    );
}

#[tokio::test]
async fn route_refine_width_transitions_preserve_existing_widths() {
    let wider = track(12, (6., 6.), (6., 10.), 2).replace("(width 0.2)", "(width 0.4)");
    let f = Fixture::new(
        &[
            pad(1, (4., 6.), 2, "F.Cu"),
            pad(2, (6., 10.), 2, "F.Cu"),
            track(11, (4., 6.), (6., 6.), 2),
            wider,
        ]
        .concat(),
    );
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    let p = &r["proposals"][0];
    let widths: Vec<_> = p["preconditions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["width"].as_f64().unwrap())
        .collect();
    assert!(widths.contains(&0.2) && widths.contains(&0.4));
    for op in p["operations"].as_array().unwrap() {
        if op["tool"] == "route_batch" {
            assert_eq!(op["arguments"]["tracks"][0]["width"], 0.4);
        } else {
            assert!(op["arguments"].get("width").is_none());
        }
    }
}

#[tokio::test]
async fn route_refine_t_branch_keeps_its_width_when_the_trunk_is_wider() {
    let trunk = [
        track(11, (4., 10.), (10., 10.), 2),
        track(12, (10., 10.), (16., 10.), 2),
    ]
    .concat()
    .replace("(width 0.2)", "(width 0.4)");
    let f = Fixture::new(
        &[
            pad(1, (4., 10.), 2, "F.Cu"),
            pad(2, (16., 10.), 2, "F.Cu"),
            pad(3, (6., 6.), 2, "F.Cu"),
            trunk,
            track(13, (6., 6.), (10., 10.), 2),
        ]
        .concat(),
    );
    let r = f.run(json!({"kinds":["t_junction"]})).await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    for op in r["proposals"][0]["operations"].as_array().unwrap() {
        if op["tool"] == "route_batch" {
            assert_eq!(op["arguments"]["tracks"][0]["width"], 0.2);
        }
    }
}

/// Every edge cut into `parts`, so a fill is large enough to be indexed.
fn dense(points: &[(f64, f64)], parts: usize) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for i in 0..points.len() {
        let (a, b) = (points[i], points[(i + 1) % points.len()]);
        for k in 0..parts {
            let t = k as f64 / parts as f64;
            out.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
        }
    }
    out
}

/// A signal that changes layer at (10, 10). Its reference is the ground fill
/// on the other side each time, and the only ground via is 8.5 mm away: the
/// fills are one copper group, yet the return current has no path at the via.
pub(super) fn reference_change() -> String {
    let front = [
        (2., 11.5),
        (11.5, 11.5),
        (11.5, 2.),
        (18., 2.),
        (18., 18.),
        (2., 18.),
    ];
    let back = [
        (2., 2.),
        (8.5, 2.),
        (8.5, 11.5),
        (18., 11.5),
        (18., 18.),
        (2., 18.),
    ];
    let via = |n: usize, at: (f64, f64), net: usize| {
        format!(
            "(via (at {} {}) (size 0.6) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") (net {net}) (uuid {}))",
            at.0,
            at.1,
            uid(n)
        )
    };
    [
        fill(30, "F.Cu", 1, &dense(&front, 8)),
        fill(31, "B.Cu", 1, &dense(&back, 8)),
        pad(1, (15., 15.), 1, "F.Cu"),
        via(50, (16., 16.), 1),
        pad(2, (4., 10.), 2, "F.Cu"),
        pad(3, (16., 10.), 2, "B.Cu"),
        track(11, (4., 10.), (10., 10.), 2),
        track(12, (10., 10.), (16., 10.), 2).replace("\"F.Cu\"", "\"B.Cu\""),
        via(51, (10., 10.), 2),
    ]
    .concat()
}

/// The copper groups are already joined, so only the audit's unstitched
/// layer change can ask for this via. It must land near the signal via,
/// inside both ground fills.
#[tokio::test]
async fn route_refine_stitches_an_unstitched_reference_change_between_layers() {
    let f = Fixture::new(&reference_change());
    let r = f.run(json!({"kinds":["stitching"]})).await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    let v = &r["proposals"][0]["operations"][0]["arguments"]["vias"][0];
    assert_eq!(v["net_name"], "GND");
    let (x, y) = (v["x"].as_f64().unwrap(), v["y"].as_f64().unwrap());
    assert!(
        (x - 10.).hypot(y - 10.) <= 3.0,
        "too far from the layer change: {r}"
    );
    assert!(
        (2.31..=17.69).contains(&x) && (11.81..=17.69).contains(&y),
        "outside the band both fills share: {r}"
    );
    assert_eq!(r["status"], "planned", "{r}");
    assert_eq!(r["coverage"]["phases"][0]["state"], "complete");
    // Without that layer change there is nothing to stitch.
    let without = reference_change().replace(&uid(51), &uid(52)).replace(
        &track(12, (10., 10.), (16., 10.), 2).replace("\"F.Cu\"", "\"B.Cu\""),
        "",
    );
    let r = Fixture::new(&without)
        .run(json!({"kinds":["stitching"]}))
        .await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
}

fn connector(proposal: &Value) -> &Value {
    let batch = proposal["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["tool"] == "route_batch")
        .unwrap();
    &batch["arguments"]["tracks"][0]["points"]
}

/// Autorouted copper sits exactly at the minimum clearance. A leg that already
/// ran 0.2 mm from a foreign track must not make its own corner unrepairable.
#[tokio::test]
async fn route_refine_repairs_a_corner_whose_leg_already_runs_at_minimum_clearance() {
    let f = Fixture::new(&(dogleg() + &track(40, (4.8, 5.6), (7.5, 5.6), 3)));
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    let p = &r["proposals"][0];
    assert_eq!(*connector(p), json!([{"x":5.5,"y":6.0},{"x":6.0,"y":6.5}]));
    assert_eq!(p["checks"]["saved_clearances_inherited"], 1, "{r}");
    assert_eq!(r["status"], "planned", "{r}");
}

/// The waiver is an inheritance, not a smaller margin: a connector that would
/// come nearer to foreign copper than the saved legs were falls back to a
/// setback that keeps the whole margin.
#[tokio::test]
async fn route_refine_inherited_clearance_never_lets_copper_move_closer() {
    let near = track(40, (5.4, 6.6), (5.35, 6.65), 3).replace("(width 0.2)", "(width 0.38)");
    let f = Fixture::new(&(dogleg() + &near));
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    let p = &r["proposals"][0];
    assert_eq!(
        *connector(p),
        json!([{"x":5.75,"y":6.0},{"x":6.0,"y":6.25}]),
        "the full setback would sit 0.205 mm from copper the legs kept 0.31 mm from: {r}"
    );
    assert_eq!(p["checks"]["saved_clearances_inherited"], 0, "{r}");
}

#[tokio::test]
async fn route_refine_lists_one_unresolved_row_per_junction_and_counts_every_refusal() {
    let f = Fixture::new(&(dogleg() + &pad(3, (5.75, 6.25), 3, "F.Cu")));
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    let blocked = r["blocked"].as_array().unwrap();
    assert_eq!(
        blocked.len(),
        1,
        "four setbacks of one corner are one finding: {r}"
    );
    assert_eq!(blocked[0]["kind"], "chamfer");
    assert_eq!(blocked[0]["objects"], json!([uid(11), uid(12)]));
    assert!(blocked[0]["reason"]
        .as_str()
        .unwrap()
        .starts_with("clearance to "));
    let coverage = &r["coverage"];
    assert_eq!(coverage["blocked_total"], 1);
    assert_eq!(coverage["blocked_listed"], 1);
    assert_eq!(coverage["candidates_considered"], 4);
    assert_eq!(coverage["candidates_rejected"], json!({"clearance":4}));
    assert_eq!(
        coverage["complete"], true,
        "every candidate was examined: {r}"
    );
    assert_eq!(r["exhaustion"], json!([]));
    assert_eq!(
        r["status"], "partial",
        "an unresolved corner is not a finished plan"
    );
}

/// 210 bends the planner does not handle fill the unresolved list. The corner
/// it can repair sorts last and must still be found, with the cap reported.
#[tokio::test]
async fn route_refine_a_full_unresolved_list_is_reported_and_never_ends_the_search() {
    let mut items = dogleg();
    for row in 0..14 {
        for column in 0..15 {
            let n = 2000 + 2 * (row * 15 + column);
            let (x, y) = (1.0 + 0.3 * column as f64, 12.0 + 1.1 * row as f64);
            items += &track(n, (x, y), (x + 0.2, y), 2);
            items += &track(n + 1, (x, y), (x, y + 0.6), 2);
        }
    }
    let f = Fixture::new(&items);
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(
        r["proposals"][0]["preconditions"][0]["uuid"],
        uid(11),
        "the repairable corner after the full list: {r}"
    );
    let coverage = &r["coverage"];
    assert_eq!(coverage["blocked_total"], 210);
    assert!(coverage["blocked_listed"].as_u64().unwrap() <= 200);
    assert_eq!(
        coverage["blocked_listed"].as_u64().unwrap() as usize,
        r["blocked"].as_array().unwrap().len()
    );
    assert!(r["exhaustion"]
        .as_array()
        .unwrap()
        .contains(&json!("blocked detail limit reached")));
    assert_eq!(coverage["complete"], true, "{r}");
    assert_eq!(phase_states(&r), [("chamfer", "complete")]);
    assert_eq!(r["status"], "partial");
    assert_eq!(r["truncated"], true);
}

/// Each requested kind with how far its search got, in search order.
fn phase_states(plan: &Value) -> Vec<(&str, &str)> {
    plan["coverage"]["phases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["kind"].as_str().unwrap(), p["state"].as_str().unwrap()))
        .collect()
}

/// Forty independent corners do not fit one response. The listed proposals must
/// be an intact prefix of the plan, and the cause must be named.
#[tokio::test]
async fn route_refine_names_the_response_size_limit_and_lists_an_intact_prefix() {
    let mut items = String::new();
    for row in 0..5 {
        for column in 0..8 {
            let i = row * 8 + column;
            let (x, y) = (1.5 + 3.5 * column as f64, 1.5 + 5.5 * row as f64);
            items += &pad(100 + 10 * i, (x, y), 2, "F.Cu");
            items += &pad(101 + 10 * i, (x + 2., y + 4.), 2, "F.Cu");
            items += &track(3000 + 2 * i, (x, y), (x + 2., y), 2);
            items += &track(3001 + 2 * i, (x + 2., y), (x + 2., y + 4.), 2);
        }
    }
    let f = Fixture::new(&items);
    let r = f
        .run(json!({"kinds":["chamfer"],"max_proposals":200}))
        .await;
    let listed = r["proposals"].as_array().unwrap();
    let coverage = &r["coverage"];
    assert_eq!(coverage["proposals_total"], 40, "{r}");
    assert_eq!(coverage["proposals_by_kind"], json!({"chamfer":40}));
    assert_eq!(
        coverage["proposals_listed"].as_u64().unwrap() as usize,
        listed.len()
    );
    assert!(!listed.is_empty() && listed.len() < 40, "{}", listed.len());
    assert!(serde_json::to_vec(&r).unwrap().len() <= 48 * 1024);
    assert_eq!(r["exhaustion"], json!(["response size limit reached"]));
    assert_eq!(coverage["complete"], true, "the search itself finished");
    assert_eq!(r["truncated"], true);
    assert_eq!(r["status"], "partial");
    let mut previous = Value::Null;
    for proposal in listed {
        assert_eq!(
            proposal["depends_on"], previous,
            "listed plan is not a prefix"
        );
        previous = proposal["id"].clone();
    }
}

#[tokio::test]
async fn route_refine_reports_the_kinds_a_stopped_search_never_started() {
    let f = Fixture::new(&(dogleg() + &pad(3, (5.75, 6.25), 3, "F.Cu")));
    let r = f.run(json!({"budget":custom_budget(1)})).await;
    assert_eq!(
        phase_states(&r),
        [
            ("t_junction", "complete"),
            ("chamfer", "stopped"),
            ("stitching", "not_started")
        ],
        "{r}"
    );
    // What each kind spent adds up to the totals; a kind never started spent nothing.
    let phases = r["coverage"]["phases"].as_array().unwrap();
    for (counter, total) in [
        ("candidates", &r["coverage"]["candidates_considered"]),
        ("pair_checks", &r["coverage"]["pair_checks"]),
    ] {
        let spent: u64 = phases.iter().filter_map(|p| p[counter].as_u64()).sum();
        assert_eq!(Some(spent), total.as_u64(), "{counter}: {r}");
    }
    assert!(phases[2].get("geometry_work").is_none(), "{r}");
    assert_eq!(r["exhaustion"], json!(["candidate limit reached"]));
    assert_eq!(r["coverage"]["complete"], false);
    assert_eq!(r["effort"]["used"]["candidates"], 1);
    assert_eq!(r["status"], "partial");
    assert!(r["proposals"].as_array().unwrap().is_empty());
}

/// KiCad treats every member of a locked group as locked.
#[tokio::test]
async fn route_refine_leaves_members_of_a_locked_group_alone() {
    for (group, locked) in [
        ("(locked yes) ", true),
        ("locked ", true),
        ("(locked no) ", false),
        ("", false),
    ] {
        let f = Fixture::new(
            &(dogleg()
                + &format!(
                    "(group \"g\" {group}(uuid {}) (members \"{}\"))",
                    uid(700),
                    uid(12)
                )),
        );
        let r = f.run(json!({"kinds":["chamfer"]})).await;
        if locked {
            assert!(
                r["proposals"].as_array().unwrap().is_empty(),
                "{group}: {r}"
            );
            assert_eq!(
                r["blocked"][0]["reason"], "junction includes locked copper",
                "{group}: {r}"
            );
        } else {
            assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{group}: {r}");
        }
    }
}

/// A two-track acute corner is outside the three supported repairs, and a
/// zero-length track at its apex gives no direction to reterminate along.
#[tokio::test]
async fn route_refine_blocks_acute_corners_it_cannot_reterminate() {
    let corner = [
        pad(1, (4., 10.), 2, "F.Cu"),
        pad(3, (6., 6.), 2, "F.Cu"),
        track(11, (4., 10.), (10., 10.), 2),
        track(13, (6., 6.), (10., 10.), 2),
    ]
    .concat();
    for (extra, reason) in [
        (
            String::new(),
            "unsupported junction: 2 straight track ends meet here, 3 needed",
        ),
        (
            track(14, (10., 10.), (10., 10.), 2),
            "junction includes a degenerate track",
        ),
    ] {
        let f = Fixture::new(&(corner.clone() + &extra));
        let r = f.run(json!({"kinds":["t_junction"]})).await;
        assert_eq!(r["before"]["corners"]["acute_corners"], 1, "{r}");
        assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
        assert_eq!(r["blocked"].as_array().unwrap().len(), 1, "{r}");
        assert_eq!(r["blocked"][0]["reason"], reason, "{r}");
        assert_eq!(r["status"], "partial");
    }
}

/// Every stitch position over a foreign inner plane is refused. Those refusals
/// are counted; they are not hundreds of findings, and the search finishes.
#[tokio::test]
async fn route_refine_counts_refused_stitch_positions_without_listing_each() {
    let square = [(2., 2.), (18., 2.), (18., 18.), (2., 18.)];
    let f = Fixture::new(
        &[
            fill(30, "F.Cu", 1, &square),
            fill(31, "B.Cu", 1, &square),
            fill(32, "In1.Cu", 3, &square),
            pad(1, (4., 4.), 1, "F.Cu"),
            pad(2, (16., 16.), 1, "B.Cu"),
        ]
        .concat(),
    );
    let board = std::fs::read_to_string(&f.board).unwrap().replace(
        "(31 \"B.Cu\" signal)",
        "(2 \"In1.Cu\" signal) (4 \"In2.Cu\" signal) (31 \"B.Cu\" signal)",
    );
    std::fs::write(&f.board, board).unwrap();
    let r = f.run(json!({"kinds":["stitching"]})).await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    let coverage = &r["coverage"];
    let refused = coverage["candidates_rejected"]["clearance"]
        .as_u64()
        .unwrap();
    assert!(refused > 200, "only {refused} refusals: {r}");
    assert!(coverage["candidates_considered"].as_u64().unwrap() >= refused);
    assert_eq!(coverage["blocked_total"], 1, "{r}");
    assert_eq!(r["blocked"][0]["kind"], "stitching");
    assert_eq!(coverage["complete"], true, "{r}");
    assert_eq!(r["exhaustion"], json!([]));
    assert_eq!(r["status"], "partial");
}

/// An unplated hole's pad may carry copper wider than its drill. That ring is
/// an obstacle even where the drill itself is far enough away.
#[tokio::test]
async fn route_refine_keeps_clearance_to_the_copper_of_an_unplated_pad() {
    let hole = format!("(footprint \"Test:Hole\" (layer \"F.Cu\") (at 0 0) (uuid {}) (property \"Reference\" \"H1\" (at 0 0) (layer \"F.SilkS\")) (pad \"\" np_thru_hole circle (at 5.2 6.8) (size 1 1) (drill 0.4) (layers \"*.Cu\" \"*.Mask\") (uuid {})))", uid(1900), uid(900));
    let f = Fixture::new(&(dogleg() + &hole));
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(
        *connector(&r["proposals"][0]),
        json!([{"x":5.75,"y":6.0},{"x":6.0,"y":6.25}]),
        "the full setback passes 0.18 mm from the ring and 0.48 mm from the drill: {r}"
    );
    assert_eq!(r["coverage"]["candidates_rejected"], json!({"clearance":1}));
}

/// One connected ground needs no stitch. The planner must see that from the
/// copper groups instead of walking a grid around every pad.
#[tokio::test]
async fn route_refine_does_not_search_for_stitches_when_nothing_is_left_to_join() {
    let f = Fixture::new(
        &[
            fill(30, "F.Cu", 1, &[(2., 2.), (18., 2.), (18., 18.), (2., 18.)]),
            pad(1, (4., 4.), 1, "F.Cu"),
            pad(2, (16., 16.), 1, "F.Cu"),
        ]
        .concat(),
    );
    let r = f.run(json!({"kinds":["stitching"]})).await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(r["coverage"]["candidates_considered"], 0, "{r}");
    assert_eq!(r["coverage"]["blocked_total"], 0, "{r}");
    assert_eq!(r["coverage"]["complete"], true);
    assert_eq!(r["status"], "planned", "{r}");
}

/// Pure file probe of scratch copies; no KiCad, simulation, Freerouting or IPC.
#[tokio::test]
#[ignore = "requires ROUTE_REFINE_PROBE_INPUTS and ROUTE_REFINE_PROBE_OUTPUT"]
async fn route_refine_file_probe() {
    let input = std::path::PathBuf::from(std::env::var("ROUTE_REFINE_PROBE_INPUTS").unwrap());
    let output = std::path::PathBuf::from(std::env::var("ROUTE_REFINE_PROBE_OUTPUT").unwrap());
    let cases: Value = serde_json::from_str(&std::fs::read_to_string(input).unwrap()).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    let tool = crate::router::registry::tools_for("pcb_routing")
        .unwrap()
        .into_iter()
        .find(|t| t.name == "plan_route_refinement")
        .unwrap();
    let audit_tool = crate::router::registry::tools_for("design_review")
        .unwrap()
        .into_iter()
        .find(|t| t.name == "audit_layout_quality")
        .unwrap();
    let ctx = Arc::new(super::ToolContext::new(
        super::ServerConfig::default(),
        Arc::new(crate::router::ToolRouter::new()),
    ));
    for case in cases.as_array().unwrap() {
        let board = std::path::PathBuf::from(case["board"].as_str().unwrap());
        let before = std::fs::read(&board).unwrap();
        let started = std::time::Instant::now();
        let result = (tool.handler)(&json!({"board":board}), ctx.clone())
            .await
            .unwrap();
        let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
            panic!("missing result")
        };
        let body: Value = serde_json::from_str(text).unwrap();
        assert_eq!(std::fs::read(&board).unwrap(), before);
        std::fs::write(
            output.join(format!("{}.json", case["id"].as_str().unwrap())),
            text,
        )
        .unwrap();
        let audited = (audit_tool.handler)(&json!({"board":board}), ctx.clone())
            .await
            .unwrap();
        let crate::mcp::protocol::ToolContent::Text { text: audit_text } = &audited.content[0]
        else {
            panic!("missing audit")
        };
        std::fs::write(
            output.join(format!("{}.audit.json", case["id"].as_str().unwrap())),
            audit_text,
        )
        .unwrap();
        println!(
            "{} status={} proposals={} duration_ms={}",
            case["id"],
            body["status"],
            body["proposals"].as_array().map_or(0, Vec::len),
            started.elapsed().as_millis()
        );
    }
}

#[test]
fn route_refine_is_discoverable_without_an_apply_or_output_escape_hatch() {
    let tools = crate::router::registry::tools_for("pcb_routing").unwrap();
    let tool = tools
        .iter()
        .find(|tool| tool.name == "plan_route_refinement")
        .expect("S7 must expose its read-only planner through pcb_routing");
    assert_eq!(tool.input_schema["required"], json!(["board"]));
    assert_eq!(tool.input_schema["additionalProperties"], false);
    assert!(tool.input_schema["properties"].get("apply").is_none());
    assert!(tool.input_schema["properties"].get("output").is_none());
}

fn on_back(s: String) -> String {
    s.replace("\"F.Cu\"", "\"B.Cu\"")
}

fn refusal(r: &Value) -> String {
    assert_eq!(r["status"], "blocked", "{r}");
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    r["blocked"][0]["reason"].as_str().unwrap().to_string()
}

/// KiCad 9 and 10 can give a via or pad different copper on each layer
/// (`padstack` with a mode other than `normal`). The planner models one size,
/// so it refuses such a board instead of certifying against the front size.
#[tokio::test]
async fn route_refine_refuses_a_padstack_with_per_layer_copper() {
    let via = |stack: &str| {
        format!(
            "(via (at 5.44 6.56) (size 0.2) (drill 0.1) (layers \"F.Cu\" \"B.Cu\") {stack}(net 3) (uuid {}))",
            uid(60)
        )
    };
    let board = |stack: &str| {
        [
            pad(1, (4., 6.), 2, "B.Cu"),
            pad(2, (6., 10.), 2, "B.Cu"),
            on_back(track(11, (4., 6.), (6., 6.), 2)),
            on_back(track(12, (6., 6.), (6., 10.), 2)),
            via(stack),
        ]
        .concat()
    };
    // The plain via is 0.2 mm wide on both layers: the corner can be repaired.
    let plain = Fixture::new(&board(""))
        .run(json!({"kinds":["chamfer"]}))
        .await;
    assert_eq!(plain["proposals"].as_array().unwrap().len(), 1, "{plain}");
    // On B.Cu this via is 0.5 mm wide: the same connector would pass 0.09 mm
    // from it. The first two are the forms KiCad writes; in the third, with
    // no mode, the layer entry replaces the copper of that layer.
    for (stack, reason) in [
        (
            "(padstack (mode front_inner_back) (layer \"Inner\" (size 0.2)) (layer \"B.Cu\" (size 0.5))) ",
            "unsupported padstack mode",
        ),
        (
            "(padstack (mode custom) (layer \"In1.Cu\" (size 0.2)) (layer \"B.Cu\" (size 0.5))) ",
            "unsupported padstack mode",
        ),
        (
            "(padstack (layer \"B.Cu\" (size 0.5))) ",
            "unsupported padstack layer copper",
        ),
    ] {
        let r = Fixture::new(&board(stack)).run(json!({"kinds":["chamfer"]})).await;
        assert_eq!(refusal(&r), reason, "{stack}");
    }
}

/// A clearance or zone-connection override on a pad, a footprint or one layer
/// of a padstack changes the rule the planner would check against.
#[tokio::test]
async fn route_refine_refuses_local_overrides_on_pads_footprints_and_padstack_layers() {
    let tht = |footprint: &str, pad_extra: &str, stack: &str| {
        format!(
            "(footprint \"Test:THT\" (layer \"F.Cu\") (at 0 0) {footprint}(uuid {}) (property \"Reference\" \"J1\" (at 0 0) (layer \"F.SilkS\")) (pad \"1\" thru_hole circle (at 20 20) (size 0.8 0.8) (drill 0.4) (layers \"*.Cu\" \"*.Mask\") {pad_extra}(net 3 \"OTHER\") {stack}(uuid {})))",
            uid(1950),
            uid(950)
        )
    };
    let run = |extra: String| async move {
        Fixture::new(&(dogleg() + &extra))
            .run(json!({"kinds":["chamfer"]}))
            .await
    };
    let plain = run(tht("", "", "")).await;
    assert_eq!(plain["proposals"].as_array().unwrap().len(), 1, "{plain}");
    for (extra, field) in [
        (tht("", "(clearance 2) ", ""), "clearance"),
        (tht("(clearance 2) ", "", ""), "clearance"),
        (tht("", "(zone_connect 2) ", ""), "zone_connect"),
        (tht("", "(thermal_gap 1) ", ""), "thermal_gap"),
        (
            tht("", "", "(padstack (layer \"B.Cu\" (clearance 2))) "),
            "clearance",
        ),
        (
            tht("", "", "(padstack (layer \"B.Cu\" (zone_connect 2))) "),
            "zone_connect",
        ),
        (
            tht(
                "",
                "",
                "(padstack (layer \"B.Cu\" (thermal_bridge_width 1))) ",
            ),
            "thermal_bridge_width",
        ),
    ] {
        let r = run(extra).await;
        assert_eq!(refusal(&r), format!("unsupported local {field} override"));
    }
}

/// KiCad locks every item inside a locked group, however deep the nesting.
#[tokio::test]
async fn route_refine_leaves_members_of_a_nested_locked_group_alone() {
    let groups = |outer: &str| {
        format!(
            "(group \"inner\" (uuid {}) (members \"{}\")) (group \"middle\" (uuid {}) (members \"{}\")) (group \"outer\" {outer}(uuid {}) (members \"{}\"))",
            uid(701),
            uid(12),
            uid(702),
            uid(701),
            uid(703),
            uid(702)
        )
    };
    let free = Fixture::new(&(dogleg() + &groups("")))
        .run(json!({"kinds":["chamfer"]}))
        .await;
    assert_eq!(free["proposals"].as_array().unwrap().len(), 1, "{free}");
    let r = Fixture::new(&(dogleg() + &groups("(locked yes) ")))
        .run(json!({"kinds":["chamfer"]}))
        .await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(r["coverage"]["blocked_total"], 1, "{r}");
    assert!(
        r["blocked"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("locked"),
        "{r}"
    );
}

/// KiCad holds copper to the edge clearance against shapes on the Margin
/// layer as well as the board edge. The planner does not model them.
#[tokio::test]
async fn route_refine_refuses_a_board_with_margin_layer_shapes() {
    let margin = |layer: &str| {
        format!(
            "(gr_line (start 1 1) (end 29 1) (stroke (width 0.1) (type default)) (layer \"{layer}\") (uuid {}))",
            uid(800)
        )
    };
    let other = Fixture::new(&(dogleg() + &margin("Dwgs.User")))
        .run(json!({"kinds":["chamfer"]}))
        .await;
    assert_eq!(other["proposals"].as_array().unwrap().len(), 1, "{other}");
    let r = Fixture::new(&(dogleg() + &margin("Margin")))
        .run(json!({"kinds":["chamfer"]}))
        .await;
    assert_eq!(
        refusal(&r),
        "items on the Margin layer are outside planner profile"
    );
    // Any item there is refused, text included, not only the shapes KiCad
    // reads for edge clearance.
    let text = format!(
        "(gr_text \"keep\" (at 5 2) (layer \"Margin\") (uuid {}) (effects (font (size 1 1) (thickness 0.15))))",
        uid(801)
    );
    let r = Fixture::new(&(dogleg() + &text))
        .run(json!({"kinds":["chamfer"]}))
        .await;
    assert_eq!(
        refusal(&r),
        "items on the Margin layer are outside planner profile"
    );
}

/// The clearance is the largest of the board minimum and every class
/// clearance. Here the board minimum is 0.1 and the class asks for 0.3: a
/// foreign stub 0.245 mm from the full-setback connector refuses that setback.
#[tokio::test]
async fn route_refine_checks_clearance_at_the_netclass_value_above_the_board_minimum() {
    let stub = track(13, (5.39, 6.6), (5.41, 6.6), 3).replace("(width 0.2)", "(width 0.3)");
    let plan = |class_clearance: f64| {
        let stub = stub.clone();
        async move {
            let f = Fixture::new(&(dogleg() + &stub));
            let path = f.board.with_extension("kicad_pro");
            let mut p: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            p["board"]["design_settings"]["rules"]["min_clearance"] = json!(0.1);
            p["net_settings"]["classes"][0]["clearance"] = json!(class_clearance);
            std::fs::write(&path, p.to_string()).unwrap();
            f.run(json!({"kinds":["chamfer"]})).await
        }
    };
    let low = plan(0.1).await;
    assert_eq!(
        *connector(&low["proposals"][0]),
        json!([{"x":5.5,"y":6.0},{"x":6.0,"y":6.5}]),
        "at 0.1 mm the full setback clears the stub: {low}"
    );
    let r = plan(0.3).await;
    assert_eq!(r["coverage"]["rule_policy"]["clearance_mm"], 0.3, "{r}");
    assert_eq!(
        *connector(&r["proposals"][0]),
        json!([{"x":5.75,"y":6.0},{"x":6.0,"y":6.25}]),
        "at 0.3 mm only the halved setback clears it: {r}"
    );
    assert_eq!(r["coverage"]["candidates_rejected"], json!({"clearance":1}));
}

/// Two fills that no pad is attached to are not ground the board uses. A via
/// between them would look like a stitch and connect nothing: never proposed,
/// even though the net has a pad nearby and the search does run.
#[tokio::test]
async fn route_refine_never_stitches_two_fills_that_no_pad_anchors() {
    let square = [(2., 2.), (18., 2.), (18., 18.), (2., 18.)];
    let f = Fixture::new(
        &[
            fill(30, "F.Cu", 1, &square),
            fill(31, "B.Cu", 1, &square),
            pad(1, (19.5, 10.), 1, "F.Cu"),
        ]
        .concat(),
    );
    let r = f.run(json!({"kinds":["stitching"]})).await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert!(
        r["coverage"]["candidates_considered"].as_u64().unwrap() > 100,
        "positions inside both fills must have been reached: {r}"
    );
}

/// A net follows its own class. The Default class width is the floor only
/// for a net that no assignment and no pattern places in another class.
#[tokio::test]
async fn route_refine_uses_the_assigned_class_width_instead_of_default() {
    let plan = |assign: bool| async move {
        let f = Fixture::new(&dogleg());
        let path = f.board.with_extension("kicad_pro");
        let mut p: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        p["net_settings"]["classes"] = json!([
            {"name":"Default","clearance":0.2,"track_width":0.25,"via_diameter":0.6,"via_drill":0.3},
            {"name":"Narrow","clearance":0.2,"track_width":0.15,"via_diameter":0.6,"via_drill":0.3}
        ]);
        if assign {
            p["net_settings"]["netclass_assignments"] = json!({"SIG":"Narrow"});
        }
        std::fs::write(&path, p.to_string()).unwrap();
        f.run(json!({"kinds":["chamfer"]})).await
    };
    let narrow = plan(true).await;
    assert_eq!(narrow["proposals"].as_array().unwrap().len(), 1, "{narrow}");
    let default = plan(false).await;
    assert!(
        default["proposals"].as_array().unwrap().is_empty(),
        "{default}"
    );
    assert_eq!(
        default["coverage"]["candidates_rejected"],
        json!({"existing width below supported saved floor":4})
    );
}

/// The pair-check limit ends the search like every other limit: partial,
/// named, and nothing that was not checked in full.
#[tokio::test]
async fn route_refine_names_the_pair_check_limit_and_returns_no_unchecked_proposal() {
    let f = Fixture::new(&dogleg());
    let budget = |pairs: usize| json!({"max_candidates":10,"max_pair_checks":pairs,"max_geometry_work":100_000_000,"max_elapsed_ms":30_000});
    let enough = f
        .run(json!({"kinds":["chamfer"],"budget":budget(1000)}))
        .await;
    assert_eq!(enough["proposals"].as_array().unwrap().len(), 1, "{enough}");
    let needed = enough["effort"]["used"]["pair_checks"].as_u64().unwrap() as usize;
    assert!(needed > 1);
    let r = f
        .run(json!({"kinds":["chamfer"],"budget":budget(needed - 1)}))
        .await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(r["status"], "partial");
    assert_eq!(r["truncated"], true);
    assert_eq!(r["coverage"]["complete"], false);
    assert_eq!(r["exhaustion"], json!(["pair-check budget exhausted"]));
    assert_eq!(phase_states(&r), [("chamfer", "stopped")]);
}

/// A whole number written as a float is an integer to the published schema.
#[tokio::test]
async fn route_refine_accepts_whole_numbers_written_as_floats() {
    let f = Fixture::new(&dogleg());
    let r = f
        .run(json!({"kinds":["chamfer"],"max_proposals":50.0}))
        .await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["parameters"]["max_proposals"], 50);
}

/// A refusal repeats the caller's own argument or net name. It must not let
/// that name grow the response without bound.
#[tokio::test]
async fn route_refine_bounds_the_input_it_echoes_in_a_refusal() {
    let long = "x".repeat(100_000);
    let f = Fixture::new(&dogleg());
    let mut args = json!({});
    args[long.as_str()] = json!(1);
    for args in [args, json!({"nets":[long]})] {
        let r = f.run(args).await;
        let reason = refusal(&r);
        assert!(reason.starts_with("unknown "), "{reason}");
        assert!(reason.len() < 200, "{}", reason.len());
        assert!(serde_json::to_vec(&r).unwrap().len() < 48 * 1024);
    }
}

fn with_project(items: &str, change: impl FnOnce(&mut Value)) -> Fixture {
    let f = Fixture::new(items);
    let path = f.board.with_extension("kicad_pro");
    let mut p: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    change(&mut p);
    std::fs::write(&path, p.to_string()).unwrap();
    f
}

/// Every board-level rule is required: a missing one is never read as zero.
/// A netclass may leave its own fields out; the board rules then stand alone.
#[tokio::test]
async fn route_refine_requires_every_board_rule_and_tolerates_missing_class_fields() {
    for key in [
        "min_clearance",
        "min_track_width",
        "min_copper_edge_clearance",
        "min_hole_clearance",
        "min_hole_to_hole",
        "min_via_annular_width",
        "min_via_diameter",
        "min_through_hole_diameter",
    ] {
        for null in [false, true] {
            let f = with_project(&dogleg(), |p| {
                let rules = p["board"]["design_settings"]["rules"]
                    .as_object_mut()
                    .unwrap();
                if null {
                    rules.insert(key.into(), Value::Null);
                } else {
                    rules.remove(key);
                }
            });
            let r = f.run(json!({"kinds":["chamfer"]})).await;
            let expected = if null {
                "rule must be numeric".to_string()
            } else {
                format!("required saved rule {key} unavailable")
            };
            assert_eq!(refusal(&r), expected);
        }
    }
    let f = with_project(&dogleg(), |p| {
        let class = p["net_settings"]["classes"][0].as_object_mut().unwrap();
        for key in ["clearance", "track_width", "via_diameter"] {
            class.remove(key);
        }
        class.insert("via_drill".into(), Value::Null);
    });
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["coverage"]["rule_policy"]["clearance_mm"], 0.2, "{r}");
}

/// A new via is never narrower than its drill plus the annular ring on both
/// sides, whatever smaller diameter the rules name.
#[tokio::test]
async fn route_refine_via_diameter_is_at_least_drill_plus_two_annular_rings() {
    let f = with_project(&dogleg(), |p| {
        p["board"]["design_settings"]["rules"]["min_via_diameter"] = json!(0.4);
        p["net_settings"]["classes"][0]["via_diameter"] = json!(0.4);
    });
    let r = f.run(json!({"kinds":["chamfer"]})).await;
    let policy = &r["coverage"]["rule_policy"];
    assert_eq!(policy["via_drill_mm"], 0.3, "{r}");
    assert_eq!(policy["via_diameter_mm"], 0.5, "0.3 + 2 x 0.1: {r}");
}

/// Copper drawn inside a footprint and blind or micro vias are outside the
/// profile; each is refused by name.
#[tokio::test]
async fn route_refine_refuses_footprint_copper_graphics_and_buried_vias() {
    let drawn = format!(
        "(footprint \"Test:Drawn\" (layer \"F.Cu\") (at 0 0) (uuid {}) (property \"Reference\" \"X1\" (at 0 0) (layer \"F.SilkS\")) (fp_line (start 20 20) (end 22 20) (stroke (width 0.2) (type default)) (layer \"F.Cu\") (uuid {})))",
        uid(1960),
        uid(960)
    );
    let r = Fixture::new(&(dogleg() + &drawn))
        .run(json!({"kinds":["chamfer"]}))
        .await;
    assert_eq!(refusal(&r), "unsupported footprint copper item fp_line");
    for kind in ["blind", "micro"] {
        let via = format!(
            "(via {kind} (at 20 20) (size 0.6) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") (net 3) (uuid {}))",
            uid(61)
        );
        let r = Fixture::new(&(dogleg() + &via))
            .run(json!({"kinds":["chamfer"]}))
            .await;
        assert_eq!(refusal(&r), "unsupported via type", "{kind}");
    }
}

/// KiCad gives a class that sets no track width the Default class's width.
#[tokio::test]
async fn route_refine_a_class_without_a_width_uses_the_default_width() {
    let plan = |default_width: f64| async move {
        let f = with_project(&dogleg(), |p| {
            p["net_settings"]["classes"] = json!([
                {"name":"Default","clearance":0.2,"track_width":default_width,"via_diameter":0.6,"via_drill":0.3},
                {"name":"Clr","clearance":0.2}
            ]);
            p["net_settings"]["netclass_assignments"] = json!({"SIG":"Clr"});
        });
        f.run(json!({"kinds":["chamfer"]})).await
    };
    let fits = plan(0.2).await;
    assert_eq!(fits["proposals"].as_array().unwrap().len(), 1, "{fits}");
    let r = plan(0.25).await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(
        r["coverage"]["candidates_rejected"],
        json!({"existing width below supported saved floor":4})
    );
}

/// A pattern gives its class to the nets it names under KiCad's wildcard
/// reading: `*` any run, `?` any one character, every other character literal,
/// the whole name, case-sensitive (KiCad 10.0.1 `common/eda_pattern_match.cpp`
/// lines 207-244; used for net names at `common/project/net_settings.cpp`
/// lines 756-758).
///
/// Documented limit: KiCad also reads each pattern as an anchored regular
/// expression (`eda_pattern_match.cpp` lines 108-119), and the planner does not.
/// Where only that reading names the net, the planner keeps the net's other
/// floor and can list a connector that `route_batch` refuses at adoption.
#[tokio::test]
async fn route_refine_a_pattern_class_applies_to_the_nets_its_wildcard_reading_names() {
    let plan = |default: f64, other: f64, pattern: &'static str| async move {
        let f = with_project(&dogleg(), |p| {
            p["net_settings"]["classes"] = json!([
                {"name":"Default","clearance":0.2,"track_width":default,"via_diameter":0.6,"via_drill":0.3},
                {"name":"Other","clearance":0.2,"track_width":other,"via_diameter":0.6,"via_drill":0.3}
            ]);
            p["net_settings"]["netclass_patterns"] =
                json!([{"netclass":"Other","pattern":pattern}]);
        });
        f.run(json!({"kinds":["chamfer"]})).await
    };
    let refused = |r: &Value, case: &str| {
        assert!(r["proposals"].as_array().unwrap().is_empty(), "{case}: {r}");
        assert_eq!(
            r["coverage"]["candidates_rejected"],
            json!({"existing width below supported saved floor":4}),
            "{case}"
        );
    };
    let listed = |r: &Value, case: &str| {
        assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{case}: {r}");
    };
    // The net is SIG and its track is 0.2 mm wide. A wider class floors it
    // when the pattern names it.
    for pattern in ["SIG", "S*", "*", "S?G", "SI?", "*G", "SIG*"] {
        refused(&plan(0.2, 0.4, pattern).await, pattern);
    }
    // A wider class does not raise the floor of a net its pattern does not
    // name. Brackets, escapes and regex characters are literal.
    for pattern in [
        "PWR*", "PWR_?V", "GND", "sig", "SI", "SIGN", "S?", "+5V", "SIG[0-9]", "[SIG]", "^S", "G$",
        "A{2}",
    ] {
        listed(&plan(0.2, 0.4, pattern).await, pattern);
    }
    // A narrower class replaces Default for the nets its pattern names...
    listed(&plan(0.25, 0.15, "S*").await, "narrow class by pattern");
    // ...and for no other net (review C, C1): SIG keeps Default's 0.25.
    refused(
        &plan(0.25, 0.1, "^USB.*$").await,
        "narrow class, other nets",
    );
    // The documented limit (review C, C2). As a regular expression each of
    // these names SIG, so KiCad puts it in the 0.4 mm class; as a wildcard
    // none does. The listed 0.2 mm connector is refused by `route_batch`.
    for pattern in ["SIG?", "(SIG|GND)", "S.G", "SI.*", "S+IG", "S\\w+"] {
        listed(&plan(0.2, 0.4, pattern).await, pattern);
    }
}

/// The older `(id ...)` spelling of a group's identity nests the same way.
#[tokio::test]
async fn route_refine_resolves_nested_locked_groups_written_with_id() {
    let groups = |outer: &str| {
        format!(
            "(group \"inner\" (id {}) (members {})) (group \"outer\" {outer}(id {}) (members {}))",
            uid(711),
            uid(12),
            uid(712),
            uid(711)
        )
    };
    let free = Fixture::new(&(dogleg() + &groups("")))
        .run(json!({"kinds":["chamfer"]}))
        .await;
    assert_eq!(free["proposals"].as_array().unwrap().len(), 1, "{free}");
    let r = Fixture::new(&(dogleg() + &groups("locked ")))
        .run(json!({"kinds":["chamfer"]}))
        .await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(r["coverage"]["blocked_total"], 1, "{r}");
}

/// Two groups written with the same `(id ...)` both stay locked when a locked
/// group contains that id.
#[tokio::test]
async fn route_refine_keeps_the_lock_of_every_group_that_shares_an_id() {
    let groups = |outer: &str| {
        format!(
            "(group \"a\" (id {x}) (members {t})) (group \"b\" (id {x}) (members {o})) (group \"outer\" {outer}(id {y}) (members {x}))",
            x = uid(721),
            t = uid(12),
            o = uid(722),
            y = uid(723)
        )
    };
    let free = Fixture::new(&(dogleg() + &groups("")))
        .run(json!({"kinds":["chamfer"]}))
        .await;
    assert_eq!(free["proposals"].as_array().unwrap().len(), 1, "{free}");
    let r = Fixture::new(&(dogleg() + &groups("locked ")))
        .run(json!({"kinds":["chamfer"]}))
        .await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(r["coverage"]["blocked_total"], 1, "{r}");
}

/// The repair is the same geometry at any orientation: a dogleg turned 45
/// degrees is chamfered at the same setback along its own legs.
#[tokio::test]
async fn route_refine_chamfers_a_dogleg_at_any_orientation() {
    let items = [
        pad(1, (7., 7.), 2, "F.Cu"),
        pad(2, (13., 7.), 2, "F.Cu"),
        track(11, (7., 7.), (10., 10.), 2),
        track(12, (10., 10.), (13., 7.), 2),
    ]
    .concat();
    let r = Fixture::new(&items).run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(
        connector(&r["proposals"][0]),
        &json!([{"x":9.646447,"y":9.646447},{"x":10.353553,"y":9.646447}]),
        "0.5 mm along each leg from the junction: {r}"
    );
}

/// The trunk is the pair of legs that face each other, wherever the branch sits
/// in the file. The branch is the leg that moves, and its new end is the
/// setback along the branch, met square to the trunk.
#[tokio::test]
async fn route_refine_finds_the_trunk_whatever_order_the_legs_are_listed_in() {
    let items = [
        pad(1, (4., 10.), 2, "F.Cu"),
        pad(2, (16., 10.), 2, "F.Cu"),
        pad(3, (7., 7.), 2, "F.Cu"),
        track(13, (7., 7.), (10., 10.), 2),
        track(11, (4., 10.), (10., 10.), 2),
        track(12, (10., 10.), (16., 10.), 2),
    ]
    .concat();
    let r = Fixture::new(&items)
        .run(json!({"kinds":["t_junction"]}))
        .await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(
        connector(&r["proposals"][0]),
        &json!([{"x":9.646447,"y":9.646447},{"x":9.646447,"y":10.0}]),
        "{r}"
    );
    let edits: Vec<_> = r["proposals"][0]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|op| op["tool"] == "modify_trace")
        .map(|op| op["arguments"]["uuid"].clone())
        .collect();
    assert_eq!(edits, vec![json!(uid(13))], "only the branch moves: {r}");
}

/// The connector meets the trunk at the foot of the perpendicular. A foot past
/// the end of either trunk leg would leave the trunk, so the junction is
/// reported instead, at every setback. A foot exactly on the end is on the trunk.
#[tokio::test]
async fn route_refine_blocks_a_repair_whose_foot_would_pass_the_trunk_end() {
    // The smallest setback is 0.0625 mm; at 45 degrees its foot is 0.044194 mm
    // from the junction. `first` lists the short leg before the long one.
    let plan = |short_x: f64, first: bool| async move {
        let (long_leg, branch) = if short_x > 10. {
            ((4., 10.), (13., 7.))
        } else {
            ((16., 10.), (7., 7.))
        };
        let long = track(11, long_leg, (10., 10.), 2);
        let short = track(12, (10., 10.), (short_x, 10.), 2);
        let trunk = if first { short + &long } else { long + &short };
        let items = [
            pad(1, long_leg, 2, "F.Cu"),
            pad(3, branch, 2, "F.Cu"),
            trunk,
            track(13, branch, (10., 10.), 2),
        ]
        .concat();
        Fixture::new(&items)
            .run(json!({"kinds":["t_junction"]}))
            .await
    };
    for (short_x, first) in [
        (10.04, false),
        (10.044, false),
        (10.044, true),
        (9.96, false),
        (9.956, false),
        (9.956, true),
    ] {
        let r = plan(short_x, first).await;
        assert!(
            r["proposals"].as_array().unwrap().is_empty(),
            "{short_x} {first}: {r}"
        );
        assert_eq!(r["coverage"]["blocked_total"], 1, "{short_x} {first}: {r}");
        assert_eq!(
            r["blocked"][0]["reason"], "perpendicular foot is beyond the trunk",
            "{short_x} {first}: {r}"
        );
    }
    for (short_x, first, branch_end) in [
        (10.044194, false, 10.044194),
        (10.044194, true, 10.044194),
        (9.955806, false, 9.955806),
        (9.955806, true, 9.955806),
    ] {
        let r = plan(short_x, first).await;
        assert_eq!(
            r["proposals"].as_array().unwrap().len(),
            1,
            "{short_x} {first}: {r}"
        );
        assert_eq!(
            connector(&r["proposals"][0]),
            &json!([{"x":branch_end,"y":9.955806},{"x":short_x,"y":10.0}]),
            "{short_x} {first}: {r}"
        );
    }
}

/// One row per junction, however many findings name it: a junction of four
/// track ends is outside the three kinds and is reported once.
#[tokio::test]
async fn route_refine_reports_a_junction_named_by_several_findings_once() {
    let items = [
        track(11, (4., 10.), (10., 10.), 2),
        track(12, (10., 10.), (16., 10.), 2),
        track(13, (13., 13.), (10., 10.), 2),
        track(14, (7., 13.), (10., 10.), 2),
    ]
    .concat();
    let r = Fixture::new(&items)
        .run(json!({"kinds":["t_junction"]}))
        .await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(r["coverage"]["blocked_total"], 1, "{r}");
    assert!(
        r["blocked"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("4 straight track ends meet here"),
        "{r}"
    );
}

/// A trunk that is not along an axis: the branch end moves to the setback
/// along the branch, and the connector runs from there to the foot of the
/// perpendicular on the trunk, at the branch's width.
#[tokio::test]
async fn route_refine_reterminates_a_branch_on_a_diagonal_trunk_at_exact_coordinates() {
    let trunk = [
        pad(1, (6., 6.), 2, "F.Cu"),
        pad(2, (14., 14.), 2, "F.Cu"),
        track(11, (6., 6.), (10., 10.), 2),
        track(12, (10., 10.), (14., 14.), 2),
    ]
    .concat();
    // Branch along (0.6, -0.8): 0.5 mm along it is (10.3, 9.6), whose foot on
    // the line y = x is (9.95, 9.95).
    let branch = [
        pad(3, (13., 6.), 2, "F.Cu"),
        track(13, (13., 6.), (10., 10.), 2),
    ]
    .concat();
    let r = Fixture::new(&(trunk.clone() + &branch))
        .run(json!({"kinds":["t_junction"]}))
        .await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    let ops = r["proposals"][0]["operations"].as_array().unwrap();
    assert_eq!(ops.len(), 2, "{r}");
    assert_eq!(
        ops[0]["arguments"]["tracks"],
        json!([{"layer":"F.Cu","net_name":"SIG","points":[{"x":10.3,"y":9.6},{"x":9.95,"y":9.95}],"width":0.2}]),
        "{r}"
    );
    assert_eq!(ops[1]["tool"], "modify_trace", "{r}");
    let moved = &ops[1]["arguments"];
    assert_eq!(moved["uuid"], uid(13), "{r}");
    assert_eq!(
        [&moved["x1"], &moved["y1"], &moved["x2"], &moved["y2"]],
        [&json!(13.0), &json!(6.0), &json!(10.3), &json!(9.6)],
        "{r}"
    );
    // The branch listed before the trunk, along +x: 0.5 mm along it is
    // (10.5, 10), whose foot on the trunk is (10.25, 10.25).
    let branch = [
        pad(3, (14., 10.), 2, "F.Cu"),
        track(13, (14., 10.), (10., 10.), 2),
    ]
    .concat();
    let r = Fixture::new(&(branch + &trunk))
        .run(json!({"kinds":["t_junction"]}))
        .await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(
        connector(&r["proposals"][0]),
        &json!([{"x":10.5,"y":10.0},{"x":10.25,"y":10.25}]),
        "{r}"
    );
    let ops = r["proposals"][0]["operations"].as_array().unwrap();
    assert_eq!(ops[1]["arguments"]["uuid"], uid(13), "{r}");
}

/// A leg must be longer than the setback plus 0.01 mm. The smallest setback
/// tried is 0.0625 mm, so a 0.07 mm leg cannot be repaired and a 0.08 mm one can.
#[tokio::test]
async fn route_refine_needs_a_leg_longer_than_the_smallest_setback() {
    let branch = |length: f64| {
        let end = ((10. - length / 2f64.sqrt()) * 1e6).round() / 1e6;
        [
            pad(1, (4., 10.), 2, "F.Cu"),
            pad(2, (16., 10.), 2, "F.Cu"),
            track(11, (4., 10.), (10., 10.), 2),
            track(12, (10., 10.), (16., 10.), 2),
            track(13, (end, end), (10., 10.), 2),
        ]
        .concat()
    };
    let r = Fixture::new(&branch(0.07))
        .run(json!({"kinds":["t_junction"]}))
        .await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(
        r["blocked"][0]["reason"], "branch shorter than the setback",
        "{r}"
    );
    let r = Fixture::new(&branch(0.08))
        .run(json!({"kinds":["t_junction"]}))
        .await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(
        connector(&r["proposals"][0]),
        &json!([{"x":9.955806,"y":9.955806},{"x":9.955806,"y":10.0}]),
        "{r}"
    );
    let segment = |length: f64| {
        [
            pad(1, (4., 6.), 2, "F.Cu"),
            track(11, (4., 6.), (6., 6.), 2),
            track(12, (6., 6.), (6., 6. + length), 2),
        ]
        .concat()
    };
    let r = Fixture::new(&segment(0.07))
        .run(json!({"kinds":["chamfer"]}))
        .await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(
        r["blocked"][0]["reason"], "segment shorter than the setback",
        "{r}"
    );
    let r = Fixture::new(&segment(0.08))
        .run(json!({"kinds":["chamfer"]}))
        .await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(
        connector(&r["proposals"][0]),
        &json!([{"x":5.9375,"y":6.0},{"x":6.0,"y":6.0625}]),
        "{r}"
    );
}

/// A branch that lies on a trunk leg has no perpendicular to drop: the new end
/// and its foot are the same point, and no zero-length connector is proposed.
#[tokio::test]
async fn route_refine_proposes_no_connector_for_a_branch_lying_on_the_trunk() {
    let items = [
        pad(1, (4., 10.), 2, "F.Cu"),
        pad(2, (16., 10.), 2, "F.Cu"),
        track(11, (4., 10.), (10., 10.), 2),
        track(12, (10., 10.), (16., 10.), 2),
        track(13, (7., 10.), (10., 10.), 2),
    ]
    .concat();
    let r = Fixture::new(&items)
        .run(json!({"kinds":["t_junction"]}))
        .await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(r["coverage"]["blocked_total"], 1, "{r}");
    assert_eq!(
        r["blocked"][0]["reason"], "retermination is not perpendicular after 1 nm rounding",
        "{r}"
    );
}

/// The right-angle band is narrow. A branch 0.17 degrees off square is outside
/// it and is repaired; its foot is 1.5 um from the junction.
#[tokio::test]
async fn route_refine_repairs_a_branch_just_outside_the_right_angle_band() {
    let items = [
        pad(1, (4., 10.), 2, "F.Cu"),
        pad(2, (16., 10.), 2, "F.Cu"),
        pad(3, (10.008901, 7.000013), 2, "F.Cu"),
        track(11, (4., 10.), (10., 10.), 2),
        track(12, (10., 10.), (16., 10.), 2),
        track(13, (10.008901, 7.000013), (10., 10.), 2),
    ]
    .concat();
    let r = Fixture::new(&items)
        .run(json!({"kinds":["t_junction"]}))
        .await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(
        connector(&r["proposals"][0]),
        &json!([{"x":10.001483,"y":9.500002},{"x":10.001483,"y":10.0}]),
        "{r}"
    );
}

/// The audit counts a corner between 89 and 91 degrees as a right angle. The
/// chamfer is built only for legs that are exactly perpendicular.
#[tokio::test]
async fn route_refine_chamfers_only_an_exactly_perpendicular_corner() {
    let items = [
        pad(1, (4., 6.), 2, "F.Cu"),
        pad(2, (5.993019, 9.999994), 2, "F.Cu"),
        track(11, (4., 6.), (6., 6.), 2),
        track(12, (6., 6.), (5.993019, 9.999994), 2),
    ]
    .concat();
    let r = Fixture::new(&items).run(json!({"kinds":["chamfer"]})).await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(r["coverage"]["blocked_total"], 1, "{r}");
    assert_eq!(
        r["blocked"][0]["reason"], "segments are not exactly perpendicular",
        "{r}"
    );
}

/// `nets` limits corner repairs to the named nets, and two corners of one net
/// that share an x coordinate are two junctions.
#[tokio::test]
async fn route_refine_corner_repairs_follow_the_net_filter_and_keep_junctions_apart() {
    let other = [
        pad(3, (14., 6.), 3, "F.Cu"),
        pad(4, (16., 10.), 3, "F.Cu"),
        track(21, (14., 6.), (16., 6.), 3),
        track(22, (16., 6.), (16., 10.), 3),
    ]
    .concat();
    let f = Fixture::new(&(dogleg() + &other));
    let all = f.run(json!({"kinds":["chamfer"]})).await;
    assert_eq!(all["proposals"].as_array().unwrap().len(), 2, "{all}");
    let r = f.run(json!({"kinds":["chamfer"],"nets":["SIG"]})).await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(
        r["proposals"][0]["operations"][0]["arguments"]["tracks"][0]["net_name"], "SIG",
        "{r}"
    );
    assert_eq!(r["coverage"]["blocked_total"], 0, "{r}");
    // Corners at (6, 6) and (6, 6.4): neither is a pad exit, both are reported.
    let steps = [
        track(11, (4., 6.), (6., 6.), 2),
        track(12, (6., 6.), (6., 6.4), 2),
        track(13, (6., 6.4), (8., 6.4), 2),
    ]
    .concat();
    let r = Fixture::new(&steps).run(json!({"kinds":["chamfer"]})).await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(r["coverage"]["blocked_total"], 2, "{r}");
}

/// A branch that already meets its trunk squarely gives the repair nothing to
/// move. A trunk bent by a few hundredths of a degree makes such a junction
/// read as acute; the planner must say so instead of proposing a null edit.
#[tokio::test]
async fn route_refine_does_not_reterminate_a_branch_that_already_meets_the_trunk_squarely() {
    let at = |degrees: f64, length: f64| {
        let r = degrees.to_radians();
        (
            ((10. + length * r.cos()) * 1e6).round() / 1e6,
            ((10. + length * r.sin()) * 1e6).round() / 1e6,
        )
    };
    let junction = |bend: f64| {
        [
            pad(1, at(45. + bend, 3.), 2, "F.Cu"),
            pad(2, at(-135., 3.), 2, "F.Cu"),
            pad(3, at(135., 3.), 2, "F.Cu"),
            track(11, at(45. + bend, 3.), (10., 10.), 2),
            track(12, (10., 10.), at(-135., 3.), 2),
            track(13, at(135., 3.), (10., 10.), 2),
        ]
        .concat()
    };
    let r = Fixture::new(&junction(0.03))
        .run(json!({"kinds":["t_junction"]}))
        .await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(r["coverage"]["blocked_total"], 1, "{r}");
    assert_eq!(
        r["blocked"][0]["reason"], "branch already meets the trunk at a right angle",
        "{r}"
    );
    // Square to the second trunk leg only: 89.93 degrees to the first one.
    let r = Fixture::new(&junction(0.07))
        .run(json!({"kinds":["t_junction"]}))
        .await;
    assert!(r["proposals"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(
        r["blocked"][0]["reason"], "branch already meets the trunk at a right angle",
        "{r}"
    );
    // A branch that is really acute to its trunk is still repaired.
    let acute = [
        pad(1, (4., 10.), 2, "F.Cu"),
        pad(2, (16., 10.), 2, "F.Cu"),
        pad(3, (7., 7.), 2, "F.Cu"),
        track(11, (4., 10.), (10., 10.), 2),
        track(12, (10., 10.), (16., 10.), 2),
        track(13, (7., 7.), (10., 10.), 2),
    ]
    .concat();
    let r = Fixture::new(&acute)
        .run(json!({"kinds":["t_junction"]}))
        .await;
    assert_eq!(r["proposals"].as_array().unwrap().len(), 1, "{r}");
}
