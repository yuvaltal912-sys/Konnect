//! Checklist §5: detours, vias, stubs, parallel runs, corners and differential
//! pairs; and the completion gate.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use konnect_sexp::planar::{point_segment_distance, segment_distance, Pt};
use serde_json::{json, Value};

use super::planes::RefMap;
use super::{dist, excess_score, path_len, r2, r3, CheckOut, Finding, Model};
use crate::tools::dfm;

/// Pads of a net and the copper groups they fall into.
pub(crate) struct Conn {
    pub groups: usize,
    /// A zone of the net has no saved fill, so a split may only be a pour that was never filled.
    pub unfilled_zone: Option<String>,
}

/// Largest gap between two copper edges still counted as touching (mm); the same tolerance the
/// dangling-end test uses.
const TOUCH_MM: f64 = 0.01;

/// The copper group of each pad of `net`, aligned with `m.pads[net]`. Decided on the real
/// shapes, not the raster: two items are joined when they share a copper layer and their
/// shapes touch within [`TOUCH_MM`], so a track that stops 0.15 mm short of its pad is open,
/// as it is for KiCad. A pad with no copper layer gets a group of its own.
pub(super) fn pad_groups(m: &Model, net: &str) -> Vec<usize> {
    use konnect_sexp::board_geometry::Shape;
    struct Item {
        shape: Shape,
        mask: u64,
        bbox: (f64, f64, f64, f64),
    }
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let bit = |l: usize| if l < 64 { 1u64 << l } else { 0 };
    let mut items: Vec<Item> = Vec::new();
    let add = |shape: Shape, mask: u64, items: &mut Vec<Item>| -> Option<usize> {
        let bbox = shape.bbox()?;
        if mask == 0 {
            return None;
        }
        items.push(Item { shape, mask, bbox });
        Some(items.len() - 1)
    };
    for &i in m.tracks.get(net).into_iter().flatten() {
        let t = m.track(i);
        if let Some(l) = t.layers.first().and_then(|l| m.layer_ix(l)) {
            add(t.shape.clone(), bit(l), &mut items);
        }
    }
    for &i in m.vias.get(net).into_iter().flatten() {
        let v = m.track(i);
        if let Some((lo, hi)) = m.via_span(v) {
            let mask = (lo..=hi).fold(0, |a, l| a | bit(l));
            add(v.shape.clone(), mask, &mut items);
        }
    }
    let pad_item: Vec<Option<usize>> = m
        .pads
        .get(net)
        .into_iter()
        .flatten()
        .map(|&pi| {
            let pad = m.pad(pi).1;
            let mask = m.pad_layers(pad).into_iter().fold(0, |a, l| a | bit(l));
            add(pad.shape.clone(), mask, &mut items)
        })
        .collect();
    for f in m.fills.iter().filter(|f| f.net.as_deref() == Some(net)) {
        let Some(l) = m.layer_ix(&f.layer) else {
            continue;
        };
        for r in &f.rings {
            add(Shape::polygon(r.pts.clone()), bit(l), &mut items);
        }
    }
    // Sweep along x; only items whose boxes meet are measured, and never two already joined.
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| {
        items[a]
            .bbox
            .0
            .partial_cmp(&items[b].bbox.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut parent: Vec<usize> = (0..items.len()).collect();
    for (k, &a) in order.iter().enumerate() {
        for &b in &order[k + 1..] {
            if items[b].bbox.0 > items[a].bbox.2 + TOUCH_MM {
                break;
            }
            let (ia, ib) = (&items[a], &items[b]);
            if ia.mask & ib.mask == 0
                || ia.bbox.1 > ib.bbox.3 + TOUCH_MM
                || ib.bbox.1 > ia.bbox.3 + TOUCH_MM
            {
                continue;
            }
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            if ra != rb && ia.shape.distance(&ib.shape) <= TOUCH_MM {
                parent[ra] = rb;
            }
        }
    }
    pad_item
        .iter()
        .enumerate()
        .map(|(k, it)| match it {
            Some(i) => find(&mut parent, *i),
            None => items.len() + k,
        })
        .collect()
}

pub(super) fn connectivity<'a>(m: &'a Model) -> &'a HashMap<String, Conn> {
    m.conn.get_or_init(|| {
        let mut out = HashMap::new();
        for (net, pads) in &m.pads {
            if pads.len() < 2 {
                continue;
            }
            let groups: BTreeSet<usize> = pad_groups(m, net).into_iter().collect();
            out.insert(
                net.clone(),
                Conn {
                    groups: groups.len(),
                    unfilled_zone: m
                        .unfilled_zone_of(net)
                        .map(|z| z.uuid.clone().unwrap_or_else(|| "?".into())),
                },
            );
        }
        out
    })
}

pub(super) fn completion(m: &Model) -> Value {
    let conn = connectivity(m);
    let mut unrouted: Vec<(&String, usize)> = Vec::new();
    // Open nets that own a zone with no saved fill: the pour may be what joins them.
    let mut not_judged: Vec<(&String, usize, &String)> = Vec::new();
    for (n, c) in conn.iter().filter(|(_, c)| c.groups > 1) {
        match &c.unfilled_zone {
            Some(z) => not_judged.push((n, c.groups, z)),
            None => unrouted.push((n, c.groups)),
        }
    }
    unrouted.sort();
    not_judged.sort();
    let complete = if !unrouted.is_empty() {
        json!(false)
    } else if !not_judged.is_empty() {
        Value::Null
    } else {
        json!(true)
    };
    let mut out = json!({
        "complete": complete,
        "status": if not_judged.is_empty() { "checked" } else { "partial" },
        "nets_with_two_or_more_pads": conn.len(),
        "unrouted_nets": unrouted.len(),
        "unrouted_listed": unrouted.iter().take(20).map(|(n, g)| json!({ "net": n, "copper_groups": g })).collect::<Vec<_>>(),
        "nets_not_judged": not_judged.len(),
        "not_judged_listed": not_judged.iter().take(20).map(|(n, g, z)| json!({ "net": n, "copper_groups": g, "zone": z })).collect::<Vec<_>>(),
        "method": "pads of a net joined through its tracks, vias, pads and saved zone fills when their shapes touch (within 0.01 mm) on a shared copper layer; a gate, not a quality measure",
    });
    if !not_judged.is_empty() {
        out["reason"] = json!(format!(
            "{} open net(s) own a zone with no saved fill, so whether the pour joins them is unknown (refill zones and save the board); they are left out of routed_share",
            not_judged.len()
        ));
    }
    out
}

pub(super) fn run(m: &Model, refs: &RefMap) -> Vec<CheckOut> {
    let (detours, per_net_stubs) = detours_vias_stubs(m);
    vec![
        detours,
        parallel_runs(m),
        corners(m),
        diff_pairs(m, refs, &per_net_stubs),
    ]
}

fn mst_len(pts: &[Pt]) -> f64 {
    if pts.len() < 2 {
        return 0.0;
    }
    let mut in_tree = vec![false; pts.len()];
    let mut best = vec![f64::INFINITY; pts.len()];
    best[0] = 0.0;
    let mut total = 0.0;
    for _ in 0..pts.len() {
        let Some(u) = (0..pts.len()).filter(|&i| !in_tree[i]).min_by(|&a, &b| {
            best[a]
                .partial_cmp(&best[b])
                .unwrap_or(std::cmp::Ordering::Equal)
        }) else {
            break;
        };
        in_tree[u] = true;
        total += best[u];
        for v in 0..pts.len() {
            if !in_tree[v] {
                best[v] = best[v].min(dist(pts[u], pts[v]));
            }
        }
    }
    total
}

fn hpwl(pts: &[Pt]) -> f64 {
    konnect_sexp::planar::bbox_of(pts).map_or(0.0, |b| (b.2 - b.0) + (b.3 - b.1))
}

/// Dangling ends of a net's tracks: neither on a pad or via of the net, nor on
/// another of its tracks, nor inside its pour.
fn stubs_of(m: &Model, net: &str) -> Vec<(String, Pt)> {
    let idxs = m.tracks.get(net).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for &i in &idxs {
        let t = m.track(i);
        let layer = t.layers.first().cloned().unwrap_or_default();
        let r = t.shape.radius;
        let pts = &t.shape.pts;
        let (Some(&a), Some(&b)) = (pts.first(), pts.last()) else {
            continue;
        };
        for end in [a, b] {
            let on_pad = m.pads.get(net).into_iter().flatten().any(|&pi| {
                let p = m.pad(pi).1;
                p.on_layer(&layer)
                    && p.shape
                        .distance(&konnect_sexp::board_geometry::Shape::disc(end, 0.0))
                        <= r + 0.01
            });
            let on_via = m.vias.get(net).into_iter().flatten().any(|&vi| {
                let v = m.track(vi);
                dist(v.at, end) <= v.shape.radius + r + 0.01
                    && m.via_span(v).is_some_and(|(lo, hi)| {
                        m.layer_ix(&layer).is_some_and(|l| (lo..=hi).contains(&l))
                    })
            });
            let on_track = idxs.iter().any(|&j| {
                if j == i {
                    return false;
                }
                let o = m.track(j);
                o.layers.first() == Some(&layer)
                    && o.shape
                        .pts
                        .windows(2)
                        .any(|w| point_segment_distance(end, w[0], w[1]) <= o.shape.radius + 0.01)
            });
            let in_pour = m
                .fill_at(&layer, end)
                .is_some_and(|f| m.fills[f].net.as_deref() == Some(net));
            if !(on_pad || on_via || on_track || in_pour) {
                out.push((t.uuid.clone().unwrap_or_default(), end));
            }
        }
    }
    out
}

/// Names of nets a measure left out, bounded; `count` is the true total.
fn named(list: &[(String, &str)]) -> Value {
    json!({
        "count": list.len(),
        "listed": list.iter().take(20).map(|(n, why)| json!({ "net": n, "why": why })).collect::<Vec<_>>(),
    })
}

fn detours_vias_stubs(m: &Model) -> (CheckOut, HashMap<String, usize>) {
    let id = "5.1";
    let name = "detours_vias_stubs";
    let conn = connectivity(m);
    let (max_ratio, max_vias) = (
        m.params.get("max_detour_ratio"),
        m.params.get("max_vias_per_connection"),
    );
    let mut out = CheckOut::checked(id, name);
    let mut stub_counts: HashMap<String, usize> = HashMap::new();
    let (mut wire, mut mst_total, mut hp_total) = (0.0, 0.0, 0.0);
    let (mut vias_total, mut stubs_total, mut measured) = (0usize, 0usize, 0usize);
    let (mut weighted, mut weight) = (0.0, 0.0);
    let mut rows: Vec<(f64, Value)> = Vec::new();
    // Nets a measure could not judge, named: never only counted.
    let mut no_ratio: Vec<(String, &str)> = Vec::new();
    let mut no_via_ratio: Vec<(String, &str)> = Vec::new();
    let mut not_scored: Vec<(String, &str)> = Vec::new();
    let mut skipped_unrouted: Vec<(String, &str)> = Vec::new();
    let mut nets: Vec<&String> = m.tracks.keys().collect();
    nets.sort();
    for net in nets {
        let pads: Vec<Pt> = m
            .pads
            .get(net)
            .into_iter()
            .flatten()
            .map(|&pi| m.pad(pi).1.at)
            .collect();
        if pads.len() < 2 {
            continue;
        }
        if let Some(c) = conn.get(net).filter(|c| c.groups > 1) {
            skipped_unrouted.push((
                net.clone(),
                if c.unfilled_zone.is_some() {
                    "open, and its zone has no saved fill"
                } else {
                    "not fully routed"
                },
            ));
            continue;
        }
        let has_pour = m
            .fills
            .iter()
            .any(|f| f.net.as_deref() == Some(net.as_str()));
        let routed: f64 = m.tracks[net]
            .iter()
            .map(|&i| path_len(&m.track(i).shape.pts))
            .sum();
        let vias = m.vias.get(net).map_or(0, Vec::len);
        let stubs = stubs_of(m, net);
        stub_counts.insert(net.clone(), stubs.len());
        stubs_total += stubs.len();
        vias_total += vias;
        let mst = mst_len(&pads);
        // Ground, power and pour nets carry stitching, thermal and fan-out vias the checklist
        // asks for (§3, §4), so their vias are not layer changes to count against the net.
        let judge_vias = !(has_pour || m.is_ground(net) || m.is_power(net));
        let vpc = judge_vias.then(|| vias as f64 / (pads.len() - 1) as f64);
        // A tree under 1 mm is judged against 1 mm, so two pads side by side cannot hide a
        // long route.
        let ratio = (!has_pour).then(|| routed / mst.max(1.0));
        if !judge_vias {
            no_via_ratio.push((
                net.clone(),
                if m.is_ground(net) || m.is_power(net) {
                    "ground or power net: its vias are stitching, not layer changes"
                } else {
                    "carried by a pour: its vias join the pour"
                },
            ));
        }
        if ratio.is_none() {
            no_ratio.push((net.clone(), "carried by a pour"));
        }
        let mut s = vpc.map_or(1.0, |v| excess_score(v, max_vias.max(0.5)));
        if let Some(r) = ratio {
            s = s.min(excess_score(r, max_ratio));
            wire += routed;
            // The same 1 mm floor as the per-net ratio, so the total agrees with them.
            mst_total += mst.max(1.0);
            hp_total += hpwl(&pads);
            measured += 1;
        }
        if !stubs.is_empty() {
            s *= 0.7;
        }
        // A net with neither a detour nor a via ratio was not measured: it must not lift the
        // score just by having no dangling end. Its stubs are still findings and counted.
        if ratio.is_some() || vpc.is_some() {
            let w = routed.max(1.0);
            weighted += s * w;
            weight += w;
        } else {
            not_scored.push((net.clone(), "no detour ratio and no via ratio"));
        }
        let uuid_of = |net: &str| -> Vec<String> {
            m.tracks[net]
                .iter()
                .filter_map(|&i| m.track(i).uuid.clone())
                .take(8)
                .collect()
        };
        if ratio.is_some_and(|r| r > max_ratio)
            || vpc.is_some_and(|v| v > max_vias)
            || !stubs.is_empty()
        {
            let mut why = Vec::new();
            if let Some(r) = ratio.filter(|r| *r > max_ratio) {
                why.push(format!("routed {routed:.1} mm for a {mst:.1} mm pin-to-pin tree ({r:.2}×; limit {max_ratio}×)"));
            }
            if let Some(v) = vpc.filter(|v| *v > max_vias) {
                why.push(format!(
                    "{vias} via(s) for {} connection(s) ({v:.1} per connection; limit {max_vias})",
                    pads.len() - 1
                ));
            }
            if !stubs.is_empty() {
                why.push(format!("{} dangling track end(s)", stubs.len()));
            }
            out.findings.push(Finding {
                severity: "warning",
                item: id,
                subject: net.clone(),
                measured: json!({ "routed_mm": r2(routed), "pin_tree_mm": r2(mst), "detour_ratio": ratio.map(r2), "vias": vias, "vias_per_connection": vpc.map(r2), "dangling_ends": stubs.len(), "stub_uuids": stubs.iter().take(5).map(|s| s.0.clone()).collect::<Vec<_>>() }),
                threshold: json!({ "max_detour_ratio": max_ratio, "max_vias_per_connection": max_vias, "dangling_ends": 0 }),
                issue: format!("Net '{net}': {}", why.join("; ")),
                fix: "Re-route more directly, remove the spur, or drop the extra layer changes".into(),
                uuids: if stubs.is_empty() { uuid_of(net) } else { stubs.iter().map(|s| s.0.clone()).take(8).collect() },
                at: stubs.first().map(|s| s.1),
            });
        }
        rows.push((ratio.unwrap_or(0.0), json!({ "net": net, "routed_mm": r2(routed), "pin_tree_mm": r2(mst), "detour_ratio": ratio.map(r2), "vias": vias, "dangling_ends": stubs.len() })));
    }
    if rows.is_empty() {
        let mut o = CheckOut::not_run(
            id,
            name,
            "no completely routed net with tracks and two or more pads",
        );
        o.metrics = json!({ "unrouted_nets_skipped": named(&skipped_unrouted) });
        return (o, stub_counts);
    }
    rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    if weight > 0.0 {
        out.score = Some(weighted / weight);
    } else {
        out.status = "partial";
        out.reason = Some("no net could be given a detour or via ratio (all are carried by a pour, or are ground or power nets); only dangling ends were checked".into());
    }
    out.metrics = json!({
        "nets_measured": rows.len(),
        "nets_with_a_detour_ratio": measured,
        "pour_carried_nets_without_a_ratio": no_ratio.iter().filter(|n| n.1 == "carried by a pour").count(),
        "nets_without_a_detour_ratio": named(&no_ratio),
        "nets_without_a_via_ratio": named(&no_via_ratio),
        "nets_not_scored": named(&not_scored),
        "unrouted_nets_skipped": named(&skipped_unrouted),
        "compare": {
            "wire_to_pin_tree": (mst_total > 0.0).then(|| r3(wire / mst_total)),
            "wire_to_hpwl": (hp_total > 0.0).then(|| r3(wire / hp_total)),
            "total_wire_mm": r2(wire),
            "vias": vias_total,
            "dangling_ends": stubs_total,
        },
        "worst_detours": rows.into_iter().take(10).map(|r| r.1).collect::<Vec<_>>(),
        "method": "routed track length over the Euclidean minimum spanning tree of the net's pad centres (a tree shorter than any rectilinear route; HPWL also reported); a tree under 1 mm is judged against 1 mm; vias per connection are counted on signal nets only (ground, power and pour nets carry stitching vias); nets that get no ratio are named in the metrics and, with no ratio at all, are left out of the score",
    });
    (out, stub_counts)
}

// ─── 5.2 parallel runs ───────────────────────────────────────────────────────

/// Total run, closest gap, example UUIDs and where, for two nets.
type PairRun = (f64, f64, Vec<String>, Pt);

fn parallel_runs(m: &Model) -> CheckOut {
    let id = "5.2";
    let name = "parallel_runs";
    let gap_limit = m.params.get("parallel_max_gap_mm");
    let run_limit = m.params.get("parallel_max_run_mm");
    // Segments of signal nets, bucketed by layer and 3 mm cell.
    let cell = 3.0;
    // A pair is compared only inside a shared cell, so each segment is filed over its box grown
    // by the farthest a partner can be: twice the gap limit (aggressors) between the two edges,
    // plus both half-widths. Without it a pair across a cell line is never compared.
    let max_r = m
        .tracks
        .values()
        .flatten()
        .map(|&i| m.track(i).shape.radius)
        .fold(0.0f64, f64::max);
    let reach = 2.0 * gap_limit + 2.0 * max_r;
    let mut buckets: HashMap<(String, i64, i64), Vec<(usize, usize)>> = HashMap::new();
    let mut segs: Vec<(String, String, Pt, Pt, f64, Option<String>)> = Vec::new();
    let pair_of: HashMap<&str, &str> = m
        .roles
        .pairs
        .iter()
        .flat_map(|(p, n, _)| [(p.as_str(), n.as_str()), (n.as_str(), p.as_str())])
        .collect();
    let mut signal_len = 0.0;
    for (net, idxs) in &m.tracks {
        if m.is_ground(net) || m.is_power(net) {
            continue;
        }
        for &i in idxs {
            let t = m.track(i);
            let layer = t.layers.first().cloned().unwrap_or_default();
            for w in t.shape.pts.windows(2) {
                if dist(w[0], w[1]) < 0.05 {
                    continue;
                }
                signal_len += dist(w[0], w[1]);
                let k = segs.len();
                segs.push((
                    net.clone(),
                    layer.clone(),
                    w[0],
                    w[1],
                    t.shape.radius,
                    t.uuid.clone(),
                ));
                let (x0, x1) = (w[0].0.min(w[1].0) - reach, w[0].0.max(w[1].0) + reach);
                let (y0, y1) = (w[0].1.min(w[1].1) - reach, w[0].1.max(w[1].1) + reach);
                for cx in ((x0 / cell).floor() as i64)..=((x1 / cell).floor() as i64) {
                    for cy in ((y0 / cell).floor() as i64)..=((y1 / cell).floor() as i64) {
                        buckets
                            .entry((layer.clone(), cx, cy))
                            .or_default()
                            .push((k, 0));
                    }
                }
            }
        }
    }
    if segs.is_empty() {
        return CheckOut::not_run(id, name, "no signal tracks (non-power, non-ground)");
    }
    let aggressor = |n: &str| m.roles.critical.contains_key(n) || n.to_uppercase().contains("SW");
    // (netA, netB) → (run mm, closest gap, uuids)
    let mut pairs: BTreeMap<(String, String), PairRun> = BTreeMap::new();
    let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
    for list in buckets.values() {
        for (ai, &(a, _)) in list.iter().enumerate() {
            for &(b, _) in &list[ai + 1..] {
                let (sa, sb) = (&segs[a], &segs[b]);
                if sa.0 == sb.0 || pair_of.get(sa.0.as_str()) == Some(&sb.0.as_str()) {
                    continue;
                }
                let key = (a.min(b), a.max(b));
                if !seen.insert(key) {
                    continue;
                }
                let factor = if aggressor(&sa.0) || aggressor(&sb.0) {
                    2.0
                } else {
                    1.0
                };
                let (ua, la) = unit(sa.2, sa.3);
                let (ub, _) = unit(sb.2, sb.3);
                if (ua.0 * ub.1 - ua.1 * ub.0).abs() > 0.17 {
                    continue; // more than ~10° apart
                }
                let rel = |p: Pt| (p.0 - sa.2 .0) * ua.0 + (p.1 - sa.2 .1) * ua.1;
                let (t0, t1) = (rel(sb.2), rel(sb.3));
                let overlap = (t0.max(t1).min(la) - t0.min(t1).max(0.0)).max(0.0);
                if overlap <= 0.0 {
                    continue;
                }
                let mid = ((sb.2 .0 + sb.3 .0) / 2.0, (sb.2 .1 + sb.3 .1) / 2.0);
                let gap = point_to_line(mid, sa.2, ua) - sa.4 - sb.4;
                if gap > gap_limit * factor {
                    continue;
                }
                let k = if sa.0 < sb.0 {
                    (sa.0.clone(), sb.0.clone())
                } else {
                    (sb.0.clone(), sa.0.clone())
                };
                let e = pairs
                    .entry(k)
                    .or_insert((0.0, f64::INFINITY, Vec::new(), mid));
                e.0 += overlap;
                if gap < e.1 {
                    e.1 = gap;
                    e.3 = mid;
                }
                for u in [&sa.5, &sb.5].into_iter().flatten() {
                    if e.2.len() < 8 && !e.2.contains(u) {
                        e.2.push(u.clone());
                    }
                }
            }
        }
    }
    let mut out = CheckOut::checked(id, name);
    let mut violating = 0.0;
    let mut longest = 0.0f64;
    let mut rows = Vec::new();
    for ((a, b), (run, gap, uuids, at)) in &pairs {
        let factor = if aggressor(a) || aggressor(b) {
            2.0
        } else {
            1.0
        };
        let limit = if factor > 1.0 {
            run_limit / 2.0
        } else {
            run_limit
        };
        longest = longest.max(*run);
        rows.push(
            json!({ "nets": [a, b], "parallel_mm": r2(*run), "closest_gap_mm": r2(gap.max(0.0)) }),
        );
        if *run > limit {
            violating += run;
            out.findings.push(Finding {
                severity: "warning",
                item: id,
                subject: format!("{a} / {b}"),
                measured: json!({ "parallel_mm": r2(*run), "closest_gap_mm": r2(gap.max(0.0)) }),
                threshold: json!({ "parallel_max_gap_mm": gap_limit * factor, "parallel_max_run_mm": limit }),
                issue: format!("Nets '{a}' and '{b}' run side by side for {run:.1} mm within {:.2} mm of each other (closest {:.2} mm); limit {limit} mm at {} mm", gap_limit * factor, gap.max(0.0), gap_limit * factor),
                fix: "Space them apart (about 3× the track width) or separate them with a ground track".into(),
                uuids: uuids.clone(),
                at: Some(*at),
            });
        }
    }
    rows.sort_by(|x, y| {
        y["parallel_mm"]
            .as_f64()
            .partial_cmp(&x["parallel_mm"].as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.score = Some(1.0 - (5.0 * violating / signal_len.max(1.0)).min(1.0));
    out.metrics = json!({
        "signal_length_mm": r2(signal_len),
        "pairs_running_together": pairs.len(),
        "compare": { "parallel_run_over_limit_mm": r2(violating), "longest_parallel_run_mm": r2(longest) },
        "worst": rows.into_iter().take(10).collect::<Vec<_>>(),
        "method": "same-layer segments within 10° of parallel and within the gap limit (doubled when either net is critical or a switch node); differential-pair partners excluded; adjacent-layer coupling is not measured",
    });
    out
}

fn unit(a: Pt, b: Pt) -> (Pt, f64) {
    let l = dist(a, b).max(1e-9);
    (((b.0 - a.0) / l, (b.1 - a.1) / l), l)
}

fn point_to_line(p: Pt, origin: Pt, u: Pt) -> f64 {
    ((p.0 - origin.0) * u.1 - (p.1 - origin.1) * u.0).abs()
}

// ─── 5.3 corners ─────────────────────────────────────────────────────────────

pub(super) type CornerRow = (String, String, Pt, Vec<String>);

/// Bends, right angles, acute bends this scan finds, right-angle rows, and
/// junctions holding a track shorter than 10 um. Junctions are read as the
/// acid-trap check reads them (`dfm::junctions`): a track shorter than 10 um
/// belongs to the junction it touches and gives no direction. A bend is two
/// real track ends of one net meeting, outside any pad or via of that net.
pub(super) fn corner_junctions(m: &Model) -> (usize, usize, usize, Vec<CornerRow>, usize) {
    let tracks = m.tracks.values().flatten().map(|&i| m.track(i));
    let (mut bends, mut right, mut acute_here, mut with_short) = (0usize, 0usize, 0usize, 0usize);
    let mut right_rows: Vec<(String, String, Pt, Vec<String>)> = Vec::new();
    for j in dfm::junctions(tracks) {
        if j.short_tracks > 0 {
            with_short += 1;
        }
        if j.ends.len() != 2 {
            continue;
        }
        let in_copper = j.leg_points().all(|p| {
            m.pads.get(&j.net).into_iter().flatten().any(|&pi| {
                let pad = m.pad(pi).1;
                pad.on_layer(&j.layer)
                    && pad
                        .shape
                        .distance(&konnect_sexp::board_geometry::Shape::disc(p, 0.0))
                        <= 0.01
            }) || m
                .vias
                .get(&j.net)
                .into_iter()
                .flatten()
                .any(|&vi| dist(m.track(vi).at, p) <= m.track(vi).shape.radius)
        });
        if in_copper {
            continue;
        }
        let angle = j.sharpest();
        if angle < 179.0 {
            bends += 1;
            if dfm::is_acute(angle) {
                acute_here += 1;
            } else if dfm::is_right_angle(angle) {
                right += 1;
                right_rows.push((
                    j.net.clone(),
                    j.layer.clone(),
                    j.at,
                    j.ends
                        .iter()
                        .map(|e| e.1.clone().unwrap_or_default())
                        .collect(),
                ));
            }
        }
    }
    (bends, right, acute_here, right_rows, with_short)
}

pub(super) fn corners(m: &Model) -> CheckOut {
    let id = "5.3";
    let name = "acute_and_right_angle_corners";
    if m.tracks.is_empty() {
        return CheckOut::not_run(id, name, "no tracks");
    }
    // Acute corners: PR 11's acid-trap check, as is.
    let limits = dfm::fab_limits(None, None);
    let report = dfm::check_board(m.geo, &limits);
    let acute: Vec<&dfm::DfmIssue> = report
        .issues
        .iter()
        .filter(|i| i.issue.starts_with("Acid trap"))
        .collect();

    let (bends, right, acute_here, mut right_rows, with_short) = corner_junctions(m);
    let short_tracks = report
        .checks
        .get("acid_trap")
        .and_then(|c| c.short_tracks)
        .unwrap_or(0);
    let mut out = CheckOut::checked(id, name);
    for i in &acute {
        // The tracks of the net that end at the corner (the acid-trap check gives only a position).
        let at = (
            i.at["x"].as_f64().unwrap_or(0.0),
            i.at["y"].as_f64().unwrap_or(0.0),
        );
        let (net, layer) = (
            i.at["net"].as_str().unwrap_or(""),
            i.at["layer"].as_str().unwrap_or(""),
        );
        let uuids: Vec<String> = m
            .tracks
            .get(net)
            .into_iter()
            .flatten()
            .map(|&ti| m.track(ti))
            .filter(|t| {
                t.layers.first().map(String::as_str) == Some(layer)
                    && [t.shape.pts.first(), t.shape.pts.last()]
                        .into_iter()
                        .flatten()
                        .any(|e| dist(*e, at) <= 0.05 + t.shape.radius)
            })
            .filter_map(|t| t.uuid.clone())
            .take(8)
            .collect();
        out.findings.push(Finding {
            severity: "warning",
            item: id,
            subject: i.at["net"].as_str().unwrap_or("").to_string(),
            measured: json!({ "at": i.at, "reused": "dfm acid_trap" }),
            threshold: json!({ "minimum_interior_angle_deg": 90 }),
            issue: i.issue.clone(),
            fix: i.fix.clone(),
            uuids,
            at: Some(at),
        });
    }
    right_rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    let signals_right: Vec<_> = right_rows
        .iter()
        .filter(|r| !m.is_ground(&r.0) && !m.is_power(&r.0))
        .collect();
    for r in signals_right.iter().take(10) {
        out.findings.push(Finding {
            severity: "info",
            item: id,
            subject: r.0.clone(),
            measured: json!({ "angle_deg": 90, "layer": r.1 }),
            threshold: json!({ "preferred_interior_angle_deg": 135 }),
            issue: format!(
                "Right-angle bend on net '{}' ({}) at ({:.2}, {:.2})",
                r.0, r.1, r.2 .0, r.2 .1
            ),
            fix: "Use two 45° bends".into(),
            uuids: r.3.clone(),
            at: Some(r.2),
        });
    }
    let total = bends.max(acute.len());
    let penalty = acute.len() as f64 + 0.3 * right as f64;
    out.score = Some(if total == 0 {
        1.0
    } else {
        (1.0 - penalty / total as f64).clamp(0.0, 1.0)
    });
    out.metrics = json!({
        "bends": bends,
        "acute_corners": acute.len(),
        "right_angle_corners": right,
        "acute_found_by_this_scan": acute_here,
        "short_tracks": short_tracks,
        "junctions_with_short_tracks": with_short,
        "compare": { "acute_corners": acute.len(), "right_angle_corners": right, "bends": bends },
        "method": "acute corners are audit_manufacturing's acid-trap check, below 89°; a track shorter than 10 µm is read as part of the junction it touches and counted in short_tracks; right angles are two track ends of one net meeting at 89–91° away from its pads and vias; arcs and 135° bends are fine",
    });
    out
}

// ─── 5.4 differential pairs ──────────────────────────────────────────────────

fn diff_pairs(m: &Model, refs: &RefMap, stubs: &HashMap<String, usize>) -> CheckOut {
    let id = "5.4";
    let name = "differential_pairs";
    let pairs: Vec<&(String, String, String)> = m
        .roles
        .pairs
        .iter()
        .filter(|(p, n, _)| m.tracks.contains_key(p) && m.tracks.contains_key(n))
        .collect();
    if pairs.is_empty() {
        return CheckOut::not_run(id, name, "no differential pair with routed tracks (record pairs in the design-intent interfaces)");
    }
    let (spread_limit, frac_limit) = (
        m.params.get("diff_max_gap_spread"),
        m.params.get("diff_min_coupled_fraction"),
    );
    let coupled_gap = 1.0;
    let mut out = CheckOut::checked(id, name);
    let mut scores = Vec::new();
    let mut rows = Vec::new();
    for (pn, nn, source) in pairs {
        let neg_segs: Vec<(Pt, Pt, f64, &str)> = m.tracks[nn]
            .iter()
            .flat_map(|&i| {
                let t = m.track(i);
                let layer = t.layers.first().map(String::as_str).unwrap_or("");
                t.shape
                    .pts
                    .windows(2)
                    .map(move |w| (w[0], w[1], t.shape.radius, layer))
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut gaps: Vec<f64> = Vec::new();
        let (mut len_p, mut coupled_len) = (0.0, 0.0);
        let mut loose_uuid = Vec::new();
        let mut loose_at = None;
        for &i in &m.tracks[pn] {
            let t = m.track(i);
            let layer = t.layers.first().map(String::as_str).unwrap_or("");
            for w in t.shape.pts.windows(2) {
                let l = dist(w[0], w[1]);
                let n = ((l / 0.25).ceil() as usize).max(1);
                for k in 0..n {
                    let f = (k as f64 + 0.5) / n as f64;
                    let p = (
                        w[0].0 + (w[1].0 - w[0].0) * f,
                        w[0].1 + (w[1].1 - w[0].1) * f,
                    );
                    let per = l / n as f64;
                    len_p += per;
                    let best = neg_segs
                        .iter()
                        .filter(|s| s.3 == layer)
                        .map(|s| point_segment_distance(p, s.0, s.1) - s.2 - t.shape.radius)
                        .fold(f64::INFINITY, f64::min);
                    if best <= coupled_gap {
                        coupled_len += per;
                        gaps.push(best.max(0.0));
                    } else if loose_at.is_none() {
                        loose_at = Some(p);
                        loose_uuid.extend(t.uuid.clone());
                    }
                }
            }
        }
        gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let q = |f: f64| {
            gaps.get(((gaps.len() as f64 - 1.0) * f).round() as usize)
                .copied()
        };
        let (g5, g95) = (q(0.05), q(0.95));
        let spread = match (g5, g95) {
            (Some(lo), Some(hi)) => Some(hi.max(0.01) / lo.max(0.01)),
            _ => None,
        };
        let frac = if len_p > 0.0 {
            coupled_len / len_p
        } else {
            0.0
        };
        let (lp, ln): (f64, f64) = (
            m.tracks[pn]
                .iter()
                .map(|&i| path_len(&m.track(i).shape.pts))
                .sum(),
            m.tracks[nn]
                .iter()
                .map(|&i| path_len(&m.track(i).shape.pts))
                .sum(),
        );
        let (vp, vn) = (
            m.vias.get(pn).map_or(0, Vec::len),
            m.vias.get(nn).map_or(0, Vec::len),
        );
        // Reference: the plane each net sits over, per layer.
        let ref_of = |net: &String| -> BTreeMap<usize, (usize, Option<String>)> {
            refs.iter()
                .filter(|((n, _), _)| n == net)
                .map(|((_, l), c)| (*l, (c.layer, c.net.clone())))
                .collect()
        };
        let (rp, rn) = (ref_of(pn), ref_of(nn));
        let ref_differs = rp.iter().any(|(l, r)| rn.get(l).is_some_and(|o| o != r));
        let stub_total = stubs.get(pn).copied().unwrap_or(0) + stubs.get(nn).copied().unwrap_or(0);
        let mut s = (frac / frac_limit.max(0.01)).min(1.0);
        if let Some(sp) = spread {
            s *= excess_score(sp, spread_limit);
        }
        if vp != vn {
            s *= 0.7;
        }
        if ref_differs {
            s *= 0.7;
        }
        if stub_total > 0 {
            s *= 0.7;
        }
        scores.push(s);
        let mut issues = Vec::new();
        if frac < frac_limit {
            issues.push(format!(
                "only {:.0}% of the pair runs coupled within {coupled_gap} mm",
                frac * 100.0
            ));
        }
        if spread.is_some_and(|sp| sp > spread_limit) {
            issues.push(format!(
                "gap varies {:.1}× between its 5th and 95th percentile (limit {spread_limit}×)",
                spread.unwrap_or(0.0)
            ));
        }
        if vp != vn {
            issues.push(format!("{vp} via(s) on {pn} against {vn} on {nn}"));
        }
        if ref_differs {
            issues.push("the two nets sit over different reference planes".to_string());
        }
        if stub_total > 0 {
            issues.push(format!("{stub_total} dangling end(s)"));
        }
        if !issues.is_empty() {
            out.findings.push(Finding {
                severity: "warning",
                item: id,
                subject: format!("{pn} / {nn}"),
                measured: json!({ "coupled_fraction": r3(frac), "gap_spread": spread.map(r2), "median_gap_mm": q(0.5).map(r3), "vias": [vp, vn], "reference_differs": ref_differs, "dangling_ends": stub_total }),
                threshold: json!({ "diff_min_coupled_fraction": frac_limit, "diff_max_gap_spread": spread_limit }),
                issue: format!("Pair {pn}/{nn} ({source}): {}", issues.join("; ")),
                fix: "Route the pair together at constant spacing over one reference plane, with matched vias and no stubs".into(),
                uuids: loose_uuid.iter().take(8).cloned().collect(),
                at: loose_at,
            });
        }
        if (lp - ln).abs() > 0.5 {
            out.findings.push(Finding {
                severity: "info",
                item: id,
                subject: format!("{pn} / {nn} length"),
                measured: json!({ "length_mm": [r2(lp), r2(ln)], "skew_mm": r2((lp - ln).abs()) }),
                threshold: json!({ "needed_when": "the interface's timing says so" }),
                issue: format!("Pair {pn}/{nn} differs by {:.2} mm in length; whether that matters needs the interface's skew limit", (lp - ln).abs()),
                fix: "Match only if the interface requires it".into(),
                uuids: [pn, nn]
                    .iter()
                    .flat_map(|n| m.tracks[*n].iter().take(4))
                    .filter_map(|&i| m.track(i).uuid.clone())
                    .collect(),
                at: None,
            });
        }
        rows.push(json!({
            "pair": [pn, nn], "source": source, "coupled_fraction": r3(frac),
            "gap_mm": { "p5": g5.map(r3), "median": q(0.5).map(r3), "p95": g95.map(r3) },
            "gap_spread": spread.map(r2), "length_mm": [r2(lp), r2(ln)], "vias": [vp, vn], "reference_differs": ref_differs,
        }));
    }
    out.score = Some(scores.iter().sum::<f64>() / scores.len() as f64);
    out.needs_human.push(("5.5", "Differential impedance (e.g. 90 Ω for USB): needs the fab stackup's dielectric constant and heights and the interface's target".into()));
    out.needs_human.push((
        "5.8",
        "Length matching within a pair or group: needs the interface's skew limit".into(),
    ));
    out.metrics = json!({
        "pairs": rows,
        "compare": { "pairs_with_issues": out.findings.iter().filter(|f| f.severity == "warning").count() },
        "method": "positive net sampled every 0.25 mm; the partner is 'coupled' where a segment on the same layer is within 1 mm edge to edge; gap spread is the 95th over the 5th percentile of the coupled gaps",
    });
    out
}

#[allow(dead_code)]
fn _kept(a: Pt, b: Pt, c: Pt, d: Pt) -> f64 {
    segment_distance(a, b, c, d)
}
