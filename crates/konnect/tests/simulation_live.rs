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
use std::path::Path;
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

fn real_library() -> std::path::PathBuf {
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
