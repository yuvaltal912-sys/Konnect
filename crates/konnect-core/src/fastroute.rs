//! fastroute, a Rust port of Freerouting, run as a separate executable: a command
//! line, a DSN in, a session and a JSON report out. It is a process, not a library,
//! so that a hung run can be killed, a run can be sandboxed off the network, and
//! parallel runs keep their memory apart (docs/ROUTE_SEARCH_DESIGN.md, decision 2).
//!
//! The build Konnect runs is the candidate the owner chose on 2026-11-16
//! (C** + BR1 + W8P), with the smart-router switches of Row GS ([`SEARCH_SWITCHES`]),
//! the fill model of Row ZR, stages ZR1 to ZR3 (`--x.zones`, and from ZR3 its cut cost,
//! [`ZONES_CUT_SWITCH`]), and Row TN's KiCad-like serpentines (`--x.serpentine`): build
//! x23-tn. [`locate`] finds a
//! binary and [`probe`] proves, before anything is routed, that it knows every
//! experimental switch Konnect sends. That is not proof that it is x23-tn: an earlier
//! engine commit may know the same switches, and x18 to x23-tn all print
//! `fastroute 0.1.14`. What identifies the build is its SHA-256: the
//! 1 Kopper package checks it against `packaging/1-kopper/fastroute.json`, and every
//! result reports it as `engine.sha256`.

use crate::specctra::RoutingHints;
use crate::specctra_seed::{lists, parse_specctra, session_routes, WIRE_DESCRIPTORS};
use crate::tools::integration::WidthFallback;
use anyhow::{anyhow, bail, Context, Result};
use konnect_sexp::SexpNode;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
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
    "--x.serpentine",
];

/// The smart-router switches (Row GS), sent on every routing pass after
/// [`CANDIDATE_SWITCHES`], in this order, and never on a Tune pass (G1):
/// - `--x.oct_bound`: the optimizer's length bound in the 45-degree metric (S1);
/// - `--x.opt_follow_rips=2`: an optimizer candidate routes up to 2 nets it ripped (F1);
/// - `--x.group_search`: the group re-route search after the optimizer.
pub(crate) const SEARCH_SWITCHES: &[&str] =
    &["--x.oct_bound", "--x.opt_follow_rips=2", "--x.group_search"];

/// Every fixed switch a routing pass sends, in its order: [`CANDIDATE_SWITCHES`],
/// then [`SEARCH_SWITCHES`] (what the result's `engine.switches` lists).
pub(crate) fn routing_switches() -> Vec<&'static str> {
    CANDIDATE_SWITCHES
        .iter()
        .chain(SEARCH_SWITCHES)
        .copied()
        .collect()
}

/// The candidate's name, as the research and the owner call it: build x23-tn, the
/// candidate C** + BR1 + W8P with the group search (GS) and [`SEARCH_SWITCHES`], the
/// fill model of Row ZR, stages ZR1 to ZR3 (`--x.zones`: the model; from ZR2 the model as
/// the engine's connectivity with the closing stage; from ZR3 the cut cost in the maze and
/// the group search's "never more pour damage" rule), and Row TN (`--x.serpentine`, a
/// [`CANDIDATE_SWITCHES`] entry, so the Tune pass sends it too: pair skew and length groups
/// are tuned with KiCad-like serpentines).
pub(crate) const CANDIDATE_NAME: &str = "C** + BR1 + W8P + GS + ZR3 + TN";

/// Row ZR, stage ZR3: the cut cost's weights, sent with every `--x.zones` (never without
/// it: the engine refuses a cut cost with no model). Per length (a share of the maze step's
/// own cost), per via and per thermal spoke (both in vias). ZR3's weight matrix chose
/// W = 2 of 0, 0.5, 1, 2 and 4 (L = V = W, S = 2W; `docs/ZONE_ROUTING_DESIGN.md`, "ZR3 as
/// built").
pub(crate) const ZONES_CUT_SWITCH: &str = "--x.zones_cut=2,2,4";

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

/// The binary Konnect will run, proven to know every switch Konnect sends ([`probe`]).
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

/// Proves that `path` is a fastroute that knows every experimental switch Konnect
/// sends; its SHA-256, not the probe, says which build it is. Each step runs sandboxed
/// ([`sandbox_profile`] with `scratch` as the run folder), with the environment
/// cleared to `HOME`, `TMPDIR` and `PATH` as in [`run_pass`], and is killed after
/// [`PROBE_TIMEOUT`]:
/// 1. `--version`: the first line of its output must start with `fastroute `.
/// 2. [`probe_arguments`]: `-de <scratch>/absent.dsn`, every [`CANDIDATE_SWITCHES`]
///    entry, every [`SEARCH_SWITCHES`] entry, `--x.gs_uncounted=<scratch>/probe-uncounted.txt`
///    (an empty file the probe writes; fastroute reads it as it parses the switch, and an
///    empty file names no net), `--x.zones=<scratch>/probe-zones.txt` ([`PROBE_ZONES_TEXT`],
///    a valid zones file with no `zone` line, so it models nothing; Row ZR, stage ZR1),
///    [`ZONES_CUT_SWITCH`] right after it (stage ZR3), then [`SENTINEL_SWITCH`], last. fastroute parses its
///    arguments in order and stops at the first unknown experimental switch
///    (`exp::parse_arg`), so the sentinel is named only when every switch before it was
///    accepted. The output must contain
///    `unknown experimental switch --x.konnect_probe_unknown` and must not name any
///    other switch as unknown. A build that lacks one of the switches names that one
///    (x18, without the search switches, names `--x.oct_bound`; x19, without the fill
///    model, names `--x.zones=`; x21, without the cut cost, names `--x.zones_cut=`; x22,
///    without Row TN's serpentines, names `--x.serpentine`); a
///    build without
///    experimental switches names none (it goes on to open the absent DSN and fails on
///    that).
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

    // Step 2: every switch Konnect sends is accepted, and only the sentinel is unknown.
    let uncounted = scratch.join(PROBE_UNCOUNTED);
    std::fs::write(&uncounted, "")
        .with_context(|| format!("{} could not be written", uncounted.display()))?;
    let zones = scratch.join(PROBE_ZONES);
    std::fs::write(&zones, PROBE_ZONES_TEXT)
        .with_context(|| format!("{} could not be written", zones.display()))?;
    let args = probe_arguments(&scratch);
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

/// The empty `--x.gs_uncounted` file of the probe's step 2, in its scratch folder.
pub(crate) const PROBE_UNCOUNTED: &str = "probe-uncounted.txt";

/// The `--x.zones` file of the probe's step 2, in its scratch folder (Row ZR, stage ZR1).
pub(crate) const PROBE_ZONES: &str = "probe-zones.txt";

/// The probe's zones file: the header the engine requires (each key exactly once) and no
/// `zone` line, so the build reads and checks the switch and models nothing.
pub(crate) const PROBE_ZONES_TEXT: &str = "zones 1\ncell_mm 0.05\nmin_spokes 2\nedge_clearance_mm 0.3\nhole_clearance_mm 0.25\nteardrop_mm 0\n";

/// The probe's step 2 command line for `scratch` ([`probe`]): `-de <scratch>/absent.dsn`,
/// [`CANDIDATE_SWITCHES`], [`SEARCH_SWITCHES`], `--x.gs_uncounted=<scratch>/`
/// [`PROBE_UNCOUNTED`], `--x.zones=<scratch>/`[`PROBE_ZONES`], [`ZONES_CUT_SWITCH`] (Row ZR,
/// stage ZR3), and [`SENTINEL_SWITCH`] last: fastroute stops at the first
/// unknown switch, so a switch after the sentinel would never be checked.
pub(crate) fn probe_arguments(scratch: &Path) -> Vec<String> {
    let mut args = vec![
        "-de".to_string(),
        scratch.join("absent.dsn").display().to_string(),
    ];
    args.extend(CANDIDATE_SWITCHES.iter().map(|switch| switch.to_string()));
    args.extend(SEARCH_SWITCHES.iter().map(|switch| switch.to_string()));
    args.push(format!(
        "--x.gs_uncounted={}",
        scratch.join(PROBE_UNCOUNTED).display()
    ));
    args.push(format!("--x.zones={}", scratch.join(PROBE_ZONES).display()));
    args.push(ZONES_CUT_SWITCH.to_string());
    args.push(SENTINEL_SWITCH.to_string());
    args
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
    /// The `--layer-heights` value (`layer_heights::LayerHeights::switch_value`), sent
    /// by [`layer_heights_argument`]. Which passes get it is the caller's choice
    /// (Row LT, L7): routing passes carry it only when the call has a resolved pair.
    /// The Tune pass takes its own ([`TuneInputs::layer_heights`]).
    pub layer_heights: Option<&'a str>,
    /// The `--x.gs_uncounted` file ([`uncounted_file_text`]): the pour nets, whose
    /// copper the group search leaves out of its cost as the judge leaves it out of
    /// its measures. Routing passes only.
    pub uncounted: Option<&'a Path>,
    /// Row ZR, stage ZR1: the `--x.zones` file (`zone_fill::file_text`), each modelled
    /// pour's fill settings. Given only for a variant with planes and a plan that
    /// models at least one pour; routing passes only, never the Tune pass.
    pub zones: Option<&'a Path>,
}

/// The command line of one pass, every setting once, in this order: `-de <dsn>`,
/// `-do <ses>`, `--report=<report>`, [`PRESET`], `--router.optimizer.enabled=<bool>`,
/// `--router.neck_width_um=<um>`, `--router.copper_to_edge_clearance_um=<um>` when
/// `hints` has one (sent as `250.001` when it is within 1e-9 of 250, as the
/// Freerouting path does), `--router.autorouter.max_threads=` and
/// `--router.optimizer.max_threads=` at [`THREADS`], [`CANDIDATE_SWITCHES`],
/// [`SEARCH_SWITCHES`] (Row GS), `--x.par_via_amps=<amps>` when given,
/// `--x.gs_uncounted=<file>` when given (Row GS, G3), `--pairs=<file>` when given,
/// `--layer-heights=<value>` when given (Row LT, L7), `--x.zones=<file>` and right after it
/// [`ZONES_CUT_SWITCH`] when given (Row ZR, ZR1 and ZR3), and
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
    args.extend(copper_edge_argument(hints));
    args.push(format!("--router.autorouter.max_threads={THREADS}"));
    args.push(format!("--router.optimizer.max_threads={THREADS}"));
    args.extend(CANDIDATE_SWITCHES.iter().map(|switch| switch.to_string()));
    args.extend(SEARCH_SWITCHES.iter().map(|switch| switch.to_string()));
    if let Some(amps) = files.amps {
        args.push(format!("--x.par_via_amps={}", amps.display()));
    }
    if let Some(uncounted) = files.uncounted {
        args.push(format!("--x.gs_uncounted={}", uncounted.display()));
    }
    if let Some(pairs) = files.pairs {
        args.push(format!("--pairs={}", pairs.display()));
    }
    if let Some(heights) = files.layer_heights {
        args.push(layer_heights_argument(heights));
    }
    if let Some(zones) = files.zones {
        args.push(format!("--x.zones={}", zones.display()));
        args.push(ZONES_CUT_SWITCH.to_string());
    }
    if let Some(initial) = &pass.initial_session {
        args.push(format!("--initial-session={}", initial.display()));
    }
    args
}

/// `--router.copper_to_edge_clearance_um=<um>` when `hints` has a clearance (sent as
/// `250.001` when it is within 1e-9 of 250: Freerouting reads exactly 250 as "unset",
/// and the path to it sends 250.001).
fn copper_edge_argument(hints: &RoutingHints) -> Option<String> {
    let clearance = hints.copper_edge_clearance_um? as f64;
    let clearance = if (clearance - 250.0).abs() < 1e-9 {
        250.001
    } else {
        clearance
    };
    Some(format!(
        "--router.copper_to_edge_clearance_um={}",
        number(clearance)
    ))
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

/// The `--x.gs_uncounted` file from the manifest's `pours`: one net name a line, each
/// net once, sorted, with a final newline. `None` when the manifest has no pour, or
/// none of its names can be written.
/// A name that is empty or holds a line break is left out (one net per line), as
/// [`amps_file_text`] does. The set is the judge's `pour_nets` (`route_judge.rs`).
///
/// fastroute reads the file line by line, trims each line, skips an empty line and a
/// line that starts with `#` (a comment), and looks each name up ignoring case; a name
/// the board does not have is logged and skipped (`exp::parse_net_names`,
/// `group_search::settings_from_exp`). So a name with white space inside is written
/// as it is, and a name that starts or ends with white space, or starts with `#`, is
/// left out too: fastroute would read it as another name or as a comment. And since
/// fastroute matches ignoring case, a pour net is left out when the manifest's `nets`
/// hold a net that is not a pour net and whose name differs from it only by letter
/// case: fastroute would leave that net, which the judge counts, out of the cost too.
/// A pour net left out is counted in the group search's cost, which is the safe side.
pub(crate) fn uncounted_file_text(manifest: &Value) -> Option<String> {
    let pours: BTreeSet<&str> = manifest["pours"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["net"].as_str())
        .collect();
    let others: Vec<&str> = manifest["nets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["name"].as_str())
        .filter(|name| !pours.contains(name))
        .collect();
    let names: BTreeSet<&str> = pours
        .iter()
        .copied()
        .filter(|net| {
            !net.is_empty()
                && !net.contains(['\n', '\r'])
                && net.trim() == *net
                && !net.starts_with('#')
                && !others.iter().any(|other| same_ignoring_case(net, other))
        })
        .collect();
    if names.is_empty() {
        return None;
    }
    let mut text = String::new();
    for net in names {
        text.push_str(net);
        text.push('\n');
    }
    Some(text)
}

/// Whether two names are one to fastroute, which compares net names ignoring case
/// (Java's `compareToIgnoreCase`, character by character after upper- and lower-casing).
/// Either casing counts, so this errs towards "the same", the safe side for
/// [`uncounted_file_text`].
fn same_ignoring_case(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase() || a.to_uppercase() == b.to_uppercase()
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
    /// Row GS: the report's `group_search` object as fastroute wrote it, when the
    /// report has one (the stage ran); `None` for a Tune pass, which never runs it.
    pub group_search: Option<Value>,
    /// Row ZR, ZR1: the report's `planes` object as fastroute wrote it, when the report
    /// has one (the pass was sent `--x.zones`); `None` otherwise, a Tune pass included.
    pub planes: Option<Value>,
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
    run_arguments(binary, folder, pass.name, deadline, |ses, report| {
        pass_arguments(dsn, ses, report, hints, files, &pass)
    })
    .await
}

/// The process part of a pass, shared by [`run_pass`] and [`run_tune_pass`] (Row LT):
/// everything [`run_pass`] documents, for the pass called `name`. `arguments` builds
/// the command line from the session path (`folder/<name>.ses`) and the report path
/// (`folder/<name>.json`) the pass writes.
async fn run_arguments(
    binary: &Binary,
    folder: &Path,
    name: &str,
    deadline: Instant,
    arguments: impl FnOnce(&Path, &Path) -> Vec<String>,
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

    let ses = folder.join(format!("{name}.ses"));
    let report = folder.join(format!("{name}.json"));
    let args = arguments(&ses, &report);

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
            "fastroute pass {name} exited with {status}; log tail: {}",
            log_tail()
        )));
    }
    match std::fs::metadata(&ses) {
        Ok(meta) if meta.is_file() && meta.len() > 0 => {}
        _ => {
            return Err(PassError::Failed(anyhow!(
                "fastroute pass {name} left no session ({} is missing or empty); log tail: {}",
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
        group_search: report_value
            .get("group_search")
            .filter(|object| !object.is_null())
            .cloned(),
        planes: report_value
            .get("planes")
            .filter(|object| !object.is_null())
            .cloned(),
    })
}

/// The kept session of a run and what each pass did.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Held {
    pub ses: PathBuf,
    pub unrouted: usize,
    /// `full`, `fallback`, `polish` or, after a kept Tune pass, [`TUNE_PASS`].
    pub kept: &'static str,
    /// One row per pass: `name`, `outcome` (`kept`, `discarded`, `skipped` or
    /// `failed`; the Tune row of a candidate the judge has not chosen yet says
    /// `candidate`), `unrouted`, `seconds`, and `reason` for a pass discarded, skipped
    /// or failed (a failed pass's reason is its error, cut to 300 characters).
    pub passes: Vec<Value>,
    pub seconds: f64,
    /// Row LT, L23: the session the routing passes kept, set when a Tune candidate
    /// exists (the judge judges it beside the candidates). Without a candidate it
    /// is `None` and `ses` is the only session.
    pub untuned: Option<PathBuf>,
    /// Row LT: what the Tune attempts gave (every candidate, every pass row).
    pub tune: TuneOutcome,
    /// Row GS: the `group_search` object of the routing pass the hold sequence kept
    /// ([`PassOutcome::group_search`]). A kept Tune pass changes neither this nor
    /// anything the search did: its report has none, so this stays the routing
    /// pass's it started from.
    pub group_search: Option<Value>,
    /// Row ZR, ZR1: the `planes` object of the same routing pass
    /// ([`PassOutcome::planes`]); a kept Tune pass leaves it as it is, as it leaves
    /// [`Held::group_search`].
    pub planes: Option<Value>,
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
///
/// Row LT: with `tune`, [`tune_held`] runs after the sequence, from the session it
/// kept, and its row joins `passes`. Routing passes never get `--tune`.
#[allow(clippy::too_many_arguments)] // the parts of one call, each read once
pub(crate) async fn route_held(
    binary: &Binary,
    folder: &Path,
    dsn: &Path,
    hints: &RoutingHints,
    files: CallFiles<'_>,
    fallback: WidthFallback,
    tune: Option<TuneRequest<'_>>,
    deadline: Instant,
) -> std::result::Result<Held, PassError> {
    let row = pass_row;
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
    let mut kept_group_search = full.group_search.clone();
    let mut kept_planes = full.planes.clone();

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
                        kept_group_search = outcome.group_search.clone();
                        kept_planes = outcome.planes.clone();
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
                    kept_group_search = outcome.group_search.clone();
                    kept_planes = outcome.planes.clone();
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

    let held = Held {
        ses: kept_ses,
        unrouted: kept_unrouted,
        kept,
        passes,
        seconds,
        untuned: None,
        tune: TuneOutcome::default(),
        group_search: kept_group_search,
        planes: kept_planes,
    };
    match tune {
        Some(tune) => tune_held(binary, folder, dsn, hints, files, &tune, held, deadline).await,
        None => Ok(held),
    }
}

/// One row of [`Held::passes`].
fn pass_row(
    name: &str,
    outcome: &str,
    unrouted: Option<usize>,
    seconds: Option<f64>,
    reason: Option<&str>,
) -> Value {
    json!({
        "name": name,
        "outcome": outcome,
        "unrouted": unrouted,
        "seconds": seconds,
        "reason": reason,
    })
}

/// Row LT: the Tune pass's name and folder (docs/LENGTH_TUNING_DESIGN.md, L6).
pub(crate) const TUNE_PASS: &str = "tune";

/// Row LT, L20: the Tune pass's switch that turns fastroute's autorouter stage off,
/// so its final tail removal never deletes held copper.
pub(crate) const TUNE_ROUTER_OFF: &str = "--router.autorouter.enabled=false";

/// Row LT, L24: the pour-safe candidate's switch: other nets' planes are obstacles,
/// so the tuner leaves every trace that touches one as it is.
pub(crate) const TUNE_POUR_SAFE: &str = "--router.plane_as_obstacle=true";

/// Row LT, L21: at most this many Tune passes per candidate, each without the nets
/// whose copper touched itself in the one before.
pub(crate) const TUNE_ROUNDS_MAX: usize = 3;

/// Row LT, L21: the margin of a touch, and the shared stretch an overlap needs, µm.
pub(crate) const CONTACT_MARGIN_UM: i64 = 1;

/// How many nets a discarded Tune pass's reason names before it says "and N more".
const MAX_NAMED_NETS: usize = 5;

/// Which Tune candidate (L24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TuneVariant {
    /// The Tune pass of L6 and L20.
    Free,
    /// The same, plus [`TUNE_POUR_SAFE`].
    PourSafe,
}

impl TuneVariant {
    /// The candidate's pass folder below the run folder: `tune`, `tune-pour-safe`;
    /// round r > 1 adds `-r<r>`.
    pub(crate) fn folder_name(self, round: usize) -> String {
        let base = match self {
            TuneVariant::Free => TUNE_PASS.to_string(),
            TuneVariant::PourSafe => format!("{TUNE_PASS}-pour-safe"),
        };
        if round > 1 {
            format!("{base}-r{round}")
        } else {
            base
        }
    }
}

/// Every Tune pass folder a run can hold (L26): `tune`, `tune-r2`, `tune-r3`,
/// `tune-pour-safe`, `tune-pour-safe-r2`, `tune-pour-safe-r3`.
pub(crate) fn tune_folder_names() -> Vec<String> {
    [TuneVariant::Free, TuneVariant::PourSafe]
        .into_iter()
        .flat_map(|variant| (1..=TUNE_ROUNDS_MAX).map(move |round| variant.folder_name(round)))
        .collect()
}

/// What the Tune pass needs beyond a routing pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TuneInputs<'a> {
    /// The call's `--tune` file (`tools/route_lengths.rs`).
    pub tune: &'a Path,
    /// Every net class of the DSN (`specctra::dsn_class_names`): the autorouter
    /// ignores them all, so it routes nothing.
    pub classes: &'a [String],
    /// The DSN names of the nets the tune file lists: the only nets whose wiring the
    /// pass may change.
    pub nets: &'a BTreeSet<String>,
    /// The Tune pass's own `--layer-heights` value (L7: sent whenever the call has
    /// heights, where the routing passes get them only with a resolved pair, so it
    /// is not [`CallFiles::layer_heights`]).
    pub layer_heights: Option<&'a str>,
    /// The nets that have a plane in the run's DSN (`specctra::dsn_planes`, the
    /// run's own variant). The pour-safe candidate (L24) runs only when one of them
    /// is outside `nets`.
    pub planes: &'a [String],
    /// [`crate::specctra::via_disc_radii`] of the run's DSN: what [`changed_outside`]
    /// takes for a via stub.
    pub via_radii: &'a BTreeMap<String, i64>,
}

/// The tune file of the next round: given every net dropped so far, it writes the
/// file into the given folder and returns its path and the nets it lists, or `None`
/// when no group is left (`route_lengths::write_file_without`, pair partners
/// included).
pub(crate) type NextTuneFile<'a> =
    &'a (dyn Fn(&Path, &BTreeSet<String>) -> Result<Option<(PathBuf, BTreeSet<String>)>> + Sync);

/// A call's Tune pass as [`route_held`] runs it: the first round's inputs and how
/// to write the next round's tune file.
#[derive(Clone, Copy)]
pub(crate) struct TuneRequest<'a> {
    pub inputs: TuneInputs<'a>,
    pub next: NextTuneFile<'a>,
}

/// `--layer-heights=<value>` from `layer_heights::LayerHeights::switch_value`.
pub(crate) fn layer_heights_argument(value: &str) -> String {
    format!("--layer-heights={value}")
}

/// The Tune pass's command line, every setting once, in this order: `-de <dsn>`,
/// `-do <ses>`, `--report=<report>`, [`PRESET`], `--router.optimizer.enabled=false`,
/// [`TUNE_ROUTER_OFF`] (L20: the autorouter stage is off, so its tail removal never
/// runs), [`TUNE_POUR_SAFE`] for [`TuneVariant::PourSafe`] only,
/// `--router.autorouter.ignore_net_classes=<classes, comma separated>` (left out
/// when there is none; with the stage off it changes nothing, and stays as a second
/// lock), `--router.copper_to_edge_clearance_um=` when `hints` has
/// one, the two thread settings at [`THREADS`], [`CANDIDATE_SWITCHES`],
/// `--x.par_via_amps=` when `files` has it, `--tune=<file>`, `--layer-heights=`
/// when `files` has it, and `--initial-session=<initial>`. Never `--pairs`,
/// `--live`, `--multi-start`, `--max-time`, `--diagnose` or a neck width, and never
/// a [`SEARCH_SWITCHES`] entry nor `--x.gs_uncounted` (Row GS, G1: with the
/// optimizer off the group search would rip and straighten the meanders the tuner
/// just added); `files.uncounted` is ignored. Never `--x.zones` either (Row ZR: its
/// autorouter and optimizer are off and it runs no closing, so a model would only cost
/// a redraw); `files.zones` is ignored.
#[allow(clippy::too_many_arguments)] // the parts of one command line, each read once
pub(crate) fn tune_pass_arguments(
    dsn: &Path,
    ses: &Path,
    report: &Path,
    hints: &RoutingHints,
    files: CallFiles<'_>,
    variant: TuneVariant,
    tune: &TuneInputs<'_>,
    initial: &Path,
) -> Vec<String> {
    let mut args = vec![
        "-de".to_string(),
        dsn.display().to_string(),
        "-do".to_string(),
        ses.display().to_string(),
        format!("--report={}", report.display()),
    ];
    args.extend(PRESET.iter().map(|argument| argument.to_string()));
    args.push("--router.optimizer.enabled=false".to_string());
    args.push(TUNE_ROUTER_OFF.to_string());
    if variant == TuneVariant::PourSafe {
        args.push(TUNE_POUR_SAFE.to_string());
    }
    if !tune.classes.is_empty() {
        args.push(format!(
            "--router.autorouter.ignore_net_classes={}",
            tune.classes.join(",")
        ));
    }
    args.extend(copper_edge_argument(hints));
    args.push(format!("--router.autorouter.max_threads={THREADS}"));
    args.push(format!("--router.optimizer.max_threads={THREADS}"));
    args.extend(CANDIDATE_SWITCHES.iter().map(|switch| switch.to_string()));
    if let Some(amps) = files.amps {
        args.push(format!("--x.par_via_amps={}", amps.display()));
    }
    args.push(format!("--tune={}", tune.tune.display()));
    if let Some(heights) = files.layer_heights {
        args.push(layer_heights_argument(heights));
    }
    args.push(format!("--initial-session={}", initial.display()));
    args
}

/// Runs the Tune pass in `folder` exactly as [`run_pass`] runs a pass (the same
/// sandbox, environment, log and deadline; [`run_arguments`]), with
/// [`tune_pass_arguments`]. Its session is `folder/tune.ses`, its report
/// `folder/tune.json`.
#[allow(clippy::too_many_arguments)] // the parts of one pass, each read once
pub(crate) async fn run_tune_pass(
    binary: &Binary,
    folder: &Path,
    variant: TuneVariant,
    dsn: &Path,
    hints: &RoutingHints,
    files: CallFiles<'_>,
    tune: &TuneInputs<'_>,
    initial: &Path,
    deadline: Instant,
) -> std::result::Result<PassOutcome, PassError> {
    run_arguments(binary, folder, TUNE_PASS, deadline, |ses, report| {
        tune_pass_arguments(dsn, ses, report, hints, files, variant, tune, initial)
    })
    .await
}

/// One net's copper in a session, as geometry (L6): its straight pieces by layer and
/// width, collinear pieces that touch merged into one, each piece's ends rounded to
/// 1 µm and ordered; its vias by padstack and rounded position. Both lists sorted.
/// Two sessions that split one trace into different pieces compare equal: the
/// merge runs on the session's own coordinates, and only the merged pieces' ends are
/// rounded, so a trace at any angle splits and merges the same way in both.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct NetCopper {
    /// (layer, width in µm, [x1, y1, x2, y2] in µm).
    pub pieces: Vec<(String, i64, [i64; 4])>,
    /// (padstack, [x, y] in µm).
    pub vias: Vec<(String, [i64; 2])>,
}

/// A session value in micrometres, rounded to 1 µm.
fn session_um(node: Option<&SexpNode>, um_per_unit: f64, what: &str) -> Result<i64> {
    let value: f64 = node
        .and_then(SexpNode::as_str)
        .with_context(|| format!("the session's {what} is missing"))?
        .parse()
        .map_err(|_| anyhow!("the session's {what} is not a number"))?;
    let um = (value * um_per_unit).round();
    // A coordinate this large is not a board: the products below must not overflow.
    if !um.is_finite() || um.abs() > 1e12 {
        bail!("the session's {what} is too large to compare");
    }
    Ok(um as i64)
}

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Session coordinates are held in thousandths of a session unit: exact for any
/// coordinate a session writes with up to three decimals (they are integers in
/// practice), and the integer products of the collinear merge stay exact.
const TICKS_PER_UNIT: f64 = 1000.0;

/// A path of a session: its layer, width in µm and points in ticks.
type SessionPath = (String, i64, Vec<[i128; 2]>);

/// A straight piece on its line: its two ends in ticks, each with its position along
/// the line's direction.
type Span = (i128, [i128; 2], i128, [i128; 2]);

/// A session coordinate in ticks, exact. The size check is on the micrometres.
fn session_ticks(node: Option<&SexpNode>, um_per_unit: f64, what: &str) -> Result<i128> {
    let value: f64 = node
        .and_then(SexpNode::as_str)
        .with_context(|| format!("the session's {what} is missing"))?
        .parse()
        .map_err(|_| anyhow!("the session's {what} is not a number"))?;
    let um = value * um_per_unit;
    // A coordinate this large is not a board: the products below must not overflow.
    if !um.is_finite() || um.abs() > 1e12 {
        bail!("the session's {what} is too large to compare");
    }
    Ok((value * TICKS_PER_UNIT).round() as i128)
}

/// A session's copper as it is written, before any merge: each net's paths (layer,
/// width in µm, points in ticks) and vias, and the session's µm per unit.
struct RawCopper {
    paths: BTreeMap<String, Vec<SessionPath>>,
    vias: BTreeMap<String, Vec<(String, [i64; 2])>>,
    um_per_unit: f64,
}

/// Reads every `(net NAME (wire …) (via …))` of `(routes (network_out …))` with the
/// session's own `resolution`. A net named in two `net` entries is one net. A
/// session that does not parse, or a wire that is not a `path` or a `qarc`, is an
/// error; a `qarc` is skipped.
fn raw_copper(ses_text: &str) -> Result<RawCopper> {
    let session = parse_specctra(ses_text, "session")?;
    let (routes, um_per_unit) = session_routes(&session)?;
    let mut paths: BTreeMap<String, Vec<SessionPath>> = BTreeMap::new();
    let mut vias: BTreeMap<String, Vec<(String, [i64; 2])>> = BTreeMap::new();
    for network in lists(routes, "network_out") {
        for net in lists(network, "net") {
            let name = net
                .get(1)
                .and_then(SexpNode::as_str)
                .context("a net of the session has no name")?
                .to_string();
            paths.entry(name.clone()).or_default();
            vias.entry(name.clone()).or_default();
            for wire in lists(net, "wire") {
                for shape in wire.children().unwrap_or(&[]).iter().skip(1) {
                    let Some(head) = shape.head() else { continue };
                    let form = head.to_ascii_lowercase();
                    if WIRE_DESCRIPTORS.contains(&form.as_str()) {
                        continue;
                    }
                    // An arc is not a straight piece: contacts with arcs are not
                    // checked (fastroute writes none; the importer accepts a `qarc`).
                    if form == "qarc" {
                        continue;
                    }
                    if form != "path" {
                        bail!("net {name}: a wire holds a {form}, not a path");
                    }
                    let layer = shape
                        .get(1)
                        .and_then(SexpNode::as_str)
                        .with_context(|| format!("net {name}: a path has no layer"))?
                        .to_string();
                    let width = session_um(shape.get(2), um_per_unit, "path width")?;
                    let values = shape.children().unwrap_or(&[]);
                    if values.len() < 7 || (values.len() - 3) % 2 != 0 {
                        bail!("net {name}: a path holds fewer than two points or an odd number of coordinates");
                    }
                    let points = values[3..]
                        .chunks_exact(2)
                        .map(|pair| {
                            Ok([
                                session_ticks(Some(&pair[0]), um_per_unit, "path coordinate")?,
                                session_ticks(Some(&pair[1]), um_per_unit, "path coordinate")?,
                            ])
                        })
                        .collect::<Result<Vec<_>>>()?;
                    paths
                        .entry(name.clone())
                        .or_default()
                        .push((layer, width, points));
                }
            }
            for via in lists(net, "via") {
                let padstack = via
                    .get(1)
                    .and_then(SexpNode::as_str)
                    .with_context(|| format!("net {name}: a via has no padstack"))?
                    .to_string();
                let at = [
                    session_um(via.get(2), um_per_unit, "via x")?,
                    session_um(via.get(3), um_per_unit, "via y")?,
                ];
                vias.entry(name.clone()).or_default().push((padstack, at));
            }
        }
    }
    Ok(RawCopper {
        paths,
        vias,
        um_per_unit,
    })
}

/// A session's copper by net: every `(net NAME (wire …) (via …))` of
/// `(routes (network_out …))`, read with the session's own `resolution`, as
/// [`NetCopper`]. Keywords in any letter case; a session that does not parse, or a
/// wire that is not a `path`, is an error.
pub(crate) fn copper_by_net(ses_text: &str) -> Result<BTreeMap<String, NetCopper>> {
    let RawCopper {
        paths,
        mut vias,
        um_per_unit,
    } = raw_copper(ses_text)?;
    let mut result = BTreeMap::new();
    for (name, paths) in paths {
        // Each straight piece on its own line (layer, width, direction, offset).
        let mut lines: BTreeMap<(String, i64, i128, i128, i128), Vec<Span>> = BTreeMap::new();
        for (layer, width, points) in paths {
            for pair in points.windows(2) {
                let (p, q) = (pair[0], pair[1]);
                if p == q {
                    continue;
                }
                let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
                let g = gcd(dx, dy);
                let (mut a, mut b) = (dx / g, dy / g);
                if a < 0 || (a == 0 && b < 0) {
                    (a, b) = (-a, -b);
                }
                let offset = a * p[1] - b * p[0];
                let along = |at: [i128; 2]| a * at[0] + b * at[1];
                let span = if along(p) <= along(q) {
                    (along(p), p, along(q), q)
                } else {
                    (along(q), q, along(p), p)
                };
                lines
                    .entry((layer.clone(), width, a, b, offset))
                    .or_default()
                    .push(span);
            }
        }
        // Only the merged pieces' ends are rounded, to the micrometre.
        let um = |ticks: i128| (ticks as f64 * um_per_unit / TICKS_PER_UNIT).round() as i64;
        let end = |span: &Span| [um(span.1[0]), um(span.1[1]), um(span.3[0]), um(span.3[1])];
        let mut pieces = Vec::new();
        for ((layer, width, _, _, _), mut spans) in lines {
            spans.sort();
            let mut current = spans[0];
            for span in &spans[1..] {
                if span.0 <= current.2 {
                    // Touching or overlapping: one piece.
                    if span.2 > current.2 {
                        current.2 = span.2;
                        current.3 = span.3;
                    }
                } else {
                    pieces.push((layer.clone(), width, end(&current)));
                    current = *span;
                }
            }
            pieces.push((layer, width, end(&current)));
        }
        pieces.sort();
        let mut net_vias = vias.remove(&name).unwrap_or_default();
        net_vias.sort();
        result.insert(
            name,
            NetCopper {
                pieces,
                vias: net_vias,
            },
        );
    }
    Ok(result)
}

/// Whether a net's copper differs between the sessions, for [`changed_outside`]: its vias
/// differ, `after` holds a piece `before` lacks, or `before` holds a piece `after` lacks
/// that is not a via stub. A *via stub* is a missing held piece whose two ends both lie
/// within the copper disc of one of the net's own vias in `before` (the padstack's
/// smallest copper radius in `via_radii`, µm): it connects nothing the via does not, and
/// it is what fastroute's load-time trace-end bridges leave out of the session it
/// writes back (FT6). A piece that reaches a micrometre past the disc is not a stub, and
/// neither is one between two vias (one via must hold both ends). The disc is taken on
/// any copper layer: Konnect's export makes every via a through via with one equal circle
/// per copper layer (`specctra.rs`, the via padstacks, and the refusal of blind, buried
/// and unlocked vias), so a via's disc is the same on every layer.
fn copper_differs(
    before: &NetCopper,
    after: &NetCopper,
    via_radii: &BTreeMap<String, i64>,
) -> bool {
    if before == after {
        return false;
    }
    if before.vias != after.vias {
        return true;
    }
    if after
        .pieces
        .iter()
        .any(|piece| !before.pieces.contains(piece))
    {
        return true;
    }
    let inside = |at: [i64; 2], via: &(String, [i64; 2]), radius: i64| {
        let (dx, dy) = (i128::from(at[0] - via.1[0]), i128::from(at[1] - via.1[1]));
        dx * dx + dy * dy <= i128::from(radius) * i128::from(radius)
    };
    before
        .pieces
        .iter()
        .filter(|piece| !after.pieces.contains(piece))
        .any(|(_, _, [x1, y1, x2, y2])| {
            !before.vias.iter().any(|via| {
                via_radii.get(&via.0).is_some_and(|radius| {
                    inside([*x1, *y1], via, *radius) && inside([*x2, *y2], via, *radius)
                })
            })
        })
}

/// The nets outside `nets` whose copper differs between two sessions (one appears
/// in only one of them, or their [`NetCopper`] differ, [`copper_differs`]), sorted: the
/// Tune pass's guard (L6). A net that has no copper in one session and is missing from
/// the other is not different. `via_radii` is [`crate::specctra::via_disc_radii`] of the
/// call's DSN.
pub(crate) fn changed_outside(
    before: &str,
    after: &str,
    nets: &BTreeSet<String>,
    via_radii: &BTreeMap<String, i64>,
) -> Result<Vec<String>> {
    let before = copper_by_net(before).context("the session before the Tune pass")?;
    let after = copper_by_net(after).context("the session after the Tune pass")?;
    let empty = NetCopper::default();
    let names: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    Ok(names
        .into_iter()
        .filter(|name| !nets.contains(*name))
        .filter(|name| {
            copper_differs(
                before.get(*name).unwrap_or(&empty),
                after.get(*name).unwrap_or(&empty),
                via_radii,
            )
        })
        .cloned()
        .collect())
}

/// How two pieces of one net touch (L21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ContactKind {
    /// The centre lines cross.
    Crossing,
    /// Two distinct collinear pieces sharing a stretch longer than
    /// [`CONTACT_MARGIN_UM`] (exact repeats are one piece, never an overlap).
    Overlap,
    /// Apart, but closer than half the sum of their widths minus
    /// [`CONTACT_MARGIN_UM`].
    Touch,
}

/// One self contact of a net's session copper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub(crate) struct SelfContact {
    pub layer: String,
    pub kind: ContactKind,
    /// The two pieces, as (width µm, [x1, y1, x2, y2] µm), each rounded to 1 µm and
    /// ordered: what L21 compares with the held session.
    pub pieces: [(i64, [i64; 4]); 2],
}

/// A straight piece of a net on one layer, in ticks, its ends ordered.
#[derive(Debug, Clone, Copy)]
struct Piece {
    width_um: i64,
    a: [i128; 2],
    b: [i128; 2],
}

/// The cross product of `a - o` and `b - o`.
fn cross(o: [i128; 2], a: [i128; 2], b: [i128; 2]) -> i128 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// Whether `p`, a point on the line of `piece`, lies within the piece's ends.
fn within(piece: &Piece, p: [i128; 2]) -> bool {
    (0..2).all(|axis| {
        piece.a[axis].min(piece.b[axis]) <= p[axis] && p[axis] <= piece.a[axis].max(piece.b[axis])
    })
}

/// The distance from `p` to `piece`, in ticks.
fn point_distance(p: [i128; 2], piece: &Piece) -> f64 {
    let (a, b) = (piece.a, piece.b);
    let (dx, dy) = ((b[0] - a[0]) as f64, (b[1] - a[1]) as f64);
    let (px, py) = ((p[0] - a[0]) as f64, (p[1] - a[1]) as f64);
    let length_squared = dx * dx + dy * dy;
    let t = ((px * dx + py * dy) / length_squared).clamp(0.0, 1.0);
    let (ex, ey) = (px - t * dx, py - t * dy);
    (ex * ex + ey * ey).sqrt()
}

/// How `first` and `second`, two pieces of one net on one layer, touch (L21), if
/// they do; `ticks_per_um` converts the session's ticks.
fn piece_contact(first: &Piece, second: &Piece, ticks_per_um: f64) -> Option<ContactKind> {
    let shared_end = [first.a, first.b]
        .iter()
        .any(|end| *end == second.a || *end == second.b);
    let side = [
        cross(first.a, first.b, second.a),
        cross(first.a, first.b, second.b),
    ];
    if side == [0, 0] {
        // Collinear: the stretch they share along their common line.
        let direction = [first.b[0] - first.a[0], first.b[1] - first.a[1]];
        let along = |p: [i128; 2]| direction[0] * p[0] + direction[1] * p[1];
        let (second_lo, second_hi) = (
            along(second.a).min(along(second.b)),
            along(second.a).max(along(second.b)),
        );
        let (lo, hi) = (along(first.a).max(second_lo), along(first.b).min(second_hi));
        if hi > lo {
            let norm = ((direction[0] * direction[0] + direction[1] * direction[1]) as f64).sqrt();
            if (hi - lo) as f64 / norm / ticks_per_um > CONTACT_MARGIN_UM as f64 {
                return Some(ContactKind::Overlap);
            }
        }
        if shared_end {
            return None;
        }
    } else {
        let other = [
            cross(second.a, second.b, first.a),
            cross(second.a, second.b, first.b),
        ];
        let opposite =
            |pair: [i128; 2]| (pair[0] > 0 && pair[1] < 0) || (pair[0] < 0 && pair[1] > 0);
        if opposite(side) && opposite(other) {
            return Some(ContactKind::Crossing);
        }
        // A junction: one piece's end lies on the other piece (two pieces that share
        // an end point are one: that point is on both).
        if (side[0] == 0 && within(first, second.a))
            || (side[1] == 0 && within(first, second.b))
            || (other[0] == 0 && within(second, first.a))
            || (other[1] == 0 && within(second, first.b))
        {
            return None;
        }
    }
    let nearest = [
        point_distance(first.a, second),
        point_distance(first.b, second),
        point_distance(second.a, first),
        point_distance(second.b, first),
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min);
    let reach_um = (first.width_um + second.width_um) as f64 / 2.0 - CONTACT_MARGIN_UM as f64;
    (nearest / ticks_per_um < reach_um).then_some(ContactKind::Touch)
}

/// A piece as the held comparison names it: width and ends in µm, each end rounded
/// to 1 µm and the ends ordered.
fn piece_key(piece: &Piece, um_per_unit: f64) -> (i64, [i64; 4]) {
    let um = |ticks: i128| (ticks as f64 * um_per_unit / TICKS_PER_UNIT).round() as i64;
    let (a, b) = (
        [um(piece.a[0]), um(piece.a[1])],
        [um(piece.b[0]), um(piece.b[1])],
    );
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    (piece.width_um, [a[0], a[1], b[0], b[1]])
}

/// The one place that decides what an exact repeat is (L21): pieces of one net
/// with the same layer, width and end points (either order) are one piece, compared
/// exactly as `specctra_ses::parse_network` compares them (its importer skips the
/// copies, so neither the judged board nor KiCad's length holds them; fastroute
/// copies its trace-end bridges when it loads a session). `by_layer` is one net's
/// pieces by layer; the first of each repeat stays.
///
/// The key is not the importer's exactly: the width is rounded to 1 µm
/// ([`session_um`]) and the ends are in the session's ticks, where the importer compares
/// exact floats. Two pieces whose widths differ by less than half a micrometre merge
/// here and are two tracks in KiCad. fastroute writes widths on its own grid, so the two
/// keys agree on every session it writes; this pins the difference rather than hides it.
fn merge_exact_repeats(by_layer: &mut BTreeMap<&str, Vec<Piece>>) {
    for pieces in by_layer.values_mut() {
        let mut seen = BTreeSet::new();
        pieces.retain(|piece| seen.insert((piece.width_um, piece.a, piece.b)));
    }
}

/// The straight pieces of a net's raw paths by layer (a piece of no length is no
/// copper), each with its ends ordered, exact repeats merged
/// ([`merge_exact_repeats`]).
fn raw_pieces(paths: &[SessionPath]) -> BTreeMap<&str, Vec<Piece>> {
    let mut by_layer: BTreeMap<&str, Vec<Piece>> = BTreeMap::new();
    for (layer, width, points) in paths {
        for pair in points.windows(2) {
            if pair[0] == pair[1] {
                continue;
            }
            let (a, b) = if pair[0] <= pair[1] {
                (pair[0], pair[1])
            } else {
                (pair[1], pair[0])
            };
            by_layer.entry(layer).or_default().push(Piece {
                width_um: *width,
                a,
                b,
            });
        }
    }
    merge_exact_repeats(&mut by_layer);
    by_layer
}

/// Whether two pieces are so far apart that nothing of L21 can hold between them
/// (a speed-up of the pairwise scan): their bounding boxes are farther apart on
/// an axis than half the sum of their widths plus the margin, which is more than a
/// touch needs, so no pair that [`piece_contact`] names is ever skipped.
fn too_far_apart(first: &Piece, second: &Piece, ticks_per_um: f64) -> bool {
    let reach_um = (first.width_um + second.width_um) as f64 / 2.0 + CONTACT_MARGIN_UM as f64;
    let reach = (reach_um * ticks_per_um).ceil() as i128;
    (0..2).any(|axis| {
        first.a[axis].max(first.b[axis]) + reach < second.a[axis].min(second.b[axis])
            || second.a[axis].max(second.b[axis]) + reach < first.a[axis].min(first.b[axis])
    })
}

/// Every self contact (L21) of one net, from its raw paths: pieces of the net on one
/// layer, compared pairwise.
fn net_contacts(paths: &[SessionPath], um_per_unit: f64) -> Vec<SelfContact> {
    let ticks_per_um = TICKS_PER_UNIT / um_per_unit;
    let mut contacts = Vec::new();
    for (layer, pieces) in raw_pieces(paths) {
        for (at, first) in pieces.iter().enumerate() {
            for second in &pieces[at + 1..] {
                if too_far_apart(first, second, ticks_per_um) {
                    continue;
                }
                let Some(kind) = piece_contact(first, second, ticks_per_um) else {
                    continue;
                };
                let mut pieces = [
                    piece_key(first, um_per_unit),
                    piece_key(second, um_per_unit),
                ];
                pieces.sort();
                contacts.push(SelfContact {
                    layer: layer.to_string(),
                    kind,
                    pieces,
                });
            }
        }
    }
    contacts
}

/// Every self contact (L21) of each net of `nets` in a session, from its raw
/// paths (pieces are not merged), after its exact repeats are taken as one piece
/// ([`merge_exact_repeats`]). Pieces of one net on
/// one layer are compared pairwise. Pieces that share an end point are compared only
/// for an overlap; a junction (one piece's end on the other, not collinear) is no
/// contact. Only nets with a contact are in the map. A session that does not parse
/// is an error.
pub(crate) fn self_contacts(
    ses_text: &str,
    nets: &BTreeSet<String>,
) -> Result<BTreeMap<String, Vec<SelfContact>>> {
    let raw = raw_copper(ses_text)?;
    Ok(raw
        .paths
        .iter()
        .filter(|(name, _)| nets.contains(*name))
        .map(|(name, paths)| (name.clone(), net_contacts(paths, raw.um_per_unit)))
        .filter(|(_, contacts)| !contacts.is_empty())
        .collect())
}

/// The pieces of a net's raw paths, exact repeats merged: layer, width and ends to
/// 1 µm.
fn piece_keys(paths: &[SessionPath], um_per_unit: f64) -> BTreeSet<(&str, (i64, [i64; 4]))> {
    raw_pieces(paths)
        .into_iter()
        .flat_map(|(layer, pieces)| {
            pieces
                .into_iter()
                .map(move |piece| (layer, piece_key(&piece, um_per_unit)))
        })
        .collect()
}

/// The nets of `nets` with a self contact after the Tune pass that involves a piece
/// not in the held session's raw pieces of that net, with those contacts. Both
/// sessions are taken with their exact repeats merged ([`merge_exact_repeats`]); a
/// piece is new when no held piece has the same layer, width and ends, rounded to
/// 1 µm.
pub(crate) fn new_self_contacts(
    before: &str,
    after: &str,
    nets: &BTreeSet<String>,
) -> Result<BTreeMap<String, Vec<SelfContact>>> {
    let held = raw_copper(before).context("the session before the Tune pass")?;
    let tuned = raw_copper(after).context("the session after the Tune pass")?;
    let mut found = BTreeMap::new();
    for (name, paths) in &tuned.paths {
        if !nets.contains(name) {
            continue;
        }
        let contacts = net_contacts(paths, tuned.um_per_unit);
        if contacts.is_empty() {
            continue;
        }
        let known = held
            .paths
            .get(name)
            .map(|paths| piece_keys(paths, held.um_per_unit))
            .unwrap_or_default();
        let new: Vec<SelfContact> = contacts
            .into_iter()
            .filter(|contact| {
                contact
                    .pieces
                    .iter()
                    .any(|piece| !known.contains(&(contact.layer.as_str(), *piece)))
            })
            .collect();
        if !new.is_empty() {
            found.insert(name.clone(), new);
        }
    }
    Ok(found)
}

/// One Tune candidate that passed its keep rule (L21, L24).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct TuneCandidate {
    pub variant: TuneVariant,
    /// Its session.
    pub ses: PathBuf,
    pub unrouted: usize,
    /// Nets dropped from the tune file in its rounds, each with its reason.
    pub dropped: Vec<(String, String)>,
    /// Passes it ran (1 to [`TUNE_ROUNDS_MAX`]).
    pub passes: usize,
}

/// What the Tune attempts of one run gave.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(crate) struct TuneOutcome {
    /// Kept candidates, `Free` first.
    pub candidates: Vec<TuneCandidate>,
    /// One row per Tune pass run, as every pass row, plus `variant` and `round`.
    pub rows: Vec<Value>,
    pub seconds: f64,
}

/// One row of [`TuneOutcome::rows`]: a pass row with its `variant` and `round`.
fn tune_row(
    variant: TuneVariant,
    round: usize,
    outcome: &str,
    unrouted: Option<usize>,
    seconds: Option<f64>,
    reason: Option<&str>,
) -> Value {
    let mut row = pass_row(TUNE_PASS, outcome, unrouted, seconds, reason);
    row["variant"] = json!(variant);
    row["round"] = json!(round);
    row
}

/// Row LT, L21: whether `remaining` leaves room for another Tune pass after one
/// that took `last_pass_seconds`: at least twice as long (a pass killed at the
/// deadline would end the run, D4).
fn room_for_another_pass(remaining: Duration, last_pass_seconds: f64) -> bool {
    remaining.as_secs_f64() >= 2.0 * last_pass_seconds
}

/// [`room_for_another_pass`] for the time `deadline` leaves from now.
fn deadline_leaves_room(deadline: Instant, last_pass_seconds: f64) -> bool {
    room_for_another_pass(
        deadline.saturating_duration_since(Instant::now()),
        last_pass_seconds,
    )
}

/// `A, B, C, D, E and 3 more`: at most [`MAX_NAMED_NETS`] nets named.
fn named_nets(nets: &[String]) -> String {
    let mut named = nets
        .iter()
        .take(MAX_NAMED_NETS)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if nets.len() > MAX_NAMED_NETS {
        named.push_str(&format!(" and {} more", nets.len() - MAX_NAMED_NETS));
    }
    named
}

/// What a net's new contacts were, for the reason it was dropped: `2 crossings, 1
/// overlap`.
fn contact_summary(contacts: &[SelfContact]) -> String {
    let count = |kind: ContactKind| contacts.iter().filter(|c| c.kind == kind).count();
    let parts: Vec<String> = [
        (ContactKind::Crossing, "crossing"),
        (ContactKind::Overlap, "overlap"),
        (ContactKind::Touch, "touch"),
    ]
    .into_iter()
    .filter_map(|(kind, word)| match count(kind) {
        0 => None,
        1 => Some(format!("1 {word}")),
        n => Some(format!(
            "{n} {word}{}",
            if word == "touch" { "es" } else { "s" }
        )),
    })
    .collect();
    format!("its copper touches itself: {}", parts.join(", "))
}

/// The time source of [`tune_attempt`]'s room rule: `Instant::now` in production, a fixed
/// time in the tests of the rule, so that they do not depend on how fast a process runs.
pub(crate) type Clock<'a> = &'a (dyn Fn() -> Instant + Sync);

/// What comparing a Tune pass's session with the held one found: the nets outside
/// the tune file it changed, and the nets whose new copper touches itself (L21).
struct Compared {
    changed: Vec<String>,
    contacts: BTreeMap<String, Vec<SelfContact>>,
}

/// One candidate's Tune passes (L20, L21), from `held_ses`, in
/// `folder/<variant folder>`:
/// 1. A pass with the current tune file ([`run_tune_pass`], with [`TUNE_POUR_SAFE`]
///    for `PourSafe`).
/// 2. Killed: `PassError::Killed` ends the run. Failed: a `failed` row, no candidate.
/// 3. Opened connections (against `held_unrouted`), or changed another net's copper
///    ([`changed_outside`]): `discarded` with today's reasons, no candidate.
/// 4. [`new_self_contacts`] names nets: they join the dropped set, `next` writes the
///    next round's file, and the next round runs. After [`TUNE_ROUNDS_MAX`] rounds,
///    when `next` gives `None`, or when the deadline leaves less than twice the last
///    pass's time: `discarded` ("meanders touch their own net: …", at most 5 named),
///    no candidate.
/// 5. Else the candidate, with every net dropped on the way.
///
/// `first` is the round-1 inputs (the call's tune file); the rows go into `outcome`.
/// A round whose pass is passed over for contacts is a `discarded` row too, so every
/// pass run has one row. The row of the pass a candidate comes from says `candidate`
/// until the judge chooses (L23, `route_judge::TuneChoice::apply`). A discard for lack of
/// rounds or time says so ("(no round left)", "(the deadline leaves no room)"). `clock`
/// gives the time the room rule reads.
#[allow(clippy::too_many_arguments)] // the parts of one pass, each read once
pub(crate) async fn tune_attempt(
    binary: &Binary,
    folder: &Path,
    variant: TuneVariant,
    dsn: &Path,
    hints: &RoutingHints,
    files: CallFiles<'_>,
    first: &TuneInputs<'_>,
    next: NextTuneFile<'_>,
    held_ses: &Path,
    held_unrouted: usize,
    deadline: Instant,
    clock: Clock<'_>,
    outcome: &mut TuneOutcome,
) -> std::result::Result<Option<TuneCandidate>, PassError> {
    // The Tune pass gets its own heights and never the call's pairs file, nor the
    // group search's uncounted file (Row GS, G1), nor the zones file (Row ZR).
    let files = CallFiles {
        amps: files.amps,
        pairs: None,
        layer_heights: first.layer_heights,
        uncounted: None,
        zones: None,
    };
    let mut tune_file = first.tune.to_path_buf();
    let mut nets = first.nets.clone();
    let mut dropped: Vec<(String, String)> = Vec::new();
    let mut dropped_nets: BTreeSet<String> = BTreeSet::new();
    for round in 1..=TUNE_ROUNDS_MAX {
        let row = |outcome: &str, unrouted, seconds, reason: Option<&str>| {
            tune_row(variant, round, outcome, unrouted, seconds, reason)
        };
        let pass_folder = folder.join(variant.folder_name(round));
        let inputs = TuneInputs {
            tune: &tune_file,
            classes: first.classes,
            nets: &nets,
            layer_heights: first.layer_heights,
            planes: first.planes,
            via_radii: first.via_radii,
        };
        let result = match std::fs::create_dir_all(&pass_folder) {
            Ok(()) => {
                run_tune_pass(
                    binary,
                    &pass_folder,
                    variant,
                    dsn,
                    hints,
                    files,
                    &inputs,
                    held_ses,
                    deadline,
                )
                .await
            }
            Err(error) => Err(PassError::Failed(anyhow!(
                "{} could not be created: {error}",
                pass_folder.display()
            ))),
        };
        let pass = match result {
            Ok(pass) => pass,
            Err(PassError::Failed(error)) => {
                outcome.rows.push(row(
                    "failed",
                    None,
                    None,
                    Some(&tail(&format!("{error:#}"), QUOTE_CHARS)),
                ));
                return Ok(None);
            }
            Err(killed) => return Err(killed),
        };
        outcome.seconds += pass.seconds;
        let discard = |reason: String| {
            row(
                "discarded",
                Some(pass.unrouted),
                Some(pass.seconds),
                Some(&reason),
            )
        };
        if pass.unrouted > held_unrouted {
            outcome
                .rows
                .push(discard(format!("opened {}", pass.unrouted - held_unrouted)));
            return Ok(None);
        }
        let compared = std::fs::read_to_string(held_ses)
            .with_context(|| format!("{} could not be read", held_ses.display()))
            .and_then(|before| {
                std::fs::read_to_string(&pass.ses)
                    .with_context(|| format!("{} could not be read", pass.ses.display()))
                    .and_then(|after| {
                        let changed = changed_outside(&before, &after, &nets, first.via_radii)?;
                        let contacts = if changed.is_empty() {
                            new_self_contacts(&before, &after, &nets)?
                        } else {
                            BTreeMap::new()
                        };
                        Ok(Compared { changed, contacts })
                    })
            });
        let compared = match compared {
            Ok(compared) => compared,
            Err(error) => {
                outcome.rows.push(discard(tail(
                    &format!("the sessions could not be compared: {error:#}"),
                    QUOTE_CHARS,
                )));
                return Ok(None);
            }
        };
        if !compared.changed.is_empty() {
            outcome.rows.push(discard(tail(
                &format!("changed the copper of {}", named_nets(&compared.changed)),
                QUOTE_CHARS,
            )));
            return Ok(None);
        }
        if compared.contacts.is_empty() {
            outcome.rows.push(row(
                "candidate",
                Some(pass.unrouted),
                Some(pass.seconds),
                None,
            ));
            return Ok(Some(TuneCandidate {
                variant,
                ses: pass.ses,
                unrouted: pass.unrouted,
                dropped,
                passes: round,
            }));
        }

        // Some nets touch themselves: tune again without them, when a round is left
        // and the deadline leaves room for it.
        let touching: Vec<String> = compared.contacts.keys().cloned().collect();
        let reason = tail(
            &format!("meanders touch their own net: {}", named_nets(&touching)),
            QUOTE_CHARS,
        );
        let room = room_for_another_pass(deadline.saturating_duration_since(clock()), pass.seconds);
        let again = if round < TUNE_ROUNDS_MAX && room {
            dropped_nets.extend(touching.iter().cloned());
            let next_folder = folder.join(variant.folder_name(round + 1));
            std::fs::create_dir_all(&next_folder)
                .map_err(|error| anyhow!("{} could not be created: {error}", next_folder.display()))
                .and_then(|()| next(&next_folder, &dropped_nets))
        } else {
            Ok(None)
        };
        match again {
            Ok(Some((path, listed))) => {
                for net in nets.difference(&listed) {
                    let why = match compared.contacts.get(net) {
                        Some(contacts) => contact_summary(contacts),
                        None => "left out with a net that touched itself (its pair partner, or a group left too small)".to_string(),
                    };
                    dropped.push((net.clone(), why));
                }
                outcome
                    .rows
                    .push(discard(format!("{reason}; tuned again without them")));
                tune_file = path;
                nets = listed;
            }
            Ok(None) => {
                // Why there is no next round: the limit, the time, or no group left.
                let why_not = if round >= TUNE_ROUNDS_MAX {
                    " (no round left)"
                } else if !room {
                    " (the deadline leaves no room)"
                } else {
                    ""
                };
                outcome.rows.push(discard(format!("{reason}{why_not}")));
                return Ok(None);
            }
            Err(error) => {
                outcome.rows.push(discard(tail(
                    &format!("{reason} (the next tune file could not be written: {error:#})"),
                    QUOTE_CHARS,
                )));
                return Ok(None);
            }
        }
    }
    Ok(None)
}

/// The Tune attempts after the hold sequence (L6, L20, L21, L24), from `held.ses`:
/// 1. [`tune_attempt`] for [`TuneVariant::Free`], in `folder/tune`.
/// 2. [`tune_attempt`] for [`TuneVariant::PourSafe`], in `folder/tune-pour-safe`,
///    only when the DSN has a plane of a net outside the tune file
///    ([`TuneInputs::planes`]) and the deadline leaves at least twice the longest
///    Tune pass so far (a pass killed at the deadline ends the run, and the free
///    candidate would go with it).
///
/// A pass killed by `deadline` ends the run (D4). The rows of every pass are in
/// [`Held::tune`], and the `Free` attempt's last row joins `held.passes`; `held.seconds`
/// adds every pass's time. When a candidate exists `held.untuned` is the session
/// the routing passes kept.
///
/// `held.ses`, `held.kept` and `held.unrouted` stay the routing passes' (the untuned
/// board) whatever the candidates are: the judge judges the untuned session and each
/// candidate and chooses (L23), and `route_judge::TuneChoice::apply` then rewrites
/// them from the choice. A run that is not judged is never a candidate, so nothing
/// tuned reaches the output without the judge's choice.
#[allow(clippy::too_many_arguments)] // the parts of one pass, each read once
pub(crate) async fn tune_held(
    binary: &Binary,
    folder: &Path,
    dsn: &Path,
    hints: &RoutingHints,
    files: CallFiles<'_>,
    request: &TuneRequest<'_>,
    mut held: Held,
    deadline: Instant,
) -> std::result::Result<Held, PassError> {
    let first = &request.inputs;
    let mut outcome = TuneOutcome::default();
    let untuned = held.ses.clone();
    let free = tune_attempt(
        binary,
        folder,
        TuneVariant::Free,
        dsn,
        hints,
        files,
        first,
        request.next,
        &untuned,
        held.unrouted,
        deadline,
        &Instant::now,
        &mut outcome,
    )
    .await?;
    if let Some(candidate) = &free {
        outcome.candidates.push(candidate.clone());
    }
    if first.planes.iter().any(|net| !first.nets.contains(net)) {
        let longest = outcome
            .rows
            .iter()
            .filter_map(|row| row["seconds"].as_f64())
            .fold(0.0, f64::max);
        let room = deadline_leaves_room(deadline, longest);
        if room {
            let pour_safe = tune_attempt(
                binary,
                folder,
                TuneVariant::PourSafe,
                dsn,
                hints,
                files,
                first,
                request.next,
                &untuned,
                held.unrouted,
                deadline,
                &Instant::now,
                &mut outcome,
            )
            .await?;
            outcome.candidates.extend(pour_safe);
        } else {
            outcome.rows.push(tune_row(
                TuneVariant::PourSafe,
                1,
                "skipped",
                None,
                None,
                Some("the deadline leaves less than twice the time of a Tune pass"),
            ));
        }
    }
    held.seconds += outcome.seconds;
    if let Some(row) = outcome
        .rows
        .iter()
        .rev()
        .find(|row| row["variant"] == json!(TuneVariant::Free))
    {
        held.passes.push(row.clone());
    }
    if !outcome.candidates.is_empty() {
        held.untuned = Some(untuned);
    }
    held.tune = outcome;
    Ok(held)
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
