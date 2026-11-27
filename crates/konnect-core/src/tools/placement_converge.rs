//! `plan_convergent_placement`: family C, the convergent placement search, as a
//! read-only planner (Row CT, `docs/CONVERGENT_PLACEMENT_TOOL_DESIGN.md`).
//!
//! The tool runs the placement bench's own search (`place_search::search`) on a
//! scratch copy of the board that is open in KiCad: families F and S make the starts,
//! family C refines them until its stop rules end it, and family C's first three
//! candidates are routed with fastroute and judged by the bench's gates and rank key.
//! It returns the winner as the exact arguments of `set_component_placements` and
//! writes nothing to the board (decision T1): the AI shows the plan and applies it
//! through that tool only on the user's OK. Every value of the search is the bench's
//! (T7); this module adds only the input side (the live board, KiCad's rules, the
//! scratch project) and the output side (the winner, the response).

use crate::fastroute::{self, Binary, Located};
use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::CallToolResult;
use crate::place_converge::{ConvergeParams, OuterRule};
use crate::place_generators::{BoardModel, Family, Matrix};
use crate::place_held::{sha256_hex, HoldRules};
use crate::place_judge::RouteResult;
use crate::place_route::{export_options, isolated_context, strip_routing, RouteInputs};
use crate::place_search::{
    rebuild, search, Outcome, PlaceRouteCall, Plan, Record, RouteCall, Routed,
};
use crate::route_search::Effort;
use crate::tool;
use crate::tools::{get_path, invalid_arg, with_board_ipc_classified, ToolContext, ToolDef};
use anyhow::Context;
use konnect_ipc::types::{IpcEffectiveRoutingRules, IpcFootprintPlacement};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Family C's candidates that are routed before a winner is named (T2, T8).
pub(crate) const ROUTED_CANDIDATES: usize = 3;

/// The routing effort of each call when the call names none (T2): the routing here
/// ranks placements, it is not the board's final routing.
const DEFAULT_EFFORT: Effort = Effort::M;

/// A bound the three calls never reach (at `xh`, 3 x 2460 s): each call's own budget
/// ends it, and this only keeps the launch rule's arithmetic finite (T8).
const TIME_GUARD: Duration = Duration::from_secs(24 * 3600);

/// The most lines a response lists of reasons or gate failures.
const MAX_LINES: usize = 8;

/// The folder below Konnect's state folder that holds one run folder per call (T5).
const PLANS_FOLDER: &str = "placement-plans";

// ─── Tool definition ─────────────────────────────────────────────────────────

pub(crate) fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "plan_convergent_placement",
        "Plan a whole-board footprint placement with Konnect's convergent placement search. \
         Read-only: nothing on the board changes. The board must be open in KiCad, and the \
         only board open: its live text and KiCad's own routing rules are read at the start \
         (twice, and both reads must agree). Every footprint KiCad marks locked stays where \
         it is (lock the parts that are fixed first), and so does a part with no pad on a \
         net; every other part may move and turn. On a scratch copy in Konnect's state folder, two generator \
         families make the starts, the convergent search refines the best of them until \
         its stop rules end it (it stops when the cost stops falling, not after a time or a \
         count), and its first three candidates are routed with fastroute at `effort` \
         (default m, 240 s per call) and ranked by the placement bench's gates and rank \
         key, so expect minutes (about 12 to 15 at m). The result's `apply_with` holds the \
         winner as the exact arguments of set_component_placements; `winner` gives its \
         cost terms, its route numbers and the gates it fails, `candidates` one line per \
         routed candidate, `search` the stop rules, and `records` the run folder. \
         `status`: planned, planned_with_gate_failures, no_candidate or not_routed (the \
         last two carry no placements and say why). Show the user the plan and call \
         set_component_placements only on their OK.",
        json!({
            "type": "object",
            "properties": {
                "board": { "type": "string", "description": "Path to the .kicad_pcb that is open in KiCad" },
                "effort": {
                    "type": "string",
                    "enum": ["fast", "m", "h", "xh"],
                    "default": "m",
                    "description": "The routing effort of each of the three ranking calls: fast = 2 runs at once, m = 240 s, h = 720 s, xh = 2400 s."
                },
                "fastroute_path": {
                    "type": "string",
                    "description": "The fastroute binary. Default as route_specctra_dsn: the environment variable KONNECT_FASTROUTE, else the user config key routing.fastroute_path, else a fastroute file next to the konnect executable, else ~/.local/share/fastroute/candidate/fastroute."
                }
            },
            "required": ["board"]
        }),
        |args, ctx| async move { handle_plan_convergent_placement(args, ctx).await }
    )
    .with_board_access(crate::tools::BoardAccess::LiveOnly)]
}

// ─── The handler ─────────────────────────────────────────────────────────────

/// The call: the arguments, the live read (T3), the run folder (T5), fastroute, the
/// scratch project, the plan (T4, T7, T8), the search, the verdict (T9, T10) and the
/// response (T11). Every refusal before the search names what to fix and says that
/// nothing was planned; the run folder is kept whatever happens after it is made.
async fn handle_plan_convergent_placement(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let call = match read_arguments(args) {
        Ok(call) => call,
        Err(refusal) => return Ok(refusal),
    };
    let board = match call.board.canonicalize() {
        Ok(board) => board,
        Err(error) => {
            return Ok(CallToolResult::error_kind(
                ToolErrorKind::FileNotFound {
                    path: call.board.display().to_string(),
                },
                format!(
                    "The board '{}' cannot be resolved: {error}. Nothing was planned.",
                    call.board.display()
                ),
            ))
        }
    };
    let live = match live_input(ctx, &board).await? {
        Ok(live) => live,
        Err(refusal) => return Ok(refusal),
    };
    let state = konnect_sexp::writer::state_root()
        .map_err(|error| anyhow::anyhow!("Konnect's state folder: {error}"))?;
    let folder = run_folder(&state, &live.text, &stamp())?;
    let binary = match locate_fastroute(call.fastroute_path.as_deref(), &folder).await? {
        Ok(binary) => binary,
        Err(refusal) => return Ok(refusal),
    };
    let project_board = scratch_project(&folder, &board, &live.text)?;
    let live_text = live.text.clone();
    let kicad_config_home = folder.join("kicad-config");
    std::fs::create_dir(&kicad_config_home)
        .with_context(|| format!("create '{}'", kicad_config_home.display()))?;
    let inputs = RouteInputs {
        options: export_options(&project_board),
        board: project_board,
        rules: live.rules,
        effort: call.effort,
        fastroute: binary.path.clone(),
        kicad_cli: ctx.config.kicad_cli.clone(),
        kicad_config_home,
    };
    let schematic = Some(board.with_extension("kicad_sch")).filter(|root| root.is_file());
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let plan = tool_plan(&folder, inputs.clone(), schematic, threads);
    let isolated = isolated_context(&folder.join("isolation"), &ctx.config.kicad_cli)?;
    let routing: Arc<dyn RouteCall> = Arc::new(PlaceRouteCall {
        inputs,
        ctx: isolated,
    });
    let planned = Planned::of(&live_text)?;
    let mut body = plan_and_judge(&plan, routing, &board, &planned).await?;
    body["fastroute"] = json!({
        "path": binary.path.display().to_string(),
        "version": binary.version,
        "source": binary.source,
    });
    Ok(CallToolResult::json(&body))
}

/// The arguments as the handler reads them.
#[derive(Debug, Clone, PartialEq)]
struct Call {
    board: PathBuf,
    effort: Effort,
    fastroute_path: Option<PathBuf>,
}

/// `board` (a `.kicad_pcb` path), `effort` (`fast`, `m`, `h` or `xh`, default `m`)
/// and `fastroute_path` (a non-empty path). Anything else is refused by the closed
/// schema before the handler runs.
fn read_arguments(args: &Value) -> Result<Call, CallToolResult> {
    let board =
        get_path(args, "board").map_err(|error| invalid_arg("board", &format!("{error:#}")))?;
    if board
        .extension()
        .is_none_or(|extension| extension != "kicad_pcb")
    {
        return Err(invalid_arg("board", "must name a .kicad_pcb file"));
    }
    let effort = match args.get("effort") {
        None => DEFAULT_EFFORT,
        Some(value) => value
            .as_str()
            .and_then(Effort::parse)
            .ok_or_else(|| invalid_arg("effort", "must be one of fast, m, h, xh"))?,
    };
    let fastroute_path = match args.get("fastroute_path") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .filter(|text| !text.is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| invalid_arg("fastroute_path", "must be a non-empty path"))?,
        ),
    };
    Ok(Call {
        board,
        effort,
        fastroute_path,
    })
}

// ─── The live read (T3) ──────────────────────────────────────────────────────

/// The board as KiCad holds it, and KiCad's effective routing rules for its nets.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LiveInput {
    pub text: String,
    pub rules: IpcEffectiveRoutingRules,
}

/// The live text and rules, read twice in one IPC session (`stable_read`). A board that
/// is not open, or a KiCad that cannot be reached, is a refusal: the routing export needs
/// KiCad's own net-class resolution, and the live text holds the user's locks even
/// before they are saved.
async fn live_input(
    ctx: &ToolContext,
    board: &Path,
) -> anyhow::Result<Result<LiveInput, CallToolResult>> {
    let read = with_board_ipc_classified(ctx, board, move |client, document| {
        // KiCad resolves the net classes without naming a document, so with a second
        // board open the rules could be that board's (as `export_specctra_dsn` guards).
        if client.get_open_board_paths()?.len() != 1 {
            return Ok(Err(
                "KiCad has more than one board open, and its routing rules are read without naming a board. Close the other boards and call again. Nothing was planned."
                    .to_string(),
            ));
        }
        let before = client.save_document_to_string_in(document.clone())?;
        let rules = client.get_effective_routing_rules_in(document.clone())?;
        let after = client.save_document_to_string_in(document.clone())?;
        let rules_after = client.get_effective_routing_rules_in(document)?;
        Ok(stable_read(before, after, rules, rules_after))
    })
    .await?;
    Ok(match read {
        Ok(Ok(live)) => Ok(live),
        Ok(Err(reason)) => Err(CallToolResult::error(reason)),
        Err(konnect_ipc::IpcFailure::Unreachable(reason)) => Err(CallToolResult::error_kind(
            ToolErrorKind::EditorUnavailable {
                editor: "pcb".to_string(),
                reason: reason.to_string(),
            },
            format!(
                "KiCad cannot be reached ({reason}). The plan reads the live board and KiCad's own routing rules, so the board must be open in KiCad's PCB editor: open '{}' there and call again. Nothing was planned.",
                board.display()
            ),
        )),
        Err(konnect_ipc::IpcFailure::Target { error, .. }) => {
            Err(crate::tools::ipc_target_error_result(&error))
        }
        Err(konnect_ipc::IpcFailure::Rejected(message)) => Err(CallToolResult::error(format!(
            "KiCad did not serve the board: {message}. Open it in KiCad's PCB editor and call again. Nothing was planned."
        ))),
        Err(konnect_ipc::IpcFailure::Recovered(message)) => {
            Err(crate::tools::ipc_recovered_result(&message))
        }
        Err(konnect_ipc::IpcFailure::Uncertain(message)) => {
            Err(crate::tools::ipc_uncertain_result(&message))
        }
    })
}

/// The two reads agree (text with text, rules with rules), or the reason they do not.
pub(crate) fn stable_read(
    before: String,
    after: String,
    rules: IpcEffectiveRoutingRules,
    rules_after: IpcEffectiveRoutingRules,
) -> Result<LiveInput, String> {
    if before != after {
        return Err(
            "The board changed in KiCad while it was read. Call again once it is still. Nothing was planned."
                .to_string(),
        );
    }
    if rules != rules_after {
        return Err(
            "KiCad's routing rules changed while they were read. Call again once they are still. Nothing was planned."
                .to_string(),
        );
    }
    Ok(LiveInput {
        text: before,
        rules,
    })
}

// ─── The run folder and the scratch project (T5) ─────────────────────────────

/// Milliseconds since the Unix epoch, the run folder's first part.
fn stamp() -> String {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", since.as_millis())
}

/// A new folder `<state>/placement-plans/<stamp>-<12 hex of the text's SHA-256>/`.
/// Errs when it is there already: no run writes into another's folder.
pub(crate) fn run_folder(state: &Path, text: &str, stamp: &str) -> anyhow::Result<PathBuf> {
    let plans = state.join(PLANS_FOLDER);
    std::fs::create_dir_all(&plans).with_context(|| format!("create '{}'", plans.display()))?;
    let digest = sha256_hex(text.as_bytes());
    let folder = plans.join(format!("{stamp}-{}", &digest[..12]));
    std::fs::create_dir(&folder)
        .with_context(|| format!("create the run folder '{}'", folder.display()))?;
    Ok(folder)
}

/// `<folder>/project/`: the files a judged copy of `board` needs, copied from the
/// board's own folder by the judge's own `scratch_board::copy_project` (project, rules,
/// library tables, the design intent, project libraries), and the live `text` with its
/// routing stripped (`place_route::strip_routing`) as the board, named like it. Reads
/// the originals and writes only below `folder`. Returns the scratch board's path.
pub(crate) fn scratch_project(folder: &Path, board: &Path, text: &str) -> anyhow::Result<PathBuf> {
    let project = folder.join("project");
    let copy = crate::scratch_board::copy_project(board, &project)?;
    std::fs::write(&copy, strip_routing(text)?)
        .with_context(|| format!("write '{}'", copy.display()))?;
    Ok(copy)
}

/// The fastroute binary, found as `route_specctra_dsn` finds it (the argument, the
/// environment variable, the user config, the packaged binary, the home default), and
/// probed in `<folder>/probe`. Anything but a usable binary is a refusal with
/// `route_specctra_dsn`'s own setup guidance.
async fn locate_fastroute(
    argument: Option<&Path>,
    folder: &Path,
) -> anyhow::Result<Result<Binary, CallToolResult>> {
    let environment = std::env::var(fastroute::PATH_VARIABLE)
        .ok()
        .filter(|value| !value.is_empty());
    let config = if argument.is_some() || environment.is_some() {
        None
    } else {
        match crate::tools::route_fastroute::user_routing_config() {
            Ok((_, path)) => path,
            Err(error) => {
                return Ok(Err(CallToolResult::error(format!(
                    "The user configuration cannot be used for routing ({error:#}). Repair or move the file, or give fastroute_path. Nothing was planned."
                ))))
            }
        }
    };
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| fastroute::bundled_path(&exe));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let scratch = folder.join("probe");
    std::fs::create_dir(&scratch).with_context(|| format!("create '{}'", scratch.display()))?;
    let located = fastroute::locate(
        argument,
        environment.as_deref(),
        config.as_deref(),
        bundled.as_deref(),
        home.as_deref(),
        &scratch,
    )
    .await;
    Ok(match located {
        Located::Found(binary) => Ok(binary),
        other => Err(crate::tools::route_fastroute::unavailable_result(&other)),
    })
}

// ─── The plan (T4, T6, T7, T8) ───────────────────────────────────────────────

/// The bench's search as the tool runs it: the matrix of F and S with family C at its
/// measured parameters (T7), the user's KiCad locks as the held set (T4), no reference
/// call, family C's first `ROUTED_CANDIDATES` by its own order and none drawn, only
/// family C routed (T8), and the schematic root given (T6).
pub(crate) fn tool_plan(
    folder: &Path,
    route: RouteInputs,
    schematic: Option<PathBuf>,
    threads: usize,
) -> Plan {
    let mut matrix = Matrix::bench().only(&[Family::Force, Family::Schematic]);
    matrix.converge = Some((ConvergeParams::measured(0), OuterRule::measured()));
    Plan {
        name: "plan".to_string(),
        folder: folder.to_path_buf(),
        matrix,
        threads,
        reference_calls: 0,
        best_per_family: ROUTED_CANDIDATES,
        drawn_per_family: 0,
        draw_seed: 0,
        time_limit: TIME_GUARD,
        route,
        ai_board: None,
        caller_held: BTreeSet::new(),
        hold: HoldRules::locks_only(),
        schematic,
        routed: Routed::ConvergeOnly,
    }
}

/// What the live read says besides the text: the text's SHA-256 and how many tracks,
/// arcs, vias and teardrop zones it holds (the scratch copy has none; the user's board
/// keeps them).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Planned {
    pub sha256: String,
    pub routing_items: usize,
}

impl Planned {
    pub(crate) fn of(text: &str) -> anyhow::Result<Self> {
        let tree = konnect_sexp::parse_sexp(text).context("the board text does not parse")?;
        // What `strip_routing` leaves out of the scratch copy: tracks, arcs, vias and
        // teardrop zones.
        let routing_items = ["segment", "arc", "via"]
            .iter()
            .map(|head| tree.find_all(head).len())
            .sum::<usize>()
            + tree
                .find_all("zone")
                .into_iter()
                .filter(|zone| crate::specctra::is_teardrop(zone))
                .count();
        Ok(Planned {
            sha256: sha256_hex(text.as_bytes()),
            routing_items,
        })
    }
}

/// The search of `plan` with `routing`, then the verdict and the response's body for the
/// user's `board`. The search is awaited on this task: phases 1 and 1b are CPU work that
/// runs inline (as in the bench), and a cancelled call drops the routing calls with it
/// (their router processes are killed when dropped). The winner is rebuilt from the
/// search's own model, never from a second read of the files.
pub(crate) async fn plan_and_judge(
    plan: &Plan,
    routing: Arc<dyn RouteCall>,
    board: &Path,
    planned: &Planned,
) -> anyhow::Result<Value> {
    let outcome = search(plan, routing).await?;
    let verdict = verdict_of(&outcome.model, &outcome)?;
    Ok(result_json(plan, &outcome, &verdict, board, planned))
}

// ─── The verdict (T9, T10) ───────────────────────────────────────────────────

/// What the tool concludes from one search.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Verdict {
    /// `planned`, `planned_with_gate_failures`, `no_candidate` or `not_routed`.
    pub status: &'static str,
    /// The winner's record id and its placements; only for the two `planned` statuses.
    pub winner: Option<(usize, Vec<IpcFootprintPlacement>)>,
    /// Every gate the winner fails, as `P1: ...` to `G4: ...` lines.
    pub gate_failures: Vec<String>,
    /// Why there is no winner: the records' own reasons, without repeats, and why the
    /// routing phase stopped when it did.
    pub reasons: Vec<String>,
}

/// The routed record of family C with the best rank is the winner; its placements are
/// rebuilt from its recipe, and `rebuild` errs unless the rebuilt board is the judged one
/// (T9). Without a routed one: `no_candidate` when no record of family C was ever sent to
/// the router, else `not_routed` (T10).
pub(crate) fn verdict_of(model: &BoardModel, outcome: &Outcome) -> anyhow::Result<Verdict> {
    let converged: Vec<&Record> = outcome
        .records
        .iter()
        .filter(|record| record.group() == "C")
        .collect();
    let best = converged
        .iter()
        .filter(|record| record.outcome == "routed")
        .filter_map(|record| record.rank.map(|rank| (rank, *record)))
        .min_by_key(|(rank, record)| (*rank, record.id))
        .map(|(_, record)| record);
    if let Some(record) = best {
        let (candidate, _) = rebuild(model, record)?;
        let gate_failures = gate_failures(record);
        let status = if gate_failures.is_empty() {
            "planned"
        } else {
            "planned_with_gate_failures"
        };
        return Ok(Verdict {
            status,
            winner: Some((record.id, candidate.placements)),
            gate_failures,
            reasons: Vec::new(),
        });
    }
    let sent = converged.iter().any(|record| was_sent(record.outcome));
    // With a record sent to the router, the reasons are those records' own; the others
    // were only not among the first three. Without one, every record's reason says why.
    let explained: Vec<&Record> = converged
        .iter()
        .copied()
        .filter(|record| !sent || was_sent(record.outcome))
        .collect();
    let mut reasons: Vec<String> = Vec::new();
    if converged.is_empty() {
        reasons.push(
            "families F and S made no candidate that passes P1, so family C had no start"
                .to_string(),
        );
    }
    for record in &explained {
        let reason = match (&record.reason, record.outcome) {
            (Some(reason), _) => reason.clone(),
            (None, "screened_out") => "a candidate of family C fails P1: a hard failure of the placement score (a courtyard overlap, a part outside the outline) names a part it moved".to_string(),
            (None, "not_routed_stopped") => {
                "a candidate of family C was not routed: the routing phase stopped".to_string()
            }
            (None, outcome) => format!("a candidate of family C ended '{outcome}'"),
        };
        if !reasons.contains(&reason) {
            reasons.push(reason);
        }
    }
    if let Some(stopped) = &outcome.stopped_routing {
        reasons.push(format!("the routing phase stopped: {stopped}"));
    }
    reasons.truncate(MAX_LINES);
    Ok(Verdict {
        status: if sent { "not_routed" } else { "no_candidate" },
        winner: None,
        gate_failures: Vec::new(),
        reasons,
    })
}

/// Whether a record with `outcome` had a routing call made for it.
fn was_called(outcome: &str) -> bool {
    matches!(
        outcome,
        "routed" | "no_winner" | "export_refused" | "route_failed"
    )
}

/// Whether a record with `outcome` was handed to the routing phase.
fn was_sent(outcome: &str) -> bool {
    matches!(
        outcome,
        "routed"
            | "no_winner"
            | "export_refused"
            | "route_failed"
            | "not_routed_time_limit"
            | "not_routed_stopped"
    )
}

/// The record's failing gates: P1 and P2 of the placement, then G1 to G4 of the route.
fn gate_failures(record: &Record) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(placement) = &record.placement_gates {
        lines.extend(placement.p1.iter().map(|line| format!("P1: {line}")));
        lines.extend(placement.p2.iter().map(|line| format!("P2: {line}")));
    }
    if let Some(gates) = &record.route_gates {
        lines.extend(gates.g1.iter().map(|line| format!("G1: {line}")));
        lines.extend(gates.g2.iter().map(|line| format!("G2: {line}")));
        lines.extend(gates.g3.iter().map(|line| format!("G3: {line}")));
        lines.extend(gates.g4.iter().map(|line| format!("G4: {line}")));
    }
    lines
}

// ─── The response (T11) ──────────────────────────────────────────────────────

/// The response's body: counts and numbers, as `search.json` gives them, plus the
/// winner's placements as `set_component_placements` arguments for `board`, and the
/// state of the board they were planned from (`planned`).
pub(crate) fn result_json(
    plan: &Plan,
    outcome: &Outcome,
    verdict: &Verdict,
    board: &Path,
    planned: &Planned,
) -> Value {
    let model = &outcome.model;
    let converged: Vec<&Record> = outcome
        .records
        .iter()
        .filter(|record| record.group() == "C")
        .collect();
    let mut called: Vec<&Record> = converged
        .iter()
        .copied()
        .filter(|record| was_called(record.outcome))
        .collect();
    called.sort_by_key(|record| (record.rank.unwrap_or(usize::MAX), record.id));
    let starts: Vec<&Record> = outcome
        .records
        .iter()
        .filter(|record| matches!(record.group(), "F" | "S"))
        .collect();
    // The starts family C could take: generated (later screened out, as nothing of F or
    // S is routed) and passing P1; a duplicate is no start.
    let usable = starts
        .iter()
        .filter(|r| matches!(r.outcome, "generated" | "screened_out"))
        .filter(|r| r.screen_key.is_some_and(|key| key[0] == 0))
        .count();
    let mut body = json!({
        "status": verdict.status,
        "board": board.display().to_string(),
        "source": "ipc",
        "planned_from": {
            "sha256": planned.sha256,
            "routing_items": planned.routing_items,
        },
        "parts": {
            "held": model.held.held.len(),
            "movable": model.held.movable.len(),
            "decoupling_pairs": model.intent.pairs.len(),
        },
        "search": {
            "start_recipes": starts.len(),
            "usable_starts": usable,
            "converge_runs": converged.len(),
            "routing_calls": called.len(),
            "effort": plan.route.effort.name(),
            "elapsed_s": round(outcome.elapsed.as_secs_f64(), 1),
        },
        "candidates": called.iter().map(|record| candidate_line(record, &plan.matrix)).collect::<Vec<_>>(),
        "records": plan.folder.display().to_string(),
    });
    if let Some(outer) = &outcome.converge {
        body["search"]["starts_taken"] = json!(outer.starts.len());
        body["search"]["starts_stop"] = json!(outer.starts_stop);
        body["search"]["effort_level_reached"] = json!(outer.cost_by_level.len().saturating_sub(1));
        body["search"]["effort_stop"] = json!(outer.effort_stop);
        body["search"]["cost_by_level"] = json!(outer
            .cost_by_level
            .iter()
            .map(|cost| round(*cost, 1))
            .collect::<Vec<_>>());
        body["search"]["converge_s"] = json!(round(outer.seconds, 1));
    }
    if let Some(stopped) = &outcome.stopped_routing {
        body["search"]["routing_stopped"] = json!(stopped);
    }
    // What the winner changes, when it changes little: a note and the first step.
    let mut small_change: Option<String> = None;
    if let Some((id, placements)) = &verdict.winner {
        let record = converged
            .iter()
            .find(|record| record.id == *id)
            .expect("the winner is a record of family C");
        let mut winner = candidate_line(record, &plan.matrix);
        winner["moved_parts"] = json!(placements.len());
        let moves = moves_of(model, placements);
        let change = change_of(&moves);
        winner["moves"] = json!(moves
            .iter()
            .take(MAX_LINES)
            .map(PartMove::json)
            .collect::<Vec<_>>());
        winner["moves_total"] = json!(moves.len());
        winner["largest_move_mm"] = json!(moves.first().map(|m| round(m.distance_mm, 3)));
        winner["change"] = json!(change.name());
        small_change = change.line(&moves);
        winner["gate_failures"] = json!(verdict
            .gate_failures
            .iter()
            .take(MAX_LINES)
            .collect::<Vec<_>>());
        winner["gate_failures_total"] = json!(verdict.gate_failures.len());
        if let Some(screen) = &record.screen {
            winner["screen"] = json!({
                "score": screen.score,
                "verdict": screen.verdict,
                "airwire_mm": screen.wirelength_mm.map(|mm| round(mm, 1)),
                "crossings": screen.crossings,
                "decoupling_violations": screen.decoupling_violations,
                "connector_violations": screen.connector_violations,
            });
        }
        if let Some(trace) = &record.converge {
            winner["cost"] = json!({
                "start": round(trace.start.cost, 1),
                "end": round(trace.end.cost, 1),
                "airwire_mm": round(trace.end.airwire_mm, 1),
                "crossings": trace.end.crossings,
                "pair_violations": trace.end.pair_violations,
                "connector_violations": trace.end.connector_violations,
                "neighbour_mm": trace.end.neighbour_mm.map(|mm| round(mm, 1)),
                "label_conflicts": trace.end.label_conflicts,
                "converged": trace.converged(),
            });
        }
        body["winner"] = winner;
        body["apply_with"] = json!({
            "tool": "set_component_placements",
            "arguments": {
                "board": board.display().to_string(),
                "placements": placements
                    .iter()
                    .map(|p| json!({
                        "reference": p.reference,
                        "x": p.x,
                        "y": p.y,
                        "rotation": p.rotation,
                    }))
                    .collect::<Vec<_>>(),
            },
        });
    } else {
        body["reasons"] = json!(verdict.reasons);
    }
    let mut notes = notes(model, outcome.schematic_reason.as_deref(), plan, planned);
    let mut steps = next_steps(verdict, plan, planned);
    add_change_note(&mut notes, &mut steps, small_change);
    body["notes"] = json!(notes);
    body["next_steps"] = json!(steps);
    body
}

/// A small or empty change's line, first among the notes and the next steps.
pub(crate) fn add_change_note(
    notes: &mut Vec<String>,
    steps: &mut Vec<String>,
    line: Option<String>,
) {
    if let Some(line) = line {
        steps.insert(
            0,
            format!("{line} Say so when you show the plan, and let the user decide whether to apply it."),
        );
        notes.insert(0, line);
    }
}

/// The most a part may move, mm, with no part turned, for a plan to count as a
/// small change: two steps of a 0.25 mm placement grid. A placement that changes the
/// routing moves parts by a pad pitch or more; a plan below this is the board as it
/// is, nudged.
pub(crate) const SMALL_MOVE_MM: f64 = 0.5;

/// One part the winner moves or turns, against the board as it was read.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PartMove {
    pub reference: String,
    pub dx_mm: f64,
    pub dy_mm: f64,
    pub distance_mm: f64,
    /// The turn in degrees, in (-180, 180]; 0 when the part is not turned.
    pub rotation_deg: f64,
}

impl PartMove {
    fn json(&self) -> Value {
        json!({
            "reference": self.reference,
            "dx_mm": round(self.dx_mm, 3),
            "dy_mm": round(self.dy_mm, 3),
            "distance_mm": round(self.distance_mm, 3),
            "rotation_deg": round(self.rotation_deg, 1),
        })
    }
}

/// How much a plan changes the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
    /// No part moves or turns.
    None,
    /// Every move is at most [`SMALL_MOVE_MM`] and no part turns.
    Small,
    /// Anything more.
    Placement,
}

impl Change {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Change::None => "none",
            Change::Small => "small",
            Change::Placement => "placement",
        }
    }

    /// The note for a plan that changes nothing or little; `None` for a placement.
    pub(crate) fn line(self, moves: &[PartMove]) -> Option<String> {
        match self {
            Change::None => Some(
                "The winner is the board as it is: no part moves or turns, so there is nothing to apply."
                    .to_string(),
            ),
            Change::Small => Some(format!(
                "The winner barely changes the board: {} part(s) move, the largest by {} mm ({}), and none turns. No meaningful placement change was found; applying it is optional.",
                moves.len(),
                round(moves[0].distance_mm, 3),
                moves[0].reference
            )),
            Change::Placement => None,
        }
    }
}

/// The parts `placements` moves or turns against `model`'s parts, the largest move
/// first (then by reference). A placement equal to the part's own position (within
/// 1e-6 mm and 1e-6 degrees) is no move. A reference the model lacks is left out:
/// the winner's placements are rebuilt from the same model, so there is none.
pub(crate) fn moves_of(model: &BoardModel, placements: &[IpcFootprintPlacement]) -> Vec<PartMove> {
    let mut moves: Vec<PartMove> = placements
        .iter()
        .filter_map(|placement| {
            let part = model
                .parts
                .iter()
                .find(|part| part.reference.as_deref() == Some(placement.reference.as_str()))?;
            let dx = placement.x - part.at.0;
            let dy = placement.y - part.at.1;
            let mut turn = (placement.rotation - part.rotation_deg).rem_euclid(360.0);
            if turn > 180.0 {
                turn -= 360.0;
            }
            if turn.abs() < 1e-6 {
                turn = 0.0;
            }
            let distance = dx.hypot(dy);
            (distance > 1e-6 || turn != 0.0).then(|| PartMove {
                reference: placement.reference.clone(),
                dx_mm: dx,
                dy_mm: dy,
                distance_mm: distance,
                rotation_deg: turn,
            })
        })
        .collect();
    moves.sort_by(|a, b| {
        b.distance_mm
            .total_cmp(&a.distance_mm)
            .then_with(|| a.reference.cmp(&b.reference))
    });
    moves
}

/// [`Change`] of a plan's moves.
pub(crate) fn change_of(moves: &[PartMove]) -> Change {
    if moves.is_empty() {
        Change::None
    } else if moves
        .iter()
        .all(|m| m.distance_mm <= SMALL_MOVE_MM && m.rotation_deg == 0.0)
    {
        Change::Small
    } else {
        Change::Placement
    }
}

/// One routed candidate: its id, label, rank, outcome, cost and route numbers.
fn candidate_line(record: &Record, matrix: &Matrix) -> Value {
    let mut line = json!({
        "candidate": format!("c{:04}", record.id),
        "label": record.label(matrix),
        "rank": record.rank,
        "outcome": record.outcome,
        "cost": record.converge.as_ref().map(|trace| round(trace.end.cost, 1)),
    });
    match record.judged.as_ref().map(|judged| &judged.route) {
        Some(RouteResult::Routed {
            gates,
            measures,
            status,
            ..
        }) => {
            let placement_pass = record
                .placement_gates
                .as_ref()
                .is_some_and(|gates| gates.pass());
            line["route"] = json!({
                "status": status,
                "unconnected": measures.unconnected,
                "signal_mm": round(measures.signal_mm, 1),
                "power_mm": round(measures.power_mm, 1),
                "vias_off_pour": measures.vias_off_pour,
                "copper_quality": measures.copper_quality.map(|q| round(q, 1)),
                "stitching_vias": measures.stitching_vias,
            });
            line["silk"] = json!(record.judged.as_ref().and_then(|judged| judged.silk));
            line["gates_pass"] = json!(placement_pass && gates.all_pass());
        }
        _ => {
            line["reason"] = json!(record.reason);
        }
    }
    line
}

/// What the response must say about the plan's known limits.
fn notes(
    model: &BoardModel,
    schematic_reason: Option<&str>,
    plan: &Plan,
    planned: &Planned,
) -> Vec<String> {
    let mut notes = Vec::new();
    if model.intent.pairs.is_empty() {
        notes.push(
            "No decoupling pair was found on this board (score_placement found none), so the search does not keep capacitors by their ICs: check each IC's capacitors in the plan before it is applied."
                .to_string(),
        );
    }
    if let Some(reason) = schematic_reason {
        notes.push(format!(
            "The schematic could not be read ({reason}), so the search does not see which parts the schematic draws together."
        ));
    }
    if planned.routing_items > 0 {
        notes.push(format!(
            "The board holds {} tracks, arcs, vias and teardrops. The plan was made without them, and applying it moves no copper: remove the old routing before the placements are applied, or it is left off the moved pads.",
            planned.routing_items
        ));
    }
    notes.push(
        "The plan is for the board as it was read (`planned_from.sha256`): if a part was moved, added or locked since, call again before applying it."
            .to_string(),
    );
    notes.push(format!(
        "The routing here only ranks the placements (effort {}); route the board again after the plan is applied.",
        plan.route.effort.name()
    ));
    notes
}

/// The calls that follow, by the verdict.
fn next_steps(verdict: &Verdict, plan: &Plan, planned: &Planned) -> Vec<String> {
    if verdict.winner.is_none() {
        return vec![
            "Nothing to apply: read `reasons`, and the records in the `records` folder (search.json, table.md).".to_string(),
            "Lock in KiCad the parts that must not move, free the ones that may, and call again.".to_string(),
        ];
    }
    let mut steps = vec![
        "Show the user `winner`, `candidates` and `notes`, and ask before changing the board."
            .to_string(),
    ];
    if !verdict.gate_failures.is_empty() {
        steps.push(
            "The winner fails a gate (`winner.gate_failures`): say so when you show the plan."
                .to_string(),
        );
    }
    let mut apply =
        "On the user's OK, with the board unchanged since the plan: create_checkpoint".to_string();
    if planned.routing_items > 0 {
        apply.push_str(", remove the old tracks, arcs, vias and teardrops");
    }
    apply.push_str(", then set_component_placements with `apply_with.arguments` exactly (one KiCad undo step).");
    steps.push(apply);
    steps.push(
        "Then score_placement on the board, and compare it with `winner.screen`.".to_string(),
    );
    steps.push(format!(
        "Then route the board with export_specctra_dsn and route_specctra_dsn at the effort the user picks; the plan's own routing (effort {}) only ranked the candidates.",
        plan.route.effort.name()
    ));
    steps
}

fn round(value: f64, digits: i32) -> f64 {
    let scale = 10f64.powi(digits);
    (value * scale).round() / scale
}

#[cfg(test)]
#[path = "placement_converge_tests.rs"]
mod tests;
