//! The board-geometry layer against `board_geometry/cases.kicad_pcb`.
//!
//! The fixture is a 40 × 30 mm board with 5 mm rounded corners (arcs, the way
//! `add_board_outline` draws them) and a 3 mm circular cut-out at its centre,
//! carrying one footprint for each situation the checks must tell apart. Every
//! expected number below was worked out by hand from the fixture's coordinates
//! (see its README), not copied from the code's output.

use konnect_sexp::board_geometry::{Allowlist, BoardGeometry, Kind, RegionQuery, Shape};
use konnect_sexp::outline::OutlineStatus;
use konnect_sexp::parse_sexp;
use std::path::PathBuf;

fn load(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn geometry() -> BoardGeometry {
    BoardGeometry::from_tree(&parse_sexp(&load("board_geometry/cases.kicad_pcb")).unwrap())
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

// ── The outline ──────────────────────────────────────────────────────────────

#[test]
fn the_rounded_outline_with_a_cutout_is_two_closed_rings() {
    let g = geometry();
    assert_eq!(g.outline.status(), OutlineStatus::Closed);
    assert!(g.outline.curved);
    assert_eq!(g.outline.loops.len(), 2, "the board ring and the cut-out");
    assert_eq!(g.outline.hole_count(), 1);
}

#[test]
fn the_outline_tells_corner_cutout_and_board_apart() {
    let o = &geometry().outline;
    assert!(o.contains((20.0, 5.0)), "plain board");
    assert!(
        o.contains((2.0, 2.0)),
        "inside the corner arc: 4.24 mm from its centre, radius 5"
    );
    assert!(
        !o.contains((0.5, 0.5)),
        "the square corner of the bbox is not board"
    );
    assert!(!o.contains((20.0, 15.0)), "the cut-out is not board");
    assert!(!o.contains((41.0, 15.0)));
}

// ── Pads ─────────────────────────────────────────────────────────────────────

#[test]
fn a_rotated_footprints_pads_land_where_the_rotation_puts_them() {
    // Q1: at (30, 22), rotated 90°. Pad 1 sits at local (-1.5, 0): KiCad's
    // screen-CCW rotation takes that to (30, 22 + 1.5). Its 0.8 × 1.6 body turns
    // with the footprint, so the long side runs along x.
    let g = geometry();
    let q1 = g.footprint("Q1").unwrap();
    let p1 = q1.pads.iter().find(|p| p.number == "1").unwrap();
    assert!(
        close(p1.at.0, 30.0, 1e-9) && close(p1.at.1, 23.5, 1e-9),
        "{:?}",
        p1.at
    );
    let (x0, y0, x1, y1) = p1.shape.bbox().unwrap();
    assert!(close(x0, 29.2, 1e-9) && close(x1, 30.8, 1e-9), "{x0} {x1}");
    assert!(close(y0, 23.1, 1e-9) && close(y1, 23.9, 1e-9), "{y0} {y1}");
    assert_eq!(p1.net.as_deref(), Some("SIG"));
}

#[test]
fn a_pad_next_to_the_cutout_is_on_the_board_only_under_the_right_rotation_sign() {
    // Q2: at (20, 18.5), rotated 90°, pad 1 at local (-0.7, 0). The right sign
    // puts it at (20, 19.2), 0.8 mm below the cut-out's edge (y = 18). The wrong
    // sign puts it at (20, 17.8), straight across that edge.
    let g = geometry();
    let q2 = g.footprint("Q2").unwrap();
    let pad = &q2.pads[0];
    assert!(close(pad.at.1, 19.2, 1e-9), "{:?}", pad.at);
    assert_eq!(g.outline.overhang(&pad.shape.pts).depth_mm, 0.0);
}

// ── Containment ──────────────────────────────────────────────────────────────

fn violation<'a>(
    check: &'a konnect_sexp::board_geometry::OutlineCheck,
    reference: &str,
) -> &'a konnect_sexp::board_geometry::FootprintOverhang {
    check
        .violations
        .iter()
        .find(|v| v.reference == reference)
        .unwrap_or_else(|| {
            panic!(
                "{reference} should be a violation: {:?}",
                check
                    .violations
                    .iter()
                    .map(|v| &v.reference)
                    .collect::<Vec<_>>()
            )
        })
}

#[test]
fn exactly_the_parts_that_cross_the_outline_are_reported() {
    let check = geometry().outline_check(&Allowlist::default());
    assert!(check.checked);
    assert_eq!(check.footprints_checked, 8);
    let mut refs: Vec<&str> = check
        .violations
        .iter()
        .map(|v| v.reference.as_str())
        .collect();
    refs.sort_unstable();
    assert_eq!(refs, ["C1", "C2", "J1", "Q2", "R9", "TP1"]);
    // U1 and Q1 are well inside and must not appear.
    assert!(check.allowed.is_empty());
}

#[test]
fn a_part_fully_outside_is_outside_by_its_distance_from_the_edge() {
    // R9's courtyard is x 59..61, y 14.5..15.5; the right edge is at x = 40.
    let check = geometry().outline_check(&Allowlist::default());
    let r9 = violation(&check, "R9");
    assert_eq!(r9.relation, "outside");
    assert!(close(r9.overhang_mm, 21.0, 0.01), "{}", r9.overhang_mm);
}

#[test]
fn a_part_straddling_the_edge_reports_courtyard_and_pad_overhangs_in_mm() {
    // J1 at (40, 15): courtyard x 37..43 (3 mm past the edge); pad 2 spans
    // x 39.5..41.5 (1.5 mm past it); pad 1 spans 38..39 and is inside.
    let check = geometry().outline_check(&Allowlist::default());
    let j1 = violation(&check, "J1");
    assert_eq!(j1.relation, "straddles");
    assert!(close(j1.courtyard.as_ref().unwrap().overhang_mm, 3.0, 0.01));
    assert_eq!(j1.pads_over, 1);
    assert_eq!(j1.pads[0].label, "2");
    assert!(
        close(j1.pads[0].overhang_mm, 1.5, 0.01),
        "{}",
        j1.pads[0].overhang_mm
    );
    assert!(close(j1.overhang_mm, 3.0, 0.01));
}

#[test]
fn the_rounded_corner_catches_a_part_a_rectangle_check_would_pass() {
    // C1 at (3, 3): its courtyard (1..5 square) and pad 3 (0.5..1.5 square) sit
    // inside the board's bounding box, so a bbox test calls them inside. The
    // arc centre is (5, 5), radius 5: the courtyard corner (1, 1) is
    // 5.657 − 5 = 0.657 mm outside it, and pad 3's corner (0.5, 0.5) is
    // 6.364 − 5 = 1.364 mm outside.
    let check = geometry().outline_check(&Allowlist::default());
    let c1 = violation(&check, "C1");
    assert!(
        close(c1.courtyard.as_ref().unwrap().overhang_mm, 0.657, 0.01),
        "{:?}",
        c1.courtyard
    );
    assert_eq!(c1.pads_over, 1);
    assert_eq!(c1.pads[0].label, "3");
    assert!(close(c1.pads[0].overhang_mm, 1.364, 0.01), "{:?}", c1.pads);
}

#[test]
fn a_part_over_the_cutout_is_reported_with_the_depth_into_it() {
    // C2 at (22.5, 15): courtyard x 20.5..24.5, y 13..17. The cut-out is the
    // circle of radius 3 about (20, 15); the courtyard's left edge midpoint
    // (20.5, 15) is 3 − 0.5 = 2.5 mm inside it.
    let check = geometry().outline_check(&Allowlist::default());
    let c2 = violation(&check, "C2");
    assert_eq!(c2.relation, "straddles", "its right part is on the board");
    assert!(
        close(c2.courtyard.as_ref().unwrap().overhang_mm, 2.5, 0.01),
        "{:?}",
        c2.courtyard
    );
    assert_eq!(
        c2.pads_over, 1,
        "pad 1 is inside the cut-out, pad 2 is on the board"
    );
    assert_eq!(c2.pads[0].label, "1");
}

#[test]
fn text_hanging_over_the_edge_is_reported_apart_from_the_body() {
    // TP1's body is inside; its reference text is centred on the top edge.
    let check = geometry().outline_check(&Allowlist::default());
    let tp1 = violation(&check, "TP1");
    assert_eq!(tp1.relation, "text_only");
    assert!(tp1.courtyard.is_none() && tp1.pads.is_empty());
    assert_eq!(tp1.texts.len(), 1);
    assert_eq!(tp1.texts[0].label, "reference");
    assert!(
        tp1.texts[0].overhang_mm > 0.3,
        "{}",
        tp1.texts[0].overhang_mm
    );
}

#[test]
fn the_allowlist_excuses_a_part_but_still_reports_it() {
    let g = geometry();
    let check = g.outline_check(&Allowlist::new(["J1"]));
    assert!(check.violations.iter().all(|v| v.reference != "J1"));
    let j1 = check
        .allowed
        .iter()
        .find(|v| v.reference == "J1")
        .expect("listed as allowed");
    assert!(j1.allowed && close(j1.overhang_mm, 3.0, 0.01));

    // A trailing `*` is a prefix.
    let by_prefix = g.outline_check(&Allowlist::new(["C*", "J*"]));
    let allowed: Vec<_> = by_prefix
        .allowed
        .iter()
        .map(|v| v.reference.as_str())
        .collect();
    assert!(allowed.contains(&"C1") && allowed.contains(&"C2") && allowed.contains(&"J1"));
    assert!(by_prefix
        .violations
        .iter()
        .all(|v| !v.reference.starts_with(['C', 'J'])));
}

#[test]
fn nothing_is_judged_against_an_open_or_missing_outline() {
    let text = load("board_geometry/cases.kicad_pcb");
    // Move the first outline line to another layer: the ring no longer closes.
    let open = text.replacen("(layer \"Edge.Cuts\")", "(layer \"Dwgs.User\")", 1);
    let g = BoardGeometry::from_tree(&parse_sexp(&open).unwrap());
    let check = g.outline_check(&Allowlist::default());
    assert_eq!(check.status, OutlineStatus::Open);
    assert!(
        !check.checked && check.violations.is_empty(),
        "an open outline proves nothing"
    );
    let no_edge = text.replace("\"Edge.Cuts\")", "\"Dwgs.User\")");
    let g = BoardGeometry::from_tree(&parse_sexp(&no_edge).unwrap());
    let check = g.outline_check(&Allowlist::default());
    assert_eq!(check.status, OutlineStatus::Missing);
    assert!(!check.checked);
    assert!(check.violations.is_empty(), "no verdict without an outline");
}

// ── Region queries ───────────────────────────────────────────────────────────

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Shape {
    Shape::polygon(vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
}

fn query(region: Shape, kinds: &[Kind]) -> RegionQuery {
    RegionQuery {
        anchor: region.clone(),
        region,
        kinds: kinds.to_vec(),
        layer: None,
        net: None,
        exclude_reference: None,
        limit: 100,
    }
}

#[test]
fn a_box_query_returns_the_copper_in_it_with_nets_and_uuids() {
    let g = geometry();
    // Around U1's routing: x 8..16, y 6..14.
    let r = g.query_region(&query(
        rect(8.0, 6.0, 16.0, 14.0),
        &[Kind::Track, Kind::Via],
    ));
    assert_eq!(r.total, 4, "two F.Cu tracks, one B.Cu track, one via");
    assert!(!r.truncated);
    assert_eq!(r.counts.get("track"), Some(&3));
    assert_eq!(r.counts.get("via"), Some(&1));
    for hit in &r.hits {
        assert!(hit.entry.uuid.is_some(), "{hit:?}");
        assert!(hit.entry.net.is_some(), "{hit:?}");
    }
}

#[test]
fn a_point_and_radius_finds_what_touches_the_point() {
    let g = geometry();
    let at = (10.0, 12.0);
    let q = RegionQuery {
        anchor: Shape::disc(at, 0.0),
        ..query(Shape::disc(at, 0.1), &[])
    };
    let r = g.query_region(&q);
    let kinds: Vec<&str> = r.hits.iter().map(|h| h.entry.kind.key()).collect();
    // The via, the F.Cu track ending there, the B.Cu track starting there, and
    // the silkscreen line passing through it.
    assert_eq!(r.total, 4, "{kinds:?}");
    assert_eq!(kinds.iter().filter(|k| **k == "via").count(), 1);
    assert_eq!(kinds.iter().filter(|k| **k == "track").count(), 2);
    assert_eq!(kinds.iter().filter(|k| **k == "graphic").count(), 1);
}

#[test]
fn layer_and_net_filters_narrow_the_answer() {
    let g = geometry();
    let wide = rect(8.0, 6.0, 16.0, 14.0);
    let on_back = g.query_region(&RegionQuery {
        layer: Some("B.Cu".into()),
        ..query(wide.clone(), &[Kind::Track, Kind::Via])
    });
    assert_eq!(on_back.total, 2, "the B.Cu track and the through via");
    let gnd = g.query_region(&RegionQuery {
        net: Some("GND".into()),
        ..query(wide, &[Kind::Track, Kind::Via, Kind::Pad])
    });
    assert!(
        gnd.total >= 2
            && gnd
                .hits
                .iter()
                .all(|h| h.entry.net.as_deref() == Some("GND"))
    );
}

#[test]
fn a_query_around_a_footprint_excludes_it_and_sorts_nearest_first() {
    let g = geometry();
    let u1 = g.footprint("U1").unwrap();
    let (body, _) = u1.body();
    let (x0, y0, x1, y1) = body.bbox().unwrap();
    let margin = 3.0;
    let q = RegionQuery {
        anchor: body,
        exclude_reference: Some("U1".into()),
        ..query(
            rect(x0 - margin, y0 - margin, x1 + margin, y1 + margin),
            &[],
        )
    };
    let r = g.query_region(&q);
    assert!(r.total > 0);
    assert!(r
        .hits
        .iter()
        .all(|h| h.entry.reference.as_deref() != Some("U1")));
    assert!(
        r.hits
            .windows(2)
            .all(|w| w[0].distance_mm <= w[1].distance_mm),
        "nearest first"
    );
}

#[test]
fn the_limit_cuts_the_list_and_says_so_without_hiding_the_total() {
    let g = geometry();
    let q = RegionQuery {
        limit: 2,
        ..query(rect(8.0, 6.0, 16.0, 14.0), &[Kind::Track, Kind::Via])
    };
    let r = g.query_region(&q);
    assert_eq!(r.hits.len(), 2);
    assert_eq!(r.total, 4);
    assert!(r.truncated);
}

#[test]
fn an_empty_area_returns_nothing_and_is_not_truncated() {
    let r = geometry().query_region(&query(rect(100.0, 100.0, 101.0, 101.0), &[]));
    assert_eq!((r.total, r.hits.len(), r.truncated), (0, 0, false));
}

#[test]
fn the_outline_can_be_queried_as_items() {
    let r = geometry().query_region(&query(rect(-1.0, -1.0, 41.0, 31.0), &[Kind::Outline]));
    assert_eq!(r.total, 2);
    let forms: Vec<&str> = r.hits.iter().map(|h| h.entry.form.as_str()).collect();
    assert!(
        forms.contains(&"outer") && forms.contains(&"cutout"),
        "{forms:?}"
    );
}

#[test]
fn a_zone_is_found_by_its_outline_and_carries_its_net() {
    let r = geometry().query_region(&query(rect(34.0, 25.0, 36.0, 26.0), &[Kind::Zone]));
    assert_eq!(r.total, 1);
    assert_eq!(r.hits[0].entry.net.as_deref(), Some("GND"));
    assert_eq!(r.hits[0].entry.name.as_deref(), Some("gnd_pour"));
}
