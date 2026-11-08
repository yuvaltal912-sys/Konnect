//! Op-amps and comparators, as one behavioural gain core.
//!
//! The core is a single-pole amplifier between the rails. A mid-supply node
//! `mid` is made by an E source from `VCC` and `VEE`, so nothing inside
//! refers to ground. The input stage is a transconductance whose current is
//! limited to `imax`, which sets the slew rate; it drives the gain node `a`,
//! loaded by `RA` and `CA`, which set the DC gain and the gain-bandwidth
//! product. Two clamp diodes hold `a` inside the rails less the output swing,
//! so a saturated output sits where the datasheet says and the gain node never
//! winds up. An E source buffers `a` to the output, through the output
//! resistance and a 0 V sense source; the sensed current, turned to a voltage
//! by an H source, is drawn from `VCC` when the output sources current, so a
//! sinking output's current returns through `VEE` and a sourcing one's comes
//! from `VCC`.
//!
//! The clamps are diodes, not `min` and `max`, because a hard clamp leaves the
//! solver a flat region: on libngspice 45.2 a clamped output failed every
//! operating-point method, and the diode clamp solved on its first.
//!
//! A comparator is the same core with a large gain and, when the datasheet
//! gives one, hysteresis: the input sees `(Vh / 2) tanh(v(a, mid) / w)` more,
//! so the trip points sit `Vh / 2` either side of the offset whatever the
//! supply. Its delay is the time the gain node takes to slew from one clamp to
//! the middle of the output swing, which the fit integrates.

use super::bench::{Bench, Check, Probe, Role, Trip};
use super::fit::{self, thermal_voltage};
use super::text::{diode_card, number};
use super::{
    of, Characteristic, Element, Fitted, Invalid, Kind, Model, Quantity, FREQUENCY,
    LOAD_RESISTANCE, OVERDRIVE, SUPPLY_VOLTAGE,
};

/// The model's pins, in the order an instance lists its nodes.
pub(crate) const PINS: [&str; 5] = ["INP", "INN", "VCC", "VEE", "OUT"];
/// The gain node's load. The input stage's transconductance is the DC gain
/// over this, so a junction's gmin leak beside it moves the gain by a part in
/// a million.
const GAIN_RESISTANCE: f64 = 1e6;
/// The clamp diodes' law.
const CLAMP_IS: f64 = 1e-15;
const CLAMP_N: f64 = 1.0;
/// A check whose characteristic states no supply runs at this total supply,
/// unless another characteristic states one.
const DEFAULT_SUPPLY: f64 = 10.0;
/// A comparator's DC gain.
const COMPARATOR_GAIN: f64 = 1e6;
/// The gain-node voltage over which a comparator's hysteresis turns over.
const HYSTERESIS_WIDTH: f64 = 0.1;
/// Without a slew rate, the input stage is linear over this many volts, as a
/// plain differential pair is.
const ASSUMED_LINEAR_RANGE: f64 = 0.2;
/// The width over which a comparator stage's `|x|` is rounded, in volts.
const SMOOTH_ABS: f64 = 1e-6;
/// The softest turnover a comparator stage takes: a current that rises as the
/// fourth root of the input past its smoothing.
const MIN_POWER: f64 = 0.25;
/// How far a trip point read on the slow ramp may lag, as a fraction of the
/// hysteresis (or of 5 mV without one).
const TRIP_LAG: f64 = 1e-3;
/// Options for the transient benches: the bench's breakdown tolerances stall
/// a transient at a step's first edge.
const TRANSIENT_TOLERANCES: &str = ".options reltol=1e-4 vntol=1e-7 abstol=1e-13";
/// The DC servo that biases an open-loop gain fixture: a resistor from the
/// output to the inverting input, and a capacitor that grounds that input at
/// every frequency a check reads. Their product is the servo's time constant.
const SERVO_SECONDS: f64 = 1e15;

/// The fitted core.
#[derive(Debug, Clone, Copy)]
struct Core {
    gm: f64,
    ca: f64,
    imax: f64,
    vos: f64,
    ib: f64,
    iq: f64,
    rout: f64,
    /// `v(ch) = v(VCC) - high_offset`; the clamp holds `a` a diode drop above.
    high_offset: f64,
    /// `v(cl) = v(VEE) + low_offset`; the clamp holds `a` a diode drop below.
    low_offset: f64,
    hysteresis: f64,
    /// A comparator's stage turns over smoothly, `imax x / (|x| + soft)`;
    /// an op-amp's is linear up to a hard limit.
    soft: Option<f64>,
    /// How sharply a comparator stage turns over: its current rises as
    /// `|x|^power` between `SMOOTH_ABS` and `soft`.
    power: f64,
    vt: f64,
}

impl Core {
    fn drop(&self, current: f64) -> f64 {
        CLAMP_N * self.vt * (current.max(0.0) / CLAMP_IS + 1.0).ln()
    }

    /// The gain node, relative to `mid`, held at the high clamp by the input
    /// stage's full current at total supply `supply`, with `load` drawn out
    /// of it by the output.
    fn high_level(&self, supply: f64) -> f64 {
        let full = self.full_current(1.0);
        let mut level = supply / 2.0;
        for _ in 0..40 {
            level =
                supply / 2.0 - self.high_offset + self.drop(full - level / GAIN_RESISTANCE);
        }
        level
    }

    fn low_level(&self, supply: f64) -> f64 {
        let full = self.full_current(-1.0);
        let mut level = -supply / 2.0;
        for _ in 0..40 {
            level =
                -supply / 2.0 + self.low_offset - self.drop(full + level / GAIN_RESISTANCE);
        }
        level
    }

    /// The input stage's current at an input difference `x`, offset and
    /// hysteresis included.
    fn law(&self, x: f64) -> f64 {
        match self.soft {
            Some(soft) => {
                let size = (x * x + SMOOTH_ABS * SMOOTH_ABS).sqrt();
                self.imax * x
                    / ((size + SMOOTH_ABS).powf(1.0 - self.power) * (size + soft).powf(self.power))
            }
            None => (self.gm * x).clamp(-self.imax, self.imax),
        }
    }

    /// The input difference over which the stage leaves its linear range.
    fn linear_range(&self) -> f64 {
        self.soft.unwrap_or(self.imax / self.gm)
    }

    /// The input a bench holds past the offset to drive the output to the
    /// rail in the direction `sign`.
    fn overdrive(&self, sign: f64) -> f64 {
        let reach = if self.soft.is_some() { 100.0 } else { 10.0 };
        sign * (self.vos + self.hysteresis + reach * self.linear_range() + 1e-3)
    }

    /// The stage's current, as a magnitude, with the output held at the rail
    /// in the direction `sign` by [`Core::overdrive`].
    fn full_current(&self, sign: f64) -> f64 {
        (self.law(self.overdrive(sign) - self.vos + sign * self.hysteresis / 2.0)).abs()
    }

    /// The input stage's current at the gain-node voltage `a` after a step
    /// to `overdrive` past the rising trip point.
    fn stage_current(&self, a: f64, overdrive: f64) -> f64 {
        let x = overdrive + self.hysteresis / 2.0 * (1.0 + (a / HYSTERESIS_WIDTH).tanh());
        self.law(x)
    }

    /// Seconds per farad of `CA` for the gain node to slew from the low clamp
    /// to the middle of the swing after a step to `overdrive`.
    fn delay_per_farad(&self, overdrive: f64, supply: f64) -> Option<f64> {
        let (low, high) = (self.low_level(supply), self.high_level(supply));
        let middle = (low + high) / 2.0;
        // The clamp diodes' anodes and cathodes, relative to `mid`. The low
        // clamp goes on pushing current into the gain node while it turns off.
        let (low_clamp, high_clamp) = (
            -supply / 2.0 + self.low_offset,
            supply / 2.0 - self.high_offset,
        );
        let diode = |volts: f64| CLAMP_IS * ((volts / (CLAMP_N * self.vt)).min(80.0).exp() - 1.0);
        let integrand = |a: f64| {
            let net = self.stage_current(a, overdrive) - a / GAIN_RESISTANCE
                + diode(low_clamp - a)
                - diode(a - high_clamp);
            (net > 0.0).then(|| 1.0 / net)
        };
        const STEPS: usize = 4000;
        let width = (middle - low) / STEPS as f64;
        let mut sum = 0.0;
        for step in 0..=STEPS {
            let weight = if step == 0 || step == STEPS {
                1.0
            } else if step % 2 == 1 {
                4.0
            } else {
                2.0
            };
            sum += weight * integrand(low + width * step as f64)?;
        }
        Some(sum * width / 3.0)
    }
}

/// The supply a check runs at: its own, else the first any characteristic
/// states, else the default.
fn supply_of(characteristic: Option<&Characteristic>, default: f64) -> f64 {
    characteristic
        .and_then(|c| c.condition(SUPPLY_VOLTAGE))
        .unwrap_or(default)
}

pub(crate) fn build(
    kind: Kind,
    name: &str,
    characteristics: &[Characteristic],
    temperature: f64,
) -> Fitted<Model> {
    let comparator = kind == Kind::Comparator;
    let mut model = Model::new(kind, name, Element::Subcircuit, PINS.to_vec());
    let stated = characteristics
        .iter()
        .find_map(|c| c.condition(SUPPLY_VOLTAGE));
    let supply = stated.unwrap_or(DEFAULT_SUPPLY);
    if stated.is_none() {
        model.assumed.push(format!(
            "no supply voltage was stated, so the checks ran at {} V across the supply pins",
            number(DEFAULT_SUPPLY)
        ));
    }
    let single = |quantity| of(characteristics, quantity).first().copied();
    let vos = single(Quantity::InputOffsetVoltage);
    let ib = single(Quantity::InputBiasCurrent);
    let iq = single(Quantity::SupplyCurrent);
    let rout = single(Quantity::OutputResistance);
    let swing_high = single(Quantity::OutputSwingFromPositiveRail);
    let swing_low = single(Quantity::OutputSwingFromNegativeRail);
    let mut core = Core {
        gm: 0.0,
        ca: 0.0,
        imax: 0.0,
        vos: vos.map_or(0.0, |c| c.value),
        ib: ib.map_or(0.0, |c| c.value),
        iq: iq.map_or(0.0, |c| c.value),
        rout: rout.map_or(0.0, |c| c.value),
        high_offset: 0.0,
        low_offset: 0.0,
        hysteresis: 0.0,
        soft: None,
        power: 1.0,
        vt: thermal_voltage(temperature),
    };
    if vos.is_some() {
        model.assumed.push(
            "the input offset is applied with one sign: a follower's output sits the offset \
             below its input"
                .to_string(),
        );
    } else {
        model.assumed.push("no input offset voltage".to_string());
    }
    if ib.is_none() {
        model.assumed.push("no input bias current".to_string());
    } else {
        model.assumed.push(
            "both inputs draw the same bias current: no offset current".to_string(),
        );
    }
    if iq.is_none() {
        model
            .assumed
            .push("no supply current beyond what the output delivers".to_string());
    }
    if rout.is_none() {
        model.assumed.push(
            "no open-loop output resistance: a saturated output holds its level under load"
                .to_string(),
        );
    }

    // The gain and the speed.
    let mut gains: Vec<&Characteristic> = Vec::new();
    let mut delays: Vec<(f64, f64, usize)> = Vec::new();
    if comparator {
        core.hysteresis = single(Quantity::Hysteresis).map_or(0.0, |c| c.value);
        model.assumed.push(format!(
            "a DC gain of {}, an input stage that turns over as x / (|x| + v), and a push-pull \
             output: an open-drain part needs its pull-up modelled apart",
            number(COMPARATOR_GAIN)
        ));
        if core.hysteresis == 0.0 {
            model.assumed.push("no hysteresis".to_string());
        }
        delays = of(characteristics, Quantity::PropagationDelay)
            .into_iter()
            .map(|c| (c.condition(OVERDRIVE).unwrap_or_default(), c.value, c.index))
            .collect();
        delays.sort_by(|a, b| a.0.total_cmp(&b.0));
        // A start for the stage's turnover, which the delays then fit.
        set_turnover(&mut core, delays.first().map_or(1e-3, |d| d.0));
        if let Some(pair) = delays.windows(2).find(|pair| pair[0].0 == pair[1].0) {
            return Err(Invalid::new(
                characteristics[pair[1].2].field(&format!("conditions.{OVERDRIVE}")),
                "two propagation delays at one overdrive",
            ));
        }
    } else {
        let (dc, at_frequency): (Vec<&Characteristic>, Vec<&Characteristic>) =
            of(characteristics, Quantity::OpenLoopGain)
                .into_iter()
                .partition(|c| c.condition(FREQUENCY).is_none());
        let [dc] = dc.as_slice() else {
            return Err(Invalid::new(
                "characteristics",
                "an op_amp needs exactly one open_loop_gain without a frequency, the DC gain; \
                 values off the Bode plot carry their frequency_hz",
            ));
        };
        if dc.value < 100.0 {
            return Err(Invalid::new(
                dc.field("value"),
                "the DC open-loop gain must be at least 100, as a ratio: 100 dB is 1e5",
            ));
        }
        let gbw = single(Quantity::GainBandwidthProduct)
            .map_or(0.0, |c| c.value);
        core.gm = dc.value / GAIN_RESISTANCE;
        core.ca = core.gm / (std::f64::consts::TAU * gbw);
        let slew = single(Quantity::SlewRate).map_or_else(
            || {
                model.assumed.push(format!(
                    "no slew rate: the input stage is linear over {} V, which gives {} V/s",
                    number(ASSUMED_LINEAR_RANGE),
                    number(std::f64::consts::TAU * gbw * ASSUMED_LINEAR_RANGE)
                ));
                std::f64::consts::TAU * gbw * ASSUMED_LINEAR_RANGE
            },
            |c| c.value,
        );
        core.imax = slew * core.ca;
        gains.push(dc);
        gains.extend(at_frequency);
        model.fitted.push(
            "the DC gain, a single pole at the gain-bandwidth product over the gain, and the \
             slew rate"
                .to_string(),
        );
        model.assumed.push(
            "a single pole: the phase margin is 90 degrees and the gain falls 20 dB a decade \
             to well past the gain-bandwidth product"
                .to_string(),
        );
    }

    // The clamps, then (for a comparator) the delay, which moves the clamps'
    // currents, so they are found together.
    let mut held_delay = None;
    for _ in 0..4 {
        clamps(&mut core, &mut model, swing_high, swing_low, supply)?;
        if comparator {
            held_delay = fit_delays(&mut core, &delays, supply, characteristics)?;
        }
    }
    clamps(&mut core, &mut model, swing_high, swing_low, supply)?;
    if comparator {
        model.fitted.push(format!(
            "the gain node's capacitance{} from {} propagation delay values",
            if delays.len() > 1 {
                " and the input stage's current"
            } else {
                ""
            },
            delays.len() - usize::from(held_delay.is_some())
        ));
        if delays.len() == 1 {
            model.assumed.push(
                "one propagation delay: the input stage is half turned over at that overdrive"
                    .to_string(),
            );
        }
    }
    model.fitted.push(format!(
        "the clamps from {}",
        match (swing_high.is_some(), swing_low.is_some()) {
            (true, true) => "both output swings",
            (true, false) => "the positive swing; the output reaches the negative rail",
            (false, true) => "the negative swing; the output reaches the positive rail",
            (false, false) => "no swing values: the output reaches each rail with no load",
        }
    ));
    model.assumed.push(
        "no common-mode or supply rejection, no input range limit, no noise and no temperature \
         behaviour"
            .to_string(),
    );
    if let (Some(ib), Some(iq)) = (ib, iq) {
        // A follower's output feeds the inverting input's bias current from
        // the positive supply, so the supply current includes it once.
        if !comparator && ib.value < iq.value {
            core.iq = iq.value - ib.value;
        }
    }

    model.body = body(name, &core, temperature);
    model.parameters = vec![
        ("transconductance".to_string(), core.gm),
        ("gain_resistance".to_string(), GAIN_RESISTANCE),
        ("gain_capacitance".to_string(), core.ca),
        ("stage_current".to_string(), core.imax),
        ("input_offset_voltage".to_string(), core.vos),
        ("input_bias_current".to_string(), core.ib),
        ("quiescent_current".to_string(), core.iq),
        ("output_resistance".to_string(), core.rout),
        ("high_clamp_offset".to_string(), core.high_offset),
        ("low_clamp_offset".to_string(), core.low_offset),
        ("hysteresis".to_string(), core.hysteresis),
    ];
    if let Some(soft) = core.soft {
        model.parameters.push(("stage_turnover".to_string(), soft));
        model.parameters.push(("stage_power".to_string(), core.power));
    }

    let mut bench = Bench::default();
    bench.tran.push(TRANSIENT_TOLERANCES.to_string());
    bench.slow.push(TRANSIENT_TOLERANCES.to_string());
    if comparator {
        for (at, &(overdrive, delay, index)) in delays.iter().enumerate() {
            let role = if Some(at) == held_delay {
                Role::HeldOut
            } else {
                Role::Fit
            };
            check_delay(
                &mut model,
                &mut bench,
                &core,
                supply_of(Some(&characteristics[index]), supply),
                (overdrive, delay, index),
                role,
            );
        }
        check_trips(&mut model, &mut bench, &core, supply, characteristics);
    } else {
        let product = single(Quantity::GainBandwidthProduct);
        check_gains(&mut model, &mut bench, &core, &gains, product, rout, supply);
        if let Some(c) = single(Quantity::SlewRate) {
            check_slew(&mut model, &mut bench, &core, c, supply, swing_high, swing_low);
        }
        if let Some(c) = vos {
            let id = follower(&mut bench, &model, supply_of(Some(c), supply), 0.0);
            model.checks.push(Check {
                characteristic: c.index,
                role: Role::Fit,
                expected: c.value,
                probe: Probe::Affine {
                    node: format!("{id}o"),
                    offset: 0.0,
                    scale: -1.0,
                },
            });
        }
    }
    if let Some(c) = ib {
        let id = held_input(&mut bench, &model, &core, supply_of(Some(c), supply), -1.0);
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
    if let Some(c) = iq {
        let volts = supply_of(Some(c), supply);
        let id = if comparator {
            held_input(&mut bench, &model, &core, volts, -1.0)
        } else {
            follower(&mut bench, &model, volts, 0.0)
        };
        model.checks.push(Check {
            characteristic: c.index,
            role: Role::Fit,
            expected: c.value,
            probe: Probe::Current {
                source: format!("V{id}p"),
                scale: -1.0,
            },
        });
    }
    for (c, sign) in [(swing_high, 1.0), (swing_low, -1.0)] {
        let Some(c) = c else { continue };
        let volts = supply_of(Some(c), supply);
        let id = held_input(&mut bench, &model, &core, volts, sign);
        let load = c.condition(LOAD_RESISTANCE).unwrap_or_default();
        bench
            .dc
            .push(format!("R{id}l {id}o 0 {}", number(load)));
        model.checks.push(Check {
            characteristic: c.index,
            role: Role::Fit,
            expected: c.value,
            probe: Probe::Affine {
                node: format!("{id}o"),
                offset: volts / 2.0,
                scale: -sign,
            },
        });
    }
    if comparator {
        if let Some(c) = rout {
            let id = bench.id();
            let volts = supply_of(Some(c), supply);
            supplies(&mut bench.ac, &id, volts);
            let (input, output) = (format!("{id}i"), format!("{id}o"));
            bench.ac.push(Bench::voltage(&id, &input, overdrive_for(&core, 1.0), false));
            bench
                .ac
                .push(model.instance(&id, &[&input, "0", &format!("{id}p"), &format!("{id}n"), &output]));
            bench.ac.push(format!("I{id}a 0 {output} DC 0 AC 1"));
            model.checks.push(Check {
                characteristic: c.index,
                role: Role::Fit,
                expected: c.value,
                probe: Probe::Gain {
                    node: output,
                    hz: 1e3,
                    scale: 1.0,
                },
            });
        }
    }
    model.bench = bench;
    Ok(model)
}

/// Place the clamps so each saturated output reads its swing.
fn clamps(
    core: &mut Core,
    model: &mut Model,
    high: Option<&Characteristic>,
    low: Option<&Characteristic>,
    supply: f64,
) -> Fitted<()> {
    let offset = |core: &Core, c: Option<&Characteristic>, sign: f64| -> Fitted<f64> {
        let full = core.full_current(sign);
        let Some(c) = c else {
            // The gain node reaches the rail with no load.
            return Ok(core.drop(full - supply / 2.0 / GAIN_RESISTANCE));
        };
        let volts = c.condition(SUPPLY_VOLTAGE).unwrap_or(supply);
        let load = c.condition(LOAD_RESISTANCE).unwrap_or_default();
        let level = (volts / 2.0 - c.value) * (load + core.rout) / load;
        if level <= 0.0 {
            return Err(Invalid::new(
                c.field("value"),
                "the swing leaves the output no room above the middle of the supply",
            ));
        }
        let current = full - level / GAIN_RESISTANCE;
        if current < 0.1 * full {
            return Err(Invalid::new(
                c.field("value"),
                "the gain stage's current, set by the slew rate or the delays, is too small \
                 to reach this swing through the gain resistance",
            ));
        }
        Ok(volts / 2.0 - level + core.drop(current))
    };
    core.high_offset = offset(core, high, 1.0)?;
    core.low_offset = offset(core, low, -1.0)?;
    let range = core.high_level(supply) - core.low_level(supply);
    if range <= 0.0 {
        return Err(Invalid::new(
            "characteristics",
            "the two swings leave the output no range",
        ));
    }
    if model.kind == Kind::Comparator
        && (core.high_level(supply) < 3.0 * HYSTERESIS_WIDTH
            || core.low_level(supply) > -3.0 * HYSTERESIS_WIDTH)
    {
        return Err(Invalid::new(
            "characteristics",
            "the supply and swings leave the output too little range either side of the \
             middle of the supply",
        ));
    }
    Ok(())
}

/// Fit the gain node's capacitance and the stage's turnover to the delays:
/// with one, the stage is half turned over at its overdrive; with two, the
/// turnover voltage is fitted; with three or more, its sharpness too, and
/// with four or more the middle one is held out. Returns the held-out
/// delay's place.
fn fit_delays(
    core: &mut Core,
    delays: &[(f64, f64, usize)],
    supply: f64,
    characteristics: &[Characteristic],
) -> Fitted<Option<usize>> {
    let held = (delays.len() >= 4).then_some(delays.len() / 2);
    let used: Vec<(f64, f64, usize)> = delays
        .iter()
        .enumerate()
        .filter(|(at, _)| Some(*at) != held)
        .map(|(_, d)| *d)
        .collect();
    let supply_at = |index: usize| supply_of(Some(&characteristics[index]), supply);
    // The capacitance that best fits the delays at a turnover, on the log of
    // each, and the residuals left.
    let fit_at = |soft: f64, power: f64| -> Option<(f64, Vec<f64>)> {
        let mut trial = *core;
        trial.power = power;
        set_turnover(&mut trial, soft);
        let mut logs = Vec::new();
        for (overdrive, delay, index) in &used {
            let per_farad = trial.delay_per_farad(*overdrive, supply_at(*index))?;
            logs.push(delay.ln() - per_farad.ln());
        }
        let mean = logs.iter().sum::<f64>() / logs.len() as f64;
        Some((mean.exp(), logs.iter().map(|l| l - mean).collect()))
    };
    let spread = |soft: f64, power: f64| {
        fit_at(soft, power).map_or(f64::INFINITY, |(_, r)| r.iter().map(|x| x * x).sum())
    };
    let (low, high) = ((used[0].0 * 1e-3).ln(), (used[used.len() - 1].0 * 1e3).ln());
    let (soft, power) = match used.len() {
        1 => (used[0].0, 1.0),
        2 => (
            fit::golden_minimum(|log| spread(log.exp(), 1.0), low, high).exp(),
            1.0,
        ),
        _ => {
            let start = fit::golden_minimum(|log| spread(log.exp(), 1.0), low, high);
            let best = fit::least_squares(
                |p| {
                    fit_at(p[0].exp(), p[1]).map_or_else(
                        || vec![f64::INFINITY; used.len()],
                        |(_, residuals)| residuals,
                    )
                },
                &[start, 1.0],
                |p| {
                    p[0] = p[0].clamp(low, high);
                    p[1] = p[1].clamp(MIN_POWER, 1.0);
                },
            );
            (best[0].exp(), best[1])
        }
    };
    let Some((ca, _)) = fit_at(soft, power) else {
        return Err(Invalid::new(
            characteristics[used[0].2].field("value"),
            "the comparator's gain node cannot slew to the middle of its swing at this \
             overdrive",
        ));
    };
    core.power = power;
    set_turnover(core, soft);
    core.ca = ca;
    Ok(held)
}

/// A comparator stage that turns over at `soft`, with the DC gain fixed.
fn set_turnover(core: &mut Core, soft: f64) {
    core.soft = Some(soft);
    core.imax = COMPARATOR_GAIN * soft / GAIN_RESISTANCE;
    core.gm = core.imax / soft;
}

/// `V<id>p` and `V<id>n`: the supplies, split about ground.
fn supplies(lines: &mut Vec<String>, id: &str, volts: f64) {
    lines.push(Bench::voltage(&format!("{id}p"), &format!("{id}p"), volts / 2.0, false));
    lines.push(Bench::voltage(&format!("{id}n"), &format!("{id}n"), -volts / 2.0, false));
}

fn instance_nodes(id: &str, inp: &str, inn: &str) -> Vec<String> {
    vec![
        inp.to_string(),
        inn.to_string(),
        format!("{id}p"),
        format!("{id}n"),
        format!("{id}o"),
    ]
}

/// A unity-gain follower on the operating point, its input at `volts`.
fn follower(bench: &mut Bench, model: &Model, supply: f64, volts: f64) -> String {
    let id = bench.id();
    supplies(&mut bench.dc, &id, supply);
    let input = format!("{id}i");
    bench.dc.push(Bench::voltage(&id, &input, volts, false));
    let nodes = instance_nodes(&id, &input, &format!("{id}o"));
    let nodes: Vec<&str> = nodes.iter().map(String::as_str).collect();
    bench.dc.push(model.instance(&id, &nodes));
    id
}

/// An input overdrive that saturates the input stage, in the direction `sign`.
fn overdrive_for(core: &Core, sign: f64) -> f64 {
    core.overdrive(sign)
}

/// The output driven hard to one rail on the operating point: `INP` held
/// past the offset, `INN` at ground.
fn held_input(bench: &mut Bench, model: &Model, core: &Core, supply: f64, sign: f64) -> String {
    let id = bench.id();
    supplies(&mut bench.dc, &id, supply);
    let input = format!("{id}i");
    bench
        .dc
        .push(Bench::voltage(&id, &input, overdrive_for(core, sign), false));
    let nodes = instance_nodes(&id, &input, "0");
    let nodes: Vec<&str> = nodes.iter().map(String::as_str).collect();
    bench.dc.push(model.instance(&id, &nodes));
    id
}

/// The open-loop gain, by a unit AC drive on `INP` with a DC servo from the
/// output to `INN`, so the offset cannot saturate the output.
fn check_gains(
    model: &mut Model,
    bench: &mut Bench,
    core: &Core,
    gains: &[&Characteristic],
    product: Option<&Characteristic>,
    rout: Option<&Characteristic>,
    supply: f64,
) {
    let a0 = core.gm * GAIN_RESISTANCE;
    let gbw = core.gm / (std::f64::consts::TAU * core.ca);
    let pole = gbw / a0;
    let id = bench.id();
    let (input, inverting, output) = (format!("{id}i"), format!("{id}m"), format!("{id}o"));
    supplies(&mut bench.ac, &id, supply);
    bench.ac.push(Bench::voltage(&id, &input, 0.0, true));
    let nodes = instance_nodes(&id, &input, &inverting);
    let nodes: Vec<&str> = nodes.iter().map(String::as_str).collect();
    bench.ac.push(model.instance(&id, &nodes));
    // The bias current through the servo resistor must leave the output well
    // inside the rails.
    let servo = if core.ib > 0.0 {
        (0.05 * supply / core.ib).min(1e6)
    } else {
        1e6
    };
    bench
        .ac
        .push(format!("R{id}f {output} {inverting} {}", number(servo)));
    bench
        .ac
        .push(format!("C{id}f {inverting} 0 {}", number(SERVO_SECONDS / servo)));
    for c in gains {
        let (hz, role) = match c.condition(FREQUENCY) {
            Some(hz) => (hz, Role::HeldOut),
            None => (pole / 100.0, Role::Fit),
        };
        model.checks.push(Check {
            characteristic: c.index,
            role,
            expected: c.value,
            probe: Probe::Gain {
                node: output.clone(),
                hz,
                scale: 1.0,
            },
        });
    }
    if let Some(c) = product {
        // Far above the pole, the gain times the frequency is the product.
        let hz = gbw / 10.0;
        model.checks.push(Check {
            characteristic: c.index,
            role: Role::Fit,
            expected: c.value,
            probe: Probe::Gain {
                node: output.clone(),
                hz,
                scale: hz,
            },
        });
    }
    if let Some(c) = rout {
        let id = bench.id();
        let (input, inverting, output) = (format!("{id}i"), format!("{id}m"), format!("{id}o"));
        supplies(&mut bench.ac, &id, supply_of(Some(c), supply));
        bench.ac.push(Bench::voltage(&id, &input, 0.0, false));
        let nodes = instance_nodes(&id, &input, &inverting);
        let nodes: Vec<&str> = nodes.iter().map(String::as_str).collect();
        bench.ac.push(model.instance(&id, &nodes));
        bench
            .ac
            .push(format!("R{id}f {output} {inverting} {}", number(servo)));
        bench
            .ac
            .push(format!("C{id}f {inverting} 0 {}", number(SERVO_SECONDS / servo)));
        bench.ac.push(format!("I{id}a 0 {output} DC 0 AC 1"));
        model.checks.push(Check {
            characteristic: c.index,
            role: Role::Fit,
            expected: c.value,
            probe: Probe::Gain {
                node: output,
                hz: gbw / 10.0,
                scale: 1.0,
            },
        });
    }
}

/// The slew rate: a follower stepped across most of its swing, timed between
/// two levels inside the slew, before the linear settling begins.
fn check_slew(
    model: &mut Model,
    bench: &mut Bench,
    core: &Core,
    c: &Characteristic,
    supply: f64,
    swing_high: Option<&Characteristic>,
    swing_low: Option<&Characteristic>,
) {
    let volts = supply_of(Some(c), supply);
    let headroom = swing_high
        .map_or(0.0, |s| s.value)
        .max(swing_low.map_or(0.0, |s| s.value));
    let span = volts / 2.0 - headroom;
    // The input stage leaves its limit when the output is within this of the
    // input.
    let linear = core.imax / core.gm;
    let top = (1.0 - 2.0 * linear / span).min(0.7);
    if span <= 0.0 || top < 0.3 {
        model.assumed.push(
            "the slew rate is not checked: the swing is too small next to the input stage's \
             linear range"
                .to_string(),
        );
        return;
    }
    let seconds = span / c.value;
    bench.tran_stop = bench.tran_stop.max(2.5 * seconds);
    let id = bench.id();
    supplies(&mut bench.tran, &id, volts);
    let input = format!("{id}i");
    let (low, high) = (-span / 2.0, span / 2.0);
    bench.tran.push(format!(
        "V{id} {input} 0 PULSE({} {} {} {} {} {} {})",
        number(low),
        number(high),
        number(0.1 * seconds),
        number(1e-3 * seconds),
        number(1e-3 * seconds),
        number(10.0 * seconds),
        number(20.0 * seconds)
    ));
    let nodes = instance_nodes(&id, &input, &format!("{id}o"));
    let nodes: Vec<&str> = nodes.iter().map(String::as_str).collect();
    bench.tran.push(model.instance(&id, &nodes));
    model.checks.push(Check {
        characteristic: c.index,
        role: Role::Fit,
        expected: c.value,
        probe: Probe::Slope {
            node: format!("{id}o"),
            low: low + 0.1 * span,
            high: low + top * span,
        },
    });
}

/// A comparator's delay: its input stepped from well below the falling trip
/// to `overdrive` past the rising one.
fn check_delay(
    model: &mut Model,
    bench: &mut Bench,
    core: &Core,
    supply: f64,
    (overdrive, delay, index): (f64, f64, usize),
    role: Role,
) {
    let id = bench.id();
    supplies(&mut bench.tran, &id, supply);
    let trip = core.vos + core.hysteresis / 2.0;
    // The same hold as the swing checks, so the gain node starts on the
    // low clamp where the fit put it.
    let start = overdrive_for(core, -1.0);
    // Each step comes after the last fixture's delay has run out, so the
    // run's time step suits every delay.
    let at = bench.tran_stop.max(delay);
    bench.tran_stop = at + 4.0 * delay;
    let input = format!("{id}i");
    bench.tran.push(format!(
        "V{id} {input} 0 PULSE({} {} {} {} {} {} {})",
        number(start),
        number(trip + overdrive),
        number(at),
        number(1e-4 * delay),
        number(1e-4 * delay),
        number(1e3),
        number(2e3)
    ));
    let nodes = instance_nodes(&id, &input, "0");
    let nodes: Vec<&str> = nodes.iter().map(String::as_str).collect();
    bench.tran.push(model.instance(&id, &nodes));
    let middle = (core.low_level(supply) + core.high_level(supply)) / 2.0;
    model.checks.push(Check {
        characteristic: index,
        role,
        expected: delay,
        probe: Probe::Delay {
            input,
            input_level: trip,
            output: format!("{id}o"),
            output_level: middle,
            slow: false,
        },
    });
}

/// A comparator's trip points: a slow triangle across both, the output read
/// where it rises and where it falls.
fn check_trips(
    model: &mut Model,
    bench: &mut Bench,
    core: &Core,
    supply: f64,
    characteristics: &[Characteristic],
) {
    let hysteresis = of(characteristics, Quantity::Hysteresis).first().copied();
    let offset = of(characteristics, Quantity::InputOffsetVoltage)
        .first()
        .copied();
    if hysteresis.is_none() && offset.is_none() {
        return;
    }
    let volts = hysteresis.or(offset).map_or(supply, |c| supply_of(Some(c), supply));
    let reach = core.hysteresis + 0.01;
    // The ramp is slow enough that the output, a quarter of the way through
    // its delay at a small overdrive, lags the trip by far less than that
    // overdrive: the trip points then read high and low by about it.
    let lag = TRIP_LAG * core.hysteresis.max(5e-3);
    let delay = core.ca * core.delay_per_farad(lag, volts).unwrap_or(1.0);
    let rate = lag / (4.0 * delay);
    let period = 4.0 * reach / rate;
    bench.slow_stop = period;
    let id = bench.id();
    supplies(&mut bench.slow, &id, volts);
    let input = format!("{id}i");
    // A PULSE with no width never falls on libngspice 45.2, so a triangle
    // is a PWL.
    bench.slow.push(format!(
        "V{id} {input} 0 PWL(0.000000e0 {} {} {} {} {})",
        number(core.vos - reach),
        number(period / 2.0),
        number(core.vos + reach),
        number(period),
        number(core.vos - reach)
    ));
    let nodes = instance_nodes(&id, &input, "0");
    let nodes: Vec<&str> = nodes.iter().map(String::as_str).collect();
    bench.slow.push(model.instance(&id, &nodes));
    let middle = (core.low_level(volts) + core.high_level(volts)) / 2.0;
    for (c, what) in [(hysteresis, Trip::Width), (offset, Trip::Center)] {
        let Some(c) = c else { continue };
        model.checks.push(Check {
            characteristic: c.index,
            role: Role::Fit,
            expected: c.value,
            probe: Probe::Trip {
                input: input.clone(),
                output: format!("{id}o"),
                level: middle,
                what,
            },
        });
    }
}

/// `+ k * expression`, with the sign written as an operator.
fn term(k: f64, expression: &str) -> String {
    if k < 0.0 {
        format!("-{}*{expression}", number(-k))
    } else {
        format!("+{}*{expression}", number(k))
    }
}

fn body(name: &str, core: &Core, temperature: f64) -> String {
    let mut cards = format!(
        ".subckt {name} INP INN VCC VEE OUT\nEMID mid VEE VCC VEE 5.000000e-1\n"
    );
    if core.ib > 0.0 {
        cards.push_str(&format!(
            "IBP INP mid {}\nIBN INN mid {}\n",
            number(core.ib),
            number(core.ib)
        ));
    }
    if core.iq > 0.0 {
        cards.push_str(&format!("IQ VCC VEE {}\n", number(core.iq)));
    }
    let mut input = "v(INP,INN)".to_string();
    if core.vos > 0.0 {
        input.push_str(&format!("-{}", number(core.vos)));
    }
    if core.hysteresis > 0.0 {
        input.push_str(&term(
            core.hysteresis / 2.0,
            &format!("tanh(v(a,mid)/{})", number(HYSTERESIS_WIDTH)),
        ));
    }
    match core.soft {
        Some(soft) => {
            let size = format!(
                "sqrt(({input})*({input})+{})",
                number(SMOOTH_ABS * SMOOTH_ABS)
            );
            // `(size + e)^(1 - p) (size + soft)^p` as exponentials of logs,
            // whose arguments stay below 14, where ngspice's exp is exact.
            let denominator = if core.power < 1.0 {
                format!(
                    "exp({}*ln({size}+{})+{}*ln({size}+{}))",
                    number(1.0 - core.power),
                    number(SMOOTH_ABS),
                    number(core.power),
                    number(soft)
                )
            } else {
                format!("({size}+{})", number(soft))
            };
            cards.push_str(&format!(
                "BG mid a I={}*({input})/{denominator}\n",
                number(core.imax)
            ));
        }
        None => cards.push_str(&format!(
            "BG mid a I=max(min({}*({input}),{}),-{})\n",
            number(core.gm),
            number(core.imax),
            number(core.imax)
        )),
    }
    cards.push_str(&format!(
        "RA a mid {}\nCA a mid {}\n",
        number(GAIN_RESISTANCE),
        number(core.ca)
    ));
    cards.push_str(&format!(
        "EH hr mid VCC mid 1.000000e0\nVH hr ch {}\nDH a ch {name}_c\n",
        number(core.high_offset)
    ));
    cards.push_str(&format!(
        "EL lr mid VEE mid 1.000000e0\nVL cl lr {}\nDL cl a {name}_c\n",
        number(core.low_offset)
    ));
    cards.push_str("EO o mid a mid 1.000000e0\n");
    if core.rout > 0.0 {
        cards.push_str(&format!(
            "RO o x {}\nVS x OUT 0.000000e0\n",
            number(core.rout)
        ));
    } else {
        cards.push_str("VS o OUT 0.000000e0\n");
    }
    cards.push_str("HS hs VEE VS 1.000000e0\nBP VCC VEE I=max(v(hs,VEE),0.000000e0)\n");
    cards.push_str(&diode_card(
        &format!("{name}_c"),
        &[("IS", CLAMP_IS), ("N", CLAMP_N), ("TNOM", temperature)],
    ));
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

    fn op_amp_values() -> Vec<Value> {
        vec![
            value("open_loop_gain", 1e5, json!({})),
            value("open_loop_gain", 100.0, json!({ "frequency_hz": 1e4 })),
            value("gain_bandwidth_product", 1e6, json!({})),
            value("slew_rate", 0.5e6, json!({})),
            value("input_offset_voltage", 2e-3, json!({})),
            value("input_bias_current", 20e-9, json!({})),
            value("supply_current", 0.5e-3, json!({ "supply_voltage_volts": 10.0 })),
            value(
                "output_swing_from_positive_rail",
                0.2,
                json!({ "supply_voltage_volts": 10.0, "load_resistance_ohms": 2000.0 }),
            ),
            value(
                "output_swing_from_negative_rail",
                0.1,
                json!({ "supply_voltage_volts": 10.0, "load_resistance_ohms": 2000.0 }),
            ),
            value("output_resistance", 50.0, json!({})),
        ]
    }

    fn delay(overdrive: f64, seconds: f64) -> Value {
        value("propagation_delay", seconds, json!({ "overdrive_volts": overdrive }))
    }

    #[test]
    fn an_op_amp_is_a_five_pin_subcircuit_whose_pole_and_slew_follow_the_datasheet() {
        let model = build_model(Kind::OpAmp, "lm358_fit", &parsed(json!(op_amp_values())), 25.0)
            .unwrap();
        let checked = text::check_fitted(model.file().as_bytes()).unwrap();
        assert_eq!(checked.pins(), ["INP", "INN", "VCC", "VEE", "OUT"]);
        assert_eq!(checked.kind, "op_amp");
        // A0 = gm RA, GBW = gm / (2 pi CA), SR = imax / CA.
        let (gm, ca, imax) = (
            fitted(&model, "transconductance"),
            fitted(&model, "gain_capacitance"),
            fitted(&model, "stage_current"),
        );
        assert!((gm * GAIN_RESISTANCE - 1e5).abs() < 1e-6);
        assert!((gm / (std::f64::consts::TAU * ca) - 1e6).abs() < 1e-3);
        assert!((imax / ca - 0.5e6).abs() < 1e-3);
        // Every value is checked, and the Bode value is held out.
        assert_eq!(model.checks.len(), 10);
        let held: Vec<_> = model.checks.iter().filter(|c| c.role == Role::HeldOut).collect();
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].characteristic, 1);
        assert!(!model.bench.tran.is_empty() && model.bench.tran_stop > 0.0);
    }

    #[test]
    fn an_op_amp_needs_one_dc_gain_a_real_gain_and_a_swing_inside_its_supply() {
        let mut no_dc = op_amp_values();
        no_dc.remove(0);
        let error = build_model(Kind::OpAmp, "x_fit", &parsed(json!(no_dc)), 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics");
        let mut two = op_amp_values();
        two.push(value("open_loop_gain", 2e5, json!({})));
        assert!(build_model(Kind::OpAmp, "x_fit", &parsed(json!(two)), 25.0).is_err());
        let mut small = op_amp_values();
        small[0] = value("open_loop_gain", 50.0, json!({}));
        let error = build_model(Kind::OpAmp, "x_fit", &parsed(json!(small)), 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[0].value");
        let mut wide = op_amp_values();
        wide[7] = value(
            "output_swing_from_positive_rail",
            6.0,
            json!({ "supply_voltage_volts": 10.0, "load_resistance_ohms": 2000.0 }),
        );
        let error = build_model(Kind::OpAmp, "x_fit", &parsed(json!(wide)), 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[7].value");
        // A comparator's values are not an op-amp's.
        let mut delayed = op_amp_values();
        delayed.push(delay(0.01, 1e-6));
        let error = build_model(Kind::OpAmp, "x_fit", &parsed(json!(delayed)), 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[10].quantity");
    }

    #[test]
    fn without_values_the_model_says_what_it_assumed() {
        let bare = parsed(json!([
            value("open_loop_gain", 1e5, json!({})),
            value("gain_bandwidth_product", 1e6, json!({}))
        ]));
        let model = build_model(Kind::OpAmp, "bare_fit", &bare, 25.0).unwrap();
        for note in ["no slew rate", "no input offset", "no supply voltage", "reaches each rail"] {
            assert!(
                model.assumed.iter().chain(&model.fitted).any(|a| a.contains(note)),
                "{note}: {:?}",
                model.assumed
            );
        }
        text::check_fitted(model.file().as_bytes()).unwrap();
    }

    #[test]
    fn a_comparators_law_reproduces_its_delays_and_holds_one_out_from_four() {
        let three = parsed(json!([
            delay(5e-3, 1.3e-6),
            delay(20e-3, 0.6e-6),
            delay(100e-3, 0.3e-6),
            value("hysteresis", 5e-3, json!({})),
            value("output_swing_from_positive_rail", 0.2, json!({ "supply_voltage_volts": 5.0, "load_resistance_ohms": 1e4 }))
        ]));
        let model = build_model(Kind::Comparator, "lm393_fit", &three, 25.0).unwrap();
        text::check_fitted(model.file().as_bytes()).unwrap();
        assert!(model.checks.iter().all(|c| c.role != Role::HeldOut));
        // The law the fit integrated gives every delay back.
        let core = Core {
            gm: fitted(&model, "transconductance"),
            ca: fitted(&model, "gain_capacitance"),
            imax: fitted(&model, "stage_current"),
            vos: 0.0,
            ib: 0.0,
            iq: 0.0,
            rout: 0.0,
            high_offset: fitted(&model, "high_clamp_offset"),
            low_offset: fitted(&model, "low_clamp_offset"),
            hysteresis: 5e-3,
            soft: Some(fitted(&model, "stage_turnover")),
            power: fitted(&model, "stage_power"),
            vt: thermal_voltage(25.0),
        };
        for (overdrive, seconds) in [(5e-3, 1.3e-6), (20e-3, 0.6e-6), (100e-3, 0.3e-6)] {
            let reading = core.ca * core.delay_per_farad(overdrive, 5.0).unwrap();
            assert!((reading / seconds - 1.0).abs() < 2e-3, "{overdrive}: {reading}");
        }
        let four = parsed(json!([
            delay(5e-3, 1.3e-6),
            delay(10e-3, 0.9e-6),
            delay(20e-3, 0.6e-6),
            delay(100e-3, 0.3e-6)
        ]));
        let model = build_model(Kind::Comparator, "lm393_fit", &four, 25.0).unwrap();
        let held: Vec<_> = model.checks.iter().filter(|c| c.role == Role::HeldOut).collect();
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].characteristic, 2);
    }

    #[test]
    fn a_comparator_refuses_two_delays_at_one_overdrive_and_takes_no_gain() {
        let twice = parsed(json!([delay(5e-3, 1.3e-6), delay(5e-3, 1.0e-6)]));
        let error = build_model(Kind::Comparator, "x_fit", &twice, 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[1].conditions.overdrive_volts");
        let gain = parsed(json!([delay(5e-3, 1.3e-6), value("open_loop_gain", 2e5, json!({}))]));
        let error = build_model(Kind::Comparator, "x_fit", &gain, 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[1].quantity");
        let one = parsed(json!([delay(5e-3, 1.3e-6)]));
        let model = build_model(Kind::Comparator, "x_fit", &one, 25.0).unwrap();
        assert!(model.assumed.iter().any(|a| a.contains("half turned over")));
    }
}
