//! Design-intent record: one per project (creation Q3, owner decision 3 in
//! `docs/CREATION_RESEARCH.md` §5).
//!
//! `<project_dir>/.konnect/design_intent.json` holds what the schematic and the
//! board cannot say on their own: the power rails and their currents, the
//! currents of other nets, the interfaces with their impedance targets, and the
//! fab target. Reviews and audits (PR 11 onward: current audit, circuit-aware
//! review, net classes from the schematic) read this record instead of each
//! taking its own inputs, so an answer that depends on intent can cite it or
//! say that it is missing.
//!
//! The record is closed and typed. Unknown keys, duplicate names, a net claimed
//! twice and non-physical numbers are refused before anything is written. A
//! file that cannot be used is never replaced, the same rule `config.rs`
//! follows. Every write names the revision it was based on, so two writers
//! cannot silently drop each other's change, and the result is read back.
//!
//! Net names are stored as given. This module does not compare them with the
//! schematic or the board; a reader that relies on them reports names it cannot
//! match.

use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::CallToolResult;
use crate::tool;
use crate::tools::config::resolve_project_dir;
use crate::tools::{invalid_arg, require_str, ToolContext, ToolDef};
use konnect_sexp::writer::{write_atomic_if_unchanged, write_new_atomic};
use konnect_sexp::SexpError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The record format this build reads and writes.
pub const SCHEMA_VERSION: u32 = 1;

/// The sections `set_design_intent` replaces, in report order.
const SECTIONS: [&str; 4] = ["rails", "currents", "interfaces", "fab_target"];

// ─── Record ──────────────────────────────────────────────────────────────────

/// One project's design intent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignIntent {
    pub schema_version: u32,
    #[serde(default)]
    pub rails: Vec<Rail>,
    #[serde(default)]
    pub currents: Vec<NetCurrent>,
    #[serde(default)]
    pub interfaces: Vec<Interface>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fab_target: Option<FabTarget>,
}

impl DesignIntent {
    fn empty() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            rails: Vec::new(),
            currents: Vec::new(),
            interfaces: Vec::new(),
            fab_target: None,
        }
    }

    /// Sections with nothing in them, so a reader can say what it lacked.
    fn missing_sections(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.rails.is_empty() {
            missing.push("rails");
        }
        if self.currents.is_empty() {
            missing.push("currents");
        }
        if self.interfaces.is_empty() {
            missing.push("interfaces");
        }
        if self.fab_target.is_none() {
            missing.push("fab_target");
        }
        missing
    }
}

/// A power rail: one or more nets at one nominal voltage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rail {
    pub name: String,
    pub nets: Vec<String>,
    pub voltage_v: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_current_a: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance_pct: Option<f64>,
    /// Reference designator of what supplies the rail (a regulator, a connector).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_temp_rise_c: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// The current one non-rail net must carry (a motor phase, a heater, a LED string).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetCurrent {
    pub net: String,
    pub max_current_a: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_temp_rise_c: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// A signal interface and its nets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interface {
    pub name: String,
    /// Free text such as `usb2`, `i2c`, `spi`, `can`, `ethernet`.
    pub kind: String,
    pub nets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub differential_pairs: Vec<DiffPair>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub single_ended_impedance_ohm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub differential_impedance_ohm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Two nets of one interface routed as a pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffPair {
    pub positive: String,
    pub negative: String,
}

/// What the board will be built to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FabTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub house: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_thickness_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outer_copper_oz: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_copper_oz: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_track_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_clearance_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_via_drill_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_via_diameter_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

// ─── Validation ──────────────────────────────────────────────────────────────

/// Every reason the record cannot be used, each prefixed with where it is.
fn validate(intent: &DesignIntent) -> Vec<String> {
    let mut problems = Vec::new();
    if intent.schema_version != SCHEMA_VERSION {
        problems.push(format!(
            "schema_version: {} is not supported (this Konnect reads {SCHEMA_VERSION})",
            intent.schema_version
        ));
    }

    let mut rail_names = BTreeSet::new();
    let mut rail_nets = BTreeSet::new();
    for (i, rail) in intent.rails.iter().enumerate() {
        let at = format!("rails[{i}]");
        name_ok(&mut problems, &format!("{at}.name"), &rail.name);
        if !rail_names.insert(rail.name.as_str()) {
            problems.push(format!("{at}.name: '{}' is used twice", rail.name));
        }
        if rail.nets.is_empty() {
            problems.push(format!("{at}.nets: a rail needs at least one net"));
        }
        for (j, net) in rail.nets.iter().enumerate() {
            name_ok(&mut problems, &format!("{at}.nets[{j}]"), net);
            if !rail_nets.insert(net.as_str()) {
                problems.push(format!("{at}.nets[{j}]: '{net}' already belongs to a rail"));
            }
        }
        positive(
            &mut problems,
            &format!("{at}.max_current_a"),
            rail.max_current_a,
        );
        positive(
            &mut problems,
            &format!("{at}.max_temp_rise_c"),
            rail.max_temp_rise_c,
        );
        if let Some(pct) = rail.tolerance_pct {
            if !(pct > 0.0 && pct <= 100.0) {
                problems.push(format!("{at}.tolerance_pct: {pct} is outside (0, 100]"));
            }
        }
        if let Some(source) = &rail.source_ref {
            name_ok(&mut problems, &format!("{at}.source_ref"), source);
        }
    }

    let mut current_nets = BTreeSet::new();
    for (i, current) in intent.currents.iter().enumerate() {
        let at = format!("currents[{i}]");
        name_ok(&mut problems, &format!("{at}.net"), &current.net);
        if !current_nets.insert(current.net.as_str()) {
            problems.push(format!("{at}.net: '{}' is listed twice", current.net));
        }
        if rail_nets.contains(current.net.as_str()) {
            problems.push(format!(
                "{at}.net: '{}' belongs to a rail; set the rail's max_current_a instead",
                current.net
            ));
        }
        positive(
            &mut problems,
            &format!("{at}.max_current_a"),
            Some(current.max_current_a),
        );
        positive(
            &mut problems,
            &format!("{at}.max_temp_rise_c"),
            current.max_temp_rise_c,
        );
    }

    let mut interface_names = BTreeSet::new();
    for (i, interface) in intent.interfaces.iter().enumerate() {
        let at = format!("interfaces[{i}]");
        name_ok(&mut problems, &format!("{at}.name"), &interface.name);
        if !interface_names.insert(interface.name.as_str()) {
            problems.push(format!("{at}.name: '{}' is used twice", interface.name));
        }
        name_ok(&mut problems, &format!("{at}.kind"), &interface.kind);
        if interface.nets.is_empty() {
            problems.push(format!("{at}.nets: an interface needs at least one net"));
        }
        let mut nets = BTreeSet::new();
        for (j, net) in interface.nets.iter().enumerate() {
            name_ok(&mut problems, &format!("{at}.nets[{j}]"), net);
            if !nets.insert(net.as_str()) {
                problems.push(format!("{at}.nets[{j}]: '{net}' is listed twice"));
            }
        }
        let mut paired = BTreeSet::new();
        for (j, pair) in interface.differential_pairs.iter().enumerate() {
            let pat = format!("{at}.differential_pairs[{j}]");
            if pair.positive == pair.negative {
                problems.push(format!(
                    "{pat}: positive and negative are both '{}'",
                    pair.positive
                ));
            }
            for (side, net) in [("positive", &pair.positive), ("negative", &pair.negative)] {
                if !nets.contains(net.as_str()) {
                    problems.push(format!(
                        "{pat}.{side}: '{net}' is not one of this interface's nets"
                    ));
                }
                if !paired.insert(net.as_str()) {
                    problems.push(format!("{pat}.{side}: '{net}' is already in a pair"));
                }
            }
        }
        positive(
            &mut problems,
            &format!("{at}.single_ended_impedance_ohm"),
            interface.single_ended_impedance_ohm,
        );
        positive(
            &mut problems,
            &format!("{at}.differential_impedance_ohm"),
            interface.differential_impedance_ohm,
        );
        if interface.differential_impedance_ohm.is_some() && interface.differential_pairs.is_empty()
        {
            problems.push(format!(
                "{at}.differential_impedance_ohm: set, but the interface has no differential_pairs"
            ));
        }
    }

    if let Some(fab) = &intent.fab_target {
        if let Some(house) = &fab.house {
            name_ok(&mut problems, "fab_target.house", house);
        }
        if let Some(layers) = fab.layer_count {
            if !(layers == 1 || (layers % 2 == 0 && (2..=32).contains(&layers))) {
                problems.push(format!(
                    "fab_target.layer_count: {layers} is not 1 or an even count from 2 to 32"
                ));
            }
            if layers <= 2 && fab.inner_copper_oz.is_some() {
                problems.push(format!(
                    "fab_target.inner_copper_oz: set, but a {layers}-layer board has no inner layers"
                ));
            }
        }
        for (field, value) in [
            ("board_thickness_mm", fab.board_thickness_mm),
            ("outer_copper_oz", fab.outer_copper_oz),
            ("inner_copper_oz", fab.inner_copper_oz),
            ("min_track_mm", fab.min_track_mm),
            ("min_clearance_mm", fab.min_clearance_mm),
            ("min_via_drill_mm", fab.min_via_drill_mm),
            ("min_via_diameter_mm", fab.min_via_diameter_mm),
        ] {
            positive(&mut problems, &format!("fab_target.{field}"), value);
        }
        if let (Some(drill), Some(diameter)) = (fab.min_via_drill_mm, fab.min_via_diameter_mm) {
            if diameter <= drill {
                problems.push(format!(
                    "fab_target.min_via_diameter_mm: {diameter} is not larger than min_via_drill_mm {drill}"
                ));
            }
        }
    }
    problems
}

/// A name or net must be non-empty and carry no surrounding whitespace, which
/// would never match a KiCad name and would read as a different entry.
fn name_ok(problems: &mut Vec<String>, at: &str, value: &str) {
    if value.is_empty() {
        problems.push(format!("{at}: is empty"));
    } else if value.trim() != value {
        problems.push(format!(
            "{at}: '{value}' has leading or trailing whitespace"
        ));
    }
}

fn positive(problems: &mut Vec<String>, at: &str, value: Option<f64>) {
    if let Some(value) = value {
        if !(value.is_finite() && value > 0.0) {
            problems.push(format!("{at}: {value} must be greater than zero"));
        }
    }
}

// ─── Storage ─────────────────────────────────────────────────────────────────

pub(crate) fn design_intent_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".konnect").join("design_intent.json")
}

/// A content revision: the SHA-256 of the exact file text.
fn revision_of(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("sha256:{hex}")
}

/// What the record on disk turned out to be.
#[derive(Debug)]
pub(crate) enum IntentRead {
    /// No file: no intent has been recorded.
    Absent,
    /// A valid record, with the exact text a conditional write must still find.
    Loaded {
        intent: Box<DesignIntent>,
        text: String,
    },
    /// The file exists and cannot be used. It is never replaced by a write.
    Unusable { reason: String },
}

pub(crate) fn read_design_intent(path: &Path) -> IntentRead {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return IntentRead::Absent,
        Err(error) => {
            return IntentRead::Unusable {
                reason: format!("unreadable: {error}"),
            }
        }
    };
    let intent: DesignIntent = match serde_json::from_str(&text) {
        Ok(intent) => intent,
        Err(error) => {
            return IntentRead::Unusable {
                reason: format!("not a design-intent record: {error}"),
            }
        }
    };
    let problems = validate(&intent);
    if !problems.is_empty() {
        return IntentRead::Unusable {
            reason: format!("invalid record: {}", problems.join("; ")),
        };
    }
    IntentRead::Loaded {
        intent: Box::new(intent),
        text,
    }
}

pub(crate) fn unusable(path: &Path, reason: String) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::InvalidConfiguration {
            path: path.display().to_string(),
            reason: reason.clone(),
        },
        format!(
            "The design-intent record at '{}' exists but cannot be used ({reason}). Konnect wrote \
             nothing and will not replace it. Repair or move the file, then retry.",
            path.display()
        ),
    )
}

fn outcome_uncertain(path: &Path, reason: String) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::MutationOutcomeUncertain {
            operation: "set_design_intent".to_string(),
            path: path.display().to_string(),
            reason: reason.clone(),
        },
        format!(
            "set_design_intent could not prove what is now on disk at '{}': {reason}. Call \
             get_design_intent before retrying.",
            path.display()
        ),
    )
}

fn stale(path: &Path, reason: String) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::StaleTarget {
            target: path.display().to_string(),
            reason: reason.clone(),
        },
        format!(
            "{reason}. Nothing was written. Call get_design_intent, reapply your change to the \
             record it returns, and pass its revision."
        ),
    )
}

/// The directory must already be a KiCad project, so a mistyped path cannot
/// leave a stray `.konnect` folder behind.
fn project_dir_refusal(project_dir: &Path) -> Option<CallToolResult> {
    let entries = match std::fs::read_dir(project_dir) {
        Ok(entries) => entries,
        Err(error) => {
            return Some(invalid_arg(
                "project_dir",
                &format!("'{}' cannot be read: {error}", project_dir.display()),
            ))
        }
    };
    let has_project = entries
        .flatten()
        .any(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("kicad_pro"));
    (!has_project).then(|| {
        invalid_arg(
            "project_dir",
            &format!(
                "'{}' holds no .kicad_pro file, so it is not a KiCad project directory",
                project_dir.display()
            ),
        )
    })
}

// ─── Tool definitions ────────────────────────────────────────────────────────

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "get_design_intent",
            "Read the project's design-intent record (<project_dir>/.konnect/design_intent.json): \
             power rails, currents of other nets, interfaces with impedance targets, and the fab \
             target. Reviews and audits that depend on intent read it from here. 'source' is \
             'file' or 'absent' (nothing recorded yet). 'missing_sections' names the empty \
             sections, so a result that needed one can say so. 'revision' is what \
             set_design_intent needs. Net names are not checked against the schematic or board. \
             A file that cannot be used is refused with invalid_configuration.",
            json!({
                "type": "object",
                "properties": {
                    "project_dir": {
                        "type": "string",
                        "description": "KiCad project directory. If omitted, uses the configured project_dir."
                    }
                },
                "additionalProperties": false
            }),
            |args, ctx| async move { handle_get_design_intent(args, ctx).await }
        ),
        tool!(
            "set_design_intent",
            "Replace one section of the project's design-intent record, keeping the others. \
             Sections: 'rails' = [{name, nets[], voltage_v, max_current_a?, tolerance_pct?, \
             source_ref?, max_temp_rise_c?, notes?}]; 'currents' (non-rail nets) = [{net, \
             max_current_a, max_temp_rise_c?, notes?}]; 'interfaces' = [{name, kind, nets[], \
             differential_pairs?: [{positive, negative}], single_ended_impedance_ohm?, \
             differential_impedance_ohm?, notes?}]; 'fab_target' = {house?, layer_count?, \
             board_thickness_mm?, outer_copper_oz?, inner_copper_oz?, min_track_mm?, \
             min_clearance_mm?, min_via_drill_mm?, min_via_diameter_mm?, notes?} or null to clear. \
             An empty array clears a list section. Unknown keys, duplicate names, a net in two \
             rails or in both a rail and 'currents', and non-positive values are refused. When a \
             record exists, pass expected_revision from get_design_intent; a mismatch is \
             stale_target and writes nothing. dry_run validates and returns the result without \
             writing. The project directory must hold a .kicad_pro file.",
            json!({
                "type": "object",
                "properties": {
                    "project_dir": {
                        "type": "string",
                        "description": "KiCad project directory. If omitted, uses the configured project_dir."
                    },
                    "section": { "type": "string", "enum": SECTIONS },
                    // Open here on purpose: dispatch closes object schemas by
                    // default, and the section's typed model is the contract.
                    "value": {
                        "type": ["array", "object", "null"],
                        "additionalProperties": true,
                        "description": "The whole new section, in the shape the tool description gives."
                    },
                    "expected_revision": {
                        "type": ["string", "null"],
                        "description": "The revision from get_design_intent. Required when a record exists; omit it or pass null when none does."
                    },
                    "dry_run": { "type": "boolean", "default": false }
                },
                "required": ["section", "value"],
                "additionalProperties": false
            }),
            |args, ctx| async move { handle_set_design_intent(args, ctx).await }
        ),
    ]
}

// ─── Handlers ────────────────────────────────────────────────────────────────

fn project_dir_arg(args: &Value, ctx: &ToolContext) -> Result<PathBuf, CallToolResult> {
    if !args["project_dir"].is_null() && !args["project_dir"].is_string() {
        return Err(invalid_arg("project_dir", "expected a string"));
    }
    resolve_project_dir(args, ctx).map_err(|error| invalid_arg("project_dir", &error.to_string()))
}

async fn handle_get_design_intent(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    Ok(match project_dir_arg(args, ctx) {
        Ok(dir) => get_design_intent_at(&dir),
        Err(refusal) => refusal,
    })
}

async fn handle_set_design_intent(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    Ok(match project_dir_arg(args, ctx) {
        Ok(dir) => set_design_intent_at(&dir, args),
        Err(refusal) => refusal,
    })
}

fn get_design_intent_at(project_dir: &Path) -> CallToolResult {
    let path = design_intent_path(project_dir);
    let (source, intent, revision) = match read_design_intent(&path) {
        IntentRead::Absent => ("absent", DesignIntent::empty(), None),
        IntentRead::Loaded { intent, text } => ("file", *intent, Some(revision_of(&text))),
        IntentRead::Unusable { reason } => return unusable(&path, reason),
    };
    CallToolResult::json(&json!({
        "source": source,
        "project_dir": project_dir.display().to_string(),
        "path": path.display().to_string(),
        "revision": revision,
        "intent": if source == "file" { json!(intent) } else { Value::Null },
        "missing_sections": intent.missing_sections(),
        "net_names_checked": false,
        "note": if source == "file" {
            "Design intent as recorded. Net names were not checked against the schematic or board."
        } else {
            "No design intent is recorded for this project. Results that depend on rails, \
             currents, interfaces or the fab target must say so rather than assume values."
        },
    }))
}

/// Parse `value` as one section and put it into `intent`.
fn replace_section(
    intent: &mut DesignIntent,
    section: &str,
    value: &Value,
) -> Result<(), CallToolResult> {
    fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, CallToolResult> {
        serde_json::from_value(value.clone())
            .map_err(|error| invalid_arg("value", &error.to_string()))
    }
    // serde would also read a struct from a JSON array, field by position, so
    // the section's shape is checked before it is parsed.
    let shape_ok = match section {
        "fab_target" => value.is_object() || value.is_null(),
        _ => value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_object)),
    };
    if !shape_ok && SECTIONS.contains(&section) {
        let wanted = if section == "fab_target" {
            "an object, or null to clear"
        } else {
            "an array of objects"
        };
        return Err(invalid_arg(
            "value",
            &format!("invalid type: section '{section}' must be {wanted}"),
        ));
    }
    match section {
        "rails" => intent.rails = parse(value)?,
        "currents" => intent.currents = parse(value)?,
        "interfaces" => intent.interfaces = parse(value)?,
        "fab_target" => intent.fab_target = parse(value)?,
        other => {
            return Err(invalid_arg(
                "section",
                &format!("'{other}' is not one of {}", SECTIONS.join(", ")),
            ))
        }
    }
    Ok(())
}

fn set_design_intent_at(project_dir: &Path, args: &Value) -> CallToolResult {
    set_design_intent_with(project_dir, args, persist)
}

/// Persistence is injectable so a test can make a write land wrong and prove
/// the read-back refuses to report it as applied.
fn set_design_intent_with(
    project_dir: &Path,
    args: &Value,
    persist: impl FnOnce(&Path, Option<&str>, &str) -> Result<(), CallToolResult>,
) -> CallToolResult {
    let section = match require_str(args, "section") {
        Ok(section) => section,
        Err(refusal) => return refusal,
    };
    let Some(value) = args.get("value") else {
        return invalid_arg("value", "missing");
    };
    let expected = match &args["expected_revision"] {
        Value::Null => None,
        Value::String(revision) => Some(revision.as_str()),
        _ => return invalid_arg("expected_revision", "expected a string or null"),
    };
    let dry_run = match &args["dry_run"] {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        _ => return invalid_arg("dry_run", "expected a boolean"),
    };
    if let Some(refusal) = project_dir_refusal(project_dir) {
        return refusal;
    }

    let path = design_intent_path(project_dir);
    let (mut intent, previous_text) = match read_design_intent(&path) {
        IntentRead::Absent => {
            if let Some(expected) = expected {
                return stale(
                    &path,
                    format!("expected_revision is '{expected}', but no record exists"),
                );
            }
            (DesignIntent::empty(), None)
        }
        IntentRead::Loaded { intent, text } => {
            let current = revision_of(&text);
            match expected {
                None => {
                    return stale(
                        &path,
                        "a record exists and expected_revision was not given".to_string(),
                    )
                }
                Some(expected) if expected != current => {
                    return stale(
                        &path,
                        format!("expected_revision '{expected}' is not the current '{current}'"),
                    )
                }
                Some(_) => {}
            }
            (*intent, Some(text))
        }
        IntentRead::Unusable { reason } => return unusable(&path, reason),
    };
    let before = intent.clone();

    if let Err(refusal) = replace_section(&mut intent, section, value) {
        return refusal;
    }
    let problems = validate(&intent);
    if !problems.is_empty() {
        return invalid_arg("value", &problems.join("; "));
    }

    let previous_revision = previous_text.as_deref().map(revision_of);
    if previous_text.is_some() && intent == before {
        return CallToolResult::json(&json!({
            "status": "unchanged",
            "section": section,
            "path": path.display().to_string(),
            "revision": previous_revision,
            "intent": intent,
            "missing_sections": intent.missing_sections(),
        }));
    }

    let content = match serde_json::to_string_pretty(&intent) {
        Ok(text) => text + "\n",
        Err(error) => return CallToolResult::error(format!("set_design_intent: {error}")),
    };
    let candidate_revision = revision_of(&content);
    if dry_run {
        return CallToolResult::json(&json!({
            "status": "preview",
            "section": section,
            "path": path.display().to_string(),
            "revision": previous_revision,
            "candidate_revision": candidate_revision,
            "intent": intent,
            "missing_sections": intent.missing_sections(),
            "written": false,
        }));
    }

    if let Err(refusal) = persist(&path, previous_text.as_deref(), &content) {
        return refusal;
    }
    match read_design_intent(&path) {
        IntentRead::Loaded {
            intent: observed,
            text,
        } if *observed == intent && text == content => CallToolResult::json(&json!({
            "status": "applied",
            "section": section,
            "path": path.display().to_string(),
            "created": previous_text.is_none(),
            "previous_revision": previous_revision,
            "revision": revision_of(&text),
            "intent": observed,
            "missing_sections": observed.missing_sections(),
        })),
        other => outcome_uncertain(
            &path,
            format!("the saved file did not read back as written ({other:?})"),
        ),
    }
}

/// Create the file when none was read; otherwise replace it only if it still
/// holds the text that was read.
fn persist(path: &Path, expected: Option<&str>, content: &str) -> Result<(), CallToolResult> {
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            return Err(CallToolResult::error(format!(
                "set_design_intent could not create '{}': {error}. Nothing was written.",
                parent.display()
            )));
        }
    }
    let written = match expected {
        Some(expected) => write_atomic_if_unchanged(path, expected, content),
        None => write_new_atomic(path, content),
    };
    match written {
        Ok(()) => Ok(()),
        // A no-clobber create that finds a file replaced nothing.
        Err(SexpError::Io(error))
            if expected.is_none() && error.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            Err(CallToolResult::error_kind(
                ToolErrorKind::Conflict {
                    paths: vec![path.display().to_string()],
                },
                format!(
                    "A design-intent record appeared at '{}' while it was being created; nothing \
                     was written. Call get_design_intent and retry with its revision.",
                    path.display()
                ),
            ))
        }
        // The conditional writer cannot always say whether it replaced the
        // file before failing (#579), so any other failure is uncertain.
        Err(error) => Err(outcome_uncertain(path, error.to_string())),
    }
}

#[cfg(test)]
#[path = "design_intent_tests.rs"]
mod tests;
