//! Read-only evidence for suspected zone-fill defects. A reported zero is a
//! formatted KiCad measurement, not independent proof of physical overlap.

use crate::tools::cli::{DrcReport, DrcViolation};
use konnect_sexp::SexpNode;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const EXAMPLES: usize = 10;
const TEXT_LIMIT: usize = 200;
const DIAGNOSTIC_BYTES: usize = 6_000;

/// KiCad's per-run DRC limits, from the KiCad 10.0.1 source
/// (`pcbnew/drc/drc_engine.cpp`: `ERROR_LIMIT` and `EXTENDED_ERROR_LIMIT`,
/// set per error code in `DRC_ENGINE::RunTests`), not from a native run. A
/// provider stops testing a code once its limit is used up, and the report
/// does not say so. `ReportViolation` counts down before exclusions are
/// filtered out, so excluded markers use up the limit too.
const KICAD_ERROR_LIMIT: usize = 199;
/// The larger limit KiCad gives `DRCE_CLEARANCE`.
const KICAD_CLEARANCE_ERROR_LIMIT: usize = 499;

/// The DRC codes this diagnostic reads, by their report and settings key.
const SUPPORTED_RULES: [&str; 3] = ["clearance", "hole_clearance", "shorting_items"];

fn kicad_rule_limit(rule: &str) -> Option<usize> {
    match rule {
        "clearance" => Some(KICAD_CLEARANCE_ERROR_LIMIT),
        "hole_clearance" | "shorting_items" => Some(KICAD_ERROR_LIMIT),
        _ => None,
    }
}

/// What the project file says about the supported rules. KiCad 10 keeps DRC
/// exclusions at `board.design_settings.drc_exclusions`, each a
/// `rule|x|y|id|id…` string or a `[string, comment]` pair
/// (`board_design_settings.cpp`, `pcb_marker.cpp`), and rule severities at
/// `board.design_settings.rule_severities`. `DRC_ENGINE::RunTests` gives an
/// ignored code a zero limit, so KiCad never tests it.
#[derive(Default)]
struct ProjectRules {
    excluded_per_rule: BTreeMap<String, usize>,
    excluded_ids: BTreeSet<String>,
    ignored: BTreeSet<&'static str>,
}

#[cfg(test)]
#[path = "zone_fill_defects_tests.rs"]
mod tests;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub(super) board: String,
    sidecars: [Option<Vec<u8>>; 2],
}

pub(super) struct Comparison {
    pub(super) before: Result<Snapshot, String>,
    pub(super) after: Result<Snapshot, String>,
    pub(super) report: DrcReport,
}

impl Snapshot {
    pub(super) async fn capture(path: &Path) -> Result<Self, String> {
        let board = tokio::fs::read_to_string(path)
            .await
            .map_err(|_| "board snapshot unavailable".to_owned())?;
        let mut sidecars = [None, None];
        for (i, extension) in ["kicad_pro", "kicad_dru"].iter().enumerate() {
            sidecars[i] = match tokio::fs::read(path.with_extension(extension)).await {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(_) => return Err("project/rule snapshot unavailable".into()),
            };
        }
        Ok(Self { board, sidecars })
    }

    /// Anything other than an absent list of string or `[string, comment]`
    /// entries is unreadable: an exclusion that cannot be read could still
    /// hide a pair or use up KiCad's limit. KiCad reads severities only from
    /// an object and disables a rule only for the exact string `ignore`
    /// (`SeverityFromString`); a supported rule's severity that is not a
    /// string is unreadable.
    fn project_rules(&self) -> Result<ProjectRules, &'static str> {
        let mut result = ProjectRules::default();
        let Some(bytes) = &self.sidecars[0] else {
            return Ok(result);
        };
        let project: Value =
            serde_json::from_slice(bytes).map_err(|_| "project_exclusions_unreadable")?;
        let settings = &project["board"]["design_settings"];
        if let Some(severities) = settings["rule_severities"].as_object() {
            for rule in SUPPORTED_RULES {
                match severities.get(rule) {
                    None => {}
                    Some(Value::String(severity)) if severity == "ignore" => {
                        result.ignored.insert(rule);
                    }
                    Some(Value::String(_)) => {}
                    Some(_) => return Err("project_rule_severity_unreadable"),
                }
            }
        }
        let list = &settings["drc_exclusions"];
        if list.is_null() {
            return Ok(result);
        }
        for entry in list.as_array().ok_or("project_exclusions_unreadable")? {
            let text = entry
                .as_str()
                .or_else(|| entry.get(0)?.as_str())
                .ok_or("project_exclusions_unreadable")?;
            let mut fields = text.split('|');
            let rule = fields.next().unwrap_or_default();
            *result.excluded_per_rule.entry(rule.to_owned()).or_default() += 1;
            result
                .excluded_ids
                .extend(fields.skip(2).filter_map(id_key));
        }
        Ok(result)
    }
}

#[derive(Clone)]
struct Object {
    node: SexpNode,
    kind: String,
    supported_owner: bool,
}

struct Index {
    objects: BTreeMap<String, Option<Object>>,
    nets: BTreeMap<String, Option<String>>,
    copper: Result<Vec<String>, &'static str>,
}

fn one<'a>(node: &'a SexpNode, tag: &str) -> Option<&'a SexpNode> {
    let all = node.find_all(tag);
    (all.len() == 1).then(|| all[0])
}

fn scalar<'a>(node: &'a SexpNode, tag: &str) -> Option<&'a str> {
    let value = one(node, tag)?;
    (value.children()?.len() == 2)
        .then(|| value.get(1)?.as_str())
        .flatten()
}

fn id_key(id: &str) -> Option<String> {
    (!id.is_empty() && id.trim() == id).then(|| id.to_ascii_lowercase())
}

fn numeric_id(id: &str) -> Option<String> {
    if id.is_empty() || !id.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let id = id.trim_start_matches('0');
    Some(if id.is_empty() { "0" } else { id }.to_owned())
}

impl Index {
    fn new(source: &str) -> Result<Self, &'static str> {
        let tree = konnect_sexp::parse_sexp(source).map_err(|_| "board_did_not_parse")?;
        if tree.head() != Some("kicad_pcb") {
            return Err("board_root_unproven");
        }
        let mut index = Self {
            objects: BTreeMap::new(),
            nets: BTreeMap::new(),
            copper: copper_layers(&tree),
        };
        for net in tree.find_all("net") {
            let Some(id) = konnect_sexp::net::net_id(net).and_then(numeric_id) else {
                return Err("ambiguous_net_table");
            };
            let name = (net.children().map_or(0, <[_]>::len) == 3)
                .then(|| konnect_sexp::net::net_name(net).map(str::to_owned))
                .flatten();
            match index.nets.entry(id) {
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    entry.insert(None);
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(name);
                }
            }
        }
        index.walk(&tree, None, 0);
        Ok(index)
    }

    fn walk(&mut self, node: &SexpNode, parent: Option<&str>, depth: usize) {
        let ids: Vec<_> = ["uuid", "tstamp"]
            .into_iter()
            .flat_map(|tag| node.find_all(tag))
            .collect();
        let valid_id = ids.len() == 1 && ids[0].children().map_or(0, <[_]>::len) == 2;
        for id in ids
            .iter()
            .filter_map(|n| n.get(1)?.as_str())
            .filter_map(id_key)
        {
            let kind = node.head().unwrap_or_default();
            let supported_owner = matches!(
                (kind, parent, depth),
                ("zone" | "via", Some("kicad_pcb"), 1) | ("pad", Some("footprint" | "module"), 2)
            );
            let value = (valid_id && !self.objects.contains_key(&id)).then(|| Object {
                node: node.clone(),
                kind: kind.into(),
                supported_owner,
            });
            self.objects.insert(id, value);
        }
        for child in node.children().unwrap_or(&[]) {
            if child.children().is_some() {
                self.walk(child, node.head(), depth + 1);
            }
        }
    }

    fn net(&self, item: &SexpNode) -> Result<String, &'static str> {
        let net = one(item, "net").ok_or("net_identity_unresolved")?;
        let fields = net.children().ok_or("net_identity_unresolved")?;
        let name = match fields {
            [_, SexpNode::Str(name)] => name.clone(),
            [_, SexpNode::Atom(id)] | [_, SexpNode::Atom(id), SexpNode::Str(_)] => {
                let id = numeric_id(id).ok_or("net_identity_unresolved")?;
                if id == "0" {
                    return Err("net_identity_unresolved");
                }
                let name = self
                    .nets
                    .get(&id)
                    .and_then(Option::as_ref)
                    .ok_or("net_identity_unresolved")?;
                if fields.len() == 3 && konnect_sexp::net::net_name(net) != Some(name) {
                    return Err("net_identity_inconsistent");
                }
                name.clone()
            }
            _ => return Err("net_identity_unresolved"),
        };
        if name.is_empty() {
            return Err("net_identity_unresolved");
        }
        if !item.find_all("net_name").is_empty() && scalar(item, "net_name") != Some(name.as_str())
        {
            return Err("net_identity_inconsistent");
        }
        Ok(name)
    }

    fn layers(&self, object: &Object) -> Result<BTreeSet<String>, &'static str> {
        let copper = self.copper.as_ref().map_err(|e| *e)?;
        if object.node.find("padstack").is_some() {
            return Err("copper_membership_unsupported");
        }
        // KiCad 10 flashes a `start_end_only` via on its end layers only.
        for tag in ["remove_unused_layers", "keep_end_layers", "start_end_only"] {
            if !object.node.find_all(tag).is_empty() && scalar(&object.node, tag) != Some("no") {
                return Err("copper_membership_unsupported");
            }
        }
        let fields: Vec<_> = ["layer", "layers"]
            .into_iter()
            .flat_map(|tag| object.node.find_all(tag))
            .collect();
        if fields.len() != 1 {
            return Err("copper_membership_unresolved");
        }
        let names = fields[0]
            .children()
            .ok_or("copper_membership_unresolved")?
            .iter()
            .skip(1)
            .map(|n| n.as_str().ok_or("copper_membership_unresolved"))
            .collect::<Result<Vec<_>, _>>()?;
        if names.is_empty() || names.iter().collect::<BTreeSet<_>>().len() != names.len() {
            return Err("copper_membership_unresolved");
        }
        if object.kind == "via" {
            if names.len() != 2 {
                return Err("copper_membership_unresolved");
            }
            let a = copper.iter().position(|l| l == names[0]);
            let b = copper.iter().position(|l| l == names[1]);
            return match (a, b) {
                (Some(a), Some(b)) if a < b => Ok(copper[a..=b].iter().cloned().collect()),
                _ => Err("copper_membership_unresolved"),
            };
        }
        let mut result = BTreeSet::new();
        for name in names {
            if name == "*.Cu" && object.kind == "pad" {
                if object.node.get(2).and_then(SexpNode::as_str) != Some("thru_hole") {
                    return Err("copper_membership_unsupported");
                }
                result.extend(copper.iter().cloned());
            } else if copper.iter().any(|l| l == name) {
                result.insert(name.to_owned());
            } else if name.ends_with(".Cu") || !matches!(object.kind.as_str(), "pad") {
                return Err("copper_membership_unresolved");
            }
        }
        if result.is_empty() {
            Err("copper_membership_unresolved")
        } else {
            Ok(result)
        }
    }
}

fn copper_layers(tree: &SexpNode) -> Result<Vec<String>, &'static str> {
    let table = one(tree, "layers").ok_or("copper_layers_unresolved")?;
    let mut names = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut copper = BTreeMap::new();
    for row in table.children().unwrap_or(&[]).iter().skip(1) {
        let id = row
            .get(0)
            .and_then(SexpNode::as_str)
            .and_then(numeric_id)
            .ok_or("copper_layers_unresolved")?;
        let name = row
            .get(1)
            .and_then(SexpNode::as_str)
            .ok_or("copper_layers_unresolved")?;
        if !ids.insert(id) || !names.insert(name) {
            return Err("copper_layers_unresolved");
        }
        let order = match name {
            "F.Cu" => Some(0),
            "B.Cu" => Some(32),
            _ if name.ends_with(".Cu") => Some(
                name.strip_prefix("In")
                    .and_then(|s| s.strip_suffix(".Cu"))
                    .and_then(|s| s.parse::<u8>().ok())
                    .filter(|n| (1..=30).contains(n))
                    .ok_or("copper_layers_unresolved")?,
            ),
            _ => None,
        };
        if let Some(order) = order {
            if copper.insert(order, name.to_owned()).is_some() {
                return Err("copper_layers_unresolved");
            }
        }
    }
    if copper.is_empty() {
        Err("copper_layers_unresolved")
    } else {
        Ok(copper.into_values().collect())
    }
}

/// Only the English suffix produced by the supported CLI is recognized. Use
/// decimal spelling, not a floating-point epsilon/underflow, to recognize zero.
fn reported_zero(description: &str) -> Result<bool, &'static str> {
    let body = description
        .strip_suffix(')')
        .ok_or("actual_distance_unrecognized")?;
    let (_, actual) = body
        .rsplit_once("; actual ")
        .ok_or("actual_distance_unrecognized")?;
    let (number, unit) = actual
        .split_once(' ')
        .ok_or("actual_distance_unrecognized")?;
    if !matches!(unit, "mm" | "mils" | "in") {
        return Err("actual_distance_unrecognized");
    }
    let number = number.strip_prefix('-').unwrap_or(number);
    if kicad_scientific_nonzero(number) {
        return Ok(false);
    }
    let mut digits = 0;
    let mut dots = 0;
    for c in number.bytes() {
        if c.is_ascii_digit() {
            digits += 1;
        } else if c == b'.' {
            dots += 1;
        } else {
            return Err("actual_distance_unrecognized");
        }
    }
    if digits == 0 || dots > 1 || number.starts_with('.') || number.ends_with('.') {
        return Err("actual_distance_unrecognized");
    }
    Ok(number.bytes().all(|c| matches!(c, b'0' | b'.')))
}

/// KiCad 10.0.1 prints a nonzero value whose fixed format would show only
/// zeros as `%.3e` (`common/eda_units.cpp`). That exact spelling, with a
/// leading digit 1–9, is a definitely nonzero distance; any other
/// scientific form stays unrecognized.
fn kicad_scientific_nonzero(number: &str) -> bool {
    let Some((mantissa, exponent)) = number.split_once('e') else {
        return false;
    };
    let (m, e) = (mantissa.as_bytes(), exponent.as_bytes());
    m.len() == 5
        && matches!(m[0], b'1'..=b'9')
        && m[1] == b'.'
        && m[2..].iter().all(u8::is_ascii_digit)
        && (3..=4).contains(&e.len())
        && matches!(e[0], b'+' | b'-')
        && e[1..].iter().all(u8::is_ascii_digit)
}

#[derive(Clone)]
struct Candidate {
    pair: [String; 2],
    zone: String,
    other: String,
    other_kind: String,
    rule: String,
    basis: &'static str,
    position: Option<cli::ReportPos>,
}

use crate::tools::cli;

fn classify(index: &Index, violation: &DrcViolation) -> Result<Option<Candidate>, &'static str> {
    if !matches!(
        violation.rule.as_str(),
        "clearance" | "hole_clearance" | "shorting_items"
    ) {
        return Ok(None);
    }
    if violation.items.len() != 2 {
        return Err("item_pair_unresolved");
    }
    let mut pair = [String::new(), String::new()];
    let mut objects = Vec::new();
    for (i, item) in violation.items.iter().enumerate() {
        pair[i] = item
            .uuid
            .as_deref()
            .and_then(id_key)
            .ok_or("item_identity_unresolved")?;
        objects.push(
            index
                .objects
                .get(&pair[i])
                .and_then(Option::as_ref)
                .ok_or("item_identity_unresolved")?,
        );
    }
    let (zi, oi) = match (objects[0].kind.as_str(), objects[1].kind.as_str()) {
        ("zone", "pad" | "via") => (0, 1),
        ("pad" | "via", "zone") => (1, 0),
        _ => return Ok(None),
    };
    let (zone, other) = (objects[zi], objects[oi]);
    if !zone.supported_owner || !other.supported_owner || zone.node.find("keepout").is_some() {
        return Err("item_ownership_unsupported");
    }
    let basis = if violation.rule == "shorting_items" {
        "reported_short"
    } else {
        if !reported_zero(&violation.description)? {
            return Ok(None);
        }
        "reported_zero_clearance"
    };
    if index.net(&zone.node)? == index.net(&other.node)? {
        return Ok(None);
    }
    if index.layers(zone)?.is_disjoint(&index.layers(other)?) {
        return Ok(None);
    }
    if other.kind == "pad"
        && !matches!(
            other.node.get(2).and_then(SexpNode::as_str),
            Some("smd" | "thru_hole")
        )
    {
        return Err("pad_copper_unsupported");
    }
    if violation.rule == "hole_clearance" {
        let plated = other.kind == "via"
            || other.node.get(2).and_then(SexpNode::as_str) == Some("thru_hole");
        let drilled = one(&other.node, "drill")
            .and_then(|n| n.get_f64(1))
            .is_some_and(|d| d.is_finite() && d > 0.0);
        if !plated || !drilled {
            return Err("plated_hole_unproven");
        }
    }
    let candidate = Candidate {
        zone: pair[zi].clone(),
        other: pair[oi].clone(),
        other_kind: other.kind.clone(),
        pair: {
            pair.sort();
            pair
        },
        rule: violation.rule.clone(),
        basis,
        position: violation.pos,
    };
    Ok(Some(candidate))
}

struct Inspection {
    candidates: Vec<Candidate>,
    unresolved: BTreeMap<&'static str, usize>,
    unavailable: Option<String>,
    provenance: &'static str,
    /// Report-level gaps: the whole report may be missing entries.
    partial: BTreeSet<&'static str>,
    rules_at_limit: BTreeSet<&'static str>,
    rules_ignored: BTreeSet<&'static str>,
    excluded_ids: BTreeSet<String>,
}

impl Inspection {
    fn new(
        report: &DrcReport,
        snapshot: Result<&Snapshot, &str>,
        provenance: &'static str,
    ) -> Self {
        let mut result = Self {
            candidates: Vec::new(),
            unresolved: BTreeMap::new(),
            unavailable: None,
            provenance,
            partial: BTreeSet::new(),
            rules_at_limit: BTreeSet::new(),
            rules_ignored: BTreeSet::new(),
            excluded_ids: BTreeSet::new(),
        };
        let snapshot = match snapshot {
            Ok(snapshot) => snapshot,
            Err(reason) => {
                result.unavailable = Some(reason.to_owned());
                return result;
            }
        };
        let index = match Index::new(&snapshot.board) {
            Ok(index) => index,
            Err(reason) => {
                result.unavailable = Some(reason.to_owned());
                return result;
            }
        };
        let project = snapshot.project_rules().unwrap_or_else(|reason| {
            result.partial.insert(reason);
            ProjectRules::default()
        });
        for rule in SUPPORTED_RULES {
            if project.ignored.contains(rule) {
                result.rules_ignored.insert(rule);
                result.partial.insert("kicad_rule_ignored");
            }
            let written = report.violations.iter().filter(|v| v.rule == rule).count();
            let excluded = project.excluded_per_rule.get(rule).copied().unwrap_or(0);
            if kicad_rule_limit(rule).is_some_and(|limit| written + excluded >= limit) {
                result.rules_at_limit.insert(rule);
                result.partial.insert("kicad_rule_limit_reached");
            }
        }
        result.excluded_ids = project.excluded_ids;
        for violation in &report.violations {
            match classify(&index, violation) {
                Ok(Some(candidate)) => result.candidates.push(candidate),
                Ok(None) => {}
                Err(reason) => *result.unresolved.entry(reason).or_default() += 1,
            }
        }
        result
    }

    fn pairs(&self) -> BTreeSet<[String; 2]> {
        self.candidates.iter().map(|c| c.pair.clone()).collect()
    }

    fn complete(&self) -> bool {
        self.unavailable.is_none() && self.unresolved.is_empty() && self.partial.is_empty()
    }

    fn summary(&self) -> Value {
        if let Some(reason) = &self.unavailable {
            return json!({"inspected": false, "provenance": self.provenance,
                "matching_entries": null, "distinct_pairs": null, "unresolved_candidates": null,
                "coverage": "unavailable", "reason": bounded(reason)});
        }
        json!({"inspected": true, "provenance": self.provenance,
            "matching_entries": self.candidates.len(), "distinct_pairs": self.pairs().len(),
            "unresolved_candidates": self.unresolved.values().sum::<usize>(),
            "coverage": if self.complete() {"complete"} else {"partial"},
            "unresolved_reasons": self.unresolved,
            "partial_reasons": self.partial, "rules_at_kicad_limit": self.rules_at_limit,
            "rules_ignored_by_project": self.rules_ignored})
    }
}

fn bounded(text: &str) -> String {
    text.chars().take(TEXT_LIMIT).collect()
}

fn diagnostic(
    primary: &Inspection,
    comparison: Option<&Inspection>,
    attribution: Result<BTreeSet<[String; 2]>, &'static str>,
) -> Value {
    // Newly reported examples come first, so the cap cannot hide them.
    let compared: &[Candidate] = comparison.map_or(&[], |i| i.candidates.as_slice());
    let is_new = |c: &Candidate| {
        attribution
            .as_ref()
            .is_ok_and(|pairs| pairs.contains(&c.pair))
    };
    let candidates = compared
        .iter()
        .filter(|c| is_new(c))
        .map(|c| ("comparison", c))
        .chain(primary.candidates.iter().map(|c| ("primary", c)))
        .chain(
            compared
                .iter()
                .filter(|c| !is_new(c))
                .map(|c| ("comparison", c)),
        );
    let total = primary.candidates.len() + compared.len();
    // A comparison that was attempted and failed is missing evidence, not a
    // deliberate skip.
    let comparison_failed = comparison.is_none() && attribution == Err("comparison_failed");
    let not_compared_reason = match &attribution {
        Err(reason) if comparison.is_none() => *reason,
        _ => "not_compared",
    };
    let newly_reported = attribution.as_ref().ok().map(|pairs| {
        comparison.map_or(0, |i| {
            i.candidates
                .iter()
                .filter(|c| pairs.contains(&c.pair))
                .count()
        })
    });
    let findings: Vec<Value> = candidates.take(EXAMPLES).map(|(phase, c)| json!({
        "report": phase, "rule": c.rule, "basis": c.basis,
        "zone_uuid": bounded(&c.zone), "other_uuid": bounded(&c.other), "other_kind": c.other_kind,
        "identifiers_truncated": c.zone.chars().count() > TEXT_LIMIT || c.other.chars().count() > TEXT_LIMIT,
        "reported_item_position": c.position,
        "refill_attribution": match &attribution {
            Ok(pairs) if phase == "comparison" && pairs.contains(&c.pair) => "newly_reported_after_refill",
            Ok(_) if phase == "comparison" => "already_reported",
            _ => "unproven",
        }
    })).collect();
    let mut result = json!({
        "status": if total > 0 {"suspected"} else if !primary.complete() || comparison_failed || comparison.is_some_and(|i| !i.complete()) {"incomplete"} else {"none_detected"},
        "primary_report": primary.summary(),
        "comparison_report": comparison.map(Inspection::summary).unwrap_or_else(|| json!({"inspected": false,
            "matching_entries": null, "distinct_pairs": null, "unresolved_candidates": null,
            "coverage": "unavailable", "reason": not_compared_reason})),
        "newly_reported_after_refill": newly_reported, "attribution_reason": attribution.err(),
        "findings": findings, "truncated": total > EXAMPLES,
        "hint": if total == 0 {
            "This checks a limited reported-zero/short signature; it does not establish a clean board."
        } else {
            "Suspected zone-fill issue; engine cause and routing correctness are unconfirmed. Do not plot until DRC is clean. See TROUBLESHOOTING.md for the KiCad iterative-refill workaround."
        }
    });
    while result.to_string().len() > DIAGNOSTIC_BYTES {
        let findings = result["findings"]
            .as_array_mut()
            .expect("diagnostic findings array");
        if findings.pop().is_none() {
            break;
        }
        result["truncated"] = json!(true);
    }
    result
}

/// Preserve strings and exact decimal precision. Scientific notation and
/// unfamiliar numeric syntax stay byte-spelled rather than being rounded.
fn decimal(atom: &str) -> String {
    let (negative, value) = atom.strip_prefix('-').map_or((false, atom), |s| (true, s));
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || parts.next().is_some()
        || fraction.is_some_and(|f| f.is_empty() || !f.bytes().all(|c| c.is_ascii_digit()))
    {
        return atom.to_owned();
    }
    let whole = whole.trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let fraction = fraction.unwrap_or_default().trim_end_matches('0');
    let sign = if negative && (whole != "0" || !fraction.is_empty()) {
        "-"
    } else {
        ""
    };
    if fraction.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{fraction}")
    }
}

fn numeric_field(head: &str) -> bool {
    matches!(
        head,
        "at" | "xy"
            | "xyz"
            | "start"
            | "mid"
            | "end"
            | "size"
            | "drill"
            | "width"
            | "thickness"
            | "offset"
            | "priority"
            | "clearance"
            | "min_thickness"
            | "thermal_gap"
            | "thermal_bridge_width"
            | "island_area_min"
            | "hatch_thickness"
            | "hatch_gap"
            | "hatch_orientation"
            | "radius"
            | "angle"
            | "solder_mask_margin"
            | "solder_paste_margin"
            | "solder_paste_margin_ratio"
            | "zone_connect"
    )
}

fn movable_field(parent: &str, head: &str) -> bool {
    match parent {
        "kicad_pcb" => matches!(
            head,
            "footprint"
                | "module"
                | "segment"
                | "arc"
                | "via"
                | "zone"
                | "gr_line"
                | "gr_arc"
                | "gr_rect"
                | "gr_circle"
                | "gr_poly"
                | "gr_text"
                | "group"
        ),
        "footprint" | "module" => {
            matches!(
                head,
                "pad"
                    | "property"
                    | "fp_text"
                    | "fp_line"
                    | "fp_arc"
                    | "fp_rect"
                    | "fp_circle"
                    | "fp_poly"
                    | "zone"
            ) || singleton_field(head)
        }
        "pad" | "via" | "segment" | "arc" | "zone" | "fill" => singleton_field(head),
        _ => false,
    }
}

fn singleton_field(head: &str) -> bool {
    numeric_field(head)
        || matches!(
            head,
            "uuid"
                | "tstamp"
                | "net"
                | "net_name"
                | "layer"
                | "layers"
                | "fill"
                | "name"
                | "locked"
                | "remove_unused_layers"
                | "keep_end_layers"
                | "connect_pads"
                | "filled_areas_thickness"
                | "hatch"
                | "island_removal_mode"
        )
}

fn normalize(node: &SexpNode) -> Result<SexpNode, &'static str> {
    let head = node.head().unwrap_or_default();
    let Some(children) = node.children() else {
        return Ok(node.clone());
    };
    if numeric_field(head) {
        return Ok(SexpNode::List(
            children
                .iter()
                .map(|c| match c {
                    SexpNode::Atom(s) => SexpNode::Atom(decimal(s)),
                    _ => c.clone(),
                })
                .collect(),
        ));
    }
    if !matches!(
        head,
        "kicad_pcb"
            | "footprint"
            | "module"
            | "pad"
            | "via"
            | "segment"
            | "arc"
            | "zone"
            | "fill"
            | "polygon"
            | "filled_polygon"
            | "pts"
            | "connect_pads"
    ) {
        return Ok(node.clone());
    }
    let zone = head == "zone" && node.find("keepout").is_none();
    let mut kept = Vec::new();
    let mut moved = Vec::new();
    let mut flags = 0;
    for child in children {
        let tag = child.head().unwrap_or_default();
        if zone && tag == "filled_polygon" {
            continue;
        }
        if matches!(head, "kicad_pcb" | "footprint" | "module")
            && matches!(tag, "generator" | "generator_version")
        {
            continue;
        }
        if matches!(head, "footprint" | "module") && tag == "version" {
            continue;
        }
        if head == "fill" && matches!(child, SexpNode::Atom(s) if s == "yes" || s == "no") {
            flags += 1;
            if flags > 1 {
                return Err("ambiguous_fill_state");
            }
            continue;
        }
        let normalized = normalize(child)?;
        let collection =
            matches!(head, "kicad_pcb" | "footprint" | "module") && !singleton_field(tag);
        if movable_field(head, tag) && (collection || node.find_all(tag).len() == 1) {
            moved.push(normalized);
        } else {
            kept.push(normalized);
        }
    }
    moved.sort_by_cached_key(|n| format!("{n:?}"));
    kept.extend(moved);
    Ok(SexpNode::List(kept))
}

/// Point multisets deliberately lose topology: an equal multiset is not proof
/// of equal copper, so a topology-only change cannot earn refill attribution.
/// They do avoid mistaking ring starting-point/winding/order changes for added
/// copper. The reported finding remains visible when attribution is unavailable.
fn fill_signature(object: &Object) -> Result<Vec<String>, &'static str> {
    let mut result = Vec::new();
    for polygon in object.node.find_all("filled_polygon") {
        let layer = scalar(polygon, "layer")
            .or_else(|| scalar(&object.node, "layer"))
            .ok_or("fill_geometry_unresolved")?;
        let points = one(polygon, "pts").ok_or("fill_geometry_unresolved")?;
        let mut coordinates = Vec::new();
        for point in points.children().unwrap_or(&[]).iter().skip(1) {
            if point.head() != Some("xy") || point.children().map_or(0, <[_]>::len) != 3 {
                return Err("fill_geometry_unresolved");
            }
            coordinates.push(format!("{:?}", normalize(point)?));
        }
        if coordinates.len() < 3 {
            return Err("fill_geometry_unresolved");
        }
        coordinates.sort();
        result.push(format!("{layer:?}:{coordinates:?}"));
    }
    result.sort();
    Ok(result)
}

fn attribute(
    source: Result<&Snapshot, &str>,
    primary: &Inspection,
    comparison: &Inspection,
    evidence: &Comparison,
) -> Result<BTreeSet<[String; 2]>, &'static str> {
    let source = source.map_err(|_| "primary_source_unproven")?;
    let before = evidence
        .before
        .as_ref()
        .map_err(|_| "comparison_input_unavailable")?;
    let after = evidence
        .after
        .as_ref()
        .map_err(|_| "comparison_output_unavailable")?;
    if source != before {
        return Err("comparison_input_differs");
    }
    // Each report-level gap is its own named refusal.
    if let Some(reason) = primary.partial.iter().chain(&comparison.partial).next() {
        return Err(*reason);
    }
    if !primary.complete() || !comparison.complete() {
        return Err("incomplete_report_evidence");
    }
    if before.sidecars != after.sidecars {
        return Err("project_or_rules_changed");
    }
    let first = Index::new(&before.board)?;
    let second = Index::new(&after.board)?;
    for index in [&first, &second] {
        if index.objects.values().any(Option::is_none) {
            return Err("ambiguous_board_identity");
        }
    }
    let before_tree = konnect_sexp::parse_sexp(&before.board).map_err(|_| "board_did_not_parse")?;
    let after_tree = konnect_sexp::parse_sexp(&after.board).map_err(|_| "board_did_not_parse")?;
    // A rule area (keepout) has no fill and is not refilled, so neither check
    // counts it, like `zone_fill_check`.
    for tree in [&before_tree, &after_tree] {
        if ["footprint", "module"]
            .into_iter()
            .flat_map(|tag| tree.find_all(tag))
            .any(|footprint| {
                footprint
                    .find_all("zone")
                    .iter()
                    .any(|zone| zone.find("keepout").is_none())
            })
        {
            return Err("footprint_zone_comparison_unsupported");
        }
    }
    for index in [&first, &second] {
        if index
            .objects
            .values()
            .flatten()
            .any(|o| o.kind == "zone" && !o.supported_owner && o.node.find("keepout").is_none())
        {
            return Err("zone_owner_unsupported");
        }
    }
    if normalize(&before_tree)? != normalize(&after_tree)? {
        return Err("non_fill_design_changed");
    }
    // Only the indexes are needed from here; free the parsed boards early.
    drop((before_tree, after_tree));
    let old = primary.pairs();
    let mut introduced = BTreeSet::new();
    for candidate in &comparison.candidates {
        if old.contains(&candidate.pair) {
            continue;
        }
        let before_zone = first
            .objects
            .get(&candidate.zone)
            .and_then(Option::as_ref)
            .ok_or("zone_identity_changed")?;
        let after_zone = second
            .objects
            .get(&candidate.zone)
            .and_then(Option::as_ref)
            .ok_or("zone_identity_changed")?;
        if fill_signature(before_zone)? == fill_signature(after_zone)? {
            return Err("implicated_fill_change_unproven");
        }
        // An excluded marker is absent from the primary report and can
        // resurface after refill under a moved collision point.
        if candidate
            .pair
            .iter()
            .any(|id| primary.excluded_ids.contains(id))
        {
            return Err("drc_exclusion_names_pair");
        }
        introduced.insert(candidate.pair.clone());
    }
    Ok(introduced)
}

pub(super) async fn finish(
    initial: Result<Snapshot, String>,
    board: &Path,
    report: &DrcReport,
    provenance: &'static str,
    comparison: Option<Result<Comparison, &'static str>>,
) -> Value {
    let current = Snapshot::capture(board).await;
    let source = match (&initial, &current) {
        (Ok(before), Ok(after)) if before == after => Ok(before),
        (Err(reason), _) => Err(reason.as_str()),
        _ => Err("source_changed_or_unavailable"),
    };
    let primary = Inspection::new(report, source, provenance);
    let (compared, attribution) = match &comparison {
        None => (None, Err("not_compared")),
        Some(Err(reason)) => (None, Err(*reason)),
        Some(Ok(evidence)) => {
            let compared = Inspection::new(
                &evidence.report,
                evidence.after.as_ref().map_err(String::as_str),
                "private_copy_refill",
            );
            let attribution = attribute(source, &primary, &compared, evidence);
            (Some(compared), attribution)
        }
    };
    diagnostic(&primary, compared.as_ref(), attribution)
}
