//! Creation PR 2: no wiring tool joins a net it was not asked to join, and each
//! reports the nets it did join (`nets_joined`). #899 (stubs) and #900
//! (`add_wire` T-dots) sit beside #887's routes, which `route_obstacle_tests`
//! covers, on the same fixture.
//!
//! The fast tests read the file. The ignored ones ask KiCad, through
//! `compare_netlist`, whether the result connects what was meant:
//! `cargo test -p konnect-core --lib route_safety -- --ignored`

use super::ServerConfig;
use crate::mcp::handler::McpHandler;
use konnect_sexp::schematic::{extract_junctions, read_schematic};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Where R53's wire ends mid-span on R51's, with no junction: two nets to KiCad.
const DOTLESS_TEE: (f64, f64) = (190.5, 156.21);

fn fixture() -> (tempfile::TempDir, PathBuf) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "route_obstacles_kicad10.kicad_sch",
        "route_obstacles_sub_kicad10.kicad_sch",
    ] {
        std::fs::copy(src.join(name), dir.path().join(name)).unwrap();
    }
    let path = dir.path().join("route_obstacles_kicad10.kicad_sch");
    (dir, path)
}

/// A second copy of the fixture as it stood, to compare a result against.
fn snapshot_of(path: &Path) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    for entry in std::fs::read_dir(path.parent().unwrap()).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
    }
    let copy = dir.path().join(path.file_name().unwrap());
    (dir, copy)
}

async fn call(path: &Path, name: &str, mut arguments: Value) -> (bool, Value) {
    let handler = McpHandler::new(ServerConfig {
        eager_toolsets: true,
        kicad_cli: std::env::var("KICAD_CLI").unwrap_or_else(|_| "kicad-cli".to_owned()),
        ..ServerConfig::default()
    })
    .await
    .expect("handler builds");
    arguments["schematic"] = json!(path.display().to_string());
    let response = handler
        .handle_message(json!({
            "jsonrpc": "2.0", "id": 902, "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }))
        .await
        .expect("tools/call receives a response");
    let result = response.result.expect("successful JSON-RPC response");
    let body = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    (result["isError"] == json!(true), body)
}

fn junctions(path: &Path) -> Vec<(f64, f64)> {
    extract_junctions(&read_schematic(path).unwrap().1)
}

/// Each entry of `nets_joined` as the list of net names it holds.
fn joined(body: &Value) -> Vec<Vec<String>> {
    body["nets_joined"]
        .as_array()
        .unwrap_or_else(|| panic!("nets_joined is not a list: {body}"))
        .iter()
        .map(|entry| {
            entry["nets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_str().unwrap().to_owned())
                .collect()
        })
        .collect()
}

// ─── #900: only the T's the new wire forms ───────────────────────────────────

#[tokio::test]
async fn add_wire_far_from_a_dotless_tee_leaves_it_undotted() {
    let (_dir, path) = fixture();
    let (is_error, body) = call(
        &path,
        "add_wire",
        json!({ "x1": 20.32, "y1": 20.32, "x2": 30.48, "y2": 20.32 }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert!(
        !junctions(&path).contains(&DOTLESS_TEE),
        "R53.2 stays off R51's net"
    );
    assert_eq!(
        joined(&body),
        Vec::<Vec<String>>::new(),
        "nothing was joined"
    );
}

#[tokio::test]
async fn batch_add_wire_far_from_a_dotless_tee_leaves_it_undotted() {
    let (_dir, path) = fixture();
    let wires = json!([
        { "x1": 20.32, "y1": 20.32, "x2": 30.48, "y2": 20.32 },
        { "x1": 20.32, "y1": 25.4, "x2": 30.48, "y2": 25.4 }
    ]);
    let (is_error, body) = call(&path, "batch_add_wire", json!({ "wires": wires })).await;
    assert!(!is_error, "{body}");
    assert_eq!(body["added_wires"], 2);
    assert!(!junctions(&path).contains(&DOTLESS_TEE));
    assert_eq!(joined(&body), Vec::<Vec<String>>::new());
}

#[tokio::test]
async fn add_wire_still_dots_the_tee_it_forms_and_reports_the_join() {
    let (_dir, path) = fixture();
    // R54.2 down onto R51's wire mid-span: its own T, and a real join.
    let (is_error, body) = call(
        &path,
        "add_wire",
        json!({ "x1": 184.15, "y1": 143.51, "x2": 184.15, "y2": 156.21 }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(junctions(&path), [(184.15, 156.21)]);
    let joins = joined(&body);
    assert_eq!(joins.len(), 1, "{body}");
    assert_eq!(joins[0].len(), 2, "R54.2's net and R51.2/R52.1's: {body}");
}

#[tokio::test]
async fn add_wire_over_another_pin_is_the_callers_ask_and_is_reported() {
    let (_dir, path) = fixture();
    // An explicit wire along y = 105.41 from R1.2 across R3.1: coordinates are
    // the caller's, so it is drawn, and the two nets it joined are named.
    let (is_error, body) = call(
        &path,
        "add_wire",
        json!({ "x1": 101.6, "y1": 105.41, "x2": 127.0, "y2": 105.41 }),
    )
    .await;
    assert!(!is_error, "{body}");
    let joins = joined(&body);
    assert_eq!(joins.len(), 1, "{body}");
    assert_eq!(joins[0].len(), 2, "R1.2's net and R3.1's: {body}");
}

// ─── #899: a stub joins nothing but its own pin ──────────────────────────────

/// R54.2 is at (184.15, 143.51). Straight down, 12.7 mm, ends mid-span on R51's
/// wire (net R51.2/R52.1). Right crosses R53.2 at (190.5, 143.51), up crosses
/// R54.1 itself, left is clear.
const STUB: f64 = 12.7;

#[tokio::test]
async fn connect_to_net_refuses_a_named_direction_that_joins_another_net() {
    let (_dir, path) = fixture();
    let before = std::fs::read_to_string(&path).unwrap();
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "reference": "R54", "pin_number": "2", "net": "N",
                "direction": "down", "stub_length": STUB }),
    )
    .await;
    assert!(is_error, "{body}");
    assert_eq!(body["error"]["kind"], "plan_blocked", "{body}");
    let reasons = body["error"]["reasons"].as_array().unwrap();
    assert_eq!(reasons.len(), 1, "a named direction is tried alone: {body}");
    assert!(reasons[0]
        .as_str()
        .unwrap()
        .starts_with("the down stub would join"));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before,
        "nothing written"
    );
}

#[tokio::test]
async fn connect_to_net_auto_takes_the_first_direction_that_joins_nothing() {
    let (_dir, path) = fixture();
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "reference": "R54", "pin_number": "2", "net": "N", "stub_length": STUB }),
    )
    .await;
    assert!(!is_error, "{body}");
    // down, right and up each join something; left does not.
    assert_eq!(body["direction"], "left", "{body}");
    let wire = &body["wire"];
    assert_eq!(wire["y2"], 143.51, "{body}");
    assert!(
        (wire["x2"].as_f64().unwrap() - 171.45).abs() < 1e-6,
        "{body}"
    );
    assert!(!junctions(&path).contains(&DOTLESS_TEE));
    assert_eq!(
        joined(&body),
        Vec::<Vec<String>>::new(),
        "N had nothing else on it"
    );
}

#[tokio::test]
async fn connect_to_net_refuses_a_stub_through_its_own_components_other_pin() {
    let (_dir, path) = fixture();
    let before = std::fs::read_to_string(&path).unwrap();
    // Up from R54.2 passes R54.1: two pins of one part on one net.
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "reference": "R54", "pin_number": "2", "net": "N", "direction": "up",
                "stub_length": STUB }),
    )
    .await;
    assert!(is_error, "{body}");
    assert_eq!(body["error"]["kind"], "plan_blocked");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[tokio::test]
async fn connect_to_net_auto_refuses_with_one_reason_per_direction_when_all_join_something() {
    let (_dir, path) = fixture();
    // A label of another net at the left stub's end closes the last direction.
    let (is_error, body) = call(
        &path,
        "add_schematic_net_label",
        json!({ "net": "BLOCKER", "x": 171.45, "y": 143.51 }),
    )
    .await;
    assert!(!is_error, "{body}");
    let before = std::fs::read_to_string(&path).unwrap();
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "reference": "R54", "pin_number": "2", "net": "N", "stub_length": STUB }),
    )
    .await;
    assert!(is_error, "{body}");
    assert_eq!(body["error"]["kind"], "plan_blocked", "{body}");
    let reasons: Vec<&str> = body["error"]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    let directions: Vec<&str> = reasons
        .iter()
        .map(|r| r.split(' ').nth(1).unwrap())
        .collect();
    // Down first (R54.2's outward direction), then the others in a fixed order.
    assert_eq!(directions, ["down", "right", "up", "left"], "{reasons:?}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before,
        "nothing written"
    );
}

#[tokio::test]
async fn a_stub_that_reaches_the_net_it_names_is_not_a_foreign_join() {
    let (_dir, path) = fixture();
    // OTHER is the net on R13.1; naming R12.1's stub OTHER and reaching R13.1
    // is what the label does anyway. R13.1 is at (114.3, 54.61).
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "pin_x": 101.6, "pin_y": 54.61, "net": "OTHER",
                "direction": "right", "stub_length": 12.7 }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["direction"], "right");
}

#[tokio::test]
async fn batch_connect_to_net_skips_only_the_blocked_stub() {
    let (_dir, path) = fixture();
    let (is_error, body) = call(
        &path,
        "batch_connect_to_net",
        json!({ "net": "N", "stub_length": STUB, "direction": "down",
                "pins": [
                    { "reference": "R54", "pin_number": "2" },
                    { "reference": "R22", "pin_number": "2" } ] }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["added_count"], 1, "{body}");
    let errors = body["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{body}");
    assert!(errors[0]
        .as_str()
        .unwrap()
        .starts_with("R54.2: the down stub would join"));
    assert!(body["nets_joined"].is_array());
}

// ─── Reporting on routed connections ─────────────────────────────────────────

#[tokio::test]
async fn connect_pins_reports_the_two_nets_it_joined_and_not_the_pin_it_avoided() {
    let (_dir, path) = fixture();
    let (is_error, body) = call(
        &path,
        "connect_pins",
        json!({ "ref1": "R21", "pin1": "2", "ref2": "R22", "pin2": "1" }),
    )
    .await;
    assert!(!is_error, "{body}");
    let joins = joined(&body);
    assert_eq!(joins.len(), 1, "{body}");
    assert_eq!(joins[0].len(), 2, "{body}");
    assert!(
        joins[0].iter().all(|n| n.starts_with("unnamed net with")),
        "{body}"
    );

    // Around R3.1: the avoided pin is not in the report.
    let (_dir, path) = fixture();
    let (is_error, body) = call(
        &path,
        "connect_pins",
        json!({ "ref1": "R1", "pin1": "2", "ref2": "R2", "pin2": "1" }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert!(!body["nets_joined"].to_string().contains("R3"), "{body}");
}

#[tokio::test]
async fn batch_connect_pins_and_add_schematic_connection_report_nets_joined() {
    let (_dir, path) = fixture();
    let (_, body) = call(
        &path,
        "batch_connect_pins",
        json!({ "connections": [
            { "ref1": "R21", "pin1": "2", "ref2": "R22", "pin2": "1" } ] }),
    )
    .await;
    assert_eq!(joined(&body).len(), 1, "{body}");
    let (_dir, path) = fixture();
    let (_, body) = call(
        &path,
        "add_schematic_connection",
        json!({ "x1": 101.6, "y1": 156.21, "x2": 127.0, "y2": 158.75 }),
    )
    .await;
    assert_eq!(joined(&body).len(), 1, "{body}");
}

// ─── Real KiCad: compare_netlist proves no unintended net changed ────────────

/// Ask KiCad whether `path` connects exactly as `reference` did, through
/// `compare_netlist` on both.
async fn assert_same_circuit(path: &Path, reference: &Path) {
    let (is_error, body) = call(
        path,
        "compare_netlist",
        json!({ "reference_schematic": reference.display().to_string() }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["result"], "pass", "{body}");
}

async fn assert_intent(path: &Path, intended: Value) {
    let (is_error, body) = call(
        path,
        "compare_netlist",
        json!({ "intended": intended, "scope": "complete" }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["result"], "pass", "{body}");
}

#[tokio::test]
#[ignore = "needs kicad-cli"]
async fn kicad_agrees_no_tool_changed_a_net_it_was_not_asked_to() {
    // add_wire far away: the dot-less tee is still two nets.
    let (_d, path) = fixture();
    let (_before, reference) = snapshot_of(&path);
    call(
        &path,
        "add_wire",
        json!({ "x1": 20.32, "y1": 20.32, "x2": 30.48, "y2": 20.32 }),
    )
    .await;
    assert_same_circuit(&path, &reference).await;

    // A refused route writes nothing and so changes nothing.
    let (_d, path) = fixture();
    let (_before, reference) = snapshot_of(&path);
    let (is_error, _) = call(
        &path,
        "connect_pins",
        json!({ "ref1": "R11", "pin1": "2", "ref2": "R12", "pin2": "1" }),
    )
    .await;
    assert!(is_error);
    assert_same_circuit(&path, &reference).await;

    // Around R3.1: R1.2 and R2.1 are one net, and R3.1 is its own.
    let (_d, path) = fixture();
    call(
        &path,
        "connect_pins",
        json!({ "ref1": "R1", "pin1": "2", "ref2": "R2", "pin2": "1" }),
    )
    .await;
    assert_intent(
        &path,
        json!({ "ROUTE": ["R1.2", "R2.1"], "AVOIDED": ["R3.1"],
                "OTHER": ["R13.1"], "TEE": ["R51.2", "R52.1"], "R53": ["R53.2"] }),
    )
    .await;

    // Around the sheet pin: R41.2 and R42.1 are one net, apart from /SUB/IN.
    let (_d, path) = fixture();
    call(
        &path,
        "connect_pins",
        json!({ "ref1": "R41", "pin1": "2", "ref2": "R42", "pin2": "1" }),
    )
    .await;
    assert_intent(
        &path,
        json!({ "ROUTE": ["R41.2", "R42.1"], "OTHER": ["R13.1"],
                "TEE": ["R51.2", "R52.1"], "R53": ["R53.2"] }),
    )
    .await;

    // The stub, auto direction: N holds R54.2 alone and the tee is untouched.
    let (_d, path) = fixture();
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "reference": "R54", "pin_number": "2", "net": "N", "stub_length": STUB }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_intent(
        &path,
        json!({ "N": ["R54.2"], "TEE": ["R51.2", "R52.1"], "R53": ["R53.2"],
                "OTHER": ["R13.1"] }),
    )
    .await;
}
