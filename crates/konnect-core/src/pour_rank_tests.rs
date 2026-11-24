//! Tests of `pour_rank`: one test and one negative control for each guard in
//! docs/POUR_RANK_DESIGN.md, "Part 2: tests, each guard with a negative control".

use super::*;
use crate::pour_damage::PourDamage;
use crate::route_judge::{rank_measures, Gates, Measures};
use crate::zone_fill::PourMeasures;
use serde_json::json;

pub(crate) fn passing() -> Gates {
    Gates {
        g1: vec![],
        g2: vec![],
        g3: vec![],
        g4: vec![],
    }
}

fn measures() -> Measures {
    crate::route_judge::tests::measures()
}

/// Pour measures with a damage of these three numbers and `starved` starved thermals.
pub(crate) fn pours(
    starved: usize,
    lost_fills: usize,
    plane_loss: f64,
    area_loss: f64,
) -> PourMeasures {
    PourMeasures {
        open_before_stitching: 0,
        open_after_stitching: 0,
        pieces: 1,
        largest_piece_share: Some(1.0),
        unfilled: 0,
        starved_thermals: starved,
        false_connected: None,
        false_open: None,
        damage: Some(PourDamage {
            lost_fills,
            plane_loss,
            area_loss,
            nets: vec![],
            layers: vec![],
        }),
    }
}

pub(crate) fn with_pours(mut m: Measures, pours: PourMeasures) -> Measures {
    m.pours = Some(pours);
    m
}

/// The keys of `runs`, every run in the pool and passing every gate.
fn keys_of(runs: &[Measures]) -> Vec<PourKey> {
    let gates = passing();
    let refs: Vec<(&Measures, &Gates, bool)> = runs.iter().map(|m| (m, &gates, true)).collect();
    pour_keys(&refs)
}

/// The indices of `keys` from the best to the worst, ties to the lower index.
fn order<K: Ord>(keys: &[K]) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..keys.len()).collect();
    indices.sort_by(|a, b| keys[*a].cmp(&keys[*b]).then(a.cmp(b)));
    indices
}

/// Where each of today's thirteen entries sits in the pour key (PK2).
/// Written out here, apart from the module's own `TODAY_AT`, so that the test does not
/// read the constant it checks.
const PLACES: [usize; 13] = [0, 1, 2, 3, 4, 5, 6, 10, 11, 12, 13, 15, 16];

/// Runs that differ in every measure today's key reads, gates included.
fn varied() -> (Vec<Measures>, Vec<Gates>) {
    let mut runs = Vec::new();
    let mut gates = Vec::new();
    for i in 0..5_u32 {
        let f = f64::from(i);
        let mut m = measures();
        m.acute = Some(f);
        m.signal_mm = 100.0 + 7.0 * f;
        m.power_mm = 10.0 + 3.0 * f;
        m.right_angles = Some(2.0 * f);
        m.vias_off_pour = i as usize * 3;
        m.copper_quality = Some(90.0 - f);
        m.route_s = 10.0 + 11.0 * f;
        m.pour_track_mm = 5.0 * f;
        m.violations
            .insert("length_out_of_range".to_string(), i as usize);
        m.lengths = Some(crate::length_probe::LengthMeasures {
            groups: vec![],
            pairs: vec![],
            unmeasured: vec![],
            paths: Default::default(),
            miss_mm: 0.5 * f,
            probe_s: 0.0,
        });
        runs.push(m);
        let mut g = passing();
        if i % 2 == 1 {
            g.g1.push("open".to_string());
        }
        if i == 2 || i == 3 {
            g.g2.push("drc".to_string());
        }
        if i == 4 {
            g.g3.push("width".to_string());
            g.g4.push("length".to_string());
        }
        gates.push(g);
    }
    (runs, gates)
}

// ─── Today's key is today's ──────────────────────────────────────────────────

#[test]
fn the_routing_mode_ranks_by_todays_key_entry_for_entry() {
    let (runs, gates) = varied();
    let refs: Vec<(&Measures, &Gates, bool)> = runs
        .iter()
        .zip(&gates)
        .enumerate()
        .map(|(i, (m, g))| (m, g, i != 1))
        .collect();
    let today: Vec<Vec<i64>> = rank_measures(&refs).iter().map(|k| k.to_vec()).collect();
    assert_eq!(ranking(RankMode::Routing, &refs), today);
    // The runs really differ in every entry somewhere.
    for position in 0..crate::route_judge::KEY_LEN {
        let first = today[0][position];
        assert!(
            today.iter().any(|key| key[position] != first),
            "entry {position} is the same for every run"
        );
    }
    // And the pour mode gives the pour key.
    let poured: Vec<Vec<i64>> = pour_keys(&refs).iter().map(|k| k.to_vec()).collect();
    assert_eq!(ranking(RankMode::Pours, &refs), poured);
    assert!(poured.iter().all(|key| key.len() == POUR_KEY_LEN));
    assert!(today
        .iter()
        .all(|key| key.len() == crate::route_judge::KEY_LEN));
}

#[test]
fn ranking_of_judged_runs_puts_every_run_in_the_pool() {
    let (runs, gates) = varied();
    let judged: Vec<Judged> = runs
        .iter()
        .zip(&gates)
        .map(|(m, g)| Judged {
            measures: m.clone(),
            gates: g.clone(),
            board: std::path::PathBuf::from("/b.kicad_pcb"),
            rounds: vec![],
            tune: None,
        })
        .collect();
    let refs: Vec<&Judged> = judged.iter().collect();
    let pooled: Vec<(&Measures, &Gates, bool)> =
        runs.iter().zip(&gates).map(|(m, g)| (m, g, true)).collect();
    for mode in [RankMode::Routing, RankMode::Pours] {
        assert_eq!(ranking_of(mode, &refs), ranking(mode, &pooled), "{mode:?}");
    }
    let today: Vec<Vec<i64>> = crate::route_judge::rank(&refs)
        .iter()
        .map(|k| k.to_vec())
        .collect();
    assert_eq!(ranking_of(RankMode::Routing, &refs), today);
}

// ─── Today's entries keep their order in the pour key ────────────────────────

#[test]
fn todays_entries_sit_at_their_places_in_the_pour_key() {
    let (runs, gates) = varied();
    let refs: Vec<(&Measures, &Gates, bool)> =
        runs.iter().zip(&gates).map(|(m, g)| (m, g, true)).collect();
    let today = rank_measures(&refs);
    let poured = pour_keys(&refs);
    for (run, (today, poured)) in today.iter().zip(&poured).enumerate() {
        for (entry, &place) in PLACES.iter().enumerate() {
            assert_eq!(
                poured[place], today[entry],
                "run {run}: today's entry {entry} at {place}"
            );
        }
    }
    assert_eq!(TODAY_AT, PLACES);
    // The four new places are the four others, in order.
    let mut new: Vec<usize> = (0..POUR_KEY_LEN)
        .filter(|place| !PLACES.contains(place))
        .collect();
    new.sort_unstable();
    assert_eq!(new, vec![POUR_FAULTS, PLANE_LOSS, POUR_TRACK, AREA_LOSS]);
    assert_eq!(
        (POUR_FAULTS, PLANE_LOSS, POUR_TRACK, AREA_LOSS),
        (7, 8, 9, 14)
    );
}

// ─── No pour record, today's order ───────────────────────────────────────────

#[test]
fn without_a_pour_record_the_two_keys_sort_alike_ties_included() {
    let (mut runs, gates) = varied();
    for m in &mut runs {
        m.pour_track_mm = 0.0;
        assert!(m.pours.is_none() && m.pours_error.is_none());
    }
    // Two exact copies: a tie both keys must keep.
    runs.push(runs[0].clone());
    let mut gates = gates;
    gates.push(gates[0].clone());
    let refs: Vec<(&Measures, &Gates, bool)> =
        runs.iter().zip(&gates).map(|(m, g)| (m, g, true)).collect();
    let today = rank_measures(&refs);
    let poured = pour_keys(&refs);
    for key in &poured {
        for place in [POUR_FAULTS, PLANE_LOSS, AREA_LOSS, POUR_TRACK] {
            assert_eq!(key[place], 0, "{key:?}");
        }
    }
    assert_eq!(order(&today), order(&poured));
    // The tie is a tie under both keys.
    let last = runs.len() - 1;
    assert_eq!(today[0], today[last]);
    assert_eq!(poured[0], poured[last]);
}

// ─── An unmeasured run does not outrank a measured one ───────────────────────

#[test]
fn a_run_whose_pours_were_not_measured_sorts_after_one_whose_were() {
    let measured = with_pours(measures(), pours(3, 1, 0.5, 0.5));
    let mut failed = measures();
    failed.pours_error = Some("the fill could not be read".to_string());
    let no_damage = {
        let mut p = pours(0, 0, 0.0, 0.0);
        p.damage = None;
        with_pours(measures(), p)
    };
    // An error beside measures is unmeasured too.
    let mut both = with_pours(measures(), pours(0, 0, 0.0, 0.0));
    both.pours_error = Some("half read".to_string());
    for unmeasured in [failed, no_damage, both] {
        let keys = keys_of(&[unmeasured.clone(), measured.clone()]);
        for place in [POUR_FAULTS, PLANE_LOSS, AREA_LOSS] {
            assert_eq!(keys[0][place], i64::MAX, "{unmeasured:?}");
        }
        assert_eq!(order(&keys), vec![1, 0], "{unmeasured:?}");
    }
}

// ─── Faults, then the plane, then the lengths ────────────────────────────────

#[test]
fn faults_come_before_the_plane_and_the_plane_before_the_lengths() {
    let base = with_pours(measures(), pours(0, 0, 0.0, 0.0));
    // One entry each.
    let one_fault = with_pours(measures(), pours(1, 0, 0.0, 0.0));
    let one_lost = with_pours(measures(), pours(0, 1, 0.0, 0.0));
    let plane = with_pours(measures(), pours(0, 0, 0.05, 0.0));
    let area = with_pours(measures(), pours(0, 0, 0.0, 0.01));
    for worse in [&one_fault, &one_lost, &plane, &area] {
        let keys = keys_of(&[worse.clone(), base.clone()]);
        assert_eq!(order(&keys), vec![1, 0], "{worse:?}");
    }
    let keys = keys_of(&[one_fault.clone(), one_lost.clone()]);
    assert_eq!(keys[0][POUR_FAULTS], 1);
    assert_eq!(keys[1][POUR_FAULTS], 1);

    // A fault outranks a lower plane loss, the plane loss outranks two signal steps.
    let faulted_whole = with_pours(measures(), pours(1, 0, 0.0, 0.0));
    let cut_clean = with_pours(measures(), pours(0, 0, 0.5, 0.0));
    assert_eq!(order(&keys_of(&[faulted_whole, cut_clean])), vec![1, 0]);

    // One starved thermal more against two length steps fewer: the starved run loses.
    let mut long = with_pours(measures(), pours(0, 0, 0.0, 0.0));
    long.signal_mm = 104.0; // 104 / 100 / 0.02 = 52, two steps above 50
    let mut short = with_pours(measures(), pours(1, 0, 0.0, 0.0));
    short.signal_mm = 100.0;
    let keys = keys_of(&[short.clone(), long.clone()]);
    assert_eq!(keys[1][10] - keys[0][10], 2, "{keys:?}");
    assert_eq!(order(&keys), vec![1, 0]);
    // The same with the plane: a whole plane over two steps shorter.
    let mut cut_short = with_pours(measures(), pours(0, 0, 0.05, 0.0));
    cut_short.signal_mm = 100.0;
    assert_eq!(order(&keys_of(&[cut_short, long.clone()])), vec![1, 0]);

    // After the vias: the area loss breaks ties, never a via.
    let mut fewer_vias_more_area = with_pours(measures(), pours(0, 0, 0.0, 0.5));
    fewer_vias_more_area.vias_off_pour = 0;
    let mut more_vias = with_pours(measures(), pours(0, 0, 0.0, 0.0));
    more_vias.vias_off_pour = 1;
    assert_eq!(
        order(&keys_of(&[more_vias, fewer_vias_more_area])),
        vec![1, 0]
    );
    // PK19: the pour track comes before the area loss now.
    let mut tracked = with_pours(measures(), pours(0, 0, 0.0, 0.0));
    tracked.pour_track_mm = 50.0;
    let lossy = with_pours(measures(), pours(0, 0, 0.0, 0.01));
    assert_eq!(order(&keys_of(&[lossy, tracked])), vec![0, 1]);
}

// ─── PK19: the ground track before the lengths ───────────────────────────────

/// Two runs that differ only in their signal and pour-track lengths (the same faults and
/// plane step): A has the shorter signal (50 steps against 51), B one 5 % step less pour
/// track (0 against 1). Control: the track left at entry 14, A first.
#[test]
fn the_track_ranks_before_the_lengths() {
    let mut a = with_pours(measures(), pours(0, 0, 0.0, 0.0));
    a.signal_mm = 100.0;
    a.pour_track_mm = 5.0; // 5 / 100 / 0.05 = 1
    let mut b = with_pours(measures(), pours(0, 0, 0.0, 0.0));
    b.signal_mm = 102.0; // 102 / 100 / 0.02 = 51
    b.pour_track_mm = 0.0;
    let keys = keys_of(&[a.clone(), b.clone()]);
    assert_eq!(
        (keys[0][POUR_TRACK], keys[1][POUR_TRACK]),
        (1, 0),
        "{keys:?}"
    );
    assert_eq!((keys[0][10], keys[1][10]), (50, 51), "{keys:?}");
    assert_eq!(order(&keys), vec![1, 0], "pours: B first");
    assert_eq!(order(&today_of(&[a, b])), vec![0, 1], "routing: A first");
}

/// The track still ranks after the plane step: B has less track (0 against 1 step) but a
/// plane step one higher. Control: the track before the plane step, B first.
#[test]
fn the_track_still_ranks_after_the_plane_step() {
    let mut a = with_pours(measures(), pours(0, 0, 0.0, 0.0));
    a.pour_track_mm = 5.0;
    let b = with_pours(measures(), pours(0, 0, 0.025, 0.0)); // one 2.5 % step (PK20)
    let keys = keys_of(&[a, b]);
    assert_eq!(
        (keys[0][PLANE_LOSS], keys[1][PLANE_LOSS]),
        (0, 1),
        "{keys:?}"
    );
    assert_eq!(
        (keys[0][POUR_TRACK], keys[1][POUR_TRACK]),
        (1, 0),
        "{keys:?}"
    );
    assert_eq!(order(&keys), vec![0, 1]);
}

/// Today's key of `runs`, every run in the pool and passing every gate.
fn today_of(runs: &[Measures]) -> Vec<Key> {
    let gates = passing();
    let refs: Vec<(&Measures, &Gates, bool)> = runs.iter().map(|m| (m, &gates, true)).collect();
    rank_measures(&refs)
}

// ─── The steps round as the lengths do ───────────────────────────────────────

#[test]
fn the_two_steps_round_half_away_from_zero() {
    assert_eq!(plane_step(0.0), 0);
    assert_eq!(plane_step(0.012), 0);
    assert_eq!(plane_step(0.0125), 1);
    assert_eq!(plane_step(0.037), 1);
    assert_eq!(plane_step(0.038), 2);
    assert_eq!(plane_step(1.0), 40);
    assert_eq!(area_step(0.004), 0);
    assert_eq!(area_step(0.005), 1);
    assert_eq!(area_step(1.0), 100);
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.01] {
        assert_eq!(plane_step(bad), i64::MAX, "{bad}");
        assert_eq!(area_step(bad), i64::MAX, "{bad}");
    }
    // Minus zero is zero.
    assert_eq!(plane_step(-0.0), 0);
    assert_eq!(area_step(-0.0), 0);
    // The key reads the steps.
    let keys = keys_of(&[with_pours(measures(), pours(0, 0, 0.076, 0.005))]);
    assert_eq!((keys[0][PLANE_LOSS], keys[0][AREA_LOSS]), (3, 1));
}

/// PK20: the plane-loss step is 2.5 % of the planes' area, half away from zero: a 3.5 %
/// loss and a 2.3 % loss are both one step (1.4 and 0.92). Control: the old 5 % step gives
/// 1 and 0.
#[test]
fn the_plane_step_is_two_and_a_half_percent() {
    assert_eq!(PLANE_STEP, 0.025);
    assert_eq!(plane_step(0.035), 1);
    assert_eq!(plane_step(0.023), 1);
    let keys = keys_of(&[
        with_pours(measures(), pours(0, 0, 0.035, 0.0)),
        with_pours(measures(), pours(0, 0, 0.023, 0.0)),
    ]);
    assert_eq!((keys[0][PLANE_LOSS], keys[1][PLANE_LOSS]), (1, 1));
}

// ─── The pour track is in signal units ───────────────────────────────────────

/// PK19: the step is 5 % of the shortest signal, half away from zero. Control: the 2 %
/// `LENGTH_STEP` kept, 4, 4 and 1.
#[test]
fn the_pour_track_is_stepped_in_five_percent_of_the_shortest_signal() {
    assert_eq!(POUR_TRACK_STEP, 0.05);
    assert_eq!(pour_track_step(70.0, Some(1000.0)), 1); // 7 %
    assert_eq!(pour_track_step(80.0, Some(1000.0)), 2); // 8 %
    assert_eq!(pour_track_step(24.0, Some(1000.0)), 0); // 2.4 %
    assert_eq!(pour_track_step(25.0, Some(1000.0)), 1); // 2.5 %: half away from zero
    assert_eq!(pour_track_step(400.0, Some(1000.0)), 8);
    assert_eq!(pour_track_step(0.0, Some(1000.0)), 0);
    assert_eq!(pour_track_step(400.0, None), 0);
    // A length that is not above 0 is no track: 0, never a negative step.
    assert_eq!(pour_track_step(-100.0, Some(1000.0)), 0);

    // In the key, against the pool's shortest signal (1000 mm), not the pour track.
    let mut a = measures();
    a.signal_mm = 1000.0;
    a.pour_track_mm = 24.0;
    let mut b = measures();
    b.signal_mm = 1200.0;
    b.pour_track_mm = 400.0;
    let mut c = measures();
    c.signal_mm = 1100.0;
    c.pour_track_mm = 70.0;
    let keys = keys_of(&[a, b, c]);
    assert_eq!(
        keys.iter().map(|k| k[POUR_TRACK]).collect::<Vec<_>>(),
        vec![0, 8, 1]
    );

    // No signal track in the pool: 0 for all.
    let mut runs = Vec::new();
    for mm in [9.0, 400.0, 1000.0] {
        let mut m = measures();
        m.signal_mm = 0.0;
        m.pour_track_mm = mm;
        runs.push(m);
    }
    assert!(keys_of(&runs).iter().all(|k| k[POUR_TRACK] == 0));
}

// ─── The pool is today's ─────────────────────────────────────────────────────

#[test]
fn a_run_outside_the_pool_or_failing_g1_does_not_set_the_unit() {
    let mut pooled = measures();
    pooled.signal_mm = 1000.0;
    pooled.pour_track_mm = 400.0;
    // Far shorter signals, but outside the pool, or incomplete.
    let mut outside = measures();
    outside.signal_mm = 10.0;
    let mut incomplete = measures();
    incomplete.signal_mm = 20.0;
    let pass = passing();
    let mut open = passing();
    open.g1.push("1 open".to_string());
    let runs = [
        (&pooled, &pass, true),
        (&outside, &pass, false),
        (&incomplete, &open, true),
    ];
    let keys = pour_keys(&runs);
    // Unit 1000 mm: 400 mm is 8 steps of 5 % (unit 10 mm would be 800, unit 20 mm 400).
    assert_eq!(keys[0][POUR_TRACK], 8);
    // The signal step of today's key uses the same reference.
    assert_eq!(keys[0][10], rank_measures(&runs)[0][7]);
    assert_eq!(keys[0][10], 50);
}

// ─── The setting ─────────────────────────────────────────────────────────────

#[test]
fn no_setting_ranks_by_todays_key_and_pours_names_the_pour_key() {
    assert_eq!(
        resolve_rank_mode(None),
        Ok((RankMode::Routing, RankSource::Default))
    );
    assert_eq!(
        resolve_rank_mode(Some(&Value::Null)),
        Ok((RankMode::Routing, RankSource::Default))
    );
    assert_eq!(
        resolve_rank_mode(Some(&json!("pours"))),
        Ok((RankMode::Pours, RankSource::UserConfig))
    );
    assert_eq!(
        resolve_rank_mode(Some(&json!("routing"))),
        Ok((RankMode::Routing, RankSource::UserConfig))
    );
}

#[test]
fn a_value_that_names_no_mode_is_refused_with_both_names() {
    for value in [
        json!("pour"),
        json!(3),
        json!(""),
        json!("Pours"),
        json!(["pours"]),
    ] {
        let error = resolve_rank_mode(Some(&value)).expect_err(&value.to_string());
        for word in ["routing.rank_key", "routing", "pours"] {
            assert!(error.contains(word), "{value}: {error}");
        }
        let shown = value
            .as_str()
            .map_or(value.to_string(), |text| format!("{text:?}"));
        assert!(error.contains(&shown), "{value}: {error}");
    }
}

#[test]
fn the_modes_have_their_names() {
    for mode in [RankMode::Routing, RankMode::Pours] {
        assert_eq!(RankMode::parse(mode.name()), Some(mode));
        assert_ne!(mode.other(), mode);
        assert_eq!(mode.other().other(), mode);
        assert_eq!(json!(mode), json!(mode.name()));
    }
    assert_eq!(RankMode::Routing.name(), "routing");
    assert_eq!(RankMode::Pours.name(), "pours");
    assert_eq!(RankMode::parse(" pours"), None);
    assert_eq!(json!(RankSource::UserConfig), json!("user_config"));
    assert_eq!(json!(RankSource::Default), json!("default"));
    assert_eq!(json!(RankSource::Fixed), json!("fixed"));
}

// ─── entries_above ───────────────────────────────────────────────────────────

#[test]
fn entries_above_names_every_higher_entry_but_the_seconds() {
    assert_eq!(
        entries_above(&[0, 0, 0, 0], &[0, 0, 0, 0]),
        Vec::<usize>::new()
    );
    assert_eq!(entries_above(&[0, 5, 1, 0], &[1, 4, 2, 99]), vec![0, 2]);
    assert_eq!(entries_above(&[], &[]), Vec::<usize>::new());
    // A pour key: the signal step is 10 (PK19).
    let now = keys_of(&[measures()])[0];
    let mut longer = measures();
    longer.signal_mm = 110.0;
    let keys = keys_of(&[measures(), longer]);
    assert_eq!(keys[0], now);
    assert_eq!(entries_above(&keys[0], &keys[1]), vec![10]);
}

#[test]
#[should_panic(expected = "two keys of one ranking have one length")]
fn two_keys_of_two_lengths_are_a_bug() {
    let _ = entries_above(&[0; crate::route_judge::KEY_LEN], &[0; POUR_KEY_LEN]);
}

// ─── The report names the other key's choice ─────────────────────────────────

fn key(first: i64) -> Key {
    let mut key = [0; crate::route_judge::KEY_LEN];
    key[0] = first;
    key
}

fn pour_key(first: i64) -> PourKey {
    let mut key = [0; POUR_KEY_LEN];
    key[0] = first;
    key
}

#[test]
fn the_report_names_the_least_of_the_other_key() {
    // Run 0 not judged; runs 1 to 3 judged (run 3 released by the search, still keyed).
    let keys = [None, Some(key(1)), Some(key(0)), Some(key(2))];
    let pour_keys = [
        None,
        Some(pour_key(2)),
        Some(pour_key(1)),
        Some(pour_key(0)),
    ];
    let report = rank_report(
        RankMode::Routing,
        RankSource::Default,
        &keys,
        &pour_keys,
        Some(2),
    );
    assert_eq!(
        report,
        json!({
            "ranked_by": "routing",
            "source": "default",
            "other_key": "pours",
            "other_winner": 3,
            "same_winner": false,
        })
    );
    let report = rank_report(
        RankMode::Pours,
        RankSource::UserConfig,
        &keys,
        &pour_keys,
        Some(3),
    );
    assert_eq!(report["ranked_by"], "pours");
    assert_eq!(report["source"], "user_config");
    assert_eq!(report["other_key"], "routing");
    assert_eq!(report["other_winner"], 2);
    assert_eq!(report["same_winner"], false);
    // Same winner.
    let report = rank_report(
        RankMode::Pours,
        RankSource::Fixed,
        &keys,
        &pour_keys,
        Some(2),
    );
    assert_eq!(report["same_winner"], true);
    assert_eq!(report["source"], "fixed");
    // A tie goes to the lower index.
    let tied = [Some(key(0)), Some(key(0))];
    let report = rank_report(
        RankMode::Pours,
        RankSource::Default,
        &tied,
        &[Some(pour_key(0)), Some(pour_key(0))],
        Some(0),
    );
    assert_eq!(report["other_winner"], 0);
    assert_eq!(report["same_winner"], true);
    // No judged run: null, and not the same winner.
    let report = rank_report(
        RankMode::Routing,
        RankSource::Default,
        &[None],
        &[None],
        None,
    );
    assert_eq!(report["other_winner"], Value::Null);
    assert_eq!(report["same_winner"], false);
    let report = rank_report(RankMode::Routing, RankSource::Default, &[], &[], None);
    assert_eq!(report["other_winner"], Value::Null);
}

// ─── Each site follows the mode: the shared pool ─────────────────────────────

/// Measures on which the two keys disagree: `signal_mm` decides today's key (its signal
/// step, entry 7), `plane_loss` decides the pour key first (its plane step, entry 8,
/// before the signal step at 9). Every gate passes; nothing else differs.
pub(crate) fn split(signal_mm: f64, plane_loss: f64) -> Measures {
    let mut m = with_pours(measures(), pours(0, 0, plane_loss, 0.0));
    m.signal_mm = signal_mm;
    m
}

/// A judged run of [`split`] measures.
pub(crate) fn split_judged(signal_mm: f64, plane_loss: f64) -> Judged {
    Judged {
        measures: split(signal_mm, plane_loss),
        gates: passing(),
        board: std::path::PathBuf::from("/nowhere/judged.kicad_pcb"),
        rounds: vec![],
        tune: None,
    }
}

/// The two judged runs every site test uses: `short` wins today's key, `whole` the pour
/// key.
pub(crate) fn short_and_whole() -> (Judged, Judged) {
    (split_judged(100.0, 0.5), split_judged(110.0, 0.0))
}

#[test]
fn the_shared_pool_splits_the_two_keys() {
    let (short, whole) = short_and_whole();
    let today = crate::route_judge::rank(&[&short, &whole]);
    assert!(today[0] < today[1], "{today:?}");
    let poured = ranking_of(RankMode::Pours, &[&short, &whole]);
    assert!(poured[1] < poured[0], "{poured:?}");
}

#[test]
fn a_runs_tuned_choice_follows_the_mode() {
    use crate::route_judge::tuned_choice;
    let (short, whole) = short_and_whole();
    // The untuned board first, then the candidates.
    assert_eq!(tuned_choice(RankMode::Routing, &[&short, &whole]), 0);
    assert_eq!(tuned_choice(RankMode::Pours, &[&short, &whole]), 1);
    assert_eq!(tuned_choice(RankMode::Routing, &[&whole, &short]), 1);
    assert_eq!(tuned_choice(RankMode::Pours, &[&whole, &short]), 0);
    // An exact tie keeps the untuned board under both modes, then the first candidate.
    for mode in [RankMode::Routing, RankMode::Pours] {
        assert_eq!(tuned_choice(mode, &[&whole, &whole.clone()]), 0, "{mode:?}");
        assert_eq!(
            tuned_choice(mode, &[&short, &whole, &whole.clone()]),
            if mode == RankMode::Pours { 1 } else { 0 },
            "{mode:?}"
        );
    }
    assert_eq!(tuned_choice(RankMode::Pours, &[]), 0);
}
