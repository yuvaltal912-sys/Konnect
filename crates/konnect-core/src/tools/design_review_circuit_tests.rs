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
