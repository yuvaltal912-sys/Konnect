//! Power MOSFETs, on ngspice's VDMOS model inside a three-pin subcircuit.
//!
//! The subcircuit keeps KiCad's export to the three pins a symbol has: a bare
//! VDMOS card on a part with four or more pins would make KiCad add thermal
//! nodes by position.
//!
//! The channel follows ngspice's own law, which a scratch run reproduced to six
//! digits: with `vgst = ksub ln(1 + exp((Vgs - VTO) / ksub))` and
//! `beta = KP / (1 + THETA max(Vgs - VTO, 0))`, the current is
//! `beta Vds (vgst - Vds / 2)` below saturation and `beta vgst^2 / 2` above it.
//! `RD` sits in series with the drain. So the fit runs on the law itself, with
//! no round trip to the engine, and the bench then checks what the engine reads.
//!
//! Every value is a magnitude. A P-channel part is the same fit with `PCHAN`,
//! a negative `VTO` (ngspice mirrors the card around it) and bench sources of
//! the opposite sign.

use super::bench::{Bench, Charge, Check, Probe, Role};
use super::diode::{fit_capacitance, fit_forward, forward_points, hold_out, Point, SILICON};
use super::fit::{least_squares, root, thermal_voltage};
use super::text::{number, vdmos_card};
use super::{
    of, Characteristic, Element, Fitted, Invalid, Kind, Model, Quantity, DRAIN_CURRENT,
    DRAIN_SOURCE_VOLTAGE, FREQUENCY, GATE_SOURCE_VOLTAGE,
};

/// ngspice's default `KSUBTHRES`, the softness of the turn-on knee.
const KSUB: f64 = 0.1;
/// The gate resistance is read at this frequency, unless the caller names one.
const GATE_RESISTANCE_HZ: f64 = 1e6;
/// Small-signal tests, unless a characteristic names its own frequency.
const CAPACITANCE_HZ: f64 = 1e6;
/// A body diode's junction potential, when the capacitance points cannot fix it.
const BODY_VJ: f64 = 0.8;
/// Rounds of fitting the channel, then the drain resistance, each on the last.
const ROUNDS: usize = 3;
const LAW_POINTS: usize = 3;

/// The parameters of the channel law.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Law {
    pub vto: f64,
    pub kp: f64,
    pub theta: f64,
    pub rd: f64,
}

impl Law {
    /// The channel current for a gate and drain-source voltage inside the die.
    pub(crate) fn inner(&self, vgs: f64, vds: f64) -> f64 {
        let vgst = KSUB * ((vgs - self.vto) / KSUB).exp().ln_1p();
        let beta = self.kp / (1.0 + self.theta * (vgs - self.vto).max(0.0));
        if vds < vgst {
            beta * vds * (vgst - vds / 2.0)
        } else {
            beta * vgst * vgst / 2.0
        }
    }

    /// The drain current at pin voltages, with `RD` in series with the drain.
    pub(crate) fn current(&self, vgs: f64, vds: f64) -> f64 {
        if self.rd <= 0.0 {
            return self.inner(vgs, vds);
        }
        // The current the channel carries falls as the drop across RD rises.
        let ceiling = vds / self.rd;
        root(
            |current| self.inner(vgs, vds - current * self.rd) - current,
            0.0,
            ceiling,
        )
        .unwrap_or(0.0)
    }

    /// The drain-source voltage at the pins that carries `current` at `vgs`,
    /// or none when the channel cannot carry it.
    pub(crate) fn drop(&self, vgs: f64, current: f64) -> Option<f64> {
        let vgst = KSUB * ((vgs - self.vto) / KSUB).exp().ln_1p();
        let saturated = self.inner(vgs, vgst.max(f64::MIN_POSITIVE) * 2.0);
        if !(saturated.is_finite() && saturated > current) {
            return None;
        }
        let inside = root(|vds| self.inner(vgs, vds) - current, 0.0, vgst)?;
        Some(inside + current * self.rd)
    }
}

/// ngspice's gate-drain capacitance at a gate-drain voltage `vgd` at or below
/// zero: `s atan(a vgd) + y`, with `s = (max - min) / (1 + pi / 2)` and
/// `y = max - s`, so it runs from `max - s` at zero to `min` far below.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GateDrain {
    pub max: f64,
    pub min: f64,
    pub a: f64,
}

impl GateDrain {
    pub(crate) fn at_vds(&self, vds: f64) -> f64 {
        let s = (self.max - self.min) / (1.0 + std::f64::consts::FRAC_PI_2);
        let y = self.max - s;
        s * (-self.a * vds).atan() + y
    }
}

/// `max`, `min` and `a` for Crss values `(Vds, capacitance)`, by least squares
/// on the log of the capacitance.
fn fit_gate_drain(points: &[(f64, f64)]) -> GateDrain {
    let highest = points.iter().map(|p| p.1).fold(0.0, f64::max);
    let lowest = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let middle = points.iter().map(|p| p.0).sum::<f64>() / points.len() as f64;
    let law = |x: &[f64]| {
        let max = x[0].exp();
        GateDrain {
            max,
            min: max * (-x[1]).exp(),
            a: x[2].exp(),
        }
    };
    let found = least_squares(
        |x| {
            let law = law(x);
            points
                .iter()
                .map(|(vds, c)| (law.at_vds(*vds).max(1e-300) / c).ln())
                .collect()
        },
        &[
            (2.0 * highest).ln(),
            (2.0 * highest / lowest).ln().max(0.5),
            (1.0 / middle).ln(),
        ],
        |x| {
            x[0] = x[0].clamp((highest * 1.0001).ln(), (highest * 100.0).ln());
            x[1] = x[1].clamp(0.01, 12.0);
            x[2] = x[2].clamp(-7.0, 7.0);
        },
    );
    law(&found)
}

/// One transfer value: the drain current at a gate and drain voltage.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Transfer {
    vgs: f64,
    vds: f64,
    current: f64,
}

/// The residuals of a channel fit: the log of predicted over stated current at
/// every transfer value, the threshold's point included.
fn residuals(law: &Law, points: &[Transfer]) -> Vec<f64> {
    points
        .iter()
        .map(|point| {
            let predicted = law.current(point.vgs, point.vds).max(1e-300);
            (predicted / point.current).ln()
        })
        .collect()
}

/// `VTO`, `KP` and `THETA` for a given `RD`.
fn fit_channel(points: &[Transfer], start: Law) -> Law {
    let rd = start.rd;
    let found = least_squares(
        |x| {
            let law = Law {
                vto: x[0],
                kp: x[1].exp(),
                theta: x[2],
                rd,
            };
            residuals(&law, points)
        },
        &[start.vto, start.kp.ln(), start.theta],
        |x| {
            x[0] = x[0].clamp(0.05, 20.0);
            x[1] = x[1].clamp(-12.0, 12.0);
            x[2] = x[2].clamp(0.0, 10.0);
        },
    );
    Law {
        vto: found[0],
        kp: found[1].exp(),
        theta: found[2],
        rd,
    }
}

/// The largest absolute log error of a law over some points.
fn worst_log_error(law: &Law, points: &[Transfer]) -> f64 {
    residuals(law, points)
        .into_iter()
        .fold(0.0, |worst, r| worst.max(r.abs()))
}

pub(crate) fn build(
    kind: Kind,
    name: &str,
    characteristics: &[Characteristic],
    temperature: f64,
) -> Fitted<Model> {
    let p_channel = kind == Kind::PMosfet;
    let sign = if p_channel { -1.0 } else { 1.0 };
    let vt = thermal_voltage(temperature);
    let mut model = Model::new(kind, name, Element::Subcircuit, vec!["D", "G", "S"]);

    let threshold = of(characteristics, Quantity::GateThresholdVoltage)[0];
    let threshold_current = threshold.condition(DRAIN_CURRENT).unwrap_or_default();
    let on = of(characteristics, Quantity::OnResistance)[0];
    let on_vgs = on.condition(GATE_SOURCE_VOLTAGE).unwrap_or_default();
    let on_current = on.condition(DRAIN_CURRENT).unwrap_or_default();

    // The curve: sorted by gate voltage, with its middle value held out when
    // there are more than the law needs.
    let mut curve: Vec<(Transfer, usize)> = of(characteristics, Quantity::DrainCurrent)
        .into_iter()
        .map(|c| {
            (
                Transfer {
                    vgs: c.condition(GATE_SOURCE_VOLTAGE).unwrap_or_default(),
                    vds: c.condition(DRAIN_SOURCE_VOLTAGE).unwrap_or_default(),
                    current: c.value,
                },
                c.index,
            )
        })
        .collect();
    curve.sort_by(|a, b| a.0.vgs.total_cmp(&b.0.vgs));
    if curve
        .windows(2)
        .any(|pair| pair[0].0.vgs == pair[1].0.vgs && pair[0].0.vds == pair[1].0.vds)
    {
        return Err(Invalid::new(
            "characteristics",
            "two drain_current values share one gate and drain voltage",
        ));
    }
    let threshold_point = Transfer {
        vgs: threshold.value,
        vds: threshold.value,
        current: threshold_current,
    };
    let (fit_curve, held) = hold_out(&curve);
    let mut fit_points: Vec<Transfer> = fit_curve.iter().map(|(point, _)| *point).collect();
    fit_points.push(threshold_point);
    if fit_points.len() < LAW_POINTS {
        return Err(Invalid::new(
            "characteristics",
            "the threshold and the drain currents give fewer values than the channel law needs",
        ));
    }

    // Start from the knee: VTO sits a little above the threshold voltage, and
    // KP follows from the highest current.
    let top = curve
        .last()
        .map(|(point, _)| *point)
        .unwrap_or(threshold_point);
    let vto0 = (threshold.value + 0.4).min(top.vgs - 0.1).max(0.1);
    let overdrive = (top.vgs - vto0).max(0.2);
    let mut law = Law {
        vto: vto0,
        kp: (2.0 * top.current / (overdrive * overdrive)).max(1e-6),
        theta: 0.0,
        rd: 0.0,
    };
    let mut clipped = false;
    for _ in 0..ROUNDS {
        law = fit_channel(&fit_points, law);
        let target = on.value * on_current;
        match law.drop(on_vgs, on_current) {
            None => {
                return Err(Invalid::new(
                    on.field("conditions.drain_current_amps"),
                    "the channel fitted to the transfer values cannot carry the on-resistance's \
                     current at its gate voltage; check that the currents and gate voltages are \
                     in amps and volts",
                ));
            }
            Some(channel) => {
                let inside = channel - on_current * law.rd;
                if inside > target {
                    law.rd = 0.0;
                    clipped = true;
                } else {
                    law.rd = (target - inside) / on_current;
                    clipped = false;
                }
            }
        }
    }
    if clipped {
        model.assumed.push(
            "the channel alone is above the stated on-resistance, so rd is zero and the model's \
             on-resistance reads high"
                .into(),
        );
    }
    if !(law.vto.is_finite() && law.kp.is_finite() && law.kp > 0.0) {
        return Err(Invalid::new(
            "characteristics",
            "the transfer values do not fit the MOSFET channel law",
        ));
    }
    model.fitted.push(
        "vto, kp and theta from the threshold and the transfer values, then rd from the \
         on-resistance"
            .into(),
    );
    let spread = worst_log_error(&law, &fit_points);
    if spread > 0.25 {
        model.assumed.push(format!(
            "the channel law follows the transfer values only to within a factor of {:.2}",
            spread.exp()
        ));
    }
    model.assumed.push(
        "rs = 0, lambda = 0, ksubthres and mtriode at ngspice's defaults: all series \
         resistance is in rd, and the saturation current does not rise with drain voltage"
            .into(),
    );
    let capacitances = capacitance_side(&mut model, characteristics)?;
    if capacitances.parameters.is_empty() {
        model.assumed.push(
            "no capacitance values: the gate and drain capacitances are ngspice's defaults, so \
             switching speed is not the part's"
                .into(),
        );
    }
    model.assumed.push(
        "no gate-charge or reverse-recovery values are fitted: the body diode has no transit \
         time"
            .into(),
    );

    let mut parameters: Vec<(&'static str, f64)> = vec![
        ("VTO", sign * law.vto),
        ("KP", law.kp),
        ("THETA", law.theta),
    ];
    if law.rd > 0.0 {
        parameters.push(("RD", law.rd));
    }

    if let Some(gate) = of(characteristics, Quantity::GateResistance).first() {
        parameters.push(("RG", gate.value));
    }
    parameters.extend(capacitances.parameters.iter().copied());

    // The body diode, from its forward voltages.
    let body = forward_points(characteristics);
    let (body_fit, body_held) = hold_out(&body);
    if body.is_empty() {
        model
            .assumed
            .push("no forward values: the body diode is ngspice's default".into());
    } else {
        let values: Vec<Point> = body_fit.iter().map(|(point, _)| *point).collect();
        let forward = fit_forward(
            &values,
            vt,
            SILICON.assumed_n,
            &of(characteristics, Quantity::ForwardVoltage)[0].field("value"),
        )?;
        model.fitted.push(format!("body diode: {}", forward.note));
        model.assumed.extend(forward.assumed.iter().cloned());
        parameters.push(("IS", forward.is));
        parameters.push(("N", forward.n));
        if forward.rs > 0.0 {
            parameters.push(("RB", forward.rs));
        }
    }
    parameters.push(("TNOM", temperature));

    let inner = format!("{name}_m");
    model.body = format!(
        ".subckt {name} D G S\nM1 D G S {inner}\n{}.ends {name}\n",
        vdmos_card(&inner, p_channel, &parameters)
    );
    model.parameters = parameters
        .iter()
        .map(|(key, value)| (format!("vdmos.{key}"), *value))
        .collect();

    let mut bench = Bench::default();
    check_threshold(&mut model, &mut bench, threshold, sign);
    check_on_resistance(&mut model, &mut bench, on, sign);
    for (point, index) in &fit_curve {
        check_transfer(&mut model, &mut bench, *point, *index, Role::Fit, sign);
    }
    if let Some((point, index)) = held {
        check_transfer(&mut model, &mut bench, point, index, Role::HeldOut, sign);
    }
    for (point, index) in &body_fit {
        check_body_diode(&mut model, &mut bench, *point, *index, Role::Fit, sign);
    }
    if let Some((point, index)) = body_held {
        check_body_diode(&mut model, &mut bench, point, index, Role::HeldOut, sign);
    }
    if let Some(gate) = of(characteristics, Quantity::GateResistance).first() {
        check_gate_resistance(&mut model, &mut bench, gate);
    }
    for cap in &capacitances.crss_fit {
        check_capacitance(
            &mut model,
            &mut bench,
            Which::ReverseTransfer,
            cap,
            Role::Fit,
            sign,
        );
    }
    if let Some(cap) = &capacitances.crss_held {
        check_capacitance(
            &mut model,
            &mut bench,
            Which::ReverseTransfer,
            cap,
            Role::HeldOut,
            sign,
        );
    }
    for cap in &capacitances.ciss {
        check_capacitance(&mut model, &mut bench, Which::Input, cap, Role::Fit, sign);
    }
    for cap in &capacitances.coss {
        check_capacitance(&mut model, &mut bench, Which::Output, cap, Role::Fit, sign);
    }
    check_charges(&mut model, &mut bench, characteristics, sign);
    model.bench = bench;
    Ok(model)
}

/// The time a gate-charge bench gives the gate current to deliver its charge,
/// and the time it runs.
const CHARGE_SECONDS: f64 = 250e-9;
const CHARGE_RUN_SECONDS: f64 = 1.0e-6;
const CHARGE_DELAY: &str = "1.000000e-8";
const CHARGE_TOLERANCES: &str = ".options reltol=1e-3 vntol=1e-6 abstol=1e-12";

/// Gate charge and gate-drain charge, as a clamped inductive turn-on: a
/// current source Id from the supply into the drain with a diode back to the
/// supply, and a constant current into the gate. They are checked, not fitted:
/// the charges follow from the capacitances, and a datasheet's figure can sit
/// well off the curve its capacitances draw.
fn check_charges(
    model: &mut Model,
    bench: &mut Bench,
    characteristics: &[Characteristic],
    sign: f64,
) {
    let total = of(characteristics, Quantity::GateCharge).first().copied();
    let miller = of(characteristics, Quantity::GateDrainCharge)
        .first()
        .copied();
    let Some(sizing) = total.map(|c| c.value).or(miller.map(|c| 4.0 * c.value)) else {
        return;
    };
    let gate_current = sizing / CHARGE_SECONDS;
    let clamp_volts = total
        .and_then(|c| c.condition(GATE_SOURCE_VOLTAGE))
        .unwrap_or(10.0);
    bench.tran_stop = CHARGE_RUN_SECONDS;
    // The bench's tight tolerances, made for a breakdown, stall a transient at
    // the gate current's first edge (a timestep of 1e-21 s).
    bench.tran.push(CHARGE_TOLERANCES.to_string());
    for (c, miller) in total
        .map(|c| (c, false))
        .into_iter()
        .chain(miller.map(|c| (c, true)))
    {
        let id = bench.id();
        let (supply, drain, gate) = (format!("{id}v"), format!("{id}d"), format!("{id}g"));
        let vds = c.condition(DRAIN_SOURCE_VOLTAGE).unwrap_or_default();
        let load = c.condition(DRAIN_CURRENT).unwrap_or_default();
        bench
            .tran
            .push(Bench::voltage(&id, &supply, sign * vds, false));
        bench.tran.push(format!(
            "I{id}l {supply} {drain} DC {}",
            number(sign * load)
        ));
        let (anode, cathode) = if sign > 0.0 {
            (&drain, &supply)
        } else {
            (&supply, &drain)
        };
        bench.tran.push(format!("D{id}c {anode} {cathode} {id}dm"));
        bench.tran.push(format!(".model {id}dm D"));
        bench.tran.push(format!(
            "I{id}g 0 {gate} PULSE(0 {} {CHARGE_DELAY} {CHARGE_DELAY} {CHARGE_DELAY} 1 2)",
            number(sign * gate_current)
        ));
        // Beside a clamp diode, whose leakage is gmin, a 1 Tohm shunt would
        // halve the gate's DC level: the shunt must be far below it.
        bench.tran.push(format!("R{id}g {gate} 0 1.000000e6"));
        // The gate current goes on after the charge is read. Clamping the gate
        // above the final gate voltage keeps it from charging to hundreds of
        // volts, where the body diode's solve fails.
        let clamp = format!("{id}k");
        bench.tran.push(Bench::voltage(
            &format!("{id}k"),
            &clamp,
            sign * (clamp_volts + 2.0),
            false,
        ));
        let (anode, cathode) = if sign > 0.0 {
            (&gate, &clamp)
        } else {
            (&clamp, &gate)
        };
        bench.tran.push(format!("D{id}k {anode} {cathode} {id}dm"));
        bench.tran.push(model.instance(&id, &[&drain, &gate, "0"]));
        let what = if miller {
            Charge::Miller
        } else {
            Charge::Total {
                vgs: c.condition(GATE_SOURCE_VOLTAGE).unwrap_or_default(),
            }
        };
        model.checks.push(Check {
            characteristic: c.index,
            role: Role::CheckOnly,
            expected: c.value,
            probe: Probe::Charge {
                gate,
                drain,
                amps: gate_current,
                vds,
                sign,
                what,
            },
        });
    }
}

/// A capacitance value at a drain voltage.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cap {
    vds: f64,
    value: f64,
    hz: f64,
    index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Input,
    Output,
    ReverseTransfer,
}

#[derive(Debug, Default)]
struct Capacitances {
    parameters: Vec<(&'static str, f64)>,
    crss_fit: Vec<Cap>,
    crss_held: Option<Cap>,
    ciss: Vec<Cap>,
    coss: Vec<Cap>,
}

fn caps_of(characteristics: &[Characteristic], quantity: Quantity) -> Vec<Cap> {
    let mut caps: Vec<Cap> = of(characteristics, quantity)
        .into_iter()
        .map(|c| Cap {
            vds: c.condition(DRAIN_SOURCE_VOLTAGE).unwrap_or_default(),
            value: c.value,
            hz: c.condition(FREQUENCY).unwrap_or(CAPACITANCE_HZ),
            index: c.index,
        })
        .collect();
    caps.sort_by(|a, b| a.vds.total_cmp(&b.vds));
    caps
}

/// `CGDMAX`, `CGDMIN` and `A` from Crss, then `CGS` from Ciss and the body
/// diode's junction from Coss, each less the gate-drain capacitance they hold.
fn capacitance_side(model: &mut Model, characteristics: &[Characteristic]) -> Fitted<Capacitances> {
    let crss = caps_of(characteristics, Quantity::ReverseTransferCapacitance);
    let ciss = caps_of(characteristics, Quantity::InputCapacitance);
    let coss = caps_of(characteristics, Quantity::OutputCapacitance);
    let field = |quantity: Quantity| {
        of(characteristics, quantity)
            .first()
            .map_or("characteristics".to_string(), |c| c.field("value"))
    };
    if crss.is_empty() {
        if let Some(quantity) = [
            (Quantity::InputCapacitance, !ciss.is_empty()),
            (Quantity::OutputCapacitance, !coss.is_empty()),
        ]
        .iter()
        .find_map(|(quantity, given)| given.then_some(*quantity))
        {
            return Err(Invalid::new(
                field(quantity),
                "input and output capacitance need reverse_transfer_capacitance values too, \
                 because the gate-drain capacitance is part of both",
            ));
        }
        return Ok(Capacitances::default());
    }
    if crss.len() < 3 {
        return Err(Invalid::new(
            field(Quantity::ReverseTransferCapacitance),
            "give at least three reverse_transfer_capacitance values off the capacitance \
             curve, or none",
        ));
    }
    for quantity in [
        Quantity::ReverseTransferCapacitance,
        Quantity::InputCapacitance,
        Quantity::OutputCapacitance,
    ] {
        let caps = caps_of(characteristics, quantity);
        if caps.windows(2).any(|pair| pair[0].vds == pair[1].vds) {
            return Err(Invalid::new(
                field(quantity),
                "two values share one drain voltage",
            ));
        }
    }
    let sorted: Vec<((f64, f64), usize)> =
        crss.iter().map(|c| ((c.vds, c.value), c.index)).collect();
    let (fit_points, held) = hold_out(&sorted);
    let values: Vec<(f64, f64)> = fit_points.iter().map(|(point, _)| *point).collect();
    let gate_drain = fit_gate_drain(&values);
    model.fitted.push(format!(
        "cgdmax, cgdmin and a from {} reverse transfer capacitance values",
        values.len()
    ));
    let mut parameters = vec![
        ("CGDMAX", gate_drain.max),
        ("CGDMIN", gate_drain.min),
        ("A", gate_drain.a),
    ];
    if !ciss.is_empty() {
        let mut total = 0.0;
        for cap in &ciss {
            let gate_source = cap.value - gate_drain.at_vds(cap.vds);
            if gate_source <= 0.0 {
                return Err(Invalid::new(
                    format!("characteristics[{}].value", cap.index),
                    "the input capacitance is not above the gate-drain capacitance at the same \
                     drain voltage",
                ));
            }
            total += gate_source;
        }
        parameters.push(("CGS", total / ciss.len() as f64));
        model
            .fitted
            .push("cgs from the input capacitance less the gate-drain capacitance".into());
    }
    if !coss.is_empty() {
        let mut junction = Vec::new();
        for cap in &coss {
            let drain_source = cap.value - gate_drain.at_vds(cap.vds);
            if drain_source <= 0.0 {
                return Err(Invalid::new(
                    format!("characteristics[{}].value", cap.index),
                    "the output capacitance is not above the gate-drain capacitance at the same \
                     drain voltage",
                ));
            }
            junction.push((cap.vds, drain_source));
        }
        let (cjo, vj, m, assumed) =
            fit_capacitance(&junction, BODY_VJ, &field(Quantity::OutputCapacitance))?;
        model.assumed.extend(assumed);
        model
            .fitted
            .push("cjo, vj and m of the body diode from the output capacitance less the gate-drain capacitance".into());
        parameters.extend([("CJO", cjo), ("VJ", vj), ("M", m)]);
    }
    let cap_at = |(point, index): ((f64, f64), usize)| {
        let source = &crss.iter().find(|c| c.index == index).copied();
        Cap {
            vds: point.0,
            value: point.1,
            hz: source.map_or(CAPACITANCE_HZ, |c| c.hz),
            index,
        }
    };
    Ok(Capacitances {
        parameters,
        crss_fit: fit_points.into_iter().map(cap_at).collect(),
        crss_held: held.map(cap_at),
        ciss,
        coss,
    })
}

/// One capacitance, measured the way a datasheet's meter does: with the gate
/// at 0 V, the drain biased at `Vds`, and the pins the quantity names tied at
/// AC.
fn check_capacitance(
    model: &mut Model,
    bench: &mut Bench,
    which: Which,
    cap: &Cap,
    role: Role,
    sign: f64,
) {
    let (drain_id, gate_id) = (bench.id(), bench.id());
    let (drain, gate) = (format!("{drain_id}d"), format!("{gate_id}g"));
    let ac_drain = which != Which::Input;
    bench
        .ac
        .push(Bench::voltage(&drain_id, &drain, sign * cap.vds, ac_drain));
    bench
        .ac
        .push(Bench::voltage(&gate_id, &gate, 0.0, !ac_drain));
    bench
        .ac
        .push(model.instance(&drain_id, &[&drain, &gate, "0"]));
    let probe = match which {
        // The gate's source drives Ciss through RG, which a series capacitance
        // ignores.
        Which::Input => Probe::SeriesCapacitance {
            source: format!("V{gate_id}"),
            hz: cap.hz,
        },
        // The drain's source drives Cds + Cgd.
        Which::Output => Probe::Capacitance {
            source: format!("V{drain_id}"),
            hz: cap.hz,
        },
        // The gate, held at AC ground, carries only the Cgd current.
        Which::ReverseTransfer => Probe::Capacitance {
            source: format!("V{gate_id}"),
            hz: cap.hz,
        },
    };
    model.checks.push(Check {
        characteristic: cap.index,
        role,
        expected: cap.value,
        probe,
    });
}

/// The gate voltage at the threshold current, with the drain tied to the gate.
fn check_threshold(model: &mut Model, bench: &mut Bench, c: &Characteristic, sign: f64) {
    let id = bench.id();
    let node = format!("{id}g");
    let amps = c.condition(DRAIN_CURRENT).unwrap_or_default();
    bench.dc.push(Bench::current_into(&id, &node, sign * amps));
    bench.dc.push(model.instance(&id, &[&node, &node, "0"]));
    model.checks.push(Check {
        characteristic: c.index,
        role: Role::Fit,
        expected: c.value,
        probe: Probe::Voltage {
            node,
            scale: sign,
            divide_by: 1.0,
        },
    });
}

/// The drain voltage over the drain current, with the gate held on.
fn check_on_resistance(model: &mut Model, bench: &mut Bench, c: &Characteristic, sign: f64) {
    let (drain_id, gate_id) = (bench.id(), bench.id());
    let (drain, gate) = (format!("{drain_id}d"), format!("{gate_id}g"));
    let amps = c.condition(DRAIN_CURRENT).unwrap_or_default();
    let vgs = c.condition(GATE_SOURCE_VOLTAGE).unwrap_or_default();
    bench
        .dc
        .push(Bench::current_into(&drain_id, &drain, sign * amps));
    bench
        .dc
        .push(Bench::voltage(&gate_id, &gate, sign * vgs, false));
    bench
        .dc
        .push(model.instance(&drain_id, &[&drain, &gate, "0"]));
    model.checks.push(Check {
        characteristic: c.index,
        role: Role::Fit,
        expected: c.value,
        probe: Probe::Voltage {
            node: drain,
            scale: sign,
            divide_by: amps,
        },
    });
}

/// The drain current at a gate and drain voltage, read through the drain's
/// voltage source.
fn check_transfer(
    model: &mut Model,
    bench: &mut Bench,
    point: Transfer,
    index: usize,
    role: Role,
    sign: f64,
) {
    let (drain_id, gate_id) = (bench.id(), bench.id());
    let (drain, gate) = (format!("{drain_id}d"), format!("{gate_id}g"));
    bench
        .dc
        .push(Bench::voltage(&drain_id, &drain, sign * point.vds, false));
    bench
        .dc
        .push(Bench::voltage(&gate_id, &gate, sign * point.vgs, false));
    bench
        .dc
        .push(model.instance(&drain_id, &[&drain, &gate, "0"]));
    model.checks.push(Check {
        characteristic: index,
        role,
        expected: point.current,
        probe: Probe::Current {
            source: format!("V{drain_id}"),
            scale: -sign,
        },
    });
}

/// The body diode: a current into the source with the gate off, whose voltage
/// is then the diode's forward voltage.
fn check_body_diode(
    model: &mut Model,
    bench: &mut Bench,
    point: Point,
    index: usize,
    role: Role,
    sign: f64,
) {
    let id = bench.id();
    let node = format!("{id}s");
    bench
        .dc
        .push(Bench::current_into(&id, &node, sign * point.current));
    bench.dc.push(model.instance(&id, &["0", "0", &node]));
    model.checks.push(Check {
        characteristic: index,
        role,
        expected: point.voltage,
        probe: Probe::Voltage {
            node,
            scale: sign,
            divide_by: 1.0,
        },
    });
}

/// The gate's AC resistance with drain and source grounded.
fn check_gate_resistance(model: &mut Model, bench: &mut Bench, c: &Characteristic) {
    let id = bench.id();
    let node = format!("{id}g");
    bench.ac.push(Bench::voltage(&id, &node, 0.0, true));
    bench.ac.push(model.instance(&id, &["0", &node, "0"]));
    model.checks.push(Check {
        characteristic: c.index,
        role: Role::Fit,
        expected: c.value,
        probe: Probe::Resistance {
            source: format!("V{id}"),
            hz: c.condition(FREQUENCY).unwrap_or(GATE_RESISTANCE_HZ),
        },
    });
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::{build as build_model, text};
    use super::*;
    use serde_json::{json, Value};

    /// Values the engine returned for `VTO=3 KP=20 THETA=0.05 RD=0.015`.
    const ENGINE: [(f64, f64, f64); 4] = [
        (3.2, 3.2, 0.447903),
        (4.5, 10.0, 20.9302),
        (6.0, 10.0, 78.2609),
        (10.0, 0.1, 4.05352),
    ];

    fn law() -> Law {
        Law {
            vto: 3.0,
            kp: 20.0,
            theta: 0.05,
            rd: 0.015,
        }
    }

    #[test]
    fn the_law_reads_what_ngspice_read() {
        for (vgs, vds, engine) in ENGINE {
            let ours = law().current(vgs, vds);
            assert!(
                (ours - engine).abs() <= 1e-5 * engine,
                "({vgs}, {vds}): {ours} against {engine}"
            );
        }
    }

    #[test]
    fn the_drop_inverts_the_current() {
        let law = law();
        let current = law.current(10.0, 0.1);
        let drop = law.drop(10.0, current).unwrap();
        assert!((drop - 0.1).abs() < 1e-6, "{drop}");
        // A current the channel cannot carry at a low gate voltage.
        assert!(law.drop(3.5, 1000.0).is_none());
    }

    fn transfer(vgs: f64, vds: f64) -> Value {
        json!({
            "quantity": "drain_current", "value": law().current(vgs, vds),
            "conditions": { "gate_source_voltage_volts": vgs, "drain_source_voltage_volts": vds },
            "statistic": "typical"
        })
    }

    fn datasheet(extra: Vec<Value>) -> Vec<Characteristic> {
        // The gate voltage at 250 uA with the drain tied to the gate.
        let l = law();
        let threshold = root(|v| l.current(v, v) - 250e-6, 0.5, 6.0).unwrap();
        let mut values = vec![
            json!({
                "quantity": "gate_threshold_voltage", "value": threshold,
                "conditions": { "drain_current_amps": 250e-6 }, "statistic": "typical"
            }),
            json!({
                "quantity": "on_resistance", "value": law().drop(10.0, 20.0).unwrap() / 20.0,
                "conditions": { "gate_source_voltage_volts": 10.0, "drain_current_amps": 20.0 },
                "statistic": "typical"
            }),
            transfer(4.0, 10.0),
            transfer(6.0, 10.0),
        ];
        values.extend(extra);
        parsed(Value::Array(values))
    }

    #[test]
    fn the_fit_recovers_the_law_that_made_the_values() {
        let model = build_model(Kind::NMosfet, "irf_fit", &datasheet(vec![]), 25.0).unwrap();
        let kp = fitted(&model, "vdmos.KP");
        let vto = fitted(&model, "vdmos.VTO");
        let theta = fitted(&model, "vdmos.THETA");
        let rd = fitted(&model, "vdmos.RD");
        assert!((vto - 3.0).abs() < 0.02, "{vto}");
        assert!((kp - 20.0).abs() < 0.5, "{kp}");
        assert!((theta - 0.05).abs() < 0.01, "{theta}");
        assert!((rd - 0.015).abs() < 0.0015, "{rd}");
    }

    #[test]
    fn a_curve_longer_than_the_law_holds_its_middle_value_out() {
        let extra = vec![transfer(5.0, 10.0), transfer(8.0, 10.0)];
        let model = build_model(Kind::NMosfet, "irf_fit", &datasheet(extra), 25.0).unwrap();
        assert_eq!(
            model
                .checks
                .iter()
                .filter(|c| c.role == Role::HeldOut)
                .count(),
            1
        );
    }

    #[test]
    fn a_p_channel_part_is_the_same_fit_with_a_negative_vto_and_mirrored_sources() {
        let n = build_model(Kind::NMosfet, "irf_fit", &datasheet(vec![]), 25.0).unwrap();
        let p = build_model(Kind::PMosfet, "irf_p_fit", &datasheet(vec![]), 25.0).unwrap();
        assert!(p.body.contains("+ PCHAN\n") && n.body.contains("+ NCHAN\n"));
        assert!((fitted(&p, "vdmos.VTO") + fitted(&n, "vdmos.VTO")).abs() < 1e-9);
        assert!((fitted(&p, "vdmos.KP") - fitted(&n, "vdmos.KP")).abs() < 1e-9);
        // The P part's bench sources point the other way.
        assert!(p.bench.dc.iter().any(|line| line.contains("DC -")));
        assert!(!n.bench.dc.iter().any(|line| line.contains("DC -")));
    }

    #[test]
    fn the_file_is_the_three_pin_subcircuit_in_the_fitted_grammar() {
        let model = build_model(Kind::NMosfet, "irf_fit", &datasheet(vec![]), 25.0).unwrap();
        assert!(model
            .body
            .starts_with(".subckt irf_fit D G S\nM1 D G S irf_fit_m\n"));
        let checked = text::check_fitted(model.file().as_bytes()).unwrap();
        assert_eq!(checked.pins(), ["D", "G", "S"]);
        assert_eq!(checked.kind, "n_mosfet");
    }

    #[test]
    fn an_on_resistance_below_the_channel_alone_is_clamped_and_said() {
        let mut values = datasheet(vec![]);
        let on = values
            .iter_mut()
            .find(|c| c.quantity == Quantity::OnResistance)
            .unwrap();
        on.value *= 0.1;
        let model = build_model(Kind::NMosfet, "irf_fit", &values, 25.0).unwrap();
        assert!(model.assumed.iter().any(|note| note.contains("rd is zero")));
        assert!(!model.parameters.iter().any(|(key, _)| key == "vdmos.RD"));
    }

    #[test]
    fn a_current_the_channel_cannot_carry_is_refused_on_the_on_resistance() {
        let mut values = datasheet(vec![]);
        let on = values
            .iter_mut()
            .find(|c| c.quantity == Quantity::OnResistance)
            .unwrap();
        for (key, value) in on.conditions.iter_mut() {
            if *key == DRAIN_CURRENT {
                *value = 1e6;
            }
        }
        let error = build_model(Kind::NMosfet, "irf_fit", &values, 25.0).unwrap_err();
        assert!(
            error.field.ends_with("conditions.drain_current_amps"),
            "{error:?}"
        );
    }

    #[test]
    fn two_values_at_one_gate_and_drain_voltage_are_refused() {
        let error = build_model(
            Kind::NMosfet,
            "irf_fit",
            &datasheet(vec![transfer(6.0, 10.0)]),
            25.0,
        )
        .unwrap_err();
        assert_eq!(error.field, "characteristics");
    }

    fn capacitance(quantity: &str, vds: f64, farads: f64) -> Value {
        json!({
            "quantity": quantity, "value": farads,
            "conditions": { "drain_source_voltage_volts": vds }, "statistic": "typical"
        })
    }

    fn crss_curve() -> Vec<Value> {
        [
            (1.0, 1.6861e-9),
            (5.0, 1.1205e-9),
            (10.0, 7.3245e-10),
            (25.0, 3.5803e-10),
            (50.0, 2.1159e-10),
        ]
        .into_iter()
        .map(|(v, c)| capacitance("reverse_transfer_capacitance", v, c))
        .collect()
    }

    #[test]
    fn crss_values_recover_the_gate_drain_law_that_made_them() {
        let truth = GateDrain {
            max: 3e-9,
            min: 6e-11,
            a: 0.15,
        };
        let points: Vec<(f64, f64)> = [1.0, 5.0, 10.0, 25.0, 50.0]
            .into_iter()
            .map(|v| (v, truth.at_vds(v)))
            .collect();
        let found = fit_gate_drain(&points);
        for (v, c) in &points {
            assert!(
                (found.at_vds(*v) - c).abs() <= 0.01 * c,
                "{v}: {} against {c}",
                found.at_vds(*v)
            );
        }
    }

    #[test]
    fn capacitances_add_their_cards_and_checks() {
        let mut extra = crss_curve();
        extra.push(capacitance("input_capacitance", 10.0, 2.7324e-9));
        extra.push(capacitance("output_capacitance", 10.0, 1.059e-9));
        let model = build_model(Kind::NMosfet, "irf_fit", &datasheet(extra), 25.0).unwrap();
        for key in ["CGDMAX", "CGDMIN", "A", "CGS", "CJO", "VJ", "M"] {
            fitted(&model, &format!("vdmos.{key}"));
        }
        assert!((fitted(&model, "vdmos.CGS") - 2.0e-9).abs() < 0.1e-9);
        assert!(text::check_fitted(model.file().as_bytes()).is_ok());
        // Crss holds its middle value out; Ciss and Coss are each one check.
        assert_eq!(
            model
                .checks
                .iter()
                .filter(|c| c.role == Role::HeldOut)
                .count(),
            1
        );
        assert!(!model
            .assumed
            .iter()
            .any(|note| note.contains("no capacitance values")));
    }

    #[test]
    fn gate_charges_are_checked_not_fitted_on_a_clamped_inductive_turn_on() {
        let mut extra = crss_curve();
        extra.push(json!({
            "quantity": "gate_charge", "value": 6.6e-8,
            "conditions": { "drain_source_voltage_volts": 25.0, "drain_current_amps": 20.0, "gate_source_voltage_volts": 10.0 },
            "statistic": "typical"
        }));
        extra.push(json!({
            "quantity": "gate_drain_charge", "value": 2.1e-8,
            "conditions": { "drain_source_voltage_volts": 25.0, "drain_current_amps": 20.0 },
            "statistic": "typical"
        }));
        let model = build_model(Kind::NMosfet, "irf_fit", &datasheet(extra), 25.0).unwrap();
        let charges: Vec<_> = model
            .checks
            .iter()
            .filter(|c| c.role == Role::CheckOnly)
            .collect();
        assert_eq!(charges.len(), 2);
        assert!(model.bench.tran_stop > 0.0);
        assert!(model.bench.tran[0].starts_with(".options reltol=1e-3"));
        // The load pushes into the drain and a diode returns it to the supply.
        assert!(model
            .bench
            .tran
            .iter()
            .any(|l| l.starts_with("Ib") && l.contains("l b")));
        assert!(model.bench.tran.iter().any(|l| l.contains("PULSE(0 2.64")));
        // A part with no charges has no transient run.
        let none = build_model(Kind::NMosfet, "irf_fit", &datasheet(vec![]), 25.0).unwrap();
        assert!(none.bench.tran.is_empty() && none.bench.tran_stop == 0.0);
    }

    #[test]
    fn without_capacitances_the_result_says_the_defaults_stand() {
        let model = build_model(Kind::NMosfet, "irf_fit", &datasheet(vec![]), 25.0).unwrap();
        assert!(model
            .assumed
            .iter()
            .any(|note| note.contains("no capacitance values")));
    }

    #[test]
    fn crss_needs_a_curve_and_ciss_and_coss_need_crss() {
        let two = datasheet(crss_curve().into_iter().take(2).collect());
        let error = build_model(Kind::NMosfet, "irf_fit", &two, 25.0).unwrap_err();
        assert!(error.reason.contains("at least three"), "{error:?}");
        let alone = datasheet(vec![capacitance("input_capacitance", 10.0, 2.7e-9)]);
        let error = build_model(Kind::NMosfet, "irf_fit", &alone, 25.0).unwrap_err();
        assert!(
            error.reason.contains("need reverse_transfer_capacitance"),
            "{error:?}"
        );
        // An input capacitance below the gate-drain capacitance at its voltage.
        let mut low = crss_curve();
        low.push(capacitance("input_capacitance", 10.0, 1e-10));
        let error = build_model(Kind::NMosfet, "irf_fit", &datasheet(low), 25.0).unwrap_err();
        assert!(
            error.reason.contains("not above the gate-drain"),
            "{error:?}"
        );
    }
}
