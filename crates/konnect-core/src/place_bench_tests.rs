//! The placement bench, an ignored test: it runs the placement search on copies of
//! boards and changes nothing in the product. One run per board named in the bench
//! folder's `bench.json`:
//!
//! ```text
//! KONNECT_PLACE_BENCH=<bench folder> KONNECT_FASTROUTE=<candidate fastroute>
//! KICAD_CONFIG_HOME=<scratch> KONNECT_STATE_DIR=<scratch>
//! cargo test -p konnect-core --release place_bench -- --ignored --nocapture
//! ```
//!
//! `bench.json` names, per board: its folder below the bench folder (holding
//! `input/` with the stripped project copy, `rules.json`, `held.json` and optionally
//! `ai/board.kicad_pcb`), its time limit, the effort, `hold` (`locks` or `rules`),
//! `families` and `converge_terms`. The bench writes only below
//! the bench folder. The format is in `xtask/placement-bench/README.md`.
//!
//! `converge_check`, the other ignored test here, runs family C without routing (design
//! measurement step 1) on every board that names `converge`, with the same guards, and
//! writes `<board folder>/converge-check.json`. With `KONNECT_CONVERGE_SILK=1` it also
//! reads each run's silk entries and parts pulled away on the run's board (design A3),
//! which starts kicad-cli.

use crate::place_converge::{converge, ConvergeParams, OuterRule, Terms, Trace};
use crate::place_generators::{
    generate, recipes, BoardModel, Family, GenerateFailure, Matrix, Params, Recipe,
    GENERATOR_VERSION,
};
use crate::place_held::HoldRules;
use crate::place_judge::SILK;
use crate::place_route::{
    export_candidate, export_options, isolated_context, rules_from_json, RouteInputs,
};
use crate::place_schematic::SchematicBaseline;
use crate::place_search::{
    converge_all, generate_all, search, search_model, silk_entries, Origin, PlaceRouteCall, Plan,
    Record, RouteCall, Routed,
};
use crate::route_search::Effort;
use anyhow::{bail, ensure, Context};
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// One board of `bench.json`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct BenchBoard {
    /// The board's bench name, as the table titles it.
    pub name: String,
    /// Its folder below the bench folder.
    pub folder: String,
    /// The board file in `<folder>/input/`; the only `.kicad_pcb` there when absent.
    #[serde(default)]
    pub board: Option<String>,
    /// The board's whole time limit in minutes, generation included (default 100).
    #[serde(default = "default_minutes")]
    pub time_limit_minutes: u64,
    /// `fast`, `m`, `h` or `xh` (default `m`).
    #[serde(default = "default_effort")]
    pub effort: String,
    /// Which parts the search holds: `locks` (default) or `rules`.
    #[serde(default)]
    pub hold: HoldMode,
    /// The families of the matrix: any of `force`, `schematic`, `perturb` and
    /// `converge` (default `force` and `schematic`; `converge` needs one of the first two).
    /// O and A are references whatever is named.
    #[serde(default = "default_families")]
    pub families: Vec<BenchFamily>,
    /// Family C's two new cost terms, the schematic neighbours and the silk labels: `on`
    /// (default) or `off` (both weights 0, the design's measurement step 3).
    #[serde(default)]
    pub converge_terms: ConvergeTerms,
}

/// The bench's `converge_terms` option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ConvergeTerms {
    /// `ConvergeParams::measured`'s neighbour weight and label cost.
    #[default]
    On,
    /// `neighbour_weight` 0 and `label_mm` 0.
    Off,
}

/// The bench's `hold` option, the owner's flow of 2026-11-19 or the earlier rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum HoldMode {
    /// Held: KiCad's locks, `held.json`'s list, and the parts with no pad on a net.
    /// The owner locks the parts he wants fixed and the search places the rest.
    #[default]
    Locks,
    /// `HoldRules::default()` too: connectors, mechanical prefixes and footprint words.
    Rules,
}

/// A family of `bench.json`'s `families` option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum BenchFamily {
    Force,
    Schematic,
    Perturb,
    /// C, the convergent search (`place_converge`), which refines F's and S's candidates.
    Converge,
}

fn default_families() -> Vec<BenchFamily> {
    vec![BenchFamily::Force, BenchFamily::Schematic]
}

impl BenchBoard {
    /// The hold rules the board's `hold` option names.
    pub(crate) fn hold_rules(&self) -> HoldRules {
        match self.hold {
            HoldMode::Locks => HoldRules::locks_only(),
            HoldMode::Rules => HoldRules::default(),
        }
    }

    /// `Matrix::bench` with only the board's `families`; with `converge`, family C at
    /// `ConvergeParams::measured(0)` and `OuterRule::measured()` (design C7), its two new
    /// terms off when `converge_terms` is `off`.
    pub(crate) fn matrix(&self) -> Matrix {
        let families: Vec<Family> = self
            .families
            .iter()
            .map(|family| match family {
                BenchFamily::Force => Family::Force,
                BenchFamily::Schematic => Family::Schematic,
                BenchFamily::Perturb => Family::Perturb,
                BenchFamily::Converge => Family::Converge,
            })
            .collect();
        let mut matrix = Matrix::bench().only(&families);
        if families.contains(&Family::Converge) {
            let mut params = ConvergeParams::measured(0);
            if self.converge_terms == ConvergeTerms::Off {
                params.neighbour_weight = 0.0;
                params.label_mm = 0.0;
            }
            matrix.converge = Some((params, OuterRule::measured()));
        }
        matrix
    }

    /// Refuses an empty `families`, and `converge` without `force` or `schematic`:
    /// family C's starts are their candidates (design C1, C8), so without them it has
    /// nothing to refine.
    pub(crate) fn check_families(&self) -> anyhow::Result<()> {
        ensure!(
            !self.families.is_empty(),
            "board {}: families names no family (use force, schematic, perturb, converge)",
            self.name
        );
        ensure!(
            !self.families.contains(&BenchFamily::Converge)
                || self
                    .families
                    .iter()
                    .any(|f| matches!(f, BenchFamily::Force | BenchFamily::Schematic)),
            "board {}: the family converge refines the candidates of force and schematic; name at least one of them with it",
            self.name
        );
        Ok(())
    }
}

fn default_minutes() -> u64 {
    100
}

fn default_effort() -> String {
    "m".to_string()
}

#[derive(Debug, Deserialize)]
struct BenchFile {
    boards: Vec<BenchBoard>,
}

/// The checked environment of one bench run.
#[derive(Debug, Clone)]
pub(crate) struct BenchEnv {
    pub folder: PathBuf,
    pub fastroute: PathBuf,
    pub kicad_cli: String,
    pub kicad_config_home: PathBuf,
    pub state_dir: PathBuf,
    /// The boards of `bench.json`, whose inputs the guards resolved. Added to the
    /// skeleton: the guards read the file, so the run reuses what they read.
    pub boards: Vec<BenchBoard>,
}

/// The values the guards read, so that a test passes its own and never the process's.
#[derive(Debug, Clone, Default)]
pub(crate) struct RawEnv {
    pub bench: Option<String>,
    pub fastroute: Option<String>,
    pub kicad_config_home: Option<String>,
    pub state_dir: Option<String>,
    /// The kicad-cli found, if any.
    pub kicad_cli: Option<PathBuf>,
    pub home: Option<PathBuf>,
    /// The system temp folder.
    pub temp: PathBuf,
}

/// `path` with links followed as far as it exists and the rest appended. Refuses a path
/// with `..` in the part that does not exist yet.
fn resolve(path: &Path) -> anyhow::Result<PathBuf> {
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    let mut at = path.to_path_buf();
    loop {
        match std::fs::canonicalize(&at) {
            Ok(real) => {
                let mut out = real;
                out.extend(rest.iter().rev());
                return Ok(out);
            }
            Err(_) => {
                let Some(name) = at.file_name() else {
                    bail!("'{}' cannot be resolved", path.display());
                };
                ensure!(
                    !matches!(at.components().next_back(), Some(Component::ParentDir)),
                    "'{}' has a `..` in a part that does not exist",
                    path.display()
                );
                rest.push(name.to_os_string());
                at = at
                    .parent()
                    .map(Path::to_path_buf)
                    .with_context(|| format!("'{}' cannot be resolved", path.display()))?;
            }
        }
    }
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The real settings folders of KiCad and Konnect, below the home folder.
const REAL_FOLDERS: [&str; 4] = [
    "Library/Preferences/kicad",
    ".local/share/konnect",
    ".config/kicad",
    "Library/Application Support/konnect",
];

/// A scratch setting: set, absolute, strictly below a scratch root, and neither inside
/// nor above a real KiCad or Konnect folder.
fn check_scratch(
    name: &str,
    value: &Option<String>,
    roots: &[PathBuf],
    real: &[PathBuf],
) -> anyhow::Result<PathBuf> {
    let value = value
        .as_deref()
        .filter(|v| !v.is_empty())
        .with_context(|| format!("{name} is not set; give the bench a scratch folder"))?;
    let path = PathBuf::from(value);
    ensure!(
        path.is_absolute(),
        "{name} '{value}' is not an absolute path"
    );
    let path = resolve(&path)?;
    for real in real {
        ensure!(
            !path.starts_with(real) && !real.starts_with(&path),
            "{name} '{}' is, or contains, the real folder '{}'",
            path.display(),
            real.display()
        );
    }
    ensure!(
        roots
            .iter()
            .any(|root| path != *root && path.starts_with(root)),
        "{name} '{}' is not below a scratch root (the bench folder or the system temp folder)",
        path.display()
    );
    Ok(path)
}

/// The input board of `board`: `<folder>/input/<file>`, links followed, which must lie
/// inside `bench`. Also the AI board when the folder has one.
fn check_board_inputs(bench: &Path, board: &BenchBoard) -> anyhow::Result<()> {
    board.check_families()?;
    let folder = Path::new(&board.folder);
    ensure!(
        folder.is_relative()
            && folder
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "board {}: its folder '{}' must be a plain relative path below the bench folder",
        board.name,
        board.folder
    );
    let input = bench.join(folder).join("input");
    let file = match &board.board {
        Some(name) => input.join(name),
        None => {
            let mut found: Vec<PathBuf> = std::fs::read_dir(&input)
                .with_context(|| format!("board {}: read '{}'", board.name, input.display()))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|e| e == "kicad_pcb"))
                .collect();
            ensure!(
                found.len() == 1,
                "board {}: '{}' must hold exactly one .kicad_pcb, it holds {}",
                board.name,
                input.display(),
                found.len()
            );
            found.remove(0)
        }
    };
    let mut files = vec![file];
    let ai = bench.join(folder).join("ai").join("board.kicad_pcb");
    if ai.symlink_metadata().is_ok() {
        files.push(ai);
    }
    for file in files {
        let real = std::fs::canonicalize(&file).with_context(|| {
            format!(
                "board {}: '{}' cannot be resolved",
                board.name,
                file.display()
            )
        })?;
        ensure!(
            real.starts_with(bench),
            "board {}: '{}' resolves to '{}', outside the bench folder",
            board.name,
            file.display(),
            real.display()
        );
    }
    // The folders the run writes (the board's own, `candidates` and `isolation` below it)
    // must be real folders: a link there would send the run's writes wherever it points,
    // even to a place inside the bench folder that another board owns.
    for name in [None, Some("candidates"), Some("isolation")] {
        // No trailing slash for the folder itself: a path ending in one is followed.
        let path = match name {
            Some(name) => bench.join(folder).join(name),
            None => bench.join(folder),
        };
        ensure!(
            !path
                .symlink_metadata()
                .is_ok_and(|meta| meta.file_type().is_symlink()),
            "board {}: '{}' is a link; the run writes there, so it must be a real folder",
            board.name,
            path.display()
        );
    }
    Ok(())
}

/// The guards on the values given (see [`bench_environment`]).
pub(crate) fn check_environment(raw: &RawEnv) -> anyhow::Result<BenchEnv> {
    let bench = raw
        .bench
        .as_deref()
        .filter(|v| !v.is_empty())
        .context("KONNECT_PLACE_BENCH is not set")?;
    ensure!(
        Path::new(bench).is_absolute(),
        "KONNECT_PLACE_BENCH '{bench}' is not an absolute path"
    );
    let bench = std::fs::canonicalize(bench)
        .with_context(|| format!("KONNECT_PLACE_BENCH '{bench}' is not a folder"))?;
    ensure!(
        bench.is_dir(),
        "KONNECT_PLACE_BENCH '{}' is not a folder",
        bench.display()
    );
    let temp = resolve(&raw.temp)?;

    // The real settings of KiCad and Konnect: the bench must never use or reach them.
    // Without the home folder they cannot be named, and so cannot be excluded: refuse.
    let home = raw.home.as_ref().context(
        "the home folder is not known, so the real KiCad and Konnect folders cannot be excluded",
    )?;
    let mut real: Vec<PathBuf> = Vec::new();
    for tail in REAL_FOLDERS {
        real.push(resolve(&home.join(tail))?);
    }
    let roots = [bench.clone(), temp];
    let kicad_config_home =
        check_scratch("KICAD_CONFIG_HOME", &raw.kicad_config_home, &roots, &real)?;
    let state_dir = check_scratch("KONNECT_STATE_DIR", &raw.state_dir, &roots, &real)?;

    let fastroute = raw
        .fastroute
        .as_deref()
        .filter(|v| !v.is_empty())
        .context("KONNECT_FASTROUTE is not set")?;
    let fastroute = PathBuf::from(fastroute);
    ensure!(
        is_executable(&fastroute),
        "KONNECT_FASTROUTE '{}' is not an executable file",
        fastroute.display()
    );
    let kicad_cli = raw
        .kicad_cli
        .as_ref()
        .filter(|path| path.is_file())
        .context("kicad-cli is missing (set KONNECT_KICAD_CLI or install KiCad)")?;

    let file = bench.join("bench.json");
    let text =
        std::fs::read_to_string(&file).with_context(|| format!("read '{}'", file.display()))?;
    let parsed: BenchFile = serde_json::from_str(&text)
        .with_context(|| format!("'{}' is not a bench.json", file.display()))?;
    ensure!(!parsed.boards.is_empty(), "bench.json names no board");
    let mut names = BTreeSet::new();
    for board in &parsed.boards {
        ensure!(
            names.insert(board.name.clone()),
            "bench.json names board {} twice",
            board.name
        );
        check_board_inputs(&bench, board)?;
    }
    Ok(BenchEnv {
        folder: bench,
        fastroute,
        kicad_cli: kicad_cli.display().to_string(),
        kicad_config_home,
        state_dir,
        boards: parsed.boards,
    })
}

/// The bench's guards, before anything runs. Refuses when: `KONNECT_PLACE_BENCH` is
/// unset or not an absolute folder; `KICAD_CONFIG_HOME` or `KONNECT_STATE_DIR` is
/// unset, or is KiCad's or Konnect's real folder (`~/Library/Preferences/kicad`,
/// `~/.config/kicad`, `~/.local/share/konnect`,
/// `~/Library/Application Support/konnect`), or is not below a scratch root (the bench
/// folder or the system temp folder), or the home folder is not known (the real folders
/// could not be excluded); `KONNECT_FASTROUTE` is not an executable file; any input
/// board named in `bench.json` resolves (links followed) to a path outside the bench
/// folder, or a board's folder, its `candidates` or its `isolation` is a link;
/// kicad-cli is missing.
pub(crate) fn bench_environment() -> anyhow::Result<BenchEnv> {
    let var = |name: &str| std::env::var(name).ok();
    check_environment(&RawEnv {
        bench: var("KONNECT_PLACE_BENCH"),
        fastroute: var("KONNECT_FASTROUTE"),
        kicad_config_home: var("KICAD_CONFIG_HOME"),
        state_dir: var("KONNECT_STATE_DIR"),
        kicad_cli: crate::kicad_install::find_cli(&var("KONNECT_KICAD_CLI").unwrap_or_default()),
        home: dirs::home_dir(),
        temp: std::env::temp_dir(),
    })
}

/// The caller's held list of a board: a JSON array of references.
fn read_held(path: &Path) -> anyhow::Result<BTreeSet<String>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("read '{}'", path.display()))?;
    let list: Vec<String> = serde_json::from_str(&text)
        .with_context(|| format!("'{}' is not a JSON array of references", path.display()))?;
    Ok(list.into_iter().collect())
}

/// The plan of one board: the bench's fixed numbers (3 calls per reference, 3 best and 1
/// drawn per family, draw seed 1), and the board's own matrix, hold rules and time limit.
/// Apart from `run_board` so that the wiring of `bench.json`'s options is tested without
/// KiCad.
fn board_plan(
    board: &BenchBoard,
    folder: PathBuf,
    route: RouteInputs,
    ai_board: Option<PathBuf>,
    caller_held: BTreeSet<String>,
) -> Plan {
    Plan {
        name: board.name.clone(),
        folder,
        matrix: board.matrix(),
        threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
        reference_calls: 3,
        best_per_family: 3,
        drawn_per_family: 1,
        draw_seed: 1,
        time_limit: Duration::from_secs(board.time_limit_minutes * 60),
        route,
        ai_board,
        caller_held,
        hold: board.hold_rules(),
        schematic: None,
        routed: Routed::Every,
    }
}

/// The routing inputs of one board as the bench reads them (its input board, `rules.json`,
/// the effort, the export options), its AI board when it has one, and its `held.json`.
fn board_inputs(
    env: &BenchEnv,
    board: &BenchBoard,
    folder: &Path,
) -> anyhow::Result<(RouteInputs, Option<PathBuf>, BTreeSet<String>)> {
    let input = folder.join("input");
    let file = match &board.board {
        Some(name) => input.join(name),
        None => std::fs::read_dir(&input)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "kicad_pcb"))
            .context("no board in input/")?,
    };
    let text = std::fs::read_to_string(&file)?;
    let rules = rules_from_json(&std::fs::read_to_string(folder.join("rules.json"))?, &text)?;
    let effort = Effort::parse(&board.effort).with_context(|| {
        format!(
            "board {}: effort '{}' is not fast, m, h or xh",
            board.name, board.effort
        )
    })?;
    let ai = folder.join("ai").join("board.kicad_pcb");
    let inputs = RouteInputs {
        options: export_options(&file),
        board: file,
        rules,
        effort,
        fastroute: env.fastroute.clone(),
        kicad_cli: env.kicad_cli.clone(),
        kicad_config_home: env.kicad_config_home.clone(),
    };
    Ok((
        inputs,
        ai.is_file().then_some(ai),
        read_held(&folder.join("held.json"))?,
    ))
}

/// One board's search; returns the line to print.
fn run_board(
    env: &BenchEnv,
    board: &BenchBoard,
    runtime: &tokio::runtime::Runtime,
) -> anyhow::Result<String> {
    let folder = env.folder.join(&board.folder);
    ensure!(
        !folder.join("search.json").exists() && !folder.join("candidates").exists(),
        "board {} already holds a run; move or remove its search.json and candidates/ first",
        board.name
    );
    let (inputs, ai, held) = board_inputs(env, board, &folder)?;
    let ctx = isolated_context(&folder.join("isolation"), &env.kicad_cli)?;
    let plan = board_plan(board, folder.clone(), inputs.clone(), ai, held);
    let outcome = runtime.block_on(search(&plan, Arc::new(PlaceRouteCall { inputs, ctx })))?;
    let mut counts = std::collections::BTreeMap::new();
    for record in &outcome.records {
        *counts.entry(record.outcome).or_insert(0usize) += 1;
    }
    Ok(format!(
        "{}: {} candidates in {:.0} s, table {}, counts {counts:?}",
        board.name,
        outcome.records.len(),
        outcome.elapsed.as_secs_f64(),
        folder.join("table.md").display()
    ))
}

/// Every board of `bench.json`: the plan (`Matrix::bench` of the board's families, its
/// hold rules, 3 calls per reference, 3
/// best and 1 drawn per family, draw seed 1, the board's time limit, effort M), the
/// production `RouteCall`
/// with `place_route::isolated_context`, then `place_search::search`. A board that
/// errs is reported and the next one runs. Prints one line per board: its table's
/// path and its counts.
#[test]
#[ignore = "needs KONNECT_PLACE_BENCH, KONNECT_FASTROUTE, KICAD_CONFIG_HOME and KONNECT_STATE_DIR"]
fn place_bench() {
    let env = bench_environment().unwrap_or_else(|error| panic!("the bench is refused: {error:#}"));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    println!(
        "bench folder {}, scratch KiCad settings {}, scratch Konnect state {}, fastroute {}",
        env.folder.display(),
        env.kicad_config_home.display(),
        env.state_dir.display(),
        env.fastroute.display()
    );
    let mut failed = 0usize;
    for board in &env.boards {
        match run_board(&env, board, &runtime) {
            Ok(line) => println!("{line}"),
            Err(error) => {
                failed += 1;
                println!("{}: failed: {error:#}", board.name);
            }
        }
    }
    assert_eq!(failed, 0, "{failed} board(s) failed; see the lines above");
}

// ─── Family C without routing (design measurement step 1) ────────────────────

/// A generator failure as one count-only phrase: `NoRoom`'s list names parts, and
/// `converge-check.json` names none.
fn failure_phrase(failure: &GenerateFailure) -> String {
    match failure {
        GenerateFailure::NoRoom(parts) => format!("no room for {} part(s)", parts.len()),
        GenerateFailure::Invalid(_) => "invalid".to_string(),
        GenerateFailure::Panicked(_) => "panicked".to_string(),
    }
}

/// What one run did, numbers only: converged, both stop reasons, stages, sweeps,
/// evaluations, repairs, the first temperature, what a broken requirement cost, the
/// start and end `Terms`, its wall time and the time of one evaluation.
fn run_fields(trace: &Trace, seconds: Option<f64>) -> serde_json::Map<String, serde_json::Value> {
    let terms = |terms: &Terms| serde_json::to_value(terms).expect("terms are JSON");
    let fields = json!({
        "converged": trace.converged(),
        "anneal_stop": trace.anneal_stop,
        "descent_stop": trace.descent_stop,
        "stages": trace.stages,
        "sweeps": trace.sweeps,
        "evaluations": trace.evaluations,
        "repairs_tried": trace.repairs_tried,
        "repairs_made": trace.repairs_made,
        "first_temperature": trace.first_temperature,
        "requirement_mm": trace.requirement_mm,
        "start": terms(&trace.start),
        "end": terms(&trace.end),
        "seconds": seconds,
        "microseconds_per_evaluation": seconds
            .filter(|_| trace.evaluations > 0)
            .map(|s| s * 1e6 / trace.evaluations as f64),
    });
    match fields {
        serde_json::Value::Object(map) => map,
        _ => unreachable!("an object"),
    }
}

/// One record of family C in `converge-check.json`: its id, its recipe (the start's
/// family, seed and parameter-set index in `matrix`, the level), its outcome, and its
/// run's numbers. No reason: a failure's reason can name parts.
fn converge_run_json(record: &Record, matrix: &Matrix) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    out.insert("id".into(), json!(record.id));
    if let Origin::Generated { recipe, .. } = &record.origin {
        if let Params::Converge(run) = &recipe.params {
            let set = match &run.start.params {
                Params::Force(params) => matrix.force.iter().position(|p| p == params),
                Params::Schematic(params) => matrix.schematic.iter().position(|p| p == params),
                _ => None,
            };
            // Its own key: the run's fields below have a `start` of their own (the terms).
            out.insert(
                "recipe".into(),
                json!({
                    "start": { "family": run.start.family, "seed": run.start.seed, "set": set },
                    "level": run.params.level,
                }),
            );
        }
    }
    out.insert("outcome".into(), json!(record.outcome));
    if let Some(trace) = &record.converge {
        out.insert("cost".into(), json!(trace.end.cost));
        out.extend(run_fields(
            trace,
            record.timings.generate_ms.map(|ms| ms / 1000.0),
        ));
    }
    serde_json::Value::Object(out)
}

/// The port's check (design, measurement step 1): from the F recipe of seed 24 whose
/// parameter set has rail weight 0 (`Matrix::bench`), 5 runs of `converge` at level 0
/// with seeds 1 to 5 and both new terms off, as the experiment ran; each run's numbers
/// and the median of their costs (the lower middle of an even count). Errs when no set
/// has rail weight 0, or the start does not generate.
fn port_check(model: &BoardModel) -> anyhow::Result<serde_json::Value> {
    let bench = Matrix::bench();
    let set = bench
        .force
        .iter()
        .position(|params| params.rail_weight == 0.0)
        .context("no F parameter set of the bench matrix has rail weight 0: the port's check has no start")?;
    let start_recipe = Recipe {
        family: Family::Force,
        seed: 24,
        params: Params::Force(bench.force[set]),
    };
    let start = generate(model, &start_recipe).map_err(|failure| {
        anyhow::anyhow!(
            "the port's start (F seed 24, set {set}) does not generate: {}",
            failure_phrase(&failure)
        )
    })?;
    let params = ConvergeParams {
        neighbour_weight: 0.0,
        label_mm: 0.0,
        ..ConvergeParams::measured(0)
    };
    let seeds: Vec<u64> = (1..=5).collect();
    let results: Vec<(u64, Result<Trace, String>, f64)> = std::thread::scope(|scope| {
        let jobs: Vec<_> = seeds
            .iter()
            .map(|&seed| {
                let (start, params) = (&start, &params);
                scope.spawn(move || {
                    let began = std::time::Instant::now();
                    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        converge(model, model.hoods.as_ref(), &start.placements, seed, params)
                    }));
                    let trace = match run {
                        Ok(Ok((_, trace))) => Ok(trace),
                        Ok(Err(failure)) => Err(failure_phrase(&failure)),
                        Err(_) => Err("panicked".to_string()),
                    };
                    (seed, trace, began.elapsed().as_secs_f64())
                })
            })
            .collect();
        jobs.into_iter()
            .map(|job| job.join().expect("a port run's thread"))
            .collect()
    });
    let costs: Vec<f64> = results
        .iter()
        .filter_map(|(_, trace, _)| trace.as_ref().ok().map(|t| t.end.cost))
        .collect();
    let runs: Vec<serde_json::Value> = results
        .iter()
        .map(|(seed, trace, seconds)| port_run_json(*seed, trace, *seconds))
        .collect();
    Ok(json!({
        "start": { "family": Family::Force, "seed": 24, "set": set },
        "params": params,
        "runs": runs,
        "runs_ended": costs.len(),
        "median_cost": median_cost(&costs),
    }))
}

/// The median of the port's costs: the lower middle of an even count; `None` for none.
fn median_cost(costs: &[f64]) -> Option<f64> {
    let mut sorted = costs.to_vec();
    sorted.sort_by(f64::total_cmp);
    (!sorted.is_empty()).then(|| sorted[(sorted.len() - 1) / 2])
}

/// One run of the port's check: its seed, and its cost and numbers, or the count-only
/// phrase of its failure.
fn port_run_json(seed: u64, trace: &Result<Trace, String>, seconds: f64) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    out.insert("seed".into(), json!(seed));
    match trace {
        Ok(trace) => {
            out.insert("cost".into(), json!(trace.end.cost));
            out.extend(run_fields(trace, Some(seconds)));
        }
        Err(phrase) => {
            out.insert("failed".into(), json!(phrase));
        }
    }
    serde_json::Value::Object(out)
}

/// The silk check's scratch folder below a board's folder (`KONNECT_CONVERGE_SILK=1`):
/// `isolation/`, `input-check/` and one `candidates/c<id>/` per run, each the bench's own
/// layout. Left in place after the run, for inspection; a later run is refused until it
/// is moved or removed.
const SILK_FOLDER: &str = "converge-check-silk";

/// Refuses a silk scratch folder that is there already, whatever it is: an earlier run's
/// folder (its candidate folders would be refused one by one, after the whole search), a
/// file, or a link (it would send the check's writes where it points), a dangling one
/// included.
fn check_silk_folder(path: &Path) -> anyhow::Result<()> {
    ensure!(
        path.symlink_metadata().is_err(),
        "'{}' exists already; move or remove it first (the silk check writes its scratch boards there)",
        path.display()
    );
    Ok(())
}

/// The silk counts of one unrouted check as `converge-check.json` holds them: each kind of
/// `place_judge::SILK` and their `sum`, which is the bench table's `silk` column
/// (`place_search::silk_entries`). No other kind.
fn silk_json(counts: &BTreeMap<String, usize>) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for kind in SILK {
        out.insert(
            (*kind).to_string(),
            json!(counts.get(*kind).copied().unwrap_or(0)),
        );
    }
    out.insert("sum".into(), json!(silk_entries(counts)));
    serde_json::Value::Object(out)
}

/// The input board's silk fields, once per board: `input_silk` (the bench's
/// `input_unrouted`, shaped by [`silk_json`]) and `input_silk_reason`, a fixed phrase
/// when the check failed.
fn input_silk_fields(
    counts: &Result<BTreeMap<String, usize>, &'static str>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    out.insert(
        "input_silk".into(),
        counts.as_ref().map_or(serde_json::Value::Null, silk_json),
    );
    out.insert("input_silk_reason".into(), json!(counts.as_ref().err()));
    out
}

/// The fields design A3 reads for one C run: `silk` ([`silk_json`] of its board's unrouted
/// check) and `silk_reason`; `schematic_neighbours`, the record's measure as `search.json`
/// records it (`Record::file_schematic`), and `schematic_neighbours_reason`. A reason is
/// `null` beside a value and one fixed phrase beside a `null`, never the error's own
/// text, which can name a part or a path. `schematic_readable`: the input's schematic
/// gave neighbourhoods.
fn silk_fields(
    counts: &Result<BTreeMap<String, usize>, &'static str>,
    record: &Record,
    schematic_readable: bool,
) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    out.insert(
        "silk".into(),
        counts.as_ref().map_or(serde_json::Value::Null, silk_json),
    );
    out.insert("silk_reason".into(), json!(counts.as_ref().err()));
    let neighbours = record
        .schematic_neighbours
        .as_ref()
        .map(|measure| serde_json::to_value(measure).expect("a measure is JSON"));
    let neighbours_reason = match (&neighbours, &record.board_text, schematic_readable) {
        (Some(_), _, _) => None,
        (None, None, _) => Some("no board"),
        (None, Some(_), false) => Some("the input's schematic gives no neighbourhoods"),
        (None, Some(_), true) => Some("the board could not be measured"),
    };
    out.insert("schematic_neighbours".into(), json!(neighbours));
    out.insert(
        "schematic_neighbours_reason".into(),
        json!(neighbours_reason),
    );
    out
}

/// What the silk check gives one board: the input's fields and each run's, in run order.
struct SilkCheck {
    input: serde_json::Map<String, serde_json::Value>,
    runs: Vec<serde_json::Map<String, serde_json::Value>>,
    /// Runs whose silk was read.
    measured: usize,
}

/// The phrase beside a `null` silk when the check could not start at all.
const SILK_NOT_STARTED: &str = "the silk check could not start";

/// A message for a board whose silk check read none of its runs, `None` when it read at
/// least one or there was nothing to read. The file keeps its `null`s and fixed phrases;
/// the message is what makes the test fail, so that a check that failed everywhere is
/// never taken for a passing one.
fn silk_unmeasured(check: &SilkCheck) -> Option<String> {
    (check.measured == 0 && !check.runs.is_empty()).then(|| {
        format!(
            "the silk check read 0 of {} run(s); every silk entry of the file is null, with its reason",
            check.runs.len()
        )
    })
}

/// Design A3's two measures of every run, with no routing call, by the bench's own code:
/// the input board's unrouted check as `search` makes it (`PlaceRouteCall::unrouted`,
/// in `<scratch>/input-check`); then for each run its board text (`placements_applied_to`,
/// as the bench routes it) written by `export_candidate` into `<scratch>/candidates/c<id>`,
/// as the bench writes a candidate before its routing call, and `PlaceRouteCall::unrouted`
/// on that copy (`place_route::unrouted_check`; the call first proves the copy on disk is
/// the record's text, and refuses a report without the unconnected-items category, as the
/// bench's own check does). Every check runs one at a time with the bench's scratch
/// `KICAD_CONFIG_HOME` (`inputs.kicad_config_home`) under `isolated_context`, whose
/// `ipc_address` nothing listens on. The schematic measure is `Record::file_schematic`'s.
/// A failed check is a phrase in the file and its full text a line in the log; so is a
/// check that cannot start ([`SILK_NOT_STARTED`]): this runs after the search, so it never
/// returns an error that would lose the search's result.
fn silk_check(
    runtime: &tokio::runtime::Runtime,
    scratch: &Path,
    inputs: &RouteInputs,
    input_text: &str,
    schematic: &Result<SchematicBaseline, String>,
    runs: &mut [Record],
) -> SilkCheck {
    let setup = isolated_context(&scratch.join("isolation"), &inputs.kicad_cli)
        .and_then(|ctx| {
            let name = inputs
                .board
                .file_name()
                .context("the input board has no file name")?
                .to_owned();
            Ok((ctx, name))
        })
        .map_err(|error| {
            println!("the silk check could not start: {error:#}");
            SILK_NOT_STARTED
        });
    let call = |ctx: &Arc<_>, board: PathBuf| PlaceRouteCall {
        inputs: RouteInputs {
            board,
            ..inputs.clone()
        },
        ctx: Arc::clone(ctx),
    };
    let input = match &setup {
        Err(phrase) => Err(*phrase),
        Ok((ctx, _)) => runtime
            .block_on(
                call(ctx, inputs.board.clone())
                    .unrouted(input_text.to_string(), scratch.join("input-check")),
            )
            .map_err(|error| {
                println!("the input board's unrouted check failed: {error:#}");
                "the unrouted check failed"
            }),
    };
    let mut fields = Vec::with_capacity(runs.len());
    let mut measured = 0usize;
    for record in runs.iter_mut() {
        record.file_schematic(schematic);
        let counts = match (&record.board_text, &setup) {
            (None, _) => Err("no board"),
            (Some(_), Err(phrase)) => Err(*phrase),
            (Some(text), Ok((ctx, file_name))) => {
                let folder = scratch
                    .join("candidates")
                    .join(format!("c{:04}", record.id));
                match export_candidate(inputs, &folder, text) {
                    Err(error) => {
                        println!("run {}: the export failed: {error:#}", record.id);
                        Err("the export failed")
                    }
                    Ok(_) => runtime
                        .block_on(call(ctx, folder.join(file_name)).unrouted(text.clone(), folder))
                        .map_err(|error| {
                            println!("run {}: the unrouted check failed: {error:#}", record.id);
                            "the unrouted check failed"
                        }),
                }
            }
        };
        measured += usize::from(counts.is_ok());
        fields.push(silk_fields(&counts, record, schematic.is_ok()));
    }
    SilkCheck {
        input: input_silk_fields(&input),
        runs: fields,
        measured,
    }
}

/// Family C on one board without routing: the model as the search builds it
/// (`search_model`: hold rules, clearances, intent, neighbourhoods), phase 1 (the board's
/// F and S recipes) and phase 1b (`converge_all`) with no routing call, and the port's
/// check when `port` is set. With `silk` (a runtime), [`silk_check`] too: each run's
/// entry gets the fields of [`silk_fields`] and the file gets [`input_silk_fields`]'s;
/// without it the file is what it was before the switch existed. Writes
/// `<board folder>/converge-check.json`, counts and numbers only, and refuses a folder
/// that already holds one (or, with `silk`, a [`SILK_FOLDER`]). Returns the line to print.
fn converge_check_board(
    env: &BenchEnv,
    board: &BenchBoard,
    port: bool,
    silk: Option<&tokio::runtime::Runtime>,
) -> anyhow::Result<String> {
    let folder = env.folder.join(&board.folder);
    let out = folder.join("converge-check.json");
    ensure!(
        !out.exists(),
        "board {} already holds a converge-check.json; move or remove it first",
        board.name
    );
    let scratch = folder.join(SILK_FOLDER);
    if silk.is_some() {
        check_silk_folder(&scratch).with_context(|| format!("board {}", board.name))?;
    }
    let (inputs, ai, held) = board_inputs(env, board, &folder)?;
    let plan = board_plan(board, folder.clone(), inputs, ai, held);
    let (params, rule) = plan
        .matrix
        .converge
        .with_context(|| format!("board {} does not name converge", board.name))?;
    let (model, schematic) = search_model(&plan)?;
    let phase_1 = std::time::Instant::now();
    let records = generate_all(&model, &recipes(&plan.matrix), plan.threads, 0);
    let phase_1_s = phase_1.elapsed().as_secs_f64();
    let starts_available = records
        .iter()
        .filter(|r| {
            matches!(&r.origin, Origin::Generated { recipe, .. }
                if matches!(recipe.family, Family::Force | Family::Schematic))
                && r.outcome == "generated"
                && r.screen_key.is_some_and(|key| key[0] == 0)
        })
        .count();
    let (mut runs, outer) = converge_all(
        &model,
        &records,
        &params,
        &rule,
        plan.threads,
        records.len(),
    );
    let port = if port {
        Some(port_check(&model)?)
    } else {
        None
    };
    let mut run_values: Vec<serde_json::Value> = runs
        .iter()
        .map(|record| converge_run_json(record, &plan.matrix))
        .collect();
    let mut silk_line = String::new();
    let mut silk_failure = None;
    let mut input_fields = None;
    if let Some(runtime) = silk {
        let check = silk_check(
            runtime,
            &scratch,
            &plan.route,
            &model.text,
            &schematic,
            &mut runs,
        );
        silk_failure = silk_unmeasured(&check);
        for (value, fields) in run_values.iter_mut().zip(check.runs) {
            if let serde_json::Value::Object(map) = value {
                map.extend(fields);
            }
        }
        silk_line = format!(
            ", silk: input {}, {} of {} run(s) read",
            check.input["input_silk"]["sum"],
            check.measured,
            runs.len()
        );
        input_fields = Some(check.input);
    }
    let mut file = json!({
        "board": board.name,
        "generator_version": GENERATOR_VERSION,
        "threads": plan.threads,
        "phase_1": {
            "records": records.len(),
            "generated": records.iter().filter(|r| r.outcome == "generated").count(),
            "starts_available": starts_available,
            "seconds": phase_1_s,
        },
        "outer": outer,
        "runs": run_values,
        "port_check": port,
    });
    if let (Some(fields), serde_json::Value::Object(map)) = (input_fields, &mut file) {
        map.extend(fields);
    }
    std::fs::write(&out, serde_json::to_string_pretty(&file)?)?;
    // After the file: the search's result is kept, and the test still fails.
    if let Some(message) = silk_failure {
        bail!("board {}: {message} ({})", board.name, out.display());
    }
    Ok(format!(
        "{}: {} start(s) of {starts_available} (rule for starts: {}), {} effort level(s) (rule for effort: {}), {} run(s), phase 1b {:.1} s{silk_line}, {}",
        board.name,
        outer.starts.len(),
        outer.starts_stop,
        outer.cost_by_level.len(),
        outer.effort_stop.unwrap_or("-"),
        runs.len(),
        outer.seconds,
        out.display()
    ))
}

/// Family C without routing (design measurement step 1) on every board of `bench.json`
/// that names `converge`: the same environment and guards as `place_bench`, then
/// `converge_check_board`. `KONNECT_CONVERGE_PORT_CHECK=1` adds the port's check.
/// Starts no router. Starts no KiCad either, unless `KONNECT_CONVERGE_SILK=1`: then
/// [`silk_check`] runs kicad-cli's refill and DRC on the input board and on each run's
/// board (design A3), one at a time, in the board's [`SILK_FOLDER`].
#[test]
#[ignore = "needs KONNECT_PLACE_BENCH, KONNECT_FASTROUTE, KICAD_CONFIG_HOME and KONNECT_STATE_DIR, as place_bench"]
fn converge_check() {
    let env = bench_environment().unwrap_or_else(|error| panic!("the bench is refused: {error:#}"));
    let port = std::env::var("KONNECT_CONVERGE_PORT_CHECK").is_ok_and(|value| value == "1");
    let silk = std::env::var("KONNECT_CONVERGE_SILK").is_ok_and(|value| value == "1");
    let runtime = silk.then(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime")
    });
    let boards: Vec<&BenchBoard> = env
        .boards
        .iter()
        .filter(|board| board.families.contains(&BenchFamily::Converge))
        .collect();
    assert!(!boards.is_empty(), "no board of bench.json names converge");
    println!(
        "bench folder {}, the port's check {}, the silk check {}",
        env.folder.display(),
        if port { "on" } else { "off" },
        if silk { "on" } else { "off" }
    );
    let mut failed = 0usize;
    for board in boards {
        match converge_check_board(&env, board, port, runtime.as_ref()) {
            Ok(line) => println!("{line}"),
            Err(error) => {
                failed += 1;
                println!("{}: failed: {error:#}", board.name);
            }
        }
    }
    assert_eq!(failed, 0, "{failed} board(s) failed; see the lines above");
}

// ─── The guards, tested on values passed in ──────────────────────────────────

struct Scene {
    /// Stands for the system temp folder.
    temp: tempfile::TempDir,
    /// A folder outside the scratch roots.
    elsewhere: tempfile::TempDir,
    bench: PathBuf,
    home: PathBuf,
    fastroute: PathBuf,
    cli: PathBuf,
}

fn write_board(dir: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, "(kicad_pcb)\n").unwrap();
    path
}

fn scene() -> Scene {
    let temp = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let bench = root.join("bench");
    write_board(&bench.join("B1/input"), "b1.kicad_pcb");
    std::fs::write(
        bench.join("bench.json"),
        r#"{"boards":[{"name":"B1","folder":"B1"}]}"#,
    )
    .unwrap();
    let home = root.join("home");
    std::fs::create_dir_all(home.join("Library/Preferences/kicad")).unwrap();
    std::fs::create_dir_all(home.join(".local/share/konnect")).unwrap();
    std::fs::create_dir_all(home.join("Library/Application Support/konnect")).unwrap();
    let fastroute = root.join("fastroute");
    std::fs::write(&fastroute, "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fastroute, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let cli = root.join("kicad-cli");
    std::fs::write(&cli, "x").unwrap();
    Scene {
        temp,
        elsewhere,
        bench,
        home,
        fastroute,
        cli,
    }
}

impl Scene {
    /// A setup that passes every guard.
    fn raw(&self) -> RawEnv {
        let s = |p: &Path| Some(p.display().to_string());
        RawEnv {
            bench: s(&self.bench),
            fastroute: s(&self.fastroute),
            kicad_config_home: s(&self.bench.join("scratch/kicad-config")),
            state_dir: s(&self.bench.join("scratch/state")),
            kicad_cli: Some(self.cli.clone()),
            home: Some(self.home.clone()),
            temp: self.temp.path().to_path_buf(),
        }
    }
}

fn refusal(raw: &RawEnv) -> String {
    format!(
        "{:#}",
        check_environment(raw).expect_err("the setup must be refused")
    )
}

#[test]
fn bench_environment_refuses_real_settings_and_foreign_inputs() {
    let scene = scene();
    let s = |p: &Path| Some(p.display().to_string());

    // A scratch setup passes, and its boards are read.
    let env = check_environment(&scene.raw()).expect("a scratch setup passes");
    assert_eq!(env.boards.len(), 1);
    assert_eq!(env.boards[0].time_limit_minutes, 100);
    assert_eq!(env.boards[0].effort, "m");
    // A setting below the system temp folder is a scratch folder too.
    let mut raw = scene.raw();
    raw.kicad_config_home = s(&scene.temp.path().join("cfg"));
    assert!(check_environment(&raw).is_ok());

    // The bench folder.
    let mut raw = scene.raw();
    raw.bench = None;
    assert!(refusal(&raw).contains("KONNECT_PLACE_BENCH"));
    raw.bench = Some("relative/bench".into());
    assert!(refusal(&raw).contains("absolute"));
    raw.bench = s(&scene.bench.join("nowhere"));
    assert!(refusal(&raw).contains("not a folder"));

    // KiCad's and Konnect's real folders, itself, below it and above it.
    let real_kicad = scene.home.join("Library/Preferences/kicad");
    let real_konnect = scene.home.join(".local/share/konnect");
    for value in [
        real_kicad.clone(),
        real_kicad.join("10.0"),
        scene.home.join("Library/Preferences"),
    ] {
        let mut raw = scene.raw();
        raw.kicad_config_home = s(&value);
        assert!(refusal(&raw).contains("real folder"), "{}", value.display());
    }
    let real_support = scene.home.join("Library/Application Support/konnect");
    for value in [
        real_konnect.clone(),
        real_konnect.join("state"),
        real_support.clone(),
        real_support.join("state"),
        scene.home.join("Library/Application Support"),
    ] {
        let mut raw = scene.raw();
        raw.state_dir = s(&value);
        assert!(refusal(&raw).contains("real folder"), "{}", value.display());
    }
    // Without the home folder the real ones cannot be excluded: refused, not skipped.
    let mut raw = scene.raw();
    raw.home = None;
    assert!(refusal(&raw).contains("home folder is not known"));
    // Unset, empty, relative, and outside every scratch root.
    let mut raw = scene.raw();
    raw.kicad_config_home = None;
    assert!(refusal(&raw).contains("KICAD_CONFIG_HOME is not set"));
    raw.kicad_config_home = Some(String::new());
    assert!(refusal(&raw).contains("KICAD_CONFIG_HOME is not set"));
    let mut raw = scene.raw();
    raw.state_dir = Some("state".into());
    assert!(refusal(&raw).contains("absolute"));
    raw.state_dir = s(&scene.elsewhere.path().join("state"));
    assert!(refusal(&raw).contains("not below a scratch root"));
    // The system temp folder itself holds the real folders of this scene, so it is refused.
    raw.state_dir = s(scene.temp.path());
    assert!(check_environment(&raw).is_err());

    // The candidate fastroute.
    let mut raw = scene.raw();
    raw.fastroute = None;
    assert!(refusal(&raw).contains("KONNECT_FASTROUTE"));
    raw.fastroute = s(&scene.bench.join("missing"));
    assert!(refusal(&raw).contains("not an executable file"));
    raw.fastroute = s(&scene.bench);
    assert!(refusal(&raw).contains("not an executable file"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let plain = scene.bench.join("plain");
        std::fs::write(&plain, "x").unwrap();
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
        raw.fastroute = s(&plain);
        assert!(refusal(&raw).contains("not an executable file"));
    }

    // No kicad-cli.
    let mut raw = scene.raw();
    raw.kicad_cli = None;
    assert!(refusal(&raw).contains("kicad-cli is missing"));

    // An input board reached through a link that leaves the bench folder.
    #[cfg(unix)]
    {
        let outside = write_board(&scene.elsewhere.path().join("owner"), "owner.kicad_pcb");
        let linked = scene.bench.join("B2/input");
        std::fs::create_dir_all(&linked).unwrap();
        std::os::unix::fs::symlink(&outside, linked.join("b2.kicad_pcb")).unwrap();
        std::fs::write(
            scene.bench.join("bench.json"),
            r#"{"boards":[{"name":"B1","folder":"B1"},{"name":"B2","folder":"B2"}]}"#,
        )
        .unwrap();
        let message = refusal(&scene.raw());
        assert!(message.contains("outside the bench folder"), "{message}");
        // The same through a linked folder, and for the AI's board.
        std::fs::remove_dir_all(scene.bench.join("B2")).unwrap();
        std::os::unix::fs::symlink(scene.elsewhere.path().join("owner"), scene.bench.join("B2"))
            .unwrap();
        std::fs::create_dir_all(scene.elsewhere.path().join("owner/input")).unwrap();
        write_board(&scene.elsewhere.path().join("owner/input"), "b2.kicad_pcb");
        assert!(refusal(&scene.raw()).contains("outside the bench folder"));
        std::fs::remove_file(scene.bench.join("B2")).unwrap();
        std::fs::write(
            scene.bench.join("bench.json"),
            r#"{"boards":[{"name":"B1","folder":"B1"}]}"#,
        )
        .unwrap();
        std::fs::create_dir_all(scene.bench.join("B1/ai")).unwrap();
        std::os::unix::fs::symlink(&outside, scene.bench.join("B1/ai/board.kicad_pcb")).unwrap();
        assert!(refusal(&scene.raw()).contains("outside the bench folder"));
        std::fs::remove_file(scene.bench.join("B1/ai/board.kicad_pcb")).unwrap();
        assert!(check_environment(&scene.raw()).is_ok());
    }

    // The folders the run writes must not be links, even to somewhere inside the bench.
    #[cfg(unix)]
    {
        let inside = scene.bench.join("elsewhere-in-bench");
        std::fs::create_dir_all(&inside).unwrap();
        for name in ["candidates", "isolation"] {
            let link = scene.bench.join("B1").join(name);
            std::os::unix::fs::symlink(&inside, &link).unwrap();
            let message = refusal(&scene.raw());
            assert!(
                message.contains("is a link") && message.contains(name),
                "{message}"
            );
            std::fs::remove_file(&link).unwrap();
            assert!(check_environment(&scene.raw()).is_ok());
        }
        // A link that points nowhere is a link all the same.
        let dangling = scene.bench.join("B1/candidates");
        std::os::unix::fs::symlink(scene.bench.join("nowhere-at-all"), &dangling).unwrap();
        assert!(refusal(&scene.raw()).contains("is a link"));
        std::fs::remove_file(&dangling).unwrap();
        // The board's own folder, linked to another folder of the bench.
        std::fs::create_dir_all(scene.bench.join("B7/input")).unwrap();
        write_board(&scene.bench.join("B7/input"), "b7.kicad_pcb");
        std::os::unix::fs::symlink(scene.bench.join("B7"), scene.bench.join("B8")).unwrap();
        std::fs::write(
            scene.bench.join("bench.json"),
            r#"{"boards":[{"name":"B1","folder":"B1"},{"name":"B8","folder":"B8"}]}"#,
        )
        .unwrap();
        let message = refusal(&scene.raw());
        assert!(
            message.contains("is a link") && message.contains("B8"),
            "{message}"
        );
        std::fs::remove_file(scene.bench.join("B8")).unwrap();
        std::fs::write(
            scene.bench.join("bench.json"),
            r#"{"boards":[{"name":"B1","folder":"B1"}]}"#,
        )
        .unwrap();
        assert!(check_environment(&scene.raw()).is_ok());
    }

    // A folder that climbs out, a missing board, no board file, two boards of one name.
    for bad in [
        r#"{"boards":[{"name":"B1","folder":"../x"}]}"#,
        r#"{"boards":[{"name":"B1","folder":"/etc"}]}"#,
        r#"{"boards":[{"name":"B1","folder":"B1","board":"missing.kicad_pcb"}]}"#,
        r#"{"boards":[{"name":"B1","folder":"B1"},{"name":"B1","folder":"B1"}]}"#,
        r#"{"boards":[]}"#,
        r#"not json"#,
    ] {
        std::fs::write(scene.bench.join("bench.json"), bad).unwrap();
        assert!(check_environment(&scene.raw()).is_err(), "{bad}");
    }
}

/// The bench's matrix of a board whose `bench.json` entry is `entry`.
fn board_of(entry: &str) -> BenchBoard {
    let file: BenchFile =
        serde_json::from_str(&format!(r#"{{"boards":[{entry}]}}"#)).expect("a bench.json");
    file.boards.into_iter().next().unwrap()
}

fn count_of(recipes: &[crate::place_generators::Recipe], family: Family) -> usize {
    recipes.iter().filter(|r| r.family == family).count()
}

#[test]
fn hold_defaults_to_locks_and_rules_keeps_the_old_rules() {
    let plain = board_of(r#"{"name":"B4","folder":"B4"}"#);
    assert_eq!(plain.hold, HoldMode::Locks);
    assert_eq!(plain.hold_rules(), HoldRules::locks_only());
    let locks = board_of(r#"{"name":"B4","folder":"B4","hold":"locks"}"#);
    assert_eq!(locks.hold_rules(), HoldRules::locks_only());
    // Negative control: `rules` is today's behaviour.
    let rules = board_of(r#"{"name":"B4","folder":"B4","hold":"rules"}"#);
    assert_eq!(rules.hold_rules(), HoldRules::default());
    assert_ne!(rules.hold_rules(), locks.hold_rules());
}

#[test]
fn families_default_to_force_and_schematic_and_perturb_brings_p_back() {
    let all = crate::place_generators::recipes(&Matrix::bench());
    let default = board_of(r#"{"name":"B4","folder":"B4"}"#);
    assert_eq!(
        default.families,
        vec![BenchFamily::Force, BenchFamily::Schematic]
    );
    let default_recipes = crate::place_generators::recipes(&default.matrix());
    assert_eq!(count_of(&default_recipes, Family::Perturb), 0);
    // The F and S recipes are the same as in the whole matrix.
    let without_p: Vec<_> = all
        .iter()
        .filter(|r| r.family != Family::Perturb)
        .cloned()
        .collect();
    assert_eq!(default_recipes, without_p);
    assert_eq!(default_recipes.len(), 96 + 64);
    // Naming the two by hand gives the same as leaving the key out.
    let named = board_of(r#"{"name":"B4","folder":"B4","families":["force","schematic"]}"#);
    assert_eq!(
        crate::place_generators::recipes(&named.matrix()),
        default_recipes
    );
    // Negative control: naming `perturb` brings P back, and the whole matrix is 288.
    let with_p =
        board_of(r#"{"name":"B4","folder":"B4","families":["force","schematic","perturb"]}"#);
    let with_p_recipes = crate::place_generators::recipes(&with_p.matrix());
    assert_eq!(with_p_recipes, all);
    assert_eq!(count_of(&with_p_recipes, Family::Perturb), 128);
    // One family alone keeps only that family.
    let only_s = board_of(r#"{"name":"B4","folder":"B4","families":["schematic"]}"#);
    let only_s = crate::place_generators::recipes(&only_s.matrix());
    assert_eq!(count_of(&only_s, Family::Force), 0);
    assert_eq!(count_of(&only_s, Family::Perturb), 0);
    assert_eq!(count_of(&only_s, Family::Schematic), 64);
}

#[test]
fn the_boards_options_reach_its_plan() {
    let route = || RouteInputs {
        rules: Default::default(),
        options: export_options(Path::new("b.kicad_pcb")),
        board: PathBuf::from("b.kicad_pcb"),
        effort: Effort::Fast,
        fastroute: PathBuf::from("fastroute"),
        kicad_cli: "kicad-cli".into(),
        kicad_config_home: PathBuf::from("cfg"),
    };
    let plan_of = |entry: &str| {
        board_plan(
            &board_of(entry),
            PathBuf::from("run"),
            route(),
            None,
            BTreeSet::new(),
        )
    };
    // The defaults: his locks only, F and S.
    let plan = plan_of(r#"{"name":"B4","folder":"B4","time_limit_minutes":7}"#);
    assert_eq!(plan.hold, HoldRules::locks_only());
    assert_eq!(crate::place_generators::recipes(&plan.matrix).len(), 160);
    assert_eq!(plan.time_limit, Duration::from_secs(7 * 60));
    // Negative control: the other options reach the plan too.
    let plan = plan_of(
        r#"{"name":"B4","folder":"B4","hold":"rules","families":["force","schematic","perturb"]}"#,
    );
    assert_eq!(plan.hold, HoldRules::default());
    assert_eq!(crate::place_generators::recipes(&plan.matrix).len(), 288);
}

#[test]
fn an_unknown_hold_or_family_or_an_empty_list_is_refused() {
    let scene = scene();
    for (bad, word) in [
        (
            r#"{"name":"B1","folder":"B1","hold":"everything"}"#,
            "everything",
        ),
        (r#"{"name":"B1","folder":"B1","hold":"Locks"}"#, "Locks"),
        (
            r#"{"name":"B1","folder":"B1","families":["force","chaos"]}"#,
            "chaos",
        ),
        (
            r#"{"name":"B1","folder":"B1","families":["F"]}"#,
            "unknown variant",
        ),
        (
            r#"{"name":"B1","folder":"B1","families":[]}"#,
            "names no family",
        ),
    ] {
        std::fs::write(
            scene.bench.join("bench.json"),
            format!(r#"{{"boards":[{bad}]}}"#),
        )
        .unwrap();
        let message = refusal(&scene.raw());
        assert!(message.contains(word), "{bad}: {message}");
    }
    // Negative control: the same entries with valid values pass.
    std::fs::write(
        scene.bench.join("bench.json"),
        r#"{"boards":[{"name":"B1","folder":"B1","hold":"rules","families":["perturb"]}]}"#,
    )
    .unwrap();
    assert!(check_environment(&scene.raw()).is_ok());
}

#[test]
fn bench_json_defaults_and_overrides() {
    let file: BenchFile = serde_json::from_str(
        r#"{"boards":[{"name":"B4","folder":"B4"},
            {"name":"B5","folder":"b5","board":"x.kicad_pcb","time_limit_minutes":5,"effort":"fast"}]}"#,
    )
    .unwrap();
    assert_eq!(file.boards[0].time_limit_minutes, 100);
    assert_eq!(file.boards[0].effort, "m");
    assert_eq!(file.boards[1].time_limit_minutes, 5);
    assert_eq!(file.boards[1].board.as_deref(), Some("x.kicad_pcb"));
    assert_eq!(Effort::parse(&file.boards[1].effort), Some(Effort::Fast));
}

#[test]
fn converge_needs_f_or_s_and_the_default_stays_f_and_s() {
    // The default: F and S, family C off.
    let default = board_of(r#"{"name":"B4","folder":"B4"}"#);
    assert_eq!(
        default.families,
        vec![BenchFamily::Force, BenchFamily::Schematic]
    );
    assert!(default.check_families().is_ok());
    assert_eq!(default.matrix().converge, None);
    // Named with F and S: C at the measured parameters and rule, and the F and S recipes
    // are the same ones.
    let with_c =
        board_of(r#"{"name":"B4","folder":"B4","families":["force","schematic","converge"]}"#);
    assert!(with_c.check_families().is_ok());
    assert_eq!(
        with_c.matrix().converge,
        Some((ConvergeParams::measured(0), OuterRule::measured()))
    );
    assert_eq!(recipes(&with_c.matrix()), recipes(&default.matrix()));
    // One of F and S is enough.
    for entry in [
        r#"{"name":"B4","folder":"B4","families":["force","converge"]}"#,
        r#"{"name":"B4","folder":"B4","families":["converge","schematic"]}"#,
    ] {
        let board = board_of(entry);
        assert!(board.check_families().is_ok(), "{entry}");
        assert!(board.matrix().converge.is_some(), "{entry}");
    }
    // Negative control: C alone, or with P only, is refused with its word, before
    // anything runs.
    for entry in [
        r#"{"name":"B1","folder":"B1","families":["converge"]}"#,
        r#"{"name":"B1","folder":"B1","families":["perturb","converge"]}"#,
    ] {
        let message = format!("{:#}", board_of(entry).check_families().unwrap_err());
        assert!(
            message.contains("converge") && message.contains("force"),
            "{entry}: {message}"
        );
        let scene = scene();
        std::fs::write(
            scene.bench.join("bench.json"),
            format!(r#"{{"boards":[{entry}]}}"#),
        )
        .unwrap();
        assert!(refusal(&scene.raw()).contains("converge"), "{entry}");
    }
    // The option reaches the plan.
    let plan = board_plan(
        &with_c,
        PathBuf::from("run"),
        RouteInputs {
            rules: Default::default(),
            options: export_options(Path::new("b.kicad_pcb")),
            board: PathBuf::from("b.kicad_pcb"),
            effort: Effort::Fast,
            fastroute: PathBuf::from("fastroute"),
            kicad_cli: "kicad-cli".into(),
            kicad_config_home: PathBuf::from("cfg"),
        },
        None,
        BTreeSet::new(),
    );
    assert!(plan.matrix.converge.is_some());
}

#[test]
fn converge_terms_off_turns_both_new_terms_off_and_nothing_else() {
    let on = board_of(r#"{"name":"B4","folder":"B4","families":["force","converge"]}"#);
    assert_eq!(on.converge_terms, ConvergeTerms::On);
    let named_on = board_of(
        r#"{"name":"B4","folder":"B4","families":["force","converge"],"converge_terms":"on"}"#,
    );
    assert_eq!(named_on.matrix(), on.matrix());
    let off = board_of(
        r#"{"name":"B4","folder":"B4","families":["force","converge"],"converge_terms":"off"}"#,
    );
    let (params, rule) = off.matrix().converge.expect("family C");
    assert_eq!(
        params,
        ConvergeParams {
            neighbour_weight: 0.0,
            label_mm: 0.0,
            ..ConvergeParams::measured(0)
        }
    );
    assert_eq!(rule, OuterRule::measured());
    // Negative control: on, the two terms keep their measured weights.
    let (on_params, _) = on.matrix().converge.unwrap();
    assert!(on_params.neighbour_weight > 0.0 && on_params.label_mm > 0.0);
    // An unknown value is refused before anything runs.
    let scene = scene();
    std::fs::write(
        scene.bench.join("bench.json"),
        r#"{"boards":[{"name":"B1","folder":"B1","families":["force","converge"],"converge_terms":"half"}]}"#,
    )
    .unwrap();
    assert!(refusal(&scene.raw()).contains("half"));
}

// ─── converge-check.json: counts and numbers only ────────────────────────────

/// What a failure's text can hold: a reference, a net name and a path.
const PRIVATE_LOOKING: [&str; 3] = ["U17", "VSECRET", "/Users/someone"];

/// Every string value anywhere in `value`.
fn strings_in(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::String(text) => vec![text.clone()],
        serde_json::Value::Array(items) => items.iter().flat_map(strings_in).collect(),
        serde_json::Value::Object(map) => map.values().flat_map(strings_in).collect(),
        _ => Vec::new(),
    }
}

/// `value` written out holds none of `PRIVATE_LOOKING`.
fn assert_names_nothing(value: &serde_json::Value, what: &str) {
    let text = value.to_string();
    for word in PRIVATE_LOOKING {
        assert!(
            !text.contains(word),
            "{what}: `{word}` reached the file: {text}"
        );
    }
}

fn check_terms(cost: f64) -> Terms {
    Terms {
        cost,
        airwire_mm: cost / 2.0,
        rail_airwire_mm: 1.0,
        crossings: 3,
        pair_violations: 0,
        pair_mm: 4.0,
        connector_violations: 0,
        neighbour_mm: Some(7.5),
        label_conflicts: 2,
        soft: cost,
    }
}

fn check_trace(cost: f64) -> Trace {
    Trace {
        level: 1,
        requirement_mm: 1000.0,
        hoods_sha256: Some("ab".repeat(32)),
        start: check_terms(2.0 * cost),
        end: check_terms(cost),
        repairs_tried: 2,
        repairs_made: 1,
        first_temperature: 12.5,
        stages: 40,
        anneal_stop: crate::place_converge::AnnealStop::Quiet,
        sweeps: 3,
        descent_stop: crate::place_converge::DescentStop::FixedPoint,
        evaluations: 400,
        best_by_step: vec![2.0 * cost, cost],
    }
}

/// A record of family C from `start` at level 1, as `converge_all` files one.
fn check_record(
    id: usize,
    start: Recipe,
    outcome: &'static str,
    reason: Option<&str>,
    trace: Option<Trace>,
) -> Record {
    Record {
        id,
        origin: Origin::Generated {
            recipe: Recipe {
                family: Family::Converge,
                seed: start.seed,
                params: Params::Converge(crate::place_generators::ConvergeRecipe {
                    start: Box::new(start),
                    params: ConvergeParams::measured(1),
                }),
            },
            generator_version: GENERATOR_VERSION,
        },
        board_sha256: None,
        input_sha256: "i".into(),
        held_sha256: "h".into(),
        intent_sha256: "t".into(),
        outcome,
        reason: reason.map(str::to_string),
        converged: trace.as_ref().map(Trace::converged),
        screen: None,
        screen_key: None,
        key: None,
        rank: None,
        timings: crate::place_search::Timings {
            generate_ms: Some(1500.0),
            ..Default::default()
        },
        route: None,
        placement_gates: None,
        unrouted: None,
        route_gates: None,
        schematic_neighbours: None,
        schematic_neighbours_reason: reason.map(str::to_string),
        converge: trace,
        board_text: None,
        judged: None,
    }
}

/// The bench matrix's recipe of `family`, seed 24, parameter set `set`.
fn bench_start(family: Family, set: usize) -> Recipe {
    let matrix = Matrix::bench();
    recipes(&matrix)
        .into_iter()
        .find(|r| {
            r.family == family
                && r.seed == 24
                && match &r.params {
                    Params::Force(p) => *p == matrix.force[set],
                    Params::Schematic(p) => *p == matrix.schematic[set],
                    _ => false,
                }
        })
        .expect("a bench recipe")
}

#[test]
fn a_failure_phrase_is_a_count_or_a_word() {
    let cases = [
        (
            GenerateFailure::NoRoom(vec!["U17".to_string(), "VSECRET".to_string()]),
            "no room for 2 part(s)",
        ),
        (
            GenerateFailure::Invalid("pad 1 of U17 on VSECRET in /Users/someone".to_string()),
            "invalid",
        ),
        (
            GenerateFailure::Panicked("U17 panicked in /Users/someone".to_string()),
            "panicked",
        ),
    ];
    for (failure, phrase) in cases {
        assert_eq!(failure_phrase(&failure), phrase, "{failure:?}");
    }
}

#[test]
fn run_fields_are_numbers_and_fixed_words_only() {
    let trace = check_trace(120.0);
    let fields = run_fields(&trace, Some(2.0));
    let keys: Vec<&str> = fields.keys().map(String::as_str).collect();
    let mut expected = vec![
        "converged",
        "anneal_stop",
        "descent_stop",
        "stages",
        "sweeps",
        "evaluations",
        "repairs_tried",
        "repairs_made",
        "first_temperature",
        "requirement_mm",
        "start",
        "end",
        "seconds",
        "microseconds_per_evaluation",
    ];
    expected.sort_unstable();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted, expected,
        "no other field: no hash, no curve, no reason"
    );
    let value = serde_json::Value::Object(fields.clone());
    let words = strings_in(&value);
    assert_eq!(words, vec!["quiet".to_string(), "fixed_point".to_string()]);
    assert_eq!(fields["converged"], true);
    assert_eq!(fields["stages"], 40);
    assert_eq!(fields["end"]["cost"], 120.0);
    assert_eq!(fields["end"]["neighbour_mm"], 7.5);
    assert_eq!(fields["microseconds_per_evaluation"], 2.0 * 1e6 / 400.0);
    // No wall time, or no evaluation: no time per evaluation.
    assert!(run_fields(&trace, None)["microseconds_per_evaluation"].is_null());
    let idle = Trace {
        evaluations: 0,
        ..trace
    };
    assert!(run_fields(&idle, Some(2.0))["microseconds_per_evaluation"].is_null());
}

#[test]
fn a_converge_check_record_names_no_part_net_or_path() {
    let matrix = Matrix::bench();
    let done = check_record(
        200,
        bench_start(Family::Force, 1),
        "generated",
        None,
        Some(check_trace(90.0)),
    );
    let failed = check_record(
        201,
        bench_start(Family::Schematic, 1),
        "generate_failed",
        Some("no room for U17 on VSECRET in /Users/someone"),
        None,
    );
    let done_json = converge_run_json(&done, &matrix);
    let failed_json = converge_run_json(&failed, &matrix);
    for (json, what) in [(&done_json, "a run"), (&failed_json, "a failed run")] {
        assert_names_nothing(json, what);
        assert!(json.get("reason").is_none(), "{what}: {json}");
    }
    // The recipe: the start's family, seed and parameter-set index, and the level.
    assert_eq!(
        done_json["recipe"],
        json!({ "start": { "family": "force", "seed": 24, "set": 1 }, "level": 1 })
    );
    assert_eq!(failed_json["recipe"]["start"]["family"], "schematic");
    assert_eq!(failed_json["recipe"]["start"]["set"], 1);
    assert_eq!(done_json["id"], 200);
    assert_eq!(done_json["outcome"], "generated");
    assert_eq!(failed_json["outcome"], "generate_failed");
    assert_eq!(done_json["cost"], 90.0);
    assert_eq!(done_json["seconds"], 1.5);
    let mut keys: Vec<&String> = failed_json.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        ["id", "outcome", "recipe"],
        "a failed run: no numbers, no reason"
    );
    // A start whose set is not one of the matrix's has no index.
    let small = Matrix {
        force: Vec::new(),
        ..Matrix::bench()
    };
    assert!(converge_run_json(&done, &small)["recipe"]["start"]["set"].is_null());

    // The silk check's keys (design A3) hold counts, numbers and fixed phrases only, the
    // reason a record carries never: it can name a part, a net or a path.
    let mut measured = done.clone();
    measured.board_text = Some("(kicad_pcb)".into());
    measured.schematic_neighbours = Some(check_measure());
    let mut unmeasured = done.clone();
    unmeasured.board_text = Some("(kicad_pcb)".into());
    unmeasured.schematic_neighbours_reason =
        Some("the schematic /Users/someone/U17.kicad_sch has no VSECRET".into());
    let counts = check_counts();
    let cases = [
        (
            silk_fields(&Ok(counts.clone()), &measured, true),
            "a measured run",
        ),
        (
            silk_fields(&Err("the unrouted check failed"), &unmeasured, true),
            "a run whose measures failed",
        ),
        (
            silk_fields(&Err("no board"), &failed, false),
            "a failed run without a schematic",
        ),
        (input_silk_fields(&Ok(counts)), "the input"),
        (
            input_silk_fields(&Err("the unrouted check failed")),
            "the input, failed",
        ),
    ];
    for (fields, what) in &cases {
        assert_names_nothing(&serde_json::Value::Object(fields.clone()), what);
    }
    let mut words: Vec<String> = cases
        .iter()
        .flat_map(|(fields, _)| strings_in(&serde_json::Value::Object(fields.clone())))
        .collect();
    words.sort();
    assert_eq!(
        words,
        [
            "no board",
            "no board",
            "the board could not be measured",
            "the unrouted check failed",
            "the unrouted check failed",
        ],
        "the only words are the fixed phrases"
    );
}

/// A measure as `SchematicBaseline::measure` gives one.
fn check_measure() -> crate::place_schematic::SchematicMeasure {
    crate::place_schematic::SchematicMeasure {
        mean_mm: 12.5,
        ratio_to_input: Some(1.25),
        parts_pulled_away: 4,
        parts_measured: 30,
        parts_unmatched: 2,
        parts_alone: 1,
    }
}

/// An unrouted check's counts: the three silk kinds and one kind that is not silk.
fn check_counts() -> BTreeMap<String, usize> {
    [
        ("silk_overlap", 7usize),
        ("silk_over_copper", 11),
        ("silk_edge_clearance", 2),
        ("clearance", 40),
    ]
    .into_iter()
    .map(|(kind, n)| (kind.to_string(), n))
    .collect()
}

#[test]
fn the_silk_fields_are_the_benchs_silk_kinds_and_sum_and_its_schematic_measure() {
    let counts = check_counts();
    let silk = json!({
        "silk_overlap": 7,
        "silk_over_copper": 11,
        "silk_edge_clearance": 2,
        "sum": 20,
    });
    // The sum is the bench's own count (the table's `silk` column), which leaves the
    // other kinds out.
    assert_eq!(silk_entries(&counts), 20);
    assert_eq!(silk_json(&counts), silk);
    // A kind the check did not report counts 0.
    assert_eq!(
        silk_json(&BTreeMap::new()),
        json!({ "silk_overlap": 0, "silk_over_copper": 0, "silk_edge_clearance": 0, "sum": 0 })
    );

    let mut record = check_record(
        210,
        bench_start(Family::Force, 0),
        "generated",
        None,
        Some(check_trace(90.0)),
    );
    record.board_text = Some("(kicad_pcb)".into());
    record.schematic_neighbours = Some(check_measure());
    let fields = serde_json::Value::Object(silk_fields(&Ok(counts.clone()), &record, true));
    assert_eq!(
        fields,
        json!({
            "silk": silk,
            "silk_reason": null,
            "schematic_neighbours": {
                "mean_mm": 12.5,
                "ratio_to_input": 1.25,
                "parts_pulled_away": 4,
                "parts_measured": 30,
                "parts_unmatched": 2,
                "parts_alone": 1,
            },
            "schematic_neighbours_reason": null,
        })
    );
    // Each `null` has its phrase beside it.
    record.schematic_neighbours = None;
    let no_schematic = silk_fields(&Err("the export failed"), &record, false);
    assert!(no_schematic["silk"].is_null());
    assert_eq!(no_schematic["silk_reason"], "the export failed");
    assert!(no_schematic["schematic_neighbours"].is_null());
    assert_eq!(
        no_schematic["schematic_neighbours_reason"],
        "the input's schematic gives no neighbourhoods"
    );
    assert_eq!(
        silk_fields(&Ok(counts.clone()), &record, true)["schematic_neighbours_reason"],
        "the board could not be measured"
    );
    record.board_text = None;
    assert_eq!(
        silk_fields(&Err("no board"), &record, true)["schematic_neighbours_reason"],
        "no board"
    );
    // The input's fields, once per board.
    assert_eq!(
        serde_json::Value::Object(input_silk_fields(&Ok(counts))),
        json!({ "input_silk": silk, "input_silk_reason": null })
    );
    assert_eq!(
        serde_json::Value::Object(input_silk_fields(&Err("the unrouted check failed"))),
        json!({ "input_silk": null, "input_silk_reason": "the unrouted check failed" })
    );
}

#[test]
fn the_silk_scratch_folder_is_refused_when_anything_is_there() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SILK_FOLDER);
    check_silk_folder(&path).expect("a folder that is not there is taken");
    std::fs::create_dir(&path).unwrap();
    assert!(check_silk_folder(&path).is_err(), "an earlier run's folder");
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, "x").unwrap();
    assert!(check_silk_folder(&path).is_err(), "a file");
    std::fs::remove_file(&path).unwrap();
    #[cfg(unix)]
    {
        // A dangling link: nothing is there to follow, and the check's writes would still
        // land where it points.
        std::os::unix::fs::symlink(dir.path().join("elsewhere"), &path).unwrap();
        let refusal = check_silk_folder(&path).expect_err("a dangling link");
        assert!(refusal.to_string().contains("exists already"), "{refusal}");
    }
}

#[test]
fn the_ports_median_is_the_lower_middle_and_a_run_holds_its_numbers_or_its_phrase() {
    assert_eq!(median_cost(&[]), None);
    assert_eq!(median_cost(&[5.0]), Some(5.0));
    assert_eq!(median_cost(&[3.0, 1.0, 2.0]), Some(2.0));
    assert_eq!(
        median_cost(&[4.0, 1.0, 3.0, 2.0]),
        Some(2.0),
        "the lower middle"
    );
    assert_eq!(
        median_cost(&[1195.0, 1121.0, 1300.0, 1163.0, 1250.0]),
        Some(1195.0)
    );
    let ok = port_run_json(3, &Ok(check_trace(80.0)), 4.0);
    assert_eq!(ok["seed"], 3);
    assert_eq!(ok["cost"], 80.0);
    assert_eq!(ok["seconds"], 4.0);
    assert!(ok.get("failed").is_none());
    let failure = GenerateFailure::NoRoom(vec!["U17".to_string()]);
    let failed = port_run_json(4, &Err(failure_phrase(&failure)), 4.0);
    assert_eq!(
        failed,
        json!({ "seed": 4, "failed": "no room for 1 part(s)" })
    );
    for (json, what) in [(&ok, "a run"), (&failed, "a failed run")] {
        assert_names_nothing(json, what);
    }
}

/// The silk check's inputs for a board that is not there: nothing in them is read before
/// the check's own start fails.
fn unstarted_inputs(root: &Path) -> RouteInputs {
    RouteInputs {
        rules: Default::default(),
        options: export_options(Path::new("b.kicad_pcb")),
        board: root.join("input/b.kicad_pcb"),
        effort: Effort::Fast,
        fastroute: root.join("fastroute"),
        kicad_cli: "kicad-cli".into(),
        kicad_config_home: root.join("cfg"),
    }
}

#[test]
fn a_silk_check_that_cannot_start_is_a_phrase_for_every_run_and_loses_nothing() {
    // `isolated_context` refuses a folder that is not empty: the silk check, which runs
    // after the whole search, answers with `null`s and one fixed phrase instead of an
    // error that would lose the search's result. No kicad-cli is started.
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let scratch = root.join(SILK_FOLDER);
    std::fs::create_dir_all(scratch.join("isolation")).unwrap();
    std::fs::write(scratch.join("isolation/occupied"), "x").unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut with_board = check_record(
        1,
        bench_start(Family::Force, 0),
        "generated",
        None,
        Some(check_trace(90.0)),
    );
    with_board.board_text = Some("(kicad_pcb)".into());
    let without_board = check_record(
        2,
        bench_start(Family::Force, 0),
        "generated",
        None,
        Some(check_trace(80.0)),
    );
    let mut runs = [with_board, without_board];
    let check = silk_check(
        &runtime,
        &scratch,
        &unstarted_inputs(&root),
        "(kicad_pcb)",
        &Err("no schematic".into()),
        &mut runs,
    );
    assert_eq!(check.measured, 0);
    assert_eq!(check.runs.len(), 2);
    assert!(check.runs[0]["silk"].is_null());
    assert_eq!(check.runs[0]["silk_reason"], SILK_NOT_STARTED);
    assert!(check.runs[1]["silk"].is_null());
    assert_eq!(check.runs[1]["silk_reason"], "no board");
    assert_eq!(
        serde_json::Value::Object(check.input.clone()),
        json!({ "input_silk": null, "input_silk_reason": SILK_NOT_STARTED })
    );
    // The scratch folder was not written into: nothing started.
    assert!(!scratch.join("candidates").exists());
    assert!(!scratch.join("input-check").exists());
    // Nothing read means the test fails, with the file kept.
    let message = silk_unmeasured(&check).expect("0 of 2 is a failure");
    assert!(message.contains("0 of 2"), "{message}");
}

#[test]
fn only_a_check_that_read_no_run_at_all_fails_the_test() {
    let check = |measured: usize, runs: usize| SilkCheck {
        input: serde_json::Map::new(),
        runs: vec![serde_json::Map::new(); runs],
        measured,
    };
    assert!(silk_unmeasured(&check(0, 3)).is_some());
    assert!(silk_unmeasured(&check(1, 3)).is_none());
    assert!(silk_unmeasured(&check(3, 3)).is_none());
    // A board with no run has nothing to read; that is not this failure.
    assert!(silk_unmeasured(&check(0, 0)).is_none());
}
