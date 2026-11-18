//! Tests of `place_judge`: the screen, the placement gates and the rank key.

use super::*;
use crate::place_held::tests::{board, evaluate_text, pad, parse, Fp, Shape};
use crate::place_held::{held_set, HoldRules};
use std::collections::BTreeSet;

const OUTLINE: Shape = Shape::Rect(0.0, 0.0, 60.0, 40.0);

fn held_of(text: &str) -> HeldSet {
    held_set(text, &BTreeSet::new(), &HoldRules::default()).expect("the held set")
}

/// `evaluate` of `text` with every held part as an edge mount, then its screen.
fn screen_of(text: &str, decoupling_violations: usize) -> (Screen, serde_json::Value, HeldSet) {
    let set = held_of(text);
    let held: Vec<&str> = set.held.keys().map(String::as_str).collect();
    let evaluated = evaluate_text(text, &held);
    let screen = screen(&evaluated, &set, decoupling_violations, None).expect("a screen");
    (screen, evaluated, set)
}

fn counts(entries: &[(&str, usize)]) -> BTreeMap<String, usize> {
    entries.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn plain_screen() -> Screen {
    Screen {
        verdict: "pass".into(),
        score: 90,
        score_incomplete: false,
        hard_failures: 0,
        movable_hard_failures: 0,
        outline_unproven: false,
        decoupling_violations: 0,
        connector_violations: 0,
        wirelength_mm: Some(100.0),
        crossings: Some(0),
        group_spread: None,
    }
}

fn measures(unconnected: usize, signal_mm: f64) -> Measures {
    Measures {
        unconnected,
        violations: BTreeMap::new(),
        routed_share: Some(1.0),
        audit_complete: Some(true),
        acute: Some(0.0),
        right_angles: Some(0.0),
        copper_quality: Some(90.0),
        nets_undersized: None,
        narrow_paths: None,
        signal_mm,
        power_mm: 0.0,
        pour_track_mm: 0.0,
        vias_off_pour: 0,
        below_floor: 0,
        width_rows: Vec::new(),
        longest_below_mm: BTreeMap::new(),
        stitching_vias: 0,
        route_s: 10.0,
    }
}

fn open_gates() -> Gates {
    Gates {
        g1: Vec::new(),
        g2: Vec::new(),
        g3: Vec::new(),
    }
}

fn routed(m: Measures, gates: Gates) -> RouteResult {
    RouteResult::Routed {
        gates,
        measures: m,
        unrouted: BTreeMap::new(),
        status: "ok".into(),
        call: serde_json::Value::Null,
    }
}

fn passing() -> PlacementGates {
    PlacementGates {
        p1: Vec::new(),
        p2: Vec::new(),
    }
}

fn judged(id: usize, screen: Screen, placement: PlacementGates, route: RouteResult) -> Judged {
    Judged {
        id,
        screen,
        placement,
        route,
        silk: Some(0),
    }
}

/// A candidate that passes everything and routed completely, `signal_mm` of signal copper.
fn complete(id: usize, signal_mm: f64) -> Judged {
    judged(
        id,
        plain_screen(),
        passing(),
        routed(measures(0, signal_mm), open_gates()),
    )
}

// ─── The screen ──────────────────────────────────────────────────────────────

#[test]
fn the_screen_reads_the_evaluation() {
    // R1 and R2 overlap (movable); J1 is a connector far from every edge.
    let parts = vec![
        Fp::new("J1", 30.0, 20.0)
            .court(2.0, 2.0)
            .pad(pad("1", 0.0, 0.0, Some("A"))),
        Fp::new("R1", 10.0, 10.0).two_pad("A", "B"),
        Fp::new("R2", 10.5, 10.0).two_pad("B", "C"),
    ];
    let (screen, evaluated, _) = screen_of(&board(OUTLINE, &parts), 3);
    assert_eq!(screen.verdict, "hard_fail");
    assert_eq!(screen.hard_failures, 1);
    assert_eq!(screen.movable_hard_failures, 1);
    assert!(!screen.outline_unproven);
    assert_eq!(screen.decoupling_violations, 3);
    assert_eq!(
        screen.connector_violations, 1,
        "{:?}",
        evaluated["deductions"]
    );
    assert_eq!(screen.score, evaluated["score"].as_i64().unwrap());
    assert_eq!(
        screen.score_incomplete,
        evaluated["score_incomplete"].as_bool().unwrap()
    );
    assert_eq!(
        screen.wirelength_mm,
        evaluated["routing_metrics"]["wirelength_mm"].as_f64()
    );
    assert_eq!(screen.crossings, Some(0));
    assert_eq!(screen.group_spread, None);

    // A board with no outline is unproven, not passed.
    let no_outline = board(OUTLINE, &parts).replace("Edge.Cuts", "Dwgs.User");
    let set = held_of(&board(OUTLINE, &parts));
    let evaluated = evaluate_text(&no_outline, &[]);
    let unproven = super::screen(&evaluated, &set, 0, Some(0.25)).unwrap();
    assert!(unproven.outline_unproven);
    assert_eq!(unproven.group_spread, Some(0.25));
}

#[test]
fn a_missing_field_is_an_error() {
    let parts = vec![Fp::new("R1", 10.0, 10.0).two_pad("A", "B")];
    let text = board(OUTLINE, &parts);
    let set = held_of(&text);
    let good = evaluate_text(&text, &[]);
    assert!(screen(&good, &set, 0, None).is_ok());
    for field in [
        "verdict",
        "score",
        "score_incomplete",
        "hard_failures",
        "outline_missing",
        "outline_unproven",
        "deductions",
        "routing_metrics",
    ] {
        let mut broken = good.clone();
        broken.as_object_mut().unwrap().remove(field);
        let error = screen(&broken, &set, 0, None).unwrap_err().to_string();
        assert!(error.contains(field), "{field}: {error}");
    }
}

#[test]
fn a_hard_failure_counts_as_movable_when_any_reference_is() {
    let parts = vec![
        Fp::new("J1", 20.0, 20.0).pad(pad("1", 0.0, 0.0, Some("A"))),
        Fp::new("J2", 20.5, 20.0).pad(pad("1", 0.0, 0.0, Some("A"))),
        Fp::new("R1", 20.2, 20.0).two_pad("A", "B"),
    ];
    // R1 overlaps both held connectors, and they overlap each other: 3 hard failures,
    // 2 of them name R1.
    let (screen, _, set) = screen_of(&board(OUTLINE, &parts), 0);
    assert!(set.held.contains_key("J1") && set.movable.contains("R1"));
    assert_eq!(screen.hard_failures, 3);
    assert_eq!(screen.movable_hard_failures, 2);
}

#[test]
fn screen_key_orders_verdict_completeness_requirements_then_airwires() {
    let base = plain_screen();
    let p1_failed = Screen {
        movable_hard_failures: 1,
        ..base.clone()
    };
    let unproven = Screen {
        outline_unproven: true,
        ..base.clone()
    };
    let incomplete = Screen {
        score_incomplete: true,
        ..base.clone()
    };
    let decoupling = Screen {
        decoupling_violations: 1,
        ..base.clone()
    };
    let connector = Screen {
        connector_violations: 1,
        ..base.clone()
    };
    let crossing = Screen {
        crossings: Some(1),
        ..base.clone()
    };
    let longer = Screen {
        wirelength_mm: Some(100.2),
        ..base.clone()
    };
    let unknown_wire = Screen {
        wirelength_mm: None,
        ..base.clone()
    };
    let unknown_crossings = Screen {
        crossings: None,
        ..base.clone()
    };
    // Each is worse than the one before it, and every later key field is dominated.
    let ladder = [
        base.clone(),
        Screen {
            wirelength_mm: Some(100.04),
            ..base.clone()
        },
        longer,
        unknown_wire,
        crossing,
        unknown_crossings,
        connector,
        decoupling,
        incomplete,
        p1_failed.clone(),
    ];
    let keys: Vec<ScreenKey> = ladder.iter().map(screen_key).collect();
    // The 0.04 mm change is below the 0.1 mm step; every other rung is strictly worse.
    assert_eq!(keys[0], keys[1]);
    for pair in keys.windows(2).skip(1) {
        assert!(pair[0] < pair[1], "{pair:?}");
    }
    // An unproven outline is a P1 failure too.
    assert_eq!(screen_key(&unproven), screen_key(&p1_failed));
    // A worse requirement outweighs any airwire advantage.
    let bad_but_short = Screen {
        decoupling_violations: 1,
        wirelength_mm: Some(1.0),
        crossings: Some(0),
        ..base.clone()
    };
    let good_but_long = Screen {
        wirelength_mm: Some(9000.0),
        crossings: Some(50),
        ..base
    };
    assert!(screen_key(&good_but_long) < screen_key(&bad_but_short));
}

// ─── The placement gates ─────────────────────────────────────────────────────

#[test]
fn p2_gates_physical_types_only() {
    let input = counts(&[
        ("courtyards_overlap", 2),
        ("unconnected_items", 10),
        ("silk_overlap", 1),
    ]);
    let screen = plain_screen();
    let gates =
        |unrouted: &BTreeMap<String, usize>| placement_gates(&screen, Some(unrouted), &input);
    // The input's own counts pass.
    assert!(gates(&input).pass());
    // One more courtyards_overlap than the input fails, and names the type.
    let worse = gates(&counts(&[
        ("courtyards_overlap", 3),
        ("unconnected_items", 10),
        ("silk_overlap", 1),
    ]));
    assert!(!worse.pass());
    assert!(
        worse.p2.len() == 1 && worse.p2[0].contains("courtyards_overlap"),
        "{worse:?}"
    );
    // Five more silk_overlap pass: silk is ordered, not gated.
    assert!(gates(&counts(&[("courtyards_overlap", 2), ("silk_overlap", 6)])).pass());
    // More open connections pass too: an unrouted board has them all.
    assert!(gates(&counts(&[("unconnected_items", 500)])).pass());
    // One more of an unknown type fails: a type KiCad adds later is physical.
    assert!(!gates(&counts(&[("some_future_type", 1)])).pass());
    // A physical type the input did not have at all fails at 1.
    assert!(!gates(&counts(&[("clearance", 1)])).pass());
    // Fewer pass.
    assert!(gates(&counts(&[("courtyards_overlap", 0)])).pass());
}

#[test]
fn p2_without_an_unrouted_check_fails() {
    let gates = placement_gates(&plain_screen(), None, &BTreeMap::new());
    assert!(!gates.pass());
    assert_eq!(gates.p1, Vec::<String>::new());
    assert_eq!(gates.p2.len(), 1);
}

#[test]
fn a_held_part_over_the_edge_or_on_another_held_part_fails_no_candidate() {
    // J1 hangs over the right edge and J2 sits on J1: both held, so neither is the
    // candidate's doing.
    let parts = vec![
        Fp::new("J1", 59.5, 20.0)
            .court(2.0, 3.0)
            .pad(pad("1", 0.0, 0.0, Some("A"))),
        Fp::new("J2", 59.0, 20.0)
            .court(2.0, 3.0)
            .pad(pad("1", 0.0, 0.0, Some("A"))),
        Fp::new("R1", 20.0, 20.0).two_pad("A", "B"),
    ];
    let (screen, evaluated, _) = screen_of(&board(OUTLINE, &parts), 0);
    assert_eq!(
        screen.hard_failures, 1,
        "the held overlap is a hard failure"
    );
    assert_eq!(screen.movable_hard_failures, 0);
    assert!(!evaluated["edge_mount_listed"]
        .as_array()
        .unwrap()
        .is_empty());
    let gates = placement_gates(&screen, Some(&BTreeMap::new()), &BTreeMap::new());
    assert!(gates.pass(), "{gates:?}");

    // The same overhang on a movable part fails P1.
    let mut movable_over = parts.clone();
    movable_over[2] = Fp::new("R1", 59.9, 30.0).two_pad("A", "B");
    let (screen, _, _) = screen_of(&board(OUTLINE, &movable_over), 0);
    assert_eq!(screen.movable_hard_failures, 1);
    let gates = placement_gates(&screen, Some(&BTreeMap::new()), &BTreeMap::new());
    assert!(!gates.pass());
    assert_eq!(gates.p1.len(), 1);
    assert!(gates.p2.is_empty());
}

#[test]
fn an_unclosed_outline_fails_p1() {
    let screen = Screen {
        outline_unproven: true,
        ..plain_screen()
    };
    let gates = placement_gates(&screen, Some(&BTreeMap::new()), &BTreeMap::new());
    assert!(!gates.pass());
    assert!(gates.p1[0].contains("outline"));
}

// ─── Decoupling pairs ────────────────────────────────────────────────────────

/// U1 with 8 other parts on VCC (so the rail is above the fanout limit), and C1 at `c1`.
fn decoupling_board(c1: (f64, f64)) -> String {
    let mut parts = vec![
        Fp::new("U1", 30.0, 20.0)
            .court(2.0, 2.0)
            .pad(pad("1", -1.5, -0.5, Some("VCC")))
            .pad(pad("2", 1.5, 0.5, Some("GND"))),
        Fp::new("C1", c1.0, c1.1)
            .value("100nF")
            .two_pad("VCC", "GND"),
    ];
    for k in 0..8 {
        parts.push(
            Fp::new(&format!("R{}", k + 1), 8.0 + 5.0 * k as f64, 35.0).two_pad("VCC", "GND"),
        );
    }
    board(OUTLINE, &parts)
}

fn pairs_of(text: &str) -> Vec<DecouplingPair> {
    let set = held_of(text);
    let held: Vec<&str> = set.held.keys().map(String::as_str).collect();
    crate::place_held::decoupling_pairs(&evaluate_text(text, &held)).unwrap()
}

#[test]
fn moving_a_capacitor_away_is_a_decoupling_violation() {
    let near = decoupling_board((27.0, 19.5));
    let pairs = pairs_of(&near);
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    assert_eq!(pairs[0].cap, "C1");
    // At its pad's side it counts nothing.
    assert_eq!(pair_violations(&parse(&near), &pairs), 0);

    // Far from every IC: the candidate's own associations drop it (the rail has more
    // than 8 references and the distance is beyond the limit), so the score sees no
    // violation, but the fixed list does.
    let far = decoupling_board((5.0, 5.0));
    let evaluated = evaluate_text(&far, &[]);
    let associations = evaluated["decoupling_associations"].as_array().unwrap();
    assert!(
        associations.iter().all(|a| a["reference"] != "C1"),
        "the moved capacitor is still associated: {associations:?}"
    );
    assert_eq!(
        evaluated["deductions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["kind"] == "decoupling")
            .count(),
        0
    );
    assert_eq!(pair_violations(&parse(&far), &pairs), 1);
    // Pad 2.4 mm from the IC pad is within the 2.5 mm limit, 3.0 mm is not.
    let inside = decoupling_board((26.6, 19.5));
    assert_eq!(pair_violations(&parse(&inside), &pairs), 0);
    let outside = decoupling_board((26.0, 19.5));
    assert_eq!(pair_violations(&parse(&outside), &pairs), 1);
}

#[test]
fn a_pair_whose_pads_are_missing_is_a_violation() {
    let text = decoupling_board((27.0, 19.5));
    let tree = parse(&text);
    let good = pairs_of(&text);
    assert_eq!(pair_violations(&tree, &good), 0);
    let with = |change: &dyn Fn(&mut DecouplingPair)| {
        let mut pair = good[0].clone();
        change(&mut pair);
        pair_violations(&tree, &[pair])
    };
    assert_eq!(
        with(&|p| p.ic_pad = "99".into()),
        1,
        "an IC pad that is not there"
    );
    assert_eq!(
        with(&|p| p.cap_pad = "99".into()),
        1,
        "a capacitor pad that is not there"
    );
    assert_eq!(
        with(&|p| p.cap = "C99".into()),
        1,
        "a capacitor that is not there"
    );
    assert_eq!(with(&|p| p.ic = "U99".into()), 1, "an IC that is not there");
    // A pad number that two pads share is not found once, whichever of them is first
    // (the second sits beside the first, so both are within the limit).
    let doubled = text.replacen(
        "(pad \"2\" smd rect (at 1.5 0.5)",
        "(pad \"1\" smd rect (at -1.4 -0.5)",
        1,
    );
    assert_ne!(doubled, text);
    assert_eq!(pair_violations(&parse(&doubled), &good), 1);
    // An empty list measures nothing.
    assert_eq!(pair_violations(&tree, &[]), 0);
}

// ─── The rank key ────────────────────────────────────────────────────────────

#[test]
fn a_shorter_candidate_with_an_open_connection_ranks_below_a_complete_one() {
    let complete_long = complete(1, 200.0);
    let open_short = judged(
        2,
        plain_screen(),
        passing(),
        routed(measures(2, 100.0), open_gates()),
    );
    let keys = rank(&[&complete_long, &open_short]);
    // The open one is shorter: its length step is the smaller.
    assert!(keys[1][9] < keys[0][9], "{keys:?}");
    // But it left connections open, and that comes first.
    assert_eq!((keys[0][3], keys[1][3]), (0, 2));
    assert!(keys[0] < keys[1]);
    assert_eq!(order(&[&open_short, &complete_long]), vec![1, 2]);
}

#[test]
fn an_illegal_placement_does_not_set_the_length_steps() {
    let a = complete(1, 100.0);
    let b = complete(2, 120.0);
    let illegal = judged(
        3,
        Screen {
            movable_hard_failures: 2,
            ..plain_screen()
        },
        PlacementGates {
            p1: vec!["two overlaps".into()],
            p2: Vec::new(),
        },
        routed(measures(0, 50.0), open_gates()),
    );
    let without = rank(&[&a, &b]);
    let with = rank(&[&a, &b, &illegal]);
    assert_eq!(with[0], without[0]);
    assert_eq!(with[1], without[1]);
    // Its own key is stepped against the legal pool, and it ranks last (P1 failed).
    assert_eq!(with[2][0], 1);
    assert!(with[2] > with[0] && with[2] > with[1]);
    assert_eq!(
        with[2][9], 25,
        "50 mm against the 100 mm reference of the legal pool"
    );

    // A candidate that fails G1 does not set the steps either.
    let incomplete = judged(
        4,
        plain_screen(),
        passing(),
        routed(
            measures(5, 30.0),
            Gates {
                g1: vec!["5 open".into()],
                g2: Vec::new(),
                g3: Vec::new(),
            },
        ),
    );
    let with = rank(&[&a, &b, &incomplete]);
    assert_eq!(with[0], without[0]);
    assert_eq!(with[1], without[1]);
}

#[test]
fn the_pool_is_the_candidates_that_pass_p1_p2_and_g1() {
    // Nothing is in the pool: the steps fall back to all the routed runs.
    let failed = PlacementGates {
        p1: vec!["x".into()],
        p2: Vec::new(),
    };
    let a = judged(
        1,
        plain_screen(),
        failed.clone(),
        routed(measures(0, 100.0), open_gates()),
    );
    let b = judged(
        2,
        plain_screen(),
        failed,
        routed(measures(0, 150.0), open_gates()),
    );
    let keys = rank(&[&a, &b]);
    assert!(keys[0][9] < keys[1][9], "{keys:?}");
    // A P2 failure alone also keeps a run out of the pool.
    let p2_failed = judged(
        3,
        plain_screen(),
        PlacementGates {
            p1: Vec::new(),
            p2: vec!["clearance".into()],
        },
        routed(measures(0, 10.0), open_gates()),
    );
    let ok = complete(4, 100.0);
    let keys = rank(&[&ok, &p2_failed]);
    assert_eq!(keys[0][9], 50, "100 mm against itself is step 1/0.02");
}

#[test]
fn a_decoupling_violation_ranks_below_any_longer_route() {
    let long_but_right = complete(1, 400.0);
    let mut short_but_wrong = complete(2, 100.0);
    short_but_wrong.screen.decoupling_violations = 1;
    let keys = rank(&[&long_but_right, &short_but_wrong]);
    assert!(keys[1][9] < keys[0][9], "the violating one is shorter");
    assert!(keys[0] < keys[1]);
    // Likewise a connector violation.
    let mut wrong_connector = complete(3, 100.0);
    wrong_connector.screen.connector_violations = 1;
    let keys = rank(&[&long_but_right, &wrong_connector]);
    assert!(keys[0] < keys[1]);
    // Decoupling comes before the connector.
    let mut both = complete(4, 100.0);
    both.screen.decoupling_violations = 1;
    let mut connector_only = complete(5, 400.0);
    connector_only.screen.connector_violations = 3;
    assert_eq!(order(&[&both, &connector_only]), vec![5, 4]);
}

#[test]
fn ties_go_to_the_lower_id() {
    let low = complete(3, 100.0);
    let high = complete(7, 100.0);
    let keys = rank(&[&high, &low]);
    assert_eq!(keys[0], keys[1], "identical candidates tie");
    assert_eq!(order(&[&high, &low]), vec![3, 7]);
    assert_eq!(order(&[&low, &high]), vec![3, 7]);
    // A strictly better candidate with the higher id still wins.
    let better = complete(9, 90.0);
    let mut better_screen = better.clone();
    better_screen.screen.score = 95;
    assert_eq!(order(&[&low, &better_screen]), vec![9, 3]);
}

#[test]
fn an_unrouted_candidate_ranks_below_every_routed_one() {
    let routed_one = complete(1, 100.0);
    let mut failed_gates = complete(2, 100.0);
    failed_gates.route = routed(
        measures(0, 100.0),
        Gates {
            g1: Vec::new(),
            g2: vec!["g2".into()],
            g3: vec!["g3".into()],
        },
    );
    let no_winner = judged(
        3,
        plain_screen(),
        passing(),
        RouteResult::NoWinner {
            unrouted: None,
            reason: "none judged".into(),
        },
    );
    let not_routed = judged(
        4,
        plain_screen(),
        passing(),
        RouteResult::NotRouted {
            why: "screened_out",
        },
    );
    let failed = judged(
        5,
        plain_screen(),
        passing(),
        RouteResult::Failed {
            stage: "export",
            reason: "x".into(),
        },
    );
    let keys = rank(&[&routed_one, &failed_gates, &no_winner, &not_routed, &failed]);
    assert_eq!(keys[0][1], 0);
    assert!(keys[2..].iter().all(|k| k[1] == 1));
    assert_eq!(keys[2][3], i64::MAX);
    assert_eq!(keys[2][8..14], [i64::MAX; 6]);
    assert_eq!(
        order(&[&failed, &not_routed, &no_winner, &failed_gates, &routed_one]),
        vec![1, 2, 3, 4, 5]
    );
    // A candidate that fails P1 ranks below one that does not, routed or not.
    let illegal_routed = judged(
        6,
        plain_screen(),
        PlacementGates {
            p1: vec!["x".into()],
            p2: Vec::new(),
        },
        routed(measures(0, 100.0), open_gates()),
    );
    assert_eq!(order(&[&illegal_routed, &not_routed]), vec![4, 6]);
}

#[test]
fn the_key_ends_with_silk_and_the_score() {
    let mut quiet = complete(1, 100.0);
    quiet.silk = Some(0);
    let mut noisy = complete(2, 100.0);
    noisy.silk = Some(4);
    let mut unknown = complete(3, 100.0);
    unknown.silk = None;
    let keys = rank(&[&quiet, &noisy, &unknown]);
    assert_eq!((keys[0][14], keys[1][14], keys[2][14]), (0, 4, i64::MAX));
    assert_eq!(order(&[&unknown, &noisy, &quiet]), vec![1, 2, 3]);
    let mut scored = complete(4, 100.0);
    scored.screen.score = 99;
    assert_eq!(rank(&[&scored])[0][15], -99);
    assert_eq!(order(&[&quiet, &scored]), vec![4, 1]);
}

#[test]
fn rank_reads_the_routing_key_through_rank_measures() {
    // Acute corners, right angles and vias are the routing key's, in the same places.
    let mut m = measures(0, 100.0);
    m.acute = Some(3.0);
    m.right_angles = Some(4.0);
    m.vias_off_pour = 5;
    m.copper_quality = Some(77.0);
    let run = judged(
        1,
        plain_screen(),
        passing(),
        routed(m.clone(), open_gates()),
    );
    let key = rank(&[&run])[0];
    assert_eq!(key[8], 3);
    assert_eq!(key[11], 4);
    assert_eq!(key[12], 5);
    assert_eq!(key[13], -77);
    let direct = rank_measures(&[(&m, &open_gates(), true)])[0];
    assert_eq!(key[8..14], direct[3..9]);
    // A measure the audit did not give sorts last.
    let mut unknown = m;
    unknown.acute = None;
    let run = judged(2, plain_screen(), passing(), routed(unknown, open_gates()));
    assert_eq!(rank(&[&run])[0][8], i64::MAX);
}
