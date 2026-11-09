//! Local Freerouting MCP orchestration.
//!
//! Konnect deliberately does not reimplement the router. It starts the local
//! Freerouting JAR in its documented headless stdio MCP mode, follows the
//! server's routing state machine, and returns only compact job evidence.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Instant};

#[cfg(test)]
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(test)]
static LAST_CHILD_PID: AtomicU32 = AtomicU32::new(0);

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_STDERR_BYTES: usize = 64 * 1024;
const MAX_SES_BYTES: u64 = 512 * 1024 * 1024;

fn server_arguments() -> &'static [&'static str] {
    &[
        "--api_server.enabled=true",
        "--api_server.authentication.enabled=false",
        "--api_server-endpoints=http://127.0.0.1:37864",
        "--mcp_server.enabled=true",
        "--mcp_server.authentication.enabled=false",
        "--mcp_server-endpoints=http://127.0.0.1:37964",
        "--mcp_server.stdio=true",
        "--gui.enabled=false",
    ]
}

const REQUIRED_TOOLS: &[&str] = &[
    "create_session",
    "enqueue_job",
    "upload_job_input_from_local_file",
    "start_job",
    "get_job_details",
    "download_job_output_to_local_file",
];

#[derive(Debug, Clone)]
pub(crate) struct RouteSettings {
    pub max_passes: Option<u32>,
    pub optimizer_enabled: Option<bool>,
    pub job_timeout_seconds: Option<u64>,
    pub poll_interval: Duration,
    pub overall_timeout: Duration,
    /// The Java to run the JAR with; `java` on `PATH` when unset.
    pub java: Option<PathBuf>,
    /// Freerouting's automatic neck-down at pins. It narrows a track below its
    /// net class to reach a small pad, which breaks the project's minimum, so
    /// `route_specctra_dsn` turns it off unless asked.
    pub automatic_neckdown: Option<bool>,
    /// The project's copper-to-board-edge clearance, which the DSN does not
    /// carry.
    pub copper_to_edge_clearance_um: Option<f64>,
    /// The width Freerouting retries a connection at when the class width
    /// cannot reach it (`neck_width_um`): the narrowest width the manifest
    /// allows, so no track ends up below the project's floor.
    pub retry_width_um: Option<f64>,
    /// Freerouting's fanout stage (`fanout.enabled`). In 2.5.0 it necks a
    /// track down at the pin whatever `automatic_neckdown` says
    /// (`FoundConnectionInserter` runs its micro neck-down when fanout or
    /// neck-down is on), which the SES importer refuses, so
    /// `route_specctra_dsn` turns it off.
    pub fanout_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RouteEvidence {
    pub session_id: String,
    pub job_id: String,
    pub final_state: String,
    pub poll_count: u32,
    pub elapsed_seconds: f64,
    pub ses_bytes: u64,
    pub server_protocol_version: String,
    pub diagnostics_path: Option<String>,
    /// The settings Konnect sent, as Freerouting reports them in effect
    /// (`get_effective_settings`), keyed by their dotted name.
    pub effective_settings: BTreeMap<String, Value>,
    /// What the optimizer did, read from the job log; `None` when the log
    /// could not be read.
    pub optimizer: Option<OptimizerReport>,
}

/// The optimizer's outcome, from Freerouting's job log.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(crate) struct OptimizerReport {
    /// Optimizer passes that ran.
    pub passes: u32,
    /// A pass stopped early after consecutive items could not be improved.
    pub early_stop: bool,
    /// Why the optimizer stopped or never ran, when the log says (a pass
    /// improved less than the threshold; the board was not fully routed).
    pub stop_reason: Option<String>,
    pub score_before: Option<f64>,
    pub score_after: Option<f64>,
    /// The optimizer's own log lines, at most [`MAX_OPTIMIZER_LINES`].
    pub log: Vec<String>,
}

const MAX_OPTIMIZER_LINES: usize = 12;
const MAX_LOG_LINE_CHARS: usize = 240;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct BridgeProbe {
    pub native_mcp_available: bool,
    pub bridge_available: bool,
    pub server_protocol_version: Option<String>,
    pub tool_count: usize,
    pub diagnostics: Option<String>,
    pub error: Option<String>,
}

pub(crate) async fn probe_local(jar: &Path, java: Option<&Path>) -> BridgeProbe {
    let mut client = match LocalMcpClient::start(jar, java).await {
        Ok(client) => client,
        Err(error) => {
            return BridgeProbe {
                native_mcp_available: false,
                bridge_available: false,
                server_protocol_version: None,
                tool_count: 0,
                diagnostics: None,
                error: Some(format!("{error:#}")),
            };
        }
    };
    let server_protocol_version = Some(client.server_protocol_version.clone());
    let contract = async {
        let tools = client.list_tools().await?;
        validate_tool_contracts(&tools)?;
        let missing = REQUIRED_TOOLS
            .iter()
            .filter(|name| !tools.contains_key(**name))
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            bail!(
                "Freerouting MCP is missing required tool(s): {}",
                missing.join(", ")
            );
        }
        Ok::<usize, anyhow::Error>(tools.len())
    }
    .await;
    let diagnostics = client.close().await;
    match contract {
        Ok(tool_count) => BridgeProbe {
            native_mcp_available: true,
            bridge_available: true,
            server_protocol_version,
            tool_count,
            diagnostics: (!diagnostics.is_empty()).then_some(diagnostics),
            error: None,
        },
        Err(error) => BridgeProbe {
            native_mcp_available: true,
            bridge_available: false,
            server_protocol_version,
            tool_count: 0,
            diagnostics: (!diagnostics.is_empty()).then_some(diagnostics),
            error: Some(format!("{error:#}")),
        },
    }
}

/// Freerouting 2.5.0 treats a copper-to-edge clearance within 1e-9 of its own
/// default, 250 um, as "not set" (`HeadlessBoardManager.
/// applyCopperToEdgeClearanceOverride`): it skips the override, or clamps it to
/// the outline's minimum pin gap, logging at debug only, while
/// `get_effective_settings` still reports 250. A project that asks for exactly
/// 0.25 mm is sent a thousandth of a micrometre more, which the router applies
/// as asked; the read-back is compared with what was sent.
const FREEROUTING_EDGE_CLEARANCE_DEFAULT_UM: f64 = 250.0;

fn edge_clearance_to_send(um: f64) -> f64 {
    if (um - FREEROUTING_EDGE_CLEARANCE_DEFAULT_UM).abs() < 1e-9 {
        FREEROUTING_EDGE_CLEARANCE_DEFAULT_UM + 0.001
    } else {
        um
    }
}

/// The `update_job_settings` body for these settings, under the names
/// Freerouting 2.5.0 reads. Its job API parses the body into its router
/// settings by their serialized snake_case names and drops any other key
/// without an error (camelCase `neckWidthUm` was silently ignored); the flat
/// `max_passes` is a deprecated alias of `autorouter.max_passes`.
pub(crate) fn job_settings(settings: &RouteSettings) -> Map<String, Value> {
    let mut job_settings = Map::new();
    if let Some(max_passes) = settings.max_passes {
        job_settings.insert(
            "autorouter".to_string(),
            json!({ "max_passes": max_passes }),
        );
    }
    if let Some(enabled) = settings.optimizer_enabled {
        job_settings.insert("optimizer".to_string(), json!({ "enabled": enabled }));
    }
    if let Some(enabled) = settings.fanout_enabled {
        job_settings.insert("fanout".to_string(), json!({ "enabled": enabled }));
    }
    if let Some(neckdown) = settings.automatic_neckdown {
        job_settings.insert("automatic_neckdown".to_string(), json!(neckdown));
    }
    if let Some(edge) = settings.copper_to_edge_clearance_um {
        job_settings.insert(
            "copper_to_edge_clearance_um".to_string(),
            json!(edge_clearance_to_send(edge)),
        );
    }
    if let Some(width) = settings.retry_width_um {
        job_settings.insert("neck_width_um".to_string(), json!(width));
    }
    if let Some(seconds) = settings.job_timeout_seconds {
        job_settings.insert(
            "job_timeout".to_string(),
            Value::String(duration_string(seconds)),
        );
    }
    job_settings
}

/// Every leaf of a settings body, by dotted name.
fn setting_leaves(body: &Map<String, Value>) -> Vec<(String, Value)> {
    fn walk(prefix: &str, value: &Value, out: &mut Vec<(String, Value)>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let name = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    walk(&name, child, out);
                }
            }
            leaf => out.push((prefix.to_string(), leaf.clone())),
        }
    }
    let mut out = Vec::new();
    walk("", &Value::Object(body.clone()), &mut out);
    out
}

/// The value at a dotted name in Freerouting's effective settings, wherever
/// the settings object sits in the tool's response (the HTTP body may be
/// wrapped, or carried as JSON text).
fn find_setting<'a>(response: &'a Value, name: &str) -> Option<&'a Value> {
    let path: Vec<&str> = name.split('.').collect();
    fn at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
        path.iter()
            .try_fold(value, |current, key| current.get(*key))
    }
    fn search<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
        if let Some(found) = at(value, path) {
            return Some(found);
        }
        match value {
            Value::Object(map) => map.values().find_map(|child| search(child, path)),
            Value::Array(items) => items.iter().find_map(|child| search(child, path)),
            _ => None,
        }
    }
    search(response, &path)
}

fn same_setting(sent: &Value, effective: &Value) -> bool {
    match (sent.as_f64(), effective.as_f64()) {
        (Some(a), Some(b)) => (a - b).abs() <= 1e-6 * a.abs().max(1.0),
        _ => sent == effective,
    }
}

/// Compare what was sent with what Freerouting says is in effect. Returns the
/// effective value of every sent setting, or an error naming each one that
/// is missing or different: a setting the router ignored must stop the run,
/// not be found later in the copper.
fn check_effective_settings(
    sent: &Map<String, Value>,
    response: &Value,
) -> Result<BTreeMap<String, Value>> {
    let mut effective = BTreeMap::new();
    let mut wrong = Vec::new();
    for (name, value) in setting_leaves(sent) {
        match find_setting(response, &name) {
            Some(found) if same_setting(&value, found) => {
                effective.insert(name, found.clone());
            }
            Some(found) => wrong.push(format!("{name}: sent {value}, in effect {found}")),
            None => wrong.push(format!(
                "{name}: sent {value}, not in Freerouting's effective settings"
            )),
        }
    }
    if !wrong.is_empty() {
        bail!(
            "Freerouting did not apply the job settings, so nothing was routed: {}",
            wrong.join("; ")
        );
    }
    Ok(effective)
}

/// The optimizer's outcome from Freerouting's job log: the passes that ran,
/// whether a pass stopped early, and the optimizer score before and after.
fn optimizer_report(log: &Value) -> OptimizerReport {
    fn messages(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    match child {
                        Value::String(text) if key == "message" => out.push(text.clone()),
                        // Text content that is not JSON may hold the log itself.
                        Value::String(text)
                            if key == "text" && !text.trim_start().starts_with(['{', '[']) =>
                        {
                            out.extend(text.lines().map(str::to_string))
                        }
                        _ => messages(child, out),
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|item| messages(item, out)),
            _ => {}
        }
    }
    fn number_after(line: &str, marker: &str) -> Option<f64> {
        let rest = &line[line.find(marker)? + marker.len()..];
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .unwrap_or(rest.len());
        rest[..end].parse().ok()
    }
    let mut lines = Vec::new();
    messages(log, &mut lines);
    let mut report = OptimizerReport::default();
    for line in lines.iter().filter(|line| line.contains("ptimiz")) {
        if let Some(pass) = line
            .find("Optimizer pass #")
            .and_then(|_| number_after(line, "Optimizer pass #"))
        {
            report.passes = report.passes.max(pass as u32);
        }
        if line.contains("early after") {
            report.early_stop = true;
        }
        let reason = [
            "Stopping optimizer because ",
            "Skipping optimization stage: ",
        ]
        .iter()
        .find_map(|marker| line.find(marker).map(|at| &line[at + marker.len()..]));
        if let Some(reason) = reason {
            report.stop_reason = Some(
                reason
                    .trim_end_matches('.')
                    .chars()
                    .take(MAX_LOG_LINE_CHARS)
                    .collect(),
            );
        }
        // Stops 2.5.0's BatchOptimizer logs without the words above: the
        // stage's time limit ("Optimizer stage timed out." and "...before
        // starting pass #N") and its item limit. These lines are the reason,
        // so the text from the marker on is kept.
        if let Some(at) = ["Optimizer stage timed out", "Max items limit reached"]
            .iter()
            .find_map(|marker| line.find(marker))
        {
            report.stop_reason = Some(
                line[at..]
                    .trim_end_matches('.')
                    .chars()
                    .take(MAX_LOG_LINE_CHARS)
                    .collect(),
            );
        }
        // The summary line says the stage ended on its timeout; it names the
        // reason only when no earlier line did.
        if line.contains("Optimization stage completed with timeout") {
            report
                .stop_reason
                .get_or_insert_with(|| "optimization stage completed with timeout".to_string());
        }
        if line.contains("Optimization stage completed") {
            report.score_before = number_after(line, "baseline optimizer score: ");
            report.score_after = number_after(line, "final optimizer score: ");
        }
        if report.log.len() < MAX_OPTIMIZER_LINES {
            report
                .log
                .push(line.chars().take(MAX_LOG_LINE_CHARS).collect());
        }
    }
    report
}

pub(crate) async fn route_local(
    jar: &Path,
    dsn: &Path,
    ses_output: &Path,
    settings: &RouteSettings,
) -> Result<RouteEvidence> {
    validate_inputs(jar, dsn, ses_output, settings)?;
    if let Some(parent) = ses_output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create SES output directory {}", parent.display()))?;
    }
    let temporary = temporary_output_path(ses_output)?;
    if temporary.exists() {
        bail!(
            "temporary SES output already exists: {}",
            temporary.display()
        );
    }

    let mut client = LocalMcpClient::start(jar, settings.java.as_deref()).await?;
    let started = Instant::now();
    let route_result = match timeout(
        settings.overall_timeout,
        run_state_machine(&mut client, dsn, &temporary, settings, started),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!(
            "Freerouting MCP job exceeded the overall timeout of {} seconds",
            settings.overall_timeout.as_secs()
        )),
    };
    let diagnostics = client.close().await;
    let diagnostics_path = write_diagnostics(ses_output, &diagnostics).await?;

    let result = match route_result {
        Ok(evidence) => Ok(evidence),
        Err(error) => {
            if diagnostics.is_empty() {
                Err(error)
            } else {
                Err(error).context(format!("Freerouting stderr: {diagnostics}"))
            }
        }
    };
    let evidence = match result {
        Ok(evidence) => evidence,
        Err(error) => {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error);
        }
    };

    let metadata = tokio::fs::metadata(&temporary)
        .await
        .with_context(|| format!("Freerouting did not create {}", temporary.display()))?;
    if metadata.len() == 0 || metadata.len() > MAX_SES_BYTES {
        let _ = tokio::fs::remove_file(&temporary).await;
        bail!(
            "Freerouting SES size {} is outside the supported range 1..={MAX_SES_BYTES}",
            metadata.len()
        );
    }
    let ses_source = tokio::fs::read_to_string(&temporary)
        .await
        .with_context(|| format!("read Freerouting SES {}", temporary.display()))?;
    let tree = konnect_sexp::parse_sexp(&ses_source).context("parse Freerouting SES output")?;
    if tree.head() != Some("session") {
        let _ = tokio::fs::remove_file(&temporary).await;
        bail!("Freerouting output is not a Specctra SES session");
    }
    konnect_sexp::write_new_atomic(ses_output, &ses_source)
        .with_context(|| format!("create SES output {}", ses_output.display()))?;
    tokio::fs::remove_file(&temporary)
        .await
        .with_context(|| format!("remove temporary SES {}", temporary.display()))?;
    Ok(RouteEvidence {
        ses_bytes: metadata.len(),
        diagnostics_path,
        ..evidence
    })
}

async fn write_diagnostics(output: &Path, diagnostics: &str) -> Result<Option<String>> {
    if diagnostics.is_empty() {
        return Ok(None);
    }
    let path = output.with_extension("freerouting.log");
    if path.exists() {
        bail!(
            "Freerouting diagnostics output already exists: {}",
            path.display()
        );
    }
    konnect_sexp::write_new_atomic(&path, diagnostics)
        .with_context(|| format!("create Freerouting diagnostics {}", path.display()))?;
    Ok(Some(path.display().to_string()))
}

fn validate_inputs(
    jar: &Path,
    dsn: &Path,
    ses_output: &Path,
    settings: &RouteSettings,
) -> Result<()> {
    if !jar.is_file() {
        bail!("Freerouting JAR does not exist: {}", jar.display());
    }
    if !dsn.is_file() || !extension_is(dsn, "dsn") {
        bail!("DSN input must be an existing .dsn file: {}", dsn.display());
    }
    if !extension_is(ses_output, "ses") {
        bail!("SES output must have the .ses extension");
    }
    if ses_output.exists() {
        bail!("SES output already exists: {}", ses_output.display());
    }
    if let Some(max_passes) = settings.max_passes {
        if !(1..=100).contains(&max_passes) {
            bail!("max_passes must be between 1 and 100");
        }
    }
    if settings.poll_interval < Duration::from_secs(2)
        || settings.poll_interval > Duration::from_secs(5)
    {
        bail!("poll interval must be between 2 and 5 seconds");
    }
    let minimum_overall_timeout = if cfg!(test) {
        Duration::from_millis(1)
    } else {
        Duration::from_secs(10)
    };
    if settings.overall_timeout < minimum_overall_timeout
        || settings.overall_timeout > Duration::from_secs(86_400)
    {
        bail!("overall timeout must be between 10 and 86400 seconds");
    }
    if settings
        .job_timeout_seconds
        .is_some_and(|seconds| seconds == 0 || seconds > 86_400)
    {
        bail!("job_timeout_seconds must be between 1 and 86400");
    }
    Ok(())
}

async fn run_state_machine(
    client: &mut LocalMcpClient,
    dsn: &Path,
    temporary: &Path,
    settings: &RouteSettings,
    started: Instant,
) -> Result<RouteEvidence> {
    let tools = client.list_tools().await?;
    validate_tool_contracts(&tools)?;
    let missing = REQUIRED_TOOLS
        .iter()
        .filter(|name| !tools.contains_key(**name))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "Freerouting MCP is missing required tool(s): {}",
            missing.join(", ")
        );
    }

    let session = client.call_tool("create_session", json!({})).await?;
    let session_id = required_string(&session, &["sessionId", "session_id", "id"], "session id")?;
    let job = client
        .call_tool(
            "enqueue_job",
            json!({ "body": { "session_id": session_id } }),
        )
        .await?;
    let job_id = required_string(&job, &["jobId", "job_id", "id"], "job id")?;
    client
        .call_tool(
            "upload_job_input_from_local_file",
            json!({ "jobId": job_id, "filePath": absolute_utf8(dsn)? }),
        )
        .await?;

    let job_settings = job_settings(settings);
    let mut effective_settings = BTreeMap::new();
    if !job_settings.is_empty() {
        for tool in ["update_job_settings", "get_effective_settings"] {
            if !tools.contains_key(tool) {
                bail!("Freerouting MCP does not expose {tool}, so the job settings cannot be applied and checked (Freerouting 2.5.0 has both)");
            }
        }
        client
            .call_tool(
                "update_job_settings",
                json!({ "path": { "jobId": job_id }, "body": job_settings }),
            )
            .await?;
        let effective = client
            .call_tool(
                "get_effective_settings",
                json!({ "path": { "jobId": job_id } }),
            )
            .await?;
        effective_settings = check_effective_settings(&job_settings, &effective)?;
    }
    client
        .call_tool("start_job", json!({ "path": { "jobId": job_id } }))
        .await?;

    let mut poll_count = 0u32;
    let final_state = loop {
        let details = client
            .call_tool("get_job_details", json!({ "path": { "jobId": job_id } }))
            .await?;
        poll_count = poll_count.saturating_add(1);
        let state = required_string(&details, &["state", "status"], "job state")?;
        match state.to_ascii_uppercase().as_str() {
            "COMPLETED" => break "COMPLETED".to_string(),
            "FAILED" | "CANCELLED" | "CANCELED" => {
                bail!("Freerouting job {job_id} ended in state {state}")
            }
            _ => tokio::time::sleep(settings.poll_interval).await,
        }
    };

    client
        .call_tool(
            "download_job_output_to_local_file",
            json!({ "jobId": job_id, "filePath": absolute_utf8(temporary)? }),
        )
        .await?;
    // Evidence only: a log that cannot be read leaves the route as it is.
    let optimizer = if tools.contains_key("get_job_logs") {
        client
            .call_tool("get_job_logs", json!({ "path": { "jobId": job_id } }))
            .await
            .ok()
            .map(|log| optimizer_report(&log))
    } else {
        None
    };
    Ok(RouteEvidence {
        session_id,
        job_id,
        final_state,
        poll_count,
        elapsed_seconds: started.elapsed().as_secs_f64(),
        ses_bytes: 0,
        server_protocol_version: client.server_protocol_version.clone(),
        diagnostics_path: None,
        effective_settings,
        optimizer,
    })
}

fn validate_tool_contracts(tools: &BTreeMap<String, Value>) -> Result<()> {
    for (tool, paths) in [
        ("enqueue_job", &[(&["body", "session_id"][..], false)][..]),
        (
            "upload_job_input_from_local_file",
            &[(&["jobId"][..], true), (&["filePath"][..], true)][..],
        ),
        ("start_job", &[(&["path", "jobId"][..], true)][..]),
        ("get_job_details", &[(&["path", "jobId"][..], true)][..]),
        (
            "download_job_output_to_local_file",
            &[(&["jobId"][..], true), (&["filePath"][..], true)][..],
        ),
    ] {
        let schema = tools
            .get(tool)
            .with_context(|| format!("Freerouting MCP is missing required tool '{tool}'"))?;
        for (path, leaf_must_be_required) in paths {
            require_schema_path(tool, schema, path, *leaf_must_be_required)?;
        }
    }
    Ok(())
}

fn require_schema_path(
    tool: &str,
    schema: &Value,
    path: &[&str],
    leaf_must_be_required: bool,
) -> Result<()> {
    let mut current = schema;
    for (index, key) in path.iter().enumerate() {
        if current.get("type").and_then(Value::as_str) != Some("object") {
            bail!(
                "Freerouting MCP tool '{tool}' has an incompatible input schema at '{}'",
                path.join(".")
            );
        }
        let required = current
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(key)));
        if !required && (index + 1 < path.len() || leaf_must_be_required) {
            bail!(
                "Freerouting MCP tool '{tool}' does not require argument '{}'",
                path.join(".")
            );
        }
        current = current
            .get("properties")
            .and_then(|properties| properties.get(*key))
            .with_context(|| {
                format!(
                    "Freerouting MCP tool '{tool}' has no argument '{}'",
                    path.join(".")
                )
            })?;
    }
    Ok(())
}

struct LocalMcpClient {
    child: Option<Child>,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    stderr_task: Option<JoinHandle<String>>,
    next_id: u64,
    server_protocol_version: String,
}

impl LocalMcpClient {
    async fn start(jar: &Path, java: Option<&Path>) -> Result<Self> {
        let mut client = Self::spawn(jar, java)?;
        if let Err(error) = client.initialize().await {
            let diagnostics = client.close().await;
            return if diagnostics.is_empty() {
                Err(error)
            } else {
                Err(error).context(format!("Freerouting stderr: {diagnostics}"))
            };
        }
        Ok(client)
    }

    fn spawn(jar: &Path, java: Option<&Path>) -> Result<Self> {
        let mut command = Command::new(java.unwrap_or_else(|| Path::new("java")));
        command
            .arg("-jar")
            .arg(jar)
            .args(server_arguments())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .context("start local Freerouting MCP server")?;
        #[cfg(test)]
        if let Some(pid) = child.id() {
            LAST_CHILD_PID.store(pid, Ordering::SeqCst);
        }
        let stdin = child.stdin.take().context("Freerouting MCP has no stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("Freerouting MCP has no stdout")?;
        let stderr = child
            .stderr
            .take()
            .context("Freerouting MCP has no stderr")?;
        let stderr_task = tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut bytes = Vec::new();
            let _ = reader
                .take(MAX_STDERR_BYTES as u64)
                .read_to_end(&mut bytes)
                .await;
            String::from_utf8_lossy(&bytes).trim().to_string()
        });
        Ok(Self {
            child: Some(child),
            stdin,
            stdout: BufReader::new(stdout).lines(),
            stderr_task: Some(stderr_task),
            next_id: 1,
            server_protocol_version: String::new(),
        })
    }

    async fn initialize(&mut self) -> Result<()> {
        let initialize = timeout(
            STARTUP_TIMEOUT,
            self.request(
                "initialize",
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "Konnect", "version": env!("CARGO_PKG_VERSION") }
                }),
                STARTUP_TIMEOUT,
            ),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Freerouting MCP initialization timed out"))??;
        self.server_protocol_version = find_string(&initialize, &["protocolVersion"])
            .unwrap_or_else(|| MCP_PROTOCOL_VERSION.to_string());
        self.notify("notifications/initialized", json!({})).await?;
        Ok(())
    }

    async fn list_tools(&mut self) -> Result<BTreeMap<String, Value>> {
        let result = self
            .request("tools/list", json!({}), REQUEST_TIMEOUT)
            .await?;
        let tools = result
            .get("tools")
            .and_then(Value::as_array)
            .context("Freerouting tools/list returned no tools array")?;
        tools
            .iter()
            .map(|tool| {
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .context("Freerouting tools/list contains an unnamed tool")?;
                let schema = tool
                    .get("inputSchema")
                    .cloned()
                    .context("Freerouting tools/list contains a tool without inputSchema")?;
                Ok((name, schema))
            })
            .collect()
    }

    async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        let result = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
                REQUEST_TIMEOUT,
            )
            .await
            .with_context(|| format!("call Freerouting MCP tool '{name}'"))?;
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            bail!(
                "Freerouting MCP tool '{name}' returned an error: {}",
                compact(&result)
            );
        }
        Ok(expand_text_content(result))
    }

    async fn request(&mut self, method: &str, params: Value, limit: Duration) -> Result<Value> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.write_json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .await?;
        let deadline = Instant::now() + limit;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("Freerouting MCP request '{method}' timed out");
            }
            let line = timeout(remaining, self.stdout.next_line())
                .await
                .map_err(|_| anyhow::anyhow!("Freerouting MCP request '{method}' timed out"))??
                .context("Freerouting MCP closed stdout before responding")?;
            if line.trim().is_empty() {
                continue;
            }
            let response: Value = serde_json::from_str(&line)
                .with_context(|| format!("Freerouting MCP emitted non-JSON stdout: {line}"))?;
            if response.get("id") != Some(&json!(id)) {
                if response.get("method").is_some() {
                    continue;
                }
                bail!("Freerouting MCP returned an unexpected response id");
            }
            if let Some(error) = response.get("error") {
                bail!(
                    "Freerouting MCP request '{method}' failed: {}",
                    compact(error)
                );
            }
            return response
                .get("result")
                .cloned()
                .context("Freerouting MCP response has no result");
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.write_json(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    async fn write_json(&mut self, value: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(value)?;
        bytes.push(b'\n');
        self.stdin
            .write_all(&bytes)
            .await
            .context("write Freerouting MCP stdin")?;
        self.stdin
            .flush()
            .await
            .context("flush Freerouting MCP stdin")
    }

    async fn close(mut self) -> String {
        let _ = self.stdin.shutdown().await;
        if let Some(mut child) = self.child.take() {
            if timeout(SHUTDOWN_TIMEOUT, child.wait()).await.is_err() {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }
        match self.stderr_task.take() {
            Some(task) => task.await.unwrap_or_default(),
            None => String::new(),
        }
    }
}

impl Drop for LocalMcpClient {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = child.wait().await;
                });
            }
        }
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }
    }
}

fn expand_text_content(mut result: Value) -> Value {
    let parsed = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .filter_map(|text| serde_json::from_str::<Value>(text).ok())
        .collect::<Vec<_>>();
    if let Some(object) = result.as_object_mut() {
        object.insert("parsedTextContent".to_string(), Value::Array(parsed));
    }
    result
}

fn required_string(value: &Value, keys: &[&str], label: &str) -> Result<String> {
    find_string(value, keys).with_context(|| {
        format!(
            "Freerouting MCP response has no {label}: {}",
            compact(value)
        )
    })
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if keys
                    .iter()
                    .any(|candidate| key.eq_ignore_ascii_case(candidate))
                {
                    if let Some(value) = value.as_str() {
                        return Some(value.to_string());
                    }
                }
            }
            object.values().find_map(|value| find_string(value, keys))
        }
        Value::Array(values) => values.iter().find_map(|value| find_string(value, keys)),
        _ => None,
    }
}

fn absolute_utf8(path: &Path) -> Result<String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    absolute
        .to_str()
        .map(str::to_string)
        .with_context(|| format!("path is not UTF-8: {}", absolute.display()))
}

fn temporary_output_path(output: &Path) -> Result<PathBuf> {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .context("SES output has no UTF-8 filename")?;
    Ok(parent.join(format!(".{name}.konnect-{}.tmp.ses", uuid::Uuid::new_v4())))
}

fn duration_string(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn extension_is(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

fn compact(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() > 2_000 {
        let truncated: String = text.chars().take(2_000).collect();
        format!("{truncated}...")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use konnect_ipc::{IpcEffectiveRoutingRules, IpcRoutingRules};

    #[cfg(windows)]
    fn process_is_running(pid: u32) -> bool {
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .expect("run tasklist");
        String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
    }

    #[cfg(not(windows))]
    fn process_is_running(pid: u32) -> bool {
        std::process::Command::new("sh")
            .args(["-c", &format!("kill -0 {pid} 2>/dev/null")])
            .status()
            .expect("probe process")
            .success()
    }

    async fn wait_for_process_exit(pid: u32) -> bool {
        for _ in 0..50 {
            if !process_is_running(pid) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    fn routing_rules() -> IpcEffectiveRoutingRules {
        ["GND", "VCC"]
            .into_iter()
            .map(|net| {
                (
                    net.to_string(),
                    IpcRoutingRules {
                        class_name: "Default".to_string(),
                        constituents: vec!["Default".to_string()],
                        track_width_mm: Some(0.25),
                        clearance_mm: Some(0.2),
                        via_diameter_mm: Some(0.6),
                        via_drill_mm: Some(0.3),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn nested_and_text_wrapped_identifiers_are_found() {
        let value = expand_text_content(json!({
            "content": [{ "type": "text", "text": "{\"data\":{\"sessionId\":\"abc\"}}" }]
        }));
        assert_eq!(find_string(&value, &["sessionId"]), Some("abc".to_string()));
    }

    fn all_settings() -> RouteSettings {
        RouteSettings {
            max_passes: Some(20),
            optimizer_enabled: Some(true),
            job_timeout_seconds: Some(600),
            poll_interval: std::time::Duration::from_secs(2),
            overall_timeout: std::time::Duration::from_secs(60),
            java: None,
            automatic_neckdown: Some(false),
            copper_to_edge_clearance_um: Some(500.0),
            retry_width_um: Some(200.0),
            fanout_enabled: Some(false),
        }
    }

    /// The names Freerouting 2.5.0's router settings serialize under (read
    /// from its class files: `RouterSettings`, `AutorouterSettings`,
    /// `OptimizerSettings`). A key outside these is dropped silently.
    #[test]
    fn job_settings_use_the_names_freerouting_reads() {
        let body = job_settings(&all_settings());
        let names: Vec<String> = setting_leaves(&body).into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names,
            [
                "automatic_neckdown",
                "autorouter.max_passes",
                "copper_to_edge_clearance_um",
                "fanout.enabled",
                "job_timeout",
                "neck_width_um",
                "optimizer.enabled",
            ]
        );
        assert_eq!(body["neck_width_um"], json!(200.0));
        assert_eq!(body["fanout"], json!({ "enabled": false }));
        assert_eq!(body["job_timeout"], json!("00:10:00"));
        let none = job_settings(&RouteSettings {
            max_passes: None,
            optimizer_enabled: None,
            job_timeout_seconds: None,
            automatic_neckdown: None,
            copper_to_edge_clearance_um: None,
            retry_width_um: None,
            fanout_enabled: None,
            ..all_settings()
        });
        assert!(none.is_empty());
    }

    #[test]
    fn effective_settings_must_match_what_was_sent() {
        let sent = job_settings(&all_settings());
        // The HTTP body as the MCP tool wraps it, and as JSON text.
        let applied = json!({
            "status": 200,
            "body": {
                "automatic_neckdown": false,
                "copper_to_edge_clearance_um": 500.0,
                "neck_width_um": 200.0,
                "job_timeout": "00:10:00",
                "autorouter": { "max_passes": 20, "enabled": true },
                "optimizer": { "enabled": true, "max_passes": 100 },
                "fanout": { "enabled": false, "max_passes": 100 },
                "scoring": { "via_costs": 50 }
            }
        });
        let effective = check_effective_settings(&sent, &applied).unwrap();
        assert_eq!(effective["neck_width_um"], json!(200.0));
        assert_eq!(effective.len(), 7);
        let as_text = expand_text_content(json!({
            "content": [{ "type": "text", "text": applied.to_string() }]
        }));
        check_effective_settings(&sent, &as_text).unwrap();

        // The 2.5.0 default for a dropped key, and a key missing altogether.
        let mut dropped = applied.clone();
        dropped["body"]["neck_width_um"] = json!(0.0);
        dropped["body"]
            .as_object_mut()
            .unwrap()
            .remove("job_timeout");
        let error = check_effective_settings(&sent, &dropped)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("neck_width_um: sent 200.0, in effect 0.0")
                && error.contains("job_timeout")
                && error.contains("nothing was routed"),
            "{error}"
        );
        // Fanout left on (2.5.0's default) necks tracks at pins.
        let mut fanout_on = applied.clone();
        fanout_on["body"]["fanout"]["enabled"] = json!(true);
        let error = check_effective_settings(&sent, &fanout_on)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("fanout.enabled: sent false, in effect true"),
            "{error}"
        );
    }

    #[test]
    fn the_optimizer_outcome_is_read_from_the_job_log() {
        let log = json!({ "body": [
            { "type": "INFO", "message": "Starting optimization on 1 thread..." },
            { "type": "INFO", "message": "Optimizer pass #1: optimizer score 349.49 -> 355.15 (IMPROVED, 1.6189%), router score: 1000.00" },
            { "type": "INFO", "message": "Optimizer pass #2: optimizer score 355.15 -> 356.87 (IMPROVED, 0.4831%), router score: 1000.00" },
            { "type": "INFO", "message": "Stopping optimization pass #2 early after 12 consecutive items could not be improved." },
            { "type": "INFO", "message": "Optimization stage completed:. Baseline router score: 1000.00, baseline optimizer score: 349.49, final router score: 1000.00, final optimizer score: 356.87" },
            { "type": "INFO", "message": "Autorouter pass #3 completed" }
        ]});
        let report = optimizer_report(&log);
        assert_eq!(report.passes, 2);
        assert!(report.early_stop);
        assert_eq!(report.score_before, Some(349.49));
        assert_eq!(report.score_after, Some(356.87));
        assert_eq!(report.log.len(), 5);
        assert_eq!(report.stop_reason, None);
        assert_eq!(
            optimizer_report(&json!({ "body": [] })),
            OptimizerReport::default()
        );
        // Freerouting 2.5.0's own line when a pass gains too little.
        let threshold = json!({ "body": [
            { "type": "INFO", "message": "[FFE3BD] Optimizer pass #1: optimizer score 503.77 -> 513.55 (IMPROVED, 1.9432%), router score: 1000.00, incomplete connections: 0, clearance violations: 0." },
            { "type": "INFO", "message": "[FFE3BD] Stopping optimizer because the improvement in this pass (1.9432%) is below the threshold (2.50%)." }
        ]});
        let report = optimizer_report(&threshold);
        assert_eq!((report.passes, report.early_stop), (1, false));
        assert_eq!(
            report.stop_reason.as_deref(),
            Some("the improvement in this pass (1.9432%) is below the threshold (2.50%)")
        );
        // And when it never ran.
        let skipped = json!({ "body": [{ "type": "INFO", "message": "[1DA299] Skipping optimization stage: the board has 1 unrouted connection(s) (optimizer only runs on completely routed boards)." }] });
        let report = optimizer_report(&skipped);
        assert_eq!(report.passes, 0);
        assert!(report
            .stop_reason
            .unwrap()
            .starts_with("the board has 1 unrouted connection(s)"));
    }

    /// 2.5.0's BatchOptimizer also stops on its stage time limit and its item
    /// limit, and its summary line says when the stage ended on the timeout.
    #[test]
    fn a_timed_out_or_capped_optimizer_has_a_stop_reason() {
        let reason = |messages: &[&str]| {
            let log = json!({ "body": messages.iter().map(|m| json!({ "type": "INFO", "message": m })).collect::<Vec<_>>() });
            optimizer_report(&log).stop_reason
        };
        assert_eq!(
            reason(&["[AB12CD] Optimizer stage timed out."]).as_deref(),
            Some("Optimizer stage timed out")
        );
        assert_eq!(
            reason(&["Optimizer stage timed out before starting pass #3"]).as_deref(),
            Some("Optimizer stage timed out before starting pass #3")
        );
        assert_eq!(
            reason(&["Max items limit reached (500). Stopping optimizer."]).as_deref(),
            Some("Max items limit reached (500). Stopping optimizer")
        );
        // The summary of a timed-out stage alone still gives a reason, and
        // its scores are still read.
        let summary = "Optimization stage completed with timeout:. Baseline router score: 1000.00, baseline optimizer score: 10.00, final router score: 1000.00, final optimizer score: 11.00";
        let log = json!({ "body": [{ "type": "INFO", "message": summary }] });
        let report = optimizer_report(&log);
        assert_eq!(
            report.stop_reason.as_deref(),
            Some("optimization stage completed with timeout")
        );
        assert_eq!(report.score_after, Some(11.0));
        // An earlier, more specific line is kept.
        assert_eq!(
            reason(&["Optimizer stage timed out.", summary]).as_deref(),
            Some("Optimizer stage timed out")
        );
        // A stage that completed normally has none.
        assert_eq!(
            reason(&[&summary.replace("completed with timeout:", "completed:")]),
            None
        );
    }

    /// Freerouting 2.5.0 reads an edge clearance within 1e-9 of 250 um as
    /// "not set", so exactly 0.25 mm is sent a thousandth of a micrometre
    /// more, and the read-back still catches a router that reports 250.
    #[test]
    fn an_edge_clearance_of_exactly_250_um_is_nudged_off_freeroutings_default() {
        let body = |edge: f64| {
            job_settings(&RouteSettings {
                copper_to_edge_clearance_um: Some(edge),
                ..all_settings()
            })
        };
        assert_eq!(body(250.0)["copper_to_edge_clearance_um"], json!(250.001));
        assert_eq!(
            body(250.0000000001)["copper_to_edge_clearance_um"],
            json!(250.001)
        );
        assert_eq!(body(250.5)["copper_to_edge_clearance_um"], json!(250.5));
        assert_eq!(body(500.0)["copper_to_edge_clearance_um"], json!(500.0));
        let mut sent = body(250.0);
        sent.retain(|name, _| name == "copper_to_edge_clearance_um");
        let applied = json!({ "copper_to_edge_clearance_um": 250.001 });
        let kept = check_effective_settings(&sent, &applied).unwrap();
        assert_eq!(kept["copper_to_edge_clearance_um"], json!(250.001));
        // A router that clamps it back to 250 is caught.
        let clamped = json!({ "copper_to_edge_clearance_um": 250.0 });
        let error = check_effective_settings(&sent, &clamped)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("copper_to_edge_clearance_um: sent 250.001, in effect 250.0"),
            "{error}"
        );
    }

    /// A stand-in for Freerouting's stdio MCP server: a shell script that
    /// answers the routing state machine and reports `neck_width_um` as the
    /// given value in its effective settings.
    #[cfg(unix)]
    fn fake_freerouting(dir: &Path, effective_neck_width: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
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
        let settings = format!(
            r#"{{"status":200,"body":{{"automatic_neckdown":false,"copper_to_edge_clearance_um":500.0,"neck_width_um":{effective_neck_width},"job_timeout":"00:10:00","autorouter":{{"max_passes":20}},"optimizer":{{"enabled":true}},"fanout":{{"enabled":false}}}}}}"#
        );
        let logs = r#"{"status":200,"body":[{"type":"INFO","message":"Optimizer pass #1: optimizer score 10.00 -> 10.00 (UNCHANGED, 0.0000%)"},{"type":"INFO","message":"Stopping optimization pass #1 early after 12 consecutive items could not be improved."},{"type":"INFO","message":"Optimization stage completed:. Baseline router score: 1000.00, baseline optimizer score: 10.00, final router score: 1000.00, final optimizer score: 10.00"}]}"#;
        let script = format!(
            r#"#!/bin/sh
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
      printf '(session "board")\n' > "$out"
      reply "$id" '{{"message":"saved"}}' ;;
    *) reply "$id" '{{}}' ;;
  esac
done
"#
        );
        let path = dir.join("fake-java");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    async fn route_with_fake(
        effective_neck_width: &str,
    ) -> (Result<RouteEvidence>, PathBuf, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_path_buf();
        let jar = dir.join("freerouting.jar");
        std::fs::write(&jar, b"not a jar").unwrap();
        let dsn = dir.join("board.dsn");
        std::fs::write(&dsn, "(pcb board)").unwrap();
        let ses = dir.join("board.ses");
        let settings = RouteSettings {
            java: Some(fake_freerouting(&dir, effective_neck_width)),
            ..all_settings()
        };
        (route_local(&jar, &dsn, &ses, &settings).await, ses, temp)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_setting_freerouting_did_not_apply_stops_the_route() {
        let (result, ses, _temp) = route_with_fake("0.0").await;
        let error = format!("{:#}", result.unwrap_err());
        assert!(
            error.contains("neck_width_um: sent 200.0, in effect 0.0"),
            "{error}"
        );
        assert!(
            !ses.exists(),
            "no SES may be written after a refused setting"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn applied_settings_and_the_optimizer_outcome_are_reported() {
        let (result, ses, _temp) = route_with_fake("200.0").await;
        let evidence = result.unwrap();
        assert!(ses.is_file());
        assert_eq!(evidence.effective_settings["neck_width_um"], json!(200.0));
        assert_eq!(
            evidence.effective_settings["autorouter.max_passes"],
            json!(20)
        );
        let optimizer = evidence.optimizer.unwrap();
        assert_eq!((optimizer.passes, optimizer.early_stop), (1, true));
        assert_eq!(optimizer.score_after, Some(10.0));
    }

    #[test]
    fn duration_settings_use_documented_shape() {
        assert_eq!(duration_string(5 * 60), "00:05:00");
        assert_eq!(duration_string(25 * 60 * 60), "25:00:00");
    }

    #[test]
    fn output_path_is_unique_and_stays_beside_destination() {
        let output = Path::new("build/board.ses");
        let first = temporary_output_path(output).unwrap();
        let second = temporary_output_path(output).unwrap();
        assert_eq!(first.parent(), output.parent());
        assert_ne!(first, second);
        assert!(extension_is(&first, "ses"));
    }

    #[test]
    fn unauthenticated_servers_are_forced_to_loopback() {
        let arguments = server_arguments();
        assert!(arguments.contains(&"--api_server.authentication.enabled=false"));
        assert!(arguments.contains(&"--api_server-endpoints=http://127.0.0.1:37864"));
        assert!(arguments.contains(&"--mcp_server.authentication.enabled=false"));
        assert!(arguments.contains(&"--mcp_server-endpoints=http://127.0.0.1:37964"));
        assert!(!arguments
            .iter()
            .any(|argument| argument.contains("0.0.0.0")));
    }

    #[test]
    fn required_tool_schema_drift_is_refused_before_routing() {
        let object = |properties: Value, required: Value| json!({ "type": "object", "properties": properties, "required": required });
        let mut tools = BTreeMap::new();
        tools.insert("enqueue_job".into(), object(json!({"body": object(json!({"session_id": {"type":"string"}}), json!(["session_id"]))}), json!(["body"])));
        tools.insert(
            "upload_job_input_from_local_file".into(),
            object(
                json!({"jobId":{"type":"string"},"filePath":{"type":"string"}}),
                json!(["jobId", "filePath"]),
            ),
        );
        tools.insert(
            "start_job".into(),
            object(
                json!({"path": object(json!({"jobId":{"type":"string"}}), json!(["jobId"]))}),
                json!(["path"]),
            ),
        );
        tools.insert("get_job_details".into(), tools["start_job"].clone());
        tools.insert(
            "download_job_output_to_local_file".into(),
            tools["upload_job_input_from_local_file"].clone(),
        );
        validate_tool_contracts(&tools).unwrap();

        tools.get_mut("start_job").unwrap()["properties"]["path"]["required"] = json!([]);
        let error = validate_tool_contracts(&tools).unwrap_err().to_string();
        assert!(
            error.contains("start_job") && error.contains("path.jobId"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn stderr_diagnostics_are_preserved_beside_the_requested_output() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("board.ses");
        let path = write_diagnostics(&output, "router failed at pass 3")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "router failed at pass 3"
        );
    }

    /// Optional local parity check for Freerouting's actual stdio MCP state
    /// machine. CI does not install Java or Freerouting.
    #[tokio::test]
    #[ignore = "requires Java and FREEROUTING_JAR"]
    async fn native_mcp_routes_the_export_fixture() {
        let jar = PathBuf::from(std::env::var_os("FREEROUTING_JAR").expect("set FREEROUTING_JAR"));
        let source = include_str!("../tests/fixtures/specctra_two_resistors.kicad_pcb");
        let temp = tempfile::tempdir().unwrap();
        let board = temp.path().join("board.kicad_pcb");
        let dsn = temp.path().join("board.dsn");
        let ses = temp.path().join("board.ses");
        std::fs::write(&board, source).unwrap();
        let export = crate::specctra::export_dsn(&board, source, &routing_rules()).unwrap();
        std::fs::write(&dsn, export.dsn).unwrap();

        let evidence = route_local(
            &jar,
            &dsn,
            &ses,
            &RouteSettings {
                max_passes: Some(2),
                optimizer_enabled: Some(false),
                job_timeout_seconds: Some(300),
                poll_interval: Duration::from_secs(2),
                overall_timeout: Duration::from_secs(300),
                java: None,
                automatic_neckdown: None,
                copper_to_edge_clearance_um: None,
                retry_width_um: None,
                fanout_enabled: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(evidence.final_state, "COMPLETED");
        assert!(evidence.ses_bytes > 0);
        assert!(ses.is_file());
    }

    #[tokio::test]
    #[ignore = "requires Java and FREEROUTING_JAR"]
    async fn overall_timeout_closes_child_and_removes_partial_output() {
        let jar = PathBuf::from(std::env::var_os("FREEROUTING_JAR").expect("set FREEROUTING_JAR"));
        let source = include_str!("../tests/fixtures/specctra_two_resistors.kicad_pcb");
        let temp = tempfile::tempdir().unwrap();
        let board = temp.path().join("board.kicad_pcb");
        let dsn = temp.path().join("board.dsn");
        let ses = temp.path().join("board.ses");
        std::fs::write(&board, source).unwrap();
        let export = crate::specctra::export_dsn(&board, source, &routing_rules()).unwrap();
        std::fs::write(&dsn, export.dsn).unwrap();

        let error = format!(
            "{:#}",
            route_local(
                &jar,
                &dsn,
                &ses,
                &RouteSettings {
                    max_passes: Some(100),
                    optimizer_enabled: Some(true),
                    job_timeout_seconds: Some(300),
                    poll_interval: Duration::from_secs(5),
                    overall_timeout: Duration::from_millis(100),
                    java: None,
                    automatic_neckdown: None,
                    copper_to_edge_clearance_um: None,
                    retry_width_um: None,
                    fanout_enabled: None,
                },
            )
            .await
            .unwrap_err()
        );
        assert!(error.contains("overall timeout"), "{error}");
        let pid = LAST_CHILD_PID.load(Ordering::SeqCst);
        assert!(
            wait_for_process_exit(pid).await,
            "Freerouting child {pid} survived timeout"
        );
        assert!(!ses.exists());
        assert!(
            std::fs::read_dir(temp.path()).unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp.ses")),
            "timeout left a temporary SES"
        );
    }

    #[tokio::test]
    #[ignore = "requires Java and FREEROUTING_JAR"]
    async fn cancelling_route_reaps_the_owned_child() {
        let jar = PathBuf::from(std::env::var_os("FREEROUTING_JAR").expect("set FREEROUTING_JAR"));
        let source = include_str!("../tests/fixtures/specctra_two_resistors.kicad_pcb");
        let temp = tempfile::tempdir().unwrap();
        let board = temp.path().join("board.kicad_pcb");
        let dsn = temp.path().join("board.dsn");
        let ses = temp.path().join("board.ses");
        std::fs::write(&board, source).unwrap();
        let export = crate::specctra::export_dsn(&board, source, &routing_rules()).unwrap();
        std::fs::write(&dsn, export.dsn).unwrap();

        LAST_CHILD_PID.store(0, Ordering::SeqCst);
        let task = tokio::spawn(async move {
            route_local(
                &jar,
                &dsn,
                &ses,
                &RouteSettings {
                    max_passes: Some(100),
                    optimizer_enabled: Some(true),
                    job_timeout_seconds: Some(300),
                    poll_interval: Duration::from_secs(5),
                    overall_timeout: Duration::from_secs(300),
                    java: None,
                    automatic_neckdown: None,
                    copper_to_edge_clearance_um: None,
                    retry_width_um: None,
                    fanout_enabled: None,
                },
            )
            .await
        });
        let pid = loop {
            let pid = LAST_CHILD_PID.load(Ordering::SeqCst);
            if pid != 0 {
                break pid;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        task.abort();
        let _ = task.await;
        assert!(
            wait_for_process_exit(pid).await,
            "Freerouting child {pid} survived cancellation"
        );
    }
}
