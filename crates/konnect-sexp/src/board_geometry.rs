//! One geometric model of a saved board: footprints with courtyards and
//! rotation-correct pads, field text, tracks, vias, zones, graphics, and the
//! true `Edge.Cuts` outline ([`crate::outline`]).
//!
//! The tools that need "what is where" read this instead of each re-parsing the
//! board: `query_board_region`, the placement score's containment check, and
//! the outline checks in the design review and the manufacturing audit.
//!
//! # Fidelity
//!
//! - **Pads** are real polygons in board coordinates: circles, ovals and
//!   rounded rectangles are tessellated within [`TOLERANCE_MM`], rectangles and
//!   trapezoids are exact. A pad's `(at x y angle)` angle is the board-space
//!   angle (the file stores it with the footprint's rotation included), and its
//!   position goes through the footprint's rigid-body transform, the same
//!   convention [`crate::bounds::pad_bbox`] was checked against KiCad with.
//!   Custom-shape pads are the box of their primitives, flagged `approximate`.
//! - **Courtyards** are the oriented box of the drawn `F.CrtYd`/`B.CrtYd`
//!   shapes, so a part turned 45° is not inflated to its axis-aligned hull.
//! - **Field text** (Reference, Value) is a box *estimated* from the font size
//!   and character count: KiCad's glyph metrics are not available here, so the
//!   box is flagged `estimated` and a verdict built on it says so.
//! - **Zones** are their authored outline, not the filled copper.
//!
//! Anything that cannot be read is counted in [`BoardGeometry::skipped`], never
//! zero-filled.

use std::collections::BTreeMap;

use crate::board::{
    coordinate_pair, exact_coordinate_pair, exact_outline_arc, footprint_locked,
    footprint_reference, footprint_root_at, footprint_side, footprints, fp_shape_bbox, merge_bbox,
    point, resolve_net, top_level_net_table, CourtyardSource, Side, ZoneOutlineElement,
};
use crate::geometry::transform_pad;
use crate::outline::{Outline, OutlineStatus};
use crate::parser::SexpNode;
use crate::planar::{
    arc_points, bbox_of, dist, flatten_bezier, point_in_ring, segment_distance, tessellate_arc,
    tessellate_circle, Pt,
};

/// Tessellation tolerance for every curved item, in millimetres.
pub const TOLERANCE_MM: f64 = 0.005;

// ─── Shapes ──────────────────────────────────────────────────────────────────

/// A planar shape: a polygon (filled or just its edge), a polyline with a
/// half-width (a track), or a disc (a via, a point with a radius).
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    /// Vertices. One vertex is a point or disc centre.
    pub pts: Vec<Pt>,
    /// Whether the last vertex joins the first.
    pub closed: bool,
    /// Whether the interior counts as part of the shape (a pad, a courtyard).
    pub filled: bool,
    /// Half-width of the stroke, or the disc radius, in mm.
    pub radius: f64,
}

impl Shape {
    /// A filled polygon.
    pub fn polygon(pts: Vec<Pt>) -> Self {
        Shape {
            pts,
            closed: true,
            filled: true,
            radius: 0.0,
        }
    }

    /// A disc (or, at radius 0, a point).
    pub fn disc(center: Pt, radius: f64) -> Self {
        Shape {
            pts: vec![center],
            closed: false,
            filled: false,
            radius,
        }
    }

    /// A stroked open polyline.
    pub fn path(pts: Vec<Pt>, half_width: f64) -> Self {
        Shape {
            pts,
            closed: false,
            filled: false,
            radius: half_width,
        }
    }

    /// A closed polyline drawn as an edge only.
    pub fn ring(pts: Vec<Pt>, half_width: f64) -> Self {
        Shape {
            pts,
            closed: true,
            filled: false,
            radius: half_width,
        }
    }

    /// `(min_x, min_y, max_x, max_y)`, stroke included.
    pub fn bbox(&self) -> Option<(f64, f64, f64, f64)> {
        let (x0, y0, x1, y1) = bbox_of(&self.pts)?;
        Some((
            x0 - self.radius,
            y0 - self.radius,
            x1 + self.radius,
            y1 + self.radius,
        ))
    }

    fn segments(&self) -> Vec<(Pt, Pt)> {
        match self.pts.len() {
            0 => Vec::new(),
            1 => vec![(self.pts[0], self.pts[0])],
            n => {
                let mut segs: Vec<(Pt, Pt)> = self.pts.windows(2).map(|w| (w[0], w[1])).collect();
                if self.closed && n > 2 {
                    segs.push((self.pts[n - 1], self.pts[0]));
                }
                segs
            }
        }
    }

    /// Shortest distance to `other`, 0 when they touch or overlap.
    pub fn distance(&self, other: &Shape) -> f64 {
        if self.filled
            && other
                .pts
                .first()
                .is_some_and(|&p| point_in_ring(p, &self.pts))
        {
            return 0.0;
        }
        if other.filled
            && self
                .pts
                .first()
                .is_some_and(|&p| point_in_ring(p, &other.pts))
        {
            return 0.0;
        }
        let (a, b) = (self.segments(), other.segments());
        let mut best = f64::INFINITY;
        for &(p, q) in &a {
            for &(r, s) in &b {
                best = best.min(segment_distance(p, q, r, s));
                if best == 0.0 {
                    break;
                }
            }
        }
        (best - self.radius - other.radius).max(0.0)
    }

    /// Whether the two shapes touch or overlap.
    pub fn intersects(&self, other: &Shape) -> bool {
        match (self.bbox(), other.bbox()) {
            (Some(a), Some(b)) if a.0 <= b.2 && b.0 <= a.2 && a.1 <= b.3 && b.1 <= a.3 => {
                self.distance(other) <= 1e-9
            }
            _ => false,
        }
    }
}

// ─── Items ───────────────────────────────────────────────────────────────────

/// What kind of board item an [`Entry`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Footprint,
    Pad,
    Track,
    Via,
    Zone,
    Graphic,
    Outline,
}

impl Kind {
    /// Every kind, in report order.
    pub const ALL: [Kind; 7] = [
        Kind::Footprint,
        Kind::Pad,
        Kind::Track,
        Kind::Via,
        Kind::Zone,
        Kind::Graphic,
        Kind::Outline,
    ];

    /// The name a request and a response use for this kind.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Footprint => "footprint",
            Kind::Pad => "pad",
            Kind::Track => "track",
            Kind::Via => "via",
            Kind::Zone => "zone",
            Kind::Graphic => "graphic",
            Kind::Outline => "outline",
        }
    }

    /// The kind named `key`, if any.
    pub fn parse(key: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.key() == key)
    }
}

/// One board item, flattened for queries.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub kind: Kind,
    pub shape: Shape,
    pub uuid: Option<String>,
    /// The owning footprint's reference (footprints and pads).
    pub reference: Option<String>,
    /// Pad number, graphic type, zone name.
    pub name: Option<String>,
    pub net: Option<String>,
    pub layers: Vec<String>,
    /// What the shape is: `courtyard`, `roundrect`, `segment`, `outer`, …
    pub form: String,
    /// Anchor position in board coordinates.
    pub at: Pt,
    /// Named dimensions in mm (`width`, `size`, `drill`).
    pub props: Vec<(&'static str, f64)>,
    /// Caveats the reader must see.
    pub notes: Vec<&'static str>,
}

/// One pad in board coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct PadGeom {
    pub number: String,
    pub form: String,
    pub shape: Shape,
    pub layers: Vec<String>,
    pub net: Option<String>,
    pub uuid: Option<String>,
    pub at: Pt,
    pub pin_function: Option<String>,
    pub through_hole: bool,
    /// The shape is a box standing in for a custom-pad geometry.
    pub approximate: bool,
}

impl PadGeom {
    /// Whether this pad has copper on `layer` (`*.Cu` is every copper layer).
    pub fn on_layer(&self, layer: &str) -> bool {
        self.layers
            .iter()
            .any(|l| l == layer || (l == "*.Cu" && layer.ends_with(".Cu")))
    }

    /// Whether a straight track from `a` to `b`, `width` wide, on `layer`
    /// touches this pad's copper.
    pub fn touches_track(&self, layer: &str, a: Pt, b: Pt, width: f64) -> bool {
        self.on_layer(layer) && self.shape.intersects(&Shape::path(vec![a, b], width / 2.0))
    }
}

/// A piece of footprint field text, as an estimated box.
#[derive(Debug, Clone, PartialEq)]
pub struct TextGeom {
    /// `reference` or `value`.
    pub field: &'static str,
    pub text: String,
    pub layer: String,
    pub shape: Shape,
}

/// One footprint with everything the geometry checks need.
#[derive(Debug, Clone, PartialEq)]
pub struct FootprintGeom {
    pub reference: Option<String>,
    pub value: Option<String>,
    pub uuid: Option<String>,
    pub at: Pt,
    pub rotation_deg: f64,
    pub side: Side,
    pub locked: bool,
    /// The oriented courtyard box; `None` when the footprint draws none.
    pub courtyard: Option<Shape>,
    pub pads: Vec<PadGeom>,
    /// Visible Reference and Value text.
    pub texts: Vec<TextGeom>,
}

impl FootprintGeom {
    /// The shape that stands for the footprint's body: its courtyard, else the
    /// box around its pads, else its anchor point; and what that is.
    pub fn body(&self) -> (Shape, CourtyardSource) {
        if let Some(c) = &self.courtyard {
            return (c.clone(), CourtyardSource::Courtyard);
        }
        let all: Vec<Pt> = self
            .pads
            .iter()
            .flat_map(|p| p.shape.pts.iter().copied())
            .collect();
        match bbox_of(&all) {
            Some((x0, y0, x1, y1)) => (
                Shape::polygon(vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]),
                CourtyardSource::PadsFallback,
            ),
            None => (Shape::disc(self.at, 0.0), CourtyardSource::AnchorOnly),
        }
    }
}

/// The geometric model of a board.
#[derive(Debug, Clone)]
pub struct BoardGeometry {
    pub outline: Outline,
    pub footprints: Vec<FootprintGeom>,
    /// Tracks, vias, zones and graphics, already flattened.
    pub others: Vec<Entry>,
    /// Items too malformed to represent, by kind. Counted, never guessed.
    pub skipped: BTreeMap<&'static str, usize>,
}

impl BoardGeometry {
    /// Build the model from a parsed `(kicad_pcb …)`.
    pub fn from_tree(tree: &SexpNode) -> Self {
        let table = top_level_net_table(tree);
        let mut skipped: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut bump = |what: &'static str| *skipped.entry(what).or_insert(0) += 1;

        let mut fps = Vec::new();
        for fp in footprints(tree) {
            match read_footprint(fp, &table, &mut bump) {
                Some(f) => fps.push(f),
                None => bump("footprints"),
            }
        }

        let mut others = Vec::new();
        for child in tree.children().unwrap_or(&[]) {
            let Some(head) = child.head() else { continue };
            let parsed = match head {
                "segment" => Some(read_track(child, &table, false)),
                "arc" => Some(read_track(child, &table, true)),
                "via" => Some(read_via(child, &table)),
                "zone" => Some(read_zone(child, &table)),
                "gr_line" | "gr_rect" | "gr_circle" | "gr_arc" | "gr_poly" | "gr_curve"
                | "gr_text" => {
                    if child.find_str("layer") == Some("Edge.Cuts") {
                        continue; // the outline, reported as such
                    }
                    Some(read_graphic(child, head))
                }
                _ => None,
            };
            match parsed {
                Some(Some(entry)) => others.push(entry),
                Some(None) => bump(match head {
                    "segment" | "arc" => "tracks",
                    "via" => "vias",
                    "zone" => "zones",
                    _ => "graphics",
                }),
                None => {}
            }
        }

        BoardGeometry {
            outline: Outline::from_tree(tree),
            footprints: fps,
            others,
            skipped,
        }
    }

    /// Pad `number` of footprint `reference`, if both exist.
    pub fn pad(&self, reference: &str, number: &str) -> Option<&PadGeom> {
        self.footprint(reference)?
            .pads
            .iter()
            .find(|p| p.number == number)
    }

    /// Footprint `reference`, if the board has one.
    pub fn footprint(&self, reference: &str) -> Option<&FootprintGeom> {
        self.footprints
            .iter()
            .find(|f| f.reference.as_deref() == Some(reference))
    }

    /// Every item as a flat list: footprints, pads, then tracks, vias, zones
    /// and graphics, then one entry per outline ring.
    pub fn entries(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        for f in &self.footprints {
            let (body, source) = f.body();
            out.push(Entry {
                kind: Kind::Footprint,
                shape: body,
                uuid: f.uuid.clone(),
                reference: f.reference.clone(),
                name: f.value.clone(),
                net: None,
                layers: vec![match f.side {
                    Side::Front => "F.Cu".to_string(),
                    Side::Back => "B.Cu".to_string(),
                }],
                form: match source {
                    CourtyardSource::Courtyard => "courtyard",
                    CourtyardSource::PadsFallback => "pads_box",
                    CourtyardSource::AnchorOnly => "anchor",
                }
                .to_string(),
                at: f.at,
                props: vec![("rotation_deg", f.rotation_deg)],
                notes: if f.locked { vec!["locked"] } else { Vec::new() },
            });
            for p in &f.pads {
                out.push(Entry {
                    kind: Kind::Pad,
                    shape: p.shape.clone(),
                    uuid: p.uuid.clone(),
                    reference: f.reference.clone(),
                    name: Some(p.number.clone()),
                    net: p.net.clone(),
                    layers: p.layers.clone(),
                    form: p.form.clone(),
                    at: p.at,
                    props: Vec::new(),
                    notes: if p.approximate {
                        vec!["approximate_shape"]
                    } else {
                        Vec::new()
                    },
                });
            }
        }
        out.extend(self.others.iter().cloned());
        for (i, ring) in self.outline.loops.iter().enumerate() {
            let hole = self
                .outline
                .loops
                .iter()
                .enumerate()
                .filter(|(j, o)| *j != i && point_in_ring(ring[0], o))
                .count()
                % 2
                == 1;
            out.push(Entry {
                kind: Kind::Outline,
                shape: Shape::ring(ring.clone(), 0.0),
                uuid: None,
                reference: None,
                name: None,
                net: None,
                layers: vec!["Edge.Cuts".to_string()],
                form: if hole { "cutout" } else { "outer" }.to_string(),
                at: ring[0],
                props: Vec::new(),
                notes: if self.outline.curved {
                    vec!["curves_tessellated"]
                } else {
                    Vec::new()
                },
            });
        }
        out
    }
}

// ─── Reading ─────────────────────────────────────────────────────────────────

fn layer_list(node: &SexpNode, tag: &str) -> Vec<String> {
    node.find(tag)
        .and_then(SexpNode::children)
        .map(|c| {
            c.iter()
                .skip(1)
                .filter_map(SexpNode::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn read_footprint(
    fp: &SexpNode,
    table: &std::collections::HashMap<String, String>,
    bump: &mut impl FnMut(&'static str),
) -> Option<FootprintGeom> {
    let (fx, fy, rot) = footprint_root_at(fp)?;
    let side = footprint_side(fp)?;

    // Courtyard: the oriented box of the drawn courtyard shapes.
    let mut local: Option<(f64, f64, f64, f64)> = None;
    for child in fp.children().unwrap_or(&[]) {
        let Some(head) = child.head() else { continue };
        if !matches!(
            head,
            "fp_line" | "fp_rect" | "fp_circle" | "fp_arc" | "fp_poly"
        ) {
            continue;
        }
        if !matches!(child.find_str("layer"), Some("F.CrtYd" | "B.CrtYd")) {
            continue;
        }
        local = Some(merge_bbox(local, fp_shape_bbox(child, head)?));
    }
    let courtyard = local.map(|(x0, y0, x1, y1)| {
        Shape::polygon(
            [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                .iter()
                .map(|&(x, y)| transform_pad(x, y, fx, fy, rot))
                .collect(),
        )
    });

    let mut pads = Vec::new();
    for pad in fp.find_all("pad") {
        match read_pad(pad, fx, fy, rot, table) {
            Some(p) => pads.push(p),
            None => bump("pads"),
        }
    }

    let mut texts = Vec::new();
    for prop in fp.find_all("property") {
        let field = match prop.get(1).and_then(SexpNode::as_str) {
            Some("Reference") => "reference",
            Some("Value") => "value",
            _ => continue,
        };
        if let Some(t) = read_text(prop, field, prop.get(2)?.as_str()?, fx, fy, rot) {
            texts.push(t);
        }
    }
    if texts.is_empty() {
        for t in fp.find_all("fp_text") {
            let field = match t.get(1).and_then(SexpNode::as_str) {
                Some("reference") => "reference",
                Some("value") => "value",
                _ => continue,
            };
            if let Some(t) = read_text(t, field, t.get(2)?.as_str()?, fx, fy, rot) {
                texts.push(t);
            }
        }
    }

    let value = fp
        .find_all("property")
        .into_iter()
        .find(|p| p.get(1).and_then(SexpNode::as_str) == Some("Value"))
        .and_then(|p| p.get(2)?.as_str().map(str::to_string));

    Some(FootprintGeom {
        reference: footprint_reference(fp),
        value,
        uuid: fp.find_str("uuid").map(str::to_string),
        at: (fx, fy),
        rotation_deg: rot,
        side,
        locked: footprint_locked(fp),
        courtyard,
        pads,
        texts,
    })
}

fn finite(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

fn read_pad(
    pad: &SexpNode,
    fx: f64,
    fy: f64,
    rot: f64,
    table: &std::collections::HashMap<String, String>,
) -> Option<PadGeom> {
    let number = pad.get(1)?.as_str()?.to_string();
    let kind = pad.get(2)?.as_str()?;
    let form = pad.get(3)?.as_str()?.to_string();
    let at = pad.find("at")?;
    let (px, py) = (finite(at.get_f64(1)?)?, finite(at.get_f64(2)?)?);
    let angle = match at.get(3) {
        None => 0.0,
        Some(v) => finite(v.as_str()?.parse().ok()?)?,
    };
    let (cx, cy) = transform_pad(px, py, fx, fy, rot);
    let size = pad.find("size")?;
    let (w, h) = (finite(size.get_f64(1)?)?, finite(size.get_f64(2)?)?);
    let (ox, oy) = match pad.find("drill").and_then(|d| d.find("offset")) {
        Some(o) => (finite(o.get_f64(1)?)?, finite(o.get_f64(2)?)?),
        None => (0.0, 0.0),
    };
    // A point in the pad's own frame (shape offset included) on the board.
    let place = |x: f64, y: f64| transform_pad(x + ox, y + oy, cx, cy, angle);

    let mut approximate = false;
    let ring: Vec<Pt> = match form.as_str() {
        "circle" => tessellate_circle((0.0, 0.0), w / 2.0, TOLERANCE_MM)
            .into_iter()
            .map(|(x, y)| place(x, y))
            .collect(),
        "rect" => rect_ring(w, h)
            .into_iter()
            .map(|(x, y)| place(x, y))
            .collect(),
        "oval" => capsule_ring(w, h)
            .into_iter()
            .map(|(x, y)| place(x, y))
            .collect(),
        "roundrect" => {
            let ratio = match pad.find("roundrect_rratio") {
                None => 0.25,
                Some(n) => finite(n.get_f64(1)?)?,
            };
            roundrect_ring(w, h, ratio * w.min(h))
                .into_iter()
                .map(|(x, y)| place(x, y))
                .collect()
        }
        "trapezoid" => {
            let (dx, dy) = match pad.find("rect_delta") {
                Some(d) => (finite(d.get_f64(1)?)?, finite(d.get_f64(2)?)?),
                None => (0.0, 0.0),
            };
            [
                (-w / 2.0 - dy / 2.0, -h / 2.0 + dx / 2.0),
                (w / 2.0 + dy / 2.0, -h / 2.0 - dx / 2.0),
                (w / 2.0 - dy / 2.0, h / 2.0 + dx / 2.0),
                (-w / 2.0 + dy / 2.0, h / 2.0 - dx / 2.0),
            ]
            .iter()
            .map(|&(x, y)| place(x, y))
            .collect()
        }
        _ => {
            // Custom (and anything newer): the box of what the pad draws.
            let (x0, y0, x1, y1) = crate::bounds::pad_bbox(pad, fx, fy, rot)?;
            approximate = true;
            vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
        }
    };
    if ring.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return None;
    }

    Some(PadGeom {
        number,
        form,
        shape: Shape::polygon(ring),
        layers: layer_list(pad, "layers"),
        net: resolve_net(pad, table),
        uuid: pad.find_str("uuid").map(str::to_string),
        at: (cx, cy),
        pin_function: pad.find_str("pinfunction").map(str::to_string),
        through_hole: matches!(kind, "thru_hole" | "np_thru_hole"),
        approximate,
    })
}

fn rect_ring(w: f64, h: f64) -> Vec<Pt> {
    vec![
        (-w / 2.0, -h / 2.0),
        (w / 2.0, -h / 2.0),
        (w / 2.0, h / 2.0),
        (-w / 2.0, h / 2.0),
    ]
}

/// A stadium: the pad's long axis ends in half-circles of radius `min(w,h)/2`.
fn capsule_ring(w: f64, h: f64) -> Vec<Pt> {
    use std::f64::consts::{FRAC_PI_2, PI};
    if (w - h).abs() < 1e-9 {
        return tessellate_circle((0.0, 0.0), w / 2.0, TOLERANCE_MM);
    }
    let r = w.min(h) / 2.0;
    if w > h {
        let c = w / 2.0 - r;
        let mut ring = arc_points((c, 0.0), r, -FRAC_PI_2, FRAC_PI_2, TOLERANCE_MM);
        ring.extend(arc_points(
            (-c, 0.0),
            r,
            FRAC_PI_2,
            PI + FRAC_PI_2,
            TOLERANCE_MM,
        ));
        ring
    } else {
        let c = h / 2.0 - r;
        let mut ring = arc_points((0.0, c), r, 0.0, PI, TOLERANCE_MM);
        ring.extend(arc_points((0.0, -c), r, PI, 2.0 * PI, TOLERANCE_MM));
        ring
    }
}

fn roundrect_ring(w: f64, h: f64, r: f64) -> Vec<Pt> {
    use std::f64::consts::{FRAC_PI_2, PI};
    if r <= 1e-9 {
        return rect_ring(w, h);
    }
    let (ix, iy) = (w / 2.0 - r, h / 2.0 - r);
    let mut ring = Vec::new();
    for (c, a0) in [
        ((ix, iy), 0.0),
        ((-ix, iy), FRAC_PI_2),
        ((-ix, -iy), PI),
        ((ix, -iy), PI + FRAC_PI_2),
    ] {
        ring.extend(arc_points(c, r, a0, a0 + FRAC_PI_2, TOLERANCE_MM));
    }
    ring
}

/// Estimated average advance of a KiCad stroke-font glyph, as a fraction of the
/// text width setting.
const GLYPH_ADVANCE: f64 = 0.65;

fn read_text(
    node: &SexpNode,
    field: &'static str,
    text: &str,
    fx: f64,
    fy: f64,
    rot: f64,
) -> Option<TextGeom> {
    let hidden = node
        .children()?
        .iter()
        .any(|c| matches!(c, SexpNode::Atom(a) if a == "hide"))
        || node
            .find("hide")
            .and_then(|h| h.get(1))
            .and_then(SexpNode::as_str)
            == Some("yes")
        || node
            .find("effects")
            .map(|e| {
                e.children()
                    .unwrap_or(&[])
                    .iter()
                    .any(|c| matches!(c, SexpNode::Atom(a) if a == "hide"))
                    || e.find("hide")
                        .and_then(|h| h.get(1))
                        .and_then(SexpNode::as_str)
                        == Some("yes")
            })
            .unwrap_or(false);
    if hidden || text.is_empty() {
        return None;
    }
    let at = node.find("at")?;
    let (lx, ly) = (finite(at.get_f64(1)?)?, finite(at.get_f64(2)?)?);
    // The angle is absolute; the position is footprint-local.
    let angle = at.get_f64(3).unwrap_or(0.0);
    let layer = node.find_str("layer")?.to_string();
    let (px, py) = transform_pad(lx, ly, fx, fy, rot);

    let effects = node.find("effects");
    let font = effects.and_then(|e| e.find("font"));
    // `(size HEIGHT WIDTH)`.
    let (size_h, size_w) = font
        .and_then(|f| f.find("size"))
        .and_then(|s| Some((s.get_f64(1)?, s.get_f64(2)?)))
        .unwrap_or((1.0, 1.0));
    let thickness = font.and_then(|f| f.find_f64("thickness")).unwrap_or(0.15);
    let justify: Vec<&str> = effects
        .and_then(|e| e.find("justify"))
        .and_then(SexpNode::children)
        .map(|c| c.iter().skip(1).filter_map(SexpNode::as_str).collect())
        .unwrap_or_default();

    let lines: Vec<&str> = text.split('\n').collect();
    let cols = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f64;
    let width = cols * GLYPH_ADVANCE * size_w + thickness;
    let height = lines.len() as f64 * size_h * 1.5 - size_h * 0.5 + thickness;

    let mirror = justify.contains(&"mirror");
    let (mut x0, mut x1) = if justify.contains(&"left") {
        (0.0, width)
    } else if justify.contains(&"right") {
        (-width, 0.0)
    } else {
        (-width / 2.0, width / 2.0)
    };
    if mirror {
        (x0, x1) = (-x1, -x0);
    }
    let (y0, y1) = if justify.contains(&"top") {
        (0.0, height)
    } else if justify.contains(&"bottom") {
        (-height, 0.0)
    } else {
        (-height / 2.0, height / 2.0)
    };
    let ring = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
        .iter()
        .map(|&(x, y)| transform_pad(x, y, px, py, angle))
        .collect();
    Some(TextGeom {
        field,
        text: text.to_string(),
        layer,
        shape: Shape::polygon(ring),
    })
}

fn read_track(
    node: &SexpNode,
    table: &std::collections::HashMap<String, String>,
    curved: bool,
) -> Option<Entry> {
    let width = node
        .find_f64("width")
        .filter(|w| w.is_finite() && *w > 0.0)?;
    let layer = node.find_str("layer")?.to_string();
    let (start, end) = (point(node, "start")?, point(node, "end")?);
    let pts = if curved {
        tessellate_arc(start, point(node, "mid")?, end, TOLERANCE_MM)
    } else {
        vec![start, end]
    };
    Some(Entry {
        kind: Kind::Track,
        shape: Shape::path(pts, width / 2.0),
        uuid: node.find_str("uuid").map(str::to_string),
        reference: None,
        name: None,
        net: resolve_net(node, table),
        layers: vec![layer],
        form: if curved { "arc" } else { "segment" }.to_string(),
        at: start,
        props: vec![("width", width)],
        notes: Vec::new(),
    })
}

fn read_via(node: &SexpNode, table: &std::collections::HashMap<String, String>) -> Option<Entry> {
    let at = point(node, "at")?;
    let size = node
        .find_f64("size")
        .filter(|v| v.is_finite() && *v > 0.0)?;
    let drill = node
        .find_f64("drill")
        .filter(|v| v.is_finite() && *v > 0.0)?;
    let layers = layer_list(node, "layers");
    if layers.is_empty() {
        return None;
    }
    Some(Entry {
        kind: Kind::Via,
        shape: Shape::disc(at, size / 2.0),
        uuid: node.find_str("uuid").map(str::to_string),
        reference: None,
        name: None,
        net: resolve_net(node, table),
        layers,
        form: "via".to_string(),
        at,
        props: vec![("size", size), ("drill", drill)],
        notes: Vec::new(),
    })
}

fn read_zone(node: &SexpNode, table: &std::collections::HashMap<String, String>) -> Option<Entry> {
    let layers = match node.find("layers") {
        Some(_) => layer_list(node, "layers"),
        None => vec![node.find_str("layer")?.to_string()],
    };
    if layers.is_empty() {
        return None;
    }
    let pts = node.find("polygon")?.find("pts")?;
    let mut ring: Vec<Pt> = Vec::new();
    for item in pts.children()?.iter().skip(1) {
        match item.head()? {
            "xy" => ring.push(exact_coordinate_pair(item)?),
            "arc" => {
                let ZoneOutlineElement::Arc { start, mid, end } = exact_outline_arc(item)? else {
                    return None;
                };
                let arc = tessellate_arc(start, mid, end, TOLERANCE_MM);
                for p in arc {
                    if ring.last() != Some(&p) {
                        ring.push(p);
                    }
                }
            }
            _ => return None,
        }
    }
    if ring.len() < 3 {
        return None;
    }
    let first = ring[0];
    Some(Entry {
        kind: Kind::Zone,
        shape: Shape::polygon(ring),
        uuid: node.find_str("uuid").map(str::to_string),
        reference: None,
        name: node
            .find_str("name")
            .map(str::to_string)
            .filter(|n| !n.is_empty()),
        net: resolve_net(node, table),
        layers,
        form: if node.find("keepout").is_some() {
            "keepout"
        } else {
            "zone_outline"
        }
        .to_string(),
        at: first,
        props: Vec::new(),
        notes: vec!["authored_outline_not_fill"],
    })
}

fn read_graphic(node: &SexpNode, head: &str) -> Option<Entry> {
    let layer = node.find_str("layer")?.to_string();
    let stroke = node
        .find("stroke")
        .and_then(|s| s.find_f64("width"))
        .or_else(|| node.find_f64("width"))
        .unwrap_or(0.0);
    let half = stroke / 2.0;
    let filled = matches!(
        node.find("fill")
            .and_then(|f| f.get(1))
            .and_then(SexpNode::as_str),
        Some("yes" | "solid")
    ) || matches!(
        node.find("fill").and_then(|f| f.find_str("type")),
        Some("solid")
    );
    let (shape, at, name): (Shape, Pt, Option<String>) = match head {
        "gr_line" => {
            let (s, e) = (point(node, "start")?, point(node, "end")?);
            (Shape::path(vec![s, e], half), s, None)
        }
        "gr_arc" => {
            let (s, m, e) = (
                point(node, "start")?,
                point(node, "mid")?,
                point(node, "end")?,
            );
            (
                Shape::path(tessellate_arc(s, m, e, TOLERANCE_MM), half),
                s,
                None,
            )
        }
        "gr_circle" => {
            let (c, e) = (point(node, "center")?, point(node, "end")?);
            let ring = tessellate_circle(c, dist(c, e), TOLERANCE_MM);
            let mut shape = Shape::ring(ring, half);
            shape.filled = filled;
            (shape, c, None)
        }
        "gr_rect" => {
            let (s, e) = (point(node, "start")?, point(node, "end")?);
            let ring = vec![s, (e.0, s.1), e, (s.0, e.1)];
            let mut shape = Shape::ring(ring, half);
            shape.filled = filled;
            (shape, s, None)
        }
        "gr_poly" => {
            let ring: Vec<Pt> = node
                .find("pts")?
                .find_all("xy")
                .into_iter()
                .map(coordinate_pair)
                .collect::<Option<_>>()?;
            if ring.len() < 3 {
                return None;
            }
            let first = ring[0];
            let mut shape = Shape::ring(ring, half);
            shape.filled = filled;
            (shape, first, None)
        }
        "gr_curve" => {
            let q: Vec<Pt> = node
                .find("pts")?
                .find_all("xy")
                .into_iter()
                .map(coordinate_pair)
                .collect::<Option<_>>()?;
            if q.len() != 4 {
                return None;
            }
            (
                Shape::path(flatten_bezier(q[0], q[1], q[2], q[3], TOLERANCE_MM), half),
                q[0],
                None,
            )
        }
        "gr_text" => {
            let at = point(node, "at")?;
            (
                Shape::disc(at, 0.0),
                at,
                node.get(1)?.as_str().map(str::to_string),
            )
        }
        _ => return None,
    };
    Some(Entry {
        kind: Kind::Graphic,
        shape,
        uuid: node.find_str("uuid").map(str::to_string),
        reference: None,
        name,
        net: None,
        layers: vec![layer],
        form: head.trim_start_matches("gr_").to_string(),
        at,
        props: if stroke > 0.0 {
            vec![("width", stroke)]
        } else {
            Vec::new()
        },
        notes: Vec::new(),
    })
}

// ─── Region queries ──────────────────────────────────────────────────────────

/// A region query over a [`BoardGeometry`].
#[derive(Debug, Clone)]
pub struct RegionQuery {
    /// What counts as "in the area": an item is a hit when its shape touches or
    /// overlaps this.
    pub region: Shape,
    /// What distances are measured from.
    pub anchor: Shape,
    /// Kinds to return; empty means every kind but the outline.
    pub kinds: Vec<Kind>,
    /// Keep only items on this layer (`*.Cu` pads count for any copper layer).
    pub layer: Option<String>,
    /// Keep only items on this net.
    pub net: Option<String>,
    /// Leave out the footprint with this reference and its pads.
    pub exclude_reference: Option<String>,
    pub limit: usize,
}

/// One item in the area.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub entry: Entry,
    /// Edge-to-edge distance from the anchor, 0 when they overlap.
    pub distance_mm: f64,
}

/// The answer to a [`RegionQuery`].
#[derive(Debug, Clone, PartialEq)]
pub struct RegionResult {
    /// The nearest `limit` hits, nearest first.
    pub hits: Vec<Hit>,
    /// How many items matched before the limit.
    pub total: usize,
    pub truncated: bool,
    /// Matches per kind, before the limit.
    pub counts: BTreeMap<&'static str, usize>,
}

fn on_layer(entry: &Entry, layer: &str) -> bool {
    entry.layers.iter().any(|l| {
        l == layer
            || (l == "*.Cu" && layer.ends_with(".Cu"))
            || (l == "F&B.Cu" && matches!(layer, "F.Cu" | "B.Cu"))
    }) || (entry.kind == Kind::Via && layer.ends_with(".Cu") && entry.layers.len() >= 2)
}

impl BoardGeometry {
    /// Items in an area, nearest first, with the true count before `limit`.
    pub fn query_region(&self, q: &RegionQuery) -> RegionResult {
        let wanted = |k: Kind| {
            if q.kinds.is_empty() {
                k != Kind::Outline
            } else {
                q.kinds.contains(&k)
            }
        };
        let mut hits: Vec<Hit> = Vec::new();
        for entry in self.entries() {
            if !wanted(entry.kind) {
                continue;
            }
            if q.exclude_reference.is_some() && entry.reference == q.exclude_reference {
                continue;
            }
            if q.layer.as_deref().is_some_and(|l| !on_layer(&entry, l)) {
                continue;
            }
            if q.net.is_some() && entry.net != q.net {
                continue;
            }
            if !entry.shape.intersects(&q.region) {
                continue;
            }
            let distance_mm = entry.shape.distance(&q.anchor);
            hits.push(Hit { entry, distance_mm });
        }
        hits.sort_by(|a, b| {
            a.distance_mm
                .total_cmp(&b.distance_mm)
                .then(a.entry.kind.cmp(&b.entry.kind))
                .then_with(|| a.entry.reference.cmp(&b.entry.reference))
                .then_with(|| a.entry.name.cmp(&b.entry.name))
                .then_with(|| a.entry.uuid.cmp(&b.entry.uuid))
        });
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        for h in &hits {
            *counts.entry(h.entry.kind.key()).or_insert(0) += 1;
        }
        let total = hits.len();
        hits.truncate(q.limit);
        RegionResult {
            truncated: total > hits.len(),
            hits,
            total,
            counts,
        }
    }
}

// ─── Outline containment ─────────────────────────────────────────────────────

/// References that are allowed to sit on or over the board edge on purpose
/// (edge-mount connectors, card-edge fingers, antennas). A trailing `*`
/// matches a prefix: `J*` is every connector.
#[derive(Debug, Clone, Default)]
pub struct Allowlist(Vec<String>);

impl Allowlist {
    pub fn new<I, S>(refs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Allowlist(
            refs.into_iter()
                .map(Into::into)
                .filter(|s: &String| !s.is_empty())
                .collect(),
        )
    }

    pub fn allows(&self, reference: &str) -> bool {
        self.0
            .iter()
            .any(|pattern| match pattern.strip_suffix('*') {
                Some(prefix) => reference.starts_with(prefix),
                None => pattern == reference,
            })
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// One part of a footprint that sticks out of the outline.
#[derive(Debug, Clone, PartialEq)]
pub struct PartOverhang {
    /// `courtyard`, `pad` or `text`.
    pub part: &'static str,
    /// Pad number, text field name, or `courtyard`.
    pub label: String,
    pub overhang_mm: f64,
    /// Where the deepest point is.
    pub at: Pt,
}

/// A footprint with something outside the board outline.
#[derive(Debug, Clone, PartialEq)]
pub struct FootprintOverhang {
    pub reference: String,
    pub side: Side,
    /// `outside` (nothing of the body is on the board), `straddles` (the body
    /// crosses the edge), or `text_only` (only the silkscreen text crosses).
    pub relation: &'static str,
    /// The deepest overhang of any part, in mm.
    pub overhang_mm: f64,
    pub courtyard: Option<PartOverhang>,
    /// Pads past the edge, deepest first, capped at [`MAX_PADS_LISTED`].
    pub pads: Vec<PartOverhang>,
    /// How many pads are past the edge (may exceed `pads.len()`).
    pub pads_over: usize,
    pub texts: Vec<PartOverhang>,
    /// The part is on the edge-mount allowlist.
    pub allowed: bool,
}

/// Pads listed per footprint before the rest are only counted.
pub const MAX_PADS_LISTED: usize = 8;

/// The outcome of checking every footprint against the true outline.
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineCheck {
    pub status: OutlineStatus,
    /// Whether containment could be judged at all (status `Closed`).
    pub checked: bool,
    pub footprints_checked: usize,
    /// Footprints past the edge that are not allowlisted.
    pub violations: Vec<FootprintOverhang>,
    /// Footprints past the edge that the allowlist excuses.
    pub allowed: Vec<FootprintOverhang>,
}

impl BoardGeometry {
    /// Check each footprint's courtyard, pads and silkscreen field text against
    /// the true outline.
    ///
    /// Courtyards and pads are physical, so a crossing there is a finding on
    /// its own; text crossing the edge is reported separately because its box
    /// is estimated. Nothing is judged unless the outline is closed.
    pub fn outline_check(&self, allow: &Allowlist) -> OutlineCheck {
        let status = self.outline.status();
        let mut out = OutlineCheck {
            status,
            checked: status == OutlineStatus::Closed,
            footprints_checked: 0,
            violations: Vec::new(),
            allowed: Vec::new(),
        };
        if !out.checked {
            return out;
        }
        let threshold = self.outline.containment_threshold_mm();

        for f in &self.footprints {
            out.footprints_checked += 1;
            let part = |kind: &'static str, label: String, shape: &Shape| {
                let o = self.outline.overhang(&shape.pts);
                (o.depth_mm > threshold).then(|| {
                    (
                        PartOverhang {
                            part: kind,
                            label,
                            overhang_mm: o.depth_mm,
                            at: o.at,
                        },
                        o.touches_board,
                    )
                })
            };
            let courtyard = f
                .courtyard
                .as_ref()
                .and_then(|c| part("courtyard", "courtyard".to_string(), c));
            let mut pads: Vec<(PartOverhang, bool)> = f
                .pads
                .iter()
                .filter_map(|p| part("pad", p.number.clone(), &p.shape))
                .collect();
            pads.sort_by(|a, b| b.0.overhang_mm.total_cmp(&a.0.overhang_mm));
            let mut texts: Vec<PartOverhang> = f
                .texts
                .iter()
                .filter(|t| t.layer.ends_with(".SilkS"))
                .filter_map(|t| part("text", t.field.to_string(), &t.shape).map(|(p, _)| p))
                .collect();
            texts.sort_by(|a, b| b.overhang_mm.total_cmp(&a.overhang_mm));

            let body_over = courtyard.is_some() || !pads.is_empty();
            if !body_over && texts.is_empty() {
                continue;
            }
            // The body is the courtyard when one is drawn, else the pads. It is
            // wholly outside when what stands for it has no point on the board.
            let relation = if !body_over {
                "text_only"
            } else if let Some((_, touches)) = &courtyard {
                if *touches {
                    "straddles"
                } else {
                    "outside"
                }
            } else if f.courtyard.is_some() {
                "straddles" // pads cross while the courtyard stays on the board
            } else if pads.len() == f.pads.len() && pads.iter().all(|(_, touches)| !touches) {
                "outside"
            } else {
                "straddles"
            };
            let deepest = courtyard
                .iter()
                .map(|(p, _)| p.overhang_mm)
                .chain(pads.iter().map(|(p, _)| p.overhang_mm))
                .chain(texts.iter().map(|p| p.overhang_mm))
                .fold(0.0, f64::max);
            let reference = f
                .reference
                .clone()
                .unwrap_or_else(|| "(no reference)".to_string());
            let pads_over = pads.len();
            let item = FootprintOverhang {
                allowed: f.reference.as_deref().is_some_and(|r| allow.allows(r)),
                reference,
                side: f.side,
                relation,
                overhang_mm: deepest,
                courtyard: courtyard.map(|(p, _)| p),
                pads: pads
                    .into_iter()
                    .map(|(p, _)| p)
                    .take(MAX_PADS_LISTED)
                    .collect(),
                pads_over,
                texts,
            };
            if item.allowed {
                out.allowed.push(item);
            } else {
                out.violations.push(item);
            }
        }
        out.violations.sort_by(|a, b| {
            b.overhang_mm
                .total_cmp(&a.overhang_mm)
                .then_with(|| a.reference.cmp(&b.reference))
        });
        out.allowed.sort_by(|a, b| a.reference.cmp(&b.reference));
        out
    }
}
