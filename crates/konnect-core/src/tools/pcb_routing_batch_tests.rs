use super::*;
use crate::tools::pcb_board::board_mock::spawn_kicad_holding_board;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
const FIXTURE: &str = include_str!("../../tests/fixtures/specctra_two_resistors.kicad_pcb");
#[derive(Default, Clone, Copy)]
struct Fault {
    ignore_create: bool,
    reject_create: bool,
    missing_receipt: bool,
    duplicate_receipt: bool,
    old_receipt: bool,
    wrong_net: bool,
    wrong_shape: bool,
    ignore_update: bool,
    ignore_delete: bool,
    corrupt_existing: bool,
    extra_item: bool,
    fail_readback: bool,
    no_rules: bool,
    locked: bool,
    wrong_pad_net: bool,
    missing_pad_layer: bool,
    repeated_pad: bool,
    missing_footprint: bool,
    inventory_swap: bool,
    pad_outside: bool,
    wrong_via_net: bool,
    wrong_via_geometry: bool,
}
struct Harness {
    _temp: tempfile::TempDir,
    server: crate::test_support::MockIpcServer,
    board: std::path::PathBuf,
    held: Arc<Mutex<Vec<prost_types::Any>>>,
    commits: Arc<AtomicUsize>,
    writes: Arc<AtomicUsize>,
}
fn raw_track(id: &str) -> prost_types::Any {
    let mut t = konnect_ipc::builders::build_track("VCC", 1, "F.Cu", 0.25, 10.0, 20.0, 15.0, 20.0);
    t.id = Some(common::Kiid { value: id.into() });
    pack_any(&t, "kiapi.board.types.Track")
}
fn raw_via(id: &str) -> prost_types::Any {
    let mut v = konnect_ipc::builders::build_via("VCC", 1, 15.0, 20.0, 0.3, 0.6);
    v.id = Some(common::Kiid { value: id.into() });
    pack_any(&v, "kiapi.board.types.Via")
}
fn footprint(reference: &str, x: f64, y: f64, fault: Fault) -> prost_types::Any {
    // The old regression's saved R1 is at y=50; the live pad is at y=60.
    let x = if fault.pad_outside { 3000.0 } else { x };
    let mut pads = Vec::new();
    for (number, px) in [("1", x - 0.5), ("2", x + 0.5)] {
        let p = pcb::Pad {
            number: number.into(),
            position: Some(konnect_ipc::builders::vec2(px, y)),
            net: Some(konnect_ipc::builders::net(
                if fault.wrong_pad_net { "GND" } else { "VCC" },
                1,
            )),
            pad_stack: Some(pcb::PadStack {
                layers: if fault.missing_pad_layer {
                    vec![]
                } else {
                    vec![pcb::BoardLayer::BlFCu as i32]
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        pads.push(pack_any(&p, "kiapi.board.types.Pad"));
    }
    if fault.repeated_pad {
        pads.push(pads[0].clone());
    }
    pack_any(
        &pcb::FootprintInstance {
            reference_field: Some(pcb::Field {
                name: "Reference".into(),
                text: Some(pcb::BoardText {
                    text: Some(common::Text {
                        text: reference.into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            definition: Some(pcb::Footprint {
                items: pads,
                ..Default::default()
            }),
            ..Default::default()
        },
        "kiapi.board.types.FootprintInstance",
    )
}
impl Harness {
    fn new(fault: Fault) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let board = temp.path().join("routing.kicad_pcb");
        std::fs::write(&board, FIXTURE).unwrap();
        let mut seed = vec![raw_track("old-trace"), raw_via("old-via")];
        if fault.locked {
            let mut t = pcb::Track::decode(seed[0].value.as_slice()).unwrap();
            t.locked = common::LockedState::LsLocked as i32;
            seed[0] = pack_any(&t, "kiapi.board.types.Track");
            let mut v = pcb::Via::decode(seed[1].value.as_slice()).unwrap();
            v.locked = common::LockedState::LsLocked as i32;
            seed[1] = pack_any(&v, "kiapi.board.types.Via");
        }
        let held = Arc::new(Mutex::new(seed));
        let items = held.clone();
        let commits = Arc::new(AtomicUsize::new(0));
        let begin = commits.clone();
        let writes = Arc::new(AtomicUsize::new(0));
        let write_count = writes.clone();
        let serial = AtomicUsize::new(0);
        let footprints = if fault.missing_footprint {
            vec![footprint("R2", 110.0, 50.0, fault)]
        } else {
            vec![
                footprint("R1", 100.0, 60.0, fault),
                footprint("R2", 110.0, 50.0, fault),
            ]
        };
        let expected =
            crate::tools::pcb_board::board_mock::board_document(&board.to_string_lossy());
        let server = spawn_kicad_holding_board(&board, move |command| {
            if command.type_url.ends_with("GetItems") {
                let r =
                    kiapi::common::commands::GetItems::decode(command.value.as_slice()).unwrap();
                assert_eq!(r.header.unwrap().document, Some(expected.clone()));
                let result = if r
                    .types
                    .contains(&(common::KiCadObjectType::KotPcbFootprint as i32))
                {
                    footprints.clone()
                } else if fault.inventory_swap {
                    if r.types
                        .contains(&(common::KiCadObjectType::KotPcbTrace as i32))
                    {
                        vec![raw_via("old-via")]
                    } else {
                        Vec::new()
                    }
                } else if fault.fail_readback && write_count.load(Ordering::SeqCst) > 0 {
                    vec![prost_types::Any {
                        type_url: "bad".into(),
                        value: vec![],
                    }]
                } else {
                    items
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|i| {
                            r.types.contains(
                                &(if any_is(i, "kiapi.board.types.Track") {
                                    common::KiCadObjectType::KotPcbTrace
                                } else {
                                    common::KiCadObjectType::KotPcbVia
                                } as i32),
                            )
                        })
                        .cloned()
                        .collect()
                };
                return Some(pack_any(
                    &kiapi::common::commands::GetItemsResponse {
                        header: None,
                        status: common::ItemRequestStatus::IrsOk as i32,
                        items: result,
                    },
                    "kiapi.common.commands.GetItemsResponse",
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
                let via = konnect_ipc::builders::build_via("VCC", 1, 0.0, 0.0, 0.3, 0.6);
                let classes = if fault.no_rules {
                    Default::default()
                } else {
                    ["VCC", "GND"]
                        .into_iter()
                        .map(|name| {
                            (
                                name.into(),
                                kiapi::common::project::NetClass {
                                    name: "Power".into(),
                                    board: Some(kiapi::common::project::NetClassBoardSettings {
                                        track_width: Some(konnect_ipc::builders::distance(0.25)),
                                        via_stack: via.pad_stack.clone(),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                },
                            )
                        })
                        .collect()
                };
                return Some(pack_any(
                    &kiapi::board::commands::NetClassForNetsResponse { classes },
                    "kiapi.board.commands.NetClassForNetsResponse",
                ));
            }
            if command.type_url.ends_with("GetBoardEnabledLayers") {
                return Some(pack_any(
                    &kiapi::board::commands::BoardEnabledLayersResponse {
                        copper_layer_count: 2,
                        layers: vec![pcb::BoardLayer::BlFCu as i32, pcb::BoardLayer::BlBCu as i32],
                    },
                    "kiapi.board.commands.BoardEnabledLayersResponse",
                ));
            }
            if command.type_url.ends_with("SaveDocumentToString") {
                let r =
                    kiapi::common::commands::SaveDocumentToString::decode(command.value.as_slice())
                        .unwrap();
                assert_eq!(r.document, Some(expected.clone()));
                return Some(pack_any(
                    &kiapi::common::commands::SavedDocumentResponse {
                        document: r.document,
                        contents: FIXTURE.into(),
                    },
                    "kiapi.common.commands.SavedDocumentResponse",
                ));
            }
            if command.type_url.ends_with("BeginCommit") {
                begin.fetch_add(1, Ordering::SeqCst);
                return Some(pack_any(
                    &kiapi::common::commands::BeginCommitResponse {
                        id: Some(common::Kiid {
                            value: "commit".into(),
                        }),
                    },
                    "kiapi.common.commands.BeginCommitResponse",
                ));
            }
            if command.type_url.ends_with("EndCommit") {
                return Some(pack_any(
                    &kiapi::common::commands::EndCommitResponse {},
                    "kiapi.common.commands.EndCommitResponse",
                ));
            }
            if command.type_url.ends_with("CreateItems") {
                let r =
                    kiapi::common::commands::CreateItems::decode(command.value.as_slice()).unwrap();
                assert_eq!(r.header.unwrap().document, Some(expected.clone()));
                write_count.fetch_add(1, Ordering::SeqCst);
                let mut receipts = Vec::new();
                let mut held = items.lock().unwrap();
                for item in r.items {
                    let id = format!("created-{}", serial.fetch_add(1, Ordering::SeqCst));
                    let item = assign_id(item, &id);
                    let mut actual = item.clone();
                    if fault.wrong_net && any_is(&actual, "kiapi.board.types.Track") {
                        let mut t = pcb::Track::decode(actual.value.as_slice()).unwrap();
                        t.net = Some(konnect_ipc::builders::net("GND", 2));
                        actual = pack_any(&t, "kiapi.board.types.Track");
                    }
                    if any_is(&actual, "kiapi.board.types.Via")
                        && (fault.wrong_via_net || fault.wrong_via_geometry)
                    {
                        let mut via = pcb::Via::decode(actual.value.as_slice()).unwrap();
                        if fault.wrong_via_net {
                            via.net = Some(konnect_ipc::builders::net("GND", 2));
                        }
                        if fault.wrong_via_geometry {
                            via.pad_stack
                                .as_mut()
                                .unwrap()
                                .drill
                                .as_mut()
                                .unwrap()
                                .diameter
                                .as_mut()
                                .unwrap()
                                .x_nm += 1;
                        }
                        actual = pack_any(&via, "kiapi.board.types.Via");
                    }
                    if fault.wrong_shape && any_is(&actual, "kiapi.board.types.Track") {
                        let mut t = pcb::Track::decode(actual.value.as_slice()).unwrap();
                        t.width.as_mut().unwrap().value_nm += 1;
                        actual = pack_any(&t, "kiapi.board.types.Track");
                    }
                    if !fault.ignore_create {
                        held.push(actual);
                    }
                    receipts.push(item);
                }
                if fault.extra_item {
                    held.push(raw_via("unexpected-extra"));
                }
                if fault.corrupt_existing {
                    let mut t = pcb::Track::decode(held[0].value.as_slice()).unwrap();
                    t.width.as_mut().unwrap().value_nm += 1;
                    held[0] = pack_any(&t, "kiapi.board.types.Track");
                }
                if fault.old_receipt {
                    receipts[0] = assign_id(receipts[0].clone(), "old-trace");
                }
                if fault.duplicate_receipt && receipts.len() > 1 {
                    receipts[1] = receipts[0].clone();
                }
                if fault.wrong_shape {
                    for receipt in &mut receipts {
                        if any_is(receipt, "kiapi.board.types.Track") {
                            let mut track = pcb::Track::decode(receipt.value.as_slice()).unwrap();
                            track.width.as_mut().unwrap().value_nm += 1;
                            *receipt = pack_any(&track, "kiapi.board.types.Track");
                        }
                    }
                }
                receipts.reverse();
                return Some(pack_any(
                    &kiapi::common::commands::CreateItemsResponse {
                        header: None,
                        status: common::ItemRequestStatus::IrsOk as i32,
                        created_items: receipts
                            .into_iter()
                            .map(|item| kiapi::common::commands::ItemCreationResult {
                                status: Some(kiapi::common::commands::ItemStatus {
                                    code: (if fault.reject_create {
                                        kiapi::common::commands::ItemStatusCode::IscUnknown
                                    } else {
                                        kiapi::common::commands::ItemStatusCode::IscOk
                                    }) as i32,
                                    error_message: String::new(),
                                }),
                                item: if fault.missing_receipt || fault.reject_create {
                                    None
                                } else {
                                    Some(item)
                                },
                            })
                            .collect(),
                    },
                    "kiapi.common.commands.CreateItemsResponse",
                ));
            }
            if command.type_url.ends_with("UpdateItems") {
                let r =
                    kiapi::common::commands::UpdateItems::decode(command.value.as_slice()).unwrap();
                assert_eq!(r.header.unwrap().document, Some(expected.clone()));
                write_count.fetch_add(1, Ordering::SeqCst);
                if !fault.ignore_update {
                    let mut held = items.lock().unwrap();
                    for item in &r.items {
                        let id = object_id(item).unwrap();
                        let slot = held
                            .iter_mut()
                            .find(|i| object_id(i).unwrap() == id)
                            .unwrap();
                        *slot = item.clone();
                    }
                }
                return Some(pack_any(
                    &kiapi::common::commands::UpdateItemsResponse {
                        header: None,
                        status: common::ItemRequestStatus::IrsOk as i32,
                        updated_items: r
                            .items
                            .into_iter()
                            .map(|item| kiapi::common::commands::ItemUpdateResult {
                                status: Some(kiapi::common::commands::ItemStatus {
                                    code: kiapi::common::commands::ItemStatusCode::IscOk as i32,
                                    error_message: String::new(),
                                }),
                                item: Some(item),
                            })
                            .collect(),
                    },
                    "kiapi.common.commands.UpdateItemsResponse",
                ));
            }
            if command.type_url.ends_with("DeleteItems") {
                let r =
                    kiapi::common::commands::DeleteItems::decode(command.value.as_slice()).unwrap();
                assert_eq!(r.header.unwrap().document, Some(expected.clone()));
                write_count.fetch_add(1, Ordering::SeqCst);
                if !fault.ignore_delete {
                    items.lock().unwrap().retain(|item| {
                        !r.item_ids
                            .iter()
                            .any(|id| id.value == object_id(item).unwrap())
                    });
                }
                return Some(pack_any(
                    &kiapi::common::commands::DeleteItemsResponse {
                        header: None,
                        status: common::ItemRequestStatus::IrsOk as i32,
                        deleted_items: Vec::new(),
                    },
                    "kiapi.common.commands.DeleteItemsResponse",
                ));
            }
            None
        });
        Self {
            _temp: temp,
            server,
            board,
            held,
            commits,
            writes,
        }
    }
    async fn call(&self, name: &str, extra: Value) -> (bool, Value) {
        let mut args = json!({"board":self.board});
        for (key, value) in extra.as_object().unwrap() {
            args[key] = value.clone();
        }
        let handler = crate::mcp::handler::McpHandler::new(crate::tools::ServerConfig {
            ipc_address: self.server.address().into(),
            eager_toolsets: true,
            ..Default::default()
        })
        .await
        .unwrap();
        let response=handler.handle_message(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})).await.unwrap();
        let r = response.result.unwrap();
        let body = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
        (r["isError"] == true, body)
    }
    fn unchanged(&self) {
        assert_eq!(self.writes.load(Ordering::SeqCst), 0);
        assert_eq!(self.commits.load(Ordering::SeqCst), 0);
        assert_eq!(std::fs::read_to_string(&self.board).unwrap(), FIXTURE);
    }
}
fn assign_id(item: prost_types::Any, id: &str) -> prost_types::Any {
    if any_is(&item, "kiapi.board.types.Track") {
        let mut t = pcb::Track::decode(item.value.as_slice()).unwrap();
        t.id = Some(common::Kiid { value: id.into() });
        pack_any(&t, "kiapi.board.types.Track")
    } else {
        let mut v = pcb::Via::decode(item.value.as_slice()).unwrap();
        v.id = Some(common::Kiid { value: id.into() });
        pack_any(&v, "kiapi.board.types.Via")
    }
}
fn batch() -> Value {
    json!({"tracks":[{"net_name":"VCC","layer":"F.Cu","points":[{"x":20,"y":20},{"x":30,"y":20},{"x":30,"y":30}]}],"vias":[{"net_name":"VCC","x":30,"y":30}]})
}
fn segment() -> Value {
    json!({"net_name":"VCC","layer":"F.Cu","x1":20,"y1":20,"x2":30,"y2":20})
}
#[tokio::test]
async fn mixed_batch_is_one_commit_and_returns_readback_ids() {
    let h = Harness::new(Fault::default());
    let (err, r) = h.call("route_batch", batch()).await;
    assert!(!err, "{r:#}");
    assert_eq!(r["outcome"]["status"], "complete");
    assert_eq!(r["created_count"], 3);
    assert_eq!(h.commits.load(Ordering::SeqCst), 1);
    assert_eq!(h.held.lock().unwrap().len(), 5);
    assert_eq!(r["created"][0]["uuid"], "created-0");
    assert_eq!(r["created"][0]["width"], 0.25);
    assert_eq!(r["created"][2]["drill"], 0.3);
    assert_eq!(std::fs::read_to_string(&h.board).unwrap(), FIXTURE);
}
#[tokio::test]
async fn live_pad_positions_not_saved_coordinates() {
    let h = Harness::new(Fault::default());
    let (err, r) = h
        .call(
            "route_pad_to_pad",
            json!({"net_name":"VCC","ref1":"R1","pad1":"1","ref2":"R2","pad2":"2"}),
        )
        .await;
    assert!(!err, "{r:#}");
    assert_eq!(r["from"]["x"], 99.5);
    assert_eq!(r["from"]["y"], 60.0);
    assert_eq!(r["to"]["x"], 110.5);
    assert_eq!(r["created_count"], 2);
    assert_eq!(r["created"][0]["from"]["y"], 60.0);
}
#[tokio::test]
async fn missing_live_footprint_and_pad_do_not_use_saved_fallback() {
    for (fault, pad, message) in [
        (
            Fault {
                missing_footprint: true,
                ..Default::default()
            },
            "1",
            "Footprint 'R1'",
        ),
        (Fault::default(), "9", "Pad '9'"),
    ] {
        let h = Harness::new(fault);
        let (err, r) = h
            .call(
                "route_pad_to_pad",
                json!({"net_name":"VCC","ref1":"R1","pad1":pad,"ref2":"R2","pad2":"2"}),
            )
            .await;
        assert!(err);
        assert!(r.to_string().contains(message), "{r:#}");
        h.unchanged();
    }
}
#[tokio::test]
async fn pad_net_layer_and_repeated_anchors_refuse_before_writing() {
    for fault in [
        Fault {
            wrong_pad_net: true,
            ..Default::default()
        },
        Fault {
            missing_pad_layer: true,
            ..Default::default()
        },
        Fault {
            repeated_pad: true,
            ..Default::default()
        },
        Fault {
            pad_outside: true,
            ..Default::default()
        },
    ] {
        let h = Harness::new(fault);
        let (err, r) = h
            .call(
                "route_pad_to_pad",
                json!({"net_name":"VCC","ref1":"R1","pad1":"1","ref2":"R2","pad2":"2"}),
            )
            .await;
        assert!(err, "{r:#}");
        h.unchanged();
    }
}
#[tokio::test]
async fn invalid_batch_entry_refuses_every_item() {
    for bad in [
        json!({"net_name":"VCC","x":30,"y":30,"drill":0.8,"pad_size":0.6}),
        json!({"net_name":"MISSING","x":30,"y":30}),
        json!({"net_name":"VCC","x":3000,"y":30}),
    ] {
        let h = Harness::new(Fault::default());
        let mut args = batch();
        args["vias"] = json!([bad]);
        let (err, r) = h.call("route_batch", args).await;
        assert!(err, "{r:#}");
        h.unchanged();
    }
}
#[tokio::test]
async fn numeric_width_layer_and_zero_length_guards() {
    for (key, value) in [
        ("width", json!(0.1)),
        ("width", json!(null)),
        ("width", json!(0)),
        ("x1", json!(3000)),
        ("layer", json!("F.SilkS")),
        ("layer", json!("In1.Cu")),
        ("x2", json!(20)),
    ] {
        let h = Harness::new(Fault::default());
        let mut args = segment();
        args[key] = value;
        let (err, r) = h.call("route_trace", args).await;
        assert!(err, "{r:#}");
        h.unchanged();
    }
}
#[tokio::test]
async fn missing_effective_rules_never_fall_back_to_saved_settings() {
    let h = Harness::new(Fault {
        no_rules: true,
        ..Default::default()
    });
    let (err, r) = h.call("route_trace", segment()).await;
    assert!(err, "{r:#}");
    h.unchanged();
}
#[tokio::test]
async fn batch_count_bound_covers_polyline_expansion() {
    let h = Harness::new(Fault::default());
    let points = (0..201).map(|x| json!({"x":x,"y":0})).collect::<Vec<_>>();
    let args = json!({"tracks":[{"net_name":"VCC","layer":"F.Cu","points":points}],"vias":[{"net_name":"VCC","x":5,"y":5}]});
    let (err, r) = h.call("route_batch", args).await;
    assert!(err, "{r:#}");
    h.unchanged();
}
#[tokio::test]
async fn creation_receipts_and_post_write_faults_are_uncertain() {
    for fault in [
        Fault {
            ignore_create: true,
            ..Default::default()
        },
        Fault {
            reject_create: true,
            ..Default::default()
        },
        Fault {
            missing_receipt: true,
            ..Default::default()
        },
        Fault {
            duplicate_receipt: true,
            ..Default::default()
        },
        Fault {
            old_receipt: true,
            ..Default::default()
        },
        Fault {
            wrong_net: true,
            ..Default::default()
        },
        Fault {
            wrong_via_net: true,
            ..Default::default()
        },
        Fault {
            wrong_via_geometry: true,
            ..Default::default()
        },
        Fault {
            wrong_shape: true,
            ..Default::default()
        },
        Fault {
            corrupt_existing: true,
            ..Default::default()
        },
        Fault {
            extra_item: true,
            ..Default::default()
        },
        Fault {
            fail_readback: true,
            ..Default::default()
        },
    ] {
        let h = Harness::new(fault);
        let mut args = batch();
        if fault.duplicate_receipt {
            let shape =
                json!({"net_name":"VCC","layer":"F.Cu","points":[{"x":20,"y":20},{"x":30,"y":20}]});
            args["tracks"] = json!([shape.clone(), shape]);
            args["vias"] = json!([]);
        }
        if fault.old_receipt {
            args["tracks"][0]["points"] = json!([{"x":10,"y":20},{"x":15,"y":20},{"x":30,"y":20}]);
        }
        let (err, r) = h.call("route_batch", args).await;
        assert!(err, "{r:#}");
        assert_eq!(r["outcome"]["status"], "uncertain", "{r:#}");
        assert_eq!(r["outcome"]["retry"]["safe"], false);
        assert_eq!(h.commits.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn modify_preserves_kiid_and_omitted_fields() {
    let h = Harness::new(Fault::default());
    let before = h.held.lock().unwrap()[0].clone();
    let (err, r) = h
        .call("modify_trace", json!({"uuid":"old-trace","width":0.5}))
        .await;
    assert!(!err, "{r:#}");
    assert_eq!(r["modified_uuid"], "old-trace");
    assert_eq!(r["modified_count"], 1);
    assert_eq!(r["created_count"], 0);
    let after = h.held.lock().unwrap()[0].clone();
    let mut expected = pcb::Track::decode(before.value.as_slice()).unwrap();
    expected.width = Some(konnect_ipc::builders::distance(0.5));
    assert_eq!(after, pack_any(&expected, "kiapi.board.types.Track"));
    assert_eq!(h.held.lock().unwrap().len(), 2);
}
#[tokio::test]
async fn modify_and_delete_require_observed_type_and_unlocked_identity() {
    for (name, args) in [
        ("modify_trace", json!({"uuid":"old-via","width":0.5})),
        ("delete_via", json!({"uuid":"old-trace"})),
        ("modify_trace", json!({"uuid":"missing","width":0.5})),
        ("delete_via", json!({"uuid":"missing"})),
        ("modify_trace", json!({"uuid":"old-trace"})),
    ] {
        let h = Harness::new(Fault::default());
        let (err, r) = h.call(name, args).await;
        assert!(err, "{r:#}");
        h.unchanged();
    }
    for (name, args) in [
        ("modify_trace", json!({"uuid":"old-trace","width":0.5})),
        ("delete_via", json!({"uuid":"old-via"})),
    ] {
        let h = Harness::new(Fault {
            locked: true,
            ..Default::default()
        });
        assert!(h.call(name, args).await.0);
        h.unchanged();
    }
}
#[tokio::test]
async fn ignored_updates_and_deletes_do_not_report_success() {
    for (name, args, fault) in [
        (
            "modify_trace",
            json!({"uuid":"old-trace","width":0.5}),
            Fault {
                ignore_update: true,
                ..Default::default()
            },
        ),
        (
            "delete_via",
            json!({"uuid":"old-via"}),
            Fault {
                ignore_delete: true,
                ..Default::default()
            },
        ),
    ] {
        let h = Harness::new(fault);
        let (err, r) = h.call(name, args).await;
        assert!(err);
        assert_eq!(r["outcome"]["status"], "uncertain", "{r:#}");
    }
}
#[tokio::test]
async fn query_and_delete_via_use_actual_live_ids() {
    let h = Harness::new(Fault::default());
    let (err, r) = h.call("query_vias", json!({"limit":1})).await;
    assert!(!err, "{r:#}");
    assert_eq!(r["vias"][0]["uuid"], "old-via");
    assert_eq!(r["vias"][0]["pad_size"], 0.6);
    assert_eq!(r["count"], 1);
    let (err, r) = h.call("delete_via", json!({"uuid":"old-via"})).await;
    assert!(!err, "{r:#}");
    assert_eq!(r["deleted_count"], 1);
    assert_eq!(r["deleted"], json!(["old-via"]));
    assert_eq!(h.held.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn single_trace_and_via_share_verified_creation() {
    for (name, args, kind) in [
        ("route_trace", segment(), "track"),
        ("add_via", json!({"net_name":"VCC","x":20,"y":30}), "via"),
    ] {
        let h = Harness::new(Fault::default());
        let (err, r) = h.call(name, args).await;
        assert!(!err, "{r:#}");
        assert_eq!(r["created"][0]["kind"], kind);
        assert_eq!(r["created"][0]["uuid"], "created-0");
    }
}

#[test]
fn input_boundaries_reject_malformed_values_and_keep_valid_limits() {
    for value in [json!(null), json!(true), json!([])] {
        assert!(number(&json!({"x":value}), "x").is_err());
    }
    assert!(text(&json!({"net_name":false}), "net_name").is_err());
    for value in ["", "bad\0net"] {
        assert!(text(&json!({"net_name":value}), "net_name").is_err());
    }
    assert!(coordinate(&json!({"x":2000}), "x").is_ok());
    assert!(coordinate(&json!({"x":2000.1}), "x").is_err());
    for value in [0.0, -1.0, 0.0000001, 101.0] {
        assert!(size(value).is_err());
    }
    assert!(size(0.000001).is_ok());
    assert!(points(&json!({"points":[{"x":0,"y":0}]})).is_err());
    assert!(points(&json!({"points":vec![json!({"x":0,"y":0});202]})).is_err());
    assert!(object_id(&prost_types::Any {
        type_url: "bad".into(),
        value: vec![]
    })
    .is_err());
    assert!(object_id(&pack_any(&pcb::Track::default(), "kiapi.board.types.Track")).is_err());
    let empty = pcb::Track {
        id: Some(common::Kiid {
            value: String::new(),
        }),
        ..Default::default()
    };
    assert!(object_id(&pack_any(&empty, "kiapi.board.types.Track")).is_err());
}
#[tokio::test]
async fn nanometer_rounding_precedes_width_and_annulus_checks() {
    let h = Harness::new(Fault::default());
    let mut args = segment();
    args["width"] = json!(0.2499997);
    let (err, r) = h.call("route_trace", args).await;
    assert!(!err, "{r:#}");
    assert_eq!(r["width"], 0.25);
    assert_eq!(r["checks"][0]["width"], 0.25);
    let h = Harness::new(Fault::default());
    let (err, r) = h
        .call(
            "add_via",
            json!({"net_name":"VCC","x":20,"y":20,"drill":0.3,"pad_size":0.30000001}),
        )
        .await;
    assert!(err, "{r:#}");
    h.unchanged();
}
#[tokio::test]
async fn ambiguous_existing_inventory_is_not_valid_empty_state() {
    for empty in [false, true] {
        let h = Harness::new(Fault::default());
        {
            let mut items = h.held.lock().unwrap();
            if empty {
                let mut t = pcb::Track::decode(items[0].value.as_slice()).unwrap();
                t.id = None;
                items[0] = pack_any(&t, "kiapi.board.types.Track");
            } else {
                items.push(raw_track("old-trace"));
            }
        }
        let (err, r) = h.call("query_vias", json!({})).await;
        assert!(err, "{r:#}");
        h.unchanged();
    }
}
#[tokio::test]
async fn empty_batch_is_a_refusal_and_via_queries_page_actual_items() {
    let h = Harness::new(Fault::default());
    let (err, r) = h.call("route_batch", json!({"tracks":[],"vias":[]})).await;
    assert!(err, "{r:#}");
    h.unchanged();
    assert!(!h.call("route_batch", batch()).await.0);
    let (err, r) = h.call("query_vias", json!({"limit":1,"offset":1})).await;
    assert!(!err, "{r:#}");
    assert_eq!(r["count"], 2);
    assert_eq!(r["vias"].as_array().unwrap().len(), 1);
    let (_, r) = h.call("query_vias", json!({"net_name":"GND"})).await;
    assert_eq!(r["count"], 0);
}

#[test]
fn live_rule_gaps_are_not_defaults_or_permissions() {
    let class = konnect_ipc::IpcRoutingRules {
        class_name: "Power".into(),
        constituents: vec![],
        track_width_mm: Some(0.25),
        clearance_mm: Some(0.2),
        via_diameter_mm: Some(0.6),
        via_drill_mm: Some(0.3),
    };
    let mut rules = Rules {
        nets: BTreeMap::from([("VCC".into(), 1)]),
        rules: BTreeMap::from([("VCC".into(), class.clone())]),
        layers: BTreeSet::from([
            pcb::BoardLayer::BlFCu as i32,
            pcb::BoardLayer::BlBCu as i32,
            pcb::BoardLayer::BlFSilkS as i32,
        ]),
    };
    rules.nets.clear();
    assert!(rules.net("VCC").is_err());
    rules.nets.insert("VCC".into(), 1);
    rules.rules.clear();
    assert!(rules.net("VCC").is_err());
    rules.rules.insert("VCC".into(), class);
    assert!(rules.layer("F.SilkS").is_err());
    assert!(rules.layer("In1.Cu").is_err());
    assert!(rules.layer("banana.Cu").is_err());
    rules.rules.get_mut("VCC").unwrap().track_width_mm = None;
    assert!(rules
        .track(
            &json!({"net_name":"VCC","layer":"F.Cu","width":0.5}),
            &[(0.0, 0.0), (1.0, 0.0)],
            &mut Vec::new()
        )
        .is_err());
    rules.rules.get_mut("VCC").unwrap().via_drill_mm = None;
    assert!(rules.via(&json!({"net_name":"VCC","x":0,"y":0})).is_err());
    rules.rules.get_mut("VCC").unwrap().via_drill_mm = Some(0.3);
    rules.rules.get_mut("VCC").unwrap().via_diameter_mm = None;
    assert!(rules.via(&json!({"net_name":"VCC","x":0,"y":0})).is_err());
}

#[tokio::test]
#[ignore = "build the release fork at HEAD first"]
async fn routing_release_binary_stdio_dogfood() {
    use std::io::{BufRead, Write};
    use std::path::PathBuf;
    let h = Harness::new(Fault::default());
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
    let upstream = h.server.address().to_string();
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
            "ipc_address = {}\neager_toolsets = true\n",
            serde_json::to_string(&address).unwrap()
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
        serde_json::json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"sync-dogfood","version":"1"}}),
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

    let mut args = batch();
    args["board"] = json!(h.board);
    let result = call("route_batch", args);
    assert_eq!(result["created_count"], 3);
    assert_eq!(result["outcome"]["status"], "complete");
    let via = result["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "via")
        .unwrap()["uuid"]
        .clone();
    let result = call(
        "modify_trace",
        json!({"board":h.board,"uuid":"old-trace","width":0.5}),
    );
    assert_eq!(result["modified_uuid"], "old-trace");
    assert_eq!(result["width"], 0.5);
    let result = call("query_vias", json!({"board":h.board,"limit":1}));
    assert_eq!(result["count"], 2);
    assert_eq!(result["truncated"], true);
    let result = call("delete_via", json!({"board":h.board,"uuid":via}));
    assert_eq!(result["deleted_count"], 1);
    assert_eq!(h.held.lock().unwrap().len(), 4);
    assert_eq!(std::fs::read_to_string(&h.board).unwrap(), FIXTURE);
}

#[tokio::test]
async fn inventory_response_type_must_match_requested_class() {
    let h = Harness::new(Fault {
        inventory_swap: true,
        ..Default::default()
    });
    let (err, r) = h.call("query_vias", json!({})).await;
    assert!(err, "{r:#}");
    h.unchanged();
}
