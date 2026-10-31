//! Read a flat deck into elements and the nodes they touch.
//!
//! This reads only what preflight needs. A line it cannot place is listed as
//! unparsed, so a check that could not see the whole circuit says so instead of
//! passing.

use super::flatten::{comment_start, directive, FlatDeck};
use std::collections::{HashMap, HashSet};

/// ngspice's digital code models, from its `modpath.lst`.
pub(crate) const DIGITAL_MODELS: [&str; 32] = [
    "adc_bridge",
    "dac_bridge",
    "bidi_bridge",
    "d_and",
    "d_buffer",
    "d_dff",
    "d_dlatch",
    "d_fdiv",
    "d_genlut",
    "d_inverter",
    "d_jkff",
    "d_lut",
    "d_nand",
    "d_nor",
    "d_open_c",
    "d_open_e",
    "d_or",
    "d_osc",
    "d_process",
    "d_pulldown",
    "d_pullup",
    "d_pwm",
    "d_ram",
    "d_source",
    "d_srff",
    "d_srlatch",
    "d_state",
    "d_tff",
    "d_tristate",
    "d_xnor",
    "d_xor",
    "d_cosim",
];
const MAX_BIPOLAR_TOKENS: usize = 6;
const MAX_MOS_TOKENS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Element {
    pub name: String,
    /// The device letter, lowercase. `p` is a potentiometer and `n` a digital
    /// code model, both written as `A` lines in the deck.
    pub kind: char,
    /// Lowercase, as ngspice stores them, with ground written as `0`.
    pub nodes: Vec<String>,
    /// What follows the nodes, as written.
    pub args: Vec<String>,
    pub from_testbench: bool,
}

impl Element {
    /// Node index pairs joined at DC. A MOS gate, a controlled source's input
    /// and a switch's control carry no DC, and neither do capacitors or current
    /// sources. Code models and subcircuits are read by [`DcPaths`].
    pub(crate) fn dc_pairs(&self) -> Vec<(usize, usize)> {
        let chain = |count: usize| (1..count).map(|at| (at - 1, at)).collect();
        match self.kind {
            'c' | 'i' | 'g' | 'f' | 'k' | 'a' | 'n' | 'x' => Vec::new(),
            'b' if self.is_behavioural_current() => Vec::new(),
            'm' => {
                let mut pairs = vec![(0, 2)];
                pairs.extend((3..self.nodes.len()).map(|at| (2, at)));
                pairs
            }
            'e' | 'h' | 's' | 'w' => vec![(0, 1)],
            't' | 'o' => vec![(0, 2), (1, 3)],
            _ => chain(self.nodes.len()),
        }
    }

    fn is_behavioural_current(&self) -> bool {
        self.args
            .first()
            .is_some_and(|word| word.to_lowercase().starts_with("i="))
    }
}

/// What a subcircuit's body can do for the circuit around it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Subcircuit {
    /// Any element on node `0`, which is a reference for AC and transient.
    pub touches_ground: bool,
    pub has_source: bool,
    calls: Vec<String>,
    code_models: Vec<String>,
    pins: Vec<String>,
    body: Vec<Element>,
    /// Holds a line the reader could not place, so its paths are unknown.
    unread: bool,
}

/// How a subcircuit's pins connect at DC: which share a path, by group number,
/// and which reach ground.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PinPaths {
    groups: Vec<usize>,
    grounded: Vec<bool>,
    /// The `.global` nodes each pin reaches.
    globals: Vec<Vec<String>>,
}

/// Adds elements to DC groups, looking inside each subcircuit once.
pub(crate) struct DcPaths<'a> {
    circuit: &'a Circuit,
    solved: HashMap<String, Option<PinPaths>>,
    open: Vec<String>,
}

impl<'a> DcPaths<'a> {
    pub(crate) fn new(circuit: &'a Circuit) -> Self {
        Self {
            circuit,
            solved: HashMap::new(),
            open: Vec::new(),
        }
    }

    pub(crate) fn add(&mut self, element: &Element, groups: &mut Groups) {
        // Digital nodes are events, not voltages, so they float by design.
        if element.kind == 'n' {
            return;
        }
        for node in &element.nodes {
            groups.add(node);
        }
        for (first, second) in element.dc_pairs() {
            groups.join(&element.nodes[first], &element.nodes[second]);
        }
        match element.kind {
            'a' => {
                for node in &element.nodes {
                    groups.join(node, "0");
                }
            }
            'x' => self.add_call(element, groups),
            _ => {}
        }
    }

    /// A body the reader could not follow counts as joining and grounding every
    /// pin. That can only silence a warning.
    fn add_call(&mut self, element: &Element, groups: &mut Groups) {
        let name = element.args.first().map(|name| name.to_lowercase());
        let paths = name.and_then(|name| self.solve(&name));
        let paths = paths.filter(|paths| paths.groups.len() == element.nodes.len());
        let Some(paths) = paths else {
            for node in &element.nodes {
                groups.join(node, "0");
            }
            return;
        };
        for (at, node) in element.nodes.iter().enumerate() {
            if paths.grounded[at] {
                groups.join(node, "0");
            }
            let partner = paths.groups[..at]
                .iter()
                .position(|group| *group == paths.groups[at]);
            if let Some(partner) = partner {
                groups.join(node, &element.nodes[partner]);
            }
            for global in &paths.globals[at] {
                groups.join(node, global);
            }
        }
    }

    fn solve(&mut self, name: &str) -> Option<PinPaths> {
        if let Some(solved) = self.solved.get(name) {
            return solved.clone();
        }
        let circuit = self.circuit;
        let subcircuit = circuit.subcircuits.get(name)?;
        if subcircuit.unread || self.open.iter().any(|open| open == name) {
            return None;
        }
        self.open.push(name.to_string());
        let mut groups = Groups::new();
        groups.add("0");
        for element in &subcircuit.body {
            self.add(element, &mut groups);
        }
        self.open.pop();
        let pins: Vec<String> = subcircuit
            .pins
            .iter()
            .map(|pin| node_name(pin, false))
            .collect();
        // A pin named like ground is not this body's ground: see gnd_pin_subcircuits.
        let pin_named_ground = pins.iter().any(|pin| is_ground(pin));
        let ground = groups.group_of("0");
        let group_ids: Vec<usize> = pins.iter().map(|pin| groups.group_of(pin)).collect();
        let reachable: Vec<(usize, &String)> = circuit
            .globals
            .iter()
            .map(|global| (groups.group_of(global), global))
            .collect();
        let paths = PinPaths {
            grounded: group_ids
                .iter()
                .map(|id| *id == ground && !pin_named_ground)
                .collect(),
            globals: group_ids
                .iter()
                .map(|id| {
                    let shared = reachable.iter().filter(|(group, _)| group == id);
                    shared.map(|(_, global)| (*global).clone()).collect()
                })
                .collect(),
            groups: group_ids,
        };
        self.solved.insert(name.to_string(), Some(paths.clone()));
        Some(paths)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Circuit {
    pub elements: Vec<Element>,
    /// Model name to type, both lowercase.
    pub models: HashMap<String, String>,
    pub subcircuits: HashMap<String, Subcircuit>,
    /// Model names a reachable code-model element instantiates, lowercase.
    pub code_models_used: HashSet<String>,
    /// Each top-level `.param`, by lowercase name, with its value as written.
    pub parameters: HashMap<String, String>,
    /// Each `.param` with its name as written, in order.
    pub parameter_values: Vec<(String, String)>,
    /// Nodes named by `.global`, lowercase.
    pub globals: HashSet<String>,
    /// Elements that were not understood, by name.
    pub unparsed: Vec<String>,
    pub has_conditionals: bool,
    /// A nonzero `rshunt` ties every node to ground. `gshunt` does not.
    pub shunted: bool,
    /// Nodes named by `.ic` or `.nodeset` lines from the testbench.
    pub testbench_initial_nodes: Vec<String>,
    /// The same from the schematic.
    pub netlist_initial_nodes: Vec<String>,
    /// Cards as the schematic or testbench wrote them, not from included files.
    pub netlist_cards: Vec<String>,
    /// KiCad's generated models that carry no parameters at all.
    pub default_models: Vec<String>,
    /// Names defined twice with different text. ngspice uses the first.
    pub conflicting_definitions: Vec<String>,
    definitions: HashMap<String, String>,
    /// Subcircuits with a pin named `gnd` or `0`, which ngspice does not treat as
    /// a pin: the body's `gnd` is global ground.
    pub gnd_pin_subcircuits: Vec<String>,
    /// How many cards name each word, for nodes used outside element fields.
    word_cards: HashMap<String, usize>,
}

impl Circuit {
    pub(crate) fn cards_naming(&self, word: &str) -> usize {
        self.word_cards.get(word).copied().unwrap_or(0)
    }
    pub(crate) fn touches_ground(&self, element: &Element) -> bool {
        element.kind == 'a'
            || self
                .definition(element)
                .is_some_and(|sub| sub.touches_ground)
    }
    pub(crate) fn has_source(&self, element: &Element) -> bool {
        self.definition(element).is_some_and(|sub| sub.has_source)
    }

    fn definition(&self, element: &Element) -> Option<&Subcircuit> {
        if element.kind != 'x' {
            return None;
        }
        self.subcircuits.get(&element.args.first()?.to_lowercase())
    }
}

pub(crate) fn is_ground(node: &str) -> bool {
    node == "0"
}

/// ngspice rewrites `gnd` to `0` only when something follows it on the line.
fn node_name(word: &str, last_on_line: bool) -> String {
    let node = word.to_lowercase();
    if node == "gnd" && !last_on_line {
        "0".to_string()
    } else {
        node
    }
}

struct Card {
    text: String,
    from_testbench: bool,
    included: bool,
}

/// Cards with `+` lines joined and comments removed. `$` starts a comment only
/// outside PSpice mode.
fn cards(deck: &FlatDeck, dollar_is_comment: bool) -> Vec<Card> {
    let mut cards: Vec<Card> = Vec::new();
    for (text, from_testbench, included) in deck.lines() {
        let code = text[..comment_start(text.as_bytes(), dollar_is_comment)].trim_end();
        match (code.strip_prefix('+'), cards.last_mut()) {
            (Some(rest), Some(card)) => {
                card.text.push(' ');
                card.text.push_str(rest);
            }
            _ => cards.push(Card {
                text: code.to_string(),
                from_testbench,
                included,
            }),
        }
    }
    cards
}

/// An expression in quotes, braces or parentheses that a space has left open.
fn is_open_expression(word: &str) -> bool {
    word.matches('\'').count() % 2 == 1
        || word.matches('{').count() > word.matches('}').count()
        || word.matches('(').count() > word.matches(')').count()
}
/// ngspice accepts commas between fields and spaces around `=`, and an
/// expression can hold spaces.
fn tokens(card: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for word in card.split(|c: char| c.is_whitespace() || c == ',') {
        if word.is_empty() {
            continue;
        }
        match words.last_mut() {
            Some(last) if is_open_expression(last) => {
                last.push(' ');
                last.push_str(word);
            }
            Some(last) if last.ends_with('=') || word.starts_with('=') => last.push_str(word),
            _ => words.push(word.to_string()),
        }
    }
    words
}

/// The leading number of a value token, whatever scale letters follow it. ngspice
/// reads `1meg` and `10k` this way, so only the sign and size matter here. An
/// expression or a parameter name has none.
fn leading_number(token: &str) -> Option<f64> {
    let bytes = token.as_bytes();
    let digits = |from: usize| {
        bytes[from..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit() || **byte == b'.')
            .count()
    };
    let sign = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let mantissa = sign + digits(sign);
    if mantissa == sign {
        return None;
    }
    // `1e3` is an exponent, and `1meg` or `5e` are not.
    let mut end = mantissa;
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let after = end + 1 + usize::from(matches!(bytes.get(end + 1), Some(b'+' | b'-')));
        let exponent = bytes[after.min(bytes.len())..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if exponent > 0 {
            end = after + exponent;
        }
    }
    token[..end].parse().ok()
}

/// A value with the bracket and spaces ngspice allows before it taken off.
fn trim_bracket(value: &str) -> String {
    value
        .trim_start_matches(|c: char| c == '(' || c.is_whitespace())
        .to_string()
}

/// The `AC` keyword of a source card, and the text of the value it gives, if any.
/// ngspice reads `AC 1`, `AC=1`, `ac = 1` and `AC(1 0)` alike. The name and two
/// nodes come first, so a node called `ac` is not the keyword.
fn ac_value(words: &[String]) -> Option<Option<String>> {
    words.iter().enumerate().skip(3).find_map(|(at, word)| {
        let lower = word.to_ascii_lowercase();
        if lower == "ac" {
            // `AC (1 0)` has a space before the bracket.
            return Some(words.get(at + 1).map(|value| trim_bracket(value)));
        }
        let rest = lower
            .strip_prefix("ac=")
            .or_else(|| lower.strip_prefix("ac("))?;
        Some((!rest.is_empty()).then(|| trim_bracket(rest)))
    })
}

/// Whether any voltage or current source can drive an AC analysis: it carries an
/// `AC` value that is not a plain zero. A source with only a DC value gives a
/// sweep of zeros and `sim_status=0`, so the engine never says so [verified: E54].
/// A source inside a subcircuit counts, because whether it is called is not followed.
pub(crate) fn has_ac_stimulus(deck: &FlatDeck, dollar_is_comment: bool) -> bool {
    cards(deck, dollar_is_comment).iter().any(|card| {
        let words = tokens(&card.text);
        let is_source = words
            .first()
            .is_some_and(|name| name.starts_with(['v', 'V', 'i', 'I']));
        if !is_source {
            return false;
        }
        // `AC` alone means 1, and an expression is taken to be non-zero.
        match ac_value(&words) {
            None => false,
            Some(None) => true,
            Some(Some(value)) => leading_number(&value) != Some(0.0),
        }
    })
}

pub(crate) fn read(deck: &FlatDeck, dollar_is_comment: bool) -> Circuit {
    let cards = cards(deck, dollar_is_comment);
    let mut circuit = Circuit::default();
    let mut depth = 0usize;
    let mut open_definition = None;
    for card in &cards {
        let word = directive(&card.text);
        if word.starts_with(".subckt") || word.starts_with(".macro") {
            depth += 1;
        } else if word.starts_with(".ends") || word == ".eom" {
            depth = depth.saturating_sub(1);
        }
        if (!card.included && depth == 0) || word == ".global" {
            count_words(&card.text, &mut circuit.word_cards);
        }
        if !card.included && !word.starts_with(".subckt") {
            circuit.netlist_cards.push(card.text.clone());
        }
        read_definitions(card, depth, &mut circuit);
        note_definition(card, &word, &mut open_definition, &mut circuit);
    }

    let mut open: Vec<(String, HashSet<String>)> = Vec::new();
    for card in &cards {
        let words = tokens(&card.text);
        let Some(first) = words.first() else {
            continue;
        };
        let word = directive(&card.text);
        if word.starts_with(".subckt") || word.starts_with(".macro") {
            let name = words
                .get(1)
                .map_or(String::new(), |name| name.to_lowercase());
            open.push((name, formal_pins(&words)));
        } else if word.starts_with(".ends") || word == ".eom" {
            open.pop();
        } else if word.starts_with('.') {
            continue;
        } else if let Some((name, pins)) = open.last() {
            read_body_element(&words, name.clone(), &pins.clone(), &mut circuit);
        } else {
            match element(&words, card.from_testbench, &circuit) {
                Some(element) => {
                    if matches!(element.kind, 'a' | 'n') {
                        let model = element.args.first().map(|name| name.to_lowercase());
                        circuit.code_models_used.extend(model);
                    }
                    circuit.elements.push(element);
                }
                None => circuit.unparsed.push(first.clone()),
            }
        }
    }
    settle_nested_calls(&mut circuit);
    note_reachable_code_models(&mut circuit);
    circuit
}
/// A name defined twice with different text, compared at the end of each one.
/// A file included twice is no conflict.
fn note_definition(
    card: &Card,
    word: &str,
    open: &mut Option<(String, String)>,
    circuit: &mut Circuit,
) {
    let name = || tokens(&card.text).get(1).map(|name| name.to_lowercase());
    let finished = if let Some((_, body)) = open.as_mut() {
        body.push_str(&card.text.to_lowercase());
        body.push('\n');
        if word.starts_with(".ends") || word == ".eom" {
            open.take()
        } else {
            None
        }
    } else if word.starts_with(".subckt") || word.starts_with(".macro") {
        *open = name().map(|name| (name, card.text.to_lowercase()));
        None
    } else if word.contains("model") && word.starts_with('.') {
        name().map(|name| (format!(".model {name}"), card.text.to_lowercase()))
    } else {
        None
    };
    if let Some((name, text)) = finished {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let previous = circuit
            .definitions
            .entry(name.clone())
            .or_insert_with(|| text.clone());
        let name = name.trim_start_matches(".model ").to_string();
        if *previous != text && !circuit.conflicting_definitions.contains(&name) {
            circuit.conflicting_definitions.push(name);
        }
    }
}

fn read_definitions(card: &Card, depth: usize, circuit: &mut Circuit) {
    let words = tokens(&card.text);
    let word = directive(&card.text);
    if word.contains("model") && word.starts_with('.') {
        if let Some(name) = words.get(1) {
            let kind = model_type(&words).unwrap_or_default();
            let bare = words[2..]
                .iter()
                .all(|word| !word.contains('=') && (!word.contains('(') || word.ends_with("()")));
            if name.starts_with("__") && bare {
                circuit.default_models.push(name.clone());
            }
            circuit.models.insert(name.to_lowercase(), kind);
        }
    } else if word.starts_with(".subckt") || word.starts_with(".macro") {
        let name = words
            .get(1)
            .map_or(String::new(), |name| name.to_lowercase());
        circuit.subcircuits.entry(name.clone()).or_default().pins = ordered_pins(&words);
        let pins = formal_pins(&words);
        if pins.contains("gnd") || pins.contains("0") {
            circuit.gnd_pin_subcircuits.push(name);
        }
    } else if word == ".global" {
        circuit
            .globals
            .extend(words.iter().skip(1).map(|word| word.to_lowercase()));
    } else if word == ".if" {
        circuit.has_conditionals = true;
    } else if word.starts_with(".para") && depth == 0 {
        for assignment in words.iter().skip(1) {
            if let Some((name, value)) = assignment.split_once('=') {
                circuit
                    .parameters
                    .insert(name.to_lowercase(), value.to_string());
                circuit
                    .parameter_values
                    .push((name.to_string(), value.to_string()));
            }
        }
    } else if word.starts_with(".opt") {
        circuit.shunted |= words.iter().any(|word| is_active_shunt(word));
    } else if word == ".ic" || word == ".nodeset" {
        let nodes = if card.from_testbench {
            &mut circuit.testbench_initial_nodes
        } else {
            &mut circuit.netlist_initial_nodes
        };
        read_initial_nodes(&card.text, nodes);
    }
}

fn is_parameter_word(word: &str) -> bool {
    word.contains('=') || word.eq_ignore_ascii_case("params:")
}

fn formal_pins(words: &[String]) -> HashSet<String> {
    words
        .iter()
        .skip(2)
        .take_while(|word| !is_parameter_word(word))
        .map(|word| word.to_lowercase())
        .collect()
}

fn ordered_pins(words: &[String]) -> Vec<String> {
    words
        .iter()
        .skip(2)
        .take_while(|word| !is_parameter_word(word))
        .cloned()
        .collect()
}

fn is_active_shunt(word: &str) -> bool {
    let lower = word.to_lowercase();
    match lower.split_once('=') {
        Some(("rshunt", value)) => value.parse::<f64>() != Ok(0.0),
        _ => false,
    }
}

fn read_initial_nodes(card: &str, nodes: &mut Vec<String>) {
    let lower = card.to_lowercase();
    for (at, _) in lower.match_indices("v(") {
        let rest = &lower[at + 2..];
        if let Some(end) = rest.find(')') {
            nodes.push(node_name(rest[..end].trim(), false));
        }
    }
}

/// Every word a card names, once per card. Node names can hold `/`, `-` and `+`,
/// so only what cannot be part of one splits a word.
fn count_words(card: &str, counts: &mut HashMap<String, usize>) {
    let lower = card.to_lowercase();
    let words: HashSet<&str> = lower
        .split(|c: char| c.is_whitespace() || "(),={}'\"*;[]~%".contains(c))
        .filter(|word| !word.is_empty())
        .collect();
    for word in words {
        *counts.entry(word.to_string()).or_default() += 1;
    }
}

/// A body reaches ground through a conducting element on global node `0`, and
/// not through a pin that happens to be named `0` or `gnd`.
fn read_body_element(
    words: &[String],
    subcircuit: String,
    pins: &HashSet<String>,
    circuit: &mut Circuit,
) {
    let parsed = element(words, false, circuit);
    let globals = circuit.globals.clone();
    let Some(entry) = circuit.subcircuits.get_mut(&subcircuit) else {
        return;
    };
    let Some(element) = parsed else {
        // Unread, so assume the best: this only ever silences a warning.
        entry.unread = true;
        entry.touches_ground = true;
        entry.has_source = true;
        return;
    };
    match element.kind {
        'a' | 'n' => {
            entry.touches_ground |= element.kind == 'a';
            entry.has_source = true;
            entry
                .code_models
                .extend(element.args.first().map(|name| name.to_lowercase()));
        }
        'x' => entry
            .calls
            .extend(element.args.first().map(|name| name.to_lowercase())),
        'v' | 'i' => entry.has_source = true,
        _ => {}
    }
    let pin_named_ground = pins.contains("0") || pins.contains("gnd");
    let touches = element.nodes.iter().any(|node| is_ground(node));
    entry.touches_ground |= !pin_named_ground && touches;
    entry.has_source |= element.nodes.iter().any(|node| globals.contains(node));
    entry.body.push(element);
}

/// Code models inside bodies the circuit really calls. A mixed vendor library
/// is full of parts nothing uses.
fn note_reachable_code_models(circuit: &mut Circuit) {
    let mut pending: Vec<String> = circuit
        .elements
        .iter()
        .filter(|element| element.kind == 'x')
        .filter_map(|element| element.args.first())
        .map(|name| name.to_lowercase())
        .collect();
    let mut seen = HashSet::new();
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(body) = circuit.subcircuits.get(&name) {
            pending.extend(body.calls.iter().cloned());
            let models = body.code_models.clone();
            circuit.code_models_used.extend(models);
        }
    }
}

/// What a call inside a body brings in, followed until nothing changes.
fn settle_nested_calls(circuit: &mut Circuit) {
    loop {
        let snapshot = circuit.subcircuits.clone();
        let mut changed = false;
        for subcircuit in circuit.subcircuits.values_mut() {
            for called in subcircuit.calls.clone() {
                let Some(callee) = snapshot.get(&called) else {
                    continue;
                };
                changed |= (callee.touches_ground && !subcircuit.touches_ground)
                    || (callee.has_source && !subcircuit.has_source);
                subcircuit.touches_ground |= callee.touches_ground;
                subcircuit.has_source |= callee.has_source;
            }
        }
        if !changed {
            return;
        }
    }
}

/// The first word after the name that is not `key=value`, as vetting reads it.
fn model_type(words: &[String]) -> Option<String> {
    let is_pair = |word: &&String| {
        word.split('(')
            .next()
            .is_some_and(|head| head.contains('='))
    };
    let word = words.iter().skip(2).find(|word| !is_pair(word))?;
    let lower = word.to_lowercase();
    if let Some(rest) = lower.strip_prefix("ako:") {
        let skip = usize::from(rest.is_empty());
        let after = words
            .iter()
            .skip_while(|word| !word.to_lowercase().starts_with("ako:"));
        let mut after = after;
        let kind = after.nth(1 + skip)?;
        return Some(kind.split('(').next()?.to_lowercase());
    }
    Some(lower.split('(').next()?.to_string())
}

/// `R1 (a 0) 1k` and `V1 (a) (0) 5` are legal: brackets around nodes vanish.
fn without_node_brackets(words: &[String]) -> Vec<String> {
    let mut plain = Vec::with_capacity(words.len());
    for (at, word) in words.iter().enumerate() {
        if at > 0 && word.starts_with('(') && word.ends_with(')') && !is_source_field(words, at) {
            let inner = word.trim_matches(['(', ')']);
            plain.extend(inner.split_whitespace().map(str::to_owned));
        } else {
            plain.push(word.clone());
        }
    }
    plain
}

/// Past the second node a bracket is a value, as in `R1 a 0 (1k)`.
fn is_source_field(words: &[String], at: usize) -> bool {
    at > 1 && !words[1].starts_with('(') || at > 2
}

fn element(words: &[String], from_testbench: bool, circuit: &Circuit) -> Option<Element> {
    let words = &without_node_brackets(words);
    let name = words.first()?;
    let kind = name.chars().next()?.to_ascii_lowercase();
    let is_model = |word: &String| circuit.models.contains_key(&word.to_lowercase());
    let node_count = match kind {
        'r' | 'c' | 'l' | 'v' | 'i' | 'b' | 'f' | 'h' | 'd' | 'w' => 2,
        'j' | 'z' => 3,
        's' | 't' | 'o' => 4,
        'k' => 0,
        'e' | 'g' => controlled_source_nodes(words),
        // These put a variable number of nodes before the model.
        'q' => first_model(words, 4, MAX_BIPOLAR_TOKENS, &is_model)? - 1,
        'm' => first_model(words, 4, MAX_MOS_TOKENS, &is_model)? - 1,
        'x' => return subcircuit_call(words, from_testbench, circuit),
        'a' => return code_model_call(words, from_testbench, circuit),
        _ => return None,
    };
    if words.len() <= node_count {
        return None;
    }
    if kind == 'd' && !words.get(3).is_some_and(is_model) {
        return None;
    }
    if matches!(kind, 'j' | 'z') && !words.get(4).is_some_and(is_model) {
        return None;
    }
    Some(Element {
        name: name.clone(),
        kind,
        nodes: (1..=node_count)
            .map(|at| node_name(&words[at], at + 1 == words.len()))
            .collect(),
        args: words[node_count + 1..].to_vec(),
        from_testbench,
    })
}

fn first_model(
    words: &[String],
    start: usize,
    end: usize,
    is_model: &impl Fn(&String) -> bool,
) -> Option<usize> {
    (start..=end.min(words.len().saturating_sub(1))).find(|at| is_model(&words[*at]))
}

/// `E1 a b c d 2` has four nodes. `VALUE=`, `POLY` and `TABLE` forms have two,
/// and read their inputs from expressions.
fn controlled_source_nodes(words: &[String]) -> usize {
    let plain = |word: Option<&String>| {
        word.is_some_and(|word| {
            let lower = word.to_lowercase();
            !word.contains(['=', '(', '{'])
                && !lower.starts_with("poly")
                && !matches!(lower.as_str(), "value" | "table")
        })
    };
    if words.len() >= 6 && plain(words.get(3)) && plain(words.get(4)) {
        4
    } else {
        2
    }
}

/// `X1 a b sub w=1`, with optional `params:` and PSpice's brackets around nodes.
fn subcircuit_call(words: &[String], from_testbench: bool, circuit: &Circuit) -> Option<Element> {
    let fields: Vec<&String> = words[1..]
        .iter()
        .take_while(|word| !word.eq_ignore_ascii_case("params:"))
        .filter(|word| !word.contains('='))
        .collect();
    let (definition, nodes) = fields.split_last()?;
    if !circuit.subcircuits.contains_key(&definition.to_lowercase()) {
        return None;
    }
    Some(Element {
        name: words[0].clone(),
        kind: 'x',
        nodes: nodes
            .iter()
            .flat_map(|word| word.split(|c: char| c.is_whitespace() || "()".contains(c)))
            .filter(|node| !node.is_empty())
            .map(|node| node_name(node, false))
            .collect(),
        args: vec![(*definition).clone()],
        from_testbench,
    })
}

/// `A1 [a b] ~c %v(d) %vd(e 0) model`: everything before the model is a node or
/// a marker, with brackets and parentheses used freely. A potentiometer is the
/// one model that is a plain resistive part, with no reference of its own.
fn code_model_call(words: &[String], from_testbench: bool, circuit: &Circuit) -> Option<Element> {
    let (model, fields) = words[1..].split_last()?;
    let kind = circuit.models.get(&model.to_lowercase())?;
    let nodes = fields
        .iter()
        .flat_map(|word| word.split(|c: char| c.is_whitespace() || "[]()~".contains(c)))
        .filter(|word| !word.is_empty() && !word.starts_with('%'))
        .filter(|word| !word.eq_ignore_ascii_case("null"))
        .map(|word| node_name(word, false))
        .collect();
    Some(Element {
        name: words[0].clone(),
        kind: match kind.as_str() {
            "potentiometer" => 'p',
            digital if digital != "dac_bridge" && DIGITAL_MODELS.contains(&digital) => 'n',
            _ => 'a',
        },
        nodes,
        args: vec![model.clone()],
        from_testbench,
    })
}

/// Nodes that share an element, tracked as groups.
pub(crate) struct Groups {
    ids: HashMap<String, usize>,
    names: Vec<String>,
    parent: Vec<usize>,
}

impl Groups {
    pub(crate) fn new() -> Self {
        Self {
            ids: HashMap::new(),
            names: Vec::new(),
            parent: Vec::new(),
        }
    }

    pub(crate) fn add(&mut self, node: &str) -> usize {
        if let Some(id) = self.ids.get(node) {
            return *id;
        }
        let id = self.parent.len();
        self.ids.insert(node.to_string(), id);
        self.names.push(node.to_string());
        self.parent.push(id);
        id
    }

    /// With path halving, so long chains stay fast.
    fn root(&mut self, mut id: usize) -> usize {
        while self.parent[id] != id {
            self.parent[id] = self.parent[self.parent[id]];
            id = self.parent[id];
        }
        id
    }

    pub(crate) fn join(&mut self, first: &str, second: &str) {
        let (first, second) = (self.add(first), self.add(second));
        let (first, second) = (self.root(first), self.root(second));
        self.parent[first] = second;
    }

    pub(crate) fn group_of(&mut self, node: &str) -> usize {
        let id = self.add(node);
        self.root(id)
    }

    /// Every group with its id, largest first, then by first node, so reports
    /// are stable.
    pub(crate) fn all(&mut self) -> Vec<(usize, Vec<String>)> {
        let mut groups: HashMap<usize, Vec<String>> = HashMap::new();
        for id in 0..self.parent.len() {
            let root = self.root(id);
            groups.entry(root).or_default().push(self.names[id].clone());
        }
        let mut groups: Vec<(usize, Vec<String>)> = groups.into_iter().collect();
        for (_, group) in &mut groups {
            group.sort();
        }
        groups.sort_by(|(_, a), (_, b)| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::flatten::{flatten, Compatibility, DeckInputs};
    use std::path::Path;

    fn deck(netlist: &str) -> FlatDeck {
        flatten(&DeckInputs {
            netlist,
            testbench: &[],
            project_dir: Path::new("/"),
            allowed_roots: &[],
            compatibility: Compatibility::DEFAULT,
            excluded_elements: &[],
        })
        .unwrap()
    }

    fn driven(card: &str) -> bool {
        let netlist = format!(".title t\n{card}\nR1 a 0 1k\n.end\n");
        has_ac_stimulus(&deck(&netlist), false)
    }

    #[test]
    fn a_value_is_read_as_ngspice_reads_it() {
        for (text, expected) in [
            ("1", Some(1.0)),
            ("0", Some(0.0)),
            ("-0.5", Some(-0.5)),
            ("1meg", Some(1.0)),
            ("10k", Some(10.0)),
            ("2e3", Some(2000.0)),
            ("1e-3x", Some(0.001)),
            ("5e", Some(5.0)),
            ("0m", Some(0.0)),
            ("amp", None),
            ("{amp}", None),
            ("sin(0", None),
            ("-", None),
        ] {
            assert_eq!(leading_number(text), expected, "{text}");
        }
    }

    #[test]
    fn a_source_with_an_ac_value_drives_an_ac_analysis() {
        for card in [
            "V1 a 0 DC 0 AC 1",
            "v1 a 0 ac 1",
            "V1 a 0 AC",
            "V1 a 0 AC 0.5 90",
            "V1 a 0 SIN(0 1 1k) AC 1",
            "I1 0 a DC 1m AC 1m",
            "V1 a 0 AC {amp}",
            "V1 a 0 DC 5 AC 1meg",
            "V1 a 0 dc=0 ac=1",
            "V1 a 0 AC=1",
            "V1 a 0 DC 0 ac=1 0",
            "V1 a 0 DC 0 AC(1 0)",
            "V1 a 0 AC (1 0)",
            "V1 a 0 AC ( 1 0 )",
            "I1 0 a ac=1m",
            "V1 a 0 SIN(0 1 1k) ac = 1",
        ] {
            assert!(driven(card), "{card}");
        }
    }

    #[test]
    fn a_source_with_no_ac_value_or_a_zero_one_drives_nothing() {
        for card in [
            "V1 a 0 DC 5",
            "V1 a 0 5",
            "V1 a 0 PULSE(0 5 0 1n 1n 1m 2m)",
            "V1 a 0 AC 0",
            "V1 a 0 DC 1 ac 0.0 90",
            "I1 0 a AC 0m",
            // The word is a node here, not the keyword.
            "V1 ac 0 DC 5",
            "R2 a b ac",
            "C1 a b 1u ac 1",
            "V1 a 0 ac=0",
            "V1 a 0 AC=0.0 90",
            "V1 a 0 DC 1 AC(0 0)",
            "V1 a 0 AC (0 0)",
            "V1 a 0 AC ( 0 0 )",
            "V1 a 0 ac=( 0 0 )",
        ] {
            assert!(!driven(card), "{card}");
        }
    }

    #[test]
    fn a_source_in_a_subcircuit_counts_because_a_call_is_not_followed() {
        let netlist = ".title t\n.subckt drive o\nV1 o 0 AC 1\n.ends\nR1 a 0 1k\n.end\n";
        assert!(has_ac_stimulus(&deck(netlist), false));
    }

    #[test]
    fn a_continued_source_line_is_read_whole() {
        assert!(driven("V1 a 0 DC 0\n+ AC 1"));
    }
}
