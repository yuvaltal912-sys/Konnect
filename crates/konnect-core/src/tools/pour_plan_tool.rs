//! `plan_pours`: the pours a board should have, as a read-only planner (Row ZR, stage
//! ZR4, `docs/POUR_PLAN_DESIGN.md`).
//!
//! The tool reads the board (as KiCad holds it, else the saved file), the project file
//! and the design-intent record, has `pour_plan` divide the copper layers among the
//! board's supply nets, has KiCad fill that plan on a scratch copy (`pour_check`), and
//! returns it as the exact arguments of `add_zone`. It writes nothing to the board
//! (PP1): the AI shows the plan and adds each zone through `add_zone` only on the user's
//! OK. This module holds the input side (the board, the sidecar files) and the output
//! side (the response); every rule of the plan is `pour_plan`'s and every measure
//! `pour_check`'s.

use crate::mcp::protocol::CallToolResult;
use crate::pour_check::FillCheck;
use crate::pour_plan::PourPlan;
use crate::tool;
use crate::tools::board_source::{self, BoardSource};
use crate::tools::design_intent::DesignIntent;
use crate::tools::{ToolContext, ToolDef};
use serde_json::{json, Value};
use std::path::Path;

/// The `sources` word for what kicad-cli measured on a scratch copy.
pub(crate) const FROM_KICAD_CLI: &str = "kicad_cli";

/// The plan was filled by KiCad and nothing needs a second look.
pub(crate) const PLANNED: &str = "planned";

/// The plan was filled by KiCad, and the check is not clean or a request of the design
/// intent could not be met.
pub(crate) const PLANNED_WITH_FINDINGS: &str = "planned_with_findings";

/// No zone is proposed; `not_proposed` says why.
pub(crate) const NOTHING_TO_PROPOSE: &str = "nothing_to_propose";

// ─── Tool definition ─────────────────────────────────────────────────────────

pub(crate) fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "plan_pours",
        "Plan the copper pours a board should have. Read-only: nothing on the board \
         changes. Reads the board as KiCad holds it when it is open there, else the saved \
         file (`board_source`), and the project file and the design-intent record beside \
         it. By default it divides the copper layers among the board's ground and power \
         nets, whose roles are the layout audit's (design-intent rails, then netclasses, \
         then net names; `nets` gives each role's source): on a 2-layer board the ground \
         net with the most pads on both layers; on a 4-layer board that net on F.Cu, \
         In1.Cu and B.Cu and the power net with the most pads on In2.Cu; any other layer \
         count gets only what the design intent asks for. The design intent's `pours` \
         section is the explicit request and wins: a net with layers takes those layers, \
         and a net without layers takes the rule's layer of its kind. Whole-layer pours \
         on the board's own outline, one net a layer. Existing zones are never changed: a \
         layer that already has a pour of the net is left out, and so is a layer where \
         another net's pour sits at priority 0. Before anything is returned the plan is \
         filled by KiCad on a scratch copy in Konnect's state folder (kicad-cli refill \
         and DRC of the board with and without the proposals; seconds). `proposals` has \
         one entry per zone: `add_zone` (the exact arguments of add_zone), `reason`, and \
         `fill` (pieces, area and the largest piece's share, as KiCad filled it). \
         `not_proposed` names what was considered and left out, with the reason; \
         `asked: true` there is a request of the design intent that could not be met. \
         `check` gives KiCad's unconnected items per proposed net before and after, the \
         DRC rules whose count rose, the starved thermals, and `clean` (no rule of \
         severity error rose and no more thermal pads are starved). \
         `rule_areas_filled` lists every rule area on a proposed layer that keeps \
         tracks, vias or pads out and lets copper pours in: KiCad fills the pour inside \
         it. `status`: planned, planned_with_findings (the check is not clean, a request \
         could not be met, or a pour fills inside such a rule area) or \
         nothing_to_propose. Show the user the plan and call add_zone once per proposal \
         only on their OK, then refill_zones and save_project. Add the zones before \
         export_specctra_dsn or after the routed session is imported, never between the \
         two: the import checks the board against what was exported.",
        json!({
            "type": "object",
            "properties": {
                "board": { "type": "string", "description": "Path to the .kicad_pcb file" },
                "board_source": board_source::board_source_schema()
            },
            "required": ["board"]
        }),
        |args, ctx| async move { handle_plan_pours(args, ctx).await }
    )
    .with_board_access(crate::tools::BoardAccess::LivePreferredWithFallback)]
}

// ─── The handler ─────────────────────────────────────────────────────────────

/// The call:
/// 1. `board` (a `.kicad_pcb` path that resolves) and `board_source`.
/// 2. [`board_text`].
/// 3. [`sidecars`].
/// 4. Off the async threads: `pour_plan::board_facts`, `tools::supply_roles` over the
///    nets that have a pad, `pour_plan::project_rules`, `pour_plan::plan`. A board the
///    export refuses, and a project file that does not read, are error results that say
///    what to fix and that nothing was planned.
/// 5. A plan with no proposal is returned as [`NOTHING_TO_PROPOSE`], with no check.
/// 6. The check (PP11): `konnect_sexp::writer::state_root`, `pour_check::run_folder`
///    (its stamp is the milliseconds since the Unix epoch, as `placement_converge` stamps
///    its folders), `route_judge::private_kicad_config` into `<folder>/kicad-config`,
///    then `pour_check::check` with `pour_check::KicadCli` (kicad-cli is
///    `ctx.config.kicad_cli`). `<folder>/kicad-config` is removed when the check ends,
///    whatever its outcome, as the judge removes its own (`route_judge::OwnConfig`): it
///    is a copy of the user's KiCad settings, not a record. A check that fails is an
///    error result: it names the step and the folder, and says that a plan is returned
///    only after KiCad has filled it, so nothing is proposed (PP12).
/// 7. [`withdraw_unfilled`], then [`result_json`] with the read's provenance
///    (`board_source::provenance`).
async fn handle_plan_pours(args: &Value, ctx: &ToolContext) -> anyhow::Result<CallToolResult> {
    todo!()
}

/// The board's text and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BoardText {
    pub text: String,
    /// `board_source::FROM_IPC` or `board_source::FROM_SAVED_BOARD`.
    pub from: &'static str,
    /// The observation behind it (`board_source::live_evidence`, or the saved read's
    /// own evidence).
    pub evidence: Value,
}

/// The board through the board-source seam: KiCad's own text of the exact board when it
/// is open there (`save_document_to_string_in`), else the saved file with the seam's
/// reason. A refusal of the seam is returned as it is.
async fn board_text(
    ctx: &ToolContext,
    board: &Path,
    source: BoardSource,
) -> anyhow::Result<Result<BoardText, CallToolResult>> {
    todo!()
}

/// The files beside the board that a plan reads.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sidecars {
    /// `<stem>.kicad_pro`, when there is one.
    pub project_text: Option<String>,
    /// The design-intent record and its exact text, when there is one.
    pub intent: Option<(DesignIntent, String)>,
}

/// The sidecars of `board`. A project file that exists and cannot be read, and a
/// design-intent record that exists and cannot be used (`design_intent::unusable`), are
/// refusals: a plan never guesses past a file that is there.
fn sidecars(board: &Path) -> Result<Sidecars, CallToolResult> {
    todo!()
}

/// Moves every proposal that KiCad filled nothing of from `plan.proposals` to
/// `plan.not_proposed` (the reason says that KiCad fills nothing of it on that layer;
/// `asked` is kept), drops its row from `check.fills`, so that the two lists stay in
/// step, and drops from `plan.filled_rule_areas` every row whose layer has no proposal
/// left. `check.clean`, `open` and `risen` are not changed: a zone with no fill adds no
/// copper.
pub(crate) fn withdraw_unfilled(plan: &mut PourPlan, check: &mut FillCheck) {
    todo!()
}

/// [`PLANNED`], [`PLANNED_WITH_FINDINGS`] or [`NOTHING_TO_PROPOSE`], in this order:
/// - no proposal (none was made, or KiCad's fill withdrew every one):
///   [`NOTHING_TO_PROPOSE`], whatever else is true; `not_proposed` and `next_steps`
///   carry a request that could not be met;
/// - a check that is not clean, a `not_proposed` row with `asked`, or a row of
///   `plan.filled_rule_areas`: [`PLANNED_WITH_FINDINGS`];
/// - else [`PLANNED`].
pub(crate) fn status(plan: &PourPlan, check: Option<&FillCheck>) -> &'static str {
    todo!()
}

/// The response:
/// - `status`, `board`, `board_sha256` (of the text the plan was made from);
/// - `stack`: `[{name, kind}]`; `nets`: `[{net, role, source, pads}]`;
/// - `proposals`: `[{net, layer, asked, reason, add_zone, fill}]`, `add_zone` from
///   `pour_plan::add_zone_arguments` and `fill` the proposal's row of `check.fills`
///   (`{pieces, area_mm2, largest_piece_share}`, areas to 3 decimals);
/// - `not_proposed`: `[{net, layer, reason, asked}]`;
/// - `rule_areas_filled`: `plan.filled_rule_areas` as `[{layer, name, uuid, forbids}]`;
/// - `check`, whenever one ran (also when it withdrew every proposal): `{clean, open,
///   risen, starved_thermals_before, starved_thermals, folder}`;
/// - `next_steps` ([`next_steps`]).
/// Provenance is added by the handler.
pub(crate) fn result_json(
    board: &Path,
    board_sha256: &str,
    plan: &PourPlan,
    check: Option<(&FillCheck, &Path)>,
) -> Value {
    todo!()
}

/// What to do with the result, each line naming only tools and arguments of the
/// registry:
/// - proposals: show the user the plan; on their OK call `add_zone` once per proposal
///   with its `add_zone` arguments, then `refill_zones` and `save_project`. The zones go
///   in before `export_specctra_dsn` or after the routed session is imported, never
///   between the two (the import checks the board against what was exported). Which of
///   the two orders to use is the skill's to say, not this tool's;
/// - a check that is not clean: the rules that rose, and that the user decides;
/// - a row of `filled_rule_areas`: the area, what it keeps out, and that the pour of
///   that layer fills inside it; when copper must stay out of it, `delete_zone` the area
///   and `add_rule_area` it again with `keepout.copperpour` true, then call `plan_pours`
///   again;
/// - a role that comes from a net's name: confirm it with the user, or record the rails
///   with `set_design_intent`;
/// - a request that could not be met (`asked` in `not_proposed`): what it asked for and
///   why not;
/// - no proposal: what `not_proposed` asks for (a ground rail in the design intent's
///   `rails`, or a `pours` request).
fn next_steps(plan: &PourPlan, check: Option<&FillCheck>) -> Vec<String> {
    todo!()
}

#[cfg(test)]
#[path = "pour_plan_tool_tests.rs"]
mod tests;
