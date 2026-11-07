//! Read-only placement proxies over declared pad nets. The Euclidean MST is
//! a straight-airwire estimate, not routed copper or an unrouted-net report.

use konnect_sexp::board::PcbConnectivityIndex;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const MAX_PAD_SITES: usize = 20_000;
const MAX_DISTANCE_CHECKS: usize = 5_000_000;
const MAX_PAIR_CHECKS: usize = 5_000_000;
const MAX_SAMPLES: usize = 100;
const SAMPLE_BYTES: usize = 8_000;

#[derive(Clone, Copy)]
struct Limits {
    sites: usize,
    distances: usize,
    pairs: usize,
    samples: usize,
    sample_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            sites: MAX_PAD_SITES,
            distances: MAX_DISTANCE_CHECKS,
            pairs: MAX_PAIR_CHECKS,
            samples: MAX_SAMPLES,
            sample_bytes: SAMPLE_BYTES,
        }
    }
}

#[derive(Clone, Debug)]
struct Site<'a> {
    reference: &'a str,
    pad: &'a str,
    at: (f64, f64),
}

#[derive(Clone, Debug)]
struct Edge<'a> {
    net: &'a str,
    a: Site<'a>,
    b: Site<'a>,
}

pub(super) struct RoutingObjective {
    pub(super) complete: bool,
    pub(super) wirelength_mm: Option<f64>,
    pub(super) crossings: Option<usize>,
    pub(super) wire_deduction: Option<i64>,
    pub(super) crossing_deduction: Option<i64>,
    pub(super) edge_count: usize,
    pub(super) eligible_nets: usize,
    max_net_references: usize,
    board_diagonal_mm: Option<f64>,
    named_nets: usize,
    pad_sites: usize,
    pads_without_net: usize,
    skipped_pads: usize,
    filtered_fanout: usize,
    filtered_internal: usize,
    filtered_fewer_sites: usize,
    reasons: Vec<String>,
    net_samples: Vec<Value>,
    crossing_samples: Vec<Value>,
    details_truncated: bool,
    limits: Limits,
}

impl RoutingObjective {
    fn unavailable(&mut self, reason: impl Into<String>) {
        self.complete = false;
        self.wirelength_mm = None;
        self.crossings = None;
        self.wire_deduction = None;
        self.crossing_deduction = None;
        self.reasons.push(reason.into());
    }

    pub(super) fn to_json(&self) -> Value {
        json!({
            "status": if self.complete { "complete" } else { "incomplete" },
            "method": "euclidean_pad_site_mst",
            "proxy": true,
            "scope": "declared pad nets on two or more distinct references within the fanout limit",
            "existing_copper_considered": false,
            "layers_considered": false,
            "wirelength_mm": self.wirelength_mm,
            "proper_crossings": self.crossings,
            "wire_deduction": self.wire_deduction,
            "crossing_deduction": self.crossing_deduction,
            "max_net_references": self.max_net_references,
            "board_diagonal_mm": self.board_diagonal_mm,
            "named_nets": self.named_nets,
            "pad_sites": self.pad_sites,
            "pads_without_net": self.pads_without_net,
            "skipped_pads": self.skipped_pads,
            "eligible_nets": self.eligible_nets,
            "airwire_edges": self.complete.then_some(self.edge_count),
            "edges_expected_from_known_sites": self.edge_count,
            "counts_scope": "Indexed pad sites; source gaps can change net eligibility",
            "filtered": {
                "above_reference_limit": self.filtered_fanout,
                "single_reference": self.filtered_internal,
                "fewer_than_two_sites": self.filtered_fewer_sites
            },
            "reasons": self.reasons,
            "net_samples": self.net_samples,
            "crossing_samples": self.crossing_samples,
            "details_truncated": self.details_truncated,
            "work_limits": {"pad_sites":self.limits.sites,"distance_checks":self.limits.distances,"edge_pairs":self.limits.pairs},
            "interpretation": "Projected straight-airwire proxies; not routed length, actual airwire connectivity, shorts or DRC. Smaller is better for the same topology and policy."
        })
    }
}

pub(super) fn assess(
    index: &PcbConnectivityIndex,
    max_net_references: usize,
    board_diagonal_mm: Option<f64>,
    ambiguous_references: &[String],
) -> RoutingObjective {
    assess_with_limits(
        index,
        max_net_references,
        board_diagonal_mm,
        ambiguous_references,
        Limits::default(),
    )
}

fn assess_with_limits(
    index: &PcbConnectivityIndex,
    max_net_references: usize,
    board_diagonal_mm: Option<f64>,
    ambiguous_references: &[String],
    limits: Limits,
) -> RoutingObjective {
    let nets = index.nets();
    let mut result = RoutingObjective {
        complete: true,
        wirelength_mm: None,
        crossings: None,
        wire_deduction: None,
        crossing_deduction: None,
        edge_count: 0,
        eligible_nets: 0,
        max_net_references,
        board_diagonal_mm: board_diagonal_mm.filter(|d| d.is_finite() && *d > 0.0),
        named_nets: nets.len(),
        pad_sites: 0,
        pads_without_net: index.pads_without_net().len(),
        skipped_pads: index.skipped_pads(),
        filtered_fanout: 0,
        filtered_internal: 0,
        filtered_fewer_sites: 0,
        reasons: Vec::new(),
        net_samples: Vec::new(),
        crossing_samples: Vec::new(),
        details_truncated: false,
        limits,
    };
    let mut groups = Vec::new();
    let mut invalid_sites = 0usize;
    for pad in index.pads_without_net() {
        if !valid_site(&pad.reference, pad.at) {
            invalid_sites += 1;
        }
    }
    for net in nets {
        let pads = index.pads_of_net(net);
        result.pad_sites += pads.len();
        invalid_sites += pads
            .iter()
            .filter(|p| !valid_site(&p.reference, p.at))
            .count();
        let references: BTreeSet<_> = pads.iter().map(|p| p.reference.as_str()).collect();
        let filtered = if pads.len() < 2 {
            result.filtered_fewer_sites += 1;
            Some("fewer_than_two_sites")
        } else if references.len() < 2 {
            result.filtered_internal += 1;
            Some("single_reference")
        } else if references.len() > max_net_references {
            result.filtered_fanout += 1;
            Some("above_reference_limit")
        } else {
            None
        };
        if filtered.is_none() {
            result.eligible_nets += 1;
            result.edge_count += pads.len() - 1;
        }
        groups.push((net, pads, references.len(), filtered));
    }
    if result.skipped_pads != 0 {
        result.unavailable(format!("{} pads could not be indexed", result.skipped_pads));
    }
    if invalid_sites != 0 {
        result.unavailable(format!(
            "{invalid_sites} pad sites have nonfinite coordinates or empty references"
        ));
    }
    if !ambiguous_references.is_empty() {
        result.unavailable(format!(
            "{} ambiguous-reference/source coverage issues",
            ambiguous_references.len()
        ));
    }
    if result.pad_sites.saturating_add(result.pads_without_net) > limits.sites {
        result.unavailable(format!("pad-site work limit exceeded ({})", limits.sites));
    }
    if !result.complete {
        return result;
    }

    let mut distance_budget = limits.distances;
    let mut pair_budget = limits.pairs;
    let mut samples = Samples::new(limits);
    let mut edges = Vec::with_capacity(result.edge_count);
    let mut length = 0.0;
    for (net, pads, references, filtered) in groups {
        if let Some(reason) = filtered {
            samples.push_net(
                json!({"net":net,"references":references,"pad_sites":pads.len(),"filtered":reason}),
            );
            continue;
        }
        let sites: Vec<_> = pads
            .iter()
            .map(|p| Site {
                reference: &p.reference,
                pad: &p.pad_number,
                at: p.at,
            })
            .collect();
        let (net_length, net_edges) = match mst(net, &sites, &mut distance_budget) {
            Ok(value) => value,
            Err(reason) => {
                result.unavailable(reason);
                return result;
            }
        };
        length += net_length;
        if !length.is_finite() {
            result.unavailable("wirelength accumulation overflowed");
            return result;
        }
        samples.push_net(
            json!({"net":net,"references":references,"pad_sites":pads.len(),"mst_mm":net_length}),
        );
        edges.extend(net_edges);
    }
    let crossings = match count_crossings(&edges, &mut pair_budget, &mut samples) {
        Ok(value) => value,
        Err(reason) => {
            result.unavailable(reason);
            return result;
        }
    };
    result.wirelength_mm = Some(length);
    result.crossings = Some(crossings);
    result.wire_deduction = wire_points(length, result.edge_count, result.board_diagonal_mm);
    result.crossing_deduction = Some(crossing_points(crossings));
    if result.wire_deduction.is_none() {
        result
            .reasons
            .push("wire deduction unavailable without a positive finite board diagonal".into());
    }
    result.net_samples = samples.nets;
    result.crossing_samples = samples.crossings;
    result.details_truncated = samples.truncated;
    result
}

fn wire_points(length: f64, edges: usize, diagonal: Option<f64>) -> Option<i64> {
    if edges == 0 {
        Some(0)
    } else {
        diagonal.map(|diagonal| {
            // Avoid overflowing edges * diagonal. A positive ratio that
            // overflows already saturates the documented 20-point cap.
            ((length / diagonal / edges as f64).min(1.0) * 20.0).ceil() as i64
        })
    }
}

fn crossing_points(crossings: usize) -> i64 {
    crossings.min(10) as i64 * 2
}

fn count_crossings(
    edges: &[Edge<'_>],
    budget: &mut usize,
    samples: &mut Samples,
) -> Result<usize, &'static str> {
    let mut crossings = 0;
    for (i, a) in edges.iter().enumerate() {
        for b in &edges[i + 1..] {
            if *budget == 0 {
                return Err("edge-pair work limit exceeded");
            }
            *budget -= 1;
            if a.net == b.net {
                continue;
            }
            if proper_crossing(a.a.at, a.b.at, b.a.at, b.b.at)? {
                crossings += 1;
                samples.push_crossing(json!({"net_a":a.net,"net_b":b.net,
                    "a":{"from":site_json(&a.a),"to":site_json(&a.b)},
                    "b":{"from":site_json(&b.a),"to":site_json(&b.b)}}));
            }
        }
    }
    Ok(crossings)
}

fn valid_site(reference: &str, at: (f64, f64)) -> bool {
    !reference.trim().is_empty() && at.0.is_finite() && at.1.is_finite()
}

fn site_json(site: &Site<'_>) -> Value {
    json!({"reference":site.reference,"pad":site.pad,"x":site.at.0,"y":site.at.1})
}

fn mst<'a>(
    net: &'a str,
    sites: &[Site<'a>],
    budget: &mut usize,
) -> Result<(f64, Vec<Edge<'a>>), &'static str> {
    let mut sites = sites.to_vec();
    sites.sort_by(|a, b| {
        a.at.0
            .total_cmp(&b.at.0)
            .then_with(|| a.at.1.total_cmp(&b.at.1))
            .then_with(|| a.reference.cmp(b.reference))
            .then_with(|| a.pad.cmp(b.pad))
    });
    let mut used = vec![false; sites.len()];
    let mut distances = vec![f64::INFINITY; sites.len()];
    let mut parents: Vec<Option<usize>> = vec![None; sites.len()];
    if let Some(first) = distances.first_mut() {
        *first = 0.0;
    }
    let mut total = 0.0;
    let mut edges = Vec::with_capacity(sites.len().saturating_sub(1));
    for _ in 0..sites.len() {
        let selected = (0..sites.len())
            .filter(|i| !used[*i])
            .min_by(|a, b| {
                distances[*a]
                    .total_cmp(&distances[*b])
                    .then_with(|| a.cmp(b))
            })
            .ok_or("MST could not select a pad site")?;
        used[selected] = true;
        if let Some(parent) = parents[selected] {
            total += distances[selected];
            if !total.is_finite() {
                return Err("MST wirelength overflowed");
            }
            edges.push(Edge {
                net,
                a: sites[parent].clone(),
                b: sites[selected].clone(),
            });
        }
        for candidate in 0..sites.len() {
            if used[candidate] {
                continue;
            }
            if *budget == 0 {
                return Err("MST distance work limit exceeded");
            }
            *budget -= 1;
            let distance = (sites[selected].at.0 - sites[candidate].at.0)
                .hypot(sites[selected].at.1 - sites[candidate].at.1);
            if !distance.is_finite() {
                return Err("MST distance is nonfinite");
            }
            if distance < distances[candidate]
                || (distance == distances[candidate]
                    && parents[candidate].is_none_or(|p| selected < p))
            {
                distances[candidate] = distance;
                parents[candidate] = Some(selected);
            }
        }
    }
    Ok((total, edges))
}

fn proper_crossing(
    a: (f64, f64),
    b: (f64, f64),
    c: (f64, f64),
    d: (f64, f64),
) -> Result<bool, &'static str> {
    if a.0.min(b.0) > c.0.max(d.0)
        || c.0.min(d.0) > a.0.max(b.0)
        || a.1.min(b.1) > c.1.max(d.1)
        || c.1.min(d.1) > a.1.max(b.1)
    {
        return Ok(false);
    }
    let orient = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
        (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
    };
    let o = [
        orient(a, b, c),
        orient(a, b, d),
        orient(c, d, a),
        orient(c, d, b),
    ];
    if o.iter().any(|v| !v.is_finite()) {
        return Err("crossing orientation is nonfinite");
    }
    let opposite = |x: f64, y: f64| (x > 0.0 && y < 0.0) || (x < 0.0 && y > 0.0);
    Ok(opposite(o[0], o[1]) && opposite(o[2], o[3]))
}

struct Samples {
    nets: Vec<Value>,
    crossings: Vec<Value>,
    bytes: usize,
    max_items: usize,
    max_bytes: usize,
    truncated: bool,
}

impl Samples {
    fn new(limits: Limits) -> Self {
        Self {
            nets: Vec::new(),
            crossings: Vec::new(),
            bytes: 0,
            max_items: limits.samples,
            max_bytes: limits.sample_bytes,
            truncated: false,
        }
    }
    fn accepts(&mut self, value: &Value) -> bool {
        let bytes = serde_json::to_vec(value).map_or(usize::MAX, |v| v.len());
        if self.nets.len() + self.crossings.len() >= self.max_items
            || bytes > self.max_bytes.saturating_sub(self.bytes)
        {
            self.truncated = true;
            false
        } else {
            self.bytes += bytes;
            true
        }
    }
    fn push_net(&mut self, value: Value) {
        if self.accepts(&value) {
            self.nets.push(value);
        }
    }
    fn push_crossing(&mut self, value: Value) {
        if self.accepts(&value) {
            self.crossings.push(value);
        }
    }
}

#[cfg(test)]
#[path = "objective_tests.rs"]
mod tests;
