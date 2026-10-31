//! The contract between the server and the `konnect sim-worker` process.
//!
//! The worker and the server can come from different builds: an in-place update
//! replaces the file `current_exe()` names while the old server keeps running.
//! Both sides therefore state [`PROTOCOL_VERSION`], and a mismatch is reported
//! rather than guessed through.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Bump when a worker argument or report field changes meaning.
pub const PROTOCOL_VERSION: u32 = 2;

/// The subcommand the worker answers to. Hidden from `--help`.
pub const WORKER_SUBCOMMAND: &str = "sim-worker";

/// Files in a run folder. The server writes the first two, and the worker writes
/// the rest. `status.json` lands last, by rename, so its presence means the worker
/// reached the end.
pub const REQUEST_FILE: &str = "request.json";
pub const DECK_FILE: &str = "deck.sp";
pub const RAW_FILE: &str = "results.raw";
pub const LOG_FILE: &str = "engine.log";
pub const STATUS_FILE: &str = "status.json";

/// What `konnect sim-worker --run-dir` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRequest {
    pub protocol: u32,
    pub library: PathBuf,
    pub code_models: Option<PathBuf>,
    pub load_digital: bool,
    /// The value for `set ngbehavior`, or none.
    pub compatibility: Option<String>,
    /// Where code models look for relative data files.
    pub input_path: Option<PathBuf>,
    /// When to halt the run softly. The worker leaves by itself a little later.
    pub budget_ms: u64,
}

/// Why the worker stopped a run that was still going.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HaltReason {
    Timeout,
}

/// What the worker saw. Return codes prove nothing, so this holds the evidence
/// and leaves deciding what it means to the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStatus {
    pub protocol: u32,
    /// Why the library or the request could not be used.
    pub load_error: Option<String>,
    pub engine_version: Option<String>,
    pub code_models_loaded: Vec<String>,
    pub code_models_failed: Vec<String>,
    /// The exit callback fired, with its status. The library is dead after this.
    pub exit_status: Option<i32>,
    /// The analysis had started, so a fatal error came from the run, not the load.
    pub run_started: bool,
    /// `$sim_status`. Absent if the circuit never parsed.
    pub sim_status: Option<i32>,
    pub halted_reason: Option<HaltReason>,
    pub results_written: bool,
}

/// What `konnect sim-worker --probe` observed by loading the library.
///
/// Every field comes from that load. A library that exists but cannot load is
/// `load_error`, so a path alone never reads as an available engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeReport {
    pub protocol: u32,
    /// Why the library could not be loaded or initialised.
    pub load_error: Option<String>,
    /// From the banner the library printed, not from KiCad's build metadata.
    pub engine_version: Option<String>,
    /// File stems of the code models that loaded, such as `analog`.
    pub code_models_loaded: Vec<String>,
    /// File stems of the code models the library rejected.
    pub code_models_failed: Vec<String>,
    /// The library called its exit callback, so it must be treated as dead.
    pub exit_callback: bool,
}
