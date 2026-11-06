//! `query_board_region` — what is in an area of the board — and the shared
//! helpers the outline checks use.
//!
//! The geometry itself lives in [`konnect_sexp::board_geometry`]; this module
//! only reads a board (the live one through the board-source seam when KiCad
//! holds it, the saved file otherwise), turns arguments into a region, and
//! shapes the answer. Nothing here writes to a board.

use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::CallToolResult;
use crate::tool;
use crate::tools::board_source::{self, BoardSource};
use crate::tools::{get_path, invalid_arg, ToolContext, ToolDef};
use konnect_sexp::board_geometry::{
    Allowlist, BoardGeometry, Entry, FootprintOverhang, Kind, OutlineCheck, PartOverhang,
    RegionQuery, Shape,
};
use konnect_sexp::outline::OutlineStatus;
use serde_json::{json, Value};

/// Items returned when the caller names no limit.
const DEFAULT_LIMIT: usize = 50;
/// The most items one answer may carry; a response stays well under client
/// size limits at this count.
const MAX_LIMIT: usize = 200;
/// Margin around a footprint when `around` names none, in mm.
const DEFAULT_MARGIN_MM: f64 = 2.0;

pub fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "query_board_region",
        "Read-only: list what is in an area of the board — footprints (courtyards), pads, \
         tracks, vias, zones, graphics and the Edge.Cuts outline — nearest first, with nets, \
         layers and UUIDs. Name the area as exactly one of 'bbox' (a box), 'around' (a \
         footprint or one of its pads, plus a margin) or 'point' (a centre and radius). \
         Replaces ad-hoc scripts that parse the board file for 'what is near this pad'. \
         Pads are real polygons in board coordinates, rotation included. 'truncated' says \
         when 'limit' cut the list; 'total_matches' and 'counts' are always the full numbers. \
         Reads the board KiCad holds open where it can, so unsaved edits count; \
         'board_source' selects that. Zone results are their authored outline, not the \
         filled copper; footprint graphics and text are not listed.",
        json!({
            "type": "object",
            "properties": {
                "board": { "type": "string", "description": "Path to .kicad_pcb file" },
                "bbox": {
                    "type": "object",
                    "description": "The area as a box, in mm board coordinates",
                    "properties": {
                        "x_min": { "type": "number" },
                        "y_min": { "type": "number" },
                        "x_max": { "type": "number" },
                        "y_max": { "type": "number" }
                    },
                    "required": ["x_min", "y_min", "x_max", "y_max"]
                },
                "around": {
                    "type": "object",
                    "description": "The area around a footprint (its courtyard) or one pad of it, grown by a margin. The footprint itself and its pads are left out of the answer",
                    "properties": {
                        "reference": { "type": "string", "description": "Footprint reference, e.g. U1" },
                        "pad": { "type": "string", "description": "Pad number: use that pad instead of the whole footprint" },
                        "margin_mm": { "type": "number", "minimum": 0, "default": 2.0 }
                    },
                    "required": ["reference"]
                },
                "point": {
                    "type": "object",
                    "description": "The area as a centre and a radius, in mm board coordinates",
                    "properties": {
                        "x": { "type": "number" },
                        "y": { "type": "number" },
                        "radius_mm": { "type": "number", "minimum": 0 }
                    },
                    "required": ["x", "y", "radius_mm"]
                },
                "kinds": {
                    "type": "array",
                    "items": { "type": "string", "enum": ["footprint", "pad", "track", "via", "zone", "graphic", "outline"] },
                    "description": "Only these kinds. Default: every kind but the outline"
                },
                "layer": { "type": "string", "description": "Only items on this layer (a through-hole pad or via is on every copper layer)" },
                "net": { "type": "string", "description": "Only items on this net" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 50 },
                "board_source": board_source::board_source_schema()
            },
            "required": ["board"]
        }),
        |args, ctx| async move { handle_query_board_region(args, ctx).await }
    )
    .with_board_access(crate::tools::BoardAccess::LivePreferredWithFallback)]
}

// ─── Arguments ───────────────────────────────────────────────────────────────

fn number(node: &Value, key: &str, parent: &str) -> Result<f64, CallToolResult> {
    node.get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or_else(|| invalid_arg(&format!("{parent}.{key}"), "missing or not a number"))
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Shape {
    Shape::polygon(vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
}

/// The area the caller named. `around` needs the board, so it resolves later.
enum Area {
    Bbox(Shape),
    Point(Shape),
    Around {
        reference: String,
        pad: Option<String>,
        margin: f64,
    },
}

fn parse_area(args: &Value) -> Result<Area, CallToolResult> {
    let named: Vec<&str> = ["bbox", "around", "point"]
        .into_iter()
        .filter(|k| args.get(*k).is_some_and(|v| !v.is_null()))
        .collect();
    if named.len() != 1 {
        return Err(invalid_arg(
            "bbox",
            &format!(
                "name exactly one of 'bbox', 'around' or 'point' (got {})",
                if named.is_empty() {
                    "none".to_string()
                } else {
                    named.join(" and ")
                }
            ),
        ));
    }
    match named[0] {
        "bbox" => {
            let b = &args["bbox"];
            let (x0, y0) = (number(b, "x_min", "bbox")?, number(b, "y_min", "bbox")?);
            let (x1, y1) = (number(b, "x_max", "bbox")?, number(b, "y_max", "bbox")?);
            if x0 > x1 || y0 > y1 {
                return Err(invalid_arg(
                    "bbox",
                    "x_min/y_min must not exceed x_max/y_max",
                ));
            }
            Ok(Area::Bbox(rect(x0, y0, x1, y1)))
        }
        "point" => {
            let p = &args["point"];
            let radius = number(p, "radius_mm", "point")?;
            if radius < 0.0 {
                return Err(invalid_arg("point.radius_mm", "must not be negative"));
            }
            let c = (number(p, "x", "point")?, number(p, "y", "point")?);
            Ok(Area::Point(Shape::disc(c, radius)))
        }
        _ => {
            let a = &args["around"];
            let reference = a
                .get("reference")
                .and_then(Value::as_str)
                .filter(|r| !r.is_empty())
                .ok_or_else(|| invalid_arg("around.reference", "missing or not a string"))?
                .to_string();
            let margin = match a.get("margin_mm") {
                None | Some(Value::Null) => DEFAULT_MARGIN_MM,
                Some(v) => v
                    .as_f64()
                    .filter(|m| m.is_finite() && *m >= 0.0)
                    .ok_or_else(|| {
                        invalid_arg("around.margin_mm", "must be a number of at least 0")
                    })?,
            };
            let pad = a.get("pad").and_then(Value::as_str).map(str::to_string);
            Ok(Area::Around {
                reference,
                pad,
                margin,
            })
        }
    }
}

fn parse_kinds(args: &Value) -> Result<Vec<Kind>, CallToolResult> {
    let Some(list) = args.get("kinds").filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let items = list
        .as_array()
        .ok_or_else(|| invalid_arg("kinds", "must be an array of kind names"))?;
    items
        .iter()
        .map(|v| {
            v.as_str().and_then(Kind::parse).ok_or_else(|| {
                invalid_arg(
                    "kinds",
                    "each kind must be one of footprint, pad, track, via, zone, graphic, outline",
                )
            })
        })
        .collect()
}

fn parse_limit(args: &Value) -> Result<usize, CallToolResult> {
    match args.get("limit") {
        None | Some(Value::Null) => Ok(DEFAULT_LIMIT),
        Some(v) => match v.as_u64() {
            Some(n) if (1..=MAX_LIMIT as u64).contains(&n) => Ok(n as usize),
            _ => Err(invalid_arg(
                "limit",
                &format!("must be an integer from 1 to {MAX_LIMIT}"),
            )),
        },
    }
}

// ─── The handler ─────────────────────────────────────────────────────────────

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

async fn handle_query_board_region(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let board_path = get_path(args, "board")?;
    let source = match BoardSource::from_args(args) {
        Ok(s) => s,
        Err(refusal) => return Ok(refusal),
    };
    let area = match parse_area(args) {
        Ok(a) => a,
        Err(refusal) => return Ok(refusal),
    };
    let kinds = match parse_kinds(args) {
        Ok(k) => k,
        Err(refusal) => return Ok(refusal),
    };
    let limit = match parse_limit(args) {
        Ok(l) => l,
        Err(refusal) => return Ok(refusal),
    };
    let layer = args
        .get("layer")
        .and_then(Value::as_str)
        .map(str::to_string);
    let net = args.get("net").and_then(Value::as_str).map(str::to_string);

    let observation = board_source::read_board(
        ctx,
        &board_path,
        source,
        "board region query",
        |client, document| client.save_document_to_string_in(document),
    )
    .await?;
    let (content, evidence, state) = match observation {
        board_source::BoardRead::Refused(refusal) => return Ok(refusal),
        board_source::BoardRead::Live(content) => (
            content,
            board_source::live_evidence(),
            board_source::FROM_IPC,
        ),
        board_source::BoardRead::Saved(why) => {
            match konnect_sexp::writer::read_consistent(&board_path) {
                Ok(content) => (content, why.evidence(), board_source::FROM_SAVED_BOARD),
                Err(_) if !board_path.exists() => {
                    return Ok(CallToolResult::error_kind(
                        ToolErrorKind::FileNotFound {
                            path: board_path.display().to_string(),
                        },
                        format!("Board file not found: {}", board_path.display()),
                    ))
                }
                Err(error) => return Err(error.into()),
            }
        }
    };
    let tree = konnect_sexp::parse_sexp(&content)?;
    let geometry = BoardGeometry::from_tree(&tree);

    let (region, anchor, exclude, described) = match area {
        Area::Bbox(shape) => {
            let (x0, y0, x1, y1) = shape.bbox().expect("a box has corners");
            let d = json!({ "type": "bbox", "x_min": x0, "y_min": y0, "x_max": x1, "y_max": y1 });
            (shape.clone(), shape, None, d)
        }
        Area::Point(shape) => {
            let c = shape.pts[0];
            let d = json!({ "type": "point", "x": c.0, "y": c.1, "radius_mm": shape.radius });
            (shape.clone(), Shape::disc(c, 0.0), None, d)
        }
        Area::Around {
            reference,
            pad,
            margin,
        } => {
            let Some(fp) = geometry.footprint(&reference) else {
                return Ok(CallToolResult::error(format!(
                    "No footprint '{reference}' on this board, so there is nothing to look around."
                )));
            };
            let anchor = match &pad {
                Some(number) => match fp.pads.iter().find(|p| &p.number == number) {
                    Some(p) => p.shape.clone(),
                    None => {
                        return Ok(CallToolResult::error(format!(
                            "{reference} has no pad '{number}'."
                        )))
                    }
                },
                None => fp.body().0,
            };
            let (x0, y0, x1, y1) = anchor.bbox().expect("a footprint has extent");
            let region = rect(x0 - margin, y0 - margin, x1 + margin, y1 + margin);
            let d = json!({
                "type": "around", "reference": reference, "pad": pad, "margin_mm": margin,
                "x_min": round3(x0 - margin), "y_min": round3(y0 - margin),
                "x_max": round3(x1 + margin), "y_max": round3(y1 + margin),
            });
            (region, anchor, Some(reference), d)
        }
    };

    let result = geometry.query_region(&RegionQuery {
        region,
        anchor,
        kinds,
        layer,
        net,
        exclude_reference: exclude,
        limit,
    });

    let items: Vec<Value> = result
        .hits
        .iter()
        .map(|h| entry_json(&h.entry, h.distance_mm))
        .collect();
    let mut body = json!({
        "region": described,
        "items": items,
        "returned": result.hits.len(),
        "total_matches": result.total,
        "truncated": result.truncated,
        "counts": result.counts,
        "outline": outline_json(&geometry.outline),
        "skipped_unreadable": geometry.skipped,
        "not_included": ["footprint graphics and text", "filled zone copper", "dimensions and tables"],
    });
    if result.truncated {
        body["note"] = json!(format!(
            "{} of {} matches returned, nearest first; narrow the area, name 'kinds', or raise 'limit' (at most {MAX_LIMIT})",
            result.hits.len(),
            result.total
        ));
    }
    board_source::provenance(&mut body, json!({ "geometry": state }), evidence);
    Ok(CallToolResult::json(&body))
}

fn entry_json(e: &Entry, distance_mm: f64) -> Value {
    let mut v = json!({
        "kind": e.kind.key(),
        "form": e.form,
        "x": round3(e.at.0),
        "y": round3(e.at.1),
        "distance_mm": round3(distance_mm),
    });
    let obj = v.as_object_mut().expect("an object");
    if let Some((x0, y0, x1, y1)) = e.shape.bbox() {
        obj.insert(
            "bbox".into(),
            json!([round3(x0), round3(y0), round3(x1), round3(y1)]),
        );
    }
    if let Some(u) = &e.uuid {
        obj.insert("uuid".into(), json!(u));
    }
    if let Some(r) = &e.reference {
        obj.insert("reference".into(), json!(r));
    }
    if let Some(n) = &e.name {
        obj.insert(
            if e.kind == Kind::Pad { "pad" } else { "name" }.into(),
            json!(n),
        );
    }
    if let Some(n) = &e.net {
        obj.insert("net".into(), json!(n));
    }
    if !e.layers.is_empty() {
        obj.insert("layers".into(), json!(e.layers));
    }
    for (k, val) in &e.props {
        obj.insert((*k).into(), json!(round3(*val)));
    }
    if !e.notes.is_empty() {
        obj.insert("notes".into(), json!(e.notes));
    }
    v
}

/// The outline as a response reports it: what it is, never a bare "ok".
pub(crate) fn outline_json(o: &konnect_sexp::outline::Outline) -> Value {
    let mut v = json!({
        "status": o.status().key(),
        "rings": o.loops.len(),
        "cutouts": o.hole_count(),
        "curved": o.curved,
        "tolerance_mm": o.tolerance_mm,
    });
    if o.open_chains > 0 {
        v["open_chains"] = json!(o.open_chains);
    }
    if o.unreadable > 0 {
        v["unreadable_graphics"] = json!(o.unreadable);
    }
    if let Some((x0, y0, x1, y1)) = o.bbox() {
        v["bbox"] = json!([round3(x0), round3(y0), round3(x1), round3(y1)]);
    }
    v
}

// ─── Shared by the outline checks ────────────────────────────────────────────

/// The schema property every tool that honours the edge-mount allowlist adds.
pub(crate) fn allowlist_schema() -> Value {
    json!({
        "type": "array",
        "items": { "type": "string" },
        "description": "References that may sit on or over the board edge on purpose (edge-mount \
                        connectors, card-edge fingers, antennas). Exact references, or a prefix \
                        ending in '*' such as 'J*'. They are still listed, as intentional, with \
                        their overhang"
    })
}

/// Read `edge_mount_references` from tool arguments.
pub(crate) fn parse_allowlist(args: &Value) -> Result<Allowlist, CallToolResult> {
    match args.get("edge_mount_references") {
        None | Some(Value::Null) => Ok(Allowlist::default()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .map(Allowlist::new)
            .ok_or_else(|| {
                invalid_arg(
                    "edge_mount_references",
                    "must be an array of reference strings",
                )
            }),
        Some(_) => Err(invalid_arg(
            "edge_mount_references",
            "must be an array of reference strings",
        )),
    }
}

fn part_json(p: &PartOverhang) -> Value {
    json!({
        "part": p.part,
        "label": p.label,
        "overhang_mm": round3(p.overhang_mm),
        "x": round3(p.at.0),
        "y": round3(p.at.1),
    })
}

/// One footprint's overhang as a response reports it.
pub(crate) fn overhang_json(f: &FootprintOverhang) -> Value {
    json!({
        "reference": f.reference,
        "side": match f.side { konnect_sexp::board::Side::Front => "front", konnect_sexp::board::Side::Back => "back" },
        "relation": f.relation,
        "overhang_mm": round3(f.overhang_mm),
        "courtyard": f.courtyard.as_ref().map(part_json),
        "pads": f.pads.iter().map(part_json).collect::<Vec<_>>(),
        "pads_over": f.pads_over,
        "text": f.texts.iter().map(part_json).collect::<Vec<_>>(),
        "intentional_edge_mount": f.allowed,
    })
}

/// A sentence naming what crosses the edge, for a review finding.
pub(crate) fn overhang_sentence(f: &FootprintOverhang) -> String {
    let mut parts = Vec::new();
    if let Some(c) = &f.courtyard {
        parts.push(format!("courtyard {:.2} mm", c.overhang_mm));
    }
    if f.pads_over > 0 {
        let worst = f
            .pads
            .first()
            .map(|p| format!(", worst pad {} at {:.2} mm", p.label, p.overhang_mm))
            .unwrap_or_default();
        parts.push(format!("{} pad(s){worst}", f.pads_over));
    }
    if let Some(t) = f.texts.first() {
        parts.push(format!(
            "{} text {:.2} mm (estimated box)",
            t.label, t.overhang_mm
        ));
    }
    let what = match f.relation {
        "outside" => "lies outside the board outline",
        "text_only" => "has silkscreen text over the board edge",
        _ => "crosses the board outline",
    };
    format!("{} {what}: {}", f.reference, parts.join("; "))
}

/// Why an outline check could not judge anything, or `None` when it did.
fn outline_unjudged_reason(check: &OutlineCheck) -> Option<&'static str> {
    match check.status {
        OutlineStatus::Closed => None,
        OutlineStatus::Missing => Some("no Edge.Cuts graphics: the board has no outline"),
        OutlineStatus::Open => Some(
            "the Edge.Cuts outline does not close into a ring, so containment cannot be judged",
        ),
        OutlineStatus::Unreadable => {
            Some("an Edge.Cuts graphic could not be read, so containment cannot be judged")
        }
    }
}

/// One finding of the outline check, in the shape both review tools turn into
/// their own output.
pub(crate) struct OutlineIssue {
    pub severity: &'static str,
    pub component: Option<String>,
    pub issue: String,
    pub fix: String,
}

/// Findings for an outline check: the outline itself when it cannot be used,
/// then each footprint that crosses it, then a note for each allowlisted one.
///
/// A pad past the edge, or a part wholly outside, cannot be built as designed
/// and is an error. A courtyard alone, or silkscreen text (an estimated box),
/// is a warning. Allowlisted parts are listed as `info`, never silently.
pub(crate) fn outline_issues(check: &OutlineCheck) -> Vec<OutlineIssue> {
    if let Some(reason) = outline_unjudged_reason(check) {
        let (severity, issue, fix) = match check.status {
            OutlineStatus::Missing => (
                "error",
                "No board outline found (the Edge.Cuts layer has no graphics)".to_string(),
                "Add a board outline on the Edge.Cuts layer using add_board_outline",
            ),
            OutlineStatus::Open => (
                "error",
                "The board outline does not close: its Edge.Cuts graphics leave a gap".to_string(),
                "Make the outline's endpoints meet, or redraw it with add_board_outline",
            ),
            _ => (
                "warning",
                format!("Board containment was not checked: {reason}"),
                "Repair or redraw the Edge.Cuts graphic that could not be read",
            ),
        };
        return vec![OutlineIssue {
            severity,
            component: None,
            issue,
            fix: fix.to_string(),
        }];
    }
    let mut out: Vec<OutlineIssue> = check
        .violations
        .iter()
        .map(|f| OutlineIssue {
            severity: if f.relation == "outside" || f.pads_over > 0 {
                "error"
            } else {
                "warning"
            },
            component: Some(f.reference.clone()),
            issue: overhang_sentence(f),
            fix: format!(
                "Move {} inside the board outline. If it is meant to sit on the edge, list it in \
                 edge_mount_references",
                f.reference
            ),
        })
        .collect();
    out.extend(check.allowed.iter().map(|f| OutlineIssue {
        severity: "info",
        component: Some(f.reference.clone()),
        issue: format!("{} (intentional edge-mount part)", overhang_sentence(f)),
        fix: "None needed; listed so the overhang is not silent".to_string(),
    }));
    out
}

/// The outline check as a response carries it.
pub(crate) fn outline_check_json(
    check: &OutlineCheck,
    outline: &konnect_sexp::outline::Outline,
) -> Value {
    json!({
        "status": check.status.key(),
        "checked": check.checked,
        "footprints_checked": check.footprints_checked,
        "geometry": outline_json(outline),
        "crossing": check.violations.iter().map(overhang_json).collect::<Vec<_>>(),
        "intentional_edge_mount": check.allowed.iter().map(overhang_json).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../konnect-sexp/tests/fixtures/board_geometry/cases.kicad_pcb"
    );

    fn ctx() -> ToolContext {
        ToolContext::new(
            crate::tools::ServerConfig::default(),
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        )
    }

    async fn call(args: Value) -> (bool, Value) {
        let result = handle_query_board_region(&args, &ctx()).await.unwrap();
        let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
            panic!("expected text");
        };
        let value = serde_json::from_str(text).unwrap_or_else(|_| json!({ "text": text }));
        (result.is_error, value)
    }

    fn base(extra: Value) -> Value {
        let mut a = json!({ "board": FIXTURE, "board_source": "saved" });
        for (k, v) in extra.as_object().unwrap() {
            a[k] = v.clone();
        }
        a
    }

    #[tokio::test]
    async fn a_box_query_returns_items_with_nets_uuids_and_provenance() {
        let (err, body) = call(base(json!({
            "bbox": { "x_min": 8, "y_min": 6, "x_max": 16, "y_max": 14 },
            "kinds": ["track", "via"]
        })))
        .await;
        assert!(!err, "{body}");
        assert_eq!(body["total_matches"], 4);
        assert_eq!(body["truncated"], false);
        let items = body["items"].as_array().unwrap();
        assert_eq!(items.len(), 4);
        assert!(
            items
                .iter()
                .all(|i| i["uuid"].is_string() && i["net"].is_string()),
            "{body}"
        );
        assert_eq!(body["sources"]["geometry"], "saved_board");
        assert_eq!(
            body["source_evidence"]["excludes_unsaved_editor_state"],
            true
        );
        assert_eq!(body["outline"]["status"], "closed");
        assert_eq!(body["outline"]["cutouts"], 1);
    }

    #[tokio::test]
    async fn the_limit_truncates_and_the_total_stays_true() {
        let (_, body) = call(base(json!({
            "bbox": { "x_min": 8, "y_min": 6, "x_max": 16, "y_max": 14 },
            "kinds": ["track", "via"], "limit": 1
        })))
        .await;
        assert_eq!(body["returned"], 1);
        assert_eq!(body["total_matches"], 4);
        assert_eq!(body["truncated"], true);
        assert!(body["note"].as_str().unwrap().contains("1 of 4"));
    }

    #[tokio::test]
    async fn around_a_pad_finds_its_neighbours_nearest_first_and_leaves_the_part_out() {
        let (err, body) = call(base(json!({
            "around": { "reference": "U1", "pad": "1", "margin_mm": 1.0 }
        })))
        .await;
        assert!(!err, "{body}");
        let items = body["items"].as_array().unwrap();
        assert!(!items.is_empty());
        assert!(items.iter().all(|i| i["reference"] != "U1"), "{body}");
        // The track that ends on U1 pad 1 touches it.
        assert_eq!(items[0]["distance_mm"], 0.0, "{body}");
        let d: Vec<f64> = items
            .iter()
            .map(|i| i["distance_mm"].as_f64().unwrap())
            .collect();
        assert!(d.windows(2).all(|w| w[0] <= w[1]));
    }

    #[tokio::test]
    async fn a_point_query_works() {
        let (err, body) = call(base(
            json!({ "point": { "x": 10, "y": 12, "radius_mm": 0.1 } }),
        ))
        .await;
        assert!(!err, "{body}");
        assert_eq!(body["total_matches"], 4);
    }

    #[tokio::test]
    async fn the_area_must_be_named_exactly_once() {
        for extra in [
            json!({}),
            json!({
                "bbox": { "x_min": 0, "y_min": 0, "x_max": 1, "y_max": 1 },
                "point": { "x": 0, "y": 0, "radius_mm": 1 }
            }),
        ] {
            let (err, body) = call(base(extra)).await;
            assert!(err, "{body}");
        }
    }

    #[tokio::test]
    async fn bad_arguments_are_refused_by_name() {
        for (extra, field) in [
            (
                json!({ "bbox": { "x_min": 5, "y_min": 0, "x_max": 1, "y_max": 1 } }),
                "bbox",
            ),
            (
                json!({ "point": { "x": 0, "y": 0, "radius_mm": -1 } }),
                "radius_mm",
            ),
            (
                json!({ "point": { "x": 0, "y": 0, "radius_mm": 1 }, "kinds": ["wire"] }),
                "kinds",
            ),
            (
                json!({ "point": { "x": 0, "y": 0, "radius_mm": 1 }, "limit": 0 }),
                "limit",
            ),
            (
                json!({ "point": { "x": 0, "y": 0, "radius_mm": 1 }, "limit": 5000 }),
                "limit",
            ),
        ] {
            let (err, body) = call(base(extra)).await;
            assert!(err, "{body}");
            assert!(body.to_string().contains(field), "{field}: {body}");
        }
    }

    #[tokio::test]
    async fn an_unknown_footprint_or_pad_is_an_error_not_an_empty_answer() {
        for around in [
            json!({ "reference": "U99" }),
            json!({ "reference": "U1", "pad": "99" }),
        ] {
            let (err, body) = call(base(json!({ "around": around }))).await;
            assert!(err, "{body}");
        }
    }

    #[tokio::test]
    async fn a_missing_board_file_is_file_not_found() {
        let (err, body) = call(json!({
            "board": "/nonexistent/board.kicad_pcb", "board_source": "saved",
            "point": { "x": 0, "y": 0, "radius_mm": 1 }
        }))
        .await;
        assert!(err, "{body}");
    }

    /// KiCad holds the board open with edits the saved file does not have: the
    /// answer must come from KiCad's copy and say so. The file on disk here is
    /// an empty board, so any item in the answer can only have come over IPC.
    #[tokio::test]
    async fn the_live_board_wins_over_a_stale_saved_file_and_the_answer_says_so() {
        use crate::tools::pcb_board::board_mock::{ctx_talking_to, spawn_kicad_holding_board};
        use konnect_ipc::gen::kiapi;
        use prost::Message;

        let dir = tempfile::tempdir().unwrap();
        let board = dir.path().join("target.kicad_pcb");
        std::fs::write(&board, "(kicad_pcb (version 20260206))").unwrap();
        let live = std::fs::read_to_string(FIXTURE).unwrap();
        let server = spawn_kicad_holding_board(&board, move |command| {
            if command.type_url.ends_with("SaveDocumentToString") {
                let request =
                    kiapi::common::commands::SaveDocumentToString::decode(command.value.as_slice())
                        .unwrap();
                return Some(konnect_ipc::builders::pack_any(
                    &kiapi::common::commands::SavedDocumentResponse {
                        document: request.document,
                        contents: live.clone(),
                    },
                    "kiapi.common.commands.SavedDocumentResponse",
                ));
            }
            None
        });
        let ctx = ctx_talking_to(server.address().to_string());
        let args = json!({
            "board": board.to_string_lossy(),
            "point": { "x": 10, "y": 12, "radius_mm": 0.1 }
        });
        let result = handle_query_board_region(&args, &ctx).await.unwrap();
        let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
            panic!("expected text");
        };
        let body: Value = serde_json::from_str(text).unwrap();
        assert!(!result.is_error, "{body}");
        assert_eq!(body["total_matches"], 4, "{body}");
        assert_eq!(body["sources"]["geometry"], "ipc");
        assert_eq!(
            body["source_evidence"]["excludes_unsaved_editor_state"],
            false
        );

        // Asking for the saved file by name reads the (empty) file instead.
        let mut saved = args.clone();
        saved["board_source"] = json!("saved");
        let result = handle_query_board_region(&saved, &ctx).await.unwrap();
        let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
            panic!("expected text");
        };
        let body: Value = serde_json::from_str(text).unwrap();
        assert_eq!(body["total_matches"], 0, "{body}");
        assert_eq!(body["sources"]["geometry"], "saved_board");
    }

    /// The independent oracle: KiCad's own DRC, run on a scratch copy of the
    /// fixture. Its `copper_edge_clearance` rule reports each pad nearer the
    /// board edge than 0.5 mm and how near it actually is. This geometry must
    /// name the same pads and agree on the distances, the rounded corner and the
    /// cut-out included. A pad inside the cut-out but far from its edge, and the
    /// rotated pad that is on the board only under the right rotation sign, are
    /// the two it must not report.
    ///
    /// Needs the real KiCad. Run:
    /// `KICAD_CLI=/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli cargo test -p konnect-core --lib board_geometry -- --ignored`
    #[tokio::test]
    #[ignore = "needs kicad-cli"]
    async fn kicad_drc_agrees_with_the_outline_about_which_pads_are_at_the_edge() {
        use std::collections::BTreeMap;

        let dir = tempfile::tempdir().unwrap();
        let board = dir.path().join("cases.kicad_pcb");
        std::fs::copy(FIXTURE, &board).unwrap();
        let cli = std::env::var("KICAD_CLI").unwrap_or_else(|_| "kicad-cli".to_owned());
        let report = crate::tools::cli::run_drc(&cli, &board, false)
            .await
            .unwrap();

        // KiCad's answer: (reference, pad) -> actual distance in mm.
        let mut kicad: BTreeMap<(String, String), f64> = BTreeMap::new();
        for v in report
            .violations
            .iter()
            .filter(|v| v.rule == "copper_edge_clearance")
        {
            let actual: f64 = v
                .description
                .split("actual ")
                .nth(1)
                .and_then(|t| t.split(' ').next())
                .and_then(|n| n.parse().ok())
                .unwrap_or_else(|| panic!("no distance in {:?}", v.description));
            let pad = v
                .items
                .iter()
                .find_map(|i| {
                    // "Pad 2 [SIG] of J1 on F.Cu"
                    let rest = i.description.strip_prefix("Pad ")?;
                    let (number, rest) = rest.split_once(' ')?;
                    let reference = rest.split(" of ").nth(1)?.split(' ').next()?;
                    Some((reference.to_string(), number.to_string()))
                })
                .unwrap_or_else(|| panic!("no pad in {:?}", v.items));
            kicad.insert(pad, actual);
        }

        // This geometry's answer: every pad within KiCad's 0.5 mm of an outline
        // edge, measured pad polygon to outline ring.
        let content = std::fs::read_to_string(FIXTURE).unwrap();
        let geometry = BoardGeometry::from_tree(&konnect_sexp::parse_sexp(&content).unwrap());
        let mut ours: BTreeMap<(String, String), f64> = BTreeMap::new();
        for f in &geometry.footprints {
            for p in &f.pads {
                let d = geometry
                    .outline
                    .loops
                    .iter()
                    .map(|ring| p.shape.distance(&Shape::ring(ring.clone(), 0.0)))
                    .fold(f64::INFINITY, f64::min);
                if d < 0.5 {
                    ours.insert((f.reference.clone().unwrap(), p.number.clone()), d);
                }
            }
        }

        let names = |m: &BTreeMap<(String, String), f64>| m.keys().cloned().collect::<Vec<_>>();
        assert_eq!(
            names(&ours),
            names(&kicad),
            "ours {ours:?} vs KiCad {kicad:?}"
        );
        assert_eq!(kicad.len(), 3, "J1 pad 2, C1 pad 3, C2 pad 2: {kicad:?}");
        for (pad, theirs) in &kicad {
            // Tessellated arcs sit up to the tolerance inside the true curve.
            assert!(
                (ours[pad] - theirs).abs() < 0.01,
                "{pad:?}: ours {} vs KiCad {theirs}",
                ours[pad]
            );
        }
    }

    #[test]
    fn the_allowlist_argument_parses_and_refuses_a_malformed_one() {
        let list = parse_allowlist(&json!({ "edge_mount_references": ["J1", "USB*"] })).unwrap();
        assert!(list.allows("J1") && list.allows("USB2") && !list.allows("J2"));
        assert!(parse_allowlist(&json!({})).unwrap().is_empty());
        assert!(parse_allowlist(&json!({ "edge_mount_references": "J1" })).is_err());
        assert!(parse_allowlist(&json!({ "edge_mount_references": [1] })).is_err());
    }
}
