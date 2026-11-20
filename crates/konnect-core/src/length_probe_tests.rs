//! Tests of the judge's length check: the probe's files and arguments, the reading of
//! KiCad's report (a report captured from a real kicad-cli run), the check against the
//! windows, G4's lines and the key's length part (docs/LENGTH_TUNING_DESIGN.md, L8 to
//! L10).

use super::*;
use crate::tools::cli::{parse_drc_report, DrcViolation, ReportItem};

/// A `kicad-cli` report of the probe on a project that lists no rule severities (so
/// every check ran), as KiCad 10.0.8 wrote it: seven probe entries (`S1` to `S6`,
/// `GND`) among 13 other violations.
const REPORT: &str = include_str!("../tests/fixtures/length_probe_report_kicad10.json");
/// The same probe with every other check set to `ignore`: only the seven entries.
const REPORT_IGNORED: &str =
    include_str!("../tests/fixtures/length_probe_report_ignored_kicad10.json");

fn names(nets: &[&str]) -> Vec<String> {
    nets.iter().map(|net| net.to_string()).collect()
}

/// The fixtures' probed nets, in rule order.
fn fixture_nets() -> Vec<String> {
    names(&["S1", "S2", "S3", "S4", "S5", "S6", "GND"])
}

/// What the fixtures' probe says, in rule order.
const FIXTURE_MM: [f64; 7] = [52.2760, 37.5011, 36.3050, 54.2499, 33.1560, 12.3816, 0.0];

fn report(raw: &str) -> DrcReport {
    parse_drc_report(&serde_json::from_str(raw).unwrap()).unwrap()
}

fn violation(rule: &str, description: &str, items: &[&str]) -> DrcViolation {
    DrcViolation {
        severity: "error".into(),
        description: description.into(),
        rule: rule.into(),
        pos: None,
        items: items
            .iter()
            .map(|description| ReportItem {
                description: description.to_string(),
                pos: None,
                uuid: None,
                kicad_reported_pos: None,
                ownership_status: None,
                item_kind: None,
                layer: None,
                owner: None,
            })
            .collect(),
    }
}

/// A probe entry as KiCad words it, for rule `number`.
fn entry(number: usize, mm: f64, items: &[&str]) -> DrcViolation {
    violation(
        "length_out_of_range",
        &format!(
            "Track length out of range (rule '__konnect_len_{number:04}' min length 2000.0000 mm; actual {mm:.4} mm)"
        ),
        items,
    )
}

fn report_of(violations: Vec<DrcViolation>) -> DrcReport {
    DrcReport {
        violations,
        unconnected_items: Some(Vec::new()),
        schematic_parity: None,
        ownership_diagnostic: None,
        schematic_parity_diagnostic: None,
    }
}

// ------------------------------------------------------------ probe files (L8)

#[test]
fn the_probe_rules_are_one_numbered_length_rule_per_net() {
    let nets = names(&["/DQ0", "/DQ1", "S3"]);
    let text = probe_rules_text(&nets);
    assert!(text.starts_with("(version 1)\n"), "{text}");
    for (index, net) in nets.iter().enumerate() {
        let name = format!("__konnect_len_{:04}", index + 1);
        assert!(text.contains(&format!("(rule \"{name}\"")), "{text}");
        assert!(
            text.contains(&format!("(condition \"A.NetName == '{net}'\"))")),
            "{text}"
        );
    }
    assert_eq!(text.matches("(severity error)").count(), 3, "{text}");
    assert_eq!(text.matches("(constraint length (min 2000mm))").count(), 3);

    // KiCad's own rule-file framing reads it back: three rules, in order.
    let rules = crate::custom_rules::parse(&text).expect("a well-formed rule file");
    let found: Vec<&str> = rules.iter().map(|rule| rule.name.as_str()).collect();
    assert_eq!(
        found,
        [
            "__konnect_len_0001",
            "__konnect_len_0002",
            "__konnect_len_0003"
        ]
    );
    // No net, no rule.
    assert_eq!(probe_rules_text(&[]), "(version 1)\n");
}

#[test]
fn the_probe_minimum_is_below_kicads_32_bit_limit_and_above_any_net() {
    // 2147.48 mm is the largest minimum KiCad keeps (`KiROUND` clamps above it).
    assert!(PROBE_MIN_MM < 2147.48 && PROBE_MIN_MM >= 1000.0);
    assert!(probe_rules_text(&names(&["A"])).contains("(min 2000mm)"));
}

fn severities(text: &str) -> serde_json::Map<String, Value> {
    let project: Value = serde_json::from_str(text).unwrap();
    project["board"]["design_settings"]["rule_severities"]
        .as_object()
        .expect("the probe project lists severities")
        .clone()
}

#[test]
fn the_probe_project_ignores_every_listed_check_and_keeps_the_rest() {
    let own = json!({
        "board": {
            "design_settings": {
                "rule_severities": {
                    "clearance": "error",
                    "silk_overlap": "warning",
                    "skew_out_of_range": "error",
                    "length_out_of_range": "error"
                },
                "rules": { "use_height_for_length_calcs": false, "min_clearance": 0.2 }
            }
        },
        "net_settings": { "classes": [{ "name": "Default" }] }
    })
    .to_string();
    let probe: Value = serde_json::from_str(&probe_project_text(&own).unwrap()).unwrap();
    let listed = severities(&probe.to_string());
    assert_eq!(listed["clearance"], "ignore");
    assert_eq!(listed["silk_overlap"], "ignore");
    assert_eq!(listed["skew_out_of_range"], "ignore");
    assert_eq!(listed["length_out_of_range"], "error");
    assert_eq!(listed.len(), 4);
    // Everything else, the via-height switch included, is kept as it is.
    assert_eq!(
        probe["board"]["design_settings"]["rules"]["use_height_for_length_calcs"],
        false
    );
    assert_eq!(
        probe["board"]["design_settings"]["rules"]["min_clearance"],
        0.2
    );
    assert_eq!(probe["net_settings"]["classes"][0]["name"], "Default");
}

/// A project that sets `length_out_of_range` to `ignore` hides every probe violation
/// whatever a rule's own `severity` says (step 0); the probe copy sets it to `error`.
#[test]
fn a_project_that_ignores_length_violations_gets_error_in_the_probe_copy() {
    let own = json!({
        "board": { "design_settings": { "rule_severities": {
            "clearance": "error", "length_out_of_range": "ignore"
        } } }
    })
    .to_string();
    let listed = severities(&probe_project_text(&own).unwrap());
    assert_eq!(listed["length_out_of_range"], "error");
    assert_eq!(listed["clearance"], "ignore");

    // `warning` is forced to `error` as well: the probe's entries are errors.
    let own = json!({
        "board": { "design_settings": { "rule_severities": { "length_out_of_range": "warning" } } }
    })
    .to_string();
    assert_eq!(
        severities(&probe_project_text(&own).unwrap())["length_out_of_range"],
        "error"
    );
}

#[test]
fn a_project_without_severities_still_gets_the_one_the_probe_needs() {
    for own in [
        "{}",
        r#"{"board": {}}"#,
        r#"{"board": {"design_settings": {"rules": {}}}}"#,
        r#"{"board": {"design_settings": {"rule_severities": []}}}"#,
        r#"{"board": 7}"#,
    ] {
        let listed = severities(&probe_project_text(own).unwrap());
        assert_eq!(
            listed,
            json!({ "length_out_of_range": "error" })
                .as_object()
                .unwrap()
                .clone(),
            "{own}"
        );
    }
    // The other design settings of such a project are not touched.
    let probe: Value = serde_json::from_str(
        &probe_project_text(r#"{"board": {"design_settings": {"rules": {"min_clearance": 0.2}}}}"#)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        probe["board"]["design_settings"]["rules"]["min_clearance"],
        0.2
    );
}

#[test]
fn a_project_that_is_not_a_json_object_cannot_be_probed() {
    for own in ["", "not json", "[]", "7", "\"text\""] {
        assert!(probe_project_text(own).is_err(), "{own:?}");
    }
}

#[test]
fn the_probe_runs_without_refill_save_or_parity_in_millimetres() {
    let args = probe_drc_args("/o/out.json", "/o/b.kicad_pcb");
    assert_eq!(
        args,
        [
            "pcb",
            "drc",
            "--output",
            "/o/out.json",
            "--format",
            "json",
            "--units",
            "mm",
            "/o/b.kicad_pcb"
        ]
    );
    for forbidden in ["--refill-zones", "--save-board", "--schematic-parity"] {
        assert!(!args.contains(&forbidden), "{forbidden}");
    }
}

// ------------------------------------------------------- actual_mm, parse_probe

#[test]
fn actual_mm_reads_every_description_of_the_real_reports() {
    for raw in [REPORT, REPORT_IGNORED] {
        let probe: Vec<f64> = report(raw)
            .violations
            .iter()
            .filter(|violation| violation.rule == "length_out_of_range")
            .map(|violation| actual_mm(&violation.description).expect("one actual length"))
            .collect();
        assert_eq!(probe, FIXTURE_MM, "in rule order of the report's entries");
    }
}

#[test]
fn actual_mm_does_not_read_the_words() {
    // The same message in other languages and with a decimal comma.
    for (text, expected) in [
        (
            "Track length out of range (rule '__konnect_len_0001' min length 2000.0000 mm; actual 52.2760 mm)",
            Some(52.2760),
        ),
        (
            "Länge außerhalb des Bereichs (Regel '__konnect_len_0001' Mindestlänge 2000,0000 mm; tatsächlich 52,2760 mm)",
            Some(52.2760),
        ),
        (
            "Longueur hors plage (règle '__konnect_len_0012' longueur min 2000.0000 mm ; réelle 37.5011\u{a0}mm)",
            Some(37.5011),
        ),
        (
            "長さが範囲外 (ルール '__konnect_len_0003' 最小 2000.0000mm; 実際 36.3050mm)",
            Some(36.3050),
        ),
        ("actual 0.0000 mm, min 2000.0000 mm", Some(0.0)),
        // A number inside a word or a name is not a length, even before "mm".
        ("rule 'abc12mm' min length 2000.0000 mm; actual 5.0000 mm", Some(5.0)),
    ] {
        assert_eq!(actual_mm(text), expected, "{text}");
    }
}

#[test]
fn actual_mm_is_none_unless_exactly_one_actual_length_is_there() {
    for text in [
        // The probe's own number only (a net at or above the minimum: no actual).
        "Track length out of range (rule '__konnect_len_0001' min length 2000.0000 mm)",
        "rule '__konnect_len_0001' min length 2000.0004 mm; actual 1999.9996 mm",
        // Two candidates.
        "min length 2000.0000 mm; actual 52.2760 mm and 3.0000 mm",
        // Not lengths: a rule number and bare numbers.
        "rule '__konnect_len_0001'",
        "length 52.2760",
        "",
        // Not below the minimum: not an actual length.
        "min length 2000.0000 mm; actual 2500.0000 mm",
    ] {
        assert_eq!(actual_mm(text), None, "{text}");
    }
}

#[test]
fn parse_probe_reads_both_real_reports() {
    for raw in [REPORT, REPORT_IGNORED] {
        let (lengths, unmeasured) = parse_probe(&report(raw), &fixture_nets()).unwrap();
        assert!(unmeasured.is_empty(), "{unmeasured:?}");
        assert_eq!(lengths.len(), 7);
        for (net, mm) in fixture_nets().iter().zip(FIXTURE_MM) {
            assert_eq!(lengths[net], mm, "{net}");
        }
    }
}

#[test]
fn parse_probe_ignores_every_other_violation_of_the_report() {
    // The full report holds 13 violations that are not the probe's.
    let full = report(REPORT);
    assert_eq!(full.violations.len(), 20);
    let (lengths, _) = parse_probe(&full, &fixture_nets()).unwrap();
    assert_eq!(lengths.len(), 7);
    // And the probe's entries of another rule key are not read.
    let mut renamed = report(REPORT_IGNORED);
    for violation in &mut renamed.violations {
        violation.rule = "clearance".into();
    }
    let (lengths, unmeasured) = parse_probe(&renamed, &fixture_nets()).unwrap();
    assert!(lengths.is_empty());
    assert_eq!(unmeasured, fixture_nets());
}

#[test]
fn a_net_with_no_entry_is_unmeasured() {
    let mut partial = report(REPORT_IGNORED);
    // Drop the entry of rule 3 (S3).
    partial
        .violations
        .retain(|violation| !violation.description.contains("__konnect_len_0003"));
    let (lengths, unmeasured) = parse_probe(&partial, &fixture_nets()).unwrap();
    assert_eq!(unmeasured, names(&["S3"]));
    assert!(!lengths.contains_key("S3"));
    assert_eq!(lengths.len(), 6);

    // A net at 2000 mm or longer has an entry only when it is below the minimum, so
    // it has none; an entry that holds no actual length is unmeasured too.
    let odd = report_of(vec![entry(1, 52.0, &["Track [A] on F.Cu"]), {
        violation(
            "length_out_of_range",
            "Track length out of range (rule '__konnect_len_0002' min length 2000.0000 mm)",
            &["Track [B] on F.Cu"],
        )
    }]);
    let (lengths, unmeasured) = parse_probe(&odd, &names(&["A", "B", "C"])).unwrap();
    assert_eq!(lengths.len(), 1);
    assert_eq!(unmeasured, names(&["B", "C"]));
}

#[test]
fn the_net_is_cross_checked_through_the_items() {
    // The entry of rule 1 is for `S1`: it counts only when an item names `[S1]`.
    let good = report_of(vec![entry(1, 10.0, &["Via [S1] on F.Cu - B.Cu"])]);
    assert_eq!(parse_probe(&good, &names(&["S1"])).unwrap().0["S1"], 10.0);

    // Any of the items may name it.
    let second = report_of(vec![entry(
        1,
        10.0,
        &[
            "Pad 1 [S9] of R1 on F.Cu",
            "Track [S1] on F.Cu, length 4 mm",
        ],
    )]);
    assert!(parse_probe(&second, &names(&["S1"])).is_ok());

    // Rule 1 is for `S1`, but its items name `S2`: the probe is not trusted.
    let wrong = report_of(vec![entry(1, 10.0, &["Via [S2] on F.Cu - B.Cu"])]);
    let error = parse_probe(&wrong, &names(&["S1", "S2"])).unwrap_err();
    assert!(format!("{error:#}").contains("another net"), "{error:#}");

    // A net name with brackets in it is matched whole.
    let bracketed = report_of(vec![entry(1, 7.0, &["Track [/DQ[0]] on F.Cu"])]);
    assert_eq!(
        parse_probe(&bracketed, &names(&["/DQ[0]"])).unwrap().0["/DQ[0]"],
        7.0
    );

    // An entry with no item at all names nothing: it does not count.
    let bare = report_of(vec![entry(1, 10.0, &[])]);
    let (lengths, unmeasured) = parse_probe(&bare, &names(&["S1"])).unwrap();
    assert!(lengths.is_empty());
    assert_eq!(unmeasured, names(&["S1"]));
}

#[test]
fn two_entries_for_one_net_are_an_error() {
    let twice = report_of(vec![
        entry(1, 10.0, &["Via [S1] on F.Cu - B.Cu"]),
        entry(1, 11.0, &["Track [S1] on F.Cu, length 4 mm"]),
    ]);
    let error = parse_probe(&twice, &names(&["S1"])).unwrap_err();
    assert!(format!("{error:#}").contains("twice"), "{error:#}");
}

#[test]
fn a_rule_number_that_was_never_written_is_an_error() {
    // The real report has seven entries; three nets means three rules.
    let error = parse_probe(&report(REPORT_IGNORED), &names(&["S1", "S2", "S3"])).unwrap_err();
    assert!(format!("{error:#}").contains("only 3 rule(s)"), "{error:#}");
    // Rule 0 does not exist either.
    let zero = report_of(vec![entry(0, 1.0, &["Track [S1] on F.Cu"])]);
    assert!(parse_probe(&zero, &names(&["S1"])).is_err());
}

// ------------------------------------------------------------------- check (L9)

fn group(name: &str, nets: &[&str], window: Window) -> CheckedGroup {
    CheckedGroup {
        interface: "ram".into(),
        name: name.into(),
        nets: names(nets),
        window,
    }
}

fn lengths_of(rows: &[(&str, f64)]) -> BTreeMap<String, f64> {
    rows.iter()
        .map(|(net, mm)| (net.to_string(), *mm))
        .collect()
}

fn checks_of(groups: Vec<CheckedGroup>, pairs: Vec<CheckedPair>) -> LengthChecks {
    LengthChecks { groups, pairs }
}

fn miss_of(measures: &LengthMeasures, net: &str) -> f64 {
    measures
        .groups
        .iter()
        .flat_map(|group| &group.nets)
        .find(|n| n.net == net)
        .unwrap_or_else(|| panic!("no net {net}"))
        .miss_mm
}

#[test]
fn a_skew_group_is_checked_against_its_longest_net() {
    let checks = checks_of(
        vec![group(
            "dq",
            &["a", "b", "c"],
            Window::Skew { tolerance_mm: 0.5 },
        )],
        vec![],
    );
    // Longest 42.00: the window is 41.50 to 42.00.
    let measures = check(
        &checks,
        &lengths_of(&[("a", 42.0), ("b", 41.5), ("c", 41.4)]),
        vec![],
        0.3,
    );
    let g = &measures.groups[0];
    assert_eq!(g.longest_mm, Some(42.0));
    assert_eq!((g.nets[1].low_mm, g.nets[1].high_mm), (41.5, 42.0));
    assert_eq!(miss_of(&measures, "a"), 0.0, "the longest net is inside");
    assert_eq!(miss_of(&measures, "b"), 0.0, "on the bound");
    assert!((miss_of(&measures, "c") - 0.1).abs() < 1e-9);
    assert!((measures.miss_mm - 0.1).abs() < 1e-9);
    assert_eq!(measures.probe_s, 0.3);
    assert!(measures.unmeasured.is_empty());
}

#[test]
fn the_bound_passes_within_half_a_printed_digit_and_one_step_past_it_fails() {
    let checks = checks_of(
        vec![group(
            "t",
            &["a"],
            Window::Target {
                target_mm: 42.0,
                tolerance_mm: 0.5,
            },
        )],
        vec![],
    );
    let miss = |mm: f64| miss_of(&check(&checks, &lengths_of(&[("a", mm)]), vec![], 0.0), "a");
    // Window 41.5 to 42.5. KiCad prints four decimals: a length printed on the bound
    // passes, and so does one that float noise puts a hair past it.
    for on_bound in [42.5, 42.500_04, 41.5, 41.499_96] {
        assert_eq!(miss(on_bound), 0.0, "{on_bound}");
    }
    // One printed step past the bound fails.
    assert!((miss(42.5001) - 0.0001).abs() < 1e-9);
    assert!((miss(41.4999) - 0.0001).abs() < 1e-9);
    // Negative control: 0.01 mm across a bound flips the verdict.
    assert_eq!(miss(42.49), 0.0);
    assert!((miss(42.51) - 0.01).abs() < 1e-9);
}

#[test]
fn a_target_group_checks_every_net_against_the_same_window() {
    let checks = checks_of(
        vec![group(
            "dq",
            &["a", "b", "c"],
            Window::Target {
                target_mm: 42.07,
                tolerance_mm: 0.25,
            },
        )],
        vec![],
    );
    let measures = check(
        &checks,
        &lengths_of(&[("a", 42.1), ("b", 41.2), ("c", 42.6)]),
        vec![],
        0.0,
    );
    assert_eq!(miss_of(&measures, "a"), 0.0);
    assert!((miss_of(&measures, "b") - 0.62).abs() < 1e-9, "below 41.82");
    assert!((miss_of(&measures, "c") - 0.28).abs() < 1e-9, "above 42.32");
    assert!(
        (measures.miss_mm - 0.90).abs() < 1e-9,
        "the misses are summed"
    );
}

#[test]
fn a_pair_is_checked_for_its_skew() {
    let pair = |max: f64| CheckedPair {
        positive: "p".into(),
        negative: "n".into(),
        max_skew_mm: max,
    };
    let measures = |p: f64, n: f64, max: f64| {
        check(
            &checks_of(vec![], vec![pair(max)]),
            &lengths_of(&[("p", p), ("n", n)]),
            vec![],
            0.0,
        )
    };
    let on_bound = measures(10.0, 10.1, 0.1);
    assert_eq!(on_bound.pairs[0].miss_mm, 0.0);
    assert!((on_bound.pairs[0].skew_mm.unwrap() - 0.1).abs() < 1e-9);
    // The sign does not matter.
    assert_eq!(measures(10.1, 10.0, 0.1).pairs[0].miss_mm, 0.0);
    // One printed step past the bound fails; the miss joins the sum.
    let past = measures(10.0, 10.1001, 0.1);
    assert!((past.pairs[0].miss_mm - 0.0001).abs() < 1e-9);
    assert!((past.miss_mm - 0.0001).abs() < 1e-9);
    // A pair with a net that has no length has no skew.
    let missing = check(
        &checks_of(vec![], vec![pair(0.1)]),
        &lengths_of(&[("p", 10.0)]),
        vec![],
        0.0,
    );
    assert_eq!(missing.pairs[0].skew_mm, None);
    assert_eq!(missing.unmeasured, names(&["n"]));
}

#[test]
fn group_and_pair_misses_are_summed_and_unmeasured_nets_are_listed() {
    let checks = checks_of(
        vec![group(
            "t",
            &["a", "b", "c"],
            Window::Target {
                target_mm: 10.0,
                tolerance_mm: 1.0,
            },
        )],
        vec![CheckedPair {
            positive: "a".into(),
            negative: "b".into(),
            max_skew_mm: 0.5,
        }],
    );
    // a is 0.5 above its window, b 0.25 below it, and the pair is 2.75 apart (2.25
    // over); c has no length.
    let measures = check(
        &checks,
        &lengths_of(&[("a", 11.5), ("b", 8.75)]),
        vec![],
        0.0,
    );
    assert_eq!(measures.unmeasured, names(&["c"]));
    assert!(
        (measures.miss_mm - (0.5 + 0.25 + 2.25)).abs() < 1e-9,
        "{}",
        measures.miss_mm
    );
    // A net the caller listed stays listed once, even when it has a length.
    let measures = check(
        &checks,
        &lengths_of(&[("a", 10.0), ("b", 10.0), ("c", 10.0)]),
        names(&["c", "z"]),
        0.0,
    );
    assert_eq!(measures.unmeasured, names(&["c", "z"]));
}

#[test]
fn a_skew_group_with_no_measured_net_has_nothing_to_miss() {
    let checks = checks_of(
        vec![group("dq", &["a", "b"], Window::Skew { tolerance_mm: 0.5 })],
        vec![],
    );
    let measures = check(&checks, &BTreeMap::new(), names(&["a", "b"]), 0.0);
    assert_eq!(measures.groups[0].longest_mm, None);
    assert_eq!(measures.miss_mm, 0.0);
    assert_eq!(measures.unmeasured, names(&["a", "b"]));
}

#[test]
fn the_check_of_the_real_probe_reads_its_numbers() {
    let (lengths, unmeasured) = parse_probe(&report(REPORT), &fixture_nets()).unwrap();
    // The two nets whose lengths are 52.2760 and 54.2499 are 1.9739 apart.
    let checks = checks_of(
        vec![group(
            "s",
            &["S1", "S4"],
            Window::Skew { tolerance_mm: 2.0 },
        )],
        vec![CheckedPair {
            positive: "S1".into(),
            negative: "S4".into(),
            max_skew_mm: 1.9,
        }],
    );
    let measures = check(&checks, &lengths, unmeasured, 0.2);
    assert_eq!(measures.groups[0].longest_mm, Some(54.2499));
    assert_eq!(
        measures.groups[0]
            .nets
            .iter()
            .map(|n| n.miss_mm)
            .sum::<f64>(),
        0.0
    );
    assert!((measures.pairs[0].skew_mm.unwrap() - 1.9739).abs() < 1e-9);
    assert!((measures.pairs[0].miss_mm - 0.0739).abs() < 1e-9);
}

// ------------------------------------------------------------ G2 / G4 split, key

#[test]
fn only_the_two_length_keys_leave_g2() {
    assert!(!counts_in_g2("length_out_of_range"));
    assert!(!counts_in_g2("skew_out_of_range"));
    for key in [
        "clearance",
        "track_width",
        "diff_pair_gap_out_of_range",
        "unconnected_items",
        "",
    ] {
        assert!(counts_in_g2(key), "{key}");
    }
}

fn counts(rows: &[(&str, usize)]) -> BTreeMap<String, usize> {
    rows.iter().map(|(k, n)| (k.to_string(), *n)).collect()
}

#[test]
fn kicads_own_length_and_skew_violations_are_summed() {
    assert_eq!(kicad_rule_violations(&BTreeMap::new()), 0);
    assert_eq!(
        kicad_rule_violations(&counts(&[
            ("length_out_of_range", 2),
            ("skew_out_of_range", 3),
            ("clearance", 40)
        ])),
        5
    );
}

fn measures_with(groups: Vec<GroupLengths>, pairs: Vec<PairSkew>, miss: f64) -> LengthMeasures {
    LengthMeasures {
        groups,
        pairs,
        unmeasured: vec![],
        miss_mm: miss,
        probe_s: 0.0,
    }
}

fn net(name: &str, mm: Option<f64>, low: f64, high: f64, miss: f64) -> NetLength {
    NetLength {
        net: name.into(),
        kicad_mm: mm,
        low_mm: low,
        high_mm: high,
        miss_mm: miss,
    }
}

#[test]
fn g4_names_each_failure_in_a_readable_line() {
    let measures = LengthMeasures {
        groups: vec![GroupLengths {
            interface: "ram".into(),
            name: "dq".into(),
            longest_mm: Some(42.1),
            nets: vec![
                net("/DQ6", Some(42.1), 41.82, 42.32, 0.0),
                net("/DQ7", Some(41.2), 41.82, 42.32, 0.62),
                net("/DQ8", Some(42.9), 41.82, 42.32, 0.58),
                net("/DQ9", None, 41.82, 42.32, 0.0),
            ],
        }],
        pairs: vec![PairSkew {
            positive: "/DQS+".into(),
            negative: "/DQS-".into(),
            skew_mm: Some(0.35),
            max_skew_mm: 0.25,
            miss_mm: 0.10,
        }],
        unmeasured: vec!["/DQ9".into()],
        miss_mm: 1.3,
        probe_s: 0.0,
    };
    let lines = g4_lines(
        Some(&measures),
        &counts(&[("length_out_of_range", 2), ("clearance", 9)]),
    );
    assert_eq!(
        lines,
        vec![
            "group 'dq' (ram): /DQ7 is 41.20 mm, 0.62 mm below its window 41.82\u{2013}42.32 mm",
            "group 'dq' (ram): /DQ8 is 42.90 mm, 0.58 mm above its window 41.82\u{2013}42.32 mm",
            "pair /DQS+ / /DQS-: skew 0.35 mm, 0.10 mm over its maximum 0.25 mm",
            "/DQ9: KiCad gave no length for this net (not on the board, or 2000 mm or longer)",
            "KiCad: 2 length_out_of_range violation(s) of the board's own rules",
        ]
    );
}

#[test]
fn g4_is_empty_when_everything_passes_or_nothing_is_checked() {
    let measures = measures_with(
        vec![GroupLengths {
            interface: "ram".into(),
            name: "dq".into(),
            longest_mm: Some(42.0),
            nets: vec![net("a", Some(42.0), 41.5, 42.0, 0.0)],
        }],
        vec![],
        0.0,
    );
    assert!(g4_lines(Some(&measures), &counts(&[("clearance", 5)])).is_empty());
    assert!(g4_lines(None, &BTreeMap::new()).is_empty());
    // KiCad's own rules count with no intent at all, absolute.
    assert_eq!(
        g4_lines(None, &counts(&[("skew_out_of_range", 1)])),
        vec!["KiCad: 1 skew_out_of_range violation(s) of the board's own rules"]
    );
}

#[test]
fn a_miss_that_rounds_to_zero_is_shown_with_the_digits_kicad_prints() {
    let measures = measures_with(
        vec![GroupLengths {
            interface: "ram".into(),
            name: "dq".into(),
            longest_mm: Some(42.0),
            nets: vec![net("a", Some(42.5001), 41.5, 42.5, 0.0001)],
        }],
        vec![],
        0.0001,
    );
    assert_eq!(
        g4_lines(Some(&measures), &BTreeMap::new()),
        vec!["group 'dq' (ram): a is 42.5001 mm, 0.0001 mm above its window 41.5000\u{2013}42.5000 mm"]
    );
}

#[test]
fn g4_lines_follow_a_real_check_and_flip_with_a_hundredth_of_a_millimetre() {
    let checks = checks_of(
        vec![group(
            "t",
            &["a"],
            Window::Target {
                target_mm: 42.0,
                tolerance_mm: 0.5,
            },
        )],
        vec![],
    );
    let lines = |mm: f64| {
        g4_lines(
            Some(&check(&checks, &lengths_of(&[("a", mm)]), vec![], 0.0)),
            &BTreeMap::new(),
        )
    };
    assert!(lines(42.49).is_empty());
    assert_eq!(lines(42.51).len(), 1);
    assert!(lines(42.51)[0].contains("0.01 mm above its window 41.50\u{2013}42.50 mm"));
}

#[test]
fn the_key_part_is_g4_then_the_miss_in_hundredths_then_kicads_violations() {
    let measures = measures_with(vec![], vec![], 0.624);
    assert_eq!(
        key_part(
            true,
            Some(&measures),
            &counts(&[("length_out_of_range", 2)])
        ),
        [1, 62, 2]
    );
    // The miss rounds to the nearest hundredth.
    assert_eq!(
        key_part(
            true,
            Some(&measures_with(vec![], vec![], 0.626)),
            &BTreeMap::new()
        )[1],
        63
    );
    assert_eq!(
        key_part(
            true,
            Some(&measures_with(vec![], vec![], 0.004)),
            &BTreeMap::new()
        )[1],
        0
    );
    // Nothing checked: zeros, whatever KiCad's own rules say elsewhere.
    assert_eq!(key_part(false, None, &BTreeMap::new()), [0, 0, 0]);
    // A net with no length sorts last, however small the rest of the miss.
    let mut unmeasured = measures_with(vec![], vec![], 0.0);
    unmeasured.unmeasured = names(&["a"]);
    assert_eq!(
        key_part(true, Some(&unmeasured), &BTreeMap::new())[1],
        i64::MAX
    );
    // KiCad's two keys are summed.
    assert_eq!(
        key_part(
            true,
            None,
            &counts(&[
                ("length_out_of_range", 1),
                ("skew_out_of_range", 4),
                ("clearance", 9)
            ])
        ),
        [1, 0, 5]
    );
}

// ----------------------------------------------- probe_lengths with a fake kicad-cli

#[cfg(unix)]
mod with_a_fake_kicad_cli {
    use super::*;
    use crate::tools::cli::test_support::write_script;

    /// Records `KICAD_CONFIG_HOME` and the arguments in `calls.log`, then copies the
    /// canned report (`report.json`, beside the script) to the `--output` path; a
    /// `fail` file beside it makes the script exit with 3.
    const FAKE_CLI: &str = r#"#!/bin/sh
dir="$(cd "$(dirname "$0")" && pwd)"
echo "KICAD_CONFIG_HOME=${KICAD_CONFIG_HOME-unset} $*" >> "$dir/calls.log"
if [ -f "$dir/fail" ]; then echo "boom" >&2; exit 3; fi
out=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--output" ]; then out="$a"; fi
  prev="$a"
done
cp "$dir/report.json" "$out"
exit 0
"#;

    struct Setup {
        dir: tempfile::TempDir,
        cli: String,
        judged: PathBuf,
        folder: PathBuf,
        config: PathBuf,
    }

    use std::path::PathBuf;

    const BOARD: &str = "(kicad_pcb (version 20260206) ; the judged board\n)\n";

    fn setup(report_text: &str) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("fake");
        std::fs::create_dir_all(&fake).unwrap();
        let cli = write_script(&fake, "kicad-cli", FAKE_CLI, "");
        std::fs::write(fake.join("report.json"), report_text).unwrap();
        // A judged board with the project files the judge copied beside it.
        let judged_dir = dir.path().join("run-0/r0");
        std::fs::create_dir_all(judged_dir.join("lib.pretty")).unwrap();
        let judged = judged_dir.join("board.kicad_pcb");
        std::fs::write(&judged, BOARD).unwrap();
        std::fs::write(
            judged_dir.join("board.kicad_pro"),
            json!({
                "board": { "design_settings": {
                    "rule_severities": { "clearance": "error", "length_out_of_range": "ignore" },
                    "rules": { "use_height_for_length_calcs": true }
                } }
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            judged_dir.join("board.kicad_dru"),
            "(version 1)\n(rule \"board_own\" (constraint skew (max 1mm)) (condition \"A.NetName == 'x'\"))\n",
        )
        .unwrap();
        std::fs::write(
            judged_dir.join("lib.pretty/a.kicad_mod"),
            "(footprint \"a\")",
        )
        .unwrap();
        let config = dir.path().join("kicad-config");
        std::fs::create_dir_all(&config).unwrap();
        Setup {
            cli: cli.display().to_string(),
            folder: dir.path().join("run-0/lengths"),
            judged,
            config,
            dir,
        }
    }

    fn calls(s: &Setup) -> Vec<String> {
        std::fs::read_to_string(s.dir.path().join("fake/calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[tokio::test]
    async fn the_probe_runs_on_a_copy_with_its_own_rules_and_the_given_settings() {
        let s = setup(REPORT_IGNORED);
        // Stale files of an earlier probe in the folder are gone.
        std::fs::create_dir_all(&s.folder).unwrap();
        std::fs::write(s.folder.join("stale.txt"), "x").unwrap();

        let nets = fixture_nets();
        let (lengths, unmeasured, seconds) =
            probe_lengths(&s.cli, Some(&s.config), &s.judged, &s.folder, &nets)
                .await
                .unwrap();
        assert_eq!(lengths.len(), 7);
        assert_eq!(lengths["S2"], 37.5011);
        assert!(unmeasured.is_empty());
        assert!(seconds >= 0.0);

        // The copy: the board's text, the probe's rules in place of the board's own,
        // the probe's project, and the libraries beside them.
        let board = s.folder.join("board.kicad_pcb");
        assert_eq!(std::fs::read_to_string(&board).unwrap(), BOARD);
        assert_eq!(
            std::fs::read_to_string(s.folder.join("board.kicad_dru")).unwrap(),
            probe_rules_text(&nets)
        );
        let listed =
            severities(&std::fs::read_to_string(s.folder.join("board.kicad_pro")).unwrap());
        assert_eq!(listed["clearance"], "ignore");
        assert_eq!(
            listed["length_out_of_range"], "error",
            "the project's ignore is undone"
        );
        assert!(s.folder.join("lib.pretty/a.kicad_mod").is_file());
        assert!(!s.folder.join("stale.txt").exists());

        // The judged board's own files are as they were.
        let own = std::fs::read_to_string(s.judged.with_extension("kicad_pro")).unwrap();
        assert_eq!(severities(&own)["length_out_of_range"], "ignore");
        assert!(
            std::fs::read_to_string(s.judged.with_extension("kicad_dru"))
                .unwrap()
                .contains("board_own")
        );

        // One kicad-cli run, on the given settings, as the probe's arguments say.
        let calls = calls(&s);
        assert_eq!(calls.len(), 1, "{calls:?}");
        let out = s.folder.join("board.length.json");
        assert_eq!(
            calls[0],
            format!(
                "KICAD_CONFIG_HOME={} pcb drc --output {} --format json --units mm {}",
                s.config.display(),
                out.display(),
                board.display()
            )
        );
    }

    #[tokio::test]
    async fn a_project_file_that_is_missing_is_left_missing() {
        let s = setup(REPORT_IGNORED);
        std::fs::remove_file(s.judged.with_extension("kicad_pro")).unwrap();
        probe_lengths(&s.cli, None, &s.judged, &s.folder, &fixture_nets())
            .await
            .unwrap();
        assert!(!s.folder.join("board.kicad_pro").exists());
        assert_eq!(calls(&s).len(), 1, "{:?}", calls(&s));
    }

    #[tokio::test]
    async fn a_probe_that_cannot_run_or_read_is_an_error() {
        // kicad-cli failing.
        let s = setup(REPORT_IGNORED);
        std::fs::write(s.dir.path().join("fake/fail"), "").unwrap();
        let error = probe_lengths(&s.cli, None, &s.judged, &s.folder, &fixture_nets())
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("length probe"), "{error:#}");

        // A report that is not JSON.
        let s = setup("this is not a report");
        assert!(
            probe_lengths(&s.cli, None, &s.judged, &s.folder, &fixture_nets())
                .await
                .is_err()
        );

        // A probe that cannot be trusted: two entries for one net.
        let twice = json!({
            "violations": [
                {"type": "length_out_of_range", "severity": "error", "description":
                    "Track length out of range (rule '__konnect_len_0001' min length 2000.0000 mm; actual 1.0000 mm)",
                 "items": [{"description": "Track [S1] on F.Cu"}]},
                {"type": "length_out_of_range", "severity": "error", "description":
                    "Track length out of range (rule '__konnect_len_0001' min length 2000.0000 mm; actual 2.0000 mm)",
                 "items": [{"description": "Via [S1] on F.Cu - B.Cu"}]}
            ],
            "unconnected_items": []
        })
        .to_string();
        let s = setup(&twice);
        let error = probe_lengths(&s.cli, None, &s.judged, &s.folder, &names(&["S1"]))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("twice"), "{error:#}");
    }
}

// ---------------------------------------------- the real kicad-cli (not in the gate)

/// Runs the probe with the real kicad-cli on a scratch copy of the public fixture
/// `specctra_pours_routing` (not routed: every net has pads and no track, so KiCad
/// reports each as 0.0000 mm, and a net that is not on the board has no entry).
/// Not run in the ordinary gate:
///
/// ```text
/// KICAD_CONFIG_HOME=<scratch folder> KONNECT_TEST_KICAD_CLI=<path to kicad-cli> \
///   cargo test -p konnect-core --lib the_probe_reads_a_real_kicad_cli -- --ignored --nocapture
/// ```
///
/// `KONNECT_TEST_KICAD_CLI` defaults to KiCad 10's path on macOS.
#[tokio::test]
#[ignore = "needs kicad-cli (KICAD_CONFIG_HOME must name a scratch folder)"]
async fn the_probe_reads_a_real_kicad_cli() {
    let config = std::env::var_os("KICAD_CONFIG_HOME")
        .expect("set KICAD_CONFIG_HOME to a scratch folder, or KiCad re-saves the real settings");
    let cli = std::env::var("KONNECT_TEST_KICAD_CLI")
        .unwrap_or_else(|_| "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli".to_string());
    let dir = tempfile::tempdir().unwrap();
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let judged_dir = dir.path().join("judged");
    std::fs::create_dir_all(&judged_dir).unwrap();
    for extension in ["kicad_pcb", "kicad_pro"] {
        std::fs::copy(
            fixtures.join(format!("specctra_pours_routing.{extension}")),
            judged_dir.join(format!("specctra_pours_routing.{extension}")),
        )
        .unwrap();
    }
    // The project ignores length violations: the probe must still see its own.
    let project = judged_dir.join("specctra_pours_routing.kicad_pro");
    let mut own: Value = serde_json::from_str(&std::fs::read_to_string(&project).unwrap()).unwrap();
    own["board"]["design_settings"]["rule_severities"] =
        json!({ "length_out_of_range": "ignore", "clearance": "ignore" });
    std::fs::write(&project, own.to_string()).unwrap();

    let nets = names(&["GND", "NOSUCHNET", "S1", "S6"]);
    let (lengths, unmeasured, seconds) = probe_lengths(
        &cli,
        Some(std::path::Path::new(&config)),
        &judged_dir.join("specctra_pours_routing.kicad_pcb"),
        &dir.path().join("lengths"),
        &nets,
    )
    .await
    .expect("the probe runs");
    eprintln!("lengths {lengths:?}, unmeasured {unmeasured:?}, {seconds:.2} s");
    for net in ["GND", "S1", "S6"] {
        assert_eq!(lengths.get(net), Some(&0.0), "{net}: pads and no track");
    }
    assert_eq!(unmeasured, names(&["NOSUCHNET"]));
}
