//! Simulation worker tests against the real binary.
//!
//! The first group needs no simulator and runs everywhere. The ignored tests
//! need an installed KiCad 10 that ships libngspice:
//!
//! ```text
//! cargo test -p konnect --test simulation_live -- --ignored
//! ```

use konnect_core::kicad_install::{find_code_models_dir, find_ngspice_library};
use konnect_core::simulation::protocol::{
    HaltReason, ProbeReport, RunRequest, RunStatus, DECK_FILE, LOG_FILE, PROTOCOL_VERSION,
    RAW_FILE, REQUEST_FILE, STATUS_FILE, WORKER_SUBCOMMAND,
};
use konnect_core::simulation::raw;
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
    call_with_config(tool, arguments, "")
}

/// The tests give the server its own `HOME`, which KiCad ignores: it still writes
/// IBIS models to the user's real cache. A test that needs them allows it here.
fn call_with_config(tool: &str, arguments: Value, config: &str) -> Value {
    let home = tempfile::tempdir().unwrap();
    for config_dir in [".config/konnect", "Library/Application Support/konnect"] {
        let config_dir = home.path().join(config_dir);
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("config.toml"), config).unwrap();
    }
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
    assert_eq!(report["ready"], true, "{report}");
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

fn project_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for path in std::fs::read_dir(folder)
            .unwrap()
            .flatten()
            .map(|e| e.path())
        {
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|ext| ext == "kicad_pro") {
                found.push(path);
            }
        }
    }
    found
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn every_simulation_demo_flattens_or_names_its_export_failure() {
    let kicad_cache = PathBuf::from(std::env::var_os("HOME").unwrap()).join("Library/Caches/KiCad");
    let model_dirs = format!("ngspice_model_dirs = [{:?}]\n", kicad_cache);
    let mut vetted = 0;
    let mut failures: Vec<String> = Vec::new();
    for project_file in project_files(&demos().join("simulation")) {
        let source = project_file.parent().unwrap();
        let name = project_file.file_name().unwrap().to_owned();
        let project = tempfile::tempdir().unwrap();
        copy_tree(source, project.path());
        let schematic = project.path().join(name).with_extension("kicad_sch");
        let report = call_with_config(
            "check_simulation_readiness",
            json!({ "schematic": schematic }),
            &model_dirs,
        );
        let demo = source.file_name().unwrap().to_string_lossy().into_owned();
        eprintln!("{demo}: ready={} {}", report["ready"], report["checks"]);
        if report["checks"][1]["passed"] == false {
            assert_eq!(report["checks"][2]["ran"], false, "{demo}: {report}");
            continue;
        }
        assert_eq!(report["checks"][2]["passed"], true, "{demo}: {report}");
        assert!(report["deck"]["line_count"].as_u64().unwrap() > 0, "{demo}");
        if report["ready"] != true {
            failures.push(demo.clone());
        }
        vetted += 1;
    }
    assert_eq!(vetted, 17, "Q17ng alone is expected to fail its export");
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_testbench_changes_what_readiness_sees_without_touching_the_project() {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/rectifier"), project.path());
    let schematic = project.path().join("rectifier.kicad_sch");
    let before = std::fs::read(&schematic).unwrap();

    let report = call_over_stdio(
        "check_simulation_readiness",
        json!({
            "schematic": schematic,
            "testbench": { "exclude_references": ["V1"], "spice_lines": ["V9 /signal_inn 0 1"] }
        }),
    );

    assert_eq!(
        report["excluded_references"]["items"],
        json!(["V1"]),
        "{report}"
    );
    let checks = report["checks"].as_array().unwrap();
    let testbench = checks
        .iter()
        .find(|c| c["name"] == "testbench_nodes")
        .unwrap();
    assert_eq!(testbench["code"], "unknown_testbench_node", "{report}");
    assert_eq!(report["ready"], false);
    assert_eq!(std::fs::read(&schematic).unwrap(), before);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_control_block_in_a_real_schematic_is_reported_as_dropped() {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/sallen_key"), project.path());
    let report = call_over_stdio(
        "check_simulation_readiness",
        json!({ "schematic": project.path().join("sallen_key.kicad_sch") }),
    );
    assert_eq!(report["checks"][2]["passed"], true, "{report}");
    let dropped = report["deck"]["dropped_directives"].as_array().unwrap();
    assert!(
        dropped.iter().any(|item| item == ".control block"),
        "{report}"
    );
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_project_relative_include_in_schematic_text_is_read_from_the_project() {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/rectifier"), project.path());
    std::fs::write(project.path().join("bare.lib"), ".param from_bare=1\n").unwrap();
    std::fs::write(project.path().join("rooted.lib"), ".param from_rooted=1\n").unwrap();
    let path = project.path().join("rectifier.kicad_sch");
    let sheet = std::fs::read_to_string(&path).unwrap();
    let text = r#"(text ".include \"bare.lib\"\n.include \"${KIPRJMOD}/rooted.lib\""
        (exclude_from_sim no) (at 50 50 0) (effects (font (size 1.27 1.27)))
        (uuid "11111111-2222-4333-8444-555555555555"))
        (sheet_instances"#;
    assert!(sheet.contains("(sheet_instances"));
    std::fs::write(&path, sheet.replacen("(sheet_instances", text, 1)).unwrap();

    let report = call_over_stdio("check_simulation_readiness", json!({ "schematic": path }));
    assert_eq!(report["checks"][2]["passed"], true, "{report}");
    assert_eq!(report["deck"]["inlined_file_count"], 3, "{report}");
}

fn run_operating_point(project: &Path, sheet: &str) -> Value {
    call_over_stdio(
        "run_simulation",
        json!({
            "schematic": project.join(sheet),
            "analysis": { "kind": "operating_point" },
            "limit": 200,
        }),
    )
}

fn row<'a>(result: &'a Value, signal: &str) -> &'a Value {
    let rows = result["operating_point"]["rows"].as_array().unwrap();
    rows.iter()
        .find(|row| row["signal"] == signal)
        .unwrap_or_else(|| panic!("no {signal} in {result}"))
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_real_operating_point_runs_and_leaves_the_project_untouched() {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/amplifier-ac"), project.path());
    let before = file_names(project.path());

    let result = run_operating_point(project.path(), "amplifier-ac.kicad_sch");

    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    assert_eq!(result["simulated_circuit"], "as_drawn");
    assert_eq!(result["analysis"]["coverage_ratio"], 1.0);
    assert_eq!(result["engine"]["version"], "44.2");
    let supply = row(&result, "v(+12v)");
    assert_eq!(supply["net"], "+12V", "{result}");
    assert_eq!(supply["unit"], "V");
    assert!((supply["value"].as_f64().unwrap() - 12.0).abs() < 1e-9);
    assert_eq!(file_names(project.path()), before);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_dropped_control_block_is_reported_and_never_reaches_the_engine() {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/sallen_key"), project.path());

    let result = run_operating_point(project.path(), "sallen_key.kicad_sch");

    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    let dropped = result["circuit"]["dropped_directives"].to_string();
    assert!(dropped.contains(".control"), "{result}");
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_digital_circuit_runs_without_the_probe_that_would_stop_it() {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/up-down-counter"), project.path());
    let sheet = file_names(project.path())
        .into_iter()
        .find(|name| name.ends_with(".kicad_sch"))
        .unwrap();

    let result = run_operating_point(project.path(), &sheet);

    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    assert!(result["warnings"]
        .to_string()
        .contains("device_currents_not_saved"));
}

/// The engine as readiness found it, so these tests need no paths of their own.
fn engine_paths() -> (PathBuf, PathBuf) {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/rectifier"), project.path());
    let report = call_over_stdio(
        "check_simulation_readiness",
        json!({ "schematic": project.path().join("rectifier.kicad_sch") }),
    );
    let engine = &report["engine"];
    (
        PathBuf::from(engine["library_path"].as_str().unwrap()),
        PathBuf::from(engine["code_models_dir"].as_str().unwrap()),
    )
}

struct WorkerRun {
    dir: tempfile::TempDir,
}

impl WorkerRun {
    /// A run folder as the server prepares one, with `deck` taken as vetted. This
    /// is how a test reaches past deck vetting to the barriers behind it.
    fn prepare(deck: &str, input_path: Option<&Path>, budget_ms: u64) -> Self {
        Self::prepare_in(&std::env::temp_dir(), deck, input_path, budget_ms)
    }

    fn prepare_in(parent: &Path, deck: &str, input_path: Option<&Path>, budget_ms: u64) -> Self {
        let dir = tempfile::tempdir_in(parent).unwrap();
        let (library, code_models) = engine_paths();
        let request = RunRequest {
            protocol: PROTOCOL_VERSION,
            library,
            code_models: Some(code_models),
            load_digital: true,
            compatibility: Some("ltpsa".to_string()),
            input_path: input_path.map(Path::to_path_buf),
            budget_ms,
        };
        std::fs::write(dir.path().join(".spiceinit"), "").unwrap();
        std::fs::write(dir.path().join(DECK_FILE), deck).unwrap();
        std::fs::write(
            dir.path().join(REQUEST_FILE),
            serde_json::to_vec(&request).unwrap(),
        )
        .unwrap();
        Self { dir }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_konnect"));
        command
            .args(["sim-worker", "--protocol", &PROTOCOL_VERSION.to_string()])
            .arg("--run-dir")
            .arg(self.dir.path())
            .current_dir(self.dir.path())
            .env_clear()
            .env("SPICE_SCRIPTS", ".")
            .env("SPICE_USERINIT_DIR", ".")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn status(&self) -> Option<RunStatus> {
        let bytes = std::fs::read(self.dir.path().join(STATUS_FILE)).ok()?;
        Some(serde_json::from_slice(&bytes).unwrap())
    }

    fn finish(&self) -> RunStatus {
        assert!(self.command().status().unwrap().success());
        self.status().expect("a status file")
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join(LOG_FILE)).unwrap()
    }

    fn plot(&self) -> raw::Plot {
        let path = self.dir.path().join(RAW_FILE);
        raw::read(&path, u64::MAX).unwrap().remove(0)
    }
}

const SLOW_DECK: &str =
    ".title t\nV1 a 0 SIN(0 1 1k)\nR1 a b 1k\nC1 b 0 1u\n.tran 1n 1 0 1n\n.end\n";

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_run_past_its_budget_halts_softly_and_keeps_its_data() {
    let run = WorkerRun::prepare(SLOW_DECK, None, 1_500);

    let status = run.finish();

    assert_eq!(status.halted_reason, Some(HaltReason::Timeout));
    assert_eq!(status.sim_status, Some(0));
    assert!(status.results_written);
    let plot = run.plot();
    assert!(plot.points > 1, "{}", plot.points);
    let time = plot.variable("time").unwrap();
    let reached = plot.real(time).last().unwrap();
    assert!(reached > 0.0 && reached < 1.0, "{reached}");
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn the_engine_closes_a_subcircuit_the_way_preflight_does() {
    let deck = ".title t\n.SUBCKT tt a b\nR1 a b 1k\n.ENDSUBCKT\nV1 c 0 1\nX1 c 0 tt\n.op\n.end\n";
    let run = WorkerRun::prepare(deck, None, 5_000);

    assert_eq!(run.finish().sim_status, Some(0), "{}", run.log());
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_budget_that_ends_before_the_engine_starts_still_halts_softly() {
    let run = WorkerRun::prepare(SLOW_DECK, None, 1);

    let status = run.finish();

    assert_eq!(
        status.halted_reason,
        Some(HaltReason::Timeout),
        "{}",
        run.log()
    );
    assert_eq!(status.sim_status, Some(0));
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_run_folder_name_never_reaches_the_engine_as_text() {
    const DECK: &str = ".title t\nV1 a 0 1\nR1 a 0 1k\n.save all\n.op\n.end\n";
    let scratch = tempfile::tempdir().unwrap();
    let long = "n".repeat(200);
    for name in ["it's", "a$b", "c{d}", "e`echo hi`f", "with space", &long] {
        let mut parent = scratch.path().join(name);
        if name == long {
            parent = parent.join(&long).join(&long);
        }
        std::fs::create_dir_all(&parent).unwrap();
        let run = WorkerRun::prepare_in(&parent, DECK, None, 5_000);
        let status = run.finish();
        assert!(status.results_written, "{name}: {}", run.log());
        assert_eq!(run.plot().points, 1, "{name}");
    }
}

/// Abort, because the engine can call it. A `SIGSEGV` sent from outside proves
/// nothing: Rust's handler resets the signal and returns, and the run carries on.
#[cfg(unix)]
#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_worker_killed_mid_run_leaves_no_report_to_misread() {
    let run = WorkerRun::prepare(SLOW_DECK, None, 30_000);
    let mut child = run.command().spawn().unwrap();
    while !run.dir.path().join(LOG_FILE).is_file() {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::thread::sleep(std::time::Duration::from_millis(300));

    let pid = child.id().to_string();
    assert!(Command::new("kill")
        .args(["-ABRT", &pid])
        .status()
        .unwrap()
        .success());
    let exit = child.wait().unwrap();

    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&exit),
        Some(6)
    );
    assert!(run.status().is_none());
    assert!(!run.dir.path().join(RAW_FILE).exists());
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn the_engine_cannot_start_a_process_even_past_deck_vetting() {
    let scratch = tempfile::tempdir().unwrap();
    let marker = scratch.path().join("started");
    let deck = format!(
        ".title t\nV1 a 0 1\nR1 a 0 1k\n.control\necho KONNECT_BLOCK_RAN\nshell /usr/bin/touch {}\n\
         .endc\n.op\n.end\n",
        marker.display()
    );
    let run = WorkerRun::prepare(&deck, None, 5_000);

    run.finish();

    assert!(run.log().contains("KONNECT_BLOCK_RAN"), "{}", run.log());
    assert!(!marker.exists(), "{}", run.log());
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn the_engine_names_a_hebrew_node_with_underscores() {
    let deck = ".title t\nV1 /\u{5e9} 0 1\nR1 /\u{5e9} 0 1k\n.save all\n.op\n.end\n";
    let run = WorkerRun::prepare(deck, None, 5_000);

    let status = run.finish();

    assert_eq!(status.sim_status, Some(0));
    assert!(run.plot().variable("v(/__)").is_some());
}

const TABLE_DECK: &str = ".title t\nV1 a 0 1\nV2 b 0 1\nR1 c 0 1k\nA1 a b c tb\n\
    .model tb table2d(offset=0 gain=1 order=2 file=\"data.table\")\n.save all\n.op\n.end\n";
const TABLE: &str = "* table\n3\n3\n0 1 2\n0 1 2\n0 1 2\n0 1 2\n0 1 2\n";

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn code_models_read_relative_data_files_from_the_project() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("data.table"), TABLE).unwrap();

    let found = WorkerRun::prepare(TABLE_DECK, Some(project.path()), 5_000);
    assert_eq!(found.finish().sim_status, Some(0), "{}", found.log());

    let lost = WorkerRun::prepare(TABLE_DECK, None, 5_000);
    assert!(lost.finish().exit_status.is_some(), "{}", lost.log());
}

const BRIDGE_DECK: &str = ".title t\nV1 a 0 1\nR1 y 0 1k\nA1 a y inv\n\
    .model inv d_inverter(family=\"kq\")\n.save all\n.op\n.end\n";
const BRIDGE_FILES: [&str; 3] = [
    "bridge_kq_d_in.subcir",
    "bridge_kq_d_out.subcir",
    "bridge_kq_d_inout.subcir",
];

fn write_bridges(dir: &Path) {
    for name in BRIDGE_FILES {
        std::fs::write(dir.join(name), "* not a bridge\n").unwrap();
    }
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_bridge_file_in_the_project_never_reaches_the_engine() {
    let project = tempfile::tempdir().unwrap();
    write_bridges(project.path());
    let clean = WorkerRun::prepare(BRIDGE_DECK, Some(project.path()), 5_000);
    assert_eq!(clean.finish().sim_status, Some(0), "{}", clean.log());

    // The control: the same files in the run folder are read, and break the deck.
    let read = WorkerRun::prepare(BRIDGE_DECK, None, 5_000);
    write_bridges(read.dir.path());
    assert_eq!(read.finish().sim_status, None, "{}", read.log());
}
