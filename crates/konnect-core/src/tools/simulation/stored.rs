//! Stored runs: list them, and measure one again without simulating.
//!
//! A run keeps its raw file and a small record (`run.json`) of what `run_simulation`
//! knew about it. Nothing here runs the engine or the exporter.

use super::{
    check_root_schematic, hash_sources, inputs_hash, invalid, simulation_failure, Analysis,
    Evaluation, Wanted, SOURCE,
};
use crate::mcp::protocol::CallToolResult;
use crate::outcome::{self, OutcomeStatus};
use crate::simulation::protocol::{RunStatus, LOG_FILE, RAW_FILE, STATUS_FILE};
use crate::simulation::runs::{self, LocateError, RecordError, RunRecord, RunState};
use crate::simulation::signals::{Names, Signal};
use crate::simulation::snapshot;
use crate::simulation::worker_client::read_results;
use crate::tools::ToolContext;
use serde_json::{json, Value};
use sha2::Sha256;
use std::path::{Path, PathBuf};

/// The most a model file may weigh for its hash to be taken again. A run's own
/// model files add up to no more than the deck inlines, which is this much.
const MAX_MODEL_BYTES: u64 = 64 * 1024 * 1024;

/// Most result rows `list_simulation_runs` returns, as its schema says.
pub(super) const MAX_LISTED_RUNS: u64 = 50;
pub(super) const DEFAULT_LISTED_RUNS: u64 = 10;

fn state_root(ctx: &ToolContext) -> Result<PathBuf, CallToolResult> {
    let root = match ctx.simulation.get() {
        Some(runtime) => runtime.state_root(),
        // Reading needs no worker, so a process without one can still list.
        None => konnect_sexp::writer::state_root(),
    };
    root.map_err(|error| {
        simulation_failure(
            "run",
            "run_store_failed",
            None,
            Vec::new(),
            format!("Failed to find the run store: {error}"),
        )
    })
}

/// The saved sheets as they are now, hashed the way `run_simulation` hashes them
/// before it exports. None if they cannot be copied, such as a schematic that is
/// gone.
pub(super) struct CurrentSources(pub(super) Option<Sha256>);

impl CurrentSources {
    pub(super) fn of(schematic: &Path) -> Self {
        let hash = (|| {
            // Named for the file as the disk spells it, as `run_simulation` does.
            let schematic = schematic.canonicalize().ok()?;
            let scratch = tempfile::Builder::new()
                .prefix("konnect-sim-check-")
                .tempdir()
                .ok()?;
            let source = scratch.path().join("src");
            std::fs::create_dir(&source).ok()?;
            snapshot::take_snapshot(&schematic, &source).ok()?;
            hash_sources(&source).ok()
        })();
        Self(hash)
    }

    /// Whether the run's inputs differ from what is saved now. A model file the
    /// run inlined that has gone counts as a change. Null when the sheets cannot
    /// be read, because nothing can then be said.
    fn changed_since(&self, record: &RunRecord) -> Option<bool> {
        let hasher = self.0.clone()?;
        // The record names these files, and a store can be written by anyone who
        // can write the state folder, so only a plain file of a sane size is read.
        let readable = |path: &PathBuf| {
            std::fs::metadata(path)
                .is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_MODEL_BYTES)
        };
        if !record.inlined_files.iter().all(readable) {
            return Some(true);
        }
        Some(match inputs_hash(hasher, &record.inlined_files) {
            Ok(now) => now != record.inputs_hash,
            Err(_) => true,
        })
    }
}

fn run_not_found(run_id: &str, detail: &str) -> CallToolResult {
    simulation_failure(
        "run",
        "run_not_found",
        None,
        vec![run_id.to_string()],
        format!(
            "No stored run has that id: {detail}. list_simulation_runs shows the runs of a \
             schematic. Only the newest 20 of each project are kept."
        ),
    )
}

fn unreadable(dir: &Path, detail: impl Into<String>) -> CallToolResult {
    simulation_failure(
        "results",
        "results_unreadable",
        Some(dir.join(LOG_FILE).display().to_string()),
        Vec::new(),
        format!("The stored run cannot be read: {}.", detail.into()),
    )
}

/// A run that another server prunes while this one reads it loses its files one by
/// one, and what went last is not what to report: if the run is gone, say so.
fn or_not_found(
    root: &Path,
    run_id: &str,
    otherwise: &dyn Fn() -> CallToolResult,
) -> CallToolResult {
    match runs::locate(root, run_id) {
        Ok(_) => otherwise(),
        Err(_) => run_not_found(run_id, "there is no such run"),
    }
}

pub(super) fn list(
    ctx: &ToolContext,
    schematic: &Path,
    limit: usize,
) -> Result<CallToolResult, CallToolResult> {
    check_root_schematic(schematic)?;
    let root = state_root(ctx)?;
    let ids = runs::list(&root, schematic).map_err(|error| {
        simulation_failure(
            "run",
            "run_store_failed",
            None,
            Vec::new(),
            format!("Failed to read the run store: {error}"),
        )
    })?;
    // A folder with no usable record is a run that failed, or one made before
    // records were kept. It has no data to offer, so it is counted and not listed.
    // One record is held at a time, whatever the store holds.
    let sources = std::cell::OnceCell::new();
    let mut total = 0usize;
    let mut without_record = 0usize;
    let mut unreadable_record = 0usize;
    let mut shown: Vec<Value> = Vec::new();
    for id in &ids {
        let record = runs::locate(&root, id).map(|dir| runs::read_record(&dir));
        let record = match record {
            Ok(Ok(record)) => record,
            // The record exists and cannot be used: damaged, too big, or written by
            // another version. The run is there, so it is not "without a record".
            Ok(Err(RecordError::Unusable(_))) => {
                unreadable_record += 1;
                continue;
            }
            Ok(Err(RecordError::Missing)) | Err(_) => {
                without_record += 1;
                continue;
            }
        };
        total += 1;
        if shown.len() < limit {
            let sources = sources.get_or_init(|| CurrentSources::of(schematic));
            shown.push(json!({
                "run_id": id,
                "created_at": runs::created_at(id),
                "analysis_kind": record.analysis_kind(),
                "run_status": record.run_status.as_str(),
                "simulated_circuit": record.simulated_circuit,
                "coverage_ratio": record.coverage_ratio,
                "inputs_changed_since_run": sources.changed_since(&record),
            }));
        }
    }
    Ok(CallToolResult::json(&json!({
        "total": total,
        "shown": shown.len(),
        "truncated": shown.len() < total,
        "without_record": without_record,
        "unreadable_record": unreadable_record,
        "runs": shown,
    })))
}

/// A refusal of `measure_simulation_run` carries the shared envelope too, counting
/// the measurements that were asked for.
pub(super) fn failed_measure_outcome(
    result: CallToolResult,
    target: &str,
    requested: usize,
) -> CallToolResult {
    let outcome = outcome::summary(
        OutcomeStatus::Failed,
        target,
        SOURCE,
        requested,
        0,
        requested,
        Some(outcome::retry_whole_request()),
    );
    outcome::attach(result, outcome)
}

/// What a request asks of a stored run: the id and the measurements, checked
/// before the store is touched.
pub(super) struct Question<'a> {
    pub run_id: &'a str,
    pub signals: &'a Value,
    pub measurements: &'a Value,
    pub envelope_points: usize,
}

impl Question<'_> {
    /// The id is two path components once it names a folder, so it must be
    /// exactly an id. The schema says so too, but a client need not honour it.
    pub(super) fn check(&self) -> Result<(), CallToolResult> {
        if !runs::is_valid_id(self.run_id) {
            return Err(invalid(
                "run_id",
                "run_id must be an id that run_simulation returned, such as \
                 3fa9c1d27b04-20261030T133512-7f3a",
            ));
        }
        let count = self.measurements.as_array().map_or(0, Vec::len);
        if count == 0 {
            return Err(invalid(
                "measurements",
                "measure_simulation_run needs at least one measurement",
            ));
        }
        Ok(())
    }

    pub(super) fn requested(&self) -> usize {
        self.measurements.as_array().map_or(0, Vec::len)
    }
}

pub(super) fn measure(
    ctx: &ToolContext,
    question: &Question<'_>,
) -> Result<CallToolResult, (CallToolResult, String)> {
    // A client's text is not repeated unless it is an id.
    let target = match runs::is_valid_id(question.run_id) {
        true => question.run_id.to_string(),
        false => "(not a run id)".to_string(),
    };
    let refuse = |result: CallToolResult| (result, target.clone());
    question.check().map_err(refuse)?;
    let root = state_root(ctx).map_err(refuse)?;
    // A run that another server prunes while this one reads it loses its files one
    // by one, and what was lost last is not what to report.
    let lost =
        |otherwise: &dyn Fn() -> CallToolResult| or_not_found(&root, question.run_id, otherwise);
    let dir = runs::locate(&root, question.run_id).map_err(|error| {
        refuse(match error {
            LocateError::InvalidId => invalid("run_id", "run_id is not a run id"),
            LocateError::NotFound => run_not_found(question.run_id, "there is no such run"),
        })
    })?;
    let record = runs::read_record(&dir).map_err(|error| {
        refuse(match error {
            RecordError::Missing => lost(&|| {
                run_not_found(
                    question.run_id,
                    "the run has no record: it failed, was made before records were kept, or \
                     its record could not be saved",
                )
            }),
            RecordError::Unusable(detail) => {
                lost(&|| unreadable(&dir, format!("its record: {detail}")))
            }
        })
    })?;
    // From here the schematic is known, and names the work in the outcome.
    let target = record.schematic.display().to_string();
    let refuse = |result: CallToolResult| (result, target.clone());
    let analysis = Analysis::from_record(&record.analysis)
        .ok_or_else(|| refuse(unreadable(&dir, "its record holds no analysis")))?;
    // The run's own message says "signals but no measurements", and a stored call
    // needs a measurement, so it would send the caller round in a circle.
    if matches!(&analysis, Analysis::DcSweep(sweep) if sweep.sweeps.len() > 1) {
        return Err(refuse(invalid(
            "measurements",
            "A dc_sweep with two sweeps takes no measurements, live or stored: its whole \
             grid is in the run's results.csv, with both sweeps' values.",
        )));
    }
    let wanted = Wanted::for_analysis(
        &analysis,
        question.signals,
        question.measurements,
        question.envelope_points,
    )
    .map_err(refuse)?;
    let status: RunStatus = runs::read_plain(&dir.join(STATUS_FILE), runs::MAX_STATUS_BYTES)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| {
            refuse(lost(&|| {
                unreadable(&dir, "its status file is missing or damaged")
            }))
        })?;
    // The raw parser reads whatever it is given, so a link or a pipe is stopped here.
    if !std::fs::symlink_metadata(dir.join(RAW_FILE)).is_ok_and(|meta| meta.is_file()) {
        return Err(refuse(lost(&|| {
            unreadable(&dir, "its raw file is missing or not a plain file")
        })));
    }
    let plot = read_results(&dir, &status)
        .map_err(|error| refuse(lost(&|| unreadable(&dir, format!("its raw file: {error}")))))?;

    let partial = record.run_status == RunState::Partial;
    let evaluation = Evaluation {
        names: Names {
            nets: &record.nets,
            elements: &record.elements,
        },
        wanted: &wanted,
    };
    let gathered = match &analysis {
        Analysis::Transient(_) => evaluation.transient(&plot, partial),
        Analysis::Ac(ac) => evaluation.ac(ac, &plot, partial),
        Analysis::DcSweep(sweep) => evaluation.sweep(sweep, &plot, partial),
        Analysis::OperatingPoint => {
            return Err(refuse(invalid(
                "measurements",
                "An operating point has no sweep to measure",
            )))
        }
    };
    let failed = gathered.failed_measurements;
    let signal_missing = gathered.signal_missing;

    let mut body = json!({
        "run_id": question.run_id,
        "analysis_kind": analysis.kind(),
        "analysis": record.result,
        "run_status": record.run_status.as_str(),
        "simulated_circuit": record.simulated_circuit,
        "circuit": record.circuit,
        "coverage_ratio": record.coverage_ratio,
        "inputs_changed_since_run": CurrentSources::of(&record.schematic).changed_since(&record),
        // What the run itself reported comes first: a peak that reads low there
        // reads low here.
        "warnings": record.warnings,
    });
    if let Some(stopped) = &record.stopped {
        body["stopped"] = json!(stopped);
    }
    evaluation.finish_body(&mut body, gathered, partial);
    let unsaved_power: Vec<String> = wanted
        .signals
        .iter()
        .map(|(text, signal)| (text, signal))
        .chain(wanted.measurements.iter().flat_map(|request| {
            let reference = request.reference_signal.iter().map(|(text, s)| (text, s));
            std::iter::once((&request.signal_text, &request.signal)).chain(reference)
        }))
        .filter(|(_, signal)| matches!(signal, Signal::Power(_)))
        .filter(|(_, signal)| {
            crate::simulation::signals::resolve(&plot, signal, evaluation.names).is_err()
        })
        .map(|(text, _)| text.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if let (false, Some(warnings)) = (unsaved_power.is_empty(), body["warnings"].as_array_mut()) {
        warnings.push(json!({
            "code": "power_not_saved",
            "detail": "This run holds no data for these. A run saves p(REF) only if it asked \
                       for it in signals or measurements, and never for a code model or a \
                       coupling. Run again with it, if the part is named right and can have one.",
            "items": unsaved_power,
        }));
    }

    // One unit for each measurement, and one for the summaries if any were asked
    // for, as `run_simulation` lets its range unit stand for them.
    let summaries = usize::from(!wanted.signals.is_empty());
    let requested = question.requested() + summaries;
    let failed = failed + summaries * usize::from(signal_missing);
    let status = match partial || failed > 0 {
        true => OutcomeStatus::Partial,
        false => OutcomeStatus::Complete,
    };
    let outcome = outcome::summary(
        status,
        &target,
        SOURCE,
        requested,
        requested - failed,
        failed,
        None,
    );
    Ok(outcome::attach(CallToolResult::json(&body), outcome))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "3fa9c1d27b04-20261030T133512-7f3a";

    fn refusal(result: &CallToolResult) -> (String, String) {
        let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
            panic!("expected text")
        };
        let body: Value = serde_json::from_str(text).unwrap();
        (
            body["error"]["kind"].as_str().unwrap().to_string(),
            body["error"]["field"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        )
    }

    #[test]
    fn the_question_checks_the_id_and_the_count_on_its_own() {
        let measurements = json!([{ "kind": "max", "signal": "v(a)" }]);
        let ask = |run_id: &'static str, measurements: &Value| {
            let question = Question {
                run_id,
                signals: &Value::Null,
                measurements,
                envelope_points: 0,
            };
            question.check().map_err(|result| refusal(&result))
        };
        assert!(ask(ID, &measurements).is_ok());
        for bad in ["../../x", "", "..", "3fa9c1d27b04-20261030T133512-7f3a/x"] {
            assert_eq!(
                ask(bad, &measurements),
                Err(("invalid_argument".to_string(), "run_id".to_string())),
                "{bad:?}"
            );
        }
        for empty in [json!([]), Value::Null, json!("max")] {
            assert_eq!(
                ask(ID, &empty),
                Err(("invalid_argument".to_string(), "measurements".to_string())),
                "{empty}"
            );
        }
    }

    #[test]
    fn a_change_is_only_reported_when_the_sheets_can_be_read() {
        let record = |hash: &str| {
            RunRecord::new(runs::RecordFields {
                schematic: PathBuf::new(),
                analysis: json!({ "kind": "operating_point" }),
                run_status: RunState::Complete,
                stopped: None,
                simulated_circuit: String::new(),
                circuit: Value::Null,
                result: Value::Null,
                coverage_ratio: 1.0,
                inputs_hash: hash.to_string(),
                inlined_files: Vec::new(),
                nets: Vec::new(),
                elements: Vec::new(),
                warnings: Vec::new(),
            })
        };
        let unreadable = CurrentSources(None);
        assert_eq!(unreadable.changed_since(&record("x")), None);
        let hasher = Sha256::default();
        let same = inputs_hash(hasher.clone(), &[]).unwrap();
        let sources = CurrentSources(Some(hasher));
        assert_eq!(sources.changed_since(&record(&same)), Some(false));
        assert_eq!(sources.changed_since(&record("other")), Some(true));
        let mut gone = record(&same);
        gone.inlined_files = vec![PathBuf::from("/nonexistent/model.lib")];
        assert_eq!(sources.changed_since(&gone), Some(true));
    }

    #[test]
    fn a_model_file_that_is_not_a_plain_file_is_a_change_and_is_never_read() {
        let dir = tempfile::tempdir().unwrap();
        let record = |files: Vec<PathBuf>| {
            RunRecord::new(runs::RecordFields {
                schematic: PathBuf::new(),
                analysis: json!({ "kind": "operating_point" }),
                run_status: RunState::Complete,
                stopped: None,
                simulated_circuit: String::new(),
                circuit: Value::Null,
                result: Value::Null,
                coverage_ratio: 1.0,
                inputs_hash: inputs_hash(Sha256::default(), &[]).unwrap(),
                inlined_files: files,
                nets: Vec::new(),
                elements: Vec::new(),
                warnings: Vec::new(),
            })
        };
        let sources = CurrentSources(Some(Sha256::default()));
        assert_eq!(sources.changed_since(&record(Vec::new())), Some(false));
        // A folder, a pipe (which would block a read for ever), and a file past the limit.
        assert_eq!(
            sources.changed_since(&record(vec![dir.path().to_path_buf()])),
            Some(true)
        );
        #[cfg(unix)]
        {
            let fifo = dir.path().join("pipe");
            let made = std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap();
            assert!(made.success());
            // A writer, so that a read would return and fail the test, not hang it.
            let writer = fifo.clone();
            std::thread::spawn(move || {
                let _ = std::fs::OpenOptions::new().write(true).open(writer);
            });
            assert_eq!(sources.changed_since(&record(vec![fifo])), Some(true));
        }
        let big = dir.path().join("big.lib");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(MAX_MODEL_BYTES + 1)
            .unwrap();
        assert_eq!(sources.changed_since(&record(vec![big])), Some(true));
    }

    #[test]
    fn a_run_pruned_while_it_is_read_is_not_found_and_not_unreadable() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("simulation/3fa9c1d27b04").join(ID);
        std::fs::create_dir_all(&dir).unwrap();
        let reason = |result: &CallToolResult| {
            let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
                panic!("expected text")
            };
            let body: Value = serde_json::from_str(text).unwrap();
            body["error"]["reason"].as_str().unwrap().to_string()
        };
        let otherwise = || unreadable(&dir, "its raw file");
        assert_eq!(
            reason(&or_not_found(root.path(), ID, &otherwise)),
            "results_unreadable"
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            reason(&or_not_found(root.path(), ID, &otherwise)),
            "run_not_found"
        );
    }

    #[test]
    fn a_model_file_past_the_size_limit_is_not_read_even_if_it_would_match() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.lib");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(MAX_MODEL_BYTES + 1)
            .unwrap();
        let hasher = Sha256::default();
        // The hash the run would have recorded, so reading the file reads unchanged.
        let recorded = inputs_hash(hasher.clone(), std::slice::from_ref(&big)).unwrap();
        let record = RunRecord::new(runs::RecordFields {
            schematic: PathBuf::new(),
            analysis: json!({ "kind": "operating_point" }),
            run_status: RunState::Complete,
            stopped: None,
            simulated_circuit: String::new(),
            circuit: Value::Null,
            result: Value::Null,
            coverage_ratio: 1.0,
            inputs_hash: recorded,
            inlined_files: vec![big.clone()],
            nets: Vec::new(),
            elements: Vec::new(),
            warnings: Vec::new(),
        });
        let sources = CurrentSources(Some(hasher));
        assert_eq!(sources.changed_since(&record), Some(true));
        // At the limit, the same file is read and matches.
        std::fs::File::options()
            .write(true)
            .open(&big)
            .unwrap()
            .set_len(MAX_MODEL_BYTES)
            .unwrap();
        let mut record = record;
        record.inputs_hash = inputs_hash(Sha256::default(), std::slice::from_ref(&big)).unwrap();
        assert_eq!(sources.changed_since(&record), Some(false));
    }
}
