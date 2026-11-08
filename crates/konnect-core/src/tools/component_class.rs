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
