//! Tests of the circuit-aware review over scratch schematics and boards.
//!
//! Every check has a case that fires, a case that passes, and a case that
//! reports it could not run. The fixtures build a schematic from a list of
//! parts (each pin labelled with its net at the pin's own end) and a board
//! from footprints with pads at chosen positions, so a distance in a test is
//! a number on the page.

use super::*;
use serde_json::{json, Value};

// ─── Fixtures ────────────────────────────────────────────────────────────────

type PinSpec = (&'static str, &'static str, &'static str, &'static str);

pub(crate) struct P {
    r: &'static str,
    lib: &'static str,
    val: &'static str,
    pins: Vec<PinSpec>,
}

pub(crate) fn p(r: &'static str, lib: &'static str, val: &'static str, pins: Vec<PinSpec>) -> P {
    P { r, lib, val, pins }
}

/// A part with pins `(number, name, electrical type, net)`.
pub(crate) fn cap(r: &'static str, val: &'static str, a: &'static str, b: &'static str) -> P {
    p(
        r,
        "Device:C",
        val,
        vec![("1", "~", "passive", a), ("2", "~", "passive", b)],
    )
}

pub(crate) fn res(r: &'static str, val: &'static str, a: &'static str, b: &'static str) -> P {
    p(
        r,
        "Device:R",
        val,
        vec![("1", "~", "passive", a), ("2", "~", "passive", b)],
    )
}

/// A three-pin IC: supply, ground and one signal pin.
pub(crate) fn mcu(r: &'static str, io_net: &'static str) -> P {
    p(
        r,
        "Fixture:MCU",
        "MCU",
        vec![
            ("1", "VDD", "power_in", "+3V3"),
            ("2", "GND", "power_in", "GND"),
            ("3", "PA0", "input", io_net),
        ],
    )
}

pub(crate) fn schematic(parts: &[P]) -> String {
    let mut libs = String::new();
    let mut seen: Vec<&str> = Vec::new();
    for part in parts {
        if seen.contains(&part.lib) {
            continue;
        }
        seen.push(part.lib);
        let short = part.lib.rsplit(':').next().unwrap();
        let pins: String = part
            .pins
            .iter()
            .enumerate()
            .map(|(k, (num, name, ty, _))| {
                format!(
                    "(pin {ty} line (at 0 {y} 0) (length 2.54) (name \"{name}\") (number \"{num}\"))",
                    y = 2.54 * k as f64
                )
            })
            .collect::<Vec<_>>()
            .join("\n        ");
        libs.push_str(&format!(
            "    (symbol \"{lib}\"\n      (property \"Reference\" \"X\" (at 0 0 0))\n      (property \"Value\" \"X\" (at 0 0 0))\n      (symbol \"{short}_1_1\"\n        {pins}\n      )\n    )\n",
            lib = part.lib
        ));
    }
    let mut body = String::new();
    let mut uuid = 0u32;
    let mut next = || {
        uuid += 1;
        format!("00000000-0000-4000-8000-{uuid:012}")
    };
    for (i, part) in parts.iter().enumerate() {
        let x = 20.0 + 30.0 * i as f64;
        let y = 100.0;
        body.push_str(&format!(
            "  (symbol (lib_id \"{lib}\") (at {x} {y} 0) (unit 1) (in_bom yes) (on_board yes) (dnp no) (uuid \"{u}\")\n    (property \"Reference\" \"{r}\" (at {x} {y} 0))\n    (property \"Value\" \"{v}\" (at {x} {y} 0))\n    (property \"Footprint\" \"\" (at {x} {y} 0))\n  )\n",
            lib = part.lib,
            u = next(),
            r = part.r,
            v = part.val,
        ));
        for (k, (_, _, _, net)) in part.pins.iter().enumerate() {
            let ly = y - 2.54 * k as f64;
            if let Some(name) = net.strip_prefix("g:") {
                body.push_str(&format!(
                    "  (global_label \"{name}\" (shape input) (at {x} {ly} 0) (uuid \"{u}\") (effects (font (size 1.27 1.27))))\n",
                    u = next()
                ));
            } else {
                body.push_str(&format!(
                    "  (label \"{net}\" (at {x} {ly} 0) (uuid \"{u}\"))\n",
                    u = next()
                ));
            }
        }
    }
    format!(
        "(kicad_sch\n  (version 20250610)\n  (generator \"konnect\")\n  (generator_version \"10.0\")\n  (uuid \"11111111-1111-4111-8111-111111111111\")\n  (paper \"A4\")\n  (lib_symbols\n{libs}  )\n{body})\n"
    )
}

/// A footprint at `(x, y)` with pads `(number, dx, dy, net)`.
pub(crate) fn fp(r: &str, x: f64, y: f64, pads: &[(&str, f64, f64, &str)]) -> String {
    let pads: String = pads
        .iter()
        .map(|(n, dx, dy, net)| {
            format!(
                "\n    (pad \"{n}\" smd rect (at {dx} {dy} 0) (size 1 1) (layers \"F.Cu\" \"F.Mask\" \"F.Paste\") (net \"{net}\") (uuid \"pad-{r}-{n}\"))"
            )
        })
        .collect();
    format!(
        "  (footprint \"Fixture:{r}\" (layer \"F.Cu\") (uuid \"fp-{r}\") (at {x} {y} 0)\n    (property \"Reference\" \"{r}\" (at 0 -3 0) (layer \"F.SilkS\") (uuid \"ref-{r}\") (effects (font (size 1 1)))){pads})\n"
    )
}

pub(crate) fn board(items: &[String], extra: &str) -> String {
    format!(
        "(kicad_pcb (version 20260206) (generator \"pcbnew\")\n  (layers (0 \"F.Cu\" signal) (2 \"B.Cu\" signal) (25 \"Edge.Cuts\" user))\n  (gr_rect (start 0 0) (end 120 80) (stroke (width 0.05) (type solid)) (fill no) (layer \"Edge.Cuts\") (uuid \"edge\"))\n{}{extra}\n)\n",
        items.concat()
    )
}

pub(crate) struct Fx {
    pub(crate) _dir: tempfile::TempDir,
    pub(crate) sch: std::path::PathBuf,
    pub(crate) board: Option<std::path::PathBuf>,
}

pub(crate) fn fx(parts: &[P], board_text: Option<String>, intent: Option<Value>) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let sch = dir.path().join("demo.kicad_sch");
    std::fs::write(&sch, schematic(parts)).unwrap();
    std::fs::write(dir.path().join("demo.kicad_pro"), "{}").unwrap();
    let board = board_text.map(|text| {
        let path = dir.path().join("demo.kicad_pcb");
        std::fs::write(&path, text).unwrap();
        path
    });
    if let Some(intent) = intent {
        let path = design_intent_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_string_pretty(&intent).unwrap()).unwrap();
    }
    Fx {
        _dir: dir,
        sch,
        board,
    }
}

fn run(f: &Fx) -> Value {
    let result = audit_files(
        &f.sch,
        std::slice::from_ref(&f.sch),
        f.board.as_deref(),
        vec![],
    )
    .unwrap();
    assert!(!result.is_error, "{result:?}");
    match result.content.first() {
        Some(crate::mcp::protocol::ToolContent::Text { text }) => {
            serde_json::from_str(text).unwrap()
        }
        other => panic!("expected text, got {other:?}"),
    }
}

fn findings<'a>(v: &'a Value, rule: &str) -> Vec<&'a Value> {
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["rule"] == rule)
        .collect()
}

fn check<'a>(v: &'a Value, rule: &str) -> &'a Value {
    v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["rule"] == rule)
        .unwrap_or_else(|| panic!("no check {rule}"))
}

/// The one finding of `rule`, with the fields every finding must carry.
fn only<'a>(v: &'a Value, rule: &str) -> &'a Value {
    let all = findings(v, rule);
    assert_eq!(all.len(), 1, "{rule}: {all:?}\n{}", v["findings"]);
    let f = all[0];
    assert_eq!(f["rule"], rule);
    assert!(
        f["checklist"].as_str().is_some_and(|c| !c.is_empty()),
        "a finding names its checklist item: {f}"
    );
    assert!(f["category"] == "circuit" && f["issue"].is_string() && f["evidence"].is_object());
    f
}

// ─── The table ───────────────────────────────────────────────────────────────

#[test]
fn every_rule_has_a_checklist_item_and_an_id_once() {
    let mut ids: Vec<&str> = RULES.iter().map(|r| r.id).collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n, "rule ids are unique");
    for r in RULES {
        assert!(!r.checklist.is_empty() && !r.title.is_empty(), "{}", r.id);
    }
    assert!(NEEDS_HUMAN.len() >= 10);
}

#[test]
fn an_empty_design_runs_nothing_and_passes_nothing() {
    let f = fx(&[res("R1", "10k", "A", "B")], None, None);
    let v = run(&f);
    assert!(v["findings"].as_array().unwrap().is_empty());
    for c in v["checks"].as_array().unwrap() {
        let status = c["status"].as_str().unwrap();
        assert!(
            matches!(status, "no_subjects" | "not_run"),
            "{c}: nothing to examine must not read as ran"
        );
        assert!(c["passed"].is_null() || c["passed"] == 0);
    }
    assert_eq!(check(&v, "decoupling_supply_far")["status"], "not_run");
    assert_eq!(check(&v, "decoupling_supply_far")["reason"], "no_board");
    assert_eq!(check(&v, "uart_tx_rx_crossed")["status"], "no_subjects");
    assert_eq!(
        check(&v, "rail_not_in_schematic")["reason"],
        "no_design_intent"
    );
    assert!(!v["needs_human"].as_array().unwrap().is_empty());
    assert!(v["summary"]
        .as_str()
        .unwrap()
        .contains("Nothing here passes a board"));
}

pub(crate) fn cap_for_tests() -> P {
    cap("C1", "100n", "+3V3", "GND")
}

// ─── Decoupling ──────────────────────────────────────────────────────────────

fn decoupled_parts() -> Vec<P> {
    vec![mcu("U1", "SIG0"), cap("C1", "100n", "+3V3", "GND")]
}

fn u1() -> String {
    fp(
        "U1",
        10.0,
        10.0,
        &[
            ("1", 0.0, 0.0, "+3V3"),
            ("2", 2.0, 0.0, "GND"),
            ("3", 4.0, 0.0, "SIG0"),
        ],
    )
}

#[test]
fn a_decoupling_cap_far_from_its_supply_pin_fires_with_the_limit_and_the_rule() {
    let b = board(
        &[
            u1(),
            fp(
                "C1",
                30.0,
                10.0,
                &[("1", 0.0, 0.0, "+3V3"), ("2", 1.0, 0.0, "GND")],
            ),
        ],
        "",
    );
    let v = run(&fx(&decoupled_parts(), Some(b), None));
    let f = only(&v, "decoupling_supply_far");
    assert_eq!(f["checklist"], "2 Placement: IC decoupling capacitor");
    assert_eq!(f["evidence"]["limit_mm"], 2.5);
    assert_eq!(f["evidence"]["distance_mm"], 20.0);
    assert_eq!(f["evidence"]["capacitor"], "C1");
    assert_eq!(check(&v, "decoupling_supply_far")["status"], "ran");
}

#[test]
fn a_decoupling_cap_beside_the_pin_with_a_short_ground_leg_passes_both_legs() {
    let b = board(
        &[
            u1(),
            fp(
                "C1",
                10.0,
                12.0,
                &[("1", 0.0, 0.0, "+3V3"), ("2", 1.0, 0.0, "GND")],
            ),
        ],
        "",
    );
    let v = run(&fx(&decoupled_parts(), Some(b), None));
    assert!(
        findings(&v, "decoupling_supply_far").is_empty(),
        "{}",
        v["findings"]
    );
    assert!(findings(&v, "decoupling_ground_far").is_empty());
    assert_eq!(check(&v, "decoupling_supply_far")["passed"], 1);
    assert_eq!(check(&v, "decoupling_ground_far")["passed"], 1);
}

#[test]
fn a_short_supply_leg_with_a_long_ground_leg_fires_the_ground_rule_only() {
    let b = board(
        &[
            u1(),
            fp(
                "C1",
                10.0,
                12.0,
                &[("1", 0.0, 0.0, "+3V3"), ("2", 6.0, 0.0, "GND")],
            ),
        ],
        "",
    );
    let v = run(&fx(&decoupled_parts(), Some(b), None));
    assert!(findings(&v, "decoupling_supply_far").is_empty());
    let f = only(&v, "decoupling_ground_far");
    assert!(f["evidence"]["distance_mm"].as_f64().unwrap() > 4.0);
    assert_eq!(f["evidence"]["ground_net"], "GND");
}

#[test]
fn without_a_board_the_decoupling_rules_are_not_run_and_say_why() {
    let v = run(&fx(&decoupled_parts(), None, None));
    for rule in ["decoupling_supply_far", "decoupling_ground_far"] {
        assert_eq!(check(&v, rule)["status"], "not_run");
        assert_eq!(check(&v, rule)["reason"], "no_board");
    }
    assert_eq!(v["board"], false);
}

#[test]
fn a_part_missing_from_the_board_is_an_unevaluated_gap_not_a_pass() {
    // The cap is not on the board at all.
    let v = run(&fx(&decoupled_parts(), Some(board(&[u1()], "")), None));
    let c = check(&v, "decoupling_supply_far");
    assert_eq!(c["unevaluated"], 1, "{c}");
    assert_eq!(c["passed"], 0);
    assert!(c["unevaluated_reasons"][0]
        .as_str()
        .unwrap()
        .contains("C1 is not on the board"));
    assert_eq!(v["status"], "partial");
    assert_eq!(v["coverage"]["unevaluated_subjects"], 1);
}

#[test]
fn a_cap_value_outside_the_families_is_skipped_not_judged() {
    let parts = vec![mcu("U1", "SIG0"), cap("C1", "4n7", "+3V3", "GND")];
    let b = board(
        &[
            u1(),
            fp(
                "C1",
                50.0,
                10.0,
                &[("1", 0.0, 0.0, "+3V3"), ("2", 1.0, 0.0, "GND")],
            ),
        ],
        "",
    );
    let v = run(&fx(&parts, Some(b), None));
    assert!(findings(&v, "decoupling_supply_far").is_empty());
    assert_eq!(check(&v, "decoupling_supply_far")["skipped"], 1);
    assert_eq!(check(&v, "decoupling_supply_far")["status"], "no_subjects");
}

// ─── Bulk capacitor ──────────────────────────────────────────────────────────

fn regulator(r: &'static str, out: &'static str) -> P {
    p(
        r,
        "Regulator_Linear:AMS1117-3.3",
        "AMS1117-3.3",
        vec![
            ("1", "GND", "power_in", "GND"),
            ("2", "VOUT", "power_out", out),
            ("3", "VIN", "power_in", "+5V"),
        ],
    )
}

#[test]
fn a_bulk_cap_far_from_a_regulator_pin_fires_and_a_near_one_passes() {
    let parts = vec![regulator("U2", "+3V3"), cap("C2", "22u", "+5V", "GND")];
    let u2 = fp(
        "U2",
        10.0,
        10.0,
        &[
            ("1", 0.0, 0.0, "GND"),
            ("2", 3.0, 0.0, "+3V3"),
            ("3", 6.0, 0.0, "+5V"),
        ],
    );
    let far = board(
        &[
            u2.clone(),
            fp(
                "C2",
                60.0,
                10.0,
                &[("1", 0.0, 0.0, "+5V"), ("2", 1.0, 0.0, "GND")],
            ),
        ],
        "",
    );
    let v = run(&fx(&parts, Some(far), None));
    let f = only(&v, "bulk_cap_far");
    assert_eq!(f["checklist"], "2 Placement: local bulk capacitor");
    assert_eq!(f["evidence"]["limit_mm"], 10.0);

    let near = board(
        &[
            u2,
            fp(
                "C2",
                16.0,
                12.0,
                &[("1", 0.0, 0.0, "+5V"), ("2", 1.0, 0.0, "GND")],
            ),
        ],
        "",
    );
    let v = run(&fx(&parts, Some(near), None));
    assert!(findings(&v, "bulk_cap_far").is_empty(), "{}", v["findings"]);
    assert_eq!(check(&v, "bulk_cap_far")["passed"], 1);
}

// ─── Protection at the connector ─────────────────────────────────────────────

fn connector(r: &'static str, net: &'static str) -> P {
    p(
        r,
        "Connector_Generic:Conn_01x02",
        "Conn",
        vec![
            ("1", "Pin_1", "passive", net),
            ("2", "Pin_2", "passive", "GND"),
        ],
    )
}

fn tvs(r: &'static str, net: &'static str) -> P {
    p(
        r,
        "Device:D_TVS",
        "PESD5V0",
        vec![("1", "K", "passive", net), ("2", "A", "passive", "GND")],
    )
}

fn receiver(net: &'static str) -> P {
    mcu("U1", net)
}

#[test]
fn a_connector_signal_with_no_protection_is_one_grouped_info_finding() {
    let parts = vec![connector("J1", "SIG"), receiver("SIG")];
    let v = run(&fx(&parts, None, None));
    let f = only(&v, "connector_signal_unprotected");
    assert_eq!(f["severity"], "info");
    assert_eq!(
        f["checklist"],
        "9 External connections: exposed signals pass the protection first"
    );
    assert_eq!(f["evidence"]["nets"], json!(["SIG"]));
}

#[test]
fn an_unprotected_net_of_a_recorded_interface_is_a_warning() {
    let parts = vec![connector("J1", "SIG"), receiver("SIG")];
    let intent = json!({"schema_version": 1, "interfaces": [{"name": "sig", "kind": "gpio", "nets": ["SIG"]}]});
    let v = run(&fx(&parts, None, Some(intent)));
    assert_eq!(
        only(&v, "connector_signal_unprotected")["severity"],
        "warning"
    );
}

#[test]
fn a_connector_net_that_continues_on_another_sheet_is_unproven_not_a_finding() {
    let parts = vec![connector("J1", "g:SIG"), receiver("g:SIG")];
    let v = run(&fx(&parts, None, None));
    assert!(findings(&v, "connector_signal_unprotected").is_empty());
    let u = v["unproven"].as_array().unwrap();
    assert!(
        u.iter()
            .any(|x| x["rule"] == "connector_signal_unprotected"),
        "{u:?}"
    );
}

#[test]
fn a_protected_connector_net_passes_the_presence_rule() {
    let parts = vec![connector("J1", "SIG"), tvs("D1", "SIG"), receiver("SIG")];
    let v = run(&fx(&parts, None, None));
    assert!(findings(&v, "connector_signal_unprotected").is_empty());
    assert_eq!(check(&v, "connector_signal_unprotected")["passed"], 1);
}

fn protection_board(d1_x: f64, u1_x: f64) -> String {
    board(
        &[
            fp(
                "J1",
                5.0,
                5.0,
                &[("1", 0.0, 0.0, "SIG"), ("2", 2.0, 0.0, "GND")],
            ),
            fp(
                "D1",
                d1_x,
                5.0,
                &[("1", 0.0, 0.0, "SIG"), ("2", 1.0, 1.0, "GND")],
            ),
            fp(
                "U1",
                u1_x,
                5.0,
                &[
                    ("1", 0.0, 3.0, "+3V3"),
                    ("2", 0.0, 6.0, "GND"),
                    ("3", 0.0, 0.0, "SIG"),
                ],
            ),
        ],
        "",
    )
}

#[test]
fn a_tvs_far_from_the_connector_fires_and_one_at_the_pad_passes() {
    let parts = vec![connector("J1", "SIG"), tvs("D1", "SIG"), receiver("SIG")];
    let far = run(&fx(&parts, Some(protection_board(40.0, 60.0)), None));
    let f = only(&far, "esd_far_from_connector");
    assert_eq!(f["checklist"], "2 Placement: TVS/ESD protection");
    assert_eq!(f["evidence"]["limit_mm"], 10.0);
    let near = run(&fx(&parts, Some(protection_board(7.0, 60.0)), None));
    assert!(findings(&near, "esd_far_from_connector").is_empty());
    assert_eq!(check(&near, "esd_far_from_connector")["passed"], 1);
}

#[test]
fn a_circuit_pin_nearer_the_connector_than_its_protection_fires() {
    let parts = vec![connector("J1", "SIG"), tvs("D1", "SIG"), receiver("SIG")];
    // The IC is 3 mm from the connector pad, the TVS 9 mm.
    let v = run(&fx(&parts, Some(protection_board(14.0, 8.0)), None));
    let f = only(&v, "protection_beyond_circuit");
    assert_eq!(
        f["checklist"],
        "9 External connections: exposed signals pass the protection first"
    );
    assert!(f["evidence"]["note"]
        .as_str()
        .unwrap()
        .contains("not trace order"));
    // Protection between connector and circuit passes.
    let ok = run(&fx(&parts, Some(protection_board(8.0, 20.0)), None));
    assert!(findings(&ok, "protection_beyond_circuit").is_empty());
    assert_eq!(check(&ok, "protection_beyond_circuit")["passed"], 1);
}

#[test]
fn protection_behind_a_series_resistor_leaves_the_connector_pin_exposed() {
    let parts = vec![
        connector("J1", "SIG"),
        res("R1", "100", "SIG", "SIGP"),
        tvs("D1", "SIGP"),
        receiver("SIGP"),
    ];
    let v = run(&fx(&parts, None, None));
    let f = only(&v, "protection_after_series_element");
    assert_eq!(f["evidence"]["protection"], "D1");
    assert!(findings(&v, "connector_signal_unprotected").is_empty());
}

// ─── CAN and RS-485 ──────────────────────────────────────────────────────────

pub(crate) fn can_xcvr() -> P {
    p(
        "U2",
        "Fixture:CANXCVR",
        "TCAN",
        vec![
            ("1", "CANH", "bidirectional", "CANH"),
            ("2", "CANL", "bidirectional", "CANL"),
            ("3", "VCC", "power_in", "+5V"),
            ("4", "GND", "power_in", "GND"),
            ("5", "TXD", "input", "TX"),
            ("6", "RXD", "output", "RX"),
        ],
    )
}

pub(crate) fn can_conn() -> P {
    p(
        "J2",
        "Connector_Generic:Conn_01x03",
        "Conn",
        vec![
            ("1", "H", "passive", "CANH"),
            ("2", "L", "passive", "CANL"),
            ("3", "G", "passive", "GND"),
        ],
    )
}

fn can_board(j2_x: f64, r1_at: (f64, f64)) -> String {
    board(
        &[
            fp(
                "U2",
                20.0,
                10.0,
                &[
                    ("1", 0.0, 0.0, "CANH"),
                    ("2", 2.0, 0.0, "CANL"),
                    ("3", 4.0, 0.0, "+5V"),
                    ("4", 6.0, 0.0, "GND"),
                    ("5", 8.0, 0.0, "TX"),
                    ("6", 10.0, 0.0, "RX"),
                ],
            ),
            fp(
                "J2",
                j2_x,
                10.0,
                &[
                    ("1", 0.0, 0.0, "CANH"),
                    ("2", 2.0, 0.0, "CANL"),
                    ("3", 4.0, 0.0, "GND"),
                ],
            ),
            fp(
                "R1",
                r1_at.0,
                r1_at.1,
                &[("1", 0.0, 0.0, "CANH"), ("2", 0.0, 2.0, "CANL")],
            ),
        ],
        "",
    )
}

#[test]
fn a_can_termination_of_120_ohm_passes_and_100_ohm_fires() {
    let ok = run(&fx(
        &[can_xcvr(), can_conn(), res("R1", "120", "CANH", "CANL")],
        None,
        None,
    ));
    assert!(findings(&ok, "can_termination_value").is_empty());
    assert_eq!(check(&ok, "can_termination_value")["passed"], 1);
    let iface = &ok["interfaces"][0];
    assert_eq!(iface["interface"], "can");
    assert_eq!(iface["termination"]["kind"], "direct");

    let bad = run(&fx(
        &[can_xcvr(), can_conn(), res("R1", "100", "CANH", "CANL")],
        None,
        None,
    ));
    let f = only(&bad, "can_termination_value");
    assert_eq!(f["checklist"], "6 Interfaces: CAN");
    assert_eq!(f["evidence"]["total_ohms"], 100.0);
}

#[test]
fn a_split_termination_of_two_60_ohm_resistors_is_read_as_120() {
    let parts = vec![
        can_xcvr(),
        can_conn(),
        res("R1", "60.4", "CANH", "MID"),
        res("R2", "60.4", "CANL", "MID"),
        cap("C1", "4n7", "MID", "GND"),
    ];
    let v = run(&fx(&parts, None, None));
    assert!(
        findings(&v, "can_termination_value").is_empty(),
        "{}",
        v["findings"]
    );
    assert_eq!(v["interfaces"][0]["termination"]["kind"], "split");
    assert_eq!(check(&v, "can_termination_value")["passed"], 1);
}

#[test]
fn a_jumper_selectable_termination_is_found_and_judged() {
    let parts = vec![
        can_xcvr(),
        can_conn(),
        res("R1", "120", "CANH", "X"),
        p(
            "JP1",
            "Jumper:Jumper_2_Open",
            "Jumper",
            vec![("1", "A", "passive", "X"), ("2", "B", "passive", "CANL")],
        ),
    ];
    let v = run(&fx(&parts, None, None));
    assert_eq!(v["interfaces"][0]["termination"]["kind"], "selectable");
    assert_eq!(check(&v, "can_termination_value")["passed"], 1);
}

#[test]
fn no_termination_is_reported_as_a_fact_not_a_finding_and_the_bus_ends_are_left_to_a_human() {
    let v = run(&fx(&[can_xcvr(), can_conn()], None, None));
    assert!(findings(&v, "can_termination_value").is_empty());
    assert_eq!(check(&v, "can_termination_value")["status"], "no_subjects");
    assert_eq!(
        v["interfaces"][0]["termination"]["kind"],
        "none_on_this_sheet"
    );
    assert!(v["interfaces"][0]["physical_ends"]
        .as_str()
        .unwrap()
        .contains("needs_human"));
    assert!(v["needs_human"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["checklist"] == "6 CAN"));
}

#[test]
fn three_connectors_on_one_can_line_is_a_star_not_a_linear_bus() {
    let extra = |r: &'static str| {
        p(
            r,
            "Connector_Generic:Conn_01x03",
            "Conn",
            vec![
                ("1", "H", "passive", "CANH"),
                ("2", "L", "passive", "CANL"),
                ("3", "G", "passive", "GND"),
            ],
        )
    };
    let v = run(&fx(
        &[can_xcvr(), can_conn(), extra("J3"), extra("J4")],
        None,
        None,
    ));
    let f = only(&v, "can_bus_branches");
    assert_eq!(f["evidence"]["connectors"], json!(["J2", "J3", "J4"]));
    let ok = run(&fx(&[can_xcvr(), can_conn(), extra("J3")], None, None));
    assert!(findings(&ok, "can_bus_branches").is_empty());
}

#[test]
fn a_can_transceiver_far_from_its_connector_fires_and_a_near_one_passes() {
    let parts = vec![can_xcvr(), can_conn(), res("R1", "120", "CANH", "CANL")];
    let far = run(&fx(&parts, Some(can_board(90.0, (22.0, 14.0))), None));
    let f = only(&far, "transceiver_far_from_connector");
    assert_eq!(f["checklist"], "2 Placement: CAN/RS-485 transceiver");
    assert_eq!(f["evidence"]["limit_mm"], 25.0);
    let near = run(&fx(&parts, Some(can_board(8.0, (22.0, 14.0))), None));
    assert!(
        findings(&near, "transceiver_far_from_connector").is_empty(),
        "{}",
        near["findings"]
    );
    assert_eq!(check(&near, "transceiver_far_from_connector")["passed"], 1);
}

#[test]
fn a_termination_far_from_the_connector_and_the_transceiver_fires() {
    let parts = vec![can_xcvr(), can_conn(), res("R1", "120", "CANH", "CANL")];
    let far = run(&fx(&parts, Some(can_board(8.0, (60.0, 60.0))), None));
    let f = only(&far, "termination_far_from_endpoint");
    assert_eq!(f["checklist"], "2 Placement: bus-end termination");
    let near = run(&fx(&parts, Some(can_board(8.0, (22.0, 14.0))), None));
    assert!(findings(&near, "termination_far_from_endpoint").is_empty());
    assert_eq!(check(&near, "termination_far_from_endpoint")["passed"], 1);
}

fn rs485_xcvr() -> P {
    p(
        "U3",
        "Fixture:RS485",
        "MAX3485",
        vec![
            ("1", "A", "bidirectional", "A"),
            ("2", "B", "bidirectional", "B"),
            ("3", "DE", "input", "DIR"),
            ("4", "~{RE}", "input", "DIR"),
            ("5", "VCC", "power_in", "+3V3"),
            ("6", "GND", "power_in", "GND"),
        ],
    )
}

#[test]
fn rs485_without_bias_is_an_info_finding_and_with_bias_passes() {
    let v = run(&fx(&[rs485_xcvr(), res("R1", "120", "A", "B")], None, None));
    let f = only(&v, "rs485_failsafe_bias");
    assert_eq!(f["severity"], "info");
    assert_eq!(f["checklist"], "6 Interfaces: RS-485");
    assert_eq!(check(&v, "rs485_termination_value")["passed"], 1);

    let ok = run(&fx(
        &[
            rs485_xcvr(),
            res("R1", "120", "A", "B"),
            res("R2", "680", "A", "+3V3"),
            res("R3", "680", "B", "GND"),
        ],
        None,
        None,
    ));
    assert!(findings(&ok, "rs485_failsafe_bias").is_empty());
    assert_eq!(check(&ok, "rs485_failsafe_bias")["passed"], 1);
}

#[test]
fn an_rs485_bus_that_continues_on_another_sheet_leaves_the_bias_unproven() {
    let parts = vec![p(
        "U3",
        "Fixture:RS485",
        "MAX3485",
        vec![
            ("1", "A", "bidirectional", "g:A"),
            ("2", "B", "bidirectional", "g:B"),
            ("3", "DE", "input", "DIR"),
            ("4", "~{RE}", "input", "DIR"),
            ("5", "VCC", "power_in", "+3V3"),
            ("6", "GND", "power_in", "GND"),
        ],
    )];
    let v = run(&fx(&parts, None, None));
    assert!(findings(&v, "rs485_failsafe_bias").is_empty());
    assert!(v["unproven"]
        .as_array()
        .unwrap()
        .iter()
        .any(|u| u["rule"] == "rs485_failsafe_bias"));
}

// ─── Series termination ──────────────────────────────────────────────────────

fn series_parts() -> Vec<P> {
    vec![
        p(
            "U1",
            "Fixture:DRV",
            "DRV",
            vec![
                ("1", "OUT", "output", "A"),
                ("2", "GND", "power_in", "GND"),
                ("3", "VDD", "power_in", "+3V3"),
            ],
        ),
        res("R1", "33", "A", "B"),
        p(
            "U2",
            "Fixture:RCV",
            "RCV",
            vec![
                ("1", "IN", "input", "B"),
                ("2", "GND", "power_in", "GND"),
                ("3", "VDD", "power_in", "+3V3"),
            ],
        ),
    ]
}

fn series_board(r1_x: f64) -> String {
    board(
        &[
            fp(
                "U1",
                10.0,
                10.0,
                &[
                    ("1", 0.0, 0.0, "A"),
                    ("2", 2.0, 0.0, "GND"),
                    ("3", 4.0, 0.0, "+3V3"),
                ],
            ),
            fp(
                "R1",
                r1_x,
                10.0,
                &[("1", 0.0, 0.0, "A"), ("2", 1.0, 0.0, "B")],
            ),
            fp(
                "U2",
                60.0,
                10.0,
                &[
                    ("1", 0.0, 0.0, "B"),
                    ("2", 2.0, 0.0, "GND"),
                    ("3", 4.0, 0.0, "+3V3"),
                ],
            ),
        ],
        "",
    )
}

#[test]
fn a_series_resistor_beside_the_receiver_fires_and_beside_the_driver_passes() {
    let far = run(&fx(&series_parts(), Some(series_board(58.0)), None));
    let f = only(&far, "series_termination_far_from_driver");
    assert_eq!(f["checklist"], "2 Placement: series termination resistor");
    assert_eq!(f["evidence"]["driver"], "U1.1");
    let near = run(&fx(&series_parts(), Some(series_board(12.0)), None));
    assert!(findings(&near, "series_termination_far_from_driver").is_empty());
    assert_eq!(
        check(&near, "series_termination_far_from_driver")["passed"],
        1
    );
}

#[test]
fn a_resistor_whose_direction_cannot_be_proven_is_not_judged() {
    // Both sides bidirectional: no driver can be named.
    let parts = vec![
        p(
            "U1",
            "Fixture:BI",
            "BI",
            vec![
                ("1", "IO", "bidirectional", "A"),
                ("2", "GND", "power_in", "GND"),
                ("3", "VDD", "power_in", "+3V3"),
            ],
        ),
        res("R1", "33", "A", "B"),
        p(
            "U2",
            "Fixture:BI",
            "BI",
            vec![
                ("1", "IO", "bidirectional", "B"),
                ("2", "GND", "power_in", "GND"),
                ("3", "VDD", "power_in", "+3V3"),
            ],
        ),
    ];
    let v = run(&fx(&parts, Some(series_board(58.0)), None));
    assert_eq!(
        check(&v, "series_termination_far_from_driver")["status"],
        "no_subjects"
    );
}

// ─── Gate drive ──────────────────────────────────────────────────────────────

fn half_bridge(source_net: &'static str) -> Vec<P> {
    vec![
        p(
            "U1",
            "Driver_FET:IR2104",
            "IR2104",
            vec![
                ("1", "VCC", "power_in", "+12V"),
                ("2", "IN", "input", "PWM"),
                ("3", "COM", "power_in", "GND"),
                ("4", "LO", "output", "LO"),
                ("5", "VS", "passive", "SW"),
                ("6", "HO", "output", "HO"),
                ("7", "VB", "power_in", "VB"),
            ],
        ),
        res("R1", "10", "HO", "GH"),
        p(
            "Q1",
            "Device:Q_NMOS",
            "NMOS",
            vec![
                ("1", "G", "input", "GH"),
                ("2", "D", "passive", "+12V"),
                ("3", "S", "passive", source_net),
            ],
        ),
        cap("C1", "100n", "VB", "SW"),
    ]
}

fn half_bridge_board(u1_x: f64, r1_x: f64, c1_x: f64) -> String {
    board(
        &[
            fp(
                "U1",
                u1_x,
                10.0,
                &[
                    ("1", 0.0, 0.0, "+12V"),
                    ("2", 1.0, 0.0, "PWM"),
                    ("3", 2.0, 0.0, "GND"),
                    ("4", 3.0, 0.0, "LO"),
                    ("5", 4.0, 0.0, "SW"),
                    ("6", 5.0, 0.0, "HO"),
                    ("7", 6.0, 0.0, "VB"),
                ],
            ),
            fp(
                "R1",
                r1_x,
                20.0,
                &[("1", 0.0, 0.0, "HO"), ("2", 1.0, 0.0, "GH")],
            ),
            fp(
                "Q1",
                60.0,
                20.0,
                &[
                    ("1", 0.0, 0.0, "GH"),
                    ("2", 3.0, 0.0, "+12V"),
                    ("3", 6.0, 0.0, "SW"),
                ],
            ),
            fp(
                "C1",
                c1_x,
                12.0,
                &[("1", 0.0, 0.0, "VB"), ("2", 1.0, 0.0, "SW")],
            ),
        ],
        "",
    )
}

#[test]
fn a_gate_driver_far_from_its_mosfet_fires_and_a_close_one_passes() {
    let far = run(&fx(
        &half_bridge("SW"),
        Some(half_bridge_board(10.0, 58.0, 14.0)),
        None,
    ));
    let f = only(&far, "gate_driver_far_from_mosfets");
    assert_eq!(
        f["checklist"],
        "2 Placement: MOSFET gate driver / gate resistor"
    );
    assert_eq!(f["evidence"]["limit_mm"], 15.0);
    let near = run(&fx(
        &half_bridge("SW"),
        Some(half_bridge_board(50.0, 58.0, 54.0)),
        None,
    ));
    assert!(
        findings(&near, "gate_driver_far_from_mosfets").is_empty(),
        "{}",
        near["findings"]
    );
    assert_eq!(check(&near, "gate_driver_far_from_mosfets")["passed"], 1);
}

#[test]
fn a_gate_resistor_away_from_the_gate_fires_and_one_beside_it_passes() {
    let far = run(&fx(
        &half_bridge("SW"),
        Some(half_bridge_board(50.0, 20.0, 54.0)),
        None,
    ));
    let f = only(&far, "gate_resistor_far_from_gate");
    assert_eq!(f["evidence"]["limit_mm"], 5.0);
    let near = run(&fx(
        &half_bridge("SW"),
        Some(half_bridge_board(50.0, 58.0, 54.0)),
        None,
    ));
    assert!(findings(&near, "gate_resistor_far_from_gate").is_empty());
    assert_eq!(check(&near, "gate_resistor_far_from_gate")["passed"], 1);
}

#[test]
fn a_bootstrap_cap_missing_far_or_beside_the_pins() {
    let mut no_cap = half_bridge("SW");
    no_cap.pop();
    let v = run(&fx(&no_cap, None, None));
    let f = only(&v, "bootstrap_cap_missing");
    assert_eq!(
        f["checklist"],
        "8 Power stage: bootstrap / charge-pump capacitors"
    );

    let far = run(&fx(
        &half_bridge("SW"),
        Some(half_bridge_board(50.0, 58.0, 5.0)),
        None,
    ));
    let f = only(&far, "bootstrap_cap_far");
    assert_eq!(f["evidence"]["limit_mm"], 5.0);
    let near = run(&fx(
        &half_bridge("SW"),
        Some(half_bridge_board(50.0, 58.0, 54.0)),
        None,
    ));
    assert!(
        findings(&near, "bootstrap_cap_far").is_empty(),
        "{}",
        near["findings"]
    );
    assert_eq!(check(&near, "bootstrap_cap_missing")["passed"], 1);
    assert_eq!(check(&near, "bootstrap_cap_far")["passed"], 1);
}

#[test]
fn a_bootstrap_whose_nets_leave_the_sheet_is_unproven_when_the_cap_is_absent() {
    let mut parts = half_bridge("SW");
    parts[0].pins[6].3 = "g:VB";
    parts.pop();
    let v = run(&fx(&parts, None, None));
    assert!(findings(&v, "bootstrap_cap_missing").is_empty());
    assert!(v["unproven"]
        .as_array()
        .unwrap()
        .iter()
        .any(|u| u["rule"] == "bootstrap_cap_missing"));
}

#[test]
fn a_gate_driver_returning_off_the_source_net_fires_and_a_shared_return_passes() {
    let ok = run(&fx(&half_bridge("SW"), None, None));
    assert!(findings(&ok, "gate_drive_return_net").is_empty());
    assert_eq!(check(&ok, "gate_drive_return_net")["passed"], 1);
    let bad = run(&fx(&half_bridge("SRC"), None, None));
    let f = only(&bad, "gate_drive_return_net");
    assert_eq!(
        f["checklist"],
        "8 Power stage: gate drive with its own return"
    );
    assert_eq!(f["evidence"]["source_net"], "SRC");
}

// ─── Current sense ───────────────────────────────────────────────────────────

fn sense_parts() -> Vec<P> {
    vec![
        p(
            "U4",
            "Amplifier_Current:INA180",
            "INA180",
            vec![
                ("1", "IN+", "input", "SP"),
                ("2", "IN-", "input", "SN"),
                ("3", "OUT", "output", "ISENSE"),
                ("4", "GND", "power_in", "GND"),
                ("5", "V+", "power_in", "+3V3"),
            ],
        ),
        res("R9", "5m", "SP", "SN"),
        p(
            "L1",
            "Connector_Generic:Conn_01x02",
            "Load",
            vec![("1", "P", "passive", "SP"), ("2", "G", "passive", "GND")],
        ),
    ]
}

fn sense_board(amp_at: (f64, f64), extra: &str) -> String {
    board(
        &[
            fp(
                "R9",
                10.0,
                10.0,
                &[("1", 0.0, 0.0, "SP"), ("2", 4.0, 0.0, "SN")],
            ),
            fp(
                "U4",
                amp_at.0,
                amp_at.1,
                &[
                    ("1", 0.0, 0.0, "SP"),
                    ("2", 2.0, 0.0, "SN"),
                    ("3", 4.0, 0.0, "ISENSE"),
                    ("4", 6.0, 0.0, "GND"),
                    ("5", 8.0, 0.0, "+3V3"),
                ],
            ),
            fp(
                "L1",
                4.0,
                10.0,
                &[("1", 0.0, 0.0, "SP"), ("2", 0.0, 4.0, "GND")],
            ),
        ],
        extra,
    )
}

fn track(a: (f64, f64), b: (f64, f64), net: &str, id: &str) -> String {
    format!(
        "  (segment (start {} {}) (end {} {}) (width 0.25) (layer \"F.Cu\") (net \"{net}\") (uuid \"{id}\"))\n",
        a.0, a.1, b.0, b.1
    )
}

#[test]
fn a_sense_amplifier_far_from_its_shunt_fires_and_a_close_one_passes() {
    let far = run(&fx(
        &sense_parts(),
        Some(sense_board((60.0, 40.0), "")),
        None,
    ));
    let f = only(&far, "sense_amp_far_from_shunt");
    assert_eq!(f["checklist"], "2 Placement: current-sense amplifier");
    let near = run(&fx(
        &sense_parts(),
        Some(sense_board((12.0, 14.0), "")),
        None,
    ));
    assert!(
        findings(&near, "sense_amp_far_from_shunt").is_empty(),
        "{}",
        near["findings"]
    );
    assert_eq!(check(&near, "sense_amp_far_from_shunt")["passed"], 1);
}

#[test]
fn a_sense_trace_straight_from_the_shunt_pad_is_kelvin() {
    let copper = [
        track((4.0, 10.0), (10.0, 10.0), "SP", "load"),
        track((10.0, 10.0), (12.0, 14.0), "SP", "sense-p"),
        track((14.0, 10.0), (14.0, 14.0), "SN", "sense-n"),
    ]
    .concat();
    let v = run(&fx(
        &sense_parts(),
        Some(sense_board((12.0, 14.0), &copper)),
        None,
    ));
    assert!(
        findings(&v, "sense_not_kelvin").is_empty(),
        "{}",
        v["findings"]
    );
    assert_eq!(check(&v, "sense_not_kelvin")["passed"], 2);
}

#[test]
fn a_sense_trace_tapped_off_the_power_trace_is_not_kelvin() {
    let copper = [
        track((4.0, 10.0), (10.0, 10.0), "SP", "load"),
        track((7.0, 10.0), (12.0, 14.0), "SP", "sense-p"),
        track((14.0, 10.0), (14.0, 14.0), "SN", "sense-n"),
    ]
    .concat();
    let v = run(&fx(
        &sense_parts(),
        Some(sense_board((12.0, 14.0), &copper)),
        None,
    ));
    let f = only(&v, "sense_not_kelvin");
    assert_eq!(
        f["checklist"],
        "8 Power stage: Kelvin sense traces from the shunt"
    );
    assert_eq!(f["evidence"]["side"], "positive");
    assert_eq!(check(&v, "sense_not_kelvin")["passed"], 1);
}

#[test]
fn an_unrouted_sense_connection_is_unevaluated_not_kelvin() {
    let v = run(&fx(
        &sense_parts(),
        Some(sense_board((12.0, 14.0), "")),
        None,
    ));
    let c = check(&v, "sense_not_kelvin");
    assert_eq!(c["unevaluated"], 2, "{c}");
    assert_eq!(c["passed"], 0);
    assert!(c["unevaluated_reasons"][0]
        .as_str()
        .unwrap()
        .contains("no copper path"));
}

#[test]
fn an_amplifier_with_no_shunt_between_its_inputs_is_skipped() {
    let mut parts = sense_parts();
    parts[1] = res("R9", "10k", "SP", "SN");
    let v = run(&fx(&parts, Some(sense_board((12.0, 14.0), "")), None));
    assert_eq!(check(&v, "sense_amp_far_from_shunt")["skipped"], 1);
    assert!(findings(&v, "sense_amp_far_from_shunt").is_empty());
}

// ─── Crystal and ADC reference ───────────────────────────────────────────────

fn xtal_parts(with_c2: bool) -> Vec<P> {
    let mut parts = vec![
        p(
            "Y1",
            "Device:Crystal",
            "8MHz",
            vec![("1", "1", "passive", "XI"), ("2", "2", "passive", "XO")],
        ),
        p(
            "U1",
            "Fixture:XMCU",
            "MCU",
            vec![
                ("1", "OSC_IN", "input", "XI"),
                ("2", "OSC_OUT", "output", "XO"),
                ("3", "VDD", "power_in", "+3V3"),
                ("4", "GND", "power_in", "GND"),
            ],
        ),
        cap("C1", "18p", "XI", "GND"),
    ];
    if with_c2 {
        parts.push(cap("C2", "18p", "XO", "GND"));
    }
    parts
}

fn xtal_board(y1_x: f64, c1_x: f64) -> String {
    board(
        &[
            fp(
                "U1",
                10.0,
                10.0,
                &[
                    ("1", 0.0, 0.0, "XI"),
                    ("2", 2.0, 0.0, "XO"),
                    ("3", 4.0, 0.0, "+3V3"),
                    ("4", 6.0, 0.0, "GND"),
                ],
            ),
            fp(
                "Y1",
                y1_x,
                14.0,
                &[("1", 0.0, 0.0, "XI"), ("2", 2.0, 0.0, "XO")],
            ),
            fp(
                "C1",
                c1_x,
                16.0,
                &[("1", 0.0, 0.0, "XI"), ("2", 1.0, 0.0, "GND")],
            ),
            fp(
                "C2",
                y1_x,
                16.0,
                &[("1", 0.0, 0.0, "XO"), ("2", 1.0, 0.0, "GND")],
            ),
        ],
        "",
    )
}

#[test]
fn a_crystal_pin_with_no_load_capacitor_fires_and_the_computed_load_is_reported() {
    let v = run(&fx(&xtal_parts(false), None, None));
    let f = only(&v, "crystal_load_cap_missing");
    assert_eq!(
        f["checklist"],
        "2 Placement: crystal / ADC reference capacitor"
    );
    assert_eq!(f["evidence"]["pin"], "2");

    let ok = run(&fx(&xtal_parts(true), None, None));
    assert!(findings(&ok, "crystal_load_cap_missing").is_empty());
    assert_eq!(check(&ok, "crystal_load_cap_missing")["passed"], 2);
    let x = &ok["crystals"][0];
    assert_eq!(x["computed_load_capacitance_pf"], 12.0);
    assert_eq!(x["stray_assumed_pf"], 3.0);
    assert!(x["target_load_capacitance_pf"]
        .as_str()
        .unwrap()
        .contains("needs_human"));
}

#[test]
fn a_load_cap_or_crystal_far_from_its_pins_fires() {
    let far_cap = run(&fx(&xtal_parts(true), Some(xtal_board(12.0, 40.0)), None));
    let f = only(&far_cap, "crystal_cap_far");
    assert_eq!(f["evidence"]["capacitor"], "C1");
    assert!(findings(&far_cap, "crystal_far_from_ic").is_empty());

    let far_xtal = run(&fx(&xtal_parts(true), Some(xtal_board(60.0, 61.0)), None));
    assert!(!findings(&far_xtal, "crystal_far_from_ic").is_empty());
    assert_eq!(
        findings(&far_xtal, "crystal_far_from_ic")[0]["evidence"]["limit_mm"],
        10.0
    );

    let near = run(&fx(&xtal_parts(true), Some(xtal_board(12.0, 12.0)), None));
    assert!(
        findings(&near, "crystal_cap_far").is_empty(),
        "{}",
        near["findings"]
    );
    assert!(findings(&near, "crystal_far_from_ic").is_empty());
    assert_eq!(check(&near, "crystal_cap_far")["passed"], 2);
}

#[test]
fn an_adc_reference_cap_far_from_its_pin_fires() {
    let parts = vec![
        p(
            "U1",
            "Fixture:ADC",
            "ADC",
            vec![
                ("1", "VREF+", "input", "VREF"),
                ("2", "GND", "power_in", "GND"),
                ("3", "VDD", "power_in", "+3V3"),
            ],
        ),
        cap("C1", "1u", "VREF", "GND"),
    ];
    let b = |x: f64| {
        board(
            &[
                fp(
                    "U1",
                    10.0,
                    10.0,
                    &[
                        ("1", 0.0, 0.0, "VREF"),
                        ("2", 2.0, 0.0, "GND"),
                        ("3", 4.0, 0.0, "+3V3"),
                    ],
                ),
                fp(
                    "C1",
                    x,
                    10.0,
                    &[("1", 0.0, 0.0, "VREF"), ("2", 1.0, 0.0, "GND")],
                ),
            ],
            "",
        )
    };
    let far = run(&fx(&parts, Some(b(40.0)), None));
    let f = only(&far, "adc_ref_cap_far");
    assert_eq!(
        f["checklist"],
        "2 Placement: crystal / ADC reference capacitor"
    );
    let near = run(&fx(&parts, Some(b(12.0)), None));
    assert!(findings(&near, "adc_ref_cap_far").is_empty());
    assert_eq!(check(&near, "adc_ref_cap_far")["passed"], 1);
}

// ─── Interfaces: I2C, UART, USB ──────────────────────────────────────────────

fn i2c_dev() -> P {
    p(
        "U1",
        "Fixture:SENSOR",
        "SENSOR",
        vec![
            ("1", "SDA", "bidirectional", "SDA"),
            ("2", "SCL", "input", "SCL"),
            ("3", "VDD", "power_in", "+3V3"),
            ("4", "GND", "power_in", "GND"),
        ],
    )
}

#[test]
fn i2c_pull_ups_to_a_higher_rail_than_the_device_fire_as_an_error() {
    let parts = vec![
        i2c_dev(),
        res("R1", "4k7", "SDA", "+5V"),
        res("R2", "4k7", "SCL", "+5V"),
    ];
    let v = run(&fx(&parts, None, None));
    let all = findings(&v, "i2c_pullup_rail");
    assert_eq!(all.len(), 2, "{}", v["findings"]);
    assert_eq!(all[0]["severity"], "error");
    assert_eq!(all[0]["checklist"], "6 Interfaces: I2C");
    assert_eq!(all[0]["evidence"]["pull_up_rail_v"], 5.0);
    assert_eq!(all[0]["evidence"]["device_supply_v"], 3.3);
}

#[test]
fn i2c_pull_ups_on_the_device_rail_pass_and_the_interface_block_counts_them() {
    let parts = vec![
        i2c_dev(),
        res("R1", "4k7", "SDA", "+3V3"),
        res("R2", "4k7", "SCL", "+3V3"),
        res("R3", "10k", "SDA", "+3V3"),
    ];
    let v = run(&fx(&parts, None, None));
    assert!(findings(&v, "i2c_pullup_rail").is_empty());
    assert!(findings(&v, "i2c_pullup_value").is_empty());
    let sda = v["interfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["line"] == "SDA")
        .unwrap();
    assert_eq!(sda["pull_ups"].as_array().unwrap().len(), 2);
    // 4.7k || 10k
    let eff = sda["effective_pull_up_ohms"].as_f64().unwrap();
    assert!((eff - 3197.279).abs() < 0.01, "{eff}");
    assert_eq!(
        sda["rise_time"].as_str().unwrap().split(':').next(),
        Some("needs_human")
    );
    assert_eq!(check(&v, "i2c_pullup_rail")["passed"], 3);
}

#[test]
fn an_i2c_pull_up_that_asks_for_too_much_sink_current_or_is_too_weak_fires() {
    let strong = run(&fx(
        &[i2c_dev(), res("R1", "330", "SDA", "+3V3")],
        None,
        None,
    ));
    let f = only(&strong, "i2c_pullup_value");
    assert!(f["evidence"]["sink_ma"].as_f64().unwrap() > 8.0);
    let weak = run(&fx(
        &[i2c_dev(), res("R1", "22k", "SDA", "+3V3")],
        None,
        None,
    ));
    assert_eq!(
        only(&weak, "i2c_pullup_value")["evidence"]["effective_ohms"],
        22000.0
    );
}

#[test]
fn a_bus_with_no_pull_up_is_left_to_audit_connections_and_a_module_connector_is_not_counted() {
    let conn = p(
        "J1",
        "Connector_Generic:Conn_01x04",
        "I2C",
        vec![
            ("1", "SDA", "passive", "SDA"),
            ("2", "SCL", "passive", "SCL"),
            ("3", "V", "passive", "+3V3"),
            ("4", "G", "passive", "GND"),
        ],
    );
    let v = run(&fx(&[i2c_dev(), conn], None, None));
    assert!(
        findings(&v, "i2c_pullup_rail").is_empty() && findings(&v, "i2c_pullup_value").is_empty()
    );
    let sda = v["interfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["line"] == "SDA")
        .unwrap();
    assert_eq!(sda["pull_ups"], json!([]));
    assert!(sda["missing_pull_up"]
        .as_str()
        .unwrap()
        .contains("audit_connections"));
    assert!(sda["module_pull_ups"]
        .as_str()
        .unwrap()
        .contains("not counted"));
    assert_eq!(check(&v, "i2c_pullup_rail")["status"], "no_subjects");
}

#[test]
fn an_i2c_rail_whose_voltage_is_unknown_is_skipped_for_a_human_not_failed() {
    let parts = vec![
        p(
            "U1",
            "Fixture:SENSOR2",
            "SENSOR",
            vec![
                ("1", "SDA", "bidirectional", "SDA"),
                ("2", "SCL", "input", "SCL"),
                ("3", "VDD", "power_in", "VDDX"),
                ("4", "GND", "power_in", "GND"),
            ],
        ),
        res("R1", "4k7", "SDA", "VCC_IO"),
    ];
    let v = run(&fx(&parts, None, None));
    assert!(findings(&v, "i2c_pullup_rail").is_empty());
    let c = check(&v, "i2c_pullup_rail");
    assert_eq!(c["skipped"], 1, "{c}");
    assert!(c["skipped_reasons"][0]
        .as_str()
        .unwrap()
        .contains("needs_human"));
    assert_eq!(v["coverage"]["unevaluated_subjects"], 0);
}

pub(crate) fn uart_dev(r: &'static str, tx: &'static str, rx: &'static str) -> P {
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

#[test]
fn a_crossed_uart_passes_and_tx_to_tx_or_rx_to_rx_fire() {
    let ok = run(&fx(
        &[uart_dev("U1", "A", "B"), uart_dev("U2", "B", "A")],
        None,
        None,
    ));
    assert!(findings(&ok, "uart_tx_rx_crossed").is_empty());
    assert_eq!(check(&ok, "uart_tx_rx_crossed")["passed"], 2);

    let bad = run(&fx(
        &[uart_dev("U1", "A", "B"), uart_dev("U2", "A", "B")],
        None,
        None,
    ));
    let all = findings(&bad, "uart_tx_rx_crossed");
    assert_eq!(all.len(), 2, "{}", bad["findings"]);
    assert!(all
        .iter()
        .all(|f| f["severity"] == "error" && f["checklist"] == "6 Interfaces: UART"));
    assert!(all
        .iter()
        .any(|f| f["issue"].as_str().unwrap().contains("Two transmit pins")));
    assert!(all
        .iter()
        .any(|f| f["issue"].as_str().unwrap().contains("no transmitter")));
}

pub(crate) fn usb_conn() -> P {
    p(
        "J1",
        "Connector:USB_B_Micro",
        "USB",
        vec![
            ("1", "VBUS", "passive", "VBUS"),
            ("2", "D-", "passive", "USB_DM"),
            ("3", "D+", "passive", "USB_DP"),
            ("4", "GND", "passive", "GND"),
        ],
    )
}

fn usb_esd(value: &'static str, lib: &'static str) -> P {
    p(
        "D1",
        lib,
        value,
        vec![
            ("1", "IO1", "passive", "USB_DP"),
            ("2", "GND", "passive", "GND"),
            ("3", "IO2", "passive", "USB_DM"),
            ("4", "IO2b", "passive", "USB_DM"),
            ("5", "VBUS", "passive", "VBUS"),
            ("6", "IO1b", "passive", "USB_DP"),
        ],
    )
}

#[test]
fn usb_data_lines_without_esd_fire_and_with_a_low_capacitance_array_pass() {
    let bare = run(&fx(&[usb_conn()], None, None));
    let all = findings(&bare, "usb_esd_missing");
    assert_eq!(all.len(), 2, "{}", bare["findings"]);
    assert_eq!(all[0]["checklist"], "6 Interfaces: USB 2.0");

    let ok = run(&fx(
        &[
            usb_conn(),
            usb_esd("USBLC6-2SC6", "Power_Protection:USBLC6-2SC6"),
        ],
        None,
        None,
    ));
    assert!(findings(&ok, "usb_esd_missing").is_empty());
    assert!(findings(&ok, "usb_esd_capacitance").is_empty());
    assert_eq!(check(&ok, "usb_esd_capacitance")["passed"], 2);
}

#[test]
fn a_power_line_tvs_on_usb_data_fires_and_an_unknown_part_is_left_to_a_human() {
    let high = run(&fx(
        &[usb_conn(), usb_esd("SMAJ5.0A", "Device:D_TVS_Dual")],
        None,
        None,
    ));
    let all = findings(&high, "usb_esd_capacitance");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0]["evidence"]["family"], "SMAJ");
    let unknown = run(&fx(
        &[usb_conn(), usb_esd("XYZ123", "Device:D_TVS_Dual")],
        None,
        None,
    ));
    assert!(findings(&unknown, "usb_esd_capacitance").is_empty());
    let c = check(&unknown, "usb_esd_capacitance");
    assert!(c["skipped"].as_u64().unwrap() >= 1);
    assert!(c["skipped_reasons"][0]
        .as_str()
        .unwrap()
        .contains("needs_human"));
}

fn usb_intent(z: Option<f64>, pairs: bool) -> Value {
    let mut iface = json!({"name": "usb", "kind": "usb2", "nets": ["USB_DP", "USB_DM"]});
    if let Some(z) = z {
        iface["differential_impedance_ohm"] = json!(z);
    }
    if pairs {
        iface["differential_pairs"] = json!([{"positive": "USB_DP", "negative": "USB_DM"}]);
    }
    json!({"schema_version": 1, "interfaces": [iface]})
}

#[test]
fn the_recorded_usb_impedance_must_be_90_ohm() {
    let bad = run(&fx(
        &[usb_conn()],
        None,
        Some(usb_intent(Some(100.0), true)),
    ));
    let f = only(&bad, "usb_pair_impedance");
    assert_eq!(f["severity"], "error");
    assert_eq!(f["evidence"]["recorded_ohms"], 100.0);
    let none = run(&fx(&[usb_conn()], None, Some(usb_intent(None, false))));
    assert_eq!(only(&none, "usb_pair_impedance")["severity"], "warning");
    let ok = run(&fx(&[usb_conn()], None, Some(usb_intent(Some(90.0), true))));
    assert!(findings(&ok, "usb_pair_impedance").is_empty());
    assert_eq!(check(&ok, "usb_pair_impedance")["passed"], 1);
    let no_record = run(&fx(&[usb_conn()], None, None));
    assert_eq!(check(&no_record, "usb_pair_impedance")["status"], "not_run");
}

// ─── Power tree ──────────────────────────────────────────────────────────────

fn buck(top: &'static str, bottom: &'static str) -> Vec<P> {
    vec![
        p(
            "U2",
            "Regulator_Switching:TPS54331",
            "TPS54331",
            vec![
                ("1", "VIN", "power_in", "+12V"),
                ("2", "GND", "power_in", "GND"),
                ("3", "SW", "output", "SWN"),
                ("4", "FB", "input", "FB"),
                ("5", "EN", "input", "+12V"),
            ],
        ),
        p(
            "L1",
            "Device:L",
            "10u",
            vec![("1", "~", "passive", "SWN"), ("2", "~", "passive", "+3V3")],
        ),
        res("R1", top, "+3V3", "FB"),
        res("R2", bottom, "FB", "GND"),
    ]
}

fn rails(source: &str, voltage: f64, nets: &[&str]) -> Value {
    json!({"schema_version": 1, "rails": [
        {"name": "3V3", "nets": nets, "voltage_v": voltage, "source_ref": source},
        {"name": "12V", "nets": ["+12V"], "voltage_v": 12.0}
    ]})
}

#[test]
fn a_feedback_divider_that_gives_the_recorded_voltage_passes_and_one_that_does_not_fires() {
    let ok = run(&fx(
        &buck("31.6k", "10k"),
        None,
        Some(rails("U2", 3.3, &["+3V3"])),
    ));
    assert!(
        findings(&ok, "feedback_vout_mismatch").is_empty(),
        "{}",
        ok["findings"]
    );
    assert_eq!(check(&ok, "feedback_vout_mismatch")["passed"], 1);
    let fb = &ok["power_tree"]["rails"][0]["feedback"];
    assert_eq!(fb["status"], "matches_record");
    assert_eq!(fb["reference_v"], 0.8);
    assert!(fb["reference_source"]
        .as_str()
        .unwrap()
        .contains("TPS54331"));
    assert!((fb["computed_voltage_v"].as_f64().unwrap() - 3.328).abs() < 0.001);

    let bad = run(&fx(
        &buck("10k", "10k"),
        None,
        Some(rails("U2", 3.3, &["+3V3"])),
    ));
    let f = only(&bad, "feedback_vout_mismatch");
    assert_eq!(f["severity"], "error");
    assert_eq!(f["checklist"], "Power tree (design-intent rails)");
    assert_eq!(f["evidence"]["computed_voltage_v"], 1.6);
}

#[test]
fn a_regulator_whose_reference_is_not_in_the_table_reports_the_implied_reference_and_is_not_judged()
{
    let mut parts = buck("31.6k", "10k");
    parts[0].val = "XR9999";
    parts[0].lib = "Regulator_Switching:XR9999";
    let v = run(&fx(&parts, None, Some(rails("U2", 3.3, &["+3V3"]))));
    assert!(findings(&v, "feedback_vout_mismatch").is_empty());
    let fb = &v["power_tree"]["rails"][0]["feedback"];
    assert_eq!(fb["status"], "reference_unknown");
    assert!((fb["implied_reference_v"].as_f64().unwrap() - 0.793).abs() < 0.001);
    assert_eq!(check(&v, "feedback_vout_mismatch")["skipped"], 1);
    assert_eq!(check(&v, "feedback_vout_mismatch")["passed"], 0);
}

#[test]
fn the_power_tree_follows_source_ref_to_the_input_rail() {
    let v = run(&fx(
        &buck("31.6k", "10k"),
        None,
        Some(rails("U2", 3.3, &["+3V3"])),
    ));
    let r = &v["power_tree"]["rails"][0];
    assert_eq!(r["source_kind"], "regulator");
    assert_eq!(r["input"]["rail"], "12V");
    assert_eq!(check(&v, "rail_source_not_on_rail")["passed"], 1);
    assert_eq!(check(&v, "rail_source_missing")["passed"], 1);
    assert!(v["power_tree"]["regulators_not_recorded_as_a_source"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn rail_records_that_disagree_with_the_schematic_fire() {
    // The rail's net does not exist.
    let v = run(&fx(
        &buck("31.6k", "10k"),
        None,
        Some(rails("U2", 3.3, &["+1V8"])),
    ));
    assert_eq!(only(&v, "rail_not_in_schematic")["evidence"]["rail"], "3V3");

    // The net is named for another voltage.
    let v = run(&fx(
        &buck("31.6k", "10k"),
        None,
        Some(rails("U2", 5.0, &["+3V3"])),
    ));
    assert_eq!(
        only(&v, "rail_name_voltage")["evidence"]["name_voltage_v"],
        3.3
    );

    // The source is not in the design, or not on the rail.
    let v = run(&fx(
        &buck("31.6k", "10k"),
        None,
        Some(rails("U9", 3.3, &["+3V3"])),
    ));
    assert_eq!(
        only(&v, "rail_source_missing")["evidence"]["source_ref"],
        "U9"
    );
    let parts = vec![regulator("U5", "+1V2"), mcu("U1", "SIG0")];
    let v = run(&fx(&parts, None, Some(rails("U5", 3.3, &["+3V3"]))));
    assert_eq!(only(&v, "rail_source_not_on_rail")["severity"], "error");
}

#[test]
fn a_regulator_nobody_recorded_is_named() {
    let v = run(&fx(
        &buck("31.6k", "10k"),
        None,
        Some(rails("U7", 3.3, &["+3V3"])),
    ));
    assert_eq!(
        v["power_tree"]["regulators_not_recorded_as_a_source"],
        json!(["U2"])
    );
}

#[test]
fn without_a_record_the_power_tree_is_not_run() {
    let v = run(&fx(&buck("31.6k", "10k"), None, None));
    assert_eq!(v["power_tree"]["status"], "not_run");
    assert_eq!(v["design_intent"]["status"], "absent");
}

#[test]
fn an_unusable_record_is_refused_not_ignored() {
    let f = fx(&[res("R1", "1k", "A", "B")], None, None);
    let path = design_intent_path(f.sch.parent().unwrap());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{not json").unwrap();
    let r = audit_files(&f.sch, std::slice::from_ref(&f.sch), None, vec![]).unwrap();
    assert!(r.is_error);
}

// ─── Classifiers ─────────────────────────────────────────────────────────────

#[test]
fn part_roles_come_from_symbols_and_pin_names() {
    use component_class as cc;
    assert_eq!(cc::norm_pin("~{RE}"), "RE");
    assert_eq!(cc::norm_pin("CAN_H"), "CANH");
    assert!(cc::is_connector("Connector_Generic:Conn_01x02", "J1"));
    assert!(!cc::is_connector("Device:R", "J1"));
    assert!(cc::is_resistor("Device:R_Small", "R3") && !cc::is_resistor("Device:C", "C1"));
    assert!(cc::is_crystal("Device:Crystal_GND24", "Y1"));
    assert!(!cc::is_crystal("Oscillator:ASE-xxxMHz", "X1"));
    assert_eq!(
        cc::protection_device("Power_Protection:USBLC6-2SC6", "USBLC6-2SC6")
            .unwrap()
            .low_capacitance,
        Some(true)
    );
    assert_eq!(
        cc::protection_device("Device:D", "SMBJ12A")
            .unwrap()
            .low_capacitance,
        Some(false)
    );
    assert_eq!(
        cc::protection_device("Device:D_TVS", "x")
            .unwrap()
            .low_capacitance,
        None
    );
    assert!(cc::protection_device("Device:D", "1N4148").is_none());
    assert_eq!(cc::uart_direction("USART1TX"), Some(0));
    assert_eq!(cc::uart_direction("UART2RXD"), Some(1));
    assert_eq!(cc::uart_direction("TXEN"), None);
    assert_eq!(cc::uart_direction("TXD"), Some(0));
}
