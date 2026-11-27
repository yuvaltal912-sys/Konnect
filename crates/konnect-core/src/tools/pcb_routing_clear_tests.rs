//! `delete_all_routing` against a mock KiCad that holds the board as text and deletes
//! items by KIID, as KiCad's `DeleteItems` does. One test per guard, plus the reads.
use super::*;
use crate::mcp::error::extract_error_kind;
use crate::mcp::protocol::ToolContent;
use crate::tools::pcb_board::board_mock::{ctx_talking_to, spawn_kicad_holding_board};
use konnect_ipc::gen::kiapi;
use prost::Message;
use std::sync::{Arc, Mutex};

const FIXTURE: &str = include_str!("../../tests/fixtures/specctra_two_resistors.kicad_pcb");

const SEGMENT_GND: &str = r#"(segment (start 100.5 50) (end 105 50) (width 0.25) (layer "F.Cu") (net "GND") (uuid "s-1"))"#;
const SEGMENT_VCC: &str =
    r#"(segment (start 110 50) (end 112 50) (width 0.25) (layer "B.Cu") (net "VCC") (uuid "s-2"))"#;
const ARC_VCC: &str = r#"(arc (start 110 52) (mid 111 53) (end 112 52) (width 0.25) (layer "F.Cu") (net "VCC") (uuid "a-1"))"#;
const VIA_GND: &str =
    r#"(via (at 105 50) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net "GND") (uuid "v-1"))"#;
const SEGMENT_LOCKED: &str = r#"(segment (start 100 55) (end 104 55) (width 0.25) (locked yes) (layer "F.Cu") (net "GND") (uuid "s-locked"))"#;
const VIA_LOCKED: &str = r#"(via (at 104 55) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (locked yes) (net "GND") (uuid "v-locked"))"#;
/// A pour whose outline holds an `(arc …)`: geometry, not a track.
const ZONE: &str = r#"(zone (net "GND") (layer "B.Cu") (uuid "z-1") (hatch edge 0.5) (connect_pads (clearance 0.2)) (min_thickness 0.2) (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.5)) (polygon (pts (xy 90 40) (arc (start 120 40) (mid 125 50) (end 120 60)) (xy 90 60))) (filled_polygon (layer "B.Cu") (pts (xy 91 41) (xy 119 41) (xy 119 59))))"#;

const UNLOCKED: [&str; 4] = ["s-1", "s-2", "a-1", "v-1"];

/// The board: the fixture with `items` before its closing parenthesis.
fn board_text(items: &[&str]) -> String {
    let end = FIXTURE.rfind(')').unwrap();
    let mut text = FIXTURE[..end].to_string();
    for item in items {
        text.push('\t');
        text.push_str(item);
        text.push('\n');
    }
    text.push_str(&FIXTURE[end..]);
    text
}

/// What the mock KiCad does wrong, one fault per guard of the read-back.
#[derive(Default, Clone, Copy)]
struct Fault {
    /// `DeleteItems` leaves this KIID on the board.
    keep: Option<&'static str>,
    /// `DeleteItems` also deletes the pour.
    delete_zone: bool,
    /// `DeleteItems` also deletes the locked track.
    delete_locked: bool,
    /// The read-back after the delete fails.
    fail_readback: bool,
}

struct Harness {
    _dir: tempfile::TempDir,
    board: PathBuf,
    saved: Vec<u8>,
    items: Arc<Mutex<Vec<String>>>,
    deleted: Arc<Mutex<Vec<Vec<String>>>>,
    commits: Arc<Mutex<usize>>,
    _server: crate::test_support::MockIpcServer,
    ctx: ToolContext,
}

fn uuid_of(item: &str) -> String {
    let at = item.find("(uuid \"").unwrap() + 7;
    item[at..at + item[at..].find('"').unwrap()].to_string()
}

impl Harness {
    fn new(items: &[&str], fault: Fault) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let board = dir.path().join("routing.kicad_pcb");
        let saved = board_text(items).into_bytes();
        std::fs::write(&board, &saved).unwrap();
        let held = Arc::new(Mutex::new(
            items
                .iter()
                .map(|item| item.to_string())
                .collect::<Vec<_>>(),
        ));
        let deleted = Arc::new(Mutex::new(Vec::new()));
        let commits = Arc::new(Mutex::new(0));
        let deletes_after = Arc::new(Mutex::new(false));
        let (held_in, deleted_in, commits_in) = (held.clone(), deleted.clone(), commits.clone());
        let server = spawn_kicad_holding_board(&board, move |command| {
            if command.type_url.ends_with("SaveDocumentToString") {
                let request =
                    kiapi::common::commands::SaveDocumentToString::decode(command.value.as_slice())
                        .unwrap();
                if fault.fail_readback && *deletes_after.lock().unwrap() {
                    return None;
                }
                let items = held_in.lock().unwrap().clone();
                let refs = items.iter().map(String::as_str).collect::<Vec<_>>();
                return Some(konnect_ipc::builders::pack_any(
                    &kiapi::common::commands::SavedDocumentResponse {
                        document: request.document,
                        contents: board_text(&refs),
                    },
                    "kiapi.common.commands.SavedDocumentResponse",
                ));
            }
            if command.type_url.ends_with("BeginCommit") {
                *commits_in.lock().unwrap() += 1;
                return Some(konnect_ipc::builders::pack_any(
                    &kiapi::common::commands::BeginCommitResponse {
                        id: Some(kiapi::common::types::Kiid {
                            value: "commit-1".into(),
                        }),
                    },
                    "kiapi.common.commands.BeginCommitResponse",
                ));
            }
            if command.type_url.ends_with("EndCommit") {
                return Some(konnect_ipc::builders::pack_any(
                    &kiapi::common::commands::EndCommitResponse {},
                    "kiapi.common.commands.EndCommitResponse",
                ));
            }
            if command.type_url.ends_with("DeleteItems") {
                let request =
                    kiapi::common::commands::DeleteItems::decode(command.value.as_slice()).unwrap();
                let ids: Vec<String> = request.item_ids.into_iter().map(|id| id.value).collect();
                held_in.lock().unwrap().retain(|item| {
                    let id = uuid_of(item);
                    let asked = ids.contains(&id) && fault.keep != Some(id.as_str());
                    let zone = fault.delete_zone && id == "z-1";
                    let locked = fault.delete_locked && id == "s-locked";
                    !(asked || zone || locked)
                });
                deleted_in.lock().unwrap().push(ids);
                *deletes_after.lock().unwrap() = true;
                return Some(konnect_ipc::builders::pack_any(
                    &kiapi::common::commands::DeleteItemsResponse {
                        header: None,
                        status: kiapi::common::types::ItemRequestStatus::IrsOk as i32,
                        deleted_items: vec![],
                    },
                    "kiapi.common.commands.DeleteItemsResponse",
                ));
            }
            None
        });
        let ctx = ctx_talking_to(server.address().to_string());
        Self {
            _dir: dir,
            board,
            saved,
            items: held,
            deleted,
            commits,
            _server: server,
            ctx,
        }
    }

    fn full() -> Self {
        Self::new(
            &[
                SEGMENT_GND,
                SEGMENT_VCC,
                ARC_VCC,
                VIA_GND,
                SEGMENT_LOCKED,
                VIA_LOCKED,
                ZONE,
            ],
            Fault::default(),
        )
    }

    async fn call(&self, extra: Value) -> (bool, Option<String>, Value) {
        let mut args = json!({ "board": self.board.to_string_lossy() });
        for (key, value) in extra.as_object().unwrap() {
            args[key] = value.clone();
        }
        let result = handle_delete_all_routing(&args, &self.ctx).await.unwrap();
        let text = match result.content.first() {
            Some(ToolContent::Text { text }) => text.clone(),
            other => panic!("expected text, got {other:?}"),
        };
        let body = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "text": text }));
        (result.is_error, extract_error_kind(&result), body)
    }

    async fn preview(&self, extra: Value) -> String {
        let (error, _, body) = self.call(extra).await;
        assert!(!error, "{body}");
        body["plan_revision"].as_str().unwrap().to_string()
    }

    fn held_ids(&self) -> Vec<String> {
        self.items
            .lock()
            .unwrap()
            .iter()
            .map(|i| uuid_of(i))
            .collect()
    }

    fn untouched(&self) {
        assert_eq!(*self.commits.lock().unwrap(), 0, "a commit was opened");
        assert!(
            self.deleted.lock().unwrap().is_empty(),
            "DeleteItems was sent"
        );
        assert_eq!(std::fs::read(&self.board).unwrap(), self.saved);
    }
}

fn row<'a>(rows: &'a Value, key: &str, name: &str) -> &'a Value {
    rows.as_array()
        .unwrap()
        .iter()
        .find(|row| row[key] == name)
        .unwrap_or_else(|| panic!("no {key} {name} in {rows}"))
}

// ─── The guards ──────────────────────────────────────────────────────────────

/// Guard: the preview writes nothing — no commit, no `DeleteItems`, no file write.
#[tokio::test]
async fn the_preview_counts_by_kind_net_and_layer_and_writes_nothing() {
    let h = Harness::full();
    for args in [json!({}), json!({ "dry_run": true })] {
        let (error, _, body) = h.call(args).await;
        assert!(!error, "{body}");
        assert_eq!(body["status"], "ready");
        assert_eq!(body["readback"], "not_written");
        let will = &body["will_delete"];
        assert_eq!(
            (
                will["total"].as_u64(),
                will["tracks"].as_u64(),
                will["arcs"].as_u64(),
                will["vias"].as_u64()
            ),
            (Some(4), Some(2), Some(1), Some(1)),
            "{body}"
        );
        let gnd = row(&will["by_net"], "net", "GND");
        assert_eq!(
            (gnd["tracks"].as_u64(), gnd["vias"].as_u64()),
            (Some(1), Some(1))
        );
        let vcc = row(&will["by_net"], "net", "VCC");
        assert_eq!(
            (vcc["tracks"].as_u64(), vcc["arcs"].as_u64()),
            (Some(1), Some(1))
        );
        let front = row(&will["by_layer"], "layer", "F.Cu");
        assert_eq!(
            (front["tracks"].as_u64(), front["arcs"].as_u64()),
            (Some(1), Some(1))
        );
        assert_eq!(row(&will["by_layer"], "layer", "B.Cu")["tracks"], 1);
        assert_eq!(row(&will["by_layer"], "layer", "F.Cu-B.Cu")["vias"], 1);
        let kept = &body["kept_locked"];
        assert_eq!(
            (kept["tracks"].as_u64(), kept["vias"].as_u64()),
            (Some(1), Some(1))
        );
        assert!(body["next_step"]
            .as_str()
            .unwrap()
            .contains(body["plan_revision"].as_str().unwrap()));
    }
    h.untouched();
    assert_eq!(h.held_ids().len(), 7);
}

/// Guard: the apply names the preview's revision; a board that changed since is refused
/// before any commit.
#[tokio::test]
async fn a_stale_revision_is_refused_before_any_commit() {
    let h = Harness::full();
    let revision = h.preview(json!({})).await;
    // The board changes in KiCad after the preview: one more track.
    h.items.lock().unwrap().push(
        r#"(segment (start 1 1) (end 2 2) (width 0.2) (layer "F.Cu") (net "GND") (uuid "s-new"))"#
            .to_string(),
    );
    let (error, kind, body) = h
        .call(json!({ "dry_run": false, "expected_plan_revision": revision }))
        .await;
    assert!(error, "{body}");
    assert_eq!(kind.as_deref(), Some("stale_target"), "{body}");
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("Nothing was deleted"));
    // A made-up revision is refused the same way.
    let (error, kind, _) = h
        .call(json!({ "dry_run": false, "expected_plan_revision": "not-a-revision" }))
        .await;
    assert!(error);
    assert_eq!(kind.as_deref(), Some("stale_target"));
    h.untouched();
    assert_eq!(h.held_ids().len(), 8);
}

/// Guard: locked items stay, and only routing KIIDs are sent to `DeleteItems`.
#[tokio::test]
async fn the_apply_deletes_only_unlocked_routing_in_one_commit() {
    let h = Harness::full();
    let revision = h.preview(json!({})).await;
    let (error, _, body) = h
        .call(json!({ "dry_run": false, "expected_plan_revision": revision }))
        .await;
    assert!(!error, "{body}");
    assert_eq!(body["status"], "applied", "{body}");
    assert_eq!(*h.commits.lock().unwrap(), 1);
    let sent = h.deleted.lock().unwrap().clone();
    assert_eq!(sent.len(), 1, "one DeleteItems call");
    let mut ids = sent[0].clone();
    ids.sort();
    let mut want = UNLOCKED.map(str::to_string).to_vec();
    want.sort();
    assert_eq!(ids, want, "nothing but unlocked routing is sent");
    assert_eq!(body["removed"]["total"], 4);
    assert_eq!(body["routing_left"]["total"], 2);
    assert_eq!(body["routing_left"]["tracks"], 1);
    assert_eq!(body["routing_left"]["vias"], 1);
    let mut held = h.held_ids();
    held.sort();
    assert_eq!(held, ["s-locked", "v-locked", "z-1"]);
    // The saved file is never written: the change lives in KiCad until a save.
    assert_eq!(std::fs::read(&h.board).unwrap(), h.saved);
}

/// Guard: `include_locked` deletes locked routing too, and its preview has its own
/// revision, so a preview without it cannot authorize it.
#[tokio::test]
async fn include_locked_needs_its_own_preview_and_deletes_the_locked_items() {
    let h = Harness::full();
    let plain = h.preview(json!({})).await;
    let with_locked = h.preview(json!({ "include_locked": true })).await;
    assert_ne!(plain, with_locked);
    let (error, kind, _) = h
        .call(json!({ "dry_run": false, "include_locked": true, "expected_plan_revision": plain }))
        .await;
    assert!(error);
    assert_eq!(kind.as_deref(), Some("stale_target"));
    h.untouched();
    let (error, _, body) = h
        .call(json!({ "dry_run": false, "include_locked": true, "expected_plan_revision": with_locked }))
        .await;
    assert!(!error, "{body}");
    assert_eq!(body["removed"]["total"], 6);
    assert_eq!(body["routing_left"]["total"], 0);
    assert_eq!(h.held_ids(), ["z-1"]);
}

/// Guard: the read-back refuses to call a delete clean when KiCad also removed a pour.
#[tokio::test]
async fn a_pour_gone_after_the_delete_is_an_uncertain_outcome() {
    let h = Harness::new(
        &[SEGMENT_GND, VIA_GND, ZONE],
        Fault {
            delete_zone: true,
            ..Fault::default()
        },
    );
    let revision = h.preview(json!({})).await;
    let (error, kind, body) = h
        .call(json!({ "dry_run": false, "expected_plan_revision": revision }))
        .await;
    assert!(error, "{body}");
    assert_eq!(
        kind.as_deref(),
        Some("mutation_outcome_uncertain"),
        "{body}"
    );
    let text = body["message"].as_str().unwrap();
    assert!(text.contains("other objects changed: zone"), "{text}");
}

/// Guard: the read-back names a planned item KiCad kept, and a locked item it lost.
#[tokio::test]
async fn an_item_left_behind_or_a_locked_item_lost_is_an_uncertain_outcome() {
    let h = Harness::new(
        &[SEGMENT_GND, VIA_GND],
        Fault {
            keep: Some("v-1"),
            ..Fault::default()
        },
    );
    let revision = h.preview(json!({})).await;
    let (error, kind, body) = h
        .call(json!({ "dry_run": false, "expected_plan_revision": revision }))
        .await;
    assert!(error);
    assert_eq!(kind.as_deref(), Some("mutation_outcome_uncertain"));
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("still on the board: 1 (v-1)"));

    let h = Harness::new(
        &[SEGMENT_GND, SEGMENT_LOCKED],
        Fault {
            delete_locked: true,
            ..Fault::default()
        },
    );
    let revision = h.preview(json!({})).await;
    let (error, kind, body) = h
        .call(json!({ "dry_run": false, "expected_plan_revision": revision }))
        .await;
    assert!(error);
    assert_eq!(kind.as_deref(), Some("mutation_outcome_uncertain"));
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("locked items that were to stay are gone: 1 (s-locked)"));
}

/// A read-back that fails after the commit is uncertain, never a refusal.
#[tokio::test]
async fn a_failed_read_back_after_the_commit_is_uncertain() {
    let h = Harness::new(
        &[SEGMENT_GND],
        Fault {
            fail_readback: true,
            ..Fault::default()
        },
    );
    let revision = h.preview(json!({})).await;
    let (error, kind, body) = h
        .call(json!({ "dry_run": false, "expected_plan_revision": revision }))
        .await;
    assert!(error);
    assert_eq!(
        kind.as_deref(),
        Some("mutation_outcome_uncertain"),
        "{body}"
    );
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("read-back failed"));
}

// ─── Arguments, empty boards, a board that is not open ──────────────────────

#[tokio::test]
async fn an_apply_without_a_revision_or_with_a_bad_flag_is_refused_before_kicad() {
    let h = Harness::full();
    for args in [
        json!({ "dry_run": false }),
        json!({ "dry_run": "false", "expected_plan_revision": "x" }),
        json!({ "include_locked": "yes" }),
        json!({ "dry_run": false, "expected_plan_revision": "" }),
    ] {
        let (error, kind, body) = h.call(args.clone()).await;
        assert!(error, "{args}: {body}");
        assert_eq!(kind.as_deref(), Some("invalid_argument"), "{args}");
    }
    let (error, kind, _) = handle_delete_all_routing(&json!({ "board": "x.kicad_sch" }), &h.ctx)
        .await
        .map(|r| (r.is_error, extract_error_kind(&r), ()))
        .unwrap();
    assert!(error);
    assert_eq!(kind.as_deref(), Some("invalid_argument"));
    h.untouched();
}

#[tokio::test]
async fn a_board_without_routing_is_a_noop_with_no_commit() {
    let h = Harness::new(&[ZONE], Fault::default());
    let (error, _, body) = h.call(json!({})).await;
    assert!(!error, "{body}");
    assert_eq!(body["status"], "noop");
    let revision = body["plan_revision"].as_str().unwrap().to_string();
    let (error, _, body) = h
        .call(json!({ "dry_run": false, "expected_plan_revision": revision }))
        .await;
    assert!(!error, "{body}");
    assert_eq!(body["status"], "noop");
    assert_eq!(body["removed"]["total"], 0);
    h.untouched();
}

#[tokio::test]
async fn a_board_kicad_does_not_hold_is_refused_and_never_edited() {
    let dir = tempfile::tempdir().unwrap();
    let other = dir.path().join("other.kicad_pcb");
    let board = dir.path().join("routing.kicad_pcb");
    let text = board_text(&[SEGMENT_GND]);
    std::fs::write(&other, &text).unwrap();
    std::fs::write(&board, &text).unwrap();
    let server = spawn_kicad_holding_board(&other, |_| None);
    let result = handle_delete_all_routing(
        &json!({ "board": board.to_string_lossy(), "dry_run": false, "expected_plan_revision": "x" }),
        &ctx_talking_to(server.address().to_string()),
    )
    .await
    .unwrap();
    assert!(result.is_error);
    let Some(ToolContent::Text { text: message }) = result.content.first() else {
        panic!("expected text");
    };
    assert!(message.contains("Nothing was deleted"), "{message}");
    assert_eq!(std::fs::read_to_string(&board).unwrap(), text);
}

// ─── The plan, pure ──────────────────────────────────────────────────────────

#[test]
fn only_top_level_tracks_arcs_and_vias_are_routing() {
    let text = board_text(&[SEGMENT_GND, ARC_VCC, VIA_GND, ZONE]);
    let (plan, reading) = Plan::of(Path::new("b.kicad_pcb"), &text, false).unwrap();
    // The zone outline's `(arc …)`, the footprints' pads and the board's gr_lines are
    // not routing.
    assert_eq!(reading.routing.len(), 3);
    let kinds: Vec<Kind> = plan.delete.iter().map(|i| i.kind).collect();
    assert_eq!(kinds, [Kind::Track, Kind::Arc, Kind::Via]);
    assert_eq!(plan.ids(), ["s-1", "a-1", "v-1"]);
    assert!(reading.fixed.iter().any(|f| f.contains("\"zone\"")));
    assert!(reading.fixed.iter().any(|f| f.contains("\"footprint\"")));
    assert!(reading.fixed.iter().any(|f| f.contains("\"gr_line\"")));
}

#[test]
fn the_revision_moves_with_the_board_the_path_and_the_locked_choice() {
    let a = board_text(&[SEGMENT_GND]);
    let b = board_text(&[SEGMENT_GND, VIA_GND]);
    let path = Path::new("b.kicad_pcb");
    assert_eq!(
        plan_revision(path, false, &a),
        plan_revision(path, false, &a)
    );
    assert_ne!(
        plan_revision(path, false, &a),
        plan_revision(path, false, &b)
    );
    assert_ne!(
        plan_revision(path, false, &a),
        plan_revision(path, true, &a)
    );
    assert_ne!(
        plan_revision(path, false, &a),
        plan_revision(Path::new("c.kicad_pcb"), false, &a)
    );
}

#[test]
fn a_legacy_net_number_is_named_from_the_net_table() {
    let text = board_text(&[
        r#"(net 3 "SIG")"#,
        r#"(segment (start 0 0) (end 1 0) (width 0.2) (layer "F.Cu") (net 3) (uuid "s-old"))"#,
    ]);
    let reading = BoardReading::of(&text).unwrap();
    assert_eq!(reading.routing[0].net, "SIG");
}

#[test]
fn an_item_without_a_kiid_blocks_the_plan() {
    let text =
        board_text(&[r#"(segment (start 0 0) (end 1 0) (width 0.2) (layer "F.Cu") (net "GND"))"#]);
    let (plan, _) = Plan::of(Path::new("b.kicad_pcb"), &text, false).unwrap();
    assert_eq!(plan.unidentified, 1);
}

#[test]
fn a_refilled_pour_and_a_linked_teardrop_are_not_reported_as_changed() {
    let teardrop = r#"(zone (net "GND") (layer "F.Cu") (uuid "td-1") (attr (teardrop (type padvia))) (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1))))"#;
    let before_text = board_text(&[SEGMENT_GND, ZONE, teardrop]);
    let refilled = ZONE.replace("(xy 119 59)", "(xy 118 58)");
    // The track is gone, KiCad refilled the pour and dropped the teardrop.
    let after_text = board_text(&[&refilled]);
    let (plan, before) = Plan::of(Path::new("b.kicad_pcb"), &before_text, false).unwrap();
    let after = BoardReading::of(&after_text).unwrap();
    assert_eq!(before.linked.get("teardrop_zones"), Some(&1));
    let readback = compare(&plan, &before, &after).unwrap();
    let field = |key: &str| readback.iter().find(|(k, _)| *k == key).unwrap().1.clone();
    assert_eq!(field("removed")["total"], 1);
    // KiCad's own removal of the teardrop is said plainly, not hidden.
    assert_eq!(
        field("never_touched")["kicad_updated"],
        json!(["KiCad removed 1 teardrop_zones with their tracks"])
    );
    // A pour whose outline moved is a change.
    let moved = ZONE.replace("(xy 90 40)", "(xy 80 40)");
    let after = BoardReading::of(&board_text(&[&moved])).unwrap();
    assert!(compare(&plan, &before, &after)
        .unwrap_err()
        .contains("other objects changed: zone"));
}

#[test]
fn the_net_list_is_capped_and_says_how_many_nets_there_are() {
    let items: Vec<RoutingItem> = (0..NETS_LISTED + 5)
        .map(|i| RoutingItem {
            kind: Kind::Track,
            uuid: Some(format!("s-{i}")),
            net: format!("N{i:03}"),
            layer: "F.Cu".into(),
            locked: false,
            in_locked_group: false,
            export_refusal: None,
        })
        .collect();
    let counts = breakdown(&items);
    assert_eq!(counts["nets_total"], NETS_LISTED + 5);
    assert_eq!(counts["nets_listed"], NETS_LISTED);
    assert_eq!(counts["by_net"].as_array().unwrap().len(), NETS_LISTED);
    assert_eq!(counts["total"], NETS_LISTED + 5);
}

#[tokio::test]
async fn routing_without_a_kiid_blocks_the_preview_and_the_apply() {
    let h = Harness::new(
        &[
            SEGMENT_GND,
            r#"(segment (start 0 0) (end 1 0) (width 0.2) (layer "F.Cu") (net "GND"))"#,
        ],
        Fault::default(),
    );
    for args in [
        json!({}),
        json!({ "dry_run": false, "expected_plan_revision": "x" }),
    ] {
        let (error, kind, body) = h.call(args).await;
        assert!(error, "{body}");
        assert_eq!(kind.as_deref(), Some("plan_blocked"), "{body}");
    }
    h.untouched();
}

/// Guard: routing held by a locked group is kept, and the preview says the export will
/// still refuse it.
#[tokio::test]
async fn routing_in_a_locked_group_is_kept_and_flagged() {
    let group = r#"(group "" (uuid "g-1") (locked yes) (members "s-1"))"#;
    let h = Harness::new(&[SEGMENT_GND, VIA_GND, group], Fault::default());
    let (error, _, body) = h.call(json!({})).await;
    assert!(!error, "{body}");
    assert_eq!(body["will_delete"]["total"], 1, "{body}");
    assert_eq!(body["kept_locked"]["in_locked_groups"], 1);
    assert_eq!(body["kept_locked"]["export_will_refuse"][0]["count"], 1);
    let revision = body["plan_revision"].as_str().unwrap().to_string();
    let (error, _, body) = h
        .call(json!({ "dry_run": false, "expected_plan_revision": revision }))
        .await;
    assert!(!error, "{body}");
    assert_eq!(h.deleted.lock().unwrap()[0], ["v-1"]);
    // include_locked deletes it too.
    let revision = h.preview(json!({ "include_locked": true })).await;
    let (error, _, body) = h
        .call(
            json!({ "dry_run": false, "include_locked": true, "expected_plan_revision": revision }),
        )
        .await;
    assert!(!error, "{body}");
    assert_eq!(h.deleted.lock().unwrap()[1], ["s-1"]);
}

#[test]
fn kept_locked_items_the_export_refuses_are_named() {
    let text = board_text(&[
        r#"(arc (start 0 0) (mid 1 1) (end 2 0) (width 0.2) (locked yes) (layer "F.Cu") (net "GND") (uuid "a-l"))"#,
        r#"(via blind (at 1 1) (size 0.6) (drill 0.3) (layers "F.Cu" "In1.Cu") (locked yes) (net "GND") (uuid "v-b"))"#,
        r#"(via (type micro) (at 1 1) (size 0.3) (drill 0.1) (layers "F.Cu" "In1.Cu") (locked yes) (net "GND") (uuid "v-m"))"#,
        r#"(segment (start 3 3) (end 3 3) (width 0.2) (locked yes) (layer "F.Cu") (net "GND") (uuid "s-0"))"#,
        SEGMENT_LOCKED,
        VIA_LOCKED,
    ]);
    let (plan, _) = Plan::of(Path::new("b.kicad_pcb"), &text, false).unwrap();
    assert!(plan.delete.is_empty());
    let reasons: Vec<Option<&str>> = plan.keep_locked.iter().map(|i| i.export_refusal).collect();
    assert_eq!(
        reasons,
        [
            Some("a locked routed arc"),
            Some("a locked via that is not a through via"),
            Some("a locked blind or micro via"),
            Some("a locked zero-length track"),
            None,
            None,
        ]
    );
    let note = kept_note(&plan);
    let total: u64 = note["export_will_refuse"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["count"].as_u64().unwrap())
        .sum();
    assert_eq!(total, 4, "{note}");
}

#[test]
fn a_kicad_9_net_table_renumbered_by_the_delete_is_not_a_change() {
    // Net A loses its only item, so KiCad 9 drops it and renumbers B from 2 to 1.
    let before = r#"(kicad_pcb (version 20240108) (net 0 "") (net 1 "A") (net 2 "B")
        (footprint "R" (uuid "f-1") (pad "1" smd (net 2 "B")))
        (zone (net 2) (net_name "B") (layer "F.Cu") (uuid "z-1") (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1))))
        (segment (start 0 0) (end 1 0) (width 0.2) (layer "F.Cu") (net 1) (uuid "s-a")))"#;
    let after = r#"(kicad_pcb (version 20240108) (net 0 "") (net 1 "B")
        (footprint "R" (uuid "f-1") (pad "1" smd (net 1 "B")))
        (zone (net 1) (net_name "B") (layer "F.Cu") (uuid "z-1") (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1)))))"#;
    let (plan, before) = Plan::of(Path::new("b.kicad_pcb"), before, false).unwrap();
    assert_eq!(plan.delete[0].net, "A");
    let after = BoardReading::of(after).unwrap();
    assert!(compare(&plan, &before, &after).is_ok());
}

#[test]
fn a_group_is_compared_without_its_routing_members() {
    let mixed =
        r#"(group "g" (uuid "g-1") (members "s-1" "11111111-1111-4111-8111-111111111111"))"#;
    let (plan, before) = Plan::of(
        Path::new("b.kicad_pcb"),
        &board_text(&[SEGMENT_GND, mixed]),
        false,
    )
    .unwrap();
    // KiCad drops the deleted track from the group: no change.
    let kept = r#"(group "g" (uuid "g-1") (members "11111111-1111-4111-8111-111111111111"))"#;
    let after = BoardReading::of(&board_text(&[kept])).unwrap();
    assert!(compare(&plan, &before, &after).is_ok());
    // A group that lost a non-routing member did change.
    let lost = r#"(group "g" (uuid "g-1") (members))"#;
    let after = BoardReading::of(&board_text(&[lost])).unwrap();
    assert!(compare(&plan, &before, &after)
        .unwrap_err()
        .contains("other objects changed: group"));
}

#[test]
fn new_routing_after_the_delete_is_a_difference() {
    let (plan, before) =
        Plan::of(Path::new("b.kicad_pcb"), &board_text(&[SEGMENT_GND]), false).unwrap();
    let after = BoardReading::of(&board_text(&[VIA_GND])).unwrap();
    assert!(compare(&plan, &before, &after)
        .unwrap_err()
        .contains("routing items that were not on the board before: 1 (v-1)"));
}

#[test]
fn a_read_that_times_out_before_the_commit_is_not_an_unknown_outcome() {
    let timeout = anyhow::Error::new(konnect_ipc::client::IpcOutcomeUnknown {
        command: "SaveDocumentToString".into(),
        reason: "timed out".into(),
    });
    let mapped = before_commit(timeout);
    assert!(mapped
        .downcast_ref::<konnect_ipc::client::IpcOutcomeUnknown>()
        .is_none());
    assert!(format!("{mapped:#}").contains("nothing was deleted"));
    // Any other error keeps its type.
    let other = before_commit(anyhow::Error::new(std::io::Error::other("x")));
    assert!(other.downcast_ref::<std::io::Error>().is_some());
}
