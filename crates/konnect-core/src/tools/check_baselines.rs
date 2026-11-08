//! ERC and DRC baselines (creation PR 15, research rank 17, finding C4).
//!
//! Tools: save_check_baseline, compare_check_baseline, list_check_baselines
//!
//! After an edit the question is not "how many violations are there" but "did
//! I add any". Agents answered it by saving a report and diffing the next one
//! in a shell (18 such calls in real use). These tools do it in one call: a
//! baseline stores one ERC or DRC run, normalized, at
//! `<project_dir>/.konnect/baselines/<erc|drc>/<name>.json`, and a compare runs
//! the same check again and reports `new`, `resolved`, `severity_changed` and
//! the unchanged count.
//!
//! ## Stable keys
//!
//! A violation is matched by a key, never by its place in the report or its
//! coordinates. The key hashes the check kind, the DRC category, KiCad's rule
//! name, the ERC sheet path and the sorted identities of the items involved.
//! Not in the key: severity (a change of severity is reported as one),
//! the violation's prose (DRC writes measured values into it) and positions.
//!
//! An item's identity is its KiCad UUID only when that UUID is part of the
//! saved design: for DRC, when ownership enrichment resolved it against the
//! board; for ERC, when it occurs in a `.kicad_sch` of the project. A UUID that
//! is not in the files was made up by KiCad for this run and would change on
//! the next one, so such an item is keyed by its description with measured
//! numbers (`0.1500 mm`, `45°`) replaced by `#`. The response counts both kinds
//! of item key (`keying`) so a reader knows how firm the match is.
//!
//! Two violations with the same key are told apart by order of appearance
//! (`key~2`, `key~3`): they are indistinguishable, so which one is "new" does
//! not matter, only how many are.
//!
//! A DRC category that one of the two runs did not report (`None`, not an
//! empty list) is left out of the compare and named in
//! `categories_not_compared`, so "not checked" never reads as "resolved".

use super::cli::{self, DrcReport, DrcViolation, ErcReport, ErcViolation, OwnershipStatus};
use super::response_budget as budget;
use super::{invalid_arg, ToolContext, ToolDef};
use crate::mcp::{error::ToolErrorKind, protocol::CallToolResult};
use crate::tool;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

const STORE_PARENT: &str = ".konnect";
const STORE_DIR: &str = "baselines";
const SCHEMA_VERSION: u32 = 1;
const MAX_NAME_CHARS: usize = 64;
/// Bounds the walk for schematic UUIDs, so a mistaken folder is not read whole.
const MAX_SCHEMATIC_FILES: usize = 512;
const MAX_WALK_DEPTH: usize = 8;
const DEFAULT_CHANGE_LIMIT: usize = 50;
const DEFAULT_LIST_LIMIT: usize = 50;
/// Units KiCad writes after measured values in violation and item text.
const MEASURE_UNITS: &[&str] = &["mils", "mil", "mm", "µm", "um", "nm", "in", "deg", "°", "%"];

// ─── Tool definitions ─────────────────────────────────────────────────────────

pub fn tools() -> Vec<ToolDef> {
    let target_properties = json!({
        "schematic": { "type": "string", "description": "Root .kicad_sch of the project, for an ERC baseline. Pass this or board." },
        "board": { "type": "string", "description": "The .kicad_pcb, for a DRC baseline. Pass this or schematic." },
        "sync_live_board": { "type": "boolean", "default": false, "description": "DRC only: as run_drc. Bind the open board, optionally refill, save and verify before the CLI check." },
        "refill_zones": { "type": "boolean", "default": false, "description": "DRC only: as run_drc. Refill before checking." }
    });
    let mut save_properties = target_properties.clone();
    let mut compare_properties = target_properties;
    let name = json!({
        "type": "string",
        "pattern": "^[A-Za-z0-9_-][A-Za-z0-9._-]{0,63}$",
        "description": "Baseline name: letters, digits, '.', '_' and '-', up to 64 characters, not starting with '.'"
    });
    save_properties["name"] = name.clone();
    save_properties["overwrite"] = json!({ "type": "boolean", "default": false, "description": "Replace an existing baseline of this name. Without it an existing name is refused before the check runs." });
    compare_properties["name"] = name;
    compare_properties["severity"] = json!({
        "type": "string", "enum": ["error", "warning", "info"], "default": "warning",
        "description": "Least severity listed and counted. The match itself always uses every violation, so a severity change is reported as one, not as new plus resolved."
    });
    compare_properties["limit"] = json!({
        "type": "integer", "minimum": 1, "maximum": budget::MAX_LIMIT, "default": DEFAULT_CHANGE_LIMIT,
        "description": "Most changes to list (a page also stops at a size cap)"
    });
    compare_properties["offset"] = json!({ "type": "integer", "minimum": 0, "default": 0, "description": "Changes to skip: new first, then severity changes, then resolved" });
    compare_properties["output"] = json!({ "type": "string", "description": "Optional path to write the whole comparison as JSON, unchanged violations included" });

    vec![
        tool!(
            "save_check_baseline",
            "Run ERC (pass `schematic`) or DRC (pass `board`) now and store every violation, \
             normalized with a stable key, as a named baseline under the project's \
             .konnect/baselines/. Use it before an edit, then call compare_check_baseline \
             after the edit to see only what changed. Returns counts and a by-rule tally, \
             not the violation list. An existing name is refused unless overwrite is true.",
            json!({ "type": "object", "properties": save_properties, "required": ["name"] }),
            |args, ctx| async move { handle_save_check_baseline(args, ctx).await }
        ),
        tool!(
            "compare_check_baseline",
            "Run the same ERC or DRC again and compare it with a stored baseline: `new`, \
             `resolved` and `severity_changed` violations are listed (paged, size-capped), \
             unchanged ones are counted, and `verdict` is `regressed` when anything is new \
             or more severe. Violations are matched by rule and item identity, never by \
             position, so a re-run or moved parts do not create false changes. `keying` \
             says how many items were matched by UUID and how many by description.",
            json!({ "type": "object", "properties": compare_properties, "required": ["name"] }),
            |args, ctx| async move { handle_compare_check_baseline(args, ctx).await }
        ),
        tool!(
            "list_check_baselines",
            "List the ERC and DRC baselines stored for a project: name, check, source file, \
             when it was taken, its counts, and whether the source file changed since. \
             Unreadable baseline files are listed separately. Read-only.",
            json!({
                "type": "object",
                "properties": {
                    "project_dir": { "type": "string", "description": "Project folder. Or pass schematic or board, whose folder is used." },
                    "schematic": { "type": "string", "description": "A .kicad_sch in the project" },
                    "board": { "type": "string", "description": "A .kicad_pcb in the project" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": budget::MAX_LIMIT, "default": DEFAULT_LIST_LIMIT },
                    "offset": { "type": "integer", "minimum": 0, "default": 0 }
                }
            }),
            |args, ctx| async move { handle_list_check_baselines(args, ctx).await }
        ),
    ]
}

// ─── Stored shape ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckKind {
    Erc,
    Drc,
}

impl CheckKind {
    fn dir(self) -> &'static str {
        match self {
            CheckKind::Erc => "erc",
            CheckKind::Drc => "drc",
        }
    }
    fn field(self) -> &'static str {
        match self {
            CheckKind::Erc => "schematic",
            CheckKind::Drc => "board",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KeyedBy {
    Uuid,
    Description,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordItem {
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    pub keyed_by: KeyedBy,
}

/// One violation as a baseline stores it and a compare reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    pub key: String,
    /// `violations`, `unconnected_items` or `schematic_parity` for DRC; `erc`.
    pub category: String,
    pub rule: String,
    pub severity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    pub description: String,
    pub items: Vec<RecordItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DrcOptions {
    pub sync_live_board: bool,
    pub refill_zones: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Baseline {
    pub schema_version: u32,
    pub kind: CheckKind,
    pub name: String,
    /// The checked file's name, relative to the project folder.
    pub source: String,
    pub source_sha256: String,
    pub created_unix_ms: u64,
    pub created_utc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kicad_version: Option<String>,
    /// Categories this run reported. A category missing here was not checked.
    pub categories_reported: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drc_options: Option<DrcOptions>,
    pub violations: Vec<Record>,
}

/// One check run, normalized: what a baseline stores and a compare reads.
#[derive(Debug, Clone)]
pub(crate) struct Normalized {
    pub records: Vec<Record>,
    pub categories_reported: Vec<String>,
    pub kicad_version: Option<String>,
}

// ─── Normalization and keys ───────────────────────────────────────────────────

/// Replace measured values (`0.1500 mm`, `45°`, `12 mils`) with `#`. Numbers
/// that are part of a name (`R12`, `Pad 3`, `Net-(U1-PA0)`) stay: a number
/// counts as a measure only when a unit follows it and no letter precedes it.
pub(crate) fn normalize_measurements(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let signed = (c == '-' || c == '+') && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        let preceded_by_word = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
        if (c.is_ascii_digit() || signed) && !preceded_by_word {
            let start = i;
            let mut j = i + usize::from(signed);
            while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == '.') {
                j += 1;
            }
            let mut k = j;
            while k < chars.len() && chars[k] == ' ' {
                k += 1;
            }
            let rest: String = chars[k..chars.len().min(k + 4)].iter().collect();
            let unit = MEASURE_UNITS.iter().find(|unit| {
                rest.starts_with(**unit) && {
                    let after = k + unit.chars().count();
                    after >= chars.len() || !chars[after].is_alphanumeric()
                }
            });
            match unit {
                Some(unit) => {
                    out.push('#');
                    out.extend(&chars[j..k]);
                    out.push_str(unit);
                    i = k + unit.chars().count();
                }
                None => {
                    out.extend(&chars[start..j]);
                    i = j;
                }
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn item_identity(item: &RecordItem) -> String {
    match (&item.keyed_by, &item.uuid) {
        (KeyedBy::Uuid, Some(uuid)) => format!("uuid:{uuid}"),
        _ => format!("text:{}", normalize_measurements(&item.description)),
    }
}

/// The key of a violation before occurrence numbering.
fn base_key(kind: CheckKind, record: &Record) -> String {
    let mut items: Vec<String> = record.items.iter().map(item_identity).collect();
    items.sort();
    let canonical = [
        kind.dir(),
        &record.category,
        &record.rule,
        record.sheet.as_deref().unwrap_or(""),
        &items.join("\u{1e}"),
    ]
    .join("\u{1f}");
    let digest = Sha256::digest(canonical.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{}-{hex}", kind.dir())
}

/// Give every record its key, numbering repeats in order of appearance.
pub(crate) fn assign_keys(kind: CheckKind, records: &mut [Record]) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for record in records.iter_mut() {
        let base = base_key(kind, record);
        let count = seen.entry(base.clone()).or_default();
        *count += 1;
        record.key = if *count == 1 {
            base
        } else {
            format!("{base}~{count}")
        };
    }
}

fn record_item(item: &cli::ReportItem, keyed_by: KeyedBy) -> RecordItem {
    RecordItem {
        description: item.description.clone(),
        uuid: item.uuid.clone(),
        x: item.pos.as_ref().map(|p| p.x),
        y: item.pos.as_ref().map(|p| p.y),
        keyed_by,
    }
}

/// Normalize an ERC report. `design_uuids` holds every UUID written in the
/// project's schematic files; an item UUID outside it is not stable.
pub(crate) fn normalize_erc(report: &ErcReport, design_uuids: &HashSet<String>) -> Normalized {
    let mut records: Vec<Record> = report
        .violations
        .iter()
        .map(|v: &ErcViolation| Record {
            key: String::new(),
            category: "erc".into(),
            rule: v.rule.clone(),
            severity: v.severity.clone(),
            sheet: v.sheet.clone(),
            description: v.description.clone(),
            items: v
                .items
                .iter()
                .map(|item| {
                    let stable = item
                        .uuid
                        .as_ref()
                        .is_some_and(|uuid| design_uuids.contains(uuid));
                    record_item(
                        item,
                        if stable {
                            KeyedBy::Uuid
                        } else {
                            KeyedBy::Description
                        },
                    )
                })
                .collect(),
        })
        .collect();
    assign_keys(CheckKind::Erc, &mut records);
    Normalized {
        records,
        categories_reported: vec!["erc".into()],
        kicad_version: report.coordinates.kicad_version.clone(),
    }
}

/// Normalize a DRC report. An item is keyed by UUID only when enrichment
/// resolved that UUID in the saved board.
pub(crate) fn normalize_drc(report: &DrcReport) -> Normalized {
    let categories: [(&str, Option<&Vec<DrcViolation>>); 3] = [
        ("violations", Some(&report.violations)),
        ("unconnected_items", report.unconnected_items.as_ref()),
        ("schematic_parity", report.schematic_parity.as_ref()),
    ];
    let mut records = Vec::new();
    let mut reported = Vec::new();
    for (category, list) in categories {
        let Some(list) = list else { continue };
        reported.push(category.to_string());
        for v in list {
            records.push(Record {
                key: String::new(),
                category: category.into(),
                rule: v.rule.clone(),
                severity: v.severity.clone(),
                sheet: None,
                description: v.description.clone(),
                items: v
                    .items
                    .iter()
                    .map(|item| {
                        let stable = item.uuid.is_some()
                            && item.ownership_status == Some(OwnershipStatus::Resolved);
                        record_item(
                            item,
                            if stable {
                                KeyedBy::Uuid
                            } else {
                                KeyedBy::Description
                            },
                        )
                    })
                    .collect(),
            });
        }
    }
    assign_keys(CheckKind::Drc, &mut records);
    Normalized {
        records,
        categories_reported: reported,
        kicad_version: None,
    }
}

// ─── Compare ──────────────────────────────────────────────────────────────────

pub(crate) fn severity_rank(severity: &str) -> u8 {
    match severity {
        "error" => 2,
        "warning" => 1,
        _ => 0,
    }
}

#[derive(Debug, Default)]
pub(crate) struct Diff {
    pub new: Vec<Record>,
    pub resolved: Vec<Record>,
    /// (baseline, current) pairs whose key matched but severity did not.
    pub severity_changed: Vec<(Record, Record)>,
    pub unchanged: Vec<Record>,
    pub categories_not_compared: Vec<String>,
}

/// Match two runs by key. Categories either side did not report are left out.
pub(crate) fn diff(
    baseline: &[Record],
    baseline_categories: &[String],
    current: &[Record],
    current_categories: &[String],
) -> Diff {
    let compared: HashSet<&String> = baseline_categories
        .iter()
        .filter(|c| current_categories.contains(c))
        .collect();
    let mut not_compared: Vec<String> = baseline_categories
        .iter()
        .chain(current_categories)
        .filter(|c| !compared.contains(c))
        .cloned()
        .collect();
    not_compared.sort();
    not_compared.dedup();

    let in_scope = |r: &&Record| compared.contains(&r.category);
    let before: HashMap<&str, &Record> = baseline
        .iter()
        .filter(in_scope)
        .map(|r| (r.key.as_str(), r))
        .collect();
    let after: HashSet<&str> = current
        .iter()
        .filter(in_scope)
        .map(|r| r.key.as_str())
        .collect();
    let mut out = Diff {
        categories_not_compared: not_compared,
        ..Diff::default()
    };
    for record in current.iter().filter(in_scope) {
        match before.get(record.key.as_str()) {
            None => out.new.push(record.clone()),
            Some(old) if old.severity != record.severity => {
                out.severity_changed.push(((*old).clone(), record.clone()))
            }
            Some(_) => out.unchanged.push(record.clone()),
        }
    }
    out.resolved = baseline
        .iter()
        .filter(in_scope)
        .filter(|r| !after.contains(r.key.as_str()))
        .cloned()
        .collect();
    out
}

fn record_json(record: &Record, change: &str) -> Value {
    let mut value = serde_json::to_value(record).unwrap_or(Value::Null);
    value["change"] = json!(change);
    value
}

pub(crate) fn keying(records: &[Record]) -> Value {
    let (mut uuid, mut text) = (0usize, 0usize);
    for item in records.iter().flat_map(|r| &r.items) {
        match item.keyed_by {
            KeyedBy::Uuid => uuid += 1,
            KeyedBy::Description => text += 1,
        }
    }
    json!({ "items_by_uuid": uuid, "items_by_description": text })
}

/// The comparison as a response: counts over every violation at or above
/// `min_rank`, a page of the changes, and per-rule tallies of new and resolved.
pub(crate) fn compare_body(diff: &Diff, min_rank: u8, offset: usize, limit: usize) -> Value {
    let keep = |r: &Record| severity_rank(&r.severity) >= min_rank;
    let new: Vec<Value> = diff
        .new
        .iter()
        .filter(|r| keep(r))
        .map(|r| record_json(r, "new"))
        .collect();
    let changed: Vec<Value> = diff
        .severity_changed
        .iter()
        .filter(|(old, now)| keep(old) || keep(now))
        .map(|(old, now)| {
            let mut value = record_json(now, "severity_changed");
            value["baseline_severity"] = json!(old.severity);
            value
        })
        .collect();
    let raised = diff
        .severity_changed
        .iter()
        .filter(|(old, now)| keep(old) || keep(now))
        .filter(|(old, now)| severity_rank(&now.severity) > severity_rank(&old.severity))
        .count();
    let resolved: Vec<Value> = diff
        .resolved
        .iter()
        .filter(|r| keep(r))
        .map(|r| record_json(r, "resolved"))
        .collect();
    let unchanged = diff.unchanged.iter().filter(|r| keep(r)).count();

    let all: Vec<Value> = new
        .iter()
        .chain(&changed)
        .chain(&resolved)
        .cloned()
        .collect();
    let page = budget::page(&all, offset, limit, budget::BYTE_BUDGET);
    let mut fields = serde_json::Map::new();
    fields.insert(
        "verdict".into(),
        json!(if new.is_empty() && raised == 0 {
            "no_regressions"
        } else {
            "regressed"
        }),
    );
    fields.insert("new_count".into(), json!(new.len()));
    fields.insert("resolved_count".into(), json!(resolved.len()));
    fields.insert("severity_changed_count".into(), json!(changed.len()));
    fields.insert("severity_raised_count".into(), json!(raised));
    fields.insert("unchanged_count".into(), json!(unchanged));
    fields.insert("new_by_rule".into(), json!(budget::tally(&new, "rule")));
    fields.insert(
        "resolved_by_rule".into(),
        json!(budget::tally(&resolved, "rule")),
    );
    fields.insert(
        "categories_not_compared".into(),
        json!(diff.categories_not_compared),
    );
    page.describe(&mut fields);
    fields.insert("changes".into(), json!(page.items));
    Value::Object(fields)
}

// ─── Storage ──────────────────────────────────────────────────────────────────

fn store_dir(project_dir: &Path, kind: CheckKind) -> PathBuf {
    project_dir
        .join(STORE_PARENT)
        .join(STORE_DIR)
        .join(kind.dir())
}

pub(crate) fn baseline_path(project_dir: &Path, kind: CheckKind, name: &str) -> PathBuf {
    store_dir(project_dir, kind).join(format!("{name}.json"))
}

/// Refuse a store folder that is a symlink or a file, so a baseline is never
/// written or read outside the project.
fn check_store_dirs(project_dir: &Path, kind: CheckKind) -> Result<(), String> {
    let mut path = project_dir.to_path_buf();
    for part in [STORE_PARENT, STORE_DIR, kind.dir()] {
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                return Err(format!(
                    "{} is not a plain folder (a symlink or a file); baselines are kept only in a real folder inside the project",
                    path.display()
                ))
            }
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn read_name(args: &Value) -> Result<String, CallToolResult> {
    match args["name"].as_str() {
        Some(name) if valid_name(name) => Ok(name.to_string()),
        Some(_) => Err(invalid_arg(
            "name",
            "use letters, digits, '.', '_' and '-' only, up to 64 characters, not starting with '.'",
        )),
        None => Err(invalid_arg("name", "is required and must be a string")),
    }
}

fn read_bool(args: &Value, field: &str) -> Result<bool, CallToolResult> {
    match &args[field] {
        Value::Null => Ok(false),
        Value::Bool(value) => Ok(*value),
        _ => Err(invalid_arg(field, "must be a boolean")),
    }
}

/// The checked file and its kind: exactly one of `schematic` and `board`.
fn read_target(args: &Value) -> Result<(CheckKind, PathBuf), CallToolResult> {
    let schematic = args["schematic"].as_str();
    let board = args["board"].as_str();
    for field in ["schematic", "board"] {
        if !args[field].is_null() && !args[field].is_string() {
            return Err(invalid_arg(field, "must be a string path"));
        }
    }
    let (kind, path, extension) = match (schematic, board) {
        (Some(path), None) => (CheckKind::Erc, path, "kicad_sch"),
        (None, Some(path)) => (CheckKind::Drc, path, "kicad_pcb"),
        (Some(_), Some(_)) => {
            return Err(invalid_arg(
                "schematic",
                "pass schematic (ERC) or board (DRC), not both",
            ))
        }
        (None, None) => {
            return Err(invalid_arg(
                "schematic",
                "pass schematic (ERC) or board (DRC)",
            ))
        }
    };
    let path = PathBuf::from(path);
    if path.extension().and_then(|e| e.to_str()) != Some(extension) {
        return Err(invalid_arg(
            kind.field(),
            &format!("must be a .{extension} file"),
        ));
    }
    if !path.is_file() {
        return Err(CallToolResult::error_kind(
            ToolErrorKind::FileNotFound {
                path: path.display().to_string(),
            },
            format!("{} does not exist; nothing was run.", path.display()),
        ));
    }
    if kind == CheckKind::Erc
        && (args["sync_live_board"] == json!(true) || args["refill_zones"] == json!(true))
    {
        return Err(invalid_arg(
            "sync_live_board",
            "sync_live_board and refill_zones apply to DRC (board) only",
        ));
    }
    Ok((kind, path))
}

fn project_dir_of(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub(crate) fn sha256_file(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

fn unusable(path: &Path, reason: impl std::fmt::Display) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::InvalidConfiguration {
            path: path.display().to_string(),
            reason: reason.to_string(),
        },
        format!(
            "The baseline at {} cannot be used: {reason}. It was left as it is; save a new baseline under another name, or pass overwrite: true to replace it.",
            path.display()
        ),
    )
}

/// Parse and check a stored baseline against where it was found.
pub(crate) fn parse_baseline(text: &str, kind: CheckKind, name: &str) -> Result<Baseline, String> {
    let baseline: Baseline = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if baseline.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "schema_version {} is not {SCHEMA_VERSION}",
            baseline.schema_version
        ));
    }
    if baseline.kind != kind {
        return Err(format!(
            "it records a {:?} run but sits in the {} folder",
            baseline.kind,
            kind.dir()
        ));
    }
    if baseline.name != name {
        return Err(format!(
            "it is named '{}' inside but stored as '{name}'",
            baseline.name
        ));
    }
    let mut keys = HashSet::new();
    for record in &baseline.violations {
        if !keys.insert(record.key.as_str()) {
            return Err(format!("key {} appears twice", record.key));
        }
        if !baseline.categories_reported.contains(&record.category) {
            return Err(format!(
                "violation {} is in category '{}', which the baseline says was not reported",
                record.key, record.category
            ));
        }
    }
    Ok(baseline)
}

/// Every UUID written in the project's `.kicad_sch` files. Hidden folders and
/// KiCad backups are skipped, as are symlinks.
pub(crate) fn schematic_uuids(project_dir: &Path) -> HashSet<String> {
    let mut uuids = HashSet::new();
    let mut files = 0usize;
    let mut stack = vec![(project_dir.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() || name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                if depth < MAX_WALK_DEPTH && !name.ends_with("-backups") {
                    stack.push((path, depth + 1));
                }
            } else if name.ends_with(".kicad_sch") && files < MAX_SCHEMATIC_FILES {
                files += 1;
                if let Ok(text) = std::fs::read_to_string(&path) {
                    collect_uuids(&text, &mut uuids);
                }
            }
        }
    }
    uuids
}

fn collect_uuids(text: &str, into: &mut HashSet<String>) {
    let mut rest = text;
    while let Some(at) = rest.find("(uuid") {
        rest = &rest[at + 5..];
        let token = rest.trim_start();
        if token.len() == rest.len() {
            continue; // `(uuidx`, not the uuid node
        }
        let token = token.trim_start_matches('"');
        let end = token
            .find(|c: char| c == '"' || c == ')' || c.is_whitespace())
            .unwrap_or(token.len());
        if end > 0 {
            into.insert(token[..end].to_string());
        }
    }
}

// ─── Running a check ──────────────────────────────────────────────────────────

/// The DRC arguments `drc::run` reads, and nothing else.
fn drc_args(args: &Value) -> Value {
    let mut subset = json!({});
    for field in ["sync_live_board", "refill_zones", "output"] {
        if !args[field].is_null() {
            subset[field] = args[field].clone();
        }
    }
    subset
}

async fn run_check(
    ctx: &ToolContext,
    kind: CheckKind,
    path: &Path,
    args: &Value,
) -> anyhow::Result<Result<(Normalized, Option<DrcOptions>, Value), CallToolResult>> {
    match kind {
        CheckKind::Erc => {
            let report = match super::sch_export::erc_report(ctx, path).await? {
                Ok(report) => report,
                Err(refusal) => return Ok(Err(refusal)),
            };
            let project_dir = project_dir_of(path);
            let uuids = tokio::task::spawn_blocking(move || schematic_uuids(&project_dir)).await?;
            let coordinates = serde_json::to_value(&report.coordinates)?;
            Ok(Ok((
                normalize_erc(&report, &uuids),
                None,
                json!({ "coordinates": coordinates }),
            )))
        }
        CheckKind::Drc => {
            let options = DrcOptions {
                sync_live_board: match read_bool(args, "sync_live_board") {
                    Ok(value) => value,
                    Err(refusal) => return Ok(Err(refusal)),
                },
                refill_zones: match read_bool(args, "refill_zones") {
                    Ok(value) => value,
                    Err(refusal) => return Ok(Err(refusal)),
                },
            };
            let (report, evidence) = match super::drc::run(ctx, path, &drc_args(args)).await? {
                Ok(result) => result,
                Err(refusal) => return Ok(Err(refusal)),
            };
            let mut extra = json!({ "source_evidence": evidence });
            if let Some(reason) = &report.ownership_diagnostic {
                extra["ownership_diagnostic"] = json!(reason);
            }
            if let Some(reason) = &report.schematic_parity_diagnostic {
                extra["schematic_parity_diagnostic"] = json!(reason);
            }
            Ok(Ok((normalize_drc(&report), Some(options), extra)))
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

pub(crate) async fn handle_save_check_baseline(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let (kind, source) = match read_target(args) {
        Ok(target) => target,
        Err(refusal) => return Ok(refusal),
    };
    let name = match read_name(args) {
        Ok(name) => name,
        Err(refusal) => return Ok(refusal),
    };
    let overwrite = match read_bool(args, "overwrite") {
        Ok(value) => value,
        Err(refusal) => return Ok(refusal),
    };
    if !args["output"].is_null() {
        return Ok(invalid_arg(
            "output",
            "save_check_baseline stores its own file; use run_erc or run_drc for a report",
        ));
    }
    let project_dir = project_dir_of(&source);
    if let Err(reason) = check_store_dirs(&project_dir, kind) {
        return Ok(unusable(&store_dir(&project_dir, kind), reason));
    }
    let path = baseline_path(&project_dir, kind, &name);
    // Refuse before the check runs: a DRC with sync_live_board saves the board.
    let previous = match std::fs::read_to_string(&path) {
        Ok(text) if overwrite => Some(text),
        Ok(_) => {
            return Ok(CallToolResult::error_kind(
                ToolErrorKind::InvalidArgument {
                    field: "name".into(),
                    reason: format!("a {} baseline named '{name}' already exists", kind.dir()),
                },
                format!(
                    "A {} baseline named '{name}' already exists at {}. Pass overwrite: true to replace it, or choose another name. Nothing was run.",
                    kind.dir(),
                    path.display()
                ),
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Ok(unusable(&path, error)),
    };

    let (normalized, drc_options, extra) = match run_check(ctx, kind, &source, args).await? {
        Ok(run) => run,
        Err(refusal) => return Ok(refusal),
    };
    // Hash after the run: with sync_live_board the board was saved by it.
    let source_sha256 = sha256_file(&source)?;
    let created_unix_ms = now_ms();
    let baseline = Baseline {
        schema_version: SCHEMA_VERSION,
        kind,
        name: name.clone(),
        source: file_name(&source),
        source_sha256: source_sha256.clone(),
        created_unix_ms,
        created_utc: crate::simulation::runs::stamp(created_unix_ms / 1000),
        kicad_version: normalized.kicad_version.clone(),
        categories_reported: normalized.categories_reported.clone(),
        drc_options,
        violations: normalized.records,
    };
    let text = format!("{}\n", serde_json::to_string_pretty(&baseline)?);

    let dir = store_dir(&project_dir, kind);
    std::fs::create_dir_all(&dir)?;
    if let Err(reason) = check_store_dirs(&project_dir, kind) {
        return Ok(unusable(&dir, reason));
    }
    let written = {
        let (path, text, previous) = (path.clone(), text.clone(), previous.clone());
        tokio::task::spawn_blocking(move || match previous {
            Some(expected) => {
                konnect_sexp::writer::write_atomic_if_unchanged(&path, &expected, &text)
            }
            None => konnect_sexp::writer::write_new_atomic(&path, &text),
        })
        .await?
    };
    if let Err(error) = written {
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::StaleTarget {
                target: path.display().to_string(),
                reason: error.to_string(),
            },
            format!(
                "The baseline was not written: {error}. Another writer may have changed {} meanwhile; list the baselines and retry.",
                path.display()
            ),
        ));
    }
    // The write is not its own witness.
    let stored = std::fs::read_to_string(&path)?;
    match parse_baseline(&stored, kind, &name) {
        Ok(read) if read == baseline => {}
        Ok(_) => {
            return Ok(unusable(
                &path,
                "the file read back differs from what was written",
            ))
        }
        Err(reason) => return Ok(unusable(&path, reason)),
    }

    let violations: Vec<Value> = baseline
        .violations
        .iter()
        .map(|r| json!({ "rule": r.rule, "severity": r.severity }))
        .collect();
    let mut body = json!({
        "saved": true,
        "replaced": previous.is_some(),
        "name": name,
        "check": kind.dir(),
        "path": path.display().to_string(),
        "source": baseline.source,
        "source_sha256": source_sha256,
        "created_utc": baseline.created_utc,
        "violation_count": baseline.violations.len(),
        "errors": baseline.violations.iter().filter(|r| r.severity == "error").count(),
        "warnings": baseline.violations.iter().filter(|r| r.severity == "warning").count(),
        "by_rule": budget::tally(&violations, "rule"),
        "categories_reported": baseline.categories_reported,
        "keying": keying(&baseline.violations),
    });
    merge(&mut body, extra);
    Ok(CallToolResult::json(&body))
}

fn merge(body: &mut Value, extra: Value) {
    if let (Some(body), Value::Object(extra)) = (body.as_object_mut(), extra) {
        body.extend(extra);
    }
}

pub(crate) async fn handle_compare_check_baseline(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let (kind, source) = match read_target(args) {
        Ok(target) => target,
        Err(refusal) => return Ok(refusal),
    };
    let name = match read_name(args) {
        Ok(name) => name,
        Err(refusal) => return Ok(refusal),
    };
    let severity = match &args["severity"] {
        Value::Null => "warning",
        Value::String(s) if matches!(s.as_str(), "error" | "warning" | "info") => s.as_str(),
        _ => {
            return Ok(invalid_arg(
                "severity",
                "must be 'error', 'warning' or 'info'",
            ))
        }
    };
    let (offset, limit) = match budget::window(args, DEFAULT_CHANGE_LIMIT) {
        Ok(window) => window,
        Err(refusal) => return Ok(refusal),
    };
    let output = match &args["output"] {
        Value::Null => None,
        Value::String(path) => Some(path.clone()),
        _ => return Ok(invalid_arg("output", "must be a string path")),
    };

    // Load the baseline first, so a wrong name runs nothing.
    let loaded = match load_for_compare(&source, kind, &name, "name") {
        Ok(loaded) => loaded,
        Err(refusal) => return Ok(refusal),
    };
    let baseline = loaded.baseline;

    let (current, drc_options, extra) = match run_check(ctx, kind, &source, args).await? {
        Ok(run) => run,
        Err(refusal) => return Ok(refusal),
    };
    let source_sha256 = sha256_file(&source)?;
    let result = diff(
        &baseline.violations,
        &baseline.categories_reported,
        &current.records,
        &current.categories_reported,
    );
    let warnings = compare_warnings(&baseline, drc_options, &current, &result);

    if let Some(output) = &output {
        let full = json!({
            "baseline": name,
            "check": kind.dir(),
            "source": baseline.source,
            "severity_filter": severity,
            "new": result.new,
            "severity_changed": result.severity_changed.iter().map(|(old, now)| {
                let mut value = serde_json::to_value(now).unwrap_or(Value::Null);
                value["baseline_severity"] = json!(old.severity);
                value
            }).collect::<Vec<_>>(),
            "resolved": result.resolved,
            "unchanged": result.unchanged,
            "categories_not_compared": result.categories_not_compared,
        });
        if let Err(error) =
            super::drc::write_report(output, &serde_json::to_string_pretty(&full)?).await
        {
            return Ok(CallToolResult::error_kind(
                ToolErrorKind::HandlerError {
                    reason: format!("could not write the comparison to {output}: {error:#}"),
                },
                format!("The check ran and the baseline is unchanged, but the comparison could not be written to {output}: {error:#}"),
            ));
        }
    }

    let mut body = compare_body(&result, severity_rank(severity), offset, limit);
    merge(
        &mut body,
        json!({
            "baseline": name,
            "check": kind.dir(),
            "source": baseline.source,
            "baseline_created_utc": baseline.created_utc,
            "baseline_count": baseline.violations.len(),
            "current_count": current.records.len(),
            "severity_filter": severity,
            "source_changed_since_baseline": baseline.source_sha256 != source_sha256,
            "keying": keying(&current.records),
            "warnings": warnings,
        }),
    );
    if let Some(output) = output {
        body["output"] = json!(output);
    }
    merge(&mut body, extra);
    Ok(CallToolResult::json(&body))
}

/// A stored baseline ready to compare against: its keys reassigned with this
/// binary's rule, where it was read from, and its revision, the SHA-256 of
/// the stored file, so a reader can tell which version of a name was used.
#[derive(Debug)]
pub(crate) struct LoadedBaseline {
    pub baseline: Baseline,
    pub path: PathBuf,
    pub revision: String,
}

/// Load the `kind` baseline called `name` for `source`, before any check
/// runs. A missing name lists the stored ones; a damaged file, a store folder
/// that is not a plain folder, or a baseline taken on another file is
/// refused. `field` is the argument that named the baseline. The refusal is the response to return: nothing was run.
pub(crate) fn load_for_compare(
    source: &Path,
    kind: CheckKind,
    name: &str,
    field: &str,
) -> Result<LoadedBaseline, CallToolResult> {
    let project_dir = project_dir_of(source);
    if let Err(reason) = check_store_dirs(&project_dir, kind) {
        return Err(unusable(&store_dir(&project_dir, kind), reason));
    }
    let path = baseline_path(&project_dir, kind, name);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let names = stored_names(&project_dir, kind);
            return Err(CallToolResult::error_kind(
                ToolErrorKind::InvalidArgument {
                    field: field.into(),
                    reason: format!("no {} baseline named '{name}'", kind.dir()),
                },
                format!(
                    "No {} baseline named '{name}' for this project. Stored: {}. Save one with save_check_baseline first. Nothing was run.",
                    kind.dir(),
                    if names.is_empty() { "none".to_string() } else { names.join(", ") }
                ),
            ));
        }
        Err(error) => return Err(unusable(&path, error)),
    };
    let mut baseline = match parse_baseline(&text, kind, name) {
        Ok(baseline) => baseline,
        Err(reason) => return Err(unusable(&path, reason)),
    };
    // Key the stored items again with this binary's rule, so the two sides
    // are always keyed the same way.
    assign_keys(kind, &mut baseline.violations);
    if baseline.source != file_name(source) {
        return Err(invalid_arg(
            kind.field(),
            &format!(
                "baseline '{name}' was taken on {}, not {}; compare a file with its own baseline",
                baseline.source,
                file_name(source)
            ),
        ));
    }
    Ok(LoadedBaseline {
        baseline,
        path,
        revision: format!("{:x}", Sha256::digest(text.as_bytes())),
    })
}

/// What can make a comparison misleading without making it wrong: options or
/// a KiCad version that differ from the baseline's, and categories only one
/// of the two runs reported.
pub(crate) fn compare_warnings(
    baseline: &Baseline,
    drc_options: Option<DrcOptions>,
    current: &Normalized,
    result: &Diff,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if baseline.drc_options != drc_options {
        warnings.push(format!(
            "options_differ: the baseline ran with {:?}, this run with {:?}; zone-fill violations may differ for that reason alone",
            baseline.drc_options, drc_options
        ));
    }
    if baseline.kicad_version.is_some()
        && current.kicad_version.is_some()
        && baseline.kicad_version != current.kicad_version
    {
        warnings.push(format!(
            "kicad_version_differs: baseline {}, now {}; a rule KiCad changed can show as new or resolved",
            baseline.kicad_version.as_deref().unwrap_or(""),
            current.kicad_version.as_deref().unwrap_or("")
        ));
    }
    if !result.categories_not_compared.is_empty() {
        warnings.push(format!(
            "categories_not_compared: {} was reported by only one of the two runs, so its violations are neither new nor resolved",
            result.categories_not_compared.join(", ")
        ));
    }
    warnings
}

fn stored_names(project_dir: &Path, kind: CheckKind) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(store_dir(project_dir, kind))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json")
                .filter(|n| valid_name(n))
                .map(str::to_string)
        })
        .collect();
    names.sort();
    names
}

pub(crate) async fn handle_list_check_baselines(
    args: &Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let given: Vec<&str> = ["project_dir", "schematic", "board"]
        .into_iter()
        .filter(|field| !args[*field].is_null())
        .collect();
    let project_dir = match given.as_slice() {
        [field] => match args[*field].as_str() {
            Some(path) if *field == "project_dir" => PathBuf::from(path),
            Some(path) => project_dir_of(Path::new(path)),
            None => return Ok(invalid_arg(field, "must be a string path")),
        },
        [] => {
            return Ok(invalid_arg(
                "project_dir",
                "pass project_dir, schematic or board",
            ))
        }
        _ => {
            return Ok(invalid_arg(
                "project_dir",
                "pass only one of project_dir, schematic and board",
            ))
        }
    };
    if !project_dir.is_dir() {
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::FileNotFound {
                path: project_dir.display().to_string(),
            },
            format!("{} is not a folder.", project_dir.display()),
        ));
    }
    let (offset, limit) = match budget::window(args, DEFAULT_LIST_LIMIT) {
        Ok(window) => window,
        Err(refusal) => return Ok(refusal),
    };

    let mut rows: Vec<(u64, Value)> = Vec::new();
    let mut unreadable = Vec::new();
    let mut digests: BTreeMap<String, Option<String>> = BTreeMap::new();
    for kind in [CheckKind::Erc, CheckKind::Drc] {
        if let Err(reason) = check_store_dirs(&project_dir, kind) {
            unreadable.push(json!({ "path": store_dir(&project_dir, kind).display().to_string(), "reason": reason }));
            continue;
        }
        for name in stored_names(&project_dir, kind) {
            let path = baseline_path(&project_dir, kind, &name);
            let parsed = std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| parse_baseline(&text, kind, &name));
            let baseline = match parsed {
                Ok(baseline) => baseline,
                Err(reason) => {
                    unreadable
                        .push(json!({ "path": path.display().to_string(), "reason": reason }));
                    continue;
                }
            };
            let digest = digests
                .entry(baseline.source.clone())
                .or_insert_with(|| sha256_file(&project_dir.join(&baseline.source)).ok())
                .clone();
            rows.push((
                baseline.created_unix_ms,
                json!({
                    "name": baseline.name,
                    "check": kind.dir(),
                    "source": baseline.source,
                    "created_utc": baseline.created_utc,
                    "violation_count": baseline.violations.len(),
                    "errors": baseline.violations.iter().filter(|r| r.severity == "error").count(),
                    "warnings": baseline.violations.iter().filter(|r| r.severity == "warning").count(),
                    "source_state": match digest {
                        None => "missing",
                        Some(digest) if digest == baseline.source_sha256 => "unchanged",
                        Some(_) => "changed",
                    },
                }),
            ));
        }
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.0));
    let all: Vec<Value> = rows.into_iter().map(|(_, row)| row).collect();
    let mut body = budget::list_response("baselines", &all, offset, limit);
    body["project_dir"] = json!(project_dir.display().to_string());
    body["unreadable"] = json!(unreadable);
    Ok(CallToolResult::json(&body))
}

#[cfg(test)]
#[path = "check_baselines_tests.rs"]
mod tests;
