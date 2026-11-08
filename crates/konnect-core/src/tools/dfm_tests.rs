use super::*;
use konnect_sexp::parser::parse_sexp;

/// A board with one item per check: a 0.1 mm track, an undersized via, a
/// via in a same-net SMD pad and one in another net's pad, an undersized
/// plated hole, one acute and one square track junction.
pub(crate) fn board(extra: &str) -> String {
    format!(
        r#"(kicad_pcb (version 20260206) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 60 40) (stroke (width 0.05) (type solid)) (fill no) (layer "Edge.Cuts") (uuid "e1"))
  (footprint "Fixture:U" (layer "F.Cu") (uuid "fu") (at 10 10 0)
    (property "Reference" "U1" (at 0 -3 0) (layer "F.SilkS") (uuid "fu-r") (effects (font (size 1 1))))
    (pad "1" smd rect (at 0 0 0) (size 1 1) (layers "F.Cu" "F.Mask" "F.Paste") (net "N1") (uuid "p1"))
    (pad "2" smd rect (at 2 0 0) (size 1 1) (layers "F.Cu" "F.Mask" "F.Paste") (net "N2") (uuid "p2"))
    (pad "3" smd rect (at 4 0 0) (size 1 1) (layers "F.Cu" "F.Mask" "F.Paste") (net "N2") (uuid "p3")))
  (footprint "Fixture:J" (layer "F.Cu") (uuid "fj") (at 30 10 0)
    (property "Reference" "J1" (at 0 -3 0) (layer "F.SilkS") (uuid "fj-r") (effects (font (size 1 1))))
    (pad "1" thru_hole circle (at 0 0 0) (size 0.45 0.45) (drill 0.2) (layers "*.Cu" "*.Mask") (net "N4") (uuid "pj1"))
    (pad "2" thru_hole circle (at 3 0 0) (size 1.7 1.7) (drill 1.0) (layers "*.Cu" "*.Mask") (net "N4") (uuid "pj2")))
  (segment (start 40 5) (end 45 5) (width 0.1) (layer "F.Cu") (net "N1") (uuid "t-narrow"))
  (segment (start 40 8) (end 45 8) (width 0.25) (layer "F.Cu") (net "N1") (uuid "t-ok"))
  (via (at 50 5) (size 0.4) (drill 0.2) (layers "F.Cu" "B.Cu") (net "N1") (uuid "v-small"))
  (via (at 50 10) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net "N1") (uuid "v-ok"))
  (via (at 10 10) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net "N1") (uuid "v-inpad"))
  (via (at 12 10) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net "N1") (uuid "v-short"))
  (segment (start 20 30) (end 25 30) (width 0.25) (layer "F.Cu") (net "N3") (uuid "a1"))
  (segment (start 20 30) (end 25 31) (width 0.25) (layer "F.Cu") (net "N3") (uuid "a2"))
  (segment (start 35 30) (end 40 30) (width 0.25) (layer "F.Cu") (net "N3") (uuid "s1"))
  (segment (start 35 30) (end 35 35) (width 0.25) (layer "F.Cu") (net "N3") (uuid "s2"))
  {extra}
)"#
    )
}

fn geometry(text: &str) -> BoardGeometry {
    BoardGeometry::from_tree(&parse_sexp(text).unwrap())
}

fn jlc() -> FabLimits {
    fab_limits(Some("jlcpcb"), None)
}

fn issues_with<'a>(report: &'a DfmReport, needle: &str) -> Vec<&'a DfmIssue> {
    report
        .issues
        .iter()
        .filter(|i| i.issue.contains(needle))
        .collect()
}

#[test]
fn fab_values_are_used_not_read_into_unused_variables() {
    let limits = jlc();
    assert_eq!(limits.min_track.mm, 0.127);
    assert_eq!(limits.min_via_drill.mm, 0.3);
    assert_eq!(limits.min_annular.mm, 0.13);
    assert!((limits.min_via_diameter.mm - 0.56).abs() < 1e-9);
    assert_eq!(limits.min_track.source, "built_in_table");
    let report = check_board(&geometry(&board("")), &limits);
    for check in [
        "track_width",
        "via_size",
        "pad_holes",
        "via_in_pad",
        "acid_trap",
    ] {
        assert_eq!(report.checks[check].status, "checked", "{check}");
    }
}

#[test]
fn a_track_below_the_fab_minimum_is_an_error_with_its_uuid() {
    let report = check_board(&geometry(&board("")), &jlc());
    let narrow = issues_with(&report, "minimum track width");
    assert_eq!(narrow.len(), 1, "{:?}", report.issues);
    assert_eq!(narrow[0].severity, "error");
    assert!(narrow[0].issue.contains("narrowest 0.100 mm"));
    assert_eq!(narrow[0].at["uuids"], json!(["t-narrow"]));
    assert_eq!(report.checks["track_width"].checked, 6);
    assert_eq!(report.checks["track_width"].failed, 1);
}

#[test]
fn an_undersized_via_fails_drill_diameter_and_annular_ring() {
    let report = check_board(&geometry(&board("")), &jlc());
    for what in ["drill below", "minimum via diameter", "mm annular ring"] {
        let hits = issues_with(&report, what);
        assert_eq!(hits.len(), 1, "{what}: {:?}", report.issues);
        assert_eq!(hits[0].at["uuids"], json!(["v-small"]), "{what}");
    }
    // One via, three defects: counted once.
    assert_eq!(report.checks["via_size"].failed, 1);
}

#[test]
fn a_small_plated_hole_and_thin_ring_name_the_part_and_pad() {
    let report = check_board(&geometry(&board("")), &jlc());
    let drill = issues_with(&report, "minimum drill");
    assert_eq!(drill.len(), 1, "{:?}", report.issues);
    assert_eq!(drill[0].component.as_deref(), Some("J1"));
    assert!(drill[0].issue.contains("pads 1"), "{}", drill[0].issue);
    let ring = issues_with(&report, "plated pad(s) have an annular ring");
    assert_eq!(ring.len(), 1);
    // Pad 2 (1.7 mm, 1.0 mm drill, 0.35 mm ring) passes.
    assert!(!ring[0].issue.contains("pads 1, 2"), "{}", ring[0].issue);
}

#[test]
fn via_in_pad_warns_on_its_own_net_and_errors_on_another() {
    let report = check_board(&geometry(&board("")), &jlc());
    let same = issues_with(&report, "pad 1: via-in-pad");
    assert_eq!(same.len(), 1, "{:?}", report.issues);
    assert_eq!(same[0].severity, "warning");
    assert_eq!(same[0].at["via_uuids"], json!(["v-inpad"]));
    let short = issues_with(&report, "pad 2: a via of another net");
    assert_eq!(short.len(), 1);
    assert_eq!(short[0].severity, "error");
    // Pad 3 has no via.
    assert!(issues_with(&report, "pad 3").is_empty());
}

#[test]
fn an_acute_junction_is_an_acid_trap_and_a_square_one_is_not() {
    let report = check_board(&geometry(&board("")), &jlc());
    let traps = issues_with(&report, "Acid trap");
    assert_eq!(traps.len(), 1, "{:?}", report.issues);
    assert_eq!(traps[0].at["x"], 20.0);
    assert_eq!(traps[0].at["y"], 30.0);
    assert!(traps[0].issue.contains("11°"), "{}", traps[0].issue);
    assert_eq!(report.checks["acid_trap"].checked, 2);
}

#[test]
fn an_acute_junction_inside_a_pad_of_its_net_is_skipped_and_counted() {
    // Two N1 tracks meeting at 45° on the centre of U1 pad 1.
    let extra = r#"(segment (start 10 10) (end 15 10) (width 0.2) (layer "F.Cu") (net "N1") (uuid "x1"))
  (segment (start 10 10) (end 15 15) (width 0.2) (layer "F.Cu") (net "N1") (uuid "x2"))"#;
    let report = check_board(&geometry(&board(extra)), &jlc());
    assert_eq!(issues_with(&report, "Acid trap").len(), 1);
    assert!(report.checks["acid_trap"]
        .reason
        .as_deref()
        .unwrap()
        .contains("1 acute junction"));
}

#[test]
fn design_intent_minimums_override_the_table_for_the_same_house() {
    let target = FabTarget {
        house: Some("JLCPCB".to_string()),
        min_track_mm: Some(0.09),
        min_via_drill_mm: Some(0.2),
        ..FabTarget::default()
    };
    let limits = fab_limits(None, Some(&target));
    assert_eq!(limits.house, "jlcpcb");
    assert_eq!(limits.min_track.mm, 0.09);
    assert_eq!(limits.min_track.source, "design_intent");
    assert_eq!(limits.min_via_drill.source, "design_intent");
    assert_eq!(limits.min_clearance.source, "built_in_table");
    // The narrow track and small via drill now pass.
    let report = check_board(&geometry(&board("")), &limits);
    assert!(issues_with(&report, "minimum track width").is_empty());
    assert!(issues_with(&report, "drill below").is_empty());
}

#[test]
fn another_house_on_the_call_does_not_take_the_intent_minimums() {
    let target = FabTarget {
        house: Some("jlcpcb".to_string()),
        min_track_mm: Some(0.09),
        ..FabTarget::default()
    };
    let limits = fab_limits(Some("oshpark"), Some(&target));
    assert_eq!(limits.house, "oshpark");
    assert_eq!(limits.min_track.mm, 0.152);
    assert!(limits.notes.iter().any(|n| n.contains("not applied")));
}

#[test]
fn an_unknown_house_says_its_limits_are_generic() {
    let limits = fab_limits(Some("acme"), None);
    assert_eq!(limits.min_track.source, "generic_default");
    assert!(limits
        .notes
        .iter()
        .any(|n| n.contains("no built-in limits")));
}

#[test]
fn project_rules_below_the_fab_capability_are_flagged() {
    let project = json!({ "board": { "design_settings": { "rules": {
        "min_track_width": 0.1,
        "min_clearance": 0.2,
        "min_via_annular_width": 0.05
    }}}});
    let mut report = DfmReport::default();
    check_rule_minimums(&project, &jlc(), &mut report);
    assert_eq!(report.checks["project_rule_minimums"].checked, 3);
    assert_eq!(report.checks["project_rule_minimums"].failed, 2);
    assert!(report
        .issues
        .iter()
        .any(|i| i.issue.contains("minimum track width rule (0.100 mm)")));
    assert!(report
        .issues
        .iter()
        .all(|i| !i.issue.contains("minimum clearance")));
}
