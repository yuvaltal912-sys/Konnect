//! The contract between the server and the `konnect sim-worker` process.
//!
//! The worker and the server can come from different builds: an in-place update
//! replaces the file `current_exe()` names while the old server keeps running.
//! Both sides therefore state [`PROTOCOL_VERSION`], and a mismatch is reported
//! rather than guessed through.

use serde::{Deserialize, Serialize};

/// Bump when a worker argument or report field changes meaning.
pub const PROTOCOL_VERSION: u32 = 1;

/// The subcommand the worker answers to. Hidden from `--help`.
pub const WORKER_SUBCOMMAND: &str = "sim-worker";

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
