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
    // OTHER is the net on R13.1. R11.2 is at (101.6, 54.61) and R13.1 at
    // (114.3, 54.61), so this stub reaches R13.1 by wire: the merge here is the
    // wire's, and naming the stub OTHER is what the label would do anyway.
    // The merge made by a label's name alone is the next test.
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "pin_x": 101.6, "pin_y": 54.61, "net": "OTHER",
                "direction": "right", "stub_length": 12.7 }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["direction"], "right");
    // The merge is reported, with the named net itself.
    assert_eq!(
        joined(&body),
        [["unnamed net with R11 pin 2", "OTHER"]],
        "{body}"
    );
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

// ─── Review of PR 20 ─────────────────────────────────────────────────────────

/// The fixture with R60 placed so its pin 1 sits mid-span on R51's wire, at
/// (196.85, 156.21), with no junction: KiCad reads the pin as unconnected.
/// `add_schematic_component` dots such a pin, so the dot is taken out again.
async fn midspan_pin_scene() -> (tempfile::TempDir, PathBuf) {
    let (dir, path) = fixture();
    let text = std::fs::read_to_string(&path).unwrap();
    // R53's block, moved: the same KiCad-authored symbol, another reference,
    // place and set of ids.
    let reference = text.find("(property \"Reference\" \"R53\"").expect("R53");
    let start = text[..reference].rfind("(symbol").expect("its symbol");
    let mut depth = 0usize;
    let mut end = start;
    for (i, c) in text[start..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = start + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    let mut block = text[start..end]
        .replace("R53", "R60")
        .replace("(at 190.5 139.7 0)", "(at 196.85 160.02 0)");
    let mut n = 0;
    while let Some(at) = block.find("(uuid \"") {
        let close = block[at..].find("\")").unwrap() + at;
        block.replace_range(
            at..=close + 1,
            &format!("(uuid_new \"00000000-0000-0000-0000-{n:012}\")"),
        );
        n += 1;
    }
    let block = block.replace("(uuid_new", "(uuid");
    let at = text.rfind(')').unwrap();
    std::fs::write(&path, format!("{}\t{block}\n{}", &text[..at], &text[at..])).unwrap();
    assert!(
        !junctions(&path).contains(&(196.85, 156.21)),
        "the scene needs a pin on a wire with no dot"
    );
    (dir, path)
}

#[tokio::test]
async fn a_pin_mid_span_on_a_foreign_wire_does_not_get_that_net_by_connect_pins() {
    let (_dir, path) = midspan_pin_scene().await;
    let before = std::fs::read_to_string(&path).unwrap();
    let (is_error, body) = call(
        &path,
        "connect_pins",
        json!({ "ref1": "R60", "pin1": "1", "ref2": "R21", "pin2": "2" }),
    )
    .await;
    assert!(is_error, "{body}");
    assert_eq!(body["error"]["kind"], "plan_blocked", "{body}");
    assert!(
        body["error"]["reasons"].to_string().contains("R51 pin 2"),
        "{body}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before,
        "nothing written"
    );
}

#[tokio::test]
async fn a_pin_mid_span_on_a_foreign_wire_does_not_get_that_net_by_a_stub() {
    let (_dir, path) = midspan_pin_scene().await;
    let before = std::fs::read_to_string(&path).unwrap();
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "reference": "R60", "pin_number": "1", "net": "SIG" }),
    )
    .await;
    assert!(is_error, "{body}");
    assert_eq!(body["error"]["kind"], "plan_blocked", "{body}");
    assert_eq!(
        body["error"]["reasons"].as_array().unwrap().len(),
        4,
        "every direction: {body}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

    // The coordinates of a placed pin's end are that pin, so the same refusal
    // follows, with a stub or without one.
    for extra in [json!({}), json!({ "stub_length": 0 })] {
        let mut args = json!({ "pin_x": 196.85, "pin_y": 156.21, "net": "SIG" });
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let (is_error, body) = call(&path, "connect_to_net", args.clone()).await;
        assert!(is_error, "{args}: {body}");
        assert_eq!(body["error"]["kind"], "plan_blocked", "{body}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "{args}");
    }

    // Any other coordinate taps the wire it lies on: that is the ask. (180.34,
    // 156.21) is on R51's wire, away from every pin.
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "pin_x": 180.34, "pin_y": 156.21, "net": "SIG", "stub_length": 0 }),
    )
    .await;
    assert!(!is_error, "{body}");
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "pin_x": 180.34, "pin_y": 156.21, "net": "SIG", "direction": "down" }),
    )
    .await;
    assert!(!is_error, "{body}");
}

#[tokio::test]
async fn a_label_on_a_pin_mid_span_is_refused_with_or_without_a_stub() {
    let (_dir, path) = midspan_pin_scene().await;
    let before = std::fs::read_to_string(&path).unwrap();
    // connect_to_net, stub_length 0.
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "reference": "R60", "pin_number": "1", "net": "EN", "stub_length": 0 }),
    )
    .await;
    assert!(is_error, "{body}");
    assert_eq!(body["error"]["kind"], "plan_blocked", "{body}");
    assert!(body["error"]["reasons"][0]
        .as_str()
        .unwrap()
        .starts_with("the label would join"));
    // batch_connect_to_net's default, a label spliced onto the pin: skipped and reported.
    let (is_error, body) = call(
        &path,
        "batch_connect_to_net",
        json!({ "net": "EN", "pins": [{ "reference": "R60", "pin_number": "1" }] }),
    )
    .await;
    assert!(!is_error || body["added_count"] == 0, "{body}");
    assert_eq!(body["added_count"], 0, "{body}");
    assert!(
        body["errors"][0]
            .as_str()
            .unwrap()
            .starts_with("R60.1: the label would join"),
        "{body}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before,
        "nothing written"
    );
}

#[tokio::test]
async fn the_default_batch_reports_nets_joined_even_for_a_label_only_splice() {
    let (_dir, path) = fixture();
    let (is_error, body) = call(
        &path,
        "batch_connect_to_net",
        json!({ "net": "N", "pins": [
            { "reference": "R21", "pin_number": "2" },
            { "reference": "R22", "pin_number": "1" } ] }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["added_count"], 2, "{body}");
    // Two nets that had nothing in common are now one, by name.
    assert_eq!(
        joined(&body),
        [["unnamed net with R21 pin 2", "unnamed net with R22 pin 1"]],
        "{body}"
    );
    // A label that joins nothing reports an empty list, not a missing key.
    let (_dir, path) = fixture();
    let (_, body) = call(
        &path,
        "batch_connect_to_net",
        json!({ "net": "N", "pins": [{ "reference": "R21", "pin_number": "2" }] }),
    )
    .await;
    assert_eq!(joined(&body), Vec::<Vec<String>>::new(), "{body}");
}

#[tokio::test]
async fn a_stub_to_a_net_that_has_the_requested_name_as_a_losing_alias_is_not_refused() {
    let (_dir, path) = fixture();
    // A wire at R54.2's left stub end, carrying two local labels. The winner
    // is the alphabetically first, AAA; the stub asks for SIG.
    for (net, x, y) in [("AAA", 171.45, 139.7), ("SIG", 171.45, 137.16)] {
        let (e, b) = call(
            &path,
            "add_schematic_net_label",
            json!({ "net": net, "x": x, "y": y }),
        )
        .await;
        assert!(!e, "{b}");
    }
    let (e, b) = call(
        &path,
        "add_wire",
        json!({ "x1": 171.45, "y1": 143.51, "x2": 171.45, "y2": 135.89 }),
    )
    .await;
    assert!(!e, "{b}");
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "reference": "R54", "pin_number": "2", "net": "SIG",
                "direction": "left", "stub_length": STUB }),
    )
    .await;
    assert!(
        !is_error,
        "the SIG label joins that net by name anyway: {body}"
    );
    assert_eq!(body["direction"], "left");
}

/// R54 has a library symbol that does not exist, so the sheet's pins cannot all be read.
fn break_one_symbol(path: &Path) {
    let text = std::fs::read_to_string(path).unwrap();
    let reference = text.find("(property \"Reference\" \"R54\"").expect("R54");
    let lib_id = text[..reference]
        .rfind("(lib_id \"Device:R\")")
        .expect("its lib_id");
    let broken = format!(
        "{}(lib_id \"Device:NoSuchSymbol\"){}",
        &text[..lib_id],
        &text[lib_id + "(lib_id \"Device:R\")".len()..]
    );
    std::fs::write(path, broken).unwrap();
}

#[tokio::test]
async fn an_unreadable_symbol_makes_a_stub_unchecked_and_says_so() {
    let (_dir, path) = fixture();
    break_one_symbol(&path);
    let before = std::fs::read_to_string(&path).unwrap();
    // connect_to_net: refused, as "cannot check", not as a join.
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "reference": "R21", "pin_number": "1", "net": "N" }),
    )
    .await;
    assert!(is_error, "{body}");
    assert_eq!(body["error"]["kind"], "plan_blocked", "{body}");
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("cannot check what the wire would join"),
        "{message}"
    );
    assert!(!message.contains("joins another net"), "{message}");
    // batch_connect_to_net with a stub: every pin skipped and explained.
    let (is_error, body) = call(
        &path,
        "batch_connect_to_net",
        json!({ "net": "N", "stub_length": 2.54, "pins": [{ "reference": "R21", "pin_number": "1" }] }),
    )
    .await;
    assert!(is_error, "every pin skipped is a failed call: {body}");
    assert_eq!(body["added_count"], 0, "{body}");
    assert!(
        body["errors"][0]
            .as_str()
            .unwrap()
            .contains("cannot check what the stub would join"),
        "{body}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before,
        "nothing written"
    );
}

#[tokio::test]
async fn with_an_unreadable_symbol_a_plain_label_still_goes_on_and_nets_joined_is_null() {
    let (_dir, path) = fixture();
    break_one_symbol(&path);
    // The label-only default does not need pin geometry to be judged.
    let (is_error, body) = call(
        &path,
        "batch_connect_to_net",
        json!({ "net": "N", "pins": [{ "reference": "R21", "pin_number": "1" }] }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["added_count"], 1, "{body}");
    assert!(
        body.get("nets_joined") == Some(&Value::Null),
        "cannot tell, and says so: {body}"
    );
    // add_wire and batch_add_wire say null too.
    let (_, body) = call(
        &path,
        "add_wire",
        json!({ "x1": 20.32, "y1": 20.32, "x2": 30.48, "y2": 20.32 }),
    )
    .await;
    assert_eq!(body.get("nets_joined"), Some(&Value::Null), "{body}");
    let (_, body) = call(
        &path,
        "batch_add_wire",
        json!({ "wires": [{ "x1": 20.32, "y1": 25.4, "x2": 30.48, "y2": 25.4 }] }),
    )
    .await;
    assert_eq!(body.get("nets_joined"), Some(&Value::Null), "{body}");
}

#[tokio::test]
async fn a_negative_stub_length_is_refused_before_anything_is_drawn() {
    let (_dir, path) = fixture();
    let before = std::fs::read_to_string(&path).unwrap();
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "reference": "R54", "pin_number": "2", "net": "N",
                "direction": "up", "stub_length": -7.62 }),
    )
    .await;
    assert!(is_error, "{body}");
    assert_eq!(body["error"]["kind"], "invalid_argument", "{body}");
    assert_eq!(body["error"]["field"], "stub_length");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

    // Called directly, past the schema: the handler refuses it too.
    let ctx = crate::tools::ToolContext::new(
        ServerConfig::default(),
        std::sync::Arc::new(crate::router::ToolRouter::new()),
    );
    let result = crate::tools::sch_wiring::handle_connect_to_net(
        &json!({ "schematic": path.display().to_string(), "reference": "R54",
                 "pin_number": "2", "net": "N", "direction": "up", "stub_length": -7.62 }),
        &ctx,
    )
    .await
    .unwrap();
    assert!(result.is_error);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

/// A scene with a pin on a wire with no dot and a symbol KiCad cannot read: the
/// label checks have to work without pin geometry.
async fn midspan_pin_on_unreadable_sheet() -> (tempfile::TempDir, PathBuf) {
    let (dir, path) = midspan_pin_scene().await;
    break_one_symbol(&path);
    (dir, path)
}

#[tokio::test]
async fn a_label_that_names_a_net_by_name_alone_is_reported_as_a_merge() {
    // R21.2 is on an unnamed net and OTHER is R13.1's: a label OTHER on R21.2
    // joins them with no wire anywhere near.
    for tool in ["connect_to_net", "batch_connect_to_net"] {
        let (_dir, path) = fixture();
        let args = if tool == "connect_to_net" {
            json!({ "reference": "R21", "pin_number": "2", "net": "OTHER", "stub_length": 0 })
        } else {
            json!({ "net": "OTHER", "pins": [{ "reference": "R21", "pin_number": "2" }] })
        };
        let (is_error, body) = call(&path, tool, args).await;
        assert!(!is_error, "{tool}: {body}");
        assert_eq!(
            joined(&body),
            [["unnamed net with R21 pin 2", "OTHER"]],
            "{tool}: {body}"
        );
    }
}

#[tokio::test]
async fn ordinary_pins_in_the_default_batch_run_no_full_check() {
    let (_dir, path) = fixture();
    let pins: Vec<Value> = (21..=22)
        .flat_map(|n| {
            ["1", "2"].map(|pin| json!({ "reference": format!("R{n}"), "pin_number": pin }))
        })
        .collect();
    let before = crate::tools::sch_wiring::FULL_CHECKS.with(|c| c.get());
    let (is_error, body) = call(
        &path,
        "batch_connect_to_net",
        json!({ "net": "N", "pins": pins }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["added_count"], 4, "{body}");
    let after = crate::tools::sch_wiring::FULL_CHECKS.with(|c| c.get());
    assert_eq!(
        after, before,
        "no pin sits under a wire, so none needs the full check"
    );

    // A pin under a wire does get it, and is still refused.
    let (_dir, path) = midspan_pin_scene().await;
    let before = crate::tools::sch_wiring::FULL_CHECKS.with(|c| c.get());
    let (_, body) = call(
        &path,
        "batch_connect_to_net",
        json!({ "net": "EN", "pins": [{ "reference": "R60", "pin_number": "1" }] }),
    )
    .await;
    assert_eq!(body["added_count"], 0, "{body}");
    assert_eq!(
        crate::tools::sch_wiring::FULL_CHECKS.with(|c| c.get()),
        before + 1
    );
}

#[tokio::test]
async fn on_an_unreadable_sheet_a_label_under_a_foreign_wire_is_still_refused() {
    let (_dir, path) = midspan_pin_on_unreadable_sheet().await;
    let before = std::fs::read_to_string(&path).unwrap();
    let (is_error, body) = call(
        &path,
        "batch_connect_to_net",
        json!({ "net": "EN", "pins": [{ "reference": "R60", "pin_number": "1" }] }),
    )
    .await;
    assert!(is_error, "{body}");
    assert_eq!(body["added_count"], 0, "{body}");
    assert!(
        body["errors"][0]
            .as_str()
            .unwrap()
            .starts_with("R60.1: the label would join the wire"),
        "{body}"
    );
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "reference": "R60", "pin_number": "1", "net": "EN", "stub_length": 0 }),
    )
    .await;
    assert!(is_error, "{body}");
    assert_eq!(body["error"]["kind"], "plan_blocked", "{body}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before,
        "nothing written"
    );
}

#[tokio::test]
async fn on_an_unreadable_sheet_a_legal_label_is_not_refused() {
    let (_dir, path) = midspan_pin_on_unreadable_sheet().await;
    // (a) The wire under R60.1 already carries the name: the label joins nothing new.
    let (e, b) = call(
        &path,
        "add_schematic_net_label",
        json!({ "net": "EN", "x": 180.34, "y": 156.21 }),
    )
    .await;
    assert!(!e, "{b}");
    let (is_error, body) = call(
        &path,
        "batch_connect_to_net",
        json!({ "net": "EN", "pins": [{ "reference": "R60", "pin_number": "1" }] }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["added_count"], 1, "{body}");
    assert_eq!(
        body.get("nets_joined"),
        Some(&Value::Null),
        "cannot tell on this sheet: {body}"
    );
    // (b) Run again: the label is there, and the call is an idempotent no-op, not an error.
    let (is_error, body) = call(
        &path,
        "batch_connect_to_net",
        json!({ "net": "EN", "pins": [{ "reference": "R60", "pin_number": "1" }] }),
    )
    .await;
    assert!(!is_error, "{body}");
    // (c) A coordinate taps the wire it lies on, with no stub.
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "pin_x": 190.5, "pin_y": 156.21, "net": "TAP", "stub_length": 0 }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body.get("nets_joined"), Some(&Value::Null), "{body}");
}

#[tokio::test]
async fn pins_that_cannot_be_read_do_not_stop_a_plain_label_connect_to_net() {
    let (_dir, path) = fixture();
    break_one_symbol(&path);
    let (is_error, body) = call(
        &path,
        "connect_to_net",
        json!({ "reference": "R21", "pin_number": "1", "net": "N", "stub_length": 0 }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body.get("nets_joined"), Some(&Value::Null), "{body}");
}

#[tokio::test]
async fn a_direct_call_with_a_negative_batch_stub_is_refused() {
    let (_dir, path) = fixture();
    let before = std::fs::read_to_string(&path).unwrap();
    let ctx = crate::tools::ToolContext::new(
        ServerConfig::default(),
        std::sync::Arc::new(crate::router::ToolRouter::new()),
    );
    let result = crate::tools::sch_batch::handle_batch_connect_to_net(
        &json!({ "schematic": path.display().to_string(), "net": "N", "stub_length": -2.54,
                 "pins": [{ "reference": "R21", "pin_number": "1" }] }),
        &ctx,
    )
    .await
    .unwrap();
    assert!(result.is_error);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[tokio::test]
async fn connect_pins_to_bus_refuses_a_pin_whose_stub_would_join_a_foreign_wire() {
    let (_dir, path) = midspan_pin_scene().await;
    let (is_error, body) = call(
        &path,
        "connect_pins_to_bus",
        json!({ "bus_y": 140.0, "connections": [
            { "reference": "R60", "pin_number": "1", "net": "D0" },
            { "reference": "R21", "pin_number": "1", "net": "D1" } ] }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["connected_count"], 1, "{body}");
    assert_eq!(body["connected"][0]["reference"], "R21", "{body}");
    assert!(
        body["errors"][0]
            .as_str()
            .unwrap()
            .starts_with("R60.1: the connection would join"),
        "{body}"
    );
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

    // A pin sitting on a wire with no dot: refused, so the circuit is unchanged.
    let (_d, path) = midspan_pin_scene().await;
    let (_before, reference) = snapshot_of(&path);
    let (is_error, _) = call(
        &path,
        "connect_pins",
        json!({ "ref1": "R60", "pin1": "1", "ref2": "R21", "pin2": "2" }),
    )
    .await;
    assert!(is_error);
    assert_same_circuit(&path, &reference).await;
    let (is_error, _) = call(
        &path,
        "batch_connect_to_net",
        json!({ "net": "EN", "pins": [{ "reference": "R60", "pin_number": "1" }] }),
    )
    .await;
    assert!(is_error, "the only pin was skipped");
    assert_same_circuit(&path, &reference).await;

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
