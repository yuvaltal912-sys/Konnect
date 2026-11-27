//! Tests of the length plan of a fastroute call (`route_lengths.rs`). Each guard has a
//! test that it holds and a counterpart on the honest case: that counterpart is the
//! guard's negative control.

use super::*;

#[test]
fn fastroute_is_given_half_of_a_declared_skew() {
    // L5 and L13: fastroute leaves alone a pair already inside its own window, so it
    // gets half and keeps the other half as the margin for the pad-edge gap.
    assert_eq!(
        FASTROUTE_WINDOW_SHARE, 0.5,
        "the share is pinned by this test"
    );
    assert_eq!(fastroute_skew_mm(0.3), 0.15);
    assert_eq!(fastroute_skew_mm(0.05), 0.025);
    assert_eq!(fastroute_skew_mm(1.0), 0.5);
    // The result is strictly inside the declared maximum, whatever it is.
    for max in [0.001, 0.05, 0.1, 2.5, 100.0] {
        let given = fastroute_skew_mm(max);
        assert!(given > 0.0 && given < max, "{max}: {given}");
    }
}

// ─── Test helpers ───────────────────────────────────────────────────────────

use crate::length_probe::{NetLength, PairSkew};
use crate::tools::route_pairs::{ResolvedPair, UnresolvedPair};

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| name.to_string()).collect()
}

fn set(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|name| name.to_string()).collect()
}

fn declared(
    interface: &str,
    name: &str,
    nets: &[&str],
    tolerance_mm: f64,
    target_mm: Option<f64>,
) -> DeclaredGroup {
    DeclaredGroup {
        interface: interface.to_string(),
        name: name.to_string(),
        nets: strings(nets),
        tolerance_mm,
        target_mm,
    }
}

fn declared_skew(positive: &str, negative: &str, max_skew_mm: f64) -> DeclaredSkew {
    DeclaredSkew {
        positive: positive.to_string(),
        negative: negative.to_string(),
        max_skew_mm,
    }
}

fn resolved_pair(
    positive: &str,
    negative: &str,
    dsn_positive: &str,
    dsn_negative: &str,
    max_skew_mm: Option<f64>,
) -> ResolvedPair {
    ResolvedPair {
        positive: positive.to_string(),
        negative: negative.to_string(),
        dsn_positive: dsn_positive.to_string(),
        dsn_negative: dsn_negative.to_string(),
        gap_mm: None,
        max_skew_mm,
    }
}

fn pairs_of(resolved: Vec<ResolvedPair>) -> PairsPlan {
    PairsPlan {
        resolved,
        ..PairsPlan::default()
    }
}

/// A group built directly, for the tests of what is done with a resolved one.
fn group(
    interface: &str,
    name: &str,
    file_name: &str,
    nets: &[(&str, &str)],
    window: Window,
    pairs: &[(&str, &str, f64)],
) -> ResolvedGroup {
    ResolvedGroup {
        interface: interface.to_string(),
        name: name.to_string(),
        file_name: file_name.to_string(),
        nets: nets
            .iter()
            .map(|(intent, dsn)| GroupNet {
                intent: intent.to_string(),
                dsn: dsn.to_string(),
            })
            .collect(),
        window,
        pairs: pairs
            .iter()
            .map(|(p, n, t)| (p.to_string(), n.to_string(), *t))
            .collect(),
    }
}

/// A board of footprints and zones, as KiCad 10 writes nets (named in place).
fn board_of(parts: &[String]) -> String {
    format!(
        "(kicad_pcb (version 20260206) (generator \"test\")\n{}\n)\n",
        parts.join("\n")
    )
}

fn pad(number: &str, net: &str, dx: f64, width: f64, height: f64) -> String {
    format!(
        "(pad \"{number}\" smd rect (at {dx} 0) (size {width} {height}) (layers \"F.Cu\") (net \"{net}\"))"
    )
}

fn footprint(x: f64, y: f64, pads: &[String]) -> String {
    format!(
        "(footprint \"T\" (layer \"F.Cu\") (at {x} {y} 0) {})",
        pads.join(" ")
    )
}

fn zone_of(net: &str) -> String {
    format!("(zone (net \"{net}\") (layer \"F.Cu\") (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10))))")
}

// ─── The window ─────────────────────────────────────────────────────────────

#[test]
fn a_window_gives_fastroute_its_share_and_kicad_the_whole_tolerance() {
    // L5: skew mode is `tolerance = T/2`, with no target.
    let skew = Window::Skew { tolerance_mm: 0.5 };
    assert_eq!(skew.fastroute_values(), (None, 0.25));
    // Target mode is `target = L + T/2` and `tolerance = T`.
    let target = Window::Target {
        target_mm: 42.0,
        tolerance_mm: 0.5,
    };
    assert_eq!(target.fastroute_values(), (Some(42.25), 0.5));
    // KiCad's own window is the whole tolerance (L2).
    assert_eq!(skew.range_mm(42.0), (41.5, 42.0));
    assert_eq!(target.range_mm(0.0), (41.5, 42.5));
    // The target window does not read the longest net; the skew window does.
    assert_eq!(target.range_mm(99.0), (41.5, 42.5));
    assert_eq!(skew.range_mm(10.0), (9.5, 10.0));
}

// ─── Declared groups and skews ──────────────────────────────────────────────

fn intent_with(interfaces: Value) -> DesignIntent {
    serde_json::from_value(json!({
        "schema_version": 1, "rails": [], "currents": [],
        "interfaces": interfaces, "fab_target": {}
    }))
    .unwrap()
}

#[test]
fn groups_and_skews_are_read_from_every_interface_in_file_order() {
    let intent = intent_with(json!([
        { "name": "ram", "kind": "ddr", "nets": ["D0", "D1", "DQS_P", "DQS_N"],
          "differential_pairs": [
              { "positive": "DQS_P", "negative": "DQS_N", "max_skew_mm": 0.2 }],
          "length_groups": [
              { "name": "byte0", "nets": ["D0", "D1", "DQS_P", "DQS_N"], "tolerance_mm": 0.5 }] },
        { "name": "spi", "kind": "spi", "nets": ["CLK", "MOSI"],
          "differential_pairs": [{ "positive": "CLK", "negative": "MOSI" }],
          "length_groups": [
              { "name": "bus", "nets": ["CLK", "MOSI"], "tolerance_mm": 1.0, "target_mm": 30.0 }] }
    ]));
    assert_eq!(
        declared_groups(&intent),
        vec![
            declared("ram", "byte0", &["D0", "D1", "DQS_P", "DQS_N"], 0.5, None),
            declared("spi", "bus", &["CLK", "MOSI"], 1.0, Some(30.0)),
        ]
    );
    // A pair without max_skew_mm is not a declared skew.
    assert_eq!(
        declared_skews(&intent),
        vec![declared_skew("DQS_P", "DQS_N", 0.2)]
    );
    // An intent that declares nothing gives nothing.
    let bare = intent_with(json!([{ "name": "x", "kind": "x", "nets": ["A"] }]));
    assert!(declared_groups(&bare).is_empty() && declared_skews(&bare).is_empty());
}

#[test]
fn an_unusable_intent_declares_lengths_only_by_its_keys() {
    assert!(declares_lengths(
        r#"{ "interfaces": [{ "length_groups": 1 }] }"#
    ));
    assert!(declares_lengths(r#"{"max_skew_mm":"a"}"#));
    assert!(declares_lengths("{ not json \"length_groups\" "));
    // The test is on the text, so a value that is exactly the key's name counts too:
    // refusing is the safe side.
    assert!(declares_lengths(r#"{ "name": "max_skew_mm" }"#));
    // Control: neither key, however much the text talks about lengths.
    assert!(!declares_lengths(
        r#"{ "interfaces": [{ "notes": "length groups" }] }"#
    ));
    assert!(!declares_lengths("{ not json"));
    // A mention inside a string is escaped, so it is not the key.
    assert!(!declares_lengths(
        r#"{ "notes": "the \"length_groups\" and \"max_skew_mm\" keys" }"#
    ));
}

#[test]
fn a_tune_token_is_one_word_without_the_characters_fastroute_and_kicad_read() {
    for good in ["/DQ0", "DQ0", "/sheet/D+", "NET-1", "a.b", "/CLK_P", "ü1"] {
        assert!(tune_token(good), "{good}");
    }
    for bad in [
        "", "a b", "a\tb", "a\nb", "a#b", "a*b", "a?b", "a'b", "a\"b", "a\\b", " a", "a\u{0}",
    ] {
        assert!(!tune_token(bad), "{bad:?}");
    }
    for c in UNSAFE_NAME_CHARS {
        assert!(!tune_token(&format!("a{c}b")), "{c}");
    }
}

#[test]
fn numbers_in_files_have_no_float_noise_and_no_trailing_zero() {
    assert_eq!(mm_text(0.25), "0.25");
    assert_eq!(mm_text(42.0), "42");
    assert_eq!(mm_text(10.0), "10");
    assert_eq!(mm_text(100.0), "100");
    assert_eq!(mm_text(42.07 + 0.5 * 0.5), "42.32");
    assert_eq!(mm_text(0.1 + 0.2), "0.3");
    assert_eq!(mm_text(0.0), "0");
    assert_eq!(mm_text(-0.0000001), "0");
    assert_eq!(mm_text(0.000_001), "0.000001");
}

// ─── Names (L4) ─────────────────────────────────────────────────────────────

fn dsn_nets() -> Vec<String> {
    strings(&["/DQ0", "/DQ1", "/DQ2", "/a/CLK", "/b/CLK", "GND"])
}

fn board_nets() -> BTreeSet<String> {
    dsn_nets().into_iter().collect()
}

/// Every refusal names the group (or the pair), the net, and a step that fixes it.
fn assert_refusal(
    result: Result<(Vec<ResolvedGroup>, Vec<GatedPair>), LengthRefusal>,
    mentions: &[&str],
    fix: &str,
) {
    let refusal = result.expect_err("the call is refused");
    for word in mentions {
        assert!(
            refusal.reason.contains(word),
            "reason lacks {word}: {}",
            refusal.reason
        );
    }
    assert!(
        refusal.next_step.contains(fix),
        "next step lacks {fix}: {}",
        refusal.next_step
    );
}

#[test]
fn names_that_resolve_give_the_plan_its_groups_with_their_dsn_nets_and_file_names() {
    // The exact name, and the one net ending in `/name`, as D23 resolves a pair's.
    let groups = [
        declared("ram", "dq", &["DQ0", "/DQ1"], 0.5, None),
        declared("ram", "dq_hi", &["DQ2"], 0.25, Some(40.0)),
    ];
    let (resolved, gated) = resolve(
        &groups,
        &[],
        &dsn_nets(),
        &board_nets(),
        &PairsPlan::default(),
    )
    .unwrap();
    assert!(gated.is_empty());
    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[0].file_name, "lt1");
    assert_eq!(resolved[1].file_name, "lt2");
    assert_eq!(resolved[0].interface, "ram");
    assert_eq!(resolved[0].name, "dq");
    assert_eq!(
        resolved[0].nets,
        vec![
            GroupNet {
                intent: "DQ0".into(),
                dsn: "/DQ0".into()
            },
            GroupNet {
                intent: "/DQ1".into(),
                dsn: "/DQ1".into()
            },
        ]
    );
    assert_eq!(resolved[0].window, Window::Skew { tolerance_mm: 0.5 });
    assert_eq!(
        resolved[1].window,
        Window::Target {
            target_mm: 40.0,
            tolerance_mm: 0.25
        }
    );
    assert_eq!(resolved[1].nets[0].dsn, "/DQ2");
    assert!(resolved.iter().all(|g| g.pairs.is_empty()));
}

#[test]
fn an_unresolved_or_ambiguous_name_refuses_naming_group_net_and_fix() {
    let nets = dsn_nets();
    let board = board_nets();
    let none = PairsPlan::default();
    assert_refusal(
        resolve(
            &[declared("ram", "dq", &["DQ0", "DQ9"], 0.5, None)],
            &[],
            &nets,
            &board,
            &none,
        ),
        &["'dq'", "'ram'", "'DQ9'", "no net of the DSN"],
        "set_design_intent",
    );
    assert_refusal(
        resolve(
            &[declared("ram", "clk", &["CLK", "DQ0"], 0.5, None)],
            &[],
            &nets,
            &board,
            &none,
        ),
        &["'clk'", "'CLK'", "ambiguous", "/a/CLK"],
        "sheet path",
    );
    // Controls: the same groups with resolvable names plan.
    assert!(resolve(
        &[declared("ram", "dq", &["DQ0", "DQ1"], 0.5, None)],
        &[],
        &nets,
        &board,
        &none
    )
    .is_ok());
    assert!(resolve(
        &[declared("ram", "clk", &["/a/CLK", "DQ0"], 0.5, None)],
        &[],
        &nets,
        &board,
        &none
    )
    .is_ok());
    // The first group that cannot be applied is the one named.
    assert_refusal(
        resolve(
            &[
                declared("ram", "good", &["DQ0"], 0.5, None),
                declared("ram", "bad", &["NOPE"], 0.5, None),
                declared("ram", "worse", &["NADA"], 0.5, None),
            ],
            &[],
            &nets,
            &board,
            &none,
        ),
        &["'bad'", "'NOPE'"],
        "set_design_intent",
    );
}

#[test]
fn a_resolved_name_the_tune_file_cannot_carry_refuses() {
    let none = PairsPlan::default();
    for bad in ["a b", "a#b", "a*b", "a?b", "a'b", "a\"b", "a\\b", "a\tb"] {
        let nets = strings(&[bad, "/DQ0"]);
        let board: BTreeSet<String> = nets.iter().cloned().collect();
        // The name as the DSN writes it resolves exactly, then is refused.
        let result = resolve(
            &[declared("ram", "dq", &[bad, "DQ0"], 0.5, None)],
            &[],
            &nets,
            &board,
            &none,
        );
        let refusal = result.expect_err(bad);
        assert!(
            refusal.reason.contains("'dq'") && refusal.reason.contains("white space"),
            "{bad:?}: {}",
            refusal.reason
        );
        assert!(refusal.next_step.contains("Rename the net"), "{bad:?}");
    }
    // Control: a name with none of them is applied.
    let nets = strings(&["a_b", "/DQ0"]);
    let board: BTreeSet<String> = nets.iter().cloned().collect();
    assert!(resolve(
        &[declared("ram", "dq", &["a_b", "DQ0"], 0.5, None)],
        &[],
        &nets,
        &board,
        &none
    )
    .is_ok());
}

#[test]
fn why_the_tune_file_needs_one_word_per_net() {
    // fastroute splits a net line at white space: a net named `a b` is read as two.
    let parsed = fastroute_parse_tune("group lt1 tolerance=1\n  a b\n").unwrap();
    assert_eq!(parsed[0].nets, strings(&["a", "b"]));
    // And `#` ends the line, so the rest of the name is lost.
    let parsed = fastroute_parse_tune("group lt1 tolerance=1\n  a#b\n").unwrap();
    assert_eq!(parsed[0].nets, strings(&["a"]));
}

#[test]
fn a_dsn_net_that_is_not_a_net_of_the_board_refuses() {
    // The DSN names `/DQ0` and the saved board does not: the session's net could not
    // be matched back.
    let board: BTreeSet<String> = set(&["/DQ1", "GND"]);
    assert_refusal(
        resolve(
            &[declared("ram", "dq", &["DQ0"], 0.5, None)],
            &[],
            &dsn_nets(),
            &board,
            &PairsPlan::default(),
        ),
        &["'dq'", "'DQ0'", "not a net of the saved board"],
        "export_specctra_dsn",
    );
    // Control: with the net on the board it plans.
    assert!(resolve(
        &[declared("ram", "dq", &["DQ0"], 0.5, None)],
        &[],
        &dsn_nets(),
        &board_nets(),
        &PairsPlan::default()
    )
    .is_ok());
}

#[test]
fn a_checked_net_equal_to_another_board_net_ignoring_case_refuses() {
    // KiCad's `==` ignores letter case: a probe rule for `/Clk` would also catch `/CLK`.
    let nets = strings(&["/Clk", "/CLK", "/DQ0"]);
    let board: BTreeSet<String> = nets.iter().cloned().collect();
    assert_refusal(
        resolve(
            &[declared("bus", "clk", &["/Clk"], 0.5, None)],
            &[],
            &nets,
            &board,
            &PairsPlan::default(),
        ),
        &["'clk'", "'/Clk'", "'/CLK'", "letter case"],
        "Rename one of the two nets",
    );
    // Control: without the other net the same group plans.
    let alone: BTreeSet<String> = set(&["/Clk", "/DQ0"]);
    assert!(resolve(
        &[declared("bus", "clk", &["/Clk"], 0.5, None)],
        &[],
        &strings(&["/Clk", "/DQ0"]),
        &alone,
        &PairsPlan::default()
    )
    .is_ok());
    // A net that merely contains another's name is no collision.
    let longer: BTreeSet<String> = set(&["/Clk", "/Clk2"]);
    assert!(resolve(
        &[declared("bus", "clk", &["/Clk"], 0.5, None)],
        &[],
        &strings(&["/Clk", "/Clk2"]),
        &longer,
        &PairsPlan::default()
    )
    .is_ok());
}

#[test]
fn two_names_for_one_net_refuse_because_a_net_has_one_window() {
    // `DQ0` and `/DQ0` resolve to the one DSN net, in one group and across two.
    assert_refusal(
        resolve(
            &[declared("ram", "dq", &["DQ0", "/DQ0"], 0.5, None)],
            &[],
            &dsn_nets(),
            &board_nets(),
            &PairsPlan::default(),
        ),
        &["'dq'", "'/DQ0'", "already in the length group 'dq'"],
        "once",
    );
    assert_refusal(
        resolve(
            &[
                declared("ram", "lo", &["DQ0", "DQ1"], 0.5, None),
                declared("ram", "hi", &["/DQ0", "DQ2"], 0.5, None),
            ],
            &[],
            &dsn_nets(),
            &board_nets(),
            &PairsPlan::default(),
        ),
        &["'hi'", "'/DQ0'", "already in the length group 'lo'"],
        "once",
    );
    // Control: distinct nets in the two groups plan.
    assert!(resolve(
        &[
            declared("ram", "lo", &["DQ0", "DQ1"], 0.5, None),
            declared("ram", "hi", &["DQ2"], 0.5, None),
        ],
        &[],
        &dsn_nets(),
        &board_nets(),
        &PairsPlan::default()
    )
    .is_ok());
}

#[test]
fn a_pair_split_across_groups_refuses_on_the_resolved_names() {
    // The pair is written `/DQ0`, `/DQ1`; the groups write the same nets `DQ0` and
    // `DQ1`, so the intent's own check, comparing spellings, cannot see the split.
    let pairs = pairs_of(vec![resolved_pair("/DQ0", "/DQ1", "/DQ0", "/DQ1", None)]);
    // One net of the pair in a group, the other in none.
    assert_refusal(
        resolve(
            &[declared("ram", "dq", &["DQ0", "DQ2"], 0.5, None)],
            &[],
            &dsn_nets(),
            &board_nets(),
            &pairs,
        ),
        &[
            "'/DQ0' and '/DQ1'",
            "'/DQ0' is in the length group 'dq' of interface 'ram'",
            "'/DQ1' is in no length group",
        ],
        "same length group",
    );
    // The two nets in two groups.
    assert_refusal(
        resolve(
            &[
                declared("ram", "lo", &["DQ0", "DQ2"], 0.5, None),
                declared("ram", "hi", &["DQ1", "/a/CLK"], 0.5, None),
            ],
            &[],
            &dsn_nets(),
            &board_nets(),
            &pairs,
        ),
        &["'lo'", "'hi'", "split"],
        "set_design_intent",
    );
    // Controls: both in one group, or in none, plan.
    for groups in [
        vec![declared("ram", "dq", &["DQ0", "DQ1"], 0.5, None)],
        vec![declared("ram", "other", &["DQ2", "/a/CLK"], 0.5, None)],
        vec![],
    ] {
        assert!(
            resolve(&groups, &[], &dsn_nets(), &board_nets(), &pairs).is_ok(),
            "{groups:?}"
        );
    }
}

#[test]
fn a_skew_on_a_pair_the_pairs_plan_left_out_refuses_with_its_reason() {
    let skews = [declared_skew("DQS_P", "DQS_N", 0.2)];
    // Left out by D23, with its reason.
    let left_out = PairsPlan {
        unresolved: vec![UnresolvedPair {
            positive: "DQS_P".into(),
            negative: "DQS_N".into(),
            reason: "no net of the DSN is named 'DQS_N'".into(),
        }],
        ..PairsPlan::default()
    };
    assert_refusal(
        resolve(&[], &skews, &dsn_nets(), &board_nets(), &left_out),
        &[
            "'DQS_P'",
            "'DQS_N'",
            "max_skew_mm 0.2",
            "no net of the DSN is named",
        ],
        "set_design_intent",
    );
    // Not in the pairs plan at all (no pair was read from the intent).
    assert_refusal(
        resolve(
            &[],
            &skews,
            &dsn_nets(),
            &board_nets(),
            &PairsPlan::default(),
        ),
        &["'DQS_P'", "not among the pairs routed coupled"],
        "set_design_intent",
    );
    // A resolved pair of other nets does not stand in for it.
    let other = pairs_of(vec![resolved_pair("X", "Y", "/DQ0", "/DQ1", None)]);
    assert_refusal(
        resolve(&[], &skews, &dsn_nets(), &board_nets(), &other),
        &["'DQS_P'", "not among the pairs routed coupled"],
        "set_design_intent",
    );
    // Control: the same skew on a resolved pair is gated.
    let nets = strings(&["/DQS_P", "/DQS_N", "/DQ0"]);
    let board: BTreeSet<String> = nets.iter().cloned().collect();
    let pairs = pairs_of(vec![resolved_pair(
        "DQS_P",
        "DQS_N",
        "/DQS_P",
        "/DQS_N",
        Some(0.2),
    )]);
    let (groups, gated) = resolve(&[], &skews, &nets, &board, &pairs).unwrap();
    assert!(groups.is_empty());
    assert_eq!(
        gated,
        vec![GatedPair {
            positive: "DQS_P".into(),
            negative: "DQS_N".into(),
            dsn_positive: "/DQS_P".into(),
            dsn_negative: "/DQS_N".into(),
            max_skew_mm: 0.2,
            group: None,
        }]
    );
}

#[test]
fn a_gated_pair_gets_the_checks_a_group_net_gets() {
    let skews = [declared_skew("P", "N", 0.2)];
    let pairs =
        |dsn_p: &str, dsn_n: &str| pairs_of(vec![resolved_pair("P", "N", dsn_p, dsn_n, Some(0.2))]);
    // A pair net the tune file cannot carry.
    let nets = strings(&["p q", "N"]);
    let board: BTreeSet<String> = nets.iter().cloned().collect();
    assert_refusal(
        resolve(&[], &skews, &nets, &board, &pairs("p q", "N")),
        &["'P' and 'N'", "'P'", "white space"],
        "Rename the net",
    );
    // A pair net the board does not have.
    let nets = strings(&["P", "N"]);
    let board = set(&["P"]);
    assert_refusal(
        resolve(&[], &skews, &nets, &board, &pairs("P", "N")),
        &["'N'", "not a net of the saved board"],
        "export_specctra_dsn",
    );
    // A pair net equal to another net ignoring case.
    let nets = strings(&["P", "N", "n"]);
    let board: BTreeSet<String> = nets.iter().cloned().collect();
    assert_refusal(
        resolve(&[], &skews, &nets, &board, &pairs("P", "N")),
        &["'N'", "'n'", "letter case"],
        "Rename one",
    );
    // Control: clean names are gated.
    let nets = strings(&["P", "N"]);
    let board: BTreeSet<String> = nets.iter().cloned().collect();
    assert!(resolve(&[], &skews, &nets, &board, &pairs("P", "N")).is_ok());
}

#[test]
fn a_pair_inside_a_group_is_listed_in_it_and_its_skew_is_gated_with_the_group() {
    let nets = strings(&["/D0", "/D1", "/S_P", "/S_N", "/C_P", "/C_N"]);
    let board: BTreeSet<String> = nets.iter().cloned().collect();
    let pairs = pairs_of(vec![
        // With a declared maximum, and one without: 0.1 mm for the trailing group.
        resolved_pair("S_P", "S_N", "/S_P", "/S_N", Some(0.3)),
        resolved_pair("C_P", "C_N", "/C_P", "/C_N", None),
    ]);
    let (resolved, gated) = resolve(
        &[
            declared("lane", "byte", &["D0", "D1", "S_P", "S_N"], 0.5, None),
            declared("lane", "clock", &["C_P", "C_N"], 1.0, Some(20.0)),
        ],
        &[declared_skew("S_P", "S_N", 0.3)],
        &nets,
        &board,
        &pairs,
    )
    .unwrap();
    assert_eq!(
        resolved[0].pairs,
        vec![("/S_P".to_string(), "/S_N".to_string(), 0.15)]
    );
    assert_eq!(
        resolved[1].pairs,
        vec![(
            "/C_P".to_string(),
            "/C_N".to_string(),
            FASTROUTE_PAIR_SKEW_MM
        )]
    );
    assert_eq!(gated.len(), 1);
    assert_eq!(gated[0].group.as_deref(), Some("byte"));
    // A pair with one net in a group and one outside is refused (L1a, on DSN names).
    let lopsided = resolve(
        &[declared("lane", "half", &["S_P"], 0.5, None)],
        &[],
        &nets,
        &board,
        &pairs,
    );
    assert_refusal(
        lopsided,
        &["'S_P'", "'S_N'", "'half'", "no length group"],
        "same length group",
    );
}

#[test]
fn a_skew_is_matched_to_its_own_pair_when_pairs_share_a_net() {
    // Three pairs over three nets: each skew names two of them, and any one net also
    // belongs to another pair.
    let pairs = pairs_of(vec![
        resolved_pair("DQ0", "DQ1", "/DQ0", "/DQ1", Some(0.1)),
        resolved_pair("DQ0", "DQ2", "/DQ0", "/DQ2", Some(0.2)),
        resolved_pair("DQ1", "DQ2", "/DQ1", "/DQ2", Some(0.3)),
    ]);
    let groups = [
        declared("clk", "clocks", &["/a/CLK", "/b/CLK"], 0.5, None),
        declared("bus", "bytes", &["DQ0", "DQ1", "DQ2"], 0.5, None),
    ];
    let skews = [
        declared_skew("DQ0", "DQ2", 0.2),
        declared_skew("DQ1", "DQ2", 0.3),
        declared_skew("DQ0", "DQ1", 0.1),
    ];
    let (_, gated) = resolve(&groups, &skews, &dsn_nets(), &board_nets(), &pairs).unwrap();
    let seen: Vec<(&str, &str, &str, &str, Option<&str>)> = gated
        .iter()
        .map(|pair| {
            (
                pair.positive.as_str(),
                pair.negative.as_str(),
                pair.dsn_positive.as_str(),
                pair.dsn_negative.as_str(),
                pair.group.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        vec![
            ("DQ0", "DQ2", "/DQ0", "/DQ2", Some("bytes")),
            ("DQ1", "DQ2", "/DQ1", "/DQ2", Some("bytes")),
            ("DQ0", "DQ1", "/DQ0", "/DQ1", Some("bytes")),
        ]
    );
    assert_eq!(
        gated
            .iter()
            .map(|pair| pair.max_skew_mm)
            .collect::<Vec<_>>(),
        vec![0.2, 0.3, 0.1]
    );
    // A pair that shares only one net with a resolved pair is not that pair.
    for (positive, negative) in [("DQ0", "/a/CLK"), ("/b/CLK", "DQ2"), ("DQ1", "DQ0")] {
        let reason = resolve(
            &[],
            &[declared_skew(positive, negative, 0.2)],
            &dsn_nets(),
            &board_nets(),
            &pairs,
        )
        .expect_err("a pair that is not in the pairs plan")
        .reason;
        assert!(
            reason.contains("not among the pairs routed coupled"),
            "{positive} {negative}: {reason}"
        );
    }
    // Nor does the reason of a left-out pair stand in for another one.
    let left_out = |positive: &str, negative: &str, reason: &str| UnresolvedPair {
        positive: positive.into(),
        negative: negative.into(),
        reason: reason.into(),
    };
    let unresolved = PairsPlan {
        unresolved: vec![
            left_out("DQ0", "DQ1", "reason one"),
            left_out("DQ0", "DQ2", "reason two"),
        ],
        ..PairsPlan::default()
    };
    let skew = |positive: &str, negative: &str| {
        resolve(
            &[],
            &[declared_skew(positive, negative, 0.2)],
            &dsn_nets(),
            &board_nets(),
            &unresolved,
        )
        .expect_err("left out")
        .reason
    };
    let reason = skew("DQ0", "DQ2");
    assert!(
        reason.contains("(reason two)") && !reason.contains("reason one"),
        "{reason}"
    );
    let reason = skew("DQ1", "DQ2");
    assert!(
        reason.contains("not among the pairs routed coupled") && !reason.contains("reason"),
        "{reason}"
    );
    let reason = skew("DQ0", "GND");
    assert!(
        reason.contains("not among the pairs routed coupled") && !reason.contains("reason"),
        "{reason}"
    );
}

#[test]
fn a_pair_outside_every_group_is_gated_with_no_group() {
    // Neither net of the pair is in a group while other groups exist.
    let pairs = pairs_of(vec![resolved_pair("DQ0", "DQ1", "/DQ0", "/DQ1", Some(0.2))]);
    let (_, gated) = resolve(
        &[declared("clk", "clocks", &["/a/CLK", "/b/CLK"], 0.5, None)],
        &[declared_skew("DQ0", "DQ1", 0.2)],
        &dsn_nets(),
        &board_nets(),
        &pairs,
    )
    .unwrap();
    assert_eq!(gated.len(), 1);
    assert_eq!(gated[0].group, None);
}

// ─── The tune file (L5) ─────────────────────────────────────────────────────

/// A copy of the rules of fastroute's `parse_tune_file` (`crates/fastroute/src/main.rs`),
/// kept in step by hand: `#` starts a comment, a line is split at white space, a
/// `group NAME key=value…` line opens a group (keys `tolerance` and `target`, a value
/// may end in `mm`), and every other line adds its words as net patterns.
#[derive(Debug, PartialEq)]
struct ParsedGroup {
    name: String,
    nets: Vec<String>,
    tolerance_mm: f64,
    target_mm: Option<f64>,
}

fn fastroute_parse_tune(text: &str) -> Result<Vec<ParsedGroup>, String> {
    let mut groups: Vec<ParsedGroup> = Vec::new();
    for (no, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut words = line.split_whitespace();
        if line.starts_with("group ") {
            words.next();
            let name = words
                .next()
                .ok_or(format!("{}: group needs a name", no + 1))?
                .to_string();
            let mut group = ParsedGroup {
                name,
                nets: Vec::new(),
                tolerance_mm: 0.5,
                target_mm: None,
            };
            for word in words {
                let (key, value) = word
                    .split_once('=')
                    .ok_or(format!("{}: expected key=value, got '{word}'", no + 1))?;
                let value: f64 = value
                    .trim_end_matches("mm")
                    .parse()
                    .map_err(|_| format!("{}: bad number '{value}'", no + 1))?;
                match key {
                    "tolerance" => group.tolerance_mm = value,
                    "target" => group.target_mm = Some(value),
                    _ => return Err(format!("{}: unknown key '{key}'", no + 1)),
                }
            }
            groups.push(group);
        } else {
            let group = groups
                .last_mut()
                .ok_or(format!("{}: net before the first group line", no + 1))?;
            group.nets.extend(words.map(str::to_string));
        }
    }
    Ok(groups)
}

fn two_group_plan() -> LengthPlan {
    LengthPlan {
        groups: vec![
            group(
                "ram",
                "dq_low",
                "lt1",
                &[
                    ("Data0", "/DQ0"),
                    ("Data1", "/DQ1"),
                    ("Strobe+", "/DQS_P"),
                    ("Strobe-", "/DQS_N"),
                ],
                Window::Skew { tolerance_mm: 0.5 },
                &[("/DQS_P", "/DQS_N", 0.15)],
            ),
            group(
                "ram",
                "addr",
                "lt2",
                &[("A0", "/A0"), ("A1", "/A1")],
                Window::Target {
                    target_mm: 42.0,
                    tolerance_mm: 0.5,
                },
                &[],
            ),
        ],
        ..LengthPlan::default()
    }
}

#[test]
fn the_tune_file_gives_fastroute_half_the_tolerance_and_trailing_pair_groups_last() {
    let text = two_group_plan().tune_file_text().unwrap();
    assert_eq!(
        text,
        "# Length groups for fastroute --tune, written by Konnect (DSN net names only).\n\
         group lt1 tolerance=0.25\n  /DQ0\n  /DQ1\n  /DQS_P\n  /DQS_N\n\
         group lt2 tolerance=0.5 target=42.25\n  /A0\n  /A1\n\
         group lt1p1 tolerance=0.15\n  /DQS_P\n  /DQS_N\n"
    );
    // The share is the one the intent's tolerance is cut by: with it at 1 the
    // lines would read `tolerance=0.5` and `target=42.5 tolerance=1`.
    assert_eq!(FASTROUTE_WINDOW_SHARE, 0.5);
    assert!(!text.contains("tolerance=1\n") && !text.contains("target=42.5"));
}

#[test]
fn a_pair_without_a_declared_skew_gets_fastroutes_own_default_in_its_trailing_group() {
    let mut plan = two_group_plan();
    plan.groups[0].pairs = vec![("/DQS_P".into(), "/DQS_N".into(), FASTROUTE_PAIR_SKEW_MM)];
    let text = plan.tune_file_text().unwrap();
    assert!(
        text.ends_with("group lt1p1 tolerance=0.1\n  /DQS_P\n  /DQS_N\n"),
        "{text}"
    );
}

#[test]
fn several_pairs_in_a_group_get_numbered_trailing_groups_after_every_group() {
    let mut plan = two_group_plan();
    plan.groups[0].pairs = vec![
        ("/DQS_P".into(), "/DQS_N".into(), 0.15),
        ("/DQ0".into(), "/DQ1".into(), 0.05),
    ];
    plan.groups[1].pairs = vec![("/A0".into(), "/A1".into(), 0.1)];
    let text = plan.tune_file_text().unwrap();
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("group "))
        .collect();
    assert_eq!(
        lines,
        vec![
            "group lt1 tolerance=0.25",
            "group lt2 tolerance=0.5 target=42.25",
            "group lt1p1 tolerance=0.15",
            "group lt1p2 tolerance=0.05",
            "group lt2p1 tolerance=0.1",
        ]
    );
}

#[test]
fn the_tune_file_holds_dsn_names_only_never_intent_text() {
    let text = two_group_plan().tune_file_text().unwrap();
    // The intent called the nets `Data0` and `Strobe+`, the group `dq_low` and the
    // interface `ram`; none of that is in the file.
    for word in ["Data0", "Data1", "Strobe", "dq_low", "ram", "addr", "ram"] {
        assert!(!text.contains(word), "{word} is in {text}");
    }
    assert!(text.contains("/DQ0") && text.contains("lt2"));
}

#[test]
fn fastroute_reads_the_tune_file_as_written() {
    let text = two_group_plan().tune_file_text().unwrap();
    let parsed = fastroute_parse_tune(&text).unwrap();
    assert_eq!(
        parsed,
        vec![
            ParsedGroup {
                name: "lt1".into(),
                nets: strings(&["/DQ0", "/DQ1", "/DQS_P", "/DQS_N"]),
                tolerance_mm: 0.25,
                target_mm: None,
            },
            ParsedGroup {
                name: "lt2".into(),
                nets: strings(&["/A0", "/A1"]),
                tolerance_mm: 0.5,
                target_mm: Some(42.25),
            },
            ParsedGroup {
                name: "lt1p1".into(),
                nets: strings(&["/DQS_P", "/DQS_N"]),
                tolerance_mm: 0.15,
                target_mm: None,
            },
        ]
    );
    // The copy of the rules does reject what it should (it is not a parser that
    // accepts anything): an unknown key, a net before a group, a bad number.
    assert!(fastroute_parse_tune("group a bogus=1\n").is_err());
    assert!(fastroute_parse_tune("/DQ0\n").is_err());
    assert!(fastroute_parse_tune("group a tolerance=x\n").is_err());
    assert!(fastroute_parse_tune("group a tolerance 1\n").is_err());
}

#[test]
fn float_noise_never_reaches_the_tune_file() {
    let plan = LengthPlan {
        groups: vec![group(
            "i",
            "g",
            "lt1",
            &[("a", "/a"), ("b", "/b")],
            Window::Target {
                target_mm: 42.1,
                tolerance_mm: 0.07,
            },
            &[],
        )],
        ..LengthPlan::default()
    };
    let text = plan.tune_file_text().unwrap();
    assert!(
        text.contains("group lt1 tolerance=0.07 target=42.135\n"),
        "{text}"
    );
    let parsed = fastroute_parse_tune(&text).unwrap();
    assert_eq!(parsed[0].target_mm, Some(42.135));
}

#[test]
fn without_a_group_there_is_no_tune_file_and_no_net_to_tune() {
    let empty = LengthPlan::default();
    assert!(empty.tune_file_text().is_none() && empty.tune_nets().is_empty());
    // A gated pair alone is checked, not tuned: it has no tune file either.
    let pair_only = LengthPlan {
        pairs: vec![gated("P", "N", 0.2, None)],
        ..LengthPlan::default()
    };
    assert!(pair_only.tune_file_text().is_none());
    assert!(pair_only.tune_nets().is_empty());
    assert!(!pair_only.is_empty());
    assert!(empty.is_empty());
    // Control: the plan with a group has both.
    let plan = two_group_plan();
    assert!(plan.tune_file_text().is_some());
    assert_eq!(
        plan.tune_nets(),
        set(&["/DQ0", "/DQ1", "/DQS_P", "/DQS_N", "/A0", "/A1"])
    );
}

fn gated(positive: &str, negative: &str, max_skew_mm: f64, group: Option<&str>) -> GatedPair {
    GatedPair {
        positive: positive.to_string(),
        negative: negative.to_string(),
        dsn_positive: format!("/{positive}"),
        dsn_negative: format!("/{negative}"),
        max_skew_mm,
        group: group.map(str::to_string),
    }
}

#[test]
fn the_file_is_written_into_the_call_folder_only_with_a_group() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        write_file(dir.path(), &LengthPlan::default()).unwrap(),
        None
    );
    assert!(!dir.path().join(TUNE_FILE).exists());
    let plan = two_group_plan();
    let path = write_file(dir.path(), &plan).unwrap().unwrap();
    assert_eq!(path, dir.path().join("tune.txt"));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        plan.tune_file_text().unwrap()
    );
    // A folder that does not exist is an error, not a silent skip.
    assert!(write_file(&dir.path().join("missing"), &plan).is_err());
}

// ---- Row LT, L21: the tune file without the nets that touched themselves ------

/// The tune file's groups as fastroute reads it, names only.
fn parsed_names(text: &str) -> Vec<(String, Vec<String>)> {
    fastroute_parse_tune(text)
        .unwrap()
        .into_iter()
        .map(|group| (group.name, group.nets))
        .collect()
}

#[test]
fn a_dropped_net_leaves_its_group_and_a_group_keeps_its_window() {
    let plan = two_group_plan();
    // A plain net of a skew group (3 nets left): the group stays, with its own
    // tolerance, and so does the pair group.
    let text = tune_file_without(&plan, &set(&["/DQ1"])).unwrap();
    assert_eq!(
        text,
        "# Length groups for fastroute --tune, written by Konnect (DSN net names only).\n\
         group lt1 tolerance=0.25\n  /DQ0\n  /DQS_P\n  /DQS_N\n\
         group lt2 tolerance=0.5 target=42.25\n  /A0\n  /A1\n\
         group lt1p1 tolerance=0.15\n  /DQS_P\n  /DQS_N\n"
    );
    // A net of the target group: it stays with one net left (a target needs no
    // second net).
    let text = tune_file_without(&plan, &set(&["/A0"])).unwrap();
    assert_eq!(
        parsed_names(&text),
        vec![
            (
                "lt1".to_string(),
                strings(&["/DQ0", "/DQ1", "/DQS_P", "/DQS_N"])
            ),
            ("lt2".to_string(), strings(&["/A1"])),
            ("lt1p1".to_string(), strings(&["/DQS_P", "/DQS_N"])),
        ]
    );
    assert!(
        text.contains("group lt2 tolerance=0.5 target=42.25\n"),
        "{text}"
    );
    // A name the plan does not hold drops nothing.
    assert_eq!(
        tune_file_without(&plan, &set(&["/NOPE"])),
        plan.tune_file_text()
    );
}

#[test]
fn a_skew_group_left_with_fewer_than_two_nets_is_left_out_with_its_pair_groups() {
    let plan = two_group_plan();
    // One net left in the skew group: nothing to skew against, so it is out, and
    // its pair group with it.
    let text = tune_file_without(&plan, &set(&["/DQ0", "/DQ1", "/DQS_P"])).unwrap();
    assert_eq!(
        parsed_names(&text),
        vec![("lt2".to_string(), strings(&["/A0", "/A1"]))]
    );
    // Exactly two nets left: the group stays.
    let text = tune_file_without(&plan, &set(&["/DQ0", "/DQ1"])).unwrap();
    assert_eq!(
        parsed_names(&text),
        vec![
            ("lt1".to_string(), strings(&["/DQS_P", "/DQS_N"])),
            ("lt2".to_string(), strings(&["/A0", "/A1"])),
            ("lt1p1".to_string(), strings(&["/DQS_P", "/DQS_N"])),
        ]
    );
    // A target group with none left is out; the other group stays.
    let text = tune_file_without(&plan, &set(&["/A0", "/A1"])).unwrap();
    assert_eq!(
        parsed_names(&text)
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["lt1", "lt1p1"]
    );
    // No group left: no file.
    assert_eq!(
        tune_file_without(&plan, &set(&["/DQ0", "/DQ1", "/DQS_P", "/A0", "/A1"])),
        None
    );
    assert_eq!(tune_file_without(&LengthPlan::default(), &set(&[])), None);
}

#[test]
fn the_pair_group_of_a_dropped_net_is_left_out_and_the_others_keep_their_numbers() {
    let mut plan = two_group_plan();
    plan.groups[0].pairs = vec![
        ("/DQS_P".into(), "/DQS_N".into(), 0.15),
        ("/DQ0".into(), "/DQ1".into(), 0.05),
    ];
    // A dropped /DQS_P (its partner is not dropped here: this function drops only
    // what it is given): the first pair group is out and the second keeps its number.
    let text = tune_file_without(&plan, &set(&["/DQS_P"])).unwrap();
    assert!(!text.contains("lt1p1"), "{text}");
    assert!(
        text.ends_with("group lt1p2 tolerance=0.05\n  /DQ0\n  /DQ1\n"),
        "{text}"
    );
    assert!(
        text.contains("group lt1 tolerance=0.25\n  /DQ0\n  /DQ1\n  /DQS_N\n"),
        "{text}"
    );
    // The other net of the pair gives the same.
    let text = tune_file_without(&plan, &set(&["/DQS_N"])).unwrap();
    assert!(!text.contains("lt1p1") && text.contains("lt1p2"), "{text}");
    // Control: a dropped net outside both pairs leaves both pair groups.
    let plan_text = tune_file_without(&plan, &set(&["/A0"])).unwrap();
    assert!(plan_text.contains("lt1p1") && plan_text.contains("lt1p2"));
}

#[test]
fn with_nothing_dropped_the_file_is_the_plans_file_and_gains_no_partner() {
    for plan in [two_group_plan(), LengthPlan::default()] {
        assert_eq!(tune_file_without(&plan, &set(&[])), plan.tune_file_text());
        assert_eq!(with_pair_partners(&plan, &set(&[])), set(&[]));
    }
    let plan = two_group_plan();
    assert_eq!(plan.tune_nets_without(&set(&[])), plan.tune_nets());
}

#[test]
fn a_group_that_lost_no_net_stays_as_the_plan_wrote_it() {
    // A skew group of one net (nothing to skew against) is written by
    // `tune_file_text`; dropping a net of another group does not take it out.
    let plan = LengthPlan {
        groups: vec![
            group(
                "i",
                "solo",
                "lt1",
                &[("a", "/A")],
                Window::Skew { tolerance_mm: 0.5 },
                &[],
            ),
            group(
                "i",
                "other",
                "lt2",
                &[("b", "/B"), ("c", "/C"), ("d", "/D")],
                Window::Skew { tolerance_mm: 0.5 },
                &[],
            ),
        ],
        ..LengthPlan::default()
    };
    let text = tune_file_without(&plan, &set(&["/D"])).unwrap();
    assert!(text.contains("group lt1 tolerance=0.25\n  /A\n"), "{text}");
    assert!(
        text.contains("group lt2 tolerance=0.25\n  /B\n  /C\n"),
        "{text}"
    );
    assert_eq!(
        plan.tune_nets_without(&set(&["/D"])),
        set(&["/A", "/B", "/C"])
    );
    // Control: a group that did lose a net and is left with one is out.
    let text = tune_file_without(&plan, &set(&["/C", "/D"])).unwrap();
    assert!(text.contains("lt1") && !text.contains("lt2"), "{text}");
}

#[test]
fn a_dropped_net_of_a_pair_adds_its_partner() {
    let plan = two_group_plan();
    assert_eq!(
        with_pair_partners(&plan, &set(&["/DQS_P"])),
        set(&["/DQS_P", "/DQS_N"])
    );
    assert_eq!(
        with_pair_partners(&plan, &set(&["/DQS_N"])),
        set(&["/DQS_P", "/DQS_N"])
    );
    // A net with no partner, a net of no pair and a name the plan lacks add none.
    assert_eq!(with_pair_partners(&plan, &set(&["/DQ0"])), set(&["/DQ0"]));
    assert_eq!(with_pair_partners(&plan, &set(&["/A1"])), set(&["/A1"]));
    assert_eq!(with_pair_partners(&plan, &set(&["/NOPE"])), set(&["/NOPE"]));
    // Both already dropped; and a mix.
    assert_eq!(
        with_pair_partners(&plan, &set(&["/DQS_P", "/DQS_N", "/A0"])),
        set(&["/DQS_P", "/DQS_N", "/A0"])
    );
    assert_eq!(
        with_pair_partners(&plan, &set(&["/DQ1", "/DQS_N"])),
        set(&["/DQ1", "/DQS_P", "/DQS_N"])
    );
    // Every pair of every group counts.
    let mut plan = two_group_plan();
    plan.groups[1].pairs = vec![("/A0".into(), "/A1".into(), 0.1)];
    assert_eq!(
        with_pair_partners(&plan, &set(&["/A1"])),
        set(&["/A0", "/A1"])
    );
}

#[test]
fn the_nets_the_next_file_lists_are_the_ones_its_text_holds() {
    let plan = two_group_plan();
    for dropped in [
        set(&[]),
        set(&["/DQ1"]),
        set(&["/DQS_P", "/DQS_N"]),
        set(&["/DQ0", "/DQ1", "/DQS_P"]),
        set(&["/A0"]),
        set(&["/A0", "/A1"]),
    ] {
        let listed = plan.tune_nets_without(&dropped);
        let in_text: BTreeSet<String> = tune_file_without(&plan, &dropped)
            .map(|text| {
                parsed_names(&text)
                    .into_iter()
                    .flat_map(|(_, nets)| nets)
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(listed, in_text, "{dropped:?}");
    }
}

#[test]
fn the_next_round_writes_its_file_without_the_nets_and_their_partners() {
    let dir = tempfile::tempdir().unwrap();
    let plan = two_group_plan();
    let (path, listed) = write_file_without(dir.path(), &plan, &set(&["/DQS_P"]))
        .unwrap()
        .unwrap();
    assert_eq!(path, dir.path().join(TUNE_FILE));
    // The partner went with it, and the pair group.
    assert_eq!(listed, set(&["/DQ0", "/DQ1", "/A0", "/A1"]));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("DQS") && !text.contains("lt1p1"), "{text}");
    assert_eq!(
        text,
        tune_file_without(&plan, &set(&["/DQS_P", "/DQS_N"])).unwrap()
    );
    // No group left: nothing is written.
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(
        write_file_without(empty.path(), &plan, &plan.tune_nets()).unwrap(),
        None
    );
    assert!(!empty.path().join(TUNE_FILE).exists());
    // A folder that does not exist is an error.
    assert!(write_file_without(&dir.path().join("missing"), &plan, &set(&["/DQ0"])).is_err());
}

#[test]
fn the_judge_is_given_every_group_and_every_gated_pair_by_dsn_name() {
    let mut plan = two_group_plan();
    plan.pairs = vec![gated("DQS_P", "DQS_N", 0.3, Some("dq_low"))];
    let checks = plan.checks();
    assert_eq!(checks.groups.len(), 2);
    assert_eq!(checks.groups[0].interface, "ram");
    assert_eq!(checks.groups[0].name, "dq_low");
    assert_eq!(
        checks.groups[0].nets,
        strings(&["/DQ0", "/DQ1", "/DQS_P", "/DQS_N"])
    );
    assert_eq!(checks.groups[1].window, plan.groups[1].window);
    assert_eq!(
        checks.pairs,
        vec![CheckedPair {
            positive: "/DQS_P".into(),
            negative: "/DQS_N".into(),
            max_skew_mm: 0.3,
        }]
    );
    assert_eq!(
        checks.nets(),
        strings(&["/A0", "/A1", "/DQ0", "/DQ1", "/DQS_N", "/DQS_P"])
    );
    assert!(LengthPlan::default().checks().is_empty());
    assert!(!checks.is_empty());
}

// ─── An impossible target (L12) ─────────────────────────────────────────────

#[test]
fn the_nets_of_a_board_are_the_names_its_items_carry() {
    let board = board_of(&[
        footprint(
            0.0,
            0.0,
            &[
                pad("1", "N1", 0.0, 1.0, 1.0),
                pad("2", "GND", 2.0, 1.0, 1.0),
            ],
        ),
        zone_of("GND"),
    ]);
    assert_eq!(board_net_names(&board).unwrap(), set(&["N1", "GND"]));
    // A KiCad 9 board has its table at the top: names once each, no `(net 0 "")`.
    let nine = "(kicad_pcb (net 0 \"\") (net 1 \"A\") (net 2 \"B\") \
                (footprint \"T\" (pad \"1\" smd rect (net 1 \"A\"))))";
    assert_eq!(board_net_names(nine).unwrap(), set(&["A", "B"]));
    assert!(board_net_names("(kicad_pcb (").is_err());
}

#[test]
fn the_shortest_length_is_the_farthest_pad_distance_less_both_circumradii() {
    // Two 1 x 1 pads, 20 mm apart centre to centre: each circumradius is
    // sqrt(2) / 2 = 0.7071, so no track can be shorter than 20 − 1.4142 = 18.5858.
    let board = board_of(&[
        footprint(0.0, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]),
        footprint(20.0, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]),
    ]);
    let shortest = shortest_possible_mm(&board, &set(&["N1"])).unwrap();
    assert!(
        (shortest["N1"] - (20.0 - 2.0_f64.sqrt())).abs() < 1e-9,
        "{shortest:?}"
    );
    // Three pads: the farthest two decide.
    let board = board_of(&[
        footprint(0.0, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]),
        footprint(20.0, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]),
        footprint(5.0, 5.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]),
    ]);
    let shortest = shortest_possible_mm(&board, &set(&["N1"])).unwrap();
    assert!(
        (shortest["N1"] - (20.0 - 2.0_f64.sqrt())).abs() < 1e-9,
        "{shortest:?}"
    );
    // Big pads can swallow the distance: never below 0.
    let board = board_of(&[
        footprint(0.0, 0.0, &[pad("1", "N1", 0.0, 10.0, 10.0)]),
        footprint(3.0, 0.0, &[pad("1", "N1", 0.0, 10.0, 10.0)]),
    ]);
    assert_eq!(
        shortest_possible_mm(&board, &set(&["N1"])).unwrap()["N1"],
        0.0
    );
    // A net with one pad, or none on the board, has no bound above 0.
    let board = board_of(&[footprint(0.0, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)])]);
    let shortest = shortest_possible_mm(&board, &set(&["N1", "NONE"])).unwrap();
    assert_eq!(shortest["N1"], 0.0);
    assert_eq!(shortest["NONE"], 0.0);
    // Only the nets asked for are computed, whatever else the board holds.
    assert!(!shortest.contains_key("GND"));
    let board = board_of(&[
        footprint(
            0.0,
            0.0,
            &[
                pad("1", "N1", 0.0, 1.0, 1.0),
                pad("2", "GND", 5.0, 1.0, 1.0),
            ],
        ),
        footprint(
            20.0,
            0.0,
            &[
                pad("1", "N1", 0.0, 1.0, 1.0),
                pad("2", "GND", 5.0, 1.0, 1.0),
            ],
        ),
    ]);
    let shortest = shortest_possible_mm(&board, &set(&["N1"])).unwrap();
    assert_eq!(shortest.keys().collect::<Vec<_>>(), vec!["N1"]);
    assert!(shortest_possible_mm(&board, &set(&["N1", "GND"]))
        .unwrap()
        .contains_key("GND"));
    assert!(shortest_possible_mm("(kicad_pcb (", &set(&["N1"])).is_err());
}

#[test]
fn a_pad_with_no_net_is_not_a_pad_of_any_net() {
    // A pad that names no net beside N1's pads, one in N1's own footprint and one far
    // away, changes nothing: N1's pads are in two footprints, 20 mm apart.
    let unnetted = |number: &str, dx: f64| {
        format!("(pad \"{number}\" smd rect (at {dx} 0) (size 1 1) (layers \"F.Cu\"))")
    };
    let board = board_of(&[
        footprint(
            0.0,
            0.0,
            &[pad("1", "N1", 0.0, 1.0, 1.0), unnetted("2", 0.5)],
        ),
        footprint(20.0, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]),
        footprint(100.0, 0.0, &[unnetted("1", 0.0)]),
    ]);
    let shortest = shortest_possible_mm(&board, &set(&["N1"])).unwrap();
    assert!(
        (shortest["N1"] - (20.0 - 2.0_f64.sqrt())).abs() < 1e-9,
        "{shortest:?}"
    );
}

#[test]
fn a_net_that_owns_a_zone_is_left_out() {
    let parts = |zone: bool| {
        let mut parts = vec![
            footprint(0.0, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]),
            footprint(20.0, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]),
        ];
        if zone {
            parts.push(zone_of("N1"));
        }
        board_of(&parts)
    };
    // With the pour, a track need not join the pads: no bound.
    let with = shortest_possible_mm(&parts(true), &set(&["N1"])).unwrap();
    assert!(!with.contains_key("N1"), "{with:?}");
    // Control: the same board without the zone has the bound.
    assert!(shortest_possible_mm(&parts(false), &set(&["N1"])).unwrap()["N1"] > 18.0);
}

#[test]
fn a_zone_that_cannot_be_read_leaves_every_net_out() {
    // A zone with two points is skipped by the geometry reader, and any net may own it.
    let broken = "(zone (net \"N1\") (layer \"F.Cu\") (polygon (pts (xy 0 0) (xy 1 0))))";
    let board = |extra: Option<&str>| {
        let mut parts = vec![
            footprint(0.0, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]),
            footprint(20.0, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]),
        ];
        if let Some(extra) = extra {
            parts.push(extra.to_string());
        }
        board_of(&parts)
    };
    assert!(shortest_possible_mm(&board(Some(broken)), &set(&["N1"]))
        .unwrap()
        .is_empty());
    // Control: without the broken zone the net has its bound.
    assert!(shortest_possible_mm(&board(None), &set(&["N1"]))
        .unwrap()
        .contains_key("N1"));
}

#[test]
fn two_pads_of_one_footprint_are_never_paired() {
    // One footprint's pads can be one pin or tied inside the footprint.
    let one = board_of(&[footprint(
        0.0,
        0.0,
        &[
            pad("1", "N1", -10.0, 1.0, 1.0),
            pad("2", "N1", 10.0, 1.0, 1.0),
        ],
    )]);
    // Such a net is left out: no bound is taken across the other footprints either.
    assert!(!shortest_possible_mm(&one, &set(&["N1"]))
        .unwrap()
        .contains_key("N1"));
    // Control: the same pads in two footprints are 20 mm apart.
    let two = board_of(&[
        footprint(-10.0, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]),
        footprint(10.0, 0.0, &[pad("2", "N1", 0.0, 1.0, 1.0)]),
    ]);
    assert!(shortest_possible_mm(&two, &set(&["N1"])).unwrap()["N1"] > 18.0);
}

fn target_group(target_mm: f64, tolerance_mm: f64) -> ResolvedGroup {
    group(
        "bus",
        "len",
        "lt1",
        &[("N1", "N1")],
        Window::Target {
            target_mm,
            tolerance_mm,
        },
        &[],
    )
}

#[test]
fn a_target_below_the_shortest_possible_length_refuses_before_routing() {
    let shortest: BTreeMap<String, f64> = [("N1".to_string(), 18.5858)].into();
    // target 10 ± 1 can never be met: the pads are 18.58 mm apart.
    let refusal = impossible_target(&[target_group(10.0, 1.0)], &shortest).expect("refused");
    for word in [
        "'len'",
        "'bus'",
        "'N1'",
        "10 mm",
        "11 mm",
        "18.585 mm",
        "never shortens",
    ] {
        assert!(refusal.reason.contains(word), "{word}: {}", refusal.reason);
    }
    assert!(
        refusal.next_step.contains("set_design_intent") && refusal.next_step.contains("18.586"),
        "{}",
        refusal.next_step
    );
    // Controls: a window whose top reaches the bound, and one above it, are not refused.
    assert!(impossible_target(&[target_group(18.0, 0.6)], &shortest).is_none());
    assert!(impossible_target(&[target_group(18.5858, 0.0001)], &shortest).is_none());
    assert!(impossible_target(&[target_group(40.0, 1.0)], &shortest).is_none());
    // One step below the bound refuses.
    assert!(impossible_target(&[target_group(18.0, 0.5)], &shortest).is_some());
    // The bound exactly at the window's top is within reach; a hair above it is not.
    let exact: BTreeMap<String, f64> = [("N1".to_string(), 18.5)].into();
    assert!(impossible_target(&[target_group(18.0, 0.5)], &exact).is_none());
    assert!(impossible_target(&[target_group(18.0, 0.499)], &exact).is_some());
    let above: BTreeMap<String, f64> = [("N1".to_string(), 18.500_001)].into();
    assert!(impossible_target(&[target_group(18.0, 0.5)], &above).is_some());
    // A net with no entry (it owns a zone) is never refused.
    assert!(impossible_target(&[target_group(1.0, 0.5)], &BTreeMap::new()).is_none());
    // A skew-mode group is never refused here, whatever the bound.
    let skew = group(
        "bus",
        "len",
        "lt1",
        &[("N1", "N1")],
        Window::Skew { tolerance_mm: 0.5 },
        &[],
    );
    assert!(impossible_target(&[skew], &shortest).is_none());
    // The first group that cannot be met is the one named.
    let mut second = target_group(5.0, 0.5);
    second.name = "second".into();
    let refusal = impossible_target(
        &[target_group(40.0, 1.0), second, target_group(1.0, 0.5)],
        &shortest,
    )
    .unwrap();
    assert!(refusal.reason.contains("'second'"), "{}", refusal.reason);
}

#[test]
fn a_target_exactly_at_the_pad_distance_is_accepted_on_a_real_board() {
    // The bound sits below the centre distance (the pads' own size is cut off), so a
    // target equal to the distance is within reach.
    let board = board_of(&[
        footprint(0.0, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]),
        footprint(20.0, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]),
    ]);
    let nets = set(&["N1"]);
    let shortest = shortest_possible_mm(&board, &nets).unwrap();
    assert!(impossible_target(&[target_group(20.0, 0.5)], &shortest).is_none());
    // Control: a target well under it is refused.
    assert!(impossible_target(&[target_group(15.0, 0.5)], &shortest).is_some());
}

#[test]
fn a_net_with_two_pads_in_one_footprint_is_not_refused() {
    // N1 has pads at 0 and 20 mm in one footprint (its copper joins them), and a third
    // pad at 20 mm in another: a track of 1 mm can complete the net, though the pads
    // at 0 and 20 mm are in different footprints in the control below and would give 18.6.
    let board = |split: bool| {
        let near = if split {
            vec![
                footprint(0.0, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]),
                footprint(20.0, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]),
            ]
        } else {
            vec![footprint(
                0.0,
                0.0,
                &[
                    pad("1", "N1", 0.0, 1.0, 1.0),
                    pad("2", "N1", 20.0, 1.0, 1.0),
                ],
            )]
        };
        let mut parts = near;
        parts.push(footprint(20.5, 0.0, &[pad("1", "N1", 0.0, 1.0, 1.0)]));
        board_of(&parts)
    };
    let nets = set(&["N1"]);
    let one_footprint = shortest_possible_mm(&board(false), &nets).unwrap();
    assert!(!one_footprint.contains_key("N1"), "{one_footprint:?}");
    assert!(impossible_target(&[target_group(10.0, 1.0)], &one_footprint).is_none());
    // Negative control: the same pads in distinct footprints keep the bound, and the
    // same target is refused.
    let split = shortest_possible_mm(&board(true), &nets).unwrap();
    assert!(split["N1"] > 18.0, "{split:?}");
    assert!(impossible_target(&[target_group(10.0, 1.0)], &split).is_some());
}

// ─── KiCad's own length and skew rules (L15) ────────────────────────────────

const DRU: &str = r#"(version 1)
(rule "ddr_data"
    (constraint length (min 40mm) (max 41mm)   (opt 40.5mm))
    (condition "A.NetName == '/DQ0' || A.NetName == '/DQ1'"))
(rule "clear" (constraint clearance (min 0.3mm)))
(rule "lane skew" (layer outer) (severity warning)
    (constraint skew (max 0.15mm))
    (condition "A.NetClass == 'lane'"))
(rule "mixed"
    (constraint clearance (min 0.2mm))
    (constraint length (max 80mm)))
"#;

#[test]
fn kicads_length_and_skew_rules_are_listed_as_the_file_writes_them() {
    let rules = kicad_length_rules(DRU).unwrap();
    assert_eq!(
        rules,
        vec![
            KicadLengthRule {
                name: "ddr_data".into(),
                constraints: vec!["(constraint length (min 40mm) (max 41mm) (opt 40.5mm))".into()],
                condition: Some("A.NetName == '/DQ0' || A.NetName == '/DQ1'".into()),
            },
            KicadLengthRule {
                name: "lane skew".into(),
                constraints: vec!["(constraint skew (max 0.15mm))".into()],
                condition: Some("A.NetClass == 'lane'".into()),
            },
            // A rule that mixes kinds lists only its length and skew constraints.
            KicadLengthRule {
                name: "mixed".into(),
                constraints: vec!["(constraint length (max 80mm))".into()],
                condition: None,
            },
        ]
    );
    // Control: a file with no length or skew rule lists none.
    assert!(
        kicad_length_rules("(version 1)\n(rule \"c\" (constraint clearance (min 0.3mm)))\n")
            .unwrap()
            .is_empty()
    );
    assert!(kicad_length_rules("(version 1)\n").unwrap().is_empty());
    // A file KiCad's own framing refuses is an error, not an empty list.
    assert!(kicad_length_rules("(version 1)\n(rule \"x\" (constraint").is_err());
    assert!(kicad_length_rules("(rule \"x\" (constraint length (max 1mm)))").is_err());
}

// ─── The rules a group is suggested as (L16) ────────────────────────────────

/// The rule text suggested for group `index` of `plan`, under its call-wide name.
fn suggested(plan: &LengthPlan, index: usize) -> String {
    kicad_rule_suggestion(&plan.groups[index], &rule_names(&plan.groups)[index])
}

#[test]
fn a_rules_name_follows_its_interface_and_group_and_is_unique_in_the_call() {
    let plan = two_group_plan();
    assert_eq!(
        rule_names(&plan.groups),
        strings(&["1kopper_len_ram_dq_low", "1kopper_len_ram_addr"])
    );
    // A reorder gives each group the same name, not the other's.
    let mut reordered = plan.clone();
    reordered.groups.reverse();
    assert_eq!(
        rule_names(&reordered.groups),
        strings(&["1kopper_len_ram_addr", "1kopper_len_ram_dq_low"])
    );
    // Characters outside [A-Za-z0-9_] become `_`.
    let odd = group(
        "my bus",
        "data/low",
        "lt1",
        &[("N1", "N1")],
        Window::Skew { tolerance_mm: 0.1 },
        &[],
    );
    assert_eq!(
        rule_names(std::slice::from_ref(&odd)),
        strings(&["1kopper_len_my_bus_data_low"])
    );
    // Two groups whose sanitized names meet get `_2`, `_3`.
    let alike = |interface: &str, name: &str| {
        group(
            interface,
            name,
            "lt1",
            &[("N1", "N1")],
            Window::Skew { tolerance_mm: 0.1 },
            &[],
        )
    };
    let clashing = [
        alike("a b", "c"),
        alike("a_b", "c"),
        alike("a.b", "c"),
        alike("a_b_c", "2"),
    ];
    assert_eq!(
        rule_names(&clashing),
        strings(&[
            "1kopper_len_a_b_c",
            "1kopper_len_a_b_c_2",
            "1kopper_len_a_b_c_3",
            "1kopper_len_a_b_c_2_2"
        ])
    );
}

#[test]
fn a_skew_group_is_suggested_as_a_kicad_skew_rule_with_the_whole_tolerance() {
    let plan = two_group_plan();
    // One printed digit (0.0001 mm) looser than the window, so a net the judge passes
    // is never flagged by the rule (L16).
    assert_eq!(
        suggested(&plan, 0),
        "(rule \"1kopper_len_ram_dq_low\" (constraint skew (max 0.5001mm)) (condition \"A.NetName == '/DQ0' || A.NetName == '/DQ1' || A.NetName == '/DQS_P' || A.NetName == '/DQS_N'\"))"
    );
    // Not fastroute's share: the whole tolerance is what the judge checked.
    assert!(!suggested(&plan, 0).contains("0.25mm"));
}

#[test]
fn a_target_group_is_suggested_as_a_kicad_length_rule_with_min_max_and_opt() {
    let plan = two_group_plan();
    assert_eq!(
        suggested(&plan, 1),
        "(rule \"1kopper_len_ram_addr\" (constraint length (min 41.4999mm) (max 42.5001mm) (opt 42mm)) (condition \"A.NetName == '/A0' || A.NetName == '/A1'\"))"
    );
    // Float noise stays out of the rule text.
    let mut noisy = target_group(42.1, 0.07);
    noisy.nets[0].dsn = "N1".into();
    assert!(
        kicad_rule_suggestion(&noisy, "n").contains("(min 42.0299mm) (max 42.1701mm) (opt 42.1mm)"),
        "{}",
        kicad_rule_suggestion(&noisy, "n")
    );
}

#[test]
fn a_gated_pair_is_suggested_as_a_kicad_skew_rule_on_its_two_nets() {
    assert_eq!(
        kicad_pair_rule_suggestion(&gated("DQS_P", "DQS_N", 0.05, None)),
        "(rule \"1kopper_skew_/DQS_P_/DQS_N\" (constraint skew (max 0.0501mm)) (condition \"A.NetName == '/DQS_P' || A.NetName == '/DQS_N'\"))"
    );
}

#[test]
fn a_suggested_rule_is_a_rule_kicad_s_file_framing_and_the_listing_accept() {
    // What the next steps hand to create_custom_drc_rule must read back as the rule it
    // says it is.
    let plan = two_group_plan();
    let pair = gated("DQS_P", "DQS_N", 0.05, None);
    let texts = [
        suggested(&plan, 0),
        suggested(&plan, 1),
        kicad_pair_rule_suggestion(&pair),
    ];
    let file = format!("(version 1)\n{}\n", texts.join("\n"));
    let listed = kicad_length_rules(&file).unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].name, "1kopper_len_ram_dq_low");
    assert_eq!(
        listed[1].constraints,
        vec!["(constraint length (min 41.4999mm) (max 42.5001mm) (opt 42mm))"]
    );
    assert_eq!(listed[2].name, "1kopper_skew_/DQS_P_/DQS_N");
    // And the board that carries them can still be exported (L19).
    assert!(custom_rules::routing_bounds(&file).is_ok());
}

// ─── The next steps (L16) ───────────────────────────────────────────────────

fn measures(miss_mm: f64, unmeasured: &[&str]) -> LengthMeasures {
    LengthMeasures {
        groups: Vec::new(),
        pairs: Vec::new(),
        unmeasured: strings(unmeasured),
        paths: Default::default(),
        miss_mm,
        probe_s: 0.1,
    }
}

/// The `rules` argument a next step hands to create_custom_drc_rule (Row RW), as
/// (name, rule) pairs, in order.
fn batch(step: &str) -> Vec<(String, String)> {
    let start = step
        .find("rules=")
        .unwrap_or_else(|| panic!("no rules batch in {step}"))
        + "rules=".len();
    let end = start
        + step[start..]
            .find(": dry_run first")
            .expect("the batch ends");
    let rules: Value = serde_json::from_str(&step[start..end]).expect("the batch is JSON");
    rules
        .as_array()
        .unwrap()
        .iter()
        .map(|rule| {
            (
                rule["name"].as_str().unwrap().to_string(),
                rule["rule"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn the_next_steps_give_every_checked_group_and_pair_in_one_batch_call() {
    let mut plan = two_group_plan();
    plan.pairs = vec![gated("DQS_P", "DQS_N", 0.05, Some("dq_low"))];
    let steps = plan.next_steps(None, 0);
    // One call for all three rules (Row RW), no line per rule; then the one
    // note: group dq_low holds the pair, and there is no winner to pin it to.
    assert_eq!(steps.len(), 2, "{steps:?}");
    assert!(
        steps[1].starts_with("Rule \"1kopper_len_ram_dq_low\"")
            && steps[1].contains("check only the pair rules"),
        "{}",
        steps[1]
    );
    let step = &steps[0];
    assert!(
        step.contains("create_custom_drc_rule once")
            && step.contains("apply_specctra_ses")
            && step.contains("2 group rule(s), 1 pair skew rule(s)")
            && step.contains("replace_existing=true"),
        "{step}"
    );
    let rules = batch(step);
    assert_eq!(
        rules,
        vec![
            ("1kopper_len_ram_dq_low".to_string(), suggested(&plan, 0)),
            (rule_names(&plan.groups)[1].clone(), suggested(&plan, 1)),
            (
                "1kopper_skew_/DQS_P_/DQS_N".to_string(),
                kicad_pair_rule_suggestion(&plan.pairs[0])
            ),
        ]
    );
    // Both modes are told apart.
    assert!(rules[0].1.contains("constraint skew") && rules[1].1.contains("constraint length"));
    // A call that checks nothing has no rule to suggest, even with KiCad's own listed.
    let rules_only = LengthPlan {
        kicad_rules: vec![KicadLengthRule {
            name: "r".into(),
            constraints: vec!["(constraint skew (max 1mm))".into()],
            condition: None,
        }],
        ..LengthPlan::default()
    };
    assert!(rules_only.next_steps(None, 0).is_empty());
    assert!(rules_only
        .next_steps(Some(&measures(1.0, &[])), 0)
        .is_empty());
}

#[test]
fn a_miss_adds_what_can_change_it_first_and_a_pass_adds_nothing() {
    let plan = two_group_plan();
    let met = plan.next_steps(Some(&measures(0.0, &[])), 0);
    assert_eq!(met, plan.next_steps(None, 0), "a pass adds no line");
    for missed in [measures(0.62, &[]), measures(0.0, &["/DQ0"])] {
        let steps = plan.next_steps(Some(&missed), 0);
        assert_eq!(steps.len(), met.len() + 1);
        assert!(
            steps[0].contains("tolerance_mm") && steps[0].contains("higher effort"),
            "{}",
            steps[0]
        );
        assert!(steps[0].contains("room"), "{}", steps[0]);
        assert_eq!(&steps[1..], &met[..], "the rules still follow");
    }
}

#[test]
fn a_violation_of_the_boards_own_rules_says_the_session_cannot_be_applied() {
    let plan = two_group_plan();
    let steps = plan.next_steps(Some(&measures(0.0, &[])), 2);
    let clean = plan.next_steps(Some(&measures(0.0, &[])), 0);
    assert_eq!(steps.len(), clean.len() + 1);
    assert!(
        steps[0].contains("2 violation(s)")
            && steps[0].contains("apply_specctra_ses rolls back")
            && steps[0].contains(".kicad_dru"),
        "{}",
        steps[0]
    );
    assert_eq!(&steps[1..], &clean[..]);
    // It is about the board's rules, so a call that checks no group has it too.
    let rules_only = LengthPlan::default().next_steps(None, 1);
    assert_eq!(rules_only.len(), 1, "{rules_only:?}");
    // Control: no violation, no line.
    assert!(LengthPlan::default().next_steps(None, 0).is_empty());
}

#[test]
fn the_judge_is_told_when_the_board_has_length_rules_of_its_own() {
    let mut plan = two_group_plan();
    assert!(!plan.checks().board_rules);
    plan.kicad_rules = kicad_length_rules(DRU).unwrap();
    assert!(plan.checks().board_rules);
    // Rules alone, with nothing to tune: not a group or a pair, so no probe.
    let only = LengthPlan {
        kicad_rules: kicad_length_rules(DRU).unwrap(),
        ..LengthPlan::default()
    };
    assert!(only.checks().board_rules && only.checks().is_empty());
}

// ─── The result's fields (L15) ──────────────────────────────────────────────

fn body(fields: &[(&'static str, Value)]) -> Value {
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].0, "length_matching");
    fields[0].1.clone()
}

#[test]
fn a_call_with_nothing_to_report_has_no_length_matching() {
    assert!(!LengthPlan::default().reported());
    assert!(LengthPlan::default().fields(None, None, None).is_empty());
    // Heights alone (a pair without a skew, no group) are not a report.
    let heights = LengthPlan {
        heights: LayerHeights {
            heights_mm: Some(vec![0.0, 1.58]),
            ..LayerHeights::default()
        },
        ..LengthPlan::default()
    };
    assert!(!heights.reported());
    // Each of the four causes is.
    assert!(two_group_plan().reported());
    assert!(LengthPlan {
        pairs: vec![gated("P", "N", 0.1, None)],
        ..LengthPlan::default()
    }
    .reported());
    assert!(LengthPlan {
        kicad_rules: vec![KicadLengthRule {
            name: "r".into(),
            constraints: vec![],
            condition: None
        }],
        ..LengthPlan::default()
    }
    .reported());
    assert!(LengthPlan {
        note: Some("n".into()),
        ..LengthPlan::default()
    }
    .reported());
    assert!(LengthPlan {
        kicad_rules_note: Some("n".into()),
        ..LengthPlan::default()
    }
    .reported());
}

fn measured_group(nets: &[(&str, Option<f64>, f64)], longest: Option<f64>) -> GroupLengths {
    GroupLengths {
        interface: "ram".into(),
        name: "addr".into(),
        longest_mm: longest,
        nets: nets
            .iter()
            .map(|(net, kicad, miss)| NetLength {
                net: net.to_string(),
                kicad_mm: *kicad,
                low_mm: 41.5,
                high_mm: 42.5,
                miss_mm: *miss,
            })
            .collect(),
    }
}

#[test]
fn the_fields_say_what_was_asked_before_anything_was_measured() {
    let mut plan = two_group_plan();
    plan.pairs = vec![gated("DQS_P", "DQS_N", 0.3, Some("dq_low"))];
    plan.heights = LayerHeights {
        heights_mm: Some(vec![0.0, 1.58]),
        source: Some(crate::layer_heights::HeightSource::KicadDefault),
        note: Some("assumed".into()),
    };
    plan.note = Some("a note".into());
    plan.kicad_rules = kicad_length_rules(DRU).unwrap();
    plan.kicad_rules_note = Some("rules note".into());
    let body = body(&plan.fields(None, None, None));
    assert_eq!(body["groups"].as_array().unwrap().len(), 2);
    let first = &body["groups"][0];
    assert_eq!(first["interface"], "ram");
    assert_eq!(first["name"], "dq_low");
    assert_eq!(first["file_name"], "lt1");
    assert_eq!(
        first["window"],
        json!({ "mode": "skew", "tolerance_mm": 0.5 })
    );
    assert_eq!(first["nets"][0]["net"], "Data0");
    assert_eq!(first["nets"][0]["dsn_net"], "/DQ0");
    assert!(first["nets"][0]["kicad_mm"].is_null());
    assert!(first["longest_mm"].is_null() && first["pass"].is_null());
    assert_eq!(
        body["groups"][1]["window"],
        json!({ "mode": "target", "target_mm": 42.0, "tolerance_mm": 0.5 })
    );
    assert_eq!(body["pairs"][0]["positive"], "DQS_P");
    assert_eq!(body["pairs"][0]["dsn_negative"], "/DQS_N");
    assert_eq!(body["pairs"][0]["max_skew_mm"], 0.3);
    assert_eq!(body["pairs"][0]["group"], "dq_low");
    assert!(body["pairs"][0]["skew_mm"].is_null() && body["pairs"][0]["pass"].is_null());
    assert_eq!(body["layer_heights"], plan.heights.json());
    assert_eq!(body["kicad_rules"].as_array().unwrap().len(), 3);
    assert_eq!(body["kicad_rules"][0]["name"], "ddr_data");
    assert_eq!(body["kicad_rules"][0]["evaluated"], false);
    assert_eq!(body["kicad_rules"][1]["condition"], "A.NetClass == 'lane'");
    assert!(body["kicad_rules"][2]["condition"].is_null());
    assert!(body["kicad_violations"].is_null() && body["tune_pass"].is_null());
    assert_eq!(
        body["pairs_tuned_net_by_net"],
        json!([{ "interface": "ram", "group": "dq_low", "positive": "/DQS_P", "negative": "/DQS_N" }])
    );
    assert_eq!(body["note"], "a note");
    assert_eq!(body["kicad_rules_note"], "rules note");
    // Control: neither note is there when none is set.
    let plain = body_without_notes();
    assert!(plain.get("note").is_none() && plain.get("kicad_rules_note").is_none());
}

fn body_without_notes() -> Value {
    body(&two_group_plan().fields(None, None, None))
}

#[test]
fn the_winners_measures_fill_the_fields_and_decide_pass() {
    let plan = two_group_plan();
    let mut met = measures(0.0, &[]);
    met.groups = vec![
        GroupLengths {
            interface: "ram".into(),
            name: "dq_low".into(),
            longest_mm: Some(50.0),
            nets: ["/DQ0", "/DQ1", "/DQS_P", "/DQS_N"]
                .iter()
                .map(|net| NetLength {
                    net: net.to_string(),
                    kicad_mm: Some(49.9),
                    low_mm: 49.5,
                    high_mm: 50.0,
                    miss_mm: 0.0,
                })
                .collect(),
        },
        measured_group(
            &[("/A0", Some(42.0), 0.0), ("/A1", Some(41.8), 0.0)],
            Some(42.0),
        ),
    ];
    let violations: BTreeMap<String, usize> = [
        ("length_out_of_range".to_string(), 2),
        ("clearance".to_string(), 9),
    ]
    .into();
    let tune = json!({ "name": "tune", "outcome": "kept" });
    let fields = plan.fields(Some(&met), Some(&violations), Some(&tune));
    let found = body(&fields);
    assert_eq!(found["groups"][0]["pass"], true);
    assert_eq!(found["groups"][1]["pass"], true);
    assert_eq!(found["groups"][0]["longest_mm"], 50.0);
    assert_eq!(found["groups"][0]["nets"][0]["kicad_mm"], 49.9);
    assert_eq!(found["groups"][0]["nets"][0]["low_mm"], 49.5);
    assert_eq!(found["groups"][1]["nets"][1]["kicad_mm"], 41.8);
    // KiCad's own counts: the two length keys only, zero when absent.
    assert_eq!(
        found["kicad_violations"],
        json!({ "length_out_of_range": 2, "skew_out_of_range": 0 })
    );
    assert_eq!(found["tune_pass"], tune);
    // A net past its window, an unmeasured net, and a net the check does not list.
    for broken in [
        measured_group(
            &[("/A0", Some(42.0), 0.0), ("/A1", Some(41.0), 0.5)],
            Some(42.0),
        ),
        measured_group(&[("/A0", Some(42.0), 0.0), ("/A1", None, 0.0)], Some(42.0)),
        measured_group(&[("/A0", Some(42.0), 0.0)], Some(42.0)),
    ] {
        let mut missed = met.clone();
        missed.groups[1] = broken;
        let found = body(&plan.fields(Some(&missed), None, None));
        assert_eq!(found["groups"][0]["pass"], true);
        assert_eq!(found["groups"][1]["pass"], false, "{found}");
    }
    // Measures that hold no entry for a group at all fail it.
    let mut empty = met.clone();
    empty.groups.clear();
    assert_eq!(
        body(&plan.fields(Some(&empty), None, None))["groups"][0]["pass"],
        false
    );
}

#[test]
fn a_gated_pairs_skew_is_reported_with_its_pass() {
    let mut plan = LengthPlan {
        pairs: vec![gated("P", "N", 0.3, None)],
        ..LengthPlan::default()
    };
    let mut found = measures(0.0, &[]);
    found.pairs = vec![PairSkew {
        positive: "/P".into(),
        negative: "/N".into(),
        skew_mm: Some(0.12),
        max_skew_mm: 0.3,
        miss_mm: 0.0,
    }];
    let body_met = body(&plan.fields(Some(&found), None, None));
    assert_eq!(body_met["pairs"][0]["skew_mm"], 0.12);
    assert_eq!(body_met["pairs"][0]["pass"], true);
    assert_eq!(body_met["pairs"][0]["miss_mm"], 0.0);
    assert!(body_met["pairs"][0]["group"].is_null());
    found.pairs[0].miss_mm = 0.2;
    found.pairs[0].skew_mm = Some(0.5);
    let body_missed = body(&plan.fields(Some(&found), None, None));
    assert_eq!(body_missed["pairs"][0]["pass"], false);
    assert_eq!(body_missed["pairs"][0]["miss_mm"], 0.2);
    // An unmeasured pair fails.
    found.pairs[0].skew_mm = None;
    found.pairs[0].miss_mm = 0.0;
    assert_eq!(
        body(&plan.fields(Some(&found), None, None))["pairs"][0]["pass"],
        false
    );
    // Without measures nothing passes or fails.
    plan.pairs[0].group = Some("g".into());
    assert!(body(&plan.fields(None, None, None))["pairs"][0]["pass"].is_null());
}

// ─── The plan of a call ─────────────────────────────────────────────────────

const POURS_BOARD: &str = include_str!("../../tests/fixtures/specctra_pours_routing.kicad_pcb");
const POURS_PRO: &str = include_str!("../../tests/fixtures/specctra_pours_routing.kicad_pro");
const POURS_DSN: &str = include_str!("../../tests/fixtures/specctra_pours_routing.dsn");

/// A folder with the pours fixture's board and project, the given design intent and
/// rules file. Returns the board's path.
fn fixture_folder(intent: Option<&str>, dru: Option<&str>) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("b.kicad_pcb");
    std::fs::write(&board, POURS_BOARD).unwrap();
    std::fs::write(dir.path().join("b.kicad_pro"), POURS_PRO).unwrap();
    if let Some(intent) = intent {
        std::fs::create_dir_all(dir.path().join(".konnect")).unwrap();
        std::fs::write(dir.path().join(".konnect/design_intent.json"), intent).unwrap();
    }
    if let Some(dru) = dru {
        std::fs::write(dir.path().join("b.kicad_dru"), dru).unwrap();
    }
    (dir, board)
}

fn intent_text(interface: Value) -> String {
    json!({ "schema_version": 1, "rails": [], "currents": [],
            "interfaces": [interface], "fab_target": {} })
    .to_string()
}

/// The fixture's six signals as one interface, with the given extra keys.
fn bus(extra: Value) -> String {
    let mut interface = json!({ "name": "bus", "kind": "bus",
                                "nets": ["S1", "S2", "S3", "S4", "S5", "S6"] });
    for (key, value) in extra.as_object().unwrap() {
        interface[key] = value.clone();
    }
    intent_text(interface)
}

/// The length plan of the fixture board, with its real pairs plan.
fn plan_of(board: &Path) -> Result<LengthPlan, LengthRefusal> {
    let pairs = route_pairs::plan(board, POURS_DSN);
    plan(board, POURS_BOARD, POURS_DSN, &pairs)
}

#[test]
fn a_plan_resolves_the_intent_against_the_dsn_and_the_board() {
    let intent = bus(json!({
        "differential_pairs": [{ "positive": "S1", "negative": "S2", "max_skew_mm": 0.2 }],
        "length_groups": [
            { "name": "pair", "nets": ["S1", "S2"], "tolerance_mm": 0.5 },
            { "name": "rest", "nets": ["S3", "S4"], "tolerance_mm": 1.0, "target_mm": 80.0 }
        ]
    }));
    let (_dir, board) = fixture_folder(Some(&intent), None);
    let plan = plan_of(&board).unwrap();
    assert_eq!(plan.groups.len(), 2);
    assert_eq!(plan.groups[0].file_name, "lt1");
    assert_eq!(plan.groups[0].nets[1].dsn, "S2");
    assert_eq!(
        plan.groups[0].pairs,
        vec![("S1".to_string(), "S2".to_string(), 0.1)]
    );
    assert_eq!(plan.pairs.len(), 1);
    assert_eq!(plan.pairs[0].group.as_deref(), Some("pair"));
    assert_eq!(plan.tune_nets(), set(&["S1", "S2", "S3", "S4"]));
    let text = plan.tune_file_text().unwrap();
    assert!(
        text.contains("group lt1 tolerance=0.25\n  S1\n  S2\n"),
        "{text}"
    );
    assert!(
        text.contains("group lt2 tolerance=1 target=80.5\n  S3\n  S4\n"),
        "{text}"
    );
    assert!(
        text.ends_with("group lt1p1 tolerance=0.1\n  S1\n  S2\n"),
        "{text}"
    );
    // The heights of the fixture's two copper layers, in DSN order, top first.
    let heights = plan.heights.heights_mm.as_ref().expect("heights");
    assert_eq!(heights.len(), 2);
    assert_eq!(heights[0], 0.0);
    assert!(heights[1] > 1.0, "{heights:?}");
    assert!(plan.note.is_none() && plan.kicad_rules.is_empty());
    assert!(plan.reported());
}

#[test]
fn without_an_intent_the_plan_is_empty_and_reads_no_heights() {
    let (_dir, board) = fixture_folder(None, None);
    let plan = plan_of(&board).unwrap();
    assert_eq!(plan, LengthPlan::default());
    assert!(!plan.reported() && plan.tune_file_text().is_none());
    // An intent with no group and no skew is as good as none.
    let (_dir, board) = fixture_folder(Some(&bus(json!({}))), None);
    assert_eq!(plan_of(&board).unwrap(), LengthPlan::default());
    // Even an unreadable DSN is no reason to look when nothing is declared.
    let pairs = PairsPlan::default();
    assert_eq!(
        super::plan(&board, POURS_BOARD, "not a dsn (", &pairs).unwrap(),
        LengthPlan::default()
    );
}

#[test]
fn a_pair_alone_gets_heights_and_a_group_alone_gets_heights() {
    // A resolved pair without a skew: no group, nothing checked, but the heights the
    // routing passes will send (L7). No note when none is needed.
    let pair = bus(json!({ "differential_pairs": [{ "positive": "S1", "negative": "S2" }] }));
    let (_dir, board) = fixture_folder(Some(&pair), None);
    let plan = plan_of(&board).unwrap();
    assert!(plan.is_empty() && !plan.reported());
    assert!(plan.heights.heights_mm.is_some());
    // A group and no pair: heights too (the Tune pass sends them).
    let group = bus(json!({ "length_groups": [
        { "name": "g", "nets": ["S1", "S2"], "tolerance_mm": 0.5 }] }));
    let (_dir, board) = fixture_folder(Some(&group), None);
    assert!(plan_of(&board).unwrap().heights.heights_mm.is_some());
    // Control: neither a group nor a resolved pair, and none are read.
    let (_dir, board) = fixture_folder(Some(&bus(json!({}))), None);
    assert!(plan_of(&board).unwrap().heights.heights_mm.is_none());
}

#[test]
fn a_project_that_does_not_count_via_heights_sends_none_and_says_so() {
    let group = bus(json!({ "length_groups": [
        { "name": "g", "nets": ["S1", "S2"], "tolerance_mm": 0.5 }] }));
    let (dir, board) = fixture_folder(Some(&group), None);
    let off = POURS_PRO.replacen(
        "\"rules\": {",
        "\"rules\": {\n        \"use_height_for_length_calcs\": false,",
        1,
    );
    assert_ne!(off, POURS_PRO);
    std::fs::write(dir.path().join("b.kicad_pro"), off).unwrap();
    let plan = plan_of(&board).unwrap();
    assert!(plan.heights.heights_mm.is_none());
    assert!(plan
        .heights
        .note
        .as_deref()
        .unwrap()
        .contains("use_height_for_length_calcs"));
    // The group is still tuned.
    assert_eq!(plan.groups.len(), 1);
}

#[test]
fn a_dsn_whose_layers_cannot_be_read_sends_no_heights_and_says_so() {
    let group = bus(json!({ "length_groups": [
        { "name": "g", "nets": ["S1", "S2"], "tolerance_mm": 0.5 }] }));
    let (_dir, board) = fixture_folder(Some(&group), None);
    // The layers are listed against their index, out of order.
    let swapped = POURS_DSN
        .replace("(index 0)", "(index X)")
        .replace("(index 1)", "(index 0)")
        .replace("(index X)", "(index 1)");
    assert_ne!(swapped, POURS_DSN);
    let pairs = PairsPlan::default();
    let plan = super::plan(&board, POURS_BOARD, &swapped, &pairs).unwrap();
    assert!(plan.heights.heights_mm.is_none());
    let note = plan.heights.note.as_deref().unwrap();
    assert!(
        note.contains("layers could not be read") && note.contains("index order"),
        "{note}"
    );
    assert_eq!(plan.groups.len(), 1);
}

#[test]
fn an_intent_that_cannot_be_used_refuses_when_it_declares_lengths_and_notes_otherwise() {
    // It names a group, but a net outside the interface: the record is unusable.
    let declares = bus(json!({ "length_groups": [
        { "name": "g", "nets": ["S1", "S9"], "tolerance_mm": 0.5 }] }));
    let (_dir, board) = fixture_folder(Some(&declares), None);
    let refusal = plan_of(&board).unwrap_err();
    assert!(
        refusal.reason.contains("design_intent.json")
            && refusal.reason.contains("cannot be used")
            && refusal.reason.contains("length_groups"),
        "{}",
        refusal.reason
    );
    assert!(
        refusal.next_step.contains("Repair or move"),
        "{}",
        refusal.next_step
    );
    // The same for a skew key.
    let skew = bus(json!({ "differential_pairs": [
        { "positive": "S1", "negative": "S2", "max_skew_mm": -1.0 }] }));
    let (_dir, board) = fixture_folder(Some(&skew), None);
    assert!(plan_of(&board).unwrap_err().reason.contains("max_skew_mm"));
    // Control: an unusable record that holds neither key keeps D23's note and routes.
    for text in [
        "{ not json".to_string(),
        bus(json!({ "differential_pairs": [{ "positive": "S1", "negative": "S9" }] })),
    ] {
        let (_dir, board) = fixture_folder(Some(&text), None);
        let plan = plan_of(&board).unwrap();
        let note = plan.note.as_deref().expect("a note");
        assert!(
            note.contains("design_intent.json") && note.contains("cannot be used"),
            "{note}"
        );
        assert!(plan.is_empty() && plan.reported());
        let fields = plan.fields(None, None, None);
        assert_eq!(body(&fields)["note"], plan.note.clone().unwrap());
    }
}

#[test]
fn the_names_are_refused_through_the_plan_before_anything_is_routed() {
    // An unresolved net.
    let unresolved = bus(json!({ "length_groups": [
        { "name": "g", "nets": ["S1", "S2"], "tolerance_mm": 0.5 }] }));
    let (_dir, board) = fixture_folder(Some(&unresolved), None);
    let pairs = PairsPlan::default();
    let refusal = super::plan(
        &board,
        POURS_BOARD,
        &POURS_DSN.replace("(net S2", "(net T2"),
        &pairs,
    )
    .unwrap_err();
    assert!(
        refusal.reason.contains("'g'") && refusal.reason.contains("'S2'"),
        "{}",
        refusal.reason
    );
    // A skew whose pair D23 left out (S9 is in no DSN net; the record is valid).
    let lost = intent_text(json!({ "name": "bus", "kind": "bus", "nets": ["S1", "S9"],
        "differential_pairs": [{ "positive": "S1", "negative": "S9", "max_skew_mm": 0.2 }] }));
    let (_dir, board) = fixture_folder(Some(&lost), None);
    let refusal = plan_of(&board).unwrap_err();
    assert!(
        refusal.reason.contains("'S1' and 'S9'") && refusal.reason.contains("no net of the DSN"),
        "{}",
        refusal.reason
    );
    // A DSN that cannot be read, with a group declared.
    let (_dir, board) = fixture_folder(Some(&unresolved), None);
    let refusal = super::plan(&board, POURS_BOARD, "not a dsn (", &pairs).unwrap_err();
    assert!(
        refusal.reason.contains("DSN's nets could not be read"),
        "{}",
        refusal.reason
    );
    // A board that does not parse.
    let refusal = super::plan(&board, "(kicad_pcb (", POURS_DSN, &pairs).unwrap_err();
    assert!(
        refusal.reason.contains("board's nets could not be read"),
        "{}",
        refusal.reason
    );
    // Control: the intent that resolves plans against the same files.
    assert!(plan_of(&fixture_folder(Some(&unresolved), None).1).is_ok());
}

#[test]
fn an_impossible_target_is_refused_through_the_plan() {
    // The fixture's S1 joins R1 and R12, 40 mm or more apart: a 1 mm target is
    // impossible, a 200 mm one is not (and is not checked against a ceiling).
    let with = |target: f64| {
        bus(json!({ "length_groups": [
            { "name": "short", "nets": ["S1", "S2"], "tolerance_mm": 0.5, "target_mm": target }] }))
    };
    let (_dir, board) = fixture_folder(Some(&with(1.0)), None);
    let refusal = plan_of(&board).unwrap_err();
    assert!(
        refusal.reason.contains("'short'") && refusal.reason.contains("cannot be that short"),
        "{}",
        refusal.reason
    );
    assert!(
        refusal.next_step.contains("target_mm"),
        "{}",
        refusal.next_step
    );
    let (_dir, board) = fixture_folder(Some(&with(200.0)), None);
    assert!(plan_of(&board).is_ok());
    // A skew-mode group has no target to miss, however small its tolerance.
    let skew = bus(json!({ "length_groups": [
        { "name": "skew", "nets": ["S1", "S2"], "tolerance_mm": 0.001 }] }));
    let (_dir, board) = fixture_folder(Some(&skew), None);
    assert!(plan_of(&board).is_ok());
}

#[test]
fn the_boards_own_length_and_skew_rules_are_listed_with_or_without_an_intent() {
    let (_dir, board) = fixture_folder(None, Some(DRU));
    let plan = plan_of(&board).unwrap();
    assert_eq!(plan.kicad_rules.len(), 3);
    assert_eq!(plan.kicad_rules[0].name, "ddr_data");
    assert!(plan.reported() && plan.is_empty());
    assert!(plan.kicad_rules_note.is_none());
    // Control: a rules file without any such rule lists none, and reports nothing.
    let (_dir, board) = fixture_folder(
        None,
        Some("(version 1)\n(rule \"c\" (constraint clearance (min 0.3mm)))\n"),
    );
    let plan = plan_of(&board).unwrap();
    assert!(plan.kicad_rules.is_empty() && !plan.reported());
    // A rules file that does not read gets a note, and routing goes on.
    let (_dir, board) = fixture_folder(None, Some("(version 1)\n(rule \"x\" (constraint"));
    let plan = plan_of(&board).unwrap();
    assert!(plan.kicad_rules.is_empty());
    let note = plan.kicad_rules_note.as_deref().expect("a note");
    assert!(
        note.contains("b.kicad_dru") && note.contains("still checks"),
        "{note}"
    );
    assert!(plan.reported());
}

// ─── The refusal as a result ────────────────────────────────────────────────

#[test]
fn the_refusal_is_an_invalid_configuration_error_on_the_intents_path() {
    let refusal = LengthRefusal {
        reason: "The length group 'dq' names the net 'DQ9'.".into(),
        next_step: "Correct that net name.".into(),
    };
    let intent = Path::new("/p/.konnect/design_intent.json");
    let result = refusal_result(&refusal, intent);
    assert!(result.is_error);
    assert_eq!(
        crate::mcp::error::extract_error_kind(&result).as_deref(),
        Some("invalid_configuration")
    );
    let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
        panic!("text");
    };
    let body: Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        body["message"],
        "Nothing was routed. The length group 'dq' names the net 'DQ9'. Correct that net name."
    );
    assert_eq!(body["error"]["path"], "/p/.konnect/design_intent.json");
    assert_eq!(body["error"]["reason"], refusal.reason);
}

// ─── Freerouting (L14) ──────────────────────────────────────────────────────

#[test]
fn freerouting_is_told_when_the_intent_declares_lengths() {
    let group = bus(json!({ "length_groups": [
        { "name": "g", "nets": ["S1", "S2"], "tolerance_mm": 0.5 }] }));
    let (_dir, board) = fixture_folder(Some(&group), None);
    let note = freerouting_note(&board).expect("a note");
    assert!(
        note.contains("does not tune lengths") && note.contains("engine fastroute"),
        "{note}"
    );
    let skew = bus(json!({ "differential_pairs": [
        { "positive": "S1", "negative": "S2", "max_skew_mm": 0.2 }] }));
    let (_dir, board) = fixture_folder(Some(&skew), None);
    assert!(freerouting_note(&board).is_some());
    // Controls: none without a group or a skew, without an intent, or with an unusable one.
    let (_dir, board) = fixture_folder(Some(&bus(json!({}))), None);
    assert!(freerouting_note(&board).is_none());
    let pair = bus(json!({ "differential_pairs": [{ "positive": "S1", "negative": "S2" }] }));
    let (_dir, board) = fixture_folder(Some(&pair), None);
    assert!(freerouting_note(&board).is_none());
    let (_dir, board) = fixture_folder(None, None);
    assert!(freerouting_note(&board).is_none());
    let (_dir, board) = fixture_folder(Some("{ \"length_groups\""), None);
    assert!(freerouting_note(&board).is_none());
    assert!(freerouting_note(Path::new("route-lengths-no-such-board.kicad_pcb")).is_none());
}

// ─── The rule of a group that holds a pair (L25) ────────────────────────────

/// What a winner measured for `plan`'s group `index`: its longest net.
fn longest_of(plan: &LengthPlan, index: usize, longest_mm: f64) -> LengthMeasures {
    LengthMeasures {
        groups: vec![GroupLengths {
            interface: plan.groups[index].interface.clone(),
            name: plan.groups[index].name.clone(),
            longest_mm: Some(longest_mm),
            nets: Vec::new(),
        }],
        ..measures(0.0, &[])
    }
}

#[test]
fn a_group_needs_a_pinned_rule_only_when_it_holds_both_nets_of_a_gated_pair() {
    let plan = two_group_plan();
    let pair = gated("DQS_P", "DQS_N", 0.05, Some("dq_low"));
    assert!(group_needs_pinned_rule(
        &plan.groups[0],
        std::slice::from_ref(&pair)
    ));
    // Not the group that holds none of its nets, nor a call with no gated pair.
    assert!(!group_needs_pinned_rule(
        &plan.groups[1],
        std::slice::from_ref(&pair)
    ));
    assert!(!group_needs_pinned_rule(&plan.groups[0], &[]));
    // One net of the pair in the group is not the pair.
    let half = GatedPair {
        dsn_negative: "/ELSEWHERE".to_string(),
        ..pair.clone()
    };
    assert!(!group_needs_pinned_rule(&plan.groups[0], &[half]));
    let other_half = GatedPair {
        dsn_positive: "/ELSEWHERE".to_string(),
        ..pair
    };
    assert!(!group_needs_pinned_rule(&plan.groups[0], &[other_half]));
}

#[test]
fn a_pinned_skew_rule_is_a_length_window_below_the_winners_longest_net() {
    let plan = two_group_plan();
    let names = rule_names(&plan.groups);
    let rule = kicad_group_rule_pinned(&plan.groups[0], &names[0], 42.0);
    // min L - T - 0.0001, max L + 0.0001; the group's nets as its condition.
    assert_eq!(
        rule,
        "(rule \"1kopper_len_ram_dq_low\" (constraint length (min 41.4999mm) (max 42.0001mm)) (condition \"A.NetName == '/DQ0' || A.NetName == '/DQ1' || A.NetName == '/DQS_P' || A.NetName == '/DQS_N'\"))"
    );
    let listed = kicad_length_rules(&format!("(version 1)\n{rule}\n")).unwrap();
    assert_eq!(
        listed[0].constraints,
        vec!["(constraint length (min 41.4999mm) (max 42.0001mm))"]
    );
    // The longest net moves the window.
    assert!(kicad_group_rule_pinned(&plan.groups[0], &names[0], 40.25)
        .contains("(min 39.7499mm) (max 40.2501mm)"));
    // A target group already is a length rule, and stays its own.
    assert_eq!(
        kicad_group_rule_pinned(&plan.groups[1], &names[1], 99.0),
        kicad_rule_suggestion(&plan.groups[1], &names[1])
    );
}

#[test]
fn the_next_step_of_a_skew_group_with_a_pair_is_the_pinned_length_rule_beside_the_pair_rule() {
    let mut plan = two_group_plan();
    plan.pairs = vec![gated("DQS_P", "DQS_N", 0.05, Some("dq_low"))];
    let names = rule_names(&plan.groups);
    let steps = plan.next_steps(Some(&longest_of(&plan, 0, 42.0)), 0);
    // The batch, then the one rule that needs a word of its own.
    assert_eq!(steps.len(), 2, "{steps:?}");
    let rules = batch(&steps[0]);
    assert_eq!(
        rules[0].1,
        kicad_group_rule_pinned(&plan.groups[0], &names[0], 42.0)
    );
    assert!(!rules[0].1.contains("constraint skew"), "{}", rules[0].1);
    assert!(
        steps[1].contains("pinned to the routed lengths")
            && steps[1].contains(&format!("Rule \"{}\"", names[0])),
        "{}",
        steps[1]
    );
    assert!(steps[1].contains("42 mm"), "{}", steps[1]);
    // The pair keeps its skew rule, and the group without a pair its own rule.
    assert_eq!(rules[2].1, kicad_pair_rule_suggestion(&plan.pairs[0]));
    assert_eq!(rules[1].1, suggested(&plan, 1));
    assert!(!steps[1].contains(&names[1]), "{}", steps[1]);
}

#[test]
fn without_a_winner_the_skew_rule_stays_with_a_note_that_kicad_checks_only_the_pair_rules() {
    let mut plan = two_group_plan();
    plan.pairs = vec![gated("DQS_P", "DQS_N", 0.05, Some("dq_low"))];
    // No winner, or a winner that measured no longest net for the group.
    for winner in [None, Some(measures(0.0, &[]))] {
        let steps = plan.next_steps(winner.as_ref(), 0);
        let rules = batch(&steps[0]);
        assert_eq!(rules[0].1, suggested(&plan, 0));
        assert!(rules[0].1.contains("constraint skew"), "{}", rules[0].1);
        assert!(
            steps[1].contains("check only the pair rules"),
            "{}",
            steps[1]
        );
        assert!(
            !steps[1].contains("pinned to the routed lengths"),
            "{}",
            steps[1]
        );
    }
}

#[test]
fn a_group_without_a_declared_pair_skew_or_in_target_mode_keeps_its_own_rule() {
    // No gated pair: today's skew rule, no note, whatever the winner measured.
    let plan = two_group_plan();
    let measured = longest_of(&plan, 0, 42.0);
    let steps = plan.next_steps(Some(&measured), 0);
    assert_eq!(steps.len(), 1, "no note: {steps:?}");
    let rules = batch(&steps[0]);
    assert_eq!(rules[0].1, suggested(&plan, 0));
    assert!(rules[0].1.contains("constraint skew"));
    // A target group that holds a pair is a length rule already: unchanged, no note.
    let mut plan = two_group_plan();
    plan.pairs = vec![gated("A0", "A1", 0.05, Some("addr"))];
    let steps = plan.next_steps(Some(&longest_of(&plan, 1, 42.0)), 0);
    assert_eq!(steps.len(), 1, "no note: {steps:?}");
    assert_eq!(batch(&steps[0])[1].1, suggested(&plan, 1));
}

#[test]
fn a_pinned_rules_minimum_is_never_negative() {
    // A longest net shorter than the tolerance: the window starts at 0, not below it.
    let plan = two_group_plan();
    let rule = kicad_group_rule_pinned(&plan.groups[0], "n", 0.3);
    assert!(rule.contains("(min 0mm) (max 0.3001mm)"), "{rule}");
    assert!(!rule.contains("-"), "{rule}");
    // Control: a longer net keeps its own minimum.
    assert!(kicad_group_rule_pinned(&plan.groups[0], "n", 0.6).contains("(min 0.0999mm)"));
}

// ─── The result's summary of lengths ─────────────────────────────────────────

fn pair_names(pairs: &[LikelyPair]) -> Vec<(&str, &str, &str)> {
    pairs
        .iter()
        .map(|p| (p.positive.as_str(), p.negative.as_str(), p.pattern))
        .collect()
}

#[test]
fn nets_named_as_two_halves_of_a_signal_are_suggested_as_pairs() {
    let nets = set(&[
        "/CAN_H", "/CAN_L", "/bus/TX+", "/bus/TX-", "USB_DP", "USB_DM", "RS485_A", "RS485_B",
        "CANH", "CANL", "LVDS_P", "LVDS_N", "SDA", "SCL",
    ]);
    let pairs = likely_pairs(&nets, &BTreeSet::new());
    assert_eq!(
        pair_names(&pairs),
        [
            ("/CAN_H", "/CAN_L", "_H/_L"),
            ("/bus/TX+", "/bus/TX-", "+/-"),
            ("CANH", "CANL", "CANH/CANL"),
            ("LVDS_P", "LVDS_N", "_P/_N"),
            ("RS485_A", "RS485_B", "_A/_B (RS-485)"),
            ("USB_DP", "USB_DM", "_DP/_DM"),
        ]
    );
}

#[test]
fn rails_declared_pairs_and_lone_halves_are_never_suggested() {
    let nets = set(&[
        // Rails and grounds named like halves.
        "V+", "V-", "/VS+", "/VS-", "+5V_H", "+5V_L", "GND_P", "GND_N",
        // Encoder phases: A and B without 485 are two signals.
        "ENC_A", "ENC_B", // A half with no partner.
        "D+",    // A declared pair.
        "CAN_H", "CAN_L", // No base before the ending.
        "_H", "_L", "H", "L",
    ]);
    let declared = set(&["CAN_H", "CAN_L"]);
    let pairs = likely_pairs(&nets, &declared);
    assert!(pairs.is_empty(), "{pairs:?}");
    // The negative control of the declared rule: undeclared, the pair is found.
    assert_eq!(
        pair_names(&likely_pairs(&nets, &BTreeSet::new())),
        [("CAN_H", "CAN_L", "_H/_L")]
    );
    // And of the 485 rule: the same endings on an RS-485 name are a pair.
    assert_eq!(
        pair_names(&likely_pairs(&set(&["/485_A", "/485_B"]), &BTreeSet::new())),
        [("/485_A", "/485_B", "_A/_B (RS-485)")]
    );
}

#[test]
fn a_plan_that_checks_nothing_says_so_and_lists_the_likely_pairs() {
    let plan = LengthPlan::default();
    assert!(!plan.checked());
    let likely = likely_pairs(&set(&["CAN_H", "CAN_L"]), &BTreeSet::new());
    let body = plan.summary_json(Some(true), &likely);
    assert_eq!(body["checked"], false);
    let line = body["line"].as_str().unwrap();
    assert!(line.starts_with("Lengths: not checked."), "{line}");
    assert!(line.contains("`status` does not cover lengths"), "{line}");
    assert_eq!(body["likely_pairs"][0]["positive"], "CAN_H");
    assert_eq!(body["likely_pairs"][0]["pattern"], "_H/_L");
    assert!(body["next_step"]
        .as_str()
        .unwrap()
        .contains("set_design_intent"));
    // No likely pair: no list and no step.
    let bare = plan.summary_json(None, &[]);
    assert!(bare.get("likely_pairs").is_none() && bare.get("next_step").is_none());
    // An unusable intent's note is carried.
    let noted = LengthPlan {
        note: Some("the design intent x cannot be used (bad)".to_string()),
        ..LengthPlan::default()
    };
    assert!(noted.summary_json(None, &[])["line"]
        .as_str()
        .unwrap()
        .contains("The design intent was not read: the design intent x cannot be used (bad)."));
}

#[test]
fn a_plan_with_a_group_or_only_kicad_rules_is_checked() {
    let plan = two_group_plan();
    assert!(plan.checked());
    let body = plan.summary_json(Some(true), &[]);
    assert_eq!(body["checked"], true);
    assert!(body["line"]
        .as_str()
        .unwrap()
        .contains("every checked length is inside its window"));
    assert!(plan.summary_json(Some(false), &[])["line"]
        .as_str()
        .unwrap()
        .contains("outside its window"));
    // KiCad's own rules alone are checked by the judge (G4(c)).
    let rules_only = LengthPlan {
        kicad_rules: vec![KicadLengthRule {
            name: "len".to_string(),
            constraints: vec!["(constraint length (max 10mm))".to_string()],
            condition: None,
        }],
        ..LengthPlan::default()
    };
    assert!(rules_only.checked());
    assert_eq!(rules_only.summary_json(None, &[])["checked"], true);
    // A likely list given to a checked plan is not shown.
    let likely = likely_pairs(&set(&["CAN_H", "CAN_L"]), &BTreeSet::new());
    assert!(plan
        .summary_json(None, &likely)
        .get("likely_pairs")
        .is_none());
}

#[test]
fn a_net_is_in_one_suggested_pair_at_most() {
    // Two spellings of one half both find the same partner: one pair only.
    let pairs = likely_pairs(&set(&["/CAN_H", "/can_h", "/CAN_L"]), &BTreeSet::new());
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    assert_eq!(pairs[0].negative, "/CAN_L");
}

#[test]
fn high_low_pairs_count_only_on_can_and_terminal_pairs_never() {
    let nets = set(&[
        "PWM_H",
        "PWM_L",
        "/GATE_A_H",
        "/GATE_A_L",
        "BAT+",
        "BAT-",
        "VREF+",
        "VREF-",
        "LED+",
        "LED-",
        "/can2/CAN2_H",
        "/can2/CAN2_L",
    ]);
    assert_eq!(
        pair_names(&likely_pairs(&nets, &BTreeSet::new())),
        [("/can2/CAN2_H", "/can2/CAN2_L", "_H/_L")]
    );
}

#[test]
fn the_not_checked_line_names_what_could_not_be_read() {
    let line = |plan: &LengthPlan| {
        plan.summary_json(None, &[])["line"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let plain = line(&LengthPlan::default());
    assert!(plain.contains("declares no length group") && plain.contains("no length or skew rule"));
    let unreadable = line(&LengthPlan {
        kicad_rules_note: Some("the rules file x could not be listed (bad)".to_string()),
        ..LengthPlan::default()
    });
    assert!(
        unreadable.contains("could not be listed: the rules file x"),
        "{unreadable}"
    );
    assert!(
        !unreadable.contains("no length or skew rule"),
        "{unreadable}"
    );
    let no_intent = line(&LengthPlan {
        note: Some("the design intent x cannot be used (bad)".to_string()),
        ..LengthPlan::default()
    });
    assert!(
        no_intent.contains("was not read: the design intent x"),
        "{no_intent}"
    );
    assert!(
        !no_intent.contains("declares no length group"),
        "{no_intent}"
    );
}

/// Row RW: more rules than one call takes are split into calls of at most
/// `MAX_RULES_PER_CALL`, each a batch of its own, every rule in exactly one.
#[test]
fn more_rules_than_one_call_takes_are_split_into_several_calls() {
    let plan = LengthPlan {
        pairs: (0..custom_rules::MAX_RULES_PER_CALL + 1)
            .map(|i| gated(&format!("P{i}"), &format!("N{i}"), 0.05, None))
            .collect(),
        ..LengthPlan::default()
    };
    let steps = plan.next_steps(None, 0);
    assert_eq!(steps.len(), 2, "{steps:?}");
    assert!(steps[0].contains("call 1 of 2") && steps[1].contains("call 2 of 2"));
    let sizes: Vec<usize> = steps.iter().map(|step| batch(step).len()).collect();
    assert_eq!(sizes, vec![custom_rules::MAX_RULES_PER_CALL, 1]);
}

/// Row FF: each `create_custom_drc_rule` line carries the rules of its call, exactly
/// the batch its text gives; every other line carries none.
#[test]
fn each_rule_line_carries_its_batch_in_structured_form() {
    let plan = LengthPlan {
        pairs: (0..custom_rules::MAX_RULES_PER_CALL + 1)
            .map(|i| gated(&format!("P{i}"), &format!("N{i}"), 0.05, None))
            .collect(),
        ..LengthPlan::default()
    };
    let entries = plan.next_step_entries(Some(&measures(1.0, &[])), 1);
    let lines: Vec<String> = entries.iter().map(|(line, _)| line.clone()).collect();
    assert_eq!(lines, plan.next_steps(Some(&measures(1.0, &[])), 1));
    let mut batches = 0;
    for (line, rules) in &entries {
        if line.contains("call create_custom_drc_rule") {
            let rules: Vec<(String, String)> = rules
                .as_ref()
                .expect("a rule line has its rules")
                .iter()
                .map(|rule| {
                    (
                        rule["name"].as_str().unwrap().to_string(),
                        rule["rule"].as_str().unwrap().to_string(),
                    )
                })
                .collect();
            assert_eq!(rules, batch(line), "{line}");
            batches += 1;
        } else {
            assert!(rules.is_none(), "{line}");
        }
    }
    assert_eq!(batches, 2);
}
