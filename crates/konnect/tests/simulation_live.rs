//! Simulation worker tests against the real binary.
//!
//! The first group needs no simulator and runs everywhere. The ignored tests
//! need an installed KiCad 10 that ships libngspice:
//!
//! ```text
//! cargo test -p konnect --test simulation_live -- --ignored
//! ```

use konnect_core::kicad_install::{find_code_models_dir, find_ngspice_library};
use konnect_core::simulation::protocol::{ProbeReport, PROTOCOL_VERSION, WORKER_SUBCOMMAND};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn worker(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_konnect"))
        .arg(WORKER_SUBCOMMAND)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("the worker starts")
}

fn report(output: &Output) -> ProbeReport {
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().last().expect("a report line");
    serde_json::from_str(line).expect("a report")
}

fn protocol() -> String {
    PROTOCOL_VERSION.to_string()
}

#[test]
fn a_missing_library_is_reported_as_a_load_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("libngspice-missing");
    let output = worker(&[
        "--protocol",
        &protocol(),
        "--probe",
        "--library",
        missing.to_str().unwrap(),
    ]);
    let report = report(&output);
    assert!(report.load_error.is_some(), "{report:?}");
    assert_eq!(report.engine_version, None);
}

#[test]
fn a_file_that_is_not_a_library_is_reported_as_a_load_error() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("libngspice-fake");
    std::fs::write(&fake, "not a library").unwrap();
    let output = worker(&[
        "--protocol",
        &protocol(),
        "--probe",
        "--library",
        fake.to_str().unwrap(),
    ]);
    assert!(report(&output).load_error.is_some());
}

#[test]
fn a_caller_on_another_protocol_is_refused_before_anything_loads() {
    let output = worker(&[
        "--protocol",
        "999999",
        "--probe",
        "--library",
        "/nonexistent",
    ]);
    let report = report(&output);
    assert_eq!(report.protocol, PROTOCOL_VERSION);
    assert!(report.load_error.unwrap().contains("999999"));
}

#[test]
fn a_bad_request_exits_nonzero() {
    assert!(!worker(&["--bogus"]).status.success());
}

#[test]
fn the_worker_is_not_advertised_in_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_konnect"))
        .arg("--help")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&output.stdout).contains(WORKER_SUBCOMMAND));
}

fn real_library() -> PathBuf {
    find_ngspice_library("", "kicad-cli")
        .library
        .expect("an installed KiCad with libngspice")
}

fn probe_real(library: &Path, extra: &[&str]) -> ProbeReport {
    let models = find_code_models_dir("", library).expect("code models beside the library");
    let protocol = protocol();
    let mut args = vec![
        "--protocol",
        &protocol,
        "--probe",
        "--library",
        library.to_str().unwrap(),
        "--code-models",
        models.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    report(&worker(&args))
}

#[test]
#[ignore = "needs KiCad 10 with libngspice"]
fn the_installed_library_loads_with_a_version_and_its_code_models() {
    let report = probe_real(&real_library(), &[]);
    assert_eq!(report.load_error, None);
    assert!(!report.exit_callback);
    assert!(report.engine_version.is_some(), "{report:?}");
    assert_eq!(report.code_models_failed, Vec::<String>::new());
    for model in ["analog", "digital", "spice2poly", "xtradev"] {
        assert!(
            report.code_models_loaded.iter().any(|name| name == model),
            "{report:?}"
        );
    }
}

#[test]
#[ignore = "needs KiCad 10 with libngspice"]
fn digital_models_are_left_out_on_request() {
    let report = probe_real(&real_library(), &["--no-digital"]);
    assert!(!report
        .code_models_loaded
        .iter()
        .any(|name| name == "digital"));
    assert!(report
        .code_models_loaded
        .iter()
        .any(|name| name == "analog"));
}

/// One tool call through a real server over stdio, as an MCP client makes it.
fn call_over_stdio(tool: &str, arguments: Value) -> Value {
    let home = tempfile::tempdir().unwrap();
    let requests = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "live-test", "version": "0"}}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
            "name": "load_toolset", "arguments": {"name": "simulation"}}}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
            "name": tool, "arguments": arguments}}),
    ];
    let mut child = Command::new(env!("CARGO_BIN_EXE_konnect"))
        .env("HOME", home.path())
        .env("KONNECT_STATE_DIR", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        for request in &requests {
            writeln!(stdin, "{request}").unwrap();
        }
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let response = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|message| message["id"] == 3)
        .expect("a tool response");
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

fn demos() -> PathBuf {
    PathBuf::from(std::env::var_os("KICAD_DEMOS").expect("KICAD_DEMOS names KiCad's demos folder"))
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_real_project_is_read_from_a_snapshot_and_left_untouched() {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/rectifier"), project.path());
    let before = file_names(project.path());

    let report = call_over_stdio(
        "check_simulation_readiness",
        json!({ "schematic": project.path().join("rectifier.kicad_sch") }),
    );

    assert_eq!(report["checks"][0]["passed"], true, "{report}");
    assert_eq!(report["checks"][1]["passed"], true, "{report}");
    assert!(report["ready"].is_null(), "{report}");
    assert_eq!(file_names(project.path()), before);
    let nets = report["nets"]["items"].as_array().unwrap();
    assert!(nets.iter().any(|net| net["kicad_name"] == "/rect_out"
        && net["spice_name"] == "/rect_out"
        && net["in_deck"] == true));
    let elements = report["elements"]["items"].as_array().unwrap();
    assert!(
        elements.iter().all(|element| element["modelled"] == true),
        "{report}"
    );
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn every_sheet_of_a_real_hierarchy_is_exported() {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/subsheets"), project.path());
    let report = call_over_stdio(
        "check_simulation_readiness",
        json!({ "schematic": project.path().join("mainsheet.kicad_sch"), "limit": 200 }),
    );
    assert_eq!(report["checks"][1]["passed"], true, "{report}");
    let nets = report["nets"]["items"].as_array().unwrap();
    assert!(nets
        .iter()
        .any(|net| net["kicad_name"].as_str().unwrap().starts_with("/subsheet")));
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_missing_model_library_fails_the_export_check() {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/rectifier"), project.path());
    std::fs::remove_file(project.path().join("diode.mod")).unwrap();
    let report = call_over_stdio(
        "check_simulation_readiness",
        json!({ "schematic": project.path().join("rectifier.kicad_sch") }),
    );
    assert_eq!(report["ready"], false, "{report}");
    assert_eq!(report["checks"][1]["passed"], false, "{report}");
    assert!(report["checks"][1]["detail"]
        .as_str()
        .unwrap()
        .contains("diode.mod"));
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_lock_on_the_project_is_disclosed_and_does_not_stop_the_check() {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/rectifier"), project.path());
    let lock = project.path().join("~rectifier.kicad_pro.lck");
    std::fs::write(&lock, r#"{"hostname":"other","username":"someone"}"#).unwrap();
    let report = call_over_stdio(
        "check_simulation_readiness",
        json!({ "schematic": project.path().join("rectifier.kicad_sch") }),
    );
    assert_eq!(report["checks"][1]["passed"], true, "{report}");
    let locks = report["source_evidence"]["locks_present"]
        .as_array()
        .unwrap();
    assert_eq!(locks.len(), 1, "{report}");
    assert!(lock.exists());
}
