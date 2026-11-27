//! The three `.kicad_pro` writers while KiCad holds the board (#791).
//!
//! Saving the board in KiCad rewrites the project file from KiCad's own copy.
//! So `set_design_rules`, `create_netclass` and `assign_net_to_class` must not
//! write while KiCad holds the board: the write would be reported and then
//! reverted. Measured on KiCad 10.0.5 before the fix: all three reported
//! success, and `run_drc` with `sync_live_board` then put every value back.
//!
//! Driven through served `tools/call`, so dispatch, schema and handler are all
//! under test. The project is the KiCad-written placement fixture
//! (`konnect-sexp/tests/fixtures/placement/`).

use crate::mcp::handler::McpHandler;
use crate::mcp::protocol::{CallToolResult, ToolContent};
use crate::tools::pcb_board::board_mock::spawn_kicad_holding_boards;
use crate::tools::ServerConfig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const FIXTURE_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../konnect-sexp/tests/fixtures/placement"
);

/// A copy of the fixture project, kept alive for the test's duration.
struct Project {
    _dir: tempfile::TempDir,
    board: PathBuf,
    project_file: PathBuf,
}

fn project() -> Project {
    let dir = tempfile::tempdir().unwrap();
    for extension in ["kicad_pcb", "kicad_pro"] {
        std::fs::copy(
            Path::new(FIXTURE_DIR).join(format!("placement_fixture.{extension}")),
            dir.path().join(format!("placement_fixture.{extension}")),
        )
        .unwrap();
    }
    Project {
        board: dir.path().join("placement_fixture.kicad_pcb"),
        project_file: dir.path().join("placement_fixture.kicad_pro"),
        _dir: dir,
    }
}

async fn handler_talking_to(address: &str) -> McpHandler {
    McpHandler::new(ServerConfig {
        kicad_cli: String::new(),
        kicad_binary: String::new(),
        ipc_address: address.to_string(),
        project_dir: None,
        jlcpcb_db_path: None,
        auto_load_toolsets: true,
        eager_toolsets: true,
    })
    .await
    .expect("handler builds")
}

async fn call(handler: &McpHandler, tool: &str, board: &Path, arguments: &Value) -> CallToolResult {
    let mut arguments = arguments.clone();
    arguments["board"] = json!(board.to_string_lossy());
    let response = handler
        .handle_message(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": tool, "arguments": arguments }
        }))
        .await
        .expect("tools/call receives a response");
    assert!(response.error.is_none(), "tool errors are MCP results");
    serde_json::from_value(response.result.expect("a result"))
        .expect("result uses the advertised MCP shape")
}

fn text_of(result: &CallToolResult) -> &str {
    match result.content.first() {
        Some(ToolContent::Text { text }) => text,
        other => panic!("expected text content, got {other:?}"),
    }
}

/// Arguments that change something the fixture's project file does not
/// already hold (its rules say 0.2, it has only `Default`, and no net is
/// assigned), so a write that happened would show in the file.
fn arguments_for(tool: &str) -> Value {
    match tool {
        "set_design_rules" => json!({ "min_trace_width": 0.35 }),
        "create_netclass" => json!({ "name": "Power", "trace_width": 0.5 }),
        "assign_net_to_class" => json!({ "net_name": "VCC", "netclass": "Default" }),
        other => panic!("no arguments for {other}"),
    }
}

/// KiCad holds this very board, and the mock answers nothing but the open
/// documents: the tool refuses, and neither the project file nor the board
/// changes by a byte.
async fn assert_refused_while_kicad_holds_the_board(tool: &str) {
    let project = project();
    let project_before = std::fs::read(&project.project_file).unwrap();
    let board_before = std::fs::read(&project.board).unwrap();
    let kicad = spawn_kicad_holding_boards(&[&project.board], |_| None);
    let handler = handler_talking_to(kicad.address()).await;

    let result = call(&handler, tool, &project.board, &arguments_for(tool)).await;

    assert!(
        result.is_error,
        "{tool} must refuse while KiCad holds the board: {}",
        text_of(&result)
    );
    let why = match tool {
        // Row RW: these two now take the live path; a KiCad that does not say
        // which version it is, or a pattern write, still refuses, saying why.
        "create_netclass" => "did not say which version it is",
        "assign_net_to_class" => "has no command for netclass patterns",
        _ => "currently holds this board open",
    };
    assert!(
        text_of(&result).contains(why),
        "{tool} must say why it refused: {}",
        text_of(&result)
    );
    assert_eq!(
        std::fs::read(&project.project_file).unwrap(),
        project_before,
        "{tool} wrote the project file while KiCad held the board"
    );
    assert_eq!(std::fs::read(&project.board).unwrap(), board_before);
}

#[tokio::test]
async fn set_design_rules_refuses_while_kicad_holds_the_board() {
    assert_refused_while_kicad_holds_the_board("set_design_rules").await;
}

#[tokio::test]
async fn create_netclass_refuses_while_kicad_holds_the_board() {
    assert_refused_while_kicad_holds_the_board("create_netclass").await;
}

#[tokio::test]
async fn assign_net_to_class_refuses_while_kicad_holds_the_board() {
    assert_refused_while_kicad_holds_the_board("assign_net_to_class").await;
}

/// The control: a KiCad holding some other board does not own this project
/// file, so each writer still writes, and the file says so.
#[tokio::test]
async fn each_writer_still_writes_while_kicad_holds_a_different_board() {
    let elsewhere = tempfile::tempdir().unwrap();
    let other_board = elsewhere.path().join("other.kicad_pcb");
    std::fs::copy(
        Path::new(FIXTURE_DIR).join("placement_fixture.kicad_pcb"),
        &other_board,
    )
    .unwrap();

    for tool in ["set_design_rules", "create_netclass", "assign_net_to_class"] {
        let project = project();
        let kicad = spawn_kicad_holding_boards(&[&other_board], |_| None);
        let handler = handler_talking_to(kicad.address()).await;

        let result = call(&handler, tool, &project.board, &arguments_for(tool)).await;
        assert!(!result.is_error, "{tool}: {}", text_of(&result));

        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&project.project_file).unwrap()).unwrap();
        let net_settings = &written["net_settings"];
        match tool {
            "set_design_rules" => assert_eq!(
                written["board"]["design_settings"]["rules"]["min_track_width"],
                json!(0.35)
            ),
            "create_netclass" => assert!(net_settings["classes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|class| class["name"] == "Power" && class["track_width"] == json!(0.5))),
            "assign_net_to_class" => assert_eq!(
                net_settings["netclass_patterns"],
                json!([{ "netclass": "Default", "pattern": "VCC" }])
            ),
            _ => unreachable!(),
        }
    }
}

// ─── Row RW: netclass writes while KiCad holds the board ─────────────────────

mod live {
    use super::*;
    use crate::test_support::MockIpcServer;
    use kiapi::common::project::{NetClass, NetClassType};
    use konnect_ipc::gen::kiapi;
    use prost::Message;
    use std::sync::{Arc, Mutex};

    /// KiCad's project as its `API_HANDLER_COMMON` serves it, with
    /// `NETCLASS::Deserialize`'s rules (`netclass.cpp:208-269`): name and
    /// priority always set (an absent priority reads 0); an `NCT_IMPLICIT`
    /// message stops there; every other field only when present. Every class
    /// is listed as KiCad 10.0.1 lists it: `NCT_IMPLICIT`, itself as its one
    /// constituent.
    pub(super) struct Kicad {
        /// What `GetVersion` answers: (major, minor, patch).
        pub version: (u32, u32, u32),
        /// Answer `SetNetClasses` with an error status.
        pub refuse_writes: bool,
        pub classes: Vec<NetClass>,
        /// Answer `SetNetClasses` and apply nothing.
        pub drop_writes: bool,
        /// Whether this KiCad also has the schematic of this name open.
        pub schematic: Option<String>,
        pub set_calls: usize,
    }

    impl Default for Kicad {
        fn default() -> Self {
            Self {
                // The KiCad on this Mac (2026-11-27).
                version: (10, 0, 8),
                refuse_writes: false,
                classes: Vec::new(),
                drop_writes: false,
                schematic: None,
                set_calls: 0,
            }
        }
    }

    fn apply(target: &mut NetClass, message: &NetClass) {
        target.name = message.name.clone();
        target.priority = Some(message.priority.unwrap_or(0));
        if message.r#type == NetClassType::NctImplicit as i32 {
            return;
        }
        let Some(board) = &message.board else { return };
        let held = target.board.get_or_insert_with(Default::default);
        macro_rules! take {
            ($($field:ident),*) => {$(if board.$field.is_some() { held.$field = board.$field.clone(); })*};
        }
        take!(
            clearance,
            track_width,
            diff_pair_track_width,
            diff_pair_gap,
            diff_pair_via_gap
        );
        if let Some(stack) = &board.via_stack {
            let into = held.via_stack.get_or_insert_with(Default::default);
            if !stack.copper_layers.is_empty() {
                into.copper_layers = stack.copper_layers.clone();
            }
            if stack.drill.is_some() {
                into.drill = stack.drill;
            }
        }
    }

    fn listed(class: &NetClass) -> NetClass {
        let mut class = class.clone();
        class.r#type = NetClassType::NctImplicit as i32;
        class.constituents = vec![class.name.clone()];
        class
    }

    pub(super) fn spawn(board: &Path, kicad: Arc<Mutex<Kicad>>) -> MockIpcServer {
        let board_doc =
            crate::tools::pcb_board::board_mock::board_document(&board.to_string_lossy());
        MockIpcServer::spawn("kicad-project", move |request| {
            let command = request.message.expect("a command");
            let mut state = kicad.lock().unwrap();
            let ok = |message: Option<prost_types::Any>| kiapi::common::ApiResponse {
                status: Some(kiapi::common::ApiResponseStatus {
                    status: kiapi::common::ApiStatusCode::AsOk as i32,
                    error_message: String::new(),
                }),
                header: None,
                message,
            };
            let url = command.type_url.as_str();
            if url.ends_with("GetOpenDocuments") {
                let asked =
                    kiapi::common::commands::GetOpenDocuments::decode(command.value.as_slice())
                        .unwrap();
                let documents = if asked.r#type
                    == kiapi::common::types::DocumentType::DoctypeSchematic as i32
                {
                    state
                        .schematic
                        .iter()
                        .map(|name| kiapi::common::types::DocumentSpecifier {
                            r#type: asked.r#type,
                            project: None,
                            identifier: Some(
                                kiapi::common::types::document_specifier::Identifier::BoardFilename(
                                    name.clone(),
                                ),
                            ),
                        })
                        .collect()
                } else {
                    vec![board_doc.clone()]
                };
                return ok(Some(konnect_ipc::builders::pack_any(
                    &kiapi::common::commands::GetOpenDocumentsResponse { documents },
                    "kiapi.common.commands.GetOpenDocumentsResponse",
                )));
            }
            if url.ends_with("GetNetClasses") {
                let net_classes = state.classes.iter().map(listed).collect();
                return ok(Some(konnect_ipc::builders::pack_any(
                    &kiapi::common::commands::NetClassesResponse { net_classes },
                    "kiapi.common.commands.NetClassesResponse",
                )));
            }
            if url.ends_with("GetVersion") {
                let (major, minor, patch) = state.version;
                return ok(Some(konnect_ipc::builders::pack_any(
                    &kiapi::common::commands::GetVersionResponse {
                        version: Some(kiapi::common::types::KiCadVersion {
                            major,
                            minor,
                            patch,
                            full_version: format!("{major}.{minor}.{patch}"),
                        }),
                    },
                    "kiapi.common.commands.GetVersionResponse",
                )));
            }
            if url.ends_with("SetNetClasses") {
                state.set_calls += 1;
                if state.refuse_writes {
                    return kiapi::common::ApiResponse {
                        status: Some(kiapi::common::ApiResponseStatus {
                            status: kiapi::common::ApiStatusCode::AsBadRequest as i32,
                            error_message: "could not unpack netclass".into(),
                        }),
                        header: None,
                        message: None,
                    };
                }
                let asked =
                    kiapi::common::commands::SetNetClasses::decode(command.value.as_slice())
                        .unwrap();
                assert_eq!(
                    asked.merge_mode,
                    kiapi::common::types::MapMergeMode::MmmMerge as i32
                );
                if !state.drop_writes {
                    for message in &asked.net_classes {
                        match state.classes.iter_mut().find(|c| c.name == message.name) {
                            Some(existing) => apply(existing, message),
                            None => {
                                // `std::make_shared<NETCLASS>( name, false )`.
                                let mut fresh = NetClass {
                                    name: message.name.clone(),
                                    priority: Some(-1),
                                    ..Default::default()
                                };
                                apply(&mut fresh, message);
                                state.classes.push(fresh);
                            }
                        }
                    }
                }
                return ok(None);
            }
            ok(None)
        })
    }

    fn nm(mm: f64) -> kiapi::common::types::Distance {
        kiapi::common::types::Distance {
            value_nm: (mm * 1e6).round() as i64,
        }
    }

    /// KiCad's Default as the fixture's project file has it.
    pub(super) fn kicad_default() -> NetClass {
        let mut class = NetClass {
            name: "Default".into(),
            priority: Some(i32::MAX),
            ..Default::default()
        };
        let board = class.board.get_or_insert_with(Default::default);
        board.clearance = Some(nm(0.2));
        board.track_width = Some(nm(0.2));
        board.diff_pair_track_width = Some(nm(0.2));
        board.diff_pair_gap = Some(nm(0.25));
        board.diff_pair_via_gap = Some(nm(0.25));
        board.via_stack = Some(kiapi::board::types::PadStack {
            copper_layers: vec![kiapi::board::types::PadStackLayer {
                size: Some(kiapi::common::types::Vector2 {
                    x_nm: 600_000,
                    y_nm: 600_000,
                }),
                ..Default::default()
            }],
            drill: Some(kiapi::board::types::DrillProperties {
                diameter: Some(kiapi::common::types::Vector2 {
                    x_nm: 300_000,
                    y_nm: 300_000,
                }),
                ..Default::default()
            }),
            ..Default::default()
        });
        class
    }

    fn file_json(project: &Project) -> Value {
        serde_json::from_str(&std::fs::read_to_string(&project.project_file).unwrap()).unwrap()
    }

    fn body(result: &CallToolResult) -> Value {
        assert!(!result.is_error, "{}", text_of(result));
        serde_json::from_str(text_of(result)).unwrap()
    }

    fn track_width_nm(kicad: &Kicad, name: &str) -> Option<i64> {
        kicad
            .classes
            .iter()
            .find(|c| c.name == name)?
            .board
            .as_ref()?
            .track_width
            .as_ref()
            .map(|d| d.value_nm)
    }

    /// The owner's case: a width change with the board open. KiCad's copy
    /// takes it first (read back), then the file; nothing else in the file
    /// moves.
    #[tokio::test]
    async fn a_new_class_reaches_kicad_and_the_file_with_the_board_open() {
        let project = project();
        let before = file_json(&project);
        let kicad = Arc::new(Mutex::new(Kicad {
            classes: vec![kicad_default()],
            ..Default::default()
        }));
        let server = spawn(&project.board, kicad.clone());
        let handler = handler_talking_to(server.address()).await;

        let result = call(
            &handler,
            "create_netclass",
            &project.board,
            &json!({ "name": "Power", "trace_width": 0.4 }),
        )
        .await;
        let body = body(&result);

        assert_eq!(
            body["written_to"],
            json!(["kicad", "project_file"]),
            "{body}"
        );
        assert_eq!(body["kicad"]["kicad_copy"], "created");
        assert_eq!(body["kicad"]["kicad_unsaved"], json!([]), "{body}");
        assert_eq!(
            track_width_nm(&kicad.lock().unwrap(), "Power"),
            Some(400_000)
        );
        let after = file_json(&project);
        let power = after["net_settings"]["classes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "Power")
            .unwrap()
            .clone();
        assert_eq!(power["track_width"], json!(0.4));
        assert_eq!(power["clearance"], json!(0.2));
        // Project text kept intact: only the new class is added.
        let mut expected = before.clone();
        expected["net_settings"]["classes"]
            .as_array_mut()
            .unwrap()
            .push(power);
        assert_eq!(after, expected);
        assert_eq!(kicad.lock().unwrap().set_calls, 1);
    }

    /// An update names one field; KiCad's other values of that class, and
    /// KiCad's other classes, stay as they were.
    #[tokio::test]
    async fn an_update_changes_only_the_named_field_in_both_copies() {
        let project = project();
        let mut power = NetClass {
            name: "Power".into(),
            priority: Some(2),
            ..Default::default()
        };
        power.board.get_or_insert_with(Default::default).clearance = Some(nm(0.3));
        power.board.as_mut().unwrap().track_width = Some(nm(0.25));
        let mut file = file_json(&project);
        file["net_settings"]["classes"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "name": "Power", "priority": 2, "clearance": 0.3, "track_width": 0.25 }));
        std::fs::write(
            &project.project_file,
            format!("{}\n", serde_json::to_string_pretty(&file).unwrap()),
        )
        .unwrap();
        let kicad = Arc::new(Mutex::new(Kicad {
            classes: vec![kicad_default(), power],
            ..Default::default()
        }));
        let server = spawn(&project.board, kicad.clone());
        let handler = handler_talking_to(server.address()).await;

        let body = body(
            &call(
                &handler,
                "create_netclass",
                &project.board,
                &json!({ "name": "Power", "trace_width": 0.5 }),
            )
            .await,
        );

        assert_eq!(body["kicad"]["kicad_copy"], "updated");
        assert_eq!(body["updated_existing"], true);
        let state = kicad.lock().unwrap();
        let held = state.classes.iter().find(|c| c.name == "Power").unwrap();
        assert_eq!(held.priority, Some(2));
        assert_eq!(held.board.as_ref().unwrap().clearance, Some(nm(0.3)));
        assert_eq!(held.board.as_ref().unwrap().track_width, Some(nm(0.5)));
        assert_eq!(state.classes[0], kicad_default());
        let after = file_json(&project);
        let saved = after["net_settings"]["classes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "Power")
            .unwrap();
        assert_eq!(
            saved,
            &json!({ "name": "Power", "priority": 2, "clearance": 0.3, "track_width": 0.5 })
        );
    }

    /// KiCad holds a value its file does not have yet (a Board Setup edit not
    /// saved): both writers and the reader name it, and the write still lands.
    #[tokio::test]
    async fn what_kicad_holds_and_has_not_saved_is_named() {
        let project = project();
        let mut unsaved = kicad_default();
        unsaved.board.as_mut().unwrap().track_width = Some(nm(0.4));
        let kicad = Arc::new(Mutex::new(Kicad {
            classes: vec![unsaved],
            ..Default::default()
        }));
        let server = spawn(&project.board, kicad.clone());
        let handler = handler_talking_to(server.address()).await;

        let read = body(&call(&handler, "get_netclasses", &project.board, &json!({})).await);
        let expected = json!([{ "netclass": "Default", "field": "track_width", "kicad_mm": 0.4, "project_file_mm": 0.2 }]);
        assert_eq!(read["kicad_unsaved"], expected, "{read}");
        assert_eq!(read["sources"]["kicad_unsaved"], "ipc");
        assert!(read["warning"]
            .as_str()
            .unwrap()
            .contains("save the board in KiCad"));

        let wrote = body(
            &call(
                &handler,
                "create_netclass",
                &project.board,
                &json!({ "name": "Power", "trace_width": 0.3 }),
            )
            .await,
        );
        assert_eq!(wrote["kicad"]["kicad_unsaved"], expected, "{wrote}");
        assert!(wrote["warning"].as_str().is_some());
    }

    /// The board-open guard's negative case: KiCad answers `SetNetClasses`
    /// but its copy does not change (as with a class sent `NCT_IMPLICIT`).
    /// The call says so, and the file is not written.
    #[tokio::test]
    async fn a_write_kicad_did_not_take_is_refused_and_the_file_is_kept() {
        let project = project();
        let before = std::fs::read(&project.project_file).unwrap();
        let kicad = Arc::new(Mutex::new(Kicad {
            classes: vec![kicad_default()],
            drop_writes: true,
            ..Default::default()
        }));
        let server = spawn(&project.board, kicad.clone());
        let handler = handler_talking_to(server.address()).await;

        let result = call(
            &handler,
            "create_netclass",
            &project.board,
            &json!({ "name": "Default", "trace_width": 0.3 }),
        )
        .await;

        assert!(result.is_error);
        assert!(
            text_of(&result).contains("could not be confirmed"),
            "{}",
            text_of(&result)
        );
        assert_eq!(std::fs::read(&project.project_file).unwrap(), before);
    }

    /// A schematic open in another KiCad keeps its own copy of the project:
    /// refused with nothing sent. Open in the same KiCad as the board: written.
    #[tokio::test]
    async fn a_schematic_open_elsewhere_refuses_and_one_open_here_does_not() {
        for here in [false, true] {
            let project = project();
            let schematic = project.board.with_extension("kicad_sch");
            std::fs::write(&schematic, "(kicad_sch)").unwrap();
            let lock = konnect_sexp::writer::kicad_editor_lock_path(&schematic).unwrap();
            std::fs::write(&lock, "held").unwrap();
            let before = std::fs::read(&project.project_file).unwrap();
            let kicad = Arc::new(Mutex::new(Kicad {
                classes: vec![kicad_default()],
                schematic: here.then(|| "placement_fixture.kicad_sch".to_string()),
                ..Default::default()
            }));
            let server = spawn(&project.board, kicad.clone());
            let handler = handler_talking_to(server.address()).await;

            let result = call(
                &handler,
                "create_netclass",
                &project.board,
                &json!({ "name": "Power", "trace_width": 0.3 }),
            )
            .await;

            if here {
                assert!(!result.is_error, "{}", text_of(&result));
                assert_eq!(kicad.lock().unwrap().set_calls, 1);
            } else {
                assert!(result.is_error);
                assert!(
                    text_of(&result).contains("another KiCad"),
                    "{}",
                    text_of(&result)
                );
                assert_eq!(kicad.lock().unwrap().set_calls, 0);
                assert_eq!(std::fs::read(&project.project_file).unwrap(), before);
            }
        }
    }

    /// A value KiCad cannot hold is refused before anything is sent.
    #[tokio::test]
    async fn a_value_out_of_range_is_refused_before_kicad_is_asked() {
        let project = project();
        let kicad = Arc::new(Mutex::new(Kicad {
            classes: vec![kicad_default()],
            ..Default::default()
        }));
        let server = spawn(&project.board, kicad.clone());
        let handler = handler_talking_to(server.address()).await;
        let result = call(
            &handler,
            "create_netclass",
            &project.board,
            &json!({ "name": "Power", "trace_width": -0.2 }),
        )
        .await;
        assert!(result.is_error);
        assert_eq!(kicad.lock().unwrap().set_calls, 0);
    }

    /// A pattern write while KiCad holds the board: refused with the steps in
    /// KiCad's own dialog, and nothing written.
    #[tokio::test]
    async fn a_pattern_write_with_the_board_open_gives_the_dialog_steps() {
        let project = project();
        let before = std::fs::read(&project.project_file).unwrap();
        let kicad = Arc::new(Mutex::new(Kicad {
            classes: vec![kicad_default()],
            ..Default::default()
        }));
        let server = spawn(&project.board, kicad.clone());
        let handler = handler_talking_to(server.address()).await;
        let result = call(
            &handler,
            "assign_net_to_class",
            &project.board,
            &json!({ "net_name": "VCC", "netclass": "Default" }),
        )
        .await;
        assert!(result.is_error);
        let text = text_of(&result);
        assert!(
            text.contains("Board Setup > Design Rules > Net Classes")
                && text.contains("Pattern 'VCC'"),
            "{text}"
        );
        assert_eq!(std::fs::read(&project.project_file).unwrap(), before);
    }

    /// The version gate: a KiCad before 10.0.7 corrupts a class written over
    /// IPC (`netclass.cpp:223` empties its constituents), so it is refused with
    /// nothing sent and the dialog steps given; 10.0.7 is written.
    #[tokio::test]
    async fn a_kicad_before_10_0_7_is_refused_and_10_0_7_is_written() {
        for (version, written) in [
            ((10, 0, 6), false),
            ((10, 0, 1), false),
            ((10, 0, 7), true),
            ((11, 0, 0), true),
        ] {
            let project = project();
            let before = std::fs::read(&project.project_file).unwrap();
            let kicad = Arc::new(Mutex::new(Kicad {
                classes: vec![kicad_default()],
                version,
                ..Default::default()
            }));
            let server = spawn(&project.board, kicad.clone());
            let handler = handler_talking_to(server.address()).await;
            let result = call(
                &handler,
                "create_netclass",
                &project.board,
                &json!({ "name": "Power", "trace_width": 0.3 }),
            )
            .await;
            assert_eq!(
                !result.is_error,
                written,
                "{version:?}: {}",
                text_of(&result)
            );
            if written {
                assert_eq!(kicad.lock().unwrap().set_calls, 1, "{version:?}");
            } else {
                let text = text_of(&result);
                assert!(
                    text.contains("fixed in 10.0.7")
                        && text.contains("Board Setup > Design Rules > Net Classes"),
                    "{text}"
                );
                assert_eq!(kicad.lock().unwrap().set_calls, 0, "{version:?}");
                assert_eq!(std::fs::read(&project.project_file).unwrap(), before);
            }
        }
    }

    /// A class KiCad has and the file lacks (an unsaved Board Setup edit) is
    /// refused before anything is sent, and the other way round.
    #[tokio::test]
    async fn a_class_only_one_copy_has_is_refused_before_anything_is_sent() {
        for kicad_has_it in [true, false] {
            let project = project();
            if !kicad_has_it {
                let mut file = file_json(&project);
                file["net_settings"]["classes"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({ "name": "Power", "priority": -1, "clearance": 1.0 }));
                std::fs::write(
                    &project.project_file,
                    format!("{}\n", serde_json::to_string_pretty(&file).unwrap()),
                )
                .unwrap();
            }
            let before = std::fs::read(&project.project_file).unwrap();
            let mut classes = vec![kicad_default()];
            if kicad_has_it {
                classes.push(NetClass {
                    name: "Power".into(),
                    priority: Some(-1),
                    ..Default::default()
                });
            }
            let kicad = Arc::new(Mutex::new(Kicad {
                classes,
                ..Default::default()
            }));
            let server = spawn(&project.board, kicad.clone());
            let handler = handler_talking_to(server.address()).await;
            let result = call(
                &handler,
                "create_netclass",
                &project.board,
                &json!({ "name": "Power", "trace_width": 0.3 }),
            )
            .await;
            assert!(result.is_error, "{kicad_has_it}");
            assert!(
                text_of(&result).contains("has not saved"),
                "{}",
                text_of(&result)
            );
            assert_eq!(kicad.lock().unwrap().set_calls, 0);
            assert_eq!(std::fs::read(&project.project_file).unwrap(), before);
        }
    }

    /// KiCad refuses `SetNetClasses`: the result says the change was sent and
    /// not confirmed, and the file is kept.
    #[tokio::test]
    async fn a_refused_set_net_classes_keeps_the_file() {
        let project = project();
        let before = std::fs::read(&project.project_file).unwrap();
        let kicad = Arc::new(Mutex::new(Kicad {
            classes: vec![kicad_default()],
            refuse_writes: true,
            ..Default::default()
        }));
        let server = spawn(&project.board, kicad.clone());
        let handler = handler_talking_to(server.address()).await;
        let result = call(
            &handler,
            "create_netclass",
            &project.board,
            &json!({ "name": "Default", "trace_width": 0.3 }),
        )
        .await;
        assert!(result.is_error);
        assert!(
            text_of(&result).contains("did not accept SetNetClasses"),
            "{}",
            text_of(&result)
        );
        assert_eq!(std::fs::read(&project.project_file).unwrap(), before);
    }

    /// The file half fails after KiCad took the change: the result says KiCad
    /// has it, read back, and that its next save writes it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_file_write_failing_after_kicad_took_the_change_says_so() {
        use std::os::unix::fs::PermissionsExt;
        let project = project();
        let dir = project.project_file.parent().unwrap().to_path_buf();
        let before = std::fs::read(&project.project_file).unwrap();
        let kicad = Arc::new(Mutex::new(Kicad {
            classes: vec![kicad_default()],
            ..Default::default()
        }));
        let server = spawn(&project.board, kicad.clone());
        let handler = handler_talking_to(server.address()).await;
        // Readable, not writable: the trial edit and KiCad's write go through,
        // the file's lock and atomic replace cannot.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = call(
            &handler,
            "create_netclass",
            &project.board,
            &json!({ "name": "Default", "trace_width": 0.3 }),
        )
        .await;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(result.is_error);
        let text = text_of(&result);
        assert!(
            text.contains("now has netclass 'Default'") && text.contains("could not be written"),
            "{text}"
        );
        assert_eq!(kicad.lock().unwrap().set_calls, 1);
        assert_eq!(std::fs::read(&project.project_file).unwrap(), before);
    }

    /// A call that names no value on a class both copies have sends nothing.
    #[tokio::test]
    async fn a_call_that_names_nothing_sends_nothing() {
        let project = project();
        let kicad = Arc::new(Mutex::new(Kicad {
            classes: vec![kicad_default()],
            ..Default::default()
        }));
        let server = spawn(&project.board, kicad.clone());
        let handler = handler_talking_to(server.address()).await;
        let body = body(
            &call(
                &handler,
                "create_netclass",
                &project.board,
                &json!({ "name": "Default" }),
            )
            .await,
        );
        assert_eq!(body["kicad"]["kicad_copy"], "updated");
        assert_eq!(kicad.lock().unwrap().set_calls, 0);
    }
}
