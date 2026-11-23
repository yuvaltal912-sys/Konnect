//! `integration` toolset — JLCPCB parts database, datasheet enrichment, and Freerouting discovery.
//!
//! JLCPCB tools query a local SQLite cache of the JLCPCB parts database.
//! Freerouting discovery locates the JAR and verifies the Java runtime. Konnect
//! does not advertise an autorouter until it has a real PCB-editor bridge.
//! Datasheet enrichment uses the LCSC HTTP API.
//!
//! The three network calls (JLCPCB database download, LCSC datasheet lookups)
//! go through `get_with_backoff`, which retries transient failures (network
//! errors, 429, 5xx) with exponential backoff before giving up.
//!
//! The three JLCPCB query tools (`search_jlcpcb_parts`, `get_jlcpcb_part`,
//! `suggest_jlcpcb_alternatives`) cache results in `ToolContext::jlcpcb_cache`
//! (5-minute TTL) to avoid re-running an identical SQLite query for repeated
//! lookups within a session. Responses carry a `"cached"` field so callers
//! can see whether a given result came from cache.

use crate::mcp::{error::ToolErrorKind, protocol::CallToolResult};
use crate::tool;
use crate::tools::{get_path, lcsc_packages, require_str, ToolContext, ToolDef};
use konnect_sexp::{
    command::{commit_command, prepare_command, ItemId, SchematicCommand},
    parse_sexp,
    writer::{find_direct_child_blocks, read_consistent},
    SexpError, SexpNode,
};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

const JLCPCB_FEED_BASE_URL: &str = "https://bouni.github.io/kicad-jlcpcb-tools";
const JLCPCB_FEED_SENTINEL: &str = "chunk_num_current_parts_fts5.txt";
const JLCPCB_FEED_ARCHIVE: &str = "current-parts-fts5.db.zip";
const JLCPCB_MAX_CHUNKS: usize = 64;
const JLCPCB_MAX_ARCHIVE_BYTES: u64 = 1_073_741_824;
const JLCPCB_MAX_DATABASE_BYTES: u64 = 8_589_934_592;

// ─── Tool definitions ─────────────────────────────────────────────────────────

/// `tool!` for a handler that takes the shared `Arc<ToolContext>` instead of
/// `&ToolContext`: the fastroute engine of `route_specctra_dsn` calls other
/// tools' handlers through it. The name ends in `tool` so that
/// `schema_parameter_usage` finds this registration as it finds a `tool!` one.
macro_rules! arc_tool {
    ($name:expr, $desc:expr, $schema:expr, $handler:expr) => {{
        let h: crate::tools::ToolHandlerFn = std::sync::Arc::new(move |args, ctx| {
            let args = args.clone();
            let ctx = ctx.clone();
            Box::pin(async move { ($handler)(&args, ctx).await })
        });
        crate::tools::ToolDef::new($name, $desc, $schema, h)
    }};
}

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "download_jlcpcb_database",
            "Download or update the local JLCPCB component parts database cache (SQLite).",
            json!({
                "type": "object",
                "properties": {
                    "output_path": { "type": "string", "description": "Local path to store the SQLite database file (optional, uses config default)" },
                    "force": { "type": "boolean", "description": "Force re-download even if cache exists", "default": false }
                },
                "required": []
            }),
            |args, ctx| async move { handle_download_jlcpcb(args, ctx).await }
        ),
        tool!(
            "search_jlcpcb_parts",
            "Search the local JLCPCB component database by keyword, value, or category. The query is split on whitespace and a part matches only if every word appears, in any order, in its LCSC number, MPN, package, manufacturer or description; case-insensitive.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search words, separated by spaces. A part must contain every word, in any order, in its LCSC number, MPN, package, manufacturer or description (case-insensitive; % and _ are literal)" },
                    "category": { "type": "string", "description": "Component category filter (optional)" },
                    "basic_only": { "type": "boolean", "description": "Restrict to JLCPCB Basic Library parts only", "default": false },
                    "in_stock": { "type": "boolean", "description": "Only return parts currently in stock", "default": true },
                    "limit": { "type": "integer", "description": "Maximum number of results", "default": 20 }
                },
                "required": ["query"]
            }),
            |args, ctx| async move { handle_search_jlcpcb_parts(args, ctx).await }
        ),
        tool!(
            "get_jlcpcb_part",
            "Retrieve full details for a single JLCPCB part by its LCSC part number.",
            json!({
                "type": "object",
                "properties": {
                    "lcsc_id": { "type": "string", "description": "LCSC part number (e.g. 'C14663')" }
                },
                "required": ["lcsc_id"]
            }),
            |args, ctx| async move { handle_get_jlcpcb_part(args, ctx).await }
        ),
        tool!(
            "suggest_jlcpcb_alternatives",
            "Suggest JLCPCB-stocked parts with the same value in the package a KiCad footprint uses. The footprint is mapped to LCSC's package names, the value must match as a whole value, and parts below a stock floor are excluded. Results rank Basic, then Preferred, then Extended parts, then known price and stock; the response states the package names matched, the ranking, and what was excluded.",
            json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string", "description": "Component value or part number, matched case-insensitively as a whole value (no digit or '.' directly before or after) against the part description and manufacturer part number, e.g. '100nF', '10k', 'AMS1117-3.3'. Unit spellings are not converted: '100nF' does not find '0.1uF'" },
                    "footprint": { "type": "string", "description": "KiCad footprint ID such as 'Capacitor_SMD:C_0402_1005Metric', or LCSC's own package name without a library prefix, such as '0402' or 'LQFP-48(7x7)'. A library footprint with no known LCSC package is refused" },
                    "max_price_usd": { "type": "number", "description": "Keep only parts whose known unit price is at or below this, in USD (optional). Parts with an unknown price are excluded when it is set" },
                    "min_stock_count": { "type": "integer", "minimum": 0, "description": "Exclude parts with fewer units in stock. Raise it to the build quantity", "default": 100 },
                    "prefer_basic": { "type": "boolean", "description": "Rank JLCPCB Basic parts first, then Preferred, then Extended, before price. Extended parts add a setup fee per unique part", "default": true },
                    "limit": { "type": "integer", "description": "Maximum number of suggestions", "default": 5 }
                },
                "required": ["value", "footprint"]
            }),
            |args, ctx| async move { handle_suggest_alternatives(args, ctx).await }
        ),
        tool!(
            "get_jlcpcb_database_stats",
            "Return statistics about the local JLCPCB database cache: part count, last updated, file size.",
            json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            |args, ctx| async move { handle_jlcpcb_stats(args, ctx).await }
        ),
        tool!(
            "enrich_datasheets",
            "Fetch and cache datasheet URLs for all components in a schematic using the LCSC API.",
            json!({
                "type": "object",
                "properties": {
                    "schematic": { "type": "string", "description": "Path to .kicad_sch file" },
                    "overwrite_existing": { "type": "boolean", "description": "Replace existing Datasheet fields", "default": false }
                },
                "required": ["schematic"]
            }),
            |args, ctx| async move { handle_enrich_datasheets(args, ctx).await }
        ),
        tool!(
            "get_datasheet_url",
            "Retrieve the datasheet URL for a component by MPN or LCSC ID.",
            json!({
                "type": "object",
                "properties": {
                    "mpn": { "type": "string", "description": "Manufacturer part number (optional)" },
                    "lcsc_id": { "type": "string", "description": "LCSC part number (optional)" }
                },
                "required": []
            }),
            |args, ctx| async move { handle_get_datasheet_url(args, ctx).await }
        ),
        tool!(
            "check_freerouting",
            "Locate a Freerouting installation, including KiCad PCM plugin directories, and verify that a Java it can run on is available. Reads the Java version the JAR needs from the JAR itself, finds a Java that meets it, and, when something is missing, returns `setup` with the exact steps (where to get the JAR and which Java to install). `version_policy` names the Freerouting release Konnect recommends, with the measured reason, and warns when the installed JAR is a different one. Nothing is downloaded or installed for you.",
            json!({
                "type": "object",
                "properties": {
                    "jar_path": { "type": "string", "description": freerouting_jar_path_description() },
                    "java_path": { "type": "string", "description": "Path to a Java executable to use (optional). When omitted, Konnect tries JAVA_HOME, java on PATH, then the usual JDK install folders, and picks the first that meets the JAR's requirement." }
                },
                "required": []
            }),
            |args, ctx| async move { handle_check_freerouting(args, ctx).await }
        ),
        arc_tool!(
            "route_specctra_dsn",
            "Route an existing Specctra DSN and create a new SES file; an existing output is never replaced. The default engine is fastroute's best-of-N search: it routes variants of the DSN in parallel for the time `effort` allows (`fast` = 2 runs, `m` = 240 s, `h` = 720 s, `xh` = 2400 s), Konnect judges every run itself on scratch copies of the board (complete, KiCad DRC no worse than the unrouted board, track widths) and writes the best session; every level runs for its whole time (no run cap); `search` counts every outcome and lists the best runs (`search.json` in the search folder lists every run), `completion.judged` the winner's verdict, and `status` is never `complete` of either kind unless the winner passes all four gates (`incomplete` when connections stay open after the refill and stitching, else `complete_with_gate_failures`; the fourth gate checks the design intent's length groups and pair skews, and the board's own KiCad length and skew rules, as KiCad measures them: with a group or skew declared, `length_matching` says what was tuned, measured and not checked, and the next steps give KiCad rules that keep checking it). When every gate passes and the judge added no stitching via, `status` is the judge's, not the router's count (which takes a connection a pour makes for one it left); `completion.router_unrouted` keeps the count. When the winner passes every gate with no stitching via, the ground vias its filled pours make redundant are removed, each with the wires that end at it, when KiCad's refill and DRC, the fill and the judge's own measures find nothing worse; `search.pour_prune` says how many went, why the others stayed and what the winner was before. `engine: \"freerouting\"` routes with the local Freerouting JAR instead (it does not tune lengths: a design intent with a length group or pair skew gets `length_matching_note`); the rest of this description is that engine. It uses the documented session/job state machine and never sends board data to a cloud service. The job settings are read back from Freerouting before it starts, and the run is refused if any was not applied; the result reports them in `effective_settings` and the optimizer's outcome in `optimizer`. `status` is `complete` only when Freerouting left nothing and was given no pour as a plane (`unverified` with a plane or an unreadable count, `incomplete` when it left connections); `completion` gives the router's own count, the planes, what decides (KiCad DRC after a refill) and the exact next calls. `intent_widths` and `width_exceptions` say, from the session itself, which nets with a recorded current ended with track below their intent width, and where; `status` is then `complete_with_width_exceptions`, never plainly `complete`. A wire of such a net that cannot be measured is listed in `width_unmeasured`, and `status` is then never `complete` of either kind (`unverified`, or `incomplete` with open connections, or `stopped`). `width_policy: \"hold\"` first routes with the retry off, so with `allow_neckdown` off (the default) that pass narrows nothing; only if it leaves connections open does a Fallback pass retry at the floor width, and that pass may narrow any track, also track the first pass laid at width: `width_exceptions` is the authority on what ended narrow (`passes` lists each pass; `stopped` when the time ran out and an earlier pass's session is returned).",
            json!({
                "type": "object",
                "properties": {
                    "dsn_path": { "type": "string", "description": "Existing Specctra .dsn input" },
                    "ses_output_path": { "type": "string", "description": "New .ses output path; existing files are never replaced" },
                    "engine": {
                        "type": "string",
                        "enum": ["fastroute", "freerouting"],
                        "default": "fastroute",
                        "description": "Which router. \"fastroute\" (default) runs a timed best-of-N search judged by Konnect; it reads effort and fastroute_path, plus dsn_path, ses_output_path, width_policy \"hold\" and width_fallback, and refuses jar_path, java_path, max_passes, optimizer_enabled, job_timeout_seconds, overall_timeout_seconds, allow_neckdown true and width_policy \"retry\". \"freerouting\" runs the single job or the width_policy passes with the local Freerouting JAR; it reads jar_path, java_path, max_passes, optimizer_enabled, allow_neckdown, width_policy, width_fallback, job_timeout_seconds and overall_timeout_seconds, and refuses effort and fastroute_path."
                    },
                    "effort": {
                        "type": "string",
                        "enum": ["fast", "m", "h", "xh"],
                        "description": "fastroute only. How long the search may run per board: fast = 2 runs at once, m = 240 s, h = 720 s, xh = 2400 s. Default: the user config key routing.effort, else m."
                    },
                    "fastroute_path": {
                        "type": "string",
                        "description": "fastroute only. The fastroute binary. Default: the environment variable KONNECT_FASTROUTE, else the user config key routing.fastroute_path, else a fastroute file next to the konnect executable (a 1 Kopper package), else ~/.local/share/fastroute/candidate/fastroute. Konnect checks that it is the candidate build before routing, with the smart-router switches every routing pass sends (--x.oct_bound, --x.opt_follow_rips=2 and --x.group_search, the group re-route search, which leaves the pour nets out of its cost); a build without them (fastroute x18) is refused."
                    },
                    "jar_path": { "type": "string", "description": freerouting_jar_path_description() },
                    "java_path": { "type": "string", "description": "Path to a Java executable to use (optional); see check_freerouting." },
                    "max_passes": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 },
                    "optimizer_enabled": { "type": "boolean", "default": true },
                    "allow_neckdown": {
                        "type": "boolean",
                        "default": false,
                        "description": "Let Freerouting narrow tracks below their net class to reach small pads. Off by default: the narrowed tracks break the project's minimum width, and the SES importer refuses them."
                    },
                    "width_policy": {
                        "type": "string",
                        "enum": ["retry", "hold"],
                        "description": "Freerouting default \"retry\". With engine \"fastroute\" every run holds, so \"hold\" is accepted and \"retry\" is refused. What to do about nets the export widened for their recorded current (the manifest's intent_widths). \"retry\": one run; a connection that fails at its width is retried at the nets' common floor width, and Freerouting's optimizer may re-lay others at it too. \"hold\": a Full pass with that retry off, which narrows nothing while allow_neckdown is off (the default); only if it leaves connections open, a Fallback pass (retry at the floor, optimizer off), kept only if it closes at least one of them, and then, once nothing is open, a Polish pass (retry off, optimizer on), each started from the pass before it. The Fallback may narrow any track, not only the connections that were open. Either way width_exceptions lists every stretch that ended below its net's intent width. With no such net in the manifest, \"hold\" has nothing to hold and the single run is made."
                    },
                    "width_fallback": {
                        "type": "string",
                        "enum": ["floor", "none"],
                        "default": "floor",
                        "description": "With width_policy \"hold\": what to do with connections the Full pass leaves open. \"floor\": try them in a Fallback pass at the floor width; what ends narrow is listed in width_exceptions. \"none\": leave them open (status incomplete). Without \"hold\" it is not used, and the result says so."
                    },
                    "job_timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 86400 },
                    "overall_timeout_seconds": { "type": "integer", "minimum": 10, "maximum": 86400, "default": 900, "description": "The whole call, every pass of it." }
                },
                "required": ["dsn_path", "ses_output_path"]
            }),
            |args, ctx| async move { handle_route_specctra_dsn(args, ctx).await }
        ),
        // A downloaded part's KiCad files, copied into a project library. Its
        // code lives with the library tools (`library_part_import.rs`).
        super::library::part_import_tool(),
    ]
}

/// Freerouting's automatic neck-down narrows tracks below their class, which
/// breaks the project's minimum and the SES importer refuses; it is off unless
/// the caller asks for it.
fn allow_neckdown(args: &serde_json::Value) -> bool {
    args["allow_neckdown"].as_bool().unwrap_or(false)
}

/// The job settings `route_specctra_dsn` sends. Fanout is always off:
/// Freerouting 2.5.0's fanout stage necks tracks down at pins even with
/// automatic neck-down off, and the SES importer refuses such tracks.
fn route_settings(
    args: &serde_json::Value,
    hints: &crate::specctra::RoutingHints,
    java: PathBuf,
) -> anyhow::Result<crate::freerouting_mcp::RouteSettings> {
    let max_passes = args["max_passes"]
        .as_u64()
        .map(u32::try_from)
        .transpose()
        .map_err(|_| anyhow::anyhow!("max_passes is too large"))?;
    let overall_timeout_seconds = args["overall_timeout_seconds"].as_u64().unwrap_or(900);
    Ok(crate::freerouting_mcp::RouteSettings {
        max_passes: Some(max_passes.unwrap_or(20)),
        optimizer_enabled: Some(args["optimizer_enabled"].as_bool().unwrap_or(true)),
        job_timeout_seconds: args["job_timeout_seconds"].as_u64(),
        poll_interval: std::time::Duration::from_secs(3),
        overall_timeout: std::time::Duration::from_secs(overall_timeout_seconds),
        java: Some(java),
        automatic_neckdown: Some(allow_neckdown(args)),
        copper_to_edge_clearance_um: hints.copper_edge_clearance_um.map(|um| um as f64),
        // With the retry off, 0 is sent rather than nothing, so a stored
        // `neck_width_um` (a `freerouting.json` in the real home) cannot turn
        // it back on; the read-back then checks it.
        retry_width_um: hints
            .retry_width_um
            .map(|um| um as f64)
            .or(hints.retry_off_reason.map(|_| 0.0)),
        fanout_enabled: Some(false),
        // S5: lower via cost, no raised direction penalty, and let the
        // optimizer examine the board before its 0.25% improvement stop.
        via_costs: Some(10),
        default_undesired_direction_trace_cost: Some(1.0),
        optimizer_improvement_threshold: Some(0.25),
        optimizer_max_consecutive_failures_pass1: Some(1_000_000),
        optimizer_max_consecutive_failures: Some(1_000_000),
    })
}

/// What width-first routing (row S9) adds to the status of a routing result:
/// how many stretches of track in the final session are below their net's
/// intent width, and the pass during which the time ran out, if one did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WidthOutcome {
    pub exceptions: usize,
    pub stopped_in: Option<&'static str>,
    /// A net has a recorded width, but the session could not be read for
    /// track widths: what is below intent width is then not known.
    pub unread: bool,
    /// Wires of nets with a recorded width that could not be measured: what
    /// is below intent width is then not fully known.
    pub unmeasured: usize,
    /// The caller's own `width_policy` and `width_fallback`, when given: every
    /// step of the result that routes again repeats them.
    pub policy: Option<&'static str>,
    pub fallback: Option<&'static str>,
}

/// What a routing run can say about completion, from Freerouting's own
/// unrouted count and the planes its DSN held. `complete`: the router left
/// nothing and was given no plane. `incomplete`: it left connections.
/// `unverified`: it left nothing but was given a plane (it counts a pad inside
/// a plane's outline as connected, while the new tracks cut the real pour), or
/// its count could not be read. A run with a plane is never `complete`; KiCad's
/// DRC after a refill is the judge.
///
/// Row S9 adds two: `stopped`, when the time ran out during a pass (it wins
/// over everything, because the session is an earlier pass's), and
/// `complete_with_width_exceptions`, when the router left nothing, was given
/// no plane, and some track is below its net's intent width. With a plane the
/// status stays `unverified` and the exceptions are listed beside it. A
/// session that could not be read for widths, or that holds a wire of such a
/// net that could not be measured, is `unverified` too: never "complete" of
/// either kind.
fn routing_status(
    planes: &[crate::specctra::DsnPlane],
    router_unrouted: Option<u32>,
    width: &WidthOutcome,
) -> &'static str {
    if width.stopped_in.is_some() {
        return "stopped";
    }
    match router_unrouted {
        Some(left) if left > 0 => "incomplete",
        // Widths that could not be read, or a wire that could not be
        // measured, are not read as "nothing is narrow".
        Some(_) if planes.is_empty() && (width.unread || width.unmeasured > 0) => "unverified",
        Some(_) if planes.is_empty() && width.exceptions > 0 => "complete_with_width_exceptions",
        Some(_) if planes.is_empty() => "complete",
        _ => "unverified",
    }
}

/// Above this many connections still open on plane nets after stitching, the
/// steps fall back to leaving the pours out (S5's rule: "about 10").
const PLANE_FALLBACK_OPEN: u32 = 10;
/// Stitching rounds the steps ask for: S7's planner listed nothing more after
/// four rounds on every board it was measured on.
const STITCH_ROUNDS: u32 = 4;
/// The planner's own default search radius, sent explicitly.
const STITCH_RADIUS_MM: f64 = 3.0;

/// The most of an error's text a completion report repeats: a parse error
/// can carry the unparsed rest of the file, which must not come back whole.
const MAX_REASON_DETAIL: usize = 160;

/// `status` and `completion` of a routing result: what Freerouting's own
/// count shows, which pours it was given as planes (from the DSN, named from
/// the manifest beside it), what decides completion, and the exact calls
/// still needed. Additive: `success` keeps meaning that the job ran and the
/// SES was written.
///
/// With planes, the steps first keep the board as it was exported (save it,
/// then `create_checkpoint`), because the fallback has to export it again and
/// the exporter refuses a board that holds unlocked routing: after the import
/// and the stitching rounds it does.
pub(crate) fn completion_report(
    dsn: &Path,
    ses: &Path,
    planes: &anyhow::Result<Vec<crate::specctra::DsnPlane>>,
    router_unrouted: Option<u32>,
    width: &WidthOutcome,
) -> (&'static str, serde_json::Value) {
    let manifest = crate::specctra::manifest_beside(dsn);
    // The board the steps name: the manifest's, when it is an existing board
    // file; anything else is not repeated into a call.
    let board = manifest
        .as_ref()
        .and_then(|manifest| manifest["board_path"].as_str())
        .map(PathBuf::from)
        .filter(|board| board.is_file() && board.extension().is_some_and(|e| e == "kicad_pcb"));
    let manifest_path = {
        let mut name = dsn.as_os_str().to_os_string();
        name.push(".konnect.json");
        PathBuf::from(name)
    };
    let known: &[crate::specctra::DsnPlane] = planes.as_deref().unwrap_or(&[]);
    // A router that left connections is incomplete whatever else is known.
    let status = match (planes, router_unrouted) {
        _ if width.stopped_in.is_some() => "stopped",
        (_, Some(left)) if left > 0 => "incomplete",
        (Ok(planes), _) => routing_status(planes, router_unrouted, width),
        (Err(_), _) => "unverified",
    };
    let mut plane_nets: Vec<String> = known.iter().map(|plane| plane.net.clone()).collect();
    plane_nets.dedup();
    let plane_rows: Vec<serde_json::Value> = known
        .iter()
        .map(|plane| {
            let zones: Vec<serde_json::Value> = manifest
                .as_ref()
                .and_then(|manifest| manifest["pours"].as_array())
                .into_iter()
                .flatten()
                .filter(|row| {
                    row["policy"] == "fixed"
                        && row["net"] == plane.net
                        && row["layer"] == plane.layer
                })
                .map(|row| json!({ "uuid": row["uuid"], "name": row["name"] }))
                .collect();
            json!({ "net": plane.net, "layer": plane.layer, "zones": zones })
        })
        .collect();
    let reason = match (planes, router_unrouted) {
        (_, Some(left)) if left > 0 => format!(
            "Freerouting reports {left} connection(s) it could not route; the SES holds a partial routing."
        ),
        (Err(error), _) => {
            let detail: String = format!("{error:#}").chars().take(MAX_REASON_DETAIL).collect();
            format!(
                "The DSN's planes could not be read ({detail}), so it is not known whether the router was given pours as planes."
            )
        }
        (Ok(planes), _) if !planes.is_empty() => format!(
            "Freerouting was given pours as planes ({}). It counts every pad inside a plane's outline as connected and lets other nets' tracks cross the plane, so after a refill the real pour can be cut into pieces that leave pads open. Its own count cannot show that the plane nets are connected.",
            planes
                .iter()
                .map(|plane| format!("{} on {}", plane.net, plane.layer))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        (_, None) => "Freerouting's job log gave no unrouted count, so the router's own result is not known.".to_string(),
        _ => "Freerouting reports 0 unrouted connections and was given no pour as a plane.".to_string(),
    };
    // Row S9: a pass that ran out of time, and track below intent width, are
    // said beside what the router's own count shows.
    let mut reason = match width.stopped_in {
        Some(pass) => format!(
            "The call's time ran out at the {pass} pass (before it started or while it ran: its row in `passes` says which), so the session is the one an earlier pass finished. {reason}"
        ),
        None => reason,
    };
    if width.exceptions > 0 {
        reason.push_str(&format!(
            " {} stretch(es) of track are below their net's intent width; `width_exceptions` lists each one.",
            width.exceptions
        ));
    }
    if width.unread {
        reason.push_str(" The session could not be read for track widths, so track below a net's intent width is not known (`width_report_error`).");
    }
    if width.unmeasured > 0 {
        reason.push_str(&format!(
            " {} wire(s) of nets with a recorded width could not be measured (`width_unmeasured`), so track below a net's intent width is not fully known.",
            width.unmeasured
        ));
    }
    let nets_text = if plane_nets.is_empty() {
        "every net".to_string()
    } else {
        format!(
            "the plane nets ({}) and every other net",
            plane_nets.join(", ")
        )
    };
    let complete_when = format!(
        "The routing is complete only when KiCad's DRC on the applied and refilled board reports 0 unconnected items, on {nets_text}. Freerouting's count, and this result's status, do not show that."
    );
    let mut steps = Vec::new();
    let mut fallback = Vec::new();
    let with_planes = !plane_nets.is_empty();
    if let Some(board) = &board {
        let project = board.with_extension("kicad_pro");
        let drc = json!({ "board": board, "sync_live_board": true, "refill_zones": true });
        fn push(steps: &mut Vec<serde_json::Value>, mut call: serde_json::Value) -> usize {
            call["step"] = json!(steps.len() + 1);
            steps.push(call);
            steps.len()
        }
        let mut checkpoint_step = None;
        if with_planes {
            push(
                &mut steps,
                json!({
                    "tool": "save_project", "arguments": {},
                    "why": "Save the open board, so the saved file is the revision the DSN was exported from. KiCad may also rewrite the project file here, if it had not saved it itself before the export: plan_specctra_ses_import then reports a stale revision, and the recovery is to export and route again. Saving the project before export_specctra_dsn avoids that."
                }),
            );
            checkpoint_step = Some(push(
                &mut steps,
                json!({
                    "tool": "create_checkpoint",
                    "arguments": { "project": project, "label": "before importing the plane routing" },
                    "why": "Keep the board as it was exported: the fallback needs it back, and only a checkpoint restores it. Keep the checkpoint_id it returns. If it fails, stop here: without it the fallback cannot put the board back."
                }),
            ));
        }
        push(
            &mut steps,
            json!({
                "tool": "plan_specctra_ses_import",
                "arguments": { "board": board, "ses_path": ses, "manifest_path": manifest_path },
                "why": "Check the session against the manifest and the live board before anything is written."
            }),
        );
        push(
            &mut steps,
            json!({
                "tool": "apply_specctra_ses",
                "arguments": { "board": board, "ses_path": ses, "manifest_path": manifest_path,
                               "candidate_output_path": ses.with_extension("candidate.kicad_pcb") },
                "why": "Write the routing into the live board as one undo step."
            }),
        );
        push(
            &mut steps,
            json!({
                "tool": "run_drc", "arguments": drc,
                "why": "Refill the zones, save, and read KiCad's unconnected items: this, not the router's count, says what is still open."
            }),
        );
        if let Some(checkpoint) = checkpoint_step {
            let plan = push(
                &mut steps,
                json!({
                    "tool": "plan_route_refinement",
                    "arguments": { "board": board, "kinds": ["stitching"], "nets": plane_nets,
                                   "search_radius_mm": STITCH_RADIUS_MM },
                    "why": "Ask the read-only planner for stitching vias that join the cut-off pieces of the plane nets' pours. Send the effort chosen for this task (the planner's default is standard)."
                }),
            );
            push(
                &mut steps,
                json!({
                    "tool": "route_batch",
                    "arguments": { "board": board },
                    "arguments_from": { "vias": "the route_batch operation of each stitching proposal you adopt" },
                    "why": "Adopt the stitching vias you support."
                }),
            );
            let check = push(
                &mut steps,
                json!({
                    "tool": "run_drc", "arguments": drc,
                    "why": "Refill, save and count the unconnected items again."
                }),
            );
            steps.last_mut().unwrap()["repeat"] = json!(format!(
                "Repeat steps {plan} to {check} until the planner lists no stitching proposal or DRC reports 0 unconnected items on the plane nets, at most {STITCH_ROUNDS} rounds."
            ));
            let from_checkpoint =
                format!("the checkpoint_id create_checkpoint returned in step {checkpoint}");
            fallback.push(json!({
                "manual": "Close the board and the project in KiCad. The routed board is already saved (run_drc saved it); restore_checkpoint refuses while KiCad has the project open."
            }));
            fallback.push(json!({
                "tool": "restore_checkpoint",
                "arguments": { "project": project, "dry_run": true },
                "arguments_from": { "checkpoint_id": from_checkpoint },
                "why": "Preview the restore. It lists every design file the checkpoint puts back, the board among them."
            }));
            fallback.push(json!({
                "tool": "restore_checkpoint",
                "arguments": { "project": project, "dry_run": false },
                "arguments_from": { "checkpoint_id": from_checkpoint, "expected_revision": "the revision the preview returned" },
                "why": "Put those files back, the board as it was exported among them. It takes its own undo checkpoint first."
            }));
            fallback.push(json!({
                "tool": "launch_kicad_ui", "arguments": { "project": project },
                "why": "Open the project again."
            }));
            fallback.push(json!({
                "manual": "Open the board in KiCad's PCB editor: export_specctra_dsn needs it open, and alone."
            }));
            fallback.push(json!({
                "tool": "export_specctra_dsn",
                "arguments": { "board": board, "output": dsn.with_extension("pours-omitted.dsn"), "pours": "omit" },
                "why": "Leave the pours out, so the router routes the plane nets as track. Pass the same other arguments as the first export (widths_from_intent, native_bridge_mode)."
            }));
            // The caller's own width arguments are repeated, so following
            // the steps cannot bring back a default the caller did not choose.
            // `engine` is named because fastroute is the default engine: this report
            // is the Freerouting engine's, and its step must route the same way. The
            // fastroute result drops the key again (`route_fastroute::carry_arguments`).
            let mut reroute = json!({ "dsn_path": dsn.with_extension("pours-omitted.dsn"),
                                      "ses_output_path": ses.with_extension("pours-omitted.ses"),
                                      "engine": "freerouting" });
            let mut carried = false;
            for (key, value) in [
                ("width_policy", width.policy),
                ("width_fallback", width.fallback),
            ] {
                if let Some(value) = value {
                    reroute[key] = json!(value);
                    carried = true;
                }
            }
            fallback.push(json!({
                "tool": "route_specctra_dsn",
                "arguments": reroute,
                "why": if carried {
                    "Route that DSN with the width arguments this call was given (width_policy, width_fallback), then follow its own next steps."
                } else {
                    "Route that DSN, then follow its own next steps."
                }
            }));
        }
    }
    let mut completion = json!({
        "router_unrouted": router_unrouted,
        "router_unrouted_source": "Freerouting's job log (the last autorouter or optimizer summary)",
        "planes": plane_rows,
        "planes_source": if planes.is_ok() { "the DSN; zones named from the manifest beside it" } else { "unreadable DSN" },
        "judge": "KiCad DRC after a refill (run_drc with sync_live_board and refill_zones)",
        "complete_when": complete_when,
        "reason": reason,
        "next_steps": steps,
    });
    if board.is_none() {
        completion["next_steps_missing"] = json!(format!(
            "No reverse manifest naming an existing board sits at {}, and apply_specctra_ses needs one. Route a DSN whose manifest is beside it under that name (export_specctra_dsn writes it there unless manifest_output_path moved it).",
            manifest_path.display()
        ));
    }
    if with_planes && board.is_some() {
        completion["if_still_open"] = json!({
            "fallback_threshold": PLANE_FALLBACK_OPEN,
            "one_to_ten": "If 1 to 10 connections on the plane nets are still open after the stitching rounds, report every one of them as open: the routing is not complete. Name each open pad and net.",
            "more_than_ten": "If more than 10 are still open, fall back: run the steps in `fallback` and report that outcome, with the open count of this plane run beside it.",
            "precondition": "The fallback exports the board again, and export_specctra_dsn refuses a board that holds unlocked routing, which this board does after the import and the stitching rounds. So the fallback first puts back the board from the checkpoint the steps took before the import (restore_checkpoint), with KiCad closed.",
            "fallback": fallback,
        });
    }
    (status, completion)
}

#[cfg(test)]
mod route_rule_tests {
    use super::{allow_neckdown, route_settings, routing_status, WidthOutcome};
    use serde_json::json;

    #[test]
    fn a_run_with_planes_is_never_reported_complete() {
        let plane = crate::specctra::DsnPlane {
            net: "GND".into(),
            layer: "B.Cu".into(),
        };
        let one = std::slice::from_ref(&plane);
        assert_eq!(
            routing_status(&[], Some(0), &WidthOutcome::default()),
            "complete"
        );
        assert_eq!(
            routing_status(one, Some(0), &WidthOutcome::default()),
            "unverified"
        );
        assert_eq!(
            routing_status(one, Some(2), &WidthOutcome::default()),
            "incomplete"
        );
        assert_eq!(
            routing_status(&[], Some(2), &WidthOutcome::default()),
            "incomplete"
        );
        // An unreadable count is never read as complete.
        assert_eq!(
            routing_status(&[], None, &WidthOutcome::default()),
            "unverified"
        );
        assert_eq!(
            routing_status(one, None, &WidthOutcome::default()),
            "unverified"
        );
    }

    // ─── S9: width exceptions and a stopped run in the status ──────────────

    fn narrowed(exceptions: usize) -> WidthOutcome {
        WidthOutcome {
            exceptions,
            ..Default::default()
        }
    }

    fn gnd_plane() -> crate::specctra::DsnPlane {
        crate::specctra::DsnPlane {
            net: "GND".into(),
            layer: "B.Cu".into(),
        }
    }

    /// Track below a net's intent width is never hidden behind `complete`,
    /// and S6's rule stays: with a plane no "complete" status is returned.
    #[test]
    fn a_run_with_width_exceptions_is_never_plainly_complete() {
        let plane = gnd_plane();
        let one = std::slice::from_ref(&plane);
        assert_eq!(
            routing_status(&[], Some(0), &narrowed(1)),
            "complete_with_width_exceptions"
        );
        assert_eq!(
            routing_status(&[], Some(0), &narrowed(7)),
            "complete_with_width_exceptions"
        );
        assert_eq!(routing_status(&[], Some(0), &narrowed(0)), "complete");
        assert_eq!(routing_status(one, Some(0), &narrowed(2)), "unverified");
        assert_eq!(routing_status(&[], None, &narrowed(2)), "unverified");
        assert_eq!(routing_status(&[], Some(3), &narrowed(2)), "incomplete");
        assert_eq!(routing_status(one, Some(3), &narrowed(2)), "incomplete");
    }

    /// A call that ran out of time says so whatever else is known: the
    /// session it returns is an earlier pass's.
    #[test]
    fn a_run_that_ran_out_of_time_is_stopped_whatever_else_is_known() {
        let plane = gnd_plane();
        let one = std::slice::from_ref(&plane);
        let none: &[crate::specctra::DsnPlane] = &[];
        for exceptions in [0, 2] {
            let stopped = WidthOutcome {
                exceptions,
                stopped_in: Some("polish"),
                unread: exceptions > 0,
                unmeasured: exceptions,
                ..Default::default()
            };
            for planes in [none, one] {
                for unrouted in [Some(0), Some(3), None] {
                    assert_eq!(
                        routing_status(planes, unrouted, &stopped),
                        "stopped",
                        "{} plane(s), unrouted {unrouted:?}, {exceptions} exception(s)",
                        planes.len()
                    );
                }
            }
        }
    }

    /// Widths that could not be read are not read as "nothing is narrow".
    #[test]
    fn a_session_that_could_not_be_read_for_widths_is_never_complete() {
        let unread = WidthOutcome {
            unread: true,
            ..Default::default()
        };
        assert_eq!(routing_status(&[], Some(0), &unread), "unverified");
        assert_eq!(routing_status(&[], Some(2), &unread), "incomplete");
        let dir = tempfile::tempdir().unwrap();
        let (dsn, ses) = exported(dir.path(), json!("omit"));
        let planes = crate::specctra::dsn_planes(&std::fs::read_to_string(&dsn).unwrap());
        let (status, completion) = super::completion_report(&dsn, &ses, &planes, Some(0), &unread);
        assert_eq!(status, "unverified");
        let reason = completion["reason"].as_str().unwrap();
        assert!(
            reason.contains("could not be read for track widths"),
            "{reason}"
        );
        assert!(reason.contains("width_report_error"), "{reason}");
    }

    #[test]
    fn the_completion_report_names_width_exceptions_and_a_stopped_pass() {
        let dir = tempfile::tempdir().unwrap();
        let (dsn, ses) = exported(dir.path(), json!("omit"));
        let planes = crate::specctra::dsn_planes(&std::fs::read_to_string(&dsn).unwrap());
        let report = |unrouted, width: WidthOutcome| {
            super::completion_report(&dsn, &ses, &planes, unrouted, &width)
        };
        let (status, plain) = report(Some(0), narrowed(0));
        assert_eq!(status, "complete");
        assert!(!plain["reason"].as_str().unwrap().contains("intent width"));

        let (status, completion) = report(Some(0), narrowed(3));
        assert_eq!(status, "complete_with_width_exceptions");
        let reason = completion["reason"].as_str().unwrap();
        assert!(reason.contains("0 unrouted connections"), "{reason}");
        assert!(
            reason.contains("3 stretch(es) of track are below their net's intent width"),
            "{reason}"
        );
        assert!(reason.contains("width_exceptions"), "{reason}");
        // What decides completion does not change.
        assert_eq!(completion["complete_when"], plain["complete_when"]);
        assert_eq!(completion["next_steps"], plain["next_steps"]);

        // A router that left connections is incomplete; the exceptions are
        // still named.
        let (status, left) = report(Some(2), narrowed(3));
        assert_eq!(status, "incomplete");
        let reason = left["reason"].as_str().unwrap();
        assert!(
            reason.contains("2 connection(s) it could not route"),
            "{reason}"
        );
        assert!(reason.contains("3 stretch(es) of track"), "{reason}");

        let stopped = WidthOutcome {
            stopped_in: Some("polish"),
            ..Default::default()
        };
        let (status, completion) = report(Some(0), stopped);
        assert_eq!(status, "stopped");
        let reason = completion["reason"].as_str().unwrap();
        assert!(
            reason.contains("The call's time ran out at the polish pass"),
            "{reason}"
        );
        assert!(reason.contains("an earlier pass"), "{reason}");
    }

    /// With a plane the status stays `unverified` and the plane reason stays;
    /// the exceptions are named beside it.
    #[test]
    fn with_a_plane_the_exceptions_are_named_but_the_status_stays_unverified() {
        let dir = tempfile::tempdir().unwrap();
        let (dsn, ses) = exported(dir.path(), json!("fixed"));
        let planes = crate::specctra::dsn_planes(&std::fs::read_to_string(&dsn).unwrap());
        let (status, completion) =
            super::completion_report(&dsn, &ses, &planes, Some(0), &narrowed(4));
        assert_eq!(status, "unverified");
        let reason = completion["reason"].as_str().unwrap();
        assert!(reason.contains("was given pours as planes"), "{reason}");
        assert!(reason.contains("4 stretch(es) of track"), "{reason}");
        // Unreadable planes: the same.
        let broken: anyhow::Result<Vec<crate::specctra::DsnPlane>> =
            Err(anyhow::anyhow!("not a DSN"));
        let (status, completion) =
            super::completion_report(&dsn, &ses, &broken, Some(0), &narrowed(4));
        assert_eq!(status, "unverified");
        assert!(completion["reason"]
            .as_str()
            .unwrap()
            .contains("4 stretch(es) of track"));
    }

    /// Review A, S1: a wire of a net with a recorded width that could not be
    /// measured is never behind a "complete" of either kind.
    #[test]
    fn a_run_with_an_unmeasured_wire_is_never_complete() {
        let plane = gnd_plane();
        let one = std::slice::from_ref(&plane);
        for exceptions in [0, 3] {
            let unmeasured = WidthOutcome {
                exceptions,
                unmeasured: 2,
                ..Default::default()
            };
            assert_eq!(
                routing_status(&[], Some(0), &unmeasured),
                "unverified",
                "{exceptions} exception(s)"
            );
            assert_eq!(routing_status(one, Some(0), &unmeasured), "unverified");
            assert_eq!(routing_status(&[], Some(1), &unmeasured), "incomplete");
            assert_eq!(routing_status(&[], None, &unmeasured), "unverified");
        }
        let stopped = WidthOutcome {
            unmeasured: 2,
            stopped_in: Some("polish"),
            ..Default::default()
        };
        assert_eq!(routing_status(&[], Some(0), &stopped), "stopped");
        let dir = tempfile::tempdir().unwrap();
        let (dsn, ses) = exported(dir.path(), json!("omit"));
        let planes = crate::specctra::dsn_planes(&std::fs::read_to_string(&dsn).unwrap());
        let width = WidthOutcome {
            exceptions: 3,
            unmeasured: 2,
            ..Default::default()
        };
        let (status, completion) = super::completion_report(&dsn, &ses, &planes, Some(0), &width);
        assert_eq!(status, "unverified");
        let reason = completion["reason"].as_str().unwrap();
        assert!(
            reason.contains("2 wire(s) of nets with a recorded width could not be measured"),
            "{reason}"
        );
        assert!(reason.contains("width_unmeasured"), "{reason}");
        assert!(reason.contains("3 stretch(es) of track"), "{reason}");
    }

    /// Review A, S3: every step that routes again repeats the caller's own
    /// width arguments, so following the steps cannot bring a default back
    /// that the caller did not choose. The arguments pass the tool's schema.
    #[test]
    fn a_step_that_routes_again_carries_the_callers_width_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let (dsn, ses) = exported(dir.path(), json!("fixed"));
        let planes = crate::specctra::dsn_planes(&std::fs::read_to_string(&dsn).unwrap());
        let reroutes = |width: WidthOutcome| -> Vec<serde_json::Value> {
            let (_, completion) = super::completion_report(&dsn, &ses, &planes, Some(0), &width);
            let steps = completion["next_steps"].as_array().unwrap();
            let fallback = completion["if_still_open"]["fallback"].as_array().unwrap();
            assert_calls_are_real(steps);
            assert_calls_are_real(fallback);
            steps
                .iter()
                .chain(fallback)
                .filter(|call| call["tool"] == "route_specctra_dsn")
                .map(|call| call["arguments"].clone())
                .collect()
        };
        let plain = reroutes(WidthOutcome::default());
        assert_eq!(plain.len(), 1, "the plane fallback routes again");
        // The report is the Freerouting engine's, so its step names that engine
        // (fastroute is the default); nothing else is added.
        let mut keys = plain[0].as_object().unwrap().keys().collect::<Vec<_>>();
        keys.sort();
        assert_eq!(keys, ["dsn_path", "engine", "ses_output_path"]);
        assert_eq!(plain[0]["engine"], "freerouting");
        for (policy, fallback) in [
            (Some("hold"), Some("none")),
            (Some("hold"), Some("floor")),
            (Some("hold"), None),
            (Some("retry"), None),
            (None, Some("floor")),
        ] {
            let calls = reroutes(WidthOutcome {
                policy,
                fallback,
                ..Default::default()
            });
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0]["width_policy"].as_str(), policy);
            assert_eq!(calls[0]["width_fallback"].as_str(), fallback);
            assert_eq!(calls[0]["dsn_path"], plain[0]["dsn_path"]);
            assert_eq!(calls[0]["ses_output_path"], plain[0]["ses_output_path"]);
            // The step's arguments route with Freerouting, "retry" included; without
            // `engine` they would resolve to fastroute (or be refused).
            assert_eq!(
                crate::tools::route_fastroute::route_engine(&calls[0]).ok(),
                Some(crate::tools::route_fastroute::Engine::Freerouting),
                "{:?}",
                calls[0]
            );
            let mut without = calls[0].clone();
            without.as_object_mut().unwrap().remove("engine");
            assert_ne!(
                crate::tools::route_fastroute::route_engine(&without).ok(),
                Some(crate::tools::route_fastroute::Engine::Freerouting)
            );
        }
    }

    // ─── S6 second half: the completion report and its next steps ──────────

    const POLICY: &str = include_str!("../../tests/fixtures/specctra_pours_policy.kicad_pcb");

    fn rules() -> konnect_ipc::IpcEffectiveRoutingRules {
        ["GND", "VCC"]
            .into_iter()
            .map(|net| {
                (
                    net.to_string(),
                    konnect_ipc::IpcRoutingRules {
                        class_name: "Default".into(),
                        constituents: vec!["Default".into()],
                        track_width_mm: Some(0.25),
                        clearance_mm: Some(0.2),
                        via_diameter_mm: Some(0.6),
                        via_drill_mm: Some(0.3),
                    },
                )
            })
            .collect()
    }

    /// A real export of the pour fixture with `pours`, its DSN and manifest
    /// side by side, as `export_specctra_dsn` leaves them.
    fn exported(
        dir: &std::path::Path,
        pours: serde_json::Value,
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        let board = dir.join("board.kicad_pcb");
        std::fs::write(&board, POLICY).unwrap();
        let bundle = crate::specctra::export_dsn_with_options(
            &board,
            POLICY,
            &rules(),
            crate::specctra::ExportOptions {
                pours: crate::specctra::PourPolicy::from_argument(Some(&pours)).unwrap(),
                ..Default::default()
            },
        )
        .unwrap();
        let dsn = dir.join("board.dsn");
        std::fs::write(&dsn, &bundle.dsn).unwrap();
        std::fs::write(dir.join("board.dsn.konnect.json"), &bundle.manifest).unwrap();
        (dsn, dir.join("board.ses"))
    }

    fn report(
        dsn: &std::path::Path,
        ses: &std::path::Path,
        unrouted: Option<u32>,
    ) -> (&'static str, serde_json::Value) {
        let planes = crate::specctra::dsn_planes(&std::fs::read_to_string(dsn).unwrap());
        super::completion_report(dsn, ses, &planes, unrouted, &WidthOutcome::default())
    }

    fn registered(name: &str) -> Option<crate::tools::ToolDef> {
        crate::router::registry::ALL_TOOLSETS
            .iter()
            .filter_map(|toolset| crate::router::registry::tools_for(toolset.name))
            .flatten()
            .find(|tool| tool.name == name)
    }

    /// Every call the steps name is a registered tool, its arguments pass
    /// that tool's own input schema, and every parameter they mention (the
    /// ones filled from the plan too) is one the tool declares. A renamed
    /// tool or parameter breaks this test, not a caller.
    fn assert_calls_are_real(calls: &[serde_json::Value]) {
        assert!(!calls.is_empty());
        for call in calls {
            // A step a person does in KiCad names no tool, and says what to do.
            if let Some(manual) = call.get("manual") {
                assert!(call.get("tool").is_none(), "{call}");
                assert!(
                    manual.as_str().is_some_and(|text| text.len() > 20),
                    "{call}"
                );
                continue;
            }
            let name = call["tool"].as_str().unwrap();
            let tool =
                registered(name).unwrap_or_else(|| panic!("{name} is not a registered tool"));
            let properties = tool.input_schema["properties"].as_object().unwrap();
            let arguments = &call["arguments"];
            for key in arguments.as_object().unwrap().keys() {
                assert!(properties.contains_key(key), "{name} declares no '{key}'");
            }
            for key in call["arguments_from"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, _)| k)
            {
                assert!(properties.contains_key(key), "{name} declares no '{key}'");
            }
            if call["arguments_from"].is_null() {
                assert!(
                    tool.input_validator.is_valid(arguments),
                    "{name} refuses {arguments}"
                );
            }
        }
    }

    /// The snake_case words in the prose fields are registered tools, or
    /// parameters of a registered tool, or fields of this result: a renamed
    /// tool breaks this test, not a caller.
    fn assert_prose_names_are_real(completion: &serde_json::Value) {
        let tools: Vec<crate::tools::ToolDef> = crate::router::registry::ALL_TOOLSETS
            .iter()
            .filter_map(|toolset| crate::router::registry::tools_for(toolset.name))
            .flatten()
            .collect();
        let known = |word: &str| {
            tools.iter().any(|tool| {
                tool.name == word
                    || tool.input_schema["properties"]
                        .as_object()
                        .is_some_and(|p| p.contains_key(word))
            }) || ["next_steps", "if_still_open", "fallback"].contains(&word)
        };
        let mut texts = vec![
            completion["judge"].clone(),
            completion["complete_when"].clone(),
            completion["reason"].clone(),
            completion["next_steps_missing"].clone(),
        ];
        if let Some(open) = completion["if_still_open"].as_object() {
            texts.extend(open.values().filter(|v| v.is_string()).cloned());
        }
        for call in completion["next_steps"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(
                completion["if_still_open"]["fallback"]
                    .as_array()
                    .into_iter()
                    .flatten(),
            )
        {
            for key in ["why", "manual", "repeat"] {
                texts.push(call[key].clone());
            }
            texts.extend(
                call["arguments_from"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(_, v)| v.clone()),
            );
        }
        for text in texts.iter().filter_map(serde_json::Value::as_str) {
            for word in text
                .split(|c: char| !(c.is_ascii_lowercase() || c == '_'))
                .filter(|w| w.contains('_') && !w.starts_with('_') && !w.ends_with('_'))
            {
                assert!(
                    known(word),
                    "'{word}' is no registered tool or parameter: {text}"
                );
            }
        }
    }

    #[test]
    fn a_run_with_planes_lists_the_stitching_sequence_with_real_tools() {
        let dir = tempfile::tempdir().unwrap();
        let (dsn, ses) = exported(
            dir.path(),
            json!({ "zones": [{ "net": "GND", "layer": "F.Cu", "policy": "omit" }] }),
        );
        let (status, completion) = report(&dsn, &ses, Some(0));
        assert_eq!(status, "unverified");
        assert_eq!(completion["router_unrouted"], 0);
        // The planes, named from the manifest.
        let planes = completion["planes"].as_array().unwrap();
        assert_eq!(planes.len(), 2);
        assert_eq!(planes[0]["net"], "GND");
        assert_eq!(planes[0]["layer"], "B.Cu");
        let names: Vec<&str> = planes[0]["zones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|zone| zone["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["GND_BOTTOM", "GND_BOTH"]);
        assert_eq!(planes[1]["net"], "VCC");
        // Only the zones that made this plane, not another net's.
        assert_eq!(
            planes[1]["zones"],
            json!([{ "uuid": "55555555-5555-4555-8555-555555555502", "name": "VCC_REGION" }])
        );
        let steps = completion["next_steps"].as_array().unwrap();
        let tools: Vec<&str> = steps.iter().map(|s| s["tool"].as_str().unwrap()).collect();
        assert_eq!(
            tools,
            [
                "save_project",
                "create_checkpoint",
                "plan_specctra_ses_import",
                "apply_specctra_ses",
                "run_drc",
                "plan_route_refinement",
                "route_batch",
                "run_drc"
            ]
        );
        let numbers: Vec<u64> = steps.iter().map(|s| s["step"].as_u64().unwrap()).collect();
        assert_eq!(numbers, (1..=8).collect::<Vec<_>>());
        assert_eq!(steps[5]["arguments"]["kinds"], json!(["stitching"]));
        assert_eq!(steps[5]["arguments"]["nets"], json!(["GND", "VCC"]));
        assert_eq!(steps[5]["arguments"]["search_radius_mm"], 3.0);
        let repeat = steps[7]["repeat"].as_str().unwrap();
        assert!(
            repeat.contains("steps 6 to 8") && repeat.contains("at most 4 rounds"),
            "{repeat}"
        );
        assert_calls_are_real(steps);
        let open = &completion["if_still_open"];
        assert_eq!(open["fallback_threshold"], 10);
        assert_calls_are_real(open["fallback"].as_array().unwrap());
        assert_prose_names_are_real(&completion);
    }

    /// The fallback exports the board again, and the exporter refuses a board
    /// holding unlocked routing, which it does after the import (review M1).
    /// So the steps keep the exported board before the import, the fallback
    /// puts it back before it exports, and both say why.
    #[test]
    fn the_fallback_puts_the_exported_board_back_before_it_exports_again() {
        let dir = tempfile::tempdir().unwrap();
        let (dsn, ses) = exported(dir.path(), json!("fixed"));
        let (_, completion) = report(&dsn, &ses, Some(0));
        let steps = completion["next_steps"].as_array().unwrap();
        let position = |tool: &str| steps.iter().position(|s| s["tool"] == tool).unwrap();
        // The board is saved and kept before anything is imported.
        assert!(position("save_project") < position("create_checkpoint"));
        assert!(position("create_checkpoint") < position("apply_specctra_ses"));
        let checkpoint = steps[position("create_checkpoint")]["step"]
            .as_u64()
            .unwrap();
        assert_eq!(
            steps[position("create_checkpoint")]["arguments"]["project"],
            json!(dir.path().join("board.kicad_pro"))
        );
        let open = &completion["if_still_open"];
        let precondition = open["precondition"].as_str().unwrap();
        for needle in [
            "unlocked routing",
            "restore_checkpoint",
            "before the import",
            "KiCad closed",
        ] {
            assert!(precondition.contains(needle), "{needle}: {precondition}");
        }
        let fallback = open["fallback"].as_array().unwrap();
        let kinds: Vec<String> = fallback
            .iter()
            .map(|c| {
                c["tool"]
                    .as_str()
                    .map_or("manual".to_string(), str::to_string)
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "manual",
                "restore_checkpoint",
                "restore_checkpoint",
                "launch_kicad_ui",
                "manual",
                "export_specctra_dsn",
                "route_specctra_dsn"
            ]
        );
        // KiCad is closed first, then a preview, then the restore with its revision.
        assert!(fallback[0]["manual"]
            .as_str()
            .unwrap()
            .contains("Close the board and the project"));
        assert_eq!(fallback[1]["arguments"]["dry_run"], true);
        assert_eq!(fallback[2]["arguments"]["dry_run"], false);
        let from = |i: usize, key: &str| {
            fallback[i]["arguments_from"][key]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert!(from(1, "checkpoint_id").contains(&format!("step {checkpoint}")));
        assert!(from(2, "expected_revision").contains("preview"));
        assert_eq!(fallback[5]["arguments"]["pours"], "omit");
        assert!(open["more_than_ten"]
            .as_str()
            .unwrap()
            .contains("fall back"));
    }

    /// The import binds the project file by hash, and `save_project` can make
    /// KiCad rewrite it (review C, nit 1). A caller who follows the steps
    /// literally must learn what that costs and how to recover; a failed
    /// checkpoint must stop the sequence; the restore puts back every file
    /// the checkpoint holds, not only the board (nits 4 and 5).
    #[test]
    fn the_save_and_checkpoint_steps_say_what_can_go_wrong_and_what_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let (dsn, ses) = exported(dir.path(), json!("fixed"));
        let (_, completion) = report(&dsn, &ses, Some(0));
        let steps = completion["next_steps"].as_array().unwrap();
        let why = |tool: &str| {
            steps.iter().find(|s| s["tool"] == tool).unwrap()["why"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let save = why("save_project");
        for needle in [
            "project file",
            "plan_specctra_ses_import then reports a stale revision",
            "export and route again",
            "Saving the project before export_specctra_dsn",
        ] {
            assert!(save.contains(needle), "{needle}: {save}");
        }
        let checkpoint = why("create_checkpoint");
        assert!(checkpoint.contains("If it fails, stop"), "{checkpoint}");
        let fallback = completion["if_still_open"]["fallback"].as_array().unwrap();
        for call in &fallback[1..=2] {
            let text = call["why"].as_str().unwrap();
            assert!(
                text.contains("the board") && text.contains("among them"),
                "{text}"
            );
        }
        assert!(fallback[1]["why"]
            .as_str()
            .unwrap()
            .contains("every design file the checkpoint puts back"));
        assert_prose_names_are_real(&completion);
    }

    /// The words a caller acts on: complete only at 0 unconnected in KiCad's
    /// DRC after the refill; 1 to 10 open is reported open; more than 10
    /// falls back. Nothing that lets a caller stop at the router's count.
    #[test]
    fn the_steps_say_when_the_routing_is_complete_and_what_open_means() {
        let dir = tempfile::tempdir().unwrap();
        let (dsn, ses) = exported(dir.path(), json!("fixed"));
        let (_, completion) = report(&dsn, &ses, Some(0));
        let complete_when = completion["complete_when"].as_str().unwrap();
        for needle in [
            "complete only when",
            "KiCad's DRC",
            "refilled",
            "0 unconnected items",
            "plane nets (GND, VCC)",
            "every other net",
            "do not show",
        ] {
            assert!(complete_when.contains(needle), "{needle}: {complete_when}");
        }
        let open = &completion["if_still_open"];
        let few = open["one_to_ten"].as_str().unwrap();
        assert!(
            few.contains("1 to 10") && few.contains("report every one of them as open"),
            "{few}"
        );
        assert!(few.contains("not complete"), "{few}");
        let many = open["more_than_ten"].as_str().unwrap();
        assert!(
            many.contains("more than 10") && many.contains("fall back"),
            "{many}"
        );
        // The router's 0 is reported as what it is, with the reason.
        assert!(completion["reason"]
            .as_str()
            .unwrap()
            .contains("counts every pad inside a plane"));
        for text in [
            complete_when,
            few,
            many,
            completion["reason"].as_str().unwrap(),
        ] {
            for loose in [
                "probably",
                "should be fine",
                "likely complete",
                "may be complete",
            ] {
                assert!(!text.contains(loose), "{loose}: {text}");
            }
        }
    }

    #[test]
    fn a_run_without_planes_still_names_the_check_that_decides() {
        let dir = tempfile::tempdir().unwrap();
        let (dsn, ses) = exported(dir.path(), json!("omit"));
        let (status, completion) = report(&dsn, &ses, Some(0));
        assert_eq!(status, "complete");
        assert_eq!(completion["planes"], json!([]));
        assert!(completion["if_still_open"].is_null());
        let steps = completion["next_steps"].as_array().unwrap();
        assert_eq!(steps.len(), 3, "apply, refill and DRC; no stitching");
        assert_calls_are_real(steps);
        assert!(completion["complete_when"]
            .as_str()
            .unwrap()
            .contains("0 unconnected items, on every net"));
        assert!(completion["reason"]
            .as_str()
            .unwrap()
            .contains("was given no pour as a plane"));
        // A router that left connections is incomplete, planes or not.
        let (status, left) = report(&dsn, &ses, Some(3));
        assert_eq!(status, "incomplete");
        assert!(left["reason"]
            .as_str()
            .unwrap()
            .contains("3 connection(s) it could not route"));
        let (status, unknown) = report(&dsn, &ses, None);
        assert_eq!(status, "unverified");
        assert!(unknown["reason"]
            .as_str()
            .unwrap()
            .contains("gave no unrouted count"));
    }

    #[test]
    fn a_dsn_with_no_manifest_or_unreadable_planes_is_never_complete() {
        let dir = tempfile::tempdir().unwrap();
        let (dsn, ses) = exported(dir.path(), json!("fixed"));
        std::fs::remove_file(dir.path().join("board.dsn.konnect.json")).unwrap();
        let (status, completion) = report(&dsn, &ses, Some(0));
        assert_eq!(status, "unverified", "the DSN alone still shows the planes");
        assert_eq!(completion["planes"].as_array().unwrap().len(), 3);
        assert_eq!(completion["next_steps"], json!([]));
        assert!(completion["next_steps_missing"]
            .as_str()
            .unwrap()
            .contains("export_specctra_dsn"));
        // Without a board to name, no fallback is offered either.
        assert!(completion["if_still_open"].is_null());
        let broken: anyhow::Result<Vec<crate::specctra::DsnPlane>> =
            Err(anyhow::anyhow!("not a DSN"));
        let (status, completion) =
            super::completion_report(&dsn, &ses, &broken, Some(0), &WidthOutcome::default());
        assert_eq!(status, "unverified");
        assert!(completion["reason"]
            .as_str()
            .unwrap()
            .contains("could not be read"));
        // A router that left connections is incomplete even then.
        assert_eq!(
            super::completion_report(&dsn, &ses, &broken, Some(3), &WidthOutcome::default()).0,
            "incomplete"
        );
        // An error that carries the file is not repeated whole (review S3).
        let long: anyhow::Result<Vec<crate::specctra::DsnPlane>> =
            Err(anyhow::anyhow!("{}", "(pcb ".repeat(50_000)));
        let (_, completion) =
            super::completion_report(&dsn, &ses, &long, Some(0), &WidthOutcome::default());
        assert!(completion["reason"].as_str().unwrap().len() < 400);
    }

    /// A manifest that names no existing board file is not repeated into calls.
    #[test]
    fn a_manifest_board_that_does_not_exist_is_not_named_in_the_steps() {
        let dir = tempfile::tempdir().unwrap();
        let (dsn, ses) = exported(dir.path(), json!("fixed"));
        std::fs::remove_file(dir.path().join("board.kicad_pcb")).unwrap();
        let (status, completion) = report(&dsn, &ses, Some(0));
        assert_eq!(status, "unverified");
        assert_eq!(completion["next_steps"], json!([]));
        assert!(completion["next_steps_missing"].is_string());
        assert!(completion["if_still_open"].is_null());
    }

    /// Only the pours written as planes name a plane: an omitted pour of the
    /// same net on the same layer is not one of its zones.
    #[test]
    fn a_plane_names_only_the_zones_that_made_it() {
        let dir = tempfile::tempdir().unwrap();
        let (dsn, ses) = exported(
            dir.path(),
            json!({ "zones": [{ "name": "GND_TOP", "policy": "omit" }] }),
        );
        let (_, completion) = report(&dsn, &ses, Some(0));
        let front = completion["planes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["net"] == "GND" && p["layer"] == "F.Cu")
            .unwrap()
            .clone();
        let names: Vec<&str> = front["zones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|z| z["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["GND_BOTH"], "GND_TOP was omitted");
    }

    #[test]
    fn measured_preset_is_sent_by_default_and_existing_overrides_survive() {
        let hints = crate::specctra::RoutingHints::default();
        let settings = route_settings(&json!({}), &hints, "java".into()).unwrap();
        let body = crate::freerouting_mcp::job_settings(&settings);
        assert_eq!(body["autorouter"]["max_passes"], 20);
        assert_eq!(body["optimizer"]["enabled"], true);
        assert_eq!(body["scoring"]["via_costs"], 10);
        assert_eq!(
            body["scoring"]["default_undesired_direction_trace_cost"],
            1.0
        );
        assert_eq!(
            body["optimizer"]["max_consecutive_failures_pass1"],
            1_000_000
        );
        assert_eq!(body["optimizer"]["max_consecutive_failures"], 1_000_000);
        // The job API accepts percentages; unlike the CLI, 0.25 means 0.25%.
        assert_eq!(body["optimizer"]["improvement_threshold"], 0.25);
        let settings = route_settings(
            &json!({"max_passes": 7, "optimizer_enabled": false}),
            &hints,
            "java".into(),
        )
        .unwrap();
        let body = crate::freerouting_mcp::job_settings(&settings);
        assert_eq!(body["autorouter"]["max_passes"], 7);
        assert_eq!(body["optimizer"]["enabled"], false);
        assert_eq!(body["optimizer"]["improvement_threshold"], 0.25);
    }

    #[test]
    fn fanout_and_neckdown_are_off_by_default() {
        let hints = crate::specctra::RoutingHints {
            copper_edge_clearance_um: Some(500),
            retry_width_um: Some(200),
            retry_off_reason: None,
        };
        let java = std::path::PathBuf::from("java");
        let settings = route_settings(&json!({}), &hints, java.clone()).unwrap();
        assert_eq!(settings.fanout_enabled, Some(false));
        assert_eq!(settings.automatic_neckdown, Some(false));
        assert_eq!(settings.copper_to_edge_clearance_um, Some(500.0));
        assert_eq!(settings.retry_width_um, Some(200.0));
        // Asking for neck-down does not turn fanout on.
        let asked = route_settings(&json!({ "allow_neckdown": true }), &hints, java).unwrap();
        assert_eq!(asked.fanout_enabled, Some(false));
    }

    #[test]
    fn no_retry_width_is_sent_when_the_manifest_turned_it_off() {
        let hints = crate::specctra::RoutingHints {
            copper_edge_clearance_um: None,
            retry_width_um: None,
            retry_off_reason: Some(crate::specctra::RETRY_OFF_DIFFERENT_FLOORS),
        };
        let settings =
            route_settings(&json!({}), &hints, std::path::PathBuf::from("java")).unwrap();
        // Sent as 0, so a stored neck width cannot turn the retry back on.
        assert_eq!(settings.retry_width_um, Some(0.0));
        let body = crate::freerouting_mcp::job_settings(&settings);
        assert_eq!(body["neck_width_um"], json!(0.0));
    }

    #[test]
    fn neckdown_is_off_unless_asked_for() {
        assert!(!allow_neckdown(&json!({})));
        assert!(!allow_neckdown(&json!({ "allow_neckdown": false })));
        assert!(allow_neckdown(&json!({ "allow_neckdown": true })));
    }
}

// ─── JLCPCB database path helper ─────────────────────────────────────────────

fn default_jlcpcb_db_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_default();
        PathBuf::from(appdata).join("konnect").join("jlcpcb.db")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(".konnect").join("jlcpcb.db")
    }
}

fn resolve_db_path(args: &serde_json::Value, ctx: &ToolContext) -> PathBuf {
    if let Some(p) = args["output_path"].as_str() {
        return PathBuf::from(p);
    }
    if let Some(p) = &ctx.config.jlcpcb_db_path {
        return p.clone();
    }
    default_jlcpcb_db_path()
}

#[cfg(test)]
mod configured_db_path_tests {
    use super::*;
    use crate::mcp::handler::McpHandler;
    use crate::tools::ServerConfig;

    #[tokio::test]
    async fn blank_database_configuration_uses_the_default_through_served_tools() {
        const CHILD: &str = "KONNECT_TEST_JLCPCB_PATH_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // Isolate platform path environment from other tests and user caches.
            let dir = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tools::integration::configured_db_path_tests::blank_database_configuration_uses_the_default_through_served_tools",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env("APPDATA", dir.path())
                .env("HOME", dir.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let default = default_jlcpcb_db_path();
        std::fs::create_dir_all(default.parent().unwrap()).unwrap();
        let configured = default.with_file_name(" configured.db");
        let override_path = default.with_file_name("override.db");
        for path in [&default, &configured, &override_path] {
            let conn = rusqlite::Connection::open(path).unwrap();
            conn.execute_batch("CREATE TABLE components (LCSC TEXT, MFR_Part TEXT, Package TEXT, Manufacturer TEXT, Library_Type TEXT, Description TEXT, Datasheet TEXT, Price REAL, Stock INTEGER, Category TEXT);")
                .unwrap();
            conn.execute("INSERT INTO components VALUES ('C678', ?1, '0603', 'Test', 'Basic', 'path probe', '', 0.1, 10, 'Test')", [path.to_str().unwrap()]).unwrap();
        }
        for (setting, expected) in [
            (None, default.clone()),
            (Some(PathBuf::new()), default.clone()),
            (Some(PathBuf::from(" \t\n ")), default.clone()),
            (Some(configured.clone()), configured.clone()),
        ] {
            let config = ServerConfig {
                jlcpcb_db_path: setting.clone(),
                eager_toolsets: true,
                ..Default::default()
            };
            let direct = ToolContext::new(
                config.clone(),
                std::sync::Arc::new(crate::router::ToolRouter::new()),
            );
            assert_eq!(resolve_db_path(&json!({}), &direct), expected);
            let handler = McpHandler::new(config).await.unwrap();
            for selected in [&expected, &override_path] {
                for (name, mut arguments) in [
                    ("download_jlcpcb_database", json!({})),
                    ("search_jlcpcb_parts", json!({"query": "path probe"})),
                ] {
                    if selected == &override_path {
                        if name == "search_jlcpcb_parts" {
                            continue;
                        }
                        arguments["output_path"] = json!(override_path);
                    }
                    let response = handler
                        .handle_message(json!({
                            "jsonrpc": "2.0", "id": 678, "method": "tools/call",
                            "params": {"name": name, "arguments": arguments}
                        }))
                        .await
                        .unwrap()
                        .result
                        .unwrap();
                    assert_ne!(response["isError"], true, "{name}: {response}");
                    let body: serde_json::Value =
                        serde_json::from_str(response["content"][0]["text"].as_str().unwrap())
                            .unwrap();
                    if name == "download_jlcpcb_database" {
                        assert_eq!(body["status"], "already_exists");
                        assert_eq!(body["path"], json!(selected));
                    } else {
                        assert_eq!(body["results"][0]["mpn"], json!(selected));
                    }
                }
            }
        }
    }
}

// ─── Retry/backoff for external HTTP calls ────────────────────────────────────
//
// JLCPCB database download and LCSC datasheet lookups are the only genuinely
// networked calls in this toolset (everything else queries the local SQLite
// cache). Both are prone to transient failures — timeouts, connection resets,
// rate limiting — that a simple retry clears up without any user action.

/// Retry policy: 3 attempts total, exponential backoff starting at 300ms
/// (300ms, then 600ms between attempts).
const RETRY_MAX_ATTEMPTS: u32 = 3;
const RETRY_BASE_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

/// Whether an HTTP status is worth retrying. 429 (rate limited) and 5xx
/// (server-side) are transient; other 4xx (404, 401, ...) are not — retrying
/// a "not found" or "unauthorized" wastes time and won't change the outcome.
fn is_transient_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// Delay before the next attempt, given the attempt number just made (1-based).
fn backoff_delay(attempt: u32) -> std::time::Duration {
    RETRY_BASE_DELAY * 2u32.pow(attempt.saturating_sub(1))
}

/// GET `url` with retry/backoff for transient failures (network-level errors,
/// 429, and 5xx). Returns the last response/error once attempts are exhausted.
async fn get_with_backoff(
    client: &reqwest::Client,
    url: &str,
) -> anyhow::Result<reqwest::Response> {
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match client.get(url).send().await {
            Ok(resp) => {
                let status = resp.status();
                if !is_transient_status(status) || attempt >= RETRY_MAX_ATTEMPTS {
                    return Ok(resp);
                }
                tracing::warn!(
                    "[BETA] {} returned {} (attempt {}/{}), retrying",
                    url,
                    status,
                    attempt,
                    RETRY_MAX_ATTEMPTS
                );
            }
            Err(e) => {
                if attempt >= RETRY_MAX_ATTEMPTS {
                    return Err(e.into());
                }
                tracing::warn!(
                    "[BETA] request to {} failed (attempt {}/{}): {}, retrying",
                    url,
                    attempt,
                    RETRY_MAX_ATTEMPTS,
                    e
                );
            }
        }
        tokio::time::sleep(backoff_delay(attempt)).await;
    }
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

fn parse_jlcpcb_chunk_count(value: &str) -> anyhow::Result<usize> {
    let chunk_count = value
        .trim()
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("Invalid JLCPCB feed chunk count: {value:?}"))?;
    if !(1..=JLCPCB_MAX_CHUNKS).contains(&chunk_count) {
        anyhow::bail!(
            "JLCPCB feed chunk count {chunk_count} is outside the supported range 1..={JLCPCB_MAX_CHUNKS}"
        );
    }
    Ok(chunk_count)
}

async fn download_jlcpcb_archive(
    client: &reqwest::Client,
    archive_path: &Path,
) -> anyhow::Result<(usize, u64)> {
    let sentinel_url = format!("{JLCPCB_FEED_BASE_URL}/{JLCPCB_FEED_SENTINEL}");
    let sentinel_response = get_with_backoff(client, &sentinel_url).await?;
    if !sentinel_response.status().is_success() {
        anyhow::bail!(
            "Failed to read JLCPCB feed manifest: HTTP {}",
            sentinel_response.status()
        );
    }
    let chunk_count = parse_jlcpcb_chunk_count(&sentinel_response.text().await?)?;

    let mut archive = tokio::fs::File::create(archive_path).await?;
    let mut downloaded_bytes = 0u64;
    for chunk_number in 1..=chunk_count {
        let chunk_url = format!("{JLCPCB_FEED_BASE_URL}/{JLCPCB_FEED_ARCHIVE}.{chunk_number:03}");
        let mut response = get_with_backoff(client, &chunk_url).await?;
        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to download JLCPCB database chunk {chunk_number}/{chunk_count}: HTTP {}",
                response.status()
            );
        }

        while let Some(bytes) = response.chunk().await? {
            downloaded_bytes = downloaded_bytes
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| anyhow::anyhow!("JLCPCB archive size overflow"))?;
            if downloaded_bytes > JLCPCB_MAX_ARCHIVE_BYTES {
                anyhow::bail!(
                    "JLCPCB archive exceeds the {} byte safety limit",
                    JLCPCB_MAX_ARCHIVE_BYTES
                );
            }
            archive.write_all(&bytes).await?;
        }
    }
    archive.sync_all().await?;
    Ok((chunk_count, downloaded_bytes))
}

fn extract_jlcpcb_database(archive_path: &Path, output_path: &Path) -> anyhow::Result<()> {
    let archive_file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(archive_file)?;
    if archive.len() != 1 {
        anyhow::bail!(
            "JLCPCB archive must contain exactly one database file; found {} entries",
            archive.len()
        );
    }

    let mut database = archive.by_index(0)?;
    if database.is_dir() || database.enclosed_name().is_none() {
        anyhow::bail!("JLCPCB archive does not contain a safe database file");
    }
    if database.size() > JLCPCB_MAX_DATABASE_BYTES {
        anyhow::bail!(
            "JLCPCB database exceeds the {} byte safety limit",
            JLCPCB_MAX_DATABASE_BYTES
        );
    }

    let output_file = std::fs::File::create(output_path)?;
    let mut output = BufWriter::new(output_file);
    io::copy(&mut database, &mut output)?;
    output.into_inner()?.sync_all()?;
    Ok(())
}

fn validate_upstream_schema(conn: &rusqlite::Connection) -> anyhow::Result<()> {
    let mut statement = conn.prepare("PRAGMA table_info(parts)")?;
    let columns: HashSet<String> = statement
        .query_map([], |row| row.get(1))?
        .collect::<rusqlite::Result<_>>()?;
    let required = [
        "LCSC Part",
        "First Category",
        "Second Category",
        "MFR.Part",
        "Package",
        "Manufacturer",
        "Library Type",
        "Description",
        "Datasheet",
        "Price",
        "Stock",
    ];
    let missing: Vec<&str> = required
        .into_iter()
        .filter(|column| !columns.contains(*column))
        .collect();
    if !missing.is_empty() {
        anyhow::bail!(
            "JLCPCB feed schema is missing required columns: {}",
            missing.join(", ")
        );
    }
    Ok(())
}

fn build_konnect_jlcpcb_database(upstream_path: &Path, output_path: &Path) -> anyhow::Result<u64> {
    let upstream = rusqlite::Connection::open_with_flags(
        upstream_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    validate_upstream_schema(&upstream)?;
    drop(upstream);

    let mut output = rusqlite::Connection::open(output_path)?;
    output.execute_batch(
        "PRAGMA journal_mode = OFF;
         PRAGMA synchronous = OFF;
         PRAGMA temp_store = MEMORY;",
    )?;
    let upstream_path = upstream_path.to_string_lossy().into_owned();
    output.execute(
        "ATTACH DATABASE ?1 AS upstream",
        rusqlite::params![upstream_path],
    )?;

    let transaction = output.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE components (
             LCSC TEXT NOT NULL PRIMARY KEY,
             MFR_Part TEXT NOT NULL,
             Package TEXT NOT NULL,
             Manufacturer TEXT NOT NULL,
             Library_Type TEXT NOT NULL,
             Description TEXT NOT NULL,
             Datasheet TEXT NOT NULL,
             Price REAL NOT NULL,
             Stock INTEGER NOT NULL,
             Category TEXT NOT NULL
         );
         PRAGMA user_version = 1;",
    )?;
    let imported = transaction.execute(
        "INSERT INTO components (
             LCSC, MFR_Part, Package, Manufacturer, Library_Type,
             Description, Datasheet, Price, Stock, Category
         )
         SELECT
             COALESCE(\"LCSC Part\", ''),
             COALESCE(\"MFR.Part\", ''),
             COALESCE(\"Package\", ''),
             COALESCE(\"Manufacturer\", ''),
             COALESCE(\"Library Type\", ''),
             COALESCE(\"Description\", ''),
             COALESCE(\"Datasheet\", ''),
             CASE
                 WHEN instr(COALESCE(\"Price\", ''), ':') > 0
                 THEN CAST(substr(\"Price\", instr(\"Price\", ':') + 1) AS REAL)
                 ELSE CAST(COALESCE(NULLIF(\"Price\", ''), '0') AS REAL)
             END,
             CAST(COALESCE(NULLIF(\"Stock\", ''), '0') AS INTEGER),
             CASE
                 WHEN COALESCE(\"First Category\", '') = '' THEN COALESCE(\"Second Category\", '')
                 WHEN COALESCE(\"Second Category\", '') = '' THEN \"First Category\"
                 ELSE \"First Category\" || ' / ' || \"Second Category\"
             END
         FROM upstream.parts",
        [],
    )?;
    transaction.execute_batch(
        "CREATE INDEX components_mfr_part_idx ON components(MFR_Part);
         CREATE INDEX components_package_idx ON components(Package);
         CREATE INDEX components_library_stock_idx ON components(Library_Type, Stock);
         CREATE INDEX components_category_idx ON components(Category);",
    )?;
    transaction.commit()?;
    output.execute_batch("DETACH DATABASE upstream; PRAGMA synchronous = FULL;")?;

    let part_count: i64 =
        output.query_row("SELECT COUNT(*) FROM components", [], |row| row.get(0))?;
    if imported == 0 || part_count <= 0 {
        anyhow::bail!("JLCPCB feed did not contain any components");
    }
    let quick_check: String = output.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if quick_check != "ok" {
        anyhow::bail!("Generated JLCPCB database failed validation: {quick_check}");
    }
    drop(output);
    Ok(part_count.try_into()?)
}

fn replace_jlcpcb_database(
    staged_path: &Path,
    destination: &Path,
    backup_path: &Path,
) -> anyhow::Result<()> {
    if !destination.exists() {
        std::fs::rename(staged_path, destination)?;
        return Ok(());
    }

    std::fs::rename(destination, backup_path)?;
    if let Err(install_error) = std::fs::rename(staged_path, destination) {
        if let Err(restore_error) = std::fs::rename(backup_path, destination) {
            anyhow::bail!(
                "Failed to install the new JLCPCB database ({install_error}) and restore the previous database ({restore_error})"
            );
        }
        return Err(install_error.into());
    }
    std::fs::remove_file(backup_path)?;
    Ok(())
}

async fn handle_download_jlcpcb(
    args: &serde_json::Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let db_path = resolve_db_path(args, ctx);
    let force = args["force"].as_bool().unwrap_or(false);

    if db_path.exists() && !force {
        let meta = tokio::fs::metadata(&db_path).await?;
        return Ok(CallToolResult::text(
            serde_json::to_string(&json!({
                "status": "already_exists",
                "path": db_path.to_str().unwrap_or(""),
                "size_bytes": meta.len(),
                "note": "Use force=true to re-download"
            }))
            .unwrap(),
        ));
    }

    if let Some(parent) = db_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let db_parent = db_path.parent().unwrap_or_else(|| Path::new("."));
    let work_dir = tempfile::Builder::new()
        .prefix("konnect-jlcpcb-")
        .tempdir_in(db_parent)?;
    let archive_path = work_dir.path().join(JLCPCB_FEED_ARCHIVE);
    let upstream_path = work_dir.path().join("upstream.db");
    let staged_path = work_dir.path().join("konnect.db");
    let backup_path = work_dir.path().join("previous.db");

    let (chunk_count, downloaded_bytes) = download_jlcpcb_archive(&client, &archive_path).await?;
    let (part_count, size_bytes) = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        move || -> anyhow::Result<(u64, u64)> {
            extract_jlcpcb_database(&archive_path, &upstream_path)?;
            let part_count = build_konnect_jlcpcb_database(&upstream_path, &staged_path)?;
            let size_bytes = std::fs::metadata(&staged_path)?.len();
            replace_jlcpcb_database(&staged_path, &db_path, &backup_path)?;
            Ok((part_count, size_bytes))
        }
    })
    .await??;

    Ok(CallToolResult::text(
        serde_json::to_string(&json!({
            "success": true,
            "path": db_path.to_str().unwrap_or(""),
            "size_bytes": size_bytes,
            "part_count": part_count,
            "source": JLCPCB_FEED_BASE_URL,
            "downloaded_chunks": chunk_count,
            "downloaded_bytes": downloaded_bytes
        }))
        .unwrap(),
    ))
}

/// Build a deterministic cache key from a tool name, the resolved DB path
/// (so pointing at a different `output_path` never serves stale results),
/// and the query parameters that affect the result set.
fn cache_key(tool: &str, db_path: &std::path::Path, parts: &[&str]) -> String {
    format!("{}|{}|{}", tool, db_path.display(), parts.join("|"))
}

/// Columns `search_jlcpcb_parts` matches every query word against, in the
/// order they appear in the SQL and in the response's `search_fields`.
const JLCPCB_SEARCH_FIELDS: [&str; 5] =
    ["Description", "MFR_Part", "LCSC", "Package", "Manufacturer"];

/// The query words a part must all contain: the single source for both the
/// SQL and the `tokens` evidence in the response.
fn jlcpcb_search_tokens(query: &str) -> Vec<String> {
    query.split_whitespace().map(String::from).collect()
}

async fn handle_search_jlcpcb_parts(
    args: &serde_json::Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    // Arguments before environment: a missing `query` is the caller's
    // mistake and is theirs to fix, whereas a missing database is transient
    // and unrelated. Checking the database first told someone who had simply
    // forgotten the query to go and download a 2.5M-part catalogue (#218).
    let query = match require_str(args, "query") {
        Ok(v) => v.to_string(),
        Err(e) => return Ok(e),
    };

    let db_path = resolve_db_path(args, ctx);
    if !db_path.exists() {
        return Ok(CallToolResult::error(
            "JLCPCB database not found. Run download_jlcpcb_database first.",
        ));
    }
    let basic_only = args["basic_only"].as_bool().unwrap_or(false);
    let in_stock = args["in_stock"].as_bool().unwrap_or(true);
    let limit = args["limit"].as_u64().unwrap_or(20) as usize;
    let category = args["category"].as_str().map(String::from);

    let key = cache_key(
        "search_jlcpcb_parts",
        &db_path,
        &[
            &query,
            category.as_deref().unwrap_or(""),
            &basic_only.to_string(),
            &in_stock.to_string(),
            &limit.to_string(),
        ],
    );
    if let Some(cached) = ctx.jlcpcb_cache.get(&key) {
        let mut body = cached;
        body["cached"] = json!(true);
        return Ok(CallToolResult::text(serde_json::to_string(&body).unwrap()));
    }

    let tokens = jlcpcb_search_tokens(&query);
    let sql_tokens = tokens.clone();
    let results = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<serde_json::Value>> {
        let conn = rusqlite::Connection::open(&db_path)?;

        // The JLCPCB db schema has columns: LCSC, MFR_Part, Package, Solder_Joint,
        // Manufacturer, Library_Type, Description, Datasheet, Price, Stock
        //
        // Every whitespace-separated word must match somewhere in one of
        // the searchable columns, in any order (#432). `%`, `_` and `\` in a
        // word are matched literally, and every value is a bound parameter.
        let mut sql = String::from(
            "SELECT LCSC, MFR_Part, Package, Manufacturer, Library_Type, Description, Datasheet, Price, Stock \
             FROM components WHERE 1=1",
        );
        let mut params: Vec<String> = Vec::new();
        for token in &sql_tokens {
            let escaped = token
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            params.push(format!("%{}%", escaped));
            let n = params.len();
            sql.push_str(&format!(
                " AND (Description LIKE ?{n} ESCAPE '\\' OR MFR_Part LIKE ?{n} ESCAPE '\\' \
                 OR LCSC LIKE ?{n} ESCAPE '\\' OR Package LIKE ?{n} ESCAPE '\\' \
                 OR Manufacturer LIKE ?{n} ESCAPE '\\')"
            ));
        }
        if basic_only {
            sql.push_str(" AND Library_Type = 'Basic'");
        }
        if in_stock {
            sql.push_str(" AND Stock > 0");
        }
        if let Some(ref cat) = category {
            params.push(format!("%{}%", cat));
            sql.push_str(&format!(" AND Category LIKE ?{}", params.len()));
        }
        // Stable order so a LIMIT page is the same on every run (#432).
        sql.push_str(&format!(" ORDER BY LCSC LIMIT {}", limit));

        let mut stmt = conn.prepare(&sql)?;
        let rows: Vec<serde_json::Value> = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), row_to_part_json)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    })
    .await??;

    let body = json!({
        "query": args["query"].as_str().unwrap_or(""),
        "count": results.len(),
        "results": results,
        "tokens": tokens,
        "search_fields": JLCPCB_SEARCH_FIELDS
    });
    ctx.jlcpcb_cache.put(key, body.clone());

    let mut body = body;
    body["cached"] = json!(false);
    Ok(CallToolResult::text(serde_json::to_string(&body).unwrap()))
}

fn row_to_part_json(row: &rusqlite::Row) -> rusqlite::Result<serde_json::Value> {
    // The exact LCSC PDF URL for most parts. Omitted here once made a caller
    // unable to get a datasheet the table already held (#255); an empty cell
    // reads as null rather than "" so callers can test presence directly.
    let datasheet_url = match row.get::<_, String>(6).unwrap_or_default() {
        url if url.is_empty() => serde_json::Value::Null,
        url => json!(url),
    };
    Ok(json!({
        "lcsc": row.get::<_, String>(0).unwrap_or_default(),
        "mpn": row.get::<_, String>(1).unwrap_or_default(),
        "package": row.get::<_, String>(2).unwrap_or_default(),
        "manufacturer": row.get::<_, String>(3).unwrap_or_default(),
        "library_type": row.get::<_, String>(4).unwrap_or_default(),
        "description": row.get::<_, String>(5).unwrap_or_default(),
        "datasheet_url": datasheet_url,
        "price": row.get::<_, f64>(7).unwrap_or(0.0),
        "stock": row.get::<_, i64>(8).unwrap_or(0)
    }))
}

async fn handle_get_jlcpcb_part(
    args: &serde_json::Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let db_path = resolve_db_path(args, ctx);
    if !db_path.exists() {
        return Ok(CallToolResult::error(
            "JLCPCB database not found. Run download_jlcpcb_database first.",
        ));
    }
    let lcsc_id = require_str(args, "lcsc_id")
        .map_err(|e| anyhow::anyhow!("{:?}", e))?
        .to_string();

    let key = cache_key("get_jlcpcb_part", &db_path, &[&lcsc_id]);
    if let Some(mut cached) = ctx.jlcpcb_cache.get(&key) {
        cached["cached"] = json!(true);
        return Ok(CallToolResult::text(
            serde_json::to_string(&cached).unwrap(),
        ));
    }

    let result =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Option<serde_json::Value>> {
            let conn = rusqlite::Connection::open(&db_path)?;
            let mut stmt = conn.prepare(
            "SELECT LCSC, MFR_Part, Package, Manufacturer, Library_Type, Description, Datasheet, Price, Stock \
             FROM components WHERE LCSC = ?1 LIMIT 1"
        )?;
            let mut rows = stmt.query_map(rusqlite::params![lcsc_id], row_to_part_json)?;
            Ok(rows.next().and_then(|r| r.ok()))
        })
        .await??;

    match result {
        Some(part) => {
            ctx.jlcpcb_cache.put(key, part.clone());
            let mut part = part;
            part["cached"] = json!(false);
            Ok(CallToolResult::text(serde_json::to_string(&part).unwrap()))
        }
        None => Ok(CallToolResult::error(format!(
            "Part not found in database: {}",
            args["lcsc_id"].as_str().unwrap_or("")
        ))),
    }
}

/// JLCPCB library types in order of preference. Basic parts carry no setup
/// fee, Preferred parts none for economic assembly, and every unique Extended
/// part adds one.
const LIBRARY_TYPE_ORDER: [&str; 3] = ["Basic", "Preferred", "Extended"];

const SUGGEST_DEFAULT_MIN_STOCK_COUNT: u64 = 100;

/// A part that matched both the value and the package, with the columns the
/// stock floor, price limit and ranking read.
#[derive(Debug, Clone)]
struct AlternativeCandidate {
    part: serde_json::Value,
    lcsc_id: String,
    library_type: String,
    price: f64,
    stock_count: i64,
}

impl AlternativeCandidate {
    /// The import stores a price the feed did not give as 0 (#582), and no
    /// part is free, so 0 or less means the price is unknown.
    fn price_known(&self) -> bool {
        self.price > 0.0
    }

    fn library_rank(&self) -> usize {
        LIBRARY_TYPE_ORDER
            .iter()
            .position(|kind| *kind == self.library_type)
            .unwrap_or(LIBRARY_TYPE_ORDER.len())
    }
}

#[derive(Debug, Clone, Copy)]
struct AlternativePolicy {
    min_stock_count: u64,
    prefer_basic: bool,
    max_price: Option<f64>,
    limit: usize,
}

impl AlternativePolicy {
    fn criteria(&self) -> Vec<&'static str> {
        let mut criteria = Vec::new();
        if self.prefer_basic {
            criteria.push("library_type: Basic, Preferred, Extended");
        }
        criteria.extend([
            "known price before unknown price",
            "price ascending",
            "stock descending",
            "LCSC number",
        ]);
        criteria
    }
}

/// What filtering removed and what was returned, so an empty or short result
/// says why.
#[derive(Debug, Default, PartialEq, Eq)]
struct AlternativeCounts {
    matched: usize,
    below_min_stock: usize,
    unknown_price: usize,
    above_max_price: usize,
    returned: usize,
}

/// Apply the stock floor and price limit, then rank. Ranking by price alone
/// put Extended parts with a dozen units in stock, and parts whose price was
/// unknown, ahead of the Basic part with millions (#785).
fn rank_alternatives(
    candidates: Vec<AlternativeCandidate>,
    policy: &AlternativePolicy,
) -> (Vec<AlternativeCandidate>, AlternativeCounts) {
    let mut counts = AlternativeCounts {
        matched: candidates.len(),
        ..AlternativeCounts::default()
    };
    let min_stock_count = i64::try_from(policy.min_stock_count).unwrap_or(i64::MAX);
    let mut kept: Vec<AlternativeCandidate> = Vec::new();
    for candidate in candidates {
        if candidate.stock_count < min_stock_count {
            counts.below_min_stock += 1;
        } else if let Some(max_price) = policy.max_price {
            if !candidate.price_known() {
                counts.unknown_price += 1;
            } else if candidate.price > max_price {
                counts.above_max_price += 1;
            } else {
                kept.push(candidate);
            }
        } else {
            kept.push(candidate);
        }
    }

    let tier = |candidate: &AlternativeCandidate| {
        if policy.prefer_basic {
            candidate.library_rank()
        } else {
            0
        }
    };
    kept.sort_by(|a, b| {
        tier(a)
            .cmp(&tier(b))
            .then(b.price_known().cmp(&a.price_known()))
            .then(a.price.total_cmp(&b.price))
            .then(b.stock_count.cmp(&a.stock_count))
            .then(a.lcsc_id.cmp(&b.lcsc_id))
    });
    kept.truncate(policy.limit);
    counts.returned = kept.len();
    (kept, counts)
}

/// Read every part in one of `packages` whose description or part number
/// contains `value` as a whole value. SQL narrows by substring; the
/// whole-value test, which SQLite's LIKE cannot express, runs here.
fn query_alternative_candidates(
    db_path: &Path,
    value: &str,
    packages: &[String],
) -> anyhow::Result<Vec<AlternativeCandidate>> {
    let conn = rusqlite::Connection::open(db_path)?;
    let placeholders = (1..=packages.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let like = packages.len() + 1;
    let sql = format!(
        "SELECT LCSC, MFR_Part, Package, Manufacturer, Library_Type, Description, Datasheet, Price, Stock \
         FROM components WHERE Package COLLATE NOCASE IN ({placeholders}) \
         AND (Description LIKE ?{like} ESCAPE '\\' OR MFR_Part LIKE ?{like} ESCAPE '\\')"
    );
    let escaped = value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let mut params: Vec<String> = packages.to_vec();
    params.push(format!("%{escaped}%"));

    let mut statement = conn.prepare(&sql)?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            let mpn: String = row.get::<_, Option<String>>(1)?.unwrap_or_default();
            let description: String = row.get::<_, Option<String>>(5)?.unwrap_or_default();
            Ok((
                mpn,
                description,
                AlternativeCandidate {
                    part: row_to_part_json(row)?,
                    lcsc_id: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    library_type: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    price: row.get::<_, Option<f64>>(7)?.unwrap_or(0.0),
                    stock_count: row.get::<_, Option<i64>>(8)?.unwrap_or(0),
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|(mpn, description, _)| {
            lcsc_packages::contains_whole_value(description, value)
                || lcsc_packages::contains_whole_value(mpn, value)
        })
        .map(|(_, _, candidate)| candidate)
        .collect())
}

async fn handle_suggest_alternatives(
    args: &serde_json::Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    // Arguments before environment — see `handle_search_jlcpcb_parts`.
    let value = match require_str(args, "value") {
        Ok(v) => v.trim().to_string(),
        Err(e) => return Ok(e),
    };
    let footprint = match require_str(args, "footprint") {
        Ok(v) => v.trim().to_string(),
        Err(e) => return Ok(e),
    };
    if value.is_empty() {
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::InvalidArgument {
                field: "value".into(),
                reason: "empty".into(),
            },
            "value is empty; give the component value or part number to match",
        ));
    }
    // The package used to be the footprint ID's last `_` segment, which for
    // every standard KiCad footprint (`C_0402_1005Metric` → `1005Metric`) is
    // a name LCSC never uses, so the search silently matched nothing (#785).
    let Some(package_match) = lcsc_packages::lcsc_packages(&footprint) else {
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::InvalidArgument {
                field: "footprint".into(),
                reason: "no known LCSC package for this footprint".into(),
            },
            format!(
                "No LCSC package name is known for footprint '{footprint}'. Pass LCSC's own \
                 package name without a library prefix instead, for example '0402', 'SOT-23' \
                 or 'LQFP-48(7x7)'."
            ),
        ));
    };

    let db_path = resolve_db_path(args, ctx);
    if !db_path.exists() {
        return Ok(CallToolResult::error(
            "JLCPCB database not found. Run download_jlcpcb_database first.",
        ));
    }
    let policy = AlternativePolicy {
        min_stock_count: args["min_stock_count"]
            .as_u64()
            .unwrap_or(SUGGEST_DEFAULT_MIN_STOCK_COUNT),
        prefer_basic: args["prefer_basic"].as_bool().unwrap_or(true),
        max_price: args["max_price_usd"].as_f64(),
        limit: args["limit"].as_u64().unwrap_or(5) as usize,
    };

    let key = cache_key(
        "suggest_jlcpcb_alternatives",
        &db_path,
        &[
            &value,
            &footprint,
            &policy.max_price.map(|v| v.to_string()).unwrap_or_default(),
            &policy.limit.to_string(),
            &policy.min_stock_count.to_string(),
            &policy.prefer_basic.to_string(),
        ],
    );
    if let Some(cached) = ctx.jlcpcb_cache.get(&key) {
        let mut body = cached;
        body["cached"] = json!(true);
        return Ok(CallToolResult::text(serde_json::to_string(&body).unwrap()));
    }

    let packages = package_match.packages.clone();
    let query_value = value.clone();
    let candidates = tokio::task::spawn_blocking(move || {
        query_alternative_candidates(&db_path, &query_value, &packages)
    })
    .await??;
    let (alternatives, counts) = rank_alternatives(candidates, &policy);

    let body = json!({
        "value": value,
        "footprint": footprint,
        "package_match": {
            "rule": package_match.rule.name(),
            "lcsc_packages": package_match.packages,
            "note": "Matched by LCSC's package name, which does not record pitch or exposed-pad size; check the land pattern against the datasheet."
        },
        "value_match": {
            "columns": ["Description", "MFR_Part"],
            "rule": "whole_value",
            "note": "Case-insensitive, with no digit or '.' directly before or after the value. Unit spellings are not converted: '100nF' does not find '0.1uF'."
        },
        "ranking": {
            "criteria": policy.criteria(),
            "prefer_basic": policy.prefer_basic,
            "min_stock_count": policy.min_stock_count,
            "max_price_usd": policy.max_price
        },
        "count": counts.returned,
        "matched_count": counts.matched,
        "exclusions": {
            "below_min_stock_count": counts.below_min_stock,
            "unknown_price_count": counts.unknown_price,
            "above_max_price_count": counts.above_max_price
        },
        "alternatives": alternatives.into_iter().map(|candidate| candidate.part).collect::<Vec<_>>()
    });
    ctx.jlcpcb_cache.put(key, body.clone());

    let mut body = body;
    body["cached"] = json!(false);
    Ok(CallToolResult::text(serde_json::to_string(&body).unwrap()))
}

async fn handle_jlcpcb_stats(
    args: &serde_json::Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let db_path = resolve_db_path(args, ctx);
    if !db_path.exists() {
        return Ok(CallToolResult::text(
            serde_json::to_string(&json!({
                "exists": false,
                "note": "Run download_jlcpcb_database to fetch the parts database"
            }))
            .unwrap(),
        ));
    }

    let meta = tokio::fs::metadata(&db_path).await?;
    let size_bytes = meta.len();

    let count = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        move || -> anyhow::Result<i64> {
            let conn = rusqlite::Connection::open(&db_path)?;
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM components", [], |r| r.get(0))?;
            Ok(count)
        }
    })
    .await??;

    Ok(CallToolResult::text(
        serde_json::to_string(&json!({
            "exists": true,
            "path": db_path.to_str().unwrap_or(""),
            "size_bytes": size_bytes,
            "part_count": count
        }))
        .unwrap(),
    ))
}

async fn handle_enrich_datasheets(
    args: &serde_json::Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let sch_path = get_path(args, "schematic")?;
    let overwrite = args["overwrite_existing"].as_bool().unwrap_or(false);

    let read_path = sch_path.clone();
    let content = tokio::task::spawn_blocking(move || read_consistent(&read_path)).await??;

    // Only placed, top-level symbol instances are enrichment targets. Library
    // definitions, quoted text, and nested decoys must not enter the target
    // set merely because they contain the same property spelling.
    let symbols = match validated_datasheet_symbols(&content) {
        Ok(symbols) => symbols,
        Err(error) => return Ok(error.into_result()),
    };
    let mut lcsc_ids: Vec<String> = symbols
        .iter()
        .map(|symbol| symbol.lcsc_id.clone())
        .collect();
    lcsc_ids.sort();
    lcsc_ids.dedup();

    if lcsc_ids.is_empty() {
        return Ok(CallToolResult::text(
            serde_json::to_string(&json!({
                "updated": 0,
                "note": "No LCSC property found in schematic components"
            }))
            .unwrap(),
        ));
    }

    // Query LCSC API for datasheet URLs
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let mut urls = HashMap::new();

    for lcsc_id in &lcsc_ids {
        let url = format!(
            "https://wmsc.lcsc.com/ftps/wm/product/detail?productCode={}",
            lcsc_id
        );
        if let Ok(resp) = get_with_backoff(&client, &url).await {
            if resp.status().is_success() {
                if let Ok(json_resp) = resp.json::<serde_json::Value>().await {
                    if let Some(datasheet_url) = json_resp
                        .pointer("/result/dataManualUrl")
                        .and_then(|v| v.as_str())
                    {
                        urls.insert(lcsc_id.clone(), datasheet_url.to_owned());
                    }
                }
            }
        }
    }

    let plan = match plan_datasheet_enrichment(&content, &urls, overwrite) {
        Ok(plan) => plan,
        Err(error) => return Ok(error.into_result()),
    };

    let observed_updates = if let Some((command, planned)) = plan {
        if let Err(error) = commit_command(&sch_path, &command) {
            if let Some(refusal) = datasheet_commit_refusal(&sch_path, &error) {
                return Ok(refusal);
            }
            return Err(error.into());
        }
        let committed = read_consistent(&sch_path)?;
        match observe_datasheet_updates(&committed, &planned) {
            Ok(observed) => observed,
            Err(error) => return Ok(error.into_result()),
        }
    } else {
        Vec::new()
    };

    Ok(CallToolResult::text(
        serde_json::to_string(&json!({
            "lcsc_ids_found": lcsc_ids.len(),
            "datasheets_enriched": observed_updates.len(),
            "schematic": sch_path.to_str().unwrap_or("")
        }))
        .unwrap(),
    ))
}

#[derive(Debug, Clone)]
struct DatasheetSymbolTarget {
    uuid: String,
    reference: String,
    unit: u32,
    lcsc_id: String,
    datasheet: Option<String>,
}

#[derive(Debug, Clone)]
struct PlannedDatasheetUpdate {
    target: DatasheetSymbolTarget,
    datasheet_url: String,
}

#[derive(Debug)]
enum DatasheetTargetError {
    Ambiguous {
        target: String,
        candidates: Vec<String>,
    },
    Stale {
        target: String,
        reason: String,
    },
}

impl DatasheetTargetError {
    fn stale(target: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Stale {
            target: target.into(),
            reason: reason.into(),
        }
    }

    fn from_sexp(target: impl Into<String>, error: SexpError) -> Self {
        Self::stale(target, error.to_string())
    }

    fn into_result(self) -> CallToolResult {
        match self {
            Self::Ambiguous { target, candidates } => {
                let reason = format!(
                    "more than one candidate was observed: {}",
                    candidates.join(", ")
                );
                CallToolResult::error_kind(
                    ToolErrorKind::StaleTarget {
                        target: target.clone(),
                        reason: reason.clone(),
                    },
                    format!("cannot safely enrich {target}: {reason}"),
                )
            }
            Self::Stale { target, reason } => CallToolResult::error_kind(
                ToolErrorKind::StaleTarget {
                    target: target.clone(),
                    reason: reason.clone(),
                },
                format!("cannot safely enrich {target}: {reason}"),
            ),
        }
    }
}

fn direct_property_values(node: &SexpNode, name: &str) -> Vec<String> {
    node.find_all("property")
        .into_iter()
        .filter(|property| property.get(1).and_then(SexpNode::as_str) == Some(name))
        .filter_map(|property| property.get(2).and_then(SexpNode::as_str))
        .map(ToOwned::to_owned)
        .collect()
}

fn one_identity_value(
    values: Vec<String>,
    field: &str,
    item: &str,
) -> Result<String, DatasheetTargetError> {
    match values.as_slice() {
        [value] if !value.trim().is_empty() => Ok(value.clone()),
        [] | [_] => Err(DatasheetTargetError::stale(
            item,
            format!("{field} is missing or empty"),
        )),
        _ => Err(DatasheetTargetError::Ambiguous {
            target: format!("{field} on {item}"),
            candidates: values,
        }),
    }
}

fn validated_datasheet_symbols(
    content: &str,
) -> Result<Vec<DatasheetSymbolTarget>, DatasheetTargetError> {
    let ranges = find_direct_child_blocks(content, "kicad_sch");
    if ranges.is_empty() {
        return Err(DatasheetTargetError::stale(
            "schematic document",
            "the kicad_sch root is missing or malformed",
        ));
    }

    let mut uuid_owners: HashMap<String, Vec<String>> = HashMap::new();
    let mut symbols = Vec::new();
    for (start, end) in ranges {
        let node = parse_sexp(&content[start..end])
            .map_err(|error| DatasheetTargetError::from_sexp("schematic item", error))?;
        let kind = node.head().unwrap_or("unknown");
        if let Some(uuid) = node
            .find("uuid")
            .and_then(|uuid| uuid.get(1))
            .and_then(SexpNode::as_str)
        {
            uuid_owners
                .entry(uuid.to_owned())
                .or_default()
                .push(format!("{kind} at byte {start}"));
        }
        if kind != "symbol" {
            continue;
        }

        let lcsc = direct_property_values(&node, "LCSC");
        if lcsc.is_empty() {
            continue;
        }
        let item = format!("symbol at byte {start}");
        let lcsc_id = one_identity_value(lcsc, "LCSC", &item)?;
        let reference = one_identity_value(
            direct_property_values(&node, "Reference"),
            "Reference",
            &item,
        )?;
        let uuid = node
            .find("uuid")
            .and_then(|uuid| uuid.get(1))
            .and_then(SexpNode::as_str)
            .filter(|uuid| !uuid.trim().is_empty())
            .map(ToOwned::to_owned)
            .ok_or_else(|| DatasheetTargetError::stale(&item, "UUID is missing or empty"))?;
        let units = node.find_all("unit");
        let unit = match units.as_slice() {
            [] => 1,
            [unit] => unit
                .get(1)
                .and_then(SexpNode::as_str)
                .and_then(|unit| unit.parse::<u32>().ok())
                .filter(|unit| *unit > 0)
                .ok_or_else(|| DatasheetTargetError::stale(&item, "unit is invalid"))?,
            _ => {
                return Err(DatasheetTargetError::Ambiguous {
                    target: format!("unit on {item}"),
                    candidates: units
                        .iter()
                        .filter_map(|unit| unit.get(1).and_then(SexpNode::as_str))
                        .map(ToOwned::to_owned)
                        .collect(),
                })
            }
        };
        let datasheets = direct_property_values(&node, "Datasheet");
        let datasheet = match datasheets.as_slice() {
            [] => None,
            [value] => Some(value.clone()),
            _ => {
                return Err(DatasheetTargetError::Ambiguous {
                    target: format!("Datasheet on {reference} unit {unit}"),
                    candidates: datasheets,
                })
            }
        };
        symbols.push(DatasheetSymbolTarget {
            uuid,
            reference,
            unit,
            lcsc_id,
            datasheet,
        });
    }

    for symbol in &symbols {
        if let Some(owners) = uuid_owners
            .get(&symbol.uuid)
            .filter(|owners| owners.len() > 1)
        {
            return Err(DatasheetTargetError::Ambiguous {
                target: format!("schematic UUID {}", symbol.uuid),
                candidates: owners.clone(),
            });
        }
    }

    let mut placed: HashMap<(String, u32), Vec<String>> = HashMap::new();
    for symbol in &symbols {
        placed
            .entry((symbol.reference.clone(), symbol.unit))
            .or_default()
            .push(symbol.uuid.clone());
    }
    if let Some(((reference, unit), uuids)) = placed.iter().find(|(_, uuids)| uuids.len() > 1) {
        return Err(DatasheetTargetError::Ambiguous {
            target: format!("symbol {reference} unit {unit}"),
            candidates: uuids.clone(),
        });
    }

    Ok(symbols)
}

fn plan_datasheet_enrichment(
    content: &str,
    urls: &HashMap<String, String>,
    overwrite: bool,
) -> Result<Option<(SchematicCommand, Vec<PlannedDatasheetUpdate>)>, DatasheetTargetError> {
    let symbols = validated_datasheet_symbols(content)?;
    let mut edited = content.to_owned();
    let mut planned = Vec::new();
    let path = Path::new("datasheet-enrichment.kicad_sch");

    for symbol in symbols {
        let Some(datasheet_url) = urls.get(&symbol.lcsc_id) else {
            continue;
        };
        let Some(existing) = &symbol.datasheet else {
            continue;
        };
        if (!overwrite && existing != "~" && !existing.is_empty()) || existing == datasheet_url {
            continue;
        }
        let id = ItemId::new(symbol.uuid.clone())
            .map_err(|error| DatasheetTargetError::from_sexp(&symbol.reference, error))?;
        let command = SchematicCommand::set_property(
            &edited,
            id,
            "Datasheet",
            datasheet_url,
            format!(
                "enrich datasheet for {} unit {}",
                symbol.reference, symbol.unit
            ),
        )
        .map_err(|error| DatasheetTargetError::from_sexp(&symbol.reference, error))?;
        edited = prepare_command(path, &edited, &command)
            .map_err(|error| DatasheetTargetError::from_sexp(&symbol.reference, error))?
            .0;
        planned.push(PlannedDatasheetUpdate {
            target: symbol,
            datasheet_url: datasheet_url.clone(),
        });
    }

    if planned.is_empty() {
        return Ok(None);
    }
    let ids = planned
        .iter()
        .map(|update| ItemId::new(update.target.uuid.clone()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| DatasheetTargetError::from_sexp("datasheet targets", error))?;
    let command = SchematicCommand::replace_items_from_document(
        content,
        &edited,
        ids,
        "enrich schematic datasheets",
    )
    .map_err(|error| DatasheetTargetError::from_sexp("datasheet targets", error))?
    .requiring_unchanged_document();
    Ok(Some((command, planned)))
}

fn observe_datasheet_updates(
    content: &str,
    planned: &[PlannedDatasheetUpdate],
) -> Result<Vec<serde_json::Value>, DatasheetTargetError> {
    let symbols = validated_datasheet_symbols(content)?;
    let mut observed = Vec::with_capacity(planned.len());
    for update in planned {
        let target = symbols
            .iter()
            .find(|symbol| symbol.uuid == update.target.uuid)
            .ok_or_else(|| {
                DatasheetTargetError::stale(
                    format!("symbol UUID {}", update.target.uuid),
                    "the committed symbol is missing",
                )
            })?;
        if target.reference != update.target.reference
            || target.unit != update.target.unit
            || target.lcsc_id != update.target.lcsc_id
        {
            return Err(DatasheetTargetError::stale(
                format!("symbol UUID {}", update.target.uuid),
                "committed identity differs from the planned reference, unit, or LCSC ID",
            ));
        }
        if target.datasheet.as_deref() != Some(&update.datasheet_url) {
            return Err(DatasheetTargetError::stale(
                format!("symbol {} unit {}", target.reference, target.unit),
                format!(
                    "committed Datasheet was {:?}, expected {}",
                    target.datasheet, update.datasheet_url
                ),
            ));
        }
        observed.push(json!({
            "uuid": target.uuid,
            "reference": target.reference,
            "unit": target.unit,
            "lcsc_id": target.lcsc_id,
            "datasheet_url": target.datasheet
        }));
    }
    Ok(observed)
}

fn datasheet_commit_refusal(path: &Path, error: &SexpError) -> Option<CallToolResult> {
    let reason = match error {
        SexpError::Conflict { .. } => "the schematic changed after enrichment was planned",
        SexpError::ItemConflict { reason, .. } => reason,
        SexpError::KiCadEditorLocked { .. } => {
            "KiCad owns the schematic; use a live editor mutation or close the document"
        }
        _ => return None,
    };
    Some(CallToolResult::error_kind(
        ToolErrorKind::StaleTarget {
            target: path.display().to_string(),
            reason: reason.to_owned(),
        },
        format!(
            "cannot commit datasheet enrichment for {}: {reason}",
            path.display()
        ),
    ))
}

/// The datasheet URL from the local JLCPCB catalog, keyed by LCSC ID or MPN.
/// The database Konnect already downloads carries the exact LCSC PDF URL in
/// its `Datasheet` column for most parts, so a hit here answers without a
/// network round trip. `Ok(None)` is an ordinary miss; `Err` means the
/// database could not be read at all.
async fn datasheet_from_catalog(
    args: &serde_json::Value,
    ctx: &ToolContext,
    lcsc_id: Option<&str>,
    mpn: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let db_path = resolve_db_path(args, ctx);
    if !db_path.exists() {
        return Ok(None);
    }
    let lcsc_id = lcsc_id.map(String::from);
    let mpn = mpn.map(String::from);
    tokio::task::spawn_blocking(move || -> anyhow::Result<Option<String>> {
        let conn = rusqlite::Connection::open(&db_path)?;
        let (sql, param): (&str, String) = match (&lcsc_id, &mpn) {
            // An LCSC ID is the primary key shape, so it wins when both are
            // given — same precedence the web path used.
            (Some(id), _) => (
                "SELECT Datasheet FROM components WHERE LCSC = ?1 LIMIT 1",
                id.clone(),
            ),
            (None, Some(mpn)) => (
                "SELECT Datasheet FROM components WHERE MFR_Part = ?1 LIMIT 1",
                mpn.clone(),
            ),
            // Unreachable: the caller rejects both-absent before calling.
            (None, None) => return Ok(None),
        };
        let mut stmt = conn.prepare(sql)?;
        let mut rows = stmt.query_map(rusqlite::params![param], |row| row.get::<_, String>(0))?;
        Ok(rows.next().transpose()?.filter(|url| !url.is_empty()))
    })
    .await?
}

async fn handle_get_datasheet_url(
    args: &serde_json::Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let mpn = args["mpn"].as_str();
    let lcsc_id = args["lcsc_id"].as_str();

    if mpn.is_none() && lcsc_id.is_none() {
        return Ok(CallToolResult::error("Provide either 'mpn' or 'lcsc_id'"));
    }

    // The local catalog first: it already holds exact URLs, so a hit saves a
    // network round trip and works offline. Only a miss falls through to the
    // live LCSC API.
    if let Ok(Some(url)) = datasheet_from_catalog(args, ctx, lcsc_id, mpn).await {
        return Ok(CallToolResult::text(
            serde_json::to_string(&json!({
                "mpn": mpn,
                "lcsc_id": lcsc_id,
                "datasheet_url": url,
                "source": "local_catalog"
            }))
            .unwrap(),
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    // Try LCSC API with lcsc_id first
    if let Some(id) = lcsc_id {
        let url = format!(
            "https://wmsc.lcsc.com/ftps/wm/product/detail?productCode={}",
            id
        );
        if let Ok(resp) = get_with_backoff(&client, &url).await {
            if resp.status().is_success() {
                if let Ok(json_resp) = resp.json::<serde_json::Value>().await {
                    if let Some(ds_url) = json_resp
                        .pointer("/result/dataManualUrl")
                        .and_then(|v| v.as_str())
                    {
                        return Ok(CallToolResult::text(
                            serde_json::to_string(&json!({
                                "lcsc_id": id,
                                "datasheet_url": ds_url,
                                "source": "lcsc_api"
                            }))
                            .unwrap(),
                        ));
                    }
                }
            }
        }
    }

    // Both sources missed — say which were tried, not just that the answer
    // is null (#255).
    let mut tried = vec![];
    if resolve_db_path(&serde_json::Value::Null, ctx).exists() {
        tried.push("local catalog");
    } else {
        tried.push("local catalog (database not downloaded)");
    }
    if lcsc_id.is_some() {
        tried.push("LCSC API");
    }
    Ok(CallToolResult::text(
        serde_json::to_string(&json!({
            "mpn": mpn,
            "lcsc_id": lcsc_id,
            "datasheet_url": null,
            "note": format!("Datasheet not found via {}", tried.join(" or "))
        }))
        .unwrap(),
    ))
}

// ─── Freerouting ──────────────────────────────────────────────────────────────

fn is_freerouting_jar(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("jar"))
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.to_ascii_lowercase().contains("freerouting"))
}

fn find_freerouting_jar_below(root: &Path, remaining_depth: usize) -> Option<PathBuf> {
    if is_freerouting_jar(root) {
        return Some(root.to_path_buf());
    }
    if remaining_depth == 0 || !root.is_dir() {
        return None;
    }

    let mut entries = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort();

    entries
        .iter()
        .find(|path| is_freerouting_jar(path))
        .cloned()
        .or_else(|| {
            entries.into_iter().find_map(|path| {
                let file_type = std::fs::symlink_metadata(&path).ok()?.file_type();
                file_type
                    .is_dir()
                    .then(|| find_freerouting_jar_below(&path, remaining_depth - 1))
                    .flatten()
            })
        })
}

/// Where Freerouting discovery looks when the caller passes no `jar_path`, in
/// the order it looks. The concrete places come from the tables below, and
/// both the search and the `jar_path` description are built from those tables,
/// so what the schema says and what the search does cannot drift apart. There
/// is no configuration setting: the schema once said "uses config default" for
/// one that never existed (#787).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum FreeroutingLocation {
    KicadThirdPartyVariable,
    KicadPluginFolder,
    WorkingDirectory,
    SystemDirectory,
    PathDirectory,
}

/// The variables naming a KiCad third-party directory, newest KiCad first.
const FREEROUTING_THIRD_PARTY_VARIABLES: [&str; 3] =
    ["KICAD10_3RD_PARTY", "KICAD9_3RD_PARTY", "KICAD8_3RD_PARTY"];

/// The KiCad versions whose plugin folders are searched, newest first.
const FREEROUTING_KICAD_VERSIONS: [&str; 3] = ["10.0", "9.0", "8.0"];

/// Plugin folders below a home directory: the variable naming the home, the
/// folder below it (`<version>` stands for each version), and whether it is
/// searched on Windows only.
const FREEROUTING_PLUGIN_FOLDERS: [(&str, &str, bool); 3] = [
    ("HOME", "Documents/KiCad/<version>/3rdparty/plugins", false),
    (
        "HOME",
        ".local/share/kicad/<version>/3rdparty/plugins",
        false,
    ),
    (
        "USERPROFILE",
        "Documents/KiCad/<version>/3rdparty/plugins",
        true,
    ),
];

/// The JAR looked for in the server's working directory.
const FREEROUTING_WORKING_DIRECTORY_JAR: &str = "freerouting.jar";

/// System install directories.
const FREEROUTING_SYSTEM_DIRECTORIES: [&str; 2] =
    ["/usr/local/lib/freerouting", "/opt/freerouting"];

/// The variable whose directories are searched last.
const FREEROUTING_PATH_VARIABLE: &str = "PATH";

impl FreeroutingLocation {
    /// Every location, in search order.
    const ALL: [Self; 5] = [
        Self::KicadThirdPartyVariable,
        Self::KicadPluginFolder,
        Self::WorkingDirectory,
        Self::SystemDirectory,
        Self::PathDirectory,
    ];

    fn kind(self) -> &'static str {
        match self {
            Self::KicadThirdPartyVariable => "kicad_3rd_party_variable",
            Self::KicadPluginFolder => "kicad_plugin_folder",
            Self::WorkingDirectory => "working_directory",
            Self::SystemDirectory => "system_directory",
            Self::PathDirectory => "path_directory",
        }
    }

    /// How many directory levels below one of these roots are searched.
    fn depth(self) -> usize {
        match self {
            Self::KicadThirdPartyVariable | Self::KicadPluginFolder => 5,
            Self::WorkingDirectory => 0,
            Self::SystemDirectory => 3,
            Self::PathDirectory => 1,
        }
    }

    fn description(self) -> String {
        match self {
            Self::KicadThirdPartyVariable => format!(
                "the directories named by {}",
                english_list(
                    FREEROUTING_THIRD_PARTY_VARIABLES
                        .iter()
                        .map(|v| v.to_string())
                )
            ),
            Self::KicadPluginFolder => format!(
                "the KiCad {} plugin folders {}",
                english_list(FREEROUTING_KICAD_VERSIONS.iter().map(|v| v.to_string())),
                english_list(FREEROUTING_PLUGIN_FOLDERS.iter().map(
                    |(home, folder, windows_only)| {
                        let place = format!("${home}/{folder}");
                        if *windows_only {
                            format!("{place} (Windows only)")
                        } else {
                            place
                        }
                    }
                ))
            ),
            Self::WorkingDirectory => {
                format!("{FREEROUTING_WORKING_DIRECTORY_JAR} in the server's working directory")
            }
            Self::SystemDirectory => {
                english_list(FREEROUTING_SYSTEM_DIRECTORIES.iter().map(|d| d.to_string()))
            }
            Self::PathDirectory => format!("every directory on {FREEROUTING_PATH_VARIABLE}"),
        }
    }
}

/// `a`, `a and b`, or `a, b and c`.
fn english_list(items: impl Iterator<Item = String>) -> String {
    let items: Vec<String> = items.collect();
    match items.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn freerouting_jar_path_description() -> String {
    let locations = FreeroutingLocation::ALL
        .iter()
        .map(|location| location.description())
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "Path to a Freerouting JAR (optional). When omitted, Konnect searches, in order: \
         {locations}. Relative paths are taken from the server's working directory. \
         There is no configuration setting for it."
    )
}

/// One place discovery looked, and how many directory levels below it.
#[derive(Debug, Clone, PartialEq)]
struct FreeroutingSearchRoot {
    location: FreeroutingLocation,
    /// Always absolute: a relative value from the environment is taken from
    /// the server's working directory, which is where the search reads it.
    path: PathBuf,
    depth: usize,
}

impl FreeroutingSearchRoot {
    fn evidence(&self) -> serde_json::Value {
        json!({
            "kind": self.location.kind(),
            "location_path": self.path,
            "max_depth": self.depth,
            "exists": self.path.exists(),
        })
    }
}

fn freerouting_search_roots() -> Vec<FreeroutingSearchRoot> {
    freerouting_search_roots_from(
        |name| std::env::var_os(name),
        &std::env::current_dir().unwrap_or_default(),
    )
}

/// The search roots for a given environment, so a test can supply one without
/// touching the process's own.
fn freerouting_search_roots_from(
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
    working_directory: &Path,
) -> Vec<FreeroutingSearchRoot> {
    let mut roots: Vec<FreeroutingSearchRoot> = Vec::new();
    let mut push = |location: FreeroutingLocation, path: PathBuf| {
        let path = if path.is_absolute() {
            path
        } else {
            working_directory.join(path)
        };
        // `HOME` and `USERPROFILE` are often the same directory (Git Bash on
        // Windows sets both), and searching a root twice only repeats it in
        // the evidence.
        if !roots.iter().any(|root| root.path == path) {
            roots.push(FreeroutingSearchRoot {
                location,
                depth: location.depth(),
                path,
            })
        }
    };
    let below = |base: PathBuf, folder: &str| folder.split('/').fold(base, |p, part| p.join(part));

    for variable in FREEROUTING_THIRD_PARTY_VARIABLES {
        if let Some(path) = var(variable) {
            push(
                FreeroutingLocation::KicadThirdPartyVariable,
                PathBuf::from(path),
            );
        }
    }

    let mut homes: Vec<&str> = FREEROUTING_PLUGIN_FOLDERS
        .iter()
        .map(|(home, _, _)| *home)
        .collect();
    homes.dedup();
    for home_variable in homes {
        let Some(home) = var(home_variable).map(PathBuf::from) else {
            continue;
        };
        for version in FREEROUTING_KICAD_VERSIONS {
            for (variable, folder, windows_only) in FREEROUTING_PLUGIN_FOLDERS {
                if variable != home_variable || (windows_only && !cfg!(target_os = "windows")) {
                    continue;
                }
                push(
                    FreeroutingLocation::KicadPluginFolder,
                    below(home.clone(), &folder.replace("<version>", version)),
                );
            }
        }
    }

    push(
        FreeroutingLocation::WorkingDirectory,
        PathBuf::from(FREEROUTING_WORKING_DIRECTORY_JAR),
    );
    for directory in FREEROUTING_SYSTEM_DIRECTORIES {
        push(
            FreeroutingLocation::SystemDirectory,
            PathBuf::from(directory),
        );
    }
    if let Some(path) = var(FREEROUTING_PATH_VARIABLE) {
        for directory in std::env::split_paths(&path) {
            push(FreeroutingLocation::PathDirectory, directory);
        }
    }
    roots
}

/// What looking for the Freerouting JAR found.
#[derive(Debug, PartialEq)]
enum FreeroutingJar {
    Found(PathBuf),
    /// The caller passed `jar_path`, and it is not a file.
    SuppliedPathIsNotAFile(PathBuf),
    /// Discovery searched every root and found no JAR.
    NotFound(Vec<FreeroutingSearchRoot>),
}

fn resolve_freerouting_jar(args: &serde_json::Value) -> FreeroutingJar {
    if let Some(path) = args["jar_path"].as_str() {
        let path = PathBuf::from(path);
        return if path.is_file() {
            FreeroutingJar::Found(path)
        } else {
            FreeroutingJar::SuppliedPathIsNotAFile(path)
        };
    }

    let roots = freerouting_search_roots();
    match roots
        .iter()
        .find_map(|root| find_freerouting_jar_below(&root.path, root.depth))
    {
        Some(jar) => FreeroutingJar::Found(jar),
        None => FreeroutingJar::NotFound(roots),
    }
}

fn command_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    [stdout.trim(), stderr.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod datasheet_enrichment_tests {
    use super::*;
    use crate::mcp::error::extract_error_kind;
    use std::io::Write;

    const KICAD_STRUCTURAL_FIXTURE: &str =
        include_str!("../../tests/fixtures/structural_scans_kicad10.kicad_sch");

    fn symbol(reference: &str, unit: u32, uuid: Option<&str>, lcsc: &str) -> String {
        let uuid = uuid
            .map(|uuid| format!("\r\n\t\t(uuid \"{uuid}\")"))
            .unwrap_or_default();
        format!(
            "\t(symbol\r\n\t\t(lib_id \"Device:R\")\r\n\t\t(at 10 20 0)\r\n\t\t(unit {unit})\r\n\t\t(property \"Reference\" \"{reference}\")\r\n\t\t(property \"Datasheet\" \"~\")\r\n\t\t(property \"Notes\" \"decoy\"\r\n\t\t\t(property \"LCSC\" \"C-DECOY\")\r\n\t\t)\r\n\t\t(property \"LCSC\" \"{lcsc}\"){uuid}\r\n\t)\r\n"
        )
    }

    fn schematic(body: &str) -> String {
        format!("(kicad_sch\r\n\t(version 20231120)\r\n{body})\r\n")
    }

    fn urls(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(id, url)| ((*id).to_owned(), (*url).to_owned()))
            .collect()
    }

    #[test]
    fn tab_crlf_multi_unit_symbols_update_atomically_and_read_back() {
        let decoy = "\t(lib_symbols\r\n\t\t(symbol \"decoy\"\r\n\t\t\t(property \"LCSC\" \"C-LIBRARY\")\r\n\t\t)\r\n\t)\r\n";
        let original = schematic(&format!(
            "{decoy}{}{}",
            symbol("U1", 1, Some("unit-1"), "C1"),
            symbol("U1", 2, Some("unit-2"), "C1")
        ));
        let Some((command, planned)) = plan_datasheet_enrichment(
            &original,
            &urls(&[("C1", "https://example.com/u1.pdf")]),
            false,
        )
        .unwrap() else {
            panic!("expected enrichment plan");
        };
        assert_eq!(planned.len(), 2);

        let mut file = tempfile::NamedTempFile::with_suffix(".kicad_sch").unwrap();
        file.write_all(original.as_bytes()).unwrap();
        file.flush().unwrap();
        let outcome = commit_command(file.path(), &command).unwrap();
        assert!(outcome.changed);

        let committed = std::fs::read_to_string(file.path()).unwrap();
        let observed = observe_datasheet_updates(&committed, &planned).unwrap();
        assert_eq!(observed.len(), 2);
        assert_eq!(observed[0]["reference"], "U1");
        assert_eq!(observed[0]["unit"], 1);
        assert_eq!(observed[1]["unit"], 2);
        assert!(observed
            .iter()
            .all(|update| update["datasheet_url"] == "https://example.com/u1.pdf"));
        assert!(committed.contains("\r\n\t\t(property \"Notes\" \"decoy\"\r\n"));
        assert!(committed.contains("(property \"LCSC\" \"C-LIBRARY\")"));
        assert!(committed.contains("(property \"LCSC\" \"C-DECOY\")"));
    }

    #[test]
    fn kicad_authored_symbol_is_enriched_structurally_and_read_back() {
        let Some((command, planned)) = plan_datasheet_enrichment(
            KICAD_STRUCTURAL_FIXTURE,
            &urls(&[("C25804", "https://example.com/c25804.pdf")]),
            false,
        )
        .unwrap() else {
            panic!("expected enrichment plan");
        };
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].target.reference, "R1");

        let mut file = tempfile::NamedTempFile::with_suffix(".kicad_sch").unwrap();
        file.write_all(KICAD_STRUCTURAL_FIXTURE.as_bytes()).unwrap();
        file.flush().unwrap();
        let outcome = commit_command(file.path(), &command).unwrap();
        assert!(outcome.changed);

        let committed = std::fs::read_to_string(file.path()).unwrap();
        let observed = observe_datasheet_updates(&committed, &planned).unwrap();
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0]["reference"], "R1");
        assert_eq!(observed[0]["lcsc_id"], "C25804");
        assert_eq!(
            observed[0]["datasheet_url"],
            "https://example.com/c25804.pdf"
        );
        parse_sexp(&committed).expect("enriched KiCad fixture must still parse");
    }

    #[test]
    fn nested_or_library_lcsc_text_is_not_a_symbol_target() {
        let content = schematic(
            "\t(lib_symbols\n\t\t(symbol \"X\" (property \"LCSC\" \"C1\"))\n\t)\n\t(text \"(property LCSC C1)\" (at 0 0) (uuid \"text-1\"))\n",
        );
        assert!(validated_datasheet_symbols(&content).unwrap().is_empty());
        assert!(plan_datasheet_enrichment(
            &content,
            &urls(&[("C1", "https://example.com/decoy.pdf")]),
            false
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn missing_uuid_is_a_stale_target() {
        let content = schematic(&symbol("R1", 1, None, "C1"));
        let error = validated_datasheet_symbols(&content).unwrap_err();
        assert!(matches!(error, DatasheetTargetError::Stale { .. }));
    }

    #[test]
    fn duplicate_reference_and_unit_is_a_stale_refusal() {
        let content = schematic(&format!(
            "{}{}",
            symbol("U1", 1, Some("first"), "C1"),
            symbol("U1", 1, Some("second"), "C2")
        ));
        let error = validated_datasheet_symbols(&content).unwrap_err();
        assert!(matches!(&error, DatasheetTargetError::Ambiguous { .. }));
        assert_eq!(
            extract_error_kind(&error.into_result()).as_deref(),
            Some("stale_target")
        );
    }

    #[test]
    fn duplicate_uuid_is_a_stale_refusal_even_across_item_kinds() {
        let content = schematic(&format!(
            "{}\t(junction (at 1 2) (uuid \"shared\"))\r\n",
            symbol("R1", 1, Some("shared"), "C1")
        ));
        let error = validated_datasheet_symbols(&content).unwrap_err();
        assert!(matches!(&error, DatasheetTargetError::Ambiguous { .. }));
        assert_eq!(
            extract_error_kind(&error.into_result()).as_deref(),
            Some("stale_target")
        );
    }

    #[test]
    fn stale_document_revision_refuses_without_overwriting() {
        let original = schematic(&symbol("R1", 1, Some("r1"), "C1"));
        let Some((command, _)) = plan_datasheet_enrichment(
            &original,
            &urls(&[("C1", "https://example.com/r1.pdf")]),
            false,
        )
        .unwrap() else {
            panic!("expected enrichment plan");
        };
        let mut file = tempfile::NamedTempFile::with_suffix(".kicad_sch").unwrap();
        let newer = original.replace("(at 10 20 0)", "(at 11 20 0)");
        file.write_all(newer.as_bytes()).unwrap();
        file.flush().unwrap();

        let error = commit_command(file.path(), &command).unwrap_err();
        let refusal = datasheet_commit_refusal(file.path(), &error).unwrap();
        assert_eq!(
            extract_error_kind(&refusal).as_deref(),
            Some("stale_target")
        );
        assert_eq!(std::fs::read_to_string(file.path()).unwrap(), newer);
    }

    #[test]
    fn overwrite_policy_and_noop_counts_are_truthful() {
        let original = schematic(&symbol("R1", 1, Some("r1"), "C1")).replace(
            "(property \"Datasheet\" \"~\")",
            "(property \"Datasheet\" \"https://existing.example/r1.pdf\")",
        );
        let replacements = urls(&[("C1", "https://new.example/r1.pdf")]);
        assert!(plan_datasheet_enrichment(&original, &replacements, false)
            .unwrap()
            .is_none());
        assert!(plan_datasheet_enrichment(&original, &replacements, true)
            .unwrap()
            .is_some());

        let same = urls(&[("C1", "https://existing.example/r1.pdf")]);
        assert!(plan_datasheet_enrichment(&original, &same, true)
            .unwrap()
            .is_none());
    }
}

async fn run_java_command(
    command: &mut tokio::process::Command,
) -> Result<std::process::Output, String> {
    command.kill_on_drop(true);
    match tokio::time::timeout(std::time::Duration::from_secs(10), command.output()).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("Java command timed out after 10 seconds".to_string()),
    }
}

/// Freerouting arguments that keep a probe quiet: no GUI, no telemetry, no
/// analytics (2.x turns both on by default).
const FREEROUTING_QUIET_ARGUMENTS: &[&str] = &[
    "--gui.enabled=false",
    "--profile.allow_telemetry=false",
    "--usage_and_diagnostic_data.disable_analytics=true",
];

/// JVM options that keep a Freerouting start off the network. 2.5.0 starts a
/// version check on every launch (a GET to `api.github.com`, with no switch
/// to turn it off), through Java's default proxy selector. Every HTTP and
/// HTTPS request goes to a closed loopback port instead and fails at once;
/// loopback stays direct.
const FREEROUTING_OFFLINE_JVM_ARGUMENTS: &[&str] = &[
    "-Dhttp.proxyHost=127.0.0.1",
    "-Dhttp.proxyPort=9",
    "-Dhttps.proxyHost=127.0.0.1",
    "-Dhttps.proxyPort=9",
    "-Dhttp.nonProxyHosts=localhost|127.*|[::1]",
];

/// `version_policy` for an installed JAR. The JAR is hashed only when it
/// claims the recommended version, so a different version costs nothing.
async fn installed_version_policy(jar: &Path, version_output: Option<&str>) -> serde_json::Value {
    let claimed = version_output
        .and_then(crate::freerouting_setup::version_in_output)
        .or_else(|| crate::freerouting_setup::version_in_file_name(jar));
    let sha = if claimed.as_deref().is_some_and(|version| {
        crate::freerouting_setup::same_version(
            version,
            crate::freerouting_setup::RECOMMENDED.version,
        )
    }) {
        let path = jar.to_path_buf();
        tokio::task::spawn_blocking(move || crate::freerouting_setup::file_sha256(&path).ok())
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    crate::freerouting_setup::version_policy(Some(jar), version_output, sha.as_deref())
}

/// Keeps a Freerouting child's settings, log and data inside `home`, whatever
/// the owner's environment says. 2.5's `UserDataPathResolver` lets
/// `FREEROUTING__USER_DATA_PATH` and `FREEROUTING__LOGGING__FILE__LOCATION`
/// override `user.home`, and `AppPaths` lets the XDG variables and (on
/// Windows) `APPDATA` and `LOCALAPPDATA` do the same. So every inherited
/// `FREEROUTING__*` variable (`inherited` lists the names) is removed and the
/// folder variables point into `home`.
fn isolate_freerouting_environment(
    command: &mut tokio::process::Command,
    home: &Path,
    inherited: impl IntoIterator<Item = std::ffi::OsString>,
) {
    for name in inherited {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("FREEROUTING__")
        {
            command.env_remove(name);
        }
    }
    for (name, folder) in [
        ("XDG_CONFIG_HOME", ".config"),
        ("XDG_STATE_HOME", ".local/state"),
        ("XDG_CACHE_HOME", ".cache"),
        ("XDG_DATA_HOME", ".local/share"),
        ("APPDATA", "AppData/Roaming"),
        ("LOCALAPPDATA", "AppData/Local"),
    ] {
        command.env(name, home.join(folder));
    }
}

#[cfg(test)]
thread_local! {
    /// A test sets this to see the answer when the probe's scratch folder
    /// cannot be made.
    static PROBE_SCRATCH_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The scratch folder the version probe runs in.
fn version_probe_scratch() -> std::io::Result<tempfile::TempDir> {
    #[cfg(test)]
    if PROBE_SCRATCH_FAILS.with(std::cell::Cell::get) {
        return Err(std::io::Error::other("no space left on device (test)"));
    }
    tempfile::tempdir()
}

async fn handle_check_freerouting(
    args: &serde_json::Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    match resolve_freerouting_jar(args) {
        FreeroutingJar::SuppliedPathIsNotAFile(path) => Ok(CallToolResult::json(&json!({
            "available": false,
            "engine_found": false,
            "native_mcp_available": false,
            "bridge_available": false,
            "checked_path": path,
            "note": format!("jar_path is not a file: {}", path.display())
        }))),
        FreeroutingJar::NotFound(roots) => Ok(CallToolResult::json(&json!({
            "available": false,
            "engine_found": false,
            "native_mcp_available": false,
            "bridge_available": false,
            "searched_locations": roots.iter().map(FreeroutingSearchRoot::evidence).collect::<Vec<_>>(),
            "note": "No Freerouting JAR was found in any searched location. Install Freerouting \
                     (KiCad's Plugin and Content Manager, or \
                     https://github.com/freerouting/freerouting/releases), or pass jar_path.",
            "version_policy": crate::freerouting_setup::version_policy(None, None, None)
        }))),
        FreeroutingJar::Found(jar_path) => {
            let requirement = crate::freerouting_setup::jar_requirement(&jar_path);
            let java_path = args["java_path"].as_str().map(PathBuf::from);
            let found = crate::freerouting_setup::discover_java(java_path.as_deref()).await;
            let setup =
                crate::freerouting_setup::guidance(Some(&jar_path), Some(&requirement), &found);
            let Some(java) =
                crate::freerouting_setup::pick(&found, requirement.required_java_major)
            else {
                // A Java is there, or none is, but not one this JAR can start on.
                let seen = if found.is_empty() {
                    "no Java was found".to_string()
                } else {
                    format!("found Java {}", {
                        let mut majors: Vec<u32> =
                            found.iter().filter_map(|java| java.major).collect();
                        majors.sort_unstable();
                        majors.dedup();
                        majors
                            .iter()
                            .map(u32::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                };
                return Ok(CallToolResult::json(&json!({
                    "available": false,
                    "engine_found": true,
                    "native_mcp_available": false,
                    "bridge_available": false,
                    "jar_path": jar_path,
                    "java_available": !found.is_empty(),
                    "java_sufficient": false,
                    "java_output": found.first().map(|java| java.version_output.clone()),
                    "required_java_major": requirement.required_java_major,
                    "note": format!(
                        "Freerouting was found, but it needs Java {} or newer ({}); {seen}. The steps are in `setup`.",
                        requirement.required_java_major, requirement.basis
                    ),
                    "setup": setup,
                    "version_policy": installed_version_policy(&jar_path, None).await
                })));
            };

            // Headless, with no GUI and telemetry off: without these, 2.1.0
            // opens its window after printing the version and never exits,
            // and 2.x turns telemetry on by default. A version run with the
            // GUI off exits 1 because it was given no board; the version line
            // it printed is the proof that the JAR started on this Java, as
            // long as nothing after it says the start-up then crashed.
            //
            // Its own home, temp and settings folders: 2.x writes settings
            // and a log there (2.5 under ~/Library), which a probe should not
            // leave in the owner's. Without a scratch folder the JAR is not
            // started, rather than started in the owner's home.
            let scratch = match version_probe_scratch() {
                Ok(scratch) => scratch,
                Err(error) => {
                    return Ok(CallToolResult::json(&json!({
                        "available": false,
                        "engine_found": true,
                        "native_mcp_available": false,
                        "bridge_available": false,
                        "jar_path": jar_path,
                        "java_available": true,
                        "java_sufficient": true,
                        "java_command": java.command,
                        "java_major": java.major,
                        "required_java_major": requirement.required_java_major,
                        "java_output": java.version_output,
                        "version_checked": false,
                        "note": format!(
                            "Freerouting was not started: a scratch folder for its version check could not be made ({error}). The JAR and the Java were found; free disk space or fix the temp folder and check again."
                        ),
                        "setup": setup,
                        "version_policy": installed_version_policy(&jar_path, None).await
                    })));
                }
            };
            let mut freerouting_command = tokio::process::Command::new(&java.command);
            freerouting_command
                .arg("-Djava.awt.headless=true")
                .arg(format!("-Duser.home={}", scratch.path().display()))
                .arg(format!("-Djava.io.tmpdir={}", scratch.path().display()))
                .args(FREEROUTING_OFFLINE_JVM_ARGUMENTS)
                .args(["-jar", jar_path.to_str().unwrap_or(""), "--version"])
                .args(FREEROUTING_QUIET_ARGUMENTS);
            isolate_freerouting_environment(
                &mut freerouting_command,
                scratch.path(),
                std::env::vars_os().map(|(name, _)| name),
            );
            let freerouting = run_java_command(&mut freerouting_command).await;
            let (version_checked, version_output) = match freerouting {
                Ok(output) => {
                    let text = command_output(&output);
                    let named = crate::freerouting_setup::version_in_output(&text).is_some();
                    (
                        output.status.success()
                            || (named && !crate::freerouting_setup::startup_crashed(&text)),
                        text,
                    )
                }
                Err(error) => (false, error),
            };
            let version_policy =
                installed_version_policy(&jar_path, version_checked.then_some(&*version_output))
                    .await;
            let bridge = crate::freerouting_mcp::probe_local(&jar_path, Some(&java.command)).await;
            Ok(CallToolResult::json(&json!({
                "available": version_checked,
                "engine_found": true,
                "native_mcp_available": bridge.native_mcp_available,
                "bridge_available": bridge.bridge_available,
                "jar_path": jar_path,
                "java_available": true,
                "java_sufficient": true,
                "java_command": java.command,
                "java_major": java.major,
                "required_java_major": requirement.required_java_major,
                "java_output": java.version_output,
                "version_checked": version_checked,
                "version_output": version_output,
                "server_protocol_version": bridge.server_protocol_version,
                "native_mcp_tool_count": bridge.tool_count,
                "native_mcp_diagnostics": bridge.diagnostics,
                "bridge_error": bridge.error,
                "setup": setup,
                "version_policy": version_policy
            })))
        }
    }
}

/// `route_specctra_dsn`: the engine the call names routes it. fastroute (the
/// default) runs the timed best-of-N search of `route_fastroute`; Freerouting
/// runs the single job or the `width_policy` passes below, unchanged.
pub(crate) async fn handle_route_specctra_dsn(
    args: &serde_json::Value,
    ctx: std::sync::Arc<ToolContext>,
) -> anyhow::Result<CallToolResult> {
    match super::route_fastroute::route_engine(args) {
        Err(refusal) => Ok(refusal),
        Ok(super::route_fastroute::Engine::Freerouting) => route_with_freerouting(args, &ctx).await,
        // Row ZR, ZR0b (P12): this tool delivers a session, so its winner is pruned.
        Ok(super::route_fastroute::Engine::Fastroute) => {
            super::route_fastroute::route_with_fastroute(
                args,
                ctx,
                super::route_fastroute::PourPrune::Winner,
            )
            .await
        }
    }
}

/// The Freerouting engine of `route_specctra_dsn`.
async fn route_with_freerouting(
    args: &serde_json::Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let dsn = get_path(args, "dsn_path")?;
    let ses_output = get_path(args, "ses_output_path")?;
    let asked = match width_arguments(args) {
        Ok(found) => found,
        Err(refusal) => return Ok(refusal),
    };
    let (hold_requested, width_fallback) = (asked.hold, asked.fallback);
    let jar = match resolve_freerouting_jar(args) {
        FreeroutingJar::Found(jar) => jar,
        FreeroutingJar::SuppliedPathIsNotAFile(path) => {
            return Ok(CallToolResult::error_kind(
                ToolErrorKind::FileNotFound {
                    path: path.display().to_string(),
                },
                format!("jar_path is not a file: {}", path.display()),
            ));
        }
        FreeroutingJar::NotFound(roots) => {
            return Ok(CallToolResult::error(format!(
                "Freerouting JAR not found in {} searched locations; install Freerouting or \
                 pass jar_path. check_freerouting lists every location searched.",
                roots.len()
            )));
        }
    };
    // Checked before anything reads it, or Java is looked for: a directory,
    // FIFO, device or huge file is refused at once.
    if let Err(error) = crate::freerouting_mcp::check_dsn_input(&dsn) {
        let reason = format!("{error:#}");
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::InvalidArgument {
                field: "dsn_path".to_string(),
                reason: reason.clone(),
            },
            format!("Argument 'dsn_path' is invalid: {reason}"),
        ));
    }
    // The JAR names the Java it needs; refuse before starting anything that
    // would fail with a class-loading error that names no fix.
    let requirement = crate::freerouting_setup::jar_requirement(&jar);
    let java_path = args["java_path"].as_str().map(PathBuf::from);
    let found = crate::freerouting_setup::discover_java(java_path.as_deref()).await;
    let Some(java) = crate::freerouting_setup::pick(&found, requirement.required_java_major) else {
        let setup = crate::freerouting_setup::guidance(Some(&jar), Some(&requirement), &found);
        let reason = format!(
            "Freerouting needs Java {} or newer ({}), and none was found that meets it. Nothing was routed. Run check_freerouting for the exact steps.",
            requirement.required_java_major, requirement.basis
        );
        let mut result = CallToolResult::error_kind(
            ToolErrorKind::HandlerError {
                reason: reason.clone(),
            },
            reason,
        );
        if let Some(crate::mcp::protocol::ToolContent::Text { text }) = result.content.first_mut() {
            text.push_str(&format!("\nsetup: {setup}"));
        }
        return Ok(result);
    };
    // The export's reverse manifest beside the DSN carries the project's
    // copper-to-edge clearance; Freerouting takes it as a job setting.
    let hints = crate::specctra::manifest_routing_hints(&dsn);
    let edge_clearance = hints.copper_edge_clearance_um;
    let allow_neckdown = allow_neckdown(args);
    let settings = route_settings(args, &hints, java.command.clone())?;
    // What the router is given as planes, from the DSN itself: the router
    // counts a pad inside a plane as connected, so a run with one is never
    // reported complete.
    let planes =
        crate::freerouting_mcp::read_dsn(&dsn).and_then(|text| crate::specctra::dsn_planes(&text));
    // Row S9: `width_policy: "hold"` runs the pass list, on a board whose
    // manifest names a net with a recorded width. Anything else is the
    // single job, with the settings above, exactly as before.
    let hold = hold_requested
        && crate::specctra::manifest_beside(&dsn)
            .as_ref()
            .is_some_and(crate::intent_width_report::has_judged_net);
    let mut held: Option<HoldOutcome> = None;
    let evidence = if hold {
        let outcome = route_held(&jar, &dsn, &ses_output, &settings, &hints, width_fallback)
            .await
            .map_err(|error| anyhow::anyhow!("Freerouting MCP routing failed: {error:#}"))?;
        let evidence = crate::freerouting_mcp::RouteEvidence {
            elapsed_seconds: outcome.elapsed_seconds,
            ..outcome.evidence.clone()
        };
        held = Some(outcome);
        evidence
    } else {
        crate::freerouting_mcp::route_local(&jar, &dsn, &ses_output, &settings)
            .await
            .map_err(|error| anyhow::anyhow!("Freerouting MCP routing failed: {error:#}"))?
    };
    let (width_report, width_error) = width_report(&dsn, &ses_output);
    let width = WidthOutcome {
        exceptions: width_report.exceptions_total,
        stopped_in: held.as_ref().and_then(|held| held.stopped_in),
        unread: width_error.is_some(),
        unmeasured: width_report.unmeasured_total,
        policy: asked.policy_given,
        fallback: asked.fallback_given,
    };
    let (status, mut completion) =
        completion_report(&dsn, &ses_output, &planes, evidence.router_unrouted, &width);
    carry_freerouting_paths(&mut completion, args);
    let mut result = json!({
        "success": true,
        // Additive (S6 second half): what completion can be claimed, and the
        // calls still needed. `success` still means the job ran and the SES
        // was written.
        "status": status,
        "completion": completion,
        "method": "local_freerouting_native_mcp",
        "engine": {
            "name": "Freerouting",
            "jar_path": jar,
            "execution": "local"
        },
        "native_mcp": {
            "used": true,
            "server_protocol_version": evidence.server_protocol_version
        },
        "bridge": {
            "mode": "dsn_ses_file_round_trip",
            "cloud_used": false
        },
        "rules_applied": {
            "automatic_neckdown": allow_neckdown,
            "fanout_enabled": settings.fanout_enabled,
            "copper_to_edge_clearance_um": edge_clearance,
            "copper_to_edge_clearance_source": if edge_clearance.is_some() { "export_manifest" } else { "none: no manifest beside the DSN, or it records no clearance" },
            "retry_width_um": hints.retry_width_um,
            // Why no retry width was sent, when the nets' floors differ.
            "retry_off_reason": hints.retry_off_reason,
        },
        // Read back from Freerouting (`get_effective_settings`): the run is
        // refused before routing when any sent setting is not in effect.
        "effective_settings": evidence.effective_settings,
        "optimizer": evidence.optimizer,
        "artifacts": {
            "dsn_path": dsn,
            "ses_output_path": ses_output,
            "diagnostics_path": evidence.diagnostics_path
        },
        "session_id": evidence.session_id,
        "job_id": evidence.job_id,
        "final_state": evidence.final_state,
        "poll_count": evidence.poll_count,
        "elapsed_seconds": evidence.elapsed_seconds,
        "ses_bytes": evidence.ses_bytes,
        // Row S9, always on: what the session did to the nets that have a
        // recorded width (the manifest's `intent_widths`). Empty when the
        // manifest names none.
        "intent_widths": width_report.nets,
        "width_exceptions": width_report.exceptions,
        "width_exceptions_total": width_report.exceptions_total,
    });
    if let Some(error) = width_error {
        result["width_report_error"] = json!(error);
    }
    // Row LT (L14): Freerouting does not tune lengths. A board whose design intent
    // declares a length group or a pair skew still routes; the note keeps the gap
    // visible. The board is the manifest's, as for the completion steps.
    let intent_board = crate::specctra::manifest_beside(&dsn)
        .and_then(|manifest| manifest["board_path"].as_str().map(PathBuf::from));
    if let Some(note) = intent_board
        .as_deref()
        .and_then(super::route_lengths::freerouting_note)
    {
        result["length_matching_note"] = json!(note);
    }
    // Only a session that holds a wire the report could not measure carries
    // these; the status is then never "complete".
    if width_report.unmeasured_total > 0 {
        result["width_unmeasured"] = json!(width_report.unmeasured);
        result["width_unmeasured_total"] = json!(width_report.unmeasured_total);
    }
    // `width_policy` is in the result when `hold` was asked for, and also when
    // `width_fallback` was given without it, to say that it was not used.
    // `passes`, `result_pass` and the passes folder come only with `hold`.
    if !hold_requested && asked.fallback_given.is_some() {
        result["width_policy"] = json!({
            "requested": "retry", "applied": "retry", "fallback": "unused",
            "reason": "width_fallback applies only with width_policy \"hold\"; it was not used",
        });
    }
    if hold_requested {
        let fallback = match width_fallback {
            WidthFallback::Floor => "floor",
            WidthFallback::None => "none",
        };
        result["width_policy"] = match &held {
            Some(_) => json!({ "requested": "hold", "applied": "hold", "fallback": fallback }),
            None => json!({
                "requested": "hold", "applied": "retry", "fallback": fallback,
                "reason": "the manifest beside the DSN names no net with a recorded width, so there is no width to hold: the single run was made, with its usual settings",
            }),
        };
    }
    if let Some(held) = held {
        result["passes"] = json!(held.passes);
        result["result_pass"] = json!(held.result_pass);
        result["artifacts"]["passes_folder"] = json!(passes_folder(&ses_output));
    }
    Ok(CallToolResult::json(&result))
}

// ─── Row S9, step B: the pass engine behind `width_policy: "hold"` ────────

/// What a width-first call does with connections the Full pass leaves open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WidthFallback {
    /// Retry them at the one floor width, in a pass of their own.
    Floor,
    /// Leave them open.
    None,
}

/// What `hold_widths` needs: the exported DSN, an empty folder for the
/// passes' own files, the settings `route_settings` built, and the one floor
/// width a fallback may retry at (the manifest's hint).
pub(crate) struct HoldPlan {
    pub dsn: PathBuf,
    pub folder: PathBuf,
    pub base: crate::freerouting_mcp::RouteSettings,
    pub floor_retry_um: Option<f64>,
    /// Why there is no floor width, when there is none.
    pub no_floor_reason: Option<String>,
    pub fallback: WidthFallback,
    /// No later pass starts with less of the call's time left than this.
    pub least_pass_time: std::time::Duration,
}

/// The result of a width-first call: every pass that ran or was skipped, and
/// the last full-board session that passed its checks.
pub(crate) struct HoldOutcome {
    pub passes: Vec<serde_json::Value>,
    pub result_pass: &'static str,
    pub ses: PathBuf,
    pub evidence: crate::freerouting_mcp::RouteEvidence,
    /// The pass at which the time ran out: before it started or while it
    /// ran (its row in `passes` says which).
    pub stopped_in: Option<&'static str>,
    pub elapsed_seconds: f64,
}

/// The shortest time worth starting a pass with: the least `route_local`
/// accepts as its overall timeout.
const LEAST_PASS_TIME: std::time::Duration = std::time::Duration::from_secs(10);

/// The one rule of every pass list: Freerouting's retry is never on while
/// its optimizer is on. The optimizer lays narrowed retries of its own on
/// every net it touches, which is what holding the width must prevent.
fn check_pass_rule(settings: &crate::freerouting_mcp::RouteSettings) -> anyhow::Result<()> {
    let retry_on = settings.retry_width_um.is_none_or(|um| um > 0.0);
    let optimizer_on = settings.optimizer_enabled != Some(false);
    if retry_on && optimizer_on {
        anyhow::bail!("a width-first pass may not run the optimizer with the retry on");
    }
    Ok(())
}

/// `width_policy` and `width_fallback` of a routing call.
struct WidthArguments {
    /// Whether `hold` was asked for.
    hold: bool,
    /// What to do with connections the Full pass leaves open.
    fallback: WidthFallback,
    /// The two arguments as the caller gave them, when given.
    policy_given: Option<&'static str>,
    fallback_given: Option<&'static str>,
}

/// Read them. A value outside the two each takes is refused.
fn width_arguments(args: &serde_json::Value) -> Result<WidthArguments, CallToolResult> {
    let refuse = |field: &str, allowed: &str| {
        let reason = format!("must be {allowed}");
        CallToolResult::error_kind(
            ToolErrorKind::InvalidArgument {
                field: field.to_string(),
                reason: reason.clone(),
            },
            format!("Argument '{field}' is invalid: {reason}"),
        )
    };
    let policy_given = match &args["width_policy"] {
        serde_json::Value::Null => None,
        value => match value.as_str() {
            Some("retry") => Some("retry"),
            Some("hold") => Some("hold"),
            _ => return Err(refuse("width_policy", "\"retry\" or \"hold\"")),
        },
    };
    let fallback_given = match &args["width_fallback"] {
        serde_json::Value::Null => None,
        value => match value.as_str() {
            Some("floor") => Some("floor"),
            Some("none") => Some("none"),
            _ => return Err(refuse("width_fallback", "\"floor\" or \"none\"")),
        },
    };
    Ok(WidthArguments {
        hold: policy_given == Some("hold"),
        fallback: if fallback_given == Some("none") {
            WidthFallback::None
        } else {
            WidthFallback::Floor
        },
        policy_given,
        fallback_given,
    })
}

/// The JAR and Java the caller named are repeated into the `route_specctra_dsn`
/// steps of a Freerouting result's completion, so that following a step routes
/// with the same JAR.
fn carry_freerouting_paths(completion: &mut serde_json::Value, args: &serde_json::Value) {
    for pointer in ["/next_steps", "/if_still_open/fallback"] {
        let Some(steps) = completion
            .pointer_mut(pointer)
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        for step in steps
            .iter_mut()
            .filter(|step| step["tool"] == "route_specctra_dsn")
        {
            for key in ["jar_path", "java_path"] {
                if let Some(value) = args[key].as_str() {
                    step["arguments"][key] = json!(value);
                }
            }
        }
    }
}

/// What `width_fallback` asks for, for the fastroute engine, which always holds
/// (`width_policy` itself is settled by `route_fastroute::route_engine`): the
/// floor unless the caller said `"none"`. A value outside the two is refused.
pub(crate) fn width_fallback_argument(
    args: &serde_json::Value,
) -> Result<WidthFallback, CallToolResult> {
    width_arguments(args).map(|asked| asked.fallback)
}

/// Where a width-first call keeps its passes' own files: beside the output,
/// `<output stem>.passes`.
fn passes_folder(ses_output: &Path) -> PathBuf {
    ses_output.with_extension("passes")
}

/// Run the pass list with real Freerouting jobs and write the result's
/// session to `ses_output`. Refused before anything is routed: an output
/// that is not a new `.ses` file, and a passes folder that already exists.
/// A call that fails after that leaves no passes folder behind: what it
/// wrote is moved aside ([`set_aside_failed_passes`]), so the same call can
/// be made again.
async fn route_held(
    jar: &Path,
    dsn: &Path,
    ses_output: &Path,
    settings: &crate::freerouting_mcp::RouteSettings,
    hints: &crate::specctra::RoutingHints,
    fallback: WidthFallback,
) -> anyhow::Result<HoldOutcome> {
    if !ses_output
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ses"))
    {
        anyhow::bail!("SES output must have the .ses extension");
    }
    if ses_output.exists() {
        anyhow::bail!("SES output already exists: {}", ses_output.display());
    }
    let folder = passes_folder(ses_output);
    if let Some(parent) = folder
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            anyhow::anyhow!("create SES output directory {}: {error}", parent.display())
        })?;
    }
    // `create_dir` refuses a folder that is there, in one step.
    std::fs::create_dir(&folder).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow::anyhow!(
                "the passes folder already exists: {} (a width-first call keeps each pass's files there and never replaces one)",
                folder.display()
            )
        } else {
            anyhow::anyhow!("create the passes folder {}: {error}", folder.display())
        }
    })?;
    let plan = HoldPlan {
        dsn: dsn.to_path_buf(),
        folder,
        base: settings.clone(),
        floor_retry_um: hints.retry_width_um.map(|um| um as f64),
        no_floor_reason: hints.retry_off_reason.map(str::to_string),
        fallback,
        least_pass_time: LEAST_PASS_TIME,
    };
    let run = async {
        let outcome = hold_widths(
            |dsn, ses, settings| {
                let jar = jar.to_path_buf();
                async move {
                    crate::freerouting_mcp::route_local(&jar, &dsn, &ses, &settings).await
                }
            },
            &plan,
        )
        .await?;
        let session = std::fs::read_to_string(&outcome.ses).map_err(|error| {
            anyhow::anyhow!("read the {} pass's session: {error}", outcome.result_pass)
        })?;
        konnect_sexp::write_new_atomic(ses_output, &session).map_err(|error| {
            anyhow::anyhow!("create SES output {}: {error}", ses_output.display())
        })?;
        anyhow::Ok(outcome)
    };
    match run.await {
        Ok(outcome) => Ok(outcome),
        // Whatever failed, the folder this call made must not block the
        // same call made again.
        Err(error) => {
            let note = set_aside_failed_passes(&plan.folder);
            Err(anyhow::anyhow!("{error:#} ({note})"))
        }
    }
}

/// The most failed calls' folders kept beside one output.
const MAX_FAILED_PASSES_KEPT: u32 = 99;

/// Take a failed width-first call's passes folder out of the way, so the
/// same call can be made again, and say what was done. An empty folder is
/// removed. One that holds files (a job's log, an earlier pass's session) is
/// renamed to `<folder>.failed-N`, the first N from 1 that is free, beside
/// the output where the call made it: a name that is taken is never
/// replaced. In two cases the folder is removed with what it holds: every
/// one of the names is taken, or the rename to a free name fails. Nothing
/// outside the folder this call made is touched.
fn set_aside_failed_passes(folder: &Path) -> String {
    // `remove_dir` removes only an empty folder.
    if std::fs::remove_dir(folder).is_ok() || !folder.exists() {
        return "the failed call left no file of its own".to_string();
    }
    for n in 1..=MAX_FAILED_PASSES_KEPT {
        let mut name = folder.as_os_str().to_os_string();
        name.push(format!(".failed-{n}"));
        let aside = PathBuf::from(name);
        // Anything by that name, a broken link too, keeps the name.
        if aside.symlink_metadata().is_ok() {
            continue;
        }
        if std::fs::rename(folder, &aside).is_ok() {
            return format!("the failed call's files are kept in {}", aside.display());
        }
        break;
    }
    match std::fs::remove_dir_all(folder) {
        Ok(()) => "the failed call's passes folder was removed".to_string(),
        Err(error) => format!(
            "the failed call's passes folder {} could not be removed ({error}); remove it before calling again",
            folder.display()
        ),
    }
}

/// One row of `passes`: what a pass was asked to do and what came of it.
fn pass_row(
    name: &'static str,
    outcome: &'static str,
    reason: Option<String>,
) -> serde_json::Value {
    let mut row = json!({ "name": name, "outcome": outcome });
    if let Some(reason) = reason {
        let reason: String = reason.chars().take(4 * MAX_REASON_DETAIL).collect();
        row["reason"] = json!(reason);
    }
    row
}

/// The same row with what the job sent and what its log and files show.
fn job_row(
    mut row: serde_json::Value,
    dsn: &Path,
    ses: &Path,
    settings: &crate::freerouting_mcp::RouteSettings,
    seeded: Option<&crate::specctra_seed::Seeded>,
    evidence: Option<&crate::freerouting_mcp::RouteEvidence>,
) -> serde_json::Value {
    row["retry_width_um"] = json!(settings.retry_width_um);
    row["optimizer_enabled"] = json!(settings.optimizer_enabled);
    if let Some(seeded) = seeded {
        row["seeded"] = json!({
            "wires": seeded.wires, "vias": seeded.vias, "nets": seeded.nets.len(),
        });
    }
    // Only files that are there: a pass that never started wrote none.
    let mut artifacts = json!({});
    for (key, path) in [("dsn_path", dsn), ("ses_path", ses)] {
        if path.is_file() {
            artifacts[key] = json!(path);
        }
    }
    if let Some(evidence) = evidence {
        row["unrouted_at_start"] = json!(evidence.unrouted_at_start);
        row["router_unrouted"] = json!(evidence.router_unrouted);
        row["effective_settings"] = json!(evidence.effective_settings);
        row["optimizer"] = json!(evidence.optimizer);
        row["elapsed_seconds"] = json!(evidence.elapsed_seconds);
        row["job_id"] = json!(evidence.job_id);
        artifacts["diagnostics_path"] = json!(evidence.diagnostics_path);
    }
    row["artifacts"] = artifacts;
    row
}

/// What became of a seeded pass.
enum Seeded {
    /// It passed its checks: its session is the result so far.
    Kept(Box<crate::freerouting_mcp::RouteEvidence>),
    /// It ran, failed, was discarded, or ran out of time: the session of the
    /// pass before it stands. `stopped` says the time ran out.
    Not { stopped: bool },
}

/// Width-first routing (row S9): hold every net at its width in a Full pass
/// with the retry off; only if that leaves connections open, a Fallback pass
/// (retry at the floor width, optimizer off) and then a Polish pass (retry
/// off, optimizer on). `route` runs one Freerouting job.
///
/// Each later pass starts from the session of the pass before it, written
/// into a scratch copy of the DSN as ordinary wiring, and its own session is
/// kept only if it started with exactly the unrouted count the pass before
/// ended with, holds every seeded net and no typed item, and ended as it
/// must: the Fallback with fewer open connections than it started with, the
/// Polish with none. Anything else leaves the earlier session as the result:
/// a later pass can only improve on a session that was already whole.
pub(crate) async fn hold_widths<R, F>(route: R, plan: &HoldPlan) -> anyhow::Result<HoldOutcome>
where
    R: Fn(PathBuf, PathBuf, crate::freerouting_mcp::RouteSettings) -> F,
    F: std::future::Future<Output = anyhow::Result<crate::freerouting_mcp::RouteEvidence>>,
{
    let started = std::time::Instant::now();
    let mut passes = Vec::new();

    // Full: every net, the retry off, the optimizer as the caller set it.
    let mut settings = plan.base.clone();
    settings.retry_width_um = Some(0.0);
    check_pass_rule(&settings)?;
    let mut ses = plan.folder.join("full.ses");
    let mut evidence = route(plan.dsn.clone(), ses.clone(), settings.clone()).await?;
    passes.push(job_row(
        pass_row("full", "kept", None),
        &plan.dsn,
        &ses,
        &settings,
        None,
        Some(&evidence),
    ));
    let mut result_pass = "full";
    let mut stopped_in = None;

    // One seeded pass: `name`, started from the session at `from`.
    let seeded_pass = |name: &'static str,
                       from: PathBuf,
                       retry_um: f64,
                       optimizer: bool,
                       expect_start: u32,
                       must_end_at_most: u32| {
        let route = &route;
        async move {
            let mut settings = plan.base.clone();
            settings.retry_width_um = Some(retry_um);
            settings.optimizer_enabled = Some(optimizer);
            let dsn = plan.folder.join(format!("{name}.dsn"));
            let ses = plan.folder.join(format!("{name}.ses"));
            // A later pass gets only the time the call has left.
            let left = plan
                .base
                .overall_timeout
                .checked_sub(started.elapsed())
                .filter(|left| *left >= plan.least_pass_time);
            if let Some(left) = left {
                settings.overall_timeout = left;
            }
            let not = |outcome, reason: String, stopped| {
                (
                    job_row(
                        pass_row(name, outcome, Some(reason)),
                        &dsn,
                        &ses,
                        &settings,
                        None,
                        None,
                    ),
                    Seeded::Not { stopped },
                    ses.clone(),
                )
            };
            if let Err(error) = check_pass_rule(&settings) {
                return not("failed", format!("{error:#}"), false);
            }
            if left.is_none() {
                return not(
                    "stopped",
                    "the call's time was spent before this pass could start".to_string(),
                    true,
                );
            }
            let seed = crate::freerouting_mcp::read_dsn(&plan.dsn).and_then(|dsn_text| {
                let session = std::fs::read_to_string(&from).map_err(|error| {
                    anyhow::anyhow!("the session to start from could not be read: {error}")
                })?;
                crate::specctra_seed::seed_dsn(&dsn_text, &session)
            });
            let seed = match seed {
                Ok(seed) => seed,
                Err(error) => {
                    return not(
                        "failed",
                        format!("the pass could not be seeded: {error:#}"),
                        false,
                    )
                }
            };
            if let Err(error) = konnect_sexp::write_new_atomic(&dsn, &seed.dsn) {
                return not(
                    "failed",
                    format!("the seeded DSN could not be written: {error}"),
                    false,
                );
            }
            let evidence = match route(dsn.clone(), ses.clone(), settings.clone()).await {
                Ok(evidence) => evidence,
                Err(error) => {
                    let stopped = error
                        .downcast_ref::<crate::freerouting_mcp::OverallTimeout>()
                        .is_some();
                    let (outcome, reason) = if stopped {
                        (
                            "stopped",
                            "the call's time ran out during this pass".to_string(),
                        )
                    } else {
                        ("failed", format!("{error:#}"))
                    };
                    let mut row = not(outcome, reason, stopped);
                    row.0["seeded"] =
                        json!({ "wires": seed.wires, "vias": seed.vias, "nets": seed.nets.len() });
                    return row;
                }
            };
            // The checks a seeded pass's session must pass to be the result.
            let discard = (|| {
                if evidence.unrouted_at_start != Some(expect_start) {
                    return Some(format!(
                        "seed_not_applied: the pass before ended with {expect_start} unrouted, and this pass started with {}",
                        evidence
                            .unrouted_at_start
                            .map_or("a count its log does not give".to_string(), |n| n.to_string())
                    ));
                }
                let wiring = match std::fs::read_to_string(&ses)
                    .map_err(anyhow::Error::from)
                    .and_then(|text| crate::specctra_seed::session_wiring(&text))
                {
                    Ok(wiring) => wiring,
                    Err(error) => {
                        return Some(format!("the pass's session could not be read: {error:#}"))
                    }
                };
                if wiring.typed > 0 {
                    return Some(format!(
                        "the pass's session holds {} typed item(s), and the session of a seeded pass must hold none",
                        wiring.typed
                    ));
                }
                let missing = seed.nets.difference(&wiring.nets).count();
                if missing > 0 {
                    return Some(format!(
                        "seed_not_applied: {missing} seeded net(s) have no copper in the pass's session"
                    ));
                }
                match evidence.router_unrouted {
                    Some(left) if left <= must_end_at_most => None,
                    Some(left) if left == expect_start => Some(format!(
                        "the pass closed no connection: it ended with {left} unrouted, as many as it started with, so the session of the pass before it stays"
                    )),
                    Some(left) => Some(format!(
                        "the pass ended with {left} unrouted, more than the {expect_start} it started with"
                    )),
                    None => Some("the pass's log gave no unrouted count".to_string()),
                }
            })();
            match discard {
                Some(reason) => (
                    job_row(
                        pass_row(name, "discarded", Some(reason)),
                        &dsn,
                        &ses,
                        &settings,
                        Some(&seed),
                        Some(&evidence),
                    ),
                    Seeded::Not { stopped: false },
                    ses,
                ),
                None => (
                    job_row(
                        pass_row(name, "kept", None),
                        &dsn,
                        &ses,
                        &settings,
                        Some(&seed),
                        Some(&evidence),
                    ),
                    Seeded::Kept(Box::new(evidence)),
                    ses,
                ),
            }
        }
    };

    // Fallback: only when the Full pass left connections open.
    let skipped = |name, reason: String| pass_row(name, "skipped", Some(reason));
    let fallback_from = match evidence.router_unrouted {
        Some(0) => None,
        None => {
            passes.push(skipped(
                "fallback",
                "the Full pass's log gave no unrouted count, so it is not known whether anything is open".to_string(),
            ));
            None
        }
        Some(left) => {
            match (plan.fallback, plan.floor_retry_um) {
                (WidthFallback::None, _) => {
                    passes.push(skipped(
                    "fallback",
                    format!("width_fallback is \"none\": the {left} open connection(s) are left open"),
                ));
                    None
                }
                (WidthFallback::Floor, None) => {
                    passes.push(skipped(
                    "fallback",
                    format!(
                        "no retry width is safe for every net ({}), so the {left} open connection(s) are left open",
                        plan.no_floor_reason.as_deref().unwrap_or("the manifest gives no floor width")
                    ),
                ));
                    None
                }
                (WidthFallback::Floor, Some(floor)) => Some((left, floor)),
            }
        }
    };
    if let Some((left, floor)) = fallback_from {
        // Kept only if it ends with fewer open connections than it started
        // with: a Fallback has the retry on over movable wiring, so one that
        // closes nothing could only have narrowed track for nothing.
        let (row, outcome, fallback_ses) = seeded_pass(
            "fallback",
            ses.clone(),
            floor,
            false,
            left,
            left.saturating_sub(1),
        )
        .await;
        passes.push(row);
        match outcome {
            Seeded::Not { stopped } => {
                if stopped {
                    stopped_in = Some("fallback");
                }
            }
            Seeded::Kept(fallback) => {
                (ses, evidence, result_pass) = (fallback_ses, *fallback, "fallback");
                // Polish: only a complete board, and only with the optimizer.
                let still_open = evidence.router_unrouted.unwrap_or(0);
                if still_open > 0 {
                    passes.push(skipped(
                        "polish",
                        format!("the fallback pass left {still_open} connection(s) open, and the optimizer runs only on a complete board"),
                    ));
                } else if plan.base.optimizer_enabled == Some(false) {
                    passes.push(skipped("polish", "optimizer_enabled is false".to_string()));
                } else {
                    let (row, outcome, polish_ses) =
                        seeded_pass("polish", ses.clone(), 0.0, true, 0, 0).await;
                    passes.push(row);
                    match outcome {
                        Seeded::Not { stopped } => {
                            if stopped {
                                stopped_in = Some("polish");
                            }
                        }
                        Seeded::Kept(polish) => {
                            (ses, evidence, result_pass) = (polish_ses, *polish, "polish");
                        }
                    }
                }
            }
        }
    }
    Ok(HoldOutcome {
        passes,
        result_pass,
        ses,
        evidence,
        stopped_in,
        elapsed_seconds: started.elapsed().as_secs_f64(),
    })
}

/// The width report of the session at `ses`, against the manifest beside
/// `dsn`. With no manifest, or one that names no net with a recorded width,
/// the session is not read and the report is empty. A session that cannot be
/// read for widths gives an empty report and the reason: the routing stands,
/// and the status then never says `complete`.
pub(crate) fn width_report(
    dsn: &Path,
    ses: &Path,
) -> (crate::intent_width_report::WidthReport, Option<String>) {
    use crate::intent_width_report::{has_judged_net, intent_width_report, WidthReport};
    let Some(manifest) = crate::specctra::manifest_beside(dsn).filter(has_judged_net) else {
        return (WidthReport::default(), None);
    };
    let read = std::fs::read_to_string(ses)
        .map_err(|error| anyhow::anyhow!("the session could not be read: {error}"))
        .and_then(|text| intent_width_report(&manifest, &text));
    match read {
        Ok(report) => (report, None),
        Err(error) => {
            let detail: String = format!("{error:#}")
                .chars()
                .take(MAX_REASON_DETAIL)
                .collect();
            (WidthReport::default(), Some(detail))
        }
    }
}

#[cfg(test)]
mod hold_widths_tests {
    use super::{check_pass_rule, hold_widths, HoldOutcome, HoldPlan, WidthFallback};
    use crate::freerouting_mcp::{OverallTimeout, RouteEvidence, RouteSettings};
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    const DSN: &str = "(pcb board\n  (parser\n    (string_quote \")\n  )\n  (resolution um 10)\n  (unit um)\n  (structure\n    (layer F.Cu\n      (type signal)\n    )\n  )\n  (library\n    (padstack via0\n      (shape (circle F.Cu 600))\n    )\n  )\n  (network\n    (net VBUS\n      (pins U1-1 J1-1)\n    )\n    (net SIG\n      (pins U1-2 J1-2)\n    )\n  )\n  (wiring)\n)\n";

    /// A session with one VBUS wire at `vbus_width` (session units, a tenth
    /// of a micrometre) and, when `sig`, one SIG wire.
    fn session(vbus_width: u32, sig: bool) -> String {
        let sig = if sig {
            "(net SIG (wire (path F.Cu 2000 0 50000 90000 50000)))"
        } else {
            ""
        };
        format!("(session board (routes (resolution um 10) (network_out (net VBUS (wire (path F.Cu {vbus_width} 0 0 90000 0)) (via via0 90000 0)) {sig})))")
    }

    enum Reply {
        /// The job writes `session` and its log says it started with `start`
        /// unrouted and ended with `end`.
        Routed {
            session: String,
            start: Option<u32>,
            end: Option<u32>,
        },
        /// The same, after sleeping this many milliseconds.
        Slow(u64, String, Option<u32>, Option<u32>),
        Timeout,
        Fail(&'static str),
    }

    fn routed(session: String, start: u32, end: u32) -> Reply {
        Reply::Routed {
            session,
            start: Some(start),
            end: Some(end),
        }
    }

    struct Call {
        dsn_name: String,
        dsn_text: String,
        ses_name: String,
        settings: RouteSettings,
    }

    #[derive(Default)]
    struct Script {
        replies: Mutex<VecDeque<Reply>>,
        calls: Mutex<Vec<Call>>,
    }

    fn evidence(start: Option<u32>, end: Option<u32>, job: usize) -> RouteEvidence {
        RouteEvidence {
            session_id: "s".into(),
            job_id: format!("job{job}"),
            final_state: "COMPLETED".into(),
            poll_count: 1,
            elapsed_seconds: 0.5,
            ses_bytes: 10,
            server_protocol_version: "2024-11-05".into(),
            diagnostics_path: None,
            effective_settings: Default::default(),
            optimizer: None,
            router_unrouted: end,
            unrouted_at_start: start,
        }
    }

    fn base(overall_ms: u64) -> RouteSettings {
        RouteSettings {
            max_passes: Some(20),
            optimizer_enabled: Some(true),
            job_timeout_seconds: None,
            poll_interval: std::time::Duration::from_secs(3),
            overall_timeout: std::time::Duration::from_millis(overall_ms),
            java: None,
            automatic_neckdown: Some(false),
            copper_to_edge_clearance_um: None,
            // What `route_settings` built from the manifest: today's retry.
            retry_width_um: Some(200.0),
            fanout_enabled: Some(false),
            via_costs: Some(10),
            default_undesired_direction_trace_cost: Some(1.0),
            optimizer_improvement_threshold: Some(0.25),
            optimizer_max_consecutive_failures_pass1: Some(1_000_000),
            optimizer_max_consecutive_failures: Some(1_000_000),
        }
    }

    struct Run {
        outcome: anyhow::Result<HoldOutcome>,
        calls: Vec<Call>,
        _dir: tempfile::TempDir,
    }

    impl Run {
        fn ok(&self) -> &HoldOutcome {
            self.outcome
                .as_ref()
                .unwrap_or_else(|error| panic!("{error:#}"))
        }
        fn pass(&self, name: &str) -> &serde_json::Value {
            self.ok()
                .passes
                .iter()
                .find(|pass| pass["name"] == name)
                .unwrap_or_else(|| panic!("no pass named {name}: {:?}", self.ok().passes))
        }
        fn names(&self) -> Vec<(String, String)> {
            self.ok()
                .passes
                .iter()
                .map(|pass| {
                    (
                        pass["name"].as_str().unwrap().to_string(),
                        pass["outcome"].as_str().unwrap().to_string(),
                    )
                })
                .collect()
        }
        fn result_text(&self) -> String {
            std::fs::read_to_string(&self.ok().ses).unwrap()
        }
    }

    fn pairs(names: &[(&str, &str)]) -> Vec<(String, String)> {
        names
            .iter()
            .map(|(name, outcome)| (name.to_string(), outcome.to_string()))
            .collect()
    }

    async fn go_with(plan_edit: impl FnOnce(&mut HoldPlan), replies: Vec<Reply>) -> Run {
        let dir = tempfile::tempdir().unwrap();
        let dsn = dir.path().join("board.dsn");
        std::fs::write(&dsn, DSN).unwrap();
        let folder = dir.path().join("board.passes");
        std::fs::create_dir(&folder).unwrap();
        let mut plan = HoldPlan {
            dsn,
            folder,
            base: base(60_000),
            floor_retry_um: Some(200.0),
            no_floor_reason: None,
            fallback: WidthFallback::Floor,
            least_pass_time: std::time::Duration::from_millis(1),
        };
        plan_edit(&mut plan);
        let script = Arc::new(Script {
            replies: Mutex::new(replies.into()),
            calls: Mutex::default(),
        });
        let route = {
            let script = script.clone();
            move |dsn: PathBuf, ses: PathBuf, settings: RouteSettings| {
                let script = script.clone();
                async move {
                    let job = {
                        let mut calls = script.calls.lock().unwrap();
                        calls.push(Call {
                            dsn_name: name_of(&dsn),
                            dsn_text: std::fs::read_to_string(&dsn).unwrap(),
                            ses_name: name_of(&ses),
                            settings,
                        });
                        calls.len()
                    };
                    let reply = script
                        .replies
                        .lock()
                        .unwrap()
                        .pop_front()
                        .expect("a job nobody scripted");
                    let (session, start, end) = match reply {
                        Reply::Routed {
                            session,
                            start,
                            end,
                        } => (session, start, end),
                        Reply::Slow(ms, session, start, end) => {
                            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                            (session, start, end)
                        }
                        Reply::Timeout => {
                            return Err(anyhow::Error::new(OverallTimeout { seconds: 1 })
                                .context("Freerouting stderr: something"))
                        }
                        Reply::Fail(reason) => return Err(anyhow::anyhow!(reason)),
                    };
                    std::fs::write(&ses, session).unwrap();
                    Ok(evidence(start, end, job))
                }
            }
        };
        let outcome = hold_widths(route, &plan).await;
        assert!(
            script.replies.lock().unwrap().is_empty(),
            "a scripted job did not run"
        );
        let calls = std::mem::take(&mut *script.calls.lock().unwrap());
        // The rule of every pass list, checked on every job of every test.
        for call in &calls {
            let retry_on = call.settings.retry_width_um.is_none_or(|um| um > 0.0);
            assert!(
                !(retry_on && call.settings.optimizer_enabled != Some(false)),
                "{}: the retry is on with the optimizer on",
                call.ses_name
            );
        }
        Run {
            outcome,
            calls,
            _dir: dir,
        }
    }

    fn name_of(path: &Path) -> String {
        path.file_name().unwrap().to_string_lossy().into_owned()
    }

    async fn go(replies: Vec<Reply>) -> Run {
        go_with(|_| {}, replies).await
    }

    #[test]
    fn the_retry_is_never_on_with_the_optimizer_on() {
        let mut settings = base(1000);
        // Today's single run: retry at the floor with the optimizer on.
        assert!(check_pass_rule(&settings).is_err());
        settings.retry_width_um = Some(0.0);
        assert!(check_pass_rule(&settings).is_ok(), "full and polish");
        settings.retry_width_um = Some(200.0);
        settings.optimizer_enabled = Some(false);
        assert!(check_pass_rule(&settings).is_ok(), "fallback");
        // Nothing sent means Freerouting's own defaults, which are both on.
        settings.retry_width_um = None;
        settings.optimizer_enabled = None;
        assert!(check_pass_rule(&settings).is_err());
        settings.optimizer_enabled = Some(false);
        assert!(check_pass_rule(&settings).is_ok());
        settings.retry_width_um = Some(0.0);
        settings.optimizer_enabled = None;
        assert!(check_pass_rule(&settings).is_ok());
    }

    #[tokio::test]
    async fn a_full_pass_that_leaves_nothing_is_the_whole_call() {
        let run = go(vec![routed(session(10000, true), 2, 0)]).await;
        assert_eq!(run.calls.len(), 1);
        let full = &run.calls[0];
        assert_eq!(full.dsn_name, "board.dsn", "the exported DSN itself");
        assert_eq!(full.dsn_text, DSN);
        assert_eq!(full.ses_name, "full.ses");
        assert_eq!(
            full.settings.retry_width_um,
            Some(0.0),
            "0 is sent, not nothing"
        );
        assert_eq!(full.settings.optimizer_enabled, Some(true));
        assert_eq!(full.settings.max_passes, Some(20));
        let outcome = run.ok();
        assert_eq!(outcome.result_pass, "full");
        assert_eq!(outcome.stopped_in, None);
        assert_eq!(outcome.evidence.router_unrouted, Some(0));
        assert_eq!(run.names(), pairs(&[("full", "kept")]));
        let pass = run.pass("full");
        assert_eq!(pass["retry_width_um"], 0.0);
        assert_eq!(pass["optimizer_enabled"], true);
        assert_eq!(pass["unrouted_at_start"], 2);
        assert_eq!(pass["router_unrouted"], 0);
        assert_eq!(pass["job_id"], "job1");
        assert!(pass["seeded"].is_null());
        assert!(pass["artifacts"]["ses_path"]
            .as_str()
            .unwrap()
            .ends_with("full.ses"));
        assert_eq!(run.result_text(), session(10000, true));
    }

    #[tokio::test]
    async fn open_connections_get_a_fallback_and_then_a_polish() {
        let run = go(vec![
            routed(session(10000, false), 2, 1),
            routed(session(10000, true), 1, 0),
            routed(session(10000, true).replace("50000", "40000"), 0, 0),
        ])
        .await;
        assert_eq!(
            run.names(),
            pairs(&[("full", "kept"), ("fallback", "kept"), ("polish", "kept")])
        );
        let [_, fallback, polish] = &run.calls[..] else {
            panic!("three jobs")
        };
        // The fallback starts from the Full pass's copper, as ordinary wiring.
        assert_eq!(fallback.dsn_name, "fallback.dsn");
        assert_eq!(fallback.ses_name, "fallback.ses");
        assert!(
            fallback.dsn_text.contains("(path F.Cu 1000 0 0 9000 0)"),
            "{}",
            fallback.dsn_text
        );
        assert!(fallback.dsn_text.contains("(via via0 9000 0"));
        assert!(!fallback.dsn_text.contains("(type fix)"));
        assert!(fallback
            .dsn_text
            .starts_with(DSN.strip_suffix("  (wiring)\n)\n").unwrap()));
        assert_eq!(fallback.settings.retry_width_um, Some(200.0));
        assert_eq!(fallback.settings.optimizer_enabled, Some(false));
        // The polish starts from the fallback's whole session.
        assert_eq!(polish.dsn_name, "polish.dsn");
        assert!(polish.dsn_text.contains("(path F.Cu 200 0 5000 9000 5000)"));
        assert!(polish.dsn_text.contains("(path F.Cu 1000 0 0 9000 0)"));
        assert_eq!(polish.settings.retry_width_um, Some(0.0));
        assert_eq!(polish.settings.optimizer_enabled, Some(true));
        let outcome = run.ok();
        assert_eq!(outcome.result_pass, "polish");
        assert_eq!(outcome.evidence.job_id, "job3");
        assert!(
            run.result_text().contains("40000"),
            "the polish pass's session"
        );
        let seeded = &run.pass("fallback")["seeded"];
        assert_eq!(
            (
                seeded["wires"].as_u64(),
                seeded["vias"].as_u64(),
                seeded["nets"].as_u64()
            ),
            (Some(1), Some(1), Some(1))
        );
        assert_eq!(run.pass("polish")["seeded"]["wires"], 2);
        assert_eq!(run.pass("fallback")["retry_width_um"], 200.0);
        assert_eq!(run.pass("fallback")["optimizer_enabled"], false);
        assert_eq!(run.pass("fallback")["unrouted_at_start"], 1);
    }

    /// The known misread: Freerouting reads an edited DSN as a board with one
    /// connection. A seeded pass must start with exactly the count the pass
    /// before it ended with.
    #[tokio::test]
    async fn a_seeded_pass_that_does_not_start_where_the_last_ended_is_discarded() {
        for start in [Some(1), Some(0), Some(4), None] {
            let run = go(vec![
                routed(session(10000, false), 5, 3),
                Reply::Routed {
                    session: session(10000, true),
                    start,
                    end: Some(0),
                },
            ])
            .await;
            assert_eq!(
                run.names(),
                pairs(&[("full", "kept"), ("fallback", "discarded")]),
                "{start:?}"
            );
            let reason = run.pass("fallback")["reason"].as_str().unwrap();
            assert!(reason.starts_with("seed_not_applied"), "{reason}");
            assert!(reason.contains("ended with 3"), "{reason}");
            let outcome = run.ok();
            assert_eq!(outcome.result_pass, "full");
            assert_eq!(outcome.evidence.router_unrouted, Some(3));
            assert_eq!(run.result_text(), session(10000, false));
            assert_eq!(outcome.stopped_in, None);
        }
        // The same for the polish pass, which must start with 0.
        let run = go(vec![
            routed(session(10000, false), 5, 3),
            routed(session(10000, true), 3, 0),
            routed(session(10000, true), 1, 0),
        ])
        .await;
        assert_eq!(
            run.names(),
            pairs(&[
                ("full", "kept"),
                ("fallback", "kept"),
                ("polish", "discarded")
            ])
        );
        assert!(run.pass("polish")["reason"]
            .as_str()
            .unwrap()
            .starts_with("seed_not_applied"));
        assert_eq!(run.ok().result_pass, "fallback");
        assert_eq!(run.ok().evidence.job_id, "job2");
    }

    #[tokio::test]
    async fn a_seeded_pass_whose_session_is_not_whole_or_holds_a_type_is_discarded() {
        // The fallback's session lost the seeded net.
        let lost = "(session board (routes (resolution um 10) (network_out (net SIG (wire (path F.Cu 2000 0 50000 90000 50000))))))";
        let run = go(vec![
            routed(session(10000, false), 2, 1),
            routed(lost.to_string(), 1, 0),
        ])
        .await;
        assert_eq!(
            run.names(),
            pairs(&[("full", "kept"), ("fallback", "discarded")])
        );
        let reason = run.pass("fallback")["reason"].as_str().unwrap();
        assert!(
            reason.starts_with("seed_not_applied") && reason.contains("1 seeded net"),
            "{reason}"
        );
        assert_eq!(run.ok().result_pass, "full");
        // A typed wire: the importer would refuse the session.
        let typed = session(10000, true).replace("90000 50000))", "90000 50000) (type fix))");
        let run = go(vec![
            routed(session(10000, false), 2, 1),
            routed(typed, 1, 0),
        ])
        .await;
        assert_eq!(
            run.names(),
            pairs(&[("full", "kept"), ("fallback", "discarded")])
        );
        assert!(run.pass("fallback")["reason"]
            .as_str()
            .unwrap()
            .contains("typed"));
        // A session that cannot be read.
        let run = go(vec![
            routed(session(10000, false), 2, 1),
            routed("(pcb x)".into(), 1, 0),
        ])
        .await;
        assert_eq!(
            run.names(),
            pairs(&[("full", "kept"), ("fallback", "discarded")])
        );
        assert_eq!(run.ok().result_pass, "full");
    }

    #[tokio::test]
    async fn a_fallback_that_still_leaves_connections_is_kept_without_a_polish() {
        let run = go(vec![
            routed(session(10000, false), 5, 3),
            routed(session(10000, true), 3, 1),
        ])
        .await;
        assert_eq!(
            run.names(),
            pairs(&[
                ("full", "kept"),
                ("fallback", "kept"),
                ("polish", "skipped")
            ])
        );
        assert!(run.pass("polish")["reason"]
            .as_str()
            .unwrap()
            .contains("1 connection"));
        assert_eq!(run.ok().result_pass, "fallback");
        assert_eq!(run.ok().evidence.router_unrouted, Some(1));
    }

    /// Review A, S2: a Fallback is kept only if it closed a connection. One
    /// that ends where it started may have re-laid track narrow for nothing.
    #[tokio::test]
    async fn a_fallback_that_closes_nothing_is_discarded_and_the_full_session_stays() {
        let run = go(vec![
            routed(session(10000, false), 5, 3),
            // The same three open, and VBUS re-laid narrow on the way.
            routed(session(2000, true), 3, 3),
        ])
        .await;
        assert_eq!(
            run.names(),
            pairs(&[("full", "kept"), ("fallback", "discarded")])
        );
        assert_eq!(run.ok().result_pass, "full");
        assert_eq!(run.ok().evidence.router_unrouted, Some(3));
        assert_eq!(run.ok().evidence.job_id, "job1");
        assert_eq!(run.result_text(), session(10000, false));
        let reason = run.pass("fallback")["reason"].as_str().unwrap();
        assert!(reason.contains("closed no connection"), "{reason}");
        assert!(reason.contains("3 unrouted"), "{reason}");
        // One fewer is enough.
        let run = go(vec![
            routed(session(10000, false), 5, 3),
            routed(session(10000, true), 3, 2),
        ])
        .await;
        assert_eq!(
            run.names(),
            pairs(&[
                ("full", "kept"),
                ("fallback", "kept"),
                ("polish", "skipped")
            ])
        );
        assert_eq!(run.ok().result_pass, "fallback");
        assert_eq!(run.ok().evidence.router_unrouted, Some(2));
    }

    /// Review A, N5: a row names only files that are there.
    #[tokio::test]
    async fn a_pass_row_names_only_files_that_exist() {
        // Never started: no file of its own.
        let run = go_with(
            |plan| plan.least_pass_time = std::time::Duration::from_secs(3600),
            vec![routed(session(10000, false), 5, 3)],
        )
        .await;
        let row = run.pass("fallback");
        assert!(row["artifacts"].as_object().unwrap().is_empty(), "{row}");
        // Seeded, and then the job failed: its DSN, and no session.
        let run = go(vec![
            routed(session(10000, false), 5, 3),
            Reply::Fail("boom"),
        ])
        .await;
        let row = run.pass("fallback");
        assert!(row["artifacts"]["dsn_path"]
            .as_str()
            .unwrap()
            .ends_with("fallback.dsn"));
        assert!(row["artifacts"]["ses_path"].is_null(), "{row}");
        // Ran and was discarded: both, for whoever wants to look.
        let run = go(vec![
            routed(session(10000, false), 5, 3),
            routed(session(10000, true), 3, 4),
        ])
        .await;
        let row = run.pass("fallback");
        assert_eq!(row["outcome"], "discarded");
        for (key, name) in [("dsn_path", "fallback.dsn"), ("ses_path", "fallback.ses")] {
            assert!(row["artifacts"][key].as_str().unwrap().ends_with(name));
        }
    }

    #[tokio::test]
    async fn a_fallback_that_ends_worse_or_unknown_is_discarded() {
        for end in [Some(4), None] {
            let run = go(vec![
                routed(session(10000, false), 5, 3),
                Reply::Routed {
                    session: session(10000, true),
                    start: Some(3),
                    end,
                },
            ])
            .await;
            assert_eq!(
                run.names(),
                pairs(&[("full", "kept"), ("fallback", "discarded")]),
                "{end:?}"
            );
            assert_eq!(run.ok().result_pass, "full");
            // The row says which of the two it was, in its own words.
            let reason = run.pass("fallback")["reason"].as_str().unwrap();
            let expected = match end {
                Some(_) => "ended with 4 unrouted, more than the 3 it started with",
                None => "gave no unrouted count",
            };
            assert!(reason.contains(expected), "{end:?}: {reason}");
            assert!(
                !reason.contains("closed no connection"),
                "{end:?}: {reason}"
            );
        }
    }

    #[tokio::test]
    async fn a_polish_that_does_not_end_complete_is_discarded() {
        for end in [Some(1), None] {
            let run = go(vec![
                routed(session(10000, false), 5, 3),
                routed(session(10000, true), 3, 0),
                Reply::Routed {
                    session: session(2000, true),
                    start: Some(0),
                    end,
                },
            ])
            .await;
            assert_eq!(
                run.names(),
                pairs(&[
                    ("full", "kept"),
                    ("fallback", "kept"),
                    ("polish", "discarded")
                ]),
                "{end:?}"
            );
            assert_eq!(run.ok().result_pass, "fallback");
            assert_eq!(run.result_text(), session(10000, true));
        }
    }

    #[tokio::test]
    async fn no_fallback_runs_when_it_is_off_or_no_floor_is_safe_or_the_count_is_unknown() {
        let full_left = || vec![routed(session(10000, false), 5, 3)];
        let off = go_with(|plan| plan.fallback = WidthFallback::None, full_left()).await;
        assert_eq!(
            off.names(),
            pairs(&[("full", "kept"), ("fallback", "skipped")])
        );
        assert!(off.pass("fallback")["reason"]
            .as_str()
            .unwrap()
            .contains("width_fallback"));
        assert_eq!(off.calls.len(), 1);
        assert_eq!(off.ok().evidence.router_unrouted, Some(3));

        let no_floor = go_with(
            |plan| {
                plan.floor_retry_um = None;
                plan.no_floor_reason = Some("net classes have different minimum widths".into());
            },
            full_left(),
        )
        .await;
        assert_eq!(
            no_floor.names(),
            pairs(&[("full", "kept"), ("fallback", "skipped")])
        );
        assert!(no_floor.pass("fallback")["reason"]
            .as_str()
            .unwrap()
            .contains("different minimum widths"));

        let unknown = go(vec![Reply::Routed {
            session: session(10000, false),
            start: Some(5),
            end: None,
        }])
        .await;
        assert_eq!(
            unknown.names(),
            pairs(&[("full", "kept"), ("fallback", "skipped")])
        );
        assert!(unknown.pass("fallback")["reason"]
            .as_str()
            .unwrap()
            .contains("no unrouted count"));
        assert_eq!(unknown.ok().evidence.router_unrouted, None);
    }

    #[tokio::test]
    async fn with_the_optimizer_turned_off_no_polish_runs() {
        let run = go_with(
            |plan| plan.base.optimizer_enabled = Some(false),
            vec![
                routed(session(10000, false), 5, 3),
                routed(session(10000, true), 3, 0),
            ],
        )
        .await;
        assert_eq!(run.calls[0].settings.optimizer_enabled, Some(false));
        assert_eq!(run.calls[0].settings.retry_width_um, Some(0.0));
        assert_eq!(
            run.names(),
            pairs(&[
                ("full", "kept"),
                ("fallback", "kept"),
                ("polish", "skipped")
            ])
        );
        assert!(run.pass("polish")["reason"]
            .as_str()
            .unwrap()
            .contains("optimizer_enabled"));
        assert_eq!(run.ok().result_pass, "fallback");
    }

    #[tokio::test]
    async fn a_full_pass_that_fails_fails_the_call() {
        let run = go(vec![Reply::Fail("the job ended in state FAILED")]).await;
        let error = run.outcome.as_ref().err().unwrap();
        assert!(format!("{error:#}").contains("FAILED"));
        let run = go_with(|_| {}, vec![Reply::Timeout]).await;
        let error = run.outcome.as_ref().err().unwrap();
        assert!(
            format!("{error:#}").contains("overall timeout"),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn a_later_pass_that_fails_leaves_the_last_good_session() {
        let run = go(vec![
            routed(session(10000, false), 5, 3),
            Reply::Fail("update_job_settings: neck_width_um: sent 200, in effect 0"),
        ])
        .await;
        assert_eq!(
            run.names(),
            pairs(&[("full", "kept"), ("fallback", "failed")])
        );
        assert!(run.pass("fallback")["reason"]
            .as_str()
            .unwrap()
            .contains("neck_width_um"));
        let outcome = run.ok();
        assert_eq!(outcome.result_pass, "full");
        assert_eq!(outcome.stopped_in, None);
        assert_eq!(run.result_text(), session(10000, false));
        // A session the seed cannot be built from: the pass never starts.
        let foreign = "(session board (routes (resolution um 10) (network_out (net OTHER (wire (path F.Cu 2000 0 0 10 10))))))";
        let run = go(vec![routed(foreign.to_string(), 5, 3)]).await;
        assert_eq!(
            run.names(),
            pairs(&[("full", "kept"), ("fallback", "failed")])
        );
        assert!(run.pass("fallback")["reason"]
            .as_str()
            .unwrap()
            .contains("which the DSN does not have"));
        assert_eq!(run.calls.len(), 1);
    }

    #[tokio::test]
    async fn a_pass_that_runs_out_of_time_stops_the_call_with_the_last_good_session() {
        let run = go(vec![routed(session(10000, false), 5, 3), Reply::Timeout]).await;
        assert_eq!(
            run.names(),
            pairs(&[("full", "kept"), ("fallback", "stopped")])
        );
        let outcome = run.ok();
        assert_eq!(outcome.stopped_in, Some("fallback"));
        assert_eq!(outcome.result_pass, "full");
        assert_eq!(run.result_text(), session(10000, false));

        let run = go(vec![
            routed(session(10000, false), 5, 3),
            routed(session(10000, true), 3, 0),
            Reply::Timeout,
        ])
        .await;
        assert_eq!(
            run.names(),
            pairs(&[
                ("full", "kept"),
                ("fallback", "kept"),
                ("polish", "stopped")
            ])
        );
        assert_eq!(run.ok().stopped_in, Some("polish"));
        assert_eq!(run.ok().result_pass, "fallback");
    }

    #[tokio::test]
    async fn no_pass_starts_once_the_time_is_spent_and_each_gets_what_is_left() {
        // The Full pass uses up the whole call.
        let run = go_with(
            |plan| plan.base.overall_timeout = std::time::Duration::from_millis(40),
            vec![Reply::Slow(80, session(10000, false), Some(5), Some(3))],
        )
        .await;
        assert_eq!(run.calls.len(), 1, "the fallback never started");
        assert_eq!(
            run.names(),
            pairs(&[("full", "kept"), ("fallback", "stopped")])
        );
        assert_eq!(run.ok().stopped_in, Some("fallback"));
        assert_eq!(run.ok().result_pass, "full");
        // A later pass is given only the time that is left.
        let run = go_with(
            |plan| plan.base.overall_timeout = std::time::Duration::from_millis(60_000),
            vec![
                Reply::Slow(120, session(10000, false), Some(5), Some(3)),
                routed(session(10000, true), 3, 1),
            ],
        )
        .await;
        assert_eq!(run.calls[0].settings.overall_timeout.as_millis(), 60_000);
        let left = run.calls[1].settings.overall_timeout.as_millis();
        assert!(left <= 60_000 - 120 && left > 50_000, "{left}");
        assert!(run.ok().elapsed_seconds >= 0.12);
        // Time is left, but less than a pass is worth starting with.
        let run = go_with(
            |plan| plan.least_pass_time = std::time::Duration::from_secs(3600),
            vec![routed(session(10000, false), 5, 3)],
        )
        .await;
        assert_eq!(run.calls.len(), 1);
        assert_eq!(run.ok().stopped_in, Some("fallback"));
        assert!(run.pass("fallback")["reason"]
            .as_str()
            .unwrap()
            .contains("before this pass could start"));
    }

    #[test]
    fn a_pass_rows_reason_is_bounded() {
        let row = super::pass_row("fallback", "failed", Some("x".repeat(5000)));
        assert_eq!(
            row["reason"].as_str().unwrap().len(),
            4 * super::MAX_REASON_DETAIL
        );
        let row = super::pass_row("fallback", "failed", Some("short".into()));
        assert_eq!(row["reason"], "short");
        assert!(super::pass_row("full", "kept", None)["reason"].is_null());
    }
}

#[cfg(test)]
mod freerouting_tests {
    use super::*;
    use crate::router::ToolRouter;
    use crate::tools::ServerConfig;
    use std::sync::Arc;

    fn test_ctx() -> ToolContext {
        ToolContext::new(
            ServerConfig {
                kicad_cli: String::new(),
                kicad_binary: String::new(),
                ipc_address: String::new(),
                project_dir: None,
                jlcpcb_db_path: None,
                auto_load_toolsets: false,
                eager_toolsets: false,
            },
            Arc::new(ToolRouter::new()),
        )
    }

    fn response_json(result: &CallToolResult) -> serde_json::Value {
        match &result.content[0] {
            crate::mcp::protocol::ToolContent::Text { text } => serde_json::from_str(text).unwrap(),
            _ => panic!("expected text content"),
        }
    }

    #[test]
    fn finds_versioned_jar_inside_pcm_plugin_tree() {
        let temp = tempfile::tempdir().unwrap();
        let plugin = temp.path().join("app_freerouting_kicad-plugin").join("lib");
        std::fs::create_dir_all(&plugin).unwrap();
        let jar = plugin.join("freerouting-2.3.0.jar");
        std::fs::write(&jar, b"fixture").unwrap();

        assert_eq!(find_freerouting_jar_below(temp.path(), 5), Some(jar));
    }

    #[tokio::test]
    async fn explicit_missing_jar_reports_each_readiness_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing.jar");
        assert_eq!(
            resolve_freerouting_jar(&json!({ "jar_path": missing })),
            FreeroutingJar::SuppliedPathIsNotAFile(missing.clone())
        );

        let result = handle_check_freerouting(&json!({ "jar_path": missing }), &test_ctx())
            .await
            .unwrap();
        let body = response_json(&result);
        assert_eq!(body["available"], false);
        assert_eq!(body["engine_found"], false);
        assert_eq!(body["native_mcp_available"], false);
        assert_eq!(body["bridge_available"], false);
        // The caller's path is what was checked; discovery never ran, so
        // there is no search to report and no download advice to give.
        assert_eq!(body["checked_path"], json!(missing));
        assert_eq!(
            body["note"],
            json!(format!("jar_path is not a file: {}", missing.display()))
        );
        assert!(body.get("searched_locations").is_none(), "{body}");
    }

    /// A full environment, so every location the search can visit is present.
    fn every_location_environment(root: &Path) -> HashMap<&'static str, std::ffi::OsString> {
        let path = std::env::join_paths([root.join("bin-a"), root.join("bin-b")]).unwrap();
        HashMap::from([
            ("KICAD10_3RD_PARTY", root.join("k10").into_os_string()),
            ("KICAD9_3RD_PARTY", root.join("k9").into_os_string()),
            ("KICAD8_3RD_PARTY", root.join("k8").into_os_string()),
            ("HOME", root.join("home").into_os_string()),
            ("USERPROFILE", root.join("profile").into_os_string()),
            ("PATH", path),
        ])
    }

    /// #787: the `jar_path` description is the documentation of the search.
    /// The search visits locations in the order the description lists them,
    /// visits every location it lists, and the description promises no
    /// configuration setting.
    #[test]
    fn the_search_visits_the_documented_locations_in_the_documented_order() {
        let temp = tempfile::tempdir().unwrap();
        let env = every_location_environment(temp.path());
        let roots =
            freerouting_search_roots_from(|name| env.get(name).cloned(), &temp.path().join("cwd"));

        let visited: Vec<FreeroutingLocation> = roots.iter().map(|root| root.location).collect();
        assert!(
            visited.windows(2).all(|pair| pair[0] <= pair[1]),
            "search order differs from the documented order: {visited:?}"
        );
        let mut distinct = visited.clone();
        distinct.dedup();
        assert_eq!(distinct, FreeroutingLocation::ALL);

        let description = freerouting_jar_path_description();
        let positions: Vec<usize> = FreeroutingLocation::ALL
            .iter()
            .map(|location| {
                description
                    .find(location.description().as_str())
                    .unwrap_or_else(|| panic!("{location:?} is not documented: {description}"))
            })
            .collect();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "{description}"
        );
        assert!(!description.contains("config default"), "{description}");
        assert!(description.contains("There is no configuration setting"));

        // The working-directory root is absolute, so `searched` names the
        // file that was actually checked.
        assert!(roots.contains(&FreeroutingSearchRoot {
            location: FreeroutingLocation::WorkingDirectory,
            path: temp.path().join("cwd").join("freerouting.jar"),
            depth: 0,
        }));
        assert_eq!(
            roots
                .iter()
                .filter(|root| root.location == FreeroutingLocation::PathDirectory)
                .map(|root| root.path.clone())
                .collect::<Vec<_>>(),
            [temp.path().join("bin-a"), temp.path().join("bin-b")]
        );
    }

    /// The tables name concrete places, not only their categories: every
    /// variable, KiCad version, plugin folder, file and directory the search
    /// visits is named in the description, and the search visits each one.
    #[test]
    fn the_description_names_every_concrete_place_the_search_visits() {
        let temp = tempfile::tempdir().unwrap();
        let env = every_location_environment(temp.path());
        let cwd = temp.path().join("cwd");
        let roots = freerouting_search_roots_from(|name| env.get(name).cloned(), &cwd);
        let searched = |path: PathBuf| roots.iter().any(|root| root.path == path);
        let description = freerouting_jar_path_description();
        let named = |text: &str| {
            assert!(
                description.contains(text),
                "{text} is not named: {description}"
            )
        };

        for variable in FREEROUTING_THIRD_PARTY_VARIABLES {
            named(variable);
            assert!(
                searched(PathBuf::from(&env[variable])),
                "{variable}: {roots:?}"
            );
        }
        for version in FREEROUTING_KICAD_VERSIONS {
            named(version);
        }
        for (home, folder, windows_only) in FREEROUTING_PLUGIN_FOLDERS {
            named(&format!("${home}/{folder}"));
            if windows_only && !cfg!(target_os = "windows") {
                continue;
            }
            for version in FREEROUTING_KICAD_VERSIONS {
                let place = folder
                    .replace("<version>", version)
                    .split('/')
                    .fold(PathBuf::from(&env[home]), |p, part| p.join(part));
                assert!(searched(place.clone()), "{place:?}: {roots:?}");
            }
        }
        named(FREEROUTING_WORKING_DIRECTORY_JAR);
        assert!(searched(cwd.join(FREEROUTING_WORKING_DIRECTORY_JAR)));
        for directory in FREEROUTING_SYSTEM_DIRECTORIES {
            named(directory);
            assert!(searched(cwd.join(directory)), "{directory}: {roots:?}");
        }
        named(FREEROUTING_PATH_VARIABLE);
        for directory in std::env::split_paths(&env[FREEROUTING_PATH_VARIABLE]) {
            assert!(searched(directory.clone()), "{directory:?}: {roots:?}");
        }
    }

    /// Every searched location is reported as an absolute path. A relative value
    /// from the environment is taken from the server's working directory, which
    /// is where the search reads it.
    #[test]
    fn every_searched_location_is_absolute() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("cwd");
        let env = HashMap::from([
            ("KICAD10_3RD_PARTY", std::ffi::OsString::from("third-party")),
            ("HOME", std::ffi::OsString::from("home")),
            ("USERPROFILE", std::ffi::OsString::from("profile")),
            (
                "PATH",
                std::env::join_paths([PathBuf::from("bin"), PathBuf::from(".")]).unwrap(),
            ),
        ]);
        let roots = freerouting_search_roots_from(|name| env.get(name).cloned(), &cwd);
        for root in &roots {
            assert!(root.path.is_absolute(), "{root:?}");
        }
        assert!(roots
            .iter()
            .any(|root| root.path == cwd.join("third-party")));
        assert!(roots.iter().any(|root| root.path == cwd.join("bin")));
        let evidence = roots[0].evidence();
        assert_eq!(evidence["location_path"], json!(cwd.join("third-party")));
        assert!(evidence.get("path").is_none(), "{evidence}");
    }

    /// `check_freerouting` with a `jar_path` that is not a file, through the
    /// served `tools/call`: the path checked is named, and no search ran.
    #[tokio::test]
    async fn served_check_freerouting_reports_a_jar_path_that_is_not_a_file() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing.jar");
        let handler = crate::mcp::handler::McpHandler::new(ServerConfig {
            kicad_cli: String::new(),
            kicad_binary: String::new(),
            ipc_address: String::new(),
            project_dir: None,
            jlcpcb_db_path: None,
            auto_load_toolsets: true,
            eager_toolsets: true,
        })
        .await
        .expect("handler builds");
        let response = handler
            .handle_message(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "check_freerouting",
                    "arguments": { "jar_path": missing.display().to_string() }
                }
            }))
            .await
            .expect("tools/call receives a response");
        let result = response.result.expect("result");
        assert_ne!(result["isError"], json!(true), "{result}");
        let body: serde_json::Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["engine_found"], false, "{body}");
        assert_eq!(body["available"], false, "{body}");
        assert_eq!(body["checked_path"], json!(missing), "{body}");
        assert_eq!(
            body["note"],
            json!(format!("jar_path is not a file: {}", missing.display()))
        );
        assert!(body.get("searched_locations").is_none(), "{body}");
    }

    /// A directory reached twice is searched, and reported, once, under the
    /// location that reached it first.
    #[test]
    fn a_root_reached_twice_is_searched_once() {
        let temp = tempfile::tempdir().unwrap();
        let shared = temp.path().join("shared").into_os_string();
        let path = std::env::join_paths([
            temp.path().join("bin"),
            temp.path().join("bin"),
            PathBuf::from(&shared),
        ])
        .unwrap();
        let env = HashMap::from([
            ("KICAD10_3RD_PARTY", shared.clone()),
            ("KICAD9_3RD_PARTY", shared.clone()),
            ("PATH", path),
        ]);
        let roots = freerouting_search_roots_from(|name| env.get(name).cloned(), temp.path());

        let mut paths: Vec<&PathBuf> = roots.iter().map(|root| &root.path).collect();
        let total = paths.len();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), total, "{roots:?}");
        assert_eq!(
            roots
                .iter()
                .find(|root| root.path.as_os_str() == shared)
                .map(|root| root.location),
            Some(FreeroutingLocation::KicadThirdPartyVariable)
        );
        assert_eq!(
            roots
                .iter()
                .filter(|root| root.location == FreeroutingLocation::PathDirectory)
                .count(),
            1
        );
    }

    /// Both tools that take `jar_path` describe it the same way, through the
    /// served `tools/list`.
    #[tokio::test]
    async fn served_jar_path_descriptions_state_the_search() {
        let handler = crate::mcp::handler::McpHandler::new(ServerConfig {
            kicad_cli: String::new(),
            kicad_binary: String::new(),
            ipc_address: String::new(),
            project_dir: None,
            jlcpcb_db_path: None,
            auto_load_toolsets: false,
            eager_toolsets: true,
        })
        .await
        .expect("handler builds");
        let response = handler
            .handle_message(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
            .await
            .expect("tools/list receives a response");
        let tools = response.result.expect("result")["tools"].clone();
        for name in ["check_freerouting", "route_specctra_dsn"] {
            let tool = tools
                .as_array()
                .unwrap()
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap_or_else(|| panic!("{name} is not listed"));
            assert_eq!(
                tool["inputSchema"]["properties"]["jar_path"]["description"],
                json!(freerouting_jar_path_description()),
                "{name}"
            );
        }
    }

    /// `route_specctra_dsn` names the missing file instead of telling a caller
    /// who passed `jar_path` to pass `jar_path`.
    #[tokio::test]
    async fn served_route_refuses_a_jar_path_that_is_not_a_file() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing.jar");
        let handler = crate::mcp::handler::McpHandler::new(ServerConfig {
            kicad_cli: String::new(),
            kicad_binary: String::new(),
            ipc_address: String::new(),
            project_dir: None,
            jlcpcb_db_path: None,
            auto_load_toolsets: true,
            eager_toolsets: true,
        })
        .await
        .expect("handler builds");
        let response = handler
            .handle_message(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "route_specctra_dsn",
                    "arguments": {
                        "dsn_path": temp.path().join("board.dsn").display().to_string(),
                        "ses_output_path": temp.path().join("board.ses").display().to_string(),
                        "engine": "freerouting",
                        "jar_path": missing.display().to_string()
                    }
                }
            }))
            .await
            .expect("tools/call receives a response");
        let result = response.result.expect("result");
        assert_eq!(result["isError"], json!(true), "{result}");
        let body: serde_json::Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["error"]["kind"], "file_not_found", "{body}");
        assert_eq!(body["error"]["path"], json!(missing.display().to_string()));
        assert_eq!(
            body["message"],
            json!(format!("jar_path is not a file: {}", missing.display()))
        );
    }

    #[test]
    fn ignores_unrelated_jars() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("other.jar"), b"fixture").unwrap();
        assert_eq!(find_freerouting_jar_below(temp.path(), 5), None);
    }

    #[tokio::test]
    #[ignore = "requires Java and FREEROUTING_JAR"]
    async fn installed_engine_reports_each_observed_readiness_boundary() {
        let jar = PathBuf::from(std::env::var_os("FREEROUTING_JAR").expect("set FREEROUTING_JAR"));
        let result = handle_check_freerouting(&json!({ "jar_path": jar }), &test_ctx())
            .await
            .unwrap();
        let body = response_json(&result);
        assert_eq!(body["available"], true);
        assert_eq!(body["engine_found"], true);
        assert_eq!(body["native_mcp_available"], true);
        assert_eq!(body["bridge_available"], true);
        assert!(body["native_mcp_tool_count"].as_u64().unwrap() >= 6);
        assert!(body["server_protocol_version"].as_str().is_some());
    }
}

#[cfg(test)]
mod retry_backoff_tests {
    use super::*;

    /// End-to-end check against a real (hand-rolled) flaky HTTP server: two
    /// 503s followed by a 200 should be retried through to success, with
    /// real backoff delays elapsed in between — not just the status-code
    /// decision logic in isolation.
    #[tokio::test]
    async fn get_with_backoff_recovers_after_transient_failures() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            for resp in [
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n",
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n",
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                socket.write_all(resp.as_bytes()).await.unwrap();
            }
        });

        let client = reqwest::Client::new();
        let url = format!("http://{}/x", addr);

        let start = std::time::Instant::now();
        let resp = get_with_backoff(&client, &url).await.unwrap();
        let elapsed = start.elapsed();

        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        // Two retries at 300ms + 600ms = 900ms minimum before the 3rd (successful) attempt.
        assert!(
            elapsed >= std::time::Duration::from_millis(900),
            "expected backoff delays to have elapsed, got {:?}",
            elapsed
        );
    }

    /// A persistent (non-transient) failure should return immediately after
    /// the first attempt — no wasted retries on a 404.
    #[tokio::test]
    async fn get_with_backoff_does_not_retry_client_errors() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            socket
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
            // If get_with_backoff retried, it would try to accept() again here
            // and this task would hang until the test times out.
        });

        let client = reqwest::Client::new();
        let url = format!("http://{}/x", addr);

        let start = std::time::Instant::now();
        let resp = get_with_backoff(&client, &url).await.unwrap();
        let elapsed = start.elapsed();

        assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
        assert!(
            elapsed < std::time::Duration::from_millis(200),
            "expected no retry delay for a 404, took {:?}",
            elapsed
        );
    }

    #[test]
    fn transient_on_rate_limit_and_server_errors() {
        assert!(is_transient_status(reqwest::StatusCode::TOO_MANY_REQUESTS));
        assert!(is_transient_status(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        ));
        assert!(is_transient_status(reqwest::StatusCode::BAD_GATEWAY));
        assert!(is_transient_status(
            reqwest::StatusCode::SERVICE_UNAVAILABLE
        ));
        assert!(is_transient_status(reqwest::StatusCode::GATEWAY_TIMEOUT));
    }

    #[test]
    fn not_transient_on_client_errors() {
        // Retrying a 404/401/403/400 wastes time — the request itself is
        // wrong, not the server having a bad moment.
        assert!(!is_transient_status(reqwest::StatusCode::BAD_REQUEST));
        assert!(!is_transient_status(reqwest::StatusCode::UNAUTHORIZED));
        assert!(!is_transient_status(reqwest::StatusCode::FORBIDDEN));
        assert!(!is_transient_status(reqwest::StatusCode::NOT_FOUND));
    }

    #[test]
    fn not_transient_on_success() {
        assert!(!is_transient_status(reqwest::StatusCode::OK));
        assert!(!is_transient_status(reqwest::StatusCode::NO_CONTENT));
    }

    #[test]
    fn backoff_delay_doubles_each_attempt() {
        assert_eq!(backoff_delay(1), std::time::Duration::from_millis(300));
        assert_eq!(backoff_delay(2), std::time::Duration::from_millis(600));
        assert_eq!(backoff_delay(3), std::time::Duration::from_millis(1200));
    }

    #[test]
    fn backoff_delay_never_panics_on_zero_attempt() {
        // attempt is 1-based in normal use, but the saturating_sub guards
        // against an accidental 0 causing an underflow panic.
        assert_eq!(backoff_delay(0), std::time::Duration::from_millis(300));
    }
}

#[cfg(test)]
mod jlcpcb_database_tests {
    use super::*;
    use std::io::Write;

    fn seed_upstream_database(path: &Path) {
        let conn = rusqlite::Connection::open(path).expect("open upstream database");
        conn.execute_batch(
            "CREATE VIRTUAL TABLE parts USING fts5 (
                 \"LCSC Part\",
                 \"First Category\",
                 \"Second Category\",
                 \"MFR.Part\",
                 \"Package\",
                 \"Manufacturer\",
                 \"Library Type\",
                 \"Description\",
                 \"Datasheet\",
                 \"Price\",
                 \"Stock\"
             );
             INSERT INTO parts VALUES (
                 'C14663', 'Resistors', 'Chip Resistor - Surface Mount',
                 'RC0402FR-0710KL', '0402', 'YAGEO', 'Basic',
                 '10k resistor 0402', 'https://example.com/datasheet.pdf',
                 '1-9:0.012,10-99:0.008', '5000'
             );",
        )
        .expect("seed upstream database");
    }

    #[test]
    fn parses_bounded_chunk_count() {
        assert_eq!(parse_jlcpcb_chunk_count("3\n").unwrap(), 3);
        assert!(parse_jlcpcb_chunk_count("0").is_err());
        assert!(parse_jlcpcb_chunk_count("65").is_err());
        assert!(parse_jlcpcb_chunk_count("not-a-number").is_err());
    }

    #[test]
    fn extracts_single_database_from_feed_archive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive_path = dir.path().join("parts.db.zip");
        let output_path = dir.path().join("upstream.db");
        let archive_file = std::fs::File::create(&archive_path).unwrap();
        let mut archive = zip::ZipWriter::new(archive_file);
        archive
            .start_file(
                "current-parts-fts5.db",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        archive.write_all(b"SQLite database bytes").unwrap();
        archive.finish().unwrap();

        extract_jlcpcb_database(&archive_path, &output_path).unwrap();

        assert_eq!(
            std::fs::read(output_path).unwrap(),
            b"SQLite database bytes"
        );
    }

    #[test]
    fn builds_stable_components_database_from_current_feed_schema() {
        let dir = tempfile::tempdir().expect("tempdir");
        let upstream_path = dir.path().join("upstream.db");
        let output_path = dir.path().join("konnect.db");
        seed_upstream_database(&upstream_path);

        let part_count = build_konnect_jlcpcb_database(&upstream_path, &output_path).unwrap();
        assert_eq!(part_count, 1);

        let conn = rusqlite::Connection::open(output_path).expect("open generated database");
        let row: (String, f64, i64, String) = conn
            .query_row(
                "SELECT LCSC, Price, Stock, Category FROM components",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, "C14663");
        assert!((row.1 - 0.012).abs() < f64::EPSILON);
        assert_eq!(row.2, 5000);
        assert_eq!(row.3, "Resistors / Chip Resistor - Surface Mount");
    }

    #[test]
    fn rejects_an_upstream_schema_missing_required_columns() {
        let dir = tempfile::tempdir().expect("tempdir");
        let upstream_path = dir.path().join("upstream.db");
        let output_path = dir.path().join("konnect.db");
        let conn = rusqlite::Connection::open(&upstream_path).expect("open upstream database");
        conn.execute("CREATE TABLE parts (\"LCSC Part\" TEXT)", [])
            .unwrap();
        drop(conn);

        let error = build_konnect_jlcpcb_database(&upstream_path, &output_path)
            .unwrap_err()
            .to_string();
        assert!(error.contains("missing required columns"));
    }

    #[test]
    fn replaces_existing_database_and_removes_temporary_backup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let destination = dir.path().join("jlcpcb.db");
        let staged = dir.path().join("new.db");
        let backup = dir.path().join("previous.db");
        std::fs::write(&destination, b"old").unwrap();
        std::fs::write(&staged, b"new").unwrap();

        replace_jlcpcb_database(&staged, &destination, &backup).unwrap();

        assert_eq!(std::fs::read(destination).unwrap(), b"new");
        assert!(!backup.exists());
    }
}

#[cfg(test)]
mod jlcpcb_cache_tests {
    use super::*;
    use crate::router::ToolRouter;
    use crate::tools::ServerConfig;
    use std::sync::Arc;

    fn test_ctx() -> ToolContext {
        ToolContext::new(
            ServerConfig {
                kicad_cli: String::new(),
                kicad_binary: String::new(),
                ipc_address: String::new(),
                project_dir: None,
                jlcpcb_db_path: None,
                auto_load_toolsets: false,
                eager_toolsets: false,
            },
            Arc::new(ToolRouter::new()),
        )
    }

    /// Builds a temp SQLite file with a `components` table matching the
    /// schema the handlers query, seeded with one part.
    fn seed_test_db() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("jlcpcb.db");
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        conn.execute(
            "CREATE TABLE components (
                LCSC TEXT, MFR_Part TEXT, Package TEXT, Manufacturer TEXT,
                Library_Type TEXT, Description TEXT, Datasheet TEXT,
                Price REAL, Stock INTEGER, Category TEXT
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO components VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                "C14663",
                "RC0402FR-0710KL",
                "0402",
                "YAGEO",
                "Basic",
                "10k resistor 0402",
                "https://www.lcsc.com/datasheet/C14663.pdf",
                0.01,
                5000,
                "Resistors / Chip Resistor - Surface Mount"
            ],
        )
        .unwrap();
        (dir, db_path)
    }

    #[tokio::test]
    async fn search_jlcpcb_parts_caches_repeated_query() {
        let (_dir, db_path) = seed_test_db();
        let ctx = test_ctx();
        let args = json!({
            "query": "10k",
            "output_path": db_path.to_str().unwrap()
        });

        let first = handle_search_jlcpcb_parts(&args, &ctx).await.unwrap();
        let second = handle_search_jlcpcb_parts(&args, &ctx).await.unwrap();

        let first_body = response_json(&first);
        let second_body = response_json(&second);
        assert_eq!(first_body["cached"], json!(false));
        assert_eq!(second_body["cached"], json!(true));
        assert_eq!(first_body["results"], second_body["results"]);
        assert_eq!(first_body["count"], json!(1));
    }

    #[tokio::test]
    async fn search_jlcpcb_parts_different_query_is_a_cache_miss() {
        let (_dir, db_path) = seed_test_db();
        let ctx = test_ctx();

        let args_a = json!({ "query": "10k", "output_path": db_path.to_str().unwrap() });
        let args_b = json!({ "query": "100nF", "output_path": db_path.to_str().unwrap() });

        handle_search_jlcpcb_parts(&args_a, &ctx).await.unwrap();
        let second = handle_search_jlcpcb_parts(&args_b, &ctx).await.unwrap();

        assert_eq!(response_json(&second)["cached"], json!(false));
    }

    #[tokio::test]
    async fn get_jlcpcb_part_caches_repeated_lookup() {
        let (_dir, db_path) = seed_test_db();
        let ctx = test_ctx();
        let args = json!({
            "lcsc_id": "C14663",
            "output_path": db_path.to_str().unwrap()
        });

        let first = handle_get_jlcpcb_part(&args, &ctx).await.unwrap();
        let second = handle_get_jlcpcb_part(&args, &ctx).await.unwrap();

        assert_eq!(response_json(&first)["cached"], json!(false));
        assert_eq!(response_json(&second)["cached"], json!(true));
        assert_eq!(response_json(&first)["lcsc"], json!("C14663"));
    }

    #[tokio::test]
    async fn suggest_alternatives_caches_repeated_query() {
        let (_dir, db_path) = seed_test_db();
        let ctx = test_ctx();
        let args = json!({
            "value": "10k",
            "footprint": "Resistor_SMD:R_0402",
            "output_path": db_path.to_str().unwrap()
        });

        let first = handle_suggest_alternatives(&args, &ctx).await.unwrap();
        let second = handle_suggest_alternatives(&args, &ctx).await.unwrap();

        assert_eq!(response_json(&first)["cached"], json!(false));
        assert_eq!(response_json(&second)["cached"], json!(true));
    }

    /// Seeds parts whose search words are spread over different columns.
    fn seed_multi_word_db() -> (tempfile::TempDir, std::path::PathBuf) {
        let (dir, db_path) = seed_test_db();
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        for (lcsc, mpn, package, mfr, desc) in [
            (
                "C1001",
                "PH-1x7",
                "2.54mm",
                "BOOMELE",
                "Pin Header 1x7 Through Hole Male",
            ),
            (
                "C1002",
                "PH-1x40",
                "2.54mm",
                "BOOMELE",
                "40P Pin Header Through Hole",
            ),
            (
                "C1003",
                "CAP-50",
                "0603",
                "Samsung",
                "50% duty_cycle capacitor",
            ),
            (
                "C1004",
                "CAP-51",
                "0603",
                "Samsung",
                "50 duty cycle capacitor",
            ),
        ] {
            conn.execute(
                "INSERT INTO components VALUES (?1, ?2, ?3, ?4, 'Extended', ?5, '', 0.1, 10, 'Misc')",
                rusqlite::params![lcsc, mpn, package, mfr, desc],
            )
            .unwrap();
        }
        (dir, db_path)
    }

    async fn search_lcsc_ids(db_path: &std::path::Path, query: &str) -> Vec<String> {
        let ctx = test_ctx();
        let args = json!({ "query": query, "output_path": db_path.to_str().unwrap() });
        let result = handle_search_jlcpcb_parts(&args, &ctx).await.unwrap();
        let mut ids: Vec<String> = response_json(&result)["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["lcsc"].as_str().unwrap().to_string())
            .collect();
        ids.sort();
        ids
    }

    /// The defect in #432: the whole query was bound as one `LIKE '%query%'`
    /// phrase against Description/MFR_Part, so reordering, trimming or adding
    /// a word that is not in that exact order flipped a hit into zero results.
    #[tokio::test]
    async fn search_jlcpcb_parts_matches_every_token_in_any_order() {
        let (_dir, db_path) = seed_multi_word_db();
        assert_eq!(
            search_lcsc_ids(&db_path, "Pin Header 1x7").await,
            vec!["C1001"]
        );
        assert_eq!(
            search_lcsc_ids(&db_path, "  1x7   header pin ").await,
            vec!["C1001"]
        );
        assert_eq!(
            search_lcsc_ids(&db_path, "through hole pin header").await,
            vec!["C1001", "C1002"]
        );
        assert_eq!(
            search_lcsc_ids(&db_path, "40P Pin Header Through Hole").await,
            vec!["C1002"]
        );
    }

    #[tokio::test]
    async fn search_jlcpcb_parts_tokens_may_span_columns() {
        let (_dir, db_path) = seed_multi_word_db();
        // Package, Manufacturer, LCSC and MFR_Part are all searchable.
        assert_eq!(
            search_lcsc_ids(&db_path, "boomele 2.54mm 1x7").await,
            vec!["C1001"]
        );
        assert_eq!(
            search_lcsc_ids(&db_path, "C1002 header").await,
            vec!["C1002"]
        );
        assert_eq!(
            search_lcsc_ids(&db_path, "PH-1x40 boomele").await,
            vec!["C1002"]
        );
    }

    #[tokio::test]
    async fn search_jlcpcb_parts_absent_token_returns_no_row() {
        let (_dir, db_path) = seed_multi_word_db();
        assert!(search_lcsc_ids(&db_path, "pin header 1x7 nonexistentword")
            .await
            .is_empty());
    }

    /// The filters and LIMIT keep working alongside the tokenised match.
    /// (The query has no ORDER BY, before or after the fix.)
    #[tokio::test]
    async fn search_jlcpcb_parts_preserves_stock_basic_category_and_limit_filters() {
        let (_dir, db_path) = seed_multi_word_db();
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        for (lcsc, lib, desc, stock, cat) in [
            ("C2001", "Basic", "Widget gadget alpha", 10, "Gadgets"),
            ("C2002", "Extended", "Widget gadget beta", 10, "Gadgets"),
            ("C2003", "Basic", "Widget gadget gamma", 0, "Gadgets"),
            ("C2004", "Basic", "Widget gadget delta", 10, "Other"),
        ] {
            conn.execute(
                "INSERT INTO components VALUES (?1, 'MPN', 'PKG', 'MFR', ?2, ?3, '', 0.1, ?4, ?5)",
                rusqlite::params![lcsc, lib, desc, stock, cat],
            )
            .unwrap();
        }
        let ctx = test_ctx();
        let run = |extra: serde_json::Value| {
            let mut args =
                json!({ "query": "gadget widget", "output_path": db_path.to_str().unwrap() });
            for (k, v) in extra.as_object().unwrap() {
                args[k] = v.clone();
            }
            let ctx = &ctx;
            async move {
                let result = handle_search_jlcpcb_parts(&args, ctx).await.unwrap();
                let mut ids: Vec<String> = response_json(&result)["results"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| r["lcsc"].as_str().unwrap().to_string())
                    .collect();
                ids.sort();
                ids
            }
        };
        // in_stock defaults to true: C2003 (stock 0) is excluded.
        assert_eq!(run(json!({})).await, vec!["C2001", "C2002", "C2004"]);
        assert_eq!(
            run(json!({ "in_stock": false })).await,
            vec!["C2001", "C2002", "C2003", "C2004"]
        );
        assert_eq!(
            run(json!({ "basic_only": true })).await,
            vec!["C2001", "C2004"]
        );
        assert_eq!(
            run(json!({ "category": "Gadgets" })).await,
            vec!["C2001", "C2002"]
        );
        assert_eq!(
            run(json!({ "category": "Gadgets", "basic_only": true, "in_stock": false })).await,
            vec!["C2001", "C2003"]
        );
        assert_eq!(run(json!({ "limit": 2 })).await.len(), 2);
    }

    #[tokio::test]
    async fn search_jlcpcb_parts_like_metacharacters_are_literal() {
        let (_dir, db_path) = seed_multi_word_db();
        assert_eq!(search_lcsc_ids(&db_path, "50%").await, vec!["C1003"]);
        assert_eq!(search_lcsc_ids(&db_path, "duty_cycle").await, vec!["C1003"]);
        assert!(search_lcsc_ids(&db_path, "%").await.len() == 1);
    }

    fn response_json(result: &CallToolResult) -> serde_json::Value {
        match &result.content[0] {
            crate::mcp::protocol::ToolContent::Text { text } => serde_json::from_str(text).unwrap(),
            _ => panic!("expected text content"),
        }
    }

    /// The defect in #255: the local catalog holds exact datasheet URLs, but
    /// get_datasheet_url only ever asked the live LCSC API and returned null.
    /// A catalog hit must answer directly — no network involved.
    #[tokio::test]
    async fn get_datasheet_url_answers_from_the_local_catalog() {
        let (_dir, db_path) = seed_test_db();
        let ctx = test_ctx();

        let by_lcsc = handle_get_datasheet_url(
            &json!({ "lcsc_id": "C14663", "output_path": db_path.to_str().unwrap() }),
            &ctx,
        )
        .await
        .unwrap();
        let body = response_json(&by_lcsc);
        assert_eq!(
            body["datasheet_url"],
            json!("https://www.lcsc.com/datasheet/C14663.pdf")
        );
        assert_eq!(body["source"], json!("local_catalog"));

        let by_mpn = handle_get_datasheet_url(
            &json!({ "mpn": "RC0402FR-0710KL", "output_path": db_path.to_str().unwrap() }),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(
            response_json(&by_mpn)["datasheet_url"],
            json!("https://www.lcsc.com/datasheet/C14663.pdf")
        );
    }

    /// The catalog projections hid the column too — a caller could not get a
    /// URL from part lookup either.
    #[tokio::test]
    async fn catalog_lookups_carry_the_datasheet_url() {
        let (_dir, db_path) = seed_test_db();
        let ctx = test_ctx();

        let part = handle_get_jlcpcb_part(
            &json!({ "lcsc_id": "C14663", "output_path": db_path.to_str().unwrap() }),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(
            response_json(&part)["datasheet_url"],
            json!("https://www.lcsc.com/datasheet/C14663.pdf")
        );

        let search = handle_search_jlcpcb_parts(
            &json!({ "query": "10k", "output_path": db_path.to_str().unwrap() }),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(
            response_json(&search)["results"][0]["datasheet_url"],
            json!("https://www.lcsc.com/datasheet/C14663.pdf")
        );
    }

    #[tokio::test]
    async fn search_jlcpcb_parts_limit_page_is_ordered_by_lcsc() {
        let (_dir, db_path) = seed_multi_word_db();
        let ctx = test_ctx();
        // Insert out of LCSC order so rowid order differs from LCSC order.
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        for lcsc in ["C3003", "C3001", "C3002"] {
            conn.execute(
                "INSERT INTO components VALUES (?1, 'ORD-1', '0805', 'Acme', 'Basic', 'ordercheck part', '', 0.1, 10, 'Misc')",
                rusqlite::params![lcsc],
            )
            .unwrap();
        }
        let args = json!({
            "query": "ordercheck",
            "limit": 2,
            "output_path": db_path.to_str().unwrap()
        });
        let body = response_json(&handle_search_jlcpcb_parts(&args, &ctx).await.unwrap());
        let ids: Vec<&str> = body["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["lcsc"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["C3001", "C3002"]);
    }

    #[tokio::test]
    async fn served_tools_call_reports_tokens_and_fields_on_miss_and_cache_hit() {
        let (_dir, db_path) = seed_multi_word_db();
        let handler = crate::mcp::handler::McpHandler::new(ServerConfig {
            kicad_cli: String::new(),
            kicad_binary: String::new(),
            ipc_address: String::new(),
            project_dir: None,
            jlcpcb_db_path: Some(db_path.clone()),
            auto_load_toolsets: true,
            eager_toolsets: true,
        })
        .await
        .expect("handler builds");
        let call = || async {
            let response = handler
                .handle_message(json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": {
                        "name": "search_jlcpcb_parts",
                        "arguments": {
                            "query": "  header   Pin 1x7 ",
                            "limit": 5
                        }
                    }
                }))
                .await
                .expect("response");
            let result = response.result.expect("result");
            serde_json::from_str::<serde_json::Value>(
                result["content"][0]["text"].as_str().expect("text"),
            )
            .expect("json body")
        };
        let fresh = call().await;
        let hit = call().await;
        assert_eq!(fresh["cached"], json!(false));
        assert_eq!(hit["cached"], json!(true));
        for body in [&fresh, &hit] {
            assert_eq!(body["tokens"], json!(["header", "Pin", "1x7"]));
            assert_eq!(
                body["search_fields"],
                json!(["Description", "MFR_Part", "LCSC", "Package", "Manufacturer"])
            );
            assert_eq!(body["query"], json!("  header   Pin 1x7 "));
        }
        assert_eq!(fresh["results"], hit["results"]);
        assert!(fresh["count"].as_u64().unwrap() >= 1);
    }

    /// With no database at all the tool degrades to its old behaviour — web
    // lookup (unreachable in tests), then null with a note naming what was
    // tried rather than blaming only the API (#255).
    #[tokio::test]
    async fn a_catalog_miss_falls_through_and_the_note_names_the_sources() {
        let ctx = test_ctx(); // jlcpcb_db_path: None → default path absent

        let result = handle_get_datasheet_url(&json!({ "lcsc_id": "C99999999" }), &ctx)
            .await
            .unwrap();
        let body = response_json(&result);
        assert_eq!(body["datasheet_url"], serde_json::Value::Null);
        let note = body["note"].as_str().unwrap();
        assert!(note.contains("local catalog"), "{note}");
    }
}

#[cfg(test)]
mod suggest_alternatives_tests {
    use super::*;
    use crate::tools::ServerConfig;

    /// Real rows of the Konnect-built catalogue (`components` table), copied
    /// verbatim from a `download_jlcpcb_database` run on 2026-10-03. The
    /// README beside it records the source and how the rows were chosen.
    const FIXTURE: &str = include_str!("../../tests/fixtures/jlcpcb/suggest_alternatives.tsv");

    fn fixture_db() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jlcpcb.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        // The table `build_konnect_jlcpcb_database` creates.
        conn.execute_batch(
            "CREATE TABLE components (
                 LCSC TEXT NOT NULL PRIMARY KEY, MFR_Part TEXT NOT NULL,
                 Package TEXT NOT NULL, Manufacturer TEXT NOT NULL,
                 Library_Type TEXT NOT NULL, Description TEXT NOT NULL,
                 Datasheet TEXT NOT NULL, Price REAL NOT NULL,
                 Stock INTEGER NOT NULL, Category TEXT NOT NULL
             );",
        )
        .unwrap();
        let mut lines = FIXTURE.lines();
        assert_eq!(
            lines.next().unwrap(),
            "LCSC\tMFR_Part\tPackage\tManufacturer\tLibrary_Type\tDescription\tDatasheet\tPrice\tStock\tCategory"
        );
        for line in lines.filter(|line| !line.is_empty()) {
            let f: Vec<&str> = line.split('\t').collect();
            assert_eq!(f.len(), 10, "{line}");
            conn.execute(
                "INSERT INTO components VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    f[0],
                    f[1],
                    f[2],
                    f[3],
                    f[4],
                    f[5],
                    f[6],
                    f[7].parse::<f64>().unwrap(),
                    f[8].parse::<i64>().unwrap(),
                    f[9]
                ],
            )
            .unwrap();
        }
        (dir, path)
    }

    async fn served(db: &Path, arguments: serde_json::Value) -> (bool, serde_json::Value) {
        let handler = crate::mcp::handler::McpHandler::new(ServerConfig {
            kicad_cli: String::new(),
            kicad_binary: String::new(),
            ipc_address: String::new(),
            project_dir: None,
            jlcpcb_db_path: Some(db.to_path_buf()),
            auto_load_toolsets: false,
            eager_toolsets: true,
        })
        .await
        .expect("handler builds");
        let response = handler
            .handle_message(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": "suggest_jlcpcb_alternatives", "arguments": arguments }
            }))
            .await
            .expect("tools/call receives a response");
        let result = response.result.expect("result");
        let body = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        (result["isError"] == json!(true), body)
    }

    fn lcsc(body: &serde_json::Value) -> Vec<&str> {
        body["alternatives"]
            .as_array()
            .unwrap_or_else(|| panic!("no alternatives: {body}"))
            .iter()
            .map(|part| part["lcsc"].as_str().unwrap())
            .collect()
    }

    /// #785: real KiCad 10 footprint IDs reach real parts, the value matches
    /// as a whole value, and the Basic part with millions in stock comes
    /// first. On `main` every one of these returned `[]`.
    #[tokio::test]
    async fn standard_kicad_footprints_find_the_value_with_the_basic_part_first() {
        let (_dir, db) = fixture_db();
        for (value, footprint, expected) in [
            (
                "100nF",
                "Capacitor_SMD:C_0402_1005Metric",
                vec!["C1525", "C307331", "C285038", "C359190", "C285045"],
            ),
            (
                "10k",
                "Resistor_SMD:R_0402_1005Metric",
                vec!["C25744", "C22356213", "C174175"],
            ),
            (
                "20pF",
                "Capacitor_SMD:C_0402_1005Metric",
                vec!["C1554", "C107000"],
            ),
            (
                "AMS1117-3.3",
                "Package_TO_SOT_SMD:SOT-223-3_TabPin2",
                vec!["C6186", "C2992570"],
            ),
            (
                "STM32F411CEU6",
                "Package_DFN_QFN:QFN-48-1EP_7x7mm_P0.5mm_EP5.6x5.6mm",
                vec!["C60420"],
            ),
            (
                "LM358",
                "Package_SO:SOIC-8_3.9x4.9mm_P1.27mm",
                vec!["C7950", "C5252902", "C5423"],
            ),
        ] {
            let (is_error, body) =
                served(&db, json!({ "value": value, "footprint": footprint })).await;
            assert!(!is_error, "{value} / {footprint}: {body}");
            assert_eq!(lcsc(&body), expected, "{value} / {footprint}: {body}");
            assert_eq!(body["count"], expected.len(), "{body}");
        }
    }

    /// The evidence for the first case: which packages were searched, how
    /// the value was matched, the ranking, and what the stock floor removed
    /// (four parts with 8 to 19 in stock, which used to lead the list).
    #[tokio::test]
    async fn the_response_states_the_package_match_ranking_and_exclusions() {
        let (_dir, db) = fixture_db();
        let (_, body) = served(
            &db,
            json!({ "value": "100nF", "footprint": "Capacitor_SMD:C_0402_1005Metric" }),
        )
        .await;
        assert_eq!(body["package_match"]["rule"], "chip_imperial");
        assert_eq!(body["package_match"]["lcsc_packages"], json!(["0402"]));
        assert_eq!(
            body["value_match"]["columns"],
            json!(["Description", "MFR_Part"])
        );
        assert_eq!(
            body["ranking"]["criteria"],
            json!([
                "library_type: Basic, Preferred, Extended",
                "known price before unknown price",
                "price ascending",
                "stock descending",
                "LCSC number"
            ])
        );
        assert_eq!(body["ranking"]["min_stock_count"], 100);
        assert_eq!(body["ranking"]["prefer_basic"], true);
        assert_eq!(body["count"], 5);
        assert_eq!(body["matched_count"], 9);
        assert_eq!(
            body["exclusions"],
            json!({
                "below_min_stock_count": 4,
                "unknown_price_count": 0,
                "above_max_price_count": 0
            })
        );
    }

    /// `10k` used to match `110kΩ` and `510kΩ`, and `0402` used to match the
    /// `0402x4` resistor arrays. Neither is counted as a match now.
    #[tokio::test]
    async fn neighbouring_values_and_array_packages_are_not_matches() {
        let (_dir, db) = fixture_db();
        let (_, body) = served(
            &db,
            json!({
                "value": "10k",
                "footprint": "Resistor_SMD:R_0402_1005Metric",
                "min_stock_count": 0,
                "limit": 50
            }),
        )
        .await;
        assert_eq!(lcsc(&body), ["C25744", "C22356213", "C174175"], "{body}");
        assert_eq!(body["matched_count"], 3, "{body}");
        for part in body["alternatives"].as_array().unwrap() {
            assert_eq!(part["package"], "0402", "{part}");
            assert!(
                part["description"].as_str().unwrap().contains(" 10kΩ "),
                "{part}"
            );
        }
    }

    /// Preferred parts rank between Basic and Extended; turning the
    /// preference off ranks by price.
    #[tokio::test]
    async fn library_type_ranks_before_price_unless_turned_off() {
        let (_dir, db) = fixture_db();
        let args = |prefer_basic: bool| {
            json!({
                "value": "510k",
                "footprint": "Resistor_SMD:R_0402_1005Metric",
                "prefer_basic": prefer_basic
            })
        };
        let (_, preferred) = served(&db, args(true)).await;
        assert_eq!(lcsc(&preferred), ["C11616", "C25564"], "{preferred}");
        // C170418 has 6 in stock and an unknown price; the floor removes it.
        assert_eq!(preferred["exclusions"]["below_min_stock_count"], 1);

        let (_, by_price) = served(&db, args(false)).await;
        assert_eq!(lcsc(&by_price), ["C25564", "C11616"], "{by_price}");
        assert_eq!(by_price["ranking"]["prefer_basic"], false);
        assert_eq!(
            by_price["ranking"]["criteria"][0],
            "known price before unknown price"
        );
    }

    /// #582's stored zero is an unknown price: it sorts after every known
    /// price, and a price limit excludes it rather than admitting it as free.
    #[tokio::test]
    async fn an_unknown_price_never_sorts_as_the_cheapest() {
        let (_dir, db) = fixture_db();
        let (_, body) = served(
            &db,
            json!({ "value": "1.5M", "footprint": "Resistor_SMD:R_0402_1005Metric" }),
        )
        .await;
        assert_eq!(lcsc(&body), ["C22369344", "C138034", "C11812"], "{body}");

        let (_, limited) = served(
            &db,
            json!({
                "value": "1.5M",
                "footprint": "Resistor_SMD:R_0402_1005Metric",
                "max_price_usd": 0.0015
            }),
        )
        .await;
        assert_eq!(lcsc(&limited), ["C22369344"], "{limited}");
        assert_eq!(limited["exclusions"]["unknown_price_count"], 1, "{limited}");
        assert_eq!(
            limited["exclusions"]["above_max_price_count"], 1,
            "{limited}"
        );
    }

    /// With both overrides relaxed, the old cheapest-first order comes back,
    /// so a caller who wants it can still have it.
    #[tokio::test]
    async fn the_stock_floor_and_preference_are_overridable() {
        let (_dir, db) = fixture_db();
        let (_, body) = served(
            &db,
            json!({
                "value": "100nF",
                "footprint": "0402",
                "min_stock_count": 0,
                "prefer_basic": false
            }),
        )
        .await;
        assert_eq!(
            lcsc(&body),
            ["C285038", "C2932181", "C2838746", "C3152530", "C18255880"],
            "{body}"
        );
        assert_eq!(body["package_match"]["rule"], "lcsc_name");
        assert_eq!(body["exclusions"]["below_min_stock_count"], 0);
    }

    /// A library footprint the table does not cover is refused, naming the
    /// argument, instead of returning an empty list that reads as "no
    /// alternatives exist".
    #[tokio::test]
    async fn an_unmapped_footprint_and_an_empty_value_are_refused() {
        let (_dir, db) = fixture_db();
        let (is_error, body) = served(
            &db,
            json!({
                "value": "100nF",
                "footprint": "Connector_PinHeader_2.54mm:PinHeader_1x02_P2.54mm_Vertical"
            }),
        )
        .await;
        assert!(is_error, "{body}");
        assert_eq!(body["error"]["kind"], "invalid_argument");
        assert_eq!(body["error"]["field"], "footprint");

        let (is_error, body) = served(
            &db,
            json!({ "value": "  ", "footprint": "Capacitor_SMD:C_0402_1005Metric" }),
        )
        .await;
        assert!(is_error, "{body}");
        assert_eq!(body["error"]["field"], "value");
    }
}

#[cfg(all(test, unix))]
mod freerouting_setup_tests {
    use super::*;
    use crate::freerouting_setup::{tests::fake_jar, ONLY_EXPLICIT};
    use crate::tools::cli::test_support::write_script;

    /// A `java` that reports Java 25 and otherwise plays Freerouting 2.5.0's
    /// MCP server: it applies whatever settings it is sent, completes the
    /// job at once, logs a stage summary with 0 unrouted, and writes a
    /// one-line session.
    fn routing_java(dir: &Path) -> PathBuf {
        routing_java_writing(dir, "java25", r#"(session "board")"#)
    }

    /// The same, writing `session` as the routed session. The text must hold
    /// no single quote: it sits inside one in the script.
    fn routing_java_writing(dir: &Path, name: &str, session: &str) -> PathBuf {
        assert!(!session.contains('\''));
        let object = r#"{"type":"object","properties":{"jobId":{"type":"string"},"filePath":{"type":"string"}},"required":["jobId","filePath"]}"#;
        let with_path = r#"{"type":"object","properties":{"path":{"type":"object","properties":{"jobId":{"type":"string"}},"required":["jobId"]}},"required":["path"]}"#;
        let enqueue = r#"{"type":"object","properties":{"body":{"type":"object","properties":{"session_id":{"type":"string"}},"required":["session_id"]}},"required":["body"]}"#;
        let tool =
            |name: &str, schema: &str| format!(r#"{{"name":"{name}","inputSchema":{schema}}}"#);
        let tools = [
            tool("create_session", r#"{"type":"object","properties":{}}"#),
            tool("enqueue_job", enqueue),
            tool("upload_job_input_from_local_file", object),
            tool("update_job_settings", with_path),
            tool("get_effective_settings", with_path),
            tool("start_job", with_path),
            tool("get_job_details", with_path),
            tool("get_job_logs", with_path),
            tool("download_job_output_to_local_file", object),
        ]
        .join(",");
        let settings = r#"{"status":200,"body":{"automatic_neckdown":false,"autorouter":{"max_passes":20},"optimizer":{"enabled":true,"improvement_threshold":0.25,"max_consecutive_failures":1000000,"max_consecutive_failures_pass1":1000000},"scoring":{"via_costs":10,"default_undesired_direction_trace_cost":1.0},"fanout":{"enabled":false}}}"#;
        let logs = r#"{"status":200,"body":[{"type":"INFO","message":"Auto-routing stage completed: started with 2 unrouted nets, completed in 0.10 seconds, final score: 1000.00 (0 unrouted and 0 violations)"}]}"#;
        let body = format!(
            r#"#!/bin/sh
if [ "$1" = '-version' ]; then echo 'openjdk version "25.0.1" 2026-01-01' >&2; exit 0; fi
reply() {{ printf '{{"jsonrpc":"2.0","id":%s,"result":%s}}\n' "$1" "$2"; }}
while IFS= read -r line; do
  id=$(printf '%s' "$line" | grep -o '"id":[0-9]*' | head -n 1 | cut -d: -f2)
  [ -z "$id" ] && continue
  case "$line" in
    *'"method":"initialize"'*) reply "$id" '{{"protocolVersion":"2024-11-05"}}' ;;
    *'"method":"tools/list"'*) reply "$id" '{{"tools":[{tools}]}}' ;;
    *'"name":"create_session"'*) reply "$id" '{{"session_id":"s1"}}' ;;
    *'"name":"enqueue_job"'*) reply "$id" '{{"job_id":"j1"}}' ;;
    *'"name":"get_effective_settings"'*) reply "$id" '{settings}' ;;
    *'"name":"get_job_details"'*) reply "$id" '{{"state":"COMPLETED"}}' ;;
    *'"name":"get_job_logs"'*) reply "$id" '{logs}' ;;
    *'"name":"download_job_output_to_local_file"'*)
      out=$(printf '%s' "$line" | grep -o '"filePath":"[^"]*"' | cut -d'"' -f4)
      printf '%s\n' '{session}' > "$out"
      reply "$id" '{{"message":"saved"}}' ;;
    *) reply "$id" '{{}}' ;;
  esac
done
"#
        );
        write_script(dir, name, &body, "")
    }

    // ─── S9, step A: the width report in the routing result ────────────────

    /// The keys of a routing result before row S9.
    const KEYS_BEFORE_S9: [&str; 16] = [
        "artifacts",
        "bridge",
        "completion",
        "effective_settings",
        "elapsed_seconds",
        "engine",
        "final_state",
        "job_id",
        "method",
        "native_mcp",
        "optimizer",
        "poll_count",
        "rules_applied",
        "ses_bytes",
        "session_id",
        "status",
    ];

    const NARROW_SESSION: &str = "(session board (routes (resolution um 10) (network_out (net VBUS (wire (path F.Cu 10000 0 0 30000 0)) (wire (path F.Cu 2000 30000 0 30000 25000))))))";

    fn widened_manifest() -> serde_json::Value {
        json!({
            "layers": [{ "dsn_name": "F.Cu", "kicad_name": "F.Cu" }],
            "intent_widths": [
                { "net": "VBUS", "current_a": 2.0, "from_mm": 0.2, "to_mm": 1.0, "status": "widened" }
            ]
        })
    }

    /// Route `dsn_text` with the fake Freerouting `java`, with `manifest`
    /// beside the DSN when one is given.
    async fn route_with(
        dir: &Path,
        java: &Path,
        name: &str,
        dsn_text: &str,
        manifest: Option<serde_json::Value>,
    ) -> serde_json::Value {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let jar = dir.join("freerouting-2.5.0.jar");
        if !jar.exists() {
            fake_jar(dir, "freerouting-2.5.0.jar", 69);
        }
        let dsn = dir.join(format!("{name}.dsn"));
        std::fs::write(&dsn, dsn_text).unwrap();
        if let Some(manifest) = manifest {
            std::fs::write(
                dir.join(format!("{name}.dsn.konnect.json")),
                manifest.to_string(),
            )
            .unwrap();
        }
        let args = json!({ "dsn_path": dsn, "ses_output_path": dir.join(format!("{name}.ses")),
                           "engine": "freerouting", "jar_path": jar, "java_path": java, "overall_timeout_seconds": 60 });
        let result = handle_route_specctra_dsn(&args, std::sync::Arc::new(ctx()))
            .await
            .unwrap();
        assert!(!result.is_error, "{:?}", result.content);
        body(&result)
    }

    /// With no net that has a recorded width, the result is what it was
    /// before row S9, plus three empty fields: no manifest, a manifest from
    /// before intent widths, an empty list, and a pour net that kept its
    /// class width all read the same.
    #[tokio::test]
    async fn with_no_recorded_width_the_result_only_gains_empty_width_fields() {
        let dir = tempfile::tempdir().unwrap();
        let java = routing_java(dir.path());
        let manifests = [
            None,
            Some(json!({ "format_version": 1 })),
            Some(json!({ "intent_widths": [] })),
            Some(json!({ "intent_widths": [
                { "net": "GND", "current_a": 2.0, "from_mm": 0.2, "to_mm": 0.2, "status": "not_widened" }
            ] })),
        ];
        for (index, manifest) in manifests.into_iter().enumerate() {
            let result = route_with(
                dir.path(),
                &java,
                &format!("plain{index}"),
                "(pcb board (structure))",
                manifest,
            )
            .await;
            let mut keys: Vec<&str> = result
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .filter(|key| *key != "success")
                .collect();
            keys.sort();
            let mut expected: Vec<&str> = KEYS_BEFORE_S9.to_vec();
            expected.extend([
                "intent_widths",
                "width_exceptions",
                "width_exceptions_total",
            ]);
            expected.sort();
            assert_eq!(keys, expected, "manifest {index}");
            assert_eq!(result["success"], true);
            assert_eq!(result["intent_widths"], json!([]), "manifest {index}");
            assert_eq!(result["width_exceptions"], json!([]), "manifest {index}");
            assert_eq!(result["width_exceptions_total"], 0, "manifest {index}");
            // The session the fake writes holds no routes: it was not read.
            assert_eq!(result["status"], "complete", "manifest {index}: {result}");
            assert_eq!(
                result["completion"]["reason"],
                "Freerouting reports 0 unrouted connections and was given no pour as a plane."
            );
        }
    }

    /// Row LT (L14): Freerouting routes a board whose design intent declares a length
    /// group or a pair skew as it always did, and says it did not tune or check
    /// lengths. Without either, the result has no such field.
    #[tokio::test]
    async fn freerouting_says_it_does_not_tune_lengths_when_the_intent_declares_them() {
        let dir = tempfile::tempdir().unwrap();
        let java = routing_java(dir.path());
        let board = dir.path().join("b.kicad_pcb");
        std::fs::write(&board, "(kicad_pcb)").unwrap();
        let manifest = json!({ "board_path": board.display().to_string() });
        let write_intent = |interface: serde_json::Value| {
            std::fs::create_dir_all(dir.path().join(".konnect")).unwrap();
            std::fs::write(
                dir.path().join(".konnect/design_intent.json"),
                json!({ "schema_version": 1, "rails": [], "currents": [],
                        "interfaces": [interface] })
                .to_string(),
            )
            .unwrap();
        };
        // No intent: no note.
        let result = route_with(
            dir.path(),
            &java,
            "none",
            "(pcb board (structure))",
            Some(manifest.clone()),
        )
        .await;
        assert!(result.get("length_matching_note").is_none(), "{result}");
        // An intent with a pair and no length declaration: no note either.
        write_intent(json!({ "name": "bus", "kind": "bus", "nets": ["A", "B"],
            "differential_pairs": [{ "positive": "A", "negative": "B" }] }));
        let result = route_with(
            dir.path(),
            &java,
            "plain",
            "(pcb board (structure))",
            Some(manifest.clone()),
        )
        .await;
        assert!(result.get("length_matching_note").is_none(), "{result}");
        // A length group: the note, and the result is otherwise the same.
        write_intent(json!({ "name": "bus", "kind": "bus", "nets": ["A", "B"],
            "length_groups": [{ "name": "g", "nets": ["A", "B"], "tolerance_mm": 0.5 }] }));
        let grouped = route_with(
            dir.path(),
            &java,
            "group",
            "(pcb board (structure))",
            Some(manifest.clone()),
        )
        .await;
        let note = grouped["length_matching_note"].as_str().expect("a note");
        assert!(
            note.contains("Freerouting does not tune lengths") && note.contains("fastroute"),
            "{note}"
        );
        assert_eq!(grouped["success"], true);
        assert!(grouped.get("length_matching").is_none(), "{grouped}");
        // A declared pair skew alone also earns the note.
        write_intent(json!({ "name": "bus", "kind": "bus", "nets": ["A", "B"],
            "differential_pairs": [{ "positive": "A", "negative": "B", "max_skew_mm": 0.2 }] }));
        let skewed = route_with(
            dir.path(),
            &java,
            "skew",
            "(pcb board (structure))",
            Some(manifest),
        )
        .await;
        assert!(skewed["length_matching_note"].is_string(), "{skewed}");
    }

    /// Track below a widened net's intent width is listed, and the status
    /// says so.
    #[tokio::test]
    async fn a_route_that_narrowed_a_widened_net_names_the_stretch() {
        let dir = tempfile::tempdir().unwrap();
        let java = routing_java_writing(dir.path(), "narrow", NARROW_SESSION);
        let result = route_with(
            dir.path(),
            &java,
            "narrowed",
            "(pcb board (structure))",
            Some(widened_manifest()),
        )
        .await;
        assert_eq!(result["success"], true);
        assert_eq!(
            result["status"], "complete_with_width_exceptions",
            "{result}"
        );
        assert_eq!(result["width_exceptions_total"], 1);
        assert_eq!(
            result["width_exceptions"],
            json!([{ "net": "VBUS", "intent_width_mm": 1.0, "width_mm": 0.2, "length_mm": 2.5,
                     "layers": ["F.Cu"], "ends_mm": [[3.0, 0.0], [3.0, -2.5]] }])
        );
        let row = &result["intent_widths"][0];
        assert_eq!(row["net"], "VBUS");
        assert_eq!(row["routed_mm"], 5.5);
        assert_eq!(row["below_intent_mm"], 2.5);
        assert!(result["completion"]["reason"]
            .as_str()
            .unwrap()
            .contains("1 stretch(es) of track"));
        assert!(result["width_report_error"].is_null());

        // With a plane the stretch is still listed; the status stays unverified.
        let with_plane = route_with(
            dir.path(),
            &java,
            "narrowed_plane",
            "(pcb board (structure (plane GND (polygon B.Cu 0 0 0 1000 0 1000 1000))))",
            Some(widened_manifest()),
        )
        .await;
        assert_eq!(with_plane["status"], "unverified", "{with_plane}");
        assert_eq!(with_plane["width_exceptions_total"], 1);

        // The same session at full width: plainly complete.
        let wide = routing_java_writing(
            dir.path(),
            "wide",
            &NARROW_SESSION.replace("F.Cu 2000", "F.Cu 10000"),
        );
        let result = route_with(
            dir.path(),
            &wide,
            "held",
            "(pcb board (structure))",
            Some(widened_manifest()),
        )
        .await;
        assert_eq!(result["status"], "complete", "{result}");
        assert_eq!(result["width_exceptions"], json!([]));
        assert_eq!(result["intent_widths"][0]["below_intent_mm"], 0.0);
    }

    /// A session that cannot be read for widths, on a board with a recorded
    /// width: the routing stands, the status is not `complete`, and the
    /// result says why.
    #[tokio::test]
    async fn a_session_unreadable_for_widths_is_reported_and_not_complete() {
        let dir = tempfile::tempdir().unwrap();
        let java = routing_java(dir.path());
        let result = route_with(
            dir.path(),
            &java,
            "unread",
            "(pcb board (structure))",
            Some(widened_manifest()),
        )
        .await;
        assert_eq!(result["success"], true);
        assert_eq!(result["status"], "unverified", "{result}");
        assert!(result["width_report_error"]
            .as_str()
            .unwrap()
            .contains("no routes"));
        assert_eq!(result["width_exceptions"], json!([]));
        assert_eq!(result["width_exceptions_total"], 0);
    }

    // ─── S9, step B: `width_policy` through the handler ────────────────────

    /// One job of a staged fake Freerouting: the settings it reports in
    /// effect, the unrouted counts its log gives, the session it writes, and
    /// how long it stays running.
    struct StagedJob {
        retry_um: f64,
        optimizer: bool,
        start: u32,
        end: u32,
        session: String,
        run_seconds: u32,
    }

    /// A `java` that plays Freerouting 2.5.0's MCP server over several jobs,
    /// one process per job as `route_local` starts them: the job number is
    /// kept in a file, and each job answers from its own files.
    fn staged_java(dir: &Path, name: &str, jobs: &[StagedJob]) -> PathBuf {
        for (index, job) in jobs.iter().enumerate() {
            let at = |kind: &str| dir.join(format!("{name}.job{}.{kind}", index + 1));
            let settings = json!({ "status": 200, "body": {
                "automatic_neckdown": false, "neck_width_um": job.retry_um,
                "autorouter": { "max_passes": 20 },
                "optimizer": { "enabled": job.optimizer, "improvement_threshold": 0.25,
                               "max_consecutive_failures": 1000000, "max_consecutive_failures_pass1": 1000000 },
                "scoring": { "via_costs": 10, "default_undesired_direction_trace_cost": 1.0 },
                "fanout": { "enabled": false } } });
            let logs = json!({ "status": 200, "body": [
                { "type": "INFO", "message": format!("Auto-routing stage started on board 'b' with baseline score 500.00 for {} unrouted items.", job.start) },
                { "type": "INFO", "message": format!("Auto-routing stage completed: started with {} unrouted nets, completed in 0.10 seconds, final score: 900.00 ({} unrouted and 0 violations)", job.start, job.end) },
            ] });
            std::fs::write(at("settings"), settings.to_string()).unwrap();
            std::fs::write(at("logs"), logs.to_string()).unwrap();
            std::fs::write(at("session"), &job.session).unwrap();
            std::fs::write(at("sleep"), job.run_seconds.to_string()).unwrap();
        }
        let tools = [
            "create_session", "enqueue_job", "upload_job_input_from_local_file", "update_job_settings",
            "get_effective_settings", "start_job", "get_job_details", "get_job_logs",
            "download_job_output_to_local_file",
        ]
        .map(|tool| {
            let schema = match tool {
                "create_session" => r#"{"type":"object","properties":{}}"#,
                "enqueue_job" => r#"{"type":"object","properties":{"body":{"type":"object","properties":{"session_id":{"type":"string"}},"required":["session_id"]}},"required":["body"]}"#,
                "upload_job_input_from_local_file" | "download_job_output_to_local_file" => r#"{"type":"object","properties":{"jobId":{"type":"string"},"filePath":{"type":"string"}},"required":["jobId","filePath"]}"#,
                _ => r#"{"type":"object","properties":{"path":{"type":"object","properties":{"jobId":{"type":"string"}},"required":["jobId"]}},"required":["path"]}"#,
            };
            format!(r#"{{"name":"{tool}","inputSchema":{schema}}}"#)
        })
        .join(",");
        let body = r#"#!/bin/sh
if [ "$1" = '-version' ]; then echo 'openjdk version "25.0.1" 2026-01-01' >&2; exit 0; fi
base='@BASE@'
reply() { printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$1" "$2"; }
job() { cat "$base.count" 2>/dev/null || echo 0; }
while IFS= read -r line; do
  id=$(printf '%s' "$line" | grep -o '"id":[0-9]*' | head -n 1 | cut -d: -f2)
  [ -z "$id" ] && continue
  case "$line" in
    *'"method":"initialize"'*) reply "$id" '{"protocolVersion":"2024-11-05"}' ;;
    *'"method":"tools/list"'*) reply "$id" '{"tools":[@TOOLS@]}' ;;
    *'"name":"create_session"'*) reply "$id" '{"session_id":"s1"}' ;;
    *'"name":"enqueue_job"'*)
      n=$(( $(job) + 1 )); echo "$n" > "$base.count"
      reply "$id" "{\"job_id\":\"j$n\"}" ;;
    *'"name":"upload_job_input_from_local_file"'*)
      in=$(printf '%s' "$line" | grep -o '"filePath":"[^"]*"' | cut -d'"' -f4)
      cp "$in" "$base.job$(job).input"
      reply "$id" '{}' ;;
    *'"name":"update_job_settings"'*)
      printf '%s\n' "$line" > "$base.job$(job).sent"
      reply "$id" '{}' ;;
    *'"name":"get_effective_settings"'*) reply "$id" "$(cat "$base.job$(job).settings")" ;;
    *'"name":"get_job_details"'*)
      sleep "$(cat "$base.job$(job).sleep")"
      reply "$id" '{"state":"COMPLETED"}' ;;
    *'"name":"get_job_logs"'*) reply "$id" "$(cat "$base.job$(job).logs")" ;;
    *'"name":"download_job_output_to_local_file"'*)
      out=$(printf '%s' "$line" | grep -o '"filePath":"[^"]*"' | cut -d'"' -f4)
      cp "$base.job$(job).session" "$out"
      reply "$id" '{"message":"saved"}' ;;
    *) reply "$id" '{}' ;;
  esac
done
"#
        .replace("@BASE@", &dir.join(name).display().to_string())
        .replace("@TOOLS@", &tools);
        write_script(dir, name, &body, "")
    }

    const HOLD_DSN: &str = "(pcb board\n  (parser\n    (string_quote \")\n  )\n  (resolution um 10)\n  (unit um)\n  (structure\n    (layer F.Cu\n      (type signal)\n    )\n  )\n  (library\n    (padstack via0\n      (shape (circle F.Cu 600))\n    )\n  )\n  (network\n    (net VBUS\n      (pins U1-1 J1-1)\n    )\n    (net SIG\n      (pins U1-2 J1-2)\n    )\n  )\n  (wiring)\n)\n";

    /// A manifest with VBUS widened to 1.0 mm over a 0.2 mm floor, the same
    /// floor as the other net's class: the one retry width is 200 um.
    fn hold_manifest() -> serde_json::Value {
        json!({
            "layers": [{ "dsn_name": "F.Cu", "kicad_name": "F.Cu" }],
            "classes": [
                { "name": "power", "track_width_um": 1000 },
                { "name": "signal", "track_width_um": 200 }
            ],
            "nets": [
                { "name": "VBUS", "class_name": "power", "min_track_width_um": 200 },
                { "name": "SIG", "class_name": "signal" }
            ],
            "intent_widths": [
                { "net": "VBUS", "current_a": 2.0, "from_mm": 0.2, "to_mm": 1.0, "status": "widened" }
            ]
        })
    }

    fn hold_session(vbus: Option<u32>, sig_y: u32) -> String {
        let vbus = vbus.map_or(String::new(), |width| {
            format!("(net VBUS (wire (path F.Cu {width} 0 0 90000 0)))")
        });
        format!("(session board (routes (resolution um 10) (network_out {vbus} (net SIG (wire (path F.Cu 2000 0 {sig_y} 90000 {sig_y}))))))")
    }

    fn staged(retry_um: f64, optimizer: bool, start: u32, end: u32, session: String) -> StagedJob {
        StagedJob {
            retry_um,
            optimizer,
            start,
            end,
            session,
            run_seconds: 0,
        }
    }

    /// Call the handler with `extra` arguments on `HOLD_DSN` and a manifest.
    async fn route_hold(
        dir: &Path,
        java: &Path,
        name: &str,
        manifest: serde_json::Value,
        extra: serde_json::Value,
    ) -> anyhow::Result<CallToolResult> {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let jar = dir.join("freerouting-2.5.0.jar");
        if !jar.exists() {
            fake_jar(dir, "freerouting-2.5.0.jar", 69);
        }
        let dsn = dir.join(format!("{name}.dsn"));
        std::fs::write(&dsn, HOLD_DSN).unwrap();
        std::fs::write(
            dir.join(format!("{name}.dsn.konnect.json")),
            manifest.to_string(),
        )
        .unwrap();
        let mut args = json!({ "dsn_path": dsn, "ses_output_path": dir.join(format!("{name}.ses")),
                               "engine": "freerouting", "jar_path": jar, "java_path": java, "overall_timeout_seconds": 60 });
        for (key, value) in extra.as_object().unwrap() {
            args[key] = value.clone();
        }
        handle_route_specctra_dsn(&args, std::sync::Arc::new(ctx())).await
    }

    fn sent_settings(dir: &Path, name: &str, job: usize) -> String {
        std::fs::read_to_string(dir.join(format!("{name}.job{job}.sent"))).unwrap()
    }

    /// The whole list through the handler: Full leaves one connection, the
    /// Fallback closes it narrow, the Polish keeps it complete.
    #[tokio::test]
    async fn hold_runs_full_then_fallback_then_polish_and_names_what_ended_narrow() {
        let dir = tempfile::tempdir().unwrap();
        let java = staged_java(
            dir.path(),
            "three",
            &[
                staged(0.0, true, 2, 1, hold_session(None, 50000)),
                staged(200.0, false, 1, 0, hold_session(Some(2000), 50000)),
                staged(0.0, true, 0, 0, hold_session(Some(2000), 40000)),
            ],
        );
        let result = route_hold(
            dir.path(),
            &java,
            "held",
            hold_manifest(),
            json!({ "width_policy": "hold", "width_fallback": "floor" }),
        )
        .await
        .unwrap();
        assert!(!result.is_error, "{:?}", result.content);
        let result = body(&result);
        assert_eq!(result["success"], true);
        assert_eq!(
            result["width_policy"],
            json!({ "requested": "hold", "applied": "hold", "fallback": "floor" })
        );
        assert_eq!(result["result_pass"], "polish");
        let passes = result["passes"].as_array().unwrap();
        let names: Vec<_> = passes
            .iter()
            .map(|pass| {
                (
                    pass["name"].as_str().unwrap(),
                    pass["outcome"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            names,
            [("full", "kept"), ("fallback", "kept"), ("polish", "kept")]
        );
        // What each job was sent: the retry never with the optimizer.
        let sent = |job| sent_settings(dir.path(), "three", job);
        assert!(
            sent(1).contains(r#""neck_width_um":0.0"#) && sent(1).contains(r#""enabled":true"#),
            "{}",
            sent(1)
        );
        assert!(
            sent(2).contains(r#""neck_width_um":200.0"#)
                && sent(2).contains(r#""optimizer":{"enabled":false"#),
            "{}",
            sent(2)
        );
        assert!(
            sent(3).contains(r#""neck_width_um":0.0"#)
                && sent(3).contains(r#""optimizer":{"enabled":true"#),
            "{}",
            sent(3)
        );
        // What each job routed: the export, then the pass before it as wiring.
        let input = |job: usize| {
            std::fs::read_to_string(dir.path().join(format!("three.job{job}.input"))).unwrap()
        };
        assert_eq!(input(1), HOLD_DSN);
        assert!(
            input(2).contains("(path F.Cu 200 0 5000 9000 5000)")
                && !input(2).contains("(net VBUS)\n    )\n  )")
        );
        assert!(
            input(3).contains("(path F.Cu 200 0 0 9000 0)"),
            "{}",
            input(3)
        );
        // The result is the polish pass's session, and its counts.
        let ses = std::fs::read_to_string(dir.path().join("held.ses")).unwrap();
        assert_eq!(ses, hold_session(Some(2000), 40000));
        assert_eq!(result["completion"]["router_unrouted"], 0);
        assert_eq!(result["job_id"], "j3");
        assert_eq!(
            result["status"], "complete_with_width_exceptions",
            "{result}"
        );
        assert_eq!(result["width_exceptions_total"], 1);
        assert_eq!(result["width_exceptions"][0]["net"], "VBUS");
        assert_eq!(result["width_exceptions"][0]["width_mm"], 0.2);
        // The passes' own files stay beside the output.
        let folder = dir.path().join("held.passes");
        assert_eq!(result["artifacts"]["passes_folder"], json!(folder));
        for file in [
            "full.ses",
            "fallback.dsn",
            "fallback.ses",
            "polish.dsn",
            "polish.ses",
        ] {
            assert!(folder.join(file).is_file(), "{file}");
        }
        assert_eq!(passes[1]["seeded"]["wires"], 1);
        assert_eq!(passes[2]["seeded"]["wires"], 2);
        // `elapsed_seconds` is the whole call, not the last job.
        let jobs: f64 = passes
            .iter()
            .map(|pass| pass["elapsed_seconds"].as_f64().unwrap())
            .sum();
        assert!(passes
            .iter()
            .all(|pass| pass["elapsed_seconds"].as_f64().unwrap() > 0.0));
        assert!(
            result["elapsed_seconds"].as_f64().unwrap() >= jobs,
            "{result}"
        );
    }

    /// With the retry off the Full pass completes: one job, at full width.
    #[tokio::test]
    async fn hold_with_a_complete_full_pass_is_one_job_at_full_width() {
        let dir = tempfile::tempdir().unwrap();
        let java = staged_java(
            dir.path(),
            "one",
            &[staged(0.0, true, 2, 0, hold_session(Some(10000), 50000))],
        );
        let result = body(
            &route_hold(
                dir.path(),
                &java,
                "held",
                hold_manifest(),
                json!({ "width_policy": "hold" }),
            )
            .await
            .unwrap(),
        );
        assert_eq!(result["status"], "complete", "{result}");
        assert_eq!(result["result_pass"], "full");
        assert_eq!(result["passes"].as_array().unwrap().len(), 1);
        assert_eq!(result["width_exceptions_total"], 0);
        assert_eq!(result["effective_settings"]["neck_width_um"], 0.0);
        assert!(!dir.path().join("one.job2.sent").exists(), "one job only");
        // `rules_applied` still gives the one floor width a fallback would use.
        assert_eq!(result["rules_applied"]["retry_width_um"], 200);
    }

    /// The default is unchanged in this PR: one job with today's settings
    /// (the retry at the floor, the optimizer on), and no pass fields.
    #[tokio::test]
    async fn without_width_policy_the_call_is_todays_single_job() {
        let dir = tempfile::tempdir().unwrap();
        for (name, extra) in [
            ("default", json!({})),
            ("retry", json!({ "width_policy": "retry" })),
        ] {
            let java = staged_java(
                dir.path(),
                name,
                &[staged(200.0, true, 2, 0, hold_session(Some(2000), 50000))],
            );
            let result = body(
                &route_hold(dir.path(), &java, name, hold_manifest(), extra)
                    .await
                    .unwrap(),
            );
            let sent = sent_settings(dir.path(), name, 1);
            assert!(
                sent.contains(r#""neck_width_um":200.0"#)
                    && sent.contains(r#""optimizer":{"enabled":true"#),
                "{sent}"
            );
            let input =
                std::fs::read_to_string(dir.path().join(format!("{name}.job1.input"))).unwrap();
            assert_eq!(input, HOLD_DSN);
            assert!(
                result["passes"].is_null() && result["result_pass"].is_null(),
                "{result}"
            );
            assert!(result["width_policy"].is_null(), "{result}");
            assert!(result["artifacts"]["passes_folder"].is_null());
            assert!(!dir.path().join(format!("{name}.passes")).exists());
            // The always-on report still names what the retry narrowed.
            assert_eq!(result["status"], "complete_with_width_exceptions");
        }
    }

    /// `hold` on a board with no recorded width has nothing to hold: today's
    /// single job runs, and the result says so.
    #[tokio::test]
    async fn hold_with_no_recorded_width_runs_todays_single_job_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut manifest = hold_manifest();
        manifest["intent_widths"] = json!([]);
        manifest["nets"][0] = json!({ "name": "VBUS", "class_name": "signal" });
        let java = staged_java(
            dir.path(),
            "plain",
            &[staged(200.0, true, 2, 0, hold_session(Some(2000), 50000))],
        );
        let result = body(
            &route_hold(
                dir.path(),
                &java,
                "plain",
                manifest,
                json!({ "width_policy": "hold", "width_fallback": "none" }),
            )
            .await
            .unwrap(),
        );
        assert_eq!(result["width_policy"]["requested"], "hold");
        assert_eq!(result["width_policy"]["applied"], "retry");
        assert!(result["width_policy"]["reason"]
            .as_str()
            .unwrap()
            .contains("no net with a recorded width"));
        assert!(result["passes"].is_null());
        assert!(sent_settings(dir.path(), "plain", 1).contains(r#""neck_width_um":200.0"#));
        assert_eq!(result["status"], "complete");
    }

    #[tokio::test]
    async fn width_fallback_none_leaves_open_connections_open() {
        let dir = tempfile::tempdir().unwrap();
        let java = staged_java(
            dir.path(),
            "none",
            &[staged(0.0, true, 2, 1, hold_session(None, 50000))],
        );
        let result = body(
            &route_hold(
                dir.path(),
                &java,
                "none",
                hold_manifest(),
                json!({ "width_policy": "hold", "width_fallback": "none" }),
            )
            .await
            .unwrap(),
        );
        assert_eq!(result["status"], "incomplete", "{result}");
        assert_eq!(result["width_policy"]["fallback"], "none");
        assert_eq!(result["passes"][1]["outcome"], "skipped");
        assert_eq!(result["completion"]["router_unrouted"], 1);
        assert!(!dir.path().join("none.job2.sent").exists());
    }

    /// A call that runs out of time during a later pass returns the session
    /// of the pass before it, as `stopped`.
    #[tokio::test]
    async fn hold_that_runs_out_of_time_returns_the_earlier_session_as_stopped() {
        let dir = tempfile::tempdir().unwrap();
        // The call has ten seconds, the least the schema allows. The Full
        // pass leaves less than the ten a pass is worth starting with, so the
        // Fallback, which would have completed, never starts.
        let java = staged_java(
            dir.path(),
            "slow",
            &[
                staged(0.0, true, 2, 1, hold_session(None, 50000)),
                staged(200.0, false, 1, 0, hold_session(Some(2000), 50000)),
            ],
        );
        let result = body(
            &route_hold(
                dir.path(),
                &java,
                "slow",
                hold_manifest(),
                json!({ "width_policy": "hold", "overall_timeout_seconds": 10 }),
            )
            .await
            .unwrap(),
        );
        assert_eq!(result["status"], "stopped", "{result}");
        assert_eq!(result["result_pass"], "full");
        assert_eq!(result["passes"][1]["outcome"], "stopped");
        assert!(result["passes"][1]["reason"]
            .as_str()
            .unwrap()
            .contains("before this pass could start"));
        assert!(!dir.path().join("slow.job2.sent").exists());
        assert!(result["completion"]["reason"]
            .as_str()
            .unwrap()
            .contains("ran out at the fallback pass"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("slow.ses")).unwrap(),
            hold_session(None, 50000)
        );
    }

    /// One job that outlasts the call fails with the typed timeout, also
    /// under the stderr context: that is how the pass engine tells "the time
    /// ran out" from "the job failed".
    #[tokio::test]
    async fn a_job_that_outlasts_the_call_fails_with_the_typed_timeout() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let dir = tempfile::tempdir().unwrap();
        let jar = fake_jar(dir.path(), "freerouting-2.5.0.jar", 69);
        let mut slow = staged(200.0, true, 2, 0, hold_session(None, 50000));
        slow.run_seconds = 5;
        let java = staged_java(dir.path(), "late", &[slow]);
        let dsn = dir.path().join("late.dsn");
        std::fs::write(&dsn, HOLD_DSN).unwrap();
        let mut settings =
            route_settings(&json!({}), &crate::specctra::RoutingHints::default(), java).unwrap();
        settings.overall_timeout = std::time::Duration::from_millis(500);
        let ses = dir.path().join("late.ses");
        let error = crate::freerouting_mcp::route_local(&jar, &dsn, &ses, &settings)
            .await
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::freerouting_mcp::OverallTimeout>()
                .is_some(),
            "{error:#}"
        );
        assert!(
            format!("{error:#}").contains("overall timeout"),
            "{error:#}"
        );
        assert!(!ses.exists());
    }

    #[tokio::test]
    async fn bad_width_arguments_and_an_existing_passes_folder_are_refused_before_routing() {
        let dir = tempfile::tempdir().unwrap();
        let java = staged_java(dir.path(), "never", &[]);
        for (extra, field) in [
            (json!({ "width_policy": "wide" }), "width_policy"),
            (json!({ "width_policy": 3 }), "width_policy"),
            (
                json!({ "width_policy": "hold", "width_fallback": "retry" }),
                "width_fallback",
            ),
            (json!({ "width_fallback": true }), "width_fallback"),
        ] {
            let result = route_hold(dir.path(), &java, "bad", hold_manifest(), extra)
                .await
                .unwrap();
            assert_eq!(
                crate::mcp::error::extract_error_kind(&result).as_deref(),
                Some("invalid_argument")
            );
            let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
                panic!("expected text")
            };
            assert!(text.contains(field), "{text}");
        }
        std::fs::create_dir(dir.path().join("taken.passes")).unwrap();
        let error = route_hold(
            dir.path(),
            &java,
            "taken",
            hold_manifest(),
            json!({ "width_policy": "hold" }),
        )
        .await
        .unwrap_err();
        assert!(
            format!("{error:#}").contains("passes folder already exists"),
            "{error:#}"
        );
        std::fs::write(dir.path().join("there.ses"), "x").unwrap();
        let error = route_hold(
            dir.path(),
            &java,
            "there",
            hold_manifest(),
            json!({ "width_policy": "hold" }),
        )
        .await
        .unwrap_err();
        assert!(
            format!("{error:#}").contains("SES output already exists"),
            "{error:#}"
        );
        assert!(!dir.path().join("there.passes").exists());
        assert!(
            !dir.path().join("never.count").exists(),
            "nothing was routed"
        );
    }

    /// Review A, S1, through the handler: a wire the report cannot measure is
    /// named in the result, and the run is not complete.
    #[tokio::test]
    async fn a_wire_that_cannot_be_measured_is_named_and_the_run_is_not_complete() {
        let dir = tempfile::tempdir().unwrap();
        let odd = "(session board (routes (resolution um 10) (network_out (net VBUS (wire (path F.Cu 10000 0 0 30000 0)) (wire (polygon F.Cu 0 0 0 10 10 0 10))))))";
        let java = routing_java_writing(dir.path(), "java-odd", odd);
        let result = route_with(
            dir.path(),
            &java,
            "odd",
            "(pcb board (structure))",
            Some(widened_manifest()),
        )
        .await;
        assert_eq!(result["success"], true);
        assert_eq!(result["status"], "unverified", "{result}");
        assert_eq!(result["width_unmeasured_total"], 1);
        assert_eq!(result["width_unmeasured"][0]["net"], "VBUS");
        assert_eq!(result["width_unmeasured"][0]["form"], "polygon");
        assert_eq!(result["intent_widths"][0]["unmeasured_wires"], 1);
        assert_eq!(result["intent_widths"][0]["routed_mm"], 3.0);
        assert!(result["width_report_error"].is_null());
        assert!(result["completion"]["reason"]
            .as_str()
            .unwrap()
            .contains("could not be measured"));
        // A session that is measured whole carries neither field.
        let fine = "(session board (routes (resolution um 10) (network_out (net VBUS (wire (path F.Cu 10000 0 0 30000 0))))))";
        let java = routing_java_writing(dir.path(), "java-fine", fine);
        let result = route_with(
            dir.path(),
            &java,
            "fine",
            "(pcb board (structure))",
            Some(widened_manifest()),
        )
        .await;
        assert_eq!(result["status"], "complete", "{result}");
        let object = result.as_object().unwrap();
        assert!(!object.contains_key("width_unmeasured"));
        assert!(!object.contains_key("width_unmeasured_total"));
        assert_eq!(result["intent_widths"][0]["unmeasured_wires"], 0);
    }

    /// Review A, S2, through the handler: a Fallback that closes nothing is
    /// not the result.
    #[tokio::test]
    async fn a_fallback_that_closes_nothing_leaves_the_full_passs_session() {
        let dir = tempfile::tempdir().unwrap();
        let java = staged_java(
            dir.path(),
            "same",
            &[
                staged(0.0, true, 2, 1, hold_session(None, 50000)),
                // VBUS laid narrow, and still one connection open.
                staged(200.0, false, 1, 1, hold_session(Some(2000), 50000)),
            ],
        );
        let result = body(
            &route_hold(
                dir.path(),
                &java,
                "same",
                hold_manifest(),
                json!({ "width_policy": "hold" }),
            )
            .await
            .unwrap(),
        );
        assert_eq!(result["result_pass"], "full", "{result}");
        let passes = result["passes"].as_array().unwrap();
        assert_eq!(passes.len(), 2, "no polish row");
        assert_eq!(passes[1]["outcome"], "discarded");
        assert!(passes[1]["reason"]
            .as_str()
            .unwrap()
            .contains("closed no connection"));
        assert_eq!(result["status"], "incomplete");
        assert_eq!(result["completion"]["router_unrouted"], 1);
        assert_eq!(result["width_exceptions_total"], 0);
        assert_eq!(result["job_id"], "j1");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("same.ses")).unwrap(),
            hold_session(None, 50000)
        );
    }

    /// Review A, S3, through the handler: the plane fallback's routing step
    /// repeats the width arguments this call was given.
    #[tokio::test]
    async fn the_plane_fallback_step_repeats_this_calls_width_arguments() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let dir = tempfile::tempdir().unwrap();
        let jar = fake_jar(dir.path(), "freerouting-2.5.0.jar", 69);
        let java = routing_java(dir.path());
        let board = dir.path().join("board.kicad_pcb");
        std::fs::write(&board, "(kicad_pcb)").unwrap();
        let reroute = |name: &str, extra: serde_json::Value| {
            let dsn = dir.path().join(format!("{name}.dsn"));
            std::fs::write(
                &dsn,
                "(pcb board (structure (plane GND (polygon B.Cu 0 0 0 1000 0 1000 1000))))",
            )
            .unwrap();
            std::fs::write(
                dir.path().join(format!("{name}.dsn.konnect.json")),
                json!({ "board_path": board }).to_string(),
            )
            .unwrap();
            let mut args = json!({ "dsn_path": dsn, "ses_output_path": dir.path().join(format!("{name}.ses")),
                                   "engine": "freerouting", "jar_path": jar, "java_path": java, "overall_timeout_seconds": 60 });
            for (key, value) in extra.as_object().unwrap() {
                args[key] = value.clone();
            }
            async move {
                let result = handle_route_specctra_dsn(&args, std::sync::Arc::new(ctx()))
                    .await
                    .unwrap();
                assert!(!result.is_error, "{:?}", result.content);
                let result = body(&result);
                let calls: Vec<serde_json::Value> = result["completion"]["if_still_open"]
                    ["fallback"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|call| call["tool"] == "route_specctra_dsn")
                    .map(|call| call["arguments"].clone())
                    .collect();
                assert_eq!(calls.len(), 1, "{result}");
                calls[0].clone()
            }
        };
        let held = reroute(
            "held",
            json!({ "width_policy": "hold", "width_fallback": "none" }),
        )
        .await;
        assert_eq!(held["width_policy"], "hold", "{held}");
        assert_eq!(held["width_fallback"], "none", "{held}");
        // The step routes with the engine and the JAR and Java of this call.
        assert_eq!(held["engine"], "freerouting", "{held}");
        assert!(
            held["jar_path"]
                .as_str()
                .unwrap()
                .ends_with("freerouting-2.5.0.jar"),
            "{held}"
        );
        assert!(held["java_path"].is_string(), "{held}");
        let plain = reroute("plain", json!({})).await;
        assert!(plain["width_policy"].is_null() && plain["width_fallback"].is_null());
        let retry = reroute("retry", json!({ "width_policy": "retry" })).await;
        assert_eq!(retry["width_policy"], "retry");
        assert!(retry["width_fallback"].is_null());
    }

    /// Review A, S5: a failed `hold` call leaves nothing that blocks the same
    /// call. What it wrote is moved aside, beside the output; an empty folder
    /// is removed.
    #[tokio::test]
    async fn a_failed_hold_call_does_not_block_the_same_call() {
        let dir = tempfile::tempdir().unwrap();
        let hold = || json!({ "width_policy": "hold" });
        let folder = dir.path().join("again.passes");
        let aside = |n: u32| dir.path().join(format!("again.passes.failed-{n}"));
        // The Full job is refused before it starts: Freerouting reports a
        // retry width in effect that was not sent. Nothing was written.
        let refused = staged_java(
            dir.path(),
            "refused",
            &[staged(200.0, true, 2, 0, hold_session(Some(10000), 50000))],
        );
        let error = route_hold(dir.path(), &refused, "again", hold_manifest(), hold())
            .await
            .unwrap_err();
        assert!(!folder.exists(), "{error:#}");
        assert!(!aside(1).exists(), "{error:#}");
        assert!(!dir.path().join("again.ses").exists());
        // A Freerouting that breaks with words on its error stream: the log
        // of the failed call is kept, out of the way, twice over.
        let broken = staged_java(dir.path(), "broken", &[]);
        for n in 1..=2 {
            let error = route_hold(dir.path(), &broken, "again", hold_manifest(), hold())
                .await
                .unwrap_err();
            assert!(!folder.exists(), "{error:#}");
            assert!(
                aside(n).join("full.freerouting.log").is_file(),
                "attempt {n}: {error:#}"
            );
            assert!(
                format!("{error:#}").contains(&format!("again.passes.failed-{n}")),
                "{error:#}"
            );
            assert!(!dir.path().join("again.ses").exists());
        }
        // And then the same call, with a Freerouting that works.
        let good = staged_java(
            dir.path(),
            "good",
            &[staged(0.0, true, 2, 0, hold_session(Some(10000), 50000))],
        );
        let result = body(
            &route_hold(dir.path(), &good, "again", hold_manifest(), hold())
                .await
                .unwrap(),
        );
        assert_eq!(result["status"], "complete", "{result}");
        assert!(folder.join("full.ses").is_file());
        assert!(dir.path().join("again.ses").is_file());
        assert!(aside(1).is_dir() && aside(2).is_dir() && !aside(3).exists());
    }

    /// The output may sit in a folder that is not there yet: a `hold` call
    /// makes it, as the single run does.
    #[tokio::test]
    async fn a_hold_call_makes_the_folder_of_its_output() {
        let dir = tempfile::tempdir().unwrap();
        let java = staged_java(
            dir.path(),
            "deep",
            &[staged(0.0, true, 2, 0, hold_session(Some(10000), 50000))],
        );
        let output = dir.path().join("not/there/yet/deep.ses");
        let result = body(
            &route_hold(
                dir.path(),
                &java,
                "deep",
                hold_manifest(),
                json!({ "width_policy": "hold", "ses_output_path": output }),
            )
            .await
            .unwrap(),
        );
        assert_eq!(result["status"], "complete", "{result}");
        assert!(output.is_file());
        assert!(dir
            .path()
            .join("not/there/yet/deep.passes/full.ses")
            .is_file());
    }

    #[test]
    fn a_failed_calls_passes_folder_is_set_aside_or_removed() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("b.passes");
        let aside = |n: u32| dir.path().join(format!("b.passes.failed-{n}"));
        // Empty: removed, and nothing is kept.
        std::fs::create_dir(&folder).unwrap();
        let note = set_aside_failed_passes(&folder);
        assert!(!folder.exists() && !aside(1).exists(), "{note}");
        assert!(note.contains("left no file"), "{note}");
        // With files: moved to the first free name.
        for n in 1..=2 {
            std::fs::create_dir(&folder).unwrap();
            std::fs::write(folder.join("full.ses"), n.to_string()).unwrap();
            let note = set_aside_failed_passes(&folder);
            assert!(!folder.exists(), "{note}");
            assert_eq!(
                std::fs::read_to_string(aside(n).join("full.ses")).unwrap(),
                n.to_string()
            );
            assert!(note.contains(&aside(n).display().to_string()), "{note}");
        }
        // A name that is taken by anything, a file too, is never replaced.
        std::fs::write(aside(3), "mine").unwrap();
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("full.ses"), "4").unwrap();
        let note = set_aside_failed_passes(&folder);
        assert!(note.contains(&aside(4).display().to_string()), "{note}");
        assert_eq!(std::fs::read_to_string(aside(3)).unwrap(), "mine");
        // Every name taken: the folder is removed, and nothing else.
        for n in 5..=99 {
            std::fs::create_dir(aside(n)).unwrap();
        }
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("full.ses"), "100").unwrap();
        let note = set_aside_failed_passes(&folder);
        assert!(!folder.exists(), "{note}");
        assert!(note.contains("removed"), "{note}");
        assert!(!aside(100).exists());
        assert_eq!(
            std::fs::read_to_string(aside(1).join("full.ses")).unwrap(),
            "1"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 99);
        // A folder that is not there: said, not a panic.
        let note = set_aside_failed_passes(&folder);
        assert!(note.contains("left no file"), "{note}");
    }

    /// Review A, N7: `width_fallback` without `hold` is said to be unused.
    #[tokio::test]
    async fn width_fallback_without_hold_is_said_to_be_unused() {
        let dir = tempfile::tempdir().unwrap();
        for (name, extra) in [
            ("alone", json!({ "width_fallback": "none" })),
            (
                "retry",
                json!({ "width_policy": "retry", "width_fallback": "floor" }),
            ),
        ] {
            let java = staged_java(
                dir.path(),
                name,
                &[staged(200.0, true, 2, 0, hold_session(Some(10000), 50000))],
            );
            let result = body(
                &route_hold(dir.path(), &java, name, hold_manifest(), extra)
                    .await
                    .unwrap(),
            );
            assert_eq!(result["width_policy"]["requested"], "retry", "{result}");
            assert_eq!(result["width_policy"]["applied"], "retry");
            assert_eq!(result["width_policy"]["fallback"], "unused");
            assert!(result["width_policy"]["reason"]
                .as_str()
                .unwrap()
                .contains("only with width_policy \"hold\""));
            assert!(result["passes"].is_null());
        }
    }

    /// The handler reads the planes from the DSN it routes and puts them into
    /// `status`: with the same 0 unrouted from the router, a DSN with a plane
    /// is `unverified` and one without is `complete` (review S2).
    #[tokio::test]
    async fn the_routing_result_judges_the_planes_of_the_dsn_it_routed() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let dir = tempfile::tempdir().unwrap();
        let jar = fake_jar(dir.path(), "freerouting-2.5.0.jar", 69);
        let java = routing_java(dir.path());
        let route = |name: &str, dsn_text: &str| {
            let dsn = dir.path().join(format!("{name}.dsn"));
            std::fs::write(&dsn, dsn_text).unwrap();
            let args = json!({ "dsn_path": dsn, "ses_output_path": dir.path().join(format!("{name}.ses")),
                               "engine": "freerouting", "jar_path": jar, "java_path": java, "overall_timeout_seconds": 60 });
            async move {
                let result = handle_route_specctra_dsn(&args, std::sync::Arc::new(ctx()))
                    .await
                    .unwrap();
                assert!(!result.is_error, "{:?}", result.content);
                body(&result)
            }
        };
        let with_plane = route(
            "plane",
            "(pcb board (structure (plane GND (polygon B.Cu 0 0 0 1000 0 1000 1000))))",
        )
        .await;
        assert_eq!(with_plane["success"], true);
        assert_eq!(with_plane["completion"]["router_unrouted"], 0);
        assert_eq!(with_plane["completion"]["planes"][0]["net"], "GND");
        assert_eq!(with_plane["status"], "unverified", "{with_plane}");
        let without = route("plain", "(pcb board (structure))").await;
        assert_eq!(without["completion"]["planes"], json!([]));
        assert_eq!(without["status"], "complete", "{without}");
    }

    /// A `dsn_path` that is not a regular `.dsn` file within the size bound is
    /// refused before anything reads it or looks for Java (review S1).
    #[tokio::test]
    async fn a_dsn_path_that_is_not_a_bounded_regular_file_is_refused_before_it_is_read() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let dir = tempfile::tempdir().unwrap();
        let jar = fake_jar(dir.path(), "freerouting-2.5.0.jar", 69);
        let folder = dir.path().join("folder.dsn");
        std::fs::create_dir(&folder).unwrap();
        let fifo = dir.path().join("pipe.dsn");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        let huge = dir.path().join("huge.dsn");
        std::fs::File::create(&huge)
            .unwrap()
            .set_len(crate::freerouting_mcp::MAX_DSN_BYTES + 1)
            .unwrap();
        for (dsn, needle) in [
            (&folder, "existing .dsn file"),
            (&fifo, "existing .dsn file"),
            (&huge, "bytes"),
        ] {
            let args = json!({ "dsn_path": dsn, "ses_output_path": dir.path().join("out.ses"), "engine": "freerouting", "jar_path": jar });
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(20),
                handle_route_specctra_dsn(&args, std::sync::Arc::new(ctx())),
            )
            .await
            .expect("refused at once, never blocked on the path")
            .unwrap();
            assert_eq!(
                crate::mcp::error::extract_error_kind(&result).as_deref(),
                Some("invalid_argument"),
                "{}: {:?}",
                dsn.display(),
                result.content
            );
            let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
                panic!("expected text");
            };
            assert!(text.contains(needle) && text.contains("dsn_path"), "{text}");
        }
    }

    fn ctx() -> ToolContext {
        ToolContext::new(
            crate::tools::ServerConfig::default(),
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        )
    }

    fn body(result: &CallToolResult) -> serde_json::Value {
        let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
            panic!("expected text");
        };
        serde_json::from_str(text).unwrap_or_else(|_| panic!("not JSON: {text}"))
    }

    /// A `java` that reports `major`, and answers `-jar <jar> --version`.
    fn fake_java(dir: &Path, name: &str, major: u32) -> PathBuf {
        write_script(
            dir,
            name,
            &format!(
                "#!/bin/sh\nif [ \"$1\" = '-version' ]; then echo 'openjdk version \"{major}.0.1\" 2026-01-01' >&2; exit 0; fi\n\
                 echo 'Freerouting 2.1.0'; exit 0\n"
            ),
            "",
        )
    }

    #[tokio::test]
    async fn too_old_a_java_is_reported_with_the_steps_to_fix_it() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let dir = tempfile::tempdir().unwrap();
        let jar = fake_jar(dir.path(), "freerouting-2.1.0.jar", 65);
        let java = fake_java(dir.path(), "java11", 11);
        let result =
            handle_check_freerouting(&json!({ "jar_path": jar, "java_path": java }), &ctx())
                .await
                .unwrap();
        let body = body(&result);
        assert_eq!(body["available"], false, "{body}");
        assert_eq!(body["engine_found"], true);
        assert_eq!(body["java_available"], true);
        assert_eq!(body["java_sufficient"], false);
        assert_eq!(body["required_java_major"], 21);
        assert_eq!(body["setup"]["status"], "needs_java");
        let steps = body["setup"]["steps"].to_string();
        assert!(
            steps.contains("brew install openjdk@21") && steps.contains("found Java 11"),
            "{steps}"
        );
        assert!(
            body["note"].as_str().unwrap().contains("needs Java 21"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn a_java_that_meets_the_jar_is_chosen_over_one_that_does_not() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let dir = tempfile::tempdir().unwrap();
        let jar = fake_jar(dir.path(), "freerouting-2.1.0.jar", 65);
        let java = fake_java(dir.path(), "java21", 21);
        let result =
            handle_check_freerouting(&json!({ "jar_path": jar, "java_path": java }), &ctx())
                .await
                .unwrap();
        let body = body(&result);
        assert_eq!(body["java_sufficient"], true, "{body}");
        assert_eq!(body["java_command"], json!(java), "{body}");
        assert_eq!(body["version_checked"], true, "{body}");
        assert_eq!(body["available"], true, "{body}");
        assert_eq!(body["setup"]["status"], "ready");
        // The fake answers "Freerouting 2.1.0", which is not the recommended
        // release, so the policy says so with the measured reason.
        let policy = &body["version_policy"];
        assert_eq!(policy["installed_version"], "2.1.0", "{body}");
        assert_eq!(
            policy["installed_version_basis"],
            "the JAR's --version output"
        );
        assert_eq!(policy["matches_recommended"], false);
        assert_eq!(
            policy["recommended_version"],
            crate::freerouting_setup::RECOMMENDED.version
        );
        assert!(policy["warning"]
            .as_str()
            .unwrap()
            .contains(crate::freerouting_setup::RECOMMENDED.reason));
        // A JAR for an older Java is happy with the old one.
        let old_jar = fake_jar(dir.path(), "freerouting-1.9.0.jar", 61);
        let java17 = fake_java(dir.path(), "java17", 17);
        let result =
            handle_check_freerouting(&json!({ "jar_path": old_jar, "java_path": java17 }), &ctx())
                .await
                .unwrap();
        let body = body_of(&result);
        assert_eq!(body["required_java_major"], 17, "{body}");
        assert_eq!(body["java_sufficient"], true, "{body}");
    }

    fn body_of(result: &CallToolResult) -> serde_json::Value {
        body(result)
    }

    #[tokio::test]
    async fn a_java_that_cannot_start_the_jar_does_not_make_it_available() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let dir = tempfile::tempdir().unwrap();
        let jar = fake_jar(dir.path(), "freerouting-2.1.0.jar", 65);
        // Reports Java 21, but the JAR itself fails to start.
        let java = write_script(
            dir.path(),
            "java21-broken-jar",
            "#!/bin/sh\nif [ \"$1\" = '-version' ]; then echo 'openjdk version \"21.0.1\"' >&2; exit 0; fi\n\
             echo 'Error: could not open the jar' >&2; exit 1\n",
            "",
        );
        let result =
            handle_check_freerouting(&json!({ "jar_path": jar, "java_path": java }), &ctx())
                .await
                .unwrap();
        let body = body(&result);
        assert_eq!(body["java_sufficient"], true, "{body}");
        assert_eq!(body["version_checked"], false, "{body}");
        assert_eq!(body["available"], false, "{body}");
    }

    #[tokio::test]
    async fn routing_with_a_java_that_is_too_old_is_refused_before_anything_starts() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let dir = tempfile::tempdir().unwrap();
        let jar = fake_jar(dir.path(), "freerouting-2.1.0.jar", 65);
        let java = fake_java(dir.path(), "java11", 11);
        let dsn = dir.path().join("board.dsn");
        std::fs::write(&dsn, "(pcb x)").unwrap();
        let ses = dir.path().join("board.ses");
        let result = handle_route_specctra_dsn(
            &json!({ "dsn_path": dsn, "ses_output_path": ses, "engine": "freerouting", "jar_path": jar, "java_path": java }),
            std::sync::Arc::new(ctx()),
        )
        .await
        .unwrap();
        assert!(result.is_error);
        let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
            panic!("expected text");
        };
        assert!(text.contains("needs Java 21 or newer"), "{text}");
        assert!(text.contains("brew install openjdk@21"), "{text}");
        assert!(text.contains("Nothing was routed"), "{text}");
        assert!(!ses.exists(), "no output was created");
    }

    /// Freerouting 2.1.0 prints its version and exits 1 when the GUI is off
    /// and no board is given; that still proves the JAR starts. The probe runs
    /// headless with telemetry off.
    #[tokio::test]
    async fn a_version_printed_before_a_nonzero_exit_counts_and_the_probe_is_quiet() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let dir = tempfile::tempdir().unwrap();
        let jar = fake_jar(dir.path(), "freerouting-2.1.0.jar", 65);
        let args_log = dir.path().join("args.txt");
        let java = write_script(
            dir.path(),
            "java21-cli",
            &format!(
                "#!/bin/sh\nif [ \"$1\" = '-version' ]; then echo 'openjdk version \"21.0.1\"' >&2; exit 0; fi\n\
                 for a in \"$@\"; do case \"$a\" in --version) echo \"$*\" > '{}';; esac; done\n\
                 case \"$*\" in *--version*) echo '2026-11-08 INFO   Freerouting v2.1.0 (build-date: 2025-04-12)'; \
                 echo 'ERROR  Both an input file and an output file must be specified' >&2; exit 1;; esac\n\
                 exit 1\n",
                args_log.display()
            ),
            "",
        );
        let result =
            handle_check_freerouting(&json!({ "jar_path": jar, "java_path": java }), &ctx())
                .await
                .unwrap();
        let body = body(&result);
        assert_eq!(body["version_checked"], true, "{body}");
        assert_eq!(body["available"], true, "{body}");
        assert_eq!(
            body["version_policy"]["installed_version"], "2.1.0",
            "{body}"
        );
        let args = std::fs::read_to_string(&args_log).unwrap();
        for quiet in [
            "-Djava.awt.headless=true",
            "-Duser.home=",
            "-Djava.io.tmpdir=",
            "--gui.enabled=false",
            "--profile.allow_telemetry=false",
            "--usage_and_diagnostic_data.disable_analytics=true",
        ] {
            assert!(args.contains(quiet), "{quiet} in {args}");
        }
        // 2.5.0's version check goes to a closed loopback port, not GitHub.
        let before_jar = &args[..args.find("-jar").expect("-jar is passed")];
        for offline in FREEROUTING_OFFLINE_JVM_ARGUMENTS {
            assert!(
                before_jar.contains(offline),
                "{offline} before -jar in {args}"
            );
        }
    }

    /// 2.5.0 logs its banner before it reads its settings, so a JAR that
    /// prints it and then dies must not read as available.
    #[tokio::test]
    async fn a_banner_followed_by_a_startup_crash_is_not_available() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let dir = tempfile::tempdir().unwrap();
        let jar = fake_jar(dir.path(), "freerouting-2.5.0.jar", 65);
        let java = write_script(
            dir.path(),
            "java21-crash",
            "#!/bin/sh\nif [ \"$1\" = '-version' ]; then echo 'openjdk version \"21.0.1\"' >&2; exit 0; fi\n\
             case \"$*\" in *--version*) echo '2026-11-09 INFO Freerouting v2.5.0 (build-date: 2026-10-05)'; \
             echo 'Exception in thread \"main\" java.lang.NullPointerException' >&2; \
             printf '\\tat app.freerouting.startup.GlobalSettingsBootstrap.initialize(GlobalSettingsBootstrap.java:1)\\n' >&2; exit 1;; esac\n\
             exit 1\n",
            "",
        );
        let result =
            handle_check_freerouting(&json!({ "jar_path": jar, "java_path": java }), &ctx())
                .await
                .unwrap();
        let body = body(&result);
        assert_eq!(body["version_checked"], false, "{body}");
        assert_eq!(body["available"], false, "{body}");
        assert!(
            body["version_output"]
                .as_str()
                .unwrap()
                .contains("NullPointerException"),
            "{body}"
        );
    }

    /// With no scratch folder the JAR is not started, because the alternative
    /// is to start it in the owner's home.
    #[tokio::test]
    async fn without_a_scratch_folder_the_jar_is_not_started() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        PROBE_SCRATCH_FAILS.with(|fails| fails.set(true));
        let dir = tempfile::tempdir().unwrap();
        let jar = fake_jar(dir.path(), "freerouting-2.1.0.jar", 65);
        let started = dir.path().join("started.txt");
        let java = write_script(
            dir.path(),
            "java21-marker",
            &format!(
                "#!/bin/sh\nif [ \"$1\" = '-version' ]; then echo 'openjdk version \"21.0.1\"' >&2; exit 0; fi\n\
                 echo started > '{}'; echo 'Freerouting v2.1.0'; exit 0\n",
                started.display()
            ),
            "",
        );
        let result =
            handle_check_freerouting(&json!({ "jar_path": jar, "java_path": java }), &ctx()).await;
        PROBE_SCRATCH_FAILS.with(|fails| fails.set(false));
        let body = body(&result.unwrap());
        assert_eq!(body["available"], false, "{body}");
        assert_eq!(body["version_checked"], false, "{body}");
        assert_eq!(body["engine_found"], true, "{body}");
        assert_eq!(body["java_sufficient"], true, "{body}");
        let note = body["note"].as_str().unwrap();
        assert!(
            note.contains("not started") && note.contains("no space left on device"),
            "{note}"
        );
        assert_eq!(body["version_policy"]["installed_version"], "2.1.0");
        assert!(!started.exists(), "the JAR must not have been started");
    }

    /// The probe's settings, state and cache folders are inside its scratch
    /// home, whatever the owner's XDG variables say.
    #[tokio::test]
    async fn the_probe_keeps_every_settings_folder_in_its_scratch_home() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let dir = tempfile::tempdir().unwrap();
        let jar = fake_jar(dir.path(), "freerouting-2.5.0.jar", 65);
        let env_log = dir.path().join("env.txt");
        let args_log = dir.path().join("args.txt");
        let java = write_script(
            dir.path(),
            "java21-env",
            &format!(
                "#!/bin/sh\nif [ \"$1\" = '-version' ]; then echo 'openjdk version \"21.0.1\"' >&2; exit 0; fi\n\
                 case \"$*\" in *--version*) env > '{}'; echo \"$*\" > '{}'; \
                 echo 'Freerouting v2.5.0'; exit 1;; esac\nexit 1\n",
                env_log.display(),
                args_log.display()
            ),
            "",
        );
        let result =
            handle_check_freerouting(&json!({ "jar_path": jar, "java_path": java }), &ctx())
                .await
                .unwrap();
        assert_eq!(body(&result)["available"], true);
        let args = std::fs::read_to_string(&args_log).unwrap();
        let home = args
            .split_whitespace()
            .find_map(|a| a.strip_prefix("-Duser.home="))
            .expect("-Duser.home");
        let env = std::fs::read_to_string(&env_log).unwrap();
        for name in [
            "XDG_CONFIG_HOME",
            "XDG_STATE_HOME",
            "XDG_CACHE_HOME",
            "XDG_DATA_HOME",
            "APPDATA",
            "LOCALAPPDATA",
        ] {
            let value = env
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{name}=")))
                .unwrap_or_else(|| panic!("{name} not set in {env}"));
            assert!(value.starts_with(home), "{name}={value} is not in {home}");
        }
    }

    #[test]
    fn inherited_freerouting_settings_are_removed_and_folders_pinned() {
        let mut command = tokio::process::Command::new("java");
        isolate_freerouting_environment(
            &mut command,
            Path::new("/scratch"),
            [
                "PATH",
                "FREEROUTING__USER_DATA_PATH",
                "freerouting__logging__x",
            ]
            .map(std::ffi::OsString::from),
        );
        let envs: std::collections::BTreeMap<String, Option<String>> = command
            .as_std()
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().to_string(),
                    v.map(|v| v.to_string_lossy().to_string()),
                )
            })
            .collect();
        assert_eq!(envs["FREEROUTING__USER_DATA_PATH"], None);
        assert_eq!(envs["freerouting__logging__x"], None);
        assert!(!envs.contains_key("PATH"), "unrelated variables are kept");
        assert_eq!(envs["XDG_CONFIG_HOME"].as_deref(), Some("/scratch/.config"));
        assert_eq!(
            envs["LOCALAPPDATA"].as_deref(),
            Some("/scratch/AppData/Local")
        );
    }

    #[tokio::test]
    async fn a_short_version_name_is_the_recommended_release_and_is_hashed() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let dir = tempfile::tempdir().unwrap();
        let jar = fake_jar(dir.path(), "freerouting-2.5.jar", 65);
        let java = write_script(
            dir.path(),
            "java21-quiet2",
            "#!/bin/sh\nif [ \"$1\" = '-version' ]; then echo 'openjdk version \"21.0.1\"' >&2; exit 0; fi\nexit 0\n",
            "",
        );
        let result =
            handle_check_freerouting(&json!({ "jar_path": jar, "java_path": java }), &ctx())
                .await
                .unwrap();
        let policy = body(&result)["version_policy"].clone();
        assert_eq!(policy["matches_recommended"], true, "{policy}");
        // It was hashed (a fake JAR is not the release), not skipped.
        assert_eq!(policy["sha256_matches_release"], false, "{policy}");
    }

    #[tokio::test]
    async fn the_recommended_release_is_hashed_and_a_rebuild_is_named() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let dir = tempfile::tempdir().unwrap();
        let name = crate::freerouting_setup::RECOMMENDED.jar_name;
        let jar = fake_jar(dir.path(), name, 65);
        let java = write_script(
            dir.path(),
            "java21-quiet",
            "#!/bin/sh\nif [ \"$1\" = '-version' ]; then echo 'openjdk version \"21.0.1\"' >&2; exit 0; fi\nexit 0\n",
            "",
        );
        let result =
            handle_check_freerouting(&json!({ "jar_path": jar, "java_path": java }), &ctx())
                .await
                .unwrap();
        let policy = body(&result)["version_policy"].clone();
        assert_eq!(
            policy["installed_version_basis"], "the JAR's file name",
            "{policy}"
        );
        assert_eq!(policy["matches_recommended"], true, "{policy}");
        // A fake JAR named like the release is not the release.
        assert_eq!(policy["sha256_matches_release"], false, "{policy}");
        assert!(policy["warning"]
            .as_str()
            .unwrap()
            .contains("not the published release"));
    }

    #[tokio::test]
    async fn too_old_a_java_still_reports_the_version_policy() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let dir = tempfile::tempdir().unwrap();
        let jar = fake_jar(dir.path(), "freerouting-2.3.0.jar", 65);
        let java = fake_java(dir.path(), "java11b", 11);
        let result =
            handle_check_freerouting(&json!({ "jar_path": jar, "java_path": java }), &ctx())
                .await
                .unwrap();
        let body = body(&result);
        assert_eq!(body["java_sufficient"], false, "{body}");
        assert_eq!(
            body["version_policy"]["installed_version"], "2.3.0",
            "{body}"
        );
        assert_eq!(body["version_policy"]["matches_recommended"], false);
    }

    #[tokio::test]
    async fn without_a_jar_the_guidance_says_where_to_get_one() {
        ONLY_EXPLICIT.with(|only| only.set(true));
        let dir = tempfile::tempdir().unwrap();
        let result = handle_check_freerouting(
            &json!({ "jar_path": dir.path().join("missing.jar") }),
            &ctx(),
        )
        .await
        .unwrap();
        let body = body(&result);
        // A jar_path that is not a file keeps its own answer.
        assert_eq!(body["available"], false);
        assert!(body["note"].as_str().unwrap().contains("not a file"));
    }
}
