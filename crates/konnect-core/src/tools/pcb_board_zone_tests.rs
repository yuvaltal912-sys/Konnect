//! Row ZR, stage ZR4: `add_zone`'s settings (docs/POUR_PLAN_DESIGN.md, "Tests, each guard
//! with a negative control", `add_zone`). One test for each guard of that table.

use super::*;
use crate::tools::pcb_board::board_mock::{ctx_talking_to, spawn_kicad_holding_board};
use crate::zone_fill::{fill_settings, IslandRule, PadConnect};
use konnect_ipc::builders::ZoneIslands;
use konnect_ipc::gen::kiapi;
use konnect_sexp::net::NetRef;
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const KICAD_10: &str = "(kicad_pcb\n\t(version 20260206)\n\t(generator \"pcbnew\")\n\t(segment\n\t\t(start 10 10)\n\t\t(end 20 10)\n\t\t(width 0.2)\n\t\t(layer \"F.Cu\")\n\t\t(net \"GND\")\n\t)\n)\n";

const SQUARE: [(f64, f64); 4] = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];

/// The zone text `add_zone` wrote before this stage (`format_zone_polygon` at `c4c34369`),
/// kept here byte for byte as the oracle of "a default call writes the zone it wrote
/// before". Only the UUID is a parameter.
#[allow(clippy::too_many_arguments)]
fn pre_zr4_zone_text(
    net_nodes: &str,
    layer_node: &str,
    clearance: f64,
    min_width: f64,
    points: &[(f64, f64)],
    name: &str,
    priority: u32,
    connect: &str,
    uuid: &str,
) -> String {
    let pts: String = points
        .iter()
        .map(|(x, y)| format!("\n      (xy {x} {y})"))
        .collect();
    let name_node = if name.is_empty() {
        String::new()
    } else {
        let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\n    (name \"{escaped}\")")
    };
    let priority_node = if priority == 0 {
        String::new()
    } else {
        format!("\n    (priority {priority})")
    };
    let thermal = 0.5;
    let hatch_pitch = 0.508;
    format!(
        "\n  (zone {net_nodes} {layer_node}{name_node} (uuid \"{uuid}\"){priority_node}\n    \
         (hatch edge {hatch_pitch})\n    (connect_pads{connect} (clearance {clearance}))\n    \
         (min_thickness {min_width})\n    \
         (fill yes (thermal_gap {thermal}) (thermal_bridge_width {thermal}))\n    \
         (polygon (pts{pts}\n    ))\n  )"
    )
}

/// `text` with every `(uuid "…")` value replaced by `U`.
fn mask_uuids(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("(uuid \"") {
        let start = at + "(uuid \"".len();
        out.push_str(&rest[..start]);
        let end = rest[start..].find('"').expect("a closed uuid") + start;
        out.push('U');
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn defaults() -> ZoneSettings {
    ZoneSettings::from_args(&json!({})).expect("the defaults read")
}

fn by_name(net: &str) -> NetRef {
    NetRef::ByName(net.to_string())
}

fn text_of(result: &CallToolResult) -> String {
    match result.content.first() {
        Some(crate::mcp::protocol::ToolContent::Text { text }) => text.clone(),
        other => panic!("expected text, got {other:?}"),
    }
}

fn body_of(result: &CallToolResult) -> Value {
    serde_json::from_str(&text_of(result)).expect("tool results are JSON")
}

/// `add_zone` on a scratch copy of `board` with `extra` arguments, on the file path.
async fn add_zone_on_file(board: &str, extra: Value) -> (CallToolResult, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.kicad_pcb");
    std::fs::write(&path, board).unwrap();
    let mut args = json!({
        "board": path.to_str().unwrap(), "net_name": "GND", "layer": "B.Cu",
        "points": [ {"x": 0.0, "y": 0.0}, {"x": 10.0, "y": 0.0}, {"x": 10.0, "y": 10.0}, {"x": 0.0, "y": 10.0} ]
    });
    for (key, value) in extra.as_object().expect("object").clone() {
        args[key] = value;
    }
    let result = add_zone_impl(&args, &ctx_talking_to(String::new()))
        .await
        .unwrap();
    (result, std::fs::read_to_string(&path).unwrap())
}

/// The `(zone …)` that `add_zone` appended to `KICAD_10`.
fn written_zone(after: &str) -> &str {
    let at = after.find("\n  (zone").expect("a zone was written");
    let end = after.rfind(')').unwrap();
    &after[at..end]
}

// ─── A default call writes the zone it wrote before ─────────────────────────────

#[test]
fn a_default_call_writes_the_zone_it_wrote_before() {
    let settings = defaults();
    assert_eq!(settings.thermal_gap_mm, 0.5);
    assert_eq!(settings.thermal_bridge_mm, 0.5);
    assert_eq!(settings.islands, ZoneIslands::Always);
    for (net, expected_nodes, layer_node) in [
        (by_name("GND"), "(net \"GND\")", "(layers \"B.Cu\")"),
        (
            NetRef::ById {
                id: "7".into(),
                name: "GND".into(),
            },
            "(net 7) (net_name \"GND\")",
            "(layer \"B.Cu\")",
        ),
    ] {
        let text = zone_sexp(&net, "B.Cu", &SQUARE, "", &settings);
        let oracle = pre_zr4_zone_text(
            expected_nodes,
            layer_node,
            0.2,
            0.2,
            &SQUARE,
            "",
            0,
            "",
            "U",
        );
        assert_eq!(mask_uuids(&text), oracle);
        assert!(!text.contains("island"), "{text}");
    }
    // A named zone with a priority and a solid connection, as before.
    let solid =
        ZoneSettings::from_args(&json!({"priority": 3, "pad_connection": "solid"})).unwrap();
    assert_eq!(
        mask_uuids(&zone_sexp(
            &by_name("GND"),
            "F.Cu",
            &SQUARE,
            "a \"b\"",
            &solid
        )),
        pre_zr4_zone_text(
            "(net \"GND\")",
            "(layers \"F.Cu\")",
            0.2,
            0.2,
            &SQUARE,
            "a \"b\"",
            3,
            " yes",
            "U"
        )
    );
}

#[tokio::test]
async fn a_default_call_on_the_file_path_writes_the_old_text() {
    let (result, after) = add_zone_on_file(KICAD_10, json!({})).await;
    assert!(!result.is_error, "{}", text_of(&result));
    let oracle = pre_zr4_zone_text(
        "(net \"GND\")",
        "(layers \"B.Cu\")",
        0.2,
        0.2,
        &SQUARE,
        "",
        0,
        "",
        "U",
    );
    assert_eq!(mask_uuids(written_zone(&after)), oracle);
    // Nothing else of the board changed.
    let close = KICAD_10.rfind(')').unwrap();
    assert!(after.starts_with(&KICAD_10[..close]));
    assert!(after.ends_with(&KICAD_10[close..]));
}

// ─── The settings reach the file ────────────────────────────────────────────────

#[tokio::test]
async fn the_settings_reach_the_file() {
    let (result, after) = add_zone_on_file(
        KICAD_10,
        json!({"thermal_gap": 0.3, "thermal_bridge_width": 0.4, "island_removal": "never"}),
    )
    .await;
    assert!(!result.is_error, "{}", text_of(&result));
    let zone = written_zone(&after);
    assert!(
        zone.contains(
            "(fill yes (thermal_gap 0.3) (thermal_bridge_width 0.4) (island_removal_mode 1))"
        ),
        "{zone}"
    );
    assert!(!zone.contains("island_area_min"), "{zone}");

    let (result, after) = add_zone_on_file(
        KICAD_10,
        json!({"island_removal": "below_area", "island_min_area": 2.5}),
    )
    .await;
    assert!(!result.is_error, "{}", text_of(&result));
    let zone = written_zone(&after);
    assert!(
        zone.contains("(island_removal_mode 2) (island_area_min 2.5))"),
        "{zone}"
    );
    assert!(konnect_sexp::parse_sexp(&after).is_ok());
}

// ─── Both paths write one zone ──────────────────────────────────────────────────

/// Every fill setting differs from KiCad's default (`fill_settings` reads a missing node
/// as KiCad's default, which would hide a dropped node) and from `add_zone`'s.
fn odd_settings() -> ZoneSettings {
    ZoneSettings::from_args(&json!({
        "clearance": 0.33, "min_width": 0.27, "priority": 4, "pad_connection": "solid",
        "thermal_gap": 0.37, "thermal_bridge_width": 0.41,
        "island_removal": "below_area", "island_min_area": 3.5
    }))
    .expect("the odd settings are valid")
}

/// What one zone's text says, through the engine's own reader, in nanometres.
fn file_side(text: &str) -> (i64, i64, String, i64, i64, u32, i32, u64) {
    let zone = konnect_sexp::parse_sexp(text).expect("the zone parses");
    let read = fill_settings(&zone).expect("the engine reads the zone");
    let (mode, area) = match read.islands {
        IslandRule::Remove => (1, 0),
        IslandRule::Keep => (2, 0),
        IslandRule::RemoveBelow { area_mm2 } => (3, (area_mm2 * 1e12).round() as u64),
    };
    let connect = match read.connect {
        PadConnect::Solid => "solid",
        PadConnect::Thermal => "thermal",
        PadConnect::None => "none",
        PadConnect::ThermalThroughHole => "thru_hole_only",
    };
    (
        konnect_ipc::builders::mm_to_nm(read.clearance_mm),
        konnect_ipc::builders::mm_to_nm(read.min_width_mm),
        connect.to_string(),
        konnect_ipc::builders::mm_to_nm(read.thermal_gap_mm),
        konnect_ipc::builders::mm_to_nm(read.thermal_bridge_mm),
        read.priority,
        mode,
        area,
    )
}

/// What `build_zone` sends for the same settings, in the same terms.
fn ipc_side(settings: &ZoneSettings) -> (i64, i64, String, i64, i64, u32, i32, u64) {
    use kiapi::board::types::ZoneConnectionStyle;
    let zone = konnect_ipc::builders::build_zone(&settings.spec("F.Cu", "GND", &SQUARE, ""), 1);
    let copper = match zone.settings.expect("settings") {
        kiapi::board::types::zone::Settings::CopperSettings(s) => s,
        other => panic!("{other:?}"),
    };
    let connection = copper.connection.expect("connection");
    let spokes = connection.thermal_spokes.expect("spokes");
    let connect = match ZoneConnectionStyle::try_from(connection.zone_connection).unwrap() {
        ZoneConnectionStyle::ZcsFull => "solid",
        ZoneConnectionStyle::ZcsThermal => "thermal",
        ZoneConnectionStyle::ZcsNone => "none",
        ZoneConnectionStyle::ZcsPthThermal => "thru_hole_only",
        other => panic!("{other:?}"),
    };
    (
        copper.clearance.unwrap().value_nm,
        copper.min_thickness.unwrap().value_nm,
        connect.to_string(),
        spokes.gap.unwrap().value_nm,
        spokes.width.unwrap().value_nm,
        zone.priority,
        copper.island_mode,
        copper.min_island_area,
    )
}

#[test]
fn both_paths_write_one_zone() {
    let settings = odd_settings();
    let text = zone_sexp(&by_name("GND"), "F.Cu", &SQUARE, "", &settings);
    let file = file_side(&text);
    let ipc = ipc_side(&settings);
    assert_eq!(file, ipc, "{text}");
    // The odd values are really odd: none is KiCad's default or add_zone's.
    assert_eq!(
        file,
        (
            330_000,
            270_000,
            "solid".to_string(),
            370_000,
            410_000,
            4,
            kiapi::board::types::IslandRemovalMode::IrmArea as i32,
            3_500_000_000_000
        )
    );
}

// ─── A bridge below the minimum width is refused before anything is written ─────

#[tokio::test]
async fn a_bridge_below_the_minimum_width_is_refused_on_the_file_path() {
    let (result, after) = add_zone_on_file(
        KICAD_10,
        json!({"min_width": 0.5, "thermal_bridge_width": 0.3}),
    )
    .await;
    assert!(result.is_error, "{}", text_of(&result));
    assert!(text_of(&result).contains("thermal_bridge_width"));
    assert_eq!(after, KICAD_10, "the board's bytes are unchanged");
}

#[tokio::test]
async fn a_bridge_below_the_minimum_width_never_reaches_kicad() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.kicad_pcb");
    std::fs::write(&path, KICAD_10).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let server = spawn_kicad_holding_board(&path, move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
        None
    });
    let ctx = ctx_talking_to(server.address().to_string());
    let result = add_zone_impl(
        &json!({
            "board": path.to_str().unwrap(), "net_name": "GND", "layer": "B.Cu",
            "points": [ {"x": 0.0, "y": 0.0}, {"x": 10.0, "y": 0.0}, {"x": 10.0, "y": 10.0} ],
            "min_width": 0.5, "thermal_bridge_width": 0.3
        }),
        &ctx,
    )
    .await
    .unwrap();
    assert!(result.is_error, "{}", text_of(&result));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), KICAD_10);
    // `spawn_kicad_holding_board` answers GetOpenDocuments itself; anything at all that
    // reached the socket would be counted by a second server below, so the request
    // count is taken from a server that counts every request.
    drop(server);
    let all = Arc::new(AtomicUsize::new(0));
    let counted = all.clone();
    let counting = crate::test_support::MockIpcServer::spawn("zr4-counting", move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        kiapi::common::ApiResponse {
            status: Some(kiapi::common::ApiResponseStatus {
                status: kiapi::common::ApiStatusCode::AsBadRequest as i32,
                error_message: "counting mock".to_string(),
            }),
            header: None,
            message: None,
        }
    });
    let ctx = ctx_talking_to(counting.address().to_string());
    let result = add_zone_impl(
        &json!({
            "board": path.to_str().unwrap(), "net_name": "GND", "layer": "B.Cu",
            "points": [ {"x": 0.0, "y": 0.0}, {"x": 10.0, "y": 0.0}, {"x": 10.0, "y": 10.0} ],
            "min_width": 0.5, "thermal_bridge_width": 0.3
        }),
        &ctx,
    )
    .await
    .unwrap();
    assert!(result.is_error, "{}", text_of(&result));
    assert_eq!(all.load(Ordering::SeqCst), 0, "no request reached KiCad");
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), KICAD_10);
}

// ─── Every value ZoneSettings::check refuses ───────────────────────────────────

#[test]
fn every_value_check_refuses_is_an_error_that_names_its_argument() {
    let cases: Vec<(Value, &str)> = vec![
        (json!({"clearance": -0.1}), "clearance"),
        (json!({"thermal_gap": -0.1}), "thermal_gap"),
        (json!({"min_width": 0.0}), "min_width"),
        (json!({"min_width": -0.2}), "min_width"),
        (json!({"thermal_bridge_width": 0.0}), "thermal_bridge_width"),
        (
            json!({"island_removal": "below_area", "island_min_area": 0.0}),
            "island_min_area",
        ),
        (
            json!({"island_removal": "below_area", "island_min_area": 2000.5}),
            "island_min_area",
        ),
        (json!({"island_removal": "sometimes"}), "island_removal"),
        (
            json!({"pad_connection": "thermal_relief"}),
            "pad_connection",
        ),
        (json!({"clearance": "0.2"}), "clearance"),
        (json!({"min_width": "wide"}), "min_width"),
        (json!({"thermal_gap": "0.5"}), "thermal_gap"),
        (
            json!({"thermal_bridge_width": true}),
            "thermal_bridge_width",
        ),
        (json!({"priority": "1"}), "priority"),
        (json!({"priority": -1}), "priority"),
        (json!({"island_removal": 2}), "island_removal"),
        (
            json!({"island_removal": "below_area", "island_min_area": "5"}),
            "island_min_area",
        ),
    ];
    for (args, named) in cases {
        let error = ZoneSettings::from_args(&args).expect_err(&format!("{args} is refused"));
        assert!(
            error.contains(named),
            "{args}: '{error}' does not name {named}"
        );
    }
    // The edges that are accepted.
    for args in [
        json!({"clearance": 0.0, "thermal_gap": 0.0}),
        json!({"island_removal": "below_area", "island_min_area": 2000.0}),
        json!({"min_width": 0.4, "thermal_bridge_width": 0.4}),
    ] {
        assert!(ZoneSettings::from_args(&args).is_ok(), "{args}");
    }
    // `check` itself refuses what no argument can give (a value built directly).
    let mut built = defaults();
    built.clearance_mm = f64::NAN;
    assert!(built.check().unwrap_err().contains("clearance"));
    let mut built = defaults();
    built.thermal_bridge_mm = f64::INFINITY;
    assert!(built.check().unwrap_err().contains("thermal_bridge_width"));
    let mut built = defaults();
    built.islands = ZoneIslands::BelowArea { area_mm2: f64::NAN };
    assert!(built.check().unwrap_err().contains("island_min_area"));
}

// ─── A raised minimum width still gets spokes ──────────────────────────────────

#[test]
fn a_raised_minimum_width_still_gets_spokes() {
    let settings = ZoneSettings::from_args(&json!({"min_width": 0.8})).unwrap();
    assert_eq!(settings.thermal_bridge_mm, 0.8);
    let settings = ZoneSettings::from_args(&json!({"min_width": 0.3})).unwrap();
    assert_eq!(settings.thermal_bridge_mm, 0.5);
}

// ─── island_min_area is read or refused ─────────────────────────────────────────

#[test]
fn island_min_area_is_read_or_refused() {
    let error = ZoneSettings::from_args(&json!({"island_min_area": 5.0})).unwrap_err();
    assert!(error.contains("island_min_area"), "{error}");
    let error =
        ZoneSettings::from_args(&json!({"island_removal": "never", "island_min_area": 5.0}))
            .unwrap_err();
    assert!(error.contains("island_min_area"), "{error}");
    let error = ZoneSettings::from_args(&json!({"island_removal": "below_area"})).unwrap_err();
    assert!(error.contains("island_min_area"), "{error}");
    let read = ZoneSettings::from_args(&json!({
        "island_removal": "below_area", "island_min_area": 5.0
    }))
    .unwrap();
    assert_eq!(read.islands, ZoneIslands::BelowArea { area_mm2: 5.0 });
}

// ─── What add_zone writes is what the engine is told ───────────────────────────

#[test]
fn what_add_zone_writes_is_what_the_engine_is_told() {
    for settings in [defaults(), odd_settings()] {
        let text = zone_sexp(&by_name("GND"), "F.Cu", &SQUARE, "", &settings);
        let zone = konnect_sexp::parse_sexp(&text).unwrap();
        let read = fill_settings(&zone).expect("the engine's model covers add_zone's zone");
        assert_eq!(read.clearance_mm, settings.clearance_mm);
        assert_eq!(read.min_width_mm, settings.min_width_mm);
        assert_eq!(read.thermal_gap_mm, settings.thermal_gap_mm);
        assert_eq!(read.thermal_bridge_mm, settings.thermal_bridge_mm);
        assert_eq!(read.priority, settings.priority);
        let islands = match settings.islands {
            ZoneIslands::Always => IslandRule::Remove,
            ZoneIslands::Never => IslandRule::Keep,
            ZoneIslands::BelowArea { area_mm2 } => IslandRule::RemoveBelow { area_mm2 },
        };
        assert_eq!(read.islands, islands);
    }
}

// ─── A name with a quote or a backslash comes back out whole ───────────────────

#[test]
fn a_name_with_a_quote_or_a_backslash_comes_back_out_whole() {
    let odd = "A\"B\\C";
    let text = zone_sexp(&by_name(odd), "F.Cu", &SQUARE, "", &defaults());
    let zone = konnect_sexp::parse_sexp(&text).expect("the escaped text parses");
    assert_eq!(
        zone.find("net").and_then(konnect_sexp::net::net_name),
        Some(odd)
    );
    for plain in ["GND", "/Sheet/+3V3", "Net-(U1-Pad3)"] {
        assert_eq!(
            mask_uuids(&zone_sexp(
                &by_name(plain),
                "F.Cu",
                &SQUARE,
                "",
                &defaults()
            )),
            pre_zr4_zone_text(
                &format!("(net \"{plain}\")"),
                "(layers \"F.Cu\")",
                0.2,
                0.2,
                &SQUARE,
                "",
                0,
                "",
                "U"
            ),
            "{plain} is written as before"
        );
    }
}

// ─── The result says what was written ──────────────────────────────────────────

fn assert_echo(body: &Value) {
    assert_eq!(body["thermal_gap"], json!(0.3), "{body}");
    assert_eq!(body["thermal_bridge_width"], json!(0.4), "{body}");
    assert_eq!(body["island_removal"], json!("below_area"), "{body}");
    assert_eq!(body["island_min_area"], json!(2.5), "{body}");
}

const ECHOED: &str = r#"{"thermal_gap": 0.3, "thermal_bridge_width": 0.4, "island_removal": "below_area", "island_min_area": 2.5}"#;

#[tokio::test]
async fn the_result_says_what_was_written_on_the_file_path() {
    let (result, _) = add_zone_on_file(KICAD_10, serde_json::from_str(ECHOED).unwrap()).await;
    assert!(!result.is_error, "{}", text_of(&result));
    let body = body_of(&result);
    assert_eq!(body["source"], json!("file"));
    assert_echo(&body);
    // The default call echoes the defaults, with no island area.
    let (result, _) = add_zone_on_file(KICAD_10, json!({})).await;
    let body = body_of(&result);
    assert_eq!(body["thermal_gap"], json!(0.5));
    assert_eq!(body["thermal_bridge_width"], json!(0.5));
    assert_eq!(body["island_removal"], json!("always"));
    assert_eq!(body["island_min_area"], Value::Null);
}

#[tokio::test]
async fn the_result_says_what_was_written_on_the_mock_ipc_path() {
    use konnect_ipc::builders::pack_any;
    use prost::Message;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.kicad_pcb");
    std::fs::write(&path, KICAD_10).unwrap();
    let sent: Arc<Mutex<Vec<kiapi::board::types::Zone>>> = Arc::new(Mutex::new(Vec::new()));
    let record = sent.clone();
    let server = spawn_kicad_holding_board(&path, move |command| {
        if command.type_url.ends_with("GetNets") {
            return Some(pack_any(
                &kiapi::board::commands::NetsResponse {
                    nets: vec![kiapi::board::types::Net {
                        code: Some(kiapi::board::types::NetCode { value: 3 }),
                        name: "GND".to_string(),
                    }],
                },
                "kiapi.board.commands.NetsResponse",
            ));
        }
        if command.type_url.ends_with("CreateItems") {
            let request =
                kiapi::common::commands::CreateItems::decode(command.value.as_slice()).unwrap();
            let mut created = Vec::new();
            for item in request.items {
                let mut zone = kiapi::board::types::Zone::decode(item.value.as_slice()).unwrap();
                record.lock().unwrap().push(zone.clone());
                zone.id = Some(kiapi::common::types::Kiid {
                    value: "zone-1".to_string(),
                });
                created.push(kiapi::common::commands::ItemCreationResult {
                    status: Some(kiapi::common::commands::ItemStatus {
                        code: kiapi::common::commands::ItemStatusCode::IscOk as i32,
                        error_message: String::new(),
                    }),
                    item: Some(pack_any(&zone, "kiapi.board.types.Zone")),
                });
            }
            return Some(pack_any(
                &kiapi::common::commands::CreateItemsResponse {
                    header: None,
                    status: kiapi::common::types::ItemRequestStatus::IrsOk as i32,
                    created_items: created,
                },
                "kiapi.common.commands.CreateItemsResponse",
            ));
        }
        None
    });
    let ctx = ctx_talking_to(server.address().to_string());
    let mut args = json!({
        "board": path.to_str().unwrap(), "net_name": "GND", "layer": "B.Cu",
        "points": [ {"x": 0.0, "y": 0.0}, {"x": 10.0, "y": 0.0}, {"x": 10.0, "y": 10.0} ]
    });
    for (key, value) in serde_json::from_str::<Value>(ECHOED)
        .unwrap()
        .as_object()
        .unwrap()
    {
        args[key] = value.clone();
    }
    let result = add_zone_impl(&args, &ctx).await.unwrap();
    assert!(!result.is_error, "{}", text_of(&result));
    let body = body_of(&result);
    assert_eq!(body["source"], json!("ipc"), "{body}");
    assert_echo(&body);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), KICAD_10);
    // And what reached the mock is what the result says.
    let zones = sent.lock().unwrap();
    assert_eq!(zones.len(), 1);
    let copper = match zones[0].settings.clone().expect("settings") {
        kiapi::board::types::zone::Settings::CopperSettings(s) => s,
        other => panic!("{other:?}"),
    };
    let spokes = copper.connection.unwrap().thermal_spokes.unwrap();
    assert_eq!(spokes.gap.unwrap().value_nm, 300_000);
    assert_eq!(spokes.width.unwrap().value_nm, 400_000);
    assert_eq!(
        copper.island_mode,
        kiapi::board::types::IslandRemovalMode::IrmArea as i32
    );
    assert_eq!(copper.min_island_area, 2_500_000_000_000);
}
