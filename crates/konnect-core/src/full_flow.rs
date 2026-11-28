//! Row FF: one call from placement through pours to routing (`docs/FULL_FLOW_DESIGN.md`).
//!
//! The flow's model, with no input or output, so that every decision is tested without
//! KiCad, fastroute or a clock: the steps and their order, the approvals and the gates,
//! `flow.json`, the plan revision, the live-step protocol's decisions, and the summary.
//! The tool (`tools::full_flow_tool`) does the work and asks this module what comes next.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// FF4, owner decision 4: whether the placement search routes its candidates with the
/// pours planned on the current placement (P2, `WithPours`). Measured both ways on the
/// public placement fixture at effort m (2026-11-27): "with" lost on the pour key by one
/// step of the ground track (entry 9: 30 against 29), though its signal was shorter and
/// its copper quality higher. "With" was not at least as good, so the flow ships without:
/// P2 is skipped and the candidates are routed as they are. The pours are still planned
/// on the board the flow builds (P4).
pub(crate) const SEARCH_WITH_POURS: bool = false;

/// The most lines `summary.lines` holds.
pub(crate) const SUMMARY_LINES: usize = 8;

// ─── Steps ───────────────────────────────────────────────────────────────────

/// Every step of the flow, in order. Its serialized form is the design table's id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub(crate) enum Step {
    #[serde(rename = "P0")]
    Preflight,
    #[serde(rename = "P1")]
    CountRouting,
    #[serde(rename = "P2")]
    PoursBefore,
    #[serde(rename = "P3")]
    Placement,
    #[serde(rename = "P4")]
    PoursAfter,
    #[serde(rename = "L1")]
    Unchanged,
    #[serde(rename = "L2")]
    DeleteRouting,
    #[serde(rename = "L3")]
    ApplyPlacement,
    #[serde(rename = "L4")]
    AddZones,
    /// Row TW: the confirmed currents written to the design intent.
    #[serde(rename = "L5")]
    Widths,
    #[serde(rename = "R1")]
    Export,
    #[serde(rename = "R2")]
    Route,
    #[serde(rename = "R3")]
    ImportCheck,
    #[serde(rename = "F1")]
    ApplySession,
    #[serde(rename = "F2a")]
    PlaneStitch,
    #[serde(rename = "F2b")]
    ReferenceVias,
    #[serde(rename = "F3")]
    LengthRules,
    #[serde(rename = "F4")]
    Finish,
}

impl Step {
    /// Every step, in the flow's order.
    pub(crate) const ALL: [Step; 18] = [
        Step::Preflight,
        Step::CountRouting,
        Step::PoursBefore,
        Step::Placement,
        Step::PoursAfter,
        Step::Unchanged,
        Step::DeleteRouting,
        Step::ApplyPlacement,
        Step::AddZones,
        Step::Widths,
        Step::Export,
        Step::Route,
        Step::ImportCheck,
        Step::ApplySession,
        Step::PlaneStitch,
        Step::ReferenceVias,
        Step::LengthRules,
        Step::Finish,
    ];

    /// The design's id ("P0" … "F4").
    pub(crate) fn id(self) -> &'static str {
        match self {
            Step::Preflight => "P0",
            Step::CountRouting => "P1",
            Step::PoursBefore => "P2",
            Step::Placement => "P3",
            Step::PoursAfter => "P4",
            Step::Unchanged => "L1",
            Step::DeleteRouting => "L2",
            Step::ApplyPlacement => "L3",
            Step::AddZones => "L4",
            Step::Widths => "L5",
            Step::Export => "R1",
            Step::Route => "R2",
            Step::ImportCheck => "R3",
            Step::ApplySession => "F1",
            Step::PlaneStitch => "F2a",
            Step::ReferenceVias => "F2b",
            Step::LengthRules => "F3",
            Step::Finish => "F4",
        }
    }

    /// The step's title, for the summary and a report's `phase`.
    pub(crate) fn title(self) -> &'static str {
        match self {
            Step::Preflight => "Pre-flight",
            Step::CountRouting => "Old routing counted",
            Step::PoursBefore => "Pours on the current placement",
            Step::Placement => "Placement search",
            Step::PoursAfter => "Pours on the winner, and the export check",
            Step::Unchanged => "The board unchanged since the plan",
            Step::DeleteRouting => "Old routing deleted",
            Step::ApplyPlacement => "Placement applied",
            Step::AddZones => "Zones added",
            Step::Widths => "Track widths recorded",
            Step::Export => "Export",
            Step::Route => "Route",
            Step::ImportCheck => "Import check",
            Step::ApplySession => "Session applied",
            Step::PlaneStitch => "Plane stitching",
            Step::ReferenceVias => "Reference vias",
            Step::LengthRules => "Length rules",
            Step::Finish => "Refill, DRC and save",
        }
    }

    /// Whether the step changes the live board or the project files: L2–L5, F1–F4. Each
    /// of these runs the live-step protocol.
    pub(crate) fn writes(self) -> bool {
        matches!(
            self,
            Step::DeleteRouting
                | Step::ApplyPlacement
                | Step::AddZones
                | Step::Widths
                | Step::ApplySession
                | Step::PlaneStitch
                | Step::ReferenceVias
                | Step::LengthRules
                | Step::Finish
        )
    }
}

/// Where the flow can stop for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Gate {
    /// Gate R: a role that comes only from a net's name, before the long search.
    Roles,
    /// Gate A: the plan (placement, old routing, pours, the export check).
    Plan,
    /// Gate B: the routing, when its approval asks for it.
    Routing,
}

impl Gate {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Gate::Roles => "roles",
            Gate::Plan => "plan",
            Gate::Routing => "routing",
        }
    }
}

// ─── Approvals ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlacementAnswer {
    Apply,
    KeepCurrent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeleteAnswer {
    Delete,
    DeleteIncludingLocked,
    Keep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PoursAnswer {
    Add,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RoutingAnswer {
    ApplyIfComplete,
    ApplyBest,
    Ask,
    Skip,
}

/// A role the user gives one net.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GivenRole {
    Ground,
    Power,
    NotASupply,
}

impl GivenRole {
    pub(crate) fn name(self) -> &'static str {
        match self {
            GivenRole::Ground => "ground",
            GivenRole::Power => "power",
            GivenRole::NotASupply => "not_a_supply",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RolesAnswer {
    /// `"as_named"`: every role that comes from a name is confirmed.
    AsNamed(AsNamed),
    /// Net to role; the nets not named keep their resolved role.
    Map(BTreeMap<String, GivenRole>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AsNamed {
    AsNamed,
}

/// Row TW: the answer about the width plan's currents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum WidthsAnswer {
    /// `"as_proposed"` or `"skip"`.
    Word(WidthsWord),
    /// Net to current (A); every other proposed current is recorded as proposed.
    Map(BTreeMap<String, f64>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WidthsWord {
    /// Record every current the plan proposes.
    AsProposed,
    /// Record nothing: the nets whose current is recorded are still widened.
    Skip,
}

impl WidthsAnswer {
    /// The map, when the answer is one.
    pub(crate) fn given(&self) -> Option<&BTreeMap<String, f64>> {
        match self {
            WidthsAnswer::Map(given) => Some(given),
            WidthsAnswer::Word(_) => None,
        }
    }

    /// Whether the answer records anything.
    pub(crate) fn records(&self) -> bool {
        *self != WidthsAnswer::Word(WidthsWord::Skip)
    }
}

/// `approve`: every key optional; a missing key is asked at its gate.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Approvals {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<PlacementAnswer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_routing: Option<DeleteAnswer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pours: Option<PoursAnswer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pour_roles: Option<RolesAnswer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<RoutingAnswer>,
    /// Row TW: the width plan's currents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_widths: Option<WidthsAnswer>,
}

/// One key of `merged`: the old value, unless the answer sets it.
fn merge_key<T: PartialEq + Clone>(
    key: &str,
    old: &Option<T>,
    new: &Option<T>,
    replaceable: bool,
) -> Result<Option<T>, String> {
    match (old, new) {
        (_, None) => Ok(old.clone()),
        (None, Some(_)) => Ok(new.clone()),
        (Some(a), Some(b)) if a == b || replaceable => Ok(Some(b.clone())),
        (Some(_), Some(_)) => Err(format!(
            "`{key}` was already answered with another value; an answer may set only keys \
             that are not set yet. Leave `{key}` out of the answer, or abandon the flow and \
             start again"
        )),
    }
}

impl Approvals {
    /// `self` with every key of `answer` that is set. A key set in both with two values is
    /// an error naming it, with two exceptions: `pour_roles` at gate A, which the answer
    /// replaces (the caller then plans the pours again, `roles_changed`), and `routing` at
    /// gate B, which is the question gate B asks.
    pub(crate) fn merged(&self, answer: &Approvals, gate: Gate) -> Result<Approvals, String> {
        Ok(Approvals {
            placement: merge_key("placement", &self.placement, &answer.placement, false)?,
            delete_routing: merge_key(
                "delete_routing",
                &self.delete_routing,
                &answer.delete_routing,
                false,
            )?,
            pours: merge_key("pours", &self.pours, &answer.pours, false)?,
            pour_roles: merge_key(
                "pour_roles",
                &self.pour_roles,
                &answer.pour_roles,
                gate == Gate::Plan,
            )?,
            routing: merge_key(
                "routing",
                &self.routing,
                &answer.routing,
                gate == Gate::Routing,
            )?,
            track_widths: merge_key(
                "track_widths",
                &self.track_widths,
                &answer.track_widths,
                false,
            )?,
        })
    }

    /// `self` with every key `answer` sets replaced: the answer to gate A after a refused
    /// combination, where any key may change.
    pub(crate) fn overridden(&self, answer: &Approvals) -> Approvals {
        Approvals {
            placement: answer.placement.or(self.placement),
            delete_routing: answer.delete_routing.or(self.delete_routing),
            pours: answer.pours.or(self.pours),
            pour_roles: answer
                .pour_roles
                .clone()
                .or_else(|| self.pour_roles.clone()),
            routing: answer.routing.or(self.routing),
            track_widths: answer
                .track_widths
                .clone()
                .or_else(|| self.track_widths.clone()),
        }
    }

    /// Whether `answer` replaces the roles `self` holds with different ones.
    pub(crate) fn roles_changed(&self, answer: &Approvals) -> bool {
        match (&self.pour_roles, &answer.pour_roles) {
            (Some(old), Some(new)) => old != new,
            _ => false,
        }
    }

    /// "Routing runs": `steps.routing` is `Run` and `routing` is not `Skip`.
    pub(crate) fn routing_runs(&self, steps: &Steps) -> bool {
        steps.routing == RunOrSkip::Run && self.routing != Some(RoutingAnswer::Skip)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunOrSkip {
    Run,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlanOrSkip {
    Plan,
    Skip,
}

/// `steps`, as the call gives them (`"run"`, `"plan"`, `"skip"`); every field defaults to
/// running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Steps {
    pub placement: RunOrSkip,
    pub pours: PlanOrSkip,
    pub routing: RunOrSkip,
}

impl Default for Steps {
    fn default() -> Self {
        Steps {
            placement: RunOrSkip::Run,
            pours: PlanOrSkip::Plan,
            routing: RunOrSkip::Run,
        }
    }
}

// ─── The plan's facts, and the gates ─────────────────────────────────────────

/// What the plan holds when gate A is decided: the facts the refusals and the
/// recommendations read. Built from the scratch steps' results.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct PlanFacts {
    /// Row HS's `change` of the winner: `none`, `small`, `placement`; `None` when the
    /// placement step was skipped or found no candidate.
    pub change: Option<String>,
    /// Unlocked tracks, arcs and vias (P1's `will_delete.total` without `include_locked`).
    pub unlocked_routing: usize,
    /// Locked tracks, arcs and vias (P1's `kept_locked.total`).
    pub locked_routing: usize,
    /// P1's `kept_locked.export_will_refuse` lists at least one kept item.
    pub locked_blocks_export: bool,
    /// Locked routing ends on a pad of a part the winner moves.
    pub locked_on_moved: bool,
    /// P4's zone count, and whether its fill check is clean.
    pub zones: usize,
    pub fill_clean: bool,
    /// P4's export check: `None` passed (or not run), `Some(reason)` refused.
    pub export_refused: Option<String>,
    /// Nets whose role comes from the name only, after the given roles.
    pub named_only: Vec<String>,
    /// P4 planned the pours on the search's winner (not on the current placement).
    #[serde(default)]
    pub p4_on_winner: bool,
    /// Row TW: the currents the width plan proposes to record (its `to_record`), and the
    /// nets they are for.
    #[serde(default)]
    pub widths_to_record: usize,
    #[serde(default)]
    pub widths_proposed_for: Vec<String>,
    /// The same for the board without P4's zones: what L5 writes when the user answers
    /// `pours: skip` (the nets those zones would pour carry their current on tracks).
    #[serde(default)]
    pub widths_to_record_unpoured: usize,
    #[serde(default)]
    pub widths_proposed_for_unpoured: Vec<String>,
}

impl PlanFacts {
    /// Row TW: whether L5 writes the plan made without P4's zones: the zones are not
    /// added (`pours` is not `add`, or none is proposed).
    pub(crate) fn widths_unpoured(&self, approved: &Approvals) -> bool {
        self.zones == 0 || approved.pours != Some(PoursAnswer::Add)
    }

    /// Row TW: the count of currents L5 writes for `approved`, and the names they are for.
    pub(crate) fn widths_for(&self, approved: &Approvals) -> (usize, &[String]) {
        if self.widths_unpoured(approved) {
            (
                self.widths_to_record_unpoured,
                &self.widths_proposed_for_unpoured,
            )
        } else {
            (self.widths_to_record, &self.widths_proposed_for)
        }
    }
}

impl PlanFacts {
    /// The winner changes the board, so applying it is a question.
    fn placement_changes(&self) -> bool {
        matches!(self.change.as_deref(), Some("small" | "placement"))
    }
}

/// One key a gate still needs: its name, the recommended value (`None` when the plan
/// recommends nothing: a fill check that is not clean, a judge gate that failed), and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Need {
    pub key: &'static str,
    pub recommended: Option<Value>,
    pub why: String,
}

/// Whether the placement may be applied: the step ran, the winner changes the board,
/// and the user did not keep the current one.
fn placement_may_apply(facts: &PlanFacts, steps: &Steps, approved: &Approvals) -> bool {
    steps.placement == RunOrSkip::Run
        && facts.placement_changes()
        && approved.placement != Some(PlacementAnswer::KeepCurrent)
}

/// The keys gate A needs, given `facts`, `steps` and what is approved:
/// - `placement` when the winner's `change` is `small` (recommended `keep_current`) or
///   `placement` (recommended `apply`); with `none` the step is skipped;
/// - `delete_routing` when the board holds routing and the placement may be applied or
///   routing runs (recommended `delete_including_locked` when the kept locked routing
///   would make the export refuse or touches a moved part, else `delete`);
/// - `pours` when the plan proposes zones (recommended `add` when the fill check is clean,
///   else nothing: the user decides);
/// - `routing` when the routing step runs (recommended `apply_if_complete`);
/// - `pour_roles` when a role still comes from a name only (recommended `as_named`);
/// - Row TW: `track_widths` when routing runs and the width plan proposes a current
///   (recommended `as_proposed`).
pub(crate) fn needs_at_plan(facts: &PlanFacts, steps: &Steps, approved: &Approvals) -> Vec<Need> {
    let mut needs = Vec::new();
    if let Some(need) = needs_roles(&facts.named_only, approved) {
        needs.push(need);
    }
    if steps.placement == RunOrSkip::Run
        && facts.placement_changes()
        && approved.placement.is_none()
    {
        let small = facts.change.as_deref() == Some("small");
        needs.push(Need {
            key: "placement",
            recommended: Some(json!(if small { "keep_current" } else { "apply" })),
            why: if small {
                "The winner barely changes the board: applying it is optional.".to_string()
            } else {
                "The winner moves parts: apply it, or keep the current placement.".to_string()
            },
        });
    }
    let has_routing = facts.unlocked_routing + facts.locked_routing > 0;
    if has_routing
        && (placement_may_apply(facts, steps, approved) || approved.routing_runs(steps))
        && approved.delete_routing.is_none()
    {
        let locked = facts.locked_blocks_export || facts.locked_on_moved;
        needs.push(Need {
            key: "delete_routing",
            recommended: Some(json!(if locked { "delete_including_locked" } else { "delete" })),
            why: if locked {
                "The board's locked routing would stay: the export refuses it, or it ends on a part that moves. Delete it too, or keep the routing and skip the routing step.".to_string()
            } else {
                "The old routing goes before the new placement and routing. It is deleted before the new routing exists; the way back is the start checkpoint.".to_string()
            },
        });
    }
    if steps.pours == PlanOrSkip::Plan && facts.zones > 0 && approved.pours.is_none() {
        needs.push(Need {
            key: "pours",
            recommended: facts.fill_clean.then(|| json!("add")),
            why: if facts.fill_clean {
                "KiCad's fill check of the proposed zones is clean.".to_string()
            } else {
                "KiCad's fill check of the proposed zones is not clean: the user decides whether to add them as they are.".to_string()
            },
        });
    }
    if steps.routing == RunOrSkip::Run && approved.routing.is_none() {
        needs.push(Need {
            key: "routing",
            recommended: Some(json!("apply_if_complete")),
            why: "The routed session is applied when every judge gate passes; the flow asks again only when one fails.".to_string(),
        });
    }
    // Asked when the plan L5 would write proposes a current, or while the pours are not
    // answered and either plan does: the pours answer, given in the same answer, then
    // decides which plan L5 writes.
    let (count, names) = facts.widths_for(approved);
    // (With the pours open, `count` is already the plan without the zones.)
    let undecided = approved.pours.is_none() && facts.zones > 0 && facts.widths_to_record > 0;
    if approved.routing_runs(steps) && (count > 0 || undecided) && approved.track_widths.is_none() {
        let names: &[String] = if count > 0 {
            names
        } else {
            &facts.widths_proposed_for
        };
        let skipped_pours = if facts.zones > 0
            && facts.widths_proposed_for_unpoured != facts.widths_proposed_for
        {
            if facts.widths_proposed_for_unpoured.is_empty() {
                " With pours: skip it proposes none.".to_string()
            } else {
                format!(
                    " With pours: skip it proposes one for {} instead, since the nets the zones would pour then carry their current on tracks.",
                    facts.widths_proposed_for_unpoured.join(", ")
                )
            }
        } else {
            String::new()
        };
        needs.push(Need {
            key: "track_widths",
            recommended: Some(json!("as_proposed")),
            why: format!(
                "No current is recorded for {}: the width plan proposes {} A for each (a rail's current covers all its nets), a stated default, and sizes their tracks and vias for it.{skipped_pours} \"as_proposed\" records it in the design intent; a map {{net: amps}} gives the nets you name their real current; \"skip\" records nothing and routes them at their class width.",
                names.join(", "),
                crate::width_plan::DEFAULT_SUPPLY_CURRENT_A
            ),
        });
    }
    needs
}

/// Gate R's need: `pour_roles` when `named_only` is not empty and it is not approved.
pub(crate) fn needs_roles(named_only: &[String], approved: &Approvals) -> Option<Need> {
    (!named_only.is_empty() && approved.pour_roles.is_none()).then(|| Need {
        key: "pour_roles",
        recommended: Some(json!("as_named")),
        why: format!(
            "The roles of {} come from their names only. \"as_named\" confirms them; a map \
             {{net: \"ground\" | \"power\" | \"not_a_supply\"}} sets the ones you name.",
            named_only.join(", ")
        ),
    })
}

/// Gate B's need: `routing` when the routing approval stops the flow. Recommended
/// `apply_best` when every judge gate passed (an `ask` approval), else nothing: a judge
/// gate failed, and the user decides.
pub(crate) fn needs_routing(approved: &Approvals, all_gates_pass: Option<bool>) -> Option<Need> {
    let approval = approved.routing?;
    stops_for_routing(approval, all_gates_pass).then(|| {
        let passed = all_gates_pass == Some(true);
        Need {
            key: "routing",
            recommended: passed.then(|| json!("apply_best")),
            why: if passed {
                "Every judge gate passed. \"apply_best\" applies the session; \"skip\" leaves the board unrouted.".to_string()
            } else {
                "A judge gate failed (`routing.gates`). \"apply_best\" applies the session as it is; \"skip\" leaves the board unrouted.".to_string()
            },
        }
    })
}

/// Whether gate B stops the flow. `Ask` always stops; `ApplyIfComplete` stops when
/// `all_gates_pass` (the route's `completion.judged.all_gates_pass`) is false or absent;
/// `ApplyBest` never stops; `Skip` never reaches the route (FF16).
pub(crate) fn stops_for_routing(approval: RoutingAnswer, all_gates_pass: Option<bool>) -> bool {
    match approval {
        RoutingAnswer::Ask => true,
        RoutingAnswer::ApplyIfComplete => all_gates_pass != Some(true),
        RoutingAnswer::ApplyBest | RoutingAnswer::Skip => false,
    }
}

/// The refusals of "Combinations that are refused", each naming the key to change:
/// - placement applied with unlocked routing kept;
/// - routing runs (`Approvals::routing_runs`) with unlocked routing kept;
/// - routing runs with a refused export check;
/// - routing runs with `Delete` while `locked_blocks_export`;
/// - a role against a design-intent rail (`rails`: net to `"ground"` or `"power"`,
///   from `pour_plan::rails_of`);
/// - a role for a net the board does not have (`nets`).
///
/// Checked before any write.
pub(crate) fn refused_combination(
    approved: &Approvals,
    facts: &PlanFacts,
    steps: &Steps,
    rails: &BTreeMap<String, &'static str>,
    nets: &BTreeSet<String>,
) -> Option<String> {
    let keep = approved.delete_routing == Some(DeleteAnswer::Keep);
    let unlocked = facts.unlocked_routing > 0;
    let routing = approved.routing_runs(steps);
    if approved.placement == Some(PlacementAnswer::Apply)
        && steps.placement == RunOrSkip::Run
        && facts.placement_changes()
        && keep
        && unlocked
    {
        return Some(format!(
            "placement: apply with delete_routing: keep is refused: the board holds {} unlocked \
             track(s), arc(s) and via(s), and old routing does not move with its pads. Change \
             delete_routing to delete, or placement to keep_current.",
            facts.unlocked_routing
        ));
    }
    if routing && keep && unlocked {
        return Some(format!(
            "routing with delete_routing: keep is refused: export_specctra_dsn refuses a board \
             that holds unlocked routing ({} item(s)). Change delete_routing to delete, or \
             routing to skip.",
            facts.unlocked_routing
        ));
    }
    if routing {
        if let Some(reason) = &facts.export_refused {
            return Some(format!(
                "routing is refused: the export check of the board the flow would build failed \
                 ({reason}). Fix that and start again, or change routing to skip."
            ));
        }
        if approved.delete_routing == Some(DeleteAnswer::Delete) && facts.locked_blocks_export {
            return Some(
                "routing with delete_routing: delete is refused: export_specctra_dsn will still \
                 refuse the locked routing that stays (P1's kept_locked.export_will_refuse). \
                 Change delete_routing to delete_including_locked, or routing to skip."
                    .to_string(),
            );
        }
    }
    if let Some(RolesAnswer::Map(given)) = &approved.pour_roles {
        for (net, role) in given {
            if !nets.contains(net) {
                return Some(format!(
                    "pour_roles names '{net}', which the board does not have. Change pour_roles."
                ));
            }
            if let Some(rail) = rails.get(net) {
                if *rail != role.name() {
                    return Some(format!(
                        "pour_roles gives '{net}' the role {}, but the design intent records it \
                         as a {rail} rail, and the design intent wins. Change pour_roles, or edit \
                         the rail with set_design_intent (section 'rails').",
                        role.name()
                    ));
                }
            }
        }
    }
    None
}

/// `value` as JSON with every object's keys in order, whatever the map's own order.
pub(crate) fn canonical_json(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let ordered: BTreeMap<&String, Value> = map
                    .iter()
                    .map(|(key, value)| (key, sorted(value)))
                    .collect();
                let mut out = serde_json::Map::new();
                for (key, value) in ordered {
                    out.insert(key.clone(), value);
                }
                Value::Object(out)
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    sorted(value).to_string()
}

/// `expected_plan_revision`: SHA-256 (hex) over the flow id, the gate, `planned_from`
/// and the canonical JSON of the gate's plan body.
pub(crate) fn plan_revision(flow_id: &str, gate: Gate, planned_from: &str, body: &Value) -> String {
    let mut hasher = Sha256::new();
    for part in [flow_id, gate.name(), planned_from, &canonical_json(body)] {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    format!("{:x}", hasher.finalize())
}

// ─── The record ──────────────────────────────────────────────────────────────

/// Where the flow stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FlowState {
    Running,
    AwaitingAnswer,
    Stopped,
    Done,
    Abandoned,
}

impl FlowState {
    pub(crate) fn name(self) -> &'static str {
        match self {
            FlowState::Running => "running",
            FlowState::AwaitingAnswer => "awaiting_answer",
            FlowState::Stopped => "stopped",
            FlowState::Done => "done",
            FlowState::Abandoned => "abandoned",
        }
    }
}

/// What the board is known to hold when the flow stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BoardState {
    /// No live step has run.
    Unchanged,
    /// Every live step that ran is recorded with its verified save.
    AsRecorded,
    /// A live step may or may not have landed: inspect the board first.
    Uncertain,
    /// A live step landed and its save did not verify: KiCad holds it unsaved.
    Unsaved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StepState {
    /// The intent is recorded and the call may be under way.
    Calling,
    Done,
    Skipped,
    Failed,
    Uncertain,
    Unsaved,
    NotRun,
}

impl StepState {
    pub(crate) fn name(self) -> &'static str {
        match self {
            StepState::Calling => "calling",
            StepState::Done => "done",
            StepState::Skipped => "skipped",
            StepState::Failed => "failed",
            StepState::Uncertain => "uncertain",
            StepState::Unsaved => "unsaved",
            StepState::NotRun => "not_run",
        }
    }

    /// Finished for the flow's order: the next step may follow.
    pub(crate) fn finished(self) -> bool {
        matches!(self, StepState::Done | StepState::Skipped)
    }
}

/// One step's record, in the Row HS style.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StepRecord {
    pub step: Step,
    pub state: StepState,
    /// Set for every state but `done`.
    pub why: Option<String>,
    /// Only what the step's own result reports (`counts_of`).
    pub counts: Value,
    pub checkpoint: Option<String>,
    /// The saved file's canonical SHA-256 before and after a writing step (after the
    /// verified save). `after` is `None` until the save is verified.
    pub before: Option<String>,
    pub after: Option<String>,
    /// For F2a and F2b: the `after` of each round done.
    pub rounds: Vec<String>,
    pub result_file: Option<String>,
    pub not_checked: Vec<String>,
}

impl StepRecord {
    /// A record with nothing but its step and state.
    pub(crate) fn new(step: Step, state: StepState) -> Self {
        StepRecord {
            step,
            state,
            why: None,
            counts: json!({}),
            checkpoint: None,
            before: None,
            after: None,
            rounds: Vec::new(),
            result_file: None,
            not_checked: Vec::new(),
        }
    }

    pub(crate) fn skipped(step: Step, why: impl Into<String>) -> Self {
        StepRecord {
            why: Some(why.into()),
            ..StepRecord::new(step, StepState::Skipped)
        }
    }
}

/// What the route left for the steps after it: its judge's verdict and which follow-ups
/// its `completion.next_steps` hold.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RouteFacts {
    /// `completion.judged.all_gates_pass`.
    pub all_gates_pass: Option<bool>,
    pub plane_stitching: bool,
    pub reference_vias: bool,
    pub length_rules: bool,
}

/// `flow.json`: the whole record of one flow, rewritten (durable) before every call (the
/// intent, `Calling`), at every step's end and at every gate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FlowRecord {
    pub flow_id: String,
    pub board: String,
    pub project: String,
    pub state: FlowState,
    pub board_state: BoardState,
    pub steps_asked: Steps,
    pub approvals: Approvals,
    /// The approvals the start call gave: what a new plan (a resume that plans again)
    /// keeps; an answer given at gate A was about the plan it saw.
    #[serde(default)]
    pub start_approvals: Approvals,
    pub effort: String,
    pub placement_effort: String,
    /// The fastroute binary P0 located (path, version, source).
    pub fastroute: Value,
    /// `canonical_sha256` of the live text when the plan was read.
    pub planned_from: String,
    /// Tracks, arcs, vias and teardrop zones on the board when the plan was read.
    pub routing_items: usize,
    /// The nets of the board that have a pad.
    pub nets: BTreeSet<String>,
    /// The design-intent rails a role may not contradict (net to `ground` or `power`).
    pub rails: BTreeMap<String, String>,
    pub gate: Option<Gate>,
    pub gate_body: Option<Value>,
    pub plan_revision: Option<String>,
    pub facts: PlanFacts,
    pub route: Option<RouteFacts>,
    /// The step running or stopped at.
    pub at: Option<Step>,
    /// The plan's attempt folder (`attempts/<n>/`) and the route's (`routes/<n>/`): a
    /// resume of a scratch or route step uses a new one (FF14).
    pub attempt: u32,
    pub route_attempt: u32,
    pub records: Vec<StepRecord>,
    /// The job module's run ids of the legs, in order.
    pub legs: Vec<String>,
    /// The first checkpoint, the way back to before the flow.
    pub start_checkpoint: Option<String>,
    pub stop_reason: Option<String>,
    /// Every checkpoint the flow took or reused, in order, without repeats.
    pub checkpoints: Vec<String>,
}

impl FlowRecord {
    /// The record of `step`, if it has one.
    pub(crate) fn record(&self, step: Step) -> Option<&StepRecord> {
        self.records.iter().rev().find(|record| record.step == step)
    }

    /// `step`'s record, replaced, or added in the flow's order.
    pub(crate) fn put(&mut self, record: StepRecord) {
        self.records.retain(|old| old.step != record.step);
        let at = self
            .records
            .iter()
            .position(|old| old.step > record.step)
            .unwrap_or(self.records.len());
        self.records.insert(at, record);
    }

    /// Whether `step` is finished (done or skipped).
    pub(crate) fn finished(&self, step: Step) -> bool {
        self.record(step)
            .is_some_and(|record| record.state.finished())
    }

    /// Whether `step` is done.
    pub(crate) fn done(&self, step: Step) -> bool {
        self.record(step)
            .is_some_and(|record| record.state == StepState::Done)
    }

    /// The `after` a writing step must find: that of the last writing step done, else
    /// `planned_from`.
    pub(crate) fn expected_before(&self, step: Step) -> String {
        // A step resumed to run again starts from the board its resume found (m1).
        if let Some(again) = self
            .record(step)
            .filter(|record| record.state == StepState::NotRun)
            .and_then(|record| record.before.clone())
        {
            return again;
        }
        self.records
            .iter()
            .filter(|record| record.step < step && record.step.writes())
            .filter(|record| record.state == StepState::Done)
            .filter_map(|record| record.after.clone())
            .next_back()
            .unwrap_or_else(|| self.planned_from.clone())
    }

    /// The rails as `refused_combination` reads them.
    pub(crate) fn rail_roles(&self) -> BTreeMap<String, &'static str> {
        self.rails
            .iter()
            .map(|(net, role)| {
                let role: &'static str = if role == "ground" { "ground" } else { "power" };
                (net.clone(), role)
            })
            .collect()
    }

    /// Whether any writing step is done: F4 runs only then.
    pub(crate) fn wrote(&self) -> bool {
        self.records.iter().any(|record| {
            record.step.writes() && record.step != Step::Finish && record.state == StepState::Done
        })
    }
}

// ─── The live-step protocol's decisions ──────────────────────────────────────

/// What a writing step does before it runs (step 3 of "Each live step").
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LiveStart {
    /// The saved board is where the step before left it: checkpoint and run.
    Run,
    /// The board is neither there nor already past this step: stop with
    /// `board_changed`, naming the expected SHA-256.
    Changed { expected: String, found: String },
}

/// Step 3's decision: `found` is the saved file's canonical SHA-256 now, `expected` the
/// `after` of the step before (or `planned_from`).
pub(crate) fn plan_live_step(found: &str, expected: &str) -> LiveStart {
    if found == expected {
        LiveStart::Run
    } else {
        LiveStart::Changed {
            expected: expected.to_string(),
            found: found.to_string(),
        }
    }
}

/// How a step's tool result reads, as B classifies it (`board_safety::Outcome::of`), with
/// one addition the tool makes: a handler `Err` from a writing tool is `MayHaveChanged`.
pub(crate) use crate::tools::board_safety::Outcome;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resume {
    /// The step's work is on the board: record it done (the board as found is its
    /// `after`) and go on.
    RecordDone,
    /// The step's work is not on the board, and the board is where the step began: run
    /// it again.
    RunAgain,
    /// The step's work landed and only its save failed: save again.
    SaveAgain,
}

/// Whether `resume` may run the stopped writing step again, given whether it is already
/// done (`done`, from `already_done` on the live board) and the saved file's SHA-256 now.
/// - A step `Unsaved`: only the save is tried again.
/// - Done: record it as done and go on.
/// - Not done, and the board is at `before`: run it again.
/// - Otherwise: the reason, naming the checkpoint to restore.
pub(crate) fn may_resume(record: &StepRecord, done: bool, found: &str) -> Result<Resume, String> {
    if record.state == StepState::Unsaved {
        return Ok(Resume::SaveAgain);
    }
    if done {
        return Ok(Resume::RecordDone);
    }
    match &record.before {
        // At the step's start, or at the end of its last stitching round.
        Some(before)
            if before == found || record.rounds.last().is_some_and(|round| round == found) =>
        {
            Ok(Resume::RunAgain)
        }
        Some(before) => {
            Err(format!(
            "board_changed: {} ({}) began on the saved board {before}, and the saved board now \
             is {found}, without that step's work. Something else changed the board. Inspect it \
             in KiCad; restore a checkpoint ({}) or abandon the flow.",
            record.step.id(),
            record.step.title(),
            record.checkpoint.as_deref().unwrap_or("the start checkpoint"),
        ))
        }
        None => Err(format!(
            "{} ({}) has no recorded starting board, so a resume cannot tell what it would \
             repeat. Abandon the flow and start again.",
            record.step.id(),
            record.step.title()
        )),
    }
}

// ─── The order ───────────────────────────────────────────────────────────────

/// What the flow does next.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Next {
    Step(Step),
    /// The step is skipped, with why (the design table's "Skipped when").
    Skip(Step, String),
    Gate(Gate),
    End,
}

/// Why `step` is skipped for `flow`, or `None` when it runs.
fn skip_reason(step: Step, flow: &FlowRecord) -> Option<String> {
    let steps = &flow.steps_asked;
    let approved = &flow.approvals;
    let facts = &flow.facts;
    let routing_runs = approved.routing_runs(steps);
    let routing_skipped = || {
        if steps.routing == RunOrSkip::Skip {
            "steps.routing is skip".to_string()
        } else {
            "routing: skip".to_string()
        }
    };
    match step {
        Step::Preflight | Step::Unchanged => None,
        Step::CountRouting => {
            (flow.routing_items == 0).then(|| "the board holds no tracks, arcs or vias".to_string())
        }
        Step::PoursBefore => {
            if steps.pours == PlanOrSkip::Skip {
                Some("steps.pours is skip".to_string())
            } else if steps.placement == RunOrSkip::Skip {
                Some("steps.placement is skip, so no search needs the pours before it".to_string())
            } else if !SEARCH_WITH_POURS {
                Some(
                    "the placement search routes its candidates without the pours (FF4, measured)"
                        .to_string(),
                )
            } else {
                None
            }
        }
        Step::Placement => {
            (steps.placement == RunOrSkip::Skip).then(|| "steps.placement is skip".to_string())
        }
        Step::PoursAfter => (steps.pours == PlanOrSkip::Skip && steps.routing == RunOrSkip::Skip)
            .then(|| "steps.pours and steps.routing are skip".to_string()),
        Step::DeleteRouting => match approved.delete_routing {
            None | Some(DeleteAnswer::Keep) => {
                Some(if facts.unlocked_routing + facts.locked_routing == 0 {
                    "nothing to delete".to_string()
                } else {
                    "delete_routing: keep".to_string()
                })
            }
            Some(DeleteAnswer::Delete) if facts.unlocked_routing == 0 => {
                Some("no unlocked routing to delete".to_string())
            }
            Some(DeleteAnswer::DeleteIncludingLocked)
                if facts.unlocked_routing + facts.locked_routing == 0 =>
            {
                Some("nothing to delete".to_string())
            }
            Some(_) => None,
        },
        Step::ApplyPlacement => {
            if steps.placement == RunOrSkip::Skip {
                Some("steps.placement is skip".to_string())
            } else if !facts.placement_changes() {
                Some(match facts.change.as_deref() {
                    Some("none") => "change: none, the winner is the board as it is".to_string(),
                    _ => "the search named no winner".to_string(),
                })
            } else if approved.placement != Some(PlacementAnswer::Apply) {
                Some("placement: keep_current".to_string())
            } else {
                None
            }
        }
        Step::AddZones => {
            if steps.pours == PlanOrSkip::Skip {
                Some("steps.pours is skip".to_string())
            } else if facts.zones == 0 {
                Some("no zone proposed".to_string())
            } else if approved.pours != Some(PoursAnswer::Add) {
                Some("pours: skip".to_string())
            } else {
                None
            }
        }
        Step::Widths => {
            if !routing_runs {
                Some(routing_skipped())
            } else if facts.widths_for(approved).0 == 0
                && approved
                    .track_widths
                    .as_ref()
                    .and_then(WidthsAnswer::given)
                    .is_none_or(BTreeMap::is_empty)
            {
                Some("every current the plan needs is recorded already".to_string())
            } else if !approved
                .track_widths
                .as_ref()
                .is_some_and(WidthsAnswer::records)
            {
                Some("track_widths: skip".to_string())
            } else {
                None
            }
        }
        Step::Export | Step::Route | Step::ImportCheck => (!routing_runs).then(routing_skipped),
        Step::ApplySession => {
            if !routing_runs {
                Some(routing_skipped())
            } else if !flow.done(Step::ImportCheck) {
                Some("the import check did not pass".to_string())
            } else {
                None
            }
        }
        Step::PlaneStitch | Step::ReferenceVias | Step::LengthRules => {
            if !flow.done(Step::ApplySession) {
                return Some("the session was not applied".to_string());
            }
            let route = flow.route.unwrap_or_default();
            let (present, what) = match step {
                Step::PlaneStitch => (route.plane_stitching, "plane-stitching steps"),
                Step::ReferenceVias => (route.reference_vias, "reference-via steps"),
                _ => (route.length_rules, "length-rule step"),
            };
            (!present).then(|| format!("the route's next steps hold no {what}"))
        }
        Step::Finish => (!flow.wrote()).then(|| "nothing was written to the board".to_string()),
    }
}

/// The next thing the flow does, from its record: the first step in order that has no
/// finished record, skipped by the "Skipped when" column; or a gate, where its approval
/// is still missing; or the end.
/// - Gate R sits after P0: `needs_roles`.
/// - Gate A sits after P4: `needs_at_plan`.
/// - Gate B sits after R3: `needs_routing`, with the route's `all_gates_pass`.
pub(crate) fn next(flow: &FlowRecord) -> Next {
    let Some(step) = Step::ALL.iter().copied().find(|step| !flow.finished(*step)) else {
        return Next::End;
    };
    // `step` is the first unfinished one, so every step before it is finished.
    if step == Step::CountRouting && needs_roles(&flow.facts.named_only, &flow.approvals).is_some()
    {
        return Next::Gate(Gate::Roles);
    }
    if step == Step::Unchanged
        && !needs_at_plan(&flow.facts, &flow.steps_asked, &flow.approvals).is_empty()
    {
        return Next::Gate(Gate::Plan);
    }
    if step == Step::ApplySession
        && flow.done(Step::ImportCheck)
        && needs_routing(
            &flow.approvals,
            flow.route.and_then(|route| route.all_gates_pass),
        )
        .is_some()
    {
        return Next::Gate(Gate::Routing);
    }
    match skip_reason(step, flow) {
        Some(why) => Next::Skip(step, why),
        None => Next::Step(step),
    }
}

// ─── Summaries ───────────────────────────────────────────────────────────────

/// One step's `counts`, from its own tool result only:
/// - L2: `removed`;
/// - L3: the placements sent;
/// - L4: the zones added;
/// - F1: the session's items as the apply reports them;
/// - F2a and F2b: rounds and vias adopted (the tool counts them from its own calls);
/// - F3: `rules` by action;
/// - F4: the DRC counts.
pub(crate) fn counts_of(step: Step, result: &Value) -> Value {
    match step {
        Step::DeleteRouting => result.get("removed").cloned().unwrap_or(json!({})),
        Step::ApplyPlacement => json!({
            "placements": result["placements"].as_array().map_or(0, Vec::len)
        }),
        Step::AddZones => json!({ "zones": result["zones"].as_u64().unwrap_or(0) }),
        Step::Widths => json!({
            "currents": result["currents"].as_u64().unwrap_or(0),
            "rails": result["rails"].as_u64().unwrap_or(0),
        }),
        Step::ApplySession => {
            let mut counts = serde_json::Map::new();
            for key in ["tracks", "vias", "arcs", "imported", "applied"] {
                if let Some(value) = result
                    .get(key)
                    .filter(|value| value.is_number() || value.is_object())
                {
                    counts.insert(key.to_string(), value.clone());
                }
            }
            Value::Object(counts)
        }
        Step::PlaneStitch | Step::ReferenceVias => json!({
            "rounds": result["rounds"].as_u64().unwrap_or(0),
            "vias": result["vias"].as_u64().unwrap_or(0),
        }),
        Step::LengthRules => {
            let mut by_action: BTreeMap<String, usize> = BTreeMap::new();
            for rule in result["rules"].as_array().into_iter().flatten() {
                let action = rule["action"].as_str().unwrap_or("unknown").to_string();
                *by_action.entry(action).or_default() += 1;
            }
            json!({ "rules": by_action })
        }
        Step::Finish => json!({
            "by_rule": result.get("by_rule").cloned().unwrap_or(json!([])),
            "total": result.get("total").cloned().unwrap_or(Value::Null),
        }),
        _ => json!({}),
    }
}

/// The count of KiCad's unconnected items in a `run_drc` result's `by_rule` tally;
/// `None` when the result has no tally.
pub(crate) fn unconnected_of(drc: &Value) -> Option<u64> {
    let rows = drc.get("by_rule")?.as_array()?;
    Some(
        rows.iter()
            .filter(|row| row["rule"] == "unconnected_items")
            .filter_map(|row| row["count"].as_u64())
            .sum(),
    )
}

/// One summary line for a record, or `None` when the step says nothing worth a line.
fn line_of(record: &StepRecord) -> Option<String> {
    let title = record.step.title();
    match record.state {
        StepState::Done => {
            let counts = match &record.counts {
                Value::Object(map) if !map.is_empty() => {
                    let parts: Vec<String> = map
                        .iter()
                        .filter(|(_, value)| !value.is_null())
                        .map(|(key, value)| format!("{key} {}", compact(value)))
                        .collect();
                    if parts.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", parts.join(", "))
                    }
                }
                _ => String::new(),
            };
            Some(format!("{title}: done{counts}."))
        }
        StepState::Skipped => match record.step {
            // Steps that are skipped whenever the route asked for nothing of them, and the
            // reads, are not worth a line when skipped.
            Step::CountRouting
            | Step::PoursBefore
            | Step::Widths
            | Step::Export
            | Step::ImportCheck
            | Step::PlaneStitch
            | Step::ReferenceVias
            | Step::LengthRules => None,
            _ => Some(format!(
                "{title}: skipped ({}).",
                record.why.as_deref().unwrap_or("no reason recorded")
            )),
        },
        StepState::Calling | StepState::NotRun => None,
        StepState::Failed | StepState::Uncertain | StepState::Unsaved => Some(format!(
            "{title}: {} ({}).",
            record.state.name(),
            record.why.as_deref().unwrap_or("no reason recorded")
        )),
    }
}

/// A short text of a count value.
fn compact(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            if let Some(total) = map.get("total") {
                return total.to_string();
            }
            let parts: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{k} {}", compact(v)))
                .collect();
            parts.join(", ")
        }
        Value::Array(items) => items.len().to_string(),
        other => other.to_string(),
    }
}

/// The final summary. `lines`: at most [`SUMMARY_LINES`], in the steps' order, one per
/// step that did or skipped something, with the route's `summary.lengths.line` and
/// `summary.pours.line` quoted as they are. Then `done`, `skipped` with `why`,
/// `not_checked` with each item's step, the checkpoint count, and `way_back`. Pure: reads
/// only `flow` and the route summary it is given.
pub(crate) fn summary(flow: &FlowRecord, route_summary: Option<&Value>) -> Value {
    // The route's own lines are always kept; the steps' lines fill the rest, in order,
    // the route's lines after the session's.
    let quotes: Vec<String> = route_summary
        .into_iter()
        .flat_map(|route| {
            ["pours", "lengths"].map(|key| route[key]["line"].as_str().map(str::to_string))
        })
        .flatten()
        .collect();
    let step_lines: Vec<(Step, String)> = flow
        .records
        .iter()
        .filter(|record| record.step.writes() || record.step == Step::Route)
        .filter_map(|record| line_of(record).map(|line| (record.step, line)))
        .take(SUMMARY_LINES.saturating_sub(quotes.len()))
        .collect();
    let mut lines: Vec<String> = Vec::new();
    let mut quoted = false;
    for (step, line) in step_lines {
        if !quoted && step > Step::ApplySession {
            lines.extend(quotes.iter().cloned());
            quoted = true;
        }
        lines.push(line);
    }
    if !quoted {
        lines.extend(quotes);
    }
    let done: Vec<&str> = flow
        .records
        .iter()
        .filter(|record| record.state == StepState::Done)
        .map(|record| record.step.id())
        .collect();
    let skipped: Vec<Value> = flow
        .records
        .iter()
        .filter(|record| record.state == StepState::Skipped)
        .map(|record| json!({ "step": record.step.id(), "title": record.step.title(), "why": record.why }))
        .collect();
    let not_checked: Vec<Value> = flow
        .records
        .iter()
        .flat_map(|record| {
            record
                .not_checked
                .iter()
                .map(move |item| json!({ "step": record.step.id(), "what": item }))
        })
        .collect();
    json!({
        "lines": lines,
        "done": done,
        "skipped": skipped,
        "not_checked": not_checked,
        "checkpoints": flow.checkpoints.len(),
        "way_back": way_back(flow, flow.start_checkpoint.as_deref()),
    })
}

/// `restore_checkpoint`'s preview and apply for `checkpoint`, with the board closed in
/// KiCad first; `null` when there is no checkpoint (nothing was written).
pub(crate) fn way_back(flow: &FlowRecord, checkpoint: Option<&str>) -> Value {
    let Some(id) = checkpoint else {
        return Value::Null;
    };
    json!({
        "checkpoint_id": id,
        "first": "Close the board and the project in KiCad: restore_checkpoint refuses while KiCad has the project open, and it puts back every design file of the checkpoint.",
        "preview": { "tool": "restore_checkpoint", "arguments": { "project": flow.project, "checkpoint_id": id, "dry_run": true } },
        "apply": {
            "tool": "restore_checkpoint",
            "arguments": { "project": flow.project, "checkpoint_id": id, "dry_run": false },
            "arguments_from": { "expected_revision": "the revision the preview returned" },
        },
    })
}

#[cfg(test)]
#[path = "full_flow_tests.rs"]
mod tests;
