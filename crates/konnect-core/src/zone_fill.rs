//! Row ZR: what the judge records about the pours of one judged run (stage ZR0), what
//! Konnect tells fastroute about each pour's fill and what it reads back (stage ZR1)
//! (docs/ZONE_ROUTING_DESIGN.md, "Stages" and "What Konnect sends and reads"; the exact
//! formats are fixed by the ZR1 contract between the two).
//!
//! Read only: nothing here changes a board, a session or a routing. ZR1 sends the
//! engine a zones file and compares its prediction with KiCad's refill; no gate and no
//! rank key reads either.

use crate::tools::cli::DrcReport;
use anyhow::{Context, Result};
use konnect_sexp::SexpNode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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
    /// Pads the engine's model put in its net's main group and KiCad leaves outside it on
    /// the first refilled board (ZR1, [`compare_pads`]). `None` when the run's pass sent
    /// no zones file or its report has no usable `planes`, for a Tune candidate's board
    /// (the model is of the routing pass's board), and when the two sides cannot be
    /// compared.
    pub false_connected: Option<usize>,
    /// Pads the engine's model left open and KiCad connects (ZR1), `None` as above.
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

// ─── Stage ZR1: the zones file ───────────────────────────────────────────────

/// The zones file's name in the call's folder.
pub(crate) const ZONES_FILE: &str = "zones.txt";

/// The model's cell, mm (Z2).
pub(crate) const CELL_MM: f64 = 0.05;

/// KiCad 10's defaults for a value a `(zone …)` leaves out (`zones.h`, `ZONE_SETTINGS`).
const DEFAULT_ZONE_CLEARANCE_MM: f64 = 0.5;
const DEFAULT_MIN_WIDTH_MM: f64 = 0.25;
const DEFAULT_THERMAL_GAP_MM: f64 = 0.5;
const DEFAULT_THERMAL_BRIDGE_MM: f64 = 0.5;
const DEFAULT_ISLAND_AREA_MM2: f64 = 10.0;
/// KiCad 10's defaults for the project rules the file's header carries
/// (`board_design_settings.h`).
const DEFAULT_MIN_SPOKES: u32 = 2;
const DEFAULT_EDGE_CLEARANCE_MM: f64 = 0.5;
const DEFAULT_HOLE_CLEARANCE_MM: f64 = 0.25;
/// `TEARDROP_PARAMETERS::m_TdMaxLen`'s default, mm.
const DEFAULT_TEARDROP_MAX_LENGTH_MM: f64 = 1.0;
/// The last board file version whose custom pads with a round anchor default to
/// 90-degree spokes (KiCad 6.0; `pcb_io_kicad_sexpr_parser.cpp`).
const LAST_V6_FILE: u64 = 20211014;

/// Custom-rule constraints that can change how a zone fills or what it connects
/// (KiCad's DRC rule language): any rule that sets one makes every pour legacy (P4).
const ZONE_RULE_CONSTRAINTS: &[&str] = &[
    "clearance",
    "physical_clearance",
    "hole_clearance",
    "physical_hole_clearance",
    "edge_clearance",
    "zone_connection",
    "thermal_relief_gap",
    "thermal_spoke_width",
    "min_resolved_spokes",
    "disallow",
];

/// How a pour joins the pads of its own net (`connect_pads`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PadConnect {
    Solid,
    Thermal,
    /// Thermal for a pad with a hole, solid for the rest (`thru_hole_only`). Resolved per pad
    /// before the file is written: the engine never sees it.
    ThermalThroughHole,
    None,
}

impl PadConnect {
    /// The word of the zones file. A zone of `thru_hole_only` is written `thermal`: its
    /// pads without a plated hole resolve to `solid` and get a `pad` line saying so.
    fn file_word(self) -> &'static str {
        match self {
            PadConnect::Solid => "solid",
            PadConnect::Thermal | PadConnect::ThermalThroughHole => "thermal",
            PadConnect::None => "none",
        }
    }

    /// KiCad's `ZONE_CONNECTION` as a file writes it in `zone_connect`: `-1` inherits.
    fn from_kicad(value: i64) -> Option<Option<PadConnect>> {
        match value {
            -1 => Some(None),
            0 => Some(Some(PadConnect::None)),
            1 => Some(Some(PadConnect::Thermal)),
            2 => Some(Some(PadConnect::Solid)),
            3 => Some(Some(PadConnect::ThermalThroughHole)),
            _ => None,
        }
    }
}

/// What KiCad does with a fill piece that touches nothing of its net.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum IslandRule {
    Remove,
    Keep,
    RemoveBelow { area_mm2: f64 },
}

/// One pour's fill settings, as its `(zone …)` gives them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct FillSettings {
    pub clearance_mm: f64,
    pub min_width_mm: f64,
    pub connect: PadConnect,
    pub thermal_gap_mm: f64,
    pub thermal_bridge_mm: f64,
    pub priority: u32,
    pub islands: IslandRule,
}

/// A value of a list node (`(name value)`) as a finite number of 0 or more, or why not.
fn non_negative(node: &SexpNode, tag: &str) -> std::result::Result<Option<f64>, String> {
    let Some(child) = node.find(tag) else {
        return Ok(None);
    };
    let text = child.get(1).and_then(SexpNode::as_str).unwrap_or("");
    match text.parse::<f64>() {
        Ok(value) if value.is_finite() && value >= 0.0 => Ok(Some(value)),
        _ => Err(format!(
            "its {tag} '{text}' is not a finite number of 0 or more"
        )),
    }
}

/// Reads one `(zone …)`: `connect_pads` and its `clearance`, `min_thickness`, `priority`,
/// and from `fill`: `thermal_gap`, `thermal_bridge_width`, `island_removal_mode`,
/// `island_area_min`. A value KiCad leaves out is KiCad 10's default for it (clearance
/// 0.5, minimum width 0.25, thermal gap and bridge 0.5, priority 0, thermal pads, islands
/// removed, an island area of 10 mm²).
///
/// `Err(reason)` for a zone the model does not cover (P4): a hatch fill, a corner smoothing
/// (`smoothing` chamfer or fillet with a radius above 0), a stroked fill
/// (`filled_areas_thickness yes`, files before KiCad 6), a value that is not a finite
/// number of 0 or more, an island mode or a connection KiCad does not know, a minimum
/// width of 0.
pub(crate) fn fill_settings(zone: &SexpNode) -> std::result::Result<FillSettings, String> {
    let fill = zone.find("fill");
    if let Some(fill) = fill {
        if fill.find_str("mode") == Some("hatch") {
            return Err("a hatch fill".to_string());
        }
        let smoothing = fill.find_str("smoothing").unwrap_or("none");
        if !matches!(smoothing, "none" | "chamfer" | "fillet") {
            return Err(format!("its corner smoothing '{smoothing}' cannot be read"));
        }
        let radius = non_negative(fill, "radius")?.unwrap_or(0.0);
        if smoothing != "none" && radius > 0.0 {
            let shape = if smoothing == "fillet" {
                "rounded"
            } else {
                "chamfered"
            };
            return Err(format!(
                "{shape} corners (smoothing {smoothing}, radius {radius} mm)"
            ));
        }
    }
    if zone.find_str("filled_areas_thickness") == Some("yes") {
        return Err("a stroked fill (filled_areas_thickness yes, from before KiCad 6)".to_string());
    }
    let mut connect = PadConnect::Thermal;
    let mut clearance_mm = DEFAULT_ZONE_CLEARANCE_MM;
    if let Some(node) = zone.find("connect_pads") {
        for child in node.children().unwrap_or(&[]).iter().skip(1) {
            match child {
                SexpNode::List(_) if child.head() == Some("clearance") => {
                    clearance_mm = non_negative(node, "clearance")?.unwrap_or(clearance_mm);
                }
                _ => match child.as_str() {
                    Some("yes") => connect = PadConnect::Solid,
                    Some("no") => connect = PadConnect::None,
                    Some("thru_hole_only") => connect = PadConnect::ThermalThroughHole,
                    other => {
                        return Err(format!(
                            "its connect_pads '{}' cannot be read",
                            other.unwrap_or("(list)")
                        ))
                    }
                },
            }
        }
    }
    let min_width_mm = non_negative(zone, "min_thickness")?.unwrap_or(DEFAULT_MIN_WIDTH_MM);
    if min_width_mm <= 0.0 {
        return Err("a minimum width of 0".to_string());
    }
    let priority = match zone.find("priority") {
        None => 0,
        Some(node) => {
            let text = node.get(1).and_then(SexpNode::as_str).unwrap_or("");
            text.parse::<u32>()
                .map_err(|_| format!("its priority '{text}' is not a whole number of 0 or more"))?
        }
    };
    let (mut thermal_gap_mm, mut thermal_bridge_mm) =
        (DEFAULT_THERMAL_GAP_MM, DEFAULT_THERMAL_BRIDGE_MM);
    let mut islands = IslandRule::Remove;
    if let Some(fill) = fill {
        thermal_gap_mm = non_negative(fill, "thermal_gap")?.unwrap_or(thermal_gap_mm);
        thermal_bridge_mm =
            non_negative(fill, "thermal_bridge_width")?.unwrap_or(thermal_bridge_mm);
        if let Some(node) = fill.find("island_removal_mode") {
            let text = node.get(1).and_then(SexpNode::as_str).unwrap_or("");
            islands = match text {
                "0" => IslandRule::Remove,
                "1" => IslandRule::Keep,
                "2" => IslandRule::RemoveBelow {
                    area_mm2: non_negative(fill, "island_area_min")?
                        .unwrap_or(DEFAULT_ISLAND_AREA_MM2),
                },
                _ => return Err(format!("its island_removal_mode '{text}' cannot be read")),
            };
        }
    }
    Ok(FillSettings {
        clearance_mm,
        min_width_mm,
        connect,
        thermal_gap_mm,
        thermal_bridge_mm,
        priority,
        islands,
    })
}

/// A pour the engine models.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ZoneRow {
    /// Its place among the DSN's `(plane …)` entries.
    pub plane_index: usize,
    pub net: String,
    /// The KiCad layer name.
    pub layer: String,
    pub uuid: Option<String>,
    pub settings: FillSettings,
}

/// A pour sent as today (or left out of the DSN), with the reason.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct LegacyZone {
    pub net: String,
    pub layer: String,
    pub uuid: Option<String>,
    pub reason: String,
}

/// A pad of a pour net, with its resolved values (pad, then footprint, then zone).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct PadRow {
    pub component: String,
    pub pin: String,
    /// Never `ThermalThroughHole`: resolved by whether the pad has a hole.
    pub connect: PadConnect,
    pub thermal_gap_mm: f64,
    pub thermal_bridge_mm: f64,
    /// The pad's own spoke angle, else KiCad's default for its shape (45 for a round pad
    /// and a custom pad with a round anchor, 90 for the rest), relative to the pad's own
    /// orientation as KiCad's filler turns it, and taken modulo 90 (four spokes repeat
    /// every quarter turn), so that the default of a rectangular pad is 0.
    pub angle_deg: f64,
}

/// A rule area that keeps pours out, on one copper layer, in DSN micrometres.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct KeepoutRow {
    /// The DSN's layer name.
    pub layer: String,
    pub corners: Vec<(i64, i64)>,
}

/// The board's pads by `(footprint uuid, pad uuid)`, named as the DSN names them
/// (`<component>-<pin>`): how the judge names the pads KiCad's refill leaves open.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PadNames {
    by_uuid: BTreeMap<(String, String), String>,
    /// Keys two pads share: never named.
    ambiguous: BTreeSet<(String, String)>,
}

impl PadNames {
    fn from_pins(pins: &[crate::specctra::PadPin]) -> Self {
        let mut names = PadNames::default();
        for pin in pins {
            let Some(pad_uuid) = &pin.pad_uuid else {
                continue;
            };
            let key = (pin.footprint_uuid.clone(), pad_uuid.clone());
            if names.ambiguous.contains(&key) {
                continue;
            }
            if names.by_uuid.insert(key.clone(), pin.dsn_name()).is_some() {
                names.by_uuid.remove(&key);
                names.ambiguous.insert(key);
            }
        }
        names
    }

    /// The pads of the saved board text, named as the export named them (the tests'
    /// way in; a call takes them from its plan, [`ZonePlan::names`]).
    #[cfg(test)]
    pub(crate) fn from_board(board_text: &str) -> Result<Self> {
        let tree = konnect_sexp::parse_sexp(board_text).context("the board does not parse")?;
        Ok(Self::from_pins(&crate::specctra::pad_pins(&tree)?))
    }
}

/// What one call tells fastroute about the pours.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub(crate) struct ZonePlan {
    pub zones: Vec<ZoneRow>,
    pub legacy: Vec<LegacyZone>,
    pub pads: Vec<PadRow>,
    pub keepouts: Vec<KeepoutRow>,
    pub min_spokes: u32,
    pub edge_clearance_mm: f64,
    pub hole_clearance_mm: f64,
    pub teardrop_mm: f64,
    /// The pads named as the DSN names them, for the judge ([`kicad_open_pads`]).
    #[serde(skip)]
    pub names: PadNames,
}

impl ZonePlan {
    /// The `pours` field of the routing result: the modelled zones, the legacy ones
    /// with their reasons, the counts of `pad` and `keepout` lines, the header values,
    /// and `zones_file`: the file the routing passes of a variant with planes are sent,
    /// `null` when the plan models nothing.
    pub(crate) fn result_json(&self, zones_file: Option<&Path>) -> Value {
        json!({
            "zones": self.zones.iter().map(|zone| json!({
                "plane_index": zone.plane_index,
                "net": zone.net,
                "layer": zone.layer,
                "uuid": zone.uuid,
                "settings": zone.settings,
            })).collect::<Vec<_>>(),
            "legacy": self.legacy,
            "pad_lines": self.pads.len(),
            "keepouts": self.keepouts.len(),
            "min_spokes": self.min_spokes,
            "edge_clearance_mm": self.edge_clearance_mm,
            "hole_clearance_mm": self.hole_clearance_mm,
            "teardrop_mm": self.teardrop_mm,
            "zones_file": zones_file.map(|path| path.display().to_string()),
        })
    }
}

/// The DSN name of each KiCad layer, from the manifest's `layers` rows; a layer the
/// manifest does not list keeps its KiCad name.
fn dsn_layer_names(manifest: Option<&Value>) -> BTreeMap<String, String> {
    manifest
        .and_then(|manifest| manifest["layers"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|row| {
            Some((
                row["kicad_name"].as_str()?.to_string(),
                row["dsn_name"].as_str()?.to_string(),
            ))
        })
        .collect()
}

/// The copper layers a pad or zone names (`*.Cu` every one, `F&B.Cu` the two outer
/// ones), among `copper`.
fn named_copper_layers(node: &SexpNode, copper: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for tag in ["layer", "layers"] {
        if let Some(list) = node.find(tag) {
            for name in list.children().unwrap_or(&[]).iter().skip(1) {
                let Some(name) = name.as_str() else { continue };
                match name {
                    "*.Cu" => out.extend(copper.iter().cloned()),
                    "F&B.Cu" => out.extend(
                        copper
                            .iter()
                            .filter(|layer| *layer == "F.Cu" || *layer == "B.Cu")
                            .cloned(),
                    ),
                    _ if copper.iter().any(|layer| layer == name) => out.push(name.to_string()),
                    _ => {}
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The header values from the saved project (`board.design_settings.rules`), else
/// KiCad's defaults: `(min_spokes, edge_clearance_mm, hole_clearance_mm)`, or why a
/// value cannot be read.
fn project_header(project_text: Option<&str>) -> std::result::Result<(u32, f64, f64), String> {
    let Some(text) = project_text else {
        return Ok((
            DEFAULT_MIN_SPOKES,
            DEFAULT_EDGE_CLEARANCE_MM,
            DEFAULT_HOLE_CLEARANCE_MM,
        ));
    };
    let project: Value = serde_json::from_str(text)
        .map_err(|error| format!("the project file is not JSON ({error})"))?;
    let rules = &project["board"]["design_settings"]["rules"];
    let number = |key: &str, default: f64| -> std::result::Result<f64, String> {
        match rules.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(value) => value
                .as_f64()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .ok_or_else(|| {
                    format!("the project's {key} ({value}) is not a finite number of 0 or more")
                }),
        }
    };
    let spokes = match rules.get("min_resolved_spokes") {
        None | Some(Value::Null) => DEFAULT_MIN_SPOKES,
        Some(value) => value
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| {
                format!("the project's min_resolved_spokes ({value}) is not a whole number")
            })?,
    };
    Ok((
        spokes,
        number("min_copper_edge_clearance", DEFAULT_EDGE_CLEARANCE_MM)?,
        number("min_hole_clearance", DEFAULT_HOLE_CLEARANCE_MM)?,
    ))
}

/// The largest `max_length` of a pad or via with teardrops on (`(teardrops … (enabled
/// yes))`), mm; 0 without one.
fn teardrop_length(tree: &SexpNode) -> std::result::Result<f64, String> {
    let mut longest: f64 = 0.0;
    let items = tree.find_all("via").into_iter().chain(
        tree.find_all("footprint")
            .into_iter()
            .flat_map(|footprint| footprint.find_all("pad")),
    );
    for item in items {
        let Some(teardrops) = item.find("teardrops") else {
            continue;
        };
        let enabled = teardrops.find("enabled").is_some_and(|node| {
            !matches!(node.get(1).and_then(SexpNode::as_str), Some("no" | "false"))
        });
        if enabled {
            let length = non_negative(teardrops, "max_length")
                .map_err(|reason| format!("a teardrop setting cannot be read: {reason}"))?
                .unwrap_or(DEFAULT_TEARDROP_MAX_LENGTH_MM);
            longest = longest.max(length);
        }
    }
    Ok(longest)
}

/// A flag KiCad reads with `parseMaybeAbsentBool(true)`: present and not `no`/`false`.
fn flag_on(node: &SexpNode, tag: &str) -> bool {
    node.find(tag)
        .is_some_and(|flag| !matches!(flag.get(1).and_then(SexpNode::as_str), Some("no" | "false")))
}

/// "names (a, b, c and 2 more)": a few of `items`, for one board-wide reason.
fn some_of(items: &[String]) -> String {
    const SHOWN: usize = 3;
    let shown = items
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > SHOWN {
        format!("{shown} and {} more", items.len() - SHOWN)
    } else {
        shown
    }
}

/// The P4 reasons that make every pour of the board legacy: a custom rule that can
/// change a zone's clearance or connection; a pad or via with unused-layer removal, a
/// zone-layer override or a backdrill; a pad of a pour net with layer-specific
/// connection or thermal values, or a custom pad of a pour net with its own spoke shapes.
fn uncovered_board(
    tree: &SexpNode,
    pour_nets: &BTreeSet<String>,
    rules_text: Option<&str>,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if let Some(text) = rules_text {
        match crate::custom_rules::parse(text) {
            Err(error) => reasons.push(format!("the custom rules file does not read ({error:#})")),
            Ok(rules) => {
                let mut setters = Vec::new();
                for rule in &rules {
                    for constraint in rule.node.find_all("constraint") {
                        if let Some(kind) = constraint.get(1).and_then(SexpNode::as_str) {
                            if ZONE_RULE_CONSTRAINTS.contains(&kind) {
                                setters.push(format!("'{}' ({kind})", rule.name));
                            }
                        }
                    }
                }
                if !setters.is_empty() {
                    reasons.push(format!(
                        "custom rules that can change a zone's clearance or connection: {}",
                        some_of(&setters)
                    ));
                }
            }
        }
    }
    let table: BTreeMap<String, String> = tree
        .find_all("net")
        .into_iter()
        .filter_map(|net| {
            Some((
                net.get(1)?.as_str()?.to_string(),
                net.get(2)?.as_str()?.to_string(),
            ))
        })
        .collect();
    let (mut unused, mut overrides, mut drilled, mut layered, mut spokes) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let machined = |node: &SexpNode| {
        [
            "backdrill",
            "tertiary_drill",
            "front_post_machining",
            "back_post_machining",
        ]
        .iter()
        .any(|tag| node.find(tag).is_some())
    };
    for via in tree.find_all("via") {
        let label = match via.find("at") {
            Some(at) => format!(
                "the via at ({}, {})",
                at.get(1).and_then(SexpNode::as_str).unwrap_or("?"),
                at.get(2).and_then(SexpNode::as_str).unwrap_or("?")
            ),
            None => "a via".to_string(),
        };
        if flag_on(via, "remove_unused_layers") || flag_on(via, "start_end_only") {
            unused.push(label.clone());
        }
        if via.find("zone_layer_connections").is_some() {
            overrides.push(label.clone());
        }
        if machined(via) {
            drilled.push(label);
        }
    }
    for footprint in tree.find_all("footprint") {
        let reference = footprint
            .find_all("property")
            .into_iter()
            .find(|p| p.get(1).and_then(SexpNode::as_str) == Some("Reference"))
            .and_then(|p| p.get(2).and_then(SexpNode::as_str))
            .unwrap_or("?")
            .to_string();
        for pad in footprint.find_all("pad") {
            let number = pad.get(1).and_then(SexpNode::as_str).unwrap_or("");
            let label = format!("pad {reference}-{number}");
            if flag_on(pad, "remove_unused_layers") {
                unused.push(label.clone());
            }
            if pad.find("zone_layer_connections").is_some() {
                overrides.push(label.clone());
            }
            if machined(pad) {
                drilled.push(label.clone());
            }
            let on_pour_net = pad
                .find("net")
                .and_then(|net| {
                    let value = net.get(2).or(net.get(1))?.as_str()?;
                    Some(
                        table
                            .get(value)
                            .cloned()
                            .unwrap_or_else(|| value.to_string()),
                    )
                })
                .is_some_and(|net| pour_nets.contains(&net));
            if !on_pour_net {
                continue;
            }
            if pad.find("padstack").is_some_and(|stack| {
                stack.find_all("layer").iter().any(|layer| {
                    [
                        "zone_connect",
                        "thermal_gap",
                        "thermal_bridge_width",
                        "thermal_bridge_angle",
                    ]
                    .iter()
                    .any(|tag| layer.find(tag).is_some())
                })
            }) {
                layered.push(label.clone());
            }
            if pad.get(3).and_then(SexpNode::as_str) == Some("custom")
                && pad
                    .find("primitives")
                    .is_some_and(|primitives| primitives.find("gr_vector").is_some())
            {
                spokes.push(label);
            }
        }
    }
    for (items, what) in [
        (&unused, "remove copper on unused layers"),
        (&overrides, "override their zone connection per layer"),
        (&drilled, "are backdrilled or post-machined"),
        (
            &layered,
            "are pads of a pour net with layer-specific connection or thermal values",
        ),
        (
            &spokes,
            "are custom pads of a pour net with spoke shapes of their own",
        ),
    ] {
        if !items.is_empty() {
            reasons.push(format!(
                "{} pad(s) or via(s) {what}: {}",
                items.len(),
                some_of(items)
            ));
        }
    }
    reasons
}

/// One pad's values under one zone, resolved as KiCad 10 resolves them (`drc_engine.cpp`
/// `EvalRules`/`EvalZoneConnection`; `pcb_io_kicad_sexpr_parser.cpp` for the angle):
/// - connection: the pad's `zone_connect`, else its footprint's, else the zone's; then
///   `thru_hole_only` is thermal for a plated through-hole pad and solid for the rest;
/// - gap: the pad's `thermal_gap` when above 0, else the zone's (a footprint's
///   `thermal_gap` is read and dropped by KiCad, so it is not inherited);
/// - bridge: the pad's `thermal_bridge_width` (or the older `thermal_width`) when above 0,
///   raised to the zone's minimum width, else the zone's bridge width;
/// - angle: the pad's `thermal_bridge_angle`, else 45 for a `circle` pad and for a custom
///   pad with a round anchor (90 in files of KiCad 6.0 and before), else 90; modulo 90.
fn pad_values(
    pad: &SexpNode,
    footprint: &SexpNode,
    zone: &FillSettings,
    board_version: Option<u64>,
) -> std::result::Result<(PadConnect, f64, f64, f64), String> {
    let connect_of = |node: &SexpNode| -> std::result::Result<Option<PadConnect>, String> {
        match node.find("zone_connect") {
            None => Ok(None),
            Some(value) => {
                let text = value.get(1).and_then(SexpNode::as_str).unwrap_or("");
                text.parse::<i64>()
                    .ok()
                    .and_then(PadConnect::from_kicad)
                    .ok_or_else(|| format!("zone_connect '{text}' cannot be read"))
            }
        }
    };
    let mut connect = connect_of(pad)?
        .or(connect_of(footprint)?)
        .unwrap_or(zone.connect);
    if connect == PadConnect::ThermalThroughHole {
        connect = if pad.get(2).and_then(SexpNode::as_str) == Some("thru_hole") {
            PadConnect::Thermal
        } else {
            PadConnect::Solid
        };
    }
    let gap = non_negative(pad, "thermal_gap")?
        .filter(|gap| *gap > 0.0)
        .unwrap_or(zone.thermal_gap_mm);
    let own_bridge = match non_negative(pad, "thermal_bridge_width")? {
        Some(width) => Some(width),
        None => non_negative(pad, "thermal_width")?,
    };
    let bridge = own_bridge
        .filter(|width| *width > 0.0)
        .map(|width| width.max(zone.min_width_mm))
        .unwrap_or(zone.thermal_bridge_mm);
    let angle = match pad.find("thermal_bridge_angle") {
        Some(node) => {
            let text = node.get(1).and_then(SexpNode::as_str).unwrap_or("");
            text.parse::<f64>()
                .ok()
                .filter(|angle| angle.is_finite())
                .ok_or_else(|| format!("thermal_bridge_angle '{text}' cannot be read"))?
        }
        None => {
            let round_anchor = pad
                .find("options")
                .and_then(|options| options.find_str("anchor"))
                == Some("circle");
            match pad.get(3).and_then(SexpNode::as_str) {
                Some("circle") => 45.0,
                Some("custom") if round_anchor => {
                    if board_version.is_some_and(|version| version <= LAST_V6_FILE) {
                        90.0
                    } else {
                        45.0
                    }
                }
                _ => 90.0,
            }
        }
    };
    let mut angle = angle.rem_euclid(90.0);
    if (angle - angle.round()).abs() < 1e-9 {
        angle = angle.round();
    }
    if angle >= 90.0 {
        angle = 0.0;
    }
    Ok((connect, gap, bridge, angle))
}

/// The call's zone plan.
/// 1. The pours in export order (`specctra::pour_fill_rows` on the saved board), paired
///    with the manifest's `pours` rows (each row's uuid, net and layer must be the
///    pour's). A row the manifest gives `omit` has no plane, and is `legacy` with that
///    reason. The others take plane indices in order; their count must equal the DSN's
///    `(plane …)` count and each must be the net and layer of its plane, else every
///    pour is `legacy` with that reason (never a shifted index). The same when
///    `manifest` is `None`.
/// 2. Each pour: [`fill_settings`], or a `legacy` row with its reason.
/// 3. Every pour becomes `legacy`, with the reasons, when the board has (P4): a custom
///    rule that can change a zone's clearance or connection (`rules_text`, the saved
///    `.kicad_dru`); a pad or via with unused-layer removal, a zone-layer override or a
///    backdrill; a pad of a pour net with layer-specific connection or thermal values,
///    or a custom pad of a pour net with spoke shapes of its own; a project value or a
///    teardrop setting that cannot be read; pads the export cannot name, or a modelled
///    net whose pins the DSN names otherwise.
/// 4. `pads`: for each pad of a modelled pour's net, its resolved connection, gap,
///    bridge and angle ([`pad_values`]) under each modelled zone of its net on a layer it
///    has copper on, kept when one of them differs from such a zone's values at angle 0.
///    A pad whose values differ between those zones, where one line cannot say both,
///    makes its net's zones `legacy`.
/// 5. `keepouts`: each rule area with `copperpour not_allowed`, per copper layer, corners
///    as the export writes a zone (DSN micrometres, Y up), without the closing repeat.
/// 6. `min_spokes`, the edge clearance and the hole clearance from the project file
///    (`project_text`), else KiCad's defaults. `teardrop_mm`: the largest `max_length` of
///    a pad or via that has teardrops on, else 0.
///
/// An error only when the board or the DSN cannot be read, or the export would refuse
/// the board's zones: the call then sends no zones file.
pub(crate) fn zone_plan(
    board_text: &str,
    manifest: Option<&Value>,
    dsn_text: &str,
    project_text: Option<&str>,
    rules_text: Option<&str>,
) -> Result<ZonePlan> {
    let tree = konnect_sexp::parse_sexp(board_text).context("the board does not parse")?;
    let rows = crate::specctra::pour_fill_rows(&tree)?;
    let copper = crate::specctra::board_copper_layers(&tree)?;
    let layer_names = dsn_layer_names(manifest);
    let dsn_layer = |layer: &str| {
        layer_names
            .get(layer)
            .cloned()
            .unwrap_or_else(|| layer.to_string())
    };
    let mut plan = ZonePlan {
        min_spokes: DEFAULT_MIN_SPOKES,
        edge_clearance_mm: DEFAULT_EDGE_CLEARANCE_MM,
        hole_clearance_mm: DEFAULT_HOLE_CLEARANCE_MM,
        ..ZonePlan::default()
    };
    let mut board_reasons: Vec<String> = Vec::new();
    match project_header(project_text) {
        Ok((spokes, edge, hole)) => {
            plan.min_spokes = spokes;
            plan.edge_clearance_mm = edge;
            plan.hole_clearance_mm = hole;
        }
        Err(reason) => board_reasons.push(reason),
    }
    match teardrop_length(&tree) {
        Ok(length) => plan.teardrop_mm = length,
        Err(reason) => board_reasons.push(reason),
    }
    let legacy = |row: &crate::specctra::PourFillRow<'_>, reason: String| LegacyZone {
        net: row.net.clone(),
        layer: row.layer.clone(),
        uuid: row.uuid.clone(),
        reason,
    };

    // 1. Pair the pours with the manifest's rows and the DSN's planes.
    let dsn_planes = crate::specctra::dsn_plane_order(dsn_text)?;
    let mut indices: Vec<Option<usize>> = vec![None; rows.len()];
    let mut omitted: Vec<bool> = vec![false; rows.len()];
    match manifest.map(|manifest| &manifest["pours"]) {
        None => board_reasons.push(
            "there is no manifest beside the DSN, so its planes cannot be matched to the board's pours"
                .to_string(),
        ),
        Some(pours) => {
            let pours: &[Value] = pours.as_array().map_or(&[], Vec::as_slice);
            if pours.len() != rows.len() {
                board_reasons.push(format!(
                    "the manifest lists {} pour(s) and the board has {}",
                    pours.len(),
                    rows.len()
                ));
            } else {
                let mut next = 0usize;
                for (at, (row, pour)) in rows.iter().zip(pours).enumerate() {
                    let same = pour["net"].as_str() == Some(row.net.as_str())
                        && pour["layer"].as_str() == Some(row.layer.as_str())
                        && pour["uuid"].as_str() == row.uuid.as_deref();
                    if !same {
                        board_reasons.push(format!(
                            "the manifest's pour {at} is not the board's pour {at} (net '{}' on {})",
                            row.net, row.layer
                        ));
                        break;
                    }
                    match pour["policy"].as_str() {
                        Some("omit") => omitted[at] = true,
                        Some("fixed") => {
                            indices[at] = Some(next);
                            next += 1;
                        }
                        other => {
                            board_reasons.push(format!(
                                "the manifest's pour {at} has the policy {other:?}, which is neither fixed nor omit"
                            ));
                            break;
                        }
                    }
                }
                if board_reasons.is_empty() && next != dsn_planes.len() {
                    board_reasons.push(format!(
                        "the manifest gives {next} pour plane(s) and the DSN has {} (plane …) entries",
                        dsn_planes.len()
                    ));
                }
                if board_reasons.is_empty() {
                    for (row, index) in rows.iter().zip(&indices) {
                        let Some(index) = *index else { continue };
                        let plane = &dsn_planes[index];
                        if plane.net != row.net || plane.layer != dsn_layer(&row.layer) {
                            board_reasons.push(format!(
                                "the DSN's plane {index} is net '{}' on {}, the manifest's is net '{}' on {}",
                                plane.net, plane.layer, row.net, row.layer
                            ));
                            break;
                        }
                    }
                }
            }
        }
    }

    // 2. Each pour's settings.
    for (at, row) in rows.iter().enumerate() {
        if omitted[at] {
            plan.legacy.push(legacy(
                row,
                "left out of the DSN by the pours policy (omit): there is no plane to model"
                    .to_string(),
            ));
            continue;
        }
        let Some(plane_index) = indices[at] else {
            plan.legacy.push(legacy(row, String::new()));
            continue;
        };
        match fill_settings(row.zone) {
            Ok(settings) => plan.zones.push(ZoneRow {
                plane_index,
                net: row.net.clone(),
                layer: row.layer.clone(),
                uuid: row.uuid.clone(),
                settings,
            }),
            Err(reason) => plan.legacy.push(legacy(row, reason)),
        }
    }

    // 3. What makes the whole board uncovered.
    let pour_nets: BTreeSet<String> = rows.iter().map(|row| row.net.clone()).collect();
    board_reasons.extend(uncovered_board(&tree, &pour_nets, rules_text));

    // 4. The pads of the modelled nets, and the names the judge will need.
    let pins = match crate::specctra::pad_pins(&tree) {
        Ok(pins) => pins,
        Err(error) => {
            board_reasons.push(format!(
                "the board's pads cannot be named as the export names them ({error:#})"
            ));
            Vec::new()
        }
    };
    plan.names = PadNames::from_pins(&pins);
    if board_reasons.is_empty() {
        let dsn_pins = crate::specctra::dsn_net_pins(dsn_text)?;
        let modelled: BTreeSet<String> = plan.zones.iter().map(|zone| zone.net.clone()).collect();
        for net in &modelled {
            let ours: BTreeSet<String> = pins
                .iter()
                .filter(|pin| pin.net.as_deref() == Some(net.as_str()))
                .map(crate::specctra::PadPin::dsn_name)
                .collect();
            if dsn_pins.get(net) != Some(&ours) {
                board_reasons.push(format!(
                    "the DSN does not name the pins of net '{net}' as the board's pads are named"
                ));
            }
        }
        if board_reasons.is_empty() {
            let version = tree.find_str("version").and_then(|v| v.parse::<u64>().ok());
            let footprints = tree.find_all("footprint");
            let mut demoted: BTreeMap<String, String> = BTreeMap::new();
            let mut lines: Vec<PadRow> = Vec::new();
            for pin in &pins {
                let Some(net) = pin.net.as_deref().filter(|net| modelled.contains(*net)) else {
                    continue;
                };
                let Some(footprint) = footprints.get(pin.footprint_index) else {
                    continue;
                };
                let Some(pad) = footprint.find_all("pad").get(pin.pad_index).copied() else {
                    continue;
                };
                let layers = named_copper_layers(pad, &copper);
                let under: Vec<&ZoneRow> = plan
                    .zones
                    .iter()
                    .filter(|zone| zone.net == net && layers.contains(&zone.layer))
                    .collect();
                let mut values = Vec::new();
                for zone in &under {
                    match pad_values(pad, footprint, &zone.settings, version) {
                        Ok(value) => values.push((value, zone)),
                        Err(reason) => {
                            demoted
                                .entry(net.to_string())
                                .or_insert(format!("pad {}: {reason}", pin.dsn_name()));
                        }
                    }
                }
                let differs = values.iter().any(|((connect, gap, bridge, angle), zone)| {
                    let own = &zone.settings;
                    *connect != own.connect
                        || *gap != own.thermal_gap_mm
                        || *bridge != own.thermal_bridge_mm
                        || *angle != 0.0
                });
                if !differs {
                    continue;
                }
                let first = values[0].0;
                if values.iter().any(|(value, _)| *value != first) {
                    demoted.entry(net.to_string()).or_insert(format!(
                        "pad {} resolves to other thermal values under each zone of its net, which one pad line cannot say",
                        pin.dsn_name()
                    ));
                    continue;
                }
                let (connect, gap, bridge, angle) = first;
                lines.push(PadRow {
                    component: pin.component.clone(),
                    pin: pin.pin.clone(),
                    connect,
                    thermal_gap_mm: gap,
                    thermal_bridge_mm: bridge,
                    angle_deg: angle,
                });
            }
            if !demoted.is_empty() {
                let (kept, out): (Vec<ZoneRow>, Vec<ZoneRow>) = std::mem::take(&mut plan.zones)
                    .into_iter()
                    .partition(|zone| !demoted.contains_key(&zone.net));
                plan.zones = kept;
                for zone in out {
                    plan.legacy.push(LegacyZone {
                        reason: demoted[&zone.net].clone(),
                        net: zone.net,
                        layer: zone.layer,
                        uuid: zone.uuid,
                    });
                }
                lines.retain(|line| {
                    pins.iter().any(|pin| {
                        pin.component == line.component
                            && pin.pin == line.pin
                            && pin
                                .net
                                .as_ref()
                                .is_some_and(|net| !demoted.contains_key(net))
                    })
                });
            }
            lines.sort_by(|a, b| (&a.component, &a.pin).cmp(&(&b.component, &b.pin)));
            plan.pads = lines;
        }
    }

    // 5. The rule areas that keep pours out.
    for zone in tree.find_all("zone") {
        let Some(keepout) = zone.find("keepout") else {
            continue;
        };
        if keepout.find_str("copperpour") != Some("not_allowed") {
            continue;
        }
        let layers = named_copper_layers(zone, &copper);
        if layers.is_empty() {
            continue;
        }
        let mut corners = match crate::specctra::zone_outline_um(zone) {
            Ok(corners) => corners,
            Err(error) => {
                board_reasons.push(format!(
                    "a rule area that keeps pours out has an outline the zones file cannot carry ({error:#})"
                ));
                continue;
            }
        };
        corners.pop();
        for layer in layers {
            plan.keepouts.push(KeepoutRow {
                layer: dsn_layer(&layer),
                corners: corners.clone(),
            });
        }
    }
    // A board the model does not cover: every pour keeps today's reading.
    if !board_reasons.is_empty() {
        let reason = board_reasons.join("; ");
        for zone in std::mem::take(&mut plan.zones) {
            plan.legacy.push(LegacyZone {
                net: zone.net,
                layer: zone.layer,
                uuid: zone.uuid,
                reason: reason.clone(),
            });
        }
        for row in &mut plan.legacy {
            if row.reason.is_empty() {
                row.reason = reason.clone();
            }
        }
        plan.pads.clear();
    }

    Ok(plan)
}

/// A number with the shortest text that reads back to the same value (Rust's `{}`).
fn file_number(value: f64) -> String {
    format!("{value}")
}

/// A name as the zones file writes it: in double quotes, with `\"` and `\\`, when it holds
/// white space, `=`, `"`, `\` or `#` or is empty; as it is otherwise.
fn file_name(name: &str) -> String {
    if name.is_empty()
        || name
            .chars()
            .any(|c| c.is_whitespace() || "=\"\\#".contains(c))
    {
        let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        name.to_string()
    }
}

/// The zones file of a plan (the ZR1 contract, §1), or `None` when the plan models no
/// pour: the call then sends no `--x.zones`. Zones in plane order, pads by component and
/// pin, keepouts in board order. Numbers are written with the shortest text that reads
/// back to the same value.
pub(crate) fn file_text(plan: &ZonePlan) -> Option<String> {
    if plan.zones.is_empty() {
        return None;
    }
    let mut text = String::from("zones 1\n");
    text.push_str(&format!("cell_mm {}\n", file_number(CELL_MM)));
    text.push_str(&format!("min_spokes {}\n", plan.min_spokes));
    text.push_str(&format!(
        "edge_clearance_mm {}\n",
        file_number(plan.edge_clearance_mm)
    ));
    text.push_str(&format!(
        "hole_clearance_mm {}\n",
        file_number(plan.hole_clearance_mm)
    ));
    text.push_str(&format!("teardrop_mm {}\n", file_number(plan.teardrop_mm)));
    let mut zones: Vec<&ZoneRow> = plan.zones.iter().collect();
    zones.sort_by_key(|zone| zone.plane_index);
    for zone in zones {
        let s = &zone.settings;
        let islands = match s.islands {
            IslandRule::Remove => "remove".to_string(),
            IslandRule::Keep => "keep".to_string(),
            IslandRule::RemoveBelow { area_mm2 } => format!("below:{}", file_number(area_mm2)),
        };
        text.push_str(&format!(
            "zone {} clearance_mm={} min_width_mm={} connect={} gap_mm={} bridge_mm={} priority={} islands={islands}\n",
            zone.plane_index,
            file_number(s.clearance_mm),
            file_number(s.min_width_mm),
            s.connect.file_word(),
            file_number(s.thermal_gap_mm),
            file_number(s.thermal_bridge_mm),
            s.priority,
        ));
    }
    for pad in &plan.pads {
        text.push_str(&format!(
            "pad {} {} connect={} gap_mm={} bridge_mm={} angle_deg={}\n",
            file_name(&pad.component),
            file_name(&pad.pin),
            pad.connect.file_word(),
            file_number(pad.thermal_gap_mm),
            file_number(pad.thermal_bridge_mm),
            file_number(pad.angle_deg),
        ));
    }
    for keepout in &plan.keepouts {
        text.push_str(&format!("keepout {}", file_name(&keepout.layer)));
        for (x, y) in &keepout.corners {
            text.push_str(&format!(" {x} {y}"));
        }
        text.push('\n');
    }
    Some(text)
}

// ─── Stage ZR1: what the engine said, and the comparison ─────────────────────

/// What the engine's model said, from the report's `planes` object (the ZR1 contract,
/// §2).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct EnginePlanes {
    /// The engine's own error: the model could not be drawn. Its lists are then empty.
    pub error: Option<String>,
    /// The modelled nets.
    pub nets: BTreeSet<String>,
    /// Pins of the modelled nets in their net's main group, as `<component>-<pin>`.
    pub connected_pads: BTreeSet<String>,
    /// Pads the model leaves open, as `<component>-<pin>`.
    pub open_pads: BTreeSet<String>,
    pub starved_pads: BTreeSet<String>,
    pub closing_routed: usize,
    pub pieces: usize,
    /// Model builds made in the run.
    pub redraws: usize,
    /// Seconds one redraw of the snapshot took, the longest of the run.
    pub redraw_s: f64,
}

/// [`EnginePlanes`] of a report's `planes` object (the object `fastroute::Held::planes`
/// keeps), or why it does not read: version 1 only; the five lists of strings; the
/// counts whole; `connected_pads` and `open_pads` disjoint.
pub(crate) fn planes_object(object: &Value) -> Result<EnginePlanes> {
    anyhow::ensure!(
        object["version"].as_u64() == Some(1),
        "its version is {}, not 1",
        object["version"]
    );
    let error = match &object["error"] {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => anyhow::bail!("its error is {other}, neither null nor a string"),
    };
    let list = |key: &str| -> Result<BTreeSet<String>> {
        object[key]
            .as_array()
            .with_context(|| format!("its {key} is not a list"))?
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .with_context(|| format!("its {key} holds {item}, not a string"))
            })
            .collect()
    };
    let count = |key: &str| -> Result<usize> {
        object[key]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .with_context(|| format!("its {key} is not a whole number"))
    };
    let planes = EnginePlanes {
        error,
        nets: list("nets")?,
        connected_pads: list("connected_pads")?,
        open_pads: list("open_pads")?,
        starved_pads: list("starved_pads")?,
        closing_routed: count("closing_routed")?,
        pieces: count("pieces")?,
        redraws: count("redraws")?,
        redraw_s: object["redraw_s"]
            .as_f64()
            .filter(|s| s.is_finite() && *s >= 0.0)
            .context("its redraw_s is not a number of 0 or more")?,
    };
    if let Some(pad) = planes.connected_pads.intersection(&planes.open_pads).next() {
        anyhow::bail!("the pad {pad} is both connected and open");
    }
    Ok(planes)
}

/// `None` when the report has no `planes` section (an older build, or a run with no
/// file) or the section does not read ([`planes_object`]). The judge reads the object a
/// run kept (`fastroute::Held::planes`) with [`planes_object`] directly, so that an
/// object that does not read is recorded, not dropped; this whole-report reader is the
/// tests' check of the contract's §2.
#[cfg(test)]
pub(crate) fn engine_planes(report: &Value) -> Option<EnginePlanes> {
    report
        .get("planes")
        .filter(|object| !object.is_null())
        .and_then(|object| planes_object(object).ok())
}

/// The engine's `planes` object as the search records it (`engine_planes` in a judged
/// run's `run.json` and in `search.winner`): the object as fastroute wrote it, with
/// `connected_pads` replaced by its count (`connected_pad_count`). With `lists` false the
/// open and starved pad lists are replaced by their counts too (`open_pad_count`,
/// `starved_pad_count`), for the result.
pub(crate) fn engine_planes_record(object: &Value, lists: bool) -> Value {
    let mut record = object.clone();
    if let Some(map) = record.as_object_mut() {
        let mut counted = vec!["connected_pads"];
        if !lists {
            counted.extend(["open_pads", "starved_pads"]);
        }
        for key in counted {
            if let Some(items) = map.remove(key) {
                let name = format!("{}_count", key.trim_end_matches('s'));
                map.insert(name, json!(items.as_array().map_or(0, Vec::len)));
            }
        }
    }
    record
}

/// KiCad's side of the comparison (Z13): the pads of each net of `nets` outside the
/// net's main copper group on the refilled board `refilled_board_text` (saved at
/// `refilled_board`), by the layout audit's own grouping over the board's saved fills
/// (`tools::pads_off_main_group`), named as the DSN names them through `names` (the
/// saved board's pads by footprint and pad UUID). A pad with no copper is no DSN pin and
/// is left out. An error when a pad cannot be named (no UUID, a UUID two pads share, a
/// pad the export did not write) or the audit cannot group the board.
pub(crate) fn kicad_open_pads(
    refilled_board: &Path,
    refilled_board_text: &str,
    names: &PadNames,
    nets: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    let mut open = BTreeSet::new();
    for pad in crate::tools::pads_off_main_group(refilled_board, refilled_board_text, nets)? {
        if !pad.has_copper {
            continue;
        }
        let (Some(footprint), Some(uuid)) = (&pad.footprint_uuid, &pad.pad_uuid) else {
            anyhow::bail!(
                "pad {} of net '{}' has no UUID, so it cannot be named as the DSN names it",
                pad.label,
                pad.net
            );
        };
        let key = (footprint.clone(), uuid.clone());
        match names.by_uuid.get(&key) {
            Some(name) => {
                open.insert(name.clone());
            }
            None if names.ambiguous.contains(&key) => anyhow::bail!(
                "pad {} of net '{}' shares its UUID with another pad",
                pad.label,
                pad.net
            ),
            None => anyhow::bail!(
                "pad {} of net '{}' is not a pad the export wrote",
                pad.label,
                pad.net
            ),
        }
    }
    Ok(open)
}

/// [`kicad_open_pads`] of the board saved at `board`, read and grouped off the async
/// threads. `names` is the call's ([`ZonePlan::names`]); `None` is an error.
pub(crate) async fn kicad_open_record(
    board: &Path,
    names: Option<std::sync::Arc<PadNames>>,
    nets: BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    let names = names.context("the call has no names for the board's pads")?;
    let path = board.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<BTreeSet<String>> {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("read the refilled board '{}'", path.display()))?;
        kicad_open_pads(&path, &text, &names, &nets)
    })
    .await
    .context("KiCad's open pads did not finish")?
}

/// The comparison (the ZR1 contract, §3): `false_connected` = |connected ∩ KiCad's
/// open|, `false_open` = |open − KiCad's open|. Both `None` when the engine reports an
/// error, and when KiCad reports 0 unconnected items on the modelled nets
/// (`kicad_unconnected`, the first refill's) while its open pads are not empty (the two
/// KiCad readings disagree).
pub(crate) fn compare_pads(
    engine: &EnginePlanes,
    kicad_open: &BTreeSet<String>,
    kicad_unconnected: usize,
) -> (Option<usize>, Option<usize>) {
    if engine.error.is_some() || (kicad_unconnected == 0 && !kicad_open.is_empty()) {
        return (None, None);
    }
    (
        Some(engine.connected_pads.intersection(kicad_open).count()),
        Some(engine.open_pads.difference(kicad_open).count()),
    )
}

/// Puts the comparison into the run's measures, after [`record_pours`]: `engine` is the
/// engine's `planes` read ([`planes_object`]), `kicad_open` KiCad's side, and
/// `kicad_unconnected` the first refill's unconnected items on the modelled nets.
/// Never an error: a side that could not be read leaves both counts `None` and its
/// reason in `pours_error`, beside the ZR0 measures, which stay. Nothing when the pours
/// were not measured.
pub(crate) fn record_comparison(
    measures: &mut crate::route_judge::Measures,
    engine: Result<EnginePlanes>,
    kicad_open: Option<Result<BTreeSet<String>>>,
    kicad_unconnected: usize,
) {
    let Some(pours) = measures.pours.as_mut() else {
        return;
    };
    let engine = match engine {
        Ok(engine) => engine,
        Err(error) => {
            measures.pours_error = Some(format!(
                "the engine's planes object does not read: {error:#}"
            ));
            return;
        }
    };
    if engine.error.is_some() {
        return;
    }
    match kicad_open {
        Some(Ok(open)) => {
            let (connected, open) = compare_pads(&engine, &open, kicad_unconnected);
            pours.false_connected = connected;
            pours.false_open = open;
        }
        Some(Err(error)) => {
            measures.pours_error = Some(format!(
                "KiCad's open pads could not be read for the comparison: {error:#}"
            ));
        }
        None => {
            measures.pours_error =
                Some("KiCad's open pads were not read for the comparison".to_string());
        }
    }
}

#[cfg(test)]
#[path = "zone_fill_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "zone_fill_zr1_tests.rs"]
mod zr1_tests;
