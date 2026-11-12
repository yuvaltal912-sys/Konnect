//! The circuit as the circuit-aware review sees it: parts with their pins, the
//! net each pin is on, and (optionally) where each part sits on the board.
//!
//! Sheet uses are read with their own instance designators, then joined by
//! proven scalar global and parent/child port connections. Electrical identity
//! is scoped to a sheet use until a connection proves otherwise. `leaves_sheet`
//! now means an unresolved boundary: absence and exhaustive conclusions must
//! not be drawn from an incomplete net. DNP/off-board parts remain excluded.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;
use std::path::{Path, PathBuf};

use konnect_sexp::board_geometry::{BoardGeometry, FootprintGeom, PadGeom};
use konnect_sexp::parser::SexpNode;
use konnect_sexp::schematic::{
    extract_all_net_labels, extract_junctions, extract_lib_pins_for_unit, extract_sheet_pins,
    extract_wires, find_lib_symbol, pin_endpoint, read_schematic, LabelKind,
};

use crate::tools::component_class;
use crate::tools::placed_pins_by_reference;
use crate::tools::sch_connectivity::{net_graph_for, pt_key};

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
    /// Connectivity may continue beyond the part of the hierarchy we proved.
    pub leaves_sheet: bool,
    /// A power symbol names it.
    pub power_symbol: bool,
    /// The sheet the net's circuit sits on, as board nets prefix it
    /// (`/` for the root, `/Power/`); `None` when the hierarchy is unknown.
    pub sheet_path: Option<String>,
    /// Exact board-style aliases, retained for every originating sheet.
    pub scoped_names: BTreeSet<String>,
    pub ambiguous_names: BTreeSet<String>,
    pub origins: BTreeSet<Origin>,
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

    /// Whether `name` (a recorded net name) names this net. A bare name or a
    /// `/X` matches an alias `X` wherever it sits. A `/A/B/X` is the board's
    /// spelling of a net local to sheet `/A/B/`: it matches only in a circuit
    /// on that sheet, never a same-named net on another sheet.
    pub fn matches_name(&self, name: &str) -> bool {
        if self.ambiguous_names.contains(name) {
            return false;
        }
        if self.scoped_names.contains(name) {
            return true;
        }
        let want = name.trim_start_matches('/');
        let qualified = name.starts_with('/') && want.contains('/');
        if !qualified
            && self
                .names
                .iter()
                .any(|n| n == want || escape_net_segment(n) == want)
        {
            return true;
        }
        let Some((dir, local)) = name.strip_prefix('/').and_then(|r| r.rsplit_once('/')) else {
            return false;
        };
        self.scoped_names.contains(name)
            || (self.sheet_path.as_deref() == Some(format!("/{dir}/").as_str())
                && self
                    .names
                    .iter()
                    .any(|n| n.trim_start_matches('/') == local))
    }
}

/// Where a circuit sits in the hierarchy.
#[derive(Debug, Clone, Default)]
pub(crate) struct SheetPlace {
    /// The path KiCad writes in a symbol's `(instances (project (path ...)))`:
    /// `/<root uuid>` for the root sheet, `/<root uuid>/<sheet uuid>` below it.
    pub symbol_path: Option<String>,
    /// The hierarchy walk's chain of sheet UUIDs: `/` or `/<uuid>/<uuid>/`.
    pub instance_path: Option<String>,
    /// The sheets' names, the way board nets are prefixed: `/` or `/Power/`.
    pub name_path: Option<String>,
}

/// A source identity is a use of a file, never just its filename.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct Origin {
    pub source: PathBuf,
    pub sheet_instance_path: Option<String>,
    pub sheet_name_path: Option<String>,
}

#[derive(Debug, Clone)]
struct Port {
    child_path: String,
    child_file: PathBuf,
    name: String,
    net: String,
}

/// A part left out of the circuit because it is not fitted.
#[derive(Debug, Clone)]
pub(crate) struct Unfitted {
    pub reference: String,
    pub why: &'static str,
    pub origin: Origin,
}

/// What a symbol's own node says besides its pins.
struct SymbolMeta {
    dnp: bool,
    on_board: bool,
    /// `(instance path, reference)` from `(instances (project (path ...)))`.
    references: Vec<(String, String, Option<u32>)>,
}

fn symbol_metas(tree: &SexpNode) -> HashMap<String, SymbolMeta> {
    let root_uuid = tree.find_str("uuid").unwrap_or("");
    let mut out = HashMap::new();
    for symbol in tree.find_all("symbol") {
        let Some(uuid) = symbol.find_str("uuid") else {
            continue;
        };
        let mut references = Vec::new();
        for project in symbol
            .find("instances")
            .map(|i| i.find_all("project"))
            .unwrap_or_default()
        {
            for path in project.find_all("path") {
                let (Some(instance), Some(reference)) = (
                    path.get(1).and_then(SexpNode::as_str),
                    path.find_str("reference"),
                ) else {
                    continue;
                };
                // An older file writes the root sheet as "/".
                let full = if instance == "/" && !root_uuid.is_empty() {
                    format!("/{root_uuid}")
                } else {
                    instance.trim_end_matches('/').to_string()
                };
                references.push((
                    full,
                    reference.to_string(),
                    path.find_str("unit").and_then(|u| u.parse().ok()),
                ));
            }
        }
        out.insert(
            uuid.to_string(),
            SymbolMeta {
                // The same reading as pcb_sync.
                dnp: symbol.find_str("dnp") == Some("yes"),
                on_board: symbol.find_str("on_board") != Some("no"),
                references,
            },
        );
    }
    out
}

/// One use of one schematic file.
#[derive(Debug)]
pub(crate) struct Circuit {
    pub file: PathBuf,
    pub sheet: SheetPlace,
    pub parts: Vec<Part>,
    pub nets: BTreeMap<String, Net>,
    /// Placed symbols whose library definition did not resolve: their pins
    /// are unknown, so nothing is concluded about them.
    pub unresolved_symbols: usize,
    /// Parts marked `dnp` or `on_board no`: not in `parts`, so no rule counts
    /// them and none looks for them on the board.
    pub unfitted: Vec<Unfitted>,
    /// Symbols (by their property reference) that have no
    /// `(instances ...)` record for this circuit's sheet path, so the
    /// designator used is the property's and may belong to another use of the
    /// sheet. Empty when the hierarchy is unknown.
    pub reference_unresolved: Vec<String>,
    pub origins: BTreeMap<String, BTreeSet<Origin>>,
    /// What keeps this circuit from being the whole truth, one entry per
    /// cause: a use that is missing, a port or label that joins nothing, a
    /// symbol or name this model cannot read. A cause is never listed twice.
    pub connectivity_gaps: BTreeSet<String>,
    /// Joins worth saying out loud that leave nothing unknown.
    pub connectivity_notes: BTreeSet<String>,
    /// Incomplete inventory can hide a rail or component with no known net.
    pub inventory_complete: bool,
    units: BTreeMap<String, Vec<u32>>,
    globals: BTreeMap<String, BTreeSet<String>>,
    hierarchical: BTreeMap<String, String>,
    ports: Vec<Port>,
    children: Vec<(String, PathBuf)>,
    /// A sheet symbol here has no usable identity (its gap is recorded): a
    /// use below this one that no symbol claims may be that symbol's child.
    anonymous_sheet: bool,
}

impl Circuit {
    pub fn load_at(file: &Path, place: &SheetPlace) -> anyhow::Result<Circuit> {
        let (_, tree) = read_schematic(file)?;
        Ok(Self::from_tree(file, &tree, place))
    }

    pub fn from_tree(file: &Path, tree: &SexpNode, place: &SheetPlace) -> Circuit {
        let metas = symbol_metas(tree);
        let lib_syms = tree
            .find("lib_symbols")
            .map(|n| n.find_all("symbol"))
            .unwrap_or_default();
        let mut placed = placed_pins_by_reference(tree);
        // KiCad's per-use record selects both reference AND unit. Resolve
        // the unit before building the exact pin universe and graph anchors.
        for (inst, pins) in &mut placed {
            let selected: BTreeSet<u32> = place
                .symbol_path
                .as_ref()
                .and_then(|path| {
                    inst.uuid.as_ref().and_then(|id| metas.get(id)).map(|m| {
                        m.references
                            .iter()
                            .filter(|(p, _, _)| p == path)
                            .filter_map(|(_, _, unit)| *unit)
                            .collect()
                    })
                })
                .unwrap_or_default();
            if selected.len() == 1 {
                inst.unit = *selected.first().expect("one unit");
                if let Some(lib) = find_lib_symbol(&lib_syms, inst) {
                    let transform = inst.pin_transform();
                    *pins = extract_lib_pins_for_unit(lib, inst.unit)
                        .into_iter()
                        .map(|p| (p, transform))
                        .collect();
                }
            }
        }
        let wires = extract_wires(tree);
        let labels = extract_all_net_labels(tree);
        // Bare alias-name text is still a scalar net in KiCad; only group
        // syntax such as {ALIAS} requests expansion.
        let supported_name = scalar_name;
        // Unsupported naming syntax cannot establish an electrical edge,
        // even within one sheet. Keep actual wire edges as positive evidence.
        let graph_labels: Vec<_> = labels
            .iter()
            .filter(|label| supported_name(&label.net))
            .cloned()
            .collect();
        let mut graph = net_graph_for(tree, &wires, &graph_labels);

        let origin = Origin {
            source: file.to_path_buf(),
            sheet_instance_path: place.instance_path.clone(),
            sheet_name_path: place.name_path.clone(),
        };
        // A gap belongs to one use of the file: say which.
        let here = match &place.instance_path {
            Some(path) => format!("{} [{path}]", file.display()),
            None => file.display().to_string(),
        };
        let mut anchors = extract_sheet_pins(tree);
        anchors.extend(extract_junctions(tree));
        anchors.extend(wires.iter().flat_map(|w| [(w.x1, w.y1), (w.x2, w.y2)]));
        anchors.extend(labels.iter().map(|l| (l.x, l.y)));
        anchors.extend(
            placed
                .iter()
                .flat_map(|(_, pins)| pins.iter().map(|(p, t)| pin_endpoint(p, *t))),
        );
        let mut canonical = BTreeMap::new();
        for &(x, y) in &anchors {
            let root = graph.root_at(x, y);
            let point = pt_key(x, y);
            canonical
                .entry(root)
                .and_modify(|c: &mut (i64, i64)| *c = (*c).min(point))
                .or_insert(point);
        }
        let key_of = |_graph: &crate::tools::sch_connectivity::NetGraph, root: (i64, i64)| {
            let p = canonical[&root];
            format!("@{},{}", p.0, p.1)
        };
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
        // A sheet containing only ports/labels still conducts between sheets.
        for root in canonical.keys().copied() {
            let key = key_of(&graph, root);
            let names = graph.aliases_of_root(root);
            nets.insert(
                key.clone(),
                Net {
                    key,
                    scoped_names: scoped_names(&names, place),
                    ambiguous_names: BTreeSet::new(),
                    names,
                    pins: Vec::new(),
                    leaves_sheet: leaves.contains_key(&root),
                    power_symbol: symbol_roots.contains_key(&root),
                    sheet_path: place.name_path.clone(),
                    origins: BTreeSet::from([origin.clone()]),
                },
            );
        }
        let mut origins: BTreeMap<String, BTreeSet<Origin>> = BTreeMap::new();
        let mut units: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        let mut connectivity_gaps = BTreeSet::new();
        let mut ambiguous_parts = BTreeSet::new();
        for inst in konnect_sexp::schematic::extract_symbol_instances(tree) {
            if find_lib_symbol(&lib_syms, &inst).is_none() {
                // This use's designator where its instance record gives one.
                let recorded: BTreeSet<&str> = place
                    .symbol_path
                    .as_ref()
                    .zip(inst.uuid.as_ref().and_then(|id| metas.get(id)))
                    .map(|(path, meta)| {
                        meta.references
                            .iter()
                            .filter(|(p, _, _)| p == path)
                            .map(|(_, reference, _)| reference.as_str())
                            .collect()
                    })
                    .unwrap_or_default();
                let reference = match recorded.len() {
                    1 => recorded.into_iter().next().expect("one record"),
                    _ => inst.reference.as_str(),
                };
                connectivity_gaps.insert(format!(
                    "{here}: unresolved symbol connectivity for {reference}"
                ));
            }
        }
        let mut unfitted: Vec<Unfitted> = Vec::new();
        let mut reference_unresolved: Vec<String> = Vec::new();
        for (inst, pins) in &placed {
            // Power symbols and flags name nets; they are not parts.
            let lib = find_lib_symbol(&lib_syms, inst);
            if inst.reference.starts_with('#')
                || inst.lib_id.starts_with("power:")
                || lib.is_some_and(|l| l.find("power").is_some())
            {
                continue;
            }
            let meta = inst.uuid.as_ref().and_then(|u| metas.get(u));
            // The designator for this use of the sheet: its own instance
            // record, else the property.
            let records: BTreeSet<(String, Option<u32>)> = place
                .symbol_path
                .as_ref()
                .map(|path| {
                    meta.map(|m| {
                        m.references
                            .iter()
                            .filter(|(p, _, _)| p == path)
                            .map(|(_, r, u)| (r.clone(), *u))
                            .collect()
                    })
                    .unwrap_or_default()
                })
                .unwrap_or_default();
            if records.len() > 1 {
                connectivity_gaps.insert(format!(
                    "{here}: conflicting instance references for {}",
                    inst.reference
                ));
                ambiguous_parts.extend(records.into_iter().map(|(r, _)| r));
                ambiguous_parts.insert(inst.reference.clone());
                continue;
            }
            let recorded = records.into_iter().next().map(|(r, _)| r);
            let fell_back = place.symbol_path.is_some() && recorded.is_none();
            let reference = recorded.unwrap_or_else(|| inst.reference.clone());
            if let Some(m) = meta {
                let why = if m.dnp {
                    Some("dnp")
                } else if !m.on_board {
                    Some("not_on_board")
                } else {
                    None
                };
                if let Some(why) = why {
                    if !unfitted.iter().any(|u| u.reference == reference) {
                        unfitted.push(Unfitted {
                            reference,
                            why,
                            origin: origin.clone(),
                        });
                    }
                    continue;
                }
            }
            if fell_back && !reference_unresolved.contains(&reference) {
                reference_unresolved.push(reference.clone());
            }
            if lib.is_some_and(unsupported_symbol_connectivity) {
                connectivity_gaps.insert(format!(
                    "{here}: unsupported hidden-power or internal-jumper connectivity on {reference}"
                ));
            }
            origins
                .entry(reference.clone())
                .or_default()
                .insert(origin.clone());
            let part_units = units.entry(reference.clone()).or_default();
            if part_units.contains(&inst.unit) {
                ambiguous_parts.insert(reference.clone());
            }
            part_units.push(inst.unit);
            let at = *index.entry(reference.clone()).or_insert_with(|| {
                parts.push(Part {
                    reference: reference.clone(),
                    lib_id: inst.lib_id.clone(),
                    value: inst.value.clone(),
                    pins: Vec::new(),
                });
                parts.len() - 1
            });
            if parts[at].lib_id != inst.lib_id || parts[at].value != inst.value {
                ambiguous_parts.insert(reference.clone());
            }
            for (pin, transform) in pins {
                let (x, y) = pin_endpoint(pin, *transform);
                let root = graph.root_at(x, y);
                let names = graph.aliases_of_root(root);
                let key = key_of(&graph, root);
                if let Some(previous) = parts[at].pins.iter().find(|p| p.number == pin.number) {
                    if previous.net != key {
                        ambiguous_parts.insert(reference.clone());
                    }
                    continue;
                }
                let net = nets.entry(key.clone()).or_insert_with(|| Net {
                    key: key.clone(),
                    names,
                    pins: Vec::new(),
                    leaves_sheet: leaves.contains_key(&root),
                    power_symbol: symbol_roots.contains_key(&root),
                    sheet_path: place.name_path.clone(),
                    scoped_names: scoped_names(&graph.aliases_of_root(root), place),
                    ambiguous_names: BTreeSet::new(),
                    origins: BTreeSet::from([origin.clone()]),
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
        for reference in &ambiguous_parts {
            connectivity_gaps.insert(format!(
                "{here}: ambiguous component/unit/pin identity for {reference}"
            ));
        }
        parts.retain(|p| !ambiguous_parts.contains(&p.reference));
        let mut circuit = Circuit {
            file: file.to_path_buf(),
            sheet: place.clone(),
            parts,
            nets,
            unresolved_symbols: total.saturating_sub(resolved),
            unfitted,
            reference_unresolved,
            origins,
            units,
            connectivity_gaps,
            connectivity_notes: BTreeSet::new(),
            inventory_complete: true,
            globals: BTreeMap::new(),
            hierarchical: BTreeMap::new(),
            ports: Vec::new(),
            children: Vec::new(),
            anonymous_sheet: false,
        };
        for label in &labels {
            let root = graph.root_at(label.x, label.y);
            let key = key_of(&graph, root);
            if !supported_name(&label.net) {
                circuit
                    .connectivity_gaps
                    .insert(format!("{here}: unsupported bus or variable label"));
                continue;
            }
            // A power symbol with no UUID would match any other symbol with
            // none. That needs a local power symbol (KiCad 9 and later) in a
            // file with no symbol UUIDs, which those versions always write.
            let global_power = label.kind == LabelKind::PowerSymbol
                && placed.iter().any(|(inst, _)| {
                    inst.uuid == label.uuid
                        && find_lib_symbol(&lib_syms, inst)
                            .and_then(|l| l.find("power"))
                            .is_some_and(|power| {
                                power.get(1).and_then(SexpNode::as_str) != Some("local")
                            })
                });
            if label.kind == LabelKind::GlobalLabel || global_power {
                circuit
                    .globals
                    .entry(key.clone())
                    .or_default()
                    .insert(label.net.clone());
            }
            if label.kind == LabelKind::HierarchicalLabel {
                circuit.hierarchical.insert(label.net.clone(), key);
            }
        }
        if !tree.find_all("bus").is_empty() || !tree.find_all("bus_alias").is_empty() {
            circuit
                .connectivity_gaps
                .insert(format!("{here}: bus connectivity is not modeled"));
        }
        for sheet in tree.find_all("sheet") {
            let property = |name| {
                sheet
                    .find_all("property")
                    .into_iter()
                    .find(|p| p.get(1).and_then(SexpNode::as_str) == Some(name))
                    .and_then(|p| p.get(2))
                    .and_then(SexpNode::as_str)
            };
            let (Some(uuid), Some(child_file), Some(parent_path)) = (
                sheet.find_str("uuid"),
                property("Sheetfile"),
                place.instance_path.as_ref(),
            ) else {
                circuit.connectivity_gaps.insert(format!(
                    "{here}: sheet has no usable instance/file identity"
                ));
                circuit.anonymous_sheet = true;
                continue;
            };
            let child_path = format!("{parent_path}{uuid}/");
            let child_file = file.parent().unwrap_or(Path::new(".")).join(child_file);
            circuit
                .children
                .push((child_path.clone(), child_file.clone()));
            for pin in sheet.find_all("pin") {
                let at = pin.find("at");
                let (Some(name), Some(x), Some(y)) = (
                    pin.get(1).and_then(SexpNode::as_str),
                    at.and_then(|a| a.get_f64(1)),
                    at.and_then(|a| a.get_f64(2)),
                ) else {
                    circuit
                        .connectivity_gaps
                        .insert(format!("{here}: malformed sheet port"));
                    continue;
                };
                if !supported_name(name) {
                    circuit
                        .connectivity_gaps
                        .insert(format!("{here}: unsupported hierarchical port name"));
                    continue;
                }
                let root = graph.root_at(x, y);
                circuit.ports.push(Port {
                    child_path: child_path.clone(),
                    child_file: child_file.clone(),
                    name: name.to_string(),
                    net: key_of(&graph, root),
                });
            }
        }
        circuit.reindex();
        circuit
    }

    /// A repeated reference is legitimate when these uses place different
    /// units of the same part. Pin/net compatibility is checked after joins.
    pub fn can_share_reference_with(&self, other: &Circuit, reference: &str) -> bool {
        let (Some(a), Some(b), Some(au), Some(bu)) = (
            self.part(reference),
            other.part(reference),
            self.units.get(reference),
            other.units.get(reference),
        ) else {
            return false;
        };
        a.lib_id == b.lib_id
            && a.value == b.value
            && !au.is_empty()
            && !bu.is_empty()
            && au.iter().all(|u| !bu.contains(u))
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

fn scoped_names(names: &BTreeSet<String>, place: &SheetPlace) -> BTreeSet<String> {
    place
        .name_path
        .as_ref()
        .map(|path| {
            names
                .iter()
                .map(|name| format!("{path}{}", escape_net_segment(name)))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn escape_net_segment(name: &str) -> String {
    name.replace('/', "{slash}").replace(['\n', '\r'], "")
}

fn scalar_name(name: &str) -> bool {
    if name.is_empty() || name.contains("${") || name.contains(['[', ']']) {
        return false;
    }
    let mut depth = 0usize;
    let mut previous = None;
    for c in name.chars() {
        match c {
            '{' if matches!(previous, Some('~' | '^' | '_')) => depth += 1,
            '{' => return false,
            '}' if depth > 0 => depth -= 1,
            '}' => return false,
            _ => {}
        }
        previous = Some(c);
    }
    depth == 0
}

fn unsupported_symbol_connectivity(symbol: &SexpNode) -> bool {
    if symbol.find_str("duplicate_pin_numbers_are_jumpers") == Some("yes")
        || symbol.find("jumper_pin_groups").is_some()
    {
        return true;
    }
    // KiCad writes `(hide yes)`, older files a bare `hide`; `(hide no)` is visible.
    let hidden = |pin: &SexpNode| match pin.find("hide") {
        Some(hide) => hide.get(1).and_then(SexpNode::as_str) != Some("no"),
        None => pin
            .children()
            .is_some_and(|cs| cs.iter().any(|c| c.as_str() == Some("hide"))),
    };
    if symbol
        .find_all("pin")
        .iter()
        .any(|pin| pin.get(1).and_then(SexpNode::as_str) == Some("power_in") && hidden(pin))
    {
        return true;
    }
    symbol
        .find_all("symbol")
        .iter()
        .any(|child| unsupported_symbol_connectivity(child))
}

fn representative(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

fn unite(parent: &mut [usize], a: usize, b: usize) {
    let a = representative(parent, a);
    let b = representative(parent, b);
    parent[a.max(b)] = a.min(b);
}

impl Circuit {
    fn reindex(&mut self) {
        for net in self.nets.values_mut() {
            net.pins.clear();
        }
        self.parts.sort_by(|a, b| a.reference.cmp(&b.reference));
        for (p, part) in self.parts.iter_mut().enumerate() {
            part.pins.sort_by(|a, b| a.number.cmp(&b.number));
            for (i, pin) in part.pins.iter().enumerate() {
                if let Some(net) = self.nets.get_mut(&pin.net) {
                    net.pins.push((p, i));
                }
            }
        }
    }

    /// Join known sheet uses once. The caller supplies completeness from the
    /// existing hierarchy traversal; no new traversal or native process runs.
    ///
    /// `set_aside` holds the instance paths of uses the caller already reports
    /// as not audited: the join treats each as missing, and leaves the
    /// reporting of it to the caller.
    ///
    /// One cause is one gap. A child use that is missing, ambiguous or not the
    /// file its parent names is one gap however many ports lead to it, and
    /// whatever sits below it; those ports and labels become open boundaries.
    /// On a child that is there, each port or label that joins nothing is its
    /// own gap.
    pub fn join(
        mut sheets: Vec<Circuit>,
        traversal_complete: bool,
        set_aside: &BTreeSet<String>,
    ) -> Circuit {
        assert!(!sheets.is_empty());
        sheets.sort_by(|a, b| {
            (&a.sheet.instance_path, &a.file).cmp(&(&b.sheet.instance_path, &b.file))
        });
        let mut use_counts = BTreeMap::new();
        for sheet in &sheets {
            *use_counts
                .entry(sheet.sheet.instance_path.clone())
                .or_insert(0usize) += 1;
        }
        let mut quarantined = BTreeSet::new();
        for sheet in &mut sheets {
            if sheet
                .sheet
                .instance_path
                .as_deref()
                .is_none_or(str::is_empty)
                || use_counts[&sheet.sheet.instance_path] != 1
            {
                quarantined.insert(sheet.sheet.instance_path.clone().unwrap_or_default());
                sheet.connectivity_gaps.insert(format!(
                    "ambiguous sheet instance path {:?}: use quarantined",
                    sheet.sheet.instance_path
                ));
                sheet.parts.clear();
                sheet.nets.clear();
                sheet.globals.clear();
                sheet.hierarchical.clear();
                sheet.ports.clear();
                sheet.children.clear();
            }
        }
        let mut joined = Circuit {
            file: sheets[0].file.clone(),
            sheet: sheets[0].sheet.clone(),
            parts: Vec::new(),
            nets: BTreeMap::new(),
            unresolved_symbols: 0,
            unfitted: Vec::new(),
            reference_unresolved: Vec::new(),
            origins: BTreeMap::new(),
            connectivity_gaps: BTreeSet::new(),
            connectivity_notes: BTreeSet::new(),
            inventory_complete: traversal_complete,
            units: BTreeMap::new(),
            globals: BTreeMap::new(),
            hierarchical: BTreeMap::new(),
            ports: Vec::new(),
            children: Vec::new(),
            anonymous_sheet: false,
        };
        let mut instances: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut ids = BTreeMap::new();
        let mut identities = Vec::new();
        for (i, sheet) in sheets.iter().enumerate() {
            joined
                .connectivity_gaps
                .extend(sheet.connectivity_gaps.iter().cloned());
            let path = sheet.sheet.instance_path.clone().unwrap_or_default();
            instances.entry(path.clone()).or_default().push(i);
            for key in sheet.nets.keys() {
                ids.insert((i, key.clone()), identities.len());
                // JSON tuple encoding prevents delimiter collisions in names.
                identities.push(
                    serde_json::to_string(&(path.as_str(), key.as_str())).expect("string tuple"),
                );
            }
        }
        let mut parent: Vec<usize> = (0..identities.len()).collect();
        let mut global: BTreeMap<&str, usize> = BTreeMap::new();
        let mut boundary_ids = BTreeSet::new();
        let mut connected_labels: BTreeMap<(usize, String), usize> = BTreeMap::new();
        let canonical = |file: &Path| file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
        // The one use at a child path, when it is the file the parent names.
        let available = |path: &str, file: &Path| {
            instances
                .get(path)
                .filter(|uses| {
                    uses.len() == 1 && canonical(&sheets[uses[0]].file) == canonical(file)
                })
                .map(|uses| uses[0])
        };
        let mut claimed = BTreeSet::new();
        let mut unavailable = BTreeSet::new();
        let anonymous: BTreeSet<&str> = sheets
            .iter()
            .filter(|sheet| sheet.anonymous_sheet)
            .filter_map(|sheet| sheet.sheet.instance_path.as_deref())
            .collect();
        for (i, sheet) in sheets.iter().enumerate() {
            for (key, names) in &sheet.globals {
                let id = ids[&(i, key.clone())];
                for name in names {
                    if let Some(previous) = global.insert(name, id) {
                        unite(&mut parent, previous, id);
                    }
                }
            }
            for (path, file) in &sheet.children {
                if available(path, file).is_some() {
                    claimed.insert(path.clone());
                } else if unavailable.insert(path.clone())
                    && !set_aside.contains(path)
                    && !quarantined.contains(path)
                {
                    // A quarantined or set-aside use already has its record.
                    joined
                        .connectivity_gaps
                        .insert(format!("unavailable or ambiguous child instance {path}"));
                }
            }
            for port in &sheet.ports {
                let id = ids[&(i, port.net.clone())];
                let Some(c) = available(&port.child_path, &port.child_file) else {
                    // The missing child is the gap; this is where it bites.
                    boundary_ids.insert(id);
                    continue;
                };
                let Some(key) = sheets[c].hierarchical.get(&port.name) else {
                    boundary_ids.insert(id);
                    joined.connectivity_gaps.insert(format!(
                        "unresolved hierarchical port {}:{}",
                        port.child_path, port.name
                    ));
                    continue;
                };
                // KiCad joins every same-name pin of a sheet symbol through
                // the child's label, so two such pins tie their parent nets.
                if let Some(first) = connected_labels.insert((c, port.name.clone()), id) {
                    if first != id {
                        joined.connectivity_notes.insert(format!(
                            "sheet {} has more than one pin named {}; their nets are one net \
                             through the child's label, as KiCad joins them",
                            port.child_path, port.name
                        ));
                    }
                }
                unite(&mut parent, id, ids[&(c, key.clone())]);
            }
        }
        for (i, sheet) in sheets.iter().enumerate() {
            let path = sheet.sheet.instance_path.as_deref().unwrap_or_default();
            let open: Vec<(&String, &String)> = sheet
                .hierarchical
                .iter()
                .filter(|(label, _)| !connected_labels.contains_key(&(i, (*label).clone())))
                .collect();
            for (_, key) in &open {
                boundary_ids.insert(ids[&(i, (*key).clone())]);
            }
            // The parent's path: this one less its last segment. A sheet
            // symbol with no UUID gives its child an empty segment (`//`).
            let trimmed = path.strip_suffix('/').unwrap_or(path);
            let above = trimmed.rfind('/').map(|at| &trimmed[..=at]);
            match above {
                // Cut off from its parent, and that parent's absence is on
                // record already: the labels are where it bites.
                Some(above)
                    if !claimed.contains(path)
                        && (unavailable.contains(path)
                            || unavailable.contains(above)
                            || set_aside.contains(above)
                            || anonymous.contains(above)) => {}
                Some(_) if !claimed.contains(path) && !open.is_empty() => {
                    joined.connectivity_gaps.insert(format!(
                        "sheet use {path} has hierarchical labels and no parent in this review"
                    ));
                }
                _ => {
                    for (label, _) in open {
                        joined.connectivity_gaps.insert(format!(
                            "unmatched hierarchical label in {:?}: {label}",
                            sheet.sheet.instance_path
                        ));
                    }
                }
            }
        }
        // Missing/unsupported inventory may hide a global participant. An
        // explicit known join remains valid, but cannot prove exhaustiveness.
        joined.inventory_complete &= joined.connectivity_gaps.is_empty()
            && set_aside.is_empty()
            && sheets.iter().all(|s| s.unresolved_symbols == 0);
        let mut remap = BTreeMap::new();
        for (i, sheet) in sheets.iter().enumerate() {
            for (key, net) in &sheet.nets {
                let id = ids[&(i, key.clone())];
                let rep = representative(&mut parent, id);
                let joined_key = identities[rep].clone();
                remap.insert((i, key.clone()), joined_key.clone());
                let mut merged = net.clone();
                merged.key = joined_key.clone();
                merged.pins.clear();
                // Only unresolved edges retain uncertainty, not every label
                // that was successfully joined to another sheet.
                merged.leaves_sheet = boundary_ids.contains(&id)
                    || (!joined.inventory_complete && sheet.globals.contains_key(key))
                    || !sheet.connectivity_gaps.is_empty()
                    || sheet.unresolved_symbols > 0;
                joined
                    .nets
                    .entry(joined_key)
                    .and_modify(|n| {
                        n.names.extend(merged.names.iter().cloned());
                        n.scoped_names.extend(merged.scoped_names.iter().cloned());
                        n.origins.extend(merged.origins.iter().cloned());
                        n.leaves_sheet |= merged.leaves_sheet;
                        n.power_symbol |= merged.power_symbol;
                        if n.sheet_path != merged.sheet_path {
                            n.sheet_path = None;
                        }
                    })
                    .or_insert(merged);
            }
        }
        let mut part_index: BTreeMap<String, usize> = BTreeMap::new();
        let mut ambiguous = BTreeSet::new();
        for (i, sheet) in sheets.into_iter().enumerate() {
            joined.unresolved_symbols += sheet.unresolved_symbols;
            joined.unfitted.extend(sheet.unfitted);
            joined
                .reference_unresolved
                .extend(sheet.reference_unresolved);
            for (reference, origins) in sheet.origins {
                joined.origins.entry(reference).or_default().extend(origins);
            }
            for mut part in sheet.parts {
                for pin in &mut part.pins {
                    pin.net = remap[&(i, pin.net.clone())].clone();
                }
                let units = sheet
                    .units
                    .get(&part.reference)
                    .cloned()
                    .unwrap_or_default();
                if let Some(&at) = part_index.get(&part.reference) {
                    let previous = &mut joined.parts[at];
                    let previous_units = joined
                        .units
                        .get(&part.reference)
                        .expect("indexed part has units");
                    if previous.lib_id != part.lib_id
                        || previous.value != part.value
                        || units.iter().any(|u| previous_units.contains(u))
                    {
                        ambiguous.insert(part.reference.clone());
                    }
                    for pin in part.pins {
                        if let Some(old) = previous.pins.iter().find(|p| p.number == pin.number) {
                            if old.net != pin.net
                                || old.name != pin.name
                                || old.electrical_type != pin.electrical_type
                            {
                                ambiguous.insert(part.reference.clone());
                            }
                        } else {
                            previous.pins.push(pin);
                        }
                    }
                } else {
                    part_index.insert(part.reference.clone(), joined.parts.len());
                    joined.parts.push(part.clone());
                }
                joined
                    .units
                    .entry(part.reference)
                    .or_default()
                    .extend(units);
            }
        }
        for reference in &ambiguous {
            joined.connectivity_gaps.insert(format!(
                "ambiguous component identity across sheet uses: {reference}"
            ));
        }
        if !ambiguous.is_empty() {
            joined.inventory_complete = false;
            // Unknown physical pins may be on any of the observed nets.
            for net in joined.nets.values_mut() {
                net.leaves_sheet = true;
            }
            joined.parts.retain(|p| !ambiguous.contains(&p.reference));
        }
        let mut aliases: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (key, net) in &joined.nets {
            for alias in &net.scoped_names {
                aliases.entry(alias.clone()).or_default().push(key.clone());
            }
        }
        for (alias, keys) in aliases {
            if keys.len() > 1 {
                joined.inventory_complete = false;
                joined
                    .connectivity_gaps
                    .insert(format!("ambiguous qualified net alias {alias}"));
                for key in keys {
                    let net = joined.nets.get_mut(&key).expect("alias net");
                    net.ambiguous_names.insert(alias.clone());
                    net.leaves_sheet = true;
                }
            }
        }
        joined.reindex();
        joined
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
