//! Measurements on parsed vectors.
//!
//! Adaptive steps cluster on edges, so nothing here may weigh samples equally:
//! a plain average of them read 5.30 where the true RMS is 2.69. A signal is
//! taken as straight lines between its samples, which is also what lets a window
//! start and end between two of them.

use super::signals::Signal;
use serde_json::{json, Value};

/// Relative slack on the end of a window: the engine stops within rounding of
/// the stop time, not on it.
const WINDOW_TOLERANCE: f64 = 1e-9;
/// A step smaller than this share of the signal's size is rounding noise, and so
/// is anything under the engine's own current tolerance.
const NOISE_SHARE: f64 = 1e-9;
const NOISE_FLOOR: f64 = 1e-12;
/// With no `final_value`, the window's last value stands in for it. That holds
/// only if the signal has stopped: this last share of the window must stay within
/// `TAIL_STEADY_SHARE` of the step, its two halves must average within
/// `STILL_SHARE` of the step, so ripple does not count and a slow tail does, and
/// the signal must stay within `MAX_EXCURSION` steps of its start, which a ring
/// does and a pulse that came back does not. The result always shows the levels
/// it used, because no test can see every window that ends on the wrong value.
const TAIL_SHARE: f64 = 0.1;
const TAIL_STEADY_SHARE: f64 = 0.05;
const STILL_SHARE: f64 = 1e-3;
const MAX_EXCURSION: f64 = 3.0;
/// For settling, the end of the window may move this share of the band, which is
/// what keeps the time it reports close to the true one.
const BAND_STILL_SHARE: f64 = 0.05;
const RISE_LOW: f64 = 0.1;
const RISE_HIGH: f64 = 0.9;
const DEFAULT_BAND_PERCENT: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Min,
    Max,
    PeakToPeak,
    PeakAbs,
    Average,
    Rms,
    Integral,
    I2t,
    ValueAt,
    Crossing,
    RiseTime,
    FallTime,
    OvershootPercent,
    SettlingTime,
}

/// Every kind, as the schema and the result name it.
pub(crate) const KINDS: [(&str, Kind); 14] = [
    ("min", Kind::Min),
    ("max", Kind::Max),
    ("peak_to_peak", Kind::PeakToPeak),
    ("peak_abs", Kind::PeakAbs),
    ("average", Kind::Average),
    ("rms", Kind::Rms),
    ("integral", Kind::Integral),
    ("i2t", Kind::I2t),
    ("value_at", Kind::ValueAt),
    ("crossing", Kind::Crossing),
    ("rise_time", Kind::RiseTime),
    ("fall_time", Kind::FallTime),
    ("overshoot_percent", Kind::OvershootPercent),
    ("settling_time", Kind::SettlingTime),
];

impl Kind {
    fn takes_level(self) -> bool {
        self == Self::Crossing
    }

    fn takes_final_value(self) -> bool {
        matches!(
            self,
            Self::RiseTime | Self::FallTime | Self::OvershootPercent | Self::SettlingTime
        )
    }

    fn unit(self, signal_unit: &str) -> String {
        match self {
            Self::Integral => format!("{signal_unit}*s"),
            Self::I2t => "A²s".to_string(),
            Self::Crossing | Self::RiseTime | Self::FallTime | Self::SettlingTime => {
                "s".to_string()
            }
            Self::OvershootPercent => "%".to_string(),
            _ => signal_unit.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    Rising,
    Falling,
    Either,
}

impl Direction {
    fn allows(self, rising: bool) -> bool {
        match self {
            Self::Rising => rising,
            Self::Falling => !rising,
            Self::Either => true,
        }
    }
}

/// What to measure, checked against the analysis before anything runs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Measurement {
    pub name: String,
    pub kind: Kind,
    pub kind_name: &'static str,
    /// As the caller wrote it, to repeat in the result.
    pub signal_text: String,
    pub signal: Signal,
    pub from: Option<f64>,
    pub to: Option<f64>,
    pub at: Option<f64>,
    pub level: Option<f64>,
    pub direction: Direction,
    pub final_value: Option<f64>,
    pub band_percent: f64,
    /// Where the run starts saving, so a window reaching before it can say so.
    pub saved_from: f64,
}

/// A measurement that does not fit, naming the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BadMeasurement {
    pub field: String,
    pub reason: String,
}

fn finite(entry: &Value, key: &str, prefix: &str) -> Result<Option<f64>, BadMeasurement> {
    let value = &entry[key];
    if value.is_null() {
        return Ok(None);
    }
    match value.as_f64() {
        Some(number) if number.is_finite() => Ok(Some(number)),
        _ => Err(BadMeasurement {
            field: format!("{prefix}.{key}"),
            reason: format!("{key} must be a finite number"),
        }),
    }
}

/// Read one entry of `measurements`, and refuse what could not give an answer.
pub(crate) fn parse(
    entry: &Value,
    index: usize,
    stop: f64,
    saved_from: f64,
) -> Result<Measurement, BadMeasurement> {
    let prefix = format!("measurements[{index}]");
    let bad = |key: &str, reason: String| BadMeasurement {
        field: format!("{prefix}.{key}"),
        reason,
    };
    let kind_name = entry["kind"].as_str().unwrap_or_default();
    let Some(&(kind_name, kind)) = KINDS.iter().find(|(name, _)| *name == kind_name) else {
        return Err(bad(
            "kind",
            format!("Unknown measurement kind: {kind_name}"),
        ));
    };
    let signal_text = entry["signal"].as_str().unwrap_or_default().to_string();
    let signal = Signal::parse(&signal_text).map_err(|reason| bad("signal", reason))?;
    if kind == Kind::I2t && !matches!(signal, Signal::Current { .. }) {
        return Err(bad("signal", "i2t needs a current".to_string()));
    }
    let from = finite(entry, "from_seconds", &prefix)?;
    let to = finite(entry, "to_seconds", &prefix)?;
    let at = finite(entry, "at_seconds", &prefix)?;
    let level = finite(entry, "level", &prefix)?;
    let final_value = finite(entry, "final_value", &prefix)?;
    let band_percent = finite(entry, "band_percent", &prefix)?;
    for (key, time) in [
        ("from_seconds", from),
        ("to_seconds", to),
        ("at_seconds", at),
    ] {
        if time.is_some_and(|time| !(0.0..=stop).contains(&time)) {
            return Err(bad(key, format!("{key} must fall within the analysis")));
        }
    }
    if let (Some(from), Some(to)) = (from, to) {
        if from >= to {
            return Err(bad(
                "to_seconds",
                "to_seconds must follow from_seconds".into(),
            ));
        }
    }
    if band_percent.is_some_and(|band| band <= 0.0) {
        return Err(bad("band_percent", "band_percent must be above 0".into()));
    }

    let direction_text = entry["direction"].as_str();
    let direction = match direction_text {
        None | Some("either") => Direction::Either,
        Some("rising") => Direction::Rising,
        Some("falling") => Direction::Falling,
        Some(other) => return Err(bad("direction", format!("Unknown direction: {other}"))),
    };

    // A field that would be ignored must not look as if it had an effect.
    let needs = |present: bool, key: &str| match present {
        true => Ok(()),
        false => Err(bad(key, format!("{kind_name} needs {key}"))),
    };
    let unused = |present: bool, key: &str| match present {
        true => Err(bad(key, format!("{kind_name} does not use {key}"))),
        false => Ok(()),
    };
    let point_in_time = kind == Kind::ValueAt;
    if point_in_time {
        needs(at.is_some(), "at_seconds")?;
        unused(from.is_some(), "from_seconds")?;
        unused(to.is_some(), "to_seconds")?;
    } else {
        unused(at.is_some(), "at_seconds")?;
    }
    if kind.takes_level() {
        needs(level.is_some(), "level")?;
    } else {
        unused(level.is_some(), "level")?;
        unused(direction_text.is_some(), "direction")?;
    }
    if !kind.takes_final_value() {
        unused(final_value.is_some(), "final_value")?;
    }
    if kind != Kind::SettlingTime {
        unused(band_percent.is_some(), "band_percent")?;
    }

    let name = match entry["name"].as_str() {
        Some(name) => name.to_string(),
        None => format!("{kind_name} {signal_text}"),
    };
    Ok(Measurement {
        name,
        kind,
        kind_name,
        signal_text,
        signal,
        from,
        to,
        at,
        level,
        direction,
        final_value,
        band_percent: band_percent.unwrap_or(DEFAULT_BAND_PERCENT),
        saved_from,
    })
}

/// An answer. A value that does not exist is `None`, with the reason in
/// `finding`: that is an engineering result, not a failed check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Reading {
    pub value: Option<f64>,
    pub at: Option<f64>,
    pub finding: Option<&'static str>,
    /// The window asked for reaches past the data.
    pub clipped: bool,
    /// The start and final levels a step was measured between.
    pub levels: Option<(f64, f64)>,
}

impl Reading {
    fn found(value: f64) -> Self {
        Self {
            value: Some(value),
            at: None,
            finding: None,
            clipped: false,
            levels: None,
        }
    }

    fn found_at(value: f64, at: f64) -> Self {
        Self {
            at: Some(at),
            ..Self::found(value)
        }
    }

    fn none(finding: &'static str) -> Self {
        Self {
            value: None,
            at: None,
            finding: Some(finding),
            clipped: false,
            levels: None,
        }
    }

    fn between(self, start: f64, end: f64) -> Self {
        Self {
            levels: Some((start, end)),
            ..self
        }
    }

    /// One entry of `measurements` in a result.
    pub(crate) fn report(&self, request: &Measurement, window_truncated: bool) -> Value {
        let mut report = json!({
            "name": request.name,
            "kind": request.kind_name,
            "signal": request.signal_text,
            "value": self.value,
            "unit": request.kind.unit(request.signal.unit()),
            "window_truncated": window_truncated || self.clipped,
        });
        if let Some(at) = self.at {
            report["at_seconds"] = json!(at);
        }
        if let Some(finding) = self.finding {
            report["finding"] = json!(finding);
        }
        if let Some((start, end)) = self.levels {
            report["start_value"] = json!(start);
            report["final_value"] = json!(end);
        }
        report
    }
}

/// A measurement of a signal that never came back from the engine.
pub(crate) fn missing_report(request: &Measurement, window_truncated: bool) -> Value {
    Reading::none("signal_not_found").report(request, window_truncated)
}

/// The straight-line value at `at`, which must lie within `time`.
fn interpolate(time: &[f64], values: &[f64], at: f64) -> f64 {
    let after = time.partition_point(|&sample| sample < at);
    if after == 0 {
        return values[0];
    }
    if after == time.len() {
        return values[after - 1];
    }
    // `time[after - 1] < at <= time[after]`, so the span is never zero.
    let (t0, t1) = (time[after - 1], time[after]);
    values[after - 1] + (values[after] - values[after - 1]) * (at - t0) / (t1 - t0)
}

/// The time between two samples where a straight line reaches `level`.
fn time_of(t0: f64, y0: f64, t1: f64, y1: f64, level: f64) -> f64 {
    if y1 == y0 {
        return t0;
    }
    t0 + (t1 - t0) * (level - y0) / (y1 - y0)
}

/// The samples of a window, with its ends placed by interpolation.
struct Window {
    time: Vec<f64>,
    values: Vec<f64>,
    clipped: bool,
}

impl Window {
    fn cut(
        time: &[f64],
        values: &[f64],
        from: Option<f64>,
        to: Option<f64>,
        saved_from: f64,
    ) -> Option<Self> {
        let (&first, &last) = (time.first()?, time.last()?);
        let slack = last.abs().max(first.abs()) * WINDOW_TOLERANCE;
        let low = from.unwrap_or(first).max(first);
        let high = to.unwrap_or(last).min(last);
        if low >= high {
            return None;
        }
        // The first saved point lands a little after `saved_from`, so a window from
        // there has lost nothing.
        let clipped = from.is_some_and(|from| from < first - slack && from < saved_from)
            || to.is_some_and(|to| to > last + slack);
        let start = time.partition_point(|&sample| sample <= low);
        let end = time.partition_point(|&sample| sample < high);
        let inner = start..end.max(start);
        let mut cut_time = Vec::with_capacity(inner.len() + 2);
        let mut cut_values = Vec::with_capacity(inner.len() + 2);
        cut_time.push(low);
        cut_values.push(interpolate(time, values, low));
        cut_time.extend_from_slice(&time[inner.clone()]);
        cut_values.extend_from_slice(&values[inner]);
        cut_time.push(high);
        cut_values.push(interpolate(time, values, high));
        Some(Self {
            time: cut_time,
            values: cut_values,
            clipped,
        })
    }

    fn duration(&self) -> f64 {
        self.time[self.time.len() - 1] - self.time[0]
    }

    fn pairs(&self) -> impl Iterator<Item = (f64, f64, f64, f64)> + '_ {
        let time = self.time.windows(2);
        let values = self.values.windows(2);
        time.zip(values).map(|(t, y)| (t[0], y[0], t[1], y[1]))
    }

    /// The integral of the signal over time.
    fn integrate(&self) -> f64 {
        self.pairs()
            .map(|(t0, y0, t1, y1)| (t1 - t0) * (y0 + y1) / 2.0)
            .sum()
    }

    /// The integral of the signal squared. The trapezoid of the squares would
    /// read high on every slope: 6.4 percent on a triangle.
    fn integrate_squared(&self) -> f64 {
        self.pairs()
            .map(|(t0, y0, t1, y1)| (t1 - t0) * (y0 * y0 + y0 * y1 + y1 * y1) / 3.0)
            .sum()
    }

    /// The first sample that `better` prefers over all the others.
    fn extreme(&self, better: impl Fn(f64, f64) -> bool) -> (f64, f64) {
        let samples = self.time.iter().zip(&self.values);
        let mut best = (self.values[0], self.time[0]);
        for (&time, &value) in samples {
            if better(value, best.0) {
                best = (value, time);
            }
        }
        best
    }

    fn first_crossing(&self, level: f64, direction: Direction) -> Option<f64> {
        self.pairs().find_map(|(t0, y0, t1, y1)| {
            let rising = y0 < level && y1 >= level;
            let falling = y0 > level && y1 <= level;
            let crossed =
                (rising && direction.allows(true)) || (falling && direction.allows(false));
            crossed.then(|| time_of(t0, y0, t1, y1, level))
        })
    }

    /// Between `final` and where the window starts, the step this window holds.
    fn step(&self, final_value: Option<f64>) -> (f64, f64) {
        let start = self.values[0];
        let end = final_value.unwrap_or(self.values[self.values.len() - 1]);
        (start, end)
    }

    /// The window's last stretch, starting between samples if it must.
    fn tail(&self) -> impl Iterator<Item = f64> + '_ {
        let last = self.time[self.time.len() - 1];
        let from = last - self.duration() * TAIL_SHARE;
        let first = interpolate(&self.time, &self.values, from);
        let rest = self.time.partition_point(|&time| time < from);
        std::iter::once(first).chain(self.values[rest..].iter().copied())
    }

    /// The time-weighted mean between two times in the window.
    fn mean(&self, from: f64, to: f64) -> f64 {
        let part = Self::cut(&self.time, &self.values, Some(from), Some(to), 0.0);
        part.map_or(0.0, |part| part.integrate() / part.duration())
    }

    /// How far the level moved across the tail, half against half.
    fn drift(&self) -> f64 {
        let last = self.time[self.time.len() - 1];
        let tail = self.duration() * TAIL_SHARE;
        let middle = last - tail / 2.0;
        (self.mean(last - tail, middle) - self.mean(middle, last)).abs()
    }

    fn noise(&self, end: f64) -> f64 {
        let size = self
            .values
            .iter()
            .fold(end.abs(), |size, y| size.max(y.abs()));
        (size * NOISE_SHARE).max(NOISE_FLOOR)
    }

    fn excursion(&self) -> f64 {
        let start = self.values[0];
        self.values
            .iter()
            .fold(0.0, |far: f64, y| far.max((y - start).abs()))
    }

    /// The step to measure, or the finding that says there is none to trust.
    fn transition(&self, final_value: Option<f64>) -> Result<(f64, f64), &'static str> {
        let (start, end) = self.step(final_value);
        let step = (end - start).abs();
        let noise = self.noise(end);
        if final_value.is_some() {
            return if step > noise {
                Ok((start, end))
            } else {
                Err("no_transition")
            };
        }
        let excursion = self.excursion();
        if excursion <= noise {
            return Err("no_transition");
        }
        let unsteady = self
            .tail()
            .any(|y| (y - end).abs() > step * TAIL_STEADY_SHARE);
        if unsteady || self.drift() > step * STILL_SHARE || excursion > step * MAX_EXCURSION {
            return Err("final_value_needed");
        }
        Ok((start, end))
    }

    /// The 10 to 90 percent time of the edge that first reaches 90 percent. The
    /// low end is taken as the last time the signal left it, so ringing before
    /// the edge does not count.
    fn edge(&self, final_value: Option<f64>, rising: bool) -> Reading {
        let (start, end) = match self.transition(final_value) {
            Ok(levels) => levels,
            Err(finding) => return Reading::none(finding),
        };
        let step = end - start;
        if (step > 0.0) != rising {
            return Reading::none("no_transition");
        }
        let progress: Vec<f64> = self.values.iter().map(|y| (y - start) / step).collect();
        let Some(top) = progress.iter().position(|&part| part >= RISE_HIGH) else {
            return Reading::none("level_not_reached");
        };
        let high_time = time_of(
            self.time[top - 1],
            progress[top - 1],
            self.time[top],
            progress[top],
            RISE_HIGH,
        );
        let left = (1..=top)
            .rev()
            .find(|&index| progress[index - 1] < RISE_LOW && progress[index] >= RISE_LOW);
        let Some(left) = left else {
            return Reading::none("level_not_reached");
        };
        let low_time = time_of(
            self.time[left - 1],
            progress[left - 1],
            self.time[left],
            progress[left],
            RISE_LOW,
        );
        Reading::found_at(high_time - low_time, high_time).between(start, end)
    }

    /// How far past the final value the signal goes, as a share of the step. A
    /// falling step overshoots downward.
    fn overshoot(&self, final_value: Option<f64>) -> Reading {
        let (start, end) = match self.transition(final_value) {
            Ok(levels) => levels,
            Err(finding) => return Reading::none(finding),
        };
        let step = end - start;
        let (peak, at) = if step > 0.0 {
            self.extreme(|value, best| value > best)
        } else {
            self.extreme(|value, best| value < best)
        };
        let beyond = if step > 0.0 { peak - end } else { end - peak };
        Reading::found_at((beyond.max(0.0)) / step.abs() * 100.0, at).between(start, end)
    }

    /// When the signal last enters the band and stays. The band is a share of
    /// the final value or of the step, whichever is larger: a rail has a large
    /// final value and a small step, a discharge the reverse, and a share of a
    /// final value of zero would be no band at all.
    fn settling(&self, final_value: Option<f64>, band_percent: f64) -> Reading {
        let (start, end) = self.step(final_value);
        let step = (end - start).abs();
        let band = end.abs().max(step) * band_percent / 100.0;
        let noise = self.noise(end);
        if final_value.is_some() {
            if band <= noise {
                return Reading::none("no_transition");
            }
        } else {
            if self.excursion() <= noise {
                return Reading::none("no_transition");
            }
            let allowed = (step * STILL_SHARE).max(band * BAND_STILL_SHARE);
            if band <= noise || self.drift() > allowed {
                return Reading::none("final_value_needed");
            }
        }
        let outside = |value: f64| (value - end).abs() > band;
        // The last value is always inside a band around itself, so with no given
        // final value the end of the window has to show it has settled.
        if final_value.is_none() && self.tail().any(outside) {
            return Reading::none("not_settled");
        }
        let last = self.values.iter().rposition(|&value| outside(value));
        let Some(last) = last else {
            return Reading::found_at(0.0, self.time[0]).between(start, end);
        };
        if last + 1 == self.values.len() {
            return Reading::none("not_settled");
        }
        let (before, after) = (self.values[last], self.values[last + 1]);
        let edge = end + band.copysign(before - end);
        let time = time_of(self.time[last], before, self.time[last + 1], after, edge);
        Reading::found_at(time - self.time[0], time).between(start, end)
    }
}

/// Measure one signal. `time` and `values` are the whole run.
pub(crate) fn measure(time: &[f64], values: &[f64], request: &Measurement) -> Reading {
    if time.len() != values.len() || time.is_empty() {
        return Reading::none("outside_data");
    }
    if request.kind == Kind::ValueAt {
        let at = request.at.unwrap_or_default();
        let last = time[time.len() - 1];
        let inside = at >= time[0] && at <= last + last.abs() * WINDOW_TOLERANCE;
        if !inside {
            return Reading::none("outside_data");
        }
        let value = interpolate(time, values, at);
        return match value.is_finite() {
            true => Reading::found_at(value, at),
            false => Reading::none("non_finite_data"),
        };
    }
    let Some(window) = Window::cut(time, values, request.from, request.to, request.saved_from)
    else {
        return Reading::none("outside_data");
    };
    if !window.values.iter().all(|value| value.is_finite()) {
        return Reading::none("non_finite_data");
    }
    let reading = over_window(&window, request);
    // Finite samples can still square to infinity.
    if reading.value.is_some_and(|value| !value.is_finite()) {
        return Reading::none("non_finite_data");
    }
    Reading {
        clipped: window.clipped,
        ..reading
    }
}

fn over_window(window: &Window, request: &Measurement) -> Reading {
    let duration = window.duration();
    match request.kind {
        Kind::Min => {
            let (value, at) = window.extreme(|value, best| value < best);
            Reading::found_at(value, at)
        }
        Kind::Max => {
            let (value, at) = window.extreme(|value, best| value > best);
            Reading::found_at(value, at)
        }
        Kind::PeakToPeak => {
            let (low, _) = window.extreme(|value, best| value < best);
            let (high, _) = window.extreme(|value, best| value > best);
            Reading::found(high - low)
        }
        Kind::PeakAbs => {
            let (value, at) = window.extreme(|value, best| value.abs() > best.abs());
            Reading::found_at(value, at)
        }
        Kind::Average => Reading::found(window.integrate() / duration),
        Kind::Rms => Reading::found((window.integrate_squared() / duration).sqrt()),
        Kind::Integral => Reading::found(window.integrate()),
        Kind::I2t => Reading::found(window.integrate_squared()),
        Kind::Crossing => {
            let level = request.level.unwrap_or_default();
            match window.first_crossing(level, request.direction) {
                Some(at) => Reading::found(at),
                None if window.values[0] == level => Reading::none("starts_on_level"),
                None => Reading::none("level_not_reached"),
            }
        }
        Kind::RiseTime => window.edge(request.final_value, true),
        Kind::FallTime => window.edge(request.final_value, false),
        Kind::OvershootPercent => window.overshoot(request.final_value),
        Kind::SettlingTime => window.settling(request.final_value, request.band_percent),
        Kind::ValueAt => Reading::none("outside_data"),
    }
}

/// Lowest, highest and last value of a whole signal, with an optional envelope:
/// evenly spaced buckets over time keeping each one's extremes, so a narrow spike
/// survives where thinning the samples would lose it.
pub(crate) fn summary(
    time: &[f64],
    values: &[f64],
    signal_text: &str,
    unit: &str,
    envelope_points: usize,
    window_truncated: bool,
) -> Value {
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    let low = finite.iter().copied().reduce(f64::min);
    let high = finite.iter().copied().reduce(f64::max);
    let mut report = json!({
        "signal": signal_text,
        "unit": unit,
        "min": low,
        "max": high,
        "final": values.last().copied().filter(|value| value.is_finite()),
        "window_truncated": window_truncated,
    });
    if finite.len() != values.len() {
        report["finding"] = json!("non_finite_data");
    }
    if envelope_points > 0 {
        report["envelope"] = envelope(time, values, envelope_points);
    }
    report
}

fn envelope(time: &[f64], values: &[f64], buckets: usize) -> Value {
    let (mut x, mut low, mut high) = (Vec::new(), Vec::new(), Vec::new());
    let (Some(&first), Some(&last)) = (time.first(), time.last()) else {
        return json!({ "x": x, "min": low, "max": high });
    };
    let width = (last - first) / buckets as f64;
    for bucket in 0..buckets {
        let start = first + width * bucket as f64;
        let end = start + width;
        // Each bucket holds its own edges, so one with no sample is not empty.
        let inner = time.partition_point(|&t| t < start)..time.partition_point(|&t| t <= end);
        let inside = values[inner].iter().copied();
        let edges = [
            interpolate(time, values, start),
            interpolate(time, values, end),
        ];
        let samples = || {
            inside
                .clone()
                .chain(edges)
                .filter(|value| value.is_finite())
        };
        x.push(start + width / 2.0);
        low.push(samples().reduce(f64::min));
        high.push(samples().reduce(f64::max));
        if width == 0.0 {
            break;
        }
    }
    json!({ "x": x, "min": low, "max": high })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(kind: &str) -> Value {
        json!({ "kind": kind, "signal": "v(a)" })
    }

    fn with(mut base: Value, key: &str, value: Value) -> Value {
        base[key] = value;
        base
    }

    fn read(entry: &Value, time: &[f64], values: &[f64]) -> Reading {
        measure(time, values, &parse(entry, 0, 1e9, 0.0).unwrap())
    }

    fn close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} is not {expected} within {tolerance}"
        );
    }

    /// Fine steps on the first stretch, as adaptive steps cluster on an edge.
    fn clustered(stop: f64, edge: f64) -> Vec<f64> {
        let fine = (0..=4000).map(|n| edge * f64::from(n) / 4000.0);
        let coarse = (1..=400).map(|n| edge + (stop - edge) * f64::from(n) / 400.0);
        fine.chain(coarse).collect()
    }

    #[test]
    fn an_rc_charge_matches_its_closed_form() {
        // v = 5(1 - e^(-t/tau)) with tau = 1 ms: 10 to 90 percent is tau ln 9, and
        // the area to 5 tau is 5(5 tau - tau(1 - e^-5)).
        let tau = 1e-3;
        let stop = 12.0 * tau;
        let time: Vec<f64> = (0..=240_000)
            .map(|n| stop * f64::from(n) / 240_000.0)
            .collect();
        let charge = |t: f64| 5.0 * (1.0 - (-t / tau).exp());
        let values: Vec<f64> = time.iter().map(|&t| charge(t)).collect();
        let end = charge(stop);

        let settled = with(request("rise_time"), "final_value", json!(5.0));
        let rise = read(&settled, &time, &values).value.unwrap();
        close(rise, tau * 9f64.ln(), 1e-8);
        let crossing = with(request("crossing"), "level", json!(2.5));
        let half = read(&crossing, &time, &values).value.unwrap();
        // The level is reached at tau ln 2.
        close(half, tau * 2f64.ln(), 1e-9);
        let area = with(request("integral"), "to_seconds", json!(5.0 * tau));
        let expected = 5.0 * (5.0 * tau - tau * (1.0 - (-5f64).exp()));
        close(read(&area, &time, &values).value.unwrap(), expected, 1e-9);

        let settle = read(&request("settling_time"), &time, &values);
        // |v - end| = 0.02 end means 5 e^(-t/tau) = 5 e^(-12) + 0.02 end.
        let exact = -tau * ((5.0 * (-12f64).exp() + 0.02 * end) / 5.0).ln();
        close(settle.value.unwrap(), exact, 1e-8);
        close(settle.at.unwrap(), exact, 1e-8);
    }

    /// A unit step into a second-order system with damping 0.2 at 1 krad/s.
    fn second_order() -> (Vec<f64>, Vec<f64>) {
        let (zeta, omega) = (0.2_f64, 1000.0_f64);
        let damped = omega * (1.0 - zeta * zeta).sqrt();
        let stop = 0.05;
        let time: Vec<f64> = (0..=200_000)
            .map(|n| stop * f64::from(n) / 200_000.0)
            .collect();
        let step = |t: f64| {
            let decay = (-zeta * omega * t).exp();
            1.0 - decay * ((damped * t).cos() + zeta * omega / damped * (damped * t).sin())
        };
        let values = time.iter().map(|&t| step(t)).collect();
        (time, values)
    }

    #[test]
    fn a_ringing_step_matches_its_closed_form() {
        let (time, values) = second_order();
        let zeta = 0.2_f64;
        let damped = 1000.0 * (1.0 - zeta * zeta).sqrt();
        let final_one = with(request("overshoot_percent"), "final_value", json!(1.0));
        let overshoot = read(&final_one, &time, &values);
        let expected = (-std::f64::consts::PI * zeta / (1.0 - zeta * zeta).sqrt()).exp() * 100.0;
        close(overshoot.value.unwrap(), expected, 1e-4);
        // The peak falls at pi / damped frequency.
        close(overshoot.at.unwrap(), std::f64::consts::PI / damped, 1e-6);

        // The last exit from a 2 percent band, solved apart from this code by
        // bisecting the closed form in Python: 0.0196019037 s.
        let settle = with(request("settling_time"), "final_value", json!(1.0));
        close(
            read(&settle, &time, &values).value.unwrap(),
            0.0196019037,
            1e-7,
        );
    }

    #[test]
    fn a_rise_ignores_ringing_before_the_edge_it_measures() {
        let time = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        // Up past 10 percent, back down, then the real edge.
        let values = [0.0, 0.3, 0.0, 0.1, 0.5, 0.9, 1.0];
        let entry = with(request("rise_time"), "final_value", json!(1.0));
        let rise = read(&entry, &time, &values);
        close(rise.value.unwrap(), 2.0, 1e-12);
    }

    #[test]
    fn a_fall_is_measured_on_a_falling_step_and_a_rise_refuses_one() {
        let time = [0.0, 1.0, 2.0];
        let values = [10.0, 5.0, 0.0];
        let entry = with(request("fall_time"), "final_value", json!(0.0));
        close(read(&entry, &time, &values).value.unwrap(), 1.6, 1e-12);
        let entry = with(request("rise_time"), "final_value", json!(0.0));
        let rise = read(&entry, &time, &values);
        assert_eq!(rise.value, None);
        assert_eq!(rise.finding, Some("no_transition"));
    }

    #[test]
    fn a_falling_step_overshoots_downward() {
        let time = [0.0, 1.0, 2.0, 3.0];
        let values = [10.0, -2.0, 0.0, 0.0];
        let overshoot = read(&request("overshoot_percent"), &time, &values);
        close(overshoot.value.unwrap(), 20.0, 1e-12);
        close(overshoot.at.unwrap(), 1.0, 1e-12);
    }

    #[test]
    fn a_step_that_never_passes_its_final_value_has_no_overshoot() {
        let (time, values) = ([0.0, 1.0, 2.0], [0.0, 1.0, 1.0]);
        let overshoot = read(&request("overshoot_percent"), &time, &values);
        assert_eq!(overshoot.value, Some(0.0));
        assert_eq!(overshoot.finding, None);
    }

    #[test]
    fn a_step_that_stops_short_of_a_given_final_value_has_no_overshoot() {
        let (time, values) = ([0.0, 1.0], [0.0, 0.5]);
        let entry = with(request("overshoot_percent"), "final_value", json!(1.0));
        assert_eq!(read(&entry, &time, &values).value, Some(0.0));
    }

    #[test]
    fn a_rail_settles_against_its_final_value_and_a_discharge_against_its_step() {
        // A 12 V rail dipping 100 mV stays inside 2 percent of 12 V.
        let time = [0.0, 1.0, 2.0, 3.0];
        let rail = [12.0, 11.9, 11.95, 12.0];
        assert_eq!(
            read(&request("settling_time"), &time, &rail).value,
            Some(0.0)
        );
        // A signal decaying to zero has no band from its final value alone.
        let time = [0.0, 1.0, 2.0, 3.0, 20.0];
        let decay = [10.0, 1.0, 0.1, 0.0, 0.0];
        let settled = read(&request("settling_time"), &time, &decay);
        // Inside 0.2 V after 1 s, between 1 V and 0.1 V.
        close(settled.value.unwrap(), 1.0 + 0.8 / 0.9, 1e-12);
    }

    #[test]
    fn a_signal_still_outside_its_band_has_not_settled() {
        let (time, values) = ([0.0, 1.0], [0.0, 0.5]);
        let entry = with(request("settling_time"), "final_value", json!(1.0));
        let reading = read(&entry, &time, &values);
        assert_eq!(reading.value, None);
        assert_eq!(reading.finding, Some("not_settled"));
    }

    #[test]
    fn i2t_matches_the_analytic_value_of_a_decaying_current() {
        // i = 10 e^(-t/tau) has i2t = 100 tau / 2 (1 - e^(-2 T / tau)).
        let tau = 1e-3;
        let stop = 20.0 * tau;
        let time = clustered(stop, 5.0 * tau);
        let values: Vec<f64> = time.iter().map(|&t| 10.0 * (-t / tau).exp()).collect();
        let entry = json!({ "kind": "i2t", "signal": "i(r1)" });
        let expected = 100.0 * tau / 2.0 * (1.0 - (-40f64).exp());
        close(read(&entry, &time, &values).value.unwrap(), expected, 1e-5);
    }

    #[test]
    fn rms_weighs_time_and_never_samples() {
        // 10 for the first tenth, then 0: the RMS is 10 / sqrt(10) and the average
        // 1. Samples crowd the edge, as the engine's do, so a plain average of
        // them would read near 7.
        let mut time: Vec<f64> = (0..=1000).map(|n| 0.1 * f64::from(n) / 1000.0).collect();
        let mut values = vec![10.0; time.len()];
        for n in 1..=4 {
            time.push(0.1 + 0.9 * f64::from(n) / 4.0);
            values.push(0.0);
        }
        // Make the step sharp: the first zero sits just after the edge.
        time.insert(1001, 0.1 + 1e-9);
        values.insert(1001, 0.0);

        let naive: f64 = values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64;
        assert!(naive.sqrt() > 9.0, "{}", naive.sqrt());
        close(
            read(&request("rms"), &time, &values).value.unwrap(),
            10f64.sqrt(),
            1e-6,
        );
        close(
            read(&request("average"), &time, &values).value.unwrap(),
            1.0,
            1e-6,
        );
    }

    #[test]
    fn a_window_starts_and_ends_between_samples() {
        let (time, values) = ([0.0, 1.0, 2.0], [0.0, 10.0, 0.0]);
        let entry = json!({
            "kind": "integral", "signal": "v(a)", "from_seconds": 0.5, "to_seconds": 1.5
        });
        // Two trapezoids from 5 up to 10 and back.
        close(read(&entry, &time, &values).value.unwrap(), 7.5, 1e-12);
        let peak = json!({
            "kind": "max", "signal": "v(a)", "from_seconds": 1.5, "to_seconds": 2.0
        });
        let reading = read(&peak, &time, &values);
        assert_eq!(reading.value, Some(5.0));
        assert_eq!(reading.at, Some(1.5));
    }

    #[test]
    fn extremes_keep_their_sign_and_place() {
        let (time, values) = ([0.0, 1.0, 2.0, 3.0], [1.0, -7.0, 4.0, 2.0]);
        let at = |kind: &str| {
            let reading = read(&request(kind), &time, &values);
            (reading.value.unwrap(), reading.at)
        };
        assert_eq!(at("min"), (-7.0, Some(1.0)));
        assert_eq!(at("max"), (4.0, Some(2.0)));
        assert_eq!(at("peak_abs"), (-7.0, Some(1.0)));
        assert_eq!(at("peak_to_peak"), (11.0, None));
    }

    #[test]
    fn value_at_interpolates_and_says_when_it_has_no_data() {
        let (time, values) = ([1.0, 2.0], [10.0, 20.0]);
        let at = |seconds: f64| {
            read(
                &with(request("value_at"), "at_seconds", json!(seconds)),
                &time,
                &values,
            )
        };
        assert_eq!(at(1.5).value, Some(15.0));
        assert_eq!(at(2.0).value, Some(20.0));
        let outside = at(3.0);
        assert_eq!(outside.value, None);
        assert_eq!(outside.finding, Some("outside_data"));
    }

    #[test]
    fn crossings_respect_direction_and_a_level_never_reached_is_null_not_zero() {
        let (time, values) = ([0.0, 1.0, 2.0, 3.0], [0.0, 2.0, 0.0, 2.0]);
        let crossing = |direction: &str, level: f64| {
            let entry = with(request("crossing"), "level", json!(level));
            read(&with(entry, "direction", json!(direction)), &time, &values)
        };
        assert_eq!(crossing("rising", 1.0).value, Some(0.5));
        assert_eq!(crossing("falling", 1.0).value, Some(1.5));
        assert_eq!(crossing("either", 1.0).value, Some(0.5));
        let missed = crossing("rising", 5.0);
        assert_eq!(missed.value, None);
        assert_eq!(missed.finding, Some("level_not_reached"));
    }

    #[test]
    fn a_signal_that_starts_on_the_level_has_not_crossed_it() {
        let (time, values) = ([0.0, 1.0], [1.0, 2.0]);
        let entry = with(request("crossing"), "level", json!(1.0));
        assert_eq!(
            read(&entry, &time, &values).finding,
            Some("starts_on_level")
        );
    }

    #[test]
    fn a_rise_that_stops_short_of_ninety_percent_is_null() {
        let (time, values) = ([0.0, 1.0, 2.0], [0.0, 0.5, 0.4]);
        let entry = with(request("rise_time"), "final_value", json!(1.0));
        let reading = read(&entry, &time, &values);
        assert_eq!(reading.value, None);
        assert_eq!(reading.finding, Some("level_not_reached"));
    }

    #[test]
    fn a_flat_signal_has_no_transition() {
        let (time, values) = ([0.0, 1.0, 2.0], [3.0, 3.0 + 4e-15, 3.0 + 2e-15]);
        for kind in ["rise_time", "fall_time", "overshoot_percent"] {
            assert_eq!(
                read(&request(kind), &time, &values).finding,
                Some("no_transition")
            );
        }
        let (zero, flat) = ([0.0, 1.0], [0.0, 0.0]);
        let settle = read(&request("settling_time"), &zero, &flat);
        assert_eq!(settle.finding, Some("no_transition"));
    }

    /// The step in a window is 5 V to 5.001 V: a pulse that nearly came back.
    fn nearly_back() -> ([f64; 5], [f64; 5]) {
        ([0.0, 1.0, 2.0, 3.0, 4.0], [0.0, 5.0, 5.0, 0.001, 0.001])
    }

    #[test]
    fn a_pulse_that_came_back_gives_no_number_from_what_was_left_over() {
        let (time, values) = nearly_back();
        for kind in ["rise_time", "overshoot_percent"] {
            let reading = read(&request(kind), &time, &values);
            assert_eq!(reading.value, None, "{kind}");
            assert_eq!(reading.finding, Some("final_value_needed"), "{kind}");
        }
        let exact = [0.0, 5.0, 5.0, 0.0, 0.0];
        let reading = read(&request("rise_time"), &time, &exact);
        assert_eq!(reading.finding, Some("final_value_needed"));
        let entry = with(request("rise_time"), "final_value", json!(5.0));
        assert!(read(&entry, &time, &values).value.is_some());
    }

    #[test]
    fn a_signal_still_moving_at_the_end_gives_no_step_to_measure() {
        let time: Vec<f64> = (0..=100).map(f64::from).collect();
        let ramp: Vec<f64> = time.clone();
        for kind in ["rise_time", "overshoot_percent"] {
            assert_eq!(
                read(&request(kind), &time, &ramp).finding,
                Some("final_value_needed"),
                "{kind}"
            );
        }
        assert_eq!(
            read(&request("settling_time"), &time, &ramp).finding,
            Some("final_value_needed")
        );
        let sine: Vec<f64> = time.iter().map(|t| (t * 0.5).sin()).collect();
        assert_eq!(read(&request("settling_time"), &time, &sine).value, None);
    }

    /// An RC charging to 1 with time constant 1, cut off at `stop` of them.
    fn charging(stop: f64) -> (Vec<f64>, Vec<f64>) {
        let time: Vec<f64> = (0..=20_000)
            .map(|n| stop * f64::from(n) / 20_000.0)
            .collect();
        let values = time.iter().map(|t| 1.0 - (-t).exp()).collect();
        (time, values)
    }

    #[test]
    fn a_charge_still_rising_at_the_end_of_the_window_gives_no_number() {
        for stop in [1.5, 2.0, 3.0, 4.0, 5.0] {
            let (time, values) = charging(stop);
            for kind in ["rise_time", "settling_time"] {
                let reading = read(&request(kind), &time, &values);
                assert_eq!(reading.finding, Some("final_value_needed"), "{kind} {stop}");
            }
        }
    }

    #[test]
    fn a_charge_that_has_finished_gives_a_number_close_to_the_true_one() {
        let (time, values) = charging(10.0);
        let rise = read(&request("rise_time"), &time, &values).value.unwrap();
        close(rise, 9f64.ln(), 1e-3);
        let settle = read(&request("settling_time"), &time, &values)
            .value
            .unwrap();
        close(settle, 50f64.ln(), 3e-3);
    }

    #[test]
    fn a_ramp_on_a_rail_is_not_settled_for_being_small_against_the_rail() {
        let time: Vec<f64> = (0..=1000).map(|n| f64::from(n) / 1000.0).collect();
        let ramp: Vec<f64> = time.iter().map(|t| 12.0 + t).collect();
        assert_eq!(
            read(&request("settling_time"), &time, &ramp).finding,
            Some("final_value_needed")
        );
    }

    #[test]
    fn a_pulse_that_came_most_of_the_way_back_gives_no_number() {
        let time = [0.0, 1.0, 5.0, 5.01, 10.0];
        let values = [0.0, 5.0, 5.0, 1.0, 1.0];
        let reading = read(&request("overshoot_percent"), &time, &values);
        assert_eq!(reading.finding, Some("final_value_needed"));
    }

    #[test]
    fn a_ring_passing_its_final_value_keeps_its_number() {
        let (time, values) = second_order();
        let reading = read(&request("overshoot_percent"), &time, &values);
        close(reading.value.unwrap(), 52.66, 0.05);
    }

    #[test]
    fn the_result_shows_the_levels_a_step_was_measured_between() {
        let time = [0.0, 1.0, 2.0, 3.0, 40.0];
        let values = [1.0, 2.0, 3.0, 3.0, 3.0];
        for kind in ["rise_time", "overshoot_percent", "settling_time"] {
            let reading = read(&request(kind), &time, &values);
            assert_eq!(reading.levels, Some((1.0, 3.0)), "{kind}");
        }
        let parsed = parse(&request("rise_time"), 0, 100.0, 0.0).unwrap();
        let report = measure(&time, &values, &parsed).report(&parsed, false);
        assert_eq!(report["start_value"], 1.0);
        assert_eq!(report["final_value"], 3.0);
        let parsed = parse(&request("max"), 0, 100.0, 0.0).unwrap();
        let report = measure(&time, &values, &parsed).report(&parsed, false);
        assert!(report.get("final_value").is_none());
    }

    #[test]
    fn a_settled_signal_moving_only_by_rounding_has_no_transition() {
        let (time, values) = ([0.0, 1.0, 2.0], [1.8e-15, -1.8e-15, 1e-16]);
        for kind in ["fall_time", "settling_time", "overshoot_percent"] {
            assert_eq!(
                read(&request(kind), &time, &values).finding,
                Some("no_transition"),
                "{kind}"
            );
        }
    }

    #[test]
    fn a_ripple_inside_the_band_at_the_end_is_not_called_unsettled() {
        let time: Vec<f64> = (0..=2000).map(|n| f64::from(n) * 0.05).collect();
        let ripple = |t: f64| 1.0 + 0.01 * (std::f64::consts::TAU * t).sin();
        let values: Vec<f64> = time
            .iter()
            .map(|&t| if t < 10.0 { t / 10.0 } else { ripple(t) })
            .collect();
        // Five whole cycles in each half of the tail, so the means agree.
        let settled = read(&request("settling_time"), &time, &values);
        close(settled.value.unwrap(), 9.8, 1e-6);
    }

    #[test]
    fn a_small_real_step_on_a_large_rail_is_not_noise() {
        let time = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        let values = [12.0, 12.0, 12.000001, 12.000001, 12.000001, 12.000001];
        let reading = read(&request("rise_time"), &time, &values);
        assert!(reading.value.is_some(), "{reading:?}");
    }

    #[test]
    fn rms_and_i2t_integrate_the_line_between_samples_exactly() {
        // A triangle from 0 to 1 and back: RMS 1 / sqrt(3), i2t 2 / 3.
        let (time, values) = ([0.0, 1.0, 2.0], [0.0, 1.0, 0.0]);
        close(
            read(&request("rms"), &time, &values).value.unwrap(),
            (1.0f64 / 3.0).sqrt(),
            1e-12,
        );
        let i2t = json!({ "kind": "i2t", "signal": "i(r1)" });
        close(read(&i2t, &time, &values).value.unwrap(), 2.0 / 3.0, 1e-12);
    }

    #[test]
    fn a_number_too_large_to_square_is_a_finding_not_a_silent_null() {
        let (time, values) = ([0.0, 1.0], [1e200, 1e200]);
        let reading = read(&request("rms"), &time, &values);
        assert_eq!(reading.value, None);
        assert_eq!(reading.finding, Some("non_finite_data"));
    }

    #[test]
    fn the_stop_time_itself_has_a_value_despite_rounding() {
        let (time, values) = ([0.0, 3.2999999999999996e-5], [0.0, 2.0]);
        let entry = with(request("value_at"), "at_seconds", json!(3.3e-5));
        assert_eq!(read(&entry, &time, &values).value, Some(2.0));
    }

    fn from_saved(from: f64, saved_from: f64, time: &[f64], values: &[f64]) -> bool {
        let entry = with(request("max"), "from_seconds", json!(from));
        let parsed = parse(&entry, 0, 1e9, saved_from).unwrap();
        measure(time, values, &parsed).clipped
    }

    #[test]
    fn a_first_point_just_after_the_saved_start_is_not_a_clipped_window() {
        let (time, values) = ([1.0096e-4, 1e-3], [2.0, 4.0]);
        assert!(!from_saved(1e-4, 1e-4, &time, &values));
    }

    #[test]
    fn a_window_past_the_data_is_flagged_and_one_outside_it_has_no_value() {
        let (time, values) = ([0.0, 1.0], [2.0, 4.0]);
        let reaching = json!({ "kind": "max", "signal": "v(a)", "to_seconds": 5.0 });
        let reading = read(&reaching, &time, &values);
        assert_eq!(reading.value, Some(4.0));
        assert!(reading.clipped);
        let beyond = json!({ "kind": "max", "signal": "v(a)", "from_seconds": 2.0 });
        let reading = read(&beyond, &time, &values);
        assert_eq!(reading.value, None);
        assert_eq!(reading.finding, Some("outside_data"));
    }

    #[test]
    fn a_window_from_before_the_saved_data_is_flagged() {
        let (time, values) = ([1.0, 2.0], [2.0, 4.0]);
        assert!(from_saved(0.0, 0.9, &time, &values));
        let (time, values) = ([9e-4, 1.0], [2.0, 4.0]);
        assert!(from_saved(0.0, 9e-4, &time, &values));
    }

    #[test]
    fn the_end_of_a_run_is_not_clipped_by_rounding() {
        let (time, values) = ([0.0, 1.0 - 1e-12], [2.0, 4.0]);
        let entry = json!({ "kind": "max", "signal": "v(a)", "to_seconds": 1.0 });
        assert!(!read(&entry, &time, &values).clipped);
    }

    #[test]
    fn repeated_times_do_not_divide_by_zero() {
        let (time, values) = ([0.0, 1.0, 1.0, 2.0], [0.0, 1.0, 3.0, 3.0]);
        let reading = read(&request("average"), &time, &values);
        close(reading.value.unwrap(), 0.25 + 1.5, 1e-12);
        let at = with(request("value_at"), "at_seconds", json!(1.0));
        assert!(read(&at, &time, &values).value.unwrap().is_finite());
    }

    #[test]
    fn data_that_is_not_a_number_gives_no_value() {
        let (time, values) = ([0.0, 1.0, 2.0], [1.0, f64::NAN, 3.0]);
        let reading = read(&request("max"), &time, &values);
        assert_eq!(reading.value, None);
        assert_eq!(reading.finding, Some("non_finite_data"));
    }

    #[test]
    fn one_point_has_no_duration_to_measure() {
        let reading = read(&request("rms"), &[1.0], &[2.0]);
        assert_eq!(reading.value, None);
        assert_eq!(reading.finding, Some("outside_data"));
    }

    #[test]
    fn a_result_names_units_and_never_hides_a_null() {
        let entry = json!({ "kind": "i2t", "signal": "i(r1)", "name": "fuse" });
        let parsed = parse(&entry, 0, 1.0, 0.0).unwrap();
        let report = measure(&[0.0, 1.0], &[2.0, 2.0], &parsed).report(&parsed, false);
        assert_eq!(report["name"], "fuse");
        assert_eq!(report["unit"], "A²s");
        assert_eq!(report["value"], 4.0);
        assert_eq!(report["window_truncated"], false);

        let unnamed = parse(&request("crossing_x"), 0, 1.0, 0.0).unwrap_err();
        assert_eq!(unnamed.field, "measurements[0].kind");
        let entry = with(request("crossing"), "level", json!(9.0));
        let parsed = parse(&entry, 0, 1.0, 0.0).unwrap();
        assert_eq!(parsed.name, "crossing v(a)");
        let report = measure(&[0.0, 1.0], &[2.0, 2.0], &parsed).report(&parsed, true);
        assert!(report["value"].is_null(), "{report}");
        assert_eq!(report["unit"], "s");
        assert_eq!(report["finding"], "level_not_reached");
        assert_eq!(report["window_truncated"], true);
    }

    #[test]
    fn what_cannot_give_an_answer_is_refused_with_its_field() {
        let refused = |entry: Value, field: &str| {
            let error = parse(&entry, 3, 1.0, 0.0).unwrap_err();
            assert_eq!(error.field, format!("measurements[3].{field}"), "{error:?}");
        };
        refused(json!({ "kind": "max", "signal": "x(a)" }), "signal");
        refused(json!({ "kind": "i2t", "signal": "v(a)" }), "signal");
        refused(request("value_at"), "at_seconds");
        refused(request("crossing"), "level");
        refused(with(request("max"), "at_seconds", json!(0.5)), "at_seconds");
        refused(with(request("max"), "level", json!(0.5)), "level");
        refused(
            with(request("max"), "direction", json!("rising")),
            "direction",
        );
        refused(with(request("max"), "final_value", json!(1)), "final_value");
        refused(
            with(request("rise_time"), "band_percent", json!(1)),
            "band_percent",
        );
        refused(
            with(request("settling_time"), "band_percent", json!(0)),
            "band_percent",
        );
        refused(
            with(request("max"), "from_seconds", json!(-1)),
            "from_seconds",
        );
        refused(with(request("max"), "to_seconds", json!(2)), "to_seconds");
        refused(
            with(
                with(request("max"), "from_seconds", json!(0.5)),
                "to_seconds",
                json!(0.5),
            ),
            "to_seconds",
        );
        refused(
            with(
                with(request("value_at"), "at_seconds", json!(0.5)),
                "to_seconds",
                json!(0.6),
            ),
            "to_seconds",
        );
        refused(
            with(
                with(request("crossing"), "level", json!(1)),
                "direction",
                json!("up"),
            ),
            "direction",
        );
        refused(
            with(request("max"), "from_seconds", json!("soon")),
            "from_seconds",
        );
    }

    #[test]
    fn a_summary_gives_the_extremes_and_last_value() {
        let summary = summary(&[0.0, 1.0, 2.0], &[1.0, 5.0, 3.0], "v(a)", "V", 0, true);
        assert_eq!(
            summary,
            json!({
                "signal": "v(a)", "unit": "V", "min": 1.0, "max": 5.0, "final": 3.0,
                "window_truncated": true
            })
        );
    }

    #[test]
    fn an_envelope_keeps_a_spike_that_thinning_would_lose() {
        let time: Vec<f64> = (0..=1000).map(|n| f64::from(n) / 1000.0).collect();
        let mut values = vec![0.0; time.len()];
        values[333] = 9.0;
        values[777] = -4.0;
        let summary = summary(&time, &values, "v(a)", "V", 4, false);
        let envelope = &summary["envelope"];
        assert_eq!(envelope["x"], json!([0.125, 0.375, 0.625, 0.875]));
        assert_eq!(envelope["max"], json!([0.0, 9.0, 0.0, 0.0]));
        assert_eq!(envelope["min"], json!([0.0, 0.0, 0.0, -4.0]));
    }

    #[test]
    fn an_envelope_bucket_with_no_sample_still_holds_the_line_across_it() {
        let summary = summary(&[0.0, 4.0], &[0.0, 4.0], "v(a)", "V", 4, false);
        assert_eq!(summary["envelope"]["min"], json!([0.0, 1.0, 2.0, 3.0]));
        assert_eq!(summary["envelope"]["max"], json!([1.0, 2.0, 3.0, 4.0]));
    }

    #[test]
    fn a_single_point_envelope_is_one_bucket() {
        let summary = summary(&[1.0], &[2.0], "v(a)", "V", 8, false);
        assert_eq!(summary["envelope"]["x"], json!([1.0]));
    }

    #[test]
    fn a_summary_of_bad_data_says_so() {
        let summary = summary(&[0.0, 1.0], &[1.0, f64::NAN], "v(a)", "V", 0, false);
        assert_eq!(summary["finding"], "non_finite_data");
        assert_eq!(summary["min"], 1.0);
        assert!(summary["final"].is_null());
    }
}
