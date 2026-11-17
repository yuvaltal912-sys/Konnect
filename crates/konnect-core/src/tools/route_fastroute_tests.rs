//! Tests of the fastroute engine's handler glue (`route_fastroute.rs`). The
//! scheduler, the judge and the process runner have their own tests; here the
//! engine switch, the user config reader, the refusals that come before anything
//! routes, the folder clean-up and the result's shape are checked. Each guard has
//! a refusal test and an honest-case test: that pair is its negative control.

use super::*;
use crate::fastroute::Held;
use crate::route_judge::{Gates, Measures};
use crate::route_search::Lanes;
use crate::router::ToolRouter;
use crate::tools::integration::handle_route_specctra_dsn;
use crate::tools::ServerConfig;
use std::collections::BTreeMap;
use std::time::Duration;

fn ctx() -> Arc<ToolContext> {
    Arc::new(ToolContext::new(
        ServerConfig::default(),
        Arc::new(ToolRouter::new()),
    ))
}

fn body(result: &CallToolResult) -> Value {
    let ToolContent::Text { text } = &result.content[0] else {
        panic!("expected text");
    };
    serde_json::from_str(text).unwrap_or_else(|_| panic!("not JSON: {text}"))
}

fn kind(result: &CallToolResult) -> String {
    body(result)["error"]["kind"]
        .as_str()
        .unwrap_or_else(|| panic!("no error kind in {:?}", result.content))
        .to_string()
}

fn message(result: &CallToolResult) -> String {
    body(result)["message"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// `{ key: value }`, the key a variable.
fn one(key: &str, value: &Value) -> Value {
    let mut args = json!({});
    args[key] = value.clone();
    args
}

fn refused(args: Value) -> CallToolResult {
    route_engine(&args).expect_err("the call is refused")
}

// ─── route_engine ────────────────────────────────────────────────────────────

#[test]
fn the_default_engine_is_fastroute() {
    assert_eq!(route_engine(&json!({})).ok(), Some(Engine::Fastroute));
    assert_eq!(
        route_engine(&json!({ "engine": null })).ok(),
        Some(Engine::Fastroute)
    );
    assert_eq!(
        route_engine(&json!({ "engine": "fastroute" })).ok(),
        Some(Engine::Fastroute)
    );
}

#[test]
fn freerouting_is_chosen_by_name() {
    assert_eq!(
        route_engine(&json!({ "engine": "freerouting" })).ok(),
        Some(Engine::Freerouting)
    );
}

#[test]
fn an_engine_that_is_not_one_of_the_two_is_refused() {
    for value in [json!("freerouter"), json!("Fastroute"), json!(""), json!(5)] {
        let result = refused(json!({ "engine": value }));
        assert!(result.is_error);
        assert_eq!(kind(&result), "invalid_argument", "{value}");
        assert_eq!(body(&result)["error"]["field"], "engine");
    }
}

#[test]
fn every_freerouting_argument_is_refused_by_name_under_fastroute() {
    let samples = [
        ("jar_path", json!("/x/freerouting.jar")),
        ("java_path", json!("/x/java")),
        ("max_passes", json!(20)),
        ("optimizer_enabled", json!(true)),
        ("job_timeout_seconds", json!(60)),
        ("overall_timeout_seconds", json!(900)),
    ];
    assert_eq!(
        samples.len(),
        FREEROUTING_ONLY.len(),
        "a sample for every Freerouting-only argument"
    );
    for (name, value) in samples {
        assert!(FREEROUTING_ONLY.contains(&name), "{name}");
        // By default, and with the engine named.
        for engine in [None, Some("fastroute")] {
            let mut args = one(name, &value);
            if let Some(engine) = engine {
                args["engine"] = json!(engine);
            }
            let result = refused(args);
            assert_eq!(kind(&result), "invalid_argument", "{name}");
            assert_eq!(body(&result)["error"]["field"], json!(name));
            let text = message(&result);
            assert!(text.contains(name), "{text}");
            assert!(text.contains("engine: \"freerouting\""), "{text}");
        }
        // The honest case: the same argument is what Freerouting reads.
        let mut own = one(name, &value);
        own["engine"] = json!("freerouting");
        assert_eq!(route_engine(&own).ok(), Some(Engine::Freerouting), "{name}");
    }
}

#[test]
fn several_freerouting_arguments_are_all_named() {
    let result = refused(json!({ "jar_path": "/x.jar", "max_passes": 5 }));
    let text = message(&result);
    assert!(
        text.contains("jar_path") && text.contains("max_passes"),
        "{text}"
    );
}

#[test]
fn allow_neckdown_true_is_refused_and_false_is_accepted_under_fastroute() {
    let result = refused(json!({ "allow_neckdown": true }));
    assert_eq!(kind(&result), "invalid_argument");
    assert_eq!(body(&result)["error"]["field"], "allow_neckdown");
    assert!(message(&result).contains("allow_neckdown true"));
    assert_eq!(
        route_engine(&json!({ "allow_neckdown": false })).ok(),
        Some(Engine::Fastroute)
    );
    // Freerouting reads it either way.
    assert_eq!(
        route_engine(&json!({ "engine": "freerouting", "allow_neckdown": true })).ok(),
        Some(Engine::Freerouting)
    );
}

#[test]
fn width_policy_retry_is_refused_and_hold_is_accepted_under_fastroute() {
    let result = refused(json!({ "width_policy": "retry" }));
    assert_eq!(kind(&result), "invalid_argument");
    assert_eq!(body(&result)["error"]["field"], "width_policy");
    assert!(message(&result).contains("retry"));
    assert_eq!(
        route_engine(&json!({ "width_policy": "hold" })).ok(),
        Some(Engine::Fastroute)
    );
    assert_eq!(
        route_engine(&json!({ "width_policy": "hold", "width_fallback": "none" })).ok(),
        Some(Engine::Fastroute)
    );
    assert_eq!(
        route_engine(&json!({ "engine": "freerouting", "width_policy": "retry" })).ok(),
        Some(Engine::Freerouting)
    );
}

#[test]
fn every_fastroute_argument_is_refused_by_name_under_freerouting() {
    let samples = [
        ("effort", json!("m")),
        ("fastroute_path", json!("/x/fastroute")),
    ];
    assert_eq!(samples.len(), FASTROUTE_ONLY.len());
    for (name, value) in samples {
        assert!(FASTROUTE_ONLY.contains(&name), "{name}");
        let mut foreign = one(name, &value);
        foreign["engine"] = json!("freerouting");
        let result = refused(foreign);
        assert_eq!(kind(&result), "invalid_argument", "{name}");
        assert_eq!(body(&result)["error"]["field"], json!(name));
        let text = message(&result);
        assert!(text.contains(name), "{text}");
        assert!(text.contains("fastroute engine"), "{text}");
        // The honest case: fastroute reads it.
        assert_eq!(
            route_engine(&one(name, &value)).ok(),
            Some(Engine::Fastroute),
            "{name}"
        );
    }
}

// ─── user_routing_config ─────────────────────────────────────────────────────

fn config_file(text: Option<&str>) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    if let Some(text) = text {
        std::fs::write(&path, text).unwrap();
    }
    (dir, path)
}

#[test]
fn a_missing_config_file_gives_nothing() {
    let (_dir, path) = config_file(None);
    assert_eq!(routing_config_at(&path).unwrap(), (None, None));
}

#[test]
fn a_config_without_the_routing_keys_gives_nothing() {
    for text in [
        "{}",
        r#"{"design_rules": []}"#,
        r#"{"routing": {}}"#,
        r#"{"routing": null}"#,
        r#"{"routing": {"effort": null, "fastroute_path": null}}"#,
    ] {
        let (_dir, path) = config_file(Some(text));
        assert_eq!(routing_config_at(&path).unwrap(), (None, None), "{text}");
    }
}

#[test]
fn both_routing_keys_are_read() {
    let (_dir, path) = config_file(Some(
        r#"{"routing": {"effort": "h", "fastroute_path": "/opt/fastroute"}}"#,
    ));
    assert_eq!(
        routing_config_at(&path).unwrap(),
        (Some(json!("h")), Some(PathBuf::from("/opt/fastroute")))
    );
    let (_dir, path) = config_file(Some(r#"{"routing": {"effort": "xh"}}"#));
    assert_eq!(routing_config_at(&path).unwrap(), (Some(json!("xh")), None));
}

#[test]
fn an_effort_that_is_no_level_is_passed_on_to_be_refused_there() {
    // The reader gives the value; `resolve_effort` judges it and names the source.
    let (_dir, path) = config_file(Some(r#"{"routing": {"effort": "ludicrous"}}"#));
    assert_eq!(
        routing_config_at(&path).unwrap().0,
        Some(json!("ludicrous"))
    );
}

#[test]
fn a_config_that_cannot_be_used_is_an_error_and_never_the_defaults() {
    for text in [
        "{not json",
        "[1, 2]",
        "\"text\"",
        r#"{"routing": "fast"}"#,
        r#"{"routing": {"fastroute_path": 7}}"#,
        r#"{"routing": {"fastroute_path": ""}}"#,
    ] {
        let (_dir, path) = config_file(Some(text));
        let error = routing_config_at(&path).expect_err(text);
        assert!(
            format!("{error:#}").contains(&path.display().to_string()),
            "{text}: {error:#}"
        );
    }
}

#[test]
fn a_config_path_that_is_a_folder_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(routing_config_at(dir.path()).is_err());
}

// ─── the handler's refusals ──────────────────────────────────────────────────

struct Paths {
    _dir: tempfile::TempDir,
    root: PathBuf,
    dsn: PathBuf,
    ses: PathBuf,
    folder: PathBuf,
}

impl Paths {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let dsn = root.join("board.dsn");
        std::fs::write(&dsn, "(pcb board)").unwrap();
        Paths {
            ses: root.join("out.ses"),
            folder: root.join("out.search"),
            root,
            dsn,
            _dir: dir,
        }
    }

    /// Arguments that reach the search folder's creation and then the locator
    /// (the explicit path is not a file).
    fn args(&self) -> Value {
        json!({
            "dsn_path": self.dsn,
            "ses_output_path": self.ses,
            "effort": "fast",
            "fastroute_path": self.root.join("no-such-fastroute"),
        })
    }

    /// Nothing of the call is left: no output, no search folder, none set aside.
    fn assert_untouched(&self) {
        assert!(!self.ses.exists(), "the output was written");
        assert!(!self.folder.exists(), "the search folder was left");
        let mut aside = self.folder.as_os_str().to_os_string();
        aside.push(".failed-1");
        assert!(
            PathBuf::from(aside).symlink_metadata().is_err(),
            "an empty search folder was set aside"
        );
    }
}

#[tokio::test]
async fn a_bad_effort_is_refused_before_anything_is_made() {
    let paths = Paths::new();
    let mut args = paths.args();
    args["effort"] = json!("ludicrous");
    let result = handle_route_specctra_dsn(&args, ctx()).await.unwrap();
    assert!(result.is_error);
    assert_eq!(kind(&result), "invalid_argument");
    assert_eq!(body(&result)["error"]["field"], "effort");
    assert!(
        message(&result).contains("ludicrous"),
        "{}",
        message(&result)
    );
    paths.assert_untouched();
}

#[tokio::test]
async fn a_good_effort_goes_on_to_look_for_fastroute() {
    let paths = Paths::new();
    let result = handle_route_specctra_dsn(&paths.args(), ctx())
        .await
        .unwrap();
    assert!(result.is_error);
    // Not the effort: the named binary is missing.
    assert_ne!(kind(&result), "invalid_argument", "{}", message(&result));
    paths.assert_untouched();
}

#[tokio::test]
async fn a_missing_fastroute_is_refused_with_setup_and_nothing_is_left() {
    let paths = Paths::new();
    let result = handle_route_specctra_dsn(&paths.args(), ctx())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        ["file_not_found", "handler_error"].contains(&kind(&result).as_str()),
        "{}",
        message(&result)
    );
    let setup = &body(&result)["setup"];
    assert!(setup["problem"].is_string(), "{setup}");
    let ways = setup["name_a_binary"].to_string();
    assert!(ways.contains("fastroute_path"), "{ways}");
    assert!(ways.contains("KONNECT_FASTROUTE"), "{ways}");
    assert!(ways.contains("routing.fastroute_path"), "{ways}");
    assert!(
        setup["alternative"]
            .as_str()
            .unwrap()
            .contains("engine: \"freerouting\""),
        "{setup}"
    );
    paths.assert_untouched();
}

#[tokio::test]
async fn an_existing_output_is_refused_and_left_alone() {
    let paths = Paths::new();
    std::fs::write(&paths.ses, "keep me").unwrap();
    let result = handle_route_specctra_dsn(&paths.args(), ctx())
        .await
        .unwrap();
    assert_eq!(kind(&result), "conflict");
    assert!(
        message(&result).contains("already exists"),
        "{}",
        message(&result)
    );
    assert_eq!(std::fs::read_to_string(&paths.ses).unwrap(), "keep me");
    assert!(!paths.folder.exists());
}

#[tokio::test]
async fn an_existing_search_folder_is_refused_and_left_alone() {
    let paths = Paths::new();
    std::fs::create_dir(&paths.folder).unwrap();
    std::fs::write(paths.folder.join("earlier.txt"), "keep me").unwrap();
    let result = handle_route_specctra_dsn(&paths.args(), ctx())
        .await
        .unwrap();
    assert_eq!(kind(&result), "conflict");
    assert!(
        message(&result).contains("search folder already exists"),
        "{}",
        message(&result)
    );
    assert_eq!(
        std::fs::read_to_string(paths.folder.join("earlier.txt")).unwrap(),
        "keep me"
    );
    assert!(!paths.ses.exists());
}

#[tokio::test]
async fn an_input_that_is_not_a_dsn_file_is_refused() {
    let paths = Paths::new();
    let mut args = paths.args();
    args["dsn_path"] = json!(paths.root.join("missing.dsn"));
    let result = handle_route_specctra_dsn(&args, ctx()).await.unwrap();
    assert_eq!(kind(&result), "invalid_argument");
    assert_eq!(body(&result)["error"]["field"], "dsn_path");
    paths.assert_untouched();
}

#[tokio::test]
async fn an_output_without_the_ses_extension_is_refused() {
    let paths = Paths::new();
    let mut args = paths.args();
    args["ses_output_path"] = json!(paths.root.join("out.txt"));
    let result = handle_route_specctra_dsn(&args, ctx()).await.unwrap();
    assert_eq!(kind(&result), "invalid_argument");
    assert_eq!(body(&result)["error"]["field"], "ses_output_path");
    assert!(!paths.root.join("out.search").exists());
}

#[tokio::test]
async fn a_width_fallback_outside_the_two_is_refused_and_none_is_accepted() {
    let paths = Paths::new();
    let mut args = paths.args();
    args["width_fallback"] = json!("sometimes");
    let result = handle_route_specctra_dsn(&args, ctx()).await.unwrap();
    assert_eq!(kind(&result), "invalid_argument");
    assert_eq!(body(&result)["error"]["field"], "width_fallback");
    // The honest case gets as far as looking for the binary.
    args["width_fallback"] = json!("none");
    let result = handle_route_specctra_dsn(&args, ctx()).await.unwrap();
    assert_ne!(kind(&result), "invalid_argument", "{}", message(&result));
    paths.assert_untouched();
}

#[tokio::test]
async fn a_foreign_argument_is_refused_through_the_handler_before_anything_is_made() {
    let paths = Paths::new();
    let mut args = paths.args();
    args["jar_path"] = json!("/x/freerouting.jar");
    let result = handle_route_specctra_dsn(&args, ctx()).await.unwrap();
    assert_eq!(kind(&result), "invalid_argument");
    assert_eq!(body(&result)["error"]["field"], "jar_path");
    paths.assert_untouched();
}

#[tokio::test]
async fn the_freerouting_engine_refuses_the_fastroute_arguments_through_the_handler() {
    let paths = Paths::new();
    let mut args = paths.args();
    args["engine"] = json!("freerouting");
    let result = handle_route_specctra_dsn(&args, ctx()).await.unwrap();
    assert_eq!(kind(&result), "invalid_argument");
    assert_eq!(body(&result)["error"]["field"], "effort");
    paths.assert_untouched();
}

fn fake_binary(root: &Path) -> Binary {
    Binary {
        path: root.join("fastroute"),
        sha256: "00".repeat(32),
        version: "fastroute test".to_string(),
        source: "argument",
    }
}

#[tokio::test]
async fn a_dsn_with_no_manifest_beside_it_is_refused_before_anything_routes() {
    let paths = Paths::new();
    let args = paths.args();
    let call = prepare_call(&args)
        .unwrap()
        .expect("the arguments are good");
    // The search folder exists from here on.
    assert!(call.folder.is_dir());
    let result = route_located(&args, ctx(), call, Located::Found(fake_binary(&paths.root)))
        .await
        .unwrap();
    assert!(result.is_error);
    assert_eq!(kind(&result), "handler_error");
    let refusal = body(&result);
    assert!(
        refusal["next_step"]
            .as_str()
            .is_some_and(|text| !text.is_empty()),
        "{refusal}"
    );
    assert!(
        message(&result).contains("Nothing was routed"),
        "{}",
        message(&result)
    );
    paths.assert_untouched();
}

#[test]
fn good_arguments_create_the_search_folder_and_name_it() {
    let paths = Paths::new();
    let call = prepare_call(&paths.args())
        .unwrap()
        .expect("good arguments");
    assert_eq!(call.folder, paths.folder);
    assert!(paths.folder.is_dir());
    assert_eq!(call.effort, (Effort::Fast, EffortSource::Argument));
    assert_eq!(call.fallback, WidthFallback::Floor);
    // Asked for again, the same call is refused: the folder is there.
    let again = prepare_call(&paths.args()).unwrap().err().expect("refused");
    assert_eq!(kind(&again), "conflict");
}

#[test]
fn the_output_folder_is_made_when_it_does_not_exist() {
    let paths = Paths::new();
    let mut args = paths.args();
    args["ses_output_path"] = json!(paths.root.join("new").join("deeper").join("out.ses"));
    let call = prepare_call(&args).unwrap().expect("good arguments");
    assert!(call.folder.is_dir());
    assert_eq!(call.folder, paths.root.join("new/deeper/out.search"));
}

// ─── Unavailable binaries ────────────────────────────────────────────────────

#[test]
fn each_way_a_binary_can_be_unavailable_carries_the_setup_steps() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("not-the-candidate");
    std::fs::write(&file, "x").unwrap();
    let cases = [
        (
            Located::NotConfigured {
                default_path: dir.path().join("default/fastroute"),
            },
            "file_not_found",
        ),
        (
            Located::Refused {
                path: dir.path().join("missing"),
                source: "argument",
                reason: "not a file".into(),
            },
            "file_not_found",
        ),
        (
            Located::Refused {
                path: file,
                source: "environment",
                reason: "not the candidate build".into(),
            },
            "handler_error",
        ),
        (
            Located::Unsupported {
                reason: "macOS only".into(),
            },
            "handler_error",
        ),
    ];
    for (located, expected) in cases {
        let result = unavailable_result(&located);
        assert!(result.is_error);
        assert_eq!(kind(&result), expected, "{located:?}");
        let setup = &body(&result)["setup"];
        assert!(setup["problem"].is_string(), "{located:?}");
        assert!(
            setup["alternative"]
                .as_str()
                .unwrap()
                .contains("engine: \"freerouting\""),
            "{located:?}"
        );
        assert!(
            message(&result).contains("Nothing was routed"),
            "{}",
            message(&result)
        );
    }
}

#[test]
fn the_setup_names_the_default_place_the_three_ways_and_the_alternative() {
    let guidance = setup_guidance(&Located::NotConfigured {
        default_path: PathBuf::from("/home/u/.local/share/fastroute/candidate/fastroute"),
    });
    assert_eq!(
        guidance["default_place"],
        "/home/u/.local/share/fastroute/candidate/fastroute"
    );
    let text = guidance.to_string();
    for needle in [
        "fastroute_path",
        "KONNECT_FASTROUTE",
        "routing.fastroute_path",
        "engine: \\\"freerouting\\\"",
        "Nothing is downloaded",
        "C** + BR1 + W8P",
    ] {
        assert!(text.contains(needle), "{needle} is missing from {text}");
    }
}

// ─── Setting a failed folder aside ───────────────────────────────────────────

#[test]
fn a_folder_that_holds_no_file_is_removed_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("out.search");
    std::fs::create_dir_all(folder.join("probe/home")).unwrap();
    std::fs::create_dir_all(folder.join("judge")).unwrap();
    let neighbour = dir.path().join("neighbour");
    std::fs::create_dir(&neighbour).unwrap();
    let note = set_aside(&folder);
    assert!(note.contains("no file"), "{note}");
    assert!(!folder.exists());
    assert!(neighbour.is_dir());
}

#[test]
fn a_folder_that_holds_a_file_is_renamed_never_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("out.search");
    for n in 1..=2 {
        std::fs::create_dir_all(folder.join("judge")).unwrap();
        std::fs::write(folder.join("judge/record.json"), format!("call {n}")).unwrap();
        let note = set_aside(&folder);
        let aside = dir.path().join(format!("out.search.failed-{n}"));
        assert!(!folder.exists(), "{note}");
        assert!(note.contains(&aside.display().to_string()), "{note}");
        assert_eq!(
            std::fs::read_to_string(aside.join("judge/record.json")).unwrap(),
            format!("call {n}")
        );
    }
}

#[cfg(unix)]
#[test]
fn a_name_that_is_taken_is_skipped_even_when_it_is_a_broken_link() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("out.search");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("keep.txt"), "x").unwrap();
    std::os::unix::fs::symlink(
        dir.path().join("nowhere"),
        dir.path().join("out.search.failed-1"),
    )
    .unwrap();
    let note = set_aside(&folder);
    assert!(
        dir.path().join("out.search.failed-2/keep.txt").is_file(),
        "{note}"
    );
}

#[test]
fn a_failed_call_that_holds_files_says_where_they_went() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("out.search");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("log"), "x").unwrap();
    let refusal = CallToolResult::error_kind(
        ToolErrorKind::HandlerError { reason: "r".into() },
        "no run was judged",
    );
    let finished = finish(&folder, Ok(refusal)).unwrap();
    assert!(finished.is_error);
    assert!(
        body(&finished)["search_folder"]
            .as_str()
            .unwrap()
            .contains("out.search.failed-1"),
        "{}",
        body(&finished)
    );
    assert!(dir.path().join("out.search.failed-1/log").is_file());
    // An error is passed on with the note too.
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("log"), "y").unwrap();
    let error = finish(&folder, Err(anyhow::anyhow!("the disk is full"))).unwrap_err();
    let text = format!("{error:#}");
    assert!(
        text.contains("the disk is full") && text.contains("failed-2"),
        "{text}"
    );
}

#[test]
fn a_success_keeps_its_search_folder() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("out.search");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("search.json"), "{}").unwrap();
    let finished = finish(
        &folder,
        Ok(CallToolResult::json(&json!({ "success": true }))),
    )
    .unwrap();
    assert!(!finished.is_error);
    assert!(folder.join("search.json").is_file());
}

#[test]
fn fields_are_added_to_a_structured_body_and_a_plain_text_body_is_left_alone() {
    let structured = with_fields(
        CallToolResult::error_kind(ToolErrorKind::HandlerError { reason: "r".into() }, "m"),
        vec![("setup", json!({ "a": 1 }))],
    );
    let parsed = body(&structured);
    assert_eq!(parsed["setup"]["a"], 1);
    assert_eq!(parsed["message"], "m");
    assert!(structured.is_error);
    let plain = with_fields(
        CallToolResult::error("plain words"),
        vec![("setup", json!(1))],
    );
    let ToolContent::Text { text } = &plain.content[0] else {
        panic!("text")
    };
    assert_eq!(text, "plain words");
}

#[test]
fn width_arguments_are_carried_into_the_reroute_steps_and_nothing_is_invented() {
    let mut result = json!({
        "completion": { "if_still_open": { "fallback": [
            { "manual": "close the board" },
            { "tool": "export_specctra_dsn", "arguments": {} },
            { "tool": "route_specctra_dsn",
              "arguments": { "dsn_path": "x.dsn", "engine": "freerouting" } },
        ] } }
    });
    carry_arguments(
        &mut result,
        &json!({ "width_policy": "hold", "width_fallback": "none" }),
        (Effort::H, EffortSource::Argument),
    );
    let steps = &result["completion"]["if_still_open"]["fallback"];
    assert_eq!(steps[2]["arguments"]["effort"], "h");
    // A fastroute result's step routes with the default engine.
    assert!(steps[2]["arguments"].get("engine").is_none(), "{steps}");
    assert_eq!(steps[2]["arguments"]["dsn_path"], "x.dsn");
    assert_eq!(steps[2]["arguments"]["width_policy"], "hold");
    assert_eq!(steps[2]["arguments"]["width_fallback"], "none");
    assert_eq!(steps[1]["arguments"], json!({}));
    // Arguments the caller did not give are not added.
    carry_arguments(&mut result, &json!({}), (Effort::M, EffortSource::Argument));
    assert_eq!(
        result["completion"]["if_still_open"]["fallback"][2]["arguments"]["width_policy"], "hold",
        "an earlier value is not erased"
    );
    // A result without the fallback is not changed.
    let mut plain = json!({ "completion": { "next_steps": [] } });
    carry_arguments(
        &mut plain,
        &json!({ "width_policy": "hold" }),
        (Effort::M, EffortSource::Argument),
    );
    assert_eq!(plain, json!({ "completion": { "next_steps": [] } }));
}

#[test]
fn an_effort_the_caller_did_not_give_is_not_carried_into_the_reroute_step() {
    for source in [EffortSource::UserConfig, EffortSource::Default] {
        let mut result = json!({
            "completion": { "if_still_open": { "fallback": [
                { "tool": "route_specctra_dsn", "arguments": { "dsn_path": "x.dsn" } },
            ] } }
        });
        carry_arguments(&mut result, &json!({}), (Effort::Xh, source));
        let arguments = &result["completion"]["if_still_open"]["fallback"][0]["arguments"];
        assert!(arguments.get("effort").is_none(), "{source:?}: {arguments}");
    }
}

// ─── A finished search ───────────────────────────────────────────────────────

fn measures() -> Measures {
    Measures {
        unconnected: 0,
        violations: BTreeMap::new(),
        routed_share: Some(1.0),
        audit_complete: Some(true),
        acute: Some(0.0),
        right_angles: Some(0.0),
        copper_quality: Some(90.0),
        nets_undersized: Some(0.0),
        narrow_paths: Some(0.0),
        signal_mm: 100.0,
        power_mm: 20.0,
        pour_track_mm: 0.0,
        vias_off_pour: 3,
        below_floor: 0,
        width_rows: vec![],
        longest_below_mm: BTreeMap::new(),
        stitching_vias: 0,
        route_s: 12.34,
    }
}

fn judged(board: PathBuf) -> Judged {
    Judged {
        measures: measures(),
        gates: Gates {
            g1: vec![],
            g2: vec![],
            g3: vec![],
        },
        board,
        rounds: vec![json!({ "round": 1, "open_before": 2, "open_after": 0 })],
    }
}

fn routed(root: &Path, index: usize, shape: Shape, seed: u64, planes: &[&str]) -> Routed {
    let folder = root.join(run_folder_name(index, shape, seed));
    Routed {
        index,
        shape,
        seed,
        dsn: folder.join("variant.dsn"),
        held: Held {
            ses: folder.join("full/full.ses"),
            unrouted: 0,
            kept: "full",
            passes: vec![
                json!({ "name": "full", "outcome": "kept", "unrouted": 0, "seconds": 12.3 }),
            ],
            seconds: 12.3,
        },
        planes: planes.iter().map(|net| net.to_string()).collect(),
        folder,
    }
}

fn record(
    index: usize,
    shape: Shape,
    seed: u64,
    outcome: &'static str,
    routed: Option<Routed>,
    judged: Option<Judged>,
) -> RunRecord {
    RunRecord {
        index,
        shape,
        seed,
        launched_s: 0.04,
        routed_s: routed.as_ref().map(|_| 12.34),
        judged_s: judged.as_ref().map(|_| 20.06),
        route_s: routed.as_ref().map(|_| 12.3),
        judge_s: judged.as_ref().map(|_| 7.71),
        outcome,
        reason: (outcome == "route_failed").then(|| "the router crashed".to_string()),
        routed,
        judged,
    }
}

fn plan() -> SearchPlan {
    SearchPlan {
        limits: Effort::M.limits(),
        lanes: Lanes {
            routers: 2,
            judges: 1,
        },
        order: route_search::Order::Fixed(vec![(Shape::AsExported, 1), (Shape::Omitted, 1)]),
        hang_cap: route_search::HANG_CAP,
    }
}

fn outcome(runs: Vec<RunRecord>, winner: Option<usize>) -> SearchOutcome {
    let keys = runs
        .iter()
        .map(|record| record.judged.as_ref().map(|_| [0; 10]))
        .collect();
    SearchOutcome {
        runs,
        winner,
        keys,
        elapsed: Duration::from_millis(61_234),
        budget_overrun: None,
        stopped_launching: Some("the launch rule".to_string()),
        baseline: Ok(Baseline {
            violations: BTreeMap::new(),
            unconnected: 41,
        }),
    }
}

fn candidate() -> Binary {
    Binary {
        path: PathBuf::from("/x/fastroute"),
        sha256: "ab".repeat(32),
        version: "fastroute 0.1.14".to_string(),
        source: "argument",
    }
}

/// A search folder as a finished search leaves it: run 0 the winner, run 1
/// judged, run 2 a failed route, each with the files `route_held` and the judge
/// make, a plane in run 0's variant.
struct Search {
    _dir: tempfile::TempDir,
    root: PathBuf,
    folder: PathBuf,
    outcome: SearchOutcome,
}

fn finished_search() -> Search {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let folder = root.join("out.search");
    let layout = [
        (0, Shape::AsExported, 1, "judged"),
        (1, Shape::Omitted, 1, "judged"),
        (2, Shape::AsExported, 2, "route_failed"),
    ];
    let mut runs = Vec::new();
    for (index, shape, seed, outcome) in layout {
        let run = folder.join(run_folder_name(index, shape, seed));
        std::fs::create_dir_all(run.join("full")).unwrap();
        std::fs::create_dir_all(run.join("polish")).unwrap();
        let variant = if index == 0 {
            "(pcb board (structure (plane GND (polygon B.Cu 0 0 0 1000 0 1000 1000))))"
        } else {
            "(pcb board)"
        };
        std::fs::write(run.join("variant.dsn"), variant).unwrap();
        std::fs::write(run.join("full/full.ses"), "(session board)").unwrap();
        std::fs::write(run.join("full/fastroute.log"), "log").unwrap();
        std::fs::write(run.join("polish/polish.ses"), "(session board)").unwrap();
        let board = folder.join(format!("judge/run-{index}/r0/board.kicad_pcb"));
        std::fs::create_dir_all(board.parent().unwrap()).unwrap();
        std::fs::write(&board, "(kicad_pcb)").unwrap();
        let planes: &[&str] = if index == 0 { &["GND"] } else { &[] };
        let routed =
            (outcome != "route_failed").then(|| routed(&folder, index, shape, seed, planes));
        let judged = (outcome == "judged").then(|| judged(board));
        runs.push(record(index, shape, seed, outcome, routed, judged));
    }
    std::fs::create_dir_all(folder.join("judge/baseline")).unwrap();
    std::fs::write(folder.join("judge/baseline/board.kicad_pcb"), "(kicad_pcb)").unwrap();
    std::fs::write(folder.join("amps.txt"), "1 VBUS\n").unwrap();
    Search {
        outcome: outcome(runs, Some(0)),
        _dir: dir,
        root,
        folder,
    }
}

#[test]
fn the_winner_is_kept_whole_and_the_others_are_reduced_to_their_record() {
    let search = finished_search();
    reduce_run_folders(&search.folder, &search.outcome).unwrap();
    let run = |index: usize, shape: Shape, seed: u64| {
        search.folder.join(run_folder_name(index, shape, seed))
    };
    // The winner: everything it had, and its judged board.
    let winner = run(0, Shape::AsExported, 1);
    for file in [
        "variant.dsn",
        "full/full.ses",
        "full/fastroute.log",
        "polish/polish.ses",
        "run.json",
    ] {
        assert!(winner.join(file).is_file(), "winner lost {file}");
    }
    assert!(search
        .folder
        .join("judge/run-0/r0/board.kicad_pcb")
        .is_file());
    // The others: run.json only.
    for (index, shape, seed) in [(1, Shape::Omitted, 1), (2, Shape::AsExported, 2)] {
        let folder = run(index, shape, seed);
        let mut left: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, vec!["run.json".to_string()], "run {index}");
        assert!(
            !search.folder.join(format!("judge/run-{index}")).exists(),
            "run {index}'s judge boards"
        );
    }
    // Nothing else of the search folder was touched.
    assert!(search
        .folder
        .join("judge/baseline/board.kicad_pcb")
        .is_file());
    assert!(search.folder.join("amps.txt").is_file());
}

#[test]
fn a_reduced_run_keeps_its_record_measures_and_gates() {
    let search = finished_search();
    reduce_run_folders(&search.folder, &search.outcome).unwrap();
    let record: Value = serde_json::from_str(
        &std::fs::read_to_string(
            search
                .folder
                .join(run_folder_name(1, Shape::Omitted, 1))
                .join("run.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record["record"]["outcome"], "judged");
    assert_eq!(record["record"]["shape"], "omitted");
    assert_eq!(record["record"]["seed"], 1);
    assert!(record["judged"]["measures"].is_object(), "{record}");
    assert!(record["judged"]["gates"]["g1"].is_array(), "{record}");
    assert!(record["judged"]["rounds"].is_array(), "{record}");
    assert_eq!(record["kept_pass"], "full");
    assert_eq!(record["board_kept"], false);
    let failed: Value = serde_json::from_str(
        &std::fs::read_to_string(
            search
                .folder
                .join(run_folder_name(2, Shape::AsExported, 2))
                .join("run.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(failed["record"]["outcome"], "route_failed");
    assert_eq!(failed["record"]["reason"], "the router crashed");
    assert!(failed.get("judged").is_none());
}

#[test]
fn only_what_the_search_made_is_removed() {
    let search = finished_search();
    let loser = search.folder.join(run_folder_name(1, Shape::Omitted, 1));
    std::fs::write(loser.join("notes.txt"), "someone else's file").unwrap();
    // A judge folder of a run this search never launched.
    std::fs::create_dir_all(search.folder.join("judge/run-9/r0")).unwrap();
    std::fs::write(search.folder.join("judge/run-9/r0/b"), "x").unwrap();
    // A file beside the run folders.
    std::fs::write(search.folder.join("run-001-readme.txt"), "x").unwrap();
    reduce_run_folders(&search.folder, &search.outcome).unwrap();
    assert!(loser.join("notes.txt").is_file());
    assert!(search.folder.join("judge/run-9/r0/b").is_file());
    assert!(search.folder.join("run-001-readme.txt").is_file());
    assert!(!loser.join("variant.dsn").exists());
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_is_never_followed_or_removed() {
    let search = finished_search();
    let outside = search.root.join("outside");
    std::fs::create_dir_all(outside.join("inner")).unwrap();
    std::fs::write(outside.join("precious.txt"), "keep").unwrap();
    std::fs::write(outside.join("inner/also.txt"), "keep").unwrap();
    let loser = search.folder.join(run_folder_name(1, Shape::Omitted, 1));
    // A pass folder that is a link, and a link inside another pass folder.
    std::fs::remove_dir_all(loser.join("polish")).unwrap();
    std::os::unix::fs::symlink(&outside, loser.join("polish")).unwrap();
    std::os::unix::fs::symlink(&outside, loser.join("full/link")).unwrap();
    // A whole run folder that is a link.
    let linked = search.folder.join(run_folder_name(2, Shape::AsExported, 2));
    std::fs::remove_dir_all(&linked).unwrap();
    std::os::unix::fs::symlink(&outside, &linked).unwrap();
    // A judge folder that is a link.
    std::fs::remove_dir_all(search.folder.join("judge/run-1")).unwrap();
    std::os::unix::fs::symlink(&outside, search.folder.join("judge/run-1")).unwrap();

    reduce_run_folders(&search.folder, &search.outcome).unwrap();

    assert_eq!(
        std::fs::read_to_string(outside.join("precious.txt")).unwrap(),
        "keep"
    );
    assert_eq!(
        std::fs::read_to_string(outside.join("inner/also.txt")).unwrap(),
        "keep"
    );
    assert!(
        !outside.join("run.json").exists(),
        "a record was written through a link"
    );
    assert!(loser
        .join("polish")
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(loser
        .join("full/link")
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(linked.symlink_metadata().unwrap().file_type().is_symlink());
    assert!(search
        .folder
        .join("judge/run-1")
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
    // What was a plain file or folder beside the links is gone.
    assert!(!loser.join("variant.dsn").exists());
    assert!(!loser.join("full/full.ses").exists());
}

#[test]
fn a_search_folder_that_is_not_a_folder_is_an_error_and_nothing_is_touched() {
    let search = finished_search();
    let file = search.root.join("not-a-folder");
    std::fs::write(&file, "x").unwrap();
    assert!(reduce_run_folders(&file, &search.outcome).is_err());
    assert!(reduce_run_folders(&search.root.join("absent"), &search.outcome).is_err());
    assert!(search
        .folder
        .join(run_folder_name(1, Shape::Omitted, 1))
        .join("variant.dsn")
        .is_file());
}

#[test]
fn run_folders_are_named_by_index_shape_and_seed() {
    assert_eq!(
        run_folder_name(7, Shape::Omitted, 12),
        "run-007-omitted-s12"
    );
    assert_eq!(
        run_folder_name(0, Shape::AsExported, 1),
        "run-000-as_exported-s1"
    );
    assert_eq!(
        run_folder_name(1234, Shape::AsExported, 1),
        "run-1234-as_exported-s1"
    );
}

#[test]
fn the_result_has_every_documented_key() {
    let search = finished_search();
    let ses = search.root.join("out.ses");
    std::fs::write(&ses, "(session board)").unwrap();
    let dsn = search.root.join("board.dsn");
    std::fs::write(&dsn, "(pcb board)").unwrap();
    let result = result_json(
        &dsn,
        &ses,
        &search.folder,
        &candidate(),
        (Effort::H, EffortSource::UserConfig),
        &plan(),
        &[Shape::AsExported, Shape::Omitted],
        &search.outcome,
    );
    assert_eq!(result["success"], true, "{result}");
    assert_eq!(result["method"], "local_fastroute_best_of_n");
    // Today's fields. The winner's variant held a plane, which alone would leave
    // the router's status `unverified`; every gate passed, so it is the judge's.
    assert_eq!(result["status"], "complete", "{result}");
    assert!(
        result["completion"]["next_steps_missing"].is_string()
            || result["completion"]["next_steps"].is_array()
    );
    assert_eq!(result["completion"]["planes"][0]["net"], "GND");
    assert_eq!(result["completion"]["router_unrouted"], 0);
    assert!(result["completion"]["router_unrouted_source"]
        .as_str()
        .unwrap()
        .contains("fastroute"));
    assert!(
        !result["completion"]["reason"]
            .as_str()
            .unwrap()
            .contains("Freerouting"),
        "{}",
        result["completion"]["reason"]
    );
    let judged = &result["completion"]["judged"];
    assert_eq!(judged["unconnected"], 0);
    assert_eq!(judged["all_gates_pass"], true);
    assert!(judged["gates"]["g2"].is_array());
    assert!(judged["judged_board"]
        .as_str()
        .unwrap()
        .ends_with("judge/run-0/r0/board.kicad_pcb"));
    for key in ["intent_widths", "width_exceptions"] {
        assert!(result[key].is_array(), "{key}");
    }
    assert_eq!(result["width_exceptions_total"], 0);
    assert!(result.get("width_unmeasured").is_none());
    // The passes of the winner, as the Freerouting hold path names them.
    assert_eq!(result["result_pass"], "full");
    assert_eq!(result["passes"][0]["name"], "full");
    // The engine.
    let engine = &result["engine"];
    assert_eq!(engine["name"], "fastroute");
    assert_eq!(engine["candidate"], "C** + BR1 + W8P");
    assert_eq!(engine["path"], "/x/fastroute");
    assert_eq!(engine["sha256"], "ab".repeat(32));
    assert_eq!(engine["version"], "fastroute 0.1.14");
    assert_eq!(engine["source"], "argument");
    assert_eq!(engine["threads"], 2);
    assert_eq!(
        engine["switches"].as_array().unwrap().len(),
        crate::fastroute::CANDIDATE_SWITCHES.len()
    );
    // The effort.
    assert_eq!(result["effort"]["level"], "h");
    assert_eq!(result["effort"]["source"], "user_config");
    assert_eq!(result["effort"]["budget_s"], 720.0);
    assert!(result["effort"]["runs"].is_null());
    // The search.
    let found = &result["search"];
    assert_eq!(found["lanes"], json!({ "routers": 2, "judges": 1 }));
    assert_eq!(found["shapes"], json!(["as_exported", "omitted"]));
    assert_eq!(found["launched"], 3);
    assert_eq!(found["judged"], 2);
    assert_eq!(found["elapsed_s"], 61.2);
    assert!(found["budget_overrun_s"].is_null());
    assert_eq!(found["stopped_launching"], "the launch rule");
    assert_eq!(found["baseline"]["ok"], true);
    assert_eq!(found["baseline"]["unconnected"], 41);
    let winner = &found["winner"];
    assert_eq!(winner["index"], 0);
    assert_eq!(winner["shape"], "as_exported");
    assert_eq!(winner["seed"], 1);
    assert_eq!(winner["kept"], "full");
    assert!(winner["gates"]["g1"].is_array());
    assert!(winner["measures"]["unconnected"].is_number());
    assert!(winner["passes"].is_array());
    let runs = found["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[0]["outcome"], "judged");
    assert_eq!(runs[2]["outcome"], "route_failed");
    // Seconds are numbers rounded to 0.1.
    assert_eq!(runs[0]["launched_s"], 0.0);
    assert_eq!(runs[0]["routed_s"], 12.3);
    assert_eq!(runs[0]["judged_s"], 20.1);
    assert_eq!(runs[0]["judge_s"], 7.7);
    // The artifacts.
    let artifacts = &result["artifacts"];
    assert_eq!(artifacts["dsn_path"], dsn.display().to_string());
    assert_eq!(artifacts["ses_output_path"], ses.display().to_string());
    assert_eq!(
        artifacts["search_folder"],
        search.folder.display().to_string()
    );
    assert!(artifacts["judged_board"].is_string());
    assert!(artifacts["passes_folder"]
        .as_str()
        .unwrap()
        .ends_with("run-000-as_exported-s1"));
}

#[test]
fn a_fast_result_has_runs_and_no_budget_and_an_overrun_is_reported() {
    let search = finished_search();
    let ses = search.root.join("out.ses");
    std::fs::write(&ses, "(session board)").unwrap();
    let dsn = search.root.join("board.dsn");
    std::fs::write(&dsn, "(pcb board)").unwrap();
    let mut finished = outcome(Vec::new(), None);
    finished.runs = search.outcome.runs;
    finished.winner = Some(1);
    finished.keys = vec![Some([0; 10]), Some([0; 10]), None];
    finished.budget_overrun = Some(Duration::from_millis(33_333));
    let result = result_json(
        &dsn,
        &ses,
        &search.folder,
        &candidate(),
        (Effort::Fast, EffortSource::Default),
        &plan(),
        &[Shape::AsExported],
        &finished,
    );
    assert_eq!(result["effort"]["level"], "fast");
    assert_eq!(result["effort"]["source"], "default");
    assert!(result["effort"]["budget_s"].is_null());
    assert_eq!(result["effort"]["runs"], 2);
    assert_eq!(result["search"]["budget_overrun_s"], 33.3);
    assert_eq!(result["search"]["winner"]["index"], 1);
    // Run 1 gave the router no plane: the router's own count is `complete`.
    assert_eq!(result["status"], "complete", "{result}");
}

#[test]
fn a_baseline_that_failed_is_reported_beside_the_runs() {
    let search = finished_search();
    let mut finished = outcome(search.outcome.runs, Some(0));
    finished.baseline = Err("kicad-cli exited 3".to_string());
    let json = search_json(&plan(), &[Shape::AsExported], &finished, Listing::Best);
    assert_eq!(
        json["baseline"],
        json!({ "ok": false, "error": "kicad-cli exited 3" })
    );
}

#[test]
fn a_search_with_no_judged_run_has_no_winner_and_says_why_in_its_runs() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("out.search");
    std::fs::create_dir(&folder).unwrap();
    let runs = vec![
        record(0, Shape::AsExported, 1, "route_failed", None, None),
        record(1, Shape::Omitted, 1, "hung", None, None),
    ];
    let mut finished = outcome(runs, None);
    finished.baseline = Err("no baseline".into());
    let call = Call {
        dsn: dir.path().join("board.dsn"),
        ses_output: dir.path().join("out.ses"),
        folder,
        effort: (Effort::M, EffortSource::Default),
        fallback: WidthFallback::Floor,
    };
    let search = search_json(
        &plan(),
        &[Shape::AsExported, Shape::Omitted],
        &finished,
        Listing::Best,
    );
    assert!(search["winner"].is_null());
    let result = no_winner_result(&call, &finished, search);
    assert!(result.is_error);
    assert_eq!(kind(&result), "handler_error");
    let text = message(&result);
    assert!(
        text.contains("1 hung") && text.contains("1 route_failed"),
        "{text}"
    );
    assert!(text.contains("baseline"), "{text}");
    assert!(text.contains("engine: \"freerouting\""), "{text}");
    assert!(!call.ses_output.exists());
    let runs = body(&result)["search"]["runs"].clone();
    assert_eq!(runs[0]["reason"], "the router crashed");
    // `result_json` of an outcome with no winner says so and writes no success.
    let none = result_json(
        &call.dsn,
        &call.ses_output,
        &call.folder,
        &candidate(),
        (Effort::M, EffortSource::Default),
        &plan(),
        &[Shape::AsExported],
        &finished,
    );
    assert_eq!(none["success"], false);
}

#[test]
fn a_failed_search_keeps_its_records_and_not_its_boards() {
    let search = finished_search();
    // No winner: every run, the judged ones too, is reduced.
    let mut failed = outcome(Vec::new(), None);
    failed.runs = search.outcome.runs;
    failed.keys = vec![None, None, None];
    failed.baseline = Err("no baseline".into());
    let call = Call {
        dsn: search.root.join("board.dsn"),
        ses_output: search.root.join("out.ses"),
        folder: search.folder.clone(),
        effort: (Effort::M, EffortSource::Default),
        fallback: WidthFallback::Floor,
    };
    let summary = search_json(&plan(), &[Shape::AsExported], &failed, Listing::Best);
    let result = no_winner_result(&call, &failed, summary);
    assert!(result.is_error);
    assert!(body(&result)["search"].get("reduce_error").is_none());
    for (index, shape, seed) in [
        (0, Shape::AsExported, 1),
        (1, Shape::Omitted, 1),
        (2, Shape::AsExported, 2),
    ] {
        let folder = search.folder.join(run_folder_name(index, shape, seed));
        let mut left: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, vec!["run.json".to_string()], "run {index}");
        assert!(
            !search.folder.join(format!("judge/run-{index}")).exists(),
            "run {index}'s judge boards"
        );
    }
    assert!(!call.ses_output.exists());
}

#[test]
fn a_reduce_error_on_a_failed_search_is_reported_and_not_fatal() {
    let search = finished_search();
    let mut failed = outcome(Vec::new(), None);
    failed.runs = search.outcome.runs;
    failed.keys = vec![None, None, None];
    // A search folder that is not there cannot be reduced.
    let call = Call {
        dsn: search.root.join("board.dsn"),
        ses_output: search.root.join("out.ses"),
        folder: search.root.join("absent.search"),
        effort: (Effort::M, EffortSource::Default),
        fallback: WidthFallback::Floor,
    };
    let summary = search_json(&plan(), &[Shape::AsExported], &failed, Listing::Best);
    let result = no_winner_result(&call, &failed, summary);
    assert!(result.is_error);
    assert!(
        body(&result)["search"]["reduce_error"].is_string(),
        "{}",
        body(&result)
    );
}

// ─── The whole thing, with real fastroute ────────────────────────────────────

// LEAD: needs the real tools; run it with `cargo test -p konnect-core
// best_of_n_routes_a_public_fixture -- --ignored` and these variables:
//   ROUTE_E2E_DSN  an exported DSN of a public fixture board (`pic_programmer` or
//                  `specctra_pours_routing`), with its manifest beside it as
//                  `<dsn>.konnect.json` and the saved board that manifest names,
//                  unchanged since the export (the judge checks its SHA-256);
//   FASTROUTE      the candidate build's path (C** + BR1 + W8P, build x18);
//   KICAD_CLI      the path of kicad-cli;
//   KICAD_CONFIG_HOME  a scratch folder (never the real KiCad settings).
// It routes at effort `fast` (2 runs at once) into a temp folder and checks that a
// session was written, that every run has a record and that a winner was judged.
#[tokio::test]
#[ignore = "needs fastroute, kicad-cli and an exported public fixture (see LEAD)"]
async fn best_of_n_routes_a_public_fixture_end_to_end() {
    let dsn = PathBuf::from(std::env::var("ROUTE_E2E_DSN").expect("set ROUTE_E2E_DSN"));
    let fastroute = std::env::var("FASTROUTE").expect("set FASTROUTE");
    let kicad_cli = std::env::var("KICAD_CLI").expect("set KICAD_CLI");
    std::env::var("KICAD_CONFIG_HOME").expect("set KICAD_CONFIG_HOME to a scratch folder");
    let dir = tempfile::tempdir().unwrap();
    let ses = dir.path().join("routed.ses");
    let ctx = Arc::new(ToolContext::new(
        ServerConfig {
            kicad_cli,
            ..ServerConfig::default()
        },
        Arc::new(ToolRouter::new()),
    ));
    let args = json!({
        "dsn_path": dsn,
        "ses_output_path": ses,
        "effort": "fast",
        "fastroute_path": fastroute,
    });
    let result = handle_route_specctra_dsn(&args, ctx).await.unwrap();
    let parsed = body(&result);
    assert!(!result.is_error, "{parsed}");
    assert_eq!(parsed["success"], true);
    assert!(ses.is_file(), "the winner's session was written");
    assert!(std::fs::metadata(&ses).unwrap().len() > 0);
    let runs = parsed["search"]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2, "Fast launches exactly 2 runs");
    for run in runs {
        assert!(run["outcome"].is_string(), "{run}");
    }
    assert!(parsed["search"]["winner"].is_object(), "{parsed}");
    assert!(dir.path().join("routed.search/search.json").is_file());
    assert!(parsed["completion"]["judged"]["gates"].is_object());
}

/// A winner's status follows the judge (D21). A failed gate decides it; with every
/// gate passed it is the judge's `complete` unless the width report or a stopped run
/// says otherwise, whatever the router counted. Each branch has a case of its own.
#[test]
fn the_status_is_the_judges_when_every_gate_passes() {
    use crate::route_judge::Gates;
    let gates = |g1: &[&str], g2: &[&str], g3: &[&str]| Gates {
        g1: g1.iter().map(|s| s.to_string()).collect(),
        g2: g2.iter().map(|s| s.to_string()).collect(),
        g3: g3.iter().map(|s| s.to_string()).collect(),
    };
    let pass = gates(&[], &[], &[]);
    let full = WidthOutcome::default();
    // Every gate passed: the router's count (`incomplete`, when a pour makes a
    // connection it left) and its plane rule (`unverified`) give way to the judge.
    for router in [
        "complete",
        "complete_with_width_exceptions",
        "unverified",
        "incomplete",
    ] {
        assert_eq!(
            judged_status(router, &pass, &full, 0),
            "complete",
            "{router}"
        );
    }
    // What the judge does not settle stays: the width report, and a stopped run.
    let narrow = WidthOutcome {
        exceptions: 2,
        ..WidthOutcome::default()
    };
    assert_eq!(
        judged_status("incomplete", &pass, &narrow, 0),
        "complete_with_width_exceptions"
    );
    let unread = WidthOutcome {
        unread: true,
        ..WidthOutcome::default()
    };
    assert_eq!(judged_status("incomplete", &pass, &unread, 0), "unverified");
    let unmeasured = WidthOutcome {
        unmeasured: 1,
        ..WidthOutcome::default()
    };
    assert_eq!(
        judged_status("complete", &pass, &unmeasured, 0),
        "unverified"
    );
    assert_eq!(judged_status("stopped", &pass, &full, 0), "stopped");
    let stopped = WidthOutcome {
        stopped_in: Some("full"),
        ..WidthOutcome::default()
    };
    assert_eq!(judged_status("complete", &pass, &stopped, 0), "stopped");
    // Every gate passed, but only with stitching vias the judge added: they are
    // not in the session, so the router's status stands.
    assert_eq!(judged_status("incomplete", &pass, &full, 2), "incomplete");
    assert_eq!(judged_status("unverified", &pass, &full, 1), "unverified");
    assert_eq!(judged_status("stopped", &pass, &full, 1), "stopped");
    assert_eq!(
        judged_status("complete", &gates(&["1 unconnected"], &[], &[]), &full, 3),
        "incomplete",
        "a failed gate outranks the stitching rule"
    );
    // A failed gate outranks everything.
    assert_eq!(
        judged_status("complete", &gates(&["2 unconnected"], &[], &[]), &full, 0),
        "incomplete"
    );
    assert_eq!(
        judged_status(
            "complete",
            &gates(&[], &["clearance: 3, unrouted board 0"], &[]),
            &full,
            0
        ),
        "complete_with_gate_failures"
    );
    assert_eq!(
        judged_status(
            "complete",
            &gates(&[], &[], &["1 track below its floor"]),
            &full,
            0
        ),
        "complete_with_gate_failures"
    );
    assert_eq!(
        judged_status(
            "unverified",
            &gates(&["1 unconnected"], &["clearance"], &[]),
            &full,
            0
        ),
        "incomplete",
        "G1 outranks the other gates"
    );
}

/// The 1 Kopper smoke's case: the winner's variant held the GND pour as a plane,
/// fastroute counted one connection it left (the one the pour makes), and the judge
/// found nothing open with every gate passing. `status` is the judge's `complete`;
/// the router's count stays in `completion`, and the reason says why they differ.
/// With a stitching via the judge added, the router's status stands (the via is not in
/// the session). The same winner with G1 failing is `incomplete`, and with only G2
/// failing `complete_with_gate_failures`; neither has a note (the controls).
#[test]
fn a_connection_the_router_left_to_a_pour_gives_way_to_the_judge() {
    let mut search = finished_search();
    search.outcome.runs[0]
        .routed
        .as_mut()
        .unwrap()
        .held
        .unrouted = 1;
    let ses = search.root.join("out.ses");
    std::fs::write(&ses, "(session board)").unwrap();
    let dsn = search.root.join("board.dsn");
    std::fs::write(&dsn, "(pcb board)").unwrap();
    let result_for = |search: &Search| {
        result_json(
            &dsn,
            &ses,
            &search.folder,
            &candidate(),
            (Effort::Fast, EffortSource::Default),
            &plan(),
            &[Shape::AsExported, Shape::Omitted],
            &search.outcome,
        )
    };

    let judged = result_for(&search);
    assert_eq!(judged["status"], "complete", "{judged}");
    assert_eq!(judged["completion"]["router_unrouted"], 1);
    assert_eq!(judged["completion"]["judged"]["unconnected"], 0);
    assert_eq!(judged["completion"]["judged"]["all_gates_pass"], true);
    assert_eq!(judged["completion"]["judged"]["stitching_vias"], 0);
    let reason = judged["completion"]["reason"].as_str().unwrap();
    assert!(reason.contains("1 connection(s)"), "{reason}");
    assert!(reason.ends_with(JUDGED_STATUS_NOTE), "{reason}");
    let complete_when = judged["completion"]["complete_when"].as_str().unwrap();
    assert!(
        complete_when.ends_with(JUDGED_COMPLETE_WHEN_END),
        "{complete_when}"
    );

    // Every gate passed only with a stitching via the judge added: the via is not in
    // the session, so the router's `incomplete` stands, and the reason says why.
    search.outcome.runs[0]
        .judged
        .as_mut()
        .unwrap()
        .measures
        .stitching_vias = 1;
    let stitched = result_for(&search);
    assert_eq!(stitched["status"], "incomplete", "{stitched}");
    assert_eq!(stitched["completion"]["judged"]["stitching_vias"], 1);
    let reason = stitched["completion"]["reason"].as_str().unwrap();
    assert!(reason.ends_with(&judged_stitched_note(1)), "{reason}");
    assert!(!reason.contains(JUDGED_STATUS_NOTE), "{reason}");
    search.outcome.runs[0]
        .judged
        .as_mut()
        .unwrap()
        .measures
        .stitching_vias = 0;

    search.outcome.runs[0].judged.as_mut().unwrap().gates.g1 =
        vec!["1 unconnected item(s) after the refill and stitching".to_string()];
    let open = result_for(&search);
    assert_eq!(open["status"], "incomplete", "{open}");
    let reason = open["completion"]["reason"].as_str().unwrap();
    assert!(!reason.contains(JUDGED_STATUS_NOTE), "{reason}");

    // A failed DRC gate also changes the router's status (its plane `unverified`),
    // but not every gate passed: no note that says so.
    {
        let winner = &mut search.outcome.runs[0];
        winner.routed.as_mut().unwrap().held.unrouted = 0;
        let gates = &mut winner.judged.as_mut().unwrap().gates;
        gates.g1.clear();
        gates.g2 = vec!["clearance: 3, unrouted board 0".to_string()];
    }
    let drc = result_for(&search);
    assert_eq!(drc["status"], "complete_with_gate_failures", "{drc}");
    let reason = drc["completion"]["reason"].as_str().unwrap();
    assert!(!reason.contains(JUDGED_STATUS_NOTE), "{reason}");

    // A failed gate with stitching vias: the gate decides, and no stitching note.
    search.outcome.runs[0]
        .judged
        .as_mut()
        .unwrap()
        .measures
        .stitching_vias = 2;
    let stitched_drc = result_for(&search);
    assert_eq!(
        stitched_drc["status"], "complete_with_gate_failures",
        "{stitched_drc}"
    );
    let reason = stitched_drc["completion"]["reason"].as_str().unwrap();
    assert!(!reason.contains(&judged_stitched_note(2)), "{reason}");
}

/// The winner's width report sets `status` as it does for Freerouting: a stretch
/// below intent width gives `complete_with_width_exceptions`, a wire that cannot be
/// measured or a session that cannot be read for widths gives `unverified`, and
/// the same session at full width is plainly `complete` (the negative control).
#[test]
fn the_winners_width_report_sets_the_status_as_for_freerouting() {
    const NARROW: &str = "(session board (routes (resolution um 10) (network_out (net VBUS (wire (path F.Cu 10000 0 0 30000 0)) (wire (path F.Cu 2000 30000 0 30000 25000))))))";
    const ODD: &str = "(session board (routes (resolution um 10) (network_out (net VBUS (wire (path F.Cu 10000 0 0 30000 0)) (wire (polygon F.Cu 0 0 0 10 10 0 10))))))";
    let mut search = finished_search();
    // Run 1 wins: its variant gave the router no plane.
    search.outcome.winner = Some(1);
    std::fs::write(
        search
            .folder
            .join(run_folder_name(1, Shape::Omitted, 1))
            .join("variant.dsn"),
        "(pcb board (structure))",
    )
    .unwrap();
    let dsn = search.root.join("board.dsn");
    std::fs::write(&dsn, "(pcb board (structure))").unwrap();
    std::fs::write(
        search.root.join("board.dsn.konnect.json"),
        json!({
            "layers": [{ "dsn_name": "F.Cu", "kicad_name": "F.Cu" }],
            "intent_widths": [
                { "net": "VBUS", "current_a": 2.0, "from_mm": 0.2, "to_mm": 1.0, "status": "widened" }
            ]
        })
        .to_string(),
    )
    .unwrap();
    let result_for = |name: &str, session: &str| {
        let ses = search.root.join(format!("{name}.ses"));
        std::fs::write(&ses, session).unwrap();
        result_json(
            &dsn,
            &ses,
            &search.folder,
            &candidate(),
            (Effort::M, EffortSource::Default),
            &plan(),
            &[Shape::AsExported, Shape::Omitted],
            &search.outcome,
        )
    };

    let narrow = result_for("narrow", NARROW);
    assert_eq!(
        narrow["status"], "complete_with_width_exceptions",
        "{narrow}"
    );
    assert_eq!(narrow["width_exceptions_total"], 1);

    let odd = result_for("odd", ODD);
    assert_eq!(odd["status"], "unverified", "{odd}");
    assert_eq!(odd["width_unmeasured_total"], 1);

    let unread = result_for("unread", "(session board)");
    assert_eq!(unread["status"], "unverified", "{unread}");
    assert!(unread["width_report_error"].is_string(), "{unread}");

    let wide = result_for("wide", &NARROW.replace("F.Cu 2000", "F.Cu 10000"));
    assert_eq!(wide["status"], "complete", "{wide}");
    assert_eq!(wide["width_exceptions_total"], 0);
}

/// A release removes the run's judge boards, the router's log, the sandbox's home
/// and tmp and the profile, and keeps what a run that may still win needs: its
/// variant, its sessions and its reports. Nothing of another run is touched.
#[test]
fn a_release_removes_the_bulky_files_and_keeps_the_sessions() {
    let search = finished_search();
    let run = search.folder.join(run_folder_name(1, Shape::Omitted, 1));
    for name in ["home/x", "tmp/y"] {
        let path = run.join("full").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "z").unwrap();
    }
    std::fs::write(run.join("full/profile.sb"), "(version 1)").unwrap();
    std::fs::write(run.join("full/full.json"), "{}").unwrap();
    let judge = search.folder.join("judge");

    release_run(&search.folder, &judge, 1, Shape::Omitted, 1);

    assert!(!judge.join("run-1").exists(), "the judge's boards");
    for gone in [
        "full/home",
        "full/tmp",
        "full/profile.sb",
        "full/fastroute.log",
    ] {
        assert!(!run.join(gone).exists(), "{gone}");
    }
    for kept in [
        "variant.dsn",
        "full/full.ses",
        "full/full.json",
        "polish/polish.ses",
    ] {
        assert!(run.join(kept).is_file(), "{kept}");
    }
    // The other runs are as they were.
    assert!(judge.join("run-0/r0/board.kicad_pcb").is_file());
    let other = search.folder.join(run_folder_name(0, Shape::AsExported, 1));
    assert!(other.join("full/fastroute.log").is_file());
    // A second release of the same run is harmless.
    release_run(&search.folder, &judge, 1, Shape::Omitted, 1);
}

#[cfg(unix)]
#[test]
fn a_release_follows_no_link() {
    let search = finished_search();
    let outside = search.root.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), "mine").unwrap();
    let judge = search.folder.join("judge");
    std::fs::remove_dir_all(judge.join("run-1")).unwrap();
    std::os::unix::fs::symlink(&outside, judge.join("run-1")).unwrap();
    release_run(&search.folder, &judge, 1, Shape::Omitted, 1);
    assert!(outside.join("keep.txt").is_file());
}

/// The production runner's release is `release_run` on its own search folder and
/// judge root: the run's scratch boards and logs go, its session stays.
#[test]
fn the_runner_releases_a_run_in_its_own_search_folder() {
    let search = finished_search();
    let judge_root = search.folder.join("judge");
    let judge = JudgeContext {
        board: search.root.join("board.kicad_pcb"),
        board_text: String::new(),
        manifest: json!({}),
        manifest_text: "{}".to_string(),
        kicad_cli: String::new(),
        pour_nets: Default::default(),
        power_nets: Default::default(),
        floors_mm: Default::default(),
        root: judge_root.clone(),
        kicad_config_home: None,
    };
    let runner = FastrouteRunner {
        binary: fake_binary(&search.root),
        dsn_text: Arc::new(String::new()),
        hints: RoutingHints::default(),
        amps: None,
        fallback: WidthFallback::Floor,
        folder: search.folder.clone(),
        judge: Arc::new(judge),
        tools: ctx(),
    };
    let run = search.folder.join(run_folder_name(1, Shape::Omitted, 1));
    runner.release(1, Shape::Omitted, 1);
    assert!(!judge_root.join("run-1").exists());
    assert!(!run.join("full/fastroute.log").exists());
    assert!(run.join("full/full.ses").is_file());
    // Control: run 0 was not released.
    assert!(judge_root.join("run-0").is_dir());
}

/// `remove_tree` raises what it cannot read or remove: only a missing path, and a
/// folder kept for a link inside it, are not errors.
#[test]
fn remove_tree_raises_a_path_it_cannot_read_or_remove() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let set = |path: &Path, mode: u32| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap()
    };
    // A folder whose parent cannot be searched: its metadata cannot be read.
    let closed = dir.path().join("closed");
    std::fs::create_dir_all(closed.join("inside")).unwrap();
    set(&closed, 0o000);
    let unreadable = remove_tree(&closed.join("inside"));
    set(&closed, 0o755);
    assert!(unreadable.is_err());
    assert!(closed.join("inside").is_dir());
    // An empty folder in a parent that cannot be written: it cannot be removed.
    let fixed = dir.path().join("fixed");
    std::fs::create_dir_all(fixed.join("empty")).unwrap();
    set(&fixed, 0o555);
    let unremovable = remove_tree(&fixed.join("empty"));
    set(&fixed, 0o755);
    assert!(unremovable.is_err());
    // Controls: a missing path is no error, and a removable folder goes.
    assert!(remove_tree(&dir.path().join("missing")).is_ok());
    assert!(remove_tree(&fixed.join("empty")).is_ok());
    assert!(!fixed.join("empty").exists());
}

/// A pass folder or a run folder that is a link is not followed: what the link
/// points at keeps its logs and sandbox folders.
#[test]
fn a_release_follows_no_linked_run_or_pass_folder() {
    let search = finished_search();
    let judge = search.folder.join("judge");
    let outside = search.root.join("outside");
    for name in ["pass", "run"] {
        std::fs::create_dir_all(outside.join(name).join("home")).unwrap();
        std::fs::write(outside.join(name).join("fastroute.log"), "mine").unwrap();
    }
    // Run 1's `full` is a link to a folder outside the search.
    let run1 = search.folder.join(run_folder_name(1, Shape::Omitted, 1));
    std::fs::remove_dir_all(run1.join("full")).unwrap();
    std::os::unix::fs::symlink(outside.join("pass"), run1.join("full")).unwrap();
    release_run(&search.folder, &judge, 1, Shape::Omitted, 1);
    assert!(outside.join("pass/home").is_dir());
    assert!(outside.join("pass/fastroute.log").is_file());
    // Run 2's folder itself is a link: inside its target `full` is a real folder.
    let run2 = search.folder.join(run_folder_name(2, Shape::AsExported, 2));
    std::fs::create_dir_all(outside.join("run/full")).unwrap();
    std::fs::rename(outside.join("run/home"), outside.join("run/full/home")).unwrap();
    std::fs::rename(
        outside.join("run/fastroute.log"),
        outside.join("run/full/fastroute.log"),
    )
    .unwrap();
    std::fs::remove_dir_all(&run2).unwrap();
    std::os::unix::fs::symlink(outside.join("run"), &run2).unwrap();
    release_run(&search.folder, &judge, 2, Shape::AsExported, 2);
    assert!(outside.join("run/full/home").is_dir());
    assert!(outside.join("run/full/fastroute.log").is_file());
    // Control: run 0's real folders are released.
    let run0 = search.folder.join(run_folder_name(0, Shape::AsExported, 1));
    release_run(&search.folder, &judge, 0, Shape::AsExported, 1);
    assert!(!run0.join("full/fastroute.log").exists());
    assert!(run0.join("full/full.ses").is_file());
}

/// The result lists the 20 best judged runs by the rank key and the first 20 runs
/// that were not judged, with every outcome counted; `search.json` lists all.
#[test]
fn the_result_lists_the_best_runs_and_counts_every_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let mut runs = Vec::new();
    for index in 0..30 {
        runs.push(record(
            index,
            Shape::AsExported,
            index as u64 + 1,
            "judged",
            Some(routed(
                dir.path(),
                index,
                Shape::AsExported,
                index as u64 + 1,
                &[],
            )),
            Some(judged(dir.path().join("board.kicad_pcb"))),
        ));
    }
    for index in 30..55 {
        runs.push(record(
            index,
            Shape::Omitted,
            index as u64,
            "route_failed",
            None,
            None,
        ));
    }
    let mut many = outcome(runs, Some(29));
    // The later the run, the better its key.
    for (index, key) in many.keys.iter_mut().enumerate().take(30) {
        *key = Some([30 - index as i64, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }
    let best = search_json(&plan(), &[Shape::AsExported], &many, Listing::Best);
    let indexes: Vec<u64> = best["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["index"].as_u64().unwrap())
        .collect();
    let mut expected: Vec<u64> = (10..30).rev().collect();
    expected.extend(30..50);
    assert_eq!(indexes, expected);
    assert_eq!(
        best["outcomes"],
        json!({ "judged": 30, "route_failed": 25 })
    );
    assert_eq!(best["launched"], 55);
    assert!(best["runs_listed"].as_str().unwrap().contains("40 of 55"));

    let every = search_json(&plan(), &[Shape::AsExported], &many, Listing::Every);
    assert_eq!(every["runs"].as_array().unwrap().len(), 55);
    assert!(every.get("runs_listed").is_none());
    // A search small enough is listed whole, with no note.
    let small = finished_search();
    let whole = search_json(&plan(), &[Shape::AsExported], &small.outcome, Listing::Best);
    assert_eq!(whole["runs"].as_array().unwrap().len(), 3);
    assert!(whole.get("runs_listed").is_none());
}

/// A winner that was released during the search (it rose to the top only later)
/// has no judged board left: the result says so instead of naming a missing file.
#[test]
fn a_winner_released_during_the_search_has_no_judged_board_and_says_why() {
    let search = finished_search();
    let ses = search.root.join("out.ses");
    std::fs::write(&ses, "(session board)").unwrap();
    let dsn = search.root.join("board.dsn");
    std::fs::write(&dsn, "(pcb board)").unwrap();
    std::fs::remove_dir_all(search.folder.join("judge/run-0")).unwrap();
    let result = result_json(
        &dsn,
        &ses,
        &search.folder,
        &candidate(),
        (Effort::M, EffortSource::Default),
        &plan(),
        &[Shape::AsExported, Shape::Omitted],
        &search.outcome,
    );
    assert_eq!(result["success"], true);
    assert!(result["completion"]["judged"]["judged_board"].is_null());
    assert!(result["completion"]["judged"]["judged_board_note"]
        .as_str()
        .unwrap()
        .contains("session"));
    assert!(result["artifacts"]["judged_board"].is_null());
    assert!(result["search"]["winner"]["judged_board"].is_null());
}
