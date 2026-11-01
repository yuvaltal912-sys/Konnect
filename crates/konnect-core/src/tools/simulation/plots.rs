//! `plot_simulation_run`: a stored run's signals as an SVG file.
//!
//! The run is reached through `runs::locate` and its record through
//! `runs::read_record`, as `measure_simulation_run` does, and the data is read from
//! the raw file again. Nothing here runs the engine.
//!
//! What each analysis draws:
//!
//! - **Transient:** value against time, one panel for each unit (volts, then amps,
//!   then watts) so a current is not squeezed against a voltage.
//! - **AC:** a Bode plot, gain in dB over unwrapped phase in degrees, against
//!   frequency on a logarithmic axis whatever the sweep's own spacing. Phase from
//!   the first sample that turns too sharply to trust is noted, as margins refuse
//!   it.
//! - **DC sweep, one sweep:** value against the swept quantity, one panel for each
//!   unit. The plot's scale vector is read by position, since the engine calls it
//!   `v-sweep` and its kin whatever it varies, and the axis is named for the
//!   element the deck holds.
//! - **DC sweep, two sweeps:** one panel for each signal (four at most) holding a
//!   curve for each step of the second sweep against the first. The engine does not
//!   save the second sweep's value, so a step is found from the point number as
//!   `results.csv` finds it. At most eight steps are drawn, evenly spaced and
//!   always the first and last, coloured dark to light.
//! - **Operating point:** refused. It holds one number for each signal and nothing
//!   to draw them against; `results.csv` holds the table.

use super::stored::{self, CurrentSources, Opened};
use super::sweeps::DcSweep;
use super::{invalid, simulation_failure, Analysis, Wanted, SOURCE};
use crate::mcp::protocol::CallToolResult;
use crate::outcome::{self, OutcomeStatus};
use crate::simulation::ac;
use crate::simulation::plot::{
    self, Axis, Chart, Panel, Rendered, Series, CATEGORICAL, DEFAULT_WIDTH, MAX_WIDTH, MIN_WIDTH,
    RAMP,
};
use crate::simulation::raw::Plot;
use crate::simulation::runs::{self, RunRecord, RunState};
use crate::simulation::signals::{self, Names};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;

pub(super) const MAX_PLOT_SIGNALS: usize = 8;
/// Two sweeps draw a panel for each signal, so fewer.
const MAX_GRID_SIGNALS: usize = 4;
/// Steps of the second sweep drawn as curves.
const MAX_CURVES: usize = 8;
/// Plots kept in one run's folder. The run is pruned with the folder, and the name
/// comes from the request, so asking again for the same plot replaces it.
const KEEP_PLOTS: usize = 16;
/// A `.partial` file older than this is left by a crash, not by a call still writing.
const STALE_PARTIAL: std::time::Duration = std::time::Duration::from_secs(60);
const PLOT_PREFIX: &str = "plot-";
const PLOT_SUFFIX: &str = ".svg";

pub(super) struct Question<'a> {
    pub run_id: &'a str,
    pub signals: &'a Value,
    pub width_px: &'a Value,
}

impl Question<'_> {
    /// The id, the signal count and the width, checked before the store is touched.
    pub(super) fn check(&self) -> Result<u32, CallToolResult> {
        if !runs::is_valid_id(self.run_id) {
            return Err(invalid(
                "run_id",
                "run_id must be an id that run_simulation returned, such as \
                 3fa9c1d27b04-20261030T133512-7f3a",
            ));
        }
        let count = self.signals.as_array().map_or(0, Vec::len);
        if count == 0 || count > MAX_PLOT_SIGNALS {
            return Err(invalid(
                "signals",
                format!("plot_simulation_run needs from 1 to {MAX_PLOT_SIGNALS} signals"),
            ));
        }
        match self.width_px {
            Value::Null => Ok(DEFAULT_WIDTH),
            value => value
                .as_u64()
                .filter(|width| (u64::from(MIN_WIDTH)..=u64::from(MAX_WIDTH)).contains(width))
                .map(|width| width as u32)
                .ok_or_else(|| {
                    invalid(
                        "width_px",
                        format!("width_px must be a whole number from {MIN_WIDTH} to {MAX_WIDTH}"),
                    )
                }),
        }
    }
}

/// A refusal carries the shared envelope, counting the plot and the range proof.
pub(super) fn failed_outcome(result: CallToolResult, target: &str) -> CallToolResult {
    stored::failed_measure_outcome(result, target, 2)
}

/// One requested signal, found in the run.
struct Found {
    text: String,
    unit: &'static str,
    real: Vec<f64>,
    imaginary: Vec<f64>,
}

/// What an analysis makes of the signals: the axis, the panels and what to say.
struct Drawing {
    x: Axis,
    panels: Vec<Panel>,
    palette: &'static [&'static str],
    legend: Vec<(usize, String)>,
    notes: Vec<String>,
    warnings: Vec<Value>,
}

fn gone(opened: &Opened, detail: &str) -> CallToolResult {
    stored::unreadable(&opened.dir, detail.to_string())
}

/// Values and imaginary parts cut to the same length as the axis, so a variable
/// the engine saved shorter than the plot cannot pair with the wrong x.
pub(super) fn paired(axis: &[f64], values: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let n = axis.len().min(values.len());
    (axis[..n].to_vec(), values[..n].to_vec())
}

fn series(color: usize, axis: &[f64], values: &[f64]) -> Series {
    let (x, y) = paired(axis, values);
    Series { color, x, y }
}

fn legend_of(found: &[Found]) -> Vec<(usize, String)> {
    found
        .iter()
        .enumerate()
        .map(|(index, found)| (index, found.text.clone()))
        .collect()
}

/// One panel for each unit that has a signal: volts, amps, then watts.
fn by_unit(found: &[Found], axis: &[f64]) -> Vec<Panel> {
    [("V", "Voltage"), ("A", "Current"), ("W", "Power")]
        .into_iter()
        .filter_map(|(unit, label)| {
            let own: Vec<Series> = found
                .iter()
                .enumerate()
                .filter(|(_, found)| found.unit == unit)
                .map(|(index, found)| series(index, axis, &found.real))
                .collect();
            (!own.is_empty()).then(|| Panel {
                title: String::new(),
                y_label: label.to_string(),
                y_unit: unit.to_string(),
                y_si: true,
                series: own,
            })
        })
        .collect()
}

fn transient(opened: &Opened, plot: &Plot, found: &[Found]) -> Result<Drawing, CallToolResult> {
    let time: Vec<f64> = plot
        .variable("time")
        .map(|index| plot.real(index).collect())
        .ok_or_else(|| gone(opened, "its data has no time"))?;
    Ok(Drawing {
        x: Axis {
            label: "Time".into(),
            unit: "s".into(),
            log: false,
            si: true,
        },
        panels: by_unit(found, &time),
        palette: &CATEGORICAL,
        legend: legend_of(found),
        notes: Vec::new(),
        warnings: Vec::new(),
    })
}

fn bode(plot: &Plot, found: &[Found], ac: &super::sweeps::Ac) -> Result<Drawing, String> {
    let freq: Vec<f64> = plot
        .variable("frequency")
        .map(|index| plot.real(index).collect())
        .ok_or("its data has no frequency")?;
    let (mut gain, mut phase) = (Vec::new(), Vec::new());
    let mut doubtful: Option<f64> = None;
    for (index, found) in found.iter().enumerate() {
        let n = freq.len().min(found.real.len()).min(found.imaginary.len());
        let response = ac::Response::new(
            &freq[..n],
            &found.real[..n],
            &found.imaginary[..n],
            ac.scale.is_logarithmic(),
        );
        let Some(response) = response else {
            continue;
        };
        let (gain_db, degrees, from) = response.bode();
        gain.push(Series {
            color: index,
            x: freq[..n].to_vec(),
            y: gain_db.to_vec(),
        });
        phase.push(Series {
            color: index,
            x: freq[..n].to_vec(),
            y: degrees.to_vec(),
        });
        if let Some(from) = from {
            doubtful = Some(doubtful.map_or(freq[from], |at| at.min(freq[from])));
        }
    }
    let mut notes = vec![
        "Gain is 20 log10 of the magnitude, so it is the gain when the AC source is 1, and \
         dB re 1 V or 1 A otherwise. Phase is unwrapped from the first sample."
            .to_string(),
    ];
    let mut warnings = Vec::new();
    if let Some(at) = doubtful {
        notes.push(format!(
            "Phase from {} Hz on may be out by whole turns: a sample there turns too sharply \
             to follow. Run again at more points.",
            plot::si(at)
        ));
        warnings.push(json!({
            "code": "phase_unreliable",
            "detail": "The unwrapped phase may be out by whole turns from this frequency on.",
            "from_hz": at,
        }));
    }
    let panel = |label: &str, unit: &str, si: bool, series: Vec<Series>| Panel {
        title: String::new(),
        y_label: label.to_string(),
        y_unit: unit.to_string(),
        y_si: si,
        series,
    };
    Ok(Drawing {
        x: Axis {
            label: "Frequency".into(),
            unit: "Hz".into(),
            log: true,
            si: false,
        },
        panels: vec![
            panel("Gain", "dB", false, gain),
            panel("Phase", "°", false, phase),
        ],
        palette: &CATEGORICAL,
        legend: legend_of(found),
        notes,
        warnings,
    })
}

fn sweep_axis(sweep: &DcSweep, plot: &Plot) -> (Vec<f64>, Axis) {
    let inner = sweep.inner();
    let axis = DcSweep::axis(plot);
    let x = Axis {
        label: inner.name.clone(),
        unit: inner.unit.to_string(),
        log: false,
        // A temperature in °C takes no prefix.
        si: inner.unit != "°C",
    };
    (axis, x)
}

fn dc_one(sweep: &DcSweep, plot: &Plot, found: &[Found]) -> Drawing {
    let (axis, x) = sweep_axis(sweep, plot);
    Drawing {
        x,
        panels: by_unit(found, &axis),
        palette: &CATEGORICAL,
        legend: legend_of(found),
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// Which steps of the second sweep to draw, from `steps`: all of them up to
/// `MAX_CURVES`, and otherwise evenly spaced from the first to the last.
fn chosen_steps(steps: usize) -> Vec<usize> {
    if steps <= MAX_CURVES {
        return (0..steps).collect();
    }
    let last = MAX_CURVES - 1;
    (0..MAX_CURVES)
        .map(|i| (i as f64 * (steps - 1) as f64 / last as f64).round() as usize)
        .collect()
}

fn dc_grid(sweep: &DcSweep, plot: &Plot, found: &[Found]) -> Drawing {
    let (axis, x) = sweep_axis(sweep, plot);
    let outer = &sweep.sweeps[1];
    let run = sweep.run_length(&axis).max(1);
    let steps = axis.len().div_ceil(run);
    let chosen = chosen_steps(steps);
    let ramp_at = |position: usize| match chosen.len() {
        1 => 0,
        n => (position as f64 * (RAMP.len() - 1) as f64 / (n - 1) as f64).round() as usize,
    };
    let panels = found
        .iter()
        .map(|found| {
            let curves = chosen
                .iter()
                .enumerate()
                .map(|(position, step)| {
                    // A variable the engine saved shorter than the plot is cut short,
                    // and never read past its end.
                    let rows = |len: usize| (step * run).min(len)..((step + 1) * run).min(len);
                    Series {
                        color: ramp_at(position),
                        x: axis[rows(axis.len())].to_vec(),
                        y: found.real[rows(found.real.len())].to_vec(),
                    }
                })
                .collect();
            Panel {
                title: found.text.clone(),
                y_label: match found.unit {
                    "V" => "Voltage",
                    "A" => "Current",
                    _ => "Power",
                }
                .to_string(),
                y_unit: found.unit.to_string(),
                y_si: true,
                series: curves,
            }
        })
        .collect();
    let value_of = |step: usize| outer.start + outer.step * step as f64;
    let legend = chosen
        .iter()
        .enumerate()
        .map(|(position, step)| {
            // A temperature takes no prefix: 500 m°C would read as a milli-degree.
            let value = match outer.unit {
                "°C" => plot::plain(value_of(*step)),
                _ => plot::si(value_of(*step)),
            };
            (
                ramp_at(position),
                format!("{} = {value} {}", outer.name, outer.unit),
            )
        })
        .collect();
    let mut notes = Vec::new();
    let mut warnings = Vec::new();
    if chosen.len() < steps {
        notes.push(format!(
            "Showing {} of the {steps} steps of {}, evenly spaced from the first to the last.",
            chosen.len(),
            outer.name
        ));
        warnings.push(json!({
            "code": "curves_thinned",
            "detail": "Only some steps of the second sweep are drawn.",
            "drawn": chosen.len(),
            "steps": steps,
        }));
    }
    Drawing {
        x,
        panels,
        palette: &RAMP,
        legend,
        notes,
        warnings,
    }
}

/// What the banner says. Empty for a complete run of the circuit as drawn.
pub(super) fn banner(record: &RunRecord, changed: Option<bool>) -> Vec<String> {
    let mut lines = Vec::new();
    if record.run_status == RunState::Partial {
        let why = record
            .stopped
            .as_ref()
            .map(|stopped| format!(" ({})", stopped.reason.chars().take(40).collect::<String>()))
            .unwrap_or_default();
        // Rounded down: 99.6 percent is not the whole range.
        let share = record.coverage_ratio * 100.0;
        let covers = match share < 1.0 {
            true => "under 1%".to_string(),
            false => format!("{:.0}%", (share + 1e-9).floor()),
        };
        lines.push(format!(
            "PARTIAL RUN: the simulation stopped early{why} and covers {covers} of the requested \
             range. The traces end where the data ends."
        ));
    }
    if record.simulated_circuit != "as_drawn" {
        lines.push(
            "MODIFIED CIRCUIT: this run simulated a modified circuit, with parts left out or \
             lines added, and not the schematic as drawn."
                .to_string(),
        );
    }
    if changed == Some(true) {
        lines.push(
            "SCHEMATIC CHANGED: the saved schematic or its model files differ from what this \
             run used."
                .to_string(),
        );
    }
    lines
}

fn kind_title(analysis: &Analysis) -> &'static str {
    match analysis {
        Analysis::Transient(_) => "Transient",
        Analysis::Ac(_) => "AC sweep",
        Analysis::DcSweep(sweep) if sweep.sweeps.len() > 1 => "DC sweeps",
        Analysis::DcSweep(_) => "DC sweep",
        Analysis::OperatingPoint => "Operating point",
    }
}

/// The file's name: a short hash of what was asked, so asking again replaces the
/// plot, and the folder cannot fill by repetition.
fn file_name(signals: &[String], width: u32) -> String {
    let mut hasher = Sha256::new();
    for signal in signals {
        hasher.update(signal.as_bytes());
        hasher.update([0]);
    }
    hasher.update(width.to_le_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
    format!("{PLOT_PREFIX}{hex}{PLOT_SUFFIX}")
}

fn is_plot_name(name: &str) -> bool {
    name.strip_prefix(PLOT_PREFIX)
        .and_then(|rest| rest.strip_suffix(PLOT_SUFFIX))
        .is_some_and(|hex| hex.len() == 12 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Write the plot into the run's folder, replacing the same request's earlier file
/// and keeping the folder to `KEEP_PLOTS` plots by dropping the oldest. Only plain
/// files of our own naming are touched, and the new file appears whole or not at
/// all.
fn write_plot(dir: &Path, name: &str, svg: &str) -> std::io::Result<u64> {
    let target = dir.join(name);
    if let Ok(meta) = std::fs::symlink_metadata(&target) {
        // Never write through, or over, anything but our own earlier plot.
        if !meta.is_file() {
            return Err(std::io::ErrorKind::AlreadyExists.into());
        }
    }
    let partial = dir.join(format!("{name}.partial"));
    if let Ok(meta) = std::fs::symlink_metadata(&partial) {
        // A recent half file is another call of the same request, writing now. An old
        // one is a crash's, and nothing else is ever taken over.
        let old = meta.is_file()
            && meta
                .modified()
                .ok()
                .and_then(|made| made.elapsed().ok())
                .is_some_and(|age| age > STALE_PARTIAL);
        if !old {
            return Err(std::io::ErrorKind::AlreadyExists.into());
        }
        std::fs::remove_file(&partial)?;
    }
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .and_then(|mut file| std::io::Write::write_all(&mut file, svg.as_bytes()))
        .and_then(|()| std::fs::rename(&partial, &target));
    if let Err(error) = written {
        std::fs::remove_file(&partial).ok();
        return Err(error);
    }
    // The new plot is in place before an old one goes, and one another call already
    // removed is not an error.
    let mut others: Vec<(std::time::SystemTime, std::path::PathBuf)> = std::fs::read_dir(dir)?
        .flatten()
        .filter(|entry| entry.file_name().to_str().is_some_and(is_plot_name))
        .filter(|entry| entry.path() != target)
        .filter_map(|entry| {
            let meta = std::fs::symlink_metadata(entry.path()).ok()?;
            meta.is_file().then(|| {
                (
                    meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                    entry.path(),
                )
            })
        })
        .collect();
    others.sort();
    while others.len() + 1 > KEEP_PLOTS {
        let (_, oldest) = others.remove(0);
        std::fs::remove_file(oldest).ok();
    }
    Ok(svg.len() as u64)
}

fn not_in_run(index: usize, text: &str, pins: &[String], power: bool) -> CallToolResult {
    let hint = match (pins.is_empty(), power) {
        (false, _) => format!(" This part has these pins: {}.", pins.join(", ")),
        (true, true) => {
            " A run saves p(REF) only if it asked for it, so run again with it.".to_string()
        }
        (true, false) => String::new(),
    };
    let shown: String = text.chars().filter(|c| !c.is_control()).take(80).collect();
    invalid(
        format!("signals[{index}]"),
        format!("{shown} is not in this run's data.{hint}"),
    )
}

pub(super) fn plot(
    ctx: &super::ToolContext,
    question: &Question<'_>,
) -> Result<CallToolResult, (CallToolResult, String)> {
    let target = match runs::is_valid_id(question.run_id) {
        true => question.run_id.to_string(),
        false => "(not a run id)".to_string(),
    };
    let width = question.check().map_err(|result| (result, target))?;
    let opened = stored::open(ctx, question.run_id)?;
    let target = opened.target.clone();
    let refuse = |result: CallToolResult| (result, target.clone());
    let analysis = &opened.analysis;
    let grid = matches!(analysis, Analysis::DcSweep(sweep) if sweep.sweeps.len() > 1);
    if matches!(analysis, Analysis::OperatingPoint) {
        return Err(refuse(invalid(
            "run_id",
            "An operating point has one value for each signal and nothing to plot them against. \
             Its table is in the run's results.csv.",
        )));
    }
    let count = question.signals.as_array().map_or(0, Vec::len);
    if grid && count > MAX_GRID_SIGNALS {
        return Err(refuse(invalid(
            "signals",
            format!("A dc_sweep with two sweeps plots at most {MAX_GRID_SIGNALS} signals"),
        )));
    }
    let wanted = Wanted::for_analysis(analysis, question.signals, &json!([]), 0).map_err(refuse)?;
    let plot = stored::read_plot(&opened, question.run_id)?;
    let record = &opened.record;
    let names = Names {
        nets: &record.nets,
        elements: &record.elements,
    };

    let mut found = Vec::new();
    let mut shared: BTreeSet<String> = BTreeSet::new();
    for (index, (text, signal)) in wanted.signals.iter().enumerate() {
        match signals::resolve(&plot, signal, names) {
            Ok(values) => {
                shared.extend(values.shared_nets.iter().cloned());
                found.push(Found {
                    text: text.clone(),
                    unit: signal.unit(),
                    real: values.values,
                    imaginary: values.imaginary,
                });
            }
            Err(missing) => {
                let power = matches!(signal, signals::Signal::Power(_));
                return Err(refuse(not_in_run(index, text, &missing.pins, power)));
            }
        }
    }

    let partial = record.run_status == RunState::Partial;
    let mut drawing = match analysis {
        Analysis::Transient(_) => transient(&opened, &plot, &found).map_err(refuse)?,
        Analysis::Ac(ac_run) => {
            bode(&plot, &found, ac_run).map_err(|detail| refuse(gone(&opened, &detail)))?
        }
        Analysis::DcSweep(sweep) if grid => dc_grid(sweep, &plot, &found),
        Analysis::DcSweep(sweep) => dc_one(sweep, &plot, &found),
        Analysis::OperatingPoint => unreachable!("refused above"),
    };
    if !shared.is_empty() {
        drawing.warnings.push(json!({
            "code": "nets_share_engine_node",
            "detail": "ngspice reads these nets as one node, so their signals are the same.",
            "items": shared,
        }));
    }

    let changed = CurrentSources::of(&record.schematic).changed_since(record);
    let file = record
        .schematic
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut chart = Chart {
        title: format!(
            "{} of {file}, run {}",
            kind_title(analysis),
            question.run_id
        ),
        banner: banner(record, changed),
        x: drawing.x,
        panels: drawing.panels,
        palette: drawing.palette,
        legend: drawing.legend,
        notes: drawing.notes,
        width,
    };
    // The note counts points once the traces are cut, so it needs a first drawing.
    let mut rendered: Rendered = plot::render(&chart);
    if rendered.points_drawn < rendered.points_in {
        chart.notes.push(format!(
            "Each trace is drawn as an envelope, the lowest and highest point of every pixel \
             column: the run holds {} points a signal, and {} points were drawn in all.",
            plot.points, rendered.points_drawn
        ));
        rendered = plot::render(&chart);
    }

    let signal_texts: Vec<String> = found.iter().map(|found| found.text.clone()).collect();
    let name = file_name(&signal_texts, width);
    let bytes = write_plot(&opened.dir, &name, &rendered.svg).map_err(|error| {
        refuse(simulation_failure(
            "run",
            "run_store_failed",
            None,
            Vec::new(),
            format!("Failed to write the plot: {error}"),
        ))
    })?;
    let path = opened.dir.join(&name).display().to_string();

    let body = json!({
        "run_id": question.run_id,
        "analysis_kind": analysis.kind(),
        "path": path,
        "width_px": rendered.width,
        "height_px": rendered.height,
        "bytes": bytes,
        "signals_plotted": signal_texts,
        "points_in_run": plot.points,
        "points_drawn": rendered.points_drawn,
        "run_status": record.run_status.as_str(),
        "coverage_ratio": record.coverage_ratio,
        "simulated_circuit": record.simulated_circuit,
        "inputs_changed_since_run": changed,
        "warnings": drawing.warnings,
    });
    // The plot is one unit and the range the run was asked for is the other, as in
    // `run_simulation`, so a partial run reads partial with both counts non-zero.
    let (status, completed, failed) = match partial {
        true => (OutcomeStatus::Partial, 1, 1),
        false => (OutcomeStatus::Complete, 2, 0),
    };
    let outcome = outcome::summary(status, &target, SOURCE, 2, completed, failed, None);
    Ok(outcome::attach(CallToolResult::json(&body), outcome))
}
