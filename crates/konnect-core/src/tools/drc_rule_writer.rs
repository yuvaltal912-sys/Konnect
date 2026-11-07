//! Named custom rules: native compilation, revision-bound edits and saved readback.
use super::*;
use anyhow::{ensure, Context, Result};
use konnect_sexp::writer::{read_consistent, write_atomic_if_unchanged, write_new_atomic};
use sha2::{Digest, Sha256};

const PROBE: &str = crate::custom_rules::PROBE;
const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/specctra_two_resistors.kicad_pcb");

pub(super) fn tools() -> Vec<ToolDef> {
    let mut schema = json!({"type":"object","properties":{"board":{"type":"string"},"name":{"type":"string"},"constraint":{"type":"string","enum":["clearance","track_width"]},"minimum_mm":{"type":"number"},"rule":{"type":"string","description":"One complete KiCad (rule ...) statement, with the same name. Conditions, assertions and numeric expressions are compiled by KiCad."},"dry_run":{"type":"boolean","default":true},"expected_plan_revision":{"type":"string"}},"required":["board","name"],"additionalProperties":false});
    schema["oneOf"] = json!([{"required":["rule"],"not":{"anyOf":[{"required":["constraint"]},{"required":["minimum_mm"]}]}},{"required":["constraint","minimum_mm"],"not":{"required":["rule"]}}]);
    let create=tool!("create_custom_drc_rule", "Preview or create a named custom DRC rule. Supply one complete KiCad rule or the clearance/track-width minimum shorthand. KiCad compiles the complete candidate and every expression on a private fixture, then DRC runs on a scratch copy of the saved board and project. Apply requires its exact preview revision.", schema, |args,ctx|async move{handle_create_custom_drc_rule(args,ctx).await});
    let edit_schema = json!({"type":"object","properties":{"board":{"type":"string"},"name":{"type":"string"},"rule":{"type":"string"},"dry_run":{"type":"boolean","default":true},"expected_plan_revision":{"type":"string"}},"required":["board","name","rule"],"additionalProperties":false});
    let mut delete_schema = edit_schema.clone();
    delete_schema["properties"]
        .as_object_mut()
        .unwrap()
        .remove("rule");
    delete_schema["required"] = json!(["board", "name"]);
    vec![create,
    tool!("edit_custom_drc_rule","Preview or replace exactly one named custom DRC rule in place, preserving other rules, comments and order. Requires native scratch validation and the preview revision before writing.",edit_schema,|args,ctx|async move{handle_rule_change(args,ctx,crate::custom_rules::Operation::Edit).await}),
    tool!("delete_custom_drc_rule","Preview or delete exactly one named custom DRC rule, preserving other rules and comments. Validates the remaining rules and runs scratch DRC; apply requires the preview revision.",delete_schema,|args,ctx|async move{handle_rule_change(args,ctx,crate::custom_rules::Operation::Delete).await})]
}

fn candidate(source: &str, name: &str, constraint: &str, minimum: f64) -> Result<String> {
    ensure!(
        !name.is_empty()
            && name.len() <= 80
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)),
        "name must contain only ASCII letters, numbers, underscore, dot or hyphen"
    );
    ensure!(name != PROBE, "name is reserved for validation");
    ensure!(
        matches!(constraint, "clearance" | "track_width"),
        "unsupported constraint"
    );
    ensure!(
        minimum.is_finite() && (0.000001..=100.0).contains(&minimum),
        "minimum must be 1 nm to 100 mm"
    );
    ensure!(
        (minimum * 1_000_000.0 - (minimum * 1_000_000.0).round()).abs() < 0.000001,
        "minimum must be an exact number of nanometers"
    );
    crate::custom_rules::change(
        source,
        name,
        Some(&format!(
            "(rule \"{}\" (constraint {} (min {:.9}mm)))",
            name, constraint, minimum
        )),
        crate::custom_rules::Operation::Create,
    )
}

fn revision(board: &Path, original: Option<&str>, result: &str) -> String {
    let mut h = Sha256::new();
    for part in [
        board.as_os_str().as_encoded_bytes(),
        original.unwrap_or("").as_bytes(),
        result.as_bytes(),
    ] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part);
    }
    h.update([original.is_some() as u8]);
    format!("{:x}", h.finalize())
}
fn target_revision(base: &str, board_source: &str, project: Option<&str>) -> String {
    let mut h = Sha256::new();
    for part in [
        base.as_bytes(),
        board_source.as_bytes(),
        project.unwrap_or("").as_bytes(),
    ] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part);
    }
    h.update([project.is_some() as u8]);
    format!("{:x}", h.finalize())
}
fn regular_target(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.file_type().is_file(),
                "project/rule path must be a regular file, not a symlink or directory"
            );
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}
fn read_optional(path: &Path) -> Result<Option<String>> {
    if regular_target(path)? {
        Ok(Some(read_consistent(path)?))
    } else {
        Ok(None)
    }
}
fn read_project(path: &Path) -> Result<Option<String>> {
    let source = read_optional(path)?;
    if let Some(source) = &source {
        super::project_json_guard::parse(source)
            .context("saved project is not unambiguous JSON")?;
    }
    Ok(source)
}
pub(crate) async fn target_drc(
    cli: &str,
    board_source: &str,
    project: Option<&str>,
    rules: &str,
) -> Result<Value> {
    target_drc_with_timeout(
        cli,
        board_source,
        project,
        rules,
        std::time::Duration::from_secs(60),
    )
    .await
}
async fn target_drc_with_timeout(
    cli: &str,
    board_source: &str,
    project: Option<&str>,
    rules: &str,
    timeout: std::time::Duration,
) -> Result<Value> {
    let temp = tempfile::tempdir()?;
    let board = temp.path().join("target.kicad_pcb");
    let output = temp.path().join("target-report.json");
    tokio::fs::write(&board, board_source).await?;
    tokio::fs::write(
        temp.path().join("target.kicad_pro"),
        project.unwrap_or("{}\n"),
    )
    .await?;
    tokio::fs::write(temp.path().join("target.kicad_dru"), rules).await?;
    let mut cmd = tokio::process::Command::new(crate::tools::cli::resolve_cli_executable(cli));
    cmd.args([
        "pcb",
        "drc",
        "--severity-all",
        "--format",
        "json",
        "--output",
    ])
    .arg(&output)
    .arg(&board)
    .kill_on_drop(true);
    let run = tokio::time::timeout(timeout, cmd.output())
        .await
        .context("target DRC timed out")??;
    ensure!(
        run.status.success(),
        "target DRC command failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let report: Value = serde_json::from_slice(
        &tokio::fs::read(output)
            .await
            .context("target DRC did not write a report")?,
    )?;
    target_summary(&report, project.is_some())
}
fn target_summary(report: &Value, has_project: bool) -> Result<Value> {
    let violations = report["violations"]
        .as_array()
        .context("target DRC report has no violations array")?;
    let unconnected = report["unconnected_items"]
        .as_array()
        .context("target DRC report has no unconnected_items array")?;
    let mut errors = 0;
    let mut warnings = 0;
    let mut other = 0;
    for item in violations {
        match item["severity"].as_str() {
            Some("error") => errors += 1,
            Some("warning") => warnings += 1,
            _ => other += 1,
        }
    }
    Ok(
        json!({"status":"checked","source":"scratch_copy_of_saved_board","project_source":if has_project{"copied_saved_project"}else{"synthetic_default_project"},"violations":violations.len(),"error_violations":errors,"warning_violations":warnings,"other_severity_violations":other,"severity_coverage":if other==0{"complete"}else{"incomplete"},"unconnected_items":unconnected.len(),"included_severities":report["included_severities"].clone(),"ignored_checks_count":report["ignored_checks"].as_array().map(|v|v.len()),"ignored_check_keys":report["ignored_checks"].as_array().map(|items|items.iter().map(|item|item["key"].as_str()).collect::<Vec<_>>()),"schematic_parity":"not_requested","zones_refilled":false,"external_libraries":"not_copied"}),
    )
}
fn witness(report: &serde_json::Value) -> bool {
    report["violations"].as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item["type"] == "clearance"
                && item["description"]
                    .as_str()
                    .is_some_and(|text| text.contains(PROBE))
        })
    })
}
pub(crate) async fn validate(cli: &str, content: &str) -> Result<()> {
    validate_with_timeout(cli, content, std::time::Duration::from_secs(60)).await
}
async fn validate_with_timeout(
    cli: &str,
    content: &str,
    timeout: std::time::Duration,
) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let board = temp.path().join("probe.kicad_pcb");
    let output = temp.path().join("report.json");
    tokio::fs::write(&board, FIXTURE).await?;
    tokio::fs::write(temp.path().join("probe.kicad_pro"), "{}\n").await?;
    let condition = crate::custom_rules::quote(&crate::custom_rules::probe_condition(content)?);
    let rules = format!(
        "{content}\n(rule \"{PROBE}\" (condition {condition}) (constraint clearance (min 50mm)))\n"
    );
    tokio::fs::write(temp.path().join("probe.kicad_dru"), rules).await?;
    let mut cmd = tokio::process::Command::new(crate::tools::cli::resolve_cli_executable(cli));
    cmd.args([
        "pcb",
        "drc",
        "--severity-all",
        "--format",
        "json",
        "--output",
    ])
    .arg(&output)
    .arg(&board)
    .kill_on_drop(true);
    let result = tokio::time::timeout(timeout, cmd.output())
        .await
        .context("KiCad validation timed out")??;
    ensure!(
        result.status.success(),
        "KiCad validation command failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(
        &tokio::fs::read(output)
            .await
            .context("KiCad did not write a DRC report")?,
    )?;
    ensure!(
        witness(&report),
        "KiCad did not demonstrate that the candidate rules loaded"
    );
    Ok(())
}
async fn handle_create_custom_drc_rule(args: &Value, ctx: &ToolContext) -> Result<CallToolResult> {
    handle_rule_change(args, ctx, crate::custom_rules::Operation::Create).await
}
async fn handle_rule_change(
    args: &Value,
    ctx: &ToolContext,
    operation: crate::custom_rules::Operation,
) -> Result<CallToolResult> {
    use crate::custom_rules::Operation;
    let board = get_path(args, "board")?;
    let name = args["name"].as_str().context("name is required")?;
    let operation_name = match operation {
        Operation::Create => "create_custom_drc_rule",
        Operation::Edit => "edit_custom_drc_rule",
        Operation::Delete => "delete_custom_drc_rule",
    };
    let minimum = args["minimum_mm"].as_f64();
    let constraint = args["constraint"].as_str();
    let dry = match args.get("dry_run") {
        None => true,
        Some(Value::Bool(v)) => *v,
        Some(_) => return Ok(CallToolResult::error("dry_run must be boolean")),
    };
    let board = board.canonicalize()?;
    if let Some(refusal) = board_lock_refusal(&board) {
        return Ok(refusal);
    }
    if let Some(r) = super::project_json_guard::schematic_lock_refusal(&board) {
        return Ok(r);
    }
    if let Some(r) =
        crate::tools::pcb_board::refuse_if_board_open_in_kicad(ctx, &board, "custom DRC rule")
            .await?
    {
        return Ok(r);
    }
    ensure!(
        board
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("kicad_pcb")),
        "board must have the .kicad_pcb extension"
    );
    let board_source = read_consistent(&board)?;
    let root = konnect_sexp::parse_sexp(&board_source).context("saved board cannot be parsed")?;
    ensure!(
        root.head() == Some("kicad_pcb"),
        "saved board must have a kicad_pcb root"
    );
    let project_path = board.with_extension("kicad_pro");
    let project = read_project(&project_path)?;
    let path = board.with_extension("kicad_dru");
    let original = read_optional(&path)?;
    let source = original.as_deref().unwrap_or("(version 1)\n");
    let proposed = match (|| -> Result<String> {
        if matches!(operation, Operation::Create) && args.get("rule").is_none() {
            return candidate(
                source,
                name,
                constraint.context("constraint is required")?,
                minimum.context("minimum_mm is required")?,
            );
        }
        ensure!(
            args.get("constraint").is_none() && args.get("minimum_mm").is_none(),
            "raw rules cannot be combined with minimum shorthand"
        );
        let raw = if matches!(operation, Operation::Delete) {
            None
        } else {
            Some(args["rule"].as_str().context("rule must be text")?)
        };
        crate::custom_rules::change(source, name, raw, operation)
    })() {
        Ok(s) => s,
        Err(e) => return Ok(CallToolResult::error(e.to_string())),
    };
    let changed = proposed != source;
    let plan = target_revision(
        &revision(&board, original.as_deref(), &proposed),
        &board_source,
        project.as_deref(),
    );
    if !dry && args["expected_plan_revision"].as_str() != Some(plan.as_str()) {
        return Ok(CallToolResult::error(
            "expected_plan_revision does not match; run a fresh preview",
        ));
    }
    if let Err(error) = validate(&ctx.config.kicad_cli, &proposed).await {
        return Ok(CallToolResult::error(format!(
            "scratch KiCad validation failed: {error:#}"
        )));
    }
    let target_report = match target_drc(
        &ctx.config.kicad_cli,
        &board_source,
        project.as_deref(),
        &proposed,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            return Ok(CallToolResult::error(format!(
                "scratch target DRC failed: {e:#}"
            )))
        }
    };
    if !dry {
        if let Some(r) =
            crate::tools::pcb_board::refuse_if_board_open_in_kicad(ctx, &board, "custom DRC rule")
                .await?
        {
            return Ok(r);
        }
        if let Some(refusal) = board_lock_refusal(&board) {
            return Ok(refusal);
        }
        if let Some(r) = super::project_json_guard::schematic_lock_refusal(&board) {
            return Ok(r);
        }
        if read_consistent(&board)? != board_source || read_project(&project_path)? != project {
            return Ok(CallToolResult::error(
                "saved board or project changed during validation; run a fresh preview",
            ));
        }
        regular_target(&path)?;
        if !changed {
            if read_optional(&path)? != original {
                return Ok(CallToolResult::error_kind(
                    crate::mcp::error::ToolErrorKind::Conflict {
                        paths: vec![path.display().to_string()],
                    },
                    "saved rule file changed during no-op validation; preview again",
                ));
            }
        } else {
            let result = match &original {
                Some(before) => write_atomic_if_unchanged(&path, before, &proposed),
                None => write_new_atomic(&path, &proposed),
            };
            if let Err(error) = result {
                return Ok(uncertain(
                    &path,
                    operation_name,
                    format!("write could not be verified: {error}"),
                ));
            }
            if !saved_matches(&path, &proposed) {
                return Ok(uncertain(
                    &path,
                    operation_name,
                    "saved rule file differs from the validated candidate".into(),
                ));
            }
        }
    }
    Ok(CallToolResult::json(
        &json!({"status":if !changed {"noop"}else if dry {"ready"}else{"applied"},"plan_revision":plan,"rules_file":path.to_string_lossy(),"rules_file_lossy":path.to_str().is_none(),"operation":operation_name,"unrelated_rules_preserved":true,"readback":if dry{"not_written"}else{"saved_bytes_match_candidate"},"validation":"scratch_kicad_drc_probe","target_board_drc":target_report,"rule":{"name":name,"constraint":constraint,"minimum_mm":minimum.map(|m|(m*1_000_000.0).round()/1_000_000.0)}}),
    ))
}

fn saved_matches(path: &Path, expected: &str) -> bool {
    read_consistent(path).ok().as_deref() == Some(expected)
}

fn board_lock_refusal_with<F>(board: &Path, inspect: F) -> Option<CallToolResult>
where
    F: FnOnce(&Path) -> std::io::Result<std::fs::Metadata>,
{
    let lock = konnect_sexp::writer::kicad_editor_lock_path(board)?;
    match inspect(&lock) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Ok(_) => Some(CallToolResult::error(format!(
            "KiCad board lock exists at {}; close the board before editing rules",
            lock.display()
        ))),
        Err(error) => Some(CallToolResult::error(format!(
            "cannot inspect KiCad board lock {}: {error}",
            lock.display()
        ))),
    }
}
pub(super) fn board_lock_refusal(board: &Path) -> Option<CallToolResult> {
    board_lock_refusal_with(board, |path| std::fs::symlink_metadata(path))
}

fn uncertain(path: &Path, operation: &str, reason: String) -> CallToolResult {
    CallToolResult::error_kind(crate::mcp::error::ToolErrorKind::MutationOutcomeUncertain {operation:operation.into(),path:path.display().to_string(),reason:reason.clone()},format!("The rule-file write could not be verified: {reason}. Inspect the saved file before retrying."))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_plan(board: &Path, original: Option<&str>, proposed: &str) -> String {
        let board = board.canonicalize().unwrap();
        let content = read_consistent(&board).unwrap();
        let project = read_project(&board.with_extension("kicad_pro")).unwrap();
        target_revision(
            &revision(&board, original, proposed),
            &content,
            project.as_deref(),
        )
    }
    #[test]
    fn inputs_refuse_injection_duplicates_and_unsafe_values() {
        let src = "(version 1)\n(rule \"old\" (constraint clearance (min 0.200000000mm)))\n";
        assert!(candidate(src, "", "clearance", 0.2).is_err());
        assert!(candidate(src, &"a".repeat(81), "clearance", 0.2).is_err());
        assert!(candidate(src, "bad\"name", "clearance", 0.2).is_err());
        assert!(candidate("(version 1)\n)", "new", "clearance", 0.2).is_err());
        let duplicated="(version 1)\n(rule \"old\" (constraint clearance (min 0.200000000mm)))\n(rule \"old\" (constraint clearance (min 0.200000000mm)))\n";
        assert!(candidate(duplicated, "new", "clearance", 0.2).is_err());
        assert!(candidate("# header\n(version 1)\n(rule \"existing label\" (condition \"A.NetName == 'VCC'\") (constraint clearance (min 0.2mm)))", "new", "clearance", 0.2).is_ok());
        assert!(candidate("(version 1)\n", PROBE, "clearance", 0.2).is_err());
        assert!(candidate(src, "old", "clearance", 0.2).is_err());
        assert!(candidate(src, "new", "unknown", 0.2).is_err());
        assert!(candidate("(version 1)\n", "new", "clearance", 0.0000011).is_err());
        for n in [0.0, -1.0, 101.0, f64::NAN] {
            assert!(candidate(src, "new", "clearance", n).is_err());
        }
        assert!(candidate(
            "(version 1)\n__konnect_rule_validation_probe__",
            "new",
            "clearance",
            0.2
        )
        .is_err());
        assert!(candidate("", "new", "clearance", 0.2).is_err());
        assert!(candidate(src, "new", "clearance", 0.2)
            .unwrap()
            .starts_with(src.trim_end()));
    }
    #[test]
    fn validation_witness_must_be_actual_probe_clearance() {
        let good = json!({"violations":[{"type":"clearance","description":format!("Clearance violation (rule '{PROBE}' clearance 50.0000 mm)")} ]});
        assert!(witness(&good));
        assert!(!witness(&json!({"violations":[]})));
        assert!(!witness(
            &json!({"violations":[{"type":"track_width","description":format!("rule '{PROBE}'")}]})
        ));
        assert!(!witness(
            &json!({"violations":[{"type":"clearance","description":"another rule"}]})
        ));
    }
    #[test]
    fn revision_binds_existing_state_and_candidate() {
        let b = Path::new("/tmp/x.kicad_pcb");
        assert_ne!(revision(b, None, "x"), revision(b, Some(""), "x"));
        assert_ne!(revision(b, Some("a"), "x"), revision(b, Some("b"), "x"));
        assert_ne!(revision(b, None, "x"), revision(b, None, "y"));
        assert_ne!(
            revision(b, None, "x"),
            revision(Path::new("/tmp/y.kicad_pcb"), None, "x")
        );
        assert_ne!(revision(b, Some("a"), "bc"), revision(b, Some("ab"), "c"));
        assert_ne!(
            target_revision("base-a", "board", None),
            target_revision("base-b", "board", None)
        );
        assert_ne!(
            target_revision("base", "board-a", None),
            target_revision("base", "board-b", None)
        );
        assert_ne!(
            target_revision("base", "board", None),
            target_revision("base", "board", Some(""))
        );
        assert_ne!(
            target_revision("base", "board", Some("a")),
            target_revision("base", "board", Some("b"))
        );
        assert_ne!(
            target_revision("base", "a", Some("bc")),
            target_revision("base", "ab", Some("c"))
        );

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let x = std::path::PathBuf::from(std::ffi::OsString::from_vec(vec![b'/', b'x', 0x80]));
            let y = std::path::PathBuf::from(std::ffi::OsString::from_vec(vec![b'/', b'x', 0x81]));
            assert_eq!(x.to_string_lossy(), y.to_string_lossy());
            assert_ne!(revision(&x, None, "x"), revision(&y, None, "x"));
        }
    }
    #[tokio::test]
    #[ignore = "requires real KiCad CLI"]
    async fn real_kicad_probe_rejects_a_malformed_candidate() {
        let cli = "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli";
        let good = candidate("(version 1)\n", "example", "clearance", 0.2).unwrap();
        validate(cli, &good).await.unwrap();
        let bad = format!("{good}\n(rule \"bad\" (constraint invalid_type (min 0.2mm)))\n");
        assert!(validate(cli, &bad).await.is_err());
    }

    #[tokio::test]
    #[ignore = "requires real KiCad CLI"]
    async fn preview_apply_and_target_rule_are_verified_on_scratch() {
        let dir = tempfile::tempdir().unwrap();
        let board = dir.path().join("board.kicad_pcb");
        let rules = board.with_extension("kicad_dru");
        std::fs::write(&board, FIXTURE).unwrap();
        std::fs::write(board.with_extension("kicad_pro"), "{}\n").unwrap();
        let ctx = ToolContext::new(
            crate::tools::ServerConfig {
                kicad_cli: "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli".into(),
                ..Default::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        let mut args =
            json!({"board":board,"name":"example","constraint":"clearance","minimum_mm":2.0});
        let preview = handle_create_custom_drc_rule(&args, &ctx).await.unwrap();
        assert!(!preview.is_error, "{preview:?}");
        assert!(!rules.exists());
        let crate::mcp::protocol::ToolContent::Text { text } = &preview.content[0] else {
            panic!()
        };
        let plan: Value = serde_json::from_str(text).unwrap();
        assert_eq!(plan["status"], "ready");
        assert_eq!(plan["target_board_drc"]["status"], "checked");
        assert!(
            plan["target_board_drc"]["error_violations"]
                .as_u64()
                .unwrap()
                >= 1
        );

        args["dry_run"] = json!(false);
        let refused = handle_create_custom_drc_rule(&args, &ctx).await.unwrap();
        assert!(refused.is_error);
        assert!(!rules.exists());
        args["expected_plan_revision"] = plan["plan_revision"].clone();
        std::fs::write(
            &board,
            format!("{}\n", std::str::from_utf8(FIXTURE).unwrap()),
        )
        .unwrap();
        assert!(
            handle_create_custom_drc_rule(&args, &ctx)
                .await
                .unwrap()
                .is_error
        );
        assert!(!rules.exists());
        std::fs::write(&board, FIXTURE).unwrap();
        let project = board.with_extension("kicad_pro");
        std::fs::write(&project, "{\"changed\":true}\n").unwrap();
        assert!(
            handle_create_custom_drc_rule(&args, &ctx)
                .await
                .unwrap()
                .is_error
        );
        assert!(!rules.exists());
        std::fs::write(&project, "{}\n").unwrap();
        let applied = handle_create_custom_drc_rule(&args, &ctx).await.unwrap();
        assert!(!applied.is_error, "{applied:?}");
        let bytes = std::fs::read_to_string(&rules).unwrap();
        assert!(bytes.contains("(rule \"example\""));
        assert!(!bytes.contains(PROBE));
        assert_eq!(std::fs::read(&board).unwrap(), FIXTURE);
        let report = dir.path().join("actual.json");
        let run =
            tokio::process::Command::new("/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli")
                .args(["pcb", "drc", "--format", "json", "--output"])
                .arg(&report)
                .arg(&board)
                .output()
                .await
                .unwrap();
        assert!(run.status.success());
        let report: Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
        assert!(report["violations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["type"] == "clearance"
                && v["description"]
                    .as_str()
                    .is_some_and(|s| s.contains("rule 'example'"))));
        args["name"] = json!("another");
        let stale = handle_create_custom_drc_rule(&args, &ctx).await.unwrap();
        assert!(stale.is_error);
        assert_eq!(std::fs::read_to_string(rules).unwrap(), bytes);
    }

    fn fake_cli(dir: &Path, pause: bool) -> std::path::PathBuf {
        let delay = if pause { "sleep 1\n" } else { "" };
        let body=format!("#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n if [ \"$1\" = \"--output\" ]; then shift; out=\"$1\"; fi\n shift\ndone\n{delay}printf '%s' '{{\"unconnected_items\":[],\"violations\":[{{\"type\":\"clearance\",\"description\":\"rule '{PROBE}'\"}}]}}' > \"$out\"\nexit {}\n",if pause {0}else{1});
        crate::tools::cli::test_support::write_script(
            dir,
            "rule-validation-fake",
            &body,
            "@echo off\r\nexit /b 1\r\n",
        )
    }
    #[tokio::test]
    async fn command_failure_is_not_a_valid_witness() {
        let d = tempfile::tempdir().unwrap();
        let cli = fake_cli(d.path(), false);
        assert!(validate(&cli.to_string_lossy(), "(version 1)\n")
            .await
            .is_err());
    }
    #[tokio::test]
    async fn validation_timeout_is_not_a_valid_witness() {
        let d = tempfile::tempdir().unwrap();
        let cli = fake_cli(d.path(), true);
        assert!(validate_with_timeout(
            &cli.to_string_lossy(),
            "(version 1)\n",
            std::time::Duration::from_millis(25)
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn an_open_board_refuses_even_a_preview() {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("open.kicad_pcb");
        std::fs::write(&board, FIXTURE).unwrap();
        let server =
            crate::tools::pcb_board::board_mock::spawn_kicad_holding_board(&board, |_| None);
        let mut ctx = crate::tools::pcb_board::board_mock::ctx_talking_to(server.address().into());
        ctx.config.kicad_cli = fake_cli(d.path(), true).to_string_lossy().into_owned();
        let result = handle_create_custom_drc_rule(
            &json!({"board":board,"name":"safe","constraint":"clearance","minimum_mm":0.2}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(result.is_error);
        assert!(!board.with_extension("kicad_dru").exists());
    }

    #[tokio::test]
    async fn invalid_dry_run_and_nonfile_board_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        std::fs::write(&board, FIXTURE).unwrap();
        let cli = fake_cli(d.path(), true);
        let ctx = ToolContext::new(
            crate::tools::ServerConfig {
                kicad_cli: cli.to_string_lossy().into_owned(),
                ..Default::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        let proposed = candidate("(version 1)\n", "safe", "clearance", 0.2).unwrap();
        let plan = test_plan(&board, None, &proposed);
        let result=handle_create_custom_drc_rule(&json!({"board":board,"name":"safe","constraint":"clearance","minimum_mm":0.2,"dry_run":"false","expected_plan_revision":plan}),&ctx).await.unwrap();
        assert!(result.is_error);
        assert!(!board.with_extension("kicad_dru").exists());
        let result = handle_create_custom_drc_rule(
            &json!({"board":d.path(),"name":"safe","constraint":"clearance","minimum_mm":0.2}),
            &ctx,
        )
        .await;
        assert!(result.is_err());
    }
    #[tokio::test]
    async fn concurrent_writer_is_not_reported_as_this_write() {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        let rule = board.with_extension("kicad_dru");
        let source = d.path().join("candidate.txt");
        std::fs::write(&board, FIXTURE).unwrap();
        std::fs::write(&rule, "(version 1)\n").unwrap();
        let proposed = candidate("(version 1)\n", "safe", "clearance", 0.2).unwrap();
        std::fs::write(&source, &proposed).unwrap();
        let plan = test_plan(&board, Some("(version 1)\n"), &proposed);
        let body=format!("#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n if [ \"$1\" = \"--output\" ]; then shift; out=\"$1\"; fi\n shift\ndone\ncp '{}' '{}'\nprintf '%s' '{{\"unconnected_items\":[],\"violations\":[{{\"type\":\"clearance\",\"description\":\"{PROBE}\"}}]}}' > \"$out\"\n",source.display(),rule.display());
        let cli = crate::tools::cli::test_support::write_script(
            d.path(),
            "concurrent-writer",
            &body,
            "@echo off\r\nexit /b 1\r\n",
        );
        let ctx = ToolContext::new(
            crate::tools::ServerConfig {
                kicad_cli: cli.to_string_lossy().into_owned(),
                ..Default::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        let result=handle_create_custom_drc_rule(&json!({"board":board,"name":"safe","constraint":"clearance","minimum_mm":0.2,"dry_run":false,"expected_plan_revision":plan}),&ctx).await.unwrap();
        assert!(result.is_error, "{result:?}");
        assert_eq!(
            crate::mcp::error::extract_error_kind(&result).as_deref(),
            Some("mutation_outcome_uncertain")
        );
        assert_eq!(std::fs::read_to_string(rule).unwrap(), proposed);
    }

    #[tokio::test]
    async fn board_opened_during_validation_refuses_apply() {
        use konnect_ipc::builders::pack_any;
        use konnect_ipc::gen::kiapi;
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("open.kicad_pcb");
        std::fs::write(&board, FIXTURE).unwrap();
        let marker = d.path().join("opened.txt");
        let document =
            crate::tools::pcb_board::board_mock::board_document(&board.to_string_lossy());
        let signal = marker.clone();
        let server = crate::test_support::MockIpcServer::spawn("rule-open-race", move |request| {
            let cmd = request.message.unwrap();
            let message = if cmd.type_url.ends_with("GetOpenDocuments") {
                Some(pack_any(
                    &kiapi::common::commands::GetOpenDocumentsResponse {
                        documents: if signal.exists() {
                            vec![document.clone()]
                        } else {
                            vec![]
                        },
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
        let body=format!("#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n if [ \"$1\" = \"--output\" ]; then shift; out=\"$1\"; fi\n shift\ndone\ntouch '{}'\nprintf '%s' '{{\"unconnected_items\":[],\"violations\":[{{\"type\":\"clearance\",\"description\":\"{PROBE}\"}}]}}' > \"$out\"\n",marker.display());
        let cli = crate::tools::cli::test_support::write_script(
            d.path(),
            "rule-race",
            &body,
            "@echo off\r\nexit /b 1\r\n",
        );
        let mut ctx = crate::tools::pcb_board::board_mock::ctx_talking_to(server.address().into());
        ctx.config.kicad_cli = cli.to_string_lossy().into_owned();
        let proposed = candidate("(version 1)\n", "safe", "clearance", 0.2).unwrap();
        let plan = test_plan(&board, None, &proposed);
        let result=handle_create_custom_drc_rule(&json!({"board":board,"name":"safe","constraint":"clearance","minimum_mm":0.2,"dry_run":false,"expected_plan_revision":plan}),&ctx).await.unwrap();
        assert!(result.is_error, "{result:?}");
        assert!(marker.exists());
        assert!(!board.with_extension("kicad_dru").exists());
    }
    #[tokio::test]
    async fn served_preview_exposes_the_rule_tool_without_writing() {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        std::fs::write(&board, FIXTURE).unwrap();
        let cli = fake_cli(d.path(), true);
        let handler = crate::mcp::handler::McpHandler::new(crate::tools::ServerConfig {
            kicad_cli: cli.to_string_lossy().into_owned(),
            eager_toolsets: true,
            ..Default::default()
        })
        .await
        .unwrap();
        let response=handler.handle_message(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_custom_drc_rule","arguments":{"board":board,"name":"safe","constraint":"clearance","minimum_mm":0.2000000000001}}})).await.unwrap();
        let result = response.result.unwrap();
        assert_ne!(result["isError"], true, "{result:#}");
        let body: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["status"], "ready");
        assert_eq!(body["target_board_drc"]["status"], "checked");
        assert_eq!(body["rule"]["minimum_mm"], 0.2);
        assert!(!board.with_extension("kicad_dru").exists());
    }

    #[test]
    fn readback_matches_only_validated_bytes() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("rules.dru");
        std::fs::write(&p, "saved").unwrap();
        assert!(saved_matches(&p, "saved"));
        assert!(!saved_matches(&p, "different"));
        assert!(!saved_matches(&d.path().join("missing"), ""));
    }

    #[tokio::test]
    async fn invalid_arguments_and_unsupported_existing_rules_are_not_silenced() {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        std::fs::write(&board, FIXTURE).unwrap();
        let cli = fake_cli(d.path(), true);
        let ctx = ToolContext::new(
            crate::tools::ServerConfig {
                kicad_cli: cli.to_string_lossy().into_owned(),
                ..Default::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        let bad = handle_create_custom_drc_rule(
            &json!({"board":board,"name":"safe","constraint":"clearance"}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(bad.is_error);
        let rules = board.with_extension("kicad_dru");
        let existing="(version 1)\n(rule \"existing\" (condition \"A.NetName == 'VCC'\") (condition \"A.NetName == 'GND'\") (constraint clearance (min 0.2mm)))\n";
        std::fs::write(&rules, existing).unwrap();
        let refused = handle_create_custom_drc_rule(
            &json!({"board":board,"name":"safe","constraint":"clearance","minimum_mm":0.2}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(refused.is_error);
        assert_eq!(std::fs::read_to_string(rules).unwrap(), existing);
    }
    #[tokio::test]
    async fn failed_validation_cannot_write_a_rule() {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        std::fs::write(&board, FIXTURE).unwrap();
        let body=format!("#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n if [ \"$1\" = \"--output\" ]; then shift; out=\"$1\"; fi\n shift\ndone\nprintf '%s' '{{\"unconnected_items\":[],\"violations\":[{{\"type\":\"clearance\",\"description\":\"{PROBE}\"}}]}}' > \"$out\"\ncase \"$out\" in */target-report.json) exit 0;; *) exit 1;; esac\n");
        let cli = crate::tools::cli::test_support::write_script(
            d.path(),
            "probe-fails-target-passes",
            &body,
            "@echo off\r\nexit /b 1\r\n",
        );
        let ctx = ToolContext::new(
            crate::tools::ServerConfig {
                kicad_cli: cli.to_string_lossy().into_owned(),
                ..Default::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        let proposed = candidate("(version 1)\n", "safe", "clearance", 0.2).unwrap();
        let plan = test_plan(&board, None, &proposed);
        let refused=handle_create_custom_drc_rule(&json!({"board":board,"name":"safe","constraint":"clearance","minimum_mm":0.2,"dry_run":false,"expected_plan_revision":plan}),&ctx).await.unwrap();
        assert!(refused.is_error);
        assert!(!board.with_extension("kicad_dru").exists());
    }

    #[tokio::test]
    async fn wrong_board_extension_or_root_refuses_before_writing() {
        let d = tempfile::tempdir().unwrap();
        let good_wrong_ext = d.path().join("not-board.txt");
        let bad_root = d.path().join("wrong.kicad_pcb");
        std::fs::write(&good_wrong_ext, FIXTURE).unwrap();
        std::fs::write(&bad_root, "(kicad_sch)").unwrap();
        let cli = fake_cli(d.path(), true);
        let ctx = ToolContext::new(
            crate::tools::ServerConfig {
                kicad_cli: cli.to_string_lossy().into_owned(),
                ..Default::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        for board in [good_wrong_ext, bad_root] {
            let result = handle_create_custom_drc_rule(
                &json!({"board":board,"name":"safe","constraint":"clearance","minimum_mm":0.2}),
                &ctx,
            )
            .await;
            assert!(result.is_err() || result.unwrap().is_error);
        }
    }
    #[test]
    fn invalid_saved_project_is_not_silently_defaulted() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("bad.kicad_pro");
        assert!(read_project(&p).unwrap().is_none());
        for source in ["not-json", "[]", "null", "{\"board\":{},\"board\":{}}"] {
            std::fs::write(&p, source).unwrap();
            assert!(read_project(&p).is_err());
        }
        std::fs::write(&p, "{}\n").unwrap();
        assert!(read_project(&p).unwrap().is_some());
    }

    #[test]
    fn target_report_missing_categories_and_unknown_severity_are_not_zeroed() {
        assert!(target_summary(&json!({}), false).is_err());
        assert!(target_summary(&json!({"unconnected_items":[]}), false).is_err());
        assert!(target_summary(&json!({"violations":[]}), false).is_err());
        let report =
            target_summary(&json!({"violations":[{}],"unconnected_items":[]}), false).unwrap();
        assert_eq!(report["other_severity_violations"], 1);
        assert_eq!(report["severity_coverage"], "incomplete");
        assert_eq!(report["ignored_checks_count"], Value::Null);
        let keys=target_summary(&json!({"violations":[],"unconnected_items":[],"ignored_checks":[{"key":"clearance"},{}]}),true).unwrap();
        assert_eq!(keys["ignored_check_keys"], json!(["clearance", null]));

        assert_eq!(report["project_source"], "synthetic_default_project");
        assert_eq!(report["source"], "scratch_copy_of_saved_board");
    }
    #[tokio::test]
    async fn target_drc_exit_failure_and_timeout_are_not_success() {
        let d = tempfile::tempdir().unwrap();
        let bad = fake_cli(d.path(), false);
        assert!(target_drc(
            &bad.to_string_lossy(),
            std::str::from_utf8(FIXTURE).unwrap(),
            None,
            "(version 1)\n"
        )
        .await
        .is_err());
        let slow = fake_cli(d.path(), true);
        assert!(target_drc_with_timeout(
            &slow.to_string_lossy(),
            std::str::from_utf8(FIXTURE).unwrap(),
            None,
            "(version 1)\n",
            std::time::Duration::from_millis(25)
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn changed_board_or_project_during_validation_refuses_apply() {
        for change_board in [true, false] {
            let d = tempfile::tempdir().unwrap();
            let board = d.path().join("board.kicad_pcb");
            std::fs::write(&board, FIXTURE).unwrap();
            let source = d.path().join("replacement.txt");
            let target = if change_board {
                std::fs::write(
                    &source,
                    format!("{}\n", std::str::from_utf8(FIXTURE).unwrap()),
                )
                .unwrap();
                board.clone()
            } else {
                std::fs::write(&source, "{}\n").unwrap();
                board.with_extension("kicad_pro")
            };
            let proposed = candidate("(version 1)\n", "safe", "clearance", 0.2).unwrap();
            let plan = test_plan(&board, None, &proposed);
            let body=format!("#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n if [ \"$1\" = \"--output\" ]; then shift; out=\"$1\"; fi\n shift\ndone\ncp '{}' '{}'\nprintf '%s' '{{\"unconnected_items\":[],\"violations\":[{{\"type\":\"clearance\",\"description\":\"{PROBE}\"}}]}}' > \"$out\"\n",source.display(),target.display());
            let cli = crate::tools::cli::test_support::write_script(
                d.path(),
                "change-snapshot",
                &body,
                "@echo off\r\nexit /b 1\r\n",
            );
            let ctx = ToolContext::new(
                crate::tools::ServerConfig {
                    kicad_cli: cli.to_string_lossy().into_owned(),
                    ..Default::default()
                },
                std::sync::Arc::new(crate::router::ToolRouter::new()),
            );
            let result=handle_create_custom_drc_rule(&json!({"board":board,"name":"safe","constraint":"clearance","minimum_mm":0.2,"dry_run":false,"expected_plan_revision":plan}),&ctx).await.unwrap();
            assert!(result.is_error, "{result:?}");
            assert!(!board.with_extension("kicad_dru").exists());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_rule_file_is_refused_without_touching_link_or_target() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        let path = board.with_extension("kicad_dru");
        let target = d.path().join("external.dru");
        std::fs::write(&board, FIXTURE).unwrap();
        std::fs::write(&target, "(version 1)\n").unwrap();
        symlink(&target, &path).unwrap();
        assert!(read_optional(&path).is_err());
        let cli = fake_cli(d.path(), true);
        let ctx = ToolContext::new(
            crate::tools::ServerConfig {
                kicad_cli: cli.to_string_lossy().into_owned(),
                ..Default::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        let result = handle_create_custom_drc_rule(
            &json!({"board":board,"name":"safe","constraint":"clearance","minimum_mm":0.2}),
            &ctx,
        )
        .await;
        assert!(result.is_err() || result.unwrap().is_error);
        assert!(std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "(version 1)\n");
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn rule_file_replaced_with_symlink_during_validation_is_not_overwritten() {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        let path = board.with_extension("kicad_dru");
        let target = d.path().join("external.dru");
        std::fs::write(&board, FIXTURE).unwrap();
        std::fs::write(&path, "(version 1)\n").unwrap();
        std::fs::write(&target, "(version 1)\n").unwrap();
        let proposed = candidate("(version 1)\n", "safe", "clearance", 0.2).unwrap();
        let plan = test_plan(&board, Some("(version 1)\n"), &proposed);
        let body=format!("#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n if [ \"$1\" = \"--output\" ]; then shift; out=\"$1\"; fi\n shift\ndone\nrm -f '{}'\nln -s '{}' '{}'\nprintf '%s' '{{\"unconnected_items\":[],\"violations\":[{{\"type\":\"clearance\",\"description\":\"{PROBE}\"}}]}}' > \"$out\"\n",path.display(),target.display(),path.display());
        let cli = crate::tools::cli::test_support::write_script(
            d.path(),
            "replace-with-symlink",
            &body,
            "@echo off\r\nexit /b 1\r\n",
        );
        let ctx = ToolContext::new(
            crate::tools::ServerConfig {
                kicad_cli: cli.to_string_lossy().into_owned(),
                ..Default::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        let result=handle_create_custom_drc_rule(&json!({"board":board,"name":"safe","constraint":"clearance","minimum_mm":0.2,"dry_run":false,"expected_plan_revision":plan}),&ctx).await;
        assert!(result.is_err() || result.unwrap().is_error);
        assert!(std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "(version 1)\n");
    }

    #[tokio::test]
    async fn failed_target_drc_cannot_write_a_rule() {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        std::fs::write(&board, FIXTURE).unwrap();
        let body=format!("#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n if [ \"$1\" = \"--output\" ]; then shift; out=\"$1\"; fi\n shift\ndone\nprintf '%s' '{{\"unconnected_items\":[],\"violations\":[{{\"type\":\"clearance\",\"description\":\"{PROBE}\"}}]}}' > \"$out\"\ncase \"$out\" in */target-report.json) exit 1;; *) exit 0;; esac\n");
        let cli = crate::tools::cli::test_support::write_script(
            d.path(),
            "target-fails-probe-passes",
            &body,
            "@echo off\r\nexit /b 1\r\n",
        );
        let ctx = ToolContext::new(
            crate::tools::ServerConfig {
                kicad_cli: cli.to_string_lossy().into_owned(),
                ..Default::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        let proposed = candidate("(version 1)\n", "safe", "clearance", 0.2).unwrap();
        let plan = test_plan(&board, None, &proposed);
        let result=handle_create_custom_drc_rule(&json!({"board":board,"name":"safe","constraint":"clearance","minimum_mm":0.2,"dry_run":false,"expected_plan_revision":plan}),&ctx).await.unwrap();
        assert!(result.is_error);
        assert!(!board.with_extension("kicad_dru").exists());
    }

    #[test]
    fn lock_inspection_failure_is_not_treated_as_absence() {
        let p = Path::new("/tmp/board.kicad_pcb");
        assert!(board_lock_refusal_with(p, |_| Err(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied
        )))
        .is_some());
        assert!(board_lock_refusal_with(p, |_| Err(std::io::Error::from(
            std::io::ErrorKind::NotFound
        )))
        .is_none());
    }
    #[tokio::test]
    async fn board_lock_refuses_even_when_ipc_reports_another_board() {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        let other = d.path().join("other.kicad_pcb");
        std::fs::write(&board, FIXTURE).unwrap();
        std::fs::write(&other, FIXTURE).unwrap();
        let board = board.canonicalize().unwrap();
        let lock = konnect_sexp::writer::kicad_editor_lock_path(&board).unwrap();
        std::fs::write(lock, "held").unwrap();
        let server =
            crate::tools::pcb_board::board_mock::spawn_kicad_holding_board(&other, |_| None);
        let mut ctx = crate::tools::pcb_board::board_mock::ctx_talking_to(server.address().into());
        ctx.config.kicad_cli = fake_cli(d.path(), true).to_string_lossy().into_owned();
        let result = handle_create_custom_drc_rule(
            &json!({"board":board,"name":"safe","constraint":"clearance","minimum_mm":0.2}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(result.is_error);
        assert!(!board.with_extension("kicad_dru").exists());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn a_board_alias_cannot_hide_the_real_board_lock() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        let alias = d.path().join("alias.kicad_pcb");
        let other = d.path().join("other.kicad_pcb");
        std::fs::write(&board, FIXTURE).unwrap();
        std::fs::write(&other, FIXTURE).unwrap();
        let board = board.canonicalize().unwrap();
        symlink(&board, &alias).unwrap();
        let lock = konnect_sexp::writer::kicad_editor_lock_path(&board).unwrap();
        std::fs::write(lock, "held").unwrap();
        let server =
            crate::tools::pcb_board::board_mock::spawn_kicad_holding_board(&other, |_| None);
        let mut ctx = crate::tools::pcb_board::board_mock::ctx_talking_to(server.address().into());
        ctx.config.kicad_cli = fake_cli(d.path(), true).to_string_lossy().into_owned();
        let result = handle_create_custom_drc_rule(
            &json!({"board":alias,"name":"safe","constraint":"clearance","minimum_mm":0.2}),
            &ctx,
        )
        .await
        .unwrap();
        assert!(result.is_error);
        let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
            panic!()
        };
        assert!(text.contains("KiCad board lock exists"), "{text}");
        assert!(!board.with_extension("kicad_dru").exists());
    }
    #[tokio::test]
    async fn lock_appearing_during_validation_prevents_apply() {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        let other = d.path().join("other.kicad_pcb");
        std::fs::write(&board, FIXTURE).unwrap();
        std::fs::write(&other, FIXTURE).unwrap();
        let board = board.canonicalize().unwrap();
        let lock = konnect_sexp::writer::kicad_editor_lock_path(&board).unwrap();
        let proposed = candidate("(version 1)\n", "safe", "clearance", 0.2).unwrap();
        let plan = test_plan(&board, None, &proposed);
        let body=format!("#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n if [ \"$1\" = \"--output\" ]; then shift; out=\"$1\"; fi\n shift\ndone\ntouch '{}'\nprintf '%s' '{{\"unconnected_items\":[],\"violations\":[{{\"type\":\"clearance\",\"description\":\"{PROBE}\"}}]}}' > \"$out\"\n",lock.display());
        let cli = crate::tools::cli::test_support::write_script(
            d.path(),
            "lock-race",
            &body,
            "@echo off\r\nexit /b 1\r\n",
        );
        let server =
            crate::tools::pcb_board::board_mock::spawn_kicad_holding_board(&other, |_| None);
        let mut ctx = crate::tools::pcb_board::board_mock::ctx_talking_to(server.address().into());
        ctx.config.kicad_cli = cli.to_string_lossy().into_owned();
        let result=handle_create_custom_drc_rule(&json!({"board":board,"name":"safe","constraint":"clearance","minimum_mm":0.2,"dry_run":false,"expected_plan_revision":plan}),&ctx).await.unwrap();
        assert!(result.is_error);
        assert!(lock.exists());
        assert!(!board.with_extension("kicad_dru").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn alias_lock_appearing_during_validation_prevents_apply() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let physical = d.path().join("board.kicad_pcb");
        let alias = d.path().join("alias.kicad_pcb");
        let other = d.path().join("other.kicad_pcb");
        std::fs::write(&physical, FIXTURE).unwrap();
        std::fs::write(&other, FIXTURE).unwrap();
        let physical = physical.canonicalize().unwrap();
        symlink(&physical, &alias).unwrap();
        let lock = konnect_sexp::writer::kicad_editor_lock_path(&alias).unwrap();
        let proposed = candidate("(version 1)\n", "safe", "clearance", 0.2).unwrap();
        let plan = test_plan(&physical, None, &proposed);
        let body=format!("#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n if [ \"$1\" = \"--output\" ]; then shift; out=\"$1\"; fi\n shift\ndone\ntouch '{}'\nprintf '%s' '{{\"unconnected_items\":[],\"violations\":[{{\"type\":\"clearance\",\"description\":\"{PROBE}\"}}]}}' > \"$out\"\n",lock.display());
        let cli = crate::tools::cli::test_support::write_script(
            d.path(),
            "alias-lock-race",
            &body,
            "@echo off\r\nexit /b 1\r\n",
        );
        let server =
            crate::tools::pcb_board::board_mock::spawn_kicad_holding_board(&other, |_| None);
        let mut ctx = crate::tools::pcb_board::board_mock::ctx_talking_to(server.address().into());
        ctx.config.kicad_cli = cli.to_string_lossy().into_owned();
        let result=handle_create_custom_drc_rule(&json!({"board":alias,"name":"safe","constraint":"clearance","minimum_mm":0.2,"dry_run":false,"expected_plan_revision":plan}),&ctx).await.unwrap();
        assert!(result.is_error);
        assert!(lock.exists());
        assert!(!physical.with_extension("kicad_dru").exists());
    }
    #[tokio::test]
    #[ignore = "requires real KiCad CLI"]
    async fn native_expressions_reject_invalid_assertions_even_when_the_global_probe_would_pass() {
        let cli = "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli";
        for rule in [
            "(rule \"X\" (condition \"A.NetName == 'VCC'\") (constraint clearance (min 0.1mm + 0.1mm)))",
            "(rule \"X\" (constraint assertion \"A.NetName == 'VCC'\"))",
        ] {validate(cli,&format!("(version 1)\n{rule}")).await.unwrap();}
        for rule in [
            "(rule \"X\" (constraint assertion \"A.NoSuchProperty == 1\"))",
            "(rule \"X\" (condition \"A.NoSuchProperty == 1\") (constraint clearance (min 0.2mm)))",
            "(rule \"X\" (constraint clearance (min garbage)))",
            "(rule \"X\" (layer \"NoSuchLayer\") (constraint clearance (min 0.2mm)))",
        ] {
            assert!(
                validate(cli, &format!("(version 1)\n{rule}"))
                    .await
                    .is_err(),
                "{rule}"
            );
        }
    }
    #[tokio::test]
    #[ignore = "requires real KiCad CLI"]
    async fn native_rule_edit_and_delete_round_trip_preserves_other_bytes() {
        use crate::custom_rules::Operation;
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        let path = board.with_extension("kicad_dru");
        std::fs::write(&board, FIXTURE).unwrap();
        let original = "# preserve header\r\n(version 1)\r\n# preserve unrelated comment\r\n";
        std::fs::write(&path, original).unwrap();
        let ctx = ToolContext::new(
            crate::tools::ServerConfig {
                kicad_cli: "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli".into(),
                ..Default::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        for (operation,rule) in [(Operation::Create,Some("(rule \"general rule\" (condition \"A.NetName == 'VCC'\") (constraint clearance (min 0.1mm + 0.1mm)))")),(Operation::Edit,Some("(rule \"general rule\" (constraint track_width (min 0.3mm)))")),(Operation::Delete,None)] {
            let mut args=json!({"board":board,"name":"general rule"});if let Some(rule)=rule{args["rule"]=json!(rule);}
            let before=std::fs::read(&path).unwrap();let preview=handle_rule_change(&args,&ctx,operation).await.unwrap();assert!(!preview.is_error,"{preview:?}");assert_eq!(std::fs::read(&path).unwrap(),before);
            let crate::mcp::protocol::ToolContent::Text{text}=&preview.content[0] else{panic!()};let body:Value=serde_json::from_str(text).unwrap();args["dry_run"]=json!(false);
            assert!(handle_rule_change(&args,&ctx,operation).await.unwrap().is_error);
            args["expected_plan_revision"]=body["plan_revision"].clone();let result=handle_rule_change(&args,&ctx,operation).await.unwrap();assert!(!result.is_error,"{result:?}");
            let saved=std::fs::read_to_string(&path).unwrap();assert!(saved.starts_with(original));assert_eq!(crate::custom_rules::parse(&saved).unwrap().len(),if matches!(operation,Operation::Delete){0}else{1});
        }
        assert_eq!(std::fs::read(&board).unwrap(), FIXTURE);
    }

    #[tokio::test]
    #[ignore = "requires real KiCad CLI"]
    async fn native_target_drc_uses_the_saved_project_and_requested_board() {
        let cli = "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli";
        let board = std::str::from_utf8(FIXTURE).unwrap();
        let mut project: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/multichannel_mixer.kicad_pro"
        ))
        .unwrap();
        project["board"]["design_settings"]["rules"]["min_clearance"] = json!(0.0);
        let low = target_drc(cli, board, Some(&project.to_string()), "(version 1)\n")
            .await
            .unwrap();
        project["board"]["design_settings"]["rules"]["min_clearance"] = json!(2.0);
        let high = target_drc(cli, board, Some(&project.to_string()), "(version 1)\n")
            .await
            .unwrap();
        assert!(
            high["error_violations"].as_u64().unwrap() > low["error_violations"].as_u64().unwrap(),
            "low={low:#}, high={high:#}"
        );
        assert_eq!(high["project_source"], "copied_saved_project");
        assert!(target_drc(cli, "(kicad_sch)", None, "(version 1)\n")
            .await
            .is_err());
    }
}

#[cfg(test)]
#[path = "drc_rule_edit_tests.rs"]
mod edit_tests;
