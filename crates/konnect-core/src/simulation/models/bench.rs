//! Benches that measure a fitted model's datasheet values on the engine.
//!
//! A bench is a set of small fixtures, each its own circuit to ground with
//! nodes and names no other fixture uses, so one operating point and one AC run
//! per frequency measure every check at once. Konnect writes every line: the
//! numbers are formatted here and the names are its own.

use super::text::number;
use crate::simulation::raw::Plot;
use crate::simulation::signals::{self, Names, Signal};

/// What a check proves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// The fit used this value, so a match shows the engine read the card as
    /// written, not that the model generalises.
    Fit,
    /// A value the fit left out, which the model had to reach on its own.
    HeldOut,
    /// A value the model's law fixes from others, compared but not fitted.
    CheckOnly,
}

impl Role {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Role::Fit => "fit_residual",
            Role::HeldOut => "held_out",
            Role::CheckOnly => "check_only",
        }
    }
}

/// One caller value the engine must reproduce.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Check {
    /// Its place in the caller's characteristics.
    pub characteristic: usize,
    pub role: Role,
    pub expected: f64,
    pub probe: Probe,
}

/// How a check reads its value from the engine's results.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Probe {
    /// Operating point: `scale * v(node) / divide_by`.
    Voltage {
        node: String,
        scale: f64,
        divide_by: f64,
    },
    /// Operating point: `scale * i(source)`, through a voltage source.
    Current { source: String, scale: f64 },
    /// AC at `hz`: `|Im(i(source))| / (2 pi hz)`, the capacitance that a unit
    /// AC voltage from `source` drives.
    Capacitance { source: String, hz: f64 },
    /// AC at `hz`: `|v(node)|`, the impedance a unit AC current meets.
    Impedance { node: String, hz: f64 },
    /// AC at `hz`: `Re(Z)`, where `Z = -1 / i(source)` is the impedance a unit
    /// AC voltage from `source` drives. A voltage source keeps every node tied
    /// at DC, which a capacitor alone would leave floating.
    Resistance { source: String, hz: f64 },
    /// AC at `hz`: `Im(Z) / (2 pi hz)`, as an inductance.
    Inductance { source: String, hz: f64 },
    /// AC at `hz`: `-1 / (2 pi hz Im(Z))`, the series capacitance an LCR meter
    /// reads.
    SeriesCapacitance { source: String, hz: f64 },
    /// AC at `hz`: `Im(v(node)) / (2 pi hz)`, the inductance a unit AC current
    /// into `node` meets.
    NodeInductance { node: String, hz: f64 },
    /// AC at `hz`: `scale * |i(source)|`, the current a unit AC drive makes
    /// `source` carry. With `scale = hz` it reads a transition frequency.
    CurrentMagnitude { source: String, scale: f64, hz: f64 },
    /// AC at `hz`: `scale * |v(node)|`, the gain from a unit AC drive. With
    /// `scale = hz` it reads a gain-bandwidth product.
    Gain { node: String, hz: f64, scale: f64 },
    /// Operating point: `offset + scale * v(node)`, such as a supply less an
    /// output.
    Affine { node: String, offset: f64, scale: f64 },
    /// Transient: the rate `node` rises from `low` to `high`, by the times it
    /// first crosses each.
    Slope { node: String, low: f64, high: f64 },
    /// Transient, or the slow transient with `slow`: the time from `input`
    /// first rising through `input_level` to `output` then rising through
    /// `output_level`.
    Delay {
        input: String,
        input_level: f64,
        output: String,
        output_level: f64,
        slow: bool,
    },
    /// Slow transient: `input` where `output` first rises through `level`, and
    /// where it next falls through it, as their difference or their middle.
    Trip {
        input: String,
        output: String,
        level: f64,
        what: Trip,
    },
    /// Transient: the charge a constant gate current `amps` has delivered by
    /// the time `what` happens. Voltages are read times `sign`, so a P-channel
    /// part reads as an N-channel one.
    Charge {
        gate: String,
        drain: String,
        amps: f64,
        vds: f64,
        sign: f64,
        what: Charge,
    },
}

/// What a pair of trip points reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trip {
    /// The rising trip less the falling one: a hysteresis.
    Width,
    /// Their middle: an input offset.
    Center,
}

/// Which charge a transient probe reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Charge {
    /// Until the gate reaches `vgs`: the datasheet's total gate charge.
    Total { vgs: f64 },
    /// While the drain falls from 90 to 10 percent of `vds`: the charge the
    /// drain's fall takes from the gate, the datasheet's gate-drain charge.
    Miller,
}

impl Probe {
    /// Whether the probe reads the transient run.
    pub(crate) fn is_transient(&self) -> bool {
        matches!(
            self,
            Probe::Charge { .. } | Probe::Slope { .. } | Probe::Delay { slow: false, .. }
        )
    }

    /// Whether the probe reads the slow transient run.
    pub(crate) fn is_slow(&self) -> bool {
        matches!(self, Probe::Trip { .. } | Probe::Delay { slow: true, .. })
    }

    /// The frequency of an AC probe; none for an operating point.
    pub(crate) fn hz(&self) -> Option<f64> {
        match self {
            Probe::Voltage { .. }
            | Probe::Current { .. }
            | Probe::Charge { .. }
            | Probe::Affine { .. }
            | Probe::Slope { .. }
            | Probe::Delay { .. }
            | Probe::Trip { .. } => None,
            Probe::Gain { hz, .. } => Some(*hz),
            Probe::Capacitance { hz, .. }
            | Probe::Impedance { hz, .. }
            | Probe::Resistance { hz, .. }
            | Probe::Inductance { hz, .. }
            | Probe::SeriesCapacitance { hz, .. }
            | Probe::NodeInductance { hz, .. }
            | Probe::CurrentMagnitude { hz, .. } => Some(*hz),
        }
    }
}

/// The fixtures of a bench.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Bench {
    /// For the operating point.
    pub dc: Vec<String>,
    /// For the AC runs: every AC run holds them all.
    pub ac: Vec<String>,
    /// For the transient run, which lasts `tran_stop` seconds.
    pub tran: Vec<String>,
    pub tran_stop: f64,
    /// For a second transient run, `slow_stop` seconds long, for checks on
    /// another time scale than the first, such as a slow ramp.
    pub slow: Vec<String>,
    pub slow_stop: f64,
    next: usize,
}

impl Bench {
    /// A fresh fixture name, so no two fixtures share a node or an element.
    pub(crate) fn id(&mut self) -> String {
        self.next += 1;
        format!("b{}", self.next)
    }

    /// `I<id> 0 <node> DC <amps>`: current pushed into `node`, with a 1 Tohm
    /// compliance path so a branch the model lacks cannot drag the solve to a
    /// teravolt.
    pub(crate) fn current_into(id: &str, node: &str, amps: f64) -> String {
        format!(
            "I{id} 0 {node} DC {}\nR{id}c {node} 0 1.000000e12",
            number(amps)
        )
    }

    /// `V<id> <node> 0 DC <volts>`, optionally with a unit AC value.
    pub(crate) fn voltage(id: &str, node: &str, volts: f64, ac: bool) -> String {
        let ac = if ac { " AC 1" } else { "" };
        format!("V{id} {node} 0 DC {}{ac}", number(volts))
    }

    /// `I<id> 0 <node> DC <amps> AC 1`: a DC bias with a unit AC current.
    pub(crate) fn ac_current_into(id: &str, node: &str, amps: f64) -> String {
        format!("I{id} 0 {node} DC {} AC 1", number(amps))
    }
}

/// The distinct frequencies the checks need an AC run at, in order.
pub(crate) fn ac_frequencies(checks: &[Check]) -> Vec<f64> {
    let mut frequencies: Vec<f64> = Vec::new();
    for hz in checks.iter().filter_map(|check| check.probe.hz()) {
        if !frequencies.contains(&hz) {
            frequencies.push(hz);
        }
    }
    frequencies
}

/// Whether any check reads the operating point.
pub(crate) fn needs_operating_point(checks: &[Check]) -> bool {
    checks.iter().any(|check| {
        check.probe.hz().is_none() && !check.probe.is_transient() && !check.probe.is_slow()
    })
}

/// Whether any check reads the slow transient run.
pub(crate) fn needs_slow_transient(checks: &[Check]) -> bool {
    checks.iter().any(|check| check.probe.is_slow())
}

/// Whether any check reads the transient run.
pub(crate) fn needs_transient(checks: &[Check]) -> bool {
    checks.iter().any(|check| check.probe.is_transient())
}

const NO_NAMES: Names<'static> = Names {
    nets: &[],
    elements: &[],
};

fn first(plot: &Plot, signal: Signal) -> Option<(f64, f64)> {
    let resolved = signals::resolve(plot, &signal, NO_NAMES).ok()?;
    let real = *resolved.values.first()?;
    let imaginary = resolved.imaginary.first().copied().unwrap_or(0.0);
    (real.is_finite() && imaginary.is_finite()).then_some((real, imaginary))
}

fn voltage(plot: &Plot, node: &str) -> Option<(f64, f64)> {
    first(plot, Signal::Voltage(node.to_string()))
}

fn current(plot: &Plot, source: &str) -> Option<(f64, f64)> {
    first(
        plot,
        Signal::Current {
            reference: source.to_string(),
            pin: None,
        },
    )
}

/// The value a check reads, from the operating point or from the AC run at its
/// frequency. None when the run does not hold the signal.
pub(crate) fn measure(
    check: &Check,
    dc: Option<&Plot>,
    ac: &[(f64, Plot)],
    tran: Option<&Plot>,
    slow: Option<&Plot>,
) -> Option<f64> {
    let tau = std::f64::consts::TAU;
    let at = |hz: f64| {
        ac.iter()
            .find(|(frequency, _)| *frequency == hz)
            .map(|(_, plot)| plot)
    };
    match &check.probe {
        Probe::Charge {
            gate,
            drain,
            amps,
            vds,
            sign,
            what,
        } => charge(tran?, gate, drain, *amps, *vds, *sign, *what),
        Probe::Gain { node, hz, scale } => {
            voltage(at(*hz)?, node).map(|(real, imaginary)| scale * real.hypot(imaginary))
        }
        Probe::Affine {
            node,
            offset,
            scale,
        } => voltage(dc?, node).map(|(value, _)| offset + scale * value),
        Probe::Slope { node, low, high } => {
            let (times, trace) = series(tran?, node)?;
            let start = crossing(&times, &trace, *low, true)?;
            let end = crossing_after(&times, &trace, *high, true, start)?;
            (end > start).then(|| (high - low) / (end - start))
        }
        Probe::Delay {
            input,
            input_level,
            output,
            output_level,
            slow: is_slow,
        } => {
            let plot = if *is_slow { slow? } else { tran? };
            let (times, drive) = series(plot, input)?;
            let (_, response) = series(plot, output)?;
            let start = crossing(&times, &drive, *input_level, true)?;
            let end = crossing_after(&times, &response, *output_level, true, start)?;
            Some(end - start)
        }
        Probe::Trip {
            input,
            output,
            level,
            what,
        } => {
            let (times, drive) = series(slow?, input)?;
            let (_, response) = series(slow?, output)?;
            let rise_at = crossing(&times, &response, *level, true)?;
            let fall_at = crossing_after(&times, &response, *level, false, rise_at)?;
            let rise = value_at(&times, &drive, rise_at)?;
            let fall = value_at(&times, &drive, fall_at)?;
            Some(match what {
                Trip::Width => rise - fall,
                Trip::Center => (rise + fall) / 2.0,
            })
        }
        Probe::Voltage {
            node,
            scale,
            divide_by,
        } => voltage(dc?, node).map(|(value, _)| scale * value / divide_by),
        Probe::Current { source, scale } => current(dc?, source).map(|(value, _)| scale * value),
        Probe::CurrentMagnitude { source, scale, hz } => {
            current(at(*hz)?, source).map(|(real, imaginary)| scale * real.hypot(imaginary))
        }
        Probe::Capacitance { source, hz } => {
            current(at(*hz)?, source).map(|(_, imaginary)| imaginary.abs() / (tau * hz))
        }
        Probe::Impedance { node, hz } => {
            voltage(at(*hz)?, node).map(|(real, imaginary)| real.hypot(imaginary))
        }
        Probe::Resistance { source, hz } => impedance(at(*hz)?, source).map(|(real, _)| real),
        Probe::Inductance { source, hz } => {
            impedance(at(*hz)?, source).map(|(_, imaginary)| imaginary / (tau * hz))
        }
        Probe::NodeInductance { node, hz } => {
            voltage(at(*hz)?, node).map(|(_, imaginary)| imaginary / (tau * hz))
        }
        Probe::SeriesCapacitance { source, hz } => impedance(at(*hz)?, source)
            .filter(|(_, imaginary)| *imaginary < 0.0)
            .map(|(_, imaginary)| -1.0 / (tau * hz * imaginary)),
    }
}

/// A node's voltage over a transient run, with its times.
fn series(plot: &Plot, node: &str) -> Option<(Vec<f64>, Vec<f64>)> {
    let times: Vec<f64> = plot.real(plot.variable("time")?).collect();
    let resolved = signals::resolve(plot, &Signal::Voltage(node.to_string()), NO_NAMES).ok()?;
    (resolved.values.len() == times.len()).then_some((times, resolved.values))
}

/// A trace's value at `time`, by a straight line between samples.
fn value_at(times: &[f64], values: &[f64], time: f64) -> Option<f64> {
    times.windows(2).zip(values.windows(2)).find_map(|(t, v)| {
        (t[0] <= time && time <= t[1]).then(|| {
            if t[1] > t[0] {
                v[0] + (v[1] - v[0]) * (time - t[0]) / (t[1] - t[0])
            } else {
                v[1]
            }
        })
    })
}

/// The first crossing at or after `after`.
fn crossing_after(
    times: &[f64],
    values: &[f64],
    level: f64,
    rising: bool,
    after: f64,
) -> Option<f64> {
    times
        .windows(2)
        .zip(values.windows(2))
        .filter(|(t, _)| t[1] >= after)
        .find_map(|(t, v)| segment_crossing(t, v, level, rising).filter(|time| *time >= after))
}

/// The first time a falling or rising trace crosses `level`, by straight lines
/// between samples.
fn crossing(times: &[f64], values: &[f64], level: f64, rising: bool) -> Option<f64> {
    times
        .windows(2)
        .zip(values.windows(2))
        .find_map(|(t, v)| segment_crossing(t, v, level, rising))
}

fn segment_crossing(t: &[f64], v: &[f64], level: f64, rising: bool) -> Option<f64> {
    let crosses = if rising {
        v[0] < level && v[1] >= level
    } else {
        v[0] > level && v[1] <= level
    };
    crosses.then(|| t[0] + (t[1] - t[0]) * (level - v[0]) / (v[1] - v[0]))
}

fn charge(
    plot: &Plot,
    gate: &str,
    drain: &str,
    amps: f64,
    vds: f64,
    sign: f64,
    what: Charge,
) -> Option<f64> {
    let times: Vec<f64> = plot.real(plot.variable("time")?).collect();
    let trace = |node: &str| -> Option<Vec<f64>> {
        let resolved = signals::resolve(plot, &Signal::Voltage(node.to_string()), NO_NAMES).ok()?;
        Some(resolved.values.iter().map(|v| v * sign).collect())
    };
    match what {
        Charge::Total { vgs } => {
            let gate = trace(gate)?;
            crossing(&times, &gate, vgs, true).map(|t| amps * t)
        }
        Charge::Miller => {
            let drain = trace(drain)?;
            let high = crossing(&times, &drain, 0.9 * vds, false)?;
            let low = crossing(&times, &drain, 0.1 * vds, false)?;
            Some(amps * (low - high))
        }
    }
}

/// `-1 / i(source)`: the impedance a unit AC voltage from `source` drives. The
/// source delivers the current, so its own current reads negative.
fn impedance(plot: &Plot, source: &str) -> Option<(f64, f64)> {
    let (real, imaginary) = current(plot, source)?;
    let size = real * real + imaginary * imaginary;
    (size > 0.0).then(|| (-real / size, imaginary / size))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(probe: Probe) -> Check {
        Check {
            characteristic: 0,
            role: Role::Fit,
            expected: 1.0,
            probe,
        }
    }

    #[test]
    fn fixtures_use_konnect_names_and_plain_numbers() {
        let mut bench = Bench::default();
        let id = bench.id();
        assert_eq!(id, "b1");
        assert_eq!(bench.id(), "b2");
        assert_eq!(
            Bench::current_into(&id, "b1a", 0.01),
            "Ib1 0 b1a DC 1.000000e-2\nRb1c b1a 0 1.000000e12"
        );
        assert_eq!(
            Bench::voltage(&id, "b1k", 25.0, true),
            "Vb1 b1k 0 DC 2.500000e1 AC 1"
        );
    }

    #[test]
    fn crossings_interpolate_between_samples() {
        let times = [0.0, 1.0, 2.0, 3.0];
        assert_eq!(
            crossing(&times, &[0.0, 2.0, 4.0, 6.0], 3.0, true),
            Some(1.5)
        );
        assert_eq!(
            crossing(&times, &[6.0, 4.0, 2.0, 0.0], 3.0, false),
            Some(1.5)
        );
        assert_eq!(crossing(&times, &[0.0, 1.0, 2.0, 2.5], 3.0, true), None);
    }

    #[test]
    fn a_charge_probe_reads_only_a_transient_run() {
        let probe = Probe::Charge {
            gate: "g".into(),
            drain: "d".into(),
            amps: 0.1,
            vds: 10.0,
            sign: 1.0,
            what: Charge::Miller,
        };
        assert!(probe.is_transient() && probe.hz().is_none());
        let checks = [check(probe)];
        assert!(needs_transient(&checks));
        assert!(!needs_operating_point(&checks));
    }

    #[test]
    fn each_frequency_gets_one_run() {
        let checks = [
            check(Probe::Voltage {
                node: "a".into(),
                scale: 1.0,
                divide_by: 1.0,
            }),
            check(Probe::Capacitance {
                source: "v1".into(),
                hz: 1e6,
            }),
            check(Probe::Impedance {
                node: "a".into(),
                hz: 1e3,
            }),
            check(Probe::Resistance {
                source: "v1".into(),
                hz: 1e6,
            }),
        ];
        assert_eq!(ac_frequencies(&checks), [1e6, 1e3]);
        assert!(needs_operating_point(&checks));
        assert!(!needs_operating_point(&checks[1..]));
    }
}
