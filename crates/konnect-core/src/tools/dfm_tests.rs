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

// ─── S7b: the corrected acid-trap reading ────────────────────────────────────
//
// A track shorter than 10 um belongs to the junction it touches and gives no
// direction; a corner is acute only below 89 degrees. Each half is tested on
// its own: a short track at a 135-degree bend (only the first rule clears
// it) and a junction at 89.97 degrees with no short track (only the second).

const JN: (f64, f64) = (20.0, 20.0);

/// A straight track from one point to another.
type Track = ((f64, f64), (f64, f64));

/// A point `length` mm from the junction at `degrees`, on the 1 nm grid.
fn at(degrees: f64, length: f64) -> (f64, f64) {
    let r = degrees.to_radians();
    (
        ((JN.0 + length * r.cos()) * 1e6).round() / 1e6,
        ((JN.1 + length * r.sin()) * 1e6).round() / 1e6,
    )
}

/// A board holding only the given SIG tracks on F.Cu, 0.2 mm wide.
fn trap_board(tracks: &[Track]) -> String {
    let mut segs = String::new();
    for (i, (a, b)) in tracks.iter().enumerate() {
        segs += &format!(
            "  (segment (start {} {}) (end {} {}) (width 0.2) (layer \"F.Cu\") (net \"SIG\") (uuid \"t{i}\"))\n",
            a.0, a.1, b.0, b.1
        );
    }
    format!(
        r#"(kicad_pcb (version 20260206) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 60 40) (stroke (width 0.05) (type solid)) (fill no) (layer "Edge.Cuts") (uuid "e1"))
{segs})"#
    )
}

fn acid_traps(tracks: &[Track]) -> (usize, Coverage) {
    let report = check_board(&geometry(&trap_board(tracks)), &jlc());
    (
        issues_with(&report, "Acid trap").len(),
        report.checks["acid_trap"].clone(),
    )
}

/// Legs from the junction at the given angles, 3 mm long.
fn legs(angles: &[f64]) -> Vec<Track> {
    angles.iter().map(|a| (JN, at(*a, 3.0))).collect()
}

/// A 0.1-um track at the junction, along +y: its two ends share one key.
fn stub() -> Track {
    (JN, (JN.0, JN.1 + 0.0001))
}

#[test]
fn a_short_track_at_a_135_degree_bend_is_not_an_acid_trap() {
    let mut t = legs(&[180., 45.]);
    // Read as the audit read it before: the stub's two directions make 45 degrees.
    t.push(stub());
    let (traps, cov) = acid_traps(&t);
    assert_eq!(traps, 0, "{cov:?}");
    assert_eq!(cov.checked, 1, "one junction of two real legs: {cov:?}");
    assert_eq!(cov.short_tracks, Some(1), "{cov:?}");
}

#[test]
fn a_short_track_on_a_straight_run_is_not_an_acid_trap() {
    let mut t = legs(&[180., 0.]);
    t.push((JN, (JN.0 - 0.0002, JN.1 + 0.0002)));
    assert_eq!(acid_traps(&t).0, 0);
}

#[test]
fn a_short_track_at_a_right_angle_bend_is_not_an_acid_trap() {
    let mut t = legs(&[180., 90.]);
    t.push(stub());
    assert_eq!(acid_traps(&t).0, 0);
}

/// A 2.8-um track bridging a trunk end to a perpendicular branch 2 um away:
/// two keys, one junction.
#[test]
fn a_short_track_at_a_three_way_junction_is_not_an_acid_trap() {
    let j1 = (JN.0 + 0.002, JN.1 + 0.002);
    let t = vec![
        (JN, at(90., 3.)),
        (JN, at(270., 3.)),
        (j1, (j1.0 + 3., j1.1)),
        (JN, j1),
    ];
    let (traps, cov) = acid_traps(&t);
    assert_eq!(traps, 0, "{cov:?}");
    assert_eq!(cov.checked, 1, "{cov:?}");
}

#[test]
fn a_junction_at_89_97_degrees_is_not_an_acid_trap() {
    let (traps, cov) = acid_traps(&legs(&[0., 89.97, 180.]));
    assert_eq!(traps, 0, "{cov:?}");
    assert_eq!(cov.short_tracks, Some(0));
}

#[test]
fn a_real_45_degree_v_stays_an_acid_trap_with_or_without_a_short_track() {
    assert_eq!(acid_traps(&legs(&[180., 135.])).0, 1);
    let mut t = legs(&[180., 135.]);
    t.push(stub());
    let report = check_board(&geometry(&trap_board(&t)), &jlc());
    let traps = issues_with(&report, "Acid trap");
    assert_eq!(traps.len(), 1);
    assert!(traps[0].issue.contains("45°"), "{}", traps[0].issue);
    assert_eq!(traps[0].at["x"], JN.0);
    assert_eq!(traps[0].at["y"], JN.1);
}

/// 9.99 um still belongs to the junction; 10.01 um is a leg of its own,
/// whose direction then makes a 45-degree corner with the bend's second leg.
#[test]
fn ten_micrometres_is_where_a_track_stops_belonging_to_its_junction() {
    for (length, expected) in [(0.00999, 0), (0.01001, 1)] {
        let mut t = legs(&[180., 45.]);
        t.push((JN, (JN.0, JN.1 + length)));
        assert_eq!(acid_traps(&t).0, expected, "track of {length} mm");
    }
}

#[test]
fn eighty_nine_degrees_is_where_a_corner_stops_being_acute() {
    for (angle, expected) in [(88.99, 1), (89.01, 0)] {
        assert_eq!(
            acid_traps(&legs(&[0., angle])).0,
            expected,
            "{angle} degrees"
        );
    }
}

/// A pad of the net covers the far end of a 5-um track at a real 45-degree V,
/// but not the point where the V's legs meet: the wedge is open, so the trap
/// is counted. Only a junction whose legs all meet inside copper is skipped.
#[test]
fn a_junction_is_skipped_only_where_its_legs_meet_inside_copper() {
    let mut t = legs(&[180., 225.]);
    t.push((JN, (JN.0, JN.1 + 0.005)));
    let segs: String = t
        .iter()
        .enumerate()
        .map(|(i, (a, b))| format!("(segment (start {} {}) (end {} {}) (width 0.2) (layer \"F.Cu\") (net \"SIG\") (uuid \"t{i}\"))", a.0, a.1, b.0, b.1))
        .collect();
    // The pad's edge passes 3 um above the junction, 2 um below the track's far end.
    let pad = format!(
        "(footprint \"Fixture:P\" (layer \"F.Cu\") (uuid \"fp\") (at {} {} 0) (property \"Reference\" \"P1\" (at 0 0 0) (layer \"F.SilkS\") (uuid \"fp-r\") (effects (font (size 1 1)))) (pad \"1\" smd circle (at 0 0 0) (size 0.8 0.8) (layers \"F.Cu\" \"F.Mask\") (net \"SIG\") (uuid \"pp\")))",
        JN.0,
        JN.1 + 0.403
    );
    let text = trap_board(&[]).replace("\n)", &format!("\n{segs}\n{pad})"));
    let report = check_board(&geometry(&text), &jlc());
    assert_eq!(
        issues_with(&report, "Acid trap").len(),
        1,
        "{:?}",
        report.checks["acid_trap"]
    );
    // Inside the pad at the junction itself: skipped, as before.
    let inside = text.replace(
        &format!("(at {} {} 0)", JN.0, JN.1 + 0.403),
        &format!("(at {} {} 0)", JN.0, JN.1 + 0.1),
    );
    let report = check_board(&geometry(&inside), &jlc());
    assert_eq!(issues_with(&report, "Acid trap").len(), 0);
    assert!(report.checks["acid_trap"]
        .reason
        .as_deref()
        .unwrap()
        .contains("1 acute junction"));
}

/// A real 45-degree V whose two legs end at keys 2 um apart, joined by a
/// short track: one junction, one acid trap. Each key alone holds one end.
#[test]
fn a_short_track_joins_the_keys_of_one_junction() {
    let j1 = (JN.0 + 0.002, JN.1);
    let t = vec![
        (JN, at(180., 3.)),
        (JN, j1),
        (j1, (j1.0 - 2.121320, j1.1 + 2.121320)),
    ];
    let (traps, cov) = acid_traps(&t);
    assert_eq!(traps, 1, "{cov:?}");
    assert_eq!(cov.checked, 1);
}

/// A real 45-degree V whose legs end at two keys 5 um apart, joined by a
/// short track: a pad of the net holds one leg's end and not the other's.
/// The wedge is not filled, so the trap is counted; with both ends inside
/// the pad it is skipped.
#[test]
fn a_junction_is_skipped_only_when_every_leg_end_is_inside_copper() {
    let j1 = (JN.0 + 0.005, JN.1);
    let t = [
        (JN, at(180., 3.)),
        (JN, j1),
        (j1, (j1.0 - 2.12132, j1.1 + 2.12132)),
    ];
    let segs: String = t
        .iter()
        .enumerate()
        .map(|(i, (a, b))| format!("(segment (start {} {}) (end {} {}) (width 0.2) (layer \"F.Cu\") (net \"SIG\") (uuid \"t{i}\"))", a.0, a.1, b.0, b.1))
        .collect();
    let pad_at = |x: f64| {
        format!(
        "(footprint \"Fixture:P\" (layer \"F.Cu\") (uuid \"fp\") (at {x} {} 0) (property \"Reference\" \"P1\" (at 0 0 0) (layer \"F.SilkS\") (uuid \"fp-r\") (effects (font (size 1 1)))) (pad \"1\" smd circle (at 0 0 0) (size 0.8 0.8) (layers \"F.Cu\" \"F.Mask\") (net \"SIG\") (uuid \"pp\")))",
        JN.1
    )
    };
    // Pad edge between the keys: the second leg's end 2 um inside, the first 3 um outside.
    let one = trap_board(&[]).replace("\n)", &format!("\n{segs}\n{})", pad_at(j1.0 + 0.398)));
    let report = check_board(&geometry(&one), &jlc());
    assert_eq!(
        issues_with(&report, "Acid trap").len(),
        1,
        "{:?}",
        report.checks["acid_trap"]
    );
    // Both ends inside.
    let both = trap_board(&[]).replace("\n)", &format!("\n{segs}\n{})", pad_at(JN.0 + 0.1)));
    let report = check_board(&geometry(&both), &jlc());
    assert_eq!(issues_with(&report, "Acid trap").len(), 0);
}

// The router leaves its tracks under 10 um while it routes: 45-degree corner
// smoothing stops 0.1 um short of the next corner, and wire ends are snapped
// to a via or pad centre. Diagonal pieces reach 6.8 x 6.8 um (9.6 um).

/// A diagonal piece at the 135-degree bend, its direction 45 degrees from the
/// first leg: 6.8 x 6.8 um and 7.07 x 7.07 um (just under 10 um) belong to
/// the junction; 7.08 x 7.08 um (10.01 um) is a leg of its own.
#[test]
fn a_diagonal_piece_belongs_to_its_junction_below_ten_micrometres() {
    for (side, expected) in [(0.0068, 0), (0.00707, 0), (0.00708, 1)] {
        let mut t = legs(&[180., 45.]);
        t.push((JN, (JN.0 - side, JN.1 + side)));
        let (traps, cov) = acid_traps(&t);
        assert_eq!(traps, expected, "diagonal piece of {side} mm: {cov:?}");
        assert_eq!(cov.short_tracks, Some(1 - expected), "{cov:?}");
    }
}

/// A real 45-degree V at a via of the net: one leg ends at the via centre,
/// the other 2.8 um away, joined to the centre by the piece the router left
/// when it snapped the wire end. Every leg end lies inside the via, so the
/// junction is skipped; without the via, or with a via of another net, the
/// V is one acid trap.
#[test]
fn a_piece_snapped_to_a_via_centre_is_read_inside_that_via() {
    let q = (JN.0 + 0.0024, JN.1 + 0.0015);
    let t = vec![
        (JN, at(180., 3.)),
        (q, JN),
        (q, (q.0 - 2.12132, q.1 + 2.12132)),
    ];
    let with_via = |net: &str| {
        trap_board(&t).replace(
            "\n)",
            &format!("\n  (via (at {} {}) (size 0.6) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") (net \"{net}\") (uuid \"v1\"))\n)", JN.0, JN.1),
        )
    };
    let report = check_board(&geometry(&with_via("SIG")), &jlc());
    assert_eq!(issues_with(&report, "Acid trap").len(), 0);
    assert!(report.checks["acid_trap"]
        .reason
        .as_deref()
        .unwrap()
        .contains("1 acute junction"));
    assert_eq!(report.checks["acid_trap"].short_tracks, Some(1));
    assert_eq!(acid_traps(&t).0, 1);
    let report = check_board(&geometry(&with_via("OTHER")), &jlc());
    assert_eq!(issues_with(&report, "Acid trap").len(), 1);
    // Nothing skipped, so no reason is given.
    assert_eq!(report.checks["acid_trap"].reason, None);
}

/// Exactly 89 degrees is a right angle and not acute: the two readings split
/// the line without overlap or gap.
#[test]
fn exactly_eighty_nine_degrees_is_a_right_angle_not_an_acute_corner() {
    assert!(!is_acute(89.0));
    assert!(is_right_angle(89.0));
    assert!(is_acute(89.0 - 1e-9));
    assert!(!is_right_angle(89.0 - 1e-9));
}

/// A track of exactly 10 um is a leg, not part of the junction: "shorter than
/// 10 um" is strict. On the line y = 0 the track's length is the double 0.01
/// itself, so the comparison is exact; its direction makes a 45-degree corner
/// with the bend's second leg.
#[test]
fn a_track_of_exactly_ten_micrometres_is_a_leg() {
    let j = (20.0, 0.0);
    let t = [
        (j, (17.0, 0.0)),
        (j, (22.12132, 2.12132)),
        (j, (20.0, SHORT_TRACK_MM)),
    ];
    let (traps, cov) = acid_traps(&t);
    assert_eq!(traps, 1, "{cov:?}");
    assert_eq!(cov.short_tracks, Some(0), "{cov:?}");
}

/// An arc is a track: its end piece is a leg of the junction it ends at. An
/// arc leaving the junction at 135 degrees (a radius of 1 mm, tangent 45
/// degrees from the straight leg) is one acid trap.
#[test]
fn an_arc_is_a_leg_of_its_junction() {
    let arc = format!(
        "  (arc (start {} {}) (mid 19.707107 20.707107) (end 20 21.414214) (width 0.2) (layer \"F.Cu\") (net \"SIG\") (uuid \"a1\"))\n)",
        JN.0, JN.1
    );
    let text = trap_board(&[(JN, at(180., 3.))]).replace("\n)", &format!("\n{arc}"));
    let report = check_board(&geometry(&text), &jlc());
    let traps = issues_with(&report, "Acid trap");
    assert_eq!(traps.len(), 1, "{:?}", report.checks["acid_trap"]);
    assert_eq!(report.checks["acid_trap"].checked, 1);
}

/// Keys joined by a chain of short tracks are one junction: a real 45-degree
/// V whose legs end 8 um apart, joined by two 4-um pieces in series. In this
/// order the first key's link runs through the second to the third.
#[test]
fn a_chain_of_short_tracks_joins_its_keys_into_one_junction() {
    let (j1, j2) = ((JN.0 + 0.004, JN.1), (JN.0 + 0.008, JN.1));
    let t = [
        (JN, at(180., 3.)),
        (JN, j1),
        (j1, j2),
        (j2, (j2.0 - 2.12132, j2.1 + 2.12132)),
    ];
    let (traps, cov) = acid_traps(&t);
    assert_eq!((traps, cov.checked), (1, 1), "{cov:?}");
    assert_eq!(cov.short_tracks, Some(2));
}

// ─── S7b review M1, S3: the cluster's bound and the 10 um line in nanometres ─

/// A point on the 1 nm grid.
fn on_nm(p: (f64, f64)) -> (f64, f64) {
    ((p.0 * 1e6).round() / 1e6, (p.1 * 1e6).round() / 1e6)
}

/// A run of `n` pieces of `step` mm from `from` in direction `degrees`.
fn chain(from: (f64, f64), degrees: f64, step: f64, n: usize) -> Vec<Track> {
    let r = degrees.to_radians();
    let p = |i: usize| {
        on_nm((
            from.0 + i as f64 * step * r.cos(),
            from.1 + i as f64 * step * r.sin(),
        ))
    };
    (0..n).map(|i| (p(i), p(i + 1))).collect()
}

/// A real 45-degree corner whose second leg is drawn as 40 pieces of 9 um
/// (0.36 mm, far wider than any track's end cap): the run is real copper, so
/// the corner is an acid trap, with or without a long track after the run.
/// The tail turns 45 degrees to +y: read as one point, the run would leave
/// the first leg and the tail at a right angle and hide the corner.
#[test]
fn a_corner_drawn_as_a_run_of_short_pieces_is_an_acid_trap() {
    let run = chain(JN, 135., 0.009, 40);
    let end = run[39].1;
    let mut with_tail = vec![(JN, at(180., 3.))];
    with_tail.extend(run.iter().copied());
    with_tail.push((end, (end.0, end.1 + 3.)));
    let (traps, cov) = acid_traps(&with_tail);
    assert_eq!(traps, 1, "with a tail: {cov:?}");
    let mut alone = vec![(JN, at(180., 3.))];
    alone.extend(run);
    let (traps, cov) = acid_traps(&alone);
    assert_eq!(traps, 1, "the run alone: {cov:?}");
}

/// A 90-degree fillet of 0.25 mm radius drawn in 1-degree pieces (4.4 um)
/// between two straight legs: every vertex turns by one degree. No acid trap.
#[test]
fn a_fillet_drawn_as_short_pieces_is_not_an_acid_trap() {
    assert_eq!(acid_traps(&fillet_legs(0.25)).0, 0);
}

/// Leg A runs left from JN; a fillet of radius `r` turns from heading +x to
/// +y; leg B leaves its far end heading +y.
fn fillet_legs(r: f64) -> Vec<Track> {
    let c = (JN.0, JN.1 + r);
    let p = |deg: f64| {
        let t = deg.to_radians();
        on_nm((c.0 + r * t.sin(), c.1 - r * t.cos()))
    };
    let mut t = vec![(JN, at(180., 3.))];
    t.extend((0..90).map(|i| (p(i as f64), p(i as f64 + 1.))));
    let f = p(90.);
    t.push((f, (f.0, f.1 + 3.)));
    t
}

/// Short pieces collapse into one junction only while every key of the run
/// lies within a 20-um box diagonal. Keys 0, 9, 18 and 20 um along +x collapse:
/// the legs (45 and 225 degrees) are a straight line. At 21 um the run is read
/// piece by piece: its first and last pieces each meet a leg at 45 degrees.
#[test]
fn twenty_micrometres_is_where_a_run_of_short_pieces_stops_being_a_point() {
    for (last, expected) in [(0.020, 0), (0.021, 2)] {
        let k = |x: f64| (JN.0 + x, JN.1);
        let end = k(last);
        let t = vec![
            (JN, at(45., 3.)),
            (k(0.), k(0.009)),
            (k(0.009), k(0.018)),
            (k(0.018), end),
            (end, on_nm((end.0 - 2.12132, end.1 - 2.12132))),
        ];
        let (traps, cov) = acid_traps(&t);
        assert_eq!(traps, expected, "run to {last} mm: {cov:?}");
    }
}

/// Exactly 10 um is a leg and 1 nm less is short, decided in whole
/// nanometres: at any position, along an axis or a diagonal. Each piece
/// leaves JN between 90 and 143 degrees, so as a leg it makes an acute corner
/// with one of the bend's legs (180 and 45 degrees).
#[test]
fn ten_micrometres_is_decided_in_whole_nanometres_at_any_position() {
    for origin in [JN, (123.456789, 45.678901), (0.000001, 299.999999)] {
        let legs_at = |deg: f64| {
            let r = deg.to_radians();
            (
                origin,
                on_nm((origin.0 + 3. * r.cos(), origin.1 + 3. * r.sin())),
            )
        };
        for (dx, dy, expected) in [
            (0, 10_000, 1),
            (0, 9_999, 0),
            (-6_000, 8_000, 1),
            (-6_000, 7_999, 0),
            (-8_000, 6_000, 1),
            (-7_999, 6_000, 0),
        ] {
            let tip = on_nm((origin.0 + dx as f64 * 1e-6, origin.1 + dy as f64 * 1e-6));
            let t = [legs_at(180.), legs_at(45.), (origin, tip)];
            let (traps, cov) = acid_traps(&t);
            assert_eq!(traps, expected, "({dx}, {dy}) nm at {origin:?}: {cov:?}");
        }
    }
}

/// A short track of another net, or on another layer, at the same point is
/// neither joined to the junction nor read as part of it.
#[test]
fn a_short_track_of_another_net_or_layer_is_not_part_of_the_junction() {
    let board = |extra: &str| {
        let text = trap_board(&legs(&[180., 45.]));
        text.replace("\n)", &format!("\n  {extra}\n)"))
    };
    for extra in [
        format!("(segment (start {} {}) (end {} {}) (width 0.2) (layer \"F.Cu\") (net \"OTHER\") (uuid \"o\"))", JN.0, JN.1, JN.0, JN.1 + 0.005),
        format!("(segment (start {} {}) (end {} {}) (width 0.2) (layer \"B.Cu\") (net \"SIG\") (uuid \"b\"))", JN.0, JN.1, JN.0, JN.1 + 0.005),
    ] {
        let report = check_board(&geometry(&board(&extra)), &jlc());
        assert_eq!(issues_with(&report, "Acid trap").len(), 0, "{extra}");
        assert_eq!(report.checks["acid_trap"].checked, 1, "{extra}");
    }
}

/// Four legs at the 89-degree line: 0, 88.99, 180 and 270 degrees hold one
/// acute pair; at 89.01 none.
#[test]
fn a_four_way_junction_is_acute_only_below_eighty_nine_degrees() {
    assert_eq!(acid_traps(&legs(&[0., 88.99, 180., 270.])).0, 1);
    assert_eq!(acid_traps(&legs(&[0., 89.01, 180., 270.])).0, 0);
}

/// A junction is reported at its key with the most track ends: two legs at JN
/// and one more 2 um away, joined by a short piece.
#[test]
fn an_acid_trap_is_reported_at_the_key_with_the_most_ends() {
    let j1 = (JN.0 + 0.002, JN.1);
    let t = vec![
        (JN, at(180., 3.)),
        (JN, at(135., 3.)),
        (JN, j1),
        (j1, (j1.0 + 3., j1.1)),
    ];
    let report = check_board(&geometry(&trap_board(&t)), &jlc());
    let traps = issues_with(&report, "Acid trap");
    assert_eq!(traps.len(), 1);
    assert_eq!(
        (traps[0].at["x"].as_f64(), traps[0].at["y"].as_f64()),
        (Some(JN.0), Some(JN.1))
    );
}

/// The issue text never rounds a trap up to the line: 88.99 degrees reads as
/// 88, not 89.
#[test]
fn an_acid_trap_just_below_the_line_reads_below_it() {
    let report = check_board(&geometry(&trap_board(&legs(&[0., 88.99]))), &jlc());
    let traps = issues_with(&report, "Acid trap");
    assert_eq!(traps.len(), 1);
    assert!(traps[0].issue.contains("meet at 88°"), "{}", traps[0].issue);
}

/// The reader's two edges: a piece of one point is no track, and no keys at
/// all are a small run.
#[test]
fn the_junction_reader_skips_a_one_point_piece_and_reads_no_keys_as_small() {
    let one = [(1.0, 1.0)];
    let two = [(1.0, 1.0), (2.0, 1.0)];
    let pieces = [(&one[..], "a"), (&two[..], "b"), (&two[..], "c")].map(|(pts, id)| Piece {
        layer: "F.Cu",
        net: "SIG",
        pts,
        id: Some(id),
    });
    let found = read_junctions(pieces.into_iter());
    assert!(found
        .iter()
        .all(|j| j.ends.iter().all(|e| e.1.as_deref() != Some("a"))));
    assert!(cluster_is_small(std::iter::empty()));
}

#[test]
fn an_arc_is_short_by_the_sum_of_its_pieces() {
    // No point or one point is no track, so not a short one.
    assert!(!is_short_path(&[]));
    assert!(!is_short_path(&[(1.0, 1.0)]));
    // Pieces of 4 and 4 um make 8: short. 6 and 6 make 12: a leg, though each
    // piece alone is short. 5 and 5 make exactly 10: a leg.
    assert!(is_short_path(&[(1.0, 1.0), (1.004, 1.0), (1.004, 1.004)]));
    assert!(!is_short_path(&[(1.0, 1.0), (1.006, 1.0), (1.006, 1.006)]));
    assert!(!is_short_path(&[(1.0, 1.0), (1.005, 1.0), (1.005, 1.005)]));
    assert!(is_short_path(&[
        (1.0, 1.0),
        (1.005, 1.0),
        (1.005, 1.004_999)
    ]));
}

/// The run bound is a box diagonal: along either axis, on a diagonal and
/// for keys that are not in a line.
#[test]
fn the_run_bound_is_the_diagonal_of_the_keys_box() {
    for (keys, small) in [
        (vec![(0, 0), (20, 0)], true),
        (vec![(0, 0), (21, 0)], false),
        (vec![(0, 0), (0, 20)], true),
        (vec![(0, 0), (0, 21)], false),
        (vec![(0, 0), (14, 14)], true),
        (vec![(0, 0), (15, 15)], false),
        (vec![(5, 0), (0, 12), (-7, 3)], true),
        // A 16 by 12 box has a diagonal of exactly 20; 17 by 12 is over.
        (vec![(5, 0), (0, 12), (-11, 3)], true),
        (vec![(5, 0), (0, 12), (-12, 3)], false),
    ] {
        assert_eq!(cluster_is_small(&keys), small, "{keys:?}");
    }
}

/// A piece of no length gives no direction: an arc whose end repeats its
/// first point adds no leg there.
#[test]
fn a_piece_of_no_length_gives_no_direction() {
    let arc = [(1.0, 1.0), (1.0, 1.0), (1.5, 1.0), (1.5, 1.5)];
    let leg = [(1.0, 1.0), (1.0, 1.5)];
    let pieces = [(&arc[..], "arc"), (&leg[..], "leg")].map(|(pts, id)| Piece {
        layer: "F.Cu",
        net: "SIG",
        pts,
        id: Some(id),
    });
    let found = read_junctions(pieces.into_iter());
    let ends: Vec<_> = found.iter().map(|j| j.ends.clone()).collect();
    assert!(found.is_empty(), "{ends:?}");
}
