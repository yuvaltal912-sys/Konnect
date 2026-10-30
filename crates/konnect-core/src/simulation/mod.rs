//! SPICE simulation support.
//!
//! ngspice runs in a separate `konnect sim-worker` process, never inside the
//! server: a simulator crash must not take the MCP session down with it. This
//! crate holds everything that needs no `unsafe`; the FFI lives with the worker
//! in the `konnect` binary crate.

pub mod protocol;
pub(crate) mod worker_client;

pub use worker_client::{SimulationRuntime, SimulationSettings};
