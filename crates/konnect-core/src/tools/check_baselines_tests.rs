//! Creation PR 15: ERC and DRC baselines. The pure matching is tested on
//! records; the tools run against a stand-in `kicad-cli` that serves whatever
//! report the test put in the scene, over scratch project folders.

use super::*;
use crate::mcp::protocol::{CallToolResult, ToolContent};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// What the client refuses is about 50,000 characters.
const CLIENT_CAP: usize = 50_000;

fn text(result: &CallToolResult) -> &str {
    match &result.content[0] {
        ToolContent::Text { text } => text,
        _ => panic!("expected text"),
    }
}

fn body(result: &CallToolResult) -> Value {
    serde_json::from_str(text(result)).unwrap_or_else(|_| panic!("not JSON: {}", text(result)))
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

/// A scratch project and a stand-in kicad-cli that copies `report.json` from
/// the scene to the report path it is given, and counts its runs.
struct Scene {
    dir: tempfile::TempDir,
    ctx: ToolContext,
}

impl Scene {
    fn new(source_name: &str, source_text: &str) -> Scene {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(source_name), source_text).unwrap();
        let report = dir.path().join("report.json");
        let runs = dir.path().join("runs.log");
        let script = crate::tools::cli::test_support::write_script(
            dir.path(),
            "fake-kicad-cli",
            &format!(
                "#!/bin/sh\necho run >> '{}'\ncp '{}' \"$4\"\nexit 0\n",
                runs.display(),
                report.display()
            ),
            "",
        );
        let ctx = ctx_with_cli(&script);
        Scene { dir, ctx }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
    fn serve(&self, report: &Value) {
        std::fs::write(self.path("report.json"), report.to_string()).unwrap();
    }
    fn runs(&self) -> usize {
        std::fs::read_to_string(self.path("runs.log"))
            .map(|log| log.lines().count())
            .unwrap_or(0)
    }
}

// ─── ERC fixtures ─────────────────────────────────────────────────────────────

const SCHEMATIC: &str = r#"(kicad_sch (version 20250114)
  (uuid "root-sheet")
  (symbol (lib_id "Device:R") (uuid "sym-r1") (pin "1" (uuid "pin-r1-1")) (pin "2" (uuid "pin-r1-2")))
  (symbol (lib_id "Device:C") (uuid "sym-c1") (pin "1" (uuid "pin-c1-1")) (pin "2" (uuid "pin-c1-2")))
  (symbol (lib_id "Device:R") (uuid "sym-r2") (pin "1" (uuid "pin-r2-1")) (pin "2" (uuid "pin-r2-2")))
  (label "VIN" (uuid "label-vin"))
)"#;

fn erc_item(description: &str, uuid: &str, x: f64, y: f64) -> Value {
    json!({ "description": description, "uuid": uuid, "pos": { "x": x, "y": y } })
}

fn erc_violation(rule: &str, severity: &str, items: Vec<Value>) -> Value {
    json!({ "type": rule, "severity": severity, "description": format!("{rule} found"), "items": items })
}

fn erc_report(violations: Vec<Value>) -> Value {
    json!({ "kicad_version": "10.0.8", "sheets": [{ "path": "/", "violations": violations }] })
}

fn r1_unconnected(x: f64) -> Value {
    erc_violation(
        "pin_not_connected",
        "error",
        vec![erc_item(
            "Symbol R1 Pin 2 [Passive, Line]",
            "pin-r1-2",
            x,
            20.0,
        )],
    )
}

fn c1_r2_conflict(x: f64) -> Value {
    erc_violation(
        "pin_to_pin",
        "warning",
        vec![
            erc_item("Symbol C1 Pin 1 [Passive, Line]", "pin-c1-1", x, 30.0),
            erc_item("Symbol R2 Pin 1 [Passive, Line]", "pin-r2-1", x + 5.0, 30.0),
        ],
    )
}

fn label_dangling() -> Value {
    erc_violation(
        "label_dangling",
        "warning",
        vec![erc_item("Label 'VIN'", "label-vin", 50.0, 50.0)],
    )
}

fn erc_args(scene: &Scene, name: &str) -> Value {
    json!({ "schematic": scene.path("top.kicad_sch").to_string_lossy(), "name": name })
}

async fn save(scene: &Scene, args: &Value) -> CallToolResult {
    handle_save_check_baseline(args, &scene.ctx).await.unwrap()
}

async fn compare(scene: &Scene, args: &Value) -> CallToolResult {
    handle_compare_check_baseline(args, &scene.ctx)
        .await
        .unwrap()
}

fn changes(response: &Value, change: &str) -> Vec<Value> {
    response["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["change"] == change)
        .cloned()
        .collect()
}

// ─── Pure matching ────────────────────────────────────────────────────────────

#[test]
fn measured_values_are_masked_and_names_are_kept() {
    assert_eq!(
        normalize_measurements("Track [GND] on F.Cu, length 2.5400 mm"),
        "Track [GND] on F.Cu, length # mm"
    );
    assert_eq!(
        normalize_measurements("(clearance 0.2000mm; actual 0.1500 mm)"),
        "(clearance #mm; actual # mm)"
    );
    assert_eq!(
        normalize_measurements("rotated -45° by 12 mils"),
        "rotated #° by # mils"
    );
    assert_eq!(
        normalize_measurements("Pad 3 [Net-(U1-PA0)] of R12 on F.Cu"),
        "Pad 3 [Net-(U1-PA0)] of R12 on F.Cu",
        "numbers that name things stay"
    );
    assert_eq!(normalize_measurements("Via 0.3 in  hole"), "Via # in hole");
    assert_eq!(
        normalize_measurements("2 items inside"),
        "2 items inside",
        "'in' must be a whole word to be a unit"
    );
}

fn record(rule: &str, severity: &str, items: &[(&str, Option<&str>, KeyedBy)]) -> Record {
    Record {
        key: String::new(),
        category: "violations".into(),
        rule: rule.into(),
        severity: severity.into(),
        sheet: None,
        description: format!("{rule} (actual 0.1 mm)"),
        items: items
            .iter()
            .map(|(description, uuid, keyed_by)| RecordItem {
                description: (*description).into(),
                uuid: uuid.map(str::to_string),
                x: Some(1.0),
                y: Some(2.0),
                keyed_by: *keyed_by,
            })
            .collect(),
    }
}

#[test]
fn keys_ignore_item_order_position_prose_and_severity() {
    let mut a = vec![record(
        "clearance",
        "error",
        &[
            ("Track A", Some("u-1"), KeyedBy::Uuid),
            ("Pad 1 of R1", Some("u-2"), KeyedBy::Uuid),
        ],
    )];
    let mut b = a.clone();
    b[0].items.reverse();
    b[0].items[0].x = Some(99.0);
    b[0].description = "clearance (actual 0.05 mm)".into();
    b[0].severity = "warning".into();
    assign_keys(CheckKind::Drc, &mut a);
    assign_keys(CheckKind::Drc, &mut b);
    assert_eq!(a[0].key, b[0].key);
    assert!(a[0].key.starts_with("drc-"), "{}", a[0].key);

    // The rule and the items do decide the key.
    let mut other_rule = a.clone();
    other_rule[0].rule = "hole_clearance".into();
    assign_keys(CheckKind::Drc, &mut other_rule);
    assert_ne!(a[0].key, other_rule[0].key);
    let mut other_item = a.clone();
    other_item[0].items[1].uuid = Some("u-3".into());
    assign_keys(CheckKind::Drc, &mut other_item);
    assert_ne!(a[0].key, other_item[0].key);
}

#[test]
fn an_unstable_uuid_is_ignored_and_the_description_keys_the_item() {
    let mut first = vec![record(
        "unconnected_items",
        "error",
        &[("Pad 1 [GND] of J1", Some("random-1"), KeyedBy::Description)],
    )];
    let mut second = vec![record(
        "unconnected_items",
        "error",
        &[("Pad 1 [GND] of J1", Some("random-2"), KeyedBy::Description)],
    )];
    assign_keys(CheckKind::Drc, &mut first);
    assign_keys(CheckKind::Drc, &mut second);
    assert_eq!(first[0].key, second[0].key);
}

#[test]
fn repeats_are_numbered_so_only_the_extra_one_is_new() {
    let same = || {
        record(
            "silk_overlap",
            "warning",
            &[("Text 'X'", None, KeyedBy::Description)],
        )
    };
    let mut before = vec![same(), same()];
    let mut after = vec![same(), same(), same()];
    assign_keys(CheckKind::Drc, &mut before);
    assign_keys(CheckKind::Drc, &mut after);
    assert!(after[1].key.ends_with("~2") && after[2].key.ends_with("~3"));
    let cats = vec!["violations".to_string()];
    let result = diff(&before, &cats, &after, &cats);
    assert_eq!(
        (
            result.new.len(),
            result.unchanged.len(),
            result.resolved.len()
        ),
        (1, 2, 0)
    );
}

#[test]
fn a_category_one_run_did_not_report_is_neither_new_nor_resolved() {
    let mut before = vec![record(
        "clearance",
        "error",
        &[("A", Some("a"), KeyedBy::Uuid)],
    )];
    before[0].category = "unconnected_items".into();
    assign_keys(CheckKind::Drc, &mut before);
    let result = diff(
        &before,
        &["violations".into(), "unconnected_items".into()],
        &[],
        &["violations".into()],
    );
    assert!(result.resolved.is_empty(), "not checked is not resolved");
    assert_eq!(result.categories_not_compared, vec!["unconnected_items"]);
}

#[test]
fn a_severity_change_is_one_change_and_raising_it_is_a_regression() {
    let mut before = vec![record(
        "clearance",
        "warning",
        &[("A", Some("a"), KeyedBy::Uuid)],
    )];
    let mut after = vec![record(
        "clearance",
        "error",
        &[("A", Some("a"), KeyedBy::Uuid)],
    )];
    assign_keys(CheckKind::Drc, &mut before);
    assign_keys(CheckKind::Drc, &mut after);
    let cats = vec!["violations".to_string()];
    let result = diff(&before, &cats, &after, &cats);
    let response = compare_body(&result, severity_rank("warning"), 0, 50);
    assert_eq!(response["new_count"], 0);
    assert_eq!(response["resolved_count"], 0);
    assert_eq!(response["severity_changed_count"], 1);
    assert_eq!(response["severity_raised_count"], 1);
    assert_eq!(response["verdict"], "regressed");
    assert_eq!(response["changes"][0]["baseline_severity"], "warning");

    // Lowered: still listed, not a regression.
    let lowered = diff(&after, &cats, &before, &cats);
    let response = compare_body(&lowered, severity_rank("warning"), 0, 50);
    assert_eq!(response["severity_changed_count"], 1);
    assert_eq!(response["verdict"], "no_regressions");
}

#[test]
fn schematic_uuids_are_read_from_every_sheet_but_not_backups_or_hidden_folders() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("top.kicad_sch"), SCHEMATIC).unwrap();
    std::fs::create_dir_all(dir.path().join("sheets")).unwrap();
    std::fs::write(
        dir.path().join("sheets/power.kicad_sch"),
        "(kicad_sch (uuid 1234-abcd) (symbol (uuid \"sub-sym\")))",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("top-backups")).unwrap();
    std::fs::write(
        dir.path().join("top-backups/old.kicad_sch"),
        "(kicad_sch (uuid \"backup-only\"))",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join(".history")).unwrap();
    std::fs::write(
        dir.path().join(".history/old.kicad_sch"),
        "(kicad_sch (uuid \"history-only\"))",
    )
    .unwrap();
    let uuids = schematic_uuids(dir.path());
    for present in ["pin-r1-2", "sym-c1", "1234-abcd", "sub-sym", "root-sheet"] {
        assert!(uuids.contains(present), "{present}");
    }
    for absent in ["backup-only", "history-only"] {
        assert!(!uuids.contains(absent), "{absent}");
    }
}

// ─── ERC through the tools ────────────────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn erc_compare_reports_new_resolved_and_unchanged() {
    let scene = Scene::new("top.kicad_sch", SCHEMATIC);
    scene.serve(&erc_report(vec![
        r1_unconnected(10.0),
        c1_r2_conflict(40.0),
    ]));
    let saved = save(&scene, &erc_args(&scene, "before-edit")).await;
    assert!(!saved.is_error, "{}", text(&saved));
    let saved = body(&saved);
    assert_eq!(saved["violation_count"], 2);
    assert_eq!(saved["check"], "erc");
    assert_eq!(saved["keying"]["items_by_uuid"], 3);
    let path = scene.path(".konnect/baselines/erc/before-edit.json");
    assert_eq!(saved["path"], path.display().to_string());
    assert!(path.is_file());

    // The edit fixed R1 and left a dangling label; the conflict moved.
    scene.serve(&erc_report(vec![label_dangling(), c1_r2_conflict(80.0)]));
    let result = compare(&scene, &erc_args(&scene, "before-edit")).await;
    assert!(!result.is_error, "{}", text(&result));
    let response = body(&result);
    assert_eq!(response["new_count"], 1);
    assert_eq!(response["resolved_count"], 1);
    assert_eq!(response["unchanged_count"], 1);
    assert_eq!(response["verdict"], "regressed");
    assert_eq!(changes(&response, "new")[0]["rule"], "label_dangling");
    assert_eq!(
        changes(&response, "resolved")[0]["rule"],
        "pin_not_connected"
    );
    assert_eq!(response["baseline_count"], 2);
    assert_eq!(response["current_count"], 2);
    assert_eq!(response["new_by_rule"][0]["rule"], "label_dangling");
    assert_eq!(response["truncated"], false);
}

#[cfg(unix)]
#[tokio::test]
async fn an_erc_rerun_reordered_and_moved_matches_its_baseline_exactly() {
    let scene = Scene::new("top.kicad_sch", SCHEMATIC);
    scene.serve(&erc_report(vec![
        r1_unconnected(10.0),
        c1_r2_conflict(40.0),
        label_dangling(),
    ]));
    assert!(!save(&scene, &erc_args(&scene, "b")).await.is_error);

    // Same violations, other order, every coordinate shifted, items swapped.
    let mut conflict = c1_r2_conflict(123.4);
    conflict["items"].as_array_mut().unwrap().reverse();
    scene.serve(&erc_report(vec![
        label_dangling(),
        conflict,
        r1_unconnected(77.7),
    ]));
    let response = body(&compare(&scene, &erc_args(&scene, "b")).await);
    assert_eq!(response["new_count"], 0, "{response}");
    assert_eq!(response["resolved_count"], 0);
    assert_eq!(response["unchanged_count"], 3);
    assert_eq!(response["verdict"], "no_regressions");
    assert_eq!(response["changes"], json!([]));
}

#[cfg(unix)]
#[tokio::test]
async fn erc_uuids_kicad_made_up_for_one_run_do_not_break_the_match() {
    let scene = Scene::new("top.kicad_sch", SCHEMATIC);
    let run = |uuid: &str| {
        erc_report(vec![erc_violation(
            "power_pin_not_driven",
            "error",
            vec![erc_item(
                "Symbol U1 Pin 7 [Power input, Line]",
                uuid,
                1.0,
                1.0,
            )],
        )])
    };
    scene.serve(&run("not-in-the-file-1"));
    let saved = body(&save(&scene, &erc_args(&scene, "b")).await);
    assert_eq!(saved["keying"]["items_by_description"], 1);
    scene.serve(&run("not-in-the-file-2"));
    let response = body(&compare(&scene, &erc_args(&scene, "b")).await);
    assert_eq!(
        (
            response["new_count"].as_u64(),
            response["unchanged_count"].as_u64()
        ),
        (Some(0), Some(1))
    );
}

#[cfg(unix)]
#[tokio::test]
async fn the_severity_filter_hides_lesser_changes_but_counts_stay_honest() {
    let scene = Scene::new("top.kicad_sch", SCHEMATIC);
    scene.serve(&erc_report(vec![r1_unconnected(1.0)]));
    assert!(!save(&scene, &erc_args(&scene, "b")).await.is_error);
    scene.serve(&erc_report(vec![r1_unconnected(1.0), label_dangling()]));
    let mut args = erc_args(&scene, "b");
    args["severity"] = json!("error");
    let response = body(&compare(&scene, &args).await);
    assert_eq!(response["new_count"], 0, "a new warning is below 'error'");
    assert_eq!(response["unchanged_count"], 1);
    assert_eq!(
        response["current_count"], 2,
        "the run itself is counted whole"
    );
    args["severity"] = json!("fatal");
    assert!(compare(&scene, &args).await.is_error);
}

#[cfg(unix)]
#[tokio::test]
async fn an_existing_name_is_refused_before_the_check_runs_unless_overwritten() {
    let scene = Scene::new("top.kicad_sch", SCHEMATIC);
    scene.serve(&erc_report(vec![r1_unconnected(1.0)]));
    assert!(!save(&scene, &erc_args(&scene, "b")).await.is_error);
    assert_eq!(scene.runs(), 1);
    let path = scene.path(".konnect/baselines/erc/b.json");
    let first = std::fs::read_to_string(&path).unwrap();

    scene.serve(&erc_report(vec![]));
    let refused = save(&scene, &erc_args(&scene, "b")).await;
    assert!(refused.is_error);
    assert!(
        text(&refused).contains("already exists"),
        "{}",
        text(&refused)
    );
    assert_eq!(scene.runs(), 1, "nothing ran");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), first);

    let mut args = erc_args(&scene, "b");
    args["overwrite"] = json!(true);
    let replaced = body(&save(&scene, &args).await);
    assert_eq!(replaced["replaced"], true);
    assert_eq!(replaced["violation_count"], 0);
}

#[cfg(unix)]
#[tokio::test]
async fn bad_arguments_and_missing_baselines_are_refused_without_running() {
    let scene = Scene::new("top.kicad_sch", SCHEMATIC);
    scene.serve(&erc_report(vec![]));
    let sch = scene.path("top.kicad_sch").to_string_lossy().into_owned();
    for bad in [
        json!({ "schematic": sch, "name": "../escape" }),
        json!({ "schematic": sch, "name": ".hidden" }),
        json!({ "schematic": sch, "name": "" }),
        json!({ "schematic": sch }),
        json!({ "name": "b" }),
        json!({ "schematic": sch, "board": "x.kicad_pcb", "name": "b" }),
        json!({ "schematic": scene.path("missing.kicad_sch").to_string_lossy(), "name": "b" }),
        json!({ "schematic": scene.path("report.json").to_string_lossy(), "name": "b" }),
        json!({ "schematic": sch, "name": "b", "sync_live_board": true }),
        json!({ "schematic": sch, "name": "b", "overwrite": "yes" }),
    ] {
        assert!(save(&scene, &bad).await.is_error, "{bad}");
    }
    let missing = compare(&scene, &erc_args(&scene, "never-saved")).await;
    assert!(missing.is_error);
    assert!(
        text(&missing).contains("Stored: none"),
        "{}",
        text(&missing)
    );
    assert_eq!(scene.runs(), 0, "no refusal ran the check");
    assert!(!scene.path(".konnect").exists(), "no refusal left a folder");
}

#[cfg(unix)]
#[tokio::test]
async fn a_damaged_baseline_is_reported_and_left_alone() {
    let scene = Scene::new("top.kicad_sch", SCHEMATIC);
    scene.serve(&erc_report(vec![r1_unconnected(1.0)]));
    assert!(!save(&scene, &erc_args(&scene, "good")).await.is_error);
    let dir = scene.path(".konnect/baselines/erc");
    std::fs::write(dir.join("broken.json"), "{ not json").unwrap();
    let renamed = std::fs::read_to_string(dir.join("good.json"))
        .unwrap()
        .replace("\"name\": \"good\"", "\"name\": \"other\"");
    std::fs::write(dir.join("moved.json"), &renamed).unwrap();

    for name in ["broken", "moved"] {
        let result = compare(&scene, &erc_args(&scene, name)).await;
        assert!(result.is_error, "{name}");
        assert!(
            text(&result).contains("invalid_configuration"),
            "{}",
            text(&result)
        );
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("broken.json")).unwrap(),
        "{ not json"
    );

    let listed = body(
        &handle_list_check_baselines(
            &json!({ "schematic": scene.path("top.kicad_sch").to_string_lossy() }),
            &scene.ctx,
        )
        .await
        .unwrap(),
    );
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["baselines"][0]["name"], "good");
    assert_eq!(listed["baselines"][0]["source_state"], "unchanged");
    assert_eq!(listed["unreadable"].as_array().unwrap().len(), 2);
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_store_folder_is_refused() {
    let scene = Scene::new("top.kicad_sch", SCHEMATIC);
    scene.serve(&erc_report(vec![]));
    let elsewhere = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), scene.path(".konnect")).unwrap();
    let refused = save(&scene, &erc_args(&scene, "b")).await;
    assert!(refused.is_error);
    assert!(
        text(&refused).contains("not a plain folder"),
        "{}",
        text(&refused)
    );
    assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
    assert_eq!(scene.runs(), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn a_baseline_is_only_compared_with_its_own_file_and_reports_source_drift() {
    let scene = Scene::new("top.kicad_sch", SCHEMATIC);
    std::fs::write(scene.path("other.kicad_sch"), SCHEMATIC).unwrap();
    scene.serve(&erc_report(vec![]));
    assert!(!save(&scene, &erc_args(&scene, "b")).await.is_error);
    let wrong =
        json!({ "schematic": scene.path("other.kicad_sch").to_string_lossy(), "name": "b" });
    let refused = compare(&scene, &wrong).await;
    assert!(refused.is_error);
    assert!(
        text(&refused).contains("was taken on top.kicad_sch"),
        "{}",
        text(&refused)
    );

    std::fs::write(scene.path("top.kicad_sch"), format!("{SCHEMATIC}\n")).unwrap();
    let response = body(&compare(&scene, &erc_args(&scene, "b")).await);
    assert_eq!(response["source_changed_since_baseline"], true);
}

#[cfg(unix)]
#[tokio::test]
async fn a_large_comparison_stays_under_the_cap_and_output_holds_everything() {
    let scene = Scene::new("top.kicad_sch", SCHEMATIC);
    scene.serve(&erc_report(vec![]));
    assert!(!save(&scene, &erc_args(&scene, "empty")).await.is_error);
    let many: Vec<Value> = (0..1500)
        .map(|i| {
            erc_violation(
                "pin_not_connected",
                if i % 3 == 0 { "error" } else { "warning" },
                vec![erc_item(
                    &format!("Symbol R{i} Pin 1 [Passive, Line], and this text is long on purpose"),
                    &format!("made-up-{i}"),
                    i as f64,
                    0.0,
                )],
            )
        })
        .collect();
    scene.serve(&erc_report(many));
    let output = scene.path("out/compare.json");
    let mut args = erc_args(&scene, "empty");
    args["limit"] = json!(5000);
    args["output"] = json!(output.to_string_lossy());
    let result = compare(&scene, &args).await;
    assert!(
        text(&result).len() < CLIENT_CAP,
        "{} characters",
        text(&result).len()
    );
    let response = body(&result);
    assert_eq!(response["new_count"], 1500);
    assert_eq!(response["truncated"], true);
    assert_eq!(response["byte_capped"], true);
    let tallied: u64 = response["new_by_rule"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["count"].as_u64().unwrap())
        .sum();
    assert_eq!(tallied, 1500);

    // Paging covers every change once.
    let mut seen = std::collections::HashSet::new();
    let mut offset = 0u64;
    loop {
        let mut page_args = erc_args(&scene, "empty");
        page_args["offset"] = json!(offset);
        page_args["limit"] = json!(400);
        let page = body(&compare(&scene, &page_args).await);
        for change in page["changes"].as_array().unwrap() {
            assert!(seen.insert(change["key"].as_str().unwrap().to_string()));
        }
        match page["next_offset"].as_u64() {
            Some(next) => offset = next,
            None => break,
        }
    }
    assert_eq!(seen.len(), 1500);

    let full: Value = serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
    assert_eq!(full["new"].as_array().unwrap().len(), 1500);
}

// ─── DRC through the tools ────────────────────────────────────────────────────

const BOARD: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (footprint "R_0603" (layer "F.Cu") (uuid "fp-r1")
    (property "Reference" "R1")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (uuid "pad-r1-1"))
    (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (uuid "pad-r1-2")))
  (segment (start 0 0) (end 5 0) (width 0.2) (layer "F.Cu") (net 1) (uuid "seg-a"))
  (segment (start 0 1) (end 5 1) (width 0.2) (layer "F.Cu") (net 2) (uuid "seg-b"))
  (via (at 3 3) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (uuid "via-a"))
)"#;

fn drc_item(description: &str, uuid: &str, x: f64) -> Value {
    json!({ "description": description, "uuid": uuid, "pos": { "x": x, "y": 1.0 } })
}

fn drc_violation(rule: &str, severity: &str, description: &str, items: Vec<Value>) -> Value {
    json!({ "type": rule, "severity": severity, "description": description, "items": items })
}

fn clearance(actual: &str, x: f64) -> Value {
    drc_violation(
        "clearance",
        "error",
        &format!(
            "Clearance violation (netclass 'Default' clearance 0.2000 mm; actual {actual} mm)"
        ),
        vec![
            drc_item("Track [N1] on F.Cu, length 5.0000 mm", "seg-a", x),
            drc_item("Track [N2] on F.Cu, length 5.0000 mm", "seg-b", x),
        ],
    )
}

fn unrouted() -> Value {
    drc_violation(
        "unconnected_items",
        "error",
        "Missing connection between items",
        vec![
            drc_item("Pad 1 [N1] of R1 on F.Cu", "pad-r1-1", 0.0),
            drc_item("Via [N1] on F.Cu - B.Cu", "via-a", 3.0),
        ],
    )
}

fn drc_report(violations: Vec<Value>, unconnected: Option<Vec<Value>>) -> Value {
    let mut report = json!({ "violations": violations, "schematic_parity": [] });
    if let Some(unconnected) = unconnected {
        report["unconnected_items"] = json!(unconnected);
    }
    report
}

fn drc_args(scene: &Scene, name: &str) -> Value {
    json!({ "board": scene.path("board.kicad_pcb").to_string_lossy(), "name": name })
}

#[cfg(unix)]
#[tokio::test]
async fn drc_compare_matches_by_board_uuid_not_by_measured_prose() {
    let scene = Scene::new("board.kicad_pcb", BOARD);
    scene.serve(&drc_report(
        vec![clearance("0.1500", 1.0)],
        Some(vec![unrouted()]),
    ));
    let saved = save(&scene, &drc_args(&scene, "pre-route")).await;
    assert!(!saved.is_error, "{}", text(&saved));
    let saved = body(&saved);
    assert_eq!(saved["check"], "drc");
    assert_eq!(saved["violation_count"], 2);
    assert_eq!(saved["keying"]["items_by_uuid"], 4);
    assert_eq!(saved["source_evidence"]["source"], "saved_file");
    assert!(scene
        .path(".konnect/baselines/drc/pre-route.json")
        .is_file());

    // Routing fixed the connection; the clearance got worse and moved, which
    // changes its description and positions but not what it is.
    scene.serve(&drc_report(vec![clearance("0.0500", 9.0)], Some(vec![])));
    let response = body(&compare(&scene, &drc_args(&scene, "pre-route")).await);
    assert_eq!(response["new_count"], 0, "{response}");
    assert_eq!(response["resolved_count"], 1);
    assert_eq!(response["unchanged_count"], 1);
    assert_eq!(response["verdict"], "no_regressions");
    let resolved = &changes(&response, "resolved")[0];
    assert_eq!(resolved["category"], "unconnected_items");
    assert_eq!(response["warnings"], json!([]));
}

#[cfg(unix)]
#[tokio::test]
async fn a_drc_item_uuid_the_board_does_not_hold_is_keyed_by_description() {
    // KiCad 8 does not save pad UUIDs, so each run makes new ones up.
    let scene = Scene::new("board.kicad_pcb", BOARD);
    let run = |uuid: &str| {
        drc_report(
            vec![],
            Some(vec![drc_violation(
                "unconnected_items",
                "error",
                "Missing connection between items",
                vec![
                    drc_item("Pad 2 [N2] of R1 on F.Cu", uuid, 1.0),
                    drc_item("Track [N2] on F.Cu, length 5.0000 mm", "seg-b", 1.0),
                ],
            )]),
        )
    };
    scene.serve(&run("pad-made-up-1"));
    let saved = body(&save(&scene, &drc_args(&scene, "b")).await);
    assert_eq!(saved["keying"]["items_by_uuid"], 1);
    assert_eq!(saved["keying"]["items_by_description"], 1);
    scene.serve(&run("pad-made-up-2"));
    let response = body(&compare(&scene, &drc_args(&scene, "b")).await);
    assert_eq!(
        (
            response["new_count"].as_u64(),
            response["unchanged_count"].as_u64()
        ),
        (Some(0), Some(1)),
        "{response}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_drc_category_missing_from_one_run_is_named_not_resolved() {
    let scene = Scene::new("board.kicad_pcb", BOARD);
    scene.serve(&drc_report(vec![], Some(vec![unrouted()])));
    assert!(!save(&scene, &drc_args(&scene, "b")).await.is_error);
    scene.serve(&drc_report(vec![], None));
    let response = body(&compare(&scene, &drc_args(&scene, "b")).await);
    assert_eq!(response["resolved_count"], 0);
    assert_eq!(
        response["categories_not_compared"],
        json!(["unconnected_items"])
    );
    assert!(response["warnings"][0]
        .as_str()
        .unwrap()
        .starts_with("categories_not_compared"));
}

#[cfg(unix)]
#[tokio::test]
async fn drc_options_that_differ_from_the_baseline_are_warned_about() {
    let scene = Scene::new("board.kicad_pcb", BOARD);
    scene.serve(&drc_report(vec![], Some(vec![])));
    assert!(!save(&scene, &drc_args(&scene, "b")).await.is_error);
    let mut args = drc_args(&scene, "b");
    args["refill_zones"] = json!(true);
    let response = body(&compare(&scene, &args).await);
    assert!(
        response["warnings"][0]
            .as_str()
            .unwrap()
            .starts_with("options_differ"),
        "{response}"
    );
    // An ERC baseline of the same name is a different baseline.
    let erc = json!({ "schematic": scene.path("board.kicad_pcb").to_string_lossy(), "name": "b" });
    assert!(compare(&scene, &erc).await.is_error, "extension is checked");
}

#[cfg(unix)]
#[tokio::test]
async fn the_list_holds_both_checks_newest_first() {
    let scene = Scene::new("board.kicad_pcb", BOARD);
    std::fs::write(scene.path("board.kicad_sch"), SCHEMATIC).unwrap();
    scene.serve(&drc_report(vec![clearance("0.1", 1.0)], Some(vec![])));
    assert!(!save(&scene, &drc_args(&scene, "drc-one")).await.is_error);
    std::thread::sleep(std::time::Duration::from_millis(5));
    scene.serve(&erc_report(vec![r1_unconnected(1.0)]));
    let erc =
        json!({ "schematic": scene.path("board.kicad_sch").to_string_lossy(), "name": "erc-one" });
    assert!(!save(&scene, &erc).await.is_error);
    std::fs::write(scene.path("board.kicad_pcb"), format!("{BOARD}\n")).unwrap();

    let listed = body(
        &handle_list_check_baselines(
            &json!({ "project_dir": scene.dir.path().to_string_lossy() }),
            &scene.ctx,
        )
        .await
        .unwrap(),
    );
    let rows = listed["baselines"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (rows[0]["name"].as_str(), rows[0]["check"].as_str()),
        (Some("erc-one"), Some("erc"))
    );
    assert_eq!(rows[1]["source_state"], "changed");
    assert_eq!(rows[1]["errors"], 1);
    let none = handle_list_check_baselines(&json!({}), &scene.ctx)
        .await
        .unwrap();
    assert!(none.is_error);
}

// ─── Registration ─────────────────────────────────────────────────────────────

#[test]
fn the_tools_are_in_the_verification_toolset_and_dispatch_checks_names() {
    let tools = crate::router::registry::tools_for("verification").unwrap();
    for name in [
        "save_check_baseline",
        "compare_check_baseline",
        "list_check_baselines",
    ] {
        assert!(tools.iter().any(|t| t.name == name), "{name}");
    }
    let save = tools
        .iter()
        .find(|t| t.name == "save_check_baseline")
        .unwrap();
    let validator = crate::tools::compile_input_validator(save.name, &save.input_schema);
    assert!(validator.is_valid(&json!({ "schematic": "a.kicad_sch", "name": "pre-edit.1" })));
    assert!(!validator.is_valid(&json!({ "schematic": "a.kicad_sch", "name": "../x" })));
}

// ─── Live kicad-cli ───────────────────────────────────────────────────────────

/// Two real kicad-cli runs on an unchanged scratch copy must match exactly:
/// the case a hand diff got wrong whenever KiCad reordered its report. Point
/// `KONNECT_BASELINE_DEMO` at a project folder whose `.kicad_sch` and
/// `.kicad_pcb` share its name (a KiCad demo, say); it is copied first, never
/// checked in place.
///
///     KONNECT_BASELINE_DEMO=<kicad>/demos/pic_programmer \
///     KICAD_CLI=/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli \
///     cargo test -p konnect-core --lib live_rerun -- --ignored --nocapture
#[tokio::test]
#[ignore = "needs a real kicad-cli and KONNECT_BASELINE_DEMO"]
async fn live_rerun_of_an_unchanged_project_matches_its_baseline() {
    let source =
        PathBuf::from(std::env::var("KONNECT_BASELINE_DEMO").expect("KONNECT_BASELINE_DEMO"));
    let stem = source.file_name().unwrap().to_string_lossy().into_owned();
    let scratch = tempfile::tempdir().unwrap();
    for entry in std::fs::read_dir(&source).unwrap().flatten() {
        if entry.file_type().unwrap().is_file() {
            std::fs::copy(entry.path(), scratch.path().join(entry.file_name())).unwrap();
        }
    }
    let cli = std::env::var("KICAD_CLI").unwrap_or_else(|_| "kicad-cli".into());
    let ctx = ctx_with_cli(Path::new(&cli));
    for (field, extension) in [("schematic", "kicad_sch"), ("board", "kicad_pcb")] {
        let target = scratch.path().join(format!("{stem}.{extension}"));
        let args = json!({ field: target.to_string_lossy(), "name": "live" });
        let saved = handle_save_check_baseline(&args, &ctx).await.unwrap();
        assert!(!saved.is_error, "{}", text(&saved));
        let saved = body(&saved);
        let compared = handle_compare_check_baseline(
            &json!({ field: target.to_string_lossy(), "name": "live", "severity": "info" }),
            &ctx,
        )
        .await
        .unwrap();
        assert!(!compared.is_error, "{}", text(&compared));
        let compared = body(&compared);
        println!(
            "{field}: {} violations, keying {}, compare new {} resolved {} unchanged {}",
            saved["violation_count"],
            compared["keying"],
            compared["new_count"],
            compared["resolved_count"],
            compared["unchanged_count"]
        );
        assert_eq!(compared["new_count"], 0, "{compared}");
        assert_eq!(compared["resolved_count"], 0, "{compared}");
        assert_eq!(compared["severity_changed_count"], 0, "{compared}");
        assert_eq!(compared["unchanged_count"], saved["violation_count"]);
    }
}
