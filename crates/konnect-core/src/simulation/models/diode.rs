//! Diodes, Schottky diodes, LEDs and Zeners, on ngspice's diode model.
//!
//! The forward law is `V = N Vt ln(I / IS + 1) + I RS`. In breakdown ngspice
//! makes the junction carry `IBV` at `BV`, and the current grows as
//! `exp(dV / (NBV Vt))` beyond it, so the slope there is `NBV Vt / I + RS`.
//! A Zener is a subcircuit of a forward diode and a breakdown diode, so its
//! forward fit and its breakdown fit each keep their own series resistance.

use super::bench::{Bench, Check, Probe, Role};
use super::fit::{self, thermal_voltage};
use super::text::{diode_card, number};
use super::{
    of, Characteristic, Element, Fitted, Invalid, Kind, Model, Quantity, FORWARD_CURRENT,
    FREQUENCY, REVERSE_CURRENT, REVERSE_VOLTAGE,
};

/// Small-signal tests, unless a characteristic names its own frequency.
const CAPACITANCE_HZ: f64 = 1e6;
const IMPEDANCE_HZ: f64 = 1e3;
/// The breakdown diode of a Zener conducts nothing forward: the forward diode
/// beside it does. Its `IS` lies in this range, and its `N` keeps its forward
/// current below 20 nA up to 1 V even at the top of it.
const BREAKDOWN_IS_RANGE: (f64, f64) = (1e-20, 1e-12);
const BREAKDOWN_N: f64 = 4.0;
/// A Zener with no forward values gets a plain silicon junction.
pub(super) const GENERIC_IS: f64 = 1e-14;
pub(super) const GENERIC_N: f64 = 1.0;
const DEFAULT_M: f64 = 0.5;
const N_RANGE: std::ops::RangeInclusive<f64> = 0.5..=12.0;
/// A series resistance that drops less than this at the highest current is
/// written as none.
const NEGLIGIBLE_DROP: f64 = 1e-6;
/// The forward tempco an LED is taken to have, in V/K.
const LED_TEMPCO: f64 = -2e-3;
/// ngspice places the breakdown knee, where the breakdown current is `IS`, at
/// `BV - NBV Vt ln(IBV / IS)`. It must stay this share of `BV` above zero, or
/// the diode leaks as if broken down at every reverse voltage.
const KNEE_FLOOR: f64 = 0.3;
/// Points a law with three parameters needs; one more is held out.
const LAW_PARAMETERS: usize = 3;
/// How far above the technology's usual emission coefficient a two-point fit
/// may land before the points are read as reaching the resistive region.
const PLAUSIBLE_N_SHARE: f64 = 1.25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Technology {
    name: &'static str,
    /// The emission coefficient taken when two points cannot fix it.
    pub assumed_n: f64,
    pub eg: f64,
    pub xti: f64,
    pub vj: f64,
}

pub(super) const SILICON: Technology = Technology {
    name: "silicon",
    assumed_n: 1.8,
    eg: 1.11,
    xti: 3.0,
    vj: 0.7,
};
const SCHOTTKY: Technology = Technology {
    name: "Schottky",
    assumed_n: 1.05,
    eg: 0.69,
    xti: 2.0,
    vj: 0.4,
};
const LED: Technology = Technology {
    name: "silicon",
    assumed_n: 2.0,
    eg: 1.11,
    xti: 3.0,
    vj: 0.7,
};

/// One forward point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Point {
    pub current: f64,
    pub voltage: f64,
}

/// `IS`, `N` and `RS` for a set of forward points, and how they were reached.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ForwardFit {
    pub is: f64,
    pub n: f64,
    pub rs: f64,
    pub note: String,
    pub assumed: Vec<String>,
}

/// The voltage the diode law gives at `current`.
pub(crate) fn forward_voltage(is: f64, n: f64, rs: f64, vt: f64, current: f64) -> f64 {
    n * vt * (current / is).ln_1p() + current * rs
}

/// The `IBV` that makes the junction carry `current` at `BV`. ngspice places
/// its knee so that the current at `BV` is `IBV + IS - IS BV / Vt`, which falls
/// short by a tenth of a typical test current for a Schottky's large `IS`.
pub(crate) fn breakdown_ibv(current: f64, bv: f64, is: f64, vt: f64) -> f64 {
    current + is * (bv / vt - 1.0)
}

fn is_from(point: Point, n: f64, rs: f64, vt: f64) -> f64 {
    point.current / ((point.voltage - point.current * rs) / (n * vt)).exp_m1()
}

/// Fit the forward law. One point takes `assumed_n` and no series resistance;
/// two points fix two of the three parameters; three or more fit all three.
pub(crate) fn fit_forward(
    points: &[Point],
    vt: f64,
    assumed_n: f64,
    field: &str,
) -> Fitted<ForwardFit> {
    let mut points = points.to_vec();
    points.sort_by(|a, b| a.current.total_cmp(&b.current));
    for pair in points.windows(2) {
        if pair[1].current == pair[0].current || pair[1].voltage <= pair[0].voltage {
            return Err(Invalid::new(
                field,
                "forward points need distinct currents, with the voltage rising as the current \
                 rises",
            ));
        }
    }
    let fit = match points.len() {
        0 => return Err(Invalid::new(field, "no forward points")),
        1 => ForwardFit {
            is: is_from(points[0], assumed_n, 0.0, vt),
            n: assumed_n,
            rs: 0.0,
            note: "is from one forward point".into(),
            assumed: vec![format!(
                "n = {assumed_n} and rs = 0, since one forward point fixes one parameter"
            )],
        },
        2 => two_points(points[0], points[1], vt, assumed_n),
        _ => many_points(&points, vt, field)?,
    };
    let sane = fit.is.is_finite() && fit.is > 0.0 && N_RANGE.contains(&fit.n) && fit.rs >= 0.0;
    if !sane {
        return Err(Invalid::new(
            field,
            format!(
                "these forward points do not fit a diode: n would be {:.3} and rs {:.3e} ohm",
                fit.n, fit.rs
            ),
        ));
    }
    Ok(fit)
}

/// Two points fix two parameters. No series resistance is tried first; if that
/// needs an emission coefficient the technology cannot have, the points reach
/// into the resistive region, so `n` is assumed and `rs` fitted.
fn two_points(low: Point, high: Point, vt: f64, assumed_n: f64) -> ForwardFit {
    let span = (high.current / low.current).ln();
    let rise = high.voltage - low.voltage;
    let n = rise / (vt * span);
    if n <= assumed_n * PLAUSIBLE_N_SHARE {
        return ForwardFit {
            is: is_from(low, n, 0.0, vt),
            n,
            rs: 0.0,
            note: "is and n from two forward points".into(),
            assumed: vec!["rs = 0, since two forward points fix two parameters".into()],
        };
    }
    let rs = (rise - assumed_n * vt * span) / (high.current - low.current);
    ForwardFit {
        is: is_from(low, assumed_n, rs, vt),
        n: assumed_n,
        rs,
        note: "is and rs from two forward points".into(),
        assumed: vec![format!(
            "n = {assumed_n}, since two forward points fix two parameters and rs = 0 would need \
             n = {n:.2}"
        )],
    }
}

/// Least squares on the voltage. For a trial `rs` the law is a straight line
/// in `ln I` against `V - I rs`, so only `rs` is searched.
fn many_points(points: &[Point], vt: f64, field: &str) -> Fitted<ForwardFit> {
    let solve = |rs: f64| -> Option<(f64, f64)> {
        let shifted: Vec<f64> = points.iter().map(|p| p.voltage - p.current * rs).collect();
        let logs: Vec<f64> = points.iter().map(|p| p.current.ln()).collect();
        let (slope, intercept) = fit::line(&shifted, &logs)?;
        (slope > 0.0).then(|| (intercept.exp(), 1.0 / (slope * vt)))
    };
    let error = |rs: f64| -> f64 {
        match solve(rs) {
            Some((is, n)) => points
                .iter()
                .map(|p| (forward_voltage(is, n, rs, vt, p.current) - p.voltage).powi(2))
                .sum(),
            None => f64::INFINITY,
        }
    };
    let ceiling = points
        .iter()
        .map(|p| p.voltage / p.current)
        .fold(f64::INFINITY, f64::min)
        * 0.999;
    let rs = fit::golden_minimum(error, 0.0, ceiling);
    // A search that ends on its lower edge returns a sliver above zero, and a
    // sliver of series resistance is a near-infinite conductance to ngspice,
    // which leaves its matrix singular. One that drops under a microvolt at
    // the highest current is none.
    let highest = points.iter().map(|p| p.current).fold(0.0, f64::max);
    let rs = if rs * highest < NEGLIGIBLE_DROP {
        0.0
    } else {
        rs
    };
    let (is, n) = solve(rs).ok_or_else(|| {
        Invalid::new(
            field,
            "these forward points do not fit a diode: the voltage must rise",
        )
    })?;
    Ok(ForwardFit {
        is,
        n,
        rs,
        note: format!(
            "is, n and rs by least squares on {} forward points",
            points.len()
        ),
        assumed: Vec::new(),
    })
}

/// `CJO`, `VJ` and `M` from points `(reverse voltage, capacitance)`.
pub(crate) fn fit_capacitance(
    points: &[(f64, f64)],
    default_vj: f64,
    field: &str,
) -> Fitted<(f64, f64, f64, Vec<String>)> {
    let law = |cjo: f64, vj: f64, m: f64, reverse: f64| cjo / (1.0 + reverse / vj).powf(m);
    let line_for = |vj: f64| -> Option<(f64, f64)> {
        let xs: Vec<f64> = points.iter().map(|(v, _)| (1.0 + v / vj).ln()).collect();
        let ys: Vec<f64> = points.iter().map(|(_, c)| c.ln()).collect();
        let (slope, intercept) = fit::line(&xs, &ys)?;
        Some((intercept.exp(), -slope))
    };
    let distinct = || Invalid::new(field, "capacitance points need distinct reverse voltages");
    let mut assumed = Vec::new();
    let (cjo, vj, m) = match points.len() {
        1 => {
            assumed.push(format!(
                "vj = {default_vj} and m = {DEFAULT_M}, since one capacitance point fixes one \
                 parameter"
            ));
            let (reverse, capacitance) = points[0];
            (
                capacitance * (1.0 + reverse / default_vj).powf(DEFAULT_M),
                default_vj,
                DEFAULT_M,
            )
        }
        2 => {
            assumed.push(format!(
                "vj = {default_vj}, since two capacitance points fix two parameters"
            ));
            let (cjo, m) = line_for(default_vj).ok_or_else(distinct)?;
            (cjo, default_vj, m)
        }
        _ => {
            let error = |vj: f64| -> f64 {
                match line_for(vj) {
                    Some((cjo, m)) => points
                        .iter()
                        .map(|(v, c)| (law(cjo, vj, m, *v) / c).ln().powi(2))
                        .sum(),
                    None => f64::INFINITY,
                }
            };
            let vj = fit::golden_minimum(error, 0.05, 3.0);
            let (cjo, m) = line_for(vj).ok_or_else(distinct)?;
            (cjo, vj, m)
        }
    };
    if !(cjo.is_finite() && cjo > 0.0 && (0.01..=2.0).contains(&m) && vj > 0.0) {
        return Err(Invalid::new(
            field,
            format!("these capacitance points do not fit a junction: m would be {m:.3}"),
        ));
    }
    Ok((cjo, vj, m, assumed))
}

/// Points with their places in the caller's list: those to fit, and the one
/// held out.
pub(super) type Split<T> = (Vec<(T, usize)>, Option<(T, usize)>);

/// Split a curve into the points to fit and the one to hold out: the middle
/// one, when there are more points than a three-parameter law needs.
pub(super) fn hold_out<T: Copy>(sorted: &[(T, usize)]) -> Split<T> {
    if sorted.len() <= LAW_PARAMETERS {
        return (sorted.to_vec(), None);
    }
    let middle = sorted.len() / 2;
    let mut fit = sorted.to_vec();
    let held = fit.remove(middle);
    (fit, Some(held))
}

pub(super) fn forward_points(characteristics: &[Characteristic]) -> Vec<(Point, usize)> {
    let mut points: Vec<(Point, usize)> = of(characteristics, Quantity::ForwardVoltage)
        .into_iter()
        .map(|c| {
            let point = Point {
                current: c.condition(FORWARD_CURRENT).unwrap_or_default(),
                voltage: c.value,
            };
            (point, c.index)
        })
        .collect();
    points.sort_by(|a, b| a.0.current.total_cmp(&b.0.current));
    points
}

fn capacitance_points(characteristics: &[Characteristic]) -> Vec<((f64, f64), usize)> {
    let mut points: Vec<((f64, f64), usize)> = of(characteristics, Quantity::JunctionCapacitance)
        .into_iter()
        .map(|c| {
            (
                (c.condition(REVERSE_VOLTAGE).unwrap_or_default(), c.value),
                c.index,
            )
        })
        .collect();
    points.sort_by(|a, b| a.0 .0.total_cmp(&b.0 .0));
    points
}

pub(super) fn field_of(characteristics: &[Characteristic], quantity: Quantity) -> String {
    of(characteristics, quantity)
        .first()
        .map_or("characteristics".to_string(), |c| c.field("value"))
}

/// Forward points: each pushes its current into the anode.
pub(super) fn check_forward(
    model: &mut Model,
    bench: &mut Bench,
    points: &[(Point, usize)],
    role: Role,
) {
    for (point, index) in points {
        let id = bench.id();
        let node = format!("{id}a");
        bench
            .dc
            .push(Bench::current_into(&id, &node, point.current));
        bench.dc.push(model.instance(&id, &[&node, "0"]));
        model.checks.push(Check {
            characteristic: *index,
            role,
            expected: point.voltage,
            probe: Probe::Voltage {
                node,
                scale: 1.0,
                divide_by: 1.0,
            },
        });
    }
}

/// A reverse current pushed into the cathode, whose voltage is then the
/// reverse voltage at that current.
fn check_reverse_voltage(model: &mut Model, bench: &mut Bench, characteristic: &Characteristic) {
    let current = characteristic
        .condition(REVERSE_CURRENT)
        .unwrap_or_default();
    check_voltage_at_current(model, bench, characteristic, current);
}

/// A current pushed into the cathode, whose voltage is then the value the
/// characteristic states at that current.
pub(super) fn check_voltage_at_current(
    model: &mut Model,
    bench: &mut Bench,
    characteristic: &Characteristic,
    current: f64,
) {
    let id = bench.id();
    let node = format!("{id}k");
    bench.dc.push(Bench::current_into(&id, &node, current));
    bench.dc.push(model.instance(&id, &["0", &node]));
    model.checks.push(Check {
        characteristic: characteristic.index,
        role: Role::Fit,
        expected: characteristic.value,
        probe: Probe::Voltage {
            node,
            scale: 1.0,
            divide_by: 1.0,
        },
    });
}

/// The reverse current at a reverse voltage. The source delivers it, so its
/// own current reads negative.
pub(super) fn check_leakage(model: &mut Model, bench: &mut Bench, characteristic: &Characteristic) {
    let id = bench.id();
    let node = format!("{id}k");
    let voltage = characteristic
        .condition(REVERSE_VOLTAGE)
        .unwrap_or_default();
    bench.dc.push(Bench::voltage(&id, &node, voltage, false));
    bench.dc.push(model.instance(&id, &["0", &node]));
    model.checks.push(Check {
        characteristic: characteristic.index,
        role: Role::CheckOnly,
        expected: characteristic.value,
        probe: Probe::Current {
            source: format!("V{id}"),
            scale: -1.0,
        },
    });
}

pub(super) fn check_capacitance(
    model: &mut Model,
    bench: &mut Bench,
    characteristics: &[Characteristic],
    points: &[((f64, f64), usize)],
    role: Role,
) {
    for ((reverse, capacitance), index) in points {
        let hz = characteristics[*index]
            .condition(FREQUENCY)
            .unwrap_or(CAPACITANCE_HZ);
        let id = bench.id();
        let node = format!("{id}k");
        bench.ac.push(Bench::voltage(&id, &node, *reverse, true));
        bench.ac.push(model.instance(&id, &["0", &node]));
        model.checks.push(Check {
            characteristic: *index,
            role,
            expected: *capacitance,
            probe: Probe::Capacitance {
                source: format!("V{id}"),
                hz,
            },
        });
    }
}

fn check_impedance(model: &mut Model, bench: &mut Bench, characteristic: &Characteristic) {
    let id = bench.id();
    let node = format!("{id}k");
    let current = characteristic
        .condition(REVERSE_CURRENT)
        .unwrap_or_default();
    bench.ac.push(Bench::ac_current_into(&id, &node, current));
    bench.ac.push(model.instance(&id, &["0", &node]));
    model.checks.push(Check {
        characteristic: characteristic.index,
        role: Role::Fit,
        expected: characteristic.value,
        probe: Probe::Impedance {
            node,
            hz: characteristic.condition(FREQUENCY).unwrap_or(IMPEDANCE_HZ),
        },
    });
}

/// The junction-capacitance card parameters, and the points behind them.
pub(super) type CapacitanceFit = (Vec<(&'static str, f64)>, Split<(f64, f64)>);

/// The junction-capacitance parameters, fitted on all but a held-out point.
pub(super) fn capacitance_parameters(
    model: &mut Model,
    characteristics: &[Characteristic],
    vj: f64,
) -> Fitted<CapacitanceFit> {
    let points = capacitance_points(characteristics);
    if points.is_empty() {
        return Ok((Vec::new(), (Vec::new(), None)));
    }
    let (fit_points, held) = hold_out(&points);
    let values: Vec<(f64, f64)> = fit_points.iter().map(|(point, _)| *point).collect();
    let (cjo, vj, m, assumed) = fit_capacitance(
        &values,
        vj,
        &field_of(characteristics, Quantity::JunctionCapacitance),
    )?;
    model.assumed.extend(assumed);
    model.fitted.push(format!(
        "cjo, vj and m from {} junction capacitance values",
        values.len()
    ));
    Ok((vec![("CJO", cjo), ("VJ", vj), ("M", m)], (fit_points, held)))
}

/// A rectifier, switching, Schottky or LED diode: one `.model D` card.
pub(crate) fn plain(
    kind: Kind,
    name: &str,
    characteristics: &[Characteristic],
    temperature: f64,
) -> Fitted<Model> {
    let technology = match kind {
        Kind::SchottkyDiode => SCHOTTKY,
        Kind::Led => LED,
        _ => SILICON,
    };
    let vt = thermal_voltage(temperature);
    let mut model = Model::new(kind, name, Element::Diode, vec!["A", "K"]);
    let points = forward_points(characteristics);
    let (fit_points, held) = hold_out(&points);
    let values: Vec<Point> = fit_points.iter().map(|(point, _)| *point).collect();
    let forward = fit_forward(
        &values,
        vt,
        technology.assumed_n,
        &field_of(characteristics, Quantity::ForwardVoltage),
    )?;
    model.fitted.push(forward.note.clone());
    model.assumed.extend(forward.assumed.iter().cloned());
    let mut parameters: Vec<(&'static str, f64)> =
        vec![("IS", forward.is), ("N", forward.n), ("RS", forward.rs)];

    let breakdown = of(characteristics, Quantity::BreakdownVoltage)
        .first()
        .copied();
    if let Some(breakdown) = breakdown {
        let current = breakdown.condition(REVERSE_CURRENT).unwrap_or_default();
        let bv = breakdown.value - current * forward.rs;
        if bv <= 0.0 {
            return Err(Invalid::new(
                breakdown.field("value"),
                "the breakdown voltage is below the series resistance's drop at its current",
            ));
        }
        let ibv = breakdown_ibv(current, bv, forward.is, vt);
        parameters.extend([("BV", bv), ("IBV", ibv), ("NBV", 1.0)]);
        model
            .fitted
            .push("bv and ibv from the breakdown voltage".into());
        model
            .assumed
            .push("nbv = 1: one breakdown value fixes the knee's place, not its sharpness".into());
    }

    let (capacitance, (capacitance_fit, capacitance_held)) =
        capacitance_parameters(&mut model, characteristics, technology.vj)?;
    parameters.extend(capacitance);

    let eg = if kind == Kind::Led {
        // At constant current dVf/dT = (Vf - EG - XTI Vt) / T. Silicon's EG
        // would give an LED the wrong sign, so EG comes from a typical tempco.
        // The law holds for the junction, so the series resistance's drop
        // comes off the terminal voltage first.
        let middle = values[values.len() / 2];
        let junction = middle.voltage - middle.current * forward.rs;
        let kelvin = temperature + fit::ZERO_CELSIUS;
        let eg = junction - technology.xti * vt - kelvin * LED_TEMPCO;
        model.assumed.push(format!(
            "eg = {eg:.3} from a forward tempco of {} mV/K, typical of LEDs",
            LED_TEMPCO * 1e3
        ));
        eg
    } else {
        model.assumed.push(format!(
            "eg = {} and xti = {}, {} defaults, set how the model follows temperature",
            technology.eg, technology.xti, technology.name
        ));
        technology.eg
    };
    parameters.extend([("EG", eg), ("XTI", technology.xti), ("TNOM", temperature)]);
    model.body = diode_card(name, &parameters);
    model.parameters = parameters
        .iter()
        .map(|(key, value)| (key.to_string(), *value))
        .collect();

    let mut bench = Bench::default();
    check_forward(&mut model, &mut bench, &fit_points, Role::Fit);
    if let Some(held) = held {
        check_forward(&mut model, &mut bench, &[held], Role::HeldOut);
    }
    if let Some(breakdown) = breakdown {
        check_reverse_voltage(&mut model, &mut bench, breakdown);
    }
    for leakage in of(characteristics, Quantity::ReverseCurrent) {
        check_leakage(&mut model, &mut bench, leakage);
    }
    if !of(characteristics, Quantity::ReverseCurrent).is_empty() {
        model
            .assumed
            .push("reverse current is compared, not fitted: the diode law sets it from is".into());
    }
    check_capacitance(
        &mut model,
        &mut bench,
        characteristics,
        &capacitance_fit,
        Role::Fit,
    );
    if let Some(held) = capacitance_held {
        check_capacitance(
            &mut model,
            &mut bench,
            characteristics,
            &[held],
            Role::HeldOut,
        );
    }
    model.bench = bench;
    Ok(model)
}

/// `NBV` and the series resistance from one or two dynamic impedances. Both
/// follow `NBV Vt / I + RZ`. A negative resistance means the data asks for a
/// sharper knee than the slope allows, so the resistance goes to zero.
fn breakdown_slope(
    impedances: &[&Characteristic],
    vt: f64,
    model: &mut Model,
) -> Fitted<(f64, f64)> {
    let point = |c: &Characteristic| (c.condition(REVERSE_CURRENT).unwrap_or_default(), c.value);
    let mut points: Vec<(f64, f64)> = impedances.iter().map(|c| point(c)).collect();
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (current, impedance) = *points.last().expect("a Zener has an impedance");
    if let [(low_current, low_impedance), _] = points.as_slice() {
        if *low_current >= current || *low_impedance <= impedance {
            return Err(Invalid::new(
                impedances[0].field("value"),
                "the impedance at the lower current must be the higher one",
            ));
        }
        let nbv = (low_impedance - impedance) / (vt * (1.0 / low_current - 1.0 / current));
        let rz = impedance - nbv * vt / current;
        if rz >= 0.0 {
            model
                .fitted
                .push("nbv and the series resistance from two dynamic impedances".into());
            return Ok((nbv, rz));
        }
        model.assumed.push(
            "the two impedances ask for a sharper knee than the model allows; fitted to the \
             higher current's"
                .into(),
        );
        return Ok((impedance * current / vt, 0.0));
    }
    let rz = impedance - vt / current;
    if rz >= 0.0 {
        model
            .assumed
            .push("nbv = 1: one dynamic impedance fixes the slope at its current only".into());
        return Ok((1.0, rz));
    }
    model
        .assumed
        .push("the dynamic impedance is below vt / I, so the series resistance is zero".into());
    Ok((impedance * current / vt, 0.0))
}

/// The breakdown diode of a Zener or a TVS, as its card holds it.
pub(super) struct Breakdown {
    pub is: f64,
    pub nbv: f64,
    pub rz: f64,
    pub bv: f64,
    pub ibv: f64,
}

/// The breakdown card for a junction that must read `voltage` at
/// `test_current`, given the slope `(nbv, rz)` its data asks for and the
/// `(current, impedance)` points behind it. `field` names the value to blame if
/// the series resistance's drop leaves no breakdown voltage.
pub(super) fn breakdown_card(
    model: &mut Model,
    field: &str,
    voltage: f64,
    test_current: f64,
    slope: (f64, f64),
    slope_points: &[(f64, f64)],
    vt: f64,
) -> Fitted<Breakdown> {
    let (nbv, rz) = slope;
    // A soft knee asks for a large NBV, which pulls ngspice's knee down. A
    // larger IS for the breakdown diode keeps the knee in place; past the top
    // of its range, NBV is held and the rest of the slope is resistance.
    let span = (1.0 - KNEE_FLOOR) * voltage;
    let breakdown_is = (test_current * (-span / (nbv * vt)).exp())
        .clamp(BREAKDOWN_IS_RANGE.0, BREAKDOWN_IS_RANGE.1);
    let ceiling = span / (vt * (test_current / breakdown_is).ln());
    let (nbv, rz) = if nbv > ceiling * (1.0 + 1e-9) {
        let (current, impedance) = slope_points
            .iter()
            .copied()
            .fold(
                (0.0, 0.0),
                |best, point| if point.0 > best.0 { point } else { best },
            );
        model.assumed.push(format!(
            "the knee is softer than the model can hold: nbv is held at {ceiling:.2} and the \
             rest of the slope is series resistance, so the knee's impedance reads low"
        ));
        (ceiling, (impedance - ceiling * vt / current).max(0.0))
    } else {
        (nbv, rz)
    };
    // A slope the law alone explains leaves a rounding sliver of resistance,
    // which ngspice reads as a near-infinite conductance.
    let highest = slope_points
        .iter()
        .map(|point| point.0)
        .fold(test_current, f64::max);
    let rz = if rz * highest < NEGLIGIBLE_DROP {
        0.0
    } else {
        rz
    };
    let bv = voltage - test_current * rz;
    if bv <= 0.0 {
        return Err(Invalid::new(
            field,
            "the voltage is below the series resistance's drop at its current",
        ));
    }
    let ibv = breakdown_ibv(test_current, bv, breakdown_is, vt);
    model
        .fitted
        .push("bv and ibv from the breakdown voltage at its test current".into());
    Ok(Breakdown {
        is: breakdown_is,
        nbv,
        rz,
        bv,
        ibv,
    })
}

/// A Zener: a forward diode and a breakdown diode, each with its own series
/// resistance.
pub(crate) fn zener(
    name: &str,
    characteristics: &[Characteristic],
    temperature: f64,
) -> Fitted<Model> {
    let vt = thermal_voltage(temperature);
    let mut model = Model::new(Kind::Zener, name, Element::Subcircuit, vec!["A", "K"]);
    let zener_voltage = of(characteristics, Quantity::ZenerVoltage)[0];
    let impedances = of(characteristics, Quantity::DynamicImpedance);
    let (nbv, rz) = breakdown_slope(&impedances, vt, &mut model)?;
    let test_current = zener_voltage.condition(REVERSE_CURRENT).unwrap_or_default();
    let slope_points: Vec<(f64, f64)> = impedances
        .iter()
        .map(|c| (c.condition(REVERSE_CURRENT).unwrap_or_default(), c.value))
        .collect();
    let Breakdown {
        is: breakdown_is,
        nbv,
        rz,
        bv,
        ibv,
    } = breakdown_card(
        &mut model,
        &zener_voltage.field("value"),
        zener_voltage.value,
        test_current,
        (nbv, rz),
        &slope_points,
        vt,
    )?;

    let points = forward_points(characteristics);
    let (fit_points, held) = hold_out(&points);
    let (is, n, rs) = if points.is_empty() {
        model.assumed.push(format!(
            "no forward values: a silicon junction, is = {} A and n = {GENERIC_N}",
            number(GENERIC_IS)
        ));
        (GENERIC_IS, GENERIC_N, 0.0)
    } else {
        let values: Vec<Point> = fit_points.iter().map(|(point, _)| *point).collect();
        let forward = fit_forward(
            &values,
            vt,
            SILICON.assumed_n,
            &field_of(characteristics, Quantity::ForwardVoltage),
        )?;
        model.fitted.push(forward.note.clone());
        model.assumed.extend(forward.assumed.iter().cloned());
        (forward.is, forward.n, forward.rs)
    };
    let forward: Vec<(&'static str, f64)> = vec![
        ("IS", is),
        ("N", n),
        ("RS", rs),
        ("EG", SILICON.eg),
        ("XTI", SILICON.xti),
        ("TNOM", temperature),
    ];
    let mut breaking: Vec<(&'static str, f64)> = vec![
        ("IS", breakdown_is),
        ("N", BREAKDOWN_N),
        ("RS", rz),
        ("BV", bv),
        ("IBV", ibv),
        ("NBV", nbv),
    ];
    let (capacitance, (capacitance_fit, capacitance_held)) =
        capacitance_parameters(&mut model, characteristics, SILICON.vj)?;
    breaking.extend(capacitance);
    breaking.push(("TNOM", temperature));
    model
        .assumed
        .push("the breakdown voltage does not follow temperature".into());
    model.body = format!(
        ".subckt {name} A K\nDF A K {name}_f\nDB A K {name}_b\n{}{}.ends {name}\n",
        diode_card(&format!("{name}_f"), &forward),
        diode_card(&format!("{name}_b"), &breaking),
    );
    model.parameters = forward
        .iter()
        .map(|(key, value)| (format!("forward.{key}"), *value))
        .chain(
            breaking
                .iter()
                .map(|(key, value)| (format!("breakdown.{key}"), *value)),
        )
        .collect();

    let mut bench = Bench::default();
    check_forward(&mut model, &mut bench, &fit_points, Role::Fit);
    if let Some(held) = held {
        check_forward(&mut model, &mut bench, &[held], Role::HeldOut);
    }
    check_reverse_voltage(&mut model, &mut bench, zener_voltage);
    for impedance in &impedances {
        check_impedance(&mut model, &mut bench, impedance);
    }
    for leakage in of(characteristics, Quantity::ReverseCurrent) {
        check_leakage(&mut model, &mut bench, leakage);
    }
    check_capacitance(
        &mut model,
        &mut bench,
        characteristics,
        &capacitance_fit,
        Role::Fit,
    );
    if let Some(held) = capacitance_held {
        check_capacitance(
            &mut model,
            &mut bench,
            characteristics,
            &[held],
            Role::HeldOut,
        );
    }
    model.bench = bench;
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::{build, text, Kind};
    use super::*;
    use serde_json::{json, Value};

    fn close(a: f64, b: f64, share: f64) -> bool {
        (a - b).abs() <= share * b.abs()
    }

    fn forward(current: f64, voltage: f64) -> Value {
        json!({
            "quantity": "forward_voltage", "value": voltage,
            "conditions": { "forward_current_amps": current }, "statistic": "typical"
        })
    }

    /// Points the law itself makes, for the fit to find again.
    fn law_points(is: f64, n: f64, rs: f64, currents: &[f64]) -> Vec<Point> {
        let vt = thermal_voltage(25.0);
        currents
            .iter()
            .map(|&current| Point {
                current,
                voltage: forward_voltage(is, n, rs, vt, current),
            })
            .collect()
    }

    #[test]
    fn three_points_recover_the_law_that_made_them() {
        let points = law_points(2.52e-9, 1.752, 0.568, &[1e-4, 1e-3, 1e-2, 0.1, 0.3]);
        let fit = fit_forward(&points, thermal_voltage(25.0), 1.8, "f").unwrap();
        assert!(close(fit.n, 1.752, 1e-3), "{fit:?}");
        assert!(close(fit.rs, 0.568, 1e-2), "{fit:?}");
        assert!(close(fit.is, 2.52e-9, 2e-2), "{fit:?}");
    }

    #[test]
    fn a_resistance_the_points_do_not_need_is_exactly_none() {
        // The exponential region alone: the best series resistance is zero,
        // and a sliver above it would leave ngspice's matrix singular.
        let points = law_points(1e-12, 1.4, 0.0, &[1e-6, 1e-5, 1e-4, 1e-3]);
        let fit = fit_forward(&points, thermal_voltage(25.0), 1.8, "f").unwrap();
        assert_eq!(fit.rs, 0.0, "{fit:?}");
    }

    #[test]
    fn two_low_points_fit_n_and_two_spread_points_fit_rs() {
        let vt = thermal_voltage(25.0);
        let low = law_points(1e-14, 1.3, 0.0, &[1e-5, 1e-3]);
        let fit = fit_forward(&low, vt, 1.8, "f").unwrap();
        assert!(close(fit.n, 1.3, 1e-6) && fit.rs == 0.0, "{fit:?}");
        let spread = law_points(5e-9, 1.8, 0.05, &[1e-3, 3.0]);
        let fit = fit_forward(&spread, vt, 1.8, "f").unwrap();
        // The fit drops the law's + 1, a part in a million here.
        assert!(close(fit.rs, 0.05, 1e-4), "{fit:?}");
        assert!(fit.assumed.iter().any(|note| note.contains("n = 1.8")));
    }

    #[test]
    fn falling_or_repeated_points_are_refused() {
        let vt = thermal_voltage(25.0);
        let falling = [
            Point {
                current: 1e-3,
                voltage: 0.7,
            },
            Point {
                current: 1e-2,
                voltage: 0.6,
            },
        ];
        assert!(fit_forward(&falling, vt, 1.8, "f").is_err());
        let repeated = [
            Point {
                current: 1e-3,
                voltage: 0.6,
            },
            Point {
                current: 1e-3,
                voltage: 0.7,
            },
        ];
        assert!(fit_forward(&repeated, vt, 1.8, "f").is_err());
        // A Schottky's 450 typed in volts where millivolts were meant.
        let millivolts = [
            Point {
                current: 1e-3,
                voltage: 300.0,
            },
            Point {
                current: 1e-1,
                voltage: 450.0,
            },
        ];
        assert!(fit_forward(&millivolts, vt, 1.05, "f").is_err());
    }

    #[test]
    fn capacitance_points_recover_the_junction_law() {
        let law = |v: f64| 4e-12 / (1.0 + v / 0.65).powf(0.33);
        let points: Vec<(f64, f64)> = [0.0, 1.0, 5.0, 20.0, 50.0]
            .iter()
            .map(|&v| (v, law(v)))
            .collect();
        let (cjo, vj, m, _) = fit_capacitance(&points, 0.7, "c").unwrap();
        assert!(
            close(cjo, 4e-12, 1e-3) && close(vj, 0.65, 1e-2) && close(m, 0.33, 1e-2),
            "{cjo} {vj} {m}"
        );
        let (cjo, _, m, assumed) = fit_capacitance(&[(10.0, 1e-12)], 0.7, "c").unwrap();
        assert!(close(cjo, 1e-12 * (1.0 + 10.0 / 0.7f64).sqrt(), 1e-9) && m == 0.5);
        assert_eq!(assumed.len(), 1);
    }

    #[test]
    fn a_long_curve_holds_its_middle_point_out() {
        let characteristics = parsed(json!([
            forward(1e-4, 0.24),
            forward(1e-3, 0.32),
            forward(1e-2, 0.40),
            forward(0.1, 0.80)
        ]));
        let model = build(Kind::SchottkyDiode, "bat54_fit", &characteristics, 25.0).unwrap();
        let roles: Vec<(usize, Role)> = model
            .checks
            .iter()
            .map(|check| (check.characteristic, check.role))
            .collect();
        assert_eq!(
            roles,
            [
                (0, Role::Fit),
                (1, Role::Fit),
                (3, Role::Fit),
                (2, Role::HeldOut)
            ]
        );
    }

    #[test]
    fn a_plain_diode_is_one_card_in_the_fitted_grammar() {
        let characteristics = parsed(json!([
            forward(1e-4, 0.24),
            forward(1e-3, 0.32),
            forward(1e-2, 0.40),
            { "quantity": "breakdown_voltage", "value": 30.0,
              "conditions": { "reverse_current_amps": 1e-4 }, "statistic": "minimum" },
            { "quantity": "reverse_current", "value": 2e-6,
              "conditions": { "reverse_voltage_volts": 25.0 }, "statistic": "maximum" },
            { "quantity": "junction_capacitance", "value": 10e-12,
              "conditions": { "reverse_voltage_volts": 1.0 }, "statistic": "typical" }
        ]));
        let model = build(Kind::SchottkyDiode, "bat54_fit", &characteristics, 25.0).unwrap();
        let checked = text::check_fitted(model.file().as_bytes()).unwrap();
        assert_eq!(checked.card, text::Card::Diode);
        assert_eq!(checked.name, "bat54_fit");
        assert_eq!(fitted(&model, "EG"), 0.69);
        assert_eq!(fitted(&model, "TNOM"), 25.0);
        let checked_roles: Vec<Role> = model.checks.iter().map(|check| check.role).collect();
        assert_eq!(
            checked_roles,
            [
                Role::Fit,
                Role::Fit,
                Role::Fit,
                Role::Fit,
                Role::CheckOnly,
                Role::Fit
            ]
        );
        assert!(
            model.bench.dc.contains(&"Db1 b1a 0 bat54_fit".to_string()),
            "{:?}",
            model.bench.dc
        );
        assert!(model.assumed.iter().any(|note| note.contains("limits")));
    }

    #[test]
    fn a_zener_splits_its_breakdown_from_its_forward_diode() {
        let characteristics = parsed(json!([
            { "quantity": "zener_voltage", "value": 5.1,
              "conditions": { "reverse_current_amps": 0.005 }, "statistic": "typical" },
            { "quantity": "dynamic_impedance", "value": 30.0,
              "conditions": { "reverse_current_amps": 0.005 }, "statistic": "maximum" },
            { "quantity": "dynamic_impedance", "value": 480.0,
              "conditions": { "reverse_current_amps": 0.00025 }, "statistic": "maximum" },
            forward(0.01, 0.9)
        ]));
        let model = build(Kind::Zener, "bzx84c5v1_fit", &characteristics, 25.0).unwrap();
        let checked = text::check_fitted(model.file().as_bytes()).unwrap();
        assert_eq!(checked.pins(), ["A", "K"]);
        let vt = thermal_voltage(25.0);
        let nbv = fitted(&model, "breakdown.NBV");
        let rz = fitted(&model, "breakdown.RS");
        assert!(close(nbv * vt / 0.005 + rz, 30.0, 1e-9));
        assert!(close(nbv * vt / 0.00025 + rz, 480.0, 1e-9));
        assert!(close(
            fitted(&model, "breakdown.BV") + 0.005 * rz,
            5.1,
            1e-12
        ));
    }

    #[test]
    fn a_zener_without_forward_values_says_its_junction_was_assumed() {
        let characteristics = parsed(json!([
            { "quantity": "zener_voltage", "value": 12.0,
              "conditions": { "reverse_current_amps": 0.002 }, "statistic": "typical" },
            { "quantity": "dynamic_impedance", "value": 25.0,
              "conditions": { "reverse_current_amps": 0.002 }, "statistic": "maximum" }
        ]));
        let model = build(Kind::Zener, "bzx12_fit", &characteristics, 25.0).unwrap();
        assert!(model
            .assumed
            .iter()
            .any(|note| note.contains("no forward values")));
        assert_eq!(fitted(&model, "forward.IS"), GENERIC_IS);
    }
}
