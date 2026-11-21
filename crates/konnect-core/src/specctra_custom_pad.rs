//! A custom pad is the union of its anchor and drawing primitives, in pad-local
//! coordinates. Fastroute keeps one convex shape on each layer of a padstack.
//! Separate shapes overwrite each other, and a concave pin becomes its convex
//! hull. Refuse those topologies before they can give the router fictitious copper.
use super::{chord_points, finite_number, xy_mm, DSN_RESOLUTION};
use anyhow::{bail, ensure, Context, Result};
use i_overlay::core::{fill_rule::FillRule, overlay::Overlay, overlay_rule::OverlayRule};
use i_overlay::i_float::int::point::IntPoint;
use i_overlay::i_shape::int::{area::UnsafeArea, path::ContourExtension};
use konnect_sexp::{planar::Pt, SexpNode};

const NM_PER_MM: f64 = 1_000_000.0;
const NM_PER_DSN_UNIT: f64 = 1000.0 / DSN_RESOLUTION as f64;
type Contour = Vec<IntPoint<i64>>;

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

/// The polygon on the DSN resolution grid, with its offset from the pin
/// already applied. Decide topology at KiCad precision first: snapping two
/// distinct edges before union could turn a detached island into a contact.
pub(super) fn outline(pad: &SexpNode, drill_mm: Option<Pt>) -> Result<Vec<(i64, i64)>> {
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
    ensure!(ring.is_convex(), "custom pad copper is concave");
    let offset = match pad.find("drill").filter(|d| d.find("offset").is_some()) {
        Some(d) => xy_mm(d, "offset")?,
        None => (0.0, 0.0),
    };
    let (ox, oy) = (nm(offset.0)?, nm(-offset.1)?);
    let copper: Contour = ring
        .iter()
        .map(|p| IntPoint::new(p.x + ox, p.y + oy))
        .collect();
    let snap = |n: i64| (n as f64 / NM_PER_DSN_UNIT).round() as i64;
    let mut projected: Contour = copper
        .iter()
        .map(|p| IntPoint::new(snap(p.x), snap(p.y)))
        .collect();
    projected.dedup();
    if projected.first() == projected.last() {
        projected.pop();
    }
    ensure!(
        projected.len() >= 3
            && projected.iter().copied().unsafe_area() > 0
            && projected.is_convex(),
        "custom pad is not a convex area at DSN resolution"
    );
    if let Some((dx, dy)) = drill_mm {
        let size_nm = (dx * NM_PER_MM, dy * NM_PER_MM);
        let size_dsn = (size_nm.0 / NM_PER_DSN_UNIT, size_nm.1 / NM_PER_DSN_UNIT);
        ensure!(
            encloses_drill(&copper, size_nm) && encloses_drill(&projected, size_dsn),
            "custom pad copper does not enclose the drill with a positive ring"
        );
    }
    Ok(projected.iter().map(|p| (p.x, p.y)).collect())
}
