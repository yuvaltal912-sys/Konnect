//! Pad-anchor decoupling proximity, not a proof of pin role or routed loop length.
//! The caller's board snapshot and connectivity loss counters are the evidence.

use super::{decoupling_limit_mm, ref_prefix};
use konnect_sexp::board::{FootprintCourtyard, PadSite, PcbConnectivityIndex};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

const PAIR_LIMIT: usize = 1_000_000;
const AMBIGUOUS_DETAIL_LIMIT: usize = 64;
const MATCHED_PAIR_DETAIL_LIMIT: usize = 32;
const WITNESS_BYTE_LIMIT: usize = 8 * 1024;
const WITNESS_ITEM_LIMIT: usize = 128;

/// One allowance for every newly exposed witness array in this Check. Count
/// serialized bytes so escaped or long identifiers cannot evade an item cap.
#[derive(Clone, Copy, Default)]
struct WitnessBudget {
    bytes: usize,
    items: usize,
}

impl WitnessBudget {
    fn admit(&mut self, value: &impl Serialize) -> bool {
        let Ok(encoded) = serde_json::to_vec(value) else {
            return false;
        };
        let bytes = encoded.len().saturating_add(1); // array separator
        if self.items >= WITNESS_ITEM_LIMIT || self.bytes.saturating_add(bytes) > WITNESS_BYTE_LIMIT
        {
            return false;
        }
        self.bytes += bytes;
        self.items += 1;
        true
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct PadPair {
    pub net: String,
    pub cap_pad: String,
    pub target_pad: String,
    pub cap_at: (f64, f64),
    pub target_at: (f64, f64),
    pub distance_mm: f64,
}

#[derive(Clone, Debug)]
pub(super) struct Association {
    pub cap: String,
    pub value: String,
    pub ic: String,
    pub qualifying_net: String,
    pub net_reference_count: usize,
    pub distance_mm: f64,
    pub limit_mm: f64,
    pub status: &'static str,
    pub association_basis: &'static str,
    pub cap_pad: String,
    pub ic_pad: String,
    pub cap_at: (f64, f64),
    pub ic_at: (f64, f64),
}

pub(super) type Violation = Association;

#[derive(Debug)]
pub(super) struct InterfaceFilterCap {
    pub cap: String,
    pub value: String,
    pub connector: String,
    /// Worst of the nearest matching pad pairs, one for every named cap net.
    pub distance_mm: f64,
    pub limit_mm: f64,
    pub matched_pairs: Vec<PadPair>,
    pub matched_pair_count: usize,
    pub matched_pairs_truncated: bool,
}

#[derive(Debug)]
pub(super) struct UnprovenCap {
    pub cap: String,
    pub value: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub(super) struct Coverage {
    pub status: &'static str,
    pub skipped_pads: usize,
    pub nonfinite_pads: usize,
    pub ambiguous_references: Vec<String>,
    pub ambiguous_reference_count: usize,
    pub ambiguous_references_truncated: bool,
    pub candidates: usize,
    pub evaluated: usize,
    pub unproven: usize,
    pub pair_comparisons: usize,
    pub pair_limit: usize,
    pub pair_limit_exceeded: bool,
    pub witness_bytes: usize,
    pub witness_byte_limit: usize,
    pub witness_items: usize,
    pub witness_item_limit: usize,
}

#[derive(Debug)]
pub(super) struct Check {
    pub violations: Vec<Violation>,
    pub uncoupled_caps: Vec<String>,
    pub interface_filter_caps: Vec<InterfaceFilterCap>,
    pub associations: Vec<Association>,
    pub unproven_caps: Vec<UnprovenCap>,
    pub coverage: Coverage,
}

#[derive(Clone)]
struct Candidate {
    reference: String,
    value: String,
    limit: Option<f64>,
}

struct Budget {
    used: usize,
    limit: usize,
    exhausted: bool,
}

impl Budget {
    fn distance(&mut self, a: &PadSite, b: &PadSite) -> Option<f64> {
        if self.used >= self.limit {
            self.exhausted = true;
            return None;
        }
        self.used += 1;
        let d = (a.at.0 - b.at.0).hypot(a.at.1 - b.at.1);
        d.is_finite().then_some(d)
    }
}

pub(super) fn check(
    items: &[FootprintCourtyard],
    index: &PcbConnectivityIndex,
    values: &HashMap<String, String>,
    max_net_references: usize,
    ambiguous_references: &[String],
) -> Check {
    check_with_budget(
        items,
        index,
        values,
        max_net_references,
        ambiguous_references,
        PAIR_LIMIT,
    )
}

fn check_with_budget(
    items: &[FootprintCourtyard],
    index: &PcbConnectivityIndex,
    values: &HashMap<String, String>,
    max_net_references: usize,
    ambiguous_references: &[String],
    pair_limit: usize,
) -> Check {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for item in items {
        if let Some(reference) = item.reference.as_deref() {
            *counts.entry(reference).or_default() += 1;
        }
    }
    let ambiguous: BTreeSet<String> = ambiguous_references
        .iter()
        .cloned()
        .chain(
            counts
                .iter()
                .filter(|(_, count)| **count > 1)
                .map(|(r, _)| (*r).to_owned()),
        )
        .collect();
    let candidates: Vec<Candidate> = counts
        .keys()
        .filter(|r| ref_prefix(r) == "C")
        .filter_map(|reference| match values.get(*reference) {
            Some(value) => decoupling_limit_mm(value).map(|limit| Candidate {
                reference: (*reference).into(),
                value: value.clone(),
                limit: Some(limit),
            }),
            None => Some(Candidate {
                reference: (*reference).into(),
                value: String::new(),
                limit: None,
            }),
        })
        .collect();
    let ics: BTreeSet<&str> = counts
        .keys()
        .copied()
        .filter(|r| ref_prefix(r) == "U")
        .collect();
    let connectors: BTreeSet<&str> = counts
        .keys()
        .copied()
        .filter(|r| ref_prefix(r) == "J")
        .collect();
    let nets = index.nets();
    let mut by_reference: BTreeMap<&str, BTreeMap<&str, Vec<&PadSite>>> = BTreeMap::new();
    let mut references_by_net: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut nonfinite_pads = index
        .pads_without_net()
        .iter()
        .filter(|p| !finite(p))
        .count();
    for net in &nets {
        for pad in index.pads_of_net(net) {
            nonfinite_pads += usize::from(!finite(pad));
            by_reference
                .entry(&pad.reference)
                .or_default()
                .entry(net)
                .or_default()
                .push(pad);
            references_by_net
                .entry(net)
                .or_default()
                .insert(&pad.reference);
        }
    }
    let ambiguous_reference_count = ambiguous.len();
    let mut details = WitnessBudget::default();
    let ambiguous_details: Vec<_> = ambiguous
        .into_iter()
        .take(AMBIGUOUS_DETAIL_LIMIT)
        .filter(|name| details.admit(name))
        .collect();
    let ambiguous_references_truncated = ambiguous_details.len() < ambiguous_reference_count;
    let mut out = Check {
        violations: Vec::new(),
        uncoupled_caps: Vec::new(),
        interface_filter_caps: Vec::new(),
        associations: Vec::new(),
        unproven_caps: Vec::new(),
        coverage: Coverage {
            status: "complete",
            skipped_pads: index.skipped_pads(),
            nonfinite_pads,
            ambiguous_references: ambiguous_details,
            ambiguous_reference_count,
            ambiguous_references_truncated,
            candidates: candidates.len(),
            evaluated: 0,
            unproven: 0,
            pair_comparisons: 0,
            pair_limit,
            pair_limit_exceeded: false,
            witness_bytes: details.bytes,
            witness_byte_limit: WITNESS_BYTE_LIMIT,
            witness_items: details.items,
            witness_item_limit: WITNESS_ITEM_LIMIT,
        },
    };
    if index.skipped_pads() > 0 || nonfinite_pads > 0 || ambiguous_reference_count > 0 {
        return unavailable(out, &candidates, "pad source is incomplete or references are ambiguous; no proximity association is established");
    }
    let mut budget = Budget {
        used: 0,
        limit: pair_limit,
        exhausted: false,
    };
    for cap in &candidates {
        let Some(limit) = cap.limit else {
            out.unproven_caps.push(unproven(
                cap,
                "the capacitor has no readable Value for family classification",
            ));
            continue;
        };
        let cap_nets = by_reference.get(cap.reference.as_str());
        let association = nearest_ic(
            cap,
            cap_nets,
            &ics,
            index,
            &references_by_net,
            max_net_references,
            &mut budget,
        );
        if budget.exhausted {
            break;
        }
        let Some(mut association) = association else {
            out.uncoupled_caps.push(cap.reference.clone());
            let reason = match cap_nets {
                None => "the capacitor has no named pad nets",
                Some(nets) if nets.keys().all(|n| ground_like(n)) => "ground-like nets alone do not identify an IC",
                _ => "no shared non-ground IC pad pair is within the family limit or has bounded fanout",
            };
            out.unproven_caps.push(unproven(cap, reason));
            continue;
        };
        out.coverage.evaluated += 1;
        if association.distance_mm <= limit {
            association.status = "within_limit";
        } else if let Some(matched) = nearest_connector(
            cap_nets.expect("an association has named pad nets"),
            &connectors,
            index,
            &references_by_net,
            &mut budget,
            details,
        )
        .filter(|matched| matched.distance_mm <= limit)
        {
            association.status = "interface_filter";
            details = matched.details;
            out.interface_filter_caps.push(InterfaceFilterCap {
                cap: cap.reference.clone(),
                value: cap.value.clone(),
                connector: matched.reference,
                distance_mm: matched.distance_mm,
                limit_mm: limit,
                matched_pairs: matched.pairs,
                matched_pair_count: matched.pair_count,
                matched_pairs_truncated: matched.pairs_truncated,
            });
        } else {
            association.status = "too_far";
            out.violations.push(association.clone());
        }
        if budget.exhausted {
            break;
        }
        out.associations.push(association);
    }
    out.coverage.pair_comparisons = budget.used;
    out.coverage.witness_bytes = details.bytes;
    out.coverage.witness_items = details.items;
    if budget.exhausted {
        out.coverage.pair_limit_exceeded = true;
        return unavailable(
            out,
            &candidates,
            "pad-pair comparison budget exhausted; proximity evaluation is unavailable",
        );
    }
    out.coverage.unproven = out.unproven_caps.len();
    if out.coverage.unproven > 0 {
        out.coverage.status = "partial";
    }
    out
}

fn unavailable(mut out: Check, candidates: &[Candidate], reason: &str) -> Check {
    out.violations.clear();
    out.interface_filter_caps.clear();
    out.associations.clear();
    out.uncoupled_caps.clear();
    out.unproven_caps = candidates.iter().map(|cap| unproven(cap, reason)).collect();
    out.coverage.status = "partial";
    out.coverage.evaluated = 0;
    out.coverage.unproven = out.unproven_caps.len();
    // Partial connector evidence was discarded along with partial results.
    let mut retained_details = WitnessBudget::default();
    for name in &out.coverage.ambiguous_references {
        let _ = retained_details.admit(name);
    }
    out.coverage.witness_bytes = retained_details.bytes;
    out.coverage.witness_items = retained_details.items;
    out
}

fn unproven(cap: &Candidate, reason: &str) -> UnprovenCap {
    UnprovenCap {
        cap: cap.reference.clone(),
        value: cap.value.clone(),
        reason: reason.into(),
    }
}

fn finite(pad: &PadSite) -> bool {
    pad.at.0.is_finite() && pad.at.1.is_finite()
}

fn ground_like(net: &str) -> bool {
    let n = net
        .trim()
        .trim_start_matches('/')
        .to_ascii_uppercase()
        .replace(['-', '_'], "");
    n == "0" || n == "0V" || n.starts_with("GND") || n.ends_with("GND") || n.starts_with("VSS")
}

fn pair_order(a: &PadPair, b: &PadPair) -> std::cmp::Ordering {
    a.distance_mm
        .total_cmp(&b.distance_mm)
        .then_with(|| a.net.cmp(&b.net))
        .then_with(|| a.cap_pad.cmp(&b.cap_pad))
        .then_with(|| a.target_pad.cmp(&b.target_pad))
        .then_with(|| a.cap_at.0.total_cmp(&b.cap_at.0))
        .then_with(|| a.cap_at.1.total_cmp(&b.cap_at.1))
        .then_with(|| a.target_at.0.total_cmp(&b.target_at.0))
        .then_with(|| a.target_at.1.total_cmp(&b.target_at.1))
}

fn pair(net: &str, a: &PadSite, b: &PadSite, distance_mm: f64) -> PadPair {
    PadPair {
        net: net.into(),
        cap_pad: a.pad_number.clone(),
        target_pad: b.pad_number.clone(),
        cap_at: a.at,
        target_at: b.at,
        distance_mm,
    }
}

fn nearest_ic(
    cap: &Candidate,
    cap_nets: Option<&BTreeMap<&str, Vec<&PadSite>>>,
    ics: &BTreeSet<&str>,
    index: &PcbConnectivityIndex,
    references_by_net: &BTreeMap<&str, BTreeSet<&str>>,
    maxrefs: usize,
    budget: &mut Budget,
) -> Option<Association> {
    let limit = cap.limit?;
    let mut best: Option<Association> = None;
    for (net, cap_pads) in cap_nets? {
        if ground_like(net) {
            continue;
        }
        let fanout = references_by_net.get(net).map_or(0, BTreeSet::len);
        for cap_pad in cap_pads {
            for ic_pad in index
                .pads_of_net(net)
                .iter()
                .filter(|p| ics.contains(p.reference.as_str()))
            {
                let Some(distance_mm) = budget.distance(cap_pad, ic_pad) else {
                    if budget.exhausted {
                        return None;
                    }
                    continue;
                };
                // A genuinely adjacent pad pair is useful even on a shared
                // VDD rail. A distant pair needs conservative fanout evidence.
                if distance_mm > limit && fanout > maxrefs {
                    continue;
                }
                let found = Association {
                    cap: cap.reference.clone(),
                    value: cap.value.clone(),
                    ic: ic_pad.reference.clone(),
                    qualifying_net: (*net).into(),
                    net_reference_count: fanout,
                    distance_mm,
                    limit_mm: limit,
                    status: "too_far",
                    association_basis: if distance_mm <= limit {
                        "pin_adjacent"
                    } else {
                        "bounded_fanout"
                    },
                    cap_pad: cap_pad.pad_number.clone(),
                    ic_pad: ic_pad.pad_number.clone(),
                    cap_at: cap_pad.at,
                    ic_at: ic_pad.at,
                };
                let ordering = |old: &Association| {
                    distance_mm
                        .total_cmp(&old.distance_mm)
                        .then_with(|| fanout.cmp(&old.net_reference_count))
                        .then_with(|| found.ic.cmp(&old.ic))
                        .then_with(|| found.qualifying_net.cmp(&old.qualifying_net))
                        .then_with(|| found.cap_pad.cmp(&old.cap_pad))
                        .then_with(|| found.ic_pad.cmp(&old.ic_pad))
                        .then_with(|| found.cap_at.0.total_cmp(&old.cap_at.0))
                        .then_with(|| found.cap_at.1.total_cmp(&old.cap_at.1))
                        .then_with(|| found.ic_at.0.total_cmp(&old.ic_at.0))
                        .then_with(|| found.ic_at.1.total_cmp(&old.ic_at.1))
                };
                if best.as_ref().is_none_or(|old| ordering(old).is_lt()) {
                    best = Some(found);
                }
            }
        }
    }
    best
}

fn accessible(a: &PadSite, b: &PadSite) -> bool {
    a.layer_side == b.layer_side || a.reaches_both_faces || b.reaches_both_faces
}

struct ConnectorMatch {
    reference: String,
    distance_mm: f64,
    pairs: Vec<PadPair>,
    pair_count: usize,
    pairs_truncated: bool,
    details: WitnessBudget,
}

fn nearest_connector(
    cap_nets: &BTreeMap<&str, Vec<&PadSite>>,
    connectors: &BTreeSet<&str>,
    index: &PcbConnectivityIndex,
    references_by_net: &BTreeMap<&str, BTreeSet<&str>>,
    budget: &mut Budget,
    details: WitnessBudget,
) -> Option<ConnectorMatch> {
    let possible: Vec<_> = connectors
        .iter()
        .copied()
        .filter(|reference| {
            cap_nets.keys().all(|net| {
                references_by_net
                    .get(net)
                    .is_some_and(|refs| refs.contains(reference))
            })
        })
        .collect();
    let mut best: Option<ConnectorMatch> = None;
    'connector: for reference in possible {
        let mut candidate_details = details;
        let mut matched_pairs = Vec::new();
        let mut pair_count = 0;
        let mut worst = 0.0_f64;
        for (net, cap_pads) in cap_nets {
            let mut closest: Option<PadPair> = None;
            for cap_pad in cap_pads {
                for connector_pad in index
                    .pads_of_net(net)
                    .iter()
                    .filter(|p| p.reference == reference && accessible(cap_pad, p))
                {
                    let Some(d) = budget.distance(cap_pad, connector_pad) else {
                        if budget.exhausted {
                            return None;
                        }
                        continue;
                    };
                    let found = pair(net, cap_pad, connector_pad, d);
                    if closest
                        .as_ref()
                        .is_none_or(|old| pair_order(&found, old).is_lt())
                    {
                        closest = Some(found);
                    }
                }
            }
            let Some(closest) = closest else {
                continue 'connector;
            };
            worst = worst.max(closest.distance_mm);
            pair_count += 1;
            if matched_pairs.len() < MATCHED_PAIR_DETAIL_LIMIT && candidate_details.admit(&closest)
            {
                matched_pairs.push(closest);
            }
        }
        if pair_count == 0 {
            continue;
        }
        if best.as_ref().is_none_or(|old| {
            worst
                .total_cmp(&old.distance_mm)
                .then_with(|| reference.cmp(old.reference.as_str()))
                .is_lt()
        }) {
            let pairs_truncated = matched_pairs.len() < pair_count;
            best = Some(ConnectorMatch {
                reference: reference.into(),
                distance_mm: worst,
                pairs: matched_pairs,
                pair_count,
                pairs_truncated,
                details: candidate_details,
            });
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use konnect_sexp::board::footprint_courtyards;
    use konnect_sexp::SexpNode;

    const FIXTURE: &str = include_str!(
        "../../../../konnect-sexp/tests/fixtures/placement/placement_fixture.kicad_pcb"
    );

    fn tree() -> SexpNode {
        konnect_sexp::parse_sexp(FIXTURE).unwrap()
    }

    fn reference(fp: &SexpNode) -> Option<&str> {
        fp.find_all("property")
            .into_iter()
            .find(|p| p.get(1).and_then(SexpNode::as_str) == Some("Reference"))
            .and_then(|p| p.get(2))
            .and_then(SexpNode::as_str)
    }

    fn children(node: &mut SexpNode) -> &mut Vec<SexpNode> {
        let SexpNode::List(children) = node else {
            panic!("list required")
        };
        children
    }

    fn fp_mut<'a>(tree: &'a mut SexpNode, name: &str) -> &'a mut SexpNode {
        children(tree)
            .iter_mut()
            .find(|n| n.head() == Some("footprint") && reference(n) == Some(name))
            .unwrap()
    }

    fn child_mut<'a>(node: &'a mut SexpNode, name: &str) -> &'a mut SexpNode {
        children(node)
            .iter_mut()
            .find(|n| n.head() == Some(name))
            .unwrap()
    }

    fn move_fp(tree: &mut SexpNode, name: &str, x: f64, y: f64) {
        let at = children(child_mut(fp_mut(tree, name), "at"));
        at[1] = SexpNode::Atom(x.to_string());
        at[2] = SexpNode::Atom(y.to_string());
    }

    fn pad_mut<'a>(fp: &'a mut SexpNode, number: &str) -> &'a mut SexpNode {
        children(fp)
            .iter_mut()
            .find(|n| {
                n.head() == Some("pad") && n.get(1).and_then(SexpNode::as_str) == Some(number)
            })
            .unwrap()
    }

    fn set_pad_at(tree: &mut SexpNode, name: &str, number: &str, x: f64, y: f64) {
        let at = children(child_mut(pad_mut(fp_mut(tree, name), number), "at"));
        at[1] = SexpNode::Atom(x.to_string());
        at[2] = SexpNode::Atom(y.to_string());
    }

    fn result(tree: &SexpNode, maxrefs: usize) -> Check {
        check(
            &footprint_courtyards(tree).items,
            &PcbConnectivityIndex::build(tree),
            &super::super::footprint_values(tree),
            maxrefs,
            &[],
        )
    }

    fn association<'a>(out: &'a Check, cap: &str) -> &'a Association {
        out.associations.iter().find(|a| a.cap == cap).unwrap()
    }

    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 0.000001, "{a} != {b}");
    }

    #[test]
    fn real_kicad_pad_coordinates_replace_the_courtyard_distance() {
        let tree = tree();
        let out = result(&tree, 8);
        assert_eq!(out.coverage.status, "complete");
        assert_eq!(out.violations.len(), 2);
        let a = association(&out, "C1");
        let b = association(&out, "C2");
        assert_eq!(a.cap_pad, "1");
        assert_eq!(a.ic_pad, "8");
        assert_eq!(a.qualifying_net, "VCC");
        assert_eq!(a.cap_at, (19.52, 15.0));
        assert_eq!(a.ic_at, (27.474999, 18.094999));
        near(a.distance_mm, 8.535867143999022);
        near(b.distance_mm, 3.6350859439636363);
        assert_eq!(b.cap_at, (30.0, 15.48));
        assert_eq!(a.association_basis, "bounded_fanout");
        let mut boxes = footprint_courtyards(&tree).items;
        boxes
            .iter_mut()
            .find(|f| f.reference.as_deref() == Some("U1"))
            .unwrap()
            .bbox = (0.0, 0.0, 60.0, 45.0);
        let changed_box = check(
            &boxes,
            &PcbConnectivityIndex::build(&tree),
            &super::super::footprint_values(&tree),
            8,
            &[],
        );
        near(association(&changed_box, "C1").distance_mm, a.distance_mm);
    }

    #[test]
    fn nearby_ground_does_not_replace_the_matching_non_ground_terminal() {
        let mut tree = tree();
        move_fp(&mut tree, "C1", 22.045, 21.904999);
        let out = result(&tree, 8);
        let a = association(&out, "C1");
        assert_eq!(a.qualifying_net, "VCC");
        assert!(a.distance_mm > 5.0);
        assert_eq!(a.status, "too_far");
    }

    #[test]
    fn pin_adjacency_can_prove_near_proximity_on_a_high_fanout_rail() {
        let mut tree = tree();
        move_fp(&mut tree, "C1", 27.954999, 18.094999);
        let out = result(&tree, 3);
        let a = association(&out, "C1");
        assert_eq!(a.net_reference_count, 4);
        near(a.distance_mm, 0.0);
        assert_eq!(a.status, "within_limit");
        assert_eq!(a.association_basis, "pin_adjacent");
        assert!(out.unproven_caps.iter().any(|c| c.cap == "C2"));
        assert!(out.violations.is_empty());
    }

    #[test]
    fn far_high_fanout_is_unproven_until_the_caller_widens_the_limit() {
        let tree = tree();
        let unproven = result(&tree, 3);
        assert!(unproven.associations.is_empty());
        assert!(unproven.violations.is_empty());
        assert_eq!(unproven.unproven_caps.len(), 2);
        assert_eq!(unproven.coverage.status, "partial");
        assert_eq!(result(&tree, 4).violations.len(), 2);
    }

    #[test]
    fn source_loss_and_nonfinite_anchors_are_not_silent_passes() {
        let mut missing = tree();
        children(pad_mut(fp_mut(&mut missing, "C1"), "1")).retain(|n| n.head() != Some("at"));
        let out = result(&missing, 8);
        assert_eq!(out.coverage.skipped_pads, 1);
        assert_eq!(out.coverage.status, "partial");
        assert!(out.associations.is_empty());
        assert!(out.violations.is_empty());
        let mut overflow = tree();
        move_fp(&mut overflow, "C1", 1e308, 15.0);
        set_pad_at(&mut overflow, "C1", "1", 1e308, 0.0);
        let out = result(&overflow, 8);
        assert!(out.coverage.nonfinite_pads > 0);
        assert!(out.associations.is_empty());
        assert!(out.violations.is_empty());
    }

    #[test]
    fn ambiguous_reference_details_are_bounded_without_losing_the_source_count() {
        let tree = tree();
        let names: Vec<_> = (0..150).map(|n| format!("R{n:03}")).collect();
        let out = check(
            &footprint_courtyards(&tree).items,
            &PcbConnectivityIndex::build(&tree),
            &super::super::footprint_values(&tree),
            8,
            &names,
        );
        assert_eq!(out.coverage.ambiguous_reference_count, 150);
        assert_eq!(out.coverage.ambiguous_references.len(), 64);
        assert!(out.coverage.ambiguous_references_truncated);
        assert!(out.associations.is_empty());
    }

    #[test]
    fn duplicate_footprint_references_do_not_form_a_virtual_component() {
        let mut tree = tree();
        children(child_mut(fp_mut(&mut tree, "C2"), "property"))[2] = SexpNode::Str("C1".into());
        let out = result(&tree, 8);
        assert_eq!(out.coverage.ambiguous_references, vec!["C1"]);
        assert!(out.associations.is_empty());
        assert!(out.violations.is_empty());
    }

    #[test]
    fn connector_waiver_measures_all_matching_terminal_nets() {
        let mut tree = tree();
        move_fp(&mut tree, "C1", 6.27, 20.0);
        let out = result(&tree, 8);
        let filter = out
            .interface_filter_caps
            .iter()
            .find(|f| f.cap == "C1")
            .unwrap();
        assert_eq!(filter.connector, "J1");
        near(filter.distance_mm, 0.79);
        assert_eq!(filter.matched_pair_count, 2);
        assert!(!filter.matched_pairs_truncated);
        assert_eq!(association(&out, "C1").status, "interface_filter");
        // VCC remains near; GND's cap terminal is moved far from J1's GND pin.
        set_pad_at(&mut tree, "C1", "2", 10.0, 0.0);
        let out = result(&tree, 8);
        assert!(out.interface_filter_caps.iter().all(|f| f.cap != "C1"));
        assert_eq!(association(&out, "C1").status, "too_far");
    }

    #[test]
    fn a_connector_bbox_near_the_cap_cannot_substitute_for_its_supply_pin() {
        let mut tree = tree();
        move_fp(&mut tree, "C1", 16.0, 20.0);
        let out = result(&tree, 8);
        assert!(out.interface_filter_caps.iter().all(|f| f.cap != "C1"));
        assert_eq!(association(&out, "C1").status, "too_far");
    }

    fn back_cap(tree: &mut SexpNode) {
        let fp = fp_mut(tree, "C1");
        children(child_mut(fp, "layer"))[1] = SexpNode::Str("B.Cu".into());
        for pad in children(fp).iter_mut().filter(|n| n.head() == Some("pad")) {
            for layer in children(child_mut(pad, "layers")).iter_mut().skip(1) {
                if layer.as_str() == Some("F.Cu") {
                    *layer = SexpNode::Str("B.Cu".into());
                }
            }
        }
    }

    fn smd_connector(tree: &mut SexpNode) {
        for pad in children(fp_mut(tree, "J1"))
            .iter_mut()
            .filter(|n| n.head() == Some("pad"))
        {
            let fields = children(pad);
            fields[2] = SexpNode::Atom("smd".into());
            fields.retain(|n| n.head() != Some("drill"));
            *child_mut(pad, "layers") = SexpNode::List(vec![
                SexpNode::Atom("layers".into()),
                SexpNode::Str("F.Cu".into()),
            ]);
        }
    }

    #[test]
    fn opposite_faces_need_copper_reaching_the_matched_terminals() {
        let mut tree = tree();
        move_fp(&mut tree, "C1", 6.27, 20.0);
        back_cap(&mut tree);
        assert!(result(&tree, 8)
            .interface_filter_caps
            .iter()
            .any(|f| f.cap == "C1"));
        // VCC still reaches the back face, but GND no longer does. One
        // accessible terminal cannot waive the other terminal's missing path.
        let ground = pad_mut(fp_mut(&mut tree, "J1"), "2");
        children(ground)[2] = SexpNode::Atom("smd".into());
        children(ground).retain(|n| n.head() != Some("drill"));
        *child_mut(ground, "layers") = SexpNode::List(vec![
            SexpNode::Atom("layers".into()),
            SexpNode::Str("F.Cu".into()),
        ]);
        assert!(result(&tree, 8)
            .interface_filter_caps
            .iter()
            .all(|f| f.cap != "C1"));
        smd_connector(&mut tree);
        assert!(result(&tree, 8)
            .interface_filter_caps
            .iter()
            .all(|f| f.cap != "C1"));
    }

    #[test]
    fn budget_exhaustion_withholds_partial_nearest_results_and_deductions() {
        let mut connector = tree();
        move_fp(&mut connector, "C1", 6.27, 20.0);
        let mut uncoupled = tree();
        children(child_mut(pad_mut(fp_mut(&mut uncoupled, "C1"), "1"), "net"))[1] =
            SexpNode::Str("GND".into());
        // A zero-work failure, then failures after a violation, an exemption,
        // or an uncoupled result has already accumulated.
        for (tree, limit) in [(tree(), 0), (tree(), 3), (connector, 3), (uncoupled, 0)] {
            let out = check_with_budget(
                &footprint_courtyards(&tree).items,
                &PcbConnectivityIndex::build(&tree),
                &super::super::footprint_values(&tree),
                8,
                &[],
                limit,
            );
            assert!(out.coverage.pair_limit_exceeded);
            assert_eq!(out.coverage.pair_comparisons, limit);
            assert_eq!(out.coverage.status, "partial");
            assert_eq!(out.coverage.evaluated, 0);
            assert_eq!(out.unproven_caps.len(), 2);
            assert!(out
                .unproven_caps
                .iter()
                .all(|c| c.reason.contains("budget")));
            assert!(out.associations.is_empty());
            assert!(out.violations.is_empty());
            assert!(out.interface_filter_caps.is_empty());
            assert!(out.uncoupled_caps.is_empty());
            assert_eq!(out.coverage.witness_bytes, 0);
        }
    }

    #[test]
    fn missing_values_are_unproven_instead_of_silently_omitted() {
        let tree = tree();
        let mut values = super::super::footprint_values(&tree);
        values.remove("C1");
        let out = check(
            &footprint_courtyards(&tree).items,
            &PcbConnectivityIndex::build(&tree),
            &values,
            8,
            &[],
        );
        assert_eq!(out.coverage.candidates, 2);
        assert_eq!(out.coverage.status, "partial");
        assert!(out
            .unproven_caps
            .iter()
            .any(|c| c.cap == "C1" && c.reason.contains("Value")));
    }

    #[test]
    fn overflow_distance_and_empty_terminal_maps_do_not_create_proximity_evidence() {
        let tree = tree();
        let index = PcbConnectivityIndex::build(&tree);
        let mut a = index.pads_of_net("VCC")[0].clone();
        let mut b = a.clone();
        a.at = (1e308, 0.0);
        b.at = (-1e308, 0.0);
        let mut budget = Budget {
            used: 0,
            limit: 10,
            exhausted: false,
        };
        assert!(budget.distance(&a, &b).is_none());
        assert!(!budget.exhausted);
        assert!(nearest_connector(
            &BTreeMap::new(),
            &BTreeSet::from(["J1"]),
            &index,
            &BTreeMap::new(),
            &mut budget,
            WitnessBudget::default()
        )
        .is_none());
    }

    #[test]
    fn deterministic_pair_ties_use_net_pad_ids_and_positions() {
        let a = PadPair {
            net: "VDD".into(),
            cap_pad: "1".into(),
            target_pad: "2".into(),
            cap_at: (1.0, 2.0),
            target_at: (3.0, 4.0),
            distance_mm: 2.0,
        };
        let mut b = a.clone();
        b.target_pad = "3".into();
        assert!(pair_order(&a, &b).is_lt());
        b = a.clone();
        b.target_at.0 = 4.0;
        assert!(pair_order(&a, &b).is_lt());
    }

    #[test]
    fn tied_ic_candidates_use_reference_identity_not_file_order() {
        let mut tree = tree();
        let mut clone = fp_mut(&mut tree, "U1").clone();
        children(child_mut(&mut clone, "property"))[2] = SexpNode::Str("U3".into());
        reuuid(&mut clone, &mut 30000);
        let root = children(&mut tree);
        let index = root
            .iter()
            .position(|n| n.head() == Some("footprint") && reference(n) == Some("U1"))
            .unwrap();
        root.insert(index, clone);
        let out = result(&tree, 8);
        assert_eq!(association(&out, "C1").ic, "U1");
        near(association(&out, "C1").distance_mm, 8.535867143999022);
    }

    fn append_terminals(tree: &mut SexpNode, count: usize) {
        for name in ["C1", "J1"] {
            let fp = fp_mut(tree, name);
            let template = fp.find("pad").unwrap().clone();
            for n in 0..count {
                let mut pad = template.clone();
                children(&mut pad)[1] = SexpNode::Str((n + 10).to_string());
                children(child_mut(&mut pad, "net"))[1] = SexpNode::Str(format!("E{n:02}"));
                children(child_mut(&mut pad, "uuid"))[1] = SexpNode::Str(
                    uuid::Uuid::from_u128(if name == "C1" {
                        1000 + n as u128
                    } else {
                        2000 + n as u128
                    })
                    .to_string(),
                );
                let at = children(child_mut(&mut pad, "at"));
                at[1] = SexpNode::Atom("0".into());
                at[2] = SexpNode::Atom(if name == "C1" { "0" } else { "1.27" }.into());
                children(fp).push(pad);
            }
        }
    }

    #[test]
    fn bounded_connector_witnesses_keep_complete_terminal_count_and_worst_distance() {
        let mut tree = tree();
        move_fp(&mut tree, "C1", 6.27, 20.0);
        append_terminals(&mut tree, 40);
        let out = result(&tree, 8);
        let filter = out
            .interface_filter_caps
            .iter()
            .find(|f| f.cap == "C1")
            .unwrap();
        assert_eq!(filter.matched_pair_count, 42);
        assert_eq!(filter.matched_pairs.len(), 32);
        assert!(filter.matched_pairs_truncated);
        near(filter.distance_mm, 0.79);
        // E39 sorts after the stored detail cap. Its distance must still count.
        set_pad_at(&mut tree, "J1", "49", 0.0, 30.0);
        let out = result(&tree, 8);
        assert!(out.interface_filter_caps.iter().all(|f| f.cap != "C1"));
        assert_eq!(association(&out, "C1").status, "too_far");
    }

    #[test]
    fn escaped_ambiguous_names_have_a_serialized_byte_budget_below_the_item_limit() {
        let tree = tree();
        let names: Vec<_> = (0..10)
            .map(|n| format!("{n:02}{}", "\\\"".repeat(700)))
            .collect();
        let out = check(
            &footprint_courtyards(&tree).items,
            &PcbConnectivityIndex::build(&tree),
            &super::super::footprint_values(&tree),
            8,
            &names,
        );
        assert_eq!(out.coverage.ambiguous_reference_count, 10);
        assert!(out.coverage.ambiguous_references.len() < 10);
        assert!(out.coverage.ambiguous_references_truncated);
        let bytes: usize = out
            .coverage
            .ambiguous_references
            .iter()
            .map(|n| serde_json::to_vec(n).unwrap().len() + 1)
            .sum();
        assert!(bytes <= WITNESS_BYTE_LIMIT);
        assert_eq!(bytes, out.coverage.witness_bytes);
    }

    fn clone_cap(tree: &mut SexpNode, new_reference: &str) {
        let mut cap = fp_mut(tree, "C1").clone();
        children(child_mut(&mut cap, "property"))[2] = SexpNode::Str(new_reference.into());
        reuuid(&mut cap, &mut 10000);
        children(tree).push(cap);
    }

    fn reuuid(node: &mut SexpNode, next: &mut u128) {
        if node.head() == Some("uuid") {
            children(node)[1] = SexpNode::Str(uuid::Uuid::from_u128(*next).to_string());
            *next += 1;
        } else if let SexpNode::List(items) = node {
            for child in items.iter_mut().skip(1) {
                reuuid(child, next);
            }
        }
    }

    #[test]
    fn terminal_witness_bytes_are_shared_across_caps_without_losing_worst_distance() {
        let mut tree = tree();
        move_fp(&mut tree, "C1", 6.27, 20.0);
        append_terminals(&mut tree, 2);
        // Each long net is still a real, matched terminal of cap and connector.
        for name in ["C1", "J1"] {
            for number in ["10", "11"] {
                children(child_mut(pad_mut(fp_mut(&mut tree, name), number), "net"))[1] =
                    SexpNode::Str(format!("E{number}{}", "\\\"".repeat(700)));
            }
        }
        clone_cap(&mut tree, "C3");
        let out = result(&tree, 8);
        assert_eq!(out.interface_filter_caps.len(), 2);
        let filters = &out.interface_filter_caps;
        assert!(filters.iter().all(|f| f.matched_pair_count == 4));
        assert!(filters.iter().any(|f| f.matched_pairs_truncated));
        let bytes: usize = filters
            .iter()
            .flat_map(|f| &f.matched_pairs)
            .map(|p| serde_json::to_vec(p).unwrap().len() + 1)
            .sum();
        assert!(bytes <= WITNESS_BYTE_LIMIT);
        assert_eq!(bytes, out.coverage.witness_bytes);
        for filter in filters {
            near(filter.distance_mm, 0.79);
        }
        // A terminal omitted from detail still prevents the waiver if far away.
        set_pad_at(&mut tree, "J1", "11", 0.0, 30.0);
        assert!(result(&tree, 8).interface_filter_caps.is_empty());
    }

    #[test]
    fn witness_budget_item_limit_is_separate_from_serialized_bytes() {
        let mut budget = WitnessBudget::default();
        for _ in 0..WITNESS_ITEM_LIMIT {
            assert!(budget.admit(&"x"));
        }
        assert!(budget.bytes < WITNESS_BYTE_LIMIT);
        assert!(!budget.admit(&"x"));
    }
}
