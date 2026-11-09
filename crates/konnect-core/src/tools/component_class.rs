//! Shared classification of parts, pins and values for the review audits and
//! placement.
//!
//! Each classifier returns the rule that fired, so a finding can say *why* a
//! pin was treated as a power pin or a part as a capacitor. The KiCad data
//! (a pin's electrical type, a symbol's library name) comes first; names and
//! reference prefixes are only the fallback.

/// A component value in base units: `100n` is 1e-7, `4u7` is 4.7e-6, `2k2`
/// is 2200, `4R7` is 4.7.
///
/// Accepts SI prefixes `p n u µ μ m k K M G` (`m` is milli, `M` mega), the
/// RKM form where the prefix stands for the decimal point (`4u7`, `2k2`,
/// `4R7`), an optional space before the prefix, an optional unit (`F`, `H`,
/// `Ω`, `ohm`), and trailing ratings after a separator (`100nF/50V`,
/// `10uF 25V X7R`). Anything else, including an empty value or `DNP`, is
/// `None`, never a guess.
pub(crate) fn parse_si_value(text: &str) -> Option<f64> {
    let chars: Vec<char> = text.trim().chars().collect();
    let mut i = 0;
    let mut mantissa = String::new();
    let mut seen_dot = false;
    while let Some(&c) = chars.get(i) {
        if c.is_ascii_digit() {
            mantissa.push(c);
        } else if c == '.' && !seen_dot {
            seen_dot = true;
            mantissa.push(c);
        } else {
            break;
        }
        i += 1;
    }
    if !mantissa.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    // One optional space between the number and its prefix ("1 µF").
    if chars.get(i) == Some(&' ') && chars.get(i + 1).is_some_and(|c| prefix(*c).is_some()) {
        i += 1;
    }
    let mut multiplier = 1.0;
    if let Some(m) = chars.get(i).and_then(|c| prefix(*c)) {
        // A lone `F`/`H` is a unit, handled below; `R` only means "point".
        multiplier = m;
        i += 1;
        // RKM: the prefix replaced the decimal point.
        let mut fraction = String::new();
        while let Some(&c) = chars.get(i) {
            if !c.is_ascii_digit() {
                break;
            }
            fraction.push(c);
            i += 1;
        }
        if !fraction.is_empty() {
            if seen_dot {
                return None; // "4.7u7" is not a value
            }
            mantissa.push('.');
            mantissa.push_str(&fraction);
        }
    }
    let rest: String = chars[i..].iter().collect();
    let unit_free = ["F", "f", "H", "h", "Ω", "ohm", "Ohm", "OHM", ""]
        .iter()
        .find_map(|unit| {
            rest.strip_prefix(unit)
                .filter(|after| after.is_empty() || after.starts_with(is_separator))
        });
    unit_free?;
    let value: f64 = mantissa.parse().ok()?;
    let scaled = value * multiplier;
    scaled.is_finite().then_some(scaled)
}

fn prefix(c: char) -> Option<f64> {
    Some(match c {
        'p' => 1e-12,
        'n' => 1e-9,
        'u' | 'µ' | 'μ' => 1e-6,
        'm' => 1e-3,
        'R' | 'r' => 1.0,
        'k' | 'K' => 1e3,
        'M' => 1e6,
        'G' => 1e9,
        _ => return None,
    })
}

fn is_separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, '/' | ',' | ';' | '_' | '±' | '@' | '(')
}

/// Why a part was taken for a capacitor.
pub(crate) const CAP_BY_LIBRARY_SYMBOL: &str = "library_symbol";
pub(crate) const CAP_BY_REFERENCE_PREFIX: &str = "reference_prefix";

/// Whether a placed symbol is a capacitor, and by which rule.
///
/// The library symbol name decides first: `Device:C`, `C_Small`,
/// `C_Polarized`, `CP`, `CP1_Small` and other `C_`/`CP` names. Failing that, a
/// reference of `C` followed by a digit (`C12`). `CN1`, `CR1` and `CONN1` are
/// not capacitors.
pub(crate) fn capacitor_rule(lib_id: &str, reference: &str) -> Option<&'static str> {
    let name = lib_id.rsplit(':').next().unwrap_or(lib_id);
    if name == "C"
        || name == "CP"
        || name.starts_with("C_")
        || name.starts_with("CP_")
        || name.starts_with("CP1")
    {
        return Some(CAP_BY_LIBRARY_SYMBOL);
    }
    let mut chars = reference.chars();
    (chars.next() == Some('C') && chars.next().is_some_and(|c| c.is_ascii_digit()))
        .then_some(CAP_BY_REFERENCE_PREFIX)
}

/// Bulk capacitance: at least 10 µF.
pub(crate) const BULK_CAPACITANCE_F: f64 = 10e-6;

/// Whether `farads` is bulk capacitance. Relative tolerance, because
/// `10.0 * 1e-6` is one ulp below `10e-6`.
pub(crate) fn is_bulk_capacitance(farads: f64) -> bool {
    farads >= BULK_CAPACITANCE_F * (1.0 - 1e-9)
}

/// Why a pin was taken for a power-input pin.
pub(crate) const PIN_BY_ELECTRICAL_TYPE: &str = "electrical_type";
pub(crate) const PIN_BY_NAME_FALLBACK: &str = "name_fallback";

/// Whether a symbol pin is a power input, and by which rule.
///
/// KiCad's `power_in` decides. A pin typed `passive` or `unspecified` (what
/// a hand-drawn or imported symbol often carries) falls back to its name
/// (`VCC`, `VDD`, `3V3`, …). A pin with any other definite type (`input`,
/// `output`, `power_out`, …) is not a power input whatever it is called.
pub(crate) fn power_pin_rule(electrical_type: &str, name: &str) -> Option<&'static str> {
    match electrical_type {
        "power_in" => Some(PIN_BY_ELECTRICAL_TYPE),
        "passive" | "unspecified" | "" => is_power_pin_name(name).then_some(PIN_BY_NAME_FALLBACK),
        _ => None,
    }
}

/// The name fallback for [`power_pin_rule`].
pub(crate) fn is_power_pin_name(name: &str) -> bool {
    let upper = name.to_uppercase();
    upper.starts_with("VCC")
        || upper.starts_with("VDD")
        || upper.starts_with("VBUS")
        || upper.starts_with("V+")
        || upper.starts_with("VIN")
        || upper.starts_with("3V3")
        || upper.starts_with("5V")
        || upper.starts_with("1V")
        || upper.starts_with("2V")
        || upper == "AVCC"
        || upper == "AVDD"
        || upper == "DVCC"
        || upper == "DVDD"
        || upper.starts_with("VCAP")
        || upper.starts_with("VREF")
        || upper.contains("POWER")
        || upper.contains("PWR")
}

/// Whether a pin or net name means ground (`GND`, `AGND`, `GNDA`, `PGND`,
/// `VSS`, `VSSA`, `0V`). A power-input pin on ground needs no decoupling of
/// its own: the capacitor's other end is there.
pub(crate) fn is_ground_name(name: &str) -> bool {
    let upper = name.trim_start_matches('/').to_uppercase();
    upper.starts_with("GND") || upper.ends_with("GND") || upper.starts_with("VSS") || upper == "0V"
}

// ─── Roles for the circuit-aware review ──────────────────────────────────────
//
// Everything below classifies by what a symbol *is* (library name, reference
// prefix, pin names), never by what a net is called, and each answer names the
// rule that gave it so a finding can say why a part was taken for what it is.

/// A pin or net name with case, `~{}` overbar markup, `/` and `_` removed, so
/// `~{RE}`, `/RE` and `RE` compare equal and `CAN_H` reads `CANH`.
pub(crate) fn norm_pin(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, '~' | '{' | '}' | '/' | '_' | ' ' | '#'))
        .collect::<String>()
        .to_uppercase()
}

fn lib_name(lib_id: &str) -> &str {
    lib_id.rsplit(':').next().unwrap_or(lib_id)
}

fn ref_has_prefix_digit(reference: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| {
        reference
            .strip_prefix(p)
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| c.is_ascii_digit())
    })
}

/// A resistor: `Device:R`, `R_Small`, `R_US`, … or a reference `R<digit>`.
pub(crate) fn is_resistor(lib_id: &str, reference: &str) -> bool {
    let name = lib_name(lib_id);
    name == "R"
        || name.starts_with("R_")
        || (ref_has_prefix_digit(reference, &["R"]) && !name.starts_with("RV"))
}

/// A two-pin inductor or ferrite bead in series with a signal. A multi-pin
/// common-mode choke is not one: see [`is_common_mode_choke`].
pub(crate) fn is_series_inductor(lib_id: &str, reference: &str) -> bool {
    let name = lib_name(lib_id);
    name == "L"
        || name.starts_with("L_")
        || name.starts_with("Ferrite")
        || name.starts_with("FB")
        || name.starts_with("L_Core")
        || ref_has_prefix_digit(reference, &["L", "FB"])
}

/// A common-mode choke: a library name that says so, or an `FL`/`CMC` + digit
/// reference. Its winding pairs cannot be read from the symbol reliably, so
/// the review stops at one rather than guess which pin continues which.
pub(crate) fn is_common_mode_choke(lib_id: &str, reference: &str) -> bool {
    let name = lib_name(lib_id).to_uppercase();
    name.contains("CHOKE")
        || name.contains("COMMONMODE")
        || name.contains("COMMON_MODE")
        || name.contains("CMC")
        || ref_has_prefix_digit(reference, &["FL", "CMC"])
}

/// A jumper or solder bridge: a library name that says jumper, or a `JP` +
/// digit reference.
pub(crate) fn is_jumper(lib_id: &str, reference: &str) -> bool {
    lib_name(lib_id).to_uppercase().contains("JUMPER") || ref_has_prefix_digit(reference, &["JP"])
}

/// A test point: a `TestPoint*` symbol (KiCad keeps them in the `Connector`
/// library) or a `TP` + digit reference. It is a probe pad, not a connector.
pub(crate) fn is_test_point(lib_id: &str, reference: &str) -> bool {
    lib_name(lib_id).to_uppercase().starts_with("TESTPOINT")
        || ref_has_prefix_digit(reference, &["TP"])
}

/// A connector: a `Connector*` library symbol, or `J`/`CN`/`P`/`X` + digit
/// on a symbol whose library name says connector, header, jack or USB. Test
/// points are not connectors.
pub(crate) fn is_connector(lib_id: &str, reference: &str) -> bool {
    if is_test_point(lib_id, reference) {
        return false;
    }
    let lib_upper = lib_id.to_uppercase();
    if lib_upper.starts_with("CONNECTOR") {
        return true;
    }
    let name = lib_upper.rsplit(':').next().unwrap_or("");
    ref_has_prefix_digit(reference, &["J", "CN", "P", "X"])
        && (name.contains("CONN")
            || name.contains("HEADER")
            || name.contains("JACK")
            || name.contains("TERMINAL")
            || name.starts_with("USB_"))
}

/// A two-terminal crystal or ceramic resonator that needs external load
/// capacitors. Active oscillators (`Oscillator:`) carry their own.
pub(crate) fn is_crystal(lib_id: &str, reference: &str) -> bool {
    let name = lib_name(lib_id).to_uppercase();
    ((name.contains("CRYSTAL") || name.contains("RESONATOR"))
        && !lib_id.to_uppercase().starts_with("OSCILLATOR"))
        || (ref_has_prefix_digit(reference, &["Y"]) && name.contains("XTAL"))
}

/// What a protection device is known to be.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Protection {
    /// The family that matched, as written in the table below.
    pub family: &'static str,
    /// `Some(true)` low-capacitance (suits USB 2.0 data lines), `Some(false)`
    /// a family whose junction capacitance is far too high for one, `None`
    /// when the family spans both and the part number decides.
    pub low_capacitance: Option<bool>,
}

/// Low-capacitance TVS/ESD arrays (a few pF per line).
const LOW_CAP_ESD: &[&str] = &[
    "USBLC6", "TPD1E", "TPD2E", "TPD3E", "TPD4E", "TPD6E", "TPD8E", "PRTR5V0", "SRV05", "IP4",
    "ESDA6V1", "ESD9", "ESD5V0S", "RCLAMP05", "RCLAMP3", "SP0504", "SP3003", "NUP4",
];
/// Power-line TVS diodes: hundreds to thousands of pF.
const HIGH_CAP_TVS: &[&str] = &[
    "SMAJ", "SMBJ", "SMCJ", "SMDJ", "P6KE", "P6SMB", "1.5KE", "SM8S", "SMF", "SM712",
];

/// A TVS/ESD protection device, by its value and library symbol. The family
/// tables are what the review can vouch for; a protection part outside them
/// is still recognised (`low_capacitance: None`) when its symbol says
/// `TVS`/`ESD`, so it counts as protection but not as a USB-grade one.
pub(crate) fn protection_device(lib_id: &str, value: &str) -> Option<Protection> {
    let value_upper = value.to_uppercase();
    let name_upper = lib_name(lib_id).to_uppercase();
    for family in LOW_CAP_ESD {
        if value_upper.contains(family) || name_upper.contains(family) {
            return Some(Protection {
                family,
                low_capacitance: Some(true),
            });
        }
    }
    for family in HIGH_CAP_TVS {
        if value_upper.contains(family) || name_upper.contains(family) {
            return Some(Protection {
                family,
                low_capacitance: Some(false),
            });
        }
    }
    let by_symbol = name_upper.contains("TVS")
        || name_upper.contains("ESD")
        || name_upper.contains("TRANSIL")
        || lib_id.to_uppercase().starts_with("POWER_PROTECTION");
    let by_value = value_upper.contains("TVS") || value_upper.starts_with("ESD");
    (by_symbol || by_value).then_some(Protection {
        family: "symbol_name",
        low_capacitance: None,
    })
}

/// The pin names of one part, normalised, with their indexes.
pub(crate) struct PinNames<'a> {
    names: Vec<(usize, String, &'a str)>,
}

impl<'a> PinNames<'a> {
    /// `pins` is `(name, electrical_type)` in a stable order.
    pub(crate) fn new(pins: &'a [(String, String)]) -> Self {
        Self {
            names: pins
                .iter()
                .enumerate()
                .map(|(i, (n, t))| (i, norm_pin(n), t.as_str()))
                .collect(),
        }
    }

    /// The first pin whose normalised name is one of `names`. The candidates
    /// are normalised too, so `FB/VO` in a list meets a pin named `FB/VO`.
    pub(crate) fn find(&self, names: &[&str]) -> Option<usize> {
        let wanted: Vec<String> = names.iter().map(|n| norm_pin(n)).collect();
        self.names
            .iter()
            .find(|(_, n, _)| wanted.contains(n))
            .map(|(i, _, _)| *i)
    }

    pub(crate) fn all(&self, names: &[&str]) -> Vec<usize> {
        let wanted: Vec<String> = names.iter().map(|n| norm_pin(n)).collect();
        self.names
            .iter()
            .filter(|(_, n, _)| wanted.contains(n))
            .map(|(i, _, _)| *i)
            .collect()
    }
}

pub(crate) const CAN_H: &[&str] = &["CANH", "CANHIGH"];
pub(crate) const CAN_L: &[&str] = &["CANL", "CANLOW"];
pub(crate) const RS485_A: &[&str] = &["A", "A+", "RS485A", "DA"];
pub(crate) const RS485_B: &[&str] = &["B", "B-", "RS485B", "DB"];
pub(crate) const RS485_DIR: &[&str] = &["DE", "RE", "DERE", "REDE", "DEN", "REN"];
pub(crate) const MOSFET_PINS: [&[&str]; 3] = [&["G", "GATE"], &["D", "DRAIN"], &["S", "SOURCE"]];
pub(crate) const DRIVER_HIGH_OUT: &[&str] = &["HO", "GH", "HDRV", "DRVH", "HG", "OUTH", "TG"];
pub(crate) const DRIVER_LOW_OUT: &[&str] = &["LO", "GL", "LDRV", "DRVL", "LG", "OUTL", "BG"];
pub(crate) const DRIVER_BOOT: &[&str] = &["VB", "BST", "BOOT", "HB", "CBOOT", "BOOTH", "BOOT1"];
pub(crate) const DRIVER_SWITCH: &[&str] = &["VS", "HS", "SW", "SH", "PHASE", "LX", "SWH"];
pub(crate) const SENSE_IN_P: &[&str] = &["IN+", "INP", "VIN+", "+IN", "INPUT+", "IS+"];
pub(crate) const SENSE_IN_N: &[&str] = &["IN-", "INN", "VIN-", "-IN", "INPUT-", "IS-"];
pub(crate) const FEEDBACK_PINS: &[&str] = &["FB", "VFB", "ADJ", "ADJUST", "VADJ", "FBK", "FB/VO"];
pub(crate) const UART_TX: &[&str] = &["TX", "TXD", "UTX", "TXO", "SOUT"];
pub(crate) const UART_RX: &[&str] = &["RX", "RXD", "URX", "RXI", "SIN"];
pub(crate) const USB_DP: &[&str] = &["D+", "DP", "USBD+", "USBDP", "USBP", "DPLUS"];
pub(crate) const USB_DM: &[&str] = &["D-", "DM", "USBD-", "USBDM", "USBN", "DMINUS"];
pub(crate) const ADC_REF_PINS: &[&str] =
    &["VREF", "VREF+", "VREFP", "AREF", "REF", "VREFH", "REFP"];

/// Whether a normalised pin name is a UART TX (`dir = 0`) or RX (`dir = 1`) pin,
/// accepting a peripheral prefix (`USART1TX`, `UART2RXD`) but not `TXEN`.
pub(crate) fn uart_direction(norm: &str) -> Option<u8> {
    let tx = |s: &str| UART_TX.contains(&s);
    let rx = |s: &str| UART_RX.contains(&s);
    if tx(norm) {
        return Some(0);
    }
    if rx(norm) {
        return Some(1);
    }
    for suffix in ["TXD", "TX"] {
        if let Some(stem) = norm.strip_suffix(suffix) {
            if stem.contains("UART")
                || stem.contains("USART")
                || stem.contains("SERIAL")
                || stem.contains("COM")
            {
                return Some(0);
            }
        }
    }
    for suffix in ["RXD", "RX"] {
        if let Some(stem) = norm.strip_suffix(suffix) {
            if stem.contains("UART")
                || stem.contains("USART")
                || stem.contains("SERIAL")
                || stem.contains("COM")
            {
                return Some(1);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() <= b.abs() * 1e-9)
    }

    #[test]
    fn values_parse_in_every_common_notation() {
        for (text, expect) in [
            ("100n", 100e-9),
            ("100nF", 100e-9),
            ("0.1uF", 0.1e-6),
            ("0.1µF", 0.1e-6),
            ("0.1μF", 0.1e-6),
            ("10µ", 10e-6),
            ("10u", 10e-6),
            ("4u7", 4.7e-6),
            ("4.7uF", 4.7e-6),
            ("1 µF", 1e-6),
            ("22pF", 22e-12),
            ("100nF/50V", 100e-9),
            ("10uF 25V X7R", 10e-6),
            ("1000uF", 1000e-6),
            ("2k2", 2200.0),
            ("4R7", 4.7),
            ("10k", 10e3),
            ("1M", 1e6),
            ("1m", 1e-3),
            ("47", 47.0),
            ("100Ω", 100.0),
            ("1 kohm", 1e3),
        ] {
            assert!(
                close(parse_si_value(text), expect),
                "{text}: {:?}",
                parse_si_value(text)
            );
        }
    }

    #[test]
    fn non_values_are_refused_not_guessed() {
        for text in ["", "DNP", "C", "uF", "4.7u7", "100nX", "LM317", "~"] {
            assert_eq!(parse_si_value(text), None, "{text}");
        }
    }

    #[test]
    fn the_micro_sign_reaches_bulk() {
        // The old matcher upper-cased U+00B5 to Greek capital mu, so "47µ"
        // never read as bulk.
        assert!(is_bulk_capacitance(parse_si_value("47µ").unwrap()));
        assert!(!is_bulk_capacitance(parse_si_value("4u7").unwrap()));
        // 10 × 1e-6 rounds one ulp under 10e-6.
        assert!(is_bulk_capacitance(parse_si_value("10uF").unwrap()));
    }

    #[test]
    fn capacitors_are_found_by_symbol_then_reference() {
        assert_eq!(
            capacitor_rule("Device:C", "X1"),
            Some(CAP_BY_LIBRARY_SYMBOL)
        );
        assert_eq!(
            capacitor_rule("Device:C_Small", "C3"),
            Some(CAP_BY_LIBRARY_SYMBOL)
        );
        assert_eq!(
            capacitor_rule("Device:CP1_Small", "C3"),
            Some(CAP_BY_LIBRARY_SYMBOL)
        );
        assert_eq!(
            capacitor_rule("my:CAP_0603", "C7"),
            Some(CAP_BY_REFERENCE_PREFIX)
        );
        for (lib, reference) in [
            ("Connector:Conn_01x02", "CN1"),
            ("Device:D", "CR1"),
            ("Connector:Conn_01x02", "CONN1"),
            ("Device:R", "R1"),
        ] {
            assert_eq!(capacitor_rule(lib, reference), None, "{lib} {reference}");
        }
    }

    #[test]
    fn pin_type_decides_and_names_only_fill_in() {
        assert_eq!(
            power_pin_rule("power_in", "EXT"),
            Some(PIN_BY_ELECTRICAL_TYPE)
        );
        assert_eq!(power_pin_rule("passive", "VCC"), Some(PIN_BY_NAME_FALLBACK));
        assert_eq!(
            power_pin_rule("unspecified", "3V3"),
            Some(PIN_BY_NAME_FALLBACK)
        );
        // A typed input or output named like a rail is not a power input.
        assert_eq!(power_pin_rule("input", "VREF"), None);
        assert_eq!(power_pin_rule("power_out", "VCC"), None);
        assert_eq!(power_pin_rule("passive", "PA3"), None);
    }

    #[test]
    fn ground_names() {
        for name in ["GND", "AGND", "GNDA", "PGND", "VSS", "VSSA", "0V", "/GND"] {
            assert!(is_ground_name(name), "{name}");
        }
        for name in ["VCC", "+3V3", "VEE", "-15V", "V-"] {
            assert!(!is_ground_name(name), "{name}");
        }
    }
}
