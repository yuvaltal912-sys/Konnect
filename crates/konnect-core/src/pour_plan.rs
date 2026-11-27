//! Row ZR, stage ZR4: the pours a board should have (docs/POUR_PLAN_DESIGN.md).
//!
//! Pure: a plan is a function of the board's text, the project file's text and the
//! design-intent record. Nothing here reads a file, starts a process or changes a board.
//! A plan is a proposal: a zone enters a board only through `add_zone` (PP1). KiCad's own
//! fill of a plan is `pour_check`'s; the tool that joins the two is
//! `tools::pour_plan_tool`.

use crate::specctra;
use crate::tools::design_intent::{DesignIntent, FabTarget, PourIntent};
use crate::tools::pcb_board::{PadConnection, ZoneSettings};
use crate::tools::SupplyRoles;
use anyhow::{bail, Context, Result};
use konnect_ipc::builders::ZoneIslands;
use konnect_sexp::SexpNode;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// KiCad's default minimum width of a new zone, mm (`zones.h`, `ZONE_THICKNESS_MM`).
pub(crate) const MIN_WIDTH_MM: f64 = 0.25;

/// The clearance of KiCad's own Default netclass, mm: what a net has when the project
/// file holds no class (`pcb_routing::kicad_default_class`).
pub(crate) const SEEDED_CLASS_CLEARANCE_MM: f64 = 0.2;

/// What a net is to the board's supply (PP4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SupplyRole {
    Ground,
    Power,
}

/// One supply net of the board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SupplyNet {
    pub net: String,
    pub role: SupplyRole,
    /// Where the role comes from, in the layout audit's words: a design-intent rail, a
    /// netclass, the net's name; or "asked for in the design intent" for a net of the
    /// record's `pours` that has no role.
    pub source: String,
    /// The net's copper pads on the board, as the Specctra export counts them.
    pub pads: usize,
}

/// One copper layer of the board's stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CopperLayer {
    pub name: String,
    /// `signal` or `power`, as the export reads the board's layer table (KiCad's `mixed`
    /// is `signal`). Reported only: no rule reads it (PP6).
    pub kind: String,
}

/// A pour the board already has, on one copper layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExistingPour {
    pub net: String,
    pub layer: String,
    pub priority: u32,
}

/// A rule area of the board that keeps tracks, vias or pads out and lets copper pours in,
/// on one copper layer (PP15). KiCad keeps a fill out of a rule area only when the area
/// forbids pours (`zone_filler.cpp:2306`, `GetDoNotAllowZoneFills`), so a pour on this
/// layer fills inside it. `add_rule_area`'s own default is such an area.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct OpenRuleArea {
    pub layer: String,
    /// The area's name; `None` when it has none.
    pub name: Option<String>,
    pub uuid: Option<String>,
    /// What the area keeps out, of `tracks`, `vias` and `pads`, in that order.
    pub forbids: Vec<String>,
}

/// What a plan reads off a board.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BoardFacts {
    /// The copper layers, outermost first (`F.Cu`, `In1.Cu` …, `B.Cu`).
    pub stack: Vec<CopperLayer>,
    /// The board's outline in KiCad's own coordinates, mm (Y down): one closed loop, the
    /// first corner not repeated.
    pub outline_mm: Vec<(f64, f64)>,
    /// Copper pads per net; a net with no pad has no entry.
    pub pads: BTreeMap<String, usize>,
    /// The copper pours of the board, one row per zone and copper layer. Rule areas and
    /// teardrops are not pours.
    pub existing: Vec<ExistingPour>,
    /// The board's rule areas that keep tracks, vias or pads out and let pours in, one
    /// row per area and copper layer: in stack order, then by name, then by UUID.
    pub open_rule_areas: Vec<OpenRuleArea>,
}

/// The facts of a board's text:
/// - `stack` from `specctra::board_copper_stack`;
/// - `outline_mm` from `specctra::board_outline_um`: each corner `(x / 1000, -y / 1000)`,
///   the repeated last corner dropped;
/// - `pads` from `specctra::pad_pins`, counted by net;
/// - `existing` from `specctra::pour_fill_rows`, each row's priority read from its
///   `(zone …)` node (`(priority N)`, 0 when absent);
/// - `open_rule_areas`: every `(zone …)` of the board's root that has a `(keepout …)`
///   node, is no teardrop, has `not_allowed` for at least one of `tracks`, `vias` and
///   `pads`, and does not have `(copperpour not_allowed)`; one row for each of its copper
///   layers that the stack has (`*.Cu` is every copper layer of the stack).
///
/// An error wherever one of those four readers of the Specctra export refuses the board,
/// with the reader's own reason: a copper stack the export does not support, an outline
/// that is not one closed loop, a footprint it cannot write (one with a rule area or a
/// zone of its own is among them, so `open_rule_areas` and `existing` need not look
/// inside footprints), a copper zone with no net or with an outline it cannot read.
/// Pours are defined before the export (Z8), so a board these readers refuse gets no
/// plan (PP10). The export's other refusals (locked routing it cannot write, copper
/// graphics on the board) are the export's to report when it runs.
pub(crate) fn board_facts(board_text: &str) -> Result<BoardFacts> {
    let tree = konnect_sexp::parse_sexp(board_text).context("the board does not parse")?;
    let stack: Vec<CopperLayer> = specctra::board_copper_stack(&tree)
        .context("the board's copper stack")?
        .into_iter()
        .map(|(name, kind)| CopperLayer {
            name,
            kind: kind.to_string(),
        })
        .collect();
    let mut outline_mm: Vec<(f64, f64)> = specctra::board_outline_um(&tree)
        .context("the board's outline")?
        .into_iter()
        // Negate in whole micrometres first, so that 0 stays 0 and never becomes -0.
        .map(|(x, y)| (x as f64 / 1000.0, (-y) as f64 / 1000.0))
        .collect();
    if outline_mm.len() > 1 && outline_mm.first() == outline_mm.last() {
        outline_mm.pop();
    }
    let mut pads: BTreeMap<String, usize> = BTreeMap::new();
    for pin in specctra::pad_pins(&tree).context("the board's footprints")? {
        if let Some(net) = pin.net.filter(|net| !net.is_empty()) {
            *pads.entry(net).or_default() += 1;
        }
    }
    let mut existing = Vec::new();
    for row in specctra::pour_fill_rows(&tree).context("the board's zones")? {
        let priority = match row.zone.find("priority") {
            None => 0,
            Some(node) => {
                let text = node.get(1).and_then(SexpNode::as_str).unwrap_or("");
                text.parse::<u32>().map_err(|_| {
                    anyhow::anyhow!(
                        "the board's zones: the {} zone of net '{}' has the priority '{text}', \
                         which is not a whole number of 0 or more",
                        row.layer,
                        row.net
                    )
                })?
            }
        };
        existing.push(ExistingPour {
            net: row.net,
            layer: row.layer,
            priority,
        });
    }
    let open_rule_areas = open_rule_areas(&tree, &stack);
    Ok(BoardFacts {
        stack,
        outline_mm,
        pads,
        existing,
        open_rule_areas,
    })
}

/// [`BoardFacts::open_rule_areas`] of a parsed board (PP15).
fn open_rule_areas(tree: &SexpNode, stack: &[CopperLayer]) -> Vec<OpenRuleArea> {
    let mut rows = Vec::new();
    for zone in tree.find_all("zone") {
        let Some(keepout) = zone.find("keepout") else {
            continue;
        };
        if specctra::is_teardrop(zone) {
            continue;
        }
        let forbidden = |tag: &str| keepout.find_str(tag) == Some("not_allowed");
        let forbids: Vec<String> = ["tracks", "vias", "pads"]
            .into_iter()
            .filter(|tag| forbidden(tag))
            .map(str::to_string)
            .collect();
        if forbids.is_empty() || forbidden("copperpour") {
            continue;
        }
        let mut layers: Vec<&str> = Vec::new();
        if let Some(single) = zone.find_str("layer") {
            layers.push(single);
        }
        if let Some(list) = zone.find("layers") {
            layers.extend(
                list.children()
                    .unwrap_or(&[])
                    .iter()
                    .skip(1)
                    .filter_map(SexpNode::as_str),
            );
        }
        let name = zone
            .find_str("name")
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        let uuid = zone
            .find_str("uuid")
            .or_else(|| zone.find_str("tstamp"))
            .map(str::to_string);
        for layer in stack {
            let on = layers
                .iter()
                .any(|named| *named == "*.Cu" || *named == layer.name);
            if on {
                rows.push(OpenRuleArea {
                    layer: layer.name.clone(),
                    name: name.clone(),
                    uuid: uuid.clone(),
                    forbids: forbids.clone(),
                });
            }
        }
    }
    let place = |layer: &str| stack.iter().position(|l| l.name == layer);
    rows.sort_by(|a, b| {
        (place(&a.layer), &a.name, &a.uuid).cmp(&(place(&b.layer), &b.name, &b.uuid))
    });
    rows
}

/// The project's numbers a pour's settings depend on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProjectRules {
    /// `board.design_settings.rules.min_clearance`, mm; 0 without one.
    pub min_clearance_mm: f64,
    /// `board.design_settings.rules.min_track_width`, mm; 0 without one.
    pub min_track_width_mm: f64,
    /// `net_settings.classes[].clearance` by class name, mm.
    pub class_clearance_mm: BTreeMap<String, f64>,
}

/// The rules of a project file's text. `None` (no project file beside the board) gives
/// KiCad's own: both minima 0 and no class.
///
/// `Err(reason)` when the text is not JSON, or one of the values read is present and is
/// not a finite number of 0 or more: a pour's clearance is never taken from a file that
/// does not read (P4).
pub(crate) fn project_rules(
    project_text: Option<&str>,
) -> std::result::Result<ProjectRules, String> {
    let mut rules = ProjectRules {
        min_clearance_mm: 0.0,
        min_track_width_mm: 0.0,
        class_clearance_mm: BTreeMap::new(),
    };
    let Some(text) = project_text else {
        return Ok(rules);
    };
    let project: Value = serde_json::from_str(text)
        .map_err(|error| format!("the project file is not JSON: {error}"))?;
    let read = |value: Option<&Value>, at: &str| -> std::result::Result<Option<f64>, String> {
        match value {
            None => Ok(None),
            Some(value) => match value.as_f64() {
                Some(number) if number.is_finite() && number >= 0.0 => Ok(Some(number)),
                _ => Err(format!(
                    "the project file's {at} is {value}, not a finite number of 0 or more"
                )),
            },
        }
    };
    let design = project
        .pointer("/board/design_settings/rules")
        .and_then(Value::as_object);
    if let Some(design) = design {
        if let Some(v) = read(design.get("min_clearance"), "min_clearance")? {
            rules.min_clearance_mm = v;
        }
        if let Some(v) = read(design.get("min_track_width"), "min_track_width")? {
            rules.min_track_width_mm = v;
        }
    }
    let classes = project
        .pointer("/net_settings/classes")
        .and_then(Value::as_array);
    for class in classes.into_iter().flatten() {
        let Some(name) = class.get("name").and_then(Value::as_str) else {
            continue;
        };
        let at = format!("netclass '{name}' clearance");
        if let Some(v) = read(class.get("clearance"), &at)? {
            rules.class_clearance_mm.insert(name.to_string(), v);
        }
    }
    Ok(rules)
}

/// The supply nets on the board, in the order the rules read them: the ground nets by
/// pads (most first), then by name; then the power nets the same way.
/// - A net with no pad on the board is left out, whether it has a role or the record
///   asks for it: a net the board does not have never takes a layer from one it has
///   (`division` reports an asked one as an unmet request).
/// - A net that `asked` names, that has a pad and no role, is added as power, with the
///   source "asked for in the design intent".
/// - A ground net that `asked` names without layers is moved to the head of the ground
///   nets, and a power net that `asked` names without layers to the head of the power
///   nets, in the record's order (PP7).
pub(crate) fn supply_nets(
    roles: &SupplyRoles,
    pads: &BTreeMap<String, usize>,
    asked: &[PourIntent],
) -> Vec<SupplyNet> {
    let on_board = |net: &str| pads.get(net).copied().filter(|count| *count > 0);
    let mut ground: Vec<SupplyNet> = roles
        .ground
        .iter()
        .filter_map(|(net, source)| {
            Some(SupplyNet {
                net: net.clone(),
                role: SupplyRole::Ground,
                source: source.clone(),
                pads: on_board(net)?,
            })
        })
        .collect();
    let mut power: Vec<SupplyNet> = roles
        .power
        .iter()
        .filter(|(net, _)| !roles.ground.contains_key(*net))
        .filter_map(|(net, source)| {
            Some(SupplyNet {
                net: net.clone(),
                role: SupplyRole::Power,
                source: source.clone(),
                pads: on_board(net)?,
            })
        })
        .collect();
    for request in asked {
        let known = roles.ground.contains_key(&request.net)
            || roles.power.contains_key(&request.net)
            || power.iter().any(|n| n.net == request.net);
        if known {
            continue;
        }
        if let Some(count) = on_board(&request.net) {
            power.push(SupplyNet {
                net: request.net.clone(),
                role: SupplyRole::Power,
                source: ASKED_SOURCE.to_string(),
                pads: count,
            });
        }
    }
    for list in [&mut ground, &mut power] {
        list.sort_by(|a, b| b.pads.cmp(&a.pads).then_with(|| a.net.cmp(&b.net)));
        // A net asked for without layers heads its kind, in the record's order (PP7).
        let mut head: Vec<SupplyNet> = Vec::new();
        for request in asked.iter().filter(|r| r.layers.is_empty()) {
            if let Some(at) = list.iter().position(|n| n.net == request.net) {
                head.push(list.remove(at));
            }
        }
        head.append(list);
        *list = head;
    }
    ground.append(&mut power);
    ground
}

/// The source of a net's role when it has none but the design intent's `pours` asks for it.
pub(crate) const ASKED_SOURCE: &str = "asked for in the design intent";

/// One copper layer given to one net.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slot {
    pub layer: String,
    pub net: String,
    /// Why this net on this layer, in one sentence.
    pub reason: String,
    /// Whether the design intent's `pours` put this net here: an entry that names this
    /// layer, or an entry without layers whose net took the rule's layer of its kind.
    pub asked: bool,
}

/// A pour that was considered and is not proposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NotProposed {
    /// `None` for a reason that is about the board or a layer and not about one net (a
    /// stack with no rule, no ground net known, a layer the rule leaves free).
    pub net: Option<String>,
    pub layer: Option<String>,
    pub reason: String,
    /// Whether the design intent's `pours` asked for it and the board does not have it:
    /// an unmet request. A request the board already meets (its net already pours that
    /// layer) is not one.
    pub asked: bool,
}

/// The copper layers divided among the supply nets, and what was left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Division {
    /// In stack order.
    pub slots: Vec<Slot>,
    pub left_out: Vec<NotProposed>,
}

/// The division (PP5 to PP7). One net a layer, whole layers only.
/// 1. **Asked, with layers.** Each entry of `asked` that names layers, and whose net is
///    in `nets`, takes each of its layers that the stack has, in the record's order. A
///    layer the stack does not have is left out as an unmet request. (The record's own
///    validation refuses two entries on one layer; should it happen, the first entry
///    takes the layer and the other is an unmet request.)
/// 2. **The rule,** for the layers step 1 left free. `ground` is the first ground net of
///    `nets` and `power` the first power net (`supply_nets` has put a net asked without
///    layers first, and the net with the most pads otherwise):
///    - 2 layers: `ground` on both;
///    - 4 layers: `ground` on `F.Cu`, `In1.Cu` and `B.Cu`; `power` on `In2.Cu`, and
///      `ground` there when the board has no power net;
///    - any other count: nothing, and one `left_out` row with no net and no layer says
///      that this stack has no rule and that the design intent's `pours` can name layers.
///
///    The rule never places a net that an entry of `asked` names with layers, and never
///    puts another net of the kind in its place: the owner has placed that net himself,
///    and a second ground net over a layer he left clear is a pour nobody asked for. Each
///    layer the rule would have given that net stays free, with one `left_out` row (the
///    layer, no net) whose reason names the net and says that the design intent's `pours`
///    place it and can name this layer too. A layer the rule gives to `ground` on a board
///    with no ground net stays free the same way; its reason says that no ground net is
///    known and that the design intent's `rails` can record one.
/// 3. **Left out,** each with a reason that is true of it:
///    - every entry of `asked` whose net is not in `nets` (it has no pad on the board):
///      an unmet request, one row for the entry;
///    - an asked net without layers that got no layer: an unmet request;
///    - every other net of `nets` that got no layer: which net the rule chose in its
///      place, or that this stack has no layer for a net of its kind by rule, and that
///      the design intent's `pours` can ask for it.
pub(crate) fn division(
    stack: &[CopperLayer],
    nets: &[SupplyNet],
    asked: &[PourIntent],
) -> Division {
    let in_stack = |layer: &str| stack.iter().any(|l| l.name == layer);
    let net_of = |name: &str| nets.iter().find(|n| n.net == name);
    let mut slots: Vec<Slot> = Vec::new();
    let mut left_out: Vec<NotProposed> = Vec::new();
    let taken = |slots: &[Slot], layer: &str| slots.iter().any(|s| s.layer == layer);

    // 1. Asked, with layers.
    for request in asked.iter().filter(|r| !r.layers.is_empty()) {
        if net_of(&request.net).is_none() {
            continue; // reported in step 3
        }
        for layer in &request.layers {
            if !in_stack(layer) {
                left_out.push(NotProposed {
                    net: Some(request.net.clone()),
                    layer: Some(layer.clone()),
                    reason: format!(
                        "asked for on {layer}, which this board's copper stack does not have"
                    ),
                    asked: true,
                });
            } else if let Some(holder) = slots.iter().find(|s| s.layer == *layer) {
                left_out.push(NotProposed {
                    net: Some(request.net.clone()),
                    layer: Some(layer.clone()),
                    reason: format!(
                        "asked for on {layer}, which an earlier entry of the design intent's \
                         pours gives to {}; one layer takes one whole-layer pour",
                        holder.net
                    ),
                    asked: true,
                });
            } else {
                slots.push(Slot {
                    layer: layer.clone(),
                    net: request.net.clone(),
                    reason: format!(
                        "the design intent's pours ask for {} on {layer}",
                        request.net
                    ),
                    asked: true,
                });
            }
        }
    }

    // 2. The rule, for the layers step 1 left free.
    let placed: BTreeSet<&str> = asked
        .iter()
        .filter(|r| !r.layers.is_empty())
        .map(|r| r.net.as_str())
        .collect();
    let asked_bare: BTreeSet<&str> = asked
        .iter()
        .filter(|r| r.layers.is_empty())
        .map(|r| r.net.as_str())
        .collect();
    let first = |role: SupplyRole| nets.iter().find(|n| n.role == role);
    let ground = first(SupplyRole::Ground);
    let power = first(SupplyRole::Power);
    let count = stack.len();
    // The rule's layers, each with the net it gives the layer (`None`: no ground net).
    let rule: Option<Vec<(&str, Option<&SupplyNet>, &str)>> = match count {
        2 => Some(vec![
            (
                "F.Cu",
                ground,
                "the ground net on both layers of a 2-layer board",
            ),
            (
                "B.Cu",
                ground,
                "the ground net on both layers of a 2-layer board",
            ),
        ]),
        4 => {
            let inner = match power {
                Some(net) => (
                    "In2.Cu",
                    Some(net),
                    "the power net on In2.Cu of a 4-layer board",
                ),
                None => (
                    "In2.Cu",
                    ground,
                    "the ground net on In2.Cu of a 4-layer board that has no power net",
                ),
            };
            Some(vec![
                (
                    "F.Cu",
                    ground,
                    "the ground net on F.Cu, In1.Cu and B.Cu of a 4-layer board",
                ),
                (
                    "In1.Cu",
                    ground,
                    "the ground net on F.Cu, In1.Cu and B.Cu of a 4-layer board",
                ),
                inner,
                (
                    "B.Cu",
                    ground,
                    "the ground net on F.Cu, In1.Cu and B.Cu of a 4-layer board",
                ),
            ])
        }
        _ => None,
    };
    match &rule {
        None => left_out.push(NotProposed {
            net: None,
            layer: None,
            reason: format!(
                "a {count}-layer stack has no rule (the rules cover 2 and 4 layers); the design \
                 intent's pours can name layers for each net"
            ),
            asked: false,
        }),
        Some(rule) => {
            for (layer, net, why) in rule {
                if taken(&slots, layer) {
                    continue;
                }
                match net {
                    None => left_out.push(NotProposed {
                        net: None,
                        layer: Some(layer.to_string()),
                        reason: format!(
                            "the rule gives {layer} to the ground net, and no ground net is known \
                             on this board; the design intent's rails can record one (a 0 V rail)"
                        ),
                        asked: false,
                    }),
                    Some(net) if placed.contains(net.net.as_str()) => {
                        let where_placed = if slots.iter().any(|s| s.net == net.net) {
                            "place on other layers".to_string()
                        } else {
                            let named: Vec<&str> = asked
                                .iter()
                                .filter(|r| r.net == net.net)
                                .flat_map(|r| r.layers.iter().map(String::as_str))
                                .collect();
                            format!(
                                "place on {}, which this board does not have",
                                named.join(", ")
                            )
                        };
                        left_out.push(NotProposed {
                            net: None,
                            layer: Some(layer.to_string()),
                            reason: format!(
                                "the rule would give {layer} to {}, which the design intent's \
                                 pours {where_placed}; the rule puts no other net in its \
                                 place, and the pours can name {layer} too",
                                net.net
                            ),
                            asked: false,
                        });
                    }
                    Some(net) => {
                        let asked = asked_bare.contains(net.net.as_str());
                        let reason = if asked {
                            format!(
                                "the design intent's pours ask for {} without layers, so it takes \
                                 the rule's layer for a net of its kind: {why}",
                                net.net
                            )
                        } else {
                            format!("{} by rule: {why}", net.net)
                        };
                        slots.push(Slot {
                            layer: layer.to_string(),
                            net: net.net.clone(),
                            reason,
                            asked,
                        });
                    }
                }
            }
        }
    }

    // 3. Left out.
    for request in asked {
        if net_of(&request.net).is_none() {
            left_out.push(NotProposed {
                net: Some(request.net.clone()),
                layer: None,
                reason: format!(
                    "the design intent's pours ask for {}, and the board has no pad of that net",
                    request.net
                ),
                asked: true,
            });
        }
    }
    let rule_net = |role: SupplyRole| match role {
        SupplyRole::Ground => ground,
        SupplyRole::Power => power,
    };
    let has_rule_layer =
        |role: SupplyRole| matches!((count, role), (2, SupplyRole::Ground) | (4, _));
    for net in nets {
        if slots.iter().any(|s| s.net == net.net) || placed.contains(net.net.as_str()) {
            continue;
        }
        let kind = match net.role {
            SupplyRole::Ground => "ground",
            SupplyRole::Power => "power",
        };
        let why = if rule.is_none() {
            format!("a {count}-layer stack has no rule")
        } else if !has_rule_layer(net.role) {
            format!("the rule of a {count}-layer board has no layer for a {kind} net")
        } else {
            match rule_net(net.role) {
                Some(chosen) if placed.contains(chosen.net.as_str()) => format!(
                    "the design intent's pours place {}, and the rule puts no other {kind} net \
                     in its place",
                    chosen.net
                ),
                Some(chosen) if slots.iter().any(|s| s.net == chosen.net) => format!(
                    "the rule pours {} in its place (one net a layer)",
                    chosen.net
                ),
                _ => format!(
                    "every layer the rule has for a {kind} net is taken by the design intent's \
                     pours"
                ),
            }
        };
        let (reason, asked) = if asked_bare.contains(net.net.as_str()) {
            (
                format!(
                    "the design intent's pours ask for {} without layers, and it gets none: \
                     {why}; the pours can name its layers",
                    net.net
                ),
                true,
            )
        } else {
            (
                format!("{why}; the design intent's pours can ask for {}", net.net),
                false,
            )
        };
        left_out.push(NotProposed {
            net: Some(net.net.clone()),
            layer: None,
            reason,
            asked,
        });
    }

    let place = |layer: &str| stack.iter().position(|l| l.name == layer);
    slots.sort_by_key(|slot| place(&slot.layer));
    Division { slots, left_out }
}

/// Whether `slot` can be added to a board that already has `existing` (PP8). `Err(reason)`:
/// - the net already has a pour on the layer ("already poured; existing zones are never
///   changed");
/// - another net has a pour on the layer at priority 0: a whole-layer pour must sit below
///   it, and KiCad has no priority below 0, so the reason names that zone's net and says
///   to raise its priority.
///
/// Another net's pours at priority 1 or more are fine: the proposal, at priority 0, fills
/// what they leave.
pub(crate) fn against_existing(
    slot: &Slot,
    existing: &[ExistingPour],
) -> std::result::Result<(), String> {
    if existing
        .iter()
        .any(|pour| pour.net == slot.net && pour.layer == slot.layer)
    {
        return Err(format!(
            "{} is already poured on {}; existing zones are never changed",
            slot.net, slot.layer
        ));
    }
    if let Some(pour) = existing
        .iter()
        .find(|pour| pour.layer == slot.layer && pour.net != slot.net && pour.priority == 0)
    {
        return Err(format!(
            "{}'s pour on {} sits at priority 0, and a whole-layer pour of {} must sit below \
             it, which KiCad cannot do (it has no priority below 0); raise that zone's \
             priority to 1 or more, then plan again",
            pour.net, slot.layer, slot.net
        ));
    }
    Ok(())
}

/// The settings of a pour of `net` (PP9):
/// - clearance: the largest of the clearances of the net's classes (`classes`, the names
///   the audit gives the net) and of the Default class (the file's, else
///   [`SEEDED_CLASS_CLEARANCE_MM`]), `rules.min_clearance_mm` and the fab target's
///   `min_clearance_mm`;
/// - minimum width: the largest of [`MIN_WIDTH_MM`], `rules.min_track_width_mm` and the
///   fab target's `min_track_mm`;
/// - thermal pads, a gap of `konnect_ipc::builders::ZONE_THERMAL_RELIEF_MM`, a bridge of
///   the larger of that and the minimum width; islands always removed; priority 0.
///
/// The result passes `ZoneSettings::check`.
pub(crate) fn settings_for(
    classes: &[String],
    rules: &ProjectRules,
    fab: Option<&FabTarget>,
) -> ZoneSettings {
    let default_class = rules
        .class_clearance_mm
        .get("Default")
        .copied()
        .unwrap_or(SEEDED_CLASS_CLEARANCE_MM);
    let clearance_mm = classes
        .iter()
        .filter_map(|class| rules.class_clearance_mm.get(class).copied())
        .chain([
            default_class,
            rules.min_clearance_mm,
            fab.and_then(|f| f.min_clearance_mm).unwrap_or(0.0),
        ])
        .fold(0.0, f64::max);
    let min_width_mm = [
        MIN_WIDTH_MM,
        rules.min_track_width_mm,
        fab.and_then(|f| f.min_track_mm).unwrap_or(0.0),
    ]
    .into_iter()
    .fold(0.0, f64::max);
    ZoneSettings {
        clearance_mm,
        min_width_mm,
        priority: 0,
        connection: PadConnection::Thermal,
        thermal_gap_mm: THERMAL_STEPS[0].0,
        thermal_bridge_mm: THERMAL_STEPS[0].1.max(min_width_mm),
        islands: ZoneIslands::Always,
    }
}

/// PP16: the thermal values the check tries, in order: dash's own, then B5's own.
/// Each is (gap, bridge floor) in mm; a proposal's bridge is the larger of the floor and
/// its minimum width. The first is `settings_for`'s (a test holds that it equals
/// `ZONE_THERMAL_RELIEF_MM` for both), so `add_zone`'s own defaults do not change.
pub(crate) const THERMAL_STEPS: [(f64, f64); 2] = [(0.5, 0.5), (0.3, 0.4)];

/// PP16: `zones` with the thermal values of `THERMAL_STEPS[step]`: `thermal_gap_mm` is
/// the gap, `thermal_bridge_mm` the larger of the floor and the zone's own
/// `min_width_mm`. Nothing else changes, and every result passes `ZoneSettings::check`.
/// `None` for a step that is not in the table.
pub(crate) fn with_thermal_step(zones: &[ProposedZone], step: usize) -> Option<Vec<ProposedZone>> {
    let (gap, floor) = *THERMAL_STEPS.get(step)?;
    Some(
        zones
            .iter()
            .map(|zone| {
                let mut zone = zone.clone();
                zone.settings.thermal_gap_mm = gap;
                zone.settings.thermal_bridge_mm = floor.max(zone.settings.min_width_mm);
                zone
            })
            .collect(),
    )
}

/// One proposed zone: every value `add_zone` takes but the board.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProposedZone {
    pub net: String,
    pub layer: String,
    pub points_mm: Vec<(f64, f64)>,
    pub settings: ZoneSettings,
    pub reason: String,
    pub asked: bool,
}

/// A plan: what to add, and what was left out.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PourPlan {
    pub stack: Vec<CopperLayer>,
    /// The supply nets considered, in the rules' order.
    pub nets: Vec<SupplyNet>,
    /// In stack order.
    pub proposals: Vec<ProposedZone>,
    pub not_proposed: Vec<NotProposed>,
    /// The rule areas a proposal's pour fills inside (PP15): every row of
    /// `BoardFacts::open_rule_areas` whose layer has a proposal.
    pub filled_rule_areas: Vec<OpenRuleArea>,
}

/// The plan of a board: [`supply_nets`], [`division`], then each slot through
/// [`against_existing`] (a refused slot is `not_proposed` with the reason), then
/// [`settings_for`] and the board's outline for every slot that stays, and last
/// `filled_rule_areas` from the layers that have a proposal. `intent` is the project's
/// record when it has one: its `pours` are the requests, its `fab_target` the fab's
/// minima.
pub(crate) fn plan(
    facts: &BoardFacts,
    roles: &SupplyRoles,
    rules: &ProjectRules,
    intent: Option<&DesignIntent>,
) -> PourPlan {
    let asked: &[PourIntent] = intent.map(|i| i.pours.as_slice()).unwrap_or(&[]);
    let fab = intent.and_then(|i| i.fab_target.as_ref());
    let nets = supply_nets(roles, &facts.pads, asked);
    let division = division(&facts.stack, &nets, asked);
    let mut not_proposed = division.left_out;
    let mut proposals = Vec::new();
    for slot in division.slots {
        if let Err(reason) = against_existing(&slot, &facts.existing) {
            // A request for a layer the net already pours is met by the board itself: it
            // is no unmet request. Every other refusal leaves a request unmet.
            let met = facts
                .existing
                .iter()
                .any(|pour| pour.net == slot.net && pour.layer == slot.layer);
            not_proposed.push(NotProposed {
                net: Some(slot.net),
                layer: Some(slot.layer),
                reason,
                asked: slot.asked && !met,
            });
            continue;
        }
        let classes = roles.netclass.get(&slot.net).cloned().unwrap_or_default();
        proposals.push(ProposedZone {
            settings: settings_for(&classes, rules, fab),
            points_mm: facts.outline_mm.clone(),
            net: slot.net,
            layer: slot.layer,
            reason: slot.reason,
            asked: slot.asked,
        });
    }
    let filled_rule_areas = facts
        .open_rule_areas
        .iter()
        .filter(|area| proposals.iter().any(|p| p.layer == area.layer))
        .cloned()
        .collect();
    PourPlan {
        stack: facts.stack.clone(),
        nets,
        proposals,
        not_proposed,
        filled_rule_areas,
    }
}

/// The `add_zone` arguments that add one proposal to `board`: `board`, `net_name`,
/// `layer`, `points` and every setting (`ZoneSettings::to_args`), the defaults included,
/// so that the zone KiCad checked is the zone that is added (PP14).
pub(crate) fn add_zone_arguments(board: &Path, zone: &ProposedZone) -> Value {
    let mut args = serde_json::Map::new();
    args.insert("board".into(), json!(board.display().to_string()));
    args.insert("net_name".into(), json!(zone.net));
    args.insert("layer".into(), json!(zone.layer));
    args.insert(
        "points".into(),
        Value::Array(
            zone.points_mm
                .iter()
                .map(|(x, y)| json!({ "x": x, "y": y }))
                .collect(),
        ),
    );
    args.extend(zone.settings.to_args());
    Value::Object(args)
}

/// `board_text` with each proposal's zone added before the closing parenthesis of its
/// root, written by `add_zone`'s own file writer (`tools::pcb_board::zone_sexp`), unnamed.
/// A net that a legacy board does not declare is refused (never net 0), and so is a text
/// whose root is not `kicad_pcb`. Nothing else in the text changes.
pub(crate) fn with_zones(board_text: &str, zones: &[ProposedZone]) -> Result<String> {
    let tree = konnect_sexp::parse_sexp(board_text).context("the board does not parse")?;
    if tree.head() != Some("kicad_pcb") {
        bail!("the board's text is not a kicad_pcb: its root is not (kicad_pcb …)");
    }
    let mut added = String::new();
    for zone in zones {
        let Some(net) = konnect_sexp::net::net_ref_for_write(&tree, &zone.net) else {
            bail!(
                "net '{}' is not declared in the board's net table; on this legacy-format board \
                 a zone must name a declared net, never net 0",
                zone.net
            );
        };
        added.push_str(&crate::tools::pcb_board::zone_sexp(
            &net,
            &zone.layer,
            &zone.points_mm,
            "",
            &zone.settings,
        ));
    }
    let close = board_text
        .rfind(')')
        .context("the board's text has no closing parenthesis")?;
    let mut out = String::with_capacity(board_text.len() + added.len());
    out.push_str(&board_text[..close]);
    out.push_str(&added);
    out.push_str(&board_text[close..]);
    Ok(out)
}

// ─── Row FF: the roles in one argument ───────────────────────────────────────

/// The source of a role the user gave in `pour_roles` (Row FF, FF5).
pub(crate) const USER_SOURCE: &str = "user";

/// The source a design-intent rail gives a role (`layout_quality::intent_roles`).
const RAIL_SOURCE: &str = "design_intent rail ";

/// The source a net's name gives a role.
const NAME_SOURCE: &str = "name";

/// The nets whose role comes from a design-intent rail (their source in `roles` says
/// so), with that role: `ground` or `power`.
pub(crate) fn rails_of(roles: &SupplyRoles) -> BTreeMap<String, &'static str> {
    let mut rails = BTreeMap::new();
    for (list, role) in [(&roles.ground, "ground"), (&roles.power, "power")] {
        for (net, source) in list {
            if source.starts_with(RAIL_SOURCE) {
                rails.insert(net.clone(), role);
            }
        }
    }
    rails
}

/// The nets whose role comes from their name only, in name order.
pub(crate) fn named_only(roles: &SupplyRoles) -> Vec<String> {
    let mut nets: Vec<String> = roles
        .ground
        .iter()
        .chain(roles.power.iter())
        .filter(|(_, source)| source.as_str() == NAME_SOURCE)
        .map(|(net, _)| net.clone())
        .collect();
    nets.sort();
    nets.dedup();
    nets
}

/// The roles of `roles`, with each net of `given` set to its role and its source
/// [`USER_SOURCE`]. `NotASupply` removes the net from both lists. `"as_named"` confirms
/// every role as it is. A net of `given` that is a design-intent rail (`rails`) of
/// another role is an error naming the rail: the intent wins. A net that `nets` does not
/// hold is an error too.
pub(crate) fn with_roles(
    roles: &SupplyRoles,
    given: &crate::full_flow::RolesAnswer,
    rails: &BTreeMap<String, &'static str>,
    nets: &BTreeSet<String>,
) -> std::result::Result<SupplyRoles, String> {
    use crate::full_flow::{GivenRole, RolesAnswer};
    let RolesAnswer::Map(given) = given else {
        return Ok(roles.clone());
    };
    let mut out = roles.clone();
    for (net, role) in given {
        if !nets.contains(net) {
            return Err(format!(
                "pour_roles names '{net}', which has no pad on the board. Name only the board's nets."
            ));
        }
        if let Some(rail) = rails.get(net) {
            if *rail != role.name() {
                return Err(format!(
                    "pour_roles gives '{net}' the role {}, but the design intent records it as a \
                     {rail} rail, and the design intent wins. Change pour_roles, or edit the rail \
                     with set_design_intent (section 'rails').",
                    role.name()
                ));
            }
        }
        out.ground.remove(net);
        out.power.remove(net);
        match role {
            GivenRole::Ground => {
                out.ground.insert(net.clone(), USER_SOURCE.to_string());
            }
            GivenRole::Power => {
                out.power.insert(net.clone(), USER_SOURCE.to_string());
            }
            GivenRole::NotASupply => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "pour_plan_tests.rs"]
pub(crate) mod tests;
