//! The hidden `konnect sim-worker` subcommand.
//!
//! libngspice can segfault, call `exit`, or hang, so it never loads inside the
//! MCP server. This process loads it, does one job, and exits. All FFI lives
//! here; `konnect-core` stays free of `unsafe`.

use anyhow::{bail, Context, Result};
use konnect_core::simulation::protocol::{
    HaltReason, ProbeReport, RunRequest, RunStatus, DECK_FILE, LOG_FILE, PROTOCOL_VERSION,
    RAW_FILE, REQUEST_FILE, STATUS_FILE,
};
use libloading::Library;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// `digital.cm` holds the only code models that can start programs or load
/// libraries. Operators can leave all of it out.
const DIGITAL_CODE_MODEL: &str = "digital";

/// `ngspice.h` declares its callback flags as C `bool`.
type SendChar = unsafe extern "C" fn(*mut c_char, c_int, *mut c_void) -> c_int;
type SendStat = unsafe extern "C" fn(*mut c_char, c_int, *mut c_void) -> c_int;
type ControlledExit = unsafe extern "C" fn(c_int, bool, bool, c_int, *mut c_void) -> c_int;
type BackgroundRunning = unsafe extern "C" fn(bool, c_int, *mut c_void) -> c_int;
type NgSpiceInit = unsafe extern "C" fn(
    Option<SendChar>,
    Option<SendStat>,
    Option<ControlledExit>,
    *mut c_void,
    *mut c_void,
    Option<BackgroundRunning>,
    *mut c_void,
) -> c_int;
type NgSpiceCommand = unsafe extern "C" fn(*const c_char) -> c_int;
type NgSpiceCirc = unsafe extern "C" fn(*mut *mut c_char) -> c_int;
type NgCmInputPath = unsafe extern "C" fn(*const c_char) -> *mut c_char;

/// How long a stopped run gets to finish writing before the worker leaves. The
/// server kills a little later still, in case this process is stuck.
const WORKER_GRACE: Duration = Duration::from_secs(10);
/// Exit code for a worker that left at its own deadline.
const OUT_OF_TIME: i32 = 124;
const STATUS_MARKER: &str = "KONNECT_SIM_STATUS=";
const POLL: Duration = Duration::from_millis(5);
const HALT_REPEAT: Duration = Duration::from_millis(250);
/// A chatty deck must not fill the disk.
const MAX_LOG_BYTES: usize = 4 * 1024 * 1024;
/// Lines kept in memory, which only need to reach the banner and code models.
const MAX_ENGINE_LINES: usize = 10_000;

static ENGINE_LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static EXIT_CALLED: AtomicBool = AtomicBool::new(false);
static EXIT_STATUS: AtomicI32 = AtomicI32::new(0);
static RUN_FINISHED: AtomicBool = AtomicBool::new(false);
static LOG: Mutex<Option<Log>> = Mutex::new(None);
static STATUS_LINE: Mutex<Option<String>> = Mutex::new(None);

/// The engine's words, on disk as they arrive, so a crash leaves them behind.
struct Log {
    file: File,
    written: usize,
}

unsafe extern "C" fn on_send_char(text: *mut c_char, _id: c_int, _user: *mut c_void) -> c_int {
    if !text.is_null() {
        // SAFETY: ngspice passes a NUL-terminated string valid for this call.
        let line = unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned();
        record_line(line);
    }
    0
}

fn record_line(line: String) {
    if line.contains(STATUS_MARKER) {
        if let Ok(mut slot) = STATUS_LINE.lock() {
            *slot = Some(line.clone());
        }
    }
    if let Ok(mut log) = LOG.lock() {
        if let Some(log) = log.as_mut().filter(|log| log.written < MAX_LOG_BYTES) {
            log.written += line.len() + 1;
            let _ = writeln!(log.file, "{line}");
        }
    }
    if let Ok(mut lines) = ENGINE_LINES.lock() {
        if lines.len() < MAX_ENGINE_LINES {
            lines.push(line);
        }
    }
}

unsafe extern "C" fn on_send_stat(_text: *mut c_char, _id: c_int, _user: *mut c_void) -> c_int {
    0
}

unsafe extern "C" fn on_controlled_exit(
    status: c_int,
    _immediate: bool,
    _quit: bool,
    _id: c_int,
    _user: *mut c_void,
) -> c_int {
    EXIT_STATUS.store(status, Ordering::SeqCst);
    EXIT_CALLED.store(true, Ordering::SeqCst);
    0
}

unsafe extern "C" fn on_background_running(
    not_running: bool,
    _id: c_int,
    _user: *mut c_void,
) -> c_int {
    if not_running {
        RUN_FINISHED.store(true, Ordering::SeqCst);
    }
    0
}

#[derive(Debug, PartialEq, Eq)]
struct Request {
    protocol: u32,
    library: PathBuf,
    code_models: Option<PathBuf>,
    load_digital: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Job {
    Probe(Request),
    Run { protocol: u32, run_dir: PathBuf },
}

fn parse_args(args: &[String]) -> Result<Job> {
    let mut protocol = None;
    let mut library = None;
    let mut code_models = None;
    let mut run_dir = None;
    let mut probe = false;
    let mut load_digital = true;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .with_context(|| format!("{arg} needs a value"))
                .cloned()
        };
        match arg.as_str() {
            "--protocol" => protocol = Some(value()?.parse::<u32>().context("--protocol")?),
            "--library" => library = Some(PathBuf::from(value()?)),
            "--code-models" => code_models = Some(PathBuf::from(value()?)),
            "--probe" => probe = true,
            "--no-digital" => load_digital = false,
            "--run-dir" => run_dir = Some(PathBuf::from(value()?)),
            other => bail!("unknown sim-worker argument: {other}"),
        }
    }
    let protocol = protocol.context("--protocol is required")?;
    match (probe, run_dir) {
        (true, None) => Ok(Job::Probe(Request {
            protocol,
            library: library.context("--library is required")?,
            code_models,
            load_digital,
        })),
        (false, Some(run_dir)) => Ok(Job::Run { protocol, run_dir }),
        _ => bail!("sim-worker needs one job: --probe or --run-dir"),
    }
}

pub fn run(args: &[String]) -> Result<()> {
    match parse_args(args)? {
        Job::Probe(request) => run_probe(&request),
        Job::Run { protocol, run_dir } => {
            let status = run_job(protocol, &run_dir);
            write_status(&run_dir, &status)?;
            // Skip destructors: see `run_probe`.
            std::process::exit(0);
        }
    }
}

fn run_probe(request: &Request) -> Result<()> {
    let report = if request.protocol == PROTOCOL_VERSION {
        probe(request)
    } else {
        empty_report(Some(format!(
            "worker speaks protocol {PROTOCOL_VERSION}, caller asked for {}",
            request.protocol
        )))
    };
    println!("{}", serde_json::to_string(&report)?);
    // Skip destructors: the library may hold threads and has no safe unload.
    std::process::exit(0);
}

fn empty_report(load_error: Option<String>) -> ProbeReport {
    ProbeReport {
        protocol: PROTOCOL_VERSION,
        load_error,
        engine_version: None,
        code_models_loaded: Vec::new(),
        code_models_failed: Vec::new(),
        exit_callback: false,
    }
}

fn probe(request: &Request) -> ProbeReport {
    match load_and_init(&request.library) {
        Ok(engine) => {
            let (loaded, failed) = match &request.code_models {
                Some(dir) => engine.load_code_models(dir, request.load_digital),
                None => (Vec::new(), Vec::new()),
            };
            let lines = engine_lines();
            ProbeReport {
                engine_version: parse_engine_version(&lines),
                code_models_loaded: loaded,
                code_models_failed: failed,
                exit_callback: EXIT_CALLED.load(Ordering::SeqCst),
                ..empty_report(None)
            }
        }
        Err(error) => empty_report(Some(format!("{error:#}"))),
    }
}

fn empty_status(load_error: Option<String>) -> RunStatus {
    RunStatus {
        protocol: PROTOCOL_VERSION,
        load_error,
        engine_version: None,
        code_models_loaded: Vec::new(),
        code_models_failed: Vec::new(),
        exit_status: None,
        sim_status: None,
        halted_reason: None,
        results_written: false,
    }
}

/// By rename, so the server never reads half a report.
fn write_status(run_dir: &Path, status: &RunStatus) -> Result<()> {
    let partial = run_dir.join(format!("{STATUS_FILE}.partial"));
    std::fs::write(&partial, serde_json::to_vec(status)?)?;
    std::fs::rename(&partial, run_dir.join(STATUS_FILE))?;
    Ok(())
}

fn run_job(protocol: u32, run_dir: &Path) -> RunStatus {
    if protocol != PROTOCOL_VERSION {
        return empty_status(Some(format!(
            "worker speaks protocol {PROTOCOL_VERSION}, caller asked for {protocol}"
        )));
    }
    match prepare_and_simulate(run_dir) {
        Ok(status) => status,
        Err(error) => empty_status(Some(format!("{error:#}"))),
    }
}

/// Leave without a report if the engine never comes back. The server reads a
/// missing report with this exit code as out of time.
fn start_deadline(limit: Duration) -> Result<()> {
    std::thread::Builder::new()
        .name("sim-deadline".to_string())
        .spawn(move || {
            std::thread::sleep(limit);
            std::process::exit(OUT_OF_TIME);
        })
        .context("Failed to start the deadline thread")?;
    Ok(())
}

fn prepare_and_simulate(run_dir: &Path) -> Result<RunStatus> {
    let started = Instant::now();
    let request: RunRequest = serde_json::from_slice(
        &std::fs::read(run_dir.join(REQUEST_FILE)).context("Failed to read the run request")?,
    )
    .context("Failed to parse the run request")?;
    let budget = Duration::from_millis(request.budget_ms);
    start_deadline(budget + WORKER_GRACE)?;
    let deck =
        std::fs::read_to_string(run_dir.join(DECK_FILE)).context("Failed to read the deck")?;
    let lines = deck_lines(&deck)?;
    let file = File::create(run_dir.join(LOG_FILE)).context("Failed to create the engine log")?;
    if let Ok(mut log) = LOG.lock() {
        *log = Some(Log { file, written: 0 });
    }

    let engine = load_and_init(&request.library)?;
    let (loaded, failed) = match &request.code_models {
        Some(dir) => engine.load_code_models(dir, request.load_digital),
        None => (Vec::new(), Vec::new()),
    };
    let mut status = RunStatus {
        engine_version: parse_engine_version(&engine_lines()),
        code_models_loaded: loaded,
        code_models_failed: failed,
        ..empty_status(None)
    };
    if let Some(path) = &request.input_path {
        engine.set_input_path(path)?;
    }
    if let Some(behavior) = &request.compatibility {
        engine.run_command(&format!("set ngbehavior={behavior}"))?;
    }
    if !EXIT_CALLED.load(Ordering::SeqCst) {
        drop_process_rights()?;
        engine.load_circuit(lines);
    }
    if !EXIT_CALLED.load(Ordering::SeqCst) {
        engine.run_command("bg_run")?;
        status.halted_reason = engine.wait_for_run(started + budget);
    }
    if EXIT_CALLED.load(Ordering::SeqCst) {
        status.exit_status = Some(EXIT_STATUS.load(Ordering::SeqCst));
        return Ok(status);
    }
    status.sim_status = engine.sim_status();
    if status.sim_status.is_some() {
        // Relative, because this process works in the run folder: ngspice expands
        // `$`, `{` and backticks in a command, and the folder name is not ours.
        engine.run_command(&format!("write {RAW_FILE}"))?;
        status.results_written = run_dir.join(RAW_FILE).is_file();
    }
    if EXIT_CALLED.load(Ordering::SeqCst) {
        status.exit_status = Some(EXIT_STATUS.load(Ordering::SeqCst));
    }
    Ok(status)
}

fn deck_lines(deck: &str) -> Result<Vec<CString>> {
    deck.lines()
        .map(|line| CString::new(line).context("the deck contains a NUL"))
        .collect()
}

/// A second barrier behind deck vetting: a process with no right to children
/// cannot run `shell`, while `bg_run` still gets its thread. macOS counts only
/// processes here. On Linux threads count too, so `bg_run` would fail.
#[cfg(target_os = "macos")]
fn drop_process_rights() -> Result<()> {
    let none = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `none` is a valid rlimit for this call to read.
    if unsafe { libc::setrlimit(libc::RLIMIT_NPROC, &none) } != 0 {
        bail!(
            "Failed to drop process rights: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn drop_process_rights() -> Result<()> {
    Ok(())
}

struct Engine {
    command: NgSpiceCommand,
    circuit: NgSpiceCirc,
    input_path: Option<NgCmInputPath>,
    // Keeps the functions valid. Never dropped: see `run_probe`.
    _library: Library,
}

fn load_and_init(path: &Path) -> Result<Engine> {
    // SAFETY: the path names libngspice as found by discovery or set by the
    // operator; loading it runs its initialisers, which is the job of this
    // disposable process. The symbol types match `sharedspice.h`.
    unsafe {
        let library =
            Library::new(path).with_context(|| format!("Failed to load {}", path.display()))?;
        // No `ngSpice_nospinit` or `ngSpice_nospiceinit`: both segfault if called
        // before init on ngspice-44.2. The parent neutralises those files instead.
        let init = *library
            .get::<NgSpiceInit>(b"ngSpice_Init\0")
            .context("ngSpice_Init is missing")?;
        let command = *library
            .get::<NgSpiceCommand>(b"ngSpice_Command\0")
            .context("ngSpice_Command is missing")?;
        let circuit = *library
            .get::<NgSpiceCirc>(b"ngSpice_Circ\0")
            .context("ngSpice_Circ is missing")?;
        let input_path = library
            .get::<NgCmInputPath>(b"ngCM_Input_Path\0")
            .ok()
            .map(|symbol| *symbol);
        let status = init(
            Some(on_send_char),
            Some(on_send_stat),
            Some(on_controlled_exit),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            Some(on_background_running),
            std::ptr::null_mut(),
        );
        if status != 0 {
            bail!("ngSpice_Init returned {status}");
        }
        Ok(Engine {
            command,
            circuit,
            input_path,
            _library: library,
        })
    }
}

impl Engine {
    fn run_command(&self, command: &str) -> Result<()> {
        let command = CString::new(command).context("command contains a NUL")?;
        // SAFETY: `command` is a valid C string that outlives this call, and
        // `_library` keeps the function loaded.
        unsafe { (self.command)(command.as_ptr()) };
        Ok(())
    }

    fn set_input_path(&self, path: &Path) -> Result<()> {
        let Some(set) = self.input_path else {
            return Ok(());
        };
        let path = CString::new(path.to_string_lossy().as_bytes()).context("path has a NUL")?;
        // SAFETY: `path` outlives the call, which copies it.
        unsafe { set(path.as_ptr()) };
        Ok(())
    }

    /// The array holds pointers into `lines`, which ngspice copies. The status it
    /// returns proves nothing, so the caller reads `$sim_status` instead.
    fn load_circuit(&self, lines: Vec<CString>) {
        let mut pointers: Vec<*mut c_char> =
            lines.iter().map(|line| line.as_ptr().cast_mut()).collect();
        pointers.push(std::ptr::null_mut());
        // SAFETY: a NULL-terminated array of valid C strings that outlive the call.
        unsafe { (self.circuit)(pointers.as_mut_ptr()) };
    }

    /// Wait for the run, halting it softly at the deadline. A halt sent before
    /// the engine's thread starts is lost, so it repeats. `bg_halt` is cooperative,
    /// so a run that ignores it ends at the worker's own deadline.
    fn wait_for_run(&self, deadline: Instant) -> Option<HaltReason> {
        let mut halted = None;
        let mut next_halt = deadline;
        while !RUN_FINISHED.load(Ordering::SeqCst) && !EXIT_CALLED.load(Ordering::SeqCst) {
            if Instant::now() >= next_halt {
                halted = Some(HaltReason::Timeout);
                next_halt = Instant::now() + HALT_REPEAT;
                let _ = self.run_command("bg_halt");
            }
            std::thread::sleep(POLL);
        }
        halted
    }

    /// The library has no getter for this, so have the engine print it. An
    /// undefined variable comes back as its own name, which reads as none.
    fn sim_status(&self) -> Option<i32> {
        *STATUS_LINE.lock().ok()? = None;
        self.run_command(&format!("echo {STATUS_MARKER}$sim_status"))
            .ok()?;
        let line = STATUS_LINE.lock().ok()?.clone()?;
        parse_sim_status(&line)
    }

    /// `ngSpice_Command` returns 0 whether or not the command worked, so a code
    /// model counts as failed when the engine printed to stderr while loading.
    fn load_code_models(&self, dir: &Path, load_digital: bool) -> (Vec<String>, Vec<String>) {
        let (mut loaded, mut failed) = (Vec::new(), Vec::new());
        for path in code_model_files(dir) {
            let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            if stem == DIGITAL_CODE_MODEL && !load_digital {
                continue;
            }
            let before = engine_lines().len();
            let worked = path
                .to_str()
                .filter(|path| !path.contains('\''))
                .is_some_and(|path| self.run_command(&format!("codemodel '{path}'")).is_ok())
                && !engine_lines()[before..]
                    .iter()
                    .any(|line| line.starts_with("stderr"));
            if worked {
                loaded.push(stem);
            } else {
                failed.push(stem);
            }
        }
        (loaded, failed)
    }
}

fn parse_sim_status(line: &str) -> Option<i32> {
    let (_, value) = line.split_once(STATUS_MARKER)?;
    value.trim().parse().ok()
}

fn engine_lines() -> Vec<String> {
    ENGINE_LINES
        .lock()
        .map(|lines| lines.clone())
        .unwrap_or_default()
}

/// `ivlng` and other helpers in the same folder are not code models.
fn code_model_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "cm"))
        .collect();
    files.sort();
    files
}

/// KiCad's version metadata can differ from the library it loads, so the banner
/// is the only evidence: `stdout ** ngspice-44.2 shared library`.
fn parse_engine_version(lines: &[String]) -> Option<String> {
    lines.iter().find_map(|line| {
        let (_, rest) = line.split_once("ngspice-")?;
        let version: String = rest
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != ':')
            .collect();
        (!version.is_empty()).then_some(version)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn a_probe_request_parses_with_defaults() {
        let job = parse_args(&args(&["--protocol", "1", "--probe", "--library", "/l"])).unwrap();
        assert_eq!(
            job,
            Job::Probe(Request {
                protocol: 1,
                library: PathBuf::from("/l"),
                code_models: None,
                load_digital: true,
            })
        );
    }

    #[test]
    fn digital_models_can_be_left_out() {
        let job = parse_args(&args(&[
            "--protocol",
            "1",
            "--probe",
            "--library",
            "/l",
            "--code-models",
            "/c",
            "--no-digital",
        ]))
        .unwrap();
        let Job::Probe(request) = job else {
            panic!("expected a probe")
        };
        assert!(!request.load_digital);
        assert_eq!(request.code_models, Some(PathBuf::from("/c")));
    }

    #[test]
    fn a_request_without_a_job_or_library_is_refused() {
        assert!(parse_args(&args(&["--protocol", "1", "--library", "/l"])).is_err());
        assert!(parse_args(&args(&["--protocol", "1", "--probe"])).is_err());
        assert!(parse_args(&args(&["--probe", "--library", "/l"])).is_err());
        assert!(parse_args(&args(&["--protocol", "1", "--probe", "--library"])).is_err());
        assert!(parse_args(&args(&["--bogus"])).is_err());
    }

    #[test]
    fn a_run_names_its_folder_and_is_not_also_a_probe() {
        let job = parse_args(&args(&["--protocol", "2", "--run-dir", "/r"])).unwrap();
        assert_eq!(
            job,
            Job::Run {
                protocol: 2,
                run_dir: PathBuf::from("/r")
            }
        );
        let both = [
            "--protocol",
            "2",
            "--run-dir",
            "/r",
            "--probe",
            "--library",
            "/l",
        ];
        assert!(parse_args(&args(&both)).is_err());
    }

    #[test]
    fn sim_status_reads_a_number_and_nothing_else() {
        let line = |value: &str| format!("stdout {STATUS_MARKER}{value}");
        assert_eq!(parse_sim_status(&line("0")), Some(0));
        assert_eq!(parse_sim_status(&line("1")), Some(1));
        assert_eq!(parse_sim_status(&line("$sim_status")), None);
        assert_eq!(parse_sim_status(&line("")), None);
        assert_eq!(parse_sim_status("stdout nothing"), None);
    }

    #[test]
    fn the_version_comes_from_the_banner() {
        let lines = args(&[
            "stdout ******",
            "stdout ** ngspice-44.2 shared library",
            "stdout ** ngspice-99 : ignored",
        ]);
        assert_eq!(parse_engine_version(&lines).as_deref(), Some("44.2"));
        assert_eq!(
            parse_engine_version(&args(&["stdout ** ngspice-47 : Circuit level"])).as_deref(),
            Some("47")
        );
    }

    #[test]
    fn no_banner_means_no_version() {
        assert_eq!(parse_engine_version(&args(&["stdout ******"])), None);
        assert_eq!(parse_engine_version(&args(&["stdout ngspice- x"])), None);
    }

    #[test]
    fn only_cm_files_count_as_code_models() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["b.cm", "a.cm", "ivlng.so", "ivlng.vpi"] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        let names: Vec<_> = code_model_files(dir.path())
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.cm", "b.cm"]);
    }
}
