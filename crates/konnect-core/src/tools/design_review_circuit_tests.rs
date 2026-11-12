//! `run_design_review` and `audit_circuit_design` over scratch projects: the
//! circuit-aware review is folded into the verdict, reports what it could not
//! judge, and refuses an unusable design-intent record.

use super::*;
use crate::mcp::protocol::ToolContent;
use crate::tools::circuit_review::tests::{fx, mcu, p, res, Fx, P};
use serde_json::{json, Value};
use std::collections::BTreeSet;

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

// H01/H15: the remote capacitor must be discovered, and auditing the joined
// design must not duplicate the IC or stamp the capacitor's sheet on it.
#[tokio::test]
async fn cross_sheet_global_decoupling_is_evaluated_once_with_its_ic_origin() {
    let ic = p(
        "U1",
        "Fixture:MCU",
        "MCU",
        vec![
            ("1", "VDD", "power_in", "g:+3V3"),
            ("2", "GND", "power_in", "g:GND"),
            ("3", "IO", "input", "LOCAL_IO"),
        ],
    );
    let b = board(
        &[
            fp(
                "U1",
                10.0,
                10.0,
                &[("1", 0.0, 0.0, "+3V3"), ("2", 0.0, 2.0, "GND")],
            ),
            fp(
                "C1",
                40.0,
                10.0,
                &[("1", 0.0, 0.0, "+3V3"), ("2", 0.0, 2.0, "GND")],
            ),
        ],
        "",
    );
    let f = fx(&[ic], Some(b), None);
    with_sheets(
        &f,
        schematic(&[cap("C1", "100n", "g:+3V3", "g:GND")]),
        &[("Caps", SHEET_A)],
    );
    let v = audit_circuit(&f).await;
    let found: Vec<_> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["rule"] == "decoupling_supply_far")
        .collect();
    assert_eq!(found.len(), 1, "{v}");
    assert_eq!(found[0]["evidence"]["capacitor"], "C1");
    assert_eq!(found[0]["sheet_instance_path"], "/");
    assert_eq!(v["scope"], "schematic_hierarchy");
    assert_eq!(v["schematic_files"], 2);
    assert_eq!(v["sheet_instances"], 2);
}

// H09/H10: incomplete coverage cannot establish an absent driver, while a
// known short between two drivers remains positive evidence.
#[tokio::test]
async fn cross_sheet_incomplete_hierarchy_withholds_absence_but_keeps_conflicts() {
    for (kind, expected) in [("input", false), ("output", true)] {
        let f = fx(
            &[
                p(
                    "U1",
                    "Fixture:UART",
                    "UART",
                    vec![
                        ("1", "RX", kind, "g:BUS"),
                        ("2", "VDD", "power_in", "+3V3"),
                        ("3", "GND", "power_in", "GND"),
                    ],
                ),
                p(
                    "U2",
                    "Fixture:UART",
                    "UART",
                    vec![
                        ("1", "RX", kind, "g:BUS"),
                        ("2", "VDD", "power_in", "+3V3"),
                        ("3", "GND", "power_in", "GND"),
                    ],
                ),
            ],
            None,
            None,
        );
        with_sheets(&f, schematic(&[]), &[("Absent", SHEET_A)]);
        std::fs::remove_file(f.sch.with_file_name("channel.kicad_sch")).unwrap();
        let v = audit_circuit(&f).await;
        let found = v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|x| x["rule"] == "uart_tx_rx_crossed")
            .count();
        assert_eq!(found, usize::from(expected), "{v}");
        assert_eq!(v["status"], "partial");
        if !expected {
            assert!(
                v["unproven"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|x| x["rule"] == "uart_tx_rx_crossed"),
                "{v}"
            );
        }
        let r = review(&f).await;
        assert!(
            r["verdict"].as_str().unwrap().starts_with("INCOMPLETE"),
            "{r}"
        );
    }
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
    // One unaudited use is one unevaluated subject: the parent's sheet entry
    // for the same use is not a second gap.
    assert_eq!(v["coverage"]["unevaluated_subjects"], 2, "{v}");
    assert_eq!(v["coverage"]["connectivity_gaps"], 0);

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

#[tokio::test]
async fn cross_sheet_missing_inventory_does_not_assert_missing_rail_or_source() {
    let f = fx(
        &[res("R1", "1k", "g:+3V3", "GND")],
        None,
        Some(crate::tools::circuit_review::tests::rails(
            "U9",
            3.3,
            &["UNSEEN"],
        )),
    );
    with_sheets(&f, schematic(&[]), &[("Absent", SHEET_A)]);
    std::fs::remove_file(f.sch.with_file_name("channel.kicad_sch")).unwrap();
    let v = audit_circuit(&f).await;
    for rule in ["rail_not_in_schematic", "rail_source_missing"] {
        assert!(
            !v["findings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["rule"] == rule),
            "{v}"
        );
        assert!(
            v["unproven"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["rule"] == rule),
            "{v}"
        );
    }
    assert_eq!(v["power_tree"]["status"], "partial");
}

#[tokio::test]
async fn cross_sheet_incomplete_feedback_withholds_the_divider_calculation() {
    let f = fx(
        &crate::tools::circuit_review::tests::buck_on("+3V3", "100k", "10k"),
        None,
        Some(crate::tools::circuit_review::tests::rails(
            "U2",
            3.3,
            &["+3V3"],
        )),
    );
    let source = std::fs::read_to_string(&f.sch)
        .unwrap()
        .replace("(label \"FB\"", "(global_label \"FB\" (shape input)");
    std::fs::write(&f.sch, source).unwrap();
    with_sheets(&f, schematic(&[]), &[("Absent", SHEET_A)]);
    std::fs::remove_file(f.sch.with_file_name("channel.kicad_sch")).unwrap();
    let v = audit_circuit(&f).await;
    assert!(
        !v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule"] == "feedback_vout_mismatch"),
        "{v}"
    );
    assert_eq!(
        v["power_tree"]["rails"][0]["feedback"]["status"],
        "unevaluated"
    );
    assert!(v["coverage"]["unevaluated_subjects"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn cross_sheet_duplicate_display_paths_do_not_resolve_qualified_intent_arbitrarily() {
    let f = fx(
        &[res("R9", "1k", "ROOT", "RET")],
        None,
        Some(crate::tools::circuit_review::tests::rails(
            "R1",
            3.3,
            &["/Power/VOUT"],
        )),
    );
    let child = schematic(&[res("R1", "1k", "VOUT", "RET")
        .instance(PATH_A, "R1")
        .instance(PATH_B, "R101")]);
    with_sheets(&f, child, &[("Power", SHEET_A), ("Power", SHEET_B)]);
    let v = audit_circuit(&f).await;
    assert!(
        v["power_tree"]["rails"][0]["present_in_schematic"].is_null(),
        "{v}"
    );
    assert!(
        v["unproven"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["rule"] == "rail_not_in_schematic"),
        "{v}"
    );
    assert_eq!(v["status"], "partial", "{v}");
    assert!(
        v["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["message"]
                .as_str()
                .is_some_and(|s| s.contains("ambiguous qualified"))),
        "{v}"
    );
}

#[tokio::test]
async fn cross_sheet_termination_banks_on_different_sheets_do_not_get_a_single_end_verdict() {
    let f = fx(
        &[
            crate::tools::circuit_review::tests::can_xcvr(),
            res("R1", "120", "CANH", "CANL"),
        ],
        None,
        None,
    );
    let globalize = |s: String| {
        s.replace("(label \"CANH\"", "(global_label \"CANH\" (shape input)")
            .replace("(label \"CANL\"", "(global_label \"CANL\" (shape input)")
    };
    std::fs::write(&f.sch, globalize(std::fs::read_to_string(&f.sch).unwrap())).unwrap();
    with_sheets(
        &f,
        globalize(schematic(&[res("R101", "120", "CANH", "CANL")])),
        &[("Remote", SHEET_A)],
    );
    let v = audit_circuit(&f).await;
    assert!(
        !v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule"] == "can_termination_value"),
        "{v}"
    );
    assert!(
        v["needs_human"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule"] == "can_termination_value"),
        "{v}"
    );
    assert_eq!(v["interfaces"][0]["termination"]["total_ohms"], 60.0);
}

#[tokio::test]
async fn cross_sheet_cycles_and_unreadable_children_keep_both_review_entry_points_incomplete() {
    for broken in ["cycle", "parse"] {
        let f = fx(&[res("R1", "1k", "g:A", "B")], None, None);
        with_sheets(&f, schematic(&[]), &[("Child", SHEET_A)]);
        let child = f.sch.with_file_name("channel.kicad_sch");
        if broken == "parse" {
            std::fs::write(&child, "(broken").unwrap();
        } else {
            let mut source = schematic(&[]);
            let at = source.rfind(')').unwrap();
            source.insert_str(
                at,
                &sheet_block("Cycle", "demo.kicad_sch", SHEET_B, 20.0, "3"),
            );
            std::fs::write(&child, source).unwrap();
        }
        let audit = audit_circuit(&f).await;
        assert_eq!(audit["status"], "partial", "{audit}");
        assert_eq!(audit["connectivity"]["status"], "partial");
        let r = review(&f).await;
        assert!(
            r["verdict"].as_str().unwrap().starts_with("INCOMPLETE"),
            "{r}"
        );
    }
}

fn fixture_model(root: &Path) -> crate::tools::circuit_model::Circuit {
    use crate::tools::circuit_model::{Circuit, SheetPlace};
    let (_, tree) = konnect_sexp::schematic::read_schematic(root).unwrap();
    let uuid = tree.find_str("uuid").unwrap();
    let traversal = inspect_hierarchy(root).unwrap();
    assert!(
        traversal.diagnostics.is_empty(),
        "{:?}",
        traversal.diagnostics
    );
    let sheets = circuit_sheet_uses(&traversal.auditable_instances)
        .iter()
        .map(|u| {
            Circuit::load_at(
                &u.file,
                &SheetPlace {
                    symbol_path: u
                        .instance_path
                        .as_ref()
                        .map(|p| format!("/{uuid}{}", p.trim_end_matches('/'))),
                    instance_path: u.instance_path.clone(),
                    name_path: u.name_path.clone(),
                },
            )
            .unwrap()
        })
        .collect();
    Circuit::join(sheets, true, &BTreeSet::new())
}

fn partition_from_model(
    c: &crate::tools::circuit_model::Circuit,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut result: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for p in &c.parts {
        for pin in &p.pins {
            result
                .entry(pin.net.clone())
                .or_default()
                .insert(format!("{}.{}", p.reference, pin.number));
        }
    }
    result
}

fn assert_same_pin_partition(
    actual: &BTreeMap<String, BTreeSet<String>>,
    expected: &BTreeMap<String, BTreeSet<String>>,
) {
    let universe = |p: &BTreeMap<String, BTreeSet<String>>| {
        p.values().flatten().cloned().collect::<BTreeSet<_>>()
    };
    assert_eq!(
        universe(actual),
        universe(expected),
        "physical pin universe differs (including singleton nets)"
    );
    assert_eq!(
        actual.values().collect::<BTreeSet<_>>(),
        expected.values().collect::<BTreeSet<_>>(),
        "pin partitions differ"
    );
}

#[test]
fn cross_sheet_real_serialized_fixture_matches_kicads_recorded_pin_partition() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let model = fixture_model(&fixtures.join("single_pin_nets.kicad_sch"));
    assert!(model.inventory_complete, "{:?}", model.connectivity_gaps);
    let netlist = crate::simulation::kicad_netlist::parse(
        &std::fs::read_to_string(fixtures.join("single_pin_nets.net")).unwrap(),
    )
    .unwrap();
    let expected = netlist
        .nets
        .iter()
        .map(|net| {
            (
                net.name.clone(),
                net.pins
                    .iter()
                    .map(|p| format!("{}.{}", p.reference, p.pin))
                    .collect(),
            )
        })
        .collect();
    assert_same_pin_partition(&partition_from_model(&model), &expected);
}

#[test]
#[should_panic(expected = "physical pin universe differs")]
fn cross_sheet_pin_partition_oracle_rejects_a_dropped_singleton() {
    let expected = BTreeMap::from([(
        "singleton".to_string(),
        BTreeSet::from(["R1.1".to_string()]),
    )]);
    assert_same_pin_partition(&BTreeMap::new(), &expected);
}

#[tokio::test]
async fn cross_sheet_public_audit_accepts_compatible_units_of_one_part_in_reused_sheet() {
    let f = fx(&[res("R9", "1k", "ROOT", "RET")], None, None);
    let child = schematic(&[p(
        "U1",
        "Fixture:Dual",
        "Dual",
        vec![("1", "A", "input", "LOCAL")],
    )
    .instance(PATH_A, "U1")
    .instance(PATH_B, "U1")]);
    let child = child.replace("(symbol \"Dual_1_1\"", "(symbol \"Dual_2_1\" (pin input line (at 0 0 0) (length 2.54) (name \"B\") (number \"2\"))) (symbol \"Dual_1_1\"")
        .replace(&format!("(path \"{PATH_B}\" (reference \"U1\") (unit 1))"), &format!("(path \"{PATH_B}\" (reference \"U1\") (unit 2))"));
    with_sheets(&f, child, &[("A", SHEET_A), ("B", SHEET_B)]);
    let v = audit_circuit(&f).await;
    assert_eq!(v["coverage"]["unaudited_sheet_uses"], 0, "{v}");
    assert_eq!(v["sheet_instances"], 3);
    assert_eq!(v["status"], "checked");
    let model = fixture_model(&f.sch);
    assert_eq!(model.part("U1").unwrap().pins.len(), 2);
    assert_eq!(model.origins["U1"].len(), 2);
}

#[tokio::test]
async fn cross_sheet_multiple_or_mixed_split_candidates_are_not_hidden_by_first_match() {
    for direct in [false, true] {
        let mut parts = vec![crate::tools::circuit_review::tests::can_xcvr()];
        if direct {
            parts.push(res("R1", "120", "CANH", "CANL"));
        } else {
            parts.extend([
                res("R1", "60", "CANH", "MID"),
                res("R2", "60", "CANL", "MID"),
            ]);
        }
        let f = fx(&parts, None, None);
        let globalize = |s: String| {
            s.replace("(label \"CANH\"", "(global_label \"CANH\" (shape input)")
                .replace("(label \"CANL\"", "(global_label \"CANL\" (shape input)")
        };
        std::fs::write(&f.sch, globalize(std::fs::read_to_string(&f.sch).unwrap())).unwrap();
        with_sheets(
            &f,
            globalize(schematic(&[
                res("R3", "100", "CANH", "OTHER"),
                res("R4", "100", "CANL", "OTHER"),
            ])),
            &[("Remote", SHEET_A)],
        );
        let v = audit_circuit(&f).await;
        let check = v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["rule"] == "can_termination_value")
            .unwrap();
        assert_ne!(check["passed"], 1, "{v}");
        assert!(
            v["needs_human"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["rule"] == "can_termination_value"),
            "{v}"
        );
        let t = &v["interfaces"][0]["termination"];
        assert!(t["candidates"].as_array().unwrap().len() >= 2);
        assert!(
            t["total_ohms"].is_null(),
            "one candidate's value is not the complete network total"
        );
        assert!(t["observed_resistors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "R4"));
    }
}

#[tokio::test]
async fn cross_sheet_child_voltage_and_unfitted_source_origins_survive_both_entry_points() {
    let f = fx(
        &[res("R9", "1k", "ROOT", "RET")],
        None,
        Some(crate::tools::circuit_review::tests::rails(
            "U2",
            3.3,
            &["+5V"],
        )),
    );
    let child = schematic(&[
        p(
            "U2",
            "Fixture:Supply",
            "Supply",
            vec![
                ("1", "OUT", "power_out", "+5V"),
                ("2", "GND", "power_in", "GND"),
                ("3", "EN", "input", "EN"),
            ],
        )
        .dnp(),
        res("R1", "1k", "+5V", "GND"),
    ]);
    with_sheets(&f, child, &[("Power", SHEET_A)]);
    for v in [audit_circuit(&f).await, review(&f).await] {
        for rule in ["rail_name_voltage", "rail_source_missing"] {
            let finding = v["findings"]
                .as_array()
                .unwrap()
                .iter()
                .find(|x| x["rule"] == rule)
                .unwrap();
            assert_eq!(
                finding["source"],
                f.sch
                    .with_file_name("channel.kicad_sch")
                    .display()
                    .to_string(),
                "{finding}"
            );
            assert_eq!(finding["sheet_instance_path"], format!("/{SHEET_A}/"));
            assert_eq!(finding["origins"].as_array().unwrap().len(), 1);
        }
    }
}

#[tokio::test]
async fn cross_sheet_net_level_i2c_finding_keeps_all_participating_origins() {
    let f = fx(
        &[p(
            "U1",
            "Fixture:I2C",
            "I2C",
            vec![
                ("1", "SDA", "bidirectional", "g:SDA"),
                ("2", "VDD", "power_in", "g:+3V3"),
                ("3", "GND", "power_in", "GND"),
            ],
        )],
        None,
        None,
    );
    with_sheets(
        &f,
        schematic(&[res("R1", "100", "g:SDA", "g:+3V3")]),
        &[("Pullups", SHEET_A)],
    );
    let v = audit_circuit(&f).await;
    let finding = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["rule"] == "i2c_pullup_value")
        .unwrap();
    assert_eq!(finding["origins"].as_array().unwrap().len(), 2, "{finding}");
    assert_eq!(finding["source_scope"], "hierarchy");
}

#[tokio::test]
async fn cross_sheet_termination_placement_sees_both_known_transceiver_endpoints_once() {
    let transceiver = |r| {
        p(
            r,
            "Fixture:CANXCVR",
            "TCAN",
            vec![
                ("1", "CANH", "bidirectional", "g:H"),
                ("2", "CANL", "bidirectional", "g:L"),
                ("3", "VCC", "power_in", "+5V"),
                ("4", "GND", "power_in", "GND"),
            ],
        )
    };
    let b = board(
        &[
            fp(
                "U1",
                10.0,
                10.0,
                &[
                    ("1", 0.0, 0.0, "H"),
                    ("2", 0.0, 2.0, "L"),
                    ("3", 4.0, 0.0, "+5V"),
                    ("4", 4.0, 2.0, "GND"),
                ],
            ),
            fp(
                "R1",
                11.0,
                10.0,
                &[("1", 0.0, 0.0, "H"), ("2", 0.0, 2.0, "L")],
            ),
            fp(
                "U2",
                100.0,
                10.0,
                &[
                    ("1", 0.0, 0.0, "H"),
                    ("2", 0.0, 2.0, "L"),
                    ("3", 4.0, 0.0, "+5V"),
                    ("4", 4.0, 2.0, "GND"),
                ],
            ),
            fp(
                "R2",
                101.0,
                10.0,
                &[("1", 0.0, 0.0, "H"), ("2", 0.0, 2.0, "L")],
            ),
        ],
        "",
    );
    let f = fx(
        &[transceiver("U1"), res("R1", "120", "g:H", "g:L")],
        Some(b),
        None,
    );
    with_sheets(
        &f,
        schematic(&[transceiver("U2"), res("R2", "120", "g:H", "g:L")]),
        &[("OtherEnd", SHEET_A)],
    );
    let v = audit_circuit(&f).await;
    assert!(
        !v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule"] == "termination_far_from_endpoint"),
        "{v}"
    );
    let check = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["rule"] == "termination_far_from_endpoint")
        .unwrap();
    assert_eq!(check["examined"], 2, "each physical resistor once");
    assert_eq!(check["passed"], 2);
}

#[test]
fn cross_sheet_generated_native_fixtures_match_kicads_complete_pin_partitions() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/circuit_hierarchy");
    for (name, pins, parts) in [
        ("two_sheet", 14, 4),
        ("repeated_child", 22, 8),
        ("local_power", 22, 8),
    ] {
        let directory = base.join(name);
        let model = fixture_model(&directory.join("cross_sheet.kicad_sch"));
        assert!(
            model.inventory_complete,
            "{name}: {:?}",
            model.connectivity_gaps
        );
        assert_eq!(model.parts.len(), parts);
        let netlist = crate::simulation::kicad_netlist::parse(
            &std::fs::read_to_string(directory.join("cross_sheet.net")).unwrap(),
        )
        .unwrap();
        let expected: BTreeMap<_, _> = netlist
            .nets
            .iter()
            .map(|net| {
                (
                    net.name.clone(),
                    net.pins
                        .iter()
                        .map(|p| format!("{}.{}", p.reference, p.pin))
                        .collect::<BTreeSet<_>>(),
                )
            })
            .collect();
        assert_eq!(expected.values().map(BTreeSet::len).sum::<usize>(), pins);
        assert_same_pin_partition(&partition_from_model(&model), &expected);
    }
}

#[tokio::test]
async fn cross_sheet_unknown_endpoint_placement_withholds_far_but_keeps_near_witness() {
    for r_x in [11.0, 50.0] {
        let xcvr = |r| {
            p(
                r,
                "Fixture:CANXCVR",
                "TCAN",
                vec![
                    ("1", "CANH", "bidirectional", "g:H"),
                    ("2", "CANL", "bidirectional", "g:L"),
                    ("3", "VCC", "power_in", "+5V"),
                    ("4", "GND", "power_in", "GND"),
                ],
            )
        };
        let b = board(
            &[
                fp(
                    "U1",
                    10.0,
                    10.0,
                    &[
                        ("1", 0.0, 0.0, "H"),
                        ("2", 0.0, 2.0, "L"),
                        ("3", 2.0, 0.0, "+5V"),
                        ("4", 2.0, 2.0, "GND"),
                    ],
                ),
                fp(
                    "R1",
                    r_x,
                    10.0,
                    &[("1", 0.0, 0.0, "H"), ("2", 0.0, 2.0, "L")],
                ),
            ],
            "",
        );
        let f = fx(&[xcvr("U1"), res("R1", "120", "g:H", "g:L")], Some(b), None);
        with_sheets(
            &f,
            schematic(&[xcvr("U2")]),
            &[("UnknownPlacement", SHEET_A)],
        );
        let v = audit_circuit(&f).await;
        assert!(
            !v["findings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|x| x["rule"] == "termination_far_from_endpoint"),
            "{v}"
        );
        let check = v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["rule"] == "termination_far_from_endpoint")
            .unwrap();
        assert_eq!(check["passed"], u64::from(r_x == 11.0));
        assert_eq!(check["unevaluated"], u64::from(r_x != 11.0));
    }
}

#[tokio::test]
async fn cross_sheet_feedback_uses_connected_output_among_unrelated_same_name_aliases() {
    let f = fx(
        &[res("R9", "1k", "+3V3", "ROOT_RETURN")],
        None,
        Some(crate::tools::circuit_review::tests::rails(
            "U2",
            3.3,
            &["+3V3"],
        )),
    );
    with_sheets(
        &f,
        schematic(&crate::tools::circuit_review::tests::buck_on(
            "+3V3", "31.6k", "10k",
        )),
        &[("Power", SHEET_A)],
    );
    let v = audit_circuit(&f).await;
    let check = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["rule"] == "feedback_vout_mismatch")
        .unwrap();
    assert_eq!(
        check["passed"], 1,
        "the unrelated root alias must not shadow the regulator's actual output: {v}"
    );
    let model = fixture_model(&f.sch);
    let pin = |r: &str, n: &str| {
        model
            .part(r)
            .unwrap()
            .pins
            .iter()
            .find(|p| p.number == n)
            .unwrap()
            .net
            .clone()
    };
    assert_ne!(
        pin("R9", "1"),
        pin("R1", "1"),
        "bare-name lookup must not union distinct electrical nets"
    );
}

// Two sheet symbols with one UUID give two uses the same instance path: which
// instance records belong to which use cannot be told, so neither is joined.
#[tokio::test]
async fn cross_sheet_duplicate_sheet_uuids_are_quarantined_by_both_entry_points() {
    let f = fx(&[res("R9", "1k", "X", "Y")], None, None);
    with_sheets(
        &f,
        schematic(&[uart_dev("U1", "A", "B"), uart_dev("U2", "A", "B")]),
        &[("First", SHEET_A), ("Second", SHEET_A)],
    );
    let v = audit_circuit(&f).await;
    let unaudited = v["unaudited_sheet_uses"].as_array().unwrap();
    assert_eq!(unaudited.len(), 2, "{v}");
    for entry in unaudited {
        assert_eq!(entry["sheet_instance_path"], format!("/{SHEET_A}/"));
        assert!(entry["why"]
            .as_str()
            .unwrap()
            .contains("ambiguous duplicate sheet-instance path"));
    }
    assert_eq!(v["coverage"]["unaudited_sheet_uses"], 2);
    assert_eq!(
        v["coverage"]["unevaluated_subjects"], 2,
        "each use once: {v}"
    );
    assert_eq!(v["coverage"]["connectivity_gaps"], 0);
    assert_eq!(v["sheet_instances"], 1, "only the root use is audited");
    assert_eq!(v["status"], "partial");
    assert_eq!(v["connectivity"]["status"], "partial");
    assert!(
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|x| x["rule"] != "uart_tx_rx_crossed"),
        "a quarantined use is not positive evidence: {v}"
    );
    let r = review(&f).await;
    assert!(
        r["verdict"].as_str().unwrap().starts_with("INCOMPLETE"),
        "{r}"
    );
    // With distinct UUIDs both uses carry the same designators, which is the
    // older, separately reported ambiguity; the duplicate-path reason is gone.
    let g = fx(&[res("R9", "1k", "X", "Y")], None, None);
    with_sheets(
        &g,
        schematic(&[uart_dev("U1", "A", "B"), uart_dev("U2", "A", "B")]),
        &[("First", SHEET_A), ("Second", SHEET_B)],
    );
    let v = audit_circuit(&g).await;
    assert!(v["unaudited_sheet_uses"]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| !entry["why"]
            .as_str()
            .unwrap()
            .contains("ambiguous duplicate sheet-instance path")));
}

// Instance records are keyed by the root sheet's UUID. Without it no use's
// designators can be resolved, so a known hierarchy is still not complete.
#[tokio::test]
async fn cross_sheet_root_without_a_uuid_is_reported_and_never_complete() {
    let f = fx(
        &[uart_dev("U1", "A", "B"), uart_dev("U2", "B", "A")],
        None,
        None,
    );
    let complete = audit_circuit(&f).await;
    assert_eq!(complete["status"], "checked", "{complete}");
    assert_eq!(complete["connectivity"]["status"], "complete");
    let source = std::fs::read_to_string(&f.sch).unwrap();
    let stripped = source.replace(&format!("  (uuid \"{ROOT_UUID}\")\n"), "");
    assert_ne!(source, stripped);
    std::fs::write(&f.sch, stripped).unwrap();
    let v = audit_circuit(&f).await;
    let gaps = v["connectivity"]["gaps"].as_array().unwrap();
    assert_eq!(gaps.len(), 1, "{v}");
    let gap = gaps[0].as_str().unwrap();
    assert!(gap.starts_with("root schematic has no UUID"), "{gap}");
    assert_eq!(v["coverage"]["connectivity_gaps"], 1);
    assert_eq!(v["coverage"]["unevaluated_subjects"], 1);
    assert_eq!(
        v["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["code"] == "circuit_connectivity_gap" && d["message"] == gap)
            .count(),
        1,
        "the gap is reported once: {v}"
    );
    assert_eq!(v["status"], "partial");
    assert_eq!(v["connectivity"]["status"], "partial");
    // What is known stays known: the crossed pair is still two passes.
    assert_eq!(
        v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["rule"] == "uart_tx_rx_crossed")
            .unwrap()["passed"],
        2,
        "{v}"
    );
    // The review must not present a verdict the circuit audit cannot back.
    let r = review(&f).await;
    assert_eq!(r["circuit"]["status"], "partial", "{r}");
    assert!(
        r["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "circuit_checks_unevaluated"),
        "{r}"
    );
    assert!(
        r["verdict"].as_str().unwrap().starts_with("INCOMPLETE"),
        "{r}"
    );
}

// A node wired with its bus pins swapped is still an endpoint of the same
// pair: its termination is placed against it, and each resistor is still
// judged once whichever way round a transceiver names the pair.
#[tokio::test]
async fn cross_sheet_termination_placement_counts_a_swapped_polarity_node_as_an_endpoint_once() {
    let transceiver = |r, high, low| {
        p(
            r,
            "Fixture:CANXCVR",
            "TCAN",
            vec![
                ("1", "CANH", "bidirectional", high),
                ("2", "CANL", "bidirectional", low),
                ("3", "VCC", "power_in", "+5V"),
                ("4", "GND", "power_in", "GND"),
            ],
        )
    };
    let node = |r: &str, x: f64, high: &str, low: &str| {
        fp(
            r,
            x,
            10.0,
            &[
                ("1", 0.0, 0.0, high),
                ("2", 0.0, 2.0, low),
                ("3", 4.0, 0.0, "+5V"),
                ("4", 4.0, 2.0, "GND"),
            ],
        )
    };
    let resistor = |r: &str, x: f64| fp(r, x, 10.0, &[("1", 0.0, 0.0, "H"), ("2", 0.0, 2.0, "L")]);
    let b = board(
        &[
            node("U1", 10.0, "H", "L"),
            resistor("R1", 11.0),
            node("U2", 100.0, "L", "H"),
            resistor("R2", 101.0),
        ],
        "",
    );
    let f = fx(
        &[
            transceiver("U1", "g:H", "g:L"),
            res("R1", "120", "g:H", "g:L"),
        ],
        Some(b),
        None,
    );
    with_sheets(
        &f,
        schematic(&[
            transceiver("U2", "g:L", "g:H"),
            res("R2", "120", "g:H", "g:L"),
        ]),
        &[("OtherEnd", SHEET_A)],
    );
    let v = audit_circuit(&f).await;
    assert!(
        !v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule"] == "termination_far_from_endpoint"),
        "R2 sits beside U2, which is on the same pair: {v}"
    );
    let check = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["rule"] == "termination_far_from_endpoint")
        .unwrap();
    assert_eq!(check["examined"], 2, "each physical resistor once: {v}");
    assert_eq!(check["passed"], 2);
}

// ─── Independent review A (2026-11-12) ───────────────────────────────────────

// B1: two receivers and no transmitter on a global net is an error when the
// hierarchy is complete. Without a root UUID the absence is withheld, and
// then neither entry point may read as complete.
#[tokio::test]
async fn cross_sheet_root_without_a_uuid_withholds_absence_and_makes_the_review_incomplete() {
    let rx_only = |r| {
        p(
            r,
            "Fixture:UART",
            "UART",
            vec![
                ("1", "RX", "input", "g:BUS"),
                ("2", "VDD", "power_in", "+3V3"),
                ("3", "GND", "power_in", "GND"),
            ],
        )
    };
    let uart = |v: &Value| {
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|x| x["rule"] == "uart_tx_rx_crossed")
            .count()
    };
    let f = fx(&[rx_only("U1"), rx_only("U2")], None, None);
    let v = audit_circuit(&f).await;
    assert_eq!(uart(&v), 1, "{v}");
    assert_eq!(v["status"], "checked");
    let source = std::fs::read_to_string(&f.sch).unwrap();
    std::fs::write(
        &f.sch,
        source.replace(&format!("  (uuid \"{ROOT_UUID}\")\n"), ""),
    )
    .unwrap();
    let v = audit_circuit(&f).await;
    assert_eq!(uart(&v), 0, "{v}");
    assert!(v["unproven"]
        .as_array()
        .unwrap()
        .iter()
        .any(|u| u["rule"] == "uart_tx_rx_crossed"));
    assert_eq!(v["status"], "partial");
    let gap = v["connectivity"]["gaps"][0].as_str().unwrap();
    assert!(gap.contains("no UUID"), "{gap}");
    assert!(
        gap.contains("save it in KiCad"),
        "the gap says what the user can do: {gap}"
    );
    let r = review(&f).await;
    assert_eq!(r["status"], "partial", "{r}");
    assert!(r["verdict"].as_str().unwrap().starts_with("INCOMPLETE"));
}

// B1, general guard: whatever makes the circuit audit partial, the review
// carries a diagnostic for it, counted subjects or not.
#[test]
fn cross_sheet_review_seam_reports_any_partial_circuit_audit() {
    let root = Path::new("/scratch/demo.kicad_sch");
    let body = |status: &str, connectivity: &str, subjects: u64| {
        json!({
            "status": status,
            "connectivity": {"status": connectivity},
            "coverage": {"unevaluated_subjects": subjects},
        })
    };
    assert!(circuit_audit_gap_diagnostic(&body("checked", "complete", 0), root).is_none());
    let counted = circuit_audit_gap_diagnostic(&body("partial", "partial", 3), root).unwrap();
    assert_eq!(counted["code"], "circuit_checks_unevaluated");
    assert!(counted["message"]
        .as_str()
        .unwrap()
        .starts_with("3 subject"));
    for (status, connectivity) in [("partial", "complete"), ("checked", "partial")] {
        let uncounted = circuit_audit_gap_diagnostic(&body(status, connectivity, 0), root)
            .unwrap_or_else(|| panic!("{status}/{connectivity} must surface"));
        assert_eq!(uncounted["code"], "circuit_review_partial");
        assert_eq!(uncounted["source"], "/scratch/demo.kicad_sch");
    }
}

// M2: a child that cannot be read is one unevaluated subject for the circuit
// audit, with or without ports leading to it.
#[tokio::test]
async fn cross_sheet_a_missing_child_with_ports_is_one_unevaluated_subject() {
    let f = fx(&[res("R1", "1k", "LEFT", "RIGHT")], None, None);
    with_sheets(&f, schematic(&[]), &[("Absent", SHEET_A)]);
    std::fs::remove_file(f.sch.with_file_name("channel.kicad_sch")).unwrap();
    let source = std::fs::read_to_string(&f.sch).unwrap();
    // Two ports on the sheet symbol, each on one of R1's nets.
    let at = source.find("(property \"Sheetname\"").unwrap();
    let mut with_ports = source.clone();
    with_ports.insert_str(
        at,
        "(pin \"P1\" input (at 20 100 0) (effects (font (size 1.27 1.27)))) (pin \"P2\" input (at 20 97.46 0) (effects (font (size 1.27 1.27)))) ",
    );
    std::fs::write(&f.sch, with_ports).unwrap();
    let v = audit_circuit(&f).await;
    assert_eq!(v["coverage"]["connectivity_gaps"], 1, "{v}");
    assert_eq!(v["coverage"]["unevaluated_subjects"], 1);
    assert_eq!(v["status"], "partial");
}

// m3: KiCad joins every same-name pin of a sheet symbol through the child's
// label (eeschema/connection_graph.cpp, propagateToNeighbors: a child label
// finds each parent subgraph holding a pin of that name, "found additional
// parent"). The join is kept, complete, and said out loud.
#[tokio::test]
async fn cross_sheet_same_name_sheet_pins_join_their_parent_nets_and_say_so() {
    let f = fx(
        &[
            res("R1", "1k", "LEFT", "RET"),
            res("R3", "1k", "RIGHT", "RET"),
        ],
        None,
        None,
    );
    let child = schematic(&[res("R2", "1k", "LINK", "CHILD_RET")]).replace(
        "(label \"LINK\"",
        "(hierarchical_label \"LINK\" (shape input)",
    );
    with_sheets(&f, child, &[("Child", SHEET_A)]);
    let source = std::fs::read_to_string(&f.sch).unwrap();
    let at = source.find("(property \"Sheetname\"").unwrap();
    let mut with_ports = source.clone();
    // R1.1 is at (20, 100) and R3.1 at (50, 100): one LINK pin on each.
    with_ports.insert_str(
        at,
        "(pin \"LINK\" input (at 20 100 0) (effects (font (size 1.27 1.27)))) (pin \"LINK\" input (at 50 100 0) (effects (font (size 1.27 1.27)))) ",
    );
    std::fs::write(&f.sch, with_ports).unwrap();
    let model = fixture_model(&f.sch);
    let pin = |r: &str| model.part(r).unwrap().pins[0].net.clone();
    assert_eq!(pin("R1"), pin("R2"));
    assert_eq!(pin("R3"), pin("R2"));
    assert!(model.inventory_complete, "{:?}", model.connectivity_gaps);
    let v = audit_circuit(&f).await;
    assert_eq!(v["connectivity"]["status"], "complete", "{v}");
    assert_eq!(v["status"], "checked");
    let notes = v["connectivity"]["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1, "{v}");
    let note = notes[0].as_str().unwrap();
    assert!(
        note.contains("LINK") && note.contains(&format!("/{SHEET_A}/")),
        "{note}"
    );
    // The review says it too, and stays complete: nothing here is unknown.
    let r = review(&f).await;
    assert_eq!(r["circuit"]["connectivity_notes"], json!([note]), "{r}");
    assert!(!r["diagnostics"].as_array().unwrap().iter().any(|d| {
        d["code"] == "circuit_review_partial" || d["code"] == "circuit_checks_unevaluated"
    }));
}

// ─── Checkpoint A: what the first mutation run left uncaught ─────────────────

// A transceiver that shares one line with a pair is not a node of that pair:
// a termination beside it is still far from the pair's own endpoints.
#[tokio::test]
async fn cross_sheet_termination_placement_ignores_a_transceiver_sharing_one_line() {
    let transceiver = |r, high, low| {
        p(
            r,
            "Fixture:CANXCVR",
            "TCAN",
            vec![
                ("1", "CANH", "bidirectional", high),
                ("2", "CANL", "bidirectional", low),
                ("3", "VCC", "power_in", "+5V"),
                ("4", "GND", "power_in", "GND"),
            ],
        )
    };
    let node = |r: &str, x: f64, high: &str, low: &str| {
        fp(
            r,
            x,
            10.0,
            &[
                ("1", 0.0, 0.0, high),
                ("2", 0.0, 2.0, low),
                ("3", 4.0, 0.0, "+5V"),
                ("4", 4.0, 2.0, "GND"),
            ],
        )
    };
    for (high, low, pads) in [("g:H", "g:X", ("H", "X")), ("g:X", "g:H", ("X", "H"))] {
        let b = board(
            &[
                node("U1", 10.0, "H", "L"),
                fp(
                    "R1",
                    101.0,
                    10.0,
                    &[("1", 0.0, 0.0, "H"), ("2", 0.0, 2.0, "L")],
                ),
                node("U2", 100.0, pads.0, pads.1),
            ],
            "",
        );
        let f = fx(
            &[
                transceiver("U1", "g:H", "g:L"),
                res("R1", "120", "g:H", "g:L"),
                transceiver("U2", high, low),
            ],
            Some(b),
            None,
        );
        let v = audit_circuit(&f).await;
        let far = v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["rule"] == "termination_far_from_endpoint")
            .count();
        assert_eq!(
            far, 1,
            "{high}/{low}: R1 is 91 mm from U1, its only node: {v}"
        );
    }
}

// An unfitted source is known even when the inventory is not, and is reported
// as unfitted. Another part being unfitted says nothing about a source that is
// not seen at all.
#[tokio::test]
async fn cross_sheet_incomplete_inventory_tells_an_unfitted_source_from_an_unseen_one() {
    use crate::tools::circuit_review::tests::rails;
    let supply = |r| {
        p(
            r,
            "Fixture:Supply",
            "Supply",
            vec![
                ("1", "OUT", "power_out", "+3V3"),
                ("2", "GND", "power_in", "GND"),
            ],
        )
        .dnp()
    };
    for (source, expected) in [("U5", (1, 0)), ("U9", (0, 1))] {
        let f = fx(
            &[supply("U5"), res("R1", "1k", "+3V3", "GND")],
            None,
            Some(rails(source, 3.3, &["+3V3"])),
        );
        with_sheets(&f, schematic(&[]), &[("Absent", SHEET_A)]);
        std::fs::remove_file(f.sch.with_file_name("channel.kicad_sch")).unwrap();
        let v = audit_circuit(&f).await;
        assert_eq!(v["connectivity"]["status"], "partial", "{v}");
        let count = |list: &str| {
            v[list]
                .as_array()
                .unwrap()
                .iter()
                .filter(|x| x["rule"] == "rail_source_missing")
                .count()
        };
        assert_eq!(
            (count("findings"), count("unproven")),
            expected,
            "{source}: {v}"
        );
    }
}

// Circuit findings keep the sheet they were found on; every other audit's
// findings are attributed to the file that audit read.
#[test]
fn cross_sheet_only_circuit_findings_keep_their_own_source() {
    let root = Path::new("/scratch/root.kicad_sch");
    let sources = |name: &'static str| {
        let result = CallToolResult::text(
            json!({"findings": [{"rule": "a", "source": "child.kicad_sch"}, {"rule": "b"}]})
                .to_string(),
        );
        let mut aggregate = AuditAggregate::new(name);
        aggregate.record(root, Ok(result), &mut Vec::new());
        aggregate
            .findings
            .iter()
            .map(|f| f["source"].as_str().unwrap_or("none").to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        sources("circuit_design"),
        ["child.kicad_sch", "/scratch/root.kicad_sch"]
    );
    assert_eq!(
        sources("decoupling"),
        ["/scratch/root.kicad_sch", "/scratch/root.kicad_sch"]
    );
}

// ─── Independent review B (2026-11-12) ───────────────────────────────────────

// R-M2b, through both entry points: a sheet symbol with no UUID is one
// unevaluated subject however many labels its child has.
#[tokio::test]
async fn cross_sheet_a_sheet_symbol_with_no_uuid_is_one_unevaluated_subject() {
    let f = fx(&[res("R1", "1k", "LEFT", "RIGHT")], None, None);
    let child = schematic(&[res("R2", "1k", "L1", "L2")])
        .replace("(label \"L1\"", "(hierarchical_label \"L1\" (shape input)")
        .replace("(label \"L2\"", "(hierarchical_label \"L2\" (shape input)");
    with_sheets(&f, child, &[("Child", SHEET_A)]);
    let source = std::fs::read_to_string(&f.sch).unwrap();
    let anonymous = source.replacen(&format!("(uuid \"{SHEET_A}\")"), "", 1);
    assert_ne!(source, anonymous);
    std::fs::write(&f.sch, anonymous).unwrap();
    let v = audit_circuit(&f).await;
    assert_eq!(v["coverage"]["connectivity_gaps"], 1, "{v}");
    assert_eq!(v["coverage"]["unevaluated_subjects"], 1);
    assert_eq!(v["status"], "partial");
    let r = review(&f).await;
    assert_eq!(r["status"], "partial", "{r}");
    assert_eq!(r["circuit"]["unevaluated_subjects"], 1);
}
