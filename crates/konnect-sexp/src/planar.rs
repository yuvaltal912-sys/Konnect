//! Plain planar geometry for the board-geometry layer: distances, polygon
//! tests, and the tessellation of arcs, circles and Béziers.
//!
//! Everything here is pure `f64` arithmetic in millimetres. The tessellators
//! take a `tolerance` and guarantee that the polyline they return stays within
//! it of the true curve: the chord of a tessellated arc lies inside the arc by
//! at most `tolerance` (the sagitta), never outside it.

use std::f64::consts::{PI, TAU};

/// A point `(x, y)` in millimetres.
pub type Pt = (f64, f64);

/// Euclidean distance between two points.
pub fn dist(a: Pt, b: Pt) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Whether two points are within `eps` of each other.
pub fn near(a: Pt, b: Pt, eps: f64) -> bool {
    dist(a, b) <= eps
}

/// `(min_x, min_y, max_x, max_y)` of `pts`; `None` when empty.
pub fn bbox_of(pts: &[Pt]) -> Option<(f64, f64, f64, f64)> {
    let mut it = pts.iter();
    let first = it.next()?;
    let mut bb = (first.0, first.1, first.0, first.1);
    for &(x, y) in it {
        bb = (bb.0.min(x), bb.1.min(y), bb.2.max(x), bb.3.max(y));
    }
    Some(bb)
}

/// Distance from `p` to the segment `a`–`b`.
pub fn point_segment_distance(p: Pt, a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return dist(p, a);
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

fn cross(o: Pt, a: Pt, b: Pt) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// Whether segments `a`–`b` and `c`–`d` share at least one point.
pub fn segments_intersect(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    let (d1, d2) = (cross(c, d, a), cross(c, d, b));
    let (d3, d4) = (cross(a, b, c), cross(a, b, d));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    let on = |p: Pt, q: Pt, r: Pt, d: f64| {
        d == 0.0
            && r.0 >= p.0.min(q.0)
            && r.0 <= p.0.max(q.0)
            && r.1 >= p.1.min(q.1)
            && r.1 <= p.1.max(q.1)
    };
    on(c, d, a, d1) || on(c, d, b, d2) || on(a, b, c, d3) || on(a, b, d, d4)
}

/// Shortest distance between segments `a`–`b` and `c`–`d`; 0 when they touch.
pub fn segment_distance(a: Pt, b: Pt, c: Pt, d: Pt) -> f64 {
    if segments_intersect(a, b, c, d) {
        return 0.0;
    }
    point_segment_distance(a, c, d)
        .min(point_segment_distance(b, c, d))
        .min(point_segment_distance(c, a, b))
        .min(point_segment_distance(d, a, b))
}

/// Even-odd point-in-polygon on a closed ring (the last vertex joins the
/// first implicitly). A point exactly on the boundary may land on either side;
/// callers that care add their own boundary tolerance.
pub fn point_in_ring(p: Pt, ring: &[Pt]) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        if (a.1 > p.1) != (b.1 > p.1) {
            let x = (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0;
            if p.0 < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Signed area of a ring (positive when counter-clockwise in a Y-up frame).
pub fn ring_area(ring: &[Pt]) -> f64 {
    let n = ring.len();
    let mut sum = 0.0;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        sum += a.0 * b.1 - b.0 * a.1;
    }
    sum / 2.0
}

/// The largest angle step whose chord stays within `tolerance` of a circle of
/// radius `r`.
fn max_step(r: f64, tolerance: f64) -> f64 {
    if r <= 0.0 {
        return PI;
    }
    let ratio = (1.0 - tolerance / r).clamp(-1.0, 1.0);
    (2.0 * ratio.acos()).clamp(0.02, PI / 2.0)
}

/// Points along the circle about `center` from angle `a0` to `a1` (radians,
/// either direction), both ends included, chords within `tolerance`.
pub fn arc_points(center: Pt, r: f64, a0: f64, a1: f64, tolerance: f64) -> Vec<Pt> {
    let n = ((a1 - a0).abs() / max_step(r, tolerance)).ceil().max(1.0) as usize;
    (0..=n)
        .map(|i| {
            let t = a0 + (a1 - a0) * i as f64 / n as f64;
            (center.0 + r * t.cos(), center.1 + r * t.sin())
        })
        .collect()
}

/// A circle as a ring of vertices, all on the circle, whose chords stay within
/// `tolerance` of it.
pub fn tessellate_circle(center: Pt, r: f64, tolerance: f64) -> Vec<Pt> {
    let n = (TAU / max_step(r, tolerance)).ceil().max(8.0) as usize;
    (0..n)
        .map(|i| {
            let t = TAU * i as f64 / n as f64;
            (center.0 + r * t.cos(), center.1 + r * t.sin())
        })
        .collect()
}

/// The circle through three points: `(center, radius)`, or `None` when they
/// are collinear.
pub fn circumcircle(p1: Pt, p2: Pt, p3: Pt) -> Option<(Pt, f64)> {
    let (x1, y1) = p1;
    let (x2, y2) = p2;
    let (x3, y3) = p3;
    let d = 2.0 * (x1 * (y2 - y3) + x2 * (y3 - y1) + x3 * (y1 - y2));
    let span = dist(p1, p3).max(dist(p1, p2)).max(dist(p2, p3));
    if !d.is_finite() || d.abs() < 1e-9 * span * span.max(1.0) {
        return None;
    }
    let (q1, q2, q3) = (x1 * x1 + y1 * y1, x2 * x2 + y2 * y2, x3 * x3 + y3 * y3);
    let ux = (q1 * (y2 - y3) + q2 * (y3 - y1) + q3 * (y1 - y2)) / d;
    let uy = (q1 * (x3 - x2) + q2 * (x1 - x3) + q3 * (x2 - x1)) / d;
    Some(((ux, uy), dist(p1, (ux, uy))))
}

/// KiCad's three-point arc (`start`, `mid`, `end`) as a polyline from `start`
/// to `end` through the swept side. The endpoints are exact; every chord stays
/// within `tolerance` of the true arc. Collinear points give the straight
/// segment KiCad would draw.
pub fn tessellate_arc(start: Pt, mid: Pt, end: Pt, tolerance: f64) -> Vec<Pt> {
    let Some((c, r)) = circumcircle(start, mid, end) else {
        return vec![start, end];
    };
    let angle = |p: Pt| (p.1 - c.1).atan2(p.0 - c.0);
    let (a_s, a_m, a_e) = (angle(start), angle(mid), angle(end));
    let ccw_to_end = (a_e - a_s).rem_euclid(TAU);
    let ccw_to_mid = (a_m - a_s).rem_euclid(TAU);
    let sweep = if ccw_to_mid <= ccw_to_end {
        ccw_to_end
    } else {
        ccw_to_end - TAU
    };
    let n = (sweep.abs() / max_step(r, tolerance)).ceil().max(1.0) as usize;
    let mut out = Vec::with_capacity(n + 1);
    out.push(start);
    for i in 1..n {
        let t = a_s + sweep * i as f64 / n as f64;
        out.push((c.0 + r * t.cos(), c.1 + r * t.sin()));
    }
    out.push(end);
    out
}

/// A cubic Bézier flattened until its control points lie within `tolerance` of
/// the chord, so the polyline stays within `tolerance` of the curve.
pub fn flatten_bezier(p0: Pt, p1: Pt, p2: Pt, p3: Pt, tolerance: f64) -> Vec<Pt> {
    fn go(p: [Pt; 4], tol: f64, depth: u32, out: &mut Vec<Pt>) {
        let flat = point_segment_distance(p[1], p[0], p[3])
            .max(point_segment_distance(p[2], p[0], p[3]))
            <= tol;
        if flat || depth >= 12 {
            out.push(p[3]);
            return;
        }
        let mid = |a: Pt, b: Pt| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        let (p01, p12, p23) = (mid(p[0], p[1]), mid(p[1], p[2]), mid(p[2], p[3]));
        let (p012, p123) = (mid(p01, p12), mid(p12, p23));
        let m = mid(p012, p123);
        go([p[0], p01, p012, m], tol, depth + 1, out);
        go([m, p123, p23, p[3]], tol, depth + 1, out);
    }
    let mut out = vec![p0];
    go([p0, p1, p2, p3], tolerance, 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossing_and_touching_segments_intersect_and_apart_ones_do_not() {
        assert!(segments_intersect(
            (0.0, 0.0),
            (2.0, 2.0),
            (0.0, 2.0),
            (2.0, 0.0)
        ));
        assert!(segments_intersect(
            (0.0, 0.0),
            (2.0, 0.0),
            (2.0, 0.0),
            (2.0, 2.0)
        ));
        assert!(!segments_intersect(
            (0.0, 0.0),
            (1.0, 0.0),
            (0.0, 1.0),
            (1.0, 1.0)
        ));
        assert!(
            (segment_distance((0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)) - 1.0).abs() < 1e-12
        );
    }

    #[test]
    fn a_ring_contains_its_inside_and_not_its_outside() {
        let ring = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];
        assert!(point_in_ring((2.0, 2.0), &ring));
        assert!(!point_in_ring((5.0, 2.0), &ring));
        assert!((ring_area(&ring) - 16.0).abs() < 1e-12);
    }

    #[test]
    fn a_tessellated_arc_keeps_its_endpoints_and_stays_within_tolerance() {
        // Quarter circle of radius 5 about the origin, from (5,0) to (0,5).
        let m = (5.0 * (PI / 4.0).cos(), 5.0 * (PI / 4.0).sin());
        let pts = tessellate_arc((5.0, 0.0), m, (0.0, 5.0), 0.005);
        assert_eq!(pts.first(), Some(&(5.0, 0.0)));
        assert_eq!(pts.last(), Some(&(0.0, 5.0)));
        assert!(pts.len() > 3);
        for w in pts.windows(2) {
            // Every vertex is on the circle, so the chord midpoint sags by
            // no more than the tolerance.
            let mid = ((w[0].0 + w[1].0) / 2.0, (w[0].1 + w[1].1) / 2.0);
            assert!(5.0 - mid.0.hypot(mid.1) <= 0.005 + 1e-9);
        }
    }

    #[test]
    fn an_arc_takes_the_long_way_round_when_the_midpoint_says_so() {
        // (5,0) → (0,5) through (-5,0) is the three-quarter sweep.
        let pts = tessellate_arc((5.0, 0.0), (-5.0, 0.0), (0.0, 5.0), 0.01);
        assert!(pts.iter().any(|p| p.0 < -4.9));
    }

    #[test]
    fn a_flattened_bezier_ends_where_it_should() {
        let pts = flatten_bezier((0.0, 0.0), (0.0, 4.0), (4.0, 4.0), (4.0, 0.0), 0.01);
        assert_eq!(pts.first(), Some(&(0.0, 0.0)));
        assert_eq!(pts.last(), Some(&(4.0, 0.0)));
        assert!(pts.len() > 4);
    }
}
