//! Which footprints a placement search may move, and the intent that steers it.
//!
//! Read from the saved board text only (in the bench, a copy), never from a live
//! editor: the text is the evidence, and what it cannot prove is refused, as the
//! `kicad-pcb` skill's bounded placement loop reports `BLOCKED` instead of guessing.

// Nothing consumes this module until `place_search` lands (the next stage); the allowance
// goes with that change.
#![allow(dead_code)]

use konnect_sexp::board::{
    footprint_courtyards, footprints, FootprintCourtyard, PcbConnectivityIndex, Side,
};
use konnect_sexp::outline::{Outline, OutlineStatus};
use konnect_sexp::parser::SexpNode;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Two numeric atoms closer than this are the same number (mm or degrees).
const SAME_NUMBER_TOLERANCE: f64 = 1e-6;

/// Why a footprint never moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HoldReason {
    /// KiCad's own lock: a direct `(locked yes)` (`FootprintCourtyard::locked`).
    KicadLock,
    /// Named by the caller (the bench's per-board list).
    Caller,
    /// A connector ([`HoldRules::connector_prefixes`]), at the edge or not: its place
    /// and its facing serve a cable, a mating board or the enclosure.
    Connector,
    /// A mounting hole, fiducial or logo: no pad on any net, or a mechanical prefix or
    /// footprint name ([`HoldRules`]).
    Mechanical,
}

/// A held footprint, as the saved text has it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HeldPart {
    pub reasons: BTreeSet<HoldReason>,
    /// The root `(at x y rot)` and the side, exactly as read.
    pub at: (f64, f64),
    pub rotation_deg: f64,
    pub side: Side,
}

/// The rules that hold a part besides KiCad's lock and the caller's list.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HoldRules {
    /// Reference prefixes (the letters before the number) that name connectors.
    /// Default `["J"]`, as `score_placement`'s connector check reads them.
    pub connector_prefixes: Vec<String>,
    /// Reference prefixes of mechanical parts. Default `["H", "MH", "FID"]`.
    pub mechanical_prefixes: Vec<String>,
    /// Words in a footprint's library ID that mark it mechanical, matched without
    /// case. Default `["MountingHole", "Fiducial"]`.
    pub mechanical_footprint_words: Vec<String>,
}

impl Default for HoldRules {
    /// The defaults above.
    fn default() -> Self {
        let owned = |words: &[&str]| words.iter().map(|w| (*w).to_string()).collect();
        Self {
            connector_prefixes: owned(&["J"]),
            mechanical_prefixes: owned(&["H", "MH", "FID"]),
            mechanical_footprint_words: owned(&["MountingHole", "Fiducial"]),
        }
    }
}

/// The parts that stay and the parts that may move.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HeldSet {
    pub held: BTreeMap<String, HeldPart>,
    /// Every other footprint: the parts a generator may move.
    pub movable: BTreeSet<String>,
    /// SHA-256 of the board text the set was read from.
    pub board_sha256: String,
    /// SHA-256 of the held list as the records write it: sorted references, each with
    /// its `at`, rotation, side and reasons.
    pub held_sha256: String,
}

/// Why the held set cannot be proved, with the step that fixes it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct HeldRefusal {
    pub reason: String,
    pub next_step: String,
}

fn refuse(reason: impl Into<String>, next_step: &str) -> HeldRefusal {
    HeldRefusal {
        reason: reason.into(),
        next_step: next_step.to_string(),
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The letters before the number of a reference: `"J12"` is `"J"`, `"MH3"` is `"MH"`.
fn prefix_of(reference: &str) -> &str {
    let end = reference
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(reference.len());
    &reference[..end]
}

/// The reference designator of a footprint node: `(property "Reference" …)`, or the
/// legacy `(fp_text reference …)`.
fn reference_of(fp: &SexpNode) -> Option<String> {
    for property in fp.find_all("property") {
        if property.get(1).and_then(SexpNode::as_str) == Some("Reference") {
            return property
                .get(2)
                .and_then(SexpNode::as_str)
                .map(str::to_string);
        }
    }
    for text in fp.find_all("fp_text") {
        if text.get(1).and_then(SexpNode::as_str) == Some("reference") {
            return text.get(2).and_then(SexpNode::as_str).map(str::to_string);
        }
    }
    None
}

fn side_word(side: Side) -> &'static str {
    match side {
        Side::Front => "front",
        Side::Back => "back",
    }
}

/// The held list as the records write it, hashed.
fn held_list_sha256(held: &BTreeMap<String, HeldPart>) -> String {
    let mut text = String::new();
    for (reference, part) in held {
        let reasons: Vec<String> = part
            .reasons
            .iter()
            .map(|reason| format!("{reason:?}"))
            .collect();
        text.push_str(&format!(
            "{reference}|{}|{}|{}|{}|{}\n",
            part.at.0,
            part.at.1,
            part.rotation_deg,
            side_word(part.side),
            reasons.join(",")
        ));
    }
    sha256_hex(text.as_bytes())
}

/// The held set of the saved board `text`. Refuses, never guesses, when:
/// - the text does not parse as a board;
/// - the outline is not closed (`Outline::status` is not `Closed`): no candidate's
///   containment could be judged or repaired;
/// - a reference in `caller` names no footprint, or more than one;
/// - a footprint has no reference, or two footprints with pads share one (the rule
///   of `score_placement`'s `ambiguous_pad_references`);
/// - the courtyard scan skipped a footprint (unreadable geometry): where it is cannot
///   be proved;
/// - nothing is left to move.
///
/// The text is the only evidence. That is the placement loop's rule that a lock record
/// counts only from a saved file known to be current, and a copy is current by
/// construction. A product caller, if one is ever allowed (owner decision 1), must
/// first refuse a board open in KiCad (`refuse_if_board_open_in_kicad`), as the
/// deprecated planner's apply does.
pub(crate) fn held_set(
    text: &str,
    caller: &BTreeSet<String>,
    rules: &HoldRules,
) -> Result<HeldSet, HeldRefusal> {
    let tree = konnect_sexp::parse_sexp(text).map_err(|error| {
        refuse(
            format!("the board text does not parse ({error})"),
            "give the search a saved .kicad_pcb that KiCad can open",
        )
    })?;
    if tree.head() != Some("kicad_pcb") {
        return Err(refuse(
            "the text is not a (kicad_pcb ...) board",
            "give the search a saved .kicad_pcb that KiCad can open",
        ));
    }
    let status = Outline::from_tree(&tree).status();
    if status != OutlineStatus::Closed {
        return Err(refuse(
            format!(
                "the board outline is {} (Edge.Cuts), so no candidate's containment can be judged or repaired",
                status.key()
            ),
            "close the Edge.Cuts outline on the board, save it, then run the search again",
        ));
    }

    // Identity: every footprint has a reference, and each reference is on the board once.
    let nodes = footprints(&tree);
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut nameless = 0usize;
    for fp in &nodes {
        match reference_of(fp) {
            Some(reference) if !reference.trim().is_empty() => {
                *seen.entry(reference).or_default() += 1;
            }
            _ => nameless += 1,
        }
    }
    if nameless > 0 {
        return Err(refuse(
            format!("{nameless} footprint(s) have no reference"),
            "give every footprint a reference in KiCad, save the board, then run the search again",
        ));
    }
    let shared: Vec<&str> = seen
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(reference, _)| reference.as_str())
        .collect();
    if !shared.is_empty() {
        return Err(refuse(
            format!(
                "{} reference(s) are shared by more than one footprint: {}",
                shared.len(),
                shared.join(", ")
            ),
            "rename the duplicate references in KiCad (Tools > Annotate), save the board, then run the search again",
        ));
    }

    let scan = footprint_courtyards(&tree);
    if scan.skipped > 0 {
        return Err(refuse(
            format!(
                "the courtyard scan skipped {} footprint(s) with unreadable geometry, so where they are cannot be proved",
                scan.skipped
            ),
            "open the board in KiCad, repair or delete the footprints it flags, save, then run the search again",
        ));
    }
    let courtyards: BTreeMap<&str, &FootprintCourtyard> = scan
        .items
        .iter()
        .filter_map(|c| c.reference.as_deref().map(|r| (r, c)))
        .collect();

    let unknown: Vec<&str> = caller
        .iter()
        .filter(|reference| !courtyards.contains_key(reference.as_str()))
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        return Err(refuse(
            format!(
                "the caller's list names {} reference(s) that are not on the board: {}",
                unknown.len(),
                unknown.join(", ")
            ),
            "correct the held list so that every reference names exactly one footprint of this board",
        ));
    }

    // A pad on a net, per footprint reference, from the connectivity index.
    let index = PcbConnectivityIndex::build(&tree);
    let mut has_netted_pad: BTreeSet<&str> = BTreeSet::new();
    for net in index.nets() {
        for pad in index.pads_of_net(net) {
            has_netted_pad.insert(pad.reference.as_str());
        }
    }
    let library_ids: BTreeMap<String, String> = nodes
        .iter()
        .filter_map(|fp| {
            Some((
                reference_of(fp)?,
                fp.get(1).and_then(SexpNode::as_str)?.to_string(),
            ))
        })
        .collect();

    let mut held: BTreeMap<String, HeldPart> = BTreeMap::new();
    let mut movable: BTreeSet<String> = BTreeSet::new();
    for (reference, courtyard) in &courtyards {
        let mut reasons: BTreeSet<HoldReason> = BTreeSet::new();
        if courtyard.locked {
            reasons.insert(HoldReason::KicadLock);
        }
        if caller.contains(*reference) {
            reasons.insert(HoldReason::Caller);
        }
        let prefix = prefix_of(reference);
        if rules.connector_prefixes.iter().any(|p| p == prefix) {
            reasons.insert(HoldReason::Connector);
        }
        let library_id = library_ids
            .get(*reference)
            .map(|id| id.to_ascii_lowercase())
            .unwrap_or_default();
        if !has_netted_pad.contains(reference)
            || rules.mechanical_prefixes.iter().any(|p| p == prefix)
            || rules
                .mechanical_footprint_words
                .iter()
                .any(|word| library_id.contains(&word.to_ascii_lowercase()))
        {
            reasons.insert(HoldReason::Mechanical);
        }
        if reasons.is_empty() {
            movable.insert((*reference).to_string());
        } else {
            held.insert(
                (*reference).to_string(),
                HeldPart {
                    reasons,
                    at: courtyard.at,
                    rotation_deg: courtyard.rotation_deg,
                    side: courtyard.layer_side,
                },
            );
        }
    }
    if movable.is_empty() {
        return Err(refuse(
            "every footprint is held, so nothing is left to move",
            "release at least one part from the held list (or from the hold rules)",
        ));
    }

    Ok(HeldSet {
        held_sha256: held_list_sha256(&held),
        board_sha256: sha256_hex(text.as_bytes()),
        held,
        movable,
    })
}

/// Proves that `candidate` keeps every held part exactly (`at`, rotation and side
/// compared as read, with no tolerance) and has the same footprint references as the
/// set's board, each once. Each difference is one line.
pub(crate) fn check_held(set: &HeldSet, candidate: &SexpNode) -> Result<(), Vec<String>> {
    let mut lines: Vec<String> = Vec::new();
    let scan = footprint_courtyards(candidate);
    if scan.skipped > 0 {
        lines.push(format!(
            "{} footprint(s) of the candidate could not be read",
            scan.skipped
        ));
    }
    let total_nodes = footprints(candidate).len();
    if total_nodes != scan.items.len() + scan.skipped {
        lines.push("the footprint count of the candidate is inconsistent".to_string());
    }
    let mut by_reference: BTreeMap<&str, Vec<&FootprintCourtyard>> = BTreeMap::new();
    for item in &scan.items {
        match item.reference.as_deref() {
            Some(reference) => by_reference.entry(reference).or_default().push(item),
            None => lines.push("a footprint of the candidate has no reference".to_string()),
        }
    }
    for (reference, items) in &by_reference {
        if items.len() > 1 {
            lines.push(format!(
                "{reference} is on the candidate {} times",
                items.len()
            ));
        }
        if !set.held.contains_key(*reference) && !set.movable.contains(*reference) {
            lines.push(format!("{reference} is a footprint the board did not have"));
        }
    }
    for reference in set.held.keys().chain(set.movable.iter()) {
        if !by_reference.contains_key(reference.as_str()) {
            lines.push(format!("{reference} is missing from the candidate"));
        }
    }
    for (reference, part) in &set.held {
        let Some(items) = by_reference.get(reference.as_str()) else {
            continue;
        };
        let Some(found) = items.first() else { continue };
        if found.at != part.at {
            lines.push(format!(
                "held {reference} moved from ({}, {}) to ({}, {})",
                part.at.0, part.at.1, found.at.0, found.at.1
            ));
        }
        if found.rotation_deg != part.rotation_deg {
            lines.push(format!(
                "held {reference} turned from {} to {} degrees",
                part.rotation_deg, found.rotation_deg
            ));
        }
        if found.layer_side != part.side {
            lines.push(format!(
                "held {reference} changed side from {} to {}",
                side_word(part.side),
                side_word(found.layer_side)
            ));
        }
    }
    if lines.is_empty() {
        Ok(())
    } else {
        Err(lines)
    }
}

/// The root anchors and rotations of `references` as `tree` has them, in reference
/// order. Errs naming a reference that is not on the board exactly once.
pub(crate) fn placements_of(
    tree: &SexpNode,
    references: &BTreeSet<String>,
) -> anyhow::Result<Vec<konnect_ipc::types::IpcFootprintPlacement>> {
    let scan = footprint_courtyards(tree);
    let mut found: BTreeMap<&str, Vec<&FootprintCourtyard>> = BTreeMap::new();
    for item in &scan.items {
        if let Some(reference) = item.reference.as_deref() {
            found.entry(reference).or_default().push(item);
        }
    }
    let mut placements = Vec::with_capacity(references.len());
    for reference in references {
        match found.get(reference.as_str()).map(Vec::as_slice) {
            Some([one]) => placements.push(konnect_ipc::types::IpcFootprintPlacement {
                reference: reference.clone(),
                x: one.at.0,
                y: one.at.1,
                rotation: one.rotation_deg,
            }),
            Some(many) => anyhow::bail!(
                "{reference} is on the board {} times, not exactly once",
                many.len()
            ),
            None => anyhow::bail!("{reference} is not on the board"),
        }
    }
    Ok(placements)
}

/// Whether two boards are the same board: equal parsed trees, with an atom that
/// reads as a number on both sides compared as a number to within 1e-6 (mm or
/// degrees) and every other atom compared exactly. Formatting and whitespace do not
/// count. Errs with the path of the first difference.
pub(crate) fn same_board(a: &SexpNode, b: &SexpNode) -> Result<(), String> {
    fn label(node: &SexpNode, position: usize) -> String {
        match node.head() {
            Some("footprint") => match reference_of(node) {
                Some(reference) => format!("footprint({reference})"),
                None => format!("footprint[{position}]"),
            },
            Some(head) => format!("{head}[{position}]"),
            None => format!("[{position}]"),
        }
    }
    fn walk(a: &SexpNode, b: &SexpNode, path: &mut Vec<String>) -> Result<(), String> {
        let here = || path.join("/");
        match (a, b) {
            (SexpNode::Atom(x), SexpNode::Atom(y)) => {
                if x == y {
                    return Ok(());
                }
                match (x.parse::<f64>(), y.parse::<f64>()) {
                    (Ok(nx), Ok(ny))
                        if nx.is_finite()
                            && ny.is_finite()
                            && (nx - ny).abs() <= SAME_NUMBER_TOLERANCE =>
                    {
                        Ok(())
                    }
                    _ => Err(format!("{}: {x} differs from {y}", here())),
                }
            }
            (SexpNode::Str(x), SexpNode::Str(y)) => {
                if x == y {
                    Ok(())
                } else {
                    Err(format!("{}: \"{x}\" differs from \"{y}\"", here()))
                }
            }
            (SexpNode::List(x), SexpNode::List(y)) => {
                if x.len() != y.len() {
                    return Err(format!(
                        "{}: {} children against {}",
                        here(),
                        x.len(),
                        y.len()
                    ));
                }
                for (position, (left, right)) in x.iter().zip(y.iter()).enumerate() {
                    path.push(label(left, position));
                    walk(left, right, path)?;
                    path.pop();
                }
                Ok(())
            }
            _ => Err(format!("{}: a different kind of node", here())),
        }
    }
    let mut path = vec![label(a, 0)];
    walk(a, b, &mut path)
}

/// Where a functional group came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GroupSource {
    Caller,
    /// A KiCad `(group …)` on the board that holds footprints.
    KicadGroup,
    /// One schematic sheet instance: the footprints whose `(path …)` share every
    /// element but the last.
    Sheet,
}

/// Parts that belong together (checklist 2: group by function).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Group {
    pub name: String,
    pub source: GroupSource,
    pub members: BTreeSet<String>,
}

/// A capacitor and the IC pad it decouples, as `score_placement` associated them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DecouplingPair {
    pub cap: String,
    pub cap_pad: String,
    pub ic: String,
    pub ic_pad: String,
    pub net: String,
    pub limit_mm: f64,
    /// `association_basis`, as the score gave it.
    pub basis: String,
}

/// What steers the generators beyond the netlist.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(crate) struct Intent {
    pub groups: Vec<Group>,
    pub pairs: Vec<DecouplingPair>,
    /// SHA-256 of this record as `search.json` writes it.
    pub sha256: String,
}

impl Intent {
    /// The intent of `groups` and `pairs`, with its SHA-256: the hash of the record's
    /// `groups` and `pairs` as JSON, in the order given.
    pub(crate) fn new(groups: Vec<Group>, pairs: Vec<DecouplingPair>) -> Self {
        let record = serde_json::json!({ "groups": groups, "pairs": pairs });
        let sha256 = sha256_hex(record.to_string().as_bytes());
        Self {
            groups,
            pairs,
            sha256,
        }
    }
}

/// One pair per entry of `evaluated["decoupling_associations"]` (an
/// `evaluate::evaluate` result for the input board), whatever the entry's status;
/// nothing else. A pair is never inferred from a shared net, ground or not. Errs on an
/// entry that lacks a field.
pub(crate) fn decoupling_pairs(
    evaluated: &serde_json::Value,
) -> anyhow::Result<Vec<DecouplingPair>> {
    let entries = evaluated["decoupling_associations"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("the evaluation has no decoupling_associations list"))?;
    let text =
        |entry: &serde_json::Value, position: usize, field: &str| -> anyhow::Result<String> {
            entry[field].as_str().map(str::to_string).ok_or_else(|| {
                anyhow::anyhow!("decoupling_associations[{position}] has no text field `{field}`")
            })
        };
    entries
        .iter()
        .enumerate()
        .map(|(position, entry)| {
            Ok(DecouplingPair {
                cap: text(entry, position, "reference")?,
                cap_pad: text(entry, position, "cap_pad")?,
                ic: text(entry, position, "ic")?,
                ic_pad: text(entry, position, "ic_pad")?,
                net: text(entry, position, "qualifying_net")?,
                limit_mm: entry["limit_mm"].as_f64().ok_or_else(|| {
                    anyhow::anyhow!("decoupling_associations[{position}] has no number `limit_mm`")
                })?,
                basis: text(entry, position, "association_basis")?,
            })
        })
        .collect()
}

/// The groups: the caller's first, then each KiCad `(group …)` that holds footprints,
/// then one per schematic sheet instance when the board has at least 2. Held members
/// are dropped (they do not move), and a group left with fewer than 2 members is
/// dropped. A part may be in several groups.
pub(crate) fn groups(tree: &SexpNode, held: &HeldSet, caller: &[Group]) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    let mut keep = |name: String, source: GroupSource, members: BTreeSet<String>| {
        let members: BTreeSet<String> = members
            .into_iter()
            .filter(|reference| held.movable.contains(reference))
            .collect();
        if members.len() >= 2 {
            out.push(Group {
                name,
                source,
                members,
            });
        }
    };

    for group in caller {
        keep(group.name.clone(), group.source, group.members.clone());
    }

    // KiCad `(group "name" (uuid …) (members "uuid" …))`: members are the uuids of
    // footprints and of other groups, which are followed.
    let nodes = footprints(tree);
    let mut footprint_of_uuid: BTreeMap<String, String> = BTreeMap::new();
    for fp in &nodes {
        if let (Some(uuid), Some(reference)) = (fp.find_str("uuid"), reference_of(fp)) {
            footprint_of_uuid.insert(uuid.to_string(), reference);
        }
    }
    let kicad_groups = tree.find_all("group");
    let mut group_of_uuid: BTreeMap<String, usize> = BTreeMap::new();
    for (position, group) in kicad_groups.iter().enumerate() {
        if let Some(uuid) = group.find_str("uuid") {
            group_of_uuid.insert(uuid.to_string(), position);
        }
    }
    fn collect_members(
        position: usize,
        groups: &[&SexpNode],
        group_of_uuid: &BTreeMap<String, usize>,
        footprint_of_uuid: &BTreeMap<String, String>,
        visited: &mut BTreeSet<usize>,
        into: &mut BTreeSet<String>,
    ) {
        if !visited.insert(position) {
            return;
        }
        let Some(members) = groups[position].find("members") else {
            return;
        };
        for member in members.children().unwrap_or(&[]).iter().skip(1) {
            let Some(uuid) = member.as_str() else {
                continue;
            };
            if let Some(reference) = footprint_of_uuid.get(uuid) {
                into.insert(reference.clone());
            } else if let Some(&inner) = group_of_uuid.get(uuid) {
                collect_members(
                    inner,
                    groups,
                    group_of_uuid,
                    footprint_of_uuid,
                    visited,
                    into,
                );
            }
        }
    }
    for (position, group) in kicad_groups.iter().enumerate() {
        let mut members = BTreeSet::new();
        collect_members(
            position,
            &kicad_groups,
            &group_of_uuid,
            &footprint_of_uuid,
            &mut BTreeSet::new(),
            &mut members,
        );
        if members.is_empty() {
            continue;
        }
        let name = match group.get(1).and_then(SexpNode::as_str) {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => format!("kicad_group_{position}"),
        };
        keep(name, GroupSource::KicadGroup, members);
    }

    // Schematic sheet instances: a footprint's `(path "/sheet/…/symbol")` minus its
    // last element names the sheet instance it came from.
    let mut by_sheet: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for fp in &nodes {
        let (Some(path), Some(reference)) = (fp.find_str("path"), reference_of(fp)) else {
            continue;
        };
        let elements: Vec<&str> = path.split('/').filter(|e| !e.is_empty()).collect();
        let Some((_, parents)) = elements.split_last() else {
            continue;
        };
        by_sheet
            .entry(format!("/{}", parents.join("/")))
            .or_default()
            .insert(reference);
    }
    if by_sheet.len() >= 2 {
        for (sheet, members) in by_sheet {
            keep(format!("sheet {sheet}"), GroupSource::Sheet, members);
        }
    }
    out
}

#[cfg(test)]
#[path = "place_held_tests.rs"]
pub(crate) mod tests;
