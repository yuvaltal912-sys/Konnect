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
//! `ai/board.kicad_pcb`), its time limit, the effort, `hold` (`locks` or `rules`) and
//! `families`. The bench writes only below
//! the bench folder. The format is in `xtask/placement-bench/README.md`.

use crate::place_generators::{Family, Matrix};
use crate::place_held::HoldRules;
use crate::place_route::{export_options, isolated_context, rules_from_json, RouteInputs};
use crate::place_search::{search, PlaceRouteCall, Plan};
use crate::route_search::Effort;
use anyhow::{bail, ensure, Context};
use serde::Deserialize;
use std::collections::BTreeSet;
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
    /// The families of the matrix: any of `force`, `schematic` and `perturb` (default
    /// `force` and `schematic`). O and A are references whatever is named.
    #[serde(default = "default_families")]
    pub families: Vec<BenchFamily>,
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

    /// `Matrix::bench` with only the board's `families`.
    pub(crate) fn matrix(&self) -> Matrix {
        let families: Vec<Family> = self
            .families
            .iter()
            .map(|family| match family {
                BenchFamily::Force => Family::Force,
                BenchFamily::Schematic => Family::Schematic,
                BenchFamily::Perturb => Family::Perturb,
            })
            .collect();
        Matrix::bench().only(&families)
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
    ensure!(
        !board.families.is_empty(),
        "board {}: families names no family (use force, schematic, perturb)",
        board.name
    );
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
    let ctx = isolated_context(&folder.join("isolation"), &env.kicad_cli)?;
    let plan = Plan {
        name: board.name.clone(),
        folder: folder.clone(),
        matrix: board.matrix(),
        threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
        reference_calls: 3,
        best_per_family: 3,
        drawn_per_family: 1,
        draw_seed: 1,
        time_limit: Duration::from_secs(board.time_limit_minutes * 60),
        route: inputs.clone(),
        ai_board: ai.is_file().then_some(ai),
        caller_held: read_held(&folder.join("held.json"))?,
        hold: board.hold_rules(),
    };
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
