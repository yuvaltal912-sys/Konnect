//! Read a flat deck into elements and the nodes they touch.
//!
//! This reads only what preflight needs. A line it cannot place is listed as
//! unparsed, so a check that could not see the whole circuit says so instead of
//! passing.

use super::flatten::{comment_start, directive, FlatDeck};
use std::collections::{HashMap, HashSet};

const MAX_BIPOLAR_TOKENS: usize = 6;
const MAX_MOS_TOKENS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Element {
    pub name: String,
    /// The device letter, lowercase.
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
    /// sources. Code models hold their own references, so they join nothing.
    pub(crate) fn dc_pairs(&self) -> Vec<(usize, usize)> {
        let chain = |count: usize| (1..count).map(|at| (at - 1, at)).collect();
        match self.kind {
            'c' | 'i' | 'g' | 'f' | 'k' | 'a' => Vec::new(),
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
    /// A conducting path from a pin to node `0`.
    pub reaches_ground: bool,
    /// Any element on node `0`, which is a reference for AC and transient.
    pub touches_ground: bool,
    pub has_source: bool,
    calls: Vec<String>,
    code_models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Circuit {
    pub elements: Vec<Element>,
    /// Model name to type, both lowercase.
    pub models: HashMap<String, String>,
    pub subcircuits: HashMap<String, Subcircuit>,
    /// Model names a reachable code-model element instantiates, lowercase.
    pub code_models_used: HashSet<String>,
    pub parameters: HashSet<String>,
    /// Each `.param` value as written, with its name.
    pub parameter_values: Vec<(String, String)>,
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
    pub(crate) fn reaches_ground(&self, element: &Element) -> bool {
        element.kind == 'a'
            || self
                .definition(element)
                .is_some_and(|sub| sub.reaches_ground)
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
                    if element.kind == 'a' {
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
        name().map(|name| (name, card.text.to_lowercase()))
    } else {
        None
    };
    if let Some((name, text)) = finished {
        let previous = circuit
            .definitions
            .entry(name.clone())
            .or_insert_with(|| text.clone());
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
        circuit.subcircuits.entry(name.clone()).or_default();
        let pins = formal_pins(&words);
        if pins.contains("gnd") || pins.contains("0") {
            circuit.gnd_pin_subcircuits.push(name);
        }
    } else if word == ".if" {
        circuit.has_conditionals = true;
    } else if word.starts_with(".para") && depth == 0 {
        for assignment in words.iter().skip(1) {
            if let Some((name, value)) = assignment.split_once('=') {
                circuit.parameters.insert(name.to_lowercase());
                let value = value.to_string();
                circuit.parameter_values.push((name.to_string(), value));
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
    let Some(entry) = circuit.subcircuits.get_mut(&subcircuit) else {
        return;
    };
    let Some(element) = parsed else {
        // Unread, so assume the best: this only ever silences a warning.
        entry.reaches_ground = true;
        entry.touches_ground = true;
        entry.has_source = true;
        return;
    };
    match element.kind {
        'a' => {
            entry.reaches_ground = true;
            entry.touches_ground = true;
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
    let on_ground = |node: &String| is_ground(node) && !pin_named_ground;
    entry.touches_ground |= element.nodes.iter().any(on_ground);
    entry.reaches_ground |= element.dc_pairs().iter().any(|(first, second)| {
        on_ground(&element.nodes[*first]) || on_ground(&element.nodes[*second])
    });
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
                changed |= (callee.reaches_ground && !subcircuit.reaches_ground)
                    || (callee.touches_ground && !subcircuit.touches_ground)
                    || (callee.has_source && !subcircuit.has_source);
                subcircuit.reaches_ground |= callee.reaches_ground;
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

fn element(words: &[String], from_testbench: bool, circuit: &Circuit) -> Option<Element> {
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
        kind: if kind == "potentiometer" { 'p' } else { 'a' },
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
