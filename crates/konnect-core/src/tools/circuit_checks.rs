//! The checks of the circuit-aware review. Each function states the
//! checklist item it serves in `circuit_review::RULES`; thresholds are named
//! constants so a finding can quote the limit it used.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use konnect_sexp::board_geometry::{Kind, PadGeom, Shape};
use serde_json::{json, Value};

use crate::tools::circuit_model::{
    pad_distance, rail_voltage_from_name, round3, BoardView, Circuit, Part, Pin,
};
use crate::tools::circuit_review::{is_regulator, Report};
use crate::tools::component_class::{self as cc, norm_pin, PinNames};
use crate::tools::design_intent::{DesignIntent, Interface};

/// Largest decoupling-family distance is score_placement's; bulk follows its
/// 10 µF family.
const BULK_CAP_MAX_MM: f64 = 10.0;
/// Protection should sit where the trace enters the board.
const ESD_MAX_MM: f64 = 10.0;
/// A protection device this much farther from the connector than the circuit
/// it protects does not come first.
const PROTECTION_ORDER_MARGIN_MM: f64 = 1.0;
/// A bus transceiver farther than this from its connector makes a stub.
const TRANSCEIVER_MAX_MM: f64 = 25.0;
const TERMINATION_MAX_MM: f64 = 25.0;
const SERIES_TERMINATION_MAX_MM: f64 = 10.0;
const GATE_DRIVER_MAX_MM: f64 = 15.0;
const GATE_RESISTOR_MAX_MM: f64 = 5.0;
const SENSE_AMP_MAX_MM: f64 = 15.0;
const CRYSTAL_CAP_MAX_MM: f64 = 5.0;
const CRYSTAL_IC_MAX_MM: f64 = 10.0;
const BOOT_CAP_MAX_MM: f64 = 5.0;
const ADC_REF_DEFAULT_MM: f64 = 5.0;
/// Shunts are at most this resistance.
const SHUNT_MAX_OHMS: f64 = 0.1;
/// Assumed board and pin capacitance added to a crystal's two load caps.
const CRYSTAL_STRAY_PF: f64 = 3.0;
/// Copper-graph size above which the Kelvin check gives up.
const KELVIN_GRAPH_LIMIT: usize = 1500;

/// Pins of one part on a net, as `(part, pin)`.
type PinRefs<'a> = Vec<(&'a Part, &'a Pin)>;

struct Cx<'a> {
    c: &'a Circuit,
    board: Option<&'a BoardView>,
    intent: Option<&'a DesignIntent>,
}

pub(super) fn run(
    circuit: &Circuit,
    board: Option<&BoardView>,
    intent: Option<&DesignIntent>,
    report: &mut Report,
) {
    let cx = Cx {
        c: circuit,
        board,
        intent,
    };
    decoupling(&cx, report);
    bulk(&cx, report);
    connectors(&cx, report);
    transceivers(&cx, report);
    series_termination(&cx, report);
    gate_drive(&cx, report);
    current_sense(&cx, report);
    crystals(&cx, report);
    adc_refs(&cx, report);
    i2c(&cx, report);
    uart(&cx, report);
    usb(&cx, report);
}

// ─── Shared helpers ──────────────────────────────────────────────────────────

fn is_cap(p: &Part) -> bool {
    cc::capacitor_rule(&p.lib_id, &p.reference).is_some()
}

fn is_res(p: &Part) -> bool {
    cc::is_resistor(&p.lib_id, &p.reference)
}

fn is_conn(p: &Part) -> bool {
    cc::is_connector(&p.lib_id, &p.reference)
}

fn is_protection(p: &Part) -> bool {
    cc::protection_device(&p.lib_id, &p.value).is_some()
}

fn is_ic(p: &Part) -> bool {
    p.pins.len() >= 3
        && !is_cap(p)
        && !is_res(p)
        && !cc::is_series_inductor(&p.lib_id, &p.reference)
        && !is_conn(p)
        && !cc::is_crystal(&p.lib_id, &p.reference)
        && !is_protection(p)
}

fn is_mosfet(p: &Part) -> Option<(usize, usize, usize)> {
    let names = p.pin_names();
    let pins = PinNames::new(&names);
    if p.pins.len() < 3 || is_conn(p) {
        return None;
    }
    let g = pins.find(cc::MOSFET_PINS[0])?;
    let d = pins.find(cc::MOSFET_PINS[1])?;
    let s = pins.find(cc::MOSFET_PINS[2])?;
    let lib = p.lib_id.to_uppercase();
    let looks = lib.contains("MOS") || lib.contains("FET") || p.reference.starts_with('Q');
    looks.then_some((g, d, s))
}

pub(super) struct GateDriver {
    high: Option<usize>,
    low: Option<usize>,
    boot: Option<usize>,
    switch: Option<usize>,
    grounds: Vec<usize>,
}

pub(super) fn gate_driver(p: &Part) -> Option<GateDriver> {
    let names = p.pin_names();
    let pins = PinNames::new(&names);
    let high = pins.find(cc::DRIVER_HIGH_OUT);
    let low = pins.find(cc::DRIVER_LOW_OUT);
    let boot = pins.find(cc::DRIVER_BOOT);
    let switch = pins.find(cc::DRIVER_SWITCH);
    let by_lib = p.lib_id.to_uppercase().starts_with("DRIVER_FET");
    let by_pins = (high.is_some() && low.is_some()) || (high.is_some() && boot.is_some());
    if !(by_lib || by_pins) || (high.is_none() && low.is_none()) {
        return None;
    }
    Some(GateDriver {
        high,
        low,
        boot,
        switch,
        grounds: pins.all(&["GND", "COM", "VSS", "PGND", "AGND", "GNDD", "GNDA"]),
    })
}

struct CapAcross<'a> {
    part: &'a Part,
    on: &'a Pin,
    ground: &'a Pin,
    farads: Option<f64>,
}

/// Two-terminal capacitors with one pin on `net` and the other on ground.
fn caps_to_ground<'a>(c: &'a Circuit, net: &str) -> Vec<CapAcross<'a>> {
    let mut out = Vec::new();
    for (part, pin) in c.on_net(net) {
        if !is_cap(part) || !part.is_two_terminal() {
            continue;
        }
        let other = part.pins.iter().find(|p| p.number != pin.number);
        let Some(other) = other else { continue };
        if other.net == net || !c.net(&other.net).is_some_and(|n| n.is_ground()) {
            continue;
        }
        out.push(CapAcross {
            part,
            on: pin,
            ground: other,
            farads: cc::parse_si_value(&part.value),
        });
    }
    out
}

/// Two-terminal parts of one class across two nets.
fn across_pair<'a>(
    c: &'a Circuit,
    a: &str,
    b: &str,
    class: impl Fn(&Part) -> bool,
) -> Vec<&'a Part> {
    let mut out: Vec<&Part> = Vec::new();
    for (part, _) in c.on_net(a) {
        if !class(part)
            || !part.is_two_terminal()
            || out.iter().any(|p| p.reference == part.reference)
        {
            continue;
        }
        if c.across(part, a) == Some(b) {
            out.push(part);
        }
    }
    out
}

/// Nets reached from `start` through series resistors, inductors or ferrite
/// beads (not onto ground or a rail), with the hops used. `start` is at 0.
fn series_reach(c: &Circuit, start: &str, max_hops: usize) -> BTreeMap<String, usize> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    seen.insert(start.to_string(), 0);
    let mut queue = VecDeque::from([start.to_string()]);
    while let Some(net) = queue.pop_front() {
        let hops = seen[&net];
        if hops >= max_hops {
            continue;
        }
        for (part, _) in c.on_net(&net) {
            if !(is_res(part) || cc::is_series_inductor(&part.lib_id, &part.reference)) {
                continue;
            }
            let Some(other) = c.across(part, &net) else {
                continue;
            };
            let Some(n) = c.net(other) else { continue };
            if n.is_ground() || n.is_rail() || seen.contains_key(other) {
                continue;
            }
            seen.insert(other.to_string(), hops + 1);
            queue.push_back(other.to_string());
        }
    }
    seen
}

/// Protection parts with a pin on `net`.
fn protection_on<'a>(c: &'a Circuit, net: &str) -> Vec<(&'a Part, &'a Pin, cc::Protection)> {
    let mut out: Vec<(&Part, &Pin, cc::Protection)> = Vec::new();
    for (p, pin) in c.on_net(net) {
        // An array has several pins on one net: it is one protection part.
        if out.iter().any(|(q, _, _)| q.reference == p.reference) {
            continue;
        }
        if let Some(k) = cc::protection_device(&p.lib_id, &p.value) {
            out.push((p, pin, k));
        }
    }
    out
}

fn pad_of<'a>(b: &'a BoardView, part: &Part, pin: &Pin) -> Result<&'a PadGeom, String> {
    if b.is_ambiguous(&part.reference) {
        return Err(format!(
            "{} appears on the board more than once",
            part.reference
        ));
    }
    if b.footprint(&part.reference).is_none() {
        return Err(format!("{} is not on the board", part.reference));
    }
    b.pad(&part.reference, &pin.number)
        .ok_or_else(|| format!("{} has no pad {} on the board", part.reference, pin.number))
}

fn pin_dist(b: &BoardView, pa: &Part, ia: &Pin, pb: &Part, ib: &Pin) -> Result<f64, String> {
    Ok(pad_distance(pad_of(b, pa, ia)?, pad_of(b, pb, ib)?))
}

fn pin_label(part: &Part, pin: &Pin) -> String {
    format!("{}.{}", part.reference, pin.number)
}

fn net_name(c: &Circuit, key: &str) -> String {
    c.net(key)
        .map(|n| n.display())
        .unwrap_or_else(|| key.into())
}

fn leaves(c: &Circuit, keys: &[&str]) -> bool {
    keys.iter().any(|k| c.net(k).is_none_or(|n| n.leaves_sheet))
}

/// An unseen capacitor, or an unresolved far-end ground connection, can
/// change the capacitor inventory. A proved ground stays a proved ground.
fn capacitor_search_incomplete(c: &Circuit, key: &str) -> bool {
    leaves(c, &[key])
        || c.on_net(key).into_iter().any(|(p, _)| {
            is_cap(p)
                && c.across(p, key).is_some_and(|other| {
                    c.net(other)
                        .is_none_or(|n| !n.is_ground() && n.leaves_sheet)
                })
        })
}

/// Use the rule's existing search radius; this does not add traversal hops.
fn reach_incomplete(c: &Circuit, key: &str, hops: usize) -> bool {
    series_reach(c, key, hops)
        .keys()
        .any(|net| leaves(c, &[net.as_str()]))
}

fn net_unproven(c: &Circuit, report: &mut Report, rule: &'static str, key: &str, why: &str) {
    report.unproven(rule, &net_name(c, key), why.into());
    if let Some(item) = report.unproven.last_mut() {
        item["component"] = Value::Null;
        item["net"] = json!(net_name(c, key));
        item["net_key"] = json!(key);
    }
}

fn pin_unproven(report: &mut Report, rule: &'static str, part: &Part, pin: &Pin, why: &str) {
    report.unproven(
        rule,
        &part.reference,
        format!("{}: {why}", pin_label(part, pin)),
    );
    if let Some(item) = report.unproven.last_mut() {
        item["pin"] = json!(pin.number);
        item["net_key"] = json!(pin.net);
    }
}

fn intent_interface<'a>(cx: &'a Cx<'_>, net: &str) -> Option<&'a Interface> {
    let n = cx.c.net(net)?;
    cx.intent?
        .interfaces
        .iter()
        .find(|i| i.nets.iter().any(|name| n.matches_name(name)))
}

/// A rail's voltage: the record's, else what its name states.
fn net_voltage(cx: &Cx<'_>, key: &str) -> Option<(f64, &'static str)> {
    let net = cx.c.net(key)?;
    if let Some(intent) = cx.intent {
        if let Some(r) = intent
            .rails
            .iter()
            .find(|r| r.nets.iter().any(|n| net.matches_name(n)))
        {
            return Some((r.voltage_v, "design_intent"));
        }
    }
    net.names
        .iter()
        .find_map(|n| rail_voltage_from_name(n))
        .map(|v| (v, "net_name"))
}

// ─── Section 2: decoupling ───────────────────────────────────────────────────

fn decoupling(cx: &Cx<'_>, report: &mut Report) {
    let Some(board) = cx.board else { return };
    let c = cx.c;
    let mut ground_done: HashSet<(String, String)> = HashSet::new();
    for part in c.parts.iter().filter(|p| is_ic(p)) {
        for pin in &part.pins {
            if cc::power_pin_rule(&pin.electrical_type, &pin.name).is_none() {
                continue;
            }
            if norm_pin(&pin.name).starts_with("VREF") {
                continue; // adc_ref_cap_far
            }
            let Some(net) = c.net(&pin.net) else { continue };
            if net.is_ground() || cc::is_ground_name(&pin.name) {
                continue;
            }
            let family: Vec<(CapAcross, f64)> = caps_to_ground(c, &pin.net)
                .into_iter()
                .filter_map(|cap| {
                    let limit = cap.farads.and_then(|_| {
                        crate::tools::placement::decoupling_limit_mm(&cap.part.value)
                    })?;
                    Some((cap, limit))
                })
                .collect();
            let incomplete = capacitor_search_incomplete(c, &pin.net);
            if family.is_empty() {
                if incomplete {
                    pin_unproven(
                        report,
                        "decoupling_supply_far",
                        part,
                        pin,
                        "the capacitor inventory is incomplete",
                    );
                    continue;
                }
                if caps_to_ground(c, &pin.net).is_empty() {
                    report.skip(
                        "decoupling_supply_far",
                        format!(
                            "{}: no capacitor to ground on {}; audit_decoupling reports missing ones",
                            pin_label(part, pin),
                            net.display()
                        ),
                    );
                    continue;
                }
                report.needs_human(
                    "decoupling_supply_far",
                    &pin_label(part, pin),
                    format!(
                        "no capacitor on {} has a value in score_placement's families (100n, 1u, 10u)",
                        net.display()
                    ),
                );
                continue;
            }
            // The pin's highest-frequency family decides: the smallest value
            // present (100n, then 1u, then 10u), judged against its own limit.
            // A nearer cap of a larger family does not stand in for it; 10 uF
            // caps are bulk() 's concern while a smaller family is present.
            let limit = family.iter().map(|(_, l)| *l).fold(f64::INFINITY, f64::min);
            let only_bulk_family = limit >= BULK_CAP_MAX_MM;
            let mut best: Option<(&CapAcross, f64)> = None;
            let mut failures: Vec<String> = Vec::new();
            for (cap, cap_limit) in &family {
                if *cap_limit > limit {
                    continue;
                }
                match pin_dist(board, part, pin, cap.part, cap.on) {
                    Ok(d) => {
                        if best.is_none_or(|(_, bd)| d < bd) {
                            best = Some((cap, d));
                        }
                    }
                    Err(why) => failures.push(why),
                }
            }
            let Some((cap, dist)) = best else {
                report.unevaluated(
                    "decoupling_supply_far",
                    format!("{}: {}", pin_label(part, pin), failures.join("; ")),
                );
                continue;
            };
            // Only a nearby member of the smallest supported family is a
            // sufficient witness without knowing every capacitor on the net.
            let smallest_family = crate::tools::placement::decoupling_limit_mm("100n")
                .is_some_and(|minimum| limit <= minimum);
            if incomplete && (dist > limit || !smallest_family) {
                pin_unproven(
                    report,
                    "decoupling_supply_far",
                    part,
                    pin,
                    "unresolved connectivity may add a nearer or higher-frequency capacitor",
                );
                pin_unproven(
                    report,
                    "decoupling_ground_far",
                    part,
                    pin,
                    "the nearest decoupling capacitor cannot be selected from an incomplete inventory",
                );
                continue;
            }
            report.examine("decoupling_supply_far");
            let evidence = json!({
                "ic": part.reference, "pin": pin.number, "capacitor": cap.part.reference,
                "value": cap.part.value, "distance_mm": round3(dist), "limit_mm": limit,
                "metric": "pad_anchor_euclidean_xy", "net": net.display(),
                "family_judged": if only_bulk_family { "10u" } else if limit <= 2.5 { "100n" } else { "1u" },
                "no_high_frequency_cap": only_bulk_family,
            });
            if dist > limit {
                report.finding(
                    "decoupling_supply_far",
                    "warning",
                    Some(&part.reference),
                    format!(
                        "Nearest decoupling capacitor {} ({}) is {:.2} mm from {} pin '{}' (limit {} mm for its value){}",
                        cap.part.reference, cap.part.value, dist, part.reference, pin.name, limit,
                        if only_bulk_family { "; the pin has no 100 nF or 1 uF capacitor" } else { "" }
                    ),
                    format!(
                        "Move {} beside pin {} of {}; the supply connection should be the shortest on the net",
                        cap.part.reference, pin.number, part.reference
                    ),
                    evidence,
                );
            } else if only_bulk_family {
                // Near enough, but only a 10 uF capacitor: not a pass.
                report.finding(
                    "decoupling_supply_far",
                    "info",
                    Some(&part.reference),
                    format!(
                        "{} pin '{}' has only a 10 uF-class capacitor ({}, {:.2} mm) and no 100 nF or 1 uF one, so its high-frequency decoupling was not judged",
                        part.reference, pin.name, cap.part.reference, dist
                    ),
                    format!(
                        "Add a 100 nF capacitor beside pin {} of {} unless the datasheet asks for bulk capacitance alone",
                        pin.number, part.reference
                    ),
                    evidence,
                );
            } else {
                report.pass("decoupling_supply_far");
            }

            // The ground side of the nearest capacitor.
            if !ground_done.insert((part.reference.clone(), cap.part.reference.clone())) {
                continue;
            }
            let cap_ground = match pad_of(board, cap.part, cap.ground) {
                Ok(p) => p,
                Err(why) => {
                    report.unevaluated(
                        "decoupling_ground_far",
                        format!("{}: {why}", cap.part.reference),
                    );
                    continue;
                }
            };
            let Some(gnet) = cap_ground.net.as_deref() else {
                report.unevaluated(
                    "decoupling_ground_far",
                    format!("{}: its ground pad has no net on the board (sync the board from the schematic)", cap.part.reference),
                );
                continue;
            };
            let ic_pads = board.pads_on_net(&part.reference, gnet);
            let Some(nearest) = ic_pads
                .iter()
                .map(|p| (pad_distance(p, cap_ground), *p))
                .min_by(|a, b| a.0.total_cmp(&b.0))
            else {
                report.unevaluated(
                    "decoupling_ground_far",
                    format!(
                        "{}: {} has no pad on {}, the net {}'s ground pad is on (split grounds?)",
                        pin_label(part, pin),
                        part.reference,
                        gnet,
                        cap.part.reference
                    ),
                );
                continue;
            };
            if incomplete && nearest.0 > limit {
                pin_unproven(
                    report,
                    "decoupling_ground_far",
                    part,
                    pin,
                    "unresolved connectivity may add a nearer decoupling capacitor with a shorter ground leg",
                );
                continue;
            }
            report.examine("decoupling_ground_far");
            if nearest.0 <= limit {
                report.pass("decoupling_ground_far");
            } else {
                report.finding(
                    "decoupling_ground_far",
                    "warning",
                    Some(&part.reference),
                    format!(
                        "{}'s ground pad is {:.2} mm from the nearest ground pad of {} (net {}; limit {} mm)",
                        cap.part.reference, nearest.0, part.reference, gnet, limit
                    ),
                    "Bring the capacitor's ground connection to the IC's ground pin: the loop is capacitor, IC, ground, capacitor, and the ground leg counts as much as the supply leg"
                        .into(),
                    json!({
                        "ic": part.reference, "capacitor": cap.part.reference,
                        "ic_ground_pad": nearest.1.number, "ground_net": gnet,
                        "distance_mm": round3(nearest.0), "limit_mm": limit,
                        "metric": "pad_anchor_euclidean_xy",
                    }),
                );
            }
        }
    }
}

// ─── Section 2: local bulk capacitor ─────────────────────────────────────────

fn bulk(cx: &Cx<'_>, report: &mut Report) {
    let Some(board) = cx.board else { return };
    let c = cx.c;
    for part in &c.parts {
        let regulator = is_regulator(part);
        let driver = gate_driver(part).is_some();
        if !(regulator || driver) {
            continue;
        }
        let names = part.pin_names();
        let pin_names = PinNames::new(&names);
        let mut pins: Vec<usize> = Vec::new();
        if regulator {
            pins.extend(pin_names.all(&["VIN", "IN", "VI", "PVIN", "VOUT", "OUT", "VO"]));
        }
        for (i, pin) in part.pins.iter().enumerate() {
            if driver
                && cc::power_pin_rule(&pin.electrical_type, &pin.name).is_some()
                && !pins.contains(&i)
            {
                pins.push(i);
            }
        }
        for i in pins {
            let pin = &part.pins[i];
            let Some(net) = c.net(&pin.net) else { continue };
            if net.is_ground() {
                continue;
            }
            let bulk: Vec<CapAcross> = caps_to_ground(c, &pin.net)
                .into_iter()
                .filter(|cap| cap.farads.is_some_and(cc::is_bulk_capacitance))
                .collect();
            let incomplete = capacitor_search_incomplete(c, &pin.net);
            if bulk.is_empty() {
                if incomplete {
                    pin_unproven(
                        report,
                        "bulk_cap_far",
                        part,
                        pin,
                        "the bulk-capacitor inventory is incomplete",
                    );
                    continue;
                }
                report.skip(
                    "bulk_cap_far",
                    format!("{}: no bulk capacitor (10 uF or more) on {}; audit_power_rails reports missing ones", pin_label(part, pin), net.display()),
                );
                continue;
            }
            let mut best: Option<(&CapAcross, f64)> = None;
            let mut failures = Vec::new();
            for cap in &bulk {
                match pin_dist(board, part, pin, cap.part, cap.on) {
                    Ok(d) => {
                        if best.is_none_or(|(_, bd)| d < bd) {
                            best = Some((cap, d));
                        }
                    }
                    Err(why) => failures.push(why),
                }
            }
            let Some((cap, dist)) = best else {
                report.unevaluated(
                    "bulk_cap_far",
                    format!("{}: {}", pin_label(part, pin), failures.join("; ")),
                );
                continue;
            };
            if incomplete && dist > BULK_CAP_MAX_MM {
                pin_unproven(
                    report,
                    "bulk_cap_far",
                    part,
                    pin,
                    "unresolved connectivity may add a nearer bulk capacitor",
                );
                continue;
            }
            report.examine("bulk_cap_far");
            if dist <= BULK_CAP_MAX_MM {
                report.pass("bulk_cap_far");
            } else {
                report.finding(
                    "bulk_cap_far",
                    "warning",
                    Some(&part.reference),
                    format!(
                        "Nearest bulk capacitor {} ({}) is {:.1} mm from {} pin '{}' (limit {} mm)",
                        cap.part.reference, cap.part.value, dist, part.reference, pin.name, BULK_CAP_MAX_MM
                    ),
                    format!("Place a bulk capacitor within {BULK_CAP_MAX_MM} mm of {} pin {}, where the transient current is drawn", part.reference, pin.number),
                    json!({"part": part.reference, "pin": pin.number, "capacitor": cap.part.reference,
                           "distance_mm": round3(dist), "limit_mm": BULK_CAP_MAX_MM, "net": net.display(),
                           "metric": "pad_anchor_euclidean_xy"}),
                );
            }
        }
    }
}

// ─── Sections 2 and 9: protection at the connector ───────────────────────────

fn connectors(cx: &Cx<'_>, report: &mut Report) {
    let c = cx.c;
    for conn in c.parts.iter().filter(|p| is_conn(p)) {
        let mut unprotected: Vec<(String, bool)> = Vec::new(); // (net, in a recorded interface)
        for pin in &conn.pins {
            let Some(net) = c.net(&pin.net) else { continue };
            if net.is_ground() {
                continue;
            }
            let others: Vec<(&Part, &Pin)> = c
                .on_net(&pin.net)
                .into_iter()
                .filter(|(p, _)| p.reference != conn.reference)
                .collect();
            if others.is_empty() {
                if net.leaves_sheet && !net.is_rail() {
                    pin_unproven(
                        report,
                        "connector_signal_unprotected",
                        conn,
                        pin,
                        "unresolved connectivity may contain the attached circuit and its protection",
                    );
                }
                continue; // nothing attached: not a signal path
            }
            let prot = protection_on(c, &pin.net);

            if prot.is_empty() && !net.is_rail() {
                // Behind a series part?
                let reach = series_reach(c, &pin.net, 2);
                let behind: Vec<(String, &Part)> = reach
                    .keys()
                    .filter(|k| **k != pin.net)
                    .flat_map(|k| {
                        protection_on(c, k)
                            .into_iter()
                            .map(move |(p, _, _)| (k.clone(), p))
                    })
                    .collect();
                if net.leaves_sheet {
                    pin_unproven(
                        report,
                        "protection_after_series_element",
                        conn,
                        pin,
                        "the connector net is incomplete, so direct protection cannot be ruled out",
                    );
                    pin_unproven(
                        report,
                        "connector_signal_unprotected",
                        conn,
                        pin,
                        "the connector net is incomplete, so protection may be unobserved",
                    );
                    continue;
                }
                if let Some((_, p)) = behind.first() {
                    report.examine("protection_after_series_element");
                    report.finding(
                        "protection_after_series_element",
                        "warning",
                        Some(&conn.reference),
                        format!(
                            "{}.{} (net {}) reaches its protection {} only through a series part, so the pin itself is exposed",
                            conn.reference, pin.number, net.display(), p.reference
                        ),
                        "Put the TVS/ESD part on the connector net itself, before any series resistor or ferrite".into(),
                        json!({"connector": conn.reference, "pin": pin.number, "net": net.display(), "protection": p.reference}),
                    );
                } else {
                    if reach.keys().any(|key| leaves(c, &[key.as_str()])) {
                        pin_unproven(
                            report,
                            "protection_after_series_element",
                            conn,
                            pin,
                            "the existing series search reaches unresolved connectivity",
                        );
                    } else {
                        report.examine("protection_after_series_element");
                        report.pass("protection_after_series_element");
                    }
                    report.examine("connector_signal_unprotected");
                    let recorded = intent_interface(cx, &pin.net).is_some();
                    unprotected.push((net.display(), recorded));
                }
                continue;
            }
            if prot.is_empty() {
                continue; // a rail with no TVS: power-entry judgement is a human one
            }
            report.examine("connector_signal_unprotected");
            report.pass("connector_signal_unprotected");

            // Placement of the protection against the connector.
            let Some(board) = cx.board else { continue };
            let Ok(conn_pad) = pad_of(board, conn, pin) else {
                report.unevaluated(
                    "esd_far_from_connector",
                    format!(
                        "{}.{}: {}",
                        conn.reference,
                        pin.number,
                        pad_of(board, conn, pin).unwrap_err()
                    ),
                );
                continue;
            };
            let mut nearest: Option<(&Part, f64)> = None;
            let mut failures = Vec::new();
            for (tvs, tpin, _) in &prot {
                match pad_of(board, tvs, tpin) {
                    Ok(p) => {
                        let d = pad_distance(p, conn_pad);
                        if nearest.is_none_or(|(_, nd)| d < nd) {
                            nearest = Some((tvs, d));
                        }
                    }
                    Err(why) => failures.push(why),
                }
            }
            let Some((tvs, d_tvs)) = nearest else {
                report.unevaluated(
                    "esd_far_from_connector",
                    format!("{}.{}: {}", conn.reference, pin.number, failures.join("; ")),
                );
                continue;
            };
            if net.leaves_sheet && d_tvs > ESD_MAX_MM {
                pin_unproven(
                    report,
                    "esd_far_from_connector",
                    conn,
                    pin,
                    "unresolved connectivity may add nearer protection",
                );
            } else if d_tvs <= ESD_MAX_MM {
                report.examine("esd_far_from_connector");
                report.pass("esd_far_from_connector");
            } else {
                report.examine("esd_far_from_connector");
                report.finding(
                    "esd_far_from_connector",
                    "warning",
                    Some(&tvs.reference),
                    format!(
                        "{} protects {}.{} (net {}) from {:.1} mm away (limit {} mm)",
                        tvs.reference, conn.reference, pin.number, net.display(), d_tvs, ESD_MAX_MM
                    ),
                    "Place the TVS/ESD part at the connector pad, where the trace enters the board, with a short path to ground".into(),
                    json!({"protection": tvs.reference, "connector": conn.reference, "pin": pin.number,
                           "net": net.display(), "distance_mm": round3(d_tvs), "limit_mm": ESD_MAX_MM,
                           "metric": "pad_anchor_euclidean_xy"}),
                );
            }
            // Does the protected circuit sit nearer the connector than the protection?
            for (ic, ipin) in others.iter().filter(|(p, _)| is_ic(p) && !is_protection(p)) {
                let Ok(ic_pad) = pad_of(board, ic, ipin) else {
                    report.unevaluated(
                        "protection_beyond_circuit",
                        format!(
                            "{}: {}",
                            pin_label(ic, ipin),
                            pad_of(board, ic, ipin).unwrap_err()
                        ),
                    );
                    continue;
                };
                let d_ic = pad_distance(ic_pad, conn_pad);
                if net.leaves_sheet && d_ic + PROTECTION_ORDER_MARGIN_MM < d_tvs {
                    pin_unproven(
                        report,
                        "protection_beyond_circuit",
                        ic,
                        ipin,
                        "unresolved connectivity may add protection nearer the connector than this circuit",
                    );
                } else if d_ic + PROTECTION_ORDER_MARGIN_MM < d_tvs {
                    report.examine("protection_beyond_circuit");
                    report.finding(
                        "protection_beyond_circuit",
                        "warning",
                        Some(&ic.reference),
                        format!(
                            "{} is {:.1} mm from {}.{} but its protection {} is {:.1} mm away: the signal may reach the circuit before the protection",
                            pin_label(ic, ipin), d_ic, conn.reference, pin.number, tvs.reference, d_tvs
                        ),
                        "Place the protection between the connector and the circuit so the trace passes it first".into(),
                        json!({"circuit_pin": pin_label(ic, ipin), "protection": tvs.reference, "connector": conn.reference,
                               "circuit_distance_mm": round3(d_ic), "protection_distance_mm": round3(d_tvs),
                               "metric": "pad_anchor_euclidean_xy", "note": "pad distances, not trace order"}),
                    );
                } else {
                    report.examine("protection_beyond_circuit");
                    report.pass("protection_beyond_circuit");
                }
            }
        }
        if !unprotected.is_empty() {
            let recorded = unprotected.iter().any(|(_, r)| *r);
            let nets: Vec<&str> = unprotected
                .iter()
                .take(12)
                .map(|(n, _)| n.as_str())
                .collect();
            report.finding(
                "connector_signal_unprotected",
                if recorded { "warning" } else { "info" },
                Some(&conn.reference),
                format!(
                    "{} has {} signal net(s) with no TVS/ESD part on the connector net: {}",
                    conn.reference, unprotected.len(), nets.join(", ")
                ),
                "If these leave the board, add TVS/ESD protection at the connector. An internal header (programming, debug) may reasonably have none: this audit cannot tell which connectors are exposed"
                    .into(),
                json!({"connector": conn.reference, "nets": nets, "count": unprotected.len(),
                       "in_recorded_interface": recorded}),
            );
        }
    }
}

// ─── Sections 2 and 6: CAN and RS-485 transceivers ───────────────────────────

struct Transceiver<'a> {
    part: &'a Part,
    kind: &'static str,
    a: &'a Pin,
    b: &'a Pin,
}

fn transceivers_of(c: &Circuit) -> Vec<Transceiver<'_>> {
    let mut out = Vec::new();
    for part in &c.parts {
        let names = part.pin_names();
        let pins = PinNames::new(&names);
        if let (Some(h), Some(l)) = (pins.find(cc::CAN_H), pins.find(cc::CAN_L)) {
            out.push(Transceiver {
                part,
                kind: "can",
                a: &part.pins[h],
                b: &part.pins[l],
            });
            continue;
        }
        if let (Some(a), Some(b), Some(_)) = (
            pins.find(cc::RS485_A),
            pins.find(cc::RS485_B),
            pins.find(cc::RS485_DIR),
        ) {
            if !is_conn(part) {
                out.push(Transceiver {
                    part,
                    kind: "rs485",
                    a: &part.pins[a],
                    b: &part.pins[b],
                });
            }
        }
    }
    out
}

/// Connector pins reached from `net` through up to two series parts.
fn connectors_reached<'a>(c: &'a Circuit, net: &str) -> Vec<(&'a Part, &'a Pin)> {
    let mut out: Vec<(&Part, &Pin)> = Vec::new();
    for key in series_reach(c, net, 2).keys() {
        for (p, pin) in c.on_net(key) {
            if is_conn(p)
                && !out
                    .iter()
                    .any(|(q, qp)| q.reference == p.reference && qp.number == pin.number)
            {
                out.push((p, pin));
            }
        }
    }
    out
}

/// Whether the existing two-series-part bus search reaches an unresolved net.
fn bus_leaves(c: &Circuit, a: &str, b: &str) -> bool {
    reach_incomplete(c, a, 2) || reach_incomplete(c, b, 2)
}

/// A multi-pin choke or inductor on a net a bus line reaches, if any. A
/// two-pin walk cannot cross it, so what lies behind it is not seen.
fn choke_on_bus(c: &Circuit, transceiver: &Part, a: &str, b: &str) -> Option<String> {
    let mut found: Vec<String> = [a, b]
        .iter()
        .flat_map(|net| series_reach(c, net, 2).into_keys())
        .flat_map(|key| c.on_net(&key))
        .filter(|(p, _)| {
            p.reference != transceiver.reference
                && p.pins.len() >= 3
                && (cc::is_common_mode_choke(&p.lib_id, &p.reference)
                    || cc::is_series_inductor(&p.lib_id, &p.reference))
        })
        .map(|(p, _)| p.reference.clone())
        .collect();
    found.sort();
    found.dedup();
    found.into_iter().next()
}

struct Termination<'a> {
    kind: &'static str,
    resistors: Vec<&'a Part>,
    /// Ohms across the pair, `None` when a value did not parse.
    ohms: Option<f64>,
    /// The jumper, switch or two-pin header that makes it selectable. It is
    /// part of the termination, not a connector on the bus.
    selector: Option<&'a Part>,
}

/// The termination between two bus nets: a direct resistor, a split pair
/// around a common node, or a resistor and a jumper/switch in series.
fn termination<'a>(c: &'a Circuit, a: &str, b: &str) -> Vec<Termination<'a>> {
    let mut candidates = Vec::new();
    let direct = across_pair(c, a, b, is_res);
    if !direct.is_empty() {
        // Parallel resistors across the pair add up.
        let ohms = direct
            .iter()
            .map(|r| cc::parse_si_value(&r.value))
            .collect::<Option<Vec<f64>>>()
            .map(|v| 1.0 / v.iter().map(|o| 1.0 / o).sum::<f64>());
        candidates.push(Termination {
            kind: "direct",
            resistors: direct,
            ohms,
            selector: None,
        });
    }
    // Split: R from a to X and from b to X, X shared.
    for (ra, _) in c
        .on_net(a)
        .into_iter()
        .filter(|(p, _)| is_res(p) && p.is_two_terminal())
    {
        let Some(x) = c.across(ra, a) else { continue };
        if c.net(x).is_some_and(|n| n.is_ground() || n.is_rail()) || x == b {
            continue;
        }
        for (rb, _) in c
            .on_net(b)
            .into_iter()
            .filter(|(p, _)| is_res(p) && p.is_two_terminal())
        {
            if c.across(rb, b) == Some(x) && ra.reference != rb.reference {
                let ohms = match (cc::parse_si_value(&ra.value), cc::parse_si_value(&rb.value)) {
                    (Some(x), Some(y)) => Some(x + y),
                    _ => None,
                };
                candidates.push(Termination {
                    kind: "split",
                    resistors: vec![ra, rb],
                    ohms,
                    selector: None,
                });
            }
        }
    }
    // Selectable: R from a to X, a jumper, switch or two-pin header from X to b.
    for (left, right) in [(a, b), (b, a)] {
        for (ra, _) in c
            .on_net(left)
            .into_iter()
            .filter(|(p, _)| is_res(p) && p.is_two_terminal())
        {
            let Some(x) = c.across(ra, left) else {
                continue;
            };
            if c.net(x).is_some_and(|n| n.is_ground() || n.is_rail()) {
                continue;
            }
            for (sw, _) in c.on_net(x) {
                let lib = sw.lib_id.to_uppercase();
                let selector = lib.contains("SW_")
                    || lib.contains("SWITCH")
                    || cc::is_jumper(&sw.lib_id, &sw.reference)
                    || is_conn(sw);
                if selector && sw.is_two_terminal() && c.across(sw, x) == Some(right) {
                    candidates.push(Termination {
                        kind: "selectable",
                        resistors: vec![ra],
                        ohms: cc::parse_si_value(&ra.value),
                        selector: Some(sw),
                    });
                }
            }
        }
    }
    // The same selectable branch can be reached from both directions in
    // degenerate drawings; its physical resistor/selector identity is one.
    let mut seen = BTreeSet::new();
    candidates.retain(|t| {
        seen.insert((
            t.kind,
            t.resistors
                .iter()
                .map(|r| r.reference.as_str())
                .collect::<Vec<_>>(),
            t.selector.map(|p| p.reference.as_str()),
        ))
    });
    candidates
}

fn transceivers(cx: &Cx<'_>, report: &mut Report) {
    let c = cx.c;
    let transceivers = transceivers_of(c);
    let mut termination_placement_done = BTreeSet::new();
    for x in &transceivers {
        let (ra, rb) = (x.a.net.as_str(), x.b.net.as_str());
        let incomplete_bus = bus_leaves(c, ra, rb);
        let (value_rule, id) = if x.kind == "can" {
            ("can_termination_value", "CAN")
        } else {
            ("rs485_termination_value", "RS-485")
        };
        let candidates = termination(c, ra, rb);
        let term = candidates.first();
        let observed_resistors: BTreeMap<&str, &Part> = candidates
            .iter()
            .flat_map(|t| t.resistors.iter().map(|r| (r.reference.as_str(), *r)))
            .collect();
        // All observed selectors belong to candidate terminations, not bus branches.
        let selectors: BTreeSet<&str> = candidates
            .iter()
            .filter_map(|t| t.selector.map(|p| p.reference.as_str()))
            .collect();
        let bus_connectors = |net: &str| -> Vec<(&Part, &Pin)> {
            connectors_reached(c, net)
                .into_iter()
                .filter(|(p, _)| !selectors.contains(p.reference.as_str()))
                .collect()
        };
        let conns_a = bus_connectors(ra);
        let conns_b = bus_connectors(rb);
        let mut block = json!({
            "interface": x.kind,
            "transceiver": x.part.reference,
            "net_key": ra,
            "nets": {"a": net_name(c, ra), "b": net_name(c, rb)},
            "connectivity_complete": !incomplete_bus,
            "connectors": conns_a.iter().map(|(p, pin)| pin_label(p, pin)).collect::<Vec<_>>(),
            "physical_ends": "needs_human: this audit sees one node, not the bus",
        });

        // Termination value.
        match &term {
            None => {
                block["termination"] = json!({"kind": "none_observed"});
                if incomplete_bus {
                    block["termination"]["note"] =
                        json!("the bus search reaches unresolved connectivity; termination may be unobserved");
                    report.unproven(
                        value_rule,
                        &x.part.reference,
                        "termination cannot be ruled out on an incomplete bus".into(),
                    );
                }
            }
            Some(t) => {
                let incomplete_term = incomplete_bus
                    || t.resistors
                        .iter()
                        .any(|r| r.pins.iter().any(|p| leaves(c, &[p.net.as_str()])));
                // Electrical parallelism does not establish which physical
                // bus end owns each direct resistor. Do not use sheet or
                // drawing proximity as an invented bank assignment.
                let bank_assignment_unknown =
                    candidates.len() > 1 || (t.kind == "direct" && t.resistors.len() > 1);
                block["termination"] = json!({
                    "bank_assignment_requires_human": bank_assignment_unknown,
                    "kind": t.kind,
                    "resistors": t.resistors.iter().map(|r| json!({"reference": r.reference, "value": r.value})).collect::<Vec<_>>(),
                    "total_ohms": if incomplete_term || candidates.len() > 1 { None } else { t.ohms },
                    "observed_resistance_ohms": if candidates.len() == 1 { t.ohms } else { None },
                    "observed_resistors": observed_resistors.keys().collect::<Vec<_>>(),
                    "candidates": candidates.iter().map(|candidate| json!({
                        "kind": candidate.kind, "ohms": candidate.ohms,
                        "selector": candidate.selector.map(|p| &p.reference),
                        "resistors": candidate.resistors.iter().map(|r| &r.reference).collect::<Vec<_>>()
                    })).collect::<Vec<_>>(),
                    "connectivity_complete": !incomplete_term,
                    "selector": t.selector.map(|p| &p.reference),
                });
                let candidate_value = if candidates.iter().any(|c| c.ohms.is_none()) {
                    None
                } else {
                    t.ohms
                };
                // More parallel paths only lower the resistance between two
                // nodes. A fixed bank whose own nodes are all known is therefore
                // an upper bound even when the bus continues out of sight. A
                // selectable bank may be open, so it bounds nothing.
                let positive =
                    |r: &&Part| cc::parse_si_value(&r.value).filter(|v| v.is_finite() && *v > 0.0);
                let nodes_known = |r: &&Part| {
                    positive(r).is_some()
                        && r.pins
                            .iter()
                            .all(|p| p.net == ra || p.net == rb || !leaves(c, &[p.net.as_str()]))
                };
                let bound_holds =
                    matches!(t.kind, "direct" | "split") && t.resistors.iter().all(nodes_known);
                // Among several candidates the end each bank sits at is not
                // known, but a bank's own value is. Direct resistors may sit
                // at different ends, so each is a bank of its own; the two
                // halves of a split are one bank.
                let low_bank = candidates
                    .iter()
                    .flat_map(|bank| match bank.kind {
                        "direct" => bank
                            .resistors
                            .iter()
                            .filter_map(|r| positive(r).map(|v| ("direct", vec![*r], v)))
                            .collect(),
                        "split" if bank.resistors.iter().all(nodes_known) => bank
                            .ohms
                            .map(|v| ("split", bank.resistors.clone(), v))
                            .into_iter()
                            .collect(),
                        _ => Vec::new(),
                    })
                    .filter(|(_, _, ohms)| *ohms < 114.0)
                    .min_by(|a, b| a.2.total_cmp(&b.2));
                match candidate_value {
                    None => report.unevaluated(
                        value_rule,
                        format!(
                            "{}: a termination resistor value does not parse",
                            x.part.reference
                        ),
                    ),
                    Some(_) if bank_assignment_unknown => match low_bank {
                        // Whichever physical end holds this bank is at most
                        // its value, and nothing in parallel can raise that.
                        Some((kind, bank, ohms)) => {
                            let names: Vec<&str> =
                                bank.iter().map(|r| r.reference.as_str()).collect();
                            report.examine(value_rule);
                            report.finding(
                                value_rule,
                                "warning",
                                Some(&x.part.reference),
                                format!(
                                    "{id} termination at {}: {} ({kind}) puts {:.0} ohm across the pair, so the bus end that holds it is at most {:.0} ohm whichever end that is; the bus wants 120 ohm (114 to 126) at each end",
                                    x.part.reference, names.join(" + "), ohms, ohms
                                ),
                                "Use 120 ohm across the pair (or two 60 ohm in a split termination) at each physical end of the bus".into(),
                                json!({"transceiver": x.part.reference, "kind": kind,
                                       "low_bank": names, "upper_bound_ohms": round3(ohms),
                                       "bank_assignment_requires_human": true,
                                       "resistors": observed_resistors.keys().collect::<Vec<_>>(),
                                       "connectivity_complete": !incomplete_term}),
                            );
                        }
                        None => report.needs_human(
                            value_rule, &x.part.reference,
                            "multiple termination candidates are observed, but their assignment to physical bus ends is not established",
                        ),
                    },
                    Some(ohms) if incomplete_term && !(ohms < 114.0 && bound_holds) => report.unproven(
                        value_rule,
                        &x.part.reference,
                        "unresolved bus or termination connectivity can change the equivalent resistance".into(),
                    ),
                    Some(ohms) => {
                        report.examine(value_rule);
                        let split_even = t.kind != "split" || {
                            let v: Vec<f64> = t
                                .resistors
                                .iter()
                                .filter_map(|r| cc::parse_si_value(&r.value))
                                .collect();
                            v.len() == 2 && (v[0] - v[1]).abs() <= 0.05 * v[0].max(v[1])
                        };
                        if (114.0..=126.0).contains(&ohms) && split_even {
                            report.pass(value_rule);
                        } else {
                            report.finding(
                                value_rule,
                                "warning",
                                Some(&x.part.reference),
                                format!(
                                    "{id} {}termination at {} gives {:.0} ohm ({}); the bus wants 120 ohm (114 to 126) at each end{}",
                                    if incomplete_term { "observed " } else { "" },
                                    x.part.reference, ohms, t.kind,
                                    if incomplete_term { "; additional parallel resistors cannot raise this resistance" } else { "" }
                                ),
                                "Use 120 ohm across the pair (or two 60 ohm in a split termination) at each physical end of the bus".into(),
                                json!({"transceiver": x.part.reference, "kind": t.kind, "total_ohms": round3(ohms),
                                       "resistors": t.resistors.iter().map(|r| &r.reference).collect::<Vec<_>>(),
                                       "connectivity_complete": !incomplete_term}),
                            );
                        }
                    }
                }
            }
        }

        // Linear bus: three or more connectors on one bus line.
        if x.kind == "can" {
            let distinct: BTreeSet<&str> =
                conns_a.iter().map(|(p, _)| p.reference.as_str()).collect();
            if distinct.len() > 2 {
                report.examine("can_bus_branches");
                report.finding(
                    "can_bus_branches",
                    "warning",
                    Some(&x.part.reference),
                    format!(
                        "{} connectors ({}) hang on the CAN bus at {}: a linear bus has one in and one out",
                        distinct.len(), distinct.iter().copied().collect::<Vec<_>>().join(", "), x.part.reference
                    ),
                    "Keep the bus linear (in, node, out) with short stubs; a star needs a different topology".into(),
                    json!({"transceiver": x.part.reference, "connectors": distinct}),
                );
            } else if let Some(choke) = choke_on_bus(c, x.part, ra, rb) {
                report.unproven(
                    "can_bus_branches",
                    &x.part.reference,
                    format!(
                        "the bus passes through {choke}, a multi-pin choke whose winding pairs are not read, so the connectors behind it were not counted"
                    ),
                );
            } else if incomplete_bus {
                report.unproven(
                    "can_bus_branches",
                    &x.part.reference,
                    "the bus search reaches unresolved connectivity, where more connectors may be attached".into(),
                );
            } else if distinct.is_empty() {
                report.unevaluated(
                    "can_bus_branches",
                    format!(
                        "{}: no connector is reached from CANH within two series parts, so the bus topology cannot be judged",
                        x.part.reference
                    ),
                );
            } else {
                report.examine("can_bus_branches");
                report.pass("can_bus_branches");
            }
        }

        // RS-485 fail-safe biasing.
        if x.kind == "rs485" {
            let bias = |net: &str, to_rail: bool| -> Vec<&Part> {
                c.on_net(net)
                    .into_iter()
                    .filter(|(p, _)| is_res(p) && p.is_two_terminal())
                    .filter(|(p, _)| {
                        c.across(p, net).and_then(|o| c.net(o)).is_some_and(|n| {
                            if to_rail {
                                n.is_rail()
                            } else {
                                n.is_ground()
                            }
                        })
                    })
                    .map(|(p, _)| p)
                    .collect()
            };
            let up = bias(ra, true);
            let down = bias(rb, false);
            if up.is_empty() || down.is_empty() {
                let bias_incomplete = |net: &str, to_rail: bool| {
                    leaves(c, &[net])
                        || c.on_net(net).into_iter().any(|(p, _)| {
                            is_res(p)
                                && c.across(p, net).is_some_and(|other| {
                                    c.net(other).is_none_or(|n| {
                                        !(if to_rail { n.is_rail() } else { n.is_ground() })
                                            && n.leaves_sheet
                                    })
                                })
                        })
                };
                if (up.is_empty() && bias_incomplete(ra, true))
                    || (down.is_empty() && bias_incomplete(rb, false))
                {
                    report.unproven(
                        "rs485_failsafe_bias",
                        &x.part.reference,
                        "unresolved connectivity prevents a complete bias-resistor inventory"
                            .into(),
                    );
                } else {
                    report.examine("rs485_failsafe_bias");
                    report.finding(
                        "rs485_failsafe_bias",
                        "info",
                        Some(&x.part.reference),
                        format!(
                            "{} has no bias resistor {} on the RS-485 pair",
                            x.part.reference,
                            match (up.is_empty(), down.is_empty()) {
                                (true, true) => "from A to a rail or from B to ground",
                                (true, false) => "from A to a rail",
                                _ => "from B to ground",
                            }
                        ),
                        "Bias the idle bus (A up, B down) unless the transceiver has fail-safe built in: check its datasheet".into(),
                        json!({"transceiver": x.part.reference, "value": x.part.value}),
                    );
                }
            } else {
                report.examine("rs485_failsafe_bias");
                report.pass("rs485_failsafe_bias");
            }
            block["failsafe_bias"] = json!({
                "a_to_rail": up.iter().map(|p| &p.reference).collect::<Vec<_>>(),
                "b_to_ground": down.iter().map(|p| &p.reference).collect::<Vec<_>>(),
            });
        }

        // Board distances.
        if let Some(board) = cx.board {
            let mut worst: Option<f64> = None;
            let mut gaps = Vec::new();
            let mut reached_any = false;
            let mut both_sides_measured = true;
            for (pin, conns) in [(x.a, &conns_a), (x.b, &conns_b)] {
                let mut nearest: Option<f64> = None;
                for (cp, cpin) in conns {
                    reached_any = true;
                    match pin_dist(board, x.part, pin, cp, cpin) {
                        Ok(d) => nearest = Some(nearest.map_or(d, |n: f64| n.min(d))),
                        Err(why) => gaps.push(why),
                    }
                }
                if let Some(n) = nearest {
                    worst = Some(worst.map_or(n, |w: f64| w.max(n)));
                } else {
                    both_sides_measured = false;
                }
            }
            if !reached_any {
                if incomplete_bus {
                    report.unproven(
                        "transceiver_far_from_connector",
                        &x.part.reference,
                        "the bus search reaches unresolved connectivity".into(),
                    );
                } else {
                    report.skip(
                        "transceiver_far_from_connector",
                        format!(
                            "{}: no connector on the bus pins within two series parts",
                            x.part.reference
                        ),
                    );
                }
            } else if let Some(w) = worst {
                if incomplete_bus && (w > TRANSCEIVER_MAX_MM || !both_sides_measured) {
                    report.unproven(
                        "transceiver_far_from_connector",
                        &x.part.reference,
                        "the nearest connector on each bus line cannot be established from incomplete connectivity".into(),
                    );
                } else {
                    report.examine("transceiver_far_from_connector");
                    block["connector_distance_mm"] = json!(round3(w));
                    if w <= TRANSCEIVER_MAX_MM {
                        report.pass("transceiver_far_from_connector");
                    } else {
                        report.finding(
                            "transceiver_far_from_connector",
                            "warning",
                            Some(&x.part.reference),
                            format!("{} is {:.1} mm from its bus connector (limit {} mm)", x.part.reference, w, TRANSCEIVER_MAX_MM),
                            "Place the transceiver (and its protection) near the bus connector; a long stub on the bus side reflects".into(),
                            json!({"transceiver": x.part.reference, "distance_mm": round3(w), "limit_mm": TRANSCEIVER_MAX_MM,
                                   "metric": "pad_anchor_euclidean_xy"}),
                        );
                    }
                }
            } else {
                report.unevaluated(
                    "transceiver_far_from_connector",
                    format!("{}: {}", x.part.reference, gaps.join("; ")),
                );
            }

            // Termination at the endpoint.
            if term.is_some() {
                for r in observed_resistors.values() {
                    let pair = if ra <= rb { (ra, rb) } else { (rb, ra) };
                    if !termination_placement_done.insert((pair.0, pair.1, r.reference.as_str())) {
                        continue;
                    }
                    let mut nearest: Option<(f64, String)> = None;
                    let mut err = None;
                    for pin in &r.pins {
                        let targets: Vec<(&Part, &Pin)> = conns_a
                            .iter()
                            .chain(conns_b.iter())
                            .copied()
                            .chain(
                                transceivers
                                    .iter()
                                    .filter(|other| {
                                        (other.a.net == ra && other.b.net == rb)
                                            || (other.a.net == rb && other.b.net == ra)
                                    })
                                    .flat_map(|other| {
                                        [(other.part, other.a), (other.part, other.b)]
                                    }),
                            )
                            .collect();
                        for (tp, tpin) in targets {
                            match pin_dist(board, r, pin, tp, tpin) {
                                Ok(d) => {
                                    if nearest.as_ref().is_none_or(|(n, _)| d < *n) {
                                        nearest = Some((d, pin_label(tp, tpin)));
                                    }
                                }
                                Err(why) => err = Some(why),
                            }
                        }
                    }
                    match (nearest, err) {
                        (Some((d, to)), measurement_error) => {
                            if d > TERMINATION_MAX_MM {
                                if let Some(why) = measurement_error {
                                    report.unevaluated("termination_far_from_endpoint", format!("{}: an eligible endpoint cannot be measured ({why}); the nearest endpoint is unproven", r.reference));
                                    continue;
                                }
                            }
                            if incomplete_bus && d > TERMINATION_MAX_MM {
                                report.unproven(
                                    "termination_far_from_endpoint",
                                    &r.reference,
                                    "unresolved connectivity may add a nearer bus endpoint".into(),
                                );
                                continue;
                            }
                            report.examine("termination_far_from_endpoint");
                            if d <= TERMINATION_MAX_MM {
                                report.pass("termination_far_from_endpoint");
                            } else {
                                report.finding(
                                    "termination_far_from_endpoint",
                                    "warning",
                                    Some(&r.reference),
                                    format!(
                                        "Termination {} is {:.1} mm from the nearest of the connector and transceiver pins ({to}); limit {} mm",
                                        r.reference, d, TERMINATION_MAX_MM
                                    ),
                                    "Put the termination at the physical end of the bus: at the connector, or at the transceiver beside it".into(),
                                    json!({"resistor": r.reference, "nearest": to, "distance_mm": round3(d), "limit_mm": TERMINATION_MAX_MM}),
                                );
                            }
                        }
                        (None, Some(why)) => report.unevaluated(
                            "termination_far_from_endpoint",
                            format!("{}: {why}", r.reference),
                        ),
                        (None, None) => {}
                    }
                }
            }
        }
        report.interfaces.push(block);
    }
}

// ─── Section 2: series termination ───────────────────────────────────────────

fn is_driver_type(t: &str) -> bool {
    matches!(
        t,
        "output" | "tri_state" | "bidirectional" | "open_collector" | "open_emitter"
    )
}

fn series_termination(cx: &Cx<'_>, report: &mut Report) {
    let Some(board) = cx.board else { return };
    let c = cx.c;
    for r in c.parts.iter().filter(|p| is_res(p) && p.is_two_terminal()) {
        let Some(ohms) = cc::parse_si_value(&r.value).filter(|o| (10.0..=100.0).contains(o)) else {
            continue;
        };
        let (n0, n1) = (r.pins[0].net.as_str(), r.pins[1].net.as_str());
        if n0 == n1 {
            continue;
        }
        let plain = |k: &str| c.net(k).is_some_and(|n| !n.is_ground() && !n.is_rail());
        if !(plain(n0) && plain(n1)) {
            continue;
        }
        if leaves(c, &[n0, n1]) {
            report.unproven(
                "series_termination_far_from_driver",
                &r.reference,
                "unresolved connectivity prevents proving exactly one driver and the receiver-only side".into(),
            );
            continue;
        }
        // Which side holds exactly one driver, and is the other side all receivers?
        let side = |key: &str| -> (PinRefs<'_>, PinRefs<'_>) {
            let on: Vec<(&Part, &Pin)> = c
                .on_net(key)
                .into_iter()
                .filter(|(p, _)| p.reference != r.reference && (is_ic(p) || is_mosfet(p).is_some()))
                .collect();
            let drivers = on
                .iter()
                .filter(|(_, pin)| is_driver_type(&pin.electrical_type))
                .copied()
                .collect();
            let receivers = on
                .iter()
                .filter(|(_, pin)| pin.electrical_type == "input")
                .copied()
                .collect();
            (drivers, receivers)
        };
        let ((d0, rx0), (d1, rx1)) = (side(n0), side(n1));
        let (driver_side, drv, recv, recv_side) =
            if d0.len() == 1 && d1.is_empty() && !rx1.is_empty() && rx0.is_empty() {
                (0usize, d0[0], rx1, 1usize)
            } else if d1.len() == 1 && d0.is_empty() && !rx0.is_empty() && rx1.is_empty() {
                (1usize, d1[0], rx0, 0usize)
            } else {
                continue; // direction not provable from the pin types
            };
        // A resistor into a MOSFET gate is a gate resistor.
        if recv
            .iter()
            .any(|(p, pin)| is_mosfet(p).is_some_and(|(g, _, _)| p.pins[g].number == pin.number))
        {
            continue;
        }
        let r_drv = &r.pins[driver_side];
        let r_rcv = &r.pins[recv_side];
        let d_driver = match pin_dist(board, r, r_drv, drv.0, drv.1) {
            Ok(d) => d,
            Err(why) => {
                report.unevaluated(
                    "series_termination_far_from_driver",
                    format!("{}: {why}", r.reference),
                );
                continue;
            }
        };
        let d_recv = recv
            .iter()
            .filter_map(|(p, pin)| pin_dist(board, r, r_rcv, p, pin).ok())
            .fold(f64::INFINITY, f64::min);
        report.examine("series_termination_far_from_driver");
        if d_driver > SERIES_TERMINATION_MAX_MM && d_driver > d_recv {
            report.finding(
                "series_termination_far_from_driver",
                "warning",
                Some(&r.reference),
                format!(
                    "Series resistor {} ({} ohm) is {:.1} mm from its driver {} but {:.1} mm from the nearest receiver",
                    r.reference, round3(ohms), d_driver, pin_label(drv.0, drv.1), d_recv
                ),
                "A source termination belongs at the driver: move the resistor next to the driver pin".into(),
                json!({"resistor": r.reference, "driver": pin_label(drv.0, drv.1), "driver_distance_mm": round3(d_driver),
                       "receiver_distance_mm": if d_recv.is_finite() { json!(round3(d_recv)) } else { Value::Null },
                       "limit_mm": SERIES_TERMINATION_MAX_MM, "metric": "pad_anchor_euclidean_xy"}),
            );
        } else {
            report.pass("series_termination_far_from_driver");
        }
    }
}

// ─── Sections 2 and 8: gate drive ────────────────────────────────────────────

/// MOSFETs driven from a gate-driver output pin, through at most one resistor.
struct Driven<'a> {
    mosfet: &'a Part,
    gate: &'a Pin,
    source: &'a Pin,
    resistor: Option<&'a Part>,
    driver: &'a Part,
    out_pin: &'a Pin,
    high_side: bool,
}

fn driven_mosfets(c: &Circuit) -> Vec<Driven<'_>> {
    let mut out = Vec::new();
    for driver in &c.parts {
        let Some(gd) = gate_driver(driver) else {
            continue;
        };
        for (idx, high_side) in [(gd.high, true), (gd.low, false)] {
            let Some(idx) = idx else { continue };
            let out_pin = &driver.pins[idx];
            for (key, via) in series_reach(c, &out_pin.net, 1) {
                for (m, mpin) in c.on_net(&key) {
                    let Some((g, _, s)) = is_mosfet(m) else {
                        continue;
                    };
                    if m.pins[g].number != mpin.number {
                        continue;
                    }
                    let resistor = if via == 0 {
                        None
                    } else {
                        c.on_net(&out_pin.net)
                            .into_iter()
                            .map(|(p, _)| p)
                            .find(|p| {
                                is_res(p)
                                    && p.is_two_terminal()
                                    && c.across(p, &out_pin.net) == Some(key.as_str())
                            })
                    };
                    out.push(Driven {
                        mosfet: m,
                        gate: &m.pins[g],
                        source: &m.pins[s],
                        resistor,
                        driver,
                        out_pin,
                        high_side,
                    });
                }
            }
        }
    }
    out
}

fn gate_drive(cx: &Cx<'_>, report: &mut Report) {
    let c = cx.c;
    for d in driven_mosfets(c) {
        if let Some(board) = cx.board {
            // Driver near the MOSFET.
            match pin_dist(board, d.driver, d.out_pin, d.mosfet, d.gate) {
                Err(why) => report.unevaluated(
                    "gate_driver_far_from_mosfets",
                    format!("{}: {why}", d.driver.reference),
                ),
                Ok(dist) => {
                    report.examine("gate_driver_far_from_mosfets");
                    if dist <= GATE_DRIVER_MAX_MM {
                        report.pass("gate_driver_far_from_mosfets");
                    } else {
                        report.finding(
                            "gate_driver_far_from_mosfets",
                            "warning",
                            Some(&d.driver.reference),
                            format!(
                                "{} output {} is {:.1} mm from the gate of {} (limit {} mm)",
                                d.driver.reference, d.out_pin.name, dist, d.mosfet.reference, GATE_DRIVER_MAX_MM
                            ),
                            "Place the gate driver close to the MOSFETs it drives: the gate loop (driver, gate, source, driver return) must stay small".into(),
                            json!({"driver": d.driver.reference, "mosfet": d.mosfet.reference, "distance_mm": round3(dist),
                                   "limit_mm": GATE_DRIVER_MAX_MM, "metric": "pad_anchor_euclidean_xy"}),
                        );
                    }
                }
            }
            // Gate resistor beside the gate.
            if let Some(r) = d.resistor {
                let on_gate = r.pins.iter().find(|p| p.net == d.gate.net);
                if let Some(rp) = on_gate {
                    match pin_dist(board, r, rp, d.mosfet, d.gate) {
                        Err(why) => report.unevaluated(
                            "gate_resistor_far_from_gate",
                            format!("{}: {why}", r.reference),
                        ),
                        Ok(dist) => {
                            report.examine("gate_resistor_far_from_gate");
                            if dist <= GATE_RESISTOR_MAX_MM {
                                report.pass("gate_resistor_far_from_gate");
                            } else {
                                report.finding(
                                    "gate_resistor_far_from_gate",
                                    "warning",
                                    Some(&r.reference),
                                    format!(
                                        "Gate resistor {} is {:.1} mm from the gate of {} (limit {} mm)",
                                        r.reference, dist, d.mosfet.reference, GATE_RESISTOR_MAX_MM
                                    ),
                                    "Put the gate resistor beside the MOSFET gate pad".into(),
                                    json!({"resistor": r.reference, "mosfet": d.mosfet.reference, "distance_mm": round3(dist),
                                           "limit_mm": GATE_RESISTOR_MAX_MM, "metric": "pad_anchor_euclidean_xy"}),
                                );
                            }
                        }
                    }
                }
            }
        }

        // The driver's return against the MOSFET source.
        let gd = gate_driver(d.driver).expect("driven_mosfets lists drivers");
        let names = d.driver.pin_names();
        let pins = PinNames::new(&names);
        let return_nets: Vec<&str> = if d.high_side {
            gd.switch
                .map(|i| vec![d.driver.pins[i].net.as_str()])
                .unwrap_or_default()
        } else {
            let _ = &pins;
            gd.grounds
                .iter()
                .map(|&i| d.driver.pins[i].net.as_str())
                .collect()
        };
        if return_nets.is_empty() {
            report.skip(
                "gate_drive_return_net",
                format!(
                    "{}: no {} pin found",
                    d.driver.reference,
                    if d.high_side { "switch-node" } else { "ground" }
                ),
            );
            continue;
        }
        if return_nets.contains(&d.source.net.as_str()) {
            report.examine("gate_drive_return_net");
            report.pass("gate_drive_return_net");
        } else if leaves(c, &[d.source.net.as_str()]) || leaves(c, &return_nets) {
            report.unproven(
                "gate_drive_return_net",
                &d.driver.reference,
                format!(
                    "the return or source connection of {} is unresolved",
                    d.mosfet.reference
                ),
            );
        } else {
            report.examine("gate_drive_return_net");
            report.finding(
                "gate_drive_return_net",
                "warning",
                Some(&d.driver.reference),
                format!(
                    "{} returns on {} but the {}-side MOSFET {} has its source on {}",
                    d.driver.reference,
                    return_nets.iter().map(|k| net_name(c, k)).collect::<Vec<_>>().join("/"),
                    if d.high_side { "high" } else { "low" },
                    d.mosfet.reference,
                    net_name(c, &d.source.net)
                ),
                "Give the gate drive its own return to the MOSFET source (or the source side of the shunt): the gate current must not flow through a shared or measured path".into(),
                json!({"driver": d.driver.reference, "mosfet": d.mosfet.reference,
                       "driver_return": return_nets.iter().map(|k| net_name(c, k)).collect::<Vec<_>>(),
                       "source_net": net_name(c, &d.source.net)}),
            );
        }
    }

    // Bootstrap capacitors, per driver.
    for driver in &c.parts {
        let Some(gd) = gate_driver(driver) else {
            continue;
        };
        let (Some(bi), Some(si)) = (gd.boot, gd.switch) else {
            continue;
        };
        let (boot, sw) = (&driver.pins[bi], &driver.pins[si]);
        let caps = across_pair(c, &boot.net, &sw.net, is_cap);
        if caps.is_empty() {
            if leaves(c, &[boot.net.as_str(), sw.net.as_str()]) {
                report.unproven(
                    "bootstrap_cap_missing",
                    &driver.reference,
                    "the boot or switch net has unresolved connectivity".into(),
                );
            } else {
                report.examine("bootstrap_cap_missing");
                report.finding(
                    "bootstrap_cap_missing",
                    "warning",
                    Some(&driver.reference),
                    format!("{} has no capacitor between its boot pin '{}' and switch pin '{}'", driver.reference, boot.name, sw.name),
                    "Add the bootstrap capacitor between the boot and switch pins; it supplies the high-side gate drive".into(),
                    json!({"driver": driver.reference, "boot": boot.name, "switch": sw.name}),
                );
            }
            continue;
        }
        report.examine("bootstrap_cap_missing");
        report.pass("bootstrap_cap_missing");
        let Some(board) = cx.board else { continue };
        let cap = caps[0];
        let leg = |net: &str| cap.pins.iter().find(|p| p.net == net);
        let (Some(cb), Some(cs)) = (leg(&boot.net), leg(&sw.net)) else {
            continue;
        };
        match (
            pin_dist(board, cap, cb, driver, boot),
            pin_dist(board, cap, cs, driver, sw),
        ) {
            (Ok(d1), Ok(d2)) => {
                report.examine("bootstrap_cap_far");
                if d1.max(d2) <= BOOT_CAP_MAX_MM {
                    report.pass("bootstrap_cap_far");
                } else {
                    report.finding(
                        "bootstrap_cap_far",
                        "warning",
                        Some(&cap.reference),
                        format!(
                            "Bootstrap capacitor {} is {:.1} mm from {}'s boot pin and {:.1} mm from its switch pin (limit {} mm)",
                            cap.reference, d1, driver.reference, d2, BOOT_CAP_MAX_MM
                        ),
                        "Place the bootstrap capacitor directly beside the driver's boot and switch pins".into(),
                        json!({"capacitor": cap.reference, "driver": driver.reference, "boot_distance_mm": round3(d1),
                               "switch_distance_mm": round3(d2), "limit_mm": BOOT_CAP_MAX_MM}),
                    );
                }
            }
            (Err(why), _) | (_, Err(why)) => {
                report.unevaluated("bootstrap_cap_far", format!("{}: {why}", cap.reference));
            }
        }
    }
}

// ─── Sections 2 and 8: current sense ─────────────────────────────────────────

fn current_sense(cx: &Cx<'_>, report: &mut Report) {
    let c = cx.c;
    for amp in &c.parts {
        let names = amp.pin_names();
        let pins = PinNames::new(&names);
        let (Some(pi), Some(ni)) = (pins.find(cc::SENSE_IN_P), pins.find(cc::SENSE_IN_N)) else {
            continue;
        };
        if is_cap(amp) || is_res(amp) || is_conn(amp) || amp.pins.len() < 4 {
            continue;
        }
        let (ip, inn) = (&amp.pins[pi], &amp.pins[ni]);
        // Candidate shunt: low-value resistor across the two input sides,
        // each reached directly or through one filter resistor.
        let reach_p = series_reach(c, &ip.net, 1);
        let reach_n = series_reach(c, &inn.net, 1);
        // The pair of nets nearest to the two inputs wins: the shunt itself
        // is a series resistor and would otherwise let the sides swap.
        let mut found: Option<(&Part, String, String)> = None;
        let mut best_hops = usize::MAX;
        for (kp, hp) in &reach_p {
            for (shunt, _) in c.on_net(kp) {
                if !is_res(shunt) || !shunt.is_two_terminal() {
                    continue;
                }
                let Some(ohms) = cc::parse_si_value(&shunt.value) else {
                    continue;
                };
                if ohms > SHUNT_MAX_OHMS {
                    continue;
                }
                let Some(other) = c.across(shunt, kp) else {
                    continue;
                };
                let Some(hn) = reach_n.get(other) else {
                    continue;
                };
                if hp + hn < best_hops {
                    best_hops = hp + hn;
                    found = Some((shunt, kp.clone(), other.to_string()));
                }
            }
        }
        let Some((shunt, np, nn)) = found else {
            if reach_p
                .keys()
                .chain(reach_n.keys())
                .any(|key| leaves(c, &[key.as_str()]))
            {
                report.unproven(
                    "sense_amp_far_from_shunt",
                    &amp.reference,
                    "the existing one-hop sense search reaches unresolved connectivity".into(),
                );
                continue;
            }
            report.skip(
                "sense_amp_far_from_shunt",
                format!(
                    "{}: no resistor of {SHUNT_MAX_OHMS} ohm or less joins its sense inputs",
                    amp.reference
                ),
            );
            continue;
        };
        let Some(board) = cx.board else { continue };
        let sp = shunt
            .pins
            .iter()
            .find(|p| p.net == np)
            .expect("shunt pin on np");
        let sn = shunt
            .pins
            .iter()
            .find(|p| p.net == nn)
            .expect("shunt pin on nn");
        match (
            pin_dist(board, amp, ip, shunt, sp),
            pin_dist(board, amp, inn, shunt, sn),
        ) {
            (Ok(a), Ok(b)) => {
                let d = a.max(b);
                report.examine("sense_amp_far_from_shunt");
                if d <= SENSE_AMP_MAX_MM {
                    report.pass("sense_amp_far_from_shunt");
                } else {
                    report.finding(
                        "sense_amp_far_from_shunt",
                        "warning",
                        Some(&amp.reference),
                        format!("{} is {:.1} mm from shunt {} (limit {} mm)", amp.reference, d, shunt.reference, SENSE_AMP_MAX_MM),
                        "Place the current-sense amplifier near the shunt so the sense traces stay short and run together".into(),
                        json!({"amplifier": amp.reference, "shunt": shunt.reference, "distance_mm": round3(d),
                               "limit_mm": SENSE_AMP_MAX_MM, "metric": "pad_anchor_euclidean_xy"}),
                    );
                }
            }
            (Err(why), _) | (_, Err(why)) => {
                report.unevaluated(
                    "sense_amp_far_from_shunt",
                    format!("{}: {why}", amp.reference),
                );
                continue;
            }
        }
        // Kelvin: each sense side's copper must not run through the power path.
        for (side, shunt_pin, from_net) in [("positive", sp, &np), ("negative", sn, &nn)] {
            let Ok(shunt_pad) = pad_of(board, shunt, shunt_pin) else {
                continue;
            };
            let Some(board_net) = shunt_pad.net.as_deref() else {
                report.unevaluated(
                    "sense_not_kelvin",
                    format!("{}: shunt pad has no board net", shunt.reference),
                );
                continue;
            };
            // The sense endpoint is the filter resistor on that net, else the amplifier.
            let amp_pin = if side == "positive" { ip } else { inn };
            let endpoint: Option<(&Part, &Pin)> = if &amp_pin.net == from_net {
                Some((amp, amp_pin))
            } else {
                c.on_net(from_net).into_iter().find(|(p, _)| {
                    is_res(p)
                        && p.is_two_terminal()
                        && p.reference != shunt.reference
                        && c.across(p, from_net) == Some(amp_pin.net.as_str())
                })
            };
            let Some((ep, epin)) = endpoint else { continue };
            let Ok(end_pad) = pad_of(board, ep, epin) else {
                report.unevaluated(
                    "sense_not_kelvin",
                    format!("{}: {}", ep.reference, pad_of(board, ep, epin).unwrap_err()),
                );
                continue;
            };
            match kelvin_path(board, board_net, shunt_pad, end_pad) {
                Err(why) => report.unevaluated(
                    "sense_not_kelvin",
                    format!("{} sense of {}: {why}", side, shunt.reference),
                ),
                Ok(shared) if shared.is_empty() => {
                    report.examine("sense_not_kelvin");
                    report.pass("sense_not_kelvin");
                }
                Ok(shared) => {
                    report.examine("sense_not_kelvin");
                    report.finding(
                        "sense_not_kelvin",
                        "warning",
                        Some(&amp.reference),
                        format!(
                            "The {side} sense connection of shunt {} to {} shares copper with the power path (via {})",
                            shunt.reference, ep.reference, shared.join(", ")
                        ),
                        "Take the sense traces straight from the shunt pads (Kelvin), routed as a pair, not from the power trace or a pad that also carries the load current".into(),
                        json!({"shunt": shunt.reference, "side": side, "net": board_net, "shared_with": shared}),
                    );
                }
            }
        }
    }
}

/// The copper between two pads of one board net: pads, tracks and vias as
/// nodes, touching as edges. Returns what the shortest path shares with the
/// rest of the net: pads of other parts on it, and track or via nodes where
/// other copper branches off. Empty means a clean point-to-point route.
fn kelvin_path(
    board: &BoardView,
    net: &str,
    from: &PadGeom,
    to: &PadGeom,
) -> Result<Vec<String>, String> {
    struct Node {
        label: String,
        shape: Shape,
        layers: Vec<String>,
        via: bool,
        pad: bool,
    }
    let mut nodes: Vec<Node> = Vec::new();
    let mut from_idx = None;
    let mut to_idx = None;
    for fp in &board.geometry.footprints {
        for pad in fp.pads.iter().filter(|p| p.net.as_deref() == Some(net)) {
            if std::ptr::eq(pad, from) {
                from_idx = Some(nodes.len());
            }
            if std::ptr::eq(pad, to) {
                to_idx = Some(nodes.len());
            }
            nodes.push(Node {
                label: format!(
                    "{}.{}",
                    fp.reference.clone().unwrap_or_default(),
                    pad.number
                ),
                shape: pad.shape.clone(),
                layers: pad.layers.clone(),
                via: false,
                pad: true,
            });
        }
    }
    let zones = board
        .geometry
        .others
        .iter()
        .any(|e| e.kind == Kind::Zone && e.net.as_deref() == Some(net));
    for e in board
        .geometry
        .others
        .iter()
        .filter(|e| e.net.as_deref() == Some(net))
    {
        match e.kind {
            Kind::Track => nodes.push(Node {
                label: format!("track {}", e.uuid.clone().unwrap_or_default()),
                shape: e.shape.clone(),
                layers: e.layers.clone(),
                via: false,
                pad: false,
            }),
            Kind::Via => nodes.push(Node {
                label: format!("via {}", e.uuid.clone().unwrap_or_default()),
                shape: e.shape.clone(),
                layers: Vec::new(),
                via: true,
                pad: false,
            }),
            _ => {}
        }
    }
    if nodes.len() > KELVIN_GRAPH_LIMIT {
        return Err(format!(
            "net {net} has {} copper items; too many to walk",
            nodes.len()
        ));
    }
    let (Some(a), Some(b)) = (from_idx, to_idx) else {
        return Err("a pad is not among the net's pads (net names differ?)".into());
    };
    let on_same_layer = |x: &Node, y: &Node| -> bool {
        x.via
            || y.via
            || x.layers.iter().any(|l| {
                y.layers
                    .iter()
                    .any(|m| l == m || l == "*.Cu" || m == "*.Cu")
            })
    };
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for i in 0..nodes.len() {
        for j in (i + 1)..nodes.len() {
            if nodes[i].pad && nodes[j].pad {
                continue; // pads join through copper, not by themselves
            }
            if on_same_layer(&nodes[i], &nodes[j]) && nodes[i].shape.intersects(&nodes[j].shape) {
                adj[i].push(j);
                adj[j].push(i);
            }
        }
    }
    // Shortest path by hops from the first pad to the second.
    let mut prev: Vec<Option<usize>> = vec![None; nodes.len()];
    let mut seen = vec![false; nodes.len()];
    seen[a] = true;
    let mut queue = VecDeque::from([a]);
    while let Some(n) = queue.pop_front() {
        if n == b {
            break;
        }
        for &m in &adj[n] {
            if !seen[m] {
                seen[m] = true;
                prev[m] = Some(n);
                queue.push_back(m);
            }
        }
    }
    if !seen[b] {
        return Err(if zones {
            format!("the pads are joined through a zone of {net}, not by tracks")
        } else {
            format!("no copper path between the pads on {net} (unrouted?)")
        });
    }
    let mut path = vec![b];
    while let Some(p) = prev[*path.last().expect("non-empty")] {
        path.push(p);
    }
    path.reverse();
    let mut shared = Vec::new();
    for w in 1..path.len() - 1 {
        let n = path[w];
        if nodes[n].pad {
            shared.push(nodes[n].label.clone());
            continue;
        }
        // Copper that touches this node and is neither on the path nor
        // beside its neighbours is a branch: another route leaves from here.
        // Copper meeting at the pad itself (the power track and the sense
        // track both start there) is not.
        let beside = |m: usize| adj[path[w - 1]].contains(&m) || adj[path[w + 1]].contains(&m);
        if let Some(&m) = adj[n].iter().find(|m| !path.contains(m) && !beside(**m)) {
            shared.push(format!(
                "a branch at {} toward {}",
                nodes[n].label, nodes[m].label
            ));
        }
    }
    Ok(shared)
}

// ─── Section 2: crystals and ADC references ──────────────────────────────────

fn crystals(cx: &Cx<'_>, report: &mut Report) {
    let c = cx.c;
    for xtal in c
        .parts
        .iter()
        .filter(|p| cc::is_crystal(&p.lib_id, &p.reference))
    {
        let signal: Vec<&Pin> = xtal
            .pins
            .iter()
            .filter(|p| c.net(&p.net).is_some_and(|n| !n.is_ground()))
            .collect();
        if signal.len() != 2 {
            if xtal.pins.iter().any(|p| leaves(c, &[p.net.as_str()])) {
                report.unproven(
                    "crystal_load_cap_missing",
                    &xtal.reference,
                    "unresolved pin connectivity prevents identifying both crystal signal pins"
                        .into(),
                );
                continue;
            }
            report.skip(
                "crystal_load_cap_missing",
                format!(
                    "{}: {} signal pin(s); only two-pin crystals are read",
                    xtal.reference,
                    signal.len()
                ),
            );
            continue;
        }
        let mut caps_pf: Vec<f64> = Vec::new();
        let mut cap_json = Vec::new();
        for pin in &signal {
            let caps = caps_to_ground(c, &pin.net);
            if caps.is_empty() {
                if capacitor_search_incomplete(c, &pin.net) {
                    report.unproven(
                        "crystal_load_cap_missing",
                        &xtal.reference,
                        format!(
                            "the load-capacitor inventory of pin {} is incomplete",
                            pin.number
                        ),
                    );
                } else {
                    report.examine("crystal_load_cap_missing");
                    report.finding(
                        "crystal_load_cap_missing",
                        "warning",
                        Some(&xtal.reference),
                        format!("{} pin {} (net {}) has no load capacitor to ground", xtal.reference, pin.number, net_name(c, &pin.net)),
                        "Add the load capacitor the crystal's datasheet calls for (some oscillators have them built in: check the IC)".into(),
                        json!({"crystal": xtal.reference, "pin": pin.number}),
                    );
                }
                continue;
            }
            report.examine("crystal_load_cap_missing");
            report.pass("crystal_load_cap_missing");
            for cap in &caps {
                if let Some(f) = cap.farads {
                    caps_pf.push(f * 1e12);
                }
                cap_json.push(json!({"reference": cap.part.reference, "value": cap.part.value}));
                if let Some(board) = cx.board {
                    match pin_dist(board, cap.part, cap.on, xtal, pin) {
                        Err(why) => report.unevaluated(
                            "crystal_cap_far",
                            format!("{}: {why}", cap.part.reference),
                        ),
                        Ok(d) => {
                            report.examine("crystal_cap_far");
                            if d <= CRYSTAL_CAP_MAX_MM {
                                report.pass("crystal_cap_far");
                            } else {
                                report.finding(
                                    "crystal_cap_far",
                                    "warning",
                                    Some(&cap.part.reference),
                                    format!("Load capacitor {} is {:.1} mm from crystal {} (limit {} mm)", cap.part.reference, d, xtal.reference, CRYSTAL_CAP_MAX_MM),
                                    "Place each load capacitor beside its crystal pin, with a short ground return".into(),
                                    json!({"capacitor": cap.part.reference, "crystal": xtal.reference, "distance_mm": round3(d), "limit_mm": CRYSTAL_CAP_MAX_MM}),
                                );
                            }
                        }
                    }
                }
            }
            if let Some(board) = cx.board {
                for (ic, ipin) in c.on_net(&pin.net).into_iter().filter(|(p, _)| is_ic(p)) {
                    match pin_dist(board, xtal, pin, ic, ipin) {
                        Err(why) => report.unevaluated(
                            "crystal_far_from_ic",
                            format!("{}: {why}", xtal.reference),
                        ),
                        Ok(d) => {
                            report.examine("crystal_far_from_ic");
                            if d <= CRYSTAL_IC_MAX_MM {
                                report.pass("crystal_far_from_ic");
                            } else {
                                report.finding(
                                    "crystal_far_from_ic",
                                    "warning",
                                    Some(&xtal.reference),
                                    format!("Crystal {} is {:.1} mm from {} (limit {} mm)", xtal.reference, d, pin_label(ic, ipin), CRYSTAL_IC_MAX_MM),
                                    "Place the crystal close to the IC's oscillator pins and keep the traces short, per the IC's layout guidance".into(),
                                    json!({"crystal": xtal.reference, "ic_pin": pin_label(ic, ipin), "distance_mm": round3(d), "limit_mm": CRYSTAL_IC_MAX_MM}),
                                );
                            }
                        }
                    }
                }
            }
        }
        let incomplete = signal
            .iter()
            .any(|pin| capacitor_search_incomplete(c, &pin.net));
        let cl = (caps_pf.len() == 2 && !incomplete)
            .then(|| caps_pf[0] * caps_pf[1] / (caps_pf[0] + caps_pf[1]) + CRYSTAL_STRAY_PF);
        report.crystals.push(json!({
            "crystal": xtal.reference,
            "value": xtal.value,
            "load_capacitors": cap_json,
            "computed_load_capacitance_pf": cl.map(round3),
            "connectivity_complete": !incomplete,
            "stray_assumed_pf": CRYSTAL_STRAY_PF,
            "target_load_capacitance_pf": "needs_human: read it from the crystal's datasheet",
        }));
    }
}

fn adc_refs(cx: &Cx<'_>, report: &mut Report) {
    let Some(board) = cx.board else { return };
    let c = cx.c;
    for part in c.parts.iter().filter(|p| is_ic(p)) {
        for pin in part
            .pins
            .iter()
            .filter(|p| cc::ADC_REF_PINS.contains(&norm_pin(&p.name).as_str()))
        {
            let Some(net) = c.net(&pin.net) else { continue };
            if net.is_ground() {
                continue;
            }
            let caps = caps_to_ground(c, &pin.net);
            let incomplete = capacitor_search_incomplete(c, &pin.net);
            if caps.is_empty() {
                if incomplete {
                    pin_unproven(
                        report,
                        "adc_ref_cap_far",
                        part,
                        pin,
                        "the reference-capacitor inventory is incomplete",
                    );
                    continue;
                }
                report.skip(
                    "adc_ref_cap_far",
                    format!(
                        "{}: no capacitor on {}; audit_decoupling reports missing ones",
                        pin_label(part, pin),
                        net.display()
                    ),
                );
                continue;
            }
            let mut best: Option<(&CapAcross, f64)> = None;
            let mut failures = Vec::new();
            for cap in &caps {
                match pin_dist(board, part, pin, cap.part, cap.on) {
                    Ok(d) => {
                        if best.is_none_or(|(_, bd)| d < bd) {
                            best = Some((cap, d));
                        }
                    }
                    Err(why) => failures.push(why),
                }
            }
            let Some((cap, d)) = best else {
                report.unevaluated(
                    "adc_ref_cap_far",
                    format!("{}: {}", pin_label(part, pin), failures.join("; ")),
                );
                continue;
            };
            let limit = crate::tools::placement::decoupling_limit_mm(&cap.part.value)
                .unwrap_or(ADC_REF_DEFAULT_MM);
            let passes_every_family = crate::tools::placement::decoupling_limit_mm("100n")
                .is_some_and(|minimum| d <= minimum);
            if incomplete && (d > limit || !passes_every_family) {
                pin_unproven(
                    report,
                    "adc_ref_cap_far",
                    part,
                    pin,
                    "unresolved connectivity may change the nearest reference capacitor and its value family",
                );
                continue;
            }
            report.examine("adc_ref_cap_far");
            if d <= limit {
                report.pass("adc_ref_cap_far");
            } else {
                report.finding(
                    "adc_ref_cap_far",
                    "warning",
                    Some(&part.reference),
                    format!("Reference capacitor {} is {:.1} mm from {} (limit {} mm)", cap.part.reference, d, pin_label(part, pin), limit),
                    "Place the reference capacitor directly at the reference pin, per the ADC's layout guidance".into(),
                    json!({"capacitor": cap.part.reference, "pin": pin_label(part, pin), "distance_mm": round3(d), "limit_mm": limit}),
                );
            }
        }
    }
}

// ─── Section 6: I2C, UART, USB ───────────────────────────────────────────────

fn i2c_line(name: &str) -> Option<&'static str> {
    let n = norm_pin(name);
    if n == "SDA" || n.ends_with("SDA") {
        Some("SDA")
    } else if n == "SCL" || n.ends_with("SCL") {
        Some("SCL")
    } else {
        None
    }
}

fn i2c(cx: &Cx<'_>, report: &mut Report) {
    let c = cx.c;
    let mut lines: BTreeMap<String, &'static str> = BTreeMap::new();
    for part in c.parts.iter().filter(|p| is_ic(p) || is_conn(p)) {
        for pin in &part.pins {
            if let Some(line) = i2c_line(&pin.name) {
                lines.insert(pin.net.clone(), line);
            }
        }
    }
    for (net_key, line) in lines {
        let on = c.on_net(&net_key);
        let devices: Vec<&Part> = on.iter().map(|(p, _)| *p).filter(|p| is_ic(p)).collect();
        let connectors: Vec<String> = on
            .iter()
            .filter(|(p, _)| is_conn(p))
            .map(|(p, pin)| pin_label(p, pin))
            .collect();
        if devices.is_empty() {
            if leaves(c, &[net_key.as_str()]) {
                net_unproven(
                    c,
                    report,
                    "i2c_pullup_value",
                    &net_key,
                    "unresolved connectivity may contain I2C devices and pull-ups",
                );
            }
            continue;
        }
        let pullups: Vec<(&Part, f64, String)> = on
            .iter()
            .filter(|(p, _)| is_res(p) && p.is_two_terminal())
            .filter_map(|(p, _)| {
                let other = c.across(p, &net_key)?;
                c.net(other).filter(|n| n.is_rail())?;
                Some((*p, cc::parse_si_value(&p.value)?, other.to_string()))
            })
            .collect();
        let r_eff = (!pullups.is_empty())
            .then(|| 1.0 / pullups.iter().map(|(_, o, _)| 1.0 / o).sum::<f64>());
        let incomplete = leaves(c, &[net_key.as_str()])
            || on.iter().any(|(p, _)| {
                is_res(p)
                    && c.across(p, &net_key).is_some_and(|other| {
                        c.net(other).is_none_or(|n| !n.is_rail() && n.leaves_sheet)
                    })
            });
        report.interfaces.push(json!({
            "interface": "i2c",
            "line": line,
            "net": net_name(c, &net_key),
            "net_key": net_key,
            "devices": devices.iter().map(|p| &p.reference).collect::<Vec<_>>(),
            "pull_ups": pullups.iter().map(|(p, o, rail)| json!({"reference": p.reference, "ohms": o, "rail": net_name(c, rail)})).collect::<Vec<_>>(),
            "effective_pull_up_ohms": if incomplete { None } else { r_eff.map(round3) },
            "observed_effective_pull_up_ohms": r_eff.map(round3),
            "connectivity_complete": !incomplete,
            "off_board_connector_pins": connectors,
            "module_pull_ups": if connectors.is_empty() { Value::Null } else {
                json!("not counted: an off-board module may add its own pull-ups in parallel; the total is a human check") },
            "rise_time": "needs_human: depends on the real bus capacitance and cable",
            "missing_pull_up": "reported by audit_connections, not repeated here",
        }));
        if pullups.is_empty() {
            if incomplete {
                net_unproven(
                    c,
                    report,
                    "i2c_pullup_value",
                    &net_key,
                    "the pull-up inventory is incomplete",
                );
            }
            continue;
        }
        // The rail: every pulled-up rail against every device's supply.
        for (pull, _, rail) in &pullups {
            for dev in &devices {
                let supplies = c.supply_nets(dev);
                if supplies.is_empty() {
                    report.needs_human(
                        "i2c_pullup_rail",
                        &dev.reference,
                        format!(
                            "no supply pin found to compare {}'s rail with",
                            pull.reference
                        ),
                    );
                    continue;
                }
                if supplies.contains(rail) {
                    report.examine("i2c_pullup_rail");
                    report.pass("i2c_pullup_rail");
                    continue;
                }
                let pv = net_voltage(cx, rail);
                let dv = supplies
                    .iter()
                    .filter_map(|s| net_voltage(cx, s))
                    .fold(None, |m: Option<f64>, (v, _)| {
                        Some(m.map_or(v, |x| x.min(v)))
                    });
                match (pv, dv) {
                    (Some((pv, psrc)), Some(dv)) => {
                        report.examine("i2c_pullup_rail");
                        if pv > dv + 0.3 {
                            report.finding(
                                "i2c_pullup_rail",
                                "error",
                                Some(&pull.reference),
                                format!(
                                    "{} pulls {} up to {} ({pv} V) but {} is supplied at {dv} V: the pin sees more than its supply",
                                    pull.reference, net_name(c, &net_key), net_name(c, rail), dev.reference
                                ),
                                "Pull the bus up to the rail the devices run from, or level-shift it".into(),
                                json!({"pull_up": pull.reference, "device": dev.reference, "pull_up_rail_v": pv, "device_supply_v": dv, "voltage_source": psrc}),
                            );
                        } else if pv < 0.7 * dv {
                            report.finding(
                                "i2c_pullup_rail",
                                "warning",
                                Some(&pull.reference),
                                format!(
                                    "{} pulls {} up to {pv} V; {} at {dv} V needs a high level above 0.7 x its supply",
                                    pull.reference, net_name(c, &net_key), dev.reference
                                ),
                                "Pull up to the devices' rail or level-shift the bus".into(),
                                json!({"pull_up": pull.reference, "device": dev.reference, "pull_up_rail_v": pv, "device_supply_v": dv}),
                            );
                        } else {
                            report.pass("i2c_pullup_rail");
                        }
                    }
                    _ => report.needs_human(
                        "i2c_pullup_rail",
                        &pull.reference,
                        format!("on {} vs {}: the voltage of one rail is neither recorded nor in its name", net_name(c, rail), dev.reference),
                    ),
                }
            }
        }
        // The value.
        let r = r_eff.expect("pull-ups present");
        let v = pullups
            .iter()
            .filter_map(|(_, _, rail)| net_voltage(cx, rail))
            .map(|(v, _)| v)
            .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |x| x.max(v))));
        let sink_ma = v.map(|v| (v - 0.4) / r * 1000.0);
        // Additional positive parallel resistors can only increase this
        // rule's sink-current estimate. Preserve an already exceeded limit,
        // but do not pass or declare weak pull-ups from a partial inventory.
        let known_overload = sink_ma.is_some_and(|i| i > 3.0)
            && pullups
                .iter()
                .all(|(_, ohms, _)| ohms.is_finite() && *ohms > 0.0);
        if incomplete && !known_overload {
            net_unproven(
                c,
                report,
                "i2c_pullup_value",
                &net_key,
                "unresolved connectivity can add parallel pull-ups and change the total resistance",
            );
            continue;
        }
        report.examine("i2c_pullup_value");
        if sink_ma.is_some_and(|i| i > 3.0) {
            report.finding(
                "i2c_pullup_value",
                "warning",
                Some(&devices[0].reference),
                format!("The observed pull-ups on {} give {:.0} ohm, which already asks a device to sink {:.1} mA (standard-mode limit 3 mA){}", net_name(c, &net_key), r, sink_ma.unwrap_or(0.0), if incomplete { "; further pull-ups may be unobserved" } else { "" }),
                "Raise the pull-up resistance, or check that every device on the bus can sink more".into(),
                json!({"net": net_name(c, &net_key), "net_key": net_key, "effective_ohms": round3(r), "sink_ma": sink_ma.map(round3), "connectivity_complete": !incomplete}),
            );
        } else if r > 10_000.0 {
            report.finding(
                "i2c_pullup_value",
                "warning",
                Some(&devices[0].reference),
                format!("The pull-ups on {} total {:.0} ohm; the rising edge will be slow", net_name(c, &net_key), r),
                "Lower the pull-up resistance (typically 2.2k to 4.7k) after checking the bus capacitance".into(),
                json!({"net": net_name(c, &net_key), "net_key": net_key, "effective_ohms": round3(r)}),
            );
        } else {
            report.pass("i2c_pullup_value");
        }
    }
}

fn uart(cx: &Cx<'_>, report: &mut Report) {
    let c = cx.c;
    for net in c.nets.values() {
        let mut tx: Vec<String> = Vec::new();
        let mut rx: Vec<String> = Vec::new();
        for (part, pin) in c.on_net(&net.key) {
            if !is_ic(part) {
                continue;
            }
            let Some(by_name) = cc::uart_direction(&norm_pin(&pin.name)) else {
                continue;
            };
            let dir = match pin.electrical_type.as_str() {
                "output" | "tri_state" => 0,
                "input" => 1,
                _ => by_name,
            };
            if dir == 0 { &mut tx } else { &mut rx }.push(pin_label(part, pin));
        }
        let parts_of = |v: &[String]| -> BTreeSet<String> {
            v.iter()
                .map(|s| s.split('.').next().unwrap_or("").to_string())
                .collect()
        };
        if tx.len() + rx.len() < 2 || parts_of(&tx).union(&parts_of(&rx)).count() < 2 {
            if net.leaves_sheet && (!tx.is_empty() || !rx.is_empty()) {
                net_unproven(
                    c,
                    report,
                    "uart_tx_rx_crossed",
                    &net.key,
                    "the UART net is incomplete, so its other endpoints are unknown",
                );
            }
            continue;
        }
        if parts_of(&tx).len() >= 2 {
            report.examine("uart_tx_rx_crossed");
            report.finding(
                "uart_tx_rx_crossed",
                "error",
                None,
                format!(
                    "Two transmit pins are joined on {}: {}",
                    net.display(),
                    tx.join(", ")
                ),
                "Cross the UART: one device's TX goes to the other's RX".into(),
                json!({"net": net.display(), "net_key": net.key, "tx_pins": tx, "rx_pins": rx}),
            );
        } else if net.leaves_sheet {
            net_unproven(
                c,
                report,
                "uart_tx_rx_crossed",
                &net.key,
                "unresolved connectivity prevents proving the complete transmitter/receiver inventory",
            );
        } else if tx.is_empty() && parts_of(&rx).len() >= 2 {
            report.examine("uart_tx_rx_crossed");
            report.finding(
                "uart_tx_rx_crossed",
                "error",
                None,
                format!(
                    "Receive pins are joined with no transmitter on {}: {}",
                    net.display(),
                    rx.join(", ")
                ),
                "Cross the UART: one device's TX goes to the other's RX".into(),
                json!({"net": net.display(), "net_key": net.key, "rx_pins": rx}),
            );
        } else {
            report.examine("uart_tx_rx_crossed");
            report.pass("uart_tx_rx_crossed");
        }
    }
}

fn usb(cx: &Cx<'_>, report: &mut Report) {
    let c = cx.c;
    let mut data: BTreeMap<String, &'static str> = BTreeMap::new();
    for part in &c.parts {
        for pin in &part.pins {
            let n = norm_pin(&pin.name);
            if cc::USB_DP.contains(&n.as_str())
                && (is_conn(part) || n.starts_with("USB") || n.contains('+') || n == "DP")
            {
                data.insert(pin.net.clone(), "D+");
            } else if cc::USB_DM.contains(&n.as_str())
                && (is_conn(part) || n.starts_with("USB") || n.contains('-') || n == "DM")
            {
                data.insert(pin.net.clone(), "D-");
            }
        }
    }
    for (key, line) in &data {
        let at_connector = c.on_net(key).iter().any(|(p, _)| is_conn(p));
        if !at_connector {
            if leaves(c, &[key.as_str()]) {
                net_unproven(
                    c,
                    report,
                    "usb_esd_missing",
                    key,
                    "the USB net is incomplete, so a connector may be unobserved",
                );
                continue;
            }
            report.skip(
                "usb_esd_missing",
                format!("{}: {line} net is not at a connector", net_name(c, key)),
            );
            continue;
        }
        let prot = protection_on(c, key);
        if prot.is_empty() {
            if leaves(c, &[key.as_str()]) {
                net_unproven(
                    c,
                    report,
                    "usb_esd_missing",
                    key,
                    "the USB net has unresolved connectivity",
                );
            } else {
                report.examine("usb_esd_missing");
                report.finding(
                    "usb_esd_missing",
                    "warning",
                    None,
                    format!("USB {line} net {} has no ESD protection at the connector", net_name(c, key)),
                    "Add a low-capacitance ESD array (for example a USBLC6 or TPD2E-class part) at the connector".into(),
                    json!({"net": net_name(c, key), "net_key": key, "line": line}),
                );
            }
            continue;
        }
        report.examine("usb_esd_missing");
        report.pass("usb_esd_missing");
        for (p, _, kind) in &prot {
            match kind.low_capacitance {
                Some(true) => {
                    report.examine("usb_esd_capacitance");
                    report.pass("usb_esd_capacitance");
                }
                Some(false) => {
                    report.examine("usb_esd_capacitance");
                    report.finding(
                        "usb_esd_capacitance",
                        "warning",
                        Some(&p.reference),
                        format!("{} ({}) on USB {line} is a {} power-line TVS: its junction capacitance is far above what USB data allows", p.reference, p.value, kind.family),
                        "Use a low-capacitance ESD array rated for USB 2.0 data lines (a few pF per line)".into(),
                        json!({"protection": p.reference, "value": p.value, "family": kind.family, "line": line}),
                    );
                }
                None => report.needs_human(
                    "usb_esd_capacitance",
                    &p.reference,
                    format!(
                        "{}: capacitance of this part is not in the table; check its datasheet",
                        p.value
                    ),
                ),
            }
        }
    }
}

/// The recorded USB pair impedance. A property of the design-intent record,
/// not of a sheet, so it is judged once per design.
pub(super) fn usb_pair_impedance(intent: &DesignIntent, report: &mut Report) {
    for iface in intent
        .interfaces
        .iter()
        .filter(|i| i.kind.to_lowercase().contains("usb"))
    {
        report.examine("usb_pair_impedance");
        match iface.differential_impedance_ohm {
            None => report.finding(
                "usb_pair_impedance",
                "warning",
                None,
                format!("Interface '{}' is USB but records no differential impedance", iface.name),
                "Record 90 ohm differential impedance and the D+/D- pair in the design-intent record so routing and review can use it".into(),
                json!({"interface": iface.name, "differential_pairs": iface.differential_pairs.len()}),
            ),
            Some(z) if (z - 90.0).abs() > 5.0 => report.finding(
                "usb_pair_impedance",
                "error",
                None,
                format!("Interface '{}' records {z} ohm differential impedance; USB 2.0 needs 90 ohm", iface.name),
                "Correct the record to 90 ohm differential and route to it".into(),
                json!({"interface": iface.name, "recorded_ohms": z}),
            ),
            Some(_) if iface.differential_pairs.is_empty() => report.finding(
                "usb_pair_impedance",
                "warning",
                None,
                format!("Interface '{}' records 90 ohm but declares no D+/D- pair", iface.name),
                "Declare the pair (differential_pairs) so the impedance applies to those nets".into(),
                json!({"interface": iface.name}),
            ),
            Some(_) => report.pass("usb_pair_impedance"),
        }
    }
}
