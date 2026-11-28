//! Tests of `width_plan` and of the export's via rule (Row TW): one test, and a negative
//! control recorded in the handoff, for each guard of docs/TRACK_WIDTH_PLAN_DESIGN.md.

use super::*;
use crate::tools::design_intent::{DiffPair, Interface};
use konnect_ipc::IpcRoutingRules;
use serde_json::json;

/// The public `pours` fixture (2 layers, twelve 0805 resistors, pads 1 × 1.25 mm, a GND
/// pour on each side), with net S1 renamed 12V and S2 renamed 5V.
fn board() -> String {
    include_str!("../tests/fixtures/specctra_pours_routing.kicad_pcb")
        .replace("(net \"S1\")", "(net \"12V\")")
        .replace("(net \"S2\")", "(net \"5V\")")
}

fn nets() -> Vec<&'static str> {
    vec!["12V", "5V", "GND", "S3", "S4", "S5", "S6"]
}

/// Every net at KiCad's Default: 0.2 mm, via 0.6/0.3.
fn rules() -> IpcEffectiveRoutingRules {
    nets()
        .into_iter()
        .map(|net| {
            (
                net.to_string(),
                IpcRoutingRules {
                    class_name: "Default".into(),
                    constituents: vec!["Default".into()],
                    track_width_mm: Some(0.2),
                    clearance_mm: Some(0.2),
                    via_diameter_mm: Some(0.6),
                    via_drill_mm: Some(0.3),
                },
            )
        })
        .collect()
}

fn roles(ground: &[&str], power: &[&str]) -> SupplyRoles {
    SupplyRoles {
        ground: ground
            .iter()
            .map(|n| (n.to_string(), "name".to_string()))
            .collect(),
        power: power
            .iter()
            .map(|n| (n.to_string(), "name".to_string()))
            .collect(),
        netclass: BTreeMap::new(),
    }
}

fn intent(value: serde_json::Value) -> DesignIntent {
    let mut value = value;
    value["schema_version"] = json!(1);
    serde_json::from_value(value).unwrap()
}

fn poured_set(nets: &[&str]) -> BTreeSet<String> {
    poured(nets)
}

fn poured(nets: &[&str]) -> BTreeSet<String> {
    nets.iter().map(|n| n.to_string()).collect()
}

fn plan_of(
    intent: Option<&DesignIntent>,
    roles: &SupplyRoles,
    poured: &BTreeSet<String>,
    given: Option<&BTreeMap<String, f64>>,
) -> WidthPlan {
    let text = board();
    let rules = rules();
    plan(&Inputs {
        board_text: &text,
        rules: &rules,
        intent,
        roles,
        poured,
        given,
        locked_vias: &BTreeSet::new(),
    })
    .unwrap()
}

fn sized<'a>(plan: &'a WidthPlan, net: &str) -> &'a SizedNet {
    plan.sized
        .iter()
        .find(|n| n.net == net)
        .unwrap_or_else(|| panic!("{net} is not sized: {plan:#?}"))
}

fn given(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|(n, a)| (n.to_string(), *a)).collect()
}

fn default_copper() -> Copper {
    let tree = konnect_sexp::parse_sexp(&board()).unwrap();
    trace_current::copper_for(&tree, &DesignIntent::empty())
}

fn two_layers() -> Vec<String> {
    vec!["F.Cu".to_string(), "B.Cu".to_string()]
}

// ─── The width and the via ───────────────────────────────────────────────────

/// G1: the width is the audit's own: at the planned width the audit's capacity holds
/// the current, a micrometre less does not (the round-up never undershoots).
#[test]
fn the_needed_width_is_the_narrowest_the_audit_passes() {
    let copper = default_copper();
    for amps in [0.5, 1.0, 2.0, 3.5] {
        let (width, layer) = needed_width_mm(amps, 10.0, &two_layers(), &copper);
        let thickness = copper.thickness(&layer).0;
        let at = trace_current::capacity_a(width * thickness, 10.0, true);
        let below = trace_current::capacity_a((width - 0.001) * thickness, 10.0, true);
        assert!(at >= amps - 1e-9, "{amps} A: {width} mm carries {at}");
        assert!(below < amps, "{amps} A: {width} mm is not the narrowest");
    }
    // 1 A on 1 oz outer copper at 10 °C: 0.30 mm.
    let (one, _) = needed_width_mm(1.0, 10.0, &two_layers(), &copper);
    assert!((one - 0.302).abs() < 0.002, "{one}");
}

/// G2: an inner layer needs more copper, so it decides the width.
#[test]
fn the_worst_layer_decides_the_width() {
    let copper = default_copper();
    let layers = vec!["F.Cu".to_string(), "In1.Cu".to_string(), "B.Cu".to_string()];
    let (outer, _) = needed_width_mm(1.0, 10.0, &two_layers(), &copper);
    let (inner, layer) = needed_width_mm(1.0, 10.0, &layers, &copper);
    assert_eq!(layer, "In1.Cu");
    assert!(inner > 2.0 * outer, "{inner} vs {outer}");
}

const CLASS: ViaSize = ViaSize {
    diameter_mm: 0.6,
    drill_mm: 0.3,
};

/// G3: a class via that carries the current is kept.
#[test]
fn a_via_that_carries_the_current_is_kept() {
    let choice = via_for_current(0.5, 10.0, CLASS);
    assert!(!choice.changed);
    assert_eq!(choice.via, CLASS);
    assert_eq!(choice.per_layer_change, 1);
}

/// G4: a class via that cannot carry it becomes the power via.
#[test]
fn a_via_that_cannot_carry_the_current_becomes_the_power_via() {
    // 0.3 mm drill: about 0.9 A; 0.4 mm: about 1.1 A.
    assert!(trace_current::via_capacity_a(0.3, 10.0) < 1.0);
    assert!(trace_current::via_capacity_a(0.4, 10.0) > 1.0);
    let choice = via_for_current(1.0, 10.0, CLASS);
    assert!(choice.changed);
    assert_eq!(choice.via, POWER_VIA);
    assert_eq!(choice.per_layer_change, 1);
}

/// G5: a current no one via carries needs several in parallel.
#[test]
fn a_large_current_needs_vias_in_parallel() {
    let choice = via_for_current(3.0, 10.0, CLASS);
    assert_eq!(choice.via, POWER_VIA);
    let each = trace_current::via_capacity_a(0.4, 10.0);
    assert_eq!(choice.per_layer_change, (3.0 / each).ceil() as usize);
    assert!(choice.per_layer_change >= 3);
    assert_eq!(
        vias_needed(2.0, 1.0),
        2,
        "exactly 2 A over 1 A each is 2, not 3"
    );
    assert_eq!(vias_needed(0.1, 1.0), 1);
    assert_eq!(vias_needed(1.0, 0.0), usize::MAX);
}

/// G6: a class via already wider than the power via is never made smaller.
#[test]
fn a_class_via_wider_than_the_power_via_is_kept() {
    let wide = ViaSize {
        diameter_mm: 1.0,
        drill_mm: 0.5,
    };
    let choice = via_for_current(3.0, 10.0, wide);
    assert!(!choice.changed);
    assert_eq!(choice.via, wide);
    assert!(choice.per_layer_change >= 2);
    // A wide class pad with a small drill keeps its diameter.
    let pad = ViaSize {
        diameter_mm: 1.2,
        drill_mm: 0.3,
    };
    let choice = via_for_current(1.0, 10.0, pad);
    assert_eq!(choice.via.diameter_mm, 1.2);
    assert_eq!(choice.via.drill_mm, POWER_VIA.drill_mm);
}

// ─── The plan ────────────────────────────────────────────────────────────────

/// G7: an unpoured supply with no recorded current gets the stated default, sized and
/// proposed for the record; a poured ground gets none.
#[test]
fn an_unpoured_supply_gets_the_default_and_a_poured_one_does_not() {
    let plan = plan_of(None, &roles(&["GND"], &["12V"]), &poured(&["GND"]), None);
    let v12 = sized(&plan, "12V");
    assert_eq!(v12.current_a, DEFAULT_SUPPLY_CURRENT_A);
    assert_eq!(v12.current_source, PROPOSED_SOURCE);
    assert!(
        v12.width_mm > 0.29 && v12.width_mm < 0.32,
        "{}",
        v12.width_mm
    );
    assert_eq!(v12.via.via, POWER_VIA);
    assert_eq!(
        plan.to_record,
        vec![Record::Currents {
            net: "12V".to_string(),
            amps: DEFAULT_SUPPLY_CURRENT_A
        }]
    );
    let gnd = plan.not_sized.iter().find(|n| n.net == "GND").unwrap();
    assert!(gnd.why.starts_with("poured"), "{}", gnd.why);
    assert!(plan.sized.iter().all(|n| n.net != "GND"));
}

/// G7, control's other side: an unpoured ground is a supply too.
#[test]
fn an_unpoured_ground_gets_the_default() {
    let plan = plan_of(None, &roles(&["GND"], &[]), &poured(&[]), None);
    assert_eq!(sized(&plan, "GND").current_source, PROPOSED_SOURCE);
    assert_eq!(sized(&plan, "GND").role, Role::Ground);
}

/// G8: a recorded current wins: it is used, with its own rise, and not recorded again.
#[test]
fn a_recorded_current_is_used_and_not_recorded_again() {
    let record = intent(json!({ "currents": [
        { "net": "12V", "max_current_a": 2.0, "max_temp_rise_c": 20.0 }
    ]}));
    let plan = plan_of(Some(&record), &roles(&[], &["12V"]), &poured(&[]), None);
    let v12 = sized(&plan, "12V");
    assert_eq!(v12.current_a, 2.0);
    assert_eq!(v12.temp_rise_c, 20.0);
    assert_eq!(v12.current_source, "currents");
    assert!(plan.to_record.is_empty(), "{:?}", plan.to_record);
}

/// G9: a rail net with no current is recorded on its rail, once for all its nets.
#[test]
fn rail_nets_share_one_record_on_their_rail() {
    let record = intent(json!({ "rails": [
        { "name": "VIN", "nets": ["12V", "5V"], "voltage_v": 12.0 }
    ]}));
    let plan = plan_of(
        Some(&record),
        &roles(&[], &["12V", "5V"]),
        &poured(&[]),
        None,
    );
    assert_eq!(
        plan.to_record,
        vec![Record::Rails {
            rail: "VIN".to_string(),
            amps: DEFAULT_SUPPLY_CURRENT_A
        }]
    );
    assert_eq!(sized(&plan, "5V").current_a, DEFAULT_SUPPLY_CURRENT_A);
}

/// G10: a given current replaces the default, and a rail net's given current is its
/// rail's, so the rail's other nets take it too.
#[test]
fn a_given_current_replaces_the_default_and_covers_its_rail() {
    let map = given(&[("12V", 3.0)]);
    let plan = plan_of(None, &roles(&[], &["12V", "5V"]), &poured(&[]), Some(&map));
    assert_eq!(sized(&plan, "12V").current_a, 3.0);
    assert_eq!(sized(&plan, "12V").current_source, USER_SOURCE);
    assert_eq!(sized(&plan, "5V").current_source, PROPOSED_SOURCE);
    assert_eq!(plan.to_record.len(), 2);

    let record = intent(json!({ "rails": [
        { "name": "VIN", "nets": ["12V", "5V"], "voltage_v": 12.0 }
    ]}));
    let plan = plan_of(
        Some(&record),
        &roles(&[], &["12V", "5V"]),
        &poured(&[]),
        Some(&map),
    );
    assert_eq!(sized(&plan, "5V").current_a, 3.0);
    assert_eq!(sized(&plan, "5V").current_source, USER_SOURCE);
    assert_eq!(
        plan.to_record,
        vec![Record::Rails {
            rail: "VIN".to_string(),
            amps: 3.0
        }]
    );
}

/// G11: a poured net with a current keeps its class width and via (Row S9's rule).
#[test]
fn a_poured_net_keeps_its_class_width_and_via() {
    let map = given(&[("GND", 3.0)]);
    let plan = plan_of(None, &roles(&["GND"], &[]), &poured(&["GND"]), Some(&map));
    let gnd = sized(&plan, "GND");
    assert!(gnd.poured);
    assert_eq!(gnd.width_mm, 0.2);
    assert!(!gnd.via.changed);
    assert!(
        gnd.needed_mm > 1.0,
        "the need is still shown: {}",
        gnd.needed_mm
    );
}

/// G12: a differential pair gets no default even when its net reads as a supply.
#[test]
fn a_differential_pair_gets_no_default() {
    let mut record = DesignIntent::empty();
    record.interfaces.push(Interface {
        name: "usb".into(),
        kind: "usb2".into(),
        nets: vec!["S3".into(), "S4".into()],
        differential_pairs: vec![DiffPair {
            positive: "S3".into(),
            negative: "S4".into(),
            max_skew_mm: None,
        }],
        length_groups: Vec::new(),
        single_ended_impedance_ohm: None,
        differential_impedance_ohm: None,
        notes: None,
    });
    let plan = plan_of(Some(&record), &roles(&[], &["S3"]), &poured(&[]), None);
    assert!(plan.to_record.is_empty(), "{:?}", plan.to_record);
    assert_eq!(
        plan.differential_pairs,
        vec![["S3".to_string(), "S4".to_string()]]
    );
}

/// G13: the pads narrower than a widened net's width are named; none for a net that is
/// not widened.
#[test]
fn pads_narrower_than_a_widened_net_are_named() {
    let map = given(&[("12V", 5.0)]);
    let plan = plan_of(None, &roles(&[], &["12V"]), &poured(&[]), Some(&map));
    let v12 = sized(&plan, "12V");
    assert!(v12.width_mm > 1.0, "{}", v12.width_mm);
    assert_eq!(v12.narrow_pads_total, 2, "both 1 mm pads of 12V");
    assert!(plan.notes.iter().any(|n| n.contains("neck-down")));
    let plan = plan_of(None, &roles(&[], &["12V"]), &poured(&[]), None);
    assert_eq!(sized(&plan, "12V").narrow_pads_total, 0);
}

/// G14: the classes: one Power row for the widened nets, then the others by their class.
#[test]
fn the_classes_are_power_then_signal() {
    let plan = plan_of(
        None,
        &roles(&["GND"], &["12V", "5V"]),
        &poured(&["GND"]),
        None,
    );
    assert_eq!(plan.classes[0].name, "Power");
    assert_eq!(plan.classes[0].nets, vec!["12V", "5V"]);
    assert_eq!(plan.classes[0].via, POWER_VIA);
    assert_eq!(plan.classes[1].name, "Signal (Default)");
    assert!(!plan.classes[1].nets.contains(&"12V".to_string()));
    assert_eq!(plan.signals, 4, "S3 to S6");
    let map = given(&[("12V", 3.0)]);
    let plan = plan_of(None, &roles(&[], &["12V", "5V"]), &poured(&[]), Some(&map));
    assert_eq!(plan.classes[0].name, "Power 1");
    assert_eq!(plan.classes[1].name, "Power 2");
    assert!(plan.classes[0].track_width_mm < plan.classes[1].track_width_mm);
}

/// G15: a net KiCad gave no complete rule is not sized, and still recorded.
#[test]
fn a_net_without_a_rule_is_not_sized() {
    let text = board();
    let mut rules = rules();
    rules.get_mut("12V").unwrap().via_drill_mm = None;
    let roles = roles(&[], &["12V"]);
    let poured = poured(&[]);
    let plan = plan(&Inputs {
        board_text: &text,
        rules: &rules,
        intent: None,
        roles: &roles,
        poured: &poured,
        given: None,
        locked_vias: &BTreeSet::new(),
    })
    .unwrap();
    assert!(plan.sized.is_empty());
    assert!(plan.not_sized[0].why.contains("no complete routing rule"));
    assert_eq!(plan.to_record.len(), 1);
}

/// A recorded current for a net the board does not have is a note.
#[test]
fn a_recorded_net_the_board_lacks_is_noted() {
    let record = intent(json!({ "currents": [{ "net": "NOPE", "max_current_a": 1.0 }] }));
    let plan = plan_of(Some(&record), &roles(&[], &[]), &poured(&[]), None);
    assert!(
        plan.notes.iter().any(|n| n.contains("'NOPE'")),
        "{:?}",
        plan.notes
    );
}

// ─── Answers and records ─────────────────────────────────────────────────────

/// G16: every refusal of a `track_widths` map, and a map that passes.
#[test]
fn a_track_widths_map_is_refused_for_each_reason() {
    let nets: BTreeSet<String> = nets().into_iter().map(str::to_string).collect();
    let record = intent(json!({
        "rails": [{ "name": "VIN", "nets": ["12V", "5V"], "voltage_v": 12.0 }],
        "currents": [{ "net": "S3", "max_current_a": 0.2 }]
    }));
    let refused = |map: &[(&str, f64)]| answer_refusal(&given(map), &nets, Some(&record));
    assert!(refused(&[("NOPE", 1.0)]).unwrap().contains("no pad"));
    for amps in [0.0, -1.0, f64::NAN, f64::INFINITY, 100.5] {
        assert!(
            refused(&[("S4", amps)]).unwrap().contains("above 0"),
            "{amps}"
        );
    }
    assert!(refused(&[("S3", 1.0)])
        .unwrap()
        .contains("design intent wins"));
    assert_eq!(
        refused(&[("S3", 0.2)]),
        None,
        "the same current recorded already (a resumed L5) is no conflict"
    );
    assert!(refused(&[("12V", 1.0), ("5V", 2.0)])
        .unwrap()
        .contains("one current"));
    assert_eq!(refused(&[("12V", 2.0), ("5V", 2.0), ("S4", 100.0)]), None);
}

/// G17: the final records: a given net replaces its proposal by key.
#[test]
fn a_given_current_replaces_its_proposal() {
    let proposed = vec![
        Record::Currents {
            net: "12V".into(),
            amps: 1.0,
        },
        Record::Currents {
            net: "5V".into(),
            amps: 1.0,
        },
    ];
    let out = final_records(&proposed, Some(&given(&[("12V", 2.5), ("S4", 0.5)])), None);
    assert_eq!(
        out,
        vec![
            Record::Currents {
                net: "12V".into(),
                amps: 2.5
            },
            Record::Currents {
                net: "5V".into(),
                amps: 1.0
            },
            Record::Currents {
                net: "S4".into(),
                amps: 0.5
            },
        ]
    );
    assert_eq!(final_records(&proposed, None, None), proposed);
}

/// G17b: against the record as it is now, a proposal for a net that is now a rail net
/// goes to its rail, and a given rail current replaces every proposal that covers it
/// (never a `currents` entry beside its rail, which `set_design_intent` refuses).
#[test]
fn a_given_rail_current_replaces_the_proposals_that_cover_it() {
    let record = intent(json!({ "rails": [
        { "name": "VIN", "nets": ["12V", "5V"], "voltage_v": 12.0 }
    ]}));
    let proposed = vec![Record::Currents {
        net: "12V".into(),
        amps: 1.0,
    }];
    assert_eq!(
        final_records(&proposed, None, Some(&record)),
        vec![Record::Rails {
            rail: "VIN".into(),
            amps: 1.0
        }]
    );
    let out = final_records(&proposed, Some(&given(&[("5V", 2.0)])), Some(&record));
    assert_eq!(
        out,
        vec![Record::Rails {
            rail: "VIN".into(),
            amps: 2.0
        }]
    );
}

/// G18: the sections L5 writes keep what the record holds and add the new values; a
/// record already there changes nothing, and `recorded` then says so.
#[test]
fn the_sections_keep_the_record_and_add_the_currents() {
    let record = intent(json!({
        "rails": [
            { "name": "VIN", "nets": ["12V"], "voltage_v": 12.0 },
            { "name": "LOGIC", "nets": ["5V"], "voltage_v": 5.0, "max_current_a": 0.3 }
        ],
        "currents": [{ "net": "S6", "max_current_a": 0.2, "notes": "kept" }]
    }));
    let records = vec![
        Record::Rails {
            rail: "VIN".into(),
            amps: 1.0,
        },
        Record::Currents {
            net: "S3".into(),
            amps: 1.0,
        },
    ];
    assert!(!recorded(Some(&record), &records));
    let (rails, currents) = sections(Some(&record), &records);
    let rails = rails.unwrap();
    assert_eq!(rails[0].max_current_a, Some(1.0));
    assert_eq!(rails[0].notes.as_deref(), Some(RECORD_NOTE));
    assert_eq!(
        rails[1].max_current_a,
        Some(0.3),
        "another rail is untouched"
    );
    let currents = currents.unwrap();
    assert_eq!(
        currents.iter().map(|c| c.net.as_str()).collect::<Vec<_>>(),
        vec!["S3", "S6"]
    );
    assert_eq!(currents[1].notes.as_deref(), Some("kept"));
    let mut after = record.clone();
    after.rails = rails;
    after.currents = currents;
    assert!(recorded(Some(&after), &records));
    assert_eq!(sections(Some(&after), &records), (None, None));
    // No record yet: the currents section is new.
    let (rails, currents) = sections(None, &records[1..]);
    assert!(rails.is_none());
    assert_eq!(currents.unwrap().len(), 1);
    // A rail the record does not have is never "recorded".
    assert!(!recorded(None, &records[..1]));
}

// ─── The export ──────────────────────────────────────────────────────────────

fn with_intent(text: &str, record: serde_json::Value) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.kicad_pcb");
    std::fs::write(&board, text).unwrap();
    let path = crate::tools::design_intent::design_intent_path(dir.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, record.to_string()).unwrap();
    (dir, board)
}

/// The board without its two GND pours, so GND may be widened too.
fn unpoured() -> String {
    let text = board();
    let start = text.find("\t(zone\n").unwrap();
    let end = text.find("\t(embedded_fonts no)\n)").unwrap();
    format!("{}{}", &text[..start], &text[end..])
}

/// G19: the export gives a net with a recorded current the plan's width and via, as its
/// own DSN class with its own via padstack; a net that needs no new via keeps its class's.
#[test]
fn the_export_gives_a_current_its_width_and_power_via() {
    let text = unpoured();
    let (_dir, board) = with_intent(
        &text,
        json!({ "schema_version": 1, "currents": [
            { "net": "12V", "max_current_a": 1.0 },
            { "net": "5V", "max_current_a": 0.5 }
        ]}),
    );
    let (widened, report) = crate::specctra::widen_for_intent(&board, &text, &rules()).unwrap();
    let plan = plan_of(
        None,
        &roles(&[], &[]),
        &poured(&[]),
        Some(&given(&[("12V", 1.0)])),
    );
    let v12 = sized(&plan, "12V");
    assert_eq!(
        widened["12V"].track_width_mm,
        Some(v12.width_mm),
        "the plan's width"
    );
    assert_eq!(widened["12V"].via_diameter_mm, Some(POWER_VIA.diameter_mm));
    assert_eq!(widened["12V"].via_drill_mm, Some(POWER_VIA.drill_mm));
    let row = report.iter().find(|r| r["net"] == "12V").unwrap();
    assert_eq!(row["via_to_mm"], json!([0.8, 0.4]));
    assert_eq!(row["vias_per_layer_change"], json!(1));
    // 0.5 A: wider than 0.2 mm is not needed, nor a new via.
    assert_eq!(widened["5V"].via_drill_mm, Some(0.3));
    let five = report.iter().find(|r| r["net"] == "5V").unwrap();
    assert!(five.get("via_to_mm").is_none(), "{five}");
    // Into the DSN: a class of the 12V width, using a via padstack of 800 µm.
    let bundle = crate::specctra::export_dsn(&board, &text, &widened).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&bundle.manifest).unwrap();
    let via_sizes: BTreeSet<i64> = manifest["padstacks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["purpose"] == "via")
        .map(|p| p["size_x_um"].as_i64().unwrap())
        .collect();
    assert_eq!(via_sizes, BTreeSet::from([600, 800]));
    let class_of = |net: &str| {
        manifest["nets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["name"] == net)
            .unwrap()["class_name"]
            .clone()
    };
    assert_ne!(class_of("12V"), class_of("S3"));
    assert_eq!(class_of("5V"), class_of("S3"));
}

/// G20: a net with locked vias keeps its class via (the export requires a locked via to
/// equal its net's rule), and says so.
#[test]
fn a_net_with_locked_vias_keeps_its_via() {
    let text = crate::pour_plan::tests::with_node(
        &unpoured(),
        "\t(via (at 110 110) (size 0.6) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") (locked yes) (net \"12V\") (uuid \"0f0e0d0c-0b0a-4908-8706-050403020100\"))",
    );
    let (_dir, board) = with_intent(
        &text,
        json!({ "schema_version": 1, "currents": [{ "net": "12V", "max_current_a": 1.0 }] }),
    );
    let (widened, report) = crate::specctra::widen_for_intent(&board, &text, &rules()).unwrap();
    assert_eq!(widened["12V"].via_drill_mm, Some(0.3));
    assert!(
        widened["12V"].track_width_mm.unwrap() > 0.29,
        "still widened"
    );
    let row = report.iter().find(|r| r["net"] == "12V").unwrap();
    assert!(row["via_kept_reason"].as_str().unwrap().contains("locked"));
    assert_eq!(row["vias_per_layer_change"], json!(2), "0.9 A each for 1 A");
    // And the export takes it: the locked via equals its rule.
    crate::specctra::export_dsn(&board, &text, &widened).unwrap();
}

/// Review R1 major 1: a proposal the record now covers is dropped (its net, or its rail).
#[test]
fn a_proposal_the_record_now_covers_is_dropped() {
    let proposed = vec![
        Record::Currents {
            net: "12V".into(),
            amps: 1.0,
        },
        Record::Currents {
            net: "5V".into(),
            amps: 1.0,
        },
    ];
    let record = intent(json!({
        "rails": [{ "name": "VIN", "nets": ["5V"], "voltage_v": 5.0, "max_current_a": 3.0 }],
        "currents": [{ "net": "12V", "max_current_a": 4.0 }]
    }));
    assert!(final_records(&proposed, None, Some(&record)).is_empty());
    let rails = vec![Record::Rails {
        rail: "VIN".into(),
        amps: 1.0,
    }];
    assert!(final_records(&rails, None, Some(&record)).is_empty());
}

/// Review R1 major 3: a rail net's new current is checked at the rail's own rise, so the
/// plan's width is the export's once the rail's current is written.
#[test]
fn a_rail_with_its_own_rise_plans_the_width_the_export_routes() {
    let text = unpoured();
    let rail =
        json!({ "name": "VIN", "nets": ["12V"], "voltage_v": 12.0, "max_temp_rise_c": 30.0 });
    let before = intent(json!({ "rails": [rail.clone()] }));
    let plan = plan_of(
        Some(&before),
        &roles(&[], &["12V"]),
        &poured(&[]),
        Some(&given(&[("12V", 3.0)])),
    );
    let v12 = sized(&plan, "12V");
    assert_eq!(v12.temp_rise_c, 30.0);
    let mut written = rail;
    written["max_current_a"] = json!(3.0);
    let (_dir, board) = with_intent(&text, json!({ "schema_version": 1, "rails": [written] }));
    let (widened, _) = crate::specctra::widen_for_intent(&board, &text, &rules()).unwrap();
    assert_eq!(widened["12V"].track_width_mm, Some(v12.width_mm));
}

/// Review R1 minor 4: a rail's current reaches every net of the rail, so every one of
/// them with a pad is sized, whatever its role.
#[test]
fn every_net_of_a_rail_with_a_new_current_is_sized() {
    let record = intent(json!({ "rails": [
        { "name": "VIN", "nets": ["12V", "S5"], "voltage_v": 12.0 }
    ]}));
    let plan = plan_of(Some(&record), &roles(&[], &["12V"]), &poured(&[]), None);
    assert_eq!(sized(&plan, "S5").current_a, DEFAULT_SUPPLY_CURRENT_A);
    assert_eq!(sized(&plan, "S5").current_source, PROPOSED_SOURCE);
    assert!(plan.classes[0].nets.contains(&"S5".to_string()));
}

/// Review R1 minor 6: the plan keeps the class via of a net with locked vias, as the
/// export does, and says so.
#[test]
fn the_plan_keeps_the_via_of_a_net_with_locked_vias() {
    let text = crate::pour_plan::tests::with_node(
        &unpoured(),
        "\t(via (at 110 110) (size 0.6) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") (locked yes) (net \"12V\") (uuid \"0f0e0d0c-0b0a-4908-8706-050403020100\"))",
    );
    let rules = rules();
    let roles = roles(&[], &["12V"]);
    let poured = poured(&[]);
    let tree = konnect_sexp::parse_sexp(&text).unwrap();
    let locked = crate::specctra::locked_via_nets(&tree).unwrap();
    assert_eq!(locked, poured_set(&["12V"]));
    let plan = plan(&Inputs {
        board_text: &text,
        rules: &rules,
        intent: None,
        roles: &roles,
        poured: &poured,
        given: None,
        locked_vias: &locked,
    })
    .unwrap();
    let v12 = sized(&plan, "12V");
    assert!(!v12.via.changed);
    assert_eq!(v12.via.via, CLASS);
    assert_eq!(v12.via.per_layer_change, 2);
    assert!(plan.notes.iter().any(|n| n.contains("locked vias")));
}

/// Review R1 minor 8: the audit counts the vias a layer change needs with the plan's
/// own function: exactly at the boundary, one via, not two.
#[test]
fn the_audit_and_the_plan_count_parallel_vias_alike() {
    let each = trace_current::via_capacity_a(0.3, 10.0);
    assert_eq!(vias_needed(each, each), 1);
    assert_eq!(vias_needed(each * 2.0, each), 2);
}

// ─── Measurement (ignored: fastroute and kicad-cli) ──────────────────────────

/// Row TW's measurement at effort M. For each board of `TW_BOARDS` (comma-separated
/// `.kicad_pcb` paths; their projects are copied, never written): the routing stripped,
/// the pours planned and added as the flow adds them, then routed twice on scratch
/// copies, without the plan and with its proposed currents recorded, every net at
/// KiCad's Default (0.2 mm, via 0.6/0.3, as the owner's board has). Each routed session
/// is written into its copy and `audit_trace_current` runs on it. Needs
/// `KONNECT_STATE_DIR` (scratch), `KONNECT_FASTROUTE` and a scratch `KICAD_CONFIG_HOME`.
/// The record is `<state>/tw-<stamp>/tw.json`.
#[test]
#[ignore = "starts fastroute and kicad-cli: needs TW_BOARDS, KONNECT_FASTROUTE, KONNECT_STATE_DIR and a scratch KICAD_CONFIG_HOME"]
fn tw_measure_the_width_plan_at_effort_m() {
    use crate::place_judge::RouteResult;
    use crate::router::ToolRouter;
    use crate::tools::{ServerConfig, ToolContext};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    let state = PathBuf::from(std::env::var("KONNECT_STATE_DIR").expect("KONNECT_STATE_DIR"));
    let fastroute = PathBuf::from(std::env::var("KONNECT_FASTROUTE").expect("KONNECT_FASTROUTE"));
    let config_home =
        PathBuf::from(std::env::var("KICAD_CONFIG_HOME").expect("a scratch KICAD_CONFIG_HOME"));
    let kicad_cli = crate::kicad_install::find_cli("")
        .expect("kicad-cli is installed")
        .display()
        .to_string();
    let ctx = Arc::new(ToolContext::new(
        ServerConfig {
            kicad_cli: kicad_cli.clone(),
            kicad_binary: String::new(),
            ipc_address: String::new(),
            project_dir: None,
            jlcpcb_db_path: None,
            auto_load_toolsets: false,
            eager_toolsets: false,
        },
        Arc::new(ToolRouter::new()),
    ));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let root = state.join(format!("tw-{stamp}"));
    let as_named = crate::full_flow::RolesAnswer::AsNamed(crate::full_flow::AsNamed::AsNamed);
    let mut record = serde_json::Map::new();
    for (index, source) in std::env::var("TW_BOARDS")
        .expect("TW_BOARDS")
        .split(',')
        .filter(|s| !s.is_empty())
        .enumerate()
    {
        let source = Path::new(source);
        let name = format!("{index}-{}", source.file_stem().unwrap().to_string_lossy());
        let folder = root.join(&name);
        let base = crate::scratch_board::copy_project(source, &folder.join("source")).unwrap();
        let text =
            crate::place_route::strip_routing(&std::fs::read_to_string(source).unwrap()).unwrap();
        std::fs::write(&base, &text).unwrap();
        // The pours, as P4 plans them.
        let pours = folder.join("pours");
        std::fs::create_dir_all(&pours).unwrap();
        let (planned, pour_plan) = runtime
            .block_on(crate::tools::pour_plan_tool::plan_text(
                &ctx,
                &base,
                &text,
                Some(&as_named),
                Some(&pours),
            ))
            .unwrap();
        let zones = if planned.is_error {
            Vec::new()
        } else {
            pour_plan.map(|plan| plan.proposals).unwrap_or_default()
        };
        let zoned = if zones.is_empty() {
            text.clone()
        } else {
            crate::pour_plan::with_zones(&text, &zones).unwrap()
        };
        let tree = konnect_sexp::parse_sexp(&zoned).unwrap();
        let index = konnect_sexp::board::PcbConnectivityIndex::build(&tree);
        let rules: IpcEffectiveRoutingRules = index
            .nets()
            .into_iter()
            .filter(|net| !net.is_empty() && !index.pads_of_net(net).is_empty())
            .map(|net| {
                (
                    net.to_string(),
                    IpcRoutingRules {
                        class_name: "Default".to_string(),
                        constituents: Vec::new(),
                        track_width_mm: Some(0.2),
                        clearance_mm: Some(0.2),
                        via_diameter_mm: Some(0.6),
                        via_drill_mm: Some(0.3),
                    },
                )
            })
            .collect();
        let mut board_record = serde_json::Map::new();
        board_record.insert("zones_added".into(), json!(zones.len()));
        for with in [false, true] {
            let variant = if with { "with_plan" } else { "without_plan" };
            let dir = folder.join(variant);
            let board = crate::scratch_board::copy_project(&base, &dir.join("project")).unwrap();
            std::fs::write(&board, &zoned).unwrap();
            let intent_path =
                crate::tools::design_intent::design_intent_path(board.parent().unwrap());
            let read_intent = || match crate::tools::design_intent::read_design_intent(&intent_path)
            {
                crate::tools::design_intent::IntentRead::Loaded { intent, .. } => Some(*intent),
                _ => None,
            };
            let (nets, roles) = match crate::tools::pour_plan_tool::board_roles(&board, &zoned) {
                Ok(read) => read,
                Err(refusal) => {
                    println!("TW {name}: refused: {refusal:?}");
                    board_record.insert("refused".into(), json!(format!("{refusal:?}")));
                    break;
                }
            };
            let roles = crate::pour_plan::with_roles(
                &roles,
                &as_named,
                &crate::pour_plan::rails_of(&roles),
                &nets,
            )
            .unwrap();
            let poured = crate::specctra::pour_nets(&tree).unwrap();
            let intent = read_intent();
            let width_plan = plan(&Inputs {
                board_text: &zoned,
                rules: &rules,
                intent: intent.as_ref(),
                roles: &roles,
                poured: &poured,
                given: None,
                locked_vias: &BTreeSet::new(),
            })
            .unwrap();
            if with {
                let records = final_records(&width_plan.to_record, None, intent.as_ref());
                let (rails, currents) = sections(intent.as_ref(), &records);
                let mut written = intent.clone().unwrap_or_else(DesignIntent::empty);
                if let Some(rails) = rails {
                    written.rails = rails;
                }
                if let Some(currents) = currents {
                    written.currents = currents;
                }
                std::fs::create_dir_all(intent_path.parent().unwrap()).unwrap();
                std::fs::write(
                    &intent_path,
                    serde_json::to_string_pretty(&written).unwrap(),
                )
                .unwrap();
            }
            let inputs = crate::place_route::RouteInputs {
                options: crate::place_route::export_options(&board),
                board: board.clone(),
                rules: rules.clone(),
                effort: crate::route_search::Effort::M,
                fastroute: fastroute.clone(),
                kicad_cli: kicad_cli.clone(),
                kicad_config_home: config_home.clone(),
            };
            let candidate = dir.join("candidate");
            let started = std::time::Instant::now();
            let dsn = match crate::place_route::export_candidate(&inputs, &candidate, &zoned) {
                Ok(dsn) => dsn,
                Err(error) => {
                    println!("TW {name} {variant}: export refused: {error:#}");
                    board_record.insert(
                        variant.to_string(),
                        json!({ "export_refused": format!("{error:#}") }),
                    );
                    continue;
                }
            };
            let route = dir.join("route");
            std::fs::create_dir_all(&route).unwrap();
            let result = runtime.block_on(crate::place_route::route_candidate(
                Arc::clone(&ctx),
                &inputs,
                &dsn,
                &route,
            ));
            let route_s = started.elapsed().as_secs_f64();
            let mut row = json!({ "route_s": route_s.round() });
            match &result {
                RouteResult::Routed {
                    gates,
                    measures,
                    status,
                    ..
                } => {
                    row["status"] = json!(status);
                    row["gates_pass"] = json!(gates.all_pass());
                    row["unconnected"] = json!(measures.unconnected);
                    row["signal_mm"] = json!(measures.signal_mm.round());
                    row["power_mm"] = json!(measures.power_mm.round());
                    row["below_floor"] = json!(measures.below_floor);
                    row["copper_quality"] = json!(measures.copper_quality);
                }
                other => {
                    row["result"] = json!(format!("{other:?}"));
                }
            }
            // The session written into the candidate's copy, then the audit on it.
            let ses = std::fs::read_to_string(route.join("route.ses"));
            let manifest_path = PathBuf::from(format!("{}.konnect.json", dsn.display()));
            if let (Ok(ses), Ok(manifest)) = (ses, std::fs::read_to_string(&manifest_path)) {
                let copy = candidate.join(board.file_name().unwrap());
                let copy = std::fs::canonicalize(copy).unwrap();
                let import = crate::specctra_ses::parse_import_plan_with(
                    &copy,
                    &zoned,
                    &manifest,
                    &ses,
                    crate::specctra_ses::SourceCheck::BytesOrTree,
                )
                .unwrap();
                let routed = crate::scratch_board::write_routing(&zoned, &import).unwrap();
                let routed_board = candidate.join("routed.kicad_pcb");
                std::fs::write(&routed_board, &routed).unwrap();
                // The audit reads the record the variant routed with.
                if with {
                    let to = crate::tools::design_intent::design_intent_path(&candidate);
                    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
                    std::fs::copy(&intent_path, &to).unwrap();
                }
                let audited = trace_current::audit(&routed_board, None).unwrap();
                let body: serde_json::Value = serde_json::from_str(
                    &audited
                        .content
                        .iter()
                        .find_map(|c| match c {
                            crate::mcp::protocol::ToolContent::Text { text } => Some(text.clone()),
                            _ => None,
                        })
                        .unwrap(),
                )
                .unwrap();
                let findings = body["findings"].as_array().cloned().unwrap_or_default();
                let of = |s: &str| findings.iter().filter(|f| f["severity"] == s).count();
                row["audit"] = json!({
                    "status": body["status"],
                    "nets": body["nets"].as_array().map_or(0, Vec::len),
                    "errors": of("error"),
                    "warnings": of("warning"),
                });
                // Track widths and via sizes per sized net, from the routed text.
                let geometry = konnect_sexp::board_geometry::BoardGeometry::from_tree(
                    &konnect_sexp::parse_sexp(&routed).unwrap(),
                );
                let mut per_net = serde_json::Map::new();
                for net in width_plan.sized.iter().map(|n| n.net.as_str()) {
                    let on =
                        |e: &&konnect_sexp::board_geometry::Entry| e.net.as_deref() == Some(net);
                    let prop = |e: &konnect_sexp::board_geometry::Entry, k: &str| {
                        e.props.iter().find(|(key, _)| *key == k).map(|(_, v)| *v)
                    };
                    let widths: Vec<f64> = geometry
                        .others
                        .iter()
                        .filter(|e| e.kind == konnect_sexp::board_geometry::Kind::Track)
                        .filter(on)
                        .filter_map(|e| prop(e, "width"))
                        .collect();
                    let drills: BTreeSet<String> = geometry
                        .others
                        .iter()
                        .filter(|e| e.kind == konnect_sexp::board_geometry::Kind::Via)
                        .filter(on)
                        .filter_map(|e| prop(e, "drill").map(|d| format!("{d:.2}")))
                        .collect();
                    let vias = geometry
                        .others
                        .iter()
                        .filter(|e| e.kind == konnect_sexp::board_geometry::Kind::Via)
                        .filter(on)
                        .count();
                    per_net.insert(
                        net.to_string(),
                        json!({
                            "segments": widths.len(),
                            "min_mm": widths.iter().copied().reduce(f64::min),
                            "max_mm": widths.iter().copied().reduce(f64::max),
                            "vias": vias,
                            "via_drills": drills,
                        }),
                    );
                }
                row["sized_nets"] = json!(per_net);
            }
            if with {
                row["plan"] = json!({
                    "sized": width_plan.sized.len(),
                    "to_record": width_plan.to_record.len(),
                    "not_sized": width_plan.not_sized.len(),
                    "classes": width_plan.classes.iter().map(|c| json!({
                        "name": c.name, "width_mm": c.track_width_mm,
                        "via": [c.via.diameter_mm, c.via.drill_mm], "nets": c.nets.len()
                    })).collect::<Vec<_>>(),
                    "narrow_pads": width_plan.sized.iter().map(|n| n.narrow_pads_total).sum::<usize>(),
                    "nets": width_plan.sized.iter().map(|n| json!({
                        "net": n.net, "amps": n.current_a, "width_mm": n.width_mm,
                        "via": [n.via.via.diameter_mm, n.via.via.drill_mm],
                        "per_layer_change": n.via.per_layer_change, "poured": n.poured,
                    })).collect::<Vec<_>>(),
                });
            }
            println!("TW {name} {variant}: {row}");
            board_record.insert(variant.to_string(), row);
        }
        record.insert(name, serde_json::Value::Object(board_record));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("tw.json"),
            serde_json::to_string_pretty(&serde_json::Value::Object(record.clone())).unwrap(),
        )
        .unwrap();
    }
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("tw.json"),
        serde_json::to_string_pretty(&serde_json::Value::Object(record)).unwrap(),
    )
    .unwrap();
    println!("TW record {}", root.join("tw.json").display());
}
