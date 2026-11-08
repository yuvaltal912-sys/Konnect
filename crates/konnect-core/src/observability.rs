//! Per-call observability: ring buffer of recent calls, per-tool counters,
//! and optional JSONL log file at `<konnect config dir>/logs/calls.jsonl`
//! (see [`calls_log_path`] for the overrides, and why tests never reach it).
//!
//! Wired into `McpHandler::execute_tool` — every tool invocation (meta-tool or
//! domain tool) lands a `CallRecord` via `CallObserver::record`. The LLM can
//! introspect via the `get_recent_calls` and `server_stats` meta-tools.
//!
//! Goals:
//! - Zero cost on the hot path: recording is lock-then-push-then-write, no
//!   serialization overhead beyond the single JSON line.
//! - Failure isolation: if the log file can't be opened or written, the tool
//!   call still succeeds; we `tracing::warn!` and move on.
//! - Bounded memory: ring buffer caps at `MAX_RECENT_CALLS`.

use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing::warn;

/// Keep the last N calls in memory for `get_recent_calls`.
pub const MAX_RECENT_CALLS: usize = 100;

// ─── Types ───────────────────────────────────────────────────────────────────

/// Outcome of a single tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallStatus {
    /// Tool ran to completion and reported success.
    Ok,
    /// Some requested items or checks completed and others did not.
    Partial,
    /// Persistence may have happened, but readback could not establish it.
    Uncertain,
    /// Tool ran to completion but reported `is_error: true`, or the handler
    /// returned an `anyhow::Error`.
    Error,
    /// Tool not found or in an unloaded toolset.
    NotFound,
}

impl CallStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CallStatus::Ok => "ok",
            CallStatus::Partial => "partial",
            CallStatus::Uncertain => "uncertain",
            CallStatus::Error => "error",
            CallStatus::NotFound => "not_found",
        }
    }
}

/// One recorded tool invocation. Keep fields compact — this is serialized to
/// JSONL on every call.
#[derive(Debug, Clone, Serialize)]
pub struct CallRecord {
    pub call_id: String,
    /// Unix epoch milliseconds when the call started.
    pub ts: u64,
    pub tool: String,
    /// `None` for meta-tools; otherwise the owning toolset name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toolset: Option<String>,
    pub dur_ms: u64,
    pub status: CallStatus,
    /// Short error identifier (e.g., "missing_argument", "ipc_unavailable").
    /// Free-form for now; promoted to a real enum in a follow-up change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
    /// Serialized size of the arguments JSON in bytes.
    pub args_bytes: usize,
    /// Serialized size of the result in bytes (content text + image data).
    pub result_bytes: usize,
}

/// Aggregated counts for a single tool across the session.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ToolStats {
    pub total: u64,
    pub errors: u64,
    pub partial: u64,
    pub uncertain: u64,
    pub total_duration_ms: u64,
    /// Last call's status for quick "is it healthy right now?" checks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

// ─── Observer ────────────────────────────────────────────────────────────────

/// Centralized call sink. Cheap to clone — all state is behind `Arc`s.
#[derive(Clone)]
pub struct CallObserver {
    inner: Arc<Inner>,
}

struct Inner {
    recent: Mutex<VecDeque<CallRecord>>,
    per_tool: Mutex<HashMap<String, ToolStats>>,
    total_calls: AtomicU64,
    error_calls: AtomicU64,
    partial_calls: AtomicU64,
    uncertain_calls: AtomicU64,
    started_at_ms: u64,
    started_instant: Instant,
    log_path: Option<PathBuf>,
    log_handle: Mutex<Option<std::fs::File>>,
}

impl CallObserver {
    /// Create a new observer. If `log_path` is provided and its parent directory
    /// can be created, new records are also appended as JSONL; otherwise records
    /// are in-memory only.
    pub fn new(log_path: Option<PathBuf>) -> Self {
        let now_ms = unix_ms();

        // Best-effort directory setup. If this fails we fall back to in-memory only.
        let resolved_log_path = log_path.and_then(|p| match p.parent() {
            Some(dir) if std::fs::create_dir_all(dir).is_ok() => Some(p),
            Some(dir) => {
                warn!(
                    "[observability] could not create log directory '{}' — JSONL logging disabled",
                    dir.display()
                );
                None
            }
            None => None,
        });

        CallObserver {
            inner: Arc::new(Inner {
                recent: Mutex::new(VecDeque::with_capacity(MAX_RECENT_CALLS)),
                per_tool: Mutex::new(HashMap::new()),
                total_calls: AtomicU64::new(0),
                error_calls: AtomicU64::new(0),
                partial_calls: AtomicU64::new(0),
                uncertain_calls: AtomicU64::new(0),
                started_at_ms: now_ms,
                started_instant: Instant::now(),
                log_path: resolved_log_path,
                log_handle: Mutex::new(None),
            }),
        }
    }

    /// Record a completed call. Never fails — logs warnings on IO issues.
    pub async fn record(&self, rec: CallRecord) {
        // Update counters.
        self.inner.total_calls.fetch_add(1, Ordering::Relaxed);
        if matches!(rec.status, CallStatus::Error | CallStatus::Uncertain) {
            self.inner.error_calls.fetch_add(1, Ordering::Relaxed);
        }
        if matches!(rec.status, CallStatus::Partial) {
            self.inner.partial_calls.fetch_add(1, Ordering::Relaxed);
        }
        if matches!(rec.status, CallStatus::Uncertain) {
            self.inner.uncertain_calls.fetch_add(1, Ordering::Relaxed);
        }

        // Update per-tool stats.
        {
            let mut per_tool = self.inner.per_tool.lock().await;
            let entry = per_tool.entry(rec.tool.clone()).or_default();
            entry.total += 1;
            entry.total_duration_ms += rec.dur_ms;
            entry.last_status = Some(rec.status.as_str().to_string());
            if matches!(rec.status, CallStatus::Error | CallStatus::Uncertain) {
                entry.errors += 1;
                entry.last_error = rec.error_kind.clone();
            }
            if matches!(rec.status, CallStatus::Partial) {
                entry.partial += 1;
            }
            if matches!(rec.status, CallStatus::Uncertain) {
                entry.uncertain += 1;
            }
        }

        // Push to ring buffer.
        {
            let mut ring = self.inner.recent.lock().await;
            if ring.len() >= MAX_RECENT_CALLS {
                ring.pop_front();
            }
            ring.push_back(rec.clone());
        }

        // Append to JSONL (best effort).
        if let Some(path) = self.inner.log_path.as_ref() {
            if let Err(e) = self.append_jsonl(path, &rec).await {
                warn!("[observability] JSONL write failed: {}", e);
            }
        }
    }

    async fn append_jsonl(&self, path: &std::path::Path, rec: &CallRecord) -> std::io::Result<()> {
        // The whole record, newline included, goes out in one `write_all` on an
        // append-mode handle. `writeln!` on a `File` issues one write for the
        // JSON and another for the newline, so two processes logging at once
        // glued records onto one line (finding C2: 1,028 unreadable lines).
        let mut line = serde_json::to_vec(rec)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push(b'\n');
        let mut guard = self.inner.log_handle.lock().await;
        if guard.is_none() {
            *guard = Some(OpenOptions::new().create(true).append(true).open(path)?);
        }
        if let Some(file) = guard.as_mut() {
            file.write_all(&line)?;
        }
        Ok(())
    }

    /// Return up to `limit` most-recent calls (newest first). `limit == 0` means
    /// the ring's current capacity (up to `MAX_RECENT_CALLS`).
    pub async fn recent(&self, limit: usize) -> Vec<CallRecord> {
        let ring = self.inner.recent.lock().await;
        let n = if limit == 0 {
            ring.len()
        } else {
            limit.min(ring.len())
        };
        ring.iter().rev().take(n).cloned().collect()
    }

    /// Snapshot of current stats for the `server_stats` meta-tool.
    pub async fn snapshot(&self) -> StatsSnapshot {
        let per_tool = self.inner.per_tool.lock().await.clone();
        let uptime_ms = self.inner.started_instant.elapsed().as_millis() as u64;
        StatsSnapshot {
            started_at_ms: self.inner.started_at_ms,
            uptime_ms,
            total_calls: self.inner.total_calls.load(Ordering::Relaxed),
            error_calls: self.inner.error_calls.load(Ordering::Relaxed),
            partial_calls: self.inner.partial_calls.load(Ordering::Relaxed),
            uncertain_calls: self.inner.uncertain_calls.load(Ordering::Relaxed),
            log_path: self
                .inner
                .log_path
                .as_ref()
                .map(|p| p.display().to_string()),
            per_tool,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StatsSnapshot {
    pub started_at_ms: u64,
    pub uptime_ms: u64,
    pub total_calls: u64,
    pub error_calls: u64,
    pub partial_calls: u64,
    pub uncertain_calls: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    pub per_tool: HashMap<String, ToolStats>,
}

// ─── Path helpers ────────────────────────────────────────────────────────────

/// Platform-specific path for konnect state files (config, logs, cache).
/// Matches the convention in `tools/config.rs::user_config_dir`.
pub fn konnect_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_default();
        PathBuf::from(appdata).join("konnect")
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("konnect")
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(".konnect")
    }
}

/// Default JSONL path for per-call logs.
pub fn default_calls_log_path() -> PathBuf {
    konnect_dir().join("logs").join("calls.jsonl")
}

/// Redirects or disables the per-call log: `off` disables it, an absolute path
/// replaces it. Anything else disables it with a warning, so a typo never writes
/// somewhere unexpected.
pub const CALL_LOG_ENV: &str = "KONNECT_CALL_LOG";

/// Where this process appends its per-call log, or `None` for in-memory only.
///
/// In order: `KONNECT_CALL_LOG` (see [`CALL_LOG_ENV`]); `KONNECT_STATE_DIR`,
/// whose `logs/calls.jsonl` is used, so a run with an isolated state folder
/// keeps its log there too; otherwise [`default_calls_log_path`].
///
/// This crate's own unit tests always get `None`: they built handlers by the
/// hundred and made up about 94% of the owner's real log (finding C1). Other
/// test processes run under `cargo`, whose `[env]` in `.cargo/config.toml` sets
/// `KONNECT_CALL_LOG=off`; tests that need a log point it at a temporary path.
pub fn calls_log_path() -> Option<PathBuf> {
    process_calls_log_path(|key| std::env::var_os(key))
}

fn process_calls_log_path(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    resolve_calls_log_path(var, default_calls_log_path)
}

fn resolve_calls_log_path(
    var: impl Fn(&str) -> Option<OsString>,
    default_path: impl FnOnce() -> PathBuf,
) -> Option<PathBuf> {
    if let Some(value) = var(CALL_LOG_ENV).filter(|value| !value.is_empty()) {
        if value.eq_ignore_ascii_case("off") {
            return None;
        }
        let path = PathBuf::from(&value);
        if path.is_absolute() {
            return Some(path);
        }
        warn!(
            "[observability] {CALL_LOG_ENV} must be `off` or an absolute path, got '{}'; JSONL logging disabled",
            path.display()
        );
        return None;
    }
    if let Some(value) = var("KONNECT_STATE_DIR").filter(|value| !value.is_empty()) {
        let root = PathBuf::from(&value);
        if root.is_absolute() {
            return Some(root.join("logs").join("calls.jsonl"));
        }
        warn!(
            "[observability] KONNECT_STATE_DIR must be an absolute path, got '{}'; JSONL logging disabled",
            root.display()
        );
        return None;
    }
    Some(default_path())
}

// ─── Utilities ───────────────────────────────────────────────────────────────

pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Generate a short call id. We don't pull in `uuid` here — 16 hex chars of
/// randomness from the running thread's nanosecond + atomic counter is plenty
/// for disambiguating a few hundred calls per session.
pub fn new_call_id() -> String {
    static CTR: AtomicU64 = AtomicU64::new(0);
    let seq = CTR.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    format!("{:08x}{:08x}", nanos, seq)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record(tool: &str, status: CallStatus) -> CallRecord {
        CallRecord {
            call_id: new_call_id(),
            ts: unix_ms(),
            tool: tool.to_string(),
            toolset: Some("test".to_string()),
            dur_ms: 5,
            status,
            error_kind: if matches!(status, CallStatus::Error | CallStatus::Uncertain) {
                Some("boom".to_string())
            } else {
                None
            },
            args_bytes: 10,
            result_bytes: 20,
        }
    }

    #[tokio::test]
    async fn ring_buffer_bounds_at_max() {
        let obs = CallObserver::new(None);
        for _ in 0..(MAX_RECENT_CALLS + 50) {
            obs.record(sample_record("t", CallStatus::Ok)).await;
        }
        let recent = obs.recent(0).await;
        assert_eq!(recent.len(), MAX_RECENT_CALLS);
    }

    #[tokio::test]
    async fn per_tool_stats_track_errors() {
        let obs = CallObserver::new(None);
        obs.record(sample_record("add_wire", CallStatus::Ok)).await;
        obs.record(sample_record("add_wire", CallStatus::Ok)).await;
        obs.record(sample_record("add_wire", CallStatus::Error))
            .await;
        obs.record(sample_record("route_trace", CallStatus::Error))
            .await;

        let snap = obs.snapshot().await;
        assert_eq!(snap.total_calls, 4);
        assert_eq!(snap.error_calls, 2);

        let aw = snap.per_tool.get("add_wire").unwrap();
        assert_eq!(aw.total, 3);
        assert_eq!(aw.errors, 1);
        assert_eq!(aw.last_status.as_deref(), Some("error"));
        assert_eq!(aw.last_error.as_deref(), Some("boom"));

        let rt = snap.per_tool.get("route_trace").unwrap();
        assert_eq!(rt.errors, 1);
    }

    #[tokio::test]
    async fn partial_and_uncertain_calls_are_accounted_separately() {
        let obs = CallObserver::new(None);
        obs.record(sample_record("batch_place_components", CallStatus::Partial))
            .await;
        obs.record(sample_record(
            "batch_place_components",
            CallStatus::Uncertain,
        ))
        .await;

        let snap = obs.snapshot().await;
        assert_eq!(snap.total_calls, 2);
        assert_eq!(snap.partial_calls, 1);
        assert_eq!(snap.uncertain_calls, 1);
        assert_eq!(snap.error_calls, 1, "uncertain is still error-like");
        let stats = snap.per_tool.get("batch_place_components").unwrap();
        assert_eq!(stats.partial, 1);
        assert_eq!(stats.uncertain, 1);
        assert_eq!(stats.last_status.as_deref(), Some("uncertain"));
    }

    #[tokio::test]
    async fn jsonl_append_roundtrip() {
        // Use a tempdir so the observer can freely create + append to a file
        // inside it without fighting Windows delete-on-close semantics.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("calls.jsonl");
        let obs = CallObserver::new(Some(path.clone()));

        obs.record(sample_record("add_wire", CallStatus::Ok)).await;
        obs.record(sample_record("add_wire", CallStatus::Error))
            .await;

        let contents = std::fs::read_to_string(&path).expect("log read");
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"status\":\"ok\""));
        assert!(lines[1].contains("\"status\":\"error\""));
        assert!(lines[1].contains("\"error_kind\":\"boom\""));
    }

    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let pairs: Vec<(String, OsString)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |key| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    fn default_for_test() -> PathBuf {
        PathBuf::from("/home/someone/.konnect/logs/calls.jsonl")
    }

    #[test]
    fn this_crates_tests_never_resolve_a_call_log() {
        // Finding C1: in-crate tests used to append to the owner's real log.
        // With no override at all, only the `cfg(test)` guard stops the default.
        assert_eq!(process_calls_log_path(|_| None), None);
        assert_eq!(
            process_calls_log_path(env_from(&[("KONNECT_CALL_LOG", "/tmp/x/calls.jsonl")])),
            None
        );
        assert_eq!(calls_log_path(), None);
    }

    #[test]
    fn the_log_path_resolves_overrides_before_the_default() {
        let resolve =
            |pairs: &[(&str, &str)]| resolve_calls_log_path(env_from(pairs), default_for_test);
        assert_eq!(resolve(&[]), Some(default_for_test()));
        assert_eq!(
            resolve(&[("KONNECT_STATE_DIR", "/tmp/state")]),
            Some(PathBuf::from("/tmp/state/logs/calls.jsonl")),
            "an isolated state folder keeps the log with it"
        );
        assert_eq!(
            resolve(&[
                ("KONNECT_CALL_LOG", "/tmp/x/calls.jsonl"),
                ("KONNECT_STATE_DIR", "/tmp/state"),
            ]),
            Some(PathBuf::from("/tmp/x/calls.jsonl")),
            "the explicit log path wins over the state folder"
        );
        for off in ["off", "OFF", "Off"] {
            assert_eq!(
                resolve(&[
                    ("KONNECT_CALL_LOG", off),
                    ("KONNECT_STATE_DIR", "/tmp/state")
                ]),
                None
            );
        }
        // A relative value disables the log rather than writing beside the cwd.
        assert_eq!(resolve(&[("KONNECT_CALL_LOG", "calls.jsonl")]), None);
        assert_eq!(resolve(&[("KONNECT_STATE_DIR", "state")]), None);
        // Empty values count as unset.
        assert_eq!(
            resolve(&[("KONNECT_CALL_LOG", ""), ("KONNECT_STATE_DIR", "")]),
            Some(default_for_test())
        );
    }

    /// Set in a child copy of this test binary: the log path it appends to.
    const APPEND_CHILD_ENV: &str = "KONNECT_TEST_APPEND_CHILD_LOG";
    const APPEND_CHILDREN: usize = 4;
    const RECORDS_PER_CHILD: usize = 400;

    /// Finding C2. Several processes append to one log at once; every line must
    /// still be one whole record. Each child is this test binary re-run on this
    /// test alone, with `APPEND_CHILD_ENV` naming the shared log.
    #[tokio::test]
    async fn concurrent_processes_append_only_whole_lines() {
        if let Some(path) = std::env::var_os(APPEND_CHILD_ENV) {
            let obs = CallObserver::new(Some(PathBuf::from(path)));
            for i in 0..RECORDS_PER_CHILD {
                let mut rec = sample_record("append_probe", CallStatus::Ok);
                // Vary the length so writes do not line up by accident.
                rec.error_kind = Some("x".repeat(i % 97));
                obs.record(rec).await;
            }
            return;
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let log = dir.path().join("calls.jsonl");
        let exe = std::env::current_exe().expect("test binary path");
        let name = format!(
            "{}::concurrent_processes_append_only_whole_lines",
            module_path!().split_once("::").map_or("", |(_, rest)| rest)
        );
        let children: Vec<_> = (0..APPEND_CHILDREN)
            .map(|_| {
                std::process::Command::new(&exe)
                    .args([name.as_str(), "--exact", "--test-threads=1", "--quiet"])
                    .env(APPEND_CHILD_ENV, &log)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .expect("spawn child test process")
            })
            .collect();
        for mut child in children {
            assert!(child.wait().expect("child exit").success(), "child failed");
        }

        let contents = std::fs::read_to_string(&log).expect("log read");
        assert!(contents.ends_with('\n'), "the last record is terminated");
        let mut ids = std::collections::HashSet::new();
        for (n, line) in contents.lines().enumerate() {
            let value: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("line {} is not one JSON record ({e}): {line}", n + 1));
            assert_eq!(value["tool"], "append_probe");
            ids.insert(value["call_id"].as_str().expect("call_id").to_string());
        }
        assert_eq!(
            contents.lines().count(),
            APPEND_CHILDREN * RECORDS_PER_CHILD,
            "every record is on its own line"
        );
        assert!(
            ids.len() > RECORDS_PER_CHILD,
            "records came from several processes"
        );
    }

    #[test]
    fn call_ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            assert!(seen.insert(new_call_id()));
        }
    }
}
