//! Checks on the deck that would run, for what the engine accepts in silence.
//!
//! A circuit with no ground, nets that merge, or a mistyped node all run and
//! return believable numbers. So this refuses what cannot give a true result,
//! and warns about what the result depends on. A check that could not see the
//! whole circuit says so, because an unrun check must never read as a pass.

use super::element::{self, Circuit, DcPaths, Element, Groups, DIGITAL_MODELS};
use super::flatten::{Compatibility, FlatDeck};
use super::kicad_netlist::{ElementEntry, KiCadNetlist, NetEntry};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

const MAX_NAMED_ELEMENTS: usize = 8;
const GROUND_LIKE_WORDS: [&str; 5] = ["gnd", "vss", "ground", "earth", "agnd"];
const PLAIN_PARTS: [&str; 5] = ["R", "C", "L", "CP", "CP1"];
const PLAIN_PART_WORDS: [&str; 5] = ["Small", "US", "Polarized", "Device", "1"];
const SOURCE_KEYWORDS: [&str; 15] = [
    "dc", "ac", "distof1", "distof2", "sin", "sine", "pulse", "pwl", "exp", "sffm", "am",
    "trnoise", "trrandom", "external", "portnum",
];
const SCALES: [(&str, f64); 11] = [
    ("meg", 1e6),
    ("mil", 25.4e-6),
    ("t", 1e12),
    ("g", 1e9),
    ("k", 1e3),
    ("m", 1e-3),
    ("u", 1e-6),
    ("n", 1e-9),
    ("p", 1e-12),
    ("f", 1e-15),
    ("a", 1e-18),
];
/// Largest believable values, above which a scale was lost or rewritten. A
/// supercapacitor trips this, and it is only a warning.
const MAX_CAPACITANCE: f64 = 1.0;
const MAX_INDUCTANCE: f64 = 1.0;
const MAX_SOURCE: f64 = 1e4;
const VALUE_KEYS: [&str; 8] = [
    "r",
    "resistance",
    "c",
    "capacitance",
    "l",
    "inductance",
    "q",
    "flux",
];
const WAVEFORM_KEYWORDS: [&str; 10] = [
    "sin", "sine", "pulse", "pwl", "exp", "sffm", "am", "trnoise", "trrandom", "distof1",
];
/// What ends a value token for ngspice, so what follows is read as a new number.
const TOKEN_ENDS: [char; 6] = ['-', '+', '/', '*', '^', '('];
/// Parts whose library default stands in for a value that must be chosen.
const GENERIC_PARTS: [&str; 2] = ["varistor", "potentiometer"];
const PULSE_VALUES: usize = 7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Check {
    pub name: &'static str,
    pub ran: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub items: Vec<String>,
}

impl Check {
    fn passed(name: &'static str) -> Self {
        Self {
            name,
            ran: true,
            passed: Some(true),
            code: None,
            reason: None,
            items: Vec::new(),
        }
    }

    fn failed(name: &'static str, code: &'static str, items: Vec<String>) -> Self {
        Self {
            passed: Some(false),
            code: Some(code),
            items,
            ..Self::passed(name)
        }
    }

    fn not_run(name: &'static str, code: &'static str, reason: String) -> Self {
        Self {
            name,
            ran: false,
            passed: None,
            code: Some(code),
            reason: Some(reason),
            items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Warning {
    pub code: &'static str,
    pub detail: &'static str,
    pub items: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SymbolPin {
    pub pin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_type: Option<String>,
    pub net: String,
    /// Null when the deck does not name this net, never a guess.
    pub spice_net: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct UnmodelledSymbol {
    pub reference: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lib_id: Option<String>,
    pub pins: Vec<SymbolPin>,
    pub suggested_fix: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Preflight {
    pub checks: Vec<Check>,
    pub warnings: Vec<Warning>,
    pub unmodelled_symbols: Vec<UnmodelledSymbol>,
    /// The circuit has digital or other event-driven nodes. ngspice stops with a
    /// fatal error if `.probe alli` meets one.
    pub event_nodes: bool,
}

pub(crate) struct Inputs<'a> {
    pub deck: &'a FlatDeck,
    pub compatibility: Compatibility,
    pub kicad: &'a KiCadNetlist,
    pub nets: &'a [NetEntry],
    pub elements: &'a [ElementEntry],
    pub allow_digital_models: bool,
    pub code_model_count: usize,
    pub unmatched_exclusions: &'a [String],
    pub has_testbench_lines: bool,
}

pub(crate) fn run(inputs: &Inputs<'_>) -> Preflight {
    let circuit = element::read(inputs.deck, inputs.compatibility.dollar_is_comment());
    let unmodelled_symbols = unmodelled_symbols(inputs);
    let reads_shorthand = inputs.compatibility.reads_value_shorthand();
    let mut checks = vec![
        model_coverage(&unmodelled_symbols),
        deck_structure(inputs.deck),
        component_values(&circuit, reads_shorthand),
        ground(&circuit, inputs.nets),
        net_names(inputs),
        element_names(&circuit),
        connectivity(&circuit, unmodelled_symbols.len()),
        code_models(&circuit, inputs.allow_digital_models),
    ];
    if inputs.has_testbench_lines {
        checks.push(testbench_nodes(&circuit, inputs.nets));
    }
    Preflight {
        event_nodes: has_event_nodes(&circuit),
        checks,
        warnings: warnings(inputs, &circuit, unmodelled_symbols.len()),
        unmodelled_symbols,
    }
}

fn model_coverage(symbols: &[UnmodelledSymbol]) -> Check {
    if symbols.is_empty() {
        return Check::passed("model_coverage");
    }
    let references = symbols.iter().map(|s| s.reference.clone()).collect();
    Check::failed("model_coverage", "unmodelled_symbols", references)
}

fn is_ground_net(net: &str) -> bool {
    let name = net.to_lowercase();
    name == "gnd" || name == "0"
}

/// Symbols that put a line in the deck, modelled or not.
fn live_references<'a>(inputs: &Inputs<'a>) -> HashSet<&'a str> {
    inputs
        .elements
        .iter()
        .filter(|element| element.element_name.is_some() && !element.excluded)
        .map(|element| element.reference.as_str())
        .collect()
}

fn unmodelled_symbols(inputs: &Inputs<'_>) -> Vec<UnmodelledSymbol> {
    let live = live_references(inputs);
    inputs
        .elements
        .iter()
        .filter(|element| element.element_name.is_some() && !element.modelled && !element.excluded)
        .filter_map(|element| {
            let component = inputs
                .kicad
                .components
                .iter()
                .find(|component| component.reference == element.reference)?;
            let mut pins = Vec::new();
            let mut signal_pins_in_use = 0;
            for net in &inputs.kicad.nets {
                for pin in net
                    .pins
                    .iter()
                    .filter(|pin| pin.reference == element.reference)
                {
                    let entry = inputs
                        .nets
                        .iter()
                        .find(|entry| entry.kicad_name == net.name);
                    pins.push(SymbolPin {
                        pin: pin.pin.clone(),
                        pin_name: pin.pin_name.clone(),
                        pin_type: pin.pin_type.clone(),
                        net: net.name.clone(),
                        spice_net: entry
                            .filter(|entry| entry.in_deck)
                            .map(|entry| entry.spice_name.clone()),
                    });
                    let shared = net.pins.iter().any(|other| {
                        other.reference != element.reference
                            && live.contains(other.reference.as_str())
                    });
                    if shared && !is_ground_net(&net.name) {
                        signal_pins_in_use += 1;
                    }
                }
            }
            let library_has_model = inputs.kicad.library_parts.iter().any(|part| {
                part.has_sim_fields && component.lib_id.as_deref() == Some(part.lib_id.as_str())
            });
            let suggested_fix = if library_has_model && !component.has_sim_fields {
                "copy_library_sim_fields"
            } else if signal_pins_in_use >= 2 {
                "replace_series_part"
            } else {
                "exclude"
            };
            Some(UnmodelledSymbol {
                reference: element.reference.clone(),
                value: component.value.clone(),
                lib_id: component.lib_id.clone(),
                pins,
                suggested_fix,
            })
        })
        .collect()
}

/// A number's value and whatever follows it.
fn split_number(word: &str) -> Option<(f64, &str)> {
    let unsigned = word.trim_start_matches(['+', '-']);
    let whole = unsigned
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(unsigned.len());
    let fraction = unsigned[whole..].strip_prefix('.').map_or(0, |after| {
        after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len())
            + 1
    });
    let mantissa = &unsigned[..whole + fraction];
    if !mantissa.contains(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let rest = strip_exponent(&unsigned[mantissa.len()..]);
    let number = unsigned[..unsigned.len() - rest.len()].replace(['d', 'D'], "e");
    Some((number.parse().ok()?, rest))
}

/// `E12` or Fortran's `D12` after a number. Letters that are not one stay.
fn strip_exponent(rest: &str) -> &str {
    let Some(after) = rest.strip_prefix(['e', 'E', 'd', 'D']) else {
        return rest;
    };
    let digits_start = after.strip_prefix(['+', '-']).unwrap_or(after);
    let end = digits_start
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits_start.len());
    if end == 0 {
        rest
    } else {
        &digits_start[end..]
    }
}

/// Which part a unit word belongs to, or `*` for one that fits any field.
fn unit_family(word: &str) -> Option<char> {
    if word.starts_with("ohm") || word == "r" {
        Some('r')
    } else if word == "f" || word.starts_with("farad") {
        Some('c')
    } else if word == "h" || word.starts_with("henr") {
        Some('l')
    } else if word.starts_with('v') {
        Some('v')
    } else if word.starts_with('a') {
        Some('i')
    } else if matches!(word, "hz" | "s" | "sec" | "deg") {
        Some('*')
    } else {
        None
    }
}

/// Read as ngspice reads a value: a number, then a scale from the first letters,
/// and it stops at anything else. Three shapes mean what they do not say. `-`,
/// `+`, `/` and `(` end the token, so `10k-1%` is -1. Letters then a digit stop early,
/// so `3V3` is 3 and, outside LTspice mode, `4k7` is 4k. `_` straight after the
/// digits hides the scale, so `100_nF` is 100. A unit from another part, such as
/// `16V` on a capacitor, is a rating in the wrong field.
fn suffix_reads_true(rest: &str, kind: char) -> bool {
    let hides_scale = rest
        .strip_prefix('_')
        .is_some_and(|tag| tag.starts_with(|c: char| c.is_ascii_alphabetic()));
    let thousands = rest
        .strip_prefix('_')
        .is_some_and(|tag| tag.len() == 3 && tag.chars().all(|c| c.is_ascii_digit()));
    if hides_scale || thousands || rest.starts_with(['.', '%']) || rest.contains(TOKEN_ENDS) {
        return false;
    }
    let lower = rest.to_lowercase();
    let length = lower
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(lower.len());
    let (letters, after) = lower.split_at(length);
    let digit_follows = after.starts_with(|c: char| c.is_ascii_digit());
    if letters.is_empty() {
        return true;
    }
    // `5A` on anything but a current is a rating, and ngspice reads it as atto.
    if kind != 'i' && letters.starts_with('a') {
        return false;
    }
    let fits =
        |word: &str| matches!(unit_family(word), Some(family) if family == kind || family == '*');
    if fits(letters) {
        return !digit_follows;
    }
    SCALES.iter().any(|(scale, _)| {
        letters.strip_prefix(scale).is_some_and(|tail| {
            if tail.is_empty() {
                !digit_follows
            } else {
                (fits(tail) && !digit_follows) || unit_family(tail).is_none()
            }
        })
    })
}

/// What ngspice would read a value word as, for a sanity check on its size.
fn magnitude(word: &str) -> Option<f64> {
    let (number, rest) = split_number(word)?;
    let lower = rest.to_lowercase();
    let scale = SCALES
        .iter()
        .find(|(scale, _)| lower.starts_with(scale))
        .map_or(1.0, |(_, factor)| *factor);
    Some(number * scale)
}

/// After `4k7`, a tag must be set apart. `1u50V` reads as 1.5u, so the digits of
/// a rating glued on become the fraction.
fn shorthand_tag_is_clean(tag: &str, kind: char) -> bool {
    let unit = tag.split(['_', '@']).next().unwrap_or("");
    !tag.contains(TOKEN_ENDS)
        && (unit.is_empty() || matches!(unit_family(unit), Some(family) if family == kind))
}

/// `4k7`, `4R7`, `R47`, `4Meg7`, `4L7` or `470L`, with a clean tag after it.
/// ngspice reads these only in LTspice mode.
fn is_value_shorthand(word: &str, kind: char) -> bool {
    let lower = word.to_lowercase();
    let letters = lower
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(lower.len());
    let (whole, rest) = lower.split_at(letters);
    if !whole.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    ["meg", "r", "k", "m", "g", "t", "u", "n", "p", "f", "l"]
        .iter()
        .filter(|scale| kind == 'r' || !matches!(**scale, "r" | "l"))
        .filter(|scale| !whole.is_empty() || **scale == "r")
        .any(|scale| {
            rest.strip_prefix(scale).is_some_and(|tail| {
                let digits = tail
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(tail.len());
                let enough_digits = digits > 0 || (*scale == "l" && !whole.is_empty());
                enough_digits && shorthand_tag_is_clean(&tail[digits..], kind)
            })
        })
}

/// What a value word can be, on what ngspice would actually read from it.
struct ValueReader<'a> {
    circuit: &'a Circuit,
    reads_shorthand: bool,
}

impl ValueReader<'_> {
    fn is_number(&self, word: &str, kind: char) -> bool {
        let unsigned = word.trim_start_matches(['+', '-']);
        if self.reads_shorthand
            && matches!(kind, 'r' | 'l' | 'c')
            && is_value_shorthand(unsigned, kind)
        {
            return true;
        }
        split_number(word).is_some_and(|(_, rest)| suffix_reads_true(rest, kind))
    }

    fn is_value(&self, word: &str, kind: char) -> bool {
        let inner = word.trim_matches(['(', ')']);
        let name = inner.trim_matches(['{', '}']).trim().to_lowercase();
        if let Some(value) = self.circuit.parameters.get(&name) {
            let literal = value.starts_with(|c: char| c.is_ascii_digit() || c == '.');
            return !literal || self.is_number(value, kind);
        }
        inner.starts_with(['{', '\'']) || self.is_number(inner, kind)
    }

    /// After `R=`, anything with an operator or bracket is an expression.
    fn is_value_after_key(&self, value: &str, kind: char) -> bool {
        !value.contains('%')
            && (value.contains(['(', '*', '/', '+', '-']) || self.is_value(value, kind))
    }

    fn passive_is_valid(&self, element: &Element) -> bool {
        let Some(first) = element.args.first() else {
            return false;
        };
        if self.circuit.models.contains_key(&first.to_lowercase()) {
            return true;
        }
        let valued = if first.starts_with(['{', '\'']) {
            self.is_value(first, element.kind)
        } else if let Some((key, value)) = first.split_once('=') {
            VALUE_KEYS.contains(&key.to_lowercase().as_str())
                && self.is_value_after_key(value, element.kind)
        } else {
            self.is_value(first, element.kind)
        };
        valued && self.later_words_are_harmless(element)
    }

    /// ngspice takes the last number, so `100n 50V` is a 50 F capacitor. A named
    /// model after a resistance is legal, `tc=a,b` carries a second number, and
    /// LTspice mode reads `noiseless`.
    fn later_words_are_harmless(&self, element: &Element) -> bool {
        element.args.iter().enumerate().skip(1).all(|(at, word)| {
            let lower = word.to_lowercase();
            let resistor_word = element.kind == 'r'
                && (self.circuit.models.contains_key(&lower)
                    || (self.reads_shorthand && lower == "noiseless"));
            let after_tc = element.args[at - 1].to_lowercase().starts_with("tc=");
            word.contains('=') || resistor_word || after_tc
        })
    }

    /// The word after `DC` must be a value, a source must start with a value or
    /// a keyword, and every number in a waveform must read as written. A `DC` or
    /// `AC` at the end sets nothing: `DC 12V DC` is 0 V, and `DC 12V AC` is a DC
    /// source with an AC magnitude of 1.
    fn source_is_valid(&self, element: &Element) -> bool {
        let spaced = without_expressions(&element.args.join(" ")).replace(['(', ')'], " ");
        let words: Vec<&str> = spaced.split_whitespace().collect();
        let numbers_are_true = words.iter().all(|word| {
            let number = word.rsplit('=').next().unwrap_or(word);
            !number.starts_with(|c: char| c.is_ascii_digit() || ".+-".contains(c))
                || (self.is_number(number, element.kind) && !number.contains('%'))
        });
        let in_pulse = words
            .iter()
            .skip_while(|word| !word.eq_ignore_ascii_case("pulse"));
        let pulse_times_are_true = in_pulse
            .take_while(|word| !word.eq_ignore_ascii_case("ac"))
            .all(|word| !word.to_lowercase().ends_with("hz"));
        let Some(first) = words.first() else {
            return true;
        };
        let lower = first.to_lowercase();
        let first_is_valid = if lower == "dc" {
            words.get(1).is_none_or(|word| {
                let keyword = word.to_lowercase();
                let keyed = matches!(keyword.as_str(), "dc" | "ac")
                    && words
                        .get(2)
                        .is_some_and(|value| self.is_value(value, element.kind));
                self.is_value(word, element.kind)
                    || word.contains('=')
                    || keyed
                    || WAVEFORM_KEYWORDS.contains(&keyword.as_str())
            })
        } else {
            SOURCE_KEYWORDS.contains(&lower.as_str())
                || first.contains('=')
                || self.is_value(first, element.kind)
        };
        let rating_before = |at: usize| {
            words[at].contains(|c: char| c.is_ascii_alphabetic())
                && words[at].starts_with(|c: char| c.is_ascii_digit())
        };
        let ends_in_rating = words.len() >= 3
            && match words[words.len() - 1].to_lowercase().as_str() {
                "dc" => true,
                "ac" => rating_before(words.len() - 2),
                _ => false,
            };
        first_is_valid && numbers_are_true && pulse_times_are_true && !ends_in_rating
    }

    /// `E1 o 0 i 0 1,5` reads as a polynomial and `3V3` as 3. Only the plain form
    /// has one gain word to judge.
    fn gain_is_valid(&self, element: &Element) -> bool {
        element.nodes.len() < 4 || self.gain_words_are_true(&element.args)
    }

    fn gain_is_valid_after_source(&self, element: &Element) -> bool {
        let plain = element
            .args
            .first()
            .is_some_and(|word| !word.to_lowercase().starts_with("poly") && !word.contains('.'));
        !plain || self.gain_words_are_true(&element.args[1..])
    }

    fn gain_words_are_true(&self, words: &[String]) -> bool {
        let is_gain_key = |word: &&String| word.to_lowercase().starts_with("gain=");
        if let Some(keyed) = words.iter().find(is_gain_key) {
            return self.is_value(&keyed["gain=".len()..], '*');
        }
        let gains: Vec<&String> = words.iter().filter(|word| !word.contains('=')).collect();
        match gains.as_slice() {
            [gain] => self.is_value(gain, '*'),
            _ => false,
        }
    }

    /// `K1 L1 L2 90%` reads as a factor of 90 and solves without complaint.
    fn coupling_is_valid(&self, element: &Element) -> bool {
        element.args.last().is_some_and(|word| {
            let value = word
                .rsplit('=')
                .next()
                .unwrap_or(word)
                .trim_matches(['(', ')']);
            value.starts_with(['{', '\''])
                || self.circuit.parameters.contains_key(&value.to_lowercase())
                || (!value.contains('%')
                    && magnitude(value).is_some_and(|value| value.abs() <= 1.0))
        })
    }
}

/// A source with each `{...}` and `'...'` replaced by `{}`, which reads as a value
/// whatever is inside it.
fn without_expressions(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '\'' if depth == 0 => {
                if !quoted {
                    out.push_str(" {} ");
                }
                quoted = !quoted;
            }
            '{' if !quoted => {
                if depth == 0 {
                    out.push_str(" {} ");
                }
                depth += 1;
            }
            '}' if !quoted => depth = depth.saturating_sub(1),
            _ if depth == 0 && !quoted => out.push(c),
            _ => {}
        }
    }
    out
}

const BRANCHES_UNDECIDED: &str = "The deck has .if blocks, and which branch runs is decided later.";

fn component_values(circuit: &Circuit, reads_shorthand: bool) -> Check {
    if circuit.has_conditionals {
        return Check::not_run(
            "component_values",
            "values_not_checked",
            BRANCHES_UNDECIDED.to_string(),
        );
    }
    let reader = ValueReader {
        circuit,
        reads_shorthand,
    };
    let invalid: Vec<String> = circuit
        .elements
        .iter()
        .filter(|element| match element.kind {
            'r' | 'l' | 'c' => !reader.passive_is_valid(element),
            'v' | 'i' => !reader.source_is_valid(element),
            'k' => !reader.coupling_is_valid(element),
            'e' | 'g' => !reader.gain_is_valid(element),
            'f' | 'h' => !reader.gain_is_valid_after_source(element),
            _ => false,
        })
        .map(|element| element.name.clone())
        .collect();
    if invalid.is_empty() {
        Check::passed("component_values")
    } else {
        Check::failed("component_values", "invalid_component_value", invalid)
    }
}

fn ground(circuit: &Circuit, nets: &[NetEntry]) -> Check {
    if circuit.has_conditionals {
        return Check::not_run(
            "ground",
            "ground_not_checked",
            BRANCHES_UNDECIDED.to_string(),
        );
    }
    let grounded = circuit.shunted
        || circuit.elements.iter().any(|element| {
            element.nodes.iter().any(|node| element::is_ground(node))
                || circuit.touches_ground(element)
        });
    if grounded {
        return Check::passed("ground");
    }
    if !circuit.unparsed.is_empty() {
        return Check::not_run(
            "ground",
            "ground_not_checked",
            format!("Could not read {}.", named(&circuit.unparsed)),
        );
    }
    let candidates = nets
        .iter()
        .filter(|net| {
            let name = net.kicad_name.to_lowercase();
            GROUND_LIKE_WORDS.iter().any(|word| name.contains(word))
                || name == "0"
                || name.ends_with("/0")
        })
        .map(|net| net.kicad_name.clone())
        .collect();
    Check::failed("ground", "no_ground_reference", candidates)
}

/// The node ngspice will really use. It lowers case, reads `gnd` as `0`, and
/// replaces each byte it cannot print with `_`. The worker runs in the C locale,
/// where that is every non-ASCII byte, so `/\u{5e9}` and `/\u{5ea}` are one node.
pub(crate) fn engine_node(spice_name: &str) -> String {
    let node: String = spice_name
        .bytes()
        .map(|byte| {
            if byte.is_ascii() {
                char::from(byte.to_ascii_lowercase())
            } else {
                '_'
            }
        })
        .collect();
    if node == "gnd" {
        "0".to_string()
    } else {
        node
    }
}

/// Only nets the deck really uses can merge, so a net whose parts are all
/// excluded or unmodelled does not count. `0` and `GND` are one node in ngspice.
fn net_names(inputs: &Inputs<'_>) -> Check {
    let live: HashSet<&str> = inputs
        .elements
        .iter()
        .filter(|element| element.modelled && !element.excluded)
        .map(|element| element.reference.as_str())
        .collect();
    let live_net = |name: &str| {
        inputs.kicad.nets.iter().any(|net| {
            net.name == name
                && net
                    .pins
                    .iter()
                    .any(|pin| live.contains(pin.reference.as_str()))
        })
    };
    let mut by_spice_name: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for net in inputs.nets.iter().filter(|net| live_net(&net.kicad_name)) {
        let node = engine_node(&net.spice_name);
        by_spice_name.entry(node).or_default().push(&net.kicad_name);
    }
    let mut collisions = Vec::new();
    for (node, names) in &by_spice_name {
        if names.len() > 1 {
            collisions.push(format!("{} become {node}", names.join(" and ")));
        }
        if node.contains('=') {
            collisions.push(format!("{node} is cut at the = by ngspice"));
        }
    }
    if collisions.is_empty() {
        Check::passed("net_names")
    } else {
        Check::failed("net_names", "net_name_collision", collisions)
    }
}

/// Two elements with one name, or a coupling or current-controlled source whose
/// partner is gone, which excluding a part can cause.
fn element_names(circuit: &Circuit) -> Check {
    if circuit.has_conditionals {
        return Check::not_run(
            "element_names",
            "names_not_checked",
            BRANCHES_UNDECIDED.to_string(),
        );
    }
    let mut seen = HashSet::new();
    let names: HashSet<String> = circuit
        .elements
        .iter()
        .map(|element| element.name.to_lowercase())
        .collect();
    let mut duplicates: Vec<String> = circuit
        .elements
        .iter()
        .filter(|element| !seen.insert(element.name.to_lowercase()))
        .map(|element| element.name.clone())
        .collect();
    duplicates.dedup();
    if !duplicates.is_empty() {
        return Check::failed("element_names", "duplicate_element", duplicates);
    }
    let missing: Vec<String> = circuit
        .elements
        .iter()
        .flat_map(|element| {
            let partners = match element.kind {
                'k' => element.args.len().saturating_sub(1),
                'f' | 'h' => 1,
                _ => 0,
            };
            element
                .args
                .iter()
                .take(partners)
                .map(move |partner| (element, partner))
        })
        .filter(|(_, partner)| {
            let partner = partner.to_lowercase();
            !names.contains(&partner) && !partner.starts_with("poly") && !partner.contains('.')
        })
        .map(|(element, partner)| format!("{} needs {partner}", element.name))
        .collect();
    if missing.is_empty() {
        Check::passed("element_names")
    } else {
        Check::failed("element_names", "missing_referenced_element", missing)
    }
}

fn named(names: &[String]) -> String {
    let shown: Vec<&str> = names
        .iter()
        .take(MAX_NAMED_ELEMENTS)
        .map(String::as_str)
        .collect();
    let more = names.len() - shown.len();
    if more == 0 {
        shown.join(", ")
    } else {
        format!("{} and {more} more", shown.join(", "))
    }
}

/// Why connectivity cannot be judged, if anything stops it.
fn connectivity_blocker(circuit: &Circuit, unmodelled: usize) -> Option<String> {
    if unmodelled > 0 {
        Some("Unmodelled symbols have no nodes in the deck yet.".to_string())
    } else if !circuit.unparsed.is_empty() {
        Some(format!("Could not read {}.", named(&circuit.unparsed)))
    } else if circuit.has_conditionals {
        Some(BRANCHES_UNDECIDED.to_string())
    } else {
        None
    }
}

/// Connectivity only reports whether it could look. What it finds is a warning,
/// because a floating group is fine for AC and transient results and wrong only
/// for an operating point, and the analysis is not known here.
fn connectivity(circuit: &Circuit, unmodelled: usize) -> Check {
    match connectivity_blocker(circuit, unmodelled) {
        Some(reason) => Check::not_run("connectivity", "connectivity_not_checked", reason),
        None => Check::passed("connectivity"),
    }
}

fn floating_groups(circuit: &Circuit, unmodelled: usize) -> Vec<String> {
    if connectivity_blocker(circuit, unmodelled).is_some() || circuit.shunted {
        return Vec::new();
    }
    let mut groups = dc_groups(circuit);
    groups
        .all()
        .into_iter()
        .filter(|(_, group)| !group.iter().any(|node| element::is_ground(node)))
        .map(|(_, group)| named(&group))
        .collect()
}

fn dc_groups(circuit: &Circuit) -> Groups {
    let mut groups = Groups::new();
    groups.add("0");
    let mut paths = DcPaths::new(circuit);
    for element in &circuit.elements {
        paths.add(element, &mut groups);
    }
    groups
}

/// An open `.if` or `.subckt` swallows the analysis line Konnect adds after it,
/// and the engine then reports a failure that says nothing of the cause.
fn deck_structure(deck: &FlatDeck) -> Check {
    let mut open_blocks = Vec::new();
    let mut stray = Vec::new();
    // By prefix, as ngspice matches them: `.ENDSUBCKT` and `.ends,f` both close.
    for (text, _, _) in deck.lines() {
        let line = text.trim().to_lowercase();
        if line.starts_with(".endif") {
            match open_blocks.last() {
                Some(&".if") => drop(open_blocks.pop()),
                _ => stray.push(".endif".to_string()),
            }
        } else if line.starts_with(".ends") {
            match open_blocks.last() {
                Some(&".subckt") => drop(open_blocks.pop()),
                _ => stray.push(".ends".to_string()),
            }
        } else if line.starts_with(".if") {
            open_blocks.push(".if");
        } else if line.starts_with(".subckt") {
            open_blocks.push(".subckt");
        }
    }
    let unclosed = open_blocks.iter().map(|block| format!("{block} left open"));
    let items: Vec<String> = stray.into_iter().chain(unclosed).collect();
    if items.is_empty() {
        Check::passed("deck_structure")
    } else {
        Check::failed("deck_structure", "unbalanced_deck_block", items)
    }
}

/// Digital parts, and the event-driven parts in `xtraevt`. A kind this misses
/// fails loudly in the engine, never silently.
fn has_event_nodes(circuit: &Circuit) -> bool {
    circuit.code_models_used.iter().any(|name| {
        circuit.models.get(name).is_some_and(|kind| {
            DIGITAL_MODELS.contains(&kind.as_str()) || kind.starts_with("real_")
        })
    })
}

fn code_models(circuit: &Circuit, allow_digital_models: bool) -> Check {
    if allow_digital_models {
        return Check::passed("code_models");
    }
    let mut digital: Vec<String> = circuit
        .code_models_used
        .iter()
        .filter(|name| {
            circuit
                .models
                .get(*name)
                .is_some_and(|kind| DIGITAL_MODELS.contains(&kind.as_str()))
        })
        .cloned()
        .collect();
    digital.sort();
    if digital.is_empty() {
        Check::passed("code_models")
    } else {
        Check::failed("code_models", "digital_models_disabled", digital)
    }
}

/// A node from the testbench that nothing else names is a typo, or the circuit
/// is silently reading zero. A `x1.int` name reaches inside a subcircuit.
fn testbench_nodes(circuit: &Circuit, nets: &[NetEntry]) -> Check {
    if !circuit.unparsed.is_empty() {
        return Check::not_run(
            "testbench_nodes",
            "testbench_nodes_not_checked",
            format!("Could not read {}.", named(&circuit.unparsed)),
        );
    }
    let known: HashSet<String> = nets
        .iter()
        .map(|net| net.spice_name.to_lowercase())
        .collect();
    let is_known = |node: &&String| {
        element::is_ground(node)
            || known.contains(*node)
            || node
                .split_once('.')
                .is_some_and(|(call, _)| call.starts_with('x'))
    };
    let mut unknown: Vec<String> = circuit
        .elements
        .iter()
        .filter(|element| element.from_testbench)
        .flat_map(|element| &element.nodes)
        .filter(|node| !is_known(node))
        .filter(|node| circuit.cards_naming(node) < 2)
        .cloned()
        .collect();
    let element_nodes: HashSet<&String> = circuit
        .elements
        .iter()
        .flat_map(|element| &element.nodes)
        .collect();
    unknown.extend(
        circuit
            .testbench_initial_nodes
            .iter()
            .filter(|node| !is_known(node) && !element_nodes.contains(*node))
            .cloned(),
    );
    unknown.sort();
    unknown.dedup();
    if unknown.is_empty() {
        Check::passed("testbench_nodes")
    } else {
        Check::failed("testbench_nodes", "unknown_testbench_node", unknown)
    }
}

fn warnings(inputs: &Inputs<'_>, circuit: &Circuit, unmodelled: usize) -> Vec<Warning> {
    let mut warnings = Vec::new();
    let mut warn = |code, detail, items: Vec<String>| {
        if !items.is_empty() {
            warnings.push(Warning {
                code,
                detail,
                items,
            });
        }
    };
    if inputs.code_model_count == 0 {
        warn(
            "code_models_missing",
            "No code models were loaded, so decks that need them will fail to parse.",
            vec!["analog".to_string()],
        );
    }
    warn(
        "floating_subcircuit",
        "These nodes have no DC path to ground. Differences within a group and AC or \
         transient results hold, but operating-point voltages are arbitrary.",
        floating_groups(circuit, unmodelled),
    );
    warn(
        "unpowered_net",
        "These nets connect to no independent source, so they will read zero.",
        unpowered_nets(circuit),
    );
    warn(
        "ambiguous_scale_letter",
        "ngspice reads M as milli, F as femto and A as atto, whatever the case. Write \
         Meg, or a number without a letter, if that is not what you mean.",
        ambiguous_scales(circuit),
    );
    warn(
        "ground_only_inside_subcircuit",
        "Nothing at the top level touches node 0, so the only reference is inside these \
         subcircuits, and a ground net with another name floats.",
        grounds_only_inside(circuit),
    );
    warn(
        "decimal_comma",
        "ngspice splits a number at a comma, so 1,5V is two fields and every later waveform \
         value moves along by one.",
        decimal_commas(circuit),
    );
    warn(
        "duplicate_definition",
        "Two different definitions share these names, and ngspice uses the first for both.",
        circuit.conflicting_definitions.clone(),
    );
    warn(
        "implausible_value",
        "These values are far too large, so a scale letter is missing or was misread. A bare \
         0.1 on a capacitor is 0.1 F, and KiCad turns a capital P into peta.",
        implausible_values(circuit),
    );
    warn(
        "source_without_value",
        "These sources have no value, so they are 0 V or 0 A.",
        sources_without_value(circuit),
    );
    warn(
        "inferred_from_value",
        "KiCad modelled these parts from their value alone, so they are ideal parts.",
        inferred_parts(inputs),
    );
    warn(
        "generic_default_model",
        "These parts still use their library's default model, whatever their value says.",
        generic_parts(inputs)
            .into_iter()
            .chain(circuit.default_models.iter().cloned())
            .collect(),
    );
    warn(
        "unknown_initial_node",
        "These .ic or .nodeset nodes match no net, and ngspice drops them without saying so.",
        unknown_initial_nodes(circuit, inputs.nets),
    );
    warn(
        "ground_like_local_net",
        "KiCad 10.0.1 keeps these as ordinary nodes, while later 10.0 builds make them ground.",
        ground_like_local_nets(inputs.nets),
    );
    warn(
        "net_map_mismatch",
        "These deck nodes match no KiCad net.",
        unmapped_nodes(circuit, inputs.nets),
    );
    warn(
        "pulse_period_defaulted",
        "Give PULSE all seven values, because the default changed at ngspice 46.",
        short_pulses(circuit),
    );
    warn(
        "gnd_pin_in_subcircuit",
        "ngspice reads a pin named gnd or 0 as global ground, so the net wired to it is not \
         connected.",
        circuit.gnd_pin_subcircuits.clone(),
    );
    warn(
        "exclusion_matched_nothing",
        "These exclude_references entries match no symbol.",
        inputs.unmatched_exclusions.to_vec(),
    );
    warnings
}

/// Groups joined by any element at all, with ground not counted as a connection
/// because everything touches it. A group with no independent source reads zero.
fn unpowered_nets(circuit: &Circuit) -> Vec<String> {
    if !circuit.unparsed.is_empty() || circuit.has_conditionals {
        return Vec::new();
    }
    let mut groups = Groups::new();
    for element in &circuit.elements {
        let nodes: Vec<&String> = element
            .nodes
            .iter()
            .filter(|node| !element::is_ground(node))
            .collect();
        for node in &nodes {
            groups.add(node);
        }
        for pair in nodes.windows(2) {
            groups.join(pair[0], pair[1]);
        }
    }
    join_coupled_inductors(circuit, &mut groups);
    // A dependent source reads other nodes through its expression or a named
    // source, which these groups cannot see. Only the plain `E` and `G` form, whose
    // inputs are nodes, depends on what is in its own group.
    let is_source = |element: &&Element| match element.kind {
        'v' | 'i' | 'a' | 'n' | 'b' | 'f' | 'h' => true,
        'e' | 'g' => element.nodes.len() < 4,
        'x' => circuit.has_source(element),
        _ => false,
    };
    let sources: Vec<&Element> = circuit.elements.iter().filter(is_source).collect();
    let powered: HashSet<usize> = sources
        .iter()
        .flat_map(|source| &source.nodes)
        .filter(|node| !element::is_ground(node))
        .map(|node| groups.group_of(node))
        .collect();
    groups
        .all()
        .into_iter()
        .filter(|(group, _)| !powered.contains(group))
        .flat_map(|(_, nodes)| nodes)
        .collect()
}

/// A capital `M` that is not `Meg` or `mil`. KiCad rewrites `1M`, but `1MOhm` and
/// `1MHz` reach ngspice as they are, and ngspice reads them as milli.
fn reads_as_milli(word: &str) -> bool {
    split_number(word).is_some_and(|(_, rest)| {
        let lower = rest.to_lowercase();
        rest.starts_with('M') && !lower.starts_with("meg") && !lower.starts_with("mil")
    })
}

/// Scale letters that surprise, in what KiCad wrote: anywhere for `M`, and by
/// part for `F` and `A`. Library files are left alone, where `10M` means milli on
/// purpose.
fn ambiguous_scales(circuit: &Circuit) -> Vec<String> {
    let mut names: Vec<String> = circuit
        .netlist_cards
        .iter()
        .filter(|card| {
            let words = card.split(|c: char| c.is_whitespace() || "()=,{}'".contains(c));
            let mut words = words.skip(1);
            words.any(reads_as_milli)
        })
        .filter_map(|card| {
            card.split_whitespace()
                .nth(usize::from(card.starts_with('.')))
        })
        .map(|name| name.split('=').next().unwrap_or(name).to_string())
        .collect();
    for element in &circuit.elements {
        let text = element.args.join(" ");
        let mut words = text.split(|c: char| c.is_whitespace() || "()=,".contains(c));
        let misleads = words.any(|word| {
            let letters = split_number(word)
                .map_or("", |(_, rest)| rest)
                .to_lowercase();
            let main = letters.split(['_', '@']).next().unwrap_or("");
            let old_farads = main.ends_with("fd");
            match element.kind {
                'i' => main.starts_with('a'),
                'c' => main == "f" || main.starts_with("farad") || old_farads,
                'l' | 'r' => main == "a",
                _ => false,
            }
        });
        if misleads {
            names.push(element.name.clone());
        }
    }
    names.extend(
        circuit
            .parameter_values
            .iter()
            .filter(|(_, value)| {
                let letters = split_number(value).map_or("", |(_, rest)| rest);
                letters.eq_ignore_ascii_case("a") || letters.eq_ignore_ascii_case("f")
            })
            .map(|(name, _)| name.clone()),
    );
    let mut seen = HashSet::new();
    names.retain(|name| seen.insert(name.clone()));
    names
}

fn grounds_only_inside(circuit: &Circuit) -> Vec<String> {
    let top_level_ground = circuit
        .elements
        .iter()
        .any(|element| element.nodes.iter().any(|node| element::is_ground(node)));
    if top_level_ground || circuit.has_conditionals {
        return Vec::new();
    }
    circuit
        .elements
        .iter()
        .filter(|element| circuit.touches_ground(element))
        .map(|element| element.name.clone())
        .collect()
}

/// `1,5V` or `0,5us` in a source. KiCad normalises a comma before a bare number,
/// but not before a unit.
fn decimal_commas(circuit: &Circuit) -> Vec<String> {
    let is_decimal_comma =
        |pair: &[char]| pair[0].is_ascii_digit() && pair[1] == ',' && pair[2].is_ascii_digit();
    circuit
        .netlist_cards
        .iter()
        .filter(|card| card.starts_with(['V', 'v', 'I', 'i']))
        .filter(|card| {
            let lower = card.to_lowercase();
            let list_of_pairs = lower.contains("pwl");
            card.split_whitespace().skip(1).any(|word| {
                let chars: Vec<char> = word.chars().collect();
                let has_comma = chars.windows(3).any(is_decimal_comma);
                let has_unit = chars.last().is_some_and(char::is_ascii_alphabetic);
                has_comma && (has_unit || !list_of_pairs)
            })
        })
        .filter_map(|card| card.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

fn implausible_values(circuit: &Circuit) -> Vec<String> {
    circuit
        .elements
        .iter()
        .filter(|element| {
            let limit = match element.kind {
                'c' => MAX_CAPACITANCE,
                'l' => MAX_INDUCTANCE,
                'v' => MAX_SOURCE,
                _ => return false,
            };
            let value = element
                .args
                .iter()
                .find(|word| !word.eq_ignore_ascii_case("dc"));
            let plain_decimal = element.kind == 'c'
                && value.is_some_and(|word| {
                    split_number(word)
                        .is_some_and(|(number, rest)| rest.is_empty() && number >= 1e-3)
                });
            plain_decimal
                || value
                    .and_then(|word| magnitude(word))
                    .is_some_and(|value| value >= limit)
        })
        .map(|element| element.name.clone())
        .collect()
}

fn unknown_initial_nodes(circuit: &Circuit, nets: &[NetEntry]) -> Vec<String> {
    let known: HashSet<String> = nets
        .iter()
        .map(|net| net.spice_name.to_lowercase())
        .collect();
    let used: HashSet<&String> = circuit.elements.iter().flat_map(|e| &e.nodes).collect();
    circuit
        .netlist_initial_nodes
        .iter()
        .filter(|node| !element::is_ground(node) && !known.contains(*node) && !used.contains(*node))
        .cloned()
        .collect()
}

fn sources_without_value(circuit: &Circuit) -> Vec<String> {
    circuit
        .elements
        .iter()
        .filter(|element| matches!(element.kind, 'v' | 'i') && element.args.is_empty())
        .map(|element| element.name.clone())
        .collect()
}

/// A transformer has no wire between its windings, but `K1 L1 L2 0.9` powers one
/// from the other.
fn join_coupled_inductors(circuit: &Circuit, groups: &mut Groups) {
    let first_node = |name: &String| {
        circuit
            .elements
            .iter()
            .find(|element| element.kind == 'l' && element.name.eq_ignore_ascii_case(name))
            .and_then(|element| element.nodes.iter().find(|node| !element::is_ground(node)))
    };
    for coupling in circuit
        .elements
        .iter()
        .filter(|element| element.kind == 'k')
    {
        let nodes: Vec<&String> = coupling.args.iter().filter_map(first_node).collect();
        for pair in nodes.windows(2) {
            groups.join(pair[0], pair[1]);
        }
    }
}

/// `R`, `R_Small_US` or a project's `R-Device`, but not `R_Potentiometer`.
fn is_plain_part(part: &str) -> bool {
    let mut words = part.split(['_', '-']);
    words.next().is_some_and(|word| PLAIN_PARTS.contains(&word))
        && words.all(|word| PLAIN_PART_WORDS.contains(&word))
}

fn inferred_parts(inputs: &Inputs<'_>) -> Vec<String> {
    inputs
        .elements
        .iter()
        .filter(|element| element.modelled && !element.excluded)
        .filter(|element| {
            element.element_name.as_deref().is_some_and(|name| {
                name.starts_with(['R', 'L', 'C', 'V', 'I', 'D', 'r', 'l', 'c', 'v', 'i', 'd'])
            })
        })
        .filter(|element| {
            inputs
                .kicad
                .components
                .iter()
                .find(|component| component.reference == element.reference)
                .is_some_and(|component| {
                    let part = component
                        .lib_id
                        .as_deref()
                        .and_then(|id| id.rsplit(':').next())
                        .unwrap_or("");
                    !component.has_sim_fields && !is_plain_part(part)
                })
        })
        .map(|element| element.reference.clone())
        .collect()
}

fn generic_parts(inputs: &Inputs<'_>) -> Vec<String> {
    inputs
        .kicad
        .components
        .iter()
        .filter(|component| {
            let lib_id = component.lib_id.as_deref().unwrap_or("").to_lowercase();
            let library = inputs
                .kicad
                .library_parts
                .iter()
                .find(|part| component.lib_id.as_deref() == Some(part.lib_id.as_str()));
            GENERIC_PARTS.iter().any(|part| lib_id.contains(part))
                && library.is_some_and(|part| part.sim_params == component.sim_params)
        })
        .map(|component| component.reference.clone())
        .collect()
}

fn ground_like_local_nets(nets: &[NetEntry]) -> Vec<String> {
    nets.iter()
        .filter(|net| {
            let name = net.kicad_name.to_lowercase();
            name.rsplit_once('/')
                .is_some_and(|(_, leaf)| matches!(leaf, "gnd" | "0"))
        })
        .map(|net| net.kicad_name.clone())
        .collect()
}

fn unmapped_nodes(circuit: &Circuit, nets: &[NetEntry]) -> Vec<String> {
    let known: HashSet<String> = nets
        .iter()
        .map(|net| net.spice_name.to_lowercase())
        .collect();
    let mut unmapped: Vec<String> = circuit
        .elements
        .iter()
        .filter(|element| !element.from_testbench)
        .flat_map(|element| &element.nodes)
        .filter(|node| !element::is_ground(node) && !known.contains(*node))
        .cloned()
        .collect();
    unmapped.sort();
    unmapped.dedup();
    unmapped
}

fn short_pulses(circuit: &Circuit) -> Vec<String> {
    circuit
        .elements
        .iter()
        .filter(|element| matches!(element.kind, 'v' | 'i'))
        .filter(|element| {
            let text = element.args.join(" ").to_lowercase();
            text.find("pulse").is_some_and(|at| {
                let values = text[at + "pulse".len()..]
                    .split(|c: char| c.is_whitespace() || "()".contains(c))
                    .filter(|word| !word.is_empty())
                    .take_while(|word| {
                        word.starts_with(|c: char| c.is_ascii_digit() || "{'.+-".contains(c))
                    })
                    .count();
                values < PULSE_VALUES
            })
        })
        .map(|element| element.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::flatten::{flatten, DeckInputs};
    use crate::simulation::kicad_netlist::{self, Component, LibraryPart, Net, SpiceNetlist};
    use std::path::Path;

    const RECTIFIER_NET: &str =
        include_str!("../../tests/fixtures/simulation/rectifier_unmodelled.net");
    const RECTIFIER_CIR: &str =
        include_str!("../../tests/fixtures/simulation/rectifier_unmodelled.cir");
    const COLLISION_NET: &str = include_str!("../../tests/fixtures/simulation/net_collision.net");
    const COLLISION_CIR: &str = include_str!("../../tests/fixtures/simulation/net_collision.cir");
    const HIERARCHY_NET: &str = include_str!("../../tests/fixtures/simulation/subsheets.net");
    const HIERARCHY_CIR: &str = include_str!("../../tests/fixtures/simulation/subsheets.cir");

    struct Setup {
        testbench: Vec<String>,
        allow_digital_models: bool,
        code_model_count: usize,
        kicad: KiCadNetlist,
        unmatched_exclusions: Vec<String>,
    }

    impl Default for Setup {
        fn default() -> Self {
            Self {
                testbench: Vec::new(),
                allow_digital_models: true,
                code_model_count: 6,
                kicad: KiCadNetlist {
                    components: Vec::new(),
                    nets: Vec::new(),
                    library_parts: Vec::new(),
                },
                unmatched_exclusions: Vec::new(),
            }
        }
    }

    /// Runs a netlist through vetting first, as the tool does.
    fn check(netlist: &str, setup: Setup) -> Preflight {
        check_in(netlist, setup, Path::new("/"), &[])
    }

    fn check_in(
        netlist: &str,
        setup: Setup,
        project_dir: &Path,
        allowed_roots: &[std::path::PathBuf],
    ) -> Preflight {
        let deck = flatten(&DeckInputs {
            netlist,
            testbench: &setup.testbench,
            project_dir,
            allowed_roots,
            compatibility: Compatibility::DEFAULT,
            excluded_elements: &[],
        })
        .unwrap();
        let spice = SpiceNetlist::parse(netlist);
        let nets = kicad_netlist::net_entries(&setup.kicad, &spice);
        let elements = kicad_netlist::element_entries(&setup.kicad, &spice, &[]);
        run(&Inputs {
            deck: &deck,
            compatibility: Compatibility::DEFAULT,
            kicad: &setup.kicad,
            nets: &nets,
            elements: &elements,
            allow_digital_models: setup.allow_digital_models,
            code_model_count: setup.code_model_count,
            unmatched_exclusions: &setup.unmatched_exclusions,
            has_testbench_lines: !setup.testbench.is_empty(),
        })
    }

    fn deck(body: &str) -> Preflight {
        check(&format!(".title t\n{body}\n.end\n"), Setup::default())
    }

    fn named<'a>(preflight: &'a Preflight, name: &str) -> &'a Check {
        preflight
            .checks
            .iter()
            .find(|check| check.name == name)
            .unwrap_or_else(|| panic!("no {name} check"))
    }

    fn fails(preflight: &Preflight, name: &str, code: &str) -> Vec<String> {
        let check = named(preflight, name);
        assert_eq!(check.passed, Some(false), "{check:?}");
        assert_eq!(check.code, Some(code));
        check.items.clone()
    }

    fn passes(preflight: &Preflight, name: &str) {
        let check = named(preflight, name);
        assert_eq!(check.passed, Some(true), "{check:?}");
    }

    fn warned(preflight: &Preflight, code: &str) -> Vec<String> {
        preflight
            .warnings
            .iter()
            .find(|warning| warning.code == code)
            .map_or_else(Vec::new, |warning| warning.items.clone())
    }

    const GOOD: &str = "V1 in 0 DC 5\nR1 in out 1k\nC1 out 0 1n\nR2 out 0 1k";

    #[test]
    fn a_sound_circuit_passes_every_check_with_no_warnings() {
        let preflight = deck(GOOD);
        assert!(
            preflight.checks.iter().all(|c| c.passed == Some(true)),
            "{preflight:?}"
        );
    }

    #[test]
    fn a_real_export_with_unmodelled_parts_names_them_and_says_what_it_could_not_check() {
        let kicad = kicad_netlist::parse(RECTIFIER_NET).unwrap();
        let preflight = check(
            RECTIFIER_CIR,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        assert_eq!(
            fails(&preflight, "model_coverage", "unmodelled_symbols"),
            ["D1", "V1"]
        );
        let connectivity = named(&preflight, "connectivity");
        assert!(!connectivity.ran);
        assert_eq!(connectivity.code, Some("connectivity_not_checked"));
        assert_eq!(connectivity.passed, None);
        assert!(connectivity.reason.as_ref().unwrap().contains("Unmodelled"));
        let diode = &preflight.unmodelled_symbols[0];
        assert_eq!(diode.suggested_fix, "copy_library_sim_fields");
        assert_eq!(diode.pins.len(), 2);
        assert!(diode.pins.iter().any(|pin| pin.spice_net.is_some()));
    }

    #[test]
    fn a_series_part_next_to_another_unmodelled_part_is_not_called_excludable() {
        let kicad = KiCadNetlist {
            components: vec![
                component("F1"),
                component("F2"),
                component("R1"),
                component("V1"),
            ],
            nets: vec![
                net("/a", &["V1", "F1"]),
                net("/m", &["F1", "F2"]),
                net("/b", &["F2", "R1"]),
                net("GND", &["V1", "R1"]),
            ],
            library_parts: Vec::new(),
        };
        let netlist = ".title t\nV1 /a 0 1\nF1 __F1\nF2 __F2\nR1 /b 0 1k\n";
        let preflight = check(
            netlist,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        for symbol in &preflight.unmodelled_symbols {
            assert_eq!(symbol.suggested_fix, "replace_series_part", "{symbol:?}");
        }
    }

    #[test]
    fn a_connector_across_a_rail_can_be_excluded() {
        let kicad = KiCadNetlist {
            components: vec![component("J1"), component("R1")],
            nets: vec![net("/vcc", &["J1", "R1"]), net("GND", &["J1", "R1"])],
            library_parts: Vec::new(),
        };
        let netlist = ".title t\nJ1 __J1\nR1 /vcc GND 1k\n";
        let preflight = check(
            netlist,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        assert_eq!(preflight.unmodelled_symbols[0].suggested_fix, "exclude");
    }

    #[test]
    fn a_pin_on_a_net_the_deck_does_not_name_has_no_spice_net() {
        let kicad = KiCadNetlist {
            components: vec![component("J1")],
            nets: vec![net("/only_j1", &["J1"])],
            library_parts: Vec::new(),
        };
        let preflight = check(
            ".title t\nJ1 __J1\nV1 a 0 1\nR1 a 0 1k\n",
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        assert_eq!(preflight.unmodelled_symbols[0].pins[0].spice_net, None);
    }

    #[test]
    fn an_excluded_symbol_is_not_unmodelled() {
        let kicad = KiCadNetlist {
            components: vec![component("J1")],
            nets: Vec::new(),
            library_parts: Vec::new(),
        };
        let netlist = ".title t\nJ1 __J1\nV1 a 0 1\nR1 a 0 1k\n";
        let spice = SpiceNetlist::parse(netlist);
        let nets = kicad_netlist::net_entries(&kicad, &spice);
        let elements = kicad_netlist::element_entries(&kicad, &spice, &["J1".to_string()]);
        let deck = flatten(&DeckInputs {
            netlist,
            testbench: &[],
            project_dir: Path::new("/"),
            allowed_roots: &[],
            compatibility: Compatibility::DEFAULT,
            excluded_elements: &["j1".to_string()],
        })
        .unwrap();
        let preflight = run(&Inputs {
            deck: &deck,
            compatibility: Compatibility::DEFAULT,
            kicad: &kicad,
            nets: &nets,
            elements: &elements,
            allow_digital_models: true,
            code_model_count: 6,
            unmatched_exclusions: &[],
            has_testbench_lines: false,
        });
        assert!(preflight.unmodelled_symbols.is_empty());
        passes(&preflight, "model_coverage");
        passes(&preflight, "connectivity");
    }

    #[test]
    fn a_part_with_no_series_role_can_be_excluded_and_one_in_series_cannot() {
        let pin = |reference: &str| kicad_netlist::Pin {
            reference: reference.to_string(),
            pin: "1".to_string(),
            pin_name: None,
            pin_type: None,
        };
        let component = |reference: &str| Component {
            reference: reference.to_string(),
            value: "x".to_string(),
            lib_id: None,
            has_sim_fields: false,
            sim_params: None,
        };
        let kicad = KiCadNetlist {
            components: vec![
                component("F1"),
                component("J1"),
                component("R1"),
                component("R2"),
            ],
            nets: vec![
                Net {
                    name: "/a".to_string(),
                    pins: vec![pin("F1"), pin("R1"), pin("J1")],
                },
                Net {
                    name: "/b".to_string(),
                    pins: vec![pin("F1"), pin("R2")],
                },
            ],
            library_parts: Vec::new(),
        };
        let netlist = ".title t\nF1 __F1\nJ1 __J1\nR1 /a 0 1k\nR2 /b 0 1k\nV1 /a 0 1\nV2 /b 0 1\n";
        let preflight = check(
            netlist,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        let fix = |reference: &str| {
            let symbols = &preflight.unmodelled_symbols;
            symbols
                .iter()
                .find(|s| s.reference == reference)
                .unwrap()
                .suggested_fix
        };
        assert_eq!(fix("F1"), "replace_series_part");
        assert_eq!(fix("J1"), "exclude");
    }

    #[test]
    fn a_circuit_with_no_ground_is_refused_and_ground_like_nets_are_offered() {
        let kicad = KiCadNetlist {
            components: Vec::new(),
            nets: ["PGND", "/AGND", "/vout"]
                .map(|name| Net {
                    name: name.to_string(),
                    pins: Vec::new(),
                })
                .to_vec(),
            library_parts: Vec::new(),
        };
        let netlist = ".title t\nV1 a PGND 12\nR1 a PGND 1k\n.end\n";
        let preflight = check(
            netlist,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        assert_eq!(
            fails(&preflight, "ground", "no_ground_reference"),
            ["PGND", "/AGND"]
        );
    }

    #[test]
    fn gnd_in_any_case_is_ground() {
        passes(&deck("V1 a GND 1\nR1 a gnd 1k"), "ground");
    }

    #[test]
    fn ground_is_not_called_missing_when_part_of_the_circuit_was_unreadable() {
        let preflight = deck("Y1 a b c\nR1 a b 1k");
        let ground = named(&preflight, "ground");
        assert!(!ground.ran);
        assert_eq!(ground.code, Some("ground_not_checked"));
    }

    #[test]
    fn real_nets_that_merge_in_spice_are_refused() {
        let kicad = kicad_netlist::parse(COLLISION_NET).unwrap();
        // The fixture's include points at the folder it was exported from.
        let lines: Vec<&str> = COLLISION_CIR
            .lines()
            .filter(|line| !line.starts_with(".include"))
            .collect();
        let netlist = lines.join("\n");
        let preflight = check(
            &netlist,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        let items = fails(&preflight, "net_names", "net_name_collision");
        assert_eq!(items.len(), 1, "{items:?}");
        assert!(items[0].contains("/a_1_"), "{items:?}");
    }

    fn net(name: &str, pins: &[&str]) -> Net {
        let pins = pins.iter().map(|reference| kicad_netlist::Pin {
            reference: reference.to_string(),
            pin: "1".to_string(),
            pin_name: None,
            pin_type: None,
        });
        Net {
            name: name.to_string(),
            pins: pins.collect(),
        }
    }

    fn component(reference: &str) -> Component {
        Component {
            reference: reference.to_string(),
            value: "x".to_string(),
            lib_id: None,
            has_sim_fields: false,
            sim_params: None,
        }
    }

    #[test]
    fn nets_that_differ_only_by_case_collide() {
        let kicad = KiCadNetlist {
            components: vec![component("R1"), component("R2")],
            nets: vec![net("/VIN", &["R1"]), net("/vin", &["R2"])],
            library_parts: Vec::new(),
        };
        let netlist = ".title t\nV1 /VIN 0 1\nR1 /VIN 0 1k\nR2 /vin 0 1k\n";
        let preflight = check(
            netlist,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        fails(&preflight, "net_names", "net_name_collision");
    }

    #[test]
    fn a_net_with_no_live_part_cannot_collide() {
        let kicad = KiCadNetlist {
            components: vec![component("R1"), component("J1")],
            nets: vec![net("/A(1)", &["R1"]), net("/A[1]", &["J1"])],
            library_parts: Vec::new(),
        };
        let netlist = ".title t\nV1 /A_1_ 0 1\nR1 /A_1_ 0 1k\nJ1 __J1\n";
        let preflight = check(
            netlist,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        passes(&preflight, "net_names");
    }

    #[test]
    fn an_open_block_would_swallow_the_analysis() {
        let circuit = "V1 a 0 1\nR1 a 0 1k";
        passes(&deck(circuit), "deck_structure");
        let balanced = format!("{circuit}\n.subckt x a b\nR2 a b 1k\n.if (1)\n.endif\n.ends x");
        passes(&deck(&balanced), "deck_structure");
        for closed in [
            ".SUBCKT tt a b\nR2 a b 1k\n.ENDSUBCKT",
            ".subckt tt a b\nR2 a b 1k\n.ends,tt",
            ".subckt tt a b\nR2 a b 1k\n.ends;tt",
            ".if (1)\nR2 a 0 1k\n.endif;x",
            ".if (1)\nR2 a 0 1k\n.else\nR3 a 0 1k\n.endif",
            ".if(1)\n.elseif(0)\n.endif",
        ] {
            passes(&deck(&format!("{circuit}\n{closed}")), "deck_structure");
        }
        for (extra, item) in [
            (".if (0)", ".if left open"),
            (".IF(1)", ".if left open"),
            (".subckt x a b", ".subckt left open"),
            (".endif", ".endif"),
            (".ends", ".ends"),
        ] {
            let items = fails(
                &deck(&format!("{circuit}\n{extra}")),
                "deck_structure",
                "unbalanced_deck_block",
            );
            assert_eq!(items, [item]);
        }
    }

    #[test]
    fn event_nodes_come_from_digital_and_real_valued_parts() {
        let analog = "V1 a 0 1\nR1 a 0 1k\nA1 a b amp\n.model amp gain(gain=2)\nR2 b 0 1k";
        assert!(!deck(analog).event_nodes);
        let digital = "V1 a 0 1\nR1 a 0 1k\nA1 [a] [b] inv\n.model inv d_inverter\nR2 b 0 1k";
        assert!(deck(digital).event_nodes);
        let real = "V1 a 0 1\nR1 a 0 1k\nA1 a b dly\n.model dly real_delay\nR2 b 0 1k";
        assert!(deck(real).event_nodes);
    }

    #[test]
    fn hebrew_nets_of_one_length_merge_in_the_engine() {
        let kicad = KiCadNetlist {
            components: vec![component("R1"), component("R2")],
            nets: vec![
                net("/\u{5e9}", &["R1"]),
                net("/\u{5ea}", &["R2"]),
                net("/a", &[]),
            ],
            library_parts: Vec::new(),
        };
        let netlist = ".title t\nV1 /a 0 1\nR1 /a /\u{5e9} 1k\nR2 /a /\u{5ea} 1k\n";
        let preflight = check(
            netlist,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        fails(&preflight, "net_names", "net_name_collision");
    }

    #[test]
    fn hebrew_nets_of_different_lengths_stay_apart() {
        assert_eq!(engine_node("/\u{5e9}"), "/__");
        assert_eq!(engine_node("/\u{5e9}\u{5e8}"), "/____");
        assert_eq!(engine_node("/Out"), "/out");
    }

    #[test]
    fn a_net_named_0_merges_with_gnd() {
        let kicad = KiCadNetlist {
            components: vec![component("R1"), component("R2")],
            nets: vec![net("0", &["R1"]), net("GND", &["R2"])],
            library_parts: Vec::new(),
        };
        let netlist = ".title t\nV1 a 0 1\nR1 a 0 1k\nR2 a GND 1k\n";
        let preflight = check(
            netlist,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        fails(&preflight, "net_names", "net_name_collision");
    }

    #[test]
    fn a_real_hierarchy_passes_the_net_check() {
        let kicad = kicad_netlist::parse(HIERARCHY_NET).unwrap();
        let preflight = check(
            HIERARCHY_CIR,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        passes(&preflight, "net_names");
        passes(&preflight, "ground");
    }

    fn floats(body: &str) -> Vec<String> {
        warned(&deck(body), "floating_subcircuit")
    }

    #[test]
    fn a_group_with_no_dc_path_to_ground_is_a_warning_not_a_refusal() {
        for (body, group) in [
            ("V1 a 0 1\nR1 a 0 1k\nR2 x y 1k\nC1 y a 1n", "x, y"),
            ("V1 a 0 AC 1\nR1 a 0 1k\nC1 a x 1n\nC2 x 0 1n", "x"),
            ("V1 a 0 1\nR1 a 0 1k\nI1 x 0 1m", "x"),
            ("V1 a 0 1\nR1 a 0 1k\nG1 x 0 a 0 1m\nR9 x y 1", "x, y"),
            ("V1 a 0 1\nR1 a 0 1k\nB1 x 0 I  =  1m", "x"),
            ("V1 a 0 1\nR1 a 0 1k\nB1 x 0 I\t=\t1m", "x"),
        ] {
            let preflight = deck(body);
            assert_eq!(warned(&preflight, "floating_subcircuit"), [group], "{body}");
            passes(&preflight, "connectivity");
        }
    }

    #[test]
    fn an_input_terminal_is_not_a_dc_path() {
        for (body, group) in [
            ("V1 a 0 1\nC1 a g 1n\nM1 a g 0 0 mm\n.model mm NMOS", "g"),
            ("V1 a 0 1\nC1 a g 1n\nE1 out 0 g 0 2\nR1 out 0 1k", "g"),
            ("V1 a 0 1\nC1 a g 1n\nS1 a 0 g 0 ss\n.model ss SW", "g"),
            ("V1 a 0 1\nR3 a 0 1k\nE1 x y a 0 2\nR9 x y 1k", "x, y"),
        ] {
            assert!(
                floats(body).contains(&group.to_string()),
                "{body}: {:?}",
                floats(body)
            );
        }
    }

    #[test]
    fn every_conducting_device_gives_a_dc_path() {
        for body in [
            "V1 a 0 1\nL1 a b 1u\nR1 b 0 1k",
            "V1 a 0 1\nD1 a b dd\nR1 b 0 1k\n.model dd D",
            "V1 a 0 1\nQ1 a a b qq\nR1 b 0 1k\n.model qq NPN",
            "V1 a 0 1\nQ1 a a b s qq\nR1 b 0 1k\nR2 s 0 1k\n.model qq NPN",
            "V1 a 0 1\nM1 a a b b mm\nR1 b 0 1k\n.model mm NMOS",
            "V1 a 0 1\nM1 a a b mm\nR1 b 0 1k\n.model mm VDMOS",
            "V1 a 0 1\nJ1 a a b jj\nR1 b 0 1k\n.model jj NJF",
            "V1 a 0 1\nS1 a b a 0 ss\nR1 b 0 1k\n.model ss SW",
            "V1 a 0 1\nE1 b 0 a 0 2\nR1 b 0 1k",
            "V1 a 0 1\nE1 b 0 VALUE={v(a)*2}\nR1 b 0 1k",
            "V1 a 0 1\nE1 b 0 POLY(1) a 0 0 2\nR1 b 0 1k",
            "V1 a 0 1\nH1 b 0 V1 2\nR1 b 0 1k",
            "V1 a 0 1\nB1 b 0 V=v(a)\nR1 b 0 1k",
            "V1 a 0 1\nT1 a 0 b 0 z0=50 td=1n\nR1 b 0 1k",
            "V1 a 0 1\nK1 L1 L2 0.9\nL1 a 0 1u\nL2 b 0 1u\nR1 b 0 1k",
            "V1 a 0 1\nW1 a b V1 ww\nR1 b 0 1k\n.model ww CSW",
        ] {
            assert!(floats(body).is_empty(), "{body}: {:?}", floats(body));
            passes(&deck(body), "connectivity");
        }
    }

    #[test]
    fn a_subcircuit_joins_its_pins_and_reaches_ground_only_through_a_real_path() {
        let call = "V1 a 0 1\nR1 a 0 1k\nC1 a x 1n\nX9 x y S\n";
        for body in [
            ".subckt S p q\nR1 p q 1k\n.ends",
            ".subckt S p q\nX1 p q inner\n.ends\n.subckt inner p q\nR1 p q 1k\n.ends",
            ".subckt S p q\nR1 p q 1k\nC1 q 0 1p\n.ends",
            ".subckt S p q\nV1 p n 0\nR1 n q 1k\n.ends",
            ".subckt S p gnd\nR1 p gnd 1k\n.ends",
            ".subckt S p 0\nR1 p 0 1k\n.ends",
        ] {
            assert_eq!(floats(&format!("{call}{body}")), ["x, y"], "{body}");
        }
        for body in [
            ".subckt S p q\nR1 p q 1k\nR2 q 0 1k\n.ends",
            ".subckt S p q\nA1 p q g\n.model g gain(gain=2)\n.ends",
        ] {
            assert!(floats(&format!("{call}{body}")).is_empty(), "{body}");
        }
        for body in [
            ".subckt S p q\nE1 q 0 p 0 2\n.ends",
            ".subckt S p q\nX1 p q inner\n.ends\n.subckt inner p q\nR1 q 0 1k\n.ends",
        ] {
            assert_eq!(floats(&format!("{call}{body}")), ["x"], "{body}");
        }
    }

    #[test]
    fn subcircuit_calls_read_their_nodes_past_parameters() {
        let body = ".subckt r p q params: v=1\nR1 p q {v}\n.ends\nV1 a 0 1\nX1 a 0 r params: v=2\nX2 a 0 r v=3";
        passes(&deck(body), "connectivity");
        assert!(floats(body).is_empty());
    }

    #[test]
    fn code_model_ports_are_read_in_every_spelling() {
        for ports in [
            "%v(in) %v(out)",
            "(in) (out)",
            "%v (in) %v (out)",
            "%vd(in 0) %vd(out 0)",
            "in out",
            "[in] [out]",
        ] {
            let body = format!("V1 in 0 1\nA1 {ports} g\nR1 out x 1k\n.model g gain(gain=2)");
            assert!(floats(&body).is_empty(), "{ports}: {:?}", floats(&body));
            assert!(warned(&deck(&body), "unpowered_net").is_empty(), "{ports}");
        }
    }

    #[test]
    fn a_nonzero_shunt_means_nothing_floats_and_a_zero_one_means_nothing() {
        let body = "V1 a 0 1\nR1 a 0 1k\nI1 x 0 1m";
        assert!(floats(&format!(".options rshunt=1e12\n{body}")).is_empty());
        assert_eq!(floats(&format!(".options gshunt=1p\n{body}")), ["x"]);
        assert_eq!(floats(&format!(".options rshunt=0\n{body}")), ["x"]);
        assert_eq!(floats(&format!(".options gshunt=0\n{body}")), ["x"]);
    }

    #[test]
    fn a_shunt_alone_is_a_reference_and_ground_says_so() {
        passes(&deck(".options rshunt=1e9\nV1 a b 5\nR1 a b 1k"), "ground");
    }

    #[test]
    fn a_reference_inside_a_subcircuit_or_code_model_counts_as_ground() {
        passes(
            &deck(".subckt tie p m\nR1 p 0 1k\nR2 m 0 1k\n.ends\nV1 a b 5\nX1 a b tie"),
            "ground",
        );
        passes(&deck("V1 a b 5\nA1 a b g\n.model g gain(gain=2)"), "ground");
        let unreferenced = deck(".subckt iso p m\nR1 p m 1k\n.ends\nV1 a b 5\nX1 a b iso");
        fails(&unreferenced, "ground", "no_ground_reference");
    }

    #[test]
    fn ground_is_not_judged_through_a_conditional() {
        for directive in [".if (0)", ".if(0)", ".IF(p==1)"] {
            let preflight = deck(&format!(
                "V1 a b 1\nR1 a b 1k\n{directive}\nR2 a 0 1k\n.endif"
            ));
            assert!(!named(&preflight, "ground").ran, "{directive}");
            assert!(!named(&preflight, "connectivity").ran, "{directive}");
        }
    }

    #[test]
    fn gnd_at_the_end_of_a_line_is_an_ordinary_node_as_in_ngspice() {
        fails(
            &deck("V1 a b 1\nR1 a b 1k\nV2 b gnd"),
            "ground",
            "no_ground_reference",
        );
    }

    #[test]
    fn a_subcircuit_with_gnd_as_its_last_pin_is_disclosed() {
        let body = "V1 a 0 5\nX1 a b s\nR1 b 0 1k\n.subckt s p gnd\nR2 p gnd 1k\n.ends";
        assert_eq!(warned(&deck(body), "gnd_pin_in_subcircuit"), ["s"]);
        let spare = "V1 a 0 5\nX1 a b s\nR1 b 0 1k\n.subckt s p vss\nR2 p vss 1k\n.ends";
        assert!(warned(&deck(spare), "gnd_pin_in_subcircuit").is_empty());
    }

    #[test]
    fn subcircuit_bodies_are_not_top_level_elements() {
        for head in [".subckt f p q", ".subcktx f p q", ".macro f p q"] {
            let tail = if head.starts_with(".macro") {
                ".eom"
            } else {
                ".ends"
            };
            let body = format!("V1 a b 1\nR1 a b 1k\n{head}\nR1 p 0 1k\n{tail}");
            fails(&deck(&body), "ground", "no_ground_reference");
        }
    }

    #[test]
    fn a_word_or_a_wrong_number_where_a_value_belongs_is_refused() {
        let items = |body: &str| fails(&deck(body), "component_values", "invalid_component_value");
        for (body, name) in [
            ("VBT1 a 0 DC Battery\nR1 a 0 1k", "VBT1"),
            ("VBT1 a 0 DC 3V3\nR1 a 0 1k", "VBT1"),
            ("VBT1 a 0 DC 2xAA\nR1 a 0 1k", "VBT1"),
            ("V1 a 0 Battery\nR1 a 0 1k", "V1"),
            ("V1 a 0 .Battery\nR1 a 0 1k", "V1"),
            ("V1 a 0 1\nR1 a 0 Battery", "R1"),
            ("V1 a 0 1\nR1 a 0", "R1"),
            ("V1 a 0 1\nR1 a 0 .", "R1"),
            ("V1 a 0 1\nR1 a 0 .Battery", "R1"),
            ("V1 a 0 1\nR1 a 0 tc=0.1", "R1"),
            ("V1 a 0 1\nR1 a 0 w=1u", "R1"),
            ("V1 a 0 1\nR1 a 0 r=Battery", "R1"),
            ("V1 a 0 1\nR1 a 0 10k 5%", "R1"),
            ("V1 a 0 1\nR1 a 0 10k 0.25W", "R1"),
            ("V1 a 0 1\nR1 a 0 1k\nC1 a 0 big", "C1"),
            ("V1 a 0 1\nR1 a 0 1k\nC1 a 0 100n 50V", "C1"),
            ("V1 a 0 1\nR1 a 0 1k\nC1 a 0 ic=1", "C1"),
            ("V1 a 0 1\nR1 a 0 1k\nL1 a 0 huge", "L1"),
            ("V1 a 0 1\nR1 a 0 1k\nL1 a 0 10u 2A", "L1"),
            ("V1 a 0 1\nR1 a 0 1k\nC1 a 0 16V", "C1"),
            ("V1 a 0 1\nR1 a 0 1k\nC1 a 0 1kOhm", "C1"),
            ("V1 a 0 1\nR1 a 0 5V", "R1"),
            ("V1 a 0 1\nR1 a 0 1.5.2k", "R1"),
            ("V1 a 0 DC 1.2.3\nR1 a 0 1k", "V1"),
            ("V1 a 0 1\nC1 a 0 2n cm\nR1 a 0 1k\n.model cm C", "C1"),
            ("V1 a 0 1\nR1 a 0 10k-1%", "R1"),
            ("V1 a 0 1\nR1 a 0 10k-0603", "R1"),
            ("V1 a 0 1\nR1 a 0 1-10k", "R1"),
            ("V1 a 0 1\nR1 a 0 1k\nC1 a 0 100n-50V", "C1"),
            ("V1 a 0 1\nR1 a 0 10_k", "R1"),
            ("V1 a 0 1\nR1 a 0 1k\nC1 a 0 100_nF", "C1"),
            ("V1 a 0 SIN(0 3V3 1k)\nR1 a 0 1k", "V1"),
            ("V1 a 0 DC 0 SIN( 0 1 4k7 0 0 0 ) AC 1\nR1 a 0 1k", "V1"),
            ("V1 a 0 PULSE(0 1V8 0 1n 1n 1u 2u)\nR1 a 0 1k", "V1"),
            ("V1 a 0 DC DC\nR1 a 0 1k", "V1"),
            ("V1 a 0 DC external\nR1 a 0 1k", "V1"),
            ("V1 a 0 DC 12V DC\nR1 a 0 1k", "V1"),
            ("V1 a 0 DC 12V AC\nR1 a 0 1k", "V1"),
            ("V1 a 0 PULSE(0 5 0 1n 1n 50% 10u)\nR1 a 0 1k", "V1"),
            ("V1 a 0 PULSE(0 5 0 1n 1n 5u 100kHz)\nR1 a 0 1k", "V1"),
            ("V1 a 0 1\nE1 b 0 a 0 1,5\nR1 b 0 1k", "E1"),
            ("V1 a 0 1\nE1 b 0 a 0 3V3\nR1 b 0 1k", "E1"),
            ("V1 a 0 1\nG1 b 0 a 0 1mS1\nR1 b 0 1k", "G1"),
            ("V1 a 0 1\nR1 a 0 R=10k-1%", "R1"),
            ("V1 a 0 1\nR1 a 0 1k\nC1 a 0 C=10u-20%", "C1"),
            ("V1 a 0 DC 5A\nR1 a 0 1k", "V1"),
            ("V1 a 0 DC 230AC\nR1 a 0 1k", "V1"),
            ("V1 a 0 1\nR1 a 0 1k\nC1 a 0 16A", "C1"),
            ("V1 a 0 1\nR1 a 0 1k\nL1 a 0 2A", "L1"),
            ("V1 a 0 1\nE1 b 0 a 0 10A\nR1 b 0 1k", "E1"),
            ("V1 a 0 1\nR1 a 0 5%", "R1"),
            ("V1 a 0 1\nR1 a 0 1_000", "R1"),
            ("V1 a 0 1\nR1 a 0 1k\nC1 a 0 R47", "C1"),
            ("V1 a 0 1\nR1 a 0 1k\nL1 a 0 4R7", "L1"),
            ("V1 a 0 1\nR1 a 0 u47", "R1"),
            (
                "V1 a 0 1\nR1 a 0 1k\nL1 a 0 1u\nL2 a 0 1u\nK1 L1 L2 0.9%",
                "K1",
            ),
            (".param cv=16V\nV1 a 0 1\nR1 a 0 1k\nC1 a 0 {cv}", "C1"),
            ("V1 a 0 1\nH1 b 0 V1 3V3\nR1 b 0 1k", "H1"),
            ("V1 a 0 1\nR1 a 0 10k(1%)", "R1"),
            ("V1 a 0 1\nR1 a 0 1k\nC1 a 0 100n(50V)", "C1"),
            ("V1 a 0 1\nR1 a 0 1k5%", "R1"),
            ("V1 a 0 1\nR1 a 0 1k\nC1 a 0 1u50V", "C1"),
            (
                "V1 a 0 1\nR1 a 0 1k\nL1 a 0 1u\nL2 a 0 1u\nK1 L1 L2 90%",
                "K1",
            ),
            (
                "V1 a 0 1\nR1 a 0 1k\nL1 a 0 1u\nL2 a 0 1u\nK1 L1 L2 2",
                "K1",
            ),
            (
                "V1 a 0 1\nR1 a 0 1k\nL1 a 0 1u\nL2 a 0 1u\nK1 L1 L2 k",
                "K1",
            ),
        ] {
            assert_eq!(items(body), [name], "{body}");
        }
    }

    #[test]
    fn every_legal_value_form_passes() {
        for body in [
            "V1 a 0 DC 5\nR1 a 0 {r}",
            "V1 a 0 SIN(0 1 1k)\nR1 a 0 '2*r'",
            "V1 a 0 PULSE(0 1 0 1n 1n 1u 2u)\nR1 a 0 r=1k",
            "V1 a 0 AC 1\nR1 a 0 1.5k tc=0.1",
            "V1 a 0\nR1 a 0 1k",
            "V1 a 0 DC\nR1 a 0 1k",
            "V1 a 0 portnum 1 z0 50\nR1 a 0 1k",
            "V1 a 0 PWL(0 0 1 1)\nR1 a 0 rmodel\n.model rmodel R(r=1k)",
            "V1 a 0 dc 0 ac 1\nR1 a 0 -1k\nC1 a 0 .1u ic=0",
            "I1 0 a pulse(0 1 0 1n 1n 1u 2u)\nR1 a 0 1k",
            "V1 a 0 DC {vin}\nR1 a 0 1k",
            "V1 a 0 DC ac=1 AC 1\nR1 a 0 1k",
            "V1 a 0 DC SINE(0 1.5 1k 0 0 0 0)\nR1 a 0 1k",
            "V1 a 0 DC (5)\nR1 a 0 (1k)",
            "V1 a 0 DC(5)\nR1 a 0 1k",
            "V1 a 0 (PULSE 0 1 0 1e-09)\nR1 a 0 1k",
            "V1 a 0 DC{1+1}\nR1 a 0 1k",
            "V1 a 0 (DC 5)\nR1 a 0 1k",
            "V1 a 0 1\nR1 a 0 r  =  1k",
            "V1 a 0 1\nR1 a 0 R = 'TIME > 70m ? 10k : 25k'",
            "V1 a 0 1\nR1 a 0 R = (time<1u ? 1k : 2k)",
            "V1 a 0 1\nR1 a 0 1k rm l=1u w=10u\n.model rm r tc1=0.01",
            "V1 a 0 1\nR1 a 0 2k tc=0.001,0.0001",
            "V1 a 0 1\nR1 a 0 2k TC = 0.001, 0.0001",
            "V1 a 0 1\nR1 a 0 470R\nR2 a 0 10mR\nR3 a 0 47R;1W\nC1 a 0 100n_X7R\nC2 a 0 22pF_NM\nL1 a 0 10uH_2A",
            "V1 a 0 5VDC\nR1 a 0 1k",
            "V1 a 0 1\nR1 a 0 4Meg7\nR2 a 0 4L7\nR3 a 0 2meg5\nR4 a 0 470L",
            "V1 a 0 1\nR1 a 0 10k@0402\nR2 a 0 10k%\nR3 a 0 10k#1\nR4 a 0 1k~1%\nR5 a 0 1kOhm@1%",
            "V1 a 0 1\nR1 a 0 4k7_1%\nC1 a 0 4u7_16V\nR2 a 0 4k7Ohm",
            "V1 a 0 5Volts\nV2 b 0 3.3Vrms\nV3 c 0 DC 12VRMS\nR1 a 0 1k\nR2 b 0 1k\nR3 c 0 1k",
            "I1 a 0 1mAmp\nR1 a 0 1k\nC1 a 0 10uFarad\nL1 a 0 10nHenry",
            "V1 a 0 1\nR1 a 0 1d3\nC1 a 0 1D-9",
            "V1 a 0 1\nC1 a 0 100nX7R\nR1 a 0 1k",
            "V1 a 0 SIN(0 1 1kHz 0 0 0)\nV2 b 0 PULSE(0 1 0 1ns 1ns 10us 20us)\nR1 a b 1k\nR2 b 0 1k",
            ".param p=2k\nV1 a 0 1\nR1 a 0 {p==2k ? 3k : 4k}\nC1 a 0 'p<=1k ? 3n : 4n'",
            ".param p=2k\nV1 a 0 1\nR1 a 0 R=p*2\nR2 a 0 R=V(a)*3k\nR3 a 0 R = V(a)*3k",
            "V1 a 0 SIN(0 1 1k)\nR1 a b 1k\nC1 b 0 Q='100n*V(b)'\nL1 b 0 flux='10u*I(V1)'",
            "V1 a 0 1\nR1 a 0 3k noiseless",
            "V1 a 0 1\nE1 b 0 a 0 2\nG1 b 0 a 0 {gm}\nR1 b 0 1k\nH1 c 0 V1 1k m=2\nR2 c 0 1k",
            "V1 a 0 1\nE1 b 0 a 0 gain=2\nR1 b 0 1k\nF1 c 0 V1 gain=2\nR2 c 0 1k",
            "V1 a 0 DC dc 3.3\nV2 b 0 DC AC 1\nR1 a b 1k\nR2 b 0 1k",
            "V1 a 0 DC 5 AC\nV2 b 0 DC 0 AC\nR1 a b 1k\nR2 b 0 1k",
            "V1 a 0 1\nR1 a 0 1k\nL1 a 0 1u\nL2 a 0 1u\nK1 L1 L2 (0.5)",
            ".param r=2k\nV1 a 0 1\nR1 a 0 {r}\nR2 a 0 { r }\nR3 a 0 {r*2}",
            "V1 a 0 SIN(0 1 1kHz 0 0 0)\nR1 a 0 1k",
            ".param vcc=5\nV1 a 0 PULSE(0 {vcc + 1} 0 1n 1n 1u 2u)\nV2 b 0 DC {2 * (vcc+1)}\nR1 a b 1k\nR2 b 0 1k",
            "V1 a 0 'vcc + 1'\nR1 a 0 1k",
            "V1 a 0 1\nR1 a 0 470_0603\nR2 a 0 100_1%",
            "V1 a 0 1\nR1 a 0 1k\nL1 a 0 1u\nL2 a 0 1u\nL3 a 0 1u\nK1 L1 L2 L3 0.5",
            "V1 a 0 1\nR1 a 0 1k\nL1 a 0 1u\nL2 a 0 1u\nK1 L1 L2 k=0.5",
            "V1 a 0 1\nR1 a 0 1k\nL1 a 0 1u\nL2 a 0 1u\nK1 L1 L2 0.9\nK2 L1 L2 {k}",
            "V1 a 0 1\nR1 a 0 {a + b}\nC1 a 0 'x * 2' ic=0",
            "V1 a 0 1\nR1 a 0 1e3\nC1 a 0 1E-9\nL1 a 0 1.5e-6",
            "V1 a 0 1\nR1 a 0 10kOhm\nC1 a 0 100nF\nL1 a 0 2.2uH",
            "V1 a 0 5V\nR1 a 0 1Meg\nR2 a 0 10mil",
            ".param rload=2k vcc=7 cval=1n\nV4 a 0 vcc\nR1 a 0 rload\nC1 a 0 cval",
            ".param vcc=7\nV4 a 0 DC vcc\nR1 a 0 1k",
            "V1 a 0 1\nR1 a 0 4k7\nR2 a 0 4R7\nR3 a 0 R47\nC1 a 0 4n7\nL1 a 0 2u2",
        ] {
            passes(&deck(body), "component_values");
        }
    }

    #[test]
    fn value_shorthand_counts_only_where_ngspice_reads_it() {
        let reader = |reads_shorthand| {
            component_values(
                &element::read(
                    &flatten(&DeckInputs {
                        netlist: ".title t\nV1 a 0 1\nR1 a 0 4k7\n",
                        testbench: &[],
                        project_dir: Path::new("/"),
                        allowed_roots: &[],
                        compatibility: Compatibility::DEFAULT,
                        excluded_elements: &[],
                    })
                    .unwrap(),
                    true,
                ),
                reads_shorthand,
            )
            .passed
        };
        assert_eq!(reader(true), Some(true));
        assert_eq!(reader(false), Some(false));
    }

    #[test]
    fn scale_letters_that_mislead_are_disclosed() {
        let body = "V1 a 0 1\nR1 a 0 1M\nR2 a 0 1Meg\nC1 a 0 1F\nC2 a 0 1uF\nI1 a 0 5A\nL1 a 0 1M\n\
                    R3 a 0 1MOhm\nR4 a 0 10MOHM\nR5 a 0 1mOhm\nV2 a 0 DC 0 SIN( 0 1 1MHz 0 0 0 ) AC 1\n\
                    V3 a 0 SIN(0 1 1Meg)\nI2 a 0 DC 5A\nI3 a 0 PULSE(0 2A 0 1n 1n 1u 2u)\n\
                    I4 a 0 DC 2ADC\nI5 a 0 SIN(0 2Apk 1k)\nC3 a 0 1F_5V5";
        let mut items = warned(&deck(body), "ambiguous_scale_letter");
        items.sort();
        assert_eq!(
            items,
            ["C1", "C3", "I1", "I2", "I3", "I4", "I5", "L1", "R1", "R3", "R4", "V2"]
        );
    }

    #[test]
    fn a_capital_m_is_disclosed_wherever_the_schematic_wrote_it() {
        let body = ".param big=1M fine=1Meg\nV1 a 0 1\nR1 a 0 big\nR2 a 0 fine\n\
                    .subckt pot p q r=1\nR1 p q {r}\n.ends\nX1 a 0 pot r=1MOhm\n\
                    S1 a 0 a 0 sw\n.model sw SW(ron=1 roff=1MOhm)";
        let mut items = warned(&deck(body), "ambiguous_scale_letter");
        items.sort();
        assert_eq!(items, ["X1", "big", "sw"]);
    }

    #[test]
    fn a_library_file_may_mean_milli() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        std::fs::write(root.join("m.lib"), ".model dd D(RS=10M)\n").unwrap();
        let deck = flatten(&DeckInputs {
            netlist: ".title t\n.include \"m.lib\"\nV1 a 0 1\nD1 a 0 dd\n",
            testbench: &[],
            project_dir: &root,
            allowed_roots: std::slice::from_ref(&root),
            compatibility: Compatibility::DEFAULT,
            excluded_elements: &[],
        })
        .unwrap();
        assert!(ambiguous_scales(&element::read(&deck, true)).is_empty());
    }

    #[test]
    fn a_model_with_no_parameters_is_disclosed() {
        let body = "V1 a 0 1\nD1 a 0 __D1\n.model __D1 D\nD2 a 0 __D2\n.model __D2 D(BV=5)";
        assert_eq!(warned(&deck(body), "generic_default_model"), ["__D1"]);
    }

    #[test]
    fn a_schematic_initial_condition_on_no_net_is_disclosed() {
        let body = "V1 a 0 1\nR1 a 0 1k\n.ic v(a)=1 v(typo)=2";
        assert_eq!(warned(&deck(body), "unknown_initial_node"), ["typo"]);
    }

    #[test]
    fn a_word_in_a_library_body_does_not_make_a_typo_look_deliberate() {
        let netlist =
            ".title t\n.subckt inv in out vdd\nR1 in out 1k\nR2 out vdd 1k\n.ends\nR1 /VDD 0 1k\n";
        let preflight = testbench(netlist, &["/VDD"], &["V1 VDD 0 3.3"]);
        assert_eq!(
            fails(&preflight, "testbench_nodes", "unknown_testbench_node"),
            ["vdd"]
        );
    }

    #[test]
    fn a_parameter_with_a_misleading_scale_is_disclosed() {
        let preflight = deck(".param big=1M fine=1Meg\nV1 a 0 1\nR1 a 0 big\nR2 a 0 fine");
        assert_eq!(warned(&preflight, "ambiguous_scale_letter"), ["big"]);
    }

    #[test]
    fn an_absurd_capacitance_or_inductance_is_disclosed() {
        let body = "V1 a 0 1\nR1 a 0 1k\nC1 a 0 2.2e+16\nC2 a 0 1Meg\nC3 a 0 100u\nL1 a 0 1e+6\nL2 a 0 10u\n\
                    L3 a 0 4.7\nC4 a 0 0.1\nC5 a 0 100\nC6 a 0 1e-9\nC7 a 0 100m\nV2 b 0 DC 18650\nR2 b 0 1k";
        assert_eq!(
            warned(&deck(body), "implausible_value"),
            ["C1", "C2", "L1", "L3", "C4", "C5", "V2"]
        );
    }

    #[test]
    fn a_source_with_no_value_is_disclosed() {
        assert_eq!(
            warned(&deck("VBT1 a 0\nR1 a 0 1k"), "source_without_value"),
            ["VBT1"]
        );
        assert!(warned(&deck(GOOD), "source_without_value").is_empty());
    }

    #[test]
    fn a_net_name_holding_an_equals_sign_is_refused_because_ngspice_cuts_it() {
        let kicad = KiCadNetlist {
            components: vec![component("R1"), component("R2")],
            nets: vec![net("/SEL=0", &["R1"]), net("/SEL=1", &["R2"])],
            library_parts: Vec::new(),
        };
        let netlist = ".title t\nV1 a 0 1\nR1 a /SEL=0 1k\nR2 a /SEL=1 1k\n";
        let preflight = check(
            netlist,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        assert_eq!(
            fails(&preflight, "net_names", "net_name_collision").len(),
            2
        );
    }

    #[test]
    fn a_potentiometer_is_a_resistive_part_and_gives_no_reference() {
        let pot = "V1 a n 5\nA1 a w n pp\nR1 w n 1k\n.model pp potentiometer(r=10k)";
        fails(&deck(pot), "ground", "no_ground_reference");
        let referenced = format!("{pot}\nR2 n 0 1k");
        assert!(floats(&referenced).is_empty(), "{:?}", floats(&referenced));
        assert!(warned(&deck(&referenced), "unpowered_net").is_empty());
    }

    #[test]
    fn a_reference_through_capacitors_inside_a_subcircuit_is_still_a_reference() {
        let body = ".subckt s p q\nC1 p 0 1n\nC2 q 0 1n\n.ends\nV1 a b AC 1\nR1 a b 1k\nX1 a b s";
        passes(&deck(body), "ground");
    }

    #[test]
    fn a_pin_named_gnd_anywhere_is_disclosed() {
        let body = ".subckt amp in gnd out\nR1 in gnd 1k\nR2 out gnd 1k\n.ends\nV1 a 0 1\nX1 a 0 b amp\nR3 b 0 1k";
        assert_eq!(warned(&deck(body), "gnd_pin_in_subcircuit"), ["amp"]);
    }

    #[test]
    fn checks_that_need_the_live_branch_wait_for_it() {
        let body = ".param s=2\n.if (s==1)\nRx1 a 0 1k\n.else\nRx1 a 0 Battery\n.endif\nV1 a 0 1";
        let preflight = deck(body);
        assert!(!named(&preflight, "element_names").ran);
        assert!(!named(&preflight, "component_values").ran);
    }

    #[test]
    fn every_winding_of_a_coupling_must_exist() {
        let body = "V1 a 0 1\nR1 a 0 1k\nL1 a 0 1u\nL2 a 0 1u\nK1 L1 L2 L9 0.5";
        fails(&deck(body), "element_names", "missing_referenced_element");
    }

    #[test]
    fn a_partner_inside_a_polynomial_or_a_subcircuit_is_not_read_as_missing() {
        passes(
            &deck("V1 a 0 1\nR1 a 0 1k\nF1 b 0 POLY(1) V1 0 2\nR2 b 0 1k"),
            "element_names",
        );
        passes(
            &deck("V1 a 0 1\nR1 a 0 1k\nF1 b 0 V.X1.Vs 2\nR2 b 0 1k"),
            "element_names",
        );
    }

    #[test]
    fn duplicate_names_and_missing_partners_are_refused() {
        fails(
            &deck("V1 a 0 1\nR1 a 0 1k\nr1 a 0 2k"),
            "element_names",
            "duplicate_element",
        );
        fails(
            &deck("V1 a 0 1\nR1 a 0 1k\nK1 L1 L2 0.9\nL1 a 0 1u"),
            "element_names",
            "missing_referenced_element",
        );
        fails(
            &deck("V1 a 0 1\nR1 a 0 1k\nH1 b 0 V2 2\nR2 b 0 1k"),
            "element_names",
            "missing_referenced_element",
        );
        passes(
            &deck("V1 a 0 1\nR1 a 0 1k\nH1 b 0 v1 2\nR2 b 0 1k"),
            "element_names",
        );
    }

    #[test]
    fn connectivity_says_when_it_could_not_see_the_circuit() {
        for body in [
            "Y1 a b c\nV1 a 0 1\nR1 a 0 1k",
            "X1 a b nothing_defines_this\nV1 a 0 1\nR1 a 0 1k",
            "D1 a b no_such_model\nV1 a 0 1\nR1 a 0 1k",
            "Q1 a b c no_such_model\nV1 a 0 1\nR1 a 0 1k",
            ".if (1)\nR2 a 0 1k\n.endif\nV1 a 0 1\nR1 a 0 1k",
            ".if(1)\nR2 a 0 1k\n.endif\nV1 a 0 1\nR1 a 0 1k",
        ] {
            let preflight = deck(body);
            let connectivity = named(&preflight, "connectivity");
            assert!(!connectivity.ran, "{body}");
            assert_eq!(connectivity.passed, None, "{body}");
            assert_eq!(connectivity.code, Some("connectivity_not_checked"));
        }
    }

    #[test]
    fn a_mistyped_testbench_node_is_refused() {
        let kicad = KiCadNetlist {
            components: Vec::new(),
            nets: ["/in", "/out"]
                .map(|name| Net {
                    name: name.to_string(),
                    pins: Vec::new(),
                })
                .to_vec(),
            library_parts: Vec::new(),
        };
        let netlist = ".title t\nR1 /in /out 1k\nR2 /out 0 1k\n";
        let setup = |line: &str, kicad: &KiCadNetlist| Setup {
            kicad: kicad.clone(),
            testbench: vec![line.to_string()],
            ..Setup::default()
        };
        let typo = check(netlist, setup("V1 /inn 0 5", &kicad));
        assert_eq!(
            fails(&typo, "testbench_nodes", "unknown_testbench_node"),
            ["/inn"]
        );
        passes(
            &check(netlist, setup("V1 /in 0 5", &kicad)),
            "testbench_nodes",
        );
        passes(
            &check(netlist, setup("V1 /IN 0 5", &kicad)),
            "testbench_nodes",
        );
    }

    fn testbench(netlist: &str, nets: &[&str], lines: &[&str]) -> Preflight {
        let kicad = KiCadNetlist {
            components: Vec::new(),
            nets: nets.iter().map(|name| net(name, &[])).collect(),
            library_parts: Vec::new(),
        };
        let testbench = lines.iter().map(|line| line.to_string()).collect();
        check(
            netlist,
            Setup {
                kicad,
                testbench,
                ..Setup::default()
            },
        )
    }

    #[test]
    fn a_typo_is_caught_in_every_way_a_testbench_can_use_a_node() {
        let netlist = ".title t\nR1 /in /out 1k\nR2 /out 0 1k\n.model g gain(gain=2)\n";
        for (lines, typo) in [
            (vec!["V1 /in 0 5", ".ic v(/outt)=3"], "/outt"),
            (vec!["V1 /in 0 5", "A9 %v(/in) %v(/outt) g"], "/outt"),
            (vec!["V1 /ou 0 5", "B2 /in 0 V=v(/out)"], "/ou"),
            (vec!["V1 12 0 SIN(12 1 1k)"], "12"),
        ] {
            let preflight = testbench(netlist, &["/in", "/out"], &lines);
            let items = fails(&preflight, "testbench_nodes", "unknown_testbench_node");
            assert_eq!(items, [typo], "{lines:?}");
        }
    }

    #[test]
    fn testbench_nodes_does_not_guess_when_it_could_not_read_a_line() {
        let preflight = testbench(
            ".title t\nR1 /in 0 1k\n",
            &["/in"],
            &["V1 sup 0 5", "Y1 sup /in x"],
        );
        assert!(!named(&preflight, "testbench_nodes").ran);
    }

    #[test]
    fn a_testbench_of_only_directives_is_still_checked() {
        let preflight = testbench(
            ".title t\nV1 /in 0 1\nR1 /in 0 1k\n",
            &["/in"],
            &[".ic v(/inn)=3"],
        );
        fails(&preflight, "testbench_nodes", "unknown_testbench_node");
    }

    /// A circuit whose nets are `/in` and `/out`, with the given files on disk
    /// for `.include`, and the testbench and netlist lines as given.
    fn with_files(netlist: &str, files: &[(&str, &str)], lines: &[&str]) -> Preflight {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        for (name, text) in files {
            std::fs::write(root.join(name), text).unwrap();
        }
        let kicad = KiCadNetlist {
            components: Vec::new(),
            nets: ["/in", "/out"].map(|name| net(name, &[])).to_vec(),
            library_parts: Vec::new(),
        };
        check_in(
            netlist,
            Setup {
                kicad,
                testbench: lines.iter().map(|line| line.to_string()).collect(),
                ..Setup::default()
            },
            &root,
            std::slice::from_ref(&root),
        )
    }

    const TWO_NETS: &str = ".title t\nR1 /in /out 1k\nR2 /out 0 1k\n";

    #[test]
    fn a_top_level_element_in_a_file_the_testbench_includes_counts() {
        let preflight = with_files(
            TWO_NETS,
            &[("load.sp", "R9 /out mid 1k\nC9 mid 0 1u\n")],
            &["V1 /in 0 5", ".include \"load.sp\""],
        );
        passes(&preflight, "testbench_nodes");
    }

    #[test]
    fn a_typo_inside_a_file_the_testbench_includes_is_still_refused() {
        let preflight = with_files(
            TWO_NETS,
            &[("load.sp", "R9 /out mid 1k\nC9 midd 0 1u\n")],
            &["V1 /in 0 5", ".include \"load.sp\""],
        );
        assert_eq!(
            fails(&preflight, "testbench_nodes", "unknown_testbench_node"),
            ["mid", "midd"]
        );
    }

    #[test]
    fn only_an_element_in_the_testbench_file_makes_a_node_look_deliberate() {
        // A model line holding the word, and a library the schematic includes,
        // are not a second card on the node.
        for (netlist, files, lines) in [
            (
                TWO_NETS,
                vec![("m.sp", ".model vv D(bv=5)\n")],
                vec!["V1 /in 0 5", "R9 /in vv 1k", ".include \"m.sp\""],
            ),
            (
                ".title t\n.include \"lib.sp\"\nR1 /in /out 1k\nR2 /out 0 1k\n",
                vec![("lib.sp", "R8 vv 0 1k\n")],
                vec!["V1 /in 0 5", "R9 /in vv 1k"],
            ),
        ] {
            let preflight = with_files(netlist, &files, &lines);
            assert_eq!(
                fails(&preflight, "testbench_nodes", "unknown_testbench_node"),
                ["vv"],
                "{lines:?}"
            );
        }
    }

    const RC_WITH_IC: &str = ".subckt rc p q\nR1 p int 1k\nC1 int q 1u\n.ic v(int)=1\n.ends\n";

    #[test]
    fn an_initial_condition_inside_a_subcircuit_is_not_a_typo() {
        let body = format!("V1 a 0 1\nR1 a 0 1k\n{RC_WITH_IC}X1 a 0 rc");
        assert!(warned(&deck(&body), "unknown_initial_node").is_empty());
        let lines = [".subckt rc p q", "R1 p int 1k", "C1 int q 1u"]
            .into_iter()
            .chain([".ic v(int)=1", ".ends", "X9 /out 0 rc", "V1 /in 0 5"]);
        let lines: Vec<&str> = lines.collect();
        passes(&with_files(TWO_NETS, &[], &lines), "testbench_nodes");
    }

    #[test]
    fn an_initial_condition_after_a_subcircuit_is_still_checked() {
        // The subcircuit's own node means nothing at the top level, and the
        // check must not stay off once `.ends` has closed the block.
        let body = format!("V1 a 0 1\nR1 a 0 1k\n{RC_WITH_IC}X1 a 0 rc\n.ic v(int)=1 v(typo)=2");
        assert_eq!(
            warned(&deck(&body), "unknown_initial_node"),
            ["int", "typo"]
        );
        let lines = [
            ".subckt rc p q",
            "R1 p int 1k",
            "C1 int q 1u",
            ".ic v(int)=1",
            ".ends",
            "X9 /out 0 rc",
            "V1 /in 0 5",
            ".ic v(int)=2",
        ];
        assert_eq!(
            fails(
                &with_files(TWO_NETS, &[], &lines),
                "testbench_nodes",
                "unknown_testbench_node"
            ),
            ["int"]
        );
    }

    #[test]
    fn a_save_a_spaced_read_a_global_or_an_inner_node_is_deliberate() {
        let netlist = ".title t\nV1 /in 0 1\nR1 /in 0 1k\n.subckt rr p q\nR1 p int 1k\nR2 int q 1k\n.ends\nX7 /in 0 rr\n";
        for lines in [
            vec!["R9 /in probe 1k", ".save probe"],
            vec!["V2 sense 0 5", "B1 /in 0 V=V( sense )*2"],
            vec![".global vcc", "V2 vcc 0 5"],
            vec!["R9 x7.int 0 1k"],
        ] {
            passes(&testbench(netlist, &["/in"], &lines), "testbench_nodes");
        }
    }

    #[test]
    fn a_new_testbench_node_used_twice_or_read_by_an_expression_is_deliberate() {
        let netlist = ".title t\nR1 /in 0 1k\n";
        let setup = |lines: &[&str]| Setup {
            testbench: lines.iter().map(|line| line.to_string()).collect(),
            kicad: KiCadNetlist {
                components: Vec::new(),
                nets: vec![Net {
                    name: "/in".to_string(),
                    pins: Vec::new(),
                }],
                library_parts: Vec::new(),
            },
            ..Setup::default()
        };
        passes(
            &check(netlist, setup(&["V1 sup 0 5", "R9 sup /in 1"])),
            "testbench_nodes",
        );
        passes(
            &check(netlist, setup(&["V1 sense 0 5", "B1 /in 0 V=v(sense)*2"])),
            "testbench_nodes",
        );
    }

    #[test]
    fn with_no_testbench_lines_there_is_no_testbench_check() {
        assert!(!deck(GOOD)
            .checks
            .iter()
            .any(|check| check.name == "testbench_nodes"));
    }

    #[test]
    fn digital_models_are_refused_only_when_they_are_off() {
        let netlist = ".title t\nV1 a 0 1\nR1 a 0 1k\nA1 [d1] [d2] inv\n.model inv d_inverter\nA2 [a] [d1] ad\n.model ad adc_bridge\n.model unused d_and\n";
        let off = check(
            netlist,
            Setup {
                allow_digital_models: false,
                ..Setup::default()
            },
        );
        assert_eq!(
            fails(&off, "code_models", "digital_models_disabled"),
            ["ad", "inv"]
        );
        passes(&check(netlist, Setup::default()), "code_models");
    }

    #[test]
    fn a_digital_model_nothing_uses_is_not_refused() {
        let netlist = ".title t\nV1 a 0 1\nR1 a 0 1k\n.model inv d_inverter\n";
        let setup = Setup {
            allow_digital_models: false,
            ..Setup::default()
        };
        passes(&check(netlist, setup), "code_models");
    }

    #[test]
    fn a_digital_model_in_a_body_nothing_calls_is_not_refused() {
        let netlist = ".title t\nV1 a 0 1\nR1 a 0 1k\n.subckt lib p\nA1 [p] [q] inv\n.model inv d_inverter\n.ends\n";
        let setup = Setup {
            allow_digital_models: false,
            ..Setup::default()
        };
        passes(&check(netlist, setup), "code_models");
    }

    #[test]
    fn a_digital_model_used_inside_a_subcircuit_is_refused() {
        let netlist = ".title t\nV1 a 0 1\nR1 a 0 1k\n.subckt s p\nA1 [p] [q] bb\n.model bb bidi_bridge\n.ends\nX1 a s\n";
        let setup = Setup {
            allow_digital_models: false,
            ..Setup::default()
        };
        assert_eq!(
            fails(
                &check(netlist, setup),
                "code_models",
                "digital_models_disabled"
            ),
            ["bb"]
        );
    }

    #[test]
    fn no_code_models_is_a_warning() {
        let preflight = check(
            ".title t\nV1 a 0 1\nR1 a 0 1k\n",
            Setup {
                code_model_count: 0,
                ..Setup::default()
            },
        );
        assert!(!warned(&preflight, "code_models_missing").is_empty());
        assert!(warned(&deck(GOOD), "code_models_missing").is_empty());
    }

    #[test]
    fn a_group_with_no_source_is_a_warning() {
        let preflight = deck("V1 in 0 1\nR1 in 0 1k\nR2 out 0 1k\nC1 out x 1n\nR3 x 0 1k");
        assert_eq!(warned(&preflight, "unpowered_net"), ["out", "x"]);
        assert!(warned(&deck(GOOD), "unpowered_net").is_empty());
    }

    #[test]
    fn a_dependent_source_or_a_passive_subcircuit_powers_nothing() {
        let preflight = deck("V1 a 0 1\nR3 a 0 1k\nR1 c 0 1k\nE1 b 0 c 0 2\nR2 b 0 1k");
        assert_eq!(warned(&preflight, "unpowered_net"), ["b", "c"]);
        let preflight =
            deck(".subckt s p q\nR1 p q 1k\n.ends\nV1 a 0 1\nR3 a 0 1k\nX1 b 0 s\nR2 b 0 1k");
        assert_eq!(warned(&preflight, "unpowered_net"), ["b"]);
        let preflight =
            deck(".subckt s p q\nV1 p q 1\n.ends\nV1 a 0 1\nR3 a 0 1k\nX1 b 0 s\nR2 b 0 1k");
        assert!(warned(&preflight, "unpowered_net").is_empty());
    }

    #[test]
    fn a_large_deck_stays_fast() {
        let mut body = String::from("V0 n0 0 1\n");
        for at in 0..5000 {
            body.push_str(&format!("R{at} n{at} n{} 1\nI{at} n{at} 0 1u\n", at + 1));
        }
        for at in 0..2500 {
            body.push_str(&format!("RQ{at} q{at} 0 1\n"));
        }
        let started = std::time::Instant::now();
        assert_eq!(warned(&deck(&body), "unpowered_net").len(), 2500);
        assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
    }

    #[test]
    fn a_coupled_winding_is_powered_through_its_core() {
        let body = "V1 in 0 1\nL1 in 0 1u\nL2 out 0 1u\nK1 L1 L2 0.9\nR1 out 0 1k";
        assert!(warned(&deck(body), "unpowered_net").is_empty());
        let uncoupled = "V1 in 0 1\nL1 in 0 1u\nL2 out 0 1u\nR1 out 0 1k";
        assert_eq!(warned(&deck(uncoupled), "unpowered_net"), ["out"]);
    }

    #[test]
    fn plain_parts_are_told_from_special_ones() {
        for part in [
            "R",
            "R_Small_US",
            "R-Device",
            "L_Small_1",
            "C_Polarized",
            "CP",
        ] {
            assert!(is_plain_part(part), "{part}");
        }
        for part in ["R_Potentiometer", "R_NTC", "Thermistor_NTC", "L_Iron", ""] {
            assert!(!is_plain_part(part), "{part}");
        }
    }

    #[test]
    fn a_short_pulse_is_a_warning_and_a_full_one_is_not() {
        let preflight = deck(
            "V1 a 0 PULSE(0 1 0 1n 1n 1u)\nV2 b 0 PULSE(0 1 0 1n 1n 1u 2u)\nR1 a b 1k\nR2 b 0 1k",
        );
        assert_eq!(warned(&preflight, "pulse_period_defaulted"), ["V1"]);
    }

    #[test]
    fn a_part_modelled_from_its_value_alone_is_disclosed() {
        let component = |reference: &str, part: &str, has_sim_fields: bool| Component {
            reference: reference.to_string(),
            value: "10k".to_string(),
            lib_id: Some(format!("Device:{part}")),
            has_sim_fields,
            sim_params: None,
        };
        let kicad = KiCadNetlist {
            components: vec![
                component("R1", "R", false),
                component("R2", "R_Small_US", false),
                component("RT1", "Thermistor_NTC", false),
                component("R3", "R_Potentiometer", true),
            ],
            nets: Vec::new(),
            library_parts: Vec::new(),
        };
        let netlist = ".title t\nR1 a 0 10k\nR2 a 0 10k\nRT1 a 0 10k\nR3 a 0 10k\nV1 a 0 1\n";
        let preflight = check(
            netlist,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        assert_eq!(warned(&preflight, "inferred_from_value"), ["RT1"]);
    }

    #[test]
    fn a_varistor_at_its_library_default_is_disclosed() {
        let params = Some("threshold=1k".to_string());
        let varistor = |reference: &str, sim_params: Option<String>| Component {
            reference: reference.to_string(),
            value: "x".to_string(),
            lib_id: Some("Device:Varistor".to_string()),
            has_sim_fields: true,
            sim_params,
        };
        let kicad = KiCadNetlist {
            components: vec![
                varistor("RV1", params.clone()),
                varistor("RV2", Some("threshold=33".to_string())),
            ],
            nets: Vec::new(),
            library_parts: vec![LibraryPart {
                lib_id: "Device:Varistor".to_string(),
                has_sim_fields: true,
                sim_params: params,
            }],
        };
        let netlist = ".title t\nV1 a 0 1\nR1 a 0 1k\n";
        let preflight = check(
            netlist,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        assert_eq!(warned(&preflight, "generic_default_model"), ["RV1"]);
    }

    #[test]
    fn a_local_net_named_like_ground_is_disclosed_and_a_global_one_is_not() {
        let net = |name: &str| NetEntry {
            kicad_name: name.to_string(),
            spice_name: name.to_string(),
            in_deck: true,
            pin_count: 1,
        };
        let nets = [
            net("/gnd"),
            net("/sub/GND"),
            net("/0"),
            net("GND"),
            net("/gnd_sense"),
        ];
        assert_eq!(ground_like_local_nets(&nets), ["/gnd", "/sub/GND", "/0"]);
    }

    #[test]
    fn a_node_no_kicad_net_explains_is_disclosed() {
        let kicad = KiCadNetlist {
            components: Vec::new(),
            nets: vec![Net {
                name: "/in".to_string(),
                pins: Vec::new(),
            }],
            library_parts: Vec::new(),
        };
        let netlist = ".title t\nV1 /in 0 1\nR1 /in mystery 1k\nR2 mystery 0 1k\n";
        let preflight = check(
            netlist,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        assert_eq!(warned(&preflight, "net_map_mismatch"), ["mystery"]);
    }

    #[test]
    fn an_exclusion_that_matches_nothing_is_disclosed() {
        let setup = Setup {
            unmatched_exclusions: vec!["J*".to_string()],
            ..Setup::default()
        };
        let preflight = check(".title t\nV1 a 0 1\nR1 a 0 1k\n", setup);
        assert_eq!(warned(&preflight, "exclusion_matched_nothing"), ["J*"]);
    }

    #[test]
    fn comments_and_continuations_are_read_as_ngspice_reads_them() {
        let preflight = deck("V1 a 0 1 ; note\nR1 a\n+ 0 1k ; 5%");
        passes(&preflight, "connectivity");
        passes(&preflight, "component_values");
        assert!(floats("V1 a,0,1\nR1 a,0,1k").is_empty());
    }

    #[test]
    fn a_model_is_found_in_cmodel_lines_and_when_its_parameters_touch_its_type() {
        passes(
            &deck("V1 a 0 1\nR1 a 0 rm\n.model rm R(r=1k)"),
            "component_values",
        );
        assert!(
            named(
                &deck("V1 a 0 1\nD1 a 0 dd\n.cmodel dd D(IS=1n)"),
                "connectivity"
            )
            .ran
        );
    }

    #[test]
    fn a_jfet_or_code_model_with_no_model_is_unread() {
        for body in ["J1 a b c nothing", "A1 a b nothing"] {
            let preflight = deck(&format!("V1 a 0 1\nR1 a 0 1k\n{body}"));
            assert!(!named(&preflight, "connectivity").ran, "{body}");
        }
    }

    #[test]
    fn a_node_the_testbench_added_is_not_a_net_map_mismatch() {
        let preflight = testbench(
            ".title t\nV1 /in 0 1\nR1 /in 0 1k\n",
            &["/in"],
            &["V2 sup 0 5", "R9 sup /in 1"],
        );
        assert!(warned(&preflight, "net_map_mismatch").is_empty());
    }

    #[test]
    fn a_warning_is_not_raised_from_a_circuit_that_was_not_read() {
        let body = "Y1 a b c\nV1 x 0 1\nR1 q 0 1k\nC1 z y 1n";
        assert!(warned(&deck(body), "unpowered_net").is_empty());
        assert!(floats(body).is_empty());
        let conditional = ".if (1)\nR2 a 0 1k\n.endif\nV1 x 0 1\nR1 q 0 1k\nC1 z y 1n";
        assert!(floats(conditional).is_empty());
    }

    #[test]
    fn unmodelled_symbols_silence_the_floating_warning_rather_than_faking_it() {
        let kicad = KiCadNetlist {
            components: vec![component("J1")],
            nets: Vec::new(),
            library_parts: Vec::new(),
        };
        let netlist = ".title t\nJ1 __J1\nV1 a 0 1\nR1 a 0 1k\nC1 x y 1n\n";
        let preflight = check(
            netlist,
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        assert!(warned(&preflight, "floating_subcircuit").is_empty());
        assert!(!named(&preflight, "connectivity").ran);
    }

    #[test]
    fn a_long_floating_group_is_cut_to_a_readable_item() {
        let mut body = String::from("V1 a 0 1\nR0 a 0 1k\n");
        for at in 0..30 {
            body.push_str(&format!("R{} n{at} n{} 1\n", at + 1, at + 1));
        }
        let items = floats(&body);
        assert_eq!(items.len(), 1);
        assert!(items[0].ends_with("and 23 more"), "{}", items[0]);
    }

    #[test]
    fn transmission_lines_join_port_to_port() {
        assert!(floats("V1 a 0 1\nR1 a 0 1k\nT1 a 0 b 0 z0=50 td=1n\nR2 b 0 1k").is_empty());
        let floating = "V1 a 0 1\nR1 a 0 1k\nT1 x y z w z0=50 td=1n\nC1 x a 1n\nC2 y 0 1n";
        assert_eq!(floats(floating), ["w, y", "x, z"]);
    }

    #[test]
    fn numbers_and_their_scales_read_as_ngspice_reads_them() {
        assert_eq!(magnitude("1k"), Some(1e3));
        assert_eq!(magnitude("2.2e+16"), Some(2.2e16));
        assert_eq!(magnitude("1Meg"), Some(1e6));
        assert_eq!(magnitude("1.5u"), Some(1.5e-6));
        assert_eq!(magnitude("x"), None);
        assert_eq!(magnitude("1d3"), Some(1e3));
    }

    #[test]
    fn nets_named_like_ground_are_offered_in_every_spelling() {
        let kicad = KiCadNetlist {
            components: Vec::new(),
            nets: ["0", "/sub/0", "/sig"].map(|name| net(name, &[])).to_vec(),
            library_parts: Vec::new(),
        };
        let preflight = check(
            ".title t\nV1 a b 5\nR1 a b 1k\n",
            Setup {
                kicad,
                ..Setup::default()
            },
        );
        assert_eq!(
            fails(&preflight, "ground", "no_ground_reference"),
            ["0", "/sub/0"]
        );
    }

    #[test]
    fn a_parenthesised_value_and_comma_separated_fields_are_read() {
        passes(&deck("V1 a 0 1\nR1 a 0 (1k)"), "component_values");
        passes(&deck("V1 a,0,1\nR1 a,0,1k"), "ground");
    }

    #[test]
    fn a_body_that_could_not_be_read_silences_warnings_about_its_pins() {
        let body = ".subckt s p q\nY1 p q z\n.ends\nV1 a 0 1\nR1 a 0 1k\nC1 a x 1n\nX1 x y s";
        assert!(floats(body).is_empty(), "{:?}", floats(body));
        assert!(warned(&deck(body), "unpowered_net").is_empty());
    }

    #[test]
    fn a_time_function_is_a_source() {}

    #[test]
    fn a_polynomial_source_keeps_its_two_output_nodes() {
        let preflight = deck("V1 a 0 1\nR1 a 0 1k\nE1 b 0 POLY(1) a 0 0 2 3\nR2 b 0 1k");
        assert!(warned(&preflight, "net_map_mismatch").len() <= 2);
        assert_eq!(named(&preflight, "connectivity").passed, Some(true));
        let elements = element::read(
            &flatten(&DeckInputs {
                netlist: ".title t\nE1 b 0 POLY(1) a 0 0 2 3\n",
                testbench: &[],
                project_dir: Path::new("/"),
                allowed_roots: &[],
                compatibility: Compatibility::DEFAULT,
                excluded_elements: &[],
            })
            .unwrap(),
            true,
        );
        assert_eq!(elements.elements[0].nodes, ["b", "0"]);
    }

    #[test]
    fn an_old_farad_spelling_is_disclosed() {
        let body = "V1 a 0 1\nR1 a 0 1k\nC1 a 0 10mfd\nC2 a 0 1Farad";
        assert_eq!(warned(&deck(body), "ambiguous_scale_letter"), ["C1", "C2"]);
    }

    #[test]
    fn a_subcircuit_is_read_pin_by_pin() {
        let amp = ".subckt amp in out\nE1 out 0 in 0 1\n.ends\n";
        let body = format!("{amp}V1 a 0 1\nC1 a x 1n\nX1 x o amp\nR2 o 0 1k");
        assert_eq!(
            floats(&body),
            ["x"],
            "an amplifier input is not a path to ground"
        );
        let biased = format!("{amp}V1 a 0 1\nC1 a x 1n\nR1 x 0 1k\nX1 x o amp\nR2 o 0 1k");
        assert!(floats(&biased).is_empty());
    }

    #[test]
    fn pins_joined_inside_a_body_join_outside_it() {
        let wire = ".subckt wire p q\nR1 p q 1\n.ends\n";
        let body = format!("{wire}V1 a 0 1\nR1 a 0 1k\nC1 a x 1n\nC2 a y 1n\nX1 x y wire");
        assert_eq!(floats(&body), ["x, y"]);
        let open = ".subckt open p q\nC1 p q 1n\n.ends\n";
        let body = format!("{open}V1 a 0 1\nR1 a 0 1k\nC1 a x 1n\nX1 x y open\nR2 y 0 1k");
        assert_eq!(floats(&body), ["x"]);
    }

    #[test]
    fn a_nested_or_recursive_subcircuit_does_not_hang() {
        let nested = ".subckt in p q\nR1 p 0 1k\n.ends\n.subckt out p q\nX1 p q in\n.ends\n";
        let body = format!("{nested}V1 a 0 1\nR9 a 0 1k\nC1 a x 1n\nX2 x y out\nC2 y a 1n");
        assert_eq!(floats(&body), ["y"]);
        let looped = ".subckt s p\nX1 p s\n.ends\nV1 a 0 1\nR1 a 0 1k\nC1 a x 1n\nX2 x s";
        assert!(floats(looped).is_empty());
    }

    #[test]
    fn a_call_with_the_wrong_pin_count_counts_as_joined() {
        let body = ".subckt s p q\nC1 p q 1n\n.ends\nV1 a 0 1\nR1 a 0 1k\nC1 a x 1n\nX1 x y z s";
        assert!(floats(body).is_empty());
    }

    #[test]
    fn a_digital_part_gives_no_reference_and_its_nodes_do_not_float() {
        let body = "V1 a pgnd 5\nR1 a pgnd 1k\nA1 [a] [d] adc\nA2 d e buf\n.model adc adc_bridge\n.model buf d_buffer";
        fails(&deck(body), "ground", "no_ground_reference");
        let grounded = format!("{body}\nR2 pgnd 0 1k");
        assert!(floats(&grounded).is_empty(), "{:?}", floats(&grounded));
        assert!(warned(&deck(&grounded), "unpowered_net").is_empty());
    }

    #[test]
    fn a_top_level_code_model_as_the_only_ground_is_disclosed() {
        let body = "V1 a b 5\nA1 a b g\n.model g gain(gain=2)";
        assert_eq!(warned(&deck(body), "ground_only_inside_subcircuit"), ["A1"]);
    }

    #[test]
    fn a_parameter_with_an_atto_or_femto_letter_is_disclosed() {
        let body = ".param il=2A cs=1F ok=1k\nI1 0 a {il}\nR1 a 0 1k";
        assert_eq!(warned(&deck(body), "ambiguous_scale_letter"), ["il", "cs"]);
    }

    #[test]
    fn a_comma_in_a_waveform_is_disclosed_even_with_no_unit() {
        let body = "V1 a 0 SIN(0 1,5 1k)\nV2 b 0 PULSE(0 1,5 0 1n 1n 1m 2m)\nR1 a b 1k\nR2 b 0 1k";
        assert_eq!(warned(&deck(body), "decimal_comma"), ["V1", "V2"]);
    }

    #[test]
    fn a_source_or_ground_in_a_nested_subcircuit_counts() {
        let body = ".subckt outer p\nX1 p inner\n.ends\n.subckt inner p\nV1 p 0 1\n.ends\nX2 a outer\nR1 a b 1k";
        assert!(warned(&deck(body), "unpowered_net").is_empty());
        passes(&deck(body), "ground");
    }

    #[test]
    fn a_global_node_inside_a_body_connects_to_the_rest_of_the_circuit() {
        let body =
            ".global vcc\n.subckt pull p\nR1 p vcc 1k\n.ends\nV1 vcc 0 5\nX1 in pull\nR2 in out 1k";
        assert!(floats(body).is_empty(), "{:?}", floats(body));
        assert!(warned(&deck(body), "unpowered_net").is_empty());
    }

    #[test]
    fn a_behavioural_or_current_controlled_source_is_powered_through_what_it_reads() {
        for line in [
            "B1 o 0 V=V(a)*2",
            "F1 o 0 V1 2",
            "H1 o 0 V1 2",
            "E1 o 0 VALUE={V(a)*2}",
        ] {
            let body = format!("V1 a 0 1\nR1 a 0 1k\n{line}\nR2 o 0 1k");
            assert!(warned(&deck(&body), "unpowered_net").is_empty(), "{line}");
        }
    }

    #[test]
    fn a_model_and_a_subcircuit_may_share_a_name_and_spacing_is_no_difference() {
        let body = "V1 a 0 1\nR1 a 0 1k\n.model dd D(IS=1n)\n.model dd  D(IS=1n)\n.subckt dd p\nR1 p 0 1k\n.ends";
        assert!(warned(&deck(body), "duplicate_definition").is_empty());
    }

    #[test]
    fn brackets_around_nodes_are_legal() {
        passes(&deck("V1 (a) (0) 5\nR1 a 0 1k"), "ground");
        passes(&deck("V1 a 0 5\nR1 (a 0) 1k"), "ground");
        passes(&deck("V1 a 0 5\nR1 (a 0) 1k"), "component_values");
    }

    #[test]
    fn a_ground_only_inside_a_subcircuit_is_disclosed() {
        let body = ".subckt amp p q\nE1 q 0 p 0 2\n.ends\nV1 a GNDA 1\nX1 a b amp\nR1 b GNDA 1k";
        let preflight = deck(body);
        passes(&preflight, "ground");
        assert_eq!(warned(&preflight, "ground_only_inside_subcircuit"), ["X1"]);
        let grounded = format!("{body}\nR2 GNDA 0 1k");
        assert!(warned(&deck(&grounded), "ground_only_inside_subcircuit").is_empty());
    }

    #[test]
    fn a_decimal_comma_with_a_unit_is_disclosed() {
        let body = "V1 a 0 SIN(0 1,5V 1k)\nV2 b 0 PWL(0,0 1,1)\nR1 a b 1k\nR2 b 0 1k";
        assert_eq!(warned(&deck(body), "decimal_comma"), ["V1"]);
    }

    #[test]
    fn two_different_definitions_under_one_name_are_disclosed() {
        let body = "V1 a 0 1\nR1 a 0 1k\n.model dd D(IS=1n)\n.model dd D(IS=2n)\n\
                    .subckt s p\nR1 p 0 1k\n.ends\n.subckt s p\nR1 p 0 2k\n.ends\n\
                    .model ee D(IS=1n)\n.model ee D(IS=1n)";
        let mut items = warned(&deck(body), "duplicate_definition");
        items.sort();
        assert_eq!(items, ["dd", "s"]);
    }

    #[test]
    fn a_parameter_inside_a_subcircuit_is_not_a_top_level_value() {
        let body = ".subckt v a b\n.param R=1k\nR9 a b {R}\n.ends\nV1 a 0 1\nR1 a 0 R\nX1 a 0 v";
        fails(&deck(body), "component_values", "invalid_component_value");
    }

    #[test]
    fn a_vdmos_with_no_parameters_is_disclosed() {
        let body = "V1 a 0 1\nR1 a b 1k\nMQ1 a b 0 __Q1\n.model __Q1 VDMOS NCHAN";
        assert_eq!(warned(&deck(body), "generic_default_model"), ["__Q1"]);
    }

    #[test]
    fn a_library_global_counts_as_naming_its_node() {
        let netlist = ".title t\n.global vdd\nR1 /in 0 1k\n";
        passes(
            &testbench(netlist, &["/in"], &["V1 vdd 0 5"]),
            "testbench_nodes",
        );
    }

    #[test]
    fn a_long_list_of_unreadable_elements_is_summarised() {
        let names: Vec<String> = (0..12).map(|n| format!("Y{n}")).collect();
        assert!(super::named(&names).ends_with("and 4 more"));
    }
}
