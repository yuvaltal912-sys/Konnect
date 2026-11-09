//! Checklist §3: reference planes, stitching vias and ground connectivity.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use konnect_sexp::planar::Pt;
use serde_json::json;

use super::grid::Cover;
use super::routing;
use super::{dist, path_len, r2, r3, CheckOut, Finding, Model};

pub(super) fn run(m: &Model) -> (Vec<CheckOut>, RefMap) {
    let (reference, refs) = reference_continuity(m);
    let stitching = stitching_vias(m, &refs);
    (vec![reference, stitching, ground_connectivity(m)], refs)
}

/// What lies on the reference layer under one sample of a track.
#[derive(Clone, Debug, PartialEq)]
enum Under {
    /// The plane the track is referenced to (a ground or a power net).
    Reference(String),
    Other(String),
    Void,
}

/// For a (net, track layer): the adjacent layer judged to be its reference,
/// and the ground or power net that plane carries.
#[derive(Clone, Debug)]
pub(super) struct RefChoice {
    pub layer: usize,
    pub net: Option<String>,
}

pub(super) type RefMap = HashMap<(String, usize), RefChoice>;

struct Run {
    kind: &'static str,
    len: f64,
    from: Pt,
    uuid: Option<String>,
    other_net: Option<String>,
}

fn reference_continuity(m: &Model) -> (CheckOut, RefMap) {
    let id = "3.1";
    let name = "reference_plane_continuity";
    let mut refs: RefMap = HashMap::new();
    let nets: Vec<&String> = m
        .roles
        .critical
        .keys()
        .filter(|n| m.tracks.contains_key(*n) && !m.is_ground(n) && !m.is_power(n))
        .collect();
    if nets.is_empty() {
        return (
            CheckOut::not_run(
                id,
                name,
                "no critical signal net has routed tracks (record interfaces in the design intent, \
                 assign netclasses, or pass critical_nets / signal_scope)",
            ),
            refs,
        );
    }
    let sample = m.params.get("sample_mm");
    let min_cov = m.params.get("min_reference_coverage");
    let max_void = m.params.get("max_void_run_mm");

    let mut out = CheckOut::checked(id, name);
    let (mut total_len, mut total_cov, mut total_void, mut total_other) = (0.0, 0.0, 0.0, 0.0);
    let mut per_net = Vec::new();
    let mut cannot: Vec<String> = Vec::new();
    let mut worst_run = 0.0f64;
    let mut crossings = 0usize;

    for net in nets {
        let idxs = &m.tracks[net];
        // Segments grouped by layer.
        let mut by_layer: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for &i in idxs {
            if let Some(l) = m.track(i).layers.first().and_then(|l| m.layer_ix(l)) {
                by_layer.entry(l).or_default().push(i);
            }
        }
        let mut net_len = 0.0;
        let mut net_cov = 0.0;
        let mut runs: Vec<Run> = Vec::new();
        let mut layers_info = Vec::new();
        let mut net_unjudged = false;
        for (&l, segs) in &by_layer {
            let adj: Vec<usize> = [l.checked_sub(1), Some(l + 1)]
                .into_iter()
                .flatten()
                .filter(|a| *a < m.layers.len())
                .collect();
            let samples = |seg: usize| -> Vec<Pt> {
                let pts = &m.track(seg).shape.pts;
                let mut out = Vec::new();
                for w in pts.windows(2) {
                    let len = dist(w[0], w[1]);
                    let n = ((len / sample).ceil() as usize).max(1);
                    for k in 0..n {
                        let t = (k as f64 + 0.5) / n as f64;
                        out.push((
                            w[0].0 + (w[1].0 - w[0].0) * t,
                            w[0].1 + (w[1].1 - w[0].1) * t,
                        ));
                    }
                }
                out
            };
            // Pick the adjacent layer with the most plane copper (ground or power: the checklist
            // asks for an uninterrupted adjacent reference plane, not for ground only) under
            // the net's tracks here; a tie goes to the layer with more ground.
            let mut best: Option<(usize, usize, usize)> = None; // (layer, hits, ground hits)
            let all_samples: Vec<(usize, Vec<Pt>)> =
                segs.iter().map(|&s| (s, samples(s))).collect();
            for &a in &adj {
                let (mut hits, mut ground_hits) = (0usize, 0usize);
                for p in all_samples.iter().flat_map(|(_, pts)| pts) {
                    let net_here = m
                        .fill_at(&m.layers[a], *p)
                        .and_then(|f| m.fills[f].net.as_deref());
                    if let Some(n) = net_here.filter(|n| m.is_ground(n) || m.is_power(n)) {
                        hits += 1;
                        ground_hits += usize::from(m.is_ground(n));
                    }
                }
                if best.is_none_or(|(_, h, g)| (hits, ground_hits) > (h, g)) {
                    best = Some((a, hits, ground_hits));
                }
            }
            let Some((ref_layer, hits, _)) = best else {
                continue;
            };
            // Zones that exist but carry no saved fill: the answer is unknown, not "void".
            let unfilled_here = m.unfilled.iter().find(|z| {
                z.layers
                    .iter()
                    .any(|zl| adj.iter().any(|a| &m.layers[*a] == zl))
            });
            if hits == 0 {
                if let Some(z) = unfilled_here {
                    cannot.push(format!(
                        "net '{net}': zone {}{} on {} has no saved fill (refill zones and save the board)",
                        z.uuid.as_deref().unwrap_or("?"),
                        z.net.as_deref().map(|n| format!(" ('{n}')")).unwrap_or_default(),
                        z.layers.join("/")
                    ));
                    net_unjudged = true;
                    continue;
                }
            }
            let ref_name = m.layers[ref_layer].clone();
            // The plane the net is referenced to on this layer: the ground or power net with
            // most samples. Every ground counts as the same reference (AGND beside GND is one
            // plane for this purpose); a different power net, or a signal net, is a split.
            let mut by_net: BTreeMap<&str, usize> = BTreeMap::new();
            for p in all_samples.iter().flat_map(|(_, pts)| pts) {
                if let Some(n) = m
                    .fill_at(&ref_name, *p)
                    .and_then(|f| m.fills[f].net.as_deref())
                    .filter(|n| m.is_ground(n) || m.is_power(n))
                {
                    *by_net.entry(n).or_default() += 1;
                }
            }
            let reference_net: Option<String> = by_net
                .iter()
                .max_by(|a, b| {
                    a.1.cmp(b.1)
                        .then_with(|| m.is_ground(a.0).cmp(&m.is_ground(b.0)))
                })
                .map(|(n, _)| n.to_string());
            let is_reference = |n: &str| match &reference_net {
                Some(r) if m.is_ground(r) => m.is_ground(n),
                Some(r) => n == r,
                None => false,
            };
            let mut layer_len = 0.0;
            let mut layer_cov = 0.0;
            for (seg, pts) in &all_samples {
                let e = m.track(*seg);
                let seg_len = path_len(&e.shape.pts);
                if pts.is_empty() {
                    continue;
                }
                let per = seg_len / pts.len() as f64;
                let mut current: Option<(Under, f64, Pt)> = None;
                let flush = |cur: &mut Option<(Under, f64, Pt)>, runs: &mut Vec<Run>| {
                    if let Some((u, len, from)) = cur.take() {
                        match u {
                            Under::Reference(_) => {}
                            Under::Other(n) => runs.push(Run {
                                kind: "other_net_plane",
                                len,
                                from,
                                uuid: e.uuid.clone(),
                                other_net: Some(n),
                            }),
                            Under::Void => runs.push(Run {
                                kind: "void",
                                len,
                                from,
                                uuid: e.uuid.clone(),
                                other_net: None,
                            }),
                        }
                    }
                };
                for p in pts {
                    let under = match m.fill_at(&ref_name, *p) {
                        Some(f) => match m.fills[f].net.clone() {
                            Some(n) if is_reference(&n) => Under::Reference(n),
                            Some(n) => Under::Other(n),
                            None => Under::Void,
                        },
                        None => Under::Void,
                    };
                    if matches!(under, Under::Reference(_)) {
                        layer_cov += per;
                    }
                    layer_len += per;
                    match &mut current {
                        Some((u, len, _))
                            if std::mem::discriminant(u) == std::mem::discriminant(&under)
                                && (!matches!(under, Under::Other(_)) || *u == under) =>
                        {
                            *len += per
                        }
                        _ => {
                            flush(&mut current, &mut runs);
                            current = Some((under, per, *p));
                        }
                    }
                }
                flush(&mut current, &mut runs);
            }
            refs.insert(
                (net.clone(), l),
                RefChoice {
                    layer: ref_layer,
                    net: reference_net.clone(),
                },
            );
            layers_info.push(json!({
                "track_layer": m.layers[l],
                "reference_layer": ref_name,
                "reference_net": reference_net,
                "reference_kind": reference_net.as_deref().map(|n| if m.is_ground(n) { "ground" } else { "power" }),
                "length_mm": r2(layer_len),
                "over_reference_mm": r2(layer_cov),
            }));
            net_len += layer_len;
            net_cov += layer_cov;
        }
        if net_len == 0.0 {
            continue;
        }
        let void_mm: f64 = runs
            .iter()
            .filter(|r| r.kind == "void")
            .map(|r| r.len)
            .sum();
        let other_mm: f64 = runs
            .iter()
            .filter(|r| r.kind == "other_net_plane")
            .map(|r| r.len)
            .sum();
        let longest = runs.iter().map(|r| r.len).fold(0.0, f64::max);
        let coverage = net_cov / net_len;
        total_len += net_len;
        total_cov += net_cov;
        total_void += void_mm;
        total_other += other_mm;
        worst_run = worst_run.max(longest);
        crossings += runs.len();
        let _ = net_unjudged;
        per_net.push(json!({
            "net": net,
            "source": m.roles.critical.get(net),
            "length_mm": r2(net_len),
            "coverage": r3(coverage),
            "void_mm": r2(void_mm),
            "other_net_plane_mm": r2(other_mm),
            "longest_run_mm": r2(longest),
            "runs_off_reference": runs.len(),
            "layers": layers_info,
        }));
        if coverage < min_cov || longest > max_void {
            let mut uuids: Vec<String> = Vec::new();
            for r in &runs {
                if let Some(u) = &r.uuid {
                    if !uuids.contains(u) && uuids.len() < 10 {
                        uuids.push(u.clone());
                    }
                }
            }
            let worst = runs.iter().max_by(|a, b| {
                a.len
                    .partial_cmp(&b.len)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let what = worst.map(|w| match &w.other_net {
                Some(n) => format!("{:.1} mm over the '{n}' copper, a different net from the reference plane (a split)", w.len),
                None => format!("{:.1} mm over a void or slot", w.len),
            });
            out.findings.push(Finding {
                severity: if coverage < 0.5 { "error" } else { "warning" },
                item: id,
                subject: net.to_string(),
                measured: json!({ "coverage": r3(coverage), "longest_run_mm": r2(longest), "void_mm": r2(void_mm), "other_net_plane_mm": r2(other_mm), "runs": runs.len() }),
                threshold: json!({ "min_reference_coverage": min_cov, "max_void_run_mm": max_void }),
                issue: format!(
                    "Net '{net}' runs {:.0}% of its {:.1} mm over its reference plane (solid ground or power) on the adjacent layer; longest stretch off it {}",
                    coverage * 100.0,
                    net_len,
                    what.unwrap_or_else(|| "none".into())
                ),
                fix: "Keep the net over an unbroken reference plane: move it off the split, void or slot, or close the plane under it".into(),
                uuids,
                at: worst.map(|w| w.from),
            });
        }
    }
    if per_net.is_empty() {
        let mut o = CheckOut::not_run(
            id,
            name,
            if cannot.is_empty() {
                "no critical track could be placed on a layer with a neighbour".to_string()
            } else {
                cannot.join("; ")
            },
        );
        o.metrics = json!({ "cannot_judge": cannot });
        return (o, refs);
    }
    let coverage = if total_len > 0.0 {
        total_cov / total_len
    } else {
        1.0
    };
    out.score = Some(coverage);
    if !cannot.is_empty() {
        out.status = "partial";
        out.reason = Some(format!(
            "{} net/layer pair(s) not judged: {}",
            cannot.len(),
            cannot.join("; ")
        ));
    }
    out.needs_human.push((
        "3.1",
        "Which cuts in a plane are intentional (isolation barriers, antenna keepouts): geometry cannot tell".into(),
    ));
    out.metrics = json!({
        "basis": m.roles.critical_basis,
        "nets_judged": per_net.len(),
        "critical_length_mm": r2(total_len),
        "reference_coverage": r3(coverage),
        "compare": {
            "reference_coverage": r3(coverage),
            "length_off_reference_mm": r2(total_len - total_cov),
            "void_mm": r2(total_void),
            "other_net_plane_mm": r2(total_other),
            "longest_run_off_reference_mm": r2(worst_run),
            "runs_off_reference": crossings,
        },
        "nets": per_net.into_iter().take(20).collect::<Vec<_>>(),
        "method": "tracks sampled every sample_mm; a sample counts as referenced when the saved zone fill of the net's reference plane covers it on the adjacent layer with the most plane copper (ground or power) under the net; the reference plane on that layer is the ground or power net with most samples, and a different net's copper is a split (fills are the real copper: voids, slots, antipads and splits are in them)",
    });
    (out, refs)
}

// ─── 3.2 stitching vias ──────────────────────────────────────────────────────

fn stitching_vias(m: &Model, refs: &RefMap) -> CheckOut {
    let id = "3.2";
    let name = "stitching_via_at_reference_change";
    let max_d = m.params.get("stitch_via_max_mm");
    let signal_nets: Vec<&String> = m
        .roles
        .critical
        .keys()
        .filter(|n| m.vias.contains_key(*n) && m.tracks.contains_key(*n))
        .collect();
    if signal_nets.is_empty() {
        return CheckOut::not_run(id, name, "no critical net changes layer with a via");
    }
    let mut out = CheckOut::checked(id, name);
    let (mut evaluated, mut needing, mut satisfied, mut unknown) = (0usize, 0usize, 0usize, 0usize);
    let mut worst: Option<f64> = None;
    let mut no_candidate = 0usize;
    for net in signal_nets {
        for &vi in &m.vias[net] {
            let v = m.track(vi);
            let reach = v.shape.radius + 0.3;
            // The track layers that end at this via.
            let mut layers: BTreeSet<usize> = BTreeSet::new();
            for &ti in &m.tracks[net] {
                let t = m.track(ti);
                let Some(l) = t.layers.first().and_then(|l| m.layer_ix(l)) else {
                    continue;
                };
                let (a, b) = (
                    t.shape.pts[0],
                    *t.shape.pts.last().unwrap_or(&t.shape.pts[0]),
                );
                if dist(a, v.at) <= reach + t.shape.radius
                    || dist(b, v.at) <= reach + t.shape.radius
                {
                    layers.insert(l);
                }
            }
            // A pad under the via (via-in-pad) is the other layer of the change.
            for &pi in m.pads.get(net.as_str()).into_iter().flatten() {
                let pad = m.pad(pi).1;
                if !pad.through_hole && dist(pad.at, v.at) <= pad.size.0.max(pad.size.1) / 2.0 + 0.1
                {
                    layers.extend(m.pad_layers(pad));
                }
            }
            if layers.len() < 2 {
                continue;
            }
            evaluated += 1;
            let layers: Vec<usize> = layers.into_iter().collect();
            let choices: Vec<Option<&RefChoice>> = layers
                .iter()
                .map(|l| refs.get(&(net.to_string(), *l)))
                .collect();
            if choices.iter().any(Option::is_none) {
                unknown += 1;
                continue;
            }
            let choices: Vec<&RefChoice> = choices.into_iter().flatten().collect();
            let ref_layers: BTreeSet<usize> = choices.iter().map(|c| c.layer).collect();
            if ref_layers.len() < 2 {
                continue; // the same plane on both sides: the return current just continues
            }
            let ref_nets: BTreeSet<&str> =
                choices.iter().filter_map(|c| c.net.as_deref()).collect();
            if choices.iter().any(|c| c.net.is_none()) {
                unknown += 1;
                continue;
            }
            needing += 1;
            let lo = *ref_layers.iter().next().unwrap_or(&0);
            let hi = *ref_layers.iter().next_back().unwrap_or(&0);
            // A via (or through pad) of a reference net that spans both reference layers.
            let mut best: Option<(f64, String)> = None;
            for rn in &ref_nets {
                for &gi in m.vias.get(*rn).into_iter().flatten() {
                    let g = m.track(gi);
                    if let Some((a, b)) = m.via_span(g) {
                        if a <= lo && b >= hi {
                            let d = dist(g.at, v.at);
                            if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                                best = Some((d, g.uuid.clone().unwrap_or_default()));
                            }
                        }
                    }
                }
                for &pi in m.pads.get(*rn).into_iter().flatten() {
                    let (_, pad) = m.pad(pi);
                    if pad.through_hole {
                        let d = dist(pad.at, v.at);
                        if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                            best = Some((d, pad.uuid.clone().unwrap_or_default()));
                        }
                    }
                }
            }
            let mut tied_by_cap = false;
            if ref_nets.len() > 1 {
                // Different planes: a capacitor between them is what carries the return.
                let nets: Vec<&&str> = ref_nets.iter().collect();
                tied_by_cap = m.geo.footprints.iter().any(|f| {
                    super::is_cap(m.fp_ref(f))
                        && nets
                            .iter()
                            .all(|n| f.pads.iter().any(|p| p.net.as_deref() == Some(**n)))
                        && dist(f.at, v.at) <= max_d
                });
            }
            let d = best.as_ref().map(|b| b.0);
            if (d.is_some_and(|d| d <= max_d) && ref_nets.len() == 1) || tied_by_cap {
                satisfied += 1;
                continue;
            }
            match d {
                Some(d) => worst = Some(worst.map_or(d, |w| w.max(d))),
                None => no_candidate += 1,
            }
            let layer_names: Vec<&str> = layers.iter().map(|l| m.layers[*l].as_str()).collect();
            let mut uuids = vec![v.uuid.clone().unwrap_or_default()];
            if let Some((_, u)) = &best {
                uuids.push(u.clone());
            }
            out.findings.push(Finding {
                severity: "warning",
                item: id,
                subject: format!("{net} via {}", v.uuid.as_deref().unwrap_or("?")),
                measured: json!({
                    "nearest_stitch_mm": d.map(r2),
                    "reference_layers": ref_layers.iter().map(|l| m.layers[*l].clone()).collect::<Vec<_>>(),
                    "reference_nets": ref_nets,
                    "signal_layers": layer_names,
                }),
                threshold: json!({ "stitch_via_max_mm": max_d }),
                issue: if ref_nets.len() > 1 {
                    format!("Net '{net}' changes from the {} plane to the {} plane with no capacitor between them within {max_d} mm", ref_nets.iter().next().unwrap_or(&""), ref_nets.iter().next_back().unwrap_or(&""))
                } else {
                    format!(
                        "Net '{net}' changes layer ({}) between reference planes {}; nearest {} stitching via {}",
                        layer_names.join(" → "),
                        ref_layers.iter().map(|l| m.layers[*l].as_str()).collect::<Vec<_>>().join(" / "),
                        ref_nets.iter().next().unwrap_or(&"ground"),
                        d.map(|d| format!("{d:.1} mm away (limit {max_d} mm)")).unwrap_or_else(|| "none on the board".into())
                    )
                },
                fix: "Add a ground via (or a capacitor between the two planes) within the limit of this via".into(),
                uuids,
                at: Some(v.at),
            });
        }
    }
    if evaluated == 0 {
        return CheckOut::not_run(
            id,
            name,
            "no via of a critical net joins tracks on two layers",
        );
    }
    out.score = (needing > 0)
        .then(|| satisfied as f64 / needing as f64)
        .or(Some(1.0));
    if unknown > 0 {
        out.status = "partial";
        out.reason = Some(format!(
            "{unknown} layer change(s) not judged: no reference plane was found under one side (see 3.1)"
        ));
    }
    out.metrics = json!({
        "layer_changes": evaluated,
        "needing_stitch": needing,
        "stitched": satisfied,
        "not_judged": unknown,
        "compare": { "unstitched_layer_changes": needing - satisfied },
        // Null when no unstitched change has any candidate at all; those are counted apart.
        "worst_nearest_mm": worst.map(r2),
        "worst_nearest_note": (worst.is_none() && no_candidate > 0)
            .then_some("no ground via or through-hole pad spans the reference planes for any unstitched layer change, so there is no distance to report"),
        "unstitched_with_no_candidate_on_board": no_candidate,
    });
    out
}

// ─── 3.3 ground connectivity ─────────────────────────────────────────────────

/// A pad's label (`REF.number`) and UUID.
type PadRef = (String, Option<String>);

fn ground_connectivity(m: &Model) -> CheckOut {
    let id = "3.3";
    let name = "ground_pours_connected";
    let grounds: Vec<&String> = m
        .roles
        .ground
        .keys()
        .filter(|n| {
            m.pads.contains_key(*n) || m.fills.iter().any(|f| f.net.as_deref() == Some(n.as_str()))
        })
        .collect();
    if grounds.is_empty() {
        return CheckOut::not_run(id, name, "no ground net with copper on the board");
    }
    let mut out = CheckOut::checked(id, name);
    let (mut pads_total, mut pads_main, mut islands) = (0usize, 0usize, 0usize);
    let mut per_net = Vec::new();
    let mut floating_area = 0.0;
    let mut cannot: Vec<String> = Vec::new();
    for net in grounds {
        // A zone with no saved fill adds no copper to the file: its pads would read as cut off
        // and its area as missing. The answer is unknown, not bad.
        if let Some(z) = m.unfilled_zone_of(net) {
            cannot.push(format!(
                "ground net '{net}': zone {} on {} has no saved fill (refill zones and save the board)",
                z.uuid.as_deref().unwrap_or("?"),
                z.layers.join("/")
            ));
            continue;
        }
        let Some(bb) = m.net_bbox(net, 1.0) else {
            continue;
        };
        let g = m.net_grid(net, bb, Cover::Touch);
        let (labels, count) = g.components();
        if count == 0 {
            continue;
        }
        // Pad groups come from the real shapes (a pad 0.15 mm short of its copper is cut off,
        // as for KiCad); the raster only tells which of its copper is anchored to a pad.
        let pad_list = m.pads.get(net).cloned().unwrap_or_default();
        let exact = routing::pad_groups(m, net);
        let mut with_pad: HashMap<usize, Vec<PadRef>> = HashMap::new();
        for (&pi, &grp) in pad_list.iter().zip(&exact) {
            let (fp, pad) = m.pad(pi);
            with_pad
                .entry(grp)
                .or_default()
                .push((format!("{}.{}", m.fp_ref(fp), pad.number), pad.uuid.clone()));
        }
        let mut anchored: BTreeSet<u32> = BTreeSet::new();
        let mut unplaced = 0usize;
        for &pi in &pad_list {
            let (_, pad) = m.pad(pi);
            let label = g
                .cells_under(&pad.shape, &m.pad_layers(pad))
                .first()
                .map(|(l, x, y)| labels[*l][g.idx(*x, *y)])
                .filter(|l| *l != 0);
            match label {
                Some(l) => {
                    anchored.insert(l);
                }
                None => unplaced += 1,
            }
        }
        // Floating copper: components that hold no pad. A via alone does not anchor copper:
        // it joins layers, not the copper to anything that carries current.
        let mut area: HashMap<u32, (usize, (usize, usize, usize))> = HashMap::new();
        for (l, plane) in labels.iter().enumerate() {
            for (i, lab) in plane.iter().enumerate() {
                if *lab != 0 && !anchored.contains(lab) {
                    let e = area.entry(*lab).or_insert((0, (l, i % g.nx, i / g.nx)));
                    e.0 += 1;
                }
            }
        }
        let cell_area = g.step * g.step;
        let mut floating: Vec<(f64, Pt, usize)> = area
            .values()
            .map(|(n, (l, x, y))| (*n as f64 * cell_area, g.centre(*x, *y), *l))
            .filter(|(a, _, _)| *a >= 0.5)
            .collect();
        floating.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let net_pads: usize = pad_list.len();
        let main = with_pad.values().map(Vec::len).max().unwrap_or(0);
        let groups = with_pad.len();
        pads_total += net_pads;
        pads_main += main;
        islands += floating.len();
        floating_area += floating.iter().map(|f| f.0).sum::<f64>();
        per_net.push(json!({
            "net": net,
            "pad_groups": groups,
            "pads": net_pads,
            "pads_in_largest_group": main,
            "floating_islands": floating.len(),
            "floating_area_mm2": r2(floating.iter().map(|f| f.0).sum()),
            "pads_not_on_copper": unplaced,
            "grid_step_mm": r3(g.step),
        }));
        if groups > 1 {
            let mut parts: Vec<(&usize, &Vec<PadRef>)> = with_pad.iter().collect();
            parts.sort_by_key(|p| (std::cmp::Reverse(p.1.len()), *p.0));
            let minor_pads: Vec<&PadRef> = parts.iter().skip(1).flat_map(|p| p.1.iter()).collect();
            let minor: Vec<String> = minor_pads.iter().map(|p| p.0.clone()).take(8).collect();
            out.findings.push(Finding {
                severity: "error",
                item: id,
                subject: net.to_string(),
                measured: json!({ "pad_groups": groups, "pads_in_largest": main, "pads": net_pads, "disconnected_pads": minor }),
                threshold: json!({ "pad_groups": 1 }),
                issue: format!("Ground net '{net}' is in {groups} separate copper groups; {} pad(s) are cut off from the main group (e.g. {})", net_pads - main, minor.join(", ")),
                fix: "Connect the groups with a track, pour or via, or correct the net assignment".into(),
                uuids: minor_pads.iter().filter_map(|p| p.1.clone()).take(8).collect(),
                at: None,
            });
        }
        for (a, at, l) in floating.iter().take(5) {
            out.findings.push(Finding {
                severity: "warning",
                item: id,
                subject: format!("{net} island on {}", m.layers[*l]),
                measured: json!({ "area_mm2": r2(*a) }),
                threshold: json!({ "touches_a_pad_of_the_net": true }),
                issue: format!("{a:.1} mm² of '{net}' copper on {} touches no pad of the net", m.layers[*l]),
                fix: "Connect the island to a ground pad or the ground plane, or remove it (zone island removal); a via alone does not tie it to anything".into(),
                uuids: m
                    .fills
                    .iter()
                    .filter(|f| f.net.as_deref() == Some(net.as_str()) && f.layer == m.layers[*l])
                    .filter_map(|f| f.zone_uuid.clone())
                    .take(3)
                    .collect(),
                at: Some(*at),
            });
        }
    }
    if per_net.is_empty() {
        return if cannot.is_empty() {
            CheckOut::not_run(id, name, "ground copper could not be rasterised")
        } else {
            CheckOut::not_run(id, name, cannot.join("; "))
        };
    }
    if !cannot.is_empty() {
        out.status = "partial";
        out.reason = Some(format!(
            "{} ground net(s) not judged: {}",
            cannot.len(),
            cannot.join("; ")
        ));
    }
    let pad_fraction = if pads_total > 0 {
        pads_main as f64 / pads_total as f64
    } else {
        1.0
    };
    out.score = Some(pad_fraction * (1.0 - (0.1 * islands as f64).min(0.5)));
    out.metrics = json!({
        "nets": per_net,
        "not_judged": cannot,
        "compare": {
            "ground_pads_in_main_group": r3(pad_fraction),
            "floating_ground_islands": islands,
            "floating_ground_area_mm2": r2(floating_area),
        },
        "method": "ground pads grouped by the real shapes of the net's tracks, vias, pads and saved zone fills (touching within 0.01 mm on a shared layer); floating islands from a raster of the same copper: touching cells join and a component holding no pad of the net counts, islands under 0.5 mm² are ignored; ground nets whose zone has no saved fill are not judged",
    });
    out
}
