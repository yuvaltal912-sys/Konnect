//! Tests of Row ZR, stage ZR6 (`pour_stitch`): the pure parts. The parts that run a judge
//! (`reference_round`, `deliver_winner`) are tested in `pour_stitch_judge_tests.rs`, with
//! a fake kicad-cli. Each guard has a negative control (C01 to C..) in the stage's records.

use super::*;
use crate::pour_prune::AuditItems;
use crate::route_judge::Gates;
use crate::tools::cli::{DrcViolation, ReportItem, ReportPos};
use crate::zone_fill::{PourMeasures, ZoneLayerFill};

// ─── S2: the targets ─────────────────────────────────────────────────────────

fn finding(subject: &str, nets: &[&str], at: Option<(f64, f64)>) -> Value {
    json!({
        "item": "3.2",
        "subject": subject,
        "measured": {
            "reference_nets": nets,
            "reference_layers": ["F.Cu", "B.Cu"],
            "nearest_stitch_mm": 5.5,
        },
        "threshold": { "stitch_via_max_mm": 3.0 },
        "at": at.map(|(x, y)| json!({ "x": x, "y": y })),
    })
}

fn audit_of(findings: Vec<Value>, count: f64) -> Value {
    json!({
        "metrics": { "3.2.unstitched_layer_changes": count },
        "findings": findings,
    })
}

fn ground() -> BTreeSet<String> {
    BTreeSet::from(["GND".to_string()])
}

#[test]
fn targets_are_the_ground_findings_and_the_rest_are_skipped_with_their_reason() {
    let audit = audit_of(
        vec![
            finding("SIG via a", &["GND"], Some((10.0, 20.0))),
            finding("SIG via b", &["GND", "VCC"], Some((1.0, 2.0))),
            finding("SIG via c", &["VCC"], Some((3.0, 4.0))),
            json!({ "item": "3.1", "subject": "not 3.2" }),
            finding("SIG via d", &[], Some((5.0, 6.0))),
            finding("SIG via e", &["GND"], None),
        ],
        5.0,
    );
    let found = targets(&audit, &ground());
    assert_eq!(
        found.targets,
        vec![Target {
            net: "GND".to_string(),
            at: (10.0, 20.0),
            reference_layers: vec!["F.Cu".to_string(), "B.Cu".to_string()],
            max_mm: 3.0,
            subject: "SIG via a".to_string(),
        }]
    );
    let reasons: Vec<(&str, &str)> = found
        .skipped
        .iter()
        .map(|s| (s.subject.as_str(), s.reason))
        .collect();
    assert_eq!(
        reasons,
        vec![
            ("SIG via b", "two_reference_nets"),
            ("SIG via c", "not_ground"),
            ("SIG via d", "not_ground"),
            ("SIG via e", "no_position"),
        ]
    );
    // Five listed of five counted: complete.
    assert!(found.complete);
}

#[test]
fn targets_are_incomplete_when_the_audit_listed_fewer_findings_than_it_counted() {
    let audit = audit_of(vec![finding("SIG via a", &["GND"], Some((1.0, 1.0)))], 2.0);
    assert!(!targets(&audit, &ground()).complete);
    let audit = audit_of(vec![finding("SIG via a", &["GND"], Some((1.0, 1.0)))], 1.0);
    assert!(targets(&audit, &ground()).complete);
    // A finding without its limit takes the audit's default.
    let mut bare = finding("SIG via a", &["GND"], Some((1.0, 1.0)));
    bare["threshold"] = json!({});
    assert_eq!(
        targets(&audit_of(vec![bare], 1.0), &ground()).targets[0].max_mm,
        3.0
    );
}

// ─── S3: the planner's vias kept ─────────────────────────────────────────────

fn via(net: &str, x: f64, y: f64) -> PlannedVia {
    PlannedVia {
        net: net.to_string(),
        x_mm: x,
        y_mm: y,
        size_mm: 0.6,
        drill_mm: 0.3,
    }
}

fn target(net: &str, at: (f64, f64)) -> Target {
    Target {
        net: net.to_string(),
        at,
        reference_layers: vec!["F.Cu".into(), "B.Cu".into()],
        max_mm: 3.0,
        subject: format!("S via at {at:?}"),
    }
}

#[test]
fn only_the_vias_of_a_target_net_within_its_limit_are_kept() {
    let items = Planned {
        tracks: vec![crate::scratch_board::PlannedTrack {
            net: "GND".into(),
            layer: "F.Cu".into(),
            width_mm: 0.2,
            x1_mm: 0.0,
            y1_mm: 0.0,
            x2_mm: 1.0,
            y2_mm: 0.0,
        }],
        vias: vec![
            via("GND", 12.0, 10.0),
            // 3 mm and 1 µm: on the limit with the slack.
            via("GND", 13.001, 10.0),
            // Beyond the slack.
            via("GND", 13.0021, 10.0),
            // Another net at the target's point.
            via("VCC", 10.0, 10.0),
            // A joiner far from every target.
            via("GND", 40.0, 40.0),
        ],
    };
    let (kept, dropped) = keep_reference_vias(&items, &[target("GND", (10.0, 10.0))]);
    assert!(kept.tracks.is_empty());
    assert_eq!(
        kept.vias,
        vec![via("GND", 12.0, 10.0), via("GND", 13.001, 10.0)]
    );
    assert_eq!(dropped, 4, "the track and three vias");
}

// ─── S5: the guard ───────────────────────────────────────────────────────────

/// The two clauses a lower copper quality gives: today's entry 11 and where the pour key
/// holds it (`pour_rank::TODAY_AT`, the layout of main b64e2f3c).
fn quality_keys() -> Vec<String> {
    vec![
        "key:routing:11".to_string(),
        format!("key:pours:{}", crate::pour_rank::TODAY_AT[11]),
    ]
}

fn pours(starved: usize) -> PourMeasures {
    PourMeasures {
        open_before_stitching: 0,
        open_after_stitching: 0,
        pieces: 2,
        largest_piece_share: Some(1.0),
        unfilled: 0,
        starved_thermals: starved,
        false_connected: None,
        false_open: None,
        damage: Some(crate::pour_damage::PourDamage {
            lost_fills: 0,
            plane_loss: 0.0,
            area_loss: 0.0,
            nets: vec![],
            layers: vec![],
        }),
    }
}

fn fill(net: &str, layer: &str, area: f64) -> ZoneLayerFill {
    ZoneLayerFill {
        net: net.to_string(),
        layer: layer.to_string(),
        zone_uuid: Some(format!("z-{net}-{layer}")),
        pieces_mm2: vec![area],
        islands: vec![false],
    }
}

fn gates() -> Gates {
    Gates {
        g1: vec![],
        g2: vec![],
        g3: vec![],
        g4: vec![],
    }
}

/// A board's check: no violation, GND and VCC fills, 3.2 at `unstitched`, every other
/// item at the same value.
fn checked(unstitched: f64) -> Checked {
    let mut measures = crate::route_judge::tests::measures();
    measures.pours = Some(pours(0));
    measures.checklist = ChecklistItems {
        length_off_reference_mm: Some(10.0),
        unstitched_layer_changes: Some(unstitched),
        floating_ground_mm2: Some(0.0),
        floating_ground_islands: Some(0.0),
        worst_loop_mm: Some(5.0),
    };
    Checked {
        unconnected: 0,
        violations: BTreeMap::from([("silk_overlap".to_string(), 2)]),
        rest: Some(CheckedRest {
            fills: vec![
                fill("GND", "B.Cu", 100.0),
                fill("GND", "F.Cu", 100.0),
                fill("VCC", "In1.Cu", 50.0),
            ],
            measures,
            gates: gates(),
            audit: AuditItems {
                ground: ground(),
                unstitched_layer_changes: Some(unstitched),
                loops_measured: Some(2),
                worst_loop_mm: Some(5.0),
                median_loop_mm: Some(4.0),
                mean_loop_area_mm2: Some(2.0),
                pin_loops: BTreeMap::new(),
                scored: BTreeSet::from(["3.2".to_string(), "4.1".to_string()]),
            },
        }),
    }
}

fn rest(checked: &mut Checked) -> &mut CheckedRest {
    checked.rest.as_mut().unwrap()
}

fn gnd() -> BTreeSet<String> {
    ground()
}

#[test]
fn a_stitched_board_that_only_serves_reference_changes_is_accepted() {
    assert_eq!(
        stitch_refusals(&checked(3.0), &checked(0.0), &gnd()),
        Vec::<String>::new()
    );
}

#[test]
fn the_unconnected_count_must_stay_equal_and_no_rule_may_rise() {
    for count in [1, 0] {
        let (mut before, mut after) = (checked(3.0), checked(0.0));
        before.unconnected = 1 - count;
        after.unconnected = count;
        // A lower count is refused too: a reference via may not close an open.
        assert_eq!(
            stitch_refusals(&before, &after, &gnd()),
            vec!["unconnected"]
        );
    }
    let mut after = checked(0.0);
    after.violations.insert("clearance".into(), 1);
    after.violations.insert("silk_overlap".into(), 1);
    // A DRC refusal comes alone, even with other clauses failing.
    rest(&mut after).measures.copper_quality = Some(10.0);
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        vec!["rule:clearance"]
    );
    let mut both = after.clone();
    both.unconnected = 2;
    assert_eq!(
        stitch_refusals(&checked(3.0), &both, &gnd()),
        vec!["unconnected", "rule:clearance"]
    );
}

#[test]
fn a_board_with_no_rest_or_no_pour_record_where_one_is_due_is_unchecked() {
    let mut after = checked(0.0);
    after.rest = None;
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        vec!["unchecked"]
    );
    let mut after = checked(0.0);
    rest(&mut after).measures.pours = None;
    rest(&mut after).measures.pours_error = Some("unreadable".into());
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        vec!["unchecked"]
    );
    // A call with no pour net takes no record on either side: nothing is due.
    let (mut before, mut after) = (checked(3.0), checked(0.0));
    rest(&mut before).measures.pours = None;
    rest(&mut after).measures.pours = None;
    assert_eq!(
        stitch_refusals(&before, &after, &gnd()),
        Vec::<String>::new()
    );
}

#[test]
fn a_gate_verdict_that_changes_either_way_is_refused() {
    let mut after = checked(0.0);
    rest(&mut after).gates.g3 = vec!["a narrow path".into()];
    let refused = stitch_refusals(&checked(3.0), &after, &gnd());
    assert!(refused.contains(&"gate:g3".to_string()), "{refused:?}");
    // A gate that passes only with the vias: the status may not depend on them.
    let mut before = checked(3.0);
    rest(&mut before).gates.g1 = vec!["open".into()];
    let refused = stitch_refusals(&before, &checked(0.0), &gnd());
    assert!(refused.contains(&"gate:g1".to_string()), "{refused:?}");
    // The same verdict with other words is not a change.
    let (mut before, mut after) = (checked(3.0), checked(0.0));
    rest(&mut before).gates.g2 = vec!["a".into()];
    rest(&mut after).gates.g2 = vec!["b".into(), "c".into()];
    let refused = stitch_refusals(&before, &after, &gnd());
    assert!(
        !refused.iter().any(|c| c.starts_with("gate:")),
        "{refused:?}"
    );
}

#[test]
fn an_entry_above_in_either_key_is_refused_whatever_the_calls_key() {
    // Copper quality: both keys (today's 11, the pour key's 15).
    let mut after = checked(0.0);
    rest(&mut after).measures.copper_quality = Some(80.0);
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        quality_keys()
    );
    // A starved thermal: the pour key alone (entry 7).
    let mut after = checked(0.0);
    rest(&mut after).measures.pours = Some(pours(1));
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        vec![format!("key:pours:{}", crate::pour_rank::POUR_FAULTS)]
    );
    // The route seconds are not compared.
    let mut after = checked(0.0);
    rest(&mut after).measures.route_s = 99.0;
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        Vec::<String>::new()
    );
}

#[test]
fn the_unstitched_count_must_fall() {
    assert_eq!(
        stitch_refusals(&checked(3.0), &checked(3.0), &gnd()),
        vec!["not_better"]
    );
    assert_eq!(
        stitch_refusals(&checked(3.0), &checked(4.0), &gnd()),
        vec!["not_better"]
    );
    let mut after = checked(0.0);
    rest(&mut after).measures.checklist.unstitched_layer_changes = None;
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        vec!["not_better"]
    );
}

#[test]
fn another_nets_fill_may_not_shrink_and_the_stitched_nets_own_is_left_to_the_pour_key() {
    let mut after = checked(0.0);
    rest(&mut after).fills[2] = fill("VCC", "In1.Cu", 49.0);
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        vec!["fill:VCC/In1.Cu"]
    );
    // Within the tolerance.
    rest(&mut after).fills[2] = fill("VCC", "In1.Cu", 50.0 - pour_prune::FILL_TOLERANCE_MM2 / 2.0);
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        Vec::<String>::new()
    );
    // A zone layer that is gone counts as 0.
    rest(&mut after).fills.pop();
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        vec!["fill:VCC/In1.Cu"]
    );
    // The ground net's own fill changes with its own vias: not this clause.
    let mut after = checked(0.0);
    rest(&mut after).fills[0] = fill("GND", "B.Cu", 90.0);
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        Vec::<String>::new()
    );
    // Unless GND is not a stitched net.
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &BTreeSet::new()),
        vec!["fill:GND/B.Cu"]
    );
}

#[test]
fn a_check_that_no_longer_scores_and_worse_loops_are_refused() {
    let mut after = checked(0.0);
    rest(&mut after).audit.scored.remove("4.1");
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        vec!["unscored"]
    );
    let mut after = checked(0.0);
    rest(&mut after).audit.worst_loop_mm = Some(5.0 + pour_prune::LOOP_TOLERANCE_MM * 2.0);
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        vec!["loops"]
    );
}

#[test]
fn the_length_off_the_reference_may_rise_by_the_tolerance_only() {
    let mut after = checked(0.0);
    rest(&mut after).measures.checklist.length_off_reference_mm = Some(10.0 + 0.09);
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        Vec::<String>::new()
    );
    rest(&mut after).measures.checklist.length_off_reference_mm = Some(10.0 + 0.11);
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        vec!["reference"]
    );
    rest(&mut after).measures.checklist.length_off_reference_mm = None;
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        vec!["reference"]
    );
}

#[test]
fn more_floating_ground_is_refused() {
    let mut after = checked(0.0);
    rest(&mut after).measures.checklist.floating_ground_mm2 = Some(0.5);
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        vec!["ground"]
    );
    let mut after = checked(0.0);
    rest(&mut after).measures.checklist.floating_ground_islands = Some(1.0);
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        vec!["ground"]
    );
}

#[test]
fn the_delivery_compares_the_same_vias_and_refuses_a_higher_count() {
    // The delivered board holds the ranked board's vias: an equal count is right.
    assert_eq!(
        delivery_refusals(&checked(0.0), &checked(0.0), &gnd()),
        Vec::<String>::new()
    );
    assert_eq!(
        delivery_refusals(&checked(0.0), &checked(1.0), &gnd()),
        vec!["stitching"]
    );
    let mut after = checked(0.0);
    rest(&mut after).measures.copper_quality = Some(80.0);
    assert_eq!(
        delivery_refusals(&checked(0.0), &after, &gnd()),
        quality_keys()
    );
}

#[test]
fn the_record_holds_both_boards_as_the_guard_read_them() {
    let (summary_before, summary_after) = summaries(&checked(3.0), &checked(0.0));
    assert_eq!(summary_before.unconnected, 0);
    assert_eq!(summary_before.gates_passed, vec!["g1", "g2", "g3", "g4"]);
    assert_eq!(
        summary_before.key_routing.len(),
        crate::route_judge::KEY_LEN
    );
    assert_eq!(
        summary_before.key_pours.len(),
        crate::pour_rank::POUR_KEY_LEN
    );
    assert_eq!(summary_before.key_routing, summary_after.key_routing);
    assert_eq!(summary_before.fill_mm2["VCC/In1.Cu/z-VCC-In1.Cu"], 50.0);
    assert_eq!(summary_before.checklist.unstitched_layer_changes, Some(3.0));
    assert_eq!(summary_after.checklist.unstitched_layer_changes, Some(0.0));
    assert_eq!(summary_before.scored, vec!["3.2", "4.1"]);
    let alone = summary_alone(&checked(3.0));
    assert_eq!(alone.key_routing, summary_before.key_routing);
}

// ─── S6: the checklist items ─────────────────────────────────────────────────

#[test]
fn the_checklist_items_are_read_from_the_audits_metrics() {
    let audit = json!({ "metrics": {
        "3.1.length_off_reference_mm": 31.7,
        "3.2.unstitched_layer_changes": 4,
        "3.3.floating_ground_area_mm2": 1.5,
        "3.3.floating_ground_islands": 2,
        "4.1.worst_loop_mm": 19.0,
    }});
    assert_eq!(
        checklist_items(&audit),
        ChecklistItems {
            length_off_reference_mm: Some(31.7),
            unstitched_layer_changes: Some(4.0),
            floating_ground_mm2: Some(1.5),
            floating_ground_islands: Some(2.0),
            worst_loop_mm: Some(19.0),
        }
    );
    assert_eq!(checklist_items(&json!({})), ChecklistItems::default());
    assert_eq!(unstitched(&audit), Some(4.0));
}

// ─── S10: the padstack and the written session ───────────────────────────────

fn manifest() -> Value {
    let all = json!(["F.Cu", "In1.Cu", "B.Cu"]);
    json!({
        "layers": [
            { "kicad_name": "F.Cu", "dsn_name": "F.Cu", "index": 0 },
            { "kicad_name": "In1.Cu", "dsn_name": "In1.Cu", "index": 1 },
            { "kicad_name": "B.Cu", "dsn_name": "B.Cu", "index": 2 },
        ],
        "padstacks": [
            { "name": "pad_round", "purpose": "pad", "shape": "circle", "layers": all, "size_x_um": 600, "size_y_um": 600, "drill_um": 300 },
            { "name": "via_oval", "purpose": "via", "shape": "oval", "layers": all, "size_x_um": 600, "size_y_um": 600, "drill_um": 300 },
            // A blind via of the standard size, listed before it: never chosen.
            { "name": "via_blind", "purpose": "via", "shape": "circle", "layers": ["F.Cu", "In1.Cu"], "size_x_um": 600, "size_y_um": 600, "drill_um": 300 },
            { "name": "via_small", "purpose": "via", "shape": "circle", "layers": all, "size_x_um": 450, "size_y_um": 450, "drill_um": 200 },
            { "name": "via_std", "purpose": "via", "shape": "circle", "layers": all, "size_x_um": 600, "size_y_um": 600, "drill_um": 300 },
        ]
    })
}

#[test]
fn the_padstack_is_the_manifests_round_via_of_the_same_size_and_drill() {
    let m = manifest();
    assert_eq!(
        manifest_padstack(&m, 0.6, 0.3).unwrap().as_deref(),
        Some("via_std")
    );
    assert_eq!(
        manifest_padstack(&m, 0.6009, 0.3).unwrap().as_deref(),
        Some("via_std")
    );
    assert_eq!(
        manifest_padstack(&m, 0.45, 0.2).unwrap().as_deref(),
        Some("via_small")
    );
    assert_eq!(manifest_padstack(&m, 0.6011, 0.3).unwrap(), None);
    assert_eq!(manifest_padstack(&m, 0.6, 0.35).unwrap(), None);
    assert_eq!(manifest_padstack(&m, 0.8, 0.4).unwrap(), None);
    assert!(manifest_padstack(&json!({}), 0.6, 0.3).is_err());
    // A manifest without its layer table is refused, not read as covering nothing.
    let mut bare = manifest();
    bare.as_object_mut().unwrap().remove("layers");
    assert!(manifest_padstack(&bare, 0.6, 0.3).is_err());
}

const SESSION: &str = "(session \"b\"
  (routes
    (resolution um 10)
    (network_out
      (net GND
        (wire (path F.Cu 2000 1050000 -500000 1095000 -500000))
      )
      (net \"S 1\"
        (via konnect_via_0001 1000000 -500000)
      )
    )
  )
)";

#[test]
fn the_vias_go_into_their_nets_block_snapped_to_the_session_grid() {
    let vias = [
        // 106.00004 mm snaps to 1060000 units of 0.1 µm; y is mirrored.
        via("GND", 106.000_04, 52.0),
        via("S 1", 101.0, 50.0),
        via("VCC", 90.0, 40.000_06),
    ];
    let text = write_vias(SESSION, &vias, "konnect_via_0001").unwrap();
    // The GND via after the GND wire, the S 1 via after its own via, VCC in a new block.
    let gnd = text.find("(net GND").unwrap();
    let gnd_via = text
        .find("(via \"konnect_via_0001\" 1060000 -520000)")
        .unwrap();
    let s1 = text.find("(net \"S 1\"").unwrap();
    let s1_via = text
        .find("(via \"konnect_via_0001\" 1010000 -500000)")
        .unwrap();
    let vcc = text.find("(net \"VCC\"").unwrap();
    let vcc_via = text
        .find("(via \"konnect_via_0001\" 900000 -400001)")
        .unwrap();
    assert!(
        gnd < gnd_via && gnd_via < s1 && s1 < s1_via && s1_via < vcc && vcc < vcc_via,
        "{text}"
    );
    // The scanner reads it back: every via in its net.
    let all: BTreeSet<String> = ["GND", "S 1", "VCC"].map(str::to_string).into();
    let units = pour_prune::units(&text, &all).unwrap();
    let vias_of = |net: &str| {
        units
            .iter()
            .filter(|u| u.net == net && u.kind == pour_prune::UnitKind::Via)
            .map(|u| u.at.unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(vias_of("GND"), vec![[1060000, -520000]]);
    assert_eq!(vias_of("S 1"), vec![[1000000, -500000], [1010000, -500000]]);
    assert_eq!(vias_of("VCC"), vec![[900000, -400001]]);
    // Nothing else changed: the session's lines are all there, in order, and the new
    // ones are the three vias and the new net block's two lines.
    let lines: Vec<&str> = text.lines().collect();
    let old: Vec<&str> = SESSION.lines().collect();
    let mut rest = lines.iter();
    assert!(
        old.iter().all(|line| rest.any(|new| new == line)),
        "the session's own lines are kept in order: {text}"
    );
    assert_eq!(lines.len(), old.len() + 5, "{text}");
}

#[test]
fn a_name_the_session_cannot_hold_is_refused() {
    assert!(write_vias(SESSION, &[via("GND", 1.0, 1.0)], "bad\"name").is_err());
    assert!(write_vias(SESSION, &[via("A\"B", 1.0, 1.0)], "konnect_via_0001").is_err());
    assert!(write_vias("(pcb)", &[via("GND", 1.0, 1.0)], "konnect_via_0001").is_err());
    // No via: the session as it was.
    assert_eq!(
        write_vias(SESSION, &[], "konnect_via_0001").unwrap(),
        SESSION
    );
}

// ─── S5's retry: what a refusal names ────────────────────────────────────────

fn violation(rule: &str, at: (f64, f64)) -> DrcViolation {
    DrcViolation {
        severity: "error".into(),
        description: "x".into(),
        rule: rule.into(),
        pos: None,
        items: vec![ReportItem {
            description: "an item".into(),
            pos: Some(ReportPos { x: at.0, y: at.1 }),
            uuid: None,
            kicad_reported_pos: None,
            ownership_status: None,
            item_kind: None,
            layer: None,
            owner: None,
        }],
    }
}

fn report(violations: Vec<DrcViolation>, open: Vec<DrcViolation>) -> DrcReport {
    DrcReport {
        violations,
        unconnected_items: Some(open),
        schematic_parity: None,
        ownership_diagnostic: None,
        schematic_parity_diagnostic: None,
    }
}

fn zone(net: &str, layer: &str, ring: &[(f64, f64)]) -> String {
    let pts: String = ring.iter().map(|(x, y)| format!("(xy {x} {y})")).collect();
    format!("(zone (net 1) (net_name \"{net}\") (layer \"{layer}\") (uuid \"z-{net}\") (filled_polygon (layer \"{layer}\") (pts {pts})))")
}

fn board(zones: &[String]) -> String {
    format!(
        "(kicad_pcb (net 0 \"\") (net 1 \"GND\") {})",
        zones.join(" ")
    )
}

#[test]
fn a_drc_item_names_the_vias_at_its_position_and_a_fill_clause_the_vias_in_that_fill() {
    let vias = [via("GND", 10.0, 10.0), via("GND", 20.0, 10.0)];
    let empty = board(&[]);
    // A clearance item at the first via's edge names it alone.
    let drc = report(vec![violation("clearance", (10.7, 10.0))], vec![]);
    let named = named_vias(
        &["rule:clearance".to_string()],
        &vias,
        &drc,
        None,
        &empty,
        &empty,
    );
    assert_eq!(named, BTreeSet::from([0]));
    // Beyond the reach: none.
    let drc = report(vec![violation("clearance", (10.0, 10.9))], vec![]);
    let named = named_vias(
        &["rule:clearance".to_string()],
        &vias,
        &drc,
        None,
        &empty,
        &empty,
    );
    assert!(named.is_empty());
    // Only the refusing rule's items are read.
    let drc = report(vec![violation("silk_overlap", (20.0, 10.0))], vec![]);
    let named = named_vias(
        &["rule:clearance".to_string()],
        &vias,
        &drc,
        None,
        &empty,
        &empty,
    );
    assert!(named.is_empty());
    // An unconnected item names the via at it.
    let drc = report(vec![], vec![violation("unconnected_items", (20.0, 10.2))]);
    let named = named_vias(
        &["unconnected".to_string()],
        &vias,
        &drc,
        None,
        &empty,
        &empty,
    );
    assert_eq!(named, BTreeSet::from([1]));
    // An island reported at one of its vertices names the vias inside it.
    let island = zone(
        "GND",
        "B.Cu",
        &[(18.0, 8.0), (22.0, 8.0), (22.0, 12.0), (18.0, 12.0)],
    );
    let drc = report(vec![violation("isolated_copper", (18.0, 8.0))], vec![]);
    let stitched = board(&[island]);
    let named = named_vias(
        &["rule:isolated_copper".to_string()],
        &vias,
        &drc,
        None,
        &empty,
        &stitched,
    );
    assert_eq!(named, BTreeSet::from([1]));
    // A fill clause names the vias in that net's fill on that layer of the board before.
    let plane = zone(
        "GND",
        "In1.Cu",
        &[(5.0, 5.0), (15.0, 5.0), (15.0, 15.0), (5.0, 15.0)],
    );
    let before = board(&[plane]);
    let named = named_vias(
        &["fill:GND/In1.Cu".to_string()],
        &vias,
        &report(vec![], vec![]),
        None,
        &before,
        &empty,
    );
    assert_eq!(named, BTreeSet::from([0]));
    let named = named_vias(
        &["fill:GND/In2.Cu".to_string()],
        &vias,
        &report(vec![], vec![]),
        None,
        &before,
        &empty,
    );
    assert!(named.is_empty());
    // A clause that names no copper names no via.
    let drc = report(vec![violation("clearance", (10.0, 10.0))], vec![]);
    let named = named_vias(
        &["key:routing:11".to_string(), "loops".to_string()],
        &vias,
        &drc,
        None,
        &before,
        &empty,
    );
    assert!(named.is_empty());
}

#[test]
fn a_ring_is_reached_from_inside_and_within_the_reach_of_its_edge() {
    let square = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];
    assert!(reaches_ring(&square, (2.0, 2.0), 0.3));
    assert!(reaches_ring(&square, (4.7, 2.0), 0.3));
    assert!(!reaches_ring(&square, (4.9, 2.0), 0.3));
    assert!(!reaches_ring(&square[..2], (2.0, 0.0), 0.3), "not a ring");
}

// ─── S13: the placement tools' switch ────────────────────────────────────────

#[test]
fn the_placement_tools_route_with_the_round_off_and_prepare_gives_off() {
    assert_eq!(crate::place_route::CANDIDATE_STITCH, ReferenceStitch::Off);
}

// ─── The mutants batch's misses (shards 0 to 5), each killed here ────────────

#[test]
fn a_pour_record_due_on_one_side_only_is_unchecked() {
    // The stitched board took no record and no error, the post-Z9 one did: due.
    let mut after = checked(0.0);
    rest(&mut after).measures.pours = None;
    rest(&mut after).measures.pours_error = None;
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        vec!["unchecked"]
    );
}

#[test]
fn a_fill_exactly_at_the_tolerance_is_kept() {
    let mut after = checked(0.0);
    rest(&mut after).fills[2] = fill("VCC", "In1.Cu", 50.0 - pour_prune::FILL_TOLERANCE_MM2);
    assert_eq!(
        stitch_refusals(&checked(3.0), &after, &gnd()),
        Vec::<String>::new()
    );
}

#[test]
fn the_summary_holds_the_drc_counts() {
    let mut board = checked(3.0);
    board.unconnected = 2;
    let summary = summary_alone(&board);
    assert_eq!(summary.unconnected, 2);
    assert_eq!(
        summary.violations,
        BTreeMap::from([("silk_overlap".to_string(), 2)])
    );
}

#[test]
fn a_position_that_is_no_fill_vertex_names_by_distance_even_beside_a_fill() {
    let vias = [via("GND", 10.0, 10.0), via("GND", 20.0, 10.0)];
    // A fill piece around the second via, none of its vertices at the item.
    let piece = zone(
        "GND",
        "B.Cu",
        &[(18.0, 8.0), (22.0, 8.0), (22.0, 12.0), (18.0, 12.0)],
    );
    let stitched = board(&[piece]);
    let drc = report(vec![violation("clearance", (10.7, 10.0))], vec![]);
    let named = named_vias(
        &["rule:clearance".to_string()],
        &vias,
        &drc,
        None,
        &board(&[]),
        &stitched,
    );
    assert_eq!(named, BTreeSet::from([0]));
}

#[test]
fn a_fill_clause_reaches_a_via_by_its_own_radius() {
    // The plane's edge at x = 15: a 0.6 mm via 0.9 mm off it is out of reach (0.3 + 0.5).
    let vias = [via("GND", 15.9, 10.0), via("GND", 15.7, 10.0)];
    let plane = zone(
        "GND",
        "In1.Cu",
        &[(5.0, 5.0), (15.0, 5.0), (15.0, 15.0), (5.0, 15.0)],
    );
    let named = named_vias(
        &["fill:GND/In1.Cu".to_string()],
        &vias,
        &report(vec![], vec![]),
        None,
        &board(&[plane]),
        &board(&[]),
    );
    assert_eq!(named, BTreeSet::from([1]));
}

#[test]
fn a_via_goes_inside_a_net_block_with_no_route_and_into_an_empty_network() {
    let empty_net = "(session \"b\"
  (routes
    (resolution um 10)
    (network_out
      (net GND
      )
    )
  )
)";
    let empty_network = "(session \"b\"
  (routes
    (resolution um 10)
    (network_out
    )
  )
)";
    let gnd: BTreeSet<String> = BTreeSet::from(["GND".to_string()]);
    for session in [empty_net, empty_network] {
        let text = write_vias(session, &[via("GND", 1.0, 2.0)], "konnect_via_0001").unwrap();
        let units = pour_prune::units(&text, &gnd).unwrap();
        let points: Vec<[i64; 2]> = units.iter().filter_map(|unit| unit.at).collect();
        assert_eq!(points, vec![[10000, -20000]], "{text}");
        // The text still parses as one session.
        konnect_sexp::parse_sexp(&text).unwrap();
        assert!(text.trim_end().ends_with(")"), "{text}");
    }
}

#[test]
fn an_unchecked_delivery_is_refused_by_that_clause_alone() {
    let mut delivered = checked(0.0);
    delivered.rest = None;
    assert_eq!(
        delivery_refusals(&checked(0.0), &delivered, &gnd()),
        vec!["unchecked"]
    );
}

/// Review item 1: a kept via that closes an open lowers the unconnected count, so the
/// stitched report no longer lists that open; the post-Z9 report's item names the via.
#[test]
fn a_lower_unconnected_count_names_the_via_at_the_post_z9_open() {
    let vias = [via("GND", 10.0, 10.0), via("GND", 20.0, 10.0)];
    let empty = board(&[]);
    let before = report(vec![], vec![violation("unconnected_items", (20.0, 10.3))]);
    let after = report(vec![], vec![]);
    let refused = ["unconnected".to_string()];
    assert_eq!(
        named_vias(&refused, &vias, &after, Some(&before), &empty, &empty),
        BTreeSet::from([1])
    );
    // A higher count reads the stitched report, as before.
    assert_eq!(
        named_vias(&refused, &vias, &before, Some(&after), &empty, &empty),
        BTreeSet::from([1])
    );
    // An open reported at a vertex of a post-Z9 fill piece names the vias that reach it.
    let piece = zone(
        "GND",
        "B.Cu",
        &[(8.0, 8.0), (12.0, 8.0), (12.0, 12.0), (8.0, 12.0)],
    );
    let at_vertex = report(vec![], vec![violation("unconnected_items", (8.0, 8.0))]);
    assert_eq!(
        named_vias(
            &refused,
            &vias,
            &after,
            Some(&at_vertex),
            &board(&[piece]),
            &empty
        ),
        BTreeSet::from([0])
    );
    // Without the post-Z9 report nothing names it.
    assert!(named_vias(&refused, &vias, &after, None, &empty, &empty).is_empty());
}
