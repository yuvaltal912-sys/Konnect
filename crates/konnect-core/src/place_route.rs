//! A candidate's way to the product's router: its own project copy, a file-based DSN
//! export, one `route_specctra_dsn` best-of-N call, and the call's result read back.
//! Nothing here writes outside the folder it is given, and nothing talks to a running
//! KiCad.

use crate::mcp::protocol::{CallToolResult, ToolContent};
use crate::place_judge::RouteResult;
use crate::route_judge::{Gates, Measures};
use crate::route_search::Effort;
use crate::specctra::ExportOptions;
use crate::tools::ToolContext;
use anyhow::Context;
use konnect_ipc::IpcEffectiveRoutingRules;
use konnect_sexp::board::PcbConnectivityIndex;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What every candidate of one board is routed with. None of it depends on placement.
#[derive(Debug, Clone)]
pub(crate) struct RouteInputs {
    /// The input board inside its stripped project copy (no tracks, arcs, vias or
    /// teardrop zones; `strip_routing`), in the bench folder.
    pub board: PathBuf,
    /// Each net's effective routing rules as KiCad resolved them, once, on the input
    /// board. Net classes follow net names and patterns, not placement.
    pub rules: IpcEffectiveRoutingRules,
    /// As `export_specctra_dsn` builds them (`export_options`).
    pub options: ExportOptions,
    pub effort: Effort,
    /// The fastroute binary, named in every call, so no call reads the user config
    /// for it.
    pub fastroute: PathBuf,
    /// The kicad-cli the unrouted checks run.
    pub kicad_cli: String,
    /// The bench's scratch `KICAD_CONFIG_HOME`.
    pub kicad_config_home: PathBuf,
}

/// Rules from KiCad's own resolution written as JSON: net name to `class_name`,
/// `constituents`, `track_width_mm`, `clearance_mm`, `via_diameter_mm`,
/// `via_drill_mm` (the router research's `net_rules.py` writes this form, from
/// `GetEffectiveNetClass`; the bench's `xtask/placement-bench/net_rules.py` does too).
/// Refuses a net with a pad on `board_text` that has no entry. The refusal's text
/// names the first few nets: it is an error for the bench's log, not a document.
// Used only by the bench (an ignored test): no product code calls the search.
#[allow(dead_code)]
pub(crate) fn rules_from_json(
    text: &str,
    board_text: &str,
) -> anyhow::Result<IpcEffectiveRoutingRules> {
    let rules: IpcEffectiveRoutingRules = serde_json::from_str(text)
        .context("the rules file is not a JSON object of net name to routing rules")?;
    let tree = konnect_sexp::parse_sexp(board_text).context("the board text does not parse")?;
    let index = PcbConnectivityIndex::build(&tree);
    let missing: Vec<&str> = index
        .nets()
        .into_iter()
        .filter(|net| !net.is_empty() && !index.pads_of_net(net).is_empty())
        .filter(|net| !rules.contains_key(*net))
        .collect();
    anyhow::ensure!(
        missing.is_empty(),
        "{} net(s) with a pad have no entry in the rules file (first: {}); make the file again with net_rules.py on this board",
        missing.len(),
        missing
            .iter()
            .take(3)
            .copied()
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(rules)
}

/// The project's rule `key` (mm) beside `board`, in whole micrometres: what
/// `export_specctra_dsn` reads from the `.kicad_pro` for the copper-to-edge and hole
/// clearances. Absent, unreadable or not above zero is `None`.
fn project_clearance_um(board: &Path, key: &str) -> Option<i64> {
    std::fs::read_to_string(board.with_extension("kicad_pro"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|project| project["board"]["design_settings"]["rules"][key].as_f64())
        .filter(|mm| mm.is_finite() && *mm > 0.0)
        .map(|mm| (mm * 1000.0).round() as i64)
}

/// The options `export_specctra_dsn` builds for `board` with its defaults: pours
/// `fixed`, `min_copper_edge_clearance` and `min_hole_clearance` from the
/// `.kicad_pro` beside it, widths from intent on, duplicate references refused.
// Used only by the bench (an ignored test): no product code calls the search.
#[allow(dead_code)]
pub(crate) fn export_options(board: &Path) -> ExportOptions {
    ExportOptions {
        // The handler's `PourPolicy::from_argument(None)`.
        pours: crate::specctra::PourPolicy::default(),
        copper_edge_clearance_um: project_clearance_um(board, "min_copper_edge_clearance"),
        widths_from_intent: true,
        hole_clearance_um: project_clearance_um(board, "min_hole_clearance"),
        allow_duplicate_references: false,
    }
}

/// The byte ranges of the direct children of the root list of `text` that are lists,
/// scanned with quotes (and `\` escapes inside them) respected. Errs on text whose
/// parentheses or quotes do not balance.
fn top_level_blocks(text: &str) -> anyhow::Result<Vec<(usize, usize)>> {
    let bytes = text.as_bytes();
    let mut blocks = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    let (mut in_string, mut escaped) = (false, false);
    for (position, &byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'(' => {
                if depth == 1 {
                    start = position;
                }
                depth += 1;
            }
            b')' => {
                anyhow::ensure!(depth > 0, "a closing parenthesis has no opening one");
                depth -= 1;
                if depth == 1 {
                    blocks.push((start, position + 1));
                }
            }
            _ => {}
        }
    }
    anyhow::ensure!(
        depth == 0 && !in_string,
        "the board text ends inside a list or a string"
    );
    Ok(blocks)
}

/// The word after the opening parenthesis of a block.
fn block_head(block: &str) -> &str {
    let inner = block.strip_prefix('(').unwrap_or(block).trim_start();
    let end = inner
        .find(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '"'))
        .unwrap_or(inner.len());
    &inner[..end]
}

/// The input board with every top-level `(segment …)`, `(arc …)` and `(via …)` and
/// every teardrop zone removed, each with the whitespace before it; every other byte
/// kept. Quote-aware block scanning. Errs on text that does not parse as a board.
// Used only by the bench (an ignored test): no product code calls the search.
#[allow(dead_code)]
pub(crate) fn strip_routing(text: &str) -> anyhow::Result<String> {
    let tree = konnect_sexp::parse_sexp(text).context("the board text does not parse")?;
    anyhow::ensure!(
        tree.head() == Some("kicad_pcb"),
        "the text is not a (kicad_pcb ...) board"
    );
    let mut out = String::with_capacity(text.len());
    let mut kept_from = 0usize;
    for (start, end) in top_level_blocks(text)? {
        let block = &text[start..end];
        let remove = match block_head(block) {
            "segment" | "arc" | "via" => true,
            "zone" => {
                let zone = konnect_sexp::parse_sexp(block)
                    .context("a zone of the board does not parse")?;
                crate::specctra::is_teardrop(&zone)
            }
            _ => false,
        };
        if !remove {
            continue;
        }
        // The whitespace before the block goes with it (never into text already kept).
        let before = text[kept_from..start].trim_end_matches(char::is_whitespace);
        out.push_str(before);
        kept_from = end;
    }
    out.push_str(&text[kept_from..]);
    Ok(out)
}

/// The candidate's own project in `folder` (which must not exist yet):
/// `scratch_board::copy_project` of `inputs.board`, the candidate's text written over
/// the copy's board (same file name), then `<stem>.dsn` and `<stem>.dsn.konnect.json`
/// from `specctra::export_dsn_with_context` with `inputs.rules`, the copy's
/// `routing_context::Snapshot` and `inputs.options`. The manifest names the copy as
/// `board_path` and the text's SHA-256 as `source_sha256`, so `route_judge::prepare`
/// accepts it. Returns the DSN's path; an export refusal is the error, and the folder
/// stays as the failed export left it (no DSN in it), for the record.
pub(crate) fn export_candidate(
    inputs: &RouteInputs,
    folder: &Path,
    board_text: &str,
) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        folder.symlink_metadata().is_err(),
        "the candidate's folder '{}' exists already",
        folder.display()
    );
    let copy = crate::scratch_board::copy_project(&inputs.board, folder)?;
    std::fs::write(&copy, board_text)
        .with_context(|| format!("write the candidate's board '{}'", copy.display()))?;
    // As `export_specctra_dsn` does: the board by its resolved path.
    let copy = std::fs::canonicalize(&copy)
        .with_context(|| format!("resolve the candidate's board '{}'", copy.display()))?;
    let snapshot = crate::routing_context::Snapshot::read(&copy)
        .context("the candidate's project rules could not be captured")?;
    let bundle = crate::specctra::export_dsn_with_context(
        &copy,
        board_text,
        &inputs.rules,
        &snapshot,
        inputs.options.clone(),
    )?;
    let dsn = copy.with_extension("dsn");
    let mut manifest = dsn.as_os_str().to_os_string();
    manifest.push(".konnect.json");
    std::fs::write(&dsn, &bundle.dsn).with_context(|| format!("write '{}'", dsn.display()))?;
    std::fs::write(PathBuf::from(&manifest), &bundle.manifest)
        .with_context(|| format!("write '{}'", Path::new(&manifest).display()))?;
    Ok(dsn)
}

/// One best-of-N routing call: `route_fastroute::route_with_fastroute` with
/// `{dsn_path, ses_output_path: <folder>/route.ses, engine: "fastroute",
/// effort: inputs.effort.name(), fastroute_path: inputs.fastroute}` (the effort as the
/// lower-case name `resolve_effort` reads), read by `read_route_result`. An `Err` from
/// the call is `Failed { stage: "call" }`. A call that succeeds keeps its search folder
/// (`<folder>/route.search`); one that does not has it set aside by the call
/// (`<folder>/route.search.failed-<n>`), and the result's `search_folder` names where,
/// which the record keeps.
pub(crate) async fn route_candidate(
    ctx: Arc<ToolContext>,
    inputs: &RouteInputs,
    dsn: &Path,
    folder: &Path,
) -> RouteResult {
    let failed = |reason: String| RouteResult::Failed {
        stage: "call",
        reason,
    };
    let (Some(dsn_path), Some(ses_path), Some(fastroute)) = (
        dsn.to_str(),
        folder.join("route.ses").to_str().map(str::to_string),
        inputs.fastroute.to_str(),
    ) else {
        return failed("a path of the routing call is not UTF-8 text".to_string());
    };
    let args = routing_arguments(dsn_path, &ses_path, inputs.effort, fastroute);
    match crate::tools::route_fastroute::route_with_fastroute(&args, ctx).await {
        Ok(result) => read_route_result(&result),
        Err(error) => failed(format!("{error:#}")),
    }
}

/// The arguments of one routing call. Split out so a test can pin what is sent
/// without routing.
pub(crate) fn routing_arguments(
    dsn_path: &str,
    ses_output_path: &str,
    effort: Effort,
    fastroute_path: &str,
) -> Value {
    json!({
        "dsn_path": dsn_path,
        "ses_output_path": ses_output_path,
        "engine": "fastroute",
        "effort": effort.name(),
        "fastroute_path": fastroute_path,
    })
}

/// The first text of a result.
fn text_of(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .find_map(|content| match content {
            ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// A violations map (`rule` to count) of a JSON object, `None` when it is not one.
fn counts_of(value: &Value) -> Option<BTreeMap<String, usize>> {
    value
        .as_object()?
        .iter()
        .map(|(rule, count)| Some((rule.clone(), usize::try_from(count.as_u64()?).ok()?)))
        .collect()
}

/// The call's result read back, its `search` object first:
/// - a result (error or not) whose `search.winner` is null is `NoWinner`, with
///   `search.baseline.violations` when `search.baseline.ok` is true. This is how the
///   call reports "no run was routed and judged": an error result that still carries
///   its `search`;
/// - an error result with no `search` is `Failed { stage: "call" }` with its text (a
///   refusal before the search: the binary, the preflight, an argument);
/// - otherwise `Routed`, with the winner's `gates` and `measures` (deserialized into
///   `route_judge`'s own types), `search.baseline.violations`, `status` and the
///   summary.
///
/// Where the call set its search folder aside, the result's `search_folder` note is
/// kept in the reason. A winner without a readable baseline, gates or measures is
/// `Failed { stage: "call" }`: the judge's yardstick is missing.
pub(crate) fn read_route_result(result: &CallToolResult) -> RouteResult {
    let text = text_of(result);
    let failed = |reason: String| RouteResult::Failed {
        stage: "call",
        reason,
    };
    let Ok(body) = serde_json::from_str::<Value>(&text) else {
        return failed(if text.is_empty() {
            "the call's result has no text".to_string()
        } else {
            format!("the call's result is not JSON: {text}")
        });
    };
    let note = body["search_folder"]
        .as_str()
        .map(|note| format!(" ({note})"))
        .unwrap_or_default();
    let reason_of = |fallback: &str| {
        let words = body["message"]
            .as_str()
            .or_else(|| body["error"].as_str())
            .unwrap_or(fallback);
        format!("{words}{note}")
    };
    let search = &body["search"];
    if !search.is_object() {
        return failed(if result.is_error {
            reason_of(&text)
        } else {
            "a successful result carries no search".to_string()
        });
    }
    let baseline_ok = search["baseline"]["ok"].as_bool() == Some(true);
    let baseline = baseline_ok
        .then(|| counts_of(&search["baseline"]["violations"]))
        .flatten();
    if search["winner"].is_null() {
        return RouteResult::NoWinner {
            unrouted: baseline,
            reason: reason_of("no run was routed and judged"),
        };
    }
    let winner = &search["winner"];
    let gates = serde_json::from_value::<Gates>(winner["gates"].clone());
    let measures = serde_json::from_value::<Measures>(winner["measures"].clone());
    let (Ok(gates), Ok(measures), Some(unrouted)) = (gates, measures, baseline) else {
        return failed(format!(
            "the result has a winner whose gates, measures or baseline cannot be read{note}"
        ));
    };
    let mut call = json!({
        "engine": body["engine"],
        "effort": body["effort"],
        "lanes": search["lanes"],
        "launched": search["launched"],
        "judged": search["judged"],
        "outcomes": search["outcomes"],
        "elapsed_s": search["elapsed_s"],
        "budget_overrun_s": search["budget_overrun_s"],
        "winner_shape": winner["shape"],
        "winner_seed": winner["seed"],
        "search_folder": body["artifacts"]["search_folder"],
        // Row DP's fields: whether a pair was passed, and which could not be.
        "differential_pairs": body["differential_pairs"],
        "differential_pairs_unresolved": body["differential_pairs_unresolved"],
    });
    if let Some(note) = body.get("differential_pairs_note") {
        call["differential_pairs_note"] = note.clone();
    }
    RouteResult::Routed {
        gates,
        measures,
        unrouted,
        status: body["status"].as_str().unwrap_or_default().to_string(),
        call,
    }
}

/// The input board's unrouted check: its text in a scratch project under `folder`,
/// then `tools::cli::refill_drc_on_private_copy_with_config` with
/// `inputs.kicad_config_home`, violations counted by type. P2's yardstick.
pub(crate) async fn unrouted_check(
    inputs: &RouteInputs,
    folder: &Path,
) -> anyhow::Result<BTreeMap<String, usize>> {
    let text = std::fs::read_to_string(&inputs.board)
        .with_context(|| format!("read the input board '{}'", inputs.board.display()))?;
    let scratch = folder.join("unrouted");
    anyhow::ensure!(
        scratch.symlink_metadata().is_err(),
        "the unrouted check's folder '{}' exists already",
        scratch.display()
    );
    let copy = crate::scratch_board::copy_project(&inputs.board, &scratch)?;
    std::fs::write(&copy, text)
        .with_context(|| format!("write the scratch board '{}'", copy.display()))?;
    let report = crate::tools::cli::refill_drc_on_private_copy_with_config(
        &inputs.kicad_cli,
        &copy,
        Some(&inputs.kicad_config_home),
    )
    .await
    .context("the input board's refill and DRC failed")?;
    let mut counts = BTreeMap::new();
    for violation in &report.violations {
        *counts.entry(violation.rule.clone()).or_insert(0usize) += 1;
    }
    Ok(counts)
}

/// The tool context every bench call runs with: the default server config with
/// `kicad_cli`, and `ipc_address` set to a socket path inside `folder` that nothing
/// listens on, so every IPC attempt of every tool the judge calls is `Unreachable` and
/// takes its saved-file path. Refuses a `folder` that is not empty or not absolute (a
/// folder that is not there yet is made).
///
/// The router is `ToolRouter::new()`: the judge resolves its tools in the static
/// registry (`route_judge`'s `find_tool`), so no toolset has to be loaded.
// Used only by the bench (an ignored test): no product code calls the search.
#[allow(dead_code)]
pub(crate) fn isolated_context(folder: &Path, kicad_cli: &str) -> anyhow::Result<Arc<ToolContext>> {
    anyhow::ensure!(
        folder.is_absolute(),
        "the isolation folder '{}' is not an absolute path",
        folder.display()
    );
    match folder.symlink_metadata() {
        Ok(meta) => {
            anyhow::ensure!(
                meta.is_dir(),
                "the isolation folder '{}' is not a folder",
                folder.display()
            );
            anyhow::ensure!(
                std::fs::read_dir(folder)
                    .with_context(|| format!("read '{}'", folder.display()))?
                    .next()
                    .is_none(),
                "the isolation folder '{}' is not empty",
                folder.display()
            );
        }
        Err(_) => std::fs::create_dir_all(folder)
            .with_context(|| format!("create the isolation folder '{}'", folder.display()))?,
    }
    let socket = folder.join("no-kicad.sock");
    let address = format!("ipc://{}", socket.display());
    Ok(Arc::new(ToolContext::new(
        crate::tools::ServerConfig {
            kicad_cli: kicad_cli.to_string(),
            kicad_binary: String::new(),
            ipc_address: address,
            project_dir: None,
            jlcpcb_db_path: None,
            auto_load_toolsets: false,
            eager_toolsets: false,
        },
        Arc::new(crate::router::ToolRouter::new()),
    )))
}

#[cfg(test)]
#[path = "place_route_tests.rs"]
mod tests;
