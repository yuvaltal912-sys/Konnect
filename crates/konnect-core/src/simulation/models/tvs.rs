//! Transient-voltage suppressors, on the same diode cards as a Zener.
//!
//! A datasheet gives a TVS two points on its breakdown curve: the breakdown
//! voltage at a small test current, and the clamping voltage at the peak pulse
//! current. Between them the curve is `NBV Vt ln(I2 / I1) + RS (I2 - I1)`, so
//! one equation holds both unknowns. The fit takes `NBV = 1`, the sharpest knee
//! the junction law allows, and puts the rest of the rise in the series
//! resistance. A bidirectional part is two such cells in series, one conducting
//! forward while the other breaks down, so the voltage of each datasheet value
//! is the breakdown cell's plus the forward cell's drop.

use super::bench::{Bench, Role};
use super::diode::{
    breakdown_card, capacitance_parameters, check_capacitance, check_forward, check_leakage,
    check_voltage_at_current, field_of, fit_forward, forward_points, forward_voltage, hold_out,
    Breakdown, GENERIC_IS, GENERIC_N, SILICON,
};
use super::fit::thermal_voltage;
use super::text::{diode_card, number};
use super::{
    of, Characteristic, Element, Fitted, Invalid, Kind, Model, Quantity, PEAK_PULSE_CURRENT,
    REVERSE_CURRENT,
};

/// The forward diode's `IS`, `N` and `RS`, and the forward values behind them.
type ForwardSide = (
    (f64, f64, f64),
    Vec<(super::diode::Point, usize)>,
    Option<(super::diode::Point, usize)>,
);

pub(crate) fn build(
    kind: Kind,
    name: &str,
    characteristics: &[Characteristic],
    temperature: f64,
) -> Fitted<Model> {
    let bidirectional = kind == Kind::TvsBidirectional;
    let vt = thermal_voltage(temperature);
    let mut model = Model::new(kind, name, Element::Subcircuit, vec!["A", "K"]);
    let breakdown = of(characteristics, Quantity::BreakdownVoltage)[0];
    let clamp = of(characteristics, Quantity::ClampingVoltage)[0];
    let test_current = breakdown.condition(REVERSE_CURRENT).unwrap_or_default();
    let pulse_current = clamp.condition(PEAK_PULSE_CURRENT).unwrap_or_default();
    if pulse_current <= test_current {
        return Err(Invalid::new(
            clamp.field(&format!("conditions.{PEAK_PULSE_CURRENT}")),
            "the peak pulse current must be above the breakdown voltage's test current",
        ));
    }
    if clamp.value <= breakdown.value {
        return Err(Invalid::new(
            clamp.field("value"),
            "the clamping voltage must be above the breakdown voltage",
        ));
    }

    // The forward diode: fitted to forward values when a unidirectional part
    // has them, otherwise a plain silicon junction.
    let ((is, n, rs), fit_points, held) = if bidirectional {
        model.assumed.push(format!(
            "the forward cell of the pair is a plain silicon junction, is = {} A and n = \
             {GENERIC_N}, because a bidirectional part has no forward value",
            number(GENERIC_IS)
        ));
        ((GENERIC_IS, GENERIC_N, 0.0), Vec::new(), None)
    } else {
        forward_side(&mut model, characteristics, vt)?
    };
    let drop = |current: f64| {
        if bidirectional {
            forward_voltage(is, n, rs, vt, current)
        } else {
            0.0
        }
    };

    // What the breakdown cell alone must read at each datasheet point.
    let low = breakdown.value - drop(test_current);
    let high = clamp.value - drop(pulse_current);
    if low <= 0.0 || high <= low {
        return Err(Invalid::new(
            clamp.field("value"),
            "the pair's forward drop leaves no rise between the breakdown and clamping voltages",
        ));
    }
    let rise = high - low;
    let decades = (pulse_current / test_current).ln();
    let (nbv, rz) = {
        let rz = (rise - vt * decades) / (pulse_current - test_current);
        if rz >= 0.0 {
            model.assumed.push(
                "nbv = 1: two voltages fix the slope between them, not the knee, so the rise \
                 beyond the junction law is series resistance"
                    .into(),
            );
            (1.0, rz)
        } else {
            model.assumed.push(
                "the clamping voltage is below what the junction law alone gives, so the series \
                 resistance is zero and nbv takes the rise"
                    .into(),
            );
            (rise / (vt * decades), 0.0)
        }
    };
    let impedance_at_pulse = nbv * vt / pulse_current + rz;
    let Breakdown {
        is: breakdown_is,
        nbv,
        rz,
        bv,
        ibv,
    } = breakdown_card(
        &mut model,
        &breakdown.field("value"),
        low,
        test_current,
        (nbv, rz),
        &[(pulse_current, impedance_at_pulse)],
        vt,
    )?;
    model
        .fitted
        .push("rs from the rise between the breakdown and clamping voltages".into());

    let forward = vec![
        ("IS", is),
        ("N", n),
        ("RS", rs),
        ("EG", SILICON.eg),
        ("XTI", SILICON.xti),
        ("TNOM", temperature),
    ];
    let mut breaking = vec![
        ("IS", breakdown_is),
        ("N", 4.0),
        ("RS", rz),
        ("BV", bv),
        ("IBV", ibv),
        ("NBV", nbv),
    ];
    let (capacitance, (capacitance_fit, capacitance_held)) = if bidirectional {
        (Vec::new(), (Vec::new(), None))
    } else {
        capacitance_parameters(&mut model, characteristics, SILICON.vj)?
    };
    breaking.extend(capacitance);
    breaking.push(("TNOM", temperature));
    model
        .assumed
        .push("the breakdown voltage does not follow temperature".into());
    let cells = if bidirectional {
        format!("DF A M {name}_f\nDB A M {name}_b\nDG K M {name}_f\nDH K M {name}_b\n")
    } else {
        format!("DF A K {name}_f\nDB A K {name}_b\n")
    };
    model.body = format!(
        ".subckt {name} A K\n{cells}{}{}.ends {name}\n",
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
    check_voltage_at_current(&mut model, &mut bench, breakdown, test_current);
    check_voltage_at_current(&mut model, &mut bench, clamp, pulse_current);
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

/// The forward diode of a unidirectional part.
fn forward_side(
    model: &mut Model,
    characteristics: &[Characteristic],
    vt: f64,
) -> Fitted<ForwardSide> {
    let points = forward_points(characteristics);
    let (fit_points, held) = hold_out(&points);
    if points.is_empty() {
        model.assumed.push(format!(
            "no forward values: a silicon junction, is = {} A and n = {GENERIC_N}",
            number(GENERIC_IS)
        ));
        return Ok(((GENERIC_IS, GENERIC_N, 0.0), fit_points, held));
    }
    let values: Vec<_> = fit_points.iter().map(|(point, _)| *point).collect();
    let forward = fit_forward(
        &values,
        vt,
        SILICON.assumed_n,
        &field_of(characteristics, Quantity::ForwardVoltage),
    )?;
    model.fitted.push(forward.note.clone());
    model.assumed.extend(forward.assumed.iter().cloned());
    Ok(((forward.is, forward.n, forward.rs), fit_points, held))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::{build as build_model, text};
    use super::*;
    use serde_json::json;

    fn tvs_points(clamp_volts: f64) -> Vec<Characteristic> {
        parsed(json!([
            {
                "quantity": "breakdown_voltage", "value": 15.0,
                "conditions": { "reverse_current_amps": 1e-3 }, "statistic": "typical"
            },
            {
                "quantity": "clamping_voltage", "value": clamp_volts,
                "conditions": { "peak_pulse_current_amps": 20.0 }, "statistic": "maximum"
            }
        ]))
    }

    #[test]
    fn a_unidirectional_part_is_a_zener_shaped_subcircuit_in_the_fitted_grammar() {
        let model = build_model(Kind::Tvs, "smbj15_fit", &tvs_points(24.0), 25.0).unwrap();
        assert!(model
            .body
            .starts_with(".subckt smbj15_fit A K\nDF A K smbj15_fit_f\n"));
        let checked = text::check_fitted(model.file().as_bytes()).unwrap();
        assert_eq!(checked.pins(), ["A", "K"]);
        assert_eq!(checked.kind, "tvs");
        // The breakdown and the clamping voltage are each checked on the engine.
        assert_eq!(model.checks.len(), 2);
        // Two voltages fix the slope between them, not the knee.
        assert!(model.assumed.iter().any(|note| note.contains("nbv = 1")));
        // The series resistance carries the rest of the rise: about
        // (9 - 0.0257 ln 20000) / 19.999 = 0.437 ohm.
        let rz = fitted(&model, "breakdown.RS");
        assert!((rz - 0.4373).abs() < 0.01, "{rz}");
    }

    #[test]
    fn a_bidirectional_part_is_two_cells_in_series_and_takes_no_forward_or_capacitance_values() {
        let model = build_model(
            Kind::TvsBidirectional,
            "smbj15ca_fit",
            &tvs_points(24.0),
            25.0,
        )
        .unwrap();
        for line in ["DF A M ", "DB A M ", "DG K M ", "DH K M "] {
            assert!(model.body.contains(line), "{line}: {}", model.body);
        }
        let checked = text::check_fitted(model.file().as_bytes()).unwrap();
        assert_eq!(checked.pins(), ["A", "K"]);
        assert_eq!(checked.kind, "tvs_bidirectional");
        // The breakdown cell reads the datasheet voltage less the other cell's
        // forward drop.
        assert!(fitted(&model, "breakdown.BV") < 15.0 - 0.3);
        let forward = parsed(json!([
            { "quantity": "breakdown_voltage", "value": 15.0,
              "conditions": { "reverse_current_amps": 1e-3 }, "statistic": "typical" },
            { "quantity": "clamping_voltage", "value": 24.0,
              "conditions": { "peak_pulse_current_amps": 20.0 }, "statistic": "maximum" },
            { "quantity": "forward_voltage", "value": 1.0,
              "conditions": { "forward_current_amps": 1.0 }, "statistic": "typical" }
        ]));
        let error =
            build_model(Kind::TvsBidirectional, "smbj15ca_fit", &forward, 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[2].quantity");
    }

    #[test]
    fn a_clamping_voltage_that_does_not_rise_or_a_pulse_that_does_not_exceed_the_test_current_is_refused(
    ) {
        let error = build_model(Kind::Tvs, "smbj15_fit", &tvs_points(14.0), 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[1].value");
        let flat = parsed(json!([
            { "quantity": "breakdown_voltage", "value": 15.0,
              "conditions": { "reverse_current_amps": 1e-3 }, "statistic": "typical" },
            { "quantity": "clamping_voltage", "value": 24.0,
              "conditions": { "peak_pulse_current_amps": 1e-3 }, "statistic": "maximum" }
        ]));
        let error = build_model(Kind::Tvs, "smbj15_fit", &flat, 25.0).unwrap_err();
        assert_eq!(
            error.field,
            "characteristics[1].conditions.peak_pulse_current_amps"
        );
    }

    #[test]
    fn a_clamp_below_the_junction_law_puts_the_rise_in_nbv_and_leaves_no_resistance() {
        // 15.1 V at 20 A, after 15 V at 1 mA: the law alone gives 0.25 V.
        let model = build_model(Kind::Tvs, "smbj15_fit", &tvs_points(15.1), 25.0).unwrap();
        assert_eq!(fitted(&model, "breakdown.RS"), 0.0);
        assert!(fitted(&model, "breakdown.NBV") < 1.0);
    }
}
