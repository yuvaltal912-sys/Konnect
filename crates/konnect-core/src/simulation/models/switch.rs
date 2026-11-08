//! Load switches, eFuses and ideal diodes, as one behavioural voltage drop.
//!
//! Each part is a 0 V sense source from `IN`, then a `B` voltage source whose
//! value is the drop across the part as a function of the sensed current,
//! made a voltage by an H source: `IN -> VS -> BO -> OUT`. The current flows
//! through the part itself, so the input carries exactly what the output
//! draws. A current-mode law, a current as a function of the drop, was tried
//! first and refused to solve: a current limit or a regulation corner is flat
//! on one side, and on libngspice 45.2 Newton then cycled from one flat to the
//! other until source stepping rescued it, or did not. The drop as a function
//! of the current solved every fixture on its first pass.
//!
//! - eFuse: `Ron I / (en + Ron / Roff)` plus a stiff term past the current
//!   limit, both ways. `en` is a smooth step up at the undervoltage lockout
//!   and down at the overvoltage cut-off, so the part opens to `Roff` outside
//!   its window.
//! - Load switch: the larger of `Ron I` and `(1 - r) Vin`, where `r` is the
//!   enable, a smooth step at the threshold, through an RC whose time
//!   constant gives the datasheet's rise time. The output follows `r Vin`
//!   while it rises, whatever the load, as a slew-controlled switch does.
//!   Reverse current is blocked, at 1 uA a volt.
//! - Ideal diode: the larger of `Vreg + Rreg I` (the regulated forward drop)
//!   and `Ron I` (fully on), and a blocking resistance for reverse current
//!   fitted to the datasheet's reverse current.

use super::bench::{Bench, Check, Probe, Role, Trip};
use super::fit;
use super::regulator::{
    expr_smooth_max_zero, smooth_max_zero, term, COMMON_LIFT, LIMIT_CORNER, LIMIT_STIFFNESS,
    QUIESCENT_KNEE, SINK_CORNER,
};
use super::text::number;
use super::{
    of, Characteristic, Element, Fitted, Invalid, Kind, Model, Quantity, FORWARD_CURRENT,
    INPUT_VOLTAGE, LOAD_CAPACITANCE, LOAD_RESISTANCE, OUTPUT_CURRENT, OUTPUT_VOLTAGE,
    REVERSE_VOLTAGE,
};

/// An eFuse's and an ideal diode's pins. `COM` is the ground pin, never
/// `GND`, which ngspice reads as ground.
pub(crate) const PINS: [&str; 3] = ["IN", "OUT", "COM"];
pub(crate) const LOAD_SWITCH_PINS: [&str; 4] = ["IN", "OUT", "EN", "COM"];
const CURRENT: &str = "v(hs,COM)";
/// What an open switch leaves across it.
const OFF_RESISTANCE: f64 = 1e9;
/// A threshold's step is this fraction of the threshold wide.
const STEP_WIDTH: f64 = 1e-4;
/// The width of a drop's corners, in volts.
const DROP_CORNER: f64 = 1e-4;
/// The resistance a blocked reverse current meets: 1 uA a volt.
const BLOCKING_RESISTANCE: f64 = 1e6;
/// An ideal diode's reverse resistance without a reverse current value.
const DEFAULT_DIODE_BLOCKING: f64 = 1e7;
/// A load switch's enable threshold without a value: a logic input.
const DEFAULT_ENABLE: f64 = 1.0;
/// A load switch's enable time constant without a rise time.
const DEFAULT_RISE_SECONDS: f64 = 1e-6;
/// The input an eFuse or load switch value is read at when it states none.
const DEFAULT_INPUT: f64 = 5.0;
/// Options for the transient benches.
const TRANSIENT_TOLERANCES: &str = ".options reltol=1e-4 vntol=1e-7 abstol=1e-13";
/// 10 to 90 percent of an exponential is this many time constants.
const RISE_FRACTIONS: (f64, f64) = (0.1, 0.9);

fn step_up(volts: f64, threshold: f64) -> f64 {
    0.5 + 0.5 * ((volts - threshold) / (STEP_WIDTH * threshold)).tanh()
}

fn step_down(volts: f64, threshold: f64) -> f64 {
    0.5 - 0.5 * ((volts - threshold) / (STEP_WIDTH * threshold)).tanh()
}

/// The larger of two, with a rounded corner.
fn smooth_max(a: f64, b: f64, width: f64) -> f64 {
    (a + b + ((a - b) * (a - b) + width * width).sqrt()) / 2.0
}

fn expr_smooth_max(a: &str, b: &str, width: f64) -> String {
    format!(
        "(({a})+({b})+sqrt((({a})-({b}))*(({a})-({b}))+{}))/2.000000e0",
        number(width * width)
    )
}

/// The fitted laws.
#[derive(Debug, Clone, Copy)]
enum Law {
    Efuse {
        ron: f64,
        limit: f64,
        stiffness: f64,
        lockout: Option<f64>,
        cutoff: Option<f64>,
    },
    LoadSwitch {
        ron: f64,
        limit: Option<(f64, f64)>,
    },
    IdealDiode {
        vreg: f64,
        rreg: f64,
        ron: f64,
        blocking: f64,
    },
}

impl Law {
    /// The drop across the part at a current, an input voltage and (for a
    /// load switch) an enable state `r`, as the engine evaluates it.
    fn drop(&self, current: f64, vin: f64, r: f64) -> f64 {
        match *self {
            Law::Efuse {
                ron,
                limit,
                stiffness,
                lockout,
                cutoff,
            } => {
                let en = lockout.map_or(1.0, |t| step_up(vin, t))
                    * cutoff.map_or(1.0, |t| step_down(vin, t));
                let width = LIMIT_CORNER * limit;
                ron * current / (en + ron / OFF_RESISTANCE)
                    + stiffness * smooth_max_zero(current - limit, width)
                    - stiffness * smooth_max_zero(-current - limit, width)
            }
            Law::LoadSwitch { ron, limit } => {
                let mut on = ron * current;
                if let Some((limit, stiffness)) = limit {
                    on += stiffness * smooth_max_zero(current - limit, LIMIT_CORNER * limit);
                }
                smooth_max(on, (1.0 - r) * vin, DROP_CORNER)
                    - BLOCKING_RESISTANCE * smooth_max_zero(-current, SINK_CORNER)
            }
            Law::IdealDiode {
                vreg,
                rreg,
                ron,
                blocking,
            } => {
                smooth_max(vreg + rreg * current, ron * current, DROP_CORNER)
                    - blocking * smooth_max_zero(-current, SINK_CORNER)
            }
        }
    }

    /// The current through the part with `drop` across it: the drop rises
    /// with the current, so one root.
    fn current_at(&self, drop: f64, vin: f64, r: f64) -> Option<f64> {
        let reach = 1e6 * (drop.abs() + 1.0);
        fit::root(|current| self.drop(current, vin, r) - drop, -reach, reach)
    }

    /// The load resistance that draws `current` from an input at `vin`.
    fn load_for(&self, current: f64, vin: f64) -> f64 {
        (vin - self.drop(current, vin, 1.0)) / current
    }
}

pub(crate) fn build(kind: Kind, name: &str, characteristics: &[Characteristic]) -> Fitted<Model> {
    let pins = if kind == Kind::LoadSwitch {
        LOAD_SWITCH_PINS.to_vec()
    } else {
        PINS.to_vec()
    };
    let mut model = Model::new(kind, name, Element::Subcircuit, pins);
    let single = |quantity| of(characteristics, quantity).first().copied();
    let quiescent = single(Quantity::QuiescentCurrent);
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
        "no temperature behaviour, no thermal shutdown and no fault latch or retry".to_string(),
    );
    let mut bench = Bench::default();
    bench.tran.push(TRANSIENT_TOLERANCES.to_string());
    bench.slow.push(TRANSIENT_TOLERANCES.to_string());
    let extra;
    let law = match kind {
        Kind::Efuse => {
            let (law, window) = efuse(&mut model, characteristics)?;
            extra = String::new();
            check_efuse(&mut model, &mut bench, &law, window, characteristics);
            law
        }
        Kind::LoadSwitch => {
            let (law, threshold, tau) = load_switch(&mut model, characteristics)?;
            extra = format!(
                "BE e COM V=5.000000e-1+5.000000e-1*tanh((v(EN,COM)-{})/{})\n\
                 RE e r 1.000000e0\nCE r COM {}\n",
                number(threshold),
                number(STEP_WIDTH * threshold),
                number(tau)
            );
            model.parameters.push(("enable_threshold".to_string(), threshold));
            model.parameters.push(("enable_time_constant".to_string(), tau));
            check_load_switch(&mut model, &mut bench, &law, threshold, tau, characteristics);
            law
        }
        _ => {
            let (law, fit_points, held) = ideal_diode(&mut model, characteristics)?;
            extra = String::new();
            check_ideal_diode(&mut model, &mut bench, &law, &fit_points, held, characteristics);
            law
        }
    };
    if let (Some(c), Some(_)) = (quiescent, iq) {
        let id = bench.id();
        let (input, output) = (format!("{id}i"), format!("{id}o"));
        let vin = c.condition(INPUT_VOLTAGE).unwrap_or_default();
        bench.dc.push(Bench::voltage(&id, &input, vin, false));
        let enable = format!("{id}e");
        if kind == Kind::LoadSwitch {
            let drive = enable_drive(&model).max(vin);
            bench.dc.push(Bench::voltage(&enable, &enable, drive, false));
            bench
                .dc
                .push(model.instance(&id, &[&input, &output, &enable, "0"]));
        } else {
            bench.dc.push(model.instance(&id, &[&input, &output, "0"]));
        }
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
    if let Some(iq) = iq {
        model.parameters.push(("quiescent_current".to_string(), iq));
    }
    model.body = body(name, kind, &law, iq, &extra);
    model.bench = bench;
    Ok(model)
}

/// The input an eFuse value is read at, inside its window.
fn window_input(lockout: Option<f64>, cutoff: Option<f64>) -> f64 {
    match (lockout, cutoff) {
        (Some(low), Some(high)) => (low + high) / 2.0,
        (Some(low), None) => 2.0 * low,
        (None, Some(high)) => 0.5 * high,
        (None, None) => DEFAULT_INPUT,
    }
}

fn efuse(
    model: &mut Model,
    characteristics: &[Characteristic],
) -> Fitted<(Law, (Option<f64>, Option<f64>))> {
    let single = |quantity| of(characteristics, quantity).first().copied();
    let on = single(Quantity::SwitchOnResistance).ok_or_else(|| {
        Invalid::new("characteristics", "an efuse needs its switch_on_resistance")
    })?;
    let limit = single(Quantity::CurrentLimit)
        .ok_or_else(|| Invalid::new("characteristics", "an efuse needs its current_limit"))?;
    let lockout = single(Quantity::UndervoltageLockout);
    let cutoff = single(Quantity::OvervoltageCutoff);
    if let (Some(low), Some(high)) = (lockout, cutoff) {
        if high.value <= low.value * 1.01 {
            return Err(Invalid::new(
                high.field("value"),
                "the overvoltage cut-off must be above the undervoltage lockout",
            ));
        }
    }
    let inside = |c: &Characteristic, vin: f64| -> Fitted<()> {
        let low = lockout.map_or(0.0, |l| l.value * (1.0 + 100.0 * STEP_WIDTH));
        let high = cutoff.map_or(f64::INFINITY, |h| h.value * (1.0 - 100.0 * STEP_WIDTH));
        if vin <= low || vin >= high {
            return Err(Invalid::new(
                c.field(&format!("conditions.{INPUT_VOLTAGE}")),
                "this input is outside the part's undervoltage and overvoltage window, where it \
                 is open",
            ));
        }
        Ok(())
    };
    let nominal = window_input(lockout.map(|c| c.value), cutoff.map(|c| c.value));
    let on_vin = on.condition(INPUT_VOLTAGE).unwrap_or(nominal);
    inside(on, on_vin)?;
    let ron = on.value;
    let on_current = on.condition(OUTPUT_CURRENT).unwrap_or_default();
    if on_current >= 0.9 * limit.value {
        return Err(Invalid::new(
            on.field(&format!("conditions.{OUTPUT_CURRENT}")),
            "the on-resistance must be read below the current limit",
        ));
    }
    let limit_vin = limit.condition(INPUT_VOLTAGE).unwrap_or(nominal);
    inside(limit, limit_vin)?;
    let held = limit.condition(OUTPUT_VOLTAGE).unwrap_or(0.0);
    if (limit_vin - held) / ron <= 1.01 * limit.value {
        return Err(Invalid::new(
            limit.field("value"),
            "the on-resistance passes less than this current with this much across it, so the \
             limit could never be reached",
        ));
    }
    let stiffness = LIMIT_STIFFNESS * limit_vin / limit.value;
    let mut law = Law::Efuse {
        ron,
        limit: limit.value,
        stiffness,
        lockout: lockout.map(|c| c.value),
        cutoff: cutoff.map(|c| c.value),
    };
    // The limit setting is solved so the engine's current at the held output
    // is the datasheet's, the stiffness and the corner included.
    let target = limit.value;
    let reading = |setting: f64| {
        let trial = with_limit(law, setting);
        trial
            .current_at(limit_vin - held, limit_vin, 1.0)
            .unwrap_or(f64::NAN)
            - target
    };
    let setting = fit::root(reading, 0.5 * target, 1.5 * target).ok_or_else(|| {
        Invalid::new(limit.field("value"), "no current limit reproduces this value")
    })?;
    law = with_limit(law, setting);
    // Each threshold is placed so the engine's output crosses half its input
    // there: the step's own shape moves the crossing a little.
    let lockout = lockout.map(|c| solve_threshold(&law, c.value, true));
    let cutoff = cutoff.map(|c| solve_threshold(&law, c.value, false));
    if let Law::Efuse {
        lockout: l,
        cutoff: h,
        ..
    } = &mut law
    {
        *l = lockout;
        *h = cutoff;
    }
    model.fitted.push(
        "the on-resistance, the current limit (flat, both ways) and the window it conducts in"
            .to_string(),
    );
    if lockout.is_none() {
        model
            .assumed
            .push("no undervoltage lockout: it conducts from 0 V".to_string());
    }
    if cutoff.is_none() {
        model
            .assumed
            .push("no overvoltage cut-off".to_string());
    }
    model.assumed.push(
        "no hysteresis on its thresholds, no foldback and no start-up delay or slew control; a \
         reverse current is limited like a forward one, not blocked"
            .to_string(),
    );
    model.parameters = vec![
        ("on_resistance".to_string(), ron),
        ("current_limit_setting".to_string(), setting),
        ("limit_stiffness".to_string(), stiffness),
    ];
    if let Some(t) = lockout {
        model.parameters.push(("undervoltage_lockout_setting".to_string(), t));
    }
    if let Some(t) = cutoff {
        model.parameters.push(("overvoltage_cutoff_setting".to_string(), t));
    }
    Ok((law, (lockout, cutoff)))
}

fn with_limit(law: Law, setting: f64) -> Law {
    match law {
        Law::Efuse {
            ron,
            stiffness,
            lockout,
            cutoff,
            ..
        } => Law::Efuse {
            ron,
            limit: setting,
            stiffness,
            lockout,
            cutoff,
        },
        Law::LoadSwitch { ron, limit } => Law::LoadSwitch {
            ron,
            limit: limit.map(|(_, stiffness)| (setting, stiffness)),
        },
        other => other,
    }
}

/// The load a threshold fixture draws through: a tenth of the limit at the
/// threshold.
fn threshold_load(law: &Law, threshold: f64) -> f64 {
    match law {
        Law::Efuse { limit, .. } => 10.0 * threshold / limit,
        _ => 1e3,
    }
}

/// The output of an eFuse at input `vin` into `load`, by the law.
fn efuse_output(law: &Law, vin: f64, load: f64) -> f64 {
    fit::root(
        |out| law.drop(out / load, vin, 1.0) - (vin - out),
        0.0,
        vin,
    )
    .unwrap_or(0.0)
}

/// The setting at which an eFuse's output crosses half its datasheet
/// threshold, in the fixture's load, at the datasheet threshold.
fn solve_threshold(law: &Law, threshold: f64, lockout: bool) -> f64 {
    let load = threshold_load(law, threshold);
    let level = 0.5 * threshold;
    let crossing = |setting: f64| {
        let trial = match *law {
            Law::Efuse {
                ron,
                limit,
                stiffness,
                lockout: l,
                cutoff: c,
            } => Law::Efuse {
                ron,
                limit,
                stiffness,
                lockout: if lockout { Some(setting) } else { l },
                cutoff: if lockout { c } else { Some(setting) },
            },
            other => other,
        };
        let span = 1e3 * STEP_WIDTH * threshold;
        fit::root(
            |vin| efuse_output(&trial, vin, load) - level,
            threshold - span,
            threshold + span,
        )
        .unwrap_or(threshold)
    };
    // The crossing moves one for one with the setting.
    threshold - (crossing(threshold) - threshold)
}

fn check_efuse(
    model: &mut Model,
    bench: &mut Bench,
    law: &Law,
    (lockout, cutoff): (Option<f64>, Option<f64>),
    characteristics: &[Characteristic],
) {
    let single = |quantity| of(characteristics, quantity).first().copied();
    let nominal = window_input(
        single(Quantity::UndervoltageLockout).map(|c| c.value),
        single(Quantity::OvervoltageCutoff).map(|c| c.value),
    );
    if let Some(c) = single(Quantity::SwitchOnResistance) {
        let vin = c.condition(INPUT_VOLTAGE).unwrap_or(nominal);
        check_on_resistance(model, bench, law, c, vin, false);
    }
    if let Some(c) = single(Quantity::CurrentLimit) {
        let vin = c.condition(INPUT_VOLTAGE).unwrap_or(nominal);
        check_limit(model, bench, c, vin, false);
    }
    bench.slow_stop = 1e-3;
    for (c, setting, lockout) in [
        (single(Quantity::UndervoltageLockout), lockout, true),
        (single(Quantity::OvervoltageCutoff), cutoff, false),
    ] {
        let (Some(c), Some(_)) = (c, setting) else {
            continue;
        };
        let id = bench.id();
        let (input, output) = (format!("{id}i"), format!("{id}o"));
        let span = 0.05 * c.value;
        bench.slow.push(format!(
            "V{id} {input} 0 PWL(0.000000e0 {} {} {})",
            number(c.value - span),
            number(bench.slow_stop),
            number(c.value + span)
        ));
        bench.slow.push(model.instance(&id, &[&input, &output, "0"]));
        bench.slow.push(format!(
            "R{id}l {output} 0 {}",
            number(threshold_load(law, c.value))
        ));
        model.checks.push(Check {
            characteristic: c.index,
            role: Role::Fit,
            expected: c.value,
            probe: Probe::Trip {
                input,
                output,
                level: 0.5 * c.value,
                what: if lockout { Trip::Rising } else { Trip::Falling },
            },
        });
    }
}

/// The on-resistance, with the stated current drawn by a resistor and `COM`
/// lifted off ground so a pin the engine tied to ground would read wrong.
fn check_on_resistance(
    model: &mut Model,
    bench: &mut Bench,
    law: &Law,
    c: &Characteristic,
    vin: f64,
    enable: bool,
) {
    let id = bench.id();
    let (input, output, common) = (format!("{id}i"), format!("{id}o"), format!("{id}c"));
    let current = c.condition(OUTPUT_CURRENT).unwrap_or_default();
    let load = law.load_for(current, vin);
    bench
        .dc
        .push(Bench::voltage(&id, &input, vin + COMMON_LIFT, false));
    bench
        .dc
        .push(Bench::voltage(&common, &common, COMMON_LIFT, false));
    if enable {
        let pin = format!("{id}e");
        bench.dc.push(Bench::voltage(
            &pin,
            &pin,
            COMMON_LIFT + enable_drive(model),
            false,
        ));
        bench
            .dc
            .push(model.instance(&id, &[&input, &output, &pin, &common]));
    } else {
        bench
            .dc
            .push(model.instance(&id, &[&input, &output, &common]));
    }
    bench
        .dc
        .push(format!("R{id}l {output} {common} {}", number(load)));
    model.checks.push(Check {
        characteristic: c.index,
        role: Role::Fit,
        expected: c.value,
        probe: Probe::Drop {
            high: input,
            low: output,
            load: Some((common, load)),
        },
    });
}

/// The enable pin held well past its threshold.
fn enable_drive(model: &Model) -> f64 {
    let threshold = model
        .parameters
        .iter()
        .find(|(key, _)| key == "enable_threshold")
        .map_or(DEFAULT_ENABLE, |(_, value)| *value);
    2.0 * threshold
}

/// The current into an output held at a voltage.
fn check_limit(model: &mut Model, bench: &mut Bench, c: &Characteristic, vin: f64, enable: bool) {
    let id = bench.id();
    let (input, output) = (format!("{id}i"), format!("{id}o"));
    let held = c.condition(OUTPUT_VOLTAGE).unwrap_or(0.0);
    bench.dc.push(Bench::voltage(&id, &input, vin, false));
    if enable {
        let pin = format!("{id}e");
        bench
            .dc
            .push(Bench::voltage(&pin, &pin, enable_drive(model), false));
        bench
            .dc
            .push(model.instance(&id, &[&input, &output, &pin, "0"]));
    } else {
        bench.dc.push(model.instance(&id, &[&input, &output, "0"]));
    }
    bench
        .dc
        .push(Bench::voltage(&format!("{id}f"), &output, held, false));
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

fn load_switch(
    model: &mut Model,
    characteristics: &[Characteristic],
) -> Fitted<(Law, f64, f64)> {
    let single = |quantity| of(characteristics, quantity).first().copied();
    let on = single(Quantity::SwitchOnResistance).ok_or_else(|| {
        Invalid::new("characteristics", "a load_switch needs its switch_on_resistance")
    })?;
    let ron = on.value;
    let threshold = single(Quantity::EnableThreshold).map_or_else(
        || {
            model.assumed.push(format!(
                "no enable threshold: the enable turns the switch on at {} V",
                number(DEFAULT_ENABLE)
            ));
            DEFAULT_ENABLE
        },
        |c| c.value,
    );
    let mut law = Law::LoadSwitch { ron, limit: None };
    if let Some(limit) = single(Quantity::CurrentLimit) {
        let vin = limit.condition(INPUT_VOLTAGE).unwrap_or(DEFAULT_INPUT);
        let held = limit.condition(OUTPUT_VOLTAGE).unwrap_or(0.0);
        if (vin - held) / ron <= 1.01 * limit.value {
            return Err(Invalid::new(
                limit.field("value"),
                "the on-resistance passes less than this current with this much across it, so \
                 the limit could never be reached",
            ));
        }
        let stiffness = LIMIT_STIFFNESS * vin / limit.value;
        law = Law::LoadSwitch {
            ron,
            limit: Some((limit.value, stiffness)),
        };
        let target = limit.value;
        let reading = |setting: f64| {
            with_limit(law, setting)
                .current_at(vin - held, vin, 1.0)
                .unwrap_or(f64::NAN)
                - target
        };
        let setting = fit::root(reading, 0.5 * target, 1.5 * target).ok_or_else(|| {
            Invalid::new(limit.field("value"), "no current limit reproduces this value")
        })?;
        law = with_limit(law, setting);
        model
            .parameters
            .push(("current_limit_setting".to_string(), setting));
    } else {
        model.assumed.push("no current limit".to_string());
    }
    if on.condition(OUTPUT_CURRENT).unwrap_or_default()
        >= 0.9 * single(Quantity::CurrentLimit).map_or(f64::INFINITY, |c| c.value)
    {
        return Err(Invalid::new(
            on.field(&format!("conditions.{OUTPUT_CURRENT}")),
            "the on-resistance must be read below the current limit",
        ));
    }
    // The output follows r Vin as r rises as 1 - exp(-t / tau), so its 10 to
    // 90 percent rise of its final value Vf is tau ln((1 - 0.1 k) / (1 - 0.9 k)),
    // k = Vf / Vin.
    let tau = match single(Quantity::OutputRiseTime) {
        Some(c) => {
            let vin = c.condition(INPUT_VOLTAGE).unwrap_or_default();
            let k = final_output(&law, vin, c.condition(LOAD_RESISTANCE)) / vin;
            let (low, high) = RISE_FRACTIONS;
            c.value / ((1.0 - low * k) / (1.0 - high * k)).ln()
        }
        None => {
            model.assumed.push(format!(
                "no rise time: the enable acts through a time constant of {} s",
                number(DEFAULT_RISE_SECONDS)
            ));
            DEFAULT_RISE_SECONDS
        }
    };
    model.fitted.push(
        "the on-resistance, the enable threshold and the time constant of the output's rise"
            .to_string(),
    );
    model.assumed.push(
        "the output rises exponentially, following the input times the enable's ramp, until \
         the on-resistance takes over; reverse current is blocked; there is no quick output \
         discharge, undervoltage lockout or enable hysteresis"
            .to_string(),
    );
    model.parameters.insert(0, ("on_resistance".to_string(), ron));
    Ok((law, threshold, tau))
}

/// A switch's settled output at `vin` into a load, or `vin` with none.
fn final_output(law: &Law, vin: f64, load: Option<f64>) -> f64 {
    match load {
        Some(load) => fit::root(
            |out| law.drop(out / load, vin, 1.0) - (vin - out),
            0.0,
            vin,
        )
        .unwrap_or(vin),
        None => vin - law.drop(0.0, vin, 1.0),
    }
}

fn check_load_switch(
    model: &mut Model,
    bench: &mut Bench,
    law: &Law,
    threshold: f64,
    tau: f64,
    characteristics: &[Characteristic],
) {
    let single = |quantity| of(characteristics, quantity).first().copied();
    if let Some(c) = single(Quantity::SwitchOnResistance) {
        let vin = c.condition(INPUT_VOLTAGE).unwrap_or(DEFAULT_INPUT);
        check_on_resistance(model, bench, law, c, vin, true);
    }
    if let Some(c) = single(Quantity::CurrentLimit) {
        let vin = c.condition(INPUT_VOLTAGE).unwrap_or(DEFAULT_INPUT);
        check_limit(model, bench, c, vin, true);
    }
    if let Some(c) = single(Quantity::OutputRiseTime) {
        let vin = c.condition(INPUT_VOLTAGE).unwrap_or_default();
        let load = c.condition(LOAD_RESISTANCE);
        let settled = final_output(law, vin, load);
        let id = bench.id();
        let (input, output, pin) = (format!("{id}i"), format!("{id}o"), format!("{id}e"));
        let start = tau;
        bench.tran_stop = start + 12.0 * tau;
        bench.tran.push(Bench::voltage(&id, &input, vin, false));
        bench.tran.push(format!(
            "V{pin} {pin} 0 PULSE(0.000000e0 {} {} {} {} {} {})",
            number(2.0 * threshold),
            number(start),
            number(1e-4 * tau),
            number(1e-4 * tau),
            number(1e3),
            number(2e3)
        ));
        bench
            .tran
            .push(model.instance(&id, &[&input, &output, &pin, "0"]));
        if let Some(load) = load {
            bench
                .tran
                .push(format!("R{id}l {output} 0 {}", number(load)));
        }
        if let Some(farads) = c.condition(LOAD_CAPACITANCE) {
            bench
                .tran
                .push(format!("C{id}l {output} 0 {}", number(farads)));
        }
        let (low, high) = RISE_FRACTIONS;
        model.checks.push(Check {
            characteristic: c.index,
            role: Role::Fit,
            expected: c.value,
            probe: Probe::Delay {
                input: output.clone(),
                input_level: low * settled,
                output,
                output_level: high * settled,
                slow: false,
            },
        });
    }
    if let Some(c) = single(Quantity::EnableThreshold) {
        // A ramp so slow next to the time constant that the output lags the
        // enable by a part in a hundred thousand of the threshold.
        let vin = c.condition(INPUT_VOLTAGE).unwrap_or(DEFAULT_INPUT);
        let id = bench.id();
        let (input, output, pin) = (format!("{id}i"), format!("{id}o"), format!("{id}e"));
        bench.slow_stop = 2e4 * tau;
        bench.slow.push(Bench::voltage(&id, &input, vin, false));
        bench.slow.push(format!(
            "V{pin} {pin} 0 PWL(0.000000e0 {} {} {})",
            number(0.9 * threshold),
            number(bench.slow_stop),
            number(1.1 * threshold)
        ));
        bench
            .slow
            .push(model.instance(&id, &[&input, &output, &pin, "0"]));
        bench
            .slow
            .push(format!("R{id}l {output} 0 {}", number(threshold_load(law, threshold))));
        model.checks.push(Check {
            characteristic: c.index,
            role: Role::Fit,
            expected: c.value,
            probe: Probe::Trip {
                input: pin,
                output,
                level: 0.5 * vin,
                what: Trip::Rising,
            },
        });
    }
}

/// One forward drop at a current.
type Point = (f64, f64, usize);

fn ideal_diode(
    model: &mut Model,
    characteristics: &[Characteristic],
) -> Fitted<(Law, Vec<Point>, Option<Point>)> {
    let single = |quantity| of(characteristics, quantity).first().copied();
    let mut points: Vec<Point> = of(characteristics, Quantity::ForwardVoltage)
        .into_iter()
        .map(|c| (c.condition(FORWARD_CURRENT).unwrap_or_default(), c.value, c.index))
        .collect();
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    if let Some(pair) = points.windows(2).find(|pair| pair[0].0 == pair[1].0) {
        return Err(Invalid::new(
            characteristics[pair[1].2].field(&format!("conditions.{FORWARD_CURRENT}")),
            "two forward voltages at one current",
        ));
    }
    let given_ron = single(Quantity::SwitchOnResistance).map(|c| c.value);
    // The free parameters, in the order the values can fix them: the
    // regulation voltage, the on-resistance unless it is given, then the
    // regulation's slope.
    let free = if given_ron.is_some() { 2 } else { 3 }.min(points.len());
    let held = (points.len() > free).then(|| points[points.len() / 2]);
    let used: Vec<Point> = points
        .iter()
        .copied()
        .filter(|p| Some(p.2) != held.map(|h| h.2))
        .collect();
    // Starts: the drop at the smallest current is the regulation, and the
    // drop over the current at the largest is the on-resistance.
    let largest = used.last().map_or(0.0, |p| p.1 / p.0);
    let smallest_drop = used.first().map_or(0.0, |p| p.1);
    let unpack = |p: &[f64]| -> (f64, f64, f64) {
        let vreg = p[0];
        let (ron, rreg) = match (given_ron, free) {
            (Some(ron), 1) => (ron, 0.0),
            (Some(ron), _) => (ron, p[1]),
            (None, 1) => (0.0, 0.0),
            (None, 2) => (p[1], 0.0),
            (None, _) => (p[1], p[2]),
        };
        (vreg, ron, rreg)
    };
    let law_of = |p: &[f64]| {
        let (vreg, ron, rreg) = unpack(p);
        Law::IdealDiode {
            vreg,
            rreg,
            ron,
            blocking: DEFAULT_DIODE_BLOCKING,
        }
    };
    let start: Vec<f64> = match free {
        1 => vec![smallest_drop],
        2 if given_ron.is_some() => vec![smallest_drop, 0.0],
        2 => vec![smallest_drop, largest],
        _ => vec![smallest_drop, largest, 0.0],
    };
    let residuals = |p: &[f64]| -> Vec<f64> {
        let law = law_of(p);
        used.iter()
            .map(|(current, volts, _)| law.drop(*current, 0.0, 1.0) / volts - 1.0)
            .collect()
    };
    let clamp = |p: &mut [f64]| {
        for value in p.iter_mut() {
            *value = value.max(0.0);
        }
    };
    // The corner between regulation and full conduction can trap one start,
    // so the fit also starts with the regulation taking every value, and
    // keeps the better.
    let mut regulated = start.clone();
    if free == 3 {
        regulated[1] = 0.0;
        regulated[2] = used
            .last()
            .zip(used.first())
            .map_or(0.0, |(l, f)| ((l.1 - f.1) / (l.0 - f.0)).max(0.0));
    }
    let cost = |p: &[f64]| residuals(p).iter().map(|r| r * r).sum::<f64>();
    let best = [start.clone(), regulated]
        .iter()
        .map(|s| fit::least_squares(residuals, s, clamp))
        .min_by(|a, b| cost(a).total_cmp(&cost(b)))
        .unwrap_or(start);
    let (vreg, ron, rreg) = unpack(&best);
    let mut law = law_of(&best);
    if let Some(c) = single(Quantity::ReverseCurrent) {
        let volts = c.condition(REVERSE_VOLTAGE).unwrap_or_default();
        let target = c.value;
        let reading = |blocking: f64| {
            let trial = Law::IdealDiode {
                vreg,
                rreg,
                ron,
                blocking,
            };
            -trial.current_at(-volts, 0.0, 1.0).unwrap_or(f64::NAN) - target
        };
        let guess = (volts + vreg) / target;
        let blocking = fit::root(reading, 0.5 * guess, 2.0 * guess).ok_or_else(|| {
            Invalid::new(c.field("value"), "no blocking resistance reproduces this value")
        })?;
        law = Law::IdealDiode {
            vreg,
            rreg,
            ron,
            blocking,
        };
        model
            .fitted
            .push("the reverse blocking from the reverse current".to_string());
    } else {
        model.assumed.push(format!(
            "no reverse current: reverse current is blocked at {} A a volt",
            number(1.0 / DEFAULT_DIODE_BLOCKING)
        ));
    }
    model.fitted.push(format!(
        "the regulated forward drop{}{} from {} forward voltage values",
        if free >= 3 || (given_ron.is_some() && free >= 2) {
            " and its slope"
        } else {
            ""
        },
        if given_ron.is_none() && free >= 2 {
            ", and the on-resistance"
        } else {
            ""
        },
        used.len()
    ));
    if given_ron.is_none() && free < 2 {
        model.assumed.push(
            "one forward value and no on-resistance: the drop is the same at every current"
                .to_string(),
        );
    }
    model.assumed.push(
        "the switch turns off at once on any reverse current: no reverse-current threshold or \
         turn-off delay"
            .to_string(),
    );
    let blocking = match law {
        Law::IdealDiode { blocking, .. } => blocking,
        _ => DEFAULT_DIODE_BLOCKING,
    };
    model.parameters = vec![
        ("regulation_voltage".to_string(), vreg),
        ("regulation_resistance".to_string(), rreg),
        ("on_resistance".to_string(), ron),
        ("blocking_resistance".to_string(), blocking),
    ];
    Ok((law, used, held))
}

fn check_ideal_diode(
    model: &mut Model,
    bench: &mut Bench,
    law: &Law,
    fit_points: &[Point],
    held: Option<Point>,
    characteristics: &[Characteristic],
) {
    // A forward value: an input source and a load resistor sized so the part
    // carries the stated current.
    let forward = |model: &mut Model, bench: &mut Bench, (current, volts, index): Point, role| {
        let id = bench.id();
        let (input, output) = (format!("{id}i"), format!("{id}o"));
        let load = 10.0 / current;
        let vin = law.drop(current, 0.0, 1.0) + current * load;
        bench.dc.push(Bench::voltage(&id, &input, vin, false));
        bench.dc.push(model.instance(&id, &[&input, &output, "0"]));
        bench.dc.push(format!("R{id}l {output} 0 {}", number(load)));
        model.checks.push(Check {
            characteristic: index,
            role,
            expected: volts,
            probe: Probe::Drop {
                high: input,
                low: output,
                load: None,
            },
        });
    };
    for point in fit_points {
        forward(model, bench, *point, Role::Fit);
    }
    if let Some(point) = held {
        forward(model, bench, point, Role::HeldOut);
    }
    if let Some(c) = of(characteristics, Quantity::SwitchOnResistance)
        .first()
        .copied()
    {
        let id = bench.id();
        let (input, output) = (format!("{id}i"), format!("{id}o"));
        let current = c.condition(OUTPUT_CURRENT).unwrap_or_default();
        let load = 10.0 / current;
        let vin = law.drop(current, 0.0, 1.0) + current * load;
        bench.dc.push(Bench::voltage(&id, &input, vin, false));
        bench.dc.push(model.instance(&id, &[&input, &output, "0"]));
        bench.dc.push(format!("R{id}l {output} 0 {}", number(load)));
        model.checks.push(Check {
            characteristic: c.index,
            role: Role::Fit,
            expected: c.value,
            probe: Probe::Drop {
                high: input,
                low: output.clone(),
                load: Some(("0".to_string(), load)),
            },
        });
    }
    if let Some(c) = of(characteristics, Quantity::ReverseCurrent).first().copied() {
        let id = bench.id();
        let (input, output) = (format!("{id}i"), format!("{id}o"));
        let volts = c.condition(REVERSE_VOLTAGE).unwrap_or_default();
        bench.dc.push(Bench::voltage(&id, &input, 0.0, false));
        bench.dc.push(model.instance(&id, &[&input, &output, "0"]));
        bench
            .dc
            .push(Bench::voltage(&format!("{id}r"), &output, volts, false));
        model.checks.push(Check {
            characteristic: c.index,
            role: Role::Fit,
            expected: c.value,
            probe: Probe::Current {
                source: format!("V{id}r"),
                scale: -1.0,
            },
        });
    }
}

/// The drop's expression.
fn drop_expression(law: &Law) -> String {
    match *law {
        Law::Efuse {
            ron,
            limit,
            stiffness,
            lockout,
            cutoff,
        } => {
            let mut enable = String::from("1.000000e0");
            if let Some(t) = lockout {
                enable = format!(
                    "{enable}*(5.000000e-1+5.000000e-1*tanh((v(IN,COM)-{})/{}))",
                    number(t),
                    number(STEP_WIDTH * t)
                );
            }
            if let Some(t) = cutoff {
                enable = format!(
                    "{enable}*(5.000000e-1-5.000000e-1*tanh((v(IN,COM)-{})/{}))",
                    number(t),
                    number(STEP_WIDTH * t)
                );
            }
            let width = LIMIT_CORNER * limit;
            format!(
                "{}*{CURRENT}/({enable}+{}){}{}",
                number(ron),
                number(ron / OFF_RESISTANCE),
                term(
                    stiffness,
                    &expr_smooth_max_zero(&format!("{CURRENT}-{}", number(limit)), width)
                ),
                term(
                    -stiffness,
                    &expr_smooth_max_zero(&format!("-{CURRENT}-{}", number(limit)), width)
                )
            )
        }
        Law::LoadSwitch { ron, limit } => {
            let mut on = format!("{}*{CURRENT}", number(ron));
            if let Some((limit, stiffness)) = limit {
                on.push_str(&term(
                    stiffness,
                    &expr_smooth_max_zero(
                        &format!("{CURRENT}-{}", number(limit)),
                        LIMIT_CORNER * limit,
                    ),
                ));
            }
            format!(
                "{}{}",
                expr_smooth_max(&on, "(1.000000e0-v(r,COM))*v(IN,COM)", DROP_CORNER),
                term(
                    -BLOCKING_RESISTANCE,
                    &expr_smooth_max_zero(&format!("-{CURRENT}"), SINK_CORNER)
                )
            )
        }
        Law::IdealDiode {
            vreg,
            rreg,
            ron,
            blocking,
        } => format!(
            "{}{}",
            expr_smooth_max(
                &format!("{}{}", number(vreg), term(rreg, CURRENT)),
                &format!("{}*{CURRENT}", number(ron)),
                DROP_CORNER
            ),
            term(
                -blocking,
                &expr_smooth_max_zero(&format!("-{CURRENT}"), SINK_CORNER)
            )
        ),
    }
}

fn body(name: &str, kind: Kind, law: &Law, iq: Option<f64>, extra: &str) -> String {
    let pins = if kind == Kind::LoadSwitch {
        LOAD_SWITCH_PINS.join(" ")
    } else {
        PINS.join(" ")
    };
    let mut cards = format!(
        ".subckt {name} {pins}\nVS IN x 0.000000e0\nHS hs COM VS 1.000000e0\nBO x OUT V={}\n",
        drop_expression(law)
    );
    cards.push_str(extra);
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

    fn value(quantity: &str, value: f64, conditions: Value) -> Value {
        json!({ "quantity": quantity, "value": value, "conditions": conditions, "statistic": "typical" })
    }

    fn efuse_values() -> Vec<Value> {
        vec![
            value(
                "switch_on_resistance",
                0.03,
                json!({ "output_current_amps": 1.0, "input_voltage_volts": 12.0 }),
            ),
            value("current_limit", 2.0, json!({ "input_voltage_volts": 12.0 })),
            value("undervoltage_lockout", 4.0, json!({})),
            value("overvoltage_cutoff", 15.0, json!({})),
            value("quiescent_current", 3e-4, json!({ "input_voltage_volts": 12.0 })),
        ]
    }

    fn efuse_law(model: &Model) -> Law {
        Law::Efuse {
            ron: fitted(model, "on_resistance"),
            limit: fitted(model, "current_limit_setting"),
            stiffness: fitted(model, "limit_stiffness"),
            lockout: Some(fitted(model, "undervoltage_lockout_setting")),
            cutoff: Some(fitted(model, "overvoltage_cutoff_setting")),
        }
    }

    #[test]
    fn an_efuse_is_a_drop_whose_law_gives_back_every_value() {
        let model = build_model(Kind::Efuse, "tps2595_fit", &parsed(json!(efuse_values())), 25.0)
            .unwrap();
        let checked = text::check_fitted(model.file().as_bytes()).unwrap();
        assert_eq!(checked.pins(), ["IN", "OUT", "COM"]);
        assert_eq!(checked.kind, "efuse");
        assert_eq!(model.checks.len(), 5);
        let law = efuse_law(&model);
        // On-resistance at 1 A, from 12 V.
        let drop = law.drop(1.0, 12.0, 1.0);
        assert!((drop - 0.03).abs() < 1e-5, "{drop}");
        // A short draws the limit.
        let short = law.current_at(12.0, 12.0, 1.0).unwrap();
        assert!((short - 2.0).abs() < 1e-6, "{short}");
        // The output crosses half each threshold at that threshold.
        for (threshold, at) in [(4.0, 4.0), (15.0, 15.0)] {
            let load = threshold_load(&law, threshold);
            let out = efuse_output(&law, at, load);
            assert!((out - 0.5 * threshold).abs() < 1e-3 * threshold, "{threshold}: {out}");
        }
        // Outside the window it is open.
        assert!(efuse_output(&law, 3.5, 20.0) < 1e-6);
        assert!(efuse_output(&law, 16.0, 75.0) < 1e-5);
    }

    #[test]
    fn efuse_values_it_cannot_hold_are_refused_on_their_field() {
        let mut outside = efuse_values();
        outside[0] = value(
            "switch_on_resistance",
            0.03,
            json!({ "output_current_amps": 1.0, "input_voltage_volts": 3.0 }),
        );
        let error = build_model(Kind::Efuse, "x_fit", &parsed(json!(outside)), 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[0].conditions.input_voltage_volts");
        let mut inverted = efuse_values();
        inverted[3] = value("overvoltage_cutoff", 3.0, json!({}));
        let error = build_model(Kind::Efuse, "x_fit", &parsed(json!(inverted)), 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[3].value");
        let mut near_limit = efuse_values();
        near_limit[0] = value("switch_on_resistance", 0.03, json!({ "output_current_amps": 1.9 }));
        let error = build_model(Kind::Efuse, "x_fit", &parsed(json!(near_limit)), 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[0].conditions.output_current_amps");
        let mut unreachable = efuse_values();
        unreachable[0] = value("switch_on_resistance", 10.0, json!({ "output_current_amps": 0.1 }));
        let error =
            build_model(Kind::Efuse, "x_fit", &parsed(json!(unreachable)), 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[1].value");
        // An eFuse needs its limit.
        let none = parsed(json!([value("switch_on_resistance", 0.03, json!({ "output_current_amps": 1.0 }))]));
        assert!(build_model(Kind::Efuse, "x_fit", &none, 25.0).is_err());
    }

    #[test]
    fn a_load_switch_has_an_enable_pin_and_a_time_constant_for_its_rise() {
        let values = parsed(json!([
            value("switch_on_resistance", 0.05, json!({ "output_current_amps": 1.0 })),
            value("enable_threshold", 1.0, json!({})),
            value(
                "output_rise_time",
                2.2e-4,
                json!({ "input_voltage_volts": 5.0, "load_resistance_ohms": 10.0 })
            ),
        ]));
        let model = build_model(Kind::LoadSwitch, "tps22918_fit", &values, 25.0).unwrap();
        let checked = text::check_fitted(model.file().as_bytes()).unwrap();
        assert_eq!(checked.pins(), ["IN", "OUT", "EN", "COM"]);
        // 10 to 90 percent of the settled output, k = 10 / 10.05.
        let k = final_output(&Law::LoadSwitch { ron: 0.05, limit: None }, 5.0, Some(10.0)) / 5.0;
        let tau = fitted(&model, "enable_time_constant");
        assert!((tau * ((1.0 - 0.1 * k) / (1.0 - 0.9 * k)).ln() - 2.2e-4).abs() < 1e-9);
        assert!(model.assumed.iter().any(|a| a.contains("no current limit")));
        // Without values, it says what it took.
        let bare = parsed(json!([value("switch_on_resistance", 0.05, json!({ "output_current_amps": 1.0 }))]));
        let model = build_model(Kind::LoadSwitch, "bare_fit", &bare, 25.0).unwrap();
        for note in ["no enable threshold", "no rise time", "no quiescent current"] {
            assert!(model.assumed.iter().any(|a| a.contains(note)), "{note}");
        }
        // The law blocks reverse current: 1 uA a volt.
        let law = Law::LoadSwitch { ron: 0.05, limit: None };
        let reverse = law.current_at(-5.0, 0.0, 1.0).unwrap();
        assert!(reverse < 0.0 && reverse > -1e-5, "{reverse}");
    }

    #[test]
    fn an_ideal_diode_fits_its_forward_curve_holds_one_out_and_blocks_reverse_current() {
        let values = parsed(json!([
            value("forward_voltage", 0.021, json!({ "forward_current_amps": 0.1 })),
            value("forward_voltage", 0.03, json!({ "forward_current_amps": 1.0 })),
            value("forward_voltage", 0.06, json!({ "forward_current_amps": 2.0 })),
            value("forward_voltage", 0.09, json!({ "forward_current_amps": 3.0 })),
            value("reverse_current", 5e-7, json!({ "reverse_voltage_volts": 5.0 }))
        ]));
        let model = build_model(Kind::IdealDiode, "lm66100_fit", &values, 25.0).unwrap();
        let checked = text::check_fitted(model.file().as_bytes()).unwrap();
        assert_eq!(checked.pins(), ["IN", "OUT", "COM"]);
        let held: Vec<_> = model.checks.iter().filter(|c| c.role == Role::HeldOut).collect();
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].characteristic, 2);
        let law = Law::IdealDiode {
            vreg: fitted(&model, "regulation_voltage"),
            rreg: fitted(&model, "regulation_resistance"),
            ron: fitted(&model, "on_resistance"),
            blocking: fitted(&model, "blocking_resistance"),
        };
        for (current, volts) in [(0.1, 0.021), (1.0, 0.03), (2.0, 0.06), (3.0, 0.09)] {
            let drop = law.drop(current, 0.0, 1.0);
            assert!((drop / volts - 1.0).abs() < 1e-3, "{current}: {drop}");
        }
        let reverse = -law.current_at(-5.0, 0.0, 1.0).unwrap();
        assert!((reverse / 5e-7 - 1.0).abs() < 1e-6, "{reverse}");
        let twice = parsed(json!([
            value("forward_voltage", 0.02, json!({ "forward_current_amps": 0.1 })),
            value("forward_voltage", 0.03, json!({ "forward_current_amps": 0.1 }))
        ]));
        let error = build_model(Kind::IdealDiode, "x_fit", &twice, 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[1].conditions.forward_current_amps");
    }
}
