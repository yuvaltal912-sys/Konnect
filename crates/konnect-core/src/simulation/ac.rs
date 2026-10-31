//! AC small-signal results: gain, phase, and the margins taken from them.
//!
//! A sweep is a list of complex values at rising frequencies. Gain is the
//! magnitude in dB, and phase is unwrapped from the lowest frequency, which is
//! the only way a margin can be read: the principal value jumps by 360 degrees
//! wherever the real phase passes -180. Between two samples, a Bode curve is a
//! straight line against the log of frequency, or against frequency on a linear
//! sweep, so that is how a crossing is placed.

use super::measure::{Kind, Measurement};
use serde_json::{json, Map, Value};

/// The point 3 dB down is 10 log10 of one half, not 3.
const HALF_POWER_DB: f64 = 3.010_299_956_639_812;
const UNITY_DB: f64 = 0.0;
const INVERSION_DEGREES: f64 = -180.0;
/// A loop that starts this close to -180 degrees, or past it, has no first crossing
/// to find: rounding in the engine decides which side of the line it begins on.
const START_TOLERANCE_DEGREES: f64 = 1e-3;
/// Relative slack on the ends of the sweep, for the rounding the engine leaves.
const EDGE_TOLERANCE: f64 = 1e-9;

/// `a / b` for complex numbers given as real and imaginary parts, point by point.
/// A zero in `b` gives non-finite values, which a measurement reports.
pub(crate) fn divide(a: (&[f64], &[f64]), b: (&[f64], &[f64])) -> (Vec<f64>, Vec<f64>) {
    let mut real = Vec::with_capacity(a.0.len());
    let mut imaginary = Vec::with_capacity(a.0.len());
    for index in 0..a.0.len().min(b.0.len()) {
        let (ar, ai, br, bi) = (a.0[index], a.1[index], b.0[index], b.1[index]);
        let power = br * br + bi * bi;
        real.push((ar * br + ai * bi) / power);
        imaginary.push((ai * br - ar * bi) / power);
    }
    (real, imaginary)
}

fn degrees(real: f64, imaginary: f64) -> f64 {
    imaginary.atan2(real).to_degrees()
}

/// Gain and phase against frequency.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Response {
    freq: Vec<f64>,
    gain_db: Vec<f64>,
    /// Unwrapped from the first sample.
    phase: Vec<f64>,
    /// The sweep is spaced by ratio, so lines are drawn against `ln f`.
    log_axis: bool,
}

impl Response {
    pub(crate) fn new(
        freq: &[f64],
        real: &[f64],
        imaginary: &[f64],
        log_axis: bool,
    ) -> Option<Self> {
        let points = freq.len();
        if points == 0 || real.len() != points || imaginary.len() != points {
            return None;
        }
        let gain_db = real
            .iter()
            .zip(imaginary)
            .map(|(re, im)| 20.0 * re.hypot(*im).log10())
            .collect();
        let mut phase: Vec<f64> = Vec::with_capacity(points);
        let mut last_principal = 0.0;
        for (re, im) in real.iter().zip(imaginary) {
            let principal = degrees(*re, *im);
            phase.push(match phase.last() {
                None => principal,
                // The smallest turn from the last sample, to a multiple of 360.
                Some(last) => {
                    let step = principal - last_principal;
                    last + step - 360.0 * (step / 360.0).round()
                }
            });
            last_principal = principal;
        }
        Some(Self {
            freq: freq.to_vec(),
            gain_db,
            phase,
            log_axis,
        })
    }

    fn len(&self) -> usize {
        self.freq.len()
    }

    fn x(&self, index: usize) -> f64 {
        match self.log_axis {
            true => self.freq[index].ln(),
            false => self.freq[index],
        }
    }

    fn frequency(&self, x: f64) -> f64 {
        match self.log_axis {
            true => x.exp(),
            false => x,
        }
    }

    /// The frequency between samples `index` and the next where a straight line
    /// of `series` reaches `level`.
    fn crossing(&self, series: &[f64], index: usize, level: f64) -> f64 {
        let (x0, x1) = (self.x(index), self.x(index + 1));
        let (y0, y1) = (series[index], series[index + 1]);
        let x = if y1 == y0 {
            x0
        } else {
            x0 + (x1 - x0) * (level - y0) / (y1 - y0)
        };
        self.frequency(x)
    }

    /// The value of `series` at `frequency`, which lies within the sweep.
    fn at(&self, series: &[f64], frequency: f64) -> f64 {
        let after = self.freq.partition_point(|&sample| sample < frequency);
        if after == 0 {
            return series[0];
        }
        if after == self.len() {
            return series[after - 1];
        }
        let x = match self.log_axis {
            true => frequency.ln(),
            false => frequency,
        };
        let (x0, x1) = (self.x(after - 1), self.x(after));
        let (y0, y1) = (series[after - 1], series[after]);
        y0 + (y1 - y0) * (x - x0) / (x1 - x0)
    }

    /// The samples from `from` to `to`, or none of the sweep if fewer than two.
    fn window(&self, from: Option<f64>, to: Option<f64>) -> Option<Self> {
        let keep = |frequency: f64| {
            from.is_none_or(|from| frequency >= from * (1.0 - EDGE_TOLERANCE))
                && to.is_none_or(|to| frequency <= to * (1.0 + EDGE_TOLERANCE))
        };
        let picked: Vec<usize> = (0..self.len()).filter(|&i| keep(self.freq[i])).collect();
        (picked.len() >= 2).then(|| Self {
            freq: picked.iter().map(|&i| self.freq[i]).collect(),
            gain_db: picked.iter().map(|&i| self.gain_db[i]).collect(),
            phase: picked.iter().map(|&i| self.phase[i]).collect(),
            log_axis: self.log_axis,
        })
    }

    fn all_finite(&self) -> bool {
        self.gain_db
            .iter()
            .chain(&self.phase)
            .all(|value| value.is_finite())
    }

    /// The first step where `series` goes from at or above `level` to below it.
    fn first_fall(&self, series: &[f64], level: f64, from: usize) -> Option<usize> {
        (from..self.len() - 1).find(|&i| series[i] >= level && series[i + 1] < level)
    }

    fn first_rise(&self, series: &[f64], level: f64, to: usize) -> Option<usize> {
        (0..to).find(|&i| series[i] < level && series[i + 1] >= level)
    }
}

/// An answer, or the finding that says why there is none.
struct Found {
    value: Option<f64>,
    at: Option<f64>,
    finding: Option<&'static str>,
    extras: Map<String, Value>,
}

impl Found {
    fn value(value: f64) -> Self {
        Self {
            value: Some(value),
            at: None,
            finding: None,
            extras: Map::new(),
        }
    }

    fn none(finding: &'static str) -> Self {
        Self {
            value: None,
            at: None,
            finding: Some(finding),
            extras: Map::new(),
        }
    }

    fn at(mut self, frequency: f64) -> Self {
        self.at = Some(frequency);
        self
    }

    fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.extras.insert(key.to_string(), value.into());
        self
    }
}

/// One AC measurement of `response`, as it appears in a result.
pub(crate) fn measure(response: &Response, request: &Measurement, window_truncated: bool) -> Value {
    let found = find(response, request);
    let mut report = json!({
        "name": request.name,
        "kind": request.kind_name,
        "signal": request.signal_text,
        "value": found.value,
        "unit": request.unit(),
        "window_truncated": window_truncated,
    });
    if let Some((text, _)) = &request.reference_signal {
        report["reference_signal"] = json!(text);
    }
    if let Some(at) = found.at {
        report["at_hz"] = json!(at);
    }
    if let Some(finding) = found.finding {
        report["finding"] = json!(finding);
    }
    for (key, value) in found.extras {
        report[key] = value;
    }
    report
}

fn find(response: &Response, request: &Measurement) -> Found {
    if request.kind == Kind::ValueAt {
        return value_at(response, request.at.unwrap_or_default());
    }
    let Some(window) = response.window(request.from, request.to) else {
        return Found::none("outside_data");
    };
    if !window.all_finite() {
        return Found::none("non_finite_data");
    }
    match request.kind {
        Kind::Bandwidth3db if request.peak_reference => band_edges(&window),
        Kind::Bandwidth3db => bandwidth(&window),
        Kind::PhaseMargin => phase_margin(&window),
        Kind::GainMargin => gain_margin(&window),
        _ => Found::none("outside_data"),
    }
}

fn value_at(response: &Response, frequency: f64) -> Found {
    let (&first, &last) = (&response.freq[0], &response.freq[response.len() - 1]);
    let inside =
        frequency >= first * (1.0 - EDGE_TOLERANCE) && frequency <= last * (1.0 + EDGE_TOLERANCE);
    if !inside {
        return Found::none("outside_data");
    }
    let gain = response.at(&response.gain_db, frequency);
    let phase = response.at(&response.phase, frequency);
    if !gain.is_finite() || !phase.is_finite() {
        return Found::none("non_finite_data");
    }
    Found::value(gain)
        .at(frequency)
        .with("phase_degrees", phase)
}

/// Where the gain first falls 3 dB below its level at the lowest frequency.
fn bandwidth(window: &Response) -> Found {
    let reference = window.gain_db[0];
    let level = reference - HALF_POWER_DB;
    match window.first_fall(&window.gain_db, level, 0) {
        Some(index) => {
            let edge = window.crossing(&window.gain_db, index, level);
            Found::value(edge)
                .at(edge)
                .with("reference_gain_db", reference)
        }
        None => Found::none("level_not_reached").with("reference_gain_db", reference),
    }
}

/// The two places the gain is 3 dB below its peak. Either may lie outside the
/// sweep, and then there is no bandwidth to give.
fn band_edges(window: &Response) -> Found {
    let (peak_at, peak) =
        window
            .gain_db
            .iter()
            .enumerate()
            .fold((0, window.gain_db[0]), |best, (index, &gain)| {
                if gain > best.1 {
                    (index, gain)
                } else {
                    best
                }
            });
    let level = peak - HALF_POWER_DB;
    let lower = window
        .first_rise(&window.gain_db, level, peak_at)
        .map(|index| window.crossing(&window.gain_db, index, level));
    let upper = (peak_at < window.len() - 1)
        .then(|| window.first_fall(&window.gain_db, level, peak_at))
        .flatten()
        .map(|index| window.crossing(&window.gain_db, index, level));
    let found = match (lower, upper) {
        (Some(lower), Some(upper)) => Found::value(upper - lower),
        _ => Found::none("level_not_reached"),
    };
    found
        .with("lower_edge_hz", lower)
        .with("upper_edge_hz", upper)
        .with("peak_gain_db", peak)
        .with("peak_hz", window.freq[peak_at])
}

/// 180 degrees plus the phase where the gain first falls through 0 dB.
fn phase_margin(window: &Response) -> Found {
    let start = window.phase[0];
    let note = |found: Found| found.with("start_phase_degrees", start);
    if window.gain_db[0] == UNITY_DB {
        return note(Found::none("starts_on_level"));
    }
    let Some(index) = window.first_fall(&window.gain_db, UNITY_DB, 0) else {
        return note(Found::none("level_not_reached"));
    };
    let crossover = window.crossing(&window.gain_db, index, UNITY_DB);
    let phase = window.at(&window.phase, crossover);
    note(
        Found::value(180.0 + phase)
            .at(crossover)
            .with("crossover_hz", crossover)
            .with("phase_degrees", phase),
    )
}

/// The gain, negated, where the phase first falls through -180 degrees.
fn gain_margin(window: &Response) -> Found {
    let start = window.phase[0];
    let note = |found: Found| found.with("start_phase_degrees", start);
    if start <= INVERSION_DEGREES + START_TOLERANCE_DEGREES {
        return note(Found::none("starts_on_level"));
    }
    let Some(index) = (0..window.len() - 1)
        .find(|&i| window.phase[i] > INVERSION_DEGREES && window.phase[i + 1] <= INVERSION_DEGREES)
    else {
        return note(Found::none("level_not_reached"));
    };
    let crossover = window.crossing(&window.phase, index, INVERSION_DEGREES);
    let gain = window.at(&window.gain_db, crossover);
    note(
        Found::value(-gain)
            .at(crossover)
            .with("phase_crossover_hz", crossover)
            .with("gain_db", gain),
    )
}

/// What a signal did over the whole sweep.
pub(crate) fn summary(
    freq: &[f64],
    real: &[f64],
    imaginary: &[f64],
    signal_text: &str,
    window_truncated: bool,
) -> Value {
    let gain: Vec<f64> = real
        .iter()
        .zip(imaginary)
        .map(|(re, im)| 20.0 * re.hypot(*im).log10())
        .collect();
    let finite = |value: Option<&f64>| value.copied().filter(|value| value.is_finite());
    let peak = gain
        .iter()
        .enumerate()
        .filter(|(_, value)| value.is_finite())
        .fold(
            None,
            |best: Option<(usize, f64)>, (index, &value)| match best {
                Some((_, high)) if high >= value => best,
                _ => Some((index, value)),
            },
        );
    let mut report = json!({
        "signal": signal_text,
        "unit": "dB",
        "peak_gain_db": peak.map(|(_, value)| value),
        "peak_at_hz": peak.and_then(|(index, _)| freq.get(index)),
        "low_frequency_gain_db": finite(gain.first()),
        "high_frequency_gain_db": finite(gain.last()),
        "window_truncated": window_truncated,
    });
    if gain.iter().any(|value| !value.is_finite()) {
        report["finding"] = json!("non_finite_data");
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::measure::{self, AxisSpec};

    const POLES_HZ: [f64; 3] = [100.0, 1_000.0, 10_000.0];
    const DC_GAIN: f64 = 100.0;

    /// Gain and phase of `K / ((1 + jf/f1)(1 + jf/f2)(1 + jf/f3))`, worked out from
    /// the formula and not from any engine.
    fn three_pole(f: f64) -> (f64, f64) {
        let magnitude = DC_GAIN
            / POLES_HZ
                .iter()
                .map(|p| (1.0 + (f / p).powi(2)).sqrt())
                .product::<f64>();
        let phase: f64 = -POLES_HZ
            .iter()
            .map(|p| (f / p).atan().to_degrees())
            .sum::<f64>();
        (magnitude, phase)
    }

    fn sweep(per_decade: usize, start: f64, stop: f64) -> Vec<f64> {
        let steps = ((stop / start).log10() * per_decade as f64).round() as usize;
        (0..=steps)
            .map(|n| start * 10f64.powf(n as f64 / per_decade as f64))
            .collect()
    }

    fn response_of(freq: &[f64], log_axis: bool) -> Response {
        let parts: Vec<(f64, f64)> = freq
            .iter()
            .map(|&f| {
                let (magnitude, phase) = three_pole(f);
                (
                    magnitude * phase.to_radians().cos(),
                    magnitude * phase.to_radians().sin(),
                )
            })
            .collect();
        let real: Vec<f64> = parts.iter().map(|p| p.0).collect();
        let imaginary: Vec<f64> = parts.iter().map(|p| p.1).collect();
        Response::new(freq, &real, &imaginary, log_axis).unwrap()
    }

    /// Bisection on a falling function, written here so it shares nothing with the
    /// code under test.
    fn solve(mut low: f64, mut high: f64, f: impl Fn(f64) -> f64) -> f64 {
        for _ in 0..200 {
            let middle = (low * high).sqrt();
            if f(middle) > 0.0 {
                low = middle;
            } else {
                high = middle;
            }
        }
        (low * high).sqrt()
    }

    fn request(kind: &str, extra: Value) -> Measurement {
        let mut entry = json!({ "kind": kind, "signal": "v(out)" });
        for (key, value) in extra.as_object().unwrap() {
            entry[key] = value.clone();
        }
        measure::parse(&entry, 0, &AxisSpec::frequency(1.0, 1e6)).unwrap()
    }

    fn value_of(report: &Value) -> f64 {
        report["value"]
            .as_f64()
            .unwrap_or_else(|| panic!("{report}"))
    }

    #[test]
    fn a_three_pole_loop_gives_the_margins_the_formula_does() {
        let response = response_of(&sweep(200, 1.0, 1e6), true);
        // Unity gain, and -180 degrees, found by bisection on the formulas.
        let fc = solve(100.0, 1e6, |f| three_pole(f).0 - 1.0);
        let pm = 180.0 + three_pole(fc).1;
        let f180 = solve(100.0, 1e6, |f| three_pole(f).1 + 180.0);
        let gm = -20.0 * three_pole(f180).0.log10();

        let report = measure(&response, &request("phase_margin", json!({})), false);
        assert!((value_of(&report) - pm).abs() < 1e-3, "{report} vs {pm}");
        assert!(
            (report["crossover_hz"].as_f64().unwrap() / fc - 1.0).abs() < 1e-4,
            "{report} vs {fc}"
        );
        assert_eq!(report["unit"], "degrees");

        let report = measure(&response, &request("gain_margin", json!({})), false);
        assert!((value_of(&report) - gm).abs() < 1e-3, "{report} vs {gm}");
        assert!((report["phase_crossover_hz"].as_f64().unwrap() / f180 - 1.0).abs() < 1e-4);
        assert_eq!(report["unit"], "dB");
        // A loop with small positive margins, so a wrong sign or a lost 360 shows.
        assert!(pm > 0.0 && pm < 20.0, "{pm}");
        assert!(gm > 0.0 && gm < 6.0, "{gm}");
    }

    #[test]
    fn bandwidth_is_where_the_gain_is_3_db_below_the_lowest_frequency() {
        let response = response_of(&sweep(200, 1.0, 1e6), true);
        let expected = solve(1.0, 1e6, |f| three_pole(f).0 - DC_GAIN * 0.5f64.sqrt());
        let report = measure(&response, &request("bandwidth_3db", json!({})), false);
        assert!(
            (value_of(&report) / expected - 1.0).abs() < 1e-4,
            "{report} vs {expected}"
        );
        assert_eq!(report["unit"], "Hz");
        assert!((report["reference_gain_db"].as_f64().unwrap() - 40.0).abs() < 1e-3);
    }

    #[test]
    fn a_curve_that_never_falls_3_db_has_no_bandwidth_and_says_so() {
        let freq = sweep(20, 1.0, 10.0);
        let response = response_of(&freq, true);
        let report = measure(&response, &request("bandwidth_3db", json!({})), false);
        assert!(report["value"].is_null());
        assert_eq!(report["finding"], "level_not_reached");
        let report = measure(&response, &request("phase_margin", json!({})), false);
        assert_eq!(report["finding"], "level_not_reached");
        let report = measure(&response, &request("gain_margin", json!({})), false);
        assert_eq!(report["finding"], "level_not_reached");
    }

    #[test]
    fn value_at_gives_gain_in_db_and_the_phase_unwrapped() {
        let response = response_of(&sweep(200, 1.0, 1e6), true);
        let f = 4_321.0;
        let report = measure(
            &response,
            &request("value_at", json!({ "at_hz": f })),
            false,
        );
        let (magnitude, phase) = three_pole(f);
        assert!(
            (value_of(&report) - 20.0 * magnitude.log10()).abs() < 1e-3,
            "{report}"
        );
        assert!(
            (report["phase_degrees"].as_f64().unwrap() - phase).abs() < 1e-2,
            "{report}"
        );
        assert_eq!(report["unit"], "dB");
        assert_eq!(report["at_hz"], f);
        // At 500 kHz the principal phase has wrapped past -180, and this has not.
        let report = measure(
            &response,
            &request("value_at", json!({ "at_hz": 5e5 })),
            false,
        );
        assert!(
            report["phase_degrees"].as_f64().unwrap() < -180.0,
            "{report}"
        );
    }

    #[test]
    fn a_frequency_outside_the_sweep_has_no_value() {
        let response = response_of(&sweep(20, 10.0, 1e4), true);
        for at in [1.0, 1e6] {
            let report = measure(
                &response,
                &request("value_at", json!({ "at_hz": at })),
                false,
            );
            assert!(report["value"].is_null());
            assert_eq!(report["finding"], "outside_data");
        }
    }

    #[test]
    fn a_ratio_of_two_signals_is_worked_out_as_complex_numbers() {
        let (real, imaginary) = divide((&[3.0, 0.0], &[4.0, 2.0]), (&[1.0, 0.0], &[1.0, 1.0]));
        // (3 + 4j)/(1 + j) = 3.5 + 0.5j, and 2j/j = 2.
        assert!((real[0] - 3.5).abs() < 1e-12 && (imaginary[0] - 0.5).abs() < 1e-12);
        assert!((real[1] - 2.0).abs() < 1e-12 && imaginary[1].abs() < 1e-12);
    }

    #[test]
    fn a_window_chooses_which_crossing_is_measured() {
        // Two resonances: the gain falls through unity twice.
        let freq = sweep(100, 1.0, 1e4);
        let gain = |f: f64| -> f64 {
            let a = 3.0 * (-(f.log10() - 1.0).powi(2) / 0.1).exp();
            let b = 3.0 * (-(f.log10() - 3.0).powi(2) / 0.1).exp();
            0.01 + a + b
        };
        let real: Vec<f64> = freq.iter().map(|&f| gain(f)).collect();
        let imaginary = vec![0.0; freq.len()];
        let response = Response::new(&freq, &real, &imaginary, true).unwrap();
        let first = measure(&response, &request("phase_margin", json!({})), false);
        let second = measure(
            &response,
            &request("phase_margin", json!({ "from_hz": 300.0, "to_hz": 1e4 })),
            false,
        );
        let early = first["crossover_hz"].as_f64().unwrap();
        let late = second["crossover_hz"].as_f64().unwrap();
        assert!(early < 100.0 && late > 1000.0, "{early} {late}");
    }

    #[test]
    fn a_bandpass_reports_both_edges_and_its_width() {
        // A resonance at 1 kHz with Q = 5: the edges are 1 kHz times
        // sqrt(1 + 1/(4 Q^2)) -/+ 1/(2 Q), and the width is 1 kHz / Q.
        let q = 5.0;
        let freq = sweep(400, 10.0, 1e5);
        let (mut real, mut imaginary) = (Vec::new(), Vec::new());
        for &f in &freq {
            let x = f / 1000.0;
            // H = (j x / Q) / (1 - x^2 + j x / Q)
            let (nr, ni) = (0.0, x / q);
            let (dr, di) = (1.0 - x * x, x / q);
            let power = dr * dr + di * di;
            real.push((nr * dr + ni * di) / power);
            imaginary.push((ni * dr - nr * di) / power);
        }
        let response = Response::new(&freq, &real, &imaginary, true).unwrap();
        let report = measure(
            &response,
            &request("bandwidth_3db", json!({ "reference_level": "peak" })),
            false,
        );
        let half = 1.0 / (2.0 * q);
        let root = (1.0 + half * half).sqrt();
        let (lower, upper) = (1000.0 * (root - half), 1000.0 * (root + half));
        assert!(
            (report["lower_edge_hz"].as_f64().unwrap() / lower - 1.0).abs() < 1e-3,
            "{report}"
        );
        assert!(
            (report["upper_edge_hz"].as_f64().unwrap() / upper - 1.0).abs() < 1e-3,
            "{report}"
        );
        assert!((value_of(&report) / 200.0 - 1.0).abs() < 1e-3, "{report}");
        assert!((report["peak_hz"].as_f64().unwrap() / 1000.0 - 1.0).abs() < 0.01);
    }

    #[test]
    fn a_low_pass_has_no_lower_edge_and_so_no_width_from_its_peak() {
        let response = response_of(&sweep(100, 1.0, 1e6), true);
        let report = measure(
            &response,
            &request("bandwidth_3db", json!({ "reference_level": "peak" })),
            false,
        );
        assert!(report["value"].is_null());
        assert_eq!(report["finding"], "level_not_reached");
        assert!(report["lower_edge_hz"].is_null());
        assert!(report["upper_edge_hz"].as_f64().is_some());
    }

    #[test]
    fn a_linear_sweep_is_interpolated_against_frequency() {
        // Gain falls 1 dB per hertz: the -3.01 dB point is at 3.0103 Hz.
        let freq: Vec<f64> = (0..=10).map(f64::from).collect();
        let real: Vec<f64> = freq.iter().map(|f| 10f64.powf(-f / 20.0)).collect();
        let response = Response::new(&freq[1..], &real[1..], &vec![0.0; 10], false).unwrap();
        let report = measure(&response, &request("bandwidth_3db", json!({})), false);
        // From the lowest frequency (1 Hz, -1 dB): the edge is -4.0103 dB, 4.0103 Hz.
        assert!((value_of(&report) - 4.0103).abs() < 1e-4, "{report}");
    }

    #[test]
    fn a_zero_response_is_a_finding_and_never_a_number() {
        let freq = sweep(10, 1.0, 100.0);
        let zeros = vec![0.0; freq.len()];
        let response = Response::new(&freq, &zeros, &zeros, true).unwrap();
        for kind in ["bandwidth_3db", "phase_margin", "gain_margin"] {
            let report = measure(&response, &request(kind, json!({})), false);
            assert!(report["value"].is_null(), "{kind}");
            assert_eq!(report["finding"], "non_finite_data", "{kind}");
        }
    }

    #[test]
    fn a_loop_that_starts_past_180_degrees_has_no_gain_margin_from_its_start() {
        // Two integrators, 1/(jw)^2: -180 degrees from the first sample. The sign of
        // the imaginary part, which the engine leaves to rounding, picks the side.
        let freq = sweep(10, 1.0, 100.0);
        let real: Vec<f64> = freq.iter().map(|f| -1.0 / (f * f)).collect();
        let tiny = vec![-1e-12; freq.len()];
        let response = Response::new(&freq, &real, &tiny, true).unwrap();
        let report = measure(&response, &request("gain_margin", json!({})), false);
        assert_eq!(report["finding"], "starts_on_level");
        assert!(report["start_phase_degrees"].as_f64().unwrap().abs() > 179.0);
    }

    #[test]
    fn the_phase_start_is_shown_because_a_sign_error_cannot_be_seen_otherwise() {
        let response = response_of(&sweep(100, 1.0, 1e6), true);
        let report = measure(&response, &request("phase_margin", json!({})), false);
        assert!(
            report["start_phase_degrees"].as_f64().unwrap().abs() < 5.0,
            "{report}"
        );
    }

    #[test]
    fn a_summary_gives_the_peak_and_both_ends_in_db() {
        let freq = sweep(50, 1.0, 1e4);
        let real: Vec<f64> = freq.iter().map(|&f| three_pole(f).0).collect();
        let summary = summary(&freq, &real, &vec![0.0; freq.len()], "v(out)", false);
        assert_eq!(summary["unit"], "dB");
        assert!((summary["peak_gain_db"].as_f64().unwrap() - 40.0).abs() < 0.01);
        assert_eq!(summary["peak_at_hz"], 1.0);
        assert!((summary["low_frequency_gain_db"].as_f64().unwrap() - 40.0).abs() < 0.01);
        assert!(summary["high_frequency_gain_db"].as_f64().unwrap() < 0.0);
        assert_eq!(summary["window_truncated"], false);
        assert!(summary.get("finding").is_none());
        let zero = super::summary(&[1.0, 2.0], &[0.0, 1.0], &[0.0, 0.0], "v(a)", true);
        assert_eq!(zero["finding"], "non_finite_data");
        assert_eq!(zero["window_truncated"], true);
    }

    #[test]
    fn mismatched_or_empty_data_is_no_response() {
        assert!(Response::new(&[], &[], &[], true).is_none());
        assert!(Response::new(&[1.0, 2.0], &[1.0], &[0.0], true).is_none());
    }
}
