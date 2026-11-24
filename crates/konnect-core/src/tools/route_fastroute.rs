//! `route_specctra_dsn` with the fastroute engine: a timed best-of-N search over
//! DSN variants, each run judged by Konnect itself, the best kept by absolute gates
//! (docs/ROUTE_SEARCH_DESIGN.md). The Freerouting engine stays in `integration.rs`,
//! unchanged.

use crate::fastroute::{self, Binary, Located, PassError};
use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::{CallToolResult, ToolContent};
pub(crate) use crate::pour_prune::PourPrune;
pub(crate) use crate::pour_rank::RankChoice;
use crate::pour_rank::{RankMode, RankSource};
pub(crate) use crate::pour_stitch::ReferenceStitch;
use crate::route_judge::{self, Baseline, JudgeContext, Judged, Refusal};
use crate::route_search::{
    self, BoxFuture, Effort, EffortSource, RouteFailure, Routed, RunRecord, Runner, SearchOutcome,
    SearchPlan, Shape,
};
use crate::specctra::RoutingHints;
use crate::tools::integration::{
    completion_report, width_fallback_argument, width_report, WidthFallback, WidthOutcome,
};
use crate::tools::route_lengths::{self, LengthPlan};
use crate::tools::route_pairs;
use crate::tools::{get_path, ToolContext};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Which engine a call routes with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Engine {
    Fastroute,
    Freerouting,
}

/// Arguments only the Freerouting engine reads.
pub(crate) const FREEROUTING_ONLY: &[&str] = &[
    "jar_path",
    "java_path",
    "max_passes",
    "optimizer_enabled",
    "job_timeout_seconds",
    "overall_timeout_seconds",
];

/// Arguments only the fastroute engine reads.
pub(crate) const FASTROUTE_ONLY: &[&str] = &["effort", "fastroute_path"];

/// What a run folder holds that this code made, and so may remove when it reduces
/// the folder to its record: the variant DSN and the three routing passes' folders
/// (`fastroute::route_held`), and the Tune pass folders
/// ([`fastroute::tune_folder_names`], which [`reduce_run_folders`] treats on their own:
/// L26 keeps the log and the session of a discarded or failed pass). Anything else a
/// run folder holds is left alone.
const RUN_ENTRIES: &[&str] = &["variant.dsn", "full", "fallback", "polish"];

/// The files of a discarded or failed Tune pass that [`reduce_run_folders`] keeps
/// (L26): its log and its session. A session and a log are small.
const KEPT_TUNE_FILES: [&str; 2] = ["fastroute.log", "tune.ses"];

/// The Tune pass folders of a run's record whose pass was `discarded` or `failed`
/// (the rows of `held.tune.rows`: `variant`, `round`, `outcome`).
fn tune_folders_to_keep(record: &RunRecord) -> Vec<String> {
    let Some(routed) = &record.routed else {
        return Vec::new();
    };
    routed
        .held
        .tune
        .rows
        .iter()
        .filter(|row| matches!(row["outcome"].as_str(), Some("discarded" | "failed")))
        .filter_map(|row| {
            let variant =
                serde_json::from_value::<fastroute::TuneVariant>(row["variant"].clone()).ok()?;
            let round = usize::try_from(row["round"].as_u64()?).ok()?;
            Some(variant.folder_name(round))
        })
        .collect()
}

/// The run index of a judge folder name this code made: `run-<i>`,
/// `run-<i>-untuned` or `run-<i>-pour-safe` (L23), `run-<i>-pruned` or `run-<i>-prune`
/// (Row ZR, ZR0b: the full judge of the pruned session and the prune's checks).
fn judge_folder_index(name: &str) -> Option<usize> {
    let rest = name.strip_prefix("run-")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let suffix = &rest[digits.len()..];
    matches!(
        suffix,
        "" | "-untuned" | "-pour-safe" | "-pruned" | "-prune" | "-stitched"
    )
    .then(|| digits.parse().ok())
    .flatten()
}

/// A refusal of one argument, worded as the other refusals of this tool are.
fn refusal(field: &str, reason: String) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::InvalidArgument {
            field: field.to_string(),
            reason: reason.clone(),
        },
        format!("Argument '{field}' is invalid: {reason}"),
    )
}

/// Whether the caller gave the argument (a JSON `null` is as good as absent).
fn given(args: &Value, key: &str) -> bool {
    !args[key].is_null()
}

/// The engine of a call, `engine` absent meaning fastroute (the default since this
/// change). Refused with `invalid_argument` naming the argument:
/// - `engine` that is not `"fastroute"` or `"freerouting"`;
/// - with fastroute: any [`FREEROUTING_ONLY`] argument, `allow_neckdown: true`
///   (`false` is accepted: it is what fastroute runs with), and
///   `width_policy: "retry"` (`"hold"` is accepted: every fastroute run holds);
///   the message says these belong to `engine: "freerouting"`;
/// - with freerouting: any [`FASTROUTE_ONLY`] argument; the message says these
///   belong to the fastroute engine.
pub(crate) fn route_engine(args: &Value) -> std::result::Result<Engine, CallToolResult> {
    let engine = match &args["engine"] {
        Value::Null => Engine::Fastroute,
        Value::String(name) if name == "fastroute" => Engine::Fastroute,
        Value::String(name) if name == "freerouting" => Engine::Freerouting,
        other => {
            return Err(refusal(
                "engine",
                format!("must be \"fastroute\" or \"freerouting\", not {other}"),
            ))
        }
    };
    match engine {
        Engine::Fastroute => {
            // (argument, how the message names it)
            let mut foreign: Vec<(&str, String)> = FREEROUTING_ONLY
                .iter()
                .filter(|name| given(args, name))
                .map(|name| (*name, (*name).to_string()))
                .collect();
            if args["allow_neckdown"] == true {
                foreign.push(("allow_neckdown", "allow_neckdown true".to_string()));
            }
            if args["width_policy"] == "retry" {
                foreign.push(("width_policy", "width_policy \"retry\"".to_string()));
            }
            if let Some((field, _)) = foreign.first() {
                let names: Vec<&str> = foreign.iter().map(|(_, name)| name.as_str()).collect();
                return Err(refusal(
                    field,
                    format!(
                        "{} belong(s) to engine \"freerouting\", and this call routes with engine \"fastroute\" (the default: a timed best-of-N search judged by Konnect). Pass engine: \"freerouting\" to route with Freerouting, or drop the argument(s).",
                        names.join(", ")
                    ),
                ));
            }
        }
        Engine::Freerouting => {
            let foreign: Vec<&str> = FASTROUTE_ONLY
                .iter()
                .copied()
                .filter(|name| given(args, name))
                .collect();
            if let Some(field) = foreign.first() {
                return Err(refusal(
                    field,
                    format!(
                        "{} belong(s) to the fastroute engine (the default), and this call routes with engine \"freerouting\", which does not read them. Drop the argument(s), or drop engine: \"freerouting\".",
                        foreign.join(", ")
                    ),
                ));
            }
        }
    }
    Ok(engine)
}

/// The user config values this engine reads: `routing.effort` and
/// `routing.fastroute_path` (and `routing.rank_key`, read apart by [`call_rank_at`] on
/// every call of the route tool), from the user config file `get_effective_config` and
/// `save_user_config` use (`tools::config`). A missing file or key is `None`; a file
/// that does not parse is an error (the call is refused, as `load_user_config`
/// refuses it).
pub(crate) fn user_routing_config() -> Result<(Option<Value>, Option<PathBuf>)> {
    routing_config_at(&crate::tools::config::user_config_path())
}

/// [`user_routing_config`] for the config file at `path`.
fn routing_config_at(path: &Path) -> Result<(Option<Value>, Option<PathBuf>)> {
    let Some(routing) = routing_object_at(path)? else {
        return Ok((None, None));
    };
    let effort = routing
        .get("effort")
        .filter(|value| !value.is_null())
        .cloned();
    let fastroute_path = match routing.get("fastroute_path") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if !text.is_empty() => Some(PathBuf::from(text)),
        Some(_) => anyhow::bail!(
            "the user config at {}: `routing.fastroute_path` must be a non-empty string",
            path.display()
        ),
    };
    Ok((effort, fastroute_path))
}

/// The `routing` object of the config file at `path`: `None` for a missing file or key
/// (or a JSON null); an error for a file that does not parse or a `routing` that is not
/// an object. What each key holds is judged by the reader that reads it.
fn routing_object_at(path: &Path) -> Result<Option<serde_json::Map<String, Value>>> {
    let object = crate::tools::config::read_user_config_object(path)
        .map_err(|reason| anyhow::anyhow!("the user config at {} {reason}", path.display()))?;
    let Some(mut object) = object else {
        return Ok(None);
    };
    match object["routing"].take() {
        Value::Null => Ok(None),
        Value::Object(routing) => Ok(Some(routing)),
        _ => anyhow::bail!(
            "the user config at {}: `routing` must be an object",
            path.display()
        ),
    }
}

/// Row ZR, ZR5 part 2 (PK9): `routing.rank_key` of the config file at `path`, as
/// written (`None` when it is missing or null); [`routing_object_at`]'s errors only. A
/// bad value of another key (`routing.fastroute_path`, say) is not this reader's: it
/// refuses only the calls that read that key.
fn rank_key_at(path: &Path) -> Result<Option<Value>> {
    Ok(routing_object_at(path)?
        .and_then(|mut routing| routing.remove("rank_key"))
        .filter(|value| !value.is_null()))
}

/// Row ZR, ZR5 part 2 (PK9, PK13): the mode of a routing call and where it came from.
/// `Fixed(mode)` is that mode and reads nothing. `UserConfig` reads the config file at
/// `path` (the route tool passes the user config), whatever the call's arguments: a file
/// that does not parse, a `routing` that is not an object, or a `routing.rank_key` that
/// names no mode, is the call's refusal (`invalid_configuration`, naming `path`), and
/// nothing is guessed. A bad value of another key does not refuse here ([`rank_key_at`]).
pub(crate) fn call_rank_at(
    choice: RankChoice,
    path: &Path,
) -> std::result::Result<(RankMode, RankSource), CallToolResult> {
    let config = match choice {
        RankChoice::Fixed(mode) => return Ok((mode, RankSource::Fixed)),
        RankChoice::UserConfig => match rank_key_at(path) {
            Ok(rank_key) => rank_key,
            Err(error) => return Err(config_refusal_at(&error, path)),
        },
    };
    crate::pour_rank::resolve_rank_mode(config.as_ref())
        .map_err(|message| config_refusal_at(&anyhow::anyhow!("{message}"), path))
}

/// What to do when no fastroute binary can be used: where the default place is, the
/// three ways to name another path, and that `engine: "freerouting"` routes with
/// Freerouting instead. Nothing is downloaded or installed.
pub(crate) fn setup_guidance(located: &Located) -> Value {
    let default_place = match located {
        Located::NotConfigured { default_path } => default_path.display().to_string(),
        _ => format!("$HOME/{}", fastroute::DEFAULT_PATH),
    };
    let problem = match located {
        Located::Found(binary) => format!("fastroute is usable at {}.", binary.path.display()),
        Located::NotConfigured { default_path } => format!(
            "No fastroute binary is at the default place {} and none was named.",
            default_path.display()
        ),
        Located::Refused {
            path,
            source,
            reason,
        } => format!(
            "The fastroute binary named by the {source} ({}) cannot be used: {reason}",
            path.display()
        ),
        Located::Unsupported { reason } => {
            format!("The fastroute engine cannot run on this system: {reason}")
        }
    };
    json!({
        "problem": problem,
        "default_place": default_place,
        "name_a_binary": [
            "the fastroute_path argument of route_specctra_dsn",
            format!("the environment variable {} of the Konnect server", fastroute::PATH_VARIABLE),
            "the user config key routing.fastroute_path (save_user_config)",
            format!("a {} file next to the konnect executable of a 1 Kopper package (cargo xtask kopper-install)", fastroute::BUNDLED_NAME),
        ],
        "steps": [
            format!("Put the fastroute candidate build ({}) at {default_place} and make it executable.", fastroute::CANDIDATE_NAME),
            "Or name its path in one of the ways above; the first that names a path is the one used, and a later one is not tried when it is refused.",
            "Konnect checks that the binary is the candidate build before it routes; any other build is refused.",
            "Nothing is downloaded or installed for you.",
        ],
        "alternative": "engine: \"freerouting\" routes with the local Freerouting JAR instead (check_freerouting lists what it needs).",
    })
}

// ─── The call ────────────────────────────────────────────────────────────────

/// What the arguments of a call settle before anything is located or routed.
pub(crate) struct Call {
    dsn: PathBuf,
    ses_output: PathBuf,
    /// `<ses_output with extension "search">`, created by [`prepare_call`].
    folder: PathBuf,
    effort: (Effort, EffortSource),
    fallback: WidthFallback,
    /// Row ZR, ZR5 part 2: which key ranks, and where that came from. [`prepare_call`]
    /// gives today's key, fixed; [`route_with_fastroute`] sets the mode of its
    /// [`RankChoice`] ([`call_rank_at`]).
    rank: (RankMode, RankSource),
    /// Row ZR, ZR6 (S13): whether the judge runs the reference round and the winner's
    /// session gets its vias. [`prepare_call`] gives `Off`; [`route_with_fastroute`] sets
    /// its caller's switch.
    stitch: ReferenceStitch,
}

/// The refusal for a user config that cannot be read for routing.
fn config_refusal(error: &anyhow::Error) -> CallToolResult {
    config_refusal_at(error, &crate::tools::config::user_config_path())
}

/// [`config_refusal`] for the config file at `path`, the one that was read.
fn config_refusal_at(error: &anyhow::Error, path: &Path) -> CallToolResult {
    let reason = format!("{error:#}");
    CallToolResult::error_kind(
        ToolErrorKind::InvalidConfiguration {
            path: path.display().to_string(),
            reason: reason.clone(),
        },
        format!(
            "The user configuration cannot be used for routing ({reason}). Konnect did not substitute defaults and routed nothing. Repair or move the file: route_specctra_dsn reads it on every fastroute call, for routing.rank_key, whatever arguments the call has."
        ),
    )
}

/// Steps 1 and 2 of [`route_with_fastroute`]: the effort, the DSN, the output and
/// the search folder. Everything that can be refused is refused here, before the
/// folder is created; a returned `Ok(Ok(call))` has created it (empty).
pub(crate) fn prepare_call(args: &Value) -> Result<std::result::Result<Call, CallToolResult>> {
    // The user config is read only when the call leaves a value to it.
    let effort = {
        let argument = Some(&args["effort"]).filter(|value| !value.is_null());
        let config = if argument.is_some() {
            None
        } else {
            match user_routing_config() {
                Ok((effort, _)) => effort,
                Err(error) => return Ok(Err(config_refusal(&error))),
            }
        };
        match route_search::resolve_effort(argument, config.as_ref()) {
            Ok(found) => found,
            Err(message) => return Ok(Err(refusal("effort", message))),
        }
    };
    let fallback = match width_fallback_argument(args) {
        Ok(fallback) => fallback,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let dsn = get_path(args, "dsn_path")?;
    let ses_output = get_path(args, "ses_output_path")?;
    if let Err(error) = crate::freerouting_mcp::check_dsn_input(&dsn) {
        return Ok(Err(refusal("dsn_path", format!("{error:#}"))));
    }
    if !ses_output
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ses"))
    {
        return Ok(Err(refusal(
            "ses_output_path",
            "SES output must have the .ses extension".to_string(),
        )));
    }
    let conflict = |path: &Path, message: String| {
        CallToolResult::error_kind(
            ToolErrorKind::Conflict {
                paths: vec![path.display().to_string()],
            },
            message,
        )
    };
    if ses_output.symlink_metadata().is_ok() {
        return Ok(Err(conflict(
            &ses_output,
            format!(
                "SES output already exists: {} (an existing output is never replaced)",
                ses_output.display()
            ),
        )));
    }
    let folder = ses_output.with_extension("search");
    let exists_message = |folder: &Path| {
        format!(
            "the search folder already exists: {} (a call keeps its runs' files there and never replaces one; move it away, or choose another ses_output_path)",
            folder.display()
        )
    };
    if folder.symlink_metadata().is_ok() {
        return Ok(Err(conflict(&folder, exists_message(&folder))));
    }
    if let Some(parent) = folder
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            anyhow::anyhow!("create SES output directory {}: {error}", parent.display())
        })?;
    }
    // `create_dir` refuses a folder that is there, in one step.
    match std::fs::create_dir(&folder) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Ok(Err(conflict(&folder, exists_message(&folder))));
        }
        Err(error) => {
            anyhow::bail!("create the search folder {}: {error}", folder.display())
        }
    }
    Ok(Ok(Call {
        dsn,
        ses_output,
        folder,
        effort,
        fallback,
        rank: (RankMode::Routing, RankSource::Fixed),
        stitch: ReferenceStitch::Off,
    }))
}

/// The fastroute engine of `route_specctra_dsn`:
/// 1. effort: `route_search::resolve_effort(args["effort"], user config)`; a bad
///    value is `invalid_argument` on `effort`.
/// 2. `dsn_path` through `freerouting_mcp::check_dsn_input` (`invalid_argument` on
///    `dsn_path`); `ses_output_path` a new `.ses` file; the search folder
///    `<ses_output_path with extension "search">` must not exist (`conflict`).
///    The folder is created now; on any later failure it is renamed aside
///    (`<folder>.failed-N`, the first free N from 1), never deleted with what it
///    holds, so the same call can be made again.
/// 3. `fastroute::locate(args["fastroute_path"], env KONNECT_FASTROUTE, user config,
///    the packaged binary next to this executable, HOME, <folder>/probe)`; anything but `Found` is an error result carrying
///    [`setup_guidance`]. Nothing has been routed.
/// 4. `route_judge::prepare(dsn, <folder>/judge, ctx.config.kicad_cli)`; a refusal is
///    an error result carrying its reason and next step.
/// 5. The DSN text, `route_search::shapes`, the manifest's hints
///    (`specctra::manifest_routing_hints`), the amps file
///    (`fastroute::amps_file_text`, written to `<folder>/amps.txt`), the pour nets
///    (`fastroute::uncounted_file_text`, written to `<folder>/uncounted.txt` and sent as
///    `--x.gs_uncounted=<file>` on every routing pass of every run, Row GS), the design intent's
///    differential pairs (`route_pairs::plan`, written to `<folder>/pairs.txt` and sent
///    as `--pairs=<file>` on every pass of every run), Row ZR's zone plan
///    ([`zone_plan_file`], written to `<folder>/zones.txt` when it models a pour and sent
///    as `--x.zones=<file>`, with `--x.zones_cut` right after it, on the routing passes of
///    a variant with planes; the result's `pours`), Row LT's length plan
///    (`route_lengths::plan`, after the pairs plan and before the search folder holds
///    anything: a group or skew that cannot be applied is a refusal, `invalid_configuration`,
///    "Nothing was routed."; `<folder>/tune.txt` when there is a group, and the Tune
///    pass after the hold sequence of every run), `width_fallback`
///    (default floor, as `integration::width_arguments` reads it), the plan
///    (`launch_order`, `lanes(available cores, effort)`, `HANG_CAP`).
/// 6. `route_search::search` with a [`FastrouteRunner`].
/// 7. No winner: an error result with every run's record and the baseline's state;
///    nothing is written to `ses_output_path`.
/// 8. The winner's session to `ses_output_path` (`konnect_sexp::write_new_atomic`),
///    `<folder>/search.json` with the whole outcome, then [`reduce_run_folders`].
/// 9. The result ([`result_json`]).
///
/// `prune` (Row ZR, ZR0b, P12): with `Winner`, the winner's redundant ground vias are
/// removed after step 6 (`pour_prune::prune_winner`), so steps 8 and 9 write and report
/// the pruned session when it was kept; with `Off` nothing of that runs.
///
/// `rank` (Row ZR, ZR5 part 2, PK9, PK13): where the call's mode comes from, settled
/// before step 1 ([`call_rank_at`] of the user config file): `UserConfig` reads the user
/// config on every call, whatever the arguments, and a config that cannot be used
/// refuses the call before anything is made; `Fixed` reads nothing. The mode goes into
/// the judge context (`JudgeContext::rank`), and the result's `search.rank` reports it
/// (`pour_rank::rank_report`).
///
/// `stitch` (Row ZR, ZR6, S13): with `On` every judged run gets the reference round
/// (`JudgeContext::reference_stitch`) and, after the prune, the winner's kept reference
/// vias are written into its session (`pour_stitch::deliver_winner`,
/// `search.reference_stitch`); with `Off` nothing of that runs.
pub(crate) async fn route_with_fastroute(
    args: &Value,
    ctx: Arc<ToolContext>,
    prune: PourPrune,
    rank: RankChoice,
    stitch: ReferenceStitch,
) -> Result<CallToolResult> {
    let mut call = match prepare_ranked_call(args, rank, &crate::tools::config::user_config_path())?
    {
        Ok(call) => call,
        Err(refusal) => return Ok(refusal),
    };
    call.stitch = stitch;
    let located = match locate_binary(args, &call).await {
        Ok(Ok(located)) => located,
        Ok(Err(refusal)) => return finish(&call.folder, Ok(refusal)),
        Err(error) => return finish(&call.folder, Err(error)),
    };
    route_located(args, ctx, call, located, prune).await
}

/// Row ZR, ZR5 part 2: the call's mode ([`call_rank_at`] of the config file at
/// `config`), then [`prepare_call`], and the call given that mode. A refusal of the mode
/// comes first, before the search folder is made.
pub(crate) fn prepare_ranked_call(
    args: &Value,
    rank: RankChoice,
    config: &Path,
) -> Result<std::result::Result<Call, CallToolResult>> {
    let rank = match call_rank_at(rank, config) {
        Ok(rank) => rank,
        Err(refusal) => return Ok(Err(refusal)),
    };
    Ok(prepare_call(args)?.map(|mut call| {
        call.rank = rank;
        call
    }))
}

/// Step 3: where the binary is, and whether it is the candidate build. The
/// user config is read only when neither the argument nor the environment
/// names a path; a config that cannot be read is a refusal.
async fn locate_binary(
    args: &Value,
    call: &Call,
) -> Result<std::result::Result<Located, CallToolResult>> {
    let argument = args["fastroute_path"].as_str().map(PathBuf::from);
    let environment = std::env::var(fastroute::PATH_VARIABLE)
        .ok()
        .filter(|value| !value.is_empty());
    let config = if argument.is_some() || environment.is_some() {
        None
    } else {
        match user_routing_config() {
            Ok((_, path)) => path,
            Err(error) => return Ok(Err(config_refusal(&error))),
        }
    };
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| fastroute::bundled_path(&exe));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let scratch = call.folder.join("probe");
    std::fs::create_dir_all(&scratch)
        .map_err(|error| anyhow::anyhow!("create {}: {error}", scratch.display()))?;
    let located = fastroute::locate(
        argument.as_deref(),
        environment.as_deref(),
        config.as_deref(),
        bundled.as_deref(),
        home.as_deref(),
        &scratch,
    )
    .await;
    // The probe's own folder, made above: what it wrote has served.
    let _ = remove_tree(&scratch);
    Ok(Ok(located))
}

/// Steps 3 to 9 once the binary has been looked for: the part of a call that
/// every test below can drive without a fastroute installed.
pub(crate) async fn route_located(
    args: &Value,
    ctx: Arc<ToolContext>,
    call: Call,
    located: Located,
    prune: PourPrune,
) -> Result<CallToolResult> {
    let result = match located {
        Located::Found(binary) => search_and_write(args, ctx, &call, binary, prune).await,
        other => Ok(unavailable_result(&other)),
    };
    finish(&call.folder, result)
}

/// The end of a call. A success keeps its search folder. Anything else sets the
/// folder aside ([`set_aside`]) and says where it went, so the same call can be
/// made again.
fn finish(folder: &Path, result: Result<CallToolResult>) -> Result<CallToolResult> {
    match result {
        Ok(result) if !result.is_error => Ok(result),
        Ok(result) => {
            let note = set_aside(folder);
            Ok(with_fields(result, vec![("search_folder", json!(note))]))
        }
        Err(error) => {
            let note = set_aside(folder);
            Err(anyhow::anyhow!("{error:#} ({note})"))
        }
    }
}

/// The most failed calls' folders kept beside one output.
const MAX_FAILED_KEPT: u32 = 99;

/// Takes a failed call's search folder out of the way and says what was done. A
/// folder that holds no file (empty folders only) is removed, with those folders
/// and nothing else. One that holds files is renamed to `<folder>.failed-N`, the
/// first N from 1 whose name is free (anything by that name, a broken link too,
/// keeps it). It is never deleted with what it holds: when no rename is possible
/// it stays, and the note says so.
fn set_aside(folder: &Path) -> String {
    remove_empty_folders(folder);
    if folder.symlink_metadata().is_err() {
        return "the failed call left no file of its own".to_string();
    }
    for n in 1..=MAX_FAILED_KEPT {
        let mut name = folder.as_os_str().to_os_string();
        name.push(format!(".failed-{n}"));
        let aside = PathBuf::from(name);
        if aside.symlink_metadata().is_ok() {
            continue;
        }
        return match std::fs::rename(folder, &aside) {
            Ok(()) => format!("the failed call's files are kept in {}", aside.display()),
            Err(error) => format!(
                "the failed call's files stay in {} ({error}); move that folder away before calling again",
                folder.display()
            ),
        };
    }
    format!(
        "the failed call's files stay in {}: every name {}.failed-1 to .failed-{MAX_FAILED_KEPT} is taken; move that folder away before calling again",
        folder.display(),
        folder.display()
    )
}

/// Removes `path` and every folder below it that holds nothing. Files, links
/// and the folders that hold them are left; a link is never followed.
fn remove_empty_folders(path: &Path) {
    let Ok(meta) = path.symlink_metadata() else {
        return;
    };
    if !meta.is_dir() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            remove_empty_folders(&entry.path());
        }
    }
    // `remove_dir` removes only an empty folder.
    let _ = std::fs::remove_dir(path);
}

/// `result` with `fields` added to its JSON body (an error result built with
/// `error_kind` has one). A body that is not a JSON object is left as it is.
fn with_fields(mut result: CallToolResult, fields: Vec<(&str, Value)>) -> CallToolResult {
    if let Some(ToolContent::Text { text }) = result.content.first_mut() {
        if let Ok(Value::Object(mut body)) = serde_json::from_str::<Value>(text) {
            for (key, value) in fields {
                body.insert(key.to_string(), value);
            }
            *text = Value::Object(body).to_string();
        }
    }
    result
}

/// The error result for a binary that cannot be used, with the setup steps.
/// `plan_convergent_placement` refuses with it too.
pub(crate) fn unavailable_result(located: &Located) -> CallToolResult {
    let (kind, message) = match located {
        Located::NotConfigured { default_path } => (
            ToolErrorKind::FileNotFound {
                path: default_path.display().to_string(),
            },
            format!(
                "No fastroute binary is installed at {} and none was named. Nothing was routed.",
                default_path.display()
            ),
        ),
        Located::Refused {
            path,
            source,
            reason,
        } => (
            if path.exists() {
                ToolErrorKind::HandlerError {
                    reason: reason.clone(),
                }
            } else {
                ToolErrorKind::FileNotFound {
                    path: path.display().to_string(),
                }
            },
            format!(
                "The fastroute binary named by the {source} ({}) cannot be used: {reason}. Nothing was routed.",
                path.display()
            ),
        ),
        Located::Unsupported { reason } => (
            ToolErrorKind::HandlerError {
                reason: reason.clone(),
            },
            format!(
                "The fastroute engine cannot run on this system: {reason}. Nothing was routed."
            ),
        ),
        Located::Found(binary) => (
            ToolErrorKind::HandlerError {
                reason: "a usable binary was treated as unavailable".to_string(),
            },
            format!(
                "fastroute at {} was found; this is an internal error.",
                binary.path.display()
            ),
        ),
    };
    with_fields(
        CallToolResult::error_kind(kind, message),
        vec![("setup", setup_guidance(located))],
    )
}

/// The error result for a judge that cannot run: its reason and the step that
/// fixes it.
fn preflight_refusal(refusal: &Refusal) -> CallToolResult {
    let reason = refusal.reason.trim_end_matches('.');
    with_fields(
        CallToolResult::error_kind(
            ToolErrorKind::HandlerError {
                reason: refusal.reason.clone(),
            },
            format!(
                "The best-of-N judge cannot run: {reason}. Nothing was routed. Next step: {}",
                refusal.next_step
            ),
        ),
        vec![
            ("reason", json!(refusal.reason)),
            ("next_step", json!(refusal.next_step)),
        ],
    )
}

/// Every shape's variant must be makeable from this DSN (seed 1 reorders, so it
/// exercises the placement too): a DSN the variants cannot be made from would
/// otherwise fail every run until the budget.
fn preflight_variants(dsn_text: &str, shapes: &[Shape]) -> Result<()> {
    for shape in shapes {
        route_search::variant_text(dsn_text, *shape, 1)
            .map_err(|error| anyhow::anyhow!("{} variant: {error:#}", shape_name(*shape)))?;
    }
    Ok(())
}

/// Steps 4 to 9: judge preflight, the DSN's variants, the search, the prune of the
/// winner when `prune` asks for it, and what follows from the outcome.
async fn search_and_write(
    args: &Value,
    ctx: Arc<ToolContext>,
    call: &Call,
    binary: Binary,
    prune: PourPrune,
) -> Result<CallToolResult> {
    let judge_root = call.folder.join("judge");
    let mut judge = match route_judge::prepare(&call.dsn, &judge_root, &ctx.config.kicad_cli) {
        Ok(judge) => judge,
        Err(refused) => return Ok(preflight_refusal(&refused)),
    };
    let dsn_text = crate::freerouting_mcp::read_dsn(&call.dsn)?;
    // The pairs the design intent declares, then its length groups and pair skews
    // (Row LT), both resolved against this DSN's nets. A group that cannot be
    // applied refuses the call here, before the search folder holds anything: a
    // pair left out still routes as two nets, a group left out is a requirement
    // silently not met (L4).
    let pairs_plan = route_pairs::plan(&judge.board, &dsn_text);
    let intent_path = crate::tools::design_intent::design_intent_path(
        judge.board.parent().unwrap_or_else(|| Path::new(".")),
    );
    let lengths_plan =
        match route_lengths::plan(&judge.board, &judge.board_text, &dsn_text, &pairs_plan) {
            Ok(plan) => plan,
            Err(refusal) => return Ok(route_lengths::refusal_result(&refusal, &intent_path)),
        };
    // The Tune pass lists every net class of the DSN so that the autorouter routes
    // nothing (L6); a DSN whose classes cannot be read cannot be tuned safely.
    let classes = if lengths_plan.groups.is_empty() {
        Vec::new()
    } else {
        match crate::specctra::dsn_class_names(&dsn_text) {
            Ok(classes) => classes,
            Err(error) => {
                return Ok(route_lengths::refusal_result(
                    &route_lengths::LengthRefusal {
                        reason: format!(
                            "The DSN's net classes could not be read ({error:#}), so the design intent's length groups cannot be tuned."
                        ),
                        next_step: "Export the DSN again with export_specctra_dsn, then call route_specctra_dsn again.".to_string(),
                    },
                    &intent_path,
                ));
            }
        }
    };
    std::fs::create_dir_all(&judge_root)
        .map_err(|error| anyhow::anyhow!("create {}: {error}", judge_root.display()))?;
    // The judge's KiCad runs on a private copy of the KiCad settings, so that
    // no run touches the user's own.
    let kicad_config = match route_judge::private_kicad_config(&call.folder.join("kicad-config")) {
        Ok(config) => config,
        Err(error) => {
            return Ok(CallToolResult::error_kind(
                ToolErrorKind::HandlerError {
                    reason: format!("{error:#}"),
                },
                format!(
                    "The private KiCad configuration the judge runs on could not be made ({error:#}). Nothing was routed."
                ),
            ));
        }
    };
    let kicad_config_home = kicad_config.home.clone();
    judge.kicad_config_home = Some(kicad_config_home.clone());
    // Row ZR, ZR5 part 2: the call's mode, kept in one place.
    judge.rank = call.rank.0;
    // Row ZR, ZR6 (S13): the reference round, on for the route tool only.
    judge.reference_stitch = call.stitch;
    let shapes = match route_search::shapes(&dsn_text)
        .and_then(|shapes| preflight_variants(&dsn_text, &shapes).map(|()| shapes))
    {
        Ok(shapes) => shapes,
        Err(error) => {
            return Ok(CallToolResult::error_kind(
                ToolErrorKind::HandlerError {
                    reason: format!("{error:#}"),
                },
                format!(
                    "The DSN cannot be turned into the variants the search routes ({error:#}). Nothing was routed. Export it again with export_specctra_dsn, or route it with engine: \"freerouting\"."
                ),
            ));
        }
    };
    let pairs = route_pairs::write_file(&call.folder, &pairs_plan)?;
    // Row ZR, ZR1: each pour's fill settings for the engine's model, after the pairs
    // plan. Recorded only: it changes no routing, and a plan that cannot be made sends
    // no zones file and says why in `pours`.
    let (zones, pours) = zone_plan_file(&call.folder, &mut judge, &dsn_text)?;
    let tune = route_lengths::write_file(&call.folder, &lengths_plan)?.map(|file| TuneFiles {
        file,
        classes,
        nets: lengths_plan.tune_nets(),
        layer_heights: lengths_plan.heights.switch_value(),
        plan: Arc::new(lengths_plan.clone()),
    });
    judge.lengths = judge_checks(&lengths_plan);
    let hints = crate::specctra::manifest_routing_hints(&call.dsn);
    let amps = match fastroute::amps_file_text(&judge.manifest) {
        Some(text) => {
            let path = call.folder.join("amps.txt");
            std::fs::write(&path, text)
                .map_err(|error| anyhow::anyhow!("write {}: {error}", path.display()))?;
            Some(path)
        }
        None => None,
    };
    // Row GS, G3: the pour nets, which the group search leaves out of its cost as the
    // judge leaves them out of its measures.
    let uncounted = match fastroute::uncounted_file_text(&judge.manifest) {
        Some(text) => {
            let path = call.folder.join("uncounted.txt");
            std::fs::write(&path, text)
                .map_err(|error| anyhow::anyhow!("write {}: {error}", path.display()))?;
            Some(path)
        }
        None => None,
    };
    let limits = call.effort.0.limits();
    let cores = std::thread::available_parallelism()
        .map(|cores| cores.get())
        .unwrap_or(1);
    let plan = SearchPlan {
        limits,
        lanes: route_search::lanes(cores, call.effort.0),
        order: route_search::launch_order(&shapes, limits),
        hang_cap: route_search::HANG_CAP,
    };
    let judge = Arc::new(judge);
    let tools = Arc::clone(&ctx);
    let runner = Arc::new(FastrouteRunner {
        binary: binary.clone(),
        dsn_text: Arc::new(dsn_text),
        hints,
        amps,
        uncounted,
        zones,
        pairs,
        routing_heights: routing_layer_heights(&lengths_plan, &pairs_plan),
        tune,
        fallback: call.fallback,
        folder: call.folder.clone(),
        judge: Arc::clone(&judge),
        tools: ctx,
    });
    let mut outcome = route_search::search(&plan, runner).await;
    // Row ZR, ZR0b (P1): once, on the winner, before anything is written.
    if prune == PourPrune::Winner {
        crate::pour_prune::prune_winner(&judge, Arc::clone(&tools), &mut outcome).await;
    }
    // Row ZR, ZR6 (S10, S11): after the prune, the winner's reference vias go into its
    // session.
    if call.stitch == ReferenceStitch::On {
        outcome.reference_stitch =
            Some(crate::pour_stitch::deliver_winner_registry(&judge, tools, &mut outcome).await);
    }
    let result = conclude(
        args,
        call,
        &binary,
        &plan,
        &shapes,
        &outcome,
        &kicad_config,
        &pairs_plan,
        &lengths_plan,
    )?;
    Ok(with_fields(result, vec![("pours", pours)]))
}

/// Row ZR, ZR1: the call's zone plan (`zone_fill::zone_plan` of the saved board, its
/// manifest, the DSN, and the project and custom rules saved beside the board), its
/// zones file written as `<folder>/zones.txt` when the plan models a pour (the file
/// the routing passes of a variant with planes are sent as `--x.zones`, with
/// `--x.zones_cut`), and the
/// result's `pours` field (`zone_fill::ZonePlan::result_json`). When the file is
/// written, the judge gets the plan's pad names. A plan that cannot be made, or a
/// project or rules file that exists and cannot be read, gives no file and
/// `pours: {"error": …}`; only a zones file that cannot be written is an error.
fn zone_plan_file(
    folder: &Path,
    judge: &mut JudgeContext,
    dsn_text: &str,
) -> Result<(Option<PathBuf>, Value)> {
    let read = |extension: &str| -> std::result::Result<Option<String>, String> {
        let path = judge.board.with_extension(extension);
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("read {}: {error}", path.display())),
        }
    };
    let plan = read("kicad_pro").and_then(|project| {
        let rules = read("kicad_dru")?;
        crate::zone_fill::zone_plan(
            &judge.board_text,
            Some(&judge.manifest),
            dsn_text,
            project.as_deref(),
            rules.as_deref(),
        )
        .map_err(|error| format!("{error:#}"))
    });
    let plan = match plan {
        Ok(plan) => plan,
        Err(reason) => return Ok((None, json!({ "error": reason }))),
    };
    let Some(text) = crate::zone_fill::file_text(&plan) else {
        return Ok((None, plan.result_json(None)));
    };
    let path = folder.join(crate::zone_fill::ZONES_FILE);
    std::fs::write(&path, text)
        .map_err(|error| anyhow::anyhow!("write {}: {error}", path.display()))?;
    judge.pad_names = Some(Arc::new(plan.names.clone()));
    Ok((Some(path.clone()), plan.result_json(Some(&path))))
}

/// What the judge checks for this call (L8, L9): the plan's groups and pair skews, or
/// `None` when it has none and the board has no length or skew rules of its own, so
/// that the judge runs no probe and G4 has no intent lines (L18). A board with such
/// rules always gets checks (its `board_rules` flag), even with no group or pair.
fn judge_checks(plan: &LengthPlan) -> Option<Arc<crate::length_probe::LengthChecks>> {
    (!plan.is_empty() || !plan.kicad_rules.is_empty()).then(|| Arc::new(plan.checks()))
}

/// The `--layer-heights` value of the routing passes (L7): only when the call has a
/// resolved pair, because fastroute reads the heights only when it measures a net
/// (a pair's skew); a call without a pair keeps DP's exact command lines. The Tune
/// pass takes the plan's heights whenever there are any ([`TuneFiles`]).
fn routing_layer_heights(plan: &LengthPlan, pairs: &route_pairs::PairsPlan) -> Option<String> {
    if pairs.resolved.is_empty() {
        None
    } else {
        plan.heights.switch_value()
    }
}

/// Steps 7 to 9 once the search has run: `search.json`, then either the no-winner
/// error ([`no_winner_result`]) or the winner's session at `ses_output_path`, the
/// reduced run folders and the result ([`result_json`]). `pairs_plan` is the call's
/// differential pairs plan and `lengths_plan` its length plan (Row LT); both are
/// reported in both results.
#[allow(clippy::too_many_arguments)] // the parts of one finished search, each read once
pub(crate) fn conclude(
    args: &Value,
    call: &Call,
    binary: &Binary,
    plan: &SearchPlan,
    shapes: &[Shape],
    outcome: &SearchOutcome,
    kicad_config: &route_judge::PrivateConfig,
    pairs_plan: &route_pairs::PairsPlan,
    lengths_plan: &LengthPlan,
) -> Result<CallToolResult> {
    // Row ZR, ZR5 part 2 (PK10): which key ranked and which run the other key puts first.
    debug_assert_eq!(
        call.rank.0, outcome.rank,
        "the search ranked by the call's mode"
    );
    let rank = crate::pour_rank::rank_report(
        call.rank.0,
        call.rank.1,
        &outcome.keys,
        &outcome.pour_keys,
        outcome.winner,
    );
    let settings = |search: &mut Value| {
        search["kicad_config_home"] = json!(text(&kicad_config.home));
        if !kicad_config.skipped_links.is_empty() {
            search["kicad_config_skipped_links"] = json!(kicad_config.skipped_links);
        }
        search["rank"] = rank.clone();
    };
    let mut every_run = search_json(plan, shapes, outcome, Listing::Every);
    settings(&mut every_run);
    let search_file = call.folder.join("search.json");
    std::fs::write(
        &search_file,
        serde_json::to_string_pretty(&every_run).unwrap_or_default(),
    )
    .map_err(|error| anyhow::anyhow!("write {}: {error}", search_file.display()))?;
    let Some(winner) = outcome
        .winner
        .and_then(|index| outcome.runs.get(index))
        .filter(|record| record.routed.is_some() && record.judged.is_some())
    else {
        let mut search = search_json(plan, shapes, outcome, Listing::Best);
        settings(&mut search);
        return Ok(no_winner_result(
            call,
            outcome,
            search,
            pairs_plan,
            lengths_plan,
        ));
    };
    let routed = winner.routed.as_ref().expect("filtered above");
    let session = std::fs::read_to_string(&routed.held.ses).map_err(|error| {
        anyhow::anyhow!(
            "read the winning run's session {}: {error}",
            routed.held.ses.display()
        )
    })?;
    konnect_sexp::write_new_atomic(&call.ses_output, &session).map_err(|error| {
        anyhow::anyhow!("create SES output {}: {error}", call.ses_output.display())
    })?;
    let reduced = reduce_run_folders(&call.folder, outcome);
    let mut result = result_json(
        &call.dsn,
        &call.ses_output,
        &call.folder,
        binary,
        call.effort,
        plan,
        shapes,
        outcome,
        pairs_plan,
        lengths_plan,
    );
    settings(&mut result["search"]);
    if let Err(error) = reduced {
        result["search"]["reduce_error"] = json!(format!("{error:#}"));
    }
    carry_arguments(&mut result, args, call.effort);
    Ok(CallToolResult::json(&result))
}

/// Step 7: nothing was routed and judged, so nothing is written to the output.
///
/// The failed call leaves its records, not its boards: every run folder is
/// reduced to its `run.json` (`search.json` was written before this) and a
/// reduce error is reported in `search.reduce_error`, never fatal.
pub(crate) fn no_winner_result(
    call: &Call,
    outcome: &SearchOutcome,
    mut search: Value,
    pairs: &route_pairs::PairsPlan,
    lengths: &LengthPlan,
) -> CallToolResult {
    if let Err(error) = reduce_run_folders(&call.folder, outcome) {
        search["reduce_error"] = json!(format!("{error:#}"));
    }
    let mut counts = std::collections::BTreeMap::new();
    for record in &outcome.runs {
        *counts.entry(record.outcome).or_insert(0usize) += 1;
    }
    let counts: Vec<String> = counts
        .iter()
        .map(|(name, count)| format!("{count} {name}"))
        .collect();
    let baseline = match &outcome.baseline {
        Ok(_) => String::new(),
        Err(error) => format!(
            " The unrouted board's DRC (the baseline every run is judged against) failed: {error}."
        ),
    };
    let message = format!(
        "No run was routed and judged: {} run(s) launched ({}). Nothing was written to {}.{baseline} `search.runs` says why runs ended as they did, and `search.json` in the search folder lists every run. engine: \"freerouting\" routes with Freerouting instead.",
        outcome.runs.len(),
        if counts.is_empty() {
            "none started".to_string()
        } else {
            counts.join(", ")
        },
        call.ses_output.display(),
    );
    with_fields(
        CallToolResult::error_kind(
            ToolErrorKind::HandlerError {
                reason: "no run was routed and judged".to_string(),
            },
            message,
        ),
        {
            let mut fields = vec![("search", search)];
            fields.extend(pairs.fields());
            fields.extend(lengths.fields(None, None, None));
            fields
        },
    )
}

/// A following call's arguments: the effort (when the caller gave it) and the
/// width arguments this call was given are repeated into the
/// `route_specctra_dsn` steps of its completion, so that following them does not
/// bring back a default the caller did not choose. A step of a fastroute result
/// never names `engine: "freerouting"`: it routes with the default engine.
fn carry_arguments(result: &mut Value, args: &Value, effort: (Effort, EffortSource)) {
    let Some(steps) = result
        .pointer_mut("/completion/if_still_open/fallback")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for step in steps
        .iter_mut()
        .filter(|step| step["tool"] == "route_specctra_dsn")
    {
        if let Some(arguments) = step["arguments"].as_object_mut() {
            arguments.remove("engine");
        }
        if effort.1 == EffortSource::Argument {
            step["arguments"]["effort"] = json!(effort.0.name());
        }
        for key in ["width_policy", "width_fallback"] {
            if let Some(value) = args[key].as_str() {
                step["arguments"][key] = json!(value);
            }
        }
    }
}

// ─── The production runner ───────────────────────────────────────────────────

/// The production runner of a search.
pub(crate) struct FastrouteRunner {
    pub binary: crate::fastroute::Binary,
    /// The exported DSN's text, read once.
    pub dsn_text: Arc<String>,
    pub hints: RoutingHints,
    pub amps: Option<PathBuf>,
    /// The call's `--x.gs_uncounted` file (`fastroute::uncounted_file_text`, Row GS),
    /// none when the manifest has no pour.
    pub uncounted: Option<PathBuf>,
    /// The call's `--x.zones` file (`zone_fill::file_text`, Row ZR, ZR1), none when the
    /// plan models no pour. Sent only to the routing passes of a variant with planes.
    pub zones: Option<PathBuf>,
    /// The call's `--pairs` file (`route_pairs::plan`), none without declared pairs.
    pub pairs: Option<PathBuf>,
    /// The `--layer-heights` value the routing passes get: only with a resolved pair
    /// ([`routing_layer_heights`], L7).
    pub routing_heights: Option<String>,
    /// What the Tune pass needs, none without a length group (L6, L18).
    pub tune: Option<TuneFiles>,
    pub fallback: WidthFallback,
    /// The search folder: run `i` lives in `run-<i, 3 digits>-<shape>-s<seed>`.
    pub folder: PathBuf,
    pub judge: Arc<JudgeContext>,
    pub tools: Arc<ToolContext>,
}

/// The Tune pass's inputs as the runner owns them (Row LT, L6): what
/// `fastroute::TuneInputs` borrows.
#[derive(Debug, Clone)]
pub(crate) struct TuneFiles {
    /// The call's `tune.txt`.
    pub file: PathBuf,
    /// Every net class of the DSN.
    pub classes: Vec<String>,
    /// The DSN names of the nets the tune file lists.
    pub nets: std::collections::BTreeSet<String>,
    /// The Tune pass's own `--layer-heights` value (L7): sent whenever the call has
    /// heights.
    pub layer_heights: Option<String>,
    /// The call's length plan: the next round's tune file is written from it (L21).
    pub plan: Arc<route_lengths::LengthPlan>,
}

/// A shape's name as the run folders and the result spell it.
fn shape_name(shape: Shape) -> &'static str {
    match shape {
        Shape::AsExported => "as_exported",
        Shape::Omitted => "omitted",
    }
}

/// The name of run `index`'s folder inside the search folder.
fn run_folder_name(index: usize, shape: Shape, seed: u64) -> String {
    format!("run-{index:03}-{}-s{seed}", shape_name(shape))
}

impl Runner for FastrouteRunner {
    /// Creates the run's folder, writes `variant.dsn` (`route_search::variant_text`),
    /// reads its planes (`specctra::dsn_planes`, nets only, sorted, each once), and
    /// runs `fastroute::route_held` with the deadline. `PassError::Killed` is
    /// `RouteFailure::Killed`; any other error is `Failed` with its message.
    fn route(
        &self,
        index: usize,
        shape: Shape,
        seed: u64,
        deadline: tokio::time::Instant,
    ) -> BoxFuture<std::result::Result<Routed, RouteFailure>> {
        let binary = self.binary.clone();
        let dsn_text = Arc::clone(&self.dsn_text);
        let hints = self.hints;
        let amps = self.amps.clone();
        let uncounted = self.uncounted.clone();
        let zones = self.zones.clone();
        let pairs = self.pairs.clone();
        let routing_heights = self.routing_heights.clone();
        let tune = self.tune.clone();
        let fallback = self.fallback;
        let folder = self.folder.join(run_folder_name(index, shape, seed));
        Box::pin(async move {
            let failed = |what: &str, error: &dyn std::fmt::Display| {
                RouteFailure::Failed(format!("{what}: {error}"))
            };
            std::fs::create_dir(&folder)
                .map_err(|error| failed(&format!("create {}", folder.display()), &error))?;
            let variant = route_search::variant_text(&dsn_text, shape, seed)
                .map_err(|error| failed("make the variant DSN", &format!("{error:#}")))?;
            let dsn = folder.join("variant.dsn");
            std::fs::write(&dsn, &variant)
                .map_err(|error| failed(&format!("write {}", dsn.display()), &error))?;
            let mut planes: Vec<String> = crate::specctra::dsn_planes(&variant)
                .map_err(|error| failed("read the variant's planes", &format!("{error:#}")))?
                .into_iter()
                .map(|plane| plane.net)
                .collect();
            planes.sort();
            planes.dedup();
            // The disc of each via padstack of this run's DSN (the Tune pass's guard).
            let via_radii = crate::specctra::via_disc_radii(&variant);
            // Row LT, L21: the next round's tune file, without the nets that touched
            // themselves.
            let plan = tune.as_ref().map(|tune| Arc::clone(&tune.plan));
            let next = |folder: &Path, dropped: &std::collections::BTreeSet<String>| match &plan {
                Some(plan) => route_lengths::write_file_without(folder, plan, dropped),
                None => Ok(None),
            };
            let held = fastroute::route_held(
                &binary,
                &folder,
                &dsn,
                &hints,
                fastroute::CallFiles {
                    amps: amps.as_deref(),
                    pairs: pairs.as_deref(),
                    layer_heights: routing_heights.as_deref(),
                    uncounted: uncounted.as_deref(),
                    // Row ZR: the model is of the planes, so a variant without any
                    // (pours omitted) gets no zones file.
                    zones: zones.as_deref().filter(|_| !planes.is_empty()),
                },
                fallback,
                tune.as_ref().map(|tune| fastroute::TuneRequest {
                    inputs: fastroute::TuneInputs {
                        tune: &tune.file,
                        classes: &tune.classes,
                        nets: &tune.nets,
                        layer_heights: tune.layer_heights.as_deref(),
                        planes: &planes,
                        via_radii: &via_radii,
                    },
                    next: &next,
                }),
                deadline,
            )
            .await
            .map_err(|error| match error {
                PassError::Killed { seconds } => {
                    tracing::warn!(
                        "fastroute run {index} was killed at its deadline after {seconds:.1} s"
                    );
                    RouteFailure::Killed
                }
                PassError::Failed(error) => RouteFailure::Failed(format!("{error:#}")),
            })?;
            Ok::<Routed, RouteFailure>(Routed {
                index,
                shape,
                seed,
                dsn,
                held,
                planes,
                folder,
            })
        })
    }

    /// `route_judge::judge` with this call's context and tool context.
    fn judge(&self, routed: Routed, baseline: Arc<Baseline>) -> BoxFuture<Result<Judged>> {
        let judge = Arc::clone(&self.judge);
        let tools = Arc::clone(&self.tools);
        Box::pin(async move {
            let (index, shape, seed) = (routed.index, routed.shape, routed.seed);
            route_judge::judge(&judge, &routed, &baseline, tools)
                .await
                .map_err(|error| {
                    error.context(format!(
                        "judging run {index} (shape {}, seed {seed})",
                        match shape {
                            Shape::AsExported => "as exported",
                            Shape::Omitted => "pours omitted",
                        }
                    ))
                })
        })
    }

    /// `route_judge::baseline` with this call's context.
    fn baseline(&self) -> BoxFuture<Result<Baseline>> {
        let judge = Arc::clone(&self.judge);
        Box::pin(async move { route_judge::baseline(&judge).await })
    }

    /// The call's mode, kept in its judge context.
    fn rank(&self) -> crate::pour_rank::RankMode {
        self.judge.rank
    }

    /// Removes the run's judge folder (`judge/run-<i>`, its scratch boards) and, in
    /// each pass folder, the sandbox's `home` and `tmp`, its profile and the
    /// router's log. The variant, the sessions and the pass reports stay: a
    /// released run can still win, and its record needs them. Best effort: a
    /// failure is logged, never raised. Nothing outside the search folder is
    /// touched, and no link is followed (`remove_tree`).
    fn release(&self, index: usize, shape: Shape, seed: u64) {
        release_run(&self.folder, &self.judge.root, index, shape, seed);
    }
}

/// What [`FastrouteRunner::release`] does: removes the judge folders of run `i`
/// (`<judge root>/run-<i>`, `run-<i>-untuned` and `run-<i>-pour-safe`) and, in each
/// pass folder of run `i`, `home`, `tmp`, `profile.sb` and `fastroute.log`, except the
/// `fastroute.log` of a Tune pass folder (L26: the log of a discarded or failed Tune
/// pass is kept for the record, and [`reduce_run_folders`] removes the rest).
/// A failure is logged, never raised; no link is followed (`remove_tree`).
fn release_run(folder: &Path, judge_root: &Path, index: usize, shape: Shape, seed: u64) {
    let run = folder.join(run_folder_name(index, shape, seed));
    let mut paths: Vec<PathBuf> = ["", "-untuned", "-pour-safe"]
        .iter()
        .map(|suffix| judge_root.join(format!("run-{index}{suffix}")))
        .collect();
    // A run or pass folder that is not a real folder (a link, a file) is not
    // ours: `remove_tree` checks only the last part of a path, so a link here
    // would be followed.
    if is_real_folder(&run) {
        let tune = fastroute::tune_folder_names();
        let passes = ["full", "fallback", "polish"]
            .iter()
            .map(|name| (name.to_string(), true))
            .chain(tune.into_iter().map(|name| (name, false)));
        for (pass, log_goes) in passes {
            let pass = run.join(pass);
            if !is_real_folder(&pass) {
                continue;
            }
            for name in ["home", "tmp", "profile.sb", "fastroute.log"] {
                if name == "fastroute.log" && !log_goes {
                    continue;
                }
                paths.push(pass.join(name));
            }
        }
    }
    for path in paths {
        if let Err(error) = remove_tree(&path) {
            tracing::warn!("releasing run {index}: remove {}: {error}", path.display());
        }
    }
}

/// `path` is a folder itself, not a link to one.
fn is_real_folder(path: &Path) -> bool {
    path.symlink_metadata().is_ok_and(|meta| meta.is_dir())
}

// ─── After the search ────────────────────────────────────────────────────────

/// Removes `path` and everything below it, and follows no symbolic link: a link
/// is left where it is (and keeps its parent folder). A path that is not there
/// is not an error.
fn remove_tree(path: &Path) -> std::io::Result<()> {
    let meta = match path.symlink_metadata() {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if !meta.is_dir() {
        return std::fs::remove_file(path);
    }
    for entry in std::fs::read_dir(path)? {
        remove_tree(&entry?.path())?;
    }
    match std::fs::remove_dir(path) {
        Ok(()) => Ok(()),
        // Something stayed (a link): the folder stays with it.
        Err(_) if std::fs::read_dir(path).is_ok_and(|mut left| left.next().is_some()) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Removes everything in the folder `path` but the entries named in `keep` (a link is
/// left where it is, as in [`remove_tree`]).
fn remove_all_but(path: &Path, keep: &[&str]) -> std::io::Result<()> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| keep.contains(&name))
        {
            continue;
        }
        remove_tree(&entry.path())?;
    }
    Ok(())
}

/// After a search: every run folder but the winner's is reduced to its
/// `run.json` (its `RunRecord` and, when judged, its measures, gates and
/// rounds and its `group_search` summary when it has one, Row GS; with a Tune pass,
/// its rows and the files it kept), and the judge folders of
/// every run but the winner's (`<folder>/judge/run-<i>`, `run-<i>-untuned`,
/// `run-<i>-pour-safe`, and Row ZR, ZR0b's `run-<i>-pruned` and `run-<i>-prune`) are
/// removed. The winner's `run.json` also holds `pour_prune` when the prune ran. The run folder keeps, for each Tune pass that was
/// discarded or failed, its `fastroute.log` and its session (L26; the pass folders of
/// the Tune passes it kept go whole). The winner's run folder and judged board stay
/// whole. Only paths inside `folder` are
/// touched; a path that is a symbolic link is left alone.
pub(crate) fn reduce_run_folders(
    folder: &Path,
    outcome: &crate::route_search::SearchOutcome,
) -> Result<()> {
    if !folder.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
        anyhow::bail!("{} is not a folder", folder.display());
    }
    let mut problems: Vec<String> = Vec::new();
    let mut others: Vec<usize> = Vec::new();
    for (position, record) in outcome.runs.iter().enumerate() {
        let is_winner = outcome.winner == Some(position);
        let run = folder.join(run_folder_name(record.index, record.shape, record.seed));
        // A run folder that is not a real folder (a link, a file) is not ours.
        if !run.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
            continue;
        }
        if !is_winner {
            others.push(record.index);
        }
        let key = outcome.keys.get(position).copied().flatten();
        // Row ZR, ZR5 part 2 (PK10): both keys, whatever key ranked.
        let pour_key = outcome.pour_keys.get(position).copied().flatten();
        let mut body = json!({
            "record": record,
            "key": key,
            "pour_key": pour_key,
            "board_kept": is_winner,
        });
        // Row ZR, ZR0b: the winner's record says what the prune of its ground vias did.
        if let (true, Some(prune)) = (is_winner, &outcome.pour_prune) {
            body["pour_prune"] = json!(prune);
        }
        // Row ZR, ZR6 (S10): and what the delivery of its reference vias did.
        if let (true, Some(delivery)) = (is_winner, &outcome.reference_stitch) {
            body["reference_stitch"] = json!(delivery);
        }
        // The winner keeps every folder whole.
        let keep_tune = if is_winner {
            Vec::new()
        } else {
            tune_folders_to_keep(record)
        };
        if let Some(routed) = &record.routed {
            body["kept_pass"] = json!(routed.held.kept);
            body["passes"] = json!(routed.held.passes);
            body["planes"] = json!(routed.planes);
            if !routed.held.tune.rows.is_empty() {
                // Row LT, L26: every Tune pass, and the files kept of those discarded
                // or failed.
                body["tune"] = json!({
                    "rows": routed.held.tune.rows,
                    "kept_files": keep_tune
                        .iter()
                        .flat_map(|name| KEPT_TUNE_FILES.iter().map(move |file| format!("{name}/{file}")))
                        .collect::<Vec<_>>(),
                });
            }
        }
        if let Some(judged) = &record.judged {
            body["judged"] = json!(judged);
            // Row GS: how each judged run's group search ended, not only the winner's.
            if let Some(summary) = record
                .routed
                .as_ref()
                .and_then(|r| held_group_search(&r.held))
            {
                body["group_search"] = summary;
            }
            // Row ZR, ZR1: what the engine's model said on the same pass.
            if let Some(object) = record.routed.as_ref().and_then(|r| r.held.planes.as_ref()) {
                body["engine_planes"] = crate::zone_fill::engine_planes_record(object, true);
            }
        }
        let record_file = run.join("run.json");
        if let Err(error) = std::fs::write(
            &record_file,
            serde_json::to_string_pretty(&body).unwrap_or_default(),
        ) {
            // No record, no removal.
            problems.push(format!("write {}: {error}", record_file.display()));
            continue;
        }
        if is_winner {
            continue;
        }
        for name in RUN_ENTRIES {
            let path = run.join(name);
            if let Err(error) = remove_tree(&path) {
                problems.push(format!("remove {}: {error}", path.display()));
            }
        }
        // The Tune pass folders (L26): a discarded or failed pass keeps its log and
        // its session; every other pass folder goes whole.
        for name in fastroute::tune_folder_names() {
            let path = run.join(&name);
            let result = if keep_tune.contains(&name) && is_real_folder(&path) {
                remove_all_but(&path, &KEPT_TUNE_FILES)
            } else {
                remove_tree(&path)
            };
            if let Err(error) = result {
                problems.push(format!("remove {}: {error}", path.display()));
            }
        }
    }
    // The judge's scratch boards of the other runs.
    let judge = folder.join("judge");
    if judge.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
        if let Ok(entries) = std::fs::read_dir(&judge) {
            for entry in entries.flatten() {
                let index = entry.file_name().to_str().and_then(judge_folder_index);
                if index.is_some_and(|index| others.contains(&index)) {
                    if let Err(error) = remove_tree(&entry.path()) {
                        problems.push(format!("remove {}: {error}", entry.path().display()));
                    }
                }
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{}", problems.join("; "))
    }
}

/// Seconds as a number rounded to 0.1.
fn round1(seconds: f64) -> f64 {
    (seconds * 10.0).round() / 10.0
}

/// A path as the result spells it.
fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Each run's record as a JSON row, its seconds rounded to 0.1.
fn run_rows(runs: &[&RunRecord]) -> Vec<Value> {
    runs.iter()
        .map(|record| {
            let mut row = serde_json::to_value(record).unwrap_or(Value::Null);
            if let Some(object) = row.as_object_mut() {
                for (key, value) in object.iter_mut() {
                    if key.ends_with("_s") {
                        if let Some(seconds) = value.as_f64() {
                            *value = json!(round1(seconds));
                        }
                    }
                }
                // Row LT (L15): G4 and the intent's miss, only for a run that was
                // checked for length, so that a call without groups reads as before.
                if let Some(judged) = &record.judged {
                    let lengths = judged.measures.lengths.as_ref();
                    if lengths.is_some() || !judged.gates.g4.is_empty() {
                        object.insert("g4_failed".into(), json!(!judged.gates.g4.is_empty()));
                    }
                    if let Some(lengths) = lengths {
                        object.insert(
                            "length_miss_mm".into(),
                            json!((lengths.miss_mm * 1000.0).round() / 1000.0),
                        );
                        if !lengths.unmeasured.is_empty() {
                            object.insert("length_unmeasured".into(), json!(lengths.unmeasured));
                        }
                    }
                }
            }
            row
        })
        .collect()
}

/// The judged board's path, or null when it is no longer there: a run outside the
/// best `KEPT_WHOLE` when it was judged was released, and can still win later.
fn judged_board(judged: &Judged) -> Value {
    if judged.board.is_file() {
        json!(text(&judged.board))
    } else {
        Value::Null
    }
}

/// Why `judged_board` is null.
const JUDGED_BOARD_RELEASED: &str = "The judged board was removed during the search: the run was not among the best runs when it was judged and only rose to win later. Its session is the result; apply it to see the board.";

/// How many runs the result lists of each kind: the best judged runs by the rank
/// key, then the runs that were not judged, in launch order. `search.json` lists
/// every run.
pub(crate) const LISTED_RUNS: usize = 20;

/// Which runs `search.runs` holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Listing {
    /// Every run, in launch order (`search.json`).
    Every,
    /// The result's list: see [`LISTED_RUNS`].
    Best,
}

/// The runs a listing holds. `Best` sorts the judged runs by the key that ranked
/// (`outcome.rank`: `keys` or `pour_keys`; Row ZR, ZR5 part 2, PK12).
fn listed_runs(outcome: &SearchOutcome, listing: Listing) -> Vec<&RunRecord> {
    if listing == Listing::Every {
        return outcome.runs.iter().collect();
    }
    let ranking_key = |position: usize| -> Option<Vec<i64>> {
        match outcome.rank {
            RankMode::Routing => outcome
                .keys
                .get(position)
                .copied()
                .flatten()
                .map(|key| key.to_vec()),
            RankMode::Pours => outcome
                .pour_keys
                .get(position)
                .copied()
                .flatten()
                .map(|key| key.to_vec()),
        }
    };
    let mut judged: Vec<(Vec<i64>, usize)> = outcome
        .runs
        .iter()
        .enumerate()
        .filter(|(_, record)| record.outcome == "judged")
        .filter_map(|(position, _)| Some((ranking_key(position)?, position)))
        .collect();
    judged.sort();
    let mut rows: Vec<&RunRecord> = judged
        .iter()
        .take(LISTED_RUNS)
        .map(|(_, position)| &outcome.runs[*position])
        .collect();
    rows.extend(
        outcome
            .runs
            .iter()
            .filter(|record| record.outcome != "judged")
            .take(LISTED_RUNS),
    );
    rows
}

/// The winner's own part of `search`: its identity, verdict and passes, and
/// `group_search` ([`group_search_summary`]) when the report of its kept routing pass
/// (the routing pass a kept Tune pass started from, when there is one) has a summary
/// to give (Row GS, G5). `None` when no run won, or the winning record lacks
/// what a result needs.
fn winner_json(outcome: &SearchOutcome) -> Option<Value> {
    let position = outcome.winner?;
    let record = outcome.runs.get(position)?;
    let (routed, judged) = (record.routed.as_ref()?, record.judged.as_ref()?);
    let mut winner = json!({
        "index": record.index,
        "shape": record.shape,
        "seed": record.seed,
        "gates": judged.gates,
        "measures": judged.measures,
        "judged_board": judged_board(judged),
        "passes": routed.held.passes,
        "kept": routed.held.kept,
        "key": outcome.keys.get(position).copied().flatten(),
        "pour_key": outcome.pour_keys.get(position).copied().flatten(),
    });
    if let Some(summary) = held_group_search(&routed.held) {
        winner["group_search"] = summary;
    }
    // Row ZR, ZR1: the engine's model of the same pass, its pad lists as counts.
    if let Some(object) = &routed.held.planes {
        winner["engine_planes"] = crate::zone_fill::engine_planes_record(object, false);
    }
    Some(winner)
}

/// [`group_search_summary`] of `held`'s kept routing pass, `None` without one.
fn held_group_search(held: &crate::fastroute::Held) -> Option<Value> {
    held.group_search.as_ref().and_then(group_search_summary)
}

/// What the group search did on the winner's kept routing pass, from the engine's
/// report (Row GS, G5): that pass's search only, never the other passes' of the run,
/// and with differential pairs the last pipeline run of the pass that ran the stage
/// (fastroute re-runs its pipeline once after it releases the pairs). `object` is the report's `group_search` object, as
/// `fastroute::Held::group_search` keeps it. The summary: `ended_by` (`converged`,
/// `pass_cap` or `stopped`, as the engine names it), `passes` (their count),
/// `kept_moves` (the sum of the passes' `kept`), `largest_group` (the largest
/// `largest_kept_group`, 0 without a pass), `start` and `end` (`length_mm`, `vias`,
/// `cost_mm`). `None` when the object does not read as one: no `ended_by` string, no
/// `passes` list, a pass without a whole `kept` or `largest_kept_group`, or a `start`
/// or `end` without the three numbers. Never an error.
///
/// Row ZR, stage ZR3: when the engine reports the pour state (fastroute x22 with
/// `--x.zones` writes `pour` in `start` and `end`), `start` and `end` keep their `pour`
/// object as written, and `pour_worse` counts the moves that only the "never more pour
/// damage" rule refused (the passes' `ends.pour_worse`, a missing or unreadable count as
/// 0). `start` decides: the engine writes `pour` in both or in neither. Without it the
/// summary is as before, key for key. The `cut` ends are not summed here: the engine's
/// `planes.search_cut_ends` (ZR2) counts them.
fn group_search_summary(object: &Value) -> Option<Value> {
    let ended_by = object["ended_by"].as_str()?;
    let passes = object["passes"].as_array()?;
    let mut kept_moves = 0u64;
    let mut largest_group = 0u64;
    for pass in passes {
        kept_moves = kept_moves.checked_add(pass["kept"].as_u64()?)?;
        largest_group = largest_group.max(pass["largest_kept_group"].as_u64()?);
    }
    let value = |key: &str| -> Option<Value> {
        let value = &object[key];
        let mut summary = json!({
            "length_mm": value["length_mm"].as_f64()?,
            "vias": value["vias"].as_u64()?,
            "cost_mm": value["cost_mm"].as_f64()?,
        });
        if let Some(pour) = value.get("pour").filter(|pour| pour.is_object()) {
            summary["pour"] = pour.clone();
        }
        Some(summary)
    };
    let mut summary = json!({
        "ended_by": ended_by,
        "passes": passes.len(),
        "kept_moves": kept_moves,
        "largest_group": largest_group,
        "start": value("start")?,
        "end": value("end")?,
    });
    if summary["start"].get("pour").is_some() {
        let pour_worse = passes
            .iter()
            .map(|pass| pass["ends"]["pour_worse"].as_u64().unwrap_or(0))
            .fold(0u64, u64::saturating_add);
        summary["pour_worse"] = json!(pour_worse);
    }
    Some(summary)
}

/// The unrouted board's check, as the result reports it.
fn baseline_json(baseline: &std::result::Result<Baseline, String>) -> Value {
    match baseline {
        Ok(baseline) => {
            let mut out = json!({
                "ok": true,
                "violations": baseline.violations,
                "unconnected": baseline.unconnected,
            });
            // Row ZR, ZR5 part 1: the "before" rows of the pour record, or why there are
            // none; neither for a call with no pour net.
            if let Some(rows) = &baseline.pour_fills {
                out["pour_fills"] = json!(rows);
            }
            if let Some(error) = &baseline.pour_fills_error {
                out["pour_fills_error"] = json!(error);
            }
            out
        }
        Err(error) => json!({ "ok": false, "error": error }),
    }
}

/// `search` of the result (`Listing::Best`) and the whole of `search.json`
/// (`Listing::Every`): the lanes, the shapes, the count of each outcome, the runs
/// the listing holds, the winner, how the search ended and the baseline.
fn search_json(
    plan: &SearchPlan,
    shapes: &[Shape],
    outcome: &SearchOutcome,
    listing: Listing,
) -> Value {
    let mut outcomes = std::collections::BTreeMap::new();
    for record in &outcome.runs {
        *outcomes.entry(record.outcome).or_insert(0usize) += 1;
    }
    let rows = listed_runs(outcome, listing);
    let mut search = json!({
        "lanes": plan.lanes,
        "shapes": shapes,
        "launched": outcome.runs.len(),
        "judged": outcome.runs.iter().filter(|record| record.outcome == "judged").count(),
        "outcomes": outcomes,
        "winner": winner_json(outcome),
        "runs": run_rows(&rows),
        "elapsed_s": round1(outcome.elapsed.as_secs_f64()),
        "budget_overrun_s": outcome.budget_overrun.map(|over| round1(over.as_secs_f64())),
        "stopped_launching": outcome.stopped_launching,
        "baseline": baseline_json(&outcome.baseline),
    });
    // Row ZR, ZR0b: what the prune of the winner's ground vias did; absent when it was off.
    if let Some(record) = &outcome.pour_prune {
        search["pour_prune"] = json!(record);
    }
    // Row ZR, ZR6 (S10): what the delivery of the winner's reference vias did; absent
    // when the round was off.
    if let Some(record) = &outcome.reference_stitch {
        search["reference_stitch"] = json!(record);
    }
    if listing == Listing::Best && rows.len() < outcome.runs.len() {
        search["runs_listed"] = json!(format!(
            "{} of {} runs: the {LISTED_RUNS} best judged by the rank key, then up to {LISTED_RUNS} that were not judged; search.json in the search folder lists every run",
            rows.len(),
            outcome.runs.len()
        ));
    }
    search
}

/// The `status` of a fastroute result, from the judge's gates first.
/// - G1 failed (connections open after the refill and the stitching): `incomplete`.
/// - G2 (DRC), G3 (widths) or G4 (lengths, Row LT) failed: `complete_with_gate_failures`,
///   never `complete`.
/// - Every gate passed (all four) and the judge added no stitching via: the winner's session alone,
///   applied to a copy of the board and refilled, leaves nothing open, which is what the
///   router's own count cannot show (D21). The router's `incomplete` (its count, which
///   takes a connection a pour makes for one it left) and its plane `unverified` give way.
///   What remains is the width report, as for a router that left nothing and had no
///   plane: `unverified` when widths could not be read or a wire could not be measured,
///   `complete_with_width_exceptions` when a stretch is below intent width, else
///   `complete`. A `stopped` router status stands.
/// - Every gate passed, but only with stitching vias the judge added: those vias are not
///   in the session, so the router's status stands, as before D21.
///
/// KiCad's DRC on the judged copy sees what the router's count cannot: on 2026-11-16 the
/// router research found fastroute runs that break a net class's larger clearance on one of
/// the owner's boards while reporting nothing open.
pub(crate) fn judged_status(
    router_status: &'static str,
    gates: &crate::route_judge::Gates,
    width: &WidthOutcome,
    stitching_vias: usize,
) -> &'static str {
    if !gates.g1.is_empty() {
        "incomplete"
    } else if !gates.g2.is_empty() || !gates.g3.is_empty() || !gates.g4.is_empty() {
        "complete_with_gate_failures"
    } else if stitching_vias > 0 {
        router_status
    } else if router_status == "stopped" || width.stopped_in.is_some() {
        "stopped"
    } else if width.unread || width.unmeasured > 0 {
        "unverified"
    } else if width.exceptions > 0 {
        "complete_with_width_exceptions"
    } else {
        "complete"
    }
}

/// What `completion.reason` adds when every gate passed and the status is not the
/// router's own: the router's count stays in the reason, and this says why the status
/// differs from it.
pub(crate) const JUDGED_STATUS_NOTE: &str = " Every judge gate passed: the judge applied this session to a copy of the board, refilled it and found 0 unconnected items with no stitching via added, so `status` is the judge's (`completion.judged`), not the router's count.";

/// What `completion.reason` adds when every gate passed only with stitching vias the
/// judge added: why `status` is still the router's.
pub(crate) fn judged_stitched_note(vias: usize) -> String {
    format!(" Every judge gate passed, but only after the judge added {vias} stitching via(s) (`completion.judged.stitching_vias`), which are not in this session: `status` stays the router's. Follow `next_steps`; they include the stitching.")
}

/// Row ZR, ZR6 (S10): what `completion.reason` adds when every gate passed only with
/// ground vias the judge's reference round added (and maybe Z9's stitching vias), which
/// the delivered session lacks: why `status` is still the router's.
pub(crate) fn judged_reference_note(stitching_vias: usize, reference_vias: usize) -> String {
    let z9 = if stitching_vias > 0 {
        format!(" and {stitching_vias} stitching via(s) (`completion.judged.stitching_vias`)")
    } else {
        String::new()
    };
    format!(" Every judge gate passed, but only after the judge added {reference_vias} ground via(s) at reference changes (`completion.judged.reference_vias`){z9}, which are not in this session (`search.reference_stitch` says why): `status` stays the router's. Follow `next_steps`; they include the stitching.")
}

/// Row ZR, ZR6 (S10): the steps that add the reference vias the delivered session lacks,
/// after the import's DRC: the read-only planner's `stitching` kind on the reference nets,
/// the adoption, and the DRC again.
pub(crate) fn reference_steps(board: &Path, nets: &[String], vias: usize) -> Vec<Value> {
    let drc = json!({ "board": board, "sync_live_board": true, "refill_zones": true });
    vec![
        json!({
            "tool": "plan_route_refinement",
            "arguments": { "board": board, "kinds": ["stitching"], "nets": nets,
                           "search_radius_mm": crate::route_judge::STITCH_RADIUS_MM },
            "why": format!("The judge added {vias} ground via(s) beside signal vias that change reference plane (checklist 3.2), and this session does not hold them. Ask the read-only planner for them. It refuses a board with local pad overrides or teardrops without a UUID; then place a ground via within 3 mm of each signal via the audit lists under 3.2."),
        }),
        json!({
            "tool": "route_batch",
            "arguments": { "board": board },
            "arguments_from": { "vias": "the route_batch operation of each stitching proposal you adopt" },
            "why": "Adopt the ground vias you support.",
        }),
        json!({
            "tool": "run_drc", "arguments": drc,
            "why": "Refill, save and check that nothing got worse.",
        }),
    ]
}

/// fastroute's `completion.complete_when`: the shared sentence says the result's status
/// cannot show completion, which the judge's status now can (D21).
const SHARED_COMPLETE_WHEN_END: &str =
    "fastroute's count, and this result's status, do not show that.";
const JUDGED_COMPLETE_WHEN_END: &str = "fastroute's count does not show that. The judge's check on a copy of the board does: `status` follows it when every judge gate passed and the judge added no stitching via (`completion.judged`).";

/// Row LT, L15: `length_matching`'s `untuned_nets` and `tune_candidates` for the
/// winner; `None` when the run had no Tune pass. `tune_nets` are the DSN names of the
/// tune file's nets.
/// - `untuned_nets`: every tune-file net the written session leaves untuned, each with
///   its reason (`net`, `reason`):
///   - the judge kept the untuned board: every net, "the judge kept the untuned board:
///     <the rejection or rank reason>" ([`route_judge::TuneChoice::untuned_reason`]);
///   - no candidate survived its passes: every net, with the last pass's reason;
///   - otherwise the chosen candidate's dropped nets (L21), each with its reason.
/// - `tune_candidates`: one row per candidate the judge saw (`route_judge::TuneChoice`
///   rows: `variant`, `passes`, `dropped`, `judged`, `g4_failed`, `rejected`, `reason`,
///   `chosen`).
fn tuning_fields(
    routed: &Routed,
    judged: &Judged,
    tune_nets: &std::collections::BTreeSet<String>,
) -> Option<(Value, Value)> {
    if routed.held.tune.rows.is_empty() {
        return None;
    }
    let every_net = |reason: &str| -> Vec<Value> {
        tune_nets
            .iter()
            .map(|net| json!({ "net": net, "reason": reason }))
            .collect()
    };
    let untuned: Vec<Value> = match (&judged.tune, judged.tune.as_ref().and_then(|c| c.chosen)) {
        (_, Some(variant)) => routed
            .held
            .tune
            .candidates
            .iter()
            .find(|candidate| candidate.variant == variant)
            .map(|candidate| {
                candidate
                    .dropped
                    .iter()
                    .map(|(net, reason)| json!({ "net": net, "reason": reason }))
                    .collect()
            })
            .unwrap_or_default(),
        (Some(choice), None) => every_net(&format!(
            "the judge kept the untuned board: {}",
            choice.untuned_reason()
        )),
        (None, None) => {
            // No candidate survived its passes: the last pass that ran says why.
            let last = routed
                .held
                .tune
                .rows
                .iter()
                .rev()
                .find(|row| row["outcome"] != "skipped")
                .or_else(|| routed.held.tune.rows.last());
            let reason = last
                .and_then(|row| {
                    row["reason"].as_str().map(str::to_string).or_else(|| {
                        row["outcome"]
                            .as_str()
                            .map(|outcome| format!("the last Tune pass was {outcome}"))
                    })
                })
                .unwrap_or_else(|| "no Tune candidate survived".to_string());
            every_net(&reason)
        }
    };
    let candidates = judged
        .tune
        .as_ref()
        .map_or_else(Vec::new, |choice| choice.rows.clone());
    Some((json!(untuned), json!(candidates)))
}

/// The success result. Today's fields, for the winner: `success: true`, `status` and
/// `completion` from `integration::completion_report(dsn, ses_output, the winner's
/// planes, the winner's unrouted, width)`, with `completion.judged` = { unconnected,
/// gates, all_gates_pass, stitching_vias, judged_board }; `intent_widths`, `width_exceptions`,
/// `width_exceptions_total` (and `width_unmeasured` when any) from
/// `integration::width_report(dsn, ses_output)`; `method: "local_fastroute_best_of_n"`;
/// `engine` { name: "fastroute", candidate, path, sha256, version, source, switches
/// (`fastroute::routing_switches`: the candidate switches, then the search switches),
/// threads }; `effort` { level, source, budget_s, runs }; `search` { lanes, shapes,
/// launched, judged, winner { index, shape, seed, gates, measures, passes, kept, and
/// group_search when the kept routing pass's report has one (Row GS) },
/// runs (every record), elapsed_s, budget_overrun_s, stopped_launching, baseline };
/// `artifacts` { dsn_path, ses_output_path, search_folder, judged_board };
/// `passes` and `result_pass` of the winner, as the Freerouting hold path names them;
/// `differential_pairs` and `differential_pairs_unresolved` (and
/// `differential_pairs_note` when the design intent could not be used), from `pairs`;
/// and, when `lengths` is reported (Row LT, L15), `length_matching` with the winner's
/// measures and the Tune pass's row, and the length lines of `completion.next_steps`
/// (L16).
#[allow(clippy::too_many_arguments)] // the parts of one finished call, each read once
pub(crate) fn result_json(
    dsn: &Path,
    ses_output: &Path,
    folder: &Path,
    binary: &crate::fastroute::Binary,
    effort: (Effort, EffortSource),
    plan: &crate::route_search::SearchPlan,
    shapes: &[Shape],
    outcome: &crate::route_search::SearchOutcome,
    pairs: &route_pairs::PairsPlan,
    lengths: &LengthPlan,
) -> Value {
    let search = search_json(plan, shapes, outcome, Listing::Best);
    let winner = outcome
        .winner
        .and_then(|position| outcome.runs.get(position))
        .and_then(|record| Some((record.routed.as_ref()?, record.judged.as_ref()?)));
    let Some((routed, judged)) = winner else {
        let mut result = json!({
            "success": false,
            "error": "no run was routed and judged",
            "search": search,
        });
        for (key, value) in pairs
            .fields()
            .into_iter()
            .chain(lengths.fields(None, None, None))
        {
            result[key] = value;
        }
        return result;
    };
    // The planes the winning variant gave the router, with their layers, from
    // the variant's own DSN.
    let planes = std::fs::read_to_string(&routed.dsn)
        .map_err(|error| {
            anyhow::anyhow!(
                "the winning variant {} could not be read: {error}",
                routed.dsn.display()
            )
        })
        .and_then(|variant| crate::specctra::dsn_planes(&variant));
    let (widths, width_error) = width_report(dsn, ses_output);
    let width = WidthOutcome {
        exceptions: widths.exceptions_total,
        unread: width_error.is_some(),
        unmeasured: widths.unmeasured_total,
        ..WidthOutcome::default()
    };
    let (status, mut completion) = completion_report(
        dsn,
        ses_output,
        &planes,
        Some(u32::try_from(routed.held.unrouted).unwrap_or(u32::MAX)),
        &width,
    );
    // The shared report says "Freerouting"; the router of this call is fastroute.
    for key in ["reason", "complete_when"] {
        let words = completion[key].as_str().map(str::to_owned);
        if let Some(words) = words {
            completion[key] = json!(words.replace("Freerouting", "fastroute"));
        }
    }
    completion["router_unrouted_source"] =
        json!("fastroute's report of the winning run's kept pass");
    let complete_when = completion["complete_when"].as_str().map(str::to_owned);
    if let Some(words) = complete_when {
        completion["complete_when"] =
            json!(words.replace(SHARED_COMPLETE_WHEN_END, JUDGED_COMPLETE_WHEN_END));
    }
    completion["judged"] = json!({
        "unconnected": judged.measures.unconnected,
        "gates": judged.gates,
        "all_gates_pass": judged.gates.all_pass(),
        "stitching_vias": judged.measures.stitching_vias,
        "reference_vias": judged.measures.reference_vias,
        "judged_board": judged_board(judged),
    });
    if completion["judged"]["judged_board"].is_null() {
        completion["judged"]["judged_board_note"] = json!(JUDGED_BOARD_RELEASED);
    }
    let router_status = status;
    let stitching_vias = judged.measures.stitching_vias;
    // Row ZR, ZR6 (S10): the reference vias the judged board has and the session lacks
    // count as Z9's do.
    let reference_vias = judged.measures.reference_vias;
    let status = judged_status(
        router_status,
        &judged.gates,
        &width,
        stitching_vias + reference_vias,
    );
    let note = if !judged.gates.all_pass() {
        None
    } else if reference_vias > 0 {
        Some(judged_reference_note(stitching_vias, reference_vias))
    } else if stitching_vias > 0 {
        Some(judged_stitched_note(stitching_vias))
    } else if status != router_status {
        Some(JUDGED_STATUS_NOTE.to_string())
    } else {
        None
    };
    if let Some(note) = note {
        let reason = completion["reason"].as_str().map(str::to_owned);
        if let Some(reason) = reason {
            completion["reason"] = json!(format!("{reason}{note}"));
        }
    }
    let limits = effort.0.limits();
    let mut result = json!({
        "success": true,
        // The judge outranks the router's own count: see `judged_status`.
        "status": status,
        "completion": completion,
        "method": "local_fastroute_best_of_n",
        "engine": {
            "name": "fastroute",
            "candidate": fastroute::CANDIDATE_NAME,
            "path": text(&binary.path),
            "sha256": binary.sha256,
            "version": binary.version,
            "source": binary.source,
            "switches": fastroute::routing_switches(),
            "threads": fastroute::THREADS,
        },
        "effort": {
            "level": effort.0.name(),
            "source": effort.1,
            "budget_s": limits.budget.map(|budget| round1(budget.as_secs_f64())),
            "runs": limits.runs,
        },
        "search": search,
        "artifacts": {
            "dsn_path": text(dsn),
            "ses_output_path": text(ses_output),
            "search_folder": text(folder),
            "judged_board": judged_board(judged),
            "passes_folder": text(&routed.folder),
        },
        "passes": routed.held.passes,
        "result_pass": routed.held.kept,
        "intent_widths": widths.nets,
        "width_exceptions": widths.exceptions,
        "width_exceptions_total": widths.exceptions_total,
    });
    for (key, value) in pairs.fields() {
        result[key] = value;
    }
    // Row LT: what was asked of the lengths, what the winner measured, and the KiCad
    // rules that keep checking it (L15, L16).
    // `held.passes` holds the chosen candidate's last Tune row (L23 rewrote it); L15's
    // `tune_candidates` and `untuned_nets` come from the judge's choice.
    let tune_row = routed
        .held
        .passes
        .iter()
        .find(|row| row["name"] == fastroute::TUNE_PASS);
    let winner_lengths = judged.measures.lengths.as_ref();
    for (key, value) in lengths.fields(winner_lengths, Some(&judged.measures.violations), tune_row)
    {
        result[key] = value;
    }
    if let (Some(body), Some((untuned, candidates))) = (
        result
            .get_mut("length_matching")
            .and_then(Value::as_object_mut),
        tuning_fields(routed, judged, &lengths.tune_nets()),
    ) {
        body.insert("untuned_nets".into(), untuned);
        body.insert("tune_candidates".into(), candidates);
    }
    // Row ZR, ZR6 (S10): the reference vias the session lacks, as Z9's stitching steps.
    let reference_nets = crate::pour_stitch::undelivered_nets(judged);
    if !reference_nets.is_empty() {
        let board = crate::specctra::manifest_beside(dsn)
            .as_ref()
            .and_then(|manifest| manifest["board_path"].as_str().map(PathBuf::from))
            .filter(|board| board.is_file());
        if let (Some(board), Some(steps)) =
            (board, result["completion"]["next_steps"].as_array_mut())
        {
            for mut step in reference_steps(&board, &reference_nets, reference_vias) {
                step["step"] = json!(steps.len() + 1);
                steps.push(step);
            }
        }
    }
    let length_steps = lengths.next_steps(
        winner_lengths,
        crate::length_probe::kicad_rule_violations(&judged.measures.violations),
    );
    if let Some(steps) = result["completion"]["next_steps"].as_array_mut() {
        for line in length_steps {
            let number = steps.len() + 1;
            steps.push(json!({ "step": number, "manual": line }));
        }
    }
    if let Some(error) = width_error {
        result["width_report_error"] = json!(error);
    }
    if widths.unmeasured_total > 0 {
        result["width_unmeasured"] = json!(widths.unmeasured);
        result["width_unmeasured_total"] = json!(widths.unmeasured_total);
    }
    result
}

#[cfg(test)]
#[path = "route_fastroute_tests.rs"]
mod tests;
