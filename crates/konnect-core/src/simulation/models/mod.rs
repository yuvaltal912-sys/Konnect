//! Models fitted to characteristics the caller read from a datasheet.
//!
//! Konnect never sees the datasheet. The caller gives values with their test
//! conditions, in SI base units; this fits model parameters to them, writes
//! the model in the narrow grammar of [`text`], and builds a bench that
//! measures the same values back on the engine. A value the engine does not
//! reproduce reads as a mismatch, and every value a fit had to take without
//! the caller's numbers is listed as assumed.

pub(crate) mod amplifier;
pub(crate) mod bench;
pub(crate) mod bjt;
pub(crate) mod diode;
pub(crate) mod fit;
pub(crate) mod mosfet;
pub(crate) mod passive;
pub(crate) mod regulator;
pub(crate) mod switch;
pub(crate) mod text;
pub(crate) mod tvs;

use serde_json::Value;

/// The 0 V source a fitted passive carries at port 1, so its current reads as
/// a plain part's does even when device currents are not saved.
pub(crate) const SENSE_SOURCE: &str = "VKONNECT";

pub(crate) use bench::{Bench, Check, Role};

/// Datasheet values are at 25 C unless the caller says otherwise.
pub(crate) const DEFAULT_TEMPERATURE: f64 = 25.0;
/// One temperature per model, within the range its laws were made for.
pub(crate) const TEMPERATURE_RANGE: std::ops::RangeInclusive<f64> = -55.0..=175.0;
pub(crate) const MAX_CHARACTERISTICS: usize = 48;
const MAX_PAGE: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Diode,
    SchottkyDiode,
    Led,
    Zener,
    Tvs,
    TvsBidirectional,
    NMosfet,
    PMosfet,
    Npn,
    Pnp,
    OpAmp,
    Comparator,
    Ldo,
    VoltageReference,
    LoadSwitch,
    Efuse,
    IdealDiode,
    Resistance,
    Capacitor,
    Inductor,
}

impl Kind {
    pub(crate) const ALL: [Kind; 20] = [
        Kind::Diode,
        Kind::SchottkyDiode,
        Kind::Led,
        Kind::Zener,
        Kind::Tvs,
        Kind::TvsBidirectional,
        Kind::NMosfet,
        Kind::PMosfet,
        Kind::Npn,
        Kind::Pnp,
        Kind::OpAmp,
        Kind::Comparator,
        Kind::Ldo,
        Kind::VoltageReference,
        Kind::LoadSwitch,
        Kind::Efuse,
        Kind::IdealDiode,
        Kind::Resistance,
        Kind::Capacitor,
        Kind::Inductor,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::Diode => "diode",
            Kind::SchottkyDiode => "schottky_diode",
            Kind::Led => "led",
            Kind::Zener => "zener",
            Kind::Tvs => "tvs",
            Kind::TvsBidirectional => "tvs_bidirectional",
            Kind::NMosfet => "n_mosfet",
            Kind::PMosfet => "p_mosfet",
            Kind::Npn => "npn",
            Kind::Pnp => "pnp",
            Kind::OpAmp => "op_amp",
            Kind::Comparator => "comparator",
            Kind::Ldo => "ldo",
            Kind::VoltageReference => "voltage_reference",
            Kind::LoadSwitch => "load_switch",
            Kind::Efuse => "efuse",
            Kind::IdealDiode => "ideal_diode",
            Kind::Resistance => "resistance",
            Kind::Capacitor => "capacitor",
            Kind::Inductor => "inductor",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// The quantities this kind takes, with how many of each: at least, at most.
    fn quantities(self) -> &'static [(Quantity, usize, usize)] {
        use Quantity::*;
        match self {
            Kind::Diode | Kind::SchottkyDiode => &[
                (ForwardVoltage, 2, 16),
                (ReverseCurrent, 0, 4),
                (BreakdownVoltage, 0, 1),
                (JunctionCapacitance, 0, 16),
            ],
            Kind::Led => &[
                (ForwardVoltage, 2, 16),
                (ReverseCurrent, 0, 4),
                (JunctionCapacitance, 0, 16),
            ],
            Kind::Zener => &[
                (ZenerVoltage, 1, 1),
                (DynamicImpedance, 1, 2),
                (ForwardVoltage, 0, 16),
                (ReverseCurrent, 0, 4),
                (JunctionCapacitance, 0, 16),
            ],
            Kind::Tvs => &[
                (BreakdownVoltage, 1, 1),
                (ClampingVoltage, 1, 1),
                (ReverseCurrent, 0, 4),
                (ForwardVoltage, 0, 16),
                (JunctionCapacitance, 0, 16),
            ],
            // Both directions are the same, so a forward voltage or a junction
            // capacitance has no one meaning for the pair.
            Kind::TvsBidirectional => &[
                (BreakdownVoltage, 1, 1),
                (ClampingVoltage, 1, 1),
                (ReverseCurrent, 0, 4),
            ],
            // Every value is a magnitude, so a P-channel part reads as an
            // N-channel one. The forward voltage is the body diode's.
            Kind::NMosfet | Kind::PMosfet => &[
                (GateThresholdVoltage, 1, 1),
                (OnResistance, 1, 1),
                (DrainCurrent, 2, 16),
                (GateResistance, 0, 1),
                (ForwardVoltage, 0, 16),
                (InputCapacitance, 0, 16),
                (OutputCapacitance, 0, 16),
                (ReverseTransferCapacitance, 0, 16),
                (GateCharge, 0, 1),
                (GateDrainCharge, 0, 1),
            ],
            // Values are magnitudes, so a PNP reads as an NPN.
            Kind::Npn | Kind::Pnp => &[
                (CurrentGain, 3, 16),
                (BaseEmitterVoltage, 1, 16),
                (CollectorEmitterSaturationVoltage, 0, 1),
                (CollectorBaseCapacitance, 0, 16),
                (EmitterBaseCapacitance, 0, 16),
                (TransitionFrequency, 0, 1),
            ],
            // One open-loop gain is the DC gain; more, each at a frequency off
            // the Bode plot, are held out against the single pole.
            Kind::OpAmp => &[
                (OpenLoopGain, 1, 9),
                (GainBandwidthProduct, 1, 1),
                (SlewRate, 0, 1),
                (InputOffsetVoltage, 0, 1),
                (InputBiasCurrent, 0, 1),
                (SupplyCurrent, 0, 1),
                (OutputSwingFromPositiveRail, 0, 1),
                (OutputSwingFromNegativeRail, 0, 1),
                (OutputResistance, 0, 1),
            ],
            Kind::Comparator => &[
                (PropagationDelay, 1, 8),
                (Hysteresis, 0, 1),
                (InputOffsetVoltage, 0, 1),
                (InputBiasCurrent, 0, 1),
                (SupplyCurrent, 0, 1),
                (OutputSwingFromPositiveRail, 0, 1),
                (OutputSwingFromNegativeRail, 0, 1),
                (OutputResistance, 0, 1),
            ],
            Kind::Ldo => &[
                (OutputVoltage, 1, 16),
                (DropoutVoltage, 1, 1),
                (QuiescentCurrent, 0, 1),
                (CurrentLimit, 0, 1),
            ],
            Kind::VoltageReference => &[
                (OutputVoltage, 1, 16),
                (DropoutVoltage, 0, 1),
                (QuiescentCurrent, 0, 1),
                (CurrentLimit, 0, 1),
            ],
            Kind::LoadSwitch => &[
                (SwitchOnResistance, 1, 1),
                (EnableThreshold, 0, 1),
                (OutputRiseTime, 0, 1),
                (QuiescentCurrent, 0, 1),
                (CurrentLimit, 0, 1),
            ],
            Kind::Efuse => &[
                (SwitchOnResistance, 1, 1),
                (CurrentLimit, 1, 1),
                (UndervoltageLockout, 0, 1),
                (OvervoltageCutoff, 0, 1),
                (QuiescentCurrent, 0, 1),
            ],
            // Forward values off the drop-against-current figure; with more
            // than the fit needs, the middle one is held out.
            Kind::IdealDiode => &[
                (ForwardVoltage, 1, 16),
                (SwitchOnResistance, 0, 1),
                (ReverseCurrent, 0, 1),
                (QuiescentCurrent, 0, 1),
            ],
            Kind::Resistance => &[(Resistance, 1, 1), (SeriesInductance, 0, 1)],
            Kind::Capacitor => &[
                (Capacitance, 1, 1),
                (EquivalentSeriesResistance, 0, 1),
                (EquivalentSeriesInductance, 0, 1),
                (LeakageCurrent, 0, 1),
            ],
            Kind::Inductor => &[
                (Inductance, 1, 1),
                (DcResistance, 0, 1),
                (SelfResonantFrequency, 0, 1),
            ],
        }
    }
}

/// What a characteristic measures. Its unit is fixed, so a value is a plain
/// number and no scale letter can enter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Quantity {
    ForwardVoltage,
    ReverseCurrent,
    BreakdownVoltage,
    ClampingVoltage,
    GateThresholdVoltage,
    OnResistance,
    DrainCurrent,
    GateResistance,
    InputCapacitance,
    OutputCapacitance,
    ReverseTransferCapacitance,
    GateCharge,
    GateDrainCharge,
    CurrentGain,
    BaseEmitterVoltage,
    CollectorEmitterSaturationVoltage,
    CollectorBaseCapacitance,
    EmitterBaseCapacitance,
    TransitionFrequency,
    ZenerVoltage,
    DynamicImpedance,
    JunctionCapacitance,
    Resistance,
    SeriesInductance,
    Capacitance,
    EquivalentSeriesResistance,
    EquivalentSeriesInductance,
    LeakageCurrent,
    Inductance,
    DcResistance,
    SelfResonantFrequency,
    OpenLoopGain,
    GainBandwidthProduct,
    SlewRate,
    InputOffsetVoltage,
    InputBiasCurrent,
    SupplyCurrent,
    OutputSwingFromPositiveRail,
    OutputSwingFromNegativeRail,
    OutputResistance,
    PropagationDelay,
    Hysteresis,
    OutputVoltage,
    DropoutVoltage,
    QuiescentCurrent,
    CurrentLimit,
    SwitchOnResistance,
    UndervoltageLockout,
    OvervoltageCutoff,
    EnableThreshold,
    OutputRiseTime,
}

pub(crate) const FORWARD_CURRENT: &str = "forward_current_amps";
pub(crate) const REVERSE_CURRENT: &str = "reverse_current_amps";
pub(crate) const REVERSE_VOLTAGE: &str = "reverse_voltage_volts";
pub(crate) const PEAK_PULSE_CURRENT: &str = "peak_pulse_current_amps";
pub(crate) const GATE_SOURCE_VOLTAGE: &str = "gate_source_voltage_volts";
pub(crate) const DRAIN_SOURCE_VOLTAGE: &str = "drain_source_voltage_volts";
pub(crate) const DRAIN_CURRENT: &str = "drain_current_amps";
pub(crate) const COLLECTOR_CURRENT: &str = "collector_current_amps";
pub(crate) const COLLECTOR_EMITTER_VOLTAGE: &str = "collector_emitter_voltage_volts";
pub(crate) const BASE_CURRENT: &str = "base_current_amps";
pub(crate) const VOLTAGE: &str = "voltage_volts";
pub(crate) const FREQUENCY: &str = "frequency_hz";
pub(crate) const DC_BIAS: &str = "dc_bias_volts";
pub(crate) const DC_CURRENT: &str = "dc_current_amps";
pub(crate) const SUPPLY_VOLTAGE: &str = "supply_voltage_volts";
pub(crate) const LOAD_RESISTANCE: &str = "load_resistance_ohms";
pub(crate) const OVERDRIVE: &str = "overdrive_volts";
pub(crate) const INPUT_VOLTAGE: &str = "input_voltage_volts";
pub(crate) const OUTPUT_CURRENT: &str = "output_current_amps";
pub(crate) const OUTPUT_VOLTAGE: &str = "output_voltage_volts";
pub(crate) const LOAD_CAPACITANCE: &str = "load_capacitance_farads";

impl Quantity {
    pub(crate) const ALL: [Quantity; 51] = [
        Quantity::ForwardVoltage,
        Quantity::ReverseCurrent,
        Quantity::BreakdownVoltage,
        Quantity::ClampingVoltage,
        Quantity::GateThresholdVoltage,
        Quantity::OnResistance,
        Quantity::DrainCurrent,
        Quantity::GateResistance,
        Quantity::InputCapacitance,
        Quantity::OutputCapacitance,
        Quantity::ReverseTransferCapacitance,
        Quantity::GateCharge,
        Quantity::GateDrainCharge,
        Quantity::CurrentGain,
        Quantity::BaseEmitterVoltage,
        Quantity::CollectorEmitterSaturationVoltage,
        Quantity::CollectorBaseCapacitance,
        Quantity::EmitterBaseCapacitance,
        Quantity::TransitionFrequency,
        Quantity::ZenerVoltage,
        Quantity::DynamicImpedance,
        Quantity::JunctionCapacitance,
        Quantity::Resistance,
        Quantity::SeriesInductance,
        Quantity::Capacitance,
        Quantity::EquivalentSeriesResistance,
        Quantity::EquivalentSeriesInductance,
        Quantity::LeakageCurrent,
        Quantity::Inductance,
        Quantity::DcResistance,
        Quantity::SelfResonantFrequency,
        Quantity::OpenLoopGain,
        Quantity::GainBandwidthProduct,
        Quantity::SlewRate,
        Quantity::InputOffsetVoltage,
        Quantity::InputBiasCurrent,
        Quantity::SupplyCurrent,
        Quantity::OutputSwingFromPositiveRail,
        Quantity::OutputSwingFromNegativeRail,
        Quantity::OutputResistance,
        Quantity::PropagationDelay,
        Quantity::Hysteresis,
        Quantity::OutputVoltage,
        Quantity::DropoutVoltage,
        Quantity::QuiescentCurrent,
        Quantity::CurrentLimit,
        Quantity::SwitchOnResistance,
        Quantity::UndervoltageLockout,
        Quantity::OvervoltageCutoff,
        Quantity::EnableThreshold,
        Quantity::OutputRiseTime,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Quantity::ForwardVoltage => "forward_voltage",
            Quantity::ReverseCurrent => "reverse_current",
            Quantity::BreakdownVoltage => "breakdown_voltage",
            Quantity::ClampingVoltage => "clamping_voltage",
            Quantity::GateThresholdVoltage => "gate_threshold_voltage",
            Quantity::OnResistance => "on_resistance",
            Quantity::DrainCurrent => "drain_current",
            Quantity::GateResistance => "gate_resistance",
            Quantity::InputCapacitance => "input_capacitance",
            Quantity::OutputCapacitance => "output_capacitance",
            Quantity::ReverseTransferCapacitance => "reverse_transfer_capacitance",
            Quantity::GateCharge => "gate_charge",
            Quantity::GateDrainCharge => "gate_drain_charge",
            Quantity::CurrentGain => "current_gain",
            Quantity::BaseEmitterVoltage => "base_emitter_voltage",
            Quantity::CollectorEmitterSaturationVoltage => "collector_emitter_saturation_voltage",
            Quantity::CollectorBaseCapacitance => "collector_base_capacitance",
            Quantity::EmitterBaseCapacitance => "emitter_base_capacitance",
            Quantity::TransitionFrequency => "transition_frequency",
            Quantity::ZenerVoltage => "zener_voltage",
            Quantity::DynamicImpedance => "dynamic_impedance",
            Quantity::JunctionCapacitance => "junction_capacitance",
            Quantity::Resistance => "resistance",
            Quantity::SeriesInductance => "series_inductance",
            Quantity::Capacitance => "capacitance",
            Quantity::EquivalentSeriesResistance => "equivalent_series_resistance",
            Quantity::EquivalentSeriesInductance => "equivalent_series_inductance",
            Quantity::LeakageCurrent => "leakage_current",
            Quantity::Inductance => "inductance",
            Quantity::DcResistance => "dc_resistance",
            Quantity::SelfResonantFrequency => "self_resonant_frequency",
            Quantity::OpenLoopGain => "open_loop_gain",
            Quantity::GainBandwidthProduct => "gain_bandwidth_product",
            Quantity::SlewRate => "slew_rate",
            Quantity::InputOffsetVoltage => "input_offset_voltage",
            Quantity::InputBiasCurrent => "input_bias_current",
            Quantity::SupplyCurrent => "supply_current",
            Quantity::OutputSwingFromPositiveRail => "output_swing_from_positive_rail",
            Quantity::OutputSwingFromNegativeRail => "output_swing_from_negative_rail",
            Quantity::OutputResistance => "output_resistance",
            Quantity::PropagationDelay => "propagation_delay",
            Quantity::Hysteresis => "hysteresis",
            Quantity::OutputVoltage => "output_voltage",
            Quantity::DropoutVoltage => "dropout_voltage",
            Quantity::QuiescentCurrent => "quiescent_current",
            Quantity::CurrentLimit => "current_limit",
            Quantity::SwitchOnResistance => "switch_on_resistance",
            Quantity::UndervoltageLockout => "undervoltage_lockout",
            Quantity::OvervoltageCutoff => "overvoltage_cutoff",
            Quantity::EnableThreshold => "enable_threshold",
            Quantity::OutputRiseTime => "output_rise_time",
        }
    }

    pub(crate) fn unit(self) -> &'static str {
        match self {
            Quantity::ForwardVoltage
            | Quantity::BreakdownVoltage
            | Quantity::ClampingVoltage
            | Quantity::BaseEmitterVoltage
            | Quantity::CollectorEmitterSaturationVoltage
            | Quantity::GateThresholdVoltage
            | Quantity::ZenerVoltage
            | Quantity::InputOffsetVoltage
            | Quantity::OutputSwingFromPositiveRail
            | Quantity::OutputSwingFromNegativeRail
            | Quantity::Hysteresis
            | Quantity::OutputVoltage
            | Quantity::DropoutVoltage
            | Quantity::UndervoltageLockout
            | Quantity::OvervoltageCutoff
            | Quantity::EnableThreshold => "V",
            Quantity::ReverseCurrent
            | Quantity::LeakageCurrent
            | Quantity::DrainCurrent
            | Quantity::InputBiasCurrent
            | Quantity::SupplyCurrent
            | Quantity::QuiescentCurrent
            | Quantity::CurrentLimit => "A",
            Quantity::DynamicImpedance
            | Quantity::Resistance
            | Quantity::EquivalentSeriesResistance
            | Quantity::OnResistance
            | Quantity::GateResistance
            | Quantity::OutputResistance
            | Quantity::SwitchOnResistance
            | Quantity::DcResistance => "ohm",
            Quantity::JunctionCapacitance
            | Quantity::Capacitance
            | Quantity::InputCapacitance
            | Quantity::OutputCapacitance
            | Quantity::CollectorBaseCapacitance
            | Quantity::EmitterBaseCapacitance
            | Quantity::ReverseTransferCapacitance => "F",
            Quantity::SeriesInductance
            | Quantity::EquivalentSeriesInductance
            | Quantity::Inductance => "H",
            Quantity::SelfResonantFrequency
            | Quantity::TransitionFrequency
            | Quantity::GainBandwidthProduct => "Hz",
            Quantity::CurrentGain | Quantity::OpenLoopGain => "ratio",
            Quantity::SlewRate => "V/s",
            Quantity::PropagationDelay | Quantity::OutputRiseTime => "s",
            Quantity::GateCharge | Quantity::GateDrainCharge => "C",
        }
    }

    /// The conditions it must state, then those it may.
    fn conditions(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            Quantity::ForwardVoltage => (&[FORWARD_CURRENT], &[]),
            Quantity::ReverseCurrent => (&[REVERSE_VOLTAGE], &[]),
            Quantity::BreakdownVoltage | Quantity::ZenerVoltage => (&[REVERSE_CURRENT], &[]),
            Quantity::ClampingVoltage => (&[PEAK_PULSE_CURRENT], &[]),
            Quantity::GateThresholdVoltage => (&[DRAIN_CURRENT], &[]),
            Quantity::OnResistance => (&[GATE_SOURCE_VOLTAGE, DRAIN_CURRENT], &[]),
            Quantity::DrainCurrent => (&[GATE_SOURCE_VOLTAGE, DRAIN_SOURCE_VOLTAGE], &[]),
            Quantity::GateResistance => (&[], &[FREQUENCY]),
            Quantity::InputCapacitance
            | Quantity::OutputCapacitance
            | Quantity::ReverseTransferCapacitance => (&[DRAIN_SOURCE_VOLTAGE], &[FREQUENCY]),
            Quantity::GateCharge => (
                &[DRAIN_SOURCE_VOLTAGE, DRAIN_CURRENT, GATE_SOURCE_VOLTAGE],
                &[],
            ),
            Quantity::GateDrainCharge => (&[DRAIN_SOURCE_VOLTAGE, DRAIN_CURRENT], &[]),
            Quantity::CurrentGain | Quantity::BaseEmitterVoltage => {
                (&[COLLECTOR_CURRENT, COLLECTOR_EMITTER_VOLTAGE], &[])
            }
            Quantity::CollectorEmitterSaturationVoltage => {
                (&[COLLECTOR_CURRENT, BASE_CURRENT], &[])
            }
            Quantity::CollectorBaseCapacitance | Quantity::EmitterBaseCapacitance => {
                (&[REVERSE_VOLTAGE], &[FREQUENCY])
            }
            Quantity::TransitionFrequency => (
                &[COLLECTOR_CURRENT, COLLECTOR_EMITTER_VOLTAGE],
                &[FREQUENCY],
            ),
            Quantity::DynamicImpedance => (&[REVERSE_CURRENT], &[FREQUENCY]),
            Quantity::JunctionCapacitance => (&[REVERSE_VOLTAGE], &[FREQUENCY]),
            Quantity::LeakageCurrent => (&[VOLTAGE], &[]),
            Quantity::Capacitance => (&[], &[FREQUENCY, DC_BIAS]),
            Quantity::Inductance => (&[], &[FREQUENCY, DC_CURRENT]),
            Quantity::EquivalentSeriesResistance => (&[], &[FREQUENCY]),
            Quantity::Resistance
            | Quantity::SeriesInductance
            | Quantity::EquivalentSeriesInductance
            | Quantity::DcResistance
            | Quantity::SelfResonantFrequency => (&[], &[]),
            Quantity::OpenLoopGain => (&[], &[FREQUENCY, SUPPLY_VOLTAGE, LOAD_RESISTANCE]),
            Quantity::GainBandwidthProduct
            | Quantity::SlewRate
            | Quantity::InputOffsetVoltage
            | Quantity::InputBiasCurrent
            | Quantity::OutputResistance
            | Quantity::Hysteresis => (&[], &[SUPPLY_VOLTAGE]),
            Quantity::SupplyCurrent => (&[SUPPLY_VOLTAGE], &[]),
            Quantity::OutputSwingFromPositiveRail | Quantity::OutputSwingFromNegativeRail => {
                (&[SUPPLY_VOLTAGE, LOAD_RESISTANCE], &[])
            }
            Quantity::PropagationDelay => (&[OVERDRIVE], &[SUPPLY_VOLTAGE]),
            Quantity::OutputVoltage => (&[INPUT_VOLTAGE, OUTPUT_CURRENT], &[]),
            Quantity::DropoutVoltage => (&[OUTPUT_CURRENT], &[]),
            Quantity::QuiescentCurrent => (&[INPUT_VOLTAGE], &[]),
            Quantity::CurrentLimit => (&[], &[INPUT_VOLTAGE, OUTPUT_VOLTAGE]),
            Quantity::SwitchOnResistance => (&[OUTPUT_CURRENT], &[INPUT_VOLTAGE]),
            Quantity::UndervoltageLockout | Quantity::OvervoltageCutoff => (&[], &[]),
            Quantity::EnableThreshold => (&[], &[INPUT_VOLTAGE]),
            Quantity::OutputRiseTime => {
                (&[INPUT_VOLTAGE], &[LOAD_RESISTANCE, LOAD_CAPACITANCE])
            }
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|quantity| quantity.name() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Statistic {
    Typical,
    Minimum,
    Maximum,
}

impl Statistic {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Statistic::Typical => "typical",
            Statistic::Minimum => "minimum",
            Statistic::Maximum => "maximum",
        }
    }
}

/// One value from the datasheet, as the caller gave it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Characteristic {
    /// Its place in the caller's list.
    pub index: usize,
    pub quantity: Quantity,
    pub value: f64,
    pub conditions: Vec<(&'static str, f64)>,
    pub statistic: Statistic,
}

impl Characteristic {
    pub(crate) fn condition(&self, key: &str) -> Option<f64> {
        self.conditions
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| *value)
    }

    /// The path of a field of this characteristic, for an error.
    pub(crate) fn field(&self, key: &str) -> String {
        format!("characteristics[{}].{key}", self.index)
    }
}

/// A value the caller gave that a fit cannot take, with the field it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Invalid {
    pub field: String,
    pub reason: String,
}

impl Invalid {
    pub(crate) fn new(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            reason: reason.into(),
        }
    }
}

pub(crate) type Fitted<T> = Result<T, Invalid>;

/// Read the caller's characteristics. Every value must be a finite number
/// above zero in its quantity's unit, with the conditions that quantity needs.
pub(crate) fn parse_characteristics(value: &Value) -> Fitted<Vec<Characteristic>> {
    let Some(items) = value.as_array() else {
        return Err(Invalid::new(
            "characteristics",
            "characteristics must be a list",
        ));
    };
    if items.is_empty() || items.len() > MAX_CHARACTERISTICS {
        return Err(Invalid::new(
            "characteristics",
            format!("give from 1 to {MAX_CHARACTERISTICS} characteristics"),
        ));
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| characteristic(index, item))
        .collect()
}

fn characteristic(index: usize, item: &Value) -> Fitted<Characteristic> {
    let path = |key: &str| format!("characteristics[{index}].{key}");
    let Some(object) = item.as_object() else {
        return Err(Invalid::new(
            format!("characteristics[{index}]"),
            "must be an object",
        ));
    };
    const KEYS: [&str; 6] = [
        "quantity",
        "value",
        "conditions",
        "statistic",
        "read_from",
        "page",
    ];
    if let Some(key) = object.keys().find(|key| !KEYS.contains(&key.as_str())) {
        return Err(Invalid::new(
            path(key),
            "is not a field of a characteristic",
        ));
    }
    let quantity = object
        .get("quantity")
        .and_then(Value::as_str)
        .and_then(Quantity::from_name)
        .ok_or_else(|| Invalid::new(path("quantity"), "is not a quantity Konnect fits"))?;
    let value = positive(object.get("value"), &path("value"))?;
    let statistic = match object.get("statistic").and_then(Value::as_str) {
        Some("typical") => Statistic::Typical,
        Some("minimum") => Statistic::Minimum,
        Some("maximum") => Statistic::Maximum,
        _ => {
            return Err(Invalid::new(
                path("statistic"),
                "must be typical, minimum or maximum, as the datasheet states it",
            ))
        }
    };
    match object.get("read_from") {
        None => {}
        Some(Value::String(source)) if matches!(source.as_str(), "table" | "figure" | "text") => {}
        Some(_) => {
            return Err(Invalid::new(
                path("read_from"),
                "must be table, figure or text",
            ))
        }
    }
    match object.get("page") {
        None => {}
        Some(Value::String(page))
            if !page.is_empty()
                && page.len() <= MAX_PAGE
                && page
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-') => {}
        Some(_) => {
            return Err(Invalid::new(
                path("page"),
                format!(
                    "must be a page number or range of at most {MAX_PAGE} letters, digits and -"
                ),
            ))
        }
    }
    let (required, optional) = quantity.conditions();
    let mut conditions = Vec::new();
    match object.get("conditions") {
        None => {}
        Some(Value::Object(given)) => {
            for (key, value) in given {
                let Some(known) = required
                    .iter()
                    .chain(optional)
                    .find(|known| **known == key.as_str())
                else {
                    return Err(Invalid::new(
                        format!("{}.{key}", path("conditions")),
                        format!("is not a condition of {}", quantity.name()),
                    ));
                };
                let field = format!("{}.{key}", path("conditions"));
                // A bias may be zero: CJO is a capacitance at 0 V, and an MLCC is
                // often stated with no bias at all.
                let number = if ZERO_BIAS.contains(&(quantity, *known)) {
                    non_negative(Some(value), &field)?
                } else {
                    positive(Some(value), &field)?
                };
                conditions.push((*known, number));
            }
        }
        Some(_) => return Err(Invalid::new(path("conditions"), "must be an object")),
    }
    if let Some(missing) = required
        .iter()
        .find(|key| !conditions.iter().any(|(name, _)| name == *key))
    {
        return Err(Invalid::new(
            format!("{}.{missing}", path("conditions")),
            format!("{} needs its {missing}", quantity.name()),
        ));
    }
    Ok(Characteristic {
        index,
        quantity,
        value,
        conditions,
        statistic,
    })
}

/// Conditions that may be zero, for the quantity they qualify.
const ZERO_BIAS: [(Quantity, &str); 5] = [
    (Quantity::JunctionCapacitance, REVERSE_VOLTAGE),
    (Quantity::Capacitance, DC_BIAS),
    (Quantity::Inductance, DC_CURRENT),
    // A reference is often stated with no load, and a current limit with the
    // output shorted.
    (Quantity::OutputVoltage, OUTPUT_CURRENT),
    (Quantity::CurrentLimit, OUTPUT_VOLTAGE),
];

fn non_negative(value: Option<&Value>, field: &str) -> Fitted<f64> {
    match value.and_then(Value::as_f64) {
        Some(number) if number.is_finite() && number >= 0.0 => Ok(number),
        _ => Err(Invalid::new(
            field,
            "must be a number of zero or more, in the quantity's SI base unit",
        )),
    }
}

fn positive(value: Option<&Value>, field: &str) -> Fitted<f64> {
    match value.and_then(Value::as_f64) {
        Some(number) if number.is_finite() && number > 0.0 => Ok(number),
        _ => Err(Invalid::new(
            field,
            "must be a number above zero, in the quantity's SI base unit",
        )),
    }
}

/// How an instance of the model is written in a deck.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Element {
    /// `D name anode cathode model`.
    Diode,
    /// `X name nodes... model`.
    Subcircuit,
}

/// A fitted model, ready to check and to write.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Model {
    pub kind: Kind,
    pub name: String,
    /// The cards, which follow the marker line in the file.
    pub body: String,
    pub element: Element,
    /// Model pin names, in the order an instance lists its nodes.
    pub pins: Vec<&'static str>,
    /// Every parameter as written, for the result.
    pub parameters: Vec<(String, f64)>,
    pub fitted: Vec<String>,
    pub assumed: Vec<String>,
    pub checks: Vec<Check>,
    pub bench: Bench,
}

impl Model {
    fn new(kind: Kind, name: &str, element: Element, pins: Vec<&'static str>) -> Self {
        Self {
            kind,
            name: name.to_string(),
            body: String::new(),
            element,
            pins,
            parameters: Vec::new(),
            fitted: Vec::new(),
            assumed: Vec::new(),
            checks: Vec::new(),
            bench: Bench::default(),
        }
    }

    /// The whole file Konnect writes.
    pub(crate) fn file(&self) -> String {
        text::file(self.kind.name(), &self.name, &self.body)
    }

    /// An instance of this model in a bench deck. `id` and `nodes` are names
    /// Konnect chose, never caller text.
    pub(crate) fn instance(&self, id: &str, nodes: &[&str]) -> String {
        let letter = match self.element {
            Element::Diode => 'D',
            Element::Subcircuit => 'X',
        };
        format!("{letter}{id} {} {}", nodes.join(" "), self.name)
    }
}

/// Fit a model of `kind` to the caller's characteristics at `temperature`.
pub(crate) fn build(
    kind: Kind,
    name: &str,
    characteristics: &[Characteristic],
    temperature: f64,
) -> Fitted<Model> {
    text::check_name(name).map_err(|reason| Invalid::new("model_name", reason))?;
    if !TEMPERATURE_RANGE.contains(&temperature) {
        return Err(Invalid::new(
            "temperature_celsius",
            format!(
                "temperature_celsius must be from {} to {}",
                TEMPERATURE_RANGE.start(),
                TEMPERATURE_RANGE.end()
            ),
        ));
    }
    let allowed = kind.quantities();
    for characteristic in characteristics {
        if !allowed
            .iter()
            .any(|(quantity, _, _)| *quantity == characteristic.quantity)
        {
            let names: Vec<&str> = allowed
                .iter()
                .map(|(quantity, _, _)| quantity.name())
                .collect();
            return Err(Invalid::new(
                characteristic.field("quantity"),
                format!(
                    "a {} takes {}, not {}",
                    kind.name(),
                    names.join(", "),
                    characteristic.quantity.name()
                ),
            ));
        }
    }
    for (quantity, least, most) in allowed {
        let count = characteristics
            .iter()
            .filter(|characteristic| characteristic.quantity == *quantity)
            .count();
        if count < *least || count > *most {
            let wanted = if least == most {
                format!("exactly {least}")
            } else {
                format!("from {least} to {most}")
            };
            return Err(Invalid::new(
                "characteristics",
                format!(
                    "a {} needs {wanted} {} values, and was given {count}",
                    kind.name(),
                    quantity.name()
                ),
            ));
        }
    }
    let mut model = match kind {
        Kind::Diode | Kind::SchottkyDiode | Kind::Led => {
            diode::plain(kind, name, characteristics, temperature)?
        }
        Kind::Zener => diode::zener(name, characteristics, temperature)?,
        Kind::Tvs | Kind::TvsBidirectional => tvs::build(kind, name, characteristics, temperature)?,
        Kind::NMosfet | Kind::PMosfet => mosfet::build(kind, name, characteristics, temperature)?,
        Kind::Npn | Kind::Pnp => bjt::build(kind, name, characteristics, temperature)?,
        Kind::OpAmp | Kind::Comparator => {
            amplifier::build(kind, name, characteristics, temperature)?
        }
        Kind::Ldo | Kind::VoltageReference => {
            regulator::build(kind, name, characteristics, temperature)?
        }
        Kind::LoadSwitch | Kind::Efuse | Kind::IdealDiode => {
            switch::build(kind, name, characteristics)?
        }
        Kind::Resistance | Kind::Capacitor | Kind::Inductor => {
            passive::build(kind, name, characteristics)?
        }
    };
    let statistics: Vec<Statistic> = characteristics.iter().map(|c| c.statistic).collect();
    if statistics
        .iter()
        .any(|statistic| *statistic != Statistic::Typical)
    {
        model.assumed.push(
            "some values are datasheet limits, not typical values, so the model sits at those \
             limits"
                .to_string(),
        );
    }
    Ok(model)
}

/// The characteristics of one quantity, in the caller's order.
pub(crate) fn of(characteristics: &[Characteristic], quantity: Quantity) -> Vec<&Characteristic> {
    characteristics
        .iter()
        .filter(|characteristic| characteristic.quantity == quantity)
        .collect()
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub(crate) fn parsed(value: Value) -> Vec<Characteristic> {
        parse_characteristics(&value).expect("test characteristics parse")
    }

    pub(crate) fn fitted(model: &Model, name: &str) -> f64 {
        model
            .parameters
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| *value)
            .unwrap_or_else(|| panic!("{name} not in {:?}", model.parameters))
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use serde_json::json;

    fn forward(current: f64, voltage: f64) -> Value {
        json!({
            "quantity": "forward_voltage",
            "value": voltage,
            "conditions": { "forward_current_amps": current },
            "statistic": "typical"
        })
    }

    #[test]
    fn every_kind_and_quantity_has_a_name_that_reads_back() {
        for kind in Kind::ALL {
            assert_eq!(Kind::from_name(kind.name()), Some(kind));
        }
        for quantity in Quantity::ALL {
            assert_eq!(Quantity::from_name(quantity.name()), Some(quantity));
        }
        assert_eq!(Kind::from_name("opamp"), None);
    }

    #[test]
    fn a_characteristic_needs_a_number_its_conditions_and_its_statistic() {
        let cases = [
            (
                json!([{ "quantity": "forward_voltage", "value": "0.6", "conditions": { "forward_current_amps": 0.01 }, "statistic": "typical" }]),
                "characteristics[0].value",
            ),
            (
                json!([{ "quantity": "forward_voltage", "value": 0.6, "statistic": "typical" }]),
                "characteristics[0].conditions.forward_current_amps",
            ),
            (
                json!([{ "quantity": "forward_voltage", "value": 0.6, "conditions": { "forward_current_amps": 0.01 } }]),
                "characteristics[0].statistic",
            ),
            (
                json!([{ "quantity": "forward_voltage", "value": 0.6, "conditions": { "forward_current_amps": 0.01, "frequency_hz": 1e6 }, "statistic": "typical" }]),
                "characteristics[0].conditions.frequency_hz",
            ),
            (
                json!([{ "quantity": "vf", "value": 0.6, "statistic": "typical" }]),
                "characteristics[0].quantity",
            ),
            (
                json!([{ "quantity": "resistance", "value": -1.0, "statistic": "typical" }]),
                "characteristics[0].value",
            ),
            (
                json!([{ "quantity": "resistance", "value": 1.0, "statistic": "typical", "note": "x" }]),
                "characteristics[0].note",
            ),
            (
                json!([{ "quantity": "resistance", "value": 1.0, "statistic": "typical", "page": "3\n*#x" }]),
                "characteristics[0].page",
            ),
            (json!({}), "characteristics"),
            (json!([]), "characteristics"),
        ];
        for (value, field) in cases {
            assert_eq!(
                parse_characteristics(&value).unwrap_err().field,
                field,
                "{value}"
            );
        }
    }

    #[test]
    fn a_capacitance_at_zero_bias_is_a_condition_but_a_leakage_at_zero_is_not() {
        let zero = parsed(json!([{
            "quantity": "junction_capacitance", "value": 4e-12,
            "conditions": { "reverse_voltage_volts": 0.0 }, "statistic": "typical"
        }]));
        assert_eq!(zero[0].condition(REVERSE_VOLTAGE), Some(0.0));
        let leakage = json!([{
            "quantity": "reverse_current", "value": 1e-6,
            "conditions": { "reverse_voltage_volts": 0.0 }, "statistic": "maximum"
        }]);
        assert_eq!(
            parse_characteristics(&leakage).unwrap_err().field,
            "characteristics[0].conditions.reverse_voltage_volts"
        );
    }

    #[test]
    fn a_kind_refuses_quantities_it_does_not_fit_and_counts_the_rest() {
        let led = parsed(json!([forward(0.02, 2.0), forward(0.005, 1.8), {
            "quantity": "breakdown_voltage", "value": 5.0,
            "conditions": { "reverse_current_amps": 1e-5 }, "statistic": "minimum"
        }]));
        let error = build(Kind::Led, "red_fit", &led, 25.0).unwrap_err();
        assert_eq!(error.field, "characteristics[2].quantity");
        let one_point = parsed(json!([forward(0.01, 0.6)]));
        let error = build(Kind::Diode, "d_one_fit", &one_point, 25.0).unwrap_err();
        assert!(
            error.reason.contains("from 2 to 16 forward_voltage"),
            "{}",
            error.reason
        );
    }

    #[test]
    fn a_bad_name_or_temperature_is_refused_before_any_fit() {
        let points = parsed(json!([forward(0.001, 0.6), forward(0.1, 0.8)]));
        assert_eq!(
            build(Kind::Diode, "1n4148", &points, 25.0)
                .unwrap_err()
                .field,
            "model_name"
        );
        assert_eq!(
            build(Kind::Diode, "sw1n4148_fit", &points, 400.0)
                .unwrap_err()
                .field,
            "temperature_celsius"
        );
    }
}
