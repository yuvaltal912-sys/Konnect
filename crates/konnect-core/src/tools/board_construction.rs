//! Board construction (creation research rank 21: A13, B25).
//!
//! Four tools in the `pcb_board` toolset that edit the saved board file:
//!
//! * `set_footprint_lock` reads or sets the lock flag of placed footprints.
//! * `add_rule_area` adds a KiCad rule area (keepout zone) with the five
//!   keepout flags. A rule area that forbids tracks or vias is exported by
//!   `export_specctra_dsn` as a keepout, so Freerouting respects it.
//! * `list_zones` lists copper zones and rule areas with their UUIDs.
//! * `delete_zone` deletes zones or rule areas by UUID or name.
//!
//! None of them has a live IPC path. Every write refuses while KiCad holds the
//! board open or its sibling lock is present (`refuse_if_board_open_in_kicad`),
//! previews by default, applies only with the preview's `revision` (which binds
//! the board path, the saved bytes and the request), writes conditionally on the
//! bytes it read, and reads the file back before reporting success.
//!
//! The polygon board outline is `add_board_outline`'s `points` mode, in
//! `pcb_board.rs`; its validation lives here beside the rule-area polygon's.

use super::refuse_if_board_open_in_kicad;
use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::CallToolResult;
use crate::tool;
use crate::tools::{get_path, invalid_arg, ToolContext, ToolDef};
use konnect_sexp::parser::{parse_sexp, SexpNode};
use konnect_sexp::writer::{
    apply_edits, find_block_with_leading_whitespace, find_direct_child_blocks, new_uuid,
    read_consistent, write_atomic_if_unchanged, SexpEdit,
};
use konnect_sexp::SexpError;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The five keepout flags of a KiCad rule area, in the order KiCad writes them.
pub(crate) const KEEPOUT_FLAGS: [&str; 5] = ["tracks", "vias", "pads", "copperpour", "footprints"];

/// What a new rule area forbids when the caller does not say: what KiCad 10's
/// own new rule area starts with. Tracks, vias and pads are forbidden; copper
/// pours and footprints are allowed. Source: the `ZONE_SETTINGS` constructor in
/// KiCad 10.0.1's `pcbnew/zone_settings.cpp`, which the rule-area dialog opens
/// from through the board's default zone settings
/// (`pcbnew/tools/zone_create_helper.cpp`). KiCad then remembers the last zone
/// settings used in a session; a file has no such memory, so this starts from
/// the constructor's values.
const DEFAULT_FORBIDDEN: [bool; 5] = [true, true, true, false, false];

pub(crate) fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "set_footprint_lock",
            "Read or set the lock flag of placed footprints in the saved board file. Without \
             'locked' it only reports each footprint's lock state (all footprints, or those in \
             'references'). With 'locked' it previews the change and returns a 'revision'; call \
             again with the same arguments plus expected_revision to apply. Applying refuses \
             while KiCad holds the board open or its lock file is present, writes only if the \
             file still matches the preview, and reads the file back. Of Konnect's placement tools, only \
             auto_place_from_schematic and refine_placement_force_directed hold a locked \
             footprint where it is, and update_pcb_from_schematic keeps a locked orphan \
             footprint instead of deleting it. move_component, rotate_component, flip_component, \
             set_component_placements, align_components and place_decoupling_caps do not check \
             the lock, so keep locked references out of them. Routing never moves a footprint: \
             apply_specctra_ses refuses a session that changes any placement.",
            json!({
                "type": "object",
                "properties": {
                    "board": { "type": "string", "description": "Path to the .kicad_pcb file" },
                    "references": {
                        "type": "array", "items": { "type": "string" },
                        "description": "Reference designators. Required with 'locked'; omit to report every footprint."
                    },
                    "locked": { "type": "boolean", "description": "true locks, false unlocks. Omit to only read." },
                    "expected_revision": { "type": "string", "description": "The preview's revision; applies the change" }
                },
                "required": ["board"]
            }),
            |args, ctx| async move { handle_set_footprint_lock(args, ctx).await }
        )
        .with_board_access(crate::tools::BoardAccess::ApplyModeDependent),
        tool!(
            "add_rule_area",
            "Add a KiCad rule area (keepout zone) to the saved board file: a rectangle \
             (x1, y1, x2, y2) or a simple polygon ('points'), on one or more copper layers. \
             'keepout' says what is forbidden inside. The default is KiCad's own for a new rule \
             area: tracks, vias and pads forbidden, copper pours and footprints allowed; an \
             antenna keepout usually also sets copperpour true. A rule area that forbids \
             tracks or vias is exported by export_specctra_dsn as a keepout, so Freerouting \
             routes around it. Previews by default and returns a 'revision'; call again with \
             the same arguments plus expected_revision to apply. Applying refuses while KiCad \
             holds the board open or its lock file is present, writes only if the file still \
             matches the preview, and reads the file back.",
            json!({
                "type": "object",
                "properties": {
                    "board":  { "type": "string", "description": "Path to the .kicad_pcb file" },
                    "layers": {
                        "type": "array", "items": { "type": "string" },
                        "description": "Copper layers, e.g. ['F.Cu'] or ['F.Cu','B.Cu']; '*.Cu' means every copper layer of the board"
                    },
                    "x1": { "type": "number", "description": "Rectangle corner X in mm" },
                    "y1": { "type": "number", "description": "Rectangle corner Y in mm" },
                    "x2": { "type": "number", "description": "Opposite corner X in mm" },
                    "y2": { "type": "number", "description": "Opposite corner Y in mm" },
                    "points": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": { "x": { "type": "number" }, "y": { "type": "number" } },
                            "required": ["x", "y"]
                        },
                        "description": "Polygon corners in mm, in order, at least 3; the edges must not cross. Instead of x1..y2."
                    },
                    "name": { "type": "string", "description": "Rule-area name; must be unique among the board's zones" },
                    "keepout": {
                        "type": "object",
                        "properties": {
                            "tracks":     { "type": "boolean", "description": "Forbid tracks (default true)" },
                            "vias":       { "type": "boolean", "description": "Forbid vias (default true)" },
                            "pads":       { "type": "boolean", "description": "Forbid pads (default true)" },
                            "copperpour": { "type": "boolean", "description": "Forbid copper pours (default false)" },
                            "footprints": { "type": "boolean", "description": "Forbid footprints (default false)" }
                        },
                        "description": "true forbids the item inside the area. At least one must be forbidden."
                    },
                    "expected_revision": { "type": "string", "description": "The preview's revision; applies the change" }
                },
                "required": ["board", "layers"]
            }),
            |args, ctx| async move { handle_add_rule_area(args, ctx).await }
        )
        .with_board_access(crate::tools::BoardAccess::ClosedBoardOnly),
        tool!(
            "list_zones",
            "List the board's copper zones and rule areas (keepouts) from the saved file: UUID, \
             name, kind ('copper' or 'rule_area'), net, layers, keepout flags, priority, corner \
             count and bounding box. Zones inside footprints are not listed. Use the UUIDs with \
             delete_zone.",
            json!({
                "type": "object",
                "properties": {
                    "board": { "type": "string", "description": "Path to the .kicad_pcb file" },
                    "kind":  { "type": "string", "enum": ["copper", "rule_area"], "description": "Only this kind" }
                },
                "required": ["board"]
            }),
            |args, _ctx| async move { handle_list_zones(args).await }
        ),
        tool!(
            "delete_zone",
            "Delete copper zones or rule areas from the saved board file, by UUID ('uuids', \
             from list_zones) or by name ('names'). A name that matches more than one zone is \
             refused; use UUIDs. A UUID that more than one zone carries is refused too, whether \
             it was named or the zone was picked by name. Zones inside footprints are not \
             touched. Previews by default and returns a 'revision'; call again with the same \
             arguments plus expected_revision to apply. Applying refuses while KiCad holds the \
             board open or its lock file is present, writes only if the file still matches the \
             preview, and reads the file back. Run refill_zones afterwards if copper zones \
             remain.",
            json!({
                "type": "object",
                "properties": {
                    "board": { "type": "string", "description": "Path to the .kicad_pcb file" },
                    "uuids": { "type": "array", "items": { "type": "string" }, "description": "Zone UUIDs" },
                    "names": { "type": "array", "items": { "type": "string" }, "description": "Zone names" },
                    "expected_revision": { "type": "string", "description": "The preview's revision; applies the change" }
                },
                "required": ["board"]
            }),
            |args, ctx| async move { handle_delete_zone(args, ctx).await }
        )
        .with_board_access(crate::tools::BoardAccess::ClosedBoardOnly),
    ]
}

// ─── Shared plumbing ──────────────────────────────────────────────────────────

/// One top-level item of the board file with its byte span.
struct Block {
    start: usize,
    end: usize,
    node: SexpNode,
}

/// Every direct child of `(kicad_pcb …)`, parsed one at a time so a span can
/// be edited without reformatting the rest of the file.
fn top_level_blocks(content: &str) -> Result<Vec<Block>, String> {
    let root = parse_sexp(content).map_err(|e| format!("the board file does not parse: {e}"))?;
    if root.head() != Some("kicad_pcb") {
        return Err("the file's root is not (kicad_pcb …)".to_string());
    }
    find_direct_child_blocks(content, "kicad_pcb")
        .into_iter()
        .map(|(start, end)| {
            parse_sexp(&content[start..end])
                .map(|node| Block { start, end, node })
                .map_err(|e| format!("a board item at byte {start} does not parse: {e}"))
        })
        .collect()
}

/// `sha256:` over the operation, the board path, the saved bytes and the
/// normalised request. A preview's revision is valid for exactly one apply of
/// the same request against the same saved file.
fn revision(operation: &str, board: &Path, content: &str, request: &Value) -> String {
    let mut hasher = Sha256::new();
    for part in [
        operation,
        &board.display().to_string(),
        content,
        &request.to_string(),
    ] {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("sha256:{hex}")
}

fn expected_revision(args: &Value) -> Result<Option<String>, CallToolResult> {
    match args.get("expected_revision") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if !text.is_empty() => Ok(Some(text.clone())),
        Some(_) => Err(invalid_arg(
            "expected_revision",
            "must be the non-empty revision a preview returned",
        )),
    }
}

/// The refusal for a revision that does not match. It carries no revision: the
/// current one would authorise exactly this request on the current file, which
/// nobody has previewed, so the caller has to preview again to get it (as
/// `restore_checkpoint` does).
fn stale(board: &Path) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::StaleTarget {
            target: board.display().to_string(),
            reason: "revision_mismatch".to_string(),
        },
        "The board file or the request changed since the preview, so nothing was written. \
         Preview again (omit expected_revision) to see what would change, and apply with the \
         revision that preview returns.",
    )
}

fn unreadable_board(board: &Path, reason: &str) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::InvalidConfiguration {
            path: board.display().to_string(),
            reason: reason.to_string(),
        },
        format!("Cannot edit '{}': {reason}", board.display()),
    )
}

fn uncertain(board: &Path, operation: &str, reason: &str) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::MutationOutcomeUncertain {
            operation: operation.to_string(),
            path: board.display().to_string(),
            reason: reason.to_string(),
        },
        format!(
            "{operation} wrote '{}', but reading it back did not prove the requested result: \
             {reason}. Inspect the board before retrying.",
            board.display()
        ),
    )
}

fn read_board(board: &Path) -> Result<String, CallToolResult> {
    read_consistent(board).map_err(|error| match error {
        SexpError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
            CallToolResult::error_kind(
                ToolErrorKind::FileNotFound {
                    path: board.display().to_string(),
                },
                format!("Board file '{}' does not exist", board.display()),
            )
        }
        other => unreadable_board(board, &other.to_string()),
    })
}

/// Write `next` only if the file still holds `before`, then return the bytes
/// now on disk for the caller's semantic readback.
fn commit(
    board: &Path,
    before: &str,
    next: &str,
    operation: &str,
) -> Result<String, CallToolResult> {
    match write_atomic_if_unchanged(board, before, next) {
        Ok(()) => {}
        Err(SexpError::Conflict { .. }) => {
            let actual = std::fs::read_to_string(board).unwrap_or_default();
            if actual == before {
                return Err(uncertain(board, operation, "the write was not confirmed"));
            }
            return Err(CallToolResult::error_kind(
                ToolErrorKind::StaleTarget {
                    target: board.display().to_string(),
                    reason: "the file changed while the edit was being written".to_string(),
                },
                "The board file changed between the read and the write, so nothing was \
                 written. Preview again and apply with the new revision.",
            ));
        }
        Err(error @ SexpError::KiCadEditorLocked { .. }) => {
            return Err(CallToolResult::error_kind(
                ToolErrorKind::UnsafeFileFallback {
                    path: board.display().to_string(),
                    reason: "kicad_lock_present".to_string(),
                },
                format!(
                    "{error}. KiCad may hold newer unsaved state, so nothing was written. Close \
                     the board in KiCad and retry."
                ),
            ));
        }
        Err(other) => return Err(unreadable_board(board, &other.to_string())),
    }
    std::fs::read_to_string(board)
        .map_err(|e| uncertain(board, operation, &format!("readback failed: {e}")))
}

fn string_list(args: &Value, field: &str) -> Result<Option<Vec<String>>, CallToolResult> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item.as_str() {
                Some(text) if !text.trim().is_empty() => Ok(text.to_string()),
                _ => Err(invalid_arg(field, "every entry must be a non-empty string")),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(invalid_arg(field, "must be an array of strings")),
    }
}

fn node_uuid(node: &SexpNode) -> Option<String> {
    node.find_str("uuid")
        .or_else(|| node.find_str("tstamp"))
        .map(str::to_string)
}

// ─── Footprint lock ───────────────────────────────────────────────────────────

/// KiCad 7+ writes `(locked yes)`; KiCad 6 wrote a bare `locked` token after
/// the library id. Both read as locked; `(locked no)` reads as unlocked.
fn footprint_locked(node: &SexpNode) -> bool {
    let children = node.children().unwrap_or(&[]);
    children.iter().skip(1).any(|child| match child {
        SexpNode::Atom(atom) => atom == "locked",
        SexpNode::List(_) if child.head() == Some("locked") => {
            !matches!(child.get(1).and_then(SexpNode::as_str), Some("no"))
        }
        _ => false,
    })
}

/// Byte offset just past the footprint's library id string, inside `block`.
fn after_library_id(block: &str) -> Option<usize> {
    let bytes = block.as_bytes();
    let mut i = block.find("(footprint")? + "(footprint".len();
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if bytes.get(i) == Some(&b'"') {
        i += 1;
        let mut escape = false;
        while i < bytes.len() {
            match bytes[i] {
                _ if escape => escape = false,
                b'\\' => escape = true,
                b'"' => return Some(i + 1),
                _ => {}
            }
            i += 1;
        }
        None
    } else {
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'('
            && bytes[i] != b')'
        {
            i += 1;
        }
        Some(i)
    }
}

/// The edits, relative to `block`, that make the footprint's lock flag
/// `locked`. Empty when it already is.
fn lock_edits(block: &str, node: &SexpNode, locked: bool) -> Result<Vec<SexpEdit>, String> {
    if footprint_locked(node) == locked {
        return Ok(Vec::new());
    }
    let name_end = after_library_id(block).ok_or("the footprint has no library id")?;
    let mut edits = Vec::new();
    // Every `(locked …)` child goes: a `(locked no)` when locking, a
    // `(locked yes)` when unlocking. One `(locked yes)` is written back when
    // locking.
    for (start, end) in find_direct_child_blocks(block, "footprint") {
        let child = &block[start..end];
        if child == "(locked)" || child.starts_with("(locked ") || child.starts_with("(locked\t") {
            let (ws_start, ws_end) =
                find_block_with_leading_whitespace(block, start).unwrap_or((start, end));
            edits.push(SexpEdit::delete(ws_start, ws_end));
        }
    }
    // The KiCad 6 bare token sits between the library id and the first child.
    let header_end = block[name_end..]
        .find('(')
        .map(|offset| name_end + offset)
        .unwrap_or(block.len());
    let header = &block[name_end..header_end];
    let mut search = 0;
    while let Some(found) = header[search..].find("locked") {
        let at = search + found;
        let before_ok = at == 0 || header.as_bytes()[at - 1].is_ascii_whitespace();
        let after = at + "locked".len();
        let after_ok = after == header.len() || header.as_bytes()[after].is_ascii_whitespace();
        if before_ok && after_ok {
            let mut ws = at;
            while ws > 0 && header.as_bytes()[ws - 1].is_ascii_whitespace() {
                ws -= 1;
            }
            edits.push(SexpEdit::delete(name_end + ws, name_end + after));
        }
        search = after;
    }
    if locked {
        edits.push(SexpEdit::insert(name_end, " (locked yes)"));
    }
    Ok(edits)
}

struct FootprintRow<'a> {
    block: &'a Block,
    reference: String,
}

fn footprint_rows(blocks: &[Block]) -> Vec<FootprintRow<'_>> {
    blocks
        .iter()
        .filter(|block| block.node.head() == Some("footprint"))
        .map(|block| FootprintRow {
            block,
            reference: footprint_reference(&block.node).unwrap_or_default(),
        })
        .collect()
}

fn footprint_reference(fp: &SexpNode) -> Option<String> {
    for prop in fp.find_all("property") {
        if prop.get(1).and_then(SexpNode::as_str) == Some("Reference") {
            return prop.get(2).and_then(SexpNode::as_str).map(str::to_string);
        }
    }
    for text in fp.find_all("fp_text") {
        if text.get(1).and_then(SexpNode::as_str) == Some("reference") {
            return text.get(2).and_then(SexpNode::as_str).map(str::to_string);
        }
    }
    None
}

async fn handle_set_footprint_lock(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    const OPERATION: &str = "set_footprint_lock";
    let board = get_path(args, "board")?;
    let references = match string_list(args, "references") {
        Ok(v) => v,
        Err(e) => return Ok(e),
    };
    let locked = match args.get("locked") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(value)) => Some(*value),
        Some(_) => return Ok(invalid_arg("locked", "must be true or false")),
    };
    let expected = match expected_revision(args) {
        Ok(v) => v,
        Err(e) => return Ok(e),
    };

    let Some(locked) = locked else {
        if expected.is_some() {
            return Ok(invalid_arg(
                "expected_revision",
                "only applies with 'locked'; without it the call only reads",
            ));
        }
        return read_footprint_locks(&board, references.as_deref());
    };
    let Some(references) = references.filter(|refs| !refs.is_empty()) else {
        return Ok(invalid_arg(
            "references",
            "name the footprints to lock or unlock; it is required with 'locked'",
        ));
    };

    if let Some(refusal) = refuse_if_board_open_in_kicad(ctx, &board, "footprint lock").await? {
        return Ok(refusal);
    }
    let content = match read_board(&board) {
        Ok(c) => c,
        Err(e) => return Ok(e),
    };
    let blocks = match top_level_blocks(&content) {
        Ok(b) => b,
        Err(reason) => return Ok(unreadable_board(&board, &reason)),
    };
    let rows = footprint_rows(&blocks);

    let mut wanted: Vec<String> = Vec::new();
    for reference in references {
        if !wanted.contains(&reference) {
            wanted.push(reference);
        }
    }
    let mut unknown = Vec::new();
    let mut ambiguous = Vec::new();
    let mut targets = Vec::new();
    for reference in &wanted {
        let matches: Vec<&FootprintRow> =
            rows.iter().filter(|r| &r.reference == reference).collect();
        match matches.as_slice() {
            [] => unknown.push(reference.clone()),
            [one] => targets.push(*one),
            _ => ambiguous.push(json!({ "reference": reference, "count": matches.len() })),
        }
    }
    if !unknown.is_empty() {
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::InvalidArgument {
                field: "references".to_string(),
                reason: format!("not on the board: {}", unknown.join(", ")),
            },
            format!(
                "No footprint on the board has the reference {}. Nothing was changed.",
                unknown.join(", ")
            ),
        ));
    }
    if !ambiguous.is_empty() {
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::AmbiguousTarget {
                target: "references".to_string(),
                candidates: ambiguous.iter().map(|a| a.to_string()).collect(),
            },
            format!(
                "More than one footprint carries the same reference ({}); annotate the board \
                 first. Nothing was changed.",
                ambiguous
                    .iter()
                    .filter_map(|a| a["reference"].as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }

    let request = json!({ "references": wanted, "locked": locked });
    let current = revision(OPERATION, &board, &content, &request);
    let mut edits = Vec::new();
    let mut changes = Vec::new();
    for target in &targets {
        let block = &content[target.block.start..target.block.end];
        let local = match lock_edits(block, &target.block.node, locked) {
            Ok(e) => e,
            Err(reason) => {
                return Ok(unreadable_board(
                    &board,
                    &format!("{}: {reason}", target.reference),
                ))
            }
        };
        changes.push(json!({
            "reference": target.reference,
            "uuid": node_uuid(&target.block.node),
            "locked_before": footprint_locked(&target.block.node),
            "locked_after": locked,
            "changed": !local.is_empty(),
        }));
        edits.extend(local.into_iter().map(|edit| SexpEdit {
            start: edit.start + target.block.start,
            end: edit.end + target.block.start,
            replacement: edit.replacement,
        }));
    }
    let changed_count = changes.iter().filter(|c| c["changed"] == true).count();

    let Some(expected) = expected else {
        return Ok(CallToolResult::json(&json!({
            "mode": "preview",
            "board": board,
            "revision": current,
            "footprints": changes,
            "would_change": changed_count,
            "next": "Call again with the same arguments and expected_revision to apply.",
        })));
    };
    if expected != current {
        return Ok(stale(&board));
    }
    if changed_count == 0 {
        return Ok(CallToolResult::json(&json!({
            "mode": "applied", "board": board, "footprints": changes, "changed": 0,
            "written": false, "source": "file",
        })));
    }

    let next = apply_edits(content.clone(), edits);
    let written = match commit(&board, &content, &next, OPERATION) {
        Ok(w) => w,
        Err(e) => return Ok(e),
    };
    // Readback: every target now carries the requested flag, and nothing but
    // the targets changed.
    let after = match top_level_blocks(&written) {
        Ok(b) => b,
        Err(reason) => return Ok(uncertain(&board, OPERATION, &reason)),
    };
    let after_rows = footprint_rows(&after);
    for reference in &wanted {
        let state = after_rows
            .iter()
            .find(|r| &r.reference == reference)
            .map(|r| footprint_locked(&r.block.node));
        if state != Some(locked) {
            return Ok(uncertain(
                &board,
                OPERATION,
                &format!("{reference} does not read back as locked={locked}"),
            ));
        }
    }
    if after.len() != blocks.len() {
        return Ok(uncertain(
            &board,
            OPERATION,
            "the number of board items changed",
        ));
    }
    Ok(CallToolResult::json(&json!({
        "mode": "applied",
        "board": board,
        "footprints": changes,
        "changed": changed_count,
        "written": true,
        "source": "file",
        "readback": "verified",
        "note": "Reload the board in KiCad before editing it there.",
    })))
}

fn read_footprint_locks(
    board: &Path,
    references: Option<&[String]>,
) -> anyhow::Result<CallToolResult> {
    let content = match read_board(board) {
        Ok(c) => c,
        Err(e) => return Ok(e),
    };
    let blocks = match top_level_blocks(&content) {
        Ok(b) => b,
        Err(reason) => return Ok(unreadable_board(board, &reason)),
    };
    let rows = footprint_rows(&blocks);
    if let Some(references) = references {
        let unknown: Vec<&String> = references
            .iter()
            .filter(|reference| !rows.iter().any(|r| &&r.reference == reference))
            .collect();
        if !unknown.is_empty() {
            return Ok(invalid_arg(
                "references",
                &format!(
                    "not on the board: {}",
                    unknown
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
    }
    let footprints: Vec<Value> = rows
        .iter()
        .filter(|row| references.is_none_or(|refs| refs.contains(&row.reference)))
        .map(|row| {
            json!({
                "reference": row.reference,
                "uuid": node_uuid(&row.block.node),
                "locked": footprint_locked(&row.block.node),
            })
        })
        .collect();
    let locked_count = footprints.iter().filter(|f| f["locked"] == true).count();
    Ok(CallToolResult::json(&json!({
        "mode": "read",
        "board": board,
        "source": "file",
        "count": footprints.len(),
        "locked_count": locked_count,
        "footprints": footprints,
    })))
}

// ─── Polygons ─────────────────────────────────────────────────────────────────

fn cross(o: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

fn on_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> bool {
    p.0 >= a.0.min(b.0) - 1e-9
        && p.0 <= a.0.max(b.0) + 1e-9
        && p.1 >= a.1.min(b.1) - 1e-9
        && p.1 <= a.1.max(b.1) + 1e-9
}

/// Whether two closed segments share any point.
fn segments_touch(a1: (f64, f64), a2: (f64, f64), b1: (f64, f64), b2: (f64, f64)) -> bool {
    const EPS: f64 = 1e-9;
    let d1 = cross(b1, b2, a1);
    let d2 = cross(b1, b2, a2);
    let d3 = cross(a1, a2, b1);
    let d4 = cross(a1, a2, b2);
    if ((d1 > EPS && d2 < -EPS) || (d1 < -EPS && d2 > EPS))
        && ((d3 > EPS && d4 < -EPS) || (d3 < -EPS && d4 > EPS))
    {
        return true;
    }
    (d1.abs() <= EPS && on_segment(a1, b1, b2))
        || (d2.abs() <= EPS && on_segment(a2, b1, b2))
        || (d3.abs() <= EPS && on_segment(b1, a1, a2))
        || (d4.abs() <= EPS && on_segment(b2, a1, a2))
}

/// Shoelace area of a closed ring (first point not repeated).
fn ring_area(ring: &[(f64, f64)]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum::<f64>()
        / 2.0
}

/// Check that `ring` (first point not repeated) is a simple polygon of
/// positive area: no repeated corner, no edge crossing or touching another
/// except its neighbours at their shared corner.
pub(crate) fn check_simple_ring(ring: &[(f64, f64)]) -> Result<(), String> {
    if ring.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
        return Err("every coordinate must be finite".to_string());
    }
    if ring.len() < 3 {
        return Err(format!(
            "needs at least 3 distinct corners, got {}",
            ring.len()
        ));
    }
    let n = ring.len();
    for i in 0..n {
        for j in i + 1..n {
            if (ring[i].0 - ring[j].0).abs() < 1e-9 && (ring[i].1 - ring[j].1).abs() < 1e-9 {
                return Err(format!("corners {i} and {j} are the same point"));
            }
        }
    }
    if ring_area(ring).abs() < 1e-9 {
        return Err("the corners enclose no area".to_string());
    }
    for i in 0..n {
        let (a1, a2) = (ring[i], ring[(i + 1) % n]);
        for j in i + 1..n {
            // Neighbouring edges share a corner by construction.
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            let (b1, b2) = (ring[j], ring[(j + 1) % n]);
            if segments_touch(a1, a2, b1, b2) {
                return Err(format!("edge {i} crosses or touches edge {j}"));
            }
        }
    }
    // Neighbouring edges that fold back over each other overlap.
    for i in 0..n {
        let (prev, here, next) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
        if cross(prev, here, next).abs() <= 1e-12 {
            let back =
                (prev.0 - here.0) * (next.0 - here.0) + (prev.1 - here.1) * (next.1 - here.1);
            if back > 0.0 {
                return Err(format!("the edges at corner {i} fold back over each other"));
            }
        }
    }
    Ok(())
}

/// How close two things must be, in mm, to count as touching.
const TOUCH_MM: f64 = 1e-9;

/// The circle and sweep of the arc from `start` through `mid` to `end`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArcGeom {
    center: (f64, f64),
    radius: f64,
    /// Angle of `start`, seen from the centre.
    a0: f64,
    /// Signed angle from `start` to `end`, the way that passes `mid`.
    sweep: f64,
}

impl ArcGeom {
    /// `None` when the three points are collinear or not finite.
    pub(crate) fn new(start: (f64, f64), mid: (f64, f64), end: (f64, f64)) -> Option<Self> {
        // Circumcentre, relative to `start` for precision far from the origin.
        let (bx, by) = (mid.0 - start.0, mid.1 - start.1);
        let (cx, cy) = (end.0 - start.0, end.1 - start.1);
        let d = 2.0 * (bx * cy - by * cx);
        if !d.is_finite() || d == 0.0 {
            return None;
        }
        let (b2, c2) = (bx * bx + by * by, cx * cx + cy * cy);
        let (ux, uy) = ((cy * b2 - by * c2) / d, (bx * c2 - cx * b2) / d);
        let center = (start.0 + ux, start.1 + uy);
        let radius = ux.hypot(uy);
        if !center.0.is_finite() || !center.1.is_finite() || !radius.is_finite() {
            return None;
        }
        let angle = |p: (f64, f64)| (p.1 - center.1).atan2(p.0 - center.0);
        let tau = std::f64::consts::TAU;
        let (a0, am, a1) = (angle(start), angle(mid), angle(end));
        // Sweep from start to end in the direction that passes the midpoint.
        let ccw = |from: f64, to: f64| (to - from).rem_euclid(tau);
        let sweep = if ccw(a0, am) < ccw(a0, a1) {
            ccw(a0, a1)
        } else {
            -(tau - ccw(a0, a1))
        };
        Some(Self {
            center,
            radius,
            a0,
            sweep,
        })
    }

    /// Whether `p`, a point on the arc's circle, lies on the arc, its ends
    /// included, give or take `TOUCH_MM` along the circle.
    fn covers(&self, p: (f64, f64)) -> bool {
        let tau = std::f64::consts::TAU;
        let theta = (p.1 - self.center.1).atan2(p.0 - self.center.0);
        let along = if self.sweep >= 0.0 {
            (theta - self.a0).rem_euclid(tau)
        } else {
            (self.a0 - theta).rem_euclid(tau)
        };
        let slack = TOUCH_MM / self.radius;
        along <= self.sweep.abs() + slack || along >= tau - slack
    }

    /// Bounding box `(x1, y1, x2, y2)`: the ends, and each point where the
    /// circle reaches furthest left, right, up or down, if the arc passes it.
    fn extent(&self, start: (f64, f64), end: (f64, f64)) -> (f64, f64, f64, f64) {
        let ((cx, cy), r) = (self.center, self.radius);
        [(cx + r, cy), (cx, cy + r), (cx - r, cy), (cx, cy - r)]
            .into_iter()
            .filter(|p| self.covers(*p))
            .chain([start, end])
            .fold(
                (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
                |(x1, y1, x2, y2), (x, y)| (x1.min(x), y1.min(y), x2.max(x), y2.max(y)),
            )
    }
}

/// Bounding box `(x1, y1, x2, y2)` of the arc from `start` through `mid` to
/// `end`, bulge included. Three collinear points give the box of the three.
pub(crate) fn arc_extent(
    start: (f64, f64),
    mid: (f64, f64),
    end: (f64, f64),
) -> (f64, f64, f64, f64) {
    match ArcGeom::new(start, mid, end) {
        Some(arc) => arc.extent(start, end),
        None => [start, mid, end].into_iter().fold(
            (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
            |(x1, y1, x2, y2), (x, y)| (x1.min(x), y1.min(y), x2.max(x), y2.max(y)),
        ),
    }
}

/// An outline corner, and the midpoint of the arc to the next corner, if any.
pub(crate) type OutlineCorner = ((f64, f64), Option<(f64, f64)>);

/// One edge of an outline.
enum Edge {
    Line((f64, f64), (f64, f64)),
    Arc(ArcGeom),
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Where the segment `a`–`b` meets the circle of `arc` (not yet limited to the
/// arc's sweep). `shared` holds the segment's ends that are also ends of the
/// arc: those are meetings by construction and are not returned, and the
/// other meeting is found from them exactly, so a tangent join is not lost to
/// rounding.
fn segment_circle_points(
    a: (f64, f64),
    b: (f64, f64),
    arc: &ArcGeom,
    shared: &[(f64, f64)],
) -> Vec<(f64, f64)> {
    let d = (b.0 - a.0, b.1 - a.1);
    let f = (a.0 - arc.center.0, a.1 - arc.center.1);
    // |a + t d - centre|^2 = r^2, as qa t^2 + qb t + qc = 0.
    let qa = d.0 * d.0 + d.1 * d.1;
    let qb = 2.0 * (f.0 * d.0 + f.1 * d.1);
    let qc = f.0 * f.0 + f.1 * f.1 - arc.radius * arc.radius;
    let roots: Vec<f64> = match (shared.contains(&a), shared.contains(&b)) {
        (true, true) => Vec::new(),
        // The roots sum to -qb/qa; one of them is the shared end.
        (true, false) => vec![-qb / qa],
        (false, true) => vec![-qb / qa - 1.0],
        (false, false) => {
            let disc = qb * qb - 4.0 * qa * qc;
            if disc < 0.0 {
                // No crossing; a near-tangent within TOUCH_MM still touches.
                let t = -qb / (2.0 * qa);
                let closest = (a.0 + t * d.0, a.1 + t * d.1);
                if (distance(closest, arc.center) - arc.radius).abs() <= TOUCH_MM {
                    vec![t]
                } else {
                    Vec::new()
                }
            } else {
                let q = -0.5 * (qb + qb.signum() * disc.sqrt());
                if q == 0.0 {
                    vec![0.0]
                } else {
                    vec![q / qa, qc / q]
                }
            }
        }
    };
    let slack = TOUCH_MM / qa.sqrt();
    roots
        .into_iter()
        .filter(|t| *t >= -slack && *t <= 1.0 + slack)
        .map(|t| (a.0 + t * d.0, a.1 + t * d.1))
        .collect()
}

/// Where the circles of two arcs meet (not yet limited to their sweeps).
/// With a shared end `v`, the other meeting is `v` mirrored across the line
/// through the centres.
fn circle_circle_points(one: &ArcGeom, two: &ArcGeom, shared: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let (dx, dy) = (two.center.0 - one.center.0, two.center.1 - one.center.1);
    let d = dx.hypot(dy);
    if d == 0.0 {
        return Vec::new();
    }
    let (ux, uy) = (dx / d, dy / d);
    if let Some(v) = shared.first() {
        let along = (v.0 - one.center.0) * ux + (v.1 - one.center.1) * uy;
        let foot = (one.center.0 + along * ux, one.center.1 + along * uy);
        return vec![(2.0 * foot.0 - v.0, 2.0 * foot.1 - v.1)];
    }
    let (r1, r2) = (one.radius, two.radius);
    if d > r1 + r2 + TOUCH_MM || d < (r1 - r2).abs() - TOUCH_MM {
        return Vec::new();
    }
    let along = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h = (r1 * r1 - along * along).max(0.0).sqrt();
    let base = (one.center.0 + along * ux, one.center.1 + along * uy);
    vec![
        (base.0 - h * uy, base.1 + h * ux),
        (base.0 + h * uy, base.1 - h * ux),
    ]
}

/// Whether two edges of an outline meet anywhere but at the corners in
/// `shared`, the ones they have in common.
fn edges_meet(one: &Edge, two: &Edge, shared: &[(f64, f64)]) -> bool {
    let away = |p: &(f64, f64)| shared.iter().all(|s| distance(*p, *s) > TOUCH_MM);
    match (one, two) {
        (Edge::Line(a1, a2), Edge::Line(b1, b2)) => match shared.first() {
            None => segments_touch(*a1, *a2, *b1, *b2),
            // Straight neighbours meet beyond their corner only by folding
            // back along one line.
            Some(v) => {
                let p = if a1 == v { *a2 } else { *a1 };
                let q = if b1 == v { *b2 } else { *b1 };
                cross(p, *v, q).abs() <= 1e-12
                    && (p.0 - v.0) * (q.0 - v.0) + (p.1 - v.1) * (q.1 - v.1) > 0.0
            }
        },
        (Edge::Line(a, b), Edge::Arc(arc)) | (Edge::Arc(arc), Edge::Line(a, b)) => {
            segment_circle_points(*a, *b, arc, shared)
                .iter()
                .any(|p| away(p) && arc.covers(*p))
        }
        (Edge::Arc(g1), Edge::Arc(g2)) => {
            if distance(g1.center, g2.center) <= TOUCH_MM
                && (g1.radius - g2.radius).abs() <= TOUCH_MM
            {
                // One circle: the arcs overlap when their spans do.
                let tau = std::f64::consts::TAU;
                let gap = (g1.a0 + g1.sweep / 2.0 - g2.a0 - g2.sweep / 2.0).rem_euclid(tau);
                let gap = gap.min(tau - gap);
                return gap < (g1.sweep.abs() + g2.sweep.abs()) / 2.0 - TOUCH_MM / g1.radius;
            }
            circle_circle_points(g1, g2, shared)
                .iter()
                .any(|p| away(p) && g1.covers(*p) && g2.covers(*p))
        }
    }
}

/// Check that the closed outline through `corners` is one simple loop. Each
/// corner may carry the midpoint of an arc to the next corner. Straight edges
/// only go through `check_simple_ring`. With an arc, every pair of edges is
/// intersected exactly (segment with circle, circle with circle, then limited
/// to each arc's sweep), so a crossing cannot hide between an arc and its
/// chords: no corner may repeat, and no edge may meet another except a
/// neighbour at their shared corner.
pub(crate) fn check_simple_outline(corners: &[OutlineCorner]) -> Result<(), String> {
    let points: Vec<(f64, f64)> = corners.iter().map(|(p, _)| *p).collect();
    if corners.iter().all(|(_, mid)| mid.is_none()) {
        return check_simple_ring(&points);
    }
    let finite = |p: &(f64, f64)| p.0.is_finite() && p.1.is_finite();
    if !corners
        .iter()
        .all(|(p, mid)| finite(p) && mid.as_ref().is_none_or(finite))
    {
        return Err("every coordinate must be finite".to_string());
    }
    let n = corners.len();
    if n < 2 {
        return Err(format!(
            "needs at least 2 corners when an edge is an arc, got {n}"
        ));
    }
    for i in 0..n {
        for j in i + 1..n {
            if distance(points[i], points[j]) < 1e-9 {
                return Err(format!("corners {i} and {j} are the same point"));
            }
        }
    }
    let mut edges = Vec::with_capacity(n);
    for (i, (start, mid)) in corners.iter().enumerate() {
        let end = points[(i + 1) % n];
        edges.push(match mid {
            None => Edge::Line(*start, end),
            Some(mid) => Edge::Arc(
                ArcGeom::new(*start, *mid, end)
                    .ok_or_else(|| format!("edge {i}: the arc's midpoint is on its chord"))?,
            ),
        });
    }
    for i in 0..n {
        for j in i + 1..n {
            // Edge k runs from corner k to corner k + 1.
            let shared: Vec<(f64, f64)> = [i, (i + 1) % n]
                .into_iter()
                .filter(|c| *c == j || *c == (j + 1) % n)
                .map(|c| points[c])
                .collect();
            if edges_meet(&edges[i], &edges[j], &shared) {
                return Err(format!("edge {i} crosses or touches edge {j}"));
            }
        }
    }
    Ok(())
}

/// Read `points` as a list of `{x, y}` in mm. A closing point equal to the
/// first is dropped.
pub(crate) fn parse_points(value: &Value, field: &str) -> Result<Vec<(f64, f64)>, CallToolResult> {
    let Some(items) = value.as_array() else {
        return Err(invalid_arg(field, "must be an array of {x, y} points"));
    };
    let mut points = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        match (
            item.get("x").and_then(Value::as_f64),
            item.get("y").and_then(Value::as_f64),
        ) {
            (Some(x), Some(y)) => points.push((x, y)),
            _ => {
                return Err(invalid_arg(
                    field,
                    &format!("point {index} needs numeric 'x' and 'y'"),
                ))
            }
        }
    }
    if points.len() > 3 && points.first() == points.last() {
        points.pop();
    }
    Ok(points)
}

// ─── Rule areas ───────────────────────────────────────────────────────────────

fn board_copper_layers(tree: &SexpNode) -> Vec<String> {
    konnect_sexp::layers::layers(tree)
        .into_iter()
        .filter(|layer| layer.is_copper())
        .map(|layer| layer.name)
        .collect()
}

fn format_rule_area(
    uuid: &str,
    layers: &[String],
    name: &str,
    forbidden: &[bool; 5],
    ring: &[(f64, f64)],
) -> String {
    let quoted: Vec<String> = layers
        .iter()
        .map(|l| format!("\"{}\"", escape(l)))
        .collect();
    let layer_node = if layers.len() == 1 {
        format!("(layer {})", quoted[0])
    } else {
        format!("(layers {})", quoted.join(" "))
    };
    let name_node = if name.is_empty() {
        String::new()
    } else {
        format!("\n    (name \"{}\")", escape(name))
    };
    let flags: Vec<String> = KEEPOUT_FLAGS
        .iter()
        .zip(forbidden)
        .map(|(flag, no)| format!("({flag} {})", if *no { "not_allowed" } else { "allowed" }))
        .collect();
    let pts: String = ring
        .iter()
        .map(|(x, y)| format!("\n        (xy {x} {y})"))
        .collect();
    format!(
        "\n  (zone\n    {layer_node}\n    (uuid \"{uuid}\"){name_node}\n    (hatch edge 0.5)\n    \
         (connect_pads (clearance 0))\n    (min_thickness 0.25)\n    \
         (keepout {flags})\n    (placement (enabled no) (sheetname \"\"))\n    \
         (fill (thermal_gap 0.5) (thermal_bridge_width 0.5))\n    \
         (polygon\n      (pts{pts}\n      )\n    )\n  )",
        flags = flags.join(" ")
    )
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn rectangle_or_points(args: &Value) -> Result<Vec<(f64, f64)>, CallToolResult> {
    let corners: Vec<Option<&Value>> = ["x1", "y1", "x2", "y2"]
        .iter()
        .map(|k| args.get(*k))
        .collect();
    let any_corner = corners.iter().any(|c| c.is_some_and(|v| !v.is_null()));
    match (args.get("points").filter(|v| !v.is_null()), any_corner) {
        (Some(_), true) => Err(invalid_arg(
            "points",
            "give either 'points' or x1, y1, x2, y2, not both",
        )),
        (Some(points), false) => parse_points(points, "points"),
        (None, true) => {
            let mut values = [0.0; 4];
            for (slot, (key, value)) in values
                .iter_mut()
                .zip(["x1", "y1", "x2", "y2"].iter().zip(&corners))
            {
                *slot = match value.and_then(|v| v.as_f64()) {
                    Some(v) => v,
                    None => {
                        return Err(invalid_arg(
                            key,
                            "must be a number; a rectangle needs x1, y1, x2 and y2",
                        ))
                    }
                };
            }
            let [x1, y1, x2, y2] = values;
            let (left, right, top, bottom) = (x1.min(x2), x1.max(x2), y1.min(y2), y1.max(y2));
            Ok(vec![
                (left, top),
                (right, top),
                (right, bottom),
                (left, bottom),
            ])
        }
        (None, false) => Err(invalid_arg(
            "points",
            "give 'points' or a rectangle x1, y1, x2, y2",
        )),
    }
}

async fn handle_add_rule_area(args: &Value, ctx: &ToolContext) -> anyhow::Result<CallToolResult> {
    const OPERATION: &str = "add_rule_area";
    let board = get_path(args, "board")?;
    let requested_layers = match string_list(args, "layers") {
        Ok(Some(layers)) if !layers.is_empty() => layers,
        Ok(_) => return Ok(invalid_arg("layers", "name at least one copper layer")),
        Err(e) => return Ok(e),
    };
    let ring = match rectangle_or_points(args) {
        Ok(r) => r,
        Err(e) => return Ok(e),
    };
    if let Err(reason) = check_simple_ring(&ring) {
        return Ok(invalid_arg(
            "points",
            &format!("the outline is not a simple polygon: {reason}"),
        ));
    }
    let name = match args.get("name") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.trim().to_string(),
        Some(_) => return Ok(invalid_arg("name", "must be a string")),
    };
    let mut forbidden = DEFAULT_FORBIDDEN;
    match args.get("keepout") {
        None | Some(Value::Null) => {}
        Some(Value::Object(map)) => {
            for (key, value) in map {
                let Some(index) = KEEPOUT_FLAGS.iter().position(|flag| flag == key) else {
                    return Ok(invalid_arg(
                        "keepout",
                        &format!(
                            "unknown flag '{key}'; expected {}",
                            KEEPOUT_FLAGS.join(", ")
                        ),
                    ));
                };
                match value.as_bool() {
                    Some(no) => forbidden[index] = no,
                    None => {
                        return Ok(invalid_arg(
                            "keepout",
                            &format!("'{key}' must be true or false"),
                        ))
                    }
                }
            }
        }
        Some(_) => return Ok(invalid_arg("keepout", "must be an object of booleans")),
    }
    if !forbidden.iter().any(|no| *no) {
        return Ok(invalid_arg(
            "keepout",
            "forbids nothing; a rule area that allows everything has no effect",
        ));
    }
    let expected = match expected_revision(args) {
        Ok(v) => v,
        Err(e) => return Ok(e),
    };

    if let Some(refusal) = refuse_if_board_open_in_kicad(ctx, &board, "rule area").await? {
        return Ok(refusal);
    }
    let content = match read_board(&board) {
        Ok(c) => c,
        Err(e) => return Ok(e),
    };
    let blocks = match top_level_blocks(&content) {
        Ok(b) => b,
        Err(reason) => return Ok(unreadable_board(&board, &reason)),
    };
    let tree = parse_sexp(&content)?;
    let copper = board_copper_layers(&tree);
    if copper.is_empty() {
        return Ok(unreadable_board(
            &board,
            "the board declares no copper layers",
        ));
    }
    let mut layers: Vec<String> = Vec::new();
    for layer in &requested_layers {
        if layer == "*.Cu" {
            layers.extend(copper.iter().cloned());
        } else if copper.contains(layer) {
            layers.push(layer.clone());
        } else {
            return Ok(invalid_arg(
                "layers",
                &format!(
                    "'{layer}' is not a copper layer of this board (it has {})",
                    copper.join(", ")
                ),
            ));
        }
    }
    // Board order, once each.
    let layers: Vec<String> = copper
        .iter()
        .filter(|l| layers.contains(l))
        .cloned()
        .collect();

    if !name.is_empty() {
        let clash = blocks
            .iter()
            .filter(|b| b.node.head() == Some("zone"))
            .any(|b| b.node.find_str("name") == Some(name.as_str()));
        if clash {
            return Ok(invalid_arg(
                "name",
                &format!("a zone named '{name}' is already on the board; names must be unique so delete_zone can find one"),
            ));
        }
    }

    let flags: BTreeMap<&str, &str> = KEEPOUT_FLAGS
        .iter()
        .zip(forbidden)
        .map(|(flag, no)| (*flag, if no { "not_allowed" } else { "allowed" }))
        .collect();
    let request = json!({
        "layers": layers,
        "points": ring.iter().map(|(x, y)| [*x, *y]).collect::<Vec<_>>(),
        "name": name,
        "keepout": flags,
    });
    let current = revision(OPERATION, &board, &content, &request);
    let routing_keepout = forbidden[0] || forbidden[1];
    let summary = json!({
        "layers": layers,
        "name": name,
        "keepout": flags,
        "corner_count": ring.len(),
        "area_mm2": ring_area(&ring).abs(),
        "exported_to_specctra_as_keepout": routing_keepout,
    });

    let Some(expected) = expected else {
        let mut body = json!({
            "mode": "preview",
            "board": board,
            "revision": current,
            "rule_area": summary,
            "next": "Call again with the same arguments and expected_revision to apply.",
        });
        if !routing_keepout {
            body["note"] = json!(
                "This rule area forbids neither tracks nor vias, so export_specctra_dsn leaves it out: Freerouting will route through it."
            );
        }
        return Ok(CallToolResult::json(&body));
    };
    if expected != current {
        return Ok(stale(&board));
    }

    let uuid = new_uuid();
    let close = content.rfind(')').unwrap_or(content.len());
    let next = apply_edits(
        content.clone(),
        vec![SexpEdit::insert(
            close,
            format_rule_area(&uuid, &layers, &name, &forbidden, &ring) + "\n",
        )],
    );
    let written = match commit(&board, &content, &next, OPERATION) {
        Ok(w) => w,
        Err(e) => return Ok(e),
    };
    let after = match top_level_blocks(&written) {
        Ok(b) => b,
        Err(reason) => return Ok(uncertain(&board, OPERATION, &reason)),
    };
    let found: Vec<&Block> = after
        .iter()
        .filter(|b| {
            b.node.head() == Some("zone") && node_uuid(&b.node).as_deref() == Some(uuid.as_str())
        })
        .collect();
    let [zone] = found.as_slice() else {
        return Ok(uncertain(
            &board,
            OPERATION,
            "the new rule area is not in the file exactly once",
        ));
    };
    let described = describe_zone(&zone.node);
    if described["kind"] != "rule_area"
        || described["layers"] != json!(layers)
        || described["keepout"] != json!(flags)
        || described["corner_count"] != json!(ring.len())
        || after.len() != blocks.len() + 1
    {
        return Ok(uncertain(
            &board,
            OPERATION,
            "the written rule area does not read back as requested",
        ));
    }
    Ok(CallToolResult::json(&json!({
        "mode": "applied",
        "board": board,
        "uuid": uuid,
        "rule_area": summary,
        "written": true,
        "source": "file",
        "readback": "verified",
        "note": "Reload the board in KiCad before editing it there.",
    })))
}

// ─── Zones: list and delete ───────────────────────────────────────────────────

fn zone_layers(zone: &SexpNode) -> Vec<String> {
    let mut layers: Vec<String> = Vec::new();
    if let Some(single) = zone.find_str("layer") {
        layers.push(single.to_string());
    }
    if let Some(list) = zone.find("layers") {
        layers.extend(
            list.children()
                .unwrap_or(&[])
                .iter()
                .skip(1)
                .filter_map(SexpNode::as_str)
                .map(str::to_string),
        );
    }
    layers
}

/// KiCad 10 names the net, `(net "GND")`; older files give an id and a
/// `(net_name …)`. A rule area has none, or net 0.
fn zone_net(zone: &SexpNode) -> Option<String> {
    if let Some(name) = zone.find_str("net_name") {
        return (!name.is_empty()).then(|| name.to_string());
    }
    let net = zone.find("net")?.get(1)?;
    match net {
        SexpNode::Str(name) => (!name.is_empty()).then(|| name.clone()),
        SexpNode::Atom(id) if id == "0" => None,
        SexpNode::Atom(id) => Some(id.clone()),
        SexpNode::List(_) => None,
    }
}

/// Points whose bounding box is the zone outline's: each corner, and for an
/// arc the corners of its own box, bulge included.
fn zone_corners(zone: &SexpNode) -> Vec<(f64, f64)> {
    zone.find_all("polygon")
        .into_iter()
        .filter_map(|polygon| polygon.find("pts"))
        .flat_map(|pts| pts.children().unwrap_or(&[]).iter().skip(1))
        .flat_map(|item| match item.head() {
            Some("xy") => item
                .get_f64(1)
                .zip(item.get_f64(2))
                .into_iter()
                .collect::<Vec<_>>(),
            Some("arc") => {
                let points: Vec<(f64, f64)> = ["start", "mid", "end"]
                    .iter()
                    .filter_map(|tag| item.find(tag))
                    .filter_map(|p| p.get_f64(1).zip(p.get_f64(2)))
                    .collect();
                match points.as_slice() {
                    [start, mid, end] => {
                        let (x1, y1, x2, y2) = arc_extent(*start, *mid, *end);
                        vec![(x1, y1), (x2, y2)]
                    }
                    _ => points,
                }
            }
            _ => Vec::new(),
        })
        .collect()
}

fn describe_zone(zone: &SexpNode) -> Value {
    let keepout = zone.find("keepout").map(|keepout| {
        KEEPOUT_FLAGS
            .iter()
            .map(|flag| (*flag, keepout.find_str(flag).unwrap_or("allowed")))
            .collect::<BTreeMap<&str, &str>>()
    });
    let corners = zone_corners(zone);
    let bbox = corners
        .iter()
        .fold(None, |acc: Option<(f64, f64, f64, f64)>, (x, y)| {
            Some(match acc {
                None => (*x, *y, *x, *y),
                Some((a, b, c, d)) => (a.min(*x), b.min(*y), c.max(*x), d.max(*y)),
            })
        });
    json!({
        "uuid": node_uuid(zone),
        "name": zone.find_str("name").unwrap_or(""),
        "kind": if keepout.is_some() { "rule_area" } else { "copper" },
        "net": zone_net(zone),
        "layers": zone_layers(zone),
        "keepout": keepout,
        "priority": zone.find_f64("priority").unwrap_or(0.0) as u64,
        "polygon_count": zone.find_all("polygon").len(),
        "corner_count": zone.find_all("polygon").first()
            .and_then(|p| p.find("pts"))
            .map(|pts| pts.children().unwrap_or(&[]).len().saturating_sub(1))
            .unwrap_or(0),
        "bbox": bbox.map(|(x1, y1, x2, y2)| json!({ "x1": x1, "y1": y1, "x2": x2, "y2": y2 })),
    })
}

async fn handle_list_zones(args: &Value) -> anyhow::Result<CallToolResult> {
    let board = get_path(args, "board")?;
    let kind = match args.get("kind") {
        None | Some(Value::Null) => None,
        Some(Value::String(k)) if k == "copper" || k == "rule_area" => Some(k.clone()),
        Some(_) => return Ok(invalid_arg("kind", "must be 'copper' or 'rule_area'")),
    };
    let content = match read_board(&board) {
        Ok(c) => c,
        Err(e) => return Ok(e),
    };
    let blocks = match top_level_blocks(&content) {
        Ok(b) => b,
        Err(reason) => return Ok(unreadable_board(&board, &reason)),
    };
    let zones: Vec<Value> = blocks
        .iter()
        .filter(|b| b.node.head() == Some("zone"))
        .map(|b| describe_zone(&b.node))
        .filter(|z| kind.as_deref().is_none_or(|k| z["kind"] == k))
        .collect();
    let rule_areas = zones.iter().filter(|z| z["kind"] == "rule_area").count();
    Ok(CallToolResult::json(&json!({
        "board": board,
        "source": "file",
        "count": zones.len(),
        "copper_count": zones.len() - rule_areas,
        "rule_area_count": rule_areas,
        "zones": zones,
    })))
}

async fn handle_delete_zone(args: &Value, ctx: &ToolContext) -> anyhow::Result<CallToolResult> {
    const OPERATION: &str = "delete_zone";
    let board = get_path(args, "board")?;
    let uuids = match string_list(args, "uuids") {
        Ok(v) => v.unwrap_or_default(),
        Err(e) => return Ok(e),
    };
    let names = match string_list(args, "names") {
        Ok(v) => v.unwrap_or_default(),
        Err(e) => return Ok(e),
    };
    if uuids.is_empty() && names.is_empty() {
        return Ok(invalid_arg(
            "uuids",
            "name the zones to delete with 'uuids' or 'names' (list_zones shows both)",
        ));
    }
    let expected = match expected_revision(args) {
        Ok(v) => v,
        Err(e) => return Ok(e),
    };

    if let Some(refusal) = refuse_if_board_open_in_kicad(ctx, &board, "zone deletion").await? {
        return Ok(refusal);
    }
    let content = match read_board(&board) {
        Ok(c) => c,
        Err(e) => return Ok(e),
    };
    let blocks = match top_level_blocks(&content) {
        Ok(b) => b,
        Err(reason) => return Ok(unreadable_board(&board, &reason)),
    };
    let zones: Vec<&Block> = blocks
        .iter()
        .filter(|b| b.node.head() == Some("zone"))
        .collect();

    let carrying = |uuid: &str| -> Vec<usize> {
        zones
            .iter()
            .enumerate()
            .filter(|(_, z)| node_uuid(&z.node).as_deref() == Some(uuid))
            .map(|(i, _)| i)
            .collect()
    };
    let mut selected: BTreeSet<usize> = BTreeSet::new();
    let mut unknown: Vec<String> = Vec::new();
    for uuid in &uuids {
        let matches = carrying(uuid);
        if matches.is_empty() {
            unknown.push(format!("uuid {uuid}"));
        }
        // Every zone that carries it; a shared UUID is refused below.
        selected.extend(matches);
    }
    let mut ambiguous: Vec<String> = Vec::new();
    for name in &names {
        let matches: Vec<usize> = zones
            .iter()
            .enumerate()
            .filter(|(_, z)| z.node.find_str("name") == Some(name.as_str()))
            .map(|(i, _)| i)
            .collect();
        match matches.as_slice() {
            [] => unknown.push(format!("name '{name}'")),
            [one] => {
                selected.insert(*one);
            }
            many => ambiguous.push(format!(
                "'{name}' ({} zones: {})",
                many.len(),
                many.iter()
                    .map(|i| node_uuid(&zones[*i].node).unwrap_or_else(|| "no uuid".to_string()))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
    if !unknown.is_empty() {
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::InvalidArgument {
                field: if uuids.is_empty() { "names" } else { "uuids" }.to_string(),
                reason: format!("no such zone: {}", unknown.join(", ")),
            },
            format!(
                "No top-level zone matches {}. Nothing was deleted; list_zones shows what is there.",
                unknown.join(", ")
            ),
        ));
    }
    if !ambiguous.is_empty() {
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::AmbiguousTarget {
                target: "names".to_string(),
                candidates: ambiguous.clone(),
            },
            format!(
                "A name matches more than one zone: {}. Nothing was deleted; pass 'uuids' instead.",
                ambiguous.join("; ")
            ),
        ));
    }
    // A target whose UUID another zone carries too, picked by that UUID or by
    // its name: the readback could not tell which zone went, so it is
    // refused before anything is written.
    let mut duplicated: Vec<String> = Vec::new();
    for index in &selected {
        if let Some(uuid) = node_uuid(&zones[*index].node) {
            let count = carrying(&uuid).len();
            let entry = format!("{uuid} ({count} zones)");
            if count > 1 && !duplicated.contains(&entry) {
                duplicated.push(entry);
            }
        }
    }
    if !duplicated.is_empty() {
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::AmbiguousTarget {
                target: "uuids".to_string(),
                candidates: duplicated.clone(),
            },
            format!(
                "More than one zone carries the same UUID: {}. Nothing was deleted. Fix the \
                 duplicate UUID in KiCad, then list_zones again.",
                duplicated.join("; ")
            ),
        ));
    }

    let targets: Vec<&Block> = selected.iter().map(|i| zones[*i]).collect();
    let described: Vec<Value> = targets.iter().map(|z| describe_zone(&z.node)).collect();
    let request = json!({
        "uuids": uuids,
        "names": names,
    });
    let current = revision(OPERATION, &board, &content, &request);
    let Some(expected) = expected else {
        let copper_left = zones
            .iter()
            .enumerate()
            .any(|(i, z)| !selected.contains(&i) && z.node.find("keepout").is_none());
        return Ok(CallToolResult::json(&json!({
            "mode": "preview",
            "board": board,
            "revision": current,
            "zones": described,
            "would_delete": described.len(),
            "copper_zones_remaining": copper_left,
            "next": "Call again with the same arguments and expected_revision to apply.",
        })));
    };
    if expected != current {
        return Ok(stale(&board));
    }

    let edits = targets
        .iter()
        .map(|z| {
            let (start, end) =
                find_block_with_leading_whitespace(&content, z.start).unwrap_or((z.start, z.end));
            SexpEdit::delete(start, end)
        })
        .collect();
    let next = apply_edits(content.clone(), edits);
    let written = match commit(&board, &content, &next, OPERATION) {
        Ok(w) => w,
        Err(e) => return Ok(e),
    };
    let after = match top_level_blocks(&written) {
        Ok(b) => b,
        Err(reason) => return Ok(uncertain(&board, OPERATION, &reason)),
    };
    let remaining_zones: Vec<&Block> = after
        .iter()
        .filter(|b| b.node.head() == Some("zone"))
        .collect();
    let deleted_still_there = targets.iter().any(|t| {
        let uuid = node_uuid(&t.node);
        uuid.is_some() && remaining_zones.iter().any(|z| node_uuid(&z.node) == uuid)
    });
    if deleted_still_there
        || remaining_zones.len() + targets.len() != zones.len()
        || after.len() + targets.len() != blocks.len()
    {
        return Ok(uncertain(
            &board,
            OPERATION,
            "the deleted zones do not read back as gone",
        ));
    }
    Ok(CallToolResult::json(&json!({
        "mode": "applied",
        "board": board,
        "zones": described,
        "deleted": described.len(),
        "written": true,
        "source": "file",
        "readback": "verified",
        "note": "Reload the board in KiCad before editing it there. Copper fills of the remaining zones are as saved; run refill_zones if their shape should change.",
    })))
}

#[cfg(test)]
#[path = "board_construction_tests.rs"]
mod tests;
