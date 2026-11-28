//! Row TW: the track widths a board should be routed at (docs/TRACK_WIDTH_PLAN_DESIGN.md).
//!
//! Pure: a plan is a function of the board's text, KiCad's effective routing rules, the
//! design-intent record, the resolved supply roles and the nets that are poured. Nothing
//! here reads a file, starts a process or changes a board. A plan is a proposal: a current
//! enters the design intent only through `set_design_intent` (the full flow's step L5),
//! and the export (`specctra::widen_for_intent`) turns a recorded current into a width and
//! a via with the same functions this module uses, so the plan, the route and
//! `audit_trace_current` cannot disagree.

use crate::tools::design_intent::{DesignIntent, NetCurrent, Rail};
use crate::tools::trace_current::{self, Copper};
use crate::tools::SupplyRoles;
use anyhow::{Context, Result};
use konnect_ipc::types::IpcEffectiveRoutingRules;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The current proposed for a supply that has no recorded current and no pour, in A. A
/// stated default: gate A shows it and asks (decision TW2).
pub(crate) const DEFAULT_SUPPLY_CURRENT_A: f64 = 1.0;

/// The via a net gets when one via of its class cannot carry its current, mm.
pub(crate) const POWER_VIA: ViaSize = ViaSize {
    diameter_mm: 0.8,
    drill_mm: 0.4,
};

/// The largest current an answer may give one net, A.
pub(crate) const MAX_GIVEN_CURRENT_A: f64 = 100.0;

/// How many of a net's narrow pads the plan names; the count of all of them is beside.
pub(crate) const LISTED_PADS: usize = 5;

/// The source of a current the plan proposes.
pub(crate) const PROPOSED_SOURCE: &str = "proposed default";

/// The source of a current an answer gives.
pub(crate) const USER_SOURCE: &str = "user";

/// The note a written record carries.
pub(crate) const RECORD_NOTE: &str = "recorded by run_full_flow (Row TW), confirmed at gate A";

/// A via's size, mm.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct ViaSize {
    pub diameter_mm: f64,
    pub drill_mm: f64,
}

/// The via a net carrying a current should use.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct ViaChoice {
    pub via: ViaSize,
    /// The power via replaced the class via.
    pub changed: bool,
    /// What one via of `via` carries, A (IPC-2221 on its plated barrel, as the audit
    /// computes it).
    pub capacity_each_a: f64,
    /// How many of `via` a layer change of the net needs in parallel.
    pub per_layer_change: usize,
}

/// The IPC-2221 width (mm) that carries `amps` at `rise_c` on the worst of `layers`
/// (an inner layer needs more copper), with the copper of `copper`; rounded up to the
/// next whole micrometre, so the DSN never rounds it below. Also the limiting layer.
/// `layers` empty: `(0.0, "")`.
pub(crate) fn needed_width_mm(
    amps: f64,
    rise_c: f64,
    layers: &[String],
    copper: &Copper,
) -> (f64, String) {
    let (needed, limiting) = layers
        .iter()
        .map(|layer| {
            let (thickness, _) = copper.thickness(layer);
            (
                trace_current::required_area_mm2(amps, rise_c, trace_current::is_outer(layer))
                    / thickness,
                layer.clone(),
            )
        })
        .fold((0.0f64, String::new()), |best, next| {
            if next.0 > best.0 {
                next
            } else {
                best
            }
        });
    ((needed * 1000.0).ceil() / 1000.0, limiting)
}

/// The via for a net that carries `amps` at `rise_c`, given its class via:
/// - the class via when one of it carries the current;
/// - else [`POWER_VIA`] when its drill is wider than the class via's (its diameter at
///   least the class via's), with as many in parallel as the current needs;
/// - else the class via, with as many in parallel as the current needs.
pub(crate) fn via_for_current(amps: f64, rise_c: f64, class: ViaSize) -> ViaChoice {
    let class_each = trace_current::via_capacity_a(class.drill_mm, rise_c);
    let per = |each: f64| vias_needed(amps, each);
    if class_each >= amps {
        return ViaChoice {
            via: class,
            changed: false,
            capacity_each_a: class_each,
            per_layer_change: 1,
        };
    }
    if POWER_VIA.drill_mm > class.drill_mm + 1e-9 {
        let via = ViaSize {
            diameter_mm: POWER_VIA.diameter_mm.max(class.diameter_mm),
            drill_mm: POWER_VIA.drill_mm,
        };
        let each = trace_current::via_capacity_a(via.drill_mm, rise_c);
        return ViaChoice {
            via,
            changed: true,
            capacity_each_a: each,
            per_layer_change: per(each),
        };
    }
    ViaChoice {
        via: class,
        changed: false,
        capacity_each_a: class_each,
        per_layer_change: per(class_each),
    }
}

/// How many vias that each carry `each` A a layer change carrying `amps` needs in
/// parallel: at least 1; `usize::MAX` when one carries nothing.
pub(crate) fn vias_needed(amps: f64, each: f64) -> usize {
    if each > 0.0 {
        ((amps / each) - 1e-9).ceil().max(1.0) as usize
    } else {
        usize::MAX
    }
}

/// What a net is to the width plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    Ground,
    Power,
    DifferentialPair,
    Signal,
}

/// One net the plan sizes: every net with a current (recorded, proposed or given).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SizedNet {
    pub net: String,
    pub role: Role,
    pub current_a: f64,
    pub temp_rise_c: f64,
    /// `currents`, `rail:<name>` (the design intent), [`PROPOSED_SOURCE`] or
    /// [`USER_SOURCE`].
    pub current_source: String,
    /// KiCad's effective class of the net, and its track width.
    pub class: String,
    pub class_width_mm: f64,
    /// What the current needs, and on which layer.
    pub needed_mm: f64,
    pub limiting_layer: String,
    /// What the export gives the net: the larger of the two, or the class width when the
    /// net is poured (Row S9's rule).
    pub width_mm: f64,
    pub poured: bool,
    pub class_via: ViaSize,
    pub via: ViaChoice,
    /// Pads of the net narrower than `width_mm` (`REF-pin`), at most [`LISTED_PADS`].
    pub narrow_pads: Vec<String>,
    pub narrow_pads_total: usize,
}

/// A supply the plan does not size, and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct NotSized {
    pub net: String,
    pub role: Role,
    pub why: String,
}

/// One row of the classes gate A shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ClassRow {
    /// `Power` (`Power 1`, `Power 2` … when several), or `Signal (<KiCad class>)`.
    pub name: String,
    pub track_width_mm: f64,
    pub via: ViaSize,
    pub nets: Vec<String>,
}

/// One current the flow writes to the design intent.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
#[serde(tag = "section", rename_all = "snake_case")]
pub(crate) enum Record {
    /// A `currents` entry for a net in no rail.
    Currents { net: String, amps: f64 },
    /// The `max_current_a` of a rail that has none (one value for all its nets).
    Rails { rail: String, amps: f64 },
}

/// The plan.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct WidthPlan {
    pub sized: Vec<SizedNet>,
    pub not_sized: Vec<NotSized>,
    pub classes: Vec<ClassRow>,
    /// Nets routed at their class width, with no current.
    pub signals: usize,
    pub differential_pairs: Vec<[String; 2]>,
    /// What step L5 writes when the answer is `as_proposed`.
    pub to_record: Vec<Record>,
    /// Per routing layer: its copper thickness (mm) and where it comes from.
    pub copper: BTreeMap<String, (f64, &'static str)>,
    pub notes: Vec<String>,
}

/// What a plan is made from.
pub(crate) struct Inputs<'a> {
    /// The board the flow builds (the zones it adds included).
    pub board_text: &'a str,
    /// KiCad's effective rules of every net.
    pub rules: &'a IpcEffectiveRoutingRules,
    pub intent: Option<&'a DesignIntent>,
    /// The roles after gate R's answer.
    pub roles: &'a SupplyRoles,
    /// Nets with a pour on the board the flow builds (`specctra::pour_nets`).
    pub poured: &'a BTreeSet<String>,
    /// The answer's `{net: amps}`, when it is a map.
    pub given: Option<&'a BTreeMap<String, f64>>,
    /// Nets with a locked via that stays on the board (`specctra::locked_via_nets` of the
    /// live board): the export keeps their class via.
    pub locked_vias: &'a BTreeSet<String>,
}

/// Why `given` is refused, or `None`:
/// - a net the board does not have (`nets`: the nets with a pad);
/// - a current that is not finite, not above 0, or above [`MAX_GIVEN_CURRENT_A`];
/// - a net whose current the design intent records (it wins: edit it with
///   `set_design_intent`);
/// - two nets of one rail given two values (a rail has one `max_current_a`).
pub(crate) fn answer_refusal(
    given: &BTreeMap<String, f64>,
    nets: &BTreeSet<String>,
    intent: Option<&DesignIntent>,
) -> Option<String> {
    let recorded = recorded_currents(intent);
    let mut by_rail: BTreeMap<&str, (&str, f64)> = BTreeMap::new();
    for (net, amps) in given {
        if !nets.contains(net) {
            return Some(format!(
                "track_widths names '{net}', which has no pad on the board. Name only the board's nets."
            ));
        }
        if !amps.is_finite() || *amps <= 0.0 || *amps > MAX_GIVEN_CURRENT_A {
            return Some(format!(
                "track_widths gives '{net}' {amps} A: a current must be above 0 and at most {MAX_GIVEN_CURRENT_A} A."
            ));
        }
        // The same current recorded already (L5 wrote it, and is resumed) is no conflict.
        if let Some((source, _)) = recorded
            .get(net.as_str())
            .filter(|(_, held)| (held - amps).abs() > 1e-9)
        {
            return Some(format!(
                "track_widths gives '{net}' a current, but the design intent records one ({source}), \
                 and the design intent wins. Leave '{net}' out, or edit it with set_design_intent."
            ));
        }
        if let Some(rail) = rail_of(intent, net) {
            match by_rail.get(rail.name.as_str()) {
                Some((other, other_amps)) if (other_amps - amps).abs() > 1e-9 => {
                    return Some(format!(
                        "track_widths gives '{other}' {other_amps} A and '{net}' {amps} A, but both \
                         are nets of the rail '{}', which has one current. Give them one value.",
                        rail.name
                    ));
                }
                _ => {
                    by_rail.insert(rail.name.as_str(), (net.as_str(), *amps));
                }
            }
        }
    }
    None
}

/// The nets whose current the design intent records, each with its source (`currents`
/// or `rail:<name>`) and its current, as `audit_trace_current` reads them.
pub(crate) fn recorded_currents(intent: Option<&DesignIntent>) -> BTreeMap<String, (String, f64)> {
    let Some(intent) = intent else {
        return BTreeMap::new();
    };
    trace_current::requirements(intent, trace_current::DEFAULT_TEMP_RISE_C, false)
        .into_iter()
        .map(|req| (req.net.clone(), (req.source.clone(), req.amps)))
        .collect()
}

/// The rail `net` belongs to, if any.
fn rail_of<'a>(intent: Option<&'a DesignIntent>, net: &str) -> Option<&'a Rail> {
    intent?
        .rails
        .iter()
        .find(|rail| rail.nets.iter().any(|n| n == net))
}

/// The record that gives `net` the current `amps`: its rail's `max_current_a` when it is
/// a rail net, else a `currents` entry.
fn record_for(intent: Option<&DesignIntent>, net: &str, amps: f64) -> Record {
    match rail_of(intent, net) {
        Some(rail) => Record::Rails {
            rail: rail.name.clone(),
            amps,
        },
        None => Record::Currents {
            net: net.to_string(),
            amps,
        },
    }
}

/// The plan for `inputs`:
/// 1. Roles: `roles.ground` and `roles.power` are supplies (ground first when a net is
///    both); the design intent's differential pairs are pairs; the rest are signals.
/// 2. Currents: the design intent's (`recorded_currents`, with their own rise); then the
///    answer's map ([`USER_SOURCE`]); then, for each supply with neither and no pour,
///    [`DEFAULT_SUPPLY_CURRENT_A`] ([`PROPOSED_SOURCE`]). A pair net gets no default. A
///    new current is checked at its rail's own rise when it has one (`rise_of`). A rail
///    that gets a current gives it to every net of the rail with a pad, as the export
///    will (`requirements`), so each of them is sized and shown.
/// 3. Each net with a current and a pad is sized: `needed_width_mm` over the routing
///    layers, `via_for_current` from its class via (the class via kept, with a note, on
///    a net with locked vias, as the export keeps it); `width_mm` is the class width when the
///    net is poured, else the larger of the class width and the need. Its narrow pads are
///    those whose smaller side is below `width_mm` (only when it is wider than the class).
/// 4. A supply with no current is `not_sized`: poured, or KiCad gave its net no rule.
/// 5. `to_record`: one [`Record`] per proposed or given current (rail nets once per rail),
///    in net order.
/// 6. `classes`: the sized nets that are widened or get the power via, grouped by width
///    and via, named `Power` (numbered when several, narrowest first); then every other net
///    with a pad grouped by its KiCad class, named `Signal (<class>)`.
///
/// An error when the board does not parse or the export's readers refuse it.
pub(crate) fn plan(inputs: &Inputs<'_>) -> Result<WidthPlan> {
    let tree = konnect_sexp::parse_sexp(inputs.board_text).context("the board does not parse")?;
    let layers = crate::specctra::routing_layers(&tree)?;
    let pads = crate::specctra::pad_pins(&tree)?;
    let empty = DesignIntent::empty();
    let intent = inputs.intent.unwrap_or(&empty);
    let copper = trace_current::copper_for(&tree, intent);
    let locked_vias = inputs.locked_vias;

    // Every net with a pad, and its pads' smaller sides.
    let mut net_pads: BTreeMap<String, Vec<(String, f64)>> = BTreeMap::new();
    for pad in &pads {
        if let Some(net) = &pad.net {
            net_pads
                .entry(net.clone())
                .or_default()
                .push((pad.dsn_name(), pad.min_size_um as f64 / 1000.0));
        }
    }

    // 1. Roles.
    let mut pairs: Vec<[String; 2]> = Vec::new();
    let mut pair_nets: BTreeSet<String> = BTreeSet::new();
    for interface in &intent.interfaces {
        for pair in &interface.differential_pairs {
            if net_pads.contains_key(&pair.positive) || net_pads.contains_key(&pair.negative) {
                pairs.push([pair.positive.clone(), pair.negative.clone()]);
                pair_nets.insert(pair.positive.clone());
                pair_nets.insert(pair.negative.clone());
            }
        }
    }
    let role_of = |net: &str| -> Role {
        if pair_nets.contains(net) {
            Role::DifferentialPair
        } else if inputs.roles.ground.contains_key(net) {
            Role::Ground
        } else if inputs.roles.power.contains_key(net) {
            Role::Power
        } else {
            Role::Signal
        }
    };

    // 2. Currents: net -> (amps, rise, source).
    let mut currents: BTreeMap<String, (f64, f64, String)> = BTreeMap::new();
    for req in trace_current::requirements(intent, trace_current::DEFAULT_TEMP_RISE_C, false) {
        currents.insert(req.net.clone(), (req.amps, req.rise_c, req.source.clone()));
    }
    let mut notes = Vec::new();
    for net in currents.keys() {
        if !net_pads.contains_key(net) {
            notes.push(format!(
                "the design intent records a current for '{net}', which has no pad on the board"
            ));
        }
    }
    let mut to_record: Vec<Record> = Vec::new();
    if let Some(given) = inputs.given {
        for (net, amps) in given {
            if currents.contains_key(net) || !net_pads.contains_key(net) {
                continue; // `answer_refusal` refuses both before a plan is made.
            }
            currents.insert(
                net.clone(),
                (*amps, rise_of(inputs.intent, net), USER_SOURCE.to_string()),
            );
            add_record(record_for(inputs.intent, net, *amps), &mut to_record);
        }
    }
    let mut not_sized: Vec<NotSized> = Vec::new();
    let supplies: Vec<&String> = inputs
        .roles
        .ground
        .keys()
        .chain(inputs.roles.power.keys())
        .collect();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for net in supplies {
        if !seen.insert(net.as_str()) || !net_pads.contains_key(net) || currents.contains_key(net) {
            continue;
        }
        let role = role_of(net);
        if role == Role::DifferentialPair {
            continue;
        }
        if inputs.poured.contains(net) {
            not_sized.push(NotSized {
                net: net.clone(),
                role,
                why: "poured: the pour carries its current (its capacity is not verified)"
                    .to_string(),
            });
            continue;
        }
        // A rail net whose rail already has a record (given, or proposed for another of
        // its nets) shares that current: a rail has one.
        let proposed = record_for(inputs.intent, net, DEFAULT_SUPPLY_CURRENT_A);
        let (amps, source) = match to_record
            .iter()
            .find(|old| record_key(old) == record_key(&proposed))
        {
            Some(Record::Rails { amps, .. }) if given_rail(inputs, net) => (*amps, USER_SOURCE),
            Some(Record::Rails { amps, .. }) | Some(Record::Currents { amps, .. }) => {
                (*amps, PROPOSED_SOURCE)
            }
            None => (DEFAULT_SUPPLY_CURRENT_A, PROPOSED_SOURCE),
        };
        currents.insert(
            net.clone(),
            (amps, rise_of(inputs.intent, net), source.to_string()),
        );
        add_record(record_for(inputs.intent, net, amps), &mut to_record);
    }
    // A rail's current reaches every net of the rail in the export (`requirements`), so
    // every one of them with a pad is sized and shown, whatever its role.
    for record in &to_record {
        let Record::Rails { rail, amps } = record else {
            continue;
        };
        let Some(rail) = intent.rails.iter().find(|r| &r.name == rail) else {
            continue;
        };
        let source = if rail
            .nets
            .iter()
            .any(|net| currents.get(net).is_some_and(|c| c.2 == USER_SOURCE))
        {
            USER_SOURCE
        } else {
            PROPOSED_SOURCE
        };
        for net in &rail.nets {
            if net_pads.contains_key(net) && !currents.contains_key(net) {
                not_sized.retain(|n| &n.net != net);
                currents.insert(
                    net.clone(),
                    (*amps, rise_of(inputs.intent, net), source.to_string()),
                );
            }
        }
    }

    // 3. Sizing.
    let mut sized: Vec<SizedNet> = Vec::new();
    for (net, (amps, rise, source)) in &currents {
        let Some(net_pad_list) = net_pads.get(net) else {
            continue;
        };
        let role = role_of(net);
        let rule = inputs.rules.get(net);
        let (Some(class_width), Some(diameter), Some(drill)) = (
            rule.and_then(|r| r.track_width_mm),
            rule.and_then(|r| r.via_diameter_mm),
            rule.and_then(|r| r.via_drill_mm),
        ) else {
            not_sized.push(NotSized {
                net: net.clone(),
                role,
                why: "KiCad gave the net no complete routing rule (track width and via)"
                    .to_string(),
            });
            continue;
        };
        let class_via = ViaSize {
            diameter_mm: diameter,
            drill_mm: drill,
        };
        let poured = inputs.poured.contains(net);
        let (needed, limiting) = needed_width_mm(*amps, *rise, &layers, &copper);
        let width = if poured {
            class_width
        } else {
            class_width.max(needed)
        };
        let kept = |per_layer_change: usize| ViaChoice {
            via: class_via,
            changed: false,
            capacity_each_a: trace_current::via_capacity_a(drill, *rise),
            per_layer_change,
        };
        let via = if poured {
            kept(1)
        } else {
            let choice = via_for_current(*amps, *rise, class_via);
            // The export keeps the class via of a net with locked vias, which must equal
            // its rule (`specctra::widen_for_intent`).
            if choice.changed && locked_vias.contains(net) {
                notes.push(format!(
                    "'{net}' keeps its class via: it has locked vias, which must equal its rule"
                ));
                kept(vias_needed(
                    *amps,
                    trace_current::via_capacity_a(drill, *rise),
                ))
            } else {
                choice
            }
        };
        let narrow: Vec<String> = if width > class_width + 1e-9 {
            net_pad_list
                .iter()
                .filter(|(_, side)| *side + 1e-9 < width)
                .map(|(name, _)| name.clone())
                .collect()
        } else {
            Vec::new()
        };
        sized.push(SizedNet {
            net: net.clone(),
            role,
            current_a: *amps,
            temp_rise_c: *rise,
            current_source: source.clone(),
            class: rule.map(|r| r.class_name.clone()).unwrap_or_default(),
            class_width_mm: class_width,
            needed_mm: needed,
            limiting_layer: limiting,
            width_mm: width,
            poured,
            class_via,
            via,
            narrow_pads_total: narrow.len(),
            narrow_pads: narrow.into_iter().take(LISTED_PADS).collect(),
        });
    }
    not_sized.sort_by(|a, b| a.net.cmp(&b.net));
    to_record.sort_by(|a, b| record_key(a).cmp(&record_key(b)));

    // 6. Classes.
    let power: Vec<&SizedNet> = sized
        .iter()
        .filter(|n| n.width_mm > n.class_width_mm + 1e-9 || n.via.changed)
        .collect();
    let mut groups: Vec<(f64, ViaSize, Vec<String>)> = Vec::new();
    for net in &power {
        match groups
            .iter_mut()
            .find(|(width, via, _)| (width - net.width_mm).abs() < 1e-9 && *via == net.via.via)
        {
            Some((_, _, nets)) => nets.push(net.net.clone()),
            None => groups.push((net.width_mm, net.via.via, vec![net.net.clone()])),
        }
    }
    groups.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.drill_mm.total_cmp(&b.1.drill_mm))
    });
    let numbered = groups.len() > 1;
    let mut classes: Vec<ClassRow> = groups
        .into_iter()
        .enumerate()
        .map(|(index, (width, via, nets))| ClassRow {
            name: if numbered {
                format!("Power {}", index + 1)
            } else {
                "Power".to_string()
            },
            track_width_mm: width,
            via,
            nets,
        })
        .collect();
    let in_power: BTreeSet<&str> = power.iter().map(|n| n.net.as_str()).collect();
    let mut by_class: BTreeMap<(String, String), ClassRow> = BTreeMap::new();
    let mut signals = 0usize;
    for net in net_pads.keys() {
        if in_power.contains(net.as_str()) {
            continue;
        }
        if role_of(net) == Role::Signal && !currents.contains_key(net) {
            signals += 1;
        }
        let Some(rule) = inputs.rules.get(net) else {
            continue;
        };
        let (Some(width), Some(diameter), Some(drill)) =
            (rule.track_width_mm, rule.via_diameter_mm, rule.via_drill_mm)
        else {
            continue;
        };
        let key = (
            rule.class_name.clone(),
            format!("{width:.4}/{diameter:.4}/{drill:.4}"),
        );
        by_class
            .entry(key)
            .or_insert_with(|| ClassRow {
                name: format!("Signal ({})", rule.class_name),
                track_width_mm: width,
                via: ViaSize {
                    diameter_mm: diameter,
                    drill_mm: drill,
                },
                nets: Vec::new(),
            })
            .nets
            .push(net.clone());
    }
    classes.extend(by_class.into_values());

    let copper_rows = layers
        .iter()
        .map(|layer| (layer.clone(), copper.thickness(layer)))
        .collect();
    if sized.iter().any(|n| n.narrow_pads_total > 0) {
        notes.push(
            "fastroute has no neck-down at a pad: a connection it cannot make at a net's \
             width is routed again at the net's class width, and the route's width report \
             lists that stretch"
                .to_string(),
        );
    }
    if sized.iter().any(|n| n.via.per_layer_change > 1) {
        notes.push(
            "a net whose layer change needs several vias in parallel gets one from the \
             router; audit_trace_current reports the shortfall"
                .to_string(),
        );
    }
    Ok(WidthPlan {
        sized,
        not_sized,
        classes,
        signals,
        differential_pairs: pairs,
        to_record,
        copper: copper_rows,
        notes,
    })
}

/// The temperature rise a new current for `net` is checked at: its rail's
/// `max_temp_rise_c` when it is a rail net that has one (as `requirements` will read it
/// once the rail has a current), else [`trace_current::DEFAULT_TEMP_RISE_C`].
fn rise_of(intent: Option<&DesignIntent>, net: &str) -> f64 {
    rail_of(intent, net)
        .and_then(|rail| rail.max_temp_rise_c)
        .unwrap_or(trace_current::DEFAULT_TEMP_RISE_C)
}

/// Adds `record` to `to_record` unless it is there already.
fn add_record(record: Record, to_record: &mut Vec<Record>) {
    if !to_record.contains(&record) {
        to_record.push(record);
    }
}

/// Whether the answer's map names a net of `net`'s rail.
fn given_rail(inputs: &Inputs<'_>, net: &str) -> bool {
    let Some(rail) = rail_of(inputs.intent, net) else {
        return false;
    };
    inputs
        .given
        .is_some_and(|given| given.keys().any(|named| rail.nets.contains(named)))
}

fn record_key(record: &Record) -> (&str, &str) {
    match record {
        Record::Currents { net, .. } => ("currents", net),
        Record::Rails { rail, .. } => ("rails", rail),
    }
}

/// What step L5 writes, against the record as it is now (`intent`):
/// - each of `proposed` (the plan's `to_record`), a `currents` entry moved to its rail
///   when the net is now a rail net, and dropped when the record now holds a current for
///   its net or rail (a recorded current wins, even over a plan made before it);
/// - each net of `given` with its current (a rail net as its rail), replacing the
///   proposal of the same section and name. `answer_refusal` against `intent` must
///   pass first.
///
/// `None` for `given` writes `proposed` as it is. In section, then rail or net, order.
pub(crate) fn final_records(
    proposed: &[Record],
    given: Option<&BTreeMap<String, f64>>,
    intent: Option<&DesignIntent>,
) -> Vec<Record> {
    let recorded = recorded_currents(intent);
    let rail_recorded = |rail: &str| {
        intent.is_some_and(|i| {
            i.rails
                .iter()
                .any(|r| r.name == rail && r.max_current_a.is_some())
        })
    };
    let mut out: Vec<Record> = Vec::new();
    for record in proposed {
        let record = match record {
            Record::Currents { net, amps } => record_for(intent, net, *amps),
            rails => rails.clone(),
        };
        let stale = match &record {
            Record::Currents { net, .. } => recorded.contains_key(net),
            Record::Rails { rail, .. } => rail_recorded(rail),
        };
        if !stale {
            add_record(record, &mut out);
        }
    }
    if let Some(given) = given {
        for (net, amps) in given {
            let record = record_for(intent, net, *amps);
            out.retain(|old| record_key(old) != record_key(&record));
            out.push(record);
        }
    }
    out.sort_by(|a, b| record_key(a).cmp(&record_key(b)));
    out
}

/// The new values of the design intent's `rails` and `currents` sections that hold
/// `records`, each `None` when no record touches it. `currents` keeps every entry it has
/// and adds the new ones in net order; `rails` keeps every rail, with the named ones'
/// `max_current_a` set. A record that the intent already holds with the same current
/// changes nothing; an empty result means everything is recorded already.
pub(crate) fn sections(
    intent: Option<&DesignIntent>,
    records: &[Record],
) -> (Option<Vec<Rail>>, Option<Vec<NetCurrent>>) {
    let mut rails: Vec<Rail> = intent.map(|i| i.rails.clone()).unwrap_or_default();
    let mut currents: Vec<NetCurrent> = intent.map(|i| i.currents.clone()).unwrap_or_default();
    let (mut rails_changed, mut currents_changed) = (false, false);
    for record in records {
        match record {
            Record::Rails { rail, amps } => {
                if let Some(entry) = rails.iter_mut().find(|r| &r.name == rail) {
                    if entry.max_current_a != Some(*amps) {
                        entry.max_current_a = Some(*amps);
                        if entry.notes.is_none() {
                            entry.notes = Some(RECORD_NOTE.to_string());
                        }
                        rails_changed = true;
                    }
                }
            }
            Record::Currents { net, amps } => match currents.iter_mut().find(|c| &c.net == net) {
                Some(entry) if entry.max_current_a == *amps => {}
                Some(entry) => {
                    entry.max_current_a = *amps;
                    currents_changed = true;
                }
                None => {
                    currents.push(NetCurrent {
                        net: net.clone(),
                        max_current_a: *amps,
                        max_temp_rise_c: None,
                        notes: Some(RECORD_NOTE.to_string()),
                    });
                    currents_changed = true;
                }
            },
        }
    }
    currents.sort_by(|a, b| a.net.cmp(&b.net));
    (
        rails_changed.then_some(rails),
        currents_changed.then_some(currents),
    )
}

/// Whether `intent` holds every one of `records` with its current: step L5 is done.
pub(crate) fn recorded(intent: Option<&DesignIntent>, records: &[Record]) -> bool {
    let (rails, currents) = sections(intent, records);
    rails.is_none()
        && currents.is_none()
        && records.iter().all(|record| match record {
            Record::Rails { rail, .. } => {
                intent.is_some_and(|i| i.rails.iter().any(|r| &r.name == rail))
            }
            Record::Currents { .. } => true,
        })
}

#[cfg(test)]
#[path = "width_plan_tests.rs"]
mod tests;
