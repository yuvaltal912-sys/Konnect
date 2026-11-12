use super::*;

const BOARD: &str = r#"(kicad_pcb (version 20260206) (generator "test")
 (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (2 "B.Cu" signal) (9 "F.Mask" user))
 (footprint "Test" (uuid "fp") (at 5 5) (layer "F.Cu")
  (pad "1" thru_hole circle (uuid "p") (at 0 0) (size 2 2) (drill 1)
   (layers "*.Cu" "*.Mask") (net "SIGNAL")))
 (via (uuid "v") (at 7 7) (size 1) (drill 0.5) (layers "F.Cu" "B.Cu") (net "SIGNAL"))
 (segment (uuid "t") (start 5 5) (end 8 8) (width 0.2) (layer "F.Cu") (net "SIGNAL"))
 (zone (uuid "z") (net "GND") (layer "F.Cu") (priority 0)
  (fill yes (thermal_gap 0.3) (thermal_bridge_width 0.3))
  (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10)))
  (filled_polygon (layer "F.Cu") (pts (xy 1 1) (xy 9 1) (xy 9 9) (xy 1 9)))))"#;

fn snapshot(source: &str) -> Snapshot {
    Snapshot {
        board: source.to_owned(),
        sidecars: [None, None],
    }
}

fn report(rule: &str, description: &str, ids: &[&str]) -> DrcReport {
    DrcReport {
        violations: vec![DrcViolation {
            severity: "error".into(),
            rule: rule.into(),
            description: description.into(),
            pos: Some(cli::ReportPos { x: 5.0, y: 5.0 }),
            items: ids
                .iter()
                .map(|id| cli::ReportItem {
                    uuid: Some((*id).into()),
                    description: "Do not trust me for type or net".into(),
                    ..Default::default()
                })
                .collect(),
        }],
        unconnected_items: Some(vec![]),
        schematic_parity: None,
        ownership_diagnostic: None,
        schematic_parity_diagnostic: None,
    }
}

const ZERO: &str = "Clearance violation (required 0.2000 mm; actual 0.0000 mm)";

fn inspect(source: &str, report: &DrcReport) -> Value {
    diagnostic(
        &Inspection::new(report, Ok(&snapshot(source)), "saved_fills"),
        None,
        Err("not_compared"),
    )
}

#[test]
fn recognizes_structural_pad_via_pairs_in_either_order_and_ignores_prose_identity() {
    // Removing UUID/object/net lookup or assuming item order loses these cases.
    for ids in [["z", "p"], ["p", "z"], ["z", "v"], ["v", "z"]] {
        for rule in ["clearance", "hole_clearance", "shorting_items"] {
            let value = inspect(
                BOARD,
                &report(
                    rule,
                    if rule == "shorting_items" {
                        "short"
                    } else {
                        ZERO
                    },
                    &ids,
                ),
            );
            assert_eq!(value["status"], "suspected", "{ids:?}/{rule}: {value}");
            assert_eq!(value["primary_report"]["matching_entries"], 1);
            assert_eq!(value["primary_report"]["distinct_pairs"], 1);
        }
    }
}

#[test]
fn actual_distance_recognition_does_not_read_required_clearance_or_round_tiny_values() {
    // A float epsilon/underflow or a substring search for zero would misclassify.
    for (description, status) in [
        (ZERO, "suspected"),
        (
            "Clearance (required 0.0000 mm; actual 0.15 mm)",
            "none_detected",
        ),
        (
            "Clearance (required 0.2 mm; actual 0.0000000000000000001 mm)",
            "none_detected",
        ),
        ("Clearance (required 0.2 mm; actual 0.0 mils)", "suspected"),
        ("Clearance (required 0.2 mm; actual 0 in)", "suspected"),
        (
            "Clearance (required 0.2 mm; actual 0.1 in)",
            "none_detected",
        ),
        (
            "Clearance (required 0.2 mm; actual 1e-999 mm)",
            "incomplete",
        ),
        (
            "Clearance (required 0.2 mm; actual 0 unknown)",
            "incomplete",
        ),
        ("Clearance (required 0.0 mm)", "incomplete"),
        ("Clearance (required 0.2 mm; actual NaN mm)", "incomplete"),
        ("Clearance (required 0.2 mm; actual 0.0.0 mm)", "incomplete"),
        ("Clearance (required 0.2 mm; actual .0 mm)", "incomplete"),
        ("Clearance (required 0.2 mm; actual 0. mm)", "incomplete"),
        (
            "Clearance (required 0.2 mm; actual 0.0000 mm) extra",
            "incomplete",
        ),
        (
            "Abstand (erforderlich 0.2 mm; tatsächlich 0.0000 mm)",
            "incomplete",
        ),
    ] {
        assert_eq!(
            inspect(BOARD, &report("clearance", description, &["z", "p"]))["status"],
            status,
            "{description}"
        );
    }
}

#[test]
fn unrelated_rules_items_same_nets_and_separate_copper_layers_do_not_qualify() {
    for rule in [
        "silk_overlap",
        "solder_mask_bridge",
        "unconnected_items",
        "copper_edge_clearance",
    ] {
        assert_eq!(
            inspect(BOARD, &report(rule, ZERO, &["z", "p"]))["status"],
            "none_detected"
        );
    }
    for ids in [["t", "p"], ["z", "t"], ["z", "z"]] {
        assert_eq!(
            inspect(BOARD, &report("clearance", ZERO, &ids))["status"],
            "none_detected"
        );
    }
    let same = BOARD.replace("(net \"SIGNAL\")", "(net \"GND\")");
    assert_eq!(
        inspect(&same, &report("clearance", ZERO, &["z", "p"]))["status"],
        "none_detected"
    );
    let separate = BOARD.replace(
        "(layers \"*.Cu\" \"*.Mask\")",
        "(layers \"B.Cu\" \"B.Mask\")",
    );
    assert_eq!(
        inspect(&separate, &report("clearance", ZERO, &["z", "p"]))["status"],
        "none_detected"
    );
    let inner = BOARD.replace(
        "(zone (uuid \"z\") (net \"GND\") (layer \"F.Cu\")",
        "(zone (uuid \"z\") (net \"GND\") (layer \"In1.Cu\")",
    );
    assert_eq!(
        inspect(&inner, &report("clearance", ZERO, &["z", "v"]))["status"],
        "suspected",
        "through-via includes inner layer"
    );
}

#[test]
fn unresolved_identity_or_net_evidence_cannot_become_a_known_different_net() {
    let r = report("clearance", ZERO, &["z", "p"]);
    for source in [
        BOARD.replace("(uuid \"p\")", ""),
        BOARD.replace("(uuid \"p\")", "(uuid \"p\") (uuid \"p\")"),
        BOARD.replace("(uuid \"v\")", "(uuid \"p\")"),
        BOARD.replace("(uuid \"p\")", "(uuid \"p\") (tstamp \"other\")"),
        BOARD.replace("(net \"SIGNAL\")", "(net 7)"),
        BOARD.replace("(net \"SIGNAL\")", "(net 0)"),
        BOARD.replace("(net \"SIGNAL\")", "(net \"\")"),
        BOARD.replace("(net \"SIGNAL\")", "(net \"SIGNAL\" extra)"),
        BOARD.replace("(net \"SIGNAL\")", "(net \"SIGNAL\") (net \"OTHER\")"),
        BOARD.replace("(net \"GND\")", "(net \"GND\") (net_name \"OTHER\")"),
        BOARD.replace("(kicad_pcb", "(not_a_board"),
    ] {
        let result = inspect(&source, &r);
        assert_eq!(result["status"], "incomplete", "{source}: {result}");
        assert!(result["newly_reported_after_refill"].is_null());
    }
    for ids in [vec!["z"], vec!["z", "missing"], vec!["z", "p", "v"]] {
        assert_eq!(
            inspect(BOARD, &report("clearance", ZERO, &ids))["status"],
            "incomplete"
        );
    }
}

#[test]
fn supports_legacy_net_table_and_current_numeric_looking_names_without_guessing() {
    let legacy = BOARD
        .replace(
            "(generator \"test\")",
            "(generator \"test\") (net 1 \"GND\") (net 2 \"SIGNAL\")",
        )
        .replace("(net \"GND\")", "(net 1) (net_name \"GND\")")
        .replace("(net \"SIGNAL\")", "(net 2 \"SIGNAL\")");
    let r = report("clearance", ZERO, &["z", "p"]);
    assert_eq!(inspect(&legacy, &r)["status"], "suspected");
    assert_eq!(
        inspect(
            &legacy.replace("(net 2 \"SIGNAL\")", "(net 2 \"SIGNAL\") (net 2 \"OTHER\")"),
            &r
        )["status"],
        "incomplete"
    );
    assert_eq!(
        inspect(
            &legacy.replace("(net 1) (net_name \"GND\")", "(net 1) (net_name \"OTHER\")"),
            &r
        )["status"],
        "incomplete"
    );
    let numeric_names = BOARD
        .replace("\"GND\"", "\"0\"")
        .replace("\"SIGNAL\"", "\"0.0\"");
    assert_eq!(inspect(&numeric_names, &r)["status"], "suspected");
}

#[test]
fn duplicate_root_net_declarations_are_ambiguous_even_with_unique_item_fields() {
    // Isolate the table guard: every item has exactly one numeric net reference.
    for repeated_name in ["SIGNAL", "OTHER"] {
        let source = BOARD
            .replace(
                "(generator \"test\")",
                &format!(
            "(generator \"test\") (net 1 \"GND\") (net 2 \"SIGNAL\") (net 2 \"{repeated_name}\")"),
            )
            .replace("(net \"GND\")", "(net 1)")
            .replace("(net \"SIGNAL\")", "(net 2)");
        let result = inspect(&source, &report("clearance", ZERO, &["z", "p"]));
        assert_eq!(result["status"], "incomplete");
        assert_eq!(
            result["primary_report"]["unresolved_reasons"]["net_identity_unresolved"],
            1
        );
    }
}

#[test]
fn uncertain_copper_membership_or_mechanical_holes_are_not_zone_defect_evidence() {
    let r = report("hole_clearance", ZERO, &["z", "p"]);
    for source in [
        BOARD.replace("thru_hole", "np_thru_hole"),
        BOARD.replace("(drill 1)", "(drill 0)"),
        BOARD.replace("(drill 1)", ""),
        BOARD.replace("(layers \"*.Cu\" \"*.Mask\")", "(layers \"Unknown.Cu\")"),
        BOARD.replace("(layers \"*.Cu\" \"*.Mask\")", "(layers \"F.Cu\" \"F.Cu\")"),
        BOARD.replace("(uuid \"p\")", "(uuid \"p\") (padstack custom)"),
        BOARD.replace("(uuid \"z\")", "(uuid \"z\") (keepout (pads not_allowed))"),
        BOARD.replace("(2 \"B.Cu\" signal)", "(0 \"B.Cu\" signal)"),
    ] {
        assert_eq!(inspect(&source, &r)["status"], "incomplete", "{source}");
    }
    let blind = BOARD.replace("(layers \"F.Cu\" \"B.Cu\")", "(layers \"In1.Cu\" \"B.Cu\")");
    assert_eq!(
        inspect(&blind, &report("clearance", ZERO, &["z", "v"]))["status"],
        "none_detected"
    );
}

#[test]
fn counts_whole_reports_and_preserves_valid_findings_with_partial_coverage() {
    let mut r = report("clearance", ZERO, &["z", "p"]);
    let entry = r.violations[0].clone();
    r.violations = vec![entry; 11];
    r.violations
        .extend(report("hole_clearance", ZERO, &["z", "p"]).violations);
    r.violations
        .extend(report("clearance", ZERO, &["z", "missing"]).violations);
    let value = inspect(BOARD, &r);
    assert_eq!(value["status"], "suspected");
    assert_eq!(value["primary_report"]["coverage"], "partial");
    assert_eq!(value["primary_report"]["matching_entries"], 12);
    assert_eq!(value["primary_report"]["distinct_pairs"], 1);
    assert_eq!(value["primary_report"]["unresolved_candidates"], 1);
    assert_eq!(value["findings"].as_array().unwrap().len(), 10);
    assert_eq!(value["truncated"], true);
    let long = "a".repeat(80_000);
    let value = inspect(
        &BOARD.replace("\"p\"", &format!("\"{long}\"")),
        &report("clearance", ZERO, &["z", &long]),
    );
    assert_eq!(value["status"], "suspected");
    assert_eq!(value["findings"][0]["identifiers_truncated"], true);
    assert!(value.to_string().len() < 4000);
}

#[test]
fn example_budget_limits_encoded_bytes_even_for_long_unicode_identifiers() {
    let zone = format!("z{}", "😀".repeat(300));
    let pad = format!("p{}", "😀".repeat(300));
    let source = BOARD
        .replace("\"z\"", &format!("\"{zone}\""))
        .replace("\"p\"", &format!("\"{pad}\""));
    let mut r = report("clearance", ZERO, &[&zone, &pad]);
    r.violations = vec![r.violations[0].clone(); 11];
    let value = inspect(&source, &r);
    assert_eq!(value["primary_report"]["matching_entries"], 11);
    assert_eq!(value["truncated"], true);
    assert!(
        value.to_string().len() <= 6000,
        "encoded bytes: {}",
        value.to_string().len()
    );
}

#[tokio::test]
async fn a_source_change_cannot_lend_a_stale_identity_to_the_primary_report() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("test.kicad_pcb");
    std::fs::write(&board, BOARD).unwrap();
    let before = Snapshot::capture(&board).await;
    std::fs::write(&board, BOARD.replace("\"SIGNAL\"", "\"GND\"")).unwrap();
    let result = finish(
        before,
        &board,
        &report("clearance", ZERO, &["z", "p"]),
        "saved_fills",
        None,
    )
    .await;
    assert_eq!(result["status"], "incomplete");
    assert!(result["primary_report"]["matching_entries"].is_null());
    assert_eq!(
        result["primary_report"]["reason"],
        "source_changed_or_unavailable"
    );
}

#[tokio::test]
async fn sidecar_absence_is_not_the_same_as_empty_rules() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("test.kicad_pcb");
    std::fs::write(&board, BOARD).unwrap();
    let before = Snapshot::capture(&board).await;
    std::fs::write(board.with_extension("kicad_dru"), "").unwrap();
    let result = finish(
        before,
        &board,
        &report("shorting_items", "short", &["z", "p"]),
        "kicad_cli_refill",
        None,
    )
    .await;
    assert_eq!(result["status"], "incomplete");
    assert_eq!(result["primary_report"]["provenance"], "kicad_cli_refill");
}

#[tokio::test]
async fn initial_rule_read_failure_is_preserved_when_the_source_later_becomes_readable() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("test.kicad_pcb");
    std::fs::write(&board, BOARD).unwrap();
    let project = board.with_extension("kicad_pro");
    std::fs::create_dir(&project).unwrap();
    let initial = Snapshot::capture(&board).await;
    assert!(initial.is_err());
    // A later successful read cannot establish the unavailable earlier input
    // or erase why the original snapshot could not be captured.
    std::fs::remove_dir(&project).unwrap();
    let result = finish(
        initial,
        &board,
        &report("clearance", ZERO, &["z", "p"]),
        "saved_fills",
        None,
    )
    .await;
    assert_eq!(result["status"], "incomplete");
    assert_eq!(result["primary_report"]["coverage"], "unavailable");
    assert!(result["primary_report"]["matching_entries"].is_null());
    assert_eq!(
        result["primary_report"]["reason"],
        "project/rule snapshot unavailable"
    );
}

#[test]
fn serialized_keep_all_pad_and_via_copper_is_supported() {
    let source = BOARD
        .replace(
            "(uuid \"p\")",
            "(uuid \"p\") (remove_unused_layers no) (keep_end_layers no)",
        )
        .replace(
            "(uuid \"v\")",
            "(uuid \"v\") (remove_unused_layers no) (keep_end_layers no)",
        );
    for other in ["p", "v"] {
        assert_eq!(
            inspect(&source, &report("clearance", ZERO, &["z", other]))["status"],
            "suspected"
        );
    }
}

fn empty_report() -> DrcReport {
    let mut r = report("clearance", ZERO, &["z", "p"]);
    r.violations.clear();
    r
}

fn changed_fill() -> String {
    BOARD.replace("(xy 9 9)", "(xy 8 9)")
}

fn comparison_result(before: &str, after: &str, old: &DrcReport, new: &DrcReport) -> Value {
    let before = snapshot(before);
    let evidence = Comparison {
        before: Ok(before.clone()),
        after: Ok(snapshot(after)),
        report: new.clone(),
    };
    let primary = Inspection::new(old, Ok(&before), "saved_fills");
    let compared = Inspection::new(
        new,
        evidence.after.as_ref().map_err(String::as_str),
        "private_copy_refill",
    );
    let attribution = attribute(Ok(&before), &primary, &compared, &evidence);
    diagnostic(&primary, Some(&compared), attribution)
}

#[test]
fn a_new_pair_requires_a_complete_comparison_and_changed_implicated_fill() {
    let new = report("clearance", ZERO, &["z", "p"]);
    let value = comparison_result(BOARD, &changed_fill(), &empty_report(), &new);
    assert_eq!(value["newly_reported_after_refill"], 1);
    assert_eq!(
        value["findings"][0]["refill_attribution"],
        "newly_reported_after_refill"
    );
    assert_eq!(
        value["comparison_report"]["provenance"],
        "private_copy_refill"
    );
    let old = report(
        "clearance",
        "Clearance (required 0.2 mm; actual 0.15 mm)",
        &["z", "p"],
    );
    assert_eq!(
        comparison_result(BOARD, &changed_fill(), &old, &new)["newly_reported_after_refill"],
        1
    );
    let unchanged = comparison_result(BOARD, BOARD, &empty_report(), &new);
    assert!(unchanged["newly_reported_after_refill"].is_null());
    assert_eq!(
        unchanged["attribution_reason"],
        "implicated_fill_change_unproven"
    );
    assert_eq!(unchanged["status"], "suspected");
}

#[test]
fn existing_pairs_do_not_become_new_when_rules_prose_order_or_counts_change() {
    let old = report("clearance", ZERO, &["z", "p"]);
    let mut new = report("shorting_items", "completely different prose", &["p", "z"]);
    new.violations[0].severity = "warning".into();
    new.violations[0].pos = Some(cli::ReportPos { x: 99.0, y: 101.0 });
    new.violations.push(new.violations[0].clone());
    let result = comparison_result(BOARD, &changed_fill(), &old, &new);
    assert_eq!(result["newly_reported_after_refill"], 0);
    assert_eq!(result["comparison_report"]["matching_entries"], 2);
    assert_eq!(result["comparison_report"]["distinct_pairs"], 1);
    assert_eq!(
        comparison_result(BOARD, &changed_fill(), &old, &empty_report())
            ["newly_reported_after_refill"],
        0
    );
}

#[test]
fn changed_design_inputs_cannot_be_reported_as_a_refill_introduction() {
    let new = report("clearance", ZERO, &["z", "p"]);
    for after in [
        changed_fill().replace("(priority 0)", "(priority 1)"),
        changed_fill().replace("(thermal_gap 0.3)", "(thermal_gap 0.4)"),
        changed_fill().replace("(at 5 5)", "(at 5.000001 5)"),
        changed_fill().replace("(at 5 5)", "(at 5.00000000001 5)"),
        changed_fill().replace("(net \"SIGNAL\")", "(net \"OTHER\")"),
        changed_fill().replace("(priority 0)", "(priority 0) (unknown_option 5)"),
        changed_fill().replace("(version 20260206)", "(version 20270206)"),
        changed_fill().replace("(xy 10 0) (xy 10 10)", "(xy 10 10) (xy 10 0)"),
    ] {
        let value = comparison_result(BOARD, &after, &empty_report(), &new);
        assert_eq!(value["status"], "suspected");
        assert!(
            value["newly_reported_after_refill"].is_null(),
            "{after}: {value}"
        );
        assert_eq!(value["attribution_reason"], "non_fill_design_changed");
    }
    let named_before = BOARD.replace("\"SIGNAL\"", "\"1\"");
    let named_after = changed_fill().replace("\"SIGNAL\"", "\"1.0\"");
    assert_eq!(
        comparison_result(&named_before, &named_after, &empty_report(), &new)["attribution_reason"],
        "non_fill_design_changed"
    );
}

#[test]
fn only_supported_metadata_order_and_numeric_spelling_are_normalized() {
    let after = changed_fill()
        .replace(
            "(generator \"test\")",
            "(generator \"pcbnew\") (generator_version \"10.0\")",
        )
        .replace("(at 5 5)", "(at 5.0000 05.0)")
        .replace("(fill yes", "(fill no")
        .replace(
            "(net \"GND\") (layer \"F.Cu\") (priority 0)",
            "(priority 0.000) (layer \"F.Cu\") (net \"GND\")",
        );
    assert_eq!(
        comparison_result(
            BOARD,
            &after,
            &empty_report(),
            &report("clearance", ZERO, &["z", "p"])
        )["newly_reported_after_refill"],
        1
    );
    let mut tree = konnect_sexp::parse_sexp(BOARD).unwrap();
    let original = tree.clone();
    let SexpNode::List(ref mut children) = tree else {
        unreachable!()
    };
    let f = children
        .iter()
        .position(|n| n.head() == Some("footprint"))
        .unwrap();
    let v = children
        .iter()
        .position(|n| n.head() == Some("via"))
        .unwrap();
    children.swap(f, v);
    assert_eq!(normalize(&tree).unwrap(), normalize(&original).unwrap());
    assert_ne!(
        normalize(&konnect_sexp::parse_sexp("(unknown (at 01 0))").unwrap()).unwrap(),
        normalize(&konnect_sexp::parse_sexp("(unknown (at 1 0))").unwrap()).unwrap()
    );
}

#[test]
fn vertex_order_alone_does_not_prove_an_implicated_fill_change() {
    let after = BOARD.replace(
        "(xy 1 1) (xy 9 1) (xy 9 9) (xy 1 9)",
        "(xy 9 9) (xy 9 1) (xy 1 1) (xy 1 9)",
    );
    let result = comparison_result(
        BOARD,
        &after,
        &empty_report(),
        &report("clearance", ZERO, &["z", "p"]),
    );
    assert_eq!(
        result["attribution_reason"],
        "implicated_fill_change_unproven"
    );
}

#[test]
fn missing_or_changed_comparison_inputs_leave_valid_observations_visible() {
    let source = snapshot(BOARD);
    let r = report("clearance", ZERO, &["z", "p"]);
    let primary = Inspection::new(&r, Ok(&source), "saved_fills");
    let compared = Inspection::new(&r, Ok(&snapshot(&changed_fill())), "private_copy_refill");
    let mut evidence = Comparison {
        before: Ok(source.clone()),
        after: Ok(snapshot(&changed_fill())),
        report: r.clone(),
    };
    evidence.before.as_mut().unwrap().board = changed_fill();
    assert_eq!(
        attribute(Ok(&source), &primary, &compared, &evidence).unwrap_err(),
        "comparison_input_differs"
    );
    evidence.before = Ok(source.clone());
    evidence.after.as_mut().unwrap().sidecars[1] = Some(vec![]);
    assert_eq!(
        attribute(Ok(&source), &primary, &compared, &evidence).unwrap_err(),
        "project_or_rules_changed"
    );
    evidence.after = Err("unavailable".into());
    let missing = Inspection::new(&r, Err("unavailable"), "private_copy_refill");
    let result = diagnostic(
        &primary,
        Some(&missing),
        attribute(Ok(&source), &primary, &missing, &evidence),
    );
    assert_eq!(result["status"], "suspected");
    assert_eq!(result["primary_report"]["matching_entries"], 1);
    assert!(result["comparison_report"]["matching_entries"].is_null());
    assert!(result["newly_reported_after_refill"].is_null());
}

#[test]
fn ambiguous_previous_evidence_and_footprint_zones_prevent_attribution() {
    let new = report("clearance", ZERO, &["z", "p"]);
    let old = report("clearance", ZERO, &["z", "missing"]);
    assert_eq!(
        comparison_result(BOARD, &changed_fill(), &old, &new)["attribution_reason"],
        "incomplete_report_evidence"
    );
    for id in ["", "(uuid \"footprint-zone\")"] {
        let before = BOARD.replace(
            "(pad \"1\"",
            &format!("(zone {id} (net \"GND\") (layer \"F.Cu\")) (pad \"1\""),
        );
        let after = before.replace("(xy 9 9)", "(xy 8 9)");
        assert_eq!(
            comparison_result(&before, &after, &empty_report(), &new)["attribution_reason"],
            "footprint_zone_comparison_unsupported"
        );
    }
}

#[test]
fn enabled_ambiguous_or_malformed_copper_removal_is_not_assumed_keep_all() {
    for flag in [
        "(remove_unused_layers yes)",
        "(remove_unused_layers maybe)",
        "(remove_unused_layers no) (remove_unused_layers no)",
        "(remove_unused_layers no extra)",
        "(keep_end_layers yes)",
    ] {
        let source = BOARD.replace("(uuid \"p\")", &format!("(uuid \"p\") {flag}"));
        assert_eq!(
            inspect(&source, &report("clearance", ZERO, &["z", "p"]))["status"],
            "incomplete"
        );
    }
}

#[test]
fn embedded_lookalike_board_objects_do_not_resolve_as_real_copper() {
    let source = BOARD
        .replace(
            "(footprint \"Test\"",
            "(unknown_container (footprint \"Test\"",
        )
        .replace("(net \"SIGNAL\")))", "(net \"SIGNAL\"))))");
    assert_eq!(
        inspect(&source, &report("clearance", ZERO, &["z", "p"]))["status"],
        "incomplete"
    );
}

/// Review A F7: a retained raw KiCad report goes through the production
/// parser and ownership enrichment (`cli::run_drc`, or
/// `cli::refill_drc_on_private_copy` for a comparison report). The stand-in
/// CLI only copies the report: no KiCad starts, and nothing is written beside
/// the retained inputs, because the board is copied into a scratch folder.
async fn production_report(raw: &str, board: &str, comparison: bool) -> DrcReport {
    let dir = tempfile::tempdir().unwrap();
    let raw = std::fs::canonicalize(raw).unwrap();
    let script = format!(
        "#!/bin/sh\nout=''; prev=''\nfor a; do\n  [ \"$prev\" = --output ] && out=\"$a\"\n  prev=\"$a\"\ndone\ncp '{}' \"$out\"\n",
        raw.display()
    );
    let cli = cli::test_support::write_script(dir.path(), "kicad-cli", &script, "")
        .display()
        .to_string();
    let copy = dir.path().join("replay.kicad_pcb");
    std::fs::copy(board, &copy).unwrap();
    if comparison {
        cli::refill_drc_on_private_copy(&cli, &copy).await.unwrap()
    } else {
        cli::run_drc(&cli, &copy, false).await.unwrap()
    }
}

#[tokio::test]
#[ignore = "read-only archived JSON/board replay; requires explicit S11_REPLAY_* paths"]
async fn replay_retained_reports_without_starting_native_tools() {
    let board = std::env::var("S11_REPLAY_BOARD").unwrap();
    let source = std::fs::read_to_string(&board).unwrap();
    let report =
        production_report(&std::env::var("S11_REPLAY_REPORT").unwrap(), &board, false).await;
    let result = inspect(&source, &report);
    let entries: u64 = std::env::var("S11_REPLAY_EXPECT_ENTRIES")
        .unwrap()
        .parse()
        .unwrap();
    let pairs: u64 = std::env::var("S11_REPLAY_EXPECT_PAIRS")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(result["primary_report"]["matching_entries"], entries);
    assert_eq!(result["primary_report"]["distinct_pairs"], pairs);
    println!(
        "matching_entries={entries}; distinct_pairs={pairs}; unresolved_candidates={}",
        result["primary_report"]["unresolved_candidates"]
    );
}

#[tokio::test]
#[ignore = "read-only matched native artifacts; explicit S11_MATCHED_* paths required"]
async fn replay_matched_native_refill_without_starting_native_tools() {
    let var = |name: &str| std::env::var(name).unwrap();
    let source = std::path::PathBuf::from(var("S11_MATCHED_SOURCE"));
    let copy_before = std::path::PathBuf::from(var("S11_MATCHED_BEFORE"));
    let after = std::path::PathBuf::from(var("S11_MATCHED_AFTER"));
    let source_snapshot = Snapshot::capture(&source).await.unwrap();
    let before_snapshot = Snapshot::capture(&copy_before).await.unwrap();
    assert!(
        source_snapshot == before_snapshot,
        "original and copied complete inputs differ"
    );
    let primary = production_report(
        &var("S11_MATCHED_PRIMARY_REPORT"),
        &var("S11_MATCHED_SOURCE"),
        false,
    )
    .await;
    let compared = production_report(
        &var("S11_MATCHED_COMPARISON_REPORT"),
        &var("S11_MATCHED_BEFORE"),
        true,
    )
    .await;
    let comparison = Comparison {
        before: Ok(before_snapshot),
        after: Snapshot::capture(&after).await,
        report: compared,
    };
    let result = finish(
        Ok(source_snapshot),
        &source,
        &primary,
        "saved_fills",
        Some(Ok(comparison)),
    )
    .await;
    assert_eq!(
        result["primary_report"]["matching_entries"], 0,
        "not a candidate-free starting snapshot"
    );
    if let Some(new) = result["newly_reported_after_refill"].as_u64() {
        assert_eq!(result["comparison_report"]["matching_entries"], new);
    }
    std::fs::write(
        var("S11_MATCHED_OUTPUT"),
        serde_json::to_string_pretty(&result).unwrap(),
    )
    .unwrap();
    println!(
        "primary_entries={}; comparison_entries={}; newly_reported={}; attribution_reason={}",
        result["primary_report"]["matching_entries"],
        result["comparison_report"]["matching_entries"],
        result["newly_reported_after_refill"],
        result["attribution_reason"]
    );
}

// ---- Independent review A (2026-11-12): reproduction and guard tests ----

fn many(rule: &str, description: &str, ids: &[&str], n: usize) -> DrcReport {
    let mut r = report(rule, description, ids);
    r.violations = vec![r.violations[0].clone(); n];
    r
}

fn with_project(source: &str, project: &str) -> Snapshot {
    Snapshot {
        board: source.to_owned(),
        sidecars: [Some(project.as_bytes().to_vec()), None],
    }
}

fn exclusions(entries: &str) -> String {
    format!(r#"{{"board":{{"design_settings":{{"drc_exclusions":[{entries}]}}}}}}"#)
}

/// A comparison whose source, copied input and refilled copy all carry the
/// same project file.
fn comparison_with_project(project: &str, old: &DrcReport, new: &DrcReport) -> Value {
    let before = with_project(BOARD, project);
    let evidence = Comparison {
        before: Ok(before.clone()),
        after: Ok(with_project(&changed_fill(), project)),
        report: new.clone(),
    };
    let primary = Inspection::new(old, Ok(&before), "saved_fills");
    let compared = Inspection::new(
        new,
        evidence.after.as_ref().map_err(String::as_str),
        "private_copy_refill",
    );
    let attribution = attribute(Ok(&before), &primary, &compared, &evidence);
    diagnostic(&primary, Some(&compared), attribution)
}

#[test]
fn a_report_at_kicads_per_rule_limit_is_partial_never_a_checked_zero() {
    // F1. KiCad stops writing a rule at its per-run limit and the report does
    // not say so. Track/pad entries are not candidates: only the count matters.
    for (rule, limit) in [
        ("clearance", 499),
        ("hole_clearance", 199),
        ("shorting_items", 199),
    ] {
        let below = inspect(BOARD, &many(rule, ZERO, &["t", "p"], limit - 1));
        assert_eq!(below["status"], "none_detected", "{rule}: {below}");
        assert_eq!(below["primary_report"]["coverage"], "complete");
        let at = inspect(BOARD, &many(rule, ZERO, &["t", "p"], limit));
        assert_eq!(at["status"], "incomplete", "{rule}: {at}");
        assert_eq!(at["primary_report"]["coverage"], "partial");
        assert_eq!(at["primary_report"]["rules_at_kicad_limit"], json!([rule]));
        assert_eq!(
            at["primary_report"]["partial_reasons"],
            json!(["kicad_rule_limit_reached"])
        );
    }
    // A rule the detector does not read cannot hide a zone/pad entry.
    let other = inspect(BOARD, &many("silk_overlap", ZERO, &["t", "p"], 499));
    assert_eq!(other["status"], "none_detected");
    // A truncated report cannot make a comparison entry newly reported.
    let old = many("clearance", ZERO, &["t", "p"], 499);
    let mut new = many("clearance", ZERO, &["t", "p"], 10);
    new.violations
        .extend(report("clearance", ZERO, &["z", "p"]).violations);
    let value = comparison_result(BOARD, &changed_fill(), &old, &new);
    assert!(value["newly_reported_after_refill"].is_null(), "{value}");
    assert_eq!(value["attribution_reason"], "kicad_rule_limit_reached");
    assert_eq!(value["findings"][0]["refill_attribution"], "unproven");
}

#[test]
fn project_exclusions_count_toward_the_limit_and_refuse_attribution_for_their_items() {
    // F1: excluded markers use up KiCad's limit before they are filtered out.
    let snap = with_project(BOARD, &exclusions(r#"["clearance|1|2|t|p", ""]"#));
    let value = diagnostic(
        &Inspection::new(
            &many("clearance", ZERO, &["t", "p"], 498),
            Ok(&snap),
            "saved_fills",
        ),
        None,
        Err("not_compared"),
    );
    assert_eq!(value["status"], "incomplete", "{value}");
    assert_eq!(
        value["primary_report"]["rules_at_kicad_limit"],
        json!(["clearance"])
    );
    // F4: an excluded pair can resurface after refill under a moved position.
    let new = report("clearance", ZERO, &["z", "p"]);
    for entry in [
        r#""clearance|9|9|z|P""#,
        r#"["hole_clearance|1|2|p|x", "kept"]"#,
    ] {
        let value = comparison_with_project(&exclusions(entry), &empty_report(), &new);
        assert!(
            value["newly_reported_after_refill"].is_null(),
            "{entry}: {value}"
        );
        assert_eq!(value["attribution_reason"], "drc_exclusion_names_pair");
        assert_eq!(value["status"], "suspected");
    }
    let unrelated =
        comparison_with_project(&exclusions(r#""clearance|1|2|t|v""#), &empty_report(), &new);
    assert_eq!(unrelated["newly_reported_after_refill"], 1, "{unrelated}");
    let no_list = comparison_with_project("{}", &empty_report(), &new);
    assert_eq!(no_list["newly_reported_after_refill"], 1);
    let numeric_entry = exclusions("7");
    for project in [
        "not json",
        r#"{"board":{"design_settings":{"drc_exclusions":"clearance|1|2|z|p"}}}"#,
        numeric_entry.as_str(),
    ] {
        let value = comparison_with_project(project, &empty_report(), &new);
        assert_eq!(value["primary_report"]["coverage"], "partial", "{project}");
        assert_eq!(
            value["primary_report"]["partial_reasons"],
            json!(["project_exclusions_unreadable"])
        );
        assert_eq!(value["attribution_reason"], "project_exclusions_unreadable");
    }
}

#[test]
fn start_end_only_vias_do_not_claim_inner_layer_copper() {
    // F2: KiCad 10 flashes a start_end_only via on its end layers only.
    let inner = BOARD.replace(
        "(zone (uuid \"z\") (net \"GND\") (layer \"F.Cu\")",
        "(zone (uuid \"z\") (net \"GND\") (layer \"In1.Cu\")",
    );
    for (flag, status) in [
        ("", "suspected"),
        ("(start_end_only no)", "suspected"),
        ("(start_end_only yes)", "incomplete"),
        ("(start_end_only maybe)", "incomplete"),
        ("(start_end_only no) (start_end_only no)", "incomplete"),
    ] {
        let source = inner.replace("(uuid \"v\")", &format!("(uuid \"v\") {flag}"));
        let value = inspect(&source, &report("clearance", ZERO, &["z", "v"]));
        assert_eq!(value["status"], status, "{flag}: {value}");
        if status == "incomplete" {
            assert_eq!(
                value["primary_report"]["unresolved_reasons"]["copper_membership_unsupported"],
                1
            );
        }
    }
}

#[test]
fn newly_reported_examples_come_before_other_examples() {
    // F8: the capped list must show the example the user most needs.
    let old = many("clearance", ZERO, &["z", "p"], 10);
    let mut new = old.clone();
    new.violations
        .extend(report("clearance", ZERO, &["z", "v"]).violations);
    let value = comparison_result(BOARD, &changed_fill(), &old, &new);
    assert_eq!(value["newly_reported_after_refill"], 1, "{value}");
    let findings = value["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 10);
    assert_eq!(
        findings[0]["refill_attribution"],
        "newly_reported_after_refill"
    );
    assert_eq!(findings[0]["other_uuid"], "v");
    assert_eq!(findings[1]["report"], "primary");
    assert_eq!(value["truncated"], true);
}

#[test]
fn kicads_scientific_spelling_of_a_tiny_nonzero_distance_is_nonzero() {
    // F10: KiCad prints a nonzero value that would read as zeros with %.3e.
    for (actual, status) in [
        ("1.000e-06 mm", "none_detected"),
        ("-2.500e-07 mm", "none_detected"),
        ("9.999e-100 mils", "none_detected"),
        ("0.000e+00 mm", "incomplete"),
        ("1.0e-06 mm", "incomplete"),
        ("1.000e-6 mm", "incomplete"),
        ("1.000E-06 mm", "incomplete"),
        ("1.000e06 mm", "incomplete"),
        ("1.0000e-06 mm", "incomplete"),
        ("1.000e-0600 mm", "incomplete"),
    ] {
        let description = format!("Clearance (required 0.2 mm; actual {actual})");
        assert_eq!(
            inspect(BOARD, &report("clearance", &description, &["z", "p"]))["status"],
            status,
            "{actual}"
        );
    }
}

// Review A F3: each guard below has a test that fails when it is removed.

fn reason_of(value: &Value) -> &Value {
    &value["primary_report"]["unresolved_reasons"]
}

#[test]
fn guard_m1_a_legacy_inline_net_name_must_match_the_table() {
    let legacy = BOARD
        .replace(
            "(generator \"test\")",
            "(generator \"test\") (net 1 \"GND\") (net 2 \"SIGNAL\")",
        )
        .replace("(net \"GND\")", "(net 1 \"GND\")");
    let r = report("clearance", ZERO, &["z", "p"]);
    let consistent = legacy.replace("(net \"SIGNAL\")", "(net 2 \"SIGNAL\")");
    assert_eq!(inspect(&consistent, &r)["status"], "suspected");
    let value = inspect(&legacy.replace("(net \"SIGNAL\")", "(net 2 \"OTHER\")"), &r);
    assert_eq!(value["status"], "incomplete", "{value}");
    assert_eq!(reason_of(&value)["net_identity_inconsistent"], 1);
}

#[test]
fn guard_m2_only_smd_and_plated_pads_count_as_pad_copper() {
    let r = report("clearance", ZERO, &["z", "p"]);
    for kind in ["connect", "np_thru_hole"] {
        let source = BOARD
            .replace("thru_hole circle", &format!("{kind} circle"))
            .replace(
                "(layers \"*.Cu\" \"*.Mask\")",
                "(layers \"F.Cu\" \"F.Mask\")",
            );
        let value = inspect(&source, &r);
        assert_eq!(value["status"], "incomplete", "{kind}: {value}");
        assert_eq!(reason_of(&value)["pad_copper_unsupported"], 1);
    }
    let smd = BOARD.replace("thru_hole circle", "smd circle").replace(
        "(layers \"*.Cu\" \"*.Mask\")",
        "(layers \"F.Cu\" \"F.Mask\")",
    );
    assert_eq!(inspect(&smd, &r)["status"], "suspected");
}

#[test]
fn guard_m3_any_duplicate_uuid_blocks_attribution() {
    let dup = "(segment (uuid \"dup\") (start 1 1) (end 2 2) (width 0.2) (layer \"F.Cu\") (net \"SIGNAL\")) (segment (uuid \"dup\") (start 3 3) (end 4 4) (width 0.2) (layer \"F.Cu\") (net \"SIGNAL\")) (segment (uuid \"t\")";
    let before = BOARD.replace("(segment (uuid \"t\")", dup);
    let after = changed_fill().replace("(segment (uuid \"t\")", dup);
    let value = comparison_result(
        &before,
        &after,
        &empty_report(),
        &report("clearance", ZERO, &["z", "p"]),
    );
    assert_eq!(value["status"], "suspected");
    assert!(value["newly_reported_after_refill"].is_null(), "{value}");
    assert_eq!(value["attribution_reason"], "ambiguous_board_identity");
}

#[test]
fn guard_m4_a_non_numeric_root_net_fails_the_board_index() {
    let source = BOARD.replace(
        "(generator \"test\")",
        "(generator \"test\") (net abc \"GND\")",
    );
    let value = inspect(&source, &report("clearance", ZERO, &["z", "p"]));
    assert_eq!(value["status"], "incomplete", "{value}");
    assert_eq!(value["primary_report"]["reason"], "ambiguous_net_table");
}

#[test]
fn guard_m5_an_item_needs_exactly_one_layer_field() {
    let source = BOARD.replace(
        "(layer \"F.Cu\") (priority 0)",
        "(layer \"F.Cu\") (layers \"B.Cu\") (priority 0)",
    );
    let value = inspect(&source, &report("clearance", ZERO, &["z", "p"]));
    assert_eq!(value["status"], "incomplete", "{value}");
    assert_eq!(reason_of(&value)["copper_membership_unresolved"], 1);
}

#[test]
fn guard_m6_an_incomplete_comparison_report_alone_blocks_attribution() {
    let mut new = report("clearance", ZERO, &["z", "p"]);
    new.violations
        .extend(report("clearance", ZERO, &["z", "missing"]).violations);
    let value = comparison_result(BOARD, &changed_fill(), &empty_report(), &new);
    assert_eq!(value["primary_report"]["coverage"], "complete");
    assert_eq!(value["comparison_report"]["coverage"], "partial");
    assert!(value["newly_reported_after_refill"].is_null(), "{value}");
    assert_eq!(value["attribution_reason"], "incomplete_report_evidence");
}

#[test]
fn guard_m7_two_fill_flags_are_an_ambiguous_fill_state() {
    let after = changed_fill().replace("(fill yes", "(fill yes no");
    let value = comparison_result(
        BOARD,
        &after,
        &empty_report(),
        &report("clearance", ZERO, &["z", "p"]),
    );
    assert!(value["newly_reported_after_refill"].is_null(), "{value}");
    assert_eq!(value["attribution_reason"], "ambiguous_fill_state");
}

#[test]
fn guard_m9_a_whitespace_padded_uuid_does_not_resolve() {
    for id in [" p", "p "] {
        let source = BOARD.replace("(uuid \"p\")", &format!("(uuid \"{id}\")"));
        let value = inspect(&source, &report("clearance", ZERO, &["z", id]));
        assert_eq!(value["status"], "incomplete", "{id:?}: {value}");
        assert_eq!(reason_of(&value)["item_identity_unresolved"], 1);
    }
}

#[test]
fn guard_m10_a_fill_polygon_needs_three_points() {
    let after = BOARD.replace("(xy 1 1) (xy 9 1) (xy 9 9) (xy 1 9)", "(xy 1 1) (xy 8 9)");
    let value = comparison_result(
        BOARD,
        &after,
        &empty_report(),
        &report("clearance", ZERO, &["z", "p"]),
    );
    assert!(value["newly_reported_after_refill"].is_null(), "{value}");
    assert_eq!(value["attribution_reason"], "fill_geometry_unresolved");
}

#[test]
fn a_footprint_rule_area_does_not_block_attribution() {
    // B2: a rule area (keepout) has no fill and is not refilled. Both the
    // index-level and the tree-level check skip it, like `zone_fill_check`.
    let rule_area = "(zone (uuid \"fp-keepout\") (net \"\") (layer \"F.Cu\") (keepout (tracks not_allowed)) (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1)))) (pad \"1\"";
    let before = BOARD.replace("(pad \"1\"", rule_area);
    let after = before.replace("(xy 9 9)", "(xy 8 9)");
    let value = comparison_result(
        &before,
        &after,
        &empty_report(),
        &report("clearance", ZERO, &["z", "p"]),
    );
    assert_eq!(value["newly_reported_after_refill"], 1, "{value}");
    assert!(value["attribution_reason"].is_null(), "{value}");
}

#[test]
fn guard_m11_a_copper_zone_under_an_unsupported_owner_blocks_attribution() {
    // The tree-level check looks only inside footprints, so only the
    // index-level check refuses a copper zone under any other owner.
    let nested = "(generated (uuid \"gen\") (zone (uuid \"nested\") (net \"GND\") (layer \"F.Cu\") (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1))))) (segment ";
    let before = BOARD.replace("(segment ", nested);
    let after = before.replace("(xy 9 9)", "(xy 8 9)");
    let value = comparison_result(
        &before,
        &after,
        &empty_report(),
        &report("clearance", ZERO, &["z", "p"]),
    );
    assert!(value["newly_reported_after_refill"].is_null(), "{value}");
    assert_eq!(value["attribution_reason"], "zone_owner_unsupported");
    // A rule area under that owner is not copper and does not block.
    let rule_area = before.replace(
        "(uuid \"nested\") (net \"GND\")",
        "(uuid \"nested\") (net \"\") (keepout (tracks not_allowed))",
    );
    let value = comparison_result(
        &rule_area,
        &rule_area.replace("(xy 9 9)", "(xy 8 9)"),
        &empty_report(),
        &report("clearance", ZERO, &["z", "p"]),
    );
    assert_eq!(value["newly_reported_after_refill"], 1, "{value}");
}

#[test]
fn guard_m12_inner_copper_layers_are_in1_to_in30() {
    let source = BOARD.replace(
        "(4 \"In1.Cu\" signal)",
        "(4 \"In1.Cu\" signal) (66 \"In31.Cu\" signal)",
    );
    let value = inspect(&source, &report("clearance", ZERO, &["z", "p"]));
    assert_eq!(value["status"], "incomplete", "{value}");
    assert_eq!(reason_of(&value)["copper_layers_unresolved"], 1);
}

fn severities(entries: &str) -> String {
    format!(r#"{{"board":{{"design_settings":{{"rule_severities":{{{entries}}}}}}}}}"#)
}

#[test]
fn a_project_that_ignores_a_supported_rule_is_partial_and_refuses_attribution() {
    // B1: KiCad gives an ignored code a zero limit and never tests it
    // (`DRC_ENGINE::RunTests`), so an empty report is not a checked zero.
    let new = report("clearance", ZERO, &["z", "p"]);
    for rule in ["clearance", "hole_clearance", "shorting_items"] {
        let project = severities(&format!(r#""{rule}":"ignore""#));
        let value = diagnostic(
            &Inspection::new(
                &empty_report(),
                Ok(&with_project(BOARD, &project)),
                "saved_fills",
            ),
            None,
            Err("not_compared"),
        );
        assert_eq!(value["status"], "incomplete", "{rule}: {value}");
        assert_eq!(value["primary_report"]["coverage"], "partial");
        assert_eq!(
            value["primary_report"]["partial_reasons"],
            json!(["kicad_rule_ignored"])
        );
        assert_eq!(
            value["primary_report"]["rules_ignored_by_project"],
            json!([rule])
        );
        let compared = comparison_with_project(&project, &empty_report(), &new);
        assert!(
            compared["newly_reported_after_refill"].is_null(),
            "{rule}: {compared}"
        );
        assert_eq!(compared["attribution_reason"], "kicad_rule_ignored");
    }
    // KiCad disables a rule only for the exact string "ignore", and reads
    // severities only from an object (`SeverityFromString`).
    let not_object = r#"{"board":{"design_settings":{"rule_severities":"ignore"}}}"#.to_owned();
    for project in [
        severities(r#""clearance":"warning""#),
        severities(r#""clearance":"error""#),
        severities(r#""clearance":"Ignore""#),
        severities(r#""silk_overlap":"ignore""#),
        not_object,
    ] {
        let value = comparison_with_project(&project, &empty_report(), &new);
        assert_eq!(
            value["primary_report"]["coverage"], "complete",
            "{project}: {value}"
        );
        assert_eq!(
            value["newly_reported_after_refill"], 1,
            "{project}: {value}"
        );
    }
    // A severity KiCad cannot read as a string is not assumed to be tested.
    let value =
        comparison_with_project(&severities(r#""shorting_items":0"#), &empty_report(), &new);
    assert_eq!(
        value["primary_report"]["partial_reasons"],
        json!(["project_rule_severity_unreadable"])
    );
    assert_eq!(
        value["attribution_reason"],
        "project_rule_severity_unreadable"
    );
}

#[test]
fn guard_layers_a_reverse_ordered_via_is_unresolved() {
    // INSTR-3 gap: the via range needs its first layer above its second.
    let reversed = BOARD.replace("(layers \"F.Cu\" \"B.Cu\")", "(layers \"B.Cu\" \"F.Cu\")");
    let value = inspect(&reversed, &report("clearance", ZERO, &["z", "v"]));
    assert_eq!(value["status"], "incomplete", "{value}");
    assert_eq!(reason_of(&value)["copper_membership_unresolved"], 1);
    let ordered = inspect(BOARD, &report("clearance", ZERO, &["z", "v"]));
    assert_eq!(ordered["status"], "suspected", "{ordered}");
}

#[test]
fn guard_layers_a_zone_with_a_non_copper_layer_is_unresolved() {
    // INSTR-3 gap: a zone layer the copper table does not name is not skipped,
    // even beside a copper layer.
    let r = report("clearance", ZERO, &["z", "p"]);
    let masked = BOARD.replace(
        "(net \"GND\") (layer \"F.Cu\")",
        "(net \"GND\") (layers \"F.Cu\" \"F.Mask\")",
    );
    let value = inspect(&masked, &r);
    assert_eq!(value["status"], "incomplete", "{value}");
    assert_eq!(reason_of(&value)["copper_membership_unresolved"], 1);
    let copper_only = BOARD.replace(
        "(net \"GND\") (layer \"F.Cu\")",
        "(net \"GND\") (layers \"F.Cu\")",
    );
    assert_eq!(inspect(&copper_only, &r)["status"], "suspected");
}

#[test]
fn guard_layers_a_pad_with_an_unknown_inner_layer_is_unresolved() {
    // INSTR-3 gap: a pad's mask layers are skipped, but a copper layer the
    // board does not have is not.
    let r = report("clearance", ZERO, &["z", "p"]);
    let unknown = BOARD.replace(
        "(layers \"*.Cu\" \"*.Mask\")",
        "(layers \"F.Cu\" \"In5.Cu\" \"F.Mask\")",
    );
    let value = inspect(&unknown, &r);
    assert_eq!(value["status"], "incomplete", "{value}");
    assert_eq!(reason_of(&value)["copper_membership_unresolved"], 1);
    let known = BOARD.replace(
        "(layers \"*.Cu\" \"*.Mask\")",
        "(layers \"F.Cu\" \"In1.Cu\" \"F.Mask\")",
    );
    assert_eq!(inspect(&known, &r)["status"], "suspected");
}
