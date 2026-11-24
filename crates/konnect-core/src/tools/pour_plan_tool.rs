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

use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::CallToolResult;
use crate::pour_check::{CheckedPlan, FillCheck};
use crate::pour_plan::{NotProposed, PourPlan};
use crate::tool;
use crate::tools::board_source::{self, BoardSource};
use crate::tools::design_intent::{self, DesignIntent, IntentRead};
use crate::tools::{get_path, invalid_arg, ToolContext, ToolDef};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The `sources` word for what kicad-cli measured on a scratch copy.
pub(crate) const FROM_KICAD_CLI: &str = "kicad_cli";

/// The plan was filled by KiCad and nothing needs a second look.
pub(crate) const PLANNED: &str = "planned";

/// The plan was filled by KiCad, and the check is not clean, a request of the design
/// intent could not be met, or a proposal fills inside a rule area that lets pours in
/// (PP15).
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
         DRC rules whose count rose, the starved thermals, the pads of a proposed net \
         that its pour should reach and does not (`unreached`), and `clean` (no rule of \
         severity error rose, no more thermal pads are starved, and every pad the pours \
         should reach is reached). When KiCad's first fill starves a thermal pad or \
         leaves one unreached, the check fills the plan once more with smaller thermal \
         values (gap 0.3 mm, spoke 0.4 mm) and keeps the better fill; \
         `check.thermal_steps` lists each fill. \
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
    // 1. The arguments.
    let board = get_path(args, "board")?;
    if board.extension().and_then(|e| e.to_str()) != Some("kicad_pcb") {
        return Ok(invalid_arg("board", "must be a .kicad_pcb file"));
    }
    if let Err(error) = std::fs::canonicalize(&board) {
        return Ok(invalid_arg(
            "board",
            &format!("'{}' cannot be found: {error}", board.display()),
        ));
    }
    let source = match BoardSource::from_args(args) {
        Ok(source) => source,
        Err(refusal) => return Ok(refusal),
    };
    plan_pours_in(ctx, &board, source, None).await
}

/// Steps 2 to 7 of [`handle_plan_pours`]. `state` is Konnect's state folder, `None` for
/// `konnect_sexp::writer::state_root` (the tests give their own, so that no test writes
/// below the user's real state folder).
async fn plan_pours_in(
    ctx: &ToolContext,
    board: &Path,
    source: BoardSource,
    state: Option<&Path>,
) -> anyhow::Result<CallToolResult> {
    let board = board.to_path_buf();
    // 2. The board, 3. the files beside it.
    let read = match board_text(ctx, &board, source).await? {
        Ok(read) => read,
        Err(refusal) => return Ok(refusal),
    };
    let sidecars = match sidecars(&board) {
        Ok(sidecars) => sidecars,
        Err(refusal) => return Ok(refusal),
    };

    // 4. The plan, off the async threads.
    let text = read.text.clone();
    let inputs = sidecars.clone();
    let planned = tokio::task::spawn_blocking(move || plan_of(&text, &inputs)).await?;
    let mut plan = match planned {
        Ok(plan) => plan,
        Err(message) => return Ok(CallToolResult::error(message)),
    };
    let board_sha256 = crate::place_held::sha256_hex(read.text.as_bytes());
    let mut sources = json!({ "board": read.from });
    if sidecars.project_text.is_some() {
        sources["project_rules"] = json!(board_source::FROM_PROJECT_FILE);
    }

    // 5. Nothing to propose: no check.
    if plan.proposals.is_empty() {
        let mut body = result_json(&board, &board_sha256, &plan, None);
        board_source::provenance(&mut body, sources, read.evidence);
        return Ok(CallToolResult::json(&body));
    }

    // 6. KiCad's fill (PP11, PP12).
    let (folder, outcome) = match run_check(ctx, &board, &read.text, &plan, state).await {
        Ok(done) => done,
        Err(message) => return Ok(CallToolResult::error(message)),
    };
    let mut check = match outcome {
        Ok(checked) => {
            // PP16: the proposals carry the values of the fill that was kept.
            plan.proposals = checked.zones;
            checked.check
        }
        Err(error) => {
            return Ok(CallToolResult::error(format!(
                "KiCad's fill of the plan failed ({error:#}). The scratch copies are in '{}'. \
                 A plan is returned only after KiCad has filled it, so nothing is proposed.",
                folder.display()
            )))
        }
    };

    // 7. The response.
    withdraw_unfilled(&mut plan, &mut check);
    sources["check"] = json!(FROM_KICAD_CLI);
    let mut body = result_json(&board, &board_sha256, &plan, Some((&check, &folder)));
    board_source::provenance(&mut body, sources, read.evidence);
    Ok(CallToolResult::json(&body))
}

/// Step 4 of the handler: the board's facts, the roles of its nets that have a pad, the
/// project's rules and the plan. `Err` is the message of an error result: what to fix,
/// and that nothing was planned.
fn plan_of(text: &str, sidecars: &Sidecars) -> Result<PourPlan, String> {
    let facts = crate::pour_plan::board_facts(text).map_err(|error| {
        format!(
            "The board cannot be planned: {error:#}. Pours are defined before the Specctra \
             export, and the export refuses this board too. Fix that, then call plan_pours \
             again. Nothing was planned."
        )
    })?;
    let nets: BTreeSet<String> = facts.pads.keys().cloned().collect();
    let intent_text = sidecars.intent.as_ref().map(|(_, text)| text.as_str());
    let roles = crate::tools::supply_roles(&nets, intent_text, sidecars.project_text.as_deref());
    let rules =
        crate::pour_plan::project_rules(sidecars.project_text.as_deref()).map_err(|reason| {
            format!(
                "The project file beside the board cannot be read for the pours' settings \
                 ({reason}). Repair it, then call plan_pours again. Nothing was planned."
            )
        })?;
    let intent = sidecars.intent.as_ref().map(|(intent, _)| intent);
    Ok(crate::pour_plan::plan(&facts, &roles, &rules, intent))
}

/// Step 6 of the handler: the call's folder, a private copy of KiCad's settings in
/// `<folder>/kicad-config` for the length of the check, and the check. `Err` is the
/// message of an error result for a folder or a settings copy that cannot be made; the
/// check's own outcome comes back with the folder. The settings copy is removed whatever
/// the outcome.
async fn run_check(
    ctx: &ToolContext,
    board: &Path,
    text: &str,
    plan: &PourPlan,
    state: Option<&Path>,
) -> Result<(PathBuf, anyhow::Result<CheckedPlan>), String> {
    let refused = |what: String| {
        format!(
            "{what}. A plan is returned only after KiCad has filled it, so nothing is proposed."
        )
    };
    let state = match state {
        Some(state) => state.to_path_buf(),
        None => konnect_sexp::writer::state_root().map_err(|error| {
            refused(format!("Konnect's state folder cannot be found ({error})"))
        })?,
    };
    let folder = crate::pour_check::run_folder(&state, text, &stamp())
        .map_err(|error| refused(format!("The plan's folder cannot be made ({error:#})")))?;
    let config = folder.join("kicad-config");
    let _removal = RemoveOnDrop(config.clone());
    if let Err(error) = crate::route_judge::private_kicad_config(&config) {
        return Err(refused(format!(
            "The private copy of KiCad's settings in '{}' cannot be made ({error:#})",
            config.display()
        )));
    }
    let cli = crate::pour_check::KicadCli {
        kicad_cli: ctx.config.kicad_cli.clone(),
        config_home: config.clone(),
    };
    let outcome = crate::pour_check::check(&folder, board, text, &plan.proposals, &cli).await;
    Ok((folder, outcome))
}

/// Removes a folder when dropped, whatever happened before it: the settings copy is a
/// copy of the user's KiCad settings, not a record.
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Milliseconds since the Unix epoch: the first part of the call's folder name.
fn stamp() -> String {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", since.as_millis())
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
    let observation =
        board_source::read_board(ctx, board, source, "pour plan", |client, document| {
            client.save_document_to_string_in(document)
        })
        .await?;
    Ok(match observation {
        board_source::BoardRead::Refused(refusal) => Err(refusal),
        board_source::BoardRead::Live(text) => Ok(BoardText {
            text,
            from: board_source::FROM_IPC,
            evidence: board_source::live_evidence(),
        }),
        board_source::BoardRead::Saved(why) => match std::fs::read_to_string(board) {
            Ok(text) => Ok(BoardText {
                text,
                from: board_source::FROM_SAVED_BOARD,
                evidence: why.evidence(),
            }),
            Err(error) => Err(CallToolResult::error(format!(
                "The saved board '{}' cannot be read ({error}). Nothing was planned.",
                board.display()
            ))),
        },
    })
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
    let project = board.with_extension("kicad_pro");
    let project_text = match std::fs::read_to_string(&project) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            let reason = format!("unreadable: {error}");
            return Err(CallToolResult::error_kind(
                ToolErrorKind::InvalidConfiguration {
                    path: project.display().to_string(),
                    reason: reason.clone(),
                },
                format!(
                    "The project file '{}' exists and cannot be read ({reason}). The pours' \
                     settings depend on its rules, so nothing was planned. Repair it, then \
                     call plan_pours again.",
                    project.display()
                ),
            ));
        }
    };
    let project_dir = match board.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let path = design_intent::design_intent_path(project_dir);
    let intent = match design_intent::read_design_intent(&path) {
        IntentRead::Absent => None,
        IntentRead::Loaded { intent, text } => Some((*intent, text)),
        IntentRead::Unusable { reason } => return Err(design_intent::unusable(&path, reason)),
    };
    Ok(Sidecars {
        project_text,
        intent,
    })
}

/// Moves every proposal that KiCad filled nothing of from `plan.proposals` to
/// `plan.not_proposed` (the reason says that KiCad fills nothing of it on that layer;
/// `asked` is kept), drops its row from `check.fills`, so that the two lists stay in
/// step, and drops from `plan.filled_rule_areas` every row whose layer has no proposal
/// left. `check.clean`, `open` and `risen` are not changed: a zone with no fill adds no
/// copper.
pub(crate) fn withdraw_unfilled(plan: &mut PourPlan, check: &mut FillCheck) {
    let empty: Vec<(String, String)> = check
        .fills
        .iter()
        .filter(|fill| fill.pieces == 0)
        .map(|fill| (fill.net.clone(), fill.layer.clone()))
        .collect();
    let is_empty = |net: &str, layer: &str| empty.iter().any(|(n, l)| n == net && l == layer);
    let mut kept = Vec::with_capacity(plan.proposals.len());
    for zone in std::mem::take(&mut plan.proposals) {
        if is_empty(&zone.net, &zone.layer) {
            plan.not_proposed.push(NotProposed {
                reason: format!(
                    "KiCad fills nothing of a {} pour on {}: clearances, other copper and rule \
                     areas leave it no copper, so the zone would add none",
                    zone.net, zone.layer
                ),
                net: Some(zone.net),
                layer: Some(zone.layer),
                asked: zone.asked,
            });
        } else {
            kept.push(zone);
        }
    }
    plan.proposals = kept;
    check.fills.retain(|fill| !is_empty(&fill.net, &fill.layer));
    let proposals = &plan.proposals;
    plan.filled_rule_areas
        .retain(|area| proposals.iter().any(|zone| zone.layer == area.layer));
}

/// [`PLANNED`], [`PLANNED_WITH_FINDINGS`] or [`NOTHING_TO_PROPOSE`], in this order:
/// - no proposal (none was made, or KiCad's fill withdrew every one):
///   [`NOTHING_TO_PROPOSE`], whatever else is true; `not_proposed` and `next_steps`
///   carry a request that could not be met;
/// - a check that is not clean, a `not_proposed` row with `asked`, or a row of
///   `plan.filled_rule_areas`: [`PLANNED_WITH_FINDINGS`];
/// - else [`PLANNED`].
pub(crate) fn status(plan: &PourPlan, check: Option<&FillCheck>) -> &'static str {
    if plan.proposals.is_empty() {
        return NOTHING_TO_PROPOSE;
    }
    let unclean = check.is_some_and(|check| !check.clean);
    let unmet = plan.not_proposed.iter().any(|row| row.asked);
    if unclean || unmet || !plan.filled_rule_areas.is_empty() {
        PLANNED_WITH_FINDINGS
    } else {
        PLANNED
    }
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
///
/// Provenance is added by the handler.
pub(crate) fn result_json(
    board: &Path,
    board_sha256: &str,
    plan: &PourPlan,
    check: Option<(&FillCheck, &Path)>,
) -> Value {
    let fill_of = |net: &str, layer: &str| {
        check
            .and_then(|(check, _)| {
                check
                    .fills
                    .iter()
                    .find(|fill| fill.net == net && fill.layer == layer)
            })
            .map(|fill| {
                json!({
                    "pieces": fill.pieces,
                    "area_mm2": round3(fill.area_mm2),
                    "largest_piece_share": fill.largest_piece_share,
                })
            })
            .unwrap_or(Value::Null)
    };
    let proposals: Vec<Value> = plan
        .proposals
        .iter()
        .map(|zone| {
            json!({
                "net": zone.net,
                "layer": zone.layer,
                "asked": zone.asked,
                "reason": zone.reason,
                "add_zone": crate::pour_plan::add_zone_arguments(board, zone),
                "fill": fill_of(&zone.net, &zone.layer),
            })
        })
        .collect();
    let fill_check = check.map(|(check, _)| check);
    let mut body = json!({
        "status": status(plan, fill_check),
        "board": board.display().to_string(),
        "board_sha256": board_sha256,
        "stack": plan.stack,
        "nets": plan.nets,
        "proposals": proposals,
        "not_proposed": plan.not_proposed,
        "rule_areas_filled": plan.filled_rule_areas,
        "next_steps": next_steps(plan, fill_check),
    });
    if let Some((check, folder)) = check {
        body["check"] = json!({
            "clean": check.clean,
            "open": check.open,
            "risen": check.risen,
            "starved_thermals_before": check.starved_thermals_before,
            "starved_thermals": check.starved_thermals,
            "unreached": check.unreached,
            "thermal_steps": check
                .thermal_steps
                .iter()
                .map(|step| json!({
                    "thermal_gap": step.thermal_gap_mm,
                    "thermal_bridge_width": step.thermal_bridge_mm,
                    "starved_thermals": step.starved_thermals,
                    "unreached_pads": step.unreached_pads,
                    "clean": step.clean,
                    "kept": step.kept,
                }))
                .collect::<Vec<_>>(),
            "folder": folder.display().to_string(),
        });
    }
    body
}

/// A value to 3 decimals.
fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
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
    let mut steps = Vec::new();
    if !plan.proposals.is_empty() {
        steps.push(
            "Show the user this plan. On their OK call add_zone once per proposal with its \
             add_zone arguments, then refill_zones and save_project. Add the zones before \
             export_specctra_dsn or after the routed session is imported, never between the \
             two: the import checks the board against what was exported."
                .to_string(),
        );
    }
    if let Some(check) = check.filter(|check| !check.clean) {
        let mut rose: Vec<String> = check
            .risen
            .iter()
            .map(|row| {
                format!(
                    "{} ({}) {} to {}",
                    row.rule, row.severity, row.before, row.after
                )
            })
            .collect();
        if check.starved_thermals > check.starved_thermals_before {
            rose.push(format!(
                "starved thermal pads at any severity {} to {}",
                check.starved_thermals_before, check.starved_thermals
            ));
        }
        let unreached: usize = check.unreached.iter().map(|net| net.pads.len()).sum();
        if unreached > 0 {
            rose.push(format!("{unreached} pads the pours do not reach"));
        }
        steps.push(format!(
            "KiCad's check of the plan is not clean: {}. Show the user what rose; the user \
             decides whether to add the zones as they are.",
            rose.join(", ")
        ));
    }
    // PP17: the pads a pour should reach and does not, by net.
    for net in check.map(|check| check.unreached.as_slice()).unwrap_or(&[]) {
        steps.push(format!(
            "The {} pour does not reach {} on its layers: route them to the pour, or change \
             what keeps the pour from them (a clearance, a rule area, a pad's own \
             connection).",
            net.net,
            net.pads.join(", ")
        ));
    }
    // PP16: the second fill, and why.
    if let Some(check) = check.filter(|check| check.thermal_steps.len() > 1) {
        let first = &check.thermal_steps[0];
        let second = &check.thermal_steps[1];
        let why = format!(
            "KiCad's first fill (thermal gap {} mm, spoke {} mm) left {} thermal pads starved \
             and {} pads unreached",
            first.thermal_gap_mm,
            first.thermal_bridge_mm,
            first.starved_thermals,
            first.unreached_pads
        );
        steps.push(if second.kept {
            format!(
                "{why}, so the plan uses the smaller thermal values, gap {} mm and spoke {} mm \
                 (the owner's own on B5), which KiCad fills with fewer faults. Each \
                 proposal's add_zone arguments carry them.",
                second.thermal_gap_mm, second.thermal_bridge_mm
            )
        } else {
            format!(
                "{why}. A second fill at gap {} mm and spoke {} mm was no better, so the plan \
                 keeps the first values.",
                second.thermal_gap_mm, second.thermal_bridge_mm
            )
        });
    }
    for area in &plan.filled_rule_areas {
        let label = match (&area.name, &area.uuid) {
            (Some(name), _) => format!("'{name}'"),
            (None, Some(uuid)) => format!("with UUID {uuid}"),
            (None, None) => "with no name".to_string(),
        };
        steps.push(format!(
            "The rule area {label} on {layer} keeps {forbids} out and lets copper pours in, so \
             the {layer} pour fills inside it. If copper must stay out of it (an antenna \
             keepout, say), delete_zone the area and add_rule_area it again with \
             keepout.copperpour true, then call plan_pours again.",
            layer = area.layer,
            forbids = area.forbids.join(", "),
        ));
    }
    let by_name: Vec<&str> = plan
        .nets
        .iter()
        .filter(|net| net.source == "name")
        .map(|net| net.net.as_str())
        .collect();
    if !by_name.is_empty() {
        steps.push(format!(
            "The roles of {} come from their names only. Confirm them with the user, or record \
             the rails with set_design_intent (section 'rails').",
            by_name.join(", ")
        ));
    }
    for row in plan.not_proposed.iter().filter(|row| row.asked) {
        let what = match (&row.net, &row.layer) {
            (Some(net), Some(layer)) => format!("{net} on {layer}"),
            (Some(net), None) => net.clone(),
            (None, Some(layer)) => layer.clone(),
            (None, None) => "a pour".to_string(),
        };
        steps.push(format!(
            "The design intent asks for {what}, and it is not met: {}.",
            row.reason
        ));
    }
    if plan.proposals.is_empty() {
        let no_ground = !plan
            .nets
            .iter()
            .any(|net| net.role == crate::pour_plan::SupplyRole::Ground);
        let no_rule = plan
            .not_proposed
            .iter()
            .any(|row| row.net.is_none() && row.layer.is_none());
        steps.push(if no_ground || no_rule {
            "Nothing is proposed. To change that, record the board's ground net as a 0 V rail \
             with set_design_intent (section 'rails'), or ask for pours by net and layer with \
             set_design_intent (section 'pours'), then call plan_pours again."
                .to_string()
        } else {
            "Nothing is proposed: not_proposed gives the reason for each pour that was \
             considered."
                .to_string()
        });
    }
    steps
}

#[cfg(test)]
#[path = "pour_plan_tool_tests.rs"]
mod tests;
