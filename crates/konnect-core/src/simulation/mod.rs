//! SPICE simulation support.
//!
//! ngspice runs in a separate `konnect sim-worker` process, never inside the
//! server: a simulator crash must not take the MCP session down with it. This
//! crate holds everything that needs no `unsafe`; the FFI lives with the worker
//! in the `konnect` binary crate.

pub(crate) mod ac;
pub(crate) mod element;
pub(crate) mod export;
pub(crate) mod flatten;
pub(crate) mod kicad_netlist;
pub(crate) mod measure;
pub(crate) mod net_names;
pub(crate) mod plot;
pub(crate) mod preflight;
pub mod protocol;
pub mod raw;
pub(crate) mod runs;
pub(crate) mod signals;
pub(crate) mod snapshot;
pub(crate) mod worker_client;

pub use worker_client::{SimulationRuntime, SimulationSettings};
