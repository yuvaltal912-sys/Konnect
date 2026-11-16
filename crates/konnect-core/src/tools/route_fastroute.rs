//! `route_specctra_dsn` with the fastroute engine: a timed best-of-N search over
//! DSN variants, each run judged by Konnect itself, the best kept by absolute gates
//! (docs/ROUTE_SEARCH_DESIGN.md). The Freerouting engine stays in `integration.rs`,
//! unchanged.

use crate::fastroute::{self, Binary, Located, PassError};
use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::{CallToolResult, ToolContent};
use crate::route_judge::{self, Baseline, JudgeContext, Judged, Refusal};
use crate::route_search::{
    self, BoxFuture, Effort, EffortSource, RouteFailure, Routed, RunRecord, Runner, SearchOutcome,
    SearchPlan, Shape,
};
use crate::specctra::RoutingHints;
use crate::tools::integration::{
    completion_report, width_fallback_argument, width_report, WidthFallback, WidthOutcome,
};
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
/// the folder to its record: the variant DSN and the three passes' folders
/// (`fastroute::route_held`). Anything else a run folder holds is left alone.
const RUN_ENTRIES: &[&str] = &["variant.dsn", "full", "fallback", "polish"];

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
/// `routing.fastroute_path`, from the user config file `get_effective_config` and
/// `save_user_config` use (`tools::config`). A missing file or key is `None`; a file
/// that does not parse is an error (the call is refused, as `load_user_config`
/// refuses it).
pub(crate) fn user_routing_config() -> Result<(Option<Value>, Option<PathBuf>)> {
    routing_config_at(&crate::tools::config::user_config_path())
}

/// [`user_routing_config`] for the config file at `path`.
fn routing_config_at(path: &Path) -> Result<(Option<Value>, Option<PathBuf>)> {
    let object = crate::tools::config::read_user_config_object(path)
        .map_err(|reason| anyhow::anyhow!("the user config at {} {reason}", path.display()))?;
    let Some(object) = object else {
        return Ok((None, None));
    };
    let routing = match &object["routing"] {
        Value::Null => return Ok((None, None)),
        Value::Object(routing) => routing,
        _ => anyhow::bail!(
            "the user config at {}: `routing` must be an object",
            path.display()
        ),
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
struct Call {
    dsn: PathBuf,
    ses_output: PathBuf,
    /// `<ses_output with extension "search">`, created by [`prepare_call`].
    folder: PathBuf,
    effort: (Effort, EffortSource),
    fallback: WidthFallback,
}

/// The refusal for a user config that cannot be read for routing.
fn config_refusal(error: &anyhow::Error) -> CallToolResult {
    let reason = format!("{error:#}");
    CallToolResult::error_kind(
        ToolErrorKind::InvalidConfiguration {
            path: crate::tools::config::user_config_path()
                .display()
                .to_string(),
            reason: reason.clone(),
        },
        format!(
            "The user configuration cannot be used for routing ({reason}). Konnect did not substitute defaults and routed nothing. Repair or move the file, or give effort and fastroute_path as arguments."
        ),
    )
}

/// Steps 1 and 2 of [`route_with_fastroute`]: the effort, the DSN, the output and
/// the search folder. Everything that can be refused is refused here, before the
/// folder is created; a returned `Ok(Ok(call))` has created it (empty).
fn prepare_call(args: &Value) -> Result<std::result::Result<Call, CallToolResult>> {
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
///    HOME, <folder>/probe)`; anything but `Found` is an error result carrying
///    [`setup_guidance`]. Nothing has been routed.
/// 4. `route_judge::prepare(dsn, <folder>/judge, ctx.config.kicad_cli)`; a refusal is
///    an error result carrying its reason and next step.
/// 5. The DSN text, `route_search::shapes`, the manifest's hints
///    (`specctra::manifest_routing_hints`), the amps file
///    (`fastroute::amps_file_text`, written to `<folder>/amps.txt`), `width_fallback`
///    (default floor, as `integration::width_arguments` reads it), the plan
///    (`launch_order`, `lanes(available cores, effort)`, `HANG_CAP`).
/// 6. `route_search::search` with a [`FastrouteRunner`].
/// 7. No winner: an error result with every run's record and the baseline's state;
///    nothing is written to `ses_output_path`.
/// 8. The winner's session to `ses_output_path` (`konnect_sexp::write_new_atomic`),
///    `<folder>/search.json` with the whole outcome, then [`reduce_run_folders`].
/// 9. The result ([`result_json`]).
pub(crate) async fn route_with_fastroute(
    args: &Value,
    ctx: Arc<ToolContext>,
) -> Result<CallToolResult> {
    let call = match prepare_call(args)? {
        Ok(call) => call,
        Err(refusal) => return Ok(refusal),
    };
    let located = match locate_binary(args, &call).await {
        Ok(Ok(located)) => located,
        Ok(Err(refusal)) => return finish(&call.folder, Ok(refusal)),
        Err(error) => return finish(&call.folder, Err(error)),
    };
    route_located(args, ctx, call, located).await
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
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let scratch = call.folder.join("probe");
    std::fs::create_dir_all(&scratch)
        .map_err(|error| anyhow::anyhow!("create {}: {error}", scratch.display()))?;
    let located = fastroute::locate(
        argument.as_deref(),
        environment.as_deref(),
        config.as_deref(),
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
async fn route_located(
    args: &Value,
    ctx: Arc<ToolContext>,
    call: Call,
    located: Located,
) -> Result<CallToolResult> {
    let result = match located {
        Located::Found(binary) => search_and_write(args, ctx, &call, binary).await,
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
fn unavailable_result(located: &Located) -> CallToolResult {
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

/// Steps 4 to 9: judge preflight, the DSN's variants, the search, and what
/// follows from its outcome.
async fn search_and_write(
    args: &Value,
    ctx: Arc<ToolContext>,
    call: &Call,
    binary: Binary,
) -> Result<CallToolResult> {
    let judge_root = call.folder.join("judge");
    let mut judge = match route_judge::prepare(&call.dsn, &judge_root, &ctx.config.kicad_cli) {
        Ok(judge) => judge,
        Err(refused) => return Ok(preflight_refusal(&refused)),
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
    let dsn_text = crate::freerouting_mcp::read_dsn(&call.dsn)?;
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
    let runner = Arc::new(FastrouteRunner {
        binary: binary.clone(),
        dsn_text: Arc::new(dsn_text),
        hints,
        amps,
        fallback: call.fallback,
        folder: call.folder.clone(),
        judge: Arc::new(judge),
        tools: ctx,
    });
    let outcome = route_search::search(&plan, runner).await;
    let settings = |search: &mut Value| {
        search["kicad_config_home"] = json!(text(&kicad_config_home));
        if !kicad_config.skipped_links.is_empty() {
            search["kicad_config_skipped_links"] = json!(kicad_config.skipped_links);
        }
    };
    let mut every_run = search_json(&plan, &shapes, &outcome, Listing::Every);
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
        let mut search = search_json(&plan, &shapes, &outcome, Listing::Best);
        settings(&mut search);
        return Ok(no_winner_result(call, &outcome, search));
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
    let reduced = reduce_run_folders(&call.folder, &outcome);
    let mut result = result_json(
        &call.dsn,
        &call.ses_output,
        &call.folder,
        &binary,
        call.effort,
        &plan,
        &shapes,
        &outcome,
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
fn no_winner_result(call: &Call, outcome: &SearchOutcome, mut search: Value) -> CallToolResult {
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
        vec![("search", search)],
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
    pub fallback: WidthFallback,
    /// The search folder: run `i` lives in `run-<i, 3 digits>-<shape>-s<seed>`.
    pub folder: PathBuf,
    pub judge: Arc<JudgeContext>,
    pub tools: Arc<ToolContext>,
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
            let held = fastroute::route_held(
                &binary,
                &folder,
                &dsn,
                &hints,
                amps.as_deref(),
                fallback,
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

/// What [`FastrouteRunner::release`] does: removes `<judge root>/run-<i>` and, in
/// each pass folder of run `i`, `home`, `tmp`, `profile.sb` and `fastroute.log`.
/// A failure is logged, never raised; no link is followed (`remove_tree`).
fn release_run(folder: &Path, judge_root: &Path, index: usize, shape: Shape, seed: u64) {
    let run = folder.join(run_folder_name(index, shape, seed));
    let mut paths = vec![judge_root.join(format!("run-{index}"))];
    // A run or pass folder that is not a real folder (a link, a file) is not
    // ours: `remove_tree` checks only the last part of a path, so a link here
    // would be followed.
    if is_real_folder(&run) {
        for pass in ["full", "fallback", "polish"] {
            let pass = run.join(pass);
            if !is_real_folder(&pass) {
                continue;
            }
            for name in ["home", "tmp", "profile.sb", "fastroute.log"] {
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

/// After a search: every run folder but the winner's is reduced to its
/// `run.json` (its `RunRecord` and, when judged, its measures, gates and
/// rounds), and `<folder>/judge/run-<i>` of every run but the winner's is removed. The
/// winner's run folder and judged board stay whole. Only paths inside `folder` are
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
        let mut body = json!({
            "record": record,
            "key": key,
            "board_kept": is_winner,
        });
        if let Some(routed) = &record.routed {
            body["kept_pass"] = json!(routed.held.kept);
            body["passes"] = json!(routed.held.passes);
            body["planes"] = json!(routed.planes);
        }
        if let Some(judged) = &record.judged {
            body["judged"] = json!(judged);
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
    }
    // The judge's scratch boards of the other runs.
    let judge = folder.join("judge");
    if judge.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
        if let Ok(entries) = std::fs::read_dir(&judge) {
            for entry in entries.flatten() {
                let index = entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.strip_prefix("run-"))
                    .and_then(|number| number.parse::<usize>().ok());
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

/// The runs a listing holds.
fn listed_runs(outcome: &SearchOutcome, listing: Listing) -> Vec<&RunRecord> {
    if listing == Listing::Every {
        return outcome.runs.iter().collect();
    }
    let mut judged: Vec<(crate::route_judge::Key, usize)> = outcome
        .runs
        .iter()
        .enumerate()
        .filter(|(_, record)| record.outcome == "judged")
        .filter_map(|(position, _)| {
            Some((outcome.keys.get(position).copied().flatten()?, position))
        })
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

/// The winner's own part of `search`: its identity, verdict and passes. `None`
/// when no run won, or the winning record lacks what a result needs.
fn winner_json(outcome: &SearchOutcome) -> Option<Value> {
    let position = outcome.winner?;
    let record = outcome.runs.get(position)?;
    let (routed, judged) = (record.routed.as_ref()?, record.judged.as_ref()?);
    Some(json!({
        "index": record.index,
        "shape": record.shape,
        "seed": record.seed,
        "gates": judged.gates,
        "measures": judged.measures,
        "judged_board": judged_board(judged),
        "passes": routed.held.passes,
        "kept": routed.held.kept,
        "key": outcome.keys.get(position).copied().flatten(),
    }))
}

/// The unrouted board's check, as the result reports it.
fn baseline_json(baseline: &std::result::Result<Baseline, String>) -> Value {
    match baseline {
        Ok(baseline) => json!({
            "ok": true,
            "violations": baseline.violations,
            "unconnected": baseline.unconnected,
        }),
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
    if listing == Listing::Best && rows.len() < outcome.runs.len() {
        search["runs_listed"] = json!(format!(
            "{} of {} runs: the {LISTED_RUNS} best judged by the rank key, then up to {LISTED_RUNS} that were not judged; search.json in the search folder lists every run",
            rows.len(),
            outcome.runs.len()
        ));
    }
    search
}

/// The `status` of a fastroute result. The router's own status (from its count and the
/// planes, as `completion_report` reads them) stands only when the winning run passes all
/// three of the judge's gates. A winner the judge found with connections open after the
/// refill and the stitching (G1) is `incomplete`; one that is complete but fails the DRC
/// gate (G2) or the width gate (G3) is `complete_with_gate_failures`, never `complete`.
/// KiCad's DRC on the judged copy sees what the router's count cannot: on 2026-11-16 the
/// router research found fastroute runs that break a net class's larger clearance on one of
/// the owner's boards while reporting nothing open.
pub(crate) fn judged_status(
    router_status: &'static str,
    gates: &crate::route_judge::Gates,
) -> &'static str {
    if !gates.g1.is_empty() {
        "incomplete"
    } else if !gates.g2.is_empty() || !gates.g3.is_empty() {
        "complete_with_gate_failures"
    } else {
        router_status
    }
}

/// The success result. Today's fields, for the winner: `success: true`, `status` and
/// `completion` from `integration::completion_report(dsn, ses_output, the winner's
/// planes, the winner's unrouted, width)`, with `completion.judged` = { unconnected,
/// gates, all_gates_pass, judged_board }; `intent_widths`, `width_exceptions`,
/// `width_exceptions_total` (and `width_unmeasured` when any) from
/// `integration::width_report(dsn, ses_output)`; `method: "local_fastroute_best_of_n"`;
/// `engine` { name: "fastroute", candidate, path, sha256, version, source, switches,
/// threads }; `effort` { level, source, budget_s, runs }; `search` { lanes, shapes,
/// launched, judged, winner { index, shape, seed, gates, measures, passes, kept },
/// runs (every record), elapsed_s, budget_overrun_s, stopped_launching, baseline };
/// `artifacts` { dsn_path, ses_output_path, search_folder, judged_board };
/// `passes` and `result_pass` of the winner, as the Freerouting hold path names them.
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
) -> Value {
    let search = search_json(plan, shapes, outcome, Listing::Best);
    let winner = outcome
        .winner
        .and_then(|position| outcome.runs.get(position))
        .and_then(|record| Some((record.routed.as_ref()?, record.judged.as_ref()?)));
    let Some((routed, judged)) = winner else {
        return json!({
            "success": false,
            "error": "no run was routed and judged",
            "search": search,
        });
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
    completion["judged"] = json!({
        "unconnected": judged.measures.unconnected,
        "gates": judged.gates,
        "all_gates_pass": judged.gates.all_pass(),
        "judged_board": judged_board(judged),
    });
    if completion["judged"]["judged_board"].is_null() {
        completion["judged"]["judged_board_note"] = json!(JUDGED_BOARD_RELEASED);
    }
    let status = judged_status(status, &judged.gates);
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
            "switches": fastroute::CANDIDATE_SWITCHES,
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
