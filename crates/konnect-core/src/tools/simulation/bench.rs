//! Run a fitted model's bench on the engine.
//!
//! The model is staged in a private folder, which is the only folder the
//! bench may include from, so the vetting also proves the model reaches for
//! nothing else. Every bench card, the `.temp` line included, goes through the
//! flattener as netlist text; after it only the analysis line follows. A run
//! leaves no record: its folder is temporary, and nothing reaches the project.

use super::sweeps::Ac;
use super::{
    check_root_schematic, engine_complaints, prepare_run, simulation_failure, usable_engine,
    Analysis,
};
use crate::mcp::protocol::CallToolResult;
use crate::simulation::flatten::{self, Compatibility, DeckInputs, FlatDeck};
use crate::simulation::models::{bench as model_bench, text, Model};
use crate::simulation::protocol::{RunRequest, LOG_FILE, PROTOCOL_VERSION};
use crate::simulation::raw::Plot;
use crate::simulation::worker_client::{
    classify, read_results, run_worker, EngineStatus, Verdict, WorkerEnd, PROBE_TIMEOUT,
};
use crate::tools::ToolContext;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const BENCH_TITLE: &str = ".title konnect model bench";
const BENCH_RAW_BYTES: u64 = 16 * 1024 * 1024;
/// An AC bench reads its first point, at the frequency; the sweep ends a
/// little above it because a sweep needs room.
const AC_SPAN: f64 = 1.001;
const MAX_COMPLAINTS: usize = 8;
/// A transient bench takes at most this many steps, so a charge is read to a
/// part in a thousand of the run.
const TRAN_STEPS: f64 = 2000.0;
/// The engine's usual tolerances stop a breakdown solution up to 30 mV short
/// at 30 V. A bench measures, so it solves to a part in a million.
const BENCH_OPTIONS: &str = ".options reltol=1e-6 vntol=1e-9 abstol=1e-14";

/// The project a fitted model belongs to.
pub(crate) struct Project {
    /// Canonical: where `simulation_models/` goes, and what `Sim.Library`
    /// paths are relative to.
    pub dir: PathBuf,
    pub compatibility: Compatibility,
}

/// The root sheet's project, refused when it is not one Konnect may read
/// models from, such as a project that holds the home folder.
pub(crate) fn project_of(schematic: &Path) -> Result<Project, CallToolResult> {
    check_root_schematic(schematic)?;
    let canonical = schematic.canonicalize().map_err(|error| {
        simulation_failure(
            "project",
            "project_unreadable",
            None,
            Vec::new(),
            format!("Cannot resolve {}: {error}", schematic.display()),
        )
    })?;
    let dir = canonical
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let (roots, _) = super::allowed_roots(&dir, &[]);
    if !roots.contains(&dir) {
        return Err(simulation_failure(
            "project",
            "project_not_allowed",
            None,
            Vec::new(),
            "The project folder holds the home folder, so models in it cannot be read. Move the \
             project into a folder of its own.",
        ));
    }
    Ok(Project {
        compatibility: Compatibility::from_project_file(&canonical.with_extension("kicad_pro")),
        dir,
    })
}

/// What the bench measured: one value per check of the model, in order, and
/// none where the run did not hold the signal.
pub(crate) struct Measured {
    pub engine_version: Option<String>,
    pub values: Vec<Option<f64>>,
}

/// Measure every check of `model` at `temperature` on the engine.
pub(crate) async fn measure(
    ctx: &ToolContext,
    project: &Project,
    model: &Model,
    temperature: f64,
    budget: Duration,
) -> Result<Measured, CallToolResult> {
    let started = Instant::now();
    let Some(runtime) = ctx.simulation.get() else {
        return Err(simulation_failure(
            "engine",
            "engine_load_failed",
            None,
            Vec::new(),
            "This process does not carry the simulation worker, so the model cannot be checked.",
        ));
    };
    let engine = usable_engine(runtime, &ctx.config.kicad_cli, budget.min(PROBE_TIMEOUT)).await?;
    let failure = |reason: &'static str, message: String| {
        simulation_failure("bench", reason, None, Vec::new(), message)
    };
    let staging = tempfile::Builder::new()
        .prefix("konnect-model-")
        .tempdir()
        .and_then(|dir| dir.path().canonicalize().map(|path| (dir, path)))
        .map_err(|error| failure("bench_store_failed", format!("No staging folder: {error}")))?;
    let (_staging_guard, staging_dir) = staging;
    let file = staging_dir.join(format!("{}.lib", model.name));
    std::fs::write(&file, model.file()).map_err(|error| {
        failure(
            "bench_store_failed",
            format!("Cannot stage the model: {error}"),
        )
    })?;

    let checks = &model.checks;
    let mut operating_point = None;
    if model_bench::needs_operating_point(checks) {
        let plot = run(
            runtime,
            &engine,
            &staging_dir,
            &file,
            &model.bench.dc,
            temperature,
            &Analysis::OperatingPoint,
            project.compatibility,
            budget.saturating_sub(started.elapsed()),
        )
        .await?;
        operating_point = Some(plot);
    }
    let mut ac_plots: Vec<(f64, Plot)> = Vec::new();
    for hz in model_bench::ac_frequencies(checks) {
        let analysis = Analysis::Ac(Ac::from_args(&json!({
            "kind": "ac",
            "start_hz": hz,
            "stop_hz": hz * AC_SPAN,
            "scale": "linear",
            "points": 3,
        }))?);
        let plot = run(
            runtime,
            &engine,
            &staging_dir,
            &file,
            &model.bench.ac,
            temperature,
            &analysis,
            project.compatibility,
            budget.saturating_sub(started.elapsed()),
        )
        .await?;
        ac_plots.push((hz, plot));
    }
    let mut transient = None;
    if model_bench::needs_transient(checks) {
        let stop = model.bench.tran_stop;
        let analysis = Analysis::from_args(&json!({
            "kind": "transient",
            "stop_time_seconds": stop,
            "max_step_seconds": stop / TRAN_STEPS,
        }))?;
        let plot = run(
            runtime,
            &engine,
            &staging_dir,
            &file,
            &model.bench.tran,
            temperature,
            &analysis,
            project.compatibility,
            budget.saturating_sub(started.elapsed()),
        )
        .await?;
        transient = Some(plot);
    }
    let values = checks
        .iter()
        .map(|check| {
            model_bench::measure(
                check,
                operating_point.as_ref(),
                &ac_plots,
                transient.as_ref(),
            )
        })
        .collect();
    Ok(Measured {
        engine_version: engine.version.clone(),
        values,
    })
}

/// The bench deck for one analysis, vetted with the staging folder as its
/// only include root.
fn bench_deck(
    staging_dir: &Path,
    file: &Path,
    fixtures: &[String],
    temperature: f64,
    compatibility: Compatibility,
) -> Result<FlatDeck, CallToolResult> {
    let mut netlist = format!(
        "{BENCH_TITLE}\n{BENCH_OPTIONS}\n.include \"{}\"\n.temp {}\n",
        file.display(),
        text::number(temperature)
    );
    for line in fixtures {
        netlist.push_str(line);
        netlist.push('\n');
    }
    let roots = [staging_dir.to_path_buf()];
    flatten::flatten(&DeckInputs {
        netlist: &netlist,
        testbench: &[],
        project_dir: staging_dir,
        allowed_roots: &roots,
        compatibility,
        excluded_elements: &[],
    })
    .map_err(|refusal| {
        simulation_failure(
            "bench",
            "model_refused",
            None,
            vec![refusal.detail.clone()],
            format!(
                "The fitted model failed Konnect's own vetting ({}), so it was not written.",
                refusal.code.as_str()
            ),
        )
    })
}

#[allow(clippy::too_many_arguments)]
async fn run(
    runtime: &crate::simulation::SimulationRuntime,
    engine: &EngineStatus,
    staging_dir: &Path,
    file: &Path,
    fixtures: &[String],
    temperature: f64,
    analysis: &Analysis,
    compatibility: Compatibility,
    budget: Duration,
) -> Result<Plot, CallToolResult> {
    if budget.is_zero() {
        return Err(simulation_failure(
            "bench",
            "timed_out",
            None,
            Vec::new(),
            "The time budget ran out before the model was checked. Raise timeout_seconds.",
        ));
    }
    let deck = bench_deck(staging_dir, file, fixtures, temperature, compatibility)?;
    let folder = tempfile::Builder::new()
        .prefix("konnect-bench-")
        .tempdir()
        .map_err(|error| {
            simulation_failure(
                "bench",
                "bench_store_failed",
                None,
                Vec::new(),
                error.to_string(),
            )
        })?;
    let library = PathBuf::from(engine.library_path.clone().unwrap_or_default());
    let request = RunRequest {
        protocol: PROTOCOL_VERSION,
        library: library.clone(),
        code_models: None,
        load_digital: false,
        compatibility: compatibility.ngbehavior.map(str::to_owned),
        input_path: None,
        budget_ms: budget.as_millis() as u64,
        max_raw_bytes: BENCH_RAW_BYTES,
        complex_data: analysis.makes_complex_data(),
    };
    // After the vetting come only Konnect's own lines: the analysis, which is
    // numbers formatted here.
    let own_lines = vec![analysis.line()];
    prepare_run(folder.path(), &deck, &own_lines, &request).map_err(|error| {
        simulation_failure(
            "bench",
            "bench_store_failed",
            None,
            Vec::new(),
            format!("{error:#}"),
        )
    })?;
    let end = run_worker(runtime, folder.path(), &library, budget)
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
    let results = match &end {
        WorkerEnd::Reported(status) => Some(read_results(folder.path(), status)),
        _ => None,
    };
    let verdict = classify(&end, results.as_ref(), |plot| analysis.covered(plot));
    let log = std::fs::read_to_string(folder.path().join(LOG_FILE)).unwrap_or_default();
    let (mut complaints, _) = engine_complaints(&log, &deck);
    complaints.truncate(MAX_COMPLAINTS);
    match (verdict, results) {
        (Verdict::Complete, Some(Ok(plot))) => Ok(plot),
        (Verdict::Complete, _) => Err(simulation_failure(
            "bench",
            "results_unreadable",
            None,
            complaints,
            "The bench ran but its results could not be read, so the model was not written.",
        )),
        (Verdict::Partial(reason) | Verdict::Failed(reason), _) => Err(simulation_failure(
            "bench",
            reason,
            None,
            complaints,
            format!("The bench did not finish ({reason}), so the model was not written."),
        )),
    }
}
