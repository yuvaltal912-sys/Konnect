//! `audit_trace_current`: routed copper against the currents recorded in the
//! design-intent record (Q3).
//!
//! Every track segment and via of a net with a recorded current is checked
//! against the IPC-2221 capacity for that current at the allowed temperature
//! rise. The model is deliberately conservative and says so: each item is
//! assumed to carry the net's full current, because finding how current
//! splits between parallel paths needs sources and sinks the record does not
//! hold. Zones on the net are named, since a pour can carry what a narrow
//! track cannot.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use konnect_sexp::board_geometry::{BoardGeometry, Entry, Kind};
use konnect_sexp::parser::{parse_sexp, SexpNode};
use serde_json::{json, Value};

use crate::mcp::protocol::CallToolResult;
use crate::tool;
use crate::tools::design_intent::{design_intent_path, read_design_intent, unusable, IntentRead};
use crate::tools::{get_path, invalid_arg, ToolContext, ToolDef};

/// IPC-2221 constants: I = k · ΔT^0.44 · A^0.725, A in square mils.
const K_EXTERNAL: f64 = 0.048;
const K_INTERNAL: f64 = 0.024;
const DT_EXPONENT: f64 = 0.44;
const AREA_EXPONENT: f64 = 0.725;
const MM_PER_MIL: f64 = 0.0254;
/// 1 oz/ft² of copper, in mm.
pub(crate) const MM_PER_OZ: f64 = 0.0348;
/// The plated barrel of a via, in mm (IPC-6012 class 2 average is 20 µm;
/// 25 µm is the common fab default).
const VIA_PLATING_MM: f64 = 0.025;
/// Used when neither the record nor the call gives a temperature rise.
pub(crate) const DEFAULT_TEMP_RISE_C: f64 = 10.0;
/// How many undersized segments a net lists by UUID.
const LISTED_ITEMS: usize = 10;

pub fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "audit_trace_current",
        "Check routed copper against the currents in the design-intent record \
         (get_design_intent: 'currents', and 'rails' with max_current_a). For each such net, \
         every track segment and via is compared with the IPC-2221 capacity at the net's \
         max_temp_rise_c (else default_temp_rise_c), using copper thickness from the record's \
         fab target, else the board stackup, else 1 oz outer / 0.5 oz inner. Conservative: \
         every item is assumed to carry the net's full current, so a track in parallel with \
         a pour or another track can be flagged although it shares the load; zones on the net \
         are listed for that reason. Reports undersized segments by UUID, the narrowest \
         segment per net, the vias needed in parallel, recorded nets missing from the board, \
         and what was not checked. Read-only.",
        json!({
            "type": "object",
            "properties": {
                "board": { "type": "string", "description": "Path to .kicad_pcb file; the record is read from its project directory" },
                "default_temp_rise_c": {
                    "type": "number",
                    "exclusiveMinimum": 0,
                    "maximum": 100,
                    "description": "Allowed temperature rise in °C for nets whose record gives none (default 10)"
                }
            },
            "required": ["board"],
            "additionalProperties": false
        }),
        |args, ctx| async move { handle_audit_trace_current(args, ctx).await }
    )]
}

/// The IPC-2221 cross-section (mm²) that carries `amps` at `rise_c`.
pub(crate) fn required_area_mm2(amps: f64, rise_c: f64, external: bool) -> f64 {
    let k = if external { K_EXTERNAL } else { K_INTERNAL };
    let area_mil2 = (amps / (k * rise_c.powf(DT_EXPONENT))).powf(1.0 / AREA_EXPONENT);
    area_mil2 * MM_PER_MIL * MM_PER_MIL
}

/// The IPC-2221 current (A) a cross-section of `area_mm2` carries at `rise_c`.
pub(crate) fn capacity_a(area_mm2: f64, rise_c: f64, external: bool) -> f64 {
    let k = if external { K_EXTERNAL } else { K_INTERNAL };
    let area_mil2 = area_mm2 / (MM_PER_MIL * MM_PER_MIL);
    k * rise_c.powf(DT_EXPONENT) * area_mil2.powf(AREA_EXPONENT)
}

pub(crate) fn is_outer(layer: &str) -> bool {
    layer == "F.Cu" || layer == "B.Cu"
}

/// One net that must carry a current.
#[derive(Debug, Clone)]
struct Requirement {
    net: String,
    amps: f64,
    rise_c: f64,
    rise_source: &'static str,
    /// `currents` or `rail:<name>`.
    source: String,
}

/// Copper thickness per copper layer, and where it came from.
pub(crate) struct Copper {
    by_layer: BTreeMap<String, (f64, &'static str)>,
    outer: (f64, &'static str),
    inner: (f64, &'static str),
}

impl Copper {
    pub(crate) fn thickness(&self, layer: &str) -> (f64, &'static str) {
        if let Some(t) = self.by_layer.get(layer) {
            return *t;
        }
        if is_outer(layer) {
            self.outer
        } else {
            self.inner
        }
    }
}

/// Copper thickness from the board's own `(setup (stackup …))`, by layer.
fn stackup_copper(tree: &SexpNode) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    let Some(stackup) = tree.find("setup").and_then(|s| s.find("stackup")) else {
        return out;
    };
    for layer in stackup.find_all("layer") {
        let Some(name) = layer.get(1).and_then(|n| n.as_str()) else {
            continue;
        };
        if layer.find_str("type") != Some("copper") {
            continue;
        }
        if let Some(t) = layer
            .find_f64("thickness")
            .filter(|t| t.is_finite() && *t > 0.0)
        {
            out.insert(name.to_string(), t);
        }
    }
    out
}

pub(crate) fn copper_for(
    tree: &SexpNode,
    intent: &crate::tools::design_intent::DesignIntent,
) -> Copper {
    let target = intent.fab_target.as_ref();
    let outer_oz = target.and_then(|t| t.outer_copper_oz);
    let inner_oz = target.and_then(|t| t.inner_copper_oz);
    let stack = stackup_copper(tree);
    let mut by_layer = BTreeMap::new();
    for (layer, t) in &stack {
        let from_intent = if is_outer(layer) { outer_oz } else { inner_oz };
        let value = match from_intent {
            Some(oz) => (oz * MM_PER_OZ, "design_intent"),
            None => (*t, "board_stackup"),
        };
        by_layer.insert(layer.clone(), value);
    }
    Copper {
        by_layer,
        outer: match outer_oz {
            Some(oz) => (oz * MM_PER_OZ, "design_intent"),
            None => (MM_PER_OZ, "default_1oz"),
        },
        inner: match inner_oz {
            Some(oz) => (oz * MM_PER_OZ, "design_intent"),
            None => (0.5 * MM_PER_OZ, "default_0.5oz"),
        },
    }
}

fn requirements(
    intent: &crate::tools::design_intent::DesignIntent,
    default_rise: f64,
    default_rise_given: bool,
) -> Vec<Requirement> {
    let fallback = if default_rise_given {
        "call_default"
    } else {
        "built_in_default"
    };
    let rise = |given: Option<f64>| match given {
        Some(r) => (r, "design_intent"),
        None => (default_rise, fallback),
    };
    let mut out = Vec::new();
    for c in &intent.currents {
        let (rise_c, rise_source) = rise(c.max_temp_rise_c);
        out.push(Requirement {
            net: c.net.clone(),
            amps: c.max_current_a,
            rise_c,
            rise_source,
            source: "currents".to_string(),
        });
    }
    for rail in &intent.rails {
        let Some(amps) = rail.max_current_a else {
            continue;
        };
        let (rise_c, rise_source) = rise(rail.max_temp_rise_c);
        for net in &rail.nets {
            out.push(Requirement {
                net: net.clone(),
                amps,
                rise_c,
                rise_source,
                source: format!("rail:{}", rail.name),
            });
        }
    }
    out.sort_by(|a, b| a.net.cmp(&b.net));
    out
}

fn prop(entry: &Entry, key: &str) -> Option<f64> {
    entry.props.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// The audit of one net: its JSON and its findings.
fn audit_net(req: &Requirement, geometry: &BoardGeometry, copper: &Copper) -> (Value, Vec<Value>) {
    let on_net = |e: &&Entry| e.net.as_deref() == Some(req.net.as_str());
    let tracks: Vec<&Entry> = geometry
        .others
        .iter()
        .filter(|e| e.kind == Kind::Track)
        .filter(on_net)
        .collect();
    let vias: Vec<&Entry> = geometry
        .others
        .iter()
        .filter(|e| e.kind == Kind::Via)
        .filter(on_net)
        .collect();
    let zone_layers: BTreeSet<String> = geometry
        .others
        .iter()
        .filter(|e| e.kind == Kind::Zone)
        .filter(on_net)
        .flat_map(|e| e.layers.iter().cloned())
        .collect();

    let mut undersized: Vec<Value> = Vec::new();
    let mut undersized_layers: BTreeSet<String> = BTreeSet::new();
    let mut narrowest: Option<Value> = None;
    let mut narrowest_margin = f64::INFINITY;
    let mut notes: Vec<String> = Vec::new();
    for t in &tracks {
        let (Some(width), Some(layer)) = (prop(t, "width"), t.layers.first()) else {
            continue;
        };
        let external = is_outer(layer);
        let (thickness, _) = copper.thickness(layer);
        let required = required_area_mm2(req.amps, req.rise_c, external) / thickness;
        let capacity = capacity_a(width * thickness, req.rise_c, external);
        let item = json!({
            "uuid": t.uuid,
            "layer": layer,
            "width_mm": r3(width),
            "required_mm": r3(required),
            "capacity_a": (capacity * 100.0).round() / 100.0,
        });
        let margin = width / required;
        if margin < narrowest_margin {
            narrowest_margin = margin;
            narrowest = Some(item.clone());
        }
        if width < required * (1.0 - 1e-9) {
            undersized_layers.insert(layer.clone());
            undersized.push(item);
        }
    }
    if req.amps > 35.0 {
        notes.push("current above 35 A: outside the IPC-2221 chart's fitted range".to_string());
    }
    if !(10.0..=100.0).contains(&req.rise_c) {
        notes.push(format!(
            "temperature rise {} °C is outside the IPC-2221 chart's 10–100 °C range",
            req.rise_c
        ));
    }
    if let Some(n) = &narrowest {
        if n["required_mm"]
            .as_f64()
            .is_some_and(|w| w > 400.0 * MM_PER_MIL)
        {
            notes.push(
                "required width above 400 mil: outside the IPC-2221 chart's fitted range"
                    .to_string(),
            );
        }
    }

    let via_info = vias
        .iter()
        .filter_map(|v| prop(v, "drill"))
        .fold(None::<f64>, |m, d| Some(m.map_or(d, |m: f64| m.min(d))))
        .map(|drill| {
            let barrel = std::f64::consts::PI * drill * VIA_PLATING_MM;
            let each = capacity_a(barrel, req.rise_c, false);
            let needed = (req.amps / each).ceil() as usize;
            json!({
                "count": vias.len(),
                "smallest_drill_mm": r3(drill),
                "capacity_each_a": (each * 100.0).round() / 100.0,
                "needed_in_parallel": needed,
            })
        });

    let mut findings = Vec::new();
    if tracks.is_empty() && zone_layers.is_empty() {
        findings.push(json!({
            "severity": "info",
            "category": "current",
            "component": Value::Null,
            "net": req.net,
            "issue": format!("Net '{}' ({} A) has no tracks or zones on the board", req.net, req.amps),
            "recommendation": "Route the net, or correct its name in the design-intent record",
        }));
    }
    if !undersized.is_empty() {
        let narrowest_bad = undersized
            .iter()
            .min_by(|a, b| {
                a["width_mm"]
                    .as_f64()
                    .partial_cmp(&b["width_mm"].as_f64())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned()
            .unwrap_or(Value::Null);
        let pour_shared = undersized_layers.iter().any(|l| zone_layers.contains(l));
        findings.push(json!({
            "severity": if pour_shared { "warning" } else { "error" },
            "category": "current",
            "component": Value::Null,
            "net": req.net,
            "issue": format!(
                "{} track segment(s) on net '{}' are narrower than IPC-2221 needs for {} A at \
                 {} °C rise (narrowest {} mm on {}, needs {} mm){}",
                undersized.len(),
                req.net,
                req.amps,
                req.rise_c,
                narrowest_bad["width_mm"],
                narrowest_bad["layer"].as_str().unwrap_or("?"),
                narrowest_bad["required_mm"],
                if pour_shared { "; a zone of this net on the same layer may share the load" } else { "" }
            ),
            "recommendation": "Widen these segments, add parallel copper, or record a lower current if this branch carries only part of it",
        }));
    }
    if let Some(info) = &via_info {
        let needed = info["needed_in_parallel"].as_u64().unwrap_or(0) as usize;
        if vias.len() < needed {
            findings.push(json!({
                "severity": "warning",
                "category": "current",
                "component": Value::Null,
                "net": req.net,
                "issue": format!(
                    "Net '{}' has {} via(s); a layer change carrying {} A needs at least {} of \
                     them in parallel ({} A each)",
                    req.net, vias.len(), req.amps, needed, info["capacity_each_a"]
                ),
                "recommendation": "Add vias at each layer transition of this net",
            }));
        }
    }

    let total_undersized = undersized.len();
    undersized.truncate(LISTED_ITEMS);
    let body = json!({
        "net": req.net,
        "current_a": req.amps,
        "temp_rise_c": req.rise_c,
        "temp_rise_source": req.rise_source,
        "source": req.source,
        "tracks_checked": tracks.len(),
        "undersized_count": total_undersized,
        "undersized": undersized,
        "narrowest": narrowest,
        "vias": via_info,
        "zone_layers": zone_layers,
        "notes": notes,
    });
    (body, findings)
}

/// The audit's JSON for `board`, or a refusal. Shared with `run_design_review`.
pub(crate) fn audit(board: &Path, default_rise: Option<f64>) -> anyhow::Result<CallToolResult> {
    let project_dir = board.parent().unwrap_or(Path::new("."));
    let intent_path = design_intent_path(project_dir);
    let intent = match read_design_intent(&intent_path) {
        IntentRead::Loaded { intent, text } => (intent, text),
        IntentRead::Absent => {
            return Ok(CallToolResult::text(serde_json::to_string(&json!({
                "audit": "trace_current",
                "status": "no_design_intent",
                "design_intent": intent_path.display().to_string(),
                "findings": [],
                "summary": "No design-intent record, so no currents to check. Record them with \
                            set_design_intent (section 'currents', or 'rails' with max_current_a).",
            }))?))
        }
        IntentRead::Unusable { reason } => return Ok(unusable(&intent_path, reason)),
    };
    let (intent, text) = intent;
    let revision = {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(text.as_bytes());
        format!(
            "sha256:{}",
            digest
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )
    };

    let reqs = requirements(
        &intent,
        default_rise.unwrap_or(DEFAULT_TEMP_RISE_C),
        default_rise.is_some(),
    );
    let rails_without_current: Vec<&str> = intent
        .rails
        .iter()
        .filter(|r| r.max_current_a.is_none())
        .map(|r| r.name.as_str())
        .collect();
    if reqs.is_empty() {
        return Ok(CallToolResult::text(serde_json::to_string(&json!({
            "audit": "trace_current",
            "status": "no_currents_recorded",
            "design_intent_revision": revision,
            "rails_without_current": rails_without_current,
            "findings": [],
            "summary": "The design-intent record holds no currents. Add them with \
                        set_design_intent (section 'currents', or 'rails' with max_current_a).",
        }))?));
    }

    let content = std::fs::read_to_string(board)?;
    let tree = parse_sexp(&content)?;
    let geometry = BoardGeometry::from_tree(&tree);
    let copper = copper_for(&tree, &intent);
    let board_nets: BTreeSet<&str> = geometry
        .others
        .iter()
        .filter_map(|e| e.net.as_deref())
        .chain(
            geometry
                .footprints
                .iter()
                .flat_map(|f| &f.pads)
                .filter_map(|p| p.net.as_deref()),
        )
        .collect();

    let mut nets = Vec::new();
    let mut findings = Vec::new();
    let mut not_on_board = Vec::new();
    for req in &reqs {
        if !board_nets.contains(req.net.as_str()) {
            not_on_board.push(req.net.clone());
            findings.push(json!({
                "severity": "warning",
                "category": "current",
                "component": Value::Null,
                "net": req.net,
                "issue": format!(
                    "The design-intent record gives net '{}' {} A, but the board has no net of that name",
                    req.net, req.amps
                ),
                "recommendation": "Correct the net name in the record (names are matched exactly, including any '/' sheet prefix)",
            }));
            continue;
        }
        let (body, mut net_findings) = audit_net(req, &geometry, &copper);
        nets.push(body);
        findings.append(&mut net_findings);
    }

    let copper_json: BTreeMap<String, Value> = geometry
        .others
        .iter()
        .flat_map(|e| e.layers.iter())
        .filter(|l| l.ends_with(".Cu") && *l != "*.Cu")
        .map(|l| {
            let (t, source) = copper.thickness(l);
            (
                l.clone(),
                json!({ "thickness_mm": r3(t * 1000.0) / 1000.0, "source": source }),
            )
        })
        .collect();
    let failing = findings
        .iter()
        .filter(|f| f["severity"] == "error" || f["severity"] == "warning")
        .count();
    Ok(CallToolResult::text(serde_json::to_string(&json!({
        "audit": "trace_current",
        "status": "checked",
        "design_intent_revision": revision,
        "method": {
            "formula": "IPC-2221: I = k·ΔT^0.44·A^0.725 (A in mil²), k = 0.048 outer, 0.024 inner",
            "current_model": "every_item_carries_full_net_current",
            "via_model": format!("plated barrel π·drill·{VIA_PLATING_MM} mm, as an inner conductor"),
        },
        "copper": copper_json,
        "nets": nets,
        "nets_not_on_board": not_on_board,
        "rails_without_current": rails_without_current,
        "not_checked": [
            "IPC-2152 board-level derating (plane proximity, board thickness)",
            "how current splits between parallel tracks, pours and vias",
            "zone and pour capacity",
            "pad and footprint neck-down capacity",
            "voltage drop",
        ],
        "items_unreadable": geometry.skipped,
        "findings": findings,
        "summary": format!(
            "{} net(s) checked, {} with problems, {} recorded net(s) not on the board.",
            nets.len(),
            failing,
            not_on_board.len()
        ),
    }))?))
}

async fn handle_audit_trace_current(
    args: &Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let board = get_path(args, "board")?;
    let default_rise = match args.get("default_temp_rise_c") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_f64() {
            Some(r) if r.is_finite() && r > 0.0 && r <= 100.0 => Some(r),
            _ => {
                return Ok(invalid_arg(
                    "default_temp_rise_c",
                    "must be a number above 0 and at most 100",
                ))
            }
        },
    };
    audit(&board, default_rise)
}

#[cfg(test)]
#[path = "trace_current_tests.rs"]
mod tests;
