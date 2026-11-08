//! Tests for the board construction tools, over scratch copies of the
//! KiCad-authored Specctra fixture (two resistors, a GND pour, one keepout).

use super::*;
use crate::mcp::error::extract_error_kind;
use crate::tools::pcb_board::board_mock;
use konnect_ipc::{IpcEffectiveRoutingRules, IpcRoutingRules};
use specctra::read::{ListTokenizer, ReadDsn};
use specctra::structure as dsn;
use std::io::{BufReader, Cursor};
use std::path::PathBuf;

const FIXTURE: &str = include_str!("../../tests/fixtures/specctra_rounded_board.kicad_pcb");

fn scratch(source: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.kicad_pcb");
    std::fs::write(&board, source).unwrap();
    (dir, board)
}

/// No KiCad: the empty address is the unreachable transport.
fn offline() -> ToolContext {
    board_mock::ctx_talking_to(String::new())
}

fn text(result: &CallToolResult) -> String {
    match result.content.first() {
        Some(crate::mcp::protocol::ToolContent::Text { text }) => text.clone(),
        other => panic!("expected text content, got {other:?}"),
    }
}

fn body(result: &CallToolResult) -> Value {
    assert!(!result.is_error, "tool failed: {}", text(result));
    serde_json::from_str(&text(result)).unwrap()
}

fn kind(result: &CallToolResult) -> String {
    assert!(result.is_error, "expected a refusal, got {}", text(result));
    extract_error_kind(result).unwrap_or_default()
}

/// Preview, then apply with the preview's revision.
async fn preview_and_apply<F, Fut>(args: Value, call: F) -> (Value, Value)
where
    F: Fn(Value) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<CallToolResult>>,
{
    let preview = body(&call(args.clone()).await.unwrap());
    assert_eq!(preview["mode"], "preview");
    let mut apply = args;
    apply["expected_revision"] = preview["revision"].clone();
    let applied = body(&call(apply).await.unwrap());
    assert_eq!(applied["mode"], "applied");
    (preview, applied)
}

fn rules() -> IpcEffectiveRoutingRules {
    ["GND", "VCC"]
        .into_iter()
        .map(|net| {
            (
                net.to_string(),
                IpcRoutingRules {
                    class_name: "Default".to_string(),
                    constituents: vec!["Default".to_string()],
                    track_width_mm: Some(0.25),
                    clearance_mm: Some(0.2),
                    via_diameter_mm: Some(0.6),
                    via_drill_mm: Some(0.3),
                },
            )
        })
        .collect()
}

/// What `export_specctra_dsn` builds from a board source, parsed back.
fn dsn_of(board: &Path) -> (dsn::Structure, Value) {
    let source = std::fs::read_to_string(board).unwrap();
    let bundle = crate::specctra::export_dsn(board, &source, &rules())
        .expect("the DSN export accepts the board");
    let mut tokenizer = ListTokenizer::new(BufReader::new(Cursor::new(bundle.dsn.as_bytes())));
    let file = dsn::DsnFile::read_dsn(&mut tokenizer).expect("the DSN parses");
    (
        file.pcb.structure,
        serde_json::from_str(&bundle.manifest).unwrap(),
    )
}

// ─── set_footprint_lock ───────────────────────────────────────────────────────

async fn lock(args: Value) -> anyhow::Result<CallToolResult> {
    handle_set_footprint_lock(&args, &offline()).await
}

#[tokio::test]
async fn reading_locks_lists_every_footprint_without_writing() {
    let (_dir, board) = scratch(FIXTURE);
    let result = body(&lock(json!({ "board": board })).await.unwrap());
    assert_eq!(result["mode"], "read");
    assert_eq!(result["count"], 2);
    assert_eq!(result["locked_count"], 0);
    let refs: Vec<&str> = result["footprints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["reference"].as_str().unwrap())
        .collect();
    assert_eq!(refs, ["R1", "R2"]);
    assert_eq!(std::fs::read_to_string(&board).unwrap(), FIXTURE);
}

#[tokio::test]
async fn lock_then_unlock_round_trips_the_file_byte_for_byte() {
    let (_dir, board) = scratch(FIXTURE);
    let args = json!({ "board": board, "references": ["R1"], "locked": true });
    let preview = body(&lock(args.clone()).await.unwrap());
    assert_eq!(preview["would_change"], 1);
    assert_eq!(
        std::fs::read_to_string(&board).unwrap(),
        FIXTURE,
        "a preview writes nothing"
    );

    let mut apply = args;
    apply["expected_revision"] = preview["revision"].clone();
    let applied = body(&lock(apply).await.unwrap());
    assert_eq!(applied["changed"], 1);
    assert_eq!(applied["readback"], "verified");
    let locked = std::fs::read_to_string(&board).unwrap();
    assert_eq!(locked.matches("(locked yes)").count(), 1);
    let read = body(
        &lock(json!({ "board": board, "references": ["R1", "R2"] }))
            .await
            .unwrap(),
    );
    assert_eq!(read["footprints"][0]["locked"], true);
    assert_eq!(read["footprints"][1]["locked"], false);

    let (_, unlocked) = preview_and_apply(
        json!({ "board": board, "references": ["R1"], "locked": false }),
        lock,
    )
    .await;
    assert_eq!(unlocked["changed"], 1);
    assert_eq!(std::fs::read_to_string(&board).unwrap(), FIXTURE);
}

#[tokio::test]
async fn an_already_locked_footprint_is_a_verified_no_op() {
    let source = FIXTURE.replacen(
        "(footprint \"Resistor_SMD:R_0402\"",
        "(footprint \"Resistor_SMD:R_0402\" (locked yes)",
        1,
    );
    let (_dir, board) = scratch(&source);
    let (preview, applied) = preview_and_apply(
        json!({ "board": board, "references": ["R1"], "locked": true }),
        lock,
    )
    .await;
    assert_eq!(preview["would_change"], 0);
    assert_eq!(applied["written"], false);
    assert_eq!(std::fs::read_to_string(&board).unwrap(), source);
}

#[tokio::test]
async fn the_kicad_6_bare_locked_token_reads_locked_and_unlocks() {
    let source = FIXTURE.replacen(
        "(footprint \"Resistor_SMD:R_0402\"",
        "(footprint \"Resistor_SMD:R_0402\" locked",
        1,
    );
    let (_dir, board) = scratch(&source);
    let read = body(&lock(json!({ "board": board })).await.unwrap());
    assert_eq!(read["footprints"][0]["locked"], true);
    preview_and_apply(
        json!({ "board": board, "references": ["R1"], "locked": false }),
        lock,
    )
    .await;
    assert_eq!(std::fs::read_to_string(&board).unwrap(), FIXTURE);
}

#[tokio::test]
async fn a_locked_no_flag_is_replaced_when_locking() {
    let source = FIXTURE.replacen(
        "(footprint \"Resistor_SMD:R_0402\"",
        "(footprint \"Resistor_SMD:R_0402\"\n\t\t(locked no)",
        1,
    );
    let (_dir, board) = scratch(&source);
    let read = body(&lock(json!({ "board": board })).await.unwrap());
    assert_eq!(read["footprints"][0]["locked"], false);
    preview_and_apply(
        json!({ "board": board, "references": ["R1"], "locked": true }),
        lock,
    )
    .await;
    let written = std::fs::read_to_string(&board).unwrap();
    assert!(!written.contains("(locked no)"), "{written}");
    assert_eq!(written.matches("(locked yes)").count(), 1);
}

#[tokio::test]
async fn an_unknown_or_duplicated_reference_is_refused_without_writing() {
    let (_dir, board) = scratch(FIXTURE);
    let unknown = lock(json!({ "board": board, "references": ["R9"], "locked": true }))
        .await
        .unwrap();
    assert_eq!(kind(&unknown), "invalid_argument");
    let missing = lock(json!({ "board": board, "locked": true }))
        .await
        .unwrap();
    assert_eq!(kind(&missing), "invalid_argument");

    let duplicated = FIXTURE.replace(
        "(property \"Reference\" \"R2\"",
        "(property \"Reference\" \"R1\"",
    );
    let (_dir2, board2) = scratch(&duplicated);
    let ambiguous = lock(json!({ "board": board2, "references": ["R1"], "locked": true }))
        .await
        .unwrap();
    assert_eq!(kind(&ambiguous), "ambiguous_target");
    assert_eq!(std::fs::read_to_string(&board).unwrap(), FIXTURE);
    assert_eq!(std::fs::read_to_string(&board2).unwrap(), duplicated);
}

#[tokio::test]
async fn a_board_changed_after_the_preview_is_stale_and_untouched() {
    let (_dir, board) = scratch(FIXTURE);
    let args = json!({ "board": board, "references": ["R1"], "locked": true });
    let preview = body(&lock(args.clone()).await.unwrap());
    let edited = FIXTURE.replace("no_route_keepout", "renamed_keepout");
    std::fs::write(&board, &edited).unwrap();
    let mut apply = args;
    apply["expected_revision"] = preview["revision"].clone();
    assert_eq!(kind(&lock(apply.clone()).await.unwrap()), "stale_target");
    assert_eq!(std::fs::read_to_string(&board).unwrap(), edited);

    // The revision also binds the request: unlocking with a lock preview's revision.
    std::fs::write(&board, FIXTURE).unwrap();
    apply["locked"] = json!(false);
    assert_eq!(kind(&lock(apply).await.unwrap()), "stale_target");
    assert_eq!(std::fs::read_to_string(&board).unwrap(), FIXTURE);
}

#[tokio::test]
async fn a_kicad_lock_file_refuses_the_write() {
    let (_dir, board) = scratch(FIXTURE);
    let lock_path = konnect_sexp::writer::kicad_editor_lock_path(&board).unwrap();
    std::fs::write(&lock_path, "user host").unwrap();
    let result = lock(json!({ "board": board, "references": ["R1"], "locked": true }))
        .await
        .unwrap();
    assert_eq!(kind(&result), "unsafe_file_fallback");
    assert_eq!(std::fs::read_to_string(&board).unwrap(), FIXTURE);
}

#[tokio::test]
async fn a_board_open_in_kicad_refuses_every_writer() {
    let (_dir, board) = scratch(FIXTURE);
    let server = board_mock::spawn_kicad_holding_board(&board, |_| None);
    let ctx = board_mock::ctx_talking_to(server.address().to_string());
    let results = [
        handle_set_footprint_lock(
            &json!({ "board": board, "references": ["R1"], "locked": true }),
            &ctx,
        )
        .await
        .unwrap(),
        handle_add_rule_area(
            &json!({ "board": board, "layers": ["F.Cu"], "x1": 90, "y1": 40, "x2": 95, "y2": 45 }),
            &ctx,
        )
        .await
        .unwrap(),
        handle_delete_zone(
            &json!({ "board": board, "names": ["no_route_keepout"] }),
            &ctx,
        )
        .await
        .unwrap(),
    ];
    for result in results {
        assert!(result.is_error);
        assert!(
            text(&result).contains("KiCAD currently holds this board open"),
            "{}",
            text(&result)
        );
    }
    assert_eq!(std::fs::read_to_string(&board).unwrap(), FIXTURE);
}

// ─── add_rule_area ────────────────────────────────────────────────────────────

async fn rule_area(args: Value) -> anyhow::Result<CallToolResult> {
    handle_add_rule_area(&args, &offline()).await
}

#[tokio::test]
async fn a_rule_area_survives_the_specctra_export_as_a_keepout_per_layer() {
    let (_dir, board) = scratch(FIXTURE);
    let (before, before_manifest) = dsn_of(&board);
    assert_eq!(before.keepouts.0.len(), 1);
    assert_eq!(
        before_manifest["supported_profile"]["zone_keepout_count"],
        1
    );

    let (preview, applied) = preview_and_apply(
        json!({
            "board": board, "layers": ["F.Cu", "B.Cu"], "name": "antenna",
            "points": [{ "x": 82, "y": 32 }, { "x": 90, "y": 32 }, { "x": 90, "y": 38 }, { "x": 86, "y": 40 }, { "x": 82, "y": 38 }]
        }),
        rule_area,
    )
    .await;
    assert_eq!(
        preview["rule_area"]["exported_to_specctra_as_keepout"],
        true
    );
    assert_eq!(applied["readback"], "verified");
    assert_eq!(applied["rule_area"]["keepout"]["tracks"], "not_allowed");
    assert_eq!(applied["rule_area"]["keepout"]["pads"], "allowed");

    let (after, manifest) = dsn_of(&board);
    assert_eq!(
        after.keepouts.0.len(),
        3,
        "one keepout per copper layer of the new area"
    );
    assert_eq!(manifest["supported_profile"]["zone_keepout_count"], 3);
    let mut layers: Vec<String> = after
        .keepouts
        .0
        .iter()
        .skip(1)
        .map(|keepout| {
            assert_eq!(keepout.kind, dsn::KeepoutKind::Normal);
            match &keepout.shape {
                dsn::Shape::Polygon(polygon) => {
                    // Five corners, closed: (82, 32) is (82000, -32000) in DSN.
                    assert_eq!(polygon.coords.len(), 6);
                    assert_eq!(
                        (polygon.coords[0].x, polygon.coords[0].y),
                        (82_000.0, -32_000.0)
                    );
                    polygon.layer.clone()
                }
                other => panic!("{other:?}"),
            }
        })
        .collect();
    layers.sort();
    assert_eq!(layers, ["B.Cu", "F.Cu"]);

    let listed = body(
        &handle_list_zones(&json!({ "board": board, "kind": "rule_area" }))
            .await
            .unwrap(),
    );
    assert_eq!(listed["count"], 2);
    let antenna = listed["zones"]
        .as_array()
        .unwrap()
        .iter()
        .find(|z| z["name"] == "antenna")
        .unwrap();
    assert_eq!(antenna["uuid"], applied["uuid"]);
    assert_eq!(antenna["layers"], json!(["F.Cu", "B.Cu"]));
    assert_eq!(antenna["corner_count"], 5);
}

#[tokio::test]
async fn keepout_flags_decide_the_specctra_kind() {
    let (_dir, board) = scratch(FIXTURE);
    preview_and_apply(
        json!({ "board": board, "layers": ["*.Cu"], "x1": 82, "y1": 32, "x2": 86, "y2": 36,
                "keepout": { "tracks": false, "copperpour": false } }),
        rule_area,
    )
    .await;
    preview_and_apply(
        json!({ "board": board, "layers": ["B.Cu"], "x1": 92, "y1": 32, "x2": 96, "y2": 36,
                "keepout": { "vias": false } }),
        rule_area,
    )
    .await;
    let (preview, _) = preview_and_apply(
        json!({ "board": board, "layers": ["F.Cu"], "x1": 100, "y1": 32, "x2": 104, "y2": 36,
                "keepout": { "tracks": false, "vias": false, "footprints": true } }),
        rule_area,
    )
    .await;
    assert_eq!(
        preview["rule_area"]["exported_to_specctra_as_keepout"],
        false
    );
    assert!(preview["note"]
        .as_str()
        .unwrap()
        .contains("Freerouting will route through it"));

    let (structure, _) = dsn_of(&board);
    let kinds: Vec<dsn::KeepoutKind> = structure
        .keepouts
        .0
        .iter()
        .skip(1)
        .map(|k| k.kind.clone())
        .collect();
    // Vias only on both layers ('*.Cu'), then tracks only on B.Cu; the
    // footprint-only area is nothing a router places.
    assert_eq!(
        kinds,
        [
            dsn::KeepoutKind::Via,
            dsn::KeepoutKind::Via,
            dsn::KeepoutKind::Wire
        ]
    );
}

#[tokio::test]
async fn bad_rule_areas_are_refused_before_anything_is_written() {
    let (_dir, board) = scratch(FIXTURE);
    let cases = [
        // Bowtie: edges 0 and 2 cross.
        json!({ "board": board, "layers": ["F.Cu"], "points": [{ "x": 0, "y": 0 }, { "x": 4, "y": 4 }, { "x": 4, "y": 0 }, { "x": 0, "y": 4 }] }),
        // Collinear.
        json!({ "board": board, "layers": ["F.Cu"], "points": [{ "x": 0, "y": 0 }, { "x": 1, "y": 0 }, { "x": 2, "y": 0 }] }),
        json!({ "board": board, "layers": ["F.SilkS"], "x1": 0, "y1": 0, "x2": 1, "y2": 1 }),
        json!({ "board": board, "layers": ["In1.Cu"], "x1": 0, "y1": 0, "x2": 1, "y2": 1 }),
        json!({ "board": board, "layers": ["F.Cu"], "x1": 0, "y1": 0, "x2": 1, "y2": 1,
                "keepout": { "tracks": false, "vias": false, "copperpour": false } }),
        json!({ "board": board, "layers": ["F.Cu"], "x1": 0, "y1": 0, "x2": 1, "y2": 1, "name": "no_route_keepout" }),
        json!({ "board": board, "layers": ["F.Cu"], "x1": 0, "y1": 0, "x2": 1 }),
        json!({ "board": board, "layers": ["F.Cu"], "x1": 0, "y1": 0, "x2": 1, "y2": 1, "points": [] }),
        json!({ "board": board, "layers": ["F.Cu"], "x1": 0, "y1": 0, "x2": 1, "y2": 1, "keepout": { "wires": true } }),
    ];
    for case in cases {
        let result = rule_area(case.clone()).await.unwrap();
        assert_eq!(kind(&result), "invalid_argument", "{case}");
    }
    assert_eq!(std::fs::read_to_string(&board).unwrap(), FIXTURE);
}

#[tokio::test]
async fn a_rule_area_preview_does_not_authorise_a_different_area() {
    let (_dir, board) = scratch(FIXTURE);
    let args =
        json!({ "board": board, "layers": ["F.Cu"], "x1": 82, "y1": 32, "x2": 86, "y2": 36 });
    let preview = body(&rule_area(args.clone()).await.unwrap());
    let mut other = args;
    other["x2"] = json!(88);
    other["expected_revision"] = preview["revision"].clone();
    assert_eq!(kind(&rule_area(other).await.unwrap()), "stale_target");
    assert_eq!(std::fs::read_to_string(&board).unwrap(), FIXTURE);
}

/// KiCad's own pcbnew loads what `add_rule_area` and `set_footprint_lock`
/// write. Needs KiCad 10's bundled Python; run with `--ignored`.
#[tokio::test]
#[ignore = "needs KiCad 10's bundled Python (pcbnew)"]
async fn live_pcbnew_loads_the_written_rule_area_and_lock() {
    let python = "/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3";
    let (_dir, board) = scratch(FIXTURE);
    preview_and_apply(
        json!({ "board": board, "layers": ["F.Cu", "B.Cu"], "name": "antenna",
                "points": [{ "x": 82, "y": 32 }, { "x": 90, "y": 32 }, { "x": 86, "y": 40 }],
                "keepout": { "vias": false, "footprints": true } }),
        rule_area,
    )
    .await;
    preview_and_apply(
        json!({ "board": board, "references": ["R2"], "locked": true }),
        lock,
    )
    .await;
    let script = "import pcbnew,sys\n\
        b=pcbnew.LoadBoard(sys.argv[1])\n\
        z=[z for z in b.Zones() if z.GetZoneName()=='antenna'][0]\n\
        print(z.GetIsRuleArea(), z.GetDoNotAllowTracks(), z.GetDoNotAllowVias(), z.GetDoNotAllowPads(), z.GetDoNotAllowZoneFills(), z.GetDoNotAllowFootprints(), z.Outline().FullPointCount(), z.GetLayerSet().CuStack().size())\n\
        print([f.GetReference() for f in b.GetFootprints() if f.IsLocked()])\n";
    let output = std::process::Command::new(python)
        .args(["-c", script, board.to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        stdout.lines().next(),
        Some("True True False False True True 3 2"),
        "{stdout}"
    );
    assert_eq!(stdout.lines().nth(1), Some("['R2']"), "{stdout}");
}

// ─── list_zones / delete_zone ─────────────────────────────────────────────────

async fn delete(args: Value) -> anyhow::Result<CallToolResult> {
    handle_delete_zone(&args, &offline()).await
}

#[tokio::test]
async fn list_zones_reports_the_pour_and_the_keepout() {
    let (_dir, board) = scratch(FIXTURE);
    let listed = body(&handle_list_zones(&json!({ "board": board })).await.unwrap());
    assert_eq!(listed["count"], 2);
    assert_eq!(listed["copper_count"], 1);
    assert_eq!(listed["rule_area_count"], 1);
    let zones = listed["zones"].as_array().unwrap();
    assert_eq!(zones[0]["kind"], "rule_area");
    assert_eq!(zones[0]["name"], "no_route_keepout");
    assert_eq!(zones[0]["net"], Value::Null);
    assert_eq!(zones[0]["keepout"]["vias"], "not_allowed");
    assert_eq!(zones[1]["kind"], "copper");
    assert_eq!(zones[1]["net"], "GND");
    assert_eq!(zones[1]["layers"], json!(["B.Cu"]));
    assert_eq!(
        zones[1]["bbox"],
        json!({ "x1": 85.0, "y1": 35.0, "x2": 125.0, "y2": 65.0 })
    );
    let copper = body(
        &handle_list_zones(&json!({ "board": board, "kind": "copper" }))
            .await
            .unwrap(),
    );
    assert_eq!(copper["count"], 1);
}

#[tokio::test]
async fn deleting_the_keepout_by_name_removes_it_from_the_specctra_export() {
    let (_dir, board) = scratch(FIXTURE);
    let (preview, applied) = preview_and_apply(
        json!({ "board": board, "names": ["no_route_keepout"] }),
        delete,
    )
    .await;
    assert_eq!(preview["would_delete"], 1);
    assert_eq!(preview["copper_zones_remaining"], true);
    assert_eq!(applied["deleted"], 1);
    assert_eq!(applied["readback"], "verified");
    let written = std::fs::read_to_string(&board).unwrap();
    assert!(!written.contains("no_route_keepout"));
    assert!(
        written.contains("44444444-4444-4444-8444-444444444441"),
        "the pour stays"
    );
    let (structure, manifest) = dsn_of(&board);
    assert!(structure.keepouts.0.is_empty());
    assert_eq!(structure.planes.len(), 1);
    assert_eq!(manifest["supported_profile"]["zone_keepout_count"], 0);
}

#[tokio::test]
async fn deleting_by_uuid_takes_exactly_that_zone() {
    let (_dir, board) = scratch(FIXTURE);
    let (_, applied) = preview_and_apply(
        json!({ "board": board, "uuids": ["44444444-4444-4444-8444-444444444441"] }),
        delete,
    )
    .await;
    assert_eq!(applied["zones"][0]["kind"], "copper");
    let listed = body(&handle_list_zones(&json!({ "board": board })).await.unwrap());
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["zones"][0]["name"], "no_route_keepout");
    // The rest of the file is what it was, minus that one block.
    let written = std::fs::read_to_string(&board).unwrap();
    assert_eq!(
        top_level_blocks(&written).unwrap().len() + 1,
        top_level_blocks(FIXTURE).unwrap().len()
    );
}

#[tokio::test]
async fn delete_refuses_unknown_ambiguous_and_empty_targets() {
    let (_dir, board) = scratch(FIXTURE);
    assert_eq!(
        kind(&delete(json!({ "board": board })).await.unwrap()),
        "invalid_argument"
    );
    assert_eq!(
        kind(
            &delete(json!({ "board": board, "uuids": ["not-a-zone"] }))
                .await
                .unwrap()
        ),
        "invalid_argument"
    );
    assert_eq!(
        kind(
            &delete(json!({ "board": board, "names": ["nothing"] }))
                .await
                .unwrap()
        ),
        "invalid_argument"
    );
    // Name the pour like the keepout: the name no longer picks one zone.
    let twin = FIXTURE.replace(
        "(uuid \"44444444-4444-4444-8444-444444444441\")",
        "(uuid \"44444444-4444-4444-8444-444444444441\")\n\t\t(name \"no_route_keepout\")",
    );
    let (_dir2, board2) = scratch(&twin);
    assert_eq!(
        kind(
            &delete(json!({ "board": board2, "names": ["no_route_keepout"] }))
                .await
                .unwrap()
        ),
        "ambiguous_target"
    );
    assert_eq!(std::fs::read_to_string(&board).unwrap(), FIXTURE);
    assert_eq!(std::fs::read_to_string(&board2).unwrap(), twin);
}

// ─── add_board_outline, polygon mode ──────────────────────────────────────────

async fn outline(args: Value) -> CallToolResult {
    super::super::handle_add_board_outline(&args, &offline())
        .await
        .unwrap()
}

/// The fixture without its rounded-rectangle outline.
fn without_outline() -> String {
    let blocks = top_level_blocks(FIXTURE).unwrap();
    let edits = blocks
        .iter()
        .filter(|b| {
            matches!(b.node.head(), Some("gr_line" | "gr_arc"))
                && b.node.find_str("layer") == Some("Edge.Cuts")
        })
        .map(|b| {
            let (start, end) = find_block_with_leading_whitespace(FIXTURE, b.start).unwrap();
            SexpEdit::delete(start, end)
        })
        .collect();
    apply_edits(FIXTURE.to_string(), edits)
}

#[tokio::test]
async fn a_polygon_outline_with_an_arc_edge_is_one_loop_the_dsn_export_accepts() {
    let source = without_outline();
    assert!(
        !source.contains("(layer \"Edge.Cuts\")"),
        "the old outline is gone"
    );
    let (_dir, board) = scratch(&source);
    // An L-shaped board whose top edge bulges out as an arc.
    let result = body(
        &outline(json!({
            "board": board,
            "points": [
                { "x": 80, "y": 30, "arc_mid": { "x": 105, "y": 26 } },
                { "x": 130, "y": 30 }, { "x": 130, "y": 70 }, { "x": 100, "y": 70 },
                { "x": 100, "y": 60 }, { "x": 80, "y": 60 }
            ]
        }))
        .await,
    );
    assert_eq!(result["source"], "file");
    assert_eq!(result["readback"], "verified");
    assert_eq!(
        (result["line_count"].as_u64(), result["arc_count"].as_u64()),
        (Some(5), Some(1))
    );
    assert!(result.get("existing_edge_cuts_shapes").is_none());
    let written = std::fs::read_to_string(&board).unwrap();
    assert_eq!(written.matches("(gr_arc").count(), 1);
    // The outline reader behind the design review and score_placement sees
    // one closed loop with the L's notch outside it.
    let outline = konnect_sexp::outline::Outline::from_tree(&parse_sexp(&written).unwrap());
    assert_eq!(
        outline.status(),
        konnect_sexp::outline::OutlineStatus::Closed
    );
    assert_eq!(outline.loops.len(), 1);
    assert!(outline.contains((115.0, 65.0)));
    assert!(
        !outline.contains((90.0, 65.0)),
        "the notch is off the board"
    );
    assert!(
        outline.contains((105.0, 28.0)),
        "the arc bulge is on the board"
    );
    let (structure, _) = dsn_of(&board);
    match structure.boundary {
        dsn::Boundary::Path(path) => {
            let ys: Vec<f64> = path.coords.iter().map(|p| p.y).collect();
            // The arc bulges above y = 30 (DSN y is negated): the chorded
            // boundary reaches past -30 000.
            assert!(ys.iter().any(|y| *y > -30_000.0 + 1.0), "{ys:?}");
            assert!(ys.contains(&-70_000.0));
        }
        other => panic!("unexpected boundary {other:?}"),
    }
}

#[tokio::test]
async fn a_second_outline_is_written_with_a_note() {
    let (_dir, board) = scratch(FIXTURE);
    let result = body(
        &outline(json!({
            "board": board,
            "points": [{ "x": 0, "y": 0 }, { "x": 10, "y": 0 }, { "x": 5, "y": 8 }, { "x": 0, "y": 0 }]
        }))
        .await,
    );
    assert_eq!(
        result["corner_count"], 3,
        "a closing corner equal to the first is dropped"
    );
    assert_eq!(result["existing_edge_cuts_shapes"], 8);
    assert!(result["note"]
        .as_str()
        .unwrap()
        .contains("more than one outline"));
}

#[tokio::test]
async fn bad_polygon_outlines_are_refused_without_writing() {
    let (_dir, board) = scratch(FIXTURE);
    let cases = [
        json!({ "board": board, "points": [{ "x": 0, "y": 0 }, { "x": 4, "y": 4 }, { "x": 4, "y": 0 }, { "x": 0, "y": 4 }] }),
        json!({ "board": board, "points": [{ "x": 0, "y": 0 }, { "x": 4, "y": 0 }] }),
        json!({ "board": board, "points": [{ "x": 0, "y": 0, "arc_mid": { "x": 2, "y": 0 } }, { "x": 4, "y": 0 }, { "x": 4, "y": 4 }] }),
        // An arc edge whose circle swings out across both side edges (at
        // y = 5.33): only sampling along the arc sees it; its chords do not.
        json!({ "board": board, "points": [{ "x": 0, "y": 0, "arc_mid": { "x": 2, "y": 6 } }, { "x": 4, "y": 0 }, { "x": 4, "y": 8 }, { "x": 0, "y": 8 }] }),
        json!({ "board": board, "points": [{ "x": 0, "y": 0 }, { "x": 4, "y": 0 }, { "x": 4, "y": 4 }], "x1": 0 }),
        json!({ "board": board, "points": [{ "x": 0, "y": 0 }, { "x": 4, "y": 0 }, { "x": 4, "y": 4 }], "corner_radius": 1 }),
    ];
    for case in cases {
        let result = outline(case.clone()).await;
        assert_eq!(kind(&result), "invalid_argument", "{case}");
    }
    assert_eq!(std::fs::read_to_string(&board).unwrap(), FIXTURE);
}

#[test]
fn arc_samples_follow_the_side_the_midpoint_is_on() {
    // A half circle of radius 2 above the x axis (KiCad Y grows down, but
    // the geometry does not care): every sample is on the circle, above.
    let up = arc_inner_points((0.0, 0.0), (2.0, 2.0), (4.0, 0.0));
    assert!(up.len() > 10);
    for (x, y) in &up {
        assert!(((x - 2.0).powi(2) + y.powi(2) - 4.0).abs() < 1e-9);
        assert!(*y > 0.0);
    }
    let down = arc_inner_points((0.0, 0.0), (2.0, -2.0), (4.0, 0.0));
    assert!(down.iter().all(|(_, y)| *y < 0.0));
    // A three-quarter arc goes the long way round.
    let long = arc_inner_points((2.0, 0.0), (-2.0, 0.0), (0.0, -2.0));
    assert!(long.iter().any(|(x, y)| *x < -1.9 && y.abs() < 0.2));
    assert!(long.iter().all(|(x, y)| !(*x > 0.5 && *y < -0.5)));
}

#[test]
fn simple_ring_check_accepts_concave_and_refuses_degenerate_rings() {
    // Concave L.
    assert!(check_simple_ring(&[
        (0.0, 0.0),
        (4.0, 0.0),
        (4.0, 1.0),
        (1.0, 1.0),
        (1.0, 4.0),
        (0.0, 4.0)
    ])
    .is_ok());
    // A spike folding back onto its own edge.
    assert!(check_simple_ring(&[(0.0, 0.0), (4.0, 0.0), (2.0, 0.0), (2.0, 3.0)]).is_err());
    // A corner touching a non-adjacent edge.
    assert!(
        check_simple_ring(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (2.0, 0.0), (0.0, 4.0)]).is_err()
    );
    assert!(check_simple_ring(&[(0.0, 0.0), (f64::NAN, 0.0), (1.0, 1.0)]).is_err());
    assert!(check_simple_ring(&[(0.0, 0.0), (1.0, 0.0), (0.0, 0.0), (1.0, 1.0)]).is_err());
}
