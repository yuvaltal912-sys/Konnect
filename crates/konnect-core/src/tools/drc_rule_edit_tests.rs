use super::*;
use crate::custom_rules::Operation;
use crate::mcp::handler::McpHandler;

fn success_cli(dir: &Path, effect: &str) -> PathBuf {
    let body=format!("#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n if [ \"$1\" = \"--output\" ]; then shift; out=\"$1\"; fi\n shift\ndone\n{effect}\nprintf '%s' '{{\"violations\":[{{\"type\":\"clearance\",\"description\":\"__konnect_rule_validation_probe__\",\"severity\":\"error\"}}],\"unconnected_items\":[],\"ignored_checks\":[],\"included_severities\":[\"error\",\"warning\",\"exclusion\"]}}' > \"$out\"\n");
    crate::tools::cli::test_support::write_script(
        dir,
        "rule-edit-cli",
        &body,
        "@echo off\r\nexit /b 1\r\n",
    )
}
async fn call(handler: &McpHandler, name: &str, args: Value) -> Value {
    handler.handle_message(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})).await.unwrap().result.unwrap()
}
fn body(response: &Value) -> Value {
    assert_ne!(response["isError"], true, "{response:#}");
    serde_json::from_str(response["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn served_general_rule_create_edit_delete_requires_revisions_and_preserves_other_rules() {
    let d = tempfile::tempdir().unwrap();
    let board = d.path().join("board.kicad_pcb");
    let path = board.with_extension("kicad_dru");
    std::fs::write(&board, FIXTURE).unwrap();
    let prefix =
        "# preserved\r\n(version 1)\r\n(rule \"Other\" (constraint track_width (min 0.2mm)))\r\n";
    std::fs::write(&path, prefix).unwrap();
    let handler = McpHandler::new(crate::tools::ServerConfig {
        kicad_cli: success_cli(d.path(), "").to_string_lossy().into_owned(),
        eager_toolsets: true,
        ..Default::default()
    })
    .await
    .unwrap();
    let mut args = json!({"board":board,"name":"General rule","rule":"(rule \"General rule\" (condition \"A.NetName == 'VCC'\") (constraint clearance (min 0.2mm + 0.1mm)))"});
    for tool in [
        "create_custom_drc_rule",
        "edit_custom_drc_rule",
        "delete_custom_drc_rule",
    ] {
        if tool == "edit_custom_drc_rule" {
            args["rule"] = json!("(rule \"General rule\" (constraint clearance (min 0.4mm)))");
        }
        if tool == "delete_custom_drc_rule" {
            args.as_object_mut().unwrap().remove("rule");
        }
        args.as_object_mut().unwrap().remove("dry_run");
        args.as_object_mut()
            .unwrap()
            .remove("expected_plan_revision");
        let before = std::fs::read(&path).unwrap();
        let preview = body(&call(&handler, tool, args.clone()).await);
        assert_eq!(preview["status"], "ready");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        args["dry_run"] = json!(false);
        let missing = call(&handler, tool, args.clone()).await;
        assert_eq!(missing["isError"], true);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        args["expected_plan_revision"] = preview["plan_revision"].clone();
        let applied = body(&call(&handler, tool, args.clone()).await);
        assert_eq!(applied["status"], "applied");
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.starts_with(prefix));
        if tool != "delete_custom_drc_rule" {
            let replay = call(&handler, tool, args.clone()).await;
            assert_eq!(replay["isError"], true);
        }
    }
    assert_eq!(
        crate::custom_rules::parse(&std::fs::read_to_string(&path).unwrap())
            .unwrap()
            .len(),
        1
    );
    assert_eq!(std::fs::read(&board).unwrap(), FIXTURE);
}

#[tokio::test]
async fn served_rule_schema_and_exact_replacement_identity_refuse_without_writes() {
    let d = tempfile::tempdir().unwrap();
    let board = d.path().join("board.kicad_pcb");
    let path = board.with_extension("kicad_dru");
    std::fs::write(&board, FIXTURE).unwrap();
    let original = "(version 1)(rule \"Original\" (constraint clearance (min 0.2mm)))";
    std::fs::write(&path, original).unwrap();
    let handler = McpHandler::new(crate::tools::ServerConfig {
        kicad_cli: success_cli(d.path(), "").to_string_lossy().into_owned(),
        eager_toolsets: true,
        ..Default::default()
    })
    .await
    .unwrap();
    for (tool, args) in [
        (
            "create_custom_drc_rule",
            json!({"board":board,"name":"New","rule":"(rule \"New\" (constraint clearance (min 0.2mm)))","constraint":"clearance","minimum_mm":0.2}),
        ),
        (
            "edit_custom_drc_rule",
            json!({"board":board,"name":"Original","rule":"(rule \"Renamed\" (constraint clearance (min 0.2mm)))"}),
        ),
        (
            "delete_custom_drc_rule",
            json!({"board":board,"name":"Missing"}),
        ),
    ] {
        let result = call(&handler, tool, args).await;
        assert_eq!(result["isError"], true, "{result:#}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
}

#[tokio::test]
async fn schematic_lock_before_preview_or_appearing_during_validation_prevents_rule_write() {
    for late in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        std::fs::write(&board, FIXTURE).unwrap();
        let board = board.canonicalize().unwrap();
        let lock = konnect_sexp::writer::kicad_editor_lock_path(&board.with_extension("kicad_sch"))
            .unwrap();
        if !late {
            std::fs::write(&lock, "held").unwrap();
        }
        let effect = if late {
            format!("touch '{}'", lock.display())
        } else {
            String::new()
        };
        let cli = success_cli(d.path(), &effect);
        let ctx = ToolContext::new(
            crate::tools::ServerConfig {
                kicad_cli: cli.to_string_lossy().into_owned(),
                ..Default::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        let mut args = json!({"board":board,"name":"New","rule":"(rule \"New\" (constraint clearance (min 0.2mm)))"});
        if late {
            let proposed = crate::custom_rules::change(
                "(version 1)\n",
                "New",
                args["rule"].as_str(),
                Operation::Create,
            )
            .unwrap();
            let plan = target_revision(
                &revision(&board, None, &proposed),
                &std::fs::read_to_string(&board).unwrap(),
                None,
            );
            args["dry_run"] = json!(false);
            args["expected_plan_revision"] = json!(plan);
        }
        let result = handle_rule_change(&args, &ctx, Operation::Create)
            .await
            .unwrap();
        assert!(result.is_error, "{result:?}");
        assert!(lock.exists());
        assert!(!board.with_extension("kicad_dru").exists());
    }
}

#[tokio::test]
async fn no_op_edit_cannot_report_a_stale_rule_after_an_external_write() {
    let d = tempfile::tempdir().unwrap();
    let board = d.path().join("board.kicad_pcb");
    let path = board.with_extension("kicad_dru");
    std::fs::write(&board, FIXTURE).unwrap();
    let board = board.canonicalize().unwrap();
    let rule = "(rule \"Same\" (constraint clearance (min 0.2mm)))";
    let source = format!("(version 1){rule}");
    std::fs::write(&path, &source).unwrap();
    let changed = source.replace("0.2mm", "0.4mm");
    let effect = format!("printf '%s' '{}' > '{}'", changed, path.display());
    let cli = success_cli(d.path(), &effect);
    let ctx = ToolContext::new(
        crate::tools::ServerConfig {
            kicad_cli: cli.to_string_lossy().into_owned(),
            ..Default::default()
        },
        std::sync::Arc::new(crate::router::ToolRouter::new()),
    );
    let plan = target_revision(
        &revision(&board, Some(&source), &source),
        &std::fs::read_to_string(&board).unwrap(),
        None,
    );
    let result=handle_rule_change(&json!({"board":board,"name":"Same","rule":rule,"dry_run":false,"expected_plan_revision":plan}),&ctx,Operation::Edit).await.unwrap();
    assert!(result.is_error, "{result:?}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), changed);
}

#[test]
fn published_create_schema_requires_exactly_one_complete_input_form() {
    let tool = tools()
        .into_iter()
        .find(|t| t.name == "create_custom_drc_rule")
        .unwrap();
    let raw = json!({"board":"board.kicad_pcb","name":"X","rule":"(rule \"X\" (constraint clearance (min 0.2mm)))"});
    let minimum =
        json!({"board":"board.kicad_pcb","name":"X","constraint":"clearance","minimum_mm":0.2});
    assert!(tool.input_validator.is_valid(&raw));
    assert!(tool.input_validator.is_valid(&minimum));
    let mut mixed = raw.clone();
    mixed["constraint"] = json!("clearance");
    mixed["minimum_mm"] = json!(0.2);
    assert!(!tool.input_validator.is_valid(&mixed));
    assert!(!tool
        .input_validator
        .is_valid(&json!({"board":"board.kicad_pcb","name":"X"})));
    let mut partial = minimum;
    partial.as_object_mut().unwrap().remove("minimum_mm");
    assert!(!tool.input_validator.is_valid(&partial));
}
#[tokio::test]
async fn direct_mixed_rule_forms_refuse_even_without_schema_dispatch() {
    let d = tempfile::tempdir().unwrap();
    let board = d.path().join("board.kicad_pcb");
    std::fs::write(&board, FIXTURE).unwrap();
    let cli = success_cli(d.path(), "");
    let ctx = ToolContext::new(
        crate::tools::ServerConfig {
            kicad_cli: cli.to_string_lossy().into_owned(),
            ..Default::default()
        },
        std::sync::Arc::new(crate::router::ToolRouter::new()),
    );
    let args = json!({"board":board,"name":"X","rule":"(rule \"X\" (constraint clearance (min 0.2mm)))","constraint":"clearance","minimum_mm":0.2});
    let result = handle_rule_change(&args, &ctx, Operation::Create)
        .await
        .unwrap();
    assert!(result.is_error, "{result:?}");
    assert!(!board.with_extension("kicad_dru").exists());
}
