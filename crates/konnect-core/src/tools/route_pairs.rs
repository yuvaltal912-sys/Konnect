//! The differential pairs of a fastroute call (docs/ROUTE_SEARCH_DESIGN.md, D23).
//!
//! The pairs come from the design intent alone (`interfaces[].differential_pairs` of
//! `<project>/.konnect/design_intent.json`), never from net names. Each name is
//! resolved against the nets of the exported DSN, because fastroute matches names
//! as the DSN writes them. A pair that cannot be resolved is left out and reported.
//! The gap of a pair is the `diff_pair_gap` of the positive net's netclass.

use crate::specctra;
use crate::tools::design_intent::{
    design_intent_path, read_design_intent, DesignIntent, IntentRead,
};
use crate::tools::layout_quality::netclasses_from_source;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The pairs file's name in the call's folder.
pub(crate) const PAIRS_FILE: &str = "pairs.txt";

/// How many nets an "ambiguous" reason lists before it says "and N more".
const LISTED_MATCHES: usize = 3;

/// A pair as the design intent names it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DeclaredPair {
    pub positive: String,
    pub negative: String,
}

/// A pair that fastroute routes coupled.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedPair {
    pub positive: String,
    pub negative: String,
    /// The names as the DSN writes them, which the pairs file holds.
    pub dsn_positive: String,
    pub dsn_negative: String,
    /// The copper gap in mm, none when the netclass sets none (fastroute then uses
    /// the clearance).
    pub gap_mm: Option<f64>,
}

/// A pair left out, and why it is routed as two ordinary nets.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UnresolvedPair {
    pub positive: String,
    pub negative: String,
    pub reason: String,
}

/// What a call does about differential pairs.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PairsPlan {
    pub resolved: Vec<ResolvedPair>,
    pub unresolved: Vec<UnresolvedPair>,
    /// Set when the design intent exists and cannot be used: no pair was read.
    pub note: Option<String>,
}

impl PairsPlan {
    /// The `--pairs` file's text, one line per resolved pair in the intent's order,
    /// or `None` without a pair (then no switch is sent).
    pub(crate) fn file_text(&self) -> Option<String> {
        if self.resolved.is_empty() {
            return None;
        }
        let mut text = String::new();
        for pair in &self.resolved {
            text.push_str(&format!("pair {} {}", pair.dsn_positive, pair.dsn_negative));
            if let Some(gap) = pair.gap_mm {
                text.push_str(&format!(" gap={gap}"));
            }
            text.push('\n');
        }
        Some(text)
    }

    /// The result's fields: `differential_pairs`, `differential_pairs_unresolved`
    /// (both lists, empty without pairs) and, only when the intent could not be
    /// used, `differential_pairs_note`.
    pub(crate) fn fields(&self) -> Vec<(&'static str, Value)> {
        let pairs: Vec<Value> = self
            .resolved
            .iter()
            .map(|pair| {
                json!({
                    "positive": pair.positive,
                    "negative": pair.negative,
                    "dsn_positive": pair.dsn_positive,
                    "dsn_negative": pair.dsn_negative,
                    "gap_mm": pair.gap_mm,
                })
            })
            .collect();
        let unresolved: Vec<Value> = self
            .unresolved
            .iter()
            .map(|pair| {
                json!({
                    "positive": pair.positive,
                    "negative": pair.negative,
                    "reason": pair.reason,
                })
            })
            .collect();
        let mut fields = vec![
            ("differential_pairs", json!(pairs)),
            ("differential_pairs_unresolved", json!(unresolved)),
        ];
        if let Some(note) = &self.note {
            fields.push(("differential_pairs_note", json!(note)));
        }
        fields
    }
}

/// Every pair of every interface, in file order.
pub(crate) fn declared_pairs(intent: &DesignIntent) -> Vec<DeclaredPair> {
    intent
        .interfaces
        .iter()
        .flat_map(|interface| &interface.differential_pairs)
        .map(|pair| DeclaredPair {
            positive: pair.positive.clone(),
            negative: pair.negative.clone(),
        })
        .collect()
}

/// The DSN net a name stands for: the net with exactly that name, else the one net
/// whose name ends with `/` and the name (the sheet path KiCad puts in front of a net
/// of a sub-sheet). `Err` says why there is none.
pub(crate) fn resolve_name(name: &str, nets: &[String]) -> Result<String, String> {
    if nets.iter().any(|net| net == name) {
        return Ok(name.to_string());
    }
    let suffix = format!("/{name}");
    let mut matches: Vec<&String> = nets.iter().filter(|net| net.ends_with(&suffix)).collect();
    matches.sort();
    match matches.as_slice() {
        [] => Err(format!(
            "no net of the DSN is named '{name}' or ends with '{suffix}'"
        )),
        [only] => Ok((*only).clone()),
        many => {
            let shown: Vec<&str> = many
                .iter()
                .take(LISTED_MATCHES)
                .map(|net| net.as_str())
                .collect();
            let more = many.len().saturating_sub(LISTED_MATCHES);
            Err(format!(
                "'{name}' is ambiguous: {} DSN nets end with '{suffix}' ({}{})",
                many.len(),
                shown.join(", "),
                if more > 0 {
                    format!(", and {more} more")
                } else {
                    String::new()
                }
            ))
        }
    }
}

/// `diff_pair_gap` (mm) of the netclass of `net`, as KiCad resolves it: the classes
/// that name the net (an assignment or a pattern) in the order of their `priority`
/// (the lowest number first, equal priorities by name), then `Default`; the first
/// that sets the value decides (`NET_SETTINGS::makeEffectiveNetclass`). `None` when
/// that value is 0 (KiCad's "not set") or no class sets one.
pub(crate) fn gap_mm(project_text: &str, net: &str) -> Option<f64> {
    let project: Value = serde_json::from_str(project_text).ok()?;
    let classes = project["net_settings"]["classes"].as_array()?;
    let named = BTreeSet::from([net.to_string()]);
    let mut order: Vec<String> = netclasses_from_source(project_text, &named)
        .remove(net)
        .unwrap_or_default();
    let priority = |name: &str| {
        classes
            .iter()
            .find(|class| class["name"].as_str() == Some(name))
            .and_then(|class| class["priority"].as_i64())
            .unwrap_or(i64::MAX)
    };
    order.sort_by(|a, b| (priority(a), a).cmp(&(priority(b), b)));
    order.push("Default".to_string());
    for name in &order {
        let Some(class) = classes
            .iter()
            .find(|class| class["name"].as_str() == Some(name.as_str()))
        else {
            continue;
        };
        if let Some(gap) = class["diff_pair_gap"].as_f64() {
            return (gap > 0.0).then_some(gap);
        }
    }
    None
}

/// A name a pairs file can hold: one token.
fn writable(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Resolves `declared` against `nets` (the DSN's net names), in order. A pair is left
/// out, with its reason, when a name resolves to no net or to several, when both names
/// are one net, when a resolved name is not one token (a pairs file is split at white
/// space), or when a net is already in an earlier pair. `project_text` is the
/// project's `.kicad_pro`, for the gaps.
pub(crate) fn resolve_pairs(
    declared: &[DeclaredPair],
    nets: &[String],
    project_text: Option<&str>,
) -> PairsPlan {
    let mut plan = PairsPlan::default();
    for pair in declared {
        let left_out = |reason: String| UnresolvedPair {
            positive: pair.positive.clone(),
            negative: pair.negative.clone(),
            reason,
        };
        let positive = resolve_name(&pair.positive, nets);
        let negative = resolve_name(&pair.negative, nets);
        let (dsn_positive, dsn_negative) = match (positive, negative) {
            (Ok(positive), Ok(negative)) => (positive, negative),
            (positive, negative) => {
                let reasons: Vec<String> = [positive, negative]
                    .into_iter()
                    .filter_map(Result::err)
                    .collect();
                plan.unresolved.push(left_out(reasons.join("; ")));
                continue;
            }
        };
        if dsn_positive == dsn_negative {
            plan.unresolved.push(left_out(format!(
                "both names are the one DSN net '{dsn_positive}'"
            )));
            continue;
        }
        if let Some(name) = [&dsn_positive, &dsn_negative]
            .into_iter()
            .find(|name| !writable(name))
        {
            plan.unresolved.push(left_out(format!(
                "the DSN net name {name:?} is not one token, which a pairs file needs"
            )));
            continue;
        }
        let taken = plan.resolved.iter().find(|earlier| {
            [&dsn_positive, &dsn_negative]
                .iter()
                .any(|name| **name == earlier.dsn_positive || **name == earlier.dsn_negative)
        });
        if let Some(earlier) = taken {
            plan.unresolved.push(left_out(format!(
                "a net of it is already in the pair '{}' and '{}', and a net is in one pair only",
                earlier.dsn_positive, earlier.dsn_negative
            )));
            continue;
        }
        let gap_mm = project_text.and_then(|text| gap_mm(text, &dsn_positive));
        plan.resolved.push(ResolvedPair {
            positive: pair.positive.clone(),
            negative: pair.negative.clone(),
            dsn_positive,
            dsn_negative,
            gap_mm,
        });
    }
    plan
}

/// Writes the plan's pairs file to `<folder>/pairs.txt` and returns its path, or
/// `None` (and writes nothing) when the plan has no resolved pair.
pub(crate) fn write_file(folder: &Path, plan: &PairsPlan) -> anyhow::Result<Option<PathBuf>> {
    let Some(text) = plan.file_text() else {
        return Ok(None);
    };
    let path = folder.join(PAIRS_FILE);
    std::fs::write(&path, text)
        .map_err(|error| anyhow::anyhow!("write {}: {error}", path.display()))?;
    Ok(Some(path))
}

/// The plan of a call for the board the DSN was exported from: the design intent in
/// the board's folder, the DSN's nets, and the board's `.kicad_pro` for the gaps.
/// Without an intent file, or with an intent that has no pair, the plan is empty and
/// the call is as it was before pairs existed. An intent that exists and cannot be
/// used gives an empty plan with a `note`.
pub(crate) fn plan(board: &Path, dsn_text: &str) -> PairsPlan {
    let folder = match board.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let path = design_intent_path(folder);
    let declared = match read_design_intent(&path) {
        IntentRead::Absent => return PairsPlan::default(),
        IntentRead::Unusable { reason } => {
            return PairsPlan {
                note: Some(format!(
                    "the design intent {} cannot be used ({reason}), so no pair was read from it",
                    path.display()
                )),
                ..PairsPlan::default()
            }
        }
        IntentRead::Loaded { intent, .. } => declared_pairs(&intent),
    };
    if declared.is_empty() {
        return PairsPlan::default();
    }
    match specctra::dsn_net_names(dsn_text) {
        Ok(nets) => {
            let project = std::fs::read_to_string(board.with_extension("kicad_pro")).ok();
            resolve_pairs(&declared, &nets, project.as_deref())
        }
        Err(error) => PairsPlan {
            unresolved: declared
                .iter()
                .map(|pair| UnresolvedPair {
                    positive: pair.positive.clone(),
                    negative: pair.negative.clone(),
                    reason: format!("the DSN's nets could not be read ({error:#})"),
                })
                .collect(),
            ..PairsPlan::default()
        },
    }
}

#[cfg(test)]
#[path = "route_pairs_tests.rs"]
mod tests;
