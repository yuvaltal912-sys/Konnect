//! Row ZR, stage ZR0b: the ground vias of the winning session that the filled pours make
//! redundant are removed, each with the wires that end at it, with KiCad's own refill and
//! DRC, the pours' fill and the judge's own measures as the test
//! (docs/ZONE_ROUTING_DESIGN.md, "ZR0b").
//!
//! Nothing here writes to the user's board or project: every board is a scratch copy
//! under the search folder's `judge` folder.

use crate::route_judge::{
    self, Baseline, Gates, JudgeContext, Judged, Key, Measures, OwnConfig, ScratchError, ToolCaller,
};
use crate::route_search::{
    child_blocks, has_head, items, root_block, Block, Routed, SearchOutcome,
};
use crate::zone_fill::ZoneLayerFill;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// The most KiCad checks one prune makes before it ends with the state accepted so far.
pub(crate) const MAX_CHECKS: usize = 400;
/// The sweeps over the vias.
pub(crate) const VIA_SWEEPS: usize = 3;
/// A fill smaller than the original's by more than this is a lost piece, mm².
pub(crate) const FILL_TOLERANCE_MM2: f64 = 0.01;
/// The tolerances of (f): a loop's length, mm, and a loop's area, mm².
pub(crate) const LOOP_TOLERANCE_MM: f64 = 0.05;
pub(crate) const LOOP_AREA_TOLERANCE_MM2: f64 = 0.1;

/// What the scanner's errors are wrapped in: they speak of "the DSN".
const SCAN: &str = "scan the session text";

/// Whether the caller wants the winner pruned (P12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PourPrune {
    Winner,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UnitKind {
    Wire,
    Via,
}

/// One removable unit: the `(wire …)` or `(via …)` blocks of one net that the importer
/// would apply as ONE item set (it applies an exact repeat once, so cutting one of two
/// would remove nothing). Vias: the same net and point (whatever the padstack). Wires:
/// the same net and the same set of segment keys, a segment key being the importer's
/// (layer, width, the two ends sorted), so a reversed path is a repeat. The importer
/// drops a repeated SEGMENT too: a wire block whose segments all lie in another block is
/// its own unit, cutting it changes no copper, and the record's wire counts can count it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Unit {
    pub kind: UnitKind,
    pub net: String,
    /// Each block's bytes in the session text, from its `(` to after its `)`, in text
    /// order; more than one for repeats.
    pub spans: Vec<(usize, usize)>,
    /// A wire's first and last path point in the session's own units (the numbers as
    /// written, parsed as `i64`); `None` for a via, for a wire that holds a `qarc`, and
    /// for a path whose numbers are not whole.
    pub ends: Option<[[i64; 2]; 2]>,
    /// A via's point, the same way; `None` for a wire.
    pub at: Option<[i64; 2]>,
    /// A wire's path length in mm (the session's resolution applied); 0 for a via.
    pub length_mm: f64,
}

/// A number of the session as a key: its value's bits, `-0` read as `0`, so that two
/// spellings of one value are one key (the importer compares values, not text).
type Bits = u64;

/// One piece of a wire as the importer keys it for its repeat check: a path segment
/// (layer, width, the two ends sorted) or a quarter arc (layer, width, start, end,
/// centre).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Piece {
    Segment {
        layer: String,
        width: Bits,
        ends: [[Bits; 2]; 2],
    },
    Arc {
        layer: String,
        width: Bits,
        points: [[Bits; 2]; 3],
    },
}

/// What makes two blocks one unit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum UnitKey {
    Via {
        net: String,
        at: [Bits; 2],
    },
    Wire {
        net: String,
        pieces: BTreeSet<Piece>,
    },
}

/// A `(net …)` block of `network_out`: its block, its name (unquoted) and its direct
/// `(wire …)` and `(via …)` children.
struct NetBlock {
    block: Block,
    name: String,
    routes: Vec<Block>,
}

/// The session's `network_out` nets, and the millimetres of one session unit.
struct Network {
    mm_per_unit: f64,
    nets: Vec<NetBlock>,
}

fn unquote(atom: &str) -> &str {
    atom.strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(atom)
}

/// The atoms (bare words and quoted strings, unquoted) of `block`, its head included.
fn atoms(text: &str, block: Block) -> Result<Vec<&str>> {
    Ok(items(text, block)
        .context(SCAN)?
        .into_iter()
        .filter(|item| !item.list)
        .map(|item| unquote(&text[item.start..item.end]))
        .collect())
}

fn first_child(text: &str, block: Block, head: &str) -> Result<Option<Block>> {
    Ok(child_blocks(text, block)
        .context(SCAN)?
        .into_iter()
        .find(|child| has_head(text, *child, head)))
}

fn number(text: &str) -> Result<f64> {
    match text.parse::<f64>() {
        Ok(value) if value.is_finite() => Ok(value),
        _ => bail!("the session has a number that is not one: '{text}'"),
    }
}

fn bits(value: f64) -> Bits {
    if value == 0.0 {
        0.0f64.to_bits()
    } else {
        value.to_bits()
    }
}

/// The millimetres of one unit of `(resolution UNIT N)`: N units per UNIT.
fn mm_per_unit(text: &str, resolution: Block) -> Result<f64> {
    let parts = atoms(text, resolution)?;
    let (Some(unit), Some(count)) = (parts.get(1), parts.get(2)) else {
        bail!("the session's (resolution ...) needs a unit and a count");
    };
    let unit_mm = match unit.to_ascii_lowercase().as_str() {
        "um" => 0.001,
        "mm" => 1.0,
        "mil" => 0.0254,
        "inch" => 25.4,
        other => bail!("the session's resolution unit '{other}' is not um, mm, mil or inch"),
    };
    let count = number(count)?;
    if count <= 0.0 {
        bail!("the session's resolution must be above 0");
    }
    Ok(unit_mm / count)
}

fn network(text: &str) -> Result<Network> {
    let root = root_block(text).context(SCAN)?;
    if !has_head(text, root, "session") {
        bail!("the session text's root block is not (session ...)");
    }
    let routes = first_child(text, root, "routes")?
        .ok_or_else(|| anyhow!("the session has no (routes ...) block"))?;
    let resolution = first_child(text, routes, "resolution")?
        .ok_or_else(|| anyhow!("the session's (routes ...) has no (resolution ...)"))?;
    let mm_per_unit = mm_per_unit(text, resolution)?;
    let network = first_child(text, routes, "network_out")?
        .ok_or_else(|| anyhow!("the session's (routes ...) has no (network_out ...)"))?;
    let mut nets = Vec::new();
    for block in child_blocks(text, network).context(SCAN)? {
        if !has_head(text, block, "net") {
            continue;
        }
        let name = atoms(text, block)?
            .get(1)
            .map(|name| name.to_string())
            .ok_or_else(|| anyhow!("the session has a (net ...) block with no name"))?;
        let routes = child_blocks(text, block)
            .context(SCAN)?
            .into_iter()
            .filter(|child| has_head(text, *child, "wire") || has_head(text, *child, "via"))
            .collect();
        nets.push(NetBlock {
            block,
            name,
            routes,
        });
    }
    Ok(Network { mm_per_unit, nets })
}

/// A point as written: `[x, y]` as `i64` when both are whole numbers in the text.
fn whole(x: &str, y: &str) -> Option<[i64; 2]> {
    Some([x.parse().ok()?, y.parse().ok()?])
}

/// A wire's key pieces, its ends as written and its length in session units.
type WireParts = (BTreeSet<Piece>, Option<[[i64; 2]; 2]>, f64);

/// One `(wire …)` block: its key's pieces, its ends and its length in session units.
fn wire(text: &str, block: Block) -> Result<WireParts> {
    let mut pieces = BTreeSet::new();
    let mut length = 0.0;
    let mut paths = 0usize;
    let mut has_arc = false;
    let mut ends = None;
    for child in child_blocks(text, block).context(SCAN)? {
        if has_head(text, child, "path") {
            paths += 1;
            let data = atoms(text, child)?;
            if data.len() < 5 || (data.len() - 3) % 2 != 0 {
                bail!("the session has a path with no complete point pair");
            }
            let layer = data[1].to_string();
            let width = bits(number(data[2])?);
            let points = data[3..]
                .chunks_exact(2)
                .map(|pair| Ok([number(pair[0])?, number(pair[1])?]))
                .collect::<Result<Vec<[f64; 2]>>>()?;
            for pair in points.windows(2) {
                length += (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]);
                let mut sorted = [
                    [bits(pair[0][0]), bits(pair[0][1])],
                    [bits(pair[1][0]), bits(pair[1][1])],
                ];
                sorted.sort();
                pieces.insert(Piece::Segment {
                    layer: layer.clone(),
                    width,
                    ends: sorted,
                });
            }
            let last = data.len() - 2;
            ends = match (whole(data[3], data[4]), whole(data[last], data[last + 1])) {
                (Some(first), Some(last)) => Some([first, last]),
                _ => None,
            };
        } else if has_head(text, child, "qarc") {
            has_arc = true;
            let data = atoms(text, child)?;
            if data.len() != 9 {
                bail!("the session has a qarc without layer, width, start, end and centre");
            }
            let n = data[2..]
                .iter()
                .map(|value| number(value))
                .collect::<Result<Vec<f64>>>()?;
            length += std::f64::consts::FRAC_PI_2 * (n[1] - n[5]).hypot(n[2] - n[6]);
            pieces.insert(Piece::Arc {
                layer: data[1].to_string(),
                width: bits(n[0]),
                points: [
                    [bits(n[1]), bits(n[2])],
                    [bits(n[3]), bits(n[4])],
                    [bits(n[5]), bits(n[6])],
                ],
            });
        }
    }
    if has_arc || paths != 1 {
        ends = None;
    }
    Ok((pieces, ends, length))
}

/// One `(via PADSTACK X Y …)` block: its key's point and its point as written.
fn via(text: &str, block: Block) -> Result<([Bits; 2], Option<[i64; 2]>)> {
    let data = atoms(text, block)?;
    let (Some(x), Some(y)) = (data.get(2), data.get(3)) else {
        bail!("the session has a via without a padstack and a point");
    };
    Ok(([bits(number(x)?), bits(number(y)?)], whole(x, y)))
}

/// Every unit of `ses_text` on the nets of `nets`, in the text order of each unit's first
/// block.
///
/// Scans the text with the block scanner of `route_search` (quote-aware byte ranges), so
/// that [`without`] can cut by bytes: the root `(session …)`, its `(routes …)`, its
/// `(network_out …)`, every `(net NAME …)` whose name (a bare word or a quoted string,
/// unquoted before it is compared) is in `nets`, every direct `(wire …)` and `(via …)`
/// child. A net named in two `net` blocks gives units from both. The resolution is the
/// `(resolution UNIT N)` of `routes`. A session without `routes` or `network_out` is an
/// error; a session with no block of those nets gives an empty list.
pub(crate) fn units(ses_text: &str, nets: &BTreeSet<String>) -> Result<Vec<Unit>> {
    let network = network(ses_text)?;
    let mut out: Vec<Unit> = Vec::new();
    let mut seen: BTreeMap<UnitKey, usize> = BTreeMap::new();
    for net in network.nets.iter().filter(|net| nets.contains(&net.name)) {
        for &block in &net.routes {
            let (key, unit) = if has_head(ses_text, block, "via") {
                let (point, at) = via(ses_text, block)?;
                (
                    UnitKey::Via {
                        net: net.name.clone(),
                        at: point,
                    },
                    Unit {
                        kind: UnitKind::Via,
                        net: net.name.clone(),
                        spans: vec![(block.start, block.end)],
                        ends: None,
                        at,
                        length_mm: 0.0,
                    },
                )
            } else {
                let (pieces, ends, length) = wire(ses_text, block)?;
                (
                    UnitKey::Wire {
                        net: net.name.clone(),
                        pieces,
                    },
                    Unit {
                        kind: UnitKind::Wire,
                        net: net.name.clone(),
                        spans: vec![(block.start, block.end)],
                        ends,
                        at: None,
                        length_mm: length * network.mm_per_unit,
                    },
                )
            };
            match seen.get(&key) {
                Some(&index) => out[index].spans.push((block.start, block.end)),
                None => {
                    seen.insert(key, out.len());
                    out.push(unit);
                }
            }
        }
    }
    Ok(out)
}

/// `ses_text` without the units whose index is in `removed`: every block of each cut
/// together with the spaces and tabs before it on its line, and the line's newline when
/// nothing else is left on the line. A `(net …)` block left with no wire and no via is
/// cut whole, the same way. Every other byte is kept. `removed` empty gives the text
/// unchanged.
pub(crate) fn without(ses_text: &str, units: &[Unit], removed: &BTreeSet<usize>) -> String {
    if removed.is_empty() {
        return ses_text.to_string();
    }
    let mut cuts: Vec<(usize, usize)> = removed
        .iter()
        .filter_map(|&index| units.get(index))
        .flat_map(|unit| unit.spans.iter().copied())
        .collect();
    // A net block whose every wire and via is cut goes whole. The text was scanned by
    // `units` already, so a scan error here cannot happen; if it did, only the blocks
    // themselves are cut.
    if let Ok(network) = network(ses_text) {
        let starts: BTreeSet<usize> = cuts.iter().map(|cut| cut.0).collect();
        for net in &network.nets {
            if !net.routes.is_empty() && net.routes.iter().all(|r| starts.contains(&r.start)) {
                let (start, end) = (net.block.start, net.block.end);
                cuts.retain(|cut| cut.0 < start || cut.1 > end);
                cuts.push((start, end));
            }
        }
    }

    let b = ses_text.as_bytes();
    let mut gone = vec![false; b.len()];
    for (start, end) in cuts {
        if start > end || end > b.len() {
            continue;
        }
        gone[start..end].iter_mut().for_each(|g| *g = true);
        let mut i = start;
        while i > 0 && matches!(b[i - 1], b' ' | b'\t') {
            i -= 1;
            gone[i] = true;
        }
    }
    // A line that a cut touched and that holds nothing else goes with its newline.
    let mut line_start = 0;
    while line_start < b.len() {
        let line_end = b[line_start..]
            .iter()
            .position(|&c| c == b'\n')
            .map_or(b.len(), |p| line_start + p);
        let touched = gone[line_start..line_end].iter().any(|&g| g);
        let empty = (line_start..line_end).all(|i| gone[i] || matches!(b[i], b' ' | b'\t' | b'\r'));
        if touched && empty {
            let through = (line_end + 1).min(b.len());
            gone[line_start..through].iter_mut().for_each(|g| *g = true);
        }
        line_start = line_end + 1;
    }
    let kept: Vec<u8> = b
        .iter()
        .zip(&gone)
        .filter(|(_, &g)| !g)
        .map(|(&c, _)| c)
        .collect();
    // Every cut starts and ends on an ASCII byte, so the rest is whole UTF-8.
    String::from_utf8_lossy(&kept).into_owned()
}

/// The indices of the wires of `units` that end at via `via` (one of their two `ends`
/// equals its `at`, same net) and are not in `removed`.
pub(crate) fn wires_ending_at(units: &[Unit], via: usize, removed: &BTreeSet<usize>) -> Vec<usize> {
    let Some(target) = units.get(via) else {
        return Vec::new();
    };
    let Some(at) = target.at else {
        return Vec::new();
    };
    units
        .iter()
        .enumerate()
        .filter(|(index, unit)| {
            unit.kind == UnitKind::Wire
                && unit.net == target.net
                && !removed.contains(index)
                && unit.ends.is_some_and(|ends| ends[0] == at || ends[1] == at)
        })
        .map(|(index, _)| index)
        .collect()
}

/// What the prune reads from the audit's JSON. A value the audit did not give is `None`.
/// - `ground`: the names in `net_roles.ground.nets` (P3).
/// - `unstitched_layer_changes`: `metrics["3.2.unstitched_layer_changes"]`.
/// - 4.1's aggregates from the top-level `metrics` map: `4.1.worst_loop_mm`,
///   `4.1.median_loop_mm`, `4.1.mean_loop_area_mm2`; `loops_measured` from the `checks[]`
///   entry whose `item` is `4.1`, its `metrics.measured`; `pin_loops` from that entry's
///   `metrics.pins[]`: `pin` to (`loop.loop_mm`, `loop.area_mm2`) (the audit lists at
///   most 25 pins).
/// - `scored`: the `item` of every `checks[]` entry whose `score` is not null.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AuditItems {
    pub ground: BTreeSet<String>,
    pub unstitched_layer_changes: Option<f64>,
    pub loops_measured: Option<u64>,
    pub worst_loop_mm: Option<f64>,
    pub median_loop_mm: Option<f64>,
    pub mean_loop_area_mm2: Option<f64>,
    pub pin_loops: BTreeMap<String, (f64, f64)>,
    pub scored: BTreeSet<String>,
}

pub(crate) fn audit_items(audit: &Value) -> AuditItems {
    // The audit writes the role lists as a map from name to its source; a list of
    // names is read too.
    let ground = match &audit["net_roles"]["ground"]["nets"] {
        Value::Object(map) => map.keys().cloned().collect(),
        Value::Array(list) => list
            .iter()
            .filter_map(|name| name.as_str().map(str::to_string))
            .collect(),
        _ => BTreeSet::new(),
    };
    let metric = |key: &str| audit["metrics"][key].as_f64();
    let checks: &[Value] = audit["checks"].as_array().map_or(&[], Vec::as_slice);
    let decoupling = checks.iter().find(|check| check["item"] == "4.1");
    let pin_loops = decoupling
        .and_then(|check| check["metrics"]["pins"].as_array())
        .map(|pins| {
            pins.iter()
                .filter_map(|row| {
                    Some((
                        row["pin"].as_str()?.to_string(),
                        (
                            row["loop"]["loop_mm"].as_f64()?,
                            row["loop"]["area_mm2"].as_f64()?,
                        ),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    AuditItems {
        ground,
        unstitched_layer_changes: metric("3.2.unstitched_layer_changes"),
        loops_measured: decoupling.and_then(|check| check["metrics"]["measured"].as_u64()),
        worst_loop_mm: metric("4.1.worst_loop_mm"),
        median_loop_mm: metric("4.1.median_loop_mm"),
        mean_loop_area_mm2: metric("4.1.mean_loop_area_mm2"),
        pin_loops,
        scored: checks
            .iter()
            .filter(|check| !check["score"].is_null())
            .filter_map(|check| check["item"].as_str().map(str::to_string))
            .collect(),
    }
}

/// What one check of a candidate session gave. `rest` is `None` when the check stopped at
/// the DRC (P7).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Checked {
    pub unconnected: usize,
    pub violations: BTreeMap<String, usize>,
    pub rest: Option<CheckedRest>,
}

/// The part of a check that needs the board parsed and the audit run.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheckedRest {
    /// The pour zones' fill on the refilled board (`zone_fill::pour_fills`).
    pub fills: Vec<ZoneLayerFill>,
    /// The light judge's measures and gates of this board (`route_judge::judge_light`).
    /// For (c) only; never reported (P9).
    pub measures: Measures,
    pub gates: Gates,
    pub audit: AuditItems,
}

/// (a) and (b): every DRC clause that refuses a candidate with these counts against
/// `current`, in order: `unconnected` when the count is higher, then `rule:<name>` for
/// every rule whose count is higher (a rule `current` does not have counts as 0), in
/// name order. Empty: the DRC allows it, and the rest of the check must run (P7).
pub(crate) fn drc_refusals(
    current: &Checked,
    unconnected: usize,
    violations: &BTreeMap<String, usize>,
) -> Vec<String> {
    let mut out = Vec::new();
    if unconnected > current.unconnected {
        out.push("unconnected".to_string());
    }
    for (rule, &count) in violations {
        if count > current.violations.get(rule).copied().unwrap_or(0) {
            out.push(format!("rule:{rule}"));
        }
    }
    out
}

/// The total fill of each zone layer (net, layer, zone UUID), mm².
fn fill_by_zone_layer(fills: &[ZoneLayerFill]) -> BTreeMap<(&str, &str, Option<&str>), f64> {
    let mut out = BTreeMap::new();
    for fill in fills {
        *out.entry((
            fill.net.as_str(),
            fill.layer.as_str(),
            fill.zone_uuid.as_deref(),
        ))
        .or_insert(0.0) += fill.pieces_mm2.iter().sum::<f64>();
    }
    out
}

/// (f) against the original: whether any decoupling-loop measure got worse beyond its
/// tolerance, or was given and is no longer.
fn loops_worse(original: &AuditItems, candidate: &AuditItems) -> bool {
    let above = |was: Option<f64>, now: Option<f64>, tolerance: f64| match (was, now) {
        (Some(_), None) => true,
        (Some(was), Some(now)) => now > was + tolerance,
        (None, _) => false,
    };
    let fewer = match (original.loops_measured, candidate.loops_measured) {
        (Some(_), None) => true,
        (Some(was), Some(now)) => now < was,
        (None, _) => false,
    };
    fewer
        || above(
            original.worst_loop_mm,
            candidate.worst_loop_mm,
            LOOP_TOLERANCE_MM,
        )
        || above(
            original.median_loop_mm,
            candidate.median_loop_mm,
            LOOP_TOLERANCE_MM,
        )
        || above(
            original.mean_loop_area_mm2,
            candidate.mean_loop_area_mm2,
            LOOP_AREA_TOLERANCE_MM2,
        )
        || original.pin_loops.iter().any(|(pin, &(length, area))| {
            candidate
                .pin_loops
                .get(pin)
                .is_some_and(|&(now_length, now_area)| {
                    now_length > length + LOOP_TOLERANCE_MM
                        || now_area > area + LOOP_AREA_TOLERANCE_MM2
                })
        })
}

/// Every clause of P5 that refuses `candidate`, empty when it may replace `current`.
/// [`drc_refusals`] first; when it refuses, only its names are returned. Else, with the
/// `rest` of all three (the candidate's must be there; `original` and `current` always
/// have it; when one is missing the clause is `unchecked`):
/// - `key:<position>` for every entry but the last (seconds) of the candidate's key that
///   is above the current's: `route_judge::rank_measures` over the two, both in the pool.
/// - `stitching`: (e), against `current`. `None` on both sides passes; `None` on one
///   side refuses.
/// - `unscored`: (g), `current.scored` is not a subset of the candidate's.
/// - `fill`: (d), against `original`: for every zone layer of `original` (net, layer,
///   zone UUID), the candidate's total fill area there is below the original's minus
///   [`FILL_TOLERANCE_MM2`]; a zone layer missing in the candidate counts as 0.
/// - `loops`: (f), against `original`: fewer loops measured; the worst or the median
///   above the original's plus [`LOOP_TOLERANCE_MM`]; the mean area above plus
///   [`LOOP_AREA_TOLERANCE_MM2`]; a pin of both `pin_loops` longer or larger by more than
///   the same tolerances; a value that was `Some` and is `None`.
pub(crate) fn refusals(original: &Checked, current: &Checked, candidate: &Checked) -> Vec<String> {
    let drc = drc_refusals(current, candidate.unconnected, &candidate.violations);
    if !drc.is_empty() {
        return drc;
    }
    let (Some(first), Some(now), Some(next)) = (&original.rest, &current.rest, &candidate.rest)
    else {
        return vec!["unchecked".to_string()];
    };
    let mut out = Vec::new();

    let keys = crate::route_judge::rank_measures(&[
        (&now.measures, &now.gates, true),
        (&next.measures, &next.gates, true),
    ]);
    // Every entry but the last (the route seconds).
    for (position, (now, next)) in keys[0]
        .iter()
        .zip(&keys[1])
        .take(crate::route_judge::KEY_LEN - 1)
        .enumerate()
    {
        if next > now {
            out.push(format!("key:{position}"));
        }
    }

    let stitching_worse = match (
        now.audit.unstitched_layer_changes,
        next.audit.unstitched_layer_changes,
    ) {
        (None, None) => false,
        (Some(was), Some(is)) => is > was,
        _ => true,
    };
    if stitching_worse {
        out.push("stitching".to_string());
    }

    if !now.audit.scored.is_subset(&next.audit.scored) {
        out.push("unscored".to_string());
    }

    let had = fill_by_zone_layer(&first.fills);
    let has = fill_by_zone_layer(&next.fills);
    if had
        .iter()
        .any(|(zone, &area)| has.get(zone).copied().unwrap_or(0.0) < area - FILL_TOLERANCE_MM2)
    {
        out.push("fill".to_string());
    }

    if loops_worse(&first.audit, &next.audit) {
        out.push("loops".to_string());
    }
    out
}

/// A future a checker returns (the crate's `BoxFuture` is `'static`; this one borrows).
pub(crate) type CheckFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<CheckOutcome>> + Send + 'a>>;

/// One check's answer: the measures, or the importer's refusal of the candidate TEXT
/// (P10: a refused candidate, not an error).
// One answer a check, moved once: its size does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CheckOutcome {
    Checked(Checked),
    ImporterRefused(String),
}

/// Checks one candidate session. `current` is the state accepted so far: the checker
/// stops at the DRC when [`drc_refusals`] against it is not empty (P7). `None`: the
/// whole check (the original). The production checker is `JudgeChecker`; the tests of
/// the loop script one.
pub(crate) trait Checker: Send + Sync {
    fn check<'a>(&'a self, ses_text: &'a str, current: Option<&'a Checked>) -> CheckFuture<'a>;
}

/// The loop's result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pruned {
    /// Indices of the removed units.
    pub removed: BTreeSet<usize>,
    /// Candidate checks made (the original's check is the caller's).
    pub checks: usize,
    /// `done`, `check_cap` or `check_error` (with `error`).
    pub ended_by: &'static str,
    pub error: Option<String>,
    /// For the vias that stayed: how many had each clause refuse their last with-wires
    /// attempt, and their last alone attempt (a via counts once under every clause that
    /// refused; an importer refusal is the clause `importer`).
    pub kept_with: BTreeMap<String, usize>,
    pub kept_alone: BTreeMap<String, usize>,
    /// The last accepted state (the original when nothing was removed).
    pub last: Checked,
}

/// What one attempt of the loop gave.
// One a check, moved once: its size does not matter.
#[allow(clippy::large_enum_variant)]
enum Attempt {
    Accepted(Checked),
    Refused(BTreeSet<String>),
}

/// The loop's running state.
struct Run<'a> {
    ses_text: &'a str,
    units: &'a [Unit],
    original: &'a Checked,
    checker: &'a dyn Checker,
    current: Checked,
    removed: BTreeSet<usize>,
    checks: usize,
}

/// Why the loop stopped early.
enum Stop {
    Cap,
    Error(String),
}

impl Run<'_> {
    /// One check of the session without `removed ∪ extra`, judged against the state
    /// accepted so far. Accepting changes nothing here; the caller takes it.
    async fn attempt(&mut self, extra: &BTreeSet<usize>) -> std::result::Result<Attempt, Stop> {
        if self.checks >= MAX_CHECKS {
            return Err(Stop::Cap);
        }
        let cut: BTreeSet<usize> = self.removed.union(extra).copied().collect();
        let text = without(self.ses_text, self.units, &cut);
        self.checks += 1;
        match self.checker.check(&text, Some(&self.current)).await {
            Err(error) => Err(Stop::Error(format!("{error:#}"))),
            Ok(CheckOutcome::ImporterRefused(_)) => {
                Ok(Attempt::Refused(BTreeSet::from(["importer".to_string()])))
            }
            Ok(CheckOutcome::Checked(checked)) => {
                let refused = refusals(self.original, &self.current, &checked);
                if refused.is_empty() {
                    Ok(Attempt::Accepted(checked))
                } else {
                    Ok(Attempt::Refused(refused.into_iter().collect()))
                }
            }
        }
    }

    fn accept(&mut self, with: BTreeSet<usize>, checked: Checked) {
        self.removed.extend(with);
        self.current = checked;
    }

    /// The sweeps; `Err` when the loop stopped early.
    async fn sweeps(
        &mut self,
        last_with: &mut BTreeMap<usize, BTreeSet<String>>,
        last_alone: &mut BTreeMap<usize, BTreeSet<String>>,
    ) -> std::result::Result<(), Stop> {
        let vias: Vec<usize> = (0..self.units.len())
            .filter(|&index| self.units[index].kind == UnitKind::Via)
            .collect();
        let mut finals: BTreeSet<usize> = BTreeSet::new();
        for _ in 0..VIA_SWEEPS {
            let mut removed_one = false;
            for &via in &vias {
                if self.removed.contains(&via) || finals.contains(&via) {
                    continue;
                }
                let mut with: BTreeSet<usize> = wires_ending_at(self.units, via, &self.removed)
                    .into_iter()
                    .collect();
                with.insert(via);
                if with.len() > 1 {
                    match self.attempt(&with).await? {
                        Attempt::Accepted(checked) => {
                            self.accept(with, checked);
                            removed_one = true;
                            continue;
                        }
                        Attempt::Refused(clauses) => {
                            last_with.insert(via, clauses);
                        }
                    }
                }
                let alone = BTreeSet::from([via]);
                match self.attempt(&alone).await? {
                    Attempt::Accepted(checked) => {
                        self.accept(alone, checked);
                        removed_one = true;
                    }
                    Attempt::Refused(clauses) => {
                        if clauses.contains("unconnected") || clauses.contains("fill") {
                            finals.insert(via);
                        }
                        last_alone.insert(via, clauses);
                    }
                }
            }
            if !removed_one {
                break;
            }
        }
        Ok(())
    }
}

/// How many of the vias that stayed had each clause in their last attempt.
fn kept(
    last: &BTreeMap<usize, BTreeSet<String>>,
    removed: &BTreeSet<usize>,
) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for (_, clauses) in last.iter().filter(|(via, _)| !removed.contains(via)) {
        for clause in clauses {
            *out.entry(clause.clone()).or_insert(0) += 1;
        }
    }
    out
}

/// The loop (P5 to P7, P10). `units` are [`units`] of `ses_text`; `original` is the
/// whole check of `ses_text` itself.
///
/// 1. `current = original`; `removed = {}`.
/// 2. Sweeps, at most [`VIA_SWEEPS`]; a sweep after the first runs only when the one
///    before removed a via. In a sweep, for each via not removed and not final, in text
///    order:
///    a. `with = {via} ∪ wires_ending_at(via)`. When `with` holds a wire: check the
///    session without `removed ∪ with`; when [`refusals`] is empty, `removed` takes
///    `with`, `current` is that check, next via. Else its clauses are the via's
///    with-wires clauses.
///    b. Check the session without `removed ∪ {via}`. Empty: the via is removed and
///    `current` is that check. Else its clauses are the via's alone clauses, and when
///    they hold `unconnected` or `fill` the via is final (P6).
///    An importer refusal is a refused attempt (clause `importer`), never final.
/// 3. Before every check: when `checks == MAX_CHECKS` the loop ends `check_cap`.
/// 4. A check that returns an error ends the loop `check_error` with the state accepted
///    so far (P10).
///
/// So `current` is always the result of one real check of exactly `removed`; by (a),
/// (b), (c), (e) and (g) it is never worse than the state before it, and by (d) and (f)
/// never worse than the original beyond one tolerance.
/// Deterministic: text order, no clock, no random number.
pub(crate) async fn prune_loop(
    ses_text: &str,
    units: &[Unit],
    original: &Checked,
    checker: &dyn Checker,
) -> Pruned {
    let mut run = Run {
        ses_text,
        units,
        original,
        checker,
        current: original.clone(),
        removed: BTreeSet::new(),
        checks: 0,
    };
    let mut last_with = BTreeMap::new();
    let mut last_alone = BTreeMap::new();
    let (ended_by, error) = match run.sweeps(&mut last_with, &mut last_alone).await {
        Ok(()) => ("done", None),
        Err(Stop::Cap) => ("check_cap", None),
        Err(Stop::Error(error)) => ("check_error", Some(error)),
    };
    Pruned {
        kept_with: kept(&last_with, &run.removed),
        kept_alone: kept(&last_alone, &run.removed),
        removed: run.removed,
        checks: run.checks,
        ended_by,
        error,
        last: run.current,
    }
}

// ─── The production checker ──────────────────────────────────────────────────

/// The judge folder of the loop's checks of run `index` (removed after the prune).
fn prune_folder(index: usize) -> String {
    format!("run-{index}-prune")
}

/// The judge folder of the full judge of run `index`'s pruned session (P14).
fn pruned_folder(index: usize) -> String {
    format!("run-{index}-pruned")
}

/// The fill of every pour zone of the call on the board kicad-cli saved, read and parsed
/// off the async threads (as `zone_fill::pour_record` reads it).
async fn fills_of(board: &Path, pour_nets: &BTreeSet<String>) -> Result<Vec<ZoneLayerFill>> {
    let path = board.to_path_buf();
    let nets = pour_nets.clone();
    tokio::task::spawn_blocking(move || -> Result<Vec<ZoneLayerFill>> {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("read the checked board '{}'", path.display()))?;
        crate::zone_fill::pour_fills(&text, &nets)
    })
    .await
    .context("the pours' fill could not be read")?
}

/// The production checker. One check: [`route_judge::routed_scratch`] of the text in
/// `<judge root>/run-<index>-prune/c` (cleared before each check; its own settings copy,
/// as `judge_one` makes one a run) with the prune's severities forced (P11). An importer
/// refusal of a candidate (`current` given): the ORIGINAL session text is parsed again by
/// the importer (`parse_import_plan_with`, the judge's arguments); when that fails too the
/// check is an error (the board or the manifest changed, P10), else
/// `CheckOutcome::ImporterRefused`. The original's own check (`current` `None`) checks that
/// very text, so its refusal is `ImporterRefused` with no second parse: the board or the
/// manifest changed since the search judged it. Then the DRC's counts; when `current` is given and
/// [`drc_refusals`] is not empty, stop there (`rest: None`). Else the fills
/// (`zone_fill::pour_fills` of the saved board, off the async threads) and
/// `route_judge::judge_light` (the audit, the measures, the gates), and `rest`.
/// Counts its KiCad runs and its audits.
pub(crate) struct JudgeChecker<'c> {
    ctx: &'c JudgeContext,
    baseline: &'c Baseline,
    tools: &'c dyn ToolCaller,
    /// The winner's own session text (P10's second parse).
    original: &'c str,
    index: usize,
    route_s: f64,
    /// `<judge root>/kicad-config/run-<index>-prune`, removed with the checker.
    config: OwnConfig,
    kicad_runs: AtomicUsize,
    audits: AtomicUsize,
}

impl<'c> JudgeChecker<'c> {
    /// A checker of run `index`'s sessions, with its own copy of the call's KiCad
    /// settings. `route_s` is the run's route time (the light judge's measures carry it).
    pub(crate) fn new(
        ctx: &'c JudgeContext,
        baseline: &'c Baseline,
        tools: &'c dyn ToolCaller,
        original: &'c str,
        index: usize,
        route_s: f64,
    ) -> Result<Self> {
        Ok(Self {
            ctx,
            baseline,
            tools,
            original,
            index,
            route_s,
            config: OwnConfig::new(ctx, &prune_folder(index))?,
            kicad_runs: AtomicUsize::new(0),
            audits: AtomicUsize::new(0),
        })
    }

    /// KiCad refills and DRCs this checker has run.
    pub(crate) fn kicad_runs(&self) -> usize {
        self.kicad_runs.load(Ordering::Relaxed)
    }

    /// Layout audits this checker has asked for.
    pub(crate) fn audits(&self) -> usize {
        self.audits.load(Ordering::Relaxed)
    }

    async fn check_text(&self, ses_text: &str, current: Option<&Checked>) -> Result<CheckOutcome> {
        let ctx = self.ctx;
        let run_dir = ctx.root.join(prune_folder(self.index)).join("c");
        let scratch = match route_judge::routed_scratch(
            ctx,
            ses_text,
            &run_dir,
            self.config.path(),
            true,
        )
        .await
        {
            Ok(scratch) => scratch,
            Err(ScratchError::Importer(refused)) => {
                // P10: the candidate, or what it is checked against? The original's own
                // check is of the original text: a second parse would only repeat it.
                if current.is_some() {
                    crate::specctra_ses::parse_import_plan_with(
                        &ctx.board,
                        &ctx.board_text,
                        &ctx.manifest_text,
                        self.original,
                        crate::specctra_ses::SourceCheck::BytesOrTree,
                    )
                    .context(
                        "the winner's own session no longer passes the importer: the board, its project files or the manifest changed during the prune",
                    )?;
                }
                return Ok(CheckOutcome::ImporterRefused(format!("{refused:#}")));
            }
            Err(ScratchError::Other(error)) => return Err(error),
        };
        self.kicad_runs.fetch_add(1, Ordering::Relaxed);
        let unconnected = route_judge::unconnected_total(&scratch.drc)?;
        let violations = route_judge::violations_by_rule(&scratch.drc);
        // P7: the audit runs only after (a) and (b) pass.
        if let Some(current) = current {
            if !drc_refusals(current, unconnected, &violations).is_empty() {
                return Ok(CheckOutcome::Checked(Checked {
                    unconnected,
                    violations,
                    rest: None,
                }));
            }
        }
        let fills = fills_of(&scratch.board, &ctx.pour_nets).await?;
        self.audits.fetch_add(1, Ordering::Relaxed);
        let (measures, gates, audit) =
            route_judge::judge_light(ctx, self.baseline, self.tools, &scratch, self.route_s)
                .await?;
        Ok(CheckOutcome::Checked(Checked {
            unconnected,
            violations,
            rest: Some(CheckedRest {
                fills,
                measures,
                gates,
                audit: audit_items(&audit),
            }),
        }))
    }
}

impl Checker for JudgeChecker<'_> {
    fn check<'a>(&'a self, ses_text: &'a str, current: Option<&'a Checked>) -> CheckFuture<'a> {
        Box::pin(self.check_text(ses_text, current))
    }
}

// ─── The prune of a finished search ──────────────────────────────────────────

/// What the result reports about the prune (`search.pour_prune`, the winner's
/// `run.json`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct PruneRecord {
    /// `pruned` (the delivered session is the pruned one), `nothing_removed`, `rejected`
    /// (the full judge of the pruned session was refused, P9), `skipped`, `failed`.
    pub outcome: &'static str,
    /// Why, for `rejected`, `skipped` and `failed`; the loop's error when it ended
    /// `check_error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The ground pour nets whose vias were tried, and the pour nets left alone (P3).
    pub nets: Vec<String>,
    pub nets_left_alone: Vec<String>,
    pub vias_before: usize,
    /// The vias the loop removed, the wires that went with them, and their length. For
    /// `rejected` (and a `failed` full judge) what the prune WOULD have removed: the
    /// delivered session keeps them all.
    pub vias_removed: usize,
    pub wires_removed: usize,
    pub track_mm_removed: f64,
    /// For the vias that stayed, the clauses that refused them ([`Pruned`]).
    pub vias_kept_with: BTreeMap<String, usize>,
    pub vias_kept_alone: BTreeMap<String, usize>,
    /// The KiCad refills and DRCs that gave a report the prune read: the original's
    /// check, every candidate the importer took, and the full judge's refill, its
    /// stitching rounds and its length probe. A refill whose report could not be read is
    /// not counted, and an importer refusal runs no KiCad.
    pub checks: usize,
    /// The layout audits the prune asked for (the original's, the candidates' that passed
    /// the DRC, the full judge's).
    pub audits: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_by: Option<&'static str>,
    pub seconds: f64,
    /// The original winner, when the delivered session is the pruned one: its session
    /// path, its key, its judged board (null when it was released).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
}

impl PruneRecord {
    fn new(outcome: &'static str, reason: Option<String>) -> Self {
        Self {
            outcome,
            reason,
            nets: Vec::new(),
            nets_left_alone: Vec::new(),
            vias_before: 0,
            vias_removed: 0,
            wires_removed: 0,
            track_mm_removed: 0.0,
            vias_kept_with: BTreeMap::new(),
            vias_kept_alone: BTreeMap::new(),
            checks: 0,
            audits: 0,
            ended_by: None,
            seconds: 0.0,
            before: None,
        }
    }

    fn ended(mut self, outcome: &'static str, reason: impl Into<Option<String>>) -> Self {
        self.outcome = outcome;
        self.reason = reason.into();
        self
    }
}

/// The winner of a finished search, owned, so that the prune runs as a task of its own.
struct Winner {
    index: usize,
    routed: Routed,
    judged: Judged,
    baseline: Baseline,
}

/// What the prune decided. `kept`: the pruned session's path and its full judge, which
/// replace the winner's when every run's keys are taken again (the record's outcome is
/// then `pruned`).
struct Decision {
    record: PruneRecord,
    kept: Option<(PathBuf, Judged)>,
    /// Files this prune wrote that go when the original stays: `pruned.ses` and the
    /// `-pruned` judge folder.
    written: Vec<PathBuf>,
}

impl Decision {
    fn of(record: PruneRecord) -> Self {
        Self {
            record,
            kept: None,
            written: Vec::new(),
        }
    }
}

/// Step 1: the winner when it may be pruned, else why not (P8).
fn winner_of(ctx: &JudgeContext, outcome: &SearchOutcome) -> std::result::Result<Winner, String> {
    if ctx.pour_nets.is_empty() {
        return Err("the call has no pour net".to_string());
    }
    let Some(index) = outcome.winner else {
        return Err("no run won".to_string());
    };
    let Some(record) = outcome.runs.get(index) else {
        return Err("no run won".to_string());
    };
    let (Some(routed), Some(judged)) = (&record.routed, &record.judged) else {
        return Err("the winning record has no routed session or no judged board".to_string());
    };
    if !judged.gates.all_pass() {
        return Err("the winner fails a gate".to_string());
    }
    if !judged.rounds.is_empty() {
        return Err(format!(
            "the judge wrote {} stitching round(s) for the winner",
            judged.rounds.len()
        ));
    }
    // A failed baseline judges no run, so it leaves no winner either.
    let baseline = outcome
        .baseline
        .as_ref()
        .map_err(|error| format!("the unrouted board's baseline failed: {error}"))?;
    Ok(Winner {
        index: record.index,
        routed: routed.clone(),
        judged: judged.clone(),
        baseline: baseline.clone(),
    })
}

/// The first clause of P9 that refuses the pruned session's full judge against the
/// original's, `None` when the pruned one may replace it.
fn full_judge_refusal(original: &Judged, pruned: &Judged) -> Option<String> {
    let gates = &pruned.gates;
    if let Some(line) = gates
        .g1
        .iter()
        .chain(&gates.g2)
        .chain(&gates.g3)
        .chain(&gates.g4)
        .next()
    {
        return Some(format!(
            "the full judge of the pruned session fails a gate: {line}"
        ));
    }
    if !pruned.rounds.is_empty() {
        return Some(format!(
            "the full judge of the pruned session wrote {} stitching round(s)",
            pruned.rounds.len()
        ));
    }
    let keys = route_judge::rank(&[original, pruned]);
    let (was, now) = (keys.first()?, keys.get(1)?);
    (0..route_judge::KEY_LEN - 1)
        .find(|&position| now[position] > was[position])
        .map(|position| {
            format!(
                "the pruned session's rank key is above the original's at position {position} ({} > {})",
                now[position], was[position]
            )
        })
}

/// The KiCad runs one full judge made: the refill, each stitching round that wrote a
/// board, and the length probe when it ran.
fn kicad_runs_of(judged: &Judged) -> usize {
    1 + judged
        .rounds
        .iter()
        .filter(|row| row.get("open_after").is_some())
        .count()
        + usize::from(judged.measures.lengths.is_some())
}

/// Removes a file or a folder this prune made; a link is never followed or removed, and
/// a path that is already gone is fine.
fn remove_own(path: &Path) {
    let Ok(meta) = path.symlink_metadata() else {
        return;
    };
    let removed = if meta.file_type().is_symlink() {
        Ok(())
    } else if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    if let Err(error) = removed {
        tracing::warn!(
            "the ground-via prune could not remove '{}': {error}",
            path.display()
        );
    }
}

/// Steps 2 to 5 of [`prune_winner`], on an owned winner.
async fn decide(ctx: Arc<JudgeContext>, tools: Arc<dyn ToolCaller>, winner: Winner) -> Decision {
    let ctx: &JudgeContext = &ctx;
    let tools: &dyn ToolCaller = &*tools;
    let record = PruneRecord::new("failed", None);
    let held = &winner.routed.held;
    let text = match tokio::fs::read_to_string(&held.ses).await {
        Ok(text) => text,
        Err(error) => {
            return Decision::of(record.ended(
                "failed",
                format!(
                    "the winner's session '{}' could not be read: {error}",
                    held.ses.display()
                ),
            ))
        }
    };
    let checker = match JudgeChecker::new(
        ctx,
        &winner.baseline,
        tools,
        &text,
        winner.index,
        held.seconds,
    ) {
        Ok(checker) => checker,
        Err(error) => {
            return Decision::of(record.ended(
                "failed",
                format!("the prune's KiCad settings copy could not be made: {error:#}"),
            ))
        }
    };
    let mut decision = decide_with(ctx, tools, &winner, &text, &checker, record).await;
    decision.record.checks += checker.kicad_runs();
    decision.record.audits += checker.audits();
    decision
}

/// [`decide`] once the checker is made: steps 2 to 5.
async fn decide_with(
    ctx: &JudgeContext,
    tools: &dyn ToolCaller,
    winner: &Winner,
    text: &str,
    checker: &JudgeChecker<'_>,
    mut record: PruneRecord,
) -> Decision {
    // 2. The original's whole check.
    let original = match checker.check(text, None).await {
        Ok(CheckOutcome::Checked(checked)) => checked,
        Ok(CheckOutcome::ImporterRefused(why)) => {
            return Decision::of(record.ended(
                "failed",
                format!(
                    "the importer no longer takes the winner's own session, which the search judged (the board, its project files or the manifest changed since): {why}"
                ),
            ))
        }
        Err(error) => {
            return Decision::of(record.ended(
                "failed",
                format!("the check of the winner's own session failed: {error:#}"),
            ))
        }
    };
    let Some(rest) = &original.rest else {
        return Decision::of(record.ended(
            "failed",
            "the check of the winner's own session has no audit".to_string(),
        ));
    };

    // 3. The nets (P3), then the units of those nets.
    let (nets, left): (BTreeSet<String>, BTreeSet<String>) = ctx
        .pour_nets
        .iter()
        .cloned()
        .partition(|net| rest.audit.ground.contains(net));
    record.nets = nets.iter().cloned().collect();
    record.nets_left_alone = left.into_iter().collect();
    if nets.is_empty() {
        return Decision::of(record.ended(
            "skipped",
            "no pour net of the call is ground by the audit's net roles".to_string(),
        ));
    }
    let units = match units(text, &nets) {
        Ok(units) => units,
        Err(error) => {
            return Decision::of(record.ended(
                "failed",
                format!("the winner's session could not be cut into units: {error:#}"),
            ))
        }
    };
    record.vias_before = units.iter().filter(|u| u.kind == UnitKind::Via).count();
    if record.vias_before == 0 {
        return Decision::of(record.ended("skipped", "no via on a ground pour net".to_string()));
    }

    // 4. The loop.
    let pruned = prune_loop(text, &units, &original, checker).await;
    let removed: Vec<&Unit> = pruned
        .removed
        .iter()
        .filter_map(|&index| units.get(index))
        .collect();
    record.vias_removed = removed.iter().filter(|u| u.kind == UnitKind::Via).count();
    record.wires_removed = removed.iter().filter(|u| u.kind == UnitKind::Wire).count();
    // Folded from +0.0: an empty `sum` of f64 is -0.0, which the record would print.
    let track_mm = removed.iter().fold(0.0, |mm, unit| mm + unit.length_mm);
    record.track_mm_removed = (track_mm * 1000.0).round() / 1000.0;
    record.vias_kept_with = pruned.kept_with.clone();
    record.vias_kept_alone = pruned.kept_alone.clone();
    record.ended_by = Some(pruned.ended_by);
    if pruned.removed.is_empty() {
        return Decision::of(record.ended("nothing_removed", pruned.error));
    }
    if let Some(error) = &pruned.error {
        record.reason = Some(error.clone());
    }

    // 5. The pruned session, written beside the original and judged in full (P9).
    let session = winner.routed.folder.join("pruned.ses");
    let pruned_text = without(text, &units, &pruned.removed);
    if let Err(error) = konnect_sexp::write_new_atomic(&session, &pruned_text) {
        return Decision::of(record.ended(
            "failed",
            format!(
                "the pruned session '{}' could not be written: {error}",
                session.display()
            ),
        ));
    }
    let folder = pruned_folder(winner.index);
    let written = vec![session.clone(), ctx.root.join(&folder)];
    let judged = match route_judge::judge_session(
        ctx,
        &winner.routed,
        &winner.baseline,
        tools,
        &session,
        &folder,
    )
    .await
    {
        Ok(judged) => judged,
        Err(error) => {
            return Decision {
                record: record.ended(
                    "failed",
                    format!("the full judge of the pruned session failed: {error:#}"),
                ),
                kept: None,
                written,
            }
        }
    };
    record.checks += kicad_runs_of(&judged);
    record.audits += 1;
    if let Some(refused) = full_judge_refusal(&winner.judged, &judged) {
        return Decision {
            record: record.ended("rejected", refused),
            kept: None,
            written,
        };
    }
    Decision {
        record: record.ended("pruned", pruned.error),
        kept: Some((session, judged)),
        written,
    }
}

/// Aborts the prune's task when the call that waits for it is dropped, so that no
/// kicad-cli outlives the call.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Seconds as a number rounded to 0.1.
fn round1(seconds: f64) -> f64 {
    (seconds * 10.0).round() / 10.0
}

/// The judged board's path, or null when it is no longer there.
fn board_or_null(judged: &Judged) -> Value {
    if judged.board.is_file() {
        json!(judged.board.to_string_lossy())
    } else {
        Value::Null
    }
}

/// Every judged run's key, by index, with the winner's judged replaced by `pruned`: one
/// `rank` call over every judged run, as the search takes them.
fn keys_with(outcome: &SearchOutcome, winner: usize, pruned: &Judged) -> Vec<Option<Key>> {
    let judged: Vec<(usize, &Judged)> = outcome
        .runs
        .iter()
        .enumerate()
        .filter_map(|(position, record)| {
            let judged = if position == winner {
                Some(pruned)
            } else {
                record.judged.as_ref()
            };
            judged.map(|judged| (position, judged))
        })
        .collect();
    let mut keys: Vec<Option<Key>> = vec![None; outcome.runs.len()];
    let refs: Vec<&Judged> = judged.iter().map(|(_, judged)| *judged).collect();
    for ((position, _), key) in judged.iter().zip(route_judge::rank(&refs)) {
        if let Some(slot) = keys.get_mut(*position) {
            *slot = Some(key);
        }
    }
    keys
}

/// Step 5's replacement: the winner's session and judged become the pruned ones, and the
/// keys are taken again. `Err` (nothing changed) when the winner would no longer rank
/// first, which P9's key clause rules out.
fn replace_winner(
    outcome: &mut SearchOutcome,
    winner: usize,
    session: PathBuf,
    mut judged: Judged,
) -> std::result::Result<Value, String> {
    let Some(record) = outcome.runs.get(winner) else {
        return Err("the winning record is gone".to_string());
    };
    let (Some(routed), Some(original)) = (&record.routed, &record.judged) else {
        return Err("the winning record has no routed session or no judged board".to_string());
    };
    judged.tune = original.tune.clone();
    // Row ZR, ZR1: the comparison is of the routing pass's board, which the pruned
    // session's judge does not make; the original run's stays with the run.
    crate::zone_fill::carry_comparison(&original.measures, &mut judged.measures);
    let keys = keys_with(outcome, winner, &judged);
    let first = keys
        .iter()
        .enumerate()
        .filter_map(|(position, key)| key.map(|key| (key, position)))
        .min()
        .map(|(_, position)| position);
    if first != Some(winner) {
        return Err(
            "the pruned winner would no longer rank first among the judged runs".to_string(),
        );
    }
    let before = json!({
        "session": routed.held.ses.to_string_lossy(),
        "key": outcome.keys.get(winner).copied().flatten(),
        "judged_board": board_or_null(original),
    });
    if let Some(record) = outcome.runs.get_mut(winner) {
        if let Some(routed) = record.routed.as_mut() {
            routed.held.ses = session;
        }
        record.judged = Some(judged);
    }
    outcome.keys = keys;
    Ok(before)
}

/// ZR0b on a finished search. Changes `outcome` only when the prune is kept; always
/// sets `outcome.pour_prune`.
///
/// 1. `skipped` with its reason when: the call has no pour net; no run won, or the
///    winning record has no `routed` or no `judged`; the winner fails a gate, or its
///    `judged.rounds` is not empty (P8).
/// 2. The original's whole check (`JudgeChecker::check(text, None)`). An error or an
///    importer refusal: `failed`.
/// 3. The nets: the call's pour nets that the original's audit lists as ground (P3);
///    the others go to `nets_left_alone`. None: `skipped`. [`units`] of those nets; no
///    via among them: `skipped`.
/// 4. [`prune_loop`]. Nothing removed: `nothing_removed` (with the counts, the kept
///    clauses, `ended_by`, and the loop's error as the reason when it ended
///    `check_error`).
/// 5. Else the pruned text is written to `<winner's run folder>/pruned.ses`
///    (`write_new_atomic`) and judged in full: `route_judge::judge_session` on that file
///    in `judge/run-<index>-pruned`. An error: `failed`. P9: when every gate passes,
///    `rounds` is empty, and no entry but the last of the pruned key is above the
///    original's (`rank` over the two): the winner's `routed.held.ses` becomes
///    `pruned.ses`, its `judged` becomes the pruned one with the original's `tune`
///    carried over, and `outcome.keys` is taken again by one `rank` call over every
///    judged run (the winner does not change: its key did not rise). `pruned`. Else
///    `rejected` with the first failing clause, `pruned.ses` and the `-pruned` judge
///    folder removed, `outcome` otherwise unchanged.
/// 6. The loop's scratch folder `judge/run-<index>-prune` is removed in every case.
///
/// Never an error and never a panic for the call: whatever fails, the original winner is
/// delivered and the record says what happened. Steps 2 to 5 run as a task of their own
/// on owned copies, so that a panic in them is a `failed` record, not the call's; the
/// task is aborted when the call is dropped.
pub(crate) async fn prune_winner(
    ctx: &JudgeContext,
    tools: Arc<crate::tools::ToolContext>,
    outcome: &mut SearchOutcome,
) {
    prune_winner_with(ctx, Arc::new(route_judge::RegistryTools(tools)), outcome).await
}

/// [`prune_winner`] with the registry behind a [`ToolCaller`] (the tests script the
/// audit and the stitching planner).
pub(crate) async fn prune_winner_with(
    ctx: &JudgeContext,
    tools: Arc<dyn ToolCaller>,
    outcome: &mut SearchOutcome,
) {
    let started = std::time::Instant::now();
    let decision = match winner_of(ctx, outcome) {
        Err(reason) => Decision::of(PruneRecord::new("skipped", Some(reason))),
        Ok(winner) => {
            let index = winner.index;
            // What the task may write; on a panic only what was not there before goes.
            let new_paths: Vec<PathBuf> = [
                winner.routed.folder.join("pruned.ses"),
                ctx.root.join(pruned_folder(index)),
            ]
            .into_iter()
            .filter(|path| path.symlink_metadata().is_err())
            .collect();
            let task = tokio::spawn(decide(Arc::new(ctx.clone()), tools, winner));
            let _abort = AbortOnDrop(task.abort_handle());
            let decision = match task.await {
                Ok(decision) => decision,
                Err(error) => Decision {
                    record: PruneRecord::new("failed", Some(format!("the prune stopped: {error}"))),
                    kept: None,
                    written: new_paths,
                },
            };
            // 6. The loop's scratch folder, in every case.
            remove_own(&ctx.root.join(prune_folder(index)));
            decision
        }
    };
    let Decision {
        mut record,
        kept,
        written,
    } = decision;
    let winner = outcome.winner;
    match (kept, winner) {
        (Some((session, judged)), Some(winner)) => {
            match replace_winner(outcome, winner, session, judged) {
                Ok(before) => record.before = Some(before),
                Err(reason) => {
                    record = record.ended("rejected", reason);
                    written.iter().for_each(|path| remove_own(path));
                }
            }
        }
        _ => written.iter().for_each(|path| remove_own(path)),
    }
    record.seconds = round1(started.elapsed().as_secs_f64());
    outcome.pour_prune = Some(record);
}

#[cfg(test)]
#[path = "pour_prune_tests.rs"]
mod tests;
