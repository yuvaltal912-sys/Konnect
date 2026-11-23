//! Row ZR, stage ZR0: what the judge records about the pours of one judged run
//! (docs/ZONE_ROUTING_DESIGN.md, "Stages" and "What Konnect sends and reads").
//!
//! Read only: nothing here changes a board, a session or a routing. The engine's side
//! of the comparison (`false_connected`, `false_open`) comes with ZR1; until then both
//! are `None`.

use crate::tools::cli::DrcReport;
use anyhow::{Context, Result};
use konnect_sexp::SexpNode;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// KiCad's DRC rule key for a thermal pad with too few spokes.
pub(crate) const STARVED_THERMAL: &str = "starved_thermal";

/// The saved fill of one pour zone on one copper layer: one area per piece. KiCad saves
/// each piece of a fill as one `filled_polygon`, its holes joined to its outline by
/// zero-width cuts, so the piece's area is the area of that one ring.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ZoneLayerFill {
    pub net: String,
    pub layer: String,
    pub zone_uuid: Option<String>,
    /// mm², largest first.
    pub pieces_mm2: Vec<f64>,
}

impl ZoneLayerFill {
    /// The largest piece's share of this zone's fill on this layer; `None` without fill.
    pub(crate) fn largest_share(&self) -> Option<f64> {
        let total: f64 = self.pieces_mm2.iter().sum();
        (total > 0.0).then(|| self.pieces_mm2[0] / total)
    }
}

/// What the judge records about the pours of one run (ZR0).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PourMeasures {
    /// KiCad's unconnected items on the pour nets after the first refill, before any
    /// stitching round.
    pub open_before_stitching: usize,
    /// The same on the last board, after the stitching rounds.
    pub open_after_stitching: usize,
    /// Fill pieces over every pour zone and copper layer of the last board.
    pub pieces: usize,
    /// The largest piece's area as a share of its zone's fill on its layer: the lowest
    /// such share over the zone layers that have fill. `None` when none has.
    pub largest_piece_share: Option<f64>,
    /// Pour zone layers with no saved fill on the last board.
    pub unfilled: usize,
    /// KiCad's `starved_thermal` violations on the last board.
    pub starved_thermals: usize,
    /// Pads the engine's model put in a main group and KiCad leaves outside (ZR1).
    pub false_connected: Option<usize>,
    /// Pads the engine's model left open and KiCad connects (ZR1).
    pub false_open: Option<usize>,
}

/// Every pour zone's saved fill on `board_text`, by zone and layer, for the zones whose
/// net is in `pour_nets`. A zone that is a keepout (a rule area) or a teardrop is no pour
/// and is left out. Zones inside footprints are not read. A zone of a pour net with no `filled_polygon` on one of its copper
/// layers gives that layer with no piece. Ordered by net, layer, then zone UUID.
pub(crate) fn pour_fills(
    board_text: &str,
    pour_nets: &BTreeSet<String>,
) -> Result<Vec<ZoneLayerFill>> {
    let tree = konnect_sexp::parse_sexp(board_text).context("the judged board does not parse")?;
    let ids = net_ids(&tree);
    let mut out = Vec::new();
    for zone in tree.find_all("zone") {
        // A rule area is written with `(keepout …)` (KiCad 10, `pcb_io_kicad_sexpr.cpp`).
        if zone.find("keepout").is_some() || crate::specctra::is_teardrop(zone) {
            continue;
        }
        let Some(net) = zone_net(zone, &ids) else {
            continue;
        };
        if !pour_nets.contains(&net) {
            continue;
        }
        let uuid = zone.find_str("uuid").map(str::to_string);
        let mut by_layer: BTreeMap<String, Vec<f64>> = zone_copper_layers(zone)
            .into_iter()
            .map(|layer| (layer, Vec::new()))
            .collect();
        for polygon in zone.find_all("filled_polygon") {
            let (Some(layer), Some(pts)) = (polygon.find_str("layer"), polygon.find("pts")) else {
                continue;
            };
            let area = ring_area_mm2(&xy_points(pts));
            if area > 0.0 {
                by_layer.entry(layer.to_string()).or_default().push(area);
            }
        }
        for (layer, mut pieces_mm2) in by_layer {
            pieces_mm2.sort_by(|a, b| b.total_cmp(a));
            out.push(ZoneLayerFill {
                net: net.clone(),
                layer,
                zone_uuid: uuid.clone(),
                pieces_mm2,
            });
        }
    }
    out.sort_by(|a, b| (&a.net, &a.layer, &a.zone_uuid).cmp(&(&b.net, &b.layer, &b.zone_uuid)));
    Ok(out)
}

/// The measures of one judged run. `first_refill` is the DRC right after the session
/// was written and refilled; `last_board_text` and `last_refill` are the last board the
/// judge wrote (after the stitching rounds) and its DRC.
pub(crate) fn pour_measures(
    first_refill: &DrcReport,
    last_board_text: &str,
    last_refill: &DrcReport,
    pour_nets: &BTreeSet<String>,
) -> Result<PourMeasures> {
    let nets: Vec<String> = pour_nets.iter().cloned().collect();
    let fills = pour_fills(last_board_text, pour_nets)?;
    Ok(PourMeasures {
        open_before_stitching: crate::route_judge::unconnected_on(first_refill, &nets),
        open_after_stitching: crate::route_judge::unconnected_on(last_refill, &nets),
        pieces: fills.iter().map(|fill| fill.pieces_mm2.len()).sum(),
        largest_piece_share: fills
            .iter()
            .filter_map(ZoneLayerFill::largest_share)
            .reduce(f64::min),
        unfilled: fills
            .iter()
            .filter(|fill| fill.pieces_mm2.is_empty())
            .count(),
        starved_thermals: last_refill
            .violations
            .iter()
            .filter(|violation| violation.rule == STARVED_THERMAL)
            .count(),
        false_connected: None,
        false_open: None,
    })
}

/// [`pour_measures`] of the judged board at `board`, read and parsed off the async
/// threads (a board with large pours is several megabytes, and up to three judges run at
/// once).
pub(crate) async fn pour_record(
    board: &Path,
    first_refill: DrcReport,
    last_refill: DrcReport,
    pour_nets: &BTreeSet<String>,
) -> Result<PourMeasures> {
    let path = board.to_path_buf();
    let nets = pour_nets.clone();
    tokio::task::spawn_blocking(move || -> Result<PourMeasures> {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("read the judged board '{}'", path.display()))?;
        pour_measures(&first_refill, &text, &last_refill, &nets)
    })
    .await
    .context("the pour measures did not finish")?
}

/// Puts a pour measure into the run's measures: the measures, or the reason they could
/// not be taken. Never an error: the pours are recorded only (Z10), so a failure here
/// must not fail the run or reject a Tune candidate.
pub(crate) fn record_pours(
    measures: &mut crate::route_judge::Measures,
    taken: Result<PourMeasures>,
) {
    match taken {
        Ok(pours) => {
            measures.pours = Some(pours);
            measures.pours_error = None;
        }
        Err(error) => {
            measures.pours = None;
            measures.pours_error = Some(format!("{error:#}"));
        }
    }
}

/// The board's net table, `(net <id> "<name>")`, for board files that name a zone's net
/// by its id.
fn net_ids(tree: &SexpNode) -> BTreeMap<String, String> {
    tree.find_all("net")
        .into_iter()
        .filter_map(|net| {
            Some((
                net.get(1)?.as_str()?.to_string(),
                net.get(2)?.as_str()?.to_string(),
            ))
        })
        .collect()
}

/// A zone's net name: `(net_name …)` of older files, KiCad 10's `(net "NAME")`, or an id
/// of the board's net table. `None` for no net (a rule area, net 0).
fn zone_net(zone: &SexpNode, ids: &BTreeMap<String, String>) -> Option<String> {
    if let Some(name) = zone.find_str("net_name") {
        return (!name.is_empty()).then(|| name.to_string());
    }
    let net = zone.find("net")?.get(1)?;
    let value = net.as_str()?;
    if let SexpNode::Atom(id) = net {
        if let Some(name) = ids.get(id) {
            return (!name.is_empty()).then(|| name.clone());
        }
        if id == "0" {
            return None;
        }
    }
    (!value.is_empty()).then(|| value.to_string())
}

/// The copper layers a zone names in `(layer …)` or `(layers …)`: `*.Cu` and `F&B.Cu`
/// stand for every copper layer that the zone's fills name, so they add nothing here.
fn zone_copper_layers(zone: &SexpNode) -> Vec<String> {
    let mut layers = Vec::new();
    for tag in ["layer", "layers"] {
        if let Some(node) = zone.find(tag) {
            for child in node.children().unwrap_or(&[]).iter().skip(1) {
                if let Some(name) = child.as_str() {
                    if name.ends_with(".Cu") && !name.contains('*') && !name.contains('&') {
                        layers.push(name.to_string());
                    }
                }
            }
        }
    }
    layers
}

fn xy_points(pts: &SexpNode) -> Vec<(f64, f64)> {
    pts.children()
        .unwrap_or(&[])
        .iter()
        .filter(|node| node.head() == Some("xy"))
        .filter_map(|node| Some((node.get_f64(1)?, node.get_f64(2)?)))
        .collect()
}

/// The area of a closed ring, mm² (the shoelace formula, either winding).
fn ring_area_mm2(points: &[(f64, f64)]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let mut twice = 0.0;
    for (i, &(x0, y0)) in points.iter().enumerate() {
        let (x1, y1) = points[(i + 1) % points.len()];
        twice += x0 * y1 - x1 * y0;
    }
    (twice / 2.0).abs()
}

#[cfg(test)]
#[path = "zone_fill_tests.rs"]
mod tests;
