//! Served MCP tests; the IPC and CLI doubles are explicit, not GUI evidence.
use super::*;
use crate::mcp::handler::McpHandler;
use kiapi::board::types as pcb;
use kiapi::common::{commands as cmd, types as common};
use konnect_ipc::builders::{distance, pack_any};
use konnect_ipc::gen::kiapi;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
const SOURCE: &str = include_str!("../../tests/fixtures/specctra_two_resistors_locked.kicad_pcb");
const OLD_TRACK: &str = "cb3de6bf-0cdb-4170-b26a-66acb0373995";
const OLD_VIA: &str = "db24f42e-63bc-4e53-a1a7-509928ef3cd6";
const RULES: &str = "(version 1)(rule \"Minimum\" (constraint clearance (min 0.2mm)))";
const SES: &str = r#"(session board (base_design board)
 (placement (resolution um 10)
  (component konnect_image_R1 (place R1 1000000 -500000 front 0))
  (component konnect_image_R2 (place R2 1100000 -500000 front 0)))
 (was_is) (routes (resolution um 10)
 (library_out (padstack konnect_via_0001 (shape (circle F.Cu 6000 0 0)) (shape (circle B.Cu 6000 0 0)) (attach off)))
 (network_out (net GND (wire (path F.Cu 2500 995000 -500000 1050000 -500000 1050000 -510000)) (via konnect_via_0001 1050000 -510000 (net GND) (type protect))))))"#;

#[derive(Clone, Copy, Default)]
struct Fault {
    bad_drc: bool,
    missing_category: bool,
    ignored_check: bool,
    ignore_delete: bool,
    wrong_receipt: bool,
    swap_readback_id: bool,
    probe_fails: bool,
    change_rules_in_drc: bool,
    change_rules_in_probe: bool,
    change_class_in_drc: bool,
    change_board_in_drc: bool,
    footprint_receipt: bool,
    wrong_initial_id: bool,
    stale_source: bool,
}
struct State {
    items: Vec<prost_types::Any>,
    class_width: f64,
    change_class_at: Option<usize>,
    class_reads: usize,
}
fn with_id(item: prost_types::Any, id: &str) -> prost_types::Any {
    if item.type_url.ends_with("Track") {
        let mut v = pcb::Track::decode(item.value.as_slice()).unwrap();
        v.id = Some(common::Kiid { value: id.into() });
        pack_any(&v, "kiapi.board.types.Track")
    } else {
        let mut v = pcb::Via::decode(item.value.as_slice()).unwrap();
        v.id = Some(common::Kiid { value: id.into() });
        pack_any(&v, "kiapi.board.types.Via")
    }
}
fn id(item: &prost_types::Any) -> String {
    if item.type_url.ends_with("Track") {
        pcb::Track::decode(item.value.as_slice())
            .unwrap()
            .id
            .unwrap()
            .value
    } else {
        pcb::Via::decode(item.value.as_slice())
            .unwrap()
            .id
            .unwrap()
            .value
    }
}
fn originals() -> Vec<prost_types::Any> {
    let mut t =
        konnect_ipc::builders::build_track("GND", 2, "F.Cu", 0.25, 100.5, 50.0, 105.0, 50.0);
    t.locked = common::LockedState::LsLocked as i32;
    let mut v = konnect_ipc::builders::build_via("GND", 2, 105.0, 50.0, 0.3, 0.6);
    v.locked = common::LockedState::LsLocked as i32;
    vec![
        with_id(pack_any(&t, "kiapi.board.types.Track"), OLD_TRACK),
        with_id(pack_any(&v, "kiapi.board.types.Via"), OLD_VIA),
    ]
}
fn serialized(state: &State) -> String {
    if state.items == originals() {
        return SOURCE.into();
    }
    let mut base = SOURCE.to_owned();
    let mut ranges = konnect_sexp::writer::find_direct_child_blocks(SOURCE, "kicad_pcb")
        .into_iter()
        .filter(|(a, b)| {
            matches!(
                konnect_sexp::parse_sexp(&SOURCE[*a..*b]).unwrap().head(),
                Some("segment" | "via")
            )
        })
        .collect::<Vec<_>>();
    ranges.sort_by_key(|r| r.0);
    for (a, b) in ranges.into_iter().rev() {
        base.replace_range(a..b, "");
    }
    let mut copper = String::new();
    for item in &state.items {
        if item.type_url.ends_with("Track") {
            let t = pcb::Track::decode(item.value.as_slice()).unwrap();
            let start = t.start.unwrap();
            let end = t.end.unwrap();
            let locked = if t.locked == common::LockedState::LsLocked as i32 {
                "(locked yes)"
            } else {
                ""
            };
            copper.push_str(&format!("(segment (start {} {}) (end {} {}) (width {}) (layer \"F.Cu\") (net {}) (uuid {}) {locked})\n",start.x_nm as f64/1e6,start.y_nm as f64/1e6,end.x_nm as f64/1e6,end.y_nm as f64/1e6,t.width.unwrap().value_nm as f64/1e6,crate::custom_rules::quote(&t.net.unwrap().name),crate::custom_rules::quote(&t.id.unwrap().value)));
        } else {
            let v = pcb::Via::decode(item.value.as_slice()).unwrap();
            let pos = v.position.unwrap();
            let stack = v.pad_stack.unwrap();
            let diameter = stack
                .copper_layers
                .iter()
                .filter_map(|l| l.size.as_ref())
                .map(|s| s.x_nm.max(s.y_nm))
                .max()
                .unwrap();
            let drill = stack.drill.unwrap().diameter.unwrap().x_nm;
            let locked = if v.locked == common::LockedState::LsLocked as i32 {
                "(locked yes)"
            } else {
                ""
            };
            copper.push_str(&format!("(via (at {} {}) (size {}) (drill {}) (layers \"F.Cu\" \"B.Cu\") (net {}) (uuid {}) {locked})\n",pos.x_nm as f64/1e6,pos.y_nm as f64/1e6,diameter as f64/1e6,drill as f64/1e6,crate::custom_rules::quote(&v.net.unwrap().name),crate::custom_rules::quote(&v.id.unwrap().value)));
        }
    }
    let end = base.rfind(')').unwrap();
    base.insert_str(end, &copper);
    base
}

struct Scene {
    dir: tempfile::TempDir,
    board: std::path::PathBuf,
    output: std::path::PathBuf,
    ses: std::path::PathBuf,
    manifest: std::path::PathBuf,
    handler: McpHandler,
    cli: std::path::PathBuf,
    _server: crate::test_support::MockIpcServer,
    state: Arc<Mutex<State>>,
    writes: Arc<AtomicUsize>,
}
impl Scene {
    async fn new(fault: Fault) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let board = dir.path().join("board.kicad_pcb");
        std::fs::write(&board, SOURCE).unwrap();
        let board = board.canonicalize().unwrap();
        std::fs::write(board.with_extension("kicad_pro"), "{}\n").unwrap();
        std::fs::write(board.with_extension("kicad_dru"), RULES).unwrap();
        let class_marker = dir.path().join("class-changed");
        let board_marker = dir.path().join("board-changed");
        let mut clean = json!({"violations":[],"unconnected_items":[],"ignored_checks":[],"included_severities":["error","warning","exclusion"]});
        if fault.bad_drc {
            clean["violations"] = json!([{"type":"clearance","severity":"error","description":"custom rule violation"}]);
        }
        if fault.missing_category {
            clean.as_object_mut().unwrap().remove("unconnected_items");
        }
        if fault.ignored_check {
            clean["ignored_checks"] = json!([{"key":"clearance"}]);
        }
        let mut probe = clean.clone();
        probe["unconnected_items"] = json!([]);
        probe["violations"] = if fault.probe_fails {
            json!([])
        } else {
            json!([{"type":"clearance","severity":"error","description":"rule '__konnect_rule_validation_probe__'"}])
        };
        let project_marker = dir.path().join("cli-board-path");
        let mutate_rules = |active| {
            if active {
                format!("printf '%s' '(version 1)(rule \"Changed\" (constraint clearance (min 0.4mm)))' > '{}'\n",board.with_extension("kicad_dru").display())
            } else {
                String::new()
            }
        };
        let mut late_effect = String::new();
        if fault.change_class_in_drc {
            late_effect.push_str(&format!("touch '{}'\n", class_marker.display()));
        }
        if fault.change_board_in_drc {
            late_effect.push_str(&format!("touch '{}'\n", board_marker.display()));
        }
        let shell_quote = |s: &str| format!("'{}'", s.replace('\'', "'\"'\"'"));
        let body=format!("#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n case \"$1\" in --output) shift; out=\"$1\";; *) input=\"$1\";; esac\n shift\ndone\nprintf '%s' \"$input\" > '{}'\ncase \"$out\" in */report.json) {}printf '%s' {} > \"$out\";; *) {}{}printf '%s' {} > \"$out\";; esac\n",project_marker.display(),mutate_rules(fault.change_rules_in_probe),shell_quote(&probe.to_string()),mutate_rules(fault.change_rules_in_drc),late_effect,shell_quote(&clean.to_string()));
        let cli = crate::tools::cli::test_support::write_script(
            dir.path(),
            "routing-cli",
            &body,
            "@echo off\r\nexit /b 1\r\n",
        );
        let state = Arc::new(Mutex::new(State {
            items: originals(),
            class_width: 0.25,
            change_class_at: None,
            class_reads: 0,
        }));
        let held = state.clone();
        let writes = Arc::new(AtomicUsize::new(0));
        let write_count = writes.clone();
        let serial = AtomicUsize::new(0);
        let server = crate::tools::pcb_board::board_mock::spawn_kicad_holding_board(
            &board,
            move |command| {
                let mut st = held.lock().unwrap();
                if board_marker.exists()
                    && !st
                        .items
                        .iter()
                        .any(|i| id(i) == "00000000-0000-4000-8000-000000000099")
                {
                    st.items.push(with_id(
                        originals()[0].clone(),
                        "00000000-0000-4000-8000-000000000099",
                    ));
                }

                if command.type_url.ends_with("SaveDocumentToString") {
                    let r = cmd::SaveDocumentToString::decode(command.value.as_slice()).unwrap();
                    return Some(pack_any(
                        &cmd::SavedDocumentResponse {
                            document: r.document,
                            contents: if fault.stale_source {
                                SOURCE.into()
                            } else {
                                serialized(&st)
                            },
                        },
                        "kiapi.common.commands.SavedDocumentResponse",
                    ));
                }
                if command.type_url.ends_with("GetNets") {
                    return Some(pack_any(
                        &kiapi::board::commands::NetsResponse {
                            nets: vec![
                                konnect_ipc::builders::net("VCC", 1),
                                konnect_ipc::builders::net("GND", 2),
                            ],
                        },
                        "kiapi.board.commands.NetsResponse",
                    ));
                }
                if command.type_url.ends_with("GetNetClassForNets") {
                    st.class_reads += 1;
                    if st.change_class_at == Some(st.class_reads) || class_marker.exists() {
                        st.class_width = 0.3;
                    }
                    let via = konnect_ipc::builders::build_via("GND", 2, 0.0, 0.0, 0.3, 0.6);
                    let classes = ["GND", "VCC"]
                        .into_iter()
                        .map(|net| {
                            (
                                net.into(),
                                kiapi::common::project::NetClass {
                                    name: "Default".into(),
                                    constituents: vec!["Default".into()],
                                    board: Some(kiapi::common::project::NetClassBoardSettings {
                                        track_width: Some(distance(st.class_width)),
                                        clearance: Some(distance(0.2)),
                                        via_stack: via.pad_stack.clone(),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                },
                            )
                        })
                        .collect();
                    return Some(pack_any(
                        &kiapi::board::commands::NetClassForNetsResponse { classes },
                        "kiapi.board.commands.NetClassForNetsResponse",
                    ));
                }
                if command.type_url.ends_with("GetItems") {
                    let r = cmd::GetItems::decode(command.value.as_slice()).unwrap();
                    let mut items = st
                        .items
                        .iter()
                        .filter(|item| {
                            r.types.contains(
                                &(if item.type_url.ends_with("Track") {
                                    common::KiCadObjectType::KotPcbTrace
                                } else {
                                    common::KiCadObjectType::KotPcbVia
                                } as i32),
                            )
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    if fault.wrong_initial_id
                        && !items.is_empty()
                        && r.types
                            .contains(&(common::KiCadObjectType::KotPcbTrace as i32))
                    {
                        items[0] = with_id(items[0].clone(), "wrong-original-id");
                    }
                    if fault.swap_readback_id
                        && st.items.len() > 2
                        && !items.is_empty()
                        && r.types
                            .contains(&(common::KiCadObjectType::KotPcbTrace as i32))
                    {
                        items[0] =
                            with_id(items[0].clone(), "00000000-0000-4000-8000-000000000099");
                    }
                    return Some(pack_any(
                        &cmd::GetItemsResponse {
                            header: None,
                            status: common::ItemRequestStatus::IrsOk as i32,
                            items,
                        },
                        "kiapi.common.commands.GetItemsResponse",
                    ));
                }
                if command.type_url.ends_with("BeginCommit") {
                    return Some(pack_any(
                        &cmd::BeginCommitResponse {
                            id: Some(common::Kiid {
                                value: "commit".into(),
                            }),
                        },
                        "kiapi.common.commands.BeginCommitResponse",
                    ));
                }
                if command.type_url.ends_with("EndCommit") {
                    let r = cmd::EndCommit::decode(command.value.as_slice()).unwrap();
                    if r.action == cmd::CommitAction::CmaDrop as i32 {
                        st.items = originals();
                    }
                    return Some(pack_any(
                        &cmd::EndCommitResponse {},
                        "kiapi.common.commands.EndCommitResponse",
                    ));
                }
                if command.type_url.ends_with("CreateItems") {
                    let r = cmd::CreateItems::decode(command.value.as_slice()).unwrap();
                    write_count.fetch_add(1, Ordering::SeqCst);
                    let mut receipts = Vec::new();
                    for item in r.items {
                        let number = serial.fetch_add(1, Ordering::SeqCst) + 1;
                        let item = with_id(item, &format!("00000000-0000-4000-8000-{number:012}"));
                        st.items.push(item.clone());
                        receipts.push(item);
                    }
                    if fault.footprint_receipt {
                        let root = konnect_sexp::parse_sexp(SOURCE).unwrap();
                        let old = root.find("footprint").unwrap().find_str("uuid").unwrap();
                        receipts[0] = with_id(receipts[0].clone(), old);
                    }
                    if fault.wrong_receipt {
                        receipts[0] = with_id(receipts[0].clone(), OLD_TRACK);
                    }
                    return Some(pack_any(
                        &cmd::CreateItemsResponse {
                            header: None,
                            status: common::ItemRequestStatus::IrsOk as i32,
                            created_items: receipts
                                .into_iter()
                                .map(|item| cmd::ItemCreationResult {
                                    status: Some(cmd::ItemStatus {
                                        code: cmd::ItemStatusCode::IscOk as i32,
                                        error_message: String::new(),
                                    }),
                                    item: Some(item),
                                })
                                .collect(),
                        },
                        "kiapi.common.commands.CreateItemsResponse",
                    ));
                }
                if command.type_url.ends_with("DeleteItems") {
                    let r = cmd::DeleteItems::decode(command.value.as_slice()).unwrap();
                    write_count.fetch_add(1, Ordering::SeqCst);
                    if !fault.ignore_delete {
                        st.items
                            .retain(|item| !r.item_ids.iter().any(|i| i.value == id(item)));
                    }
                    return Some(pack_any(
                        &cmd::DeleteItemsResponse {
                            header: None,
                            status: common::ItemRequestStatus::IrsOk as i32,
                            deleted_items: vec![],
                        },
                        "kiapi.common.commands.DeleteItemsResponse",
                    ));
                }
                None
            },
        );
        let handler = McpHandler::new(crate::tools::ServerConfig {
            ipc_address: server.address().into(),
            kicad_cli: cli.to_string_lossy().into_owned(),
            eager_toolsets: true,
            ..Default::default()
        })
        .await
        .unwrap();
        let output = dir.path().join("export.dsn");
        let manifest = dir.path().join("export.dsn.konnect.json");
        let ses = dir.path().join("board.ses");
        std::fs::write(&ses, SES).unwrap();
        let scene = Self {
            dir,
            board,
            output,
            ses,
            manifest,
            handler,
            cli,
            _server: server,
            state,
            writes,
        };
        let result = scene
            .call(
                "export_specctra_dsn",
                json!({"board":scene.board,"output":scene.output,"native_bridge_mode":"disable"}),
            )
            .await;
        assert_ne!(result["isError"], true, "{result:#}");
        scene
    }
    async fn call(&self, name: &str, args: serde_json::Value) -> serde_json::Value {
        self.handler.handle_message(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})).await.unwrap().result.unwrap()
    }
    fn args(&self) -> serde_json::Value {
        json!({"board":self.board,"ses_path":self.ses,"manifest_path":self.manifest,"candidate_output_path":self.dir.path().join("candidate.kicad_pcb")})
    }
    async fn apply(&self) -> serde_json::Value {
        self.call("apply_specctra_ses", self.args()).await
    }
    fn unchanged(&self) {
        assert_eq!(self.state.lock().unwrap().items.len(), 2);
        assert_eq!(std::fs::read_to_string(&self.board).unwrap(), SOURCE);
    }
}

#[tokio::test]
async fn served_custom_rule_export_plan_apply_uses_private_candidate_context() {
    let scene = Scene::new(Fault::default()).await;
    let mut plan = scene.args();
    plan.as_object_mut()
        .unwrap()
        .remove("candidate_output_path");
    let result = scene.call("plan_specctra_ses_import", plan).await;
    assert_ne!(result["isError"], true, "{result:#}");
    assert_eq!(scene.writes.load(Ordering::SeqCst), 0);
    let candidate = scene.dir.path().join("candidate.kicad_pcb");
    let foreign = candidate.with_extension("kicad_dru");
    std::fs::write(&foreign, "foreign invalid rules").unwrap();
    std::fs::write(
        candidate.with_extension("kicad_pro"),
        "foreign invalid project",
    )
    .unwrap();
    let result = scene.apply().await;
    assert_ne!(result["isError"], true, "{result:#}");
    let body: serde_json::Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["drc"]["clean"], true);
    assert_eq!(body["drc"]["custom_rules_checked"], true);
    assert_eq!(
        body["drc"]["schematic_parity_violations"],
        serde_json::Value::Null
    );
    assert_eq!(
        body["drc"]["included_severities"],
        json!(["error", "warning", "exclusion"])
    );
    assert_eq!(body["drc"]["ignored_check_keys"], json!([]));
    assert_eq!(body["ipc_readback"], "exact_identity_and_count_match");
    assert_eq!(scene.state.lock().unwrap().items.len(), 5);
    assert!(candidate.exists());
    let used = std::fs::read_to_string(scene.dir.path().join("cli-board-path")).unwrap();
    assert_ne!(Path::new(&used), candidate);
    assert_eq!(
        std::fs::read_to_string(foreign).unwrap(),
        "foreign invalid rules"
    );
    assert_eq!(std::fs::read_to_string(&scene.board).unwrap(), SOURCE);
}
#[tokio::test]
async fn served_nonclean_or_incomplete_or_suppressed_custom_drc_proves_rollback() {
    for fault in [
        Fault {
            stale_source: true,
            ..Default::default()
        },
        Fault {
            bad_drc: true,
            ..Default::default()
        },
        Fault {
            missing_category: true,
            ..Default::default()
        },
        Fault {
            ignored_check: true,
            ..Default::default()
        },
        Fault {
            swap_readback_id: true,
            ..Default::default()
        },
    ] {
        let scene = Scene::new(fault).await;
        let result = scene.apply().await;
        assert_eq!(result["isError"], true, "{result:#}");
        assert!(scene.writes.load(Ordering::SeqCst) >= 2);
        scene.unchanged();
        assert!(!scene.dir.path().join("candidate.kicad_pcb").exists());
        assert!(
            !result.to_string().contains("mutation_outcome_uncertain"),
            "{result:#}"
        );
    }
}
#[tokio::test]
async fn served_failed_restoration_or_colliding_receipt_reports_uncertainty() {
    for fault in [
        Fault {
            footprint_receipt: true,
            ..Default::default()
        },
        Fault {
            change_board_in_drc: true,
            ..Default::default()
        },
        Fault {
            bad_drc: true,
            ignore_delete: true,
            ..Default::default()
        },
        Fault {
            wrong_receipt: true,
            ..Default::default()
        },
    ] {
        let scene = Scene::new(fault).await;
        let result = scene.apply().await;
        assert_eq!(result["isError"], true);
        assert!(
            result.to_string().contains("mutation_outcome_uncertain"),
            "{result:#}"
        );
        if fault.change_board_in_drc {
            assert_eq!(
                scene.state.lock().unwrap().items.len(),
                3,
                "external copper must survive compensation"
            );
        }
        if fault.wrong_receipt || fault.footprint_receipt {
            scene.unchanged();
            assert_eq!(
                scene.writes.load(Ordering::SeqCst),
                1,
                "a colliding receipt must never delete original copper"
            );
        }
    }
}
#[tokio::test]
async fn served_rule_or_class_changes_refuse_before_or_recover_after_commit() {
    for fault in [
        Fault {
            probe_fails: true,
            ..Default::default()
        },
        Fault {
            change_rules_in_probe: true,
            ..Default::default()
        },
    ] {
        let scene = Scene::new(fault).await;
        let result = scene.apply().await;
        assert_eq!(result["isError"], true);
        assert_eq!(scene.writes.load(Ordering::SeqCst), 0);
        scene.unchanged();
    }
    for fault in [
        Fault {
            change_rules_in_drc: true,
            ..Default::default()
        },
        Fault {
            change_class_in_drc: true,
            ..Default::default()
        },
    ] {
        let scene = Scene::new(fault).await;
        let result = scene.apply().await;
        assert_eq!(result["isError"], true);
        assert!(scene.writes.load(Ordering::SeqCst) >= 2);
        scene.unchanged();
    }
    let scene = Scene::new(Fault::default()).await;
    {
        let mut state = scene.state.lock().unwrap();
        state.change_class_at = Some(state.class_reads + 2);
    }
    let result = scene.apply().await;
    assert_eq!(result["isError"], true);
    assert_eq!(scene.writes.load(Ordering::SeqCst), 0);
    scene.unchanged();
}
#[tokio::test]
async fn served_export_refuses_native_requirement_and_unrepresentable_rules() {
    let scene = Scene::new(Fault::default()).await;
    let output = scene.dir.path().join("other.dsn");
    let result = scene
        .call(
            "export_specctra_dsn",
            json!({"board":scene.board,"output":output,"native_bridge_mode":"require"}),
        )
        .await;
    assert_eq!(result["isError"], true);
    assert!(!output.exists());
    std::fs::write(scene.board.with_extension("kicad_dru"),"(version 1)(rule \"Condition\" (condition \"A.NetName == 'GND'\") (constraint clearance (min 0.2mm)))").unwrap();
    let reads = scene.state.lock().unwrap().class_reads;
    let result = scene
        .call(
            "export_specctra_dsn",
            json!({"board":scene.board,"output":output,"native_bridge_mode":"disable"}),
        )
        .await;
    assert_eq!(result["isError"], true);
    assert_eq!(scene.state.lock().unwrap().class_reads, reads);
    assert!(!output.exists());
}

#[tokio::test]
async fn served_export_rejects_live_classes_that_change_between_snapshots() {
    let scene = Scene::new(Fault::default()).await;
    {
        let mut state = scene.state.lock().unwrap();
        state.change_class_at = Some(state.class_reads + 2);
    }
    let output = scene.dir.path().join("changed.dsn");
    let result = scene
        .call(
            "export_specctra_dsn",
            json!({"board":scene.board,"output":output,"native_bridge_mode":"disable"}),
        )
        .await;
    assert_eq!(result["isError"], true, "{result:#}");
    assert!(!output.exists());
    assert_eq!(scene.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn served_initial_ipc_inventory_must_match_serialized_source_before_mutation() {
    let scene = Scene::new(Fault {
        wrong_initial_id: true,
        ..Default::default()
    })
    .await;
    let result = scene.apply().await;
    assert_eq!(result["isError"], true, "{result:#}");
    assert_eq!(scene.writes.load(Ordering::SeqCst), 0);
    scene.unchanged();
}
#[test]
fn snapshot_identity_inputs_and_empty_receipts_are_not_guessed() {
    for source in [
        "(other)",
        "(kicad_pcb (footprint (uuid \"\")))",
        "(kicad_pcb (segment))",
    ] {
        assert!(snapshot_routing_ids(source).is_err(), "{source}");
    }
    let item = with_id(originals()[0].clone(), "");
    assert!(created_route_item_ids(&[item]).is_err());
    let (all, copper) = snapshot_routing_ids(SOURCE).unwrap();
    assert!(all.len() > copper.len());
    assert!(all.contains(OLD_TRACK));
    assert_eq!(copper.len(), 2);
}

#[tokio::test]
#[ignore = "build the release fork at HEAD first; private IPC/CLI doubles"]
async fn custom_rule_release_binary_stdio_dogfood() {
    use std::io::{BufRead, Write};
    use std::path::PathBuf;
    let h = Scene::new(Fault::default()).await;
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let exe = repo.join("target/release/konnect");
    let home = repo.join("target/dogfood-home");
    std::fs::create_dir_all(&home).unwrap();
    let state = tempfile::tempdir_in(&home).unwrap();
    // The normal mock uses inproc://, which cannot cross process boundaries.
    // Own a private IPC endpoint and forward raw messages inside this process.
    use nng::options::Options;
    let address = format!("ipc://{}", state.path().join("fork.sock").display());
    let socket = nng::Socket::new(nng::Protocol::Rep0).unwrap();
    socket.listen(&address).unwrap();
    let control = socket.clone();
    let upstream = h._server.address().to_string();
    let worker = std::thread::spawn(move || {
        let forward = nng::Socket::new(nng::Protocol::Req0).unwrap();
        forward
            .set_opt::<nng::options::RecvTimeout>(Some(std::time::Duration::from_secs(60)))
            .unwrap();
        forward.dial(&upstream).unwrap();
        while let Ok(message) = socket.recv() {
            if forward.send(message).is_err() {
                break;
            }
            let Ok(response) = forward.recv() else { break };
            if socket.send(response).is_err() {
                break;
            }
        }
    });
    struct Bridge {
        control: nng::Socket,
        worker: Option<std::thread::JoinHandle<()>>,
    }
    impl Drop for Bridge {
        fn drop(&mut self) {
            self.control.close();
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
    let _bridge = Bridge {
        control,
        worker: Some(worker),
    };
    let config = state.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            "ipc_address = {}\neager_toolsets = true\nkicad_cli = {}\nlog_level = \"error\"\n",
            serde_json::to_string(&address).unwrap(),
            serde_json::to_string(h.cli.to_str().unwrap()).unwrap()
        ),
    )
    .unwrap();
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(
        std::process::Command::new(&exe)
            .arg("--config")
            .arg(&config)
            .env("HOME", &home)
            .env("KONNECT_STATE_DIR", &home)
            .env_remove("KICAD_API_SOCKET")
            .env_remove("KICAD_API_TOKEN")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            if send.send(line).is_err() {
                break;
            }
        }
    });
    let mut sequence = 0;
    let mut request = |method: &str, params: serde_json::Value| -> serde_json::Value {
        sequence += 1;
        writeln!(
            child.0.stdin.as_mut().unwrap(),
            "{}",
            serde_json::json!({"jsonrpc":"2.0","id":sequence,"method":method,"params":params})
        )
        .unwrap();
        child.0.stdin.as_mut().unwrap().flush().unwrap();
        loop {
            let line = receive
                .recv_timeout(std::time::Duration::from_secs(60))
                .unwrap()
                .unwrap();
            let value: serde_json::Value = serde_json::from_str(&line).unwrap();
            if value["id"] == sequence {
                assert!(value.get("error").is_none(), "{value:#}");
                return value["result"].clone();
            }
        }
    };
    request(
        "initialize",
        serde_json::json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"pr13-routing-dogfood","version":"1"}}),
    );
    let mut call = |name: &str, args: serde_json::Value| -> serde_json::Value {
        let result = request(
            "tools/call",
            serde_json::json!({"name":name,"arguments":args}),
        );
        assert_ne!(result["isError"], true, "{result:#}");
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let info = call("get_installation_info", serde_json::json!({}));
    assert_eq!(
        PathBuf::from(info["runtime"]["executable_path"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        exe.canonicalize().unwrap()
    );
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert_eq!(
        info["build"]["commit"],
        String::from_utf8(head.stdout).unwrap().trim()
    );
    call("load_toolset", serde_json::json!({"name":"simulation"}));
    call("load_toolset", serde_json::json!({"name":"pcb_routing"}));

    call("load_toolset", json!({"name":"pcb_export"}));
    let output = h.dir.path().join("runtime.dsn");
    let exported = call(
        "export_specctra_dsn",
        json!({"board":h.board,"output":output,"native_bridge_mode":"prefer"}),
    );
    assert_eq!(
        exported["custom_rule_projection"],
        "conservative_global_minima"
    );
    assert!(exported["routing_context"]["custom_rules_sha256"].is_string());
    let manifest = exported["manifest_output_path"].clone();
    let plan = call(
        "plan_specctra_ses_import",
        json!({"board":h.board,"ses_path":h.ses,"manifest_path":manifest}),
    );
    assert_eq!(plan["mutated"], false);
    assert_eq!(plan["track_count"], 2);
    assert_eq!(h.writes.load(Ordering::SeqCst), 0);
    let candidate = h.dir.path().join("runtime-candidate.kicad_pcb");
    let result = call(
        "apply_specctra_ses",
        json!({"board":h.board,"ses_path":h.ses,"manifest_path":manifest,"candidate_output_path":candidate}),
    );
    assert_eq!(result["created_count"], 3);
    assert_eq!(result["drc"]["custom_rules_checked"], true);
    assert_eq!(result["drc"]["clean"], true);
    assert_eq!(result["ipc_readback"], "exact_identity_and_count_match");
    assert_eq!(h.state.lock().unwrap().items.len(), 5);
    assert_eq!(std::fs::read_to_string(&h.board).unwrap(), SOURCE);
    assert!(candidate.exists());
}
