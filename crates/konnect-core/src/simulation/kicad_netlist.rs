//! Read KiCad's own netlist and tie it to the SPICE one.
//!
//! The SPICE export cannot name a pin: an unmodelled symbol is `REF __REF`, with
//! no nodes at all. The KiCad-format export of the same snapshot has every
//! component and pin, so it is the source of the net map.

use super::net_names;
use anyhow::{Context, Result};
use konnect_sexp::{parse_sexp, SexpNode};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Component {
    pub reference: String,
    pub value: String,
    pub lib_id: Option<String>,
    /// Has `Sim.*` fields of its own. Without them KiCad infers a model from
    /// the value, or leaves the part unmodelled.
    pub has_sim_fields: bool,
    pub sim_params: Option<String>,
}

/// What a library symbol carries, which a placed symbol may not have copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LibraryPart {
    pub lib_id: String,
    pub has_sim_fields: bool,
    pub sim_params: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Pin {
    pub reference: String,
    pub pin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Net {
    pub name: String,
    pub pins: Vec<Pin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KiCadNetlist {
    pub components: Vec<Component>,
    pub nets: Vec<Net>,
    pub library_parts: Vec<LibraryPart>,
}

pub(crate) fn parse(source: &str) -> Result<KiCadNetlist> {
    let root = parse_sexp(source).context("invalid KiCad netlist S-expression")?;
    let components = root
        .find("components")
        .context("KiCad netlist has no components section")?
        .find_all("comp")
        .into_iter()
        .map(component)
        .collect::<Result<Vec<_>>>()?;
    let nets = root
        .find("nets")
        .map(|nets| nets.find_all("net"))
        .unwrap_or_default()
        .into_iter()
        .map(net)
        .collect::<Result<Vec<_>>>()?;
    let library_parts = root
        .find("libparts")
        .map(|parts| parts.find_all("libpart"))
        .unwrap_or_default()
        .into_iter()
        .filter_map(library_part)
        .collect();
    Ok(KiCadNetlist {
        components,
        nets,
        library_parts,
    })
}

/// Whether any `Sim.*` field exists, and the value of `Sim.Params`.
fn sim_fields(node: &SexpNode) -> (bool, Option<String>) {
    let fields = node
        .find("fields")
        .map(|fields| fields.find_all("field"))
        .unwrap_or_default();
    let named = |field: &&SexpNode, name: &str| field.find_str("name") == Some(name);
    // `Sim.Pins` alone says nothing about the model.
    let has_sim_fields = fields.iter().any(|field| {
        field
            .find_str("name")
            .is_some_and(|name| name.starts_with("Sim.") && name != "Sim.Pins")
    });
    let sim_params = fields
        .iter()
        .find(|field| named(field, "Sim.Params"))
        .and_then(|field| field.children()?.last()?.as_str().map(str::to_owned));
    (has_sim_fields, sim_params)
}

fn library_part(node: &SexpNode) -> Option<LibraryPart> {
    let (has_sim_fields, sim_params) = sim_fields(node);
    Some(LibraryPart {
        lib_id: format!("{}:{}", node.find_str("lib")?, node.find_str("part")?),
        has_sim_fields,
        sim_params,
    })
}

fn required(node: &SexpNode, tag: &str) -> Result<String> {
    node.find_str(tag)
        .map(str::to_owned)
        .with_context(|| format!("KiCad netlist entry has no {tag}"))
}

fn component(node: &SexpNode) -> Result<Component> {
    let lib_id = node.find("libsource").and_then(|source| {
        let lib = source.find_str("lib")?;
        let part = source.find_str("part")?;
        Some(format!("{lib}:{part}"))
    });
    let (has_sim_fields, sim_params) = sim_fields(node);
    Ok(Component {
        reference: required(node, "ref")?,
        value: node.find_str("value").unwrap_or_default().to_owned(),
        lib_id,
        has_sim_fields,
        sim_params,
    })
}

fn net(node: &SexpNode) -> Result<Net> {
    let pins = node
        .find_all("node")
        .into_iter()
        .map(|pin| {
            Ok(Pin {
                reference: required(pin, "ref")?,
                pin: required(pin, "pin")?,
                pin_name: pin.find_str("pinfunction").map(str::to_owned),
                pin_type: pin.find_str("pintype").map(str::to_owned),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Net {
        name: required(node, "name")?,
        pins,
    })
}

/// The SPICE netlist's element lines, split into tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpiceNetlist {
    elements: Vec<Vec<String>>,
}

impl SpiceNetlist {
    pub(crate) fn parse(source: &str) -> Self {
        let elements = source
            .lines()
            .map(str::trim)
            .filter(|line| {
                line.chars()
                    .next()
                    .is_some_and(|first| first.is_ascii_alphabetic())
            })
            .map(|line| line.split_whitespace().map(str::to_owned).collect())
            .collect();
        Self { elements }
    }

    fn tokens(&self) -> HashSet<&str> {
        self.elements
            .iter()
            .flat_map(|tokens| tokens.iter().skip(1))
            .map(String::as_str)
            .collect()
    }

    /// KiCad prepends the device letter when a reference does not start with
    /// it, so `BT1` can be `VBT1` and `U1` can be `XU1`. An exact match wins.
    fn element_for(&self, reference: &str) -> Option<&[String]> {
        let matches_name = |tokens: &&Vec<String>, name: &dyn Fn(&str) -> bool| {
            tokens.first().is_some_and(|first| name(first))
        };
        self.elements
            .iter()
            .find(|tokens| matches_name(tokens, &|first| first == reference))
            .or_else(|| {
                self.elements.iter().find(|tokens| {
                    matches_name(tokens, &|first| {
                        first.get(1..).is_some_and(|rest| rest == reference)
                    })
                })
            })
            .map(Vec::as_slice)
    }
}

/// A net as the simulator will see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NetEntry {
    pub kicad_name: String,
    pub spice_name: String,
    /// False when only unmodelled parts touch this net, so the deck does not
    /// name it yet. `spice_name` is then the conversion, not an observation.
    pub in_deck: bool,
    pub pin_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ElementEntry {
    pub reference: String,
    /// Null when the symbol has no line in the deck: excluded from simulation.
    pub element_name: Option<String>,
    pub modelled: bool,
    pub excluded: bool,
}

pub(crate) fn net_entries(kicad: &KiCadNetlist, spice: &SpiceNetlist) -> Vec<NetEntry> {
    let tokens = spice.tokens();
    kicad
        .nets
        .iter()
        .map(|net| {
            let candidates = net_names::spice_candidates(&net.name);
            let found = candidates
                .iter()
                .find(|candidate| tokens.contains(candidate.as_str()));
            NetEntry {
                kicad_name: net.name.clone(),
                spice_name: found.unwrap_or(&candidates[0]).clone(),
                in_deck: found.is_some(),
                pin_count: net.pins.len(),
            }
        })
        .collect()
}

pub(crate) fn element_entries(
    kicad: &KiCadNetlist,
    spice: &SpiceNetlist,
    excluded: &[String],
) -> Vec<ElementEntry> {
    let references: HashSet<&str> = kicad
        .components
        .iter()
        .map(|component| component.reference.as_str())
        .collect();
    kicad
        .components
        .iter()
        .map(|component| {
            // `VR7` may be `R7` with a device letter, or a symbol of its own.
            let element = spice.element_for(&component.reference).filter(|tokens| {
                tokens[0] == component.reference || !references.contains(tokens[0].as_str())
            });
            ElementEntry {
                reference: component.reference.clone(),
                element_name: element.and_then(|tokens| tokens.first().cloned()),
                modelled: element.is_some_and(|tokens| !is_unmodelled(tokens)),
                excluded: excluded.contains(&component.reference),
            }
        })
        .collect()
}

/// `Q2 __Q2`: a name, KiCad's generated model name, and no nodes. A real model
/// also uses `__REF`, but with nodes and a `.model` line, so the token count
/// is what tells them apart.
fn is_unmodelled(tokens: &[String]) -> bool {
    matches!(tokens, [name, model] if model.strip_prefix("__") == Some(name.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECTIFIER_NET: &str =
        include_str!("../../tests/fixtures/simulation/rectifier_unmodelled.net");
    const RECTIFIER_CIR: &str =
        include_str!("../../tests/fixtures/simulation/rectifier_unmodelled.cir");
    const COLLISION_NET: &str = include_str!("../../tests/fixtures/simulation/net_collision.net");
    const COLLISION_CIR: &str = include_str!("../../tests/fixtures/simulation/net_collision.cir");
    const HIERARCHY_NET: &str = include_str!("../../tests/fixtures/simulation/subsheets.net");
    const HIERARCHY_CIR: &str = include_str!("../../tests/fixtures/simulation/subsheets.cir");

    #[test]
    fn real_kicad_output_gives_components_pins_and_pin_details() {
        let netlist = parse(RECTIFIER_NET).unwrap();
        assert_eq!(netlist.components.len(), 5);
        let r1 = netlist
            .components
            .iter()
            .find(|c| c.reference == "R1")
            .unwrap();
        assert_eq!(r1.value, "1k");
        assert_eq!(r1.lib_id.as_deref(), Some("rectifier_schlib:R"));

        let diode_pin = netlist
            .nets
            .iter()
            .flat_map(|net| &net.pins)
            .find(|pin| pin.reference == "D1" && pin.pin == "1")
            .unwrap();
        assert!(diode_pin.pin_name.is_some());
        assert_eq!(diode_pin.pin_type.as_deref(), Some("passive"));
    }

    #[test]
    fn unmodelled_symbols_are_found_and_modelled_ones_are_not() {
        let kicad = parse(RECTIFIER_NET).unwrap();
        let spice = SpiceNetlist::parse(RECTIFIER_CIR);
        let elements = element_entries(&kicad, &spice, &[]);
        let modelled = |reference: &str| {
            elements
                .iter()
                .find(|e| e.reference == reference)
                .unwrap()
                .modelled
        };
        assert!(!modelled("D1"));
        assert!(!modelled("V1"));
        assert!(modelled("R1"));
        assert!(modelled("C1"));
    }

    #[test]
    fn every_real_net_with_modelled_pins_is_found_in_the_deck() {
        let kicad = parse(RECTIFIER_NET).unwrap();
        let spice = SpiceNetlist::parse(RECTIFIER_CIR);
        let nets = net_entries(&kicad, &spice);
        let entry = |name: &str| nets.iter().find(|n| n.kicad_name == name).unwrap();
        assert!(entry("/rect_out").in_deck);
        assert_eq!(entry("/rect_out").spice_name, "/rect_out");
        assert_eq!(entry("/rect_out").pin_count, 3);
    }

    #[test]
    fn a_hierarchy_keeps_its_sheet_paths() {
        let kicad = parse(HIERARCHY_NET).unwrap();
        let spice = SpiceNetlist::parse(HIERARCHY_CIR);
        let nets = net_entries(&kicad, &spice);
        assert!(!nets.is_empty());
        assert!(nets.iter().any(|net| net.in_deck));
        assert!(
            kicad
                .nets
                .iter()
                .any(|net| net.name.matches('/').count() > 1),
            "the fixture should contain a sub-sheet net"
        );
    }

    #[test]
    fn colliding_kicad_nets_share_one_spice_name() {
        let kicad = parse(COLLISION_NET).unwrap();
        let spice = SpiceNetlist::parse(COLLISION_CIR);
        let nets = net_entries(&kicad, &spice);
        let colliding: Vec<_> = nets.iter().filter(|n| n.spice_name == "/A_1_").collect();
        assert_eq!(colliding.len(), 2, "{nets:?}");
    }

    #[test]
    fn a_net_touched_only_by_unmodelled_parts_is_marked_as_not_in_the_deck() {
        let kicad = KiCadNetlist {
            components: Vec::new(),
            nets: vec![Net {
                name: "/isolated(1)".to_string(),
                pins: Vec::new(),
            }],
            library_parts: Vec::new(),
        };
        let nets = net_entries(&kicad, &SpiceNetlist::parse("R1 a b 1k\n"));
        assert!(!nets[0].in_deck);
        assert_eq!(nets[0].spice_name, "/isolated_1_");
    }

    #[test]
    fn a_device_letter_prefix_is_matched_but_an_exact_name_wins() {
        let spice = SpiceNetlist::parse("XU1 a b opamp\nVBT1 a 0 DC 12\nU1 a __U1\n");
        assert_eq!(spice.element_for("BT1").unwrap()[0], "VBT1");
        assert_eq!(spice.element_for("U1").unwrap()[0], "U1");
        assert!(spice.element_for("Q9").is_none());
    }

    #[test]
    fn a_symbol_with_no_line_does_not_claim_another_symbols_line() {
        let component = |reference: &str| Component {
            reference: reference.to_string(),
            value: String::new(),
            lib_id: None,
            has_sim_fields: false,
            sim_params: None,
        };
        let kicad = KiCadNetlist {
            components: vec![component("R7"), component("VR7"), component("BT1")],
            nets: Vec::new(),
            library_parts: Vec::new(),
        };
        let spice = SpiceNetlist::parse("VR7 a 0 10\nVBT1 a 0 1\n");
        let elements = element_entries(&kicad, &spice, &[]);
        assert_eq!(elements[0].element_name, None);
        assert_eq!(elements[1].element_name.as_deref(), Some("VR7"));
        assert_eq!(elements[2].element_name.as_deref(), Some("VBT1"));
    }

    #[test]
    fn a_symbol_with_no_line_is_not_modelled_and_has_no_element() {
        let kicad = KiCadNetlist {
            components: vec![Component {
                reference: "H1".to_string(),
                value: "Hole".to_string(),
                lib_id: None,
                has_sim_fields: false,
                sim_params: None,
            }],
            nets: Vec::new(),
            library_parts: Vec::new(),
        };
        let elements = element_entries(&kicad, &SpiceNetlist::parse("R1 a b 1k\n"), &[]);
        assert_eq!(elements[0].element_name, None);
        assert!(!elements[0].modelled);
    }

    #[test]
    fn only_the_exact_nodeless_form_counts_as_unmodelled() {
        let tokens =
            |line: &str| -> Vec<String> { line.split_whitespace().map(str::to_owned).collect() };
        assert!(is_unmodelled(&tokens("Q2 __Q2")));
        assert!(!is_unmodelled(&tokens("Q1 c b e __Q1")));
        assert!(!is_unmodelled(&tokens("Q2 __Q3")));
        assert!(!is_unmodelled(&tokens("R1 a b 1k")));
    }

    #[test]
    fn a_pin_map_alone_is_not_a_model() {
        let source = |field: &str| {
            format!(
                "(export (components (comp (ref \"D1\") (fields (field (name \"{field}\") \"x\")))))"
            )
        };
        let has_fields = |field: &str| parse(&source(field)).unwrap().components[0].has_sim_fields;
        assert!(!has_fields("Sim.Pins"));
        assert!(has_fields("Sim.Device"));
    }

    #[test]
    fn a_netlist_without_components_is_an_error_not_an_empty_design() {
        assert!(parse("(export (version \"E\"))").is_err());
        assert!(parse("not an s-expression").is_err());
    }
}
