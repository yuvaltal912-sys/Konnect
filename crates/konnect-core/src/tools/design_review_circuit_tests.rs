//! `run_design_review` and `audit_circuit_design` over scratch projects: the
//! circuit-aware review is folded into the verdict, reports what it could not
//! judge, and refuses an unusable design-intent record.

use super::*;
use crate::mcp::protocol::ToolContent;
use crate::tools::circuit_review::tests::{fx, mcu, p, res, Fx, P};
use serde_json::{json, Value};

fn text(result: CallToolResult) -> Value {
    let ToolContent::Text { text } = &result.content[0] else {
        panic!("text expected")
    };
    serde_json::from_str(text)
        .unwrap_or_else(|_| json!({ "raw": text, "is_error": result.is_error }))
}

async fn review(f: &Fx) -> Value {
    let mut args = json!({ "schematic": f.sch.display().to_string() });
    if let Some(b) = &f.board {
        args["board"] = json!(b.display().to_string());
    }
    let result = handle_run_design_review(&args, &test_ctx()).await.unwrap();
    text(result)["design_review"].clone()
}

fn uart_dev(r: &'static str, tx: &'static str, rx: &'static str) -> P {
    p(
        r,
        "Fixture:UARTDEV",
        "UART",
        vec![
            ("1", "TXD", "output", tx),
            ("2", "RXD", "input", rx),
            ("3", "VDD", "power_in", "+3V3"),
            ("4", "GND", "power_in", "GND"),
        ],
    )
}

#[tokio::test]
async fn a_circuit_error_is_folded_into_the_verdict_with_its_checklist_item() {
    let f = fx(
        &[uart_dev("U1", "A", "B"), uart_dev("U2", "A", "B")],
        None,
        None,
    );
    let r = review(&f).await;
    let found: Vec<&Value> = r["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["audit"] == "circuit_design")
        .collect();
    assert_eq!(found.len(), 2, "{r}");
    assert_eq!(found[0]["rule"], "uart_tx_rx_crossed");
    assert_eq!(found[0]["checklist"], "6 Interfaces: UART");
    assert!(r["errors"].as_u64().unwrap() >= 2);
    assert!(
        r["verdict"].as_str().unwrap().starts_with("NOT READY")
            || r["verdict"].as_str().unwrap().starts_with("INCOMPLETE"),
        "{}",
        r["verdict"]
    );
    assert_eq!(r["audits"]["circuit_design"]["status"], "complete");
}

#[tokio::test]
async fn the_review_reports_what_the_circuit_audit_could_not_judge() {
    let f = fx(&[mcu("U1", "SIG0")], None, None);
    let r = review(&f).await;
    let c = &r["circuit"];
    assert_eq!(c["status"], "checked", "{c}");
    assert!(c["rules"].as_u64().unwrap() > 30);
    assert!(
        c["rules_not_run"].as_u64().unwrap() > 0,
        "no board, so placement rules did not run"
    );
    assert!(!c["needs_human"].as_array().unwrap().is_empty());
    // The verdict text is unchanged; needs_human is carried beside it.
    assert!(r["audits"]["circuit_design"].is_object());
}

#[tokio::test]
async fn a_part_missing_from_the_board_leaves_a_diagnostic_the_review_cannot_ignore() {
    // The board holds an empty outline plus U1 only; C1 is not on it.
    let parts = vec![
        mcu("U1", "SIG0"),
        res("R1", "10k", "SIG0", "+3V3"),
        crate::tools::circuit_review::tests::cap_for_tests(),
    ];
    let board = "(kicad_pcb (version 20260206) (generator \"pcbnew\")\n  (layers (0 \"F.Cu\" signal) (2 \"B.Cu\" signal) (25 \"Edge.Cuts\" user))\n  (footprint \"Fixture:U1\" (layer \"F.Cu\") (uuid \"fp-u1\") (at 10 10 0)\n    (property \"Reference\" \"U1\" (at 0 -3 0) (layer \"F.SilkS\") (uuid \"r\") (effects (font (size 1 1))))\n    (pad \"1\" smd rect (at 0 0 0) (size 1 1) (layers \"F.Cu\") (net \"+3V3\") (uuid \"a\"))\n    (pad \"2\" smd rect (at 2 0 0) (size 1 1) (layers \"F.Cu\") (net \"GND\") (uuid \"b\"))\n    (pad \"3\" smd rect (at 4 0 0) (size 1 1) (layers \"F.Cu\") (net \"SIG0\") (uuid \"c\")))\n)\n";
    let f = fx(&parts, Some(board.to_string()), None);
    let r = review(&f).await;
    assert!(
        r["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "circuit_checks_unevaluated"),
        "{}",
        r["diagnostics"]
    );
    assert_ne!(r["status"], "complete");
    assert!(r["verdict"].as_str().unwrap().starts_with("INCOMPLETE"));
}

#[tokio::test]
async fn an_unusable_design_intent_record_makes_the_review_incomplete() {
    let f = fx(&[mcu("U1", "SIG0")], None, None);
    let path = crate::tools::design_intent::design_intent_path(f.sch.parent().unwrap());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{broken").unwrap();
    let r = review(&f).await;
    assert!(
        r["verdict"].as_str().unwrap().starts_with("INCOMPLETE"),
        "{}",
        r["verdict"]
    );
    assert!(r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["audit"] == "circuit_design"));
}

#[tokio::test]
async fn the_standalone_tool_is_registered_and_reads_every_sheet() {
    assert!(tools().iter().any(|t| t.name == "audit_circuit_design"));
    // Child sheet holds the crossed UART mistake; the root is clean.
    let child = fx(
        &[uart_dev("U1", "A", "B"), uart_dev("U2", "A", "B")],
        None,
        None,
    );
    let dir = child.sch.parent().unwrap().to_path_buf();
    let root = dir.join("root.kicad_sch");
    // Reference the child from a root that only holds a resistor.
    let clean = fx(&[res("R9", "1k", "X", "Y")], None, None);
    let mut root_text = std::fs::read_to_string(&clean.sch).unwrap();
    let insert_at = root_text.rfind(')').unwrap();
    let block = konnect_sexp::schematic::format_hierarchical_sheet(
        konnect_sexp::schematic::HierarchicalSheetSpec {
            name: "Child",
            file: "demo.kicad_sch",
            x: 20.0,
            y: 20.0,
            width: 80.0,
            height: 50.0,
            project_name: "root",
            parent_instance_path: "/11111111-1111-4111-8111-111111111111",
            page: "2",
        },
    );
    root_text.insert_str(insert_at, &block);
    std::fs::write(&root, root_text).unwrap();
    let result = handle_audit_circuit_design(
        &json!({ "schematic": root.display().to_string() }),
        &test_ctx(),
    )
    .await
    .unwrap();
    let v = text(result);
    assert_eq!(v["schematic_files"], 2, "{v}");
    let hits: Vec<&Value> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["rule"] == "uart_tx_rx_crossed")
        .collect();
    assert_eq!(hits.len(), 2);
    assert!(hits[0]["source"]
        .as_str()
        .unwrap()
        .ends_with("demo.kicad_sch"));
}

// ─── Reused sheets and sheet-path names ──────────────────────────────────────

use crate::tools::circuit_review::tests::{
    board, buck_on, can_conn, can_xcvr, cap, fp, rails, schematic,
};

const ROOT_UUID: &str = "11111111-1111-4111-8111-111111111111";
const SHEET_A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const SHEET_B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const PATH_A: &str = "/11111111-1111-4111-8111-111111111111/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const PATH_B: &str = "/11111111-1111-4111-8111-111111111111/bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

/// A root sheet block for `file` with a known UUID.
fn sheet_block(name: &str, file: &str, uuid: &str, x: f64, page: &str) -> String {
    let block = konnect_sexp::schematic::format_hierarchical_sheet(
        konnect_sexp::schematic::HierarchicalSheetSpec {
            name,
            file,
            x,
            y: 20.0,
            width: 40.0,
            height: 30.0,
            project_name: "demo",
            parent_instance_path: &format!("/{ROOT_UUID}"),
            page,
        },
    );
    let at = block.find("(uuid \"").unwrap() + "(uuid \"".len();
    let generated = block[at..at + 36].to_string();
    block.replace(&generated, uuid)
}

/// `f`'s root schematic with the given sheets (name, uuid) all pointing at
/// `channel.kicad_sch`, whose text is `child`.
fn with_sheets(f: &Fx, child: String, sheets: &[(&str, &str)]) {
    let mut root = std::fs::read_to_string(&f.sch).unwrap();
    for (i, (name, uuid)) in sheets.iter().enumerate() {
        let at = root.rfind(')').unwrap();
        root.insert_str(
            at,
            &sheet_block(
                name,
                "channel.kicad_sch",
                uuid,
                20.0 + 50.0 * i as f64,
                &(i + 2).to_string(),
            ),
        );
    }
    std::fs::write(&f.sch, root).unwrap();
    std::fs::write(f.sch.with_file_name("channel.kicad_sch"), child).unwrap();
}

async fn audit_circuit(f: &Fx) -> Value {
    let mut args = json!({ "schematic": f.sch.display().to_string() });
    if let Some(b) = &f.board {
        args["board"] = json!(b.display().to_string());
    }
    text(
        handle_audit_circuit_design(&args, &test_ctx())
            .await
            .unwrap(),
    )
}

fn channel_board() -> String {
    let pads = |x: f64, y: f64, r: &str| {
        fp(
            r,
            x,
            y,
            &[
                ("1", 0.0, 0.0, "+3V3"),
                ("2", 2.0, 0.0, "GND"),
                ("3", 4.0, 0.0, "SIG0"),
            ],
        )
    };
    let cap_at = |r: &str, x: f64| {
        fp(
            r,
            x,
            10.0,
            &[("1", 0.0, 0.0, "+3V3"), ("2", 1.0, 0.0, "GND")],
        )
    };
    board(
        &[
            pads(10.0, 10.0, "U1"),
            cap_at("C1", 11.0),
            pads(60.0, 10.0, "U101"),
            cap_at("C101", 100.0),
        ],
        "",
    )
}

#[tokio::test]
async fn a_sheet_used_twice_is_audited_once_per_use_with_each_uses_designators() {
    let f = fx(&[res("R9", "1k", "X", "Y")], Some(channel_board()), None);
    let child = schematic(&[
        mcu("U1", "SIG0")
            .instance(PATH_A, "U1")
            .instance(PATH_B, "U101"),
        cap("C1", "100n", "+3V3", "GND")
            .instance(PATH_A, "C1")
            .instance(PATH_B, "C101"),
    ]);
    with_sheets(&f, child, &[("Channel A", SHEET_A), ("Channel B", SHEET_B)]);
    let v = audit_circuit(&f).await;
    let found: Vec<&Value> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["rule"] == "decoupling_supply_far")
        .collect();
    assert_eq!(found.len(), 1, "{}", v["findings"]);
    assert_eq!(found[0]["evidence"]["capacitor"], "C101");
    assert_eq!(found[0]["evidence"]["ic"], "U101");
    assert_eq!(found[0]["sheet_instance_path"], format!("/{SHEET_B}/"));
    assert_eq!(v["sheet_instances"], 3);
    assert_eq!(v["schematic_files"], 2);
    assert_eq!(v["coverage"]["unaudited_sheet_uses"], 0);
    assert_eq!(v["status"], "checked");
}

#[tokio::test]
async fn a_sheet_used_twice_without_instance_records_is_reported_not_audited_once() {
    let f = fx(&[res("R9", "1k", "X", "Y")], Some(channel_board()), None);
    // Both uses carry the property reference only: which use is which is not
    // written anywhere.
    let child = schematic(&[mcu("U1", "SIG0"), cap("C1", "100n", "+3V3", "GND")]);
    with_sheets(&f, child, &[("Channel A", SHEET_A), ("Channel B", SHEET_B)]);
    let v = audit_circuit(&f).await;
    assert_eq!(v["coverage"]["unaudited_sheet_uses"], 2, "{v}");
    assert_eq!(
        v["unaudited_sheet_uses"][0]["sheet_name_path"],
        "/Channel A/"
    );
    assert!(v["unaudited_sheet_uses"][0]["why"]
        .as_str()
        .unwrap()
        .contains("no instance record"));
    assert_eq!(v["status"], "partial");
    assert!(v["coverage"]["unevaluated_subjects"].as_u64().unwrap() >= 2);

    let r = review(&f).await;
    assert!(
        r["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "circuit_checks_unevaluated"),
        "{}",
        r["diagnostics"]
    );
    assert!(r["verdict"].as_str().unwrap().starts_with("INCOMPLETE"));
}

#[tokio::test]
async fn a_sheet_used_twice_with_the_same_designators_is_reported_not_audited_twice() {
    let f = fx(&[res("R9", "1k", "X", "Y")], Some(channel_board()), None);
    let child = schematic(&[
        mcu("U1", "SIG0")
            .instance(PATH_A, "U1")
            .instance(PATH_B, "U1"),
        cap("C1", "100n", "+3V3", "GND")
            .instance(PATH_A, "C1")
            .instance(PATH_B, "C1"),
    ]);
    with_sheets(&f, child, &[("Channel A", SHEET_A), ("Channel B", SHEET_B)]);
    let v = audit_circuit(&f).await;
    assert_eq!(v["coverage"]["unaudited_sheet_uses"], 1, "{v}");
    assert!(v["unaudited_sheet_uses"][0]["why"]
        .as_str()
        .unwrap()
        .contains("same designator"));
    assert_eq!(v["status"], "partial");
}

#[tokio::test]
async fn a_rail_recorded_with_its_sheet_path_matches_a_local_net_on_that_sheet() {
    let child = |out: &'static str| schematic(&buck_on(out, "31.6k", "10k"));
    let power = |record: &str| {
        let f = fx(
            &[res("R9", "1k", "X", "Y")],
            None,
            Some(rails("U2", 3.3, &[record])),
        );
        // One use of the sheet, named Power; its output net is the local VOUT.
        let mut root = std::fs::read_to_string(&f.sch).unwrap();
        let at = root.rfind(')').unwrap();
        root.insert_str(
            at,
            &sheet_block("Power", "channel.kicad_sch", SHEET_A, 20.0, "2"),
        );
        std::fs::write(&f.sch, root).unwrap();
        std::fs::write(f.sch.with_file_name("channel.kicad_sch"), child("VOUT")).unwrap();
        f
    };
    let f = power("/Power/VOUT");
    let v = audit_circuit(&f).await;
    let by = |rule: &str| {
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|x| x["rule"] == rule)
            .count()
    };
    assert_eq!(by("rail_not_in_schematic"), 0, "{}", v["findings"]);
    assert_eq!(by("rail_source_not_on_rail"), 0, "{}", v["findings"]);
    assert_eq!(
        v["power_tree"]["rails"][0]["feedback"]["status"], "matches_record",
        "{}",
        v["power_tree"]
    );

    // The same local name on another sheet is a different net.
    let other = power("/Audio/VOUT");
    let v = audit_circuit(&other).await;
    assert!(
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["rule"] == "rail_not_in_schematic"),
        "{}",
        v["findings"]
    );
}

// ─── Review-level accounting ─────────────────────────────────────────────────

#[tokio::test]
async fn an_off_board_part_does_not_make_the_review_incomplete() {
    let parts = vec![
        mcu("U1", "SIG0"),
        cap("C1", "100n", "+3V3", "GND").off_board(),
    ];
    let board = board(
        &[fp(
            "U1",
            10.0,
            10.0,
            &[
                ("1", 0.0, 0.0, "+3V3"),
                ("2", 2.0, 0.0, "GND"),
                ("3", 4.0, 0.0, "SIG0"),
            ],
        )],
        "",
    );
    let f = fx(&parts, Some(board), None);
    let r = review(&f).await;
    assert!(
        !r["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "circuit_checks_unevaluated"),
        "{}",
        r["diagnostics"]
    );
    assert_eq!(r["circuit"]["unfitted_parts"], 1);
}

#[tokio::test]
async fn subjects_for_a_human_are_counted_in_the_review_without_making_it_incomplete() {
    let mut parts = buck_on("+3V3", "31.6k", "10k");
    parts[0] = {
        // A regulator whose reference voltage the table does not hold.
        p(
            "U2",
            "Regulator_Switching:XR9999",
            "XR9999",
            vec![
                ("1", "VIN", "power_in", "+12V"),
                ("2", "GND", "power_in", "GND"),
                ("3", "SW", "output", "SWN"),
                ("4", "FB", "input", "FB"),
            ],
        )
    };
    let f = fx(&parts, None, Some(rails("U2", 3.3, &["+3V3"])));
    let r = review(&f).await;
    assert_eq!(r["circuit"]["needs_human_subjects"], 1, "{}", r["circuit"]);
    assert!(
        !r["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "circuit_checks_unevaluated"),
        "{}",
        r["diagnostics"]
    );
}

#[tokio::test]
async fn an_unreadable_termination_value_makes_the_review_incomplete_without_a_board() {
    let f = fx(
        &[can_xcvr(), can_conn(), res("R1", "DNP", "CANH", "CANL")],
        None,
        None,
    );
    assert!(f.board.is_none());
    let r = review(&f).await;
    assert!(
        r["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "circuit_checks_unevaluated"),
        "{}",
        r["diagnostics"]
    );
    assert!(r["verdict"].as_str().unwrap().starts_with("INCOMPLETE"));
}
