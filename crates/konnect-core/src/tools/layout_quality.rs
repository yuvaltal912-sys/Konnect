//! `audit_layout_quality`: a routed board against the owner's PCB layout
//! checklist (`docs/CREATION_RESEARCH.md` §5, the checklist the owner keeps in
//! `target/creation-research/layout-checklist.md`).
//!
//! The checklist's core principle is that every important signal or power path
//! is designed together with its return path. This tool measures what geometry
//! can measure of that, from the saved board file alone: reference planes under
//! critical routes (§3), decoupling loops and copper sizing (§4), detours,
//! vias, parallel runs, corners and differential pairs (§5), and the hot loop
//! of a switching regulator (§8). Everything that needs a datasheet, a
//! simulation or a bench is listed as `needs_human`, and a check that cannot
//! run says why instead of scoring zero.
//!
//! The thresholds are the tool's arguments (`thresholds` in the result lists
//! every value used and where it came from), so a routing loop can hold them
//! fixed and compare runs by `quality.score` and `metrics`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use konnect_sexp::board_geometry::{BoardGeometry, Entry, FootprintGeom, Kind, PadGeom};
use konnect_sexp::parser::{parse_sexp, SexpNode};
use konnect_sexp::planar::{point_in_ring, Pt};
use serde_json::{json, Map, Value};

use crate::mcp::protocol::CallToolResult;
use crate::tool;
use crate::tools::design_intent::{
    design_intent_path, read_design_intent, DesignIntent, IntentRead, SCHEMA_VERSION,
};
use crate::tools::trace_current::{self, Copper};
use crate::tools::{get_path, invalid_arg, ToolContext, ToolDef};

#[path = "layout_quality_grid.rs"]
pub(crate) mod grid;
#[path = "layout_quality_planes.rs"]
mod planes;
#[path = "layout_quality_power.rs"]
mod power;
#[path = "layout_quality_routing.rs"]
mod routing;
#[cfg(test)]
#[path = "layout_quality_tests.rs"]
mod tests;

use grid::{Cover, Grid};

// ─── Thresholds ──────────────────────────────────────────────────────────────

/// One adjustable threshold: name, default, allowed range, what it limits.
struct Knob(&'static str, f64, f64, f64, &'static str);

const KNOBS: &[Knob] = &[
    Knob("sample_mm", 0.25, 0.05, 2.0, "§3 spacing of the samples taken along a critical track to see what lies on its reference layer"),
    Knob("min_reference_coverage", 0.90, 0.0, 1.0, "§3 least share of a critical net's length that must run over solid ground on an adjacent layer"),
    Knob("max_void_run_mm", 2.0, 0.1, 100.0, "§3 longest stretch of one track allowed over a void, slot or other-net plane"),
    Knob("stitch_via_max_mm", 3.0, 0.1, 50.0, "§3 farthest a ground stitching via may sit from a signal via that changes reference plane"),
    Knob("decap_search_mm", 15.0, 1.0, 100.0, "§4 how far from an IC supply pin a decoupling capacitor is looked for"),
    Knob("decap_max_loop_mm", 10.0, 1.0, 200.0, "§4 longest capacitor → pin → ground → capacitor copper path (both sides) for one supply pin"),
    Knob("decap_max_loop_area_mm2", 20.0, 0.1, 1000.0, "§4 largest decoupling loop area (plan view of the same path)"),
    Knob("hot_loop_max_mm", 12.0, 1.0, 200.0, "§8 longest copper path of a switching regulator's critical loop"),
    Knob("hot_loop_max_area_mm2", 25.0, 0.1, 1000.0, "§8 largest plan-view area of that loop"),
    Knob("max_switch_node_area_mm2", 30.0, 0.5, 1000.0, "§8 most copper (all layers) on a switch node"),
    Knob("min_switch_feedback_gap_mm", 1.0, 0.05, 20.0, "§8 least clearance between switch-node copper and a feedback route, away from the regulator's own pins"),
    Knob("bottleneck_min_current_a", 0.5, 0.0, 1000.0, "§4 smallest recorded current for which neck-downs are searched"),
    Knob("default_temp_rise_c", 10.0, 1.0, 100.0, "§4 temperature rise used for sizing when the record gives none"),
    Knob("max_detour_ratio", 1.5, 1.0, 20.0, "§5 routed length over the pin-to-pin minimum spanning tree"),
    Knob("max_vias_per_connection", 2.0, 0.0, 50.0, "§5 vias per pin-to-pin connection of one net"),
    Knob("parallel_max_gap_mm", 0.5, 0.05, 10.0, "§5 edge-to-edge gap under which two unrelated tracks count as running together"),
    Knob("parallel_max_run_mm", 8.0, 0.5, 500.0, "§5 longest run at that gap before it is reported"),
    Knob("diff_max_gap_spread", 1.5, 1.0, 20.0, "§5 largest gap over smallest gap along a differential pair"),
    Knob("diff_min_coupled_fraction", 0.8, 0.0, 1.0, "§5 least share of a pair's length that runs coupled"),
    Knob("raster_step_mm", 0.1, 0.02, 1.0, "resolution of the copper raster behind loop lengths, areas and neck-downs (grows on very large boards)"),
    Knob("via_cost_mm", 0.5, 0.0, 10.0, "path length counted for each layer change in a loop"),
];

pub(super) struct Params {
    values: BTreeMap<&'static str, (f64, bool)>,
    pub max_findings: usize,
    pub scope_all_signals: bool,
    pub extra_critical: Vec<String>,
    pub extra_ground: Vec<String>,
    pub extra_power: Vec<String>,
    pub feedback_nets: Vec<String>,
    pub regulators: Vec<(String, String)>,
}

impl Params {
    pub fn get(&self, name: &str) -> f64 {
        self.values.get(name).map(|v| v.0).unwrap_or(f64::NAN)
    }

    fn from_args(args: &Value) -> Result<Params, CallToolResult> {
        let mut values = BTreeMap::new();
        for Knob(name, default, min, max, _) in KNOBS {
            let (v, given) = match args.get(*name) {
                None | Some(Value::Null) => (*default, false),
                Some(raw) => match raw.as_f64() {
                    Some(v) if v.is_finite() && v >= *min && v <= *max => (v, true),
                    _ => {
                        return Err(invalid_arg(
                            name,
                            &format!("must be a number from {min} to {max}"),
                        ))
                    }
                },
            };
            values.insert(*name, (v, given));
        }
        let strings = |field: &str| -> Result<Vec<String>, CallToolResult> {
            match args.get(field) {
                None | Some(Value::Null) => Ok(Vec::new()),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|i| {
                        i.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| invalid_arg(field, "must be an array of strings"))
                    })
                    .collect(),
                Some(_) => Err(invalid_arg(field, "must be an array of strings")),
            }
        };
        let max_findings = match args.get("max_findings") {
            None | Some(Value::Null) => 40,
            Some(v) => match v.as_u64() {
                Some(n) if (1..=500).contains(&n) => n as usize,
                _ => return Err(invalid_arg("max_findings", "must be a whole number from 1 to 500")),
            },
        };
        let scope_all_signals = match args.get("signal_scope").and_then(Value::as_str) {
            None | Some("critical") => false,
            Some("all_signals") => true,
            Some(_) => return Err(invalid_arg("signal_scope", "must be 'critical' or 'all_signals'")),
        };
        let mut regulators = Vec::new();
        if let Some(list) = args.get("switching_regulators").filter(|v| !v.is_null()) {
            let Some(items) = list.as_array() else {
                return Err(invalid_arg("switching_regulators", "must be an array of {switch_net, topology}"));
            };
            for item in items {
                let net = item.get("switch_net").and_then(Value::as_str);
                let topo = item.get("topology").and_then(Value::as_str);
                match (net, topo) {
                    (Some(n), Some(t)) if t == "buck" || t == "boost" => {
                        regulators.push((n.to_string(), t.to_string()))
                    }
                    _ => {
                        return Err(invalid_arg(
                            "switching_regulators",
                            "each entry needs switch_net and topology ('buck' or 'boost')",
                        ))
                    }
                }
            }
        }
        Ok(Params {
            values,
            max_findings,
            scope_all_signals,
            extra_critical: strings("critical_nets")?,
            extra_ground: strings("ground_nets")?,
            extra_power: strings("power_nets")?,
            feedback_nets: strings("feedback_nets")?,
            regulators,
        })
    }

    fn to_json(&self) -> Value {
        let mut out = Map::new();
        for Knob(name, default, _, _, what) in KNOBS {
            let (v, given) = self.values[name];
            out.insert(
                name.to_string(),
                json!({ "value": v, "default": default, "source": if given { "argument" } else { "default" }, "limits": what }),
            );
        }
        Value::Object(out)
    }
}

// ─── Tool definition ─────────────────────────────────────────────────────────

pub fn tools() -> Vec<ToolDef> {
    let mut props = Map::new();
    props.insert("board".into(), json!({ "type": "string", "description": "Path to the routed .kicad_pcb file (saved zone fills are read, so refill and save first). The design-intent record and netclasses are read from its project folder" }));
    props.insert("signal_scope".into(), json!({ "type": "string", "enum": ["critical", "all_signals"], "description": "Which signals get the reference-plane checks: 'critical' (default; the record's interfaces and differential pairs, netclasses and the lists below, else every signal net over 5 mm, said in net_roles) or 'all_signals'" }));
    for (name, what) in [
        ("critical_nets", "Nets to treat as critical signals, added to those found"),
        ("ground_nets", "Nets to treat as ground, added to those found"),
        ("power_nets", "Nets to treat as power, added to those found"),
        ("feedback_nets", "Feedback nets of a switching regulator, added to those found from pin names"),
    ] {
        props.insert(name.into(), json!({ "type": "array", "items": { "type": "string" }, "description": what }));
    }
    props.insert("switching_regulators".into(), json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "switch_net": { "type": "string" },
                "topology": { "type": "string", "enum": ["buck", "boost"] }
            },
            "required": ["switch_net", "topology"],
            "additionalProperties": false
        },
        "description": "Name the topology of a switch node when the design-intent record cannot (it uses rails' source_ref). Otherwise an undecidable one is reported as needs_human"
    }));
    props.insert("max_findings".into(), json!({ "type": "integer", "minimum": 1, "maximum": 500, "description": "Findings listed, most severe first (default 40); counts cover all" }));
    for Knob(name, default, min, max, what) in KNOBS {
        props.insert(
            name.to_string(),
            json!({ "type": "number", "minimum": min, "maximum": max, "description": format!("{what} (default {default})") }),
        );
    }
    vec![tool!(
        "audit_layout_quality",
        "Score a routed board against the owner's PCB layout checklist: reference planes under \
         critical routes (voids, slots, plane splits, missing stitching vias, floating ground \
         islands), decoupling loops per IC supply pin, copper neck-downs on recorded currents, \
         detours, vias, stubs, 90°/acute corners, parallel runs and differential-pair \
         consistency, and a buck's input or boost's output hot loop with switch-node copper \
         near feedback. Nets are judged critical, power or ground from the design-intent \
         record and netclasses (net_roles says which and why). Every finding names the \
         checklist item, the measured value against the stated threshold (all adjustable) and \
         UUIDs; items needing datasheets, simulation or a bench are needs_human, and a check \
         that cannot run says why. quality.score (0–100) and metrics compare two routing runs \
         of the same board. Read-only, from the saved file.",
        json!({ "type": "object", "properties": props, "required": ["board"], "additionalProperties": false }),
        |args, ctx| async move { handle(args, ctx).await }
    )]
}

// ─── Roles ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(super) struct Rail {
    pub source_ref: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct CurrentReq {
    pub net: String,
    pub amps: f64,
    pub rise_c: f64,
    pub source: String,
}

#[derive(Debug, Default)]
pub(super) struct Roles {
    pub ground: BTreeMap<String, String>,
    pub power: BTreeMap<String, String>,
    pub critical: BTreeMap<String, String>,
    pub pairs: Vec<(String, String, String)>,
    pub rails: HashMap<String, Rail>,
    pub currents: Vec<CurrentReq>,
    pub netclass: HashMap<String, String>,
    pub intent: &'static str,
    pub critical_basis: &'static str,
}

fn tail(net: &str) -> String {
    let t = net.rsplit('/').next().unwrap_or(net);
    t.trim_start_matches(['+', '-']).to_uppercase()
}

pub(super) fn ground_by_name(net: &str) -> bool {
    let n = tail(net);
    n.starts_with("GND")
        || n.ends_with("GND")
        || n.starts_with("VSS")
        || n.starts_with("GROUND")
        || n == "0V"
}

pub(super) fn power_by_name(net: &str) -> bool {
    let n = tail(net);
    if ground_by_name(net) {
        return false;
    }
    let first_digit = n.chars().next().is_some_and(|c| c.is_ascii_digit());
    (first_digit && n.contains('V'))
        || ["VCC", "VDD", "VBAT", "VIN", "VBUS", "VSYS", "VOUT", "VREG", "VPP"]
            .iter()
            .any(|p| n.starts_with(p))
}

fn critical_by_name(net: &str) -> bool {
    let n = tail(net);
    ["CLK", "CLOCK", "SCK", "SCLK", "XTAL", "OSC", "USB", "CANH", "CANL", "LVDS", "ETH", "HDMI"]
        .iter()
        .any(|p| n.contains(p))
}

fn critical_class(class: &str) -> bool {
    let c = class.to_lowercase();
    ["clk", "clock", "crit", "analog", "sens", "rf", "speed", "diff", "usb", "can", "eth", "lvds", "ddr"]
        .iter()
        .any(|p| c.contains(p))
}

fn power_class(class: &str) -> bool {
    let c = class.to_lowercase();
    ["power", "pwr", "supply", "hv", "motor"].iter().any(|p| c.contains(p))
}

fn glob(pattern: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') => (0..=t.len()).any(|i| go(&p[1..], &t[i..])),
            Some('?') => !t.is_empty() && go(&p[1..], &t[1..]),
            Some(c) => t.first() == Some(c) && go(&p[1..], &t[1..]),
        }
    }
    go(
        &pattern.chars().collect::<Vec<_>>(),
        &text.chars().collect::<Vec<_>>(),
    )
}

fn read_netclasses(board: &Path, nets: &BTreeSet<String>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let pro = board.with_extension("kicad_pro");
    let Ok(text) = std::fs::read_to_string(pro) else {
        return out;
    };
    let Ok(json) = serde_json::from_str::<Value>(&text) else {
        return out;
    };
    let settings = &json["net_settings"];
    if let Some(patterns) = settings["netclass_patterns"].as_array() {
        for p in patterns {
            let (Some(class), Some(pattern)) = (p["netclass"].as_str(), p["pattern"].as_str()) else {
                continue;
            };
            for net in nets {
                if glob(pattern, net) {
                    out.entry(net.clone()).or_insert_with(|| class.to_string());
                }
            }
        }
    }
    if let Some(map) = settings["netclass_assignments"].as_object() {
        for (net, class) in map {
            let class = class
                .as_str()
                .or_else(|| class.as_array().and_then(|a| a.first()).and_then(Value::as_str));
            if let Some(class) = class {
                out.entry(net.clone()).or_insert_with(|| class.to_string());
            }
        }
    }
    out.retain(|_, c| c != "Default");
    out
}

// ─── Zone fills ──────────────────────────────────────────────────────────────

pub(super) struct Fill {
    pub zone_uuid: Option<String>,
    pub net: Option<String>,
    pub layer: String,
    pub rings: Vec<Ring>,
}

pub(super) struct Ring {
    pub pts: Vec<Pt>,
    pub bbox: (f64, f64, f64, f64),
}

pub(super) struct UnfilledZone {
    pub uuid: Option<String>,
    pub net: Option<String>,
    pub layers: Vec<String>,
}

fn xy_list(pts: &SexpNode) -> Vec<Pt> {
    pts.children()
        .unwrap_or(&[])
        .iter()
        .filter(|n| n.head() == Some("xy"))
        .filter_map(|n| Some((n.get_f64(1)?, n.get_f64(2)?)))
        .collect()
}

fn read_fills(tree: &SexpNode, geometry: &BoardGeometry) -> (Vec<Fill>, Vec<UnfilledZone>) {
    let zone_info: HashMap<&str, &Entry> = geometry
        .others
        .iter()
        .filter(|e| e.kind == Kind::Zone)
        .filter_map(|e| Some((e.uuid.as_deref()?, e)))
        .collect();
    let mut fills = Vec::new();
    let mut unfilled = Vec::new();
    for zone in tree.find_all("zone") {
        if zone.find("keepout").is_some() || zone.find("rule_area").is_some() {
            continue;
        }
        let uuid = zone.find_str("uuid").map(str::to_string);
        let entry = uuid.as_deref().and_then(|u| zone_info.get(u).copied());
        let net = entry.and_then(|e| e.net.clone());
        if entry.is_some_and(|e| e.form == "keepout") {
            continue;
        }
        let is_filled = zone
            .find("fill")
            .and_then(|f| f.get(1))
            .and_then(SexpNode::as_str)
            == Some("yes");
        let mut by_layer: BTreeMap<String, Vec<Ring>> = BTreeMap::new();
        for fp in zone.find_all("filled_polygon") {
            let (Some(layer), Some(pts)) = (fp.find_str("layer"), fp.find("pts")) else {
                continue;
            };
            let pts = xy_list(pts);
            if let Some(bbox) = konnect_sexp::planar::bbox_of(&pts) {
                if pts.len() >= 3 {
                    by_layer
                        .entry(layer.to_string())
                        .or_default()
                        .push(Ring { pts, bbox });
                }
            }
        }
        if by_layer.is_empty() && !is_filled {
            unfilled.push(UnfilledZone {
                uuid: uuid.clone(),
                net: net.clone(),
                layers: entry.map(|e| e.layers.clone()).unwrap_or_default(),
            });
        }
        for (layer, rings) in by_layer {
            fills.push(Fill {
                zone_uuid: uuid.clone(),
                net: net.clone(),
                layer,
                rings,
            });
        }
    }
    (fills, unfilled)
}

fn copper_layers(tree: &SexpNode) -> Vec<String> {
    let mut names: Vec<String> = tree
        .find("layers")
        .and_then(SexpNode::children)
        .map(|c| {
            c.iter()
                .skip(1)
                .filter_map(|l| l.get(1).and_then(SexpNode::as_str))
                .filter(|n| n.ends_with(".Cu"))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let rank = |n: &str| -> i64 {
        match n {
            "F.Cu" => -1,
            "B.Cu" => i64::MAX,
            _ => n
                .trim_start_matches("In")
                .trim_end_matches(".Cu")
                .parse::<i64>()
                .unwrap_or(1000),
        }
    };
    names.sort_by_key(|n| rank(n));
    names
}

// ─── The model every check reads ─────────────────────────────────────────────

pub(super) type PadIx = (usize, usize);

pub(super) struct Model<'a> {
    pub geo: &'a BoardGeometry,
    pub layers: Vec<String>,
    pub fills: Vec<Fill>,
    pub unfilled: Vec<UnfilledZone>,
    pub roles: Roles,
    pub params: &'a Params,
    pub copper: Copper,
    pub tracks: HashMap<String, Vec<usize>>,
    pub vias: HashMap<String, Vec<usize>>,
    pub pads: HashMap<String, Vec<PadIx>>,
    pub conn: std::cell::OnceCell<HashMap<String, routing::Conn>>,
}

impl<'a> Model<'a> {
    pub fn layer_ix(&self, name: &str) -> Option<usize> {
        self.layers.iter().position(|l| l == name)
    }

    pub fn pad(&self, (f, p): PadIx) -> (&FootprintGeom, &PadGeom) {
        let fp = &self.geo.footprints[f];
        (fp, &fp.pads[p])
    }

    pub fn track(&self, i: usize) -> &Entry {
        &self.geo.others[i]
    }

    pub fn is_ground(&self, net: &str) -> bool {
        self.roles.ground.contains_key(net)
    }

    pub fn is_power(&self, net: &str) -> bool {
        self.roles.power.contains_key(net)
    }

    pub fn thickness(&self, layer: &str) -> f64 {
        self.copper.thickness(layer).0
    }

    /// The layers a via spans, as indices (inclusive).
    pub fn via_span(&self, via: &Entry) -> Option<(usize, usize)> {
        let ix: Vec<usize> = via.layers.iter().filter_map(|l| self.layer_ix(l)).collect();
        Some((*ix.iter().min()?, *ix.iter().max()?))
    }

    /// The copper layers a pad has copper on, as indices.
    pub fn pad_layers(&self, pad: &PadGeom) -> Vec<usize> {
        if pad.through_hole {
            return (0..self.layers.len()).collect();
        }
        self.layers
            .iter()
            .enumerate()
            .filter(|(_, l)| pad.on_layer(l))
            .map(|(i, _)| i)
            .collect()
    }

    /// Bounding box of a net's copper, `margin` added.
    pub fn net_bbox(&self, net: &str, margin: f64) -> Option<(f64, f64, f64, f64)> {
        let mut pts: Vec<Pt> = Vec::new();
        let mut grow = |b: (f64, f64, f64, f64)| {
            pts.push((b.0, b.1));
            pts.push((b.2, b.3));
        };
        for &i in self.tracks.get(net).into_iter().flatten() {
            if let Some(b) = self.track(i).shape.bbox() {
                grow(b);
            }
        }
        for &i in self.vias.get(net).into_iter().flatten() {
            if let Some(b) = self.track(i).shape.bbox() {
                grow(b);
            }
        }
        for &pi in self.pads.get(net).into_iter().flatten() {
            if let Some(b) = self.pad(pi).1.shape.bbox() {
                grow(b);
            }
        }
        for f in self.fills.iter().filter(|f| f.net.as_deref() == Some(net)) {
            for r in &f.rings {
                grow(r.bbox);
            }
        }
        let b = konnect_sexp::planar::bbox_of(&pts)?;
        Some((b.0 - margin, b.1 - margin, b.2 + margin, b.3 + margin))
    }

    /// Rasterise every copper item of `net` inside `window`.
    pub fn net_grid(&self, net: &str, window: (f64, f64, f64, f64), cover: Cover) -> Grid {
        let step = self.params.get("raster_step_mm");
        let mut g = Grid::new(window, step, self.layers.clone());
        let inside = |b: (f64, f64, f64, f64)| {
            b.0 <= window.2 && b.2 >= window.0 && b.1 <= window.3 && b.3 >= window.1
        };
        for &i in self.tracks.get(net).into_iter().flatten() {
            let t = self.track(i);
            let (Some(b), Some(l)) = (t.shape.bbox(), t.layers.first().and_then(|l| self.layer_ix(l))) else {
                continue;
            };
            if inside(b) {
                g.add_path(l, &t.shape.pts, t.shape.radius, cover);
            }
        }
        for &i in self.vias.get(net).into_iter().flatten() {
            let v = self.track(i);
            let (Some(b), Some((lo, hi))) = (v.shape.bbox(), self.via_span(v)) else {
                continue;
            };
            if inside(b) {
                g.add_link(v.at, v.shape.radius, lo, hi);
            }
        }
        for &pi in self.pads.get(net).into_iter().flatten() {
            let (_, pad) = self.pad(pi);
            let Some(b) = pad.shape.bbox() else { continue };
            if !inside(b) {
                continue;
            }
            let layers = self.pad_layers(pad);
            for &l in &layers {
                g.add_shape(l, &pad.shape, cover);
            }
            if pad.through_hole && layers.len() > 1 {
                let r = (pad.size.0.min(pad.size.1) / 2.0).max(g.step);
                g.add_link(pad.at, r, layers[0], *layers.last().unwrap_or(&0));
            }
        }
        for f in self.fills.iter().filter(|f| f.net.as_deref() == Some(net)) {
            let Some(l) = self.layer_ix(&f.layer) else { continue };
            for r in &f.rings {
                if inside(r.bbox) {
                    g.add_ring(l, &r.pts);
                }
            }
        }
        g
    }

    /// Which fill covers `p` on `layer`, as an index into `fills`.
    pub fn fill_at(&self, layer: &str, p: Pt) -> Option<usize> {
        self.fills.iter().position(|f| {
            f.layer == layer
                && f.rings.iter().any(|r| {
                    p.0 >= r.bbox.0 && p.0 <= r.bbox.2 && p.1 >= r.bbox.1 && p.1 <= r.bbox.3
                        && point_in_ring(p, &r.pts)
                })
        })
    }

    pub fn fp_ref<'b>(&self, fp: &'b FootprintGeom) -> &'b str {
        fp.reference.as_deref().unwrap_or("?")
    }
}

pub(super) fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

pub(super) fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

pub(super) fn path_len(pts: &[Pt]) -> f64 {
    pts.windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum()
}

pub(super) fn is_ic(reference: &str) -> bool {
    reference.starts_with('U') || reference.starts_with("IC")
}

fn prefix_digit(reference: &str, prefix: &str) -> bool {
    reference
        .strip_prefix(prefix)
        .and_then(|r| r.chars().next())
        .is_some_and(|c| c.is_ascii_digit())
}

pub(super) fn is_cap(reference: &str) -> bool {
    prefix_digit(reference, "C")
}

pub(super) fn is_inductor(reference: &str) -> bool {
    prefix_digit(reference, "L")
}

pub(super) fn is_mosfet(reference: &str) -> bool {
    prefix_digit(reference, "Q") || prefix_digit(reference, "M")
}

pub(super) fn is_diode(reference: &str) -> bool {
    prefix_digit(reference, "D")
}

pub(super) fn dist(a: Pt, b: Pt) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

// ─── Check output ────────────────────────────────────────────────────────────

pub(super) struct Finding {
    pub severity: &'static str,
    pub item: &'static str,
    pub subject: String,
    pub measured: Value,
    pub threshold: Value,
    pub issue: String,
    pub fix: String,
    pub uuids: Vec<String>,
    pub at: Option<Pt>,
}

impl Finding {
    fn to_json(&self) -> Value {
        json!({
            "severity": self.severity,
            "checklist": checklist(self.item),
            "item": self.item,
            "subject": self.subject,
            "measured": self.measured,
            "threshold": self.threshold,
            "issue": self.issue,
            "recommendation": self.fix,
            "uuids": self.uuids,
            "at": self.at.map(|p| json!({ "x": r3(p.0), "y": r3(p.1) })),
        })
    }
}

pub(super) fn checklist(item: &str) -> &'static str {
    match item {
        "3.1" => "§3 Uninterrupted adjacent reference plane under critical routes; no crossing of plane splits, slots, voids",
        "3.2" => "§3 Stitching via near a layer change between different reference planes",
        "3.3" => "§3 Ground pours actually connected",
        "3.4" => "§3/§4 No narrow ground bottleneck on a high-current return",
        "4.1" => "§4 Minimise the whole capacitor → IC → ground → capacitor loop (ground side too)",
        "4.2" => "§4 Size traces, pours and vias for current",
        "4.3" => "§4 Inspect bottlenecks: pads, neck-downs, thermal-relief spokes, layer transitions",
        "5.1" => "§5 Direct routes, few branches, vias and detours",
        "5.2" => "§5 Reduce long parallel runs between unrelated signals",
        "5.3" => "§5 Acute and 90° corners",
        "5.4" => "§5 Differential pairs: consistent spacing, reference and transitions; no stubs",
        "8.1" => "§8 Minimise the loop carrying fast-changing current (buck input loop, boost output loop)",
        "8.2" => "§8 Compact switch-node copper; feedback and sensitive routes away from it",
        _ => "",
    }
}

pub(super) struct CheckOut {
    pub id: &'static str,
    pub name: &'static str,
    /// `checked`, `partial` or `not_run`.
    pub status: &'static str,
    pub reason: Option<String>,
    /// 0..=1 when the check measured something to score.
    pub score: Option<f64>,
    pub metrics: Value,
    pub findings: Vec<Finding>,
    pub needs_human: Vec<(&'static str, String)>,
}

impl CheckOut {
    pub fn not_run(id: &'static str, name: &'static str, reason: impl Into<String>) -> Self {
        CheckOut {
            id,
            name,
            status: "not_run",
            reason: Some(reason.into()),
            score: None,
            metrics: Value::Null,
            findings: Vec::new(),
            needs_human: Vec::new(),
        }
    }

    pub fn checked(id: &'static str, name: &'static str) -> Self {
        CheckOut {
            id,
            name,
            status: "checked",
            reason: None,
            score: None,
            metrics: Value::Null,
            findings: Vec::new(),
            needs_human: Vec::new(),
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "item": self.id,
            "name": self.name,
            "checklist": checklist(self.id),
            "status": self.status,
            "reason": self.reason,
            "score": self.score.map(|s| r3(s)),
            "metrics": self.metrics,
            "findings": self.findings.len(),
        })
    }
}

/// 1 up to half the limit, 0.5 at the limit, 0 at twice the limit.
pub(super) fn margin_score(value: f64, limit: f64) -> f64 {
    let r = value / limit.max(1e-9);
    if r <= 0.5 {
        1.0
    } else if r <= 1.0 {
        1.0 - (r - 0.5)
    } else if r <= 2.0 {
        0.5 - 0.5 * (r - 1.0)
    } else {
        0.0
    }
}

/// 1 up to the limit, then falling to 0 at twice the limit.
pub(super) fn excess_score(value: f64, limit: f64) -> f64 {
    if value <= limit {
        1.0
    } else {
        (1.0 - (value - limit) / limit.max(1e-9)).clamp(0.0, 1.0)
    }
}

// ─── Assembly ────────────────────────────────────────────────────────────────

fn intent_roles(
    board: &Path,
    nets: &BTreeSet<String>,
    params: &Params,
    pairs_out: &mut Vec<(String, String, String)>,
) -> (Option<DesignIntent>, Roles) {
    let mut roles = Roles {
        intent: "absent",
        ..Roles::default()
    };
    let project_dir = board.parent().unwrap_or(Path::new("."));
    let mut intent = None;
    match read_design_intent(&design_intent_path(project_dir)) {
        IntentRead::Loaded { intent: i, .. } => {
            roles.intent = "loaded";
            intent = Some(*i);
        }
        IntentRead::Unusable { .. } => roles.intent = "unusable",
        IntentRead::Absent => {}
    }
    roles.netclass = read_netclasses(board, nets);
    let rise = params.get("default_temp_rise_c");
    if let Some(i) = &intent {
        for rail in &i.rails {
            for net in &rail.nets {
                roles.rails.insert(
                    net.clone(),
                    Rail {
                        source_ref: rail.source_ref.clone(),
                    },
                );
                let src = format!("design_intent rail '{}' at {} V", rail.name, rail.voltage_v);
                if rail.voltage_v == 0.0 {
                    roles.ground.insert(net.clone(), src);
                } else {
                    roles.power.insert(net.clone(), src);
                }
                if let Some(a) = rail.max_current_a {
                    roles.currents.push(CurrentReq {
                        net: net.clone(),
                        amps: a,
                        rise_c: rail.max_temp_rise_c.unwrap_or(rise),
                        source: format!("rail:{}", rail.name),
                    });
                }
            }
        }
        for c in &i.currents {
            roles.currents.push(CurrentReq {
                net: c.net.clone(),
                amps: c.max_current_a,
                rise_c: c.max_temp_rise_c.unwrap_or(rise),
                source: "currents".into(),
            });
        }
        for itf in &i.interfaces {
            for net in &itf.nets {
                roles
                    .critical
                    .entry(net.clone())
                    .or_insert_with(|| format!("design_intent interface '{}' ({})", itf.name, itf.kind));
            }
            for p in &itf.differential_pairs {
                for net in [&p.positive, &p.negative] {
                    roles
                        .critical
                        .entry(net.clone())
                        .or_insert_with(|| format!("design_intent pair of '{}'", itf.name));
                }
                pairs_out.push((
                    p.positive.clone(),
                    p.negative.clone(),
                    format!("design_intent interface '{}'", itf.name),
                ));
            }
        }
    }
    for net in nets {
        if let Some(class) = roles.netclass.get(net) {
            if power_class(class) && !roles.ground.contains_key(net) {
                roles
                    .power
                    .entry(net.clone())
                    .or_insert_with(|| format!("netclass '{class}'"));
            } else if critical_class(class) {
                roles
                    .critical
                    .entry(net.clone())
                    .or_insert_with(|| format!("netclass '{class}'"));
            }
        }
        if ground_by_name(net) && !roles.power.contains_key(net) {
            roles
                .ground
                .entry(net.clone())
                .or_insert_with(|| "name".into());
        } else if power_by_name(net) && !roles.ground.contains_key(net) {
            roles.power.entry(net.clone()).or_insert_with(|| "name".into());
        }
    }
    for (list, ground, power, critical) in [
        (&params.extra_ground, true, false, false),
        (&params.extra_power, false, true, false),
        (&params.extra_critical, false, false, true),
    ] {
        for net in list {
            if ground {
                roles.power.remove(net);
                roles.ground.insert(net.clone(), "argument".into());
            } else if power {
                roles.ground.remove(net);
                roles.power.insert(net.clone(), "argument".into());
            } else if critical {
                roles.critical.insert(net.clone(), "argument".into());
            }
        }
    }
    (intent, roles)
}

fn handle_sync(args: &Value) -> anyhow::Result<CallToolResult> {
    let board = get_path(args, "board")?;
    let params = match Params::from_args(args) {
        Ok(p) => p,
        Err(refusal) => return Ok(refusal),
    };
    audit(&board, &params)
}

async fn handle(args: &Value, _ctx: &ToolContext) -> anyhow::Result<CallToolResult> {
    let args = args.clone();
    tokio::task::spawn_blocking(move || handle_sync(&args)).await?
}

pub(super) fn audit(board: &Path, params: &Params) -> anyhow::Result<CallToolResult> {
    let content = std::fs::read_to_string(board)?;
    let tree = parse_sexp(&content)?;
    let geometry = BoardGeometry::from_tree(&tree);
    let layers = copper_layers(&tree);
    if layers.is_empty() {
        return Ok(CallToolResult::text(serde_json::to_string(&json!({
            "audit": "layout_quality",
            "status": "not_run",
            "reason": "the board file lists no copper layers",
        }))?));
    }
    let (fills, unfilled) = read_fills(&tree, &geometry);

    let mut tracks: HashMap<String, Vec<usize>> = HashMap::new();
    let mut vias: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, e) in geometry.others.iter().enumerate() {
        let Some(net) = e.net.clone().filter(|n| !n.is_empty()) else {
            continue;
        };
        match e.kind {
            Kind::Track => tracks.entry(net).or_default().push(i),
            Kind::Via => vias.entry(net).or_default().push(i),
            _ => {}
        }
    }
    let mut pads: HashMap<String, Vec<PadIx>> = HashMap::new();
    for (fi, f) in geometry.footprints.iter().enumerate() {
        for (pi, p) in f.pads.iter().enumerate() {
            if let Some(net) = p.net.clone().filter(|n| !n.is_empty()) {
                pads.entry(net).or_default().push((fi, pi));
            }
        }
    }
    let nets: BTreeSet<String> = tracks
        .keys()
        .chain(vias.keys())
        .chain(pads.keys())
        .cloned()
        .collect();

    let mut pairs = Vec::new();
    let (intent, mut roles) = intent_roles(board, &nets, params, &mut pairs);
    let empty_intent: DesignIntent = serde_json::from_value(json!({ "schema_version": SCHEMA_VERSION }))?;
    let copper = trace_current::copper_for(&tree, intent.as_ref().unwrap_or(&empty_intent));

    // Critical signals: the record and netclasses first; failing that, every
    // signal net long enough to matter, and the result says so.
    let is_signal = |n: &str| !roles.ground.contains_key(n) && !roles.power.contains_key(n);
    for net in &nets {
        if is_signal(net) && critical_by_name(net) && !roles.critical.contains_key(net) {
            roles.critical.insert(net.clone(), "name heuristic".into());
        }
    }
    roles.critical.retain(|n, _| nets.contains(n));
    let basis: &'static str;
    if params.scope_all_signals || roles.critical.is_empty() {
        let mut added = 0;
        for net in &nets {
            let len: f64 = tracks
                .get(net)
                .map(|v| v.iter().map(|&i| path_len(&geometry.others[i].shape.pts)).sum())
                .unwrap_or(0.0);
            let n_pads = pads.get(net).map_or(0, Vec::len);
            if is_signal(net) && n_pads >= 2 && len >= 5.0 && !roles.critical.contains_key(net) {
                roles.critical.insert(
                    net.clone(),
                    if params.scope_all_signals {
                        "signal_scope all_signals".into()
                    } else {
                        "fallback: no critical nets recorded, so every signal net over 5 mm".into()
                    },
                );
                added += 1;
            }
        }
        basis = if params.scope_all_signals {
            "all_signals"
        } else if added > 0 {
            "fallback_all_signals"
        } else {
            "none"
        };
    } else {
        basis = "recorded_or_named";
    }
    roles.critical_basis = basis;
    roles.pairs = pairs;
    let positives: BTreeSet<String> = roles.pairs.iter().map(|p| p.0.clone()).collect();
    if roles.pairs.is_empty() {
        // Name-based pairs: X_P/X_N, X+/X-, DP/DM never inferred silently.
        for net in &nets {
            let mate = net
                .strip_suffix("_P")
                .map(|s| format!("{s}_N"))
                .or_else(|| net.strip_suffix('+').map(|s| format!("{s}-")))
                .or_else(|| net.strip_suffix("P").filter(|s| s.ends_with(['_', '/'])).map(|s| format!("{s}N")));
            if let Some(m) = mate {
                if nets.contains(&m) && !positives.contains(net) && is_signal(net) {
                    roles.pairs.push((net.clone(), m, "name heuristic".into()));
                }
            }
        }
    }

    let model = Model {
        geo: &geometry,
        layers,
        fills,
        unfilled,
        roles,
        params,
        copper,
        tracks,
        vias,
        pads,
        conn: std::cell::OnceCell::new(),
    };

    let mut checks: Vec<CheckOut> = Vec::new();
    let (plane_checks, refs) = planes::run(&model);
    checks.extend(plane_checks);
    checks.extend(power::run(&model, board, &tree));
    checks.extend(routing::run(&model, &refs));

    Ok(CallToolResult::text(serde_json::to_string(&assemble(
        board, &model, checks, &geometry,
    ))?))
}

/// Category weights of the overall score; a category with nothing measured is
/// left out and the rest renormalised.
const CATEGORIES: &[(&str, f64, &[(&str, f64)])] = &[
    ("return_path", 0.25, &[("3.1", 0.5), ("3.2", 0.2), ("3.3", 0.2), ("3.4", 0.1)]),
    ("decoupling_and_current", 0.25, &[("4.1", 0.5), ("4.2", 0.25), ("4.3", 0.25)]),
    ("switching_loops", 0.15, &[("8.1", 0.7), ("8.2", 0.3)]),
    ("routing", 0.35, &[("5.1", 0.3), ("5.2", 0.2), ("5.3", 0.3), ("5.4", 0.2)]),
];

fn assemble(board: &Path, m: &Model, checks: Vec<CheckOut>, geometry: &BoardGeometry) -> Value {
    let by_id: HashMap<&str, &CheckOut> = checks.iter().map(|c| (c.id, c)).collect();
    let mut categories = Vec::new();
    let mut weighted = 0.0;
    let mut total_weight = 0.0;
    let mut not_scored: Vec<Value> = Vec::new();
    for (name, weight, members) in CATEGORIES {
        let (mut sum, mut wsum) = (0.0, 0.0);
        let mut used = Vec::new();
        for (id, w) in *members {
            match by_id.get(id).and_then(|c| c.score.map(|s| (c, s))) {
                Some((_, s)) => {
                    sum += s * w;
                    wsum += w;
                    used.push(json!({ "item": id, "weight": w }));
                }
                None => not_scored.push(json!({
                    "item": id,
                    "reason": by_id.get(id).and_then(|c| c.reason.clone()).unwrap_or_else(|| "nothing to measure on this board".into()),
                })),
            }
        }
        if wsum > 0.0 {
            let score = sum / wsum;
            weighted += score * weight;
            total_weight += weight;
            categories.push(json!({ "category": name, "weight": weight, "score": r3(score * 100.0), "checks": used }));
        } else {
            categories.push(json!({ "category": name, "weight": weight, "score": Value::Null, "checks": [] }));
        }
    }
    let copper_quality = (total_weight > 0.0).then(|| r2(weighted / total_weight * 100.0));

    // Completion is a gate: unrouted nets make two scores incomparable, and a board with nets
    // left unrouted must not out-score one that routed them, so the score is scaled by the
    // share of multi-pad nets that are joined.
    let completion = routing::completion(m);
    let routed_share = {
        let total = completion["nets_with_two_or_more_pads"].as_f64().unwrap_or(0.0);
        let open = completion["unrouted_nets"].as_f64().unwrap_or(0.0);
        if total > 0.0 { (total - open) / total } else { 1.0 }
    };
    let score = copper_quality.map(|q| r2(q * routed_share));

    let mut findings: Vec<&Finding> = checks.iter().flat_map(|c| &c.findings).collect();
    let rank = |s: &str| match s {
        "error" => 0,
        "warning" => 1,
        _ => 2,
    };
    findings.sort_by(|a, b| {
        rank(a.severity)
            .cmp(&rank(b.severity))
            .then(a.item.cmp(b.item))
            .then(a.subject.cmp(&b.subject))
    });
    let count = |sev: &str| findings.iter().filter(|f| f.severity == sev).count();
    let (errors, warnings, infos) = (count("error"), count("warning"), count("info"));
    let listed: Vec<Value> = findings
        .iter()
        .take(m.params.max_findings)
        .map(|f| f.to_json())
        .collect();

    let mut metrics = Map::new();
    for c in &checks {
        if let Some(obj) = c.metrics.get("compare").and_then(Value::as_object) {
            for (k, v) in obj {
                metrics.insert(format!("{}.{k}", c.id), v.clone());
            }
        }
    }

    let mut needs_human: Vec<Value> = Vec::new();
    let mut seen = BTreeSet::new();
    for c in &checks {
        for (item, why) in &c.needs_human {
            if seen.insert((item.to_string(), why.clone())) {
                needs_human.push(json!({ "item": item, "why": why }));
            }
        }
    }
    for (item, why) in [
        ("5.5", "Controlled impedance: needs the fab stackup's dielectric constants and heights and the part's impedance target"),
        ("5.6", "Rise and fall time decide whether a stub or length is a transmission line: needs the driver's datasheet"),
        ("5.7", "Source termination value and place: needs the driver and interconnect"),
        ("4.4", "Capacitor DC-bias derating, ESR and ripple rating: needs the capacitor datasheet"),
        ("3.5", "Isolation barriers and antenna keepouts: need the intent behind each cut"),
        ("8.3", "Gate drive, Kelvin sensing, bootstrap and recirculation paths: need the schematic's intent and the parts' datasheets"),
        ("11", "Everything in §11 prototype validation: needs a bench"),
    ] {
        needs_human.push(json!({ "item": item, "why": why }));
    }

    let roles = &m.roles;
    let listed_nets = |map: &BTreeMap<String, String>| -> Value {
        let mut out = Map::new();
        for (n, s) in map.iter().take(60) {
            out.insert(n.clone(), json!(s));
        }
        json!({ "count": map.len(), "nets": out, "listed": map.len().min(60) })
    };
    let stats = json!({
        "footprints": geometry.footprints.len(),
        "pads": m.pads.values().map(Vec::len).sum::<usize>(),
        "tracks": m.tracks.values().map(Vec::len).sum::<usize>(),
        "vias": m.vias.values().map(Vec::len).sum::<usize>(),
        "zones_with_fill": m.fills.iter().filter_map(|f| f.zone_uuid.as_ref()).collect::<BTreeSet<_>>().len(),
        "zones_unfilled": m.unfilled.len(),
        "copper_layers": m.layers,
    });

    json!({
        "audit": "layout_quality",
        "status": "checked",
        "board": board.display().to_string(),
        "summary": format!(
            "Layout quality {} / 100 over {} categor{} ({:.0}% of the weight); {} error(s), {} warning(s), {} info; routing {}.",
            score.map(|s| format!("{s:.1}")).unwrap_or_else(|| "n/a".into()),
            categories.iter().filter(|c| !c["score"].is_null()).count(),
            if categories.iter().filter(|c| !c["score"].is_null()).count() == 1 { "y" } else { "ies" },
            total_weight * 100.0,
            errors, warnings, infos,
            if completion["complete"] == true { "complete" } else { "INCOMPLETE (scores are not comparable with a complete run)" },
        ),
        "quality": {
            "score": score,
            "scale": "0 to 100; each check scores 0 to 1 against its stated threshold, categories are weighted means, copper_quality is the weighted mean of the categories that measured something, and score is copper_quality times the share of multi-pad nets that are routed",
            "copper_quality": copper_quality,
            "routed_share": r3(routed_share),
            "weight_measured": r2(total_weight),
            "categories": categories,
            "not_scored": not_scored,
            "comparable_when": "same board, same thresholds, same net_roles, and both runs complete",
        },
        "completion": completion,
        "metrics": metrics,
        "net_roles": {
            "design_intent": roles.intent,
            "critical_basis": roles.critical_basis,
            "ground": listed_nets(&roles.ground),
            "power": listed_nets(&roles.power),
            "critical": listed_nets(&roles.critical),
            "differential_pairs": roles.pairs.iter().map(|(p, n, s)| json!({ "positive": p, "negative": n, "source": s })).collect::<Vec<_>>(),
            "netclass_assigned_nets": roles.netclass.len(),
            "recorded_currents": roles.currents.len(),
        },
        "thresholds": m.params.to_json(),
        "checks": checks.iter().map(CheckOut::to_json).collect::<Vec<_>>(),
        "counts": { "error": errors, "warning": warnings, "info": infos, "findings_listed": listed.len(), "findings_total": findings.len() },
        "findings": listed,
        "needs_human": needs_human,
        "not_measured": [
            "plan-view loop areas stand in for 3-D loop inductance (layer separation is not weighed)",
            "parallel tracks and pours are not summed when a neck-down is searched (the best single path is judged)",
            "broadside coupling between adjacent layers",
            "zones that are not saved filled",
        ],
        "board_stats": stats,
        "items_unreadable": geometry.skipped,
    })
}

