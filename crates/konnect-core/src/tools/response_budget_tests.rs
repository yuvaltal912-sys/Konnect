//! Creation PR 3: a long result stays inside the client's size cap, and says
//! what it left out. The budget itself is tested in `response_budget.rs`; these
//! drive it through the tools.

use super::cli::{DrcViolation, ReportItem, ReportPos};
use super::ToolContext;
use crate::mcp::protocol::{CallToolResult, ToolContent};
use serde_json::{json, Value};
use std::path::Path;

/// What the client refuses is about 50,000 characters.
const CLIENT_CAP: usize = 50_000;

fn body(result: &CallToolResult) -> Value {
    let ToolContent::Text { text } = &result.content[0] else {
        panic!("expected text");
    };
    assert!(
        text.len() < CLIENT_CAP + 20_000,
        "not under test: {}",
        text.len()
    );
    serde_json::from_str(text).unwrap_or_else(|_| panic!("not JSON: {text}"))
}

fn text_len(result: &CallToolResult) -> usize {
    match &result.content[0] {
        ToolContent::Text { text } => text.len(),
        _ => panic!("expected text"),
    }
}

fn ctx_with_cli(cli: &Path) -> ToolContext {
    ToolContext::new(
        crate::tools::ServerConfig {
            kicad_cli: cli.to_str().unwrap().to_owned(),
            ..Default::default()
        },
        std::sync::Arc::new(crate::router::ToolRouter::new()),
    )
}

fn plain_ctx() -> ToolContext {
    ToolContext::new(
        crate::tools::ServerConfig::default(),
        std::sync::Arc::new(crate::router::ToolRouter::new()),
    )
}

// ─── ERC ─────────────────────────────────────────────────────────────────────

/// A report shaped like the ones the client refused: hundreds of violations,
/// each with long item text.
fn big_erc_report(count: usize) -> String {
    let violations: Vec<Value> = (0..count)
        .map(|i| {
            json!({
                "description": format!("Pin {i} not connected, and this text is long on purpose, as KiCad's own is"),
                "severity": if i.is_multiple_of(3) { "error" } else { "warning" },
                "type": if i.is_multiple_of(2) { "pin_not_connected" } else { "label_dangling" },
                "items": [
                    { "description": format!("Symbol R{i} Pin 1 [Passive, Line]"), "pos": { "x": 1.0, "y": 2.0 } },
                    { "description": format!("Symbol C{i} Pin 2 [Passive, Line]"), "pos": { "x": 3.0, "y": 4.0 } }
                ]
            })
        })
        .collect();
    json!({
        "kicad_version": "10.0.8",
        "sheets": [{ "path": "/", "violations": violations }]
    })
    .to_string()
}

#[cfg(unix)]
fn erc_scene(count: usize) -> (tempfile::TempDir, std::path::PathBuf, ToolContext) {
    let dir = tempfile::tempdir().unwrap();
    let report = dir.path().join("report.json");
    std::fs::write(&report, big_erc_report(count)).unwrap();
    let schematic = dir.path().join("top.kicad_sch");
    std::fs::write(&schematic, "(kicad_sch)").unwrap();
    let script = crate::tools::cli::test_support::write_script(
        dir.path(),
        "fake-kicad-cli",
        &format!("#!/bin/sh\ncp '{}' \"$4\"\nexit 0\n", report.display()),
        "",
    );
    let ctx = ctx_with_cli(&script);
    (dir, schematic, ctx)
}

#[cfg(unix)]
#[tokio::test]
async fn a_large_erc_stays_under_the_cap_and_counts_everything() {
    let (_dir, schematic, ctx) = erc_scene(700);
    let result = super::sch_export::handle_run_erc(
        &json!({ "schematic": schematic.to_string_lossy(), "limit": 5000 }),
        &ctx,
    )
    .await
    .unwrap();
    assert!(!result.is_error, "{result:?}");
    assert!(
        text_len(&result) < CLIENT_CAP,
        "{} characters",
        text_len(&result)
    );
    let response = body(&result);
    // Counts cover all 700, whatever the page holds.
    assert_eq!(response["total"], 700);
    assert_eq!(response["errors"], 234);
    assert_eq!(response["warnings"], 466);
    let shown = response["shown"].as_u64().unwrap();
    assert!(shown > 20 && shown < 700, "{shown}");
    assert_eq!(
        response["violations"].as_array().unwrap().len() as u64,
        shown
    );
    assert_eq!(response["truncated"], true);
    assert_eq!(response["byte_capped"], true);
    assert_eq!(response["next_offset"], shown);
    // The tally adds up to every violation, not the page.
    let tallied: u64 = response["by_rule"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["count"].as_u64().unwrap())
        .sum();
    assert_eq!(tallied, 700);
}

#[cfg(unix)]
#[tokio::test]
async fn erc_pages_resume_where_the_last_stopped_and_cover_every_violation_once() {
    let (_dir, schematic, ctx) = erc_scene(250);
    let mut seen = Vec::new();
    let mut offset = 0u64;
    loop {
        let result = super::sch_export::handle_run_erc(
            &json!({ "schematic": schematic.to_string_lossy(), "limit": 60, "offset": offset }),
            &ctx,
        )
        .await
        .unwrap();
        let page = body(&result);
        for violation in page["violations"].as_array().unwrap() {
            seen.push(violation["description"].as_str().unwrap().to_owned());
        }
        if page["truncated"] == false {
            assert!(page.get("next_offset").is_none());
            break;
        }
        offset = page["next_offset"].as_u64().unwrap();
    }
    assert_eq!(seen.len(), 250);
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 250, "no violation twice");
}

#[cfg(unix)]
#[tokio::test]
async fn erc_output_is_the_full_response_even_when_the_page_is_short() {
    let (dir, schematic, ctx) = erc_scene(300);
    let written = dir.path().join("full.json");
    let result = super::sch_export::handle_run_erc(
        &json!({ "schematic": schematic.to_string_lossy(), "limit": 10,
                 "output": written.to_string_lossy() }),
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(body(&result)["violations"].as_array().unwrap().len(), 10);
    let file: Value = serde_json::from_str(&std::fs::read_to_string(&written).unwrap()).unwrap();
    assert_eq!(file["violations"].as_array().unwrap().len(), 300);
    assert_eq!(file["total"], 300);
}

#[tokio::test]
async fn erc_refuses_a_limit_or_offset_it_cannot_read_before_running_anything() {
    for args in [
        json!({ "limit": 0 }),
        json!({ "limit": 100000 }),
        json!({ "limit": "ten" }),
        json!({ "offset": -3 }),
    ] {
        let mut args = args;
        args["schematic"] = json!("/nowhere/top.kicad_sch");
        let result = super::sch_export::handle_run_erc(&args, &plain_ctx())
            .await
            .unwrap();
        assert!(result.is_error, "{args}");
        let text = match &result.content[0] {
            ToolContent::Text { text } => text.clone(),
            _ => unreachable!(),
        };
        assert!(
            text.contains("limit") || text.contains("offset"),
            "{args}: {text}"
        );
    }
}

// ─── DRC ─────────────────────────────────────────────────────────────────────

fn drc_violation(i: usize) -> DrcViolation {
    DrcViolation {
        severity: if i.is_multiple_of(4) {
            "error"
        } else {
            "warning"
        }
        .to_owned(),
        description: format!(
            "Clearance violation {i}, with the long prose KiCad writes after the number"
        ),
        rule: if i.is_multiple_of(2) {
            "clearance"
        } else {
            "silk_over_copper"
        }
        .to_owned(),
        pos: Some(ReportPos {
            x: i as f64,
            y: 2.0,
        }),
        items: (0..3)
            .map(|k| ReportItem {
                description: format!("Track on F.Cu, length {k}.{i} mm, net GND_ANALOG_{i}"),
                pos: Some(ReportPos { x: 1.0, y: 2.0 }),
                uuid: Some(format!("00000000-0000-0000-0000-{i:012}")),
                ..Default::default()
            })
            .collect(),
    }
}

#[test]
fn drc_violations_page_by_size_and_tally_every_rule() {
    let all: Vec<DrcViolation> = (0..900).map(drc_violation).collect();
    let refs: Vec<&DrcViolation> = all.iter().collect();
    let fields = super::drc::violations_page(&refs, 0, 5000);
    let text = serde_json::to_string(&fields).unwrap();
    assert!(text.len() < CLIENT_CAP, "{} characters", text.len());
    assert_eq!(fields["truncated"], true);
    assert_eq!(fields["byte_capped"], true);
    let tallied: u64 = fields["by_rule"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["count"].as_u64().unwrap())
        .sum();
    assert_eq!(tallied, 900);
    // Resuming at next_offset continues the same list.
    let next = fields["next_offset"].as_u64().unwrap() as usize;
    let again = super::drc::violations_page(&refs, next, 5000);
    assert_eq!(again["violations"][0]["pos"]["x"], next as f64);
}

#[tokio::test]
async fn both_drc_tools_refuse_a_bad_window_before_running() {
    for (tool, handler) in [("run_drc", 0), ("get_drc_violations", 1)] {
        for extra in [
            json!({ "limit": 0 }),
            json!({ "offset": "x" }),
            json!({ "limit": 1.5 }),
        ] {
            let mut args = extra;
            args["board"] = json!("/nowhere/top.kicad_pcb");
            let result = if handler == 0 {
                super::verification::handle_run_drc(&args, &plain_ctx()).await
            } else {
                super::pcb_export::handle_get_drc_violations(&args, &plain_ctx()).await
            }
            .unwrap();
            assert!(result.is_error, "{tool} {args}");
        }
    }
}

// ─── List tools ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_schematic_components_pages_and_counts_the_whole_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixer.kicad_sch");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multichannel_mixer.kicad_sch"),
        &path,
    )
    .unwrap();
    let call = |extra: Value| {
        let mut args = extra;
        args["schematic"] = json!(path.to_string_lossy());
        async move {
            super::sch_components::handle_list_schematic_components(&args, &plain_ctx())
                .await
                .unwrap()
        }
    };
    let whole = body(&call(json!({})).await);
    let count = whole["count"].as_u64().unwrap();
    assert!(count > 3, "{whole}");
    assert_eq!(whole["truncated"], false);

    let first = body(&call(json!({ "limit": 3 })).await);
    assert_eq!(first["count"], count);
    assert_eq!(first["components"].as_array().unwrap().len(), 3);
    assert_eq!(first["truncated"], true);
    assert_eq!(first["next_offset"], 3);
    let rest = body(&call(json!({ "offset": 3, "limit": 10000 })).await);
    assert_eq!(
        rest["components"].as_array().unwrap().len() as u64,
        count - 3
    );
    assert_eq!(
        rest["components"][0], whole["components"][3],
        "the page starts where the first stopped"
    );
    let refused = call(json!({ "limit": 0 })).await;
    assert!(refused.is_error);
}

#[tokio::test]
async fn library_lists_page_and_count_every_library() {
    let dir = tempfile::tempdir().unwrap();
    let entries: String = (0..260)
        .map(|i| {
            format!(
                "\t(lib (name \"Lib_{i:03}\") (type \"KiCad\") (uri \"${{KIPRJMOD}}/Lib_{i:03}.kicad_sym\") (options \"\") (descr \"{}\"))\n",
                "a library description of realistic length ".repeat(4)
            )
        })
        .collect();
    std::fs::write(
        dir.path().join("sym-lib-table"),
        format!("(sym_lib_table\n\t(version 7)\n{entries})\n"),
    )
    .unwrap();
    let project = dir.path().join("p.kicad_pro");
    std::fs::write(&project, "{}").unwrap();
    let list = |extra: Value| {
        let mut args = extra;
        args["project"] = json!(project.to_string_lossy());
        args["scope"] = json!("project");
        async move {
            super::library::handle_list_symbol_libraries(&args, &plain_ctx())
                .await
                .unwrap()
        }
    };
    let page = list(json!({})).await;
    assert!(text_len(&page) < CLIENT_CAP);
    let page = body(&page);
    assert_eq!(page["count"], 260);
    assert_eq!(page["libraries"].as_array().unwrap().len(), 100);
    assert_eq!(page["truncated"], true);
    assert_eq!(page["next_offset"], 100);
    let last = body(&list(json!({ "offset": 200 })).await);
    assert_eq!(last["libraries"].as_array().unwrap().len(), 60);
    assert_eq!(last["truncated"], false);
}

// ─── Rank 3 leftovers (A30): batch edits, manufacturing check ───────────────

async fn call_tool(
    tools: Vec<super::ToolDef>,
    name: &str,
    args: Value,
    ctx: ToolContext,
) -> CallToolResult {
    let tool = tools.into_iter().find(|tool| tool.name == name).unwrap();
    (tool.handler)(&args, std::sync::Arc::new(ctx))
        .await
        .unwrap()
}

const EMPTY_SCH: &str = "(kicad_sch\n\t(version 20250610)\n\t(generator \"eeschema\")\n\t(uuid \"root\")\n\t(paper \"A4\")\n\t(lib_symbols)\n\t(sheet_instances\n\t\t(path \"/\" (page \"1\"))\n\t)\n)\n";

fn empty_schematic() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("batch.kicad_sch");
    std::fs::write(&path, EMPTY_SCH).unwrap();
    (dir, path)
}

#[tokio::test]
async fn a_large_no_connect_batch_is_cut_to_the_cap_with_the_count_whole() {
    let (_dir, path) = empty_schematic();
    let positions: Vec<Value> = (0..3000)
        .map(|i| json!({ "x": 1.27 * (i % 100) as f64, "y": 1.27 * (i / 100) as f64 }))
        .collect();
    let result = call_tool(
        super::sch_wiring::tools(),
        "batch_add_no_connect",
        json!({ "schematic": path.display().to_string(), "positions": positions }),
        plain_ctx(),
    )
    .await;
    assert!(text_len(&result) < CLIENT_CAP, "{}", text_len(&result));
    let body = body(&result);
    assert_eq!(body["added_count"], 3000);
    assert_eq!(body["truncated_lists"]["added"]["total"], 3000);
    let shown = body["added"].as_array().unwrap().len();
    assert!(shown > 100 && shown < 3000, "{shown}");
    assert_eq!(body["truncated_lists"]["added"]["shown"], shown);
    let written = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        written.matches("(no_connect").count(),
        3000,
        "every marker is saved"
    );
}

#[tokio::test]
async fn a_small_no_connect_batch_is_unchanged() {
    let (_dir, path) = empty_schematic();
    let result = call_tool(
        super::sch_wiring::tools(),
        "batch_add_no_connect",
        json!({ "schematic": path.display().to_string(), "positions": [{ "x": 1.27, "y": 2.54 }, { "y": 1 }] }),
        plain_ctx(),
    )
    .await;
    let body = body(&result);
    let keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["added", "added_count", "errors"], "{body}");
    assert_eq!(body["errors"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn thousands_of_failed_pin_pairs_stay_under_the_cap_and_say_how_many() {
    let (_dir, path) = empty_schematic();
    let connections: Vec<Value> = (0..2000)
        .map(|i| json!({ "ref1": format!("MISSING{i}"), "pin1": "1", "ref2": format!("ABSENT{i}"), "pin2": "2" }))
        .collect();
    let result = call_tool(
        super::sch_batch::tools(),
        "batch_connect_pins",
        json!({ "schematic": path.display().to_string(), "connections": connections }),
        plain_ctx(),
    )
    .await;
    assert!(result.is_error, "nothing connected is still a failed call");
    assert!(text_len(&result) < CLIENT_CAP, "{}", text_len(&result));
    let body = body(&result);
    assert_eq!(body["connected_count"], 0);
    assert_eq!(body["truncated_lists"]["errors"]["total"], 2000);
    assert!(body["errors"].as_array().unwrap().len() < 2000);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), EMPTY_SCH);
}

#[cfg(unix)]
fn manufacturing_scene(errors: usize) -> (tempfile::TempDir, std::path::PathBuf, ToolContext) {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.kicad_pcb");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../konnect-sexp/tests/fixtures/board_geometry/cases.kicad_pcb"
        ),
        &board,
    )
    .unwrap();
    let violations: Vec<Value> = (0..errors)
        .map(|i| {
            json!({
                "type": format!("rule_{}", i % 7),
                "severity": "error",
                "description": format!("Violation {i}, described at the length KiCad uses"),
                "items": [{ "description": "Track", "pos": { "x": 1.0, "y": 2.0 } }]
            })
        })
        .collect();
    let report = dir.path().join("report.json");
    std::fs::write(
        &report,
        json!({ "violations": violations, "unconnected_items": [], "schematic_parity": [] })
            .to_string(),
    )
    .unwrap();
    let script = crate::tools::cli::test_support::write_script(
        dir.path(),
        "fake-kicad-cli",
        &format!(
            "#!/bin/sh\necho drc >> '{0}/calls'\ncp '{1}' \"$4\"\nexit 0\n",
            dir.path().display(),
            report.display()
        ),
        "",
    );
    let ctx = ctx_with_cli(&script);
    (dir, board, ctx)
}

#[cfg(unix)]
#[tokio::test]
async fn manufacturing_issues_page_and_the_verdict_reads_them_all() {
    let (dir, board, _) = manufacturing_scene(900);
    let script = dir.path().join("fake-kicad-cli");
    let mut seen = std::collections::BTreeSet::new();
    let mut offset = 0u64;
    let mut total = None;
    loop {
        let result = call_tool(
            super::manufacturing::tools(),
            "validate_for_manufacturing",
            json!({ "board": board.display().to_string(), "offset": offset }),
            ctx_with_cli(&script),
        )
        .await;
        assert!(text_len(&result) < CLIENT_CAP, "{}", text_len(&result));
        let page = body(&result);
        assert_eq!(page["verdict"], "NOT READY");
        let count = page["issue_count"].as_u64().unwrap();
        assert!(count >= 900, "{count}");
        assert_eq!(
            *total.get_or_insert(count),
            count,
            "the count is whole on every page"
        );
        assert!(page["issues_by_severity"]["error"].as_u64().unwrap() >= 900);
        assert!(page["summary"]
            .as_str()
            .unwrap()
            .contains(&format!("{count} issues")));
        for issue in page["issues"].as_array().unwrap() {
            assert!(seen.insert(issue.to_string()), "an issue shown twice");
        }
        match page["next_offset"].as_u64() {
            Some(next) => offset = next,
            None => break,
        }
    }
    assert_eq!(seen.len() as u64, total.unwrap());
}

#[cfg(unix)]
#[tokio::test]
async fn a_short_manufacturing_report_is_whole_and_a_bad_window_runs_nothing() {
    let (dir, board, ctx) = manufacturing_scene(2);
    let result = call_tool(
        super::manufacturing::tools(),
        "validate_for_manufacturing",
        json!({ "board": board.display().to_string() }),
        ctx,
    )
    .await;
    let page = body(&result);
    assert_eq!(page["truncated"], false);
    assert_eq!(
        page["issues"].as_array().unwrap().len() as u64,
        page["issue_count"].as_u64().unwrap()
    );
    assert!(page.get("next_offset").is_none() && page.get("hint").is_none());
    std::fs::remove_file(dir.path().join("calls")).unwrap();
    for bad in [json!({ "limit": 0 }), json!({ "offset": "two" })] {
        let mut args = json!({ "board": board.display().to_string() });
        args.as_object_mut()
            .unwrap()
            .extend(bad.as_object().unwrap().clone());
        let refused = call_tool(
            super::manufacturing::tools(),
            "validate_for_manufacturing",
            args,
            ctx_with_cli(&dir.path().join("fake-kicad-cli")),
        )
        .await;
        assert!(refused.is_error, "{bad}");
    }
    assert!(
        !dir.path().join("calls").exists(),
        "a refused window ran DRC"
    );
}
