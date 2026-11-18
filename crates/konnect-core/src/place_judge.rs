//! One judge for every candidate, references included: the placement's own checks,
//! then what the product's best-of-N routing call makes of it, then one rank key.

// Nothing consumes this module until `place_search` lands (the next stage); the allowance
// goes with that change.
#![allow(dead_code)]

use crate::place_held::{DecouplingPair, HeldSet};
use crate::route_judge::{rank_measures, Gates, Measures};
use konnect_sexp::board::PcbConnectivityIndex;
use konnect_sexp::parser::SexpNode;
use serde::Serialize;
use std::collections::BTreeMap;

/// The cheap measures of one candidate, from its `evaluate::evaluate` result (run
/// with every held part as `edge_mount_references`, so a held part that sits over the
/// edge on purpose is listed, not failed) and its own board.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Screen {
    pub verdict: String,
    pub score: i64,
    pub score_incomplete: bool,
    pub hard_failures: usize,
    /// Hard failures that name at least one movable part. A failure among held parts
    /// alone is the same on every candidate, O included, and is no candidate's fault.
    pub movable_hard_failures: usize,
    /// `outline_missing` or `outline_unproven`.
    pub outline_unproven: bool,
    /// The intent's decoupling pairs (taken from O) that this board breaks
    /// (`pair_violations`). Measured on the fixed pair list, never on the candidate's
    /// own associations, so a capacitor moved away from every IC counts.
    pub decoupling_violations: usize,
    /// References of the score's `connector_edge` deduction (`deductions`).
    pub connector_violations: usize,
    /// `routing_metrics`: straight-airwire MST length and crossings; `None` when the
    /// score could not establish them.
    pub wirelength_mm: Option<f64>,
    pub crossings: Option<usize>,
    /// Mean member distance to its group's centre over the board diagonal, averaged
    /// over the groups. Recorded, not ranked (decision D8).
    pub group_spread: Option<f64>,
}

/// The screen of one candidate: `evaluated` read, its hard failures split by `held`,
/// `decoupling_violations` as given. Errs on a field the score always gives that is
/// missing.
pub(crate) fn screen(
    evaluated: &serde_json::Value,
    held: &HeldSet,
    decoupling_violations: usize,
    group_spread: Option<f64>,
) -> anyhow::Result<Screen> {
    let missing = |field: &str| anyhow::anyhow!("the evaluation has no `{field}`");
    let verdict = evaluated["verdict"]
        .as_str()
        .ok_or_else(|| missing("verdict"))?
        .to_string();
    let score = evaluated["score"]
        .as_i64()
        .ok_or_else(|| missing("score"))?;
    let score_incomplete = evaluated["score_incomplete"]
        .as_bool()
        .ok_or_else(|| missing("score_incomplete"))?;
    let hard_failures = evaluated["hard_failures"]
        .as_array()
        .ok_or_else(|| missing("hard_failures"))?;
    // A failure counts as movable when any reference it names is a movable part.
    let movable_hard_failures = hard_failures
        .iter()
        .filter(|failure| {
            failure["references"].as_array().is_some_and(|references| {
                references
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .any(|reference| held.movable.contains(reference))
            })
        })
        .count();
    let outline_unproven = evaluated["outline_missing"]
        .as_bool()
        .ok_or_else(|| missing("outline_missing"))?
        || evaluated["outline_unproven"]
            .as_bool()
            .ok_or_else(|| missing("outline_unproven"))?;
    let deductions = evaluated["deductions"]
        .as_array()
        .ok_or_else(|| missing("deductions"))?;
    let connector_violations = deductions
        .iter()
        .filter(|deduction| deduction["kind"] == "connector_edge")
        .map(|deduction| deduction["references"].as_array().map_or(0, Vec::len))
        .sum();
    let metrics = evaluated["routing_metrics"]
        .as_object()
        .ok_or_else(|| missing("routing_metrics"))?;
    Ok(Screen {
        verdict,
        score,
        score_incomplete,
        hard_failures: hard_failures.len(),
        movable_hard_failures,
        outline_unproven,
        decoupling_violations,
        connector_violations,
        wirelength_mm: metrics
            .get("wirelength_mm")
            .and_then(serde_json::Value::as_f64),
        crossings: metrics
            .get("proper_crossings")
            .and_then(serde_json::Value::as_u64)
            .map(|crossings| crossings as usize),
        group_spread,
    })
}

/// The pairs of `pairs` whose capacitor pad and IC pad lie farther apart on `board`
/// than the pair's `limit_mm` (straight pad-anchor distance, the score's own metric),
/// plus every pair whose pads cannot both be found once on `board`: a pair is never
/// left unmeasured.
pub(crate) fn pair_violations(board: &SexpNode, pairs: &[DecouplingPair]) -> usize {
    let index = PcbConnectivityIndex::build(board);
    // Every pad site of the pairs' references, by (reference, pad number), in file order.
    let mut sites: BTreeMap<(&str, &str), Vec<(f64, f64)>> = BTreeMap::new();
    let netted = index
        .nets()
        .into_iter()
        .flat_map(|net| index.pads_of_net(net).iter());
    for pad in netted.chain(index.pads_without_net()) {
        sites
            .entry((pad.reference.as_str(), pad.pad_number.as_str()))
            .or_default()
            .push(pad.at);
    }
    let found_once = |reference: &str, pad: &str| match sites.get(&(reference, pad)) {
        Some(at) if at.len() == 1 => Some(at[0]),
        _ => None,
    };
    pairs
        .iter()
        .filter(|pair| {
            match (
                found_once(&pair.cap, &pair.cap_pad),
                found_once(&pair.ic, &pair.ic_pad),
            ) {
                (Some(cap), Some(ic)) => (cap.0 - ic.0).hypot(cap.1 - ic.1) > pair.limit_mm,
                _ => true,
            }
        })
        .count()
}

/// Least is best: P1 failed (`movable_hard_failures` above 0 or the outline unproven),
/// score incomplete, decoupling violations, connector violations, crossings,
/// wirelength in 0.1 mm steps. A measure the score did not give sorts last
/// (`i64::MAX`).
pub(crate) type ScreenKey = [i64; 6];

pub(crate) fn screen_key(screen: &Screen) -> ScreenKey {
    [
        i64::from(screen.movable_hard_failures > 0 || screen.outline_unproven),
        i64::from(screen.score_incomplete),
        screen.decoupling_violations as i64,
        screen.connector_violations as i64,
        screen
            .crossings
            .map_or(i64::MAX, |crossings| crossings as i64),
        screen
            .wirelength_mm
            .map_or(i64::MAX, |mm| (mm * 10.0).round() as i64),
    ]
}

/// DRC types P2 does not gate: an unrouted board has every connection open; silk and
/// text move with their footprints and are ordered, not gated (`SILK`); library
/// mismatches do not depend on placement; starved thermals and isolated copper on an
/// unrouted board are the pours' state before routing. Every other type is physical,
/// a type KiCad adds later included.
pub(crate) const NOT_PHYSICAL: &[&str] = &[
    "unconnected_items",
    "silk_overlap",
    "silk_over_copper",
    "silk_edge_clearance",
    "text_height",
    "text_thickness",
    "lib_footprint_issues",
    "lib_footprint_mismatch",
    "starved_thermal",
    "isolated_copper",
];

/// The silk types the rank key counts.
pub(crate) const SILK: &[&str] = &["silk_overlap", "silk_over_copper", "silk_edge_clearance"];

/// Each gate passes when its list is empty; each entry is one readable line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct PlacementGates {
    /// P1: no hard failure of the placement score (a courtyard overlap, a part
    /// outside the true outline) names a movable part, and the outline is closed.
    pub p1: Vec<String>,
    /// P2: no physical DRC type worse than on the input board, both unrouted.
    pub p2: Vec<String>,
}

impl PlacementGates {
    pub(crate) fn pass(&self) -> bool {
        self.p1.is_empty() && self.p2.is_empty()
    }
}

/// P1 from `screen`. P2: every DRC type of `unrouted` (the candidate's unrouted board,
/// refilled and checked as the routing judge's baseline does) that is not in
/// `NOT_PHYSICAL`, at most `input_unrouted`'s count of that type. `unrouted` = `None`
/// (no check came back) is a P2 line, never a pass.
pub(crate) fn placement_gates(
    screen: &Screen,
    unrouted: Option<&BTreeMap<String, usize>>,
    input_unrouted: &BTreeMap<String, usize>,
) -> PlacementGates {
    let mut p1 = Vec::new();
    if screen.movable_hard_failures > 0 {
        p1.push(format!(
            "{} hard failure(s) of the placement score name a movable part",
            screen.movable_hard_failures
        ));
    }
    if screen.outline_unproven {
        p1.push("the outline is not closed, so containment is not proved".to_string());
    }
    let mut p2 = Vec::new();
    match unrouted {
        None => p2.push("no unrouted check came back for this board".to_string()),
        Some(counts) => {
            for (kind, count) in counts {
                if NOT_PHYSICAL.contains(&kind.as_str()) {
                    continue;
                }
                let allowed = input_unrouted.get(kind).copied().unwrap_or(0);
                if *count > allowed {
                    p2.push(format!(
                        "{kind}: {count} on this board, {allowed} on the input board"
                    ));
                }
            }
        }
    }
    PlacementGates { p1, p2 }
}

/// What a candidate's routing call gave.
// One record per candidate, a few hundred in a search: the size gap between the variants
// costs nothing worth a `Box` in every reader of the skeleton's shape.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RouteResult {
    /// The call's winner (`search.winner`: its gates and measures), the candidate's
    /// unrouted board check (`search.baseline`), the call's `status`, and the call's
    /// summary as recorded (engine, effort, lanes, outcome counts, elapsed, winner shape
    /// and seed, the search folder).
    Routed {
        gates: Gates,
        measures: Measures,
        unrouted: BTreeMap<String, usize>,
        status: String,
        call: serde_json::Value,
    },
    /// The call ran but judged no run; its baseline when it had one.
    NoWinner {
        unrouted: Option<BTreeMap<String, usize>>,
        reason: String,
    },
    /// A stage failed: `export` (the DSN export refused) or `call` (the call refused or
    /// erred).
    Failed { stage: &'static str, reason: String },
    /// Not routed: `screened_out`, `time_limit` or `stopped`.
    NotRouted { why: &'static str },
}

/// Everything the key reads about one candidate.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Judged {
    /// The candidate's id: O's routing calls first, then A's, then the generated
    /// candidates in recipe order.
    pub id: usize,
    pub screen: Screen,
    pub placement: PlacementGates,
    pub route: RouteResult,
    /// Silk DRC entries (`SILK`) on the candidate's unrouted board; `None` without one.
    pub silk: Option<usize>,
}

/// A candidate's rank key; the least is the best.
pub(crate) type Key = [i64; 16];

/// The key of every judged candidate at once (the length steps need the pool):
/// 0. P1 or P2 failed;
/// 1. not routed (`Routed` is 0; every other result 1);
/// 2. G1 failed;
/// 3. KiCad's unconnected count after the refill and stitching (`i64::MAX` when not
///    routed): a candidate that routes less can never win by being shorter;
/// 4. G2 failed; 5. G3 failed;
/// 6. decoupling violations; 7. connector violations (requirements before preferences);
/// 8. to 13. the routing key's acute corners, signal and power length steps, right
///    angles, vias off the pour nets and minus copper quality, from
///    `route_judge::rank_measures`, the steps taken against the shortest routed
///    candidate that passes P1, P2 and G1 (so an illegal placement cannot set them);
/// 14. silk entries; 15. minus the placement score.
///
/// Missing values sort last. Ties go to the lower id, so O, then A, then the
/// generated candidates in recipe order: a generated candidate must be strictly
/// better to rank above a reference.
pub(crate) fn rank(judged: &[&Judged]) -> Vec<Key> {
    // The routed candidates, with the pool flag the length steps read: it passes P1,
    // P2 and G1.
    let routed: Vec<(usize, &Measures, &Gates, bool)> = judged
        .iter()
        .enumerate()
        .filter_map(|(position, run)| match &run.route {
            RouteResult::Routed {
                gates, measures, ..
            } => Some((
                position,
                measures,
                gates,
                run.placement.pass() && gates.g1.is_empty(),
            )),
            _ => None,
        })
        .collect();
    let runs: Vec<(&Measures, &Gates, bool)> = routed
        .iter()
        .map(|&(_, measures, gates, pool)| (measures, gates, pool))
        .collect();
    let routing_keys = rank_measures(&runs);
    let mut routing_of: BTreeMap<usize, (&Measures, &Gates, crate::route_judge::Key)> =
        BTreeMap::new();
    for ((position, measures, gates, _), key) in routed.iter().zip(routing_keys) {
        routing_of.insert(*position, (*measures, *gates, key));
    }

    judged
        .iter()
        .enumerate()
        .map(|(position, run)| {
            let mut key = [i64::MAX; 16];
            key[0] = i64::from(!run.placement.pass());
            key[1] = i64::from(!routing_of.contains_key(&position));
            if let Some((measures, gates, route)) = routing_of.get(&position) {
                key[2] = i64::from(!gates.g1.is_empty());
                key[3] = measures.unconnected as i64;
                key[4] = i64::from(!gates.g2.is_empty());
                key[5] = i64::from(!gates.g3.is_empty());
                key[8..14].copy_from_slice(&route[3..9]);
            }
            key[6] = run.screen.decoupling_violations as i64;
            key[7] = run.screen.connector_violations as i64;
            key[14] = run.silk.map_or(i64::MAX, |silk| silk as i64);
            key[15] = -run.screen.score;
            key
        })
        .collect()
}

/// The ids of `judged`, best first: by [`rank`]'s key, ties to the lower id.
pub(crate) fn order(judged: &[&Judged]) -> Vec<usize> {
    let keys = rank(judged);
    let mut positions: Vec<usize> = (0..judged.len()).collect();
    positions.sort_by(|&a, &b| {
        keys[a]
            .cmp(&keys[b])
            .then_with(|| judged[a].id.cmp(&judged[b].id))
    });
    positions.into_iter().map(|p| judged[p].id).collect()
}

#[cfg(test)]
#[path = "place_judge_tests.rs"]
mod tests;
