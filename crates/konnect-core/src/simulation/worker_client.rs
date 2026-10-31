//! Start and supervise the `konnect sim-worker` process.

use super::protocol::{
    HaltReason, ProbeReport, RunStatus, PROTOCOL_VERSION, RAW_FILE, STATUS_FILE, WORKER_SUBCOMMAND,
};
use super::raw::{self, Plot};
use crate::kicad_install;
use anyhow::Context;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;
use tokio::process::Command;

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// After its budget the worker gets this long to stop by itself. It leaves on its
/// own a little sooner, so this only catches a process that is truly stuck.
const KILL_GRACE: Duration = Duration::from_secs(15);
/// The exit code of a worker that left at its own deadline.
const OUT_OF_TIME_EXIT: i32 = 124;
const WORKER_OUTPUT_FILE: &str = "worker.out";
/// Far above any raw file the first analyses write.
const MAX_RAW_BYTES: u64 = 1024 * 1024 * 1024;

/// Simulation settings from the server configuration.
#[derive(Debug, Clone, Default)]
pub struct SimulationSettings {
    /// Explicit libngspice path. Blank means discover it.
    pub ngspice_library: String,
    /// Explicit code-model folder. Blank means look beside the library.
    pub ngspice_code_models_dir: String,
    /// Load `digital.cm`. Deck vetting refuses its two dangerous models.
    pub allow_digital_models: bool,
    /// Extra folders a model include may read from.
    pub model_dirs: Vec<PathBuf>,
}

/// What a tool needs to reach the worker. Set once by the standalone binary,
/// which contains the worker; absent in embedded and test contexts, so those
/// report the engine as unavailable instead of running the host executable.
#[derive(Debug, Clone)]
pub struct SimulationRuntime {
    worker_exe: PathBuf,
    settings: SimulationSettings,
    /// Where runs go, when a test must not use the shared state folder.
    #[cfg(test)]
    pub(crate) state_root: Option<PathBuf>,
}

impl SimulationRuntime {
    pub fn new(worker_exe: PathBuf, settings: SimulationSettings) -> Self {
        Self {
            worker_exe,
            settings,
            #[cfg(test)]
            state_root: None,
        }
    }

    pub(crate) fn state_root(&self) -> Result<PathBuf, konnect_sexp::SexpError> {
        #[cfg(test)]
        if let Some(root) = &self.state_root {
            return Ok(root.clone());
        }
        konnect_sexp::writer::state_root()
    }

    pub(crate) fn model_dirs(&self) -> &[PathBuf] {
        &self.settings.model_dirs
    }

    pub(crate) fn allows_digital_models(&self) -> bool {
        self.settings.allow_digital_models
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

    let mut command = worker_command(runtime, scratch.path());
    command
        .arg("--probe")
        .arg("--library")
        .arg(library)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = code_models {
        command.arg("--code-models").arg(dir);
    }
    if !runtime.settings.allow_digital_models {
        command.arg("--no-digital");
    }
    allow_dll_dependencies(&mut command, library);

    let child = spawn_worker(&mut command)?;
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

/// A command for the worker, working in `dir`. A worker must not hold the MCP
/// stdin, and `SPICE_*` must not change results.
fn worker_command(runtime: &SimulationRuntime, dir: &Path) -> Command {
    let mut command = Command::new(&runtime.worker_exe);
    command
        .arg(WORKER_SUBCOMMAND)
        .args(["--protocol", &PROTOCOL_VERSION.to_string()])
        .current_dir(dir)
        .env_clear()
        .env("SPICE_SCRIPTS", dir)
        .env("SPICE_USERINIT_DIR", dir)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    command
}

/// Windows resolves a DLL's own dependencies through PATH.
#[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
fn allow_dll_dependencies(command: &mut Command, library: &Path) {
    #[cfg(target_os = "windows")]
    if let Some(dir) = library.parent() {
        command.env("PATH", dir);
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
    }
}

fn spawn_worker(command: &mut Command) -> anyhow::Result<tokio::process::Child> {
    #[cfg(test)]
    let child = crate::tools::cli::test_support::spawn_cli(command);
    #[cfg(not(test))]
    let child = command.spawn();
    child.context("Failed to start the simulation worker")
}

/// How the worker process ended, before anything reads what it wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkerEnd {
    Reported(RunStatus),
    OutOfTime,
    Died(String),
}

/// Run the worker on a prepared folder. Dropping this future kills the worker.
pub(crate) async fn run_worker(
    runtime: &SimulationRuntime,
    run_dir: &Path,
    library: &Path,
    budget: Duration,
) -> anyhow::Result<WorkerEnd> {
    std::fs::write(run_dir.join(".spiceinit"), "")
        .context("Failed to prepare the simulation worker's working directory")?;
    let output = std::fs::File::create(run_dir.join(WORKER_OUTPUT_FILE))
        .context("Failed to create the worker's output file")?;
    let errors = output.try_clone()?;
    let mut command = worker_command(runtime, run_dir);
    command
        .arg("--run-dir")
        .arg(run_dir)
        .stdout(Stdio::from(output))
        .stderr(Stdio::from(errors));
    allow_dll_dependencies(&mut command, library);
    let mut child = spawn_worker(&mut command)?;
    let finished = tokio::time::timeout(budget + KILL_GRACE, child.wait()).await;
    let exit = match finished {
        Ok(exit) => exit.context("Failed to wait for the simulation worker")?,
        Err(_) => {
            child.kill().await.ok();
            return Ok(WorkerEnd::OutOfTime);
        }
    };
    Ok(read_end(run_dir, exit))
}

fn read_end(run_dir: &Path, exit: ExitStatus) -> WorkerEnd {
    let report = std::fs::read(run_dir.join(STATUS_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<RunStatus>(&bytes).ok());
    match report {
        Some(status) => WorkerEnd::Reported(status),
        None if exit.code() == Some(OUT_OF_TIME_EXIT) => WorkerEnd::OutOfTime,
        None => WorkerEnd::Died(describe_exit(exit)),
    }
}

fn describe_exit(exit: ExitStatus) -> String {
    #[cfg(unix)]
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&exit) {
        return format!("signal {signal}");
    }
    match exit.code() {
        Some(code) => format!("exit code {code}"),
        None => "no exit code".to_string(),
    }
}

/// What a run came to. Logs explain a result; they never decide it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Complete,
    Partial(&'static str),
    Failed(&'static str),
}

/// The last plot in the raw file, which is the one `write` saved.
pub(crate) fn read_results(run_dir: &Path, status: &RunStatus) -> Result<Plot, raw::RawError> {
    if !status.results_written {
        return Err(raw::RawError::NoResults);
    }
    let mut plots = raw::read(&run_dir.join(RAW_FILE), MAX_RAW_BYTES)?;
    plots.pop().ok_or(raw::RawError::NoResults)
}

/// Decide from the evidence alone. `covered` is the proof that the requested
/// range was simulated; a run that reads success without it is only partial.
pub(crate) fn classify(
    end: &WorkerEnd,
    results: Option<&Result<Plot, raw::RawError>>,
    covered: impl Fn(&Plot) -> bool,
) -> Verdict {
    let status = match end {
        WorkerEnd::OutOfTime => return Verdict::Failed("timed_out"),
        WorkerEnd::Died(_) => return Verdict::Failed("engine_crashed"),
        WorkerEnd::Reported(status) => status,
    };
    if status.protocol != PROTOCOL_VERSION || status.load_error.is_some() {
        return Verdict::Failed("engine_load_failed");
    }
    if status.exit_status.is_some() {
        return Verdict::Failed(if status.run_started {
            "engine_crashed"
        } else {
            "load_failed"
        });
    }
    let Some(sim_status) = status.sim_status else {
        return Verdict::Failed("circuit_not_parsed");
    };
    let plot = results.and_then(|results| results.as_ref().ok());
    let has_points = plot.is_some_and(|plot| plot.points > 0);
    if let Some(reason) = status.halted_reason {
        let HaltReason::Timeout = reason;
        return match has_points {
            true => Verdict::Partial("timeout"),
            false => Verdict::Failed("timed_out"),
        };
    }
    match (sim_status, has_points) {
        (0, _) => match plot {
            Some(plot) if covered(plot) => Verdict::Complete,
            Some(_) => Verdict::Partial("incomplete_coverage"),
            None => Verdict::Failed("results_unreadable"),
        },
        (_, true) => Verdict::Partial("no_convergence"),
        (_, false) => Verdict::Failed("no_convergence"),
    }
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

    fn status(sim_status: Option<i32>) -> RunStatus {
        RunStatus {
            protocol: PROTOCOL_VERSION,
            load_error: None,
            engine_version: Some("44.2".to_string()),
            code_models_loaded: Vec::new(),
            code_models_failed: Vec::new(),
            exit_status: None,
            run_started: true,
            sim_status,
            halted_reason: None,
            results_written: true,
        }
    }

    fn plot(text: &str) -> Result<Plot, raw::RawError> {
        raw::parse(text.as_bytes()).map(|mut plots| plots.remove(0))
    }

    const ONE_POINT: &str = "Title: t\nPlotname: Operating Point\nFlags: real\n\
        No. Variables: 1\nNo. Points: 1\nVariables:\n\t0\tv(a)\tvoltage\nValues:\n 0\t1.0\n\n";
    const NO_POINTS: &str = "Title: t\nPlotname: Operating Point\nFlags: real\n\
        No. Variables: 1\nNo. Points: 1\nVariables:\n\t0\tv(a)\tvoltage\nValues:\n";

    fn verdict(status: RunStatus, results: &Result<Plot, raw::RawError>) -> Verdict {
        classify(&WorkerEnd::Reported(status), Some(results), |_| true)
    }

    #[test]
    fn every_row_of_the_classification_table() {
        let good = plot(ONE_POINT);
        let empty = plot(NO_POINTS);
        let none = Err(raw::RawError::NoResults);
        let halted = RunStatus {
            halted_reason: Some(HaltReason::Timeout),
            ..status(Some(0))
        };

        assert_eq!(verdict(status(Some(0)), &good), Verdict::Complete);
        assert_eq!(
            verdict(status(None), &none),
            Verdict::Failed("circuit_not_parsed")
        );
        assert_eq!(
            verdict(status(Some(1)), &none),
            Verdict::Failed("no_convergence")
        );
        assert_eq!(
            verdict(status(Some(1)), &empty),
            Verdict::Failed("no_convergence")
        );
        assert_eq!(
            verdict(status(Some(1)), &good),
            Verdict::Partial("no_convergence")
        );
        assert_eq!(verdict(halted.clone(), &good), Verdict::Partial("timeout"));
        assert_eq!(verdict(halted.clone(), &none), Verdict::Failed("timed_out"));
        assert_eq!(verdict(halted, &empty), Verdict::Failed("timed_out"));
        assert_eq!(
            verdict(status(Some(0)), &none),
            Verdict::Failed("results_unreadable")
        );
    }

    #[test]
    fn a_run_that_reads_success_without_coverage_is_only_partial() {
        let good = plot(ONE_POINT);
        let end = WorkerEnd::Reported(status(Some(0)));
        assert_eq!(
            classify(&end, Some(&good), |_| false),
            Verdict::Partial("incomplete_coverage")
        );
    }

    #[test]
    fn a_worker_that_gave_no_report_never_reads_as_a_result() {
        let good = plot(ONE_POINT);
        let pass = |end: WorkerEnd| classify(&end, Some(&good), |_| true);
        assert_eq!(
            pass(WorkerEnd::Died("signal 11".to_string())),
            Verdict::Failed("engine_crashed")
        );
        assert_eq!(pass(WorkerEnd::OutOfTime), Verdict::Failed("timed_out"));
    }

    #[test]
    fn a_dead_library_outranks_whatever_else_it_left() {
        let good = plot(ONE_POINT);
        let exited = RunStatus {
            exit_status: Some(1),
            ..status(Some(0))
        };
        assert_eq!(
            verdict(exited.clone(), &good),
            Verdict::Failed("engine_crashed")
        );
        let while_loading = RunStatus {
            run_started: false,
            ..exited
        };
        assert_eq!(
            verdict(while_loading, &good),
            Verdict::Failed("load_failed")
        );
        let unloaded = RunStatus {
            load_error: Some("no library".to_string()),
            ..status(Some(0))
        };
        assert_eq!(
            verdict(unloaded, &good),
            Verdict::Failed("engine_load_failed")
        );
        let other_build = RunStatus {
            protocol: PROTOCOL_VERSION + 1,
            ..status(Some(0))
        };
        assert_eq!(
            verdict(other_build, &good),
            Verdict::Failed("engine_load_failed")
        );
    }

    async fn end_of(body: &str) -> WorkerEnd {
        let dir = tempfile::tempdir().unwrap();
        let tools = tempfile::tempdir().unwrap();
        let worker = write_script(tools.path(), "worker", body, "");
        run_worker(
            &runtime(worker),
            dir.path(),
            Path::new("/lib"),
            Duration::from_secs(5),
        )
        .await
        .unwrap()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_crash_is_reported_with_its_signal() {
        let end = end_of("kill -SEGV $$\n").await;
        assert_eq!(end, WorkerEnd::Died("signal 11".to_string()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_worker_at_its_own_deadline_reads_as_out_of_time() {
        assert_eq!(end_of("exit 124\n").await, WorkerEnd::OutOfTime);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn garbage_in_the_status_file_is_a_crash_not_a_result() {
        let dir = tempfile::tempdir().unwrap();
        let tools = tempfile::tempdir().unwrap();
        let worker = write_script(
            tools.path(),
            "worker",
            "echo '{' > status.json\nexit 3\n",
            "",
        );
        let end = run_worker(
            &runtime(worker),
            dir.path(),
            Path::new("/lib"),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(end, WorkerEnd::Died("exit code 3".to_string()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_report_is_read_and_the_worker_gets_a_clean_room() {
        let dir = tempfile::tempdir().unwrap();
        let tools = tempfile::tempdir().unwrap();
        let body = format!(
            "test -f .spiceinit && test ! -s .spiceinit && test -z \"$HOME\" && \
             test \"$SPICE_SCRIPTS\" = \"$PWD\" || exit 9\ncat > {STATUS_FILE} <<'JSON'\n{}\nJSON\n",
            serde_json::to_string(&status(Some(0))).unwrap()
        );
        let worker = write_script(tools.path(), "worker", &body, "");
        let dir_path = dir.path().canonicalize().unwrap();
        let end = run_worker(
            &runtime(worker),
            &dir_path,
            Path::new("/lib"),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(end, WorkerEnd::Reported(status(Some(0))));
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
