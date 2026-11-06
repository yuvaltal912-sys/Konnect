//! The true board outline: every `Edge.Cuts` graphic joined into closed rings,
//! with containment and overhang measured against those rings.
//!
//! [`crate::board::board_outline_shape`] answers a narrower question — "is the
//! outline provably an axis-aligned rectangle?" — and gives up on anything
//! else. This module answers the real one. A rounded corner, a notch, a
//! circular cut-out and a polygon outline are all just rings here.
//!
//! # Exactness
//!
//! Lines, rectangles and polygons are exact. Arcs, circles and Béziers are
//! tessellated into polylines whose chords stay within
//! [`Outline::tolerance_mm`] of the true curve (the polygon lies on or just
//! inside a convex arc). A point that is further than the tolerance from the
//! boundary is classified exactly; one closer than that may land either side.
//!
//! # What counts as the outline
//!
//! Every direct `gr_line`, `gr_arc`, `gr_circle`, `gr_rect`, `gr_poly` and
//! `gr_curve` on `Edge.Cuts`, plus the same shapes drawn on `Edge.Cuts` inside a
//! footprint (a mounting slot, say), moved into board coordinates. Open
//! graphics are joined end to end when their endpoints are within
//! [`JOIN_TOLERANCE_MM`]. A ring inside another ring is a hole; the even-odd
//! rule decides which side of the boundary is board.
//!
//! The outline is only usable for containment when [`Outline::status`] is
//! [`OutlineStatus::Closed`]. Anything left open or unreadable is counted and
//! reported rather than guessed at.

use crate::board::footprints;
use crate::parser::SexpNode;
use crate::planar::{
    bbox_of, dist, flatten_bezier, near, point_in_ring, point_segment_distance, ring_area,
    segment_distance, tessellate_arc, tessellate_circle, Pt,
};

/// Default tessellation tolerance, in millimetres.
pub const DEFAULT_TOLERANCE_MM: f64 = 0.005;

/// Two outline endpoints this close are one vertex. KiCad joins outline
/// segments with a tolerance of the same order.
pub const JOIN_TOLERANCE_MM: f64 = 0.01;

/// A point this close to the boundary is treated as on it, and so inside.
const ON_EDGE_MM: f64 = 1e-4;

/// Whether the outline can answer containment questions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineStatus {
    /// No `Edge.Cuts` graphics at all.
    Missing,
    /// At least one graphic could not be read (a missing or non-finite
    /// coordinate, or the pre-KiCad-7 arc form); containment is not claimed.
    Unreadable,
    /// The graphics read cleanly but at least one chain does not close.
    Open,
    /// At least one ring, nothing left open, nothing unreadable.
    Closed,
}

impl OutlineStatus {
    /// The name a response reports this status under.
    pub fn key(self) -> &'static str {
        match self {
            OutlineStatus::Missing => "missing",
            OutlineStatus::Unreadable => "unreadable",
            OutlineStatus::Open => "open",
            OutlineStatus::Closed => "closed",
        }
    }
}

/// The board outline as closed rings.
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    /// Closed rings (last vertex implicitly joins the first), outer boundaries
    /// and holes alike.
    pub loops: Vec<Vec<Pt>>,
    /// Chains of graphics whose ends did not meet.
    pub open_chains: usize,
    /// `Edge.Cuts` graphics that could not be read.
    pub unreadable: usize,
    /// How many `Edge.Cuts` graphics were found.
    pub graphics: usize,
    /// Whether any graphic was an arc, circle or Bézier (so tessellated).
    pub curved: bool,
    /// The tessellation tolerance used, in millimetres.
    pub tolerance_mm: f64,
}

/// How far a polygon sticks out of the outline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Overhang {
    /// The deepest point outside the board, in mm from the board edge. 0 when
    /// the polygon is inside.
    pub depth_mm: f64,
    /// Where that deepest point is.
    pub at: Pt,
    /// Whether any sampled point of the polygon is on the board.
    pub touches_board: bool,
}

impl Outline {
    /// Read the outline from a parsed `(kicad_pcb …)` with the default
    /// tolerance.
    pub fn from_tree(tree: &SexpNode) -> Self {
        Self::from_tree_with(tree, DEFAULT_TOLERANCE_MM)
    }

    /// Read the outline with an explicit tessellation tolerance.
    pub fn from_tree_with(tree: &SexpNode, tolerance_mm: f64) -> Self {
        let tol = tolerance_mm.max(1e-6);
        let mut pieces = Pieces::default();

        for child in tree.children().unwrap_or(&[]) {
            let Some(head) = child.head() else { continue };
            if child.find_str("layer") != Some("Edge.Cuts") {
                continue;
            }
            if let Some(kind) = head.strip_prefix("gr_") {
                pieces.add(kind, child, tol, &|p| p);
            }
        }
        for fp in footprints(tree) {
            let Some(at) = fp.find("at") else { continue };
            let (Some(fx), Some(fy)) = (at.get_f64(1), at.get_f64(2)) else {
                continue;
            };
            let rot = at.get_f64(3).unwrap_or(0.0);
            for child in fp.children().unwrap_or(&[]) {
                let Some(head) = child.head() else { continue };
                if child.find_str("layer") != Some("Edge.Cuts") {
                    continue;
                }
                if let Some(kind) = head.strip_prefix("fp_") {
                    let to_board =
                        move |p: Pt| crate::geometry::transform_pad(p.0, p.1, fx, fy, rot);
                    pieces.add(kind, child, tol, &to_board);
                }
            }
        }

        let (mut loops, open_chains) = join_chains(pieces.chains, JOIN_TOLERANCE_MM);
        loops.extend(pieces.rings);
        loops.retain(|ring| ring.len() >= 3 && ring_area(ring).abs() > 1e-9);

        Outline {
            loops,
            open_chains,
            unreadable: pieces.unreadable,
            graphics: pieces.graphics,
            curved: pieces.curved,
            tolerance_mm: tol,
        }
    }

    /// Whether the outline can answer containment questions.
    pub fn status(&self) -> OutlineStatus {
        if self.graphics == 0 {
            OutlineStatus::Missing
        } else if self.unreadable > 0 {
            OutlineStatus::Unreadable
        } else if self.open_chains > 0 || self.loops.is_empty() {
            OutlineStatus::Open
        } else {
            OutlineStatus::Closed
        }
    }

    /// The overhang below which a part counts as on the board. A polygon of
    /// straight edges is exact, so only floating-point noise (1 nm, KiCad's own
    /// coordinate resolution) is forgiven; once an arc has been tessellated the
    /// chord error is forgiven too.
    pub fn containment_threshold_mm(&self) -> f64 {
        if self.curved {
            self.tolerance_mm + 1e-6
        } else {
            1e-6
        }
    }

    /// `(min_x, min_y, max_x, max_y)` of every ring.
    pub fn bbox(&self) -> Option<(f64, f64, f64, f64)> {
        let all: Vec<Pt> = self.loops.iter().flatten().copied().collect();
        bbox_of(&all)
    }

    /// Rings nested an odd number of levels deep — the cut-outs.
    pub fn hole_count(&self) -> usize {
        self.loops
            .iter()
            .enumerate()
            .filter(|(i, ring)| {
                let depth = self
                    .loops
                    .iter()
                    .enumerate()
                    .filter(|(j, other)| j != i && point_in_ring(ring[0], other))
                    .count();
                depth % 2 == 1
            })
            .count()
    }

    fn edges(&self) -> impl Iterator<Item = (Pt, Pt)> + '_ {
        self.loops.iter().flat_map(|ring| {
            let n = ring.len();
            (0..n).map(move |i| (ring[i], ring[(i + 1) % n]))
        })
    }

    /// Shortest distance from `p` to the boundary, whichever side it is on.
    pub fn boundary_distance(&self, p: Pt) -> f64 {
        self.edges()
            .map(|(a, b)| point_segment_distance(p, a, b))
            .fold(f64::INFINITY, f64::min)
    }

    /// Whether `p` is on the board: inside the even-odd region or on its edge.
    pub fn contains(&self, p: Pt) -> bool {
        let crossings = self
            .loops
            .iter()
            .filter(|ring| point_in_ring(p, ring))
            .count();
        crossings % 2 == 1 || self.boundary_distance(p) <= ON_EDGE_MM
    }

    /// 0 when `p` is on the board, otherwise its distance to the board edge.
    pub fn outside_distance(&self, p: Pt) -> f64 {
        if self.contains(p) {
            0.0
        } else {
            self.boundary_distance(p)
        }
    }

    /// How far the closed polygon `poly` sticks out of the board.
    ///
    /// The polygon's boundary is sampled at every vertex and at least every
    /// 0.1 mm along its edges, and each sample is measured against the board.
    /// Outline vertices lying inside the polygon count too: a notch or cut-out
    /// that bites into a part is an overhang even when every vertex of the part
    /// is on the board.
    pub fn overhang(&self, poly: &[Pt]) -> Overhang {
        const STEP_MM: f64 = 0.1;
        const MAX_SAMPLES: usize = 4000;

        let Some(bb) = bbox_of(poly) else {
            return Overhang {
                depth_mm: 0.0,
                at: (0.0, 0.0),
                touches_board: true,
            };
        };
        // Fast path: when no outline edge reaches the polygon's bounding box,
        // the whole box is on one side of the boundary and one vertex says
        // which.
        if !self.edges().any(|(a, b)| edge_touches_box(a, b, bb)) {
            return if self.contains(poly[0]) {
                Overhang {
                    depth_mm: 0.0,
                    at: poly[0],
                    touches_board: true,
                }
            } else {
                let (depth, at) = poly.iter().map(|&p| (self.outside_distance(p), p)).fold(
                    (0.0, poly[0]),
                    |acc, cur| if cur.0 > acc.0 { cur } else { acc },
                );
                Overhang {
                    depth_mm: depth,
                    at,
                    touches_board: false,
                }
            };
        }

        let n = poly.len();
        let perimeter: f64 = (0..n).map(|i| dist(poly[i], poly[(i + 1) % n])).sum();
        let step = STEP_MM.max(perimeter / MAX_SAMPLES as f64);
        let mut worst = (0.0_f64, poly[0]);
        let mut touches = false;
        let mut consider = |p: Pt, touches: &mut bool| {
            let d = self.outside_distance(p);
            if d == 0.0 {
                *touches = true;
            } else if d > worst.0 {
                worst = (d, p);
            }
        };
        for i in 0..n {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            let k = (dist(a, b) / step).ceil().max(1.0) as usize;
            for s in 0..k {
                let t = s as f64 / k as f64;
                consider((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)), &mut touches);
            }
        }
        // Outline vertices inside the polygon: the board edge runs through it.
        for &v in self.loops.iter().flatten() {
            if point_in_ring(v, poly) {
                let depth = (0..n)
                    .map(|i| point_segment_distance(v, poly[i], poly[(i + 1) % n]))
                    .fold(f64::INFINITY, f64::min);
                if depth > worst.0 {
                    worst = (depth, v);
                }
                touches = true;
            }
        }
        Overhang {
            depth_mm: worst.0,
            at: worst.1,
            touches_board: touches,
        }
    }
}

/// Whether the segment `a`–`b` meets the box.
fn edge_touches_box(a: Pt, b: Pt, (x0, y0, x1, y1): (f64, f64, f64, f64)) -> bool {
    let inside = |p: Pt| p.0 >= x0 && p.0 <= x1 && p.1 >= y0 && p.1 <= y1;
    if inside(a) || inside(b) {
        return true;
    }
    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    (0..4).any(|i| segment_distance(a, b, corners[i], corners[(i + 1) % 4]) == 0.0)
}

/// Graphics gathered while reading, before chains are joined.
#[derive(Default)]
struct Pieces {
    /// Open runs of vertices still to be joined end to end.
    chains: Vec<Vec<Pt>>,
    /// Shapes that close by themselves (circle, rectangle, polygon).
    rings: Vec<Vec<Pt>>,
    unreadable: usize,
    graphics: usize,
    curved: bool,
}

impl Pieces {
    /// Add one `Edge.Cuts` graphic. `kind` is the tag without its `gr_`/`fp_`
    /// prefix; `to_board` maps its coordinates into board space.
    fn add(&mut self, kind: &str, node: &SexpNode, tol: f64, to_board: &dyn Fn(Pt) -> Pt) {
        let supported = matches!(kind, "line" | "arc" | "circle" | "rect" | "poly" | "curve");
        if !supported {
            return;
        }
        self.graphics += 1;
        let pt = |tag: &str| -> Option<Pt> {
            let n = node.find(tag)?;
            let (x, y) = (n.get_f64(1)?, n.get_f64(2)?);
            (x.is_finite() && y.is_finite()).then(|| to_board((x, y)))
        };
        let parsed = (|| -> Option<()> {
            match kind {
                "line" => {
                    let (s, e) = (pt("start")?, pt("end")?);
                    if dist(s, e) > 0.0 {
                        self.chains.push(vec![s, e]);
                    }
                }
                "arc" => {
                    self.curved = true;
                    self.chains
                        .push(tessellate_arc(pt("start")?, pt("mid")?, pt("end")?, tol));
                }
                "circle" => {
                    self.curved = true;
                    let (c, e) = (pt("center")?, pt("end")?);
                    self.rings.push(tessellate_circle(c, dist(c, e), tol));
                }
                "rect" => {
                    // A rectangle under a footprint rotation is the four mapped
                    // corners, not the mapped bbox of two.
                    let raw = |tag: &str| {
                        let n = node.find(tag)?;
                        let (x, y) = (n.get_f64(1)?, n.get_f64(2)?);
                        (x.is_finite() && y.is_finite()).then_some((x, y))
                    };
                    let (rs, re) = (raw("start")?, raw("end")?);
                    let corners = [rs, (re.0, rs.1), re, (rs.0, re.1)].map(to_board);
                    self.rings.push(corners.to_vec());
                }
                "poly" => {
                    let pts = node.find("pts")?;
                    let mut ring = Vec::new();
                    for xy in pts.find_all("xy") {
                        let (x, y) = (xy.get_f64(1)?, xy.get_f64(2)?);
                        if !x.is_finite() || !y.is_finite() {
                            return None;
                        }
                        ring.push(to_board((x, y)));
                    }
                    if ring.len() > 1 && near(ring[0], ring[ring.len() - 1], 0.0) {
                        ring.pop();
                    }
                    self.rings.push(ring);
                }
                "curve" => {
                    self.curved = true;
                    let p = node.find("pts")?.find_all("xy");
                    if p.len() != 4 {
                        return None;
                    }
                    let q: Vec<Pt> = p
                        .iter()
                        .map(|n| Some(to_board((n.get_f64(1)?, n.get_f64(2)?))))
                        .collect::<Option<_>>()?;
                    self.chains
                        .push(flatten_bezier(q[0], q[1], q[2], q[3], tol));
                }
                _ => {}
            }
            Some(())
        })();
        if parsed.is_none() {
            self.unreadable += 1;
        }
    }
}

/// Join chains end to end. Returns the closed rings and how many chains were
/// left open.
fn join_chains(mut chains: Vec<Vec<Pt>>, eps: f64) -> (Vec<Vec<Pt>>, usize) {
    let mut rings = Vec::new();
    let mut open = 0;
    while let Some(mut cur) = chains.pop() {
        let mut flipped = false;
        loop {
            let first = cur[0];
            let last = cur[cur.len() - 1];
            if cur.len() >= 3 && near(first, last, eps) {
                cur.pop();
                rings.push(cur);
                break;
            }
            let attach = chains
                .iter()
                .position(|c| near(c[0], last, eps))
                .map(|i| (i, false))
                .or_else(|| {
                    chains
                        .iter()
                        .position(|c| near(c[c.len() - 1], last, eps))
                        .map(|i| (i, true))
                });
            match attach {
                Some((i, reverse)) => {
                    let mut next = chains.swap_remove(i);
                    if reverse {
                        next.reverse();
                    }
                    cur.extend(next.into_iter().skip(1));
                }
                None if !flipped => {
                    cur.reverse();
                    flipped = true;
                }
                None => {
                    open += 1;
                    break;
                }
            }
        }
    }
    (rings, open)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_sexp;

    fn board(body: &str) -> SexpNode {
        parse_sexp(&format!("(kicad_pcb (version 20240108) {body})")).unwrap()
    }

    fn line(x1: f64, y1: f64, x2: f64, y2: f64) -> String {
        format!("(gr_line (start {x1} {y1}) (end {x2} {y2}) (layer \"Edge.Cuts\") (width 0.05))")
    }

    /// A 40 × 30 board whose four corners are quarter-circle arcs of radius 5,
    /// the way `add_board_outline` draws a rounded rectangle.
    fn rounded_body() -> String {
        let r = 5.0;
        let k = r * (1.0 - std::f64::consts::FRAC_1_SQRT_2);
        let mut s = String::new();
        s += &line(5.0, 0.0, 35.0, 0.0);
        s += &line(40.0, 5.0, 40.0, 25.0);
        s += &line(35.0, 30.0, 5.0, 30.0);
        s += &line(0.0, 25.0, 0.0, 5.0);
        for (start, mid, end) in [
            ((35.0, 0.0), (40.0 - k, k), (40.0, 5.0)),
            ((40.0, 25.0), (40.0 - k, 30.0 - k), (35.0, 30.0)),
            ((5.0, 30.0), (k, 30.0 - k), (0.0, 25.0)),
            ((0.0, 5.0), (k, k), (5.0, 0.0)),
        ] {
            s += &format!(
                "(gr_arc (start {} {}) (mid {} {}) (end {} {}) (layer \"Edge.Cuts\") (width 0.05))",
                start.0, start.1, mid.0, mid.1, end.0, end.1
            );
        }
        s
    }

    #[test]
    fn a_missing_outline_is_missing_not_closed() {
        let o = Outline::from_tree(&board(""));
        assert_eq!(o.status(), OutlineStatus::Missing);
    }

    #[test]
    fn four_lines_close_into_one_ring() {
        let body = [
            line(0.0, 0.0, 10.0, 0.0),
            line(10.0, 0.0, 10.0, 10.0),
            line(10.0, 10.0, 0.0, 10.0),
            line(0.0, 10.0, 0.0, 0.0),
        ]
        .concat();
        let o = Outline::from_tree(&board(&body));
        assert_eq!(o.status(), OutlineStatus::Closed);
        assert_eq!(o.loops.len(), 1);
        assert!(o.contains((5.0, 5.0)));
        assert!(!o.contains((11.0, 5.0)));
    }

    #[test]
    fn an_outline_with_a_gap_is_open_and_says_so() {
        let body = [
            line(0.0, 0.0, 10.0, 0.0),
            line(10.0, 0.0, 10.0, 10.0),
            line(10.0, 10.0, 0.0, 10.0),
        ]
        .concat();
        let o = Outline::from_tree(&board(&body));
        assert_eq!(o.status(), OutlineStatus::Open);
        assert_eq!(o.open_chains, 1);
    }

    #[test]
    fn a_rounded_outline_is_closed_and_its_cut_corner_is_outside() {
        let o = Outline::from_tree(&board(&rounded_body()));
        assert_eq!(o.status(), OutlineStatus::Closed);
        assert!(o.curved);
        assert!(o.contains((20.0, 15.0)));
        // The sharp corner of the bounding box is not board.
        assert!(!o.contains((0.2, 0.2)));
        // Just inside the arc is board.
        assert!(o.contains((
            5.0 - 5.0 * std::f64::consts::FRAC_1_SQRT_2 + 0.2,
            5.0 - 5.0 * std::f64::consts::FRAC_1_SQRT_2 + 0.2 + 0.5
        )));
    }

    #[test]
    fn a_circular_cutout_is_a_hole() {
        let body = format!(
            "{}(gr_circle (center 20 15) (end 23 15) (layer \"Edge.Cuts\") (width 0.05))",
            [
                line(0.0, 0.0, 40.0, 0.0),
                line(40.0, 0.0, 40.0, 30.0),
                line(40.0, 30.0, 0.0, 30.0),
                line(0.0, 30.0, 0.0, 0.0)
            ]
            .concat()
        );
        let o = Outline::from_tree(&board(&body));
        assert_eq!(o.status(), OutlineStatus::Closed);
        assert_eq!(o.loops.len(), 2);
        assert_eq!(o.hole_count(), 1);
        assert!(o.contains((10.0, 15.0)));
        assert!(!o.contains((20.0, 15.0)), "the cut-out is not board");
    }

    #[test]
    fn the_legacy_arc_form_is_unreadable_not_guessed() {
        let body = "(gr_arc (start 5 5) (end 10 5) (angle 90) (layer \"Edge.Cuts\") (width 0.05))";
        let o = Outline::from_tree(&board(body));
        assert_eq!(o.status(), OutlineStatus::Unreadable);
    }

    #[test]
    fn a_part_over_a_notch_is_an_overhang_even_with_every_vertex_on_board() {
        // 40 × 30 board with a slot cut in from the bottom edge, 2 mm wide,
        // 10 mm deep; a 6 × 4 part straddles the slot's tip.
        let body = [
            line(0.0, 0.0, 19.0, 0.0),
            line(19.0, 0.0, 19.0, 10.0),
            line(19.0, 10.0, 21.0, 10.0),
            line(21.0, 10.0, 21.0, 0.0),
            line(21.0, 0.0, 40.0, 0.0),
            line(40.0, 0.0, 40.0, 30.0),
            line(40.0, 30.0, 0.0, 30.0),
            line(0.0, 30.0, 0.0, 0.0),
        ]
        .concat();
        let o = Outline::from_tree(&board(&body));
        assert_eq!(o.status(), OutlineStatus::Closed);
        let part = [(17.0, 8.0), (23.0, 8.0), (23.0, 12.0), (17.0, 12.0)];
        let over = o.overhang(&part);
        assert!(
            over.depth_mm > 1.5,
            "the slot bites 2 mm into the part: {over:?}"
        );
        // The same part well clear of the slot is inside.
        let clear = [(5.0, 15.0), (11.0, 15.0), (11.0, 19.0), (5.0, 19.0)];
        assert_eq!(o.overhang(&clear).depth_mm, 0.0);
    }

    #[test]
    fn overhang_is_measured_in_mm_past_the_edge() {
        let body = [
            line(0.0, 0.0, 10.0, 0.0),
            line(10.0, 0.0, 10.0, 10.0),
            line(10.0, 10.0, 0.0, 10.0),
            line(0.0, 10.0, 0.0, 0.0),
        ]
        .concat();
        let o = Outline::from_tree(&board(&body));
        let part = [(8.0, 4.0), (12.5, 4.0), (12.5, 6.0), (8.0, 6.0)];
        let over = o.overhang(&part);
        assert!((over.depth_mm - 2.5).abs() < 1e-6, "{over:?}");
        assert!(over.touches_board);
        let away = [(20.0, 4.0), (22.0, 4.0), (22.0, 6.0), (20.0, 6.0)];
        let far = o.overhang(&away);
        assert!(!far.touches_board);
        assert!((far.depth_mm - 12.0).abs() < 1e-6, "{far:?}");
    }

    #[test]
    fn a_footprint_edge_cut_slot_is_part_of_the_outline() {
        let body = format!(
            "{}(footprint \"Slot\" (layer \"F.Cu\") (at 20 15 90) (fp_circle (center 0 0) (end 2 0) (layer \"Edge.Cuts\") (width 0.05)))",
            [line(0.0, 0.0, 40.0, 0.0), line(40.0, 0.0, 40.0, 30.0), line(40.0, 30.0, 0.0, 30.0), line(0.0, 30.0, 0.0, 0.0)].concat()
        );
        let o = Outline::from_tree(&board(&body));
        assert_eq!(o.status(), OutlineStatus::Closed);
        assert_eq!(o.hole_count(), 1);
        assert!(!o.contains((20.0, 15.0)));
    }
}
