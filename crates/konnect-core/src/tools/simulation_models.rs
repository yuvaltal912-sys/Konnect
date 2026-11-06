//! Fit SPICE models to datasheet values, and attach them to parts.
//!
//! `create_simulation_model` writes a model file into the project's
//! `simulation_models` folder only after the engine has measured every fitted
//! value back within tolerance. A check that could not run writes nothing.
//! The file holds no caller text: the values and the datasheet reference go in
//! a JSON sidecar beside it, which no simulator reads.

use super::simulation::bench::{self as engine_bench, Project};
use super::{get_path, ToolContext, ToolDef};
use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::CallToolResult;
use crate::outcome::{self, OutcomeStatus};
use crate::simulation::models::{self, text, Characteristic, Kind, Model, Role, Statistic};
use crate::tool;
use konnect_sexp::transaction::{commit_file_transaction, FileTransition};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where fitted models live, relative to the project folder.
pub(crate) const MODELS_DIR: &str = "simulation_models";
const SIDECAR_FORMAT: &str = "konnect-fitted-model/1";
const SIDECAR_SUFFIX: &str = ".konnect.json";
/// The numbers came from the caller, who read them from a datasheet.
const SOURCE: &str = "caller_characteristics";
const DEFAULT_TOLERANCE: f64 = 5.0;
const MAX_TOLERANCE: f64 = 50.0;
const DEFAULT_TIMEOUT_SECONDS: u64 = 40;
const MAX_TIMEOUT_SECONDS: u64 = 600;
const MAX_REFERENCE_TEXT: usize = 200;
const MAX_SIDECAR_BYTES: u64 = 1024 * 1024;
/// Parts one assignment may attach a model to.
const MAX_ASSIGNED: usize = 200;

fn characteristic_schema() -> Value {
    let quantities: Vec<&str> = models::Quantity::ALL.iter().map(|q| q.name()).collect();
    let positive = |description: &str| json!({ "type": "number", "exclusiveMinimum": 0, "description": description });
    let non_negative =
        |description: &str| json!({ "type": "number", "minimum": 0, "description": description });
    json!({
        "type": "object",
        "properties": {
            "quantity": {
                "enum": quantities,
                "description": "What the value is. Units are fixed: volts, amps, ohms, farads, henries, seconds, hertz"
            },
            "value": positive("The datasheet value, as a plain number in the quantity's SI base unit: 0.45 for 450 mV, 1e-6 for 1 uA"),
            "conditions": {
                "type": "object",
                "description": "The datasheet's test conditions for this value, in SI base units",
                "properties": {
                    "forward_current_amps": positive("For forward_voltage"),
                    "reverse_current_amps": positive("For breakdown_voltage, zener_voltage and dynamic_impedance"),
                    "collector_current_amps": positive("For current_gain, base_emitter_voltage, collector_emitter_saturation_voltage and transition_frequency"),
                    "collector_emitter_voltage_volts": positive("For current_gain, base_emitter_voltage and transition_frequency"),
                    "base_current_amps": positive("For collector_emitter_saturation_voltage: the base current the saturation voltage is stated at"),
                    "gate_source_voltage_volts": positive("For on_resistance, drain_current and gate_charge (the gate voltage the charge runs to), as a magnitude"),
                    "drain_source_voltage_volts": positive("For drain_current, input_capacitance, output_capacitance, reverse_transfer_capacitance, gate_charge and gate_drain_charge, as a magnitude"),
                    "drain_current_amps": positive("For gate_threshold_voltage (usually 250 uA), on_resistance, gate_charge and gate_drain_charge"),
                    "peak_pulse_current_amps": positive("For clamping_voltage: the peak pulse current the clamping voltage is stated at"),
                    "reverse_voltage_volts": non_negative("For reverse_current, above 0, junction_capacitance, where 0 gives the zero-bias CJO, and collector_base_capacitance and emitter_base_capacitance"),
                    "voltage_volts": positive("For leakage_current"),
                    "frequency_hz": positive("For junction_capacitance (default 1 MHz), dynamic_impedance (1 kHz), capacitance (1 kHz), equivalent_series_resistance (100 kHz) and inductance (100 kHz)"),
                    "dc_bias_volts": non_negative("For capacitance: the DC bias it was read at, 0 for none. An MLCC loses much of its capacitance under bias, so give the value at the working voltage"),
                    "dc_current_amps": non_negative("For inductance: the DC current it was read at, 0 for none, below saturation")
                }
            },
            "statistic": {
                "enum": ["typical", "minimum", "maximum"],
                "description": "Which column of the datasheet the value is from. A limit is checked one way: a maximum passes at or below it. Never enter an absolute maximum rating"
            },
            "read_from": { "enum": ["table", "figure", "text"] },
            "page": { "type": "string", "pattern": "^[A-Za-z0-9-]{1,16}$" }
        },
        "required": ["quantity", "value", "statistic"]
    })
}

pub fn tools() -> Vec<ToolDef> {
    let kinds: Vec<&str> = Kind::ALL.iter().map(|kind| kind.name()).collect();
    vec![tool!(
        "create_simulation_model",
        "Fit a SPICE model to datasheet values you read, check it on the simulator, and write it to the project's simulation_models folder only if every fitted value comes back within tolerance. Konnect never reads the datasheet: it fits and checks the numbers given. Kinds: diode, schottky_diode, led and zener need at least two forward_voltage values (a zener needs zener_voltage and dynamic_impedance instead); n_mosfet and p_mosfet (power MOSFETs, as a three-pin subcircuit) need a gate_threshold_voltage, an on_resistance and at least two drain_current values read off the transfer curve, all as magnitudes, with optional forward_voltage values for the body diode, a gate_resistance, a capacitance curve (three or more reverse_transfer_capacitance values, plus input_capacitance and output_capacitance at the same drain voltages) and gate_charge and gate_drain_charge, which are checked on a switching bench and never fitted; npn and pnp (bipolar transistors, as a three-pin subcircuit C, B, E) need at least three current_gain values at collector currents (a curve off the figure), and at least one base_emitter_voltage, all as magnitudes, with optional collector_emitter_saturation_voltage, collector_base_capacitance and emitter_base_capacitance values (reverse_voltage_volts) and a transition_frequency; tvs and tvs_bidirectional need a breakdown_voltage at its test current and a clamping_voltage at its peak pulse current (a bidirectional part takes no forward or capacitance values); resistance (a fuse's cold resistance, a jumper, a shunt), capacitor (capacitance with ESR, ESL and leakage) and inductor (inductance with DC resistance and self-resonance) are exact. Writes the model file and a .konnect.json sidecar, and touches no schematic: attach the model with assign_simulation_model.",
        json!({
            "type": "object",
            "properties": {
                "schematic": { "type": "string", "description": "Root .kicad_sch path. The model goes in its project's simulation_models folder" },
                "kind": { "enum": kinds },
                "model_name": {
                    "type": "string",
                    "pattern": "^[a-z][a-z0-9_]{0,27}_fit$",
                    "description": "Lowercase, ending in _fit, such as bat54_fit. It names the model and its file"
                },
                "characteristics": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": models::MAX_CHARACTERISTICS,
                    "items": characteristic_schema(),
                    "description": "Datasheet values with their test conditions. A curve read from a figure is several values of one quantity; with more than the fit needs, the middle one is held out as a check"
                },
                "temperature_celsius": {
                    "type": "number",
                    "minimum": -55,
                    "maximum": 175,
                    "default": 25,
                    "description": "The one temperature all the values were measured at"
                },
                "tolerance_percent": {
                    "type": "number",
                    "exclusiveMinimum": 0,
                    "maximum": MAX_TOLERANCE,
                    "default": DEFAULT_TOLERANCE,
                    "description": "How far an engine value may be from its target before nothing is written"
                },
                "datasheet_reference": {
                    "type": "object",
                    "description": "Where the values came from, kept in the sidecar file only",
                    "properties": {
                        "title": { "type": "string", "maxLength": MAX_REFERENCE_TEXT },
                        "revision": { "type": "string", "maxLength": MAX_REFERENCE_TEXT },
                        "url": { "type": "string", "maxLength": MAX_REFERENCE_TEXT }
                    }
                },
                "overwrite_existing": {
                    "type": "boolean",
                    "default": false,
                    "description": "Replace a model of this name that Konnect wrote and nobody has edited. A model whose pins would change is never replaced"
                },
                "timeout_seconds": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_TIMEOUT_SECONDS,
                    "default": DEFAULT_TIMEOUT_SECONDS,
                    "description": "Budget for the checks on the simulator"
                }
            },
            "required": ["schematic", "kind", "model_name", "characteristics"]
        }),
        |args, ctx| async move { handle_create_simulation_model(args, ctx).await }
    ), tool!(
        "assign_simulation_model",
        "Attach a model that create_simulation_model wrote to one or more parts that share it, writing the Sim.* fields KiCad's Simulation Model dialog writes: Sim.Library, Sim.Name, Sim.Device and Sim.Pins, hidden, with every old and legacy simulation field removed. Before anything is written, a private copy of the project is exported with kicad-cli, and each part must come out as an element of the model with every model pin on the net of the symbol pin pin_map gives it. All parts are written in one transaction, or none. Refused while KiCad has any sheet of the project open. Run check_simulation_readiness afterwards.",
        json!({
            "type": "object",
            "properties": {
                "schematic": { "type": "string", "description": "Root .kicad_sch path" },
                "references": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_ASSIGNED,
                    "uniqueItems": true,
                    "items": { "type": "string", "pattern": "^[^\\s=]{1,32}$" },
                    "description": "Parts that take this model, from any sheet of the design"
                },
                "library_path": {
                    "type": "string",
                    "description": "The model file, relative to the project folder, as create_simulation_model returned it: simulation_models/<model_name>.lib"
                },
                "model_name": {
                    "type": "string",
                    "pattern": "^[a-z][a-z0-9_]{0,27}_fit$",
                    "description": "The model the file defines"
                },
                "pin_map": {
                    "type": "object",
                    "minProperties": 1,
                    "additionalProperties": { "type": "string" },
                    "description": "Symbol pin number to model pin, each model pin once: {\"1\": \"K\", \"2\": \"A\"} for KiCad's Device:D. The model's pins are in create_simulation_model's model_pins, and readiness lists each symbol's pins and its library Sim.Pins"
                }
            },
            "required": ["schematic", "references", "library_path", "model_name", "pin_map"]
        }),
        |args, ctx| async move { handle_assign_simulation_model(args, ctx).await }
    )]
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

fn failed(result: CallToolResult, target: &str) -> CallToolResult {
    let outcome = outcome::summary(
        OutcomeStatus::Failed,
        target,
        SOURCE,
        1,
        0,
        1,
        Some(outcome::retry_whole_request()),
    );
    outcome::attach(result, outcome)
}

/// The datasheet reference, as text that is safe to keep: short, one line,
/// printable ASCII, and an http(s) URL.
fn datasheet_reference(value: &Value) -> Result<Value, CallToolResult> {
    let Some(object) = value.as_object() else {
        return Ok(Value::Null);
    };
    let mut kept = Map::new();
    for (key, text) in object {
        let field = format!("datasheet_reference.{key}");
        let Some(text) = text.as_str() else {
            return Err(invalid(field, "must be text"));
        };
        let clean = !text.is_empty()
            && text.len() <= MAX_REFERENCE_TEXT
            && text.bytes().all(|byte| (0x20..0x7f).contains(&byte));
        if !clean {
            return Err(invalid(
                field,
                format!(
                    "must be one line of printable ASCII, at most {MAX_REFERENCE_TEXT} characters"
                ),
            ));
        }
        if key == "url" && !(text.starts_with("https://") || text.starts_with("http://")) {
            return Err(invalid(field, "must be an http or https URL"));
        }
        kept.insert(key.clone(), json!(text));
    }
    Ok(Value::Object(kept))
}

/// Where the model and its sidecar go, and what is there now.
struct Plan {
    dir: PathBuf,
    make_dir: bool,
    library: PathBuf,
    sidecar: PathBuf,
    library_before: Option<String>,
    sidecar_before: Option<String>,
}

/// Decide where to write before spending the engine, refusing anything that
/// would block the write: a folder that is a link, a file that is not
/// Konnect's or was edited, a replacement that changes the pins.
fn plan(project: &Project, model: &Model, overwrite: bool) -> Result<Plan, CallToolResult> {
    let conflict = |reason: String| {
        CallToolResult::error_kind(
            ToolErrorKind::Conflict {
                paths: vec![format!("{MODELS_DIR}/{}.lib", model.name)],
            },
            reason,
        )
    };
    let dir = project.dir.join(MODELS_DIR);
    let make_dir = match std::fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() => false,
        Ok(_) => {
            return Err(conflict(format!(
                "{MODELS_DIR} in the project is not a plain folder; Konnect writes models only \
                 into a real folder inside the project"
            )))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => return Err(conflict(format!("Cannot read {MODELS_DIR}: {error}"))),
    };
    let library = dir.join(format!("{}.lib", model.name));
    let sidecar = dir.join(format!("{}{SIDECAR_SUFFIX}", model.name));
    if !make_dir {
        // ngspice and KiCad read names without case, so a file differing only
        // in case is the same model to them.
        let wanted = format!("{}.lib", model.name);
        let clash = std::fs::read_dir(&dir)
            .map_err(|error| conflict(format!("Cannot list {MODELS_DIR}: {error}")))?
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .find(|name| name.eq_ignore_ascii_case(&wanted) && *name != wanted);
        if let Some(clash) = clash {
            return Err(conflict(format!(
                "{MODELS_DIR}/{clash} differs from {wanted} only in case, which the simulator \
                 cannot tell apart; choose another model_name"
            )));
        }
    }
    let existing = |path: &Path, limit: u64| -> Result<Option<String>, CallToolResult> {
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(conflict(format!("Cannot read {}: {error}", path.display()))),
            Ok(meta) if !meta.is_file() => Err(conflict(format!(
                "{} is not a plain file, so Konnect will not write it",
                path.display()
            ))),
            Ok(meta) if meta.len() > limit => Err(conflict(format!(
                "{} is larger than a file Konnect writes",
                path.display()
            ))),
            Ok(_) => std::fs::read_to_string(path)
                .map(Some)
                .map_err(|error| conflict(format!("Cannot read {}: {error}", path.display()))),
        }
    };
    let library_before = existing(&library, text::MAX_FILE_BYTES as u64)?;
    let sidecar_before = existing(&sidecar, MAX_SIDECAR_BYTES)?;
    let konnect_sidecar = |text: &String| {
        serde_json::from_str::<Value>(text)
            .ok()
            .is_some_and(|value| value["format"] == SIDECAR_FORMAT)
    };
    if let (None, Some(sidecar)) = (&library_before, &sidecar_before) {
        // A sidecar left without its model, as an interrupted write leaves one,
        // describes nothing: a Konnect one is replaced.
        if !konnect_sidecar(sidecar) {
            return Err(conflict(format!(
                "{MODELS_DIR}/{}{SIDECAR_SUFFIX} is not a file Konnect wrote; remove it or choose \
                 another model_name",
                model.name
            )));
        }
    }
    if let Some(before) = &library_before {
        if !overwrite {
            return Err(conflict(format!(
                "{MODELS_DIR}/{}.lib already exists. Pass overwrite_existing to replace a model \
                 Konnect wrote, or choose another model_name",
                model.name
            )));
        }
        let old = text::check_fitted(before.as_bytes()).map_err(|reason| {
            conflict(format!(
                "the existing model is not one Konnect wrote, or was edited ({reason}), so it is \
                 not replaced"
            ))
        })?;
        let new = text::check_fitted(model.file().as_bytes())
            .expect("a model Konnect builds passes its own check");
        if old.card != new.card {
            return Err(conflict(format!(
                "the new model's pins ({}) differ from the existing one's ({}), which would \
                 rewire every part it is attached to; choose another model_name",
                new.pins().join(" "),
                old.pins().join(" ")
            )));
        }
        if sidecar_before
            .as_ref()
            .is_some_and(|sidecar| !konnect_sidecar(sidecar))
        {
            return Err(conflict(
                "the existing sidecar is not one Konnect wrote".into(),
            ));
        }
    }
    Ok(Plan {
        dir,
        make_dir,
        library,
        sidecar,
        library_before,
        sidecar_before,
    })
}

/// One row of the comparison: the caller's target against the engine's value.
fn rows(
    characteristics: &[Characteristic],
    model: &Model,
    values: &[Option<f64>],
    tolerance: f64,
) -> Vec<Value> {
    let mut rows = Vec::new();
    for characteristic in characteristics {
        let conditions: Map<String, Value> = characteristic
            .conditions
            .iter()
            .map(|(key, value)| (key.to_string(), json!(value)))
            .collect();
        let base = json!({
            "characteristic": characteristic.index,
            "quantity": characteristic.quantity.name(),
            "statistic": characteristic.statistic.name(),
            "conditions": conditions,
            "target": characteristic.value,
            "unit": characteristic.quantity.unit(),
        });
        let checks: Vec<(usize, &models::Check)> = model
            .checks
            .iter()
            .enumerate()
            .filter(|(_, check)| check.characteristic == characteristic.index)
            .collect();
        if checks.is_empty() {
            let mut row = base;
            row["role"] = json!("not_checked");
            row["engine"] = Value::Null;
            rows.push(row);
            continue;
        }
        for (at, check) in checks {
            let mut row = base.clone();
            row["role"] = json!(check.role.name());
            row["measured_at_hz"] = json!(check.probe.hz());
            match values.get(at).copied().flatten() {
                Some(engine) => {
                    let error = 100.0 * (engine - check.expected) / check.expected.abs();
                    // A value the model is only compared with, given as a datasheet
                    // limit, passes on its own side of the limit.
                    let (comparison, within) = match (check.role, characteristic.statistic) {
                        (Role::CheckOnly, Statistic::Maximum) => ("at_most", error <= tolerance),
                        (Role::CheckOnly, Statistic::Minimum) => ("at_least", error >= -tolerance),
                        _ => ("within", error.abs() <= tolerance),
                    };
                    row["engine"] = json!(engine);
                    row["error_percent"] = json!(error);
                    row["comparison"] = json!(comparison);
                    row["within_tolerance"] = json!(within);
                }
                None => {
                    row["engine"] = Value::Null;
                    row["within_tolerance"] = json!(false);
                    row["finding"] = json!("signal_not_found");
                }
            }
            rows.push(row);
        }
    }
    rows
}

/// Rows that decide whether the model is written: the fitted and held-out ones.
fn deciding(row: &Value) -> bool {
    matches!(
        row["role"].as_str(),
        Some(name) if name == Role::Fit.name() || name == Role::HeldOut.name()
    )
}

async fn handle_create_simulation_model(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let schematic = get_path(args, "schematic")?;
    let model_name = args["model_name"].as_str().unwrap_or_default();
    let target = format!("{MODELS_DIR}/{model_name}.lib");
    let fail = |result: CallToolResult| Ok(failed(result, &target));
    let kind_name = args["kind"].as_str().unwrap_or_default();
    let Some(kind) = Kind::from_name(kind_name) else {
        return fail(invalid(
            "kind",
            format!("{kind_name} is not a kind Konnect fits"),
        ));
    };
    let temperature = args["temperature_celsius"]
        .as_f64()
        .unwrap_or(models::DEFAULT_TEMPERATURE);
    let tolerance = args["tolerance_percent"]
        .as_f64()
        .unwrap_or(DEFAULT_TOLERANCE);
    if !(tolerance > 0.0 && tolerance <= MAX_TOLERANCE) {
        return fail(invalid(
            "tolerance_percent",
            format!("tolerance_percent must be above 0 and at most {MAX_TOLERANCE}"),
        ));
    }
    let overwrite = args["overwrite_existing"].as_bool().unwrap_or(false);
    let timeout = args["timeout_seconds"]
        .as_u64()
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
        .clamp(1, MAX_TIMEOUT_SECONDS);
    let reference = match datasheet_reference(&args["datasheet_reference"]) {
        Ok(reference) => reference,
        Err(refusal) => return fail(refusal),
    };
    let characteristics = match models::parse_characteristics(&args["characteristics"]) {
        Ok(characteristics) => characteristics,
        Err(error) => return fail(invalid(error.field, error.reason)),
    };
    let model = match models::build(kind, model_name, &characteristics, temperature) {
        Ok(model) => model,
        Err(error) => return fail(invalid(error.field, error.reason)),
    };
    let project = match engine_bench::project_of(&schematic) {
        Ok(project) => project,
        Err(refusal) => return fail(refusal),
    };
    let plan = match plan(&project, &model, overwrite) {
        Ok(plan) => plan,
        Err(refusal) => return fail(refusal),
    };
    let measured = match engine_bench::measure(
        ctx,
        &project,
        &model,
        temperature,
        Duration::from_secs(timeout),
    )
    .await
    {
        Ok(measured) => measured,
        Err(refusal) => return fail(refusal),
    };
    let rows = rows(&characteristics, &model, &measured.values, tolerance);
    let worst = rows
        .iter()
        .filter(|row| deciding(row))
        .filter_map(|row| row["error_percent"].as_f64())
        .map(f64::abs)
        .fold(0.0, f64::max);
    let passed = rows
        .iter()
        .filter(|row| deciding(row))
        .all(|row| row["within_tolerance"] == true);
    let mut warnings = Vec::new();
    if rows
        .iter()
        .any(|row| row["role"] == Role::CheckOnly.name() && row["within_tolerance"] == false)
    {
        warnings.push(json!({
            "code": "check_only_outside_tolerance",
            "detail": "A value the model is only compared with, such as a typical reverse current, reads outside tolerance or past its datasheet limit. It did not stop the write; say so when it matters."
        }));
    }
    let parameters: Map<String, Value> = model
        .parameters
        .iter()
        .map(|(key, value)| (key.clone(), json!(value)))
        .collect();
    let verification = json!({
        "status": if passed { "passed" } else { "failed" },
        "engine_version": measured.engine_version,
        "temperature_celsius": temperature,
        "tolerance_percent": tolerance,
        "worst_error_percent": worst,
        "rows": rows,
    });
    let fit = json!({ "fitted": model.fitted, "assumed": model.assumed });
    let library_path = format!("{MODELS_DIR}/{model_name}.lib");
    if !passed {
        let body = json!({
            "written": false,
            "reason": "fit_out_of_tolerance",
            "model_name": model_name,
            "kind": kind.name(),
            "parameters": parameters,
            "fit": fit,
            "verification": verification,
            "warnings": warnings,
        });
        let outcome = outcome::summary(
            OutcomeStatus::Failed,
            &target,
            SOURCE,
            1,
            0,
            1,
            Some(outcome::retry_whole_request()),
        );
        return Ok(outcome::attach(CallToolResult::json(&body), outcome));
    }

    let file = model.file();
    let sidecar = json!({
        "format": SIDECAR_FORMAT,
        "model_name": model_name,
        "kind": kind.name(),
        "library_path": library_path,
        "body_sha256": format!("{:x}", Sha256::digest(model.body.as_bytes())),
        "datasheet_reference": reference,
        "characteristics": args["characteristics"],
        "parameters": parameters,
        "fit": fit,
        "verification": verification,
    });
    let sidecar_text = format!("{}\n", serde_json::to_string_pretty(&sidecar)?);
    let folder_conflict = |reason: String| {
        CallToolResult::error_kind(
            ToolErrorKind::Conflict {
                paths: vec![MODELS_DIR.to_string()],
            },
            reason,
        )
    };
    if plan.make_dir {
        match std::fs::create_dir(&plan.dir) {
            Ok(()) => {}
            // Another call made it meanwhile: a real folder is as good.
            Err(error)
                if error.kind() == std::io::ErrorKind::AlreadyExists
                    && std::fs::symlink_metadata(&plan.dir).is_ok_and(|meta| meta.is_dir()) => {}
            Err(error) => {
                return fail(folder_conflict(format!(
                    "Cannot create {MODELS_DIR}: {error}"
                )))
            }
        }
    }
    let changes = [
        (&plan.library, &plan.library_before, file.as_str()),
        (&plan.sidecar, &plan.sidecar_before, sidecar_text.as_str()),
    ];
    // A refit that comes out byte for byte the same writes nothing: the
    // transaction refuses a target it would not change.
    let pending: Vec<Target<'_>> = changes
        .iter()
        .filter(|(_, before, after)| before.as_deref() != Some(*after))
        .map(|(path, before, after)| (path.as_path(), before.as_deref(), *after))
        .collect();
    let transitions: Vec<FileTransition> = pending
        .iter()
        .map(|(path, before, after)| match before {
            Some(before) => FileTransition::replace(*path, before.to_string(), after.to_string()),
            None => FileTransition::create(*path, after.to_string()),
        })
        .collect();
    let changed_files = transitions.len();
    if !transitions.is_empty() {
        if let Err(error) = commit_file_transaction(&project.dir, transitions) {
            return match left_after_error(&project.dir, &pending) {
                Left::Nothing => fail(folder_conflict(format!(
                    "The model passed its checks, but nothing was written: {error}"
                ))),
                Left::Changed(files) => fail(changed_meanwhile(files)),
                Left::Maybe { files, journal } => Ok(uncertain_write(
                    Uncertain {
                        operation: "create_simulation_model",
                        target: &target,
                        source: SOURCE,
                        requested: 1,
                    },
                    &files,
                    journal,
                    format!("the write stopped before it finished ({error})"),
                )),
            };
        }
    }
    // What the result reports comes from the file as it now is.
    let written = std::fs::read(&plan.library).unwrap_or_default();
    let readback = text::check_fitted(&written);
    if written != file.as_bytes() || readback.is_err() {
        return match after_mismatch(&pending, &plan.library) {
            Mismatch::Changed(files) => fail(changed_meanwhile(files)),
            Mismatch::Uncertain(files) => Ok(uncertain_write(
                Uncertain {
                    operation: "create_simulation_model",
                    target: &target,
                    source: SOURCE,
                    requested: 1,
                },
                &files,
                false,
                "the model on disk does not read back as the one checked".to_string(),
            )),
        };
    }
    let checked = readback.expect("checked above");
    let body = json!({
        "written": changed_files > 0,
        "unchanged": changed_files == 0,
        "replaced": plan.library_before.is_some(),
        "model_name": checked.name,
        "kind": checked.kind,
        "library_path": library_path,
        "sidecar_path": format!("{MODELS_DIR}/{model_name}{SIDECAR_SUFFIX}"),
        "file": {
            "bytes": written.len(),
            "sha256": format!("{:x}", Sha256::digest(&written)),
        },
        "model_pins": checked.pins(),
        "sim_device": checked.sim_device(),
        "parameters": parameters,
        "fit": fit,
        "verification": verification,
        "warnings": warnings,
    });
    let outcome = outcome::summary(OutcomeStatus::Complete, &target, SOURCE, 1, 1, 0, None);
    Ok(outcome::attach(CallToolResult::json(&body), outcome))
}

/// Read a model file Konnect wrote, refusing any other: a fitted file is the
/// one kind of library this tool will wire into KiCad, whose GUI includes it
/// unvetted.
fn fitted_library(
    project: &Project,
    library_path: &str,
) -> Result<(String, text::Fitted), CallToolResult> {
    let relative = library_path
        .strip_prefix("${KIPRJMOD}/")
        .unwrap_or(library_path);
    let plain = !relative.is_empty()
        && !relative.starts_with('/')
        && !relative.contains(['\\', ':', '$', '~'])
        && relative
            .split('/')
            .all(|part| !part.is_empty() && part != ".." && part != ".");
    if !plain {
        return Err(invalid(
            "library_path",
            "library_path is a path inside the project folder, such as simulation_models/x_fit.lib",
        ));
    }
    let path = project.dir.join(relative);
    let inside = path
        .canonicalize()
        .is_ok_and(|canonical| canonical.starts_with(&project.dir) && canonical == path);
    let file = std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file());
    if !inside || !file {
        return Err(invalid(
            "library_path",
            format!("{relative} is not a plain file inside the project"),
        ));
    }
    let bytes = match std::fs::metadata(&path) {
        Ok(meta) if meta.len() <= text::MAX_FILE_BYTES as u64 => std::fs::read(&path).ok(),
        _ => None,
    }
    .ok_or_else(|| {
        invalid(
            "library_path",
            format!("{relative} is not a model Konnect wrote"),
        )
    })?;
    let fitted = text::check_fitted(&bytes).map_err(|reason| {
        invalid(
            "library_path",
            format!(
                "{relative} is not a model Konnect wrote, or was edited since ({reason}). Only \
                 fitted models are attached by this tool"
            ),
        )
    })?;
    Ok((relative.to_string(), fitted))
}

/// Symbol pins in KiCad's order: numbers by value, then text.
fn pin_order(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(a), Ok(b)) => a.cmp(&b),
        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        _ => a.cmp(b),
    }
}

/// The pin map as `(symbol pin, model pin)` pairs in pin order, each model pin
/// mapped exactly once.
fn read_pin_map(value: &Value, ports: &[String]) -> Result<Vec<(String, String)>, CallToolResult> {
    let Some(map) = value.as_object() else {
        return Err(invalid(
            "pin_map",
            "pin_map maps symbol pin numbers to model pins",
        ));
    };
    let mut pairs = Vec::new();
    for (pin, port) in map {
        let pin_ok = !pin.is_empty()
            && pin.len() <= 16
            && !pin.contains(|c: char| c.is_whitespace() || c == '=');
        if !pin_ok {
            return Err(invalid(
                format!("pin_map.{pin}"),
                "a symbol pin number has no blanks or =",
            ));
        }
        let Some(port) = port
            .as_str()
            .filter(|port| ports.iter().any(|known| known == port))
        else {
            return Err(invalid(
                format!("pin_map.{pin}"),
                format!("the model's pins are {}", ports.join(", ")),
            ));
        };
        pairs.push((pin.clone(), port.to_string()));
    }
    for port in ports {
        let uses = pairs.iter().filter(|(_, mapped)| mapped == port).count();
        if uses != 1 {
            return Err(invalid(
                "pin_map",
                format!("model pin {port} must be mapped exactly once, and is mapped {uses} times"),
            ));
        }
    }
    pairs.sort_by(|a, b| pin_order(&a.0, &b.0));
    Ok(pairs)
}

fn assign_failed(result: CallToolResult, target: &str, requested: usize) -> CallToolResult {
    let outcome = outcome::summary(
        OutcomeStatus::Failed,
        target,
        "saved_schematic",
        requested,
        0,
        requested,
        Some(outcome::retry_whole_request()),
    );
    outcome::attach(result, outcome)
}

fn named(fields: &[(String, String)]) -> Vec<Value> {
    fields
        .iter()
        .map(|(name, value)| json!({ "name": name, "value": value }))
        .collect()
}

async fn handle_assign_simulation_model(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    use crate::simulation::snapshot;
    use crate::tools::sch_components::{
        plan_sim_assignment, read_back_sim_assignment, symbol_pins,
    };
    use crate::tools::simulation::assignment;

    let schematic = get_path(args, "schematic")?;
    let target = schematic.display().to_string();
    let library_path = args["library_path"].as_str().unwrap_or_default();
    let model_name = args["model_name"].as_str().unwrap_or_default();
    let references: Vec<String> = args["references"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let requested = references.len().max(1);
    let fail = |result: CallToolResult| Ok(assign_failed(result, &target, requested));
    if references.is_empty() || references.len() > MAX_ASSIGNED {
        return fail(invalid(
            "references",
            format!("give from 1 to {MAX_ASSIGNED} references"),
        ));
    }
    let project = match engine_bench::project_of(&schematic) {
        Ok(project) => project,
        Err(refusal) => return fail(refusal),
    };
    let (library, fitted) = match fitted_library(&project, library_path) {
        Ok(found) => found,
        Err(refusal) => return fail(refusal),
    };
    if fitted.name != model_name {
        return fail(invalid(
            "model_name",
            format!("{library} defines {}, not {model_name}", fitted.name),
        ));
    }
    let ports = fitted.pins();
    let pin_map = match read_pin_map(&args["pin_map"], &ports) {
        Ok(pin_map) => pin_map,
        Err(refusal) => return fail(refusal),
    };
    let root = schematic.canonicalize()?;
    let sheets = match snapshot::hierarchy_files(&root) {
        Ok(sheets) => sheets,
        Err(error) => return fail(invalid("schematic", format!("{error:#}"))),
    };
    // Eeschema locks only the file it opened, and saves every sheet it holds:
    // a write to any sheet while it is open would be lost at its next save.
    let project_file = root.with_extension("kicad_pro");
    let locks = kicad_locks(&sheets, &project_file);
    if !locks.is_empty() {
        return fail(CallToolResult::error_kind(
            ToolErrorKind::Conflict { paths: locks },
            "KiCad has this project open. Close it in KiCad, then assign the models, then reopen it.",
        ));
    }
    let mut contents: Vec<(PathBuf, String)> = Vec::new();
    for sheet in &sheets {
        match konnect_sexp::writer::read_consistent(sheet) {
            Ok(content) => contents.push((sheet.clone(), content)),
            Err(error) => {
                return fail(CallToolResult::error_kind(
                    ToolErrorKind::Conflict {
                        paths: vec![sheet.display().to_string()],
                    },
                    format!("Cannot read {}: {error}", sheet.display()),
                ))
            }
        }
    }
    let mut placed: Vec<(usize, Vec<String>)> = Vec::new();
    let mut parts = Vec::new();
    for reference in &references {
        let holders: Vec<usize> = contents
            .iter()
            .enumerate()
            .filter(|(_, (_, content))| {
                !crate::tools::find_all_symbol_instance_blocks(content, reference).is_empty()
            })
            .map(|(index, _)| index)
            .collect();
        let [sheet] = holders.as_slice() else {
            let reason = match holders.len() {
                0 => format!("{reference} is not on any sheet of this design"),
                count => format!("{reference} is placed on {count} sheets, so it is ambiguous"),
            };
            return fail(invalid("references", reason));
        };
        let pins = match symbol_pins(&contents[*sheet].1, reference) {
            Ok(pins) => pins,
            Err(reason) => return fail(invalid("references", reason)),
        };
        if let Some((pin, _)) = pin_map
            .iter()
            .find(|(pin, _)| !pins.iter().any(|(number, _)| number == pin))
        {
            let known: Vec<String> = pins
                .iter()
                .map(|(number, name)| match name {
                    Some(name) => format!("{number} ({name})"),
                    None => number.clone(),
                })
                .collect();
            return fail(invalid(
                format!("pin_map.{pin}"),
                format!(
                    "{reference} has no pin {pin}; its pins are {}",
                    known.join(", ")
                ),
            ));
        }
        if let Some(reason) = contradiction(&pins, &pin_map, &ports) {
            return fail(invalid("pin_map", format!("{reference}: {reason}")));
        }
        if is_open_jumper(&contents[*sheet].1, reference) {
            return fail(invalid(
                "references",
                format!(
                    "{reference} is an open solder jumper, which conducts nothing. Leave it out \
                     with testbench.exclude_references, or bridge it in the schematic first"
                ),
            ));
        }
        parts.push(assignment::Expected {
            reference,
            pins: pins.into_iter().map(|(number, _)| number).collect(),
        });
        match placed.iter_mut().find(|(index, _)| index == sheet) {
            Some((_, list)) => list.push(reference.clone()),
            None => placed.push((*sheet, vec![reference.clone()])),
        }
    }
    let sim_pins: Vec<String> = pin_map
        .iter()
        .map(|(pin, port)| format!("{pin}={port}"))
        .collect();
    let fields: Vec<(String, String)> = vec![
        // KiCad tries a bare relative path against its working folder first.
        ("Sim.Library".into(), format!("${{KIPRJMOD}}/{library}")),
        ("Sim.Name".into(), model_name.to_string()),
        ("Sim.Device".into(), fitted.sim_device().to_string()),
        ("Sim.Pins".into(), sim_pins.join(" ")),
    ];
    let mut edited: Vec<(PathBuf, String)> = Vec::new();
    let mut plans = Vec::new();
    for (index, list) in &placed {
        let (sheet, content) = &contents[*index];
        match plan_sim_assignment(sheet, content, list, &fields) {
            Ok((after, assignments)) => {
                edited.push((sheet.clone(), after));
                plans.push((sheet.clone(), assignments));
            }
            Err(refusal) => return fail(refusal),
        }
    }
    let elements = match assignment::check(
        &ctx.config.kicad_cli,
        &root,
        &project.dir,
        &edited,
        &parts,
        model_name,
        &ports,
        &pin_map,
    )
    .await
    {
        Ok(elements) => elements,
        Err(refusal) => return fail(refusal),
    };
    let pending: Vec<Target<'_>> = edited
        .iter()
        .filter_map(|(sheet, after)| {
            let before = &contents.iter().find(|(path, _)| path == sheet)?.1;
            (before != after).then_some((sheet.as_path(), Some(before.as_str()), after.as_str()))
        })
        .collect();
    let transitions: Vec<FileTransition> = pending
        .iter()
        .map(|(sheet, before, after)| {
            FileTransition::replace(
                *sheet,
                before.unwrap_or_default().to_string(),
                after.to_string(),
            )
        })
        .collect();
    // The export took time, and KiCad may have opened the project meanwhile;
    // it would then save its own copy of every sheet over this write.
    let late_locks = kicad_locks(&sheets, &project_file);
    if !late_locks.is_empty() {
        return fail(CallToolResult::error_kind(
            ToolErrorKind::Conflict { paths: late_locks },
            "KiCad opened this project while the assignment was being checked. Nothing was \
             written. Close it in KiCad and retry.",
        ));
    }
    if !transitions.is_empty() {
        if let Err(error) = commit_file_transaction(&project.dir, transitions) {
            return match left_after_error(&project.dir, &pending) {
                Left::Nothing => fail(CallToolResult::error_kind(
                    ToolErrorKind::Conflict {
                        paths: pending
                            .iter()
                            .map(|(sheet, ..)| sheet.display().to_string())
                            .collect(),
                    },
                    format!("Nothing was written: {error}"),
                )),
                Left::Changed(files) => fail(changed_meanwhile(files)),
                Left::Maybe { files, journal } => Ok(uncertain_write(
                    Uncertain {
                        operation: "assign_simulation_model",
                        target: &target,
                        source: "saved_schematic",
                        requested,
                    },
                    &files,
                    journal,
                    format!("the write stopped before it finished ({error})"),
                )),
            };
        }
    }
    let mut reports = Vec::new();
    let mut observed_fields = Value::Null;
    for (sheet, assignments) in &plans {
        for assignment in assignments {
            let observed = match read_back_sim_assignment(sheet, assignment, &fields) {
                Ok(observed) => observed,
                Err(_) => {
                    return match after_mismatch(&pending, sheet) {
                        Mismatch::Changed(files) => fail(changed_meanwhile(files)),
                        Mismatch::Uncertain(files) => Ok(uncertain_write(
                            Uncertain {
                                operation: "assign_simulation_model",
                                target: &target,
                                source: "saved_schematic",
                                requested,
                            },
                            &files,
                            false,
                            format!(
                                "{} does not read back with the planned fields",
                                sheet.display()
                            ),
                        )),
                    };
                }
            };
            let sim: Map<String, Value> = observed["units"][0]["fields"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(name, _)| name.starts_with("Sim."))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            if observed_fields.is_null() {
                observed_fields = Value::Object(sim);
            }
            let element = elements
                .iter()
                .find(|element| element["reference"] == assignment.reference.as_str())
                .cloned()
                .unwrap_or(Value::Null);
            reports.push(json!({
                "reference": assignment.reference,
                "sheet": sheet.strip_prefix(&project.dir).unwrap_or(sheet).display().to_string(),
                "units": assignment.unit_count,
                "sim_fields_dropped": named(&assignment.dropped),
                "element": element["element"],
                "nodes": element["nodes"],
            }));
        }
    }
    let pin_map_report: Map<String, Value> = pin_map
        .iter()
        .map(|(pin, port)| (pin.clone(), json!(port)))
        .collect();
    let body = json!({
        "assigned": reports.len(),
        "model": {
            "name": fitted.name,
            "kind": fitted.kind,
            "sim_device": fitted.sim_device(),
            "pins": ports,
        },
        "library_path": library,
        "pin_map": pin_map_report,
        "sim_fields": observed_fields,
        "parts": reports,
    });
    let outcome = outcome::summary(
        OutcomeStatus::Complete,
        &target,
        "saved_schematic",
        requested,
        requested,
        0,
        None,
    );
    Ok(outcome::attach(CallToolResult::json(&body), outcome))
}

/// A pin map that contradicts the symbol's own pin names: an anode mapped to a
/// cathode, or a one-way diode on a symbol whose pins say it conducts both
/// ways. Pin numbers alone cannot show either.
fn contradiction(
    pins: &[(String, Option<String>)],
    pin_map: &[(String, String)],
    ports: &[String],
) -> Option<String> {
    let one_way = ports.iter().any(|port| port == "A") && ports.iter().any(|port| port == "K");
    for (pin, port) in pin_map {
        let Some(name) = pins
            .iter()
            .find(|(number, _)| number == pin)
            .and_then(|(_, name)| name.as_deref())
        else {
            continue;
        };
        let name = name.to_ascii_uppercase();
        if one_way && matches!(name.as_str(), "A" | "K") && name != *port {
            return Some(format!(
                "symbol pin {pin} is named {name}, but pin_map gives it the model's {port}, which \
                 would put the diode in backwards"
            ));
        }
        if one_way && matches!(name.as_str(), "A1" | "A2") {
            return Some(
                "the symbol's pins are A1 and A2, a part that conducts both ways, and a one-way \
                 diode model would clamp one side at a diode drop"
                    .to_string(),
            );
        }
    }
    None
}

/// Whether the part is a solder jumper drawn open.
fn is_open_jumper(content: &str, reference: &str) -> bool {
    crate::tools::find_all_symbol_instance_blocks(content, reference)
        .first()
        .and_then(|(start, end)| {
            let block = &content[*start..*end];
            let at = block.find("(lib_id \"")? + "(lib_id \"".len();
            let lib_id = &block[at..at + block[at..].find('"')?];
            Some(lib_id.to_ascii_lowercase())
        })
        .is_some_and(|lib_id| lib_id.contains("jumper") && lib_id.contains("open"))
}

/// The KiCad editor locks on any sheet of the design, or on its project file.
fn kicad_locks(sheets: &[PathBuf], project_file: &Path) -> Vec<String> {
    sheets
        .iter()
        .map(PathBuf::as_path)
        .chain(std::iter::once(project_file))
        .filter_map(konnect_sexp::writer::kicad_editor_lock_path)
        .filter(|lock| lock.exists())
        .map(|lock| lock.display().to_string())
        .collect()
}

/// A file a transaction writes: its path, its text before, and the text this
/// call would leave.
type Target<'a> = (&'a Path, Option<&'a str>, &'a str);

/// What a transaction that returned an error left behind.
#[derive(Debug)]
enum Left {
    /// Every file is as it was, and no journal can finish the write.
    Nothing,
    /// Another writer changed these files while this call ran; it wrote none.
    Changed(Vec<String>),
    /// These files may hold this call's text: one did when it was read, or a
    /// journal the write left can still complete it.
    Maybe { files: Vec<String>, journal: bool },
}

/// Read each file after a failed commit, to tell this call's write from
/// another writer's change.
fn left_after_error(project_dir: &Path, targets: &[Target<'_>]) -> Left {
    // Journals first: a recovery that finishes one between the two reads
    // then shows up in the files.
    let journal = journal_names(project_dir, targets);
    let mut written = Vec::new();
    let mut changed = Vec::new();
    let mut unknown = Vec::new();
    for (path, before, after) in targets {
        let shown = path.display().to_string();
        match std::fs::read_to_string(path) {
            Ok(now) if now == *after => written.push(shown),
            Ok(now) if Some(now.as_str()) == *before => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && before.is_none() => {}
            // A transaction never deletes a file, so a gone one is another
            // writer's doing.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => changed.push(shown),
            Ok(_) => changed.push(shown),
            Err(_) => unknown.push(shown),
        }
    }
    if journal {
        let files = targets
            .iter()
            .map(|(path, ..)| path.display().to_string())
            .collect();
        return Left::Maybe { files, journal };
    }
    if !written.is_empty() || !unknown.is_empty() {
        written.extend(unknown);
        return Left::Maybe {
            files: written,
            journal,
        };
    }
    if !changed.is_empty() {
        return Left::Changed(changed);
    }
    Left::Nothing
}

/// Whether an active journal in the project names one of these files, so a
/// later recovery may still write it. A project that cannot be inspected
/// counts as one that does.
fn journal_names(project_dir: &Path, targets: &[Target<'_>]) -> bool {
    let Ok(statuses) = konnect_sexp::transaction::inspect_file_transactions(project_dir) else {
        return true;
    };
    // A journal names each file under its folder's canonical spelling, which
    // on a case-insensitive disk may differ from the one this call used.
    let relative: Vec<PathBuf> = targets
        .iter()
        .filter_map(|(path, ..)| {
            let folder = path.parent()?.canonicalize().ok()?;
            let file = folder.join(path.file_name()?);
            file.strip_prefix(project_dir).ok().map(Path::to_path_buf)
        })
        .collect();
    statuses.iter().any(|status| {
        status
            .targets
            .iter()
            .any(|target| relative.contains(&target.path))
    })
}

/// What a file that reads back wrong after the write means.
#[derive(Debug, PartialEq)]
enum Mismatch {
    /// Nothing was committed, so another writer changed it.
    Changed(Vec<String>),
    /// The commit wrote these files, the mismatched one listed last if it
    /// was not among them: inspect them all.
    Uncertain(Vec<String>),
}

fn after_mismatch(pending: &[Target<'_>], file: &Path) -> Mismatch {
    let file = file.display().to_string();
    if pending.is_empty() {
        return Mismatch::Changed(vec![file]);
    }
    let mut files: Vec<String> = pending
        .iter()
        .map(|(path, ..)| path.display().to_string())
        .collect();
    if !files.contains(&file) {
        files.push(file);
    }
    Mismatch::Uncertain(files)
}

/// Another writer changed files this call had read, so it wrote nothing.
fn changed_meanwhile(files: Vec<String>) -> CallToolResult {
    let shown = files.join(", ");
    CallToolResult::error_kind(
        ToolErrorKind::Conflict { paths: files },
        format!("{shown} changed while this call ran, so it wrote nothing. Check the change, then retry."),
    )
}

/// Which call an uncertain answer is about.
struct Uncertain<'a> {
    operation: &'a str,
    target: &'a str,
    source: &'a str,
    requested: usize,
}

/// Files this call may have written, though not as it meant to: inspect them
/// before retrying.
fn uncertain_write(
    call: Uncertain<'_>,
    files: &[String],
    journal: bool,
    reason: String,
) -> CallToolResult {
    let shown = files.join(", ");
    let pending = if journal {
        " A journal the write left can still complete it on Konnect's next write to this \
         project; `konnect transaction status <project-dir>` shows it."
    } else {
        ""
    };
    let result = CallToolResult::error_kind(
        ToolErrorKind::MutationOutcomeUncertain {
            operation: call.operation.to_string(),
            path: shown.clone(),
            reason: reason.clone(),
        },
        format!(
            "{} may have written {shown}: {reason}.{pending} Inspect before retrying.",
            call.operation
        ),
    );
    let outcome = outcome::summary(
        OutcomeStatus::Uncertain,
        call.target,
        call.source,
        call.requested,
        0,
        call.requested,
        Some(outcome::inspect_before_retry()),
    );
    outcome::attach(result, outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::error::extract_error_kind;
    use crate::mcp::protocol::ToolContent;
    use crate::router::ToolRouter;
    use crate::tools::ServerConfig;
    use std::sync::Arc;

    const SHEET: &str = include_str!("../../tests/fixtures/component_fields_kicad10.kicad_sch");

    fn context() -> ToolContext {
        ToolContext::new(
            ServerConfig {
                kicad_cli: "/nonexistent/kicad-cli".to_string(),
                ..Default::default()
            },
            Arc::new(ToolRouter::new()),
        )
    }

    fn body(result: &CallToolResult) -> Value {
        match result.content.first() {
            Some(ToolContent::Text { text }) => serde_json::from_str(text).unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }

    /// A project holding the KiCad-authored sheet, with R1 (Device:R).
    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("top.kicad_sch"), SHEET).unwrap();
        dir
    }

    fn resistance() -> Value {
        json!([{ "quantity": "resistance", "value": 0.012, "statistic": "typical" }])
    }

    /// Write a fitted resistance model into the project, as create would.
    fn fitted_model(dir: &Path, name: &str) -> String {
        let characteristics = models::parse_characteristics(&resistance()).unwrap();
        let model = models::build(Kind::Resistance, name, &characteristics, 25.0).unwrap();
        std::fs::create_dir_all(dir.join(MODELS_DIR)).unwrap();
        let path = format!("{MODELS_DIR}/{name}.lib");
        std::fs::write(dir.join(&path), model.file()).unwrap();
        path
    }

    fn create_args(dir: &Path, name: &str) -> Value {
        json!({
            "schematic": dir.join("top.kicad_sch"),
            "kind": "resistance",
            "model_name": name,
            "characteristics": resistance(),
        })
    }

    fn assign_args(dir: &Path, library_path: &str, name: &str, pin_map: Value) -> Value {
        json!({
            "schematic": dir.join("top.kicad_sch"),
            "references": ["R1"],
            "library_path": library_path,
            "model_name": name,
            "pin_map": pin_map,
        })
    }

    async fn create(args: Value) -> CallToolResult {
        handle_create_simulation_model(&args, &context())
            .await
            .unwrap()
    }

    async fn assign(args: Value) -> CallToolResult {
        handle_assign_simulation_model(&args, &context())
            .await
            .unwrap()
    }

    fn field(result: &CallToolResult) -> String {
        body(result)["error"]["field"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    #[tokio::test]
    async fn create_refuses_bad_input_before_anything_is_written() {
        let dir = project();
        let mut args = create_args(dir.path(), "Fuse_fit");
        let refused = create(args.clone()).await;
        assert_eq!(field(&refused), "model_name", "{:?}", body(&refused));
        args["model_name"] = json!("fuse_fit");
        args["kind"] = json!("op_amp");
        assert_eq!(field(&create(args.clone()).await), "kind");
        args["kind"] = json!("resistance");
        args["characteristics"] =
            json!([{ "quantity": "resistance", "value": "12m", "statistic": "typical" }]);
        assert_eq!(
            field(&create(args.clone()).await),
            "characteristics[0].value"
        );
        args["characteristics"] = resistance();
        args["datasheet_reference"] = json!({ "title": "fuse\n*#shell echo KMARK" });
        let refused = create(args).await;
        assert_eq!(field(&refused), "datasheet_reference.title");
        assert_eq!(body(&refused)["outcome"]["status"], "failed");
        assert!(!dir.path().join(MODELS_DIR).exists());
    }

    #[tokio::test]
    async fn create_writes_nothing_when_the_engine_cannot_check_the_model() {
        let dir = project();
        let refused = create(create_args(dir.path(), "fuse_fit")).await;
        assert!(refused.is_error);
        assert_eq!(
            body(&refused)["error"]["kind"],
            "simulation_failed",
            "{:?}",
            body(&refused)
        );
        assert!(!dir.path().join(MODELS_DIR).exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn create_refuses_a_models_folder_that_is_a_link() {
        let dir = project();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join(MODELS_DIR)).unwrap();
        let refused = create(create_args(dir.path(), "fuse_fit")).await;
        assert_eq!(extract_error_kind(&refused).as_deref(), Some("conflict"));
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn create_never_replaces_a_file_konnect_did_not_write_or_that_was_edited() {
        let dir = project();
        std::fs::create_dir(dir.path().join(MODELS_DIR)).unwrap();
        let path = dir.path().join(MODELS_DIR).join("fuse_fit.lib");
        std::fs::write(&path, ".model fuse_fit D\n+ IS=1e-14\n").unwrap();
        let mut args = create_args(dir.path(), "fuse_fit");
        assert_eq!(
            extract_error_kind(&create(args.clone()).await).as_deref(),
            Some("conflict")
        );
        args["overwrite_existing"] = json!(true);
        let refused = create(args.clone()).await;
        assert_eq!(extract_error_kind(&refused).as_deref(), Some("conflict"));
        assert!(body(&refused)["message"]
            .as_str()
            .unwrap_or_default()
            .contains("not one Konnect wrote"));
        // A Konnect file edited by hand is not replaced either.
        fitted_model(dir.path(), "fuse_fit");
        let edited = std::fs::read_to_string(&path)
            .unwrap()
            .replace("1.200000e-2", "1.300000e-2");
        std::fs::write(&path, &edited).unwrap();
        assert_eq!(
            extract_error_kind(&create(args).await).as_deref(),
            Some("conflict")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    }

    #[tokio::test]
    async fn an_orphan_sidecar_is_replaced_only_when_konnect_wrote_it() {
        let dir = project();
        std::fs::create_dir(dir.path().join(MODELS_DIR)).unwrap();
        let sidecar = dir
            .path()
            .join(MODELS_DIR)
            .join(format!("fuse_fit{SIDECAR_SUFFIX}"));
        std::fs::write(&sidecar, "{\"notes\": \"mine\"}\n").unwrap();
        let refused = create(create_args(dir.path(), "fuse_fit")).await;
        assert_eq!(extract_error_kind(&refused).as_deref(), Some("conflict"));
        // One an interrupted write left gets past the plan: only the missing
        // engine stops this create, and the sidecar is untouched.
        let orphan = format!("{{\"format\": \"{SIDECAR_FORMAT}\"}}\n");
        std::fs::write(&sidecar, &orphan).unwrap();
        let stopped = create(create_args(dir.path(), "fuse_fit")).await;
        assert_eq!(
            body(&stopped)["error"]["kind"],
            "simulation_failed",
            "{:?}",
            body(&stopped)
        );
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), orphan);
    }

    #[test]
    fn a_failed_write_is_read_back_file_by_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let old = root.join("old.lib");
        let new = root.join("new.lib");
        std::fs::write(&old, "before").unwrap();
        let targets: Vec<Target<'_>> = vec![
            (old.as_path(), Some("before"), "after"),
            (new.as_path(), None, "fresh"),
        ];
        let left = left_after_error(&root, &targets);
        assert!(matches!(left, Left::Nothing), "{left:?}");
        // Another writer's change is no write of this call's.
        std::fs::write(&old, "theirs").unwrap();
        let left = left_after_error(&root, &targets);
        assert!(
            matches!(&left, Left::Changed(files) if *files == [old.display().to_string()]),
            "{left:?}"
        );
        std::fs::remove_file(&old).unwrap();
        let left = left_after_error(&root, &targets);
        assert!(matches!(left, Left::Changed(_)), "{left:?}");
        // This call's text on disk means it may have written it.
        std::fs::write(&new, "fresh").unwrap();
        let left = left_after_error(&root, &targets);
        assert!(
            matches!(&left, Left::Maybe { files, journal: false } if *files == [new.display().to_string()]),
            "{left:?}"
        );
    }

    #[test]
    fn a_mismatch_after_a_commit_is_uncertain_and_one_without_is_a_change() {
        let sheet_a = Path::new("/p/a.kicad_sch");
        let sheet_b = Path::new("/p/b.kicad_sch");
        assert_eq!(
            after_mismatch(&[], sheet_a),
            Mismatch::Changed(vec!["/p/a.kicad_sch".to_string()])
        );
        // The commit wrote b, so the answer covers b as well as a.
        let pending: Vec<Target<'_>> = vec![(sheet_b, Some("old"), "new")];
        assert_eq!(
            after_mismatch(&pending, sheet_a),
            Mismatch::Uncertain(vec![
                "/p/b.kicad_sch".to_string(),
                "/p/a.kicad_sch".to_string()
            ])
        );
        assert_eq!(
            after_mismatch(&pending, sheet_b),
            Mismatch::Uncertain(vec!["/p/b.kicad_sch".to_string()])
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_journal_is_matched_under_its_folders_own_spelling() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let folder = root.join("Simulation_Models");
        std::fs::create_dir(&folder).unwrap();
        let spelled = root.join(MODELS_DIR);
        if !spelled.is_dir() {
            // A case-sensitive disk has no second spelling of one folder.
            return;
        }
        let library = spelled.join("x_fit.lib");
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        let committed = commit_file_transaction(
            &root,
            vec![FileTransition::create(&library, "fresh".to_string())],
        );
        let left = left_after_error(&root, &[(library.as_path(), None, "fresh")]);
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        if committed.is_ok() {
            return;
        }
        assert!(
            matches!(left, Left::Maybe { journal: true, .. }),
            "{left:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_journal_left_by_a_write_that_failed_first_is_uncertain() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let models = root.join(MODELS_DIR);
        std::fs::create_dir(&models).unwrap();
        let library = models.join("x_fit.lib");
        // The journal goes in the project folder, and the first file fails in
        // a folder that cannot be written, before anything is renamed.
        std::fs::set_permissions(&models, std::fs::Permissions::from_mode(0o555)).unwrap();
        let committed = commit_file_transaction(
            &root,
            vec![FileTransition::create(&library, "fresh".to_string())],
        );
        let left = left_after_error(&root, &[(library.as_path(), None, "fresh")]);
        std::fs::set_permissions(&models, std::fs::Permissions::from_mode(0o755)).unwrap();
        if committed.is_ok() {
            // A user who may write anywhere, such as root, cannot see this.
            return;
        }
        assert!(!library.exists());
        assert!(
            matches!(left, Left::Maybe { journal: true, .. }),
            "{left:?}"
        );
    }

    #[tokio::test]
    async fn assign_refuses_a_library_konnect_did_not_write() {
        let dir = project();
        std::fs::create_dir(dir.path().join(MODELS_DIR)).unwrap();
        std::fs::write(
            dir.path().join(MODELS_DIR).join("vendor_fit.lib"),
            "*#shell echo KMARK\n.subckt vendor_fit 1 2\nR1 1 2 1\n.ends\n",
        )
        .unwrap();
        let refused = assign(assign_args(
            dir.path(),
            "simulation_models/vendor_fit.lib",
            "vendor_fit",
            json!({ "1": "1", "2": "2" }),
        ))
        .await;
        assert_eq!(field(&refused), "library_path", "{:?}", body(&refused));
        for outside in [
            "../x_fit.lib",
            "/etc/x_fit.lib",
            "${HOME}/x_fit.lib",
            "simulation_models/../top.kicad_sch",
        ] {
            let refused = assign(assign_args(
                dir.path(),
                outside,
                "x_fit",
                json!({ "1": "1" }),
            ))
            .await;
            assert_eq!(field(&refused), "library_path", "{outside}");
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("top.kicad_sch")).unwrap(),
            SHEET
        );
    }

    #[tokio::test]
    async fn assign_refuses_a_pin_map_that_does_not_fit_the_model_or_the_symbol() {
        let dir = project();
        let library = fitted_model(dir.path(), "fuse_fit");
        let cases = [
            (json!({ "1": "1" }), "pin_map"),
            (json!({ "1": "1", "2": "1" }), "pin_map"),
            (json!({ "1": "1", "2": "K" }), "pin_map.2"),
            (json!({ "1": "1", "3": "2" }), "pin_map.3"),
            (json!({ "1 2": "1", "2": "2" }), "pin_map.1 2"),
        ];
        for (pin_map, expected) in cases {
            let refused = assign(assign_args(
                dir.path(),
                &library,
                "fuse_fit",
                pin_map.clone(),
            ))
            .await;
            assert_eq!(field(&refused), expected, "{pin_map}: {:?}", body(&refused));
        }
        let mut wrong_name = assign_args(
            dir.path(),
            &library,
            "other_fit",
            json!({ "1": "1", "2": "2" }),
        );
        assert_eq!(field(&assign(wrong_name.clone()).await), "model_name");
        wrong_name["model_name"] = json!("fuse_fit");
        wrong_name["references"] = json!(["R99"]);
        assert_eq!(field(&assign(wrong_name).await), "references");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("top.kicad_sch")).unwrap(),
            SHEET
        );
    }

    #[tokio::test]
    async fn assign_refuses_while_kicad_has_the_project_open() {
        let dir = project();
        let library = fitted_model(dir.path(), "fuse_fit");
        std::fs::write(dir.path().join("~top.kicad_sch.lck"), "").unwrap();
        let refused = assign(assign_args(
            dir.path(),
            &library,
            "fuse_fit",
            json!({ "1": "1", "2": "2" }),
        ))
        .await;
        assert_eq!(
            extract_error_kind(&refused).as_deref(),
            Some("conflict"),
            "{:?}",
            body(&refused)
        );
        assert_eq!(body(&refused)["outcome"]["status"], "failed");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("top.kicad_sch")).unwrap(),
            SHEET
        );
    }

    #[test]
    fn a_pin_map_that_reverses_a_named_diode_is_a_contradiction() {
        let ports = vec!["A".to_string(), "K".to_string()];
        let pins = vec![
            ("1".to_string(), Some("K".to_string())),
            ("2".to_string(), Some("A".to_string())),
        ];
        let right = vec![
            ("1".to_string(), "K".to_string()),
            ("2".to_string(), "A".to_string()),
        ];
        let reversed = vec![
            ("1".to_string(), "A".to_string()),
            ("2".to_string(), "K".to_string()),
        ];
        assert!(contradiction(&pins, &right, &ports).is_none());
        assert!(contradiction(&pins, &reversed, &ports).is_some());
        let two_way = vec![
            ("1".to_string(), Some("A1".to_string())),
            ("2".to_string(), Some("A2".to_string())),
        ];
        assert!(contradiction(&two_way, &right, &ports).is_some());
        let unnamed = vec![("1".to_string(), None), ("2".to_string(), None)];
        assert!(contradiction(&unnamed, &reversed, &ports).is_none());
    }
}
