//! Runs that outlive the request that started them (Row AP2).
//!
//! A long tool (first `plan_convergent_placement`) hands its work to [`Jobs::start`] and
//! answers at once with a run id. A later call with that id ([`Jobs::report`]) gives the
//! run's progress while it runs and its whole result once it has ended. What a later call
//! needs is in the run's own folder below Konnect's state folder, so a finished run is
//! read back after a server restart as long as that folder exists:
//!
//! - `run.json`: the run's record ([`RunRecord`]): its state, its last progress, its error.
//!   It is rewritten every [`PERSIST_EVERY`] while the run runs, and last when it ends.
//! - `result.json`: the tool's whole result, written before `run.json` says `done`.
//! - `run.lock`: held under an exclusive lock by the process that runs the run, for as
//!   long as it runs. A record that says `running` beside a free lock is a run whose
//!   server stopped before it ended (`interrupted`); a held one is a run of another
//!   Konnect process, whose progress is read from its `run.json`.
//!
//! One run per board at a time ([`claim_board`]): `<state>/<kind>/.boards/<16 hex of the
//! board path's SHA-256>.lock`, held under an exclusive lock for the run's whole life,
//! with the claiming run's id in the `.json` beside it (a separate file, as
//! `run_registry` explains: Windows' locks are mandatory, so a body inside the locked
//! file could not be read while it matters). The locks are the operating system's
//! (`File::try_lock`), so they hold across Konnect processes, and the system releases
//! them when a process ends however it ends.
//!
//! Nothing here knows what a run computes: a tool gives a future that returns its result
//! and a [`Progress`] to ask while it runs. `route_specctra_dsn` can use the same module
//! with its own [`JobKind`].

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions, TryLockError};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

/// A tool whose runs this module keeps: the folder below the state folder that holds
/// one folder per run, and the tool's name (recorded in each run, and checked on read).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct JobKind {
    pub folder: &'static str,
    pub tool: &'static str,
}

/// The run's record, in its folder.
pub(crate) const RUN_FILE: &str = "run.json";
/// The finished run's whole result, in its folder.
pub(crate) const RESULT_FILE: &str = "result.json";
/// Held by the running process, in the run's folder.
pub(crate) const LOCK_FILE: &str = "run.lock";
/// Below the kind's folder: one lock and one body per board with a run.
const BOARDS_FOLDER: &str = ".boards";

/// How often a running run's progress is written to its `run.json`, so another Konnect
/// process (or a person) can read it.
pub(crate) const PERSIST_EVERY: Duration = Duration::from_secs(10);

/// The longest a report waits for a run to end. A stdio server answers one request at a
/// time, so a report that waits holds every other call back: this keeps that bounded and
/// well under the 1,800 s after which the owner's MCP client gave up (Row AP2).
pub(crate) const MAX_WAIT: Duration = Duration::from_secs(600);

/// How often a report that waits on another process's run reads its record again.
const POLL_OTHER: Duration = Duration::from_secs(2);

/// Where a run stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunState {
    Running,
    Done,
    Failed,
}

/// `run.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RunRecord {
    pub run_id: String,
    pub tool: String,
    pub board: String,
    pub state: RunState,
    /// The process that runs (or ran) it.
    pub pid: u32,
    pub started_unix_ms: u64,
    pub finished_unix_ms: Option<u64>,
    /// The tool's progress as of the last write.
    pub progress: Value,
    /// Why a `failed` run failed.
    pub error: Option<String>,
}

/// What a running run can say about itself, asked from another task at any time.
pub(crate) trait Progress: Send + Sync + 'static {
    fn snapshot(&self) -> Value;
}

/// A run's id is its folder's name: `<milliseconds>-<12 hex>`. Anything else is refused
/// before a path is made from it, so no id reaches outside the kind's folder.
pub(crate) fn valid_run_id(id: &str) -> bool {
    let Some((stamp, digest)) = id.split_once('-') else {
        return false;
    };
    (1..=20).contains(&stamp.len())
        && stamp.bytes().all(|b| b.is_ascii_digit())
        && digest.len() == 12
        && digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The folder of run `run_id` of `kind` below `state`.
pub(crate) fn run_folder_of(state: &Path, kind: &JobKind, run_id: &str) -> PathBuf {
    state.join(kind.folder).join(run_id)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// `value` as JSON at `path`, by a new file synced to disk and renamed over it, so a
/// reader never sees half of it, and a `done` record never names a result that a power
/// loss could still take away.
fn write_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    use std::io::Write as _;
    let text = serde_json::to_string_pretty(value)?;
    let temporary = path.with_extension("json.tmp");
    let mut file = File::create(&temporary)
        .map_err(|error| anyhow::anyhow!("create '{}': {error}", temporary.display()))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| anyhow::anyhow!("write '{}': {error}", temporary.display()))?;
    drop(file);
    std::fs::rename(&temporary, path)
        .map_err(|error| anyhow::anyhow!("rename to '{}': {error}", path.display()))?;
    Ok(())
}

fn read_record(folder: &Path) -> Result<RunRecord, String> {
    let path = folder.join(RUN_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("'{}' cannot be read: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("'{}' is not a run record: {error}", path.display()))
}

/// Whether the lock file at `path` is held by an open file anywhere (this process
/// included). A missing file is not held.
fn lock_held(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    matches!(file.try_lock(), Err(TryLockError::WouldBlock))
}

// ─── One run per board ───────────────────────────────────────────────────────

/// A board claimed for one run: the lock is held until this is dropped.
#[derive(Debug)]
pub(crate) struct BoardClaim {
    _lock: File,
    pub run_id: String,
}

/// `board_claim.json`: which run holds a board.
#[derive(Debug, Serialize, Deserialize)]
struct ClaimBody {
    run_id: String,
    board: String,
    pid: u32,
}

/// Why a board cannot be claimed: another run holds it. `run_id` names it when its
/// claim says which; `None` while that run is still starting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Busy {
    pub run_id: Option<String>,
}

/// The board's lock file, opened (made when missing), and the path of its body.
fn board_lock(state: &Path, kind: &JobKind, board: &Path) -> anyhow::Result<(File, PathBuf)> {
    let boards = state.join(kind.folder).join(BOARDS_FOLDER);
    std::fs::create_dir_all(&boards)
        .map_err(|error| anyhow::anyhow!("create '{}': {error}", boards.display()))?;
    let digest = Sha256::digest(board.to_string_lossy().as_bytes());
    let key: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    let lock_path = boards.join(format!("{key}.lock"));
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|error| anyhow::anyhow!("open '{}': {error}", lock_path.display()))?;
    Ok((lock, boards.join(format!("{key}.json"))))
}

/// Take `lock` without waiting: `Ok(true)` taken, `Ok(false)` held elsewhere.
fn take(lock: &File) -> anyhow::Result<bool> {
    match lock.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(error)) => Err(anyhow::anyhow!("lock a board's claim: {error}")),
    }
}

/// The run a held board's claim body names. A body left by a run that has ended (its
/// record says so, or says `running` beside a free lock) names no one: the holder has
/// not written its own yet. A body whose run has no record yet is a run still starting,
/// and is named.
fn holder(state: &Path, kind: &JobKind, body_path: &Path) -> Busy {
    let run_id = std::fs::read_to_string(body_path)
        .ok()
        .and_then(|text| serde_json::from_str::<ClaimBody>(&text).ok())
        .map(|body| body.run_id)
        .filter(|id| valid_run_id(id))
        .filter(|id| {
            let folder = run_folder_of(state, kind, id);
            read_record(&folder).map_or(true, |record| {
                record.state == RunState::Running && lock_held(&folder.join(LOCK_FILE))
            })
        });
    Busy { run_id }
}

/// Which run holds `board`, if one does: the claim's test without the claim, for a
/// start to refuse before it does any work. Errs only when the files cannot be made.
pub(crate) fn board_busy(
    state: &Path,
    kind: &JobKind,
    board: &Path,
) -> anyhow::Result<Option<Busy>> {
    let (lock, body_path) = board_lock(state, kind, board)?;
    Ok((!take(&lock)?).then(|| holder(state, kind, &body_path)))
}

/// Claim `board` (a canonical path) for the new run `run_id` of `kind`, or say which run
/// holds it. The claim is an exclusive lock on the board's lock file below
/// `<state>/<kind>/.boards/`, taken without waiting; the run's id is written beside it.
/// Errs only when the files cannot be made.
pub(crate) fn claim_board(
    state: &Path,
    kind: &JobKind,
    board: &Path,
    run_id: &str,
) -> anyhow::Result<Result<BoardClaim, Busy>> {
    let (lock, body_path) = board_lock(state, kind, board)?;
    if !take(&lock)? {
        return Ok(Err(holder(state, kind, &body_path)));
    }
    write_json(
        &body_path,
        &ClaimBody {
            run_id: run_id.to_string(),
            board: board.display().to_string(),
            pid: std::process::id(),
        },
    )?;
    Ok(Ok(BoardClaim {
        _lock: lock,
        run_id: run_id.to_string(),
    }))
}

// ─── The registry ────────────────────────────────────────────────────────────

/// A run of this process that has not ended.
struct Active {
    record: RunRecord,
    started: Instant,
    progress: Arc<dyn Progress>,
    ended: watch::Receiver<bool>,
}

/// The runs of this process that have not ended, by folder. Cloning shares them.
#[derive(Clone, Default)]
pub(crate) struct Jobs {
    active: Arc<Mutex<BTreeMap<PathBuf, Arc<Active>>>>,
}

/// What a run's task leaves when it ends, however it ends (an unwind too): the locks
/// first, so the board is free once the run reads as ended; then the registry entry;
/// then the signal that wakes the reports that wait.
struct Leave {
    held: Option<(BoardClaim, File)>,
    jobs: Jobs,
    folder: PathBuf,
    ended: Option<watch::Sender<bool>>,
}

impl Drop for Leave {
    fn drop(&mut self) {
        drop(self.held.take());
        self.jobs.lock().remove(&self.folder);
        if let Some(ended) = self.ended.take() {
            let _ = ended.send(true);
        }
    }
}

/// What a run's own thread hands back: the work's answer, or why there is none.
type Answer = Result<anyhow::Result<Value>, String>;

/// `work` to its end on a runtime of its own (two workers), so the CPU-heavy parts of a
/// run never hold the server's runtime: the server keeps answering, and its timers
/// (the progress writes, the reports' deadlines) keep firing. A panic in the work is
/// its message.
fn run_alone<W>(work: W) -> Answer
where
    W: Future<Output = anyhow::Result<Value>> + Send + 'static,
{
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("konnect-run")
        .enable_all()
        .build()
        .map_err(|error| format!("the run's runtime could not start: {error}"))?;
    let joined = runtime.block_on(async move { tokio::spawn(work).await });
    // Nothing of the run is left to wait for: its routers were dropped with its future.
    runtime.shutdown_background();
    joined.map_err(|join| match join.try_into_panic() {
        Ok(payload) => format!("the run panicked: {}", panic_text(&*payload)),
        Err(join) => format!("the run stopped: {join}"),
    })
}

/// What a report finds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Report {
    /// Not ended. `here`: run by this process, so `record.progress` is live; else by
    /// another Konnect process, and `record.progress` is its last write.
    Running {
        record: RunRecord,
        elapsed_s: f64,
        here: bool,
    },
    /// Ended with a result: `result` is the tool's whole result, as it was written.
    Done { record: RunRecord, result: Value },
    /// Ended with an error (`record.error`).
    Failed { record: RunRecord },
    /// Its record says it runs, but no process holds its lock: the server that ran it
    /// stopped before it ended.
    Interrupted { record: RunRecord },
    /// No run by that id can be read; `why` says what was found instead.
    Unknown { why: String },
}

impl Jobs {
    /// The server's registry, one per process.
    pub(crate) fn global() -> &'static Jobs {
        static JOBS: LazyLock<Jobs> = LazyLock::new(Jobs::default);
        &JOBS
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<PathBuf, Arc<Active>>> {
        self.active.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Start `work` in the background as run `claim.run_id` of `kind` for `board`, in
    /// `folder` (made by the tool, empty of this module's files). Writes `run.json` and
    /// takes `run.lock` before it returns the first record. `work` runs on a thread and
    /// runtime of its own ([`run_alone`]); a task on the caller's runtime writes its
    /// progress and its end, and keeps the claim and the lock until the run's files are
    /// written: `result.json`, then `run.json` as `done`; or `run.json` as `failed` when
    /// `work` errs, panics, or its result cannot be written. Needs a Tokio runtime.
    pub(crate) fn start<W>(
        &self,
        kind: &JobKind,
        claim: BoardClaim,
        board: &Path,
        folder: PathBuf,
        progress: Arc<dyn Progress>,
        work: W,
    ) -> anyhow::Result<RunRecord>
    where
        W: Future<Output = anyhow::Result<Value>> + Send + 'static,
    {
        let lock_path = folder.join(LOCK_FILE);
        let run_lock = File::create(&lock_path)
            .map_err(|error| anyhow::anyhow!("create '{}': {error}", lock_path.display()))?;
        run_lock
            .try_lock()
            .map_err(|error| anyhow::anyhow!("lock '{}': {error}", lock_path.display()))?;
        let record = RunRecord {
            run_id: claim.run_id.clone(),
            tool: kind.tool.to_string(),
            board: board.display().to_string(),
            state: RunState::Running,
            pid: std::process::id(),
            started_unix_ms: now_ms(),
            finished_unix_ms: None,
            progress: progress.snapshot(),
            error: None,
        };
        write_json(&folder.join(RUN_FILE), &record)?;
        let (answer_tx, mut answer) = tokio::sync::oneshot::channel::<Answer>();
        std::thread::Builder::new()
            .name(format!("konnect-run-{}", record.run_id))
            .spawn(move || {
                let _ = answer_tx.send(run_alone(work));
            })
            .map_err(|error| anyhow::anyhow!("start the run's thread: {error}"))?;
        let (ended_tx, ended) = watch::channel(false);
        let active = Arc::new(Active {
            record: record.clone(),
            started: Instant::now(),
            progress,
            ended,
        });
        self.lock().insert(folder.clone(), Arc::clone(&active));
        let leave = Leave {
            held: Some((claim, run_lock)),
            jobs: self.clone(),
            folder: folder.clone(),
            ended: Some(ended_tx),
        };
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(PERSIST_EVERY);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            tick.tick().await;
            let joined = loop {
                tokio::select! {
                    joined = &mut answer => break joined,
                    _ = tick.tick() => {
                        let mut now = active.record.clone();
                        now.progress = active.progress.snapshot();
                        if let Err(error) = write_json(&folder.join(RUN_FILE), &now) {
                            tracing::warn!("run {}: progress not written: {error:#}", now.run_id);
                        }
                    }
                }
            };
            let mut last = active.record.clone();
            last.progress = active.progress.snapshot();
            last.finished_unix_ms = Some(now_ms());
            let outcome = match joined {
                Ok(Ok(Ok(result))) => write_json(&folder.join(RESULT_FILE), &result)
                    .map_err(|error| format!("the result could not be written: {error:#}")),
                Ok(Ok(Err(error))) => Err(format!("{error:#}")),
                Ok(Err(why)) => Err(why),
                Err(_) => Err("the run's thread ended without an answer".to_string()),
            };
            match outcome {
                Ok(()) => last.state = RunState::Done,
                Err(error) => {
                    last.state = RunState::Failed;
                    last.error = Some(error);
                }
            }
            // Once more on a failure: a `running` record beside a free lock reads as
            // interrupted (unless `result.json` is there to read).
            if let Err(error) = write_json(&folder.join(RUN_FILE), &last)
                .or_else(|_| write_json(&folder.join(RUN_FILE), &last))
            {
                tracing::warn!("run {}: its end was not written: {error:#}", last.run_id);
            }
            drop(leave);
        });
        Ok(record)
    }

    /// Run `run_id` of `kind` below `state`: waits up to `wait` (at most [`MAX_WAIT`])
    /// for it to end, then reads it. A run of this process is read from memory while it
    /// runs; every other case from its folder, so a finished run is read after a restart.
    pub(crate) async fn report(
        &self,
        state: &Path,
        kind: &JobKind,
        run_id: &str,
        wait: Duration,
    ) -> Report {
        if !valid_run_id(run_id) {
            return Report::Unknown {
                why: format!(
                    "'{run_id}' is not a run id: a run id reads like 1700000000000-0123456789ab"
                ),
            };
        }
        let folder = run_folder_of(state, kind, run_id);
        let deadline = tokio::time::Instant::now() + wait.min(MAX_WAIT);
        let active = self.lock().get(&folder).cloned();
        if let Some(active) = active {
            let mut ended = active.ended.clone();
            let timed_out = tokio::time::timeout_at(deadline, ended.wait_for(|ended| *ended))
                .await
                .is_err();
            // Ended, or its task gone (the channel closed): the folder says how it ended.
            if timed_out && self.lock().contains_key(&folder) {
                let mut record = active.record.clone();
                record.progress = active.progress.snapshot();
                return Report::Running {
                    record,
                    elapsed_s: active.started.elapsed().as_secs_f64(),
                    here: true,
                };
            }
        }
        loop {
            let report = read_report(&folder, kind);
            let now = tokio::time::Instant::now();
            if !matches!(report, Report::Running { .. }) || now >= deadline {
                return report;
            }
            tokio::time::sleep((deadline - now).min(POLL_OTHER)).await;
        }
    }
}

/// A run as its folder says: `run.json`, then `result.json` for a `done` one and the
/// lock for a `running` one.
fn read_report(folder: &Path, kind: &JobKind) -> Report {
    if !folder.is_dir() {
        return Report::Unknown {
            why: format!("there is no run folder '{}'", folder.display()),
        };
    }
    if !folder.join(RUN_FILE).is_file() {
        return Report::Unknown {
            why: format!(
                "'{}' holds no run record: the run was made before runs were recorded, or its start was refused",
                folder.display()
            ),
        };
    }
    let record = match read_record(folder) {
        Ok(record) => record,
        Err(why) => return Report::Unknown { why },
    };
    if record.tool != kind.tool {
        return Report::Unknown {
            why: format!("run {} is a run of {}", record.run_id, record.tool),
        };
    }
    match record.state {
        RunState::Done => match read_result(folder) {
            Ok(result) => Report::Done { record, result },
            Err(error) => Report::Unknown {
                why: format!(
                    "run {} ended, but its result '{}' cannot be read: {error}",
                    record.run_id,
                    folder.join(RESULT_FILE).display()
                ),
            },
        },
        RunState::Failed => Report::Failed { record },
        RunState::Running if lock_held(&folder.join(LOCK_FILE)) => {
            let elapsed_s = now_ms().saturating_sub(record.started_unix_ms) as f64 / 1000.0;
            Report::Running {
                record,
                elapsed_s,
                here: false,
            }
        }
        // A run whose result was written but whose end was not (a failed last write, a
        // crash between the two): the result is there, so the run is done.
        RunState::Running => match read_result(folder) {
            Ok(result) => Report::Done { record, result },
            Err(_) => Report::Interrupted { record },
        },
    }
}

fn read_result(folder: &Path) -> Result<Value, String> {
    let path = folder.join(RESULT_FILE);
    std::fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|text| serde_json::from_str(&text).map_err(|error| error.to_string()))
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| text.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".to_string())
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;
