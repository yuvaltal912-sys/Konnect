//! Bipolar transistors, on ngspice's Gummel-Poon card inside a three-pin
//! subcircuit.
//!
//! With no Early voltage, no base or emitter resistance and `NR = 1`, the
//! terminal currents are, from the junction voltages:
//! `IF = IS (e^(Vbe / NF Vt) - 1)`, `IR = IS (e^(Vbc / Vt) - 1)`,
//! `qb = (1 + sqrt(1 + 4 IF / IKF)) / 2`,
//! `Ic = (IF - IR) / qb - IR / BR` and
//! `Ib = IF / BF + ISE (e^(Vbe / NE Vt) - 1) + IR / BR`.
//! A scratch run of libngspice 45.2 agreed with these to 0.05 percent, the
//! difference being the rounding of the thermal voltage, so the fit runs on the
//! law itself and the bench then checks what the engine reads.
//!
//! Every value is a magnitude: a PNP is the same fit on a `PNP` card, with
//! bench sources of the opposite sign.

use super::bench::{Bench, Check, Probe, Role};
use super::diode::{fit_capacitance, hold_out};
use super::fit::{least_squares, root, thermal_voltage};
use super::text::bjt_card;
use super::{
    of, Characteristic, Element, Fitted, Invalid, Kind, Model, Quantity, BASE_CURRENT,
    COLLECTOR_CURRENT, COLLECTOR_EMITTER_VOLTAGE, FREQUENCY, REVERSE_VOLTAGE,
};

/// The low-current leakage's emission coefficient, which a datasheet never
/// states.
const NE: f64 = 1.5;
/// A Vbe error of this many volts counts as one unit in the fit.
const VBE_SCALE: f64 = 0.02;
const JUNCTION_POTENTIAL: f64 = 0.75;
/// ngspice's `FC`, where the depletion capacitance turns linear.
const FC: f64 = 0.5;
const BR_RANGE: (f64, f64) = (0.05, 200.0);
const DEFAULT_BR: f64 = 1.0;
/// A transition frequency is read at a third of itself, unless the caller
/// names a test frequency: there the gain is about 3 and the pole is far below.
const FT_SHARE: f64 = 3.0;

/// The parameters of the DC law.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Law {
    pub is: f64,
    pub nf: f64,
    pub bf: f64,
    pub ikf: f64,
    pub ise: f64,
    pub br: f64,
    pub rc: f64,
    pub vt: f64,
}

impl Law {
    /// `(Ic, Ib)` for junction voltages.
    pub(crate) fn currents(&self, vbe: f64, vbc: f64) -> (f64, f64) {
        let forward = self.is * (vbe / (self.nf * self.vt)).exp_m1();
        let reverse = self.is * (vbc / self.vt).exp_m1();
        let qb = 0.5 * (1.0 + (1.0 + 4.0 * forward / self.ikf).max(0.0).sqrt());
        let leakage = self.ise * (vbe / (NE * self.vt)).exp_m1();
        let ic = (forward - reverse) / qb - reverse / self.br;
        let ib = forward / self.bf + leakage + reverse / self.br;
        (ic, ib)
    }

    /// The base-emitter voltage at which `ic` flows with `vce` across the
    /// collector and emitter.
    pub(crate) fn vbe_for(&self, ic: f64, vce: f64) -> Option<f64> {
        root(|vbe| self.currents(vbe, vbe - vce).0 - ic, 0.05, 1.3)
    }

    /// `Ic / Ib` at `ic` and `vce`.
    pub(crate) fn gain(&self, ic: f64, vce: f64) -> Option<f64> {
        let vbe = self.vbe_for(ic, vce)?;
        let (collector, base) = self.currents(vbe, vbe - vce);
        (base > 0.0).then_some(collector / base)
    }

    /// The collector-emitter voltage with `ib` forced into the base and `ic`
    /// into the collector, which is the saturation voltage when the base is
    /// overdriven.
    pub(crate) fn saturation(&self, ic: f64, ib: f64) -> Option<f64> {
        let found = least_squares(
            |x| {
                let (collector, base) = self.currents(x[0], x[1]);
                vec![(collector - ic) / ic, (base - ib) / ib]
            },
            &[0.78, 0.7],
            |x| {
                x[0] = x[0].clamp(0.0, 1.3);
                x[1] = x[1].clamp(-1.0, 1.3);
            },
        );
        let (collector, base) = self.currents(found[0], found[1]);
        let settled = ((collector - ic) / ic).abs() < 1e-6 && ((base - ib) / ib).abs() < 1e-6;
        settled.then_some(found[0] - found[1] + ic * self.rc)
    }
}

/// ngspice's depletion capacitance at junction voltage `v` (forward positive).
fn depletion(c0: f64, vj: f64, m: f64, v: f64) -> f64 {
    if v < FC * vj {
        c0 / (1.0 - v / vj).powf(m)
    } else {
        c0 / (1.0 - FC).powf(1.0 + m) * (1.0 - FC * (1.0 + m) + m * v / vj)
    }
}

#[derive(Debug, Clone, Copy)]
struct GainPoint {
    ic: f64,
    vce: f64,
    gain: f64,
}

#[derive(Debug, Clone, Copy)]
struct VbePoint {
    ic: f64,
    vce: f64,
    vbe: f64,
}

fn law_from(x: &[f64], vt: f64) -> Law {
    Law {
        is: x[0].exp(),
        nf: x[1],
        bf: x[2].exp(),
        ikf: x[3].exp(),
        ise: x[4].exp(),
        br: DEFAULT_BR,
        rc: 0.0,
        vt,
    }
}

fn residuals(law: &Law, gains: &[GainPoint], vbes: &[VbePoint]) -> Vec<f64> {
    let mut out: Vec<f64> = vbes
        .iter()
        .map(|p| {
            law.vbe_for(p.ic, p.vce)
                .map_or(1e3, |vbe| (vbe - p.vbe) / VBE_SCALE)
        })
        .collect();
    out.extend(gains.iter().map(|p| {
        law.gain(p.ic, p.vce)
            .map_or(10.0, |gain| (gain / p.gain).ln())
    }));
    out
}

pub(crate) fn build(
    kind: Kind,
    name: &str,
    characteristics: &[Characteristic],
    temperature: f64,
) -> Fitted<Model> {
    let pnp = kind == Kind::Pnp;
    let sign = if pnp { -1.0 } else { 1.0 };
    let vt = thermal_voltage(temperature);
    let mut model = Model::new(kind, name, Element::Subcircuit, vec!["C", "B", "E"]);

    let mut gains: Vec<(GainPoint, usize)> = of(characteristics, Quantity::CurrentGain)
        .into_iter()
        .map(|c| {
            (
                GainPoint {
                    ic: c.condition(COLLECTOR_CURRENT).unwrap_or_default(),
                    vce: c.condition(COLLECTOR_EMITTER_VOLTAGE).unwrap_or_default(),
                    gain: c.value,
                },
                c.index,
            )
        })
        .collect();
    gains.sort_by(|a, b| a.0.ic.total_cmp(&b.0.ic));
    if gains
        .windows(2)
        .any(|w| w[0].0.ic == w[1].0.ic && w[0].0.vce == w[1].0.vce)
    {
        return Err(Invalid::new(
            "characteristics",
            "two current_gain values share one collector current and voltage",
        ));
    }
    let mut vbes: Vec<(VbePoint, usize)> = of(characteristics, Quantity::BaseEmitterVoltage)
        .into_iter()
        .map(|c| {
            (
                VbePoint {
                    ic: c.condition(COLLECTOR_CURRENT).unwrap_or_default(),
                    vce: c.condition(COLLECTOR_EMITTER_VOLTAGE).unwrap_or_default(),
                    vbe: c.value,
                },
                c.index,
            )
        })
        .collect();
    vbes.sort_by(|a, b| a.0.ic.total_cmp(&b.0.ic));
    let (fit_gains, held) = hold_out(&gains);
    let fit_gain_points: Vec<GainPoint> = fit_gains.iter().map(|(p, _)| *p).collect();
    let vbe_points: Vec<VbePoint> = vbes.iter().map(|(p, _)| *p).collect();

    // IS from the first Vbe point as if NF were 1; the rest by least squares.
    let first = vbe_points[0];
    let is0 = (first.ic / (first.vbe / vt).exp()).clamp(1e-30, 1e-6);
    let top_gain = gains.iter().map(|(p, _)| p.gain).fold(0.0, f64::max);
    let top_current = gains
        .iter()
        .map(|(p, _)| p.ic)
        .chain(vbe_points.iter().map(|p| p.ic))
        .fold(0.0, f64::max);
    let free_nf = vbe_points.len() >= 2;
    let found = least_squares(
        |x| residuals(&law_from(x, vt), &fit_gain_points, &vbe_points),
        &[
            is0.ln(),
            1.0,
            (top_gain * 1.3).ln(),
            (10.0 * top_current).ln(),
            (50.0 * is0).ln(),
        ],
        |x| {
            x[0] = x[0].clamp(-60.0, -10.0);
            x[1] = if free_nf { x[1].clamp(0.8, 2.0) } else { 1.0 };
            x[2] = x[2].clamp(5.0_f64.ln(), 3000.0_f64.ln());
            x[3] = x[3].clamp(1e-4_f64.ln(), 1e3_f64.ln());
            x[4] = x[4].clamp(-70.0, -20.0);
        },
    );
    let mut law = law_from(&found, vt);
    if !(law.is.is_finite() && law.bf.is_finite() && law.ikf.is_finite() && law.ise.is_finite()) {
        return Err(Invalid::new(
            "characteristics",
            "the values do not fit the bipolar transistor law",
        ));
    }
    model
        .fitted
        .push("is, nf, bf, ikf and ise from the gain curve and the base-emitter voltages".into());
    if !free_nf {
        model.assumed.push(
            "nf = 1: one base-emitter voltage fixes is, not the slope; give two for nf".into(),
        );
    }
    let worst = residuals(&law, &fit_gain_points, &[])
        .into_iter()
        .fold(0.0_f64, |w, r| w.max(r.abs()));
    if worst > 0.25 {
        model.assumed.push(format!(
            "the gain curve follows the transistor law only to within a factor of {:.2}",
            worst.exp()
        ));
    }

    // BR and RC from the saturation voltage.
    let saturation = of(characteristics, Quantity::CollectorEmitterSaturationVoltage)
        .first()
        .copied();
    if let Some(c) = saturation {
        let ic = c.condition(COLLECTOR_CURRENT).unwrap_or_default();
        let ib = c.condition(BASE_CURRENT).unwrap_or_default();
        let at = |br: f64, rc: f64| Law { br, rc, ..law }.saturation(ic, ib);
        match (at(BR_RANGE.0, 0.0), at(BR_RANGE.1, 0.0)) {
            (Some(high), Some(low)) if c.value >= low && c.value <= high => {
                let ln_br = root(
                    |l| at(l.exp(), 0.0).map_or(f64::NAN, |v| v - c.value),
                    BR_RANGE.0.ln(),
                    BR_RANGE.1.ln(),
                );
                law.br = ln_br.map_or(DEFAULT_BR, f64::exp);
                model.fitted.push("br from the saturation voltage".into());
            }
            (Some(high), _) if c.value > high => {
                law.br = BR_RANGE.0;
                law.rc = (c.value - high) / ic;
                model
                    .fitted
                    .push("rc from the saturation voltage, with br at its floor".into());
                model.assumed.push(format!(
                    "the saturation voltage is above what reverse injection alone gives, so br \
                     is {} and the rest is collector resistance",
                    BR_RANGE.0
                ));
            }
            (_, Some(low)) if c.value < low => {
                law.br = BR_RANGE.1;
                model.assumed.push(
                    "the saturation voltage is below what the model reaches: br is at its ceiling \
                     and the model's saturation voltage reads high"
                        .into(),
                );
            }
            _ => {
                law.br = DEFAULT_BR;
                model.assumed.push(
                    "the saturation values cannot be solved on this law: br = 1, rc = 0".into(),
                );
            }
        }
    } else {
        model
            .assumed
            .push("no saturation voltage: br = 1, rc = 0, ngspice's default".into());
    }

    let mut parameters: Vec<(&'static str, f64)> = vec![
        ("IS", law.is),
        ("NF", law.nf),
        ("BF", law.bf),
        ("IKF", law.ikf),
        ("ISE", law.ise),
        ("NE", NE),
        ("BR", law.br),
    ];
    if law.rc > 0.0 {
        parameters.push(("RC", law.rc));
    }

    // Junction capacitances, by the same law as a diode's.
    let junction = |model: &mut Model,
                    quantity: Quantity,
                    keys: [&'static str; 3]|
     -> Fitted<Option<(f64, f64, f64)>> {
        let mut points: Vec<(f64, f64)> = of(characteristics, quantity)
            .into_iter()
            .map(|c| (c.condition(REVERSE_VOLTAGE).unwrap_or_default(), c.value))
            .collect();
        if points.is_empty() {
            return Ok(None);
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        let field = of(characteristics, quantity)[0].field("value");
        let (c0, vj, m, assumed) = fit_capacitance(&points, JUNCTION_POTENTIAL, &field)?;
        model.assumed.extend(assumed);
        model.fitted.push(format!(
            "{} from {}",
            keys.join(", ").to_lowercase(),
            quantity.name()
        ));
        Ok(Some((c0, vj, m)))
    };
    let cjc = junction(
        &mut model,
        Quantity::CollectorBaseCapacitance,
        ["CJC", "VJC", "MJC"],
    )?;
    let cje = junction(
        &mut model,
        Quantity::EmitterBaseCapacitance,
        ["CJE", "VJE", "MJE"],
    )?;
    if let Some((c0, vj, m)) = cjc {
        parameters.extend([("CJC", c0), ("VJC", vj), ("MJC", m)]);
    }
    if let Some((c0, vj, m)) = cje {
        parameters.extend([("CJE", c0), ("VJE", vj), ("MJE", m)]);
    }
    if cjc.is_none() && cje.is_none() {
        model.assumed.push(
            "no junction capacitance values: the junctions have none, so the transistor is \
             as fast as its transit time"
                .into(),
        );
    }

    // TF from the transition frequency at its bias.
    let mut transition = None;
    if let Some(c) = of(characteristics, Quantity::TransitionFrequency)
        .first()
        .copied()
    {
        let ic = c.condition(COLLECTOR_CURRENT).unwrap_or_default();
        let vce = c.condition(COLLECTOR_EMITTER_VOLTAGE).unwrap_or_default();
        let Some(vbe) = law.vbe_for(ic, vce) else {
            return Err(Invalid::new(
                c.field("conditions.collector_current_amps"),
                "the fitted transistor cannot carry this collector current",
            ));
        };
        let step = 1e-4;
        let gm = (law.currents(vbe + step, vbe + step - vce).0
            - law.currents(vbe - step, vbe - step - vce).0)
            / (2.0 * step);
        let c_je = cje.map_or(0.0, |(c0, vj, m)| depletion(c0, vj, m, vbe));
        let c_jc = cjc.map_or(0.0, |(c0, vj, m)| depletion(c0, vj, m, vbe - vce));
        let tf = 1.0 / (std::f64::consts::TAU * c.value) - (c_je + c_jc) / gm;
        if tf > 0.0 {
            parameters.push(("TF", tf));
            model
                .fitted
                .push("tf from the transition frequency, less the junction capacitances".into());
        } else {
            model.assumed.push(
                "the junction capacitances alone give a transition frequency below the stated \
                 one, so tf is zero and the model reads slow"
                    .into(),
            );
        }
        transition = Some((c, ic, vce));
    }
    parameters.push(("TNOM", temperature));
    model.assumed.push(
        "no Early voltage, no base or emitter resistance and no high-current beta roll-off beyond \
         ikf: the collector current does not rise with collector voltage"
            .into(),
    );

    let inner = format!("{name}_q");
    model.body = format!(
        ".subckt {name} C B E\nQ1 C B E {inner}\n{}.ends {name}\n",
        bjt_card(&inner, pnp, &parameters)
    );
    model.parameters = parameters
        .iter()
        .map(|(key, value)| (format!("bjt.{key}"), *value))
        .collect();

    let mut bench = Bench::default();
    for (point, index) in &fit_gains {
        check_gain(&mut model, &mut bench, *point, *index, Role::Fit, sign);
    }
    if let Some((point, index)) = held {
        check_gain(&mut model, &mut bench, point, index, Role::HeldOut, sign);
    }
    for (point, index) in &vbes {
        let gain = law.gain(point.ic, point.vce).unwrap_or(100.0);
        check_vbe(&mut model, &mut bench, *point, gain, *index, sign);
    }
    if let Some(c) = saturation {
        check_saturation(&mut model, &mut bench, c, sign);
    }
    for c in of(characteristics, Quantity::CollectorBaseCapacitance) {
        check_junction(&mut model, &mut bench, c, true, sign);
    }
    for c in of(characteristics, Quantity::EmitterBaseCapacitance) {
        check_junction(&mut model, &mut bench, c, false, sign);
    }
    if let Some((c, ic, vce)) = transition {
        let gain = law.gain(ic, vce).unwrap_or(100.0);
        check_transition(&mut model, &mut bench, c, ic, vce, gain, sign);
    }
    model.bench = bench;
    Ok(model)
}

/// The gain at a bias: a base current of `Ic / hFE` is forced and the
/// collector current read, so `Ic_read / Ib` is the gain at that base current.
fn check_gain(
    model: &mut Model,
    bench: &mut Bench,
    point: GainPoint,
    index: usize,
    role: Role,
    sign: f64,
) {
    let id = bench.id();
    let (collector, base) = (format!("{id}c"), format!("{id}b"));
    let ib = point.ic / point.gain;
    bench
        .dc
        .push(Bench::voltage(&id, &collector, sign * point.vce, false));
    bench.dc.push(Bench::current_into(&id, &base, sign * ib));
    bench
        .dc
        .push(model.instance(&id, &[&collector, &base, "0"]));
    model.checks.push(Check {
        characteristic: index,
        role,
        expected: point.gain,
        probe: Probe::Current {
            source: format!("V{id}"),
            scale: -sign / ib,
        },
    });
}

/// Vbe at a collector current: the base is grounded and the emitter current
/// forced, so the emitter node sits at `-Vbe`. The collector is set so that
/// the collector-emitter voltage is the stated one.
fn check_vbe(
    model: &mut Model,
    bench: &mut Bench,
    point: VbePoint,
    gain: f64,
    index: usize,
    sign: f64,
) {
    let id = bench.id();
    let (collector, emitter) = (format!("{id}c"), format!("{id}e"));
    let ie = point.ic * (1.0 + 1.0 / gain);
    bench.dc.push(Bench::voltage(
        &id,
        &collector,
        sign * (point.vce - point.vbe),
        false,
    ));
    bench
        .dc
        .push(Bench::current_into(&format!("{id}e"), &emitter, -sign * ie));
    bench
        .dc
        .push(model.instance(&id, &[&collector, "0", &emitter]));
    model.checks.push(Check {
        characteristic: index,
        role: Role::Fit,
        expected: point.vbe,
        probe: Probe::Voltage {
            node: emitter,
            scale: -sign,
            divide_by: 1.0,
        },
    });
}

/// The collector voltage with both currents forced.
fn check_saturation(model: &mut Model, bench: &mut Bench, c: &Characteristic, sign: f64) {
    let (collector_id, base_id) = (bench.id(), bench.id());
    let (collector, base) = (format!("{collector_id}c"), format!("{base_id}b"));
    let ic = c.condition(COLLECTOR_CURRENT).unwrap_or_default();
    let ib = c.condition(BASE_CURRENT).unwrap_or_default();
    bench
        .dc
        .push(Bench::current_into(&collector_id, &collector, sign * ic));
    bench
        .dc
        .push(Bench::current_into(&base_id, &base, sign * ib));
    bench
        .dc
        .push(model.instance(&collector_id, &[&collector, &base, "0"]));
    model.checks.push(Check {
        characteristic: c.index,
        role: Role::Fit,
        expected: c.value,
        probe: Probe::Voltage {
            node: collector,
            scale: sign,
            divide_by: 1.0,
        },
    });
}

/// Cobo (`collector` true) or Cibo: a reverse bias with an AC drive on the
/// biased pin, the other junction's far pin left open through 1 Gohm.
fn check_junction(
    model: &mut Model,
    bench: &mut Bench,
    c: &Characteristic,
    collector_side: bool,
    sign: f64,
) {
    let id = bench.id();
    let (collector, base, emitter) = (format!("{id}c"), format!("{id}b"), format!("{id}e"));
    let reverse = c.condition(REVERSE_VOLTAGE).unwrap_or_default();
    let (driven, open) = if collector_side {
        (&collector, &emitter)
    } else {
        (&emitter, &collector)
    };
    bench
        .ac
        .push(Bench::voltage(&id, driven, sign * reverse, true));
    bench
        .ac
        .push(Bench::voltage(&format!("{id}b"), &base, 0.0, false));
    bench.ac.push(format!("R{id}o {open} 0 1.000000e9"));
    bench
        .ac
        .push(model.instance(&id, &[&collector, &base, &emitter]));
    model.checks.push(Check {
        characteristic: c.index,
        role: Role::Fit,
        expected: c.value,
        probe: Probe::Capacitance {
            source: format!("V{id}"),
            hz: c.condition(FREQUENCY).unwrap_or(1e6),
        },
    });
}

/// fT: a unit AC base current and the collector at AC ground, read as the
/// magnitude of the collector current times the test frequency.
fn check_transition(
    model: &mut Model,
    bench: &mut Bench,
    c: &Characteristic,
    ic: f64,
    vce: f64,
    gain: f64,
    sign: f64,
) {
    let id = bench.id();
    let (collector, base) = (format!("{id}c"), format!("{id}b"));
    let hz = c.condition(FREQUENCY).unwrap_or(c.value / FT_SHARE);
    bench
        .ac
        .push(Bench::voltage(&id, &collector, sign * vce, false));
    bench.ac.push(Bench::ac_current_into(
        &format!("{id}b"),
        &base,
        sign * ic / gain,
    ));
    bench
        .ac
        .push(model.instance(&id, &[&collector, &base, "0"]));
    model.checks.push(Check {
        characteristic: c.index,
        role: Role::Fit,
        expected: c.value,
        probe: Probe::CurrentMagnitude {
            source: format!("V{id}"),
            scale: hz,
            hz,
        },
    });
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::{build as build_model, text};
    use super::*;
    use serde_json::{json, Value};

    fn law() -> Law {
        Law {
            is: 1e-14,
            nf: 1.0,
            bf: 200.0,
            ikf: 0.5,
            ise: 2e-13,
            br: 2.0,
            rc: 0.0,
            vt: thermal_voltage(25.0),
        }
    }

    /// What libngspice 45.2 read for `IS=1e-14 NF=1 BF=200 IKF=0.5 ISE=2e-13
    /// NE=1.5 BR=2` at 25 C: `(Vbe, Vce, Ic, Ib)`.
    const ENGINE: [(f64, f64, f64, f64); 4] = [
        (0.65, 5.0, 9.69252e-4, 9.08119e-6),
        (0.78, 5.0, 1.22837e-1, 8.88343e-4),
        (0.8, 0.1, 2.20619e-1, 5.27307e-3),
        (0.75, 0.05, 3.41192e-2, 3.69417e-3),
    ];

    #[test]
    fn the_law_reads_what_ngspice_read() {
        for (vbe, vce, ic, ib) in ENGINE {
            let (pc, pb) = law().currents(vbe, vbe - vce);
            assert!(
                (pc - ic).abs() <= 2e-4 * ic,
                "ic at {vbe}: {pc} against {ic}"
            );
            assert!(
                (pb - ib).abs() <= 2e-4 * ib,
                "ib at {vbe}: {pb} against {ib}"
            );
        }
    }

    #[test]
    fn the_inverse_laws_return_their_inputs() {
        let law = law();
        let vbe = law.vbe_for(1e-2, 5.0).unwrap();
        let (ic, _) = law.currents(vbe, vbe - 5.0);
        assert!((ic - 1e-2).abs() < 1e-8, "{ic}");
        // Saturation: the pair of currents lands on the junction voltages that
        // make them, and a larger BR lowers the voltage.
        let low = Law { br: 0.1, ..law }.saturation(0.1, 0.02).unwrap();
        let high = Law { br: 50.0, ..law }.saturation(0.1, 0.02).unwrap();
        assert!(low > high, "{low} vs {high}");
        // A collector resistance adds its drop.
        let with_rc = Law { rc: 0.5, ..law }.saturation(0.1, 0.02).unwrap();
        let without = law.saturation(0.1, 0.02).unwrap();
        assert!((with_rc - without - 0.05).abs() < 1e-6);
    }

    fn gain_point(ic: f64) -> Value {
        let law = law();
        json!({
            "quantity": "current_gain", "value": law.gain(ic, 5.0).unwrap(),
            "conditions": { "collector_current_amps": ic, "collector_emitter_voltage_volts": 5.0 },
            "statistic": "typical"
        })
    }

    fn vbe_point(ic: f64) -> Value {
        json!({
            "quantity": "base_emitter_voltage", "value": law().vbe_for(ic, 5.0).unwrap(),
            "conditions": { "collector_current_amps": ic, "collector_emitter_voltage_volts": 5.0 },
            "statistic": "typical"
        })
    }

    fn datasheet(extra: Vec<Value>) -> Vec<Characteristic> {
        let mut values = vec![
            gain_point(1e-4),
            gain_point(1e-3),
            gain_point(1e-2),
            gain_point(1e-1),
            vbe_point(1e-3),
            vbe_point(1e-1),
        ];
        values.extend(extra);
        parsed(Value::Array(values))
    }

    #[test]
    fn the_fit_recovers_the_law_that_made_the_values() {
        let model = build_model(Kind::Npn, "bc547_fit", &datasheet(vec![]), 25.0).unwrap();
        // Every value the datasheet states is reproduced by the fitted law.
        let fitted_law = Law {
            is: fitted(&model, "bjt.IS"),
            nf: fitted(&model, "bjt.NF"),
            bf: fitted(&model, "bjt.BF"),
            ikf: fitted(&model, "bjt.IKF"),
            ise: fitted(&model, "bjt.ISE"),
            br: fitted(&model, "bjt.BR"),
            rc: 0.0,
            vt: thermal_voltage(25.0),
        };
        for ic in [1e-4, 1e-3, 1e-2, 1e-1] {
            let (want, got) = (
                law().gain(ic, 5.0).unwrap(),
                fitted_law.gain(ic, 5.0).unwrap(),
            );
            assert!(
                (got - want).abs() <= 0.02 * want,
                "gain at {ic}: {got} against {want}"
            );
        }
        for ic in [1e-3, 1e-1] {
            let (want, got) = (
                law().vbe_for(ic, 5.0).unwrap(),
                fitted_law.vbe_for(ic, 5.0).unwrap(),
            );
            assert!(
                (got - want).abs() < 5e-3,
                "vbe at {ic}: {got} against {want}"
            );
        }
    }

    #[test]
    fn a_curve_longer_than_the_law_holds_a_value_out_and_two_vbe_values_free_nf() {
        let model = build_model(Kind::Npn, "bc547_fit", &datasheet(vec![]), 25.0).unwrap();
        assert_eq!(
            model
                .checks
                .iter()
                .filter(|c| c.role == Role::HeldOut)
                .count(),
            1
        );
        assert!(!model.assumed.iter().any(|note| note.contains("nf = 1")));
        let one = parsed(json!([
            gain_point(1e-4),
            gain_point(1e-3),
            gain_point(1e-2),
            vbe_point(1e-3)
        ]));
        let model = build_model(Kind::Npn, "bc547_fit", &one, 25.0).unwrap();
        assert!(model.assumed.iter().any(|note| note.contains("nf = 1")));
        assert_eq!(fitted(&model, "bjt.NF"), 1.0);
    }

    #[test]
    fn saturation_voltage_fits_br_then_rc() {
        let base = law();
        let vce = |br: f64| Law { br, ..base }.saturation(0.1, 0.01).unwrap();
        let sat = |v: f64| {
            json!({
                "quantity": "collector_emitter_saturation_voltage", "value": v,
                "conditions": { "collector_current_amps": 0.1, "base_current_amps": 0.01 },
                "statistic": "maximum"
            })
        };
        // A value inside the BR range fits BR alone.
        let inside = build_model(
            Kind::Npn,
            "bc547_fit",
            &datasheet(vec![sat(vce(3.0))]),
            25.0,
        )
        .unwrap();
        assert!(
            (fitted(&inside, "bjt.BR") - 3.0).abs() < 0.2,
            "{}",
            fitted(&inside, "bjt.BR")
        );
        assert!(!inside.parameters.iter().any(|(k, _)| k == "bjt.RC"));
        // A value above the floor takes the rest as collector resistance.
        let above = build_model(Kind::Npn, "bc547_fit", &datasheet(vec![sat(0.9)]), 25.0).unwrap();
        assert_eq!(fitted(&above, "bjt.BR"), BR_RANGE.0);
        assert!(fitted(&above, "bjt.RC") > 0.0);
    }

    #[test]
    fn the_file_is_the_three_pin_subcircuit_in_the_fitted_grammar_for_both_polarities() {
        let npn = build_model(Kind::Npn, "bc547_fit", &datasheet(vec![]), 25.0).unwrap();
        let pnp = build_model(Kind::Pnp, "bc557_fit", &datasheet(vec![]), 25.0).unwrap();
        assert!(npn
            .body
            .starts_with(".subckt bc547_fit C B E\nQ1 C B E bc547_fit_q\n"));
        assert!(npn.body.contains(" NPN\n") && pnp.body.contains(" PNP\n"));
        let checked = text::check_fitted(pnp.file().as_bytes()).unwrap();
        assert_eq!(checked.pins(), ["C", "B", "E"]);
        assert_eq!(checked.kind, "pnp");
        // The collector supply of the PNP's gain bench points the other way.
        assert!(pnp
            .bench
            .dc
            .iter()
            .any(|line| line.starts_with("Vb1 ") && line.contains("DC -5")));
        assert!(npn
            .bench
            .dc
            .iter()
            .any(|line| line.starts_with("Vb1 ") && line.contains("DC 5")));
    }

    #[test]
    fn junction_capacitances_and_a_transition_frequency_add_their_cards_and_checks() {
        let cap = |quantity: &str, v: f64, f: f64| {
            json!({
                "quantity": quantity, "value": f,
                "conditions": { "reverse_voltage_volts": v }, "statistic": "typical"
            })
        };
        let extra = vec![
            cap("collector_base_capacitance", 1.0, 4e-12),
            cap("collector_base_capacitance", 10.0, 2e-12),
            cap("emitter_base_capacitance", 0.5, 12e-12),
            json!({
                "quantity": "transition_frequency", "value": 3e8,
                "conditions": { "collector_current_amps": 0.01, "collector_emitter_voltage_volts": 5.0 },
                "statistic": "typical"
            }),
        ];
        let model = build_model(Kind::Npn, "bc547_fit", &datasheet(extra), 25.0).unwrap();
        for key in ["CJC", "VJC", "MJC", "CJE", "VJE", "MJE", "TF"] {
            fitted(&model, &format!("bjt.{key}"));
        }
        assert!(text::check_fitted(model.file().as_bytes()).is_ok());
        assert!(model
            .checks
            .iter()
            .any(|c| matches!(c.probe, Probe::CurrentMagnitude { .. })));
    }

    #[test]
    fn duplicate_bias_points_are_refused() {
        let error = build_model(
            Kind::Npn,
            "bc547_fit",
            &datasheet(vec![gain_point(1e-3)]),
            25.0,
        )
        .unwrap_err();
        assert_eq!(error.field, "characteristics");
    }
}
