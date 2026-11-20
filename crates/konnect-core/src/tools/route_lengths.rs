//! The length groups of a fastroute call (docs/LENGTH_TUNING_DESIGN.md, Row LT).
//!
//! The groups come from the design intent alone (`interfaces[].length_groups`, and
//! `differential_pairs[].max_skew_mm` for a pair's own skew), never from KiCad's
//! custom rules, which KiCad's own DRC checks on every judged run (L3). Each name is
//! resolved against the exported DSN's nets as D23 resolves a pair's; a group that
//! cannot be applied refuses the call before anything is routed (L4). The plan
//! writes the `--tune` file (L5), carries the layer heights (L7), and gives the
//! judge what it checks (`length_probe::LengthChecks`).

use crate::custom_rules;
use crate::layer_heights::{self, LayerHeights};
use crate::length_probe::{
    CheckedGroup, CheckedPair, GroupLengths, LengthChecks, LengthMeasures, LENGTH_RULE_KEYS,
};
use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::CallToolResult;
use crate::specctra;
use crate::tools::design_intent::{
    design_intent_path, read_design_intent, DesignIntent, IntentRead,
};
use crate::tools::route_pairs::{self, PairsPlan};
use konnect_sexp::board_geometry::{BoardGeometry, Kind};
use konnect_sexp::{net as sexp_net, parse_sexp, SexpNode};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The tune file's name in the call's folder.
pub(crate) const TUNE_FILE: &str = "tune.txt";

/// fastroute's own `--pair-skew` default, mm: the tolerance of a trailing pair group
/// for a pair whose intent gives no `max_skew_mm` (L5, L13).
pub(crate) const FASTROUTE_PAIR_SKEW_MM: f64 = 0.1;

/// The share of an intent tolerance fastroute is given (L5, L13): fastroute leaves
/// alone every net inside its own window, so the rest is the margin for the gap
/// between its measure and KiCad's.
pub(crate) const FASTROUTE_WINDOW_SHARE: f64 = 0.5;

/// What a net name in the tune file or a probe condition may not hold (L4):
/// fastroute splits a tune line at white space, cuts it at `#` and reads `*` as a
/// wildcard; KiCad's `==` reads `*` and `?` as wildcards, and a quote or a backslash
/// would end or escape the condition's string.
pub(crate) const UNSAFE_NAME_CHARS: &[char] = &['#', '*', '?', '\'', '"', '\\'];

/// The prefix of the KiCad rules the next steps suggest (L16); the rule of group
/// `lt1` is `1kopper_len_lt1`.
const RULE_PREFIX: &str = "1kopper_";

/// L14: what the Freerouting path says when the intent declares lengths.
pub(crate) const FREEROUTING_NOTE: &str = "Freerouting does not tune lengths; the groups were neither tuned nor checked; route with engine fastroute";

/// A group as the design intent declares it, with its interface's name.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DeclaredGroup {
    pub interface: String,
    pub name: String,
    pub nets: Vec<String>,
    pub tolerance_mm: f64,
    pub target_mm: Option<f64>,
}

/// A pair's declared skew, as the intent names its nets.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DeclaredSkew {
    pub positive: String,
    pub negative: String,
    pub max_skew_mm: f64,
}

/// What a group's nets must meet, in KiCad's length (L2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Window {
    /// Every net within `tolerance_mm` of the group's longest net.
    Skew { tolerance_mm: f64 },
    /// Every net within `target_mm` ± `tolerance_mm`.
    Target { target_mm: f64, tolerance_mm: f64 },
}

impl Window {
    /// The group's `target=` (none in skew mode) and `tolerance=` in the tune file
    /// (L5), with s = [`FASTROUTE_WINDOW_SHARE`]: skew `(None, s·T)`; target
    /// `(Some(L + s·T), 2s·T)`. fastroute lengthens every net below its target −
    /// tolerance and aims at target − tolerance / 2, so in target mode it lengthens
    /// every net below `L − s·T` and aims at `L`.
    pub(crate) fn fastroute_values(&self) -> (Option<f64>, f64) {
        let share = FASTROUTE_WINDOW_SHARE;
        match *self {
            Window::Skew { tolerance_mm } => (None, share * tolerance_mm),
            Window::Target {
                target_mm,
                tolerance_mm,
            } => (
                Some(target_mm + share * tolerance_mm),
                2.0 * share * tolerance_mm,
            ),
        }
    }

    /// The range a net's KiCad length must lie in, mm, both ends inclusive: skew
    /// `[longest − T, longest]`, target `[L − T, L + T]`. `longest_mm` is the group's
    /// longest measured net (read only in skew mode).
    pub(crate) fn range_mm(&self, longest_mm: f64) -> (f64, f64) {
        match *self {
            Window::Skew { tolerance_mm } => (longest_mm - tolerance_mm, longest_mm),
            Window::Target {
                target_mm,
                tolerance_mm,
            } => (target_mm - tolerance_mm, target_mm + tolerance_mm),
        }
    }

    /// The window as the result carries it.
    fn json(&self) -> Value {
        match *self {
            Window::Skew { tolerance_mm } => {
                json!({ "mode": "skew", "tolerance_mm": tolerance_mm })
            }
            Window::Target {
                target_mm,
                tolerance_mm,
            } => json!({ "mode": "target", "target_mm": target_mm, "tolerance_mm": tolerance_mm }),
        }
    }
}

/// A net of a group: its intent name and its DSN name, which is also the board's
/// (the SES importer maps names one to one).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GroupNet {
    pub intent: String,
    pub dsn: String,
}

/// A group fastroute tunes and the judge checks.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedGroup {
    pub interface: String,
    pub name: String,
    /// `lt<N>`, N from 1 in intent order: the only group name the tune file holds.
    pub file_name: String,
    pub nets: Vec<GroupNet>,
    pub window: Window,
    /// Each resolved pair whose two nets are in this group: DSN names and the
    /// tolerance of its trailing pair group ([`fastroute_skew_mm`] of its
    /// `max_skew_mm`, else [`FASTROUTE_PAIR_SKEW_MM`]).
    pub pairs: Vec<(String, String, f64)>,
}

/// A pair whose skew the judge checks (it has `max_skew_mm`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GatedPair {
    pub positive: String,
    pub negative: String,
    pub dsn_positive: String,
    pub dsn_negative: String,
    pub max_skew_mm: f64,
    /// The group both nets are in, if any (intent name).
    pub group: Option<String>,
}

/// A `length` or `skew` rule of the board's own `.kicad_dru`: listed, never
/// evaluated (L3).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct KicadLengthRule {
    pub name: String,
    /// Each `length` or `skew` constraint as the file writes it, white space folded.
    pub constraints: Vec<String>,
    /// The rule's condition text as written, when it has one.
    pub condition: Option<String>,
}

/// Why the call is refused before routing, with the step that fixes it (L4, L12).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LengthRefusal {
    pub reason: String,
    pub next_step: String,
}

/// What a call does about lengths.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LengthPlan {
    pub groups: Vec<ResolvedGroup>,
    pub pairs: Vec<GatedPair>,
    pub heights: LayerHeights,
    pub kicad_rules: Vec<KicadLengthRule>,
    /// Why the board's `.kicad_dru` could not be listed, when it exists and does not
    /// parse.
    pub kicad_rules_note: Option<String>,
    /// Set when the design intent exists and cannot be used: nothing was read.
    pub note: Option<String>,
}

/// A number in mm for a file or a rule: rounded to the nanometre, without trailing
/// zeros (`0.25`, `42`), so that float noise (`42.07000000000001`) never reaches a
/// file fastroute or KiCad reads.
fn mm_text(mm: f64) -> String {
    let text = format!("{mm:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// A number for the result, rounded to the nanometre.
fn round_mm(mm: f64) -> f64 {
    (mm * 1_000_000.0).round() / 1_000_000.0
}

impl LengthPlan {
    /// True when nothing is tuned or checked from the intent (no group, no gated
    /// pair): then there is no tune file, no Tune pass and no probe.
    pub(crate) fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.pairs.is_empty()
    }

    /// The `--tune` file (L5), or `None` without a group: a header comment, then
    /// each group as `group lt<N> tolerance=<t>[ target=<l>]` (`t` and `l` from
    /// `Window::fastroute_values`) with its DSN nets on indented lines, then each
    /// group's trailing pair groups (`group lt<N>p<K> tolerance=<skew>` with the
    /// pair's two nets). Numbers in mm without a trailing `.0`. No intent text
    /// appears in the file.
    pub(crate) fn tune_file_text(&self) -> Option<String> {
        if self.groups.is_empty() {
            return None;
        }
        let mut text = String::from(
            "# Length groups for fastroute --tune, written by Konnect (DSN net names only).\n",
        );
        for group in &self.groups {
            let (target, tolerance) = group.window.fastroute_values();
            text.push_str(&format!(
                "group {} tolerance={}",
                group.file_name,
                mm_text(tolerance)
            ));
            if let Some(target) = target {
                text.push_str(&format!(" target={}", mm_text(target)));
            }
            text.push('\n');
            for net in &group.nets {
                text.push_str(&format!("  {}\n", net.dsn));
            }
        }
        // fastroute processes groups in file order, so a pair's skew is restored
        // after the group's tuning has lengthened each of its nets on its own.
        for group in &self.groups {
            for (index, (positive, negative, tolerance)) in group.pairs.iter().enumerate() {
                text.push_str(&format!(
                    "group {}p{} tolerance={}\n  {}\n  {}\n",
                    group.file_name,
                    index + 1,
                    mm_text(*tolerance),
                    positive,
                    negative
                ));
            }
        }
        Some(text)
    }

    /// The DSN names of every net the tune file lists: the nets the Tune pass may
    /// change (L6).
    pub(crate) fn tune_nets(&self) -> BTreeSet<String> {
        self.groups
            .iter()
            .flat_map(|group| group.nets.iter().map(|net| net.dsn.clone()))
            .collect()
    }

    /// What the judge checks: every group's DSN nets with its window, every gated
    /// pair with its DSN names.
    pub(crate) fn checks(&self) -> LengthChecks {
        LengthChecks {
            groups: self
                .groups
                .iter()
                .map(|group| CheckedGroup {
                    interface: group.interface.clone(),
                    name: group.name.clone(),
                    nets: group.nets.iter().map(|net| net.dsn.clone()).collect(),
                    window: group.window,
                })
                .collect(),
            pairs: self
                .pairs
                .iter()
                .map(|pair| CheckedPair {
                    positive: pair.dsn_positive.clone(),
                    negative: pair.dsn_negative.clone(),
                    max_skew_mm: pair.max_skew_mm,
                })
                .collect(),
        }
    }

    /// True when the result carries `length_matching` (L15): a group, a gated pair,
    /// a listed KiCad rule, or a note.
    pub(crate) fn reported(&self) -> bool {
        !self.is_empty()
            || !self.kicad_rules.is_empty()
            || self.note.is_some()
            || self.kicad_rules_note.is_some()
    }

    /// The result's `length_matching` (L15), with the winner's measures when there
    /// is a judged winner: `groups`, `pairs`, `layer_heights`, `kicad_rules`,
    /// `kicad_violations` (from `winner_violations`, KiCad's own DRC counts of
    /// `length_out_of_range` and `skew_out_of_range`), `tune_pass` (the winner's
    /// row), `pairs_tuned_net_by_net`, and `note` and `kicad_rules_note` when set.
    /// The one field returned is `length_matching`; nothing is returned when
    /// [`LengthPlan::reported`] is false.
    pub(crate) fn fields(
        &self,
        winner: Option<&LengthMeasures>,
        winner_violations: Option<&BTreeMap<String, usize>>,
        tune_pass: Option<&Value>,
    ) -> Vec<(&'static str, Value)> {
        if !self.reported() {
            return Vec::new();
        }
        let groups: Vec<Value> = self
            .groups
            .iter()
            .map(|group| {
                let measured = winner.and_then(|measures| {
                    measures
                        .groups
                        .iter()
                        .find(|m| m.interface == group.interface && m.name == group.name)
                });
                group_json(group, winner.is_some(), measured)
            })
            .collect();
        let pairs: Vec<Value> = self
            .pairs
            .iter()
            .map(|pair| {
                let measured = winner.and_then(|measures| {
                    measures.pairs.iter().find(|m| {
                        m.positive == pair.dsn_positive && m.negative == pair.dsn_negative
                    })
                });
                json!({
                    "positive": pair.positive,
                    "negative": pair.negative,
                    "dsn_positive": pair.dsn_positive,
                    "dsn_negative": pair.dsn_negative,
                    "group": pair.group,
                    "max_skew_mm": pair.max_skew_mm,
                    "skew_mm": measured.and_then(|m| m.skew_mm).map(round_mm),
                    "miss_mm": measured.map(|m| round_mm(m.miss_mm)),
                    "pass": measured.map(|m| m.skew_mm.is_some() && m.miss_mm <= 0.0),
                })
            })
            .collect();
        let rules: Vec<Value> = self
            .kicad_rules
            .iter()
            .map(|rule| {
                json!({
                    "name": rule.name,
                    "constraints": rule.constraints,
                    "condition": rule.condition,
                    "evaluated": false,
                })
            })
            .collect();
        let tuned_net_by_net: Vec<Value> = self
            .groups
            .iter()
            .flat_map(|group| {
                group.pairs.iter().map(|(positive, negative, _)| {
                    json!({
                        "interface": group.interface,
                        "group": group.name,
                        "positive": positive,
                        "negative": negative,
                    })
                })
            })
            .collect();
        let violations = winner_violations.map(|counts| {
            let mut found = Map::new();
            for key in LENGTH_RULE_KEYS {
                found.insert(
                    key.to_string(),
                    json!(counts.get(key).copied().unwrap_or(0)),
                );
            }
            Value::Object(found)
        });
        let mut body = Map::new();
        body.insert("groups".into(), json!(groups));
        body.insert("pairs".into(), json!(pairs));
        body.insert("layer_heights".into(), self.heights.json());
        body.insert("kicad_rules".into(), json!(rules));
        body.insert("kicad_violations".into(), json!(violations));
        body.insert("tune_pass".into(), json!(tune_pass));
        body.insert("pairs_tuned_net_by_net".into(), json!(tuned_net_by_net));
        if let Some(note) = &self.note {
            body.insert("note".into(), json!(note));
        }
        if let Some(note) = &self.kicad_rules_note {
            body.insert("kicad_rules_note".into(), json!(note));
        }
        vec![("length_matching", Value::Object(body))]
    }

    /// `completion.next_steps` lines (L16): the KiCad rule text of every checked
    /// group and pair ([`kicad_rule_suggestion`], [`kicad_pair_rule_suggestion`])
    /// with `create_custom_drc_rule`; and, on a miss, what can change it. A plan that
    /// checks nothing has no lines, whatever it lists of KiCad's own rules.
    pub(crate) fn next_steps(&self, winner: Option<&LengthMeasures>) -> Vec<String> {
        let mut steps = Vec::new();
        if let Some(measures) = winner.filter(|_| !self.is_empty()) {
            if measures.miss_mm > 0.0 || !measures.unmeasured.is_empty() {
                steps.push(
                    "A length window was missed (see `length_matching.groups` and `.pairs` for each net and its miss). \
                     To change that: widen `tolerance_mm` of the group (or `max_skew_mm` of the pair) with set_design_intent, \
                     give the short nets room (fastroute lengthens a net only into free space, so move parts or clear the area around it), \
                     or route again at a higher effort."
                        .to_string(),
                );
            }
        }
        for group in &self.groups {
            steps.push(format!(
                "To keep KiCad checking group '{}' of interface '{}' after hand edits, call create_custom_drc_rule with name \"{}\" \
                 and rule {} once apply_specctra_ses has run (dry_run first, then apply with the plan revision it returns). \
                 The rule is checked by KiCad's own DRC and stays in the board when it is exported again.",
                group.name,
                group.interface,
                rule_name(group),
                kicad_rule_suggestion(group)
            ));
        }
        for pair in &self.pairs {
            steps.push(format!(
                "To keep KiCad checking the skew of the pair '{}' and '{}' after hand edits, call create_custom_drc_rule with name \"{}\" \
                 and rule {} once apply_specctra_ses has run (dry_run first, then apply with the plan revision it returns).",
                pair.positive,
                pair.negative,
                pair_rule_name(pair),
                kicad_pair_rule_suggestion(pair)
            ));
        }
        steps
    }
}

/// One group of the result's `length_matching.groups`. `judged` says whether there is
/// a judged winner; without one nothing was measured, and each measure is `null`.
fn group_json(group: &ResolvedGroup, judged: bool, measured: Option<&GroupLengths>) -> Value {
    let nets: Vec<Value> = group
        .nets
        .iter()
        .map(|net| {
            let found = measured.and_then(|m| m.nets.iter().find(|n| n.net == net.dsn));
            json!({
                "net": net.intent,
                "dsn_net": net.dsn,
                "kicad_mm": found.and_then(|n| n.kicad_mm).map(round_mm),
                "low_mm": found.map(|n| round_mm(n.low_mm)),
                "high_mm": found.map(|n| round_mm(n.high_mm)),
                "miss_mm": found.map(|n| round_mm(n.miss_mm)),
            })
        })
        .collect();
    let pass = judged.then(|| {
        measured.is_some_and(|m| {
            group.nets.iter().all(|net| {
                m.nets
                    .iter()
                    .find(|n| n.net == net.dsn)
                    .is_some_and(|n| n.kicad_mm.is_some() && n.miss_mm <= 0.0)
            })
        })
    });
    json!({
        "interface": group.interface,
        "name": group.name,
        "file_name": group.file_name,
        "window": group.window.json(),
        "nets": nets,
        "longest_mm": measured.and_then(|m| m.longest_mm).map(round_mm),
        "pass": pass,
    })
}

/// Every length group of every interface, in file order.
pub(crate) fn declared_groups(intent: &DesignIntent) -> Vec<DeclaredGroup> {
    intent
        .interfaces
        .iter()
        .flat_map(|interface| {
            interface.length_groups.iter().map(|group| DeclaredGroup {
                interface: interface.name.clone(),
                name: group.name.clone(),
                nets: group.nets.clone(),
                tolerance_mm: group.tolerance_mm,
                target_mm: group.target_mm,
            })
        })
        .collect()
}

/// Every pair with `max_skew_mm`, in file order.
pub(crate) fn declared_skews(intent: &DesignIntent) -> Vec<DeclaredSkew> {
    intent
        .interfaces
        .iter()
        .flat_map(|interface| &interface.differential_pairs)
        .filter_map(|pair| {
            Some(DeclaredSkew {
                positive: pair.positive.clone(),
                negative: pair.negative.clone(),
                max_skew_mm: pair.max_skew_mm?,
            })
        })
        .collect()
}

/// The skew fastroute is given for a declared maximum (L13): `max_skew_mm` ×
/// [`FASTROUTE_WINDOW_SHARE`]. DP's pairs file writes it as `skew=`, and a trailing
/// pair group as its `tolerance=`.
pub(crate) fn fastroute_skew_mm(max_skew_mm: f64) -> f64 {
    max_skew_mm * FASTROUTE_WINDOW_SHARE
}

/// Whether an unusable design intent's raw text declares lengths: it holds the key
/// `"length_groups"` or `"max_skew_mm"`. Then the call is refused (L4); otherwise
/// D23's note stands and the call routes without the intent.
pub(crate) fn declares_lengths(raw: &str) -> bool {
    raw.contains("\"length_groups\"") || raw.contains("\"max_skew_mm\"")
}

/// Whether a resolved DSN name can be written to the tune file and a probe
/// condition: not empty, no white space or control character, none of
/// [`UNSAFE_NAME_CHARS`].
pub(crate) fn tune_token(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || UNSAFE_NAME_CHARS.contains(&c))
}

const FIX_SET_NAME: &str = "Correct that net name in the design intent with set_design_intent (get_nets_list shows the board's nets; write the full name, with its sheet path, when the short one is ambiguous), then call route_specctra_dsn again.";
const FIX_RENAME_NET: &str = "Rename the net in the schematic so that its name holds no white space and none of # * ? ' \" \\, update the board, export the DSN again, and correct the design intent; or leave the net out of its length group.";
const FIX_REEXPORT: &str = "Export the DSN again from the saved board with export_specctra_dsn. If the DSN still names the net differently from the board, rename the net so both agree, or leave it out of the length group.";
const FIX_CASE: &str = "Rename one of the two nets so that they differ by more than letter case, or leave the net out of its length group.";

fn refuse(reason: String, next_step: &str) -> LengthRefusal {
    LengthRefusal {
        reason,
        next_step: next_step.to_string(),
    }
}

/// The checks of L4 on a net's DSN name, once it is resolved: a tune-file token, a
/// net of the saved board, and no other board net equal to it when letter case is
/// ignored (KiCad's `==` ignores it, so a probe rule for `/Clk` would also catch
/// `/CLK`). `who` says where the name was written.
fn check_dsn_name(
    who: &str,
    intent: &str,
    dsn: &str,
    board_nets: &BTreeSet<String>,
) -> Result<(), LengthRefusal> {
    if !tune_token(dsn) {
        return Err(refuse(
            format!(
                "{who} names the net '{intent}', which is the DSN net {dsn:?}. Its name holds white space or one of # * ? ' \" \\, which the tune file and KiCad's rules cannot carry exactly."
            ),
            FIX_RENAME_NET,
        ));
    }
    if !board_nets.contains(dsn) {
        return Err(refuse(
            format!(
                "{who} names the net '{intent}', which is the DSN net '{dsn}'. That name is not a net of the saved board, so the session's net could not be matched back to the board."
            ),
            FIX_REEXPORT,
        ));
    }
    let lower = dsn.to_lowercase();
    if let Some(other) = board_nets
        .iter()
        .find(|other| other.as_str() != dsn && other.to_lowercase() == lower)
    {
        return Err(refuse(
            format!(
                "{who} names the net '{intent}' ('{dsn}'), and the board has another net '{other}' that differs only by letter case. KiCad's rules ignore letter case, so a length check of one would measure both."
            ),
            FIX_CASE,
        ));
    }
    Ok(())
}

/// Resolves the groups and skews against the DSN's nets (`route_pairs::resolve_name`),
/// in order. `Err` on the first group or skew that cannot be applied (L4): a name
/// resolving to no net or several, a name that is not a [`tune_token`], a DSN name
/// not in `board_nets`, a checked net equal to another of `board_nets` when letter
/// case is ignored (KiCad's `==` ignores it), or a skew whose pair is not among
/// `pairs.resolved`. Each
/// group gets `lt<N>` and its pairs (from `pairs.resolved`, matched by intent
/// names); each skew becomes a [`GatedPair`] with the group both nets are in.
///
/// Also refused: two intent names that resolve to one DSN net, in one group or in two
/// (a net has one window).
pub(crate) fn resolve(
    groups: &[DeclaredGroup],
    skews: &[DeclaredSkew],
    dsn_nets: &[String],
    board_nets: &BTreeSet<String>,
    pairs: &PairsPlan,
) -> Result<(Vec<ResolvedGroup>, Vec<GatedPair>), LengthRefusal> {
    let mut resolved: Vec<ResolvedGroup> = Vec::new();
    // The DSN net each group's net resolved to, with the group that holds it.
    let mut held: BTreeMap<String, (String, String)> = BTreeMap::new();
    for (index, group) in groups.iter().enumerate() {
        let who = format!(
            "The length group '{}' of interface '{}'",
            group.name, group.interface
        );
        let mut nets = Vec::with_capacity(group.nets.len());
        for intent in &group.nets {
            let dsn = route_pairs::resolve_name(intent, dsn_nets).map_err(|reason| {
                refuse(
                    format!("{who} names the net '{intent}', which cannot be applied: {reason}."),
                    FIX_SET_NAME,
                )
            })?;
            check_dsn_name(&who, intent, &dsn, board_nets)?;
            if let Some((interface, name)) = held.get(&dsn) {
                return Err(refuse(
                    format!(
                        "{who} names the net '{intent}', which resolves to the DSN net '{dsn}'. That net is already in the length group '{name}' of interface '{interface}', and a net has one length window."
                    ),
                    "List each net once, under one name, in one length group (set_design_intent), then call route_specctra_dsn again.",
                ));
            }
            held.insert(dsn.clone(), (group.interface.clone(), group.name.clone()));
            nets.push(GroupNet {
                intent: intent.clone(),
                dsn,
            });
        }
        let window = match group.target_mm {
            Some(target_mm) => Window::Target {
                target_mm,
                tolerance_mm: group.tolerance_mm,
            },
            None => Window::Skew {
                tolerance_mm: group.tolerance_mm,
            },
        };
        let in_group = |name: &str| nets.iter().any(|net| net.dsn == name);
        let group_pairs = pairs
            .resolved
            .iter()
            .filter(|pair| in_group(&pair.dsn_positive) && in_group(&pair.dsn_negative))
            .map(|pair| {
                (
                    pair.dsn_positive.clone(),
                    pair.dsn_negative.clone(),
                    pair.max_skew_mm
                        .map_or(FASTROUTE_PAIR_SKEW_MM, fastroute_skew_mm),
                )
            })
            .collect();
        resolved.push(ResolvedGroup {
            interface: group.interface.clone(),
            name: group.name.clone(),
            file_name: format!("lt{}", index + 1),
            nets,
            window,
            pairs: group_pairs,
        });
    }
    let mut gated = Vec::with_capacity(skews.len());
    for skew in skews {
        let who = format!(
            "The pair '{}' and '{}', which declares max_skew_mm {}",
            skew.positive,
            skew.negative,
            mm_text(skew.max_skew_mm)
        );
        let Some(pair) = pairs
            .resolved
            .iter()
            .find(|pair| pair.positive == skew.positive && pair.negative == skew.negative)
        else {
            let why = pairs
                .unresolved
                .iter()
                .find(|pair| pair.positive == skew.positive && pair.negative == skew.negative)
                .map(|pair| pair.reason.clone())
                .unwrap_or_else(|| {
                    "it is not among the pairs routed coupled, which come from the design intent's differential_pairs"
                        .to_string()
                });
            return Err(refuse(
                format!(
                    "{who}, was left out of the differential pairs ({why}), so its skew cannot be applied or checked."
                ),
                "Correct the pair's net names in the design intent with set_design_intent (each name must resolve to one net of the DSN, and a net is in one pair only), then call route_specctra_dsn again.",
            ));
        };
        for (intent, dsn) in [
            (&pair.positive, &pair.dsn_positive),
            (&pair.negative, &pair.dsn_negative),
        ] {
            check_dsn_name(&who, intent, dsn, board_nets)?;
        }
        let group = resolved
            .iter()
            .find(|group| {
                group.nets.iter().any(|net| net.dsn == pair.dsn_positive)
                    && group.nets.iter().any(|net| net.dsn == pair.dsn_negative)
            })
            .map(|group| group.name.clone());
        gated.push(GatedPair {
            positive: pair.positive.clone(),
            negative: pair.negative.clone(),
            dsn_positive: pair.dsn_positive.clone(),
            dsn_negative: pair.dsn_negative.clone(),
            max_skew_mm: skew.max_skew_mm,
            group,
        });
    }
    Ok((resolved, gated))
}

/// The net names of the saved board's net table (`(net N "name")`), for the
/// one-to-one check of L4. KiCad 10 writes no table, so this reads every net the
/// board names (`konnect_sexp::net::collect_net_keys`).
pub(crate) fn board_net_names(board_text: &str) -> anyhow::Result<BTreeSet<String>> {
    let tree = parse_sexp(board_text)
        .map_err(|error| anyhow::anyhow!("the board does not parse: {error}"))?;
    Ok(sexp_net::collect_net_keys(&tree).into_iter().collect())
}

/// Each net's shortest possible KiCad length, mm (L12): the largest distance between
/// two of its pads' centres, minus both pads' circumradii (half the pad diagonal),
/// never below 0; 0 for a net with fewer than two pads. From the saved board
/// (`konnect_sexp::board_geometry::BoardGeometry`). Only `nets` are computed, and a
/// net that owns a zone is left out (a pour can join its pads with no track).
///
/// Two pads of one footprint are never paired (they may be one pin or a net tie, which
/// join them inside the footprint), and a pad's radius is the larger of its half
/// diagonal and its farthest outline point from its centre. A board with a zone
/// Konnect cannot read leaves every net out: any of them may own it.
pub(crate) fn shortest_possible_mm(
    board_text: &str,
    nets: &BTreeSet<String>,
) -> anyhow::Result<BTreeMap<String, f64>> {
    let tree = parse_sexp(board_text)
        .map_err(|error| anyhow::anyhow!("the board does not parse: {error}"))?;
    let geometry = BoardGeometry::from_tree(&tree);
    if geometry.skipped.get("zones").copied().unwrap_or(0) > 0 {
        return Ok(BTreeMap::new());
    }
    let zoned: BTreeSet<&str> = geometry
        .others
        .iter()
        .filter(|entry| entry.kind == Kind::Zone)
        .filter_map(|entry| entry.net.as_deref())
        .collect();
    // Per net: (footprint index, pad centre, circumradius).
    type PadPoint = (usize, (f64, f64), f64);
    let mut pads: BTreeMap<&str, Vec<PadPoint>> = BTreeMap::new();
    for (index, footprint) in geometry.footprints.iter().enumerate() {
        for pad in &footprint.pads {
            let Some(name) = pad.net.as_deref() else {
                continue;
            };
            let half_diagonal = pad.size.0.hypot(pad.size.1) / 2.0;
            let reach = pad
                .shape
                .pts
                .iter()
                .map(|point| (point.0 - pad.at.0).hypot(point.1 - pad.at.1))
                .fold(0.0_f64, f64::max);
            pads.entry(name)
                .or_default()
                .push((index, pad.at, half_diagonal.max(reach)));
        }
    }
    let mut shortest = BTreeMap::new();
    for net in nets {
        if zoned.contains(net.as_str()) {
            continue;
        }
        let mut bound = 0.0_f64;
        let points = pads.get(net.as_str()).map(Vec::as_slice).unwrap_or(&[]);
        for (i, (first_footprint, first, first_radius)) in points.iter().enumerate() {
            for (second_footprint, second, second_radius) in &points[i + 1..] {
                if first_footprint == second_footprint {
                    continue;
                }
                let apart =
                    (first.0 - second.0).hypot(first.1 - second.1) - first_radius - second_radius;
                bound = bound.max(apart);
            }
        }
        shortest.insert(net.clone(), bound);
    }
    Ok(shortest)
}

/// The first target-mode group with a net whose shortest possible length is above
/// `target + tolerance`, as a refusal naming the group, the net, the shortest length
/// and the window (L12). Skew-mode groups are never refused here.
pub(crate) fn impossible_target(
    groups: &[ResolvedGroup],
    shortest: &BTreeMap<String, f64>,
) -> Option<LengthRefusal> {
    for group in groups {
        let Window::Target {
            target_mm,
            tolerance_mm,
        } = group.window
        else {
            continue;
        };
        for net in &group.nets {
            let Some(least) = shortest.get(&net.dsn).copied() else {
                continue;
            };
            if least > target_mm + tolerance_mm {
                return Some(LengthRefusal {
                    reason: format!(
                        "The length group '{}' of interface '{}' asks for {} mm plus or minus {} mm (at most {} mm), but the net '{}' cannot be that short: its pads are so far apart that no track can join them in less than {} mm, and fastroute never shortens a net.",
                        group.name,
                        group.interface,
                        mm_text(target_mm),
                        mm_text(tolerance_mm),
                        mm_text(target_mm + tolerance_mm),
                        net.intent,
                        mm_text((least * 1000.0).floor() / 1000.0)
                    ),
                    next_step: format!(
                        "Raise target_mm of the group '{}' with set_design_intent so that target_mm plus tolerance_mm is at least {} mm, or move the parts closer together, then call route_specctra_dsn again.",
                        group.name,
                        mm_text((least * 1000.0).ceil() / 1000.0)
                    ),
                });
            }
        }
    }
    None
}

/// Each clause of a rule as KiCad's file would write it, white space folded.
fn node_text(node: &SexpNode) -> String {
    match node {
        SexpNode::Atom(text) => text.clone(),
        SexpNode::Str(text) => custom_rules::quote(text),
        SexpNode::List(items) => format!(
            "({})",
            items.iter().map(node_text).collect::<Vec<_>>().join(" ")
        ),
    }
}

/// The `length` and `skew` rules of a `.kicad_dru` (`custom_rules::parse`), in file
/// order, listed with their constraints and condition, not evaluated.
pub(crate) fn kicad_length_rules(dru_text: &str) -> anyhow::Result<Vec<KicadLengthRule>> {
    let mut listed = Vec::new();
    for rule in custom_rules::parse(dru_text)? {
        let mut constraints = Vec::new();
        let mut condition = None;
        for clause in rule.node.children().unwrap_or(&[]).iter().skip(2) {
            match clause.head() {
                Some("constraint") => {
                    let kind = clause
                        .children()
                        .and_then(|args| args.get(1))
                        .and_then(SexpNode::as_str);
                    if matches!(kind, Some("length" | "skew")) {
                        constraints.push(node_text(clause));
                    }
                }
                Some("condition") => {
                    condition = clause
                        .children()
                        .and_then(|args| args.get(1))
                        .and_then(SexpNode::as_str)
                        .map(str::to_string);
                }
                _ => {}
            }
        }
        if !constraints.is_empty() {
            listed.push(KicadLengthRule {
                name: rule.name,
                constraints,
                condition,
            });
        }
    }
    Ok(listed)
}

/// `A.NetName == '<net>' || …`: KiCad's condition for a set of nets.
fn net_condition<'a>(nets: impl IntoIterator<Item = &'a str>) -> String {
    nets.into_iter()
        .map(|net| format!("A.NetName == '{net}'"))
        .collect::<Vec<_>>()
        .join(" || ")
}

/// The name of the rule that checks `group` (L16): `1kopper_len_<file name>`.
fn rule_name(group: &ResolvedGroup) -> String {
    format!("{RULE_PREFIX}len_{}", group.file_name)
}

/// The name of the rule that checks a gated pair's skew.
fn pair_rule_name(pair: &GatedPair) -> String {
    format!(
        "{RULE_PREFIX}skew_{}_{}",
        pair.dsn_positive, pair.dsn_negative
    )
}

/// The KiCad rule that checks `group` as the judge did (L16), named by its file
/// name: `(rule "1kopper_len_lt1" (constraint skew (max <T>mm)) (condition
/// "A.NetName == '<net>' || …"))` in skew mode, `(constraint length (min <L−T>mm)
/// (max <L+T>mm) (opt <L>mm))` in target mode. The full intent tolerance, not
/// fastroute's share.
pub(crate) fn kicad_rule_suggestion(group: &ResolvedGroup) -> String {
    let constraint = match group.window {
        Window::Skew { tolerance_mm } => {
            format!("(constraint skew (max {}mm))", mm_text(tolerance_mm))
        }
        Window::Target {
            target_mm,
            tolerance_mm,
        } => format!(
            "(constraint length (min {}mm) (max {}mm) (opt {}mm))",
            mm_text(target_mm - tolerance_mm),
            mm_text(target_mm + tolerance_mm),
            mm_text(target_mm)
        ),
    };
    format!(
        "(rule {} {constraint} (condition \"{}\"))",
        custom_rules::quote(&rule_name(group)),
        net_condition(group.nets.iter().map(|net| net.dsn.as_str()))
    )
}

/// The KiCad rule that checks a gated pair's skew: `(constraint skew (max <S>mm))`
/// on its two nets.
pub(crate) fn kicad_pair_rule_suggestion(pair: &GatedPair) -> String {
    format!(
        "(rule {} (constraint skew (max {}mm)) (condition \"{}\"))",
        custom_rules::quote(&pair_rule_name(pair)),
        mm_text(pair.max_skew_mm),
        net_condition([pair.dsn_positive.as_str(), pair.dsn_negative.as_str()])
    )
}

/// Writes the plan's tune file to `<folder>/tune.txt` and returns its path, or
/// `None` (and writes nothing) when the plan has no group.
pub(crate) fn write_file(folder: &Path, plan: &LengthPlan) -> anyhow::Result<Option<PathBuf>> {
    let Some(text) = plan.tune_file_text() else {
        return Ok(None);
    };
    let path = folder.join(TUNE_FILE);
    std::fs::write(&path, text)
        .map_err(|error| anyhow::anyhow!("write {}: {error}", path.display()))?;
    Ok(Some(path))
}

/// The plan of a call for the board the DSN was exported from (`board`, its saved
/// text `board_text`), after the pairs plan:
/// 1. the design intent beside the board (absent: an empty plan; unusable: refused
///    when [`declares_lengths`] reads its raw text, else an empty plan with `note`,
///    as D23);
/// 2. `resolve` against `specctra::dsn_net_names` and [`board_net_names`];
/// 3. [`impossible_target`] with [`shortest_possible_mm`];
/// 4. heights: `layer_heights::layer_heights` with the board's `.kicad_pro` and
///    `specctra::dsn_layer_names`, only when the plan has a group or the pairs plan
///    has a resolved pair (else none, no note); routing passes send them only with a
///    resolved pair, the Tune pass whenever there are any (L7);
/// 5. [`kicad_length_rules`] of `<board>.kicad_dru` when it exists.
pub(crate) fn plan(
    board: &Path,
    board_text: &str,
    dsn_text: &str,
    pairs: &PairsPlan,
) -> Result<LengthPlan, LengthRefusal> {
    // A bare file name has an empty parent, which joins as the current folder.
    let folder = board.parent().unwrap_or_else(|| Path::new("."));
    let intent_path = design_intent_path(folder);
    let mut plan = LengthPlan::default();
    let (groups, skews) = match read_design_intent(&intent_path) {
        IntentRead::Absent => (Vec::new(), Vec::new()),
        IntentRead::Unusable { reason } => {
            let raw = std::fs::read_to_string(&intent_path).unwrap_or_default();
            if declares_lengths(&raw) {
                return Err(LengthRefusal {
                    reason: format!(
                        "The design intent {} cannot be used ({reason}), and its text declares length groups or a pair skew (length_groups or max_skew_mm), which cannot be applied while the record is unusable.",
                        intent_path.display()
                    ),
                    next_step: "Repair or move the file by hand so that the record validates (Konnect never replaces an unusable record), then call route_specctra_dsn again.".to_string(),
                });
            }
            plan.note = Some(format!(
                "the design intent {} cannot be used ({reason}), so no length group or pair skew was read from it",
                intent_path.display()
            ));
            (Vec::new(), Vec::new())
        }
        IntentRead::Loaded { intent, .. } => (declared_groups(&intent), declared_skews(&intent)),
    };
    if !groups.is_empty() || !skews.is_empty() {
        let dsn_nets = specctra::dsn_net_names(dsn_text).map_err(|error| {
            refuse(
                format!(
                    "The DSN's nets could not be read ({error:#}), so the design intent's length groups and pair skews cannot be applied."
                ),
                "Export the DSN again with export_specctra_dsn, then call route_specctra_dsn again.",
            )
        })?;
        let board_nets = board_net_names(board_text).map_err(|error| {
            refuse(
                format!(
                    "The saved board's nets could not be read ({error:#}), so the design intent's length groups and pair skews cannot be applied."
                ),
                "Save the board in KiCad, export the DSN again, then call route_specctra_dsn again.",
            )
        })?;
        let (resolved, gated) = resolve(&groups, &skews, &dsn_nets, &board_nets, pairs)?;
        let target_nets: BTreeSet<String> = resolved
            .iter()
            .filter(|group| matches!(group.window, Window::Target { .. }))
            .flat_map(|group| group.nets.iter().map(|net| net.dsn.clone()))
            .collect();
        if !target_nets.is_empty() {
            let shortest = shortest_possible_mm(board_text, &target_nets).map_err(|error| {
                refuse(
                    format!(
                        "The saved board's pads could not be read ({error:#}), so a group's target could not be checked against its nets."
                    ),
                    "Save the board in KiCad, export the DSN again, then call route_specctra_dsn again.",
                )
            })?;
            if let Some(refusal) = impossible_target(&resolved, &shortest) {
                return Err(refusal);
            }
        }
        plan.groups = resolved;
        plan.pairs = gated;
    }
    if !plan.groups.is_empty() || !pairs.resolved.is_empty() {
        let project = std::fs::read_to_string(board.with_extension("kicad_pro")).ok();
        plan.heights = match specctra::dsn_layer_names(dsn_text) {
            Ok(layers) => layer_heights::layer_heights(board_text, project.as_deref(), &layers),
            Err(error) => LayerHeights {
                note: Some(format!(
                    "the DSN's layers could not be read ({error:#}), so no layer heights are sent"
                )),
                ..LayerHeights::default()
            },
        };
    }
    let dru_path = board.with_extension("kicad_dru");
    if dru_path.exists() {
        match std::fs::read_to_string(&dru_path)
            .map_err(anyhow::Error::from)
            .and_then(|text| kicad_length_rules(&text))
        {
            Ok(rules) => plan.kicad_rules = rules,
            Err(error) => {
                plan.kicad_rules_note = Some(format!(
                    "the board's rules file {} could not be listed ({error:#}); KiCad's own DRC still checks its length and skew rules",
                    dru_path.display()
                ));
            }
        }
    }
    Ok(plan)
}

/// The refusal as a tool result: `invalid_configuration` on the design intent's
/// path, "Nothing was routed." plus the reason and the next step.
pub(crate) fn refusal_result(refusal: &LengthRefusal, intent: &Path) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::InvalidConfiguration {
            path: intent.display().to_string(),
            reason: refusal.reason.clone(),
        },
        format!(
            "Nothing was routed. {} {}",
            refusal.reason, refusal.next_step
        ),
    )
}

/// L14: the note the Freerouting path adds when the board's design intent declares a
/// length group or a pair skew; `None` otherwise (also when the intent is absent or
/// unusable).
pub(crate) fn freerouting_note(board: &Path) -> Option<String> {
    let folder = board.parent().unwrap_or_else(|| Path::new("."));
    match read_design_intent(&design_intent_path(folder)) {
        IntentRead::Loaded { intent, .. }
            if !declared_groups(&intent).is_empty() || !declared_skews(&intent).is_empty() =>
        {
            Some(FREEROUTING_NOTE.to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "route_lengths_tests.rs"]
mod tests;
