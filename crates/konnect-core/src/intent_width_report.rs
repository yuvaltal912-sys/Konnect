//! Row S9: what a routed session did to the nets that carry a recorded
//! current.
//!
//! The export widens such a net to the width its current needs and records
//! that in the manifest's `intent_widths`. Freerouting's retry width can then
//! lay part of the net narrower, down to the net's floor, and the importer
//! accepts it. This module reads the final session's geometry and says, per
//! net, how much track is below the intent width, and lists each stretch that
//! is: nothing here comes from a pass's bookkeeping.
//!
//! A net is judged when the export widened it (`widened`: against the width
//! it was widened to) or found it wide enough (`already_wide_enough`: against
//! the width its current needs). A net that owns a pour (`not_widened`) and a
//! net the board does not route are not judged.
//!
//! The report is never silent about a judged net. Keywords are read in any
//! letter case. A wire is measured when it is a `path`, a `polyline_path` or
//! a `qarc`; a wire of a judged net in any other form, or with no shape, or a
//! polyline whose lines do not meet, is listed as unmeasured, and the routing
//! result is then never reported complete. A session that cannot be read at
//! all is an error, with the same effect.
//!
//! What a stretch is: narrow pieces that share an end point, on any layer (no
//! via is looked for). A narrow wire that ends in the middle of another one
//! (a T) is a stretch of its own, so `longest_below_intent_mm` can be shorter
//! than the narrow copper that really hangs together there. Copper the board
//! already held locked is not in a session and is not judged.

use crate::specctra_seed::{
    lists, parse_specctra, session_routes, MAX_FORM_CHARS, WIRE_DESCRIPTORS,
};
use anyhow::{bail, Context, Result};
use konnect_sexp::SexpNode;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The most stretches a result lists, and the most unmeasured wires; the
/// count of all of them is reported beside each list.
pub(crate) const MAX_WIDTH_EXCEPTIONS: usize = 100;

/// A wire this far below its intent width, or less, is at width: the same
/// half micrometre the importer allows below a net's floor. In whole
/// nanometres, so that the boundary is a rule and not rounding noise.
const WIDTH_TOLERANCE_NM: i64 = 500;

/// The per-net figures and the stretches below intent width of one session.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct WidthReport {
    /// One row per judged net, in the manifest's order.
    pub nets: Vec<Value>,
    /// The longest stretches below intent width, at most
    /// [`MAX_WIDTH_EXCEPTIONS`], longest first.
    pub exceptions: Vec<Value>,
    /// How many stretches there are in all.
    pub exceptions_total: usize,
    /// Wires of judged nets that could not be measured, at most
    /// [`MAX_WIDTH_EXCEPTIONS`]: a run with any is never reported complete.
    pub unmeasured: Vec<Value>,
    /// How many such wires there are in all.
    pub unmeasured_total: usize,
}

struct Judged {
    intent_mm: f64,
    basis: &'static str,
}

/// The nets the export recorded a width for, with that width.
fn judged_nets(manifest: &Value) -> Vec<(String, Judged)> {
    manifest["intent_widths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let net = row["net"].as_str()?.to_string();
            let (width, basis) = match row["status"].as_str()? {
                "widened" => (row["to_mm"].as_f64()?, "widened"),
                "already_wide_enough" => (row["needed_mm"].as_f64()?, "already_wide_enough"),
                _ => return None,
            };
            // A JSON number is always finite; a width of 0 or less is no width.
            (width > 0.0).then_some((
                net,
                Judged {
                    intent_mm: width,
                    basis,
                },
            ))
        })
        .collect()
}

/// Whether the manifest names a net whose width this report judges.
pub(crate) fn has_judged_net(manifest: &Value) -> bool {
    !judged_nets(manifest).is_empty()
}

/// One piece of track of a judged net: a path's polyline or an arc.
struct Piece {
    layer: String,
    width_mm: f64,
    /// The width in whole nanometres, for the comparison with the intent.
    width_nm: i64,
    length_mm: f64,
    /// Both ends in session units, for joining pieces end to end.
    ends: [(i64, i64); 2],
}

/// A wire of a judged net that could not be measured.
struct Unmeasured {
    form: String,
    width_mm: Option<f64>,
    reason: &'static str,
}

/// What a session holds of one judged net.
#[derive(Default)]
struct NetTrack {
    pieces: Vec<Piece>,
    unmeasured: Vec<Unmeasured>,
}

fn atom<'a>(node: &'a SexpNode, index: usize, what: &str) -> Result<&'a str> {
    node.get(index)
        .and_then(SexpNode::as_str)
        .with_context(|| format!("the session's {what} is missing"))
}

fn number(text: &str, what: &str) -> Result<f64> {
    let value: f64 = text
        .parse()
        .map_err(|_| anyhow::anyhow!("the session's {what} is not a number"))?;
    if !value.is_finite() {
        bail!("the session's {what} is not finite");
    }
    Ok(value)
}

fn round3(value: f64) -> f64 {
    let rounded = (value * 1000.0).round() / 1000.0;
    // Never "-0.0" in a result.
    if rounded == 0.0 {
        0.0
    } else {
        rounded
    }
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

/// The corners of a `polyline_path`: its values are lines, four numbers each
/// (two points the line passes through), and the track runs between the
/// points where each line meets the next. `None` when two lines that follow
/// each other do not meet.
fn polyline_corners(values: &[f64]) -> Option<Vec<(f64, f64)>> {
    let lines: Vec<[(f64, f64); 2]> = values
        .chunks_exact(4)
        .map(|line| [(line[0], line[1]), (line[2], line[3])])
        .collect();
    lines
        .windows(2)
        .map(|pair| {
            let ([a, b], [c, d]) = (pair[0], pair[1]);
            let (along, other) = ((b.0 - a.0, b.1 - a.1), (d.0 - c.0, d.1 - c.1));
            let cross = along.0 * other.1 - along.1 * other.0;
            if cross == 0.0 {
                return None;
            }
            let t = ((c.0 - a.0) * other.1 - (c.1 - a.1) * other.0) / cross;
            Some((a.0 + t * along.0, a.1 + t * along.1))
        })
        .collect()
}

/// Read one `wire` node into `track`. `um_per_unit` is micrometres per
/// session unit.
fn wire_pieces(
    wire: &SexpNode,
    um_per_unit: f64,
    layers: &BTreeMap<&str, &str>,
    track: &mut NetTrack,
) -> Result<()> {
    let mm = |units: f64| units * um_per_unit / 1000.0;
    let key = |point: (f64, f64)| (point.0.round() as i64, point.1.round() as i64);
    let found_before = track.pieces.len() + track.unmeasured.len();
    for shape in wire.children().unwrap_or(&[]) {
        // An atom (the keyword itself) is not a shape.
        let Some(head) = shape.head() else { continue };
        let form = head.to_ascii_lowercase();
        if WIRE_DESCRIPTORS.contains(&form.as_str()) {
            continue;
        }
        if !matches!(form.as_str(), "path" | "polyline_path" | "qarc") {
            track.unmeasured.push(Unmeasured {
                form: form.chars().take(MAX_FORM_CHARS).collect(),
                width_mm: None,
                reason: "a wire shape this report does not measure",
            });
            continue;
        }
        let layer = atom(shape, 1, "wire layer")?;
        let layer = layers.get(layer).copied().unwrap_or(layer).to_string();
        let width_mm = mm(number(atom(shape, 2, "wire width")?, "wire width")?);
        if width_mm <= 0.0 {
            bail!("the session holds a wire with no width");
        }
        let values: Vec<f64> = shape
            .children()
            .unwrap_or(&[])
            .iter()
            .skip(3)
            .map(|value| {
                number(
                    value
                        .as_str()
                        .context("the session's wire holds a list where a coordinate belongs")?,
                    "wire coordinate",
                )
            })
            .collect::<Result<_>>()?;
        let (length, ends) = match form.as_str() {
            "path" => {
                if values.len() < 4 || !values.len().is_multiple_of(2) {
                    bail!("a session path does not hold complete point pairs");
                }
                let points: Vec<(f64, f64)> =
                    values.chunks_exact(2).map(|p| (p[0], p[1])).collect();
                let length: f64 = points
                    .windows(2)
                    .map(|pair| distance(pair[0], pair[1]))
                    .sum();
                (length, [points[0], points[points.len() - 1]])
            }
            "polyline_path" => {
                // Three lines meet in two points: the least that is a track.
                if values.len() < 12 || !values.len().is_multiple_of(4) {
                    bail!("a session polyline does not hold three whole lines or more");
                }
                let Some(corners) = polyline_corners(&values) else {
                    track.unmeasured.push(Unmeasured {
                        form,
                        width_mm: Some(round3(width_mm)),
                        reason: "two lines of the polyline that follow each other do not meet",
                    });
                    continue;
                };
                let length: f64 = corners
                    .windows(2)
                    .map(|pair| distance(pair[0], pair[1]))
                    .sum();
                (length, [corners[0], corners[corners.len() - 1]])
            }
            _ => {
                // (qarc layer width x1 y1 x2 y2 cx cy): a quarter arc at most.
                if values.len() != 6 {
                    bail!("a session arc does not hold a start, an end and a centre");
                }
                let (start, end, centre) = (
                    (values[0], values[1]),
                    (values[2], values[3]),
                    (values[4], values[5]),
                );
                let radius = distance(centre, start);
                let chord = distance(start, end);
                let length = if radius > 0.0 {
                    2.0 * radius * (chord / (2.0 * radius)).clamp(-1.0, 1.0).asin()
                } else {
                    chord
                };
                (length, [start, end])
            }
        };
        let length_mm = mm(length);
        if !length_mm.is_finite() || !width_mm.is_finite() {
            bail!("the session holds a wire too large to measure");
        }
        track.pieces.push(Piece {
            layer,
            width_mm,
            width_nm: (width_mm * 1e6).round() as i64,
            length_mm,
            ends: [key(ends[0]), key(ends[1])],
        });
    }
    if track.pieces.len() + track.unmeasured.len() == found_before {
        track.unmeasured.push(Unmeasured {
            form: "none".to_string(),
            width_mm: None,
            reason: "the wire holds no shape",
        });
    }
    Ok(())
}

/// The judged nets' track in a session, by net name, and micrometres per
/// session unit.
fn session_track(
    ses: &str,
    manifest: &Value,
    wanted: &BTreeSet<&str>,
) -> Result<(BTreeMap<String, NetTrack>, f64)> {
    let root = parse_specctra(ses, "session")?;
    let (routes, um_per_unit) = session_routes(&root)?;
    let layers: BTreeMap<&str, &str> = manifest["layers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|layer| Some((layer["dsn_name"].as_str()?, layer["kicad_name"].as_str()?)))
        .collect();
    let mut by_net: BTreeMap<String, NetTrack> = BTreeMap::new();
    for network in lists(routes, "network_out") {
        for net in lists(network, "net") {
            let name = atom(net, 1, "net name")?;
            if !wanted.contains(name) {
                continue;
            }
            let track = by_net.entry(name.to_string()).or_default();
            for wire in lists(net, "wire") {
                wire_pieces(wire, um_per_unit, &layers, track)?;
            }
        }
    }
    Ok((by_net, um_per_unit))
}

/// Groups of pieces joined end to end, as lists of indexes into `pieces`.
fn stretches(pieces: &[&Piece]) -> Vec<Vec<usize>> {
    fn root(parent: &mut [usize], mut at: usize) -> usize {
        while parent[at] != at {
            parent[at] = parent[parent[at]];
            at = parent[at];
        }
        at
    }
    let mut parent: Vec<usize> = (0..pieces.len()).collect();
    let mut at_point: BTreeMap<(i64, i64), usize> = BTreeMap::new();
    for (index, piece) in pieces.iter().enumerate() {
        for end in piece.ends {
            match at_point.get(&end) {
                Some(&other) => {
                    let (a, b) = (root(&mut parent, index), root(&mut parent, other));
                    parent[a.max(b)] = a.min(b);
                }
                None => {
                    at_point.insert(end, index);
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..pieces.len() {
        let top = root(&mut parent, index);
        groups.entry(top).or_default().push(index);
    }
    groups.into_values().collect()
}

/// The width report of `ses` against the widths `manifest` records. With no
/// judged net the session is not read at all and the report is empty.
pub(crate) fn intent_width_report(manifest: &Value, ses: &str) -> Result<WidthReport> {
    let judged = judged_nets(manifest);
    if judged.is_empty() {
        return Ok(WidthReport::default());
    }
    let wanted: BTreeSet<&str> = judged.iter().map(|(net, _)| net.as_str()).collect();
    let (by_net, um_per_unit) = session_track(ses, manifest, &wanted)?;
    let point_mm = |point: (i64, i64)| {
        // Board coordinates: the session's y axis points up, KiCad's down.
        json!([
            round3(point.0 as f64 * um_per_unit / 1000.0),
            round3(-(point.1 as f64) * um_per_unit / 1000.0)
        ])
    };
    let mut report = WidthReport::default();
    let mut all: Vec<(f64, Value)> = Vec::new();
    let mut seen = BTreeSet::new();
    let empty = NetTrack::default();
    for (net, rule) in &judged {
        if !seen.insert(net.as_str()) {
            continue; // a net named twice is judged once, by its first row
        }
        let track = by_net.get(net).unwrap_or(&empty);
        let pieces = &track.pieces;
        for wire in &track.unmeasured {
            report.unmeasured_total += 1;
            if report.unmeasured.len() < MAX_WIDTH_EXCEPTIONS {
                let mut row = json!({ "net": net, "form": wire.form, "reason": wire.reason });
                if let Some(width) = wire.width_mm {
                    row["width_mm"] = json!(width);
                }
                report.unmeasured.push(row);
            }
        }
        let routed: f64 = pieces.iter().map(|piece| piece.length_mm).sum();
        // In whole nanometres: a wire exactly half a micrometre below the
        // intent is at width, one nanometre more is not.
        let least_nm = (rule.intent_mm * 1e6).round() as i64 - WIDTH_TOLERANCE_NM;
        let below: Vec<&Piece> = pieces
            .iter()
            .filter(|piece| piece.width_nm < least_nm)
            .collect();
        let below_mm: f64 = below.iter().map(|piece| piece.length_mm).sum();
        let groups = stretches(&below);
        let mut longest = 0.0f64;
        for group in &groups {
            let length: f64 = group.iter().map(|&index| below[index].length_mm).sum();
            longest = longest.max(length);
            let width = group
                .iter()
                .map(|&index| below[index].width_mm)
                .fold(f64::INFINITY, f64::min);
            let layers: BTreeSet<&str> = group
                .iter()
                .map(|&index| below[index].layer.as_str())
                .collect();
            // The ends of the stretch: points only one of its pieces touches.
            let mut touches: BTreeMap<(i64, i64), usize> = BTreeMap::new();
            for &index in group {
                for end in below[index].ends {
                    *touches.entry(end).or_default() += 1;
                }
            }
            let mut ends: Vec<Value> = touches
                .iter()
                .filter(|(_, count)| **count == 1)
                .map(|(point, _)| point_mm(*point))
                .collect();
            if ends.is_empty() {
                ends = touches
                    .keys()
                    .take(1)
                    .map(|point| point_mm(*point))
                    .collect();
            }
            ends.truncate(4);
            all.push((
                length,
                json!({
                    "net": net,
                    "intent_width_mm": rule.intent_mm,
                    "width_mm": round3(width),
                    "length_mm": round3(length),
                    "layers": layers,
                    "ends_mm": ends,
                }),
            ));
        }
        report.nets.push(json!({
            "net": net,
            "intent_width_mm": rule.intent_mm,
            "basis": rule.basis,
            "routed_mm": round3(routed),
            "below_intent_mm": round3(below_mm),
            "below_intent_share": if routed > 0.0 { round3(below_mm / routed) } else { 0.0 },
            "longest_below_intent_mm": round3(longest),
            "narrowest_width_mm": pieces
                .iter()
                .map(|piece| piece.width_mm)
                .fold(None, |least: Option<f64>, width| Some(least.map_or(width, |l| l.min(width))))
                .map(round3),
            "stretches_below_intent": groups.len(),
            "unmeasured_wires": track.unmeasured.len(),
        }));
    }
    report.exceptions_total = all.len();
    // Longest first; ties keep the manifest's net order.
    all.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    report.exceptions = all
        .into_iter()
        .take(MAX_WIDTH_EXCEPTIONS)
        .map(|(_, row)| row)
        .collect();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A manifest with one widened net (VBUS to 1.0 mm), one net wide enough
    /// (VAUX needs 0.4 mm), a pour net and a net the board does not route.
    fn manifest() -> Value {
        json!({
            "layers": [
                { "dsn_name": "F.Cu", "kicad_name": "F.Cu" },
                { "dsn_name": "B.Cu", "kicad_name": "B.Cu" }
            ],
            "intent_widths": [
                { "net": "VBUS", "current_a": 2.0, "from_mm": 0.2, "to_mm": 1.0, "status": "widened" },
                { "net": "VAUX", "current_a": 0.5, "width_mm": 0.5, "needed_mm": 0.4, "status": "already_wide_enough" },
                { "net": "GND", "current_a": 2.0, "from_mm": 0.2, "to_mm": 0.2, "status": "not_widened" },
                { "net": "GHOST", "current_a": 1.0, "status": "not_routed" }
            ]
        })
    }

    /// A session at `resolution` units per micrometre. Each wire is
    /// `(net, layer, width in um, points in um)`.
    /// A wire of a test session: net, layer, width and points, in micrometres.
    type Wire<'a> = (&'a str, &'a str, f64, &'a [(f64, f64)]);

    fn session(resolution: i64, wires: &[Wire]) -> String {
        let mut nets: BTreeMap<&str, String> = BTreeMap::new();
        for (net, layer, width, points) in wires {
            let coordinates: Vec<String> = points
                .iter()
                .flat_map(|(x, y)| [x * resolution as f64, y * resolution as f64])
                .map(|value| format!("{value}"))
                .collect();
            nets.entry(net).or_default().push_str(&format!(
                "(wire (path {layer} {} {}))",
                width * resolution as f64,
                coordinates.join(" ")
            ));
        }
        let body: String = nets
            .iter()
            .map(|(net, wires)| format!("(net {net} {wires})"))
            .collect();
        format!("(session board (routes (resolution um {resolution}) (network_out {body})))")
    }

    fn net_row<'a>(report: &'a WidthReport, net: &str) -> &'a Value {
        report.nets.iter().find(|row| row["net"] == net).unwrap()
    }

    #[test]
    fn a_wire_below_intent_width_is_listed_with_its_length() {
        let ses = session(
            10,
            &[
                (
                    "VBUS",
                    "F.Cu",
                    1000.0,
                    &[(0.0, 0.0), (3000.0, 0.0), (3000.0, 4000.0)],
                ),
                ("VBUS", "F.Cu", 200.0, &[(3000.0, 4000.0), (3000.0, 6500.0)]),
            ],
        );
        let report = intent_width_report(&manifest(), &ses).unwrap();
        let row = net_row(&report, "VBUS");
        assert_eq!(row["intent_width_mm"], 1.0);
        assert_eq!(row["basis"], "widened");
        assert_eq!(row["routed_mm"], 9.5);
        assert_eq!(row["below_intent_mm"], 2.5);
        assert_eq!(row["below_intent_share"], 0.263);
        assert_eq!(row["longest_below_intent_mm"], 2.5);
        assert_eq!(row["narrowest_width_mm"], 0.2);
        assert_eq!(row["stretches_below_intent"], 1);
        assert_eq!(report.exceptions_total, 1);
        assert_eq!(
            report.exceptions,
            vec![json!({
                "net": "VBUS", "intent_width_mm": 1.0, "width_mm": 0.2, "length_mm": 2.5,
                "layers": ["F.Cu"],
                // The session's y points up; the board's points down.
                "ends_mm": [[3.0, -4.0], [3.0, -6.5]],
            })]
        );
    }

    #[test]
    fn a_wire_at_intent_width_or_within_half_a_micrometre_is_not_listed() {
        for (width, listed) in [(1000.0, 0), (1200.0, 0), (999.6, 0), (999.0, 1), (500.0, 1)] {
            let ses = session(10, &[("VBUS", "F.Cu", width, &[(0.0, 0.0), (1000.0, 0.0)])]);
            let report = intent_width_report(&manifest(), &ses).unwrap();
            assert_eq!(report.exceptions_total, listed, "width {width} um");
            assert_eq!(report.exceptions.len(), listed, "width {width} um");
            assert_eq!(net_row(&report, "VBUS")["stretches_below_intent"], listed);
        }
    }

    #[test]
    fn a_net_that_was_wide_enough_is_judged_against_the_width_its_current_needs() {
        let at = session(10, &[("VAUX", "F.Cu", 400.0, &[(0.0, 0.0), (1000.0, 0.0)])]);
        assert_eq!(
            intent_width_report(&manifest(), &at)
                .unwrap()
                .exceptions_total,
            0
        );
        let below = session(10, &[("VAUX", "F.Cu", 200.0, &[(0.0, 0.0), (1000.0, 0.0)])]);
        let report = intent_width_report(&manifest(), &below).unwrap();
        assert_eq!(report.exceptions_total, 1);
        assert_eq!(report.exceptions[0]["intent_width_mm"], 0.4);
        assert_eq!(net_row(&report, "VAUX")["basis"], "already_wide_enough");
    }

    #[test]
    fn a_pour_net_an_unrouted_name_and_any_other_net_are_not_judged() {
        let ses = session(
            10,
            &[
                ("GND", "F.Cu", 100.0, &[(0.0, 0.0), (1000.0, 0.0)]),
                ("GHOST", "F.Cu", 100.0, &[(0.0, 0.0), (1000.0, 0.0)]),
                ("SIG", "F.Cu", 100.0, &[(0.0, 0.0), (1000.0, 0.0)]),
            ],
        );
        let report = intent_width_report(&manifest(), &ses).unwrap();
        assert_eq!(report.exceptions_total, 0);
        let nets: Vec<&str> = report
            .nets
            .iter()
            .map(|row| row["net"].as_str().unwrap())
            .collect();
        assert_eq!(
            nets,
            ["VBUS", "VAUX"],
            "only the nets with a recorded width"
        );
        // A judged net with no track in the session is said to have none.
        let row = net_row(&report, "VBUS");
        assert_eq!(row["routed_mm"], 0.0);
        assert_eq!(row["below_intent_share"], 0.0);
        assert!(row["narrowest_width_mm"].is_null());
    }

    #[test]
    fn narrow_wires_joined_end_to_end_are_one_stretch_and_apart_are_two() {
        let ses = session(
            10,
            &[
                // One stretch through a layer change at (2000, 0).
                ("VBUS", "F.Cu", 200.0, &[(0.0, 0.0), (2000.0, 0.0)]),
                ("VBUS", "B.Cu", 300.0, &[(2000.0, 0.0), (2000.0, 1000.0)]),
                // A full-width wire between them and the second stretch.
                (
                    "VBUS",
                    "B.Cu",
                    1000.0,
                    &[(2000.0, 1000.0), (9000.0, 1000.0)],
                ),
                ("VBUS", "B.Cu", 200.0, &[(9000.0, 1000.0), (9000.0, 1500.0)]),
            ],
        );
        let report = intent_width_report(&manifest(), &ses).unwrap();
        assert_eq!(report.exceptions_total, 2);
        let first = &report.exceptions[0];
        assert_eq!(first["length_mm"], 3.0, "the longest comes first");
        assert_eq!(first["width_mm"], 0.2, "the narrowest of the stretch");
        assert_eq!(first["layers"], json!(["B.Cu", "F.Cu"]));
        assert_eq!(first["ends_mm"], json!([[0.0, 0.0], [2.0, -1.0]]));
        assert_eq!(report.exceptions[1]["length_mm"], 0.5);
        let row = net_row(&report, "VBUS");
        assert_eq!(row["below_intent_mm"], 3.5);
        assert_eq!(row["longest_below_intent_mm"], 3.0);
        assert_eq!(row["stretches_below_intent"], 2);
    }

    #[test]
    fn the_resolution_scales_widths_and_lengths() {
        let wires: &[Wire] = &[("VBUS", "F.Cu", 200.0, &[(0.0, 0.0), (2500.0, 0.0)])];
        let fine = intent_width_report(&manifest(), &session(10, wires)).unwrap();
        let coarse = intent_width_report(&manifest(), &session(1, wires)).unwrap();
        assert_eq!(fine, coarse);
        assert_eq!(fine.exceptions[0]["length_mm"], 2.5);
        assert_eq!(fine.exceptions[0]["width_mm"], 0.2);
    }

    #[test]
    fn layer_names_come_from_the_manifest_and_arcs_are_measured() {
        let mut manifest = manifest();
        manifest["layers"] = json!([{ "dsn_name": "1", "kicad_name": "In1.Cu" }]);
        // A quarter arc of radius 1 mm, at 0.2 mm.
        let ses = "(session b (routes (resolution um 10) (network_out (net VBUS (wire (qarc 1 2000 10000 0 0 10000 0 0))))))";
        let report = intent_width_report(&manifest, ses).unwrap();
        assert_eq!(report.exceptions[0]["layers"], json!(["In1.Cu"]));
        assert_eq!(report.exceptions[0]["length_mm"], 1.571);
    }

    #[test]
    fn a_recorded_width_of_zero_or_less_is_no_width() {
        for width in [0.0, -1.0] {
            let manifest = json!({ "intent_widths": [
                { "net": "VBUS", "from_mm": 0.2, "to_mm": width, "status": "widened" },
                { "net": "VAUX", "width_mm": 0.5, "needed_mm": width, "status": "already_wide_enough" }
            ] });
            assert!(!has_judged_net(&manifest), "{width}");
        }
    }

    #[test]
    fn an_arc_is_measured_wherever_its_centre_is() {
        // A quarter arc of radius 1 mm about (0.5, 0.5) mm, from a point off
        // both axes: every term of the radius and of the chord counts.
        let ses = "(session b (routes (resolution um 10) (network_out (net VBUS (wire (qarc F.Cu 2000 11000 13000 -3000 11000 5000 5000))))))";
        let report = intent_width_report(&manifest(), ses).unwrap();
        assert_eq!(report.exceptions[0]["length_mm"], 1.571);
        // An arc whose start is its centre has no radius: its chord is its length.
        let ses = "(session b (routes (resolution um 10) (network_out (net VBUS (wire (qarc F.Cu 2000 0 0 10000 0 0 0))))))";
        let report = intent_width_report(&manifest(), ses).unwrap();
        assert_eq!(report.exceptions[0]["length_mm"], 1.0);
    }

    #[test]
    fn a_path_needs_two_whole_points() {
        let path = |values: &str| {
            format!("(session b (routes (resolution um 10) (network_out (net VBUS (wire (path F.Cu 2000 {values}))))))")
        };
        // One point; two points and a half; no point at all.
        for values in ["0 0", "0 0 10 10 20", "0", ""] {
            let error = intent_width_report(&manifest(), &path(values)).unwrap_err();
            assert!(
                format!("{error:#}").contains("complete point pairs"),
                "{values:?}: {error:#}"
            );
        }
        assert!(intent_width_report(&manifest(), &path("0 0 10 10")).is_ok());
        assert!(intent_width_report(&manifest(), &path("0 0 10 10 20 20")).is_ok());
    }

    #[test]
    fn with_no_judged_net_the_session_is_not_read() {
        for manifest in [
            json!({}),
            json!({ "intent_widths": [] }),
            json!({ "intent_widths": [{ "net": "GND", "status": "not_widened", "from_mm": 0.2, "to_mm": 0.2 }] }),
        ] {
            assert!(!has_judged_net(&manifest));
            let report = intent_width_report(&manifest, "not a session at all (").unwrap();
            assert_eq!(report, WidthReport::default());
        }
        assert!(has_judged_net(&self::manifest()));
    }

    #[test]
    fn a_session_that_cannot_be_read_is_an_error_when_a_net_is_judged() {
        for (ses, needle) in [
            ("(session b (routes (resolution um 10)", "readable S-expression"),
            ("(pcb b)", "not a Specctra session"),
            ("(session b)", "no routes"),
            ("(session b (routes (network_out)))", "no resolution"),
            (
                "(session b (routes (resolution furlong 10) (network_out)))",
                "unit 'furlong' is not known",
            ),
            ("(session b (routes (resolution um 0) (network_out)))", "not positive"),
            (
                "(session b (routes (resolution um 10) (network_out (net VBUS (wire (path F.Cu 2000 0 0 10))))))",
                "complete point pairs",
            ),
            (
                "(session b (routes (resolution um 10) (network_out (net VBUS (wire (path F.Cu wide 0 0 10 10))))))",
                "not a number",
            ),
            (
                "(session b (routes (resolution um 10) (network_out (net VBUS (wire (path F.Cu 0 0 0 10 10))))))",
                "no width",
            ),
        ] {
            let error = intent_width_report(&manifest(), ses).unwrap_err();
            assert!(format!("{error:#}").contains(needle), "{ses}: {error:#}");
        }
        // The same trouble in a net that is not judged is not this report's.
        let other = "(session b (routes (resolution um 10) (network_out (net SIG (wire (path F.Cu wide 0 0 10 10))))))";
        assert!(intent_width_report(&manifest(), other).is_ok());
    }

    #[test]
    fn the_list_is_capped_and_the_count_is_not() {
        let wires: Vec<(f64, f64)> = (0..MAX_WIDTH_EXCEPTIONS + 5)
            .map(|i| (i as f64 * 10_000.0, 0.0))
            .collect();
        let body: String = wires
            .iter()
            .enumerate()
            .map(|(i, (x, _))| {
                format!(
                    "(wire (path F.Cu 2000 {x} 0 {} 0))",
                    x + 1000.0 + i as f64 * 10.0
                )
            })
            .collect();
        let ses =
            format!("(session b (routes (resolution um 10) (network_out (net VBUS {body}))))");
        let report = intent_width_report(&manifest(), &ses).unwrap();
        assert_eq!(report.exceptions_total, MAX_WIDTH_EXCEPTIONS + 5);
        assert_eq!(report.exceptions.len(), MAX_WIDTH_EXCEPTIONS);
        // The longest are the ones kept.
        let kept_shortest = report.exceptions.last().unwrap()["length_mm"]
            .as_f64()
            .unwrap();
        assert!(kept_shortest > 0.1 + 0.004, "{kept_shortest}");
        assert_eq!(
            net_row(&report, "VBUS")["stretches_below_intent"],
            MAX_WIDTH_EXCEPTIONS + 5
        );
    }

    #[test]
    fn a_net_named_twice_in_the_manifest_is_judged_once() {
        let mut manifest = manifest();
        manifest["intent_widths"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "net": "VBUS", "from_mm": 0.2, "to_mm": 5.0, "status": "widened" }));
        let ses = session(10, &[("VBUS", "F.Cu", 200.0, &[(0.0, 0.0), (1000.0, 0.0)])]);
        let report = intent_width_report(&manifest, &ses).unwrap();
        assert_eq!(
            report
                .nets
                .iter()
                .filter(|row| row["net"] == "VBUS")
                .count(),
            1
        );
        assert_eq!(report.exceptions_total, 1);
        assert_eq!(report.exceptions[0]["intent_width_mm"], 1.0);
    }

    // ─── Review A, S1: the report is never silent ──────────────────────────

    /// A session at ten units per micrometre whose one net, VBUS, holds `body`.
    fn one_net(body: &str) -> String {
        format!("(session b (routes (resolution um 10) (network_out (net VBUS {body}))))")
    }

    #[test]
    fn keywords_in_any_letter_case_are_measured() {
        // Each of these read as no track at all, and so as nothing narrow.
        for ses in [
            one_net("(wire (PATH F.Cu 2000 0 0 10000 0))"),
            one_net("(WIRE (path F.Cu 2000 0 0 10000 0))"),
            "(session b (routes (resolution um 10) (network_out (NET VBUS (wire (path F.Cu 2000 0 0 10000 0))))))".to_string(),
            "(SESSION b (ROUTES (RESOLUTION UM 10) (NETWORK_OUT (NET VBUS (WIRE (PATH F.Cu 2000 0 0 10000 0))))))".to_string(),
            one_net("(Wire (Qarc F.Cu 2000 10000 0 0 10000 0 0))"),
        ] {
            let report = intent_width_report(&manifest(), &ses).unwrap();
            assert_eq!(report.exceptions_total, 1, "{ses}");
            assert!(
                net_row(&report, "VBUS")["routed_mm"].as_f64().unwrap() >= 1.0,
                "{ses}"
            );
            assert_eq!(report.unmeasured_total, 0, "{ses}");
        }
    }

    #[test]
    fn a_polyline_path_is_measured_between_the_points_where_its_lines_meet() {
        // Four lines: x = 0, y = 0, x = 3 mm and y = 2.5 mm. They meet at
        // (0, 0), (3, 0) and (3, 2.5) mm: 5.5 mm of track at 0.2 mm.
        let ses = one_net("(wire (polyline_path F.Cu 2000 0 -100 0 100 0 0 30000 0 30000 0 30000 25000 0 25000 100 25000))");
        let report = intent_width_report(&manifest(), &ses).unwrap();
        assert_eq!(report.unmeasured_total, 0);
        assert_eq!(report.exceptions_total, 1);
        assert_eq!(report.exceptions[0]["length_mm"], 5.5);
        assert_eq!(report.exceptions[0]["width_mm"], 0.2);
        assert_eq!(
            report.exceptions[0]["ends_mm"],
            json!([[0.0, 0.0], [3.0, -2.5]])
        );
        assert_eq!(net_row(&report, "VBUS")["routed_mm"], 5.5);
        // In upper case, sloped, and joined end to end with a plain path:
        // x + y = 0, y = x, x = 3 mm and y = 6 mm meet at (0, 0), (3, 3) and
        // (3, 6) mm.
        let ses = one_net("(wire (POLYLINE_PATH F.Cu 2000 -100 100 100 -100 0 0 100 100 30000 0 30000 100 0 60000 100 60000)) (wire (path F.Cu 2000 30000 60000 40000 60000))");
        let report = intent_width_report(&manifest(), &ses).unwrap();
        assert_eq!(report.exceptions_total, 1, "one stretch");
        // 3 * sqrt(2) + 3 + 1 mm.
        assert_eq!(report.exceptions[0]["length_mm"], 8.243);
        assert_eq!(
            report.exceptions[0]["ends_mm"],
            json!([[0.0, 0.0], [4.0, -6.0]])
        );
        // Lines that are not whole, or fewer than three, are not a polyline.
        for values in ["0 0 0 10 0 0 10 0", "0 0 0 10 0 0 10 0 10 0 10", "0 0", ""] {
            let ses = one_net(&format!("(wire (polyline_path F.Cu 2000 {values}))"));
            let error = intent_width_report(&manifest(), &ses).unwrap_err();
            assert!(
                format!("{error:#}").contains("three whole lines"),
                "{values:?}: {error:#}"
            );
        }
    }

    #[test]
    fn a_wire_that_cannot_be_measured_is_named_never_skipped() {
        for (body, form, width) in [
            ("(wire (polygon F.Cu 0 0 0 10 10 0 10))", "polygon", None),
            ("(wire (rect F.Cu 0 0 10 10))", "rect", None),
            ("(wire (circle F.Cu 100))", "circle", None),
            ("(wire (POLYGON F.Cu 0 0 0 10 10 0 10))", "polygon", None),
            ("(wire (blob 1 2))", "blob", None),
            ("(wire)", "none", None),
            ("(wire (net VBUS) (type route))", "none", None),
            // The second and third lines never meet.
            (
                "(wire (polyline_path F.Cu 2000 0 0 0 10 0 0 10 0 0 5 10 5))",
                "polyline_path",
                Some(0.2),
            ),
        ] {
            let report = intent_width_report(&manifest(), &one_net(body)).unwrap();
            assert_eq!(report.unmeasured_total, 1, "{body}");
            assert_eq!(report.unmeasured.len(), 1, "{body}");
            let row = &report.unmeasured[0];
            assert_eq!(row["net"], "VBUS", "{body}");
            assert_eq!(row["form"], form, "{body}");
            assert_eq!(row["width_mm"].as_f64(), width, "{body}");
            assert!(row["reason"].as_str().unwrap().len() > 10, "{body}");
            assert_eq!(net_row(&report, "VBUS")["unmeasured_wires"], 1, "{body}");
            assert_eq!(net_row(&report, "VAUX")["unmeasured_wires"], 0, "{body}");
            assert_eq!(report.exceptions_total, 0, "{body}");
        }
        // What can be measured beside it still is.
        let ses = one_net("(wire (path F.Cu 2000 0 0 10000 0) (polygon F.Cu 0 0 0 10 10 0 10)) (wire (path F.Cu 10000 0 0 0 10000))");
        let report = intent_width_report(&manifest(), &ses).unwrap();
        assert_eq!(report.exceptions_total, 1);
        assert_eq!(report.unmeasured_total, 1);
        assert_eq!(report.unmeasured[0]["form"], "polygon");
        let row = net_row(&report, "VBUS");
        assert_eq!(row["routed_mm"], 2.0);
        assert_eq!(row["unmeasured_wires"], 1);
        // What a wire may carry beside its shape is not a shape.
        let ses = one_net("(wire (path F.Cu 10000 0 0 10000 0) (net VBUS) (type route) (clearance_class default))");
        let report = intent_width_report(&manifest(), &ses).unwrap();
        assert_eq!(report.unmeasured_total, 0);
        // A wire with no shape is named wherever it stands among the net's
        // wires: after a measured one, and after another unmeasured one.
        let ses = one_net("(wire (path F.Cu 10000 0 0 10000 0)) (wire) (wire (polygon F.Cu 0 0 0 10 10 0 10)) (wire (net VBUS)) (wire (path F.Cu 10000 0 0 0 10000)) (wire)");
        let report = intent_width_report(&manifest(), &ses).unwrap();
        let forms: Vec<&str> = report
            .unmeasured
            .iter()
            .map(|row| row["form"].as_str().unwrap())
            .collect();
        assert_eq!(forms, ["none", "polygon", "none", "none"]);
        assert_eq!(net_row(&report, "VBUS")["routed_mm"], 2.0);
        // A net that is not judged is not this report's.
        let other = "(session b (routes (resolution um 10) (network_out (net SIG (wire (polygon F.Cu 0 0 0 10 10 0 10))))))";
        let report = intent_width_report(&manifest(), other).unwrap();
        assert_eq!(report.unmeasured_total, 0);
        assert!(report.unmeasured.is_empty());
    }

    #[test]
    fn the_unmeasured_list_is_capped_and_the_count_is_not() {
        let body = "(wire (polygon F.Cu 0 0 0 10 10 0 10))".repeat(MAX_WIDTH_EXCEPTIONS + 5);
        let report = intent_width_report(&manifest(), &one_net(&body)).unwrap();
        assert_eq!(report.unmeasured_total, MAX_WIDTH_EXCEPTIONS + 5);
        assert_eq!(report.unmeasured.len(), MAX_WIDTH_EXCEPTIONS);
        assert_eq!(
            net_row(&report, "VBUS")["unmeasured_wires"],
            MAX_WIDTH_EXCEPTIONS + 5
        );
        // A shape's name comes back bounded.
        let long = format!("(wire ({} 1 2))", "x".repeat(500));
        let report = intent_width_report(&manifest(), &one_net(&long)).unwrap();
        assert_eq!(report.unmeasured[0]["form"].as_str().unwrap().len(), 40);
    }

    /// Review A, N1: the half micrometre is a rule, not rounding noise. For
    /// every whole-micrometre intent width from 0.1 to 3.0 mm, a wire exactly
    /// half a micrometre below it is at width, and one a tenth of a
    /// micrometre narrower is not.
    #[test]
    fn the_half_micrometre_holds_at_every_intent_width() {
        for intent_um in 100..=3000i64 {
            let manifest = json!({ "intent_widths": [
                { "net": "VBUS", "from_mm": 0.1, "to_mm": intent_um as f64 / 1000.0, "status": "widened" }
            ] });
            for (units, listed) in [(intent_um * 10 - 5, 0), (intent_um * 10 - 6, 1)] {
                let ses = one_net(&format!("(wire (path F.Cu {units} 0 0 10000 0))"));
                let report = intent_width_report(&manifest, &ses).unwrap();
                assert_eq!(
                    report.exceptions_total, listed,
                    "intent {intent_um} um, a wire of {units} tenths of a micrometre"
                );
            }
        }
    }

    /// Review A, N3: a number too large to work with is an error (the status
    /// is then `unverified`), never a `null` beside a "complete".
    #[test]
    fn numbers_too_large_to_measure_are_an_error() {
        for body in [
            "(wire (path F.Cu 2000 -1e308 0 1e308 0))",
            "(wire (qarc F.Cu 2000 -1e308 0 1e308 0 0 1e308))",
            "(wire (polyline_path F.Cu 2000 0 0 0 1 -1e308 0 1e308 1 1e308 0 1e308 1 -1e308 1e308 1e308 -1e308))",
        ] {
            let error = intent_width_report(&manifest(), &one_net(body)).unwrap_err();
            assert!(
                format!("{error:#}").contains("too large to measure"),
                "{body}: {error:#}"
            );
        }
        // A resolution that nothing can be scaled by.
        let tiny = "(session b (routes (resolution um 1e-320) (network_out (net VBUS (wire (path F.Cu 2000 0 0 10 10))))))";
        let error = intent_width_report(&manifest(), tiny).unwrap_err();
        assert!(format!("{error:#}").contains("resolution"), "{error:#}");
        // A narrow path of no length is still listed: naming too much is the
        // safe side.
        let dot = one_net("(wire (path F.Cu 2000 500 500 500 500))");
        let report = intent_width_report(&manifest(), &dot).unwrap();
        assert_eq!(report.exceptions_total, 1);
        assert_eq!(report.exceptions[0]["length_mm"], 0.0);
    }

    #[test]
    fn a_session_in_another_unit_is_measured_in_millimetres() {
        // 1000 units per millimetre.
        let mm = "(session b (routes (resolution mm 1000) (network_out (net VBUS (wire (path F.Cu 200 0 0 2500 0))))))";
        let report = intent_width_report(&manifest(), mm).unwrap();
        assert_eq!(report.exceptions[0]["width_mm"], 0.2);
        assert_eq!(report.exceptions[0]["length_mm"], 2.5);
        assert_eq!(
            report.exceptions[0]["ends_mm"],
            json!([[0.0, 0.0], [2.5, 0.0]])
        );
        // Ten units per mil: 8 mil wide, 100 mil long.
        let mil = "(session b (routes (resolution MIL 10) (network_out (net VBUS (wire (path F.Cu 80 0 0 1000 0))))))";
        let report = intent_width_report(&manifest(), mil).unwrap();
        assert_eq!(report.exceptions[0]["width_mm"], 0.203);
        assert_eq!(report.exceptions[0]["length_mm"], 2.54);
    }

    /// Review A, M1: a cut or odd session is an error, never a panic.
    #[test]
    fn a_cut_or_odd_session_is_an_error_never_a_panic() {
        let whole = "(session \u{dc}bung (routes (resolution um 10) (network_out (net VBUS (wire (path \u{d6}.Cu 2000 0 0 10000 0)) (wire (polyline_path F.Cu 2000 0 -100 0 100 0 0 30000 0 30000 -100 30000 100)) (wire (qarc F.Cu 2000 10000 0 0 10000 0 0)) (wire (polygon F.Cu 0 0 0 10 10 0 10)) (via v 0 0)) (net \"\u{3a9} \u{5d0}\" (wire (path F.Cu 2000 0 0 10 10))))))";
        assert!(intent_width_report(&manifest(), whole).is_ok());
        for end in (0..whole.len()).filter(|end| whole.is_char_boundary(*end)) {
            assert!(
                intent_width_report(&manifest(), &whole[..end]).is_err(),
                "{end}"
            );
        }
        for odd in [
            "",
            "(",
            ")",
            "\"",
            "\u{e9}",
            "(session)",
            "(session b (routes))",
            "(session b (routes (resolution)))",
            "(session b (routes (resolution um)))",
            "(session b (routes (resolution um 10) (network_out (net))))",
            "(session b (routes (resolution um 10) (network_out (net VBUS (wire (path))))))",
            "(session b (routes (resolution um 10) (network_out (net VBUS (wire (path F.Cu))))))",
            "(session b (routes (resolution um 10) (network_out (net VBUS (wire (qarc F.Cu 1))))))",
            "(session b (routes (resolution um 10) (network_out (net VBUS (wire (polyline_path (x) 1))))))",
            "(session b (routes (resolution um 10) (network_out (net VBUS (wire ((path) 1))))))",
        ] {
            let _ = intent_width_report(&manifest(), odd);
        }
        // And a manifest of any shape.
        for manifest in [
            json!(null),
            json!([]),
            json!({ "intent_widths": 3 }),
            json!({ "intent_widths": [3, null, [], { "net": 3 }, { "net": "VBUS" },
                { "net": "VBUS", "status": "widened" },
                { "net": "VBUS", "status": "widened", "to_mm": "wide" },
                { "net": "\u{e9}", "status": "widened", "to_mm": 1e308 }],
                "layers": [3, { "dsn_name": 1 }, { "dsn_name": "F.Cu" }] }),
        ] {
            let _ = has_judged_net(&manifest);
            let _ = intent_width_report(&manifest, whole);
        }
    }
}
