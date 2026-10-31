//! The AC and DC sweep analyses: their arguments, the line each adds to the deck,
//! and the proof that a run covered what was asked.

use super::{invalid, simulation_failure, COVERAGE_TOLERANCE};
use crate::mcp::protocol::CallToolResult;
use crate::simulation::kicad_netlist::ElementEntry;
use crate::simulation::raw::Plot;
use serde_json::{json, Value};

/// The most points a sweep may ask for. A run that makes more would be stopped by
/// its budget or size limit anyway, and this says so before the export.
pub(super) const MAX_SWEEP_POINTS: f64 = 1_000_000.0;
const DEFAULT_AC_POINTS: u64 = 50;
const MAX_AC_POINTS: u64 = 10_000;
/// ngspice counts the points of a DC sweep with a little room for rounding, so
/// `0` to `0.3` by `0.1` has four.
const COUNT_TOLERANCE: f64 = 1e-9;
/// Longest target name repeated in a refusal.
const MAX_TARGET_CHARS: usize = 40;
const TEMPERATURE: &str = "temp";
const TEMPERATURE_UNIT: &str = "°C";

/// `field` as a finite number, or none if absent.
pub(super) fn number(
    object: &Value,
    field: &str,
    path: &str,
) -> Result<Option<f64>, CallToolResult> {
    let value = &object[field];
    if value.is_null() {
        return Ok(None);
    }
    match value.as_f64() {
        Some(number) if number.is_finite() => Ok(Some(number)),
        _ => Err(invalid(
            format!("{path}.{field}"),
            format!("{field} must be a finite number"),
        )),
    }
}

fn required(object: &Value, field: &str, path: &str, what: &str) -> Result<f64, CallToolResult> {
    number(object, field, path)?
        .ok_or_else(|| invalid(format!("{path}.{field}"), format!("{what} needs {field}")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scale {
    Decade,
    Octave,
    Linear,
}

impl Scale {
    fn parse(text: Option<&str>) -> Result<Self, CallToolResult> {
        match text {
            None | Some("decade") => Ok(Self::Decade),
            Some("octave") => Ok(Self::Octave),
            Some("linear") => Ok(Self::Linear),
            Some(other) => Err(invalid(
                "analysis.scale",
                format!("Unknown scale: {other}. Use decade, octave or linear"),
            )),
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Decade => "dec",
            Self::Octave => "oct",
            Self::Linear => "lin",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Decade => "decade",
            Self::Octave => "octave",
            Self::Linear => "linear",
        }
    }

    /// Points are spaced by a ratio on every scale but the linear one.
    pub(super) fn is_logarithmic(self) -> bool {
        self != Self::Linear
    }
}

/// An AC sweep. Its limits are numbers formatted here and never caller text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Ac {
    pub start: f64,
    pub stop: f64,
    pub scale: Scale,
    pub points: u64,
}

impl Ac {
    pub(super) fn from_args(analysis: &Value) -> Result<Self, CallToolResult> {
        let start = required(analysis, "start_hz", "analysis", "An ac analysis")?;
        let stop = required(analysis, "stop_hz", "analysis", "An ac analysis")?;
        if start <= 0.0 {
            return Err(invalid("analysis.start_hz", "start_hz must be above 0"));
        }
        if stop <= start {
            return Err(invalid(
                "analysis.stop_hz",
                "stop_hz must be above start_hz",
            ));
        }
        let scale = Scale::parse(analysis["scale"].as_str())?;
        if !analysis["scale"].is_null() && analysis["scale"].as_str().is_none() {
            return Err(invalid("analysis.scale", "scale must be text"));
        }
        let points = match &analysis["points"] {
            Value::Null => DEFAULT_AC_POINTS,
            value => value
                .as_u64()
                .filter(|points| (1..=MAX_AC_POINTS).contains(points))
                .ok_or_else(|| {
                    invalid(
                        "analysis.points",
                        format!("points must be a whole number from 1 to {MAX_AC_POINTS}"),
                    )
                })?,
        };
        if scale == Scale::Linear && points < 2 {
            // One point is the start alone, and the sweep would never reach the stop.
            return Err(invalid(
                "analysis.points",
                "A linear sweep needs at least 2 points to reach stop_hz",
            ));
        }
        let ac = Self {
            start,
            stop,
            scale,
            points,
        };
        let total = ac.estimated_points();
        if !(total <= MAX_SWEEP_POINTS) {
            return Err(invalid(
                "analysis.points",
                format!(
                    "This sweep would take about {total:.0} points. At most {MAX_SWEEP_POINTS:.0}"
                ),
            ));
        }
        Ok(ac)
    }

    /// A safe overestimate of the points, for the limit.
    fn estimated_points(&self) -> f64 {
        let steps = match self.scale {
            Scale::Decade => (self.stop / self.start).log10() * self.points as f64,
            Scale::Octave => (self.stop / self.start).log2() * self.points as f64,
            Scale::Linear => return self.points as f64,
        };
        steps.ceil() + 1.0
    }

    pub(super) fn line(&self) -> String {
        format!(
            ".ac {} {} {:e} {:e}",
            self.scale.word(),
            self.points,
            self.start,
            self.stop
        )
    }

    /// The factor between neighbouring points, which says how far short of
    /// `stop_hz` the last may fall. ngspice ends an octave sweep, and some decade
    /// ones, on the last grid point at or below the stop and not on the stop itself.
    fn step_ratio(&self) -> f64 {
        match self.scale {
            Scale::Decade => 10f64.powf(1.0 / self.points as f64),
            Scale::Octave => 2f64.powf(1.0 / self.points as f64),
            Scale::Linear => 1.0,
        }
    }

    pub(super) fn reached(plot: &Plot) -> Option<f64> {
        plot.real(plot.variable("frequency")?).last()
    }

    pub(super) fn first_point(plot: &Plot) -> Option<f64> {
        plot.real(plot.variable("frequency")?).next()
    }

    /// The sweep ran to the stop, or ended where ngspice's grid does: on the last
    /// point at or below it, so that the next point lies past the stop. A run that
    /// ends where the next point is the stop itself was cut short.
    pub(super) fn covered(&self, plot: &Plot) -> bool {
        plot.complex
            && !plot.is_truncated()
            && Self::reached(plot).is_some_and(|last| {
                last >= self.stop * (1.0 - COVERAGE_TOLERANCE)
                    || last * self.step_ratio() > self.stop * (1.0 + COVERAGE_TOLERANCE)
            })
    }

    /// Whether the last point sits short of the stop the caller asked for.
    pub(super) fn ends_short(&self, reached: f64) -> bool {
        reached < self.stop * (1.0 - COVERAGE_TOLERANCE)
    }

    /// How much of the sweep ran, on the sweep's own scale.
    pub(super) fn coverage(&self, reached: f64) -> f64 {
        let share = match self.scale.is_logarithmic() {
            true => (reached / self.start).ln() / (self.stop / self.start).ln(),
            false => (reached - self.start) / (self.stop - self.start),
        };
        share.clamp(0.0, 1.0)
    }

    pub(super) fn describe(&self, plot: &Plot) -> Value {
        let reached = Self::reached(plot).unwrap_or(0.0);
        json!({
            "kind": "ac",
            "start_hz": self.start,
            "stop_hz": self.stop,
            "scale": self.scale.name(),
            "points_requested": self.points,
            "first_point_hz": Self::first_point(plot).unwrap_or(0.0),
            "simulated_to_hz": reached,
            "coverage_ratio": self.coverage(reached),
            "points": plot.points,
        })
    }
}

/// What a sweep varies, as the deck has it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Sweep {
    /// As the caller wrote it.
    pub target: String,
    pub start: f64,
    pub stop: f64,
    pub step: f64,
    pub points: u64,
    /// The element as the deck spells it, or `temp`. Empty until the deck is read,
    /// so caller text can never reach the engine through this.
    pub name: String,
    pub unit: &'static str,
}

impl Sweep {
    fn from_args(entry: &Value, index: usize) -> Result<Self, CallToolResult> {
        let path = format!("analysis.sweeps[{index}]");
        let target = match entry["target"].as_str().map(str::trim) {
            Some(target) if !target.is_empty() => target.to_string(),
            _ => return Err(invalid(format!("{path}.target"), "A sweep needs a target")),
        };
        let start = required(entry, "start_value", &path, "A sweep")?;
        let stop = required(entry, "stop_value", &path, "A sweep")?;
        let step = required(entry, "step_value", &path, "A sweep")?;
        if step == 0.0 {
            return Err(invalid(
                format!("{path}.step_value"),
                "step_value cannot be 0",
            ));
        }
        // A step that points away from the stop makes the engine write no data and
        // call that a success [verified: run E55 and its inverse].
        if start != stop && (stop - start).signum() != step.signum() {
            return Err(invalid(
                format!("{path}.step_value"),
                "step_value must point from start_value toward stop_value: negative to sweep down",
            ));
        }
        let ratio = (stop - start) / step;
        if !(ratio * (1.0 + COUNT_TOLERANCE) < MAX_SWEEP_POINTS) {
            return Err(invalid(
                format!("{path}.step_value"),
                format!("This sweep would take more than {MAX_SWEEP_POINTS:.0} points"),
            ));
        }
        Ok(Self {
            target,
            start,
            stop,
            step,
            points: count(start, stop, step),
            name: String::new(),
            unit: "",
        })
    }

    fn line(&self) -> String {
        format!(
            "{} {:e} {:e} {:e}",
            self.name, self.start, self.stop, self.step
        )
    }
}

/// How many points the engine saves for a sweep: every step that does not pass the
/// stop, counting the start. Checked against the engine on 22 sweeps, including
/// `0` to `0.3` by `0.1`, which only counts four with a little tolerance.
pub(super) fn count(start: f64, stop: f64, step: f64) -> u64 {
    let steps = (stop - start) / step * (1.0 + COUNT_TOLERANCE) + COUNT_TOLERANCE;
    steps.floor() as u64 + 1
}

/// One or two DC sweeps. The first varies fastest.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct DcSweep {
    pub sweeps: Vec<Sweep>,
}

impl DcSweep {
    pub(super) fn from_args(analysis: &Value) -> Result<Self, CallToolResult> {
        let Some(entries) = analysis["sweeps"]
            .as_array()
            .filter(|sweeps| (1..=2).contains(&sweeps.len()))
        else {
            return Err(invalid(
                "analysis.sweeps",
                "A dc_sweep needs one or two sweeps",
            ));
        };
        let sweeps = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| Sweep::from_args(entry, index))
            .collect::<Result<Vec<_>, _>>()?;
        let total: f64 = sweeps.iter().map(|sweep| sweep.points as f64).product();
        if total > MAX_SWEEP_POINTS {
            return Err(invalid(
                "analysis.sweeps",
                format!("These sweeps would take {total:.0} points. At most {MAX_SWEEP_POINTS:.0}"),
            ));
        }
        Ok(Self { sweeps })
    }

    pub(super) fn expected_points(&self) -> u64 {
        self.sweeps.iter().map(|sweep| sweep.points).product()
    }

    pub(super) fn line(&self) -> String {
        let sweeps: Vec<String> = self.sweeps.iter().map(Sweep::line).collect();
        format!(".dc {}", sweeps.join(" "))
    }

    /// Every point of the grid was saved. A sweep the engine cut short, or one it
    /// counted differently, is not covered.
    pub(super) fn covered(&self, plot: &Plot) -> bool {
        !plot.is_truncated()
            && !plot.variables.is_empty()
            && plot.points as u64 == self.expected_points()
    }

    /// The axis the first sweep draws along, the one the engine saves.
    pub(super) fn axis(plot: &Plot) -> Vec<f64> {
        if plot.variables.is_empty() {
            return Vec::new();
        }
        plot.real(0).collect()
    }

    /// The first sweep, which measurements are taken along.
    pub(super) fn inner(&self) -> &Sweep {
        &self.sweeps[0]
    }

    /// Find each target in the deck and give it the deck's own spelling. A target
    /// must be a voltage source, a current source, a resistor or `TEMP`: the engine
    /// stops on anything else, and on a name that is not there.
    pub(super) fn resolve(
        mut self,
        deck_elements: &[&str],
        elements: &[ElementEntry],
    ) -> Result<Self, CallToolResult> {
        for index in 0..self.sweeps.len() {
            let path = format!("analysis.sweeps[{index}].target");
            let target = self.sweeps[index].target.clone();
            if target.eq_ignore_ascii_case(TEMPERATURE) {
                self.sweeps[index].name = TEMPERATURE.to_string();
                self.sweeps[index].unit = TEMPERATURE_UNIT;
                continue;
            }
            // A reference names its deck element through the netlist, as in a signal.
            // A testbench element has no reference, so its own name stands.
            let part = elements
                .iter()
                .find(|part| part.reference.eq_ignore_ascii_case(&target) && !part.excluded);
            let wanted = part
                .and_then(|part| part.element_name.as_deref())
                .unwrap_or(&target);
            let found = deck_elements
                .iter()
                .find(|name| name.eq_ignore_ascii_case(wanted));
            let Some(&name) = found else {
                return Err(not_found(&target));
            };
            let unit = match name.chars().next().map(|c| c.to_ascii_lowercase()) {
                Some('v') => "V",
                Some('i') => "A",
                Some('r') => "Ω",
                _ => return Err(invalid(
                    path,
                    "A dc_sweep can vary a voltage source, a current source, a resistor or TEMP",
                )),
            };
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(invalid(
                    path,
                    "That element's name holds characters Konnect does not pass to the engine",
                ));
            }
            self.sweeps[index].name = name.to_string();
            self.sweeps[index].unit = unit;
        }
        let names: Vec<String> = self
            .sweeps
            .iter()
            .map(|sweep| sweep.name.to_ascii_lowercase())
            .collect();
        if names.len() == 2 && names[0] == names[1] {
            return Err(invalid(
                "analysis.sweeps[1].target",
                "Both sweeps vary the same element",
            ));
        }
        Ok(self)
    }

    pub(super) fn describe(&self, plot: &Plot) -> Value {
        let sweeps: Vec<Value> = self
            .sweeps
            .iter()
            .map(|sweep| {
                json!({
                    "target": sweep.target,
                    "element": sweep.name,
                    "unit": sweep.unit,
                    "start_value": sweep.start,
                    "stop_value": sweep.stop,
                    "step_value": sweep.step,
                    "points": sweep.points,
                })
            })
            .collect();
        let expected = self.expected_points();
        json!({
            "kind": "dc_sweep",
            "sweeps": sweeps,
            "points": plot.points,
            "expected_points": expected,
            "coverage_ratio": (plot.points as f64 / expected as f64).min(1.0),
        })
    }
}

/// The target the caller named, cut to something safe to repeat.
fn not_found(target: &str) -> CallToolResult {
    let shown: String = target
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TARGET_CHARS)
        .collect();
    simulation_failure(
        "preflight",
        "sweep_target_not_found",
        None,
        vec![shown],
        "No voltage source, current source or resistor has that name in the circuit that \
         would run. Give a part's reference, a testbench element's own name, or TEMP. \
         check_simulation_readiness lists the elements.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dc(sweeps: Value) -> Result<DcSweep, CallToolResult> {
        DcSweep::from_args(&json!({ "kind": "dc_sweep", "sweeps": sweeps }))
    }

    fn ac(analysis: Value) -> Result<Ac, CallToolResult> {
        Ac::from_args(&analysis)
    }

    fn body(result: &CallToolResult) -> Value {
        let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
            panic!("expected text")
        };
        serde_json::from_str(text).unwrap()
    }

    fn field(refusal: CallToolResult) -> String {
        body(&refusal)["error"]["field"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    #[test]
    fn an_ac_line_holds_only_numbers_written_here() {
        let run = ac(json!({ "start_hz": 10.0, "stop_hz": 1e6, "scale": "octave", "points": 12 }))
            .unwrap();
        assert_eq!(run.line(), ".ac oct 12 1e1 1e6");
        let defaults = ac(json!({ "start_hz": 1.0, "stop_hz": 1000.0 })).unwrap();
        assert_eq!(defaults.line(), ".ac dec 50 1e0 1e3");
        let linear =
            ac(json!({ "start_hz": 100.0, "stop_hz": 1100.0, "scale": "linear", "points": 11 }))
                .unwrap();
        assert_eq!(linear.line(), ".ac lin 11 1e2 1.1e3");
    }

    #[test]
    fn an_ac_sweep_the_engine_cannot_take_is_refused_with_its_field() {
        for (analysis, expected) in [
            (
                json!({ "start_hz": 0.0, "stop_hz": 10.0 }),
                "analysis.start_hz",
            ),
            (
                json!({ "start_hz": -1.0, "stop_hz": 10.0 }),
                "analysis.start_hz",
            ),
            (
                json!({ "start_hz": 10.0, "stop_hz": 10.0 }),
                "analysis.stop_hz",
            ),
            (
                json!({ "start_hz": 10.0, "stop_hz": 1.0 }),
                "analysis.stop_hz",
            ),
            (json!({ "stop_hz": 1.0 }), "analysis.start_hz"),
            (json!({ "start_hz": 1.0 }), "analysis.stop_hz"),
            (
                json!({ "start_hz": "1", "stop_hz": 10.0 }),
                "analysis.start_hz",
            ),
            (
                json!({ "start_hz": 1.0, "stop_hz": 10.0, "scale": "log" }),
                "analysis.scale",
            ),
            (
                json!({ "start_hz": 1.0, "stop_hz": 10.0, "points": 0 }),
                "analysis.points",
            ),
            (
                json!({ "start_hz": 1.0, "stop_hz": 10.0, "points": 10001 }),
                "analysis.points",
            ),
            (
                json!({ "start_hz": 1.0, "stop_hz": 10.0, "points": 2.5 }),
                "analysis.points",
            ),
            (
                json!({ "start_hz": 1.0, "stop_hz": 10.0, "scale": "linear", "points": 1 }),
                "analysis.points",
            ),
            (
                json!({ "start_hz": 1e-300, "stop_hz": 1e300, "points": 10000 }),
                "analysis.points",
            ),
        ] {
            let refusal = ac(analysis.clone()).expect_err(&analysis.to_string());
            assert_eq!(field(refusal), expected, "{analysis}");
        }
    }

    #[test]
    fn a_dc_line_names_the_element_as_the_deck_spells_it() {
        let mut run = dc(json!([
            { "target": "bt1", "start_value": -12.0, "stop_value": 12.0, "step_value": 0.25 },
            { "target": "R1", "start_value": 1000.0, "stop_value": 3000.0, "step_value": 1000.0 }
        ]))
        .unwrap();
        // Before the deck is read nothing from the caller can reach the line.
        assert_eq!(run.line(), ".dc  -1.2e1 1.2e1 2.5e-1  1e3 3e3 1e3");
        run.sweeps[0].name = "VBT1".into();
        run.sweeps[1].name = "R1".into();
        assert_eq!(run.line(), ".dc VBT1 -1.2e1 1.2e1 2.5e-1 R1 1e3 3e3 1e3");
        assert_eq!(run.expected_points(), 97 * 3);
    }

    #[test]
    fn the_engines_point_count_is_reproduced_including_rounding() {
        for (start, stop, step, points) in [
            (0.0, 0.3, 0.1, 4),
            (0.0, 1.0, 0.1, 11),
            (0.0, 0.7, 0.1, 8),
            (-12.0, 12.0, 0.25, 97),
            (12.0, -12.0, -0.25, 97),
            (0.0, 1.0, 0.4, 3),
            (0.0, 1.0, 0.7, 2),
            (5.0, 0.3, -0.1, 48),
            (0.0, 0.0, 1.0, 1),
            (0.0, 3.3, 0.1, 34),
            (0.0, 4.6, 0.1, 47),
        ] {
            assert_eq!(count(start, stop, step), points, "{start} {stop} {step}");
        }
    }

    #[test]
    fn a_dc_sweep_the_engine_would_run_on_nothing_is_refused() {
        let ok =
            json!({ "target": "V1", "start_value": 0.0, "stop_value": 5.0, "step_value": 1.0 });
        let with = |key: &str, value: Value| {
            let mut entry = ok.clone();
            entry[key] = value;
            entry
        };
        assert!(dc(json!([ok.clone()])).is_ok());
        for (sweeps, expected) in [
            (json!([]), "analysis.sweeps"),
            (
                json!([ok.clone(), ok.clone(), ok.clone()]),
                "analysis.sweeps",
            ),
            (json!("V1"), "analysis.sweeps"),
            (
                json!([with("step_value", json!(0.0))]),
                "analysis.sweeps[0].step_value",
            ),
            // The engine writes no data for this and reports success.
            (
                json!([with("step_value", json!(-1.0))]),
                "analysis.sweeps[0].step_value",
            ),
            (
                json!([with("stop_value", json!(-5.0))]),
                "analysis.sweeps[0].step_value",
            ),
            (
                json!([with("target", json!(""))]),
                "analysis.sweeps[0].target",
            ),
            (
                json!([with("target", json!(3))]),
                "analysis.sweeps[0].target",
            ),
            (
                json!([with("start_value", json!("0"))]),
                "analysis.sweeps[0].start_value",
            ),
            (
                json!([with("stop_value", json!(null))]),
                "analysis.sweeps[0].stop_value",
            ),
            (
                json!([with("step_value", json!(1e-9))]),
                "analysis.sweeps[0].step_value",
            ),
            (
                json!([
                    with("step_value", json!(0.001)),
                    with("step_value", json!(0.001))
                ]),
                "analysis.sweeps",
            ),
        ] {
            let refusal = dc(sweeps.clone()).expect_err(&sweeps.to_string());
            assert_eq!(field(refusal), expected, "{sweeps}");
        }
    }

    #[test]
    fn a_sweep_that_stands_still_takes_either_sign() {
        for step in [1.0, -1.0] {
            let run = dc(json!([{ "target": "V1", "start_value": 2.0, "stop_value": 2.0, "step_value": step }])).unwrap();
            assert_eq!(run.expected_points(), 1);
        }
    }

    fn entry(reference: &str, element: Option<&str>, excluded: bool) -> ElementEntry {
        ElementEntry {
            reference: reference.to_string(),
            element_name: element.map(str::to_string),
            modelled: true,
            excluded,
        }
    }

    fn resolved(
        target: &str,
        deck: &[&str],
        elements: &[ElementEntry],
    ) -> Result<DcSweep, CallToolResult> {
        dc(json!([{ "target": target, "start_value": 0.0, "stop_value": 1.0, "step_value": 1.0 }]))
            .unwrap()
            .resolve(deck, elements)
    }

    #[test]
    fn a_reference_finds_its_element_and_the_deck_spelling_wins() {
        let elements = [
            entry("BT1", Some("VBT1"), false),
            entry("R1", Some("R1"), false),
        ];
        let deck = ["VBT1", "R1", "Ctop", "x1"];
        let run = resolved("bt1", &deck, &elements).unwrap();
        assert_eq!(
            (run.sweeps[0].name.as_str(), run.sweeps[0].unit),
            ("VBT1", "V")
        );
        let run = resolved("r1", &deck, &elements).unwrap();
        assert_eq!(
            (run.sweeps[0].name.as_str(), run.sweeps[0].unit),
            ("R1", "Ω")
        );
        let run = resolved("Temp", &deck, &elements).unwrap();
        assert_eq!(
            (run.sweeps[0].name.as_str(), run.sweeps[0].unit),
            ("temp", "°C")
        );
        // A testbench element has no reference, so its own name stands.
        let run = resolved("Ibias", &["IBIAS"], &[]).unwrap();
        assert_eq!(
            (run.sweeps[0].name.as_str(), run.sweeps[0].unit),
            ("IBIAS", "A")
        );
    }

    #[test]
    fn a_target_the_deck_lacks_is_a_typed_refusal_and_never_reaches_the_engine() {
        let refusal = resolved("V9", &["V1", "R1"], &[]).unwrap_err();
        let text = body(&refusal).to_string();
        assert!(text.contains("sweep_target_not_found"), "{text}");
        // An excluded part is not in the deck, so a reference to it finds nothing.
        let elements = [entry("BT1", None, true)];
        assert!(resolved("BT1", &["R1"], &elements).is_err());
        // Text from the caller is cut to a short printable name.
        let text = body(&not_found(&format!("V\n{}", "x".repeat(200)))).to_string();
        assert!(!text.contains(&"x".repeat(50)), "{text}");
        assert!(text.contains("sweep_target_not_found"));
    }

    #[test]
    fn a_target_of_another_kind_or_an_odd_name_is_refused_as_an_argument() {
        for deck in [["C1"], ["L1"], ["Q1"], ["X1"], ["R1:2"], ["V1-b"]] {
            let target = deck[0];
            let refusal = resolved(target, &deck, &[]).expect_err(target);
            assert_eq!(field(refusal), "analysis.sweeps[0].target", "{target}");
        }
    }

    #[test]
    fn two_sweeps_cannot_vary_one_element() {
        let run = dc(json!([
            { "target": "v1", "start_value": 0.0, "stop_value": 1.0, "step_value": 1.0 },
            { "target": "V1", "start_value": 0.0, "stop_value": 1.0, "step_value": 1.0 }
        ]))
        .unwrap();
        let refusal = run.resolve(&["V1"], &[]).unwrap_err();
        assert_eq!(field(refusal), "analysis.sweeps[1].target");
    }
}
