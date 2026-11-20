//! Tests of the differential pairs of a fastroute call (`route_pairs.rs`). Each guard
//! has a test that it holds and a counterpart on the honest case: that counterpart is
//! the guard's negative control.

use super::*;
use serde_json::json;

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| name.to_string()).collect()
}

fn declared(positive: &str, negative: &str) -> DeclaredPair {
    DeclaredPair {
        positive: positive.to_string(),
        negative: negative.to_string(),
    }
}

/// A project file with the given classes and patterns.
fn project(classes: Value, patterns: Value, assignments: Value) -> String {
    json!({
        "net_settings": {
            "classes": classes,
            "netclass_patterns": patterns,
            "netclass_assignments": assignments,
        }
    })
    .to_string()
}

/// A DSN with the given nets in its network and a wire that also names a net.
fn dsn_with(nets: &[&str]) -> String {
    let mut text = String::from("(pcb board.kicad_pcb\n  (network\n");
    for net in nets {
        text.push_str(&format!("    (net {net}\n      (pins C1-1)\n    )\n"));
    }
    text.push_str("    (class k1 GND\n      (rule (width 250))\n    )\n  )\n");
    text.push_str("  (wiring\n    (wire (path F.Cu 200 0 0 10 0)(net WIRED_ONLY))\n  )\n)\n");
    text
}

// ─── Name resolution ────────────────────────────────────────────────────────

#[test]
fn a_name_resolves_to_the_net_with_a_sheet_path_in_front_when_only_that_exists() {
    let nets = names(&["/CAN_H", "/CAN_L", "GND"]);
    assert_eq!(resolve_name("CAN_H", &nets), Ok("/CAN_H".to_string()));
    // The name as the DSN writes it resolves to itself.
    assert_eq!(resolve_name("/CAN_H", &nets), Ok("/CAN_H".to_string()));
    assert_eq!(resolve_name("GND", &nets), Ok("GND".to_string()));
}

#[test]
fn an_exact_match_wins_over_a_net_with_a_sheet_path() {
    // Both `CAN_H` and `/CAN_H` exist: the exact name is the one.
    let nets = names(&["/CAN_H", "CAN_H", "/sheet/CAN_H"]);
    assert_eq!(resolve_name("CAN_H", &nets), Ok("CAN_H".to_string()));
    // Control: without the exact net the same name is ambiguous.
    let nets = names(&["/CAN_H", "/sheet/CAN_H"]);
    assert!(resolve_name("CAN_H", &nets).is_err());
}

#[test]
fn an_ambiguous_suffix_is_unresolved_and_names_the_nets() {
    let nets = names(&["/a/CAN_H", "/b/CAN_H", "/c/CAN_H", "/d/CAN_H", "/e/CAN_H"]);
    let reason = resolve_name("CAN_H", &nets).unwrap_err();
    assert!(reason.contains("ambiguous"), "{reason}");
    assert!(reason.contains("5 DSN nets"), "{reason}");
    // Three are listed, in order, and the rest counted.
    assert!(reason.contains("/a/CAN_H, /b/CAN_H, /c/CAN_H"), "{reason}");
    assert!(!reason.contains("/d/CAN_H"), "{reason}");
    assert!(reason.contains("and 2 more"), "{reason}");
    // Control: two matches list both and count no more.
    let two = resolve_name("CAN_H", &names(&["/a/CAN_H", "/b/CAN_H"])).unwrap_err();
    assert!(two.contains("/a/CAN_H, /b/CAN_H"), "{two}");
    assert!(!two.contains("more"), "{two}");
}

#[test]
fn a_missing_name_is_unresolved_and_a_suffix_must_start_at_a_slash() {
    let nets = names(&["/XCAN_H", "GND"]);
    let reason = resolve_name("CAN_H", &nets).unwrap_err();
    assert!(reason.contains("no net of the DSN"), "{reason}");
    assert!(
        reason.contains("'CAN_H'") && reason.contains("'/CAN_H'"),
        "{reason}"
    );
    // Control: the same net with the slash is found.
    assert_eq!(
        resolve_name("CAN_H", &names(&["/X/CAN_H"])),
        Ok("/X/CAN_H".to_string())
    );
}

// ─── Pairs ──────────────────────────────────────────────────────────────────

#[test]
fn a_pair_resolves_and_keeps_both_spellings() {
    let nets = names(&["/CAN_H", "/CAN_L", "GND"]);
    let plan = resolve_pairs(&[declared("CAN_H", "CAN_L")], &nets, None);
    assert!(plan.unresolved.is_empty(), "{plan:?}");
    assert_eq!(
        plan.resolved,
        vec![ResolvedPair {
            positive: "CAN_H".to_string(),
            negative: "CAN_L".to_string(),
            dsn_positive: "/CAN_H".to_string(),
            dsn_negative: "/CAN_L".to_string(),
            gap_mm: None,
        }]
    );
}

#[test]
fn a_pair_with_a_missing_name_is_left_out_and_says_which() {
    let nets = names(&["/CAN_H", "GND"]);
    let plan = resolve_pairs(&[declared("CAN_H", "CAN_L")], &nets, None);
    assert!(plan.resolved.is_empty());
    assert_eq!(plan.unresolved.len(), 1);
    let left_out = &plan.unresolved[0];
    assert_eq!(
        (left_out.positive.as_str(), left_out.negative.as_str()),
        ("CAN_H", "CAN_L")
    );
    assert!(left_out.reason.contains("'CAN_L'"), "{}", left_out.reason);
    assert!(!left_out.reason.contains("'CAN_H'"), "{}", left_out.reason);
    // Both missing: both are named, in one reason.
    let both = resolve_pairs(&[declared("A", "B")], &nets, None);
    let reason = &both.unresolved[0].reason;
    assert!(reason.contains("'A'") && reason.contains("'B'"), "{reason}");
}

#[test]
fn a_pair_whose_two_names_are_one_net_is_left_out() {
    let nets = names(&["/CAN_H", "GND"]);
    for (positive, negative) in [("CAN_H", "/CAN_H"), ("CAN_H", "CAN_H")] {
        let plan = resolve_pairs(&[declared(positive, negative)], &nets, None);
        assert!(plan.resolved.is_empty(), "{positive} {negative}");
        let reason = &plan.unresolved[0].reason;
        assert!(reason.contains("one DSN net '/CAN_H'"), "{reason}");
    }
    // Control: two different nets are a pair.
    let nets = names(&["/CAN_H", "/CAN_L"]);
    assert_eq!(
        resolve_pairs(&[declared("CAN_H", "CAN_L")], &nets, None)
            .resolved
            .len(),
        1
    );
}

#[test]
fn a_net_name_a_pairs_file_cannot_hold_is_left_out() {
    let nets = names(&["/CAN H", "/CAN_L", "/USB_P", "/USB_N"]);
    let plan = resolve_pairs(
        &[declared("/CAN H", "CAN_L"), declared("USB_P", "USB_N")],
        &nets,
        None,
    );
    assert_eq!(plan.resolved.len(), 1);
    assert_eq!(plan.resolved[0].dsn_positive, "/USB_P");
    assert!(
        plan.unresolved[0].reason.contains("one token"),
        "{:?}",
        plan.unresolved
    );
}

#[test]
fn a_net_is_in_one_pair_only_and_the_first_pair_keeps_it() {
    let nets = names(&["/A", "/B", "/C"]);
    let plan = resolve_pairs(
        &[declared("A", "B"), declared("B", "C"), declared("A", "B")],
        &nets,
        None,
    );
    assert_eq!(plan.resolved.len(), 1);
    assert_eq!(plan.resolved[0].dsn_positive, "/A");
    assert_eq!(plan.unresolved.len(), 2);
    assert!(plan.unresolved[0]
        .reason
        .contains("already in the pair '/A' and '/B'"));
    assert_eq!(plan.unresolved[1].negative, "B");
}

#[test]
fn an_unresolved_pair_does_not_stop_the_others_and_order_is_kept() {
    let nets = names(&["/Z_P", "/Z_N", "/A_P", "/A_N"]);
    let plan = resolve_pairs(
        &[
            declared("Z_P", "Z_N"),
            declared("X_P", "X_N"),
            declared("A_P", "A_N"),
        ],
        &nets,
        None,
    );
    let order: Vec<&str> = plan
        .resolved
        .iter()
        .map(|p| p.dsn_positive.as_str())
        .collect();
    assert_eq!(order, ["/Z_P", "/A_P"]);
    assert_eq!(plan.unresolved.len(), 1);
    assert_eq!(plan.unresolved[0].positive, "X_P");
}

// ─── The gap ────────────────────────────────────────────────────────────────

#[test]
fn the_gap_is_the_diff_pair_gap_of_the_class_that_names_the_net() {
    let text = project(
        json!([
            { "name": "Default", "priority": 2147483647, "diff_pair_gap": 0.25 },
            { "name": "CAN", "priority": 0, "diff_pair_gap": 0.18 },
        ]),
        json!([]),
        json!({ "/CAN_H": "CAN" }),
    );
    assert_eq!(gap_mm(&text, "/CAN_H"), Some(0.18));
    // A net no class names is in Default.
    assert_eq!(gap_mm(&text, "/OTHER"), Some(0.25));
}

#[test]
fn a_pattern_gives_the_class_as_an_assignment_does() {
    let text = project(
        json!([{ "name": "CAN", "priority": 0, "diff_pair_gap": 0.3 }]),
        json!([{ "netclass": "CAN", "pattern": "/CAN_*" }]),
        Value::Null,
    );
    assert_eq!(gap_mm(&text, "/CAN_H"), Some(0.3));
    // Control: a net the pattern does not match is in no class, and nothing sets a gap.
    assert_eq!(gap_mm(&text, "/USB_P"), None);
}

#[test]
fn the_gap_is_absent_when_the_class_sets_zero_or_nothing() {
    let zero = project(
        json!([
            { "name": "Default", "diff_pair_gap": 0.25 },
            { "name": "CAN", "priority": 0, "diff_pair_gap": 0 },
        ]),
        json!([]),
        json!({ "/CAN_H": "CAN" }),
    );
    // A class that sets 0 decides: KiCad takes it, and 0 is "no gap".
    assert_eq!(gap_mm(&zero, "/CAN_H"), None);
    let nothing = project(
        json!([
            { "name": "Default", "clearance": 0.2 },
            { "name": "CAN", "priority": 0 },
        ]),
        json!([]),
        json!({ "/CAN_H": "CAN" }),
    );
    assert_eq!(gap_mm(&nothing, "/CAN_H"), None);
    // `null` is unset, too.
    let null = project(
        json!([{ "name": "CAN", "priority": 0, "diff_pair_gap": null }]),
        json!([]),
        json!({ "/CAN_H": "CAN" }),
    );
    assert_eq!(gap_mm(&null, "/CAN_H"), None);
    // Control: a positive value is read.
    let some = project(
        json!([{ "name": "CAN", "priority": 0, "diff_pair_gap": 0.2 }]),
        json!([]),
        json!({ "/CAN_H": "CAN" }),
    );
    assert_eq!(gap_mm(&some, "/CAN_H"), Some(0.2));
}

#[test]
fn a_class_that_leaves_the_gap_unset_takes_the_defaults_as_in_kicad() {
    let text = project(
        json!([
            { "name": "Default", "diff_pair_gap": 0.25 },
            { "name": "CAN", "priority": 0, "clearance": 0.2 },
        ]),
        json!([]),
        json!({ "/CAN_H": "CAN" }),
    );
    assert_eq!(gap_mm(&text, "/CAN_H"), Some(0.25));
}

#[test]
fn of_several_classes_the_lowest_priority_number_decides_then_the_name() {
    let classes = json!([
        { "name": "Default", "diff_pair_gap": 0.25 },
        { "name": "Slow", "priority": 5, "diff_pair_gap": 0.5 },
        { "name": "Fast", "priority": 1, "diff_pair_gap": 0.1 },
        { "name": "B", "priority": 3, "diff_pair_gap": 0.31 },
        { "name": "A", "priority": 3, "diff_pair_gap": 0.32 },
    ]);
    let assign = |classes: &[&str]| json!({ "/N": classes });
    let text = project(classes.clone(), json!([]), assign(&["Slow", "Fast"]));
    assert_eq!(gap_mm(&text, "/N"), Some(0.1), "priority 1 beats 5");
    let text = project(classes.clone(), json!([]), assign(&["Fast", "Slow"]));
    assert_eq!(
        gap_mm(&text, "/N"),
        Some(0.1),
        "the order of the list does not matter"
    );
    let text = project(classes.clone(), json!([]), assign(&["B", "A"]));
    assert_eq!(gap_mm(&text, "/N"), Some(0.32), "equal priority: by name");
    // A high-priority class that sets none leaves it to the next one.
    let text = project(
        json!([
            { "name": "Fast", "priority": 1 },
            { "name": "Slow", "priority": 5, "diff_pair_gap": 0.5 },
        ]),
        json!([]),
        assign(&["Fast", "Slow"]),
    );
    assert_eq!(gap_mm(&text, "/N"), Some(0.5));
}

#[test]
fn a_project_that_cannot_be_read_gives_no_gap() {
    assert_eq!(gap_mm("not json", "/CAN_H"), None);
    assert_eq!(gap_mm("{}", "/CAN_H"), None);
    assert_eq!(
        gap_mm(r#"{"net_settings":{"classes":"x"}}"#, "/CAN_H"),
        None
    );
}

#[test]
fn the_gap_of_a_pair_is_the_positive_nets() {
    let text = project(
        json!([
            { "name": "P", "priority": 0, "diff_pair_gap": 0.15 },
            { "name": "N", "priority": 0, "diff_pair_gap": 0.4 },
        ]),
        json!([]),
        json!({ "/CAN_H": "P", "/CAN_L": "N" }),
    );
    let nets = names(&["/CAN_H", "/CAN_L"]);
    let plan = resolve_pairs(&[declared("CAN_H", "CAN_L")], &nets, Some(&text));
    assert_eq!(plan.resolved[0].gap_mm, Some(0.15));
    // Control: the negative net's class is not the one read.
    let swapped = resolve_pairs(&[declared("CAN_L", "CAN_H")], &nets, Some(&text));
    assert_eq!(swapped.resolved[0].gap_mm, Some(0.4));
    // Without a project there is no gap.
    assert_eq!(
        resolve_pairs(&[declared("CAN_H", "CAN_L")], &nets, None).resolved[0].gap_mm,
        None
    );
}

// ─── The file and the fields ────────────────────────────────────────────────

fn pair(dsn_positive: &str, dsn_negative: &str, gap_mm: Option<f64>) -> ResolvedPair {
    ResolvedPair {
        positive: "P".to_string(),
        negative: "N".to_string(),
        dsn_positive: dsn_positive.to_string(),
        dsn_negative: dsn_negative.to_string(),
        gap_mm,
    }
}

#[test]
fn the_file_has_one_line_per_pair_with_the_gap_only_when_set() {
    let plan = PairsPlan {
        resolved: vec![
            pair("/CAN_H", "/CAN_L", Some(0.2)),
            pair("USB_P", "USB_N", None),
        ],
        ..PairsPlan::default()
    };
    assert_eq!(
        plan.file_text().as_deref(),
        Some("pair /CAN_H /CAN_L gap=0.2\npair USB_P USB_N\n")
    );
    // No pair, no file.
    assert_eq!(PairsPlan::default().file_text(), None);
    let only_unresolved = PairsPlan {
        unresolved: vec![UnresolvedPair {
            positive: "A".into(),
            negative: "B".into(),
            reason: "x".into(),
        }],
        ..PairsPlan::default()
    };
    assert_eq!(only_unresolved.file_text(), None);
}

#[test]
fn the_fields_are_lists_and_empty_lists_without_pairs() {
    let empty = PairsPlan::default().fields();
    assert_eq!(
        empty,
        vec![
            ("differential_pairs", json!([])),
            ("differential_pairs_unresolved", json!([])),
        ]
    );
    let plan = PairsPlan {
        resolved: vec![pair("/CAN_H", "/CAN_L", Some(0.2)), pair("A", "B", None)],
        unresolved: vec![UnresolvedPair {
            positive: "X".into(),
            negative: "Y".into(),
            reason: "no net".into(),
        }],
        note: None,
    };
    let fields = plan.fields();
    assert_eq!(fields[0].0, "differential_pairs");
    assert_eq!(
        fields[0].1,
        json!([
            { "positive": "P", "negative": "N", "dsn_positive": "/CAN_H",
              "dsn_negative": "/CAN_L", "gap_mm": 0.2 },
            { "positive": "P", "negative": "N", "dsn_positive": "A",
              "dsn_negative": "B", "gap_mm": null },
        ])
    );
    assert_eq!(fields[1].0, "differential_pairs_unresolved");
    assert_eq!(
        fields[1].1,
        json!([{ "positive": "X", "negative": "Y", "reason": "no net" }])
    );
    assert_eq!(fields.len(), 2, "no note key without a note");
    let noted = PairsPlan {
        note: Some("unusable".into()),
        ..PairsPlan::default()
    };
    assert_eq!(
        noted.fields()[2],
        ("differential_pairs_note", json!("unusable"))
    );
}

#[test]
fn the_file_is_written_into_the_call_folder_only_with_a_pair() {
    let dir = tempfile::tempdir().unwrap();
    let plan = PairsPlan {
        resolved: vec![pair("/CAN_H", "/CAN_L", Some(0.2))],
        ..PairsPlan::default()
    };
    let path = write_file(dir.path(), &plan).unwrap().expect("a path");
    assert_eq!(path, dir.path().join("pairs.txt"));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "pair /CAN_H /CAN_L gap=0.2\n"
    );
    // Control: no pair writes no file.
    let other = tempfile::tempdir().unwrap();
    assert_eq!(
        write_file(other.path(), &PairsPlan::default()).unwrap(),
        None
    );
    assert!(!other.path().join("pairs.txt").exists());
    // A folder that cannot take the file is an error naming it.
    let error = write_file(&dir.path().join("missing"), &plan).unwrap_err();
    assert!(format!("{error:#}").contains("pairs.txt"), "{error:#}");
}

// ─── The DSN's nets ─────────────────────────────────────────────────────────

#[test]
fn the_dsn_nets_are_the_networks_in_order_without_the_wiring() {
    let dsn = dsn_with(&["/CAN_H", "\"Net-(R1-Pad1)\"", "GND", "/CAN_H"]);
    assert_eq!(
        specctra::dsn_net_names(&dsn).unwrap(),
        names(&["/CAN_H", "Net-(R1-Pad1)", "GND"])
    );
    assert!(specctra::dsn_net_names("(pcb x)").unwrap().is_empty());
    assert!(specctra::dsn_net_names("(session x)").is_err());
    assert!(specctra::dsn_net_names("not a dsn (").is_err());
}

// ─── The plan of a call ─────────────────────────────────────────────────────

/// A project folder with a board (never read), its project file and the given intent.
fn folder_with(intent: Option<&str>, pro: Option<&str>) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("b.kicad_pcb");
    std::fs::write(&board, "(kicad_pcb)").unwrap();
    if let Some(intent) = intent {
        std::fs::create_dir_all(dir.path().join(".konnect")).unwrap();
        std::fs::write(dir.path().join(".konnect/design_intent.json"), intent).unwrap();
    }
    if let Some(pro) = pro {
        std::fs::write(dir.path().join("b.kicad_pro"), pro).unwrap();
    }
    (dir, board)
}

fn intent(interfaces: Value) -> String {
    json!({ "schema_version": 1, "rails": [], "currents": [],
            "interfaces": interfaces, "fab_target": {} })
    .to_string()
}

fn can_interface() -> Value {
    json!({ "name": "CAN", "kind": "can", "nets": ["CAN_H", "CAN_L"],
            "differential_pairs": [{ "positive": "CAN_H", "negative": "CAN_L" }] })
}

#[test]
fn an_intent_with_a_pair_gives_the_pair_the_gap_and_the_file_line() {
    let pro = project(
        json!([{ "name": "CAN", "priority": 0, "diff_pair_gap": 0.2 }]),
        json!([{ "netclass": "CAN", "pattern": "/CAN_*" }]),
        Value::Null,
    );
    let (_dir, board) = folder_with(Some(&intent(json!([can_interface()]))), Some(&pro));
    let plan = plan(&board, &dsn_with(&["/CAN_H", "/CAN_L", "GND"]));
    assert!(plan.unresolved.is_empty(), "{plan:?}");
    assert_eq!(plan.note, None);
    assert_eq!(
        plan.file_text().as_deref(),
        Some("pair /CAN_H /CAN_L gap=0.2\n")
    );
}

#[test]
fn without_an_intent_or_with_no_pair_in_it_the_plan_is_empty() {
    let dsn = dsn_with(&["/CAN_H", "/CAN_L"]);
    // No intent file.
    let (_dir, board) = folder_with(None, None);
    assert_eq!(plan(&board, &dsn), PairsPlan::default());
    // An intent whose interface declares no pair.
    let bare = json!([{ "name": "CAN", "kind": "can", "nets": ["CAN_H", "CAN_L"] }]);
    let (_dir, board) = folder_with(Some(&intent(bare)), None);
    assert_eq!(plan(&board, &dsn), PairsPlan::default());
    // An intent with no interface at all.
    let (_dir, board) = folder_with(Some(&intent(json!([]))), None);
    assert_eq!(plan(&board, &dsn), PairsPlan::default());
    // Control: the pair in the same folder shape does give a plan.
    let (_dir, board) = folder_with(Some(&intent(json!([can_interface()]))), None);
    assert_eq!(plan(&board, &dsn).resolved.len(), 1);
}

#[test]
fn the_nets_are_never_guessed_from_their_names() {
    // Nets named like a pair, and no intent: nothing is paired.
    let (_dir, board) = folder_with(None, None);
    let plan = plan(
        &board,
        &dsn_with(&["/USB_DP", "/USB_DN", "/CAN_H", "/CAN_L"]),
    );
    assert!(plan.resolved.is_empty() && plan.unresolved.is_empty());
}

#[test]
fn pairs_of_every_interface_are_read_in_file_order() {
    let usb = json!({ "name": "USB", "kind": "usb2", "nets": ["USB_DP", "USB_DN"],
        "differential_pairs": [{ "positive": "USB_DP", "negative": "USB_DN" }] });
    let (_dir, board) = folder_with(Some(&intent(json!([usb, can_interface()]))), None);
    let dsn = dsn_with(&["/CAN_H", "/CAN_L", "/USB_DP", "/USB_DN"]);
    let plan = plan(&board, &dsn);
    assert_eq!(
        plan.file_text().as_deref(),
        Some("pair /USB_DP /USB_DN\npair /CAN_H /CAN_L\n")
    );
}

#[test]
fn an_intent_that_cannot_be_used_adds_a_note_and_no_pair() {
    let (_dir, board) = folder_with(Some("{ not json"), None);
    let plan = plan(&board, &dsn_with(&["/CAN_H", "/CAN_L"]));
    assert!(plan.resolved.is_empty() && plan.unresolved.is_empty());
    let note = plan.note.expect("a note");
    assert!(
        note.contains("design_intent.json") && note.contains("cannot be used"),
        "{note}"
    );
    let fields = PairsPlan {
        note: Some(note),
        ..PairsPlan::default()
    }
    .fields();
    assert_eq!(fields.len(), 3);
}

#[test]
fn a_dsn_that_cannot_be_read_leaves_every_declared_pair_unresolved() {
    let (_dir, board) = folder_with(Some(&intent(json!([can_interface()]))), None);
    let plan = plan(&board, "not a dsn (");
    assert!(plan.resolved.is_empty());
    assert_eq!(plan.unresolved.len(), 1);
    assert!(
        plan.unresolved[0].reason.contains("could not be read"),
        "{:?}",
        plan.unresolved
    );
    // An unreadable DSN is no reason to look at it when nothing is declared.
    let (_dir, board) = folder_with(Some(&intent(json!([]))), None);
    assert_eq!(super::plan(&board, "not a dsn ("), PairsPlan::default());
}

#[test]
fn a_pair_that_is_not_in_the_dsn_is_reported_through_the_plan() {
    let (_dir, board) = folder_with(Some(&intent(json!([can_interface()]))), None);
    let plan = plan(&board, &dsn_with(&["/CAN_H", "GND"]));
    assert!(plan.file_text().is_none());
    let fields = plan.fields();
    let unresolved = fields[1].1.as_array().unwrap();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0]["positive"], "CAN_H");
    assert!(unresolved[0]["reason"]
        .as_str()
        .unwrap()
        .contains("'CAN_L'"));
}

#[test]
fn a_board_with_no_folder_reads_the_current_one_and_finds_nothing() {
    // `plan` is given the board's path as the manifest names it; a bare file name has
    // an empty parent, which stands for the current folder.
    assert_eq!(
        plan(Path::new("route-pairs-no-such-board.kicad_pcb"), "(pcb x)"),
        PairsPlan::default()
    );
}
