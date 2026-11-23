//! The judge of a best-of-N search: Konnect judges every routed run itself, on
//! scratch copies of the board, and ranks the runs by gates that need no human
//! routing (docs/ROUTE_SEARCH_DESIGN.md, decisions 10 to 12; the research's
//! "Konnect changes", item 7).
//!
//! Nothing here writes to the user's board or project: every board it writes is a
//! copy under the search folder.

use crate::board_identity;
use crate::route_search::Routed;
use crate::specctra_ses::SourceCheck;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The largest share of an intent-width net's length that may be below its intent
/// width (the bench's `below_intent_share`).
pub(crate) const BELOW_INTENT_SHARE: f64 = 0.05;
/// The longest stretch of an intent-width net that may be below its intent width,
/// in mm (the bench's `below_intent_section_mm`).
pub(crate) const BELOW_INTENT_SECTION_MM: f64 = 2.0;
/// Stitching rounds after the first refill (the research's `MAX_ROUNDS`).
pub(crate) const STITCH_ROUNDS: usize = 4;
/// The stitching planner's search radius, mm (the research's `STITCH_RADIUS`).
pub(crate) const STITCH_RADIUS_MM: f64 = 3.0;
/// The step in which the rank compares length ratios (the research key's 0.02).
pub(crate) const LENGTH_STEP: f64 = 0.02;
/// A segment this far below its net's floor, or less, is at the floor: the same
/// half micrometre the importer allows.
pub(crate) const FLOOR_TOLERANCE_MM: f64 = 0.0005;

/// Why the judge cannot run, with the step that fixes it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Refusal {
    pub reason: String,
    pub next_step: String,
}

/// What every judge of one call needs, read once.
#[derive(Debug, Clone)]
pub(crate) struct JudgeContext {
    /// The manifest's `board_path`, canonical: the board the DSN was exported from.
    pub board: PathBuf,
    /// The saved board's text: the board the DSN was exported from (see
    /// [`saved_board_matches`]).
    pub board_text: String,
    pub manifest: Value,
    pub manifest_text: String,
    pub kicad_cli: String,
    /// Nets of the manifest's `pours` rows.
    pub pour_nets: BTreeSet<String>,
    /// Nets of the manifest's `intent_widths` rows whose status is `widened` or
    /// `already_wide_enough` and that are not pour nets.
    pub power_nets: BTreeSet<String>,
    /// Each net's floor in mm: its `min_track_width_um` when set, else its class's
    /// `track_width_um`.
    pub floors_mm: BTreeMap<String, f64>,
    /// `<search folder>/judge`: every scratch board of this call is below it.
    pub root: PathBuf,
    /// The private copy of KiCad's settings (`private_kicad_config`) that every
    /// kicad-cli the judge starts runs with as `KICAD_CONFIG_HOME`, so that none of
    /// them writes the owner's real settings. `prepare` sets `None`; the caller sets
    /// it once the search folder exists.
    pub kicad_config_home: Option<PathBuf>,
    /// Row LT: what the call's length plan checks (`route_lengths::LengthPlan::
    /// checks`); `None` or empty: no probe, no G4(a) or G4(b). `prepare` sets `None`;
    /// the caller sets it after the plan is built.
    pub lengths: Option<Arc<crate::length_probe::LengthChecks>>,
    /// Row ZR, ZR1: the saved board's pads named as the DSN names them
    /// (`zone_fill::ZonePlan::names`), for KiCad's side of the pad comparison. `prepare`
    /// sets `None`; the caller sets it when the call sends a zones file.
    pub pad_names: Option<Arc<crate::zone_fill::PadNames>>,
}

/// The unrouted board, refilled and checked the same way every run is (G2's
/// baseline).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Baseline {
    /// KiCad's violations by type (rule key).
    pub violations: BTreeMap<String, usize>,
    pub unconnected: usize,
    /// Row ZR, ZR5 part 1: the pour nets' layer fills of the refilled baseline board
    /// (`pour_damage::layer_fills`), when the call has pour nets and they could be read:
    /// the "before" of every run's `pours.damage`. Recorded only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pour_fills: Option<Vec<crate::pour_damage::LayerFill>>,
    /// Row ZR, ZR5 part 1: why `pour_fills` could not be read. Never fails the baseline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pour_fills_error: Option<String>,
}

/// Whether the saved board is the board the DSN was exported from.
///
/// The export hashes KiCad's in-memory text of the board (IPC), and KiCad saves
/// that board in a different layout and without each footprint's library
/// metadata, so the saved bytes almost never hash to `source_sha256`. The rule is
/// [`board_identity::matches_export`], which the judge's import-plan call
/// ([`SourceCheck::BytesOrTree`]) uses too, so the preflight and the run cannot
/// disagree: the bytes match `source_sha256`, or the manifest has
/// `source_canonical_sha256` and the saved text is the same board as a tree.
///
/// A manifest without `source_canonical_sha256` (an export before it was written)
/// gets the byte check only. A saved file that does not parse matches only by
/// bytes.
pub(crate) fn saved_board_matches(saved: &[u8], manifest: &Value) -> bool {
    board_identity::matches_export(
        saved,
        manifest["source_sha256"].as_str(),
        manifest["source_canonical_sha256"].as_str(),
    )
}

/// Everything the judge needs, checked before anything is routed. Each refusal
/// says what to do:
/// - no manifest beside the DSN (`<dsn>.konnect.json`): "export the DSN with
///   export_specctra_dsn, which writes the manifest beside it";
/// - the manifest names no board, or the board file is missing;
/// - the saved board is not the exported board ([`saved_board_matches`]): "save
///   the board in KiCad, then export the DSN again";
/// - `kicad_cli` is not an executable file.
///
/// `root` is created by the caller later; this only records it.
pub(crate) fn prepare(
    dsn: &Path,
    root: &Path,
    kicad_cli: &str,
) -> std::result::Result<JudgeContext, Refusal> {
    fn refuse(reason: String, next_step: &str) -> Refusal {
        Refusal {
            reason,
            next_step: next_step.to_string(),
        }
    }
    const EXPORT_AGAIN: &str =
        "export the DSN with export_specctra_dsn, which writes the manifest beside it";

    // `<dsn file name>.konnect.json`, as `specctra::manifest_beside` names it.
    let mut manifest_name = dsn.as_os_str().to_os_string();
    manifest_name.push(".konnect.json");
    let manifest_path = PathBuf::from(manifest_name);
    let manifest_text = std::fs::read_to_string(&manifest_path).map_err(|error| {
        refuse(
            format!(
                "no reverse manifest beside the DSN ('{}': {error})",
                manifest_path.display()
            ),
            EXPORT_AGAIN,
        )
    })?;
    let manifest: Value = serde_json::from_str(&manifest_text).map_err(|error| {
        refuse(
            format!(
                "the manifest '{}' is not JSON ({error})",
                manifest_path.display()
            ),
            EXPORT_AGAIN,
        )
    })?;

    let board_name = manifest["board_path"]
        .as_str()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| refuse("the manifest names no board".to_string(), EXPORT_AGAIN))?;
    let board = std::fs::canonicalize(board_name).map_err(|error| {
        refuse(
            format!("the manifest's board '{board_name}' is missing ({error})"),
            "restore the board file, or export the DSN again from the board's current location",
        )
    })?;
    let saved = std::fs::read(&board).map_err(|error| {
        refuse(
            format!("the board '{}' cannot be read ({error})", board.display()),
            "check the board file, then export the DSN again",
        )
    })?;
    if !saved_board_matches(&saved, &manifest) {
        return Err(refuse(
            format!(
                "the saved board '{}' is not the board the DSN was exported from (neither its SHA-256 nor its canonical SHA-256 is the manifest's)",
                board.display()
            ),
            "save the board in KiCad, then export the DSN again",
        ));
    }
    let board_text = String::from_utf8(saved).map_err(|_| {
        refuse(
            format!("the board '{}' is not UTF-8 text", board.display()),
            "check the board file, then export the DSN again",
        )
    })?;

    // Resolved as every other kicad-cli tool resolves it (`resolve_cli_executable`:
    // a bare name is looked for on PATH, then in KiCad's install locations), so the
    // judge runs the same kicad-cli `run_drc` would.
    let Some(kicad_cli) = resolve_executable(kicad_cli) else {
        return Err(refuse(
            format!("kicad-cli '{kicad_cli}' is not an executable file"),
            "install KiCad 10, or set the kicad-cli path in the Konnect config",
        ));
    };
    let kicad_cli = kicad_cli.display().to_string();

    let names = |rows: &Value, wanted: &dyn Fn(&Value) -> bool| -> BTreeSet<String> {
        rows.as_array()
            .into_iter()
            .flatten()
            .filter(|row| wanted(row))
            .filter_map(|row| row["net"].as_str().map(str::to_string))
            .collect()
    };
    let pour_nets = names(&manifest["pours"], &|_| true);
    let power_nets: BTreeSet<String> = names(&manifest["intent_widths"], &|row| {
        matches!(
            row["status"].as_str(),
            Some("widened") | Some("already_wide_enough")
        )
    })
    .into_iter()
    .filter(|net| !pour_nets.contains(net))
    .collect();

    // `specctra::manifest_net_floors`' rule, kept per net.
    let class_width_um: BTreeMap<&str, i64> = manifest["classes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|class| Some((class["name"].as_str()?, class["track_width_um"].as_i64()?)))
        .collect();
    let floors_mm: BTreeMap<String, f64> = manifest["nets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|net| {
            let floor_um = net["min_track_width_um"]
                .as_i64()
                .or_else(|| class_width_um.get(net["class_name"].as_str()?).copied())?;
            Some((net["name"].as_str()?.to_string(), floor_um as f64 / 1000.0))
        })
        .collect();

    Ok(JudgeContext {
        board,
        board_text,
        manifest,
        manifest_text,
        kicad_cli,
        pour_nets,
        power_nets,
        floors_mm,
        root: root.to_path_buf(),
        kicad_config_home: None,
        lengths: None,
        pad_names: None,
    })
}

/// An executable regular file: any execute bit on Unix. A name with no path
/// separator is looked up on `PATH`.
fn is_executable_file(path: &Path) -> bool {
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

fn resolve_executable(cli: &str) -> Option<PathBuf> {
    if cli.trim().is_empty() {
        return None;
    }
    let resolved = crate::tools::cli::resolve_cli_executable(cli);
    is_executable_file(&resolved).then_some(resolved)
}

/// KiCad's violations counted by rule key.
pub(crate) fn violations_by_rule(drc: &crate::tools::cli::DrcReport) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for violation in &drc.violations {
        *counts.entry(violation.rule.clone()).or_insert(0) += 1;
    }
    counts
}

/// The unconnected items of a report; a report that has no such category is an
/// error, never zero. Shared with `place_route::unrouted_counts`, so that the placement
/// bench's unrouted check refuses what the judge's baseline refuses.
pub(crate) fn unconnected_total(drc: &crate::tools::cli::DrcReport) -> Result<usize> {
    drc.unconnected_items.as_ref().map(Vec::len).context(
        "the DRC report has no unconnected-items category; an unrouted board cannot be told from a routed one",
    )
}

/// A message cut to a readable length.
fn clipped(text: &str) -> String {
    const SHOWN: usize = 300;
    match text.char_indices().nth(SHOWN) {
        Some((cut, _)) => format!("{}...", &text[..cut]),
        None => text.to_string(),
    }
}

/// Removes the whole folder of an earlier stitching round (`<root>/run-<i>/r<k>`,
/// made by this judge): the board named by `board`, its DRC report and the project
/// files copied beside them. A folder that is already gone is fine. A link is never
/// followed or removed, and nothing outside `root` or not named `r<number>` is.
fn remove_round_folder(root: &Path, board: &Path) -> Result<()> {
    let folder = board
        .parent()
        .with_context(|| format!("'{}' has no folder", board.display()))?;
    ensure!(
        folder != root && folder.starts_with(root),
        "'{}' is not a folder below the judge's root '{}'",
        folder.display(),
        root.display()
    );
    let named_as_a_round = folder
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix('r'))
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()));
    ensure!(
        named_as_a_round,
        "'{}' is not a round folder (r<number>)",
        folder.display()
    );
    let meta = match std::fs::symlink_metadata(folder) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("inspect '{}'", folder.display())),
    };
    ensure!(
        !meta.file_type().is_symlink(),
        "'{}' is a symbolic link; it is not removed",
        folder.display()
    );
    ensure!(meta.is_dir(), "'{}' is not a folder", folder.display());
    std::fs::remove_dir_all(folder)
        .with_context(|| format!("remove the earlier round folder '{}'", folder.display()))
}

/// Refill and DRC of a scratch board, with `config` as kicad-cli's settings folder.
async fn refill(
    ctx: &JudgeContext,
    config: Option<&Path>,
    board: &Path,
) -> Result<crate::tools::cli::DrcReport> {
    crate::tools::cli::refill_drc_on_private_copy_with_config(&ctx.kicad_cli, board, config).await
}

/// One kicad-cli user's own copy of the call's private settings (the baseline, or
/// one run's judge), in `<root>/kicad-config/<name>`, removed again when it is
/// dropped. The baseline and up to three judges run kicad-cli at once, and kicad-cli
/// re-saves its settings on exit, so no two of them share a settings folder, and
/// the call's copy itself is never written: every run starts from the same settings.
/// Row ZR, ZR0b: the prune's checker holds one too (`run-<i>-prune`).
pub(crate) struct OwnConfig(Option<PathBuf>);

impl OwnConfig {
    pub(crate) fn new(ctx: &JudgeContext, name: &str) -> Result<Self> {
        let Some(master) = ctx.kicad_config_home.as_deref() else {
            return Ok(Self(None));
        };
        let own = ctx.root.join("kicad-config").join(name);
        if own.exists() {
            std::fs::remove_dir_all(&own)
                .with_context(|| format!("clear the old settings copy '{}'", own.display()))?;
        }
        crate::scratch_board::copy_folder(master, &own)?;
        Ok(Self(Some(own)))
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.0.as_deref()
    }
}

impl Drop for OwnConfig {
    fn drop(&mut self) {
        if let Some(own) = &self.0 {
            let _ = std::fs::remove_dir_all(own);
        }
    }
}

/// The KiCad versions whose settings folder is looked for under `KICAD_CONFIG_HOME`,
/// highest first.
const CONFIG_VERSIONS: [&str; 3] = ["10.0", "9.0", "8.0"];

/// The settings folder of the highest version present under `root`, with its name.
fn config_source_in(root: &Path) -> Option<(PathBuf, String)> {
    CONFIG_VERSIONS
        .iter()
        .find(|version| root.join(version).is_dir())
        .map(|version| (root.join(version), version.to_string()))
}

/// KiCad's current settings folder: under `$KICAD_CONFIG_HOME` when that is set, else
/// the one `tools::kicad_config_dir` names.
fn config_source() -> Option<(PathBuf, String)> {
    match std::env::var_os("KICAD_CONFIG_HOME").filter(|home| !home.is_empty()) {
        Some(root) => config_source_in(Path::new(&root)),
        None => {
            let dir = crate::tools::kicad_config_dir();
            let version = dir.file_name()?.to_str()?.to_string();
            dir.is_dir().then_some((dir, version))
        }
    }
}

/// The call's private copy of KiCad's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrivateConfig {
    /// The folder every judge's own copy is made from (see [`OwnConfig`]).
    pub home: PathBuf,
    /// Links in the source settings, left out of the copy (reported, not fatal).
    pub skipped_links: Vec<String>,
}

/// A private copy of KiCad's current settings, made once per call: the settings
/// folder of the current version is copied into `into/<version>/`, and `into` is
/// the value for `KICAD_CONFIG_HOME` the judges' own copies come from. kicad-cli
/// writes to its settings when it runs; this keeps it off the owner's real ones.
/// The version folder is read through a link to it; a link inside it is left out
/// (a copied link would let kicad-cli write through it) and named in
/// `skipped_links`, so a linked file never stops a call. When there is no settings
/// folder to copy, `into` is left empty (KiCad then uses its defaults) and a log
/// line says so.
pub(crate) fn private_kicad_config(into: &Path) -> Result<PrivateConfig> {
    copy_kicad_config(config_source(), into)
}

fn copy_kicad_config(source: Option<(PathBuf, String)>, into: &Path) -> Result<PrivateConfig> {
    std::fs::create_dir_all(into).with_context(|| {
        format!(
            "create the private KiCad settings folder '{}'",
            into.display()
        )
    })?;
    let mut skipped_links = Vec::new();
    match source {
        None => tracing::warn!(
            "no KiCad settings folder to copy; kicad-cli runs on '{}' with its defaults",
            into.display()
        ),
        Some((folder, version)) => {
            let real = std::fs::canonicalize(&folder)
                .with_context(|| format!("resolve '{}'", folder.display()))?;
            copy_settings(&real, &into.join(&version), &mut skipped_links)?;
            for link in &skipped_links {
                tracing::warn!("the KiCad settings link '{link}' is left out of the judge's copy");
            }
        }
    }
    Ok(PrivateConfig {
        home: into.to_path_buf(),
        skipped_links,
    })
}

/// `from` and everything below it copied to `to` as regular files and folders; a
/// link, or anything that is neither a file nor a folder, is left out and named in
/// `skipped`.
fn copy_settings(from: &Path, to: &Path, skipped: &mut Vec<String>) -> Result<()> {
    std::fs::create_dir_all(to).with_context(|| format!("create '{}'", to.display()))?;
    let mut entries: Vec<_> = std::fs::read_dir(from)
        .with_context(|| format!("read '{}'", from.display()))?
        .collect::<std::io::Result<_>>()
        .with_context(|| format!("read '{}'", from.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let source = entry.path();
        let kind = entry
            .file_type()
            .with_context(|| format!("inspect '{}'", source.display()))?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_settings(&source, &target, skipped)?;
        } else if kind.is_file() {
            std::fs::copy(&source, &target).with_context(|| {
                format!("copy '{}' to '{}'", source.display(), target.display())
            })?;
        } else {
            skipped.push(source.display().to_string());
        }
    }
    Ok(())
}

/// A tool of the registry, in whichever toolset holds it.
fn find_tool(name: &str) -> Option<crate::tools::ToolDef> {
    crate::router::registry::ALL_TOOLSETS
        .iter()
        .find_map(|toolset| {
            crate::router::registry::tools_for(toolset.name)?
                .into_iter()
                .find(|tool| tool.name == name)
        })
}

/// Calls a registry tool in-process: whether it refused (`is_error`) and its first
/// text content.
async fn call_tool(
    tools: &Arc<crate::tools::ToolContext>,
    name: &str,
    args: &Value,
) -> Result<(bool, String)> {
    let tool = find_tool(name).with_context(|| format!("tool '{name}' is not in the registry"))?;
    let result = (tool.handler)(args, tools.clone()).await?;
    let text = result
        .content
        .iter()
        .find_map(|content| match content {
            crate::mcp::protocol::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    Ok((result.is_error, text))
}

/// G2's baseline: the saved board, unchanged, in `root/baseline`
/// (`scratch_board::copy_project`, then the text written), refilled and checked by
/// `tools::cli::refill_drc_on_private_copy_with_config` (with the context's private
/// KiCad settings); violations counted by `rule`, and the
/// unconnected items counted (a report with no unconnected category is an error,
/// never zero).
pub(crate) async fn baseline(ctx: &JudgeContext) -> Result<Baseline> {
    let folder = ctx.root.join("baseline");
    if folder.exists() {
        std::fs::remove_dir_all(&folder)
            .with_context(|| format!("clear the old baseline folder '{}'", folder.display()))?;
    }
    let board = crate::scratch_board::copy_project(&ctx.board, &folder)?;
    write_board(&board, &ctx.board_text)?;
    // The same severities as the judged boards, so that G2 compares like with like.
    if ctx
        .lengths
        .as_deref()
        .is_some_and(|checks| !checks.is_empty())
    {
        crate::length_probe::force_check_severities(&board)?;
    }
    let config = OwnConfig::new(ctx, "baseline")?;
    let report = refill(ctx, config.path(), &board)
        .await
        .context("the unrouted board's refill and DRC failed")?;
    let (pour_fills, pour_fills_error) = baseline_fills(&board, &ctx.pour_nets).await;
    Ok(Baseline {
        violations: violations_by_rule(&report),
        unconnected: unconnected_total(&report)?,
        pour_fills,
        pour_fills_error,
    })
}

/// Row ZR, ZR5 part 1: the layer fills of the refilled baseline `board`, read off the
/// async threads, or why they could not be read; neither when the call has no pour net.
/// Never an error: the pour record never fails the baseline (PK3, recorded only).
async fn baseline_fills(
    board: &Path,
    pour_nets: &BTreeSet<String>,
) -> (Option<Vec<crate::pour_damage::LayerFill>>, Option<String>) {
    if pour_nets.is_empty() {
        return (None, None);
    }
    let path = board.to_path_buf();
    let nets = pour_nets.clone();
    let read = tokio::task::spawn_blocking(move || -> Result<_> {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("read the baseline board '{}'", path.display()))?;
        let fills = crate::zone_fill::pour_fills(&text, &nets)?;
        Ok(crate::pour_damage::layer_fills(&fills))
    })
    .await
    .context("the baseline's pour fills did not finish");
    match read.and_then(|read| read) {
        Ok(rows) => (Some(rows), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    }
}

/// Writes a new scratch board; the file must not exist yet.
fn write_board(board: &Path, text: &str) -> Result<()> {
    konnect_sexp::write_new_atomic(board, text)
        .map_err(|error| anyhow::anyhow!("write scratch board '{}': {error}", board.display()))
}

/// The measures of one judged run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Measures {
    /// KiCad's count after the refill and the stitching.
    pub unconnected: usize,
    /// KiCad's violations by type, same board.
    pub violations: BTreeMap<String, usize>,
    /// The audit's `quality.routed_share`.
    pub routed_share: Option<f64>,
    /// The audit's `completion.complete`.
    pub audit_complete: Option<bool>,
    /// The audit's metrics `5.3.acute_corners` and `5.3.right_angle_corners`.
    pub acute: Option<f64>,
    pub right_angles: Option<f64>,
    /// The audit's `quality.copper_quality`.
    pub copper_quality: Option<f64>,
    /// The audit's metrics `4.2.nets_undersized` and `4.3.narrow_paths`, when it ran
    /// those checks.
    pub nets_undersized: Option<f64>,
    pub narrow_paths: Option<f64>,
    /// Track length from the run's session, mm, by category: pour nets, power nets
    /// (`JudgeContext::power_nets`), the rest.
    pub signal_mm: f64,
    pub power_mm: f64,
    pub pour_track_mm: f64,
    /// The session's vias on nets that are not pour nets.
    pub vias_off_pour: usize,
    /// The session's tracks and arcs below their net's floor (beyond
    /// [`FLOOR_TOLERANCE_MM`]).
    pub below_floor: usize,
    /// `intent_width_report` rows of this session (one per judged net).
    pub width_rows: Vec<Value>,
    /// The longest stretch below intent width per net, mm, from the same report.
    pub longest_below_mm: BTreeMap<String, f64>,
    pub stitching_vias: usize,
    /// The route's own time, seconds.
    pub route_s: f64,
    /// Row LT: what the length probe found, when the call checks a group or a pair
    /// skew (`length_probe::check`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lengths: Option<crate::length_probe::LengthMeasures>,
    /// Row ZR, ZR0: the pours of the run (`zone_fill::pour_measures`), when the call
    /// has pour nets. Recorded only: no gate and no rank-key position reads it (Z10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pours: Option<crate::zone_fill::PourMeasures>,
    /// Row ZR, ZR0: why the pours could not be measured, when they could not. A
    /// failed pour measure never fails the judged run: it is recorded only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pours_error: Option<String>,
}

/// Each gate passes when its list is empty; each entry is one readable line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Gates {
    pub g1: Vec<String>,
    pub g2: Vec<String>,
    pub g3: Vec<String>,
    /// Row LT, length (L9): the intent's groups and pair skews on KiCad's own
    /// lengths, and KiCad's own `length` and `skew` rules, absolute.
    #[serde(default)]
    pub g4: Vec<String>,
}

impl Gates {
    pub(crate) fn all_pass(&self) -> bool {
        self.g1.is_empty() && self.g2.is_empty() && self.g3.is_empty() && self.g4.is_empty()
    }
}

/// One judged run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Judged {
    pub measures: Measures,
    pub gates: Gates,
    /// The last board the judge wrote: routed, refilled and stitched.
    pub board: PathBuf,
    /// One row per stitching round: `round`, `open_before`, `proposals`,
    /// `written_vias`, `open_after`, or `planner_refused`.
    pub rounds: Vec<Value>,
    /// Row LT, L23: what the judge did with the run's Tune candidates, for a run that
    /// had any. `board`, `measures` and `gates` are then the chosen board's, and the
    /// scheduler rewrites the run's `Routed.held` from the choice
    /// ([`TuneChoice::apply`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tune: Option<TuneChoice>,
}

/// One run, judged by Konnect itself under `ctx.root/run-<index>`:
/// 1. `specctra_ses::parse_import_plan_with(ctx.board, ctx.board_text,
///    ctx.manifest_text, session, SourceCheck::BytesOrTree)`: a session the importer
///    would refuse fails the judge (an error, not a candidate). The judge works on
///    the saved board, so it accepts the same identity as the preflight
///    ([`saved_board_matches`]): the bytes of `source_sha256` or the tree of
///    `source_canonical_sha256`. The live import (`apply_specctra_ses`) keeps the
///    byte check only.
/// 2. `scratch_board::copy_project` into `r0`, then `scratch_board::write_routing`
///    of the plan over the saved text, written as `r0/<board file name>`.
/// 3. `tools::cli::refill_drc_on_private_copy_with_config` on that board (every
///    kicad-cli of the judge runs with `ctx.kicad_config_home` as `KICAD_CONFIG_HOME`).
/// 4. Stitching, only for `routed.planes` nets, for at most [`STITCH_ROUNDS`] rounds
///    while one of those nets has an unconnected item (an unconnected item that
///    names `[<net>]` in an item description): `plan_route_refinement` through the
///    tool registry (`board`, `kinds: ["stitching"]`, `nets`, `search_radius_mm`
///    [`STITCH_RADIUS_MM`], `effort: "standard"`). A refusal or no proposal stops
///    the rounds (recorded). Else `scratch_board::planned_items` +
///    `scratch_board::write_planned` into `r<n>` (project copied the same way),
///    refill and DRC; the round's row records the counts.
/// 5. `audit_layout_quality` through the tool registry on the last board.
/// 6. Row LT: [`judge_lengths`] on the last board when the call checks a group or a
///    pair skew.
/// 7. [`measure`], then [`gates`]. The folders of earlier rounds are removed whole
///    (board, report and copied project files), once the next round's board is
///    written and checked; the last round's folder is kept, and its board is named in
///    `Judged::board`.
///
/// Row LT, L23: a run whose Tune pass gave candidates (`routed.held.tune.candidates`)
/// is judged by [`judge_candidates`]: steps 1 to 7 on its untuned session and on each
/// candidate, and the result is the chosen board's, with [`Judged::tune`] set.
pub(crate) async fn judge(
    ctx: &JudgeContext,
    routed: &Routed,
    baseline: &Baseline,
    tools: Arc<crate::tools::ToolContext>,
) -> Result<Judged> {
    judge_with(ctx, routed, baseline, &RegistryTools(tools)).await
}

/// One registry tool call in flight: whether it refused, and its first text.
pub(crate) type ToolCall<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(bool, String)>> + Send + 'a>>;

/// How the judge calls the tools of the registry: whether the call refused
/// (`is_error`) and its first text. The one implementation is [`RegistryTools`]; the
/// tests of the round logic put a scripted planner in its place, and the tests of the
/// ground-via prune (Row ZR, ZR0b) a scripted audit.
pub(crate) trait ToolCaller: Send + Sync {
    fn call<'a>(&'a self, name: &'a str, args: &'a Value) -> ToolCall<'a>;
}

/// The production [`ToolCaller`]: the tool registry, in-process.
pub(crate) struct RegistryTools(pub(crate) Arc<crate::tools::ToolContext>);

impl ToolCaller for RegistryTools {
    fn call<'a>(&'a self, name: &'a str, args: &'a Value) -> ToolCall<'a> {
        Box::pin(async move { call_tool(&self.0, name, args).await })
    }
}

/// [`judge`] with the tool calls behind [`ToolCaller`]: the run's one session in
/// `run-<index>`, or, when the Tune pass gave candidates, [`judge_candidates`].
async fn judge_with(
    ctx: &JudgeContext,
    routed: &Routed,
    baseline: &Baseline,
    tools: &dyn ToolCaller,
) -> Result<Judged> {
    if routed.held.tune.candidates.is_empty() {
        return judge_one(
            ctx,
            routed,
            baseline,
            tools,
            &routed.held.ses,
            &format!("run-{}", routed.index),
            routed.held.planes.as_ref(),
        )
        .await;
    }
    let (mut judged, choice) = judge_candidates(ctx, routed, baseline, tools).await?;
    judged.tune = Some(choice);
    Ok(judged)
}

/// Steps 1 to 7 of [`judge`] on one session of the run, in `ctx.root/<folder_name>`
/// (`run-<index>`, `run-<index>-untuned`, `run-<index>-pour-safe`, and Row ZR, ZR0b's
/// `run-<index>-pruned` through [`judge_session`]; the folder name also names the
/// judge's own KiCad settings copy). The prune's own checks use `run-<index>-prune`.
///
/// Row ZR, ZR1: `engine` is the `planes` object of the routing pass that made this
/// session (`fastroute::Held::planes`; `None` for a Tune candidate and for the pruned
/// session, whose boards the model is not of). When the run has pour nets and `engine`
/// reads with no error of the engine's, KiCad's open pads of the modelled nets are taken
/// on the first refilled board ([`routed_scratch`]'s), before any stitching round
/// (`zone_fill::kicad_open_record`, which reads the board right then: a stitching round
/// removes that board's folder), and compared after the measures
/// (`zone_fill::record_comparison`). Neither side's failure fails the judge.
async fn judge_one(
    ctx: &JudgeContext,
    routed: &Routed,
    baseline: &Baseline,
    tools: &dyn ToolCaller,
    session: &Path,
    folder_name: &str,
    engine: Option<&Value>,
) -> Result<Judged> {
    // 1 to 3. The session, read by the importer's own checks (a session it would
    //    refuse is a failed judge), written into a scratch copy of the saved board,
    //    then KiCad's refill and DRC on this judge's own settings copy.
    let ses_text = tokio::fs::read_to_string(session)
        .await
        .with_context(|| format!("read the session '{}'", session.display()))?;
    let run_dir = ctx.root.join(folder_name);
    let config = OwnConfig::new(ctx, folder_name)?;
    let Scratch {
        plan,
        widths,
        board,
        drc,
    } = routed_scratch(ctx, &ses_text, &run_dir, config.path(), false)
        .await
        .map_err(ScratchError::into_error)?;
    let (mut board, mut drc) = (board, drc);
    // Row ZR, ZR0: the pours' open count is taken before any stitching round.
    let first_drc = (!ctx.pour_nets.is_empty()).then(|| drc.clone());
    // Row ZR, ZR1: the engine's prediction, and KiCad's open pads of the modelled nets
    // on this first refilled board, before a stitching round removes it.
    let engine = engine
        .filter(|_| first_drc.is_some())
        .map(crate::zone_fill::planes_object);
    let (kicad_open, kicad_unconnected) = match &engine {
        Some(Ok(planes)) if planes.error.is_none() => {
            let nets: Vec<String> = planes.nets.iter().cloned().collect();
            (
                Some(
                    crate::zone_fill::kicad_open_record(
                        &board,
                        ctx.pad_names.clone(),
                        planes.nets.clone(),
                    )
                    .await,
                ),
                unconnected_on(&drc, &nets),
            )
        }
        _ => (None, 0),
    };

    // 4. Stitching, for the nets the router got as planes.
    let planes = &routed.planes;
    let mut rounds: Vec<Value> = Vec::new();
    let mut stitching_vias = 0usize;
    if !planes.is_empty() {
        for round in 1..=STITCH_ROUNDS {
            let open_before = unconnected_on(&drc, planes);
            if open_before == 0 {
                break;
            }
            let stopped = |extra: Value| {
                let mut row = json!({ "round": round, "open_before": open_before });
                if let (Some(row), Some(extra)) = (row.as_object_mut(), extra.as_object()) {
                    row.extend(extra.clone());
                }
                row
            };
            let args = json!({
                "board": board.display().to_string(),
                "kinds": ["stitching"],
                "nets": planes,
                "search_radius_mm": STITCH_RADIUS_MM,
                "effort": "standard",
            });
            let plan_json = match tools.call("plan_route_refinement", &args).await {
                Err(error) => {
                    rounds.push(stopped(json!({ "planner_refused": format!("{error:#}") })));
                    break;
                }
                Ok((true, text)) => {
                    let reason = serde_json::from_str::<Value>(&text)
                        .ok()
                        .and_then(|body| body["blocked"][0]["reason"].as_str().map(str::to_string))
                        .unwrap_or_else(|| clipped(&text));
                    rounds.push(stopped(json!({ "planner_refused": reason })));
                    break;
                }
                Ok((false, text)) => match serde_json::from_str::<Value>(&text) {
                    Ok(body) => body,
                    Err(error) => {
                        rounds.push(stopped(
                            json!({ "planner_refused": format!("unreadable planner result: {error}") }),
                        ));
                        break;
                    }
                },
            };
            let proposals = plan_json["proposals"].as_array().map_or(0, Vec::len);
            if proposals == 0 {
                rounds.push(stopped(json!({ "proposals": 0 })));
                break;
            }
            // Every proposed item is written, as the research's `post` adopted
            // every listed via.
            let items = match crate::scratch_board::planned_items(&plan_json) {
                Ok(items) => items,
                Err(error) => {
                    rounds.push(stopped(json!({
                        "proposals": proposals,
                        "planner_refused": format!("proposals not usable: {error:#}"),
                    })));
                    break;
                }
            };
            if items.tracks.is_empty() && items.vias.is_empty() {
                rounds.push(stopped(
                    json!({ "proposals": proposals, "written_vias": 0 }),
                ));
                break;
            }
            let current_text = tokio::fs::read_to_string(&board)
                .await
                .with_context(|| format!("read the scratch board '{}'", board.display()))?;
            let next_text = match crate::scratch_board::write_planned(&current_text, &items) {
                Ok(text) => text,
                Err(error) => {
                    rounds.push(stopped(json!({
                        "proposals": proposals,
                        "planner_refused": format!("proposals not writable: {error:#}"),
                    })));
                    break;
                }
            };
            let next_board =
                crate::scratch_board::copy_project(&board, &run_dir.join(format!("r{round}")))?;
            write_board(&next_board, &next_text)?;
            let next_drc = refill(ctx, config.path(), &next_board)
                .await
                .with_context(|| format!("refill and DRC after stitching round {round}"))?;
            // The earlier round's whole folder is no longer needed: its board, its
            // report and the project files copied beside them.
            let cleanup = remove_round_folder(&ctx.root, &board);
            board = next_board;
            drc = next_drc;
            stitching_vias += items.vias.len();
            let mut row = stopped(json!({
                "proposals": proposals,
                "written_vias": items.vias.len(),
                "open_after": unconnected_on(&drc, planes),
            }));
            if let Err(error) = cleanup {
                row["earlier_round_kept"] = json!(format!("{error:#}"));
            }
            rounds.push(row);
        }
    }

    // 5. The layout audit on the last board: a refusal is a failed judge.
    let audit = audit(tools, &board).await?;

    // 6. Row LT: KiCad's own lengths of the checked nets, on a copy of the last board.
    let lengths = match ctx.lengths.as_deref() {
        Some(checks) => {
            judge_lengths(
                ctx,
                checks,
                config.path(),
                &board,
                &run_dir,
                &ses_text,
                &drc,
            )
            .await?
        }
        None => None,
    };

    // 7. Measures and gates.
    let mut measures = measure(
        ctx,
        &drc,
        &audit,
        &plan,
        &widths,
        stitching_vias,
        routed.held.seconds,
    )?;
    measures.lengths = lengths;
    if let Some(first_drc) = first_drc {
        let taken = crate::zone_fill::pour_record(
            &board,
            first_drc,
            drc.clone(),
            &ctx.pour_nets,
            baseline.pour_fills.as_deref(),
        )
        .await;
        crate::zone_fill::record_pours(&mut measures, taken);
        if let Some(engine) = engine {
            crate::zone_fill::record_comparison(
                &mut measures,
                engine,
                kicad_open,
                kicad_unconnected,
            );
        }
    }
    let gates = gates(&measures, baseline);
    Ok(Judged {
        measures,
        gates,
        board,
        rounds,
        tune: None,
    })
}

/// What [`routed_scratch`] gives: the plan, the width report, the board's path, the DRC.
pub(crate) struct Scratch {
    pub plan: crate::specctra_ses::SesImportPlan,
    pub widths: crate::intent_width_report::WidthReport,
    /// `<run_dir>/r0/<board file name>`, refilled and saved by kicad-cli.
    pub board: PathBuf,
    pub drc: crate::tools::cli::DrcReport,
}

/// `Importer`: `parse_import_plan_with` or the width report refused. `Other`: the rest.
#[derive(Debug)]
pub(crate) enum ScratchError {
    Importer(anyhow::Error),
    Other(anyhow::Error),
}

impl ScratchError {
    /// The error itself, whichever step gave it ([`judge_one`] does not tell them apart).
    pub(crate) fn into_error(self) -> anyhow::Error {
        match self {
            ScratchError::Importer(error) | ScratchError::Other(error) => error,
        }
    }
}

/// Steps 1 to 3 of [`judge`] on a session TEXT: the importer's plan and the width
/// report, the scratch board `<run_dir>/r0/<board file>` with the routing written and
/// the severities forced as the call's checks ask (and, with `prune_rules`, P11's four
/// rules at least warnings), and KiCad's refill and DRC on `config`. The importer's
/// refusal is told apart from every other error.
///
/// The order is [`judge_one`]'s: the importer and the width report, then `run_dir` is
/// cleared (so a session the importer refuses leaves the folder as it was), then the
/// board is written and the refill runs. The errors keep `judge_one`'s wording.
pub(crate) async fn routed_scratch(
    ctx: &JudgeContext,
    ses_text: &str,
    run_dir: &Path,
    config: Option<&Path>,
    prune_rules: bool,
) -> std::result::Result<Scratch, ScratchError> {
    let plan = crate::specctra_ses::parse_import_plan_with(
        &ctx.board,
        &ctx.board_text,
        &ctx.manifest_text,
        ses_text,
        SourceCheck::BytesOrTree,
    )
    .context("the importer refuses the run's session")
    .map_err(ScratchError::Importer)?;
    let widths = crate::intent_width_report::intent_width_report(&ctx.manifest, ses_text)
        .context("the width report of the run's session")
        .map_err(ScratchError::Importer)?;
    let other = ScratchError::Other;

    // The session written into a scratch copy of the saved board.
    if run_dir.exists() {
        std::fs::remove_dir_all(run_dir)
            .with_context(|| format!("clear the old run folder '{}'", run_dir.display()))
            .map_err(other)?;
    }
    let board =
        crate::scratch_board::copy_project(&ctx.board, &run_dir.join("r0")).map_err(other)?;
    let routed_text = crate::scratch_board::write_routing(&ctx.board_text, &plan).map_err(other)?;
    write_board(&board, &routed_text).map_err(other)?;
    // The board's own length and skew rules must be seen whatever the project's
    // severities say (G4(c)): forced in this scratch copy's project, never the owner's.
    if ctx
        .lengths
        .as_deref()
        .is_some_and(|checks| checks.board_rules)
    {
        crate::length_probe::force_length_severities(&board).map_err(other)?;
    }
    // A call with checks must see dangling and unconnected items whatever the project's
    // severities say (G4(d), the harm check): forced in this scratch copy only.
    if ctx
        .lengths
        .as_deref()
        .is_some_and(|checks| !checks.is_empty())
    {
        crate::length_probe::force_check_severities(&board).map_err(other)?;
    }
    // Row ZR, ZR0b (P11): the prune's test can refuse only what KiCad reports.
    if prune_rules {
        crate::length_probe::force_prune_severities(&board).map_err(other)?;
    }

    // KiCad's refill and DRC, on the caller's settings copy.
    let drc = refill(ctx, config, &board)
        .await
        .context("refill and DRC of the routed board")
        .map_err(other)?;
    Ok(Scratch {
        plan,
        widths,
        board,
        drc,
    })
}

/// Step 5: the layout audit of `board` through the tool registry; a refusal is an error.
async fn audit(tools: &dyn ToolCaller, board: &Path) -> Result<Value> {
    let (refused, text) = tools
        .call(
            "audit_layout_quality",
            &json!({ "board": board.display().to_string() }),
        )
        .await
        .context("the layout audit failed")?;
    ensure!(!refused, "the layout audit refused: {}", clipped(&text));
    serde_json::from_str(&text).context("the layout audit result is not JSON")
}

/// ZR0b: the rest of a light judge on a scratch already made: [`audit`], [`measure`]
/// (0 stitching vias, `route_s` as given), [`gates`]; no stitching round, no length
/// probe, no pour record. Returns the measures, the gates and the audit's JSON.
///
/// The tools are a [`ToolCaller`] (production: [`RegistryTools`]), so that the tests of
/// the prune script the audit.
pub(crate) async fn judge_light(
    ctx: &JudgeContext,
    baseline: &Baseline,
    tools: &dyn ToolCaller,
    scratch: &Scratch,
    route_s: f64,
) -> Result<(Measures, Gates, Value)> {
    let audit = audit(tools, &scratch.board).await?;
    let measures = measure(
        ctx,
        &scratch.drc,
        &audit,
        &scratch.plan,
        &scratch.widths,
        0,
        route_s,
    )?;
    let gates = gates(&measures, baseline);
    Ok((measures, gates, audit))
}

/// ZR0b: [`judge_one`] on one session file of a run in `ctx.root/<folder_name>`, for a
/// caller outside this module (the prune's full judge of the pruned session). The
/// tools are a [`ToolCaller`], as for [`judge_light`]. Row ZR, ZR1: no comparison with
/// the engine's model, which is of the routing pass's board, not the pruned one.
pub(crate) async fn judge_session(
    ctx: &JudgeContext,
    routed: &Routed,
    baseline: &Baseline,
    tools: &dyn ToolCaller,
    session: &Path,
    folder_name: &str,
) -> Result<Judged> {
    judge_one(ctx, routed, baseline, tools, session, folder_name, None).await
}

/// Row LT, step 6 of the judge (after the audit): when `checks` is not empty,
/// `length_probe::probe_lengths` on the last board, in `<run dir>/lengths`, with the
/// run's own KiCad settings; G4(d)'s faults of the checked nets
/// (`length_probe::path_faults`, from `fastroute::self_contacts` of the judged
/// session's text `ses_text` and the last board's `drc` report); then
/// `length_probe::check`. `None` when nothing is checked. A probe error, or a session
/// the self-contact scan cannot read, fails the judge.
pub(crate) async fn judge_lengths(
    ctx: &JudgeContext,
    checks: &crate::length_probe::LengthChecks,
    config: Option<&Path>,
    board: &Path,
    run_dir: &Path,
    ses_text: &str,
    drc: &crate::tools::cli::DrcReport,
) -> Result<Option<crate::length_probe::LengthMeasures>> {
    if checks.is_empty() {
        return Ok(None);
    }
    let nets = checks.nets();
    let (lengths, unmeasured, probe_s) = crate::length_probe::probe_lengths(
        &ctx.kicad_cli,
        config,
        board,
        &run_dir.join(crate::length_probe::PROBE_FOLDER),
        &nets,
    )
    .await
    .context("the length probe failed")?;
    // The pairwise scan is quadratic per layer: off the async threads.
    let wanted: BTreeSet<String> = nets.iter().cloned().collect();
    let text = ses_text.to_string();
    let contacts =
        tokio::task::spawn_blocking(move || crate::fastroute::self_contacts(&text, &wanted))
            .await
            .context("the self-contact scan did not finish")?
            .context("the judged session's self contacts could not be read")?;
    let faults = crate::length_probe::path_faults(checks, &contacts, drc);
    Ok(Some(crate::length_probe::check(
        checks, &lengths, unmeasured, faults, probe_s,
    )))
}

/// Row LT, L23: what the judge did with a run's Tune candidates.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct TuneChoice {
    /// The candidate whose board is the run's; `None`: the untuned board.
    pub chosen: Option<crate::fastroute::TuneVariant>,
    /// One row per candidate: `variant`, `passes`, `dropped` (net and reason),
    /// `judged`, `g4_failed`, `rejected` (its [`tune_harm`] lines, or why the judge
    /// failed on it), `chosen`, and `reason` for a candidate that was not judged.
    pub rows: Vec<Value>,
    /// The untuned board's measures, for the record.
    pub untuned: Measures,
}

impl TuneChoice {
    /// Rewrites the run's `held` from the choice, so that the SES the call writes and
    /// the `kept_pass` it reports are those of the judged board:
    /// - a candidate: `ses`, `kept` ([`crate::fastroute::TUNE_PASS`]) and `unrouted`
    ///   are its, the run's Tune row in `passes` is the candidate's last row, and that
    ///   row in `tune.rows` says `kept`;
    /// - none: `ses`, `kept` and `unrouted` are the untuned board's, which the Tune
    ///   attempts leave in `held`, and the Tune row in `passes` that says `candidate`
    ///   now says `discarded`, with [`TuneChoice::untuned_reason`].
    ///
    /// Every other `candidate` row of `tune.rows` says `not_chosen` (an unjudged run's
    /// rows keep saying `candidate`). An error when the chosen variant is not among the
    /// run's candidates: the written session would then differ from the judged board,
    /// so the judge fails.
    pub(crate) fn apply(&self, held: &mut crate::fastroute::Held) -> Result<()> {
        use crate::fastroute::TUNE_PASS;
        let mut chosen_row = None;
        if let Some(variant) = self.chosen {
            let candidate = held
                .tune
                .candidates
                .iter()
                .find(|candidate| candidate.variant == variant)
                .cloned()
                .with_context(|| {
                    format!(
                        "the judge chose the {variant:?} Tune candidate, which the run does not hold"
                    )
                })?;
            held.ses = candidate.ses;
            held.kept = TUNE_PASS;
            held.unrouted = candidate.unrouted;
            chosen_row =
                held.tune.rows.iter().rposition(|row| {
                    row["variant"] == json!(variant) && row["outcome"] == "candidate"
                });
        }
        for (at, row) in held.tune.rows.iter_mut().enumerate() {
            if row["outcome"] == "candidate" {
                row["outcome"] = json!(if Some(at) == chosen_row {
                    "kept"
                } else {
                    "not_chosen"
                });
            }
        }
        match (self.chosen, chosen_row) {
            (Some(_), Some(at)) => {
                let row = held.tune.rows[at].clone();
                match held
                    .passes
                    .iter()
                    .position(|pass| pass["name"] == TUNE_PASS)
                {
                    Some(at) => held.passes[at] = row,
                    None => held.passes.push(row),
                }
            }
            // A chosen candidate whose pass row is missing leaves `passes` as it is.
            (Some(_), None) => {}
            (None, _) => {
                let why = self.untuned_reason();
                if let Some(row) = held
                    .passes
                    .iter_mut()
                    .find(|pass| pass["name"] == TUNE_PASS && pass["outcome"] == "candidate")
                {
                    row["outcome"] = json!("discarded");
                    row["reason"] = json!(format!("the judge kept the untuned board: {why}"));
                }
            }
        }
        Ok(())
    }

    /// Why the judge kept the untuned board, from the candidates' rows: each candidate's
    /// `tune_harm` lines, or the reason it was not judged or ranked no better
    /// ("free: unconnected items 21 → 25 / pour_safe: ranked no better than the untuned
    /// board").
    pub(crate) fn untuned_reason(&self) -> String {
        let parts: Vec<String> = self
            .rows
            .iter()
            .filter_map(|row| {
                let variant = row["variant"].as_str()?;
                let lines: Vec<&str> = row["rejected"]
                    .as_array()
                    .map(|lines| lines.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                let why = if lines.is_empty() {
                    row["reason"].as_str()?.to_string()
                } else {
                    lines.join("; ")
                };
                Some(format!("{variant}: {why}"))
            })
            .collect();
        parts.join(" / ")
    }
}

/// Row LT, L23: one line for each measure on which `tuned` is worse than `untuned`
/// (the same run), by the numbers and not by the count of gate lines (a line such as
/// "N narrow path(s)" is one line whatever N is): unconnected items; each DRC
/// type's violations, except the keys [`crate::length_probe::counts_in_g2`] leaves out;
/// the audit's undersized nets and narrow paths; tracks below their floor; the routed
/// share (any drop) and the audit's completion (true to false). Each line names both
/// values ("unconnected items 21 → 25"). Stitching vias are not compared: the judge
/// adds them to repair pours, and what they leave open is in `unconnected`.
pub(crate) fn tune_harm(untuned: &Judged, tuned: &Judged) -> Vec<String> {
    let (before, after) = (&untuned.measures, &tuned.measures);
    let mut lines = Vec::new();
    if after.unconnected > before.unconnected {
        lines.push(format!(
            "unconnected items {} \u{2192} {}",
            before.unconnected, after.unconnected
        ));
    }
    for (rule, count) in &after.violations {
        if !crate::length_probe::counts_in_g2(rule) {
            continue;
        }
        let was = before.violations.get(rule).copied().unwrap_or(0);
        if *count > was {
            lines.push(format!("{rule} violations {was} \u{2192} {count}"));
        }
    }
    for (name, was, now) in [
        (
            "nets undersized",
            before.nets_undersized,
            after.nets_undersized,
        ),
        ("narrow paths", before.narrow_paths, after.narrow_paths),
    ] {
        if now.unwrap_or(0.0) > was.unwrap_or(0.0) {
            lines.push(format!(
                "{name} {} \u{2192} {}",
                was.unwrap_or(0.0),
                now.unwrap_or(0.0)
            ));
        }
    }
    if after.below_floor > before.below_floor {
        lines.push(format!(
            "tracks below their floor {} \u{2192} {}",
            before.below_floor, after.below_floor
        ));
    }
    // A routed share that drops, or that the audit no longer gives.
    match (before.routed_share, after.routed_share) {
        (Some(was), Some(now)) if now < was => {
            lines.push(format!("routed share {was} \u{2192} {now}"));
        }
        (Some(was), None) => lines.push(format!("routed share {was} \u{2192} none")),
        _ => {}
    }
    if before.audit_complete == Some(true) && after.audit_complete != Some(true) {
        lines.push(format!(
            "audit completion true \u{2192} {}",
            after
                .audit_complete
                .map_or("none".to_string(), |complete| complete.to_string())
        ));
    }
    lines
}

/// Row LT, L23 and L24: judges the run's untuned session in `run-<i>-untuned`, then
/// the `Free` candidate in `run-<i>`; the `PourSafe` candidate in
/// `run-<i>-pour-safe` only when `Free` is missing, rejected or fails G4. A candidate
/// with a [`tune_harm`] line is rejected (a candidate the judge fails on is rejected
/// with that error: the untuned board stands). The pool is the untuned board and every
/// candidate left: the best by the rank key ([`rank`]) is the run's board, and on an
/// exact tie the untuned board wins, then the first candidate: no meander without a gain
/// (a candidate that overshoots a window cannot beat the untuned board). Returns the chosen board's [`Judged`] with the [`TuneChoice`];
/// the scheduler rewrites the run's `held` from it ([`TuneChoice::apply`]). A failure
/// of the untuned judge fails the judge.
async fn judge_candidates(
    ctx: &JudgeContext,
    routed: &Routed,
    baseline: &Baseline,
    tools: &dyn ToolCaller,
) -> Result<(Judged, TuneChoice)> {
    use crate::fastroute::TuneVariant;
    let held = &routed.held;
    let index = routed.index;
    let untuned_ses = held.untuned.clone().unwrap_or_else(|| held.ses.clone());
    let untuned = judge_one(
        ctx,
        routed,
        baseline,
        tools,
        &untuned_ses,
        &format!("run-{index}-untuned"),
        held.planes.as_ref(),
    )
    .await
    .context("judging the run's untuned session")?;

    let mut rows: Vec<Value> = Vec::new();
    // (row, variant, judged) of each candidate tuning did not make worse.
    let mut harmless: Vec<(usize, TuneVariant, Judged)> = Vec::new();
    let mut free_is_enough = false;
    for candidate in &held.tune.candidates {
        let mut row = json!({
            "variant": candidate.variant,
            "passes": candidate.passes,
            "dropped": candidate
                .dropped
                .iter()
                .map(|(net, reason)| json!({ "net": net, "reason": reason }))
                .collect::<Vec<_>>(),
            "judged": false,
            "rejected": [],
            "chosen": false,
        });
        if candidate.variant == TuneVariant::PourSafe && free_is_enough {
            row["reason"] =
                json!("the free candidate was not worse than the untuned board and passes G4");
            rows.push(row);
            continue;
        }
        let folder = match candidate.variant {
            TuneVariant::Free => format!("run-{index}"),
            TuneVariant::PourSafe => format!("run-{index}-pour-safe"),
        };
        match judge_one(ctx, routed, baseline, tools, &candidate.ses, &folder, None).await {
            Err(error) => {
                row["rejected"] = json!([format!("the judge failed on it: {error:#}")]);
            }
            Ok(judged) => {
                let harm = tune_harm(&untuned, &judged);
                row["judged"] = json!(true);
                row["g4_failed"] = json!(!judged.gates.g4.is_empty());
                row["rejected"] = json!(harm);
                if harm.is_empty() {
                    if candidate.variant == TuneVariant::Free && judged.gates.g4.is_empty() {
                        free_is_enough = true;
                    }
                    harmless.push((rows.len(), candidate.variant, judged));
                }
            }
        }
        rows.push(row);
    }

    // The pool: the untuned board first, then the candidates tuning did not make worse.
    let keys = {
        let refs: Vec<&Judged> = std::iter::once(&untuned)
            .chain(harmless.iter().map(|(_, _, judged)| judged))
            .collect();
        rank(&refs)
    };
    // The least key; the earliest on a tie, so the untuned board, then `Free`.
    let best = keys
        .iter()
        .enumerate()
        .min_by_key(|(at, key)| (**key, *at))
        .map_or(0, |(at, _)| at);
    let untuned_measures = untuned.measures.clone();
    let (chosen, judged) = match best.checked_sub(1) {
        Some(at) => {
            let (row, variant, judged) = harmless.swap_remove(at);
            rows[row]["chosen"] = json!(true);
            (Some(variant), judged)
        }
        None => (None, untuned),
    };
    // A candidate that was not harmful and still not chosen says why.
    for (row, _, _) in &harmless {
        rows[*row]["reason"] = json!(if chosen.is_some() {
            "ranked below the chosen candidate"
        } else {
            "ranked no better than the untuned board"
        });
    }
    Ok((
        judged,
        TuneChoice {
            chosen,
            rows,
            untuned: untuned_measures,
        },
    ))
}

/// The measures of a run from its parts: KiCad's last report, the audit's JSON, the
/// import plan of its session, the width report of its session, the stitching vias
/// written, and the route time.
pub(crate) fn measure(
    ctx: &JudgeContext,
    drc: &crate::tools::cli::DrcReport,
    audit: &Value,
    plan: &crate::specctra_ses::SesImportPlan,
    widths: &crate::intent_width_report::WidthReport,
    stitching_vias: usize,
    route_s: f64,
) -> Result<Measures> {
    let unconnected = unconnected_total(drc)?;
    let metric = |key: &str| audit["metrics"][key].as_f64();

    let mut lengths = Category::default();
    let mut below_floor = 0usize;
    for track in &plan.tracks {
        let length = (track.x2_mm - track.x1_mm).hypot(track.y2_mm - track.y1_mm);
        lengths.add(ctx, &track.net_name, length);
        if is_below_floor(ctx, &track.net_name, track.width_mm) {
            below_floor += 1;
        }
    }
    for arc in &plan.arcs {
        let length = arc_length_mm(
            [arc.start_x_mm, arc.start_y_mm],
            [arc.mid_x_mm, arc.mid_y_mm],
            [arc.end_x_mm, arc.end_y_mm],
        );
        lengths.add(ctx, &arc.net_name, length);
        if is_below_floor(ctx, &arc.net_name, arc.width_mm) {
            below_floor += 1;
        }
    }
    let vias_off_pour = plan
        .vias
        .iter()
        .filter(|via| !ctx.pour_nets.contains(&via.net_name))
        .count();

    let longest_below_mm: BTreeMap<String, f64> = widths
        .nets
        .iter()
        .filter_map(|row| {
            Some((
                row["net"].as_str()?.to_string(),
                row["longest_below_intent_mm"].as_f64()?,
            ))
        })
        .collect();

    Ok(Measures {
        unconnected,
        violations: violations_by_rule(drc),
        routed_share: audit["quality"]["routed_share"].as_f64(),
        audit_complete: audit["completion"]["complete"].as_bool(),
        acute: metric("5.3.acute_corners"),
        right_angles: metric("5.3.right_angle_corners"),
        copper_quality: audit["quality"]["copper_quality"].as_f64(),
        nets_undersized: metric("4.2.nets_undersized"),
        narrow_paths: metric("4.3.narrow_paths"),
        signal_mm: lengths.signal,
        power_mm: lengths.power,
        pour_track_mm: lengths.pour,
        vias_off_pour,
        below_floor,
        width_rows: widths.nets.clone(),
        longest_below_mm,
        stitching_vias,
        route_s,
        lengths: None,
        pours: None,
        pours_error: None,
    })
}

/// Track length by category (pour beats power beats signal).
#[derive(Default)]
struct Category {
    signal: f64,
    power: f64,
    pour: f64,
}

impl Category {
    fn add(&mut self, ctx: &JudgeContext, net: &str, length_mm: f64) {
        if ctx.pour_nets.contains(net) {
            self.pour += length_mm;
        } else if ctx.power_nets.contains(net) {
            self.power += length_mm;
        } else {
            self.signal += length_mm;
        }
    }
}

/// Whether a track of `net` and `width_mm` is below the net's floor by more than
/// [`FLOOR_TOLERANCE_MM`]. Compared in whole nanometres, so that the boundary is a
/// rule and not rounding noise. A net with no floor is never below one.
fn is_below_floor(ctx: &JudgeContext, net: &str, width_mm: f64) -> bool {
    let Some(floor_mm) = ctx.floors_mm.get(net) else {
        return false;
    };
    let tolerance_nm = (FLOOR_TOLERANCE_MM * 1e6).round() as i64;
    ((floor_mm - width_mm) * 1e6).round() as i64 > tolerance_nm
}

/// The length of the arc from `start` through `mid` to `end`, mm: the arc of the
/// circle through the three points that passes through `mid`. Three points on a
/// line have no circle: the length is then the chord.
fn arc_length_mm(start: [f64; 2], mid: [f64; 2], end: [f64; 2]) -> f64 {
    let chord = (end[0] - start[0]).hypot(end[1] - start[1]);
    let [ax, ay] = start;
    let [bx, by] = mid;
    let [cx, cy] = end;
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    if !d.is_finite() || d.abs() < 1e-12 {
        return chord;
    }
    let (a2, b2, c2) = (ax * ax + ay * ay, bx * bx + by * by, cx * cx + cy * cy);
    let centre_x = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / d;
    let centre_y = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / d;
    let radius = (ax - centre_x).hypot(ay - centre_y);
    let angle = |x: f64, y: f64| (y - centre_y).atan2(x - centre_x);
    let tau = std::f64::consts::TAU;
    let (a_start, a_mid, a_end) = (angle(ax, ay), angle(bx, by), angle(cx, cy));
    let to_mid = (a_mid - a_start).rem_euclid(tau);
    let to_end = (a_end - a_start).rem_euclid(tau);
    // Counter-clockwise from the start reaches the mid point before the end: the
    // arc runs that way. Otherwise it runs the other way round.
    let sweep = if to_mid <= to_end {
        to_end
    } else {
        tau - to_end
    };
    let length = radius * sweep;
    if length.is_finite() {
        length
    } else {
        chord
    }
}

/// The absolute gates (decision 10):
/// - G1 complete: 0 unconnected; the audit's routed share is 1 (within 1e-9); the
///   audit calls the routing complete. A missing audit value is a failure.
/// - G2 DRC: every violation type at most the baseline's count of that type.
/// - G3 width: no track below its floor; 0 `track_width` violations; each
///   intent-width net with at most [`BELOW_INTENT_SHARE`] of its length and no
///   stretch longer than [`BELOW_INTENT_SECTION_MM`] below its intent width; audit
///   4.2 nets undersized 0 and 4.3 narrow paths 0 when the audit gave them.
/// - G4 length (Row LT, L9): every checked group and pair skew meets its window on
///   KiCad's own lengths, no checked net is unmeasured, and KiCad's own DRC has 0
///   `length_out_of_range` and 0 `skew_out_of_range`, absolute. G2 does not count
///   those two keys. (d) (L22): no checked net's copper crosses or overlaps itself,
///   or is named in a `track_dangling` or `unconnected_items` entry of the judged
///   board's DRC.
pub(crate) fn gates(m: &Measures, baseline: &Baseline) -> Gates {
    let mut g1 = Vec::new();
    if m.unconnected != 0 {
        g1.push(format!(
            "{} unconnected item(s) after the refill and stitching",
            m.unconnected
        ));
    }
    match m.routed_share {
        None => g1.push("the audit gave no routed share".to_string()),
        Some(share) if (share - 1.0).abs() > 1e-9 => {
            g1.push(format!("the audit's routed share is {share}, not 1"))
        }
        Some(_) => {}
    }
    match m.audit_complete {
        None => g1.push("the audit gave no completion verdict".to_string()),
        Some(false) => g1.push("the audit calls the routing incomplete".to_string()),
        Some(true) => {}
    }

    let mut g2 = Vec::new();
    for (rule, count) in &m.violations {
        // Row LT: KiCad's length and skew rules are G4's, absolute (L9).
        if !crate::length_probe::counts_in_g2(rule) {
            continue;
        }
        let allowed = baseline.violations.get(rule).copied().unwrap_or(0);
        if *count > allowed {
            g2.push(format!(
                "{rule}: {count} violation(s), the unrouted board has {allowed}"
            ));
        }
    }

    let mut g3 = Vec::new();
    if m.below_floor > 0 {
        g3.push(format!(
            "{} track(s) below their net's floor width",
            m.below_floor
        ));
    }
    let track_width = m.violations.get("track_width").copied().unwrap_or(0);
    if track_width > 0 {
        g3.push(format!("{track_width} track_width violation(s)"));
    }
    for row in &m.width_rows {
        let net = row["net"].as_str().unwrap_or("?");
        let routed = row["routed_mm"].as_f64().unwrap_or(0.0);
        let below = row["below_intent_mm"].as_f64().unwrap_or(0.0);
        let share = row["below_intent_share"]
            .as_f64()
            .unwrap_or(if routed > 0.0 { below / routed } else { 0.0 });
        if share > BELOW_INTENT_SHARE {
            g3.push(format!(
                "{net}: {:.1} % of its length is below its intent width (at most {:.0} %)",
                share * 100.0,
                BELOW_INTENT_SHARE * 100.0
            ));
        }
        let longest = row["longest_below_intent_mm"]
            .as_f64()
            .unwrap_or(0.0)
            .max(m.longest_below_mm.get(net).copied().unwrap_or(0.0));
        if longest > BELOW_INTENT_SECTION_MM {
            g3.push(format!(
                "{net}: a {longest:.2} mm stretch is below its intent width (at most {BELOW_INTENT_SECTION_MM} mm)"
            ));
        }
    }
    if let Some(undersized) = m.nets_undersized.filter(|count| *count > 0.0) {
        g3.push(format!(
            "the audit finds {undersized} net(s) undersized (4.2)"
        ));
    }
    if let Some(narrow) = m.narrow_paths.filter(|count| *count > 0.0) {
        g3.push(format!("the audit finds {narrow} narrow path(s) (4.3)"));
    }

    let mut g4 = crate::length_probe::g4_lines(m.lengths.as_ref(), &m.violations);
    // Row LT, L22 (G4(d)): a checked net whose copper is not a clean path.
    if let Some(lengths) = &m.lengths {
        g4.extend(crate::length_probe::contact_lines(&lengths.paths));
    }

    Gates { g1, g2, g3, g4 }
}

/// Row LT: the length of the rank key (docs/LENGTH_TUNING_DESIGN.md, L10): G1, G2,
/// G3, G4, intent miss, KiCad's length violations, then the ten-entry key's
/// measures from acute corners on. `Key` is `[i64; KEY_LEN]`.
pub(crate) const KEY_LEN: usize = 13;

/// Where the quality measures sit in the key (acute corners to minus copper
/// quality): what the placement key copies (it read `3..9` before Row LT).
pub(crate) const KEY_QUALITY: std::ops::Range<usize> = 6..12;

/// A run's rank key; the least is the best.
pub(crate) type Key = [i64; KEY_LEN];

/// The rank key of decision 12 and L10 for every judged run at once (the length
/// steps need the shortest): G1, G2, G3 and G4 failed (0 or 1 each); the intent
/// miss (the sum of every measured net's distance outside its window, in 0.01 mm
/// steps; `i64::MAX` when a checked net is unmeasured; 0 when nothing is checked);
/// KiCad's own length and skew violations; then [`KEY_QUALITY`]: acute corners
/// (whole); signal length over the shortest signal length among the runs that pass G1
/// (among all judged runs when none does), in steps of
/// [`LENGTH_STEP`] (`round(ratio / step)`); power length the same; right angles
/// (whole); vias off the pour nets; minus copper quality (rounded to whole points);
/// and last the route seconds (rounded down). A measure the audit did not give sorts
/// last (`i64::MAX`). When every run's length of a category is 0, that step is 0 for
/// all. Without length groups, pair skews or KiCad length rules, positions 3 to 5
/// are 0 for every run and the order is the one before Row LT.
pub(crate) fn rank(judged: &[&Judged]) -> Vec<Key> {
    let runs: Vec<(&Measures, &Gates, bool)> = judged
        .iter()
        .map(|run| (&run.measures, &run.gates, true))
        .collect();
    rank_measures(&runs)
}

/// [`rank`]'s key for each run, with the length steps taken against the shortest of
/// the runs that are in the pool (the `bool`) and pass G1; against all runs in the pool
/// when none of them passes; against every run when none is in the pool. `rank(judged)`
/// is this with every run in the pool, so its keys do not change. A run outside the
/// pool still gets a key, stepped against the pool's shortest, but never sets a step
/// (a candidate with an illegal placement ranks, and does not move the others).
pub(crate) fn rank_measures(runs: &[(&Measures, &Gates, bool)]) -> Vec<Key> {
    // The shortest judged length of a category, among runs that have any: a run with
    // none of it is at least as short as the shortest, and its step is 0.
    // The reference is the shortest among the pool's runs that pass G1 (complete): a
    // degenerate run that is not complete must not set the steps. When none passes,
    // the shortest among the pool; when the pool is empty, among all runs.
    let in_pool: Vec<&(&Measures, &Gates, bool)> = runs.iter().filter(|run| run.2).collect();
    let complete: Vec<&(&Measures, &Gates, bool)> = in_pool
        .iter()
        .copied()
        .filter(|run| run.1.g1.is_empty())
        .collect();
    let pool: Vec<&(&Measures, &Gates, bool)> = if !complete.is_empty() {
        complete
    } else if !in_pool.is_empty() {
        in_pool
    } else {
        runs.iter().collect()
    };
    let shortest = |length: fn(&Measures) -> f64| -> Option<f64> {
        pool.iter()
            .map(|run| length(run.0))
            .filter(|mm| *mm > 0.0)
            .fold(None, |least: Option<f64>, mm| {
                Some(least.map_or(mm, |least| least.min(mm)))
            })
    };
    let step = |mm: f64, shortest: Option<f64>| -> i64 {
        match shortest {
            Some(shortest) if mm > 0.0 => (mm / shortest / LENGTH_STEP).round() as i64,
            _ => 0,
        }
    };
    let shortest_signal = shortest(|m| m.signal_mm);
    let shortest_power = shortest(|m| m.power_mm);
    // A measure the audit did not give sorts last.
    let whole = |value: Option<f64>| value.map_or(i64::MAX, |value| value.round() as i64);
    runs.iter()
        .map(|&(m, g, _)| {
            let [g4, miss, kicad] =
                crate::length_probe::key_part(!g.g4.is_empty(), m.lengths.as_ref(), &m.violations);
            [
                i64::from(!g.g1.is_empty()),
                i64::from(!g.g2.is_empty()),
                i64::from(!g.g3.is_empty()),
                g4,
                miss,
                kicad,
                whole(m.acute),
                step(m.signal_mm, shortest_signal),
                step(m.power_mm, shortest_power),
                whole(m.right_angles),
                m.vias_off_pour as i64,
                m.copper_quality
                    .map_or(i64::MAX, |quality| -(quality.round() as i64)),
                m.route_s.max(0.0).floor() as i64,
            ]
        })
        .collect()
}

/// The unconnected items that name one of `nets` as `[<net>]` in an item's
/// description, counted.
pub(crate) fn unconnected_on(drc: &crate::tools::cli::DrcReport, nets: &[String]) -> usize {
    let Some(open) = &drc.unconnected_items else {
        return 0;
    };
    let tags: Vec<String> = nets.iter().map(|net| format!("[{net}]")).collect();
    open.iter()
        .filter(|violation| {
            violation.items.iter().any(|item| {
                tags.iter()
                    .any(|tag| item.description.contains(tag.as_str()))
            })
        })
        .count()
}

#[cfg(test)]
#[path = "route_judge_tests.rs"]
pub(crate) mod tests;
