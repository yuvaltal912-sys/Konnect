//! `cargo xtask freerouting-bench`: route a folder of Specctra DSN files with
//! one or more Freerouting JARs and record, per board and version, how many
//! connections stay unrouted, the via count, the wirelength, the runtime and
//! whether two runs write the same SES.
//!
//! Opt-in only. It needs Java and Freerouting JARs, takes minutes, and is never
//! part of CI or `cargo test`; its parsing and ranking are unit-tested without
//! Java. Results go under `target/freerouting-bench/`, never into tracked docs
//! (only counts are quoted there).
//!
//! Each run gets its own folder with a copy of the DSN, its own `user.home`
//! and its own `java.io.tmpdir`. Freerouting keeps `freerouting.json` and its log
//! under one of those (2.1 under the temp folder, 2.5 under
//! `~/Library/Application Support` and `~/Library/Logs` on macOS), so a run
//! neither reads nor changes the settings of a Freerouting the owner uses, and
//! every version starts from the same settings. Both places are seeded with
//! telemetry and analytics off, and the same is passed on the command line and
//! in the environment: Freerouting 2.x turns both on by default. Its update
//! check still asks GitHub for the latest release; nothing here can turn that
//! off, and it carries no board data.

pub mod parse;
pub mod quality;
pub mod report;

use parse::{
    dsn_connection_count, final_counts_from_log, ses_fingerprint, statistics_block, summarize_ses,
    version_from_file_name, version_from_log,
};
use report::{BoardResult, RunRecord};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Newest first. One Java runs every JAR, so versions are compared on the same
/// JVM; Freerouting 2.5.0 needs Java 25 (class file 69), 2.1 to 2.4 need 21.
const HOMEBREW_JAVAS: &[&str] = &[
    "/opt/homebrew/opt/openjdk@25/bin/java",
    "/opt/homebrew/opt/openjdk@21/bin/java",
];

const USAGE: &str =
    "usage: cargo xtask freerouting-bench --jar <freerouting.jar> [--jar <another.jar> ...]
       [--dsn-dir <dir> ...] [--java <java>] [--runs <n>] [--max-passes <n>]
       [--threads <n>] [--job-timeout-secs <s>] [--timeout-secs <s>] [--out <dir>]
       [--fr-arg <arg> ...]
       cargo xtask freerouting-bench --rescore <results folder>

  --jar           a Freerouting JAR to benchmark; repeat for each version
  --dsn-dir       folder of .dsn files (not recursive); repeatable.
                  Default: crates/konnect-core/tests/fixtures
  --java          the Java that runs every JAR. Default: the newest of
                  /opt/homebrew/opt/openjdk@25 and @21 present, else `java`.
                  Freerouting 2.5.0 needs Java 25; 2.1 to 2.4 need 21
  --runs          runs per board and JAR, compared for determinism (default 2)
  --max-passes    Freerouting -mp (default 100)
  --threads       Freerouting -mt (default 1, the setting most likely to repeat)
  --job-timeout-secs
                  Freerouting's own job budget, --router.job_timeout (default 60).
                  A job that hits it stops and writes what it routed, so a big
                  board still gives unrouted counts. Such runs are budget-limited
                  and are not compared for determinism
  --timeout-secs  hard limit per run; the process is killed (default 600)
  --out           results folder. Default: target/freerouting-bench/<unix time>
  --fr-arg        an extra argument passed to every Freerouting run
  --rescore <dir> re-measure a finished run's folder from its saved DSN, SES
                  and results.json, without Java, and rewrite its report";

/// Options as given on the command line.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub jars: Vec<PathBuf>,
    pub dsn_dirs: Vec<PathBuf>,
    pub java: Option<PathBuf>,
    pub runs: u32,
    pub max_passes: u32,
    pub threads: u32,
    pub job_timeout: Duration,
    pub timeout: Duration,
    pub out: Option<PathBuf>,
    pub extra_args: Vec<String>,
    /// Re-measure a finished run's folder instead of routing.
    pub rescore: Option<PathBuf>,
}

pub fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        jars: Vec::new(),
        dsn_dirs: Vec::new(),
        java: None,
        runs: 2,
        max_passes: 100,
        threads: 1,
        job_timeout: Duration::from_secs(60),
        timeout: Duration::from_secs(600),
        out: None,
        extra_args: Vec::new(),
        rescore: None,
    };
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        let number = |text: String, min: u64| -> Result<u64, String> {
            text.parse::<u64>()
                .ok()
                .filter(|n| *n >= min)
                .ok_or_else(|| {
                    format!("{flag} wants a whole number of at least {min}, not {text:?}")
                })
        };
        match flag.as_str() {
            "--jar" => options.jars.push(value()?.into()),
            "--dsn-dir" => options.dsn_dirs.push(value()?.into()),
            "--java" => options.java = Some(value()?.into()),
            "--runs" => options.runs = number(value()?, 1)? as u32,
            "--max-passes" => options.max_passes = number(value()?, 1)? as u32,
            "--threads" => options.threads = number(value()?, 1)? as u32,
            "--job-timeout-secs" => options.job_timeout = Duration::from_secs(number(value()?, 1)?),
            "--timeout-secs" => options.timeout = Duration::from_secs(number(value()?, 1)?),
            "--out" => options.out = Some(value()?.into()),
            "--fr-arg" => options.extra_args.push(value()?),
            "--rescore" => options.rescore = Some(value()?.into()),
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown option {other:?}\n{USAGE}")),
        }
    }
    if options.jars.is_empty() && options.rescore.is_none() {
        return Err(format!("at least one --jar is needed\n{USAGE}"));
    }
    Ok(options)
}

/// The `.dsn` files directly inside each folder, sorted by name.
pub fn dsn_files(dirs: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for dir in dirs {
        let entries =
            std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        let mut here: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dsn")))
            .collect();
        here.sort();
        files.extend(here);
    }
    if files.is_empty() {
        return Err("no .dsn files in the given folders".to_string());
    }
    Ok(files)
}

/// JVM options that keep a run off the network: 2.5.0 starts a version check
/// (a GET to `api.github.com`, with no switch) on every launch. Every HTTP and
/// HTTPS request goes to a closed loopback port instead; loopback stays direct.
const OFFLINE_JVM_ARGUMENTS: [&str; 5] = [
    "-Dhttp.proxyHost=127.0.0.1",
    "-Dhttp.proxyPort=9",
    "-Dhttps.proxyHost=127.0.0.1",
    "-Dhttps.proxyPort=9",
    "-Dhttp.nonProxyHosts=localhost|127.*|[::1]",
];

/// The arguments for one batch run in `run_dir`, after `java`. The run reads
/// `run_dir/board.dsn` and writes `run_dir/board.ses`.
pub fn freerouting_arguments(run_dir: &Path, jar: &Path, options: &Options) -> Vec<String> {
    let mut args = vec![
        "-Djava.awt.headless=true".to_string(),
        format!("-Duser.home={}", run_dir.join("home").display()),
        format!("-Djava.io.tmpdir={}", run_dir.join("tmp").display()),
    ];
    args.extend(OFFLINE_JVM_ARGUMENTS.iter().map(|a| a.to_string()));
    args.extend([
        "-jar".to_string(),
        jar.display().to_string(),
        "-de".to_string(),
        run_dir.join("board.dsn").display().to_string(),
        "-do".to_string(),
        run_dir.join("board.ses").display().to_string(),
        "-mp".to_string(),
        options.max_passes.to_string(),
        "-mt".to_string(),
        options.threads.to_string(),
        format!("--router.job_timeout={}", hh_mm_ss(options.job_timeout)),
        "--gui.enabled=false".to_string(),
        "--profile.allow_telemetry=false".to_string(),
        "--usage_and_diagnostic_data.disable_analytics=true".to_string(),
    ]);
    args.extend(options.extra_args.iter().cloned());
    args
}

/// `00:01:30` for 90 seconds, the form `router.job_timeout` takes.
pub fn hh_mm_ss(duration: Duration) -> String {
    let s = duration.as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// The job ran out of its budget. 2.3 and later log `TIMED_OUT` or "completed
/// with timeout", and a `finished with state:` line for every job, so their
/// own word decides: the process wall clock also holds JVM start-up and the
/// SES write, which the job's budget does not. 2.1 says nothing, so for it a
/// run that lasted the whole budget counts.
pub fn hit_job_budget(stdout: &str, runtime: Duration, budget: Duration) -> bool {
    if stdout.contains("TIMED_OUT") || stdout.contains("completed with timeout") {
        return true;
    }
    if stdout.contains("finished with state:") {
        return false;
    }
    runtime >= budget
}

/// Settings Freerouting reads from its `freerouting.json` before the command
/// line: telemetry, contact and analytics off.
/// `profile.id` must be a UUID: 2.3 and later stop with a
/// `NullPointerException` without one. A fixed all-zero one keeps runs alike
/// and identifies nobody.
const SEEDED_SETTINGS: &str = r#"{"profile":{"id":"00000000-0000-0000-0000-000000000000","allow_telemetry":false,"allow_contact":false},"usage_and_diagnostic_data":{"disable_analytics":true}}"#;

/// Where the Freerouting releases benchmarked so far look for
/// `freerouting.json`, relative to the run folder: 2.1 under
/// `java.io.tmpdir`, 2.5 under `user.home` (`Library/Application Support` on
/// macOS, `.config` on Linux; `.local/share` there is only its data folder).
/// 2.5 also copies a settings file from `java.io.tmpdir/freerouting` when it
/// finds none in its own folder.
const SETTINGS_PLACES: &[&str] = &[
    "tmp/freerouting",
    "home/Library/Application Support/freerouting",
    "home/.config/freerouting",
];

/// The environment of one run: telemetry, contact and analytics off, and the
/// settings, state, cache and data folders pinned inside the run folder.
/// 2.5 lets `XDG_*` (Linux) and `APPDATA` and `LOCALAPPDATA` (Windows) override
/// `user.home`, so a host that sets them would send the run to the owner's
/// real folders. Pairs with [`freerouting_env_removed`].
pub fn freerouting_env(run_dir: &Path) -> Vec<(String, String)> {
    let home = run_dir.join("home");
    let at = |folder: &str| home.join(folder).display().to_string();
    [
        ("FREEROUTING__PROFILE__ALLOW_TELEMETRY", "false".to_string()),
        ("FREEROUTING__PROFILE__ALLOW_CONTACT", "false".to_string()),
        (
            "FREEROUTING__USAGE_AND_DIAGNOSTIC_DATA__DISABLE_ANALYTICS",
            "true".to_string(),
        ),
        ("XDG_CONFIG_HOME", at(".config")),
        ("XDG_STATE_HOME", at(".local/state")),
        ("XDG_CACHE_HOME", at(".cache")),
        ("XDG_DATA_HOME", at(".local/share")),
        ("APPDATA", at("AppData/Roaming")),
        ("LOCALAPPDATA", at("AppData/Local")),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_string(), value))
    .collect()
}

/// Inherited `FREEROUTING__*` variables to drop, from `names` (the process
/// environment's): `FREEROUTING__USER_DATA_PATH` and
/// `FREEROUTING__LOGGING__FILE__LOCATION` would override the run's own
/// folders. Those [`freerouting_env`] sets itself are kept for it to set.
pub fn freerouting_env_removed(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let ours: Vec<String> = freerouting_env(Path::new(""))
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    names
        .into_iter()
        .filter(|name| name.to_ascii_uppercase().starts_with("FREEROUTING__"))
        .filter(|name| !ours.contains(name))
        .collect()
}

/// Every `freerouting.log` below `dir`, concatenated.
fn logs_below(dir: &Path) -> String {
    let mut out = String::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|n| n == "freerouting.log") {
                out.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    out
}

pub fn run(args: &[String]) -> Result<(), String> {
    let options = parse_options(args)?;
    if let Some(dir) = &options.rescore {
        return rescore(dir);
    }
    let root = crate::repo_root();
    let dsn_dirs = if options.dsn_dirs.is_empty() {
        vec![root.join("crates/konnect-core/tests/fixtures")]
    } else {
        options.dsn_dirs.clone()
    };
    let boards = dsn_files(&dsn_dirs)?;
    let java = options.java.clone().unwrap_or_else(|| {
        HOMEBREW_JAVAS
            .iter()
            .map(PathBuf::from)
            .find(|java| java.exists())
            .unwrap_or_else(|| PathBuf::from("java"))
    });
    for jar in &options.jars {
        if !jar.is_file() {
            return Err(format!("--jar is not a file: {}", jar.display()));
        }
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let out = options.out.clone().unwrap_or_else(|| {
        root.join("target/freerouting-bench")
            .join(stamp.to_string())
    });
    std::fs::create_dir_all(&out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    let out = out.canonicalize().map_err(|e| e.to_string())?;
    let java_version = java_version(&java);
    eprintln!(
        "freerouting-bench: {} boards x {} JARs x {} runs, java {}, results in {}",
        boards.len(),
        options.jars.len(),
        options.runs,
        java_version.as_deref().unwrap_or("(version unknown)"),
        out.display()
    );

    let jars = options
        .jars
        .iter()
        .map(|jar| jar.canonicalize().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let labels = jar_labels(&jars);
    let mut results: Vec<BoardResult> = Vec::new();
    for (jar, label) in jars.iter().zip(&labels) {
        let jar = jar.clone();
        let file_version = jar
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(version_from_file_name);
        for dsn in &boards {
            let board = dsn
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let dsn_text = std::fs::read_to_string(dsn).unwrap_or_default();
            let dsn_board = quality::DsnBoard::parse(&dsn_text);
            let mut runs = Vec::new();
            for index in 1..=options.runs {
                let dir = out.join(label).join(&board).join(format!("run{index}"));
                let record = run_once(&java, &jar, dsn, dsn_board.as_ref().ok(), &dir, &options);
                eprintln!(
                    "  {label:>8} {board:<48} run {index}: {}",
                    record.one_line()
                );
                runs.push(record);
            }
            let version = runs
                .iter()
                .find_map(|r| r.reported_version.clone())
                .or_else(|| file_version.clone())
                .unwrap_or_else(|| "unknown".to_string());
            results.push(BoardResult {
                board,
                dir: label.clone(),
                version,
                jar: jar.clone(),
                dsn_connections: dsn_connection_count(&dsn_text),
                components: dsn_board.as_ref().ok().map(|b| b.components),
                signal_layers: dsn_board.as_ref().ok().map(|b| b.signal_layers),
                runs,
            });
        }
    }

    let summary = report::render_markdown(&results, &options, java_version.as_deref());
    let json = report::to_json(&report::results_json(
        &results,
        &options,
        java_version.as_deref(),
    ));
    std::fs::write(out.join("results.json"), json).map_err(|e| e.to_string())?;
    std::fs::write(out.join("summary.md"), &summary).map_err(|e| e.to_string())?;
    println!("{summary}");
    eprintln!("wrote {}", out.join("results.json").display());
    Ok(())
}

fn java_version(java: &Path) -> Option<String> {
    let output = Command::new(java).arg("-version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stderr).to_string()
        + &String::from_utf8_lossy(&output.stdout);
    text.split('"').nth(1).map(str::to_string)
}

fn run_once(
    java: &Path,
    jar: &Path,
    dsn: &Path,
    board: Option<&quality::DsnBoard>,
    dir: &Path,
    options: &Options,
) -> RunRecord {
    let mut record = RunRecord::default();
    let prepared = SETTINGS_PLACES
        .iter()
        .try_for_each(|place| {
            std::fs::create_dir_all(dir.join(place))?;
            std::fs::write(dir.join(place).join("freerouting.json"), SEEDED_SETTINGS)
        })
        .and_then(|()| std::fs::copy(dsn, dir.join("board.dsn")).map(drop));
    if let Err(error) = prepared {
        record.error = Some(format!("cannot prepare {}: {error}", dir.display()));
        return record;
    }
    let ses = dir.join("board.ses");
    let _ = std::fs::remove_file(&ses);
    let stdout_path = dir.join("stdout.log");
    let stderr_path = dir.join("stderr.log");
    let (Ok(stdout), Ok(stderr)) = (
        std::fs::File::create(&stdout_path),
        std::fs::File::create(&stderr_path),
    ) else {
        record.error = Some("cannot create the run's log files".to_string());
        return record;
    };
    let started = Instant::now();
    let mut command = Command::new(java);
    for name in freerouting_env_removed(
        std::env::vars_os().map(|(name, _)| name.to_string_lossy().into_owned()),
    ) {
        command.env_remove(name);
    }
    let child = command
        .args(freerouting_arguments(dir, jar, options))
        .current_dir(dir)
        .envs(freerouting_env(dir))
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            record.error = Some(format!("cannot start {}: {error}", java.display()));
            return record;
        }
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                record.exit_code = status.code();
                break;
            }
            Ok(None) if started.elapsed() > options.timeout => {
                let _ = child.kill();
                let _ = child.wait();
                record.timed_out = true;
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(error) => {
                record.error = Some(format!("waiting for Freerouting failed: {error}"));
                let _ = child.kill();
                break;
            }
        }
    }
    record.runtime_s = started.elapsed().as_secs_f64();
    let elapsed = started.elapsed();

    let stdout = std::fs::read_to_string(&stdout_path).unwrap_or_default();
    let log = logs_below(dir);
    record.hit_job_budget = hit_job_budget(&stdout, elapsed, options.job_timeout);
    record.reported_version = version_from_log(&stdout).or_else(|| version_from_log(&log));
    record.analytics_disabled = log.contains("Analytics are disabled");
    if let Some(stats) = statistics_block(&stdout) {
        record.unrouted = stats
            .number_at("connections.incomplete_count")
            .map(|n| n as u64);
        record.connections = stats
            .number_at("connections.maximum_count")
            .map(|n| n as u64);
        record.stats_vias = stats.number_at("vias.total_count").map(|n| n as u64);
        record.clearance_violations = stats
            .number_at("clearance_violations.total_count")
            .map(|n| n as u64);
    }
    // 2.5 prints no statistics block; its log lines say the same.
    if record.unrouted.is_none() {
        if let Some((unrouted, violations)) = final_counts_from_log(&stdout) {
            record.unrouted = Some(unrouted);
            record.clearance_violations = violations;
        }
    }
    match std::fs::read_to_string(&ses) {
        Ok(text) => read_ses(&mut record, &text, board),
        Err(_) if record.error.is_none() => {
            let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
            record.error = Some(if record.timed_out {
                format!("timed out after {} s", options.timeout.as_secs())
            } else {
                no_ses_reason(record.exit_code, &stderr)
            });
        }
        Err(_) => {}
    }
    record
}

/// What a run's SES says: its fingerprints, its summary and its quality.
fn read_ses(record: &mut RunRecord, text: &str, board: Option<&quality::DsnBoard>) {
    record.ses_fingerprint = Some(ses_fingerprint(text));
    if let Some(board) = board {
        match quality::measure(board, text) {
            Ok(measured) => record.quality = Some(measured),
            Err(error) => record.quality_error = Some(error),
        }
    }
    match summarize_ses(text) {
        Ok(summary) => record.ses = Some(summary),
        Err(error) => {
            record
                .error
                .get_or_insert(format!("unreadable SES: {error}"));
        }
    }
}

/// The folder name a JAR's runs go in, from its file name's version.
fn jar_label(jar: &Path) -> String {
    jar.file_name()
        .and_then(|n| n.to_str())
        .and_then(version_from_file_name)
        .unwrap_or_else(|| "unknown".to_string())
}

/// One folder name per JAR, all different: a second JAR with a label already
/// taken (a rebuild of the same version, or two files that name none) gets
/// `-2`, then `-3`. Otherwise their runs would overwrite each other's files and
/// their totals would merge.
pub fn jar_labels(jars: &[PathBuf]) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for jar in jars {
        let base = jar_label(jar);
        let mut label = base.clone();
        let mut n = 2;
        while labels.contains(&label) {
            label = format!("{base}-{n}");
            n += 1;
        }
        labels.push(label);
    }
    labels
}

/// Re-measure a finished run from `dir/results.json` and each run folder's
/// `board.dsn` and `board.ses`. What Freerouting reported (exit, runtime,
/// unrouted, violations) is kept; everything measured from files is redone.
pub fn rescore(dir: &Path) -> Result<(), String> {
    use parse::Json;
    let text = std::fs::read_to_string(dir.join("results.json"))
        .map_err(|e| format!("cannot read {}: {e}", dir.join("results.json").display()))?;
    let (saved, _) = parse::parse_json_prefix(&text).ok_or("results.json is not JSON")?;
    let whole = |key: &str| saved.number_at(key).map(|n| n as u64);
    let mut options = parse_options(&["--rescore".to_string(), dir.display().to_string()])?;
    options.runs = whole("runs_per_board").unwrap_or(2) as u32;
    options.max_passes = whole("max_passes").unwrap_or(100) as u32;
    options.threads = whole("threads").unwrap_or(1) as u32;
    options.job_timeout = Duration::from_secs(whole("job_timeout_s").unwrap_or(60));
    options.timeout = Duration::from_secs(whole("timeout_s").unwrap_or(600));
    let text_of = |value: &Json, key: &str| match value.at(key) {
        Some(Json::Text(s)) => Some(s.clone()),
        _ => None,
    };
    let flag = |value: &Json, key: &str| matches!(value.at(key), Some(Json::Bool(true)));
    let java = text_of(&saved, "java");
    let Some(Json::Array(boards)) = saved.at("boards") else {
        return Err("results.json has no boards".to_string());
    };
    let mut results = Vec::new();
    for saved_board in boards {
        let board = text_of(saved_board, "board").ok_or("a board has no name")?;
        let version = text_of(saved_board, "version").unwrap_or_default();
        let jar = PathBuf::from(text_of(saved_board, "jar").unwrap_or_default());
        // The folder this board's runs were written to; a results.json from
        // before it was saved falls back to the label the JAR's name gives.
        let label = text_of(saved_board, "dir").unwrap_or_else(|| jar_label(&jar));
        let Some(Json::Array(saved_runs)) = saved_board.at("runs") else {
            continue;
        };
        let mut dsn_board = None;
        let mut runs = Vec::new();
        for (index, saved_run) in saved_runs.iter().enumerate() {
            let run_dir = dir
                .join(&label)
                .join(&board)
                .join(format!("run{}", index + 1));
            if dsn_board.is_none() {
                dsn_board = std::fs::read_to_string(run_dir.join("board.dsn"))
                    .ok()
                    .and_then(|t| quality::DsnBoard::parse(&t).ok());
            }
            let count = |key: &str| saved_run.number_at(key).map(|n| n as u64);
            let mut record = RunRecord {
                exit_code: saved_run.number_at("exit_code").map(|n| n as i32),
                timed_out: flag(saved_run, "timed_out"),
                hit_job_budget: flag(saved_run, "hit_job_budget"),
                runtime_s: saved_run.number_at("runtime_s").unwrap_or(0.0),
                reported_version: text_of(saved_run, "reported_version"),
                analytics_disabled: flag(saved_run, "analytics_disabled"),
                unrouted: count("unrouted"),
                connections: count("connections"),
                stats_vias: count("stats_vias"),
                clearance_violations: count("clearance_violations"),
                // An error from reading the SES is redone; any other stays.
                error: text_of(saved_run, "error").filter(|e| !e.starts_with("unreadable SES")),
                ..RunRecord::default()
            };
            if let Ok(ses) = std::fs::read_to_string(run_dir.join("board.ses")) {
                read_ses(&mut record, &ses, dsn_board.as_ref());
            }
            runs.push(record);
        }
        results.push(BoardResult {
            board,
            dir: label,
            version,
            jar,
            dsn_connections: saved_board.number_at("dsn_connections").map(|n| n as usize),
            components: dsn_board.as_ref().map(|b| b.components),
            signal_layers: dsn_board.as_ref().map(|b| b.signal_layers),
            runs,
        });
    }
    let summary = report::render_markdown(&results, &options, java.as_deref());
    let json = report::to_json(&report::results_json(&results, &options, java.as_deref()));
    std::fs::write(dir.join("results.json"), json).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("summary.md"), &summary).map_err(|e| e.to_string())?;
    println!("{summary}");
    eprintln!("rescored {}", dir.display());
    Ok(())
}

/// Why a run wrote no SES: its exit code and the JVM's or Freerouting's last
/// words on stderr (a too-old Java says `UnsupportedClassVersionError` there).
pub fn no_ses_reason(exit_code: Option<i32>, stderr: &str) -> String {
    let last = stderr
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(|line| line.chars().take(240).collect::<String>());
    match last {
        Some(line) => format!("no SES written (exit {exit_code:?}): {line}"),
        None => format!("no SES written (exit {exit_code:?})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn options_have_defaults_and_repeat_jars_and_folders() {
        let options = parse_options(&args(&[
            "--jar",
            "a.jar",
            "--jar",
            "b.jar",
            "--dsn-dir",
            "x",
            "--dsn-dir",
            "y",
        ]))
        .unwrap();
        assert_eq!(
            options.jars,
            vec![PathBuf::from("a.jar"), PathBuf::from("b.jar")]
        );
        assert_eq!(options.dsn_dirs.len(), 2);
        assert_eq!(
            (options.runs, options.max_passes, options.threads),
            (2, 100, 1)
        );
        assert_eq!(options.timeout, Duration::from_secs(600));
        assert_eq!(options.job_timeout, Duration::from_secs(60));
        let options = parse_options(&args(&[
            "--jar",
            "a.jar",
            "--runs",
            "3",
            "--max-passes",
            "7",
            "--threads",
            "4",
            "--job-timeout-secs",
            "45",
            "--timeout-secs",
            "9",
            "--java",
            "/j",
            "--out",
            "o",
            "--fr-arg",
            "--x=1",
        ]))
        .unwrap();
        assert_eq!(
            (options.runs, options.max_passes, options.threads),
            (3, 7, 4)
        );
        assert_eq!(options.timeout, Duration::from_secs(9));
        assert_eq!(options.job_timeout, Duration::from_secs(45));
        assert_eq!(options.java, Some(PathBuf::from("/j")));
        assert_eq!(options.extra_args, vec!["--x=1".to_string()]);
    }

    #[test]
    fn rescoring_needs_no_jar() {
        let options = parse_options(&args(&["--rescore", "target/x"])).unwrap();
        assert_eq!(options.rescore, Some(PathBuf::from("target/x")));
        assert!(options.jars.is_empty());
        assert!(parse_options(&args(&["--rescore"])).is_err());
    }

    #[test]
    fn bad_options_are_refused_with_the_reason() {
        assert!(parse_options(&args(&[])).unwrap_err().contains("--jar"));
        assert!(parse_options(&args(&["--jar"]))
            .unwrap_err()
            .contains("needs a value"));
        assert!(parse_options(&args(&["--jar", "a", "--runs", "0"]))
            .unwrap_err()
            .contains("at least 1"));
        assert!(parse_options(&args(&["--jar", "a", "--runs", "two"])).is_err());
        assert!(parse_options(&args(&["--jar", "a", "--bogus"]))
            .unwrap_err()
            .contains("unknown option"));
    }

    #[test]
    fn every_run_turns_telemetry_off_and_keeps_its_files_in_its_folder() {
        let options = parse_options(&args(&["--jar", "fr.jar", "--fr-arg", "--extra"])).unwrap();
        let line = freerouting_arguments(Path::new("/run"), Path::new("/j/fr.jar"), &options);
        for expected in [
            "-Djava.awt.headless=true",
            "-Duser.home=/run/home",
            "-Djava.io.tmpdir=/run/tmp",
            "--gui.enabled=false",
            "--profile.allow_telemetry=false",
            "--usage_and_diagnostic_data.disable_analytics=true",
            "--extra",
        ] {
            assert!(line.iter().any(|a| a == expected), "{expected} in {line:?}");
        }
        let jar = line.iter().position(|a| a == "-jar").unwrap();
        for offline in OFFLINE_JVM_ARGUMENTS {
            assert!(
                line[..jar].iter().any(|a| a == offline),
                "{offline} in {line:?}"
            );
        }
        let pairs: Vec<(&str, &str)> = line
            .windows(2)
            .map(|w| (w[0].as_str(), w[1].as_str()))
            .collect();
        for pair in [
            ("-de", "/run/board.dsn"),
            ("-do", "/run/board.ses"),
            ("-mp", "100"),
            ("-mt", "1"),
            ("--router.job_timeout=00:01:00", "--gui.enabled=false"),
            ("-jar", "/j/fr.jar"),
        ] {
            assert!(pairs.contains(&pair), "{pair:?} in {line:?}");
        }
        assert!(SEEDED_SETTINGS.contains(r#""allow_telemetry":false"#));
        assert!(SEEDED_SETTINGS.contains(r#""id":"00000000-0000-0000-0000-000000000000""#));
        assert!(SEEDED_SETTINGS.contains(r#""disable_analytics":true"#));
        assert!(parse::parse_json_prefix(SEEDED_SETTINGS).is_some());
    }

    #[test]
    fn the_job_budget_is_written_and_detected() {
        assert_eq!(hh_mm_ss(Duration::from_secs(60)), "00:01:00");
        assert_eq!(hh_mm_ss(Duration::from_secs(3725)), "01:02:05");
        let budget = Duration::from_secs(60);
        let quick = Duration::from_secs(5);
        assert!(hit_job_budget(
            "Job 'x' finished with state: TIMED_OUT",
            quick,
            budget
        ));
        assert!(hit_job_budget(
            "Auto-routing stage completed with timeout: …",
            quick,
            budget
        ));
        // 2.3 and later report their own outcome; trust it over the process
        // wall clock, which also holds JVM start-up and the SES write.
        assert!(!hit_job_budget(
            "INFO   Job 'x' finished with state: COMPLETED (elapsed: 58.90 seconds)",
            Duration::from_secs(61),
            budget
        ));
        assert!(!hit_job_budget(
            "finished with state: COMPLETED",
            quick,
            budget
        ));
        // 2.1 says nothing, so the wall clock decides.
        assert!(hit_job_budget("", Duration::from_secs(61), budget));
        assert!(!hit_job_budget("", quick, budget));
    }

    #[test]
    fn two_jars_with_one_version_get_folders_of_their_own() {
        let jars = |names: &[&str]| -> Vec<PathBuf> { names.iter().map(PathBuf::from).collect() };
        assert_eq!(
            jar_labels(&jars(&[
                "/a/freerouting-2.5.0.jar",
                "/b/freerouting-2.5.0.jar",
                "/c/freerouting-2.1.0.jar",
                "/d/freerouting-2.5.0.jar",
            ])),
            ["2.5.0", "2.5.0-2", "2.1.0", "2.5.0-3"]
        );
        // JARs that name no version are not merged either.
        assert_eq!(
            jar_labels(&jars(&["/a/freerouting.jar", "/b/freerouting.jar"])),
            ["unknown", "unknown-2"]
        );
    }

    #[test]
    fn settings_are_seeded_where_2_5_reads_them() {
        assert!(SETTINGS_PLACES.contains(&"home/.config/freerouting"));
        assert!(SETTINGS_PLACES.contains(&"home/Library/Application Support/freerouting"));
        assert!(SETTINGS_PLACES.contains(&"tmp/freerouting"));
        assert!(!SETTINGS_PLACES.contains(&"home/.local/share/freerouting"));
    }

    #[test]
    fn a_run_pins_its_settings_folders_into_its_own_folder() {
        let env = freerouting_env(Path::new("/run"));
        let get = |name: &str| {
            env.iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
                .unwrap_or_else(|| panic!("{name} in {env:?}"))
        };
        assert_eq!(get("XDG_CONFIG_HOME"), "/run/home/.config");
        assert_eq!(get("XDG_STATE_HOME"), "/run/home/.local/state");
        assert_eq!(get("XDG_CACHE_HOME"), "/run/home/.cache");
        assert_eq!(get("XDG_DATA_HOME"), "/run/home/.local/share");
        assert_eq!(get("APPDATA"), "/run/home/AppData/Roaming");
        assert_eq!(get("LOCALAPPDATA"), "/run/home/AppData/Local");
        assert_eq!(get("FREEROUTING__PROFILE__ALLOW_TELEMETRY"), "false");
        // The seeded config folder is the one XDG_CONFIG_HOME names.
        assert!(SETTINGS_PLACES
            .iter()
            .any(|p| format!("/run/{p}") == format!("{}/freerouting", get("XDG_CONFIG_HOME"))));
    }

    #[test]
    fn inherited_freerouting_settings_that_would_leave_the_run_folder_are_dropped() {
        let names = [
            "PATH",
            "FREEROUTING__USER_DATA_PATH",
            "FREEROUTING__LOGGING__FILE__LOCATION",
            "FREEROUTING__PROFILE__ALLOW_TELEMETRY",
        ]
        .map(String::from);
        // The three the run sets itself are not removed; it overrides them.
        assert_eq!(
            freerouting_env_removed(names),
            [
                "FREEROUTING__USER_DATA_PATH",
                "FREEROUTING__LOGGING__FILE__LOCATION"
            ]
        );
    }

    /// `--rescore` reads each board's runs from the folder the run wrote to,
    /// which is saved as `dir`; an older results.json without it falls back to
    /// the JAR's file name.
    #[test]
    fn rescoring_reads_the_saved_folder_and_falls_back_to_the_jar_name() {
        let dsn = crate::repo_root()
            .join("crates/konnect-core/tests/fixtures/specctra_two_resistors.native-kicad-10.dsn");
        let root =
            std::env::temp_dir().join(format!("konnect-bench-rescore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let components = |saved_dir: Option<&str>, run_folder: &str| {
            let out = root.join(format!("{}-{run_folder}", saved_dir.unwrap_or("old")));
            let run = out.join(run_folder).join("b").join("run1");
            std::fs::create_dir_all(&run).unwrap();
            std::fs::copy(&dsn, run.join("board.dsn")).unwrap();
            let dir_field = saved_dir.map_or(String::new(), |d| format!(r#""dir":"{d}","#));
            std::fs::write(
                out.join("results.json"),
                format!(
                    r#"{{"boards":[{{"board":"b",{dir_field}"version":"2.5.0","jar":"/a/freerouting-2.5.0.jar","runs":[{{"exit_code":0}}]}}]}}"#
                ),
            )
            .unwrap();
            rescore(&out).unwrap();
            let text = std::fs::read_to_string(out.join("results.json")).unwrap();
            let (json, _) = parse::parse_json_prefix(&text).unwrap();
            let Some(parse::Json::Array(boards)) = json.at("boards") else {
                panic!("boards in {text}")
            };
            (
                boards[0].text_at("dir").map(str::to_string),
                boards[0].number_at("components"),
            )
        };
        // Saved dir "2.5.0-2": the second 2.5.0 JAR's folder, not "2.5.0".
        assert_eq!(
            components(Some("2.5.0-2"), "2.5.0-2"),
            (Some("2.5.0-2".to_string()), Some(2.0))
        );
        assert_eq!(components(Some("2.5.0-2"), "2.5.0").1, None);
        // No saved dir: the JAR's name gives "2.5.0".
        assert_eq!(
            components(None, "2.5.0"),
            (Some("2.5.0".to_string()), Some(2.0))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_run_that_wrote_nothing_says_why() {
        let stderr = "Error: LinkageError occurred while loading main class app.freerouting.Freerouting\n\tjava.lang.UnsupportedClassVersionError: compiled by a more recent version (class file version 69.0)\n\n";
        let reason = no_ses_reason(Some(1), stderr);
        assert!(
            reason.starts_with(
                "no SES written (exit Some(1)): java.lang.UnsupportedClassVersionError"
            ),
            "{reason}"
        );
        assert_eq!(no_ses_reason(None, " \n"), "no SES written (exit None)");
    }

    #[test]
    fn the_default_corpus_is_the_fixture_dsns() {
        let files =
            dsn_files(&[crate::repo_root().join("crates/konnect-core/tests/fixtures")]).unwrap();
        assert!(files.len() >= 4, "{files:?}");
        assert!(files
            .iter()
            .all(|f| f.extension().is_some_and(|e| e == "dsn")));
        assert!(dsn_files(&[crate::repo_root().join("xtask/src")]).is_err());
    }
}
