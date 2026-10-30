//! The hidden `konnect sim-worker` subcommand.
//!
//! libngspice can segfault, call `exit`, or hang, so it never loads inside the
//! MCP server. This process loads it, does one job, and exits. All FFI lives
//! here; `konnect-core` stays free of `unsafe`.

use anyhow::{bail, Context, Result};
use konnect_core::simulation::protocol::{ProbeReport, PROTOCOL_VERSION};
use libloading::Library;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// `digital.cm` holds the only code models that can start programs or load
/// libraries. Operators can leave all of it out.
const DIGITAL_CODE_MODEL: &str = "digital";

/// `ngspice.h` declares its callback flags as C `bool`.
type SendChar = unsafe extern "C" fn(*mut c_char, c_int, *mut c_void) -> c_int;
type SendStat = unsafe extern "C" fn(*mut c_char, c_int, *mut c_void) -> c_int;
type ControlledExit = unsafe extern "C" fn(c_int, bool, bool, c_int, *mut c_void) -> c_int;
type NgSpiceInit = unsafe extern "C" fn(
    Option<SendChar>,
    Option<SendStat>,
    Option<ControlledExit>,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> c_int;
type NgSpiceCommand = unsafe extern "C" fn(*const c_char) -> c_int;

static ENGINE_LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static EXIT_CALLED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn on_send_char(text: *mut c_char, _id: c_int, _user: *mut c_void) -> c_int {
    if !text.is_null() {
        // SAFETY: ngspice passes a NUL-terminated string valid for this call.
        let line = unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned();
        if let Ok(mut lines) = ENGINE_LINES.lock() {
            lines.push(line);
        }
    }
    0
}

unsafe extern "C" fn on_send_stat(_text: *mut c_char, _id: c_int, _user: *mut c_void) -> c_int {
    0
}

unsafe extern "C" fn on_controlled_exit(
    _status: c_int,
    _immediate: bool,
    _quit: bool,
    _id: c_int,
    _user: *mut c_void,
) -> c_int {
    EXIT_CALLED.store(true, Ordering::SeqCst);
    0
}

#[derive(Debug, PartialEq, Eq)]
struct Request {
    protocol: u32,
    library: PathBuf,
    code_models: Option<PathBuf>,
    load_digital: bool,
}

fn parse_args(args: &[String]) -> Result<Request> {
    let mut protocol = None;
    let mut library = None;
    let mut code_models = None;
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
            other => bail!("unknown sim-worker argument: {other}"),
        }
    }
    if !probe {
        bail!("sim-worker needs a job; only --probe exists in this build");
    }
    Ok(Request {
        protocol: protocol.context("--protocol is required")?,
        library: library.context("--library is required")?,
        code_models,
        load_digital,
    })
}

pub fn run(args: &[String]) -> Result<()> {
    let request = parse_args(args)?;
    let report = if request.protocol == PROTOCOL_VERSION {
        probe(&request)
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

struct Engine {
    command: NgSpiceCommand,
    // Keeps `command` valid. Never dropped: see `run`.
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
        let status = init(
            Some(on_send_char),
            Some(on_send_stat),
            Some(on_controlled_exit),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        if status != 0 {
            bail!("ngSpice_Init returned {status}");
        }
        Ok(Engine {
            command,
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
        let request =
            parse_args(&args(&["--protocol", "1", "--probe", "--library", "/l"])).unwrap();
        assert_eq!(
            request,
            Request {
                protocol: 1,
                library: PathBuf::from("/l"),
                code_models: None,
                load_digital: true,
            }
        );
    }

    #[test]
    fn digital_models_can_be_left_out() {
        let request = parse_args(&args(&[
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
