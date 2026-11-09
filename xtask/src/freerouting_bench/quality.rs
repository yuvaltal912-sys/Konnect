//! How good a routing is, not only whether it is complete.
//!
//! Completion is a gate: an unrouted connection or a clearance violation is a
//! defect, and a board with either is not compared on quality. On the boards
//! that pass, a routing is better when its wiring is closer to the shortest it
//! could be, it uses fewer vias and layer changes, it has no acute corners
//! (acid traps) and few right-angle ones, and every segment is at least its net
//! class's width.
//!
//! The DSN gives the pin positions, nets and net classes; the SES gives the
//! wiring. Nothing here needs Java.

use super::parse::{parse_sexp, unit_mm, Sexp};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// What a DSN says about where pins are and how wide each net must be.
#[derive(Debug, Clone, PartialEq)]
pub struct DsnBoard {
    /// `REF-PIN` → position in mm, as the network section names pins.
    pub pins: HashMap<String, (f64, f64)>,
    /// Net name → its pins.
    pub nets: BTreeMap<String, Vec<String>>,
    /// Net name → (class name, minimum width in mm).
    pub net_width: HashMap<String, (String, f64)>,
    /// The structure's default width, for nets in no class.
    pub default_width_mm: Option<f64>,
    pub components: usize,
    pub signal_layers: usize,
    /// Nets with a plane or pour (`(plane NET …)`): part of their connection
    /// is copper area, so their wiring can be shorter than HPWL and is left out
    /// of the ratio.
    pub plane_nets: BTreeSet<String>,
    /// Nets with wiring already in the DSN (`(wiring …)`, usually fixed):
    /// Freerouting keeps it but does not write it to the SES, so these nets
    /// are left out of the ratio too.
    pub prewired_nets: BTreeSet<String>,
}

fn number(sexp: Option<&Sexp>) -> Option<f64> {
    sexp?.atom()?.parse().ok()
}

fn direct<'a>(sexp: &'a Sexp, head: &'a str) -> impl Iterator<Item = &'a Sexp> + 'a {
    sexp.items()
        .iter()
        .filter(move |item| item.head() == Some(head))
}

/// The width in a `(rule … (width W) …)` directly inside `sexp`.
fn rule_width(sexp: &Sexp) -> Option<f64> {
    direct(sexp, "rule")
        .flat_map(|rule| direct(rule, "width"))
        .find_map(|width| number(width.items().get(1)))
}

impl DsnBoard {
    pub fn parse(text: &str) -> Result<Self, String> {
        let root = parse_sexp(text).ok_or("the DSN is not one balanced S-expression")?;
        if root.head() != Some("pcb") {
            return Err("the DSN does not start with (pcb".to_string());
        }
        // Coordinates are in the pcb's (unit …), else the resolution's unit.
        let unit = direct(&root, "unit")
            .find_map(|u| u.items().get(1).and_then(Sexp::atom))
            .or_else(|| {
                direct(&root, "resolution").find_map(|r| r.items().get(1).and_then(Sexp::atom))
            })
            .unwrap_or("mil");
        let scale = unit_mm(unit).ok_or_else(|| format!("unknown DSN unit {unit:?}"))?;

        let structure = direct(&root, "structure").next();
        let default_width_mm = structure.and_then(rule_width).map(|w| w * scale);
        let signal_layers = structure.map_or(0, |s| {
            direct(s, "layer")
                .filter(|layer| {
                    direct(layer, "type")
                        .any(|t| t.items().get(1).and_then(Sexp::atom) == Some("signal"))
                })
                .count()
        });
        let plane_nets = structure
            .map(|s| {
                direct(s, "plane")
                    .filter_map(|p| p.items().get(1).and_then(Sexp::atom))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        // image name → pin id → offset in the image.
        let mut images: HashMap<&str, Vec<(&str, f64, f64)>> = HashMap::new();
        if let Some(library) = root.first("library") {
            for image in direct(library, "image") {
                let Some(name) = image.items().get(1).and_then(Sexp::atom) else {
                    continue;
                };
                let pins = direct(image, "pin")
                    .filter_map(|pin| {
                        // (pin padstack [(rotate r)] id x y): the rotation turns
                        // the pad's shape, not its position.
                        let atoms: Vec<&str> =
                            pin.items().iter().skip(1).filter_map(Sexp::atom).collect();
                        match atoms.as_slice() {
                            [_padstack, id, x, y, ..] => {
                                Some((*id, x.parse().ok()?, y.parse().ok()?))
                            }
                            _ => None,
                        }
                    })
                    .collect();
                images.insert(name, pins);
            }
        }

        let mut pins = HashMap::new();
        let mut components = 0;
        if let Some(placement) = root.first("placement") {
            for component in direct(placement, "component") {
                let image = component.items().get(1).and_then(Sexp::atom).unwrap_or("");
                let offsets = images.get(image).cloned().unwrap_or_default();
                for place in direct(component, "place") {
                    let items = place.items();
                    let (Some(reference), Some(x), Some(y)) = (
                        items.get(1).and_then(Sexp::atom),
                        number(items.get(2)),
                        number(items.get(3)),
                    ) else {
                        continue;
                    };
                    components += 1;
                    let back = items.get(4).and_then(Sexp::atom) == Some("back");
                    let rotation = number(items.get(5)).unwrap_or(0.0).to_radians();
                    for (id, px, py) in &offsets {
                        let (x0, y0) = place_pin(*px, *py, back, rotation);
                        pins.insert(
                            format!("{reference}-{id}"),
                            ((x + x0) * scale, (y + y0) * scale),
                        );
                    }
                }
            }
        }

        let mut nets = BTreeMap::new();
        let mut net_width = HashMap::new();
        if let Some(network) = root.first("network") {
            for net in direct(network, "net") {
                let Some(name) = net.items().get(1).and_then(Sexp::atom) else {
                    continue;
                };
                let members = direct(net, "pins")
                    .flat_map(|p| p.items().iter().skip(1).filter_map(Sexp::atom))
                    .map(str::to_string)
                    .collect();
                nets.insert(name.to_string(), members);
            }
            for class in direct(network, "class") {
                let Some(class_name) = class.items().get(1).and_then(Sexp::atom) else {
                    continue;
                };
                let Some(width) = rule_width(class).or(default_width_mm.map(|w| w / scale)) else {
                    continue;
                };
                for net in class.items().iter().skip(2).filter_map(Sexp::atom) {
                    net_width.insert(net.to_string(), (class_name.to_string(), width * scale));
                }
            }
        }
        let mut prewired_nets = BTreeSet::new();
        if let Some(wiring) = direct(&root, "wiring").next() {
            for item in wiring.items() {
                if matches!(item.head(), Some("wire") | Some("via")) {
                    if let Some(net) =
                        direct(item, "net").find_map(|n| n.items().get(1).and_then(Sexp::atom))
                    {
                        prewired_nets.insert(net.to_string());
                    }
                }
            }
        }
        Ok(DsnBoard {
            pins,
            nets,
            net_width,
            default_width_mm,
            components,
            signal_layers,
            plane_nets,
            prewired_nets,
        })
    }

    fn net_points(&self, net: &str) -> Option<Vec<(f64, f64)>> {
        let points: Vec<(f64, f64)> = self
            .nets
            .get(net)?
            .iter()
            .filter_map(|pin| self.pins.get(pin).copied())
            .collect();
        (points.len() >= 2).then_some(points)
    }

    /// Half-perimeter of the box around a net's pins. A lower bound only for
    /// routing in 90° steps: Freerouting also routes at 45° and other angles,
    /// so a net's wiring can be shorter than this. Reported for reference.
    /// `None` when fewer than two of its pins are placed.
    pub fn hpwl_mm(&self, net: &str) -> Option<f64> {
        let points = self.net_points(net)?;
        let (mut lo_x, mut lo_y, mut hi_x, mut hi_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (x, y) in points {
            lo_x = lo_x.min(x);
            lo_y = lo_y.min(y);
            hi_x = hi_x.max(x);
            hi_y = hi_y.max(y);
        }
        Some((hi_x - lo_x) + (hi_y - lo_y))
    }

    /// Half the perimeter of the convex hull of a net's pins: no wiring of the
    /// net at any angle is shorter, since a walk around any tree that connects
    /// the pins encloses that hull. For two pins it is their distance.
    pub fn bound_mm(&self, net: &str) -> Option<f64> {
        let hull = convex_hull(self.net_points(net)?);
        let perimeter: f64 = (0..hull.len())
            .map(|i| {
                let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
                (b.0 - a.0).hypot(b.1 - a.1)
            })
            .fold(0.0, |s, v| s + v);
        Some(perimeter / 2.0)
    }
}

/// Andrew's monotone chain. Collinear points leave a two-point hull, whose
/// perimeter is twice the span.
fn convex_hull(mut points: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };
    let mut hull: Vec<(f64, f64)> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let ordered: Box<dyn Iterator<Item = &(f64, f64)>> = if pass == 0 {
            Box::new(points.iter())
        } else {
            Box::new(points.iter().rev())
        };
        for &p in ordered {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0
            {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

/// A pin offset placed on the board: mirrored in x on the back side, then
/// turned counter-clockwise. Checked against Freerouting's own wire ends on
/// boards with back-side parts at 90° and 270°.
pub fn place_pin(x: f64, y: f64, back: bool, rotation_rad: f64) -> (f64, f64) {
    let x = if back { -x } else { x };
    let (sin, cos) = rotation_rad.sin_cos();
    (x * cos - y * sin, x * sin + y * cos)
}

/// Width compliance for one net class.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassWidth {
    pub class: String,
    pub width_mm: f64,
    pub segments: usize,
    pub narrow_segments: usize,
}

/// The quality of one routing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Quality {
    /// Nets with any wire.
    pub nets_routed: usize,
    /// Wiring of the routed nets whose lower bound is known: not plane nets.
    pub wirelength_mm: f64,
    /// Routed nets that also have a plane, left out of the ratio.
    pub plane_nets_routed: usize,
    /// Routed nets with wiring already in the DSN, left out of the ratio.
    pub prewired_nets_routed: usize,
    /// Their summed HPWL (reference; see `DsnBoard::hpwl_mm`).
    pub hpwl_mm: f64,
    /// Their summed any-angle lower bound (`DsnBoard::bound_mm`).
    pub bound_mm: f64,
    pub vias: usize,
    /// Vias where the net's wiring actually changes layer (wires on two or
    /// more layers end there). A via that only drops into a pour is not one.
    pub layer_changes: usize,
    /// Corners of a trace, away from pads and vias, at 90°.
    pub right_corners: usize,
    /// Corners under 90°: acid traps.
    pub acute_corners: usize,
    /// Unique segments.
    pub segments: usize,
    /// Segments narrower than their net class's width.
    pub narrow_segments: usize,
    pub widths: Vec<ClassWidth>,
    /// SES nets with wiring that the DSN does not name.
    pub unknown_nets: usize,
}

impl Quality {
    /// Wiring over its any-angle lower bound: 1.0 is the unreachable ideal,
    /// and nothing goes below it.
    pub fn wire_ratio(&self) -> Option<f64> {
        (self.bound_mm > 0.0).then(|| self.wirelength_mm / self.bound_mm)
    }

    /// Wiring over HPWL. Can go below 1.0 with diagonal wiring.
    pub fn hpwl_ratio(&self) -> Option<f64> {
        (self.hpwl_mm > 0.0).then(|| self.wirelength_mm / self.hpwl_mm)
    }

    pub fn vias_per_net(&self) -> Option<f64> {
        (self.nets_routed > 0).then(|| self.vias as f64 / self.nets_routed as f64)
    }
}

/// `(layer, a, b)` with `a <= b`, in SES units.
type Segment = (String, (i64, i64), (i64, i64));

const RIGHT_ANGLE_TOLERANCE_DEG: f64 = 0.5;
/// A pin within this distance of a corner covers it with pad copper.
const PIN_CAPTURE_MM: f64 = 0.01;

/// Measure a session file against its DSN.
pub fn measure(board: &DsnBoard, ses: &str) -> Result<Quality, String> {
    let root = parse_sexp(ses).ok_or("the SES is not one balanced S-expression")?;
    let routes = root
        .first("routes")
        .ok_or("the SES has no routes section")?;
    let resolution = routes
        .first("resolution")
        .ok_or("the SES has no resolution")?;
    let unit = resolution.items().get(1).and_then(Sexp::atom).unwrap_or("");
    let per_unit = number(resolution.items().get(2))
        .filter(|v| *v > 0.0)
        .ok_or("the SES resolution has no positive value")?;
    let mm = unit_mm(unit).ok_or_else(|| format!("unknown SES unit {unit:?}"))? / per_unit;

    // Pins on a 0.01 mm grid, for "is this corner on a pad".
    let grid = |v: f64| (v / PIN_CAPTURE_MM).round() as i64;
    let pin_cells: BTreeSet<(i64, i64)> = board
        .pins
        .values()
        .map(|(x, y)| (grid(*x), grid(*y)))
        .collect();
    let on_pin = |x: f64, y: f64| {
        let (cx, cy) = (grid(x), grid(y));
        (-1..=1).any(|dx| (-1..=1).any(|dy| pin_cells.contains(&(cx + dx, cy + dy))))
    };

    let mut quality = Quality::default();
    let mut classes: BTreeMap<String, ClassWidth> = BTreeMap::new();
    let Some(network) = routes.first("network_out") else {
        return Ok(quality);
    };
    for net in direct(network, "net") {
        let name = net.items().get(1).and_then(Sexp::atom).unwrap_or("");
        let required = board
            .net_width
            .get(name)
            .cloned()
            .or_else(|| board.default_width_mm.map(|w| ("(default)".to_string(), w)));
        let mut segments: BTreeSet<Segment> = BTreeSet::new();
        let mut widths: HashMap<Segment, f64> = HashMap::new();
        let mut vias: Vec<(i64, i64)> = Vec::new();
        for item in net.items() {
            match item.head() {
                Some("via") => {
                    let atoms: Vec<&str> =
                        item.items().iter().skip(2).filter_map(Sexp::atom).collect();
                    if let [x, y, ..] = atoms.as_slice() {
                        if let (Ok(x), Ok(y)) = (x.parse::<f64>(), y.parse::<f64>()) {
                            vias.push((x.round() as i64, y.round() as i64));
                        }
                    }
                }
                Some("wire") => {
                    for path in direct(item, "path") {
                        let fields = path.items();
                        let layer = fields.get(1).and_then(Sexp::atom).unwrap_or("").to_string();
                        let width = number(fields.get(2)).unwrap_or(0.0) * mm;
                        let coords: Vec<i64> = fields
                            .iter()
                            .skip(3)
                            .filter_map(Sexp::atom)
                            .filter_map(|v| v.parse::<f64>().ok())
                            .map(|v| v.round() as i64)
                            .collect();
                        let points: Vec<(i64, i64)> =
                            coords.chunks_exact(2).map(|c| (c[0], c[1])).collect();
                        for pair in points.windows(2) {
                            if pair[0] == pair[1] {
                                continue;
                            }
                            let (a, b) = if pair[0] <= pair[1] {
                                (pair[0], pair[1])
                            } else {
                                (pair[1], pair[0])
                            };
                            let key = (layer.clone(), a, b);
                            if segments.insert(key.clone()) {
                                widths.insert(key, width);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        quality.vias += vias.len();
        if segments.is_empty() {
            continue;
        }
        quality.nets_routed += 1;
        quality.segments += segments.len();
        if !board.nets.contains_key(name) {
            quality.unknown_nets += 1;
        }
        let length: f64 = segments
            .iter()
            .map(|(_, a, b)| ((b.0 - a.0) as f64).hypot((b.1 - a.1) as f64) * mm)
            .fold(0.0, |s, v| s + v);
        if board.plane_nets.contains(name) {
            quality.plane_nets_routed += 1;
        } else if board.prewired_nets.contains(name) {
            quality.prewired_nets_routed += 1;
        } else if let (Some(bound), Some(hpwl)) = (
            board.bound_mm(name).filter(|b| *b > 0.0),
            board.hpwl_mm(name),
        ) {
            quality.wirelength_mm += length;
            quality.bound_mm += bound;
            quality.hpwl_mm += hpwl;
        }

        // Directions leaving each node, per layer.
        let mut nodes: HashMap<(String, (i64, i64)), Vec<f64>> = HashMap::new();
        for (layer, a, b) in &segments {
            let forward = ((b.1 - a.1) as f64).atan2((b.0 - a.0) as f64).to_degrees();
            let backward = ((a.1 - b.1) as f64).atan2((a.0 - b.0) as f64).to_degrees();
            nodes.entry((layer.clone(), *a)).or_default().push(forward);
            nodes.entry((layer.clone(), *b)).or_default().push(backward);
        }
        let via_set: BTreeSet<(i64, i64)> = vias.iter().copied().collect();
        for via in &vias {
            let layers: BTreeSet<&String> = nodes
                .keys()
                .filter(|(_, point)| point == via)
                .map(|(layer, _)| layer)
                .collect();
            if layers.len() >= 2 {
                quality.layer_changes += 1;
            }
        }
        for ((_, point), mut directions) in nodes {
            if directions.len() < 2
                || via_set.contains(&point)
                || on_pin(point.0 as f64 * mm, point.1 as f64 * mm)
            {
                continue;
            }
            directions.sort_by(|a, b| a.total_cmp(b));
            let gaps: Vec<f64> = (0..directions.len())
                .map(|i| {
                    let next = directions[(i + 1) % directions.len()];
                    let gap = next - directions[i];
                    if gap <= 0.0 {
                        gap + 360.0
                    } else {
                        gap
                    }
                })
                .collect();
            // Two directions make one corner (the smaller gap); more make
            // every gap a corner between neighbouring traces.
            let corners: Vec<f64> = if gaps.len() == 2 {
                vec![gaps[0].min(gaps[1])]
            } else {
                gaps
            };
            for angle in corners {
                if (angle - 90.0).abs() <= RIGHT_ANGLE_TOLERANCE_DEG {
                    quality.right_corners += 1;
                } else if angle < 90.0 - RIGHT_ANGLE_TOLERANCE_DEG {
                    quality.acute_corners += 1;
                }
            }
        }

        if let Some((class, width)) = required {
            let entry = classes.entry(class.clone()).or_insert(ClassWidth {
                class,
                width_mm: width,
                segments: 0,
                narrow_segments: 0,
            });
            for segment in &segments {
                entry.segments += 1;
                if widths.get(segment).copied().unwrap_or(0.0) < width - 0.0005 {
                    entry.narrow_segments += 1;
                    quality.narrow_segments += 1;
                }
            }
        }
    }
    quality.widths = classes.into_values().collect();
    Ok(quality)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two resistors, R2 on the back turned 90°, a class for PWR.
    const DSN: &str = r#"(pcb test
  (parser (string_quote "))
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer In1.Cu (type power))
    (layer B.Cu (type signal))
    (rule (width 200) (clearance 200))
    (plane GND (polygon B.Cu 0  0 0  1000 0  1000 1000))
  )
  (placement
    (component R0402
      (place R1 0 0 front 0)
      (place R2 10000 0 back 90)
    )
  )
  (library
    (image R0402
      (pin Rect[T]Pad (rotate 90) 1 -500 0)
      (pin Rect[T]Pad 2 500 0)
    )
  )
  (network
    (net SIG (pins R1-2 R2-1))
    (net PWR (pins R1-1 R2-2))
    (class kicad_default SIG (rule (width 200)))
    (class power PWR (rule (width 500)))
  )
)"#;

    #[test]
    fn pins_are_placed_mirrored_then_turned() {
        let board = DsnBoard::parse(DSN).unwrap();
        assert_eq!((board.components, board.signal_layers), (2, 2));
        let close =
            |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9;
        assert!(close(board.pins["R1-1"], (-0.5, 0.0)));
        assert!(close(board.pins["R1-2"], (0.5, 0.0)));
        // R2 on the back: pin 1 at x -500 mirrors to +500, then turns 90° to (0, 500).
        assert!(
            close(board.pins["R2-1"], (10.0, 0.5)),
            "{:?}",
            board.pins["R2-1"]
        );
        assert!(
            close(board.pins["R2-2"], (10.0, -0.5)),
            "{:?}",
            board.pins["R2-2"]
        );
        assert_eq!(board.net_width["PWR"], ("power".to_string(), 0.5));
        assert_eq!(board.default_width_mm, Some(0.2));
        assert!(board.plane_nets.contains("GND"));
        // SIG: from (0.5, 0) to (10, 0.5): 9.5 + 0.5.
        assert!((board.hpwl_mm("SIG").unwrap() - 10.0).abs() < 1e-9);
        assert_eq!(board.hpwl_mm("NOPE"), None);
    }

    #[test]
    fn the_lower_bound_is_half_the_convex_hull_perimeter() {
        let mut board = DsnBoard::parse(DSN).unwrap();
        let square = [
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (5.0, 5.0),
        ];
        for (i, p) in square.iter().enumerate() {
            board.pins.insert(format!("Q{i}-1"), *p);
        }
        board.nets.insert(
            "SQ".into(),
            (0..square.len()).map(|i| format!("Q{i}-1")).collect(),
        );
        // Square of side 10: hull perimeter 40, bound 20; HPWL is also 20.
        assert!((board.bound_mm("SQ").unwrap() - 20.0).abs() < 1e-9);
        // A diagonal pair: the bound is the distance, below the HPWL.
        board
            .nets
            .insert("DIAG".into(), vec!["Q0-1".into(), "Q2-1".into()]);
        assert!((board.bound_mm("DIAG").unwrap() - 200f64.sqrt()).abs() < 1e-9);
        assert!((board.hpwl_mm("DIAG").unwrap() - 20.0).abs() < 1e-9);
        // Three points on a line: twice the span around, so the span.
        board.pins.insert("L0-1".into(), (0.0, 0.0));
        board.pins.insert("L1-1".into(), (3.0, 4.0));
        board.pins.insert("L2-1".into(), (6.0, 8.0));
        board.nets.insert(
            "LINE".into(),
            vec!["L0-1".into(), "L1-1".into(), "L2-1".into()],
        );
        assert!((board.bound_mm("LINE").unwrap() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn a_rotation_without_mirroring_turns_counter_clockwise() {
        let (x, y) = place_pin(1.0, 0.0, false, 90f64.to_radians());
        assert!(x.abs() < 1e-12 && (y - 1.0).abs() < 1e-12);
        let (x, y) = place_pin(1.0, 0.0, true, 0.0);
        assert!((x + 1.0).abs() < 1e-12 && y.abs() < 1e-12);
    }

    fn ses(network: &str) -> String {
        format!("(session s (routes (resolution um 10) (network_out {network})))")
    }

    #[test]
    fn wirelength_is_compared_with_its_lower_bound() {
        let board = DsnBoard::parse(DSN).unwrap();
        // SIG from R1-2 (5000, 0) right to (100000, 0) then up to R2-1 (100000, 5000):
        // 9.5 mm + 0.5 mm, exactly its HPWL; the straight line is 9.513 mm.
        // One 90° corner, on no pad.
        let q = measure(
            &board,
            &ses("(net SIG (wire (path F.Cu 2000 5000 0 100000 0 100000 5000)))"),
        )
        .unwrap();
        assert_eq!(q.nets_routed, 1);
        assert!((q.wirelength_mm - 10.0).abs() < 1e-9, "{q:?}");
        assert!((q.hpwl_ratio().unwrap() - 1.0).abs() < 1e-9);
        let straight = 9.5f64.hypot(0.5);
        assert!((q.wire_ratio().unwrap() - 10.0 / straight).abs() < 1e-9);
        assert_eq!((q.right_corners, q.acute_corners), (1, 0));
        assert_eq!(q.segments, 2);
        assert_eq!(q.narrow_segments, 0);
        assert_eq!(q.vias_per_net(), Some(0.0));
    }

    #[test]
    fn acute_corners_vias_layer_changes_and_narrow_segments_are_counted() {
        let board = DsnBoard::parse(DSN).unwrap();
        let q = measure(
            &board,
            &ses("(net PWR \
                   (wire (path F.Cu 5000 -5000 0 50000 0 0 20000)) \
                   (wire (path B.Cu 2000 0 20000 100000 -5000)) \
                   (via V 0 20000) \
                   (via V 70000 70000) \
                   (wire (path F.Cu 5000 70000 70000 80000 70000)))"),
        )
        .unwrap();
        // (50000,0) is a corner between (-5000,0) and (0,20000): about 22°.
        assert_eq!(q.acute_corners, 1, "{q:?}");
        assert_eq!(q.vias, 2);
        // Wires on F.Cu and B.Cu meet at the first via; the second only has F.Cu.
        assert_eq!(q.layer_changes, 1);
        // PWR needs 0.5 mm; the B.Cu segment is 0.2 mm.
        assert_eq!(q.narrow_segments, 1);
        assert_eq!(q.widths.len(), 1);
        assert_eq!(q.widths[0].class, "power");
        assert_eq!((q.widths[0].segments, q.widths[0].narrow_segments), (4, 1));
        assert_eq!(q.acute_corners + q.narrow_segments, 2);
    }

    #[test]
    fn corners_on_pads_and_vias_and_straight_runs_are_not_counted() {
        let board = DsnBoard::parse(DSN).unwrap();
        // A sharp turn exactly on R1-2's pad, a straight joint, and a turn on a via.
        let q = measure(
            &board,
            &ses("(net SIG \
                   (wire (path F.Cu 2000 0 3000 5000 0 0 -3000)) \
                   (wire (path F.Cu 2000 20000 0 30000 0 40000 0)) \
                   (wire (path F.Cu 2000 60000 0 70000 0 70000 10000)) \
                   (via V 70000 0))"),
        )
        .unwrap();
        assert_eq!((q.right_corners, q.acute_corners), (0, 0), "{q:?}");
        assert_eq!(q.layer_changes, 0);
    }

    #[test]
    fn a_three_way_junction_counts_each_gap() {
        let board = DsnBoard::parse(DSN).unwrap();
        // A T away from any pad: two right angles and one straight (180°) gap.
        let q = measure(
            &board,
            &ses("(net SIG (wire (path F.Cu 2000 20000 0 30000 0)) (wire (path F.Cu 2000 30000 0 40000 0)) (wire (path F.Cu 2000 30000 0 30000 10000)))"),
        )
        .unwrap();
        assert_eq!((q.right_corners, q.acute_corners), (2, 0), "{q:?}");
    }

    #[test]
    fn real_fixture_boards_parse_and_measure() {
        let root = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/konnect-core/tests/fixtures/"
        );
        let dsn =
            std::fs::read_to_string(format!("{root}specctra_rounded_board.native-kicad-10.dsn"))
                .unwrap();
        let ses = std::fs::read_to_string(format!(
            "{root}specctra_rounded_board.freerouting-2.3.0.ses"
        ))
        .unwrap();
        let board = DsnBoard::parse(&dsn).unwrap();
        assert_eq!(board.components, 2);
        assert!(board.pins.len() >= 4, "{:?}", board.pins);
        let q = measure(&board, &ses).unwrap();
        assert!(q.nets_routed >= 1, "{q:?}");
        assert_eq!(q.unknown_nets, 0);
        // GND has a pour on this board, so only VCC is in the ratio.
        assert_eq!(q.plane_nets_routed, 1, "{q:?}");
        let ratio = q.wire_ratio().expect("a lower bound");
        assert!(
            ratio >= 1.0 - 1e-9,
            "wiring cannot beat its lower bound: {ratio}"
        );
    }

    #[test]
    fn nets_with_wiring_already_in_the_dsn_are_left_out_of_the_ratio() {
        let root = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/konnect-core/tests/fixtures/"
        );
        let dsn = std::fs::read_to_string(format!(
            "{root}specctra_two_resistors_locked.native-kicad-10.dsn"
        ))
        .unwrap();
        let ses = std::fs::read_to_string(format!(
            "{root}specctra_two_resistors_locked.freerouting-2.3.0.ses"
        ))
        .unwrap();
        let board = DsnBoard::parse(&dsn).unwrap();
        assert!(board.prewired_nets.contains("GND"), "{board:?}");
        let q = measure(&board, &ses).unwrap();
        assert!(q.prewired_nets_routed >= 1, "{q:?}");
        let ratio = q.wire_ratio().expect("VCC is routed by Freerouting alone");
        assert!(ratio >= 1.0 - 1e-9, "{ratio}");
    }

    #[test]
    fn broken_input_is_an_error() {
        assert!(DsnBoard::parse("(session x)").is_err());
        assert!(DsnBoard::parse("(pcb x (unit parsec))").is_err());
        let board = DsnBoard::parse(DSN).unwrap();
        assert!(measure(&board, "(session s").is_err());
        assert_eq!(
            measure(&board, "(session s (routes (resolution um 10)))").unwrap(),
            Quality::default()
        );
    }
}
