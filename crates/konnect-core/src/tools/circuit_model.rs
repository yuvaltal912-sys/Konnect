//! The circuit as the circuit-aware review sees it: parts with their pins, the
//! net each pin is on, and (optionally) where each part sits on the board.
//!
//! One [`Circuit`] is one schematic file. Nets are the per-file net graph's
//! (`sch_connectivity`), so a net that continues on another sheet through a
//! global or hierarchical label is marked `leaves_sheet`: a check that wants
//! to say "nothing is connected here" must not say it about such a net.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use konnect_sexp::board_geometry::{BoardGeometry, FootprintGeom, PadGeom};
use konnect_sexp::parser::SexpNode;
use konnect_sexp::schematic::{
    extract_all_net_labels, extract_wires, pin_endpoint, read_schematic, LabelKind,
};

use crate::tools::component_class;
use crate::tools::placed_pins_by_reference;
use crate::tools::sch_connectivity::net_graph_for;

/// One pin of one part.
#[derive(Debug, Clone)]
pub(crate) struct Pin {
    pub number: String,
    pub name: String,
    pub electrical_type: String,
    /// Key into [`Circuit::nets`].
    pub net: String,
}

/// One part (all units of a reference merged).
#[derive(Debug, Clone)]
pub(crate) struct Part {
    pub reference: String,
    pub lib_id: String,
    pub value: String,
    pub pins: Vec<Pin>,
}

impl Part {
    pub fn pin_names(&self) -> Vec<(String, String)> {
        self.pins
            .iter()
            .map(|p| (p.name.clone(), p.electrical_type.clone()))
            .collect()
    }

    pub fn is_two_terminal(&self) -> bool {
        self.pins.len() == 2
    }
}

/// One net.
#[derive(Debug, Clone)]
pub(crate) struct Net {
    pub key: String,
    /// Every name on it; empty for a net nothing names.
    pub names: BTreeSet<String>,
    /// `(part index, pin index)` of every pin on it.
    pub pins: Vec<(usize, usize)>,
    /// A global or hierarchical label puts it on another sheet too.
    pub leaves_sheet: bool,
    /// A power symbol names it.
    pub power_symbol: bool,
}

impl Net {
    pub fn display(&self) -> String {
        self.names
            .iter()
            .next()
            .cloned()
            .unwrap_or_else(|| self.key.clone())
    }

    pub fn is_ground(&self) -> bool {
        self.names
            .iter()
            .any(|n| component_class::is_ground_name(n))
    }

    /// A rail: a power symbol names it, or a name reads like a supply.
    pub fn is_rail(&self) -> bool {
        !self.is_ground() && (self.power_symbol || self.names.iter().any(|n| looks_like_rail(n)))
    }

    pub fn matches_name(&self, name: &str) -> bool {
        let want = name.trim_start_matches('/');
        self.names.iter().any(|n| n.trim_start_matches('/') == want)
    }
}

/// One schematic file.
#[derive(Debug)]
pub(crate) struct Circuit {
    pub file: PathBuf,
    pub parts: Vec<Part>,
    pub nets: BTreeMap<String, Net>,
    /// Placed symbols whose library definition did not resolve: their pins
    /// are unknown, so nothing is concluded about them.
    pub unresolved_symbols: usize,
}

impl Circuit {
    pub fn load(file: &Path) -> anyhow::Result<Circuit> {
        let (_, tree) = read_schematic(file)?;
        Ok(Self::from_tree(file, &tree))
    }

    pub fn from_tree(file: &Path, tree: &SexpNode) -> Circuit {
        let placed = placed_pins_by_reference(tree);
        let wires = extract_wires(tree);
        let labels = extract_all_net_labels(tree);
        let mut graph = net_graph_for(tree, &wires, &labels);

        let mut leaves: BTreeMap<(i64, i64), bool> = BTreeMap::new();
        let mut symbol_roots: BTreeMap<(i64, i64), bool> = BTreeMap::new();
        for label in &labels {
            let root = graph.root_at(label.x, label.y);
            if matches!(
                label.kind,
                LabelKind::GlobalLabel | LabelKind::HierarchicalLabel
            ) {
                leaves.insert(root, true);
            }
            if label.kind == LabelKind::PowerSymbol {
                symbol_roots.insert(root, true);
            }
        }

        let mut parts: Vec<Part> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut nets: BTreeMap<String, Net> = BTreeMap::new();
        for (inst, pins) in &placed {
            // Power symbols and flags name nets; they are not parts.
            if inst.reference.starts_with('#') || inst.lib_id.starts_with("power:") {
                continue;
            }
            let at = *index.entry(inst.reference.clone()).or_insert_with(|| {
                parts.push(Part {
                    reference: inst.reference.clone(),
                    lib_id: inst.lib_id.clone(),
                    value: inst.value.clone(),
                    pins: Vec::new(),
                });
                parts.len() - 1
            });
            for (pin, transform) in pins {
                if parts[at].pins.iter().any(|p| p.number == pin.number) {
                    continue;
                }
                let (x, y) = pin_endpoint(pin, *transform);
                let root = graph.root_at(x, y);
                let names = graph.aliases_of_root(root);
                let key = graph
                    .name_of_root(root)
                    .unwrap_or_else(|| format!("(unnamed {},{})", root.0, root.1));
                let net = nets.entry(key.clone()).or_insert_with(|| Net {
                    key: key.clone(),
                    names,
                    pins: Vec::new(),
                    leaves_sheet: leaves.contains_key(&root),
                    power_symbol: symbol_roots.contains_key(&root),
                });
                let pin_index = parts[at].pins.len();
                net.pins.push((at, pin_index));
                parts[at].pins.push(Pin {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    electrical_type: pin.electrical_type.clone(),
                    net: key,
                });
            }
        }
        let resolved = placed
            .iter()
            .filter(|(i, _)| !i.reference.starts_with('#') && !i.lib_id.starts_with("power:"))
            .count();
        let total = konnect_sexp::schematic::extract_symbol_instances(tree)
            .iter()
            .filter(|i| !i.reference.starts_with('#') && !i.lib_id.starts_with("power:"))
            .count();
        Circuit {
            file: file.to_path_buf(),
            parts,
            nets,
            unresolved_symbols: total.saturating_sub(resolved),
        }
    }

    pub fn part(&self, reference: &str) -> Option<&Part> {
        self.parts.iter().find(|p| p.reference == reference)
    }

    pub fn net(&self, key: &str) -> Option<&Net> {
        self.nets.get(key)
    }

    /// The parts on a net, with the pin each uses.
    pub fn on_net<'a>(&'a self, key: &str) -> Vec<(&'a Part, &'a Pin)> {
        self.nets
            .get(key)
            .map(|n| {
                n.pins
                    .iter()
                    .map(|(p, i)| (&self.parts[*p], &self.parts[*p].pins[*i]))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The other net of a two-terminal part, given one of its nets. `None`
    /// for a part that is not two-terminal or has both pins on one net.
    pub fn across<'a>(&self, part: &'a Part, net: &str) -> Option<&'a str> {
        if !part.is_two_terminal() {
            return None;
        }
        let (a, b) = (&part.pins[0].net, &part.pins[1].net);
        if a == b {
            return None;
        }
        if a == net {
            Some(b)
        } else if b == net {
            Some(a)
        } else {
            None
        }
    }

    /// The first rail a part's power inputs sit on (not ground).
    pub fn supply_nets(&self, part: &Part) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for pin in &part.pins {
            if component_class::power_pin_rule(&pin.electrical_type, &pin.name).is_none() {
                continue;
            }
            let Some(net) = self.net(&pin.net) else {
                continue;
            };
            if net.is_ground() || component_class::is_ground_name(&pin.name) {
                continue;
            }
            if !out.contains(&pin.net) {
                out.push(pin.net.clone());
            }
        }
        out
    }
}

/// A name that reads like a supply rail.
pub(crate) fn looks_like_rail(name: &str) -> bool {
    let upper = name.trim_start_matches('/').to_uppercase();
    upper.starts_with("VCC")
        || upper.starts_with("VDD")
        || upper.starts_with("VBUS")
        || upper.starts_with("VBAT")
        || upper.starts_with("VIN")
        || upper.starts_with("AVCC")
        || upper.starts_with("AVDD")
        || upper.starts_with("DVDD")
        || upper.starts_with("DVCC")
        || upper.starts_with('+')
        || upper.starts_with("V+")
        || rail_voltage_from_name(&upper).is_some()
}

/// The voltage a rail name states: `+3V3` → 3.3, `5V` → 5, `+3.3V` → 3.3,
/// `VDD_1V8` → 1.8. `None` when the name states none.
pub(crate) fn rail_voltage_from_name(name: &str) -> Option<f64> {
    let chars: Vec<char> = name.to_uppercase().chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        // A number must not continue a longer alphanumeric token (`LM317`).
        let start = i;
        if start > 0 && chars[start - 1].is_ascii_alphabetic() {
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            continue;
        }
        let mut whole = String::new();
        while i < chars.len() && chars[i].is_ascii_digit() {
            whole.push(chars[i]);
            i += 1;
        }
        if i < chars.len() && chars[i] == 'V' {
            // `3V3`: the V stands for the point; `5V`, `12V`: the V is the unit.
            let mut frac = String::new();
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                frac.push(chars[j]);
                j += 1;
            }
            let text = if frac.is_empty() {
                whole
            } else {
                format!("{whole}.{frac}")
            };
            return text.parse().ok().filter(|v: &f64| *v > 0.0 && *v <= 1000.0);
        }
        if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
            let mut frac = String::new();
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                frac.push(chars[j]);
                j += 1;
            }
            if j < chars.len() && chars[j] == 'V' {
                return format!("{whole}.{frac}")
                    .parse()
                    .ok()
                    .filter(|v: &f64| *v > 0.0 && *v <= 1000.0);
            }
            i = j;
        }
    }
    None
}

// ─── The board ───────────────────────────────────────────────────────────────

/// A board for distance questions: pad anchors, the way `score_placement`
/// measures decoupling (`pad_anchor_euclidean_xy`).
pub(crate) struct BoardView {
    pub geometry: BoardGeometry,
    /// References that appear on more than one footprint.
    ambiguous: BTreeSet<String>,
}

impl BoardView {
    pub fn new(geometry: BoardGeometry) -> Self {
        let mut seen = BTreeSet::new();
        let mut ambiguous = BTreeSet::new();
        for fp in &geometry.footprints {
            if let Some(r) = &fp.reference {
                if !seen.insert(r.clone()) {
                    ambiguous.insert(r.clone());
                }
            }
        }
        BoardView {
            geometry,
            ambiguous,
        }
    }

    /// The footprint for a reference, unless the reference is on the board
    /// twice (then no position can be trusted).
    pub fn footprint(&self, reference: &str) -> Option<&FootprintGeom> {
        if self.ambiguous.contains(reference) {
            return None;
        }
        self.geometry.footprint(reference)
    }

    pub fn is_ambiguous(&self, reference: &str) -> bool {
        self.ambiguous.contains(reference)
    }

    pub fn pad(&self, reference: &str, number: &str) -> Option<&PadGeom> {
        self.footprint(reference)?
            .pads
            .iter()
            .find(|p| p.number == number)
    }

    /// Pads of a footprint on a board net.
    pub fn pads_on_net<'a>(&'a self, reference: &str, net: &str) -> Vec<&'a PadGeom> {
        self.footprint(reference)
            .map(|fp| {
                fp.pads
                    .iter()
                    .filter(|p| p.net.as_deref() == Some(net))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Distance between two pad anchors, mm.
pub(crate) fn pad_distance(a: &PadGeom, b: &PadGeom) -> f64 {
    (a.at.0 - b.at.0).hypot(a.at.1 - b.at.1)
}

pub(crate) fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rail_names_state_their_voltage() {
        assert_eq!(rail_voltage_from_name("+3V3"), Some(3.3));
        assert_eq!(rail_voltage_from_name("+5V"), Some(5.0));
        assert_eq!(rail_voltage_from_name("+3.3V"), Some(3.3));
        assert_eq!(rail_voltage_from_name("VDD_1V8"), Some(1.8));
        assert_eq!(rail_voltage_from_name("12V"), Some(12.0));
        assert_eq!(rail_voltage_from_name("/PWR/24V"), Some(24.0));
        assert_eq!(rail_voltage_from_name("GND"), None);
        assert_eq!(rail_voltage_from_name("LM317_OUT"), None);
        assert_eq!(rail_voltage_from_name("VCC"), None);
    }

    #[test]
    fn rail_and_ground_names_are_told_apart() {
        assert!(looks_like_rail("+3V3"));
        assert!(looks_like_rail("VCC"));
        assert!(looks_like_rail("VBUS"));
        assert!(!looks_like_rail("CAN_H"));
        assert!(!looks_like_rail("SDA"));
    }
}
