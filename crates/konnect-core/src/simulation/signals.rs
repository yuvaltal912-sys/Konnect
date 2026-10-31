//! What a caller may ask to see, and where the engine keeps it.
//!
//! Nets come as KiCad shows them and parts by reference. A signal never enters
//! the deck: it only picks vectors out of the results, or names an element the
//! deck already holds.

use super::kicad_netlist::{ElementEntry, NetEntry};
use super::preflight;
use super::raw::Plot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Signal {
    /// `v(NET)` or `v(NET1,NET2)`, held whole: a net name may contain a comma.
    Voltage(String),
    Current {
        reference: String,
        pin: Option<String>,
    },
    Power(String),
}

impl Signal {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let bad = || {
            format!("Not a signal: {text}. Use v(NET), v(NET1,NET2), i(REF), i(REF:PIN) or p(REF)")
        };
        let text = text.trim();
        let (kind, rest) = text.split_at_checked(1).ok_or_else(bad)?;
        let inner = rest
            .strip_prefix('(')
            .and_then(|rest| rest.strip_suffix(')'))
            .filter(|inner| !inner.is_empty())
            .ok_or_else(bad)?;
        match kind {
            "v" | "V" => Ok(Self::Voltage(inner.to_string())),
            "i" | "I" => {
                let (reference, pin) = match inner.split_once(':') {
                    Some((reference, pin)) if !pin.is_empty() => (reference, Some(pin)),
                    Some(_) => return Err(bad()),
                    None => (inner, None),
                };
                Ok(Self::Current {
                    reference: reference.to_string(),
                    pin: pin.map(str::to_owned),
                })
            }
            "p" | "P" => Ok(Self::Power(inner.to_string())),
            _ => Err(bad()),
        }
    }

    pub(crate) fn unit(&self) -> &'static str {
        match self {
            Self::Voltage(_) => "V",
            Self::Current { .. } => "A",
            Self::Power(_) => "W",
        }
    }
}

/// The netlist's names, for turning a signal into the engine's.
#[derive(Clone, Copy)]
pub(crate) struct Names<'a> {
    pub nets: &'a [NetEntry],
    pub elements: &'a [ElementEntry],
}

/// Why a signal has no values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotFound {
    /// The pins this part does have, for `i(M1)` where the data is on `i(m1:d)`.
    pub pins: Vec<String>,
}

/// A signal's values, and any nets that its node is shared with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Resolved {
    pub values: Vec<f64>,
    /// KiCad names for one engine node, which ngspice cannot tell apart.
    pub shared_nets: Vec<String>,
}

/// Two vectors closer than this share of their peak are one signal. An inductor's
/// branch and its current probe are separate unknowns, so they differ by rounding.
const SAME_SIGNAL_SHARE: f64 = 1e-6;
/// Below the engine's current tolerance, two vectors are both only rounding.
const SAME_SIGNAL_FLOOR: f64 = 1e-12;

fn same_signal(first: &[f64], other: &[f64]) -> bool {
    let peak = first
        .iter()
        .fold(0.0_f64, |peak, value| peak.max(value.abs()));
    let limit = (peak * SAME_SIGNAL_SHARE).max(SAME_SIGNAL_FLOOR);
    first.len() == other.len() && first.iter().zip(other).all(|(a, b)| (a - b).abs() <= limit)
}

/// The vector of this name. Two can share a name, and a device with a branch
/// reports its current twice under one. The same data twice is one signal;
/// different data is not ours to choose between.
fn vector(plot: &Plot, name: &str) -> Option<Vec<f64>> {
    let mut matches = plot
        .variables
        .iter()
        .enumerate()
        .filter(|(_, variable)| variable.name.eq_ignore_ascii_case(name))
        .map(|(index, _)| plot.real(index).collect::<Vec<f64>>());
    let first = matches.next()?;
    matches
        .all(|other| same_signal(&first, &other))
        .then_some(first)
}

fn node_of(name: &str, nets: &[NetEntry]) -> String {
    let net = nets.iter().find(|net| net.kicad_name == name);
    preflight::engine_node(net.map_or(name, |net| net.spice_name.as_str()))
}

fn nets_on(node: &str, nets: &[NetEntry]) -> Vec<String> {
    nets.iter()
        .filter(|net| preflight::engine_node(&net.spice_name) == node)
        .map(|net| net.kicad_name.clone())
        .collect()
}

fn node_voltage(plot: &Plot, node: &str) -> Option<Vec<f64>> {
    if node == "0" {
        return Some(vec![0.0; plot.points]);
    }
    vector(plot, &format!("v({node})"))
}

/// One node, or else two split at a comma. A whole name that is a net wins,
/// because a net name may hold a comma, and so does a split where both sides
/// resolve.
fn voltage(plot: &Plot, text: &str, nets: &[NetEntry]) -> Option<Resolved> {
    let known = |name: &str| nets.iter().any(|net| net.kicad_name == name);
    let shared = |node: &str| match nets_on(node, nets) {
        names if names.len() > 1 => names,
        _ => Vec::new(),
    };
    if known(text) || !text.contains(',') {
        let whole = node_of(text, nets);
        return Some(Resolved {
            values: node_voltage(plot, &whole)?,
            shared_nets: shared(&whole),
        });
    }
    text.match_indices(',').find_map(|(comma, _)| {
        let first = node_of(text[..comma].trim(), nets);
        let second = node_of(text[comma + 1..].trim(), nets);
        let plus = node_voltage(plot, &first)?;
        let minus = node_voltage(plot, &second)?;
        let mut shared_nets = shared(&first);
        shared_nets.extend(shared(&second));
        Some(Resolved {
            values: plus.iter().zip(&minus).map(|(p, m)| p - m).collect(),
            shared_nets,
        })
    })
}

/// The deck's name for a part: `BT1` is the source `VBT1`. A testbench element
/// has no reference, so its own name stands, even over an excluded part it
/// replaces.
fn element_name<'a>(reference: &'a str, elements: &'a [ElementEntry]) -> &'a str {
    elements
        .iter()
        .find(|element| element.reference == reference && !element.excluded)
        .and_then(|element| element.element_name.as_deref())
        .unwrap_or(reference)
}

fn pins_of(plot: &Plot, element: &str) -> Vec<String> {
    let prefix = format!("i({}:", element.to_ascii_lowercase());
    let names = plot.variables.iter().map(|variable| &variable.name);
    let pins = names.filter_map(|name| name.strip_prefix(&prefix)?.strip_suffix(')'));
    pins.map(str::to_owned).collect()
}

/// Find a signal in a finished run.
pub(crate) fn resolve(plot: &Plot, signal: &Signal, names: Names) -> Result<Resolved, NotFound> {
    let plain = |values: Option<Vec<f64>>| {
        values.map(|values| Resolved {
            values,
            shared_nets: Vec::new(),
        })
    };
    let found = match signal {
        Signal::Voltage(text) => voltage(plot, text, names.nets),
        Signal::Current { reference, pin } => {
            let element = preflight::engine_node(element_name(reference, names.elements));
            let name = match pin {
                Some(pin) => format!("i({element}:{pin})"),
                None => format!("i({element})"),
            };
            plain(vector(plot, &name))
        }
        Signal::Power(reference) => {
            let element = preflight::engine_node(element_name(reference, names.elements));
            plain(vector(plot, &format!("{element}:power")))
        }
    };
    found.ok_or_else(|| NotFound {
        pins: match signal {
            Signal::Current { reference, .. } => pins_of(
                plot,
                &preflight::engine_node(element_name(reference, names.elements)),
            ),
            _ => Vec::new(),
        },
    })
}

/// `.probe p(NAME)` for each part asked for. The name comes from the deck, so no
/// caller text reaches the engine. `.probe` splices hidden sources into the
/// circuit, so this runs only for parts someone asked about, as KiCad's own
/// loop does for all of them. ngspice ignores code models here, and a probe on a
/// coupling stops the run, so neither gets one and they read as `signal_not_found`.
pub(crate) fn power_probes<'a>(
    signals: impl Iterator<Item = &'a Signal>,
    names: Names,
    deck_elements: &[&str],
) -> Vec<String> {
    let mut probes: Vec<String> = Vec::new();
    for signal in signals {
        let Signal::Power(reference) = signal else {
            continue;
        };
        let wanted = element_name(reference, names.elements);
        let in_deck = deck_elements
            .iter()
            .find(|name| name.eq_ignore_ascii_case(wanted));
        let Some(name) = in_deck.filter(|name| probe_safe(name)) else {
            continue;
        };
        let probe = format!(".probe p({name})");
        if !probes.contains(&probe) {
            probes.push(probe);
        }
    }
    probes
}

fn probe_safe(name: &str) -> bool {
    let no_power = name.starts_with(['a', 'A', 'k', 'K']);
    !no_power && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::raw;

    fn plot(names: &[&str], rows: &[&[f64]]) -> Plot {
        let mut text = format!(
            "Title: t\nPlotname: Transient Analysis\nFlags: real\nNo. Variables: {}\n\
             No. Points: {}\nVariables:\n",
            names.len(),
            rows.len()
        );
        for (index, name) in names.iter().enumerate() {
            text.push_str(&format!("\t{index}\t{name}\tvoltage\n"));
        }
        text.push_str("Values:\n");
        for (point, row) in rows.iter().enumerate() {
            text.push_str(&format!(" {point}"));
            for value in *row {
                text.push_str(&format!("\t{value:e}\n"));
            }
        }
        raw::parse(text.as_bytes()).unwrap().remove(0)
    }

    fn net(kicad_name: &str, spice_name: &str) -> NetEntry {
        NetEntry {
            kicad_name: kicad_name.to_string(),
            spice_name: spice_name.to_string(),
            in_deck: true,
            pin_count: 2,
        }
    }

    fn element(reference: &str, name: &str) -> ElementEntry {
        ElementEntry {
            reference: reference.to_string(),
            element_name: Some(name.to_string()),
            modelled: true,
            excluded: false,
        }
    }

    fn values(plot: &Plot, text: &str, names: Names) -> Result<Vec<f64>, NotFound> {
        let signal = Signal::parse(text).unwrap();
        resolve(plot, &signal, names).map(|resolved| resolved.values)
    }

    const NONE: Names = Names {
        nets: &[],
        elements: &[],
    };

    #[test]
    fn the_grammar_takes_every_form_and_keeps_a_net_name_whole() {
        assert_eq!(Signal::parse("v(/OUT)"), Ok(Signal::Voltage("/OUT".into())));
        assert_eq!(Signal::parse("V(a,b)"), Ok(Signal::Voltage("a,b".into())));
        assert_eq!(
            Signal::parse("v(Net-(C1-Pad1))"),
            Ok(Signal::Voltage("Net-(C1-Pad1)".into()))
        );
        assert_eq!(
            Signal::parse("i(BT1)"),
            Ok(Signal::Current {
                reference: "BT1".into(),
                pin: None
            })
        );
        assert_eq!(
            Signal::parse(" I(M1:d) "),
            Ok(Signal::Current {
                reference: "M1".into(),
                pin: Some("d".into())
            })
        );
        assert_eq!(Signal::parse("p(R1)"), Ok(Signal::Power("R1".into())));
    }

    #[test]
    fn anything_else_is_not_a_signal() {
        for text in [
            "", "v", "v()", "x(a)", "v(a", "va)", "i(m1:)", "שׁ(a)", "vv(a)",
        ] {
            assert!(Signal::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn a_net_is_looked_up_by_the_name_kicad_shows() {
        let nets = [net("/VOUT", "/VOUT"), net("Net-(C1-Pad1)", "Net-_C1-Pad1_")];
        let names = Names {
            nets: &nets,
            elements: &[],
        };
        let plot = plot(
            &["time", "v(/vout)", "v(net-_c1-pad1_)"],
            &[&[0.0, 1.0, 2.0]],
        );
        assert_eq!(values(&plot, "v(/VOUT)", names), Ok(vec![1.0]));
        assert_eq!(values(&plot, "v(Net-(C1-Pad1))", names), Ok(vec![2.0]));
    }

    #[test]
    fn a_hebrew_net_reads_the_node_the_engine_made_of_it() {
        let nets = [net("/\u{5e9}", "/\u{5e9}")];
        let names = Names {
            nets: &nets,
            elements: &[],
        };
        let plot = plot(&["time", "v(/__)"], &[&[0.0, 3.0]]);
        assert_eq!(values(&plot, "v(/\u{5e9})", names), Ok(vec![3.0]));
    }

    #[test]
    fn two_nets_on_one_node_are_named_rather_than_guessed_between() {
        let nets = [net("/\u{5e9}", "/\u{5e9}"), net("/\u{5ea}", "/\u{5ea}")];
        let names = Names {
            nets: &nets,
            elements: &[],
        };
        let plot = plot(&["time", "v(/__)"], &[&[0.0, 3.0]]);
        let signal = Signal::parse("v(/\u{5e9})").unwrap();
        let resolved = resolve(&plot, &signal, names).unwrap();
        assert_eq!(resolved.shared_nets, ["/\u{5e9}", "/\u{5ea}"]);
    }

    #[test]
    fn a_difference_is_worked_out_here_and_ground_is_zero() {
        let plot = plot(
            &["time", "v(a)", "v(b)"],
            &[&[0.0, 5.0, 2.0], &[1.0, 7.0, 3.0]],
        );
        assert_eq!(values(&plot, "v(a,b)", NONE), Ok(vec![3.0, 4.0]));
        assert_eq!(values(&plot, "v(a, 0)", NONE), Ok(vec![5.0, 7.0]));
        assert_eq!(values(&plot, "v(GND,b)", NONE), Ok(vec![-2.0, -3.0]));
        assert_eq!(values(&plot, "v(0)", NONE), Ok(vec![0.0, 0.0]));
    }

    #[test]
    fn a_net_whose_name_holds_a_comma_is_not_a_difference() {
        let nets = [net("/A,B", "/A_B")];
        let names = Names {
            nets: &nets,
            elements: &[],
        };
        let plot = plot(&["time", "v(/a_b)"], &[&[0.0, 9.0]]);
        assert_eq!(values(&plot, "v(/A,B)", names), Ok(vec![9.0]));
    }

    #[test]
    fn a_part_is_found_through_its_deck_name() {
        let elements = [element("BT1", "VBT1")];
        let names = Names {
            nets: &[],
            elements: &elements,
        };
        let plot = plot(&["time", "i(vbt1)", "vbt1:power"], &[&[0.0, -2.0, 24.0]]);
        assert_eq!(values(&plot, "i(BT1)", names), Ok(vec![-2.0]));
        assert_eq!(values(&plot, "p(BT1)", names), Ok(vec![24.0]));
    }

    #[test]
    fn a_testbench_element_is_found_by_its_own_name() {
        let plot = plot(&["time", "i(vbat)"], &[&[0.0, -2.0]]);
        assert_eq!(values(&plot, "i(VBAT)", NONE), Ok(vec![-2.0]));
    }

    #[test]
    fn a_pin_current_is_named_and_a_missing_pin_lists_the_real_ones() {
        let plot = plot(&["time", "i(m1:d)", "i(m1:g)"], &[&[0.0, 1.0, 2.0]]);
        assert_eq!(values(&plot, "i(M1:d)", NONE), Ok(vec![1.0]));
        let missing = values(&plot, "i(M1)", NONE).unwrap_err();
        assert_eq!(missing.pins, ["d", "g"]);
    }

    #[test]
    fn the_same_data_twice_is_one_signal_and_different_data_is_none() {
        let same = plot(
            &["time", "i(l1)", "i(l1)"],
            &[&[0.0, 1.0, 1.0], &[1.0, 2.0, 2.0]],
        );
        assert_eq!(values(&same, "i(L1)", NONE), Ok(vec![1.0, 2.0]));
        let bridge = plot(
            &["time", "i(a1)", "i(a1)"],
            &[&[0.0, 1.0, 1.0], &[1.0, 2.0, 3.0]],
        );
        assert!(values(&bridge, "i(A1)", NONE).is_err());
    }

    #[test]
    fn an_inductors_two_vectors_differ_by_rounding_and_are_one_signal() {
        let plot = plot(
            &["time", "i(l1)", "i(l1)"],
            &[&[0.0, 1.0, 1.0 + 1e-10], &[1.0, 2.0, 2.0]],
        );
        assert_eq!(values(&plot, "i(L1)", NONE), Ok(vec![1.0, 2.0]));
    }

    #[test]
    fn two_vectors_of_nothing_but_rounding_are_one_signal() {
        let plot = plot(
            &["time", "i(l1)", "i(l1)"],
            &[&[0.0, 1e-15, -2e-15], &[1.0, 2e-15, 1e-15]],
        );
        assert_eq!(values(&plot, "i(L1)", NONE), Ok(vec![1e-15, 2e-15]));
    }

    #[test]
    fn a_stand_in_for_an_excluded_part_answers_to_its_own_name() {
        let elements = [ElementEntry {
            excluded: true,
            ..element("Q1", "MQ1")
        }];
        let names = Names {
            nets: &[],
            elements: &elements,
        };
        let plot = plot(&["time", "i(q1:c)", "i(mq1:d)"], &[&[0.0, 7.0, 9.0]]);
        assert_eq!(values(&plot, "i(Q1:c)", names), Ok(vec![7.0]));
        let asked = [Signal::Power("Q1".into())];
        assert_eq!(power_probes(asked.iter(), names, &["Q1"]), [".probe p(Q1)"]);
    }

    #[test]
    fn a_hebrew_reference_reads_the_name_the_engine_made_of_it() {
        let plot = plot(&["time", "i(r__1)"], &[&[0.0, 4.0]]);
        assert_eq!(values(&plot, "i(R\u{5d0}1)", NONE), Ok(vec![4.0]));
    }

    #[test]
    fn a_difference_finds_the_comma_that_splits_two_real_nets() {
        let nets = [net("/A,B", "/A_B")];
        let names = Names {
            nets: &nets,
            elements: &[],
        };
        let plot = plot(&["time", "v(/a_b)", "v(c)"], &[&[0.0, 5.0, 2.0]]);
        assert_eq!(values(&plot, "v(/A,B,c)", names), Ok(vec![3.0]));
    }

    #[test]
    fn a_signal_the_run_does_not_hold_is_not_found() {
        let plot = plot(&["time", "v(a)"], &[&[0.0, 1.0]]);
        for text in ["v(zzz)", "i(R9)", "p(R9)", "v(a,zzz)"] {
            assert!(values(&plot, text, NONE).is_err(), "{text}");
        }
    }

    #[test]
    fn a_probe_line_takes_its_name_from_the_deck_and_never_from_the_caller() {
        let elements = [element("BT1", "VBT1")];
        let names = Names {
            nets: &[],
            elements: &elements,
        };
        let asked = [
            Signal::Power("BT1".into()),
            Signal::Power("r1".into()),
            Signal::Power("BT1".into()),
            Signal::Power("U1".into()),
            Signal::Power("r9)\n.control".into()),
            Signal::Voltage("a".into()),
        ];
        let deck = ["R1", "VBT1", "A1", "u1)x"];
        assert_eq!(
            power_probes(asked.iter(), names, &deck),
            [".probe p(VBT1)", ".probe p(R1)"]
        );
    }

    #[test]
    fn a_code_model_or_coupling_gets_no_probe_and_a_strange_name_is_left_out() {
        let asked = [
            Signal::Power("A1".into()),
            Signal::Power("K1".into()),
            Signal::Power("r)1".into()),
        ];
        assert!(power_probes(asked.iter(), NONE, &["A1", "K1", "r)1"]).is_empty());
    }
}
