use super::*;
use crate::mcp::protocol::ToolContent;
use crate::router::ToolRouter;
use crate::test_support::MockIpcServer;
use crate::tools::ServerConfig;
use konnect_ipc::gen::kiapi;
use std::sync::Arc;

const BOARD: &str = "(kicad_pcb (version 20240108)\n  (net 0 \"\")\n)\n";
const SCHEMATIC: &str = "(kicad_sch (version 20231120) (generator eeschema))\n";
const CHILD: &str = "(kicad_sch (version 20231120) (generator eeschema)\n  (title \"child\"))\n";
/// CRLF must survive a round trip byte for byte.
const FOOTPRINT: &str = "(footprint \"R\"\r\n  (layer \"F.Cu\"))\r\n";
const PROJECT: &str = "{\n  \"meta\": { \"version\": 1 }\n}\n";
const TABLE: &str = "(fp_lib_table\n  (version 7)\n)\n";

fn ctx() -> ToolContext {
    ctx_at(String::new())
}

fn ctx_at(ipc_address: String) -> ToolContext {
    ToolContext::new(
        ServerConfig {
            ipc_address,
            ..Default::default()
        },
        Arc::new(ToolRouter::new()),
    )
}

fn write(dir: &Path, relative: &str, content: &str) {
    let path = join_relative(dir, relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn read(dir: &Path, relative: &str) -> String {
    std::fs::read_to_string(join_relative(dir, relative)).unwrap()
}

/// A project with the captured kinds plus every kind of noise that must be
/// left out.
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "amp.kicad_pro", PROJECT);
    write(root, "amp.kicad_sch", SCHEMATIC);
    write(root, "amp.kicad_pcb", BOARD);
    write(root, "sheets/child.kicad_sch", CHILD);
    write(root, "lib/my.pretty/R.kicad_mod", FOOTPRINT);
    write(root, "fp-lib-table", TABLE);
    // Noise.
    write(root, "amp.kicad_prl", "{}");
    write(root, "notes.txt", "hello");
    write(root, "_autosave-amp.kicad_sch", SCHEMATIC);
    write(root, ".history/amp.kicad_sch", SCHEMATIC);
    write(root, "amp-backups/amp.kicad_pcb", BOARD);
    let project = root.join("amp.kicad_pro");
    (dir, project)
}

fn args(project: &Path) -> Value {
    json!({ "project": project.display().to_string() })
}

fn body(result: &CallToolResult) -> Value {
    match &result.content[0] {
        ToolContent::Text { text } => serde_json::from_str(text).unwrap(),
        _ => panic!("expected text content"),
    }
}

fn error_kind(result: &CallToolResult) -> String {
    assert!(result.is_error, "expected an error: {:?}", body(result));
    body(result)["error"]["kind"].as_str().unwrap().to_string()
}

fn reasons(result: &CallToolResult) -> Vec<String> {
    body(result)["error"]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect()
}

async fn checkpoint(project: &Path, label: Option<&str>) -> Value {
    let mut request = args(project);
    if let Some(label) = label {
        request["label"] = json!(label);
    }
    let result = handle_create_checkpoint(&request, &ctx()).await.unwrap();
    assert!(!result.is_error, "{:?}", body(&result));
    body(&result)
}

async fn preview(ctx: &ToolContext, project: &Path, id: &str) -> CallToolResult {
    let mut request = args(project);
    request["checkpoint_id"] = json!(id);
    handle_restore_checkpoint(&request, ctx).await.unwrap()
}

async fn apply(ctx: &ToolContext, project: &Path, id: &str, revision: &str) -> CallToolResult {
    let mut request = args(project);
    request["checkpoint_id"] = json!(id);
    request["dry_run"] = json!(false);
    request["expected_revision"] = json!(revision);
    handle_restore_checkpoint(&request, ctx).await.unwrap()
}

fn store(project: &Path) -> PathBuf {
    project.parent().unwrap().join(".konnect/checkpoints")
}

fn checkpoint_ids(project: &Path) -> Vec<String> {
    let mut ids: Vec<String> = std::fs::read_dir(store(project))
        .map(|entries| {
            entries
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

// ─── create ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_checkpoint_holds_exactly_the_design_files_byte_for_byte() {
    let (dir, project) = fixture();
    let created = checkpoint(&project, Some("before rerouting")).await;
    let id = created["checkpoint_id"].as_str().unwrap();
    assert!(is_valid_checkpoint_id(id), "{id}");
    assert_eq!(created["label"], "before rerouting");
    assert_eq!(created["file_count"], 6);
    assert_eq!(created["locks_present"], json!([]));
    assert!(created.get("warning").is_none());

    let manifest = load_manifest(&store(&project), id).unwrap();
    let paths: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "amp.kicad_pcb",
            "amp.kicad_pro",
            "amp.kicad_sch",
            "fp-lib-table",
            "lib/my.pretty/R.kicad_mod",
            "sheets/child.kicad_sch",
        ]
    );
    assert_eq!(manifest.reason, "manual");
    assert_eq!(manifest.project_file, "amp.kicad_pro");
    let copy = store(&project)
        .join(id)
        .join("files/lib/my.pretty/R.kicad_mod");
    assert_eq!(std::fs::read(copy).unwrap(), FOOTPRINT.as_bytes());
    // Only the published checkpoint remains in the store.
    assert_eq!(checkpoint_ids(&project), [id.to_string()]);
    drop(dir);
}

#[tokio::test]
async fn an_unchanged_project_is_reported_identical_to_the_last_checkpoint() {
    let (_dir, project) = fixture();
    let first = checkpoint(&project, None).await;
    let second = checkpoint(&project, None).await;
    assert_eq!(second["identical_to"], first["checkpoint_id"]);
    assert_eq!(second["content_hash"], first["content_hash"]);
    assert_ne!(second["checkpoint_id"], first["checkpoint_id"]);
}

#[tokio::test]
async fn open_documents_are_named_because_unsaved_changes_are_not_captured() {
    let (dir, project) = fixture();
    write(dir.path(), "~amp.kicad_pcb.lck", "{}");
    let created = checkpoint(&project, None).await;
    assert_eq!(created["locks_present"], json!(["~amp.kicad_pcb.lck"]));
    assert!(created["warning"].as_str().unwrap().contains("unsaved"));
}

#[tokio::test]
async fn a_file_that_is_not_utf8_refuses_the_checkpoint_and_keeps_nothing() {
    let (dir, project) = fixture();
    std::fs::write(dir.path().join("bad.kicad_sym"), [0xff, 0xfe, 0x00]).unwrap();
    let result = handle_create_checkpoint(&args(&project), &ctx())
        .await
        .unwrap();
    assert_eq!(error_kind(&result), "plan_blocked");
    assert!(reasons(&result)[0].starts_with("not_utf8: bad.kicad_sym"));
    assert!(checkpoint_ids(&project).is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_files_are_skipped_and_a_symlinked_store_is_refused() {
    let (dir, project) = fixture();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("x.kicad_sym"), "(kicad_symbol_lib)").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("x.kicad_sym"),
        dir.path().join("x.kicad_sym"),
    )
    .unwrap();
    let created = checkpoint(&project, None).await;
    assert_eq!(
        created["skipped"],
        json!([{ "path": "x.kicad_sym", "reason": "symlink" }])
    );

    let other = fixture();
    let target = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(target.path(), other.0.path().join(".konnect")).unwrap();
    let result = handle_create_checkpoint(&args(&other.1), &ctx())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(body(&result)["message"]
        .as_str()
        .unwrap()
        .contains("real folder"));
    assert_eq!(std::fs::read_dir(target.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn the_project_argument_must_be_a_kicad_pro_file() {
    let (dir, _project) = fixture();
    let board = json!({ "project": dir.path().join("amp.kicad_pcb").display().to_string() });
    let result = handle_create_checkpoint(&board, &ctx()).await.unwrap();
    assert_eq!(error_kind(&result), "invalid_argument");

    let missing = json!({ "project": dir.path().join("nope.kicad_pro").display().to_string() });
    let result = handle_create_checkpoint(&missing, &ctx()).await.unwrap();
    assert_eq!(error_kind(&result), "file_not_found");

    let mut long = args(&dir.path().join("amp.kicad_pro"));
    long["label"] = json!("x".repeat(MAX_LABEL_CHARS + 1));
    let result = handle_create_checkpoint(&long, &ctx()).await.unwrap();
    assert_eq!(error_kind(&result), "invalid_argument");
}

// ─── list ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_shows_newest_first_with_the_difference_from_the_saved_files() {
    let (dir, project) = fixture();
    let result = handle_list_checkpoints(&args(&project), &ctx())
        .await
        .unwrap();
    assert_eq!(body(&result)["count"], 0);

    let first = checkpoint(&project, Some("one")).await;
    write(
        dir.path(),
        "amp.kicad_pcb",
        "(kicad_pcb (version 20240108))\n",
    );
    std::fs::remove_file(dir.path().join("sheets/child.kicad_sch")).unwrap();
    write(dir.path(), "new.kicad_sch", SCHEMATIC);
    let second = checkpoint(&project, Some("two")).await;

    let listed = body(
        &handle_list_checkpoints(&args(&project), &ctx())
            .await
            .unwrap(),
    );
    assert_eq!(listed["count"], 2);
    let rows = listed["checkpoints"].as_array().unwrap();
    assert_eq!(rows[0]["checkpoint_id"], second["checkpoint_id"]);
    assert_eq!(rows[0]["difference_from_saved_files"]["matches"], true);
    assert_eq!(rows[1]["checkpoint_id"], first["checkpoint_id"]);
    assert_eq!(
        rows[1]["difference_from_saved_files"],
        json!({ "matches": false, "changed": 1, "missing": 1, "added": 1 })
    );

    let mut limited = args(&project);
    limited["limit"] = json!(1);
    let listed = body(&handle_list_checkpoints(&limited, &ctx()).await.unwrap());
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["truncated"], true);
}

#[tokio::test]
async fn an_unreadable_checkpoint_is_reported_not_hidden() {
    let (_dir, project) = fixture();
    let created = checkpoint(&project, None).await;
    let id = created["checkpoint_id"].as_str().unwrap();
    let manifest = store(&project).join(id).join(MANIFEST_FILE);
    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
    value["surprise"] = json!(true);
    std::fs::write(&manifest, value.to_string()).unwrap();
    std::fs::create_dir(store(&project).join(".partial-abc")).unwrap();

    let listed = body(
        &handle_list_checkpoints(&args(&project), &ctx())
            .await
            .unwrap(),
    );
    assert_eq!(listed["count"], 0);
    let unreadable = listed["unreadable"].as_array().unwrap();
    assert_eq!(unreadable.len(), 1, "{unreadable:?}");
    assert_eq!(unreadable[0]["id"], id);
    assert!(unreadable[0]["reason"]
        .as_str()
        .unwrap()
        .contains("malformed"));
}

// ─── restore ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn restore_previews_then_applies_and_can_be_undone() {
    let (dir, project) = fixture();
    let root = dir.path();
    let id = checkpoint(&project, None).await["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    let edited_board = "(kicad_pcb (version 20240108) (edited))\n";
    write(root, "amp.kicad_pcb", edited_board);
    std::fs::remove_dir_all(root.join("sheets")).unwrap();
    write(root, "extra.kicad_sch", SCHEMATIC);

    let ctx = ctx();
    let previewed = preview(&ctx, &project, &id).await;
    assert!(!previewed.is_error, "{:?}", body(&previewed));
    let plan = body(&previewed);
    assert_eq!(plan["dry_run"], true);
    assert_eq!(
        plan["changes"],
        json!([
            { "path": "amp.kicad_pcb", "action": "replace" },
            { "path": "sheets/child.kicad_sch", "action": "create" }
        ])
    );
    assert_eq!(plan["unchanged"], 4);
    assert_eq!(plan["added_since"], json!(["extra.kicad_sch"]));
    assert_eq!(plan["blocked_by"], json!([]));
    assert_eq!(plan["can_apply"], true);
    assert_eq!(plan["kicad_ipc"]["status"], "not_configured");
    // A preview writes nothing.
    assert_eq!(read(root, "amp.kicad_pcb"), edited_board);
    assert_eq!(checkpoint_ids(&project).len(), 1);

    let applied = apply(&ctx, &project, &id, plan["revision"].as_str().unwrap()).await;
    assert!(!applied.is_error, "{:?}", body(&applied));
    let done = body(&applied);
    assert_eq!(done["verified"], true);
    assert_eq!(read(root, "amp.kicad_pcb"), BOARD);
    assert_eq!(read(root, "sheets/child.kicad_sch"), CHILD);
    assert_eq!(
        read(root, "extra.kicad_sch"),
        SCHEMATIC,
        "restore never deletes"
    );

    // The undo point is the edited state.
    let undo = done["undo_checkpoint"].as_str().unwrap().to_string();
    let undo_manifest = load_manifest(&store(&project), &undo).unwrap();
    assert_eq!(undo_manifest.reason, "pre_restore");
    let undo_plan = body(&preview(&ctx, &project, &undo).await);
    let undone = apply(
        &ctx,
        &project,
        &undo,
        undo_plan["revision"].as_str().unwrap(),
    )
    .await;
    assert!(!undone.is_error, "{:?}", body(&undone));
    assert_eq!(read(root, "amp.kicad_pcb"), edited_board);
    // The undo checkpoint did not hold the child sheet; restore never deletes it.
    assert_eq!(read(root, "sheets/child.kicad_sch"), CHILD);
}

#[tokio::test]
async fn crlf_bytes_are_restored_exactly() {
    let (dir, project) = fixture();
    let id = checkpoint(&project, None).await["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    write(
        dir.path(),
        "lib/my.pretty/R.kicad_mod",
        "(footprint \"R\")\n",
    );
    let ctx = ctx();
    let plan = body(&preview(&ctx, &project, &id).await);
    let applied = apply(&ctx, &project, &id, plan["revision"].as_str().unwrap()).await;
    assert!(!applied.is_error, "{:?}", body(&applied));
    assert_eq!(
        std::fs::read(dir.path().join("lib/my.pretty/R.kicad_mod")).unwrap(),
        FOOTPRINT.as_bytes()
    );
}

#[tokio::test]
async fn apply_needs_the_preview_revision_and_refuses_a_changed_project() {
    let (dir, project) = fixture();
    let id = checkpoint(&project, None).await["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    write(dir.path(), "amp.kicad_pcb", "(kicad_pcb (edited))\n");
    let ctx = ctx();

    let mut no_revision = args(&project);
    no_revision["checkpoint_id"] = json!(id);
    no_revision["dry_run"] = json!(false);
    let result = handle_restore_checkpoint(&no_revision, &ctx).await.unwrap();
    assert_eq!(error_kind(&result), "invalid_argument");

    let plan = body(&preview(&ctx, &project, &id).await);
    write(dir.path(), "amp.kicad_pcb", "(kicad_pcb (edited again))\n");
    let result = apply(&ctx, &project, &id, plan["revision"].as_str().unwrap()).await;
    assert_eq!(error_kind(&result), "stale_target");
    assert_eq!(
        read(dir.path(), "amp.kicad_pcb"),
        "(kicad_pcb (edited again))\n"
    );
    assert_eq!(
        checkpoint_ids(&project).len(),
        1,
        "no undo checkpoint for a refusal"
    );
}

#[tokio::test]
async fn an_open_board_or_project_blocks_restore_and_nothing_is_written() {
    let (dir, project) = fixture();
    let id = checkpoint(&project, None).await["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    let edited = "(kicad_pcb (edited))\n";
    write(dir.path(), "amp.kicad_pcb", edited);
    let ctx = ctx();

    write(dir.path(), "~amp.kicad_pcb.lck", "{}");
    let plan = body(&preview(&ctx, &project, &id).await);
    assert_eq!(plan["can_apply"], false);
    let result = apply(&ctx, &project, &id, plan["revision"].as_str().unwrap()).await;
    assert_eq!(error_kind(&result), "plan_blocked");
    assert!(reasons(&result)[0].starts_with("kicad_lock_present: amp.kicad_pcb"));
    std::fs::remove_file(dir.path().join("~amp.kicad_pcb.lck")).unwrap();

    write(dir.path(), "~amp.kicad_pro.lck", "{}");
    let plan = body(&preview(&ctx, &project, &id).await);
    let result = apply(&ctx, &project, &id, plan["revision"].as_str().unwrap()).await;
    assert_eq!(error_kind(&result), "plan_blocked");
    assert!(reasons(&result)[0].starts_with("kicad_project_open"));

    assert_eq!(read(dir.path(), "amp.kicad_pcb"), edited);
    assert_eq!(checkpoint_ids(&project).len(), 1);
}

#[tokio::test]
async fn a_lock_on_an_unchanged_file_does_not_block() {
    let (dir, project) = fixture();
    let id = checkpoint(&project, None).await["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    write(dir.path(), "amp.kicad_pcb", "(kicad_pcb (edited))\n");
    // The schematic is open in KiCad but is not a restore target.
    write(dir.path(), "~amp.kicad_sch.lck", "{}");
    let ctx = ctx();
    let mut request = args(&project);
    request["checkpoint_id"] = json!(id);
    request["paths"] = json!(["amp.kicad_pcb"]);
    let plan = body(&handle_restore_checkpoint(&request, &ctx).await.unwrap());
    assert_eq!(plan["blocked_by"], json!([]), "{plan}");
    request["dry_run"] = json!(false);
    request["expected_revision"] = plan["revision"].clone();
    let applied = handle_restore_checkpoint(&request, &ctx).await.unwrap();
    assert!(!applied.is_error, "{:?}", body(&applied));
    assert_eq!(read(dir.path(), "amp.kicad_pcb"), BOARD);
}

#[tokio::test]
async fn a_board_seen_live_this_session_blocks_its_restore() {
    let (dir, project) = fixture();
    let id = checkpoint(&project, None).await["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    write(dir.path(), "amp.kicad_pcb", "(kicad_pcb (edited))\n");
    let ctx = ctx();
    // Observed live without a lock: there is no evidence of a clean close.
    ctx.board_session
        .observe_live(&dir.path().join("amp.kicad_pcb"));
    let plan = body(&preview(&ctx, &project, &id).await);
    let blocked = plan["blocked_by"].as_array().unwrap();
    assert!(
        blocked[0]
            .as_str()
            .unwrap()
            .starts_with("board_observed_live"),
        "{blocked:?}"
    );
}

#[tokio::test]
async fn a_subset_restore_touches_only_the_named_files() {
    let (dir, project) = fixture();
    let id = checkpoint(&project, None).await["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    write(dir.path(), "amp.kicad_pcb", "(kicad_pcb (edited))\n");
    write(dir.path(), "amp.kicad_sch", "(kicad_sch (edited))\n");
    let ctx = ctx();

    let mut request = args(&project);
    request["checkpoint_id"] = json!(id);
    request["paths"] = json!(["missing.kicad_pcb"]);
    let result = handle_restore_checkpoint(&request, &ctx).await.unwrap();
    assert_eq!(error_kind(&result), "invalid_argument");

    request["paths"] = json!(["amp.kicad_pcb"]);
    let plan = body(&handle_restore_checkpoint(&request, &ctx).await.unwrap());
    assert_eq!(
        plan["changes"],
        json!([{ "path": "amp.kicad_pcb", "action": "replace" }])
    );
    // A revision binds the subset it was computed for.
    let result = apply(&ctx, &project, &id, plan["revision"].as_str().unwrap()).await;
    assert_eq!(error_kind(&result), "stale_target");

    request["dry_run"] = json!(false);
    request["expected_revision"] = plan["revision"].clone();
    let applied = handle_restore_checkpoint(&request, &ctx).await.unwrap();
    assert!(!applied.is_error, "{:?}", body(&applied));
    assert_eq!(read(dir.path(), "amp.kicad_pcb"), BOARD);
    assert_eq!(read(dir.path(), "amp.kicad_sch"), "(kicad_sch (edited))\n");
}

#[tokio::test]
async fn a_tampered_checkpoint_is_never_used() {
    let (dir, project) = fixture();
    let id = checkpoint(&project, None).await["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    write(dir.path(), "amp.kicad_pcb", "(kicad_pcb (edited))\n");
    std::fs::write(
        store(&project).join(&id).join("files/amp.kicad_pcb"),
        "(kicad_pcb (tampered))\n",
    )
    .unwrap();
    let result = preview(&ctx(), &project, &id).await;
    assert_eq!(error_kind(&result), "plan_blocked");
    assert!(reasons(&result)[0].starts_with("checkpoint_corrupt"));
}

#[tokio::test]
async fn checkpoint_ids_are_checked_before_any_path_is_built() {
    let (_dir, project) = fixture();
    for bad in ["../../etc", "", "20261108T101500-1a2b3c4d-7f3a/..", "x"] {
        let result = preview(&ctx(), &project, bad).await;
        assert_eq!(error_kind(&result), "invalid_argument", "{bad}");
    }
    let result = preview(&ctx(), &project, "20261108T101500-1a2b3c4d-7f3a").await;
    assert_eq!(error_kind(&result), "file_not_found");
}

#[tokio::test]
async fn restoring_a_matching_checkpoint_writes_nothing() {
    let (_dir, project) = fixture();
    let id = checkpoint(&project, None).await["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    let ctx = ctx();
    let plan = body(&preview(&ctx, &project, &id).await);
    assert_eq!(plan["can_apply"], false);
    let done = body(&apply(&ctx, &project, &id, plan["revision"].as_str().unwrap()).await);
    assert_eq!(done["restored"], json!([]));
    assert!(done.get("undo_checkpoint").is_none());
    assert_eq!(checkpoint_ids(&project).len(), 1);
}

#[test]
fn manifest_paths_cannot_escape_or_hide() {
    for bad in [
        "../amp.kicad_pcb",
        "/etc/amp.kicad_pcb",
        "a//b.kicad_pcb",
        ".konnect/x.kicad_pcb",
        "a\\b.kicad_pcb",
        "notes.txt",
        "~amp.kicad_pcb.lck",
    ] {
        assert!(!is_safe_relative(bad), "{bad}");
    }
    assert!(is_safe_relative("sheets/child.kicad_sch"));
    assert!(is_safe_relative("fp-lib-table"));
}

#[test]
fn utc_text_is_iso_8601() {
    assert_eq!(utc_text(1_793_450_096), "2026-10-31T12:34:56Z");
}

// ─── readiness ───────────────────────────────────────────────────────────────

fn row<'a>(report: &'a Value, path: &str) -> &'a Value {
    report["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["path"] == path)
        .unwrap_or_else(|| panic!("no row for {path}: {report}"))
}

#[tokio::test]
async fn a_closed_project_is_ready_for_file_edits_and_says_kicad_is_not_answering() {
    let (_dir, project) = fixture();
    let report = body(
        &handle_get_edit_readiness(&args(&project), &ctx())
            .await
            .unwrap(),
    );
    assert_eq!(report["verdict"], "ready");
    assert_eq!(report["summary"]["files"], 6);
    assert_eq!(report["summary"]["blocked"], 0);
    assert_eq!(report["kicad"]["ipc"], "not_configured");
    assert_eq!(report["kicad"]["project_lock"], "absent");
    assert!(report["guidance"][0]
        .as_str()
        .unwrap()
        .contains("not answering over IPC"));
    let board = row(&report, "amp.kicad_pcb");
    assert_eq!(board["kind"], "board");
    assert_eq!(board["lock"], "absent");
    assert_eq!(board["edit_route"], "file");
    assert_eq!(row(&report, "fp-lib-table")["lock"], Value::Null);
}

#[tokio::test]
async fn locks_block_the_files_they_name_and_the_project_lock_warns() {
    let (dir, project) = fixture();
    write(dir.path(), "sheets/~child.kicad_sch.lck", "{}");
    write(dir.path(), "~amp.kicad_pcb.lck", "{}");
    write(dir.path(), "~amp.kicad_pro.lck", "{}");
    let report = body(
        &handle_get_edit_readiness(&args(&project), &ctx())
            .await
            .unwrap(),
    );
    assert_eq!(report["verdict"], "blocked");
    // One locked sheet holds the whole schematic.
    assert_eq!(report["summary"]["blocked"], 3);
    assert_eq!(
        row(&report, "sheets/child.kicad_sch")["edit_route"],
        "blocked"
    );
    assert_eq!(row(&report, "amp.kicad_pcb")["edit_route"], "blocked");
    assert_eq!(row(&report, "amp.kicad_sch")["edit_route"], "blocked");
    let pro = row(&report, "amp.kicad_pro");
    assert_eq!(pro["edit_route"], "file");
    assert!(pro["warnings"][0]
        .as_str()
        .unwrap()
        .starts_with("kicad_project_open"));
    assert_eq!(report["kicad"]["project_lock"], "present");
    let guidance = report["guidance"].as_array().unwrap();
    assert!(guidance
        .iter()
        .any(|g| g.as_str().unwrap().contains("never deletes KiCad lock")));
    // Read-only: the locks are still there.
    assert!(dir.path().join("~amp.kicad_pcb.lck").exists());
}

#[tokio::test]
async fn an_unreadable_transaction_journal_blocks_document_edits() {
    let (dir, project) = fixture();
    write(
        dir.path(),
        ".konnect-transaction-00000000-0000-4000-8000-000000000000.json",
        "not json",
    );
    let report = body(
        &handle_get_edit_readiness(&args(&project), &ctx())
            .await
            .unwrap(),
    );
    assert!(report["transaction_journal_error"].is_string(), "{report}");
    assert_eq!(report["verdict"], "blocked");
    assert_eq!(row(&report, "amp.kicad_pcb")["edit_route"], "blocked");
}

fn document(
    kind: kiapi::common::types::DocumentType,
    project_dir: &str,
    board: Option<&str>,
) -> kiapi::common::types::DocumentSpecifier {
    use kiapi::common::types::document_specifier::Identifier;
    kiapi::common::types::DocumentSpecifier {
        r#type: kind as i32,
        identifier: Some(match board {
            Some(board) => Identifier::BoardFilename(board.to_string()),
            None => Identifier::SheetPath(kiapi::common::types::SheetPath {
                path: vec![kiapi::common::types::Kiid {
                    value: "00000000-0000-0000-0000-000000000001".to_string(),
                }],
                path_human_readable: "/".to_string(),
            }),
        }),
        project: Some(kiapi::common::types::ProjectSpecifier {
            name: "amp".to_string(),
            path: project_dir.to_string(),
        }),
    }
}

/// A KiCad holding this project's board (and optionally its schematic) open.
fn spawn_kicad_with(project_dir: &Path, schematic_open: bool) -> MockIpcServer {
    use prost::Message;
    let dir = project_dir.display().to_string();
    MockIpcServer::spawn("checkpoints-readiness", move |request| {
        let command = request.message.expect("a command");
        let message = if command.type_url.ends_with("GetVersion") {
            Some(konnect_ipc::builders::pack_any(
                &kiapi::common::commands::GetVersionResponse {
                    version: Some(kiapi::common::types::KiCadVersion {
                        major: 10,
                        minor: 0,
                        patch: 5,
                        full_version: "10.0.5".to_string(),
                    }),
                },
                "kiapi.common.commands.GetVersionResponse",
            ))
        } else if command.type_url.ends_with("GetOpenDocuments") {
            let asked = kiapi::common::commands::GetOpenDocuments::decode(command.value.as_slice())
                .expect("GetOpenDocuments request");
            let documents = if asked.r#type == kiapi::common::types::DocumentType::DoctypePcb as i32
            {
                vec![document(
                    kiapi::common::types::DocumentType::DoctypePcb,
                    &dir,
                    Some("amp.kicad_pcb"),
                )]
            } else if schematic_open {
                vec![document(
                    kiapi::common::types::DocumentType::DoctypeSchematic,
                    &dir,
                    None,
                )]
            } else {
                Vec::new()
            };
            Some(konnect_ipc::builders::pack_any(
                &kiapi::common::commands::GetOpenDocumentsResponse { documents },
                "kiapi.common.commands.GetOpenDocumentsResponse",
            ))
        } else {
            None
        };
        kiapi::common::ApiResponse {
            status: Some(kiapi::common::ApiResponseStatus {
                status: kiapi::common::ApiStatusCode::AsOk as i32,
                error_message: String::new(),
            }),
            header: None,
            message,
        }
    })
}

#[tokio::test]
async fn a_board_open_in_kicad_is_edited_live_and_its_schematic_is_blocked() {
    let (dir, project) = fixture();
    write(dir.path(), "~amp.kicad_pcb.lck", "{}");
    write(dir.path(), "~amp.kicad_sch.lck", "{}");
    let kicad = spawn_kicad_with(dir.path(), true);
    let ctx = ctx_at(kicad.address().to_string());
    let report = body(
        &handle_get_edit_readiness(&args(&project), &ctx)
            .await
            .unwrap(),
    );
    assert_eq!(report["kicad"]["ipc"], "reachable", "{report}");
    assert_eq!(report["kicad"]["project_open"], true);
    let board = row(&report, "amp.kicad_pcb");
    assert_eq!(board["open_in_kicad"], true);
    assert_eq!(board["edit_route"], "live");
    assert_eq!(row(&report, "amp.kicad_sch")["edit_route"], "blocked");
    assert_eq!(report["verdict"], "blocked");
}

#[tokio::test]
async fn a_board_kicad_reports_open_blocks_restore_even_without_a_lock() {
    let (dir, project) = fixture();
    let id = checkpoint(&project, None).await["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    write(dir.path(), "amp.kicad_pcb", "(kicad_pcb (edited))\n");
    let kicad = spawn_kicad_with(dir.path(), false);
    let ctx = ctx_at(kicad.address().to_string());
    let plan = body(&preview(&ctx, &project, &id).await);
    assert_eq!(plan["kicad_ipc"]["status"], "reachable", "{plan}");
    let blocked: Vec<&str> = plan["blocked_by"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_str().unwrap())
        .collect();
    assert!(
        blocked
            .iter()
            .any(|b| b.starts_with("open_in_kicad: amp.kicad_pcb")),
        "{blocked:?}"
    );
    assert!(
        blocked.iter().any(|b| b.starts_with("kicad_project_open")),
        "{blocked:?}"
    );
}

#[test]
fn the_project_toolset_lists_the_new_tools() {
    let names: Vec<String> = crate::tools::project::tools()
        .iter()
        .map(|tool| tool.name.to_string())
        .collect();
    for name in [
        "create_checkpoint",
        "list_checkpoints",
        "restore_checkpoint",
        "get_edit_readiness",
    ] {
        assert!(
            names.iter().any(|n| n == name),
            "{name} missing from {names:?}"
        );
    }
}

#[tokio::test]
async fn appledouble_sidecars_are_not_design_files() {
    let (dir, project) = fixture();
    std::fs::write(
        dir.path().join("._amp.kicad_sch"),
        [0x00, 0x05, 0x16, 0x07, 0xff],
    )
    .unwrap();
    let created = checkpoint(&project, None).await;
    assert_eq!(created["file_count"], 6);
    let id = created["checkpoint_id"].as_str().unwrap();
    assert!(load_manifest(&store(&project), id).is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn restore_never_reads_or_writes_through_a_symlinked_folder() {
    let (dir, project) = fixture();
    let id = checkpoint(&project, None).await["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    let outside = tempfile::tempdir().unwrap();
    write(
        outside.path(),
        "my.pretty/R.kicad_mod",
        "(footprint \"elsewhere\")\n",
    );
    std::fs::remove_dir_all(dir.path().join("lib")).unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("lib")).unwrap();

    let ctx = ctx();
    let plan = body(&preview(&ctx, &project, &id).await);
    let blocked = plan["blocked_by"].as_array().unwrap();
    assert!(
        blocked.iter().any(|b| b
            .as_str()
            .unwrap()
            .starts_with("target_parent_not_a_folder: lib")),
        "{blocked:?}"
    );
    let result = apply(&ctx, &project, &id, plan["revision"].as_str().unwrap()).await;
    assert_eq!(error_kind(&result), "plan_blocked");
    assert_eq!(
        read(outside.path(), "my.pretty/R.kicad_mod"),
        "(footprint \"elsewhere\")\n"
    );
}

#[tokio::test]
async fn a_locked_root_sheet_holds_every_sheet_of_the_schematic() {
    let (dir, project) = fixture();
    let id = checkpoint(&project, None).await["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    let edited = "(kicad_sch (edited))\n";
    write(dir.path(), "sheets/child.kicad_sch", edited);
    // KiCad locks only the root sheet it opened.
    write(dir.path(), "~amp.kicad_sch.lck", "{}");

    let report = body(
        &handle_get_edit_readiness(&args(&project), &ctx())
            .await
            .unwrap(),
    );
    let child = row(&report, "sheets/child.kicad_sch");
    assert_eq!(child["edit_route"], "blocked", "{child}");
    assert!(child["blocked_by"][0]
        .as_str()
        .unwrap()
        .starts_with("kicad_schematic_open"));

    let ctx = ctx();
    let plan = body(&preview(&ctx, &project, &id).await);
    let result = apply(&ctx, &project, &id, plan["revision"].as_str().unwrap()).await;
    assert_eq!(error_kind(&result), "plan_blocked");
    assert!(reasons(&result)[0].starts_with("kicad_schematic_open: sheets/child.kicad_sch"));
    assert_eq!(read(dir.path(), "sheets/child.kicad_sch"), edited);
}

#[tokio::test]
async fn an_open_schematic_editor_blocks_sheets_without_locks() {
    let (dir, project) = fixture();
    let kicad = spawn_kicad_with(dir.path(), true);
    let ctx = ctx_at(kicad.address().to_string());
    let report = body(
        &handle_get_edit_readiness(&args(&project), &ctx)
            .await
            .unwrap(),
    );
    assert_eq!(
        row(&report, "sheets/child.kicad_sch")["edit_route"],
        "blocked",
        "{report}"
    );
    assert_eq!(row(&report, "amp.kicad_sch")["edit_route"], "blocked");
}

#[tokio::test]
async fn any_pending_or_unreadable_journal_blocks_the_verdict() {
    // No schematic or board rows to block: the journal alone must decide.
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "amp.kicad_pro", PROJECT);
    write(
        dir.path(),
        ".konnect-transaction-00000000-0000-4000-8000-000000000000.json",
        "not json",
    );
    let project = dir.path().join("amp.kicad_pro");
    let report = body(
        &handle_get_edit_readiness(&args(&project), &ctx())
            .await
            .unwrap(),
    );
    assert_eq!(report["summary"]["blocked"], 0, "{report}");
    assert_eq!(report["verdict"], "blocked");
}

#[tokio::test]
async fn a_kicad_that_answers_without_documents_is_not_called_unreachable() {
    let (_dir, project) = fixture();
    // Answers Ping, refuses everything else.
    let kicad = MockIpcServer::spawn("checkpoints-no-documents", |request| {
        let command = request.message.expect("a command");
        let status = if command.type_url.ends_with("Ping") {
            kiapi::common::ApiStatusCode::AsOk
        } else {
            kiapi::common::ApiStatusCode::AsBadRequest
        };
        kiapi::common::ApiResponse {
            status: Some(kiapi::common::ApiResponseStatus {
                status: status as i32,
                error_message: String::new(),
            }),
            header: None,
            message: None,
        }
    });
    let ctx = ctx_at(kicad.address().to_string());
    let report = body(
        &handle_get_edit_readiness(&args(&project), &ctx)
            .await
            .unwrap(),
    );
    assert_eq!(
        report["kicad"]["ipc"], "reachable_without_documents",
        "{report}"
    );
    let first = report["guidance"][0].as_str().unwrap();
    assert!(first.contains("lock files only"), "{first}");
    assert!(!first.contains("not answering"));
    assert_eq!(row(&report, "amp.kicad_pcb")["open_in_kicad"], Value::Null);
}
