//! Resistances, capacitors and inductors with the parasitics a datasheet gives.
//!
//! KiCad cannot read a library `.model` as R, C or L, so each is a subcircuit
//! with ports 1 and 2. A 0 V source sits at port 1, so the part's current can
//! be read as a plain part's is. Every fit is exact: the impedance is worked
//! out here, and each parameter is solved so that the value the caller gave
//! comes back at its own test frequency.

use super::bench::{Bench, Check, Probe, Role};
use super::fit;
use super::text::number;
use super::{
    of, Characteristic, Element, Fitted, Invalid, Kind, Model, Quantity, FREQUENCY, VOLTAGE,
};
use std::f64::consts::TAU;

const CAPACITANCE_HZ: f64 = 1e3;
const ESR_HZ: f64 = 1e5;
const INDUCTANCE_HZ: f64 = 1e5;
const SERIES_INDUCTANCE_HZ: f64 = 1e6;
/// A series inductance is read this many times above the resonance it makes,
/// where the capacitance's share of the reactance is a millionth.
const ABOVE_RESONANCE: f64 = 1e3;
/// The test current of a DC resistance check.
const TEST_AMPS: f64 = 1.0;

/// A complex number, enough of one for an impedance.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Z {
    re: f64,
    im: f64,
}

impl Z {
    fn add(self, other: Z) -> Z {
        Z {
            re: self.re + other.re,
            im: self.im + other.im,
        }
    }

    fn mul(self, other: Z) -> Z {
        Z {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }

    fn inverse(self) -> Z {
        let size = self.re * self.re + self.im * self.im;
        Z {
            re: self.re / size,
            im: -self.im / size,
        }
    }

    /// Two impedances in parallel.
    fn parallel(self, other: Z) -> Z {
        self.mul(other).mul(self.add(other).inverse())
    }
}

fn resistor(ohm: f64) -> Z {
    Z { re: ohm, im: 0.0 }
}

fn inductor(henry: f64, omega: f64) -> Z {
    Z {
        re: 0.0,
        im: omega * henry,
    }
}

fn capacitor(farad: f64, omega: f64) -> Z {
    Z {
        re: 0.0,
        im: -1.0 / (omega * farad),
    }
}

fn frequency(characteristic: &Characteristic, default: f64) -> f64 {
    characteristic.condition(FREQUENCY).unwrap_or(default)
}

fn one(characteristics: &[Characteristic], quantity: Quantity) -> Option<&Characteristic> {
    of(characteristics, quantity).first().copied()
}

/// The subcircuit text: a sense source on port 1, then the lines given.
fn subcircuit(name: &str, lines: &[String]) -> String {
    let mut body = format!(".subckt {name} 1 2\nVKONNECT 1 n1 0\n");
    for line in lines {
        body.push_str(line);
        body.push('\n');
    }
    body.push_str(&format!(".ends {name}\n"));
    body
}

fn line(element: &str, from: &str, to: &str, value: f64) -> String {
    format!("{element} {from} {to} {}", number(value))
}

/// The DC resistance between the ports, read from the voltage a 1 A current
/// raises.
fn check_dc_resistance(
    model: &mut Model,
    bench: &mut Bench,
    characteristic: &Characteristic,
    expected: f64,
) {
    let id = bench.id();
    let node = format!("{id}a");
    bench.dc.push(Bench::current_into(&id, &node, TEST_AMPS));
    bench.dc.push(model.instance(&id, &[&node, "0"]));
    model.checks.push(Check {
        characteristic: characteristic.index,
        role: Role::Fit,
        expected,
        probe: Probe::Voltage {
            node,
            scale: 1.0,
            divide_by: TEST_AMPS,
        },
    });
}

/// An AC check driven by a unit AC current into port 1, reading the impedance
/// from the voltage it raises. An inductor needs this: at DC it is a 0 V
/// branch, so a voltage source across it and the sense source would close a
/// loop of voltage branches, which leaves the engine's matrix singular.
fn check_ac_current(
    model: &mut Model,
    bench: &mut Bench,
    characteristic: &Characteristic,
    probe: impl Fn(String) -> Probe,
) {
    let id = bench.id();
    let node = format!("{id}a");
    bench.ac.push(Bench::ac_current_into(&id, &node, 0.0));
    bench.ac.push(model.instance(&id, &[&node, "0"]));
    model.checks.push(Check {
        characteristic: characteristic.index,
        role: Role::Fit,
        expected: characteristic.value,
        probe: probe(node),
    });
}

/// An AC check driven by a unit AC voltage across the ports, which ties every
/// node at DC as a capacitor alone would not.
fn check_ac(
    model: &mut Model,
    bench: &mut Bench,
    characteristic: &Characteristic,
    probe: impl Fn(String) -> Probe,
) {
    let id = bench.id();
    let node = format!("{id}a");
    bench.ac.push(Bench::voltage(&id, &node, 0.0, true));
    bench.ac.push(model.instance(&id, &[&node, "0"]));
    model.checks.push(Check {
        characteristic: characteristic.index,
        role: Role::Fit,
        expected: characteristic.value,
        probe: probe(format!("V{id}")),
    });
}

pub(crate) fn build(kind: Kind, name: &str, characteristics: &[Characteristic]) -> Fitted<Model> {
    let mut model = Model::new(kind, name, Element::Subcircuit, vec!["1", "2"]);
    match kind {
        Kind::Resistance => resistance(&mut model, characteristics)?,
        Kind::Capacitor => capacitance(&mut model, characteristics)?,
        _ => inductance(&mut model, characteristics)?,
    }
    Ok(model)
}

fn resistance(model: &mut Model, characteristics: &[Characteristic]) -> Fitted<()> {
    let resistance = one(characteristics, Quantity::Resistance).expect("counted");
    let inductance = one(characteristics, Quantity::SeriesInductance);
    let r = resistance.value;
    let lines = match inductance {
        Some(inductance) => vec![
            line("R1", "n1", "n2", r),
            line("L1", "n2", "2", inductance.value),
        ],
        None => vec![line("R1", "n1", "2", r)],
    };
    model.body = subcircuit(&model.name, &lines);
    model.parameters.push(("R1".into(), r));
    model.fitted.push("the resistance as given".into());
    if let Some(inductance) = inductance {
        model.parameters.push(("L1".into(), inductance.value));
        model.fitted.push("a series inductance as given".into());
    }
    model.assumed.push(
        "a resistance is linear and fixed: a fuse modelled so never opens, and nothing heats"
            .into(),
    );
    let mut bench = Bench::default();
    check_dc_resistance(model, &mut bench, resistance, r);
    if let Some(inductance) = inductance {
        // A series R and L read back exactly at any frequency.
        let hz = SERIES_INDUCTANCE_HZ;
        check_ac_current(model, &mut bench, inductance, |node| {
            Probe::NodeInductance { node, hz }
        });
    }
    model.bench = bench;
    Ok(())
}

fn capacitance(model: &mut Model, characteristics: &[Characteristic]) -> Fitted<()> {
    let capacitance = one(characteristics, Quantity::Capacitance).expect("counted");
    let esr = one(characteristics, Quantity::EquivalentSeriesResistance);
    let esl = one(characteristics, Quantity::EquivalentSeriesInductance);
    let leakage = one(characteristics, Quantity::LeakageCurrent);
    let esl_value = esl.map_or(0.0, |esl| esl.value);
    let leak_ohm = match leakage {
        Some(leakage) => {
            let voltage = leakage.condition(VOLTAGE).unwrap_or_default();
            let ohm = voltage / leakage.value - esr.map_or(0.0, |esr| esr.value);
            if ohm <= 0.0 {
                return Err(Invalid::new(
                    leakage.field("value"),
                    "the leakage is larger than the ESR allows",
                ));
            }
            Some(ohm)
        }
        None => None,
    };
    let core = |farad: f64, omega: f64| {
        let c = capacitor(farad, omega);
        leak_ohm.map_or(c, |ohm| c.parallel(resistor(ohm)))
    };
    // The capacitance an LCR meter reads at the test frequency, from the whole
    // series impedance. Solve the card's value so that it comes back.
    let c_hz = frequency(capacitance, CAPACITANCE_HZ);
    let c_omega = TAU * c_hz;
    let read = |farad: f64| {
        let z = core(farad, c_omega).add(inductor(esl_value, c_omega));
        -1.0 / (c_omega * z.im)
    };
    let target = capacitance.value;
    let esl_term = c_omega * c_omega * esl_value;
    let farad = match leak_ohm {
        // C / (1 - w^2 L C) reads as the target.
        None => Some(target / (1.0 + esl_term * target)),
        Some(_) => {
            let below_resonance = if esl_term > 0.0 {
                0.999 / esl_term
            } else {
                f64::INFINITY
            };
            fit::root(
                |farad| read(farad) - target,
                target / 1e3,
                (target * 1e3).min(below_resonance),
            )
        }
    }
    .filter(|farad| farad.is_finite() && *farad > 0.0 && read(*farad) > 0.0)
    .ok_or_else(|| {
        Invalid::new(
            capacitance.field("value"),
            "no capacitance reads as given at that frequency with this leakage and ESL",
        )
    })?;
    let esr_ohm = match esr {
        Some(esr) => {
            let omega = TAU * frequency(esr, ESR_HZ);
            let ohm = esr.value - core(farad, omega).re;
            if ohm < 0.0 {
                return Err(Invalid::new(
                    esr.field("value"),
                    "the ESR is below what the leakage path gives at that frequency",
                ));
            }
            Some(ohm)
        }
        None => None,
    };
    let mut lines = vec![line("C1", "n1", "n2", farad)];
    if let Some(ohm) = leak_ohm {
        lines.push(line("RL", "n1", "n2", ohm));
    }
    match (esr_ohm, esl) {
        (Some(ohm), Some(esl)) => {
            lines.push(line("R1", "n2", "n3", ohm));
            lines.push(line("L1", "n3", "2", esl.value));
        }
        (Some(ohm), None) => lines.push(line("R1", "n2", "2", ohm)),
        (None, Some(esl)) => lines.push(line("L1", "n2", "2", esl.value)),
        (None, None) => {
            // Nothing in series: the capacitor ends on port 2.
            for line in &mut lines {
                *line = line.replacen(" n2 ", " 2 ", 1);
            }
        }
    }
    model.body = subcircuit(&model.name, &lines);
    model.parameters.push(("C1".into(), farad));
    if let Some(ohm) = leak_ohm {
        model.parameters.push(("RL".into(), ohm));
    }
    if let Some(ohm) = esr_ohm {
        model.parameters.push(("R1".into(), ohm));
    }
    if let Some(esl) = esl {
        model.parameters.push(("L1".into(), esl.value));
    }
    model
        .fitted
        .push("the capacitance, so that it reads as given at its test frequency".into());
    if esr.is_some() {
        model
            .fitted
            .push("the ESR, so that it reads as given at its test frequency".into());
    }
    model.assumed.push(
        "ESR and capacitance are fixed: DC bias, ageing and temperature do not change them".into(),
    );
    let mut bench = Bench::default();
    check_ac(model, &mut bench, capacitance, |source| {
        Probe::SeriesCapacitance { source, hz: c_hz }
    });
    if let Some(esr) = esr {
        let hz = frequency(esr, ESR_HZ);
        check_ac(model, &mut bench, esr, |source| Probe::Resistance {
            source,
            hz,
        });
    }
    if let Some(esl) = esl {
        let resonance = 1.0 / (TAU * (esl.value * farad).sqrt());
        let hz = (resonance * ABOVE_RESONANCE).min(1e15);
        check_ac(model, &mut bench, esl, |source| Probe::Inductance {
            source,
            hz,
        });
    }
    if let Some(leakage) = leakage {
        let id = bench.id();
        let node = format!("{id}a");
        let voltage = leakage.condition(VOLTAGE).unwrap_or_default();
        bench.dc.push(Bench::voltage(&id, &node, voltage, false));
        bench.dc.push(model.instance(&id, &[&node, "0"]));
        model.checks.push(Check {
            characteristic: leakage.index,
            role: Role::Fit,
            expected: leakage.value,
            probe: Probe::Current {
                source: format!("V{id}"),
                scale: -1.0,
            },
        });
    }
    model.bench = bench;
    Ok(())
}

fn inductance(model: &mut Model, characteristics: &[Characteristic]) -> Fitted<()> {
    let inductance = one(characteristics, Quantity::Inductance).expect("counted");
    let dcr = one(characteristics, Quantity::DcResistance);
    let resonance = one(characteristics, Quantity::SelfResonantFrequency);
    let dcr_ohm = dcr.map_or(0.0, |dcr| dcr.value);
    let l_hz = frequency(inductance, INDUCTANCE_HZ);
    if let Some(resonance) = resonance {
        if resonance.value <= l_hz {
            return Err(Invalid::new(
                inductance.field("conditions.frequency_hz"),
                "the inductance must be read below the self-resonant frequency",
            ));
        }
    }
    let parallel =
        |henry: f64| resonance.map(|resonance| 1.0 / ((TAU * resonance.value).powi(2) * henry));
    let omega = TAU * l_hz;
    let read = |henry: f64| {
        let winding = resistor(dcr_ohm).add(inductor(henry, omega));
        let z = match parallel(henry) {
            Some(farad) => winding.parallel(capacitor(farad, omega)),
            None => winding,
        };
        z.im / omega
    };
    let target = inductance.value;
    // A series R and L read back exactly. With the winding capacitance, the
    // reading rises above L where the coil's Q is high and falls below it
    // where Q is low, so the root may lie on either side of the target; the
    // reading rises with L wherever it is positive, so there is one root.
    let henry = match resonance {
        None => Some(target),
        Some(_) => fit::root(|henry| read(henry) - target, target / 1e3, target * 1e3),
    }
    .ok_or_else(|| {
        Invalid::new(
            inductance.field("value"),
            "no inductance reads as given at that frequency",
        )
    })?;
    let mut lines = Vec::new();
    match dcr {
        Some(dcr) => {
            lines.push(line("L1", "n1", "n2", henry));
            lines.push(line("R1", "n2", "2", dcr.value));
        }
        None => lines.push(line("L1", "n1", "2", henry)),
    }
    if let Some(farad) = parallel(henry) {
        lines.push(line("C1", "n1", "2", farad));
    }
    model.body = subcircuit(&model.name, &lines);
    model.parameters.push(("L1".into(), henry));
    if let Some(dcr) = dcr {
        model.parameters.push(("R1".into(), dcr.value));
    }
    if let Some(farad) = parallel(henry) {
        model.parameters.push(("C1".into(), farad));
        model.fitted.push(
            "a winding capacitance from the self-resonant frequency, which the engine does not \
             check"
                .into(),
        );
    }
    model
        .fitted
        .push("the inductance, so that it reads as given at its test frequency".into());
    model
        .assumed
        .push("the inductance is fixed: saturation and core loss are not modelled".into());
    let mut bench = Bench::default();
    check_ac_current(model, &mut bench, inductance, |node| {
        Probe::NodeInductance { node, hz: l_hz }
    });
    if let Some(dcr) = dcr {
        check_dc_resistance(model, &mut bench, dcr, dcr.value);
    }
    model.bench = bench;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::{build as build_model, text, Kind};
    use super::*;
    use serde_json::json;

    fn close(a: f64, b: f64, share: f64) -> bool {
        (a - b).abs() <= share * b.abs()
    }

    #[test]
    fn a_fuse_is_its_cold_resistance_behind_a_sense_source() {
        let characteristics = parsed(json!([
            { "quantity": "resistance", "value": 0.012, "statistic": "typical" }
        ]));
        let model = build_model(Kind::Resistance, "fuse_10a_fit", &characteristics, 25.0).unwrap();
        assert_eq!(
            model.body,
            ".subckt fuse_10a_fit 1 2\nVKONNECT 1 n1 0\nR1 n1 2 1.200000e-2\n.ends fuse_10a_fit\n"
        );
        let checked = text::check_fitted(model.file().as_bytes()).unwrap();
        assert_eq!(checked.pins(), ["1", "2"]);
        assert!(model
            .assumed
            .iter()
            .any(|note| note.contains("never opens")));
    }

    #[test]
    fn a_capacitor_reads_back_its_capacitance_and_esr_at_their_frequencies() {
        let characteristics = parsed(json!([
            { "quantity": "capacitance", "value": 100e-6, "conditions": { "frequency_hz": 120.0 }, "statistic": "typical" },
            { "quantity": "equivalent_series_resistance", "value": 0.08, "conditions": { "frequency_hz": 100e3 }, "statistic": "maximum" },
            { "quantity": "equivalent_series_inductance", "value": 12e-9, "statistic": "typical" },
            { "quantity": "leakage_current", "value": 3e-6, "conditions": { "voltage_volts": 25.0 }, "statistic": "maximum" }
        ]));
        let model = build_model(Kind::Capacitor, "cap_100u_fit", &characteristics, 25.0).unwrap();
        text::check_fitted(model.file().as_bytes()).unwrap();
        let farad = fitted(&model, "C1");
        let leak = fitted(&model, "RL");
        let esr = fitted(&model, "R1");
        let omega = TAU * 120.0;
        let z = capacitor(farad, omega)
            .parallel(resistor(leak))
            .add(resistor(esr))
            .add(inductor(12e-9, omega));
        assert!(close(-1.0 / (omega * z.im), 100e-6, 1e-9));
        let omega = TAU * 100e3;
        let z = capacitor(farad, omega)
            .parallel(resistor(leak))
            .add(resistor(esr));
        assert!(close(z.re, 0.08, 1e-9));
        assert!(close(25.0 / (leak + esr), 3e-6, 1e-12));
        assert_eq!(model.checks.len(), 4);
    }

    #[test]
    fn a_plain_capacitor_ends_on_port_two() {
        let characteristics = parsed(json!([
            { "quantity": "capacitance", "value": 1e-6, "statistic": "typical" }
        ]));
        let model = build_model(Kind::Capacitor, "c1u_fit", &characteristics, 25.0).unwrap();
        assert_eq!(
            model.body,
            ".subckt c1u_fit 1 2\nVKONNECT 1 n1 0\nC1 n1 2 1.000000e-6\n.ends c1u_fit\n"
        );
    }

    #[test]
    fn an_inductor_is_checked_with_a_current_so_no_voltage_loop_forms() {
        let characteristics = parsed(json!([
            { "quantity": "inductance", "value": 3.3e-6, "statistic": "typical" }
        ]));
        let model = build_model(Kind::Inductor, "l3u3_fit", &characteristics, 25.0).unwrap();
        // The value an E12 series rounds below its own reading still fits.
        assert_eq!(fitted(&model, "L1"), 3.3e-6);
        assert!(model
            .bench
            .ac
            .iter()
            .any(|line| line.starts_with("Ib1 0 b1a DC 0.000000e0 AC 1")));
        assert!(!model.bench.ac.iter().any(|line| line.starts_with('V')));
    }

    #[test]
    fn a_low_q_inductor_with_a_winding_capacitance_fits_above_its_reading() {
        // An RF chip inductor read at 100 kHz, where its Q is 0.06: the
        // winding capacitance there lowers the reading, so L1 sits above it.
        let characteristics = parsed(json!([
            { "quantity": "inductance", "value": 100e-9, "statistic": "typical" },
            { "quantity": "dc_resistance", "value": 1.07, "statistic": "maximum" },
            { "quantity": "self_resonant_frequency", "value": 1e9, "statistic": "minimum" }
        ]));
        let model = build_model(Kind::Inductor, "l100n_fit", &characteristics, 25.0).unwrap();
        let henry = fitted(&model, "L1");
        let farad = fitted(&model, "C1");
        assert!(henry > 100e-9);
        let omega = TAU * INDUCTANCE_HZ;
        let z = resistor(1.07)
            .add(inductor(henry, omega))
            .parallel(capacitor(farad, omega));
        assert!(close(z.im / omega, 100e-9, 1e-9));
    }

    #[test]
    fn a_capacitor_near_its_resonance_still_fits_below_it() {
        let characteristics = parsed(json!([
            { "quantity": "capacitance", "value": 2200e-6, "statistic": "typical" },
            { "quantity": "equivalent_series_inductance", "value": 20e-9, "statistic": "typical" }
        ]));
        let model = build_model(Kind::Capacitor, "c2200u_fit", &characteristics, 25.0).unwrap();
        let omega = TAU * CAPACITANCE_HZ;
        let farad = fitted(&model, "C1");
        let z = capacitor(farad, omega).add(inductor(20e-9, omega));
        assert!(close(-1.0 / (omega * z.im), 2200e-6, 1e-9));
    }

    #[test]
    fn an_inductor_reads_back_its_inductance_below_resonance() {
        let characteristics = parsed(json!([
            { "quantity": "inductance", "value": 10e-6, "conditions": { "frequency_hz": 100e3 }, "statistic": "typical" },
            { "quantity": "dc_resistance", "value": 0.025, "statistic": "maximum" },
            { "quantity": "self_resonant_frequency", "value": 30e6, "statistic": "typical" }
        ]));
        let model = build_model(Kind::Inductor, "l10u_fit", &characteristics, 25.0).unwrap();
        text::check_fitted(model.file().as_bytes()).unwrap();
        let henry = fitted(&model, "L1");
        let farad = fitted(&model, "C1");
        assert!(close(1.0 / (TAU * (henry * farad).sqrt()), 30e6, 1e-9));
        let omega = TAU * 100e3;
        let z = resistor(0.025)
            .add(inductor(henry, omega))
            .parallel(capacitor(farad, omega));
        assert!(close(z.im / omega, 10e-6, 1e-9));
        let above = parsed(json!([
            { "quantity": "inductance", "value": 10e-6, "conditions": { "frequency_hz": 50e6 }, "statistic": "typical" },
            { "quantity": "self_resonant_frequency", "value": 30e6, "statistic": "typical" }
        ]));
        assert!(build_model(Kind::Inductor, "l10u_fit", &above, 25.0).is_err());
    }
}
