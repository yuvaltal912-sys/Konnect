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
//! check (a GET to GitHub on every start) goes to a closed loopback port.
//!
//! `--config NAME=ARGS` routes and ranks named settings, not only JAR
//! versions, and `--score <spec>` judges every run on the real board against
//! its existing routing (see `board_score`).

pub mod board_score;
pub mod parse;
pub mod quality;
pub mod report;

use board_score::ScoreContext;
use parse::{
    dsn_connection_count, final_counts_from_log, ses_fingerprint, statistics_block, summarize_ses,
    version_from_file_name, version_from_log, Json,
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
       [--fr-arg <arg> ...] [--config <name>=<args> ...] [--score <spec.json>]
       [--max-jobs <n>] [--resume [--retry-failed]]
       cargo xtask freerouting-bench --rescore <results folder> [--score <spec.json>]

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
  --config        NAME=ARGS: a named set of router settings, routed and ranked
                  on its own; repeat for each. ARGS are split at spaces and may
                  only be --router.<path>=<value> or the router's short flags
                  (-mp -mt -is -us -oit -inc, each with a value). A config
                  that sets the passes or the router's threads replaces
                  --max-passes or --threads for its runs
  --score         judge every run on the real board against its existing
                  routing: refill and DRC with kicad-cli, the layout audit,
                  and track length by net category. The JSON spec names the
                  project folder, the baseline board and the tools (DEV.md)
  --max-jobs      refuse to start when this invocation's Freerouting runs, with
                  those already in --out's jobs.log, would exceed this many
  --resume        add to the results already in --out: saved runs are kept
                  and only the missing ones are routed. A config or a DSN that
                  is not the one its saved runs used is refused
  --retry-failed  with --resume: a saved run that failed (no session, settings
                  not in effect, not offline) and the runs after it are routed
                  again; a config all of whose runs failed may change
  --rescore <dir> re-measure a finished run's folder from its saved DSN, SES,
                  score files and results.json, without Java, KiCad or
                  Konnect, and rewrite its report. With --score, runs that
                  have no board score yet are scored now (KiCad and Konnect
                  run; Java does not)";

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
    /// Named settings to route and rank; none means one run set per JAR.
    pub configs: Vec<Config>,
    /// The spec for scoring every run on the real board.
    pub score: Option<PathBuf>,
    /// The most Freerouting runs `--out` may hold after this invocation.
    pub max_jobs: Option<u64>,
    /// Keep the finished runs already in `--out`.
    pub resume: bool,
    /// With `resume`: route again the saved runs that failed.
    pub retry_failed: bool,
    /// Re-measure a finished run's folder instead of routing.
    pub rescore: Option<PathBuf>,
}

/// A named set of router settings, routed and ranked on its own.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub name: String,
    pub args: Vec<String>,
}

/// The router's short flags a config may carry, each followed by one value.
const CONFIG_SHORT_FLAGS: [&str; 6] = ["-mp", "-mt", "-is", "-us", "-oit", "-inc"];

impl Config {
    /// `NAME=ARGS`. Only router settings are let through: `--router.<path>=<value>`
    /// and the short router flags. A config can therefore not turn telemetry
    /// on, open the GUI, or send a run's files out of its folder, and each
    /// setting appears once, so what a config sets is unambiguous.
    pub fn parse(text: &str) -> Result<Config, String> {
        let (name, rest) = text
            .split_once('=')
            .ok_or_else(|| format!("--config wants NAME=ARGS, not {text:?}"))?;
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        {
            return Err(format!(
                "a config name is letters, digits, '_', '-' and '.', not {name:?}"
            ));
        }
        let args: Vec<String> = rest.split_whitespace().map(str::to_string).collect();
        let mut seen: Vec<&str> = Vec::new();
        let mut at = 0;
        while at < args.len() {
            let arg = args[at].as_str();
            let key = if CONFIG_SHORT_FLAGS.contains(&arg) {
                at += 1;
                if args.get(at).is_none_or(|value| value.starts_with('-')) {
                    return Err(format!("config {name}: {arg} needs a value"));
                }
                arg
            } else {
                match arg.split_once('=') {
                    Some((key, value))
                        if key.starts_with("--router.")
                            && key.len() > "--router.".len()
                            && !value.is_empty() =>
                    {
                        key
                    }
                    _ => {
                        return Err(format!(
                            "config {name}: {arg:?} is not a router setting (--router.<path>=<value>, or one of {})",
                            CONFIG_SHORT_FLAGS.join(" ")
                        ))
                    }
                }
            };
            if key == "--router.job_timeout" {
                return Err(format!(
                    "config {name}: the job budget is --job-timeout-secs, the same for every config"
                ));
            }
            if seen.contains(&key) {
                return Err(format!("config {name}: {key} is set twice"));
            }
            seen.push(key);
            at += 1;
        }
        Ok(Config {
            name: name.to_string(),
            args,
        })
    }

    /// The config sets one of these flags or `--router.` paths itself.
    fn sets(&self, keys: &[&str]) -> bool {
        self.args.iter().any(|arg| {
            keys.iter().any(|key| {
                arg == key
                    || arg
                        .strip_prefix(key)
                        .is_some_and(|rest| rest.starts_with('='))
            })
        })
    }
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
        configs: Vec::new(),
        score: None,
        max_jobs: None,
        resume: false,
        retry_failed: false,
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
            "--config" => {
                let config = Config::parse(&value()?)?;
                if options.configs.iter().any(|c| c.name == config.name) {
                    return Err(format!("config {} is given twice", config.name));
                }
                options.configs.push(config);
            }
            "--score" => options.score = Some(value()?.into()),
            "--max-jobs" => options.max_jobs = Some(number(value()?, 1)?),
            "--resume" => options.resume = true,
            "--retry-failed" => options.retry_failed = true,
            "--rescore" => options.rescore = Some(value()?.into()),
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown option {other:?}\n{USAGE}")),
        }
    }
    if options.retry_failed && !options.resume {
        return Err(format!("--retry-failed goes with --resume\n{USAGE}"));
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
/// `run_dir/board.dsn` and writes `run_dir/board.ses`. A config's settings
/// come last, and a config that sets the passes or the router's threads
/// replaces the harness's own `-mp` or `-mt` (2.5 reads `-mt` as
/// `router.max_threads`), so no setting is given twice.
pub fn freerouting_arguments(
    run_dir: &Path,
    jar: &Path,
    options: &Options,
    config: Option<&Config>,
) -> Vec<String> {
    let sets = |keys: &[&str]| config.is_some_and(|config| config.sets(keys));
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
    ]);
    if !sets(&[
        "-mp",
        "--router.autorouter.max_passes",
        "--router.max_passes",
    ]) {
        args.extend(["-mp".to_string(), options.max_passes.to_string()]);
    }
    if !sets(&["-mt", "--router.max_threads"]) {
        args.extend(["-mt".to_string(), options.threads.to_string()]);
    }
    args.extend([
        format!("--router.job_timeout={}", hh_mm_ss(options.job_timeout)),
        "--gui.enabled=false".to_string(),
        "--profile.allow_telemetry=false".to_string(),
        "--usage_and_diagnostic_data.disable_analytics=true".to_string(),
    ]);
    args.extend(options.extra_args.iter().cloned());
    if let Some(config) = config {
        args.extend(config.args.iter().cloned());
    }
    args
}

/// What is known about the settings one run really used, from Freerouting's
/// own log. 2.3 and later write it under the run's home at debug level,
/// whatever the console shows; the `freerouting.json` they save back holds no
/// router settings, so it is no evidence.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SettingsEvidence {
    /// Freerouting's own complaints: a setting it could not apply, an
    /// argument or a property it does not know.
    pub failures: Vec<String>,
    /// Every "Applied CLI router setting" line, as path (after `router.`)
    /// and value.
    pub applied: Vec<(String, String)>,
    /// The routing parameters Freerouting printed for its routing: the via
    /// costs and each layer's trace costs, after its own adjustments.
    pub parameters: Vec<String>,
    /// `--router.` settings given to the run that were applied with another
    /// value, or that the log's list of applied settings leaves out.
    pub differing: Vec<String>,
    /// `--router.` settings given to a run whose log lists no applied
    /// settings at all (2.1), so they could not be checked.
    pub unconfirmed: Vec<String>,
}

impl SettingsEvidence {
    /// Why the run's settings cannot be trusted, if they cannot.
    pub fn problem(&self) -> Option<String> {
        let mut problems = self.failures.clone();
        problems.extend(self.differing.iter().cloned());
        (!problems.is_empty()).then(|| format!("settings not in effect: {}", problems.join("; ")))
    }
}

/// The lines in which Freerouting says a setting did not take. It logs them
/// as warnings and routes on with its default, so each one fails the run.
const SETTING_FAILURES: [&str; 4] = [
    "Failed to apply CLI router setting",
    "Unknown command line argument",
    "Unknown settings property",
    "Failed to set property value for",
];

/// Freerouting's complaints about settings in a run's output, each once.
pub fn setting_failures(log: &str) -> Vec<String> {
    let mut failures: Vec<String> = Vec::new();
    for line in log.lines() {
        if let Some(at) = SETTING_FAILURES.iter().find_map(|marker| line.find(marker)) {
            let message = line[at..].trim().to_string();
            if !failures.contains(&message) {
                failures.push(message);
            }
        }
    }
    failures
}

/// The settings Freerouting says it applied from the command line, as
/// (path after `router.`, value). A setting applied twice keeps its last value.
pub fn applied_settings(log: &str) -> Vec<(String, String)> {
    let mut applied: Vec<(String, String)> = Vec::new();
    for line in log.lines() {
        let Some((key, value)) = line
            .split_once("Applied CLI router setting: ")
            .and_then(|(_, rest)| rest.split_once(" = "))
        else {
            continue;
        };
        let key = key.trim();
        let key = key.strip_prefix("router.").unwrap_or(key).to_string();
        // `true (implicit from -de/-do batch mode)` is `true`.
        let value = value.split(" (").next().unwrap_or(value).trim().to_string();
        match applied.iter_mut().find(|(known, _)| *known == key) {
            Some(entry) => entry.1 = value,
            None => applied.push((key, value)),
        }
    }
    applied
}

/// The via and layer costs of the last "Routing Parameters" block in a log:
/// what the router really used, after Freerouting's own board-specific changes.
pub fn routing_parameters(log: &str) -> Vec<String> {
    let lines: Vec<&str> = log.lines().collect();
    let Some(start) = lines
        .iter()
        .rposition(|line| line.contains("=== Routing Parameters ==="))
    else {
        return Vec::new();
    };
    // The message is what follows the level: `<date> <time> DEBUG    Layer 1: …`.
    let message = |line: &str| {
        line.split_once("DEBUG")
            .map_or(line, |(_, rest)| rest)
            .trim()
            .to_string()
    };
    lines[start + 1..]
        .iter()
        .map(|line| message(line))
        .take_while(|text| {
            text.starts_with("Via Costs")
                || text.starts_with("Layer Costs")
                || text.starts_with("Layer ")
        })
        .filter(|text| !text.starts_with("Layer Costs"))
        .collect()
}

/// The `--router.<path>=<value>` settings among a run's arguments.
pub fn router_arguments(args: &[String]) -> Vec<(String, String)> {
    args.iter()
        .filter_map(|arg| arg.strip_prefix("--router."))
        .filter_map(|rest| rest.split_once('='))
        .map(|(path, value)| (path.to_string(), value.to_string()))
        .collect()
}

/// Two setting values that mean the same: equal numbers (`500` and `500.0`),
/// else the same text whatever its case.
fn same_setting(a: &str, b: &str) -> bool {
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(a), Ok(b)) => (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0),
        _ => a.eq_ignore_ascii_case(b),
    }
}

/// Judge a run's settings from its `log`: what Freerouting complained about,
/// and each `--router.` setting in `args` against the settings it says it
/// applied. A log that lists applied settings and leaves a given one out
/// means it was not applied; a log that lists none proves nothing either way.
pub fn settings_evidence(args: &[String], log: &str) -> SettingsEvidence {
    let mut evidence = SettingsEvidence {
        failures: setting_failures(log),
        applied: applied_settings(log),
        parameters: routing_parameters(log),
        ..SettingsEvidence::default()
    };
    for (path, value) in router_arguments(args) {
        match evidence
            .applied
            .iter()
            .find(|(applied, _)| *applied == path)
        {
            Some((_, used)) if same_setting(used, &value) => {}
            Some((_, used)) => evidence
                .differing
                .push(format!("router.{path} was applied as {used}, not {value}")),
            None if evidence.applied.is_empty() => {
                evidence.unconfirmed.push(format!("router.{path}={value}"))
            }
            None => evidence.differing.push(format!(
                "router.{path}={value} is not among the settings Freerouting says it applied"
            )),
        }
    }
    evidence
}

/// Everything the run in `run_dir` printed and logged.
fn run_output(run_dir: &Path) -> String {
    let mut log = std::fs::read_to_string(run_dir.join("stdout.log")).unwrap_or_default();
    log.push_str(&std::fs::read_to_string(run_dir.join("stderr.log")).unwrap_or_default());
    log.push_str(&logs_below(run_dir));
    log
}

/// Freerouting logs this when its version check got an answer from GitHub:
/// the run was not offline.
const NETWORK_REPLY: &str = "GitHub release check response";

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

/// The inputs of one run, written to `inputs.json` in its folder before
/// Freerouting starts: with the folder's own copy of the DSN, enough to make
/// the same run again. A run whose inputs cannot be recorded is not started.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunInputs {
    pub started_unix: u64,
    /// The machine's 1, 5 and 15 minute load averages as the run started.
    pub load_average: Option<String>,
    pub jar_sha256: String,
    pub dsn_sha256: String,
    /// The fixed wiring that came with the DSN, when it has any.
    pub fixed_ses_sha256: Option<String>,
    /// The commit this bench was built from, and whether `xtask/` had
    /// changes that were not committed.
    pub harness_commit: Option<String>,
    pub harness_dirty: bool,
    pub java_version: Option<String>,
    /// Everything after `java` on the command line.
    pub arguments: Vec<String>,
}

impl RunInputs {
    pub fn to_json(&self) -> Json {
        let text = |value: &Option<String>| value.clone().map_or(Json::Null, Json::Text);
        Json::Object(vec![
            (
                "started_unix".into(),
                Json::Number(self.started_unix as f64),
            ),
            ("load_average".into(), text(&self.load_average)),
            ("jar_sha256".into(), Json::Text(self.jar_sha256.clone())),
            ("dsn_sha256".into(), Json::Text(self.dsn_sha256.clone())),
            ("fixed_ses_sha256".into(), text(&self.fixed_ses_sha256)),
            ("harness_commit".into(), text(&self.harness_commit)),
            ("harness_dirty".into(), Json::Bool(self.harness_dirty)),
            ("java_version".into(), text(&self.java_version)),
            (
                "arguments".into(),
                Json::Array(self.arguments.iter().cloned().map(Json::Text).collect()),
            ),
        ])
    }

    pub fn from_json(json: &Json) -> Option<RunInputs> {
        Some(RunInputs {
            started_unix: json.number_at("started_unix")? as u64,
            load_average: text_of(json, "load_average"),
            jar_sha256: text_of(json, "jar_sha256")?,
            dsn_sha256: text_of(json, "dsn_sha256")?,
            fixed_ses_sha256: text_of(json, "fixed_ses_sha256"),
            harness_commit: text_of(json, "harness_commit"),
            harness_dirty: matches!(json.at("harness_dirty"), Some(Json::Bool(true))),
            java_version: text_of(json, "java_version"),
            arguments: texts_of(json, "arguments"),
        })
    }
}

/// Where a run keeps its inputs, in its own folder.
const RUN_INPUTS_FILE: &str = "inputs.json";

/// The SHA-256 of a file, from the system's `shasum` or `sha256sum` (the
/// xtask has no hashing dependency of its own).
pub fn sha256_of(path: &Path) -> Result<String, String> {
    let tools: [(&str, &[&str]); 2] = [("shasum", &["-a", "256"]), ("sha256sum", &[])];
    for (tool, arguments) in tools {
        let Ok(output) = Command::new(tool).args(arguments).arg(path).output() else {
            continue;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let digest = text.split_whitespace().next().unwrap_or_default();
        if output.status.success()
            && digest.len() == 64
            && digest.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Ok(digest.to_ascii_lowercase());
        }
    }
    Err(format!(
        "no SHA-256 of {} (shasum and sha256sum both failed)",
        path.display()
    ))
}

/// The 1, 5 and 15 minute load averages, as `1.40 1.14 1.11`.
fn load_average() -> Option<String> {
    let text = std::fs::read_to_string("/proc/loadavg").ok().or_else(|| {
        let output = Command::new("sysctl")
            .args(["-n", "vm.loadavg"])
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    })?;
    let numbers: Vec<&str> = text
        .split_whitespace()
        .filter(|part| part.parse::<f64>().is_ok())
        .take(3)
        .collect();
    (numbers.len() == 3).then(|| numbers.join(" "))
}

/// The commit of the checkout this bench was built from, and whether its
/// `xtask/` folder has changes that are not committed.
fn harness_state(repo: &Path) -> Option<(String, bool)> {
    let git = |arguments: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(arguments)
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    let commit = git(&["rev-parse", "HEAD"])?;
    let dirty = git(&["status", "--porcelain", "--", "xtask"]).is_none_or(|text| !text.is_empty());
    Some((commit, dirty))
}

/// What one invocation fixes for every run it starts.
struct Invocation {
    java: PathBuf,
    java_version: Option<String>,
    harness: Option<(String, bool)>,
}

/// Move what an earlier attempt left in `dir` aside, as
/// `<dir>.superseded-<unix time>`: a stopped or replaced run is kept as
/// evidence, and the new run starts in an empty folder.
fn set_aside(dir: &Path) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut attempt = 0;
    loop {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let target = dir.with_file_name(format!("{name}.superseded-{stamp}{suffix}"));
        if !target.exists() {
            return std::fs::rename(dir, target);
        }
        attempt += 1;
    }
}

/// One JAR with one config: the unit that is routed, totalled and ranked.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub jar: PathBuf,
    /// The folder its runs go in, different for every entry.
    pub label: String,
    pub config: Option<Config>,
}

/// Every JAR with every config, each with a folder label of its own:
/// the JAR's label alone without configs, else `<config>@<jar label>`.
pub fn entries(jars: &[PathBuf], configs: &[Config]) -> Vec<Entry> {
    let labels = jar_labels(jars);
    let mut entries = Vec::new();
    for (jar, jar_label) in jars.iter().zip(&labels) {
        if configs.is_empty() {
            entries.push(Entry {
                jar: jar.clone(),
                label: jar_label.clone(),
                config: None,
            });
        }
        for config in configs {
            entries.push(Entry {
                jar: jar.clone(),
                label: format!("{}@{jar_label}", config.name),
                config: Some(config.clone()),
            });
        }
    }
    entries
}

/// Why this entry and DSN may not add runs to `saved`, if they may not: the
/// saved runs were routed with other settings, or from another DSN of the
/// same name. Mixing them would make one candidate out of two. An entry with
/// no run left (every one failed and was dropped) may start over.
pub fn resume_conflict(
    saved: &BoardResult,
    entry: &Entry,
    fr_args: &[String],
    dsn_text: &str,
    saved_dsn: Option<&str>,
) -> Option<String> {
    if saved.runs.is_empty() {
        return None;
    }
    let args = entry.config.as_ref().map_or(&[][..], |c| &c.args[..]);
    if saved.config_args != args || saved.fr_args != fr_args {
        return Some(format!(
            "{} on {} has saved runs with other settings: give the changed config another name, or another --out (with --retry-failed a config may change once all its runs failed)",
            saved.dir, saved.board
        ));
    }
    if saved_dsn.is_some_and(|saved_dsn| saved_dsn != dsn_text) {
        return Some(format!(
            "{}.dsn is not the DSN the saved runs of {} used: give the changed DSN another name, or another --out",
            saved.board, saved.dir
        ));
    }
    None
}

/// Drop a saved entry's first failed run and every run after it, so they are
/// routed again in order. Returns how many were dropped.
pub fn drop_failed_runs(saved: &mut BoardResult) -> usize {
    let keep = saved
        .runs
        .iter()
        .position(|run| !run.succeeded())
        .unwrap_or(saved.runs.len());
    let dropped = saved.runs.len() - keep;
    saved.runs.truncate(keep);
    dropped
}

/// The Freerouting starts recorded in `out/jobs.log`, one line each.
fn jobs_started(out: &Path) -> u64 {
    std::fs::read_to_string(out.join("jobs.log"))
        .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count() as u64)
        .unwrap_or(0)
}

/// Refuse a plan that would take `--out` past `--max-jobs`.
pub fn check_job_budget(started: u64, planned: u64, max_jobs: Option<u64>) -> Result<(), String> {
    match max_jobs {
        Some(max) if started + planned > max => Err(format!(
            "{planned} Freerouting runs planned and {started} already in jobs.log: more than --max-jobs {max}"
        )),
        _ => Ok(()),
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let options = parse_options(args)?;
    if let Some(dir) = &options.rescore {
        return rescore(dir, options.score.as_deref());
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
    let has_results = out.join("results.json").is_file();
    if has_results && !options.resume {
        return Err(format!(
            "{} already holds results: pass --resume to add to them, or another --out",
            out.display()
        ));
    }
    if let (true, None, Some(spec)) = (has_results, &options.score, saved_score_spec(&out)) {
        return Err(format!(
            "the results in {} were scored on the board: pass --score {} to add to them",
            out.display(),
            spec.display()
        ));
    }
    let java_version = java_version(&java);
    let jars = options
        .jars
        .iter()
        .map(|jar| jar.canonicalize().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let entries = entries(&jars, &options.configs);
    let invocation = Invocation {
        java: java.clone(),
        java_version: java_version.clone(),
        harness: harness_state(&root),
    };
    let jar_digests = jars
        .iter()
        .map(|jar| sha256_of(jar).map(|digest| (jar.clone(), digest)))
        .collect::<Result<std::collections::HashMap<_, _>, _>>()?;

    let context = match &options.score {
        Some(spec) => {
            eprintln!("freerouting-bench: scoring the baseline board");
            let context = ScoreContext::prepare(spec, &root, &out, options.resume)?;
            if let Err(error) = &context.baseline {
                return Err(format!("the baseline board could not be scored: {error}"));
            }
            Some(context)
        }
        None => None,
    };
    let mut results: Vec<BoardResult> = if has_results {
        load_saved(&out, context.as_ref())?.results
    } else {
        Vec::new()
    };
    if let Some(context) = &context {
        if score_missing(&mut results, context, &out) > 0 {
            save(
                &out,
                &results,
                &options,
                java_version.as_deref(),
                Some(context),
            )?;
        }
    }

    let board_name = |dsn: &Path| {
        dsn.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    };
    // Saved runs are only added to by the same settings on the same DSN.
    for entry in &entries {
        for dsn in &boards {
            let board = board_name(dsn);
            let Some(saved) = results
                .iter_mut()
                .find(|r| r.dir == entry.label && r.board == board)
            else {
                continue;
            };
            if options.retry_failed {
                let dropped = drop_failed_runs(saved);
                if dropped > 0 {
                    eprintln!(
                        "  {} {board}: {dropped} failed run(s) will be routed again",
                        entry.label
                    );
                }
            }
            let saved_dsn =
                std::fs::read_to_string(out.join(&entry.label).join(&board).join("run1/board.dsn"))
                    .ok();
            let dsn_text = std::fs::read_to_string(dsn).unwrap_or_default();
            if let Some(conflict) = resume_conflict(
                saved,
                entry,
                &options.extra_args,
                &dsn_text,
                saved_dsn.as_deref(),
            ) {
                return Err(conflict);
            }
        }
    }
    let done = |results: &[BoardResult], label: &str, board: &str| {
        results
            .iter()
            .find(|r| r.dir == label && r.board == board)
            .map_or(0, |r| r.runs.len() as u32)
    };
    let planned: u64 = entries
        .iter()
        .flat_map(|entry| boards.iter().map(move |dsn| (entry, dsn)))
        .map(|(entry, dsn)| {
            u64::from(
                options
                    .runs
                    .saturating_sub(done(&results, &entry.label, &board_name(dsn))),
            )
        })
        .sum();
    check_job_budget(jobs_started(&out), planned, options.max_jobs)?;
    eprintln!(
        "freerouting-bench: {} boards x {} JARs x {} configs x {} runs = {planned} runs to route, java {}, results in {}",
        boards.len(),
        options.jars.len(),
        options.configs.len().max(1),
        options.runs,
        java_version.as_deref().unwrap_or("(version unknown)"),
        out.display()
    );

    for entry in &entries {
        let label = &entry.label;
        let file_version = entry
            .jar
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(version_from_file_name);
        for dsn in &boards {
            let board = board_name(dsn);
            let dsn_text = std::fs::read_to_string(dsn).unwrap_or_default();
            let dsn_board = quality::DsnBoard::parse(&dsn_text);
            let fresh = || BoardResult {
                board: board.clone(),
                dir: label.clone(),
                version: file_version
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                jar: entry.jar.clone(),
                config: entry.config.as_ref().map(|c| c.name.clone()),
                config_args: entry.config.as_ref().map_or(Vec::new(), |c| c.args.clone()),
                fr_args: options.extra_args.clone(),
                dsn_connections: dsn_connection_count(&dsn_text),
                components: dsn_board.as_ref().ok().map(|b| b.components),
                signal_layers: dsn_board.as_ref().ok().map(|b| b.signal_layers),
                runs: Vec::new(),
            };
            let slot = match results
                .iter()
                .position(|r| &r.dir == label && r.board == board)
            {
                // A saved entry with no run left starts over as this one.
                Some(slot) if results[slot].runs.is_empty() => {
                    results[slot] = fresh();
                    slot
                }
                Some(slot) => slot,
                None => {
                    results.push(fresh());
                    results.len() - 1
                }
            };
            for index in (results[slot].runs.len() as u32 + 1)..=options.runs {
                let dir = out.join(label).join(&board).join(format!("run{index}"));
                record_job(&out, label, &board, index);
                let mut record = run_once(
                    &invocation,
                    (&entry.jar, &jar_digests[&entry.jar]),
                    dsn,
                    dsn_board.as_ref().ok(),
                    &dir,
                    &options,
                    entry.config.as_ref(),
                );
                if let Some(context) = &context {
                    score_run(&mut record, context, &dir, dsn_board.as_ref().ok());
                }
                eprintln!(
                    "  {label:>8} {board:<48} run {index}: {}",
                    record.one_line()
                );
                if let Some(version) = record.reported_version.clone() {
                    results[slot].version = version;
                }
                results[slot].runs.push(record);
                // After every run, so a stopped invocation can be resumed.
                save(
                    &out,
                    &results,
                    &options,
                    java_version.as_deref(),
                    context.as_ref(),
                )?;
            }
        }
    }

    let summary = save(
        &out,
        &results,
        &options,
        java_version.as_deref(),
        context.as_ref(),
    )?;
    println!("{summary}");
    eprintln!("wrote {}", out.join("results.json").display());
    Ok(())
}

/// Write `results.json` and `summary.md` into `out`; returns the summary.
fn save(
    out: &Path,
    results: &[BoardResult],
    options: &Options,
    java: Option<&str>,
    context: Option<&ScoreContext>,
) -> Result<String, String> {
    let summary = report::render_markdown(results, options, java, context);
    let json = report::to_json(&report::results_json(results, options, java, context));
    // Written beside and moved into place: a bench stopped in the middle of a
    // write must not leave half a results.json, which holds what no file in
    // the run folders does (exit, runtime, Freerouting's own counts).
    for (name, text) in [("results.json", &json), ("summary.md", &summary)] {
        let partial = out.join(format!("{name}.partial"));
        std::fs::write(&partial, text)
            .and_then(|()| std::fs::rename(&partial, out.join(name)))
            .map_err(|e| format!("cannot write {}: {e}", out.join(name).display()))?;
    }
    Ok(summary)
}

/// Note one Freerouting start in `out/jobs.log`, which `--max-jobs` counts.
fn record_job(out: &Path, label: &str, board: &str, index: u32) {
    use std::io::Write;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut log) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out.join("jobs.log"))
    {
        let _ = writeln!(log, "{stamp} {label} {board} run{index}");
    }
}

/// The session files that make up a run's routing: wiring the DSN already
/// held as fixed (`fixed.ses`, which Freerouting does not write back) first,
/// then what the run routed.
fn run_sessions(dir: &Path) -> Vec<PathBuf> {
    ["fixed.ses", "board.ses"]
        .iter()
        .map(|name| dir.join(name))
        .filter(|path| path.is_file())
        .collect()
}

/// Score one run on the real board, when it wrote a session at all.
fn score_run(
    record: &mut RunRecord,
    context: &ScoreContext,
    dir: &Path,
    dsn: Option<&quality::DsnBoard>,
) {
    if !dir.join("board.ses").is_file() {
        return;
    }
    match context.score_run(&dir.join("score"), &run_sessions(dir), dsn) {
        Ok(score) => record.judge(score, context),
        Err(error) => record.score_error = Some(error),
    }
}

fn java_version(java: &Path) -> Option<String> {
    let output = Command::new(java).arg("-version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stderr).to_string()
        + &String::from_utf8_lossy(&output.stdout);
    text.split('"').nth(1).map(str::to_string)
}

fn run_once(
    invocation: &Invocation,
    (jar, jar_sha256): (&Path, &str),
    dsn: &Path,
    board: Option<&quality::DsnBoard>,
    dir: &Path,
    options: &Options,
    config: Option<&Config>,
) -> RunRecord {
    let java = invocation.java.as_path();
    let mut record = RunRecord::default();
    // A folder left by a run that was stopped is kept under another name, but
    // its Freerouting may still be running and would write into the new run.
    stop_leftover_run(dir);
    if let Err(error) = set_aside(dir) {
        record.error = Some(format!(
            "cannot set the earlier {} aside: {error}",
            dir.display()
        ));
        return record;
    }
    // Wiring the DSN holds as fixed comes with it as `<name>.fixed.ses`.
    let fixed = dsn.with_extension("fixed.ses");
    let prepared = SETTINGS_PLACES
        .iter()
        .try_for_each(|place| {
            std::fs::create_dir_all(dir.join(place))?;
            std::fs::write(dir.join(place).join("freerouting.json"), SEEDED_SETTINGS)
        })
        .and_then(|()| std::fs::copy(dsn, dir.join("board.dsn")).map(drop))
        .and_then(|()| match fixed.is_file() {
            true => std::fs::copy(&fixed, dir.join("fixed.ses")).map(drop),
            false => Ok(()),
        });
    if let Err(error) = prepared {
        record.error = Some(format!("cannot prepare {}: {error}", dir.display()));
        return record;
    }
    let ses = dir.join("board.ses");
    let stdout_path = dir.join("stdout.log");
    let stderr_path = dir.join("stderr.log");
    let (Ok(stdout), Ok(stderr)) = (
        std::fs::File::create(&stdout_path),
        std::fs::File::create(&stderr_path),
    ) else {
        record.error = Some("cannot create the run's log files".to_string());
        return record;
    };
    let arguments = freerouting_arguments(dir, jar, options, config);
    // The inputs are on record before anything starts, or nothing starts.
    let inputs = sha256_of(&dir.join("board.dsn")).and_then(|dsn_sha256| {
        let fixed_ses = dir.join("fixed.ses");
        let fixed_ses_sha256 = match fixed_ses.is_file() {
            true => Some(sha256_of(&fixed_ses)?),
            false => None,
        };
        let inputs = RunInputs {
            started_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            load_average: load_average(),
            jar_sha256: jar_sha256.to_string(),
            dsn_sha256,
            fixed_ses_sha256,
            harness_commit: invocation
                .harness
                .as_ref()
                .map(|(commit, _)| commit.clone()),
            harness_dirty: invocation.harness.as_ref().is_none_or(|(_, dirty)| *dirty),
            java_version: invocation.java_version.clone(),
            arguments: arguments.clone(),
        };
        std::fs::write(
            dir.join(RUN_INPUTS_FILE),
            report::to_json(&inputs.to_json()),
        )
        .map_err(|e| e.to_string())?;
        Ok(inputs)
    });
    match inputs {
        Ok(inputs) => record.inputs = Some(inputs),
        Err(error) => {
            record.error = Some(format!("the run's inputs could not be recorded: {error}"));
            return record;
        }
    }
    let started = Instant::now();
    let mut command = Command::new(java);
    for name in freerouting_env_removed(
        std::env::vars_os().map(|(name, _)| name.to_string_lossy().into_owned()),
    ) {
        command.env_remove(name);
    }
    let child = command
        .args(&arguments)
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
    let _ = std::fs::write(dir.join(RUN_PID_FILE), child.id().to_string());
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
    apply_run_evidence(&mut record, dir, &arguments);
    record
}

/// Where a run notes the process id of its Freerouting, in its own folder.
const RUN_PID_FILE: &str = "java.pid";

/// Stop the Freerouting a stopped invocation left running in `dir`. When the
/// bench itself is killed, the JVM it started lives on and would later write
/// its session into the folder a resumed run is using. The process is only
/// stopped when its command line names this run folder, so a process id that
/// was reused by something else is left alone.
fn stop_leftover_run(dir: &Path) {
    let Some(pid) = std::fs::read_to_string(dir.join(RUN_PID_FILE))
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok())
    else {
        return;
    };
    let command_line = || {
        Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "command="])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).to_string())
            .unwrap_or_default()
    };
    let ours = |line: &str| line.contains(&dir.display().to_string());
    if !ours(&command_line()) {
        return;
    }
    eprintln!(
        "  stopping the Freerouting a stopped run left in {}",
        dir.display()
    );
    let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    let waited = Instant::now();
    while ours(&command_line()) && waited.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Attach what the run in `run_dir` left as evidence: its settings, judged
/// against the arguments it was given, and whether it stayed offline.
fn apply_run_evidence(record: &mut RunRecord, run_dir: &Path, args: &[String]) {
    let output = run_output(run_dir);
    apply_settings_evidence(record, settings_evidence(args, &output));
    if output.contains(NETWORK_REPLY) {
        record.reached_network = true;
        record.error.get_or_insert(
            "the run was not offline: Freerouting's version check got an answer".to_string(),
        );
    }
}

/// Attach a run's settings evidence. A setting Freerouting did not take
/// fails the run: its routing was made with other settings than its name says.
fn apply_settings_evidence(record: &mut RunRecord, evidence: SettingsEvidence) {
    if let Some(problem) = evidence.problem() {
        record.error.get_or_insert(problem);
    }
    record.settings = Some(evidence);
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

/// A finished run's folder, read back.
struct Saved {
    results: Vec<BoardResult>,
    options: Options,
    java: Option<String>,
}

fn text_of(value: &Json, key: &str) -> Option<String> {
    match value.at(key) {
        Some(Json::Text(s)) => Some(s.clone()),
        _ => None,
    }
}

fn texts_of(value: &Json, key: &str) -> Vec<String> {
    match value.at(key) {
        Some(Json::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Json::Text(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The score spec a finished run was judged with, as its `results.json` names it.
fn saved_score_spec(dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(dir.join("results.json")).ok()?;
    let (saved, _) = parse::parse_json_prefix(&text)?;
    text_of(&saved, "score_spec").map(PathBuf::from)
}

/// Read a finished run from `dir/results.json` and each run folder's files.
/// What Freerouting reported (exit, runtime, unrouted, violations) is kept;
/// everything measured from files is redone: the SES, the settings evidence
/// and, with a `context`, the board score from the run's `score/` files.
fn load_saved(dir: &Path, context: Option<&ScoreContext>) -> Result<Saved, String> {
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
    options.score = text_of(&saved, "score_spec").map(PathBuf::from);
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
        let config = text_of(saved_board, "config");
        let config_args = texts_of(saved_board, "config_args");
        let fr_args = texts_of(saved_board, "fr_args");
        let Some(Json::Array(saved_runs)) = saved_board.at("runs") else {
            continue;
        };
        // The settings each run was given, for its evidence.
        let mut passed = fr_args.clone();
        passed.extend(config_args.iter().cloned());
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
                // An error from reading the SES, the settings or the log is
                // redone; any other stays.
                error: text_of(saved_run, "error").filter(|e| {
                    !e.starts_with("unreadable SES")
                        && !e.starts_with("settings not in effect")
                        && !e.starts_with("the run was not offline")
                }),
                ..RunRecord::default()
            };
            if let Ok(ses) = std::fs::read_to_string(run_dir.join("board.ses")) {
                read_ses(&mut record, &ses, dsn_board.as_ref());
            }
            if run_dir.join("stdout.log").is_file() {
                apply_run_evidence(&mut record, &run_dir, &passed);
            }
            record.inputs = std::fs::read_to_string(run_dir.join(RUN_INPUTS_FILE))
                .ok()
                .and_then(|text| parse::parse_json_prefix(&text))
                .and_then(|(json, _)| RunInputs::from_json(&json));
            let score_dir = run_dir.join("score");
            match context {
                Some(context) if score_dir.join("audit.json").is_file() => {
                    match board_score::BoardScore::read(
                        &score_dir,
                        &context.roles(dsn_board.as_ref()),
                    ) {
                        Ok(score) => record.judge(score, context),
                        Err(error) => record.score_error = Some(error),
                    }
                }
                _ => record.score_error = text_of(saved_run, "score_error"),
            }
            runs.push(record);
        }
        results.push(BoardResult {
            board,
            dir: label,
            version,
            jar,
            config,
            config_args,
            fr_args,
            dsn_connections: saved_board.number_at("dsn_connections").map(|n| n as usize),
            components: dsn_board.as_ref().map(|b| b.components),
            signal_layers: dsn_board.as_ref().map(|b| b.signal_layers),
            runs,
        });
    }
    Ok(Saved {
        results,
        options,
        java,
    })
}

/// Score on the board every saved run that has a session file and no board
/// score yet (a bench that ran without `--score`, or a scoring step that
/// failed). KiCad and Konnect run; Freerouting does not. Returns how many
/// runs were tried.
fn score_missing(results: &mut [BoardResult], context: &ScoreContext, out: &Path) -> usize {
    let mut tried = 0;
    for result in results.iter_mut() {
        let folder = out.join(&result.dir).join(&result.board);
        let dsn = std::fs::read_to_string(folder.join("run1/board.dsn"))
            .ok()
            .and_then(|text| quality::DsnBoard::parse(&text).ok());
        for (index, record) in result.runs.iter_mut().enumerate() {
            let dir = folder.join(format!("run{}", index + 1));
            if record.board_score.is_none() && dir.join("board.ses").is_file() {
                eprintln!(
                    "  scoring {} {} run {}",
                    result.dir,
                    result.board,
                    index + 1
                );
                score_run(record, context, &dir, dsn.as_ref());
                tried += 1;
            }
        }
    }
    tried
}

/// Re-measure a finished run from `dir/results.json` and each run folder's
/// saved files, and rewrite its report. Nothing is routed. With `score` (a
/// spec given with `--score`), runs that have no board score yet are scored
/// now; without it, the saved score files are only read again.
pub fn rescore(dir: &Path, score: Option<&Path>) -> Result<(), String> {
    let root = crate::repo_root();
    // Absolute, because the scoring tools run with a run's folder as theirs.
    let dir = &dir
        .canonicalize()
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let context = match (score, saved_score_spec(dir)) {
        (Some(spec), _) => {
            let context = ScoreContext::prepare(spec, &root, dir, true)?;
            if let Err(error) = &context.baseline {
                return Err(format!("the baseline board could not be scored: {error}"));
            }
            Some(context)
        }
        // Rewriting the report without the scores it had would drop them
        // from results.json for good, so that is refused, not done quietly.
        (None, Some(spec)) => Some(ScoreContext::reload(&spec, &root, dir).map_err(|error| {
            format!(
                "these results were scored on the board, and the scores cannot be read again: {error}. Restore {} or pass --score <spec.json>",
                spec.display()
            )
        })?),
        (None, None) => None,
    };
    let mut saved = load_saved(dir, context.as_ref())?;
    if let (Some(_), Some(context)) = (score, &context) {
        score_missing(&mut saved.results, context, dir);
    }
    let summary = save(
        dir,
        &saved.results,
        &saved.options,
        saved.java.as_deref(),
        context.as_ref(),
    )?;
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
        let line = freerouting_arguments(Path::new("/run"), Path::new("/j/fr.jar"), &options, None);
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
    fn a_config_holds_named_router_settings_and_nothing_else() {
        let config =
            Config::parse("S2=--router.optimizer.improvement_threshold=0.25 -is prioritized")
                .unwrap();
        assert_eq!(config.name, "S2");
        assert_eq!(
            config.args,
            [
                "--router.optimizer.improvement_threshold=0.25",
                "-is",
                "prioritized"
            ]
        );
        // No settings at all is a config too: the harness's own defaults.
        assert!(Config::parse("plain=").unwrap().args.is_empty());
        let refused = |text: &str| Config::parse(text).unwrap_err();
        assert!(refused("S2").contains("NAME=ARGS"));
        assert!(refused("=--router.x=1").contains("config name"));
        assert!(refused("a b=--router.x=1").contains("config name"));
        // Nothing but the router: a config cannot turn telemetry on, open the
        // GUI, move the run's files, or reach the JVM.
        for outside in [
            "--profile.allow_telemetry=true",
            "--usage_and_diagnostic_data.disable_analytics=false",
            "--gui.enabled=true",
            "--user_data_path=/somewhere",
            "--logging.file.location=/somewhere/x.log",
            "-Duser.home=/somewhere",
            "-de",
            // Not a flag 2.5.0 knows, and nothing a sweep needs.
            "-hr",
            "-ll",
            "--router.",
            "--router.x",
            "--router.x=",
            "--router.=1",
        ] {
            let error = refused(&format!("S={outside}"));
            assert!(error.contains("not a router setting"), "{outside}: {error}");
        }
        assert!(refused("S=--router.job_timeout=00:20:00").contains("--job-timeout-secs"));
        assert!(refused("S=-mp").contains("needs a value"));
        assert!(refused("S=-mp --router.x=1").contains("needs a value"));
        // Each setting once, so what a config sets is not a matter of order.
        assert!(refused("S=--router.x=1 --router.x=2").contains("set twice"));
        assert!(refused("S=-mp 5 -mp 6").contains("set twice"));
    }

    #[test]
    fn configs_the_score_spec_the_job_cap_and_resume_are_options() {
        let options = parse_options(&args(&[
            "--jar",
            "a.jar",
            "--config",
            "S0=--router.fanout.enabled=false",
            "--config",
            "S1=-mp 20",
            "--score",
            "target/spec.json",
            "--max-jobs",
            "60",
            "--resume",
        ]))
        .unwrap();
        let names: Vec<&str> = options.configs.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["S0", "S1"]);
        assert_eq!(options.score, Some(PathBuf::from("target/spec.json")));
        assert_eq!(options.max_jobs, Some(60));
        assert!(options.resume);
        let plain = parse_options(&args(&["--jar", "a.jar"])).unwrap();
        assert!(plain.configs.is_empty() && plain.max_jobs.is_none() && !plain.resume);
        // Two configs with one name would share a folder.
        assert!(parse_options(&args(&[
            "--jar", "a", "--config", "S=", "--config", "S=-mp 5"
        ]))
        .unwrap_err()
        .contains("given twice"));
        assert!(parse_options(&args(&["--jar", "a", "--config", "S=--gui.enabled=true"])).is_err());
        assert!(parse_options(&args(&["--jar", "a", "--max-jobs", "0"])).is_err());
    }

    #[test]
    fn a_configs_settings_come_last_and_replace_the_harness_passes_and_threads() {
        let options = parse_options(&args(&["--jar", "fr.jar", "--fr-arg", "--extra"])).unwrap();
        let line = |config: &str| {
            freerouting_arguments(
                Path::new("/run"),
                Path::new("/j/fr.jar"),
                &options,
                Some(&Config::parse(config).unwrap()),
            )
        };
        let has = |line: &[String], flag: &str| line.iter().any(|a| a == flag);
        // A config that sets neither keeps the harness's own -mp and -mt.
        let other = line("S=--router.fanout.enabled=false");
        assert!(has(&other, "-mp") && has(&other, "-mt"));
        assert_eq!(other.last().unwrap(), "--router.fanout.enabled=false");
        assert_eq!(other[other.len() - 2], "--extra");
        // One that sets the passes drops -mp, by either name or the short flag.
        for passes in [
            "S=--router.autorouter.max_passes=20",
            "S=--router.max_passes=20",
            "S=-mp 20",
        ] {
            let line = line(passes);
            assert_eq!(
                line.iter().filter(|a| *a == "-mp").count(),
                usize::from(passes.contains("-mp"))
            );
            assert!(has(&line, "-mt"), "{passes}");
        }
        // Likewise the router's threads, which is what 2.5 reads -mt as. The
        // optimizer's threads are another setting and leave -mt in place.
        let threads = line("S=--router.max_threads=1");
        assert!(!has(&threads, "-mt") && has(&threads, "-mp"));
        assert!(has(&line("S=--router.optimizer.max_threads=14"), "-mt"));
        // A setting that only starts like one of those does not count.
        assert!(has(
            &line("S=--router.autorouter.max_passes_extra=1"),
            "-mp"
        ));
        // Whatever the config, every run stays quiet and in its folder.
        for expected in [
            "--gui.enabled=false",
            "--profile.allow_telemetry=false",
            "--usage_and_diagnostic_data.disable_analytics=true",
            "--router.job_timeout=00:01:00",
            "-Duser.home=/run/home",
        ] {
            assert!(has(&threads, expected), "{expected}");
        }
    }

    #[test]
    fn every_jar_and_config_gets_a_folder_of_its_own() {
        let jars: Vec<PathBuf> = ["/a/freerouting-2.5.0.jar", "/b/freerouting-2.5.0.jar"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let configs = [
            Config::parse("S0=").unwrap(),
            Config::parse("S1=-mp 5").unwrap(),
        ];
        let labels: Vec<String> = entries(&jars, &configs)
            .into_iter()
            .map(|e| e.label)
            .collect();
        assert_eq!(labels, ["S0@2.5.0", "S1@2.5.0", "S0@2.5.0-2", "S1@2.5.0-2"]);
        // Without configs an entry is its JAR, as before.
        let plain = entries(&jars, &[]);
        assert_eq!(
            (plain[0].label.as_str(), plain[1].label.as_str()),
            ("2.5.0", "2.5.0-2")
        );
        assert!(plain.iter().all(|e| e.config.is_none()));
    }

    #[test]
    fn the_job_cap_counts_the_runs_already_made() {
        assert!(check_job_budget(0, 60, Some(60)).is_ok());
        assert!(check_job_budget(26, 34, Some(60)).is_ok());
        let over = check_job_budget(26, 35, Some(60)).unwrap_err();
        assert!(
            over.contains("35") && over.contains("26") && over.contains("60"),
            "{over}"
        );
        // No cap asked for, no cap.
        assert!(check_job_budget(500, 500, None).is_ok());
        // The ledger is one line per start.
        let dir = std::env::temp_dir().join(format!("konnect-bench-jobs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(jobs_started(&dir), 0);
        record_job(&dir, "S0@2.5.0", "V0", 1);
        record_job(&dir, "S0@2.5.0", "V0", 2);
        assert_eq!(jobs_started(&dir), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_setting_freerouting_did_not_take_fails_the_run() {
        let log = "2026-11-10 10:00:00.000 INFO   Freerouting v2.5.0 (build-date: 2026-10-05)\n\
                   2026-11-10 10:00:00.100 WARN   Failed to apply CLI router setting: router.optimizer.max_pases: no such setting\n\
                   2026-11-10 10:00:00.100 WARN   Failed to apply CLI router setting: router.optimizer.max_pases: no such setting\n\
                   2026-11-10 10:00:00.200 WARN   Unknown command line argument: --rooter.x=1\n";
        assert_eq!(
            setting_failures(log),
            [
                "Failed to apply CLI router setting: router.optimizer.max_pases: no such setting",
                "Unknown command line argument: --rooter.x=1"
            ]
        );
        assert!(
            setting_failures("INFO   Opening 'board.dsn'...\nWARN   Deprecated settings path")
                .is_empty()
        );
        let mut record = RunRecord::default();
        apply_settings_evidence(&mut record, settings_evidence(&[], log));
        assert!(record
            .error
            .as_deref()
            .unwrap()
            .starts_with("settings not in effect: Failed to apply"));
        assert!(!record.succeeded());
        // An earlier error is kept; the evidence is recorded either way.
        let mut failed = RunRecord {
            error: Some("no SES written".into()),
            ..RunRecord::default()
        };
        apply_settings_evidence(&mut failed, settings_evidence(&[], log));
        assert_eq!(failed.error.as_deref(), Some("no SES written"));
        assert_eq!(failed.settings.unwrap().failures.len(), 2);
        // Clean output and no settings: nothing to complain about.
        let mut clean = RunRecord::default();
        apply_settings_evidence(&mut clean, settings_evidence(&[], "INFO fine"));
        assert_eq!(clean.error, None);
    }

    /// The lines of a real 2.5.0 file log (2026-11-08), which holds debug
    /// lines whatever the console level is.
    const APPLIED_LOG: &str = "\
2026-11-08 18:22:28.653 WARN   Deprecated settings path '-mp / --router.max_passes'; use '--router.autorouter.max_passes' instead.
2026-11-08 18:22:29.735 DEBUG  Applied CLI router setting: router.max_passes = 100
2026-11-08 18:22:29.735 DEBUG  Applied CLI router setting: router.max_threads = 1
2026-11-08 18:22:29.735 DEBUG  Applied CLI router setting: router.job_timeout = 00:02:00
2026-11-08 18:22:29.735 DEBUG  Applied CLI router setting: router.neck_width_um = 200
2026-11-08 18:22:29.735 DEBUG  Applied CLI router setting: router.automatic_neckdown = false
2026-11-08 18:22:29.735 DEBUG  Applied CLI router setting: router.autorouter.enabled = true (implicit from -de/-do batch mode)
2026-11-08 18:22:30.100 DEBUG  === Performance Profile ===
2026-11-08 18:22:30.100 DEBUG  === Routing Parameters ===
2026-11-08 18:22:30.100 DEBUG    Via Costs: 50, Plane Via Costs: 5
2026-11-08 18:22:30.100 DEBUG    Layer Costs (Preferred / Against):
2026-11-08 18:22:30.100 DEBUG      Layer 1: 1.0 / 2.2
2026-11-08 18:22:30.100 DEBUG      Layer 2: 1.0 / 1.8
2026-11-08 18:22:30.100 DEBUG  
2026-11-08 18:22:30.100 DEBUG    autoroute_item                          :        4 ms total,        2 calls,      2 ms avg
";

    #[test]
    fn given_settings_are_checked_against_what_freerouting_says_it_applied() {
        let applied = applied_settings(APPLIED_LOG);
        let get = |path: &str| {
            applied
                .iter()
                .find(|(p, _)| p == path)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("max_passes"), Some("100"));
        assert_eq!(get("job_timeout"), Some("00:02:00"));
        // The note after an implicit setting is not part of its value.
        assert_eq!(get("autorouter.enabled"), Some("true"));
        assert_eq!(applied.len(), 6);
        // A setting applied twice keeps its last value.
        let twice =
            format!("{APPLIED_LOG}x DEBUG  Applied CLI router setting: router.max_threads = 4\n");
        assert_eq!(applied_settings(&twice).len(), 6);
        assert!(applied_settings(&twice).contains(&("max_threads".to_string(), "4".to_string())));

        let given = args(&[
            "-mp",
            "100",
            "--router.neck_width_um=200.0",
            "--router.automatic_neckdown=FALSE",
            "--router.max_threads=4",
            "--router.scoring.via_costs=25",
            "--router.job_timeout=00:02:00",
            "--gui.enabled=false",
        ]);
        let evidence = settings_evidence(&given, APPLIED_LOG);
        // 200 and 200.0 are one value, and so are false and FALSE. A setting
        // applied with another value, and one the list leaves out although
        // the log does list applied settings, were not in effect as given.
        assert_eq!(
            evidence.differing,
            [
                "router.max_threads was applied as 1, not 4",
                "router.scoring.via_costs=25 is not among the settings Freerouting says it applied"
            ]
        );
        assert!(evidence.unconfirmed.is_empty());
        assert!(evidence.problem().unwrap().contains("applied as 1, not 4"));
        // All given settings applied as given: nothing to report.
        let fine = settings_evidence(
            &args(&[
                "--router.neck_width_um=200",
                "--router.job_timeout=00:02:00",
            ]),
            APPLIED_LOG,
        );
        assert_eq!((fine.problem(), fine.unconfirmed.len()), (None, 0));
        // A log that lists no applied settings (2.1) proves nothing: the
        // given settings are unconfirmed, and that is said, not passed over.
        let blind = settings_evidence(&given, "INFO   Freerouting v2.1.0");
        assert!(blind.differing.is_empty());
        assert_eq!(blind.unconfirmed.len(), 5);
        assert_eq!(blind.problem(), None);
    }

    #[test]
    fn the_routing_parameters_freerouting_used_are_kept() {
        assert_eq!(
            routing_parameters(APPLIED_LOG),
            [
                "Via Costs: 50, Plane Via Costs: 5",
                "Layer 1: 1.0 / 2.2",
                "Layer 2: 1.0 / 1.8"
            ]
        );
        // The last block is the one that routed.
        let later = format!(
            "{APPLIED_LOG}x DEBUG  === Routing Parameters ===\nx DEBUG    Via Costs: 25, Plane Via Costs: 5\nx DEBUG  \n"
        );
        assert_eq!(
            routing_parameters(&later),
            ["Via Costs: 25, Plane Via Costs: 5"]
        );
        assert!(routing_parameters("INFO   nothing of the kind").is_empty());
        assert_eq!(settings_evidence(&[], APPLIED_LOG).parameters.len(), 3);
    }

    #[test]
    fn a_run_whose_version_check_was_answered_is_not_offline() {
        let dir = std::env::temp_dir().join(format!("konnect-bench-net-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home/Library/Logs/freerouting")).unwrap();
        std::fs::write(dir.join("stdout.log"), "INFO   Freerouting v2.5.0\n").unwrap();
        let log = dir.join("home/Library/Logs/freerouting/freerouting.log");
        std::fs::write(
            &log,
            "2026-11-10 10:00:01.000 DEBUG  Version check skipped or timed out: java.net.ConnectException\n",
        )
        .unwrap();
        let mut offline = RunRecord::default();
        apply_run_evidence(&mut offline, &dir, &[]);
        assert!(!offline.reached_network && offline.error.is_none());
        std::fs::write(
            &log,
            "2026-11-08 18:22:30.050 DEBUG  GitHub release check response: API rate limit exceeded\n",
        )
        .unwrap();
        let mut online = RunRecord::default();
        apply_run_evidence(&mut online, &dir, &[]);
        assert!(online.reached_network);
        assert!(online.error.unwrap().starts_with("the run was not offline"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn saved_entry(config: &str, runs: Vec<RunRecord>) -> (BoardResult, Entry) {
        let config = Config::parse(config).unwrap();
        let entry = Entry {
            jar: PathBuf::from("/a/freerouting-2.5.0.jar"),
            label: format!("{}@2.5.0", config.name),
            config: Some(config.clone()),
        };
        let saved = BoardResult {
            board: "V0".into(),
            dir: entry.label.clone(),
            version: "2.5.0".into(),
            jar: entry.jar.clone(),
            config: Some(config.name.clone()),
            config_args: config.args.clone(),
            fr_args: Vec::new(),
            dsn_connections: None,
            components: None,
            signal_layers: None,
            runs,
        };
        (saved, entry)
    }

    fn good_run() -> RunRecord {
        RunRecord {
            unrouted: Some(0),
            ses: Some(
                parse::summarize_ses("(session s (routes (resolution um 10) (network_out)))")
                    .unwrap(),
            ),
            ..RunRecord::default()
        }
    }

    fn failed_run() -> RunRecord {
        RunRecord {
            error: Some("settings not in effect: x".into()),
            ..RunRecord::default()
        }
    }

    #[test]
    fn saved_runs_are_only_added_to_by_the_same_settings_on_the_same_dsn() {
        let (saved, entry) = saved_entry("S0=--router.neck_width_um=200", vec![good_run()]);
        let none: [String; 0] = [];
        assert_eq!(
            resume_conflict(&saved, &entry, &none, "(pcb a)", Some("(pcb a)")),
            None
        );
        // A run folder that is gone cannot contradict the DSN.
        assert_eq!(
            resume_conflict(&saved, &entry, &none, "(pcb a)", None),
            None
        );
        // The same name with other settings would make one candidate of two.
        let (_, changed) = saved_entry("S0=--router.neck_width_um=349", vec![]);
        let error = resume_conflict(&saved, &changed, &none, "(pcb a)", Some("(pcb a)")).unwrap();
        assert!(
            error.contains("S0@2.5.0 on V0 has saved runs with other settings"),
            "{error}"
        );
        // So would other --fr-arg values.
        let extra = args(&["--router.fanout.enabled=false"]);
        assert!(resume_conflict(&saved, &entry, &extra, "(pcb a)", Some("(pcb a)")).is_some());
        // And another DSN under the same name.
        let error = resume_conflict(&saved, &entry, &none, "(pcb b)", Some("(pcb a)")).unwrap();
        assert!(error.contains("V0.dsn is not the DSN"), "{error}");
        // An entry with no run left may start over with anything.
        let (empty, _) = saved_entry("S0=--router.neck_width_um=200", vec![]);
        assert_eq!(
            resume_conflict(&empty, &changed, &none, "(pcb b)", Some("(pcb a)")),
            None
        );
    }

    #[test]
    fn retrying_drops_the_first_failed_run_and_those_after_it() {
        let (mut saved, _) = saved_entry("S0=", vec![good_run(), failed_run(), good_run()]);
        assert_eq!(drop_failed_runs(&mut saved), 2);
        assert_eq!(saved.runs.len(), 1);
        assert!(saved.runs[0].succeeded());
        // Nothing failed, nothing dropped.
        assert_eq!(drop_failed_runs(&mut saved), 0);
        let (mut all_failed, _) = saved_entry("S0=", vec![failed_run()]);
        assert_eq!(drop_failed_runs(&mut all_failed), 1);
        assert!(all_failed.runs.is_empty());
        // It only goes with --resume.
        assert!(parse_options(&args(&["--jar", "a", "--retry-failed"]))
            .unwrap_err()
            .contains("goes with --resume"));
        assert!(
            parse_options(&args(&["--jar", "a", "--resume", "--retry-failed"]))
                .unwrap()
                .retry_failed
        );
    }

    /// A bench that is killed leaves its Freerouting running. A resumed run
    /// in the same folder stops it first, and only it.
    #[cfg(unix)]
    #[test]
    fn a_resumed_run_stops_the_freerouting_its_folder_was_left_with() {
        let dir =
            std::env::temp_dir().join(format!("konnect-bench-leftover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gone = |child: &mut std::process::Child| {
            let waited = Instant::now();
            while waited.elapsed() < Duration::from_secs(5) {
                if child.try_wait().unwrap().is_some() {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            false
        };
        // A process whose command line names this run folder, as the JVM's
        // does. The shell waits on its own `read`, so it starts no child that
        // would outlive it.
        let mut leftover = Command::new("/bin/sh")
            .args(["-c", "read line", "leftover"])
            .arg(format!("-Duser.home={}", dir.join("home").display()))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::fs::write(dir.join(RUN_PID_FILE), leftover.id().to_string()).unwrap();
        stop_leftover_run(&dir);
        assert!(gone(&mut leftover), "the leftover process is still running");
        // A process id that now belongs to something else is left alone.
        let mut stranger = Command::new("/bin/sleep")
            .arg("60")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::fs::write(dir.join(RUN_PID_FILE), stranger.id().to_string()).unwrap();
        stop_leftover_run(&dir);
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            stranger.try_wait().unwrap().is_none(),
            "a stranger was stopped"
        );
        let _ = stranger.kill();
        let _ = stranger.wait();
        // Whatever the outcome above, nothing is left running.
        let _ = leftover.kill();
        let _ = leftover.wait();
        // No note, nothing to stop.
        std::fs::remove_file(dir.join(RUN_PID_FILE)).unwrap();
        stop_leftover_run(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_is_named_by_its_sha_256() {
        let dir = std::env::temp_dir().join(format!("konnect-bench-sha-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("abc"), "abc").unwrap();
        assert_eq!(
            sha256_of(&dir.join("abc")).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(sha256_of(&dir.join("absent"))
            .unwrap_err()
            .contains("no SHA-256"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_runs_inputs_round_trip_through_json() {
        let inputs = RunInputs {
            started_unix: 1_794_266_161,
            load_average: Some("1.40 1.14 1.11".into()),
            jar_sha256: "f6".repeat(32),
            dsn_sha256: "0a".repeat(32),
            fixed_ses_sha256: None,
            harness_commit: Some("5d112d9".into()),
            harness_dirty: false,
            java_version: Some("25.0.1".into()),
            arguments: args(&["-jar", "fr.jar", "--router.neck_width_um=200"]),
        };
        let text = report::to_json(&inputs.to_json());
        let (json, _) = parse::parse_json_prefix(&text).unwrap();
        assert_eq!(RunInputs::from_json(&json), Some(inputs));
        // Without the digests it is not a record of a run's inputs.
        let (partial, _) = parse::parse_json_prefix(r#"{"started_unix":1}"#).unwrap();
        assert_eq!(RunInputs::from_json(&partial), None);
        // The load is three numbers, on this machine too.
        let load = load_average().expect("a load average");
        assert_eq!(
            load.split(' ').filter(|n| n.parse::<f64>().is_ok()).count(),
            3
        );
    }

    /// The inputs are on record before the process starts, and what an
    /// earlier attempt left in the folder is kept beside it.
    #[cfg(unix)]
    #[test]
    fn a_run_records_its_inputs_before_it_starts_and_keeps_what_was_there() {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("konnect-bench-inputs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("S0@x").join("b").join("run1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("board.ses"), "an earlier attempt").unwrap();
        // A stand-in for Java: it notes whether the inputs were there when it
        // started, and writes no session.
        let java = root.join("java");
        std::fs::write(
            &java,
            "#!/bin/sh\n[ -f inputs.json ] && echo yes > had_inputs\necho started > started\n",
        )
        .unwrap();
        std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755)).unwrap();
        let dsn = crate::repo_root()
            .join("crates/konnect-core/tests/fixtures/specctra_two_resistors.native-kicad-10.dsn");
        let options = parse_options(&args(&["--jar", "fr.jar"])).unwrap();
        let invocation = Invocation {
            java: java.clone(),
            java_version: Some("25-test".into()),
            harness: Some(("abc123".into(), false)),
        };
        let config = Config::parse("S0=--router.neck_width_um=200").unwrap();
        let record = run_once(
            &invocation,
            (Path::new("/j/fr.jar"), "f6f6"),
            &dsn,
            None,
            &dir,
            &options,
            Some(&config),
        );
        assert!(dir.join("started").is_file(), "{:?}", record.error);
        assert!(
            dir.join("had_inputs").is_file(),
            "the run started before its inputs were recorded"
        );
        let inputs = record.inputs.expect("inputs");
        assert_eq!(inputs.jar_sha256, "f6f6");
        assert_eq!(inputs.dsn_sha256, sha256_of(&dsn).unwrap());
        assert_eq!(inputs.harness_commit.as_deref(), Some("abc123"));
        assert!(!inputs.harness_dirty);
        assert_eq!(
            inputs.arguments.last().unwrap(),
            "--router.neck_width_um=200"
        );
        assert!(inputs.started_unix > 1_700_000_000);
        // The same record is in the folder, for whoever repeats the run.
        let saved = std::fs::read_to_string(dir.join(RUN_INPUTS_FILE)).unwrap();
        let (json, _) = parse::parse_json_prefix(&saved).unwrap();
        assert_eq!(RunInputs::from_json(&json), Some(inputs));
        // No session was written, and that is the run's error, not a pass.
        assert!(record.error.unwrap().starts_with("no SES written"));
        // The earlier attempt's folder was moved aside, not deleted.
        assert!(!dir.join("board.ses").exists());
        let kept: Vec<PathBuf> = std::fs::read_dir(dir.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("run1.superseded-")
            })
            .collect();
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(
            std::fs::read_to_string(kept[0].join("board.ses")).unwrap(),
            "an earlier attempt"
        );
        // Twice in one second, and both are kept.
        std::fs::create_dir_all(&dir).unwrap();
        set_aside(&dir).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        set_aside(&dir).unwrap();
        let count = std::fs::read_dir(dir.parent().unwrap()).unwrap().count();
        assert_eq!(count, 3);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fixed_wiring_comes_with_its_dsn_and_is_put_in_first() {
        let dir = std::env::temp_dir().join(format!("konnect-bench-fixed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(run_sessions(&dir).is_empty());
        std::fs::write(dir.join("board.ses"), "").unwrap();
        assert_eq!(run_sessions(&dir), [dir.join("board.ses")]);
        std::fs::write(dir.join("fixed.ses"), "").unwrap();
        assert_eq!(
            run_sessions(&dir),
            [dir.join("fixed.ses"), dir.join("board.ses")]
        );
        let _ = std::fs::remove_dir_all(&dir);
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
            rescore(&out, None).unwrap();
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

    /// `--rescore` judges every run again from the files its scoring left,
    /// with the spec as it is now, and runs no tool: a changed threshold shows
    /// without routing or scoring anything again.
    #[test]
    fn rescoring_judges_the_saved_score_files_again_with_the_spec_as_it_is_now() {
        let root = std::env::temp_dir().join(format!("konnect-bench-score-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let out = root.join("out");
        let score_files = |dir: &Path, signal_nm: u64, unconnected: &str| {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(
                dir.join("measure.json"),
                format!(
                    r#"{{"placement":"p","pads":{{"GND":4,"+5V":2,"SIG":2}},
                        "tracks":[["GND","F.Cu",200000,0,0,8000000,0,8000000],
                                  ["+5V","F.Cu",500000,0,1000000,20000000,1000000,20000000],
                                  ["SIG","F.Cu",200000,0,2000000,30000000,2000000,{signal_nm}]],
                        "vias":[["SIG",0,2000000,600000,300000]],
                        "zones":[{{"net":"GND","layers":["B.Cu"]}}]}}"#
                ),
            )
            .unwrap();
            std::fs::write(
                dir.join("drc.json"),
                format!(r#"{{"violations":[{{"type":"silk_overlap"}}],"unconnected_items":[{unconnected}]}}"#),
            )
            .unwrap();
            std::fs::write(
                dir.join("audit.json"),
                r#"{"audit":"layout_quality","quality":{"score":60,"copper_quality":60,"routed_share":1.0,
                    "categories":[{"category":"routing","score":60}]},"completion":{"complete":true},
                    "metrics":{"5.3.acute_corners":0,"5.3.right_angle_corners":3,"4.2.nets_undersized":0,
                               "4.3.narrow_paths":0,"3.3.floating_ground_islands":0}}"#,
            )
            .unwrap();
            std::fs::write(
                dir.join("konnect.json"),
                r#"{"executable":"/x","commit":"abc123"}"#,
            )
            .unwrap();
        };
        score_files(&out.join("baseline/score"), 30_000_000, "");
        let run = out.join("S1@2.5.0/V0/run1");
        score_files(&run.join("score"), 31_200_000, "");
        std::fs::write(run.join("stdout.log"), "").unwrap();
        let project = root.join("project/.konnect");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("design_intent.json"),
            r#"{"schema_version":1,"currents":[{"net":"+5V","max_current_a":1},{"net":"GND","max_current_a":1}]}"#,
        )
        .unwrap();
        let spec = root.join("spec.json");
        let write_spec = |thresholds: &str| {
            std::fs::write(
                &spec,
                format!(
                    r#"{{"project":"project","board":"b.kicad_pcb","baseline":"r.kicad_pcb","kicad_config_home":"cfg",
                        "currents":"test values","thresholds":{thresholds}}}"#
                ),
            )
            .unwrap();
        };
        write_spec("{}");
        std::fs::write(
            out.join("results.json"),
            format!(
                r#"{{"score_spec":{:?},"boards":[{{"board":"V0","dir":"S1@2.5.0","version":"2.5.0","jar":"/a/freerouting-2.5.0.jar",
                    "config":"S1","config_args":["--router.fanout.enabled=false"],
                    "runs":[{{"exit_code":0,"unrouted":0,"clearance_violations":0,"runtime_s":40}}]}}]}}"#,
                spec.display().to_string()
            ),
        )
        .unwrap();
        let rescored = || {
            rescore(&out, None).unwrap();
            let text = std::fs::read_to_string(out.join("results.json")).unwrap();
            parse::parse_json_prefix(&text).unwrap().0
        };
        let json = rescored();
        assert_eq!(json.text_at("score.konnect_commit"), Some("abc123"));
        assert_eq!(json.number_at("score.baseline.length_mm.pour"), Some(8.0));
        let Some(Json::Array(boards)) = json.at("boards") else {
            panic!("boards")
        };
        let Some(Json::Array(runs)) = boards[0].at("runs") else {
            panic!("runs")
        };
        // 31.2 mm of signal track over the baseline's 30: x1.04, outside the
        // default x1.03 band. The power net counts as power, the pour net not.
        assert_eq!(boards[0].text_at("config"), Some("S1"));
        assert!((runs[0].number_at("compared.signal_ratio").unwrap() - 1.04).abs() < 1e-9);
        assert_eq!(runs[0].number_at("compared.power_ratio"), Some(1.0));
        assert_eq!(runs[0].number_at("board_score.length_mm.power"), Some(20.0));
        assert_eq!(runs[0].text_at("compared.against_baseline"), Some("below"));
        assert!(matches!(
            runs[0].at("compared.passes"),
            Some(Json::Bool(true))
        ));
        // The given setting has no saved file to confirm it, and that is kept.
        assert!(
            matches!(runs[0].at("settings.unconfirmed"), Some(Json::Array(list)) if list.len() == 1)
        );
        // A wider band in the spec, and the same files match.
        write_spec(r#"{"signal_match":1.05}"#);
        let json = rescored();
        let Some(Json::Array(boards)) = json.at("boards") else {
            panic!("boards")
        };
        let Some(Json::Array(runs)) = boards[0].at("runs") else {
            panic!("runs")
        };
        assert_eq!(
            runs[0].text_at("compared.against_baseline"),
            Some("matches")
        );
        // Both files were written beside and moved into place.
        assert!(!out.join("results.json.partial").exists());
        assert!(!out.join("summary.md.partial").exists());
        // Results that were scored are not rewritten without their scores:
        // with the spec gone, rescoring stops, and results.json stays.
        let before = std::fs::read_to_string(out.join("results.json")).unwrap();
        std::fs::rename(&spec, root.join("moved.json")).unwrap();
        let error = rescore(&out, None).unwrap_err();
        assert!(error.contains("the scores cannot be read again"), "{error}");
        assert_eq!(
            std::fs::read_to_string(out.join("results.json")).unwrap(),
            before
        );
        std::fs::rename(root.join("moved.json"), &spec).unwrap();
        // The run left no session file here, so it stays a failed run and is
        // not chosen, however its saved score reads.
        let summary = std::fs::read_to_string(out.join("summary.md")).unwrap();
        assert!(
            summary.contains("1. S1@2.5.0: 1 of 1 runs failed;"),
            "{summary}"
        );
        assert!(
            summary.contains("No candidate passes every gate in every run."),
            "{summary}"
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
