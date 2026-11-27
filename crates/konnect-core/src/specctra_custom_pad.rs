//! A custom pad is the union of its anchor and drawing primitives, in pad-local
//! coordinates. Fastroute keeps one convex shape on each layer of a padstack.
//! Separate shapes overwrite each other, and a concave pin becomes its convex
//! hull. Refuse islands and holes before they can give the router fictitious
//! copper. A concave pad is split instead (see [`split_concave`]): the pin gets
//! a convex part of the copper, and the rest is kept from the router as keepouts.
use super::{chord_points, finite_number, xy_mm, DSN_RESOLUTION};
use anyhow::{bail, ensure, Context, Result};
use i_overlay::core::{fill_rule::FillRule, overlay::Overlay, overlay_rule::OverlayRule};
use i_overlay::i_float::int::point::IntPoint;
use i_overlay::i_shape::int::{area::UnsafeArea, path::ContourExtension};
use konnect_sexp::{planar::Pt, SexpNode};

const NM_PER_MM: f64 = 1_000_000.0;
const NM_PER_DSN_UNIT: f64 = 1000.0 / DSN_RESOLUTION as f64;
pub(super) type Contour = Vec<IntPoint<i64>>;

fn rectangle(a: Pt, b: Pt) -> Vec<Pt> {
    vec![a, (b.0, a.1), b, (a.0, b.1)]
}

/// Include all four tangent extrema so an adjoining straight edge meets the
/// circle even after rounding to the DSN grid. Use the exporter's chord error.
fn circle((cx, cy): Pt, radius: f64) -> Result<Vec<Pt>> {
    ensure!(
        radius.is_finite() && radius > 0.0,
        "circle has no usable radius"
    );
    let mut quadrant = vec![(radius, 0.0)];
    quadrant.extend(chord_points(
        0.0,
        std::f64::consts::FRAC_PI_2,
        (0.0, 0.0),
        radius,
    )?);
    let mut points = Vec::with_capacity(quadrant.len() * 4);
    for _ in 0..4 {
        points.extend(quadrant.iter().map(|&(x, y)| (cx + x, cy + y)));
        for p in &mut quadrant {
            *p = (-p.1, p.0);
        }
    }
    Ok(points)
}

/// A KiCad internal coordinate is a signed 32-bit number of nanometres.
fn nm(mm: f64) -> Result<i64> {
    let n = (mm * NM_PER_MM).round();
    ensure!(
        (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&n),
        "custom pad exceeds KiCad's coordinate range"
    );
    Ok(n as i64)
}

fn mm(nm: i64) -> f64 {
    nm as f64 / NM_PER_MM
}

fn grid_contour(points: Vec<Pt>) -> Result<Contour> {
    let mut ring = Vec::with_capacity(points.len());
    for (x, y) in points {
        ring.push(IntPoint::new(nm(x)?, nm(-y)?));
    }
    ensure!(
        ring.len() >= 3,
        "custom pad contour needs at least three points"
    );
    let area = ring.iter().copied().unsafe_area();
    ensure!(
        area != 0,
        "custom pad contour has no area on the KiCad grid"
    );
    // All solids must wind the same way. Otherwise overlapping copper cancels
    // under the non-zero fill rule (KiCad unions them regardless of winding).
    if area < 0 {
        ring.reverse();
    }
    Ok(ring)
}

/// Sweep the same inscribed disk used for the anchor along a line. Using
/// separately tessellated semicircles can cut small notches into a stroke
/// that shares an endpoint with a circular anchor.
fn stroke(a: Pt, b: Pt, radius: f64) -> Result<Vec<Pt>> {
    if a == b {
        return circle(a, radius);
    }
    let disk = circle((0.0, 0.0), radius)?;
    let normal = (a.1 - b.1, b.0 - a.0);
    let support = |p: Pt| p.0 * normal.0 + p.1 * normal.1;
    let start = (0..disk.len())
        .max_by(|&i, &j| support(disk[i]).total_cmp(&support(disk[j])))
        .unwrap();
    let half = disk.len() / 2;
    let mut ring = Vec::with_capacity(disk.len() + 2);
    for (vertex, centre) in [(start, a), (start + half, b)] {
        for i in 0..=half {
            let (x, y) = disk[(vertex + i) % disk.len()];
            ring.push((centre.0 + x, centre.1 + y));
        }
    }
    Ok(ring)
}

fn primitive(node: &SexpNode) -> Result<Option<Vec<Pt>>> {
    let kind = node.head().context("invalid custom pad primitive")?;
    match kind {
        // KiCad skips these proxy shapes in PAD::MergePrimitivesAsPolygon.
        "gr_bbox" | "gr_vector" => return Ok(None),
        "gr_circle" | "gr_poly" | "gr_rect" | "gr_line" => {}
        _ => bail!("unsupported custom pad primitive '{kind}'"),
    }
    let stroke_node = node.find("stroke").unwrap_or(node);
    let width = match stroke_node.find("width") {
        Some(w) => finite_number(w.get_f64(1), "primitive width")?,
        None => 0.0,
    };
    ensure!(width >= 0.0, "negative primitive width");
    ensure!(
        matches!(
            stroke_node.find_str("type"),
            None | Some("default" | "solid")
        ),
        "unsupported {kind} stroke style"
    );
    if kind == "gr_line" {
        ensure!(width > 0.0, "a custom pad line needs positive width");
        let (a, b) = (xy_mm(node, "start")?, xy_mm(node, "end")?);
        return stroke(a, b, mm(nm(width)? / 2)).map(Some);
    }
    // KiCad's legacy zero-width rectangles/circles and legacy polygons are
    // filled when the fill field is absent. A present, unknown fill is refused.
    let filled = match node.find("fill") {
        Some(fill) => matches!(
            fill.get(1).and_then(SexpNode::as_str),
            Some("yes" | "solid")
        ),
        None => width == 0.0 || kind == "gr_poly",
    };
    ensure!(
        filled,
        "unsupported unfilled or patterned {kind} in custom pad"
    );
    let points = match kind {
        "gr_circle" => {
            let (c, end) = (xy_mm(node, "center")?, xy_mm(node, "end")?);
            let radius = nm((end.0 - c.0).hypot(end.1 - c.1))?.clamp(1, i64::from(i32::MAX) / 2);
            circle(c, mm(radius + nm(width)? / 2))?
        }
        "gr_rect" => {
            ensure!(width == 0.0, "unsupported stroked custom pad rectangle");
            if let Some(radius) = node.find("radius") {
                ensure!(
                    finite_number(radius.get_f64(1), "rectangle corner radius")? == 0.0,
                    "unsupported rounded custom pad rectangle"
                );
            }
            rectangle(xy_mm(node, "start")?, xy_mm(node, "end")?)
        }
        _ => {
            ensure!(width == 0.0, "unsupported stroked custom pad polygon");
            let pts = node
                .find("pts")
                .context("custom pad polygon has no points")?;
            let mut points = Vec::new();
            for xy in pts.children().unwrap_or_default().iter().skip(1) {
                ensure!(
                    xy.head() == Some("xy"),
                    "unsupported custom pad polygon vertex"
                );
                points.push((
                    finite_number(xy.get_f64(1), "polygon x")?,
                    finite_number(xy.get_f64(2), "polygon y")?,
                ));
            }
            if points.len() > 1 && points.first() == points.last() {
                points.pop();
            }
            crate::tools::pcb_board::check_simple_ring(&points)
                .map_err(|reason| anyhow::anyhow!("invalid custom pad polygon: {reason}"))?;
            points
        }
    };
    Ok(Some(points))
}

/// A slot is the union of disks along its centreline. Both end disks must
/// lie strictly inside every edge of the CCW convex copper (a positive ring).
fn encloses_drill(copper: &[IntPoint<i64>], (w, h): Pt) -> bool {
    let radius = w.min(h) * 0.5;
    let (ex, ey) = ((w - h).max(0.0) * 0.5, (h - w).max(0.0) * 0.5);
    copper
        .iter()
        .zip(copper.iter().cycle().skip(1))
        .all(|(a, b)| {
            let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
            let centre_cross = dy * a.x as f64 - dx * a.y as f64;
            centre_cross - (dx * ey - dy * ex).abs() > radius * dx.hypot(dy)
        })
}

/// A custom pad's copper as the router is given it, on the DSN resolution grid
/// in the pad's frame (Y up), its offset from the pin already applied.
#[derive(Debug)]
pub(super) struct CustomCopper {
    /// The pin's convex polygon.
    pub(super) pin: Vec<(i64, i64)>,
    /// Copper of a concave pad outside the pin, one simple polygon each, to be
    /// written as keepouts: the router routes no other net across it, and its
    /// own net reaches the pad through the pin. Empty for a convex pad.
    pub(super) rest: Vec<Vec<(i64, i64)>>,
}

/// Twice the signed area (positive counter-clockwise, as `unsafe_area`).
pub(super) fn area2(ring: &[IntPoint<i64>]) -> i128 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(p, q)| i128::from(p.x) * i128::from(q.y) - i128::from(p.y) * i128::from(q.x))
        .sum()
}

/// Positive when `p` lies left of the directed line from `a` to `b`.
pub(super) fn side(a: IntPoint<i64>, b: IntPoint<i64>, p: IntPoint<i64>) -> i128 {
    i128::from(b.x - a.x) * i128::from(p.y - a.y) - i128::from(b.y - a.y) * i128::from(p.x - a.x)
}

fn length(a: IntPoint<i64>, b: IntPoint<i64>) -> f64 {
    ((b.x - a.x) as f64).hypot((b.y - a.y) as f64)
}

/// The counter-clockwise convex hull, without collinear vertices.
fn convex_hull(points: &[IntPoint<i64>]) -> Contour {
    let mut sorted: Vec<IntPoint<i64>> = points.to_vec();
    sorted.sort_by_key(|p| (p.x, p.y));
    sorted.dedup();
    if sorted.len() < 3 {
        return sorted;
    }
    let mut hull: Contour = Vec::with_capacity(sorted.len() + 1);
    for pass in [false, true] {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &IntPoint<i64>>> = if pass {
            Box::new(sorted.iter().rev())
        } else {
            Box::new(sorted.iter())
        };
        for &p in iter {
            while hull.len() >= start + 2
                && side(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0
            {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

/// The part of a counter-clockwise convex polygon on the left of the directed
/// line `a`-`b` (`left`) or on its right, crossing points rounded to the grid.
fn clip(polygon: &[IntPoint<i64>], a: IntPoint<i64>, b: IntPoint<i64>, left: bool) -> Contour {
    let sign = if left { 1 } else { -1 };
    let mut out = Vec::with_capacity(polygon.len() + 1);
    for (&p, &q) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
        let (sp, sq) = (sign * side(a, b, p), sign * side(a, b, q));
        if sp >= 0 {
            out.push(p);
        }
        if (sp > 0 && sq < 0) || (sp < 0 && sq > 0) {
            let t = sp as f64 / (sp - sq) as f64;
            out.push(IntPoint::new(
                p.x + ((q.x - p.x) as f64 * t).round() as i64,
                p.y + ((q.y - p.y) as f64 * t).round() as i64,
            ));
        }
    }
    out.dedup();
    if out.len() > 1 && out.first() == out.last() {
        out.pop();
    }
    out
}

/// Whether the segment `a`-`b` runs through the counter-clockwise convex
/// polygon at least `depth` nanometres inside every one of its edges.
fn runs_inside(polygon: &[IntPoint<i64>], a: IntPoint<i64>, b: IntPoint<i64>, depth: f64) -> bool {
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for (&p, &q) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
        let margin = depth * length(p, q);
        let (fa, fb) = (side(p, q, a) as f64 - margin, side(p, q, b) as f64 - margin);
        if fa <= 0.0 && fb <= 0.0 {
            return false;
        }
        if fa < 0.0 || fb < 0.0 {
            let t = fa / (fa - fb);
            if fa < 0.0 {
                lo = lo.max(t);
            } else {
                hi = hi.min(t);
            }
        }
    }
    lo < hi
}

/// Grid slack for a split, in nanometres: crossing points round to 1 nm.
const SPLIT_SLACK_NM: f64 = 2.0;
/// The pin's centre stays this far inside every cut: twice the 5 µm the rest's
/// keepouts are widened by, so the centre is never under a keepout.
const SEED_MARGIN_NM: f64 = 10_000.0;

fn perimeter(ring: &[IntPoint<i64>]) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(&p, &q)| length(p, q))
        .sum()
}

/// Twice the area of `subject` outside `clip`.
pub(super) fn area2_outside(subject: &Contour, clip: &Contour) -> i128 {
    Overlay::from_subj_and_clip(&vec![subject.clone()], &vec![clip.clone()])
        .overlay(OverlayRule::Difference, FillRule::NonZero)
        .iter()
        .flat_map(|shape| shape.iter())
        .map(|contour| area2(contour))
        .sum()
}

/// A counter-clockwise convex polygon with each edge moved out by `d`
/// nanometres, its corners mitred.
pub(super) fn grown(polygon: &[IntPoint<i64>], d: f64) -> Contour {
    let n = polygon.len();
    let line = |i: usize| {
        let (p, q) = (polygon[i], polygon[(i + 1) % n]);
        let (dx, dy) = ((q.x - p.x) as f64, (q.y - p.y) as f64);
        let len = dx.hypot(dy);
        // Outward is to the right of a counter-clockwise edge.
        (
            (p.x as f64 + d * dy / len, p.y as f64 - d * dx / len),
            (dx, dy),
        )
    };
    (0..n)
        .map(|i| {
            let ((ax, ay), (adx, ady)) = line((i + n - 1) % n);
            let ((bx, by), (bdx, bdy)) = line(i);
            let cross = adx * bdy - ady * bdx;
            let t = ((bx - ax) * bdy - (by - ay) * bdx) / cross;
            IntPoint::new((ax + t * adx).round() as i64, (ay + t * ady).round() as i64)
        })
        .collect()
}

/// Split concave copper (counter-clockwise, in nanometres) into a convex pin
/// that holds `seed`, the point the router attaches the pin at, and the copper
/// the pin leaves out.
///
/// The pin starts as the copper's convex hull. While an edge of the copper runs
/// through it, the hull is cut along one such edge's line, keeping the side of
/// `seed`: of all those cuts, the one that keeps the largest pin. A cut whose
/// line passes within [`SEED_MARGIN_NM`] of `seed` is never made; when only such
/// cuts are left the pad is refused. Each cut takes its edge out of the pin, so
/// the cuts end within the copper's edge count. Then each cut a later one made
/// needless is dropped, while that leaves a larger pin. The rest is the copper outside
/// the pin grown by the grid slack, so the rounding of the cuts leaves no
/// slivers along the pin.
pub(super) fn split_concave(
    copper: &Contour,
    seed: IntPoint<i64>,
) -> Result<(Contour, Vec<Contour>)> {
    type Cut = (IntPoint<i64>, IntPoint<i64>, bool);
    let crossing = |pin: &Contour| -> Vec<(IntPoint<i64>, IntPoint<i64>)> {
        copper
            .iter()
            .zip(copper.iter().cycle().skip(1))
            .map(|(&a, &b)| (a, b))
            .filter(|&(a, b)| runs_inside(pin, a, b, SPLIT_SLACK_NM))
            .collect()
    };
    let hull = convex_hull(copper);
    let apply = |cuts: &[Cut]| {
        cuts.iter()
            .fold(hull.clone(), |pin, &(a, b, left)| clip(&pin, a, b, left))
    };
    let mut cuts: Vec<Cut> = Vec::new();
    let mut pin = hull.clone();
    for _ in 0..=copper.len() {
        let edges = crossing(&pin);
        if edges.is_empty() {
            break;
        }
        let mut best: Option<(i128, Contour, Cut)> = None;
        for (a, b) in edges {
            let s = side(a, b, seed);
            if (s.unsigned_abs() as f64) < SEED_MARGIN_NM * length(a, b) {
                continue;
            }
            let cut = clip(&pin, a, b, s > 0);
            let area = area2(&cut);
            if cut.len() >= 3 && best.as_ref().is_none_or(|(kept, ..)| area > *kept) {
                best = Some((area, cut, (a, b, s > 0)));
            }
        }
        let Some((_, cut, made)) = best else {
            bail!(
                "custom pad copper is concave, and no convex part of it holds the pad's centre \
                 10 um inside it"
            );
        };
        pin = cut;
        cuts.push(made);
    }
    ensure!(
        crossing(&pin).is_empty(),
        "custom pad copper is concave, and cutting it into a convex pin did not end"
    );
    // A later cut can make an earlier one needless: drop each cut whose removal
    // still leaves a pin no copper edge runs into, and a larger one.
    loop {
        let area = area2(&pin);
        let wider = (0..cuts.len()).find_map(|i| {
            let mut fewer = cuts.clone();
            fewer.remove(i);
            let candidate = apply(&fewer);
            (candidate.len() >= 3 && area2(&candidate) > area && crossing(&candidate).is_empty())
                .then_some((fewer, candidate))
        });
        let Some((fewer, candidate)) = wider else {
            break;
        };
        cuts = fewer;
        pin = candidate;
    }
    ensure!(
        (area2_outside(&pin, copper) as f64) <= 4.0 * SPLIT_SLACK_NM * perimeter(&pin),
        "custom pad copper could not be split into a convex pin inside it"
    );
    let cover = grown(&pin, SPLIT_SLACK_NM);
    let rest: Vec<Contour> = Overlay::from_subj_and_clip(&vec![copper.clone()], &vec![cover])
        .overlay(OverlayRule::Difference, FillRule::NonZero)
        .into_iter()
        .map(|mut shape| {
            ensure!(
                shape.len() == 1,
                "custom pad copper outside its convex pin has a hole"
            );
            Ok(shape.swap_remove(0))
        })
        .collect::<Result<_>>()?;
    Ok((pin, rest))
}

/// A ring on the DSN grid, or `None` when snapping leaves no area.
fn project(ring: &[IntPoint<i64>]) -> Option<Contour> {
    let snap = |n: i64| (n as f64 / NM_PER_DSN_UNIT).round() as i64;
    let mut projected: Contour = ring
        .iter()
        .map(|p| IntPoint::new(snap(p.x), snap(p.y)))
        .collect();
    projected.dedup();
    if projected.len() > 1 && projected.first() == projected.last() {
        projected.pop();
    }
    (projected.len() >= 3 && area2(&projected) != 0).then_some(projected)
}

/// The copper on the DSN resolution grid, with its offset from the pin
/// already applied. Decide topology at KiCad precision first: snapping two
/// distinct edges before union could turn a detached island into a contact.
pub(super) fn outline(pad: &SexpNode, drill_mm: Option<Pt>) -> Result<CustomCopper> {
    let (width, height) = xy_mm(pad, "size")?;
    let (width, height) = (nm(width)?, nm(height)?);
    // KiCad centres an integer-sized rectangle at -(size/2), then adds
    // the full size. Odd nanometres therefore belong on its positive side.
    let (left, top) = (-(width / 2), -(height / 2));
    let anchor = match pad.find("options").and_then(|o| o.find("anchor")) {
        // A custom pad's default anchor in KiCad is a circle of its X size.
        None => circle((0.0, 0.0), mm(width / 2))?,
        Some(a) => match a.get(1).and_then(SexpNode::as_str) {
            Some("circle") => circle((0.0, 0.0), mm(width / 2))?,
            Some("rect") => rectangle((mm(left), mm(top)), (mm(left + width), mm(top + height))),
            other => bail!("unsupported custom pad anchor {other:?}"),
        },
    };
    let mut rings = vec![grid_contour(anchor)?];
    for primitives in pad.find_all("primitives") {
        for node in primitives.children().unwrap_or_default().iter().skip(1) {
            if let Some(points) = primitive(node)? {
                rings.push(grid_contour(points)?);
            }
        }
    }
    let shapes = Overlay::from_subj(&rings).overlay(OverlayRule::Subject, FillRule::NonZero);
    let [region] = shapes.as_slice() else {
        bail!("custom pad copper is empty or disconnected")
    };
    let [ring] = region.as_slice() else {
        bail!("custom pad copper contains a hole")
    };
    let mut ring = ring.clone();
    if area2(&ring) < 0 {
        ring.reverse();
    }
    let offset = match pad.find("drill").filter(|d| d.find("offset").is_some()) {
        Some(d) => xy_mm(d, "offset")?,
        None => (0.0, 0.0),
    };
    let (ox, oy) = (nm(offset.0)?, nm(-offset.1)?);
    // A split keeps in the pin the pad's origin, where the router attaches it
    // and a hole is drilled (before the offset moves the copper, at -offset),
    // when that lies on the copper; otherwise the anchor's centre, the origin.
    let origin = IntPoint::new(-ox, -oy);
    let seed = if ring.as_slice().contains_point(origin) {
        origin
    } else {
        IntPoint::new(0, 0)
    };
    let (pin, rest) = if ring.is_convex() {
        (ring, Vec::new())
    } else {
        split_concave(&ring, seed)?
    };
    let shift = |ring: &[IntPoint<i64>]| -> Contour {
        ring.iter()
            .map(|p| IntPoint::new(p.x + ox, p.y + oy))
            .collect()
    };
    let copper = shift(&pin);
    let projected = project(&copper)
        .filter(|projected| area2(projected) > 0 && projected.is_convex())
        .context("custom pad is not a convex area at DSN resolution")?;
    if let Some((dx, dy)) = drill_mm {
        let size_nm = (dx * NM_PER_MM, dy * NM_PER_MM);
        let size_dsn = (size_nm.0 / NM_PER_DSN_UNIT, size_nm.1 / NM_PER_DSN_UNIT);
        ensure!(
            encloses_drill(&copper, size_nm) && encloses_drill(&projected, size_dsn),
            "custom pad copper does not enclose the drill with a positive ring"
        );
    }
    let points = |ring: &[IntPoint<i64>]| ring.iter().map(|p| (p.x, p.y)).collect();
    Ok(CustomCopper {
        pin: points(&projected),
        rest: rest
            .iter()
            .filter_map(|ring| project(&shift(ring)))
            .map(|ring| points(&ring))
            .collect(),
    })
}
