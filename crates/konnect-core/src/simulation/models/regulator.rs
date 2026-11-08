//! Linear regulators and series voltage references, as one behavioural
//! voltage source between `OUT` and `COM`, the ground or adjust pin.
//!
//! The source is the smooth lesser of two laws: the regulated output,
//! `V0 + k (Vin - Vin0)` less a current-limit term, and the input less the
//! dropout resistance's drop, `Vin - Rdo I`. `I` is the output current, read
//! by a 0 V source and turned into a voltage by an H source, so the law needs
//! no current inside an expression. The output resistance after the source is
//! the load regulation. The input pin carries the output current, mirrored by
//! a behavioural current, and the quiescent current, so the supply sees what
//! the part draws.
//!
//! Every corner is smooth, `(a + b - sqrt((a - b)^2 + e^2)) / 2` for the lesser
//! of two, because a hard `min` leaves the solver a flat region where Newton
//! steps stall (a hard clamp failed every operating-point method on
//! libngspice 45.2, and this form solved on its first). Each width `e` is far
//! below what a check reads, so a value read away from a corner moves by less
//! than `e^2 / (4 d)` at a distance `d` from it.
//!
//! An LDO sources current only: driven above its setting, its output takes
//! 1 uA a volt, as a feedback divider would. A reference also sinks, through
//! the same output resistance, with the sunk current going to ground.

use super::bench::{Bench, Check, Probe, Role};
use super::fit;
use super::text::number;
use super::{
    of, Characteristic, Element, Fitted, Invalid, Kind, Model, Quantity, INPUT_VOLTAGE,
    OUTPUT_CURRENT, OUTPUT_VOLTAGE,
};

/// The model's pins, in the order an instance lists its nodes.
/// The third pin is `COM`, the part's ground or adjust pin, never `GND`:
/// ngspice reads a node named `gnd` as ground, even a subcircuit's port, so a
/// port of that name would tie the pin to the circuit's ground.
pub(crate) const PINS: [&str; 3] = ["IN", "OUT", "COM"];
/// The width of the corner between regulation and dropout, in volts.
const DROPOUT_CORNER: f64 = 1e-3;
/// The width of the current limit's corner, as a fraction of the limit.
pub(super) const LIMIT_CORNER: f64 = 1e-5;
/// How much the current limit pulls the output down per amp past it, as
/// volts of output setting per amp of limit: the current at a short is then
/// within a part in ten thousand of the limit.
pub(super) const LIMIT_STIFFNESS: f64 = 1e4;
/// An LDO driven above its setting sinks a current of this conductance's
/// inverse: 1 uA a volt.
const SINK_RESISTANCE: f64 = 1e6;
/// The width of the sink corner, in amps.
pub(super) const SINK_CORNER: f64 = 1e-12;
/// The quiescent current reaches its value over the first tenth of a volt of
/// input: `Iq Vin / sqrt(Vin^2 + 0.01)`, which is passive at 0 V.
pub(super) const QUIESCENT_KNEE: f64 = 1e-2;
/// How far above ground an output check holds `COM`.
pub(super) const COMMON_LIFT: f64 = 1.0;
/// Output values closer than this to the current limit are refused.
const LIMIT_MARGIN: f64 = 0.9;

/// One output voltage at an input voltage and a load current.
#[derive(Debug, Clone, Copy)]
struct Point {
    vin: f64,
    iout: f64,
    vout: f64,
    index: usize,
}

/// The fitted law.
#[derive(Debug, Clone, Copy)]
struct Law {
    v0: f64,
    vin0: f64,
    line: f64,
    rout: f64,
    rdo: f64,
    limit: Option<f64>,
    sinks: bool,
}

pub(super) fn smooth_max_zero(z: f64, width: f64) -> f64 {
    (z + (z * z + width * width).sqrt()) / 2.0
}

pub(super) fn smooth_min(a: f64, b: f64, width: f64) -> f64 {
    (a + b - ((a - b) * (a - b) + width * width).sqrt()) / 2.0
}

impl Law {
    fn stiffness(&self, limit: f64) -> f64 {
        LIMIT_STIFFNESS * self.v0 / limit
    }

    /// The source's voltage at an input voltage and an output current, as
    /// the engine evaluates the expression.
    fn source(&self, vin: f64, current: f64) -> f64 {
        let mut regulated = self.v0 + self.line * (vin - self.vin0);
        if let Some(limit) = self.limit {
            regulated -=
                self.stiffness(limit) * smooth_max_zero(current - limit, LIMIT_CORNER * limit);
        }
        let mut value = smooth_min(regulated, vin - self.rdo * current, DROPOUT_CORNER);
        if !self.sinks {
            value += SINK_RESISTANCE * smooth_max_zero(-current, SINK_CORNER);
        }
        value
    }

    /// The output voltage at a load current.
    fn output(&self, vin: f64, current: f64) -> f64 {
        self.source(vin, current) - self.rout * current
    }

    /// The current into an output held at `volts`.
    fn current_at(&self, vin: f64, volts: f64) -> Option<f64> {
        let high = (vin - volts) / self.rdo.max(1e-9) + 1.0;
        fit::root(|current| self.output(vin, current) - volts, 0.0, high)
    }
}

pub(crate) fn build(
    kind: Kind,
    name: &str,
    characteristics: &[Characteristic],
    _temperature: f64,
) -> Fitted<Model> {
    let mut model = Model::new(kind, name, Element::Subcircuit, PINS.to_vec());
    let points: Vec<Point> = of(characteristics, Quantity::OutputVoltage)
        .into_iter()
        .map(|c| Point {
            vin: c.condition(INPUT_VOLTAGE).unwrap_or_default(),
            iout: c.condition(OUTPUT_CURRENT).unwrap_or_default(),
            vout: c.value,
            index: c.index,
        })
        .collect();
    for point in &points {
        if point.vin <= point.vout {
            return Err(Invalid::new(
                characteristics[point.index].field(&format!("conditions.{INPUT_VOLTAGE}")),
                "the input voltage must be above the output voltage it was read at",
            ));
        }
    }
    let (fit_points, held) = hold_out(&points);
    let law_line = distinct(&fit_points, |p| p.vin);
    let law_load = distinct(&fit_points, |p| p.iout);
    let vin0 = points[0].vin;
    let (v0, line, rout) = fit_output(&fit_points, vin0, law_line, law_load).ok_or_else(|| {
        Invalid::new(
            "characteristics",
            "the output_voltage values do not fit one line in input voltage and load current",
        )
    })?;
    if v0 <= 0.0 {
        return Err(Invalid::new(
            characteristics[points[0].index].field("value"),
            "the output voltage values fit a setting at or below zero",
        ));
    }
    if rout < 0.0 {
        return Err(Invalid::new(
            characteristics[points[0].index].field("value"),
            "the output voltage rises with load current; Konnect fits a flat or falling load line",
        ));
    }
    model.fitted.push(format!(
        "the output setting{}{} from {} output_voltage values",
        if law_line { ", line regulation" } else { "" },
        if law_load { " and load regulation" } else { "" },
        fit_points.len()
    ));
    if !law_line {
        model.assumed.push(
            "no line regulation: the output voltage values share one input voltage".to_string(),
        );
    }
    if !law_load {
        model.assumed.push(
            "no load regulation: the output voltage values share one load current".to_string(),
        );
    }

    // The dropout resistance, from the dropout voltage or, for a reference
    // without one, from half the smallest headroom any output value shows.
    let largest_load = points.iter().map(|p| p.iout).fold(1e-3, f64::max);
    let dropout = of(characteristics, Quantity::DropoutVoltage)
        .first()
        .copied();
    let rdo = match dropout {
        Some(c) => {
            let current = c.condition(OUTPUT_CURRENT).unwrap_or_default();
            let rdo = c.value / current - rout;
            if rdo <= 0.0 {
                return Err(Invalid::new(
                    c.field("value"),
                    "the dropout voltage is no larger than the load regulation's drop at its \
                     current, so no dropout resistance is left",
                ));
            }
            model
                .fitted
                .push("the dropout resistance from the dropout voltage".to_string());
            rdo
        }
        None => {
            let headroom = points
                .iter()
                .map(|p| p.vin - p.vout)
                .fold(f64::INFINITY, f64::min);
            let rdo = 0.5 * headroom / largest_load;
            model.assumed.push(format!(
                "no dropout voltage: the input-to-output drop is {} V at {} A, half the smallest \
                 headroom of the output values",
                number(rdo * largest_load),
                number(largest_load)
            ));
            rdo
        }
    };
    let mut law = Law {
        v0,
        vin0,
        line,
        rout,
        rdo,
        limit: None,
        sinks: kind == Kind::VoltageReference,
    };
    for point in &points {
        let regulated = v0 + line * (point.vin - vin0) - rout * point.iout;
        let floor = point.vin - (rdo + rout) * point.iout;
        if floor < regulated + 20.0 * DROPOUT_CORNER {
            return Err(Invalid::new(
                characteristics[point.index].field(&format!("conditions.{INPUT_VOLTAGE}")),
                "this output value is in dropout by the dropout voltage given, so it is not a \
                 regulated output value",
            ));
        }
    }

    let limit = of(characteristics, Quantity::CurrentLimit).first().copied();
    let limit_setup = match limit {
        Some(c) => {
            let vin = c.condition(INPUT_VOLTAGE).unwrap_or(vin0);
            let volts = c.condition(OUTPUT_VOLTAGE).unwrap_or(0.0);
            if let Some(point) = points.iter().find(|p| p.iout > LIMIT_MARGIN * c.value) {
                return Err(Invalid::new(
                    characteristics[point.index].field(&format!("conditions.{OUTPUT_CURRENT}")),
                    "an output value read within 10 percent of the current limit cannot be \
                     told from the limit",
                ));
            }
            if volts >= v0 + line * (vin - vin0) {
                return Err(Invalid::new(
                    c.field(&format!("conditions.{OUTPUT_VOLTAGE}")),
                    "the current limit must be read with the output held below its setting",
                ));
            }
            if (vin - volts) / (rdo + rout) <= c.value * 1.01 {
                return Err(Invalid::new(
                    c.field("value"),
                    "the dropout resistance passes less than this current at this input \
                     voltage, so the limit could never be reached",
                ));
            }
            // The limit term is solved so the engine's current at the held
            // output is the datasheet's, smoothing and stiffness included.
            let target = c.value;
            let reading = |setting: f64| {
                let trial = Law {
                    limit: Some(setting),
                    ..law
                };
                trial.current_at(vin, volts).unwrap_or(f64::NAN) - target
            };
            let setting = fit::root(reading, 0.5 * target, 1.5 * target).ok_or_else(|| {
                Invalid::new(c.field("value"), "no current limit reproduces this value")
            })?;
            law.limit = Some(setting);
            model
                .fitted
                .push("the current limit, flat down to a short".to_string());
            Some((c, vin, volts))
        }
        None => {
            model.assumed.push(
                "no current limit: a short draws what the dropout resistance passes".to_string(),
            );
            None
        }
    };

    let quiescent = of(characteristics, Quantity::QuiescentCurrent)
        .first()
        .copied();
    let iq = quiescent.map(|c| {
        let vin = c.condition(INPUT_VOLTAGE).unwrap_or_default();
        c.value * (vin * vin + QUIESCENT_KNEE).sqrt() / vin
    });
    if iq.is_none() {
        model
            .assumed
            .push("no quiescent current: the input draws only the load's current".to_string());
    }
    model.assumed.push(
        "no loop dynamics: the output answers load and line steps at once, and needs no \
         output capacitor"
            .to_string(),
    );
    model
        .assumed
        .push("no temperature behaviour and no thermal shutdown".to_string());
    model.assumed.push(if law.sinks {
        "the output sinks current through the same output resistance it sources through".to_string()
    } else {
        "the output sources current only and does not conduct back into the input".to_string()
    });

    model.body = body(name, &law, iq);
    model.parameters = vec![
        ("output_setting".to_string(), v0),
        ("setting_input_voltage".to_string(), vin0),
        ("line_regulation".to_string(), line),
        ("output_resistance".to_string(), rout),
        ("dropout_resistance".to_string(), rdo),
    ];
    if let Some(setting) = law.limit {
        model
            .parameters
            .push(("current_limit_setting".to_string(), setting));
    }
    if let Some(iq) = iq {
        model.parameters.push(("quiescent_current".to_string(), iq));
    }

    let mut bench = Bench::default();
    for point in &fit_points {
        check_output(&mut model, &mut bench, *point, Role::Fit);
    }
    if let Some(point) = held {
        check_output(&mut model, &mut bench, point, Role::HeldOut);
    }
    if let Some(c) = dropout {
        let current = c.condition(OUTPUT_CURRENT).unwrap_or_default();
        // In dropout by half the dropout voltage, where the input less the
        // output is the dropout resistance's drop alone and the output stays
        // above ground.
        let vin = v0 + 0.5 * c.value;
        let id = bench.id();
        let (input, output) = (format!("{id}i"), format!("{id}o"));
        bench.dc.push(Bench::voltage(&id, &input, vin, false));
        bench.dc.push(model.instance(&id, &[&input, &output, "0"]));
        bench.dc.push(sink(&id, &output, current));
        model.checks.push(Check {
            characteristic: c.index,
            role: Role::Fit,
            expected: c.value,
            probe: Probe::Affine {
                node: output,
                offset: vin,
                scale: -1.0,
            },
        });
    }
    if let Some(c) = quiescent {
        let id = bench.id();
        let (input, output) = (format!("{id}i"), format!("{id}o"));
        let vin = c.condition(INPUT_VOLTAGE).unwrap_or_default();
        bench.dc.push(Bench::voltage(&id, &input, vin, false));
        bench.dc.push(model.instance(&id, &[&input, &output, "0"]));
        model.checks.push(Check {
            characteristic: c.index,
            role: Role::Fit,
            expected: c.value,
            probe: Probe::Current {
                source: format!("V{id}"),
                scale: -1.0,
            },
        });
    }
    if let Some((c, vin, volts)) = limit_setup {
        let id = bench.id();
        let (input, output) = (format!("{id}i"), format!("{id}o"));
        bench.dc.push(Bench::voltage(&id, &input, vin, false));
        bench.dc.push(model.instance(&id, &[&input, &output, "0"]));
        bench
            .dc
            .push(Bench::voltage(&format!("{id}f"), &output, volts, false));
        model.checks.push(Check {
            characteristic: c.index,
            role: Role::Fit,
            expected: c.value,
            probe: Probe::Current {
                source: format!("V{id}f"),
                scale: 1.0,
            },
        });
    }
    model.bench = bench;
    Ok(model)
}

/// A load that draws `amps` out of `node`, or nothing for none.
fn sink(id: &str, node: &str, amps: f64) -> String {
    if amps > 0.0 {
        format!("I{id}l {node} 0 DC {}", number(amps))
    } else {
        format!("R{id}l {node} 0 1.000000e12")
    }
}

/// An output value, with `COM` lifted off ground so a pin the engine tied to
/// ground would read wrong.
fn check_output(model: &mut Model, bench: &mut Bench, point: Point, role: Role) {
    let id = bench.id();
    let (input, output, common) = (format!("{id}i"), format!("{id}o"), format!("{id}c"));
    bench
        .dc
        .push(Bench::voltage(&id, &input, point.vin + COMMON_LIFT, false));
    bench
        .dc
        .push(Bench::voltage(&common, &common, COMMON_LIFT, false));
    bench
        .dc
        .push(model.instance(&id, &[&input, &output, &common]));
    bench.dc.push(sink(&id, &output, point.iout));
    model.checks.push(Check {
        characteristic: point.index,
        role,
        expected: point.vout,
        probe: Probe::Affine {
            node: output,
            offset: -COMMON_LIFT,
            scale: 1.0,
        },
    });
}

/// Whether the points hold two or more distinct values of `key`.
fn distinct(points: &[Point], key: impl Fn(&Point) -> f64) -> bool {
    let Some(first) = points.first() else {
        return false;
    };
    points
        .iter()
        .any(|p| (key(p) - key(first)).abs() > 1e-9 * key(first).abs().max(1e-12))
}

/// The unknowns the points fix: the setting always, the line and load terms
/// when the points vary in input voltage and in load current.
fn unknowns(points: &[Point]) -> usize {
    1 + usize::from(distinct(points, |p| p.vin)) + usize::from(distinct(points, |p| p.iout))
}

/// With more values than the fit needs, the one nearest the middle of the
/// load (then input) order whose absence leaves every term fixed is held out.
fn hold_out(points: &[Point]) -> (Vec<Point>, Option<Point>) {
    let needed = unknowns(points);
    if points.len() <= needed {
        return (points.to_vec(), None);
    }
    let mut order: Vec<usize> = (0..points.len()).collect();
    order.sort_by(|&a, &b| {
        points[a]
            .iout
            .total_cmp(&points[b].iout)
            .then(points[a].vin.total_cmp(&points[b].vin))
    });
    let middle = (order.len() - 1) as f64 / 2.0;
    let mut by_distance: Vec<usize> = (0..order.len()).collect();
    by_distance.sort_by(|&a, &b| {
        ((a as f64) - middle)
            .abs()
            .total_cmp(&((b as f64) - middle).abs())
    });
    for rank in by_distance {
        let held = order[rank];
        let rest: Vec<Point> = points
            .iter()
            .enumerate()
            .filter(|(at, _)| *at != held)
            .map(|(_, p)| *p)
            .collect();
        if unknowns(&rest) == needed {
            return (rest, Some(points[held]));
        }
    }
    (points.to_vec(), None)
}

/// `vout = v0 + line (vin - vin0) - rout iout`, by least squares on the
/// terms the points fix.
fn fit_output(points: &[Point], vin0: f64, line: bool, load: bool) -> Option<(f64, f64, f64)> {
    let row = |p: &Point| {
        let mut row = vec![1.0];
        if line {
            row.push(p.vin - vin0);
        }
        if load {
            row.push(-p.iout);
        }
        row
    };
    let size = 1 + usize::from(line) + usize::from(load);
    let mut normal = vec![vec![0.0; size]; size];
    let mut rhs = vec![0.0; size];
    for point in points {
        let r = row(point);
        for a in 0..size {
            rhs[a] += r[a] * point.vout;
            for b in 0..size {
                normal[a][b] += r[a] * r[b];
            }
        }
    }
    let solution = fit::solve(normal, rhs)?;
    let mut terms = solution.into_iter();
    let v0 = terms.next()?;
    let k = if line { terms.next()? } else { 0.0 };
    let rout = if load { terms.next()? } else { 0.0 };
    Some((v0, k, rout))
}

/// `+ k * expression`, with the sign written as an operator.
pub(super) fn term(k: f64, expression: &str) -> String {
    if k < 0.0 {
        format!("-{}*{expression}", number(-k))
    } else {
        format!("+{}*{expression}", number(k))
    }
}

pub(super) fn expr_smooth_max_zero(z: &str, width: f64) -> String {
    format!(
        "(({z})+sqrt(({z})*({z})+{}))/2.000000e0",
        number(width * width)
    )
}

pub(super) fn expr_smooth_min(a: &str, b: &str, width: f64) -> String {
    format!(
        "(({a})+({b})-sqrt((({a})-({b}))*(({a})-({b}))+{}))/2.000000e0",
        number(width * width)
    )
}

/// The cards: the source, its output resistance, the current sense and the
/// input's currents.
fn body(name: &str, law: &Law, iq: Option<f64>) -> String {
    const CURRENT: &str = "v(hs,COM)";
    let mut regulated = format!(
        "{}{}",
        number(law.v0),
        term(law.line, &format!("(v(IN,COM)-{})", number(law.vin0)))
    );
    if let Some(limit) = law.limit {
        regulated.push_str(&term(
            -law.stiffness(limit),
            &expr_smooth_max_zero(
                &format!("{CURRENT}-{}", number(limit)),
                LIMIT_CORNER * limit,
            ),
        ));
    }
    let floor = format!("v(IN,COM){}", term(-law.rdo, CURRENT));
    let mut source = expr_smooth_min(&regulated, &floor, DROPOUT_CORNER);
    if !law.sinks {
        source.push_str(&term(
            SINK_RESISTANCE,
            &expr_smooth_max_zero(&format!("-{CURRENT}"), SINK_CORNER),
        ));
    }
    let mut cards = format!(".subckt {name} IN OUT COM\nBO s COM V={source}\n");
    if law.rout > 0.0 {
        cards.push_str(&format!(
            "ROUT s x {}\nVS x OUT 0.000000e0\n",
            number(law.rout)
        ));
    } else {
        cards.push_str("VS s OUT 0.000000e0\n");
    }
    cards.push_str("HS hs COM VS 1.000000e0\n");
    let input_current = if law.sinks {
        expr_smooth_max_zero(CURRENT, SINK_CORNER)
    } else {
        CURRENT.to_string()
    };
    cards.push_str(&format!("BI IN COM I={input_current}\n"));
    if let Some(iq) = iq {
        cards.push_str(&format!(
            "BQ IN COM I={}*v(IN,COM)/sqrt(v(IN,COM)*v(IN,COM)+{})\n",
            number(iq),
            number(QUIESCENT_KNEE)
        ));
    }
    cards.push_str(&format!(".ends {name}\n"));
    cards
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::{build as build_model, text};
    use super::*;
    use serde_json::{json, Value};

    fn output(vin: f64, iout: f64, vout: f64) -> Value {
        json!({
            "quantity": "output_voltage", "value": vout,
            "conditions": { "input_voltage_volts": vin, "output_current_amps": iout },
            "statistic": "typical"
        })
    }

    fn dropout(volts: f64, amps: f64) -> Value {
        json!({
            "quantity": "dropout_voltage", "value": volts,
            "conditions": { "output_current_amps": amps }, "statistic": "typical"
        })
    }

    fn ldo_values() -> Vec<Characteristic> {
        parsed(json!([
            output(5.0, 1e-3, 3.3),
            output(5.0, 0.15, 3.29),
            output(5.0, 0.3, 3.28),
            output(12.0, 1e-3, 3.307),
            dropout(0.25, 0.3),
            { "quantity": "quiescent_current", "value": 55e-6,
              "conditions": { "input_voltage_volts": 5.0 }, "statistic": "typical" },
            { "quantity": "current_limit", "value": 0.5,
              "conditions": { "output_voltage_volts": 0.0 }, "statistic": "typical" }
        ]))
    }

    #[test]
    fn an_ldo_is_a_three_pin_subcircuit_in_the_fitted_grammar() {
        let model = build_model(Kind::Ldo, "tlv733_fit", &ldo_values(), 25.0).unwrap();
        let checked = text::check_fitted(model.file().as_bytes()).unwrap();
        assert_eq!(checked.pins(), ["IN", "OUT", "COM"]);
        assert_eq!(checked.kind, "ldo");
        // The line and load terms are fixed by three of the four values, and
        // the middle of the load order is held out.
        let held: Vec<_> = model
            .checks
            .iter()
            .filter(|c| c.role == Role::HeldOut)
            .collect();
        assert_eq!(held.len(), 1);
        assert!((fitted(&model, "output_resistance") - 0.0668).abs() < 1e-3);
        assert!((fitted(&model, "line_regulation") - 1e-3).abs() < 1e-6);
        // Dropout: 0.25 V at 0.3 A, less the output resistance's share.
        let rdo = fitted(&model, "dropout_resistance");
        assert!((rdo - (0.25 / 0.3 - fitted(&model, "output_resistance"))).abs() < 1e-9);
        assert_eq!(model.checks.len(), 7);
    }

    #[test]
    fn the_law_reproduces_every_value_it_was_fitted_to() {
        let model = build_model(Kind::Ldo, "tlv733_fit", &ldo_values(), 25.0).unwrap();
        let law = Law {
            v0: fitted(&model, "output_setting"),
            vin0: fitted(&model, "setting_input_voltage"),
            line: fitted(&model, "line_regulation"),
            rout: fitted(&model, "output_resistance"),
            rdo: fitted(&model, "dropout_resistance"),
            limit: Some(fitted(&model, "current_limit_setting")),
            sinks: false,
        };
        // The fitted values, to a part in ten thousand.
        for (vin, iout, vout) in [(5.0, 1e-3, 3.3), (5.0, 0.3, 3.28), (12.0, 1e-3, 3.307)] {
            let reading = law.output(vin, iout);
            assert!((reading - vout).abs() < 3.3e-4, "{vin} {iout}: {reading}");
        }
        // Deep in dropout the drop is the dropout voltage.
        let vin = law.v0 - 0.25;
        assert!((vin - law.output(vin, 0.3) - 0.25).abs() < 1e-5);
        // A short draws the limit.
        let short = law.current_at(5.0, 0.0).unwrap();
        assert!((short - 0.5).abs() < 1e-6, "{short}");
        // Driven above its setting, an LDO sinks a microamp or so.
        let driven = law.current_at(5.0, 4.3).unwrap_or(0.0);
        assert!(driven.abs() < 1e-5, "{driven}");
    }

    #[test]
    fn a_reference_sinks_and_needs_no_dropout_value() {
        let values = parsed(json!([output(5.0, 0.0, 2.5), output(5.0, 0.01, 2.499)]));
        let model = build_model(Kind::VoltageReference, "ref25_fit", &values, 25.0).unwrap();
        text::check_fitted(model.file().as_bytes()).unwrap();
        assert!(model
            .assumed
            .iter()
            .any(|a| a.contains("no dropout voltage")));
        assert!(model.assumed.iter().any(|a| a.contains("sinks current")));
        assert!(!model.body.contains("1.000000e6*"));
        assert!(build_model(Kind::Ldo, "ref25_fit", &values, 25.0).is_err());
    }

    #[test]
    fn values_the_law_cannot_hold_are_refused_on_their_field() {
        // A rising load line.
        let rising = parsed(json!([
            output(5.0, 0.0, 3.3),
            output(5.0, 0.1, 3.31),
            dropout(0.2, 0.1)
        ]));
        let error = build_model(Kind::Ldo, "x_fit", &rising, 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[0].value");
        // An output read in dropout.
        let dropped = parsed(json!([output(3.4, 0.3, 3.3), dropout(0.3, 0.3)]));
        let error = build_model(Kind::Ldo, "x_fit", &dropped, 25.0).unwrap_err();
        assert_eq!(
            error.field,
            "characteristics[0].conditions.input_voltage_volts"
        );
        // An input below the output.
        let below = parsed(json!([output(3.0, 0.1, 3.3), dropout(0.3, 0.3)]));
        let error = build_model(Kind::Ldo, "x_fit", &below, 25.0).unwrap_err();
        assert_eq!(
            error.field,
            "characteristics[0].conditions.input_voltage_volts"
        );
        // A dropout smaller than the load line's drop.
        let tiny = parsed(json!([
            output(5.0, 0.0, 3.3),
            output(5.0, 0.1, 3.2),
            dropout(0.05, 0.1)
        ]));
        let error = build_model(Kind::Ldo, "x_fit", &tiny, 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[2].value");
        // A limit the dropout resistance could never pass.
        let unreachable = parsed(json!([output(5.0, 0.0, 3.3), dropout(1.0, 0.1), {
            "quantity": "current_limit", "value": 1.0, "statistic": "typical"
        }]));
        let error = build_model(Kind::Ldo, "x_fit", &unreachable, 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[2].value");
        // The LDO needs its dropout.
        let none = parsed(json!([output(5.0, 0.0, 3.3)]));
        assert!(build_model(Kind::Ldo, "x_fit", &none, 25.0).is_err());
    }
}
