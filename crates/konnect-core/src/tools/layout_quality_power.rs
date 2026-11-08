//! Checklist §4 (decoupling loops, copper sizing, bottlenecks) and §8
//! (switching regulators' critical loop and switch-node copper), plus the §3
//! ground-return bottleneck, which shares the neck-down search.

use std::collections::HashMap;

use konnect_sexp::parser::SexpNode;
use konnect_sexp::planar::{point_segment_distance, segment_distance, Pt};
use serde_json::{json, Value};

use super::grid::{self, Cell, Cover, Grid};
use super::{
    dist, excess_score, is_cap, is_diode, is_ic, is_inductor, is_mosfet, margin_score, r2,
    CheckOut, Finding, Model, PadIx,
};
use crate::tools::trace_current;

pub(super) fn run(m: &Model, board: &std::path::Path, _tree: &SexpNode) -> Vec<CheckOut> {
    let (decoupling, loops) = decoupling_and_hot_loops(m);
    let mut out = vec![decoupling, current_sizing(m, board)];
    out.extend(bottlenecks(m));
    out.extend(loops);
    out
}

// ─── Loops ───────────────────────────────────────────────────────────────────

/// A loop to measure: capacitor between `cap_net` and ground, then `entry`
/// (a pad on `cap_net`), the middle legs, and `exit` (a ground pad) back to the
/// capacitor's ground pad.
struct LoopSpec {
    cap_net: String,
    entry: PadIx,
    middle: Vec<(String, PadIx, PadIx)>,
    exit: PadIx,
}

struct Leg {
    net: String,
    from: String,
    to: String,
    length_mm: f64,
    straight_mm: f64,
    vias: usize,
}

struct LoopResult {
    cap: String,
    cap_uuid: Option<String>,
    length_mm: f64,
    straight_mm: f64,
    area_mm2: f64,
    vias: usize,
    legs: Vec<Leg>,
    cap_pads: [Option<String>; 2],
}

fn pad_label(m: &Model, pi: PadIx) -> String {
    let (fp, pad) = m.pad(pi);
    format!("{}.{}", m.fp_ref(fp), pad.number)
}

fn shoelace(pts: &[Pt]) -> f64 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut a = 0.0;
    for i in 0..n {
        let (p, q) = (pts[i], pts[(i + 1) % n]);
        a += p.0 * q.1 - q.0 * p.1;
    }
    (a / 2.0).abs()
}

/// Thin a path to its corners so the loop polygon stays small.
fn simplify(pts: &[Pt]) -> Vec<Pt> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = vec![pts[0]];
    for w in pts.windows(3) {
        let (a, b, c) = (w[0], w[1], w[2]);
        let cross = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
        if cross.abs() > 1e-9 {
            out.push(b);
        }
    }
    out.push(*pts.last().unwrap_or(&pts[0]));
    out
}

struct LegPath {
    length: f64,
    vias: usize,
    pts: Vec<Pt>,
}

fn leg_path(m: &Model, net: &str, a: PadIx, b: PadIx) -> Option<LegPath> {
    let (_, pa) = m.pad(a);
    let (_, pb) = m.pad(b);
    let (ba, bb) = (pa.shape.bbox()?, pb.shape.bbox()?);
    let d = dist(pa.at, pb.at);
    let margin = 6.0 + 0.5 * d;
    let window = (
        ba.0.min(bb.0) - margin,
        ba.1.min(bb.1) - margin,
        ba.2.max(bb.2) + margin,
        ba.3.max(bb.3) + margin,
    );
    let g = m.net_grid(net, window, Cover::Touch);
    let from = g.cells_under(&pa.shape, &m.pad_layers(pa));
    let to = g.cells_under(&pb.shape, &m.pad_layers(pb));
    let p = grid::shortest_path(&g, &from, &to, m.params.get("via_cost_mm"))?;
    Some(LegPath {
        length: p.length_mm,
        vias: p.vias,
        pts: p.points,
    })
}

fn ground_nets_of_cap(m: &Model, fp: usize, cap_net: &str) -> Option<(PadIx, PadIx)> {
    let f = &m.geo.footprints[fp];
    let plus = f
        .pads
        .iter()
        .position(|p| p.net.as_deref() == Some(cap_net))?;
    let minus = f
        .pads
        .iter()
        .position(|p| p.net.as_deref().is_some_and(|n| m.is_ground(n)))?;
    Some(((fp, plus), (fp, minus)))
}

fn measure_loop(m: &Model, spec: &LoopSpec, search_mm: f64) -> Result<LoopResult, String> {
    let (_, entry) = m.pad(spec.entry);
    let mut caps: Vec<(f64, PadIx, PadIx)> = Vec::new();
    for (fi, f) in m.geo.footprints.iter().enumerate() {
        if !is_cap(m.fp_ref(f)) {
            continue;
        }
        if let Some((plus, minus)) = ground_nets_of_cap(m, fi, &spec.cap_net) {
            let d = dist(m.pad(plus).1.at, entry.at);
            if d <= search_mm {
                caps.push((d, plus, minus));
            }
        }
    }
    if caps.is_empty() {
        return Err(format!(
            "no capacitor between '{}' and ground within {search_mm} mm of {}",
            spec.cap_net,
            pad_label(m, spec.entry)
        ));
    }
    caps.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut best: Option<LoopResult> = None;
    let ground_of = |pi: PadIx| m.pad(pi).1.net.clone().unwrap_or_default();
    for (_, plus, minus) in caps.into_iter().take(3) {
        let mut legs = Vec::new();
        let mut poly: Vec<Pt> = Vec::new();
        let (mut total, mut straight, mut vias) = (0.0, 0.0, 0);
        let mut ok = true;
        let mut chain: Vec<(String, PadIx, PadIx)> = Vec::new();
        chain.push((spec.cap_net.clone(), plus, spec.entry));
        chain.extend(spec.middle.iter().cloned());
        chain.push((ground_of(spec.exit), spec.exit, minus));
        let cap_pad_ground_net = ground_of(minus);
        for (k, (net, a, b)) in chain.iter().enumerate() {
            // The last leg runs on the capacitor's own ground net.
            let net = if k == chain.len() - 1 && net != &cap_pad_ground_net {
                // exit pad and capacitor are on different ground nets: no copper path.
                ok = false;
                net.clone()
            } else {
                net.clone()
            };
            let Some(p) = leg_path(m, &net, *a, *b) else {
                ok = false;
                break;
            };
            let s = dist(m.pad(*a).1.at, m.pad(*b).1.at);
            legs.push(Leg {
                net: net.clone(),
                from: pad_label(m, *a),
                to: pad_label(m, *b),
                length_mm: p.length,
                straight_mm: s,
                vias: p.vias,
            });
            total += p.length;
            straight += s;
            vias += p.vias;
            poly.push(m.pad(*a).1.at);
            poly.extend(simplify(&p.pts));
            poly.push(m.pad(*b).1.at);
        }
        if !ok {
            continue;
        }
        let cap_fp = &m.geo.footprints[plus.0];
        let res = LoopResult {
            cap: m.fp_ref(cap_fp).to_string(),
            cap_uuid: cap_fp.uuid.clone(),
            length_mm: total,
            straight_mm: straight,
            area_mm2: shoelace(&poly),
            vias,
            legs,
            cap_pads: [m.pad(plus).1.uuid.clone(), m.pad(minus).1.uuid.clone()],
        };
        if best.as_ref().is_none_or(|b| res.length_mm < b.length_mm) {
            best = Some(res);
        }
    }
    best.ok_or_else(|| {
        format!(
            "capacitors near {} have no connected copper path through '{}' and ground",
            pad_label(m, spec.entry),
            spec.cap_net
        )
    })
}

fn loop_json(r: &LoopResult) -> Value {
    json!({
        "capacitor": r.cap,
        "loop_mm": r2(r.length_mm),
        "straight_line_mm": r2(r.straight_mm),
        "area_mm2": r2(r.area_mm2),
        "layer_changes": r.vias,
        "legs": r.legs.iter().map(|l| json!({
            "net": l.net, "from": l.from, "to": l.to,
            "copper_mm": r2(l.length_mm), "straight_mm": r2(l.straight_mm), "layer_changes": l.vias,
        })).collect::<Vec<_>>(),
    })
}

fn decoupling_and_hot_loops(m: &Model) -> (CheckOut, Vec<CheckOut>) {
    let search = m.params.get("decap_search_mm");
    let switchers = find_switchers(m);
    let switch_nets: Vec<&str> = switchers.iter().map(|s| s.net.as_str()).collect();

    // 4.1: every IC supply pin.
    let id = "4.1";
    let name = "decoupling_loop";
    let (max_len, max_area) = (
        m.params.get("decap_max_loop_mm"),
        m.params.get("decap_max_loop_area_mm2"),
    );
    let mut dec = CheckOut::checked(id, name);
    let mut rows = Vec::new();
    let mut scores = Vec::new();
    let mut lens = Vec::new();
    let (mut areas, mut no_cap, mut cannot) = (Vec::new(), 0usize, Vec::new());
    let mut pins = 0usize;
    'ics: for (fi, f) in m.geo.footprints.iter().enumerate() {
        if !is_ic(m.fp_ref(f)) {
            continue;
        }
        for (pi, pad) in f.pads.iter().enumerate() {
            let Some(net) = pad.net.as_deref() else {
                continue;
            };
            if !m.is_power(net) || switch_nets.contains(&net) {
                continue;
            }
            if pins >= 300 {
                cannot.push("stopped after 300 supply pins".to_string());
                break 'ics;
            }
            // The ground pad of the same IC closest to this pin.
            let ground = f
                .pads
                .iter()
                .enumerate()
                .filter(|(_, g)| g.net.as_deref().is_some_and(|n| m.is_ground(n)))
                .min_by(|a, b| {
                    dist(a.1.at, pad.at)
                        .partial_cmp(&dist(b.1.at, pad.at))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            let Some((gi, _)) = ground else {
                cannot.push(format!("{} has no ground pad", m.fp_ref(f)));
                continue;
            };
            pins += 1;
            let spec = LoopSpec {
                cap_net: net.to_string(),
                entry: (fi, pi),
                middle: Vec::new(),
                exit: (fi, gi),
            };
            let label = pad_label(m, (fi, pi));
            match measure_loop(m, &spec, search) {
                Ok(r) => {
                    let ratio = (r.length_mm / max_len).max(r.area_mm2 / max_area);
                    scores.push(margin_score(ratio, 1.0));
                    lens.push(r.length_mm);
                    areas.push(r.area_mm2);
                    if ratio > 1.0 {
                        dec.findings.push(Finding {
                            severity: if ratio > 2.0 { "error" } else { "warning" },
                            item: id,
                            subject: format!("{label} ({net})"),
                            measured: json!({ "loop_mm": r2(r.length_mm), "area_mm2": r2(r.area_mm2), "capacitor": r.cap, "straight_line_mm": r2(r.straight_mm) }),
                            threshold: json!({ "decap_max_loop_mm": max_len, "decap_max_loop_area_mm2": max_area }),
                            issue: format!(
                                "Decoupling loop of {label}: capacitor {} → pin → ground → capacitor is {:.1} mm of copper ({:.1} mm² enclosed); limits {max_len} mm and {max_area} mm²",
                                r.cap, r.length_mm, r.area_mm2
                            ),
                            fix: "Move the capacitor beside the pin and shorten both its supply and its ground connection (a ground via at the capacitor pad)".into(),
                            uuids: [f.uuid.clone(), pad.uuid.clone(), r.cap_uuid.clone()].into_iter().flatten().collect(),
                            at: Some(pad.at),
                        });
                    }
                    rows.push(json!({ "pin": label, "net": net, "loop": loop_json(&r) }));
                }
                Err(why) => {
                    if why.starts_with("no capacitor") {
                        no_cap += 1;
                        scores.push(0.0);
                        dec.findings.push(Finding {
                            severity: "warning",
                            item: id,
                            subject: format!("{label} ({net})"),
                            measured: json!({ "capacitor_within_mm": Value::Null }),
                            threshold: json!({ "decap_search_mm": search }),
                            issue: format!("Supply pin {label}: {why}"),
                            fix: "Place a decoupling capacitor beside this pin, or list the net as ground/power correctly if it is not a supply".into(),
                            uuids: [f.uuid.clone(), pad.uuid.clone()].into_iter().flatten().collect(),
                            at: Some(pad.at),
                        });
                    } else {
                        cannot.push(format!("{label}: {why}"));
                    }
                    rows.push(json!({ "pin": label, "net": net, "loop": Value::Null, "why": why }));
                }
            }
        }
    }
    let mut hot = Vec::new();
    if pins == 0 {
        dec = CheckOut::not_run(id, name, "no IC (reference U…/IC…) has a pad on a power net; record rails in the design intent or pass power_nets");
    } else {
        dec.score = (!scores.is_empty()).then(|| scores.iter().sum::<f64>() / scores.len() as f64);
        if !cannot.is_empty() {
            dec.status = "partial";
            dec.reason = Some(format!(
                "{} pin(s) not measured: {}",
                cannot.len(),
                cannot
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        lens.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = lens.get(lens.len() / 2).copied();
        dec.needs_human.push(("4.4", "Whether the capacitor value and type suit the part: needs the datasheet's recommended values".into()));
        dec.metrics = json!({
            "supply_pins": pins,
            "measured": lens.len(),
            "pins_without_capacitor": no_cap,
            "compare": {
                "worst_loop_mm": lens.last().map(|v| r2(*v)),
                "median_loop_mm": median.map(r2),
                "mean_loop_area_mm2": (!areas.is_empty()).then(|| r2(areas.iter().sum::<f64>() / areas.len() as f64)),
                "pins_without_capacitor": no_cap,
            },
            "pins": rows.into_iter().take(25).collect::<Vec<_>>(),
            "method": "shortest copper path (tracks, vias, pours) from the capacitor pad to the pin, plus from the IC's nearest ground pad to the capacitor's ground pad; the loop area is the plan-view polygon of both paths. Up to the 3 nearest capacitors are tried and the shortest loop kept",
        });
    }
    hot.extend(switching_loops(m, &switchers));
    (dec, hot)
}

// ─── Switching regulators (§8) ───────────────────────────────────────────────

struct Switcher {
    net: String,
    topology: Option<&'static str>,
    how: String,
    inductor: PadIx,
    /// Parts with a pad on the node (ICs, MOSFETs, diodes).
    parts: Vec<usize>,
    far_net: String,
}

fn find_switchers(m: &Model) -> Vec<Switcher> {
    let mut out = Vec::new();
    for (net, pads) in &m.pads {
        if m.is_ground(net) {
            continue;
        }
        let Some(&ind) = pads.iter().find(|&&pi| is_inductor(m.fp_ref(m.pad(pi).0))) else {
            continue;
        };
        let parts: Vec<usize> = {
            let mut v: Vec<usize> = pads
                .iter()
                .map(|p| p.0)
                .filter(|&fi| {
                    let r = m.fp_ref(&m.geo.footprints[fi]);
                    is_ic(r) || is_mosfet(r) || is_diode(r)
                })
                .collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        if parts.is_empty() {
            continue;
        }
        // A rail with an inductor in series (a power inductor on a power net) is a filter, not a node.
        if m.is_power(net) && m.roles.power.get(net).is_some_and(|s| s != "name") {
            continue;
        }
        let ind_fp = &m.geo.footprints[ind.0];
        let Some(far) = ind_fp
            .pads
            .iter()
            .filter_map(|p| p.net.clone())
            .find(|n| n != net)
        else {
            continue;
        };
        let (topology, how) = decide_topology(m, net, &far, &parts);
        out.push(Switcher {
            net: net.clone(),
            topology,
            how,
            inductor: ind,
            parts,
            far_net: far,
        });
    }
    out.sort_by(|a, b| a.net.cmp(&b.net));
    out
}

fn decide_topology(
    m: &Model,
    node: &str,
    far: &str,
    parts: &[usize],
) -> (Option<&'static str>, String) {
    if let Some((_, t)) = m.params.regulators.iter().find(|(n, _)| n == node) {
        let t = if t == "buck" { "buck" } else { "boost" };
        return (Some(t), "argument switching_regulators".into());
    }
    let ics: Vec<&str> = parts
        .iter()
        .map(|&fi| m.fp_ref(&m.geo.footprints[fi]))
        .filter(|r| is_ic(r))
        .collect();
    let source_of = |net: &str| m.roles.rails.get(net).and_then(|r| r.source_ref.as_deref());
    if let Some(src) = source_of(far) {
        if ics.contains(&src) {
            return (
                Some("buck"),
                format!(
                    "design_intent: rail '{far}' is supplied by {src}, the IC on the switch node"
                ),
            );
        }
    }
    for &fi in parts {
        let f = &m.geo.footprints[fi];
        if !is_ic(m.fp_ref(f)) {
            continue;
        }
        for p in &f.pads {
            if let Some(n) = p.net.as_deref().filter(|n| *n != far && m.is_power(n)) {
                if source_of(n) == Some(m.fp_ref(f)) {
                    return (Some("boost"), format!("design_intent: rail '{n}' is supplied by {}, the IC on the switch node", m.fp_ref(f)));
                }
            }
        }
    }
    // Voltages of the rails on each side, when the record has them.
    (
        None,
        "design-intent record does not say which rail the regulator supplies".into(),
    )
}

fn nearest_pad(
    m: &Model,
    fp: usize,
    to: Pt,
    pred: impl Fn(&konnect_sexp::board_geometry::PadGeom) -> bool,
) -> Option<PadIx> {
    m.geo.footprints[fp]
        .pads
        .iter()
        .enumerate()
        .filter(|(_, p)| pred(p))
        .min_by(|a, b| {
            dist(a.1.at, to)
                .partial_cmp(&dist(b.1.at, to))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| (fp, i))
}

fn spec_for(m: &Model, s: &Switcher) -> Result<LoopSpec, String> {
    let topo = s.topology.ok_or("topology unknown")?;
    let node_pad = |fp: usize| {
        nearest_pad(m, fp, m.pad(s.inductor).1.at, |p| {
            p.net.as_deref() == Some(s.net.as_str())
        })
    };
    let ground = |fp: usize, near: Pt| {
        // Prefer a pin named PGND, else the nearest ground pad.
        nearest_pad(m, fp, near, |p| {
            p.net.as_deref().is_some_and(|n| m.is_ground(n))
                && p.pin_function
                    .as_deref()
                    .is_some_and(|f| f.to_uppercase().starts_with("PGND"))
        })
        .or_else(|| {
            nearest_pad(m, fp, near, |p| {
                p.net.as_deref().is_some_and(|n| m.is_ground(n))
            })
        })
    };
    let ics: Vec<usize> = s
        .parts
        .iter()
        .copied()
        .filter(|&fi| is_ic(m.fp_ref(&m.geo.footprints[fi])))
        .collect();
    let mosfets: Vec<usize> = s
        .parts
        .iter()
        .copied()
        .filter(|&fi| is_mosfet(m.fp_ref(&m.geo.footprints[fi])))
        .collect();
    let diodes: Vec<usize> = s
        .parts
        .iter()
        .copied()
        .filter(|&fi| is_diode(m.fp_ref(&m.geo.footprints[fi])))
        .collect();
    let power_pad_other_than = |fp: usize, not: &str, near: Pt| {
        // The input (buck) or output (boost) pin: a power pad that is not the inductor's far side.
        let named = nearest_pad(m, fp, near, |p| {
            p.net.as_deref().is_some_and(|n| n != not && m.is_power(n))
                && p.pin_function.as_deref().is_some_and(|f| {
                    let f = f.to_uppercase();
                    f.starts_with("VIN")
                        || f.starts_with("PVIN")
                        || f.starts_with("VOUT")
                        || f == "IN"
                        || f == "OUT"
                })
        });
        named.or_else(|| {
            nearest_pad(m, fp, near, |p| {
                p.net.as_deref().is_some_and(|n| n != not && m.is_power(n))
            })
        })
    };
    let node_at = m.pad(s.inductor).1.at;
    if let Some(&ic) = ics.first() {
        let sw = node_pad(ic).ok_or("no pad of the IC on the switch node")?;
        let sw_at = m.pad(sw).1.at;
        let pwr = power_pad_other_than(ic, &s.far_net, sw_at)
            .ok_or("the IC has no power pad apart from the inductor's far side")?;
        let gnd = ground(ic, sw_at).ok_or("the IC has no ground pad")?;
        let cap_net = m.pad(pwr).1.net.clone().unwrap_or_default();
        if topo == "boost" {
            if let Some(&d) = diodes.first() {
                let anode = node_pad(d).ok_or("no diode pad on the switch node")?;
                let cathode = nearest_pad(m, d, node_at, |p| {
                    p.net
                        .as_deref()
                        .is_some_and(|n| n != s.net && m.is_power(n))
                })
                .ok_or("the diode's other pad is not on a power net")?;
                let out_net = m.pad(cathode).1.net.clone().unwrap_or_default();
                return Ok(LoopSpec {
                    cap_net: out_net,
                    entry: cathode,
                    middle: vec![(s.net.clone(), anode, sw)],
                    exit: gnd,
                });
            }
        }
        return Ok(LoopSpec {
            cap_net,
            entry: pwr,
            middle: Vec::new(),
            exit: gnd,
        });
    }
    // Discrete stages.
    if topo == "buck" {
        let hs = mosfets
            .iter()
            .find(|&&q| {
                m.geo.footprints[q].pads.iter().any(|p| {
                    p.net
                        .as_deref()
                        .is_some_and(|n| n != s.net && m.is_power(n))
                })
            })
            .copied()
            .ok_or("no high-side MOSFET (a pad on the node and one on a power net)")?;
        let ls = mosfets
            .iter()
            .find(|&&q| {
                q != hs
                    && m.geo.footprints[q]
                        .pads
                        .iter()
                        .any(|p| p.net.as_deref().is_some_and(|n| m.is_ground(n)))
            })
            .copied()
            .ok_or("no low-side MOSFET (a pad on the node and one on ground)")?;
        let vin = nearest_pad(m, hs, node_at, |p| {
            p.net
                .as_deref()
                .is_some_and(|n| n != s.net && m.is_power(n))
        })
        .ok_or("no input pad")?;
        let hs_sw = node_pad(hs).ok_or("no high-side pad on the node")?;
        let ls_sw = node_pad(ls).ok_or("no low-side pad on the node")?;
        let gnd = ground(ls, node_at).ok_or("no low-side ground pad")?;
        return Ok(LoopSpec {
            cap_net: m.pad(vin).1.net.clone().unwrap_or_default(),
            entry: vin,
            middle: vec![(s.net.clone(), hs_sw, ls_sw)],
            exit: gnd,
        });
    }
    let (Some(&q), Some(&d)) = (mosfets.first(), diodes.first()) else {
        return Err("boost stage without a MOSFET and a diode on the node".into());
    };
    let anode = node_pad(d).ok_or("no diode pad on the node")?;
    let cathode = nearest_pad(m, d, node_at, |p| {
        p.net
            .as_deref()
            .is_some_and(|n| n != s.net && m.is_power(n))
    })
    .ok_or("diode cathode is not on a power net")?;
    let q_sw = node_pad(q).ok_or("no MOSFET pad on the node")?;
    let gnd = ground(q, node_at).ok_or("no MOSFET ground pad")?;
    Ok(LoopSpec {
        cap_net: m.pad(cathode).1.net.clone().unwrap_or_default(),
        entry: cathode,
        middle: vec![(s.net.clone(), anode, q_sw)],
        exit: gnd,
    })
}

fn switching_loops(m: &Model, switchers: &[Switcher]) -> Vec<CheckOut> {
    let (mut hot, mut node) = (
        CheckOut::checked("8.1", "switching_hot_loop"),
        CheckOut::checked("8.2", "switch_node_copper"),
    );
    if switchers.is_empty() {
        let why = "no switch node found: no net joins an inductor (L…) to an IC, MOSFET or diode";
        return vec![
            CheckOut::not_run("8.1", "switching_hot_loop", why),
            CheckOut::not_run("8.2", "switch_node_copper", why),
        ];
    }
    let (max_len, max_area) = (
        m.params.get("hot_loop_max_mm"),
        m.params.get("hot_loop_max_area_mm2"),
    );
    let search = m.params.get("decap_search_mm");
    let mut hot_rows = Vec::new();
    let mut hot_scores = Vec::new();
    let (mut worst_len, mut worst_area) = (0.0f64, 0.0f64);
    let mut notes = Vec::new();
    for s in switchers {
        let Some(topo) = s.topology else {
            notes.push(format!("switch node '{}': {}", s.net, s.how));
            hot.needs_human.push(("8.1", format!("Switch node '{}': topology not determined ({}); name it with switching_regulators or record the supplying rail's source_ref", s.net, s.how)));
            hot_rows.push(json!({ "switch_net": s.net, "topology": Value::Null, "why": s.how }));
            continue;
        };
        match spec_for(m, s).and_then(|spec| measure_loop(m, &spec, search)) {
            Ok(r) => {
                let ratio = (r.length_mm / max_len).max(r.area_mm2 / max_area);
                hot_scores.push(margin_score(ratio, 1.0));
                worst_len = worst_len.max(r.length_mm);
                worst_area = worst_area.max(r.area_mm2);
                let what = if topo == "buck" {
                    "input loop"
                } else {
                    "output loop"
                };
                if ratio > 1.0 {
                    hot.findings.push(Finding {
                        severity: if ratio > 2.0 { "error" } else { "warning" },
                        item: "8.1",
                        subject: format!("{} {} ({what})", s.net, topo),
                        measured: json!({ "loop_mm": r2(r.length_mm), "area_mm2": r2(r.area_mm2), "capacitor": r.cap, "legs": r.legs.iter().map(|l| json!({"net": l.net, "from": l.from, "to": l.to, "copper_mm": r2(l.length_mm)})).collect::<Vec<_>>() }),
                        threshold: json!({ "hot_loop_max_mm": max_len, "hot_loop_max_area_mm2": max_area }),
                        issue: format!("{topo} {what} on switch node '{}' is {:.1} mm of copper enclosing {:.1} mm² (capacitor {}); limits {max_len} mm and {max_area} mm²", s.net, r.length_mm, r.area_mm2, r.cap),
                        fix: "Put the capacitor against the switching device's pins and run its ground return on the shortest path (same layer, via beside the capacitor)".into(),
                        uuids: [r.cap_uuid.clone()].into_iter().flatten().chain(r.cap_pads.iter().flatten().cloned()).collect(),
                        at: Some(m.pad(s.inductor).1.at),
                    });
                }
                hot_rows.push(json!({ "switch_net": s.net, "topology": topo, "topology_basis": s.how, "loop": loop_json(&r) }));
            }
            Err(why) => {
                hot_rows.push(json!({ "switch_net": s.net, "topology": topo, "loop": Value::Null, "why": why }));
                notes.push(format!("switch node '{}': {why}", s.net));
            }
        }
    }
    if hot_scores.is_empty() {
        let kept = std::mem::take(&mut hot.needs_human);
        hot = CheckOut::not_run(
            "8.1",
            "switching_hot_loop",
            if notes.is_empty() {
                "no loop could be measured".to_string()
            } else {
                notes.join("; ")
            },
        );
        hot.needs_human = kept;
        hot.metrics = json!({ "switch_nodes": hot_rows });
    } else {
        hot.score = Some(hot_scores.iter().sum::<f64>() / hot_scores.len() as f64);
        if !notes.is_empty() {
            hot.status = "partial";
            hot.reason = Some(notes.join("; "));
        }
        hot.needs_human.push(("8.3", "Gate-drive loop, bootstrap capacitor, Kelvin sensing and recirculation in every switching state need the schematic's intent and datasheets".into()));
        hot.metrics = json!({
            "switch_nodes": hot_rows,
            "compare": { "worst_hot_loop_mm": r2(worst_len), "worst_hot_loop_area_mm2": r2(worst_area) },
            "method": "same copper path measure as 4.1, over the devices' pins: buck = input capacitor → VIN → PGND, boost = output capacitor → output pin or diode → switch → ground",
        });
    }

    // 8.2: copper on each switch node and its distance from feedback routes.
    let max_area_node = m.params.get("max_switch_node_area_mm2");
    let min_gap = m.params.get("min_switch_feedback_gap_mm");
    let mut node_rows = Vec::new();
    let mut node_scores = Vec::new();
    let mut total_area = 0.0;
    let mut min_gap_seen = f64::INFINITY;
    for s in switchers {
        let Some(bb) = m.net_bbox(&s.net, 2.0) else {
            continue;
        };
        let g = m.net_grid(&s.net, bb, Cover::Centre);
        let area = g.area_mm2();
        total_area += area;
        let reg_ics: Vec<usize> = s
            .parts
            .iter()
            .copied()
            .filter(|&fi| is_ic(m.fp_ref(&m.geo.footprints[fi])))
            .collect();
        // Feedback nets: named by the call, else pins called FB/VFB/ADJ/SENSE/COMP on the regulator.
        let mut fb: Vec<(String, String)> = m
            .params
            .feedback_nets
            .iter()
            .map(|n| (n.clone(), "argument".to_string()))
            .collect();
        for &fi in &reg_ics {
            for p in &m.geo.footprints[fi].pads {
                let (Some(net), Some(func)) = (p.net.as_deref(), p.pin_function.as_deref()) else {
                    continue;
                };
                let f = func.to_uppercase();
                if ["FB", "VFB", "ADJ", "SENSE", "COMP", "VSENSE"]
                    .iter()
                    .any(|k| f.starts_with(k))
                    && !m.is_ground(net)
                    && !m.is_power(net)
                    && !fb.iter().any(|(n, _)| n == net)
                {
                    fb.push((
                        net.to_string(),
                        format!("pin {} of {}", func, m.fp_ref(&m.geo.footprints[fi])),
                    ));
                }
            }
        }
        let own_ics: Vec<Pt> = reg_ics.iter().map(|&fi| m.geo.footprints[fi].at).collect();
        let (mut gap, mut overlap_mm, mut fb_uuid) = (f64::INFINITY, 0.0f64, Vec::new());
        let mut at = None;
        for (net, _) in &fb {
            for &ti in m.tracks.get(net).into_iter().flatten() {
                let t = m.track(ti);
                let Some(l) = t.layers.first().and_then(|l| m.layer_ix(l)) else {
                    continue;
                };
                // Pieces away from the regulator's own pins.
                for w in t.shape.pts.windows(2) {
                    let len = dist(w[0], w[1]);
                    let n = ((len / 0.25).ceil() as usize).max(1);
                    for k in 0..n {
                        let f = (k as f64 + 0.5) / n as f64;
                        let p = (
                            w[0].0 + (w[1].0 - w[0].0) * f,
                            w[0].1 + (w[1].1 - w[0].1) * f,
                        );
                        if own_ics.iter().any(|c| dist(*c, p) < 3.5) {
                            continue;
                        }
                        // Adjacent layers: switch copper straight above or below.
                        for al in [l.checked_sub(1), Some(l + 1)]
                            .into_iter()
                            .flatten()
                            .filter(|a| *a < m.layers.len())
                        {
                            if let Some((ix, iy)) = g.cell_of(p) {
                                if g.is_copper(al, ix, iy) {
                                    overlap_mm += len / n as f64;
                                    if fb_uuid.len() < 5 {
                                        fb_uuid.extend(t.uuid.clone());
                                    }
                                    break;
                                }
                            }
                        }
                        // Same layer: distance to the node's tracks, vias and other parts' pads.
                        let mut d = f64::INFINITY;
                        for &si in m.tracks.get(&s.net).into_iter().flatten() {
                            let st = m.track(si);
                            if st.layers.first() != t.layers.first() {
                                continue;
                            }
                            for sw in st.shape.pts.windows(2) {
                                d = d.min(
                                    point_segment_distance(p, sw[0], sw[1])
                                        - st.shape.radius
                                        - t.shape.radius,
                                );
                            }
                        }
                        for &pi in m.pads.get(&s.net).into_iter().flatten() {
                            let (pf, pad) = m.pad(pi);
                            if reg_ics.contains(&(pi.0)) || !pad.on_layer(&t.layers[0]) {
                                let _ = pf;
                                continue;
                            }
                            d = d.min(pad.shape.distance(
                                &konnect_sexp::board_geometry::Shape::disc(p, t.shape.radius),
                            ));
                        }
                        if d < gap {
                            gap = d;
                            at = Some(p);
                            fb_uuid.extend(t.uuid.clone());
                        }
                    }
                }
            }
        }
        let _ = (segment_distance as fn(Pt, Pt, Pt, Pt) -> f64,);
        min_gap_seen = min_gap_seen.min(gap);
        let gap_ok = gap >= min_gap;
        let fb_score = if fb.is_empty() {
            1.0
        } else if overlap_mm > 0.0 {
            0.0
        } else if gap_ok {
            1.0
        } else {
            (gap / min_gap).clamp(0.0, 1.0)
        };
        node_scores.push(0.5 * excess_score(area, max_area_node) + 0.5 * fb_score);
        if area > max_area_node {
            node.findings.push(Finding {
                severity: "warning",
                item: "8.2",
                subject: s.net.clone(),
                measured: json!({ "copper_area_mm2": r2(area) }),
                threshold: json!({ "max_switch_node_area_mm2": max_area_node }),
                issue: format!("Switch node '{}' has {:.1} mm² of copper on all layers; limit {max_area_node} mm²", s.net, area),
                fix: "Trim the node's pour or tracks to what the current and heat need".into(),
                uuids: m.tracks.get(&s.net).into_iter().flatten().filter_map(|&i| m.track(i).uuid.clone()).take(10).collect(),
                at: Some(m.pad(s.inductor).1.at),
            });
        }
        if !fb.is_empty() && (overlap_mm > 0.0 || !gap_ok) {
            node.findings.push(Finding {
                severity: "warning",
                item: "8.2",
                subject: format!("{} vs feedback", s.net),
                measured: json!({ "feedback_gap_mm": if gap.is_finite() { json!(r2(gap)) } else { Value::Null }, "feedback_over_switch_copper_mm": r2(overlap_mm), "feedback_nets": fb.iter().map(|f| f.0.clone()).collect::<Vec<_>>() }),
                threshold: json!({ "min_switch_feedback_gap_mm": min_gap, "overlap_mm": 0 }),
                issue: format!(
                    "Feedback route ({}) comes within {} mm of switch node '{}'{}",
                    fb.iter().map(|f| f.0.as_str()).collect::<Vec<_>>().join(", "),
                    if gap.is_finite() { format!("{gap:.2}") } else { "—".into() },
                    s.net,
                    if overlap_mm > 0.0 { format!(" and runs {overlap_mm:.1} mm over its copper on an adjacent layer") } else { String::new() }
                ),
                fix: "Route feedback on the far side of the regulator, over ground, away from the switch node and inductor".into(),
                uuids: fb_uuid.into_iter().take(10).collect(),
                at,
            });
        }
        node_rows.push(json!({
            "switch_net": s.net,
            "copper_area_mm2": r2(area),
            "feedback_nets": fb.iter().map(|f| json!({ "net": f.0, "found_from": f.1 })).collect::<Vec<_>>(),
            "feedback_min_gap_mm": if gap.is_finite() { json!(r2(gap)) } else { Value::Null },
            "feedback_over_switch_copper_mm": r2(overlap_mm),
        }));
    }
    if node_scores.is_empty() {
        node = CheckOut::not_run(
            "8.2",
            "switch_node_copper",
            "switch node copper could not be rasterised",
        );
    } else {
        node.score = Some(node_scores.iter().sum::<f64>() / node_scores.len() as f64);
        node.metrics = json!({
            "switch_nodes": node_rows,
            "compare": { "switch_node_area_mm2": r2(total_area), "feedback_min_gap_mm": if min_gap_seen.is_finite() { json!(r2(min_gap_seen)) } else { Value::Null } },
            "method": "the node's copper on every layer (cell count of its raster); feedback = pins named FB/VFB/ADJ/SENSE/COMP of the regulator plus feedback_nets, judged beyond 3.5 mm of the regulator's own pins",
        });
    }
    vec![hot, node]
}

// ─── 4.2 current sizing (reuses audit_trace_current) ─────────────────────────

fn current_sizing(m: &Model, board: &std::path::Path) -> CheckOut {
    let id = "4.2";
    let name = "copper_sizing_for_current";
    let result = match trace_current::audit(board, Some(m.params.get("default_temp_rise_c"))) {
        Ok(r) => r,
        Err(e) => return CheckOut::not_run(id, name, format!("audit_trace_current failed: {e}")),
    };
    let Some(crate::mcp::protocol::ToolContent::Text { text }) = result.content.first() else {
        return CheckOut::not_run(id, name, "audit_trace_current returned no text");
    };
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return CheckOut::not_run(id, name, "audit_trace_current returned unreadable output");
    };
    match v["status"].as_str() {
        Some("checked") => {}
        Some("no_design_intent") => {
            return CheckOut::not_run(id, name, "no design-intent record: no currents to size copper for (set_design_intent: 'currents', or 'rails' with max_current_a)")
        }
        Some("no_currents_recorded") => {
            return CheckOut::not_run(id, name, "the design-intent record holds no currents")
        }
        other => return CheckOut::not_run(id, name, format!("audit_trace_current status {other:?}")),
    }
    let mut out = CheckOut::checked(id, name);
    let nets = v["nets"].as_array().cloned().unwrap_or_default();
    let mut bad = 0usize;
    for f in v["findings"].as_array().into_iter().flatten() {
        let sev = f["severity"].as_str().unwrap_or("info");
        if sev == "info" {
            continue;
        }
        let net = f["net"].as_str().unwrap_or("").to_string();
        let body = nets.iter().find(|n| n["net"] == net.as_str());
        let uuids: Vec<String> = body
            .and_then(|b| b["undersized"].as_array())
            .into_iter()
            .flatten()
            .filter_map(|u| u["uuid"].as_str().map(str::to_string))
            .collect();
        if body.is_some() {
            bad += 1;
        }
        out.findings.push(Finding {
            severity: if sev == "error" { "error" } else { "warning" },
            item: id,
            subject: net,
            measured: body.map(|b| json!({ "narrowest": b["narrowest"], "undersized_count": b["undersized_count"], "vias": b["vias"] })).unwrap_or(Value::Null),
            threshold: body.map(|b| json!({ "current_a": b["current_a"], "temp_rise_c": b["temp_rise_c"], "formula": "IPC-2221" })).unwrap_or(Value::Null),
            issue: f["issue"].as_str().unwrap_or("").to_string(),
            fix: f["recommendation"].as_str().unwrap_or("").to_string(),
            uuids,
            at: None,
        });
    }
    let n = nets.len().max(1);
    out.score = (!nets.is_empty()).then(|| 1.0 - (bad.min(n) as f64 / n as f64));
    out.metrics = json!({
        "nets_checked": nets.len(),
        "nets_with_problems": bad,
        "via": "audit_trace_current",
        "compare": { "nets_undersized": bad },
        "not_checked_there": v["not_checked"],
    });
    out
}

// ─── 3.4 / 4.3 neck-downs ────────────────────────────────────────────────────

struct GridCache {
    grids: HashMap<String, (Grid, Vec<Vec<f32>>)>,
}

fn bottlenecks(m: &Model) -> Vec<CheckOut> {
    let min_a = m.params.get("bottleneck_min_current_a");
    let mut reqs: Vec<&super::CurrentReq> = m
        .roles
        .currents
        .iter()
        .filter(|c| {
            c.amps >= min_a
                && !m.is_ground(&c.net)
                && m.pads.get(&c.net).is_some_and(|p| p.len() >= 2)
        })
        .collect();
    reqs.sort_by(|a, b| a.net.cmp(&b.net));
    reqs.dedup_by(|a, b| a.net == b.net);
    let (mut power, mut ground) = (
        CheckOut::checked("4.3", "neck_downs_on_power_paths"),
        CheckOut::checked("3.4", "ground_return_bottlenecks"),
    );
    if reqs.is_empty() {
        let why = if m.roles.currents.is_empty() {
            "no currents recorded in the design intent: nothing says which paths must carry how much".to_string()
        } else {
            format!(
                "no net with a recorded current of at least {min_a} A and two pads on the board"
            )
        };
        return vec![
            CheckOut::not_run("4.3", "neck_downs_on_power_paths", &why),
            CheckOut::not_run("3.4", "ground_return_bottlenecks", why),
        ];
    }
    let mut cache = GridCache {
        grids: HashMap::new(),
    };
    let (mut p_ratios, mut g_ratios) = (Vec::new(), Vec::new());
    let (mut p_rows, mut g_rows) = (Vec::new(), Vec::new());
    let mut runs = 0usize;
    for req in reqs {
        let pads = &m.pads[&req.net];
        // Source: the supplier from the record, else the largest pad on the net.
        let rail_src = m
            .roles
            .rails
            .get(&req.net)
            .and_then(|r| r.source_ref.clone());
        let area = |pi: &PadIx| {
            let p = m.pad(*pi).1;
            p.size.0 * p.size.1
        };
        let src = pads
            .iter()
            .filter(|pi| {
                rail_src
                    .as_deref()
                    .is_none_or(|r| m.fp_ref(m.pad(**pi).0) == r)
            })
            .max_by(|a, b| {
                area(a)
                    .partial_cmp(&area(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .or_else(|| {
                pads.iter().max_by(|a, b| {
                    area(a)
                        .partial_cmp(&area(b))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            })
            .copied();
        let Some(src) = src else { continue };
        let mut sinks: Vec<PadIx> = pads.iter().copied().filter(|pi| pi.0 != src.0).collect();
        sinks.sort_by(|a, b| {
            dist(m.pad(*a).1.at, m.pad(src).1.at)
                .partial_cmp(&dist(m.pad(*b).1.at, m.pad(src).1.at))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sinks.truncate(12);
        let required: Vec<f64> = m
            .layers
            .iter()
            .map(|l| {
                trace_current::required_area_mm2(req.amps, req.rise_c, trace_current::is_outer(l))
                    / m.thickness(l)
            })
            .collect();
        // Power path.
        for (kind, net_of) in [("power", None::<()>), ("ground", Some(()))] {
            for &sink in &sinks {
                if runs >= 80 {
                    break;
                }
                let (a, b, net) = if net_of.is_none() {
                    (src, sink, req.net.clone())
                } else {
                    let near = |fp: usize, to: Pt| {
                        nearest_pad(m, fp, to, |p| {
                            p.net.as_deref().is_some_and(|n| m.is_ground(n))
                        })
                    };
                    let (Some(ga), Some(gb)) =
                        (near(src.0, m.pad(src).1.at), near(sink.0, m.pad(sink).1.at))
                    else {
                        continue;
                    };
                    let (na, nb) = (
                        m.pad(ga).1.net.clone().unwrap_or_default(),
                        m.pad(gb).1.net.clone().unwrap_or_default(),
                    );
                    if na != nb {
                        continue;
                    }
                    (ga, gb, na)
                };
                runs += 1;
                if !cache.grids.contains_key(&net) {
                    let Some(bb) = m.net_bbox(&net, 1.0) else {
                        continue;
                    };
                    let g = m.net_grid(&net, bb, Cover::Centre);
                    let dt = g.distance_transform();
                    cache.grids.insert(net.clone(), (g, dt));
                }
                let (g, dt) = &cache.grids[&net];
                let all_pads: Vec<Cell> = m
                    .pads
                    .get(&net)
                    .into_iter()
                    .flatten()
                    .flat_map(|pi| {
                        let p = m.pad(*pi).1;
                        g.cells_under(&p.shape, &m.pad_layers(p))
                    })
                    .collect();
                let (pa, pb) = (m.pad(a).1, m.pad(b).1);
                let from = g.cells_under(&pa.shape, &m.pad_layers(pa));
                let to = g.cells_under(&pb.shape, &m.pad_layers(pb));
                let req_cells: Vec<f64> = required.clone();
                let label = format!("{} → {}", pad_label(m, a), pad_label(m, b));
                let w = grid::widest_path(g, dt, &from, &to, &req_cells, &all_pads);
                let (row, ratio) = match w {
                    Some(w) if w.width_mm.is_finite() => {
                        let need = required[w.layer];
                        let ok = w.width_mm + 1.5 * g.step >= need;
                        let ratio = (w.width_mm / need).clamp(0.0, 1.0);
                        let ratio = if ok { 1.0 } else { ratio };
                        if !ok {
                            let target = if kind == "power" {
                                &mut power
                            } else {
                                &mut ground
                            };
                            target.findings.push(Finding {
                                severity: if w.width_mm < 0.5 * need { "error" } else { "warning" },
                                item: if kind == "power" { "4.3" } else { "3.4" },
                                subject: format!("{} {}", req.net, label),
                                measured: json!({ "narrowest_mm": r2(w.width_mm), "layer": m.layers[w.layer] }),
                                threshold: json!({ "required_mm": r2(need), "current_a": req.amps, "current_from": req.source, "temp_rise_c": req.rise_c, "formula": "IPC-2221" }),
                                issue: format!(
                                    "{} path {label} for {} A narrows to {:.2} mm on {} near ({:.1}, {:.1}); {:.2} mm needed at {} °C rise",
                                    if kind == "power" { "Power" } else { "Ground return" }, req.amps, w.width_mm, m.layers[w.layer], w.at.0, w.at.1, need, req.rise_c
                                ),
                                fix: "Widen the neck, add a parallel path or pour, or record a lower current for this branch".into(),
                                uuids: neck_uuids(m, &net, w.at, w.layer),
                                at: Some(w.at),
                            });
                        }
                        (
                            json!({ "path": label, "net": net, "narrowest_mm": r2(w.width_mm), "required_mm": r2(need), "at": [r2(w.at.0), r2(w.at.1)], "layer": m.layers[w.layer] }),
                            Some(ratio),
                        )
                    }
                    Some(_) => (
                        json!({ "path": label, "net": net, "note": "pads joined directly" }),
                        Some(1.0),
                    ),
                    None => (
                        json!({ "path": label, "net": net, "note": "no copper path between the pads (unrouted or not rasterised)" }),
                        None,
                    ),
                };
                if kind == "power" {
                    p_rows.push(row);
                    p_ratios.extend(ratio);
                } else {
                    g_rows.push(row);
                    g_ratios.extend(ratio);
                }
            }
        }
    }
    for (out, ratios, rows, id) in [
        (&mut power, &p_ratios, &p_rows, "4.3"),
        (&mut ground, &g_ratios, &g_rows, "3.4"),
    ] {
        if ratios.is_empty() {
            *out = CheckOut::not_run(id, out.name, "no source-to-load copper path could be measured (pads not joined by copper, or off the grid)");
            continue;
        }
        out.score = Some(ratios.iter().sum::<f64>() / ratios.len() as f64);
        out.metrics = json!({
            "paths": rows.iter().take(25).cloned().collect::<Vec<_>>(),
            "paths_measured": ratios.len(),
            "compare": { "narrow_paths": ratios.iter().filter(|r| **r < 1.0).count() },
            "method": "widest-path search over a raster of the net's copper: the path whose narrowest local width (twice the distance to the nearest non-copper cell) is largest, against the IPC-2221 width for the recorded current on that layer; end pads are exempt; a single best path is judged, parallel copper is not summed; layer changes are free here (via capacity is audited by audit_trace_current)",
        });
    }
    vec![power, ground]
}

fn neck_uuids(m: &Model, net: &str, at: Pt, layer: usize) -> Vec<String> {
    let mut best: Vec<(f64, String)> = Vec::new();
    for &i in m.tracks.get(net).into_iter().flatten() {
        let t = m.track(i);
        if t.layers.first().map(String::as_str) != Some(m.layers[layer].as_str()) {
            continue;
        }
        let d = t
            .shape
            .pts
            .windows(2)
            .map(|w| point_segment_distance(at, w[0], w[1]))
            .fold(f64::INFINITY, f64::min);
        if let Some(u) = &t.uuid {
            best.push((d, u.clone()));
        }
    }
    for f in m
        .fills
        .iter()
        .filter(|f| f.net.as_deref() == Some(net) && f.layer == m.layers[layer])
    {
        if f.rings.iter().any(|r| {
            at.0 >= r.bbox.0 - 1.0
                && at.0 <= r.bbox.2 + 1.0
                && at.1 >= r.bbox.1 - 1.0
                && at.1 <= r.bbox.3 + 1.0
        }) {
            if let Some(u) = &f.zone_uuid {
                best.push((0.5, u.clone()));
            }
        }
    }
    best.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    best.into_iter().take(3).map(|b| b.1).collect()
}
