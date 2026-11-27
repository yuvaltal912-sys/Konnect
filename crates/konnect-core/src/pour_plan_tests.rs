//! Tests of `pour_plan`: one test and one negative control for each guard in
//! docs/POUR_PLAN_DESIGN.md, "Tests, each guard with a negative control".

use super::*;

/// The public `pours` fixture: 2 layers, a 50 x 40 mm outline from (100, 100), twelve
/// resistors whose pad 2 is GND, and a GND pour on each side.
pub(crate) const POURS: &str = include_str!("../tests/fixtures/specctra_pours_routing.kicad_pcb");

/// [`POURS`] with its two pours removed.
pub(crate) fn pours_without_pours() -> String {
    let start = POURS.find("\t(zone\n").expect("the fixture has pours");
    let end = POURS
        .find("\t(embedded_fonts no)\n)")
        .expect("the board's last node");
    format!("{}{}", &POURS[..start], &POURS[end..])
}

/// `text` with `node` added before the closing parenthesis of its root.
pub(crate) fn with_node(text: &str, node: &str) -> String {
    let close = text.rfind(')').unwrap();
    format!("{}{node}\n{}", &text[..close], &text[close..])
}

fn stack(count: usize) -> Vec<CopperLayer> {
    let mut names = vec!["F.Cu".to_string()];
    names.extend((1..count.saturating_sub(1)).map(|i| format!("In{i}.Cu")));
    names.push("B.Cu".to_string());
    names
        .into_iter()
        .map(|name| CopperLayer {
            name,
            kind: "signal".to_string(),
        })
        .collect()
}

const OUTLINE: [(f64, f64); 4] = [(0.0, 0.0), (50.0, 0.0), (50.0, 40.0), (0.0, 40.0)];

fn facts(count: usize, pads: &[(&str, usize)]) -> BoardFacts {
    BoardFacts {
        stack: stack(count),
        outline_mm: OUTLINE.to_vec(),
        pads: pads.iter().map(|(n, c)| (n.to_string(), *c)).collect(),
        existing: Vec::new(),
        open_rule_areas: Vec::new(),
    }
}

/// Roles that come from the nets' names, as the audit gives them.
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

fn intent(value: Value) -> DesignIntent {
    let mut record = json!({ "schema_version": 1 });
    for (key, item) in value.as_object().unwrap() {
        record[key] = item.clone();
    }
    serde_json::from_value(record).expect("a valid record")
}

fn asks(pours: Value) -> DesignIntent {
    intent(json!({ "pours": pours }))
}

fn kicad_rules() -> ProjectRules {
    project_rules(None).unwrap()
}

fn laid(plan: &PourPlan) -> Vec<(&str, &str)> {
    plan.proposals
        .iter()
        .map(|p| (p.layer.as_str(), p.net.as_str()))
        .collect()
}

fn plan_of(facts: &BoardFacts, roles: &SupplyRoles, intent: Option<&DesignIntent>) -> PourPlan {
    plan(facts, roles, &kicad_rules(), intent)
}

// ─── 2 layers: ground on both ───────────────────────────────────────────────────

#[test]
fn two_layers_get_the_ground_net_on_both() {
    let plan = plan_of(
        &facts(2, &[("GND", 12), ("+5V", 4)]),
        &roles(&["GND"], &["+5V"]),
        None,
    );
    assert_eq!(laid(&plan), [("F.Cu", "GND"), ("B.Cu", "GND")]);
    assert!(plan.proposals.iter().all(|p| !p.asked));
    assert!(plan.proposals.iter().all(|p| p.points_mm == OUTLINE));
    // The power net has no layer on 2 layers, and says so.
    let row = plan
        .not_proposed
        .iter()
        .find(|r| r.net.as_deref() == Some("+5V"))
        .unwrap();
    assert!(!row.asked);
    assert!(row.reason.contains("2-layer"), "{}", row.reason);
}

// ─── 4 layers: ground on three, power on In2.Cu ────────────────────────────────

#[test]
fn four_layers_get_ground_on_three_and_the_largest_power_net_on_in2() {
    let plan = plan_of(
        &facts(4, &[("GND", 30), ("+5V", 10), ("+3V3", 20)]),
        &roles(&["GND"], &["+5V", "+3V3"]),
        None,
    );
    assert_eq!(
        laid(&plan),
        [
            ("F.Cu", "GND"),
            ("In1.Cu", "GND"),
            ("In2.Cu", "+3V3"),
            ("B.Cu", "GND")
        ]
    );
    let row = plan
        .not_proposed
        .iter()
        .find(|r| r.net.as_deref() == Some("+5V"))
        .unwrap();
    assert!(row.reason.contains("+3V3"), "{}", row.reason);

    let plan = plan_of(&facts(4, &[("GND", 30)]), &roles(&["GND"], &[]), None);
    assert_eq!(
        laid(&plan),
        [
            ("F.Cu", "GND"),
            ("In1.Cu", "GND"),
            ("In2.Cu", "GND"),
            ("B.Cu", "GND")
        ]
    );
}

// ─── No rule for another stack ──────────────────────────────────────────────────

#[test]
fn another_stack_has_no_rule_but_a_request_with_a_layer_is_met() {
    let board = facts(6, &[("GND", 30), ("+5V", 10)]);
    let roles = roles(&["GND"], &["+5V"]);
    let plan = plan_of(&board, &roles, None);
    assert!(plan.proposals.is_empty());
    let board_rows: Vec<_> = plan
        .not_proposed
        .iter()
        .filter(|r| r.net.is_none() && r.layer.is_none())
        .collect();
    assert_eq!(board_rows.len(), 1, "{:?}", plan.not_proposed);
    assert!(board_rows[0].reason.contains("6-layer"));

    let record = asks(json!([{ "net": "GND", "layers": ["In3.Cu"] }]));
    let plan = plan_of(&board, &roles, Some(&record));
    assert_eq!(laid(&plan), [("In3.Cu", "GND")]);
    assert!(plan.proposals[0].asked);
}

// ─── Most pads first, then the name ─────────────────────────────────────────────

#[test]
fn the_net_with_the_most_pads_comes_first_then_the_name() {
    let r = roles(&["GND", "AGND"], &[]);
    let pads: BTreeMap<String, usize> = [("GND".to_string(), 40), ("AGND".to_string(), 6)].into();
    let nets = supply_nets(&r, &pads, &[]);
    assert_eq!(nets[0].net, "GND");
    let plan = plan_of(&facts(2, &[("GND", 40), ("AGND", 6)]), &r, None);
    assert_eq!(laid(&plan), [("F.Cu", "GND"), ("B.Cu", "GND")]);

    let pads: BTreeMap<String, usize> = [("GND".to_string(), 6), ("AGND".to_string(), 6)].into();
    let nets = supply_nets(&r, &pads, &[]);
    assert_eq!(nets[0].net, "AGND", "equal pads: by name");
    assert_eq!(nets[0].pads, 6);
    assert_eq!(nets[0].source, "name");
}

// ─── A request with layers is exact ────────────────────────────────────────────

#[test]
fn a_request_with_layers_is_exact() {
    let record = asks(json!([{ "net": "GND", "layers": ["B.Cu"] }]));
    let plan = plan_of(
        &facts(2, &[("GND", 40)]),
        &roles(&["GND"], &[]),
        Some(&record),
    );
    assert_eq!(laid(&plan), [("B.Cu", "GND")]);
    assert!(plan.proposals[0].asked);
    let free: Vec<_> = plan
        .not_proposed
        .iter()
        .filter(|r| r.layer.as_deref() == Some("F.Cu"))
        .collect();
    assert_eq!(free.len(), 1);
    assert_eq!(free[0].net, None);
    assert!(!free[0].asked);
    assert!(free[0].reason.contains("GND"), "{}", free[0].reason);
}

// ─── The rule never fills a placed net's layers with another net ──────────────

#[test]
fn the_rule_never_fills_a_placed_nets_layers_with_another_net() {
    let record = asks(json!([{ "net": "GND", "layers": ["B.Cu"] }]));
    let plan = plan_of(
        &facts(2, &[("GND", 40), ("AGND", 6)]),
        &roles(&["GND", "AGND"], &[]),
        Some(&record),
    );
    assert_eq!(laid(&plan), [("B.Cu", "GND")]);
    assert!(plan
        .not_proposed
        .iter()
        .any(|r| r.layer.as_deref() == Some("F.Cu") && r.net.is_none()));
    let agnd = plan
        .not_proposed
        .iter()
        .find(|r| r.net.as_deref() == Some("AGND"))
        .expect("AGND is named");
    assert!(agnd.reason.contains("place GND"), "{}", agnd.reason);

    let record = asks(json!([{ "net": "+5V", "layers": ["In1.Cu"] }]));
    let plan = plan_of(
        &facts(4, &[("GND", 40), ("+5V", 20), ("+3V3", 8)]),
        &roles(&["GND"], &["+5V", "+3V3"]),
        Some(&record),
    );
    assert_eq!(
        laid(&plan),
        [("F.Cu", "GND"), ("In1.Cu", "+5V"), ("B.Cu", "GND")]
    );
    assert!(plan
        .not_proposed
        .iter()
        .any(|r| r.layer.as_deref() == Some("In2.Cu") && r.net.is_none()));
    assert!(plan
        .not_proposed
        .iter()
        .any(|r| r.net.as_deref() == Some("+3V3")));
}

// ─── A request without layers heads its kind ───────────────────────────────────

#[test]
fn a_request_without_layers_heads_its_kind() {
    let record = asks(json!([{ "net": "+5V" }]));
    let plan = plan_of(
        &facts(4, &[("GND", 40), ("+5V", 8), ("+3V3", 20)]),
        &roles(&["GND"], &["+5V", "+3V3"]),
        Some(&record),
    );
    let in2 = plan.proposals.iter().find(|p| p.layer == "In2.Cu").unwrap();
    assert_eq!(in2.net, "+5V");
    assert!(in2.asked);
    assert!(plan
        .proposals
        .iter()
        .filter(|p| p.net == "GND")
        .all(|p| !p.asked));
}

// ─── A request that cannot be met is reported ──────────────────────────────────

#[test]
fn a_request_that_cannot_be_met_is_reported() {
    let record = asks(json!([
        { "net": "VBAT", "layers": ["F.Cu"] },
        { "net": "GND", "layers": ["In1.Cu"] },
        { "net": "+5V" }
    ]));
    let plan = plan_of(
        &facts(2, &[("GND", 40), ("+5V", 8)]),
        &roles(&["GND"], &["+5V"]),
        Some(&record),
    );
    let unmet: Vec<(Option<&str>, Option<&str>)> = plan
        .not_proposed
        .iter()
        .filter(|r| r.asked)
        .map(|r| (r.net.as_deref(), r.layer.as_deref()))
        .collect();
    assert!(unmet.contains(&(Some("VBAT"), None)), "{unmet:?}");
    assert!(unmet.contains(&(Some("GND"), Some("In1.Cu"))), "{unmet:?}");
    assert!(unmet.contains(&(Some("+5V"), None)), "{unmet:?}");
    // GND was placed on a layer the stack lacks: the rule does not pour it either.
    assert!(plan.proposals.is_empty(), "{:?}", laid(&plan));
}

// ─── A net the board does not have takes no layer ──────────────────────────────

#[test]
fn a_net_the_board_does_not_have_takes_no_layer() {
    let mut r = roles(&["GND", "GND_RAIL"], &["+5V", "+3V3", "+12V"]);
    r.ground.insert(
        "GND_RAIL".to_string(),
        "design_intent rail 'GND_RAIL' at 0 V".to_string(),
    );
    let record = asks(json!([{ "net": "+12V" }]));
    let plan = plan_of(
        &facts(4, &[("GND", 40), ("+5V", 8), ("+3V3", 20)]),
        &r,
        Some(&record),
    );
    let in2 = plan.proposals.iter().find(|p| p.layer == "In2.Cu").unwrap();
    assert_eq!(in2.net, "+3V3");
    let named: Vec<_> = plan
        .not_proposed
        .iter()
        .filter(|row| row.net.as_deref() == Some("+12V"))
        .collect();
    assert_eq!(named.len(), 1);
    assert!(named[0].asked);
    assert!(plan.nets.iter().all(|n| n.net != "GND_RAIL"));
    assert!(plan.nets.iter().all(|n| n.net != "+12V"));
    // A padless rail never heads the ground nets either.
    assert_eq!(plan.proposals[0].net, "GND");
}

#[test]
fn a_net_with_no_role_that_the_record_asks_for_is_power() {
    let record = asks(json!([{ "net": "MOTOR", "layers": ["F.Cu"] }]));
    let r = roles(&["GND"], &[]);
    let nets = supply_nets(
        &r,
        &facts(2, &[("GND", 4), ("MOTOR", 2)]).pads,
        &record.pours,
    );
    let motor = nets.iter().find(|n| n.net == "MOTOR").unwrap();
    assert_eq!(motor.role, SupplyRole::Power);
    assert_eq!(motor.source, ASKED_SOURCE);
    let plan = plan_of(&facts(2, &[("GND", 4), ("MOTOR", 2)]), &r, Some(&record));
    assert_eq!(laid(&plan), [("F.Cu", "MOTOR"), ("B.Cu", "GND")]);
}

// ─── Existing zones are never changed ──────────────────────────────────────────

#[test]
fn existing_zones_are_never_changed() {
    let mut board = facts(2, &[("GND", 40)]);
    board.existing.push(ExistingPour {
        net: "GND".into(),
        layer: "B.Cu".into(),
        priority: 0,
    });
    let plan = plan_of(&board, &roles(&["GND"], &[]), None);
    assert_eq!(laid(&plan), [("F.Cu", "GND")]);
    let row = plan
        .not_proposed
        .iter()
        .find(|r| r.layer.as_deref() == Some("B.Cu"))
        .unwrap();
    assert_eq!(row.net.as_deref(), Some("GND"));
    assert!(row.reason.contains("already poured"), "{}", row.reason);
}

// ─── Only a pour is an existing pour ───────────────────────────────────────────

#[test]
fn only_a_pour_is_an_existing_pour() {
    let rule_area = "\t(zone (net \"GND\") (layer \"B.Cu\") (uuid \"aaaaaaaa-0000-4000-8000-000000000001\") (name \"strip\") (hatch edge 0.5) (keepout (tracks not_allowed) (vias not_allowed) (pads allowed) (copperpour allowed) (footprints allowed)) (polygon (pts (xy 110 110) (xy 120 110) (xy 120 120) (xy 110 120))))";
    let teardrop = "\t(zone (net \"GND\") (layer \"B.Cu\") (uuid \"aaaaaaaa-0000-4000-8000-000000000002\") (hatch edge 0.5) (attr (teardrop (type padvia))) (fill yes) (polygon (pts (xy 130 130) (xy 131 130) (xy 131 131))))";
    let text = with_node(&with_node(&pours_without_pours(), rule_area), teardrop);
    let board = board_facts(&text).unwrap();
    assert!(board.existing.is_empty(), "{:?}", board.existing);
    let plan = plan_of(&board, &roles(&["GND"], &[]), None);
    assert!(laid(&plan).contains(&("B.Cu", "GND")));
}

// ─── A rule area that lets pours in is a finding ───────────────────────────────

fn rule_area(layer: &str, keepout: &str, name: &str) -> String {
    format!(
        "\t(zone (layer \"{layer}\") (uuid \"bbbbbbbb-0000-4000-8000-00000000000{}\") (name \"{name}\") (hatch edge 0.5) (keepout {keepout}) (polygon (pts (xy 110 110) (xy 120 110) (xy 120 120) (xy 110 120))))",
        name.len() % 10
    )
}

#[test]
fn a_rule_area_that_lets_pours_in_is_a_finding() {
    let open = rule_area(
        "F.Cu",
        "(tracks not_allowed) (vias not_allowed) (pads allowed) (copperpour allowed) (footprints allowed)",
        "antenna",
    );
    let text = with_node(&pours_without_pours(), &open);
    let board = board_facts(&text).unwrap();
    assert_eq!(
        board.open_rule_areas,
        [OpenRuleArea {
            layer: "F.Cu".into(),
            name: Some("antenna".into()),
            uuid: Some("bbbbbbbb-0000-4000-8000-000000000007".into()),
            forbids: vec!["tracks".into(), "vias".into()],
        }]
    );
    let plan = plan_of(&board, &roles(&["GND"], &[]), None);
    assert_eq!(plan.filled_rule_areas, board.open_rule_areas);

    // Pours forbidden too, or only footprints forbidden: no row.
    for keepout in [
        "(tracks not_allowed) (vias not_allowed) (pads allowed) (copperpour not_allowed) (footprints allowed)",
        "(tracks allowed) (vias allowed) (pads allowed) (copperpour allowed) (footprints not_allowed)",
    ] {
        let text = with_node(&pours_without_pours(), &rule_area("F.Cu", keepout, "a"));
        assert!(board_facts(&text).unwrap().open_rule_areas.is_empty(), "{keepout}");
    }
    // Every copper layer: one row each; pads alone count too.
    let text = with_node(
        &pours_without_pours(),
        &rule_area("*.Cu", "(pads not_allowed) (copperpour allowed)", "pads"),
    );
    let rows = board_facts(&text).unwrap().open_rule_areas;
    assert_eq!(
        rows.iter().map(|r| r.layer.as_str()).collect::<Vec<_>>(),
        ["F.Cu", "B.Cu"]
    );
    assert_eq!(rows[0].forbids, ["pads"]);

    // On a layer with no proposal: not in filled_rule_areas.
    let mut board = board_facts(&with_node(&pours_without_pours(), &open)).unwrap();
    board.existing.push(ExistingPour {
        net: "GND".into(),
        layer: "F.Cu".into(),
        priority: 0,
    });
    let plan = plan_of(&board, &roles(&["GND"], &[]), None);
    assert_eq!(laid(&plan), [("B.Cu", "GND")]);
    assert!(plan.filled_rule_areas.is_empty());
}

// ─── A whole-layer pour sits below every other ─────────────────────────────────

#[test]
fn a_whole_layer_pour_sits_below_every_other() {
    let slot = Slot {
        layer: "F.Cu".into(),
        net: "GND".into(),
        reason: String::new(),
        asked: false,
    };
    let other = |priority| ExistingPour {
        net: "+5V".into(),
        layer: "F.Cu".into(),
        priority,
    };
    let reason = against_existing(&slot, &[other(0)]).unwrap_err();
    assert!(
        reason.contains("+5V") && reason.contains("priority"),
        "{reason}"
    );
    assert_eq!(against_existing(&slot, &[other(1)]), Ok(()));
    // Another layer's pour at priority 0 is no reason.
    let elsewhere = ExistingPour {
        net: "+5V".into(),
        layer: "B.Cu".into(),
        priority: 0,
    };
    assert_eq!(against_existing(&slot, &[elsewhere]), Ok(()));

    let mut board = facts(2, &[("GND", 40), ("+5V", 3)]);
    board.existing.push(other(0));
    let plan = plan_of(&board, &roles(&["GND"], &["+5V"]), None);
    assert_eq!(laid(&plan), [("B.Cu", "GND")]);
    board.existing[0].priority = 1;
    let plan = plan_of(&board, &roles(&["GND"], &["+5V"]), None);
    assert_eq!(laid(&plan), [("F.Cu", "GND"), ("B.Cu", "GND")]);
}

// ─── The clearance is the largest floor ────────────────────────────────────────

#[test]
fn the_clearance_is_the_largest_floor() {
    let classes = vec!["Power".to_string(), "HV".to_string()];
    let mut rules = kicad_rules();
    rules.class_clearance_mm.insert("Power".into(), 0.3);
    rules.class_clearance_mm.insert("HV".into(), 0.4);
    assert_eq!(settings_for(&classes, &rules, None).clearance_mm, 0.4);
    rules.class_clearance_mm.insert("Default".into(), 0.5);
    assert_eq!(settings_for(&classes, &rules, None).clearance_mm, 0.5);
    rules.min_clearance_mm = 0.6;
    assert_eq!(settings_for(&classes, &rules, None).clearance_mm, 0.6);
    let fab = FabTarget {
        min_clearance_mm: Some(0.7),
        ..FabTarget::default()
    };
    assert_eq!(settings_for(&classes, &rules, Some(&fab)).clearance_mm, 0.7);
    // No class at all: KiCad's own Default class.
    assert_eq!(
        settings_for(&[], &kicad_rules(), None).clearance_mm,
        SEEDED_CLASS_CLEARANCE_MM
    );
    // A Default below the seeded value is the file's, not the seed.
    let mut low = kicad_rules();
    low.class_clearance_mm.insert("Default".into(), 0.15);
    assert_eq!(settings_for(&[], &low, None).clearance_mm, 0.15);
}

// ─── The minimum width and the bridge follow the fab ───────────────────────────

#[test]
fn the_minimum_width_and_the_bridge_follow_the_fab() {
    let fab = |track| FabTarget {
        min_track_mm: Some(track),
        ..FabTarget::default()
    };
    let s = settings_for(&[], &kicad_rules(), Some(&fab(0.4)));
    assert_eq!((s.min_width_mm, s.thermal_bridge_mm), (0.4, 0.5));
    assert_eq!(s.check(), Ok(()));
    let s = settings_for(&[], &kicad_rules(), Some(&fab(0.8)));
    assert_eq!((s.min_width_mm, s.thermal_bridge_mm), (0.8, 0.8));
    assert_eq!(s.check(), Ok(()));
    let s = settings_for(&[], &kicad_rules(), None);
    assert_eq!((s.min_width_mm, s.thermal_bridge_mm), (MIN_WIDTH_MM, 0.5));
    let mut rules = kicad_rules();
    rules.min_track_width_mm = 0.3;
    assert_eq!(settings_for(&[], &rules, None).min_width_mm, 0.3);
    assert_eq!(s.thermal_gap_mm, 0.5);
    assert_eq!(s.priority, 0);
    assert_eq!(s.islands, ZoneIslands::Always);
    assert_eq!(s.connection, PadConnection::Thermal);
}

// ─── A project file that does not read is refused ──────────────────────────────

#[test]
fn a_project_file_that_does_not_read_is_refused() {
    assert!(project_rules(Some("{")).is_err());
    let bad_rule = |value: Value| {
        json!({ "board": { "design_settings": { "rules": { "min_clearance": value } } } })
            .to_string()
    };
    assert!(project_rules(Some(&bad_rule(json!(-1)))).is_err());
    assert!(project_rules(Some(&bad_rule(json!("0.2")))).is_err());
    let bad_class =
        json!({ "net_settings": { "classes": [{ "name": "Default", "clearance": "x" }] } });
    assert!(project_rules(Some(&bad_class.to_string())).is_err());
    let none = project_rules(None).unwrap();
    assert_eq!(
        none,
        ProjectRules {
            min_clearance_mm: 0.0,
            min_track_width_mm: 0.0,
            class_clearance_mm: BTreeMap::new(),
        }
    );
    let fixture = include_str!("../tests/fixtures/specctra_pours_routing.kicad_pro");
    let read = project_rules(Some(fixture)).unwrap();
    assert_eq!(read.min_clearance_mm, 0.2);
    assert_eq!(read.min_track_width_mm, 0.2);
    assert_eq!(read.class_clearance_mm.get("Default"), Some(&0.2));
}

// ─── The outline is the board's, in KiCad's axes ───────────────────────────────

#[test]
fn the_outline_is_the_boards_in_kicads_axes() {
    let board = board_facts(&pours_without_pours()).unwrap();
    assert_eq!(board.outline_mm.len(), 4, "{:?}", board.outline_mm);
    let mut corners = board.outline_mm.clone();
    corners.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(
        corners,
        [
            (100.0, 100.0),
            (100.0, 140.0),
            (150.0, 100.0),
            (150.0, 140.0)
        ]
    );
    assert_ne!(board.outline_mm.first(), board.outline_mm.last());
    assert!(board.outline_mm.iter().all(|(_, y)| *y > 0.0));

    let two_loops = with_node(
        &pours_without_pours(),
        "\t(gr_rect (start 0 0) (end 5 5) (stroke (width 0.05) (type default)) (fill no) (layer \"Edge.Cuts\") (uuid \"cccccccc-0000-4000-8000-000000000001\"))",
    );
    let error = format!("{:#}", board_facts(&two_loops).unwrap_err());
    assert!(error.contains("more than one loop"), "{error}");
}

#[test]
fn board_facts_reads_the_fixture() {
    let board = board_facts(POURS).unwrap();
    assert_eq!(
        board
            .stack
            .iter()
            .map(|l| (l.name.as_str(), l.kind.as_str()))
            .collect::<Vec<_>>(),
        [("F.Cu", "signal"), ("B.Cu", "signal")]
    );
    assert_eq!(board.pads.get("GND"), Some(&12));
    assert_eq!(board.pads.get("S1"), Some(&2));
    assert_eq!(
        board.existing,
        [
            ExistingPour {
                net: "GND".into(),
                layer: "F.Cu".into(),
                priority: 0
            },
            ExistingPour {
                net: "GND".into(),
                layer: "B.Cu".into(),
                priority: 0
            }
        ]
    );
    let raised = POURS.replacen("(name \"GND_TOP\")", "(name \"GND_TOP\") (priority 2)", 1);
    assert_eq!(board_facts(&raised).unwrap().existing[0].priority, 2);
    // A copper stack the export refuses is no plan.
    let odd = POURS.replace("(2 \"B.Cu\" signal)", "(2 \"B.Cu\" jumper)");
    assert!(board_facts(&odd).is_err());
}

// ─── The roles are the audit's ─────────────────────────────────────────────────

#[test]
fn the_roles_are_the_audits() {
    let nets: BTreeSet<String> = ["REF", "GND", "N1"].iter().map(|n| n.to_string()).collect();
    let record = json!({
        "schema_version": 1,
        "rails": [{ "name": "REF0", "nets": ["REF"], "voltage_v": 0.0 }]
    })
    .to_string();
    let roles = crate::tools::supply_roles(&nets, Some(&record), None);
    assert!(
        roles.ground["REF"].starts_with("design_intent rail"),
        "{roles:?}"
    );
    assert_eq!(roles.ground["GND"], "name");
    assert!(!roles.ground.contains_key("N1") && !roles.power.contains_key("N1"));
    let roles = crate::tools::supply_roles(&nets, None, None);
    assert!(!roles.ground.contains_key("REF"));
}

// ─── The arguments read back ───────────────────────────────────────────────────

#[test]
fn the_arguments_read_back() {
    let mut zone = ProposedZone {
        net: "GND".into(),
        layer: "F.Cu".into(),
        points_mm: OUTLINE.to_vec(),
        settings: settings_for(&[], &kicad_rules(), None),
        reason: String::new(),
        asked: false,
    };
    for islands in [
        ZoneIslands::Always,
        ZoneIslands::Never,
        ZoneIslands::BelowArea { area_mm2: 3.25 },
    ] {
        zone.settings.islands = islands;
        zone.settings.clearance_mm = 0.35;
        let args = add_zone_arguments(Path::new("/b/board.kicad_pcb"), &zone);
        assert_eq!(ZoneSettings::from_args(&args), Ok(zone.settings), "{args}");
        assert_eq!(args["board"], "/b/board.kicad_pcb");
        assert_eq!(args["net_name"], "GND");
        assert_eq!(args["layer"], "F.Cu");
        assert_eq!(args["points"][2], json!({ "x": 50.0, "y": 40.0 }));
        for key in [
            "clearance",
            "min_width",
            "priority",
            "pad_connection",
            "thermal_gap",
            "thermal_bridge_width",
            "island_removal",
        ] {
            assert!(args.get(key).is_some(), "{key} is written: {args}");
        }
    }
}

// ─── The scratch zone is add_zone's ────────────────────────────────────────────

fn mask_uuids(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("(uuid \"") {
        let start = at + "(uuid \"".len();
        out.push_str(&rest[..start]);
        let end = rest[start..].find('"').unwrap() + start;
        out.push('U');
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

#[test]
fn the_scratch_zone_is_add_zones() {
    let text = pours_without_pours();
    let board = board_facts(&text).unwrap();
    let plan = plan_of(&board, &roles(&["GND"], &[]), None);
    assert_eq!(plan.proposals.len(), 2);
    let written = with_zones(&text, &plan.proposals).unwrap();
    let close = text.rfind(')').unwrap();
    let zones: String = plan
        .proposals
        .iter()
        .map(|zone| {
            crate::tools::pcb_board::zone_sexp(
                &konnect_sexp::net::NetRef::ByName(zone.net.clone()),
                &zone.layer,
                &zone.points_mm,
                "",
                &zone.settings,
            )
        })
        .collect();
    let expected = format!("{}{zones}{}", &text[..close], &text[close..]);
    assert_eq!(mask_uuids(&written), mask_uuids(&expected));
    // The board's own UUIDs are untouched, and the board reads back with both pours.
    assert!(written.starts_with(&text[..close]));
    let again = board_facts(&written).unwrap();
    assert_eq!(again.existing.len(), 2);
}

// ─── with_zones refuses what it cannot write ───────────────────────────────────

#[test]
fn with_zones_refuses_what_it_cannot_write() {
    let zone = ProposedZone {
        net: "PWR".into(),
        layer: "F.Cu".into(),
        points_mm: OUTLINE.to_vec(),
        settings: settings_for(&[], &kicad_rules(), None),
        reason: String::new(),
        asked: false,
    };
    let legacy = "(kicad_pcb (version 20240108) (net 0 \"\") (net 1 \"GND\"))";
    let error = format!(
        "{:#}",
        with_zones(legacy, std::slice::from_ref(&zone)).unwrap_err()
    );
    assert!(error.contains("PWR"), "{error}");
    let gnd = ProposedZone {
        net: "GND".into(),
        ..zone.clone()
    };
    let ok = with_zones(legacy, std::slice::from_ref(&gnd)).unwrap();
    assert!(ok.contains("(net 1) (net_name \"GND\")"), "{ok}");
    let error = with_zones("(kicad_sch (version 1))", &[gnd]).unwrap_err();
    assert!(format!("{error:#}").contains("kicad_pcb"));
}

// ─── The plan joins its parts ──────────────────────────────────────────────────

#[test]
fn the_plan_of_the_fixture_without_pours_is_the_rule() {
    let text = pours_without_pours();
    let board = board_facts(&text).unwrap();
    let project = include_str!("../tests/fixtures/specctra_pours_routing.kicad_pro");
    let rules = project_rules(Some(project)).unwrap();
    let nets: BTreeSet<String> = board.pads.keys().cloned().collect();
    let roles = crate::tools::supply_roles(&nets, None, Some(project));
    let plan = plan(&board, &roles, &rules, None);
    assert_eq!(laid(&plan), [("F.Cu", "GND"), ("B.Cu", "GND")]);
    for zone in &plan.proposals {
        assert_eq!(zone.settings.clearance_mm, 0.2);
        assert_eq!(zone.settings.min_width_mm, MIN_WIDTH_MM);
        assert_eq!(zone.settings.check(), Ok(()));
        assert_eq!(zone.points_mm.len(), 4);
    }
    assert_eq!(plan.nets.len(), 1);
    assert_eq!(plan.nets[0].source, "name");
}

// ─── Added for the mutants batch's misses (phase A) ────────────────────────────

/// A left-out net's reason names what the rule did instead: the net it poured, or that
/// the design intent's pours took every layer the rule has for its kind.
#[test]
fn a_left_out_nets_reason_says_what_the_rule_did_instead() {
    let plan = plan_of(
        &facts(4, &[("GND", 30), ("+5V", 10), ("+3V3", 20)]),
        &roles(&["GND"], &["+5V", "+3V3"]),
        None,
    );
    let row = plan
        .not_proposed
        .iter()
        .find(|r| r.net.as_deref() == Some("+5V"))
        .unwrap();
    assert!(
        row.reason.contains("the rule pours +3V3 in its place"),
        "{}",
        row.reason
    );

    let record = asks(json!([{ "net": "MOTOR", "layers": ["F.Cu", "B.Cu"] }]));
    let plan = plan_of(
        &facts(2, &[("GND", 40), ("MOTOR", 2)]),
        &roles(&["GND"], &[]),
        Some(&record),
    );
    assert_eq!(laid(&plan), [("F.Cu", "MOTOR"), ("B.Cu", "MOTOR")]);
    let row = plan
        .not_proposed
        .iter()
        .find(|r| r.net.as_deref() == Some("GND"))
        .unwrap();
    assert!(
        row.reason.contains("taken by the design intent's pours"),
        "{}",
        row.reason
    );
}

/// Two entries on one layer (the record's own validation refuses it): the first takes
/// the layer, the second is an unmet request that names the first.
#[test]
fn the_first_of_two_entries_on_one_layer_takes_it() {
    let record = asks(json!([{ "net": "GND", "layers": ["B.Cu"] }]));
    let mut pours = record.pours.clone();
    pours.push(PourIntent {
        net: "AGND".into(),
        layers: vec!["B.Cu".into()],
        notes: None,
    });
    let r = roles(&["GND", "AGND"], &[]);
    let nets = supply_nets(&r, &facts(2, &[("GND", 40), ("AGND", 6)]).pads, &pours);
    let division = division(&stack(2), &nets, &pours);
    let on_back: Vec<&str> = division
        .slots
        .iter()
        .filter(|s| s.layer == "B.Cu")
        .map(|s| s.net.as_str())
        .collect();
    assert_eq!(on_back, ["GND"]);
    let row = division
        .left_out
        .iter()
        .find(|row| row.net.as_deref() == Some("AGND") && row.layer.as_deref() == Some("B.Cu"))
        .expect("AGND's request is unmet");
    assert!(row.asked);
    assert!(row.reason.contains("gives to GND"), "{}", row.reason);
}

/// A net with no pad is left out even when the pad map names it with 0, and a net asked
/// for that already has a role keeps it, once.
#[test]
fn supply_nets_keep_each_net_once_with_its_own_role() {
    let r = roles(&["GND"], &["+5V"]);
    let pads: BTreeMap<String, usize> = [("GND".to_string(), 4), ("+5V".to_string(), 0)].into();
    let nets = supply_nets(&r, &pads, &[]);
    assert_eq!(
        nets.iter().map(|n| n.net.as_str()).collect::<Vec<_>>(),
        ["GND"]
    );

    let pads: BTreeMap<String, usize> = [("GND".to_string(), 4), ("+5V".to_string(), 2)].into();
    for asked in [
        asks(json!([{ "net": "GND" }])),
        asks(json!([{ "net": "+5V" }])),
    ] {
        let nets = supply_nets(&r, &pads, &asked.pours);
        let roles: Vec<(&str, SupplyRole)> =
            nets.iter().map(|n| (n.net.as_str(), n.role)).collect();
        assert_eq!(
            roles,
            [("GND", SupplyRole::Ground), ("+5V", SupplyRole::Power)],
            "{:?}",
            asked.pours
        );
        assert!(nets.iter().all(|n| n.source == "name"));
    }
    // A net with no role is added as power beside the board's own power nets.
    let pads: BTreeMap<String, usize> = [
        ("GND".to_string(), 4),
        ("+5V".to_string(), 2),
        ("MOTOR".to_string(), 3),
    ]
    .into();
    let nets = supply_nets(&r, &pads, &asks(json!([{ "net": "MOTOR" }])).pours);
    let motor: Vec<&SupplyNet> = nets.iter().filter(|n| n.net == "MOTOR").collect();
    assert_eq!(motor.len(), 1, "{nets:?}");
    assert_eq!(motor[0].role, SupplyRole::Power);
    assert_eq!(motor[0].source, ASKED_SOURCE);
    // Asked for without layers, it heads the power nets.
    assert_eq!(nets[1].net, "MOTOR");
}

// ─── Added for the review of phase A ───────────────────────────────────────────

/// A request the board already meets (its net already pours the layer) is no unmet
/// request; a request that another net's pour blocks still is.
#[test]
fn a_request_the_board_already_meets_is_not_unmet() {
    let mut board = facts(2, &[("GND", 12)]);
    board.existing.push(ExistingPour {
        net: "GND".into(),
        layer: "B.Cu".into(),
        priority: 0,
    });
    let r = roles(&["GND"], &[]);
    for record in [
        asks(json!([{ "net": "GND" }])),
        asks(json!([{ "net": "GND", "layers": ["B.Cu"] }])),
    ] {
        let plan = plan_of(&board, &r, Some(&record));
        let row = plan
            .not_proposed
            .iter()
            .find(|row| row.layer.as_deref() == Some("B.Cu"))
            .expect("B.Cu is left out");
        assert!(row.reason.contains("already poured"), "{}", row.reason);
        assert!(!row.asked, "{:?}: the board meets it", record.pours);
        assert!(plan.not_proposed.iter().all(|row| !row.asked), "{plan:?}");
    }
    // Blocked by another net's pour at priority 0: still unmet.
    let mut blocked = facts(2, &[("GND", 12), ("+5V", 3)]);
    blocked.existing.push(ExistingPour {
        net: "+5V".into(),
        layer: "B.Cu".into(),
        priority: 0,
    });
    let record = asks(json!([{ "net": "GND", "layers": ["B.Cu"] }]));
    let plan = plan_of(&blocked, &roles(&["GND"], &["+5V"]), Some(&record));
    let row = plan
        .not_proposed
        .iter()
        .find(|row| row.layer.as_deref() == Some("B.Cu") && row.net.as_deref() == Some("GND"))
        .unwrap();
    assert!(row.asked);
}

/// A net placed only on layers the stack lacks: the free layer's reason says so.
#[test]
fn a_free_layers_reason_says_the_requested_layers_are_not_on_the_board() {
    let record = asks(json!([{ "net": "GND", "layers": ["In1.Cu"] }]));
    let plan = plan_of(
        &facts(2, &[("GND", 12)]),
        &roles(&["GND"], &[]),
        Some(&record),
    );
    let row = plan
        .not_proposed
        .iter()
        .find(|row| row.layer.as_deref() == Some("F.Cu") && row.net.is_none())
        .unwrap();
    assert!(
        row.reason
            .contains("place on In1.Cu, which this board does not have"),
        "{}",
        row.reason
    );
    assert!(!row.reason.contains("other layers"), "{}", row.reason);
    // Placed on a layer it has: "other layers" is true.
    let record = asks(json!([{ "net": "GND", "layers": ["B.Cu"] }]));
    let plan = plan_of(
        &facts(2, &[("GND", 12)]),
        &roles(&["GND"], &[]),
        Some(&record),
    );
    let row = plan
        .not_proposed
        .iter()
        .find(|row| row.layer.as_deref() == Some("F.Cu"))
        .unwrap();
    assert!(
        row.reason.contains("place on other layers"),
        "{}",
        row.reason
    );
}

// ─── PP16: the thermal steps ───────────────────────────────────────────────────

#[test]
fn the_first_thermal_step_is_todays() {
    use konnect_ipc::builders::ZONE_THERMAL_RELIEF_MM;
    assert_eq!(
        THERMAL_STEPS[0],
        (ZONE_THERMAL_RELIEF_MM, ZONE_THERMAL_RELIEF_MM)
    );
    let s = settings_for(&[], &kicad_rules(), None);
    assert_eq!(
        (s.thermal_gap_mm, s.thermal_bridge_mm),
        (THERMAL_STEPS[0].0, THERMAL_STEPS[0].1)
    );
    assert_eq!(THERMAL_STEPS[1], (0.3, 0.4));
}

#[test]
fn the_bridge_never_drops_below_the_minimum_width() {
    let mut wide = settings_for(&[], &kicad_rules(), None);
    wide.min_width_mm = 0.5;
    wide.thermal_bridge_mm = 0.5;
    let zone = |settings: ZoneSettings| ProposedZone {
        net: "GND".into(),
        layer: "F.Cu".into(),
        points_mm: OUTLINE.to_vec(),
        settings,
        reason: String::new(),
        asked: false,
    };
    let zones = [zone(wide), zone(settings_for(&[], &kicad_rules(), None))];
    let stepped = with_thermal_step(&zones, 1).unwrap();
    assert_eq!(stepped[0].settings.thermal_bridge_mm, 0.5);
    assert_eq!(stepped[1].settings.thermal_bridge_mm, 0.4);
    for zone in &stepped {
        assert_eq!(zone.settings.thermal_gap_mm, 0.3);
        assert_eq!(zone.settings.check(), Ok(()));
    }
    // Nothing else changes.
    let mut back = stepped[1].clone();
    back.settings.thermal_gap_mm = zones[1].settings.thermal_gap_mm;
    back.settings.thermal_bridge_mm = zones[1].settings.thermal_bridge_mm;
    assert_eq!(back, zones[1]);
    assert_eq!(with_thermal_step(&zones, 0).unwrap(), zones.to_vec());
    assert!(with_thermal_step(&zones, 2).is_none());
}

// ─── Row FF: the roles in one argument ───────────────────────────────────────

fn row_ff_roles() -> crate::tools::SupplyRoles {
    let mut roles = crate::tools::SupplyRoles::default();
    roles
        .ground
        .insert("GND".into(), "design_intent rail 'GND' at 0 V".into());
    roles.ground.insert("AGND".into(), "name".into());
    roles.power.insert("VCC".into(), "name".into());
    roles.power.insert("3V3".into(), "netclass 'Power'".into());
    roles
}

fn row_ff_nets() -> BTreeSet<String> {
    ["GND", "AGND", "VCC", "3V3", "SIG"]
        .iter()
        .map(|n| n.to_string())
        .collect()
}

#[test]
fn rails_and_names_are_read_from_the_sources() {
    let roles = row_ff_roles();
    assert_eq!(
        rails_of(&roles),
        BTreeMap::from([("GND".to_string(), "ground")])
    );
    assert_eq!(named_only(&roles), ["AGND", "VCC"]);
}

#[test]
fn with_roles_confirms_sets_and_removes() {
    use crate::full_flow::{AsNamed, GivenRole, RolesAnswer};
    let roles = row_ff_roles();
    let rails = rails_of(&roles);
    let nets = row_ff_nets();
    assert_eq!(
        with_roles(
            &roles,
            &RolesAnswer::AsNamed(AsNamed::AsNamed),
            &rails,
            &nets
        )
        .unwrap(),
        roles
    );
    let given = RolesAnswer::Map(BTreeMap::from([
        ("AGND".to_string(), GivenRole::Power),
        ("VCC".to_string(), GivenRole::NotASupply),
        ("SIG".to_string(), GivenRole::Ground),
    ]));
    let out = with_roles(&roles, &given, &rails, &nets).unwrap();
    assert_eq!(out.power.get("AGND").map(String::as_str), Some(USER_SOURCE));
    assert!(!out.ground.contains_key("AGND"));
    assert!(!out.power.contains_key("VCC") && !out.ground.contains_key("VCC"));
    assert_eq!(out.ground.get("SIG").map(String::as_str), Some(USER_SOURCE));
    assert_eq!(named_only(&out), Vec::<String>::new());
}

#[test]
fn with_roles_refuses_a_rail_of_another_role_and_a_missing_net() {
    use crate::full_flow::{GivenRole, RolesAnswer};
    let roles = row_ff_roles();
    let rails = rails_of(&roles);
    let nets = row_ff_nets();
    let against = RolesAnswer::Map(BTreeMap::from([("GND".to_string(), GivenRole::Power)]));
    assert!(with_roles(&roles, &against, &rails, &nets)
        .unwrap_err()
        .contains("set_design_intent"));
    let same = RolesAnswer::Map(BTreeMap::from([("GND".to_string(), GivenRole::Ground)]));
    assert!(with_roles(&roles, &same, &rails, &nets).is_ok());
    let missing = RolesAnswer::Map(BTreeMap::from([("NOPE".to_string(), GivenRole::Ground)]));
    assert!(with_roles(&roles, &missing, &rails, &nets)
        .unwrap_err()
        .contains("NOPE"));
}
