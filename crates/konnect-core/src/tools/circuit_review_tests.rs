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
    dnp: bool,
    on_board: bool,
    /// `(instance path, reference)` records, as a reused sheet's symbols carry.
    instances: Vec<(&'static str, &'static str)>,
}

pub(crate) fn p(r: &'static str, lib: &'static str, val: &'static str, pins: Vec<PinSpec>) -> P {
    P {
        r,
        lib,
        val,
        pins,
        dnp: false,
        on_board: true,
        instances: Vec::new(),
    }
}

impl P {
    /// Marked do-not-populate.
    pub(crate) fn dnp(mut self) -> P {
        self.dnp = true;
        self
    }

    /// Marked `on_board no`.
    pub(crate) fn off_board(mut self) -> P {
        self.on_board = false;
        self
    }

    /// An `(instances (project (path ...)))` record: this symbol is
    /// `reference` on the sheet at `path`.
    pub(crate) fn instance(mut self, path: &'static str, reference: &'static str) -> P {
        self.instances.push((path, reference));
        self
    }
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
        let instances = if part.instances.is_empty() {
            String::new()
        } else {
            let paths: String = part
                .instances
                .iter()
                .map(|(path, reference)| {
                    format!("(path \"{path}\" (reference \"{reference}\") (unit 1))")
                })
                .collect();
            format!("    (instances (project \"demo\" {paths}))\n")
        };
        body.push_str(&format!(
            "  (symbol (lib_id \"{lib}\") (at {x} {y} 0) (unit 1) (in_bom yes) (on_board {ob}) (dnp {dnp}) (uuid \"{u}\")\n    (property \"Reference\" \"{r}\" (at {x} {y} 0))\n    (property \"Value\" \"{v}\" (at {x} {y} 0))\n    (property \"Footprint\" \"\" (at {x} {y} 0))\n{instances}  )\n",
            lib = part.lib,
            u = next(),
            r = part.r,
            v = part.val,
            ob = if part.on_board { "yes" } else { "no" },
            dnp = if part.dnp { "yes" } else { "no" },
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
    json_of(
        audit_files(
            &f.sch,
            std::slice::from_ref(&f.sch),
            f.board.as_deref(),
            vec![],
        )
        .unwrap(),
    )
}

fn json_of(result: crate::mcp::protocol::CallToolResult) -> Value {
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

/// How many subjects of `rule` passed (0 when the rule examined nothing).
fn passed(v: &Value, rule: &str) -> u64 {
    check(v, rule)["passed"].as_u64().unwrap_or(0)
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
fn a_cap_value_outside_the_families_is_left_to_a_human_not_judged_and_not_called_empty() {
    for value in ["4n7", "220n", "4u7"] {
        let parts = vec![mcu("U1", "SIG0"), cap("C1", value, "+3V3", "GND")];
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
        assert!(findings(&v, "decoupling_supply_far").is_empty(), "{value}");
        let c = check(&v, "decoupling_supply_far");
        assert_eq!(c["needs_human"], 1, "{value}: {c}");
        assert_eq!(c["status"], "needs_human", "{value}: {c}");
        assert_eq!(c["passed"], 0);
        let reason = c["reason"].as_str().unwrap();
        assert!(
            !reason.contains("nothing in the design"),
            "a rule whose only subject was met is not empty: {reason}"
        );
        assert!(c["needs_human_reasons"][0]
            .as_str()
            .unwrap()
            .starts_with("U1.1:"));
        // Counted, listed with its subject, and not a gap in the review.
        assert_eq!(v["coverage"]["needs_human_subjects"], 1);
        assert_eq!(v["coverage"]["unevaluated_subjects"], 0);
        assert_eq!(v["status"], "checked");
        assert!(v["needs_human"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["rule"] == "decoupling_supply_far" && n["subject"] == "U1.1"));
    }
}

#[test]
fn a_rule_whose_only_subjects_were_skipped_says_so_instead_of_claiming_it_was_empty() {
    // The regulator has no bulk capacitor: skipped (audit_power_rails reports
    // that), and the rule had something in front of it.
    let parts = vec![regulator("U2", "+3V3")];
    let b = board(
        &[fp(
            "U2",
            20.0,
            10.0,
            &[
                ("1", 0.0, 0.0, "GND"),
                ("2", 2.0, 0.0, "+3V3"),
                ("3", 4.0, 0.0, "+5V"),
            ],
        )],
        "",
    );
    let v = run(&fx(&parts, Some(b), None));
    let c = check(&v, "bulk_cap_far");
    assert_eq!(c["status"], "no_subjects", "{c}");
    assert!(c["skipped"].as_u64().unwrap() >= 1);
    let reason = c["reason"].as_str().unwrap();
    assert!(reason.contains("outside this rule's remit"), "{reason}");
    assert!(!reason.contains("nothing in the design"), "{reason}");
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
    assert_eq!(v["interfaces"][0]["termination"]["kind"], "none_observed");
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
    assert_eq!(c["needs_human"], 1, "{c}");
    assert_eq!(c["status"], "needs_human", "{c}");
    assert!(c["needs_human_reasons"][0]
        .as_str()
        .unwrap()
        .starts_with("R1:"));
    assert_eq!(v["coverage"]["unevaluated_subjects"], 0);
    assert_eq!(v["coverage"]["needs_human_subjects"], 1);
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
    assert!(c["needs_human"].as_u64().unwrap() >= 1, "{c}");
    assert_eq!(c["status"], "needs_human");
    assert!(c["needs_human_reasons"][0]
        .as_str()
        .unwrap()
        .starts_with("D1:"));
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
    buck_on("+3V3", top, bottom)
}

/// The buck with its output on net `out`.
pub(crate) fn buck_on(out: &'static str, top: &'static str, bottom: &'static str) -> Vec<P> {
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
            vec![("1", "~", "passive", "SWN"), ("2", "~", "passive", out)],
        ),
        res("R1", top, out, "FB"),
        res("R2", bottom, "FB", "GND"),
    ]
}

pub(crate) fn rails(source: &str, voltage: f64, nets: &[&str]) -> Value {
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
    let c = check(&v, "feedback_vout_mismatch");
    assert_eq!(c["needs_human"], 1, "{c}");
    assert_eq!(c["status"], "needs_human");
    assert_eq!(c["passed"], 0);
    assert_eq!(v["coverage"]["needs_human_subjects"], 1);
    assert_eq!(v["status"], "checked");
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
    // A test point lives in the Connector library but is no connector.
    assert!(!cc::is_connector("Connector:TestPoint", "TP1"));
    assert!(!cc::is_connector("Connector:TestPoint_Small", "TP2"));
    assert!(!cc::is_connector("Connector:Conn_01x01", "TP3"));
    assert!(cc::is_connector("Connector:USB_C_Receptacle_USB2.0", "J1"));
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

// ─── Review follow-up (2026-11-09) ───────────────────────────────────────────

fn test_point(r: &'static str, net: &'static str) -> P {
    p(
        r,
        "Connector:TestPoint",
        "TP",
        vec![("1", "1", "passive", net)],
    )
}

#[test]
fn a_test_point_on_usb_data_is_not_a_connector_the_protection_is_measured_from() {
    let parts = vec![
        usb_conn(),
        usb_esd("USBLC6-2SC6", "Power_Protection:USBLC6-2SC6"),
        test_point("TP1", "USB_DP"),
    ];
    let b = board(
        &[
            fp(
                "J1",
                5.0,
                5.0,
                &[
                    ("1", 0.0, 0.0, "VBUS"),
                    ("2", 1.0, 0.0, "USB_DM"),
                    ("3", 2.0, 0.0, "USB_DP"),
                    ("4", 3.0, 0.0, "GND"),
                ],
            ),
            fp(
                "D1",
                8.0,
                5.0,
                &[
                    ("1", 0.0, 0.0, "USB_DP"),
                    ("2", 1.0, 0.0, "GND"),
                    ("3", 2.0, 0.0, "USB_DM"),
                    ("4", 3.0, 0.0, "USB_DM"),
                    ("5", 4.0, 0.0, "VBUS"),
                    ("6", 5.0, 0.0, "USB_DP"),
                ],
            ),
            fp("TP1", 40.0, 5.0, &[("1", 0.0, 0.0, "USB_DP")]),
        ],
        "",
    );
    let v = run(&fx(&parts, Some(b), None));
    for rule in [
        "esd_far_from_connector",
        "protection_beyond_circuit",
        "connector_signal_unprotected",
    ] {
        assert!(findings(&v, rule).is_empty(), "{rule}: {}", v["findings"]);
    }
}

#[test]
fn a_test_point_on_an_unprotected_signal_is_not_an_unprotected_connector_pin() {
    let v = run(&fx(
        &[receiver("SIG"), test_point("TP1", "SIG")],
        None,
        None,
    ));
    assert!(
        findings(&v, "connector_signal_unprotected").is_empty(),
        "{}",
        v["findings"]
    );
}

#[test]
fn a_test_point_on_a_can_line_does_not_make_the_bus_a_star() {
    let j1 = p(
        "J1",
        "Connector_Generic:Conn_01x03",
        "Conn",
        vec![
            ("1", "H", "passive", "CANH"),
            ("2", "L", "passive", "CANL"),
            ("3", "G", "passive", "GND"),
        ],
    );
    let v = run(&fx(
        &[can_xcvr(), can_conn(), j1, test_point("TP1", "CANH")],
        None,
        None,
    ));
    assert!(
        findings(&v, "can_bus_branches").is_empty(),
        "{}",
        v["findings"]
    );
    assert_eq!(check(&v, "can_bus_branches")["passed"], 1);
}

// Decoupling: the pin's highest-frequency capacitor is the one judged.

fn cap_fp(r: &str, x: f64) -> String {
    fp(
        r,
        x,
        10.0,
        &[("1", 0.0, 0.0, "+3V3"), ("2", 1.0, 0.0, "GND")],
    )
}

#[test]
fn a_near_ten_microfarad_cap_does_not_excuse_a_far_hundred_nanofarad_cap() {
    let parts = vec![
        mcu("U1", "SIG0"),
        cap("C1", "100n", "+3V3", "GND"),
        cap("C2", "10u", "+3V3", "GND"),
    ];
    // C1 is 15 mm from U1.1, C2 8 mm.
    let b = board(&[u1(), cap_fp("C1", 25.0), cap_fp("C2", 18.0)], "");
    let v = run(&fx(&parts, Some(b), None));
    let f = only(&v, "decoupling_supply_far");
    assert_eq!(f["evidence"]["capacitor"], "C1", "{f}");
    assert_eq!(f["evidence"]["limit_mm"], 2.5);
    assert_eq!(f["severity"], "warning");
    assert_eq!(check(&v, "decoupling_supply_far")["passed"], 0);
}

#[test]
fn a_near_hundred_nanofarad_cap_passes_whatever_the_ten_microfarad_cap_does() {
    let parts = vec![
        mcu("U1", "SIG0"),
        cap("C1", "100n", "+3V3", "GND"),
        cap("C2", "10u", "+3V3", "GND"),
    ];
    let b = board(&[u1(), cap_fp("C1", 11.5), cap_fp("C2", 60.0)], "");
    let v = run(&fx(&parts, Some(b), None));
    assert!(
        findings(&v, "decoupling_supply_far").is_empty(),
        "{}",
        v["findings"]
    );
    assert_eq!(check(&v, "decoupling_supply_far")["passed"], 1);
}

#[test]
fn a_pin_with_only_a_ten_microfarad_cap_is_not_a_silent_pass() {
    let parts = vec![mcu("U1", "SIG0"), cap("C2", "10u", "+3V3", "GND")];
    let near = run(&fx(
        &parts,
        Some(board(&[u1(), cap_fp("C2", 14.0)], "")),
        None,
    ));
    let f = only(&near, "decoupling_supply_far");
    assert_eq!(f["severity"], "info");
    assert_eq!(f["evidence"]["no_high_frequency_cap"], true);
    assert_eq!(f["evidence"]["family_judged"], "10u");
    assert!(f["issue"].as_str().unwrap().contains("no 100 nF or 1 uF"));
    assert_eq!(check(&near, "decoupling_supply_far")["passed"], 0);

    let far = run(&fx(
        &parts,
        Some(board(&[u1(), cap_fp("C2", 30.0)], "")),
        None,
    ));
    let f = only(&far, "decoupling_supply_far");
    assert_eq!(f["severity"], "warning");
    assert_eq!(f["evidence"]["limit_mm"], 10.0);
    assert!(f["issue"].as_str().unwrap().contains("no 100 nF or 1 uF"));
}

#[test]
fn a_hundred_nanofarad_cap_missing_from_the_board_is_not_replaced_by_a_nearer_ten_microfarad_one() {
    let parts = vec![
        mcu("U1", "SIG0"),
        cap("C1", "100n", "+3V3", "GND"),
        cap("C2", "10u", "+3V3", "GND"),
    ];
    // C1 is not on the board; C2 sits beside the pin.
    let b = board(&[u1(), cap_fp("C2", 11.0)], "");
    let v = run(&fx(&parts, Some(b), None));
    let c = check(&v, "decoupling_supply_far");
    assert_eq!(c["unevaluated"], 1, "{c}");
    assert_eq!(c["passed"], 0);
    assert_eq!(v["status"], "partial");
}

// Unfitted parts.

#[test]
fn a_dnp_usb_esd_array_does_not_satisfy_the_protection_rule() {
    let parts = vec![
        usb_conn(),
        usb_esd("USBLC6-2SC6", "Power_Protection:USBLC6-2SC6").dnp(),
    ];
    let v = run(&fx(&parts, None, None));
    assert_eq!(
        findings(&v, "usb_esd_missing").len(),
        2,
        "{}",
        v["findings"]
    );
    assert_eq!(check(&v, "usb_esd_missing")["passed"], 0);
    assert_eq!(v["coverage"]["unfitted_parts"], 1);
    assert_eq!(v["unfitted_parts"]["count"], 1);
    assert_eq!(v["unfitted_parts"]["parts"][0]["reference"], "D1");
    assert_eq!(v["unfitted_parts"]["parts"][0]["why"], "dnp");
}

#[test]
fn a_dnp_decoupling_cap_is_not_counted_even_when_it_sits_beside_the_pin() {
    let parts = vec![mcu("U1", "SIG0"), cap("C1", "100n", "+3V3", "GND").dnp()];
    let b = board(&[u1(), cap_fp("C1", 11.0)], "");
    let v = run(&fx(&parts, Some(b), None));
    let c = check(&v, "decoupling_supply_far");
    assert_eq!(c["passed"], 0, "{c}");
    assert_eq!(c["examined"].as_u64().unwrap_or(0), 0);
    assert_eq!(v["unfitted_parts"]["parts"][0]["reference"], "C1");
}

#[test]
fn an_off_board_part_is_not_missing_from_the_board() {
    let parts = vec![
        mcu("U1", "SIG0"),
        cap("C1", "100n", "+3V3", "GND").off_board(),
    ];
    let v = run(&fx(&parts, Some(board(&[u1()], "")), None));
    assert_eq!(check(&v, "decoupling_supply_far")["unevaluated"], 0);
    assert_eq!(v["coverage"]["unevaluated_subjects"], 0);
    assert_eq!(v["status"], "checked");
    assert_eq!(v["unfitted_parts"]["parts"][0]["why"], "not_on_board");
}

#[test]
fn an_unfitted_rail_source_is_reported_as_unfitted_not_as_unknown() {
    let parts = vec![regulator("U5", "+3V3").dnp(), mcu("U1", "SIG0")];
    let v = run(&fx(&parts, None, Some(rails("U5", 3.3, &["+3V3"]))));
    let f = only(&v, "rail_source_missing");
    assert!(f["issue"].as_str().unwrap().contains("not fitted"), "{f}");
}

// CAN: no pass without the evidence.

#[test]
fn a_can_transceiver_that_reaches_no_connector_is_not_a_pass() {
    let v = run(&fx(
        &[can_xcvr(), res("R1", "120", "CANH", "CANL")],
        None,
        None,
    ));
    let c = check(&v, "can_bus_branches");
    assert_eq!(c["passed"], 0, "{c}");
    assert_eq!(c["unevaluated"], 1);
    assert_eq!(v["status"], "partial");
}

fn global_can_xcvr() -> P {
    p(
        "U2",
        "Fixture:CANXCVR",
        "TCAN",
        vec![
            ("1", "CANH", "bidirectional", "g:CANH"),
            ("2", "CANL", "bidirectional", "g:CANL"),
            ("3", "VCC", "power_in", "+5V"),
            ("4", "GND", "power_in", "GND"),
        ],
    )
}

fn global_can_conn(r: &'static str) -> P {
    p(
        r,
        "Connector_Generic:Conn_01x03",
        "Conn",
        vec![
            ("1", "H", "passive", "g:CANH"),
            ("2", "L", "passive", "g:CANL"),
            ("3", "G", "passive", "GND"),
        ],
    )
}

#[test]
fn a_can_bus_that_leaves_the_sheet_is_unproven_not_a_pass() {
    for parts in [
        vec![global_can_xcvr()],
        vec![global_can_xcvr(), global_can_conn("J2")],
        vec![
            global_can_xcvr(),
            global_can_conn("J2"),
            global_can_conn("J3"),
        ],
    ] {
        let v = run(&fx(&parts, None, None));
        assert_eq!(passed(&v, "can_bus_branches"), 0);
        assert!(
            v["unproven"]
                .as_array()
                .unwrap()
                .iter()
                .any(|u| u["rule"] == "can_bus_branches"),
            "{}",
            v["unproven"]
        );
        assert_eq!(v["coverage"]["unevaluated_subjects"], 0);
    }
    // A lone transceiver whose bus leaves the sheet: the rule had a subject, so it
    // must not read as empty.
    let v = run(&fx(&[global_can_xcvr()], None, None));
    let reason = check(&v, "can_bus_branches")["reason"].as_str().unwrap();
    assert!(reason.contains("withheld as unproven"), "{reason}");
}

#[test]
fn a_can_bus_behind_a_common_mode_choke_is_unproven_and_names_the_choke() {
    let choke = p(
        "FL1",
        "Filter:Choke_CommonMode_FerriteCore_1234",
        "CMC",
        vec![
            ("1", "1", "passive", "CANH"),
            ("2", "2", "passive", "BUS_H"),
            ("3", "3", "passive", "CANL"),
            ("4", "4", "passive", "BUS_L"),
        ],
    );
    let on_bus = |r: &'static str| {
        p(
            r,
            "Connector_Generic:Conn_01x03",
            "Conn",
            vec![
                ("1", "H", "passive", "BUS_H"),
                ("2", "L", "passive", "BUS_L"),
                ("3", "G", "passive", "GND"),
            ],
        )
    };
    let v = run(&fx(
        &[can_xcvr(), choke, on_bus("J2"), on_bus("J3"), on_bus("J4")],
        None,
        None,
    ));
    assert_eq!(passed(&v, "can_bus_branches"), 0);
    let u = v["unproven"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["rule"] == "can_bus_branches")
        .expect("unproven");
    assert!(u["why"].as_str().unwrap().contains("FL1"), "{u}");
}

// Termination selected by a header or a JP jumper.

#[test]
fn a_two_pin_header_that_selects_the_termination_is_not_a_bus_branch() {
    let j1 = p(
        "J1",
        "Connector_Generic:Conn_01x03",
        "Conn",
        vec![
            ("1", "H", "passive", "CANH"),
            ("2", "L", "passive", "CANL"),
            ("3", "G", "passive", "GND"),
        ],
    );
    let j3 = p(
        "J3",
        "Connector_Generic:Conn_01x02",
        "Conn",
        vec![("1", "1", "passive", "X"), ("2", "2", "passive", "CANL")],
    );
    let v = run(&fx(
        &[
            can_xcvr(),
            can_conn(),
            j1,
            res("R1", "120", "CANH", "X"),
            j3,
        ],
        None,
        None,
    ));
    let term = &v["interfaces"][0]["termination"];
    assert_eq!(term["kind"], "selectable", "{term}");
    assert_eq!(term["selector"], "J3");
    assert_eq!(check(&v, "can_termination_value")["passed"], 1);
    assert!(
        findings(&v, "can_bus_branches").is_empty(),
        "{}",
        v["findings"]
    );
    assert_eq!(
        v["interfaces"][0]["connectors"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn a_jp_reference_jumper_selects_the_termination_whatever_its_symbol() {
    let jp = p(
        "JP3",
        "Fixture:SolderBridge",
        "SB",
        vec![("1", "1", "passive", "X"), ("2", "2", "passive", "CANL")],
    );
    let v = run(&fx(
        &[can_xcvr(), can_conn(), res("R1", "120", "CANH", "X"), jp],
        None,
        None,
    ));
    assert_eq!(v["interfaces"][0]["termination"]["kind"], "selectable");
    assert_eq!(v["interfaces"][0]["termination"]["selector"], "JP3");
}

// USB impedance: once per design.

#[test]
fn the_recorded_usb_impedance_is_judged_once_per_design_not_once_per_sheet() {
    let f = fx(&[usb_conn()], None, Some(usb_intent(Some(100.0), true)));
    let child = f.sch.with_file_name("child.kicad_sch");
    std::fs::write(&child, schematic(&[res("R9", "1k", "X", "Y")])).unwrap();
    let v = json_of(audit_files(&f.sch, &[f.sch.clone(), child], None, vec![]).unwrap());
    let all = findings(&v, "usb_pair_impedance");
    assert_eq!(all.len(), 1, "{}", v["findings"]);
    assert_eq!(check(&v, "usb_pair_impedance")["examined"], 1);
    assert!(
        all[0]["source"].as_str().unwrap().contains("design"),
        "the source is the design-intent record, not a sheet: {}",
        all[0]["source"]
    );
}

// Power tree: which pin may link a source to its rail.

fn ldo_with_power_good(r: &'static str) -> P {
    p(
        r,
        "Fixture:LDO",
        "AP2112K-5.0",
        vec![
            ("1", "VIN", "power_in", "+12V"),
            ("2", "GND", "power_in", "GND"),
            ("3", "VOUT", "power_out", "+5V"),
            ("4", "PG", "open_collector", "PG5"),
        ],
    )
}

#[test]
fn a_source_tied_to_the_rail_only_by_a_power_good_pull_up_is_not_on_the_rail() {
    let parts = vec![
        ldo_with_power_good("U5"),
        res("R9", "100k", "PG5", "+3V3"),
        mcu("U1", "SIG0"),
    ];
    let v = run(&fx(&parts, None, Some(rails("U5", 3.3, &["+3V3"]))));
    assert_eq!(only(&v, "rail_source_not_on_rail")["severity"], "error");
    assert_eq!(check(&v, "rail_source_not_on_rail")["passed"], 0);
}

#[test]
fn a_regulator_pin_that_is_not_a_switch_or_output_does_not_link_to_a_rail_through_a_ferrite() {
    let parts = vec![
        ldo_with_power_good("U5"),
        p(
            "FB2",
            "Device:FerriteBead",
            "BLM",
            vec![("1", "1", "passive", "PG5"), ("2", "2", "passive", "+3V3")],
        ),
        mcu("U1", "SIG0"),
    ];
    let v = run(&fx(&parts, None, Some(rails("U5", 3.3, &["+3V3"]))));
    assert_eq!(only(&v, "rail_source_not_on_rail")["severity"], "error");
}

#[test]
fn a_connector_reaches_its_rail_through_a_ferrite_but_not_through_a_resistor() {
    let feed = |link: P| {
        vec![
            p(
                "J7",
                "Connector_Generic:Conn_01x02",
                "Conn",
                vec![("1", "1", "passive", "RAW"), ("2", "2", "passive", "GND")],
            ),
            link,
            mcu("U1", "SIG0"),
        ]
    };
    let ferrite = p(
        "FB1",
        "Device:FerriteBead",
        "BLM",
        vec![("1", "1", "passive", "RAW"), ("2", "2", "passive", "+3V3")],
    );
    let ok = run(&fx(&feed(ferrite), None, Some(rails("J7", 3.3, &["+3V3"]))));
    assert_eq!(check(&ok, "rail_source_not_on_rail")["passed"], 1);
    let bad = run(&fx(
        &feed(res("R4", "100", "RAW", "+3V3")),
        None,
        Some(rails("J7", 3.3, &["+3V3"])),
    ));
    assert_eq!(only(&bad, "rail_source_not_on_rail")["severity"], "error");
}

// What counts as a regulator.

#[test]
fn a_low_side_gate_driver_is_not_a_regulator() {
    let part = |lib: &'static str, pins: Vec<PinSpec>| {
        let part = p("U3", lib, "DRV", pins);
        crate::tools::circuit_model::Part {
            reference: part.r.into(),
            lib_id: part.lib.into(),
            value: part.val.into(),
            pins: part
                .pins
                .iter()
                .map(|(num, name, ty, net)| crate::tools::circuit_model::Pin {
                    number: (*num).into(),
                    name: (*name).into(),
                    electrical_type: (*ty).into(),
                    net: (*net).into(),
                })
                .collect(),
        }
    };
    let tc4421 = part(
        "Driver_FET:TC4421",
        vec![
            ("1", "GND", "power_in", "GND"),
            ("2", "INPUT", "input", "PWM"),
            ("3", "NC", "no_connect", "N"),
            ("4", "VDD", "power_in", "+12V"),
            ("5", "~{OUTPUT}", "output", "GATE"),
        ],
    );
    let fan3122 = part(
        "Driver_FET:FAN3122xMX",
        vec![
            ("1", "EN", "input", "+12V"),
            ("2", "GND", "power_in", "GND"),
            ("3", "IN", "input", "PWM"),
            ("4", "OUT", "output", "GATE"),
            ("5", "V_{DD}", "power_in", "+12V"),
        ],
    );
    // Same pin names in a library of the owner's own: only the pin types
    // tell it from a regulator.
    let homemade = part(
        "Mine:DRIVER",
        vec![
            ("1", "VDD", "power_in", "+12V"),
            ("2", "IN", "input", "PWM"),
            ("3", "GND", "power_in", "GND"),
            ("4", "OUT", "output", "GATE"),
        ],
    );
    // Typed like a regulator, but in a Driver_* library.
    let by_library = part(
        "Driver_Motor:DRV8833",
        vec![
            ("1", "VIN", "power_in", "+12V"),
            ("2", "GND", "power_in", "GND"),
            ("3", "VOUT", "power_out", "MOTOR"),
        ],
    );
    // A switch pin and a supply, but high- and low-side gate outputs.
    let half_bridge = part(
        "Fixture:BUCKCONTROLLER",
        vec![
            ("1", "VIN", "power_in", "+12V"),
            ("2", "SW", "output", "SWN"),
            ("3", "HO", "output", "GH"),
            ("4", "LO", "output", "GL"),
            ("5", "GND", "power_in", "GND"),
        ],
    );
    for driver in [&tc4421, &fan3122, &homemade, &by_library, &half_bridge] {
        assert!(!is_regulator(driver), "{}", driver.lib_id);
    }
    let ldo = part(
        "Mine:LDO",
        vec![
            ("1", "VIN", "power_in", "+5V"),
            ("2", "GND", "power_in", "GND"),
            ("3", "VOUT", "power_out", "+3V3"),
        ],
    );
    assert!(is_regulator(&ldo));
}

// Sheet-path-aware rail names.

#[test]
fn a_sheet_prefixed_net_name_matches_only_on_its_own_sheet() {
    use crate::tools::circuit_model::Net;
    let net = |sheet: Option<&str>| Net {
        key: "VOUT".into(),
        names: ["VOUT".to_string()].into_iter().collect(),
        pins: Vec::new(),
        leaves_sheet: false,
        power_symbol: false,
        sheet_path: sheet.map(str::to_string),
        scoped_names: Default::default(),
        ambiguous_names: Default::default(),
        origins: Default::default(),
    };
    let power = net(Some("/Power/"));
    assert!(power.matches_name("VOUT") && power.matches_name("/VOUT"));
    assert!(power.matches_name("/Power/VOUT"));
    assert!(!power.matches_name("/Audio/VOUT"));
    assert!(!power.matches_name("/Power/Sub/VOUT"));
    assert!(!net(Some("/")).matches_name("/Power/VOUT"));
    assert!(!net(None).matches_name("/Power/VOUT"));
    assert!(net(Some("/Power/Sub/")).matches_name("/Power/Sub/VOUT"));
}

// The tables and the docs.

#[test]
fn the_rule_count_the_docs_state_is_the_table_s() {
    assert_eq!(RULES.len(), 36);
}

#[test]
fn a_feedback_pin_named_with_a_slash_is_found() {
    let pins = vec![("FB/VO".to_string(), "input".to_string())];
    assert_eq!(
        component_class::PinNames::new(&pins).find(FEEDBACK_PINS),
        Some(0)
    );
    let pins = vec![("FB/V_{O}".to_string(), "input".to_string())];
    assert_eq!(
        component_class::PinNames::new(&pins).find(FEEDBACK_PINS),
        Some(0)
    );
}

#[test]
fn a_supply_pin_with_no_capacitor_at_all_is_skipped_for_audit_decoupling_not_called_a_human_check()
{
    let v = run(&fx(&[mcu("U1", "SIG0")], Some(board(&[u1()], "")), None));
    let c = check(&v, "decoupling_supply_far");
    assert_eq!(c["skipped"], 1, "{c}");
    assert_eq!(c["needs_human"], 0);
    assert!(c["skipped_reasons"][0]
        .as_str()
        .unwrap()
        .contains("audit_decoupling"));
    assert_eq!(v["coverage"]["needs_human_subjects"], 0);
}

// Hierarchy model fixtures deliberately exercise pin partitions, not display
// names. Removing a join/scope/identity guard must change these assertions.
fn joined_model(
    sheets: &[(&str, &str, String)],
    complete: bool,
) -> crate::tools::circuit_model::Circuit {
    joined_model_with(sheets, complete, &[])
}

/// `joined_model`, with the instance paths the caller reports as unaudited.
fn joined_model_with(
    sheets: &[(&str, &str, String)],
    complete: bool,
    set_aside: &[&str],
) -> crate::tools::circuit_model::Circuit {
    use crate::tools::circuit_model::{Circuit, SheetPlace};
    let dir = tempfile::tempdir().unwrap();
    let mut circuits = Vec::new();
    for (file, instance, source) in sheets {
        let file = dir.path().join(file);
        std::fs::write(&file, source).unwrap();
        let place = SheetPlace {
            symbol_path: Some(format!(
                "/11111111-1111-4111-8111-111111111111{}",
                instance.trim_end_matches('/')
            )),
            instance_path: Some(instance.to_string()),
            name_path: Some(instance.to_string()),
        };
        circuits.push(Circuit::load_at(&file, &place).unwrap());
    }
    let set_aside = set_aside.iter().map(|path| path.to_string()).collect();
    Circuit::join(circuits, complete, &set_aside)
}

fn pin_net<'a>(c: &'a crate::tools::circuit_model::Circuit, reference: &str, pin: &str) -> &'a str {
    &c.part(reference)
        .unwrap()
        .pins
        .iter()
        .find(|p| p.number == pin)
        .unwrap()
        .net
}

fn add_root_nodes(mut source: String, nodes: &str) -> String {
    let end = source.rfind(')').unwrap();
    source.insert_str(end, nodes);
    source
}

fn hierarchy_sheet(file: &str, uuid: &str, ports: &[(&str, f64)]) -> String {
    let pins = ports
        .iter()
        .map(|(name, y)| {
            format!("(pin \"{name}\" input (at 60 {y} 0) (effects (font (size 1.27 1.27))))")
        })
        .collect::<String>();
    format!("(sheet (at 20 20) (size 40 30) (uuid \"{uuid}\") (property \"Sheetname\" \"{uuid}\" (at 20 19 0)) (property \"Sheetfile\" \"{file}\" (at 20 51 0)) {pins})")
}

#[test]
fn cross_sheet_label_scope_is_electrical_identity_not_display_text() {
    for (a, b, connected) in [
        ("g:BUS", "g:BUS", true),
        ("BUS", "BUS", false),
        ("g:BUS", "BUS", false),
    ] {
        let c = joined_model(
            &[
                (
                    "root.kicad_sch",
                    "/",
                    schematic(&[res("R1", "1k", a, "LOCAL")]),
                ),
                (
                    "child.kicad_sch",
                    "/a/",
                    schematic(&[res("R2", "1k", b, "LOCAL")]),
                ),
            ],
            true,
        );
        assert_eq!(pin_net(&c, "R1", "1") == pin_net(&c, "R2", "1"), connected);
        assert_ne!(pin_net(&c, "R1", "2"), pin_net(&c, "R2", "2"));
        assert!(c.inventory_complete, "{:?}", c.connectivity_gaps);
    }
}

fn power_fixture(reference: &'static str, local: bool) -> String {
    schematic(&[
        p(
            "#PWR01",
            "Fixture:Power",
            "+3V3",
            vec![("1", "VCC", "power_in", "+3V3")],
        ),
        res(reference, "1k", "+3V3", "LOCAL"),
    ])
    .replace(
        "(symbol \"Fixture:Power\"\n",
        &format!(
            "(symbol \"Fixture:Power\" (power {})\n",
            if local { "local" } else { "global" }
        ),
    )
}

#[test]
fn cross_sheet_global_power_and_labels_share_scope_but_local_power_does_not() {
    for local in [false, true] {
        let c = joined_model(
            &[
                (
                    "root.kicad_sch",
                    "/",
                    schematic(&[res("R1", "1k", "g:+3V3", "GND")]),
                ),
                ("child.kicad_sch", "/a/", power_fixture("R2", local)),
            ],
            true,
        );
        assert_eq!(pin_net(&c, "R1", "1") == pin_net(&c, "R2", "1"), !local);
        assert_eq!(
            c.parts.len(),
            2,
            "custom virtual power symbols are not fitted parts"
        );
    }
}

#[test]
fn cross_sheet_exact_ports_join_through_an_empty_middle_sheet() {
    let root = add_root_nodes(
        schematic(&[res("R1", "1k", "INPUT", "RET")]),
        &(hierarchy_sheet("middle.kicad_sch", "a", &[("LINK", 30.0)])
            + "(label \"INPUT\" (at 60 30 0))"),
    );
    let middle = add_root_nodes(
        schematic(&[]),
        &(hierarchy_sheet("leaf.kicad_sch", "b", &[("REMOTE", 30.0)])
            + "(hierarchical_label \"LINK\" (at 60 30 0) (shape input))"),
    );
    let leaf = schematic(&[res("R2", "1k", "REMOTE", "RET")]).replace(
        "(label \"REMOTE\"",
        "(hierarchical_label \"REMOTE\" (shape input)",
    );
    let c = joined_model(
        &[
            ("root.kicad_sch", "/", root),
            ("middle.kicad_sch", "/a/", middle),
            ("leaf.kicad_sch", "/a/b/", leaf),
        ],
        true,
    );
    assert_eq!(pin_net(&c, "R1", "1"), pin_net(&c, "R2", "1"));
    assert_ne!(pin_net(&c, "R1", "2"), pin_net(&c, "R2", "2"));
    assert!(c.inventory_complete, "{:?}", c.connectivity_gaps);
}

#[test]
fn cross_sheet_unmatched_port_and_incomplete_global_never_become_complete() {
    let root = add_root_nodes(
        schematic(&[res("R1", "1k", "g:BUS", "RET")]),
        &(hierarchy_sheet("child.kicad_sch", "a", &[("EXPECTED", 30.0)])
            + "(label \"BUS\" (at 60 30 0))"),
    );
    let child = schematic(&[res("R2", "1k", "DIFFERENT", "RET")]).replace(
        "(label \"DIFFERENT\"",
        "(hierarchical_label \"DIFFERENT\" (shape input)",
    );
    let c = joined_model(
        &[
            ("root.kicad_sch", "/", root),
            ("child.kicad_sch", "/a/", child),
        ],
        true,
    );
    assert!(!c.inventory_complete);
    assert!(!c.connectivity_gaps.is_empty());
    assert_ne!(pin_net(&c, "R1", "1"), pin_net(&c, "R2", "1"));
    assert!(c.net(pin_net(&c, "R1", "1")).unwrap().leaves_sheet);
}

#[test]
fn cross_sheet_net_tie_and_series_components_do_not_union_nets() {
    let c = joined_model(
        &[(
            "root.kicad_sch",
            "/",
            schematic(&[
                p(
                    "NT1",
                    "Device:NetTie_2",
                    "NetTie",
                    vec![("1", "1", "passive", "g:A"), ("2", "2", "passive", "g:B")],
                ),
                res("R1", "0", "g:B", "g:C"),
                p(
                    "FB1",
                    "Device:Ferrite_Bead",
                    "Ferrite",
                    vec![("1", "1", "passive", "g:C"), ("2", "2", "passive", "g:D")],
                ),
            ]),
        )],
        true,
    );
    for r in ["NT1", "R1", "FB1"] {
        assert_ne!(pin_net(&c, r, "1"), pin_net(&c, r, "2"));
    }
}

#[test]
fn cross_sheet_unsupported_jumper_and_bus_metadata_are_gaps() {
    for metadata in [
        "(duplicate_pin_numbers_are_jumpers yes)",
        "(jumper_pin_groups (1 2))",
    ] {
        let source = schematic(&[res("R1", "1k", "A", "B")]).replace(
            "(symbol \"Device:R\"\n",
            &format!("(symbol \"Device:R\" {metadata}\n"),
        );
        let c = joined_model(&[("root.kicad_sch", "/", source)], true);
        assert!(!c.inventory_complete);
        assert!(!c.connectivity_gaps.is_empty());
    }
    let c = joined_model(
        &[(
            "root.kicad_sch",
            "/",
            add_root_nodes(
                schematic(&[res("R1", "1k", "A", "B")]),
                "(bus (pts (xy 10 10) (xy 20 10)))",
            ),
        )],
        true,
    );
    assert!(!c.inventory_complete);
}

#[test]
fn cross_sheet_ambiguous_references_are_not_arbitrarily_selected() {
    let source = schematic(&[res("R1", "1k", "A", "B")
        .instance("/11111111-1111-4111-8111-111111111111", "R1")
        .instance("/11111111-1111-4111-8111-111111111111", "R9")]);
    let c = joined_model(&[("root.kicad_sch", "/", source)], true);
    assert!(c.parts.is_empty());
    assert!(!c.inventory_complete);
    let c = joined_model(
        &[
            (
                "root.kicad_sch",
                "/",
                schematic(&[res("R1", "1k", "A", "B")]),
            ),
            (
                "child.kicad_sch",
                "/a/",
                schematic(&[res("R1", "1k", "A", "B")]),
            ),
        ],
        true,
    );
    assert!(c.parts.is_empty());
    assert!(!c.inventory_complete);
}

#[test]
fn cross_sheet_scoped_aliases_survive_a_global_join() {
    let c = joined_model(
        &[
            (
                "root.kicad_sch",
                "/",
                schematic(&[res("R1", "1k", "g:VOUT", "RET")]),
            ),
            (
                "child.kicad_sch",
                "/Power/",
                schematic(&[res("R2", "1k", "g:VOUT", "RET")]),
            ),
            (
                "other.kicad_sch",
                "/Audio/",
                schematic(&[res("R3", "1k", "VOUT", "RET")]),
            ),
        ],
        true,
    );
    let power = c.net(pin_net(&c, "R1", "1")).unwrap();
    assert!(power.matches_name("/Power/VOUT"));
    assert!(!power.matches_name("/Audio/VOUT"));
    assert_ne!(pin_net(&c, "R1", "1"), pin_net(&c, "R3", "1"));
}

#[test]
fn cross_sheet_net_keys_are_stable_across_reloads_and_input_order() {
    let root = add_root_nodes(
        schematic(&[res("R1", "1k", "g:A", "B")]),
        "(label \"A\" (at 80 80 0))",
    );
    let child = schematic(&[res("R2", "1k", "g:A", "B")]);
    let a = joined_model(
        &[
            ("root.kicad_sch", "/", root.clone()),
            ("child.kicad_sch", "/a/", child.clone()),
        ],
        true,
    );
    for _ in 0..12 {
        let b = joined_model(
            &[
                ("child.kicad_sch", "/a/", child.clone()),
                ("root.kicad_sch", "/", root.clone()),
            ],
            true,
        );
        for r in ["R1", "R2"] {
            for pin in ["1", "2"] {
                assert_eq!(pin_net(&a, r, pin), pin_net(&b, r, pin));
            }
        }
    }
}

#[test]
fn cross_sheet_multi_unit_parts_preserve_both_origins_and_common_pins() {
    let a = schematic(&[p(
        "U1",
        "Fixture:Dual",
        "DUAL",
        vec![("1", "A", "input", "A"), ("8", "GND", "power_in", "g:GND")],
    )]);
    let b = schematic(&[p(
        "U1",
        "Fixture:Dual",
        "DUAL",
        vec![("2", "B", "input", "B"), ("8", "GND", "power_in", "g:GND")],
    )])
    .replace("Dual_1_1", "Dual_2_1")
    .replace("(unit 1)", "(unit 2)");
    let c = joined_model(
        &[("root.kicad_sch", "/", a), ("child.kicad_sch", "/a/", b)],
        true,
    );
    assert!(c.inventory_complete, "{:?}", c.connectivity_gaps);
    assert_eq!(c.parts.len(), 1);
    assert_eq!(c.part("U1").unwrap().pins.len(), 3);
    assert_eq!(c.origins["U1"].len(), 2);
}

#[test]
fn cross_sheet_reused_file_uses_its_own_instance_references_and_local_nets() {
    const A: &str = "/11111111-1111-4111-8111-111111111111/a";
    const B: &str = "/11111111-1111-4111-8111-111111111111/b";
    let child = schematic(&[res("R1", "1k", "LOCAL", "RET")
        .instance(A, "R1")
        .instance(B, "R101")]);
    let c = joined_model(
        &[
            ("child.kicad_sch", "/a/", child.clone()),
            ("child.kicad_sch", "/b/", child),
        ],
        true,
    );
    assert_eq!(c.parts.len(), 2);
    assert_ne!(pin_net(&c, "R1", "1"), pin_net(&c, "R101", "1"));
    assert_ne!(c.origins["R1"], c.origins["R101"]);
    assert!(c.inventory_complete);
}

#[test]
fn cross_sheet_unknown_global_participants_leave_the_known_partition_intact() {
    let c = joined_model(
        &[
            (
                "root.kicad_sch",
                "/",
                schematic(&[res("R1", "1k", "g:A", "LOCAL")]),
            ),
            (
                "child.kicad_sch",
                "/a/",
                schematic(&[res("R2", "1k", "g:A", "LOCAL")]),
            ),
        ],
        false,
    );
    assert_eq!(pin_net(&c, "R1", "1"), pin_net(&c, "R2", "1"));
    assert!(c.net(pin_net(&c, "R1", "1")).unwrap().leaves_sheet);
    assert_ne!(pin_net(&c, "R1", "2"), pin_net(&c, "R2", "2"));
    assert!(!c.inventory_complete);
}

#[test]
fn cross_sheet_duplicate_instance_paths_are_quarantined_before_union() {
    let c = joined_model(
        &[
            (
                "first.kicad_sch",
                "/a/",
                schematic(&[res("R1", "1k", "LOCAL_A", "RET")]),
            ),
            (
                "second.kicad_sch",
                "/a/",
                schematic(&[res("R2", "1k", "LOCAL_B", "RET")]),
            ),
        ],
        true,
    );
    assert!(!c.inventory_complete);
    assert!(
        c.parts.is_empty(),
        "ambiguous uses must not create positive connection evidence"
    );
}

#[test]
fn cross_sheet_formatted_scalar_names_join_without_becoming_buses() {
    for name in ["g:~{RESET}", "g:A_{B}", "g:X^{2}"] {
        let c = joined_model(
            &[
                (
                    "root.kicad_sch",
                    "/",
                    schematic(&[res("R1", "1k", name, "A")]),
                ),
                (
                    "child.kicad_sch",
                    "/a/",
                    schematic(&[res("R2", "1k", name, "B")]),
                ),
            ],
            true,
        );
        assert!(c.inventory_complete, "{:?}", c.connectivity_gaps);
        assert_eq!(pin_net(&c, "R1", "1"), pin_net(&c, "R2", "1"));
    }
}

#[test]
fn cross_sheet_instance_unit_controls_the_exact_pin_universe() {
    let source = schematic(&[
        p("U1", "Fixture:Dual", "Dual", vec![("1", "A", "input", "A")])
            .instance("/11111111-1111-4111-8111-111111111111/a", "U1")
            .instance("/11111111-1111-4111-8111-111111111111/b", "U2"),
    ]);
    let source = source.replace("(symbol \"Dual_1_1\"", "(symbol \"Dual_2_1\" (pin input line (at 0 0 0) (length 2.54) (name \"B\") (number \"2\"))) (symbol \"Dual_1_1\"")
        .replace("(reference \"U2\") (unit 1)", "(reference \"U2\") (unit 2)");
    let c = joined_model(
        &[
            ("child.kicad_sch", "/a/", source.clone()),
            ("child.kicad_sch", "/b/", source),
        ],
        true,
    );
    assert_eq!(
        c.part("U1")
            .unwrap()
            .pins
            .iter()
            .map(|p| p.number.as_str())
            .collect::<Vec<_>>(),
        ["1"]
    );
    assert_eq!(
        c.part("U2")
            .unwrap()
            .pins
            .iter()
            .map(|p| p.number.as_str())
            .collect::<Vec<_>>(),
        ["2"]
    );
    assert!(c.inventory_complete);
}

#[test]
fn cross_sheet_unresolved_symbols_taint_local_frontiers_and_virtual_power_is_not_ignored() {
    let physical = schematic(&[
        res("R1", "1k", "LOCAL", "RET"),
        res("R2", "1k", "LOCAL", "RET"),
    ])
    .replacen("(lib_id \"Device:R\")", "(lib_id \"Missing:Symbol\")", 1);
    let c = joined_model(&[("root.kicad_sch", "/", physical)], true);
    assert!(!c.inventory_complete);
    assert!(c.net(pin_net(&c, "R2", "1")).unwrap().leaves_sheet);
    let virtual_power = power_fixture("R2", false)
        .replace("(lib_id \"Fixture:Power\")", "(lib_id \"Missing:Power\")");
    let c = joined_model(&[("root.kicad_sch", "/", virtual_power)], true);
    assert!(
        !c.inventory_complete,
        "unreadable virtual power connectivity is also a gap"
    );
}

fn incomplete_labels(f: &Fx) -> Value {
    let source = std::fs::read_to_string(&f.sch)
        .unwrap()
        .replace("(label ", "(global_label ");
    std::fs::write(&f.sch, source).unwrap();
    run(f)
}

fn is_unproven(v: &Value, rule: &str) -> bool {
    v["unproven"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["rule"] == rule)
}

#[test]
fn cross_sheet_incomplete_decoupling_keeps_near_small_cap_witness_but_withholds_nearest_absence() {
    for (x, value, expected_pass) in [
        (11.0, "100n", true),
        (30.0, "100n", false),
        (11.0, "1u", false),
    ] {
        let b = board(
            &[
                u1(),
                fp(
                    "C1",
                    x,
                    10.0,
                    &[("1", 0.0, 0.0, "+3V3"), ("2", 1.0, 0.0, "GND")],
                ),
            ],
            "",
        );
        let f = fx(
            &[mcu("U1", "SIG0"), cap("C1", value, "+3V3", "GND")],
            Some(b),
            None,
        );
        let v = incomplete_labels(&f);
        assert!(findings(&v, "decoupling_supply_far").is_empty(), "{v}");
        assert_eq!(
            passed(&v, "decoupling_supply_far"),
            u64::from(expected_pass)
        );
        assert_eq!(is_unproven(&v, "decoupling_supply_far"), !expected_pass);
    }
}

#[test]
fn cross_sheet_incomplete_termination_reports_observed_resistance_and_keeps_proven_overload() {
    for (value, warns) in [("120", false), ("60", true)] {
        let f = fx(
            &[
                global_can_xcvr(),
                global_can_conn("J2"),
                res("R1", value, "g:CANH", "g:CANL"),
            ],
            None,
            None,
        );
        let v = run(&f);
        assert_eq!(
            findings(&v, "can_termination_value").len(),
            usize::from(warns),
            "{v}"
        );
        assert_eq!(passed(&v, "can_termination_value"), 0);
        let bus = v["interfaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["interface"] == "can")
            .unwrap();
        assert!(bus["termination"]["total_ohms"].is_null());
        assert!(bus["termination"]["observed_resistance_ohms"]
            .as_f64()
            .is_some());
        assert_eq!(bus["connectivity_complete"], false);
    }
}

#[test]
fn cross_sheet_incomplete_i2c_does_not_call_an_observed_total_complete() {
    for (value, warns) in [("47k", false), ("4k7", false), ("100", true)] {
        let f = fx(&[i2c_dev(), res("R1", value, "SDA", "+3V3")], None, None);
        let v = incomplete_labels(&f);
        assert_eq!(
            findings(&v, "i2c_pullup_value").len(),
            usize::from(warns),
            "{v}"
        );
        assert_eq!(passed(&v, "i2c_pullup_value"), 0);
        let line = v["interfaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["line"] == "SDA")
            .unwrap();
        assert!(line["effective_pull_up_ohms"].is_null());
        assert!(line["observed_effective_pull_up_ohms"].as_f64().is_some());
        assert_eq!(line["connectivity_complete"], false);
        assert!(line.get("net_key").is_none(), "internal key must not leak");
    }
}

#[test]
fn cross_sheet_incomplete_crystal_inventory_keeps_presence_but_not_a_complete_cl() {
    let f = fx(&xtal_parts(true), None, None);
    let v = incomplete_labels(&f);
    assert!(findings(&v, "crystal_load_cap_missing").is_empty());
    assert_eq!(passed(&v, "crystal_load_cap_missing"), 2);
    assert!(v["crystals"][0]["computed_load_capacitance_pf"].is_null());
    assert_eq!(v["crystals"][0]["connectivity_complete"], false);
}

#[test]
fn cross_sheet_incomplete_connector_does_not_claim_absence_even_with_protection_behind_series() {
    let f = fx(
        &[
            connector("J1", "SIG"),
            res("R1", "100", "SIG", "SIGP"),
            tvs("D1", "SIGP"),
            receiver("SIGP"),
        ],
        None,
        None,
    );
    let v = incomplete_labels(&f);
    for rule in [
        "protection_after_series_element",
        "connector_signal_unprotected",
    ] {
        assert!(findings(&v, rule).is_empty(), "{v}");
        assert!(is_unproven(&v, rule), "{v}");
        assert_eq!(passed(&v, rule), 0);
    }
}

#[test]
fn cross_sheet_scoped_aliases_escape_literal_slashes_like_board_nets() {
    let c = joined_model(
        &[(
            "root.kicad_sch",
            "/",
            schematic(&[res("R1", "1k", "IN/OUT", "RET")]),
        )],
        true,
    );
    let net = c.net(pin_net(&c, "R1", "1")).unwrap();
    assert!(net.matches_name("/IN{slash}OUT"));
}

#[test]
fn cross_sheet_multiple_direct_termination_resistors_need_bank_assignment_even_on_one_sheet() {
    for value in ["120", "240"] {
        let v = run(&fx(
            &[
                can_xcvr(),
                can_conn(),
                res("R1", value, "CANH", "CANL"),
                res("R2", value, "CANH", "CANL"),
            ],
            None,
            None,
        ));
        assert!(findings(&v, "can_termination_value").is_empty(), "{v}");
        assert_eq!(passed(&v, "can_termination_value"), 0);
        assert!(
            v["needs_human"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["rule"] == "can_termination_value"),
            "{v}"
        );
    }
}

#[test]
fn cross_sheet_reversed_selectable_termination_is_not_omitted_from_candidates() {
    let v = run(&fx(
        &[
            can_xcvr(),
            res("R1", "120", "CANH", "CANL"),
            res("R2", "120", "CANL", "MID"),
            p(
                "JP1",
                "Jumper:Jumper_2_Open",
                "Jumper",
                vec![("1", "A", "passive", "MID"), ("2", "B", "passive", "CANH")],
            ),
        ],
        None,
        None,
    ));
    let termination = &v["interfaces"][0]["termination"];
    assert!(
        termination["candidates"].as_array().unwrap().len() >= 2,
        "{v}"
    );
    assert!(termination["observed_resistors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r == "R2"));
    assert!(termination["total_ohms"].is_null());
    assert_eq!(passed(&v, "can_termination_value"), 0);
}

#[test]
fn cross_sheet_incomplete_bulk_and_adc_searches_keep_only_sufficient_near_witnesses() {
    for x in [None, Some(11.0), Some(60.0)] {
        let mut bulk = vec![regulator("U2", "+3V3")];
        let mut bulk_board = vec![fp(
            "U2",
            10.0,
            10.0,
            &[
                ("1", 0.0, 2.0, "GND"),
                ("2", 2.0, 0.0, "+3V3"),
                ("3", 0.0, 0.0, "+5V"),
            ],
        )];
        if let Some(x) = x {
            bulk.push(cap("C2", "22u", "+5V", "GND"));
            bulk_board.push(fp(
                "C2",
                x,
                10.0,
                &[("1", 0.0, 0.0, "+5V"), ("2", 0.0, 1.0, "GND")],
            ));
        }
        let v = incomplete_labels(&fx(&bulk, Some(board(&bulk_board, "")), None));
        assert!(findings(&v, "bulk_cap_far").is_empty(), "{v}");
        assert_eq!(passed(&v, "bulk_cap_far"), u64::from(x == Some(11.0)));
        assert_eq!(
            v["unproven"]
                .as_array()
                .unwrap()
                .iter()
                .any(|u| u["rule"] == "bulk_cap_far" && u["component"] == "U2" && u["pin"] == "3"),
            x != Some(11.0)
        );
        let mut adc = vec![p(
            "U1",
            "Fixture:ADC",
            "ADC",
            vec![
                ("1", "VREF+", "input", "VREF"),
                ("2", "GND", "power_in", "GND"),
                ("3", "VDD", "power_in", "+3V3"),
            ],
        )];
        let mut adc_board = vec![fp(
            "U1",
            10.0,
            10.0,
            &[
                ("1", 0.0, 0.0, "VREF"),
                ("2", 0.0, 2.0, "GND"),
                ("3", 2.0, 0.0, "+3V3"),
            ],
        )];
        if let Some(x) = x {
            adc.push(cap("C1", "1u", "VREF", "GND"));
            adc_board.push(fp(
                "C1",
                x,
                10.0,
                &[("1", 0.0, 0.0, "VREF"), ("2", 0.0, 1.0, "GND")],
            ));
        }
        let v = incomplete_labels(&fx(&adc, Some(board(&adc_board, "")), None));
        assert!(findings(&v, "adc_ref_cap_far").is_empty(), "{v}");
        assert_eq!(passed(&v, "adc_ref_cap_far"), u64::from(x == Some(11.0)));
        assert_eq!(is_unproven(&v, "adc_ref_cap_far"), x != Some(11.0));
    }
}

#[test]
fn cross_sheet_unresolved_far_end_cannot_establish_missing_capacitor_or_bias() {
    let b = board(
        &[
            u1(),
            fp(
                "C1",
                11.0,
                10.0,
                &[("1", 0.0, 0.0, "+3V3"), ("2", 0.0, 1.0, "RETURN")],
            ),
        ],
        "",
    );
    let v = run(&fx(
        &[mcu("U1", "SIG"), cap("C1", "100n", "+3V3", "g:RETURN")],
        Some(b),
        None,
    ));
    assert!(is_unproven(&v, "decoupling_supply_far"));
    let v = run(&fx(
        &[
            rs485_xcvr(),
            res("R1", "1k", "A", "g:UNKNOWN"),
            res("R2", "1k", "B", "GND"),
        ],
        None,
        None,
    ));
    assert!(findings(&v, "rs485_failsafe_bias").is_empty());
    assert!(is_unproven(&v, "rs485_failsafe_bias"));
}

#[test]
fn cross_sheet_incomplete_driver_and_sense_searches_do_not_claim_exhaustive_topology() {
    let v = incomplete_labels(&fx(&series_parts(), Some(series_board(60.0)), None));
    assert!(findings(&v, "series_termination_far_from_driver").is_empty());
    assert!(is_unproven(&v, "series_termination_far_from_driver"));
    let v = run(&fx(&half_bridge("g:SRC"), None, None));
    assert!(findings(&v, "gate_drive_return_net").is_empty());
    assert!(is_unproven(&v, "gate_drive_return_net"));
    let v = run(&fx(&half_bridge("g:SW"), None, None));
    assert_eq!(passed(&v, "gate_drive_return_net"), 1);
    let mut sense = sense_parts();
    sense.remove(1);
    sense[0].pins[0].3 = "g:SP";
    let v = run(&fx(&sense, Some(sense_board((20.0, 20.0), "")), None));
    assert!(is_unproven(&v, "sense_amp_far_from_shunt"));
}

#[test]
fn cross_sheet_incomplete_usb_keeps_known_bad_protection_but_not_absence() {
    let bare = incomplete_labels(&fx(&[usb_conn()], None, None));
    assert!(findings(&bare, "usb_esd_missing").is_empty());
    assert!(is_unproven(&bare, "usb_esd_missing"));
    let known = incomplete_labels(&fx(
        &[usb_conn(), usb_esd("SMAJ5.0A", "Device:D_TVS_Dual")],
        None,
        None,
    ));
    assert_eq!(findings(&known, "usb_esd_capacitance").len(), 2);
    assert_eq!(passed(&known, "usb_esd_missing"), 2);
}

#[test]
fn cross_sheet_unsupported_label_semantics_do_not_create_proven_local_joins() {
    for name in ["g:DATA{A B}", "g:DATA[0..3]", "g:${SIGNAL}", "g:{BUNDLE}"] {
        let source = schematic(&[
            res("R1", "1k", name, "LEFT"),
            res("R2", "1k", name, "RIGHT"),
        ]);
        let source = if name == "g:{BUNDLE}" {
            add_root_nodes(source, "(bus_alias \"BUNDLE\" (members \"A\" \"B\"))")
        } else {
            source
        };
        let c = joined_model(&[("root.kicad_sch", "/", source.clone())], true);
        assert!(!c.inventory_complete);
        assert_ne!(
            pin_net(&c, "R1", "1"),
            pin_net(&c, "R2", "1"),
            "unsupported label text is not a proved electrical edge"
        );
        let wired = add_root_nodes(
            source,
            "(wire (pts (xy 20 100) (xy 50 100)) (stroke (width 0) (type default)))",
        );
        let c = joined_model(&[("root.kicad_sch", "/", wired)], true);
        assert_eq!(
            pin_net(&c, "R1", "1"),
            pin_net(&c, "R2", "1"),
            "explicit wire evidence remains valid"
        );
    }
}

#[test]
fn cross_sheet_bare_alias_names_remain_proven_scalar_labels() {
    let root = add_root_nodes(
        schematic(&[res("R1", "1k", "g:LINK", "LEFT")]),
        r#"(bus_alias "LINK" (members "A" "B"))"#,
    );
    let child = schematic(&[res("R2", "1k", "g:LINK", "RIGHT")]);
    let c = joined_model(
        &[
            ("root.kicad_sch", "/", root),
            ("child.kicad_sch", "/a/", child),
        ],
        true,
    );
    assert!(
        !c.inventory_complete,
        "unsupported alias declaration remains disclosed"
    );
    assert_eq!(
        pin_net(&c, "R1", "1"),
        pin_net(&c, "R2", "1"),
        "bare LINK is scalar, not {{LINK}} group syntax"
    );
}

// ─── Identity and uncertainty paths found unexecuted by scoped coverage ──────
//
// Each case pairs the incomplete or ambiguous drawing with the complete one,
// so the assertion fails when the guard is removed and when it is over-applied.

/// One sheet holding both units of `U1` (unit 1: pins 1 and 8; unit 2: pins 2
/// and 8). `second_value` is unit 2's value and `shared` the net of its pin 8.
fn two_units_on_one_sheet(second_value: &'static str, shared: &'static str) -> String {
    let mut source = schematic(&[
        p(
            "U1",
            "Fixture:Dual",
            "DUAL",
            vec![("1", "A", "input", "A"), ("8", "GND", "power_in", "GND")],
        ),
        p(
            "U1",
            "Fixture:Dual",
            second_value,
            vec![("2", "B", "input", "B"), ("8", "GND", "power_in", shared)],
        ),
    ])
    .replace(
        "(symbol \"Dual_1_1\"",
        "(symbol \"Dual_2_1\" (pin input line (at 0 0 0) (length 2.54) (name \"B\") (number \"2\")) (pin power_in line (at 0 2.54 0) (length 2.54) (name \"GND\") (number \"8\"))) (symbol \"Dual_1_1\"",
    );
    let second = source.rfind("(unit 1) (in_bom").unwrap();
    source.replace_range(second..second + "(unit 1)".len(), "(unit 2)");
    source
}

fn has_gap(c: &crate::tools::circuit_model::Circuit, text: &str) -> bool {
    c.connectivity_gaps.iter().any(|gap| gap.contains(text))
}

#[test]
fn cross_sheet_units_of_one_part_on_one_sheet_merge_only_when_identity_and_shared_pins_agree() {
    // Compatible units: one part, the shared pin kept once.
    let c = joined_model(
        &[("root.kicad_sch", "/", two_units_on_one_sheet("DUAL", "GND"))],
        true,
    );
    assert!(c.inventory_complete, "{:?}", c.connectivity_gaps);
    assert_eq!(
        c.part("U1")
            .unwrap()
            .pins
            .iter()
            .map(|p| p.number.as_str())
            .collect::<Vec<_>>(),
        ["1", "2", "8"]
    );
    // The shared pin drawn on two different nets: which one is real is unknown.
    let c = joined_model(
        &[(
            "root.kicad_sch",
            "/",
            two_units_on_one_sheet("DUAL", "OTHER"),
        )],
        true,
    );
    assert!(
        c.part("U1").is_none(),
        "a conflicting pin is not dropped silently"
    );
    assert!(has_gap(&c, "ambiguous component/unit/pin identity for U1"));
    assert!(!c.inventory_complete);
    // Two symbols that share a designator but not a value are not one part.
    let c = joined_model(
        &[(
            "root.kicad_sch",
            "/",
            two_units_on_one_sheet("OTHER_PART", "GND"),
        )],
        true,
    );
    assert!(c.part("U1").is_none());
    assert!(has_gap(&c, "ambiguous component/unit/pin identity for U1"));
    assert!(!c.inventory_complete);
}

#[test]
fn cross_sheet_one_sheet_placing_the_same_unit_twice_is_a_gap_even_when_the_nets_agree() {
    let c = joined_model(
        &[(
            "root.kicad_sch",
            "/",
            schematic(&[res("R1", "1k", "A", "B"), res("R1", "1k", "A", "B")]),
        )],
        true,
    );
    assert!(
        c.part("R1").is_none(),
        "two symbols claiming R1 unit 1 are not one known part"
    );
    assert!(has_gap(&c, "ambiguous component/unit/pin identity for R1"));
    assert!(!c.inventory_complete);
    // Every net of the sheet is then uncertain: the unknown part may sit on any.
    assert!(c.nets.values().all(|net| net.leaves_sheet));
}

#[test]
fn cross_sheet_sheet_symbols_without_identity_or_readable_ports_are_named_gaps() {
    let child = || {
        schematic(&[res("R2", "1k", "LINK", "RET")]).replace(
            "(label \"LINK\"",
            "(hierarchical_label \"LINK\" (shape input)",
        )
    };
    let root_with = |sheet: &str| {
        add_root_nodes(
            schematic(&[res("R1", "1k", "INPUT", "RET")]),
            &format!("{sheet}(label \"INPUT\" (at 60 30 0))"),
        )
    };
    let properties = "(property \"Sheetname\" \"a\" (at 20 19 0)) (property \"Sheetfile\" \"child.kicad_sch\" (at 20 51 0))";
    let port = "(pin \"LINK\" input (at 60 30 0) (effects (font (size 1.27 1.27))))";
    for (sheet, gap) in [
        // No UUID: the child use this symbol names cannot be identified.
        (
            format!("(sheet (at 20 20) (size 40 30) {properties} {port})"),
            "sheet has no usable instance/file identity",
        ),
        // No file property.
        (
            format!(
                "(sheet (at 20 20) (size 40 30) (uuid \"a\") (property \"Sheetname\" \"a\" (at 20 19 0)) {port})"
            ),
            "sheet has no usable instance/file identity",
        ),
        // A port with no position cannot be attached to a parent net.
        (
            format!(
                "(sheet (at 20 20) (size 40 30) (uuid \"a\") {properties} (pin \"LINK\" input (effects (font (size 1.27 1.27)))))"
            ),
            "malformed sheet port",
        ),
    ] {
        let c = joined_model(
            &[
                ("root.kicad_sch", "/", root_with(&sheet)),
                ("child.kicad_sch", "/a/", child()),
            ],
            true,
        );
        assert!(has_gap(&c, gap), "{gap}: {:?}", c.connectivity_gaps);
        assert!(!c.inventory_complete);
        assert_ne!(
            pin_net(&c, "R1", "1"),
            pin_net(&c, "R2", "1"),
            "{gap}: an unreadable sheet symbol proves no join"
        );
        assert!(c.net(pin_net(&c, "R1", "1")).unwrap().leaves_sheet);
        assert!(c.net(pin_net(&c, "R2", "1")).unwrap().leaves_sheet);
    }
    // The same drawing with a complete sheet symbol joins and is complete.
    let c = joined_model(
        &[
            (
                "root.kicad_sch",
                "/",
                root_with(&format!(
                    "(sheet (at 20 20) (size 40 30) (uuid \"a\") {properties} {port})"
                )),
            ),
            ("child.kicad_sch", "/a/", child()),
        ],
        true,
    );
    assert!(c.inventory_complete, "{:?}", c.connectivity_gaps);
    assert_eq!(pin_net(&c, "R1", "1"), pin_net(&c, "R2", "1"));
}

#[test]
fn cross_sheet_unsupported_port_names_do_not_join_parent_and_child() {
    for name in ["DATA[0..3]", "{BUNDLE}", "${SIGNAL}"] {
        let root = add_root_nodes(
            schematic(&[res("R1", "1k", "INPUT", "RET")]),
            &(hierarchy_sheet("child.kicad_sch", "a", &[(name, 30.0)])
                + "(label \"INPUT\" (at 60 30 0))"),
        );
        let child = schematic(&[res("R2", "1k", "PORT", "RET")]).replace(
            "(label \"PORT\"",
            &format!("(hierarchical_label \"{name}\" (shape input)"),
        );
        let c = joined_model(
            &[
                ("root.kicad_sch", "/", root),
                ("child.kicad_sch", "/a/", child),
            ],
            true,
        );
        assert!(
            has_gap(&c, "unsupported hierarchical port name"),
            "{name}: {:?}",
            c.connectivity_gaps
        );
        assert!(has_gap(&c, "unsupported bus or variable label"));
        assert!(!c.inventory_complete);
        assert_ne!(
            pin_net(&c, "R1", "1"),
            pin_net(&c, "R2", "1"),
            "{name}: bus or variable port text is not a proved scalar join"
        );
        assert!(c.net(pin_net(&c, "R1", "1")).unwrap().leaves_sheet);
    }
}

#[test]
fn cross_sheet_unbalanced_brace_names_are_not_scalar_joins() {
    for name in ["g:A}B", "g:~{RESET", "g:}", "g:A{B"] {
        let c = joined_model(
            &[
                (
                    "root.kicad_sch",
                    "/",
                    schematic(&[res("R1", "1k", name, "A")]),
                ),
                (
                    "child.kicad_sch",
                    "/a/",
                    schematic(&[res("R2", "1k", name, "B")]),
                ),
            ],
            true,
        );
        assert!(
            has_gap(&c, "unsupported bus or variable label"),
            "{name}: {:?}",
            c.connectivity_gaps
        );
        assert!(!c.inventory_complete);
        assert_ne!(pin_net(&c, "R1", "1"), pin_net(&c, "R2", "1"), "{name}");
    }
}

#[test]
fn cross_sheet_hidden_power_input_pins_are_a_gap_and_visible_ones_are_not() {
    let logic = || {
        schematic(&[p(
            "U1",
            "Fixture:Logic",
            "74HC00",
            vec![("1", "A", "input", "A"), ("14", "VCC", "power_in", "VCC")],
        )])
    };
    // KiCad 8 and later write `(hide yes)`; older files a bare `hide`.
    for hide in ["(hide yes)", "hide"] {
        let source = logic().replace(
            "(length 2.54) (name \"VCC\")",
            &format!("(length 2.54) {hide} (name \"VCC\")"),
        );
        assert!(source.contains(hide));
        let c = joined_model(&[("root.kicad_sch", "/", source)], true);
        assert!(
            has_gap(
                &c,
                "unsupported hidden-power or internal-jumper connectivity on U1"
            ),
            "{hide}: {:?}",
            c.connectivity_gaps
        );
        assert!(!c.inventory_complete);
    }
    // A hidden pin that is not a power input does not connect by name.
    let source = logic().replace(
        "(length 2.54) (name \"A\")",
        "(length 2.54) (hide yes) (name \"A\")",
    );
    let c = joined_model(&[("root.kicad_sch", "/", source)], true);
    assert!(c.inventory_complete, "{:?}", c.connectivity_gaps);
    let c = joined_model(&[("root.kicad_sch", "/", logic())], true);
    assert!(c.inventory_complete, "{:?}", c.connectivity_gaps);
}

fn unproven_for<'a>(v: &'a Value, rule: &str) -> Vec<&'a Value> {
    v["unproven"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|u| u["rule"] == rule)
        .collect()
}

#[test]
fn cross_sheet_incomplete_decoupling_withholds_a_far_ground_leg_and_keeps_a_near_one() {
    let board_with_ground_pad_at = |dx: f64| {
        board(
            &[
                u1(),
                fp(
                    "C1",
                    10.0,
                    12.0,
                    &[("1", 0.0, 0.0, "+3V3"), ("2", dx, 0.0, "GND")],
                ),
            ],
            "",
        )
    };
    // Supply leg 2 mm, ground leg over 4 mm (limit 2.5 mm).
    let far = board_with_ground_pad_at(6.0);
    let complete = run(&fx(&decoupled_parts(), Some(far.clone()), None));
    assert_eq!(findings(&complete, "decoupling_ground_far").len(), 1);
    let v = incomplete_labels(&fx(&decoupled_parts(), Some(far), None));
    assert_eq!(passed(&v, "decoupling_supply_far"), 1, "{v}");
    assert!(findings(&v, "decoupling_ground_far").is_empty(), "{v}");
    assert_eq!(passed(&v, "decoupling_ground_far"), 0);
    let withheld = unproven_for(&v, "decoupling_ground_far");
    assert_eq!(withheld.len(), 1, "{v}");
    assert_eq!(withheld[0]["component"], "U1");
    assert_eq!(withheld[0]["pin"], "1");
    // A short ground leg is a witness no unseen capacitor can undo.
    let v = incomplete_labels(&fx(
        &decoupled_parts(),
        Some(board_with_ground_pad_at(1.0)),
        None,
    ));
    assert_eq!(passed(&v, "decoupling_ground_far"), 1, "{v}");
    assert!(unproven_for(&v, "decoupling_ground_far").is_empty());
}

#[test]
fn cross_sheet_series_search_into_unresolved_connectivity_withholds_only_the_series_rule() {
    // The connector net itself is complete and unprotected: that absence is
    // proven. What sits behind R1 continues on a net we cannot follow.
    let v = run(&fx(
        &[
            connector("J1", "SIG"),
            res("R1", "100", "SIG", "g:SIGP"),
            receiver("SIG"),
        ],
        None,
        None,
    ));
    assert!(findings(&v, "protection_after_series_element").is_empty());
    assert_eq!(passed(&v, "protection_after_series_element"), 0, "{v}");
    let withheld = unproven_for(&v, "protection_after_series_element");
    assert_eq!(withheld.len(), 1, "{v}");
    assert_eq!(withheld[0]["component"], "J1");
    assert_eq!(findings(&v, "connector_signal_unprotected").len(), 1);
    // With the far side known and empty, the series rule passes.
    let v = run(&fx(
        &[
            connector("J1", "SIG"),
            res("R1", "100", "SIG", "SIGP"),
            receiver("SIG"),
        ],
        None,
        None,
    ));
    assert_eq!(passed(&v, "protection_after_series_element"), 1, "{v}");
    assert!(unproven_for(&v, "protection_after_series_element").is_empty());
}

#[test]
fn cross_sheet_incomplete_connector_net_withholds_far_protection_placement_only() {
    let parts = || vec![connector("J1", "SIG"), tvs("D1", "SIG"), receiver("SIG")];
    // TVS 35 mm from the connector pad (limit 10 mm): nearer protection may be unseen.
    let far = incomplete_labels(&fx(&parts(), Some(protection_board(40.0, 60.0)), None));
    assert!(findings(&far, "esd_far_from_connector").is_empty(), "{far}");
    assert_eq!(passed(&far, "esd_far_from_connector"), 0);
    let withheld = unproven_for(&far, "esd_far_from_connector");
    assert_eq!(withheld.len(), 1, "{far}");
    assert_eq!(withheld[0]["component"], "J1");
    // TVS at the pad: a witness that stays.
    let near = incomplete_labels(&fx(&parts(), Some(protection_board(7.0, 60.0)), None));
    assert_eq!(passed(&near, "esd_far_from_connector"), 1, "{near}");
    assert!(unproven_for(&near, "esd_far_from_connector").is_empty());
    // The circuit is nearer the connector than the only protection we know of.
    let order = incomplete_labels(&fx(&parts(), Some(protection_board(14.0, 8.0)), None));
    assert!(
        findings(&order, "protection_beyond_circuit").is_empty(),
        "{order}"
    );
    assert_eq!(passed(&order, "protection_beyond_circuit"), 0);
    let withheld = unproven_for(&order, "protection_beyond_circuit");
    assert_eq!(withheld.len(), 1, "{order}");
    assert_eq!(withheld[0]["component"], "U1");
    // Protection between connector and circuit stays a pass.
    let ok = incomplete_labels(&fx(&parts(), Some(protection_board(8.0, 20.0)), None));
    assert_eq!(passed(&ok, "protection_beyond_circuit"), 1, "{ok}");
    assert!(unproven_for(&ok, "protection_beyond_circuit").is_empty());
}

#[test]
fn cross_sheet_incomplete_bus_withholds_far_transceiver_and_termination_placement() {
    let parts = || vec![can_xcvr(), can_conn(), res("R1", "120", "CANH", "CANL")];
    // Connector 70 mm away (limit 25 mm): a nearer one may be on an unread sheet.
    let far = incomplete_labels(&fx(&parts(), Some(can_board(90.0, (22.0, 14.0))), None));
    assert!(
        findings(&far, "transceiver_far_from_connector").is_empty(),
        "{far}"
    );
    assert_eq!(passed(&far, "transceiver_far_from_connector"), 0);
    let withheld = unproven_for(&far, "transceiver_far_from_connector");
    assert_eq!(withheld.len(), 1, "{far}");
    assert_eq!(withheld[0]["component"], "U2");
    // Both bus lines measured to a near connector, termination beside it.
    let near = incomplete_labels(&fx(&parts(), Some(can_board(8.0, (22.0, 14.0))), None));
    assert_eq!(passed(&near, "transceiver_far_from_connector"), 1, "{near}");
    assert!(unproven_for(&near, "transceiver_far_from_connector").is_empty());
    assert_eq!(passed(&near, "termination_far_from_endpoint"), 1);
    assert!(unproven_for(&near, "termination_far_from_endpoint").is_empty());
    // Termination far from every known endpoint: a nearer endpoint may be unseen.
    let far_r = incomplete_labels(&fx(&parts(), Some(can_board(8.0, (60.0, 60.0))), None));
    assert!(
        findings(&far_r, "termination_far_from_endpoint").is_empty(),
        "{far_r}"
    );
    assert_eq!(passed(&far_r, "termination_far_from_endpoint"), 0);
    let withheld = unproven_for(&far_r, "termination_far_from_endpoint");
    assert_eq!(withheld.len(), 1, "{far_r}");
    assert_eq!(withheld[0]["component"], "R1");
}

#[test]
fn cross_sheet_incomplete_bus_does_not_pass_a_transceiver_on_one_measured_line_or_none() {
    // Only CANH reaches this connector; CANL's nearest connector is not known.
    let one_line = || {
        p(
            "J2",
            "Connector_Generic:Conn_01x02",
            "Conn",
            vec![("1", "H", "passive", "CANH"), ("2", "G", "passive", "GND")],
        )
    };
    let near = || Some(can_board(8.0, (22.0, 14.0)));
    let complete = run(&fx(&[can_xcvr(), one_line()], near(), None));
    assert_eq!(passed(&complete, "transceiver_far_from_connector"), 1);
    let v = incomplete_labels(&fx(&[can_xcvr(), one_line()], near(), None));
    assert_eq!(passed(&v, "transceiver_far_from_connector"), 0, "{v}");
    assert_eq!(unproven_for(&v, "transceiver_far_from_connector").len(), 1);
    // No connector at all on an incomplete bus is withheld, not skipped as empty.
    let v = incomplete_labels(&fx(&[can_xcvr()], near(), None));
    assert_eq!(
        unproven_for(&v, "transceiver_far_from_connector").len(),
        1,
        "{v}"
    );
    assert_eq!(passed(&v, "transceiver_far_from_connector"), 0);
    let v = run(&fx(&[can_xcvr()], near(), None));
    assert!(unproven_for(&v, "transceiver_far_from_connector").is_empty());
}

#[test]
fn cross_sheet_incomplete_crystal_pins_withhold_a_missing_load_cap_and_an_unreadable_pin_count() {
    // XO has no load capacitor on what is known.
    let v = incomplete_labels(&fx(&xtal_parts(false), None, None));
    assert!(findings(&v, "crystal_load_cap_missing").is_empty(), "{v}");
    assert_eq!(passed(&v, "crystal_load_cap_missing"), 1);
    let withheld = unproven_for(&v, "crystal_load_cap_missing");
    assert_eq!(withheld.len(), 1, "{v}");
    assert_eq!(withheld[0]["component"], "Y1");
    let v = run(&fx(&xtal_parts(false), None, None));
    assert_eq!(findings(&v, "crystal_load_cap_missing").len(), 1);
    assert!(unproven_for(&v, "crystal_load_cap_missing").is_empty());
    // One pin reads as ground and the other continues elsewhere: the two
    // signal pins cannot be identified, which is not the same as a crystal
    // this rule does not read.
    let odd = |xi: &'static str| {
        vec![p(
            "Y1",
            "Device:Crystal",
            "8MHz",
            vec![("1", "1", "passive", xi), ("2", "2", "passive", "GND")],
        )]
    };
    let v = run(&fx(&odd("g:XI"), None, None));
    assert_eq!(unproven_for(&v, "crystal_load_cap_missing").len(), 1, "{v}");
    let v = run(&fx(&odd("XI"), None, None));
    assert!(unproven_for(&v, "crystal_load_cap_missing").is_empty());
}

#[test]
fn cross_sheet_incomplete_i2c_line_without_known_devices_or_with_an_unresolved_pull_up_is_withheld()
{
    let header = |sda: &'static str| {
        p(
            "J1",
            "Connector_Generic:Conn_01x02",
            "Conn",
            vec![("1", "SDA", "passive", sda), ("2", "GND", "passive", "GND")],
        )
    };
    // A header carries SDA off this sheet: devices and pull-ups may be there.
    let v = run(&fx(&[header("g:SDA")], None, None));
    let withheld = unproven_for(&v, "i2c_pullup_value");
    assert_eq!(withheld.len(), 1, "{v}");
    assert_eq!(withheld[0]["net"], "SDA");
    assert!(withheld[0].get("net_key").is_none(), "internal key leaked");
    let v = run(&fx(&[header("SDA")], None, None));
    assert!(unproven_for(&v, "i2c_pullup_value").is_empty());

    // R1 leaves SDA for a net that is not a known rail and continues
    // elsewhere: it may be a pull-up whose rail is named on another sheet.
    let line = |v: &Value| {
        v["interfaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["line"] == "SDA")
            .unwrap()
            .clone()
    };
    let v = run(&fx(
        &[i2c_dev(), res("R1", "4k7", "SDA", "g:PULL")],
        None,
        None,
    ));
    assert_eq!(line(&v)["connectivity_complete"], false, "{v}");
    assert!(unproven_for(&v, "i2c_pullup_value")
        .iter()
        .any(|u| u["net"] == "SDA"));
    let v = run(&fx(
        &[i2c_dev(), res("R1", "4k7", "SDA", "PULL")],
        None,
        None,
    ));
    assert_eq!(line(&v)["connectivity_complete"], true, "{v}");
    assert!(unproven_for(&v, "i2c_pullup_value").is_empty());
}

#[test]
fn cross_sheet_incomplete_uart_net_with_one_known_endpoint_is_withheld_not_ignored() {
    let lone = |tx: &'static str| vec![uart_dev("U1", tx, "RX_LOCAL")];
    let v = run(&fx(&lone("g:LINK"), None, None));
    let withheld = unproven_for(&v, "uart_tx_rx_crossed");
    assert_eq!(withheld.len(), 1, "{v}");
    assert_eq!(withheld[0]["net"], "LINK");
    assert!(findings(&v, "uart_tx_rx_crossed").is_empty());
    assert_eq!(passed(&v, "uart_tx_rx_crossed"), 0);
    // A complete net with one endpoint is not a UART link to judge.
    let v = run(&fx(&lone("LINK"), None, None));
    assert!(unproven_for(&v, "uart_tx_rx_crossed").is_empty());
}

#[test]
fn cross_sheet_incomplete_usb_data_net_away_from_any_known_connector_is_withheld() {
    let phy = |dp: &'static str, dm: &'static str| {
        p(
            "U1",
            "Fixture:USBPHY",
            "PHY",
            vec![
                ("1", "USB_DP", "bidirectional", dp),
                ("2", "USB_DM", "bidirectional", dm),
                ("3", "VDD", "power_in", "+3V3"),
                ("4", "GND", "power_in", "GND"),
            ],
        )
    };
    let v = run(&fx(&[phy("g:USB_DP", "g:USB_DM")], None, None));
    assert_eq!(unproven_for(&v, "usb_esd_missing").len(), 2, "{v}");
    assert!(findings(&v, "usb_esd_missing").is_empty());
    assert_eq!(passed(&v, "usb_esd_missing"), 0);
    let v = run(&fx(&[phy("USB_DP", "USB_DM")], None, None));
    assert!(unproven_for(&v, "usb_esd_missing").is_empty());
}

#[test]
fn cross_sheet_complete_reference_pin_without_a_capacitor_is_skipped_not_withheld() {
    let adc = || {
        vec![p(
            "U1",
            "Fixture:ADC",
            "ADC",
            vec![
                ("1", "VREF+", "input", "VREF"),
                ("2", "GND", "power_in", "GND"),
                ("3", "VDD", "power_in", "+3V3"),
            ],
        )]
    };
    let b = || {
        Some(board(
            &[fp(
                "U1",
                10.0,
                10.0,
                &[
                    ("1", 0.0, 0.0, "VREF"),
                    ("2", 0.0, 2.0, "GND"),
                    ("3", 2.0, 0.0, "+3V3"),
                ],
            )],
            "",
        ))
    };
    // Everything on the reference net is known: the absence is audit_decoupling's
    // to report, and this rule says it had nothing to place.
    let v = run(&fx(&adc(), b(), None));
    assert!(unproven_for(&v, "adc_ref_cap_far").is_empty(), "{v}");
    assert_eq!(check(&v, "adc_ref_cap_far")["skipped"], 1);
    // The same pin on a net that may continue: withheld instead.
    let v = incomplete_labels(&fx(&adc(), b(), None));
    assert_eq!(unproven_for(&v, "adc_ref_cap_far").len(), 1, "{v}");
    assert_eq!(passed(&v, "adc_ref_cap_far"), 0);
}

// ─── Cases added for mutants the first scoped mutation run left alive ────────

#[test]
fn cross_sheet_a_capacitor_to_a_ground_that_continues_elsewhere_is_still_a_known_capacitor() {
    // The supply net is complete; only the ground it returns to is global. A
    // proved ground stays a proved ground, so the far capacitor is a finding.
    let ic = p(
        "U1",
        "Fixture:MCU",
        "MCU",
        vec![
            ("1", "VDD", "power_in", "+3V3"),
            ("2", "GND", "power_in", "g:GND"),
            ("3", "PA0", "input", "SIG0"),
        ],
    );
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
    let v = run(&fx(
        &[ic, cap("C1", "100n", "+3V3", "g:GND")],
        Some(b),
        None,
    ));
    assert_eq!(findings(&v, "decoupling_supply_far").len(), 1, "{v}");
    assert!(unproven_for(&v, "decoupling_supply_far").is_empty());
}

#[test]
fn cross_sheet_incomplete_placement_limits_stay_inclusive_like_the_complete_rules() {
    // Exactly on the limit is a pass for the complete rule and stays a
    // witness when the inventory is incomplete: only "farther than" is withheld.
    let decoupled = board(
        &[
            u1(),
            fp(
                "C1",
                10.0,
                12.5,
                &[("1", 0.0, 0.0, "+3V3"), ("2", 2.0, 0.0, "GND")],
            ),
        ],
        "",
    );
    let v = incomplete_labels(&fx(&decoupled_parts(), Some(decoupled), None));
    for rule in ["decoupling_supply_far", "decoupling_ground_far"] {
        assert_eq!(passed(&v, rule), 1, "{rule}: {v}");
        assert!(unproven_for(&v, rule).is_empty(), "{rule}");
    }
    // Bulk capacitor exactly 10 mm from VIN.
    let bulk = board(
        &[
            fp(
                "U2",
                10.0,
                10.0,
                &[
                    ("1", 0.0, 0.0, "GND"),
                    ("2", 3.0, 0.0, "+3V3"),
                    ("3", 6.0, 0.0, "+5V"),
                ],
            ),
            fp(
                "C2",
                26.0,
                10.0,
                &[("1", 0.0, 0.0, "+5V"), ("2", 1.0, 0.0, "GND")],
            ),
        ],
        "",
    );
    let v = incomplete_labels(&fx(
        &[regulator("U2", "+3V3"), cap("C2", "22u", "+5V", "GND")],
        Some(bulk),
        None,
    ));
    assert_eq!(passed(&v, "bulk_cap_far"), 1, "{v}");
    assert!(unproven_for(&v, "bulk_cap_far")
        .iter()
        .all(|u| u["pin"] != "3"));
    // TVS exactly 10 mm from the connector pad; the circuit exactly the order
    // margin (1 mm) nearer than the TVS, which is not "before the protection".
    let protected = || vec![connector("J1", "SIG"), tvs("D1", "SIG"), receiver("SIG")];
    let v = incomplete_labels(&fx(&protected(), Some(protection_board(15.0, 60.0)), None));
    assert_eq!(passed(&v, "esd_far_from_connector"), 1, "{v}");
    assert!(unproven_for(&v, "esd_far_from_connector").is_empty());
    for complete in [true, false] {
        let f = fx(&protected(), Some(protection_board(14.0, 13.0)), None);
        let v = if complete {
            run(&f)
        } else {
            incomplete_labels(&f)
        };
        assert_eq!(passed(&v, "protection_beyond_circuit"), 1, "{v}");
        assert!(findings(&v, "protection_beyond_circuit").is_empty());
        assert!(unproven_for(&v, "protection_beyond_circuit").is_empty());
    }
    // Connector exactly 25 mm from the transceiver, termination exactly 25 mm
    // from the nearest endpoint pin.
    let v = incomplete_labels(&fx(
        &[can_xcvr(), can_conn(), res("R1", "120", "CANH", "CANL")],
        Some(can_board(45.0, (20.0, 35.0))),
        None,
    ));
    for rule in [
        "transceiver_far_from_connector",
        "termination_far_from_endpoint",
    ] {
        assert_eq!(passed(&v, rule), 1, "{rule}: {v}");
        assert!(unproven_for(&v, rule).is_empty(), "{rule}");
    }
}

#[test]
fn cross_sheet_a_lone_connector_pin_on_an_incomplete_signal_net_is_withheld_but_a_rail_is_not() {
    let lone = |net: &'static str| vec![connector("J1", net)];
    // Nothing else is on the net here, but the net continues: the circuit and
    // its protection may be on a sheet that was not read.
    let v = run(&fx(&lone("g:SIG"), None, None));
    let withheld = unproven_for(&v, "connector_signal_unprotected");
    assert_eq!(withheld.len(), 1, "{v}");
    assert_eq!(withheld[0]["component"], "J1");
    assert_eq!(withheld[0]["pin"], "1");
    assert!(findings(&v, "connector_signal_unprotected").is_empty());
    // A complete net with nothing attached is not a signal path.
    let v = run(&fx(&lone("SIG"), None, None));
    assert!(unproven_for(&v, "connector_signal_unprotected").is_empty());
    // A power-entry pin is a human judgement, incomplete or not.
    let v = run(&fx(&lone("g:+5V"), None, None));
    assert!(unproven_for(&v, "connector_signal_unprotected").is_empty());
}

#[test]
fn cross_sheet_one_unresolved_bus_line_makes_the_whole_bus_search_incomplete() {
    for (high, low) in [("g:CANH", "CANL"), ("CANH", "g:CANL")] {
        let v = run(&fx(
            &[
                p(
                    "U2",
                    "Fixture:CANXCVR",
                    "TCAN",
                    vec![
                        ("1", "CANH", "bidirectional", high),
                        ("2", "CANL", "bidirectional", low),
                        ("3", "VCC", "power_in", "+5V"),
                        ("4", "GND", "power_in", "GND"),
                    ],
                ),
                p(
                    "J2",
                    "Connector_Generic:Conn_01x03",
                    "Conn",
                    vec![
                        ("1", "H", "passive", high),
                        ("2", "L", "passive", low),
                        ("3", "G", "passive", "GND"),
                    ],
                ),
            ],
            None,
            None,
        ));
        assert_eq!(passed(&v, "can_bus_branches"), 0, "{high}/{low}: {v}");
        assert_eq!(unproven_for(&v, "can_bus_branches").len(), 1);
        assert_eq!(v["interfaces"][0]["connectivity_complete"], false);
    }
}

#[test]
fn cross_sheet_selectable_termination_needs_a_resistor_a_floating_node_and_a_selector() {
    let kind = |parts: &[P]| {
        run(&fx(parts, None, None))["interfaces"][0]["termination"]["kind"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let two_pin = |r: &'static str, lib: &'static str, a: &'static str, b: &'static str| {
        p(
            r,
            lib,
            "X",
            vec![("1", "A", "passive", a), ("2", "B", "passive", b)],
        )
    };
    // A slide switch (lib id carries `SW_`, not `SWITCH`) selects like a jumper.
    assert_eq!(
        kind(&[
            can_xcvr(),
            res("R1", "120", "CANH", "X"),
            two_pin("S1", "Fixture:SW_SPST", "X", "CANL"),
        ]),
        "selectable"
    );
    // A capacitor in the resistor's place is not a termination.
    assert_eq!(
        kind(&[
            can_xcvr(),
            cap("C9", "100n", "CANH", "X"),
            two_pin("JP1", "Jumper:Jumper_2_Open", "X", "CANL"),
        ]),
        "none_observed"
    );
    // A pull-down to ground with a header from ground to the other line is
    // not a selectable termination: the middle node must be a floating one.
    assert_eq!(
        kind(&[
            can_xcvr(),
            res("R1", "120", "CANH", "GND"),
            two_pin("J5", "Connector_Generic:Conn_01x02", "GND", "CANL"),
        ]),
        "none_observed"
    );
}

#[test]
fn cross_sheet_incomplete_termination_value_keeps_only_a_proven_overload() {
    // The pair itself is complete, but a series part leads off it to a net
    // that continues: another termination may sit behind it.
    let value_check = |ohms: &'static str| {
        run(&fx(
            &[
                can_xcvr(),
                can_conn(),
                res("R1", ohms, "CANH", "CANL"),
                res("R9", "22", "CANH", "g:FAR"),
            ],
            None,
            None,
        ))
    };
    let v = value_check("120");
    assert_eq!(passed(&v, "can_termination_value"), 0, "{v}");
    assert_eq!(unproven_for(&v, "can_termination_value").len(), 1);
    assert_eq!(
        v["interfaces"][0]["termination"]["connectivity_complete"],
        false
    );
    // 114 ohm is the lower edge of the accepted band, not an overload: unseen
    // parallel resistors could still pull it out of the band.
    let v = value_check("114");
    assert!(findings(&v, "can_termination_value").is_empty(), "{v}");
    assert_eq!(passed(&v, "can_termination_value"), 0);
    assert_eq!(unproven_for(&v, "can_termination_value").len(), 1);
    // Below the band stays a finding: more parallel resistance cannot raise it.
    let v = value_check("100");
    assert_eq!(findings(&v, "can_termination_value").len(), 1, "{v}");
    // A zero-ohm link is not a termination value to call overloaded.
    let v = value_check("0");
    assert!(findings(&v, "can_termination_value").is_empty(), "{v}");
    assert_eq!(passed(&v, "can_termination_value"), 0);
}

#[test]
fn cross_sheet_termination_exactly_at_the_limit_stands_when_another_endpoint_is_unmeasurable() {
    // J2 is in the schematic but not on the board, so one eligible endpoint
    // cannot be measured. R1 is exactly 25 mm from the transceiver pin: that
    // is within the limit, and no unmeasured endpoint can make it farther.
    let b = board(
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
                "R1",
                20.0,
                35.0,
                &[("1", 0.0, 0.0, "CANH"), ("2", 0.0, 2.0, "CANL")],
            ),
        ],
        "",
    );
    let v = run(&fx(
        &[can_xcvr(), can_conn(), res("R1", "120", "CANH", "CANL")],
        Some(b),
        None,
    ));
    assert_eq!(passed(&v, "termination_far_from_endpoint"), 1, "{v}");
    assert_eq!(check(&v, "termination_far_from_endpoint")["unevaluated"], 0);
}

#[test]
fn cross_sheet_incomplete_reference_cap_within_its_own_limit_but_not_every_family_s_is_withheld() {
    // A 1 uF capacitor 4 mm from VREF passes its own 5 mm limit. An unseen
    // 100 nF capacitor would be judged at 2.5 mm instead, so on an incomplete
    // net this distance proves nothing; on a complete net it is a pass.
    let parts = || {
        vec![
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
        ]
    };
    let b = || {
        Some(board(
            &[
                fp(
                    "U1",
                    10.0,
                    10.0,
                    &[
                        ("1", 0.0, 0.0, "VREF"),
                        ("2", 0.0, 2.0, "GND"),
                        ("3", 2.0, 0.0, "+3V3"),
                    ],
                ),
                fp(
                    "C1",
                    14.0,
                    10.0,
                    &[("1", 0.0, 0.0, "VREF"), ("2", 0.0, 1.0, "GND")],
                ),
            ],
            "",
        ))
    };
    let v = run(&fx(&parts(), b(), None));
    assert_eq!(passed(&v, "adc_ref_cap_far"), 1, "{v}");
    let v = incomplete_labels(&fx(&parts(), b(), None));
    assert_eq!(passed(&v, "adc_ref_cap_far"), 0, "{v}");
    assert_eq!(unproven_for(&v, "adc_ref_cap_far").len(), 1);
    assert!(findings(&v, "adc_ref_cap_far").is_empty());
}

// ─── Independent review A (2026-11-12): one reproduction per finding ─────────

// M1: a direct resistor below the band is a low bank at every end assignment.
#[test]
fn cross_sheet_a_low_direct_resistor_among_several_is_a_finding_at_any_end_assignment() {
    for (r1, r2, low) in [
        ("60", "60", true),
        ("60", "120", true),
        // 114 ohm is in the band: the bound is strict.
        ("114", "114", false),
        ("120", "120", false),
        ("240", "240", false),
    ] {
        let v = run(&fx(
            &[
                can_xcvr(),
                can_conn(),
                res("R1", r1, "CANH", "CANL"),
                res("R2", r2, "CANH", "CANL"),
            ],
            None,
            None,
        ));
        let human = v["needs_human"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["rule"] == "can_termination_value")
            .count();
        assert_eq!(
            findings(&v, "can_termination_value").len(),
            usize::from(low),
            "{r1}+{r2}: {v}"
        );
        assert_eq!(human, usize::from(!low), "{r1}+{r2}");
        assert_eq!(passed(&v, "can_termination_value"), 0);
        if low {
            let f = only(&v, "can_termination_value");
            assert_eq!(f["evidence"]["kind"], "direct");
            assert_eq!(f["evidence"]["low_bank"], json!(["R1"]));
            assert_eq!(f["evidence"]["upper_bound_ohms"], 60.0);
            assert_eq!(f["evidence"]["bank_assignment_requires_human"], true);
        }
    }
    // Unseen parallel paths cannot raise 60 ohm either.
    let v = run(&fx(
        &[
            global_can_xcvr(),
            global_can_conn("J2"),
            res("R1", "60", "g:CANH", "g:CANL"),
            res("R2", "120", "g:CANH", "g:CANL"),
        ],
        None,
        None,
    ));
    assert_eq!(findings(&v, "can_termination_value").len(), 1, "{v}");
    assert!(unproven_for(&v, "can_termination_value").is_empty());
}

// m1: the same bound holds for a split bank; a selectable one may be open.
#[test]
fn cross_sheet_incomplete_split_below_the_band_is_a_finding_but_a_selectable_one_is_withheld() {
    let split = |value: &'static str| {
        vec![
            global_can_xcvr(),
            global_can_conn("J2"),
            res("R1", value, "g:CANH", "MID"),
            res("R2", value, "g:CANL", "MID"),
        ]
    };
    let v = run(&fx(&split("30"), None, None));
    let f = only(&v, "can_termination_value");
    assert_eq!(f["evidence"]["kind"], "split");
    assert_eq!(f["evidence"]["total_ohms"], 60.0);
    assert_eq!(f["evidence"]["connectivity_complete"], false);
    assert!(unproven_for(&v, "can_termination_value").is_empty());
    // In the band on what is known: unseen parallel paths could pull it out.
    let v = run(&fx(&split("60"), None, None));
    assert!(findings(&v, "can_termination_value").is_empty(), "{v}");
    assert_eq!(passed(&v, "can_termination_value"), 0);
    assert_eq!(unproven_for(&v, "can_termination_value").len(), 1);
    // The bound needs the split's own middle node: one that continues out of
    // sight may not be the node both resistors are seen on.
    let v = run(&fx(
        &[
            global_can_xcvr(),
            global_can_conn("J2"),
            res("R1", "30", "g:CANH", "g:MID"),
            res("R2", "30", "g:CANL", "g:MID"),
        ],
        None,
        None,
    ));
    assert!(findings(&v, "can_termination_value").is_empty(), "{v}");
    assert_eq!(unproven_for(&v, "can_termination_value").len(), 1);
    let v = run(&fx(
        &[
            global_can_xcvr(),
            global_can_conn("J2"),
            res("R1", "60", "g:CANH", "X"),
            p(
                "JP1",
                "Jumper:Jumper_2_Open",
                "Jumper",
                vec![("1", "A", "passive", "X"), ("2", "B", "passive", "g:CANL")],
            ),
        ],
        None,
        None,
    ));
    assert!(findings(&v, "can_termination_value").is_empty(), "{v}");
    assert_eq!(unproven_for(&v, "can_termination_value").len(), 1);
}

// m2: a gap belongs to one use and says which, with that use's designator.
#[test]
fn cross_sheet_per_use_gaps_name_their_use_and_its_designator() {
    const A: &str = "/11111111-1111-4111-8111-111111111111/a";
    const B: &str = "/11111111-1111-4111-8111-111111111111/b";
    let child = schematic(&[
        res("R1", "1k", "LOCAL", "RET")
            .instance(A, "R1")
            .instance(B, "R101"),
        res("R2", "1k", "LOCAL", "RET")
            .instance(A, "R2")
            .instance(B, "R102"),
    ])
    .replacen("(lib_id \"Device:R\")", "(lib_id \"Missing:Symbol\")", 1);
    let c = joined_model(
        &[
            ("child.kicad_sch", "/a/", child.clone()),
            ("child.kicad_sch", "/b/", child),
        ],
        true,
    );
    let gaps: Vec<&String> = c
        .connectivity_gaps
        .iter()
        .filter(|gap| gap.contains("unresolved symbol connectivity"))
        .collect();
    assert_eq!(gaps.len(), 2, "{:?}", c.connectivity_gaps);
    assert_eq!(c.unresolved_symbols, 2, "counted per use, across uses");
    assert!(gaps
        .iter()
        .any(|gap| gap.contains("[/a/]") && gap.ends_with("for R1")));
    assert!(gaps
        .iter()
        .any(|gap| gap.contains("[/b/]") && gap.ends_with("for R101")));
}

// M2: one cause is one gap. A child use that is not there is one gap however
// many ports lead to it; each port's parent net is still an open boundary.
#[test]
fn cross_sheet_an_unavailable_child_is_one_gap_however_many_ports_lead_to_it() {
    let root = add_root_nodes(
        schematic(&[res("R1", "1k", "LEFT", "RIGHT")]),
        &(hierarchy_sheet("child.kicad_sch", "a", &[("P1", 30.0), ("P2", 40.0)])
            + "(label \"LEFT\" (at 60 30 0))(label \"RIGHT\" (at 60 40 0))"),
    );
    let c = joined_model(&[("root.kicad_sch", "/", root)], true);
    assert_eq!(
        c.connectivity_gaps.iter().collect::<Vec<_>>(),
        ["unavailable or ambiguous child instance /a/"],
    );
    assert!(!c.inventory_complete);
    for pin in ["1", "2"] {
        assert!(c.net(pin_net(&c, "R1", pin)).unwrap().leaves_sheet);
    }
}

// m4 (G2/G4): the use found at a child path must be the file the parent names.
#[test]
fn cross_sheet_a_use_loaded_from_another_file_than_the_parent_names_is_not_joined() {
    let root = add_root_nodes(
        schematic(&[res("R1", "1k", "INPUT", "RET")]),
        &(hierarchy_sheet("child.kicad_sch", "a", &[("LINK", 30.0)])
            + "(label \"INPUT\" (at 60 30 0))"),
    );
    let child = || {
        schematic(&[res("R2", "1k", "LINK", "RET")]).replace(
            "(label \"LINK\"",
            "(hierarchical_label \"LINK\" (shape input)",
        )
    };
    let c = joined_model(
        &[
            ("root.kicad_sch", "/", root.clone()),
            ("other.kicad_sch", "/a/", child()),
        ],
        true,
    );
    assert_ne!(pin_net(&c, "R1", "1"), pin_net(&c, "R2", "1"));
    assert_eq!(
        c.connectivity_gaps.iter().collect::<Vec<_>>(),
        ["unavailable or ambiguous child instance /a/"],
        "one cause: not the port, and not the stray use's own label"
    );
    assert!(c.net(pin_net(&c, "R1", "1")).unwrap().leaves_sheet);
    assert!(!c.inventory_complete);
    let c = joined_model(
        &[
            ("root.kicad_sch", "/", root),
            ("child.kicad_sch", "/a/", child()),
        ],
        true,
    );
    assert_eq!(pin_net(&c, "R1", "1"), pin_net(&c, "R2", "1"));
    assert!(c.inventory_complete, "{:?}", c.connectivity_gaps);
}

// m4 (G1): a part dropped for ambiguous identity may sit on any observed net,
// so no net of the design can prove an absence.
#[test]
fn cross_sheet_an_ambiguous_part_across_uses_leaves_every_net_uncertain() {
    let c = joined_model(
        &[
            (
                "root.kicad_sch",
                "/",
                schematic(&[res("R1", "1k", "A", "B"), res("R7", "1k", "QUIET", "ALONE")]),
            ),
            (
                "child.kicad_sch",
                "/a/",
                schematic(&[res("R1", "1k", "A", "B")]),
            ),
        ],
        true,
    );
    assert!(c.part("R1").is_none());
    assert!(has_gap(
        &c,
        "ambiguous component identity across sheet uses: R1"
    ));
    assert!(
        c.nets.values().all(|net| net.leaves_sheet),
        "R7's nets are uncertain too: the dropped R1 pins may be on them"
    );
}

// n1 (H06): identity does not depend on the order items are written in a file.
#[test]
fn cross_sheet_net_keys_do_not_depend_on_item_order_within_a_file() {
    let parts = || {
        vec![
            res("R1", "1k", "g:A", "B"),
            res("R2", "1k", "B", "g:C"),
            res("R3", "1k", "g:C", "D"),
        ]
    };
    let source = add_root_nodes(
        schematic(&parts()),
        "  (wire (pts (xy 20 97.46) (xy 50 100)) (stroke (width 0) (type default)))\n  (wire (pts (xy 50 97.46) (xy 80 100)) (stroke (width 0) (type default)))\n",
    );
    // Reverse every top-level item after `lib_symbols`: symbols, labels, wires.
    let split = source.find("  (symbol (lib_id").unwrap();
    let (head, body) = source.split_at(split);
    let body = body.strip_suffix(")\n").unwrap();
    let mut items: Vec<String> = Vec::new();
    for line in body.lines() {
        if line.starts_with("  (") {
            items.push(String::new());
        }
        let item = items.last_mut().unwrap();
        item.push_str(line);
        item.push('\n');
    }
    assert!(items.len() > 8);
    items.reverse();
    let reordered = format!("{head}{})\n", items.concat());
    assert_ne!(source, reordered);
    let a = joined_model(&[("root.kicad_sch", "/", source)], true);
    let b = joined_model(&[("root.kicad_sch", "/", reordered)], true);
    for reference in ["R1", "R2", "R3"] {
        for pin in ["1", "2"] {
            assert_eq!(pin_net(&a, reference, pin), pin_net(&b, reference, pin));
        }
    }
    assert_eq!(
        a.nets.keys().collect::<Vec<_>>(),
        b.nets.keys().collect::<Vec<_>>()
    );
    assert_eq!(pin_net(&a, "R1", "2"), pin_net(&a, "R2", "1"));
    assert_ne!(pin_net(&a, "R1", "1"), pin_net(&a, "R3", "2"));
}

// n3: `(hide no)` is a visible pin.
#[test]
fn cross_sheet_a_power_input_pin_written_hide_no_is_visible() {
    let source = schematic(&[p(
        "U1",
        "Fixture:Logic",
        "74HC00",
        vec![("1", "A", "input", "A"), ("14", "VCC", "power_in", "VCC")],
    )])
    .replace(
        "(length 2.54) (name \"VCC\")",
        "(length 2.54) (hide no) (name \"VCC\")",
    );
    assert!(source.contains("(hide no)"));
    let c = joined_model(&[("root.kicad_sch", "/", source)], true);
    assert!(c.inventory_complete, "{:?}", c.connectivity_gaps);
}

fn link_child() -> String {
    schematic(&[res("R2", "1k", "LINK", "OTHER")])
        .replace(
            "(label \"LINK\"",
            "(hierarchical_label \"LINK\" (shape input)",
        )
        .replace(
            "(label \"OTHER\"",
            "(hierarchical_label \"OTHER\" (shape input)",
        )
}

fn root_with_link_ports(ports: &[(&str, f64)], labels: &str) -> String {
    add_root_nodes(
        schematic(&[res("R1", "1k", "INPUT", "RET")]),
        &(hierarchy_sheet("child.kicad_sch", "a", ports) + labels),
    )
}

// M2: two uses at one instance path are one cause, named once.
#[test]
fn cross_sheet_a_duplicated_instance_path_is_one_gap() {
    let root = root_with_link_ports(&[("LINK", 30.0)], "(label \"INPUT\" (at 60 30 0))");
    let c = joined_model(
        &[
            ("root.kicad_sch", "/", root),
            ("child.kicad_sch", "/a/", link_child()),
            ("child.kicad_sch", "/a/", link_child()),
        ],
        true,
    );
    assert_eq!(c.connectivity_gaps.len(), 1, "{:?}", c.connectivity_gaps);
    assert!(has_gap(&c, "use quarantined"));
    assert!(c.part("R2").is_none());
    assert!(c.net(pin_net(&c, "R1", "1")).unwrap().leaves_sheet);
    assert!(!c.inventory_complete);
}

// M2: a use the caller reports is not recorded again, and neither is what
// hangs below a missing use. Both still leave the model incomplete.
#[test]
fn cross_sheet_uses_below_a_missing_use_add_no_gap_of_their_own() {
    let gaps = |c: &crate::tools::circuit_model::Circuit| {
        c.connectivity_gaps.iter().cloned().collect::<Vec<_>>()
    };
    let sheets = || {
        [
            (
                "root.kicad_sch",
                "/",
                root_with_link_ports(&[("LINK", 30.0)], "(label \"INPUT\" (at 60 30 0))"),
            ),
            ("leaf.kicad_sch", "/a/c/", link_child()),
        ]
    };
    // The caller reports /a/ itself.
    let c = joined_model_with(&sheets(), true, &["/a/"]);
    assert!(c.connectivity_gaps.is_empty(), "{:?}", c.connectivity_gaps);
    assert!(!c.inventory_complete, "a set-aside use is still missing");
    assert!(c.net(pin_net(&c, "R1", "1")).unwrap().leaves_sheet);
    assert!(c.net(pin_net(&c, "R2", "1")).unwrap().leaves_sheet);
    // Nobody reports it: the missing use is the one gap.
    let c = joined_model_with(&sheets(), true, &[]);
    assert_eq!(gaps(&c), ["unavailable or ambiguous child instance /a/"]);
    // Two levels set aside: the leaf's parent is on record though no use names it.
    let leaf = || [("leaf.kicad_sch", "/a/b/c/", link_child())];
    let c = joined_model_with(&leaf(), true, &["/a/", "/a/b/"]);
    assert!(c.connectivity_gaps.is_empty(), "{:?}", c.connectivity_gaps);
    assert!(!c.inventory_complete, "set-aside uses are still missing");
    assert!(c.net(pin_net(&c, "R2", "1")).unwrap().leaves_sheet);
    // No record of a parent anywhere: one gap for the use, not one per label.
    let c = joined_model_with(&leaf(), true, &[]);
    assert_eq!(
        gaps(&c),
        ["sheet use /a/b/c/ has hierarchical labels and no parent in this review"]
    );
    assert!(!c.inventory_complete);
}

// m3: the note is for pins that tie different parent nets, not for a repeat.
#[test]
fn cross_sheet_same_name_sheet_pins_note_only_a_tie_between_parent_nets() {
    let model = |second: &str| {
        joined_model(
            &[
                (
                    "root.kicad_sch",
                    "/",
                    root_with_link_ports(
                        &[("LINK", 30.0), ("LINK", 40.0), ("OTHER", 50.0)],
                        &format!("(label \"INPUT\" (at 60 30 0))(label \"{second}\" (at 60 40 0))"),
                    ),
                ),
                ("child.kicad_sch", "/a/", link_child()),
            ],
            true,
        )
    };
    let repeat = model("INPUT");
    assert!(
        repeat.connectivity_notes.is_empty(),
        "{:?}",
        repeat.connectivity_notes
    );
    assert!(repeat.inventory_complete, "{:?}", repeat.connectivity_gaps);
    assert_ne!(pin_net(&repeat, "R1", "1"), pin_net(&repeat, "R1", "2"));
    let tie = model("RET");
    assert_eq!(
        tie.connectivity_notes.iter().collect::<Vec<_>>(),
        ["sheet /a/ has more than one pin named LINK; their nets are one net through the child's label, as KiCad joins them"]
    );
    assert!(tie.inventory_complete, "{:?}", tie.connectivity_gaps);
    assert_eq!(pin_net(&tie, "R1", "1"), pin_net(&tie, "R1", "2"));
}

// ─── Checkpoint A: what the first mutation run left uncaught ─────────────────

// The sink current is the drop from the rail to the 0.4 V low level over the
// pull-up: 2.9 V over 1 k is under the 3 mA limit, over 910 ohm it is not.
#[test]
fn cross_sheet_i2c_sink_current_is_the_drop_from_the_rail_to_the_low_level() {
    let v = run(&fx(
        &[i2c_dev(), res("R1", "1k", "SDA", "+3V3")],
        None,
        None,
    ));
    assert!(findings(&v, "i2c_pullup_value").is_empty(), "{v}");
    assert_eq!(passed(&v, "i2c_pullup_value"), 1);
    let v = run(&fx(
        &[i2c_dev(), res("R1", "910", "SDA", "+3V3")],
        None,
        None,
    ));
    only(&v, "i2c_pullup_value");
}

// On a partial inventory only a current already over the limit is known: one
// exactly at it, or a pull-up of no resistance, proves nothing.
#[test]
fn cross_sheet_incomplete_i2c_pull_up_at_the_limit_or_of_zero_ohm_is_withheld() {
    let dev = || {
        p(
            "U1",
            "Fixture:SENSOR",
            "SENSOR",
            vec![
                ("1", "SDA", "bidirectional", "g:SDA"),
                ("2", "SCL", "input", "SCL"),
                ("3", "VDD", "power_in", "+3V4"),
                ("4", "GND", "power_in", "GND"),
            ],
        )
    };
    // (3.4 - 0.4) V over 1 k is 3 mA exactly.
    for value in ["1k", "0"] {
        let v = run(&fx(&[dev(), res("R1", value, "g:SDA", "+3V4")], None, None));
        let pull_ups = v["interfaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["line"] == "SDA")
            .unwrap()["pull_ups"]
            .as_array()
            .unwrap()
            .len();
        assert_eq!(pull_ups, 1, "{value}: the pull-up is seen: {v}");
        assert!(findings(&v, "i2c_pullup_value").is_empty(), "{value}: {v}");
        assert_eq!(passed(&v, "i2c_pullup_value"), 0, "{value}");
        assert_eq!(unproven_for(&v, "i2c_pullup_value").len(), 1, "{value}");
    }
    // Over the limit on what is seen: more pull-ups only add to it.
    let v = run(&fx(&[dev(), res("R1", "900", "g:SDA", "+3V4")], None, None));
    assert_eq!(findings(&v, "i2c_pullup_value").len(), 1, "{v}");
}

#[test]
fn cross_sheet_incomplete_uart_net_with_a_lone_receiver_is_withheld_and_other_nets_are_not() {
    let v = run(&fx(&[uart_dev("U1", "TX_LOCAL", "g:LINK")], None, None));
    let withheld = unproven_for(&v, "uart_tx_rx_crossed");
    assert_eq!(withheld.len(), 1, "{v}");
    assert_eq!(withheld[0]["net"], "LINK");
    // A net that leaves the sheet with no UART pin on it is not a UART subject.
    let v = run(&fx(&[res("R1", "1k", "g:ELSEWHERE", "GND")], None, None));
    assert!(unproven_for(&v, "uart_tx_rx_crossed").is_empty(), "{v}");
}

#[test]
fn cross_sheet_a_label_with_a_literal_slash_matches_written_either_way() {
    let c = joined_model(
        &[(
            "root.kicad_sch",
            "/",
            schematic(&[res("R1", "1k", "A/B", "GND")]),
        )],
        true,
    );
    let net = c.net(pin_net(&c, "R1", "1")).unwrap();
    assert!(net.matches_name("A/B"));
    assert!(net.matches_name("A{slash}B"));
    assert!(!net.matches_name("A"));
}

// A symbol is a power symbol, not a part, on any one of three signs.
#[test]
fn cross_sheet_a_power_library_symbol_is_not_a_part_whatever_its_designator() {
    let c = joined_model(
        &[(
            "root.kicad_sch",
            "/",
            schematic(&[
                res("R1", "1k", "A", "B"),
                p("P1", "power:Odd", "ODD", vec![("1", "ODD", "passive", "A")]),
            ]),
        )],
        true,
    );
    assert!(c.part("P1").is_none());
    assert!(c.part("R1").is_some());
    assert_eq!(c.unresolved_symbols, 0);
}

#[test]
fn cross_sheet_each_unfitted_part_is_listed_once() {
    let v = run(&fx(
        &[
            mcu("U1", "SIG0"),
            cap("C1", "100n", "+3V3", "GND").dnp(),
            cap("C2", "100n", "+3V3", "GND").dnp(),
        ],
        None,
        None,
    ));
    assert_eq!(v["coverage"]["unfitted_parts"], 2, "{v}");
}

#[test]
fn cross_sheet_a_sheet_pin_with_no_name_is_a_named_gap() {
    let c = joined_model(
        &[
            (
                "root.kicad_sch",
                "/",
                root_with_link_ports(&[("", 30.0)], "(label \"INPUT\" (at 60 30 0))"),
            ),
            (
                "child.kicad_sch",
                "/a/",
                schematic(&[res("R2", "1k", "X", "Y")]),
            ),
        ],
        true,
    );
    assert!(
        has_gap(&c, "unsupported hierarchical port name"),
        "{:?}",
        c.connectivity_gaps
    );
    assert!(!has_gap(&c, "unresolved hierarchical port"));
}

// What any sheet knows of a joined net holds for the whole net.
#[test]
fn cross_sheet_a_joined_net_keeps_what_each_sheet_knows_of_it() {
    let root = add_root_nodes(
        schematic(&[
            res("R1", "1k", "INPUT", "FEED"),
            p(
                "#PWR01",
                "power:VSRC",
                "VSRC",
                vec![("1", "VSRC", "power_in", "FEED")],
            ),
            res("R3", "1k", "g:SAME", "X3"),
            res("R4", "1k", "g:SAME", "X4"),
        ])
        .replace(
            "(symbol \"power:VSRC\"\n",
            "(symbol \"power:VSRC\" (power)\n",
        ),
        &(hierarchy_sheet(
            "child.kicad_sch",
            "a",
            &[("LINK", 30.0), ("OTHER", 50.0)],
        ) + &hierarchy_sheet("gone.kicad_sch", "b", &[("P", 40.0)])
            + "(label \"INPUT\" (at 60 30 0))(label \"INPUT\" (at 60 40 0))(label \"FEED\" (at 60 50 0))"),
    );
    assert!(root.contains("(power)"));
    let c = joined_model(
        &[
            ("root.kicad_sch", "/", root),
            ("child.kicad_sch", "/a/", link_child()),
        ],
        true,
    );
    // INPUT also runs into the missing sheet b: the whole joined net is open.
    let input = c.net(pin_net(&c, "R1", "1")).unwrap();
    assert_eq!(pin_net(&c, "R1", "1"), pin_net(&c, "R2", "1"));
    assert!(input.leaves_sheet);
    assert!(input.sheet_path.is_none(), "it spans two sheets");
    // FEED carries a power symbol on the root; the child's OTHER is the same net.
    let feed = c.net(pin_net(&c, "R1", "2")).unwrap();
    assert_eq!(pin_net(&c, "R1", "2"), pin_net(&c, "R2", "2"));
    assert!(feed.power_symbol);
    // Two global labels on one sheet are one net of that sheet.
    let same = c.net(pin_net(&c, "R3", "1")).unwrap();
    assert_eq!(pin_net(&c, "R3", "1"), pin_net(&c, "R4", "1"));
    assert_eq!(same.sheet_path.as_deref(), Some("/"));
}

/// One unit of the two-unit part U1 alone on a sheet. Unit 1 has pin 1, unit
/// 2 pin 2; both carry the shared pin 8.
fn dual_unit_sheet(unit: u8, value: &'static str, shared_name: &'static str) -> String {
    let (number, name) = if unit == 1 { ("1", "A") } else { ("2", "B") };
    schematic(&[p(
        "U1",
        "Fixture:Dual",
        value,
        vec![
            (number, name, "input", name),
            ("8", shared_name, "power_in", "g:GND"),
        ],
    )])
    .replace(
        "(symbol \"Dual_1_1\"",
        &format!("(symbol \"Dual_{unit}_1\""),
    )
    .replace("(unit 1) (in_bom", &format!("(unit {unit}) (in_bom"))
}

// Units of one part on different sheets are one part only when each sign of
// identity agrees: the value, and the name of a pin both units carry.
#[test]
fn cross_sheet_units_on_different_sheets_merge_only_when_every_sign_of_identity_agrees() {
    let model = |value: &'static str, shared: &'static str| {
        joined_model(
            &[
                ("root.kicad_sch", "/", dual_unit_sheet(1, "DUAL", "GND")),
                ("child.kicad_sch", "/a/", dual_unit_sheet(2, value, shared)),
            ],
            true,
        )
    };
    let one = model("DUAL", "GND");
    assert!(one.inventory_complete, "{:?}", one.connectivity_gaps);
    assert_eq!(
        one.part("U1")
            .unwrap()
            .pins
            .iter()
            .map(|p| p.number.as_str())
            .collect::<Vec<_>>(),
        ["1", "2", "8"]
    );
    for (value, shared) in [("OTHER", "GND"), ("DUAL", "VSS")] {
        let c = model(value, shared);
        assert!(c.part("U1").is_none(), "{value}/{shared}");
        assert!(
            has_gap(&c, "ambiguous component identity across sheet uses: U1"),
            "{value}/{shared}: {:?}",
            c.connectivity_gaps
        );
    }
}

// The divider's own resistors are known whatever else their far nets do; a
// resistor from FB to a net that continues out of sight is not.
#[test]
fn cross_sheet_feedback_divider_stands_on_its_own_resistors_not_on_where_the_output_goes() {
    let v = run(&fx(
        &buck_on("g:+3V3", "31.6k", "10k"),
        None,
        Some(rails("U2", 3.3, &["+3V3"])),
    ));
    assert_eq!(
        v["power_tree"]["rails"][0]["feedback"]["status"], "matches_record",
        "{v}"
    );
    let mut parts = buck_on("+3V3", "31.6k", "10k");
    parts.push(res("R3", "100k", "FB", "g:ELSEWHERE"));
    let v = run(&fx(&parts, None, Some(rails("U2", 3.3, &["+3V3"]))));
    assert_eq!(
        v["power_tree"]["rails"][0]["feedback"]["status"], "unevaluated",
        "{v}"
    );
}

// A 100 nF reference capacitor exactly at its 2.5 mm limit is within every
// family's limit, so the distance stands even on an incomplete net.
#[test]
fn cross_sheet_incomplete_reference_cap_exactly_at_the_tightest_limit_still_passes() {
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
        cap("C1", "100n", "VREF", "GND"),
    ];
    let b = board(
        &[
            fp(
                "U1",
                10.0,
                10.0,
                &[
                    ("1", 0.0, 0.0, "VREF"),
                    ("2", 0.0, 2.0, "GND"),
                    ("3", 2.0, 0.0, "+3V3"),
                ],
            ),
            fp(
                "C1",
                12.5,
                10.0,
                &[("1", 0.0, 0.0, "VREF"), ("2", 0.0, 1.0, "GND")],
            ),
        ],
        "",
    );
    let v = incomplete_labels(&fx(&parts, Some(b), None));
    assert_eq!(passed(&v, "adc_ref_cap_far"), 1, "{v}");
    assert!(unproven_for(&v, "adc_ref_cap_far").is_empty());
    assert!(findings(&v, "adc_ref_cap_far").is_empty());
}

// ─── Independent review B (2026-11-12) ───────────────────────────────────────

// R-B1: a split bank whose middle node is known bounds its bus end like a
// direct resistor does, among several candidates too.
#[test]
fn cross_sheet_a_low_split_bank_among_several_candidates_is_a_finding_at_any_end_assignment() {
    let run_with = |extra: Vec<P>| {
        let mut parts = vec![can_xcvr(), can_conn()];
        parts.extend(extra);
        run(&fx(&parts, None, None))
    };
    let human = |v: &Value| {
        v["needs_human"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["rule"] == "can_termination_value")
            .count()
    };
    // 120 ohm direct, and 30 + 30 through a middle node of this sheet.
    let v = run_with(vec![
        res("R1", "120", "CANH", "CANL"),
        res("R2", "30", "CANH", "MID"),
        res("R3", "30", "CANL", "MID"),
    ]);
    let f = only(&v, "can_termination_value");
    assert_eq!(f["evidence"]["kind"], "split", "{f}");
    assert_eq!(f["evidence"]["low_bank"], json!(["R2", "R3"]));
    assert_eq!(f["evidence"]["upper_bound_ohms"], 60.0);
    assert_eq!(f["evidence"]["bank_assignment_requires_human"], true);
    assert_eq!(human(&v), 0);
    // Two split banks: the lower one is named.
    let v = run_with(vec![
        res("R2", "30", "CANH", "MID"),
        res("R3", "30", "CANL", "MID"),
        res("R4", "27", "CANH", "MID2"),
        res("R5", "27", "CANL", "MID2"),
    ]);
    let f = only(&v, "can_termination_value");
    assert_eq!(f["evidence"]["low_bank"], json!(["R4", "R5"]));
    assert_eq!(f["evidence"]["upper_bound_ohms"], 54.0);
    // Nothing below the band: which end holds which bank is a human's call.
    let v = run_with(vec![
        res("R1", "120", "CANH", "CANL"),
        res("R2", "60", "CANH", "MID"),
        res("R3", "60", "CANL", "MID"),
    ]);
    assert!(findings(&v, "can_termination_value").is_empty(), "{v}");
    assert_eq!(human(&v), 1);
    // 57 + 57 is 114 ohm, the edge of the band and inside it.
    let v = run_with(vec![
        res("R1", "120", "CANH", "CANL"),
        res("R2", "57", "CANH", "MID"),
        res("R3", "57", "CANL", "MID"),
    ]);
    assert!(findings(&v, "can_termination_value").is_empty(), "{v}");
    assert_eq!(human(&v), 1);
    // A middle node that continues out of sight may not be one node.
    let v = run_with(vec![
        res("R1", "120", "CANH", "CANL"),
        res("R2", "30", "CANH", "g:MID"),
        res("R3", "30", "CANL", "g:MID"),
    ]);
    assert!(findings(&v, "can_termination_value").is_empty(), "{v}");
    assert_eq!(human(&v), 1);
    // A selectable bank may be open.
    let v = run_with(vec![
        res("R1", "120", "CANH", "CANL"),
        res("R2", "60", "CANH", "X"),
        p(
            "JP1",
            "Jumper:Jumper_2_Open",
            "Jumper",
            vec![("1", "A", "passive", "X"), ("2", "B", "passive", "CANL")],
        ),
    ]);
    assert!(findings(&v, "can_termination_value").is_empty(), "{v}");
    assert_eq!(human(&v), 1);
}

// R-M2b: a sheet symbol with no UUID is one cause. The hierarchy walker gives
// its child the instance path `//`; that child's labels are where it bites.
#[test]
fn cross_sheet_a_sheet_symbol_with_no_uuid_is_one_gap_whatever_its_child_holds() {
    let root = add_root_nodes(
        schematic(&[res("R1", "1k", "INPUT", "RET")]),
        "(sheet (at 20 20) (size 40 30) (property \"Sheetname\" \"a\" (at 20 19 0)) (property \"Sheetfile\" \"child.kicad_sch\" (at 20 51 0)) (pin \"LINK\" input (at 60 30 0) (effects (font (size 1.27 1.27)))))(label \"INPUT\" (at 60 30 0))",
    );
    let c = joined_model(
        &[
            ("root.kicad_sch", "/", root),
            ("child.kicad_sch", "//", link_child()),
        ],
        true,
    );
    assert_eq!(c.connectivity_gaps.len(), 1, "{:?}", c.connectivity_gaps);
    assert!(has_gap(&c, "sheet has no usable instance/file identity"));
    assert!(!c.inventory_complete);
    assert_ne!(pin_net(&c, "R1", "1"), pin_net(&c, "R2", "1"));
    for (reference, pin) in [("R1", "1"), ("R2", "1"), ("R2", "2")] {
        assert!(c.net(pin_net(&c, reference, pin)).unwrap().leaves_sheet);
    }
}

// R-n2: no production path is empty (every one is `/…/`), but a caller of the
// join could pass one. It is no identity, so the use is quarantined.
#[test]
fn cross_sheet_a_use_with_an_empty_instance_path_is_quarantined() {
    let c = joined_model(
        &[(
            "root.kicad_sch",
            "",
            schematic(&[res("R1", "1k", "A", "B")]),
        )],
        true,
    );
    assert!(has_gap(&c, "use quarantined"), "{:?}", c.connectivity_gaps);
    assert!(c.part("R1").is_none());
    assert!(!c.inventory_complete);
}
