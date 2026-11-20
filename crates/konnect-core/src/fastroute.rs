//! fastroute, a Rust port of Freerouting, run as a separate executable: a command
//! line, a DSN in, a session and a JSON report out. It is a process, not a library,
//! so that a hung run can be killed, a run can be sandboxed off the network, and
//! parallel runs keep their memory apart (docs/ROUTE_SEARCH_DESIGN.md, decision 2).
//!
//! The build Konnect runs is the candidate the owner chose on 2026-11-16
//! (C** + BR1 + W8P, build x18). [`locate`] finds it and [`probe`] proves that the
//! binary found is that build before anything is routed.

use crate::specctra::RoutingHints;
use crate::tools::integration::WidthFallback;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::time::Instant;

/// The one program that may start a run: macOS's own sandbox launcher.
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// The only `PATH` a run sees.
const RUN_PATH: &str = "/usr/bin:/bin";

/// How much of a log or an output a reason quotes.
const QUOTE_CHARS: usize = 300;

/// The candidate's own switches, sent on every pass in this order.
pub(crate) const CANDIDATE_SWITCHES: &[&str] = &[
    "--no-time-limits",
    "--x.no_clock",
    "--x.par_vias=340",
    "--x.work_cap=2000000",
    "--x.par_vias_noacute",
    "--x.bridge_full_width",
    "--x.par_vias_net",
    "--x.par_vias_strict",
    "--x.par_via_each_a=0.8975",
];

/// The candidate's name, as the research and the owner call it.
pub(crate) const CANDIDATE_NAME: &str = "C** + BR1 + W8P";

/// Threads per run, for the autorouter and the optimizer each (the research's 7 x 2).
pub(crate) const THREADS: u32 = 2;

/// The default place of the binary, under `HOME` (decision 2).
pub(crate) const DEFAULT_PATH: &str = ".local/share/fastroute/candidate/fastroute";

/// The file name of a binary packaged next to the Konnect executable (1 Kopper:
/// `<prefix>/bin/konnect` beside `<prefix>/bin/fastroute`).
pub(crate) const BUNDLED_NAME: &str = "fastroute";

/// The manifest a 1 Kopper package holds at `<prefix>/`, one folder above the
/// executables (`cargo xtask kopper-package` writes it). Only an executable inside
/// such a package looks for a bundled binary.
pub(crate) const PACKAGE_MANIFEST: &str = "1-kopper.json";

/// The environment variable that names the binary.
pub(crate) const PATH_VARIABLE: &str = "KONNECT_FASTROUTE";

/// A switch no build knows. A candidate build names it as unknown only after it has
/// accepted every candidate switch given before it on the same command line.
pub(crate) const SENTINEL_SWITCH: &str = "--x.konnect_probe_unknown";

/// `route_specctra_dsn`'s measured preset as the research gave it to fastroute
/// (`lab2.py` `PRESET`), without the per-board values (the retry width and the
/// copper-to-edge clearance), which come from the manifest, and without the
/// optimizer switch, which each pass sets.
pub(crate) const PRESET: &[&str] = &[
    "--router.autorouter.max_passes=20",
    "--router.optimizer.improvement_threshold=0.0025",
    "--router.optimizer.max_consecutive_failures=1000000",
    "--router.optimizer.max_consecutive_failures_pass1=1000000",
    "--router.scoring.via_costs=10",
    "--router.scoring.default_undesired_direction_trace_cost=1.0",
    "--router.fanout.enabled=false",
    "--router.automatic_neckdown=false",
    "--router.job_timeout=00:10:00",
    "--router.optimizer.timeout=00:10:00",
];

/// How long one probe step may take before it is killed.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// The binary Konnect will run, proven to be the candidate build.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Binary {
    /// Canonical path.
    pub path: PathBuf,
    pub sha256: String,
    /// The first line `--version` printed, such as `fastroute 0.1.14`.
    pub version: String,
    /// What named it: `argument`, `environment`, `user_config`, `bundled` or `default`.
    pub source: &'static str,
}

/// What [`locate`] found.
#[derive(Debug)]
pub(crate) enum Located {
    Found(Binary),
    /// Nothing named a binary and none is at the default place.
    NotConfigured {
        default_path: PathBuf,
    },
    /// A path was named (or the default place holds a file) but it is not a regular
    /// executable file, or it is not the candidate build.
    Refused {
        path: PathBuf,
        source: &'static str,
        reason: String,
    },
    /// This system cannot isolate a run (macOS only for now, decision 13).
    Unsupported {
        reason: String,
    },
}

/// The binary to run: the first of `argument`, the [`PATH_VARIABLE`] environment
/// variable (`environment` is its value, passed in so tests do not touch the
/// process environment), `config` (the user config's `routing.fastroute_path`),
/// `bundled` (a [`BUNDLED_NAME`] file next to the Konnect executable, see
/// [`bundled_path`]) and `$HOME/` + [`DEFAULT_PATH`] (`home`). `bundled` and the
/// default place count only when something is there. The first source that names a
/// path decides: a later one is never tried after an earlier one is refused, so a
/// broken packaged binary is reported, never passed over for another build. No other
/// folder is searched and nothing is downloaded or built.
///
/// Refuses with `Unsupported` off macOS and when `/usr/bin/sandbox-exec` is not an
/// executable file. A named path that is not a regular executable file is `Refused`.
/// Then [`probe`] in `scratch` (a fresh folder the caller owns); a probe failure is
/// `Refused` with its reason.
pub(crate) async fn locate(
    argument: Option<&Path>,
    environment: Option<&str>,
    config: Option<&Path>,
    bundled: Option<&Path>,
    home: Option<&Path>,
    scratch: &Path,
) -> Located {
    if !cfg!(target_os = "macos") {
        return Located::Unsupported {
            reason: "fastroute runs only on macOS for now (it is isolated with sandbox-exec); use engine \"freerouting\" on this system".to_string(),
        };
    }
    if let Err(reason) = regular_executable(Path::new(SANDBOX_EXEC)) {
        return Located::Unsupported {
            reason: format!("{SANDBOX_EXEC} is not usable, so a run cannot be isolated: {reason}"),
        };
    }
    let default_path = home.map_or_else(
        || Path::new("~").join(DEFAULT_PATH),
        |home| home.join(DEFAULT_PATH),
    );
    let (path, source): (PathBuf, &'static str) = if let Some(argument) = argument {
        (argument.to_path_buf(), "argument")
    } else if let Some(environment) = environment.filter(|value| !value.is_empty()) {
        (PathBuf::from(environment), "environment")
    } else if let Some(config) = config {
        (config.to_path_buf(), "user_config")
    } else if let Some(bundled) = bundled.filter(|bundled| present(bundled)) {
        (bundled.to_path_buf(), "bundled")
    } else {
        match std::fs::metadata(&default_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Located::NotConfigured { default_path };
            }
            _ => (default_path.clone(), "default"),
        }
    };
    if let Err(reason) = regular_executable(&path) {
        return Located::Refused {
            path,
            source,
            reason,
        };
    }
    let canonical = match path.canonicalize() {
        Ok(canonical) => canonical,
        Err(error) => {
            return Located::Refused {
                path,
                source,
                reason: format!("its path could not be resolved: {error}"),
            }
        }
    };
    match probe(&canonical, source, scratch).await {
        Ok(binary) => Located::Found(binary),
        Err(error) => Located::Refused {
            path: canonical,
            source,
            reason: format!("{error:#}"),
        },
    }
}

/// Where a packaged binary would be for the Konnect executable `exe`: a
/// [`BUNDLED_NAME`] file in the folder of `exe`'s resolved path, so a link to the
/// executable finds the package it points into. `None` when `exe` cannot be resolved
/// or is not inside a 1 Kopper package (no [`PACKAGE_MANIFEST`] one folder up), so an
/// unrelated `fastroute` that shares a folder with `konnect` is never picked up.
pub(crate) fn bundled_path(exe: &Path) -> Option<PathBuf> {
    let bin = exe.canonicalize().ok()?.parent()?.to_path_buf();
    bin.parent()?
        .join(PACKAGE_MANIFEST)
        .is_file()
        .then(|| bin.join(BUNDLED_NAME))
}

/// Whether anything (a file, a folder or a link, even a broken one) is at `path`.
fn present(path: &Path) -> bool {
    !matches!(
        std::fs::symlink_metadata(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    )
}

/// Proves `path` is the candidate build. Each step runs sandboxed
/// ([`sandbox_profile`] with `scratch` as the run folder), with the environment
/// cleared to `HOME`, `TMPDIR` and `PATH` as in [`run_pass`], and is killed after
/// [`PROBE_TIMEOUT`]:
/// 1. `--version`: the first line of its output must start with `fastroute `.
/// 2. `-de <scratch>/absent.dsn`, then every [`CANDIDATE_SWITCHES`] entry, then
///    [`SENTINEL_SWITCH`]: the output must contain
///    `unknown experimental switch --x.konnect_probe_unknown` and must not name any
///    other switch as unknown. A build that lacks a candidate switch names that one;
///    a build without experimental switches names none (it goes on to open the
///    absent DSN and fails on that).
///
/// Returns the binary with its SHA-256 (of the file's bytes) and version line. The
/// error says which step failed and quotes at most 300 characters of the output.
pub(crate) async fn probe(path: &Path, source: &'static str, scratch: &Path) -> Result<Binary> {
    let scratch = prepare_run_folder(scratch).context("the probe folder could not be prepared")?;
    let path = path
        .canonicalize()
        .with_context(|| format!("{} could not be resolved", path.display()))?;
    let bytes = tokio::fs::read(&path)
        .await
        .with_context(|| format!("{} could not be read", path.display()))?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    drop(bytes);

    let profile = scratch.join("probe.sb");
    std::fs::write(&profile, sandbox_profile(&scratch, &path)?)
        .with_context(|| format!("{} could not be written", profile.display()))?;

    // Step 1: the version line.
    let output = sandboxed_output(&profile, &path, &scratch, &["--version".to_string()])
        .await
        .context("probe step 1 (--version)")?;
    let version = output.lines().next().unwrap_or("").trim_end().to_string();
    if !version.starts_with("fastroute ") {
        bail!(
            "probe step 1 (--version): the first line is not a fastroute version line; output: {}",
            tail(&output, QUOTE_CHARS)
        );
    }

    // Step 2: every candidate switch is accepted, and only the sentinel is unknown.
    let mut args = vec![
        "-de".to_string(),
        scratch.join("absent.dsn").display().to_string(),
    ];
    args.extend(CANDIDATE_SWITCHES.iter().map(|switch| switch.to_string()));
    args.push(SENTINEL_SWITCH.to_string());
    let output = sandboxed_output(&profile, &path, &scratch, &args)
        .await
        .context("probe step 2 (candidate switches)")?;
    let marker = "unknown experimental switch ";
    let mut named = Vec::new();
    let mut rest = output.as_str();
    while let Some(at) = rest.find(marker) {
        let after = &rest[at + marker.len()..];
        let end = after.find(char::is_whitespace).unwrap_or(after.len());
        named.push(
            after[..end]
                .trim_end_matches(['\'', '"', '`', ',', ';', ':', '.'])
                .to_string(),
        );
        rest = &after[end..];
    }
    if let Some(other) = named.iter().find(|name| name.as_str() != SENTINEL_SWITCH) {
        bail!(
            "probe step 2: this build names {other} as an unknown experimental switch, so it is not the {CANDIDATE_NAME} build; output: {}",
            tail(&output, QUOTE_CHARS)
        );
    }
    if !named.iter().any(|name| name.as_str() == SENTINEL_SWITCH) {
        bail!(
            "probe step 2: this build did not name {SENTINEL_SWITCH} as an unknown experimental switch, so it does not check experimental switches and is not the {CANDIDATE_NAME} build; output: {}",
            tail(&output, QUOTE_CHARS)
        );
    }
    Ok(Binary {
        path,
        sha256,
        version,
        source,
    })
}

/// One pass of the hold sequence.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pass {
    /// `full`, `fallback` or `polish`.
    pub name: &'static str,
    /// The retry width in micrometres; 0 means the retry is off.
    pub neck_width_um: f64,
    pub optimizer: bool,
    /// The session the pass starts from (`--initial-session`).
    pub initial_session: Option<PathBuf>,
}

/// The files of one call that every pass of every run reads (they live in the call's
/// folder, never in a run's), each given only when the call has it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct CallFiles<'a> {
    /// The `--x.par_via_amps` file ([`amps_file_text`]).
    pub amps: Option<&'a Path>,
    /// The `--pairs` file: the differential pairs the design intent declares
    /// (`tools/route_pairs.rs`). A per-call input, so not a [`CANDIDATE_SWITCHES`]
    /// entry.
    pub pairs: Option<&'a Path>,
}

/// The command line of one pass, every setting once, in this order: `-de <dsn>`,
/// `-do <ses>`, `--report=<report>`, [`PRESET`], `--router.optimizer.enabled=<bool>`,
/// `--router.neck_width_um=<um>`, `--router.copper_to_edge_clearance_um=<um>` when
/// `hints` has one (sent as `250.001` when it is within 1e-9 of 250, as the
/// Freerouting path does), `--router.autorouter.max_threads=` and
/// `--router.optimizer.max_threads=` at [`THREADS`], [`CANDIDATE_SWITCHES`],
/// `--x.par_via_amps=<amps>` when given, `--pairs=<file>` when given, and
/// `--initial-session=<file>` as one token when the pass has one. Numbers are written without a trailing `.0` when
/// whole. Never `--live`, `--multi-start`, `--max-time` or `--diagnose`.
pub(crate) fn pass_arguments(
    dsn: &Path,
    ses: &Path,
    report: &Path,
    hints: &RoutingHints,
    files: CallFiles<'_>,
    pass: &Pass,
) -> Vec<String> {
    let mut args = vec![
        "-de".to_string(),
        dsn.display().to_string(),
        "-do".to_string(),
        ses.display().to_string(),
        format!("--report={}", report.display()),
    ];
    args.extend(PRESET.iter().map(|argument| argument.to_string()));
    args.push(format!("--router.optimizer.enabled={}", pass.optimizer));
    args.push(format!(
        "--router.neck_width_um={}",
        number(pass.neck_width_um)
    ));
    if let Some(clearance) = hints.copper_edge_clearance_um {
        // Freerouting reads exactly 250 as "unset"; the path to it sends 250.001.
        let clearance = if (clearance as f64 - 250.0).abs() < 1e-9 {
            250.001
        } else {
            clearance as f64
        };
        args.push(format!(
            "--router.copper_to_edge_clearance_um={}",
            number(clearance)
        ));
    }
    args.push(format!("--router.autorouter.max_threads={THREADS}"));
    args.push(format!("--router.optimizer.max_threads={THREADS}"));
    args.extend(CANDIDATE_SWITCHES.iter().map(|switch| switch.to_string()));
    if let Some(amps) = files.amps {
        args.push(format!("--x.par_via_amps={}", amps.display()));
    }
    if let Some(pairs) = files.pairs {
        args.push(format!("--pairs={}", pairs.display()));
    }
    if let Some(initial) = &pass.initial_session {
        args.push(format!("--initial-session={}", initial.display()));
    }
    args
}

/// The `--x.par_via_amps` file from the manifest's `intent_widths` rows: one
/// `<amps> <net>` line for each row that has a numeric `current_a` and a `status`
/// other than `not_routed`, sorted by net name, each net once (the largest current
/// when a net has two rows), with a final newline. `None` when there is no such
/// row. A net name with a line break is left out (the file is one net per line).
pub(crate) fn amps_file_text(manifest: &Value) -> Option<String> {
    let mut largest: BTreeMap<String, f64> = BTreeMap::new();
    for row in manifest["intent_widths"].as_array().into_iter().flatten() {
        let (Some(net), Some(amps)) = (row["net"].as_str(), row["current_a"].as_f64()) else {
            continue;
        };
        if row["status"].as_str() == Some("not_routed")
            || net.is_empty()
            || net.contains(['\n', '\r'])
            || !amps.is_finite()
        {
            continue;
        }
        let entry = largest.entry(net.to_string()).or_insert(amps);
        if amps > *entry {
            *entry = amps;
        }
    }
    if largest.is_empty() {
        return None;
    }
    let mut text = String::new();
    for (net, amps) in &largest {
        text.push_str(&format!("{} {net}\n", number(*amps)));
    }
    Some(text)
}

/// The sandbox profile of one run folder (macOS `sandbox-exec`):
///
/// ```text
/// (version 1)
/// (allow default)
/// (deny network*)
/// (deny file-write*)
/// (allow file-write* (subpath "<folder>") (literal "/dev/null") (literal "/dev/zero")
///                    (literal "/dev/tty") (literal "/dev/dtracehelper"))
/// (deny process-exec)
/// (allow process-exec (literal "<binary>"))
/// ```
///
/// Both paths must be absolute. A path holding `"`, `\`, a control character or a
/// newline is refused (the profile language has no safe quoting for them).
pub(crate) fn sandbox_profile(folder: &Path, binary: &Path) -> Result<String> {
    for path in [folder, binary] {
        if !path.is_absolute() {
            bail!(
                "sandbox profile: {} is not an absolute path",
                path.display()
            );
        }
        let Some(text) = path.to_str() else {
            bail!("sandbox profile: {} is not valid UTF-8", path.display());
        };
        if text
            .chars()
            .any(|c| c == '"' || c == '\\' || c.is_control())
        {
            bail!(
                "sandbox profile: {text:?} holds a quote, a backslash or a control character, which the profile language cannot quote safely"
            );
        }
    }
    Ok(format!(
        "(version 1)\n\
         (allow default)\n\
         (deny network*)\n\
         (deny file-write*)\n\
         (allow file-write* (subpath \"{folder}\") (literal \"/dev/null\") (literal \"/dev/zero\")\n\
         {indent}(literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))\n\
         (deny process-exec)\n\
         (allow process-exec (literal \"{binary}\"))\n",
        folder = folder.display(),
        binary = binary.display(),
        indent = " ".repeat(19),
    ))
}

/// What one pass left.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct PassOutcome {
    pub ses: PathBuf,
    /// The number of entries of the report's `unrouted` list.
    pub unrouted: usize,
    /// The number of entries of the report's `clearance_violations` list.
    pub clearance_violations: usize,
    pub seconds: f64,
}

/// Why a pass gave no session.
#[derive(Debug)]
pub(crate) enum PassError {
    /// Killed at its deadline.
    Killed { seconds: f64 },
    /// It ran and failed: a non-zero exit, no session, or a report that does not read.
    Failed(anyhow::Error),
}

/// Runs one pass in `folder` (created by the caller; it holds nothing yet):
/// - writes `folder/profile.sb` ([`sandbox_profile`] for `folder` and the binary),
///   creates `folder/home` and `folder/tmp`;
/// - runs `/usr/bin/sandbox-exec -f folder/profile.sb <binary> <pass_arguments>`
///   with `folder` as the working folder, the environment cleared and set to
///   `HOME=folder/home`, `TMPDIR=folder/tmp`, `PATH=/usr/bin:/bin` only, stdin
///   closed, stdout and stderr appended to `folder/fastroute.log`, and
///   `kill_on_drop(true)`, so dropping the future kills the run;
/// - the session is `folder/<pass name>.ses`, the report `folder/<pass name>.json`;
/// - at `deadline` the child is killed and waited for: `Killed`;
/// - `Failed` on a non-zero exit (the last 300 characters of the log in the
///   error), a session that is missing or empty, or a report whose `unrouted` or
///   `clearance_violations` is not a list.
///
/// sandbox-exec replaces itself with the binary, and the profile lets no other
/// program start, so killing the child kills the whole run.
pub(crate) async fn run_pass(
    binary: &Binary,
    folder: &Path,
    dsn: &Path,
    hints: &RoutingHints,
    files: CallFiles<'_>,
    pass: Pass,
    deadline: Instant,
) -> std::result::Result<PassOutcome, PassError> {
    let started = Instant::now();

    // The profile names the real path: the sandbox resolves symbolic links first.
    let real = prepare_run_folder(folder).map_err(|error| {
        PassError::Failed(error.context("the run folder could not be prepared"))
    })?;
    let profile = folder.join("profile.sb");
    let profile_text = sandbox_profile(&real, &binary.path).map_err(PassError::Failed)?;
    std::fs::write(&profile, profile_text).map_err(|error| {
        PassError::Failed(anyhow!(
            "{} could not be written: {error}",
            profile.display()
        ))
    })?;

    let ses = folder.join(format!("{}.ses", pass.name));
    let report = folder.join(format!("{}.json", pass.name));
    let args = pass_arguments(dsn, &ses, &report, hints, files, &pass);

    let log_path = folder.join("fastroute.log");
    let (log, log_err) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .and_then(|log| log.try_clone().map(|clone| (log, clone)))
        .map_err(|error| {
            PassError::Failed(anyhow!(
                "{} could not be opened: {error}",
                log_path.display()
            ))
        })?;

    let mut command = tokio::process::Command::new(SANDBOX_EXEC);
    command
        .arg("-f")
        .arg(&profile)
        .arg(&binary.path)
        .args(&args)
        .current_dir(folder)
        .env_clear()
        .env("HOME", real.join("home"))
        .env("TMPDIR", real.join("tmp"))
        .env("PATH", RUN_PATH)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| {
        PassError::Failed(anyhow!("{SANDBOX_EXEC} could not be started: {error}"))
    })?;

    let status = match tokio::time::timeout_at(deadline, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            return Err(PassError::Failed(anyhow!(
                "waiting for fastroute failed: {error}"
            )))
        }
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(PassError::Killed {
                seconds: started.elapsed().as_secs_f64(),
            });
        }
    };
    let seconds = started.elapsed().as_secs_f64();
    let log_tail = || {
        tail(
            &std::fs::read_to_string(&log_path).unwrap_or_default(),
            QUOTE_CHARS,
        )
    };
    if !status.success() {
        return Err(PassError::Failed(anyhow!(
            "fastroute pass {} exited with {status}; log tail: {}",
            pass.name,
            log_tail()
        )));
    }
    match std::fs::metadata(&ses) {
        Ok(meta) if meta.is_file() && meta.len() > 0 => {}
        _ => {
            return Err(PassError::Failed(anyhow!(
                "fastroute pass {} left no session ({} is missing or empty); log tail: {}",
                pass.name,
                ses.display(),
                log_tail()
            )))
        }
    }
    let report_value: Value = std::fs::read_to_string(&report)
        .with_context(|| format!("the report {} could not be read", report.display()))
        .and_then(|text| {
            serde_json::from_str(&text)
                .with_context(|| format!("the report {} is not JSON", report.display()))
        })
        .map_err(PassError::Failed)?;
    let count = |key: &str| -> std::result::Result<usize, PassError> {
        report_value[key]
            .as_array()
            .map(Vec::len)
            .ok_or_else(|| PassError::Failed(anyhow!("the report's {key} is not a list")))
    };
    Ok(PassOutcome {
        ses,
        unrouted: count("unrouted")?,
        clearance_violations: count("clearance_violations")?,
        seconds,
    })
}

/// The kept session of a run and what each pass did.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Held {
    pub ses: PathBuf,
    pub unrouted: usize,
    /// `full`, `fallback` or `polish`.
    pub kept: &'static str,
    /// One row per pass: `name`, `outcome` (`kept`, `discarded`, `skipped` or
    /// `failed`), `unrouted`, `seconds`, and `reason` for a pass discarded, skipped
    /// or failed (a failed pass's reason is its error, cut to 300 characters).
    pub passes: Vec<Value>,
    pub seconds: f64,
}

/// The hold sequence (design decision 4), each pass in its own subfolder of
/// `folder` (`full`, `fallback`, `polish`):
/// 1. Full: retry off (neck 0), optimizer on, no initial session. Kept.
///    If it leaves nothing open, the run ends here; Fallback and Polish are
///    `skipped` with reason "nothing open".
/// 2. Fallback, only when Full left connections open: skipped with its reason when
///    `fallback` is `WidthFallback::None` or `hints.retry_width_um` is `None`
///    (`hints.retry_off_reason` is the reason). Else: neck = the floor
///    (`hints.retry_width_um`), optimizer off, initial session = Full's. Kept only
///    when it leaves fewer open than Full; else `discarded` ("closed none") and the
///    run ends with Full.
/// 3. Polish, only after a kept Fallback: neck 0, optimizer on, initial session =
///    Fallback's. Kept when it leaves no more open than Fallback; else discarded.
///
/// One `deadline` bounds every pass. A pass killed by it ends the run with
/// `PassError::Killed`, whatever an earlier pass kept: a run cut short is never a
/// candidate. Full failing ends the run with its `Failed`. A Fallback or Polish that
/// fails (non-zero exit, no session, an unreadable report) does not: its row is
/// `failed` with the error as the reason, and the run ends with the last kept
/// session (Full's, or Fallback's when that was kept), as the product's sequence
/// leaves the earlier session when a later pass cannot improve on it. A failed
/// Fallback leaves Polish `skipped`.
pub(crate) async fn route_held(
    binary: &Binary,
    folder: &Path,
    dsn: &Path,
    hints: &RoutingHints,
    files: CallFiles<'_>,
    fallback: WidthFallback,
    deadline: Instant,
) -> std::result::Result<Held, PassError> {
    let row = |name: &str,
               outcome: &str,
               unrouted: Option<usize>,
               seconds: Option<f64>,
               reason: Option<&str>| {
        json!({
            "name": name,
            "outcome": outcome,
            "unrouted": unrouted,
            "seconds": seconds,
            "reason": reason,
        })
    };
    let pass_folder = |name: &str| -> std::result::Result<PathBuf, PassError> {
        let path = folder.join(name);
        std::fs::create_dir_all(&path).map_err(|error| {
            PassError::Failed(anyhow!("{} could not be created: {error}", path.display()))
        })?;
        Ok(path)
    };
    let mut passes = Vec::new();

    // Full: the retry off, the optimizer on.
    let full = run_pass(
        binary,
        &pass_folder("full")?,
        dsn,
        hints,
        files,
        Pass {
            name: "full",
            neck_width_um: 0.0,
            optimizer: true,
            initial_session: None,
        },
        deadline,
    )
    .await?;
    let mut seconds = full.seconds;
    passes.push(row(
        "full",
        "kept",
        Some(full.unrouted),
        Some(full.seconds),
        None,
    ));
    let mut kept = "full";
    let mut kept_ses = full.ses.clone();
    let mut kept_unrouted = full.unrouted;

    // Fallback: only when Full left connections open, and only with a floor.
    let mut fallback_kept: Option<PassOutcome> = None;
    if full.unrouted == 0 {
        passes.push(row("fallback", "skipped", None, None, Some("nothing open")));
    } else {
        let floor = match (fallback, hints.retry_width_um) {
            (WidthFallback::Floor, Some(um)) => Ok(um as f64),
            (WidthFallback::Floor, None) => Err(hints
                .retry_off_reason
                .unwrap_or("the manifest gives no single floor width")
                .to_string()),
            (WidthFallback::None, _) => Err("the width fallback is off for this call".to_string()),
        };
        match floor {
            Err(reason) => {
                passes.push(row(
                    "fallback",
                    "skipped",
                    None,
                    None,
                    Some(reason.as_str()),
                ));
            }
            Ok(floor) => {
                let result = run_pass(
                    binary,
                    &pass_folder("fallback")?,
                    dsn,
                    hints,
                    files,
                    Pass {
                        name: "fallback",
                        neck_width_um: floor,
                        optimizer: false,
                        initial_session: Some(full.ses.clone()),
                    },
                    deadline,
                )
                .await;
                if let Some(outcome) = settle("fallback", result, &mut passes)? {
                    seconds += outcome.seconds;
                    if outcome.unrouted < full.unrouted {
                        passes.push(row(
                            "fallback",
                            "kept",
                            Some(outcome.unrouted),
                            Some(outcome.seconds),
                            None,
                        ));
                        kept = "fallback";
                        kept_ses = outcome.ses.clone();
                        kept_unrouted = outcome.unrouted;
                        fallback_kept = Some(outcome);
                    } else {
                        let reason = if outcome.unrouted == full.unrouted {
                            "closed none"
                        } else {
                            "left more open than Full"
                        };
                        passes.push(row(
                            "fallback",
                            "discarded",
                            Some(outcome.unrouted),
                            Some(outcome.seconds),
                            Some(reason),
                        ));
                    }
                }
            }
        }
    }

    // Polish: only after a kept Fallback; the retry off, the optimizer on.
    match fallback_kept {
        Some(after_fallback) => {
            let result = run_pass(
                binary,
                &pass_folder("polish")?,
                dsn,
                hints,
                files,
                Pass {
                    name: "polish",
                    neck_width_um: 0.0,
                    optimizer: true,
                    initial_session: Some(after_fallback.ses.clone()),
                },
                deadline,
            )
            .await;
            if let Some(outcome) = settle("polish", result, &mut passes)? {
                seconds += outcome.seconds;
                if outcome.unrouted <= after_fallback.unrouted {
                    passes.push(row(
                        "polish",
                        "kept",
                        Some(outcome.unrouted),
                        Some(outcome.seconds),
                        None,
                    ));
                    kept = "polish";
                    kept_ses = outcome.ses.clone();
                    kept_unrouted = outcome.unrouted;
                } else {
                    passes.push(row(
                        "polish",
                        "discarded",
                        Some(outcome.unrouted),
                        Some(outcome.seconds),
                        Some("left more open than Fallback"),
                    ));
                }
            }
        }
        None => {
            let reason = if full.unrouted == 0 {
                "nothing open"
            } else {
                "no Fallback was kept"
            };
            passes.push(row("polish", "skipped", None, None, Some(reason)));
        }
    }

    Ok(Held {
        ses: kept_ses,
        unrouted: kept_unrouted,
        kept,
        passes,
        seconds,
    })
}

/// What a later pass's result means for the run: its outcome, or `None` after a
/// failure, which is recorded as a `failed` row (the run then keeps the earlier
/// session). A pass killed by the deadline ends the run.
fn settle(
    name: &str,
    result: std::result::Result<PassOutcome, PassError>,
    passes: &mut Vec<Value>,
) -> std::result::Result<Option<PassOutcome>, PassError> {
    match result {
        Ok(outcome) => Ok(Some(outcome)),
        Err(PassError::Failed(error)) => {
            passes.push(json!({
                "name": name,
                "outcome": "failed",
                "unrouted": null,
                "seconds": null,
                "reason": tail(&format!("{error:#}"), QUOTE_CHARS),
            }));
            Ok(None)
        }
        Err(killed) => Err(killed),
    }
}

/// A short, readable reason, for refusals and records.
pub(crate) fn tail(text: &str, chars: usize) -> String {
    let count = text.chars().count();
    if count <= chars {
        text.to_string()
    } else {
        text.chars().skip(count - chars).collect()
    }
}

/// `0.0` as `0`, `200.0` as `200`, `250.001` as `250.001`.
fn number(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Fails with the reason when `path` is not a regular, executable file.
fn regular_executable(path: &Path) -> std::result::Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|error| format!("it cannot be read: {error}"))?;
    if !meta.is_file() {
        return Err("it is not a regular file".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err("it is not executable".to_string());
        }
    }
    Ok(())
}

/// Creates `folder` with its `home` and `tmp` subfolders and returns its real
/// (symbolic links resolved) path, the one the sandbox profile must name.
fn prepare_run_folder(folder: &Path) -> Result<PathBuf> {
    for sub in ["", "home", "tmp"] {
        let path = folder.join(sub);
        std::fs::create_dir_all(&path)
            .with_context(|| format!("{} could not be created", path.display()))?;
    }
    folder
        .canonicalize()
        .with_context(|| format!("{} could not be resolved", folder.display()))
}

/// Runs `binary` under the profile at `profile` with the run environment (HOME
/// and TMPDIR inside `folder`) and returns its stdout and stderr together.
/// Killed after [`PROBE_TIMEOUT`]. The exit status is not evidence and is not
/// checked.
async fn sandboxed_output(
    profile: &Path,
    binary: &Path,
    folder: &Path,
    args: &[String],
) -> Result<String> {
    let mut command = tokio::process::Command::new(SANDBOX_EXEC);
    command
        .arg("-f")
        .arg(profile)
        .arg(binary)
        .args(args)
        .current_dir(folder)
        .env_clear()
        .env("HOME", folder.join("home"))
        .env("TMPDIR", folder.join("tmp"))
        .env("PATH", RUN_PATH)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = match tokio::time::timeout(PROBE_TIMEOUT, command.output()).await {
        Ok(output) => output.with_context(|| format!("{SANDBOX_EXEC} could not be run"))?,
        Err(_) => bail!(
            "it did not answer within {} s and was killed",
            PROBE_TIMEOUT.as_secs()
        ),
    };
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text)
}

#[cfg(test)]
#[path = "fastroute_tests.rs"]
mod tests;
