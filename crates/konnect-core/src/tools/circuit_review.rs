//! Circuit-aware review (creation rank 29, findings B22 and B23 in part).
//!
//! The owner's PCB layout checklist is the standard: this audit automates the
//! schematic- and placement-level items of its sections 2 (placement table),
//! 6 (interfaces), 8 (power stage) and 9 (protection), plus the power tree
//! from the design-intent record's rails (`source_ref`) and regulator
//! feedback dividers. Every finding names the checklist item and the rule
//! that fired. What it cannot judge (datasheet values, waveforms, bench work)
//! is listed as `needs_human`, and so is each subject a rule met but could not
//! judge without a datasheet; a rule that had nothing to examine reports
//! `no_subjects`, one whose every subject needed a human reports
//! `needs_human`, and one that needs a board it was not given reports
//! `not_run`. None of them is ever a pass.
//!
//! Written from the checklist and the parts' pin names; no code is ported
//! from another project.
//!
//! The model is one [`Circuit`] per use of a schematic file (a sheet used
//! twice is two circuits, each read with its own designators), so a net that
//! continues on another sheet is marked and absence-type findings about it
//! are listed under `unproven` instead of being asserted. Distances are pad-anchor
//! Euclidean, the metric `score_placement` uses for decoupling; they are not
//! routed lengths.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use konnect_sexp::board_geometry::BoardGeometry;
use konnect_sexp::parser::parse_sexp;
use konnect_sexp::schematic::read_schematic;
use serde_json::{json, Value};

use crate::mcp::protocol::CallToolResult;
use crate::tools::circuit_model::{
    rail_voltage_from_name, round3, BoardView, Circuit, Part, SheetPlace,
};
use crate::tools::component_class::{self, FEEDBACK_PINS};
use crate::tools::design_intent::{
    design_intent_path, read_design_intent, DesignIntent, IntentRead,
};

// ─── The rules ───────────────────────────────────────────────────────────────

/// What a rule needs in order to run at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Needs {
    Schematic,
    Board,
    Intent,
}

impl Needs {
    fn key(self) -> &'static str {
        match self {
            Needs::Schematic => "schematic",
            Needs::Board => "board",
            Needs::Intent => "design_intent",
        }
    }
}

pub(super) struct Rule {
    pub id: &'static str,
    /// The checklist section and item the rule serves.
    pub checklist: &'static str,
    pub title: &'static str,
    pub needs: Needs,
}

const fn rule(
    id: &'static str,
    checklist: &'static str,
    title: &'static str,
    needs: Needs,
) -> Rule {
    Rule {
        id,
        checklist,
        title,
        needs,
    }
}

const S2_DECOUPLING: &str = "2 Placement: IC decoupling capacitor";
const S2_BULK: &str = "2 Placement: local bulk capacitor";
const S2_TVS: &str = "2 Placement: TVS/ESD protection";
const S2_XCVR: &str = "2 Placement: CAN/RS-485 transceiver";
const S2_SERIES: &str = "2 Placement: series termination resistor";
const S2_BUS_END: &str = "2 Placement: bus-end termination";
const S2_GATE: &str = "2 Placement: MOSFET gate driver / gate resistor";
const S2_SENSE: &str = "2 Placement: current-sense amplifier";
const S2_XTAL: &str = "2 Placement: crystal / ADC reference capacitor";
const S6_CAN: &str = "6 Interfaces: CAN";
const S6_I2C: &str = "6 Interfaces: I2C";
const S6_UART: &str = "6 Interfaces: UART";
const S6_USB: &str = "6 Interfaces: USB 2.0";
const S6_485: &str = "6 Interfaces: RS-485";
const S8_BOOT: &str = "8 Power stage: bootstrap / charge-pump capacitors";
const S8_GATE: &str = "8 Power stage: gate drive with its own return";
const S8_KELVIN: &str = "8 Power stage: Kelvin sense traces from the shunt";
const S9_FIRST: &str = "9 External connections: exposed signals pass the protection first";
const PT: &str = "Power tree (design-intent rails)";

/// Every rule the audit can fire, in report order. A finding's `rule` is one
/// of these ids; a test keeps the table and the code in step.
pub(super) const RULES: &[Rule] = &[
    rule(
        "decoupling_supply_far",
        S2_DECOUPLING,
        "Decoupling capacitor close to its supply pin",
        Needs::Board,
    ),
    rule(
        "decoupling_ground_far",
        S2_DECOUPLING,
        "Decoupling capacitor's ground close to the IC's ground",
        Needs::Board,
    ),
    rule(
        "bulk_cap_far",
        S2_BULK,
        "Bulk capacitor close to a regulator or gate driver supply pin",
        Needs::Board,
    ),
    rule(
        "esd_far_from_connector",
        S2_TVS,
        "TVS/ESD part at the connector",
        Needs::Board,
    ),
    rule(
        "protection_beyond_circuit",
        S9_FIRST,
        "Protection between the connector and the circuit it protects",
        Needs::Board,
    ),
    rule(
        "connector_signal_unprotected",
        S9_FIRST,
        "Connector signal nets carry a TVS/ESD part",
        Needs::Schematic,
    ),
    rule(
        "protection_after_series_element",
        S9_FIRST,
        "Protection on the connector side of series parts",
        Needs::Schematic,
    ),
    rule(
        "transceiver_far_from_connector",
        S2_XCVR,
        "CAN/RS-485 transceiver near its bus connector",
        Needs::Board,
    ),
    rule(
        "series_termination_far_from_driver",
        S2_SERIES,
        "Series termination resistor at the driver",
        Needs::Board,
    ),
    rule(
        "termination_far_from_endpoint",
        S2_BUS_END,
        "Bus termination at the connector or transceiver",
        Needs::Board,
    ),
    rule(
        "gate_driver_far_from_mosfets",
        S2_GATE,
        "Gate driver near the MOSFETs it drives",
        Needs::Board,
    ),
    rule(
        "gate_resistor_far_from_gate",
        S2_GATE,
        "Gate resistor beside the MOSFET gate",
        Needs::Board,
    ),
    rule(
        "sense_amp_far_from_shunt",
        S2_SENSE,
        "Current-sense amplifier near the shunt",
        Needs::Board,
    ),
    rule(
        "sense_not_kelvin",
        S8_KELVIN,
        "Sense connection reaches the shunt without sharing the power path",
        Needs::Board,
    ),
    rule(
        "crystal_load_cap_missing",
        S2_XTAL,
        "Crystal pins have load capacitors",
        Needs::Schematic,
    ),
    rule(
        "crystal_cap_far",
        S2_XTAL,
        "Crystal load capacitor near the crystal",
        Needs::Board,
    ),
    rule(
        "crystal_far_from_ic",
        S2_XTAL,
        "Crystal near the IC oscillator pins",
        Needs::Board,
    ),
    rule(
        "adc_ref_cap_far",
        S2_XTAL,
        "ADC reference capacitor near the reference pin",
        Needs::Board,
    ),
    rule(
        "bootstrap_cap_missing",
        S8_BOOT,
        "Bootstrap capacitor between the driver's boot and switch pins",
        Needs::Schematic,
    ),
    rule(
        "bootstrap_cap_far",
        S8_BOOT,
        "Bootstrap capacitor beside the driver pins",
        Needs::Board,
    ),
    rule(
        "gate_drive_return_net",
        S8_GATE,
        "Gate driver returns to the MOSFET source net",
        Needs::Schematic,
    ),
    rule(
        "can_termination_value",
        S6_CAN,
        "CAN termination is 120 ohm",
        Needs::Schematic,
    ),
    rule(
        "can_bus_branches",
        S6_CAN,
        "CAN bus is linear (no star through three or more connectors)",
        Needs::Schematic,
    ),
    rule(
        "i2c_pullup_rail",
        S6_I2C,
        "I2C pull-ups on a rail the devices tolerate",
        Needs::Schematic,
    ),
    rule(
        "i2c_pullup_value",
        S6_I2C,
        "I2C pull-up total resistance",
        Needs::Schematic,
    ),
    rule(
        "uart_tx_rx_crossed",
        S6_UART,
        "UART TX goes to RX",
        Needs::Schematic,
    ),
    rule(
        "usb_esd_missing",
        S6_USB,
        "USB D+/D- carry ESD protection",
        Needs::Schematic,
    ),
    rule(
        "usb_esd_capacitance",
        S6_USB,
        "USB ESD protection is low-capacitance",
        Needs::Schematic,
    ),
    rule(
        "usb_pair_impedance",
        S6_USB,
        "USB D+/D- recorded as a 90 ohm pair",
        Needs::Intent,
    ),
    rule(
        "rs485_termination_value",
        S6_485,
        "RS-485 termination is 120 ohm",
        Needs::Schematic,
    ),
    rule(
        "rs485_failsafe_bias",
        S6_485,
        "RS-485 fail-safe biasing present",
        Needs::Schematic,
    ),
    rule(
        "rail_not_in_schematic",
        PT,
        "Recorded rail nets exist in the schematic",
        Needs::Intent,
    ),
    rule(
        "rail_name_voltage",
        PT,
        "Rail net names agree with the recorded voltage",
        Needs::Intent,
    ),
    rule(
        "rail_source_missing",
        PT,
        "Recorded rail source is a part of the design",
        Needs::Intent,
    ),
    rule(
        "rail_source_not_on_rail",
        PT,
        "Recorded rail source is connected to the rail",
        Needs::Intent,
    ),
    rule(
        "feedback_vout_mismatch",
        PT,
        "Regulator feedback divider gives the recorded voltage",
        Needs::Intent,
    ),
];

fn rule_of(id: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.id == id)
}

/// Checklist items the audit does not judge, with why. `needs_human` is not
/// a pass: these are reported on every run.
pub(super) const NEEDS_HUMAN: &[(&str, &str, &str)] = &[
    ("1 Requirements", "supply range, transients, currents, temperature, mechanical", "inputs the design-intent record only partly holds; nothing here checks them against the parts"),
    ("1 Requirements", "component ratings, logic levels, connector pinouts, footprint pin numbering", "needs the datasheets; pin numbering is compared by sync and DRC, not here"),
    ("1 Requirements", "safe behaviour at power-up, reset, programming and loss of communication", "behaviour, not topology"),
    ("2 Placement", "connectors, enclosure clearances, mating and probing room", "mechanical; not in the board file"),
    ("2 Placement", "separating switching stages and clocks from sensitive analog", "needs the designer's idea of which blocks are sensitive"),
    ("2 Placement", "crystal and ADC reference placement 'per the device's layout guidance'", "the distances checked are generic limits, not the datasheet's"),
    ("3 Return paths", "reference planes, splits, stitching vias, ground bottlenecks", "board-level audit of its own; not part of this one"),
    ("4 Power distribution", "capacitor voltage rating, DC-bias capacitance, ESR, ripple current", "needs the capacitor datasheets"),
    ("4 Power distribution", "trace, pour and via sizing", "audit_trace_current covers the recorded currents"),
    ("5 Routing", "transmission-line effects, slew rate, controlled impedance from the stackup", "needs rise times and the fab's stackup"),
    ("6 CAN", "120 ohm at each physical end of the bus, ground reference, common mode, isolation", "this audit sees one node; the whole bus is a human check"),
    ("6 I2C", "rise time with the real bus capacitance and cable; level shifting", "needs the capacitance"),
    ("6 USB", "90 ohm differential geometry, skew limits, stubs", "needs the stackup; only the recorded impedance is read"),
    ("6 RS-485", "topology, common mode, reference, isolation", "system-level"),
    ("7 Analog and RF", "analog away from clocks and switch nodes; crystal keepouts; antenna keepouts", "needs the designer's classification of nets and the keepout rules"),
    ("8 Power stage", "high-current loop area in each switching state, switch-node copper, recirculation paths, H-bridge protection", "needs the routed board and the current paths"),
    ("8 Power stage", "gate drive 'own return' as routed", "the schematic nets are compared (gate_drive_return_net); the routed return is a layout check"),
    ("9 Protection", "protection working voltage, clamping, leakage, energy; shields, creepage and clearance", "needs the datasheets and the standards"),
    ("10 Manufacturing", "pin 1 and polarity, courtyards, mask and paste, 3D, exported files", "audit_manufacturing, run_drc and the exports cover what is automatable"),
    ("11 Prototype validation", "everything: rails, droop, ripple, signals at the receiver, longest cable, faults", "bench work"),
];

// ─── The report ──────────────────────────────────────────────────────────────

/// What one rule did.
#[derive(Default, Debug)]
pub(super) struct Stat {
    pub examined: usize,
    pub passed: usize,
    pub findings: usize,
    /// Subjects outside the rule's remit, with why.
    pub skipped: Vec<String>,
    /// Subjects the rule met but cannot judge without a datasheet or other
    /// human input, as `(subject, why)`. Not a pass, and not a gap a tool
    /// could close, so they do not make a review incomplete by themselves.
    pub needs_human: Vec<(String, String)>,
    /// Subjects the rule should have judged but could not, with why. A gap in
    /// the review, not a pass.
    pub unevaluated: Vec<String>,
}

/// The accumulating result of one audit.
#[derive(Default)]
pub(super) struct Report {
    pub findings: Vec<Value>,
    pub stats: BTreeMap<&'static str, Stat>,
    /// Absence findings withheld because the net continues on another sheet.
    pub unproven: Vec<Value>,
    /// Per-interface and per-circuit facts that are not findings.
    pub interfaces: Vec<Value>,
    pub crystals: Vec<Value>,
}

impl Report {
    fn stat(&mut self, id: &'static str) -> &mut Stat {
        debug_assert!(rule_of(id).is_some(), "unknown rule {id}");
        self.stats.entry(id).or_default()
    }

    pub fn examine(&mut self, id: &'static str) {
        self.stat(id).examined += 1;
    }

    pub fn pass(&mut self, id: &'static str) {
        self.stat(id).passed += 1;
    }

    pub fn skip(&mut self, id: &'static str, why: impl Into<String>) {
        self.stat(id).skipped.push(why.into());
    }

    /// A subject that only a human (with a datasheet, say) can judge.
    pub fn needs_human(&mut self, id: &'static str, subject: &str, why: impl Into<String>) {
        self.stat(id)
            .needs_human
            .push((subject.to_string(), why.into()));
    }

    /// A subject that needed board data (or another input) and had none.
    /// `examined` is counted too, so `examined = passed + findings +
    /// unevaluated` holds.
    pub fn unevaluated(&mut self, id: &'static str, why: impl Into<String>) {
        let s = self.stat(id);
        s.examined += 1;
        s.unevaluated.push(why.into());
    }

    pub fn unproven(&mut self, id: &'static str, component: &str, why: String) {
        self.unproven.push(json!({
            "rule": id,
            "checklist": rule_of(id).map(|r| r.checklist),
            "component": component,
            "why": why,
        }));
    }

    pub fn finding(
        &mut self,
        id: &'static str,
        severity: &'static str,
        component: Option<&str>,
        issue: String,
        recommendation: String,
        evidence: Value,
    ) {
        let r = rule_of(id).expect("a finding names a rule in RULES");
        self.stat(id).findings += 1;
        self.findings.push(json!({
            "severity": severity,
            "category": "circuit",
            "component": component,
            "issue": issue,
            "recommendation": recommendation,
            "checklist": r.checklist,
            "rule": id,
            "evidence": evidence,
        }));
    }
}

// ─── Inputs ──────────────────────────────────────────────────────────────────

/// What `audit_files` reports about the record it read.
pub(super) enum IntentState {
    Absent,
    Loaded { revision: String },
    Unusable { reason: String },
}

/// The design-intent record beside the root schematic.
pub(super) fn read_intent(root: &Path) -> (Option<DesignIntent>, IntentState, PathBuf) {
    let dir = root.parent().unwrap_or(Path::new("."));
    let path = design_intent_path(dir);
    match read_design_intent(&path) {
        IntentRead::Absent => (None, IntentState::Absent, path),
        IntentRead::Unusable { reason } => (None, IntentState::Unusable { reason }, path),
        IntentRead::Loaded { intent, text } => {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(text.as_bytes());
            let revision = format!(
                "sha256:{}",
                digest
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            );
            (Some(*intent), IntentState::Loaded { revision }, path)
        }
    }
}

// ─── Orchestration ───────────────────────────────────────────────────────────

/// One use of a schematic file in the hierarchy.
#[derive(Debug, Clone)]
pub(super) struct SheetUse {
    pub file: PathBuf,
    /// The hierarchy walk's chain of sheet UUIDs: `/` for the root.
    pub instance_path: Option<String>,
    /// The sheets' names, the way board nets are prefixed: `/` or `/Power/`.
    pub name_path: Option<String>,
}

/// Audit `files` (each schematic file of one hierarchy once, hierarchy
/// unknown) and, when given, `board`.
#[cfg(test)]
pub(super) fn audit_files(
    root: &Path,
    files: &[PathBuf],
    board: Option<&Path>,
    diagnostics: Vec<Value>,
) -> anyhow::Result<CallToolResult> {
    let uses: Vec<SheetUse> = files
        .iter()
        .map(|file| SheetUse {
            file: file.clone(),
            instance_path: None,
            name_path: None,
        })
        .collect();
    audit_uses(root, &uses, board, diagnostics)
}

/// Audit every use of every sheet in `uses` and, when given, `board`. `Err`
/// only when no schematic could be read at all.
///
/// A file used more than once is audited once per use, with each symbol's
/// designator read from its `(instances ...)` record for that use. A use whose
/// designators cannot be read that way is not audited and is reported under
/// `unaudited_sheet_uses`, which makes the review partial: auditing only one
/// use would pass the others unseen.
pub(super) fn audit_uses(
    root: &Path,
    uses: &[SheetUse],
    board: Option<&Path>,
    mut diagnostics: Vec<Value>,
) -> anyhow::Result<CallToolResult> {
    let (intent, intent_state, intent_path) = read_intent(root);
    if let IntentState::Unusable { reason } = &intent_state {
        return Ok(crate::tools::design_intent::unusable(
            &intent_path,
            reason.clone(),
        ));
    }

    let root_uuid = read_schematic(root).ok().and_then(|(_, tree)| {
        tree.find_str("uuid")
            .filter(|u| !u.is_empty())
            .map(str::to_string)
    });
    let canonical = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let mut times_used: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for u in uses {
        *times_used.entry(canonical(&u.file)).or_default() += 1;
    }

    let mut circuits: Vec<Circuit> = Vec::new();
    let mut unaudited: Vec<Value> = Vec::new();
    for u in uses {
        let place = SheetPlace {
            symbol_path: root_uuid.as_ref().and_then(|r| {
                u.instance_path
                    .as_ref()
                    .map(|p| format!("/{r}{}", p.trim_end_matches('/')))
            }),
            instance_path: u.instance_path.clone(),
            name_path: u.name_path.clone(),
        };
        let circuit = match Circuit::load_at(&u.file, &place) {
            Ok(c) => c,
            Err(error) => {
                diagnostics.push(json!({
                    "code": "schematic_parse_failed",
                    "source": u.file.display().to_string(),
                    "message": error.to_string(),
                }));
                continue;
            }
        };
        let key = canonical(&u.file);
        if times_used.get(&key).copied().unwrap_or(1) > 1 {
            let why = if place.symbol_path.is_none() {
                Some(
                    "the sheet is used more than once and the root schematic carries no uuid to \
                     read each use's designators from"
                        .to_string(),
                )
            } else if !circuit.reference_unresolved.is_empty() {
                Some(format!(
                    "the sheet is used more than once and {} symbol(s) ({}) have no instance \
                     record for this use, so its designators cannot be read",
                    circuit.reference_unresolved.len(),
                    circuit
                        .reference_unresolved
                        .iter()
                        .take(5)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            } else {
                circuits
                    .iter()
                    .filter(|c| canonical(&c.file) == key)
                    .find_map(|c| {
                        circuit
                            .parts
                            .iter()
                            .find(|p| c.part(&p.reference).is_some())
                            .map(|p| {
                                format!(
                                    "the sheet is used more than once and {} has the same \
                                     designator in another use, so the board cannot tell them apart",
                                    p.reference
                                )
                            })
                    })
            };
            if let Some(why) = why {
                unaudited.push(json!({
                    "source": u.file.display().to_string(),
                    "sheet_instance_path": u.instance_path,
                    "sheet_name_path": u.name_path,
                    "why": why,
                }));
                continue;
            }
        }
        circuits.push(circuit);
    }
    if circuits.is_empty() && unaudited.is_empty() {
        anyhow::bail!("no schematic file could be read");
    }

    let mut board_diag = None;
    let board_view = match board {
        None => None,
        Some(path) => match std::fs::read_to_string(path)
            .map_err(anyhow::Error::from)
            .and_then(|text| parse_sexp(&text).map_err(anyhow::Error::from))
        {
            Ok(tree) => Some(BoardView::new(BoardGeometry::from_tree(&tree))),
            Err(error) => {
                board_diag = Some(error.to_string());
                diagnostics.push(json!({
                    "code": "board_parse_failed",
                    "source": path.display().to_string(),
                    "message": error.to_string(),
                }));
                None
            }
        },
    };

    let mut report = Report::default();
    for circuit in &circuits {
        let source = circuit.file.display().to_string();
        let sheet = circuit.sheet.instance_path.clone();
        let before = report.findings.len();
        let unproven_before = report.unproven.len();
        let interfaces_before = report.interfaces.len();
        let crystals_before = report.crystals.len();
        super::circuit_checks::run(circuit, board_view.as_ref(), intent.as_ref(), &mut report);
        let tag = |v: &mut Value| {
            v["source"] = json!(source);
            if let Some(path) = &sheet {
                v["sheet_instance_path"] = json!(path);
            }
        };
        report.findings[before..].iter_mut().for_each(tag);
        report.unproven[unproven_before..].iter_mut().for_each(tag);
        report.interfaces[interfaces_before..]
            .iter_mut()
            .for_each(tag);
        report.crystals[crystals_before..].iter_mut().for_each(tag);
    }
    // Design-level: the recorded USB pair impedance does not depend on a sheet.
    if let Some(intent) = &intent {
        let before = report.findings.len();
        super::circuit_checks::usb_pair_impedance(intent, &mut report);
        for f in &mut report.findings[before..] {
            f["source"] = json!(intent_path.display().to_string());
        }
    }
    let power_tree = power_tree(&circuits, intent.as_ref(), &mut report);

    // ── Account for every rule ─────────────────────────────────────────────
    let have_board = board_view.is_some();
    let have_intent = intent.is_some();
    let mut checks = Vec::new();
    let mut ran = 0usize;
    let mut not_run = 0usize;
    // A sheet use that was not audited is a gap, not a pass.
    let mut gaps = unaudited.len();
    for r in RULES {
        let stat = report.stats.get(r.id);
        let blocked = match r.needs {
            Needs::Schematic => None,
            Needs::Board if !have_board => Some(if board.is_some() {
                "board_unreadable"
            } else {
                "no_board"
            }),
            Needs::Board => None,
            Needs::Intent if !have_intent => Some("no_design_intent"),
            Needs::Intent => None,
        };
        let (status, detail): (&str, Option<String>) = match (blocked, stat) {
            (Some(reason), _) => {
                not_run += 1;
                ("not_run", Some(reason.to_string()))
            }
            (None, Some(s)) if s.examined > 0 => {
                ran += 1;
                gaps += s.unevaluated.len();
                ("ran", None)
            }
            (None, Some(s)) if !s.needs_human.is_empty() => {
                ran += 1;
                (
                    "needs_human",
                    Some(format!(
                        "none of this rule's {} subject(s) could be judged without a human \
                         (see needs_human_reasons){}",
                        s.needs_human.len() + s.skipped.len(),
                        if s.skipped.is_empty() {
                            String::new()
                        } else {
                            format!("; {} more were outside its remit", s.skipped.len())
                        }
                    )),
                )
            }
            (None, Some(s)) if !s.skipped.is_empty() => {
                ran += 1;
                (
                    "no_subjects",
                    Some(format!(
                        "{} subject(s) were outside this rule's remit and none was judged \
                         (see skipped_reasons)",
                        s.skipped.len()
                    )),
                )
            }
            _ => {
                ran += 1;
                // Subjects withheld as unproven were there; only an empty rule is empty.
                let withheld = report.unproven.iter().filter(|u| u["rule"] == r.id).count();
                (
                    "no_subjects",
                    Some(if withheld > 0 {
                        format!(
                            "{withheld} subject(s) were withheld as unproven and none was \
                             judged (see unproven)"
                        )
                    } else {
                        "nothing in the design for this rule to examine".to_string()
                    }),
                )
            }
        };
        let mut entry = json!({
            "rule": r.id,
            "checklist": r.checklist,
            "title": r.title,
            "requires": r.needs.key(),
            "status": status,
        });
        if let Some(d) = detail {
            entry["reason"] = json!(d);
        }
        if let Some(s) = stat {
            entry["examined"] = json!(s.examined);
            entry["passed"] = json!(s.passed);
            entry["findings"] = json!(s.findings);
            entry["unevaluated"] = json!(s.unevaluated.len());
            entry["skipped"] = json!(s.skipped.len());
            entry["needs_human"] = json!(s.needs_human.len());
            if !s.needs_human.is_empty() {
                entry["needs_human_reasons"] = json!(s
                    .needs_human
                    .iter()
                    .take(5)
                    .map(|(subject, why)| format!("{subject}: {why}"))
                    .collect::<Vec<_>>());
            }
            if !s.unevaluated.is_empty() {
                entry["unevaluated_reasons"] =
                    json!(s.unevaluated.iter().take(5).collect::<Vec<_>>());
            }
            if !s.skipped.is_empty() {
                entry["skipped_reasons"] = json!(s.skipped.iter().take(5).collect::<Vec<_>>());
            }
        }
        checks.push(entry);
    }

    // Severity order: errors first, stable within a severity.
    let rank = |f: &Value| match f["severity"].as_str() {
        Some("error") => 0,
        Some("warning") => 1,
        _ => 2,
    };
    report.findings.sort_by_key(rank);

    let errors = report.findings.iter().filter(|f| rank(f) == 0).count();
    let warnings = report.findings.iter().filter(|f| rank(f) == 1).count();
    let mut needs_human: Vec<Value> = NEEDS_HUMAN
        .iter()
        .map(|(section, item, why)| json!({"checklist": section, "item": item, "why": why}))
        .collect();
    let checklist_items = needs_human.len();
    // The subjects the rules met and could not judge, each with its subject.
    let mut needs_human_subjects = 0usize;
    for r in RULES {
        for (subject, why) in report
            .stats
            .get(r.id)
            .map(|s| s.needs_human.as_slice())
            .unwrap_or_default()
        {
            needs_human_subjects += 1;
            needs_human.push(
                json!({"rule": r.id, "checklist": r.checklist, "subject": subject, "why": why}),
            );
        }
    }
    let unfitted: Vec<Value> = circuits
        .iter()
        .flat_map(|c| {
            c.unfitted.iter().map(move |u| {
                json!({"reference": u.reference, "why": u.why, "source": c.file.display().to_string()})
            })
        })
        .collect();
    let unresolved_symbols: usize = circuits.iter().map(|c| c.unresolved_symbols).sum();
    let status = if gaps > 0 || !diagnostics.is_empty() || unresolved_symbols > 0 {
        "partial"
    } else {
        "checked"
    };
    let summary = format!(
        "{} finding(s) ({errors} error, {warnings} warning) from {ran} rule(s) that could run; \
         {not_run} rule(s) not run{}; {gaps} subject(s) could not be evaluated; \
         {needs_human_subjects} subject(s) and {checklist_items} checklist item(s) need a human. \
         Nothing here passes a board: a rule with no subjects or no input is not a pass.",
        report.findings.len(),
        if have_board { "" } else { " (no board given)" },
    );
    let _ = board_diag;

    let body = json!({
        "audit": "circuit_design",
        "status": status,
        "scope": "per_schematic_file",
        "standard": "owner's PCB layout checklist, sections 2, 6, 8 and 9 plus the design-intent power tree",
        "distance_metric": "pad_anchor_euclidean_xy",
        "schematic_files": circuits
            .iter()
            .map(|c| c.file.as_path())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        "sheet_instances": circuits.len(),
        "board": have_board,
        "design_intent": match &intent_state {
            IntentState::Absent => json!({"status": "absent", "path": intent_path.display().to_string()}),
            IntentState::Loaded { revision } => json!({"status": "loaded", "revision": revision}),
            IntentState::Unusable { .. } => unreachable!("returned above"),
        },
        "findings": report.findings,
        "checks": checks,
        "coverage": {
            "rules": RULES.len(),
            "ran": ran,
            "not_run": not_run,
            "unevaluated_subjects": gaps,
            "needs_human_subjects": needs_human_subjects,
            "unaudited_sheet_uses": unaudited.len(),
            "unfitted_parts": unfitted.len(),
            "unresolved_symbols": unresolved_symbols,
        },
        "unaudited_sheet_uses": unaudited,
        "unfitted_parts": {
            "count": unfitted.len(),
            "note": "marked dnp or not on the board in the schematic: no rule counts them as fitted, and none looks for them on the board",
            "parts": unfitted.iter().take(25).collect::<Vec<_>>(),
        },
        "power_tree": power_tree,
        "interfaces": report.interfaces,
        "crystals": report.crystals,
        "unproven": report.unproven,
        "needs_human": needs_human,
        "diagnostics": diagnostics,
        "summary": summary,
    });
    Ok(CallToolResult::text(serde_json::to_string(&body)?))
}

// ─── Power tree and feedback dividers ────────────────────────────────────────

/// Reference voltages of adjustable regulators, from their datasheets. Only
/// parts whose reference is a stated, single number are listed; the slack is
/// the tolerance (percent) applied when comparing the computed output with the
/// recorded one (reference, resistor and adjust-pin error). A part not in the
/// table is reported with its implied reference instead of being judged.
const VREF_TABLE: &[(&str, f64, f64)] = &[
    ("LM317", 1.25, 10.0),
    ("LM1117", 1.25, 8.0),
    ("AMS1117", 1.25, 8.0),
    ("LM1085", 1.25, 8.0),
    ("LM2596", 1.23, 5.0),
    ("LM2595", 1.23, 5.0),
    ("LM2576", 1.23, 5.0),
    ("TPS5430", 1.221, 5.0),
    ("TPS54331", 0.8, 5.0),
    ("TPS54231", 0.8, 5.0),
    ("TPS62130", 0.8, 5.0),
    ("TLV62130", 0.8, 5.0),
    ("MP1584", 0.8, 5.0),
    ("MP2315", 0.8, 5.0),
    ("MP2307", 0.925, 5.0),
];

/// Whether `haystack` contains `key` not followed by another digit, so
/// `LM317` does not match `LM3170`.
fn contains_part_number(haystack: &str, key: &str) -> bool {
    let h = haystack.to_uppercase();
    let mut from = 0;
    while let Some(at) = h[from..].find(key) {
        let end = from + at + key.len();
        let next = h[end..].chars().next();
        if !next.is_some_and(|c| c.is_ascii_digit()) {
            return true;
        }
        from = end;
    }
    false
}

fn vref_for(part: &Part) -> Option<(&'static str, f64, f64)> {
    VREF_TABLE
        .iter()
        .find(|(key, _, _)| {
            contains_part_number(&part.value, key) || contains_part_number(&part.lib_id, key)
        })
        .map(|(k, v, s)| (*k, *v, *s))
}

/// A regulator: a `Regulator_*` library symbol, or a part with an input pin
/// and an output, switch or feedback pin. A gate driver (a `Driver_*` library
/// symbol, or a part with gate-driver outputs) is not one, and neither is a
/// part whose `IN`/`OUT` pins are plain signals: a match on pin names alone
/// needs a `power_in`/`power_out` pin or a switch or feedback pin.
pub(super) fn is_regulator(part: &Part) -> bool {
    let lib = part.lib_id.to_uppercase();
    if lib.starts_with("REGULATOR_") {
        return true;
    }
    if lib.starts_with("DRIVER_") || super::circuit_checks::gate_driver(part).is_some() {
        return false;
    }
    let names = part.pin_names();
    let pins = component_class::PinNames::new(&names);
    let input = pins.find(&["VIN", "IN", "VI", "INPUT", "PVIN"]);
    let output = pins.find(&["VOUT", "OUT", "VO", "OUTPUT"]);
    let switch = pins.find(&["SW", "LX"]);
    let feedback = pins.find(FEEDBACK_PINS);
    let typed = input.is_some_and(|i| part.pins[i].electrical_type == "power_in")
        || output.is_some_and(|o| part.pins[o].electrical_type == "power_out");
    input.is_some()
        && (output.is_some() || switch.is_some() || feedback.is_some())
        && (typed || switch.is_some() || feedback.is_some())
        && part.pins.len() >= 3
        && !part.reference.starts_with('J')
}

struct Located<'a> {
    circuit: &'a Circuit,
    part: &'a Part,
}

fn find_part<'a>(circuits: &'a [Circuit], reference: &str) -> Vec<Located<'a>> {
    circuits
        .iter()
        .filter_map(|c| c.part(reference).map(|part| Located { circuit: c, part }))
        .collect()
}

fn power_tree(circuits: &[Circuit], intent: Option<&DesignIntent>, report: &mut Report) -> Value {
    let Some(intent) = intent else {
        return json!({"status": "not_run", "reason": "no_design_intent", "rails": []});
    };
    if intent.rails.is_empty() {
        return json!({"status": "not_run", "reason": "the record holds no rails", "rails": []});
    }

    // Every net of every sheet, by name, for rail lookups.
    let rail_net_names: Vec<(&str, &str)> = intent
        .rails
        .iter()
        .flat_map(|r| r.nets.iter().map(move |n| (n.as_str(), r.name.as_str())))
        .collect();
    let rail_of_net = |circuit: &Circuit, key: &str| -> Option<&str> {
        let net = circuit.net(key)?;
        rail_net_names
            .iter()
            .find(|(n, _)| net.matches_name(n))
            .map(|(_, rail)| *rail)
    };

    let mut rails_json = Vec::new();
    let mut recorded_sources: Vec<&str> = Vec::new();
    for rail in &intent.rails {
        let mut present = false;
        let mut named_voltages = Vec::new();
        for circuit in circuits {
            for net in circuit.nets.values() {
                if rail.nets.iter().any(|n| net.matches_name(n)) {
                    present = true;
                    for n in &net.names {
                        if let Some(v) = rail_voltage_from_name(n) {
                            named_voltages.push((n.clone(), v));
                        }
                    }
                }
            }
        }
        report.examine("rail_not_in_schematic");
        if present {
            report.pass("rail_not_in_schematic");
        } else {
            report.finding(
                "rail_not_in_schematic",
                "warning",
                None,
                format!(
                    "Rail '{}' lists nets {:?} but none of them exists in the schematic",
                    rail.name, rail.nets
                ),
                "Correct the net names in the design-intent record (set_design_intent, section \
                 'rails') or the schematic. Every rail check on this rail is blind until they agree."
                    .into(),
                json!({"rail": rail.name, "nets": rail.nets}),
            );
        }
        named_voltages.sort_by(|a, b| a.0.cmp(&b.0));
        named_voltages.dedup();
        for (name, v) in &named_voltages {
            report.examine("rail_name_voltage");
            if (v - rail.voltage_v).abs() <= 0.05 * rail.voltage_v.abs().max(0.1) {
                report.pass("rail_name_voltage");
            } else {
                report.finding(
                    "rail_name_voltage",
                    "warning",
                    None,
                    format!(
                        "Net '{name}' is named for {v} V but rail '{}' records {} V",
                        rail.name, rail.voltage_v
                    ),
                    "One of the two is wrong: rename the net or correct the record's voltage_v."
                        .into(),
                    json!({"rail": rail.name, "net": name, "name_voltage_v": v, "recorded_voltage_v": rail.voltage_v}),
                );
            }
        }

        // The source.
        let mut entry = json!({
            "rail": rail.name,
            "voltage_v": rail.voltage_v,
            "nets": rail.nets,
            "present_in_schematic": present,
            "source_ref": rail.source_ref,
        });
        if let Some(src) = &rail.source_ref {
            recorded_sources.push(src.as_str());
            let found = find_part(circuits, src);
            report.examine("rail_source_missing");
            if found.is_empty() {
                report.finding(
                    "rail_source_missing",
                    "warning",
                    Some(src),
                    match circuits
                        .iter()
                        .flat_map(|c| &c.unfitted)
                        .find(|u| u.reference == *src)
                    {
                        Some(u) => format!(
                            "Rail '{}' records source_ref {src}, which is not fitted ({})",
                            rail.name,
                            if u.why == "dnp" {
                                "marked DNP"
                            } else {
                                "excluded from the board"
                            }
                        ),
                        None => format!(
                            "Rail '{}' records source_ref {src}, which is not a part of this schematic hierarchy",
                            rail.name
                        ),
                    },
                    "Correct source_ref in the design-intent record, annotate the part, or fit it.".into(),
                    json!({"rail": rail.name, "source_ref": src}),
                );
                entry["source_kind"] = json!("missing");
            } else {
                report.pass("rail_source_missing");
                let loc = &found[0];
                let kind = if is_regulator(loc.part) {
                    "regulator"
                } else if component_class::is_connector(&loc.part.lib_id, &loc.part.reference) {
                    "connector"
                } else {
                    "other"
                };
                entry["source_kind"] = json!(kind);
                entry["source_value"] = json!(loc.part.value);
                // On the rail?
                let on_rail = found
                    .iter()
                    .any(|l| part_reaches_rail(l.circuit, l.part, &rail.nets));
                if on_rail {
                    report.examine("rail_source_not_on_rail");
                    report.pass("rail_source_not_on_rail");
                } else if found.iter().all(|l| {
                    l.part.pins.iter().all(|p| {
                        l.circuit.net(&p.net).is_some_and(|n| n.leaves_sheet)
                            || l.circuit.net(&p.net).is_none()
                    })
                }) {
                    report.unproven(
                        "rail_source_not_on_rail",
                        src,
                        format!("every pin of {src} is on a net that continues on another sheet"),
                    );
                } else {
                    report.examine("rail_source_not_on_rail");
                    report.finding(
                        "rail_source_not_on_rail",
                        "error",
                        Some(src),
                        format!(
                            "{src} is recorded as the source of rail '{}' but none of its pins is on the rail's nets {:?}",
                            rail.name, rail.nets
                        ),
                        "Connect the source to the rail, or fix source_ref.".into(),
                        json!({"rail": rail.name, "source_ref": src}),
                    );
                }
                // Input side and feedback.
                if kind == "regulator" {
                    let names = loc.part.pin_names();
                    let pins = component_class::PinNames::new(&names);
                    if let Some(i) = pins.find(&["VIN", "IN", "VI", "INPUT", "PVIN"]) {
                        let net = &loc.part.pins[i].net;
                        let input_rail = rail_of_net(loc.circuit, net);
                        entry["input"] = json!({
                            "net": loc.circuit.net(net).map(|n| n.display()),
                            "rail": input_rail,
                        });
                    }
                    if let Some(fb) = feedback(loc, rail, circuits, report) {
                        entry["feedback"] = fb;
                    }
                }
            }
        }
        rails_json.push(entry);
    }

    // Regulators nobody recorded.
    let mut unrecorded = Vec::new();
    for circuit in circuits {
        for part in &circuit.parts {
            if is_regulator(part) && !recorded_sources.contains(&part.reference.as_str()) {
                unrecorded.push(part.reference.clone());
            }
        }
    }
    unrecorded.sort();
    unrecorded.dedup();
    json!({
        "status": "checked",
        "rails": rails_json,
        "regulators_not_recorded_as_a_source": unrecorded,
        "note": "inputs are resolved per sheet; a source whose input net continues on another sheet shows no input rail",
    })
}

/// Whether a part has a pin on one of the rail's nets, or on a net one
/// series part away from it. Which pin may take the hop depends on the part:
/// a regulator's switch or output pin reaches its rail through an inductor or
/// ferrite, its feedback pin through the divider resistor, and no other pin
/// (power-good, enable, soft-start, reset) links it to anything. Any pin of
/// a part that is not a regulator, a connector say, may reach the rail through
/// an inductor or ferrite, but never through a resistor.
fn part_reaches_rail(circuit: &Circuit, part: &Part, rail_nets: &[String]) -> bool {
    let on = |key: &str| {
        circuit
            .net(key)
            .is_some_and(|n| rail_nets.iter().any(|r| n.matches_name(r)))
    };
    let regulator = is_regulator(part);
    let names = part.pin_names();
    let pin_names = component_class::PinNames::new(&names);
    let feedback = pin_names.all(FEEDBACK_PINS);
    let switch_or_output = pin_names.all(&[
        "SW", "LX", "PH", "PHASE", "SWITCH", "VOUT", "OUT", "VO", "OUTPUT",
    ]);
    part.pins.iter().enumerate().any(|(i, pin)| {
        let by_inductor =
            !regulator || switch_or_output.contains(&i) || pin.electrical_type == "power_out";
        let by_resistor = feedback.contains(&i);
        on(&pin.net)
            || circuit.on_net(&pin.net).iter().any(|(q, _)| {
                q.reference != part.reference
                    && q.is_two_terminal()
                    && ((by_inductor
                        && component_class::is_series_inductor(&q.lib_id, &q.reference))
                        || (by_resistor && component_class::is_resistor(&q.lib_id, &q.reference)))
                    && circuit.across(q, &pin.net).is_some_and(on)
            })
    })
}

/// The feedback divider of a regulator: top resistor from the output to the
/// feedback pin, bottom from the feedback pin to ground.
fn feedback(
    loc: &Located<'_>,
    rail: &crate::tools::design_intent::Rail,
    _circuits: &[Circuit],
    report: &mut Report,
) -> Option<Value> {
    let part = loc.part;
    let circuit = loc.circuit;
    let names = part.pin_names();
    let pins = component_class::PinNames::new(&names);
    let fb_pin = pins.find(FEEDBACK_PINS)?;
    let fb_net = part.pins[fb_pin].net.clone();
    let out_net = rail
        .nets
        .iter()
        .find_map(|n| circuit.nets.values().find(|net| net.matches_name(n)))
        .map(|n| n.key.clone());

    let mut top: Vec<(&Part, f64)> = Vec::new();
    let mut bottom: Vec<(&Part, f64)> = Vec::new();
    let mut unparsed: Vec<String> = Vec::new();
    for (p, _) in circuit.on_net(&fb_net) {
        if !component_class::is_resistor(&p.lib_id, &p.reference) || !p.is_two_terminal() {
            continue;
        }
        let Some(other) = circuit.across(p, &fb_net) else {
            continue;
        };
        let Some(ohms) = component_class::parse_si_value(&p.value) else {
            unparsed.push(p.reference.clone());
            continue;
        };
        if circuit.net(other).is_some_and(|n| n.is_ground()) {
            bottom.push((p, ohms));
        } else if Some(other) == out_net.as_deref() {
            top.push((p, ohms));
        }
    }
    let r = |v: &[(&Part, f64)]| -> Vec<Value> {
        v.iter()
            .map(|(p, o)| json!({"reference": p.reference, "ohms": o}))
            .collect()
    };
    if top.len() != 1 || bottom.len() != 1 {
        let why = format!(
            "{}: feedback pin {} has {} resistor(s) to the output and {} to ground{}",
            part.reference,
            part.pins[fb_pin].name,
            top.len(),
            bottom.len(),
            if unparsed.is_empty() {
                String::new()
            } else {
                format!(" ({} with an unreadable value)", unparsed.len())
            }
        );
        report.needs_human("feedback_vout_mismatch", &part.reference, why.clone());
        return Some(json!({"status": "divider_not_recognised", "reason": why}));
    }
    let (rt, r_top) = top[0];
    let (rb, r_bot) = bottom[0];
    let ratio = 1.0 + r_top / r_bot;
    let table = vref_for(part);
    let mut out = json!({
        "regulator": part.reference,
        "top": {"reference": rt.reference, "ohms": r_top},
        "bottom": {"reference": rb.reference, "ohms": r_bot},
        "ratio": round3(ratio),
        "recorded_voltage_v": rail.voltage_v,
        "implied_reference_v": round3(rail.voltage_v / ratio),
        "dividers": {"top": r(&top), "bottom": r(&bottom)},
    });
    let Some((key, vref, slack)) = table else {
        // Cannot be judged without the part's reference; say what it implies.
        report.needs_human(
            "feedback_vout_mismatch",
            &part.reference,
            format!(
                "reference voltage of '{}' is not in the built-in table; the recorded {} V implies {:.3} V",
                part.value, rail.voltage_v, rail.voltage_v / ratio
            ),
        );
        out["status"] = json!("reference_unknown");
        out["reference_v"] = Value::Null;
        out["needs_human"] = json!("compare the implied reference with the regulator's datasheet");
        return Some(out);
    };
    let vout = vref * ratio;
    let error_pct = (vout - rail.voltage_v) / rail.voltage_v * 100.0;
    out["reference_v"] = json!(vref);
    out["reference_source"] = json!(format!(
        "built-in table entry {key}; confirm against the datasheet"
    ));
    out["computed_voltage_v"] = json!(round3(vout));
    out["error_pct"] = json!(round3(error_pct));
    out["slack_pct"] = json!(slack);
    report.examine("feedback_vout_mismatch");
    if error_pct.abs() <= slack {
        report.pass("feedback_vout_mismatch");
        out["status"] = json!("matches_record");
    } else {
        out["status"] = json!("mismatch");
        report.finding(
            "feedback_vout_mismatch",
            "error",
            Some(&part.reference),
            format!(
                "{}'s feedback divider ({} {:.0} ohm over {} {:.0} ohm, reference {} V) gives {:.3} V, \
                 but rail '{}' records {} V ({:+.1} %)",
                part.reference, rt.reference, r_top, rb.reference, r_bot, vref, vout, rail.name,
                rail.voltage_v, error_pct
            ),
            "Change the divider, or correct the recorded voltage if the divider is what you intend."
                .into(),
            out.clone(),
        );
    }
    Some(out)
}

#[cfg(test)]
#[path = "circuit_review_tests.rs"]
pub(super) mod tests;
