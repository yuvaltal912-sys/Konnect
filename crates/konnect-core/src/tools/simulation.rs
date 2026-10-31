//! SPICE simulation tools.

use super::{cli, get_path, sch_export, ToolContext, ToolDef};
use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::CallToolResult;
use crate::outcome::{self, OutcomeStatus};
use crate::simulation::flatten::{self, Compatibility, DeckInputs, FlatDeck, Refusal};
use crate::simulation::kicad_netlist::{self, ElementEntry, KiCadNetlist, NetEntry, SpiceNetlist};
use crate::simulation::preflight::{self, Preflight};
use crate::simulation::protocol::{
    RunRequest, RunStatus, DECK_FILE, LOG_FILE, PROTOCOL_VERSION, RAW_FILE, REQUEST_FILE,
};
use crate::simulation::raw::Plot;
use crate::simulation::worker_client::{
    classify, probe_engine, read_results, run_worker, EngineStatus, SimulationRuntime, Verdict,
    WorkerEnd,
};
use crate::simulation::{runs, snapshot};
use crate::tool;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const EXPORT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_LIMIT: u64 = 25;
const MAX_LIMIT: u64 = 200;
const DETAIL_LIMIT: usize = 800;
const MAX_SPICE_LINES: usize = 200;
const DEFAULT_TIMEOUT_SECONDS: u64 = 40;
const MAX_TIMEOUT_SECONDS: u64 = 600;
const DEFAULT_ROW_LIMIT: u64 = 40;
const SOURCE: &str = "saved_schematic_snapshot";
/// "Engine ran" and "requested range covered", so a run with no measurements can
/// still read as partial.
const FIXED_OUTCOME_UNITS: usize = 2;
const MAX_ERROR_ITEMS: usize = 20;
const MAX_LOG_ITEMS: usize = 12;
const MAX_LOG_ITEM_CHARS: usize = 300;
/// The worker runs with no init file on purpose, so this says nothing.
const SPINIT_NOTICE: &str = "Warning: can't find the initialization file";
const OPERATING_POINT_FILE: &str = "operating_point.json";
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
        "Run one analysis on a schematic in an isolated simulator process and return compact results, with the full data on disk. Works from the saved files on a private copy. Read simulated_circuit: a result from a modified circuit does not describe the schematic as drawn.",
        json!({
            "type": "object",
            "properties": {
                "schematic": { "type": "string", "description": "Root .kicad_sch path" },
                "analysis": {
                    "oneOf": [{
                        "type": "object",
                        "properties": { "kind": { "const": "operating_point" } },
                        "required": ["kind"]
                    }]
                },
                "testbench": testbench_schema(),
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
                }
            },
            "required": ["schematic", "analysis"]
        }),
        |args, ctx| async move { handle_run_simulation(args, ctx).await }
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
    let testbench = Testbench::from_args(&args["testbench"]);
    let request = RunArguments {
        schematic: &schematic,
        kind: args["analysis"]["kind"].as_str().unwrap_or_default(),
        testbench: &testbench,
        limit,
        budget: Duration::from_secs(timeout),
    };
    Ok(match run_analysis(&request, ctx).await {
        Ok(result) => result,
        Err(refusal) => failed_outcome(refusal, &target),
    })
}

struct RunArguments<'a> {
    schematic: &'a Path,
    kind: &'a str,
    testbench: &'a Testbench,
    limit: usize,
    budget: Duration,
}

async fn usable_engine(
    runtime: &SimulationRuntime,
    kicad_cli: &str,
) -> Result<EngineStatus, CallToolResult> {
    let engine = probe_engine(Some(runtime), kicad_cli).await;
    if engine.available {
        return Ok(engine);
    }
    let not_found = engine.probe_status == "not_found";
    let reason = if not_found {
        "engine_not_found"
    } else {
        "engine_load_failed"
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
    let floating = preflight
        .warnings
        .iter()
        .find(|warning| warning.code == "floating_subcircuit" && kind == "operating_point")?;
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

fn prepare_run(
    run: &runs::RunFolder,
    deck: &FlatDeck,
    event_nodes: bool,
    request: &RunRequest,
) -> anyhow::Result<()> {
    let own_lines: &[&str] = if event_nodes {
        &[SAVE_ALL_VOLTAGES, OPERATING_POINT]
    } else {
        &[SAVE_ALL_VOLTAGES, SAVE_ALL_CURRENTS, OPERATING_POINT]
    };
    std::fs::write(run.dir.join(DECK_FILE), deck.render_with(own_lines))?;
    std::fs::write(run.dir.join(REQUEST_FILE), serde_json::to_vec(request)?)?;
    Ok(())
}

async fn run_analysis(
    request: &RunArguments<'_>,
    ctx: &ToolContext,
) -> Result<CallToolResult, CallToolResult> {
    let started = Instant::now();
    if request.kind != "operating_point" {
        let reason = format!("Unknown analysis kind: {}", request.kind);
        return Err(CallToolResult::error_kind(
            ToolErrorKind::InvalidArgument {
                field: "analysis.kind".to_string(),
                reason: reason.clone(),
            },
            reason,
        ));
    }
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
    let engine = usable_engine(runtime, &ctx.config.kicad_cli).await?;
    let inputs = DesignInputs {
        model_dirs: runtime.model_dirs(),
        testbench: request.testbench,
        allow_digital_models: runtime.allows_digital_models(),
        code_model_count: engine.code_model_count,
        export_timeout: request.budget.min(EXPORT_TIMEOUT),
    };
    let design = inspect_design(&ctx.config.kicad_cli, request.schematic, &inputs)
        .await
        .map_err(|error| export_failure(&error))?;
    let deck = design.deck.as_ref().map_err(deck_refusal)?;
    if let Some(refusal) = design
        .preflight
        .as_ref()
        .and_then(|preflight| preflight_refusal(preflight, request.kind))
    {
        return Err(refusal);
    }
    let Some(budget) = request
        .budget
        .checked_sub(started.elapsed())
        .filter(|budget| !budget.is_zero())
    else {
        return Err(simulation_failure(
            "export",
            "netlist_export_timed_out",
            None,
            Vec::new(),
            "The time budget ran out before the simulator could start.",
        ));
    };

    let internal =
        |error: String| simulation_failure("run", "engine_load_failed", None, Vec::new(), error);
    let library = PathBuf::from(engine.library_path.clone().unwrap_or_default());
    let run = runtime
        .state_root()
        .map_err(|error| error.to_string())
        .and_then(|root| runs::create(&root, request.schematic).map_err(|error| error.to_string()))
        .map_err(|error| internal(format!("Failed to create a run folder: {error}")))?;
    let worker_request = RunRequest {
        protocol: PROTOCOL_VERSION,
        library: library.clone(),
        code_models: engine.code_models_dir.as_ref().map(PathBuf::from),
        load_digital: runtime.allows_digital_models(),
        compatibility: design.compatibility.ngbehavior.map(str::to_owned),
        input_path: Some(design.project_dir.clone()),
        budget_ms: budget.as_millis() as u64,
    };
    let event_nodes = design
        .preflight
        .as_ref()
        .is_some_and(|preflight| preflight.event_nodes);
    prepare_run(&run, deck, event_nodes, &worker_request)
        .map_err(|error| internal(format!("Failed to prepare the run: {error:#}")))?;
    let end = run_worker(runtime, &run.dir, &library, budget)
        .await
        .map_err(|error| internal(format!("{error:#}")))?;

    let finish = RunEnd {
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

/// What the engine said went wrong. Its log can quote any deck line, and text
/// from a model file is not ours to repeat, so those lines are named only.
fn engine_complaints(log: &str, deck: &FlatDeck) -> Vec<String> {
    let included: Vec<String> = deck
        .lines()
        .filter(|(_, _, included)| *included)
        .map(|(text, _, _)| text.trim().to_lowercase())
        .filter(|text| !text.is_empty())
        .collect();
    let ours = log.lines().take_while(|line| !line.contains("sim_status"));
    ours.filter_map(|line| line.strip_prefix("stderr "))
        .filter(|line| !line.starts_with(SPINIT_NOTICE))
        .map(|line| {
            let lower = line.to_lowercase();
            if included.iter().any(|text| lower.contains(text)) {
                "A line from an included model file. See the log.".to_string()
            } else {
                line.chars().take(MAX_LOG_ITEM_CHARS).collect()
            }
        })
        .take(MAX_LOG_ITEMS)
        .collect()
}

fn failure_message(reason: &str) -> &'static str {
    match reason {
        "engine_crashed" => "The simulator stopped on a fatal error. The server is unaffected.",
        "engine_load_failed" => "The simulator could not start this run.",
        "load_failed" => "The simulator stopped on a fatal error while loading the circuit.",
        "circuit_not_parsed" => "The simulator could not parse the circuit.",
        "no_convergence" => "The analysis did not converge and left no data.",
        "timed_out" => "The time budget ran out before the simulator left any data.",
        "results_unreadable" => "The simulator finished but its results could not be read.",
        _ => "The simulation failed.",
    }
}

impl RunEnd<'_> {
    fn path(&self, file: &str) -> String {
        self.run.dir.join(file).display().to_string()
    }

    fn status(&self) -> Option<&RunStatus> {
        match self.end {
            WorkerEnd::Reported(status) => Some(status),
            _ => None,
        }
    }

    fn failure(&self, reason: &'static str) -> CallToolResult {
        let stage = match reason {
            "engine_load_failed" => "engine",
            "results_unreadable" => "results",
            _ => "run",
        };
        let log = std::fs::read_to_string(self.run.dir.join(LOG_FILE)).unwrap_or_default();
        let mut items = engine_complaints(&log, self.deck);
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
        let verdict = classify(self.end, results.as_ref(), operating_point_covered);
        let stopped = match verdict {
            Verdict::Failed(reason) => return Err(self.failure(reason)),
            Verdict::Partial(reason) => Some(reason),
            Verdict::Complete => None,
        };
        let Some(Ok(plot)) = &results else {
            return Err(self.failure("results_unreadable"));
        };
        let rows = operating_point_rows(plot, &self.design.nets);
        std::fs::write(
            self.run.dir.join(OPERATING_POINT_FILE),
            serde_json::to_vec_pretty(&rows).unwrap_or_default(),
        )
        .ok();
        let body = self.result_body(plot, &rows, stopped);
        let (status, completed) = match stopped {
            Some(_) => (OutcomeStatus::Partial, 1),
            None => (OutcomeStatus::Complete, FIXED_OUTCOME_UNITS),
        };
        let outcome = outcome::summary(
            status,
            self.request.schematic.display().to_string(),
            SOURCE,
            FIXED_OUTCOME_UNITS,
            completed,
            FIXED_OUTCOME_UNITS - completed,
            None,
        );
        Ok(outcome::attach(CallToolResult::json(&body), outcome))
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
                               only voltages and source currents are saved.",
                }));
            }
        }
        if !self.engine.code_models_failed.is_empty() {
            warnings.push(json!({
                "code": "code_models_failed",
                "items": self.engine.code_models_failed,
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

    fn result_body(&self, plot: &Plot, rows: &[OperatingPointRow], stopped: Option<&str>) -> Value {
        let (simulated_circuit, circuit) = self.circuit();
        let status = self.status();
        let raw_bytes = std::fs::metadata(self.run.dir.join(RAW_FILE)).map_or(0, |file| file.len());
        let coverage = if operating_point_covered(plot) {
            1.0
        } else {
            0.0
        };
        let mut body = json!({
            "run_id": self.run.id,
            "simulated_circuit": simulated_circuit,
            "circuit": circuit,
            "analysis": {
                "kind": self.request.kind,
                "coverage_ratio": coverage,
                "points": plot.points,
            },
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
            "operating_point": {
                "total": rows.len(),
                "shown": rows.len().min(self.request.limit),
                "truncated": self.request.limit < rows.len(),
                "rows": rows.iter().take(self.request.limit).collect::<Vec<_>>(),
            },
            "warnings": self.warnings(),
            "artifacts": {
                "dir": self.run.dir.display().to_string(),
                "deck_path": self.path(DECK_FILE),
                "raw_path": self.path(RAW_FILE),
                "log_path": self.path(LOG_FILE),
                "table_path": self.path(OPERATING_POINT_FILE),
                "raw_bytes": raw_bytes,
            },
        });
        if let Some(reason) = stopped {
            body["stopped"] = json!({ "reason": reason });
        }
        body
    }
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
    let snapshot = snapshot::take_snapshot(schematic, &source)?;
    let project_dir = schematic
        .canonicalize()?
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
        std::fs::write(&model, ".model secret_vendor_part D(IS=1)\n").unwrap();
        let deck = flat(
            ".title t\n.include \"INCLUDE\"\nR1 a 0 1k\n",
            &model,
            std::slice::from_ref(&root),
        );
        let log = "stdout noise\n\
                   stderr Warning: can't find the initialization file spinit.\n\
                   stderr .MODEL secret_vendor_part D(IS=1)\n\
                   stderr r1 a 0 1k\n\
                   stderr could not find a valid modelname\n\
                   stderr Error: sim_status: no such variable.\n\
                   stderr after the marker\n";
        let complaints = engine_complaints(log, &deck);
        assert_eq!(
            complaints,
            [
                "A line from an included model file. See the log.",
                "r1 a 0 1k",
                "could not find a valid modelname",
            ]
        );
    }

    #[test]
    fn complaints_are_bounded() {
        let log = format!("stderr {}\n", "x".repeat(5_000)).repeat(100);
        let complaints = engine_complaints(&log, &FlatDeck::default());
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

    mod with_fake_cli {
        use super::*;
        use crate::tools::cli::test_support::write_script;

        const OP_FIXTURE: &str = "tests/fixtures/simulation/raw/op.ascii.raw";
        /// Good status fields, for a fake worker to write.
        const GOOD: &str = r#""protocol":2,"load_error":null,"engine_version":"44.2","code_models_loaded":["analog"],"code_models_failed":[],"exit_status":null,"run_started":true"#;

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
            format!(
                "cat > status.json <<'JSON'\n{{{GOOD},\"sim_status\":{sim_status},\
                 \"halted_reason\":{halted},\"results_written\":{written}}}\nJSON"
            )
        }

        fn copy_raw() -> String {
            let raw = Path::new(env!("CARGO_MANIFEST_DIR")).join(OP_FIXTURE);
            format!("cp '{}' results.raw", raw.display())
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
            let projects = scene.state.path().join("simulation");
            std::fs::read_dir(projects)
                .unwrap()
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
            let exited = "cat > status.json <<'JSON'\n{\"protocol\":2,\"load_error\":null,\
                 \"engine_version\":null,\"code_models_loaded\":[],\"code_models_failed\":[],\
                 \"exit_status\":1,\"run_started\":false,\"sim_status\":null,\
                 \"halted_reason\":null,\"results_written\":false}\nJSON"
                .to_string();
            fails_with(&exited, "load_failed", "run").await;
            let unloaded = exited.replace("\"load_error\":null", "\"load_error\":\"no\"");
            let body = fails_with(&unloaded, "engine_load_failed", "engine").await;
            assert_eq!(body["error"]["items"][0], "no");
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
    }
}
