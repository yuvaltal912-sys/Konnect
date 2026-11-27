//! Row BS, board safety: three things a long or bulk call does about the real board.
//!
//! 1. **A plain "board not open".** When KiCad answers a board-targeted call without
//!    naming a board, every consumer of that answer reads the same sentence first:
//!    "the board is not open in KiCad's PCB editor: open <board> and call again".
//!    [`unserved_message`] builds it at the one place that answer is classified
//!    (`with_bound_board_ipc_classified`), so no tool words it on its own.
//! 2. **A pre-flight before a long route.** `route_specctra_dsn` can search for many
//!    minutes, and the session it writes is applied to the live board
//!    (`apply_specctra_ses`, live only). [`route_preflight`] checks, before anything
//!    starts, that the board the DSN was exported from is open in KiCad's PCB editor,
//!    and refuses at once with the sentence above when it is not.
//!    `require_live_board: false` routes the saved board without KiCad on purpose.
//!    `plan_convergent_placement` needs no new check: it reads the live board before
//!    its search starts, and the sentence above now reaches its refusal.
//! 3. **A checkpoint before a bulk edit.** [`guard`] wraps the tools in
//!    [`BULK_LIVE_EDITS`] where the registry builds them: before the tool runs, the
//!    project's saved files are checkpointed (an existing checkpoint of exactly
//!    these files is reused), and the result carries `live_project`, which says that
//!    the call edited the real project, which checkpoint restores it, and what that
//!    checkpoint does not hold (unsaved changes in KiCad).

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
/// `apply_specctra_ses` lays a whole routed session onto the live board.
pub(crate) const BULK_LIVE_EDITS: &[&str] = &["set_component_placements", "apply_specctra_ses"];

/// The label of a checkpoint [`guard`] takes.
pub(crate) fn checkpoint_label(tool: &str) -> String {
    format!("automatic: before {tool}")
}

// ─── 1. The plain sentence ───────────────────────────────────────────────────

/// "the board is not open in KiCad's PCB editor: open <board> and call again".
pub(crate) fn not_open_sentence(board: &Path) -> String {
    format!(
        "the board is not open in KiCad's PCB editor: open {} and call again",
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

/// A refusal made before anything ran: the plain sentence (or `detail` when KiCad
/// said something more specific), what was not done, and KiCad's answer.
fn not_open_refusal(board: &Path, detail: &str, consequence: &str) -> CallToolResult {
    let sentence = not_open_sentence(board);
    let lead = if detail.starts_with(&sentence) || detail.starts_with("KiCad cannot say") {
        detail.to_string()
    } else {
        format!("{sentence} ({detail})")
    };
    let mut lead_chars = lead.chars();
    let lead = match lead_chars.next() {
        Some(first) => first.to_uppercase().chain(lead_chars).collect::<String>(),
        None => lead,
    };
    CallToolResult::error_kind(
        ToolErrorKind::EditorUnavailable {
            editor: "pcb".to_string(),
            reason: detail.to_string(),
        },
        format!("{lead}. {consequence}"),
    )
}

// ─── 2. The route pre-flight ─────────────────────────────────────────────────

/// The board a DSN was exported from, as its reverse manifest names it. `None`
/// when there is no readable manifest: the route then refuses with its own reason.
fn manifest_board(dsn: &Path) -> Option<PathBuf> {
    let mut name = dsn.as_os_str().to_os_string();
    name.push(".konnect.json");
    let text = std::fs::read_to_string(PathBuf::from(name)).ok()?;
    let manifest: Value = serde_json::from_str(&text).ok()?;
    manifest["board_path"]
        .as_str()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

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
    const CONSEQUENCE: &str = "Nothing was routed: route_specctra_dsn checks this before its \
        search, because the session it writes is applied to the live board (apply_specctra_ses) \
        and the search can run for many minutes. To route the saved board without KiCad on \
        purpose, pass require_live_board: false";
    Ok(
        match live_board::observe(ctx, &board, |_, _| Ok(())).await? {
            LiveBoard::Answered(()) => None,
            LiveBoard::Unresolved(error) => Some(crate::tools::ipc_target_error_result(&error)),
            LiveBoard::Unserved { message, .. } => {
                Some(not_open_refusal(&board, &message, CONSEQUENCE))
            }
            LiveBoard::NotOpen { message, .. } => Some(not_open_refusal(
                &board,
                &format!("KiCad is running and {message}"),
                CONSEQUENCE,
            )),
            LiveBoard::NeverReached(message) => Some(not_open_refusal(
                &board,
                &format!("KiCad's IPC API cannot be reached: {message}"),
                CONSEQUENCE,
            )),
            LiveBoard::LostAfterObservation { situation, .. } => {
                Some(not_open_refusal(&board, situation, CONSEQUENCE))
            }
            LiveBoard::Rejected(message)
            | LiveBoard::Uncertain(message)
            | LiveBoard::Recovered(message) => Some(not_open_refusal(
                &board,
                &format!("KiCad did not confirm that it holds the board: {message}"),
                CONSEQUENCE,
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

/// What Konnect saw of the board just before the bulk edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Liveness {
    /// KiCad named this exact board as open.
    open: bool,
    /// The live board differs from its saved file (by the canonical tree, the
    /// identity the route judge uses); `None` when that could not be read.
    unsaved: Option<bool>,
}

async fn liveness(ctx: &ToolContext, board: &Path) -> anyhow::Result<Liveness> {
    let saved = std::fs::read_to_string(board).ok();
    Ok(
        match live_board::observe(ctx, board, |client, document| {
            Ok(client.save_document_to_string_in(document).ok())
        })
        .await?
        {
            LiveBoard::Answered(live) => Liveness {
                open: true,
                unsaved: match (live, saved) {
                    (Some(live), Some(saved)) => match (
                        crate::board_identity::canonical_sha256(&live),
                        crate::board_identity::canonical_sha256(&saved),
                    ) {
                        (Ok(live), Ok(saved)) => Some(live != saved),
                        _ => None,
                    },
                    _ => None,
                },
            },
            _ => Liveness {
                open: false,
                unsaved: None,
            },
        },
    )
}

/// The checkpoint, then the tool, then `live_project` on its result.
async fn checkpointed(
    tool: &'static str,
    access: BoardAccess,
    args: &Value,
    ctx: Arc<ToolContext>,
    inner: ToolHandlerFn,
) -> anyhow::Result<CallToolResult> {
    let Some(board) = args["board"].as_str().map(PathBuf::from) else {
        return inner(args, ctx).await;
    };
    let live = liveness(&ctx, &board).await?;
    // A live-only tool refuses a board KiCad does not hold, and changes nothing;
    // a checkpoint would only add a copy.
    if access == BoardAccess::LiveOnly && !live.open {
        return inner(args, ctx).await;
    }
    let project = board.with_extension("kicad_pro");
    let checkpoint = if project.is_file() {
        let (task_project, label) = (project.clone(), checkpoint_label(tool));
        match tokio::task::spawn_blocking(move || ensure_checkpoint(&task_project, &label)).await? {
            Ok(checkpoint) => Ok(checkpoint),
            Err(reason) => {
                return Ok(CallToolResult::error_kind(
                    ToolErrorKind::PlanBlocked {
                        operation: tool.to_string(),
                        reasons: vec![reason.clone()],
                    },
                    format!(
                        "Nothing was changed: {tool} edits the real project, and the automatic \
                         checkpoint that would restore it could not be made ({reason}). Fix \
                         that, or take one with create_checkpoint, and call again."
                    ),
                ))
            }
        }
    } else {
        Err(format!(
            "no project file '{}' beside the board, so no checkpoint was made",
            project.display()
        ))
    };
    let result = inner(args, ctx).await?;
    let note = live_project_note(&board, &project, live, &checkpoint, result.is_error);
    Ok(with_note(result, note))
}

/// The `live_project` field: what was edited, the checkpoint, how to go back.
fn live_project_note(
    board: &Path,
    project: &Path,
    live: Liveness,
    checkpoint: &Result<EnsuredCheckpoint, String>,
    call_failed: bool,
) -> Value {
    let mut summary = Vec::new();
    if call_failed {
        summary.push("The call reported an error, so check whether the board changed.".to_string());
    }
    summary.push(if live.open {
        format!(
            "This call edits the live board open in KiCad ({}): the real project, not a copy.",
            board.display()
        )
    } else {
        format!(
            "This call edits the saved board file of the real project ({}), not a copy.",
            board.display()
        )
    });
    let restore = match checkpoint {
        Ok(checkpoint) => {
            summary.push(format!(
                "Checkpoint {} ({}) holds the project's saved files as of {}.",
                checkpoint.id,
                if checkpoint.made_now {
                    "made just before this call"
                } else {
                    "made earlier; the saved files have not changed since"
                },
                checkpoint.created_utc
            ));
            summary.push(if live.open {
                format!(
                    "To go back: Edit > Undo in KiCad reverts this call in one step; or close the \
                     board in KiCad and call restore_checkpoint with project '{}' and \
                     checkpoint_id '{}' (it previews first).",
                    project.display(),
                    checkpoint.id
                )
            } else {
                format!(
                    "To go back, call restore_checkpoint with project '{}' and checkpoint_id \
                     '{}' (it previews first).",
                    project.display(),
                    checkpoint.id
                )
            });
            json!({
                "tool": "restore_checkpoint",
                "arguments": { "project": project.display().to_string(), "checkpoint_id": checkpoint.id },
                "close_the_board_in_kicad_first": live.open,
            })
        }
        Err(reason) => {
            summary.push(format!(
                "No checkpoint protects this call: {reason}.{}",
                if live.open {
                    " Edit > Undo in KiCad reverts it in one step."
                } else {
                    ""
                }
            ));
            Value::Null
        }
    };
    if live.open {
        match live.unsaved {
            Some(true) => summary.push(
                "The live board had changes that were not saved when this call started; the \
                 checkpoint does not hold them (save in KiCad first to make them restorable)."
                    .to_string(),
            ),
            Some(false) => {}
            None => summary.push(
                "Konnect could not tell whether the live board had unsaved changes; if it had, \
                 the checkpoint does not hold them."
                    .to_string(),
            ),
        }
    }
    json!({
        "summary": summary.join(" "),
        "board": board.display().to_string(),
        "project": project.display().to_string(),
        "board_open_in_kicad": live.open,
        "unsaved_changes_before_call": live.unsaved,
        "checkpoint": match checkpoint {
            Ok(checkpoint) => json!({
                "checkpoint_id": checkpoint.id,
                "created_utc": checkpoint.created_utc,
                "made_by_this_call": checkpoint.made_now,
                "location": checkpoint.location.display().to_string(),
                "locks_present": checkpoint.locks_present,
            }),
            Err(_) => Value::Null,
        },
        "checkpoint_skipped_reason": checkpoint.as_ref().err(),
        "restore": restore,
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
