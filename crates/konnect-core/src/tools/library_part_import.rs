//! `import_library_part`: copy one part's KiCad-format symbol, footprint and
//! optional 3D model from local files into a project-local library.
//!
//! This is rank 33 (finding B20), rescoped on 2026-11-08. Peer servers import
//! parts by LCSC number by fetching EasyEDA's internal component API. EasyEDA's
//! terms (effective 2026-09-21, section VII) restrict automated collection of
//! their content and derivative works from it, and their robots.txt disallows
//! `/api/`. So this tool never touches the network. The owner exports or
//! downloads the part themself (EasyEDA/LCSC export, SnapEDA, Ultra Librarian,
//! a vendor's KiCad files) and the tool takes it from there.
//!
//! Contract:
//! - Preview by default. The preview shows every file it would write or
//!   append, the symbol's pins, the footprint's pads, the library-table
//!   registrations, the pin-pad check on the candidate, and a `revision`.
//! - Apply only with that revision. The plan is recomputed from disk and
//!   refused as `stale_target` when anything it depends on changed.
//! - Never overwrite. An existing symbol, or a different footprint or model
//!   file at the target path, blocks the import; an identical footprint or
//!   model is reused. A project nickname already bound to another library, a
//!   library already registered under another nickname, and a disabled
//!   registration block it too.
//! - Everything is written in one durable multi-file transaction, then read
//!   back, and the pin-pad check runs again on the saved files.
//! - No format conversion. The inputs must already be KiCad 6+ files.

use super::*;
use crate::mcp::error::ToolErrorKind;
use konnect_sexp::{commit_file_transaction, FileTransition, SexpError};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Default project nickname when the caller names none.
const DEFAULT_LIBRARY: &str = "imported_parts";
/// Largest symbol library or footprint file read. A single part is a few
/// hundred KiB at most; anything far larger is not one part.
const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
/// Largest 3D model copied. Vendor STEP files of large connectors reach tens
/// of MiB; the transaction journal holds a copy while it commits.
const MAX_MODEL_BYTES: u64 = 64 * 1024 * 1024;
/// Most pins or pads listed in a response; counts are always complete.
const MAX_LISTED: usize = 300;
const OPERATION: &str = "import_library_part";

pub(super) fn tool() -> ToolDef {
    tool!(
        "import_library_part",
        "Import one part's symbol, footprint and optional 3D model from local KiCad-format files \
         (for example an EasyEDA/LCSC export, SnapEDA or Ultra Librarian download the user saved) \
         into a project-local library: <library>.kicad_sym, <library>.pretty and \
         <library>.3dshapes in the project folder, registered in the project's sym-lib-table and \
         fp-lib-table. Makes no network requests. Previews by default (dry_run: true): returns \
         the files it would write, the symbol pins, the footprint pads, the pin-pad check and a \
         revision; apply with dry_run: false and that expected_revision. Never overwrites: an \
         identical footprint or model already in the library is reused, and an existing symbol \
         or a different footprint or model blocks the import. Sets the symbol's Footprint field to \
         <library>:<footprint> and optional LCSC, MPN and Manufacturer fields. Pin-pad mismatches \
         are reported, not fixed.",
        json!({
            "type": "object",
            "properties": {
                "project": { "type": "string", "description": "Path to the project's .kicad_pro file (or its folder)" },
                "symbol_file": { "type": "string", "description": "Local .kicad_sym file holding the part's symbol" },
                "symbol_name": { "type": "string", "description": "Symbol to import; required when symbol_file holds more than one" },
                "footprint_file": { "type": "string", "description": "Local .kicad_mod file holding the part's footprint" },
                "model_file": { "type": "string", "description": "Optional local 3D model (.step, .stp or .wrl). It replaces the footprint's model references, keeping the first one's offset, scale, rotation, visibility and opacity" },
                "library": { "type": "string", "description": "Project library nickname (letters, digits, _ . -)", "default": DEFAULT_LIBRARY },
                "lcsc": { "type": "string", "description": "LCSC part number to record in the symbol's LCSC field, e.g. C25804" },
                "mpn": { "type": "string", "description": "Manufacturer part number for the MPN field" },
                "manufacturer": { "type": "string", "description": "Manufacturer name for the Manufacturer field" },
                "dry_run": { "type": "boolean", "default": true, "description": "true (default) previews; false applies" },
                "expected_revision": { "type": "string", "description": "The revision a preview returned; required to apply" }
            },
            "required": ["project", "symbol_file", "footprint_file"],
            "additionalProperties": false
        }),
        |args, ctx| async move { handle_import_library_part(args, ctx).await }
    )
}

// ─── Request ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Request {
    project: String,
    symbol_file: PathBuf,
    symbol_name: Option<String>,
    footprint_file: PathBuf,
    model_file: Option<PathBuf>,
    library: String,
    fields: BTreeMap<String, String>,
    dry_run: bool,
    expected_revision: Option<String>,
}

fn invalid(field: &str, reason: impl Into<String>) -> CallToolResult {
    invalid_library_argument(field, reason)
}

fn optional_string(args: &Value, key: &str) -> Result<Option<String>, CallToolResult> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            if value.chars().any(char::is_control) {
                return Err(invalid(key, "must not contain control characters"));
            }
            Ok(Some(value.clone()))
        }
        Some(_) => Err(invalid(key, "must be a string")),
    }
}

fn required_path(args: &Value, key: &str) -> Result<PathBuf, CallToolResult> {
    match optional_string(args, key)? {
        Some(value) if !value.trim().is_empty() => Ok(PathBuf::from(value)),
        _ => Err(invalid(key, "is required and must be a nonempty string")),
    }
}

/// A KiCad nickname that is also a safe file stem: it names the library file,
/// the `.pretty` folder and the `.3dshapes` folder.
fn valid_nickname(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// A footprint or model file name that stays inside its folder and that
/// KiCad can address as `Library:Name`.
fn safe_file_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.trim() != name {
        return Err("must be nonempty without leading or trailing spaces");
    }
    if name == "." || name == ".." || name.starts_with('.') {
        return Err("must not start with '.'");
    }
    if name.chars().any(|c| {
        c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    }) {
        return Err("must not contain / \\ : * ? \" < > | or control characters");
    }
    if name.len() > 200 {
        return Err("is longer than 200 bytes");
    }
    Ok(())
}

fn parse_request(args: &Value) -> Result<Request, CallToolResult> {
    let project = match optional_string(args, "project")? {
        Some(value) if !value.trim().is_empty() => value,
        _ => {
            return Err(invalid(
                "project",
                "is required and must be a nonempty string",
            ))
        }
    };
    let symbol_file = required_path(args, "symbol_file")?;
    let footprint_file = required_path(args, "footprint_file")?;
    let model_file = optional_string(args, "model_file")?
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let symbol_name = optional_string(args, "symbol_name")?;
    let library = optional_string(args, "library")?.unwrap_or_else(|| DEFAULT_LIBRARY.to_string());
    if !valid_nickname(&library) {
        return Err(invalid(
            "library",
            "must be 1-64 characters of letters, digits, '_', '-' or '.', starting with a letter or digit",
        ));
    }
    let mut fields = BTreeMap::new();
    if let Some(lcsc) = optional_string(args, "lcsc")? {
        let normalized = lcsc.trim().to_ascii_uppercase();
        let digits = normalized.strip_prefix('C').unwrap_or("");
        if digits.is_empty() || digits.len() > 12 || !digits.chars().all(|c| c.is_ascii_digit()) {
            return Err(invalid("lcsc", "must be an LCSC part number like C25804"));
        }
        fields.insert("LCSC".to_string(), normalized);
    }
    for (arg, key) in [("mpn", "MPN"), ("manufacturer", "Manufacturer")] {
        if let Some(value) = optional_string(args, arg)? {
            if value.trim().is_empty() {
                return Err(invalid(arg, "must be nonempty when supplied"));
            }
            fields.insert(key.to_string(), value.trim().to_string());
        }
    }
    let dry_run = match args.get("dry_run") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => return Err(invalid("dry_run", "must be a boolean")),
    };
    let expected_revision = optional_string(args, "expected_revision")?;
    if !dry_run && expected_revision.is_none() {
        return Err(invalid(
            "expected_revision",
            "is required to apply; preview with dry_run: true and pass the revision it returns",
        ));
    }
    Ok(Request {
        project,
        symbol_file,
        symbol_name,
        footprint_file,
        model_file,
        library,
        fields,
        dry_run,
        expected_revision,
    })
}

// ─── Plan ───────────────────────────────────────────────────────────────────

/// One file the import writes. `expected` is `None` for a new file.
#[derive(Debug, Clone)]
struct PlannedWrite {
    path: PathBuf,
    kind: &'static str,
    expected: Option<String>,
    replacement: String,
}

impl PlannedWrite {
    fn action(&self) -> &'static str {
        match (self.kind, self.expected.is_some()) {
            ("symbol_library", true) => "append_symbol",
            ("symbol_lib_table" | "footprint_lib_table", true) => "register",
            (_, false) => "create",
            (_, true) => "update",
        }
    }
}

#[derive(Debug)]
struct Plan {
    project_dir: PathBuf,
    library: String,
    symbol_library: PathBuf,
    footprint_library: PathBuf,
    model_dir: PathBuf,
    symbol_name: String,
    footprint_name: String,
    writes: Vec<PlannedWrite>,
    /// Model file already at its target with the same bytes: referenced, not written.
    model_reused: Option<PathBuf>,
    /// Footprint file already at its target with the same bytes: used, not written.
    footprint_reused: Option<PathBuf>,
    registrations: Value,
    symbol_report: Value,
    footprint_report: Value,
    pin_pad_check: Value,
    blockers: Vec<String>,
    warnings: Vec<String>,
    revision: String,
}

/// An input that cannot be used: reported as an error, nothing planned.
enum PlanError {
    Result(CallToolResult),
    Io(anyhow::Error),
}

impl From<CallToolResult> for PlanError {
    fn from(value: CallToolResult) -> Self {
        Self::Result(value)
    }
}

fn read_source(path: &Path, field: &str, limit: u64) -> Result<String, PlanError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(PlanError::Result(CallToolResult::error_kind(
                ToolErrorKind::FileNotFound {
                    path: path.display().to_string(),
                },
                format!("{field} not found: {}", path.display()),
            )))
        }
        Err(error) => return Err(PlanError::Io(error.into())),
    };
    if !metadata.is_file() {
        return Err(invalid(field, format!("{} is not a file", path.display())).into());
    }
    if metadata.len() > limit {
        return Err(invalid(
            field,
            format!(
                "{} is {} bytes, more than the {limit}-byte limit for one part",
                path.display(),
                metadata.len()
            ),
        )
        .into());
    }
    let bytes = std::fs::read(path).map_err(|error| PlanError::Io(error.into()))?;
    String::from_utf8(bytes).map_err(|_| {
        // The model is copied as text through the transaction. A conforming
        // STEP file is 7-bit ASCII (ISO 10303-21 escapes anything else), so a
        // raw Latin-1 header byte means the exporter broke the format.
        let hint = if field == "model_file" {
            "re-export it as an ISO 10303-21 conforming (ASCII) STEP"
        } else {
            "KiCad library files are text"
        };
        invalid(
            field,
            format!("{} is not UTF-8 text; {hint}", path.display()),
        )
        .into()
    })
}

fn sha256_text(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The project folder: the argument resolved the way the `register_*`
/// tools resolve it, and required to hold a `.kicad_pro`, so a mistyped path
/// never grows a stray library.
fn project_dir(project: &str) -> Result<PathBuf, CallToolResult> {
    let dir = project_table_dir(project)?;
    let holds_project = std::fs::read_dir(&dir)
        .map(|entries| {
            entries.flatten().any(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "kicad_pro")
            })
        })
        .unwrap_or(false);
    if !holds_project {
        return Err(invalid(
            "project",
            format!("{} holds no .kicad_pro project file", dir.display()),
        ));
    }
    // Absolute from here on: the transaction canonicalises its root and joins
    // a relative target onto it again, so a folder relative to the server's
    // working directory would send every write to <root>/<relative>/….
    std::fs::canonicalize(&dir).map_err(|error| {
        invalid(
            "project",
            format!("{} cannot be resolved: {error}", dir.display()),
        )
    })
}

/// `path` with `.` dropped and `..` applied, without reading the disk. Two
/// spellings that agree here name the same file unless a link sits between
/// them, which can only make a duplicate row look like one: that blocks.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Direct `(symbol …)` children of a symbol library, with byte spans.
fn top_level_symbols(source: &str) -> Vec<(String, usize, usize)> {
    find_direct_child_blocks(source, "kicad_symbol_lib")
        .into_iter()
        .filter_map(|(start, end)| {
            let node = parse_sexp(&source[start..end]).ok()?;
            (node.head() == Some("symbol"))
                .then(|| node.get(1).and_then(SexpNode::as_str).map(str::to_string))
                .flatten()
                .map(|name| (name, start, end))
        })
        .collect()
}

fn version_of(root: &SexpNode) -> Option<u64> {
    root.find_str("version")?.parse().ok()
}

/// A one-symbol library around `block`, so the shared field editor and
/// parser can work on the symbol alone.
fn wrap_symbol(block: &str, version: u64) -> String {
    format!(
        "(kicad_symbol_lib\n  (version {version})\n  (generator \"konnect\")\n  (generator_version \"10.0\")\n  {block}\n)\n"
    )
}

fn pin_report(symbol: &SexpNode) -> (usize, Vec<Value>) {
    let pins = descendants_with_head(symbol, "pin");
    let listed = pins
        .iter()
        .take(MAX_LISTED)
        .map(|pin| {
            json!({
                "number": pin.find_str("number").unwrap_or_default(),
                "name": pin.find_str("name").unwrap_or_default(),
                "type": pin.get(1).and_then(SexpNode::as_str).unwrap_or_default(),
            })
        })
        .collect();
    (pins.len(), listed)
}

fn pad_report(footprint: &SexpNode) -> (usize, Vec<Value>) {
    let pads = footprint.find_all("pad");
    let listed = pads
        .iter()
        .take(MAX_LISTED)
        .map(|pad| {
            let at = pad.find("at");
            let size = pad.find("size");
            let mut entry = json!({
                "number": pad.get(1).and_then(SexpNode::as_str).unwrap_or_default(),
                "type": pad.get(2).and_then(SexpNode::as_str).unwrap_or_default(),
                "shape": pad.get(3).and_then(SexpNode::as_str).unwrap_or_default(),
                "x": at.and_then(|n| n.get_f64(1)),
                "y": at.and_then(|n| n.get_f64(2)),
                "width": size.and_then(|n| n.get_f64(1)),
                "height": size.and_then(|n| n.get_f64(2)),
            });
            if let Some(rotation) = at.and_then(|n| n.get_f64(3)) {
                entry["rotation"] = json!(rotation);
            }
            if let Some(drill) = pad.find("drill") {
                let diameter = drill
                    .children()
                    .unwrap_or(&[])
                    .iter()
                    .skip(1)
                    .find_map(|child| child.as_str()?.parse::<f64>().ok());
                entry["drill"] = json!(diameter);
            }
            entry
        })
        .collect();
    (pads.len(), listed)
}

/// KiCad 9+ addresses a model stored inside the footprint as
/// `kicad-embed://<name>`, with the data in `(embedded_files (file …))`.
const EMBED_PREFIX: &str = "kicad-embed://";

/// Names of the footprint's embedded files of type `model`.
fn embedded_model_names(footprint: &SexpNode) -> Vec<String> {
    footprint
        .find("embedded_files")
        .map(|files| {
            files
                .find_all("file")
                .into_iter()
                .filter(|file| file.find_str("type") == Some("model"))
                .filter_map(|file| file.find_str("name").map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Where an existing `(model …)` reference points, for the report.
fn model_reference_state(
    path: &str,
    project_dir: &Path,
    footprint_file: &Path,
    footprint: &SexpNode,
) -> &'static str {
    // An embedded model travels inside the footprint text, which is copied
    // as it is.
    if let Some(name) = path.strip_prefix(EMBED_PREFIX) {
        return if embedded_model_names(footprint).iter().any(|n| n == name) {
            "embedded"
        } else {
            "embedded_file_absent"
        };
    }
    if let Some(rest) = path.strip_prefix("${KIPRJMOD}") {
        let rest = rest.trim_start_matches(['/', '\\']);
        return if project_dir.join(rest).is_file() {
            "found"
        } else {
            "missing"
        };
    }
    if path.starts_with("${") {
        return "uses_a_path_variable";
    }
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return if candidate.is_file() {
            "found"
        } else {
            "missing"
        };
    }
    // KiCad resolves a relative model path against the project, not the
    // footprint file, so a path that only resolves next to the source file
    // will break once the footprint is in the project library.
    if project_dir.join(candidate).is_file() {
        "found"
    } else if footprint_file
        .parent()
        .is_some_and(|parent| parent.join(candidate).is_file())
    {
        "relative_to_the_source_file_only"
    } else {
        "missing"
    }
}

fn xyz_of(model: &SexpNode, tag: &str, default: [f64; 3]) -> [f64; 3] {
    let Some(xyz) = model.find(tag).and_then(|node| node.find("xyz")) else {
        return default;
    };
    match (xyz.get_f64(1), xyz.get_f64(2), xyz.get_f64(3)) {
        (Some(x), Some(y), Some(z)) if x.is_finite() && y.is_finite() && z.is_finite() => [x, y, z],
        _ => default,
    }
}

fn indent_before(source: &str, offset: usize) -> String {
    let line_start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
    let indent = &source[line_start..offset];
    if indent.chars().all(|c| c == ' ' || c == '\t') {
        indent.to_string()
    } else {
        "  ".to_string()
    }
}

/// Replace every direct `(model …)` child with one reference to `model_uri`,
/// keeping the first existing model's transform (vendor exports align their
/// model with it), visibility and opacity. Embedded model files that only the
/// replaced references used are dropped with them.
fn replace_models(source: &str, root: &SexpNode, model_uri: &str) -> anyhow::Result<String> {
    let first = root.find("model");
    let offset = first.map_or([0.0; 3], |m| xyz_of(m, "offset", [0.0; 3]));
    let scale = first.map_or([1.0; 3], |m| xyz_of(m, "scale", [1.0; 3]));
    let rotate = first.map_or([0.0; 3], |m| xyz_of(m, "rotate", [0.0; 3]));
    let xyz = |v: [f64; 3]| format!("{} {} {}", fmt_f64(v[0]), fmt_f64(v[1]), fmt_f64(v[2]));
    // KiCad reads a bare `hide` (older files), `(hide)`, `(hide yes)` and
    // `(hide true)` alike, and writes `(hide yes)` then `(opacity …)` before
    // the transform.
    let hidden = first.is_some_and(|m| {
        m.children().unwrap_or(&[]).iter().skip(2).any(|child| {
            child.as_str() == Some("hide")
                || (child.head() == Some("hide")
                    && matches!(
                        child.get(1).and_then(SexpNode::as_str),
                        None | Some("yes" | "true")
                    ))
        })
    });
    let opacity = first
        .and_then(|m| m.find_f64("opacity"))
        .filter(|o| o.is_finite() && *o != 1.0);

    let children = find_direct_child_blocks(source, "footprint");
    let models: Vec<(usize, usize)> = children
        .iter()
        .copied()
        .filter(|(start, end)| {
            parse_sexp(&source[*start..*end]).is_ok_and(|node| node.head() == Some("model"))
        })
        .collect();
    let indent = models.first().or(children.first()).map_or_else(
        || "  ".to_string(),
        |(start, _)| indent_before(source, *start),
    );
    let mut appearance = String::new();
    if hidden {
        appearance.push_str(&format!("\n{indent}  (hide yes)"));
    }
    if let Some(opacity) = opacity {
        appearance.push_str(&format!("\n{indent}  (opacity {})", fmt_f64(opacity)));
    }
    let block = format!(
        "(model {}{appearance}\n{indent}  (offset (xyz {}))\n{indent}  (scale (xyz {}))\n{indent}  (rotate (xyz {}))\n{indent})",
        quote_lib_table_string(model_uri),
        xyz(offset),
        xyz(scale),
        xyz(rotate)
    );
    let mut edits = Vec::new();
    if let Some(((first_start, first_end), rest)) = models.split_first() {
        edits.push(SexpEdit::replace(*first_start, *first_end, block));
        for (start, _) in rest {
            let (a, b) = konnect_sexp::writer::find_block_with_leading_whitespace(source, *start)
                .ok_or_else(|| anyhow::anyhow!("unbalanced model block"))?;
            edits.push(SexpEdit::delete(a, b));
        }
    } else {
        let root_end = find_balanced_block(source, 0)
            .map(|range| range.1 - 1)
            .ok_or_else(|| anyhow::anyhow!("unbalanced footprint root"))?;
        let lead = if source[..root_end].ends_with('\n') {
            ""
        } else {
            "\n"
        };
        edits.push(SexpEdit::insert(
            root_end,
            format!("{lead}{indent}{block}\n"),
        ));
    }

    // An embedded model a replaced reference named is dead data once the
    // reference is gone; the whole `(embedded_files …)` goes when it empties.
    let replaced_embeds: Vec<&str> = root
        .find_all("model")
        .into_iter()
        .filter_map(|m| m.get(1)?.as_str()?.strip_prefix(EMBED_PREFIX))
        .collect();
    let embedded = children.iter().copied().find(|(start, end)| {
        parse_sexp(&source[*start..*end]).is_ok_and(|node| node.head() == Some("embedded_files"))
    });
    if let (false, Some((files_start, files_end))) = (replaced_embeds.is_empty(), embedded) {
        let files: Vec<(usize, usize)> =
            find_direct_child_blocks(&source[files_start..files_end], "embedded_files")
                .into_iter()
                .map(|(start, end)| (files_start + start, files_start + end))
                .collect();
        let dropped: Vec<(usize, usize)> = files
            .iter()
            .copied()
            .filter(|(start, end)| {
                parse_sexp(&source[*start..*end]).is_ok_and(|file| {
                    file.head() == Some("file")
                        && file.find_str("type") == Some("model")
                        && file
                            .find_str("name")
                            .is_some_and(|name| replaced_embeds.contains(&name))
                })
            })
            .collect();
        let starts = if !dropped.is_empty() && dropped.len() == files.len() {
            vec![files_start]
        } else {
            dropped.iter().map(|(start, _)| *start).collect()
        };
        for start in starts {
            let (a, b) = konnect_sexp::writer::find_block_with_leading_whitespace(source, start)
                .ok_or_else(|| anyhow::anyhow!("unbalanced embedded file block"))?;
            edits.push(SexpEdit::delete(a, b));
        }
    }
    let replaced = apply_edits(source.to_string(), edits);
    let parsed = parse_sexp(&replaced)?;
    anyhow::ensure!(
        parsed.head() == Some("footprint") && parsed.find_all("model").len() == 1,
        "model replacement did not leave exactly one model"
    );
    Ok(replaced)
}

/// The `(disabled)` and `(hidden)` flags of the table row named `nickname`.
fn row_flags(source: &str, nickname: &str) -> (bool, bool) {
    find_block_starts(source, "lib")
        .into_iter()
        .filter_map(|start| find_balanced_block(source, start))
        .filter_map(|(start, end)| parse_sexp(&source[start..end]).ok())
        .find(|row| row.find_str("name") == Some(nickname))
        .map_or((false, false), |row| {
            (row.find("disabled").is_some(), row.find("hidden").is_some())
        })
}

/// The project lib-table write that registers `nickname` at `uri`, or the
/// reason it is blocked.
#[allow(clippy::too_many_arguments)]
fn plan_registration(
    table_path: &Path,
    lib_path: &Path,
    project_dir: &Path,
    nickname: &str,
    uri: &str,
    kind: &'static str,
    blockers: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Result<(Option<PlannedWrite>, &'static str), PlanError> {
    // The transaction refuses a symlinked or non-file target; say so here
    // rather than preview a write that apply cannot make.
    if let Some(blocker) = target_type_blocker(table_path, false) {
        blockers.push(blocker);
        return Ok((None, "blocked"));
    }
    let root = table_root_element(table_path);
    let existing = match std::fs::read_to_string(table_path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(PlanError::Io(error.into())),
    };
    let source = existing
        .clone()
        .unwrap_or_else(|| format!("({root}\n  (version 7)\n)\n"));
    let entries = parse_lib_table(&source);
    let bound: Vec<String> = entries
        .iter()
        .filter(|entry| entry["nickname"].as_str() == Some(nickname))
        .filter_map(|entry| entry["uri"].as_str().map(str::to_string))
        .collect();
    // Same library: the same URI text, or a URI that resolves to the same
    // file (canonicalising also folds case on a case-insensitive disk).
    // Before the file exists nothing resolves, so a `${KIPRJMOD}` or absolute
    // URI is compared as spelled, with `.` and `..` applied:
    // `${KIPRJMOD}/./x.kicad_sym` still names `x.kicad_sym`.
    let target = std::fs::canonicalize(lib_path).ok();
    let lexical_target = lexical(lib_path);
    let spelled = |bound_uri: &str| match bound_uri.strip_prefix("${KIPRJMOD}") {
        Some(tail) => Some(project_dir.join(tail.trim_start_matches(['/', '\\']))),
        None => Path::new(bound_uri)
            .is_absolute()
            .then(|| PathBuf::from(bound_uri)),
    };
    let same_library = |bound_uri: &str| {
        bound_uri == uri
            || (target.is_some()
                && expand_lib_uri(bound_uri, Some(project_dir))
                    .and_then(|path| std::fs::canonicalize(path).ok())
                    == target)
            || (target.is_none()
                && spelled(bound_uri).is_some_and(|path| lexical(&path) == lexical_target))
    };
    if let Some(other) = bound.iter().find(|bound_uri| !same_library(bound_uri)) {
        blockers.push(format!(
            "nickname_taken: {} already binds '{nickname}' to {other}; pass another library name",
            table_path.display()
        ));
        return Ok((None, "blocked"));
    }
    // A new row for a library another row already registers would make
    // KiCad list it twice.
    if bound.is_empty() {
        if let Some(alias) = entries.iter().find(|entry| {
            entry["nickname"].as_str() != Some(nickname)
                && entry["uri"].as_str().is_some_and(same_library)
        }) {
            blockers.push(format!(
                "library_already_registered: {} already registers {} as '{}'; a second row would \
                 list it twice, so pass another library name",
                table_path.display(),
                lib_path.display(),
                alias["nickname"].as_str().unwrap_or_default()
            ));
            return Ok((None, "blocked"));
        }
    }
    let (disabled, hidden) = row_flags(&source, nickname);
    if disabled {
        blockers.push(format!(
            "library_disabled: {} has '{nickname}' disabled, so KiCad does not load it; enable it \
             in KiCad's Manage Libraries or pass another library name",
            table_path.display()
        ));
        return Ok((None, "blocked"));
    }
    if hidden {
        warnings.push(format!(
            "{} marks '{nickname}' hidden: KiCad loads it but its chooser does not list it",
            table_path.display()
        ));
    }
    match prepare_lib_table_registration(&source, root, nickname, uri, "KiCad", false) {
        Ok(prepared) if prepared.content == source && existing.is_some() => Ok((None, "unchanged")),
        Ok(prepared) => Ok((
            Some(PlannedWrite {
                path: table_path.to_path_buf(),
                kind,
                expected: existing,
                replacement: prepared.content,
            }),
            "inserted",
        )),
        Err(error) => {
            blockers.push(format!(
                "lib_table_unusable: {}: {}",
                table_path.display(),
                error.reason
            ));
            Ok((None, "blocked"))
        }
    }
}

/// A blocker when `path` exists as a symlink or as the wrong kind of entry.
/// The transaction refuses both, so the preview must not plan them.
fn target_type_blocker(path: &Path, want_dir: bool) -> Option<String> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() {
        Some(format!("target_is_a_symlink: {}", path.display()))
    } else if metadata.is_dir() != want_dir {
        Some(format!(
            "target_has_wrong_type: {} is {}",
            path.display(),
            if metadata.is_dir() {
                "a folder"
            } else {
                "a file"
            }
        ))
    } else {
        None
    }
}

/// A blocker when the folder holds `path` only under another case. A
/// case-insensitive disk would use that entry while the tables name this
/// spelling, which a case-sensitive clone cannot resolve.
fn case_blocker(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let parent = path.parent()?;
    let mut variant = None;
    for entry in std::fs::read_dir(parent).ok()?.flatten() {
        let other = entry.file_name();
        let Some(other) = other.to_str() else {
            continue;
        };
        if other == name {
            return None;
        }
        if other.to_lowercase() == name.to_lowercase() {
            variant = Some(other.to_string());
        }
    }
    variant.map(|other| {
        format!(
            "target_case_mismatch: {} holds '{other}', which differs from '{name}' only in case; \
             rename the source file or the library to match the existing spelling",
            parent.display()
        )
    })
}

/// Records why `path` cannot be a target and says whether it can.
fn usable_target(path: &Path, want_dir: bool, blockers: &mut Vec<String>) -> bool {
    match target_type_blocker(path, want_dir).or_else(|| case_blocker(path)) {
        Some(blocker) => {
            blockers.push(blocker);
            false
        }
        None => true,
    }
}

fn global_nickname_in_use(table: &Path, nickname: &str) -> bool {
    read_flat_lib_table(table)
        .iter()
        .any(|entry| entry["nickname"].as_str() == Some(nickname))
}

fn build_plan(request: &Request) -> Result<Plan, PlanError> {
    let project_dir = project_dir(&request.project)?;
    let library = request.library.clone();
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();

    // ── Symbol source ──
    let symbol_source = read_source(&request.symbol_file, "symbol_file", MAX_SOURCE_BYTES)?;
    let symbol_root = correctness::library(&symbol_source).map_err(|error| {
        invalid(
            "symbol_file",
            format!("is not a KiCad 6+ symbol library: {error}"),
        )
    })?;
    let symbols = top_level_symbols(&symbol_source);
    let (symbol_name, start, end) = match &request.symbol_name {
        Some(wanted) => symbols
            .iter()
            .find(|(name, _, _)| name == wanted)
            .cloned()
            .ok_or_else(|| {
                invalid(
                    "symbol_name",
                    format!(
                        "'{wanted}' is not in {}; it holds: {}",
                        request.symbol_file.display(),
                        list_names(&symbols)
                    ),
                )
            })?,
        None if symbols.len() == 1 => symbols[0].clone(),
        None if symbols.is_empty() => {
            return Err(invalid("symbol_file", "holds no symbols").into());
        }
        None => {
            return Err(invalid(
                "symbol_name",
                format!(
                    "is required: {} holds {} symbols: {}",
                    request.symbol_file.display(),
                    symbols.len(),
                    list_names(&symbols)
                ),
            )
            .into())
        }
    };
    if symbol_name.contains(':') || symbol_name.chars().any(char::is_control) {
        return Err(invalid(
            "symbol_name",
            format!(
                "'{symbol_name}' contains ':' or a control character, which a lib_id cannot carry"
            ),
        )
        .into());
    }
    let source_symbol =
        parse_sexp(&symbol_source[start..end]).map_err(|error| PlanError::Io(error.into()))?;
    if source_symbol.find("extends").is_some() {
        return Err(invalid(
            "symbol_name",
            format!(
                "'{symbol_name}' is derived (extends another symbol); flatten it in KiCad's symbol \
                 editor (Export Symbol) and import that file"
            ),
        )
        .into());
    }
    let source_version = version_of(&symbol_root).unwrap_or(20211014);

    // ── Footprint source ──
    let footprint_source =
        read_source(&request.footprint_file, "footprint_file", MAX_SOURCE_BYTES)?;
    let footprint_root = parse_sexp(&footprint_source).map_err(|error| {
        invalid(
            "footprint_file",
            format!("is not an S-expression footprint: {error}"),
        )
    })?;
    if footprint_root.head() != Some("footprint") {
        return Err(invalid(
            "footprint_file",
            crate::tools::footprint_root_reason(footprint_root.head()),
        )
        .into());
    }
    let footprint_name = footprint_root
        .get(1)
        .and_then(SexpNode::as_str)
        .unwrap_or_default()
        .to_string();
    if let Err(reason) = safe_file_name(&footprint_name) {
        return Err(invalid(
            "footprint_file",
            format!("footprint name '{footprint_name}' {reason}"),
        )
        .into());
    }

    // ── Targets ──
    let symbol_library = project_dir.join(format!("{library}.kicad_sym"));
    let footprint_library = project_dir.join(format!("{library}.pretty"));
    let footprint_path = footprint_library.join(format!("{footprint_name}.kicad_mod"));
    let model_dir = project_dir.join(format!("{library}.3dshapes"));
    let mut symbol_library_usable = usable_target(&symbol_library, false, &mut blockers);
    let footprint_library_usable = usable_target(&footprint_library, true, &mut blockers);
    let model_dir_usable = usable_target(&model_dir, true, &mut blockers);
    let mut writes = Vec::new();

    // ── Model ──
    let mut model_reused = None;
    let (footprint_text, model_report) = if let Some(model_file) = &request.model_file {
        let extension = model_file
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if !matches!(extension.as_deref(), Some("step" | "stp" | "wrl")) {
            return Err(invalid("model_file", "must be a .step, .stp or .wrl file").into());
        }
        let model_name = model_file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if let Err(reason) = safe_file_name(&model_name) {
            return Err(invalid("model_file", format!("file name {reason}")).into());
        }
        let model_text = read_source(model_file, "model_file", MAX_MODEL_BYTES)?;
        // A leading UTF-8 byte-order mark is not whitespace to `trim_start`.
        // The sniff skips it; the bytes are copied and compared as they are.
        let head = model_text
            .strip_prefix('\u{feff}')
            .unwrap_or(&model_text)
            .trim_start();
        let looks_right = if extension.as_deref() == Some("wrl") {
            head.starts_with("#VRML")
        } else {
            head.starts_with("ISO-10303-21")
        };
        if !looks_right {
            return Err(invalid(
                "model_file",
                "does not start like a STEP (ISO-10303-21) or VRML (#VRML) file",
            )
            .into());
        }
        let model_path = model_dir.join(&model_name);
        let model_action = if !model_dir_usable || !usable_target(&model_path, false, &mut blockers)
        {
            "blocked"
        } else {
            match std::fs::read(&model_path) {
                Ok(existing) if existing == model_text.as_bytes() => {
                    model_reused = Some(model_path.clone());
                    "reuse_identical"
                }
                Ok(_) => {
                    blockers.push(format!(
                        "model_exists: {} holds a different file; rename the model or remove it",
                        model_path.display()
                    ));
                    "blocked"
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    writes.push(PlannedWrite {
                        path: model_path.clone(),
                        kind: "model",
                        expected: None,
                        replacement: model_text,
                    });
                    "create"
                }
                Err(error) => return Err(PlanError::Io(error.into())),
            }
        };
        let model_uri = format!("${{KIPRJMOD}}/{library}.3dshapes/{model_name}");
        let replaced = replace_models(&footprint_source, &footprint_root, &model_uri)
            .map_err(|error| invalid("footprint_file", format!("cannot set its model: {error}")))?;
        let replaced_count = footprint_root.find_all("model").len();
        let kept_embeds = parse_sexp(&replaced)
            .map(|node| embedded_model_names(&node))
            .unwrap_or_default();
        let removed_embeds: Vec<String> = embedded_model_names(&footprint_root)
            .into_iter()
            .filter(|name| !kept_embeds.contains(name))
            .collect();
        let mut report = json!({
            "path": model_path.display().to_string(),
            "reference": model_uri,
            "action": model_action,
            "replaced_references": replaced_count,
        });
        if !removed_embeds.is_empty() {
            report["embedded_models_removed"] = json!(removed_embeds);
        }
        (replaced, report)
    } else {
        let references: Vec<Value> = footprint_root
            .find_all("model")
            .iter()
            .filter_map(|model| model.get(1).and_then(SexpNode::as_str))
            .map(|path| {
                let state = model_reference_state(
                    path,
                    &project_dir,
                    &request.footprint_file,
                    &footprint_root,
                );
                if !matches!(state, "found" | "embedded") {
                    warnings.push(format!(
                        "The footprint's 3D model reference '{path}' is {state}; pass model_file to copy the model into the library"
                    ));
                }
                json!({ "reference": path, "state": state })
            })
            .collect();
        (
            footprint_source.clone(),
            json!({ "kept_references": references }),
        )
    };
    let footprint_candidate =
        parse_sexp(&footprint_text).map_err(|error| PlanError::Io(error.into()))?;

    // An identical footprint (a second part sharing it) is used as it is;
    // a different one under this name is never overwritten.
    let mut footprint_reused = None;
    let footprint_action =
        if !footprint_library_usable || !usable_target(&footprint_path, false, &mut blockers) {
            "blocked"
        } else {
            match std::fs::read(&footprint_path) {
                Ok(existing) if existing == footprint_text.as_bytes() => {
                    footprint_reused = Some(footprint_path.clone());
                    "reuse_identical"
                }
                Ok(_) => {
                    blockers.push(format!(
                        "footprint_exists: {} holds a different footprint with this name",
                        footprint_path.display()
                    ));
                    "blocked"
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    writes.push(PlannedWrite {
                        path: footprint_path.clone(),
                        kind: "footprint",
                        expected: None,
                        replacement: footprint_text,
                    });
                    "create"
                }
                Err(error) => return Err(PlanError::Io(error.into())),
            }
        };

    // ── Symbol candidate ──
    let mut fields = request.fields.clone();
    let footprint_field_before = properties_of(&source_symbol).get("Footprint").cloned();
    fields.insert(
        "Footprint".to_string(),
        format!("{library}:{footprint_name}"),
    );
    let wrapped = wrap_symbol(&symbol_source[start..end], source_version);
    let edited = correctness::prepare(&wrapped, &symbol_name, &fields).map_err(|error| {
        invalid(
            "symbol_file",
            format!("cannot set the symbol fields: {error}"),
        )
    })?;
    let (_, block_start, block_end) = top_level_symbols(&edited)
        .into_iter()
        .find(|(name, _, _)| name == &symbol_name)
        .ok_or_else(|| PlanError::Io(anyhow::anyhow!("edited symbol vanished")))?;
    let block = edited[block_start..block_end].to_string();

    let existing_library = if symbol_library_usable {
        match std::fs::read_to_string(&symbol_library) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                blockers.push(format!(
                    "symbol_library_unusable: {}: not UTF-8 text",
                    symbol_library.display()
                ));
                symbol_library_usable = false;
                None
            }
            Err(error) => return Err(PlanError::Io(error.into())),
        }
    } else {
        None
    };
    let library_text = match &existing_library {
        _ if !symbol_library_usable => None,
        Some(existing) => match correctness::library(existing) {
            Err(error) => {
                blockers.push(format!(
                    "symbol_library_unusable: {}: {error}",
                    symbol_library.display()
                ));
                None
            }
            Ok(existing_root) => {
                if top_level_symbols(existing)
                    .iter()
                    .any(|(name, _, _)| name == &symbol_name)
                {
                    blockers.push(format!(
                        "symbol_exists: {} already holds '{symbol_name}'",
                        symbol_library.display()
                    ));
                    None
                } else {
                    Some(append_symbol(existing, &existing_root, &block, source_version, &mut warnings)?)
                }
            }
        },
        None => Some(format!(
            "(kicad_symbol_lib\n  (version {source_version})\n  (generator \"konnect\")\n  (generator_version \"10.0\")\n  {block}\n)\n"
        )),
    };
    let symbol_candidate = match &library_text {
        Some(text) => {
            let root = correctness::library(text).map_err(|error| {
                PlanError::Io(anyhow::anyhow!("candidate library does not parse: {error}"))
            })?;
            writes.push(PlannedWrite {
                path: symbol_library.clone(),
                kind: "symbol_library",
                expected: existing_library.clone(),
                replacement: text.clone(),
            });
            Some(root)
        }
        None => None,
    };
    // Report the symbol as it will be saved (with the new fields); when the
    // target library blocks, the edited one-symbol library stands in.
    let report_root = match symbol_candidate {
        Some(root) => root,
        None => correctness::library(&edited).map_err(PlanError::Io)?,
    };
    let saved_symbol = report_root
        .find_all("symbol")
        .into_iter()
        .find(|node| node.get(1).and_then(SexpNode::as_str) == Some(symbol_name.as_str()))
        .ok_or_else(|| PlanError::Io(anyhow::anyhow!("candidate symbol missing")))?;
    let (pin_count, pins) = pin_report(saved_symbol);
    let (pad_count, pads) = pad_report(&footprint_candidate);
    let mut pin_pad_check =
        correctness::pin_pad_check(&report_root, &symbol_name, &footprint_candidate);
    pin_pad_check["source"] = json!("candidate");
    if pin_pad_check["result"] != "pass" {
        warnings.push(format!(
            "Pin-pad check: {}. Compare with the part's datasheet before using it; \
             mounting or exposed pads without a pin are common and may be fine",
            pin_pad_check["result"].as_str().unwrap_or("incomplete")
        ));
    }

    // ── Library tables ──
    let symbol_uri = project_relative_uri(&symbol_library, &project_dir);
    let footprint_uri = project_relative_uri(&footprint_library, &project_dir);
    let (symbol_table_write, symbol_registration) = plan_registration(
        &project_dir.join("sym-lib-table"),
        &symbol_library,
        &project_dir,
        &library,
        &symbol_uri,
        "symbol_lib_table",
        &mut blockers,
        &mut warnings,
    )?;
    let (footprint_table_write, footprint_registration) = plan_registration(
        &project_dir.join("fp-lib-table"),
        &footprint_library,
        &project_dir,
        &library,
        &footprint_uri,
        "footprint_lib_table",
        &mut blockers,
        &mut warnings,
    )?;
    writes.extend(symbol_table_write);
    writes.extend(footprint_table_write);
    if global_nickname_in_use(&global_sym_lib_table(), &library)
        || global_nickname_in_use(&global_fp_lib_table(), &library)
    {
        warnings.push(format!(
            "The global library table also has a library named '{library}'; KiCad uses the project's entry for this project"
        ));
    }

    let symbol_report = json!({
        "name": symbol_name,
        "lib_id": format!("{library}:{symbol_name}"),
        "source": request.symbol_file.display().to_string(),
        "fields_set": fields,
        "footprint_field_before": footprint_field_before,
        "pin_count": pin_count,
        "pins": pins,
        "pins_listed": pin_count.min(MAX_LISTED),
    });
    let footprint_report = json!({
        "name": footprint_name,
        "lib_id": format!("{library}:{footprint_name}"),
        "source": request.footprint_file.display().to_string(),
        "path": footprint_path.display().to_string(),
        "pad_count": pad_count,
        "pads": pads,
        "pads_listed": pad_count.min(MAX_LISTED),
        "action": footprint_action,
        "model": model_report,
    });
    let registrations = json!({
        "symbol": { "table": project_dir.join("sym-lib-table").display().to_string(), "uri": symbol_uri, "state": symbol_registration },
        "footprint": { "table": project_dir.join("fp-lib-table").display().to_string(), "uri": footprint_uri, "state": footprint_registration },
    });

    let reused: Vec<&Path> = model_reused
        .iter()
        .chain(&footprint_reused)
        .map(PathBuf::as_path)
        .collect();
    let revision = plan_revision(&writes, &reused, &blockers);
    Ok(Plan {
        project_dir,
        library,
        symbol_library,
        footprint_library,
        model_dir,
        symbol_name,
        footprint_name,
        writes,
        model_reused,
        footprint_reused,
        registrations,
        symbol_report,
        footprint_report,
        pin_pad_check,
        blockers,
        warnings,
        revision,
    })
}

fn list_names(symbols: &[(String, usize, usize)]) -> String {
    let mut names: Vec<&str> = symbols
        .iter()
        .map(|(name, _, _)| name.as_str())
        .take(20)
        .collect();
    if symbols.len() > 20 {
        names.push("…");
    }
    names.join(", ")
}

fn properties_of(symbol: &SexpNode) -> BTreeMap<String, String> {
    symbol
        .find_all("property")
        .into_iter()
        .filter_map(|p| {
            Some((
                p.get(1)?.as_str()?.to_string(),
                p.get(2)?.as_str()?.to_string(),
            ))
        })
        .collect()
}

/// `existing` with `block` inserted before the root's closing paren. A newer
/// source format raises the library's `(version …)`: KiCad reads older
/// syntax under a newer version, never the reverse.
fn append_symbol(
    existing: &str,
    existing_root: &SexpNode,
    block: &str,
    source_version: u64,
    warnings: &mut Vec<String>,
) -> Result<String, PlanError> {
    let root_end = find_balanced_block(existing, 0)
        .map(|range| range.1 - 1)
        .ok_or_else(|| PlanError::Io(anyhow::anyhow!("unbalanced symbol library root")))?;
    let lead = if existing[..root_end].ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let mut edits = vec![SexpEdit::insert(root_end, format!("{lead}  {block}\n"))];
    if let Some(target_version) = version_of(existing_root) {
        if source_version > target_version {
            if let Some((start, end)) = find_direct_child_blocks(existing, "kicad_symbol_lib")
                .into_iter()
                .find(|(a, b)| {
                    parse_sexp(&existing[*a..*b]).is_ok_and(|node| node.head() == Some("version"))
                })
            {
                edits.push(SexpEdit::replace(
                    start,
                    end,
                    format!("(version {source_version})"),
                ));
                warnings.push(format!(
                    "The symbol library's format version rises from {target_version} to {source_version} to match the imported symbol"
                ));
            }
        }
    }
    Ok(apply_edits(existing.to_string(), edits))
}

/// Binds the exact transitions, the reused model and footprint, and the
/// blockers, so any change on disk or in the request yields a different
/// revision.
fn plan_revision(writes: &[PlannedWrite], reused: &[&Path], blockers: &[String]) -> String {
    let mut hasher = Sha256::new();
    for write in writes {
        hasher.update(write.path.display().to_string().as_bytes());
        hasher.update([0]);
        hasher.update(
            write
                .expected
                .as_deref()
                .map_or_else(|| "absent".to_string(), sha256_text)
                .as_bytes(),
        );
        hasher.update([0]);
        hasher.update(sha256_text(&write.replacement).as_bytes());
        hasher.update([0]);
    }
    for path in reused {
        hasher.update(b"reuse:");
        hasher.update(path.display().to_string().as_bytes());
        hasher.update([0]);
    }
    for blocker in blockers {
        hasher.update(b"blocked:");
        hasher.update(blocker.as_bytes());
    }
    let digest = hasher.finalize();
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

fn plan_body(plan: &Plan, dry_run: bool) -> Value {
    let writes: Vec<Value> = plan
        .writes
        .iter()
        .map(|write| {
            json!({
                "path": write.path.display().to_string(),
                "kind": write.kind,
                "action": write.action(),
                "bytes": write.replacement.len(),
            })
        })
        .collect();
    json!({
        "dry_run": dry_run,
        "library": {
            "nickname": plan.library,
            "symbol_library": plan.symbol_library.display().to_string(),
            "footprint_library": plan.footprint_library.display().to_string(),
            "model_dir": plan.model_dir.display().to_string(),
        },
        "writes": writes,
        "registrations": plan.registrations,
        "symbol": plan.symbol_report,
        "footprint": plan.footprint_report,
        "pin_pad_check": plan.pin_pad_check,
        "warnings": plan.warnings,
        "blocked_by": plan.blockers,
        "can_apply": plan.blockers.is_empty(),
        "revision": plan.revision,
        "network": "none: the tool reads only the local files given",
    })
}

// ─── Handler ────────────────────────────────────────────────────────────────

async fn handle_import_library_part(
    args: &Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let request = match parse_request(args) {
        Ok(request) => request,
        Err(error) => return Ok(error),
    };
    tokio::task::spawn_blocking(move || import_blocking(&request)).await?
}

fn import_blocking(request: &Request) -> anyhow::Result<CallToolResult> {
    let plan = match build_plan(request) {
        Ok(plan) => plan,
        Err(PlanError::Result(result)) => return Ok(result),
        Err(PlanError::Io(error)) => return Err(error),
    };
    if request.dry_run {
        return Ok(CallToolResult::json(&plan_body(&plan, true)));
    }
    if request.expected_revision.as_deref() != Some(plan.revision.as_str()) {
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::StaleTarget {
                target: plan.project_dir.display().to_string(),
                reason: "revision_mismatch".to_string(),
            },
            "The inputs, the target library or the library tables changed since the preview \
             (or the request differs). Nothing was written; preview again and apply with the \
             new revision.",
        ));
    }
    if !plan.blockers.is_empty() {
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::PlanBlocked {
                operation: OPERATION.to_string(),
                reasons: plan.blockers.clone(),
            },
            format!(
                "Import refused, nothing written: {}",
                plan.blockers.join("; ")
            ),
        ));
    }

    // The transaction needs every parent folder to exist. Folders created
    // here are removed again when nothing was written into them.
    let mut created_dirs: Vec<PathBuf> = Vec::new();
    for write in &plan.writes {
        if let Some(parent) = write.path.parent() {
            if !parent.exists() {
                if let Err(error) = std::fs::create_dir(parent) {
                    for dir in created_dirs.iter().rev() {
                        let _ = std::fs::remove_dir(dir);
                    }
                    return Ok(CallToolResult::error_kind(
                        ToolErrorKind::PlanBlocked {
                            operation: OPERATION.to_string(),
                            reasons: vec![format!(
                                "cannot_create_folder: {}: {error}",
                                parent.display()
                            )],
                        },
                        format!(
                            "Import refused, nothing written: cannot create {}: {error}",
                            parent.display()
                        ),
                    ));
                }
                created_dirs.push(parent.to_path_buf());
            }
        }
    }
    let transitions: Vec<FileTransition> = plan
        .writes
        .iter()
        .map(|write| match &write.expected {
            Some(expected) => {
                FileTransition::replace(&write.path, expected.clone(), write.replacement.clone())
            }
            None => FileTransition::create(&write.path, write.replacement.clone()),
        })
        .collect();
    if let Err(error) = commit_file_transaction(&plan.project_dir, transitions) {
        let untouched = plan
            .writes
            .iter()
            .all(|write| std::fs::read_to_string(&write.path).ok() == write.expected)
            && !journal_pending(&plan.project_dir);
        if untouched {
            for dir in created_dirs.iter().rev() {
                let _ = std::fs::remove_dir(dir);
            }
        }
        return Ok(match error {
            SexpError::KiCadEditorLocked { path, .. } if untouched => CallToolResult::error_kind(
                ToolErrorKind::PlanBlocked {
                    operation: OPERATION.to_string(),
                    reasons: vec![format!("kicad_lock_present: {}", path.display())],
                },
                format!("KiCad holds {} open; nothing was written", path.display()),
            ),
            SexpError::Conflict { .. } | SexpError::TransactionConflict { .. } if untouched => {
                CallToolResult::error_kind(
                    ToolErrorKind::StaleTarget {
                        target: plan.project_dir.display().to_string(),
                        reason: "revision_mismatch".to_string(),
                    },
                    "A target file changed during the import; nothing was written. Preview again.",
                )
            }
            // Every target still reads as before and no journal is left, so
            // the transaction refused before writing anything.
            other if untouched => CallToolResult::error_kind(
                ToolErrorKind::PlanBlocked {
                    operation: OPERATION.to_string(),
                    reasons: vec![format!("transaction_refused: {other}")],
                },
                format!("Import refused, nothing written: {other}"),
            ),
            other => crate::tools::mutation_outcome_uncertain(
                &plan.project_dir,
                OPERATION,
                format!(
                    "{other}. Some library files may be written; run `konnect transaction status {}`",
                    plan.project_dir.display()
                ),
            ),
        });
    }

    // Read back every write, then check the saved symbol against the saved
    // footprint.
    let mismatched: Vec<String> = plan
        .writes
        .iter()
        .filter(|write| {
            std::fs::read_to_string(&write.path).ok().as_deref() != Some(&write.replacement)
        })
        .map(|write| write.path.display().to_string())
        .collect();
    if !mismatched.is_empty() {
        return Ok(crate::tools::mutation_outcome_uncertain(
            &plan.project_dir,
            OPERATION,
            format!("readback differs for {}", mismatched.join(", ")),
        ));
    }
    let footprint_path = plan
        .footprint_library
        .join(format!("{}.kicad_mod", plan.footprint_name));
    let saved_check = (|| -> anyhow::Result<Value> {
        let root = correctness::library(&read_consistent(&plan.symbol_library)?)?;
        let footprint = parse_sexp(&read_consistent(&footprint_path)?)?;
        Ok(correctness::pin_pad_check(
            &root,
            &plan.symbol_name,
            &footprint,
        ))
    })();
    let saved_check = match saved_check {
        Ok(check) => check,
        Err(error) => {
            return Ok(crate::tools::mutation_outcome_uncertain(
                &plan.project_dir,
                OPERATION,
                format!("saved files do not read back as a library part: {error}"),
            ))
        }
    };

    let mut body = plan_body(&plan, false);
    body["pin_pad_check"] = saved_check;
    body["written"] = json!(plan
        .writes
        .iter()
        .map(|write| write.path.display().to_string())
        .collect::<Vec<_>>());
    if let Some(path) = &plan.model_reused {
        body["model_reused"] = json!(path.display().to_string());
    }
    if let Some(path) = &plan.footprint_reused {
        body["footprint_reused"] = json!(path.display().to_string());
    }
    body["next"] = json!(format!(
        "Place it with add_schematic_component lib_id '{}:{}'. If KiCad has this project open, \
         reopen the project (or the library manager) so it reads the updated library tables.",
        plan.library, plan.symbol_name
    ));
    if let Some(object) = body.as_object_mut() {
        object.remove("can_apply");
    }
    Ok(CallToolResult::json(&body))
}

/// Whether an active transaction journal is left in the project, by the
/// transaction crate's own rule: `.abandoned.json` evidence files do not
/// count, and a project that cannot be inspected counts as one that has one.
fn journal_pending(project_dir: &Path) -> bool {
    !konnect_sexp::inspect_file_transactions(project_dir).is_ok_and(|pending| pending.is_empty())
}

#[cfg(test)]
#[path = "library_part_import_tests.rs"]
mod tests;
