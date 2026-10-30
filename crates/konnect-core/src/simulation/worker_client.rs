//! Start and supervise the `konnect sim-worker` process.

use super::protocol::{ProbeReport, PROTOCOL_VERSION, WORKER_SUBCOMMAND};
use crate::kicad_install;
use anyhow::Context;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Simulation settings from the server configuration.
#[derive(Debug, Clone, Default)]
pub struct SimulationSettings {
    /// Explicit libngspice path. Blank means discover it.
    pub ngspice_library: String,
    /// Explicit code-model folder. Blank means look beside the library.
    pub ngspice_code_models_dir: String,
    /// Load `digital.cm`. Deck vetting refuses its two dangerous models.
    pub allow_digital_models: bool,
}

/// What a tool needs to reach the worker. Set once by the standalone binary,
/// which contains the worker; absent in embedded and test contexts, so those
/// report the engine as unavailable instead of running the host executable.
#[derive(Debug, Clone)]
pub struct SimulationRuntime {
    worker_exe: PathBuf,
    settings: SimulationSettings,
}

impl SimulationRuntime {
    pub fn new(worker_exe: PathBuf, settings: SimulationSettings) -> Self {
        Self {
            worker_exe,
            settings,
        }
    }

    /// The running executable as the worker. Only `main` calls this: in the
    /// plugin library `current_exe()` names the host.
    pub fn for_current_executable(settings: SimulationSettings) -> std::io::Result<Self> {
        Ok(Self::new(
            crate::runtime_info::installed_executable_path()?,
            settings,
        ))
    }
}

/// The engine's state, as observed. `available` is true only after a probe
/// loaded the library, so a file that merely exists cannot read as a pass.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct EngineStatus {
    pub probe_status: &'static str,
    pub available: bool,
    pub version: Option<String>,
    pub library_path: Option<String>,
    pub code_models_dir: Option<String>,
    pub code_model_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub code_models_failed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub searched_locations: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl EngineStatus {
    fn unavailable(probe_status: &'static str) -> Self {
        Self {
            probe_status,
            available: false,
            version: None,
            library_path: None,
            code_models_dir: None,
            code_model_count: 0,
            code_models_failed: Vec::new(),
            searched_locations: Vec::new(),
            detail: None,
        }
    }
}

/// Find libngspice and load it in a worker process. Every status other than
/// `ok` leaves `available` false.
pub(crate) async fn probe_engine(
    runtime: Option<&SimulationRuntime>,
    kicad_cli: &str,
) -> EngineStatus {
    let Some(runtime) = runtime else {
        let mut status = EngineStatus::unavailable("not_available");
        status.detail = Some(
            "This process does not carry the simulation worker (embedded or library use)."
                .to_string(),
        );
        return status;
    };
    let search = kicad_install::find_ngspice_library(&runtime.settings.ngspice_library, kicad_cli);
    let Some(library) = search.library else {
        let mut status = EngineStatus::unavailable("not_found");
        status.searched_locations = search.searched.iter().map(|p| display(p)).collect();
        return status;
    };
    let code_models =
        kicad_install::find_code_models_dir(&runtime.settings.ngspice_code_models_dir, &library);
    probe_library(runtime, &library, code_models.as_deref(), PROBE_TIMEOUT).await
}

async fn probe_library(
    runtime: &SimulationRuntime,
    library: &Path,
    code_models: Option<&Path>,
    limit: Duration,
) -> EngineStatus {
    let mut status = match run_probe(runtime, library, code_models, limit).await {
        Ok(status) => status,
        Err(error) => {
            let mut status = EngineStatus::unavailable("launch_failed");
            status.detail = Some(format!("{error:#}"));
            status
        }
    };
    status.library_path = Some(display(library));
    status.code_models_dir = code_models.map(display);
    status
}

async fn run_probe(
    runtime: &SimulationRuntime,
    library: &Path,
    code_models: Option<&Path>,
    limit: Duration,
) -> anyhow::Result<EngineStatus> {
    // Init files must not change results, and `spinit` could load the digital
    // code models past `--no-digital`. ngspice reads `.spiceinit` from the
    // working directory first, so an empty one here shadows the user's. The
    // calls meant to disable these files crash on ngspice-44.2, so point
    // `spinit` lookup at this empty folder instead.
    let scratch = tempfile::Builder::new()
        .prefix("konnect-sim-probe-")
        .tempdir()
        .context("Failed to create a working directory for the simulation worker")?;
    std::fs::write(scratch.path().join(".spiceinit"), "")
        .context("Failed to prepare the simulation worker's working directory")?;

    let mut command = Command::new(&runtime.worker_exe);
    command
        .arg(WORKER_SUBCOMMAND)
        .args(["--protocol", &PROTOCOL_VERSION.to_string()])
        .arg("--probe")
        .arg("--library")
        .arg(library)
        .current_dir(scratch.path())
        // A worker must not hold the MCP stdin; `SPICE_*` must not change results.
        .env_clear()
        .env("SPICE_SCRIPTS", scratch.path())
        .env("SPICE_USERINIT_DIR", scratch.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = code_models {
        command.arg("--code-models").arg(dir);
    }
    if !runtime.settings.allow_digital_models {
        command.arg("--no-digital");
    }
    // Windows resolves a DLL's own dependencies through PATH.
    #[cfg(target_os = "windows")]
    if let Some(dir) = library.parent() {
        command.env("PATH", dir);
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
    }

    #[cfg(test)]
    let child = crate::tools::cli::test_support::spawn_cli(&mut command)
        .context("Failed to start the simulation worker")?;
    #[cfg(not(test))]
    let child = command
        .spawn()
        .context("Failed to start the simulation worker")?;

    let output = match tokio::time::timeout(limit, child.wait_with_output()).await {
        Ok(output) => output.context("Failed to read the simulation worker's output")?,
        Err(_) => return Ok(EngineStatus::unavailable("timed_out")),
    };
    if !output.status.success() {
        let mut status = EngineStatus::unavailable("nonzero_exit");
        status.detail = Some(diagnostic_tail(&output.stderr));
        return Ok(status);
    }
    Ok(interpret_probe_output(&output.stdout))
}

/// The report is the last stdout line: ngspice may print to stdout itself.
fn interpret_probe_output(stdout: &[u8]) -> EngineStatus {
    let text = String::from_utf8_lossy(stdout);
    let report = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .and_then(|line| serde_json::from_str::<ProbeReport>(line).ok());
    let Some(report) = report else {
        return EngineStatus::unavailable("unrecognized_output");
    };
    if report.protocol != PROTOCOL_VERSION {
        let mut status = EngineStatus::unavailable("protocol_mismatch");
        status.detail = Some(format!(
            "The worker speaks protocol {}, this server speaks {PROTOCOL_VERSION}. Restart the server.",
            report.protocol
        ));
        return status;
    }
    if let Some(error) = report.load_error {
        let mut status = EngineStatus::unavailable("load_failed");
        status.detail = Some(error);
        return status;
    }
    if report.exit_callback {
        let mut status = EngineStatus::unavailable("load_failed");
        status.detail = Some("The library called its exit callback during startup.".to_string());
        return status;
    }
    let Some(version) = report.engine_version else {
        return EngineStatus::unavailable("unrecognized_output");
    };
    EngineStatus {
        probe_status: "ok",
        available: true,
        version: Some(version),
        library_path: None,
        code_models_dir: None,
        code_model_count: report.code_models_loaded.len(),
        code_models_failed: report.code_models_failed,
        searched_locations: Vec::new(),
        detail: None,
    }
}

fn diagnostic_tail(stderr: &[u8]) -> String {
    const LIMIT: usize = 500;
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    let start = text.len().saturating_sub(LIMIT);
    let start = (start..=text.len())
        .find(|index| text.is_char_boundary(*index))
        .unwrap_or(text.len());
    text[start..].to_string()
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::cli::test_support::write_script;

    fn report(protocol: u32) -> ProbeReport {
        ProbeReport {
            protocol,
            load_error: None,
            engine_version: Some("44.2".to_string()),
            code_models_loaded: vec!["analog".to_string(), "digital".to_string()],
            code_models_failed: Vec::new(),
            exit_callback: false,
        }
    }

    fn json_line(report: &ProbeReport) -> Vec<u8> {
        serde_json::to_vec(report).unwrap()
    }

    fn runtime(worker: PathBuf) -> SimulationRuntime {
        SimulationRuntime::new(
            worker,
            SimulationSettings {
                allow_digital_models: true,
                ..Default::default()
            },
        )
    }

    #[test]
    fn a_loaded_library_reads_as_available_with_its_observed_version() {
        let status = interpret_probe_output(&json_line(&report(PROTOCOL_VERSION)));
        assert_eq!(status.probe_status, "ok");
        assert!(status.available);
        assert_eq!(status.version.as_deref(), Some("44.2"));
        assert_eq!(status.code_model_count, 2);
    }

    #[test]
    fn the_report_is_taken_from_the_last_line_after_engine_chatter() {
        let mut stdout = b"chatter from the engine\n\n".to_vec();
        stdout.extend(json_line(&report(PROTOCOL_VERSION)));
        stdout.push(b'\n');
        assert!(interpret_probe_output(&stdout).available);
    }

    #[test]
    fn a_load_error_is_not_available() {
        let mut failed = report(PROTOCOL_VERSION);
        failed.load_error = Some("dlopen failed".to_string());
        let status = interpret_probe_output(&json_line(&failed));
        assert_eq!(status.probe_status, "load_failed");
        assert!(!status.available);
        assert_eq!(status.detail.as_deref(), Some("dlopen failed"));
    }

    #[test]
    fn an_exit_callback_during_startup_is_not_available() {
        let mut failed = report(PROTOCOL_VERSION);
        failed.exit_callback = true;
        let status = interpret_probe_output(&json_line(&failed));
        assert_eq!(status.probe_status, "load_failed");
        assert!(!status.available);
    }

    #[test]
    fn a_library_with_no_recognisable_banner_is_not_available() {
        let mut silent = report(PROTOCOL_VERSION);
        silent.engine_version = None;
        let status = interpret_probe_output(&json_line(&silent));
        assert_eq!(status.probe_status, "unrecognized_output");
        assert!(!status.available);
    }

    #[test]
    fn a_worker_from_another_build_is_refused_by_protocol() {
        let status = interpret_probe_output(&json_line(&report(PROTOCOL_VERSION + 1)));
        assert_eq!(status.probe_status, "protocol_mismatch");
        assert!(!status.available);
    }

    #[test]
    fn garbage_output_is_not_available() {
        for stdout in [&b""[..], b"not json\n", b"{\"protocol\":1}\n"] {
            let status = interpret_probe_output(stdout);
            assert_eq!(status.probe_status, "unrecognized_output");
            assert!(!status.available);
        }
    }

    #[test]
    fn a_diagnostic_tail_never_splits_a_character() {
        let text = "ש".repeat(400);
        assert!(diagnostic_tail(text.as_bytes()).chars().all(|c| c == 'ש'));
    }

    #[tokio::test]
    async fn a_process_without_the_worker_says_so() {
        let status = probe_engine(None, "").await;
        assert_eq!(status.probe_status, "not_available");
        assert!(!status.available);
    }

    #[tokio::test]
    async fn a_missing_configured_library_lists_where_it_looked_and_starts_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let worker = dir.path().join("never-started");
        let missing = dir.path().join("libngspice.so.0");
        let runtime = SimulationRuntime::new(
            worker,
            SimulationSettings {
                ngspice_library: missing.display().to_string(),
                ..Default::default()
            },
        );
        let status = probe_engine(Some(&runtime), "").await;
        assert_eq!(status.probe_status, "not_found");
        assert_eq!(status.searched_locations, vec![display(&missing)]);
    }

    #[tokio::test]
    async fn a_worker_that_cannot_start_reports_launch_failed() {
        let dir = tempfile::tempdir().unwrap();
        let status = probe_library(
            &runtime(dir.path().join("missing-worker")),
            Path::new("/lib"),
            None,
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(status.probe_status, "launch_failed");
        assert!(!status.available);
    }

    #[tokio::test]
    async fn a_crashing_worker_is_reported_and_the_server_carries_on() {
        let dir = tempfile::tempdir().unwrap();
        let worker = write_script(
            dir.path(),
            "crashing-worker",
            "echo 'simulated crash' >&2\nkill -SEGV $$\n",
            "echo simulated crash 1>&2\r\nexit /b 139\r\n",
        );
        let status = probe_library(
            &runtime(worker),
            Path::new("/lib"),
            None,
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(status.probe_status, "nonzero_exit");
        assert!(!status.available);
    }

    #[tokio::test]
    async fn a_hung_worker_times_out_and_is_not_available() {
        let dir = tempfile::tempdir().unwrap();
        let worker = write_script(
            dir.path(),
            "hung-worker",
            "exec /bin/sleep 30\n",
            "ping -n 30 127.0.0.1 >NUL\r\n",
        );
        let status = probe_library(
            &runtime(worker),
            Path::new("/lib"),
            None,
            Duration::from_millis(200),
        )
        .await;
        assert_eq!(status.probe_status, "timed_out");
        assert!(!status.available);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_worker_gets_null_stdin_a_clear_environment_and_a_blank_spiceinit() {
        let dir = tempfile::tempdir().unwrap();
        let seen = dir.path().join("seen.txt");
        let good = String::from_utf8(json_line(&report(PROTOCOL_VERSION))).unwrap();
        let body = format!(
            "{{ if read -r line; then echo stdin-open; else echo stdin-null; fi; \
             test -f .spiceinit && test ! -s .spiceinit && echo spiceinit-blank; \
             test ! -e \"$SPICE_SCRIPTS/spinit\" && echo spinit-absent; \
             /usr/bin/env; }} > '{}'\necho '{good}'\n",
            seen.display()
        );
        let worker = write_script(dir.path(), "recording-worker", &body, "");
        std::env::set_var("SPICE_LIB_DIR", "/must/not/reach/the/worker");
        let status = probe_library(
            &runtime(worker),
            Path::new("/lib"),
            None,
            Duration::from_secs(5),
        )
        .await;
        std::env::remove_var("SPICE_LIB_DIR");
        assert!(status.available, "{status:?}");
        let seen = std::fs::read_to_string(seen).unwrap();
        for expected in ["stdin-null", "spiceinit-blank", "spinit-absent"] {
            assert!(seen.contains(expected), "{seen}");
        }
        assert!(!seen.contains("SPICE_LIB_DIR"), "{seen}");
    }
}
