//! Project rule and netclass edits independent of the routing toolset.
use super::*;
use anyhow::{bail, ensure, Context, Result};
use konnect_sexp::writer::read_consistent;
use sha2::{Digest, Sha256};

pub(super) fn tools() -> Vec<ToolDef> {
    vec![
 tool!("set_netclass_diff_pair","Preview or set an existing saved netclass's differential-pair width, gap and via gap. Requires a matching preview revision to write. Refuses observed open boards or KiCad lock files.",json!({"type":"object","properties":{"board":{"type":"string"},"name":{"type":"string"},"diff_pair_width":{"type":"number"},"diff_pair_gap":{"type":"number"},"diff_pair_via_gap":{"type":"number"},"dry_run":{"type":"boolean","default":true},"expected_plan_revision":{"type":"string"}},"required":["board","name"],"additionalProperties":false}),|args,ctx|async move{handle_set_netclass_diff_pair(args,ctx).await}),
 tool!("delete_netclass","Preview or delete one netclass with no references in the saved project. Default, ambiguous and referenced classes refuse. Apply requires the preview revision.",json!({"type":"object","properties":{"board":{"type":"string"},"name":{"type":"string"},"dry_run":{"type":"boolean","default":true},"expected_plan_revision":{"type":"string"}},"required":["board","name"],"additionalProperties":false}),|args,ctx|async move{handle_delete_netclass(args,ctx).await})]
}
async fn handle_set_netclass_diff_pair(args: &Value, ctx: &ToolContext) -> Result<CallToolResult> {
    handle(args, ctx, false).await
}
async fn handle_delete_netclass(args: &Value, ctx: &ToolContext) -> Result<CallToolResult> {
    handle(args, ctx, true).await
}

/// Values are in mm. KiCad's saved fields are numeric preferences; this does
/// not claim a clearance, impedance or manufacturing proof.
fn dimensions(args: &Value) -> Result<Vec<(&'static str, f64)>> {
    let mut values = Vec::new();
    for (key, positive) in [
        ("diff_pair_width", true),
        ("diff_pair_gap", false),
        ("diff_pair_via_gap", false),
    ] {
        if let Some(raw) = args.get(key) {
            let value = raw
                .as_f64()
                .with_context(|| format!("{key} must be numeric"))?;
            ensure!(
                value.is_finite()
                    && value <= 100.0
                    && if positive {
                        value >= 0.000001
                    } else {
                        value == 0.0 || value >= 0.000001
                    },
                "{key} is outside the supported range"
            );
            ensure!(
                (value * 1_000_000.0 - (value * 1_000_000.0).round()).abs() < 0.000001,
                "{key} must be a whole number of nanometers"
            );
            values.push((key, (value * 1_000_000.0).round() / 1_000_000.0));
        }
    }
    ensure!(!values.is_empty(), "supply a differential-pair field");
    Ok(values)
}
fn referenced(settings: &Value, name: &str) -> Result<bool> {
    if let Some(patterns) = settings.get("netclass_patterns").filter(|v| !v.is_null()) {
        for entry in patterns
            .as_array()
            .context("netclass_patterns must be an array")?
        {
            let pattern = entry
                .get("pattern")
                .and_then(Value::as_str)
                .context("netclass pattern lacks pattern")?;
            let class = entry
                .get("netclass")
                .and_then(Value::as_str)
                .context("netclass pattern lacks class")?;
            let _ = pattern;
            if class == name {
                return Ok(true);
            }
        }
    }
    if let Some(assignments) = settings
        .get("netclass_assignments")
        .filter(|v| !v.is_null())
    {
        for value in assignments
            .as_object()
            .context("netclass_assignments must be an object")?
            .values()
        {
            match value {
                Value::String(class) => {
                    if class == name {
                        return Ok(true);
                    }
                }
                Value::Array(classes) => {
                    for class in classes {
                        if class.as_str().context("invalid netclass assignment")? == name {
                            return Ok(true);
                        }
                    }
                }
                _ => bail!("invalid netclass assignment"),
            }
        }
    }
    Ok(false)
}
fn plan(source: &str, name: &str, args: &Value, delete: bool) -> Result<(String, bool)> {
    ensure!(
        !name.trim().is_empty() && !name.contains('\0'),
        "netclass name must be nonempty without NUL"
    );
    let mut project = super::project_json_guard::parse(source)?;
    let settings = project
        .get_mut("net_settings")
        .and_then(Value::as_object_mut)
        .context("project has no net_settings object")?;
    if delete {
        ensure!(name != "Default", "the Default netclass cannot be deleted");
        ensure!(
            !referenced(&Value::Object(settings.clone()), name)?,
            "netclass is still referenced by a pattern or assignment"
        );
    }
    let classes = settings
        .get_mut("classes")
        .and_then(Value::as_array_mut)
        .context("net_settings.classes must be an array")?;
    let mut matches = Vec::new();
    for (index, class) in classes.iter().enumerate() {
        let class_name = class
            .get("name")
            .and_then(Value::as_str)
            .context("a netclass has no readable name")?;
        if class_name == name {
            matches.push(index);
        }
    }
    ensure!(matches.len() == 1, "netclass must exist exactly once");
    let index = matches[0];
    if delete {
        classes.remove(index);
    } else {
        let values = dimensions(args)?;
        let target = classes[index]
            .as_object_mut()
            .context("netclass is not an object")?;
        for (key, value) in values {
            target.insert(key.into(), json!(value));
        }
    }
    let result = format!("{}\n", serde_json::to_string_pretty(&project)?);
    let original: Value = super::project_json_guard::parse(source)?;
    Ok((result, project != original))
}
fn revision(board: &Path, source: &str, candidate: &str) -> String {
    let mut hash = Sha256::new();
    for bytes in [
        board.as_os_str().as_encoded_bytes(),
        source.as_bytes(),
        candidate.as_bytes(),
    ] {
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}
async fn handle(args: &Value, ctx: &ToolContext, delete: bool) -> Result<CallToolResult> {
    let board = get_path(args, "board")?;
    let name = args["name"].as_str().context("name must be a string")?;
    let dry = match args.get("dry_run") {
        None => true,
        Some(Value::Bool(v)) => *v,
        Some(_) => return Ok(CallToolResult::error("dry_run must be boolean")),
    };
    let board = board.canonicalize()?;
    if let Some(r) = super::drc_rule_writer::board_lock_refusal(&board) {
        return Ok(r);
    }
    if let Some(r) = super::project_json_guard::schematic_lock_refusal(&board) {
        return Ok(r);
    }
    if let Some(r) =
        crate::tools::pcb_board::refuse_if_board_open_in_kicad(ctx, &board, "netclass change")
            .await?
    {
        return Ok(r);
    }
    ensure!(
        board
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("kicad_pcb")),
        "expected a .kicad_pcb board"
    );
    let board_source = read_consistent(&board)?;
    let parsed = konnect_sexp::parse_sexp(&board_source)?;
    ensure!(
        parsed.head() == Some("kicad_pcb"),
        "board root is not kicad_pcb"
    );
    let path = board.with_extension("kicad_pro");
    let source = super::project_json_guard::project_source(&path)?;
    let (candidate, changed) = match plan(&source, name, args, delete) {
        Ok(v) => v,
        Err(e) => return Ok(CallToolResult::error(e.to_string())),
    };
    let token = revision(&board, &source, &candidate);
    if !dry && args["expected_plan_revision"].as_str() != Some(token.as_str()) {
        return Ok(CallToolResult::error(
            "expected_plan_revision does not match; preview again",
        ));
    }
    if !dry && changed {
        if let Some(r) =
            crate::tools::pcb_board::refuse_if_board_open_in_kicad(ctx, &board, "netclass change")
                .await?
        {
            return Ok(r);
        }
        if let Some(r) = super::drc_rule_writer::board_lock_refusal(&board) {
            return Ok(r);
        }
        if let Some(r) = super::project_json_guard::schematic_lock_refusal(&board) {
            return Ok(r);
        }
        if let Err(e) = super::project_json_guard::write_verified(&path, &source, &candidate) {
            return Ok(super::project_json_guard::uncertain(
                &path,
                "edit_netclass",
                e.to_string(),
            ));
        }
    }
    Ok(CallToolResult::json(
        &json!({"status":if !changed{"noop"}else if dry{"ready"}else{"applied"},"plan_revision":token,"project":path.to_string_lossy(),"project_path_lossy":path.to_str().is_none(),"name":name,"operation":if delete{"delete"}else{"set_diff_pair"},"source":"saved_project","live_netclass_validation":"not_run","schematic_connectivity":"not_inspected"}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    const PCB: &[u8] = include_bytes!("../../tests/fixtures/specctra_two_resistors.kicad_pcb");
    fn source() -> String {
        let mut project: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/multichannel_mixer.kicad_pro"
        ))
        .unwrap();
        project["net_settings"]["classes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"Power","track_width":0.5,"custom":"preserve"}));
        project["net_settings"]["netclass_patterns"] = json!([]);
        project["net_settings"]["netclass_assignments"] = Value::Null;
        serde_json::to_string(&project).unwrap()
    }
    #[test]
    fn accepted_float_noise_is_normalized_to_nanometers() {
        let values = dimensions(&json!({"diff_pair_gap": 0.2000000000001})).unwrap();
        assert_eq!(values[0].1, 0.2);
    }
    #[test]
    fn edits_only_requested_fields_and_keeps_other_data() {
        let src = source();
        let (output, changed) = plan(
            &src,
            "Power",
            &json!({"diff_pair_width":0.3,"diff_pair_gap":0.2,"diff_pair_via_gap":0.4}),
            false,
        )
        .unwrap();
        assert!(changed);
        let before: Value = serde_json::from_str(&src).unwrap();
        let after: Value = serde_json::from_str(&output).unwrap();
        let class = after["net_settings"]["classes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "Power")
            .unwrap();
        assert_eq!(class["track_width"], 0.5);
        assert_eq!(class["custom"], "preserve");
        assert_eq!(class["diff_pair_width"], 0.3);
        assert_eq!(class["diff_pair_gap"], 0.2);
        assert_eq!(class["diff_pair_via_gap"], 0.4);
        assert_eq!(before["board"], after["board"]);
        assert!(
            !plan(&output, "Power", &json!({"diff_pair_width":0.3}), false)
                .unwrap()
                .1
        );
    }
    #[test]
    fn invalid_names_refuse_even_if_class_exists() {
        for name in ["", "bad\0name"] {
            let mut project: Value = serde_json::from_str(&source()).unwrap();
            project["net_settings"]["classes"]
                .as_array_mut()
                .unwrap()
                .push(json!({"name":name}));
            assert!(plan(&project.to_string(), name, &json!({}), true).is_err());
        }
    }
    #[test]
    fn unsafe_names_values_duplicates_and_references_refuse() {
        let src = source();
        assert!(plan(&src, "Default", &json!({}), true).is_err());
        assert!(plan(&src, "Missing", &json!({}), true).is_err());
        assert!(plan(&src, "", &json!({}), true).is_err());
        assert!(plan(&src, "bad\0name", &json!({}), true).is_err());
        assert!(plan(&src, "Power", &json!({}), false).is_err());
        for v in [
            json!(null),
            json!(false),
            json!(-1),
            json!(101),
            json!(0.0000011),
        ] {
            assert!(plan(&src, "Power", &json!({"diff_pair_width":v}), false).is_err());
        }
        assert!(plan(&src, "Power", &json!({"diff_pair_width":0}), false).is_err());
        assert!(plan(&src, "Power", &json!({"diff_pair_gap":0}), false).is_ok());
        let mut p: Value = serde_json::from_str(&src).unwrap();
        p["net_settings"]["classes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"Power"}));
        assert!(plan(&p.to_string(), "Power", &json!({}), true).is_err());
        let mut p: Value = serde_json::from_str(&src).unwrap();
        p["net_settings"]["netclass_patterns"] = json!([{"pattern":"VCC","netclass":"Power"}]);
        assert!(plan(&p.to_string(), "Power", &json!({}), true).is_err());
        p["net_settings"]["netclass_patterns"] = json!([]);
        p["net_settings"]["netclass_assignments"] = json!({"VCC":["Power"]});
        assert!(plan(&p.to_string(), "Power", &json!({}), true).is_err());
        p["net_settings"]["netclass_assignments"] = json!({"VCC":[1]});
        assert!(plan(&p.to_string(), "Power", &json!({}), true).is_err());
    }
    #[test]
    fn token_binds_target_original_and_candidate() {
        let p = Path::new("/tmp/one.kicad_pcb");
        assert_ne!(revision(p, "a", "bc"), revision(p, "ab", "c"));
        assert_ne!(
            revision(p, "a", "x"),
            revision(Path::new("/tmp/two.kicad_pcb"), "a", "x")
        );
        assert_ne!(revision(p, "a", "x"), revision(p, "b", "x"));
    }
    #[tokio::test]
    async fn preview_revision_apply_and_delete_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        let project = board.with_extension("kicad_pro");
        std::fs::write(&board, PCB).unwrap();
        let src = source();
        std::fs::write(&project, &src).unwrap();
        let ctx = ToolContext::new(
            Default::default(),
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        let mut args = json!({"board":board,"name":"Power","diff_pair_width":0.3,"diff_pair_gap":0.2,"diff_pair_via_gap":0.4});
        let preview = handle(&args, &ctx, false).await.unwrap();
        assert!(!preview.is_error);
        assert_eq!(std::fs::read_to_string(&project).unwrap(), src);
        let crate::mcp::protocol::ToolContent::Text { text } = &preview.content[0] else {
            panic!()
        };
        let body: Value = serde_json::from_str(text).unwrap();
        args["dry_run"] = json!(false);
        assert!(handle(&args, &ctx, false).await.unwrap().is_error);
        args["expected_plan_revision"] = body["plan_revision"].clone();
        let result = handle(&args, &ctx, false).await.unwrap();
        assert!(!result.is_error, "{result:?}");
        let after = std::fs::read_to_string(&project).unwrap();
        assert!(handle(&args, &ctx, false).await.unwrap().is_error);
        assert_eq!(std::fs::read_to_string(&project).unwrap(), after);
        assert_eq!(std::fs::read(&board).unwrap(), PCB);
        let mut delete = json!({"board":board,"name":"Power"});
        let preview = handle(&delete, &ctx, true).await.unwrap();
        let crate::mcp::protocol::ToolContent::Text { text } = &preview.content[0] else {
            panic!()
        };
        let body: Value = serde_json::from_str(text).unwrap();
        delete["dry_run"] = json!(false);
        delete["expected_plan_revision"] = body["plan_revision"].clone();
        let result = handle(&delete, &ctx, true).await.unwrap();
        assert!(!result.is_error, "{result:?}");
        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(project).unwrap()).unwrap();
        assert!(!saved["net_settings"]["classes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "Power"));
    }

    #[tokio::test]
    async fn open_board_and_schematic_lock_refuse_project_edits() {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        let project = board.with_extension("kicad_pro");
        std::fs::write(&board, PCB).unwrap();
        let src = source();
        std::fs::write(&project, &src).unwrap();
        let server =
            crate::tools::pcb_board::board_mock::spawn_kicad_holding_board(&board, |_| None);
        let ctx = crate::tools::pcb_board::board_mock::ctx_talking_to(server.address().into());
        for delete in [false, true] {
            let result = handle(
                &json!({"board":board,"name":"Power","diff_pair_width":0.3}),
                &ctx,
                delete,
            )
            .await
            .unwrap();
            assert!(result.is_error);
            assert_eq!(std::fs::read_to_string(&project).unwrap(), src);
        }
        drop(server);
        let lock = konnect_sexp::writer::kicad_editor_lock_path(&board.with_extension("kicad_sch"))
            .unwrap();
        std::fs::write(&lock, "held").unwrap();
        let ctx = ToolContext::new(
            Default::default(),
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        let result = handle(
            &json!({"board":board,"name":"Power","diff_pair_width":0.3}),
            &ctx,
            false,
        )
        .await
        .unwrap();
        assert!(result.is_error);
        assert_eq!(std::fs::read_to_string(project).unwrap(), src);
    }
    #[tokio::test]
    async fn schematic_lock_appearing_before_write_refuses_project_edits() {
        use konnect_ipc::builders::pack_any;
        use konnect_ipc::gen::kiapi;
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        for netclass in [false, true] {
            let d = tempfile::tempdir().unwrap();
            let board = d.path().join("board.kicad_pcb");
            let project = board.with_extension("kicad_pro");
            std::fs::write(&board, PCB).unwrap();
            let src = source();
            std::fs::write(&project, &src).unwrap();
            let board = board.canonicalize().unwrap();
            let lock =
                konnect_sexp::writer::kicad_editor_lock_path(&board.with_extension("kicad_sch"))
                    .unwrap();
            let lock_to_create = lock.clone();
            let count = Arc::new(AtomicUsize::new(0));
            let server =
                crate::test_support::MockIpcServer::spawn("project-lock-race", move |request| {
                    let cmd = request.message.unwrap();
                    let message = if cmd.type_url.ends_with("GetOpenDocuments") {
                        if count.fetch_add(1, Ordering::SeqCst) == 1 {
                            std::fs::write(&lock_to_create, "held").unwrap();
                        }
                        Some(pack_any(
                            &kiapi::common::commands::GetOpenDocumentsResponse {
                                documents: vec![],
                            },
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
                });
            let ctx = crate::tools::pcb_board::board_mock::ctx_talking_to(server.address().into());
            let result = if netclass {
                let mut args =
                    json!({"board":board,"name":"Power","diff_pair_width":0.3,"dry_run":false});
                let candidate = plan(&src, "Power", &args, false).unwrap().0;
                args["expected_plan_revision"] = json!(revision(&board, &src, &candidate));
                handle(&args, &ctx, false).await.unwrap()
            } else {
                super::super::handle_set_design_rules(
                    &json!({"board":board,"min_via_annular_width":0.2}),
                    &ctx,
                )
                .await
                .unwrap()
            };
            assert!(lock.exists());
            assert!(result.is_error, "{result:?}");
            assert_eq!(std::fs::read_to_string(&project).unwrap(), src);
        }
    }
    #[tokio::test]
    async fn served_netclass_preview_is_registered_and_nonmutating() {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        let project = board.with_extension("kicad_pro");
        std::fs::write(&board, PCB).unwrap();
        let src = source();
        std::fs::write(&project, &src).unwrap();
        let handler = crate::mcp::handler::McpHandler::new(crate::tools::ServerConfig {
            eager_toolsets: true,
            ..Default::default()
        })
        .await
        .unwrap();
        let response=handler.handle_message(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"set_netclass_diff_pair","arguments":{"board":board,"name":"Power","diff_pair_width":0.3}}})).await.unwrap().result.unwrap();
        assert_ne!(response["isError"], true, "{response:#}");
        let body: Value =
            serde_json::from_str(response["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["status"], "ready");
        assert_eq!(body["schematic_connectivity"], "not_inspected");
        assert_eq!(std::fs::read_to_string(project).unwrap(), src);
    }
}
