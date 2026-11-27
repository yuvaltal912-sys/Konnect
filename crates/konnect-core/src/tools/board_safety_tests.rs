//! Row BS, board safety: the plain "board not open", the route pre-flight and the
//! checkpoint before a bulk edit. Each guard has a test named for it here; the
//! negative controls in `docs/CREATION_HANDOFF.md` ("Row BS") break each guard and
//! name the test that fails.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use konnect_ipc::gen::kiapi;
use serde_json::{json, Value};

use super::board_safety::{self, not_open_sentence, unserved_message, BULK_LIVE_EDITS};
use super::pcb_board::board_mock::{ctx_talking_to, spawn_kicad_holding_board};
use super::{BoardAccess, BoardEditorAbsence, ToolContext, ToolDef};
use crate::mcp::protocol::{CallToolResult, ToolContent};
use crate::test_support::MockIpcServer;

const BOARD_TEXT: &str = "(kicad_pcb\n\t(version 20241229)\n\t(generator \"pcbnew\")\n)\n";
const EDITED_BOARD_TEXT: &str =
    "(kicad_pcb\n\t(version 20241229)\n\t(generator \"pcbnew\")\n\t(general\n\t\t(thickness 1.6)\n\t)\n)\n";

fn text_of(result: &CallToolResult) -> String {
    match &result.content[0] {
        ToolContent::Text { text } => text.clone(),
        other => panic!("not text: {other:?}"),
    }
}

/// The message of a result: a structured error's `message`, else its first text.
fn message_of(result: &CallToolResult) -> String {
    let text = text_of(result);
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|body| body["message"].as_str().map(str::to_string))
        .unwrap_or(text)
}

/// Whether `message` carries the plain sentence for `board`, at its start
/// (capitalised) or inside it.
fn says_not_open(message: &str, board: &Path) -> bool {
    message
        .to_lowercase()
        .contains(&not_open_sentence(board).to_lowercase())
}

/// Whether `message` carries the KiCad-down sentence for `board`, and does not
/// claim the board is closed.
fn says_unreachable(message: &str, board: &Path) -> bool {
    message
        .to_lowercase()
        .contains(&board_safety::unreachable_sentence(board).to_lowercase())
        && !says_not_open(message, board)
}

fn body_of(result: &CallToolResult) -> Value {
    serde_json::from_str(&text_of(result)).expect("a JSON body")
}

/// A project folder holding `amp.kicad_pro` and `amp.kicad_pcb`.
fn project(dir: &Path) -> (PathBuf, PathBuf) {
    let pro = dir.join("amp.kicad_pro");
    let board = dir.join("amp.kicad_pcb");
    std::fs::write(&pro, "{}\n").unwrap();
    std::fs::write(&board, BOARD_TEXT).unwrap();
    (pro, board.canonicalize().unwrap())
}

/// A KiCad whose every answer is `status`: the project manager alone answers
/// `AS_UNHANDLED`, a build without the open-document command `AS_UNIMPLEMENTED`.
fn spawn_kicad_answering(status: kiapi::common::ApiStatusCode) -> MockIpcServer {
    MockIpcServer::spawn("board-safety-status", move |_| kiapi::common::ApiResponse {
        status: Some(kiapi::common::ApiResponseStatus {
            status: status as i32,
            error_message: format!("the double answers {}", status.as_str_name()),
        }),
        header: None,
        message: None,
    })
}

/// A KiCad holding `board` whose live text is `live`.
fn spawn_kicad_holding(board: &Path, live: &'static str) -> MockIpcServer {
    spawn_kicad_holding_board(board, move |command| {
        command.type_url.ends_with("SaveDocumentToString").then(|| {
            konnect_ipc::builders::pack_any(
                &kiapi::common::commands::SavedDocumentResponse {
                    document: None,
                    contents: live.to_string(),
                },
                "kiapi.common.commands.SavedDocumentResponse",
            )
        })
    })
}

fn arc(ctx: ToolContext) -> Arc<ToolContext> {
    Arc::new(ctx)
}

// ─── 1. The plain sentence ───────────────────────────────────────────────────

#[test]
fn an_endpoint_with_no_pcb_editor_reads_board_not_open_first() {
    let board = Path::new("/work/amp.kicad_pcb");
    let message = unserved_message(
        board,
        BoardEditorAbsence::NoPcbEditor,
        "KiCad IPC error: no handler (AS_UNHANDLED)",
    );
    assert!(
        message.starts_with(
            "the board is not open in KiCad's PCB editor: open /work/amp.kicad_pcb and call again"
        ),
        "{message}"
    );
    assert!(
        message.contains("AS_UNHANDLED"),
        "KiCad's answer is kept: {message}"
    );
}

#[test]
fn an_unimplemented_open_documents_command_does_not_claim_the_board_is_closed() {
    let message = unserved_message(
        Path::new("/work/amp.kicad_pcb"),
        BoardEditorAbsence::OpenDocumentsUnimplemented,
        "KiCad IPC error: no (AS_UNIMPLEMENTED)",
    );
    assert!(!message.contains("is not open"), "{message}");
    assert!(message.contains("cannot confirm"), "{message}");
}

/// The seam: every helper built on `with_bound_board_ipc_classified` hands its
/// callers the plain sentence for an `AS_UNHANDLED` answer.
#[tokio::test]
async fn the_shared_classifier_words_as_unhandled_as_board_not_open() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let server = spawn_kicad_answering(kiapi::common::ApiStatusCode::AsUnhandled);
    let ctx = ctx_talking_to(server.address().to_string());
    let read = super::with_board_ipc_classified(&ctx, &board, |_, _| Ok(()))
        .await
        .unwrap();
    let Err(konnect_ipc::IpcFailure::Rejected(message)) = read else {
        panic!("an endpoint serving no board is a rejection")
    };
    assert!(message.starts_with(&not_open_sentence(&board)), "{message}");
}

/// A mutating tool's write gate, for the same answer: refused, file untouched,
/// and the refusal says the board is not open rather than "KiCad rejected".
#[tokio::test]
async fn a_write_refused_for_no_pcb_editor_says_the_board_is_not_open() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let server = spawn_kicad_answering(kiapi::common::ApiStatusCode::AsUnhandled);
    let ctx = ctx_talking_to(server.address().to_string());
    let write = super::pcb_board::attempt_ipc_write(&ctx, &board, "test write", |_| Ok(()))
        .await
        .unwrap();
    let super::pcb_board::BoardWrite::Refused(refusal) = write else {
        panic!("an endpoint serving no board refuses the write")
    };
    let message = message_of(&refusal);
    assert!(says_not_open(&message, &board), "{message}");
    assert!(!message.contains("KiCAD rejected"), "{message}");
    assert_eq!(std::fs::read_to_string(&board).unwrap(), BOARD_TEXT);
}

/// `board_source: live` with KiCad down reads as plainly, without claiming the
/// board is closed.
#[tokio::test]
async fn a_live_read_with_kicad_down_says_kicad_cannot_be_reached() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let ctx = ctx_talking_to(String::new());
    let read = super::board_source::read_board(
        &ctx,
        &board,
        super::board_source::BoardSource::Live,
        "layer list",
        |_, _| Ok(()),
    )
    .await
    .unwrap();
    let super::board_source::BoardRead::Refused(refusal) = read else {
        panic!("live required, KiCad down: refused")
    };
    let message = message_of(&refusal);
    assert!(says_unreachable(&message, &board), "{message}");
}

/// `plan_convergent_placement` reads the live board before its search, so its
/// refusal for a KiCad with no PCB editor now carries the plain sentence, and
/// nothing was planned.
#[tokio::test]
async fn the_placement_search_refuses_a_board_not_open_before_it_starts() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let server = spawn_kicad_answering(kiapi::common::ApiStatusCode::AsUnhandled);
    let ctx = arc(ctx_talking_to(server.address().to_string()));
    let tool = crate::router::registry::tools_for("placement")
        .unwrap()
        .into_iter()
        .find(|tool| tool.name == "plan_convergent_placement")
        .expect("registered");
    let started = std::time::Instant::now();
    let result = (tool.handler)(&json!({ "board": board }), ctx)
        .await
        .unwrap();
    assert!(result.is_error);
    let message = message_of(&result);
    assert!(says_not_open(&message, &board), "{message}");
    assert!(message.contains("Nothing was planned"), "{message}");
    assert!(started.elapsed() < std::time::Duration::from_secs(30));
}

// ─── 2. The route pre-flight ─────────────────────────────────────────────────

/// A DSN beside its reverse manifest naming `board`, and the output path.
fn staged_dsn(dir: &Path, board: &Path) -> (PathBuf, PathBuf) {
    let dsn = dir.join("amp.dsn");
    std::fs::write(&dsn, "(pcb amp)\n").unwrap();
    std::fs::write(
        dir.join("amp.dsn.konnect.json"),
        json!({ "board_path": board }).to_string(),
    )
    .unwrap();
    (dsn, dir.join("amp.ses"))
}

/// Arguments that, past the pre-flight, refuse at once and make nothing: the
/// Freerouting engine with a JAR that does not exist.
fn route_args(dsn: &Path, ses: &Path, extra: Value) -> Value {
    let mut args = json!({
        "dsn_path": dsn,
        "ses_output_path": ses,
        "engine": "freerouting",
        "jar_path": "/nonexistent/board-safety/freerouting.jar",
    });
    for (key, value) in extra.as_object().unwrap() {
        args[key] = value.clone();
    }
    args
}

const PREFLIGHT_MARK: &str = "Nothing was routed: route_specctra_dsn checks this before its search";

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn the_route_refuses_at_once_when_kicad_has_no_pcb_editor() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let (dsn, ses) = staged_dsn(dir.path(), &board);
    let before = names_in(dir.path());
    let server = spawn_kicad_answering(kiapi::common::ApiStatusCode::AsUnhandled);
    let ctx = arc(ctx_talking_to(server.address().to_string()));
    let result =
        super::integration::handle_route_specctra_dsn(&route_args(&dsn, &ses, json!({})), ctx)
            .await
            .unwrap();
    assert!(result.is_error);
    let body = body_of(&result);
    assert_eq!(body["error"]["kind"], "editor_unavailable", "{body}");
    let message = message_of(&result);
    let expected = format!(
        "The board is not open in KiCad's PCB editor: open {} and call again",
        board.display()
    );
    assert!(message.starts_with(&expected), "{message}");
    assert!(message.contains(PREFLIGHT_MARK), "{message}");
    assert_eq!(names_in(dir.path()), before, "nothing was made");
}

#[tokio::test]
async fn the_route_refuses_at_once_when_kicad_cannot_be_reached() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let (dsn, ses) = staged_dsn(dir.path(), &board);
    let ctx = arc(ctx_talking_to(String::new()));
    let result =
        super::integration::handle_route_specctra_dsn(&route_args(&dsn, &ses, json!({})), ctx)
            .await
            .unwrap();
    let message = message_of(&result);
    assert!(says_unreachable(&message, &board), "{message}");
    assert!(message.contains(PREFLIGHT_MARK), "{message}");
}

/// A build that cannot list its open documents: refused, without the claim
/// that the board is closed.
#[tokio::test]
async fn the_route_refuses_without_claiming_closed_when_kicad_cannot_list_boards() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let (dsn, ses) = staged_dsn(dir.path(), &board);
    let server = spawn_kicad_answering(kiapi::common::ApiStatusCode::AsUnimplemented);
    let ctx = arc(ctx_talking_to(server.address().to_string()));
    let result =
        super::integration::handle_route_specctra_dsn(&route_args(&dsn, &ses, json!({})), ctx)
            .await
            .unwrap();
    let message = message_of(&result);
    assert!(
        message.starts_with("KiCad cannot say which boards"),
        "{message}"
    );
    assert!(!message.to_lowercase().contains("is not open"), "{message}");
    assert!(message.contains(PREFLIGHT_MARK), "{message}");
}

/// KiCad refuses the open-document command outright: it may hold the board, so
/// the refusal says it did not confirm, not that the board is closed.
#[tokio::test]
async fn the_route_refuses_without_claiming_closed_when_kicad_rejects_the_question() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let (dsn, ses) = staged_dsn(dir.path(), &board);
    let server = spawn_kicad_answering(kiapi::common::ApiStatusCode::AsBadRequest);
    let ctx = arc(ctx_talking_to(server.address().to_string()));
    let result =
        super::integration::handle_route_specctra_dsn(&route_args(&dsn, &ses, json!({})), ctx)
            .await
            .unwrap();
    let message = message_of(&result);
    assert!(
        message.starts_with("KiCad did not confirm that it holds"),
        "{message}"
    );
    assert!(!says_not_open(&message, &board), "{message}");
    assert!(message.contains(PREFLIGHT_MARK), "{message}");
}

#[tokio::test]
async fn the_route_refuses_at_once_when_kicad_holds_other_boards() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let (dsn, ses) = staged_dsn(dir.path(), &board);
    let elsewhere = dir.path().join("other.kicad_pcb");
    std::fs::write(&elsewhere, BOARD_TEXT).unwrap();
    let server = spawn_kicad_holding_board(&elsewhere, |_| None);
    let ctx = arc(ctx_talking_to(server.address().to_string()));
    let result =
        super::integration::handle_route_specctra_dsn(&route_args(&dsn, &ses, json!({})), ctx)
            .await
            .unwrap();
    let message = message_of(&result);
    assert!(says_not_open(&message, &board), "{message}");
    assert!(message.contains(PREFLIGHT_MARK), "{message}");
}

#[tokio::test]
async fn the_route_passes_the_preflight_when_the_board_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let (dsn, ses) = staged_dsn(dir.path(), &board);
    let server = spawn_kicad_holding(&board, BOARD_TEXT);
    let ctx = arc(ctx_talking_to(server.address().to_string()));
    let result =
        super::integration::handle_route_specctra_dsn(&route_args(&dsn, &ses, json!({})), ctx)
            .await
            .unwrap();
    let message = message_of(&result);
    assert!(!message.contains(PREFLIGHT_MARK), "{message}");
    assert!(message.contains("jar_path is not a file"), "{message}");
}

/// The opt-out, and a DSN with no manifest (the route refuses with its own reason).
#[tokio::test]
async fn the_route_skips_the_preflight_on_request_or_without_a_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let (dsn, ses) = staged_dsn(dir.path(), &board);
    let ctx = arc(ctx_talking_to(String::new()));
    let opted_out = super::integration::handle_route_specctra_dsn(
        &route_args(&dsn, &ses, json!({ "require_live_board": false })),
        ctx.clone(),
    )
    .await
    .unwrap();
    assert!(
        message_of(&opted_out).contains("jar_path is not a file"),
        "{}",
        message_of(&opted_out)
    );
    let bare = dir.path().join("bare.dsn");
    std::fs::write(&bare, "(pcb bare)\n").unwrap();
    let no_manifest =
        super::integration::handle_route_specctra_dsn(&route_args(&bare, &ses, json!({})), ctx)
            .await
            .unwrap();
    assert!(
        !message_of(&no_manifest).contains(PREFLIGHT_MARK),
        "{}",
        message_of(&no_manifest)
    );
}

/// The advertised schema accepts the opt-out (the dispatcher refuses unknown keys).
#[test]
fn the_route_schema_advertises_require_live_board() {
    let tool = crate::router::registry::tools_for("integration")
        .unwrap()
        .into_iter()
        .find(|tool| tool.name == "route_specctra_dsn")
        .unwrap();
    let property = &tool.input_schema["properties"]["require_live_board"];
    assert_eq!(property["type"], "boolean");
    assert_eq!(property["default"], true);
    assert!(tool
        .input_validator
        .validate(&json!({ "dsn_path": "a", "ses_output_path": "b", "require_live_board": false }))
        .is_ok());
}

// ─── 3. The checkpoint before a bulk edit ────────────────────────────────────

/// A stand-in bulk edit named `name`: counts its calls and answers `reply`.
fn fake_bulk_edit(
    name: &'static str,
    access: BoardAccess,
    calls: Arc<AtomicUsize>,
    reply: fn() -> CallToolResult,
) -> ToolDef {
    let handler: super::ToolHandlerFn = Arc::new(move |_, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(reply()) })
    });
    ToolDef::new(
        name,
        "test double",
        json!({ "type": "object", "properties": { "board": { "type": "string" } } }),
        handler,
    )
    .with_board_access(access)
}

fn ok_reply() -> CallToolResult {
    CallToolResult::json(&json!({ "success": true }))
}

fn checkpoint_ids(dir: &Path) -> Vec<String> {
    let store = dir.join(".konnect/checkpoints");
    if !store.exists() {
        return Vec::new();
    }
    names_in(&store)
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .collect()
}

async fn call(tool: &ToolDef, board: &Path, ctx: Arc<ToolContext>) -> CallToolResult {
    (tool.handler)(&json!({ "board": board }), ctx)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_bulk_edit_checkpoints_first_and_reuses_it_until_the_files_change() {
    let dir = tempfile::tempdir().unwrap();
    let (pro, board) = project(dir.path());
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = board_safety::guard(fake_bulk_edit(
        "set_component_placements",
        BoardAccess::LivePreferredWithFallback,
        calls.clone(),
        ok_reply,
    ));
    let ctx = arc(ctx_talking_to(String::new()));

    let first = body_of(&call(&tool, &board, ctx.clone()).await);
    assert_eq!(first["success"], true, "the tool's own body is kept");
    let note = &first["live_project"];
    assert_eq!(note["board_open_in_kicad"], false, "{note}");
    assert_eq!(note["checkpoint"]["made_by_this_call"], true, "{note}");
    let id = note["checkpoint"]["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(checkpoint_ids(dir.path()), vec![id.clone()]);
    assert_eq!(note["restore"]["tool"], "restore_checkpoint");
    assert_eq!(note["restore"]["arguments"]["checkpoint_id"], id.as_str());
    assert_eq!(
        note["restore"]["arguments"]["project"],
        board.with_extension("kicad_pro").display().to_string()
    );
    assert!(pro.is_file());
    let summary = note["summary"].as_str().unwrap();
    assert!(summary.contains("of the real project"), "{summary}");
    assert!(summary.contains("not a copy"), "{summary}");
    assert!(summary.contains(&id), "{summary}");

    // Nothing saved since: the same checkpoint, no second copy.
    let second = body_of(&call(&tool, &board, ctx.clone()).await);
    assert_eq!(
        second["live_project"]["checkpoint"]["made_by_this_call"],
        false
    );
    assert_eq!(
        second["live_project"]["checkpoint"]["checkpoint_id"],
        id.as_str()
    );
    assert_eq!(checkpoint_ids(dir.path()).len(), 1);

    // The saved board changed: a new checkpoint.
    std::fs::write(&board, EDITED_BOARD_TEXT).unwrap();
    let third = body_of(&call(&tool, &board, ctx).await);
    assert_eq!(
        third["live_project"]["checkpoint"]["made_by_this_call"],
        true
    );
    assert_ne!(
        third["live_project"]["checkpoint"]["checkpoint_id"],
        id.as_str()
    );
    assert_eq!(checkpoint_ids(dir.path()).len(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_bulk_edit_of_a_live_board_says_so_and_names_unsaved_changes() {
    for (live, unsaved) in [(BOARD_TEXT, false), (EDITED_BOARD_TEXT, true)] {
        let dir = tempfile::tempdir().unwrap();
        let (_, board) = project(dir.path());
        let server = spawn_kicad_holding(&board, live);
        let ctx = arc(ctx_talking_to(server.address().to_string()));
        let tool = board_safety::guard(fake_bulk_edit(
            "apply_specctra_ses",
            BoardAccess::LiveOnly,
            Arc::new(AtomicUsize::new(0)),
            ok_reply,
        ));
        let note = body_of(&call(&tool, &board, ctx).await)["live_project"].clone();
        assert_eq!(note["board_open_in_kicad"], true, "{note}");
        assert_eq!(note["unsaved_changes_before_call"], unsaved, "{note}");
        let summary = note["summary"].as_str().unwrap();
        assert!(summary.contains("live board open in KiCad"), "{summary}");
        assert!(summary.contains("Edit > Undo"), "{summary}");
        assert_eq!(note["restore"]["close_the_board_in_kicad_first"], true);
        assert_eq!(note["restore"]["only_after_the_edit_is_saved"], true);
        assert_eq!(
            summary.contains("differed from its saved file when this call started"),
            unsaved,
            "{summary}"
        );
    }
}

/// A live-only edit of a board KiCad does not hold refuses on its own and
/// changes nothing, so no checkpoint is taken for it.
#[tokio::test]
async fn a_live_only_edit_of_a_board_not_open_takes_no_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = board_safety::guard(fake_bulk_edit(
        "apply_specctra_ses",
        BoardAccess::LiveOnly,
        calls.clone(),
        ok_reply,
    ));
    let result = call(&tool, &board, arc(ctx_talking_to(String::new()))).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(body_of(&result).get("live_project").is_none());
    assert!(checkpoint_ids(dir.path()).is_empty());
}

/// No checkpoint can be made: nothing runs.
#[tokio::test]
async fn a_bulk_edit_is_refused_when_its_checkpoint_cannot_be_made() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    std::fs::write(dir.path().join("amp.kicad_sch"), [0xff, 0xfe, 0x00]).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = board_safety::guard(fake_bulk_edit(
        "set_component_placements",
        BoardAccess::LivePreferredWithFallback,
        calls.clone(),
        ok_reply,
    ));
    let result = call(&tool, &board, arc(ctx_talking_to(String::new()))).await;
    assert!(result.is_error);
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the tool did not run");
    let message = message_of(&result);
    assert!(message.starts_with("Nothing was changed"), "{message}");
    assert!(message.contains("not_utf8"), "{message}");
}

/// No project file beside the board: no checkpoint can protect the edit, so
/// nothing runs.
#[tokio::test]
async fn a_bulk_edit_without_a_project_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("loose.kicad_pcb");
    std::fs::write(&board, BOARD_TEXT).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = board_safety::guard(fake_bulk_edit(
        "set_component_placements",
        BoardAccess::LivePreferredWithFallback,
        calls.clone(),
        ok_reply,
    ));
    let result = call(&tool, &board, arc(ctx_talking_to(String::new()))).await;
    assert!(result.is_error);
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the tool did not run");
    let message = message_of(&result);
    assert!(message.starts_with("Nothing was changed"), "{message}");
    assert!(message.contains("no project file"), "{message}");
    assert_eq!(body_of(&result)["error"]["kind"], "plan_blocked");
}

/// A failed call on a live board: no way back is offered that could undo the
/// user's own earlier edit, and the checkpoint is still named.
#[tokio::test]
async fn a_failed_bulk_edit_offers_no_undo() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let server = spawn_kicad_holding(&board, BOARD_TEXT);
    let ctx = arc(ctx_talking_to(server.address().to_string()));
    let tool = board_safety::guard(fake_bulk_edit(
        "apply_specctra_ses",
        BoardAccess::LiveOnly,
        Arc::new(AtomicUsize::new(0)),
        || {
            crate::mcp::protocol::CallToolResult::error_kind(
                crate::mcp::error::ToolErrorKind::HandlerError {
                    reason: "no session".to_string(),
                },
                "the session file is missing",
            )
        },
    ));
    let note = body_of(&call(&tool, &board, ctx).await)["live_project"].clone();
    let summary = note["summary"].as_str().unwrap();
    assert!(!summary.contains("Undo"), "{summary}");
    assert!(!summary.contains("edited"), "{summary}");
    assert!(
        summary.contains("no change to the board is known"),
        "{summary}"
    );
    assert!(note["checkpoint"]["checkpoint_id"].is_string(), "{note}");
}

/// A live-only edit while KiCad holds another board: refused by the tool, no copy.
#[tokio::test]
async fn a_live_only_edit_while_kicad_holds_another_board_takes_no_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let elsewhere = dir.path().join("other.kicad_pcb");
    std::fs::write(&elsewhere, BOARD_TEXT).unwrap();
    let server = spawn_kicad_holding_board(&elsewhere, |_| None);
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = board_safety::guard(fake_bulk_edit(
        "apply_specctra_ses",
        BoardAccess::LiveOnly,
        calls.clone(),
        ok_reply,
    ));
    call(
        &tool,
        &board,
        arc(ctx_talking_to(server.address().to_string())),
    )
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(checkpoint_ids(dir.path()).is_empty());
}

/// The board named by a path that is not canonical (macOS `/var` is a link to
/// `/private/var`): the same project, the same checkpoint store.
#[tokio::test]
async fn a_board_named_by_a_linked_path_checkpoints_its_project() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let linked = dir.path().join("amp.kicad_pcb");
    let tool = board_safety::guard(fake_bulk_edit(
        "set_component_placements",
        BoardAccess::LivePreferredWithFallback,
        Arc::new(AtomicUsize::new(0)),
        ok_reply,
    ));
    let ctx = arc(ctx_talking_to(String::new()));
    let first = body_of(&call(&tool, &linked, ctx.clone()).await);
    let second = body_of(&call(&tool, &linked.canonicalize().unwrap(), ctx).await);
    assert_eq!(checkpoint_ids(dir.path()).len(), 1);
    assert_eq!(
        first["live_project"]["checkpoint"]["checkpoint_id"],
        second["live_project"]["checkpoint"]["checkpoint_id"]
    );
}

/// A checkpoint of the same files whose stored copy no longer reads back is not
/// reused: a fresh one is taken.
#[tokio::test]
async fn a_damaged_checkpoint_is_not_reused() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let tool = board_safety::guard(fake_bulk_edit(
        "set_component_placements",
        BoardAccess::LivePreferredWithFallback,
        Arc::new(AtomicUsize::new(0)),
        ok_reply,
    ));
    let ctx = arc(ctx_talking_to(String::new()));
    let first = body_of(&call(&tool, &board, ctx.clone()).await);
    let id = first["live_project"]["checkpoint"]["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::write(
        dir.path()
            .join(".konnect/checkpoints")
            .join(&id)
            .join("files/amp.kicad_pcb"),
        "damaged",
    )
    .unwrap();
    let second = body_of(&call(&tool, &board, ctx).await);
    assert_eq!(
        second["live_project"]["checkpoint"]["made_by_this_call"],
        true
    );
    assert_ne!(
        second["live_project"]["checkpoint"]["checkpoint_id"],
        id.as_str()
    );
}

/// A refusal in plain text keeps its text; the note follows as its own block.
#[tokio::test]
async fn a_plain_text_result_gets_the_note_as_a_second_block() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let tool = board_safety::guard(fake_bulk_edit(
        "set_component_placements",
        BoardAccess::LivePreferredWithFallback,
        Arc::new(AtomicUsize::new(0)),
        || CallToolResult::error("no such reference"),
    ));
    let result = call(&tool, &board, arc(ctx_talking_to(String::new()))).await;
    assert!(result.is_error);
    assert_eq!(text_of(&result), "no such reference");
    let ToolContent::Text { text } = &result.content[1] else {
        panic!("a second text block")
    };
    let note: Value = serde_json::from_str(text).unwrap();
    assert!(note["live_project"]["summary"]
        .as_str()
        .unwrap()
        .starts_with("The call reported an error"));
    assert!(!text.contains("Undo"), "{text}");
}

/// Tools that are not bulk edits are returned untouched.
#[tokio::test]
async fn other_tools_are_not_wrapped() {
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let tool = board_safety::guard(fake_bulk_edit(
        "move_component",
        BoardAccess::LivePreferredWithFallback,
        Arc::new(AtomicUsize::new(0)),
        ok_reply,
    ));
    let body = body_of(&call(&tool, &board, arc(ctx_talking_to(String::new()))).await);
    assert!(body.get("live_project").is_none());
    assert!(checkpoint_ids(dir.path()).is_empty());
}

/// The registry's own bulk edits are the wrapped ones: each name is registered,
/// and a real call through the registry takes the checkpoint.
#[tokio::test]
async fn the_registry_wraps_every_bulk_edit() {
    for name in BULK_LIVE_EDITS {
        let found = crate::router::registry::ALL_TOOLSETS
            .iter()
            .flat_map(|set| crate::router::registry::tools_for(set.name).unwrap())
            .any(|tool| tool.name == *name);
        assert!(found, "{name} is registered");
    }

    // set_component_placements through the registry, KiCad down: the file path,
    // which refuses an unknown reference, after the checkpoint.
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let tool = crate::router::registry::tools_for("pcb_components")
        .unwrap()
        .into_iter()
        .find(|tool| tool.name == "set_component_placements")
        .unwrap();
    let result = (tool.handler)(
        &json!({
            "board": board,
            "placements": [{ "reference": "R404", "x": 1.0, "y": 2.0, "rotation": 0.0 }]
        }),
        arc(ctx_talking_to(String::new())),
    )
    .await
    .unwrap();
    assert_eq!(checkpoint_ids(dir.path()).len(), 1, "{:?}", result.content);
    assert!(
        result.content.iter().any(
            |content| matches!(content, ToolContent::Text { text } if text.contains("live_project"))
        ),
        "{:?}",
        result.content
    );

    // apply_specctra_ses through the registry with the board live: the checkpoint
    // comes first, then the tool refuses its missing session file.
    let dir = tempfile::tempdir().unwrap();
    let (_, board) = project(dir.path());
    let server = spawn_kicad_holding(&board, BOARD_TEXT);
    let tool = crate::router::registry::tools_for("pcb_routing")
        .unwrap()
        .into_iter()
        .find(|tool| tool.name == "apply_specctra_ses")
        .unwrap();
    let result = (tool.handler)(
        &json!({
            "board": board,
            "ses_path": dir.path().join("missing.ses"),
            "manifest_path": dir.path().join("missing.dsn.konnect.json"),
            "candidate_output_path": dir.path().join("candidate.kicad_pcb"),
        }),
        arc(ctx_talking_to(server.address().to_string())),
    )
    .await
    .unwrap();
    assert_eq!(checkpoint_ids(dir.path()).len(), 1, "{:?}", result.content);
    assert!(
        result
            .content
            .iter()
            .any(|content| matches!(content, ToolContent::Text { text } if text.contains("\"board_open_in_kicad\":true"))),
        "{:?}",
        result.content
    );
}
