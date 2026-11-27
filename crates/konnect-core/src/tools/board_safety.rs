//! Row BS, board safety: three things a long or bulk call does about the real board.
//!
//! 1. **A plain "board not open".** When KiCad answers a board-targeted call without
//!    naming a board, every consumer of that answer reads the same sentence first:
//!    "the board is not open in KiCad's PCB editor: open <board> and call again".
//!    [`unserved_message`] builds it at the one place that answer is classified
//!    (`with_bound_board_ipc_classified`), so no tool words it on its own. KiCad
//!    down reads as plainly ([`unreachable_sentence`]), without claiming the board
//!    is closed.
//! 2. **A pre-flight before a long route.** `route_specctra_dsn` can search for many
//!    minutes, and the session it writes is applied to the live board
//!    (`apply_specctra_ses`, live only). [`route_preflight`] checks, before anything
//!    starts, that the board the DSN was exported from is open in KiCad's PCB editor,
//!    and refuses at once when it is not. `require_live_board: false` routes the
//!    saved board without KiCad on purpose. `plan_convergent_placement` needs no new
//!    check: it reads the live board before its search starts, and the sentence
//!    above now reaches its refusal.
//! 3. **A checkpoint before a bulk edit.** [`guard`] wraps the tools in
//!    [`BULK_LIVE_EDITS`] where the registry builds them: before the tool runs, the
//!    project's saved files are checkpointed (an existing checkpoint of exactly these
//!    files, whose copies still read back, is reused), and the result carries
//!    `live_project`: that the call edited the real project, how to go back, and what
//!    the checkpoint does not hold (changes in KiCad that were not saved). No
//!    checkpoint, no edit. A preview (`delete_all_routing` without `dry_run: false`)
//!    changes nothing and gets neither.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::{CallToolResult, ToolContent};
use crate::tools::checkpoints::{ensure_checkpoint, EnsuredCheckpoint};
use crate::tools::live_board::{self, LiveBoard};
use crate::tools::{BoardAccess, BoardEditorAbsence, ToolContext, ToolDef, ToolHandlerFn};

/// The tools that change a board in bulk, and so get [`guard`]'s checkpoint.
/// `set_component_placements` moves many footprints in one call;
/// `apply_specctra_ses` lays a whole routed session onto the live board;
/// `delete_all_routing` removes every track, arc and via (its apply only, see
/// [`previews_only`]).
pub(crate) const BULK_LIVE_EDITS: &[&str] = &[
    "set_component_placements",
    "apply_specctra_ses",
    "delete_all_routing",
];

/// Whether this call of `tool` only previews, and so changes nothing and gets
/// no checkpoint. `delete_all_routing` deletes only when `dry_run` is exactly
/// `false`: its handler defaults a missing `dry_run` to true and refuses one
/// that is not a boolean.
fn previews_only(tool: &str, args: &Value) -> bool {
    tool == "delete_all_routing" && args.get("dry_run") != Some(&Value::Bool(false))
}

/// The label of a checkpoint [`guard`] takes.
pub(crate) fn checkpoint_label(tool: &str) -> String {
    format!("automatic: before {tool}")
}

// ─── 1. The plain sentences ──────────────────────────────────────────────────

/// "the board is not open in KiCad's PCB editor: open <board> and call again".
pub(crate) fn not_open_sentence(board: &Path) -> String {
    format!(
        "the board is not open in KiCad's PCB editor: open {} and call again",
        board.display()
    )
}

/// KiCad down: as plain, but it does not claim the board is closed, because a
/// KiCad Konnect cannot reach may still hold it.
pub(crate) fn unreachable_sentence(board: &Path) -> String {
    format!(
        "KiCad cannot be reached, so Konnect cannot see {} open in its PCB editor: start KiCad, \
         open the board there and call again",
        board.display()
    )
}

/// The message of an answer that named no board, plain sentence first and KiCad's
/// own words (`raw`) after it. `AS_UNIMPLEMENTED` proves nothing about KiCad's
/// editors, so it does not claim the board is closed: it says Konnect cannot tell.
pub(crate) fn unserved_message(board: &Path, absence: BoardEditorAbsence, raw: &str) -> String {
    match absence {
        BoardEditorAbsence::NoPcbEditor => format!(
            "{} (KiCad answered that no PCB editor is open at this endpoint: {raw})",
            not_open_sentence(board)
        ),
        BoardEditorAbsence::OpenDocumentsUnimplemented => format!(
            "KiCad cannot say which boards it has open (this build does not implement the \
             command that lists them), so Konnect cannot confirm that {} is open: open it in \
             the PCB editor of a KiCad that implements it and call again (KiCad answered: {raw})",
            board.display()
        ),
    }
}

/// A refusal made before anything ran: `lead` (a sentence the caller formed, its
/// first letter raised here), then what was not done. `reason` is KiCad's answer.
fn refusal(lead: &str, reason: &str, consequence: &str) -> CallToolResult {
    let mut chars = lead.chars();
    let lead = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
        None => String::new(),
    };
    CallToolResult::error_kind(
        ToolErrorKind::EditorUnavailable {
            editor: "pcb".to_string(),
            reason: reason.to_string(),
        },
        format!("{lead}. {consequence}"),
    )
}

// ─── 2. The route pre-flight ─────────────────────────────────────────────────

/// The board a DSN was exported from, as its reverse manifest names it. `None`
/// when there is no readable manifest: the route then refuses with its own reason.
fn manifest_board(dsn: &Path) -> Option<PathBuf> {
    crate::specctra::manifest_beside(dsn)?["board_path"]
        .as_str()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// What a refused route did not do, and the way to route without KiCad.
const NOT_ROUTED: &str = "Nothing was routed: route_specctra_dsn checks this before its search, \
    because the session it writes is applied to the live board (apply_specctra_ses) and the \
    search can run for many minutes. To route the saved board without KiCad on purpose, pass \
    require_live_board: false";

/// As [`NOT_ROUTED`], without the way round: offered only where the saved file is
/// known to be what KiCad last held.
const NOT_ROUTED_LOST: &str = "Nothing was routed: route_specctra_dsn checks this before its \
    search, and the saved board may be older than what KiCad held, so it is not routed instead";

/// Before a route starts: the board the DSN came from must be open in KiCad's PCB
/// editor, unless `require_live_board` is `false`. `Some` is the refusal to return;
/// nothing has been made when it is returned.
pub(crate) async fn route_preflight(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<Option<CallToolResult>> {
    if args["require_live_board"] == json!(false) {
        return Ok(None);
    }
    let Some(dsn) = args["dsn_path"].as_str().map(PathBuf::from) else {
        return Ok(None);
    };
    let Some(board) = manifest_board(&dsn) else {
        return Ok(None);
    };
    Ok(
        match live_board::observe(ctx, &board, |_, _| Ok(())).await? {
            LiveBoard::Answered(()) => None,
            LiveBoard::Unresolved(error) => Some(crate::tools::ipc_target_error_result(&error)),
            // Already formed by `unserved_message`, for either absence.
            LiveBoard::Unserved { message, .. } => Some(refusal(&message, &message, NOT_ROUTED)),
            LiveBoard::NotOpen { message, .. } => Some(refusal(
                &format!(
                    "{} (KiCad is running and {message})",
                    not_open_sentence(&board)
                ),
                &message,
                NOT_ROUTED,
            )),
            LiveBoard::NeverReached(message) => Some(refusal(
                &format!("{} ({message})", unreachable_sentence(&board)),
                &message,
                NOT_ROUTED,
            )),
            LiveBoard::LostAfterObservation { situation, .. } => Some(refusal(
                &format!(
                    "{situation} Open {} in KiCad's PCB editor again and call again",
                    board.display()
                ),
                situation,
                NOT_ROUTED_LOST,
            )),
            LiveBoard::Rejected(message)
            | LiveBoard::Uncertain(message)
            | LiveBoard::Recovered(message) => Some(refusal(
                &format!(
                    "KiCad did not confirm that it holds {} ({message}): check that it is open \
                     in KiCad's PCB editor and call again",
                    board.display()
                ),
                &message,
                NOT_ROUTED,
            )),
        },
    )
}

// ─── 3. The checkpoint before a bulk edit ────────────────────────────────────

/// `tool` with its handler wrapped by [`checkpointed`] when it is a bulk edit;
/// any other tool is returned as it is.
pub(crate) fn guard(mut tool: ToolDef) -> ToolDef {
    if !BULK_LIVE_EDITS.contains(&tool.name) {
        return tool;
    }
    let inner = tool.handler.clone();
    let name = tool.name;
    let access = tool.board_access;
    tool.handler = Arc::new(move |args, ctx| {
        let inner = inner.clone();
        let args = args.clone();
        Box::pin(async move { checkpointed(name, access, &args, ctx, inner).await })
    });
    tool
}

/// Whether KiCad held the board just before the bulk edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    /// KiCad named this exact board as open.
    Open,
    /// KiCad answered, and the board is not among its open boards (`NotOpen`), or
    /// no PCB editor is open at the endpoint (`AS_UNHANDLED`).
    Closed,
    /// Konnect could not see: KiCad unreachable, a build that cannot list its
    /// boards, or a board it held and no longer reaches. A live-only tool refuses
    /// on its own here too.
    NotSeen,
    /// KiCad answered, but not cleanly (a rejected question, an ambiguous list,
    /// an unknown outcome): it may hold the board.
    Unknown,
}

impl Held {
    /// `board_open_in_kicad`: known only for `Open` and `Closed`.
    fn open(self) -> Value {
        match self {
            Self::Open => json!(true),
            Self::Closed => json!(false),
            Self::NotSeen | Self::Unknown => Value::Null,
        }
    }
}

/// What Konnect saw of the board just before the bulk edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Liveness {
    held: Held,
    /// The live board differs from its saved file (by the canonical tree, the
    /// identity the route judge uses); `None` when that could not be read.
    unsaved: Option<bool>,
}

async fn liveness(ctx: &ToolContext, board: &Path) -> anyhow::Result<Liveness> {
    let observed = live_board::observe(ctx, board, |client, document| {
        Ok(client.save_document_to_string_in(document).ok())
    })
    .await?;
    let held = |held| Liveness {
        held,
        unsaved: None,
    };
    Ok(match observed {
        LiveBoard::Answered(live) => {
            let saved_path = board.to_path_buf();
            // Two whole-board parses: off the async runtime.
            let unsaved = tokio::task::spawn_blocking(move || {
                let saved = std::fs::read_to_string(saved_path).ok()?;
                let live = crate::board_identity::canonical_sha256(&live?).ok()?;
                let saved = crate::board_identity::canonical_sha256(&saved).ok()?;
                Some(live != saved)
            })
            .await?;
            Liveness {
                held: Held::Open,
                unsaved,
            }
        }
        LiveBoard::NotOpen { .. }
        | LiveBoard::Unserved {
            absence: BoardEditorAbsence::NoPcbEditor,
            ..
        } => held(Held::Closed),
        LiveBoard::NeverReached(_)
        | LiveBoard::Unserved {
            absence: BoardEditorAbsence::OpenDocumentsUnimplemented,
            ..
        }
        | LiveBoard::LostAfterObservation { .. } => held(Held::NotSeen),
        LiveBoard::Unresolved(_)
        | LiveBoard::Rejected(_)
        | LiveBoard::Uncertain(_)
        | LiveBoard::Recovered(_) => held(Held::Unknown),
    })
}

/// How the wrapped call ended, as far as its result says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Succeeded,
    /// An error that says nothing was applied, or says nothing either way.
    Failed,
    /// An error whose kind says the board may have changed.
    MayHaveChanged,
}

impl Outcome {
    fn of(result: &CallToolResult) -> Self {
        if !result.is_error {
            return Self::Succeeded;
        }
        match crate::mcp::error::extract_error_kind(result).as_deref() {
            Some("mutation_outcome_uncertain" | "ipc_outcome_unknown") => Self::MayHaveChanged,
            _ => Self::Failed,
        }
    }
}

/// The checkpoint, then the tool, then `live_project` on its result.
async fn checkpointed(
    tool: &'static str,
    access: BoardAccess,
    args: &Value,
    ctx: Arc<ToolContext>,
    inner: ToolHandlerFn,
) -> anyhow::Result<CallToolResult> {
    // A preview changes nothing: no checkpoint, no note.
    if previews_only(tool, args) {
        return inner(args, ctx).await;
    }
    // No board path, or no such file: the tool refuses on its own terms (a
    // missing argument, a board not found), and changes nothing.
    let Some(board) = args["board"]
        .as_str()
        .map(PathBuf::from)
        .filter(|board| board.is_file())
    else {
        return inner(args, ctx).await;
    };
    let live = liveness(&ctx, &board).await?;
    // A live-only tool refuses a board KiCad does not hold or cannot be seen to
    // hold, and changes nothing; a checkpoint would only add a copy. A board
    // KiCad may hold still gets one.
    if access == BoardAccess::LiveOnly && matches!(live.held, Held::Closed | Held::NotSeen) {
        return inner(args, ctx).await;
    }
    let blocked = |reason: String, message: String| {
        CallToolResult::error_kind(
            ToolErrorKind::PlanBlocked {
                operation: tool.to_string(),
                reasons: vec![reason],
            },
            message,
        )
    };
    let project = board.with_extension("kicad_pro");
    if !project.is_file() {
        let reason = format!("no project file '{}' beside the board", project.display());
        return Ok(blocked(
            reason.clone(),
            format!(
                "Nothing was changed: {tool} edits the real project, and there is {reason}, so no \
                 checkpoint can protect the edit. Save the board in a KiCad project of the same \
                 name, then call again."
            ),
        ));
    }
    let (task_project, label) = (project.clone(), checkpoint_label(tool));
    let checkpoint = match tokio::task::spawn_blocking(move || {
        ensure_checkpoint(&task_project, &label)
    })
    .await?
    {
        Ok(checkpoint) => checkpoint,
        Err(reason) => {
            return Ok(blocked(
                reason.clone(),
                format!(
                    "Nothing was changed: {tool} edits the real project, and the automatic \
                     checkpoint that would restore it could not be made ({reason}). Fix that \
                     and call again; create_checkpoint would fail for the same reason."
                ),
            ))
        }
    };
    let result = inner(args, ctx).await?;
    let note = live_project_note(&board, &project, live, &checkpoint, Outcome::of(&result));
    Ok(with_note(result, note))
}

/// The `live_project` field: what was edited, the checkpoint, how to go back.
fn live_project_note(
    board: &Path,
    project: &Path,
    live: Liveness,
    checkpoint: &EnsuredCheckpoint,
    outcome: Outcome,
) -> Value {
    let restore_call = format!(
        "restore_checkpoint with project '{}' and checkpoint_id '{}' (it previews first)",
        project.display(),
        checkpoint.id
    );
    let held = format!(
        "Checkpoint {} ({}) holds the project's saved files as of {}",
        checkpoint.id,
        if checkpoint.made_now {
            "made just before this call"
        } else {
            "made earlier; the saved files have not changed since"
        },
        checkpoint.created_utc
    );
    let maybe_open = format!(
        "{held}. If the board is open in KiCad, Edit > Undo reverts this call; once the edit is \
         in the saved file, close the board in KiCad and call {restore_call}."
    );
    let mut summary = Vec::new();
    match (outcome, live.held) {
        // Nothing here knows whether the failed call changed anything, so no way
        // back is offered that could undo something else.
        (Outcome::Failed, _) => summary.push(format!(
            "The call reported an error, and no change to the board is known to have been made. \
             {held}, in case it did."
        )),
        (Outcome::MayHaveChanged, _) => summary.push(format!(
            "The call reported that the board may have changed: inspect it before anything else. \
             {held}, from before this call. If the board is open in KiCad and changed, Edit > \
             Undo reverts it; once a change is in the saved file, close the board in KiCad and \
             call {restore_call}."
        )),
        (Outcome::Succeeded, Held::Open) => {
            summary.push(format!(
                "This call edited the live board open in KiCad ({}): the real project, not a \
                 copy. Edit > Undo in KiCad reverts it in one step.",
                board.display()
            ));
            summary.push(format!(
                "{held}, from before this call; the edit reaches the saved file only when the \
                 board is saved. After a save, the way back is to close the board in KiCad and \
                 call {restore_call}."
            ));
        }
        (Outcome::Succeeded, Held::Closed) => {
            summary.push(format!(
                "This call edited the saved board file of the real project ({}), not a copy.",
                board.display()
            ));
            summary.push(format!("{held}. To go back, call {restore_call}."));
        }
        (Outcome::Succeeded, Held::NotSeen) => {
            summary.push(format!(
                "This call edited the real project ({}), not a copy; Konnect could not see \
                 whether KiCad held the board when it started.",
                board.display()
            ));
            summary.push(maybe_open);
        }
        (Outcome::Succeeded, Held::Unknown) => {
            summary.push(format!(
                "This call edited the real project ({}), not a copy; KiCad did not confirm \
                 whether it held the board when it started.",
                board.display()
            ));
            summary.push(maybe_open);
        }
    }
    if live.held == Held::Open {
        match live.unsaved {
            Some(true) => summary.push(
                "The live board differed from its saved file when this call started; that \
                 difference is not in the checkpoint. Save the board in KiCad before the next \
                 bulk edit to make it restorable."
                    .to_string(),
            ),
            Some(false) => {}
            None => summary.push(
                "Konnect could not tell whether the live board differed from its saved file; \
                 if it did, the difference is not in the checkpoint."
                    .to_string(),
            ),
        }
    }
    let after_save = match live.held {
        Held::Open => json!(true),
        Held::Closed => json!(false),
        Held::NotSeen | Held::Unknown => Value::Null,
    };
    json!({
        "summary": summary.join(" "),
        "board": board.display().to_string(),
        "project": project.display().to_string(),
        "board_open_in_kicad": live.held.open(),
        "unsaved_changes_before_call": live.unsaved,
        "checkpoint": {
            "checkpoint_id": checkpoint.id,
            "created_utc": checkpoint.created_utc,
            "made_by_this_call": checkpoint.made_now,
            "location": checkpoint.location.display().to_string(),
            "locks_present": checkpoint.locks_present,
        },
        "restore": {
            "tool": "restore_checkpoint",
            "arguments": { "project": project.display().to_string(), "checkpoint_id": checkpoint.id },
            "close_the_board_in_kicad_first": after_save,
            "only_after_the_edit_is_saved": after_save,
        },
    })
}

/// `result` with `live_project` added to its JSON body, or, when its first text is
/// not a JSON object, as a second text block of its own.
fn with_note(mut result: CallToolResult, note: Value) -> CallToolResult {
    if let Some(ToolContent::Text { text }) = result.content.first_mut() {
        if let Ok(Value::Object(mut body)) = serde_json::from_str::<Value>(text) {
            body.insert("live_project".to_string(), note);
            *text = Value::Object(body).to_string();
            return result;
        }
    }
    result.content.push(ToolContent::Text {
        text: json!({ "live_project": note }).to_string(),
    });
    result
}
