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

use anyhow::{bail, Context, Result};
use konnect_sexp::SexpNode;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The most stretches a result lists; the count of all of them is reported
/// beside the list.
pub(crate) const MAX_WIDTH_EXCEPTIONS: usize = 100;

/// A wire this far below its intent width, or less, is at width: the same
/// half micrometre the importer allows below a net's floor.
const WIDTH_TOLERANCE_MM: f64 = 0.0005;

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
    length_mm: f64,
    /// Both ends in session units, for joining pieces end to end.
    ends: [(i64, i64); 2],
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

/// The pieces of one `wire` node. `resolution` is session units per
/// micrometre.
fn wire_pieces(
    wire: &SexpNode,
    resolution: f64,
    layers: &BTreeMap<&str, &str>,
) -> Result<Vec<Piece>> {
    let mm = |units: f64| units / resolution / 1000.0;
    let key = |x: f64, y: f64| (x.round() as i64, y.round() as i64);
    let mut pieces = Vec::new();
    for shape in wire.children().unwrap_or(&[]) {
        let Some(head) = shape.head() else { continue };
        if head != "path" && head != "qarc" {
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
        if head == "path" {
            if values.len() < 4 || !values.len().is_multiple_of(2) {
                bail!("a session path does not hold complete point pairs");
            }
            let points: Vec<(f64, f64)> = values.chunks_exact(2).map(|p| (p[0], p[1])).collect();
            let length: f64 = points
                .windows(2)
                .map(|pair| {
                    mm(((pair[1].0 - pair[0].0).powi(2) + (pair[1].1 - pair[0].1).powi(2)).sqrt())
                })
                .sum();
            let (first, last) = (points[0], points[points.len() - 1]);
            pieces.push(Piece {
                layer,
                width_mm,
                length_mm: length,
                ends: [key(first.0, first.1), key(last.0, last.1)],
            });
        } else {
            // (qarc layer width x1 y1 x2 y2 cx cy): a quarter arc at most.
            if values.len() != 6 {
                bail!("a session arc does not hold a start, an end and a centre");
            }
            let (start, end, centre) = (
                (values[0], values[1]),
                (values[2], values[3]),
                (values[4], values[5]),
            );
            let radius = ((start.0 - centre.0).powi(2) + (start.1 - centre.1).powi(2)).sqrt();
            let chord = ((end.0 - start.0).powi(2) + (end.1 - start.1).powi(2)).sqrt();
            let length = if radius > 0.0 {
                2.0 * radius * (chord / (2.0 * radius)).clamp(-1.0, 1.0).asin()
            } else {
                chord
            };
            pieces.push(Piece {
                layer,
                width_mm,
                length_mm: mm(length),
                ends: [key(start.0, start.1), key(end.0, end.1)],
            });
        }
    }
    Ok(pieces)
}

/// The judged nets' track in a session, by net name.
fn session_pieces(
    ses: &str,
    manifest: &Value,
    wanted: &BTreeSet<&str>,
) -> Result<(BTreeMap<String, Vec<Piece>>, f64)> {
    let root = konnect_sexp::parse_sexp(ses)
        .map_err(|_| anyhow::anyhow!("the session is not a readable S-expression"))?;
    if root.head() != Some("session") {
        bail!("the file is not a Specctra session");
    }
    let routes = root.find("routes").context("the session has no routes")?;
    let resolution_node = routes
        .find("resolution")
        .context("the session's routes have no resolution")?;
    if atom(resolution_node, 1, "resolution unit")? != "um" {
        bail!("only a session resolution in 'um' is supported");
    }
    let resolution = number(atom(resolution_node, 2, "resolution")?, "resolution")?;
    if resolution <= 0.0 {
        bail!("the session's resolution is not positive");
    }
    let layers: BTreeMap<&str, &str> = manifest["layers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|layer| Some((layer["dsn_name"].as_str()?, layer["kicad_name"].as_str()?)))
        .collect();
    let mut by_net: BTreeMap<String, Vec<Piece>> = BTreeMap::new();
    for network in routes.find_all("network_out") {
        for net in network.find_all("net") {
            let name = atom(net, 1, "net name")?;
            if !wanted.contains(name) {
                continue;
            }
            for wire in net.find_all("wire") {
                by_net
                    .entry(name.to_string())
                    .or_default()
                    .extend(wire_pieces(wire, resolution, &layers)?);
            }
        }
    }
    Ok((by_net, resolution))
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
    let (by_net, resolution) = session_pieces(ses, manifest, &wanted)?;
    let point_mm = |point: (i64, i64)| {
        // Board coordinates: the session's y axis points up, KiCad's down.
        json!([
            round3(point.0 as f64 / resolution / 1000.0),
            round3(-(point.1 as f64) / resolution / 1000.0)
        ])
    };
    let mut report = WidthReport::default();
    let mut all: Vec<(f64, Value)> = Vec::new();
    let mut seen = BTreeSet::new();
    for (net, rule) in &judged {
        if !seen.insert(net.as_str()) {
            continue; // a net named twice is judged once, by its first row
        }
        let empty = Vec::new();
        let pieces = by_net.get(net).unwrap_or(&empty);
        let routed: f64 = pieces.iter().map(|piece| piece.length_mm).sum();
        let below: Vec<&Piece> = pieces
            .iter()
            .filter(|piece| piece.width_mm < rule.intent_mm - WIDTH_TOLERANCE_MM)
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
            ("(session b (routes (resolution mil 10) (network_out)))", "'um'"),
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
}
