//! SPICE simulation tools.

use super::{cli, get_path, sch_export, ToolContext, ToolDef};
use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::CallToolResult;
use crate::outcome::{self, OutcomeStatus};
use crate::simulation::export::{self, Layout};
use crate::simulation::flatten::{self, Compatibility, DeckInputs, FlatDeck, Refusal};
use crate::simulation::kicad_netlist::{self, ElementEntry, KiCadNetlist, NetEntry, SpiceNetlist};
use crate::simulation::measure::{self, AxisSpec, Measurement};
use crate::simulation::plot;
use crate::simulation::preflight::{self, Preflight};
use crate::simulation::protocol::{
    RunRequest, RunStatus, DECK_FILE, LOG_FILE, PROTOCOL_VERSION, RAW_FILE, REQUEST_FILE,
};
use crate::simulation::raw::Plot;
use crate::simulation::runs::{RecordFields, RunRecord, RunState, Stopped};
use crate::simulation::signals::{self, Names, NotFound, Signal};
use crate::simulation::worker_client::{
    classify, probe_engine, read_results, run_worker, EngineStatus, SimulationRuntime, Verdict,
    WorkerEnd, MAX_RAW_LIMIT, PROBE_TIMEOUT,
};
use crate::simulation::{ac, element, runs, snapshot};
use crate::tool;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod plots;
mod stored;
mod sweeps;
use sweeps::{Ac, DcSweep};

const EXPORT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_LIMIT: u64 = 25;
const MAX_LIMIT: u64 = 200;
const DETAIL_LIMIT: usize = 800;
const MAX_SPICE_LINES: usize = 200;
const DEFAULT_TIMEOUT_SECONDS: u64 = 40;
const MAX_TIMEOUT_SECONDS: u64 = 600;
const DEFAULT_ROW_LIMIT: u64 = 40;
const DEFAULT_RAW_BYTES: u64 = 256 * 1024 * 1024;
const MIN_RAW_BYTES: u64 = 1024 * 1024;
const MAX_SIGNALS: usize = 16;
const MAX_MEASUREMENTS: usize = 32;
const MAX_ENVELOPE_POINTS: u64 = 64;
/// The print step as a share of the stop time. Too small a share kills the engine
/// outright, as `.tran 1n 10` does, because it sizes memory from this ratio.
const PRINT_STEPS: f64 = 2000.0;
/// The stop times that run on the engine. Far longer ones stop for no reason in
/// the circuit: ngspice's smallest step is a fixed share of its largest.
const STOP_TIME_RANGE: std::ops::RangeInclusive<f64> = 1e-15..=1e6;
/// How close to the stop time a run must end to count as covering it.
const COVERAGE_TOLERANCE: f64 = 1e-6;
const SOURCE: &str = "saved_schematic_snapshot";
/// "Engine ran" and "requested range covered", so a run with no measurements can
/// still read as partial.
const FIXED_OUTCOME_UNITS: usize = 2;
const MAX_ERROR_ITEMS: usize = 20;
const MAX_LOG_ITEMS: usize = 12;
const MAX_LOG_ITEM_CHARS: usize = 300;
/// The worker runs with no init file on purpose, so this says nothing.
const SPINIT_NOTICE: &str = "Warning: can't find the initialization file";
const MODEL_LINES_NOTE: &str = "lines quoting a model file are in the log.";
const MIN_PRIVATE_WORD: usize = 4;
/// Progress such as gmin stepping, which says nothing about the circuit.
const ENGINE_NOTE: &str = "Note:";
/// The engine answering Konnect's own halt, which `stopped` already reports.
const HALT_NOTICES: [&str; 2] = ["doAnalyses: pause requested", "run simulation interrupted"];
/// The engine's status variable, which ends the part of a log worth reading.
const STATUS_NAME: &str = "sim_status";
const OPERATING_POINT_FILE: &str = "operating_point.json";
const CSV_FILE: &str = "results.csv";
/// KiCad's own defaults for what to save. `.probe alli` stops the engine on any
/// event-driven node, so it is left out of those circuits.
const SAVE_ALL_VOLTAGES: &str = ".save all";
const SAVE_ALL_CURRENTS: &str = ".probe alli";
const OPERATING_POINT: &str = ".op";
/// Warnings that mean the result rests on an assumption about a part.
const APPROXIMATIONS: [&str; 2] = ["inferred_from_value", "generic_default_model"];

fn testbench_schema() -> Value {
    json!({
        "type": "object",
        "description": "Changes to the deck for simulation only. The schematic is not touched",
        "properties": {
            "exclude_references": {
                "type": "array",
                "items": { "type": "string" },
                "description": "References to leave out, or prefix wildcards such as J*"
            },
            "spice_lines": {
                "type": "array",
                "maxItems": MAX_SPICE_LINES,
                "items": { "type": "string" },
                "description": "Elements, models and includes to add. Use spice_name values from check_simulation_readiness"
            }
        }
    })
}

fn analysis_schema() -> Value {
    json!({
        "oneOf": [{
            "type": "object",
            "properties": {
                "kind": { "const": "transient" },
                "stop_time_seconds": { "type": "number", "exclusiveMinimum": 0 },
                "max_step_seconds": {
                    "type": "number",
                    "exclusiveMinimum": 0,
                    "description": "Largest internal step. Default is stop_time_seconds / 2000"
                },
                "start_save_seconds": {
                    "type": "number",
                    "minimum": 0,
                    "description": "Save no data before this time"
                },
                "use_initial_conditions": {
                    "type": "boolean",
                    "default": false,
                    "description": "Start from .ic values and skip the operating point"
                }
            },
            "required": ["kind", "stop_time_seconds"]
        }, {
            "type": "object",
            "properties": {
                "kind": { "const": "ac" },
                "start_hz": { "type": "number", "exclusiveMinimum": 0 },
                "stop_hz": {
                    "type": "number",
                    "exclusiveMinimum": 0,
                    "description": "Above start_hz. Both from 1e-9 to 1e18. An octave sweep ends on the last point at or below it"
                },
                "scale": {
                    "enum": ["decade", "octave", "linear"],
                    "default": "decade"
                },
                "points": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 10000,
                    "default": 50,
                    "description": "Per decade or octave, or in all if linear (at least 3). Phase margin and gain margin need enough points to follow the phase: use 100 a decade or more near a resonance"
                }
            },
            "required": ["kind", "start_hz", "stop_hz"],
            "description": "Small-signal sweep. Needs a source with an AC value, such as `AC 1`"
        }, {
            "type": "object",
            "properties": {
                "kind": { "const": "dc_sweep" },
                "sweeps": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 2,
                    "description": "The first sweep varies fastest",
                    "items": {
                        "type": "object",
                        "properties": {
                            "target": {
                                "type": "string",
                                "description": "Reference of a voltage source, current source or resistor, or TEMP. Values use its unit: V, A, ohm or degrees C"
                            },
                            "start_value": { "type": "number" },
                            "stop_value": { "type": "number" },
                            "step_value": {
                                "type": "number",
                                "description": "Not 0. Negative to sweep down"
                            }
                        },
                        "required": ["target", "start_value", "stop_value", "step_value"]
                    }
                }
            },
            "required": ["kind", "sweeps"]
        }, {
            "type": "object",
            "properties": { "kind": { "const": "operating_point" } },
            "required": ["kind"]
        }]
    })
}

fn measurement_schema() -> Value {
    let kinds: Vec<&str> = measure::KINDS.iter().map(|(name, _)| *name).collect();
    json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "description": "Label to repeat in the result" },
            "kind": { "enum": kinds },
            "signal": {
                "type": "string",
                "description": "v(NET), v(NET1,NET2), i(REF), i(REF:PIN) or p(REF)"
            },
            "reference_signal": {
                "type": "string",
                "description": "AC only: the measured signal is divided by this one, as in a loop gain taken between two nodes"
            },
            "from_seconds": { "type": "number", "description": "Transient window start. Default is the first point" },
            "to_seconds": { "type": "number", "description": "Transient window end. Default is the last point" },
            "at_seconds": { "type": "number", "description": "Transient value_at" },
            "from_hz": { "type": "number", "description": "AC: only the points from here up are examined" },
            "to_hz": { "type": "number", "description": "AC: only the points up to here are examined" },
            "at_hz": { "type": "number", "description": "AC value_at: gain in dB, with phase" },
            "from_sweep": { "type": "number", "description": "DC sweep window start, in the order of the sweep" },
            "to_sweep": { "type": "number", "description": "DC sweep window end, in the order of the sweep" },
            "at_sweep": { "type": "number", "description": "DC sweep value_at" },
            "level": { "type": "number", "description": "For crossing" },
            "reference_level": {
                "enum": ["low_frequency", "peak"],
                "default": "low_frequency",
                "description": "For bandwidth_3db: 3 dB below the lowest frequency's gain, or below the peak, which gives both edges"
            },
            "direction": {
                "enum": ["rising", "falling", "either"],
                "default": "either",
                "description": "For crossing"
            },
            "final_value": {
                "type": "number",
                "description": "For rise_time, fall_time, overshoot_percent and settling_time. Default is the value at the end of the window"
            },
            "band_percent": {
                "type": "number",
                "exclusiveMinimum": 0,
                "default": 2,
                "description": "For settling_time: a share of the final value or the step, whichever is larger"
            }
        },
        "required": ["kind", "signal"]
    })
}

pub fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "check_simulation_readiness",
        "Report whether a schematic can be simulated, and why not, without running a simulation. Works from the saved files on a private copy, so the project is not touched and unsaved editor changes are not seen.",
        json!({
            "type": "object",
            "properties": {
                "schematic": { "type": "string", "description": "Root .kicad_sch path" },
                "testbench": testbench_schema(),
                "net_filter": { "type": "string", "description": "Only list nets whose name contains this text" },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_LIMIT,
                    "default": DEFAULT_LIMIT,
                    "description": "Most nets and elements to list; totals are always complete"
                }
            },
            "required": ["schematic"]
        }),
        |args, ctx| async move { handle_check_simulation_readiness(args, ctx).await }
    ), tool!(
        "run_simulation",
        "Run one analysis on a schematic in an isolated simulator process and return compact results, with the full data on disk. Works from the saved files on a private copy. Read simulated_circuit: a result from a modified circuit does not describe the schematic as drawn. Runs an operating point, a transient, an AC sweep or a DC sweep. A transient can summarise signals and take time-weighted measurements such as inrush peak, I²t and settling time. An AC sweep gives gain in dB, and bandwidth, phase margin and gain margin. A DC sweep varies a source, a resistor or the temperature, and takes extremes and crossings along it.",
        json!({
            "type": "object",
            "properties": {
                "schematic": { "type": "string", "description": "Root .kicad_sch path" },
                "analysis": analysis_schema(),
                "testbench": testbench_schema(),
                "signals": {
                    "type": "array",
                    "maxItems": MAX_SIGNALS,
                    "items": { "type": "string" },
                    "description": "Signals to summarise: v(NET), v(NET1,NET2), i(REF), i(REF:PIN) or p(REF; not in an AC sweep). Net names as KiCad shows them. An AC summary is in dB, 20 log10 of the magnitude, which is the gain when the AC source is 1"
                },
                "measurements": {
                    "type": "array",
                    "maxItems": MAX_MEASUREMENTS,
                    "items": measurement_schema(),
                    "description": "Values to work out from the run. Each analysis takes its own kinds, and a kind that does not fit it is refused: a transient takes all but the AC ones, an AC sweep takes bandwidth_3db, phase_margin, gain_margin and value_at, a DC sweep with one sweep takes min, max, peak_to_peak, peak_abs, value_at and crossing"
                },
                "envelope_points": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_ENVELOPE_POINTS,
                    "default": 0,
                    "description": "Transient only. Buckets of min and max to add to each summary"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_LIMIT,
                    "default": DEFAULT_ROW_LIMIT,
                    "description": "Most result rows to return; totals are always complete"
                },
                "timeout_seconds": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_TIMEOUT_SECONDS,
                    "default": DEFAULT_TIMEOUT_SECONDS,
                    "description": "Budget for the whole call, export included"
                },
                "max_raw_bytes": {
                    "type": "integer",
                    "minimum": MIN_RAW_BYTES,
                    "maximum": MAX_RAW_LIMIT,
                    "default": DEFAULT_RAW_BYTES,
                    "description": "Stop the run softly when its data reaches this size, keeping what it has"
                }
            },
            "required": ["schematic", "analysis"]
        }),
        |args, ctx| async move { handle_run_simulation(args, ctx).await }
    ), tool!(
        "list_simulation_runs",
        "List the stored runs of a schematic, newest first, so a run is not lost with a response that never arrived. Each shows its analysis, whether it is complete or partial, whether it simulated the circuit as drawn, how much of the requested range it covered, and whether the saved schematic or its model files have changed since. The newest 20 runs of each project are kept.",
        json!({
            "type": "object",
            "properties": {
                "schematic": { "type": "string", "description": "Root .kicad_sch path" },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": stored::MAX_LISTED_RUNS,
                    "default": stored::DEFAULT_LISTED_RUNS,
                    "description": "Most runs to list; the total is always complete"
                }
            },
            "required": ["schematic"]
        }),
        |args, ctx| async move { handle_list_simulation_runs(args, ctx).await }
    ), tool!(
        "measure_simulation_run",
        "Work out more measurements and signal summaries on a stored run without simulating again. Measurements take the same kinds as run_simulation, and one that does not fit the run's analysis is refused. Read run_status and simulated_circuit: a measurement on a partial run covers only the data it has, and one on a modified circuit does not describe the schematic as drawn. inputs_changed_since_run says whether the saved schematic differs from what the run used. A dc_sweep with two sweeps takes no measurements.",
        json!({
            "type": "object",
            "properties": {
                "run_id": {
                    "type": "string",
                    "pattern": "^[0-9a-f]{12}-[0-9]{8}T[0-9]{6}-[0-9a-f]{4}$",
                    "description": "The run_id from run_simulation or list_simulation_runs"
                },
                "signals": {
                    "type": "array",
                    "maxItems": MAX_SIGNALS,
                    "items": { "type": "string" },
                    "description": "Signals to summarise: v(NET), v(NET1,NET2), i(REF), i(REF:PIN) or p(REF), which only a run that asked for it has saved"
                },
                "measurements": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_MEASUREMENTS,
                    "items": measurement_schema(),
                    "description": "Values to work out from the run. A kind that does not fit the run's analysis is refused"
                },
                "envelope_points": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_ENVELOPE_POINTS,
                    "default": 0,
                    "description": "Transient runs only. Buckets of min and max to add to each summary"
                }
            },
            "required": ["run_id", "measurements"]
        }),
        |args, ctx| async move { handle_measure_simulation_run(args, ctx).await }
    ), tool!(
        "plot_simulation_run",
        "Draw signals of a stored run as an SVG file and return its path, without simulating again. A transient, a dc_sweep with one sweep or an ac sweep is drawn against its own axis (an ac sweep as gain in dB and phase against log frequency); a dc_sweep with two sweeps draws up to eight steps of the second as curves, for up to four signals; an operating point is refused. Traces are cut to an envelope, so the file stays small however long the run. The picture carries a banner when the run was partial, simulated a modified circuit, or has changed inputs, and the result repeats run_status, simulated_circuit and inputs_changed_since_run: relay them. The file is a viewer or browser file, not an inline image.",
        json!({
            "type": "object",
            "properties": {
                "run_id": {
                    "type": "string",
                    "pattern": "^[0-9a-f]{12}-[0-9]{8}T[0-9]{6}-[0-9a-f]{4}$",
                    "description": "The run_id from run_simulation or list_simulation_runs"
                },
                "signals": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": plots::MAX_PLOT_SIGNALS,
                    "items": { "type": "string" },
                    "description": "Signals to draw: v(NET), v(NET1,NET2), i(REF), i(REF:PIN) or p(REF), which only a run that asked for it has saved. One that the run does not hold is refused as invalid_argument"
                },
                "width_px": {
                    "type": "integer",
                    "minimum": plot::MIN_WIDTH,
                    "maximum": plot::MAX_WIDTH,
                    "default": plot::DEFAULT_WIDTH,
                    "description": "Width of the picture in pixels; its height follows from what it holds"
                }
            },
            "required": ["run_id", "signals"]
        }),
        |args, ctx| async move { handle_plot_simulation_run(args, ctx).await }
    )]
}

/// Refuse a path that is missing, or a sheet inside another project.
fn check_root_schematic(schematic: &Path) -> Result<(), CallToolResult> {
    if !schematic.is_file() {
        return Err(CallToolResult::error_kind(
            ToolErrorKind::FileNotFound {
                path: schematic.display().to_string(),
            },
            format!("Schematic not found: {}", schematic.display()),
        ));
    }
    match sch_export::owning_project_root(schematic) {
        Ok(None) => Ok(()),
        Ok(Some(root)) => {
            let reason = format!(
                "{} is a sheet inside the project rooted at {}. KiCad would treat it as the \
                 root and simulate only part of the design: use {}.",
                schematic.display(),
                root.display(),
                root.display()
            );
            Err(CallToolResult::error_kind(
                ToolErrorKind::InvalidArgument {
                    field: "schematic".to_string(),
                    reason: reason.clone(),
                },
                reason,
            ))
        }
        Err(error) => Err(error.into_tool_result()),
    }
}

async fn handle_check_simulation_readiness(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let schematic = get_path(args, "schematic")?;
    let net_filter = args["net_filter"].as_str().unwrap_or("");
    let limit = args["limit"]
        .as_u64()
        .unwrap_or(DEFAULT_LIMIT)
        .min(MAX_LIMIT) as usize;

    if let Err(refusal) = check_root_schematic(&schematic) {
        return Ok(refusal);
    }

    let engine = probe_engine(
        ctx.simulation.get().map(|r| r.as_ref()),
        &ctx.config.kicad_cli,
        PROBE_TIMEOUT,
    )
    .await;
    let model_dirs = ctx
        .simulation
        .get()
        .map(|runtime| runtime.model_dirs().to_vec())
        .unwrap_or_default();
    let testbench = Testbench::from_args(&args["testbench"]);
    let allow_digital_models = ctx
        .simulation
        .get()
        .is_none_or(|runtime| runtime.allows_digital_models());
    let inputs = DesignInputs {
        model_dirs: &model_dirs,
        testbench: &testbench,
        allow_digital_models,
        code_model_count: engine.code_model_count,
        export_timeout: EXPORT_TIMEOUT,
    };
    let design = inspect_design(&ctx.config.kicad_cli, &schematic, &inputs).await;
    Ok(CallToolResult::json(&readiness_report(
        &engine, &design, net_filter, limit,
    )))
}

fn simulation_failure(
    stage: &'static str,
    reason: &'static str,
    log_path: Option<String>,
    items: Vec<String>,
    message: impl Into<String>,
) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::SimulationFailed {
            stage,
            reason,
            log_path,
            items,
        },
        message,
    )
}

fn failed_outcome(result: CallToolResult, target: &str) -> CallToolResult {
    let outcome = outcome::summary(
        OutcomeStatus::Failed,
        target,
        SOURCE,
        FIXED_OUTCOME_UNITS,
        0,
        FIXED_OUTCOME_UNITS,
        Some(outcome::retry_whole_request()),
    );
    outcome::attach(result, outcome)
}

async fn handle_run_simulation(args: &Value, ctx: &ToolContext) -> anyhow::Result<CallToolResult> {
    let schematic = get_path(args, "schematic")?;
    let target = schematic.display().to_string();
    let limit = args["limit"]
        .as_u64()
        .unwrap_or(DEFAULT_ROW_LIMIT)
        .clamp(1, MAX_LIMIT) as usize;
    let timeout = args["timeout_seconds"]
        .as_u64()
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
        .clamp(1, MAX_TIMEOUT_SECONDS);
    let max_raw_bytes = args["max_raw_bytes"]
        .as_u64()
        .unwrap_or(DEFAULT_RAW_BYTES)
        .clamp(MIN_RAW_BYTES, MAX_RAW_LIMIT);
    let envelope_points = args["envelope_points"]
        .as_u64()
        .unwrap_or(0)
        .min(MAX_ENVELOPE_POINTS) as usize;
    let testbench = Testbench::from_args(&args["testbench"]);
    let wanted = Wanted::from_args(
        &args["analysis"],
        &args["signals"],
        &args["measurements"],
        envelope_points,
    );
    let (analysis, wanted) = match wanted {
        Ok(wanted) => wanted,
        Err(refusal) => return Ok(failed_outcome(refusal, &target)),
    };
    let request = RunArguments {
        schematic: &schematic,
        analysis,
        wanted,
        testbench: &testbench,
        limit,
        budget: Duration::from_secs(timeout),
        max_raw_bytes,
    };
    Ok(match run_analysis(&request, ctx).await {
        Ok(result) => result,
        Err(refusal) => failed_outcome(refusal, &target),
    })
}

async fn handle_list_simulation_runs(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let schematic = get_path(args, "schematic")?;
    let limit = args["limit"]
        .as_u64()
        .unwrap_or(stored::DEFAULT_LISTED_RUNS)
        .clamp(1, stored::MAX_LISTED_RUNS) as usize;
    Ok(stored::list(ctx, &schematic, limit).unwrap_or_else(|refusal| refusal))
}

async fn handle_measure_simulation_run(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let envelope_points = args["envelope_points"]
        .as_u64()
        .unwrap_or(0)
        .min(MAX_ENVELOPE_POINTS) as usize;
    let question = stored::Question {
        run_id: args["run_id"].as_str().unwrap_or_default(),
        signals: &args["signals"],
        measurements: &args["measurements"],
        envelope_points,
    };
    Ok(match stored::measure(ctx, &question) {
        Ok(result) => result,
        Err((refusal, target)) => {
            stored::failed_measure_outcome(refusal, &target, question.requested())
        }
    })
}

async fn handle_plot_simulation_run(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let question = plots::Question {
        run_id: args["run_id"].as_str().unwrap_or_default(),
        signals: &args["signals"],
        width_px: &args["width_px"],
    };
    Ok(match plots::plot(ctx, &question) {
        Ok(result) => result,
        Err((refusal, target)) => plots::failed_outcome(refusal, &target),
    })
}

struct RunArguments<'a> {
    schematic: &'a Path,
    analysis: Analysis,
    wanted: Wanted,
    testbench: &'a Testbench,
    limit: usize,
    budget: Duration,
    max_raw_bytes: u64,
}

fn invalid(field: impl Into<String>, reason: impl Into<String>) -> CallToolResult {
    let reason = reason.into();
    CallToolResult::error_kind(
        ToolErrorKind::InvalidArgument {
            field: field.into(),
            reason: reason.clone(),
        },
        reason,
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Transient {
    stop: f64,
    max_step: Option<f64>,
    start_save: Option<f64>,
    use_initial_conditions: bool,
}

impl Transient {
    /// Every value here reaches the deck, so each must be a number the engine can
    /// take. Serde leaves only finite numbers, and a nonsense one fails loudly.
    fn from_args(analysis: &Value) -> Result<Self, CallToolResult> {
        let number = |field: &str| -> Result<Option<f64>, CallToolResult> {
            let value = &analysis[field];
            if value.is_null() {
                return Ok(None);
            }
            match value.as_f64() {
                Some(number) if number.is_finite() => Ok(Some(number)),
                _ => Err(invalid(
                    format!("analysis.{field}"),
                    format!("{field} must be a finite number"),
                )),
            }
        };
        let Some(stop) = number("stop_time_seconds")? else {
            return Err(invalid(
                "analysis.stop_time_seconds",
                "A transient needs stop_time_seconds",
            ));
        };
        if !STOP_TIME_RANGE.contains(&stop) {
            return Err(invalid(
                "analysis.stop_time_seconds",
                format!(
                    "stop_time_seconds must be from {:e} to {:e}",
                    STOP_TIME_RANGE.start(),
                    STOP_TIME_RANGE.end()
                ),
            ));
        }
        let max_step = number("max_step_seconds")?;
        if max_step.is_some_and(|step| step < stop * f64::EPSILON || step > stop) {
            return Err(invalid(
                "analysis.max_step_seconds",
                "max_step_seconds must be above 0, within reach of the stop time, and no \
                 larger than it",
            ));
        }
        let start_save = number("start_save_seconds")?;
        if start_save.is_some_and(|start| start < 0.0 || start >= stop) {
            return Err(invalid(
                "analysis.start_save_seconds",
                "start_save_seconds must fall before the stop time",
            ));
        }
        Ok(Self {
            stop,
            max_step,
            start_save,
            use_initial_conditions: analysis["use_initial_conditions"]
                .as_bool()
                .unwrap_or(false),
        })
    }

    /// The arguments again, for `Transient::from_args` to read back.
    fn record(&self) -> Value {
        json!({
            "kind": "transient",
            "stop_time_seconds": self.stop,
            "max_step_seconds": self.max_step,
            "start_save_seconds": self.start_save,
            "use_initial_conditions": self.use_initial_conditions,
        })
    }

    /// Only numbers formatted here, never text from a caller.
    fn line(&self) -> String {
        let mut line = format!(".tran {:e} {:e}", self.stop / PRINT_STEPS, self.stop);
        // Left to itself, ngspice sizes the largest step from the saved stretch
        // alone, so a late `start_save_seconds` would crawl through the whole run.
        if self.start_save.is_some() || self.max_step.is_some() {
            let step = self.max_step.unwrap_or(self.stop / PRINT_STEPS);
            let start = self.start_save.unwrap_or(0.0);
            line.push_str(&format!(" {start:e} {step:e}"));
        }
        if self.use_initial_conditions {
            line.push_str(" uic");
        }
        line
    }

    fn reached(plot: &Plot) -> Option<f64> {
        plot.real(plot.variable("time")?).last()
    }

    fn first_point(plot: &Plot) -> Option<f64> {
        plot.real(plot.variable("time")?).next()
    }

    fn covered(&self, plot: &Plot) -> bool {
        !plot.is_truncated()
            && Self::reached(plot)
                .is_some_and(|last| last >= self.stop * (1.0 - COVERAGE_TOLERANCE))
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Analysis {
    OperatingPoint,
    Transient(Transient),
    Ac(Ac),
    DcSweep(DcSweep),
}

impl Analysis {
    fn from_args(analysis: &Value) -> Result<Self, CallToolResult> {
        match analysis["kind"].as_str().unwrap_or_default() {
            "operating_point" => Ok(Self::OperatingPoint),
            "transient" => Transient::from_args(analysis).map(Self::Transient),
            "ac" => Ac::from_args(analysis).map(Self::Ac),
            "dc_sweep" => DcSweep::from_args(analysis).map(Self::DcSweep),
            other => Err(invalid(
                "analysis.kind",
                format!("Unknown analysis kind: {other}"),
            )),
        }
    }

    /// What a stored run keeps of the analysis. Numbers only, and for a sweep the
    /// element as the deck spells it, so a later call needs no deck.
    fn record(&self) -> Value {
        match self {
            Self::OperatingPoint => json!({ "kind": "operating_point" }),
            Self::Transient(transient) => transient.record(),
            Self::Ac(ac) => ac.record(),
            Self::DcSweep(sweep) => sweep.record(),
        }
    }

    /// The analysis of a stored run. None if the record does not hold one.
    fn from_record(record: &Value) -> Option<Self> {
        match record["kind"].as_str()? {
            "operating_point" => Some(Self::OperatingPoint),
            "transient" => Transient::from_args(record).ok().map(Self::Transient),
            "ac" => Ac::from_args(record).ok().map(Self::Ac),
            "dc_sweep" => DcSweep::restore(record).map(Self::DcSweep),
            _ => None,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::OperatingPoint => "operating_point",
            Self::Transient(_) => "transient",
            Self::Ac(_) => "ac",
            Self::DcSweep(_) => "dc_sweep",
        }
    }

    fn line(&self) -> String {
        match self {
            Self::OperatingPoint => OPERATING_POINT.to_string(),
            Self::Transient(transient) => transient.line(),
            Self::Ac(ac) => ac.line(),
            Self::DcSweep(sweep) => sweep.line(),
        }
    }

    fn covered(&self, plot: &Plot) -> bool {
        match self {
            Self::OperatingPoint => operating_point_covered(plot),
            Self::Transient(transient) => transient.covered(plot),
            Self::Ac(ac) => ac.covered(plot),
            Self::DcSweep(sweep) => sweep.covered(plot),
        }
    }

    fn uses_initial_conditions(&self) -> bool {
        matches!(self, Self::Transient(run) if run.use_initial_conditions)
    }

    /// An AC sweep makes complex values, which take twice the room of a real one.
    fn makes_complex_data(&self) -> bool {
        matches!(self, Self::Ac(_))
    }

    /// What measurements are taken along. Not known for an operating point, and
    /// not for a DC sweep until its target is read from the deck.
    fn axis(&self) -> Option<AxisSpec> {
        match self {
            Self::OperatingPoint => None,
            Self::Transient(run) => Some(AxisSpec::time(run.stop, run.start_save.unwrap_or(0.0))),
            Self::Ac(ac) => Some(AxisSpec::frequency(ac.start, ac.stop)),
            Self::DcSweep(sweep) => {
                let inner = sweep.inner();
                Some(AxisSpec::sweep(inner.start, inner.stop, ""))
            }
        }
    }

    /// Read what the deck holds that the analysis names. A refusal here stops the
    /// run before the engine does.
    fn prepared(
        self,
        deck: &FlatDeck,
        compatibility: Compatibility,
        elements: &[ElementEntry],
    ) -> Result<Self, CallToolResult> {
        match self {
            Self::Ac(_) if !element::has_ac_stimulus(deck, compatibility.dollar_is_comment()) => {
                Err(simulation_failure(
                    "preflight",
                    "no_ac_stimulus",
                    None,
                    Vec::new(),
                    "No voltage or current source has an AC value, so every signal would read \
                     zero. Give a source an AC value, such as `AC 1`, or add a source through \
                     the testbench.",
                ))
            }
            Self::DcSweep(sweep) => sweep
                .resolve(&top_level_elements(deck), elements)
                .map(Self::DcSweep),
            other => Ok(other),
        }
    }
}

/// What the caller wants back from a run, checked before anything starts.
#[derive(Debug, Clone, Default)]
struct Wanted {
    /// As written, to repeat in the result.
    signals: Vec<(String, Signal)>,
    measurements: Vec<Measurement>,
    envelope_points: usize,
}

impl Wanted {
    fn from_args(
        analysis: &Value,
        signals: &Value,
        measurements: &Value,
        envelope_points: usize,
    ) -> Result<(Analysis, Self), CallToolResult> {
        let analysis = Analysis::from_args(analysis)?;
        let wanted = Self::for_analysis(&analysis, signals, measurements, envelope_points)?;
        Ok((analysis, wanted))
    }

    /// What is asked of an analysis that is already known, as for a stored run.
    fn for_analysis(
        analysis: &Analysis,
        signals: &Value,
        measurements: &Value,
        envelope_points: usize,
    ) -> Result<Self, CallToolResult> {
        let no_items = Vec::new();
        let signals = signals.as_array().unwrap_or(&no_items);
        let measurements = measurements.as_array().unwrap_or(&no_items);
        if signals.len() > MAX_SIGNALS {
            return Err(invalid("signals", format!("At most {MAX_SIGNALS} signals")));
        }
        if measurements.len() > MAX_MEASUREMENTS {
            return Err(invalid(
                "measurements",
                format!("At most {MAX_MEASUREMENTS} measurements"),
            ));
        }
        let Some(axis) = analysis.axis() else {
            let fits = signals.is_empty() && measurements.is_empty() && envelope_points == 0;
            return match fits {
                true => Ok(Self::default()),
                false => Err(invalid(
                    "measurements",
                    "signals, measurements and envelope_points need a transient, an ac or a \
                     dc_sweep analysis",
                )),
            };
        };
        if envelope_points > 0 && !matches!(analysis, Analysis::Transient(_)) {
            return Err(invalid(
                "envelope_points",
                "envelope_points applies to a transient",
            ));
        }
        if envelope_points > 0 && signals.is_empty() {
            return Err(invalid("envelope_points", "envelope_points needs signals"));
        }
        // Along two sweeps a value has two places, and a window, a crossing or a
        // single value over the grid would not say which.
        let grid = matches!(analysis, Analysis::DcSweep(sweep) if sweep.sweeps.len() > 1);
        if grid && !measurements.is_empty() {
            return Err(invalid(
                "measurements",
                "A dc_sweep with two sweeps takes signals but no measurements: the whole grid is \
                 in the raw file and in results.csv. Use one sweep to measure.",
            ));
        }
        let ac = matches!(analysis, Analysis::Ac(_));
        let signals = signals
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let text = entry.as_str().unwrap_or_default();
                match Signal::parse(text) {
                    Ok(Signal::Power(_)) if ac => Err(invalid(
                        format!("signals[{index}]"),
                        "p() has no value in an ac analysis",
                    )),
                    Ok(signal) => Ok((text.to_string(), signal)),
                    Err(reason) => Err(invalid(format!("signals[{index}]"), reason)),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let measurements = measurements
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                measure::parse(entry, index, &axis).map_err(|bad| invalid(bad.field, bad.reason))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            signals,
            measurements,
            envelope_points,
        })
    }

    fn all_signals(&self) -> impl Iterator<Item = &Signal> {
        let summaries = self.signals.iter().map(|(_, signal)| signal);
        summaries.chain(self.measurements.iter().map(|wanted| &wanted.signal))
    }
}

async fn usable_engine(
    runtime: &SimulationRuntime,
    kicad_cli: &str,
    limit: Duration,
) -> Result<EngineStatus, CallToolResult> {
    let engine = probe_engine(Some(runtime), kicad_cli, limit).await;
    if engine.available {
        return Ok(engine);
    }
    let reason = match engine.probe_status {
        "not_found" => "engine_not_found",
        "timed_out" => "timed_out",
        _ => "engine_load_failed",
    };
    let detail = engine.detail.clone().unwrap_or_default();
    Err(simulation_failure(
        "engine",
        reason,
        None,
        engine.searched_locations,
        format!(
            "The simulator is not usable: {} {detail}",
            engine.probe_status
        ),
    ))
}

fn export_failure(error: &anyhow::Error) -> CallToolResult {
    let message = export_detail(&format!("{error:#}"));
    let reason = if error
        .downcast_ref::<snapshot::SheetOutsideProject>()
        .is_some()
    {
        "sheet_outside_project"
    } else if message.contains("kicad-cli timed out") {
        "netlist_export_timed_out"
    } else {
        "netlist_export_failed"
    };
    simulation_failure("export", reason, None, Vec::new(), message)
}

fn out_of_budget() -> CallToolResult {
    simulation_failure(
        "export",
        "netlist_export_timed_out",
        None,
        Vec::new(),
        "The time budget ran out before the simulator could start.",
    )
}

fn deck_refusal(refusal: &Refusal) -> CallToolResult {
    let place = match refusal.line {
        Some(line) => format!("{} line {line}", refusal.origin),
        None => refusal.origin.clone(),
    };
    simulation_failure(
        "preflight",
        refusal.code.as_str(),
        None,
        vec![format!("{place}: {}", refusal.detail)],
        "The deck was refused before reaching the simulator.",
    )
}

/// A floating group is only wrong for an operating point, where its voltages are
/// arbitrary, so this is the one warning that refuses here.
fn preflight_refusal(preflight: &Preflight, kind: &str) -> Option<CallToolResult> {
    let failed = preflight
        .checks
        .iter()
        .find(|check| check.passed == Some(false));
    if let Some(check) = failed {
        let code = check.code.unwrap_or(check.name);
        let mut items: Vec<String> = check.items.clone();
        items.truncate(MAX_ERROR_ITEMS);
        let message = check.reason.clone().unwrap_or_else(|| {
            format!("Preflight refused this circuit: {code}. See check_simulation_readiness.")
        });
        return Some(simulation_failure("preflight", code, None, items, message));
    }
    // Both solve the circuit at DC, where an isolated group has no defined voltage.
    let dc = matches!(kind, "operating_point" | "dc_sweep");
    let floating = preflight
        .warnings
        .iter()
        .find(|warning| warning.code == "floating_subcircuit" && dc)?;
    Some(simulation_failure(
        "preflight",
        floating.code,
        None,
        floating
            .items
            .iter()
            .take(MAX_ERROR_ITEMS)
            .cloned()
            .collect(),
        floating.detail,
    ))
}

/// The names of the elements at the deck's top level, as the deck spells them.
fn top_level_elements(deck: &FlatDeck) -> Vec<&str> {
    let mut names = Vec::new();
    let mut depth = 0usize;
    for (text, _, _) in deck.lines() {
        let Some(first) = text.split_whitespace().next() else {
            continue;
        };
        let lower = first.to_ascii_lowercase();
        if lower.starts_with(".subckt") {
            depth += 1;
        } else if lower.starts_with(".ends") {
            depth = depth.saturating_sub(1);
        } else if depth == 0 && !first.starts_with(['.', '+']) {
            names.push(first);
        }
    }
    names
}

/// The text of every card that starts with `prefix`, in any case.
fn cards_starting<'a>(deck: &'a FlatDeck, prefix: &'a str) -> impl Iterator<Item = &'a str> {
    deck.lines().filter_map(move |(text, _, _)| {
        let first = text.split_whitespace().next()?;
        first
            .to_ascii_lowercase()
            .starts_with(prefix)
            .then_some(text)
    })
}

/// Whether the deck seeds node voltages. Under `uic` both cards do, through the
/// same vector.
fn sets_node_conditions(deck: &FlatDeck) -> bool {
    [".ic", ".nodeset"]
        .iter()
        .any(|prefix| cards_starting(deck, prefix).next().is_some())
}

/// A caller's own `.probe` would lose the voltages that Konnect's are held back to
/// keep, and it cannot be taken out of a deck that names it on purpose.
fn probe_conflict(deck: &FlatDeck) -> Option<CallToolResult> {
    let items: Vec<String> = cards_starting(deck, ".probe")
        .take(MAX_ERROR_ITEMS)
        .map(str::to_owned)
        .collect();
    (!items.is_empty()).then(|| {
        simulation_failure(
            "preflight",
            "probe_loses_initial_conditions",
            None,
            items,
            "A .probe card moves a device's pin onto a hidden node, so under \
             use_initial_conditions a .ic or .nodeset voltage on that net would be lost. \
             Remove the card, or use IC= on the device.",
        )
    })
}

/// Konnect's own lines after the vetted deck. They hold nothing from a caller:
/// the analysis is numbers formatted here, and a probe names a deck element.
fn own_lines(analysis: &Analysis, save_currents: bool, probes: &[String]) -> Vec<String> {
    let mut lines = vec![SAVE_ALL_VOLTAGES.to_string()];
    if save_currents {
        lines.push(SAVE_ALL_CURRENTS.to_string());
    }
    lines.extend_from_slice(probes);
    lines.push(analysis.line());
    lines
}

fn prepare_run(
    run: &runs::RunFolder,
    deck: &FlatDeck,
    own_lines: &[String],
    request: &RunRequest,
) -> anyhow::Result<()> {
    let own_lines: Vec<&str> = own_lines.iter().map(String::as_str).collect();
    std::fs::write(run.dir.join(DECK_FILE), deck.render_with(&own_lines))?;
    std::fs::write(run.dir.join(REQUEST_FILE), serde_json::to_vec(request)?)?;
    Ok(())
}

async fn run_analysis(
    request: &RunArguments<'_>,
    ctx: &ToolContext,
) -> Result<CallToolResult, CallToolResult> {
    let started = Instant::now();
    check_root_schematic(request.schematic)?;
    let Some(runtime) = ctx.simulation.get() else {
        return Err(simulation_failure(
            "engine",
            "engine_load_failed",
            None,
            Vec::new(),
            "This process does not carry the simulation worker (embedded or library use).",
        ));
    };
    let probe_limit = request.budget.min(PROBE_TIMEOUT);
    let engine = usable_engine(runtime, &ctx.config.kicad_cli, probe_limit).await?;
    let export_budget = request.budget.saturating_sub(started.elapsed());
    let inputs = DesignInputs {
        model_dirs: runtime.model_dirs(),
        testbench: request.testbench,
        allow_digital_models: runtime.allows_digital_models(),
        code_model_count: engine.code_model_count,
        export_timeout: export_budget.min(EXPORT_TIMEOUT),
    };
    let inspected = tokio::time::timeout(
        export_budget,
        inspect_design(&ctx.config.kicad_cli, request.schematic, &inputs),
    )
    .await;
    let design = match inspected {
        Ok(design) => design.map_err(|error| export_failure(&error))?,
        Err(_) => return Err(out_of_budget()),
    };
    let deck = design.deck.as_ref().map_err(deck_refusal)?;
    if let Some(refusal) = design
        .preflight
        .as_ref()
        .and_then(|preflight| preflight_refusal(preflight, request.analysis.kind()))
    {
        return Err(refusal);
    }
    let analysis =
        request
            .analysis
            .clone()
            .prepared(deck, design.compatibility, &design.elements)?;
    // A probe moves a device's pin onto a hidden node, and under `uic` a device
    // takes its start from its own pins: a voltage set on the net would be lost.
    let keep_conditions = analysis.uses_initial_conditions() && sets_node_conditions(deck);
    if keep_conditions {
        if let Some(refusal) = probe_conflict(deck) {
            return Err(refusal);
        }
    }
    let Some(budget) = request
        .budget
        .checked_sub(started.elapsed())
        .filter(|budget| !budget.is_zero())
    else {
        return Err(out_of_budget());
    };

    let store_failure =
        |error: String| simulation_failure("run", "run_store_failed", None, Vec::new(), error);
    let library = PathBuf::from(engine.library_path.clone().unwrap_or_default());
    let run = runtime
        .state_root()
        .map_err(|error| error.to_string())
        .and_then(|root| runs::create(&root, request.schematic).map_err(|error| error.to_string()))
        .map_err(|error| store_failure(format!("Failed to create a run folder: {error}")))?;
    let worker_request = RunRequest {
        protocol: PROTOCOL_VERSION,
        library: library.clone(),
        code_models: engine.code_models_dir.as_ref().map(PathBuf::from),
        load_digital: runtime.allows_digital_models(),
        compatibility: design.compatibility.ngbehavior.map(str::to_owned),
        input_path: Some(design.project_dir.clone()),
        budget_ms: budget.as_millis() as u64,
        max_raw_bytes: request.max_raw_bytes,
        complex_data: analysis.makes_complex_data(),
    };
    let event_nodes = design
        .preflight
        .as_ref()
        .is_some_and(|preflight| preflight.event_nodes);
    let probes = match keep_conditions {
        true => Vec::new(),
        false => signals::power_probes(
            request.wanted.all_signals(),
            Names {
                nets: &design.nets,
                elements: &design.elements,
            },
            &top_level_elements(deck),
        ),
    };
    let own_lines = own_lines(&analysis, !event_nodes && !keep_conditions, &probes);
    prepare_run(&run, deck, &own_lines, &worker_request)
        .map_err(|error| store_failure(format!("Failed to prepare the run: {error:#}")))?;
    let end = run_worker(runtime, &run.dir, &library, budget)
        .await
        .map_err(|error| {
            simulation_failure(
                "engine",
                "engine_load_failed",
                None,
                Vec::new(),
                format!("{error:#}"),
            )
        })?;

    let finish = RunEnd {
        keep_conditions,
        analysis: &analysis,
        request,
        run: &run,
        design: &design,
        deck,
        engine: &engine,
        end: &end,
    };
    finish.report()
}

/// Everything known once the worker has gone.
struct RunEnd<'a> {
    /// Probes were left out so `.ic` voltages hold.
    keep_conditions: bool,
    /// The analysis as it ran, with a sweep's targets read from the deck.
    analysis: &'a Analysis,
    request: &'a RunArguments<'a>,
    run: &'a runs::RunFolder,
    design: &'a Design,
    deck: &'a FlatDeck,
    engine: &'a EngineStatus,
    end: &'a WorkerEnd,
}

/// One operating-point value. `net` is the name KiCad shows, where exactly one
/// net maps to this node.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct OperatingPointRow {
    signal: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    net: Option<String>,
    value: f64,
    unit: &'static str,
}

/// Units come from the signal, because ngspice calls `v(r1:r)` a voltage whatever
/// it holds.
fn signal_unit(signal: &str) -> &'static str {
    if signal.ends_with(":power") {
        "W"
    } else if signal.starts_with("i(") || signal.ends_with("#branch") {
        "A"
    } else if signal.starts_with("v(") {
        "V"
    } else {
        ""
    }
}

fn kicad_net(signal: &str, nets: &[NetEntry]) -> Option<String> {
    let node = signal.strip_prefix("v(")?.strip_suffix(')')?;
    let mut names = nets
        .iter()
        .filter(|net| preflight::engine_node(&net.spice_name) == node)
        .map(|net| net.kicad_name.clone());
    let first = names.next();
    first.filter(|_| names.next().is_none())
}

/// Top-level signals first, so the cut never hides them behind subcircuit nodes.
fn operating_point_rows(plot: &Plot, nets: &[NetEntry]) -> Vec<OperatingPointRow> {
    let mut rows: Vec<OperatingPointRow> = plot
        .variables
        .iter()
        .enumerate()
        .filter_map(|(index, variable)| {
            Some(OperatingPointRow {
                signal: variable.name.clone(),
                net: kicad_net(&variable.name, nets),
                value: plot.real(index).next()?,
                unit: signal_unit(&variable.name),
            })
        })
        .collect();
    rows.sort_by_key(|row| row.signal.contains('.'));
    rows
}

fn operating_point_covered(plot: &Plot) -> bool {
    plot.points == 1 && !plot.is_truncated() && !plot.variables.is_empty()
}

/// Split as the engine would rewrite them: it turns a non-ASCII byte into `_`.
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
}

/// What the engine said went wrong, and how many lines were held back. It rewrites
/// deck lines, quotes pieces of them and cuts long words, and text from a model
/// file is not ours to repeat. So a line stays in the log if any of its words
/// starts a word found only in model files. Short words are not evidence: `is`
/// comes from every diode model and from most of the engine's sentences. A value
/// computed from a model parameter cannot be recognised, and may still show.
fn engine_complaints(log: &str, deck: &FlatDeck) -> (Vec<String>, usize) {
    let mut model_words = BTreeSet::new();
    let mut own_words = HashSet::new();
    for (text, _, included) in deck.lines() {
        if included {
            model_words.extend(words(text));
        } else {
            own_words.extend(words(text));
        }
    }
    model_words.retain(|word| !own_words.contains(word));
    let quotes_model = |line: &str| {
        words(line)
            .filter(|word| word.len() >= MIN_PRIVATE_WORD)
            .any(|word| {
                let next = model_words.range(word.clone()..).next();
                next.is_some_and(|model_word: &String| model_word.starts_with(&word))
            })
    };
    let ours = log.lines().take_while(|line| !line.contains(STATUS_NAME));
    let lines = ours
        .filter_map(|line| line.strip_prefix("stderr "))
        .filter(|line| !line.starts_with(SPINIT_NOTICE) && !line.starts_with(ENGINE_NOTE))
        .filter(|line| !HALT_NOTICES.iter().any(|notice| line.starts_with(notice)));
    let (hidden, shown): (Vec<&str>, Vec<&str>) = lines.partition(|line| quotes_model(line));
    let shown = shown
        .iter()
        .take(MAX_LOG_ITEMS)
        .map(|line| line.chars().take(MAX_LOG_ITEM_CHARS).collect())
        .collect();
    (shown, hidden.len())
}

/// The complaints as items, naming any lines left in the log.
fn complaint_items(log: &str, deck: &FlatDeck) -> Vec<String> {
    let (mut items, hidden) = engine_complaints(log, deck);
    if hidden > 0 {
        items.push(format!("{hidden} {MODEL_LINES_NOTE}"));
    }
    items
}

fn failure_message(reason: &str) -> &'static str {
    match reason {
        "engine_crashed" => "The simulator process died. The server is unaffected.",
        "engine_failed" => "The simulator stopped on a fatal error. The server is unaffected.",
        "engine_load_failed" => "The simulator could not start this run.",
        "circuit_not_parsed" => "The simulator could not parse the circuit.",
        "no_convergence" => "The analysis did not converge and left no data.",
        "timed_out" => "The time budget ran out before the simulator left any data.",
        "size_limit" => "The size limit stopped the run before it left data that could be read.",
        "results_unreadable" => "The simulator finished but its results could not be read.",
        _ => "The simulation failed.",
    }
}

/// Signals and measurements read from a run.
#[derive(Default)]
struct Gathered {
    summaries: Vec<Value>,
    measurements: Vec<Value>,
    shared_nets: BTreeSet<String>,
    signal_missing: bool,
    failed_measurements: usize,
}

/// What a run's signals and measurements come to. A stored run is read the same
/// way as a fresh one, from the plot and the names that were kept with it.
struct Evaluation<'a> {
    names: Names<'a>,
    wanted: &'a Wanted,
}

impl Evaluation<'_> {
    /// Look up a measurement's signal. With a `reference_signal` it is the ratio of
    /// the two, worked out as complex numbers.
    fn effective_signal(
        &self,
        plot: &Plot,
        request: &Measurement,
    ) -> Result<signals::Resolved, NotFound> {
        let mut found = signals::resolve(plot, &request.signal, self.names)?;
        let Some((_, reference)) = &request.reference_signal else {
            return Ok(found);
        };
        let other = signals::resolve(plot, reference, self.names)?;
        let (real, imaginary) = ac::divide(
            (&found.values, &found.imaginary),
            (&other.values, &other.imaginary),
        );
        found.values = real;
        found.imaginary = imaginary;
        found.shared_nets.extend(other.shared_nets);
        Ok(found)
    }

    /// Resolve every signal and measurement, and say what each analysis made of it.
    fn gather(
        &self,
        plot: &Plot,
        partial: bool,
        measurements: &[Measurement],
        summary: impl Fn(&str, &Signal, &signals::Resolved) -> Value,
        measurement: impl Fn(&Measurement, &signals::Resolved) -> Value,
    ) -> Gathered {
        let mut gathered = Gathered::default();
        for (text, signal) in &self.wanted.signals {
            match signals::resolve(plot, signal, self.names) {
                Ok(found) => {
                    gathered
                        .shared_nets
                        .extend(found.shared_nets.iter().cloned());
                    gathered.summaries.push(summary(text, signal, &found));
                }
                Err(not_found) => {
                    gathered.signal_missing = true;
                    let report = json!({ "signal": text, "finding": "signal_not_found" });
                    gathered.summaries.push(with_pins(report, &not_found));
                }
            }
        }
        for request in measurements {
            match self.effective_signal(plot, request) {
                Ok(found) => {
                    gathered.failed_measurements += usize::from(partial);
                    gathered
                        .shared_nets
                        .extend(found.shared_nets.iter().cloned());
                    gathered.measurements.push(measurement(request, &found));
                }
                Err(not_found) => {
                    gathered.failed_measurements += 1;
                    let report = measure::missing_report(request, partial);
                    gathered.measurements.push(with_pins(report, &not_found));
                }
            }
        }
        gathered
    }

    /// Put what was gathered in the body, and count the outcome units that failed.
    /// The "range covered" unit also stands for the signals asked for: `signals`
    /// has no units of its own.
    fn finish_body(&self, body: &mut Value, gathered: Gathered, partial: bool) -> usize {
        let wanted = self.wanted;
        if !wanted.measurements.is_empty() {
            body["measurements"] = json!(gathered.measurements);
        }
        if !wanted.signals.is_empty() {
            body["signal_summaries"] = json!(gathered.summaries);
        }
        if let Some(warnings) = body["warnings"].as_array_mut() {
            if !gathered.shared_nets.is_empty() {
                warnings.push(json!({
                    "code": "nets_share_engine_node",
                    "detail": "ngspice reads these nets as one node, so their signals are the same.",
                    "items": gathered.shared_nets,
                }));
            }
        }
        usize::from(partial || gathered.signal_missing) + gathered.failed_measurements
    }

    fn transient(&self, plot: &Plot, partial: bool) -> Gathered {
        let wanted = self.wanted;
        let time: Vec<f64> = plot
            .variable("time")
            .map(|index| plot.real(index).collect())
            .unwrap_or_default();
        self.gather(
            plot,
            partial,
            &wanted.measurements,
            |text, signal, found| {
                measure::summary(
                    &time,
                    &found.values,
                    text,
                    signal.unit(),
                    wanted.envelope_points,
                    partial,
                )
            },
            |request, found| {
                measure::measure(&time, &found.values, request).report(request, partial)
            },
        )
    }

    fn ac(&self, ac: &Ac, plot: &Plot, partial: bool) -> Gathered {
        let freq: Vec<f64> = plot
            .variable("frequency")
            .map(|index| plot.real(index).collect())
            .unwrap_or_default();
        let logarithmic = ac.scale.is_logarithmic();
        self.gather(
            plot,
            partial,
            &self.wanted.measurements,
            |text, _, found| ac::summary(&freq, &found.values, &found.imaginary, text, partial),
            |request, found| match ac::Response::new(
                &freq,
                &found.values,
                &found.imaginary,
                logarithmic,
            ) {
                Some(response) => ac::measure(&response, request, partial),
                None => measure::missing_report(request, partial),
            },
        )
    }

    fn sweep(&self, sweep: &DcSweep, plot: &Plot, partial: bool) -> Gathered {
        let inner = sweep.inner();
        // Falling sweeps are measured on a rising axis, and put back on the way out.
        let direction = AxisSpec::sweep(inner.start, inner.stop, inner.unit).direction();
        let axis: Vec<f64> = DcSweep::axis(plot).iter().map(|x| x * direction).collect();
        let measurements: Vec<Measurement> = self
            .wanted
            .measurements
            .iter()
            .cloned()
            .map(|mut request| {
                request.axis.unit = inner.unit.to_string();
                request
            })
            .collect();
        self.gather(
            plot,
            partial,
            &measurements,
            |text, signal, found| {
                measure::sweep_summary(&found.values, text, signal.unit(), partial)
            },
            |request, found| {
                measure::measure(&axis, &found.values, request).report(request, partial)
            },
        )
    }
}

impl RunEnd<'_> {
    fn path(&self, file: &str) -> String {
        self.run.dir.join(file).display().to_string()
    }

    fn evaluation(&self) -> Evaluation<'_> {
        Evaluation {
            names: Names {
                nets: &self.design.nets,
                elements: &self.design.elements,
            },
            wanted: &self.request.wanted,
        }
    }

    fn status(&self) -> Option<&RunStatus> {
        match self.end {
            WorkerEnd::Reported(status) => Some(status),
            _ => None,
        }
    }

    fn engine_log(&self) -> String {
        std::fs::read_to_string(self.run.dir.join(LOG_FILE)).unwrap_or_default()
    }

    fn failure(&self, reason: &'static str) -> CallToolResult {
        let stage = match reason {
            "engine_load_failed" => "engine",
            "results_unreadable" => "results",
            _ => "run",
        };
        let mut items = complaint_items(&self.engine_log(), self.deck);
        if let WorkerEnd::Died(how) = self.end {
            items.insert(0, how.clone());
        }
        if let Some(error) = self.status().and_then(|status| status.load_error.as_ref()) {
            items.insert(0, error.chars().take(MAX_LOG_ITEM_CHARS).collect());
        }
        simulation_failure(
            stage,
            reason,
            Some(self.path(LOG_FILE)),
            items,
            format!("{} Run {}.", failure_message(reason), self.run.id),
        )
    }

    fn report(&self) -> Result<CallToolResult, CallToolResult> {
        let results = self
            .status()
            .map(|status| read_results(&self.run.dir, status));
        let verdict = classify(self.end, results.as_ref(), |plot| {
            self.analysis.covered(plot)
        });
        let stopped = match verdict {
            Verdict::Failed(reason) => return Err(self.failure(reason)),
            Verdict::Partial(reason) => Some(reason),
            Verdict::Complete => None,
        };
        let Some(Ok(plot)) = &results else {
            return Err(self.failure("results_unreadable"));
        };
        let (mut body, failed) = match self.analysis {
            Analysis::OperatingPoint => (
                self.operating_point_body(plot, stopped),
                usize::from(stopped.is_some()),
            ),
            Analysis::Transient(transient) => self.transient_body(transient, plot, stopped),
            Analysis::Ac(ac) => self.ac_body(ac, plot, stopped),
            Analysis::DcSweep(sweep) => self.sweep_body(sweep, plot, stopped),
        };
        self.add_table(&mut body, plot);
        self.keep_record(&mut body, stopped.is_some());
        let requested = FIXED_OUTCOME_UNITS + self.request.wanted.measurements.len();
        let status = match failed {
            0 => OutcomeStatus::Complete,
            _ => OutcomeStatus::Partial,
        };
        let outcome = outcome::summary(
            status,
            self.request.schematic.display().to_string(),
            SOURCE,
            requested,
            requested - failed,
            failed,
            None,
        );
        Ok(outcome::attach(CallToolResult::json(&body), outcome))
    }

    /// How the plot is written as a table.
    fn table_layout(&self, plot: &Plot) -> Layout {
        match self.analysis {
            Analysis::OperatingPoint => Layout::Table,
            Analysis::DcSweep(sweep) => sweep.table_layout(plot),
            Analysis::Transient(_) | Analysis::Ac(_) => Layout::Series,
        }
    }

    /// Write the plot as CSV beside the raw file. A run is not lost by this failing,
    /// so a refusal or an error is a warning and `csv_path` stays null.
    fn add_table(&self, body: &mut Value, plot: &Plot) {
        let path = self.run.dir.join(CSV_FILE);
        let written = export::write(&path, plot, &self.table_layout(plot));
        match table_problem(&written) {
            None => {
                body["artifacts"]["csv_path"] = json!(self.path(CSV_FILE));
                if let Ok(export::Written::Bytes(bytes)) = written {
                    body["artifacts"]["csv_bytes"] = json!(bytes);
                }
            }
            Some(problem) => {
                body["artifacts"]["csv_path"] = Value::Null;
                if let Some(warnings) = body["warnings"].as_array_mut() {
                    warnings.push(problem);
                }
            }
        }
    }

    /// Save what a later `list_simulation_runs` or `measure_simulation_run` needs.
    fn keep_record(&self, body: &mut Value, partial: bool) {
        let stopped = body.get("stopped").map(|stopped| Stopped {
            reason: stopped["reason"].as_str().unwrap_or_default().to_string(),
            at_point: stopped["at_point"].as_u64().unwrap_or(0) as usize,
            node: stopped["node"].as_str().map(str::to_owned),
        });
        let record = RunRecord::new(RecordFields {
            schematic: self
                .request
                .schematic
                .canonicalize()
                .unwrap_or_else(|_| self.request.schematic.to_path_buf()),
            analysis: self.analysis.record(),
            run_status: if partial {
                RunState::Partial
            } else {
                RunState::Complete
            },
            stopped,
            simulated_circuit: body["simulated_circuit"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            circuit: body["circuit"].clone(),
            result: body["analysis"].clone(),
            coverage_ratio: body["analysis"]["coverage_ratio"].as_f64().unwrap_or(0.0),
            inputs_hash: self.design.inputs_hash.clone(),
            inlined_files: self.deck.inlined_files.clone(),
            nets: self.design.nets.clone(),
            elements: self.design.elements.clone(),
            warnings: carried_warnings(body),
        });
        if let Err(error) = runs::write_record(&self.run.dir, &record) {
            if let Some(warnings) = body["warnings"].as_array_mut() {
                warnings.push(json!({
                    "code": "run_record_not_saved",
                    "detail": format!(
                        "list_simulation_runs and measure_simulation_run cannot use this run: {error}"
                    ),
                }));
            }
        }
    }

    /// The body and how many outcome units failed.
    fn transient_body(
        &self,
        transient: &Transient,
        plot: &Plot,
        stopped: Option<&'static str>,
    ) -> (Value, usize) {
        let partial = stopped.is_some();
        let gathered = self.evaluation().transient(plot, partial);
        let reached = Transient::reached(plot).unwrap_or(0.0);
        let first_point = Transient::first_point(plot).unwrap_or(0.0);
        let analysis = json!({
            "kind": self.analysis.kind(),
            "stop_time_seconds": transient.stop,
            "first_point_seconds": first_point,
            "simulated_to_seconds": reached,
            "coverage_ratio": (reached / transient.stop).min(1.0),
            "points": plot.points,
        });
        let mut body = self.result_body(plot, analysis, stopped);
        if let Some(warnings) = body["warnings"].as_array_mut() {
            let saves_from_start =
                transient.start_save.unwrap_or(0.0) < transient.stop / PRINT_STEPS;
            if first_point > 0.0 && saves_from_start {
                warnings.push(json!({
                    "code": "no_point_at_start",
                    "detail": "This run has no point at time zero, which is how ngspice saves \
                               under use_initial_conditions, so an event at that instant is cut \
                               short: a peak reads low, and an integral misses the first step. \
                               See first_point_seconds.",
                }));
            }
        }
        let failed = self.evaluation().finish_body(&mut body, gathered, partial);
        (body, failed)
    }

    fn ac_body(&self, ac: &Ac, plot: &Plot, stopped: Option<&'static str>) -> (Value, usize) {
        let partial = stopped.is_some();
        let gathered = self.evaluation().ac(ac, plot, partial);
        let mut body = self.result_body(plot, ac.describe(plot), stopped);
        let reached = Ac::reached(plot).unwrap_or(0.0);
        if let Some(warnings) = body["warnings"].as_array_mut() {
            if !partial && ac.ends_short(reached) {
                warnings.push(json!({
                    "code": "sweep_ends_before_stop",
                    "detail": "ngspice's frequency grid ends on its last point at or below \
                               stop_hz, here simulated_to_hz. Every point it plans was run.",
                }));
            }
        }
        let failed = self.evaluation().finish_body(&mut body, gathered, partial);
        (body, failed)
    }

    fn sweep_body(
        &self,
        sweep: &DcSweep,
        plot: &Plot,
        stopped: Option<&'static str>,
    ) -> (Value, usize) {
        let partial = stopped.is_some();
        let gathered = self.evaluation().sweep(sweep, plot, partial);
        let mut body = self.result_body(plot, sweep.describe(plot), stopped);
        if let Some(warnings) = body["warnings"].as_array_mut() {
            let short = sweep.shortfalls(plot);
            if !partial && !short.is_empty() {
                warnings.push(json!({
                    "code": "sweep_ends_before_stop",
                    "detail": "The engine adds its step each time, and rounding can cost a \
                               sweep its last point. See simulated_to_value.",
                    "items": short,
                }));
            }
        }
        let failed = self.evaluation().finish_body(&mut body, gathered, partial);
        (body, failed)
    }

    fn operating_point_body(&self, plot: &Plot, stopped: Option<&'static str>) -> Value {
        let rows = operating_point_rows(plot, &self.design.nets);
        std::fs::write(
            self.run.dir.join(OPERATING_POINT_FILE),
            serde_json::to_vec_pretty(&rows).unwrap_or_default(),
        )
        .ok();
        let coverage = if operating_point_covered(plot) {
            1.0
        } else {
            0.0
        };
        let analysis = json!({
            "kind": self.analysis.kind(),
            "coverage_ratio": coverage,
            "points": plot.points,
        });
        let mut body = self.result_body(plot, analysis, stopped);
        body["operating_point"] = json!({
            "total": rows.len(),
            "shown": rows.len().min(self.request.limit),
            "truncated": self.request.limit < rows.len(),
            "rows": rows.iter().take(self.request.limit).collect::<Vec<_>>(),
        });
        body["artifacts"]["table_path"] = json!(self.path(OPERATING_POINT_FILE));
        body
    }

    fn warnings(&self) -> Vec<Value> {
        let mut warnings = Vec::new();
        if let Some(preflight) = &self.design.preflight {
            let not_run = preflight.checks.iter().filter(|check| !check.ran);
            warnings.extend(not_run.map(
                |check| json!({ "code": check.code.unwrap_or(check.name), "detail": check.reason }),
            ));
            let limit = self.request.limit;
            warnings.extend(preflight.warnings.iter().map(|w| warning_report(w, limit)));
            if preflight.event_nodes {
                warnings.push(json!({
                    "code": "device_currents_not_saved",
                    "detail": "ngspice cannot probe device currents next to digital nodes, so \
                               only analog node voltages and voltage source currents are saved.",
                }));
            }
        }
        if self.keep_conditions {
            warnings.push(json!({
                "code": "device_currents_not_saved",
                "detail": "Device probes would lose .ic and .nodeset voltages under \
                           use_initial_conditions, so only node voltages and voltage source \
                           currents are saved.",
            }));
        }
        if !self.engine.code_models_failed.is_empty() {
            warnings.push(json!({
                "code": "code_models_failed",
                "items": self.engine.code_models_failed,
            }));
        }
        // A run can finish after the engine ignored a parameter or changed a value.
        let messages = complaint_items(&self.engine_log(), self.deck);
        if !messages.is_empty() {
            warnings.push(json!({
                "code": "engine_messages",
                "detail": "The engine reported these and carried on. They can change the circuit.",
                "items": messages,
            }));
        }
        if self.run.old_runs_kept > 0 {
            warnings.push(json!({
                "code": "old_runs_kept",
                "detail": "Old runs could not be removed, so they still use disk.",
                "total": self.run.old_runs_kept,
            }));
        }
        warnings
    }

    fn circuit(&self) -> (&'static str, Value) {
        let design = self.design;
        let testbench = self.request.testbench;
        let approximations: Vec<Value> = design
            .preflight
            .iter()
            .flat_map(|preflight| &preflight.warnings)
            .filter(|warning| APPROXIMATIONS.contains(&warning.code))
            .map(|warning| json!({ "code": warning.code, "items": warning.items }))
            .collect();
        let modified = !design.excluded_references.is_empty() || !testbench.spice_lines.is_empty();
        let circuit = json!({
            "excluded_references": design.excluded_references,
            "added_line_count": testbench.spice_lines.len(),
            "dropped_directives": self.deck.dropped_directives,
            "approximations": approximations,
        });
        (if modified { "modified" } else { "as_drawn" }, circuit)
    }

    /// The node a stopped analysis names, if the engine's message is one that may
    /// be repeated.
    fn trouble_node(&self) -> Option<String> {
        let (lines, _) = engine_complaints(&self.engine_log(), self.deck);
        lines.iter().find_map(|line| {
            let (_, rest) = line.split_once("node \"")?;
            rest.split('"').next().map(str::to_owned)
        })
    }

    fn result_body(&self, plot: &Plot, analysis: Value, stopped: Option<&str>) -> Value {
        let (simulated_circuit, circuit) = self.circuit();
        let status = self.status();
        let raw_bytes = std::fs::metadata(self.run.dir.join(RAW_FILE)).map_or(0, |file| file.len());
        let mut body = json!({
            "run_id": self.run.id,
            "simulated_circuit": simulated_circuit,
            "circuit": circuit,
            "analysis": analysis,
            "engine": {
                "name": "ngspice",
                "version": status.and_then(|status| status.engine_version.as_ref()),
                "compatibility": self.design.compatibility.ngbehavior,
                "code_models_loaded": status.map_or(0, |status| status.code_models_loaded.len()),
            },
            "sources": [SOURCE],
            "source_evidence": {
                "excludes_unsaved_editor_state": true,
                "locks_present": self.design.locks_present,
                "inputs_hash": self.design.inputs_hash,
            },
            "warnings": self.warnings(),
            "artifacts": {
                "dir": self.run.dir.display().to_string(),
                "deck_path": self.path(DECK_FILE),
                "raw_path": self.path(RAW_FILE),
                "log_path": self.path(LOG_FILE),
                "raw_bytes": raw_bytes,
            },
        });
        if let Some(reason) = stopped {
            body["stopped"] = json!({ "reason": reason, "at_point": plot.points });
            if let Some(node) = self.trouble_node() {
                body["stopped"]["node"] = json!(node);
            }
        }
        body
    }
}

/// What a run said about itself that a later measurement should be read with.
/// Left out are what concerns this call alone: files it wrote, runs it pruned, and
/// the nets that its own signals share, which a new measurement works out again.
fn carried_warnings(body: &Value) -> Vec<Value> {
    const THIS_CALL_ONLY: [&str; 4] = [
        "old_runs_kept",
        "csv_not_written",
        "run_record_not_saved",
        "nets_share_engine_node",
    ];
    let warnings = body["warnings"].as_array().into_iter().flatten();
    warnings
        .filter(|warning| {
            warning["code"]
                .as_str()
                .is_some_and(|code| !THIS_CALL_ONLY.contains(&code))
        })
        .cloned()
        .collect()
}

/// The warning for a CSV that was not written, or none if it was.
fn table_problem(written: &std::io::Result<export::Written>) -> Option<Value> {
    match written {
        Ok(export::Written::Bytes(_)) => None,
        Ok(export::Written::TooLarge { cells }) => Some(json!({
            "code": "csv_not_written",
            "detail": format!(
                "The data holds {cells} values, past the {} that results.csv takes. The raw \
                 file has all of it.",
                export::MAX_CELLS
            ),
        })),
        Err(error) => Some(json!({
            "code": "csv_not_written",
            "detail": format!("results.csv could not be written: {error}"),
        })),
    }
}

/// Name the pins a part really has, so `i(M1)` shows the way to `i(M1:d)`.
fn with_pins(mut report: Value, not_found: &NotFound) -> Value {
    if !not_found.pins.is_empty() {
        report["pins"] = json!(not_found.pins);
    }
    report
}

#[derive(Default)]
struct Testbench {
    exclude_references: Vec<String>,
    spice_lines: Vec<String>,
}

impl Testbench {
    fn from_args(value: &Value) -> Self {
        let strings = |field: &str| -> Vec<String> {
            value[field]
                .as_array()
                .map(|items| {
                    let text = items.iter().filter_map(Value::as_str);
                    text.map(str::to_owned).collect()
                })
                .unwrap_or_default()
        };
        Self {
            exclude_references: strings("exclude_references"),
            spice_lines: strings("spice_lines"),
        }
    }
}

struct DesignInputs<'a> {
    model_dirs: &'a [PathBuf],
    testbench: &'a Testbench,
    allow_digital_models: bool,
    code_model_count: usize,
    export_timeout: Duration,
}

/// References each pattern selects, and the patterns that select nothing. A
/// trailing `*` matches a prefix.
fn excluded_references(kicad: &KiCadNetlist, patterns: &[String]) -> (Vec<String>, Vec<String>) {
    let mut references = Vec::new();
    let mut unmatched = Vec::new();
    for pattern in patterns {
        let matches = |reference: &str| match pattern.strip_suffix('*') {
            Some(prefix) => reference.starts_with(prefix),
            None => reference == pattern,
        };
        let selected: Vec<String> = kicad
            .components
            .iter()
            .filter(|component| matches(&component.reference))
            .map(|component| component.reference.clone())
            .collect();
        if selected.is_empty() {
            unmatched.push(pattern.clone());
        }
        references.extend(selected);
    }
    references.sort();
    references.dedup();
    (references, unmatched)
}

/// The design as `kicad-cli` sees it, from a private copy of the saved files.
struct Design {
    nets: Vec<NetEntry>,
    elements: Vec<ElementEntry>,
    locks_present: Vec<String>,
    compatibility: Compatibility,
    deck: Result<FlatDeck, Refusal>,
    ignored_model_dirs: Vec<String>,
    excluded_references: Vec<String>,
    preflight: Option<Preflight>,
    project_dir: PathBuf,
    inputs_hash: String,
}

/// The copied sheets and project file, in a stable order. Taken before the
/// export, which works inside the same folder.
fn hash_sources(source_dir: &Path) -> std::io::Result<Sha256> {
    fn files_under(dir: &Path, found: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                files_under(&entry.path(), found)?;
            } else {
                found.push(entry.path());
            }
        }
        Ok(())
    }
    let mut sources = Vec::new();
    files_under(source_dir, &mut sources)?;
    sources.sort();
    let mut hasher = Sha256::new();
    for path in &sources {
        let name = path.strip_prefix(source_dir).unwrap_or(path);
        hasher.update(name.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(std::fs::read(path)?);
    }
    Ok(hasher)
}

/// Everything a result rests on: the sources, then each model file the deck
/// inlined, so a changed model reads as a changed input.
fn inputs_hash(mut hasher: Sha256, inlined_files: &[PathBuf]) -> std::io::Result<String> {
    for path in inlined_files {
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(std::fs::read(path)?);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// KiCad wipes its per-user IBIS cache on every SPICE export, from any project,
/// so an export and the read of its includes must not interleave with another.
static EXPORT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Folders an include may read from, canonical to compare with resolved paths,
/// and any configured folder that was left out. A root that holds the user's
/// home folder would stop the check meaning anything, so it is never used.
fn allowed_roots(project_dir: &Path, model_dirs: &[PathBuf]) -> (Vec<PathBuf>, Vec<String>) {
    let home = dirs::home_dir().and_then(|home| home.canonicalize().ok());
    let usable = |path: &Path| {
        path.parent().is_some() && !home.as_ref().is_some_and(|home| home.starts_with(path))
    };
    let cache = dirs::cache_dir()
        .into_iter()
        .chain(std::env::var_os("KICAD_CACHE_HOME").map(PathBuf::from))
        .flat_map(|dir| [dir.join("KiCad"), dir.join("kicad")]);
    let mut roots: Vec<PathBuf> = std::iter::once(project_dir.to_path_buf())
        .chain(crate::kicad_install::share_roots())
        .chain(cache)
        .filter_map(|path| path.canonicalize().ok())
        .filter(|path| usable(path))
        .collect();
    let mut ignored = Vec::new();
    for dir in model_dirs {
        match dir.canonicalize() {
            Ok(path) if dir.is_absolute() && usable(&path) => roots.push(path),
            _ => ignored.push(dir.display().to_string()),
        }
    }
    (roots, ignored)
}

async fn inspect_design(
    kicad_cli: &str,
    schematic: &Path,
    inputs: &DesignInputs<'_>,
) -> anyhow::Result<Design> {
    let scratch = tempfile::Builder::new()
        .prefix("konnect-sim-export-")
        .tempdir()?;
    let source = scratch.path().join("src");
    std::fs::create_dir(&source)?;
    // The copy is named for the file as the disk spells it, so the hash of the
    // sheets does not depend on how the caller typed the path.
    let schematic = &schematic.canonicalize()?;
    let snapshot = snapshot::take_snapshot(schematic, &source)?;
    let project_dir = schematic
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let compatibility = Compatibility::from_project_file(&schematic.with_extension("kicad_pro"));
    let sources_hash = hash_sources(&source)?;
    let _export_guard = EXPORT_LOCK.lock().await;
    let netlists = cli::export_simulation_netlists(
        kicad_cli,
        &snapshot.root_sheet,
        scratch.path(),
        inputs.export_timeout,
    )
    .await?;
    let kicad = kicad_netlist::parse(&netlists.kicad)?;
    let spice = SpiceNetlist::parse(&netlists.spice);
    let (allowed_roots, ignored_model_dirs) = allowed_roots(&project_dir, inputs.model_dirs);
    let (excluded_references, unmatched_exclusions) =
        excluded_references(&kicad, &inputs.testbench.exclude_references);
    let nets = kicad_netlist::net_entries(&kicad, &spice);
    let elements = kicad_netlist::element_entries(&kicad, &spice, &excluded_references);
    let excluded_elements: Vec<String> = elements
        .iter()
        .filter(|element| element.excluded)
        .filter_map(|element| element.element_name.as_deref())
        .map(str::to_lowercase)
        .collect();
    let deck = flatten::flatten(&DeckInputs {
        netlist: &netlists.spice,
        testbench: &inputs.testbench.spice_lines,
        project_dir: &project_dir,
        allowed_roots: &allowed_roots,
        compatibility,
        excluded_elements: &excluded_elements,
    });
    let preflight = deck.as_ref().ok().map(|deck| {
        preflight::run(&preflight::Inputs {
            deck,
            compatibility,
            kicad: &kicad,
            nets: &nets,
            elements: &elements,
            allow_digital_models: inputs.allow_digital_models,
            code_model_count: inputs.code_model_count,
            unmatched_exclusions: &unmatched_exclusions,
            has_testbench_lines: !inputs.testbench.spice_lines.is_empty(),
        })
    });
    let inlined = deck.as_ref().map_or(&[][..], |deck| &deck.inlined_files);
    let inputs_hash = inputs_hash(sources_hash, inlined)?;
    Ok(Design {
        compatibility,
        inputs_hash,
        project_dir,
        deck,
        ignored_model_dirs,
        excluded_references,
        preflight,
        nets,
        elements,
        locks_present: snapshot
            .locks_present
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
    })
}

#[derive(Serialize)]
struct Listing<'a, T> {
    total: usize,
    shown: usize,
    truncated: bool,
    items: Vec<&'a T>,
}

/// Counted before the cut, as `run_drc` does, so a short list never reads as a
/// small design.
fn listing<'a, T>(items: impl Iterator<Item = &'a T>, limit: usize) -> Listing<'a, T> {
    let items: Vec<&T> = items.collect();
    let total = items.len();
    let items: Vec<&T> = items.into_iter().take(limit).collect();
    Listing {
        total,
        shown: items.len(),
        truncated: items.len() < total,
        items,
    }
}

/// `ready` is true only when every check ran and passed, false when one failed,
/// and null when one could not run. A check that did not run is never a pass.
fn readiness_report(
    engine: &EngineStatus,
    design: &anyhow::Result<Design>,
    net_filter: &str,
    limit: usize,
) -> Value {
    let engine_ran = engine.probe_status != "not_available";
    let engine_check = if engine_ran {
        json!({ "name": "engine", "ran": true, "passed": engine.available })
    } else {
        json!({ "name": "engine", "ran": false, "reason": engine.detail })
    };
    let export_check = match design {
        Ok(_) => json!({ "name": "netlist_export", "ran": true, "passed": true }),
        Err(error) => json!({
            "name": "netlist_export",
            "ran": true,
            "passed": false,
            "detail": export_detail(&format!("{error:#}")),
        }),
    };
    let deck_check = match design.as_ref().map(|design| &design.deck) {
        Ok(Ok(_)) => json!({ "name": "deck_vetting", "ran": true, "passed": true }),
        Ok(Err(refusal)) => json!({
            "name": "deck_vetting",
            "ran": true,
            "passed": false,
            "refusal": refusal,
        }),
        Err(_) => json!({
            "name": "deck_vetting",
            "ran": false,
            "reason": "The netlist could not be exported.",
        }),
    };
    let preflight = design
        .as_ref()
        .ok()
        .and_then(|design| design.preflight.as_ref());
    let mut checks = vec![engine_check, export_check, deck_check];
    match preflight {
        Some(preflight) => checks.extend(
            preflight
                .checks
                .iter()
                .map(|check| check_report(check, limit)),
        ),
        None => checks.push(json!({
            "name": "preflight",
            "ran": false,
            "reason": "Preflight runs on a deck that passed vetting."
        })),
    }
    let ready = if checks.iter().any(|check| check["passed"] == false) {
        json!(false)
    } else if checks.iter().any(|check| check["ran"] == false) {
        Value::Null
    } else {
        json!(true)
    };

    let mut report = json!({
        "ready": ready,
        "checks": checks,
        "engine": engine,
        "sources": ["saved_schematic_snapshot"],
    });
    if let Ok(design) = design {
        let filter = net_filter.to_lowercase();
        let nets = design.nets.iter().filter(|net| {
            net.kicad_name.to_lowercase().contains(&filter)
                || net.spice_name.to_lowercase().contains(&filter)
        });
        report["nets"] = json!(listing(nets, limit));
        report["elements"] = json!(listing(design.elements.iter(), limit));
        report["engine"]["compatibility"] = json!(design.compatibility.ngbehavior);
        if !design.ignored_model_dirs.is_empty() {
            report["ignored_model_dirs"] = json!(design.ignored_model_dirs);
        }
        if let Ok(deck) = &design.deck {
            report["deck"] = json!({
                "line_count": deck.line_count(),
                "inlined_file_count": deck.inlined_files.len(),
                "dropped_directives": deck.dropped_directives,
            });
        }
        if let Some(preflight) = &design.preflight {
            report["unmodelled_symbols"] = unmodelled_report(&preflight.unmodelled_symbols, limit);
            let warnings = preflight.warnings.iter();
            report["warnings"] = json!(warnings
                .map(|warning| warning_report(warning, limit))
                .collect::<Vec<_>>());
        }
        if !design.excluded_references.is_empty() {
            report["excluded_references"] =
                json!(listing(design.excluded_references.iter(), limit));
        }
        report["source_evidence"] = json!({
            "excludes_unsaved_editor_state": true,
            "locks_present": design.locks_present,
        });
    }
    report
}

/// A check with its items cut to the limit and counted before the cut.
fn check_report(check: &preflight::Check, limit: usize) -> Value {
    let mut report = json!(check);
    add_items(&mut report, &check.items, limit);
    report
}

fn add_items(report: &mut Value, items: &[String], limit: usize) {
    let shown: Vec<&String> = items.iter().take(limit).collect();
    report["total"] = json!(items.len());
    report["shown"] = json!(shown.len());
    report["truncated"] = json!(shown.len() < items.len());
    report["items"] = json!(shown);
}

fn warning_report(warning: &preflight::Warning, limit: usize) -> Value {
    let mut report = json!(warning);
    add_items(&mut report, &warning.items, limit);
    report
}

/// Listed like the other lists, with each symbol's pins cut to the same limit.
fn unmodelled_report(symbols: &[preflight::UnmodelledSymbol], limit: usize) -> Value {
    let mut report = json!(listing(symbols.iter(), limit));
    if let Some(items) = report["items"].as_array_mut() {
        for (item, symbol) in items.iter_mut().zip(symbols) {
            item["pin_total"] = json!(symbol.pins.len());
            item["pins"] = json!(symbol.pins.iter().take(limit).collect::<Vec<_>>());
        }
    }
    report
}

/// Keep KiCad's own words and drop library noise that says nothing about the
/// design. Exported text is data from the project, so it stays bounded.
fn export_detail(message: &str) -> String {
    let lines: Vec<&str> = message
        .lines()
        .filter(|line| !line.contains("Fontconfig"))
        .collect();
    lines.join("\n").chars().take(DETAIL_LIMIT).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::ToolRouter;
    use crate::tools::ServerConfig;
    use std::sync::Arc;

    const NET: &str = include_str!("../../tests/fixtures/simulation/rectifier_unmodelled.net");
    const CIR: &str = include_str!("../../tests/fixtures/simulation/rectifier_unmodelled.cir");

    fn engine(probe_status: &'static str, available: bool) -> EngineStatus {
        EngineStatus {
            probe_status,
            available,
            version: available.then(|| "44.2".to_string()),
            library_path: None,
            code_models_dir: None,
            code_model_count: 0,
            code_models_failed: Vec::new(),
            searched_locations: Vec::new(),
            detail: None,
        }
    }

    static NO_TESTBENCH: Testbench = Testbench {
        exclude_references: Vec::new(),
        spice_lines: Vec::new(),
    };

    fn no_inputs() -> DesignInputs<'static> {
        DesignInputs {
            model_dirs: &[],
            testbench: &NO_TESTBENCH,
            allow_digital_models: true,
            code_model_count: 1,
            export_timeout: EXPORT_TIMEOUT,
        }
    }

    #[test]
    fn exclusions_take_exact_references_and_prefix_wildcards() {
        let kicad = kicad_netlist::parse(NET).unwrap();
        let (references, unmatched) = excluded_references(
            &kicad,
            &["D1".to_string(), "R*".to_string(), "J*".to_string()],
        );
        assert_eq!(references, ["D1", "R1", "R2"]);
        assert_eq!(unmatched, ["J*"]);
    }

    #[test]
    fn a_symbol_with_many_pins_is_cut_to_the_limit() {
        let pin = |number: usize| preflight::SymbolPin {
            pin: number.to_string(),
            pin_name: None,
            pin_type: None,
            net: "/a".to_string(),
            spice_net: None,
        };
        let symbol = preflight::UnmodelledSymbol {
            reference: "U1".to_string(),
            value: String::new(),
            lib_id: None,
            pins: (0..5).map(pin).collect(),
            suggested_fix: "exclude",
        };
        let report = unmodelled_report(&[symbol], 2);
        assert_eq!(report["items"][0]["pin_total"], 5);
        assert_eq!(report["items"][0]["pins"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_passing_design_is_ready_only_when_every_check_ran() {
        let mut passing = design().unwrap();
        passing.preflight = Some(Preflight {
            checks: Vec::new(),
            warnings: Vec::new(),
            unmodelled_symbols: Vec::new(),
            event_nodes: false,
        });
        let report = readiness_report(&engine("ok", true), &Ok(passing), "", 25);
        assert_eq!(report["ready"], true, "{report}");
        let report = readiness_report(&engine("not_available", false), &design(), "", 25);
        assert_eq!(report["ready"], Value::Null, "{report}");
    }

    #[test]
    fn check_items_are_cut_to_the_limit_and_counted_first() {
        let kicad = kicad_netlist::parse(NET).unwrap();
        let mut design = design().unwrap();
        design.preflight = Some(preflight::run(&preflight::Inputs {
            deck: &flatten::flatten(&DeckInputs {
                netlist: CIR,
                testbench: &[],
                project_dir: Path::new("/"),
                allowed_roots: &[],
                compatibility: Compatibility::DEFAULT,
                excluded_elements: &[],
            })
            .unwrap(),
            compatibility: Compatibility::DEFAULT,
            kicad: &kicad,
            nets: &design.nets,
            elements: &design.elements,
            allow_digital_models: true,
            code_model_count: 1,
            unmatched_exclusions: &[],
            has_testbench_lines: false,
        }));
        let report = readiness_report(&engine("ok", true), &Ok(design), "", 1);
        assert_eq!(report["ready"], false);
        let coverage = report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["name"] == "model_coverage")
            .unwrap();
        assert_eq!(coverage["total"], 2, "{report}");
        assert_eq!(coverage["items"].as_array().unwrap().len(), 1);
        assert_eq!(report["unmodelled_symbols"]["total"], 2);
        assert_eq!(report["unmodelled_symbols"]["truncated"], true);
    }

    fn design() -> anyhow::Result<Design> {
        let kicad = kicad_netlist::parse(NET)?;
        let spice = SpiceNetlist::parse(CIR);
        Ok(Design {
            nets: kicad_netlist::net_entries(&kicad, &spice),
            elements: kicad_netlist::element_entries(&kicad, &spice, &[]),
            locks_present: vec!["/p/~top.kicad_sch.lck".to_string()],
            compatibility: Compatibility::DEFAULT,
            deck: Ok(FlatDeck::default()),
            ignored_model_dirs: Vec::new(),
            excluded_references: Vec::new(),
            preflight: None,
            project_dir: PathBuf::new(),
            inputs_hash: String::new(),
        })
    }

    fn context(kicad_cli: &str) -> ToolContext {
        ToolContext::new(
            ServerConfig {
                kicad_cli: kicad_cli.to_string(),
                ..Default::default()
            },
            Arc::new(ToolRouter::new()),
        )
    }

    fn sheet() -> &'static str {
        "(kicad_sch (version 20250114) (generator \"eeschema\") \
         (uuid \"00000000-0000-0000-0000-000000000001\") (paper \"A4\"))"
    }

    #[test]
    fn a_working_engine_and_export_are_not_ready_while_preflight_has_not_run() {
        let report = readiness_report(&engine("ok", true), &design(), "", 25);
        assert!(report["ready"].is_null());
        assert_eq!(report["checks"][0]["passed"], true);
        assert_eq!(report["checks"][1]["passed"], true);
        assert_eq!(report["checks"][2]["passed"], true);
        assert_eq!(report["checks"][3]["ran"], false);
    }

    #[test]
    fn a_refused_deck_is_not_ready_and_says_where() {
        let mut refused = design().unwrap();
        refused.deck = Err(Refusal {
            code: flatten::RefusalCode::EncryptedModel,
            origin: "/p/tps.lib".to_string(),
            line: Some(2),
            detail: "encrypted".to_string(),
        });
        let report = readiness_report(&engine("ok", true), &Ok(refused), "", 25);
        assert_eq!(report["ready"], false);
        assert_eq!(report["checks"][2]["passed"], false);
        assert_eq!(report["checks"][2]["refusal"]["code"], "encrypted_model");
        assert_eq!(report["checks"][2]["refusal"]["line"], 2);
        assert!(report.get("deck").is_none());
    }

    #[test]
    fn a_model_folder_that_cannot_be_used_is_reported_rather_than_lost() {
        let project = tempfile::tempdir().unwrap();
        let extra = tempfile::tempdir().unwrap();
        let home = dirs::home_dir().unwrap();
        let dirs = [
            extra.path().to_path_buf(),
            PathBuf::from("/"),
            home.clone(),
            PathBuf::from("~/spice"),
            PathBuf::from("relative"),
            PathBuf::from("."),
            extra.path().join("absent"),
        ];
        let (roots, ignored) = allowed_roots(project.path(), &dirs);
        assert!(roots.contains(&project.path().canonicalize().unwrap()));
        assert!(roots.contains(&extra.path().canonicalize().unwrap()));
        assert!(!roots.iter().any(|root| root.parent().is_none()));
        assert_eq!(ignored.len(), 6, "{ignored:?}");
    }

    #[test]
    fn a_project_that_holds_the_home_folder_is_not_a_root() {
        let home = dirs::home_dir().unwrap().canonicalize().unwrap();
        let (roots, _) = allowed_roots(&home, &[]);
        assert!(!roots.contains(&home));
    }

    #[test]
    fn a_failed_export_cannot_claim_the_deck_was_vetted() {
        let report = readiness_report(&engine("ok", true), &Err(anyhow::anyhow!("failed")), "", 25);
        assert_eq!(report["checks"][2]["ran"], false);
    }

    #[test]
    fn a_missing_engine_is_not_ready() {
        let report = readiness_report(&engine("not_found", false), &design(), "", 25);
        assert_eq!(report["ready"], false);
        assert_eq!(report["checks"][0]["passed"], false);
    }

    #[test]
    fn an_engine_check_that_could_not_run_is_unknown_not_a_failure() {
        let report = readiness_report(&engine("not_available", false), &design(), "", 25);
        assert!(report["ready"].is_null());
        assert_eq!(report["checks"][0]["ran"], false);
    }

    #[test]
    fn a_failed_export_is_not_ready_and_lists_no_nets() {
        let report = readiness_report(
            &engine("ok", true),
            &Err(anyhow::anyhow!("kicad-cli exited with 2")),
            "",
            25,
        );
        assert_eq!(report["ready"], false);
        assert_eq!(report["checks"][1]["passed"], false);
        assert!(report["checks"][1]["detail"]
            .as_str()
            .unwrap()
            .contains("exited with 2"));
        assert!(report.get("nets").is_none());
        assert!(report.get("source_evidence").is_none());
    }

    #[test]
    fn totals_are_counted_before_the_cut() {
        let report = readiness_report(&engine("ok", true), &design(), "", 2);
        assert_eq!(report["elements"]["total"], 5);
        assert_eq!(report["elements"]["shown"], 2);
        assert_eq!(report["elements"]["truncated"], true);
        assert_eq!(report["elements"]["items"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn nets_can_be_filtered_by_name() {
        let report = readiness_report(&engine("ok", true), &design(), "RECT_", 25);
        assert_eq!(report["nets"]["total"], 1);
        assert_eq!(report["nets"]["items"][0]["kicad_name"], "/rect_out");
    }

    #[test]
    fn results_state_they_come_from_saved_files_and_name_any_locks() {
        let report = readiness_report(&engine("ok", true), &design(), "", 25);
        assert_eq!(report["sources"][0], "saved_schematic_snapshot");
        assert_eq!(
            report["source_evidence"]["excludes_unsaved_editor_state"],
            true
        );
        assert_eq!(
            report["source_evidence"]["locks_present"][0],
            "/p/~top.kicad_sch.lck"
        );
    }

    #[test]
    fn export_details_drop_font_noise_and_stay_bounded() {
        let noisy = format!(
            "Fontconfig warning: x\nSimulation model library not found\n{}",
            "y".repeat(5000)
        );
        let detail = export_detail(&noisy);
        assert!(!detail.contains("Fontconfig"));
        assert!(detail.contains("library not found"));
        assert!(detail.chars().count() <= DETAIL_LIMIT);
    }

    #[tokio::test]
    async fn a_missing_schematic_is_a_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let args = json!({ "schematic": dir.path().join("absent.kicad_sch") });
        let result = handle_check_simulation_readiness(&args, &context(""))
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn a_child_sheet_is_refused_with_the_root_named() {
        let dir = tempfile::tempdir().unwrap();
        let root_sheet = "(kicad_sch (version 20250114) (generator \"eeschema\") \
             (uuid \"00000000-0000-0000-0000-000000000001\") (paper \"A4\") \
             (sheet (at 0 0) (size 10 10) (uuid \"00000000-0000-0000-0000-000000000002\") \
             (property \"Sheetname\" \"C\" (at 0 0 0)) \
             (property \"Sheetfile\" \"child.kicad_sch\" (at 0 0 0))))";
        std::fs::write(dir.path().join("top.kicad_sch"), root_sheet).unwrap();
        std::fs::write(dir.path().join("top.kicad_pro"), "{}").unwrap();
        let child = dir.path().join("child.kicad_sch");
        std::fs::write(&child, sheet()).unwrap();
        let result =
            handle_check_simulation_readiness(&json!({ "schematic": child }), &context(""))
                .await
                .unwrap();
        assert!(result.is_error);
    }

    #[cfg(unix)]
    #[test]
    fn units_come_from_the_signal() {
        assert_eq!(signal_unit("v(/out)"), "V");
        assert_eq!(signal_unit("i(r1)"), "A");
        assert_eq!(signal_unit("i(m1:d)"), "A");
        assert_eq!(signal_unit("v1#branch"), "A");
        assert_eq!(signal_unit("m1:power"), "W");
        assert_eq!(signal_unit("temperature"), "");
    }

    fn net(kicad_name: &str, spice_name: &str) -> NetEntry {
        NetEntry {
            kicad_name: kicad_name.to_string(),
            spice_name: spice_name.to_string(),
            pin_count: 1,
            in_deck: true,
        }
    }

    #[test]
    fn a_node_maps_back_to_the_one_net_that_makes_it() {
        let nets = [
            net("/OUT", "/OUT"),
            net("/\u{5e9}", "/\u{5e9}"),
            net("GND", "GND"),
        ];
        assert_eq!(kicad_net("v(/out)", &nets).as_deref(), Some("/OUT"));
        assert_eq!(kicad_net("v(/__)", &nets).as_deref(), Some("/\u{5e9}"));
        assert_eq!(kicad_net("i(r1)", &nets), None);
        assert_eq!(kicad_net("v(/missing)", &nets), None);
        let merged = [net("/\u{5e9}", "/\u{5e9}"), net("/\u{5ea}", "/\u{5ea}")];
        assert_eq!(kicad_net("v(/__)", &merged), None);
    }

    const OP_RAW: &[u8] = include_bytes!("../../tests/fixtures/simulation/raw/op.ascii.raw");

    fn op_plot() -> Plot {
        crate::simulation::raw::parse(OP_RAW).unwrap().remove(0)
    }

    #[test]
    fn rows_carry_the_values_with_top_level_signals_first() {
        let mut plot = op_plot();
        let rows = operating_point_rows(&plot, &[net("/OUT", "/OUT")]);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].signal, "v(/out)");
        assert_eq!(rows[1].net.as_deref(), Some("/OUT"));
        assert_eq!(rows[1].value, 2.0);
        assert_eq!(rows[1].unit, "V");

        plot.variables[0].name = "v(x1.inner)".to_string();
        let rows = operating_point_rows(&plot, &[]);
        assert_eq!(rows[2].signal, "v(x1.inner)");
    }

    #[test]
    fn an_operating_point_is_covered_by_one_whole_point_of_data() {
        assert!(operating_point_covered(&op_plot()));
        let mut cut = op_plot();
        cut.points = 0;
        assert!(!operating_point_covered(&cut));
        let mut empty = op_plot();
        empty.variables.clear();
        assert!(!operating_point_covered(&empty));
    }

    fn flat(netlist: &str, include: &Path, roots: &[PathBuf]) -> FlatDeck {
        let netlist = netlist.replace("INCLUDE", &include.display().to_string());
        flatten::flatten(&DeckInputs {
            netlist: &netlist,
            testbench: &[],
            project_dir: include.parent().unwrap(),
            allowed_roots: roots,
            compatibility: Compatibility::DEFAULT,
            excluded_elements: &[],
        })
        .unwrap()
    }

    #[test]
    fn the_log_never_repeats_text_from_a_model_file() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().canonicalize().unwrap();
        let model = root.join("private.lib");
        std::fs::write(&model, "R77 b 0 1k SECRETTOKEN=3\n.param secretparam=2\n").unwrap();
        let deck = flat(
            ".title t\n.include \"INCLUDE\"\nR1 own 0 1k\n",
            &model,
            std::slice::from_ref(&root),
        );
        let log = "stdout noise\n\
                   stderr Warning: can't find the initialization file spinit.\n\
                   stderr Note: Starting dynamic gmin stepping\n\
                   stderr r77 b probe_int_0_r77 1k secrettoken=3\n\
                   stderr unknown parameter (secrettoken)\n\
                   stderr Expression err: secretparam*2}\n\
                   stderr unknown parameter (secret__word)\n\
                   stderr .model ddd d(secrettok ...\n\
                   stderr r1 own 0 1k\n\
                   stderr could not find a valid modelname\n\
                   stderr Error: sim_status: no such variable.\n\
                   stderr after the marker\n";
        assert_eq!(
            complaint_items(log, &deck),
            [
                "r1 own 0 1k",
                "could not find a valid modelname",
                "5 lines quoting a model file are in the log.",
            ]
        );
    }

    #[test]
    fn a_non_ascii_model_word_is_recognised_after_the_engine_rewrites_it() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().canonicalize().unwrap();
        let model = root.join("hebrew.lib");
        std::fs::write(&model, ".model dx D(AB\u{5e9}HIDDENPART=3)\n").unwrap();
        let deck = flat(
            ".title t\n.include \"INCLUDE\"\nR1 own 0 1k\n",
            &model,
            std::slice::from_ref(&root),
        );
        let log = "stderr unknown parameter (ab__hiddenpart)\n";
        let (shown, hidden) = engine_complaints(log, &deck);
        assert!(shown.is_empty(), "{shown:?}");
        assert_eq!(hidden, 1);
    }

    #[test]
    fn a_word_the_schematic_also_uses_is_not_private() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().canonicalize().unwrap();
        let model = root.join("shared.lib");
        std::fs::write(&model, ".param sharedname=1\n").unwrap();
        let deck = flat(
            ".title t\n.include \"INCLUDE\"\n.param sharedname=2\nR1 own 0 1k\n",
            &model,
            std::slice::from_ref(&root),
        );
        let log = "stderr unknown parameter (sharedname)\n";
        let (shown, hidden) = engine_complaints(log, &deck);
        assert_eq!(shown.len(), 1, "{shown:?}");
        assert_eq!(hidden, 0);
    }

    #[test]
    fn a_short_model_word_does_not_hide_a_line_about_the_schematic() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().canonicalize().unwrap();
        let model = root.join("diode.lib");
        std::fs::write(&model, ".model dx D(IS=1e-14 N=1)\n").unwrap();
        let deck = flat(
            ".title t\n.include \"INCLUDE\"\nR9 own 0 1k\n",
            &model,
            std::slice::from_ref(&root),
        );
        let log = "stderr Warning: Value of resistor r9 is too small, set to 1.000000e-12\n";
        let (shown, hidden) = engine_complaints(log, &deck);
        assert_eq!(shown.len(), 1, "{shown:?}");
        assert_eq!(hidden, 0);
    }

    #[test]
    fn a_long_model_warning_cannot_crowd_out_the_schematics_lines() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().canonicalize().unwrap();
        let model = root.join("big.lib");
        std::fs::write(&model, ".param vendorsecret=1\n").unwrap();
        let deck = flat(
            ".title t\n.include \"INCLUDE\"\nR1 own 0 1k\n",
            &model,
            std::slice::from_ref(&root),
        );
        let log = format!(
            "{}stderr r1 own 0 1k\n",
            "stderr bad vendorsecret\n".repeat(50)
        );
        let (shown, hidden) = engine_complaints(&log, &deck);
        assert_eq!(shown, ["r1 own 0 1k"]);
        assert_eq!(hidden, 50);
    }

    #[test]
    fn complaints_are_bounded() {
        let log = format!("stderr {}\n", "x".repeat(5_000)).repeat(100);
        let (complaints, _) = engine_complaints(&log, &FlatDeck::default());
        assert_eq!(complaints.len(), MAX_LOG_ITEMS);
        assert_eq!(complaints[0].chars().count(), MAX_LOG_ITEM_CHARS);
    }

    #[test]
    fn a_changed_source_or_model_changes_the_hash() {
        let folder = tempfile::tempdir().unwrap();
        let sources = folder.path().join("src");
        std::fs::create_dir_all(sources.join("sub")).unwrap();
        std::fs::write(sources.join("top.kicad_sch"), "a").unwrap();
        std::fs::write(sources.join("sub/one.kicad_sch"), "b").unwrap();
        let model = folder.path().join("m.lib");
        std::fs::write(&model, "m").unwrap();
        let hash = || {
            let models = std::slice::from_ref(&model);
            inputs_hash(hash_sources(&sources).unwrap(), models).unwrap()
        };
        let first = hash();
        assert_eq!(hash(), first);
        std::fs::write(sources.join("sub/one.kicad_sch"), "c").unwrap();
        let second = hash();
        assert_ne!(second, first);
        std::fs::write(&model, "n").unwrap();
        assert_ne!(hash(), second);
    }

    fn transient(analysis: Value) -> Result<Transient, CallToolResult> {
        Transient::from_args(&analysis)
    }

    #[test]
    fn a_transient_line_holds_only_numbers_written_here() {
        let line = |analysis: Value| transient(analysis).unwrap().line();
        assert_eq!(line(json!({ "stop_time_seconds": 10 })), ".tran 5e-3 1e1");
        assert_eq!(
            line(json!({ "stop_time_seconds": 1, "max_step_seconds": 1e-6 })),
            ".tran 5e-4 1e0 0e0 1e-6"
        );
        assert_eq!(
            line(json!({ "stop_time_seconds": 1, "start_save_seconds": 0.5 })),
            ".tran 5e-4 1e0 5e-1 5e-4"
        );
        assert_eq!(
            line(json!({
                "stop_time_seconds": 1,
                "start_save_seconds": 0.5,
                "max_step_seconds": 1e-3,
                "use_initial_conditions": true
            })),
            ".tran 5e-4 1e0 5e-1 1e-3 uic"
        );
    }

    #[test]
    fn a_transient_the_engine_could_not_take_is_refused_with_its_field() {
        let refused = |analysis: Value, field: &str| {
            let result = transient(analysis).unwrap_err();
            let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
                panic!("expected text")
            };
            let body: Value = serde_json::from_str(text).unwrap();
            assert_eq!(body["error"]["field"], field, "{body}");
        };
        let stop = "analysis.stop_time_seconds";
        refused(json!({}), stop);
        refused(json!({ "stop_time_seconds": 0 }), stop);
        refused(json!({ "stop_time_seconds": -1 }), stop);
        refused(json!({ "stop_time_seconds": "soon" }), stop);
        refused(json!({ "stop_time_seconds": 1e-16 }), stop);
        refused(json!({ "stop_time_seconds": 1e7 }), stop);
        let step = "analysis.max_step_seconds";
        refused(
            json!({ "stop_time_seconds": 1, "max_step_seconds": 0 }),
            step,
        );
        refused(
            json!({ "stop_time_seconds": 1, "max_step_seconds": -1 }),
            step,
        );
        refused(
            json!({ "stop_time_seconds": 1, "max_step_seconds": 1e-20 }),
            step,
        );
        refused(
            json!({ "stop_time_seconds": 1, "max_step_seconds": 2 }),
            step,
        );
        let start = "analysis.start_save_seconds";
        refused(
            json!({ "stop_time_seconds": 1, "start_save_seconds": 1 }),
            start,
        );
        refused(
            json!({ "stop_time_seconds": 1, "start_save_seconds": -1 }),
            start,
        );
    }

    fn time_plot(last: f64) -> Plot {
        let text = format!(
            "Title: t\nPlotname: Transient Analysis\nFlags: real\nNo. Variables: 2\n\
             No. Points: 2\nVariables:\n\t0\ttime\ttime\n\t1\tv(a)\tvoltage\nValues:\n\
             0\t0.0\n\t1.0\n1\t{last:e}\n\t2.0\n"
        );
        crate::simulation::raw::parse(text.as_bytes())
            .unwrap()
            .remove(0)
    }

    #[test]
    fn a_transient_is_covered_only_by_reaching_its_stop_time() {
        let run = transient(json!({ "stop_time_seconds": 1.0 })).unwrap();
        assert!(run.covered(&time_plot(1.0)));
        assert!(run.covered(&time_plot(1.0 - 1e-7)));
        assert!(!run.covered(&time_plot(1.0 - 1e-5)));
        assert!(!run.covered(&time_plot(0.5)));
        assert!(!run.covered(&op_plot()));
    }

    #[test]
    fn a_file_cut_short_never_covers_a_transient() {
        let run = transient(json!({ "stop_time_seconds": 1.0 })).unwrap();
        let mut plot = time_plot(1.0);
        plot.declared_points += 1;
        assert!(!run.covered(&plot));
    }

    #[test]
    fn only_a_transient_takes_signals_and_measurements() {
        let op = json!({ "kind": "operating_point" });
        let run = json!({ "kind": "transient", "stop_time_seconds": 1 });
        let none = Value::Null;
        let signals = json!(["v(a)"]);
        let measurements = json!([{ "kind": "max", "signal": "v(a)" }]);
        assert!(Wanted::from_args(&op, &none, &none, 0).is_ok());
        assert!(Wanted::from_args(&op, &signals, &none, 0).is_err());
        assert!(Wanted::from_args(&op, &none, &measurements, 0).is_err());
        assert!(Wanted::from_args(&op, &none, &none, 4).is_err());
        assert!(Wanted::from_args(&run, &signals, &measurements, 4).is_ok());
    }

    #[test]
    fn each_analysis_takes_the_measurements_that_fit_it() {
        let ac = json!({ "kind": "ac", "start_hz": 1, "stop_hz": 100 });
        let dc = json!({ "kind": "dc_sweep", "sweeps": [
            { "target": "V1", "start_value": 0, "stop_value": 5, "step_value": 1 }
        ] });
        let run = json!({ "kind": "transient", "stop_time_seconds": 1 });
        let none = Value::Null;
        let kinds = |names: &[&str]| -> Value {
            let items: Vec<Value> = names
                .iter()
                .map(|kind| {
                    let mut entry = json!({ "kind": kind, "signal": "v(a)" });
                    match *kind {
                        "value_at" => entry["at_hz"] = json!(10),
                        "crossing" => entry["level"] = json!(1),
                        _ => {}
                    }
                    entry
                })
                .collect();
            json!(items)
        };
        let fits = |analysis: &Value, names: &[&str]| {
            let mut measurements = kinds(names);
            // The point-in-axis field follows the analysis.
            for entry in measurements.as_array_mut().unwrap() {
                if entry["kind"] == "value_at" {
                    entry.as_object_mut().unwrap().remove("at_hz");
                    let key = match analysis["kind"].as_str() {
                        Some("ac") => "at_hz",
                        Some("dc_sweep") => "at_sweep",
                        _ => "at_seconds",
                    };
                    entry[key] = json!(1);
                }
            }
            Wanted::from_args(analysis, &none, &measurements, 0).is_ok()
        };
        let time_kinds = [
            "min",
            "max",
            "peak_to_peak",
            "peak_abs",
            "average",
            "rms",
            "integral",
            "i2t",
            "crossing",
            "rise_time",
            "fall_time",
            "overshoot_percent",
            "settling_time",
        ];
        let ac_kinds = ["bandwidth_3db", "phase_margin", "gain_margin"];
        let sweep_kinds = ["min", "max", "peak_to_peak", "peak_abs", "crossing"];
        for kind in time_kinds {
            let current = kind != "i2t";
            let entry = if current { kind } else { "min" };
            assert!(fits(&run, &[entry]), "transient {kind}");
            assert!(!fits(&ac, &[kind]), "ac {kind}");
        }
        for kind in ac_kinds {
            assert!(fits(&ac, &[kind]), "ac {kind}");
            assert!(!fits(&run, &[kind]), "transient {kind}");
            assert!(!fits(&dc, &[kind]), "dc {kind}");
        }
        for kind in sweep_kinds {
            assert!(fits(&dc, &[kind]), "dc {kind}");
        }
        for kind in [
            "average",
            "rms",
            "integral",
            "rise_time",
            "fall_time",
            "overshoot_percent",
            "settling_time",
        ] {
            assert!(!fits(&dc, &[kind]), "dc {kind}");
        }
        assert!(fits(&ac, &["value_at"]) && fits(&dc, &["value_at"]) && fits(&run, &["value_at"]));
    }

    #[test]
    fn an_envelope_with_no_signals_or_a_bad_signal_is_refused() {
        let run = json!({ "kind": "transient", "stop_time_seconds": 1 });
        let none = Value::Null;
        assert!(Wanted::from_args(&run, &none, &none, 4).is_err());
        assert!(Wanted::from_args(&run, &json!(["x(a)"]), &none, 0).is_err());
        let many = json!(vec!["v(a)"; MAX_SIGNALS + 1]);
        assert!(Wanted::from_args(&run, &many, &none, 0).is_err());
        let wanted = json!({ "kind": "max", "signal": "v(a)" });
        let many = json!(vec![wanted; MAX_MEASUREMENTS + 1]);
        assert!(Wanted::from_args(&run, &none, &many, 0).is_err());
    }

    #[test]
    fn subcircuit_bodies_are_not_top_level_elements() {
        let root = tempfile::tempdir().unwrap();
        let roots = [root.path().to_path_buf()];
        let netlist = ".title t\nR1 a 0 1k\n.subckt s a\nR2 a 0 1k\n.SUBCKT t a\nR3 a 0 1k\n.ends\nR4 a 0 1k\n.ENDS s\nM1 a a 0 0 n\n.model n NMOS\n+ VTO=1\n.end\n";
        let deck = flat(netlist, root.path(), &roots);
        assert_eq!(top_level_elements(&deck), ["R1", "M1"]);
    }

    #[test]
    fn a_node_condition_is_seen_in_any_case_and_a_device_value_is_not_one() {
        let root = tempfile::tempdir().unwrap();
        let roots = [root.path().to_path_buf()];
        let with = |card: &str| {
            let netlist = format!(".title t\nR1 a 0 1k\nC1 a 0 1u IC=1\n{card}.end\n");
            sets_node_conditions(&flat(&netlist, root.path(), &roots))
        };
        assert!(with(".ic v(a)=5\n"));
        assert!(with(".IC V(a)=5\n"));
        assert!(with(".nodeset v(a)=5\n"));
        assert!(with(".NODESET V(a)=5\n"));
        assert!(!with(""));
    }

    #[test]
    fn every_analysis_reads_back_from_its_stored_form() {
        let transient = |args: Value| Analysis::Transient(Transient::from_args(&args).unwrap());
        let analyses = [
            Analysis::OperatingPoint,
            transient(json!({ "kind": "transient", "stop_time_seconds": 0.5 })),
            transient(json!({ "kind": "transient", "stop_time_seconds": 0.5,
                              "max_step_seconds": 1e-5, "start_save_seconds": 0.1,
                              "use_initial_conditions": true })),
            Analysis::Ac(
                Ac::from_args(&json!({ "kind": "ac", "start_hz": 10.0, "stop_hz": 1e5,
                                       "scale": "octave", "points": 12 }))
                .unwrap(),
            ),
        ];
        for analysis in analyses {
            let stored = analysis.record();
            assert_eq!(Analysis::from_record(&stored), Some(analysis), "{stored}");
        }
        for broken in [
            json!({}),
            json!({ "kind": "noise" }),
            json!({ "kind": "ac" }),
            json!({ "kind": "transient" }),
            json!({ "kind": "dc_sweep" }),
        ] {
            assert_eq!(Analysis::from_record(&broken), None, "{broken}");
        }
    }

    #[test]
    fn a_run_carries_what_it_reported_except_what_belongs_to_its_own_call() {
        let body = json!({ "warnings": [
            { "code": "engine_messages", "items": ["x"] },
            { "code": "no_point_at_start" },
            { "code": "old_runs_kept" },
            { "code": "csv_not_written" },
            { "code": "run_record_not_saved" },
            { "code": "nets_share_engine_node" },
            { "detail": "no code at all" },
        ] });
        let carried: Vec<&str> = carried_warnings(&body)
            .iter()
            .map(|warning| warning["code"].as_str().unwrap_or_default().to_owned())
            .collect::<Vec<_>>()
            .iter()
            .map(|code| match code.as_str() {
                "engine_messages" => "engine_messages",
                "no_point_at_start" => "no_point_at_start",
                other => panic!("carried {other}"),
            })
            .collect();
        assert_eq!(carried, ["engine_messages", "no_point_at_start"]);
        assert!(carried_warnings(&json!({})).is_empty());
    }

    #[test]
    fn a_csv_left_unwritten_is_a_warning_that_says_why() {
        assert_eq!(table_problem(&Ok(export::Written::Bytes(10))), None);
        let large = table_problem(&Ok(export::Written::TooLarge { cells: 30_000_000 })).unwrap();
        assert_eq!(large["code"], "csv_not_written");
        let detail = large["detail"].as_str().unwrap();
        assert!(
            detail.contains("30000000") && detail.contains("raw file"),
            "{detail}"
        );
        let broken = table_problem(&Err(std::io::Error::other("disk full"))).unwrap();
        assert_eq!(broken["code"], "csv_not_written");
        assert!(broken["detail"].as_str().unwrap().contains("disk full"));
    }

    #[test]
    fn a_probe_goes_ahead_of_the_analysis_and_after_the_saves() {
        let run = Analysis::Transient(transient(json!({ "stop_time_seconds": 1 })).unwrap());
        let probes = [".probe p(R1)".to_string()];
        assert_eq!(
            own_lines(&run, true, &probes),
            [".save all", ".probe alli", ".probe p(R1)", ".tran 5e-4 1e0"]
        );
        assert_eq!(
            own_lines(&Analysis::OperatingPoint, false, &[]),
            [".save all", ".op"]
        );
    }

    #[test]
    fn the_engine_answering_a_halt_is_not_a_complaint() {
        let root = tempfile::tempdir().unwrap();
        let roots = [root.path().to_path_buf()];
        let deck = flat(".title t\nR1 a 0 1k\n.end\n", root.path(), &roots);
        let log = "stderr doAnalyses: pause requested\nstderr run simulation interrupted\n\
                   stderr something else\n";
        let (shown, hidden) = engine_complaints(log, &deck);
        assert_eq!(shown, ["something else"]);
        assert_eq!(hidden, 0);
    }

    mod with_fake_cli {
        use super::*;
        use crate::tools::cli::test_support::write_script;

        const OP_FIXTURE: &str = "tests/fixtures/simulation/raw/op.ascii.raw";
        /// Good status fields, for a fake worker to write.
        fn good() -> String {
            format!(
                r#""protocol":{PROTOCOL_VERSION},"load_error":null,"engine_version":"44.2","code_models_loaded":["analog"],"code_models_failed":[],"exit_status":null"#
            )
        }

        /// Answers a probe as a working engine, then does `job` in the run folder.
        fn fake_worker(dir: &Path, job: &str) -> PathBuf {
            let body = format!(
                "case \"$*\" in *--probe*)\n\
                   echo '{{\"protocol\":{PROTOCOL_VERSION},\"load_error\":null,\"engine_version\":\"44.2\",\
                 \"code_models_loaded\":[\"analog\"],\"code_models_failed\":[],\"exit_callback\":false}}'\n\
                   exit 0 ;;\n\
                 esac\n{job}\n"
            );
            write_script(dir, "fake-worker", &body, "")
        }

        fn status_json(sim_status: &str, halted: &str, written: bool) -> String {
            let good = good();
            format!(
                "cat > status.json <<'JSON'\n{{{good},\"sim_status\":{sim_status},\
                 \"halted_reason\":{halted},\"results_written\":{written}}}\nJSON"
            )
        }

        fn copy_raw() -> String {
            let raw = Path::new(env!("CARGO_MANIFEST_DIR")).join(OP_FIXTURE);
            format!("cp '{}' results.raw", raw.display())
        }

        /// A transient of three points: `v(a)` climbs 0, 1, 2 over 2 s.
        fn copy_transient() -> String {
            "cat > results.raw <<'RAW'\nTitle: t\nPlotname: Transient Analysis\nFlags: real\n\
             No. Variables: 3\nNo. Points: 3\nVariables:\n\t0\ttime\ttime\n\
             \t1\tv(a)\tvoltage\n\t2\ti(m1:d)\tcurrent\nValues:\n\
             0\t0.0\n\t0.0\n\t5.0\n1\t1.0\n\t1.0\n\t5.0\n2\t2.0\n\t2.0\n\t5.0\n\nRAW"
                .to_string()
        }

        fn transient_args(root: &Path, stop: f64) -> Value {
            let mut arguments = args(root);
            arguments["analysis"] = json!({ "kind": "transient", "stop_time_seconds": stop });
            arguments["signals"] = json!(["v(a)"]);
            arguments["measurements"] = json!([
                { "name": "top", "kind": "max", "signal": "v(a)" },
                { "kind": "crossing", "signal": "v(a)", "level": 9.0 }
            ]);
            arguments
        }

        #[tokio::test]
        async fn a_finished_transient_counts_every_unit_and_a_null_is_not_a_failure() {
            let job = format!("{}\n{}", copy_transient(), status_json("0", "null", true));
            let scene = scene(&job);
            let (failed, body) = call(&scene, transient_args(&scene.root, 2.0)).await;
            assert!(!failed, "{body}");
            assert_eq!(body["outcome"]["status"], "complete", "{body}");
            assert_eq!(body["outcome"]["requested"], 4);
            assert_eq!(body["outcome"]["completed"], 4);
            assert_eq!(body["analysis"]["coverage_ratio"], 1.0);
            assert_eq!(body["analysis"]["simulated_to_seconds"], 2.0);
            assert_eq!(body["analysis"]["stop_time_seconds"], 2.0);
            assert_eq!(body["analysis"]["points"], 3);
            assert_eq!(body["measurements"][0]["value"], 2.0);
            assert_eq!(body["measurements"][0]["window_truncated"], false);
            assert!(body["measurements"][1]["value"].is_null());
            assert_eq!(body["measurements"][1]["finding"], "level_not_reached");
            assert_eq!(body["signal_summaries"][0]["final"], 2.0);
            assert!(body.get("operating_point").is_none());
            assert!(body.get("stopped").is_none());
            let dirs = run_dirs(&scene);
            let deck = std::fs::read_to_string(dirs[0].join(DECK_FILE)).unwrap();
            assert!(deck.contains("\n.tran 1e-3 2e0\n.end"), "{deck}");
        }

        /// An AC sweep of `v(a)`: each point is a frequency, then the real and
        /// imaginary parts of the signal.
        fn copy_ac(points: &[(f64, f64, f64)]) -> String {
            let mut raw = format!(
                "cat > results.raw <<'RAW'\nTitle: t\nPlotname: AC Analysis\nFlags: complex\n\
                 No. Variables: 2\nNo. Points: {}\nVariables:\n\t0\tfrequency\tfrequency grid=3\n\
                 \t1\tv(a)\tvoltage\nValues:\n",
                points.len()
            );
            for (index, (frequency, real, imaginary)) in points.iter().enumerate() {
                raw.push_str(&format!(
                    "{index}\t{frequency:e},0\n\t{real:e},{imaginary:e}\n"
                ));
            }
            raw.push_str("\nRAW");
            raw
        }

        /// A DC sweep with `v(a)` as the one signal, after the sweep's own value.
        fn copy_dc(points: &[(f64, f64)]) -> String {
            let mut raw = format!(
                "cat > results.raw <<'RAW'\nTitle: t\nPlotname: DC transfer characteristic\n\
                 Flags: real\nNo. Variables: 2\nNo. Points: {}\nVariables:\n\
                 \t0\tv(v-sweep)\tvoltage\n\t1\tv(a)\tvoltage\nValues:\n",
                points.len()
            );
            for (index, (sweep, value)) in points.iter().enumerate() {
                raw.push_str(&format!("{index}\t{sweep:e}\n\t{value:e}\n"));
            }
            raw.push_str("\nRAW");
            raw
        }

        /// The rectifier fixture with its parts out and a source of ours in.
        fn sweep_args(root: &Path, analysis: Value, source: &str) -> Value {
            let mut arguments = args(root);
            arguments["analysis"] = analysis;
            arguments["testbench"]["spice_lines"] = json!([source]);
            arguments
        }

        fn ac_args(root: &Path, analysis: Value) -> Value {
            let ac = json!({ "kind": "ac", "start_hz": 1.0, "stop_hz": 100.0 });
            let mut merged = ac;
            for (key, value) in analysis.as_object().unwrap() {
                merged[key] = value.clone();
            }
            sweep_args(root, merged, "VAC /signal_in 0 DC 0 AC 1")
        }

        /// A low-pass at 10 Hz: gain 1/sqrt(1 + (f/10)^2), phase -atan(f/10).
        fn low_pass(frequencies: &[f64]) -> Vec<(f64, f64, f64)> {
            frequencies
                .iter()
                .map(|&f| {
                    let denominator = 1.0 + (f / 10.0).powi(2);
                    (f, 1.0 / denominator, -(f / 10.0) / denominator)
                })
                .collect()
        }

        fn request_of(scene: &Scene) -> Value {
            let dirs = run_dirs(scene);
            serde_json::from_slice(&std::fs::read(dirs[0].join(REQUEST_FILE)).unwrap()).unwrap()
        }

        #[tokio::test]
        async fn a_finished_ac_sweep_gives_gain_in_db_and_tells_the_worker_its_data_is_complex() {
            let raw = copy_ac(&low_pass(&[
                1.0, 2.0, 4.0, 8.0, 10.0, 12.0, 16.0, 32.0, 64.0, 100.0,
            ]));
            let job = format!("{raw}\n{}", status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = ac_args(&scene.root, json!({ "points": 1, "scale": "decade" }));
            arguments["signals"] = json!(["v(a)"]);
            arguments["measurements"] = json!([
                { "name": "gain", "kind": "value_at", "signal": "v(a)", "at_hz": 10.0 },
                { "name": "corner", "kind": "bandwidth_3db", "signal": "v(a)" },
            ]);
            let (failed, body) = call(&scene, arguments).await;
            assert!(!failed, "{body}");
            assert_eq!(body["outcome"]["status"], "complete", "{body}");
            assert_eq!(body["outcome"]["requested"], 4);
            assert_eq!(body["analysis"]["kind"], "ac");
            assert_eq!(body["analysis"]["points"], 10);
            assert_eq!(body["analysis"]["simulated_to_hz"], 100.0);
            assert_eq!(body["analysis"]["coverage_ratio"], 1.0);
            // 20 log10(1/sqrt(2)) at the corner, and -45 degrees.
            let gain = &body["measurements"][0];
            assert!(
                (gain["value"].as_f64().unwrap() + 3.0103).abs() < 1e-3,
                "{gain}"
            );
            assert!(
                (gain["phase_degrees"].as_f64().unwrap() + 45.0).abs() < 1e-6,
                "{gain}"
            );
            assert_eq!(gain["unit"], "dB");
            assert_eq!(gain["at_hz"], 10.0);
            // From -0.04 dB at 1 Hz, the edge is -3.05 dB, a little above the 10 Hz corner.
            let corner = body["measurements"][1]["value"].as_f64().unwrap();
            assert!(corner > 10.0 && corner < 10.5, "{body}");
            let summary = &body["signal_summaries"][0];
            assert_eq!(summary["unit"], "dB");
            assert!(
                summary["high_frequency_gain_db"].as_f64().unwrap() < -19.0,
                "{summary}"
            );
            let dirs = run_dirs(&scene);
            let deck = std::fs::read_to_string(dirs[0].join(DECK_FILE)).unwrap();
            assert!(deck.contains("\n.ac dec 1 1e0 1e2\n.end"), "{deck}");
            assert!(!deck.contains(".probe p("), "{deck}");
            assert_eq!(request_of(&scene)["complex_data"], true);
            // The grid ended on the stop, so there is nothing to warn about.
            let warnings = body["warnings"].to_string();
            assert!(!warnings.contains("sweep_ends_before_stop"), "{warnings}");
        }

        #[tokio::test]
        async fn a_transient_and_a_dc_sweep_leave_the_data_real() {
            let job = format!("{}\n{}", copy_transient(), status_json("0", "null", true));
            let scene = scene(&job);
            call(&scene, transient_args(&scene.root, 2.0)).await;
            assert_eq!(request_of(&scene)["complex_data"], false);
        }

        #[tokio::test]
        async fn an_ac_sweep_that_stops_early_is_partial_and_flagged() {
            let raw = copy_ac(&low_pass(&[1.0, 10.0]));
            let job = format!("{raw}\n{}", status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = ac_args(&scene.root, json!({ "points": 1 }));
            arguments["measurements"] =
                json!([{ "kind": "value_at", "signal": "v(a)", "at_hz": 5.0 }]);
            let (failed, body) = call(&scene, arguments).await;
            assert!(!failed, "{body}");
            assert_eq!(body["outcome"]["status"], "partial", "{body}");
            assert_eq!(body["stopped"]["reason"], "incomplete_coverage");
            assert_eq!(body["measurements"][0]["window_truncated"], true);
            assert!(body["analysis"]["coverage_ratio"].as_f64().unwrap() < 1.0);
        }

        #[tokio::test]
        async fn an_octave_sweep_ending_one_step_short_is_complete_and_says_where_it_ended() {
            // 1 to 100 Hz by octave: the grid ends at 64, as ngspice does.
            let grid: Vec<f64> = (0..=6).map(|n| 2f64.powi(n)).collect();
            let job = format!(
                "{}\n{}",
                copy_ac(&low_pass(&grid)),
                status_json("0", "null", true)
            );
            let scene = scene(&job);
            let arguments = ac_args(&scene.root, json!({ "scale": "octave", "points": 1 }));
            let (failed, body) = call(&scene, arguments).await;
            assert!(!failed, "{body}");
            assert_eq!(body["outcome"]["status"], "complete", "{body}");
            assert_eq!(body["analysis"]["simulated_to_hz"], 64.0);
            let warnings = body["warnings"].to_string();
            assert!(warnings.contains("sweep_ends_before_stop"), "{warnings}");
            // Two steps short is not a finished sweep.
            let job = format!(
                "{}\n{}",
                copy_ac(&low_pass(&grid[..5])),
                status_json("0", "null", true)
            );
            let scene = self::scene(&job);
            let arguments = ac_args(&scene.root, json!({ "scale": "octave", "points": 1 }));
            let (_, body) = call(&scene, arguments).await;
            assert_eq!(body["outcome"]["status"], "partial", "{body}");
        }

        #[tokio::test]
        async fn an_ac_sweep_with_no_source_to_drive_it_is_refused_before_the_engine_starts() {
            let scene = scene("exit 1");
            let mut arguments = args(&scene.root);
            arguments["analysis"] = json!({ "kind": "ac", "start_hz": 1.0, "stop_hz": 10.0 });
            let (failed, body) = call(&scene, arguments).await;
            assert!(failed);
            assert_eq!(body["error"]["reason"], "no_ac_stimulus", "{body}");
            assert_eq!(body["error"]["stage"], "preflight");
            assert_eq!(body["outcome"]["status"], "failed");
            assert!(run_dirs(&scene).is_empty());
            // A source with only a DC value or a zero AC value drives nothing.
            for line in ["VAC /signal_in 0 DC 5", "VAC /signal_in 0 AC 0"] {
                let arguments = sweep_args(
                    &scene.root,
                    json!({ "kind": "ac", "start_hz": 1.0, "stop_hz": 10.0 }),
                    line,
                );
                let (failed, body) = call(&scene, arguments).await;
                assert!(failed, "{line}");
                assert_eq!(body["error"]["reason"], "no_ac_stimulus", "{line}");
            }
        }

        #[tokio::test]
        async fn a_ratio_of_two_signals_is_the_loop_gain_a_margin_is_read_from() {
            // v(a) is the loop's return and v(b) the injection, which is 2: L = v(a)/v(b).
            // The loop's gain falls 10 dB a decade from 20 dB at 1 Hz, and its phase
            // 67.5 degrees a decade. So unity gain is at 100 Hz with -135 degrees, and
            // -180 degrees comes at 10^(8/3) Hz with a gain of -20/3 dB.
            let points = 61;
            let mut raw = format!(
                "cat > results.raw <<'RAW'\nTitle: t\nPlotname: AC Analysis\nFlags: complex\n\
                 No. Variables: 3\nNo. Points: {points}\nVariables:\n\t0\tfrequency\tfrequency grid=3\n\
                 \t1\tv(a)\tvoltage\n\t2\tv(b)\tvoltage\nValues:\n"
            );
            for index in 0..points {
                let u = index as f64 * 0.05;
                let magnitude = 10f64.powf((20.0 - 10.0 * u) / 20.0);
                let phase = (-67.5 * u).to_radians();
                let (re, im) = (2.0 * magnitude * phase.cos(), 2.0 * magnitude * phase.sin());
                let f = 10f64.powf(u);
                raw.push_str(&format!("{index}\t{f:e},0\n\t{re:e},{im:e}\n\t2e0,0e0\n"));
            }
            raw.push_str("\nRAW");
            let job = format!("{raw}\n{}", status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = ac_args(
                &scene.root,
                json!({ "start_hz": 1.0, "stop_hz": 1000.0, "points": 20 }),
            );
            arguments["measurements"] = json!([
                { "name": "pm", "kind": "phase_margin", "signal": "v(a)", "reference_signal": "v(b)" },
                { "name": "gm", "kind": "gain_margin", "signal": "v(a)", "reference_signal": "v(b)" },
            ]);
            let (failed, body) = call(&scene, arguments).await;
            assert!(!failed, "{body}");
            let pm = &body["measurements"][0];
            assert!((pm["value"].as_f64().unwrap() - 45.0).abs() < 1e-6, "{pm}");
            assert_eq!(pm["reference_signal"], "v(b)");
            assert!(
                (pm["crossover_hz"].as_f64().unwrap() / 100.0 - 1.0).abs() < 1e-6,
                "{pm}"
            );
            let gm = &body["measurements"][1];
            assert!(
                (gm["value"].as_f64().unwrap() - 20.0 / 3.0).abs() < 1e-6,
                "{gm}"
            );
            let f180 = 10f64.powf(8.0 / 3.0);
            assert!(
                (gm["phase_crossover_hz"].as_f64().unwrap() / f180 - 1.0).abs() < 1e-6,
                "{gm}"
            );
        }

        #[tokio::test]
        async fn a_dc_sweep_reads_its_target_from_the_deck_and_measures_along_the_sweep() {
            // v(a) = 2 x over 0 to 5 by 1 V.
            let points: Vec<(f64, f64)> = (0..=5)
                .map(|n| (f64::from(n), 2.0 * f64::from(n)))
                .collect();
            let job = format!("{}\n{}", copy_dc(&points), status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = sweep_args(
                &scene.root,
                json!({ "kind": "dc_sweep", "sweeps": [
                    { "target": "vsrc", "start_value": 0.0, "stop_value": 5.0, "step_value": 1.0 }
                ] }),
                "VSRC /rect_out 0 DC 0",
            );
            arguments["signals"] = json!(["v(a)"]);
            arguments["measurements"] = json!([
                { "name": "top", "kind": "max", "signal": "v(a)" },
                { "name": "cross", "kind": "crossing", "signal": "v(a)", "level": 5.0 },
                { "name": "mid", "kind": "value_at", "signal": "v(a)", "at_sweep": 2.5 },
                { "name": "late", "kind": "min", "signal": "v(a)", "from_sweep": 3.0 },
            ]);
            let (failed, body) = call(&scene, arguments).await;
            assert!(!failed, "{body}");
            assert_eq!(body["outcome"]["status"], "complete", "{body}");
            assert_eq!(body["outcome"]["requested"], 6);
            assert_eq!(body["analysis"]["kind"], "dc_sweep");
            assert_eq!(body["analysis"]["points"], 6);
            assert_eq!(body["analysis"]["expected_points"], 6);
            assert_eq!(body["analysis"]["sweeps"][0]["element"], "VSRC");
            assert_eq!(body["analysis"]["sweeps"][0]["unit"], "V");
            let measured = |name: &str| {
                let items = body["measurements"].as_array().unwrap();
                items
                    .iter()
                    .find(|item| item["name"] == name)
                    .unwrap()
                    .clone()
            };
            assert_eq!(measured("top")["value"], 10.0);
            assert_eq!(measured("top")["at_sweep"], 5.0);
            assert_eq!(measured("cross")["value"], 2.5);
            assert_eq!(measured("cross")["unit"], "V");
            assert_eq!(measured("mid")["value"], 5.0);
            assert_eq!(measured("late")["value"], 6.0);
            let summary = &body["signal_summaries"][0];
            assert_eq!(
                (
                    summary["min"].clone(),
                    summary["max"].clone(),
                    summary["at_start"].clone(),
                    summary["at_stop"].clone()
                ),
                (json!(0.0), json!(10.0), json!(0.0), json!(10.0)),
            );
            let dirs = run_dirs(&scene);
            let deck = std::fs::read_to_string(dirs[0].join(DECK_FILE)).unwrap();
            // The deck's own spelling of the name, not the caller's.
            assert!(deck.contains("\n.dc VSRC 0e0 5e0 1e0\n.end"), "{deck}");
            assert!(deck.contains(".save all\n.probe alli"), "{deck}");
        }

        #[tokio::test]
        async fn a_falling_dc_sweep_is_measured_in_the_order_it_ran() {
            // 5 V down to 0: v(a) = 2 x, so it falls through 5 at x = 2.5.
            let points: Vec<(f64, f64)> = (0..=5)
                .map(|n| (5.0 - f64::from(n), 2.0 * (5.0 - f64::from(n))))
                .collect();
            let job = format!("{}\n{}", copy_dc(&points), status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = sweep_args(
                &scene.root,
                json!({ "kind": "dc_sweep", "sweeps": [
                    { "target": "VSRC", "start_value": 5.0, "stop_value": 0.0, "step_value": -1.0 }
                ] }),
                "VSRC /rect_out 0 DC 0",
            );
            arguments["measurements"] = json!([
                { "name": "falls", "kind": "crossing", "signal": "v(a)", "level": 5.0, "direction": "falling" },
                { "name": "rises", "kind": "crossing", "signal": "v(a)", "level": 5.0, "direction": "rising" },
                { "name": "low", "kind": "min", "signal": "v(a)", "from_sweep": 3.0, "to_sweep": 1.0 },
                { "name": "mid", "kind": "value_at", "signal": "v(a)", "at_sweep": 4.5 },
            ]);
            let (failed, body) = call(&scene, arguments).await;
            assert!(!failed, "{body}");
            let measured = |name: &str| {
                let items = body["measurements"].as_array().unwrap();
                items
                    .iter()
                    .find(|item| item["name"] == name)
                    .unwrap()
                    .clone()
            };
            // Going down the sweep, the signal falls, and the place is the caller's.
            assert_eq!(measured("falls")["value"], 2.5);
            assert!(measured("rises")["value"].is_null());
            assert_eq!(measured("low")["value"], 2.0);
            assert_eq!(measured("low")["at_sweep"], 1.0);
            assert_eq!(measured("mid")["value"], 9.0);
            let dirs = run_dirs(&scene);
            let deck = std::fs::read_to_string(dirs[0].join(DECK_FILE)).unwrap();
            assert!(deck.contains(".dc VSRC 5e0 0e0 -1e0\n"), "{deck}");
        }

        #[tokio::test]
        async fn a_dc_sweep_one_point_short_is_complete_and_says_where_it_ended() {
            // The engine adds its step, and a run of 0 to 5 can end at 4: one short.
            let points: Vec<(f64, f64)> = (0..5).map(|n| (f64::from(n), f64::from(n))).collect();
            let job = format!("{}\n{}", copy_dc(&points), status_json("0", "null", true));
            let scene = scene(&job);
            let arguments = sweep_args(
                &scene.root,
                json!({ "kind": "dc_sweep", "sweeps": [
                    { "target": "VSRC", "start_value": 0.0, "stop_value": 5.0, "step_value": 1.0 }
                ] }),
                "VSRC /rect_out 0 DC 0",
            );
            let (failed, body) = call(&scene, arguments).await;
            assert!(!failed, "{body}");
            assert_eq!(body["outcome"]["status"], "complete", "{body}");
            assert_eq!(body["analysis"]["simulated_to_value"], 4.0);
            assert_eq!(body["analysis"]["points"], 5);
            assert_eq!(body["analysis"]["expected_points"], 6);
            let warnings = body["warnings"].to_string();
            assert!(warnings.contains("sweep_ends_before_stop"), "{warnings}");
            assert!(warnings.contains("VSRC ran 5 of 6 points"), "{warnings}");
        }

        #[tokio::test]
        async fn two_dc_sweeps_are_covered_only_by_the_whole_grid() {
            let grid = |count: usize| -> Vec<(f64, f64)> {
                (0..count).map(|n| ((n % 3) as f64, n as f64)).collect()
            };
            let analysis = json!({ "kind": "dc_sweep", "sweeps": [
                { "target": "VSRC", "start_value": 0.0, "stop_value": 2.0, "step_value": 1.0 },
                { "target": "R2", "start_value": 1000.0, "stop_value": 3000.0, "step_value": 1000.0 },
            ] });
            let job = format!("{}\n{}", copy_dc(&grid(9)), status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = sweep_args(&scene.root, analysis.clone(), "VSRC /rect_out 0 DC 0");
            arguments["signals"] = json!(["v(a)"]);
            let (failed, body) = call(&scene, arguments).await;
            assert!(!failed, "{body}");
            assert_eq!(body["outcome"]["status"], "complete", "{body}");
            assert_eq!(body["analysis"]["expected_points"], 9);
            assert_eq!(body["analysis"]["sweeps"][1]["element"], "R2");
            assert_eq!(body["analysis"]["sweeps"][1]["unit"], "Ω");
            let dirs = run_dirs(&scene);
            let deck = std::fs::read_to_string(dirs[0].join(DECK_FILE)).unwrap();
            assert!(
                deck.contains(".dc VSRC 0e0 2e0 1e0 R2 1e3 3e3 1e3\n"),
                "{deck}"
            );
            // Eight of nine points is a sweep that stopped.
            let job = format!("{}\n{}", copy_dc(&grid(8)), status_json("0", "null", true));
            let scene = self::scene(&job);
            let arguments = sweep_args(&scene.root, analysis, "VSRC /rect_out 0 DC 0");
            let (_, body) = call(&scene, arguments).await;
            assert_eq!(body["outcome"]["status"], "partial", "{body}");
            assert_eq!(body["stopped"]["reason"], "incomplete_coverage");
        }

        #[tokio::test]
        async fn a_dc_sweep_of_something_the_deck_lacks_never_starts_the_worker() {
            let scene = scene("exit 1");
            let arguments = sweep_args(
                &scene.root,
                json!({ "kind": "dc_sweep", "sweeps": [
                    { "target": "VNOPE", "start_value": 0.0, "stop_value": 1.0, "step_value": 1.0 }
                ] }),
                "VSRC /rect_out 0 DC 0",
            );
            let (failed, body) = call(&scene, arguments).await;
            assert!(failed);
            assert_eq!(body["error"]["reason"], "sweep_target_not_found", "{body}");
            assert_eq!(body["error"]["stage"], "preflight");
            assert!(run_dirs(&scene).is_empty());
            // A capacitor is in the deck but the engine cannot sweep it.
            let arguments = sweep_args(
                &scene.root,
                json!({ "kind": "dc_sweep", "sweeps": [
                    { "target": "C1", "start_value": 0.0, "stop_value": 1.0, "step_value": 1.0 }
                ] }),
                "VSRC /rect_out 0 DC 0",
            );
            let (failed, body) = call(&scene, arguments).await;
            assert!(failed);
            assert_eq!(body["error"]["kind"], "invalid_argument", "{body}");
            assert_eq!(body["error"]["field"], "analysis.sweeps[0].target");
        }

        #[tokio::test]
        async fn what_a_sweep_cannot_give_is_refused_before_anything_runs() {
            let scene = scene("exit 1");
            let refused = |analysis: Value, signals: Value, measurements: Value, envelope: u64| {
                let mut arguments = args(&scene.root);
                arguments["analysis"] = analysis;
                arguments["signals"] = signals;
                arguments["measurements"] = measurements;
                arguments["envelope_points"] = json!(envelope);
                arguments
            };
            let ac = json!({ "kind": "ac", "start_hz": 1.0, "stop_hz": 10.0 });
            let dc = |count: usize| {
                let sweep = json!({ "target": "VSRC", "start_value": 0.0, "stop_value": 1.0, "step_value": 1.0 });
                json!({ "kind": "dc_sweep", "sweeps": vec![sweep; count] })
            };
            let max = json!([{ "kind": "max", "signal": "v(a)" }]);
            let margin = json!([{ "kind": "phase_margin", "signal": "v(a)" }]);
            let none = Value::Null;
            for (arguments, field) in [
                (
                    refused(ac.clone(), json!(["p(R1)"]), none.clone(), 0),
                    "signals[0]",
                ),
                (
                    refused(ac.clone(), json!(["v(a)"]), none.clone(), 4),
                    "envelope_points",
                ),
                (
                    refused(ac.clone(), none.clone(), max.clone(), 0),
                    "measurements[0].kind",
                ),
                (
                    refused(dc(1), none.clone(), margin.clone(), 0),
                    "measurements[0].kind",
                ),
                (
                    refused(
                        dc(1),
                        none.clone(),
                        json!([{ "kind": "average", "signal": "v(a)" }]),
                        0,
                    ),
                    "measurements[0].kind",
                ),
                (
                    refused(dc(1), json!(["v(a)"]), none.clone(), 4),
                    "envelope_points",
                ),
                (refused(dc(2), none.clone(), max.clone(), 0), "measurements"),
                (
                    refused(
                        ac.clone(),
                        none.clone(),
                        json!([{ "kind": "value_at", "signal": "v(a)", "at_seconds": 1.0 }]),
                        0,
                    ),
                    "measurements[0].at_seconds",
                ),
                (
                    refused(
                        ac.clone(),
                        none.clone(),
                        json!([{ "kind": "value_at", "signal": "v(a)", "at_hz": 100.0 }]),
                        0,
                    ),
                    "measurements[0].at_hz",
                ),
                (
                    refused(
                        dc(1),
                        none.clone(),
                        json!([{ "kind": "max", "signal": "v(a)", "from_hz": 1.0 }]),
                        0,
                    ),
                    "measurements[0].from_hz",
                ),
                (
                    refused(
                        dc(1),
                        none.clone(),
                        json!([{ "kind": "max", "signal": "v(a)", "reference_signal": "v(b)" }]),
                        0,
                    ),
                    "measurements[0].reference_signal",
                ),
                (
                    refused(
                        ac.clone(),
                        none.clone(),
                        json!([{ "kind": "value_at", "signal": "p(R1)", "at_hz": 5.0 }]),
                        0,
                    ),
                    "measurements[0].signal",
                ),
                (
                    refused(
                        ac.clone(),
                        none.clone(),
                        json!([{ "kind": "phase_margin", "signal": "v(a)", "reference_signal": "p(R1)" }]),
                        0,
                    ),
                    "measurements[0].reference_signal",
                ),
                (
                    refused(
                        ac.clone(),
                        none.clone(),
                        json!([{ "kind": "phase_margin", "signal": "v(a)", "reference_level": "peak" }]),
                        0,
                    ),
                    "measurements[0].reference_level",
                ),
            ] {
                let (failed, body) = call(&scene, arguments.clone()).await;
                assert!(failed, "{arguments}");
                assert_eq!(body["error"]["kind"], "invalid_argument", "{body}");
                assert_eq!(body["error"]["field"], field, "{body}");
            }
            assert!(run_dirs(&scene).is_empty());
        }

        #[tokio::test]
        async fn no_summaries_or_measurements_come_back_unless_asked_for() {
            let job = format!("{}\n{}", copy_transient(), status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = args(&scene.root);
            arguments["analysis"] = json!({ "kind": "transient", "stop_time_seconds": 2.0 });
            let (_, body) = call(&scene, arguments).await;
            assert_eq!(body["outcome"]["requested"], 2, "{body}");
            assert!(body.get("signal_summaries").is_none(), "{body}");
            assert!(body.get("measurements").is_none(), "{body}");
        }

        #[tokio::test]
        async fn a_run_that_stops_early_is_partial_and_keeps_its_data_flagged() {
            for reason in ["timeout", "size_limit"] {
                let halted = format!("\"{reason}\"");
                let job = format!("{}\n{}", copy_transient(), status_json("0", &halted, true));
                let scene = scene(&job);
                let (failed, body) = call(&scene, transient_args(&scene.root, 4.0)).await;
                assert!(!failed, "{body}");
                assert_eq!(body["outcome"]["status"], "partial", "{body}");
                assert_eq!(body["outcome"]["requested"], 4);
                assert_eq!(body["outcome"]["completed"], 1);
                assert_eq!(body["outcome"]["failed"], 3);
                assert_eq!(body["stopped"]["reason"], reason);
                assert_eq!(body["stopped"]["at_point"], 3);
                assert_eq!(body["analysis"]["coverage_ratio"], 0.5);
                assert_eq!(body["measurements"][0]["value"], 2.0, "{body}");
                assert_eq!(body["measurements"][0]["window_truncated"], true);
                assert_eq!(body["measurements"][1]["window_truncated"], true);
                assert_eq!(body["signal_summaries"][0]["window_truncated"], true);
            }
        }

        #[tokio::test]
        async fn a_run_short_of_its_stop_time_is_never_complete() {
            let job = format!("{}\n{}", copy_transient(), status_json("0", "null", true));
            let scene = scene(&job);
            let (_, body) = call(&scene, transient_args(&scene.root, 4.0)).await;
            assert_eq!(body["outcome"]["status"], "partial", "{body}");
            assert_eq!(body["stopped"]["reason"], "incomplete_coverage");
        }

        #[tokio::test]
        async fn a_run_that_lost_convergence_names_the_node() {
            let job = format!(
                "echo 'stderr doAnalyses: TRAN:  Timestep too small: trouble with node \"x1\"' \
                 > engine.log\n{}\n{}",
                copy_transient(),
                status_json("1", "null", true)
            );
            let scene = scene(&job);
            let (_, body) = call(&scene, transient_args(&scene.root, 4.0)).await;
            assert_eq!(body["stopped"]["reason"], "no_convergence", "{body}");
            assert_eq!(body["stopped"]["node"], "x1", "{body}");
        }

        #[tokio::test]
        async fn a_signal_the_run_does_not_hold_is_partial_and_names_it() {
            let job = format!("{}\n{}", copy_transient(), status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = transient_args(&scene.root, 2.0);
            arguments["signals"] = json!(["v(zzz)"]);
            arguments["measurements"] = json!([
                { "kind": "max", "signal": "v(a)" },
                { "kind": "max", "signal": "i(M1)" }
            ]);
            let (failed, body) = call(&scene, arguments).await;
            assert!(!failed, "{body}");
            assert_eq!(body["outcome"]["status"], "partial", "{body}");
            assert_eq!(body["outcome"]["requested"], 4);
            assert_eq!(body["outcome"]["failed"], 2);
            assert_eq!(body["measurements"][0]["value"], 2.0);
            assert_eq!(body["measurements"][1]["finding"], "signal_not_found");
            assert_eq!(body["measurements"][1]["pins"], json!(["d"]));
            assert_eq!(body["signal_summaries"][0]["finding"], "signal_not_found");
            assert!(body.get("stopped").is_none());
        }

        fn run_from_half_a_second(start_save: f64) -> (String, Value) {
            let job = format!(
                "{}\nsed -i.bak 's/^0\\t0.0$/0\\t0.5/' results.raw\n{}",
                copy_transient(),
                status_json("0", "null", true)
            );
            (
                job,
                json!({
                    "kind": "transient",
                    "stop_time_seconds": 2.0,
                    "start_save_seconds": start_save,
                }),
            )
        }

        #[tokio::test]
        async fn a_save_start_at_zero_does_not_silence_the_missing_point() {
            let (job, analysis) = run_from_half_a_second(0.0);
            let scene = scene(&job);
            let mut arguments = args(&scene.root);
            arguments["analysis"] = analysis;
            let (_, body) = call(&scene, arguments).await;
            let warnings = body["warnings"].to_string();
            assert!(warnings.contains("no_point_at_start"), "{body}");
        }

        #[tokio::test]
        async fn a_late_save_start_is_not_a_missing_point() {
            let (job, analysis) = run_from_half_a_second(0.4);
            let scene = scene(&job);
            let mut arguments = args(&scene.root);
            arguments["analysis"] = analysis;
            let (_, body) = call(&scene, arguments).await;
            let warnings = body["warnings"].to_string();
            assert!(!warnings.contains("no_point_at_start"), "{body}");
        }

        #[tokio::test]
        async fn a_run_that_starts_at_zero_gives_no_start_warning() {
            let job = format!("{}\n{}", copy_transient(), status_json("0", "null", true));
            let scene = scene(&job);
            let (_, body) = call(&scene, transient_args(&scene.root, 2.0)).await;
            assert_eq!(body["analysis"]["first_point_seconds"], 0.0, "{body}");
            assert!(!body["warnings"].to_string().contains("no_point_at_start"));
            assert_eq!(body["signal_summaries"][0]["window_truncated"], false);
        }

        #[tokio::test]
        async fn a_summary_signal_alone_can_make_a_run_partial() {
            let job = format!("{}\n{}", copy_transient(), status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = args(&scene.root);
            arguments["analysis"] = json!({ "kind": "transient", "stop_time_seconds": 2.0 });
            arguments["signals"] = json!(["v(zzz)"]);
            let (_, body) = call(&scene, arguments).await;
            assert_eq!(body["outcome"]["status"], "partial", "{body}");
            assert_eq!(body["outcome"]["requested"], 2);
            assert_eq!(body["outcome"]["completed"], 1);
        }

        #[tokio::test]
        async fn a_measurement_that_does_not_fit_is_refused_before_the_engine_starts() {
            let scene = scene("touch worker-ran");
            let mut arguments = transient_args(&scene.root, 2.0);
            arguments["measurements"] = json!([{ "kind": "max", "signal": "nonsense" }]);
            let (failed, body) = call(&scene, arguments).await;
            assert!(failed, "{body}");
            assert_eq!(body["error"]["kind"], "invalid_argument", "{body}");
            assert_eq!(body["error"]["field"], "measurements[0].signal");
            assert_eq!(body["outcome"]["status"], "failed");
            assert!(!scene.state.path().join("simulation").exists());
        }

        #[tokio::test]
        async fn a_size_limit_that_left_no_data_says_so() {
            fails_with(
                &status_json("0", "\"size_limit\"", false),
                "size_limit",
                "run",
            )
            .await;
        }

        #[tokio::test]
        async fn a_power_probe_names_the_deck_element_and_never_the_callers_text() {
            let job = format!("{}\n{}", copy_transient(), status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = transient_args(&scene.root, 2.0);
            arguments["testbench"]["spice_lines"] = json!(["VX a 0 1", "RX a 0 1k"]);
            arguments["signals"] = json!(["p(rx)", "p(NOPE)", "p(rx)"]);
            let (failed, body) = call(&scene, arguments).await;
            assert!(!failed, "{body}");
            let dirs = run_dirs(&scene);
            let deck = std::fs::read_to_string(dirs[0].join(DECK_FILE)).unwrap();
            assert_eq!(deck.matches(".probe p(").count(), 1, "{deck}");
            assert!(deck.contains("\n.probe p(RX)\n"), "{deck}");
            assert!(!deck.to_lowercase().contains("nope"), "{deck}");
        }

        fn with_initial_conditions(root: &Path, line: &str) -> Value {
            let mut arguments = args(root);
            arguments["analysis"] = json!({
                "kind": "transient",
                "stop_time_seconds": 2.0,
                "use_initial_conditions": true,
            });
            arguments["signals"] = json!(["p(RX)"]);
            arguments["testbench"]["spice_lines"] = json!(["VX a 0 1", "RX a 0 1k", line]);
            arguments
        }

        #[tokio::test]
        async fn a_node_condition_under_uic_keeps_every_probe_out_of_the_deck() {
            let job = format!("{}\n{}", copy_transient(), status_json("0", "null", true));
            let scene = scene(&job);
            let arguments = with_initial_conditions(&scene.root, ".ic v(a)=5");
            let (failed, body) = call(&scene, arguments).await;
            assert!(!failed, "{body}");
            let deck = std::fs::read_to_string(run_dirs(&scene)[0].join(DECK_FILE)).unwrap();
            assert!(!deck.contains(".probe"), "{deck}");
            assert!(deck.contains("uic\n"), "{deck}");
            let warnings = body["warnings"].to_string();
            assert!(warnings.contains("device_currents_not_saved"), "{body}");
        }

        #[tokio::test]
        async fn a_callers_probe_that_would_lose_a_condition_is_refused() {
            let scene = scene("touch worker-ran");
            let mut arguments = with_initial_conditions(&scene.root, ".ic v(a)=5");
            arguments["testbench"]["spice_lines"]
                .as_array_mut()
                .unwrap()
                .push(json!(".probe alli"));
            let (failed, body) = call(&scene, arguments).await;
            assert!(failed, "{body}");
            assert_eq!(
                body["error"]["reason"], "probe_loses_initial_conditions",
                "{body}"
            );
            assert_eq!(body["error"]["items"][0], ".probe alli");
            assert!(!scene.state.path().join("simulation").exists());
        }

        /// The protection stand-ins the skill shows, under their fixture names.
        const PROTECTION_DECKS: [&str; 5] = ["short", "fuse", "efuse", "uvlo", "reverse"];

        fn protection_lines(name: &str) -> Vec<String> {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/simulation/protection")
                .join(format!("{name}.sp"));
            let text = std::fs::read_to_string(path).unwrap();
            text.lines().map(str::to_owned).collect()
        }

        fn protection_args(root: &Path, lines: &[String], uic: bool) -> Value {
            let mut arguments = transient_args(root, 2.0);
            arguments["analysis"]["use_initial_conditions"] = json!(uic);
            arguments["testbench"] = json!({ "exclude_references": ["*"], "spice_lines": lines });
            arguments
        }

        /// Hosted CI has no engine, so this is the part of the contract it can hold:
        /// vetting keeps every line, preflight raises nothing, and no stand-in adds
        /// a card PR 7 refuses. `protection/spliced.sp` needs the rectifier demo's own
        /// parts, so only the live test runs it.
        #[tokio::test]
        async fn every_protection_stand_in_passes_vetting_and_preflight_untouched() {
            for name in PROTECTION_DECKS {
                for uic in [false, true] {
                    let job = format!("{}\n{}", copy_transient(), status_json("0", "null", true));
                    let scene = scene(&job);
                    let lines = protection_lines(name);
                    let (failed, body) =
                        call(&scene, protection_args(&scene.root, &lines, uic)).await;
                    assert!(!failed, "{name}: {body}");
                    assert_eq!(body["simulated_circuit"], "modified", "{name}");
                    assert_eq!(body["warnings"], json!([]), "{name}, uic {uic}");

                    let dirs = run_dirs(&scene);
                    let deck = std::fs::read_to_string(dirs[0].join(DECK_FILE)).unwrap();
                    for line in lines.iter().filter(|line| !line.starts_with('*')) {
                        assert!(
                            deck.lines().any(|kept| kept == line),
                            "{name} lost {line}\n{deck}"
                        );
                    }
                    // The only probe is Konnect's own, and with no `.ic` it stays under uic.
                    assert_eq!(deck.matches(".probe").count(), 1, "{name}: {deck}");
                    assert!(deck.contains("\n.probe alli\n"), "{name}: {deck}");
                    for refused in [".ic", ".nodeset", "\nk", ".options interp"] {
                        assert!(!deck.to_lowercase().contains(refused), "{name}: {refused}");
                    }
                }
            }
        }

        #[tokio::test]
        async fn a_callers_probe_is_left_alone_without_a_condition_to_lose() {
            let job = format!("{}\n{}", copy_transient(), status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = transient_args(&scene.root, 2.0);
            arguments["testbench"]["spice_lines"] =
                json!(["VX a 0 1", "RX a 0 1k", ".ic v(a)=5", ".probe alli"]);
            let (failed, body) = call(&scene, arguments).await;
            assert!(!failed, "{body}");
        }

        #[tokio::test]
        async fn a_device_condition_under_uic_keeps_its_probes() {
            let job = format!("{}\n{}", copy_transient(), status_json("0", "null", true));
            let scene = scene(&job);
            let arguments = with_initial_conditions(&scene.root, "CX a 0 1u IC=1");
            call(&scene, arguments).await;
            let deck = std::fs::read_to_string(run_dirs(&scene)[0].join(DECK_FILE)).unwrap();
            assert!(deck.contains(".probe alli"), "{deck}");
            assert!(deck.contains(".probe p(RX)"), "{deck}");
        }

        #[tokio::test]
        async fn a_run_with_no_point_at_zero_says_so() {
            let job = format!(
                "{}\nsed -i.bak 's/^0\\t0.0$/0\\t0.5/' results.raw\n{}",
                copy_transient(),
                status_json("0", "null", true)
            );
            let scene = scene(&job);
            let mut arguments = args(&scene.root);
            arguments["analysis"] = json!({ "kind": "transient", "stop_time_seconds": 2.0 });
            let (_, body) = call(&scene, arguments).await;
            assert_eq!(body["analysis"]["first_point_seconds"], 0.5, "{body}");
            assert!(
                body["warnings"].to_string().contains("no_point_at_start"),
                "{body}"
            );
        }

        #[tokio::test]
        async fn the_worker_is_told_the_size_limit() {
            let job = format!("{}\n{}", copy_transient(), status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = transient_args(&scene.root, 2.0);
            arguments["max_raw_bytes"] = json!(5_000_000);
            call(&scene, arguments).await;
            let dirs = run_dirs(&scene);
            let request: RunRequest =
                serde_json::from_slice(&std::fs::read(dirs[0].join(REQUEST_FILE)).unwrap())
                    .unwrap();
            assert_eq!(request.max_raw_bytes, 5_000_000);
        }

        struct Scene {
            tools: tempfile::TempDir,
            project: tempfile::TempDir,
            state: tempfile::TempDir,
            ctx: ToolContext,
            root: PathBuf,
        }

        fn scene(job: &str) -> Scene {
            let tools = tempfile::tempdir().unwrap();
            let project_dir = tempfile::tempdir().unwrap();
            let state = tempfile::tempdir().unwrap();
            let root = project(project_dir.path());
            let cli = exporting_cli(tools.path());
            let library = tools.path().join("libngspice.dylib");
            std::fs::write(&library, "").unwrap();
            let ctx = context(cli.to_str().unwrap());
            let mut runtime = SimulationRuntime::new(
                fake_worker(tools.path(), job),
                crate::simulation::SimulationSettings {
                    ngspice_library: library.display().to_string(),
                    allow_digital_models: true,
                    ..Default::default()
                },
            );
            runtime.state_root = Some(state.path().to_path_buf());
            ctx.simulation.set(Arc::new(runtime)).ok();
            Scene {
                tools,
                project: project_dir,
                state,
                ctx,
                root,
            }
        }

        /// The fixture's unmodelled parts are gone, so the deck is modified.
        fn args(root: &Path) -> Value {
            json!({
                "schematic": root,
                "analysis": { "kind": "operating_point" },
                "testbench": { "exclude_references": ["D1", "V1", "R1"] },
            })
        }

        async fn call(scene: &Scene, arguments: Value) -> (bool, Value) {
            let result = handle_run_simulation(&arguments, &scene.ctx).await.unwrap();
            let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
                panic!("expected text")
            };
            (result.is_error, serde_json::from_str(text).unwrap())
        }

        fn run_dirs(scene: &Scene) -> Vec<PathBuf> {
            // No folder at all means no run was ever made.
            let projects = scene.state.path().join("simulation");
            let Ok(projects) = std::fs::read_dir(projects) else {
                return Vec::new();
            };
            projects
                .flatten()
                .flat_map(|project| std::fs::read_dir(project.path()).unwrap().flatten())
                .map(|run| run.path())
                .collect()
        }

        #[tokio::test]
        async fn a_clean_run_reads_complete_and_says_what_it_simulated() {
            let job = format!("{}\n{}", copy_raw(), status_json("0", "null", true));
            let scene = scene(&job);
            let (failed, body) = call(&scene, args(&scene.root)).await;
            assert!(!failed, "{body}");
            assert_eq!(body["outcome"]["status"], "complete", "{body}");
            assert_eq!(body["outcome"]["requested"], 2);
            assert_eq!(body["outcome"]["completed"], 2);
            assert_eq!(body["simulated_circuit"], "modified");
            assert_eq!(
                body["circuit"]["excluded_references"],
                json!(["D1", "R1", "V1"])
            );
            assert_eq!(body["analysis"]["coverage_ratio"], 1.0);
            assert_eq!(body["operating_point"]["total"], 3);
            assert_eq!(body["operating_point"]["rows"][1]["value"], 2.0);
            assert_eq!(body["engine"]["version"], "44.2");
            assert_eq!(
                body["source_evidence"]["excludes_unsaved_editor_state"],
                true
            );
            assert_eq!(
                body["source_evidence"]["inputs_hash"]
                    .as_str()
                    .unwrap()
                    .len(),
                64
            );
            assert!(runs::is_valid_id(body["run_id"].as_str().unwrap()));
            assert!(body.get("stopped").is_none());

            let dirs = run_dirs(&scene);
            assert_eq!(dirs.len(), 1);
            let deck = std::fs::read_to_string(dirs[0].join(DECK_FILE)).unwrap();
            let tail: Vec<&str> = deck.lines().rev().take(4).collect();
            assert_eq!(tail, [".end", ".op", ".probe alli", ".save all"], "{deck}");
            assert!(dirs[0].join(OPERATING_POINT_FILE).is_file());
            assert!(body["artifacts"]["table_path"]
                .as_str()
                .unwrap()
                .ends_with(OPERATING_POINT_FILE));
            assert!(!scene.project.path().join("results.raw").exists());
        }

        #[tokio::test]
        async fn the_limit_cuts_rows_after_counting_them() {
            let job = format!("{}\n{}", copy_raw(), status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = args(&scene.root);
            arguments["limit"] = json!(1);
            let (_, body) = call(&scene, arguments).await;
            assert_eq!(body["operating_point"]["total"], 3, "{body}");
            assert_eq!(body["operating_point"]["shown"], 1);
            assert_eq!(body["operating_point"]["truncated"], true);
            assert_eq!(body["operating_point"]["rows"].as_array().unwrap().len(), 1);
        }

        #[tokio::test]
        async fn a_halted_run_with_data_is_partial_and_keeps_the_data() {
            let job = format!("{}\n{}", copy_raw(), status_json("0", "\"timeout\"", true));
            let scene = scene(&job);
            let (failed, body) = call(&scene, args(&scene.root)).await;
            assert!(!failed, "{body}");
            assert_eq!(body["outcome"]["status"], "partial");
            assert_eq!(body["outcome"]["completed"], 1);
            assert_eq!(body["outcome"]["failed"], 1);
            assert_eq!(body["stopped"]["reason"], "timeout");
            assert_eq!(body["operating_point"]["total"], 3);
        }

        async fn fails_with(job: &str, reason: &str, stage: &str) -> Value {
            let scene = scene(job);
            let (failed, body) = call(&scene, args(&scene.root)).await;
            assert!(failed, "{body}");
            assert_eq!(body["error"]["kind"], "simulation_failed", "{body}");
            assert_eq!(body["error"]["reason"], reason, "{body}");
            assert_eq!(body["error"]["stage"], stage, "{body}");
            assert_eq!(body["outcome"]["status"], "failed", "{body}");
            assert_eq!(body["outcome"]["failed"], 2);
            body
        }

        #[tokio::test]
        async fn a_crash_is_a_typed_failure_that_names_the_signal() {
            let body = fails_with("kill -SEGV $$", "engine_crashed", "run").await;
            assert_eq!(body["error"]["items"][0], "signal 11");
            assert!(body["error"]["log_path"]
                .as_str()
                .unwrap()
                .ends_with("engine.log"));

            let scene = scene("kill -SEGV $$");
            let (failed, _) = call(&scene, args(&scene.root)).await;
            assert!(failed);
            let after =
                handle_check_simulation_readiness(&json!({ "schematic": scene.root }), &scene.ctx)
                    .await
                    .unwrap();
            assert!(!after.is_error, "the server must carry on after a crash");
        }

        #[tokio::test]
        async fn every_run_failure_has_its_reason() {
            let no_data = status_json("1", "null", false);
            fails_with(&no_data, "no_convergence", "run").await;
            let no_parse = status_json("null", "null", false);
            fails_with(&no_parse, "circuit_not_parsed", "run").await;
            let halted = status_json("0", "\"timeout\"", false);
            fails_with(&halted, "timed_out", "run").await;
            let lost = status_json("0", "null", true);
            fails_with(&lost, "results_unreadable", "results").await;
            fails_with("exit 124", "timed_out", "run").await;
            let exited = format!(
                "cat > status.json <<'JSON'\n{{\"protocol\":{PROTOCOL_VERSION},\"load_error\":null,\
                 \"engine_version\":null,\"code_models_loaded\":[],\"code_models_failed\":[],\
                 \"exit_status\":1,\"sim_status\":null,\
                 \"halted_reason\":null,\"results_written\":false}}\nJSON"
            );
            fails_with(&exited, "engine_failed", "run").await;
            let unloaded = exited.replace("\"load_error\":null", "\"load_error\":\"no\"");
            let body = fails_with(&unloaded, "engine_load_failed", "engine").await;
            assert_eq!(body["error"]["items"][0], "no");
        }

        #[tokio::test]
        async fn what_the_engine_ignored_reaches_a_complete_result() {
            let job = format!(
                "echo 'stderr unrecognized parameter (bogus) - ignored' > engine.log\n{}\n{}",
                copy_raw(),
                status_json("0", "null", true)
            );
            let scene = scene(&job);
            let (failed, body) = call(&scene, args(&scene.root)).await;
            assert!(!failed, "{body}");
            assert_eq!(body["outcome"]["status"], "complete");
            let warnings = body["warnings"].to_string();
            assert!(warnings.contains("engine_messages"), "{body}");
            assert!(
                warnings.contains("unrecognized parameter (bogus)"),
                "{body}"
            );
        }

        #[tokio::test]
        async fn a_probe_past_the_budget_reads_as_out_of_time() {
            let scene = scene("touch worker-ran");
            let body = "case \"$*\" in *--probe*) exec /bin/sleep 20 ;; esac\n";
            let worker = write_script(scene.tools.path(), "fake-worker", body, "");
            let mut runtime = (**scene.ctx.simulation.get().unwrap()).clone();
            runtime.set_worker_exe(worker);
            let ctx = context("kicad-cli");
            ctx.simulation.set(Arc::new(runtime)).ok();
            let scene = Scene { ctx, ..scene };
            let mut arguments = args(&scene.root);
            arguments["timeout_seconds"] = json!(1);
            let (failed, body) = call(&scene, arguments).await;
            assert!(failed, "{body}");
            assert_eq!(body["error"]["reason"], "timed_out", "{body}");
            assert_eq!(body["error"]["stage"], "engine");
        }

        #[tokio::test]
        async fn the_budget_covers_the_export() {
            let scene = scene("touch worker-ran");
            let slow = write_script(scene.tools.path(), "slow-cli", "exec /bin/sleep 20\n", "");
            let ctx = context(slow.to_str().unwrap());
            let runtime = scene.ctx.simulation.get().unwrap().clone();
            ctx.simulation.set(runtime).ok();
            let scene = Scene { ctx, ..scene };
            let mut arguments = args(&scene.root);
            arguments["timeout_seconds"] = json!(1);
            let started = Instant::now();
            let (failed, body) = call(&scene, arguments).await;
            assert!(failed, "{body}");
            assert_eq!(
                body["error"]["reason"], "netlist_export_timed_out",
                "{body}"
            );
            assert!(started.elapsed() < Duration::from_secs(5));
        }

        #[tokio::test]
        async fn a_file_the_worker_did_not_report_is_not_read() {
            let job = format!("{}\n{}", copy_raw(), status_json("1", "null", false));
            fails_with(&job, "no_convergence", "run").await;
        }

        #[tokio::test]
        async fn a_digital_circuit_saves_no_device_currents_and_says_so() {
            let job = format!("{}\n{}", copy_raw(), status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = args(&scene.root);
            arguments["testbench"]["spice_lines"] = json!([
                "A1 [/rect_out] [dout] inv",
                ".model inv d_inverter",
                "R9 dout 0 1k",
            ]);
            let (failed, body) = call(&scene, arguments).await;
            assert!(!failed, "{body}");
            assert!(body["warnings"]
                .to_string()
                .contains("device_currents_not_saved"));
            let deck = std::fs::read_to_string(run_dirs(&scene)[0].join(DECK_FILE)).unwrap();
            assert!(deck.contains(".save all"), "{deck}");
            assert!(!deck.contains(".probe alli"), "{deck}");
        }

        #[tokio::test]
        async fn a_constants_only_file_reads_as_no_results() {
            let raw = "Title: t\nPlotname: constants\nFlags: real\nNo. Variables: 1\n\
                       No. Points: 1\nVariables:\n\t0\tpi\tnotype\nValues:\n 0\t3.0\n\n";
            let job = format!(
                "printf '{raw}' > results.raw\n{}",
                status_json("0", "null", true)
            );
            fails_with(&job, "results_unreadable", "results").await;
        }

        #[tokio::test]
        async fn a_failed_preflight_never_starts_the_worker() {
            let job = "touch worker-ran";
            let scene = scene(job);
            let arguments = json!({
                "schematic": scene.root,
                "analysis": { "kind": "operating_point" },
            });
            let (failed, body) = call(&scene, arguments).await;
            assert!(failed, "{body}");
            assert_eq!(body["error"]["reason"], "unmodelled_symbols", "{body}");
            assert_eq!(body["error"]["stage"], "preflight");
            assert_eq!(body["error"]["items"], json!(["D1", "V1"]));
            assert_eq!(body["outcome"]["status"], "failed");
            assert!(!scene.state.path().join("simulation").exists());
        }

        #[tokio::test]
        async fn a_floating_group_refuses_a_dc_sweep_but_not_an_ac_one() {
            let sweep = json!({ "kind": "dc_sweep", "sweeps": [
                { "target": "Vx", "start_value": 0.0, "stop_value": 1.0, "step_value": 1.0 }
            ] });
            let scene = scene("touch worker-ran");
            let mut arguments = args(&scene.root);
            arguments["analysis"] = sweep;
            arguments["testbench"]["spice_lines"] = json!(["Vx fl1 fl2 1", "Rx fl1 fl2 1k"]);
            let (failed, body) = call(&scene, arguments).await;
            assert!(failed, "{body}");
            assert_eq!(body["error"]["reason"], "floating_subcircuit", "{body}");
            assert!(!scene.state.path().join("simulation").exists());
            // Series capacitors give a true AC answer, so only a DC solve refuses.
            let mut arguments = args(&scene.root);
            arguments["analysis"] = json!({ "kind": "ac", "start_hz": 1.0, "stop_hz": 10.0 });
            arguments["testbench"]["spice_lines"] = json!(["Vx fl1 fl2 AC 1", "Rx fl1 fl2 1k"]);
            let (_, body) = call(&scene, arguments).await;
            assert_ne!(body["error"]["reason"], "floating_subcircuit", "{body}");
        }

        #[tokio::test]
        async fn a_floating_group_refuses_an_operating_point() {
            let scene = scene("touch worker-ran");
            let mut arguments = args(&scene.root);
            arguments["testbench"]["spice_lines"] = json!(["Vx fl1 fl2 1", "Rx fl1 fl2 1k"]);
            let (failed, body) = call(&scene, arguments).await;
            assert!(failed, "{body}");
            assert_eq!(body["error"]["reason"], "floating_subcircuit", "{body}");
            assert!(!scene.state.path().join("simulation").exists());
        }

        #[tokio::test]
        async fn a_refused_deck_names_the_form_and_stops_there() {
            let scene = scene("touch worker-ran");
            let mut arguments = args(&scene.root);
            arguments["testbench"]["spice_lines"] = json!([".control"]);
            let (failed, body) = call(&scene, arguments).await;
            assert!(failed, "{body}");
            assert_eq!(body["error"]["reason"], "unsafe_deck_content", "{body}");
            assert!(!scene.state.path().join("simulation").exists());
        }

        #[tokio::test]
        async fn a_broken_export_is_a_typed_failure() {
            let scene = scene("touch worker-ran");
            let broken = write_script(scene.tools.path(), "broken-cli", "exit 2\n", "");
            let ctx = context(broken.to_str().unwrap());
            let runtime = scene.ctx.simulation.get().unwrap().clone();
            ctx.simulation.set(runtime).ok();
            let scene = Scene { ctx, ..scene };
            let (failed, body) = call(&scene, args(&scene.root)).await;
            assert!(failed, "{body}");
            assert_eq!(body["error"]["reason"], "netlist_export_failed", "{body}");
            assert_eq!(body["error"]["stage"], "export");
        }

        #[tokio::test]
        async fn arguments_and_missing_engines_are_refused_with_an_outcome() {
            let scene = scene("touch worker-ran");
            let mut other = args(&scene.root);
            other["analysis"]["kind"] = json!("transient");
            let (failed, body) = call(&scene, other).await;
            assert!(failed);
            assert_eq!(body["error"]["kind"], "invalid_argument", "{body}");
            assert_eq!(body["outcome"]["status"], "failed");

            let missing = json!({
                "schematic": scene.project.path().join("gone.kicad_sch"),
                "analysis": { "kind": "operating_point" },
            });
            let (failed, body) = call(&scene, missing).await;
            assert!(failed);
            assert_eq!(body["error"]["kind"], "file_not_found", "{body}");
            assert_eq!(body["outcome"]["status"], "failed");

            let bare = Scene {
                ctx: context("kicad-cli"),
                ..scene
            };
            let (failed, body) = call(&bare, args(&bare.root)).await;
            assert!(failed);
            assert_eq!(body["error"]["reason"], "engine_load_failed", "{body}");
            assert_eq!(body["outcome"]["status"], "failed");
        }

        #[tokio::test]
        async fn a_missing_library_lists_where_it_looked() {
            let scene = scene("touch worker-ran");
            let ctx = context("kicad-cli");
            let missing = scene.tools.path().join("libngspice-gone.dylib");
            let runtime = SimulationRuntime::new(
                scene.tools.path().join("fake-worker"),
                crate::simulation::SimulationSettings {
                    ngspice_library: missing.display().to_string(),
                    ..Default::default()
                },
            );
            ctx.simulation.set(Arc::new(runtime)).ok();
            let scene = Scene { ctx, ..scene };
            let (failed, body) = call(&scene, args(&scene.root)).await;
            assert!(failed);
            assert_eq!(body["error"]["reason"], "engine_not_found", "{body}");
            assert_eq!(body["error"]["items"][0], missing.display().to_string());
        }

        /// Scans argv, so it survives argument reordering, and logs each format.
        fn exporting_cli(dir: &Path) -> std::path::PathBuf {
            let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/simulation/rectifier_unmodelled");
            let body = format!(
                "format=''; output=''\n\
                 while [ $# -gt 0 ]; do\n\
                   case \"$1\" in\n\
                     --output) output=\"$2\"; shift ;;\n\
                     --format) format=\"$2\"; shift ;;\n\
                   esac\n\
                   shift\n\
                 done\n\
                 if read -r line; then echo stdin-open >> '{log}'; fi\n\
                 echo \"$format\" >> '{log}'\n\
                 case \"$format\" in\n\
                   spice) cp '{fixture}.cir' \"$output\" ;;\n\
                   *) cp '{fixture}.net' \"$output\" ;;\n\
                 esac\n",
                log = dir.join("calls.log").display(),
                fixture = fixtures.display(),
            );
            write_script(dir, "fake-kicad-cli", &body, "")
        }

        fn project(dir: &Path) -> std::path::PathBuf {
            let root = dir.join("top.kicad_sch");
            std::fs::write(&root, sheet()).unwrap();
            std::fs::write(dir.join("top.kicad_pro"), "{}").unwrap();
            root
        }

        #[tokio::test]
        async fn readiness_exports_kicad_format_first_and_lists_the_design() {
            let tools = tempfile::tempdir().unwrap();
            let project_dir = tempfile::tempdir().unwrap();
            let root = project(project_dir.path());
            let fake = exporting_cli(tools.path());

            let design = inspect_design(fake.to_str().unwrap(), &root, &no_inputs())
                .await
                .unwrap();
            assert_eq!(design.elements.len(), 5);
            assert!(design.nets.iter().any(|net| net.kicad_name == "/rect_out"));

            let calls = std::fs::read_to_string(tools.path().join("calls.log")).unwrap();
            assert_eq!(calls, "kicadsexpr\nspice\n");
        }

        #[tokio::test]
        async fn the_export_never_writes_into_the_project() {
            use std::os::unix::fs::PermissionsExt;
            let tools = tempfile::tempdir().unwrap();
            let project_dir = tempfile::tempdir().unwrap();
            let root = project(project_dir.path());
            let fake = exporting_cli(tools.path());
            let read_only = std::fs::Permissions::from_mode(0o555);
            std::fs::set_permissions(project_dir.path(), read_only).unwrap();

            let result = inspect_design(fake.to_str().unwrap(), &root, &no_inputs()).await;

            let writable = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(project_dir.path(), writable).unwrap();
            assert!(result.is_ok(), "{:?}", result.err());
            assert_eq!(std::fs::read_dir(project_dir.path()).unwrap().count(), 2);
        }

        #[tokio::test]
        async fn a_nonzero_exit_fails_even_when_a_file_was_written() {
            let tools = tempfile::tempdir().unwrap();
            let project_dir = tempfile::tempdir().unwrap();
            let root = project(project_dir.path());
            let fake = write_script(
                tools.path(),
                "failing-cli",
                "while [ $# -gt 1 ]; do\n\
                   if [ \"$1\" = --output ]; then echo written > \"$2\"; fi\n\
                   shift\n\
                 done\n\
                 echo 'Simulation model library not found'\n\
                 exit 2\n",
                "",
            );
            let error = inspect_design(fake.to_str().unwrap(), &root, &no_inputs())
                .await
                .err()
                .unwrap();
            assert!(
                format!("{error:#}").contains("library not found"),
                "{error:#}"
            );
        }

        #[tokio::test]
        async fn an_empty_export_is_a_failure() {
            let tools = tempfile::tempdir().unwrap();
            let project_dir = tempfile::tempdir().unwrap();
            let root = project(project_dir.path());
            let fake = write_script(
                tools.path(),
                "silent-cli",
                "while [ $# -gt 1 ]; do\n\
                   if [ \"$1\" = --output ]; then : > \"$2\"; fi\n\
                   shift\n\
                 done\n",
                "",
            );
            assert!(inspect_design(fake.to_str().unwrap(), &root, &no_inputs())
                .await
                .is_err());
        }

        #[tokio::test]
        async fn a_hung_export_is_killed_at_the_timeout() {
            let tools = tempfile::tempdir().unwrap();
            let pid_file = tools.path().join("pid");
            let fake = write_script(
                tools.path(),
                "hung-cli",
                &format!("echo $$ > '{}'\nexec /bin/sleep 30\n", pid_file.display()),
                "",
            );
            let result = cli::export_simulation_netlists(
                fake.to_str().unwrap(),
                Path::new("/x.kicad_sch"),
                tools.path(),
                Duration::from_millis(500),
            )
            .await;
            assert!(result.is_err());
            let pid = std::fs::read_to_string(&pid_file).unwrap();
            let pid = pid.trim();
            let mut gone = false;
            for _ in 0..50 {
                let alive = std::process::Command::new("kill")
                    .args(["-0", pid])
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap()
                    .success();
                if !alive {
                    gone = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(gone, "the hung export is still running");
        }

        // --- stored runs -----------------------------------------------------

        async fn listed(scene: &Scene, extra: Value) -> (bool, Value) {
            let mut arguments = json!({ "schematic": scene.root });
            for (key, value) in extra.as_object().into_iter().flatten() {
                arguments[key] = value.clone();
            }
            let result = handle_list_simulation_runs(&arguments, &scene.ctx)
                .await
                .unwrap();
            read_body(result)
        }

        async fn measured(scene: &Scene, arguments: Value) -> (bool, Value) {
            let result = handle_measure_simulation_run(&arguments, &scene.ctx)
                .await
                .unwrap();
            read_body(result)
        }

        fn read_body(result: crate::mcp::protocol::CallToolResult) -> (bool, Value) {
            let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
                panic!("expected text")
            };
            (result.is_error, serde_json::from_str(text).unwrap())
        }

        fn id_of(body: &Value) -> String {
            body["run_id"].as_str().unwrap().to_string()
        }

        fn transient_job() -> String {
            format!("{}\n{}", copy_transient(), status_json("0", "null", true))
        }

        fn first_run_dir(scene: &Scene) -> PathBuf {
            run_dirs(scene).remove(0)
        }

        #[tokio::test]
        async fn a_run_is_recorded_and_listed_with_what_it_proved() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            let (failed, list) = listed(&scene, json!({})).await;
            assert!(!failed, "{list}");
            assert_eq!(list["total"], 1);
            assert_eq!(list["shown"], 1);
            assert_eq!(list["truncated"], false);
            assert_eq!(list["without_record"], 0);
            let entry = &list["runs"][0];
            assert_eq!(entry["run_id"], run["run_id"]);
            assert_eq!(entry["analysis_kind"], "transient");
            assert_eq!(entry["run_status"], "complete");
            assert_eq!(entry["simulated_circuit"], "modified");
            assert_eq!(entry["coverage_ratio"], 1.0);
            assert_eq!(entry["inputs_changed_since_run"], false);
            let created = entry["created_at"].as_str().unwrap();
            assert!(created.ends_with('Z') && created.contains('T'), "{created}");
            // The record keeps the names a later call needs to find a signal.
            let record = runs::read_record(&first_run_dir(&scene)).unwrap();
            assert!(record.nets.iter().any(|net| net.kicad_name == "/rect_out"));
            assert!(record
                .elements
                .iter()
                .any(|element| element.reference == "R2"));
            assert_eq!(record.schematic, scene.root.canonicalize().unwrap());
            assert_eq!(record.inputs_hash, run["source_evidence"]["inputs_hash"]);
        }

        #[tokio::test]
        async fn a_new_measurement_needs_no_new_simulation() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            // With no worker left, any attempt to simulate again would fail loudly.
            std::fs::remove_file(scene.tools.path().join("fake-worker")).unwrap();
            let (failed, body) = measured(
                &scene,
                json!({
                    "run_id": run["run_id"],
                    "signals": ["v(a)"],
                    "measurements": [
                        { "name": "bottom", "kind": "min", "signal": "v(a)" },
                        { "name": "top", "kind": "max", "signal": "v(a)" },
                    ],
                }),
            )
            .await;
            assert!(!failed, "{body}");
            assert_eq!(run_dirs(&scene).len(), 1);
            assert_eq!(body["outcome"]["status"], "complete", "{body}");
            assert_eq!(
                body["outcome"]["requested"], 3,
                "two measurements and the summaries"
            );
            assert_eq!(body["outcome"]["completed"], 3);
            assert_eq!(body["measurements"][0]["value"], 0.0);
            assert_eq!(body["measurements"][1]["value"], 2.0);
            assert_eq!(body["signal_summaries"][0]["final"], 2.0);
            assert_eq!(body["run_id"], run["run_id"]);
            assert_eq!(body["run_status"], "complete");
            assert_eq!(body["simulated_circuit"], "modified");
            assert_eq!(body["coverage_ratio"], 1.0);
            assert_eq!(body["inputs_changed_since_run"], false);
            assert_eq!(body["analysis_kind"], "transient");
            // Where the run ended and what it left out come with its measurements.
            assert_eq!(body["analysis"], run["analysis"], "{body}");
            assert_eq!(body["circuit"], run["circuit"], "{body}");
            assert_eq!(
                body["circuit"]["excluded_references"],
                json!(["D1", "R1", "V1"])
            );
            // The run's own answer and the stored one are the same measurement.
            assert_eq!(
                run["measurements"][0]["value"],
                body["measurements"][1]["value"]
            );
        }

        #[tokio::test]
        async fn a_measurement_that_does_not_fit_the_stored_analysis_is_refused() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            for measurement in [
                json!({ "kind": "bandwidth_3db", "signal": "v(a)" }),
                json!({ "kind": "value_at", "signal": "v(a)", "at_hz": 5.0 }),
                json!({ "kind": "max", "signal": "v(a)", "from_sweep": 1.0 }),
            ] {
                let (failed, body) = measured(
                    &scene,
                    json!({ "run_id": run["run_id"], "measurements": [measurement] }),
                )
                .await;
                assert!(failed, "{body}");
                assert_eq!(body["error"]["kind"], "invalid_argument", "{body}");
                assert_eq!(body["outcome"]["status"], "failed", "{body}");
                assert_eq!(body["outcome"]["requested"], 1);
                assert_eq!(body["outcome"]["failed"], 1);
            }
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"], "measurements": [] }),
            )
            .await;
            assert!(failed);
            assert_eq!(body["error"]["field"], "measurements", "{body}");
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"], "envelope_points": 4,
                        "measurements": [{ "kind": "max", "signal": "v(a)" }] }),
            )
            .await;
            assert!(failed);
            assert_eq!(body["error"]["field"], "envelope_points", "{body}");
        }

        #[tokio::test]
        async fn an_id_that_could_leave_the_store_is_refused_before_any_lookup() {
            let scene = scene(&transient_job());
            call(&scene, transient_args(&scene.root, 2.0)).await;
            let state = scene.state.path().display().to_string();
            let hostile = [
                "../../../../etc/passwd".to_string(),
                "..".to_string(),
                String::new(),
                "/etc/passwd".to_string(),
                "3fa9c1d27b04-20261030T133512-7f3a/..".to_string(),
                "3fa9c1d27b04-20261030T133512-7f3a\n".to_string(),
                "3fa9c1d27b04-20261030T133512-7f3a/../../x".to_string(),
                "..\\..\\3fa9c1d27b04-20261030T133512-7f3a".to_string(),
                format!("{state}/simulation"),
                "3fa9c1d27b04-20261030T133512-7F3A".to_string(),
                "\u{5e9}fa9c1d27b04-20261030T133512-7f3a".to_string(),
            ];
            for id in hostile {
                let (failed, body) = measured(
                    &scene,
                    json!({ "run_id": id, "measurements": [{ "kind": "max", "signal": "v(a)" }] }),
                )
                .await;
                assert!(failed, "{id:?}: {body}");
                assert_eq!(body["error"]["kind"], "invalid_argument", "{id:?}: {body}");
                assert_eq!(body["error"]["field"], "run_id", "{id:?}: {body}");
                assert_eq!(body["outcome"]["status"], "failed", "{id:?}: {body}");
                assert_eq!(
                    body["outcome"]["target"], "(not a run id)",
                    "{id:?}: {body}"
                );
            }
            // Not a string at all.
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": 7, "measurements": [{ "kind": "max", "signal": "v(a)" }] }),
            )
            .await;
            assert!(failed);
            assert_eq!(body["error"]["field"], "run_id", "{body}");
        }

        #[cfg(unix)]
        #[tokio::test]
        async fn a_link_in_the_store_is_no_run() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            let id = id_of(&run);
            let real = first_run_dir(&scene);
            let outside = tempfile::tempdir().unwrap();
            for entry in std::fs::read_dir(&real).unwrap().flatten() {
                std::fs::copy(entry.path(), outside.path().join(entry.file_name())).ok();
            }
            std::fs::remove_dir_all(&real).unwrap();
            std::os::unix::fs::symlink(outside.path(), &real).unwrap();
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": id, "measurements": [{ "kind": "max", "signal": "v(a)" }] }),
            )
            .await;
            assert!(failed, "{body}");
            assert_eq!(body["error"]["reason"], "run_not_found", "{body}");
            // The linked folder is not listed either.
            let (_, list) = listed(&scene, json!({})).await;
            assert_eq!(list["total"], 0, "{list}");
        }

        #[tokio::test]
        async fn a_missing_run_and_a_damaged_one_have_their_own_reasons() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            let ask = json!({ "run_id": run["run_id"],
                              "measurements": [{ "kind": "max", "signal": "v(a)" }] });
            let dir = first_run_dir(&scene);
            let reason = |body: &Value| body["error"]["reason"].as_str().unwrap().to_string();

            let gone = json!({ "run_id": "3fa9c1d27b04-20261030T133512-7f3a",
                               "measurements": [{ "kind": "max", "signal": "v(a)" }] });
            let (failed, body) = measured(&scene, gone).await;
            assert!(failed);
            assert_eq!(reason(&body), "run_not_found", "{body}");
            assert_eq!(body["error"]["kind"], "simulation_failed");
            assert_eq!(body["outcome"]["status"], "failed");

            let raw = std::fs::read(dir.join(RAW_FILE)).unwrap();
            std::fs::write(dir.join(RAW_FILE), b"not a raw file").unwrap();
            let (failed, body) = measured(&scene, ask.clone()).await;
            assert!(failed);
            assert_eq!(reason(&body), "results_unreadable", "{body}");
            assert_eq!(body["error"]["stage"], "results");
            std::fs::remove_file(dir.join(RAW_FILE)).unwrap();
            let (_, body) = measured(&scene, ask.clone()).await;
            assert_eq!(reason(&body), "results_unreadable", "{body}");
            std::fs::write(dir.join(RAW_FILE), &raw).unwrap();

            std::fs::remove_file(dir.join(crate::simulation::protocol::STATUS_FILE)).unwrap();
            let (_, body) = measured(&scene, ask.clone()).await;
            assert_eq!(reason(&body), "results_unreadable", "{body}");

            let record = std::fs::read(dir.join(runs::RECORD_FILE)).unwrap();
            std::fs::write(dir.join(runs::RECORD_FILE), b"{ not json").unwrap();
            let (_, body) = measured(&scene, ask.clone()).await;
            assert_eq!(reason(&body), "results_unreadable", "{body}");
            let mut wrong = serde_json::from_slice::<Value>(&record).unwrap();
            wrong["version"] = json!(99);
            std::fs::write(dir.join(runs::RECORD_FILE), wrong.to_string()).unwrap();
            let (_, body) = measured(&scene, ask.clone()).await;
            assert_eq!(reason(&body), "results_unreadable", "{body}");
            let mut wrong = serde_json::from_slice::<Value>(&record).unwrap();
            wrong["analysis"] = json!({ "kind": "transient" });
            std::fs::write(dir.join(runs::RECORD_FILE), wrong.to_string()).unwrap();
            let (_, body) = measured(&scene, ask.clone()).await;
            assert_eq!(reason(&body), "results_unreadable", "{body}");

            std::fs::remove_file(dir.join(runs::RECORD_FILE)).unwrap();
            let (_, body) = measured(&scene, ask).await;
            assert_eq!(reason(&body), "run_not_found", "{body}");
        }

        #[tokio::test]
        async fn a_partial_run_stays_partial_when_it_is_measured_again() {
            let job = format!(
                "{}\n{}",
                copy_transient(),
                status_json("0", "\"timeout\"", true)
            );
            let scene = scene(&job);
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            assert_eq!(run["outcome"]["status"], "partial", "{run}");
            let (_, list) = listed(&scene, json!({})).await;
            assert_eq!(list["runs"][0]["run_status"], "partial", "{list}");
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"],
                        "measurements": [{ "kind": "max", "signal": "v(a)" }] }),
            )
            .await;
            assert!(!failed, "{body}");
            assert_eq!(body["run_status"], "partial");
            assert_eq!(body["outcome"]["status"], "partial", "{body}");
            assert_eq!(body["stopped"]["reason"], "timeout", "{body}");
            assert_eq!(body["measurements"][0]["window_truncated"], true, "{body}");
        }

        #[tokio::test]
        async fn a_changed_schematic_reads_as_changed_inputs() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            let ask = json!({ "run_id": run["run_id"],
                              "measurements": [{ "kind": "max", "signal": "v(a)" }] });
            let original = std::fs::read_to_string(&scene.root).unwrap();
            std::fs::write(&scene.root, format!("{original} ")).unwrap();
            let (_, list) = listed(&scene, json!({})).await;
            assert_eq!(list["runs"][0]["inputs_changed_since_run"], true, "{list}");
            let (_, body) = measured(&scene, ask.clone()).await;
            assert_eq!(body["inputs_changed_since_run"], true, "{body}");

            std::fs::write(&scene.root, &original).unwrap();
            let (_, body) = measured(&scene, ask.clone()).await;
            assert_eq!(body["inputs_changed_since_run"], false, "{body}");

            // The project file is an input too.
            let project = scene.root.with_extension("kicad_pro");
            std::fs::write(&project, "{\"a\":1}").unwrap();
            let (_, body) = measured(&scene, ask.clone()).await;
            assert_eq!(body["inputs_changed_since_run"], true, "{body}");
            std::fs::write(&project, "{}").unwrap();

            // With the sheet gone, nothing can be said.
            std::fs::remove_file(&scene.root).unwrap();
            let (failed, body) = measured(&scene, ask).await;
            assert!(!failed, "{body}");
            assert!(body["inputs_changed_since_run"].is_null(), "{body}");
            let (failed, list) = listed(&scene, json!({})).await;
            assert!(failed);
            assert_eq!(list["error"]["kind"], "file_not_found", "{list}");
        }

        #[tokio::test]
        async fn a_model_file_the_deck_inlined_is_an_input_that_can_change() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            let model = scene.project.path().join("part.lib");
            std::fs::write(&model, ".model D1N dioc\n").unwrap();
            let dir = first_run_dir(&scene);
            let path = dir.join(runs::RECORD_FILE);
            let mut record: RunRecord =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            // Pretend the deck inlined it, with the hash it had then.
            let before = stored::CurrentSources::of(&scene.root);
            record.inlined_files = vec![model.clone()];
            record.inputs_hash =
                inputs_hash(before.0.clone().unwrap(), &record.inlined_files).unwrap();
            std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
            let ask = json!({ "run_id": run["run_id"],
                              "measurements": [{ "kind": "max", "signal": "v(a)" }] });
            let (_, body) = measured(&scene, ask.clone()).await;
            assert_eq!(body["inputs_changed_since_run"], false, "{body}");
            std::fs::write(&model, ".model D1N dioc2\n").unwrap();
            let (_, body) = measured(&scene, ask.clone()).await;
            assert_eq!(body["inputs_changed_since_run"], true, "{body}");
            std::fs::remove_file(&model).unwrap();
            let (_, body) = measured(&scene, ask).await;
            assert_eq!(body["inputs_changed_since_run"], true, "{body}");
        }

        #[tokio::test]
        async fn a_signal_the_run_lacks_fails_alone_and_unsaved_power_says_so() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["p(R2)"],
                        "measurements": [
                            { "kind": "max", "signal": "v(a)" },
                            { "kind": "max", "signal": "v(nowhere)" },
                        ] }),
            )
            .await;
            assert!(!failed, "{body}");
            assert_eq!(body["measurements"][0]["value"], 2.0);
            assert_eq!(
                body["measurements"][1]["finding"], "signal_not_found",
                "{body}"
            );
            assert_eq!(body["outcome"]["status"], "partial", "{body}");
            assert_eq!(body["outcome"]["requested"], 3);
            assert_eq!(body["outcome"]["completed"], 1);
            assert_eq!(body["outcome"]["failed"], 2);
            let warnings = body["warnings"].to_string();
            assert!(warnings.contains("power_not_saved"), "{warnings}");
            assert!(warnings.contains("p(R2)"), "{warnings}");
        }

        #[tokio::test]
        async fn the_list_is_newest_first_cut_to_the_limit_and_counts_what_has_no_record() {
            let scene = scene("exit 0");
            let first = runs::create(scene.state.path(), &scene.root).unwrap();
            let project = first.dir.parent().unwrap().to_path_buf();
            let hash = first.id[..12].to_string();
            let record = |circuit: &str| {
                RunRecord::new(RecordFields {
                    schematic: scene.root.clone(),
                    analysis: json!({ "kind": "operating_point" }),
                    run_status: RunState::Complete,
                    stopped: None,
                    simulated_circuit: circuit.to_string(),
                    circuit: Value::Null,
                    result: Value::Null,
                    coverage_ratio: 1.0,
                    inputs_hash: String::new(),
                    inlined_files: Vec::new(),
                    nets: Vec::new(),
                    elements: Vec::new(),
                    warnings: Vec::new(),
                })
            };
            let mut made = Vec::new();
            for (stamp, circuit) in [
                ("20250102T000000", "as_drawn"),
                ("20250104T000000", "modified"),
                ("20250103T000000", "as_drawn"),
            ] {
                let id = format!("{hash}-{stamp}-0000");
                std::fs::create_dir(project.join(&id)).unwrap();
                runs::write_record(&project.join(&id), &record(circuit)).unwrap();
                made.push(id);
            }
            std::fs::create_dir(project.join("not-a-run")).unwrap();
            std::fs::write(project.join(format!("{hash}-20250105T000000-0000")), "").unwrap();

            let (failed, list) = listed(&scene, json!({})).await;
            assert!(!failed, "{list}");
            let ids: Vec<&str> = list["runs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|run| run["run_id"].as_str().unwrap())
                .collect();
            assert_eq!(ids, [made[1].as_str(), made[2].as_str(), made[0].as_str()]);
            assert_eq!(list["total"], 3);
            assert_eq!(list["without_record"], 1, "the empty folder made first");
            assert_eq!(list["runs"][0]["simulated_circuit"], "modified");
            assert_eq!(list["runs"][0]["analysis_kind"], "operating_point");
            assert_eq!(list["runs"][0]["created_at"], "2025-01-04T00:00:00Z");

            let (_, list) = listed(&scene, json!({ "limit": 2 })).await;
            assert_eq!(list["shown"], 2);
            assert_eq!(list["total"], 3);
            assert_eq!(list["truncated"], true);
            assert_eq!(list["runs"][1]["run_id"], made[2].as_str());
        }

        #[tokio::test]
        async fn no_runs_is_an_empty_list_and_a_missing_sheet_is_refused() {
            let scene = scene("exit 0");
            let (failed, list) = listed(&scene, json!({})).await;
            assert!(!failed, "{list}");
            assert_eq!(list["total"], 0);
            assert_eq!(list["runs"], json!([]));
            let missing = scene.project.path().join("nope.kicad_sch");
            let result = handle_list_simulation_runs(&json!({ "schematic": missing }), &scene.ctx)
                .await
                .unwrap();
            let (failed, body) = read_body(result);
            assert!(failed);
            assert_eq!(body["error"]["kind"], "file_not_found", "{body}");
        }

        #[tokio::test]
        async fn a_run_that_failed_leaves_no_record_to_list_or_measure() {
            let scene = scene("kill -ABRT $$");
            let (failed, _) = call(&scene, transient_args(&scene.root, 2.0)).await;
            assert!(failed);
            let (_, list) = listed(&scene, json!({})).await;
            assert_eq!(list["total"], 0, "{list}");
            assert_eq!(list["without_record"], 1, "{list}");
        }

        #[tokio::test]
        async fn the_csv_is_the_raw_data_as_text() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            let path = run["artifacts"]["csv_path"].as_str().unwrap();
            assert!(path.ends_with("results.csv"), "{path}");
            assert_eq!(
                std::fs::read_to_string(path).unwrap(),
                "time,v(a),i(m1:d)\n0e0,0e0,5e0\n1e0,1e0,5e0\n2e0,2e0,5e0\n"
            );
            assert_eq!(
                run["artifacts"]["csv_bytes"],
                std::fs::metadata(path).unwrap().len()
            );
        }

        #[tokio::test]
        async fn an_operating_point_csv_reads_down_the_rows() {
            let job = format!("{}\n{}", copy_raw(), status_json("0", "null", true));
            let scene = scene(&job);
            let (_, run) = call(&scene, args(&scene.root)).await;
            let text =
                std::fs::read_to_string(run["artifacts"]["csv_path"].as_str().unwrap()).unwrap();
            assert!(text.starts_with("signal,value\n"), "{text}");
            assert_eq!(text.lines().count(), 4, "{text}");
        }

        #[tokio::test]
        async fn an_ac_csv_has_a_real_scale_and_a_stored_ac_run_measures_like_the_run() {
            let raw = copy_ac(&low_pass(&[
                1.0, 2.0, 4.0, 8.0, 10.0, 12.0, 16.0, 32.0, 64.0, 100.0,
            ]));
            let job = format!("{raw}\n{}", status_json("0", "null", true));
            let scene = scene(&job);
            let measurements = json!([
                { "name": "gain", "kind": "value_at", "signal": "v(a)", "at_hz": 10.0 },
                { "name": "corner", "kind": "bandwidth_3db", "signal": "v(a)" },
            ]);
            let mut arguments = ac_args(&scene.root, json!({ "points": 1, "scale": "decade" }));
            arguments["measurements"] = measurements.clone();
            let (_, run) = call(&scene, arguments).await;
            let text =
                std::fs::read_to_string(run["artifacts"]["csv_path"].as_str().unwrap()).unwrap();
            let mut lines = text.lines();
            assert_eq!(lines.next(), Some("frequency,v(a).real,v(a).imag"));
            assert_eq!(lines.count(), 10);
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"], "measurements": measurements }),
            )
            .await;
            assert!(!failed, "{body}");
            assert_eq!(body["analysis_kind"], "ac");
            assert_eq!(body["measurements"], run["measurements"]);
            assert_eq!(body["outcome"]["requested"], 2);
        }

        #[tokio::test]
        async fn a_stored_dc_sweep_measures_like_the_run_and_its_csv_is_named_for_the_target() {
            let points: Vec<(f64, f64)> = (0..=5)
                .map(|n| (f64::from(n), 2.0 * f64::from(n)))
                .collect();
            let job = format!("{}\n{}", copy_dc(&points), status_json("0", "null", true));
            let scene = scene(&job);
            let measurements = json!([
                { "name": "cross", "kind": "crossing", "signal": "v(a)", "level": 5.0 },
                { "name": "mid", "kind": "value_at", "signal": "v(a)", "at_sweep": 2.5 },
                { "name": "late", "kind": "min", "signal": "v(a)", "from_sweep": 3.0 },
            ]);
            let mut arguments = sweep_args(
                &scene.root,
                json!({ "kind": "dc_sweep", "sweeps": [
                    { "target": "vsrc", "start_value": 0.0, "stop_value": 5.0, "step_value": 1.0 }
                ] }),
                "VSRC /rect_out 0 DC 0",
            );
            arguments["measurements"] = measurements.clone();
            let (_, run) = call(&scene, arguments).await;
            let text =
                std::fs::read_to_string(run["artifacts"]["csv_path"].as_str().unwrap()).unwrap();
            assert_eq!(text.lines().next(), Some("VSRC (V),v(a)"));
            assert_eq!(text.lines().nth(3), Some("2e0,4e0"));
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"], "measurements": measurements }),
            )
            .await;
            assert!(!failed, "{body}");
            assert_eq!(body["analysis_kind"], "dc_sweep");
            assert_eq!(body["measurements"], run["measurements"]);
        }

        #[tokio::test]
        async fn a_falling_stored_sweep_is_measured_in_the_order_it_ran() {
            let points: Vec<(f64, f64)> = (0..=5)
                .map(|n| (5.0 - f64::from(n), 2.0 * (5.0 - f64::from(n))))
                .collect();
            let job = format!("{}\n{}", copy_dc(&points), status_json("0", "null", true));
            let scene = scene(&job);
            let measurements = json!([
                { "kind": "crossing", "signal": "v(a)", "level": 5.0 },
                { "kind": "value_at", "signal": "v(a)", "at_sweep": 2.5 },
            ]);
            let mut arguments = sweep_args(
                &scene.root,
                json!({ "kind": "dc_sweep", "sweeps": [
                    { "target": "vsrc", "start_value": 5.0, "stop_value": 0.0, "step_value": -1.0 }
                ] }),
                "VSRC /rect_out 0 DC 0",
            );
            arguments["measurements"] = measurements.clone();
            let (_, run) = call(&scene, arguments).await;
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"], "measurements": measurements }),
            )
            .await;
            assert!(!failed, "{body}");
            assert_eq!(body["measurements"], run["measurements"]);
            assert_eq!(body["measurements"][1]["value"], 5.0, "{body}");
        }

        #[tokio::test]
        async fn two_sweeps_are_exported_with_their_second_value_and_are_not_measured() {
            let grid: Vec<(f64, f64)> = (0..9).map(|n| ((n % 3) as f64, n as f64)).collect();
            let job = format!("{}\n{}", copy_dc(&grid), status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = sweep_args(
                &scene.root,
                json!({ "kind": "dc_sweep", "sweeps": [
                    { "target": "VSRC", "start_value": 0.0, "stop_value": 2.0, "step_value": 1.0 },
                    { "target": "R2", "start_value": 1000.0, "stop_value": 3000.0, "step_value": 1000.0 },
                ] }),
                "VSRC /rect_out 0 DC 0",
            );
            arguments["signals"] = json!(["v(a)"]);
            let (_, run) = call(&scene, arguments).await;
            assert_eq!(run["outcome"]["status"], "complete", "{run}");
            let text =
                std::fs::read_to_string(run["artifacts"]["csv_path"].as_str().unwrap()).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(lines[0], "R2 (Ω),VSRC (V),v(a)");
            assert_eq!(lines[1], "1e3,0e0,0e0");
            assert_eq!(lines[3], "1e3,2e0,2e0");
            assert_eq!(lines[4], "2e3,0e0,3e0");
            assert_eq!(lines[9], "3e3,2e0,8e0");
            assert_eq!(lines.len(), 10);

            let (failed, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"],
                        "measurements": [{ "kind": "max", "signal": "v(a)" }] }),
            )
            .await;
            assert!(failed, "{body}");
            assert_eq!(body["error"]["kind"], "invalid_argument", "{body}");
            assert_eq!(body["error"]["field"], "measurements", "{body}");
            let (_, list) = listed(&scene, json!({})).await;
            assert_eq!(list["runs"][0]["analysis_kind"], "dc_sweep");
        }

        #[tokio::test]
        async fn a_csv_that_cannot_be_written_costs_the_run_nothing() {
            let job = format!("mkdir results.csv\n{}", transient_job());
            let scene = scene(&job);
            let (failed, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            assert!(!failed, "{run}");
            assert_eq!(run["outcome"]["status"], "complete", "{run}");
            assert!(run["artifacts"]["csv_path"].is_null(), "{run}");
            assert!(
                run["warnings"].to_string().contains("csv_not_written"),
                "{run}"
            );
            let (_, list) = listed(&scene, json!({})).await;
            assert_eq!(list["total"], 1);
        }

        #[tokio::test]
        async fn a_record_that_cannot_be_written_is_a_warning_and_leaves_the_run_unlisted() {
            let job = format!("mkdir run.json\n{}", transient_job());
            let scene = scene(&job);
            let (failed, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            assert!(!failed, "{run}");
            assert_eq!(run["outcome"]["status"], "complete", "{run}");
            assert!(
                run["warnings"].to_string().contains("run_record_not_saved"),
                "{run}"
            );
            let (_, list) = listed(&scene, json!({})).await;
            assert_eq!(list["total"], 0);
            // A folder stands where the record belongs: there, and not usable.
            assert_eq!(list["unreadable_record"], 1, "{list}");
        }

        #[tokio::test]
        async fn a_stored_run_is_read_with_the_names_it_was_made_with() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            let dir = first_run_dir(&scene);
            // Its data names the nets and parts as the engine does.
            std::fs::write(
                dir.join(RAW_FILE),
                "Title: t\nPlotname: Transient Analysis\nFlags: real\nNo. Variables: 3\n\
                 No. Points: 2\nVariables:\n\t0\ttime\ttime\n\t1\tv(/out_x)\tvoltage\n\
                 \t2\ti(vbt1)\tcurrent\nValues:\n0\t0.0\n\t1.0\n\t3.0\n1\t2.0\n\t4.0\n\t6.0\n\n",
            )
            .unwrap();
            let path = dir.join(runs::RECORD_FILE);
            let mut record: RunRecord =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            record.nets = vec![crate::simulation::kicad_netlist::NetEntry {
                kicad_name: "/Out".to_string(),
                spice_name: "/out_x".to_string(),
                in_deck: true,
                pin_count: 2,
            }];
            record.elements = vec![crate::simulation::kicad_netlist::ElementEntry {
                reference: "BT1".to_string(),
                element_name: Some("VBT1".to_string()),
                modelled: true,
                excluded: false,
            }];
            std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"], "measurements": [
                    { "kind": "max", "signal": "v(/Out)" },
                    { "kind": "max", "signal": "i(BT1)" },
                ] }),
            )
            .await;
            assert!(!failed, "{body}");
            assert_eq!(body["measurements"][0]["value"], 4.0, "{body}");
            assert_eq!(body["measurements"][1]["value"], 6.0, "{body}");
        }

        #[tokio::test]
        async fn power_a_run_saved_is_measured_without_a_warning() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            let dir = first_run_dir(&scene);
            std::fs::write(
                dir.join(RAW_FILE),
                "Title: t\nPlotname: Transient Analysis\nFlags: real\nNo. Variables: 2\n\
                 No. Points: 2\nVariables:\n\t0\ttime\ttime\n\t1\tr2:power\tpower\n\
                 Values:\n0\t0.0\n\t1.0\n1\t2.0\n\t9.0\n\n",
            )
            .unwrap();
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["p(R2)"],
                        "measurements": [{ "kind": "max", "signal": "p(R2)" }] }),
            )
            .await;
            assert!(!failed, "{body}");
            assert_eq!(body["measurements"][0]["value"], 9.0, "{body}");
            assert!(
                !body["warnings"].to_string().contains("power_not_saved"),
                "{body}"
            );
            assert_eq!(body["outcome"]["status"], "complete", "{body}");
        }

        #[tokio::test]
        async fn a_summary_signal_the_run_lacks_makes_the_outcome_partial_alone() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(nowhere)"],
                        "measurements": [{ "kind": "max", "signal": "v(a)" }] }),
            )
            .await;
            assert!(!failed, "{body}");
            assert_eq!(body["signal_summaries"][0]["finding"], "signal_not_found");
            assert_eq!(body["outcome"]["status"], "partial", "{body}");
            assert_eq!(body["outcome"]["requested"], 2);
            assert_eq!(body["outcome"]["completed"], 1);
            assert_eq!(body["outcome"]["failed"], 1);
            assert!(
                !body["warnings"].to_string().contains("power_not_saved"),
                "{body}"
            );
        }

        #[tokio::test]
        async fn a_run_that_covered_half_its_range_lists_that_share() {
            let scene = scene(&transient_job());
            // Data to 2 s of a 4 s stop, from an engine that reported success.
            let (_, run) = call(&scene, transient_args(&scene.root, 4.0)).await;
            assert_eq!(run["outcome"]["status"], "partial", "{run}");
            let (_, list) = listed(&scene, json!({})).await;
            assert_eq!(list["runs"][0]["run_status"], "partial", "{list}");
            assert_eq!(list["runs"][0]["coverage_ratio"], 0.5, "{list}");
            let (_, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"],
                        "measurements": [{ "kind": "max", "signal": "v(a)" }] }),
            )
            .await;
            assert_eq!(body["coverage_ratio"], 0.5, "{body}");
            assert_eq!(body["stopped"]["reason"], "incomplete_coverage", "{body}");
        }

        #[tokio::test]
        async fn an_operating_point_run_is_listed_and_has_nothing_to_measure() {
            let job = format!("{}\n{}", copy_raw(), status_json("0", "null", true));
            let scene = scene(&job);
            let (_, run) = call(&scene, args(&scene.root)).await;
            let (_, list) = listed(&scene, json!({})).await;
            assert_eq!(
                list["runs"][0]["analysis_kind"], "operating_point",
                "{list}"
            );
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"],
                        "measurements": [{ "kind": "max", "signal": "v(a)" }] }),
            )
            .await;
            assert!(failed);
            assert_eq!(body["error"]["kind"], "invalid_argument", "{body}");
        }

        #[tokio::test]
        async fn a_model_file_the_run_inlined_is_an_input_that_can_change() {
            let scene = scene(&transient_job());
            let model = scene.project.path().join("part.lib");
            std::fs::write(&model, ".model DX D(IS=1e-14)\n").unwrap();
            let mut arguments = transient_args(&scene.root, 2.0);
            arguments["testbench"]["spice_lines"] = json!([".include part.lib"]);
            let (failed, run) = call(&scene, arguments).await;
            assert!(!failed, "{run}");
            let record = runs::read_record(&first_run_dir(&scene)).unwrap();
            assert_eq!(record.inlined_files.len(), 1, "{record:?}");
            let ask = json!({ "run_id": run["run_id"],
                              "measurements": [{ "kind": "max", "signal": "v(a)" }] });
            let (_, body) = measured(&scene, ask.clone()).await;
            assert_eq!(body["inputs_changed_since_run"], false, "{body}");
            std::fs::write(&model, ".model DX D(IS=2e-14)\n").unwrap();
            let (_, list) = listed(&scene, json!({})).await;
            assert_eq!(list["runs"][0]["inputs_changed_since_run"], true, "{list}");
            let (_, body) = measured(&scene, ask.clone()).await;
            assert_eq!(body["inputs_changed_since_run"], true, "{body}");
            std::fs::remove_file(&model).unwrap();
            let (_, body) = measured(&scene, ask).await;
            assert_eq!(body["inputs_changed_since_run"], true, "{body}");
        }

        #[tokio::test]
        async fn what_the_run_warned_of_comes_with_a_later_measurement() {
            // No point at time zero under uic: a peak at the start reads low.
            let raw = "cat > results.raw <<'RAW'\nTitle: t\nPlotname: Transient Analysis\n\
                Flags: real\nNo. Variables: 2\nNo. Points: 3\nVariables:\n\t0\ttime\ttime\n\
                \t1\tv(a)\tvoltage\nValues:\n0\t5e-1\n\t1.0\n1\t1.0\n\t2.0\n2\t2.0\n\t3.0\n\nRAW";
            let job = format!("{raw}\n{}", status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = transient_args(&scene.root, 2.0);
            arguments["analysis"]["use_initial_conditions"] = json!(true);
            let (failed, run) = call(&scene, arguments).await;
            assert!(!failed, "{run}");
            assert!(
                run["warnings"].to_string().contains("no_point_at_start"),
                "{run}"
            );
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"],
                        "measurements": [{ "kind": "max", "signal": "v(a)" }] }),
            )
            .await;
            assert!(!failed, "{body}");
            let warnings = body["warnings"].to_string();
            assert!(warnings.contains("no_point_at_start"), "{warnings}");
            // The warning points at a number, and the body has it.
            assert_eq!(body["analysis"]["first_point_seconds"], 0.5, "{body}");
            // What belonged to the run's own call is not carried.
            assert!(!warnings.contains("csv_not_written"), "{warnings}");
            assert!(!warnings.contains("old_runs_kept"), "{warnings}");
        }

        #[tokio::test]
        async fn the_sheets_are_named_as_the_disk_spells_them_whatever_the_caller_typed() {
            let scene = scene(&transient_job());
            let shouted = scene.root.with_file_name("TOP.KICAD_SCH");
            // A disk that tells the two apart has no such file, and nothing to test.
            if !shouted.exists() {
                return;
            }
            let mut arguments = transient_args(&scene.root, 2.0);
            arguments["schematic"] = json!(shouted);
            let (failed, run) = call(&scene, arguments).await;
            assert!(!failed, "{run}");
            let ask = json!({ "run_id": run["run_id"],
                              "measurements": [{ "kind": "max", "signal": "v(a)" }] });
            let (_, body) = measured(&scene, ask).await;
            assert_eq!(body["inputs_changed_since_run"], false, "{body}");
            for schematic in [&scene.root, &shouted] {
                let result =
                    handle_list_simulation_runs(&json!({ "schematic": schematic }), &scene.ctx)
                        .await
                        .unwrap();
                let (_, list) = read_body(result);
                assert_eq!(list["total"], 1, "{list}");
                assert_eq!(list["runs"][0]["inputs_changed_since_run"], false, "{list}");
            }
        }

        #[cfg(unix)]
        #[tokio::test]
        async fn a_status_file_or_raw_file_that_is_a_link_is_never_read() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            let ask = json!({ "run_id": run["run_id"],
                              "measurements": [{ "kind": "max", "signal": "v(a)" }] });
            let dir = first_run_dir(&scene);
            let outside = tempfile::tempdir().unwrap();
            for name in [crate::simulation::protocol::STATUS_FILE, RAW_FILE] {
                let file = dir.join(name);
                let kept = outside.path().join(name);
                std::fs::rename(&file, &kept).unwrap();
                std::os::unix::fs::symlink(&kept, &file).unwrap();
                let (failed, body) = measured(&scene, ask.clone()).await;
                assert!(failed, "{name}: {body}");
                assert_eq!(
                    body["error"]["reason"], "results_unreadable",
                    "{name}: {body}"
                );
                std::fs::remove_file(&file).unwrap();
                std::fs::rename(&kept, &file).unwrap();
            }
            let (failed, body) = measured(&scene, ask).await;
            assert!(!failed, "{body}");
        }

        #[tokio::test]
        async fn a_status_file_past_its_limit_is_not_read() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            let dir = first_run_dir(&scene);
            let status = dir.join(crate::simulation::protocol::STATUS_FILE);
            let mut text = std::fs::read_to_string(&status).unwrap();
            text.push_str(&" ".repeat(runs::MAX_STATUS_BYTES as usize));
            std::fs::write(&status, text).unwrap();
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"],
                        "measurements": [{ "kind": "max", "signal": "v(a)" }] }),
            )
            .await;
            assert!(failed, "{body}");
            assert_eq!(body["error"]["reason"], "results_unreadable", "{body}");
        }

        #[tokio::test]
        async fn a_record_that_cannot_be_used_is_counted_apart_from_a_missing_one() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            let record = first_run_dir(&scene).join(runs::RECORD_FILE);
            std::fs::write(&record, "{ not json").unwrap();
            let (_, list) = listed(&scene, json!({})).await;
            assert_eq!(list["total"], 0, "{list}");
            assert_eq!(list["unreadable_record"], 1, "{list}");
            assert_eq!(list["without_record"], 0, "{list}");
            std::fs::remove_file(&record).unwrap();
            let (_, list) = listed(&scene, json!({})).await;
            assert_eq!(list["unreadable_record"], 0, "{list}");
            assert_eq!(list["without_record"], 1, "{list}");
            let _ = run;
        }

        #[tokio::test]
        async fn a_stored_two_sweep_run_says_why_it_takes_no_measurement_and_where_the_grid_is() {
            let grid: Vec<(f64, f64)> = (0..9).map(|n| ((n % 3) as f64, n as f64)).collect();
            let job = format!("{}\n{}", copy_dc(&grid), status_json("0", "null", true));
            let scene = scene(&job);
            let arguments = sweep_args(
                &scene.root,
                json!({ "kind": "dc_sweep", "sweeps": [
                    { "target": "VSRC", "start_value": 0.0, "stop_value": 2.0, "step_value": 1.0 },
                    { "target": "R2", "start_value": 1000.0, "stop_value": 3000.0, "step_value": 1000.0 },
                ] }),
                "VSRC /rect_out 0 DC 0",
            );
            let (_, run) = call(&scene, arguments).await;
            let (failed, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"],
                        "measurements": [{ "kind": "max", "signal": "v(a)" }] }),
            )
            .await;
            assert!(failed, "{body}");
            let message = body["error"]["reason"]
                .as_str()
                .unwrap_or_default()
                .to_string()
                + body["error"]["message"].as_str().unwrap_or_default();
            assert!(message.contains("results.csv"), "{body}");
            assert!(!message.contains("takes signals but"), "{body}");
        }

        #[tokio::test]
        async fn a_power_signal_asked_twice_is_named_once() {
            let scene = scene(&transient_job());
            let (_, run) = call(&scene, transient_args(&scene.root, 2.0)).await;
            let (_, body) = measured(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["p(R2)"],
                        "measurements": [{ "kind": "max", "signal": "p(R2)" }] }),
            )
            .await;
            let warning = body["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .find(|warning| warning["code"] == "power_not_saved")
                .unwrap_or_else(|| panic!("{body}"));
            assert_eq!(warning["items"], json!(["p(R2)"]), "{warning}");
        }

        // --- plots -----------------------------------------------------------

        async fn plotted(scene: &Scene, arguments: Value) -> (bool, Value) {
            let result = handle_plot_simulation_run(&arguments, &scene.ctx)
                .await
                .unwrap();
            read_body(result)
        }

        fn svg_of(body: &Value) -> String {
            std::fs::read_to_string(body["path"].as_str().unwrap()).unwrap()
        }

        fn svg_texts(svg: &str) -> Vec<String> {
            let document = usvg::roxmltree::Document::parse(svg).expect("well-formed");
            document
                .descendants()
                .filter(|node| node.has_tag_name("text"))
                .map(|node| node.text().unwrap_or_default().to_string())
                .collect()
        }

        /// The points of each polyline, in pixels.
        fn svg_lines(svg: &str) -> Vec<Vec<(f64, f64)>> {
            let document = usvg::roxmltree::Document::parse(svg).expect("well-formed");
            document
                .descendants()
                .filter(|node| node.has_tag_name("polyline"))
                .map(|node| {
                    node.attribute("points")
                        .unwrap()
                        .split(' ')
                        .map(|pair| {
                            let (x, y) = pair.split_once(',').unwrap();
                            (x.parse().unwrap(), y.parse().unwrap())
                        })
                        .collect()
                })
                .collect()
        }

        fn plots_in(dir: &Path) -> Vec<PathBuf> {
            std::fs::read_dir(dir)
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("plot-"))
                })
                .collect()
        }

        async fn run_of(scene: &Scene, arguments: Value) -> Value {
            let (failed, run) = call(scene, arguments).await;
            assert!(!failed, "{run}");
            run
        }

        fn refused(failed: bool, body: &Value, kind: &str, field: &str) {
            assert!(failed, "{body}");
            assert_eq!(body["error"]["kind"], kind, "{body}");
            if !field.is_empty() {
                assert_eq!(body["error"]["field"], field, "{body}");
            }
            assert_eq!(body["outcome"]["status"], "failed", "{body}");
            assert_eq!(body["outcome"]["requested"], 2, "{body}");
            assert_eq!(body["outcome"]["failed"], 2, "{body}");
        }

        #[tokio::test]
        async fn a_transient_is_drawn_with_a_panel_for_each_unit_and_a_banner_for_the_modified_circuit(
        ) {
            let scene = scene(&transient_job());
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)", "i(m1:d)"] }),
            )
            .await;
            assert!(!failed, "{body}");
            let path = PathBuf::from(body["path"].as_str().unwrap());
            assert!(path.starts_with(first_run_dir(&scene)), "{path:?}");
            assert_eq!(path.extension().unwrap(), "svg");
            let svg = svg_of(&body);
            assert_eq!(body["bytes"], svg.len());
            assert_eq!(body["width_px"], 800);
            let document = usvg::roxmltree::Document::parse(&svg).unwrap();
            let height = document.root_element().attribute("height").unwrap();
            assert_eq!(body["height_px"].to_string(), height);
            assert_eq!(body["signals_plotted"], json!(["v(a)", "i(m1:d)"]));
            assert_eq!(body["analysis_kind"], "transient");
            assert_eq!(body["run_status"], "complete");
            assert_eq!(body["simulated_circuit"], "modified");
            assert_eq!(body["coverage_ratio"], 1.0);
            assert_eq!(body["inputs_changed_since_run"], false);
            assert_eq!(body["outcome"]["status"], "complete", "{body}");
            assert_eq!(body["outcome"]["requested"], 2);
            assert_eq!(body["outcome"]["completed"], 2);
            let shown = svg_texts(&svg);
            for label in ["Voltage (V)", "Current (A)", "Time (s)", "v(a)", "i(m1:d)"] {
                assert!(shown.iter().any(|t| t == label), "{label} in {shown:?}");
            }
            assert!(
                shown.iter().any(|t| t.starts_with("MODIFIED CIRCUIT")),
                "{shown:?}"
            );
            assert!(!shown.iter().any(|t| t.starts_with("PARTIAL RUN")));
            // Volts above amps.
            let place = |label: &str| shown.iter().position(|t| t == label).unwrap();
            assert!(place("Voltage (V)") < place("Current (A)"), "{shown:?}");
            // v(a) climbs 0, 1, 2 and i(m1:d) holds 5: two lines.
            let lines = svg_lines(&svg);
            assert_eq!(lines.len(), 2, "{lines:?}");
            assert!(lines[0][0].1 > lines[0][2].1, "v(a) rises");
            assert_eq!(lines[1][0].1, lines[1][2].1, "the current is flat");
        }

        #[tokio::test]
        async fn a_run_that_stopped_short_is_drawn_as_far_as_it_got_under_a_banner() {
            let scene = scene(&transient_job());
            // Data to 2 s of 5 s asked for.
            let run = run_of(&scene, transient_args(&scene.root, 5.0)).await;
            assert_eq!(run["outcome"]["status"], "partial");
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(!failed, "{body}");
            assert_eq!(body["run_status"], "partial");
            assert_eq!(body["coverage_ratio"], 0.4);
            assert_eq!(body["outcome"]["status"], "partial", "{body}");
            assert_eq!(body["outcome"]["requested"], 2);
            assert_eq!(body["outcome"]["completed"], 1);
            assert_eq!(body["outcome"]["failed"], 1);
            let svg = svg_of(&body);
            let shown = svg_texts(&svg);
            assert!(
                shown
                    .iter()
                    .any(|t| t.starts_with("PARTIAL RUN") && t.contains("40%")),
                "{shown:?}"
            );
            assert!(
                shown.iter().any(|t| t.contains("incomplete_coverage")),
                "{shown:?}"
            );
            // What was not simulated is not drawn: the data fill the axis, and the
            // banner says how little of the range they are.
            let line = &svg_lines(&svg)[0];
            let (first, last) = (line[0].0, line[line.len() - 1].0);
            let share = (last - first) / (800.0 - 76.0 - 18.0);
            assert!(share > 0.99, "{share}");
            assert!(!svg.contains("not simulated"));
        }

        #[tokio::test]
        async fn an_ac_sweep_is_a_bode_plot_on_a_log_axis() {
            let raw = copy_ac(&low_pass(&[
                1.0, 2.0, 4.0, 8.0, 10.0, 12.0, 16.0, 32.0, 64.0, 100.0,
            ]));
            let job = format!("{raw}\n{}", status_json("0", "null", true));
            let scene = scene(&job);
            let run = run_of(
                &scene,
                ac_args(&scene.root, json!({ "points": 1, "scale": "decade" })),
            )
            .await;
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(!failed, "{body}");
            assert_eq!(body["analysis_kind"], "ac");
            let svg = svg_of(&body);
            let shown = svg_texts(&svg);
            for label in ["Gain (dB)", "Phase (°)", "Frequency (Hz)", "1", "10", "100"] {
                assert!(shown.iter().any(|t| t == label), "{label} in {shown:?}");
            }
            let lines = svg_lines(&svg);
            assert_eq!(lines.len(), 2, "gain and phase");
            // Log frequency: 1 to 10 Hz takes as much room as 10 to 100.
            let gain = &lines[0];
            let (x1, x10, x100) = (gain[0].0, gain[4].0, gain[9].0);
            assert!(((x10 - x1) - (x100 - x10)).abs() < 0.5, "{x1} {x10} {x100}");
            // Both fall: the pixel row grows. The gain is 3 dB down at the corner.
            assert!(gain[9].1 > gain[4].1 && gain[4].1 > gain[0].1);
            assert!(lines[1][9].1 > lines[1][0].1, "the phase falls");
        }

        #[tokio::test]
        async fn a_dc_sweep_is_drawn_against_the_element_it_swept_and_a_falling_one_runs_leftward()
        {
            let rising: Vec<(f64, f64)> = (0..=5)
                .map(|n| (f64::from(n), 2.0 * f64::from(n)))
                .collect();
            let sweep = |start: f64, stop: f64, step: f64| {
                json!({ "kind": "dc_sweep", "sweeps": [
                    { "target": "vsrc", "start_value": start, "stop_value": stop,
                      "step_value": step }
                ] })
            };
            let job = format!("{}\n{}", copy_dc(&rising), status_json("0", "null", true));
            let scene = scene(&job);
            let run = run_of(
                &scene,
                sweep_args(&scene.root, sweep(0.0, 5.0, 1.0), "VSRC /rect_out 0 DC 0"),
            )
            .await;
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(!failed, "{body}");
            let svg = svg_of(&body);
            let shown = svg_texts(&svg);
            assert!(shown.iter().any(|t| t == "VSRC (V)"), "{shown:?}");
            let line = &svg_lines(&svg)[0];
            assert!(line[0].0 < line[5].0 && line[0].1 > line[5].1);

            let falling: Vec<(f64, f64)> = rising.iter().rev().copied().collect();
            let job = format!("{}\n{}", copy_dc(&falling), status_json("0", "null", true));
            let scene = self::scene(&job);
            let run = run_of(
                &scene,
                sweep_args(&scene.root, sweep(5.0, 0.0, -1.0), "VSRC /rect_out 0 DC 0"),
            )
            .await;
            let (_, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            let line = &svg_lines(&svg_of(&body))[0];
            // It ran from 5 V down to 0 V: the line starts at the right, high.
            assert!(line[0].0 > line[5].0 && line[0].1 < line[5].1, "{line:?}");
        }

        fn grid_job(inner: usize, outer: usize) -> String {
            let points: Vec<(f64, f64)> = (0..inner * outer)
                .map(|n| ((n % inner) as f64, n as f64))
                .collect();
            format!("{}\n{}", copy_dc(&points), status_json("0", "null", true))
        }

        fn grid_args(root: &Path, inner: usize, outer: usize) -> Value {
            let mut arguments = sweep_args(
                root,
                json!({ "kind": "dc_sweep", "sweeps": [
                    { "target": "VSRC", "start_value": 0.0, "stop_value": (inner - 1) as f64,
                      "step_value": 1.0 },
                    { "target": "R2", "start_value": 1000.0,
                      "stop_value": 1000.0 * outer as f64, "step_value": 1000.0 },
                ] }),
                "VSRC /rect_out 0 DC 0",
            );
            arguments["signals"] = json!(["v(a)"]);
            arguments
        }

        #[tokio::test]
        async fn two_dc_sweeps_draw_a_curve_for_each_step_of_the_second_found_from_the_point_number(
        ) {
            let scene = scene(&grid_job(3, 3));
            let run = run_of(&scene, grid_args(&scene.root, 3, 3)).await;
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(!failed, "{body}");
            let svg = svg_of(&body);
            let shown = svg_texts(&svg);
            for label in ["R2 = 1k Ω", "R2 = 2k Ω", "R2 = 3k Ω", "VSRC (V)", "v(a)"] {
                assert!(shown.iter().any(|t| t == label), "{label} in {shown:?}");
            }
            let lines = svg_lines(&svg);
            assert_eq!(lines.len(), 3);
            // Step k holds values 3k to 3k + 2, so each curve starts above the last.
            assert!(lines[0][0].1 > lines[1][0].1 && lines[1][0].1 > lines[2][0].1);
            // Dark to light along the ramp.
            let colour = |n: usize| {
                let document = usvg::roxmltree::Document::parse(&svg).unwrap();
                document
                    .descendants()
                    .filter(|node| node.has_tag_name("polyline"))
                    .nth(n)
                    .and_then(|node| node.attribute("stroke").map(str::to_string))
                    .unwrap()
            };
            assert_eq!(colour(0), plot::RAMP[0]);
            assert_eq!(colour(2), plot::RAMP[7]);
            assert_eq!(body["warnings"], json!([]));
        }

        #[tokio::test]
        async fn a_second_sweep_of_one_step_is_one_curve_in_the_first_colour() {
            let scene = scene(&grid_job(3, 1));
            let run = run_of(&scene, grid_args(&scene.root, 3, 1)).await;
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(!failed, "{body}");
            let svg = svg_of(&body);
            assert_eq!(svg_lines(&svg).len(), 1);
            assert!(svg.contains(&format!("stroke=\"{}\"", plot::RAMP[0])));
            assert!(!svg.contains(plot::RAMP[3]));
            assert!(svg_texts(&svg).iter().any(|t| t == "R2 = 1k Ω"));
        }

        #[tokio::test]
        async fn more_than_eight_steps_are_thinned_with_the_first_and_last_kept_and_said_so() {
            let scene = scene(&grid_job(2, 12));
            let run = run_of(&scene, grid_args(&scene.root, 2, 12)).await;
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(!failed, "{body}");
            let svg = svg_of(&body);
            assert_eq!(svg_lines(&svg).len(), 8);
            let shown = svg_texts(&svg);
            assert!(shown.iter().any(|t| t == "R2 = 1k Ω"), "{shown:?}");
            assert!(shown.iter().any(|t| t == "R2 = 12k Ω"), "{shown:?}");
            assert!(shown
                .iter()
                .any(|t| t.contains("Showing 8 of the 12 steps")));
            let strokes: std::collections::BTreeSet<String> = {
                let document = usvg::roxmltree::Document::parse(&svg).unwrap();
                document
                    .descendants()
                    .filter(|node| node.has_tag_name("polyline"))
                    .filter_map(|node| node.attribute("stroke").map(str::to_string))
                    .collect()
            };
            assert_eq!(strokes.len(), 8, "every curve has a colour of its own");
            assert_eq!(body["warnings"][0]["code"], "curves_thinned");
            assert_eq!(body["warnings"][0]["steps"], 12);
        }

        #[tokio::test]
        async fn two_sweeps_take_four_signals_at_most() {
            let scene = scene(&grid_job(3, 3));
            let run = run_of(&scene, grid_args(&scene.root, 3, 3)).await;
            let five = json!(["v(a)", "v(a)", "v(a)", "v(a)", "v(a)"]);
            let (failed, body) =
                plotted(&scene, json!({ "run_id": run["run_id"], "signals": five })).await;
            refused(failed, &body, "invalid_argument", "signals");
        }

        #[tokio::test]
        async fn an_operating_point_has_nothing_to_plot_and_says_where_its_table_is() {
            let job = format!("{}\n{}", copy_raw(), status_json("0", "null", true));
            let scene = scene(&job);
            let run = run_of(&scene, args(&scene.root)).await;
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            refused(failed, &body, "invalid_argument", "run_id");
            assert!(body["error"]["reason"]
                .as_str()
                .unwrap()
                .contains("results.csv"));
            assert!(plots_in(&first_run_dir(&scene)).is_empty());
        }

        #[tokio::test]
        async fn a_signal_the_run_lacks_refuses_the_plot_and_writes_nothing() {
            let scene = scene(&transient_job());
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            for (signal, hint) in [
                ("v(nowhere)", ""),
                ("i(M1)", "pins: d"),
                ("p(R2)", "asked for it"),
            ] {
                let (failed, body) = plotted(
                    &scene,
                    json!({ "run_id": run["run_id"], "signals": ["v(a)", signal] }),
                )
                .await;
                refused(failed, &body, "invalid_argument", "signals[1]");
                let reason = body["error"]["reason"].as_str().unwrap();
                assert!(reason.contains("not in this run's data"), "{reason}");
                assert!(reason.contains(hint), "{reason}");
            }
            assert!(plots_in(&first_run_dir(&scene)).is_empty());
        }

        #[tokio::test]
        async fn arguments_out_of_range_are_refused_before_the_store_is_read() {
            let scene = scene(&transient_job());
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            let id = run["run_id"].clone();
            let nine = json!(vec!["v(a)"; 9]);
            let cases = [
                (json!({ "run_id": id, "signals": [] }), "signals"),
                (json!({ "run_id": id, "signals": nine }), "signals"),
                (json!({ "run_id": id }), "signals"),
                (
                    json!({ "run_id": id, "signals": ["v(a)"], "width_px": 199 }),
                    "width_px",
                ),
                (
                    json!({ "run_id": id, "signals": ["v(a)"], "width_px": 1601 }),
                    "width_px",
                ),
                (
                    json!({ "run_id": id, "signals": ["v(a)"], "width_px": 800.5 }),
                    "width_px",
                ),
                (
                    json!({ "run_id": id, "signals": ["v(a)"], "width_px": "800" }),
                    "width_px",
                ),
                (
                    json!({ "run_id": id, "signals": ["v(a)"], "width_px": -5 }),
                    "width_px",
                ),
                (
                    json!({ "run_id": "../../x", "signals": ["v(a)"] }),
                    "run_id",
                ),
                (json!({ "signals": ["v(a)"] }), "run_id"),
                (
                    json!({ "run_id": id, "signals": ["nonsense"] }),
                    "signals[0]",
                ),
                (json!({ "run_id": id, "signals": [7] }), "signals[0]"),
            ];
            for (arguments, field) in cases {
                let (failed, body) = plotted(&scene, arguments.clone()).await;
                refused(failed, &body, "invalid_argument", field);
            }
            assert!(plots_in(&first_run_dir(&scene)).is_empty());
            // A well-formed id nobody made.
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": "000000000000-20260101T000000-0000", "signals": ["v(a)"] }),
            )
            .await;
            assert!(failed);
            assert_eq!(body["error"]["reason"], "run_not_found", "{body}");
            assert_eq!(body["outcome"]["status"], "failed");
            // The widest and narrowest are allowed.
            for width in [200, 1600] {
                let (failed, body) = plotted(
                    &scene,
                    json!({ "run_id": id, "signals": ["v(a)"], "width_px": width }),
                )
                .await;
                assert!(!failed, "{body}");
                assert_eq!(body["width_px"], width);
            }
        }

        #[tokio::test]
        async fn a_run_without_a_record_is_not_found() {
            let scene = scene(&transient_job());
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            std::fs::remove_file(first_run_dir(&scene).join(runs::RECORD_FILE)).unwrap();
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(failed);
            assert_eq!(body["error"]["reason"], "run_not_found", "{body}");
        }

        #[tokio::test]
        async fn a_run_whose_data_is_gone_is_unreadable_and_a_run_in_a_link_is_not_found() {
            let scene = scene(&transient_job());
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            let dir = first_run_dir(&scene);
            std::fs::remove_file(dir.join("results.raw")).unwrap();
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(failed);
            assert_eq!(body["error"]["reason"], "results_unreadable", "{body}");
            assert_eq!(body["outcome"]["status"], "failed");
            // Replace the whole run folder by a link to another: `locate` refuses it.
            let moved = dir.with_extension("moved");
            std::fs::rename(&dir, &moved).unwrap();
            std::os::unix::fs::symlink(&moved, &dir).unwrap();
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(failed);
            assert_eq!(body["error"]["reason"], "run_not_found", "{body}");
            assert!(plots_in(&moved).is_empty());
        }

        #[tokio::test]
        async fn the_same_request_replaces_its_plot_and_the_folder_keeps_sixteen() {
            let scene = scene(&transient_job());
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            let id = run["run_id"].clone();
            let dir = first_run_dir(&scene);
            let scene = &scene;
            let ask = |width: u32| {
                let arguments = json!({ "run_id": id, "signals": ["v(a)"], "width_px": width });
                async move { plotted(scene, arguments).await }
            };
            let (_, first) = ask(300).await;
            let (_, again) = ask(300).await;
            assert_eq!(first["path"], again["path"]);
            assert_eq!(plots_in(&dir).len(), 1);
            for width in 301..=330 {
                let (failed, body) = ask(width).await;
                assert!(!failed, "{body}");
                assert!(PathBuf::from(body["path"].as_str().unwrap()).is_file());
            }
            let left = plots_in(&dir);
            assert_eq!(left.len(), 16, "{left:?}");
            assert!(!PathBuf::from(first["path"].as_str().unwrap()).exists());
            // Nothing but plots, the run's own files and no half-written file.
            let strays = std::fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".partial"))
                .count();
            assert_eq!(strays, 0);
        }

        #[tokio::test]
        async fn a_link_where_the_plot_goes_is_never_written_through() {
            let scene = scene(&transient_job());
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            let dir = first_run_dir(&scene);
            let (_, first) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            let path = PathBuf::from(first["path"].as_str().unwrap());
            let outside = scene.tools.path().join("precious.txt");
            std::fs::write(&outside, "keep").unwrap();
            std::fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(&outside, &path).unwrap();
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(failed, "{body}");
            assert_eq!(body["error"]["reason"], "run_store_failed", "{body}");
            assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep");
            assert!(dir.join(path.file_name().unwrap()).is_symlink());
        }

        #[tokio::test]
        async fn net_names_with_markup_in_them_reach_the_file_as_plain_text() {
            let name = "a<script>alert(1)</script>&amp;\"'";
            let raw = format!(
                "cat > results.raw <<'RAW'\nTitle: t\nPlotname: Transient Analysis\nFlags: real\n\
                 No. Variables: 2\nNo. Points: 3\nVariables:\n\t0\ttime\ttime\n\
                 \t1\tv({name})\tvoltage\nValues:\n0\t0.0\n\t0.0\n1\t1.0\n\t1.0\n2\t2.0\n\t2.0\n\nRAW"
            );
            let scene = scene(&format!("{raw}\n{}", status_json("0", "null", true)));
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            let signal = format!("v({name})");
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": [signal] }),
            )
            .await;
            assert!(!failed, "{body}");
            let svg = svg_of(&body);
            assert!(!svg.contains("<script"), "{svg}");
            // The legend shows the name as it was written, cut to a length.
            let shown = svg_texts(&svg);
            assert!(
                shown
                    .iter()
                    .any(|t| t.contains("<script>alert(1)</script>")),
                "{shown:?}"
            );
        }

        #[test]
        fn the_banner_names_what_a_reader_must_know_and_is_empty_for_a_clean_run() {
            let record = |status: RunState, circuit: &str| {
                RunRecord::new(RecordFields {
                    schematic: PathBuf::from("/p/board.kicad_sch"),
                    analysis: json!({ "kind": "transient", "stop_time_seconds": 1.0 }),
                    run_status: status,
                    stopped: (status == RunState::Partial).then(|| Stopped {
                        reason: "timeout".into(),
                        at_point: 3,
                        node: None,
                    }),
                    simulated_circuit: circuit.to_string(),
                    circuit: Value::Null,
                    result: Value::Null,
                    coverage_ratio: 0.25,
                    inputs_hash: String::new(),
                    inlined_files: Vec::new(),
                    nets: Vec::new(),
                    elements: Vec::new(),
                    warnings: Vec::new(),
                })
            };
            let clean = plots::banner(&record(RunState::Complete, "as_drawn"), Some(false));
            assert!(clean.is_empty(), "{clean:?}");
            assert!(plots::banner(&record(RunState::Complete, "as_drawn"), None).is_empty());
            let partial = plots::banner(&record(RunState::Partial, "as_drawn"), Some(false));
            assert_eq!(partial.len(), 1);
            assert!(partial[0].starts_with("PARTIAL RUN") && partial[0].contains("25%"));
            assert!(partial[0].contains("timeout"));
            let modified = plots::banner(&record(RunState::Complete, "modified"), Some(false));
            assert!(modified[0].starts_with("MODIFIED CIRCUIT"));
            let changed = plots::banner(&record(RunState::Complete, "as_drawn"), Some(true));
            assert!(changed[0].starts_with("SCHEMATIC CHANGED"));
            let all = plots::banner(&record(RunState::Partial, "modified"), Some(true));
            assert_eq!(all.len(), 3);
        }

        #[tokio::test]
        async fn a_long_run_is_drawn_as_an_envelope_and_says_so() {
            let mut raw = "cat > results.raw <<'RAW'\nTitle: t\nPlotname: Transient Analysis\n\
                           Flags: real\nNo. Variables: 2\nNo. Points: 3001\nVariables:\n\
                           \t0\ttime\ttime\n\t1\tv(a)\tvoltage\nValues:\n"
                .to_string();
            for i in 0..=3000 {
                let t = f64::from(i) * 1e-3;
                raw.push_str(&format!("{i}\t{t:e}\n\t{:e}\n", (t * 40.0).sin()));
            }
            raw.push_str("\nRAW");
            let scene = scene(&format!("{raw}\n{}", status_json("0", "null", true)));
            let run = run_of(&scene, transient_args(&scene.root, 3.0)).await;
            assert_eq!(run["outcome"]["status"], "complete", "{run}");
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(!failed, "{body}");
            assert_eq!(body["points_in_run"], 3001);
            let drawn = body["points_drawn"].as_u64().unwrap();
            assert!(drawn <= 2 * 707 && drawn > 100, "{drawn}");
            assert!(body["bytes"].as_u64().unwrap() < 60_000, "{body}");
            let shown = svg_texts(&svg_of(&body));
            assert!(
                shown
                    .iter()
                    .any(|t| t.contains("envelope") && t.contains("3001")),
                "{shown:?}"
            );
            // A short run says nothing of the kind.
            let small = self::scene(&transient_job());
            let run = run_of(&small, transient_args(&small.root, 2.0)).await;
            let (_, body) = plotted(
                &small,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(!svg_texts(&svg_of(&body))
                .iter()
                .any(|t| t.contains("envelope")));
        }

        #[tokio::test]
        async fn a_sample_that_turns_the_phase_too_far_is_noted_in_the_picture_and_the_result() {
            // One decade a point: 1, 10 and 100 Hz, and the phase goes from 0 to 174
            // degrees in one step.
            let raw = copy_ac(&[(1.0, 1.0, 0.0), (10.0, -1.0, 0.1), (100.0, 0.5, -0.5)]);
            let job = format!("{raw}\n{}", status_json("0", "null", true));
            let scene = scene(&job);
            let run = run_of(
                &scene,
                ac_args(&scene.root, json!({ "points": 1, "scale": "decade" })),
            )
            .await;
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(!failed, "{body}");
            assert_eq!(body["warnings"][0]["code"], "phase_unreliable", "{body}");
            assert_eq!(body["warnings"][0]["from_hz"], 10.0);
            let shown = svg_texts(&svg_of(&body));
            assert!(
                shown
                    .iter()
                    .any(|t| t.contains("Phase from 10 Hz on may be out by whole turns")),
                "{shown:?}"
            );
        }

        #[tokio::test]
        async fn only_our_own_plots_are_ever_removed_and_a_stale_half_file_does_not_block() {
            let scene = scene(&transient_job());
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            let id = run["run_id"].clone();
            let dir = first_run_dir(&scene);
            let foreign = [
                "plot-notes.svg",
                "plot-zzzzzzzzzzzz.svg",
                "plot-0123456789abc.svg",
                "plot-0123456789ab.png",
            ];
            for name in foreign {
                std::fs::write(dir.join(name), "mine").unwrap();
            }
            let scene = &scene;
            let ask = |width: u32| {
                let arguments = json!({ "run_id": id, "signals": ["v(a)"], "width_px": width });
                async move { plotted(scene, arguments).await }
            };
            let mut paths = Vec::new();
            for width in 300..320 {
                let (failed, body) = ask(width).await;
                assert!(!failed, "{body}");
                paths.push(PathBuf::from(body["path"].as_str().unwrap()));
            }
            for name in foreign {
                assert_eq!(std::fs::read_to_string(dir.join(name)).unwrap(), "mine");
            }
            // Asking again for one that is still there costs no other plot.
            let kept: Vec<&PathBuf> = paths.iter().filter(|p| p.exists()).collect();
            assert_eq!(kept.len(), 16);
            let (_, again) = ask(319).await;
            assert_eq!(PathBuf::from(again["path"].as_str().unwrap()), paths[19]);
            let (_, again) = ask(paths.len() as u32 + 296).await;
            assert!(!again.is_null());
            for path in &kept {
                assert!(path.exists(), "{path:?} was removed by a repeat");
            }
            // A half file that is just made may be another call writing the same plot,
            // and is not taken over.
            let target = paths[19].clone();
            let mut half = target.clone().into_os_string();
            half.push(".partial");
            std::fs::write(&half, "junk").unwrap();
            let (failed, body) = ask(319).await;
            assert!(failed, "{body}");
            assert_eq!(body["error"]["reason"], "run_store_failed", "{body}");
            assert_eq!(std::fs::read_to_string(&half).unwrap(), "junk");
            // One left by a crash an hour ago does not stop the plot.
            let hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
            std::fs::File::options()
                .write(true)
                .open(&half)
                .unwrap()
                .set_modified(hour_ago)
                .unwrap();
            let (failed, body) = ask(319).await;
            assert!(!failed, "{body}");
            assert!(!PathBuf::from(&half).exists());
            assert!(std::fs::read_to_string(&target)
                .unwrap()
                .ends_with("</svg>\n"));
        }

        #[tokio::test]
        async fn nets_that_ngspice_reads_as_one_are_named_in_a_warning() {
            let scene = scene(&transient_job());
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            let record_path = first_run_dir(&scene).join(runs::RECORD_FILE);
            let mut record: Value =
                serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
            let net = |name: &str| json!({ "kicad_name": name, "spice_name": "a", "in_deck": true, "pin_count": 2 });
            record["nets"] = json!([net("NetOne"), net("NetTwo")]);
            std::fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(NetOne)"] }),
            )
            .await;
            assert!(!failed, "{body}");
            assert_eq!(
                body["warnings"][0]["code"], "nets_share_engine_node",
                "{body}"
            );
            assert_eq!(body["warnings"][0]["items"], json!(["NetOne", "NetTwo"]));
        }

        #[tokio::test]
        async fn a_changed_schematic_shows_its_banner_in_the_picture() {
            let scene = scene(&transient_job());
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            let ask = json!({ "run_id": run["run_id"], "signals": ["v(a)"] });
            let (_, body) = plotted(&scene, ask.clone()).await;
            assert!(!svg_texts(&svg_of(&body))
                .iter()
                .any(|t| t.starts_with("SCHEMATIC CHANGED")));
            let original = std::fs::read_to_string(&scene.root).unwrap();
            std::fs::write(&scene.root, format!("{original} ")).unwrap();
            let (_, body) = plotted(&scene, ask).await;
            assert_eq!(body["inputs_changed_since_run"], true, "{body}");
            let shown = svg_texts(&svg_of(&body));
            assert!(
                shown.iter().any(|t| t.starts_with("SCHEMATIC CHANGED")),
                "{shown:?}"
            );
        }

        #[tokio::test]
        async fn a_temperature_sweep_is_drawn_in_plain_degrees() {
            let points: Vec<(f64, f64)> = (1..=5)
                .map(|n| (f64::from(n) / 10.0, f64::from(n)))
                .collect();
            let job = format!("{}\n{}", copy_dc(&points), status_json("0", "null", true));
            let scene = scene(&job);
            let run = run_of(
                &scene,
                sweep_args(
                    &scene.root,
                    json!({ "kind": "dc_sweep", "sweeps": [
                        { "target": "TEMP", "start_value": 0.1, "stop_value": 0.5,
                          "step_value": 0.1 }
                    ] }),
                    "VSRC /rect_out 0 DC 0",
                ),
            )
            .await;
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(!failed, "{body}");
            let shown = svg_texts(&svg_of(&body));
            assert!(shown.iter().any(|t| t == "temp (°C)"), "{shown:?}");
        }

        #[tokio::test]
        async fn data_with_no_time_or_no_frequency_is_unreadable_and_not_a_blank_plot() {
            let scene = scene(&transient_job());
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            let raw = first_run_dir(&scene).join("results.raw");
            let kept = std::fs::read_to_string(&raw).unwrap();
            // The same data under a scale that is not time.
            std::fs::write(&raw, kept.replace("\ttime\ttime", "\tseconds\ttime")).unwrap();
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(failed, "{body}");
            assert_eq!(body["error"]["reason"], "results_unreadable", "{body}");
            assert!(body["message"].as_str().unwrap().contains("no time"));

            let ac = copy_ac(&low_pass(&[1.0, 10.0, 100.0]));
            let job = format!("{ac}\n{}", status_json("0", "null", true));
            let scene = self::scene(&job);
            let run = run_of(
                &scene,
                ac_args(&scene.root, json!({ "points": 1, "scale": "decade" })),
            )
            .await;
            let raw = first_run_dir(&scene).join("results.raw");
            let kept = std::fs::read_to_string(&raw).unwrap();
            std::fs::write(
                &raw,
                kept.replace("\tfrequency\tfrequency", "\thz\tfrequency"),
            )
            .unwrap();
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(failed, "{body}");
            assert!(body["message"].as_str().unwrap().contains("no frequency"));
        }

        #[test]
        fn the_banner_never_rounds_a_short_run_up_to_the_whole_range() {
            let record = |ratio: f64| {
                RunRecord::new(RecordFields {
                    schematic: PathBuf::from("/p/board.kicad_sch"),
                    analysis: json!({ "kind": "transient", "stop_time_seconds": 1.0 }),
                    run_status: RunState::Partial,
                    stopped: Some(Stopped {
                        reason: "x".repeat(500),
                        at_point: 3,
                        node: None,
                    }),
                    simulated_circuit: "as_drawn".into(),
                    circuit: Value::Null,
                    result: Value::Null,
                    coverage_ratio: ratio,
                    inputs_hash: String::new(),
                    inlined_files: Vec::new(),
                    nets: Vec::new(),
                    elements: Vec::new(),
                    warnings: Vec::new(),
                })
            };
            let says = |ratio: f64| plots::banner(&record(ratio), None).remove(0);
            assert!(says(0.996).contains("covers 99% of"), "{}", says(0.996));
            assert!(says(0.004).contains("covers under 1% of"));
            assert!(says(0.5).contains("covers 50% of"));
            // A reason from a damaged record is cut, not repeated whole.
            assert!(says(0.5).len() < 300, "{}", says(0.5).len());
        }

        #[test]
        fn a_step_of_the_second_sweep_never_reads_past_the_end_of_a_short_variable() {
            assert_eq!(plots::step_rows(0, 3, 10), 0..3);
            assert_eq!(plots::step_rows(1, 3, 5), 3..5);
            assert_eq!(plots::step_rows(2, 3, 5), 5..5);
            assert_eq!(plots::step_rows(9, 3, 5), 5..5);
            assert_eq!(plots::step_rows(0, 3, 0), 0..0);
        }

        #[tokio::test]
        async fn a_second_sweep_of_temperature_names_its_curves_in_plain_degrees() {
            let points: Vec<(f64, f64)> = (0..9).map(|n| ((n % 3) as f64, n as f64)).collect();
            let job = format!("{}\n{}", copy_dc(&points), status_json("0", "null", true));
            let scene = scene(&job);
            let mut arguments = sweep_args(
                &scene.root,
                json!({ "kind": "dc_sweep", "sweeps": [
                    { "target": "VSRC", "start_value": 0.0, "stop_value": 2.0, "step_value": 1.0 },
                    { "target": "TEMP", "start_value": 0.1, "stop_value": 0.3, "step_value": 0.1 },
                ] }),
                "VSRC /rect_out 0 DC 0",
            );
            arguments["signals"] = json!(["v(a)"]);
            let run = run_of(&scene, arguments).await;
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            assert!(!failed, "{body}");
            let shown = svg_texts(&svg_of(&body));
            for label in ["temp = 0.1 °C", "temp = 0.2 °C", "temp = 0.3 °C"] {
                assert!(shown.iter().any(|t| t == label), "{label} in {shown:?}");
            }
        }

        #[test]
        fn the_file_name_tells_apart_signal_lists_that_join_to_the_same_text() {
            let name = |signals: &[&str], width| {
                let signals: Vec<String> = signals.iter().map(|s| s.to_string()).collect();
                plots::file_name(&signals, width)
            };
            assert_ne!(name(&["ab", "c"], 800), name(&["a", "bc"], 800));
            assert_ne!(name(&["v(a)"], 800), name(&["v(a)"], 801));
            assert_eq!(name(&["v(a)"], 800), name(&["v(a)"], 800));
            assert!(
                name(&["v(a)"], 800).starts_with("plot-") && name(&["v(a)"], 800).ends_with(".svg")
            );
            // The lowest of the frequencies a phase turned too far at, across signals.
            assert_eq!(plots::earlier(None, 50.0), Some(50.0));
            assert_eq!(plots::earlier(Some(50.0), 10.0), Some(10.0));
            assert_eq!(plots::earlier(Some(10.0), 50.0), Some(10.0));
        }

        #[tokio::test]
        async fn every_signal_keeps_its_colour_in_the_legend_and_on_the_curve() {
            let raw = copy_ac(&low_pass(&[
                1.0, 2.0, 4.0, 8.0, 10.0, 12.0, 16.0, 32.0, 64.0, 100.0,
            ]));
            let job = format!("{raw}\n{}", status_json("0", "null", true));
            let scene = scene(&job);
            let run = run_of(
                &scene,
                ac_args(&scene.root, json!({ "points": 1, "scale": "decade" })),
            )
            .await;
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)", "v(a,0)"] }),
            )
            .await;
            assert!(!failed, "{body}");
            let svg = svg_of(&body);
            let document = usvg::roxmltree::Document::parse(&svg).unwrap();
            let strokes = |tag: &str| -> Vec<String> {
                document
                    .descendants()
                    .filter(|node| node.has_tag_name(tag))
                    .filter(|node| tag != "line" || node.attribute("stroke-width") == Some("3"))
                    .filter_map(|node| node.attribute("stroke").map(str::to_string))
                    .collect()
            };
            // Two signals in a gain and a phase panel: the curves are in signal order
            // in each, and the legend repeats the same two colours.
            let curves = strokes("polyline");
            assert_eq!(
                curves,
                [
                    plot::CATEGORICAL[0],
                    plot::CATEGORICAL[1],
                    plot::CATEGORICAL[0],
                    plot::CATEGORICAL[1]
                ]
            );
            assert_eq!(
                strokes("line"),
                [plot::CATEGORICAL[0], plot::CATEGORICAL[1]]
            );

            // Two sweeps: the swatches are the colours of the curves they name.
            let scene = self::scene(&grid_job(3, 3));
            let run = run_of(&scene, grid_args(&scene.root, 3, 3)).await;
            let (_, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            let svg = svg_of(&body);
            let document = usvg::roxmltree::Document::parse(&svg).unwrap();
            let strokes = |tag: &str| -> Vec<String> {
                document
                    .descendants()
                    .filter(|node| node.has_tag_name(tag))
                    .filter(|node| tag != "line" || node.attribute("stroke-width") == Some("3"))
                    .filter_map(|node| node.attribute("stroke").map(str::to_string))
                    .collect()
            };
            assert_eq!(strokes("line"), strokes("polyline"));
        }

        #[tokio::test]
        async fn a_link_a_folder_and_a_capital_hex_name_in_the_run_folder_are_never_pruned() {
            let scene = scene(&transient_job());
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            let id = run["run_id"].clone();
            let dir = first_run_dir(&scene);
            let outside = scene.tools.path().join("precious");
            std::fs::write(&outside, "keep").unwrap();
            std::fs::create_dir(dir.join("plot-aaaaaaaaaaaa.svg")).unwrap();
            std::os::unix::fs::symlink(&outside, dir.join("plot-bbbbbbbbbbbb.svg")).unwrap();
            std::fs::write(dir.join("plot-ABCDEF012345.svg"), "mine").unwrap();
            for width in 300..330 {
                let (failed, body) = plotted(
                    &scene,
                    json!({ "run_id": id, "signals": ["v(a)"], "width_px": width }),
                )
                .await;
                assert!(!failed, "{body}");
            }
            assert!(dir.join("plot-aaaaaaaaaaaa.svg").is_dir());
            assert!(dir.join("plot-bbbbbbbbbbbb.svg").is_symlink());
            assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep");
            assert_eq!(
                std::fs::read_to_string(dir.join("plot-ABCDEF012345.svg")).unwrap(),
                "mine"
            );
        }

        #[tokio::test]
        async fn the_name_of_a_signal_that_is_not_there_is_cut_before_it_is_repeated() {
            let scene = scene(&transient_job());
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            let long = format!("v({})", "x".repeat(5000));
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": [long] }),
            )
            .await;
            refused(failed, &body, "invalid_argument", "signals[0]");
            assert!(
                body["error"]["reason"].as_str().unwrap().len() < 200,
                "{body}"
            );
        }

        #[tokio::test]
        async fn a_half_file_dated_in_the_future_is_treated_as_a_crashs() {
            let scene = scene(&transient_job());
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            let ask = json!({ "run_id": run["run_id"], "signals": ["v(a)"] });
            let (_, first) = plotted(&scene, ask.clone()).await;
            let path = PathBuf::from(first["path"].as_str().unwrap());
            let mut half = path.clone().into_os_string();
            half.push(".partial");
            std::fs::write(&half, "junk").unwrap();
            let future = std::time::SystemTime::now() + Duration::from_secs(3600);
            std::fs::File::options()
                .write(true)
                .open(&half)
                .unwrap()
                .set_modified(future)
                .unwrap();
            let (failed, body) = plotted(&scene, ask).await;
            assert!(!failed, "{body}");
            assert!(!PathBuf::from(&half).exists());
        }

        #[tokio::test]
        async fn a_folder_that_cannot_be_listed_after_the_write_does_not_fail_the_plot() {
            use std::os::unix::fs::PermissionsExt;
            let scene = scene(&transient_job());
            let run = run_of(&scene, transient_args(&scene.root, 2.0)).await;
            let dir = first_run_dir(&scene);
            // Write and search, no read: the plot can be made and the folder not scanned.
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o300)).unwrap();
            let (failed, body) = plotted(
                &scene,
                json!({ "run_id": run["run_id"], "signals": ["v(a)"] }),
            )
            .await;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert!(!failed, "{body}");
            assert!(PathBuf::from(body["path"].as_str().unwrap()).is_file());
        }

        #[test]
        fn a_variable_shorter_than_the_plot_is_cut_to_pair_with_its_own_axis() {
            let (x, y) = plots::paired(&[0.0, 1.0, 2.0], &[5.0, 6.0]);
            assert_eq!((x, y), (vec![0.0, 1.0], vec![5.0, 6.0]));
            let (x, y) = plots::paired(&[0.0], &[5.0, 6.0]);
            assert_eq!((x, y), (vec![0.0], vec![5.0]));
        }
    }
}
