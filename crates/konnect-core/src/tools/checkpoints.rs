//! Restorable project checkpoints and the edit-readiness report (creation PR 14).
//!
//! Tools: create_checkpoint, list_checkpoints, restore_checkpoint, get_edit_readiness
//!
//! A checkpoint is an exact byte copy of a project's saved design files, kept
//! beside the project at `<project_dir>/.konnect/checkpoints/<id>/`:
//!
//! - `files/<relative path>`: one copy per captured file;
//! - `manifest.json`: the file list with a SHA-256 per file, written last.
//!
//! A checkpoint is built in a hidden `.partial-*` folder and renamed into place
//! only after every copy has been read back and the project has been read again
//! and found unchanged, so a listed checkpoint is always complete.
//!
//! Captured: `.kicad_pro`, `.kicad_sch`, `.kicad_pcb`, `.kicad_dru`,
//! `.kicad_sym`, `.kicad_mod`, `sym-lib-table` and `fp-lib-table`, found
//! recursively. Not captured: hidden folders (`.git`, `.konnect`, KiCad 10's
//! `.history`), `*-backups`, KiCad lock and autosave files, `.kicad_prl`
//! (per-user view state) and symlinks, which are reported as skipped.
//!
//! Restore previews by default. Applying needs the preview's revision, is
//! refused while KiCad owns a target or has the project open, writes through
//! the durable multi-file transaction every Konnect writer shares, takes an
//! automatic `pre_restore` checkpoint first (so a restore can be undone), and
//! reads every restored file back. Files created after the checkpoint are
//! listed and left in place: restore never deletes.
//!
//! The readiness report says, per design file, whether Konnect can edit the
//! saved file, must edit live through KiCad, or is blocked, and why. KiCad lock
//! files carry no PID, so a stale lock cannot be told from a live one; Konnect
//! never clears them.

use crate::mcp::{error::ToolErrorKind, protocol::CallToolResult};
use crate::tool;
use crate::tools::live_board::{editor_lock, EditorLock};
use crate::tools::{invalid_arg, opt_str, require_str, ToolContext, ToolDef};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STORE_PARENT: &str = ".konnect";
const STORE_DIR: &str = "checkpoints";
const MANIFEST_FILE: &str = "manifest.json";
const FILES_DIR: &str = "files";
const PARTIAL_PREFIX: &str = ".partial-";
const SCHEMA_VERSION: u32 = 1;
/// A design is text measured in megabytes; this only stops a mistaken
/// project path (a home folder, say) from being copied wholesale.
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FILES: usize = 4096;
const MAX_DEPTH: usize = 16;
const MAX_LABEL_CHARS: usize = 120;
const DEFAULT_LIST_LIMIT: u64 = 20;
const MAX_LIST_LIMIT: u64 = 200;
const DEFAULT_IPC_TIMEOUT_SECONDS: u64 = 2;
const MAX_IPC_TIMEOUT_SECONDS: u64 = 30;

const CAPTURED_SUFFIXES: &[&str] = &[
    ".kicad_pro",
    ".kicad_sch",
    ".kicad_pcb",
    ".kicad_dru",
    ".kicad_sym",
    ".kicad_mod",
];
const CAPTURED_NAMES: &[&str] = &["sym-lib-table", "fp-lib-table"];

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "create_checkpoint",
            "Save a restorable checkpoint of a project's saved design files (.kicad_pro, \
             .kicad_sch, .kicad_pcb, .kicad_dru, .kicad_sym, .kicad_mod and the library \
             tables) under <project_dir>/.konnect/checkpoints/<id>/. Take one before a risky \
             edit. The copy is verified and the project re-read before the checkpoint is \
             listed. It holds the SAVED files: when KiCad has a document open, unsaved editor \
             changes are not in it, and locks_present names those documents. Returns the id \
             for restore_checkpoint.",
            json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Path to the project's .kicad_pro file"
                    },
                    "label": {
                        "type": "string",
                        "description": "Optional short note, such as 'before rerouting USB' (at most 120 characters)"
                    }
                },
                "required": ["project"]
            }),
            |args, ctx| async move { handle_create_checkpoint(args, ctx).await }
        ),
        tool!(
            "list_checkpoints",
            "List a project's checkpoints, newest first, with label, time, file count and \
             how each differs from the saved files now (changed, missing, added). Unreadable \
             checkpoints are reported, never skipped silently.",
            json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Path to the project's .kicad_pro file"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_LIST_LIMIT,
                        "default": DEFAULT_LIST_LIMIT,
                        "description": "Most checkpoints to return"
                    }
                },
                "required": ["project"]
            }),
            |args, ctx| async move { handle_list_checkpoints(args, ctx).await }
        ),
        tool!(
            "restore_checkpoint",
            "Restore a project's saved files from a checkpoint. Previews by default \
             (dry_run: true) and returns the plan and a revision; apply with dry_run: false \
             and that expected_revision. Refused (plan_blocked) while KiCad has a target \
             document or the project open, while a write journal is pending, or after a \
             board was seen live this session. Before writing it takes a pre_restore \
             checkpoint, returned as undo_checkpoint, so the restore can be undone. Files \
             added after the checkpoint are listed and left in place; restore never deletes. \
             Pass paths to restore only some files.",
            json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Path to the project's .kicad_pro file"
                    },
                    "checkpoint_id": {
                        "type": "string",
                        "description": "Checkpoint id from create_checkpoint or list_checkpoints"
                    },
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional subset of checkpoint paths (relative, forward slashes) to restore"
                    },
                    "dry_run": {
                        "type": "boolean",
                        "default": true,
                        "description": "true (default) previews; false applies"
                    },
                    "expected_revision": {
                        "type": "string",
                        "description": "The revision a preview returned; required to apply"
                    }
                },
                "required": ["project", "checkpoint_id"]
            }),
            |args, ctx| async move { handle_restore_checkpoint(args, ctx).await }
        ),
        tool!(
            "get_edit_readiness",
            "One table of who owns each design file of a project before editing it: KiCad \
             lock presence, whether KiCad has it open (over IPC), boards seen live earlier in \
             this session, and pending write journals. Each file gets edit_route: 'file' (edit \
             the saved file), 'live' (the board is open in KiCad, so Konnect's board tools \
             edit it through IPC), or 'blocked' with the reasons. Also says once whether KiCad \
             answers over IPC, so live-only tools are not tried against a closed KiCad. \
             Read-only; never clears a lock.",
            json!({
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Path to the project's .kicad_pro file"
                    },
                    "ipc_timeout_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_IPC_TIMEOUT_SECONDS,
                        "default": DEFAULT_IPC_TIMEOUT_SECONDS,
                        "description": "How long to wait for KiCad's IPC answer"
                    }
                },
                "required": ["project"]
            }),
            |args, ctx| async move { handle_get_edit_readiness(args, ctx).await }
        ),
    ]
}

// ─── Project and file discovery ──────────────────────────────────────────────

/// A resolved project: its canonical `.kicad_pro` and folder.
#[derive(Debug, Clone)]
struct Project {
    file: PathBuf,
    dir: PathBuf,
    /// The `.kicad_pro` file name, e.g. `amp.kicad_pro`.
    name: String,
}

fn resolve_project(args: &Value) -> Result<Project, CallToolResult> {
    let raw = require_str(args, "project")?;
    if raw.is_empty() {
        return Err(invalid_arg("project", "must not be empty"));
    }
    let path = PathBuf::from(raw);
    let is_pro = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("kicad_pro"));
    if !is_pro {
        return Err(invalid_arg(
            "project",
            "must be the project's .kicad_pro file",
        ));
    }
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(CallToolResult::error_kind(
                ToolErrorKind::FileNotFound {
                    path: path.display().to_string(),
                },
                format!("Project file not found: {}", path.display()),
            ));
        }
        Err(error) => {
            return Err(invalid_arg("project", &format!("cannot be read: {error}")));
        }
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(invalid_arg(
                "project",
                "must be the project file itself, not a symlink",
            ));
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(invalid_arg("project", "is not a regular file"));
        }
        Ok(_) => {}
    }
    let file = path
        .canonicalize()
        .map_err(|error| invalid_arg("project", &format!("cannot be resolved: {error}")))?;
    let dir = file
        .parent()
        .ok_or_else(|| invalid_arg("project", "has no parent folder"))?
        .to_path_buf();
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| invalid_arg("project", "file name is not valid UTF-8"))?
        .to_string();
    Ok(Project { file, dir, name })
}

fn is_captured_name(name: &str) -> bool {
    // `._*` are macOS AppleDouble sidecars on exFAT and network volumes.
    if name.starts_with('~')
        || name.starts_with('.')
        || name.starts_with("_autosave-")
        || name.ends_with(".lck")
    {
        return false;
    }
    CAPTURED_NAMES.contains(&name) || CAPTURED_SUFFIXES.iter().any(|s| name.ends_with(s))
}

fn skipped_dir(name: &str) -> bool {
    name.starts_with('.') || name.ends_with("-backups")
}

/// What a file is, for the readiness table.
fn file_kind(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.ends_with(".kicad_sch") {
        "schematic"
    } else if name.ends_with(".kicad_pcb") {
        "board"
    } else if name.ends_with(".kicad_pro") {
        "project"
    } else if name.ends_with(".kicad_dru") {
        "design_rules"
    } else if name.ends_with(".kicad_sym") {
        "symbol_library"
    } else if name.ends_with(".kicad_mod") {
        "footprint"
    } else {
        "library_table"
    }
}

#[derive(Debug, Clone, Serialize)]
struct Skipped {
    path: String,
    reason: &'static str,
}

#[derive(Debug, Default)]
struct Scan {
    /// Relative path (forward slashes) to absolute path, sorted.
    files: BTreeMap<String, PathBuf>,
    skipped: Vec<Skipped>,
}

fn relative_name(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_string()
    } else {
        format!("{base}/{name}")
    }
}

/// Walk the project folder for captured files without following symlinks.
fn scan_project(dir: &Path) -> std::io::Result<Scan> {
    let mut scan = Scan::default();
    let mut stack = vec![(dir.to_path_buf(), String::new(), 0usize)];
    while let Some((folder, relative, depth)) = stack.pop() {
        for entry in std::fs::read_dir(&folder)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let os_name = entry.file_name();
            let Some(name) = os_name.to_str() else {
                let lossy = relative_name(&relative, &os_name.to_string_lossy());
                if file_type.is_dir() || is_captured_name(&os_name.to_string_lossy()) {
                    scan.skipped.push(Skipped {
                        path: lossy,
                        reason: "name_not_utf8",
                    });
                }
                continue;
            };
            let rel = relative_name(&relative, name);
            if file_type.is_dir() {
                if skipped_dir(name) {
                    continue;
                }
                if depth + 1 > MAX_DEPTH {
                    scan.skipped.push(Skipped {
                        path: rel,
                        reason: "folder_too_deep",
                    });
                    continue;
                }
                stack.push((entry.path(), rel, depth + 1));
                continue;
            }
            if !is_captured_name(name) {
                continue;
            }
            if file_type.is_symlink() {
                scan.skipped.push(Skipped {
                    path: rel,
                    reason: "symlink",
                });
                continue;
            }
            if !file_type.is_file() {
                scan.skipped.push(Skipped {
                    path: rel,
                    reason: "not_a_regular_file",
                });
                continue;
            }
            scan.files.insert(rel, entry.path());
        }
    }
    Ok(scan)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// One file's bytes as read, with its digest.
#[derive(Debug, Clone)]
struct Captured {
    path: String,
    content: String,
    sha256: String,
}

#[derive(Debug)]
enum CaptureError {
    NotUtf8(String),
    TooMany(usize),
    TooLarge(u64),
    Io(String),
}

impl CaptureError {
    fn reason(&self) -> String {
        match self {
            Self::NotUtf8(path) => format!("not_utf8: {path} is not UTF-8 text, so it cannot be restored exactly"),
            Self::TooMany(count) => format!("too_many_files: {count} design files, the limit is {MAX_FILES}; is this the right project folder?"),
            Self::TooLarge(bytes) => format!("too_large: {bytes} bytes of design files, the limit is {MAX_TOTAL_BYTES}; is this the right project folder?"),
            Self::Io(reason) => format!("unreadable: {reason}"),
        }
    }
}

fn read_scan(scan: &Scan) -> Result<Vec<Captured>, CaptureError> {
    if scan.files.len() > MAX_FILES {
        return Err(CaptureError::TooMany(scan.files.len()));
    }
    let mut total = 0u64;
    let mut out = Vec::with_capacity(scan.files.len());
    for (path, absolute) in &scan.files {
        let bytes = std::fs::read(absolute)
            .map_err(|error| CaptureError::Io(format!("{path}: {error}")))?;
        total += bytes.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err(CaptureError::TooLarge(total));
        }
        let sha256 = sha256_hex(&bytes);
        let content = String::from_utf8(bytes).map_err(|_| CaptureError::NotUtf8(path.clone()))?;
        out.push(Captured {
            path: path.clone(),
            content,
            sha256,
        });
    }
    Ok(out)
}

/// Digest of a file set: sorted `path\0sha256\0` frames.
fn content_hash<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut sorted: Vec<_> = files.into_iter().collect();
    sorted.sort();
    let mut hasher = Sha256::new();
    for (path, sha) in sorted {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(sha.as_bytes());
        hasher.update([0]);
    }
    format!("sha256:{:x}", hasher.finalize())
}

/// KiCad lock files present beside the captured schematics and boards, plus
/// the project manager's own lock.
fn present_locks(project: &Project, files: &[&str]) -> Vec<String> {
    let mut locks = Vec::new();
    for path in files {
        let kind = file_kind(path);
        if kind != "schematic" && kind != "board" {
            continue;
        }
        match editor_lock(&project.dir.join(path)) {
            EditorLock::Absent => {}
            EditorLock::Present(lock) | EditorLock::Unreadable(lock, _) => {
                locks.push(display_relative(&project.dir, &lock));
            }
        }
    }
    match project_lock(project) {
        LockState::Absent => {}
        LockState::Present(lock) | LockState::Unreadable(lock, _) => {
            locks.push(display_relative(&project.dir, &lock))
        }
    }
    locks
}

fn display_relative(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir)
        .map(|p| {
            p.components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_else(|_| path.display().to_string())
}

#[derive(Debug)]
enum LockState {
    Absent,
    Present(PathBuf),
    Unreadable(PathBuf, String),
}

impl LockState {
    fn label(&self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Present(_) => "present",
            Self::Unreadable(..) => "unreadable",
        }
    }
}

/// KiCad's project manager lock, `~<name>.kicad_pro.lck`: KiCad has the
/// project open and may rewrite project files when it saves or closes.
fn project_lock(project: &Project) -> LockState {
    let lock = project.dir.join(format!("~{}.lck", project.name));
    match std::fs::symlink_metadata(&lock) {
        Ok(_) => LockState::Present(lock),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => LockState::Absent,
        Err(error) => LockState::Unreadable(lock, error.to_string()),
    }
}

/// Why every sheet of the project counts as open in KiCad's schematic editor,
/// if it does. KiCad locks only the root sheet it opened, so a lock on any
/// sheet, or IPC reporting the schematic editor on this project, covers them all.
fn schematic_editor_evidence(project: &Project, open: Option<&OpenInKicad>) -> Option<String> {
    if open.is_some_and(|open| open.schematic_open) {
        return Some("KiCad reports the schematic editor open over IPC".to_string());
    }
    let scan = scan_project(&project.dir).ok()?;
    scan.files
        .iter()
        .filter(|(path, _)| file_kind(path) == "schematic")
        .find_map(|(path, absolute)| match editor_lock_state(absolute) {
            LockState::Absent => None,
            LockState::Present(_) => Some(format!("{path} is locked")),
            LockState::Unreadable(..) => Some(format!("the lock on {path} is unreadable")),
        })
}

fn editor_lock_state(path: &Path) -> LockState {
    match editor_lock(path) {
        EditorLock::Absent => LockState::Absent,
        EditorLock::Present(lock) => LockState::Present(lock),
        EditorLock::Unreadable(lock, error) => LockState::Unreadable(lock, error.to_string()),
    }
}

// ─── Checkpoint store ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    path: String,
    sha256: String,
    bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    id: String,
    label: Option<String>,
    /// `manual` or `pre_restore`.
    reason: String,
    /// Milliseconds since the Unix epoch; orders checkpoints within one second.
    created_unix_ms: u64,
    created_utc: String,
    project_file: String,
    content_hash: String,
    files: Vec<ManifestFile>,
    locks_at_creation: Vec<String>,
}

/// `yyyymmddThhmmss-<8 hex>-<4 hex>`; ids become folder names, so nothing else
/// is ever joined onto a path.
fn is_valid_checkpoint_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.len() != 15 + 1 + 8 + 1 + 4 {
        return false;
    }
    let digits = |range: std::ops::Range<usize>| bytes[range].iter().all(u8::is_ascii_digit);
    let hex = |range: std::ops::Range<usize>| {
        bytes[range]
            .iter()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    };
    digits(0..8)
        && bytes[8] == b'T'
        && digits(9..15)
        && bytes[15] == b'-'
        && hex(16..24)
        && bytes[24] == b'-'
        && hex(25..29)
}

/// A manifest path is relative, forward-slashed, and never climbs or hides.
fn is_safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && !part.starts_with('.'))
        && path.rsplit('/').next().is_some_and(is_captured_name)
}

/// The first folder on the way to `relative` (below `base`) that is a symlink
/// or not a folder, if any. A missing folder ends the walk: restore creates it.
fn unsafe_parent(base: &Path, relative: &str) -> Option<String> {
    let parts: Vec<&str> = relative.split('/').collect();
    let mut path = base.to_path_buf();
    for (index, part) in parts[..parts.len().saturating_sub(1)].iter().enumerate() {
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Some(parts[..=index].join("/"));
            }
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

fn join_relative(base: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(base.to_path_buf(), |p, c| p.join(c))
}

/// The checkpoint store, refusing a symlinked `.konnect` or `checkpoints`
/// folder so a checkpoint can never be written or read outside the project.
fn store_dir(project: &Project, create: bool) -> Result<Option<PathBuf>, String> {
    let mut path = project.dir.clone();
    for component in [STORE_PARENT, STORE_DIR] {
        path.push(component);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(format!(
                    "{} must be a real folder, not a symlink or file",
                    display_relative(&project.dir, &path)
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !create {
                    return Ok(None);
                }
                std::fs::create_dir(&path).map_err(|error| {
                    format!(
                        "cannot create {}: {error}",
                        display_relative(&project.dir, &path)
                    )
                })?;
            }
            Err(error) => {
                return Err(format!(
                    "cannot inspect {}: {error}",
                    display_relative(&project.dir, &path)
                ))
            }
        }
    }
    Ok(Some(path))
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

fn utc_text(seconds: u64) -> String {
    let stamp = crate::simulation::runs::stamp(seconds);
    format!(
        "{}-{}-{}T{}:{}:{}Z",
        &stamp[0..4],
        &stamp[4..6],
        &stamp[6..8],
        &stamp[9..11],
        &stamp[11..13],
        &stamp[13..15]
    )
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[derive(Debug)]
enum CreateError {
    /// The project could not be captured as it stands (with a reason code).
    Capture(String),
    /// The saved files changed while they were being copied.
    Changed,
    /// The store could not be written; nothing was listed.
    Store(String),
}

impl CreateError {
    fn into_result(self, operation: &str, project: &Project) -> CallToolResult {
        match self {
            Self::Capture(reason) => CallToolResult::error_kind(
                ToolErrorKind::PlanBlocked {
                    operation: operation.to_string(),
                    reasons: vec![reason.clone()],
                },
                format!("Cannot checkpoint {}: {reason}", project.file.display()),
            ),
            Self::Changed => CallToolResult::error_kind(
                ToolErrorKind::StaleTarget {
                    target: project.file.display().to_string(),
                    reason: "project_changed_during_checkpoint".to_string(),
                },
                "The project's saved files changed while the checkpoint was being taken (KiCad \
                 or another tool saved). Nothing was kept; call again once saving has finished.",
            ),
            Self::Store(reason) => CallToolResult::error_kind(
                ToolErrorKind::HandlerError {
                    reason: reason.clone(),
                },
                format!("Cannot write the checkpoint store: {reason}"),
            ),
        }
    }
}

/// Take a checkpoint of the project as saved now.
fn create_checkpoint_in(
    project: &Project,
    label: Option<String>,
    reason: &str,
) -> Result<(Manifest, Vec<Skipped>), CreateError> {
    let scan = scan_project(&project.dir)
        .map_err(|error| CreateError::Capture(format!("unreadable: {error}")))?;
    let captured = read_scan(&scan).map_err(|error| CreateError::Capture(error.reason()))?;
    if captured.is_empty() {
        return Err(CreateError::Capture(
            "no_design_files: the project folder holds no captured design files".to_string(),
        ));
    }
    let hash = content_hash(
        captured
            .iter()
            .map(|c| (c.path.as_str(), c.sha256.as_str())),
    );
    let paths: Vec<&str> = captured.iter().map(|c| c.path.as_str()).collect();
    let locks = present_locks(project, &paths);

    let store = store_dir(project, true)
        .map_err(CreateError::Store)?
        .expect("store is created on demand");
    let partial = store.join(format!("{PARTIAL_PREFIX}{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&partial).map_err(|error| {
        CreateError::Store(format!("cannot create a checkpoint folder: {error}"))
    })?;
    let cleanup = |error: CreateError| {
        let _ = std::fs::remove_dir_all(&partial);
        error
    };

    let created_unix_ms = now_unix_ms();
    let created_unix = created_unix_ms / 1000;
    let id = format!(
        "{}-{}-{}",
        crate::simulation::runs::stamp(created_unix),
        &hash["sha256:".len().."sha256:".len() + 8],
        &uuid::Uuid::new_v4().simple().to_string()[..4]
    );
    let files_root = partial.join(FILES_DIR);
    for file in &captured {
        let target = join_relative(&files_root, &file.path);
        let written = target
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| write_synced(&target, file.content.as_bytes()));
        if let Err(error) = written {
            return Err(cleanup(CreateError::Store(format!(
                "cannot copy {}: {error}",
                file.path
            ))));
        }
    }
    // Every copy must read back as the bytes that were hashed.
    for file in &captured {
        match std::fs::read(join_relative(&files_root, &file.path)) {
            Ok(bytes) if sha256_hex(&bytes) == file.sha256 => {}
            Ok(_) => {
                return Err(cleanup(CreateError::Store(format!(
                    "the copy of {} did not read back as written",
                    file.path
                ))))
            }
            Err(error) => {
                return Err(cleanup(CreateError::Store(format!(
                    "cannot read back {}: {error}",
                    file.path
                ))))
            }
        }
    }
    // The project must still be what was copied: a save during the copy would
    // leave a checkpoint that matches no state the design was ever in.
    let rescanned = scan_project(&project.dir)
        .ok()
        .and_then(|scan| read_scan(&scan).ok())
        .map(|files| content_hash(files.iter().map(|c| (c.path.as_str(), c.sha256.as_str()))));
    if rescanned.as_deref() != Some(hash.as_str()) {
        return Err(cleanup(CreateError::Changed));
    }

    let manifest = Manifest {
        schema_version: SCHEMA_VERSION,
        id: id.clone(),
        label,
        reason: reason.to_string(),
        created_unix_ms,
        created_utc: utc_text(created_unix),
        project_file: project.name.clone(),
        content_hash: hash,
        files: captured
            .iter()
            .map(|c| ManifestFile {
                path: c.path.clone(),
                sha256: c.sha256.clone(),
                bytes: c.content.len() as u64,
            })
            .collect(),
        locks_at_creation: locks,
    };
    let text = serde_json::to_string_pretty(&manifest).expect("manifest serializes");
    if let Err(error) = write_synced(&partial.join(MANIFEST_FILE), text.as_bytes()) {
        return Err(cleanup(CreateError::Store(format!(
            "cannot write the manifest: {error}"
        ))));
    }
    let final_dir = store.join(&id);
    if let Err(error) = std::fs::rename(&partial, &final_dir) {
        return Err(cleanup(CreateError::Store(format!(
            "cannot publish checkpoint {id}: {error}"
        ))));
    }
    if let Ok(store_handle) = std::fs::File::open(&store) {
        let _ = store_handle.sync_all();
    }
    Ok((manifest, scan.skipped))
}

/// Read and validate one checkpoint's manifest (not its file copies).
fn load_manifest(store: &Path, id: &str) -> Result<Manifest, String> {
    if !is_valid_checkpoint_id(id) {
        return Err("not a checkpoint id".to_string());
    }
    let dir = store.join(id);
    match std::fs::symlink_metadata(&dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("checkpoint is not a real folder".to_string())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("not_found".to_string())
        }
        Err(error) => return Err(format!("cannot inspect: {error}")),
    }
    let manifest_path = dir.join(MANIFEST_FILE);
    match std::fs::symlink_metadata(&manifest_path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err("manifest is not a regular file".to_string()),
        Err(error) => return Err(format!("manifest unreadable: {error}")),
    }
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("manifest unreadable: {error}"))?;
    let manifest: Manifest =
        serde_json::from_str(&text).map_err(|error| format!("manifest malformed: {error}"))?;
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "manifest schema_version {} is not supported",
            manifest.schema_version
        ));
    }
    if manifest.id != id {
        return Err("manifest id does not match its folder".to_string());
    }
    let mut seen = BTreeSet::new();
    for file in &manifest.files {
        if !is_safe_relative(&file.path) || !seen.insert(file.path.as_str()) {
            return Err(format!(
                "manifest names an unsafe or duplicate path: {}",
                file.path
            ));
        }
        if file.sha256.len() != 64 || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("manifest digest for {} is malformed", file.path));
        }
    }
    Ok(manifest)
}

/// Read every stored copy and prove it is the manifest's bytes.
fn load_checkpoint_files(
    store: &Path,
    manifest: &Manifest,
) -> Result<BTreeMap<String, String>, String> {
    let root = store.join(&manifest.id).join(FILES_DIR);
    let mut out = BTreeMap::new();
    for file in &manifest.files {
        let path = join_relative(&root, &file.path);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => {
                return Err(format!(
                    "stored copy of {} is not a regular file",
                    file.path
                ))
            }
            Err(error) => {
                return Err(format!(
                    "stored copy of {} is unreadable: {error}",
                    file.path
                ))
            }
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("stored copy of {} is unreadable: {error}", file.path))?;
        if sha256_hex(&bytes) != file.sha256 {
            return Err(format!(
                "stored copy of {} does not match its digest",
                file.path
            ));
        }
        let text = String::from_utf8(bytes)
            .map_err(|_| format!("stored copy of {} is not UTF-8", file.path))?;
        out.insert(file.path.clone(), text);
    }
    Ok(out)
}

// ─── IPC observation ─────────────────────────────────────────────────────────

/// What KiCad's IPC endpoint said, bounded by a timeout.
struct IpcView {
    status: &'static str,
    detail: Value,
    /// Open documents when the endpoint answered.
    observation: Option<konnect_ipc::IpcEditorStateObservation>,
}

async fn observe_ipc(ctx: &ToolContext, timeout: Duration) -> IpcView {
    let address = ctx.config.ipc_address.clone();
    if address.is_empty() {
        return IpcView {
            status: "not_configured",
            detail: json!({ "reason": "no KiCad IPC endpoint is configured" }),
            observation: None,
        };
    }
    let probe = tokio::task::spawn_blocking(move || {
        let client = konnect_ipc::KiCadIpcClient::new(address);
        let ping = client.ping_outcome();
        if !ping.is_responsive() {
            return (Some(ping), None);
        }
        (Some(ping), Some(client.observe_editor_state()))
    });
    match tokio::time::timeout(timeout, probe).await {
        Err(_) => IpcView {
            status: "timed_out",
            detail: json!({ "timeout_seconds": timeout.as_secs() }),
            observation: None,
        },
        Ok(Err(error)) => IpcView {
            status: "error",
            detail: json!({ "reason": error.to_string() }),
            observation: None,
        },
        Ok(Ok((ping, observed))) => {
            let failure = ping
                .as_ref()
                .map_or(Value::Null, crate::tools::ipc_failure_evidence);
            match observed {
                None => IpcView {
                    status: "unreachable",
                    detail: json!({ "ipc_failure": failure }),
                    observation: None,
                },
                Some(Err(error)) => IpcView {
                    status: "reachable_without_documents",
                    detail: json!({ "reason": format!("{error:#}") }),
                    observation: None,
                },
                Some(Ok(observation)) => IpcView {
                    status: "reachable",
                    detail: json!({ "kicad_version": observation.kicad_version.full_version }),
                    observation: Some(observation),
                },
            }
        }
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => left == right,
    }
}

/// Whether `path` lies in the (canonical) project folder. KiCad may report a
/// different spelling of the same folder, such as `/var` for `/private/var`.
fn within(dir: &Path, path: &Path) -> bool {
    path.canonicalize()
        .or_else(|_| {
            let parent = path.parent().ok_or(std::io::ErrorKind::NotFound)?;
            Ok::<_, std::io::Error>(
                parent
                    .canonicalize()?
                    .join(path.file_name().unwrap_or_default()),
            )
        })
        .map_or_else(
            |_| path.starts_with(dir),
            |resolved| resolved.starts_with(dir),
        )
}

/// Which of this project's documents KiCad reports open.
#[derive(Debug, Default)]
struct OpenInKicad {
    project_open: bool,
    schematic_open: bool,
    boards: Vec<PathBuf>,
}

fn open_in_kicad(
    project: &Project,
    observation: &konnect_ipc::IpcEditorStateObservation,
) -> OpenInKicad {
    let mut open = OpenInKicad::default();
    for editor in &observation.editors {
        for document in &editor.documents {
            let in_project = document.project.as_ref().is_some_and(|identity| {
                let path = Path::new(&identity.path);
                same_path(path, &project.dir) || same_path(path, &project.file)
            });
            let board = document
                .document_path
                .as_ref()
                .map(PathBuf::from)
                .filter(|path| within(&project.dir, path));
            if !in_project && board.is_none() {
                continue;
            }
            open.project_open = true;
            match document.editor {
                konnect_ipc::IpcEditorKind::Schematic => open.schematic_open = true,
                konnect_ipc::IpcEditorKind::Pcb => {
                    if let Some(board) = board {
                        open.boards.push(board);
                    }
                }
            }
        }
    }
    open
}

fn ipc_timeout(args: &Value) -> Result<Duration, CallToolResult> {
    match &args["ipc_timeout_seconds"] {
        Value::Null => Ok(Duration::from_secs(DEFAULT_IPC_TIMEOUT_SECONDS)),
        value => match value.as_u64() {
            Some(seconds) if (1..=MAX_IPC_TIMEOUT_SECONDS).contains(&seconds) => {
                Ok(Duration::from_secs(seconds))
            }
            _ => Err(invalid_arg(
                "ipc_timeout_seconds",
                &format!("must be an integer from 1 to {MAX_IPC_TIMEOUT_SECONDS}"),
            )),
        },
    }
}

// ─── Handlers ────────────────────────────────────────────────────────────────

fn parse_label(args: &Value) -> Result<Option<String>, CallToolResult> {
    match &args["label"] {
        Value::Null => Ok(None),
        Value::String(label) => {
            let label = label.trim();
            if label.is_empty() {
                return Ok(None);
            }
            if label.chars().count() > MAX_LABEL_CHARS {
                return Err(invalid_arg(
                    "label",
                    &format!("must be at most {MAX_LABEL_CHARS} characters"),
                ));
            }
            if label.chars().any(char::is_control) {
                return Err(invalid_arg("label", "must not contain control characters"));
            }
            Ok(Some(label.to_string()))
        }
        _ => Err(invalid_arg("label", "must be a string")),
    }
}

async fn handle_create_checkpoint(
    args: &Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let project = match resolve_project(args) {
        Ok(project) => project,
        Err(error) => return Ok(error),
    };
    let label = match parse_label(args) {
        Ok(label) => label,
        Err(error) => return Ok(error),
    };
    let task_project = project.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let previous = newest_manifest(&task_project);
        create_checkpoint_in(&task_project, label, "manual").map(|done| (done, previous))
    })
    .await?;
    let ((manifest, skipped), previous) = match outcome {
        Ok(done) => done,
        Err(error) => return Ok(error.into_result("create_checkpoint", &project)),
    };
    let total_bytes: u64 = manifest.files.iter().map(|f| f.bytes).sum();
    let mut result = json!({
        "checkpoint_id": manifest.id,
        "label": manifest.label,
        "created_utc": manifest.created_utc,
        "file_count": manifest.files.len(),
        "total_bytes": total_bytes,
        "content_hash": manifest.content_hash,
        "location": project.dir.join(STORE_PARENT).join(STORE_DIR).join(&manifest.id).display().to_string(),
        "locks_present": manifest.locks_at_creation,
        "skipped": skipped,
    });
    if !manifest.locks_at_creation.is_empty() {
        result["warning"] = json!(
            "KiCad has documents of this project open. The checkpoint holds the saved files; \
             unsaved editor changes are not in it."
        );
    }
    if let Some(previous) = previous.filter(|p| p.content_hash == manifest.content_hash) {
        result["identical_to"] = json!(previous.id);
    }
    Ok(CallToolResult::json(&result))
}

fn newest_manifest(project: &Project) -> Option<Manifest> {
    let store = store_dir(project, false).ok()??;
    read_store(&store).ok()?.0.into_iter().next()
}

/// Every readable manifest, newest first, and every entry that is not one.
///
/// Ids carry the creation second, so two checkpoints in one second are ordered
/// by their millisecond time, not by the hash in their ids.
fn read_store(store: &Path) -> Result<(Vec<Manifest>, Vec<Value>), String> {
    let mut manifests = Vec::new();
    let mut unreadable = Vec::new();
    for entry in std::fs::read_dir(store).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let Ok(name) = entry.file_name().into_string() else {
            unreadable.push(
                json!({ "id": entry.file_name().to_string_lossy(), "reason": "name_not_utf8" }),
            );
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if !is_valid_checkpoint_id(&name) {
            unreadable.push(json!({ "id": name, "reason": "not a checkpoint id" }));
            continue;
        }
        match load_manifest(store, &name) {
            Ok(manifest) => manifests.push(manifest),
            Err(reason) => unreadable.push(json!({ "id": name, "reason": reason })),
        }
    }
    manifests.sort_by(|a, b| (b.created_unix_ms, &b.id).cmp(&(a.created_unix_ms, &a.id)));
    Ok((manifests, unreadable))
}

fn current_digests(project: &Project) -> Result<BTreeMap<String, String>, String> {
    let scan = scan_project(&project.dir).map_err(|error| error.to_string())?;
    let mut out = BTreeMap::new();
    for (path, absolute) in &scan.files {
        let bytes = std::fs::read(absolute).map_err(|error| format!("{path}: {error}"))?;
        out.insert(path.clone(), sha256_hex(&bytes));
    }
    Ok(out)
}

async fn handle_list_checkpoints(
    args: &Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let project = match resolve_project(args) {
        Ok(project) => project,
        Err(error) => return Ok(error),
    };
    let limit = match &args["limit"] {
        Value::Null => DEFAULT_LIST_LIMIT,
        value => match value.as_u64() {
            Some(limit) if (1..=MAX_LIST_LIMIT).contains(&limit) => limit,
            _ => {
                return Ok(invalid_arg(
                    "limit",
                    &format!("must be an integer from 1 to {MAX_LIST_LIMIT}"),
                ))
            }
        },
    } as usize;
    let task_project = project.clone();
    let listing =
        tokio::task::spawn_blocking(move || list_checkpoints_in(&task_project, limit)).await?;
    match listing {
        Ok(result) => Ok(CallToolResult::json(&result)),
        Err(reason) => Ok(CallToolResult::error_kind(
            ToolErrorKind::HandlerError {
                reason: reason.clone(),
            },
            format!("Cannot list checkpoints: {reason}"),
        )),
    }
}

fn list_checkpoints_in(project: &Project, limit: usize) -> Result<Value, String> {
    let Some(store) = store_dir(project, false)? else {
        return Ok(json!({ "checkpoints": [], "count": 0, "truncated": false, "unreadable": [] }));
    };
    let (manifests, unreadable) = read_store(&store)?;
    let current = current_digests(project).ok();
    let mut checkpoints = Vec::new();
    let total = manifests.len();
    for manifest in manifests.iter().take(limit) {
        let difference = current.as_ref().map(|current| {
            let stored: BTreeMap<&str, &str> = manifest
                .files
                .iter()
                .map(|f| (f.path.as_str(), f.sha256.as_str()))
                .collect();
            let changed = stored
                .iter()
                .filter(|(path, sha)| current.get(**path).is_some_and(|now| now != **sha))
                .count();
            let missing = stored
                .keys()
                .filter(|path| !current.contains_key(**path))
                .count();
            let added = current
                .keys()
                .filter(|path| !stored.contains_key(path.as_str()))
                .count();
            json!({
                "matches": changed == 0 && missing == 0 && added == 0,
                "changed": changed,
                "missing": missing,
                "added": added,
            })
        });
        checkpoints.push(json!({
            "checkpoint_id": manifest.id,
            "label": manifest.label,
            "reason": manifest.reason,
            "created_utc": manifest.created_utc,
            "file_count": manifest.files.len(),
            "total_bytes": manifest.files.iter().map(|f| f.bytes).sum::<u64>(),
            "difference_from_saved_files": difference,
        }));
    }
    Ok(json!({
        "checkpoints": checkpoints,
        "count": checkpoints.len(),
        "total": total,
        "truncated": total > limit,
        "unreadable": unreadable,
        "current_files_readable": current.is_some(),
    }))
}

/// One restore target and what restoring it would do.
#[derive(Debug, Clone, Serialize)]
struct RestoreEntry {
    path: String,
    /// `replace`, `create` or `unchanged`.
    action: &'static str,
    #[serde(skip)]
    current: Option<String>,
    #[serde(skip)]
    current_sha: Option<String>,
}

struct RestorePlan {
    entries: Vec<RestoreEntry>,
    added_since: Vec<String>,
    blockers: Vec<String>,
    revision: String,
}

fn restore_revision(id: &str, paths: Option<&[String]>, entries: &[RestoreEntry]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"konnect-restore-v1\0");
    hasher.update(id.as_bytes());
    hasher.update([0]);
    if let Some(paths) = paths {
        for path in paths {
            hasher.update(b"only\0");
            hasher.update(path.as_bytes());
            hasher.update([0]);
        }
    }
    for entry in entries {
        hasher.update(entry.path.as_bytes());
        hasher.update([0]);
        hasher.update(entry.current_sha.as_deref().unwrap_or("absent").as_bytes());
        hasher.update([0]);
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn plan_restore(
    project: &Project,
    manifest: &Manifest,
    stored: &BTreeMap<String, String>,
    only: Option<&[String]>,
    board_live: &dyn Fn(&Path) -> bool,
    open: &OpenInKicad,
) -> RestorePlan {
    let selected: Vec<&ManifestFile> = manifest
        .files
        .iter()
        .filter(|f| only.is_none_or(|only| only.contains(&f.path)))
        .collect();
    let mut entries = Vec::with_capacity(selected.len());
    let mut blockers = Vec::new();
    for file in &selected {
        let absolute = join_relative(&project.dir, &file.path);
        if let Some(folder) = unsafe_parent(&project.dir, &file.path) {
            // Never read or write through a folder that leads out of the project.
            blockers.push(format!(
                "target_parent_not_a_folder: {folder} (on the way to {}) is a symlink or a file",
                file.path
            ));
            entries.push(RestoreEntry {
                path: file.path.clone(),
                action: "replace",
                current: None,
                current_sha: Some("unsafe_parent".to_string()),
            });
            continue;
        }
        let (current, current_sha) = match std::fs::symlink_metadata(&absolute) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, None),
            Err(error) => {
                blockers.push(format!("target_unreadable: {}: {error}", file.path));
                (None, Some("unreadable".to_string()))
            }
            Ok(metadata) if metadata.file_type().is_symlink() => {
                blockers.push(format!("target_is_symlink: {}", file.path));
                (None, Some("symlink".to_string()))
            }
            Ok(metadata) if !metadata.is_file() => {
                blockers.push(format!("target_not_a_regular_file: {}", file.path));
                (None, Some("not_a_file".to_string()))
            }
            Ok(_) => match std::fs::read(&absolute) {
                Ok(bytes) => {
                    let sha = sha256_hex(&bytes);
                    match String::from_utf8(bytes) {
                        Ok(text) => (Some(text), Some(sha)),
                        Err(_) => {
                            blockers.push(format!("target_not_utf8: {}", file.path));
                            (None, Some(sha))
                        }
                    }
                }
                Err(error) => {
                    blockers.push(format!("target_unreadable: {}: {error}", file.path));
                    (None, Some("unreadable".to_string()))
                }
            },
        };
        let action = match (&current_sha, stored.get(&file.path)) {
            (Some(sha), _) if *sha == file.sha256 => "unchanged",
            (None, _) => "create",
            _ => "replace",
        };
        entries.push(RestoreEntry {
            path: file.path.clone(),
            action,
            current,
            current_sha,
        });
    }

    let editor_holding_sheets = schematic_editor_evidence(project, Some(open));
    for entry in entries.iter().filter(|e| e.action != "unchanged") {
        let absolute = join_relative(&project.dir, &entry.path);
        let kind = file_kind(&entry.path);
        if kind == "schematic" {
            if let Some(evidence) = &editor_holding_sheets {
                if !matches!(editor_lock_state(&absolute), LockState::Present(_)) {
                    blockers.push(format!(
                        "kicad_schematic_open: {} belongs to a schematic KiCad is editing ({evidence}); sub-sheets have no lock of their own",
                        entry.path
                    ));
                }
            }
        }
        if kind == "schematic" || kind == "board" {
            match editor_lock_state(&absolute) {
                LockState::Absent => {}
                LockState::Present(_) => blockers.push(format!(
                    "kicad_lock_present: {} is open in KiCad; save and close it first",
                    entry.path
                )),
                LockState::Unreadable(_, error) => {
                    blockers.push(format!("kicad_lock_unreadable: {}: {error}", entry.path))
                }
            }
        }
        if kind == "board" {
            if open.boards.iter().any(|board| same_path(board, &absolute)) {
                blockers.push(format!(
                    "open_in_kicad: {} is open in KiCad's PCB editor",
                    entry.path
                ));
            } else if board_live(&absolute) {
                blockers.push(format!(
                    "board_observed_live: {} was open in KiCad earlier in this session and may hold unsaved changes",
                    entry.path
                ));
            }
        }
    }
    let changes = entries.iter().any(|e| e.action != "unchanged");
    if changes {
        match project_lock(project) {
            LockState::Absent => {}
            LockState::Present(_) => blockers.push(format!(
                "kicad_project_open: KiCad has {} open and may rewrite project files when it saves or closes; close the project first",
                project.name
            )),
            LockState::Unreadable(_, error) => {
                blockers.push(format!("kicad_project_lock_unreadable: {error}"))
            }
        }
        if open.project_open && !blockers.iter().any(|b| b.starts_with("kicad_project_open")) {
            blockers.push(format!(
                "kicad_project_open: KiCad reports documents of {} open over IPC",
                project.name
            ));
        }
        match konnect_sexp::inspect_file_transactions(&project.dir) {
            Ok(pending) => {
                for transaction in pending {
                    blockers.push(format!(
                        "pending_transaction: {} (resolve with `konnect transaction status|recover <project-dir>`)",
                        transaction.id
                    ));
                }
            }
            Err(error) => blockers.push(format!("transaction_journal_unreadable: {error}")),
        }
    }

    let selected_paths: BTreeSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    let added_since = if only.is_some() {
        Vec::new()
    } else {
        scan_project(&project.dir)
            .map(|scan| {
                scan.files
                    .into_keys()
                    .filter(|path| !selected_paths.contains(path.as_str()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let revision = restore_revision(&manifest.id, only, &entries);
    RestorePlan {
        entries,
        added_since,
        blockers,
        revision,
    }
}

fn parse_paths(args: &Value, manifest: &Manifest) -> Result<Option<Vec<String>>, CallToolResult> {
    let Some(paths) = crate::tools::opt_str_list(args, "paths")? else {
        return Ok(None);
    };
    if paths.is_empty() {
        return Err(invalid_arg(
            "paths",
            "must name at least one file when given",
        ));
    }
    let known: BTreeSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    let unknown: Vec<&str> = paths
        .iter()
        .map(String::as_str)
        .filter(|path| !known.contains(path))
        .collect();
    if !unknown.is_empty() {
        return Err(invalid_arg(
            "paths",
            &format!(
                "not in checkpoint {}: {}. Use the relative paths list_checkpoints and the preview report",
                manifest.id,
                unknown.join(", ")
            ),
        ));
    }
    let mut paths = paths;
    paths.sort();
    paths.dedup();
    Ok(Some(paths))
}

async fn handle_restore_checkpoint(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let project = match resolve_project(args) {
        Ok(project) => project,
        Err(error) => return Ok(error),
    };
    let id = match require_str(args, "checkpoint_id") {
        Ok(id) => id.to_string(),
        Err(error) => return Ok(error),
    };
    if !is_valid_checkpoint_id(&id) {
        return Ok(invalid_arg(
            "checkpoint_id",
            "is not a checkpoint id (expected yyyymmddThhmmss-xxxxxxxx-xxxx from list_checkpoints)",
        ));
    }
    let dry_run = match &args["dry_run"] {
        Value::Null => true,
        Value::Bool(flag) => *flag,
        _ => return Ok(invalid_arg("dry_run", "must be a boolean")),
    };
    let expected_revision = opt_str(args, "expected_revision").map(str::to_string);
    if !dry_run && expected_revision.is_none() {
        return Ok(invalid_arg(
            "expected_revision",
            "is required to apply; preview with dry_run: true and pass the revision it returns",
        ));
    }

    let store = match store_dir(&project, false) {
        Ok(Some(store)) => store,
        Ok(None) => return Ok(checkpoint_not_found(&project, &id)),
        Err(reason) => {
            return Ok(CallToolResult::error_kind(
                ToolErrorKind::PlanBlocked {
                    operation: "restore_checkpoint".to_string(),
                    reasons: vec![format!("checkpoint_store_unsafe: {reason}")],
                },
                format!("Cannot use the checkpoint store: {reason}"),
            ))
        }
    };
    let manifest = match load_manifest(&store, &id) {
        Ok(manifest) => manifest,
        Err(reason) if reason == "not_found" => return Ok(checkpoint_not_found(&project, &id)),
        Err(reason) => return Ok(checkpoint_corrupt(&id, &reason)),
    };
    let only = match parse_paths(args, &manifest) {
        Ok(only) => only,
        Err(error) => return Ok(error),
    };
    let ipc = observe_ipc(ctx, Duration::from_secs(DEFAULT_IPC_TIMEOUT_SECONDS)).await;
    let open = ipc
        .observation
        .as_ref()
        .map(|observation| open_in_kicad(&project, observation))
        .unwrap_or_default();

    let task_project = project.clone();
    let task_store = store.clone();
    let board_session = ctx.board_session.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        restore_blocking(
            &task_project,
            &task_store,
            &manifest,
            only.as_deref(),
            &|board| !board_session.authorize_file_fallback(board),
            &open,
            dry_run,
            expected_revision.as_deref(),
        )
    })
    .await?;
    let mut result = outcome;
    if dry_run && !result.is_error {
        // The preview says once whether KiCad answered, so its blockers are read
        // with the evidence they rest on.
        if let Some(crate::mcp::protocol::ToolContent::Text { text }) = result.content.first() {
            if let Ok(mut body) = serde_json::from_str::<Value>(text) {
                body["kicad_ipc"] = json!({ "status": ipc.status, "detail": ipc.detail });
                result = CallToolResult::json(&body);
            }
        }
    }
    Ok(result)
}

fn checkpoint_not_found(project: &Project, id: &str) -> CallToolResult {
    let path = project
        .dir
        .join(STORE_PARENT)
        .join(STORE_DIR)
        .join(id)
        .display()
        .to_string();
    CallToolResult::error_kind(
        ToolErrorKind::FileNotFound { path: path.clone() },
        format!("No checkpoint {id} for this project ({path}); call list_checkpoints"),
    )
}

fn checkpoint_corrupt(id: &str, reason: &str) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::PlanBlocked {
            operation: "restore_checkpoint".to_string(),
            reasons: vec![format!("checkpoint_corrupt: {reason}")],
        },
        format!("Checkpoint {id} cannot be trusted and was not used: {reason}"),
    )
}

#[allow(clippy::too_many_arguments)]
fn restore_blocking(
    project: &Project,
    store: &Path,
    manifest: &Manifest,
    only: Option<&[String]>,
    board_live: &dyn Fn(&Path) -> bool,
    open: &OpenInKicad,
    dry_run: bool,
    expected_revision: Option<&str>,
) -> CallToolResult {
    let stored = match load_checkpoint_files(store, manifest) {
        Ok(stored) => stored,
        Err(reason) => return checkpoint_corrupt(&manifest.id, &reason),
    };
    let plan = plan_restore(project, manifest, &stored, only, board_live, open);
    let changes: Vec<&RestoreEntry> = plan
        .entries
        .iter()
        .filter(|e| e.action != "unchanged")
        .collect();
    let unchanged = plan.entries.len() - changes.len();

    if dry_run {
        return CallToolResult::json(&json!({
            "dry_run": true,
            "checkpoint_id": manifest.id,
            "label": manifest.label,
            "created_utc": manifest.created_utc,
            "changes": changes,
            "unchanged": unchanged,
            "added_since": plan.added_since,
            "added_since_note": "Files created after the checkpoint are left in place; restore never deletes.",
            "blocked_by": plan.blockers,
            "can_apply": plan.blockers.is_empty() && !changes.is_empty(),
            "revision": plan.revision,
        }));
    }

    if expected_revision != Some(plan.revision.as_str()) {
        return CallToolResult::error_kind(
            ToolErrorKind::StaleTarget {
                target: project.file.display().to_string(),
                reason: "revision_mismatch".to_string(),
            },
            "The project's files changed since the preview (or the preview was for other \
             paths). Nothing was restored; preview again and apply with the new revision.",
        );
    }
    if !plan.blockers.is_empty() {
        return CallToolResult::error_kind(
            ToolErrorKind::PlanBlocked {
                operation: "restore_checkpoint".to_string(),
                reasons: plan.blockers.clone(),
            },
            format!(
                "Restore refused, nothing written: {}",
                plan.blockers.join("; ")
            ),
        );
    }
    if changes.is_empty() {
        return CallToolResult::json(&json!({
            "dry_run": false,
            "checkpoint_id": manifest.id,
            "restored": [],
            "unchanged": unchanged,
            "added_since": plan.added_since,
            "note": "The saved files already match the checkpoint; nothing was written.",
        }));
    }

    // The undo point is the project as it stands now, and must be exactly the
    // state the preview was computed on.
    let undo = match create_checkpoint_in(
        project,
        Some(format!("before restoring {}", manifest.id)),
        "pre_restore",
    ) {
        Ok((undo, _)) => undo,
        Err(error) => {
            let reason = match error {
                CreateError::Capture(reason) | CreateError::Store(reason) => reason,
                CreateError::Changed => "project_changed_during_checkpoint".to_string(),
            };
            return CallToolResult::error_kind(
                ToolErrorKind::PlanBlocked {
                    operation: "restore_checkpoint".to_string(),
                    reasons: vec![format!("undo_checkpoint_failed: {reason}")],
                },
                format!("Restore refused, nothing written: the pre-restore checkpoint failed ({reason})"),
            );
        }
    };
    let undo_digests: BTreeMap<&str, &str> = undo
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.sha256.as_str()))
        .collect();
    let drifted = changes.iter().any(|entry| {
        undo_digests.get(entry.path.as_str()).copied() != entry.current_sha.as_deref()
    });
    if drifted {
        return CallToolResult::error_kind(
            ToolErrorKind::StaleTarget {
                target: project.file.display().to_string(),
                reason: "revision_mismatch".to_string(),
            },
            format!(
                "The project changed while the restore was being prepared. Nothing was restored; \
                 checkpoint {} holds the state before this call. Preview again.",
                undo.id
            ),
        );
    }

    let mut transitions = Vec::with_capacity(changes.len());
    for entry in &changes {
        let absolute = join_relative(&project.dir, &entry.path);
        let content = stored[&entry.path].clone();
        match &entry.current {
            Some(current) => transitions.push(konnect_sexp::FileTransition::replace(
                absolute,
                current.clone(),
                content,
            )),
            None => {
                if let Some(parent) = absolute.parent() {
                    if let Err(error) = std::fs::create_dir_all(parent) {
                        return CallToolResult::error_kind(
                            ToolErrorKind::PlanBlocked {
                                operation: "restore_checkpoint".to_string(),
                                reasons: vec![format!("cannot_create_folder: {}: {error}", entry.path)],
                            },
                            format!("Restore refused, nothing restored: cannot create the folder for {}", entry.path),
                        );
                    }
                }
                transitions.push(konnect_sexp::FileTransition::create(absolute, content));
            }
        }
    }

    if let Err(error) = konnect_sexp::commit_file_transaction(&project.dir, transitions) {
        // The transaction rechecks KiCad's lock per file, so a lock or conflict
        // can follow a partial write. "Nothing written" is claimed only on
        // evidence: every target reads as before and no journal was left.
        let untouched = changes.iter().all(|entry| {
            let now = std::fs::read(join_relative(&project.dir, &entry.path))
                .ok()
                .map(|bytes| sha256_hex(&bytes));
            now == entry.current_sha
        }) && konnect_sexp::inspect_file_transactions(&project.dir)
            .is_ok_and(|pending| pending.is_empty());
        let refused = |kind: ToolErrorKind, why: String| {
            CallToolResult::error_kind(
                kind,
                format!(
                    "Restore refused, nothing written: {why}. Checkpoint {} holds the state \
                     before this call.",
                    undo.id
                ),
            )
        };
        return match error {
            konnect_sexp::SexpError::KiCadEditorLocked { path, .. } if untouched => refused(
                ToolErrorKind::PlanBlocked {
                    operation: "restore_checkpoint".to_string(),
                    reasons: vec![format!(
                        "kicad_lock_present: {}",
                        display_relative(&project.dir, &path)
                    )],
                },
                format!("KiCad opened {} during the restore", path.display()),
            ),
            konnect_sexp::SexpError::Conflict { .. }
            | konnect_sexp::SexpError::TransactionConflict { .. }
                if untouched =>
            {
                refused(
                    ToolErrorKind::StaleTarget {
                        target: project.file.display().to_string(),
                        reason: "revision_mismatch".to_string(),
                    },
                    "a file changed during the restore; preview again".to_string(),
                )
            }
            other => CallToolResult::error_kind(
                ToolErrorKind::MutationOutcomeUncertain {
                    operation: "restore_checkpoint".to_string(),
                    path: project.dir.display().to_string(),
                    reason: format!("{other}; undo checkpoint {}", undo.id),
                },
                format!(
                    "The restore did not finish cleanly ({other}). Some files may be restored. \
                     Run get_edit_readiness and `konnect transaction status <project-dir>`; \
                     checkpoint {} holds the state before this call.",
                    undo.id
                ),
            ),
        };
    }

    let mut mismatched = Vec::new();
    for entry in &changes {
        let absolute = join_relative(&project.dir, &entry.path);
        let matches = std::fs::read(&absolute)
            .map(|bytes| sha256_hex(&bytes))
            .ok()
            .as_deref()
            == manifest
                .files
                .iter()
                .find(|f| f.path == entry.path)
                .map(|f| f.sha256.as_str());
        if !matches {
            mismatched.push(entry.path.clone());
        }
    }
    if !mismatched.is_empty() {
        return CallToolResult::error_kind(
            ToolErrorKind::MutationOutcomeUncertain {
                operation: "restore_checkpoint".to_string(),
                path: project.dir.display().to_string(),
                reason: format!(
                    "readback differs for {}; undo checkpoint {}",
                    mismatched.join(", "),
                    undo.id
                ),
            },
            format!(
                "The restore committed, but {} did not read back as the checkpoint. Another \
                 writer may have changed them. Checkpoint {} holds the state before this call.",
                mismatched.join(", "),
                undo.id
            ),
        );
    }

    CallToolResult::json(&json!({
        "dry_run": false,
        "checkpoint_id": manifest.id,
        "restored": changes,
        "unchanged": unchanged,
        "added_since": plan.added_since,
        "undo_checkpoint": undo.id,
        "verified": true,
        "next": format!(
            "Restored files were read back. To undo, call restore_checkpoint with checkpoint_id {}.",
            undo.id
        ),
    }))
}

async fn handle_get_edit_readiness(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let project = match resolve_project(args) {
        Ok(project) => project,
        Err(error) => return Ok(error),
    };
    let timeout = match ipc_timeout(args) {
        Ok(timeout) => timeout,
        Err(error) => return Ok(error),
    };
    let ipc = observe_ipc(ctx, timeout).await;
    let open = ipc
        .observation
        .as_ref()
        .map(|observation| open_in_kicad(&project, observation));
    let task_project = project.clone();
    let board_session = ctx.board_session.clone();
    let report = tokio::task::spawn_blocking(move || {
        readiness_report(
            &task_project,
            open.as_ref(),
            &|board| board_session.was_observed_live(board),
            &|board| board_session.authorize_file_fallback(board),
        )
    })
    .await?;
    let mut report = match report {
        Ok(report) => report,
        Err(reason) => {
            return Ok(CallToolResult::error_kind(
                ToolErrorKind::HandlerError {
                    reason: reason.clone(),
                },
                format!("Cannot read the project folder: {reason}"),
            ))
        }
    };
    let ipc_reachable = ipc.status == "reachable";
    report["kicad"]["ipc"] = json!(ipc.status);
    report["kicad"]["ipc_detail"] = ipc.detail;
    if ipc.status == "reachable_without_documents" {
        report["guidance"]
            .as_array_mut()
            .expect("guidance is an array")
            .insert(
                0,
                json!(
                    "KiCad answered over IPC but did not report its open documents, so \
                     open_in_kicad is unknown (null) and ownership below comes from KiCad's \
                     lock files only."
                ),
            );
    } else if !ipc_reachable {
        report["guidance"]
            .as_array_mut()
            .expect("guidance is an array")
            .insert(
                0,
                json!(format!(
                    "KiCad is not answering over IPC ({}). Live tools (save_project, editor \
                     navigation and selection, live board edits) will fail until KiCad runs with \
                     its API enabled; do not retry them this session without checking again. \
                     File edits work on the saved files.",
                    ipc.status
                )),
            );
    }
    Ok(CallToolResult::json(&report))
}

fn readiness_report(
    project: &Project,
    open: Option<&OpenInKicad>,
    observed_live: &dyn Fn(&Path) -> bool,
    authorize_file_fallback: &dyn Fn(&Path) -> bool,
) -> Result<Value, String> {
    let scan = scan_project(&project.dir).map_err(|error| error.to_string())?;
    let project_lock = project_lock(project);
    let (pending, journal_error) = match konnect_sexp::inspect_file_transactions(&project.dir) {
        Ok(pending) => (pending, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    let mut pending_by_path: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for transaction in &pending {
        for target in &transaction.targets {
            let path = target
                .path
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            pending_by_path
                .entry(path)
                .or_default()
                .push(transaction.id.clone());
        }
    }

    let editor_holding_sheets = schematic_editor_evidence(project, open);
    let mut rows = Vec::with_capacity(scan.files.len());
    let mut counts = BTreeMap::from([("file", 0usize), ("live", 0), ("blocked", 0)]);
    let mut blocked_files = Vec::new();
    for (path, absolute) in &scan.files {
        let kind = file_kind(path);
        let mut blocked_by = Vec::new();
        let mut warnings = Vec::new();
        let document = kind == "schematic" || kind == "board";
        let lock = if document {
            editor_lock_state(absolute)
        } else {
            LockState::Absent
        };
        let open_now = open.map(|open| match kind {
            "board" => open.boards.iter().any(|board| same_path(board, absolute)),
            // KiCad 10 names a schematic by sheet, not file, so an open
            // schematic editor is attributed to every sheet of the project.
            "schematic" => open.schematic_open,
            _ => false,
        });
        let seen_live = kind == "board" && observed_live(absolute);
        let mut route = "file";

        if let Some(ids) = pending_by_path.get(path) {
            blocked_by.push(format!("pending_transaction: {}", ids.join(", ")));
        }
        if journal_error.is_some() && document {
            blocked_by.push("transaction_journal_unreadable".to_string());
        }
        match (&lock, kind, open_now) {
            (LockState::Present(_), "board", Some(true)) => route = "live",
            (LockState::Present(_), _, _) => blocked_by.push(
                "kicad_lock_present: open in KiCad (or KiCad exited without closing it); save and close it there"
                    .to_string(),
            ),
            (LockState::Unreadable(_, error), _, _) => {
                blocked_by.push(format!("kicad_lock_unreadable: {error}"))
            }
            (LockState::Absent, "board", Some(true)) => route = "live",
            (LockState::Absent, "schematic", _) => {
                if let Some(evidence) = &editor_holding_sheets {
                    blocked_by.push(format!(
                        "kicad_schematic_open: {evidence}; KiCad locks only the root sheet, so this sheet is open too"
                    ));
                }
            }
            (LockState::Absent, ..) => {}
        }
        if kind == "board" && route != "live" && seen_live && !authorize_file_fallback(absolute) {
            blocked_by.push(
                "board_observed_live: open in KiCad earlier this session; its saved file may be stale"
                    .to_string(),
            );
        }
        if !matches!(project_lock, LockState::Absent) && !document {
            warnings.push(
                "kicad_project_open: KiCad may rewrite this file when it saves or closes the project"
                    .to_string(),
            );
        }
        if !blocked_by.is_empty() {
            route = "blocked";
            blocked_files.push(path.clone());
        }
        *counts.get_mut(route).expect("known route") += 1;
        rows.push(json!({
            "path": path,
            "kind": kind,
            "lock": if document { json!(lock.label()) } else { Value::Null },
            "open_in_kicad": open_now,
            "observed_live_this_session": if kind == "board" { json!(seen_live) } else { Value::Null },
            "edit_route": route,
            "blocked_by": blocked_by,
            "warnings": warnings,
        }));
    }

    let mut guidance = Vec::new();
    if !blocked_files.is_empty() {
        guidance.push(format!(
            "Blocked: {}. Save and close them in KiCad, then call get_edit_readiness again. \
             Konnect never deletes KiCad lock files: they carry no process id, so a stale \
             lock cannot be told from a live one. If KiCad is not running and a lock remains, \
             the user decides whether to remove it.",
            blocked_files.join(", ")
        ));
    }
    if !pending.is_empty() {
        guidance.push(
            "A Konnect write journal is pending. Inspect it with `konnect transaction status \
             <project-dir>` and recover it before editing."
                .to_string(),
        );
    }
    if counts["live"] > 0 {
        guidance.push(
            "Boards with edit_route 'live' are open in KiCad: Konnect's board tools edit them \
             through IPC, and save_project writes them."
                .to_string(),
        );
    }
    // A pending journal is finished by the next Konnect write to this project,
    // whatever file that write names, so it blocks the project as a whole.
    let verdict = if counts["blocked"] > 0 || !pending.is_empty() || journal_error.is_some() {
        "blocked"
    } else if counts["live"] > 0 {
        "live_editing"
    } else {
        "ready"
    };
    Ok(json!({
        "project": project.file.display().to_string(),
        "verdict": verdict,
        "summary": {
            "files": rows.len(),
            "file": counts["file"],
            "live": counts["live"],
            "blocked": counts["blocked"],
        },
        "kicad": {
            "project_lock": project_lock.label(),
            "project_open": open.map(|open| open.project_open),
        },
        "pending_transactions": pending
            .iter()
            .map(|t| json!({ "id": t.id, "targets": t.targets.len() }))
            .collect::<Vec<_>>(),
        "transaction_journal_error": journal_error,
        "files": rows,
        "skipped": scan.skipped,
        "guidance": guidance,
    }))
}

#[cfg(test)]
#[path = "checkpoints_tests.rs"]
mod tests;
