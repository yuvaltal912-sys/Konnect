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
    call_in_home(home.path(), tool, arguments, config)
}

/// The same call with the server's `HOME` and state folder given, so a test can
/// read the run it left behind.
fn call_in_home(home: &Path, tool: &str, arguments: Value, config: &str) -> Value {
    for config_dir in [".config/konnect", "Library/Application Support/konnect"] {
        let config_dir = home.join(config_dir);
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
        .env("HOME", home)
        .env("KONNECT_STATE_DIR", home)
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
            max_raw_bytes: 1 << 40,
            complex_data: false,
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

    fn limit_size(&mut self, max_raw_bytes: u64) {
        let path = self.dir.path().join(REQUEST_FILE);
        let mut request: RunRequest =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        request.max_raw_bytes = max_raw_bytes;
        std::fs::write(path, serde_json::to_vec(&request).unwrap()).unwrap();
    }

    /// Tell the worker its data is complex, as the server does for an AC sweep.
    fn make_complex(&mut self) {
        let path = self.dir.path().join(REQUEST_FILE);
        let mut request: RunRequest =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        request.complex_data = true;
        std::fs::write(path, serde_json::to_vec(&request).unwrap()).unwrap();
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

/// The worked deck, as a testbench on a schematic emptied of its own parts.
fn soft_start_lines() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../konnect-core/tests/fixtures/simulation/soft_start.sp");
    let text = std::fs::read_to_string(path).unwrap();
    text.lines().map(str::to_owned).collect()
}

fn run_on_testbench(lines: &[String], arguments: Value) -> Value {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/rectifier"), project.path());
    let mut request = arguments;
    request["schematic"] = json!(project.path().join("rectifier.kicad_sch"));
    request["testbench"] = json!({ "exclude_references": ["*"], "spice_lines": lines });
    call_over_stdio("run_simulation", request)
}

fn measured(result: &Value, name: &str) -> f64 {
    let items = result["measurements"].as_array().unwrap();
    let item = items.iter().find(|item| item["name"] == name);
    let item = item.unwrap_or_else(|| panic!("no {name} in {result}"));
    item["value"].as_f64().unwrap_or_else(|| panic!("{item}"))
}

fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} is not {expected} within {tolerance}"
    );
}

const SOFT_START_MEASUREMENTS: fn() -> Value = || {
    json!([
        { "name": "inrush_peak", "kind": "peak_abs", "signal": "i(VBAT)" },
        { "name": "inrush_i2t", "kind": "i2t", "signal": "i(VBAT)", "to_seconds": 60e-3 },
        { "name": "rail_before", "kind": "value_at", "signal": "v(vout)", "at_seconds": 59.9e-3 },
        { "name": "rail_low", "kind": "min", "signal": "v(vout)", "from_seconds": 60e-3 },
        { "name": "device_peak", "kind": "max", "signal": "i(M1:d)" },
        { "name": "dissipation", "kind": "max", "signal": "p(M1)" },
    ])
};

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_soft_start_and_load_step_match_a_reference_worked_out_without_spice() {
    let result = run_on_testbench(
        &soft_start_lines(),
        json!({
            "analysis": {
                "kind": "transient",
                "stop_time_seconds": 0.14,
                "max_step_seconds": 5e-7,
            },
            "signals": ["v(vout)", "v(drain,vout)"],
            "envelope_points": 8,
            "measurements": SOFT_START_MEASUREMENTS(),
        }),
    );

    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    assert_eq!(result["outcome"]["requested"], 8);
    assert_eq!(result["outcome"]["completed"], 8);
    assert_eq!(result["simulated_circuit"], "modified");
    assert_eq!(result["analysis"]["coverage_ratio"], 1.0);
    // From `soft_start_reference.py`. A delivering source reads negative.
    near(measured(&result, "inrush_peak"), -16.9040, 2e-3);
    near(measured(&result, "device_peak"), 16.9040, 2e-3);
    near(measured(&result, "inrush_i2t"), 0.24465, 2e-4);
    let (before, low) = (
        measured(&result, "rail_before"),
        measured(&result, "rail_low"),
    );
    near(before, 11.98087, 1e-4);
    near(low, 11.88536, 1e-4);
    near((before - low) * 1e3, 95.507, 0.1);
    let peak = &result["measurements"][0];
    near(peak["at_seconds"].as_f64().unwrap(), 10.8566e-3, 2e-6);
    // Near 16 A across most of the 12 V, as the ramp starts.
    let dissipation = measured(&result, "dissipation");
    assert!((150.0..200.0).contains(&dissipation), "{dissipation}");

    let summary = &result["signal_summaries"][0];
    assert_eq!(summary["unit"], "V");
    near(summary["final"].as_f64().unwrap(), 11.98087, 1e-4);
    assert_eq!(summary["envelope"]["max"].as_array().unwrap().len(), 8);
    let headroom = &result["signal_summaries"][1];
    near(headroom["max"].as_f64().unwrap(), 12.0, 1e-3);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn the_default_step_lands_within_one_percent_of_the_reference() {
    let result = run_on_testbench(
        &soft_start_lines(),
        json!({
            "analysis": { "kind": "transient", "stop_time_seconds": 0.14 },
            "measurements": SOFT_START_MEASUREMENTS(),
        }),
    );

    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    near(measured(&result, "inrush_peak"), -16.9040, 0.17);
    near(measured(&result, "inrush_i2t"), 0.24465, 0.0025);
    let droop = measured(&result, "rail_before") - measured(&result, "rail_low");
    near(droop * 1e3, 95.507, 1.0);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn rise_overshoot_and_settling_match_a_simulated_circuit_of_known_shape() {
    // An RC at 1 ms, and a series RLC at 1 krad/s with damping 0.2.
    let lines: Vec<String> = [
        "V1 in 0 PULSE(0 1 0 1n 1n 1 2)",
        "R1 in rc 1k",
        "C1 rc 0 1u",
        "R2 in a 400",
        "L1 a b 1",
        "C2 b 0 1u",
    ]
    .iter()
    .map(|line| line.to_string())
    .collect();
    let result = run_on_testbench(
        &lines,
        json!({
            "analysis": {
                "kind": "transient",
                "stop_time_seconds": 0.05,
                "max_step_seconds": 1e-6,
            },
            "measurements": [
                { "name": "rise", "kind": "rise_time", "signal": "v(rc)", "final_value": 1 },
                { "name": "half", "kind": "crossing", "signal": "v(rc)", "level": 0.5 },
                { "name": "over", "kind": "overshoot_percent", "signal": "v(b)", "final_value": 1 },
                { "name": "settle", "kind": "settling_time", "signal": "v(b)", "final_value": 1 },
                { "name": "charge", "kind": "i2t", "signal": "i(R1)" },
                { "name": "rms", "kind": "rms", "signal": "i(R1)" },
            ],
        }),
    );

    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    let tau = 1e-3_f64;
    near(measured(&result, "rise"), tau * 9f64.ln(), 2e-6);
    near(measured(&result, "half"), tau * 2f64.ln(), 2e-6);
    // Peak at 100 e^(-pi 0.2 / sqrt(1 - 0.04)), and the closed form's last exit
    // from a 2 percent band, bisected apart from this code.
    let expected = 100.0 * (-std::f64::consts::PI * 0.2 / 0.96f64.sqrt()).exp();
    near(measured(&result, "over"), expected, 0.01);
    near(measured(&result, "settle"), 0.0196019, 5e-6);
    // i = 1 mA e^(-t/tau), so the integral of its square is 1e-6 tau / 2.
    near(measured(&result, "charge"), 5e-10, 2e-11);
    near(measured(&result, "rms"), (5e-10f64 / 0.05).sqrt(), 1e-6);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_schematic_as_drawn_answers_by_kicads_net_names_and_references() {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/rectifier"), project.path());
    let schematic = project.path().join("rectifier.kicad_sch");
    let before = file_names(project.path());

    let report = call_over_stdio(
        "check_simulation_readiness",
        json!({ "schematic": schematic }),
    );
    let names: Vec<String> = report["nets"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|net| net["kicad_name"].as_str().unwrap().to_string())
        .filter(|name| name != "GND")
        .collect();
    let signals: Vec<String> = names.iter().map(|name| format!("v({name})")).collect();
    let mut wanted = signals.clone();
    wanted.extend(["i(R1)", "p(R1)", "i(C1)"].map(str::to_owned));
    let result = call_over_stdio(
        "run_simulation",
        json!({
            "schematic": schematic,
            "analysis": { "kind": "transient", "stop_time_seconds": 0.05 },
            "signals": wanted,
        }),
    );

    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    assert_eq!(result["simulated_circuit"], "as_drawn");
    let summaries = result["signal_summaries"].as_array().unwrap();
    assert_eq!(summaries.len(), wanted.len());
    for summary in summaries {
        assert!(summary.get("finding").is_none(), "{summary}");
        assert!(summary["max"].as_f64().unwrap().abs() > 0.0, "{summary}");
    }
    assert_eq!(summaries[names.len()]["unit"], "A");
    assert_eq!(summaries[names.len() + 1]["unit"], "W");
    assert_eq!(file_names(project.path()), before);
}

/// Far more points than a budget of seconds or a small size limit allows.
const RUNAWAY_STEP: f64 = 2e-9;

fn runaway(extra: Value) -> Value {
    let mut arguments = json!({
        "analysis": {
            "kind": "transient",
            "stop_time_seconds": 0.14,
            "max_step_seconds": RUNAWAY_STEP,
        },
        "signals": ["v(gate)"],
        "measurements": [
            { "name": "top", "kind": "max", "signal": "v(gate)" },
            { "name": "gate", "kind": "value_at", "signal": "v(gate)", "at_seconds": 5e-6 },
        ],
    });
    for (key, value) in extra.as_object().unwrap() {
        arguments[key] = value.clone();
    }
    run_on_testbench(&soft_start_lines(), arguments)
}

fn assert_stopped_with_data(result: &Value, reason: &str) {
    assert_eq!(result["outcome"]["status"], "partial", "{result}");
    assert_eq!(result["stopped"]["reason"], reason, "{result}");
    assert_eq!(result["outcome"]["requested"], 4);
    assert_eq!(result["outcome"]["completed"], 1);
    assert_eq!(result["outcome"]["failed"], 3);
    let ratio = result["analysis"]["coverage_ratio"].as_f64().unwrap();
    assert!(ratio > 0.0 && ratio < 1.0, "{ratio}");
    assert_eq!(result["stopped"]["at_point"], result["analysis"]["points"]);
    assert!(result["analysis"]["points"].as_u64().unwrap() > 1000);
    assert!(result["measurements"][0]["value"].is_number(), "{result}");
    assert_eq!(result["measurements"][0]["window_truncated"], true);
    assert_eq!(result["signal_summaries"][0]["final"], 0.0);
    let warnings = result["warnings"].to_string();
    assert!(!warnings.contains("pause requested"), "{warnings}");
    assert!(result["artifacts"]["raw_bytes"].as_u64().unwrap() > 0);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_transient_past_its_budget_reads_partial_with_its_data_kept() {
    let result = runaway(json!({ "timeout_seconds": 2 }));
    assert_stopped_with_data(&result, "timeout");
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_transient_at_its_size_limit_reads_partial_with_its_data_kept() {
    const LIMIT: u64 = 2 * 1024 * 1024;
    let result = runaway(json!({ "max_raw_bytes": LIMIT }));
    assert_stopped_with_data(&result, "size_limit");
    let bytes = result["artifacts"]["raw_bytes"].as_u64().unwrap();
    assert!((LIMIT..LIMIT + LIMIT / 2).contains(&bytes), "{bytes}");
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_transient_that_loses_convergence_keeps_what_it_had_and_names_the_node() {
    let lines: Vec<String> = [
        "V1 a 0 PWL(0 0 1m 1 3m 1)",
        "B1 x 0 I=1/(1e-9+0*V(a))*(V(x)-1)*exp(1000*time*V(a)*V(x))",
        "R1 x 0 1k",
        "R2 a 0 1k",
    ]
    .iter()
    .map(|line| line.to_string())
    .collect();
    let result = run_on_testbench(
        &lines,
        json!({
            "analysis": { "kind": "transient", "stop_time_seconds": 3e-3 },
            "measurements": [{ "name": "top", "kind": "max", "signal": "v(a)" }],
        }),
    );

    assert_eq!(result["stopped"]["reason"], "no_convergence", "{result}");
    assert_eq!(result["stopped"]["node"], "b1#branch", "{result}");
    assert_eq!(result["outcome"]["status"], "partial");
    let reached = result["analysis"]["simulated_to_seconds"].as_f64().unwrap();
    assert!(reached > 1e-3 && reached < 3e-3, "{reached}");
    assert_eq!(result["measurements"][0]["window_truncated"], true);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn the_size_limit_counts_every_vector_the_engine_saves() {
    const LIMIT: u64 = 1 << 21;
    let mut run = WorkerRun::prepare(SLOW_DECK, None, 30_000);
    run.limit_size(LIMIT);

    let status = run.finish();

    assert_eq!(status.halted_reason, Some(HaltReason::SizeLimit));
    assert_eq!(status.sim_status, Some(0));
    assert!(status.results_written);
    let plot = run.plot();
    let bytes = 8 * plot.points as u64 * plot.variables.len() as u64;
    assert!((LIMIT..LIMIT + LIMIT / 2).contains(&bytes), "{bytes}");
}

fn lines(deck: &[&str]) -> Vec<String> {
    deck.iter().map(|line| line.to_string()).collect()
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn an_initial_voltage_holds_whichever_way_round_the_part_is_drawn() {
    for capacitor in ["C1 0 a 1u", "C1 a 0 1u"] {
        let result = run_on_testbench(
            &lines(&[capacitor, "R1 a 0 1k", ".ic v(a)=5"]),
            json!({
                "analysis": {
                    "kind": "transient",
                    "stop_time_seconds": 2e-3,
                    "max_step_seconds": 1e-6,
                    "use_initial_conditions": true,
                },
                "signals": ["p(R1)"],
                "measurements": [
                    { "name": "at_1ms", "kind": "value_at", "signal": "v(a)", "at_seconds": 1e-3 },
                ],
            }),
        );

        // 5 e^-1, from the RC time constant.
        near(measured(&result, "at_1ms"), 1.8394, 2e-3);
        assert!(result["warnings"]
            .to_string()
            .contains("device_currents_not_saved"));
        assert_eq!(result["signal_summaries"][0]["finding"], "signal_not_found");
    }
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_nodeset_under_uic_holds_as_an_initial_voltage_does() {
    let result = run_on_testbench(
        &lines(&["C1 0 a 1u", "R1 a 0 1k", ".nodeset v(a)=5"]),
        json!({
            "analysis": {
                "kind": "transient",
                "stop_time_seconds": 2e-3,
                "max_step_seconds": 1e-6,
                "use_initial_conditions": true,
            },
            "measurements": [
                { "name": "at_1ms", "kind": "value_at", "signal": "v(a)", "at_seconds": 1e-3 },
            ],
        }),
    );

    near(measured(&result, "at_1ms"), 1.8394, 2e-3);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn the_engines_sample_thinning_option_cannot_reach_a_run() {
    let result = run_on_testbench(
        &lines(&["V1 a 0 1", "R1 a 0 1k", ".options interp"]),
        json!({ "analysis": { "kind": "transient", "stop_time_seconds": 1e-3 } }),
    );

    assert_eq!(result["error"]["reason"], "unsafe_deck_content", "{result}");
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_step_cut_off_while_still_rising_gives_no_number_and_a_finished_one_does() {
    let deck = lines(&[
        "V1 in 0 PULSE(0 5 0 1n 1n 100 200)",
        "R1 in a 1k",
        "C1 a 0 1u",
    ]);
    let run = |stop: f64| {
        run_on_testbench(
            &deck,
            json!({
                "analysis": {
                    "kind": "transient",
                    "stop_time_seconds": stop,
                    "max_step_seconds": 1e-6,
                },
                "measurements": [
                    { "name": "rise", "kind": "rise_time", "signal": "v(a)" },
                    { "name": "settle", "kind": "settling_time", "signal": "v(a)" },
                ],
            }),
        )
    };

    let cut_off = run(3e-3);
    assert_eq!(cut_off["measurements"][0]["finding"], "final_value_needed");
    assert_eq!(cut_off["measurements"][1]["finding"], "final_value_needed");
    let finished = run(12e-3);
    near(measured(&finished, "rise"), 2.197e-3, 3e-6);
    near(measured(&finished, "settle"), 3.912e-3, 3e-6);
    near(
        finished["measurements"][0]["final_value"].as_f64().unwrap(),
        5.0,
        1e-3,
    );
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn an_initial_value_on_a_device_keeps_its_probes_and_says_the_run_has_no_start() {
    let result = run_on_testbench(
        &lines(&["C1 0 a 1u IC=-5", "R1 a 0 1k"]),
        json!({
            "analysis": {
                "kind": "transient",
                "stop_time_seconds": 2e-3,
                "max_step_seconds": 1e-6,
                "use_initial_conditions": true,
            },
            "measurements": [
                { "name": "at_1ms", "kind": "value_at", "signal": "v(a)", "at_seconds": 1e-3 },
                { "name": "current", "kind": "max", "signal": "i(R1)" },
            ],
        }),
    );

    near(measured(&result, "at_1ms"), 1.8394, 2e-3);
    near(measured(&result, "current"), 5e-3, 1e-5);
    assert!(result["analysis"]["first_point_seconds"].as_f64().unwrap() > 0.0);
    assert!(result["warnings"].to_string().contains("no_point_at_start"));
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn every_inductor_current_is_found_though_the_engine_reports_it_twice() {
    let result = run_on_testbench(
        &lines(&[
            "V1 a 0 PULSE(0 12 10u 1u 1u 5m 10m)",
            "L1 a b 1m",
            "R1 b c 1",
            "L2 c d 1m",
            "C1 d 0 10u",
            "R2 d 0 2",
        ]),
        json!({
            "analysis": { "kind": "transient", "stop_time_seconds": 1e-3 },
            "signals": ["i(L1)", "i(L2)"],
        }),
    );

    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    let summaries = &result["signal_summaries"];
    assert_eq!(summaries[0]["unit"], "A", "{result}");
    near(
        summaries[0]["max"].as_f64().unwrap(),
        summaries[1]["max"].as_f64().unwrap(),
        1e-6,
    );
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_probe_on_a_coupling_cannot_kill_the_run() {
    let result = run_on_testbench(
        &lines(&[
            "V1 a 0 SIN(0 1 1k)",
            "L1 a 0 1m",
            "L2 b 0 1m",
            "K1 L1 L2 .9",
            "R1 b 0 1k",
        ]),
        json!({
            "analysis": { "kind": "transient", "stop_time_seconds": 1e-3 },
            "signals": ["p(K1)", "v(b)"],
        }),
    );

    assert_eq!(result["outcome"]["status"], "partial", "{result}");
    assert_eq!(result["signal_summaries"][0]["finding"], "signal_not_found");
    assert!(result["signal_summaries"][1]["max"].as_f64().unwrap() > 0.0);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_save_start_near_the_end_keeps_the_ordinary_step() {
    let result = run_on_testbench(
        &lines(&["V1 a 0 SIN(0 1 1k)", "R1 a 0 1k"]),
        json!({
            "analysis": {
                "kind": "transient",
                "stop_time_seconds": 1,
                "start_save_seconds": 0.999999,
            },
            "signals": ["v(a)"],
            "timeout_seconds": 10,
        }),
    );

    assert_eq!(result["outcome"]["status"], "complete", "{result}");
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

// Protection stand-ins. The decks are the ones the `kicad-simulation` skill shows,
// and `protection/reference.py` works the expected values out with no SPICE engine.

fn protection_deck(name: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../konnect-core/tests/fixtures/simulation/protection")
        .join(format!("{name}.sp"));
    let text = std::fs::read_to_string(path).unwrap();
    text.lines().map(str::to_owned).collect()
}

/// The same deck with one line changed, so a missing line fails the test.
fn with_line_replaced(mut deck: Vec<String>, old: &str, new: &str) -> Vec<String> {
    let line = deck
        .iter_mut()
        .find(|line| line.as_str() == old)
        .unwrap_or_else(|| panic!("no line {old:?}"));
    *line = new.to_string();
    deck
}

fn transient(stop: f64, step: f64) -> Value {
    json!({ "kind": "transient", "stop_time_seconds": stop, "max_step_seconds": step })
}

fn assert_clean_and_modified(result: &Value) {
    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    assert_eq!(result["simulated_circuit"], "modified", "{result}");
    assert_eq!(result["warnings"], json!([]), "{result}");
    assert_eq!(result["analysis"]["coverage_ratio"], 1.0, "{result}");
}

/// The time a `crossing` measurement returns on the analysis axis.
fn at(result: &Value, name: &str) -> f64 {
    measured(result, name)
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn an_output_short_draws_the_current_the_circuit_equation_gives() {
    let result = run_on_testbench(
        &protection_deck("short"),
        json!({
            "analysis": transient(10e-3, 1e-6),
            "signals": ["v(out)"],
            "measurements": [
                { "name": "before", "kind": "value_at", "signal": "i(VBAT)", "at_seconds": 4e-3 },
                { "name": "peak", "kind": "peak_abs", "signal": "i(VBAT)" },
                { "name": "after", "kind": "value_at", "signal": "i(VBAT)", "at_seconds": 9.9e-3 },
                { "name": "output", "kind": "value_at", "signal": "v(out)", "at_seconds": 9.9e-3 },
            ],
        }),
    );

    assert_clean_and_modified(&result);
    // From `protection/reference.py`. A delivering source reads negative.
    near(measured(&result, "before"), -0.9983361, 1e-6);
    near(measured(&result, "peak"), -480.03999, 1e-3);
    near(measured(&result, "after"), -480.03999, 1e-3);
    near(measured(&result, "output"), 2.3992003, 1e-6);
    // A smooth edge needs no more than a few points per microsecond of it.
    let points = result["analysis"]["points"].as_u64().unwrap();
    assert!(points < 20_000, "{points}");
}

fn fuse_run(deck: Vec<String>, stop: f64) -> Value {
    run_on_testbench(
        &deck,
        json!({
            "analysis": transient(stop, 1e-7),
            "measurements": [
                { "name": "before", "kind": "value_at", "signal": "i(VFZS)", "at_seconds": 4e-3 },
                { "name": "peak", "kind": "peak_abs", "signal": "i(VFZS)" },
                { "name": "open", "kind": "crossing", "signal": "i(VFZS)", "level": 100.0,
                  "direction": "falling", "from_seconds": 5e-3 },
                { "name": "let_through", "kind": "value_at", "signal": "v(fz_q)", "at_seconds": stop - 1e-4 },
                { "name": "after", "kind": "value_at", "signal": "i(VFZS)", "at_seconds": stop - 1e-4 },
            ],
        }),
    )
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn an_i2t_fuse_opens_when_the_integral_reaches_its_rating_and_stays_open() {
    let result = fuse_run(protection_deck("fuse"), 10e-3);

    assert_clean_and_modified(&result);
    // From `protection/reference.py`, which integrates i squared by RK4: the fuse
    // trips at 2.002 A2s, 5.020498 ms.
    near(measured(&result, "before"), 0.9979210, 1e-6);
    near(at(&result, "open"), 5.020498e-3, 1e-7);
    // A step cannot stop the integral on the rating: it overshoots by at most
    // one step of the prospective current squared, 400 A and 0.1 us.
    let let_through = measured(&result, "let_through");
    assert!(
        (2.002..2.002 + 0.017).contains(&let_through),
        "{let_through}"
    );
    near(measured(&result, "peak"), 399.5503, 0.05);
    // 12 V through the open 1 Mohm, and it stays open for the rest of the run.
    near(measured(&result, "after"), 1.2e-5, 1e-8);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn an_i2t_fuse_carrying_less_than_its_rating_does_not_open() {
    let deck = with_line_replaced(
        protection_deck("fuse"),
        ".param fz_rated=2",
        ".param fz_rated=3000",
    );
    let result = fuse_run(deck, 10e-3);

    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    // The reference integrates 782.83 A2s by 9.9 ms, a quarter of the rating.
    near(measured(&result, "let_through"), 782.83, 0.1);
    near(measured(&result, "after"), 400.0278, 0.01);
    assert!(result["measurements"][2]["value"].is_null(), "{result}");
    assert_eq!(result["measurements"][2]["finding"], "level_not_reached");
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn an_efuse_holds_its_limit_into_a_short_and_drops_only_its_resistance_below_it() {
    let result = run_on_testbench(
        &protection_deck("efuse"),
        json!({
            "analysis": transient(10e-3, 2e-7),
            "signals": ["v(out)"],
            "measurements": [
                { "name": "load_current", "kind": "value_at", "signal": "i(VBAT)", "at_seconds": 4e-3 },
                { "name": "load_output", "kind": "value_at", "signal": "v(out)", "at_seconds": 4e-3 },
                { "name": "worst", "kind": "peak_abs", "signal": "i(VBAT)" },
                { "name": "short_current", "kind": "value_at", "signal": "i(VBAT)", "at_seconds": 9.9e-3 },
                { "name": "short_output", "kind": "value_at", "signal": "v(out)", "at_seconds": 9.9e-3 },
            ],
        }),
    );

    assert_clean_and_modified(&result);
    // From `protection/reference.py`: 5 tanh(v / 0.1) against a 12 ohm load.
    near(measured(&result, "load_current"), -0.99831349, 1e-6);
    near(measured(&result, "load_output"), 11.9797619, 1e-5);
    // Never above the limit, through the whole fall into the short.
    let worst = measured(&result, "worst").abs();
    assert!((5.0..5.0001).contains(&worst), "{worst}");
    near(measured(&result, "short_current"), -5.0, 1e-6);
    near(measured(&result, "short_output"), 0.0249896, 1e-6);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn an_undervoltage_lockout_closes_at_its_on_threshold_and_opens_at_its_off_threshold() {
    let result = run_on_testbench(
        &protection_deck("uvlo"),
        json!({
            "analysis": transient(28e-3, 1e-5),
            "signals": ["v(out)"],
            "measurements": [
                { "name": "on", "kind": "crossing", "signal": "v(out)", "level": 5.0, "direction": "rising" },
                { "name": "off", "kind": "crossing", "signal": "v(out)", "level": 5.0, "direction": "falling" },
                { "name": "between_up", "kind": "value_at", "signal": "v(out)", "at_seconds": 9.5e-3 },
                { "name": "between_down", "kind": "value_at", "signal": "v(out)", "at_seconds": 18.5e-3 },
            ],
        }),
    );

    assert_clean_and_modified(&result);
    // The 1 V per ms ramp reaches 10 V at 10 ms and falls through 9 V at 19 ms. A
    // crossing is interpolated inside the one-step jump, so each is right to within
    // one 10 us step: 10 mV.
    near(at(&result, "on"), 10e-3, 1e-5);
    near(at(&result, "off"), 19e-3, 1e-5);
    // At 9.5 V it is off on the way up and on on the way down: the hysteresis.
    near(measured(&result, "between_up"), 1.14e-7, 1e-6);
    near(measured(&result, "between_down"), 9.4920899, 1e-5);
    let points = result["analysis"]["points"].as_u64().unwrap();
    assert!(points < 5_000, "{points}");
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_reversed_battery_is_blocked_to_its_leakage_and_a_forward_one_drops_the_on_resistance() {
    let result = run_on_testbench(
        &protection_deck("reverse"),
        json!({
            "analysis": transient(24e-3, 1e-5),
            "signals": ["v(out)"],
            "measurements": [
                { "name": "reverse_current", "kind": "value_at", "signal": "i(VBAT)", "at_seconds": 0.0 },
                { "name": "reverse_worst", "kind": "peak_abs", "signal": "i(VBAT)", "to_seconds": 11e-3 },
                { "name": "reverse_output", "kind": "min", "signal": "v(out)", "to_seconds": 11e-3 },
                { "name": "forward_current", "kind": "value_at", "signal": "i(VBAT)", "at_seconds": 24e-3 },
                { "name": "forward_output", "kind": "value_at", "signal": "v(out)", "at_seconds": 24e-3 },
            ],
        }),
    );

    assert_clean_and_modified(&result);
    // From `protection/reference.py`: -12 V into 12 Mohm and 12 ohm, then +12 V
    // through 30 mohm, the knee included. i(VBAT) is the current into the battery.
    near(measured(&result, "reverse_current"), 9.99999e-7, 1e-10);
    near(measured(&result, "reverse_worst"), 9.99999e-7, 1e-10);
    near(measured(&result, "reverse_output"), -1.1999988e-5, 1e-9);
    near(measured(&result, "forward_current"), -0.99750007, 1e-6);
    near(measured(&result, "forward_output"), 11.9700008, 1e-5);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_stand_in_joins_a_real_schematic_by_the_net_name_kicad_shows() {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/rectifier"), project.path());
    let result = call_over_stdio(
        "run_simulation",
        json!({
            "schematic": project.path().join("rectifier.kicad_sch"),
            "analysis": transient(1e-3, 1e-5),
            "testbench": {
                "exclude_references": ["V1", "D1", "R1"],
                "spice_lines": protection_deck("spliced"),
            },
            "measurements": [
                { "name": "rail", "kind": "value_at", "signal": "v(/rect_out)", "at_seconds": 1e-3 },
                { "name": "draw", "kind": "value_at", "signal": "i(VBAT)", "at_seconds": 1e-3 },
            ],
        }),
    );

    assert_clean_and_modified(&result);
    // R2 is 100 kohm: 120 uA, and 0.1 ohm * atanh(24e-6) = 2.4 uV across the stand-in.
    near(measured(&result, "rail"), 12.0 - 2.4e-6, 1e-7);
    near(measured(&result, "draw"), -1.2e-4, 1e-8);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn every_stand_in_runs_under_initial_conditions_and_keeps_its_device_currents() {
    // (deck, stop, step, signal, time, expected). Values are the ones above.
    let cases: [(&str, f64, f64, &str, f64, f64); 5] = [
        ("short", 10e-3, 1e-6, "i(VBAT)", 9.9e-3, -480.03999),
        ("fuse", 10e-3, 1e-7, "i(VFZS)", 9.9e-3, 1.2e-5),
        ("efuse", 10e-3, 2e-7, "i(VBAT)", 9.9e-3, -5.0),
        ("uvlo", 28e-3, 1e-5, "v(out)", 16e-3, 11.990008),
        ("reverse", 24e-3, 1e-5, "i(VBAT)", 24e-3, -0.99750007),
    ];
    for (name, stop, step, signal, time, expected) in cases {
        let mut analysis = transient(stop, step);
        analysis["use_initial_conditions"] = json!(true);
        let result = run_on_testbench(
            &protection_deck(name),
            json!({
                "analysis": analysis,
                // A device current, which a `.ic` or a stand-in `.probe` would have cost.
                "measurements": [
                    { "name": "value", "kind": "value_at", "signal": signal, "at_seconds": time },
                    { "name": "load", "kind": "value_at", "signal": "i(RLOAD)", "at_seconds": time },
                ],
            }),
        );

        assert_eq!(result["outcome"]["status"], "complete", "{name}: {result}");
        assert_eq!(result["outcome"]["failed"], 0, "{name}: {result}");
        let codes: Vec<&str> = result["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|warning| warning["code"].as_str())
            .collect();
        assert_eq!(codes, ["no_point_at_start"], "{name}: {result}");
        near(
            measured(&result, "value"),
            expected,
            expected.abs() * 1e-5 + 1e-8,
        );
    }
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_lockout_narrower_than_the_supply_sag_chatters_and_stops_the_run() {
    // 3 ohm ahead of the input: the 12 ohm load pulls 10 V down to 8, below the off point.
    let mut deck = protection_deck("uvlo");
    deck[1] = "VSRC vsrc 0 PWL(0 0 14m 14 28m 0)".to_string();
    deck.push("RSAG vsrc vin 3".to_string());
    let result = run_on_testbench(
        &deck,
        json!({
            "analysis": transient(28e-3, 1e-5),
            "measurements": [{ "name": "top", "kind": "max", "signal": "v(out)" }],
        }),
    );

    assert_eq!(result["outcome"]["status"], "partial", "{result}");
    assert_eq!(result["stopped"]["reason"], "no_convergence", "{result}");
    let reached = result["analysis"]["simulated_to_seconds"].as_f64().unwrap();
    near(reached, 10e-3, 1e-6);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn exp_in_an_expression_is_linear_above_an_argument_of_fourteen() {
    // Why the decks write powers with `**`. If the engine ever fixes this, the
    // skill's warning can go.
    let lines = lines(&[
        "VX x 0 1",
        "RX x 0 1k",
        "BEXP e 0 V='exp(ln(1G))'",
        "RE e 0 1k",
        "BPOW p 0 V='1G**1'",
        "RP p 0 1k",
        "BLOW l 0 V='exp(10)'",
        "RL l 0 1k",
    ]);
    let result = run_on_testbench(
        &lines,
        json!({
            "analysis": transient(1e-3, 1e-4),
            "measurements": [
                { "name": "exp", "kind": "value_at", "signal": "v(e)", "at_seconds": 1e-4 },
                { "name": "pow", "kind": "value_at", "signal": "v(p)", "at_seconds": 1e-4 },
                { "name": "low", "kind": "value_at", "signal": "v(l)", "at_seconds": 1e-4 },
            ],
        }),
    );

    // exp(14) * (1 + 20.723 - 14): 9.288e6 where the exact value is 1e9.
    near(measured(&result, "exp"), 9.288032e6, 1.0);
    near(measured(&result, "pow"), 1e9, 1.0);
    near(measured(&result, "low"), 10f64.exp(), 1e-6);
}

const AC_DECK: &str = "title\nV1 a 0 DC 1 AC 1\nR1 a b 1k\nC1 b 0 1u\nR2 b 0 100\n.save all\n\
                       .ac dec 10000 1 1G\n.end\n";

/// The bytes of an AC plot, which holds a real and an imaginary double per value.
fn complex_bytes(plot: &raw::Plot) -> u64 {
    16 * plot.points as u64 * plot.variables.len() as u64
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn the_size_limit_counts_a_complex_value_as_two_doubles() {
    const LIMIT: u64 = 1 << 20;
    let mut run = WorkerRun::prepare(AC_DECK, None, 30_000);
    run.limit_size(LIMIT);
    run.make_complex();

    let status = run.finish();

    assert_eq!(status.halted_reason, Some(HaltReason::SizeLimit));
    assert!(status.results_written);
    let plot = run.plot();
    assert!(plot.complex);
    let bytes = complex_bytes(&plot);
    assert!((LIMIT..LIMIT + LIMIT / 2).contains(&bytes), "{bytes}");

    // Told nothing, the worker counts a real double and lets the data reach twice
    // the limit: the reason the server sends the flag.
    let mut real = WorkerRun::prepare(AC_DECK, None, 30_000);
    real.limit_size(LIMIT);
    let status = real.finish();
    assert_eq!(status.halted_reason, Some(HaltReason::SizeLimit));
    let bytes = complex_bytes(&real.plot());
    assert!(bytes > LIMIT + LIMIT / 2, "{bytes}");
}

fn ac_loop_lines() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../konnect-core/tests/fixtures/simulation/ac_loop.sp");
    let text = std::fs::read_to_string(path).unwrap();
    text.lines().map(str::to_owned).collect()
}

fn ac_loop(extra: Value) -> Value {
    let mut arguments = json!({
        "analysis": { "kind": "ac", "start_hz": 1.0, "stop_hz": 1e6, "points": 200 },
        "signals": ["v(out)"],
    });
    for (key, value) in extra.as_object().unwrap() {
        arguments[key] = value.clone();
    }
    run_on_testbench(&ac_loop_lines(), arguments)
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_three_pole_loop_gives_the_margins_the_formula_does() {
    let result = ac_loop(json!({
        "measurements": [
            { "name": "pm", "kind": "phase_margin", "signal": "v(out)" },
            { "name": "gm", "kind": "gain_margin", "signal": "v(out)" },
            { "name": "bw", "kind": "bandwidth_3db", "signal": "v(out)" },
            { "name": "at_1k", "kind": "value_at", "signal": "v(out)", "at_hz": 1000.0 },
        ],
    }));

    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    assert_eq!(result["outcome"]["requested"], 6);
    assert_eq!(result["simulated_circuit"], "modified");
    assert_eq!(result["analysis"]["coverage_ratio"], 1.0, "{result}");
    assert_eq!(result["warnings"], json!([]), "{result}");
    // From `ac_loop_reference.py`, which solves the formulas with no SPICE engine.
    near(measured(&result, "pm"), 3.485942, 2e-3);
    near(measured(&result, "gm"), 1.742135, 2e-3);
    near(measured(&result, "bw"), 99.014567, 0.05);
    near(measured(&result, "at_1k"), 16.903273, 1e-3);
    let items = result["measurements"].as_array().unwrap();
    let find = |name: &str| items.iter().find(|item| item["name"] == name).unwrap();
    near(find("pm")["crossover_hz"].as_f64().unwrap(), 3013.707, 0.5);
    near(
        find("gm")["phase_crossover_hz"].as_f64().unwrap(),
        3331.666,
        0.5,
    );
    near(
        find("at_1k")["phase_degrees"].as_f64().unwrap(),
        -135.0,
        1e-3,
    );
    // A loop that starts at zero degrees, so the unwrapping began on the right side.
    near(
        find("pm")["start_phase_degrees"].as_f64().unwrap(),
        -0.6359,
        0.01,
    );
    let summary = &result["signal_summaries"][0];
    near(
        summary["low_frequency_gain_db"].as_f64().unwrap(),
        40.0,
        1e-3,
    );
    assert_eq!(summary["unit"], "dB");
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_loop_gain_taken_as_a_ratio_of_two_nodes_gives_the_same_margins() {
    let result = ac_loop(json!({
        "measurements": [
            { "name": "pm", "kind": "phase_margin", "signal": "v(out)", "reference_signal": "v(in)" },
            { "name": "gm", "kind": "gain_margin", "signal": "v(out)", "reference_signal": "v(in)" },
        ],
    }));

    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    near(measured(&result, "pm"), 3.485942, 2e-3);
    near(measured(&result, "gm"), 1.742135, 2e-3);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn octave_and_linear_sweeps_cover_what_the_engine_plans() {
    for (analysis, last) in [
        // Ends on the last grid point at or below the stop, short of it.
        (
            json!({ "kind": "ac", "start_hz": 7.0, "stop_hz": 1000.0, "scale": "octave", "points": 3 }),
            896.0,
        ),
        (
            json!({ "kind": "ac", "start_hz": 1.0, "stop_hz": 5.0, "points": 10 }),
            3.98107,
        ),
        (
            json!({ "kind": "ac", "start_hz": 100.0, "stop_hz": 1100.0, "scale": "linear", "points": 11 }),
            1100.0,
        ),
        (
            json!({ "kind": "ac", "start_hz": 3.0, "stop_hz": 5500.0, "points": 7 }),
            5500.0,
        ),
    ] {
        let result = ac_loop(json!({ "analysis": analysis.clone() }));
        assert_eq!(
            result["outcome"]["status"], "complete",
            "{analysis} {result}"
        );
        let reached = result["analysis"]["simulated_to_hz"].as_f64().unwrap();
        assert!((reached / last - 1.0).abs() < 1e-4, "{analysis}: {reached}");
        let short = analysis["stop_hz"].as_f64().unwrap() * 0.999 > reached;
        assert_eq!(
            result["warnings"]
                .to_string()
                .contains("sweep_ends_before_stop"),
            short,
            "{analysis} {result}"
        );
    }
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_real_filter_with_a_vendor_model_matches_its_formula() {
    // KiCad's sallen_key demo: unity gain, R = 1 k and C = 100 n twice, on an
    // AD8051 model. Equal parts give Q = 1/2, a double pole at 1/(2 pi R C), a gain
    // of -6.02 dB there with -90 degrees, and a -3 dB edge at f0 sqrt(sqrt(2) - 1).
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/sallen_key"), project.path());
    let before = file_names(project.path());
    let f0 = 1.0 / (2.0 * std::f64::consts::PI * 1e3 * 100e-9);

    let result = call_over_stdio(
        "run_simulation",
        json!({
            "schematic": project.path().join("sallen_key.kicad_sch"),
            "analysis": { "kind": "ac", "start_hz": 10.0, "stop_hz": 1e6, "points": 200 },
            "measurements": [
                { "name": "edge", "kind": "bandwidth_3db", "signal": "v(/lowpass)" },
                { "name": "at_f0", "kind": "value_at", "signal": "v(/lowpass)", "at_hz": f0 },
            ],
        }),
    );

    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    assert_eq!(result["simulated_circuit"], "as_drawn");
    let dropped = result["circuit"]["dropped_directives"].to_string();
    assert!(
        dropped.contains(".ac") && dropped.contains(".control"),
        "{dropped}"
    );
    near(
        measured(&result, "edge") / f0,
        (2f64.sqrt() - 1.0).sqrt(),
        1e-4,
    );
    near(measured(&result, "at_f0"), -6.0206, 2e-3);
    let phase = result["measurements"][1]["phase_degrees"].as_f64().unwrap();
    near(phase, -90.0, 0.01);
    assert_eq!(file_names(project.path()), before);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn an_ac_sweep_with_nothing_to_drive_it_is_refused_on_a_real_schematic() {
    let project = tempfile::tempdir().unwrap();
    copy_tree(
        &demos().join("simulation/analog-multiplier"),
        project.path(),
    );

    let result = call_over_stdio(
        "run_simulation",
        json!({
            "schematic": project.path().join("a-multi.kicad_sch"),
            "analysis": { "kind": "ac", "start_hz": 10.0, "stop_hz": 1e4 },
        }),
    );

    assert_eq!(result["error"]["reason"], "no_ac_stimulus", "{result}");
    assert_eq!(result["outcome"]["status"], "failed");
}

fn ramp_and_sweep() -> (Value, Value) {
    let ramp = run_on_testbench(
        &protection_deck("reverse"),
        json!({
            "analysis": transient(24e-3, 5e-7),
            "measurements": REVERSE_PLACES.iter().flat_map(|&volts| [
                json!({ "name": format!("out {volts}"), "kind": "value_at", "signal": "v(out)",
                        "at_seconds": (volts + 12.0) / 1000.0 }),
                json!({ "name": format!("in {volts}"), "kind": "value_at", "signal": "i(VBAT)",
                        "at_seconds": (volts + 12.0) / 1000.0 }),
            ]).collect::<Vec<_>>(),
        }),
    );
    let sweep = run_on_testbench(
        &protection_deck("reverse"),
        json!({
            "analysis": { "kind": "dc_sweep", "sweeps": [
                { "target": "VBAT", "start_value": -12.0, "stop_value": 12.0, "step_value": 0.05 }
            ] },
            "signals": ["v(out)", "i(VBAT)"],
            "measurements": REVERSE_PLACES.iter().flat_map(|&volts| [
                json!({ "name": format!("out {volts}"), "kind": "value_at", "signal": "v(out)",
                        "at_sweep": volts }),
                json!({ "name": format!("in {volts}"), "kind": "value_at", "signal": "i(VBAT)",
                        "at_sweep": volts }),
            ]).collect::<Vec<_>>(),
        }),
    );
    (ramp, sweep)
}

const REVERSE_PLACES: [f64; 10] = [-12.0, -6.0, -1.0, -0.1, -0.05, 0.05, 0.1, 1.0, 6.0, 12.0];

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn the_reversed_battery_swept_from_minus_12_to_12_volts_agrees_with_its_ramp() {
    let (ramp, sweep) = ramp_and_sweep();

    assert_clean_and_modified(&ramp);
    assert_clean_and_modified(&sweep);
    assert_eq!(sweep["analysis"]["points"], 481, "{sweep}");
    for volts in REVERSE_PLACES {
        for signal in ["out", "in"] {
            let name = format!("{signal} {volts}");
            let (ramped, swept) = (measured(&ramp, &name), measured(&sweep, &name));
            // The ramp is read between steps of 0.5 us, which is 0.5 mV of battery. The
            // knee passes current in proportion to e^(V/5 mV), so a coarser step
            // reads 25 percent off at -50 mV by interpolation alone.
            near(swept, ramped, 1e-6 + 2e-3 * swept.abs());
        }
    }
    // From `protection/reference.py`: the same leakage and drop as the ramp gives.
    near(measured(&sweep, "out -12"), -1.1999988e-5, 1e-9);
    near(measured(&sweep, "in -12"), 9.99999e-7, 1e-10);
    near(measured(&sweep, "out 12"), 11.9700008, 1e-5);
    near(measured(&sweep, "in 12"), -0.99750007, 1e-6);
    let summary = &sweep["signal_summaries"][0];
    // Over the whole sweep the lowest output is not the -12 V leakage: within 50 mV
    // of zero the stand-in's 10 mV knee passes a little current backward, which the
    // handoff names and the ramp's summary would show too.
    near(summary["min"].as_f64().unwrap(), -1.108e-3, 2e-5);
    near(summary["max"].as_f64().unwrap(), 11.9700008, 1e-5);
    assert_eq!(sweep["analysis"]["sweeps"][0]["element"], "VBAT");
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_falling_sweep_and_a_rising_one_give_the_same_curve() {
    let lines = protection_deck("reverse");
    let run = |start: f64, stop: f64, step: f64, positive: [f64; 2]| {
        run_on_testbench(
            &lines,
            json!({
                "analysis": { "kind": "dc_sweep", "sweeps": [
                    { "target": "VBAT", "start_value": start, "stop_value": stop, "step_value": step }
                ] },
                "measurements": [
                    { "name": "out", "kind": "value_at", "signal": "v(out)", "at_sweep": 6.0 },
                    { "name": "turn_on", "kind": "crossing", "signal": "v(out)", "level": 5.0 },
                    { "name": "high", "kind": "max", "signal": "v(out)", "from_sweep": positive[0], "to_sweep": positive[1] },
                ],
            }),
        )
    };
    let (up, down) = (
        run(-12.0, 12.0, 0.5, [0.0, 12.0]),
        run(12.0, -12.0, -0.5, [12.0, 0.0]),
    );

    assert_eq!(up["outcome"]["status"], "complete", "{up}");
    assert_eq!(down["outcome"]["status"], "complete", "{down}");
    // The solver stops within a microvolt of the answer, from either side.
    near(measured(&up, "out"), measured(&down, "out"), 5e-6);
    // The crossing is where the output passes 5 V, going up one way and down the other.
    near(measured(&up, "turn_on"), measured(&down, "turn_on"), 1e-4);
    near(measured(&up, "high"), 11.9700008, 1e-5);
    near(measured(&down, "high"), 11.9700008, 1e-5);
    assert_eq!(down["analysis"]["points"], 49);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn two_dc_sweeps_run_the_whole_grid_with_the_first_sweeping_fastest() {
    let lines: Vec<String> = [
        "VA a 0 DC 0",
        "VB b 0 DC 0",
        "R1 a c 1k",
        "R2 b c 1k",
        "R3 c 0 1k",
    ]
    .map(str::to_owned)
    .to_vec();
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/rectifier"), project.path());
    let home = tempfile::tempdir().unwrap();
    let result = call_in_home(
        home.path(),
        "run_simulation",
        json!({
            "schematic": project.path().join("rectifier.kicad_sch"),
            "testbench": { "exclude_references": ["*"], "spice_lines": lines },
            "analysis": { "kind": "dc_sweep", "sweeps": [
                { "target": "VA", "start_value": 0.0, "stop_value": 2.0, "step_value": 1.0 },
                { "target": "VB", "start_value": 0.0, "stop_value": 3.0, "step_value": 1.0 },
            ] },
            "signals": ["v(c)"],
        }),
        "",
    );

    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    assert_eq!(result["analysis"]["points"], 12);
    assert_eq!(result["analysis"]["expected_points"], 12);
    let summary = &result["signal_summaries"][0];
    // v(c) = (VA + VB) / 3: nothing at the first point and (2 + 3) / 3 at the last.
    assert_eq!(summary["at_start"], 0.0, "{summary}");
    near(summary["at_stop"].as_f64().unwrap(), 5.0 / 3.0, 1e-9);
    near(summary["max"].as_f64().unwrap(), 5.0 / 3.0, 1e-9);
    // The raw file shows VA moving first: the third point has VA at 2 and VB at 0.
    let raw_path = result["artifacts"]["raw_path"].as_str().unwrap();
    let plots = raw::read(Path::new(raw_path), u64::MAX).unwrap();
    let plot = &plots[0];
    let a: Vec<f64> = plot.real(plot.variable("v(a)").unwrap()).collect();
    let b: Vec<f64> = plot.real(plot.variable("v(b)").unwrap()).collect();
    assert_eq!(&a[..6], [0.0, 1.0, 2.0, 0.0, 1.0, 2.0]);
    assert_eq!(&b[..6], [0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_resistor_and_the_temperature_can_be_swept() {
    let lines: Vec<String> = ["VS a 0 DC 10", "R1 a b 1k", "R2 b 0 1k"]
        .map(str::to_owned)
        .to_vec();
    let sweep = |target: &str, start: f64, stop: f64, step: f64| {
        run_on_testbench(
            &lines,
            json!({
                "analysis": { "kind": "dc_sweep", "sweeps": [
                    { "target": target, "start_value": start, "stop_value": stop, "step_value": step }
                ] },
                "measurements": [{ "name": "last", "kind": "value_at", "signal": "v(b)", "at_sweep": stop }],
            }),
        )
    };
    let resistor = sweep("R2", 1000.0, 3000.0, 1000.0);
    assert_eq!(resistor["outcome"]["status"], "complete", "{resistor}");
    near(measured(&resistor, "last"), 7.5, 1e-9);
    assert_eq!(resistor["analysis"]["sweeps"][0]["unit"], "Ω");
    let temperature = sweep("TEMP", -40.0, 125.0, 55.0);
    assert_eq!(
        temperature["outcome"]["status"], "complete",
        "{temperature}"
    );
    assert_eq!(temperature["analysis"]["points"], 4);
    assert_eq!(temperature["analysis"]["sweeps"][0]["unit"], "°C");
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_sweep_that_would_run_the_wrong_way_is_refused_where_the_engine_would_write_nothing() {
    let lines: Vec<String> = ["VS a 0 DC 10", "R1 a 0 1k"].map(str::to_owned).to_vec();
    let result = run_on_testbench(
        &lines,
        json!({ "analysis": { "kind": "dc_sweep", "sweeps": [
            { "target": "VS", "start_value": 0.0, "stop_value": 5.0, "step_value": -1.0 }
        ] } }),
    );
    assert_eq!(result["error"]["kind"], "invalid_argument", "{result}");
    assert_eq!(result["error"]["field"], "analysis.sweeps[0].step_value");
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_temperature_sweep_that_ends_one_point_short_is_complete_and_says_so() {
    // The engine adds 0.1 a thousand times and its last value is 99.90000000002, so it
    // saves 1000 points and not 1001 [verified: run E100].
    let lines: Vec<String> = ["VS a 0 DC 10", "R1 a 0 1k"].map(str::to_owned).to_vec();
    let result = run_on_testbench(
        &lines,
        json!({
            "analysis": { "kind": "dc_sweep", "sweeps": [
                { "target": "TEMP", "start_value": 0.0, "stop_value": 100.0, "step_value": 0.1 }
            ] },
            "measurements": [
                { "name": "last", "kind": "value_at", "signal": "i(VS)", "at_sweep": 99.9 },
                { "name": "at_stop", "kind": "value_at", "signal": "i(VS)", "at_sweep": 100.0 },
            ],
        }),
    );

    assert_eq!(result["analysis"]["points"], 1000, "{result}");
    assert_eq!(result["analysis"]["expected_points"], 1001);
    near(
        result["analysis"]["simulated_to_value"].as_f64().unwrap(),
        99.9,
        1e-6,
    );
    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    let warnings = result["warnings"].to_string();
    assert!(warnings.contains("sweep_ends_before_stop"), "{warnings}");
    let find = |name: &str| {
        let items = result["measurements"].as_array().unwrap();
        items
            .iter()
            .find(|item| item["name"] == name)
            .unwrap()
            .clone()
    };
    assert!(find("last")["value"].as_f64().is_some());
    // The engine never reached 100, and the result does not pretend it did.
    assert_eq!(find("at_stop")["finding"], "outside_data");
}

/// Two buffered second-order low-passes at 1 kHz with Q = 50 and a gain of 10: the
/// phase falls 360 degrees within a few percent of frequency.
fn resonant_loop(points: u64) -> Value {
    let lines: Vec<String> = [
        "VIN in 0 DC 0 AC 1",
        "R1 in a 3.183",
        "L1 a b 25.33m",
        "C1 b 0 1u",
        "E1 c 0 b 0 1",
        "R2 c d 3.183",
        "L2 d e 25.33m",
        "C2 e 0 1u",
        "E2 out 0 e 0 10",
        "RL out 0 1k",
    ]
    .map(str::to_owned)
    .to_vec();
    run_on_testbench(
        &lines,
        json!({
            "analysis": { "kind": "ac", "start_hz": 1.1, "stop_hz": 1e5, "points": points },
            "measurements": [
                { "name": "pm", "kind": "phase_margin", "signal": "v(out)" },
                { "name": "gm", "kind": "gain_margin", "signal": "v(out)" },
            ],
        }),
    )
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_sweep_too_coarse_to_follow_the_phase_gives_no_margin_instead_of_a_stable_one() {
    // The formulas give a gain margin of -88 dB and a phase margin of -178.5 degrees.
    // At 50 points a decade the phase steps over its 360 degree turn, and an
    // unwrapping that trusted it read +181 degrees and no gain margin.
    let coarse = resonant_loop(50);
    for name in ["pm", "gm"] {
        let items = coarse["measurements"].as_array().unwrap();
        let item = items.iter().find(|item| item["name"] == name).unwrap();
        assert!(item["value"].is_null(), "{item}");
        assert_eq!(item["finding"], "phase_unreliable", "{item}");
        assert!(
            item["phase_unreliable_from_hz"].as_f64().is_some(),
            "{item}"
        );
    }
    let fine = resonant_loop(1000);
    near(measured(&fine, "pm"), -178.5, 0.1);
    near(measured(&fine, "gm"), -88.0, 0.5);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_source_written_as_ac_equals_or_ac_in_brackets_drives_a_sweep() {
    for source in [
        "V1 in 0 dc=0 ac=1",
        "V1 in 0 AC=1",
        "V1 in 0 DC 0 ac=1 0",
        "V1 in 0 DC 0 AC(1 0)",
        "V1 in 0 SIN(0 1 1k) ac = 1",
    ] {
        let lines: Vec<String> = [source, "R1 in out 1k", "C1 out 0 1u"]
            .map(str::to_owned)
            .to_vec();
        let result = run_on_testbench(
            &lines,
            json!({
                "analysis": { "kind": "ac", "start_hz": 10.0, "stop_hz": 1000.0 },
                "measurements": [{ "name": "g", "kind": "value_at", "signal": "v(out)", "at_hz": 10.0 }],
            }),
        );
        assert_eq!(
            result["outcome"]["status"], "complete",
            "{source}: {result}"
        );
        // 1k and 1 uF: 159 Hz corner, so -0.017 dB at 10 Hz.
        near(measured(&result, "g"), -0.0171, 1e-3);
    }
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn the_reversed_battery_stand_in_has_a_second_solution_near_zero_that_a_downward_sweep_lands_on() {
    // Between about -0.56 V and 0 the stand-in has two DC solutions, so which one the
    // engine finds depends on where it came from and the step. Swept downward from 0
    // it stays on; swept upward at the documented 0.05 V it blocks [verified: run E100].
    let lowest = |start: f64, stop: f64, step: f64| {
        let result = run_on_testbench(
            &protection_deck("reverse"),
            json!({
                "analysis": { "kind": "dc_sweep", "sweeps": [
                    { "target": "VBAT", "start_value": start, "stop_value": stop, "step_value": step }
                ] },
                "signals": ["v(out)"],
            }),
        );
        assert_eq!(result["outcome"]["status"], "complete", "{result}");
        result["signal_summaries"][0]["min"].as_f64().unwrap()
    };
    assert!(lowest(-12.0, 0.0, 0.05) > -0.005);
    assert!(lowest(0.0, -12.0, -0.01) < -0.3);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_grid_that_loses_its_last_outer_row_says_so() {
    let lines: Vec<String> = ["VS a 0 DC 10", "R1 a 0 1k"].map(str::to_owned).to_vec();
    let result = run_on_testbench(
        &lines,
        json!({
            "analysis": { "kind": "dc_sweep", "sweeps": [
                { "target": "VS", "start_value": 0.0, "stop_value": 1.0, "step_value": 1.0 },
                { "target": "TEMP", "start_value": 0.0, "stop_value": 100.0, "step_value": 0.1 },
            ] },
            "signals": ["i(VS)"],
        }),
    );

    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    assert_eq!(result["analysis"]["points"], 2000, "{result}");
    assert_eq!(result["analysis"]["expected_points"], 2002);
    let warnings = result["warnings"].to_string();
    assert!(warnings.contains("sweep_ends_before_stop"), "{warnings}");
    assert!(warnings.contains("TEMP ran 1000 of 1001"), "{warnings}");
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_sweep_that_lands_where_it_planned_says_nothing_about_its_end() {
    // 0 to 1.5 by 0.003 runs every one of 501 points, and ends at 1.49999999999.
    let lines: Vec<String> = ["VS a 0 DC 10", "R1 a 0 1k"].map(str::to_owned).to_vec();
    let result = run_on_testbench(
        &lines,
        json!({ "analysis": { "kind": "dc_sweep", "sweeps": [
            { "target": "TEMP", "start_value": 0.0, "stop_value": 1.5, "step_value": 0.003 }
        ] } }),
    );
    assert_eq!(result["analysis"]["points"], 501, "{result}");
    assert_eq!(result["warnings"], json!([]), "{result}");
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_linear_sweep_too_coarse_for_a_resonance_gives_no_margin_and_a_fine_one_gives_it() {
    // The same two-stage Q = 50 loop on a linear grid: 100 Hz steps step over the
    // resonance at 1 kHz, and 5001 points do not [verified: run E102].
    let lines: Vec<String> = [
        "V1 in 0 DC 0 AC 1",
        "R1 in a 1.25663706144",
        "L1 a b 0.01",
        "C1 b 0 2.53302959106e-06",
        "E1 c 0 b 0 1",
        "R2 c d 1.25663706144",
        "L2 d e 0.01",
        "C2 e 0 2.53302959106e-06",
        "E2 out 0 e 0 10",
        "RO out 0 1meg",
    ]
    .map(str::to_owned)
    .to_vec();
    let run = |points: u64| {
        run_on_testbench(
            &lines,
            json!({
                "analysis": { "kind": "ac", "start_hz": 50.0, "stop_hz": 10050.0, "scale": "linear", "points": points },
                "measurements": [
                    { "name": "pm", "kind": "phase_margin", "signal": "v(out)" },
                    { "name": "gm", "kind": "gain_margin", "signal": "v(out)" },
                ],
            }),
        )
    };
    let items = |result: &Value| result["measurements"].as_array().unwrap().clone();
    for item in items(&run(101)) {
        assert!(item["value"].is_null(), "{item}");
        assert_eq!(item["finding"], "phase_unreliable", "{item}");
    }
    let fine = run(5001);
    near(measured(&fine, "pm"), -178.52, 0.05);
    near(measured(&fine, "gm"), -87.96, 0.1);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_pole_at_the_start_of_a_linear_sweep_is_not_mistaken_for_a_resonance() {
    // One pole at 1 Hz on a sweep from 1 Hz by 10: the first step falls 20 dB, with a
    // phase that is easy to follow. A gain of 1e4 puts unity gain near 10 kHz.
    let lines: Vec<String> = [
        "V1 in 0 DC 0 AC 1",
        "R1 in out 1k",
        "C1 out 0 159.15494309189535m",
        "E1 o2 0 out 0 10000",
        "RL o2 0 1meg",
    ]
    .map(str::to_owned)
    .to_vec();
    let result = run_on_testbench(
        &lines,
        json!({
            "analysis": { "kind": "ac", "start_hz": 1.0, "stop_hz": 100001.0, "scale": "linear", "points": 10000 },
            "measurements": [{ "name": "pm", "kind": "phase_margin", "signal": "v(o2)" }],
        }),
    );
    assert_eq!(result["outcome"]["status"], "complete", "{result}");
    near(measured(&result, "pm"), 90.007, 0.01);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_peak_bandwidth_is_refused_where_the_peak_falls_between_samples_and_read_where_it_does_not() {
    // KiCad's amplifier-ac: a resonance at about 5 kHz. Its own `.ac dec 10` reads
    // 380 Hz and the answer is 521.7 [verified: run E101, E102].
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/amplifier-ac"), project.path());
    let run = |points: u64| {
        call_over_stdio(
            "run_simulation",
            json!({
                "schematic": project.path().join("amplifier-ac.kicad_sch"),
                "analysis": { "kind": "ac", "start_hz": 1.0, "stop_hz": 1e6, "points": points },
                "measurements": [{
                    "name": "bw", "kind": "bandwidth_3db", "signal": "v(VOUT)", "reference_level": "peak"
                }],
            }),
        )
    };
    for points in [10, 20] {
        let item = &run(points)["measurements"][0].clone();
        assert!(item["value"].is_null(), "{points}: {item}");
        assert_eq!(item["finding"], "sweep_too_coarse", "{points}: {item}");
    }
    // 50 a decade is within half a percent; 1000 is the reference.
    near(measured(&run(50), "bw"), 521.7, 3.0);
    near(measured(&run(1000), "bw"), 521.7, 0.5);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn linear_sweeps_that_step_over_a_resonance_or_a_steep_start_are_flagged_and_a_decade_sweep_reads_true(
) {
    // Review round 4 found both read as stable: the first step of a sweep is not
    // judged on its gain, so the step after it must be [verified: run E103].
    let two_stage: Vec<String> = [
        "V1 in 0 DC 0 AC 1",
        "R1 in a 1.25663706144",
        "L1 a b 0.01",
        "C1 b 0 2.53302959106e-06",
        "E1 c 0 b 0 1",
        "R2 c d 1.25663706144",
        "L2 d e 0.01",
        "C2 e 0 2.53302959106e-06",
        "E2 out 0 e 0 10",
        "RO out 0 1meg",
    ]
    .map(str::to_owned)
    .to_vec();
    let four_poles: Vec<String> = [
        "V1 in 0 DC 0 AC 1",
        "R1 in a 1k",
        "C1 a 0 15.915u",
        "E1 b 0 a 0 1",
        "R2 b c 1k",
        "C2 c 0 15.915u",
        "E2 d 0 c 0 1",
        "R3 d e 1k",
        "C3 e 0 15.915u",
        "E3 f 0 e 0 1",
        "R4 f g 1k",
        "C4 g 0 15.915u",
        "E4 out 0 g 0 6.25e6",
        "RL out 0 1meg",
    ]
    .map(str::to_owned)
    .to_vec();
    let measurements = json!([
        { "name": "pm", "kind": "phase_margin", "signal": "v(out)" },
        { "name": "gm", "kind": "gain_margin", "signal": "v(out)" },
    ]);
    let run = |lines: &[String], analysis: Value| {
        run_on_testbench(
            lines,
            json!({ "analysis": analysis, "measurements": measurements }),
        )
    };
    let flagged = |result: &Value| {
        let items = result["measurements"].as_array().unwrap();
        items
            .iter()
            .all(|item| item["value"].is_null() && item["finding"] == "phase_unreliable")
    };
    let linear = |start: f64, stop: f64, points: u64| json!({ "kind": "ac", "start_hz": start, "stop_hz": stop, "scale": "linear", "points": points });
    assert!(flagged(&run(&two_stage, linear(880.0, 100_880.0, 1001))));
    for points in [100, 200] {
        assert!(
            flagged(&run(&four_poles, linear(1.0, 1e4, points))),
            "{points}"
        );
    }
    let decade = run(
        &four_poles,
        json!({ "kind": "ac", "start_hz": 1.0, "stop_hz": 1e4, "points": 100 }),
    );
    near(measured(&decade, "pm"), -175.42, 0.1);
    near(measured(&decade, "gm"), -123.9, 0.2);
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_peak_that_two_samples_straddle_evenly_is_refused_and_one_on_a_sample_is_read() {
    let lines: Vec<String> = [
        "V1 in 0 DC 0 AC 1",
        "L1 in a 10m",
        "C1 a b 2.533u",
        "R1 b out 7.854",
        "RG out 0 1e-9",
    ]
    .map(str::to_owned)
    .to_vec();
    let run = |offset: f64| {
        run_on_testbench(
            &lines,
            json!({
                "analysis": { "kind": "ac", "start_hz": 1000.0 * 10f64.powf(offset),
                              "stop_hz": 1000.0 * 10f64.powf(offset + 2.0), "points": 50 },
                "measurements": [{ "name": "bw", "kind": "bandwidth_3db", "signal": "v(out)",
                                   "reference_level": "peak" }],
            }),
        )
    };
    // Q = 8: the width is 125 Hz, and an evenly straddled peak read 141 Hz.
    let straddled = run(-1.01);
    assert_eq!(
        straddled["measurements"][0]["finding"], "sweep_too_coarse",
        "{straddled}"
    );
    near(measured(&run(-1.0), "bw"), 125.0, 1.5);
}

/// One tool call in a server whose state folder is `home`, so a later call sees the
/// runs of an earlier one.
fn call_again(home: &Path, tool: &str, arguments: Value) -> Value {
    call_in_home(home, tool, arguments, "")
}

fn run_folders(home: &Path) -> usize {
    let Ok(projects) = std::fs::read_dir(home.join("simulation")) else {
        return 0;
    };
    projects
        .flatten()
        .map(|project| std::fs::read_dir(project.path()).unwrap().count())
        .sum()
}

fn csv_rows(path: &str) -> Vec<Vec<String>> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| line.split(',').map(str::to_owned).collect())
        .collect()
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_stored_soft_start_is_measured_again_and_listed_without_simulating() {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/rectifier"), project.path());
    let sheet = project.path().join("rectifier.kicad_sch");
    let home = tempfile::tempdir().unwrap();
    let run = call_again(
        home.path(),
        "run_simulation",
        json!({
            "schematic": sheet,
            "testbench": { "exclude_references": ["*"], "spice_lines": soft_start_lines() },
            "analysis": { "kind": "transient", "stop_time_seconds": 0.14, "max_step_seconds": 5e-7 },
            "signals": ["v(vout)"],
            "measurements": [
                { "name": "inrush_peak", "kind": "peak_abs", "signal": "i(VBAT)" },
                { "name": "rail_low", "kind": "min", "signal": "v(vout)", "from_seconds": 60e-3 },
            ],
        }),
    );
    assert_eq!(run["outcome"]["status"], "complete", "{run}");
    assert_eq!(run_folders(home.path()), 1);

    // The CSV holds the raw file's points under the raw file's names.
    let csv = csv_rows(run["artifacts"]["csv_path"].as_str().unwrap());
    assert_eq!(csv[0][0], "time");
    assert!(csv[0].iter().any(|name| name == "v(vout)"), "{:?}", csv[0]);
    assert_eq!(
        csv.len() - 1,
        run["analysis"]["points"].as_u64().unwrap() as usize
    );

    let listed = call_again(
        home.path(),
        "list_simulation_runs",
        json!({ "schematic": sheet }),
    );
    assert_eq!(listed["total"], 1, "{listed}");
    let entry = &listed["runs"][0];
    assert_eq!(entry["run_id"], run["run_id"]);
    assert_eq!(entry["analysis_kind"], "transient");
    assert_eq!(entry["run_status"], "complete");
    assert_eq!(entry["simulated_circuit"], "modified");
    assert_eq!(entry["coverage_ratio"], 1.0);
    assert_eq!(entry["inputs_changed_since_run"], false, "{entry}");

    // The same two measurements, and one the run never took, from the stored data.
    let again = call_again(
        home.path(),
        "measure_simulation_run",
        json!({
            "run_id": run["run_id"],
            "signals": ["v(vout)"],
            "measurements": [
                { "name": "inrush_peak", "kind": "peak_abs", "signal": "i(VBAT)" },
                { "name": "rail_low", "kind": "min", "signal": "v(vout)", "from_seconds": 60e-3 },
                { "name": "rail_before", "kind": "value_at", "signal": "v(vout)", "at_seconds": 59.9e-3 },
            ],
        }),
    );
    assert_eq!(again["outcome"]["status"], "complete", "{again}");
    assert_eq!(again["measurements"][0], run["measurements"][0]);
    assert_eq!(again["measurements"][1], run["measurements"][1]);
    assert_eq!(again["signal_summaries"], run["signal_summaries"]);
    // From `soft_start_reference.py`.
    near(measured(&again, "rail_before"), 11.98087, 1e-4);
    near(measured(&again, "inrush_peak"), -16.9040, 2e-3);
    assert_eq!(again["inputs_changed_since_run"], false);
    assert_eq!(run_folders(home.path()), 1, "no new run was made");

    // Saving a different sheet makes the run read as made from other inputs.
    let text = std::fs::read_to_string(&sheet).unwrap();
    std::fs::write(&sheet, format!("{text}\n")).unwrap();
    let after = call_again(
        home.path(),
        "list_simulation_runs",
        json!({ "schematic": sheet }),
    );
    assert_eq!(
        after["runs"][0]["inputs_changed_since_run"], true,
        "{after}"
    );
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn a_stored_ac_loop_gives_the_same_margins_and_a_complex_csv() {
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/rectifier"), project.path());
    let sheet = project.path().join("rectifier.kicad_sch");
    let home = tempfile::tempdir().unwrap();
    let measurements = json!([
        { "name": "pm", "kind": "phase_margin", "signal": "v(out)" },
        { "name": "gm", "kind": "gain_margin", "signal": "v(out)" },
        { "name": "bw", "kind": "bandwidth_3db", "signal": "v(out)" },
        { "name": "at_1k", "kind": "value_at", "signal": "v(out)", "at_hz": 1000.0 },
    ]);
    let run = call_again(
        home.path(),
        "run_simulation",
        json!({
            "schematic": sheet,
            "testbench": { "exclude_references": ["*"], "spice_lines": ac_loop_lines() },
            "analysis": { "kind": "ac", "start_hz": 1.0, "stop_hz": 1e6, "points": 200 },
            "measurements": measurements,
        }),
    );
    assert_eq!(run["outcome"]["status"], "complete", "{run}");
    let again = call_again(
        home.path(),
        "measure_simulation_run",
        json!({ "run_id": run["run_id"], "measurements": measurements }),
    );
    assert_eq!(again["analysis_kind"], "ac");
    assert_eq!(again["measurements"], run["measurements"], "{again}");
    near(measured(&again, "pm"), 3.485942, 2e-3);

    let csv = csv_rows(run["artifacts"]["csv_path"].as_str().unwrap());
    assert_eq!(csv[0][0], "frequency");
    assert!(
        csv[0].iter().any(|name| name == "v(out).real"),
        "{:?}",
        csv[0]
    );
    assert!(
        csv[0].iter().any(|name| name == "v(out).imag"),
        "{:?}",
        csv[0]
    );
    assert_eq!(
        csv.len() - 1,
        run["analysis"]["points"].as_u64().unwrap() as usize
    );
    // The first frequency is the start, and every row has a cell for every column.
    near(csv[1][0].parse().unwrap(), 1.0, 1e-12);
    assert!(csv[1..].iter().all(|row| row.len() == csv[0].len()));
}

#[test]
#[ignore = "needs KiCad 10 with libngspice, and KICAD_DEMOS"]
fn the_csv_of_two_sweeps_has_the_second_value_the_engine_does_not_save() {
    let lines: Vec<String> = [
        "VA a 0 DC 0",
        "VB b 0 DC 0",
        "R1 a c 1k",
        "R2 b c 1k",
        "R3 c 0 1k",
    ]
    .map(str::to_owned)
    .to_vec();
    let project = tempfile::tempdir().unwrap();
    copy_tree(&demos().join("simulation/rectifier"), project.path());
    let sheet = project.path().join("rectifier.kicad_sch");
    let home = tempfile::tempdir().unwrap();
    let analysis = json!({ "kind": "dc_sweep", "sweeps": [
        { "target": "VA", "start_value": 0.0, "stop_value": 2.0, "step_value": 1.0 },
        { "target": "VB", "start_value": 1.0, "stop_value": 4.0, "step_value": 1.0 },
    ] });
    let run = call_again(
        home.path(),
        "run_simulation",
        json!({
            "schematic": sheet,
            "testbench": { "exclude_references": ["*"], "spice_lines": lines },
            "analysis": analysis,
            "signals": ["v(c)"],
        }),
    );
    assert_eq!(run["outcome"]["status"], "complete", "{run}");
    let csv = csv_rows(run["artifacts"]["csv_path"].as_str().unwrap());
    let column = |name: &str| csv[0].iter().position(|cell| cell == name).unwrap();
    let (outer, inner) = (column("VB (V)"), column("VA (V)"));
    assert!(outer < inner, "{:?}", csv[0]);
    assert_eq!(csv.len() - 1, 12);
    // The engine saves v(a) and v(b) as the circuit's own nodes, which the sweeps set,
    // so they check the rebuilt columns against the engine's own.
    let (a, b, c) = (column("v(a)"), column("v(b)"), column("v(c)"));
    for row in &csv[1..] {
        let number = |cell: usize| row[cell].parse::<f64>().unwrap();
        near(number(inner), number(a), 1e-9);
        near(number(outer), number(b), 1e-9);
        near(number(c), (number(a) + number(b)) / 3.0, 1e-9);
    }
    near(csv[1][outer].parse().unwrap(), 1.0, 1e-12);
    near(csv[12][outer].parse().unwrap(), 4.0, 1e-12);

    // A stored run of two sweeps is listed, and takes no measurement.
    let listed = call_again(
        home.path(),
        "list_simulation_runs",
        json!({ "schematic": sheet }),
    );
    assert_eq!(listed["runs"][0]["analysis_kind"], "dc_sweep", "{listed}");
    let refused = call_again(
        home.path(),
        "measure_simulation_run",
        json!({ "run_id": run["run_id"],
                "measurements": [{ "kind": "max", "signal": "v(c)" }] }),
    );
    assert_eq!(refused["error"]["kind"], "invalid_argument", "{refused}");
}
