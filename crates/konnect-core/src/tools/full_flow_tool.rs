//! `run_full_flow` (Row FF, `docs/FULL_FLOW_DESIGN.md`): one tracked job that takes a
//! board from placement through pours to routing.
//!
//! The flow plans everything on scratch copies first (the pours on the current
//! placement, Row CT's placement search routed with those pours, the pours again on the
//! winner, and an export check of the board it would build), shows one plan, and on one
//! answer changes the live board one step at a time. Every write is a public tool's own
//! handler from the registry, called as the MCP dispatcher calls it (`LiveIo`), so B's
//! checkpoint guard, C's revision check and E's live rule writes apply exactly as they do
//! by hand. Before each writing step the flow checks that the saved board is where the
//! step before left it, takes a checkpoint, and after it saves and verifies the save.
//!
//! The work runs as legs on `crate::jobs` (D): a leg runs the steps from where the flow
//! stands to the next gate, stop or end, and its record is `flow.json` in the flow's own
//! folder. The model (what comes next, the gates, the refusals, the summary) is
//! `crate::full_flow`'s; this module does the input and output.

use crate::full_flow::*;
use crate::jobs::{
    board_busy, claim_board, run_folder_of, write_json, JobKind, Jobs, Progress, Report,
};
use crate::mcp::protocol::{CallToolResult, ToolContent};
use crate::place_search::{RouteCall, SearchProgress};
use crate::pour_plan::ProposedZone;
use crate::route_search::BoxFuture;
use crate::tools::placement_converge::{
    live_input, locate_fastroute, plan_with_outcome, prepare_search, Planned, PLACEMENT_RUNS,
};
use crate::tools::{invalid_arg, ToolContext, ToolDef};
use anyhow::Context;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The flows' own folder below the state folder. `<state>/full-flow/<flow_id>/` holds
/// `flow.json`, `steps/` and `attempts/<n>/` (the plan's) and `routes/<n>/` (the
/// route's). `<state>/full-flow/.boards/<16 hex>.json` names a board's open flow.
const FLOWS_FOLDER: &str = "full-flow";

/// The flow's record, in its folder.
const FLOW_FILE: &str = "flow.json";

/// The legs, as `crate::jobs` keeps them: one run per leg.
pub(crate) const FLOW_LEGS: JobKind = JobKind {
    folder: "full-flow-legs",
    tool: "run_full_flow",
};

/// The `wait_s` a report suggests (D's value, for the same reason).
const SUGGESTED_WAIT_S: u64 = 300;

/// Reads of the live board: tries in all, and the pause between them (FF7).
const READ_TRIES: u32 = 3;
const READ_PAUSE: Duration = Duration::from_secs(5);

/// How long an answer waits for the leg before it to end (the leg handoff).
const HANDOFF_WAIT: Duration = Duration::from_secs(10);

/// The most plane-stitching rounds (the skill's rule), and `route_batch`'s cap.
const STITCH_ROUNDS: usize = 4;
const BATCH_CAP: usize = 200;

/// The plan's files in an attempt folder.
const WINNER_FILE: &str = "winner.kicad_pcb";
const P1_FILE: &str = "p1-routing.json";
const P2_FILE: &str = "p2-pours.json";
const P3_FILE: &str = "p3-placement.json";
const P4_FILE: &str = "p4-pours.json";
const EXPORT_FILE: &str = "p4-export-check.json";
/// The signature of the pours gate A showed (`pours_signature`).
const P4_SIGNATURE_FILE: &str = "p4-signature.json";
/// The route's result, in a route folder.
const ROUTE_FILE: &str = "route.json";

// ─── Tool definition ─────────────────────────────────────────────────────────

const DESCRIPTION: &str = "Take a board from placement through pours to routing in one tracked job (Row FF). \
The board must be open in KiCad's PCB editor, alone, with no unsaved changes. A start call (board, and optionally effort, \
placement_effort, fastroute_path, steps, approve) returns at once with a run_id; a report call (run_id, wait_s up to 600) \
gives the progress and, at the end, the whole result. The flow first plans on scratch copies: the pours on the current \
placement (plan_pours), Row CT's placement search with each candidate routed with those pours \
(plan_convergent_placement), the pours again on the winner, and an export check of the board it would build. Nothing on \
the board changes before the plan is shown. It stops at a gate only where approve does not answer in advance: gate R asks \
for pour_roles within seconds when a supply role comes only from a net's name; gate A shows the plan (parts that move, \
the old routing that goes, the zones, the roles, the export check) and asks placement, delete_routing, pours and routing; \
gate B asks again only when a judge gate of the route failed (routing apply_if_complete) or routing is ask. Answer a gate \
with run_id, answer (the keys it needs; a plain yes is its answer_with, unchanged) and expected_plan_revision. Then the job \
changes the live board one step at a time through the public tools (delete_all_routing, set_component_placements, \
add_zone and refill_zones, export_specctra_dsn, route_specctra_dsn with fastroute at effort, plan_specctra_ses_import, \
apply_specctra_ses, the route's plane stitching and reference vias, its length rules with create_custom_drc_rule, then \
run_drc with refill): before each writing step it checks that the saved board is where the step before left it and takes \
a checkpoint, and after it saves and checks the save. It never retries a write and never restores a checkpoint: a stopped \
flow says why, what is done, and the restore_checkpoint arguments; resume: true continues once the board is where the \
step left it, and abandon: true ends it, undoing nothing. The final result lists every step as done, skipped (with why) or \
not checked, the placement's moves, the route's status, summary and gates, the DRC counts and the way back.";

pub(crate) fn tools() -> Vec<ToolDef> {
    vec![arc_tool!(
        "run_full_flow",
        DESCRIPTION,
        schema(),
        |args, ctx| async move { handle_run_full_flow(args, ctx).await }
    )
    .with_board_access(crate::tools::BoardAccess::LiveOnly)]
}

fn schema() -> Value {
    let effort = json!({ "type": "string", "enum": ["fast", "m", "h", "xh"] });
    json!({
        "type": "object",
        "properties": {
            "board": { "type": "string", "description": "Start: the .kicad_pcb open in KiCad's PCB editor" },
            "effort": { "description": "Start: the final routing's effort. Default: the user config routing.effort, else m", "type": "string", "enum": ["fast", "m", "h", "xh"] },
            "placement_effort": { "description": "Start: the effort at which the placement candidates are routed. Default m", "type": effort["type"], "enum": effort["enum"] },
            "fastroute_path": { "type": "string", "minLength": 1, "description": "Start: the fastroute binary, as route_specctra_dsn reads it" },
            "steps": {
                "type": "object",
                "description": "Start: which steps run. Default every one",
                "properties": {
                    "placement": { "type": "string", "enum": ["run", "skip"] },
                    "pours": { "type": "string", "enum": ["plan", "skip"] },
                    "routing": { "type": "string", "enum": ["run", "skip"] }
                },
                "additionalProperties": false
            },
            "approve": approve_schema("Start: the gates' keys answered in advance. Leave a key out to be asked."),
            "run_id": { "type": "string", "description": "Report, answer, resume or abandon: the run_id the start returned" },
            "wait_s": { "type": "integer", "minimum": 0, "maximum": 600, "default": 0, "description": "Report: seconds to wait for the flow to reach a gate, a stop or its end. 300 suits most clients" },
            "answer": approve_schema("Answer: the keys the gate needs (its answer_with)."),
            "expected_plan_revision": { "type": "string", "description": "Answer: the plan_revision of the gate being answered" },
            "resume": { "type": "boolean", "const": true, "description": "Resume a stopped flow" },
            "abandon": { "type": "boolean", "const": true, "description": "End the flow; nothing on the board is undone" }
        },
        "additionalProperties": false
    })
}

fn approve_schema(description: &str) -> Value {
    json!({
        "type": "object",
        "description": description,
        "properties": {
            "placement": { "type": "string", "enum": ["apply", "keep_current"] },
            "delete_routing": { "type": "string", "enum": ["delete", "delete_including_locked", "keep"] },
            "pours": { "type": "string", "enum": ["add", "skip"] },
            "pour_roles": crate::tools::pour_plan_tool::pour_roles_schema(),
            "routing": { "type": "string", "enum": ["apply_if_complete", "apply_best", "ask", "skip"] }
        },
        "additionalProperties": false
    })
}

// ─── The call ────────────────────────────────────────────────────────────────

/// The call's form; any mix is refused, naming the argument of the other form.
#[derive(Debug, Clone, PartialEq)]
enum Request {
    Start(StartCall),
    Report {
        run_id: String,
        wait: Duration,
    },
    Answer {
        run_id: String,
        answer: Approvals,
        expected_plan_revision: String,
    },
    Resume {
        run_id: String,
    },
    Abandon {
        run_id: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct StartCall {
    board: PathBuf,
    effort: Option<String>,
    placement_effort: Option<String>,
    fastroute_path: Option<PathBuf>,
    steps: Steps,
    approve: Approvals,
}

/// The arguments of each form, in the order the refusal names them.
const START_ONLY: [&str; 6] = [
    "board",
    "effort",
    "placement_effort",
    "fastroute_path",
    "steps",
    "approve",
];

fn read_request(args: &Value) -> Result<Request, CallToolResult> {
    let has = |key: &str| args.get(key).is_some_and(|value| !value.is_null());
    let Some(run_id) = args.get("run_id").filter(|v| !v.is_null()) else {
        for key in [
            "wait_s",
            "answer",
            "expected_plan_revision",
            "resume",
            "abandon",
        ] {
            if has(key) {
                return Err(invalid_arg(
                    key,
                    "goes with run_id: a start call gives board, a later call gives run_id",
                ));
            }
        }
        return read_start(args).map(Request::Start);
    };
    let run_id = run_id
        .as_str()
        .filter(|id| crate::jobs::valid_run_id(id))
        .ok_or_else(|| {
            invalid_arg(
                "run_id",
                "is not a run id: it reads like 1700000000000-0123456789ab",
            )
        })?
        .to_string();
    if let Some(key) = START_ONLY.iter().find(|key| has(key)) {
        return Err(invalid_arg(
            key,
            "is a start argument; a call with run_id reports, answers, resumes or abandons",
        ));
    }
    let forms: Vec<&str> = ["answer", "resume", "abandon"]
        .into_iter()
        .filter(|key| has(key))
        .collect();
    if forms.len() > 1 {
        return Err(invalid_arg(
            forms[1],
            &format!("cannot go with {} in one call", forms[0]),
        ));
    }
    let wait = args.get("wait_s").and_then(Value::as_u64);
    if has("wait_s") && forms.first().is_some() {
        return Err(invalid_arg("wait_s", "goes with a report only"));
    }
    if has("expected_plan_revision") && forms.first() != Some(&"answer") {
        return Err(invalid_arg("expected_plan_revision", "goes with answer"));
    }
    match forms.first() {
        None => {
            let seconds = wait.unwrap_or(0);
            if seconds > crate::jobs::MAX_WAIT.as_secs() {
                return Err(invalid_arg("wait_s", "must be at most 600"));
            }
            Ok(Request::Report {
                run_id,
                wait: Duration::from_secs(seconds),
            })
        }
        Some(&"answer") => {
            let answer: Approvals = serde_json::from_value(args["answer"].clone())
                .map_err(|error| invalid_arg("answer", &format!("{error}")))?;
            let expected_plan_revision = args["expected_plan_revision"]
                .as_str()
                .filter(|text| !text.is_empty())
                .ok_or_else(|| {
                    invalid_arg(
                        "expected_plan_revision",
                        "is needed with answer: the plan_revision of the gate being answered",
                    )
                })?
                .to_string();
            Ok(Request::Answer {
                run_id,
                answer,
                expected_plan_revision,
            })
        }
        Some(&"resume") => Ok(Request::Resume { run_id }),
        _ => Ok(Request::Abandon { run_id }),
    }
}

fn read_start(args: &Value) -> Result<StartCall, CallToolResult> {
    let board = args["board"]
        .as_str()
        .filter(|board| !board.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| invalid_arg("board", "is needed: the .kicad_pcb open in KiCad"))?;
    if board.extension().and_then(|e| e.to_str()) != Some("kicad_pcb") {
        return Err(invalid_arg("board", "must be a .kicad_pcb file"));
    }
    let effort = |key: &str| -> Result<Option<String>, CallToolResult> {
        match args.get(key).filter(|v| !v.is_null()) {
            None => Ok(None),
            Some(value) => value
                .as_str()
                .filter(|level| crate::route_search::Effort::parse(level).is_some())
                .map(|level| Some(level.to_string()))
                .ok_or_else(|| invalid_arg(key, "must be fast, m, h or xh")),
        }
    };
    let steps: Steps = match args.get("steps").filter(|v| !v.is_null()) {
        None => Steps::default(),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| invalid_arg("steps", &format!("{error}")))?,
    };
    let approve: Approvals = match args.get("approve").filter(|v| !v.is_null()) {
        None => Approvals::default(),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| invalid_arg("approve", &format!("{error}")))?,
    };
    Ok(StartCall {
        board,
        effort: effort("effort")?,
        placement_effort: effort("placement_effort")?,
        fastroute_path: args["fastroute_path"].as_str().map(PathBuf::from),
        steps,
        approve,
    })
}

async fn handle_run_full_flow(
    args: &Value,
    ctx: Arc<ToolContext>,
) -> anyhow::Result<CallToolResult> {
    let request = match read_request(args) {
        Ok(request) => request,
        Err(refusal) => return Ok(refusal),
    };
    let state = konnect_sexp::writer::state_root()
        .map_err(|error| anyhow::anyhow!("Konnect's state folder: {error}"))?;
    let jobs = Jobs::global();
    match request {
        Request::Start(call) => start(jobs, &state, call, ctx).await,
        Request::Report { run_id, wait } => Ok(report(jobs, &state, &run_id, wait).await),
        Request::Answer {
            run_id,
            answer: given,
            expected_plan_revision,
        } => answer(jobs, &state, &run_id, given, &expected_plan_revision, ctx).await,
        Request::Resume { run_id } => resume(jobs, &state, &run_id, ctx).await,
        Request::Abandon { run_id } => abandon(jobs, &state, &run_id).await,
    }
}

// ─── The input and output of the live steps ──────────────────────────────────

/// How a call through the registry ended.
#[derive(Debug, Clone)]
pub(crate) enum Called {
    /// Refused before the handler ran (a missing argument, the input validator, a tool
    /// that is not in the registry): nothing was sent.
    Before(CallToolResult),
    /// The handler's result.
    Answered(CallToolResult),
    /// The handler returned an error: not evidence that nothing changed.
    Erred(String),
}

/// The live board and the registry, as the flow uses them. Production is [`LiveIo`]; the
/// tests give a fake that applies the writes to a text.
pub(crate) trait FlowIo: Send + Sync + 'static {
    /// A public tool's handler, called as the MCP dispatcher calls it.
    fn call(&self, tool: &'static str, args: Value) -> BoxFuture<Called>;
    /// KiCad's own text of the board, or why it cannot be read.
    fn live_text(&self) -> BoxFuture<Result<String, String>>;
    /// The saved file, or why it cannot be read.
    fn saved_text(&self) -> BoxFuture<Result<String, String>>;
    /// A checkpoint of the project's saved files (made, or one of exactly these files
    /// reused): its id, or why there is none.
    fn checkpoint(&self, label: String) -> BoxFuture<Result<String, String>>;
    /// The pause between two tries of a read.
    fn pause(&self, pause: Duration) -> BoxFuture<()>;
}

/// The production [`FlowIo`] (FF2): what `McpHandler::dispatch_tool` does, minus the
/// transport. In order: the `ToolDef` from the catalogue (which applies B's `guard`);
/// `first_missing_required`, then the definition's input validator (a refusal is
/// [`Called::Before`], nothing sent); the handler with the server's `Arc<ToolContext>`;
/// the call recorded in the observer.
pub(crate) struct LiveIo {
    pub ctx: Arc<ToolContext>,
    pub board: PathBuf,
    pub project: PathBuf,
}

/// The registry's definition of `name`, as the catalogue builds it.
fn find_tool(name: &str) -> Option<ToolDef> {
    crate::router::registry::ALL_TOOLSETS
        .iter()
        .find_map(|toolset| {
            crate::router::registry::tools_for(toolset.name)?
                .into_iter()
                .find(|tool| tool.name == name)
        })
}

/// One registry call, as the dispatcher makes it, recorded in `ctx`'s observer.
pub(crate) async fn registry_call(ctx: &Arc<ToolContext>, tool: &str, args: &Value) -> Called {
    let Some(def) = find_tool(tool) else {
        return Called::Before(CallToolResult::error(format!(
            "{tool} is not in the registry"
        )));
    };
    if let Some(missing) = crate::mcp::handler::first_missing_required(&def.input_schema, args) {
        return Called::Before(invalid_arg(&missing, "missing"));
    }
    if let Err(error) = def.input_validator.validate(args) {
        return Called::Before(crate::mcp::handler::schema_argument_error(&error).0);
    }
    let started = std::time::Instant::now();
    let ts = crate::observability::unix_ms();
    let outcome = (def.handler)(args, Arc::clone(ctx)).await;
    let (status, error_kind, result_bytes) = match &outcome {
        Ok(result) => (
            crate::mcp::handler::call_status(result),
            crate::mcp::error::extract_error_kind(result),
            result
                .content
                .iter()
                .map(|content| match content {
                    ToolContent::Text { text } => text.len(),
                    ToolContent::Image { data, .. } => data.len(),
                })
                .sum(),
        ),
        Err(_) => (
            crate::observability::CallStatus::Error,
            Some("handler_error".to_string()),
            0,
        ),
    };
    ctx.observer
        .record(crate::observability::CallRecord {
            call_id: format!("run_full_flow:{}", crate::observability::new_call_id()),
            ts,
            tool: tool.to_string(),
            toolset: ctx.router.find_toolset_for_tool(tool).map(str::to_string),
            dur_ms: started.elapsed().as_millis() as u64,
            status,
            error_kind,
            args_bytes: args.to_string().len(),
            result_bytes,
        })
        .await;
    match outcome {
        Ok(result) => Called::Answered(result),
        Err(error) => Called::Erred(format!("{error:#}")),
    }
}

impl FlowIo for LiveIo {
    fn call(&self, tool: &'static str, args: Value) -> BoxFuture<Called> {
        let ctx = Arc::clone(&self.ctx);
        Box::pin(async move { registry_call(&ctx, tool, &args).await })
    }

    fn live_text(&self) -> BoxFuture<Result<String, String>> {
        let (ctx, board) = (Arc::clone(&self.ctx), self.board.clone());
        Box::pin(async move { read_live_text(&ctx, &board).await })
    }

    fn saved_text(&self) -> BoxFuture<Result<String, String>> {
        let board = self.board.clone();
        Box::pin(async move {
            tokio::fs::read_to_string(&board).await.map_err(|error| {
                format!(
                    "the saved board '{}' cannot be read: {error}",
                    board.display()
                )
            })
        })
    }

    fn checkpoint(&self, label: String) -> BoxFuture<Result<String, String>> {
        let project = self.project.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                crate::tools::checkpoints::ensure_checkpoint(&project, &label)
            })
            .await
            .map_err(|error| format!("the checkpoint task did not finish: {error}"))?
            .map(|checkpoint| checkpoint.id)
        })
    }

    fn pause(&self, pause: Duration) -> BoxFuture<()> {
        Box::pin(tokio::time::sleep(pause))
    }
}

/// KiCad's own text of `board`, through B's observation of the live board.
async fn read_live_text(ctx: &ToolContext, board: &Path) -> Result<String, String> {
    use crate::tools::live_board::{observe, LiveBoard};
    let read = observe(ctx, board, |client, document| {
        let open = client.get_open_board_paths()?.len();
        Ok((open, client.save_document_to_string_in(document)?))
    })
    .await;
    match read {
        Ok(LiveBoard::Answered((1, text))) => Ok(text),
        Ok(LiveBoard::Answered((open, _))) => Err(format!(
            "KiCad has {open} boards open; the flow saves KiCad's current board, so it works with one board open only: close the others"
        )),
        Ok(other) => Err(format!(
            "{} ({})",
            crate::tools::board_safety::not_open_sentence(board),
            live_board_word(&other)
        )),
        Err(error) => Err(format!("KiCad could not be asked for the board: {error:#}")),
    }
}

fn live_board_word<T>(observed: &crate::tools::live_board::LiveBoard<T>) -> &'static str {
    use crate::tools::live_board::LiveBoard;
    match observed {
        LiveBoard::Answered(_) => "answered",
        LiveBoard::Rejected(_) => "KiCad refused the read",
        LiveBoard::Uncertain(_) => "KiCad did not finish the read",
        LiveBoard::Recovered(_) => "the link to KiCad was recovered during the read",
        LiveBoard::Unserved { .. } => "KiCad answered without naming a board",
        LiveBoard::NotOpen { .. } => "KiCad is running and the board is not open",
        LiveBoard::NeverReached(_) => "KiCad cannot be reached",
        LiveBoard::LostAfterObservation { .. } => "KiCad held the board and no longer answers",
        LiveBoard::Unresolved(_) => "the board's path cannot be resolved",
    }
}

/// `read`, tried [`READ_TRIES`] times in all, [`READ_PAUSE`] apart (FF7). Writes are
/// never retried.
async fn with_tries(
    io: &dyn FlowIo,
    read: impl Fn() -> BoxFuture<Result<String, String>>,
) -> Result<String, String> {
    let mut last = String::new();
    for attempt in 0..READ_TRIES {
        if attempt > 0 {
            io.pause(READ_PAUSE).await;
        }
        match read().await {
            Ok(text) => return Ok(text),
            Err(why) => last = why,
        }
    }
    Err(last)
}

/// The canonical SHA-256 of a board text, off the async threads.
async fn canonical(text: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || crate::board_identity::canonical_sha256(&text))
        .await
        .map_err(|error| format!("the hash task did not finish: {error}"))?
        .map_err(|error| format!("the board text does not parse: {error:#}"))
}

/// The live text and the saved file, each read with tries, and their canonical hashes:
/// `Ok((live, saved_hash))` when they are equal, else why not.
async fn live_equals_saved(io: &dyn FlowIo) -> Result<(String, String), Stop> {
    let live = with_tries(io, || io.live_text())
        .await
        .map_err(|why| Stop::new(StopKind::KicadUnreachable, why))?;
    let saved = with_tries(io, || io.saved_text())
        .await
        .map_err(|why| Stop::new(StopKind::KicadUnreachable, why))?;
    let (live_hash, saved_hash) = (
        canonical(live.clone())
            .await
            .map_err(|why| Stop::new(StopKind::Failed, why))?,
        canonical(saved)
            .await
            .map_err(|why| Stop::new(StopKind::Failed, why))?,
    );
    if live_hash != saved_hash {
        return Err(Stop::new(
            StopKind::BoardChanged,
            "the live board differs from its saved file: something changed the board in KiCad and it was not saved",
        ));
    }
    Ok((live, saved_hash))
}

// ─── Stops ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopKind {
    /// Refused, nothing changed.
    Failed,
    /// KiCad could not be read after the tries: resumable.
    KicadUnreachable,
    /// The board is not where the flow left it: resuming is refused.
    BoardChanged,
    /// A write may have landed.
    Uncertain,
    /// A write landed and its save did not verify.
    Unsaved,
}

#[derive(Debug, Clone)]
struct Stop {
    kind: StopKind,
    why: String,
}

impl Stop {
    fn new(kind: StopKind, why: impl Into<String>) -> Self {
        Stop {
            kind,
            why: why.into(),
        }
    }

    fn word(&self) -> &'static str {
        match self.kind {
            StopKind::Failed => "failed",
            StopKind::KicadUnreachable => "kicad_unreachable",
            StopKind::BoardChanged => "board_changed",
            StopKind::Uncertain => "uncertain",
            StopKind::Unsaved => "unsaved",
        }
    }
}

// ─── The flow's files ────────────────────────────────────────────────────────

fn flows_root(state: &Path) -> PathBuf {
    state.join(FLOWS_FOLDER)
}

fn flow_folder(state: &Path, flow_id: &str) -> PathBuf {
    flows_root(state).join(flow_id)
}

fn attempt_folder(state: &Path, flow: &FlowRecord) -> PathBuf {
    flow_folder(state, &flow.flow_id)
        .join("attempts")
        .join(flow.attempt.to_string())
}

fn route_folder(state: &Path, flow: &FlowRecord) -> PathBuf {
    flow_folder(state, &flow.flow_id)
        .join("routes")
        .join(flow.route_attempt.to_string())
}

fn read_flow(state: &Path, flow_id: &str) -> Result<FlowRecord, String> {
    let path = flow_folder(state, flow_id).join(FLOW_FILE);
    let text = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "there is no flow '{flow_id}' ('{}': {error})",
            path.display()
        )
    })?;
    serde_json::from_str(&text)
        .map_err(|error| format!("'{}' cannot be read: {error}", path.display()))
}

/// `flow.json`, written durably: the intent of a call is on disk before the call.
fn save_flow(state: &Path, flow: &FlowRecord) -> anyhow::Result<()> {
    write_json(
        &flow_folder(state, &flow.flow_id).join(FLOW_FILE),
        flow,
        true,
    )
}

fn read_json(path: &Path) -> anyhow::Result<Value> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("read '{}'", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parse '{}'", path.display()))
}

fn write_value(path: &Path, value: &Value) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create '{}'", parent.display()))?;
    }
    write_json(path, value, false)
}

/// The marker of `board`'s open flow.
fn marker_path(state: &Path, board: &Path) -> PathBuf {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(board.to_string_lossy().as_bytes());
    let key: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    flows_root(state)
        .join(".boards")
        .join(format!("{key}.json"))
}

/// The flow open on `board`, if one is. A marker whose flow has ended (done or
/// abandoned), or has no readable record, names none.
pub(crate) fn open_flow(state: &Path, board: &Path) -> anyhow::Result<Option<String>> {
    let path = marker_path(state, board);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(None);
    };
    let Some(id) = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|body| body["flow_id"].as_str().map(str::to_string))
        .filter(|id| crate::jobs::valid_run_id(id))
    else {
        return Ok(None);
    };
    Ok(match read_flow(state, &id) {
        Ok(flow) if !matches!(flow.state, FlowState::Done | FlowState::Abandoned) => Some(id),
        _ => None,
    })
}

/// Make `board`'s marker for `flow_id` with `create_new`: `Ok(Err(holder))` when another
/// flow holds it. A marker that names an ended flow is replaced.
fn mark_open(state: &Path, board: &Path, flow_id: &str) -> anyhow::Result<Result<(), String>> {
    use std::io::Write as _;
    let path = marker_path(state, board);
    std::fs::create_dir_all(path.parent().expect("the marker has a folder"))?;
    for _ in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(
                    json!({ "flow_id": flow_id, "board": board.display().to_string() })
                        .to_string()
                        .as_bytes(),
                )?;
                file.sync_all()?;
                return Ok(Ok(()));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if let Some(holder) = open_flow(state, board)? {
                    return Ok(Err(holder));
                }
                // An ended flow's marker: remove it and try once more.
                let _ = std::fs::remove_file(&path);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Err("another flow, still starting".to_string()))
}

/// Remove `board`'s marker when it names `flow_id`.
fn clear_open(state: &Path, board: &Path, flow_id: &str) {
    let path = marker_path(state, board);
    let names = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|body| body["flow_id"] == flow_id);
    if names {
        let _ = std::fs::remove_file(path);
    }
}

fn stamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// A new id in the job module's form: `<milliseconds>-<12 hex of seed's SHA-256>`.
fn new_id(seed: &str) -> String {
    let stamp = stamp_ms();
    let hex = crate::place_held::sha256_hex(
        format!("{seed}\0{stamp}\0{}", std::process::id()).as_bytes(),
    );
    format!("{stamp}-{}", &hex[..12])
}

// ─── The start ───────────────────────────────────────────────────────────────

/// What the pre-flight read.
struct Preflight {
    live_text: String,
    planned_from: String,
    routing_items: usize,
    nets: BTreeSet<String>,
    rails: BTreeMap<String, String>,
    named_only: Vec<String>,
}

/// P0, in the start call itself (seconds), as "P0 in full" lists it. Fastroute is
/// located later, into the flow folder, so that a refusal leaves nothing behind.
async fn preflight(
    ctx: &Arc<ToolContext>,
    state: &Path,
    board: &Path,
    call: &StartCall,
) -> anyhow::Result<Result<Preflight, CallToolResult>> {
    if let Some(flow) = open_flow(state, board)? {
        return Ok(Err(CallToolResult::error(format!(
            "A full flow is already open on this board: run {flow}. Call run_full_flow with {{\"run_id\": \"{flow}\"}} for where it stands, resume it, or abandon it. Nothing new was started."
        ))));
    }
    for (kind, what) in [
        (&PLACEMENT_RUNS, "plan_convergent_placement"),
        (&FLOW_LEGS, "run_full_flow"),
    ] {
        if let Some(busy) = board_busy(state, kind, board)? {
            return Ok(Err(CallToolResult::error(format!(
                "A {what} run is running on this board{}. One run per board at a time: nothing new was started.",
                busy.run_id.map(|id| format!(": run {id}")).unwrap_or_default()
            ))));
        }
    }
    let project = board.with_extension("kicad_pro");
    if !project.is_file() {
        return Ok(Err(CallToolResult::error(format!(
            "There is no project file '{}' beside the board, so no checkpoint can protect the flow's edits. Save the board in a KiCad project of the same name, then call again. Nothing was started.",
            project.display()
        ))));
    }
    let live = match live_input(ctx, board).await? {
        Ok(live) => live,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let saved = std::fs::read_to_string(board)
        .with_context(|| format!("read the saved board '{}'", board.display()))?;
    let (live_hash, saved_hash) = (canonical(live.text.clone()).await, canonical(saved).await);
    let planned_from = match (live_hash, saved_hash) {
        (Ok(live), Ok(saved)) if live == saved => live,
        (Ok(_), Ok(_)) => {
            return Ok(Err(CallToolResult::error(
                "The board open in KiCad has unsaved changes. Save it in KiCad (or undo them), then call again: the flow's first checkpoint must hold the board you are looking at. Nothing was started.",
            )))
        }
        (Err(why), _) | (_, Err(why)) => {
            return Ok(Err(CallToolResult::error(format!(
                "The board cannot be read: {why}. Nothing was started."
            ))))
        }
    };
    let (nets, roles) = match crate::tools::pour_plan_tool::board_roles(board, &live.text) {
        Ok(read) => read,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let rails = crate::pour_plan::rails_of(&roles);
    let roles = match &call.approve.pour_roles {
        Some(given) => match crate::pour_plan::with_roles(&roles, given, &rails, &nets) {
            Ok(roles) => roles,
            Err(reason) => return Ok(Err(invalid_arg("approve.pour_roles", &reason))),
        },
        None => roles,
    };
    let routing_items = Planned::of(&live.text)?.routing_items;
    Ok(Ok(Preflight {
        named_only: crate::pour_plan::named_only(&roles),
        live_text: live.text,
        planned_from,
        routing_items,
        nets,
        rails: rails
            .into_iter()
            .map(|(net, role)| (net, role.to_string()))
            .collect(),
    }))
}

/// The start: the pre-flight, the marker, the flow folder, fastroute located into it,
/// `flow.json`; then gate R, or the first leg. A refusal leaves no flow folder, no
/// marker and no claim.
async fn start(
    jobs: &Jobs,
    state: &Path,
    call: StartCall,
    ctx: Arc<ToolContext>,
) -> anyhow::Result<CallToolResult> {
    let board = match call.board.canonicalize() {
        Ok(board) => board,
        Err(error) => {
            return Ok(invalid_arg(
                "board",
                &format!("'{}' cannot be found: {error}", call.board.display()),
            ))
        }
    };
    let pre = match preflight(&ctx, state, &board, &call).await? {
        Ok(pre) => pre,
        Err(refusal) => return Ok(refusal),
    };
    let flow_id = new_id(&board.display().to_string());
    let folder = flow_folder(state, &flow_id);
    let made = (|| -> anyhow::Result<()> {
        std::fs::create_dir_all(folder.join("steps"))?;
        Ok(())
    })();
    if let Err(error) = made {
        let _ = std::fs::remove_dir_all(&folder);
        return Err(error);
    }
    let effort = match &call.effort {
        Some(effort) => effort.clone(),
        None => crate::tools::route_fastroute::user_routing_config()
            .ok()
            .and_then(|(config, _)| config?["effort"].as_str().map(str::to_string))
            .filter(|effort| crate::route_search::Effort::parse(effort).is_some())
            .unwrap_or_else(|| "m".to_string()),
    };
    let mut flow = FlowRecord {
        flow_id: flow_id.clone(),
        board: board.display().to_string(),
        project: board.with_extension("kicad_pro").display().to_string(),
        state: FlowState::Running,
        board_state: BoardState::Unchanged,
        steps_asked: call.steps,
        approvals: call.approve.clone(),
        effort,
        placement_effort: call
            .placement_effort
            .clone()
            .unwrap_or_else(|| "m".to_string()),
        fastroute: Value::Null,
        planned_from: pre.planned_from,
        routing_items: pre.routing_items,
        nets: pre.nets,
        rails: pre.rails,
        gate: None,
        gate_body: None,
        plan_revision: None,
        facts: PlanFacts {
            named_only: pre.named_only,
            ..PlanFacts::default()
        },
        route: None,
        at: None,
        attempt: 1,
        route_attempt: 1,
        records: vec![StepRecord::new(Step::Preflight, StepState::Done)],
        legs: Vec::new(),
        start_checkpoint: None,
        stop_reason: None,
        checkpoints: Vec::new(),
    };
    // The record is on disk before the marker that names it, so a second start that
    // finds the marker reads the record and is refused (m4).
    save_flow(state, &flow)?;
    if let Err(holder) = mark_open(state, &board, &flow_id)? {
        let _ = std::fs::remove_dir_all(&folder);
        return Ok(CallToolResult::error(format!(
            "Another full flow opened this board at the same moment ({holder}). Nothing new was started."
        )));
    }
    let binary = match locate_fastroute(call.fastroute_path.as_deref(), &folder).await {
        Ok(Ok(binary)) => binary,
        Ok(Err(refusal)) => {
            clear_open(state, &board, &flow_id);
            let _ = std::fs::remove_dir_all(&folder);
            return Ok(refusal);
        }
        Err(error) => {
            clear_open(state, &board, &flow_id);
            let _ = std::fs::remove_dir_all(&folder);
            return Err(error);
        }
    };
    flow.fastroute = json!({
        "path": binary.path.display().to_string(),
        "version": binary.version,
        "source": binary.source,
    });
    drop(pre.live_text);
    if let Next::Gate(gate) = next(&flow) {
        open_gate(state, &mut flow, gate);
        save_flow(state, &flow)?;
        return Ok(CallToolResult::json(&view(state, &flow, None)));
    }
    save_flow(state, &flow)?;
    let io: Arc<dyn FlowIo> = Arc::new(LiveIo {
        ctx: Arc::clone(&ctx),
        board: board.clone(),
        project: board.with_extension("kicad_pro"),
    });
    match start_leg(jobs, state, flow.clone(), ctx, io) {
        Ok(body) => Ok(CallToolResult::json(&body)),
        Err(error) => {
            clear_open(state, &board, &flow_id);
            Err(error)
        }
    }
}

// ─── Legs ────────────────────────────────────────────────────────────────────

/// A leg's progress: the step, and the placement search's counts while it runs.
#[derive(Default)]
pub(crate) struct FlowProgress {
    step: Mutex<Option<Step>>,
    search: SearchProgress,
}

impl FlowProgress {
    fn at(&self, step: Step) {
        *self.step.lock().unwrap_or_else(PoisonError::into_inner) = Some(step);
    }
}

impl Progress for FlowProgress {
    fn snapshot(&self) -> Value {
        let step = *self.step.lock().unwrap_or_else(PoisonError::into_inner);
        let mut body = json!({
            "step": step.map(Step::id),
            "phase": step.map(Step::title),
        });
        if step == Some(Step::Placement) {
            body["placement"] =
                crate::tools::placement_converge::progress_json(&self.search.snapshot(), "m");
        }
        body
    }
}

/// Start the next leg as a job run: the claim, the leg's empty run folder, and
/// `Jobs::start` with `run_leg`. The leg's id is in `flow.json` before the start returns.
fn start_leg(
    jobs: &Jobs,
    state: &Path,
    mut flow: FlowRecord,
    ctx: Arc<ToolContext>,
    io: Arc<dyn FlowIo>,
) -> anyhow::Result<Value> {
    let board = PathBuf::from(&flow.board);
    let leg_id = new_id(&format!("{}-leg-{}", flow.flow_id, flow.legs.len()));
    let claim = match claim_board(state, &FLOW_LEGS, &board, &leg_id)? {
        Ok(claim) => claim,
        Err(busy) => anyhow::bail!(
            "the board's leg claim is held{}: wait for it to end and call again",
            busy.run_id
                .map(|id| format!(" by run {id}"))
                .unwrap_or_default()
        ),
    };
    let folder = run_folder_of(state, &FLOW_LEGS, &leg_id);
    std::fs::create_dir_all(&folder).with_context(|| format!("create '{}'", folder.display()))?;
    flow.legs.push(leg_id);
    flow.state = FlowState::Running;
    flow.gate = None;
    flow.gate_body = None;
    flow.plan_revision = None;
    flow.stop_reason = None;
    save_flow(state, &flow)?;
    let progress = Arc::new(FlowProgress::default());
    let observed = Arc::clone(&progress);
    let state_owned = state.to_path_buf();
    let started = flow.clone();
    jobs.start(&FLOW_LEGS, claim, &board, folder, progress, async move {
        let flow = run_leg(&state_owned, started, io, ctx, observed).await?;
        Ok(view(&state_owned, &flow, None))
    })?;
    Ok(view(state, &flow, None))
}

/// One leg: the steps from where the flow stands until a gate, a stop or the end, with
/// `flow.json` rewritten as the protocol says. Returns the flow at the leg's end.
async fn run_leg(
    state: &Path,
    mut flow: FlowRecord,
    io: Arc<dyn FlowIo>,
    ctx: Arc<ToolContext>,
    progress: Arc<FlowProgress>,
) -> anyhow::Result<FlowRecord> {
    let mut cache = LegCache::default();
    loop {
        match next(&flow) {
            Next::Skip(step, why) => {
                flow.put(StepRecord::skipped(step, why));
                save_flow(state, &flow)?;
            }
            Next::Gate(gate) => {
                open_gate(state, &mut flow, gate);
                save_flow(state, &flow)?;
                return Ok(flow);
            }
            Next::End => {
                flow.state = FlowState::Done;
                flow.at = None;
                save_flow(state, &flow)?;
                clear_open(state, Path::new(&flow.board), &flow.flow_id);
                return Ok(flow);
            }
            Next::Step(step) => {
                progress.at(step);
                flow.at = Some(step);
                if step == Step::Unchanged {
                    let rails = flow.rail_roles();
                    if let Some(reason) = refused_combination(
                        &flow.approvals,
                        &flow.facts,
                        &flow.steps_asked,
                        &rails,
                        &flow.nets,
                    ) {
                        // Back to gate A with the reason: the answer there may change
                        // any key (M4). Nothing has been written.
                        open_refused_gate(state, &mut flow, &reason);
                        save_flow(state, &flow)?;
                        return Ok(flow);
                    }
                }
                let ran = run_step(
                    step,
                    state,
                    &mut flow,
                    io.as_ref(),
                    &ctx,
                    &progress,
                    &mut cache,
                )
                .await;
                let outcome = match ran {
                    Ok(outcome) => outcome,
                    Err(error) => Err(Stop::new(
                        if step.writes() {
                            StopKind::Uncertain
                        } else {
                            StopKind::Failed
                        },
                        format!("{error:#}"),
                    )),
                };
                match outcome {
                    Ok(record) => {
                        if step.writes() && record.state == StepState::Done {
                            flow.board_state = BoardState::AsRecorded;
                        }
                        flow.put(record);
                        save_flow(state, &flow)?;
                    }
                    Err(stop) => {
                        stop_flow(&mut flow, step, stop);
                        save_flow(state, &flow)?;
                        return Ok(flow);
                    }
                }
            }
        }
    }
}

/// The flow stopped at `step` for `stop`: its record (unless the step left its own), the
/// state, the reason, what the board is known to hold.
fn stop_flow(flow: &mut FlowRecord, step: Step, stop: Stop) {
    flow.state = FlowState::Stopped;
    flow.at = Some(step);
    flow.stop_reason = Some(format!("{}: {}", stop.word(), stop.why));
    flow.board_state = match stop.kind {
        StopKind::Uncertain => BoardState::Uncertain,
        StopKind::Unsaved => BoardState::Unsaved,
        _ if flow.wrote() => BoardState::AsRecorded,
        _ => flow.board_state,
    };
    let state = match stop.kind {
        StopKind::Uncertain => StepState::Uncertain,
        StopKind::Unsaved => StepState::Unsaved,
        _ => StepState::Failed,
    };
    let mut record = flow
        .record(step)
        .cloned()
        .unwrap_or_else(|| StepRecord::new(step, state));
    // A writing step's record keeps its `before`, `checkpoint` and calls; its state
    // becomes the stop's, unless it already says more (an unsaved step stays unsaved).
    if record.state != StepState::Unsaved || stop.kind != StopKind::Failed {
        record.state = state;
    }
    record.why = Some(format!("{}: {}", stop.word(), stop.why));
    flow.put(record);
}

/// The plan's texts a leg keeps between its steps.
#[derive(Default)]
struct LegCache {
    /// P2's zones, for P3's routing.
    before_zones: Vec<ProposedZone>,
}

/// One step. A reading or scratch step runs as its table row says; a writing step runs
/// `live_step`. Returns its record, or why the flow stops.
async fn run_step(
    step: Step,
    state: &Path,
    flow: &mut FlowRecord,
    io: &dyn FlowIo,
    ctx: &Arc<ToolContext>,
    progress: &FlowProgress,
    cache: &mut LegCache,
) -> anyhow::Result<Result<StepRecord, Stop>> {
    match step {
        Step::Preflight => Ok(Ok(StepRecord::new(step, StepState::Done))),
        Step::CountRouting => count_routing(state, flow, io).await,
        Step::PoursBefore => pours_before(state, flow, io, ctx, cache).await,
        Step::Placement => placement(state, flow, io, ctx, progress, cache).await,
        Step::PoursAfter => pours_after(state, flow, io, ctx).await,
        Step::Unchanged => unchanged(flow, io).await,
        Step::Export | Step::Route | Step::ImportCheck => route_step(step, state, flow, io).await,
        _ => live_step(step, state, flow, io).await,
    }
}

// ─── The reads and the scratch steps ─────────────────────────────────────────

/// The first text of a result, parsed as JSON when it is an object; else the text.
pub(crate) fn body_of(result: &CallToolResult) -> Value {
    let text = result
        .content
        .iter()
        .find_map(|content| match content {
            ToolContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .unwrap_or_default();
    serde_json::from_str::<Value>(text).unwrap_or_else(|_| json!({ "text": text }))
}

/// A reading call: `Ok(body)` when it succeeded, else the stop (a read never changes
/// the board, so its failure is a refusal). The whole call is written to `file`.
async fn read_call(
    io: &dyn FlowIo,
    tool: &'static str,
    args: Value,
    file: &Path,
) -> anyhow::Result<Result<Value, Stop>> {
    let called = io.call(tool, args.clone()).await;
    let (body, refused) = match &called {
        Called::Answered(result) => (body_of(result), result.is_error),
        Called::Before(result) => (body_of(result), true),
        Called::Erred(error) => (json!({ "error": error }), true),
    };
    write_value(
        file,
        &json!({ "tool": tool, "arguments": args, "result": body }),
    )?;
    Ok(if refused {
        Err(Stop::new(
            StopKind::Failed,
            format!("{tool} refused: {}", short(&body)),
        ))
    } else {
        Ok(body)
    })
}

/// A short text of a result, for a reason.
fn short(body: &Value) -> String {
    let text = body["error"]
        .as_str()
        .or_else(|| body["text"].as_str())
        .or_else(|| body["message"].as_str())
        .or_else(|| body["reason"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| body.to_string());
    text.chars().take(600).collect()
}

/// P1: `delete_all_routing`'s dry run, without `include_locked`: the counts gate A shows.
async fn count_routing(
    state: &Path,
    flow: &mut FlowRecord,
    io: &dyn FlowIo,
) -> anyhow::Result<Result<StepRecord, Stop>> {
    let file = attempt_folder(state, flow).join(P1_FILE);
    let log = flow_folder(state, &flow.flow_id)
        .join("steps")
        .join(format!("p1-{}-delete_all_routing.json", flow.attempt));
    let body = match read_call(
        io,
        "delete_all_routing",
        delete_preview_args(Path::new(&flow.board), false),
        &log,
    )
    .await?
    {
        Ok(body) => body,
        Err(stop) => return Ok(Err(stop)),
    };
    write_value(&file, &body)?;
    flow.facts.unlocked_routing = body["will_delete"]["total"].as_u64().unwrap_or(0) as usize;
    flow.facts.locked_routing = body["kept_locked"]["total"].as_u64().unwrap_or(0) as usize;
    flow.facts.locked_blocks_export = body["kept_locked"]["export_will_refuse"]
        .as_array()
        .is_some_and(|rows| !rows.is_empty());
    let mut record = StepRecord::new(Step::CountRouting, StepState::Done);
    record.counts = json!({ "will_delete": body["will_delete"]["total"], "kept_locked": body["kept_locked"]["total"] });
    record.result_file = Some(file.display().to_string());
    Ok(Ok(record))
}

/// The live text, read with tries, checked against `planned_from`: the scratch steps plan
/// the board as it was when the plan was read.
async fn planned_text(flow: &FlowRecord, io: &dyn FlowIo) -> Result<String, Stop> {
    let live = with_tries(io, || io.live_text())
        .await
        .map_err(|why| Stop::new(StopKind::KicadUnreachable, why))?;
    let hash = canonical(live.clone())
        .await
        .map_err(|why| Stop::new(StopKind::Failed, why))?;
    if hash != flow.planned_from {
        return Err(Stop::new(
            StopKind::BoardChanged,
            "the board changed in KiCad since the plan was read; abandon this flow and start again",
        ));
    }
    Ok(live)
}

/// P2 and P4: `pour_plan_tool::plan_text` on `text`, with the flow's roles, in `folder`.
/// Returns the result body and the final proposals.
async fn plan_pours_on(
    ctx: &Arc<ToolContext>,
    flow: &FlowRecord,
    text: &str,
    folder: &Path,
) -> anyhow::Result<Result<(Value, Vec<ProposedZone>), Stop>> {
    std::fs::create_dir_all(folder)?;
    let (result, plan) = crate::tools::pour_plan_tool::plan_text(
        ctx,
        Path::new(&flow.board),
        text,
        flow.approvals.pour_roles.as_ref(),
        Some(folder),
    )
    .await?;
    let body = body_of(&result);
    if result.is_error {
        return Ok(Err(Stop::new(
            StopKind::Failed,
            format!("plan_pours refused: {}", short(&body)),
        )));
    }
    Ok(Ok((
        body,
        plan.map(|plan| plan.proposals).unwrap_or_default(),
    )))
}

async fn pours_before(
    state: &Path,
    flow: &mut FlowRecord,
    io: &dyn FlowIo,
    ctx: &Arc<ToolContext>,
    cache: &mut LegCache,
) -> anyhow::Result<Result<StepRecord, Stop>> {
    let live = match planned_text(flow, io).await {
        Ok(live) => live,
        Err(stop) => return Ok(Err(stop)),
    };
    let stripped = crate::place_route::strip_routing(&live)?;
    let folder = attempt_folder(state, flow);
    let (body, zones) =
        match plan_pours_on(ctx, flow, &stripped, &folder.join("pours").join("p2")).await? {
            Ok(planned) => planned,
            Err(stop) => return Ok(Err(stop)),
        };
    write_value(&folder.join(P2_FILE), &body)?;
    cache.before_zones = zones;
    let mut record = StepRecord::new(Step::PoursBefore, StepState::Done);
    record.counts = json!({ "zones": cache.before_zones.len() });
    record.result_file = Some(folder.join(P2_FILE).display().to_string());
    Ok(Ok(record))
}

/// P3: Row CT's search through `prepare_search` and `plan_with_outcome`. When P2
/// proposed zones, each candidate is routed with them (`WithPours`, FF4). The winner's
/// board text (without P2's zones) is written beside the body.
async fn placement(
    state: &Path,
    flow: &mut FlowRecord,
    io: &dyn FlowIo,
    ctx: &Arc<ToolContext>,
    progress: &FlowProgress,
    cache: &mut LegCache,
) -> anyhow::Result<Result<StepRecord, Stop>> {
    let board = PathBuf::from(&flow.board);
    if let Err(stop) = planned_text(flow, io).await {
        return Ok(Err(stop));
    }
    let live = match live_input(ctx, &board).await? {
        Ok(live) => live,
        Err(refusal) => return Ok(Err(Stop::new(StopKind::Failed, short(&body_of(&refusal))))),
    };
    let folder = attempt_folder(state, flow);
    let search = folder.join("placement");
    std::fs::create_dir_all(&search)?;
    let effort = crate::route_search::Effort::parse(&flow.placement_effort)
        .unwrap_or(crate::route_search::Effort::M);
    let fastroute = PathBuf::from(flow.fastroute["path"].as_str().unwrap_or_default());
    let prepared = prepare_search(ctx, &search, &board, &live, effort, &fastroute)?;
    let routing: Arc<dyn RouteCall> = if cache.before_zones.is_empty() {
        Arc::clone(&prepared.routing)
    } else {
        let plain_text = std::fs::read_to_string(&prepared.inputs.board)?;
        let zoned_text = crate::pour_plan::with_zones(&plain_text, &cache.before_zones)?;
        let zoned_folder = folder.join("zoned");
        let zoned_board = crate::scratch_board::copy_project(&board, &zoned_folder)?;
        std::fs::write(&zoned_board, &zoned_text)?;
        let mut inputs = prepared.inputs.clone();
        inputs.options = crate::place_route::export_options(&zoned_board);
        inputs.board = zoned_board;
        Arc::new(WithPours {
            zoned: Arc::new(crate::place_search::PlaceRouteCall {
                inputs,
                ctx: Arc::clone(&prepared.isolated),
            }),
            zones: cache.before_zones.clone(),
            plain_text,
            zoned_text,
        })
    };
    let planned = Planned::of(&live.text)?;
    let (mut body, outcome, winner) =
        plan_with_outcome(&prepared.plan, routing, &board, &planned, &progress.search).await?;
    let mut record = StepRecord::new(Step::Placement, StepState::Done);
    if let Some(id) = winner {
        let found = outcome.records.iter().find(|record| record.id == id);
        if let Some(found) = found {
            let (_, text) = crate::place_search::rebuild(&outcome.model, found)?;
            std::fs::write(folder.join(WINNER_FILE), text)?;
        }
    }
    body["with_pours"] = json!(!cache.before_zones.is_empty());
    write_value(&folder.join(P3_FILE), &body)?;
    flow.facts.change = body["winner"]["change"].as_str().map(str::to_string);
    record.counts = json!({
        "status": body["status"],
        "moved_parts": body["winner"]["moved_parts"],
        "change": body["winner"]["change"],
    });
    if flow.facts.locked_routing > 0 {
        record.not_checked.push(
            "the search ran without the board's locked routing (it is stripped from the scratch copies)".to_string(),
        );
    }
    for note in body["notes"].as_array().into_iter().flatten() {
        if let Some(note) = note
            .as_str()
            .filter(|note| note.contains("could not") || note.contains("cannot"))
        {
            record.not_checked.push(note.to_string());
        }
    }
    record.result_file = Some(folder.join(P3_FILE).display().to_string());
    Ok(Ok(record))
}

/// P4: the pours again on the board the flow would build (the winner's text, or the
/// stripped live text), then the export check of that board with P4's zones (FF17).
async fn pours_after(
    state: &Path,
    flow: &mut FlowRecord,
    io: &dyn FlowIo,
    ctx: &Arc<ToolContext>,
) -> anyhow::Result<Result<StepRecord, Stop>> {
    let board = PathBuf::from(&flow.board);
    let live = match planned_text(flow, io).await {
        Ok(live) => live,
        Err(stop) => return Ok(Err(stop)),
    };
    let folder = attempt_folder(state, flow);
    let winner = folder.join(WINNER_FILE);
    let applies = flow
        .facts
        .change
        .as_deref()
        .is_some_and(|change| change != "none")
        && flow.approvals.placement != Some(PlacementAnswer::KeepCurrent);
    flow.facts.p4_on_winner = applies && winner.is_file();
    let base = if applies && winner.is_file() {
        std::fs::read_to_string(&winner)?
    } else {
        crate::place_route::strip_routing(&live)?
    };
    let mut record = StepRecord::new(Step::PoursAfter, StepState::Done);
    let mut zones: Vec<ProposedZone> = Vec::new();
    if flow.steps_asked.pours == PlanOrSkip::Plan {
        let p4 = folder.join("pours").join(format!("p4-{}", stamp_ms()));
        let (body, proposals) = match plan_pours_on(ctx, flow, &base, &p4).await? {
            Ok(planned) => planned,
            Err(stop) => return Ok(Err(stop)),
        };
        write_value(&folder.join(P4_FILE), &body)?;
        flow.facts.zones = proposals.len();
        flow.facts.fill_clean = body["status"] == crate::tools::pour_plan_tool::PLANNED;
        // P4 ran again (the roles or the placement answered at gate A): when the zones
        // or their fill check differ from the plan that was approved, the pours go back
        // to gate A (M3).
        let signature = pours_signature(&body);
        let signature_file = folder.join(P4_SIGNATURE_FILE);
        if let Ok(old) = read_json(&signature_file) {
            if old != signature && flow.approvals.pours.is_some() {
                flow.approvals.pours = None;
                record.not_checked.push(
                    "the zones changed after the plan was approved, so gate A asks about them again"
                        .to_string(),
                );
            }
        }
        write_value(&signature_file, &signature)?;
        if !body["check"]["clean"].as_bool().unwrap_or(true) {
            record
                .not_checked
                .push("KiCad's fill check of the proposed zones is not clean".to_string());
        }
        zones = proposals;
    }
    // Locked routing kept on a net of a part that moves (conservative: by net).
    flow.facts.locked_on_moved = locked_on_moved(&live, &folder.join(P3_FILE));
    flow.facts.export_refused = None;
    if flow.steps_asked.routing == RunOrSkip::Run {
        let text = if zones.is_empty() {
            base.clone()
        } else {
            crate::pour_plan::with_zones(&base, &zones)?
        };
        match export_check(ctx, &board, &live, &text, &folder).await? {
            Ok(()) => {}
            Err(reason) => flow.facts.export_refused = Some(reason),
        }
        write_value(
            &folder.join(EXPORT_FILE),
            &json!({ "refused": flow.facts.export_refused }),
        )?;
        if flow.facts.locked_routing > 0 {
            record
                .not_checked
                .push("the export check ran without the board's locked routing".to_string());
        }
    }
    record.counts =
        json!({ "zones": flow.facts.zones, "export_refused": flow.facts.export_refused.is_some() });
    record.result_file = Some(folder.join(P4_FILE).display().to_string());
    Ok(Ok(record))
}

/// Whether locked routing on the live board shares a net with a pad of a part the
/// winner moves. By net, not geometry: a conservative reading (gate A then recommends
/// deleting the locked routing too).
fn locked_on_moved(live: &str, placement: &Path) -> bool {
    let Ok(body) = read_json(placement) else {
        return false;
    };
    let moved: BTreeSet<String> = body["apply_with"]["arguments"]["placements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["reference"].as_str().map(str::to_string))
        .collect();
    if moved.is_empty() {
        return false;
    }
    let Ok(tree) = konnect_sexp::parse_sexp(live) else {
        return false;
    };
    let names = net_names(&tree);
    let locked: BTreeSet<String> = ["segment", "arc", "via"]
        .iter()
        .flat_map(|head| tree.find_all(head))
        .filter(|item| is_locked(item))
        .filter_map(|item| net_of(item, &names))
        .collect();
    if locked.is_empty() {
        return false;
    }
    tree.find_all("footprint").into_iter().any(|footprint| {
        reference_of(footprint).is_some_and(|r| moved.contains(&r))
            && footprint
                .find_all("pad")
                .into_iter()
                .filter_map(|pad| net_of(pad, &names))
                .any(|net| locked.contains(&net))
    })
}

/// The board's net table: number to name.
fn net_names(tree: &konnect_sexp::SexpNode) -> BTreeMap<String, String> {
    tree.find_all("net")
        .into_iter()
        .filter_map(|net| {
            Some((
                net.get(1)?.as_str()?.to_string(),
                net.get(2)?.as_str()?.to_string(),
            ))
        })
        .collect()
}

/// An item's net by name: `(net <number> "<name>")`, `(net "<name>")` or `(net <number>)`
/// read through the net table.
fn net_of(item: &konnect_sexp::SexpNode, names: &BTreeMap<String, String>) -> Option<String> {
    let net = item.find("net")?;
    if let Some(name) = net.get(2).and_then(|n| n.as_str()) {
        return Some(name.to_string());
    }
    let first = net.get(1)?.as_str()?;
    Some(
        names
            .get(first)
            .cloned()
            .unwrap_or_else(|| first.to_string()),
    )
}

/// `(locked yes)`, or the older bare `locked` atom.
fn is_locked(item: &konnect_sexp::SexpNode) -> bool {
    if let Some(locked) = item.find("locked") {
        return locked
            .get(1)
            .and_then(|v| v.as_str())
            .is_none_or(|v| v == "yes");
    }
    item.children()
        .unwrap_or(&[])
        .iter()
        .any(|child| child.as_str() == Some("locked"))
}

/// A footprint's reference: `(property "Reference" "<ref>")`.
fn reference_of(footprint: &konnect_sexp::SexpNode) -> Option<String> {
    footprint
        .find_all("property")
        .into_iter()
        .find(|p| p.get(1).and_then(|v| v.as_str()) == Some("Reference"))
        .and_then(|p| p.get(2)?.as_str().map(str::to_string))
}

/// FF17: `place_route::export_candidate` on `text`, in `<attempt>/export-check-<stamp>/`.
async fn export_check(
    ctx: &Arc<ToolContext>,
    board: &Path,
    live: &str,
    text: &str,
    folder: &Path,
) -> anyhow::Result<Result<(), String>> {
    let rules = match live_input(ctx, board).await? {
        Ok(live) => live.rules,
        Err(refusal) => return Ok(Err(short(&body_of(&refusal)))),
    };
    let root = folder.join(format!("export-check-{}", stamp_ms()));
    std::fs::create_dir_all(&root)?;
    let source = crate::tools::placement_converge::scratch_project(&root, board, live)?;
    let inputs = crate::place_route::RouteInputs {
        options: crate::place_route::export_options(&source),
        board: source,
        rules,
        effort: crate::route_search::Effort::M,
        fastroute: PathBuf::new(),
        kicad_cli: ctx.config.kicad_cli.clone(),
        kicad_config_home: root.join("kicad-config"),
    };
    let candidate = root.join("candidate");
    let text = text.to_string();
    Ok(tokio::task::spawn_blocking(move || {
        crate::place_route::export_candidate(&inputs, &candidate, &text)
    })
    .await?
    .map(|_| ())
    .map_err(|error| format!("{error:#}")))
}

/// What gate A approved of the pours: every proposal's `add_zone` arguments and its
/// fill, and whether the fill check is clean.
fn pours_signature(pours: &Value) -> Value {
    let zones: Vec<Value> = pours["proposals"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|zone| json!({ "add_zone": zone["add_zone"], "fill": zone["fill"] }))
        .collect();
    json!({ "zones": zones, "status": pours["status"], "clean": pours["check"]["clean"] })
}

/// FF4: the candidates routed with P2's zones. `route` adds the zones to the candidate's
/// text (`pour_plan::with_zones`) and hands it to `zoned`. `unrouted` checks that the
/// text it is given is the model's input board (`plain_text`), then hands `zoned_text` to
/// `zoned`, whose input board on disk holds `zoned_text`, so `PlaceRouteCall`'s own
/// check holds. The search's model, its input SHA-256 and `rebuild` never see the zones.
pub(crate) struct WithPours {
    pub zoned: Arc<dyn RouteCall>,
    pub zones: Vec<ProposedZone>,
    pub plain_text: String,
    pub zoned_text: String,
}

impl RouteCall for WithPours {
    fn route(
        &self,
        id: usize,
        board_text: String,
        folder: PathBuf,
    ) -> BoxFuture<(
        crate::place_judge::RouteResult,
        crate::place_search::Timings,
    )> {
        let zoned = Arc::clone(&self.zoned);
        let with = crate::pour_plan::with_zones(&board_text, &self.zones);
        Box::pin(async move {
            match with {
                Ok(text) => zoned.route(id, text, folder).await,
                Err(error) => (
                    crate::place_judge::RouteResult::Failed {
                        stage: "export",
                        reason: format!(
                            "the planned pours cannot be added to the candidate: {error:#}"
                        ),
                    },
                    crate::place_search::Timings::default(),
                ),
            }
        })
    }

    fn unrouted(
        &self,
        board_text: String,
        folder: PathBuf,
    ) -> BoxFuture<anyhow::Result<BTreeMap<String, usize>>> {
        let zoned = Arc::clone(&self.zoned);
        let same = board_text == self.plain_text;
        let zoned_text = self.zoned_text.clone();
        Box::pin(async move {
            anyhow::ensure!(same, "the text given is not the search's input board");
            zoned.unrouted(zoned_text, folder).await
        })
    }
}

/// L1: the live board is the planned one, and equals its saved file.
async fn unchanged(flow: &FlowRecord, io: &dyn FlowIo) -> anyhow::Result<Result<StepRecord, Stop>> {
    let (_, saved) = match live_equals_saved(io).await {
        Ok(read) => read,
        Err(stop) => return Ok(Err(stop)),
    };
    if saved != flow.planned_from {
        return Ok(Err(Stop::new(
            StopKind::BoardChanged,
            format!(
                "the saved board is {saved}, the plan was read from {}: the board changed since the plan. Abandon this flow and start again",
                flow.planned_from
            ),
        )));
    }
    Ok(Ok(StepRecord::new(Step::Unchanged, StepState::Done)))
}

/// R1 to R3, in `routes/<n>/`: the export, the route, the import check. The route's
/// facts (its judge's verdict, its follow-ups) go into the record for gate B and F2–F3.
async fn route_step(
    step: Step,
    state: &Path,
    flow: &mut FlowRecord,
    io: &dyn FlowIo,
) -> anyhow::Result<Result<StepRecord, Stop>> {
    let board = PathBuf::from(&flow.board);
    let folder = route_folder(state, flow);
    std::fs::create_dir_all(&folder)?;
    let steps_dir = flow_folder(state, &flow.flow_id).join("steps");
    let mut record = StepRecord::new(step, StepState::Done);
    match step {
        Step::Export => {
            let file = steps_dir.join(format!(
                "r1-{}-export_specctra_dsn.json",
                flow.route_attempt
            ));
            if let Err(stop) = read_call(
                io,
                "export_specctra_dsn",
                export_args(&board, &folder),
                &file,
            )
            .await?
            {
                return Ok(Err(stop));
            }
            record.result_file = Some(file.display().to_string());
        }
        Step::Route => {
            let file = folder.join(ROUTE_FILE);
            let log = steps_dir.join(format!("r2-{}-route_specctra_dsn.json", flow.route_attempt));
            let fastroute = PathBuf::from(flow.fastroute["path"].as_str().unwrap_or_default());
            let body = match read_call(
                io,
                "route_specctra_dsn",
                route_args(&folder, &flow.effort, &fastroute),
                &log,
            )
            .await?
            {
                Ok(body) => body,
                Err(stop) => return Ok(Err(stop)),
            };
            write_value(&file, &body)?;
            if body["success"] == json!(false) {
                return Ok(Err(Stop::new(
                    StopKind::Failed,
                    format!("the route named no winner: {}", short(&body)),
                )));
            }
            let follow = route_follow_ups(&body);
            flow.route = Some(RouteFacts {
                all_gates_pass: body["completion"]["judged"]["all_gates_pass"].as_bool(),
                plane_stitching: follow.plane.is_some(),
                reference_vias: follow.reference.is_some(),
                length_rules: !follow.rules.is_empty(),
            });
            record.counts = json!({ "status": body["status"], "all_gates_pass": body["completion"]["judged"]["all_gates_pass"] });
            record.not_checked = body["completion"]["not_checked"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| item.as_str().map(str::to_string))
                .chain(
                    follow
                        .left
                        .iter()
                        .map(|left| format!("left for you: {left}")),
                )
                .collect();
            record.result_file = Some(file.display().to_string());
        }
        _ => {
            let route = read_json(&folder.join(ROUTE_FILE))?;
            let Some(args) = step_arguments(&route, "plan_specctra_ses_import") else {
                return Ok(Err(Stop::new(
                    StopKind::Failed,
                    "the route's next steps hold no plan_specctra_ses_import step",
                )));
            };
            let file = steps_dir.join(format!(
                "r3-{}-plan_specctra_ses_import.json",
                flow.route_attempt
            ));
            let body = match read_call(io, "plan_specctra_ses_import", args, &file).await? {
                Ok(body) => body,
                Err(stop) => return Ok(Err(stop)),
            };
            if body["ok"] == json!(false) || body["status"] == "blocked" {
                return Ok(Err(Stop::new(
                    StopKind::Failed,
                    format!("the import check refused the session: {}", short(&body)),
                )));
            }
            record.result_file = Some(file.display().to_string());
        }
    }
    Ok(Ok(record))
}

/// The arguments of the route's first next step that calls `tool`.
fn step_arguments(route: &Value, tool: &str) -> Option<Value> {
    route["completion"]["next_steps"]
        .as_array()?
        .iter()
        .find(|step| step["tool"] == tool)
        .map(|step| step["arguments"].clone())
}

// ─── The live-step protocol ──────────────────────────────────────────────────

/// The calls of one writing step, each recorded and classified.
struct Calls<'a> {
    io: &'a dyn FlowIo,
    dir: PathBuf,
    step: Step,
    /// The attempt's stamp, so a step run again never overwrites an earlier call's file.
    tag: u128,
    made: usize,
    files: Vec<String>,
    /// A write of this step succeeded: a later refusal no longer means "nothing changed".
    wrote: bool,
    /// F2a and F2b: the verified `after` of each round done, and the checkpoint taken
    /// before each further round.
    rounds: Vec<String>,
    checkpoints: Vec<String>,
}

/// Why a step's calls ended early.
enum Halt {
    /// Refused; nothing changed by this call.
    Failed(String),
    /// The board may have changed.
    MayHaveChanged(String),
}

impl Calls<'_> {
    async fn run(
        &mut self,
        tool: &'static str,
        args: Value,
        writes: bool,
    ) -> anyhow::Result<Result<Value, Halt>> {
        self.made += 1;
        let called = self.io.call(tool, args.clone()).await;
        let outcome = outcome_of_call(&called, writes);
        let body = match &called {
            Called::Answered(result) | Called::Before(result) => body_of(result),
            Called::Erred(error) => json!({ "error": error }),
        };
        let file = self.dir.join(format!(
            "{}-{}-{:02}-{tool}.json",
            self.step.id(),
            self.tag,
            self.made
        ));
        write_value(
            &file,
            &json!({ "tool": tool, "arguments": args, "result": body }),
        )?;
        self.files.push(file.display().to_string());
        Ok(match outcome {
            Outcome::Succeeded | Outcome::Unchanged => {
                self.wrote |= writes;
                Ok(body)
            }
            // A refusal after an earlier write of the same step: the board holds that
            // write, unsaved (M5).
            Outcome::Failed if self.wrote => Err(Halt::MayHaveChanged(format!(
                "{tool} refused after an earlier call of this step changed the board, which KiCad holds unsaved: {}",
                short(&body)
            ))),
            Outcome::Failed => Err(Halt::Failed(format!("{tool} refused: {}", short(&body)))),
            Outcome::MayHaveChanged => Err(Halt::MayHaveChanged(format!(
                "{tool} may have changed the board: {}",
                short(&body)
            ))),
        })
    }

    async fn read(
        &mut self,
        tool: &'static str,
        args: Value,
    ) -> anyhow::Result<Result<Value, Halt>> {
        self.run(tool, args, false).await
    }

    async fn write(
        &mut self,
        tool: &'static str,
        args: Value,
    ) -> anyhow::Result<Result<Value, Halt>> {
        self.run(tool, args, true).await
    }
}

/// B's `Outcome::of` for a result, for a writing call. A handler `Err` of a writing tool
/// is `MayHaveChanged`; anything refused before the handler is `Failed`; a reading call
/// that fails is `Failed` whatever its kind, since it changes nothing.
fn outcome_of_call(called: &Called, writes: bool) -> Outcome {
    let outcome = match called {
        Called::Before(_) => Outcome::Failed,
        Called::Answered(result) => Outcome::of(result),
        Called::Erred(_) => Outcome::MayHaveChanged,
    };
    if !writes && outcome == Outcome::MayHaveChanged {
        Outcome::Failed
    } else {
        outcome
    }
}

/// What a writing step's body did.
struct Work {
    counts: Value,
    rounds: Vec<String>,
    not_checked: Vec<String>,
}

/// The live-step protocol of "Each live step", steps 1 to 6:
/// 1. the intent in `flow.json` (`Calling`, with `before`);
/// 2. liveness: the live board equals its saved file, with tries;
/// 3. `plan_live_step` against the `after` of the step before;
/// 4. the checkpoint;
/// 5. the step's calls, each recorded and classified, the first that is not `Succeeded`
///    ending the step;
/// 6. `verified_save`, giving `after` (for rounds, after each round).
async fn live_step(
    step: Step,
    state: &Path,
    flow: &mut FlowRecord,
    io: &dyn FlowIo,
) -> anyhow::Result<Result<StepRecord, Stop>> {
    // 2 and 3 first: nothing is recorded as under way before the board is known to be
    // where the step begins.
    let (_, found) = match live_equals_saved(io).await {
        Ok(read) => read,
        Err(stop) => return Ok(Err(stop)),
    };
    let expected = flow.expected_before(step);
    if let LiveStart::Changed { expected, found } = plan_live_step(&found, &expected) {
        return Ok(Err(Stop::new(
            StopKind::BoardChanged,
            format!("the saved board is {found}, and the step before left it at {expected}"),
        )));
    }
    // 4. The checkpoint.
    let checkpoint = match io
        .checkpoint(format!(
            "full flow {}: before {} ({})",
            flow.flow_id,
            step.id(),
            step.title()
        ))
        .await
    {
        Ok(id) => id,
        Err(why) => {
            return Ok(Err(Stop::new(
                StopKind::Failed,
                format!("no checkpoint could be made ({why}), so nothing was changed"),
            )))
        }
    };
    if flow.start_checkpoint.is_none() {
        flow.start_checkpoint = Some(checkpoint.clone());
    }
    if !flow.checkpoints.contains(&checkpoint) {
        flow.checkpoints.push(checkpoint.clone());
    }
    // 1. The intent.
    let mut record = StepRecord::new(step, StepState::Calling);
    record.before = Some(found.clone());
    record.checkpoint = Some(checkpoint);
    flow.put(record.clone());
    flow.at = Some(step);
    save_flow(state, flow)?;
    // 5. The calls.
    let mut calls = Calls {
        io,
        dir: flow_folder(state, &flow.flow_id).join("steps"),
        step,
        tag: stamp_ms(),
        made: 0,
        files: Vec::new(),
        wrote: false,
        rounds: Vec::new(),
        checkpoints: Vec::new(),
    };
    let work = step_work(step, state, flow, &mut calls).await?;
    record.result_file = calls.files.last().cloned();
    record.rounds = calls.rounds.clone();
    for checkpoint in &calls.checkpoints {
        if !flow.checkpoints.contains(checkpoint) {
            flow.checkpoints.push(checkpoint.clone());
        }
    }
    let work = match work {
        Ok(work) => work,
        Err(Halt::Failed(why)) => {
            record.state = StepState::Failed;
            record.why = Some(why.clone());
            flow.put(record);
            return Ok(Err(Stop::new(StopKind::Failed, why)));
        }
        Err(Halt::MayHaveChanged(why)) => {
            record.state = StepState::Uncertain;
            record.why = Some(why.clone());
            flow.put(record);
            return Ok(Err(Stop::new(StopKind::Uncertain, why)));
        }
    };
    record.counts = work.counts;
    if !work.rounds.is_empty() {
        record.rounds = work.rounds;
    }
    record.not_checked = work.not_checked;
    // 6. The verified save.
    match verified_save(io).await {
        Ok(after) => {
            record.after = Some(after);
            record.state = StepState::Done;
            Ok(Ok(record))
        }
        Err(why) => {
            record.state = StepState::Unsaved;
            record.why = Some(why.clone());
            flow.put(record);
            Ok(Err(Stop::new(StopKind::Unsaved, why)))
        }
    }
}

/// `save_project`, then the live text and the saved file, both read and their
/// `canonical_sha256` compared: `Ok(hash)` when equal; else why the step is unsaved.
async fn verified_save(io: &dyn FlowIo) -> Result<String, String> {
    // `save_project` saves KiCad's current board: the live read checks that it is this
    // board and that it is the only one open (M6).
    with_tries(io, || io.live_text()).await?;
    match io.call("save_project", json!({})).await {
        Called::Answered(result) if !result.is_error => {}
        Called::Answered(result) | Called::Before(result) => {
            return Err(format!(
                "save_project refused: {}",
                short(&body_of(&result))
            ))
        }
        Called::Erred(error) => return Err(format!("save_project failed: {error}")),
    }
    match live_equals_saved(io).await {
        Ok((_, saved)) => Ok(saved),
        Err(stop) => Err(format!("the save did not verify: {}", stop.why)),
    }
}

/// The calls of each writing step.
async fn step_work(
    step: Step,
    state: &Path,
    flow: &FlowRecord,
    calls: &mut Calls<'_>,
) -> anyhow::Result<Result<Work, Halt>> {
    let board = PathBuf::from(&flow.board);
    let attempt = attempt_folder(state, flow);
    let route = route_folder(state, flow);
    let plain = |counts: Value| Work {
        counts,
        rounds: Vec::new(),
        not_checked: Vec::new(),
    };
    match step {
        Step::DeleteRouting => {
            let include_locked =
                flow.approvals.delete_routing == Some(DeleteAnswer::DeleteIncludingLocked);
            let approved = read_json(&attempt.join(P1_FILE))?;
            let fresh = match calls
                .read(
                    "delete_all_routing",
                    delete_preview_args(&board, include_locked),
                )
                .await?
            {
                Ok(fresh) => fresh,
                Err(halt) => return Ok(Err(halt)),
            };
            if let Err(why) = same_deletion(&approved, &fresh, include_locked) {
                return Ok(Err(Halt::Failed(why)));
            }
            if fresh["status"] == "noop" {
                return Ok(Ok(plain(json!({ "total": 0 }))));
            }
            let revision = fresh["plan_revision"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            match calls
                .write(
                    "delete_all_routing",
                    delete_apply_args(&board, include_locked, &revision),
                )
                .await?
            {
                Ok(body) => Ok(Ok(plain(counts_of(step, &body)))),
                Err(halt) => Ok(Err(halt)),
            }
        }
        Step::ApplyPlacement => {
            let placement = read_json(&attempt.join(P3_FILE))?;
            let Some(args) = placement_args(&placement) else {
                return Ok(Err(Halt::Failed(
                    "the placement plan has no apply_with".to_string(),
                )));
            };
            match calls
                .write("set_component_placements", args.clone())
                .await?
            {
                Ok(_) => Ok(Ok(plain(counts_of(step, &args)))),
                Err(halt) => Ok(Err(halt)),
            }
        }
        Step::AddZones => {
            let pours = read_json(&attempt.join(P4_FILE))?;
            let live = match with_tries(calls.io, || calls.io.live_text()).await {
                Ok(live) => live,
                Err(why) => {
                    return Ok(Err(Halt::Failed(format!(
                        "the board cannot be read: {why}"
                    ))))
                }
            };
            let present = zones_present(&live);
            let mut added = 0;
            for args in zone_calls(&pours) {
                let key = format!(
                    "{}@{}",
                    args["net_name"].as_str().unwrap_or_default(),
                    args["layer"].as_str().unwrap_or_default()
                );
                if present.contains(&key) {
                    continue;
                }
                if let Err(halt) = calls.write("add_zone", args).await? {
                    return Ok(Err(halt));
                }
                added += 1;
            }
            match calls
                .write("refill_zones", json!({ "board": board }))
                .await?
            {
                Ok(_) => Ok(Ok(plain(json!({ "zones": added })))),
                Err(halt) => Ok(Err(halt)),
            }
        }
        Step::ApplySession => {
            let body = read_json(&route.join(ROUTE_FILE))?;
            let Some(mut args) = step_arguments(&body, "apply_specctra_ses") else {
                return Ok(Err(Halt::Failed(
                    "the route's next steps hold no apply_specctra_ses step".to_string(),
                )));
            };
            // A new candidate path for every attempt: the apply never replaces one (FF14).
            args["candidate_output_path"] = json!(candidate_path(&args, &route));
            match calls.write("apply_specctra_ses", args).await? {
                Ok(applied) => Ok(Ok(plain(counts_of(step, &applied)))),
                Err(halt) => Ok(Err(halt)),
            }
        }
        Step::PlaneStitch | Step::ReferenceVias => {
            let body = read_json(&route.join(ROUTE_FILE))?;
            let follow = route_follow_ups(&body);
            let group = if step == Step::PlaneStitch {
                follow.plane
            } else {
                follow.reference
            };
            let Some(group) = group else {
                return Ok(Ok(plain(json!({ "rounds": 0, "vias": 0 }))));
            };
            stitch_rounds(step, &group, calls).await
        }
        Step::LengthRules => {
            let body = read_json(&route.join(ROUTE_FILE))?;
            let mut by_action: BTreeMap<String, usize> = BTreeMap::new();
            for args in route_follow_ups(&body).rules {
                let mut preview = args.clone();
                preview["dry_run"] = json!(true);
                let previewed = match calls.read("create_custom_drc_rule", preview).await? {
                    Ok(previewed) => previewed,
                    Err(halt) => return Ok(Err(halt)),
                };
                if previewed["status"] == "noop" {
                    *by_action.entry("unchanged".to_string()).or_default() += 1;
                    continue;
                }
                let mut apply = args.clone();
                apply["dry_run"] = json!(false);
                apply["expected_plan_revision"] = previewed["plan_revision"].clone();
                match calls.write("create_custom_drc_rule", apply).await? {
                    Ok(applied) => {
                        for rule in applied["rules"].as_array().into_iter().flatten() {
                            let action = rule["action"].as_str().unwrap_or("written").to_string();
                            *by_action.entry(action).or_default() += 1;
                        }
                    }
                    Err(halt) => return Ok(Err(halt)),
                }
            }
            Ok(Ok(plain(json!({ "rules": by_action }))))
        }
        Step::Finish => match calls
            .write(
                "run_drc",
                json!({ "board": board, "sync_live_board": true, "refill_zones": true }),
            )
            .await?
        {
            Ok(drc) => {
                write_value(&flow_folder(state, &flow.flow_id).join("drc.json"), &drc)?;
                Ok(Ok(plain(counts_of(step, &drc))))
            }
            Err(halt) => Ok(Err(halt)),
        },
        _ => Ok(Err(Halt::Failed(format!(
            "{} is not a writing step",
            step.id()
        )))),
    }
}

/// A new candidate path for F1, beside the route's own one, unique per attempt.
fn candidate_path(args: &Value, route: &Path) -> PathBuf {
    let folder = args["candidate_output_path"]
        .as_str()
        .and_then(|path| Path::new(path).parent().map(Path::to_path_buf))
        .unwrap_or_else(|| route.to_path_buf());
    folder.join(format!("candidate-{}.kicad_pcb", stamp_ms()))
}

/// F2a and F2b (FF10). A round is the planner's call (its arguments as the route's step
/// gives them); then, when it proposes stitching vias, `route_batch` calls of at most
/// [`BATCH_CAP`] objects; then `run_drc` with refill. F2a stops when the planner proposes
/// nothing or the DRC finds 0 unconnected items, after at most [`STITCH_ROUNDS`]; a save
/// is made before each further round, because the planner reads the saved board. F2b
/// runs one round, and a planner refusal ends it as "left for you".
async fn stitch_rounds(
    step: Step,
    group: &[Value],
    calls: &mut Calls<'_>,
) -> anyhow::Result<Result<Work, Halt>> {
    let planner = group
        .iter()
        .find(|s| s["tool"] == "plan_route_refinement")
        .map(|s| s["arguments"].clone());
    let drc = group
        .iter()
        .find(|s| s["tool"] == "run_drc")
        .map(|s| s["arguments"].clone());
    let board = group
        .iter()
        .find(|s| s["tool"] == "route_batch")
        .map(|s| s["arguments"]["board"].clone());
    let (Some(planner), Some(drc), Some(board)) = (planner, drc, board) else {
        return Ok(Ok(Work {
            counts: json!({ "rounds": 0, "vias": 0 }),
            rounds: Vec::new(),
            not_checked: vec![format!(
                "left for you: the {} steps of the route are not in the expected form",
                step.title()
            )],
        }));
    };
    let max = if step == Step::PlaneStitch {
        STITCH_ROUNDS
    } else {
        1
    };
    let (mut vias, mut rounds_done, mut rounds, mut not_checked) =
        (0usize, 0usize, Vec::new(), Vec::new());
    for round in 0..max {
        if round > 0 {
            // The planner reads the saved board: save, verify, and take a checkpoint of
            // the round's board before the next round (FF10).
            match verified_save(calls.io).await {
                Ok(after) => {
                    rounds.push(after.clone());
                    calls.rounds.push(after);
                }
                Err(why) => return Ok(Err(Halt::MayHaveChanged(why))),
            }
            match calls
                .io
                .checkpoint(format!(
                    "full flow: before {} round {}",
                    step.id(),
                    round + 1
                ))
                .await
            {
                Ok(id) => calls.checkpoints.push(id),
                Err(why) => {
                    return Ok(Err(Halt::MayHaveChanged(format!(
                        "no checkpoint could be made before round {} ({why})",
                        round + 1
                    ))))
                }
            }
        }
        let planned = match calls.read("plan_route_refinement", planner.clone()).await? {
            Ok(planned) => planned,
            Err(Halt::Failed(why)) if step == Step::ReferenceVias => {
                not_checked.push(format!("left for you: the reference vias ({why}); place a ground via within 3 mm of each signal via the audit lists under 3.2"));
                break;
            }
            Err(halt) => return Ok(Err(halt)),
        };
        let batches = stitch_batches(&planned, &board);
        if batches.is_empty() {
            break;
        }
        for batch in batches {
            vias += batch["vias"].as_array().map_or(0, Vec::len);
            if let Err(halt) = calls.write("route_batch", batch).await? {
                return Ok(Err(halt));
            }
        }
        rounds_done += 1;
        let checked = match calls.write("run_drc", drc.clone()).await? {
            Ok(checked) => checked,
            Err(halt) => return Ok(Err(halt)),
        };
        if unconnected_of(&checked) == Some(0) {
            break;
        }
    }
    Ok(Ok(Work {
        counts: json!({ "rounds": rounds_done, "vias": vias }),
        rounds,
        not_checked,
    }))
}

/// The `route_batch` calls that adopt every stitching proposal of a planner result: the
/// tracks and vias of their `route_batch` operations, in order, split so that no call
/// holds more than [`BATCH_CAP`] objects.
fn stitch_batches(planned: &Value, board: &Value) -> Vec<Value> {
    let mut objects: Vec<(&'static str, Value)> = Vec::new();
    for proposal in planned["proposals"].as_array().into_iter().flatten() {
        if proposal["kind"] != "stitching" {
            continue;
        }
        for operation in proposal["operations"].as_array().into_iter().flatten() {
            if operation["tool"] != "route_batch" {
                continue;
            }
            for track in operation["arguments"]["tracks"]
                .as_array()
                .into_iter()
                .flatten()
            {
                objects.push(("tracks", track.clone()));
            }
            for via in operation["arguments"]["vias"]
                .as_array()
                .into_iter()
                .flatten()
            {
                objects.push(("vias", via.clone()));
            }
        }
    }
    objects
        .chunks(BATCH_CAP)
        .map(|chunk| {
            let (mut tracks, mut vias) = (Vec::new(), Vec::new());
            for (kind, object) in chunk {
                if *kind == "tracks" {
                    tracks.push(object.clone());
                } else {
                    vias.push(object.clone());
                }
            }
            json!({ "board": board, "tracks": tracks, "vias": vias })
        })
        .collect()
}

/// The nets and layers of the zones on a board text, as `net@layer`.
fn zones_present(text: &str) -> BTreeSet<String> {
    let Ok(tree) = konnect_sexp::parse_sexp(text) else {
        return BTreeSet::new();
    };
    tree.find_all("zone")
        .into_iter()
        .filter_map(|zone| {
            let net = zone.find_str("net_name")?.to_string();
            let layer = zone
                .find_str("layer")
                .or_else(|| zone.find("layers")?.get(1)?.as_str())?
                .to_string();
            Some(format!("{net}@{layer}"))
        })
        .collect()
}

// ─── The adapters to B to E: one place each, pinned by a test ────────────────

/// C: `delete_all_routing`'s dry run (P1, L2's fresh one, and `already_done`).
pub(crate) fn delete_preview_args(board: &Path, include_locked: bool) -> Value {
    json!({ "board": board, "dry_run": true, "include_locked": include_locked })
}

/// C: the apply, bound to the fresh dry run's revision.
pub(crate) fn delete_apply_args(board: &Path, include_locked: bool, plan_revision: &str) -> Value {
    json!({ "board": board, "dry_run": false, "include_locked": include_locked, "expected_plan_revision": plan_revision })
}

/// FF9: the approved counts (P1, without `include_locked`) against a fresh dry run's.
/// Without `include_locked` the fresh `will_delete` and `kept_locked` must equal P1's;
/// with it, the fresh `will_delete` must be P1's `will_delete` and `kept_locked` together.
pub(crate) fn same_deletion(
    approved: &Value,
    fresh: &Value,
    include_locked: bool,
) -> Result<(), String> {
    for kind in ["total", "tracks", "arcs", "vias"] {
        let count = |body: &Value, part: &str| body[part][kind].as_u64().unwrap_or(0);
        let (want, got) = if include_locked {
            (
                count(approved, "will_delete") + count(approved, "kept_locked"),
                count(fresh, "will_delete"),
            )
        } else {
            (count(approved, "will_delete"), count(fresh, "will_delete"))
        };
        if want != got {
            return Err(format!(
                "the routing to delete changed since the plan: {kind} was {want}, a fresh count says {got}. Nothing was deleted; the board changed, so abandon this flow and start again"
            ));
        }
        if !include_locked && count(approved, "kept_locked") != count(fresh, "kept_locked") {
            return Err(format!(
                "the locked routing kept changed since the plan ({kind}). Nothing was deleted; abandon this flow and start again"
            ));
        }
    }
    Ok(())
}

/// D/CT: the winner's `apply_with` arguments (`set_component_placements`).
pub(crate) fn placement_args(placement: &Value) -> Option<Value> {
    let apply = &placement["apply_with"];
    (apply["tool"] == "set_component_placements").then(|| apply["arguments"].clone())
}

/// ZR4: `add_zone` once per proposal, from the plan's own `add_zone` arguments.
pub(crate) fn zone_calls(pours: &Value) -> Vec<Value> {
    pours["proposals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|proposal| proposal.get("add_zone").cloned())
        .collect()
}

/// R1: the export into the route folder, pours as the default (`fixed`).
pub(crate) fn export_args(board: &Path, route: &Path) -> Value {
    json!({ "board": board, "output": route.join("board.dsn") })
}

/// R2: the route with fastroute at the flow's effort, with the located binary.
pub(crate) fn route_args(route: &Path, effort: &str, fastroute: &Path) -> Value {
    json!({
        "dsn_path": route.join("board.dsn"),
        "ses_output_path": route.join("routed.ses"),
        "effort": effort,
        "fastroute_path": fastroute,
    })
}

/// F2a, F2b and F3: the steps of the route's `completion.next_steps` the flow runs.
pub(crate) struct FollowUps {
    /// The plane-stitching group (planner, `route_batch`, `run_drc` with `repeat`).
    pub plane: Option<Vec<Value>>,
    /// The reference-via group (the same three, after the plane group, no `repeat`).
    pub reference: Option<Vec<Value>>,
    /// Each structured `create_custom_drc_rule` step's arguments.
    pub rules: Vec<Value>,
    /// Every other step that is not the flow's own (export, import, apply, the first DRC,
    /// a save, a checkpoint), as its text: left for the user.
    pub left: Vec<String>,
}

/// The tools of the route's next steps the flow runs itself in its own steps.
const FLOW_OWN: [&str; 5] = [
    "save_project",
    "create_checkpoint",
    "plan_specctra_ses_import",
    "apply_specctra_ses",
    "run_drc",
];

pub(crate) fn route_follow_ups(route: &Value) -> FollowUps {
    let steps: Vec<Value> = route["completion"]["next_steps"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut follow = FollowUps {
        plane: None,
        reference: None,
        rules: Vec::new(),
        left: Vec::new(),
    };
    let mut index = 0;
    while index < steps.len() {
        let step = &steps[index];
        if step["tool"] == "plan_route_refinement" {
            let group: Vec<Value> = steps[index..steps.len().min(index + 3)].to_vec();
            let shaped = group.len() == 3
                && group[1]["tool"] == "route_batch"
                && group[2]["tool"] == "run_drc";
            if shaped {
                if group[2].get("repeat").is_some() && follow.plane.is_none() {
                    follow.plane = Some(group);
                } else if follow.reference.is_none() {
                    follow.reference = Some(group);
                }
                index += 3;
                continue;
            }
        }
        if step["tool"] == "create_custom_drc_rule" && step["arguments"]["rules"].is_array() {
            follow.rules.push(step["arguments"].clone());
        } else if !step["tool"]
            .as_str()
            .is_some_and(|tool| FLOW_OWN.contains(&tool))
        {
            let text = step["manual"]
                .as_str()
                .or_else(|| step["why"].as_str())
                .or_else(|| step["tool"].as_str())
                .unwrap_or("a next step of the route")
                .to_string();
            follow.left.push(text);
        }
        index += 1;
    }
    follow
}

// ─── Resume: is a step already done (FF15) ───────────────────────────────────

/// Whether a writing step's work is on the board, from the live board and the step's
/// arguments. L2: a fresh dry run says `noop`. L3: every footprint of `apply_with` is at
/// its target position and turn. L4: a zone of each proposal's net and layer exists. F1:
/// the board holds more routing than the locked routing that stayed. F3: E's dry run
/// reports every rule batch `noop`. F2a, F2b and F4: never (they run again: a further
/// stitching round or DRC changes nothing that is already right).
async fn already_done(
    step: Step,
    state: &Path,
    flow: &FlowRecord,
    io: &dyn FlowIo,
) -> anyhow::Result<bool> {
    let board = PathBuf::from(&flow.board);
    let attempt = attempt_folder(state, flow);
    let live = || async { with_tries(io, || io.live_text()).await.unwrap_or_default() };
    Ok(match step {
        Step::DeleteRouting => {
            let include_locked =
                flow.approvals.delete_routing == Some(DeleteAnswer::DeleteIncludingLocked);
            match io
                .call(
                    "delete_all_routing",
                    delete_preview_args(&board, include_locked),
                )
                .await
            {
                Called::Answered(result) if !result.is_error => {
                    body_of(&result)["status"] == "noop"
                }
                _ => false,
            }
        }
        Step::ApplyPlacement => {
            let Ok(plan) = read_json(&attempt.join(P3_FILE)) else {
                return Ok(false);
            };
            let Some(args) = placement_args(&plan) else {
                return Ok(false);
            };
            placements_at(&live().await, &args)
        }
        Step::AddZones => {
            let Ok(pours) = read_json(&attempt.join(P4_FILE)) else {
                return Ok(false);
            };
            let present = zones_present(&live().await);
            zone_calls(&pours).iter().all(|args| {
                present.contains(&format!(
                    "{}@{}",
                    args["net_name"].as_str().unwrap_or_default(),
                    args["layer"].as_str().unwrap_or_default()
                ))
            })
        }
        Step::ApplySession => routing_count(&live().await) > flow.facts.locked_routing,
        Step::LengthRules => {
            let Ok(route) = read_json(&route_folder(state, flow).join(ROUTE_FILE)) else {
                return Ok(false);
            };
            let mut all = true;
            for mut args in route_follow_ups(&route).rules {
                args["dry_run"] = json!(true);
                all &= matches!(io.call("create_custom_drc_rule", args).await,
                    Called::Answered(result) if !result.is_error && body_of(&result)["status"] == "noop");
            }
            all
        }
        _ => false,
    })
}

/// Top-level tracks, arcs and vias of a board text (teardrop zones are not routing here).
fn routing_count(text: &str) -> usize {
    konnect_sexp::parse_sexp(text).map_or(0, |tree| {
        ["segment", "arc", "via"]
            .iter()
            .map(|head| tree.find_all(head).len())
            .sum()
    })
}

/// Whether every footprint `args` places is at its target (0.001 mm, 0.01 degree).
fn placements_at(live: &str, args: &Value) -> bool {
    let Ok(tree) = konnect_sexp::parse_sexp(live) else {
        return false;
    };
    let mut at: BTreeMap<String, (f64, f64, f64)> = BTreeMap::new();
    for footprint in tree.find_all("footprint") {
        let reference = reference_of(footprint);
        let position = footprint
            .find("at")
            .and_then(|a| Some((a.get_f64(1)?, a.get_f64(2)?, a.get_f64(3).unwrap_or(0.0))));
        if let (Some(reference), Some(position)) = (reference, position) {
            at.insert(reference, position);
        }
    }
    let turn = |a: f64, b: f64| ((a - b).rem_euclid(360.0)).min((b - a).rem_euclid(360.0));
    args["placements"]
        .as_array()
        .into_iter()
        .flatten()
        .all(|p| {
            let Some(reference) = p["reference"].as_str() else {
                return false;
            };
            let (Some(x), Some(y)) = (p["x"].as_f64(), p["y"].as_f64()) else {
                return false;
            };
            let rotation = p["rotation"].as_f64().unwrap_or(0.0);
            at.get(reference).is_some_and(|(ax, ay, ar)| {
                (ax - x).abs() < 1e-3 && (ay - y).abs() < 1e-3 && turn(*ar, rotation) < 1e-2
            })
        })
}

// ─── Gates, reports, answers, resume, abandon ────────────────────────────────

/// The flow waits at `gate`: its body, `needs` and revision.
/// Gate A again, for a combination of approvals that is refused: the plan, and the
/// refusal that names the key to change.
fn open_refused_gate(state: &Path, flow: &mut FlowRecord, reason: &str) {
    let mut body = gate_body(state, flow, Gate::Plan);
    body["refused"] = json!(reason);
    flow.plan_revision = Some(plan_revision(
        &flow.flow_id,
        Gate::Plan,
        &flow.planned_from,
        &body,
    ));
    flow.gate_body = Some(body);
    flow.gate = Some(Gate::Plan);
    flow.state = FlowState::AwaitingAnswer;
    flow.at = None;
}

fn open_gate(state: &Path, flow: &mut FlowRecord, gate: Gate) {
    let body = gate_body(state, flow, gate);
    flow.plan_revision = Some(plan_revision(
        &flow.flow_id,
        gate,
        &flow.planned_from,
        &body,
    ));
    flow.gate_body = Some(body);
    flow.gate = Some(gate);
    flow.state = FlowState::AwaitingAnswer;
}

/// The needs of `gate` for `flow`.
fn gate_needs(flow: &FlowRecord, gate: Gate) -> Vec<Need> {
    match gate {
        Gate::Roles => needs_roles(&flow.facts.named_only, &flow.approvals)
            .into_iter()
            .collect(),
        Gate::Plan => needs_at_plan(&flow.facts, &flow.steps_asked, &flow.approvals),
        Gate::Routing => needs_routing(&flow.approvals, flow.route.and_then(|r| r.all_gates_pass))
            .into_iter()
            .collect(),
    }
}

/// What a gate shows, as "What gate A shows" lists it, from the plan's files.
fn gate_body(state: &Path, flow: &FlowRecord, gate: Gate) -> Value {
    let file = |name: &str| -> Value {
        read_json(&attempt_folder(state, flow).join(name)).unwrap_or(Value::Null)
    };
    let needs: Vec<Value> = gate_needs(flow, gate)
        .iter()
        .map(|need| json!({ "key": need.key, "recommended": need.recommended, "why": need.why }))
        .collect();
    match gate {
        Gate::Roles => json!({
            "gate": "roles",
            "named_only": flow.facts.named_only,
            "needs": needs,
        }),
        Gate::Plan => {
            let p1 = file(P1_FILE);
            let p3 = file(P3_FILE);
            let p4 = file(P4_FILE);
            json!({
                "gate": "plan",
                "placement": {
                    "status": p3["status"],
                    "change": p3["winner"]["change"],
                    "moves": p3["winner"]["moves"],
                    "moves_total": p3["winner"]["moves_total"],
                    "gate_failures": p3["winner"]["gate_failures"],
                    "routed_with_pours": p3["with_pours"],
                    "reasons": p3["reasons"],
                },
                "routing_that_goes": { "will_delete": p1["will_delete"], "kept_locked": p1["kept_locked"] },
                "zones": p4["proposals"].as_array().map(|rows| rows.iter().map(|z| json!({
                    "net": z["net"], "layer": z["layer"], "fill": z["fill"], "reason": z["reason"],
                })).collect::<Vec<_>>()),
                "fill_check": { "status": p4["status"], "clean": p4["check"]["clean"], "risen": p4["check"]["risen"] },
                "roles": p4["nets"],
                "export_check": match &flow.facts.export_refused {
                    None => json!({ "passed": flow.steps_asked.routing == RunOrSkip::Run }),
                    Some(reason) => json!({ "passed": false, "reason": reason }),
                },
                "warning": "The old routing is deleted before the new routing exists; the way back is the start checkpoint.",
                "needs": needs,
            })
        }
        Gate::Routing => {
            let route =
                read_json(&route_folder(state, flow).join(ROUTE_FILE)).unwrap_or(Value::Null);
            json!({
                "gate": "routing",
                "status": route["status"],
                "summary": route["summary"],
                "gates": route["completion"]["judged"]["gates"],
                "all_gates_pass": route["completion"]["judged"]["all_gates_pass"],
                "needs": needs,
            })
        }
    }
}

/// The view a call returns: `status`, `phase`, the gate with `needs`, `answer_with` and
/// `plan_revision`, the summary when done, the stop's reason and way back, and
/// `next_steps`.
fn view(state: &Path, flow: &FlowRecord, running: Option<(&Value, f64)>) -> Value {
    let wait = format!(
        "{{\"run_id\": \"{}\", \"wait_s\": {SUGGESTED_WAIT_S}}}",
        flow.flow_id
    );
    let mut body = json!({
        "status": flow.state.name(),
        "run_id": flow.flow_id,
        "board": flow.board,
        "board_state": flow.board_state,
        "phase": flow.at.map(Step::title),
        "steps": flow.records.iter().map(|r| json!({ "step": r.step.id(), "title": r.step.title(), "state": r.state.name(), "why": r.why })).collect::<Vec<_>>(),
        "records": flow_folder(state, &flow.flow_id).display().to_string(),
    });
    if let Some((progress, elapsed)) = running {
        body["progress"] = progress.clone();
        body["elapsed_s"] = json!((elapsed * 10.0).round() / 10.0);
    }
    let next_steps: Vec<String> = match flow.state {
        FlowState::Running => vec![
            format!("The flow runs in the background. Call run_full_flow with {wait} until `status` is no longer `running`; keep wait_s under your client's tool time limit."),
            "Start no other run on this board meanwhile.".to_string(),
        ],
        FlowState::AwaitingAnswer => {
            let gate = flow.gate.unwrap_or(Gate::Plan);
            let needs = gate_needs(flow, gate);
            let mut answer = serde_json::Map::new();
            for need in &needs {
                if let Some(value) = &need.recommended {
                    answer.insert(need.key.to_string(), value.clone());
                }
            }
            body["gate"] = flow.gate_body.clone().unwrap_or(Value::Null);
            body["plan_revision"] = json!(flow.plan_revision);
            body["answer_with"] = json!({
                "run_id": flow.flow_id,
                "answer": Value::Object(answer),
                "expected_plan_revision": flow.plan_revision,
            });
            let undecided: Vec<&str> = needs.iter().filter(|n| n.recommended.is_none()).map(|n| n.key).collect();
            let mut steps = vec![format!(
                "Show the user the plan in `gate` ({}). On a plain yes call run_full_flow with `answer_with` unchanged; otherwise change the keys the user named.",
                gate.name()
            )];
            if !undecided.is_empty() {
                steps.push(format!("The plan recommends nothing for {}: the user decides, and the answer must give it.", undecided.join(", ")));
            }
            steps
        }
        FlowState::Stopped => {
            body["stop_reason"] = json!(flow.stop_reason);
            let at = flow.at.and_then(|step| flow.record(step));
            body["way_back"] = json!({
                "start": way_back(flow, flow.start_checkpoint.as_deref()),
                "before_step": way_back(flow, at.and_then(|r| r.checkpoint.as_deref())),
            });
            let mut steps = Vec::new();
            if flow.board_state == BoardState::Uncertain {
                steps.push("Inspect the board in KiCad first: the stopped step may or may not have changed it.".to_string());
            }
            steps.push(format!(
                "Tell the user why the flow stopped. Once the cause is fixed call run_full_flow with {{\"run_id\": \"{}\", \"resume\": true}}; or end it with abandon: true (nothing is undone). The way back is `way_back`.",
                flow.flow_id
            ));
            steps
        }
        FlowState::Done => {
            let route = read_json(&route_folder(state, flow).join(ROUTE_FILE)).ok();
            body["summary"] = summary(flow, route.as_ref().map(|r| &r["summary"]));
            body["placement"] = read_json(&attempt_folder(state, flow).join(P3_FILE))
                .map(|p| json!({ "moves": p["winner"]["moves"], "change": p["winner"]["change"] }))
                .unwrap_or(Value::Null);
            if let Some(route) = &route {
                body["routing"] = json!({
                    "status": route["status"],
                    "summary": route["summary"],
                    "gates": route["completion"]["judged"]["gates"],
                });
            }
            body["drc"] = read_json(&flow_folder(state, &flow.flow_id).join("drc.json"))
                .map(|drc| counts_of(Step::Finish, &drc))
                .unwrap_or(Value::Null);
            let mut steps = vec!["Tell the user what the flow did, step by step, from `summary.lines`, and what was not checked.".to_string()];
            let rails: Vec<String> = flow.approvals.pour_roles.iter().filter_map(|roles| match roles {
                RolesAnswer::Map(map) => Some(map.keys().cloned().collect::<Vec<_>>().join(", ")),
                RolesAnswer::AsNamed(_) => (!flow.facts.named_only.is_empty()).then(|| flow.facts.named_only.join(", ")),
            }).collect();
            if !rails.is_empty() {
                steps.push(format!("The pour roles of {} were confirmed for this flow only. To keep them, record the rails with their voltages with set_design_intent (section 'rails').", rails.join(", ")));
            }
            steps
        }
        FlowState::Abandoned => vec!["The flow was abandoned; nothing on the board was undone.".to_string()],
    };
    body["next_steps"] = json!(next_steps);
    body
}

/// A report: `flow.json`, and the running leg's report with `wait`. A leg that ended
/// without recording its end (the server stopped, a panic) stops the flow here.
async fn report(jobs: &Jobs, state: &Path, run_id: &str, wait: Duration) -> CallToolResult {
    let mut flow = match read_flow(state, run_id) {
        Ok(flow) => flow,
        Err(why) => return CallToolResult::error(format!("{why}. Nothing was read.")),
    };
    if flow.state != FlowState::Running {
        return CallToolResult::json(&view(state, &flow, None));
    }
    let Some(leg) = flow.legs.last().cloned() else {
        return CallToolResult::json(&view(state, &flow, None));
    };
    match jobs.report(state, &FLOW_LEGS, &leg, wait).await {
        Report::Running {
            record, elapsed_s, ..
        } => CallToolResult::json(&view(state, &flow, Some((&record.progress, elapsed_s)))),
        Report::Done { .. } => match read_flow(state, run_id) {
            Ok(flow) => CallToolResult::json(&view(state, &flow, None)),
            Err(why) => CallToolResult::error(why),
        },
        Report::Failed { record } => {
            leg_ended(
                state,
                &mut flow,
                record.error.as_deref().unwrap_or("the leg failed"),
            );
            CallToolResult::json(&view(state, &flow, None))
        }
        Report::Interrupted { .. } => {
            leg_ended(state, &mut flow, crate::jobs::STOPPED);
            CallToolResult::json(&view(state, &flow, None))
        }
        Report::Unknown { why } => {
            CallToolResult::error(format!("The flow's leg {leg} cannot be read: {why}"))
        }
    }
}

/// A leg that ended without recording its end: the flow stops, `uncertain` when a
/// writing step's intent was recorded and its end was not.
fn leg_ended(state: &Path, flow: &mut FlowRecord, why: &str) {
    let Ok(fresh) = read_flow(state, &flow.flow_id) else {
        return;
    };
    *flow = fresh;
    if flow.state != FlowState::Running {
        return;
    }
    let step = flow.at.unwrap_or(Step::Preflight);
    if flow.finished(step) {
        // The leg ended between two steps: the last one is done as recorded (m2).
        flow.state = FlowState::Stopped;
        flow.stop_reason = Some(format!("failed: the leg ended between steps ({why})"));
        let _ = save_flow(state, flow);
        return;
    }
    let calling = flow
        .record(step)
        .is_some_and(|r| r.state == StepState::Calling);
    let kind = if calling && step.writes() {
        StopKind::Uncertain
    } else {
        StopKind::Failed
    };
    stop_flow(
        flow,
        step,
        Stop::new(
            kind,
            format!("the leg ended before it recorded its end ({why})"),
        ),
    );
    let _ = save_flow(state, flow);
}

/// The last leg has ended (waiting up to [`HANDOFF_WAIT`]), or why not.
async fn last_leg_ended(
    jobs: &Jobs,
    state: &Path,
    flow: &mut FlowRecord,
) -> Result<(), CallToolResult> {
    let Some(leg) = flow.legs.last().cloned() else {
        return Ok(());
    };
    match jobs.report(state, &FLOW_LEGS, &leg, HANDOFF_WAIT).await {
        Report::Running { .. } => Err(CallToolResult::error(format!(
            "The flow's leg {leg} is still running: call again in a few seconds (a report with wait_s waits for it). Nothing was changed."
        ))),
        Report::Failed { record } => {
            leg_ended(state, flow, record.error.as_deref().unwrap_or("the leg failed"));
            Ok(())
        }
        Report::Interrupted { .. } => {
            leg_ended(state, flow, crate::jobs::STOPPED);
            Ok(())
        }
        _ => Ok(()),
    }
}

/// An answer, in order: the flow awaits one and its last leg has ended; the revision is
/// the current one; `merged`, then `refused_combination`; when the roles changed at gate
/// A, P4 runs again in the next leg (and goes back to gate A if its proposals changed);
/// then the next leg.
async fn answer(
    jobs: &Jobs,
    state: &Path,
    run_id: &str,
    given: Approvals,
    revision: &str,
    ctx: Arc<ToolContext>,
) -> anyhow::Result<CallToolResult> {
    let mut flow = match read_flow(state, run_id) {
        Ok(flow) => flow,
        Err(why) => {
            return Ok(CallToolResult::error(format!(
                "{why}. Nothing was changed."
            )))
        }
    };
    if let Err(refusal) = last_leg_ended(jobs, state, &mut flow).await {
        return Ok(refusal);
    }
    let (FlowState::AwaitingAnswer, Some(gate)) = (flow.state, flow.gate) else {
        return Ok(CallToolResult::error(format!(
            "The flow is {}, not awaiting an answer. Nothing was changed.",
            flow.state.name()
        )));
    };
    if let Err(refusal) = apply_answer(&mut flow, gate, &given, revision) {
        return Ok(refusal);
    }
    let board = PathBuf::from(&flow.board);
    let io: Arc<dyn FlowIo> = Arc::new(LiveIo {
        ctx: Arc::clone(&ctx),
        board: board.clone(),
        project: PathBuf::from(&flow.project),
    });
    Ok(CallToolResult::json(&start_leg(
        jobs, state, flow, ctx, io,
    )?))
}

/// What an answer does to the record, before the next leg, or its refusal (nothing is
/// changed by a refusal):
/// 1. `expected_plan_revision` is the gate's;
/// 2. the approvals merged (`merged`), or, at gate A after a refused combination, the
///    answer's keys replacing the old ones (`overridden`, M4);
/// 3. `refused_combination` on the result;
/// 4. P4 again when the roles changed at gate A, or when the user keeps the current
///    placement and P4 planned on the winner (m5): gate A comes back when its zones differ;
/// 5. otherwise every key the gate needs must be answered (gate R included, m3).
fn apply_answer(
    flow: &mut FlowRecord,
    gate: Gate,
    given: &Approvals,
    revision: &str,
) -> Result<(), CallToolResult> {
    if flow.plan_revision.as_deref() != Some(revision) {
        return Err(CallToolResult::error(format!(
            "expected_plan_revision is not the current one: the gate's plan_revision is {}. Show the user the current plan (a report gives it) and answer that. Nothing was changed.",
            flow.plan_revision.as_deref().unwrap_or("none")
        )));
    }
    let refused_before = gate == Gate::Plan
        && flow
            .gate_body
            .as_ref()
            .is_some_and(|body| body.get("refused").is_some());
    let merged = if refused_before {
        flow.approvals.overridden(given)
    } else {
        flow.approvals
            .merged(given, gate)
            .map_err(|why| invalid_arg("answer", &why))?
    };
    let rails = flow.rail_roles();
    if let Some(reason) =
        refused_combination(&merged, &flow.facts, &flow.steps_asked, &rails, &flow.nets)
    {
        return Err(CallToolResult::error(format!(
            "{reason} Nothing was changed."
        )));
    }
    let roles_changed = gate == Gate::Plan && flow.approvals.roles_changed(given);
    let kept_after_winner = gate == Gate::Plan
        && merged.placement == Some(PlacementAnswer::KeepCurrent)
        && flow.facts.p4_on_winner;
    let replan = roles_changed || kept_after_winner;
    if gate == Gate::Roles {
        if let Some(need) = needs_roles(&flow.facts.named_only, &merged) {
            return Err(invalid_arg(
                "answer",
                &format!("the gate still needs {}", need.key),
            ));
        }
    }
    if !replan {
        let missing = match gate {
            Gate::Roles => Vec::new(),
            Gate::Plan => needs_at_plan(&flow.facts, &flow.steps_asked, &merged),
            Gate::Routing => needs_routing(&merged, flow.route.and_then(|r| r.all_gates_pass))
                .into_iter()
                .collect(),
        };
        if !missing.is_empty() {
            let keys: Vec<&str> = missing.iter().map(|need| need.key).collect();
            return Err(invalid_arg(
                "answer",
                &format!("the gate still needs {}", keys.join(", ")),
            ));
        }
    }
    flow.approvals = merged;
    if roles_changed {
        // The placement stays the one judged with the earlier pours, and the record says so.
        if let Some(mut placed) = flow.record(Step::Placement).cloned() {
            placed
                .not_checked
                .push("placement judged with the earlier pour roles".to_string());
            flow.put(placed);
        }
    }
    if replan {
        flow.records
            .retain(|record| record.step != Step::PoursAfter);
    }
    if gate == Gate::Roles {
        flow.facts.named_only.clear();
    }
    Ok(())
}

/// A resume. The flow must be stopped and no leg running. For a writing step:
/// `already_done`, the saved file's SHA-256, then `may_resume`. For a reading or scratch
/// step: the plan's steps again in a new attempt folder (P1 to P4), or the route's (R1
/// to R3) in a new route folder.
async fn resume(
    jobs: &Jobs,
    state: &Path,
    run_id: &str,
    ctx: Arc<ToolContext>,
) -> anyhow::Result<CallToolResult> {
    let mut flow = match read_flow(state, run_id) {
        Ok(flow) => flow,
        Err(why) => {
            return Ok(CallToolResult::error(format!(
                "{why}. Nothing was changed."
            )))
        }
    };
    if let Err(refusal) = last_leg_ended(jobs, state, &mut flow).await {
        return Ok(refusal);
    }
    if flow.state != FlowState::Stopped {
        return Ok(CallToolResult::error(format!(
            "The flow is {}, not stopped: only a stopped flow resumes. Nothing was changed.",
            flow.state.name()
        )));
    }
    let board = PathBuf::from(&flow.board);
    let io: Arc<dyn FlowIo> = Arc::new(LiveIo {
        ctx: Arc::clone(&ctx),
        board: board.clone(),
        project: PathBuf::from(&flow.project),
    });
    match resume_step(state, &mut flow, io.as_ref()).await? {
        Ok(()) => {}
        Err(reason) => {
            return Ok(CallToolResult::error(format!(
                "{reason} Nothing was changed."
            )))
        }
    }
    save_flow(state, &flow)?;
    Ok(CallToolResult::json(&start_leg(
        jobs, state, flow, ctx, io,
    )?))
}

/// What a resume does to the record before the next leg, or why it is refused.
async fn resume_step(
    state: &Path,
    flow: &mut FlowRecord,
    io: &dyn FlowIo,
) -> anyhow::Result<Result<(), String>> {
    let Some(step) = flow.at else {
        return Ok(Err("The flow stopped before any step.".to_string()));
    };
    if flow
        .stop_reason
        .as_deref()
        .is_some_and(|why| why.starts_with("board_changed"))
        && !step.writes()
    {
        return Ok(Err(format!(
            "The flow stopped because the board changed ({}). Restore the board or abandon the flow.",
            flow.stop_reason.clone().unwrap_or_default()
        )));
    }
    if !step.writes() {
        if step <= Step::PoursAfter {
            flow.records.retain(|record| record.step == Step::Preflight);
            flow.attempt += 1;
        } else if (Step::Export..=Step::ImportCheck).contains(&step) {
            flow.records.retain(|record| record.step < Step::Export);
            flow.route_attempt += 1;
            flow.route = None;
        } else {
            flow.records.retain(|record| record.step != step);
        }
        flow.state = FlowState::Running;
        return Ok(Ok(()));
    }
    let Some(record) = flow.record(step).cloned() else {
        return Ok(Err(format!("{} has no record to resume from.", step.id())));
    };
    if record.state.finished() {
        // Stopped between two steps (m2): the next step runs as usual.
        flow.state = FlowState::Running;
        flow.stop_reason = None;
        return Ok(Ok(()));
    }
    if record.before.is_none() {
        // The step stopped before its intent was recorded, so no call was sent (M1). It
        // runs again, and its own checks decide; a board that changed is refused here.
        if flow
            .stop_reason
            .as_deref()
            .is_some_and(|why| why.starts_with("board_changed"))
        {
            return Ok(Err(format!(
                "The flow stopped because the board changed ({}). Restore the board or abandon the flow.",
                flow.stop_reason.clone().unwrap_or_default()
            )));
        }
        flow.records.retain(|r| r.step != step);
        flow.state = FlowState::Running;
        flow.stop_reason = None;
        return Ok(Ok(()));
    }
    // An unsaved step's work is on the board by its own record: only the save again.
    let done = record.state != StepState::Unsaved && already_done(step, state, flow, io).await?;
    let found = match with_tries(io, || io.saved_text()).await {
        Ok(text) => canonical(text).await,
        Err(why) => Err(why),
    };
    let found = match found {
        Ok(found) => found,
        Err(why) => return Ok(Err(format!("The saved board cannot be read ({why})."))),
    };
    match may_resume(&record, done, &found) {
        Ok(Resume::RunAgain) => {
            // It runs again from the board as found: its start, or the end of its last
            // stitching round (m1).
            let mut again = StepRecord::new(step, StepState::NotRun);
            again.before = Some(found.clone());
            flow.put(again);
        }
        Ok(Resume::RecordDone) => match live_equals_saved(io).await {
            Ok((_, saved)) => {
                let mut done = record.clone();
                done.state = StepState::Done;
                done.after = Some(saved);
                done.why = None;
                done.not_checked
                    .push("recorded done on resume: its work was found on the board".to_string());
                flow.put(done);
            }
            Err(_) => match verified_save(io).await {
                Ok(saved) => {
                    let mut done = record.clone();
                    done.state = StepState::Done;
                    done.after = Some(saved);
                    done.why = None;
                    flow.put(done);
                }
                Err(why) => {
                    return Ok(Err(format!(
                        "The step's work is on the board, and it could not be saved ({why})."
                    )))
                }
            },
        },
        Ok(Resume::SaveAgain) => match verified_save(io).await {
            Ok(saved) => {
                let mut done = record.clone();
                done.state = StepState::Done;
                done.after = Some(saved);
                done.why = None;
                flow.put(done);
            }
            Err(why) => return Ok(Err(format!("The save failed again ({why})."))),
        },
        Err(reason) => return Ok(Err(reason)),
    }
    flow.state = FlowState::Running;
    flow.board_state = BoardState::AsRecorded;
    flow.stop_reason = None;
    Ok(Ok(()))
}

/// An abandon. Refused while a leg runs (the job module has no per-run stop). Otherwise
/// `flow.json` says `abandoned`, the marker is cleared, and the result names the start
/// checkpoint with `restore_checkpoint`'s preview and apply. Nothing is undone.
async fn abandon(jobs: &Jobs, state: &Path, run_id: &str) -> anyhow::Result<CallToolResult> {
    let mut flow = match read_flow(state, run_id) {
        Ok(flow) => flow,
        Err(why) => {
            return Ok(CallToolResult::error(format!(
                "{why}. Nothing was changed."
            )))
        }
    };
    if let Err(refusal) = last_leg_ended(jobs, state, &mut flow).await {
        return Ok(refusal);
    }
    if matches!(flow.state, FlowState::Done | FlowState::Abandoned) {
        return Ok(CallToolResult::error(format!(
            "The flow is already {}.",
            flow.state.name()
        )));
    }
    flow.state = FlowState::Abandoned;
    save_flow(state, &flow)?;
    clear_open(state, Path::new(&flow.board), &flow.flow_id);
    let mut body = view(state, &flow, None);
    body["way_back"] = way_back(&flow, flow.start_checkpoint.as_deref());
    Ok(CallToolResult::json(&body))
}

#[cfg(test)]
#[path = "full_flow_tool_tests.rs"]
mod tests;
