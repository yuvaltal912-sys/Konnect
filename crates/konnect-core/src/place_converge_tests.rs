//! Tests of `place_converge`, the convergent placement search (family C): the cost model
//! (stage 2). The stages that fill the search's bodies add their tests here.

use super::*;
use crate::place_generators::tests::{
    converge_params, demo_model, demo_parts, demo_text, model_of,
};
use crate::place_generators::{board_model, placements_from, unit};
use crate::place_held::tests::{board, evaluate_text, pad, parse, Fp, Shape};
use crate::place_held::{decoupling_pairs, groups, held_set, placements_of, DecouplingPair};
use crate::place_held::{HoldRules, Intent};
use crate::place_judge::{pair_violations, screen};
use crate::place_schematic::part_distances;
use crate::tools::pcb_components::placements_applied_to;

const OUTLINE: Shape = Shape::Rect(0.0, 0.0, 60.0, 40.0);

/// The placement tests' stock rectangle and the rounded board with a cut-out.
const RECTANGLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../konnect-sexp/tests/fixtures/placement/placement_fixture.kicad_pcb"
);
const ROUNDED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../konnect-sexp/tests/fixtures/board_geometry/cases.kicad_pcb"
);

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// The model of `text` with the held set of `rules`, the intent from the board's own
/// score, as the search builds it.
fn model_with(text: &str, rules: &HoldRules) -> BoardModel {
    let set = held_set(text, &BTreeSet::new(), rules).expect("the held set");
    let held: Vec<&str> = set.held.keys().map(String::as_str).collect();
    let evaluated = evaluate_text(text, &held);
    let pairs = decoupling_pairs(&evaluated).unwrap();
    let found = groups(&parse(text), &set, &[]);
    board_model(text, set, Intent::new(found, pairs)).expect("the model")
}

fn cost_of<'a>(model: &'a BoardModel, hoods: Option<&Neighbourhoods>) -> CostModel<'a> {
    CostModel::build(model, hoods, &converge_params()).expect("the cost model")
}

fn index_of(model: &BoardModel, reference: &str) -> usize {
    model
        .references
        .iter()
        .position(|r| r == reference)
        .unwrap_or_else(|| panic!("{reference} is on the board"))
}

/// The state of the board as read.
fn as_read(cost: &CostModel<'_>) -> State {
    cost.state(cost.read_centres.clone(), vec![0; cost.read_centres.len()])
}

/// The board of `state`, written as a candidate is written.
fn written(model: &BoardModel, state: &State) -> String {
    let turns: Vec<i32> = state.turns.iter().map(|&q| i32::from(q)).collect();
    placements_applied_to(&model.text, &placements_from(model, &state.centres, &turns))
        .expect("the board is written")
}

/// The state rebuilt from the anchors and rotations of the board `text` (a board the
/// model's text was written into).
fn rebuilt(cost: &CostModel<'_>, text: &str) -> State {
    let placements = placements_of(&parse(text), &cost.model.held.movable).unwrap();
    let (centres, turns) = centres_of(cost, &placements).unwrap();
    cost.state(centres, turns)
}

/// `evaluate`'s result for `text`, the held parts as edge mounts, as the screen runs it.
fn evaluated(model: &BoardModel, text: &str) -> serde_json::Value {
    let held: Vec<&str> = model.held.held.keys().map(String::as_str).collect();
    evaluate_text(text, &held)
}

/// `Terms::airwire_mm` and `crossings` of `state` against `routing_metrics` of `text`.
fn assert_the_scores_own(cost: &CostModel<'_>, state: &State, text: &str, what: &str) {
    let scored = evaluated(cost.model, text);
    let metrics = &scored["routing_metrics"];
    let length = metrics["wirelength_mm"]
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: the score has a length"));
    let crossings = metrics["proper_crossings"]
        .as_u64()
        .unwrap_or_else(|| panic!("{what}: the score has crossings")) as usize;
    let terms = cost.terms(state);
    assert!(
        (terms.airwire_mm - length).abs() <= 1e-6,
        "{what}: the model's airwire {} mm, the score's {length} mm",
        terms.airwire_mm
    );
    assert_eq!(terms.crossings, crossings, "{what}: crossings");
}

/// Every cache of `state` equal to `fresh` (the same placement computed from nothing):
/// the placement, the trees, the counts and every distance exactly, the running cost to
/// 1e-6.
fn assert_same_caches(state: &State, fresh: &State, what: &str) {
    assert_eq!(state.centres, fresh.centres, "{what}: centres");
    assert_eq!(state.turns, fresh.turns, "{what}: turns");
    assert_eq!(state.sizes, fresh.sizes, "{what}: sizes");
    assert_eq!(state.net_mm, fresh.net_mm, "{what}: net lengths");
    assert_eq!(state.net_edges, fresh.net_edges, "{what}: net edges");
    assert_eq!(state.net_box, fresh.net_box, "{what}: net boxes");
    assert_eq!(state.crossings, fresh.crossings, "{what}: crossings");
    assert_eq!(state.pair_mm, fresh.pair_mm, "{what}: pairs");
    assert_eq!(state.connector_mm, fresh.connector_mm, "{what}: connectors");
    assert_eq!(state.neighbour_mm, fresh.neighbour_mm, "{what}: neighbours");
    assert_eq!(
        state.label_conflicts, fresh.label_conflicts,
        "{what}: label conflicts"
    );
    assert!(
        (state.cost - fresh.cost).abs() <= 1e-6,
        "{what}: cost {} against {}",
        state.cost,
        fresh.cost
    );
}

/// A shift of up to `radius` mm on each axis, on the 0.25 mm grid.
fn shift(rng: &mut SplitMix64, radius: f64) -> f64 {
    ((2.0 * unit(rng) - 1.0) * radius / 0.25).round() * 0.25
}

/// A random move of the movable parts: a shift (0), a turn (1), a swap (2), a move of
/// three or four parts at once (3), a part back where and as it was read (4).
fn random_move(cost: &CostModel<'_>, state: &State, rng: &mut SplitMix64, kind: u64) -> Move {
    let movable = &cost.movable;
    let pick = |rng: &mut SplitMix64| movable[rng.below(movable.len() as u64) as usize];
    let p = pick(rng);
    let (at, turns) = (state.centres[p], state.turns[p]);
    let parts = match kind {
        0 => vec![(p, (at.0 + shift(rng, 4.0), at.1 + shift(rng, 4.0)), turns)],
        1 => vec![(p, at, (turns + 1 + rng.below(3) as u8) % 4)],
        2 => {
            let o = pick(rng);
            vec![(p, state.centres[o], turns), (o, at, state.turns[o])]
        }
        3 => {
            let mut parts = vec![(p, (at.0 + shift(rng, 2.0), at.1 + shift(rng, 2.0)), turns)];
            let wanted = 3 + rng.below(2) as usize;
            for _ in 0..50 {
                if parts.len() == wanted.min(movable.len()) {
                    break;
                }
                let o = pick(rng);
                if parts.iter().any(|&(q, ..)| q == o) {
                    continue;
                }
                let at_o = state.centres[o];
                parts.push((
                    o,
                    (at_o.0 + shift(rng, 2.0), at_o.1 + shift(rng, 2.0)),
                    state.turns[o],
                ));
            }
            parts
        }
        _ => vec![(p, cost.read_centres[p], 0)],
    };
    Move { parts }
}

/// `count` random legal moves applied to `state`, kinds drawn from `kinds`; the moves
/// made, by kind.
fn random_walk(
    cost: &CostModel<'_>,
    state: &mut State,
    rng: &mut SplitMix64,
    count: usize,
    kinds: &[u64],
    mut after: impl FnMut(&State, &Move, &Delta),
) -> Vec<usize> {
    let mut made = vec![0; 5];
    let mut tries = 0;
    while made.iter().sum::<usize>() < count {
        tries += 1;
        assert!(tries < 200 * count, "too few legal moves: {made:?}");
        let kind = kinds[rng.below(kinds.len() as u64) as usize];
        let mv = random_move(cost, state, rng, kind);
        if !cost.legal(state, &mv) {
            continue;
        }
        let delta = cost.delta(state, &mv);
        let copy = delta.clone();
        cost.apply(state, &mv, delta);
        after(state, &mv, &copy);
        made[kind as usize] += 1;
    }
    made
}

/// Neighbourhoods named by hand, as `place_schematic::neighbourhoods` gives them.
fn hoods_of(lists: &[(&str, &[&str])]) -> Neighbourhoods {
    Neighbourhoods {
        neighbours: lists
            .iter()
            .map(|(part, near)| {
                (
                    part.to_string(),
                    near.iter().map(|n| n.to_string()).collect(),
                )
            })
            .collect(),
        measured: lists.len(),
        unmatched: 0,
        alone: 0,
    }
}

/// The demo with decoupling pairs named by hand: C1 to C4 on U1's pad 1, C5 to C8 on
/// its pad 2, each within 5 mm (the demo's own score pairs none of them).
fn demo_with_pairs() -> BoardModel {
    let text = demo_text();
    let set = held_set(&text, &BTreeSet::new(), &HoldRules::default()).unwrap();
    let pairs = (1..=8)
        .map(|k| DecouplingPair {
            cap: format!("C{k}"),
            cap_pad: "1".to_string(),
            ic: "U1".to_string(),
            ic_pad: if k <= 4 { "1" } else { "2" }.to_string(),
            net: "VCC".to_string(),
            limit_mm: 5.0,
            basis: "test".to_string(),
        })
        .collect();
    board_model(&text, set, Intent::new(Vec::new(), pairs)).unwrap()
}

fn demo_hoods() -> Neighbourhoods {
    hoods_of(&[
        ("C1", &["U1", "C2"]),
        ("C5", &["R4"]),
        ("R1", &["R2", "R3", "C1"]),
        ("U1", &["C1"]),
        ("R6", &["J1"]),
    ])
}

/// A board on which the tree's tie rule decides a crossing: net N's site D is as far from
/// B as from C, the score's tree joins D to B, and net M crosses the edge C-D only.
fn ties_text() -> String {
    let one = |reference: &str, x: f64, y: f64, net: &str| {
        Fp::new(reference, x, y)
            .court(0.2, 0.2)
            .pad(pad("1", 0.0, 0.0, Some(net)))
    };
    board(
        OUTLINE,
        &[
            one("A1", 18.0, 10.0, "N"),
            one("B1", 19.0, 20.0, "N"),
            one("C1", 20.0, 19.0, "N"),
            one("D1", 23.0, 23.0, "N"),
            one("M1", 20.2, 20.1, "M"),
            one("M2", 21.0, 19.5, "M"),
        ],
    )
}

/// A board whose R1 has its courtyard off its anchor by 0.4 µm: its written anchor is
/// rounded to 1 µm, so its pads are only where the written board has them when the model
/// rounds the anchor as `placements_from` does.
fn offset_text() -> String {
    board(
        OUTLINE,
        &[
            Fp::new("R1", 20.0, 20.0)
                .court_rect(-1.0004, -0.5, 0.9996, 0.5)
                .two_pad("A", "B"),
            Fp::new("R2", 30.0, 20.0).two_pad("A", "B"),
            Fp::new("R3", 25.0, 30.0).two_pad("B", "C"),
            Fp::new("R4", 35.0, 30.0).two_pad("C", "A"),
        ],
    )
}

// ─── The parameters ──────────────────────────────────────────────────────────

#[test]
fn the_measured_parameters_are_the_experiments() {
    // Design C2, C5, C6, written out: what stage 1's tests wrote by hand.
    assert_eq!(
        ConvergeParams::measured(0),
        ConvergeParams {
            crossing_mm: 4.0,
            fanout_limit: 8,
            rail_weight: 0.3,
            requirement_mm: 1000.0,
            requirement_slope: 50.0,
            neighbour_weight: 0.1,
            label_mm: 0.2,
            turn_parts: true,
            level: 0,
            stage_per_part: 200,
            cooling: 0.95,
            radius_exponent: 0.594,
            quiet_gain: 0.001,
            quiet_stages: 5,
            uphill_floor: 0.01,
            quiet_sweeps: 3,
            max_stages: 2000,
            max_sweeps: 200,
        }
    );
    assert_eq!(ConvergeParams::measured(3).level, 3);
    assert_eq!(
        OuterRule::measured(),
        OuterRule {
            gain: 0.01,
            window: 3,
            max_starts: 64,
            max_level: 5,
        }
    );
}

#[test]
fn bad_parameters_are_refused() {
    assert_eq!(ConvergeParams::measured(0).validate(), Ok(()));
    assert_eq!(ConvergeParams::measured(16).validate(), Ok(()));
    type Edit = fn(&mut ConvergeParams);
    let cases: Vec<(&str, Edit)> = vec![
        ("a negative crossing weight", |p| p.crossing_mm = -1.0),
        ("a negative rail weight", |p| p.rail_weight = -0.1),
        ("a negative requirement", |p| p.requirement_mm = -1.0),
        ("a negative slope", |p| p.requirement_slope = -1.0),
        ("a negative neighbour weight", |p| p.neighbour_weight = -0.1),
        ("a negative label weight", |p| p.label_mm = -0.2),
        ("a negative uphill floor", |p| p.uphill_floor = -0.01),
        ("cooling 1", |p| p.cooling = 1.0),
        ("cooling 0", |p| p.cooling = 0.0),
        ("a quiet gain of 1", |p| p.quiet_gain = 1.0),
        ("a quiet gain of 0", |p| p.quiet_gain = 0.0),
        ("a radius exponent of 0", |p| p.radius_exponent = 0.0),
        ("a crossing weight that is not finite", |p| {
            p.crossing_mm = f64::INFINITY
        }),
        ("a label weight that is not a number", |p| {
            p.label_mm = f64::NAN
        }),
        ("a cooling that is not a number", |p| p.cooling = f64::NAN),
        ("a radius exponent that is not finite", |p| {
            p.radius_exponent = f64::INFINITY
        }),
        ("level 17", |p| p.level = 17),
        ("a fanout of 0", |p| p.fanout_limit = 0),
        ("0 proposals per part", |p| p.stage_per_part = 0),
        ("0 quiet stages", |p| p.quiet_stages = 0),
        ("0 quiet sweeps", |p| p.quiet_sweeps = 0),
        ("0 stages", |p| p.max_stages = 0),
        ("0 sweeps", |p| p.max_sweeps = 0),
    ];
    let model = demo_model();
    for (what, edit) in cases {
        let mut params = ConvergeParams::measured(0);
        edit(&mut params);
        assert!(
            matches!(params.validate(), Err(GenerateFailure::Invalid(_))),
            "{what} is refused"
        );
        assert!(
            matches!(
                CostModel::build(&model, None, &params),
                Err(GenerateFailure::Invalid(_))
            ),
            "{what}: the model is refused"
        );
    }
}

#[test]
fn a_level_scales_the_cooling_the_patience_and_the_quiet_gain() {
    let base = ConvergeParams::measured(0);
    let fall = base.quiet_stages as f64 * base.cooling.ln();
    let patience = base.quiet_gain * base.quiet_stages as f64;
    for level in 0..=3u32 {
        let params = ConvergeParams::measured(level);
        let times = 1usize << level;
        // The same fall of the temperature over the quiet stages at every level.
        let quiet_fall = params.quiet_stages_at_level() as f64 * params.cooling_at_level().ln();
        assert!(
            (quiet_fall - fall).abs() < 1e-12,
            "level {level}: {quiet_fall} against {fall}"
        );
        assert!((params.cooling_at_level().powi(times as i32) - base.cooling).abs() < 1e-12);
        // The same gain over them.
        let gained = params.quiet_gain_at_level() * params.quiet_stages_at_level() as f64;
        assert!((gained - patience).abs() < 1e-15, "level {level}: {gained}");
        assert_eq!(params.quiet_stages_at_level(), base.quiet_stages * times);
        assert_eq!(params.max_stages_at_level(), base.max_stages * times);
        assert!((params.quiet_gain_at_level() * times as f64 - base.quiet_gain).abs() < 1e-15);
    }
}

#[test]
fn a_cost_pays_when_it_is_lower_by_the_gain_and_any_finite_cost_pays_infinity() {
    let rule = OuterRule::measured();
    assert!(rule.pays(100.0, 98.9));
    assert!(!rule.pays(100.0, 99.0));
    assert!(!rule.pays(100.0, 99.5));
    assert!(!rule.pays(100.0, 101.0));
    assert!(rule.pays(f64::INFINITY, 1e300));
    assert!(!rule.pays(f64::INFINITY, f64::INFINITY));
    assert!(!rule.pays(100.0, f64::NAN));
}

#[test]
fn a_run_converged_when_both_stops_are_a_rules() {
    let terms = Terms {
        cost: 0.0,
        airwire_mm: 0.0,
        rail_airwire_mm: 0.0,
        crossings: 0,
        pair_violations: 0,
        pair_mm: 0.0,
        connector_violations: 0,
        neighbour_mm: None,
        label_conflicts: 0,
        soft: 0.0,
    };
    let trace = |anneal_stop, descent_stop| Trace {
        level: 0,
        requirement_mm: 1000.0,
        hoods_sha256: None,
        start: terms,
        end: terms,
        repairs_tried: 0,
        repairs_made: 0,
        first_temperature: 1.0,
        stages: 1,
        anneal_stop,
        sweeps: 1,
        descent_stop,
        evaluations: 1,
        best_by_step: Vec::new(),
    };
    assert!(trace(AnnealStop::Quiet, DescentStop::FixedPoint).converged());
    assert!(trace(AnnealStop::Quiet, DescentStop::Quiet).converged());
    assert!(!trace(AnnealStop::Guard, DescentStop::FixedPoint).converged());
    assert!(!trace(AnnealStop::Quiet, DescentStop::Guard).converged());
}

// ─── The score's own numbers ─────────────────────────────────────────────────

#[test]
fn the_models_airwire_and_crossings_are_the_scores_own() {
    let boards = [
        ("rectangle", std::fs::read_to_string(RECTANGLE).unwrap()),
        ("rounded", std::fs::read_to_string(ROUNDED).unwrap()),
        ("demo", demo_text()),
        ("ties", ties_text()),
        ("offset", offset_text()),
    ];
    for (name, text) in boards {
        let model = model_of(&text, &[]);
        let cost = cost_of(&model, None);
        let mut state = as_read(&cost);
        assert_the_scores_own(&cost, &state, &model.text, &format!("{name} as read"));

        let mut rng = SplitMix64(0x5eed_0001);
        let mut most = 0;
        random_walk(
            &cost,
            &mut state,
            &mut rng,
            1000,
            &[0, 1, 2, 3, 4],
            |state, _, _| {
                most = most.max(state.crossings);
            },
        );
        assert!(most > 0, "{name}: the walk crossed nets");
        let text = written(&model, &state);
        let again = rebuilt(&cost, &text);
        assert_the_scores_own(
            &cost,
            &again,
            &text,
            &format!("{name} rebuilt after the walk"),
        );
        // The state the walk kept is the written board's too: its pads are where the
        // written board puts them.
        assert_the_scores_own(&cost, &state, &text, &format!("{name} after the walk"));
        assert_eq!(cost.terms(&state).airwire_mm, cost.terms(&again).airwire_mm);
    }
}

#[test]
fn the_ties_board_crosses_only_by_the_scores_tie_rule() {
    // The score joins D to B (the later of two equal candidates): no crossing. A tree
    // that kept the first equal edge, C-D, would cross M once.
    let model = model_of(&ties_text(), &[]);
    let cost = cost_of(&model, None);
    let terms = cost.terms(&as_read(&cost));
    assert_eq!(terms.crossings, 0);
    let c_to_d = ((20.0, 19.0), (23.0, 23.0));
    let m = ((20.2, 20.1), (21.0, 19.5));
    assert!(objectives::crosses(c_to_d.0, c_to_d.1, m.0, m.1).unwrap());
}

#[test]
fn a_states_caches_equal_a_full_evaluation() {
    let mut model = demo_with_pairs();
    model.clearances.part_gap_mm = 0.25;
    let hoods = demo_hoods();
    let cost = cost_of(&model, Some(&hoods));
    assert!(!cost.pairs.is_empty(), "the demo has decoupling pairs");
    let mut state = as_read(&cost);
    let mut rng = SplitMix64(0x5eed_0002);
    let mut seen = (0usize, 0usize, 0usize);
    let made = random_walk(
        &cost,
        &mut state,
        &mut rng,
        10_000,
        &[0, 0, 1, 2, 3, 3, 4],
        |state, mv, _| {
            let fresh = cost.state(state.centres.clone(), state.turns.clone());
            assert_same_caches(state, &fresh, &format!("after {mv:?}"));
            seen.0 = seen.0.max(state.crossings);
            seen.1 = seen.1.max(state.label_conflicts);
            seen.2 = seen.2.max(cost.terms(state).pair_violations);
        },
    );
    assert!(
        made.iter().all(|&n| n > 300),
        "every kind of move was made: {made:?}"
    );
    assert!(
        seen.0 > 0 && seen.1 > 0 && seen.2 > 0,
        "the walk crossed nets, labels and pairs: {seen:?}"
    );
}

// ─── Legality ────────────────────────────────────────────────────────────────

/// The demo with a part gap of 0.3 mm, and R1 and R2's indices.
fn gapped_demo() -> (BoardModel, usize, usize) {
    let mut model = demo_model();
    model.clearances.part_gap_mm = 0.3;
    let (r1, r2) = (index_of(&model, "R1"), index_of(&model, "R2"));
    (model, r1, r2)
}

#[test]
fn family_c_keeps_the_part_gap_between_real_courtyards() {
    let (model, r1, r2) = gapped_demo();
    let cost = cost_of(&model, None);
    let state = as_read(&cost);
    // R1 is 2 mm wide at (8, 32); R2 beside it, its left edge `gap` from R1's right edge.
    let beside = |gap: f64| Move {
        parts: vec![(r2, (8.0 + 2.0 + gap, 32.0), 0)],
    };
    assert!(!cost.legal(&state, &beside(0.003)), "touching at 3 µm");
    assert!(!cost.legal(&state, &beside(0.2)), "0.2 mm apart");
    assert!(cost.legal(&state, &beside(0.301)), "0.301 mm apart");
    // The same between the two parts of one move.
    let both = |gap: f64| Move {
        parts: vec![(r1, (30.0, 28.0), 0), (r2, (30.0 + 2.0 + gap, 28.0), 0)],
    };
    assert!(!cost.legal(&state, &both(0.003)));
    assert!(cost.legal(&state, &both(0.301)));
}

#[test]
fn a_start_whose_courtyards_touch_is_made_legal_first() {
    let (model, r1, r2) = gapped_demo();
    let cost = cost_of(&model, None);
    let part = |p: usize| &model.parts[p];
    let offset = |p: usize| {
        let c = cost.read_centres[p];
        (part(p).at.0 - c.0, part(p).at.1 - c.1)
    };
    // R2 put 3 µm right of R1 (legal for `legalize`, not for family C); R3 moved to a
    // free place that keeps the gap.
    let r3 = index_of(&model, "R3");
    let anchor = |p: usize, centre: (f64, f64)| IpcFootprintPlacement {
        reference: model.references[p].clone(),
        x: centre.0 + offset(p).0,
        y: centre.1 + offset(p).1,
        rotation: part(p).rotation_deg,
    };
    let start = vec![anchor(r2, (10.003, 32.0)), anchor(r3, (40.0, 26.0))];
    let state = state_from(&cost, &start).expect("the start");
    let (a, b) = (state.centres[r1], state.centres[r2]);
    let gap = (b.0 - a.0).abs() - 2.0;
    assert!(
        gap >= 0.3 - 1e-9 || (b.1 - a.1).abs() >= 1.3 - 1e-9,
        "R2 keeps the part gap from R1: {a:?} {b:?}"
    );
    assert_eq!(state.centres[r1], cost.read_centres[r1], "R1 did not move");
    assert!(
        (state.centres[r3].0 - 40.0).abs() < 1e-9 && (state.centres[r3].1 - 26.0).abs() < 1e-9,
        "R3 kept its place"
    );
    // Every moved part is legal where it ended.
    let moved = [r2, r3];
    for &p in &moved {
        let mv = Move {
            parts: vec![(p, state.centres[p], state.turns[p])],
        };
        assert!(cost.legal(&state, &mv), "{} is legal", model.references[p]);
    }

    // A start that names a held part, or a rotation that is not a quarter turn, is no start.
    let j1 = index_of(&model, "J1");
    assert!(matches!(
        state_from(&cost, &[anchor(j1, (50.0, 20.0))]),
        Err(GenerateFailure::Invalid(_))
    ));
    let mut tilted = anchor(r3, (40.0, 26.0));
    tilted.rotation = 45.0;
    assert!(matches!(
        state_from(&cost, &[tilted]),
        Err(GenerateFailure::Invalid(_))
    ));
}

/// R1 with a large courtyard and R2 with a small one overlapping it, as the input has them.
fn overlapping_pair() -> (BoardModel, usize, usize) {
    let parts = vec![
        Fp::new("R1", 20.0, 20.0).court(3.0, 1.0).two_pad("A", "B"),
        Fp::new("R2", 21.0, 20.0).court(0.5, 0.5).two_pad("A", "B"),
    ];
    let model = model_of(&board(OUTLINE, &parts), &[]);
    let (r1, r2) = (index_of(&model, "R1"), index_of(&model, "R2"));
    (model, r1, r2)
}

#[test]
fn the_parts_of_a_swap_are_tested_against_each_other() {
    let (model, r1, r2) = overlapping_pair();
    let cost = cost_of(&model, None);
    let state = as_read(&cost);
    // A swap of the two keeps them over each other: each is legal against the others,
    // not against the part it swapped with.
    let swap = Move {
        parts: vec![(r1, state.centres[r2], 0), (r2, state.centres[r1], 0)],
    };
    assert!(!cost.legal(&state, &swap), "the swap leaves an overlap");
    let apart = |x: f64| Move {
        parts: vec![(r1, (40.0, 20.0), 0), (r2, (x, 20.0), 0)],
    };
    assert!(
        !cost.legal(&state, &apart(42.0)),
        "two parts moved over each other"
    );
    assert!(cost.legal(&state, &apart(45.0)));
}

#[test]
fn a_move_that_names_a_held_part_is_illegal() {
    // The search tests a move before it costs one, so `legal` is where a held part is
    // refused; `delta` reads the state and checks nothing.
    let model = demo_model();
    let cost = cost_of(&model, None);
    let state = as_read(&cost);
    let (j1, r1) = (index_of(&model, "J1"), index_of(&model, "R1"));
    assert!(!model.movable[j1]);
    let free = (45.0, 26.0);
    assert!(cost.legal(
        &state,
        &Move {
            parts: vec![(r1, free, 0)]
        }
    ));
    assert!(!cost.legal(
        &state,
        &Move {
            parts: vec![(j1, free, 0)]
        }
    ));
    assert!(!cost.legal(
        &state,
        &Move {
            parts: vec![(r1, free, 0), (j1, state.centres[j1], 0)]
        }
    ));
    // Nor a part twice, no part at all, turns above 3, or a centre that is not finite.
    assert!(!cost.legal(
        &state,
        &Move {
            parts: vec![(r1, free, 0), (r1, (45.0, 30.0), 0)]
        }
    ));
    assert!(!cost.legal(&state, &Move { parts: Vec::new() }));
    assert!(!cost.legal(
        &state,
        &Move {
            parts: vec![(r1, free, 4)]
        }
    ));
    assert!(!cost.legal(
        &state,
        &Move {
            parts: vec![(r1, (f64::NAN, 26.0), 0)]
        }
    ));
}

#[test]
fn a_part_read_off_the_quarter_turns_is_never_turned() {
    let parts = vec![
        Fp::new("R1", 20.0, 20.0).rot(45.0).two_pad("A", "B"),
        Fp::new("R2", 40.0, 20.0).two_pad("A", "B"),
    ];
    let model = model_of(&board(OUTLINE, &parts), &[]);
    let (r1, r2) = (index_of(&model, "R1"), index_of(&model, "R2"));
    let cost = cost_of(&model, None);
    assert!(!cost.may_turn[r1] && cost.may_turn[r2]);
    let state = as_read(&cost);
    let turn = |p: usize, q: u8| Move {
        parts: vec![(p, state.centres[p], q)],
    };
    for q in 1..=3 {
        assert!(
            !cost.legal(&state, &turn(r1, q)),
            "R1, read at 45 degrees, turned {q}"
        );
        assert!(
            cost.legal(&state, &turn(r2, q)),
            "R2, read at 0 degrees, turned {q}"
        );
    }
    // It still moves.
    assert!(cost.legal(
        &state,
        &Move {
            parts: vec![(r1, (25.0, 30.0), 0)]
        }
    ));

    // With turns off, no part turns.
    let mut off = converge_params();
    off.turn_parts = false;
    let cost = CostModel::build(&model, None, &off).unwrap();
    assert!(!cost.legal(&state, &turn(r2, 1)));
}

#[test]
fn no_part_changes_side() {
    // A move names a centre and turns, never a side: after any walk, every footprint of
    // the written board is on the side it was read on.
    let model = demo_model();
    let cost = cost_of(&model, None);
    let mut state = as_read(&cost);
    let mut rng = SplitMix64(0x5eed_0003);
    random_walk(
        &cost,
        &mut state,
        &mut rng,
        300,
        &[0, 1, 2, 3],
        |_, _, _| {},
    );
    let text = written(&model, &state);
    let scan = konnect_sexp::board::footprint_courtyards(&parse(&text));
    for item in &scan.items {
        let reference = item.reference.as_deref().unwrap();
        let p = index_of(&model, reference);
        assert_eq!(item.layer_side, model.parts[p].layer_side, "{reference}");
    }
    assert!(model
        .parts
        .iter()
        .any(|p| p.layer_side == konnect_sexp::board::Side::Back));
}

// ─── The requirement terms ───────────────────────────────────────────────────

/// An unlocked connector in the middle (J2), one at the edge (J3), a locked one in the
/// middle (J4), and two resistors, every part movable unless KiCad locks it.
fn connector_board() -> BoardModel {
    let parts = vec![
        Fp::new("J2", 20.0, 20.0).two_pad("A", "B"),
        Fp::new("J3", 58.0, 20.0).two_pad("B", "C"),
        Fp::new("J4", 40.0, 20.0).locked().two_pad("C", "A"),
        Fp::new("R1", 30.0, 10.0).two_pad("A", "C"),
        Fp::new("R2", 30.0, 30.0).two_pad("B", "C"),
    ];
    model_with(&board(OUTLINE, &parts), &HoldRules::locks_only())
}

fn screened_connectors(model: &BoardModel, text: &str) -> usize {
    screen(&evaluated(model, text), &model.held, &[], 0, None)
        .unwrap()
        .connector_violations
}

#[test]
fn the_connector_term_counts_what_the_score_deducts() {
    let model = connector_board();
    let j4 = index_of(&model, "J4");
    assert!(!model.movable[j4] && model.movable[index_of(&model, "J2")]);
    let cost = cost_of(&model, None);
    let mut state = as_read(&cost);
    assert_eq!(cost.terms(&state).connector_violations, 2);
    assert_eq!(screened_connectors(&model, &model.text), 2);
    let mut rng = SplitMix64(0x5eed_0004);
    let mut counts = BTreeSet::new();
    for round in 0..20 {
        random_walk(&cost, &mut state, &mut rng, 10, &[0, 0, 2], |_, _, _| {});
        let text = written(&model, &state);
        let model_count = cost.terms(&rebuilt(&cost, &text)).connector_violations;
        assert_eq!(cost.terms(&state).connector_violations, model_count);
        assert_eq!(
            model_count,
            screened_connectors(&model, &text),
            "round {round}"
        );
        counts.insert(model_count);
    }
    assert!(counts.len() > 1, "the walk changed the count: {counts:?}");
}

#[test]
fn a_held_connector_changes_no_moves_cost() {
    let model = connector_board();
    let cost = cost_of(&model, None);
    let held = cost.part_connector[index_of(&model, "J4")].unwrap();
    let mut state = as_read(&cost);
    let constant = state.connector_mm[held];
    assert!(constant > CONNECTOR_EDGE_LIMIT_MM);
    let mut rng = SplitMix64(0x5eed_0005);
    random_walk(
        &cost,
        &mut state,
        &mut rng,
        500,
        &[0, 1, 2, 3, 4],
        |state, _, delta| {
            assert!(
                delta.connectors.iter().all(|&(c, _)| c != held),
                "{delta:?}"
            );
            assert_eq!(state.connector_mm[held], constant);
        },
    );
}

#[test]
fn an_unmeasured_pair_is_a_constant_violation_and_no_move_changes_it() {
    let text = board(OUTLINE, &demo_parts());
    let set = held_set(&text, &BTreeSet::new(), &HoldRules::default()).unwrap();
    // U1 has no pad 9.
    let pairs = vec![DecouplingPair {
        cap: "C1".to_string(),
        cap_pad: "1".to_string(),
        ic: "U1".to_string(),
        ic_pad: "9".to_string(),
        net: "VCC".to_string(),
        limit_mm: 3.0,
        basis: "test".to_string(),
    }];
    let model = board_model(&text, set, Intent::new(Vec::new(), pairs.clone())).unwrap();
    let cost = cost_of(&model, None);
    assert!(cost.pairs.is_empty() && cost.unmeasured_pairs == 1);
    let mut state = as_read(&cost);
    let check = |state: &State, text: &str| {
        let terms = cost.terms(state);
        assert_eq!(terms.pair_violations, 1);
        assert_eq!(terms.pair_violations, pair_violations(&parse(text), &pairs));
        assert_eq!(terms.pair_mm, 0.0);
        assert!((terms.cost - terms.soft - converge_params().requirement_mm).abs() < 1e-9);
    };
    check(&state, &model.text);
    let mut rng = SplitMix64(0x5eed_0006);
    random_walk(
        &cost,
        &mut state,
        &mut rng,
        300,
        &[0, 1, 2, 3],
        |state, _, delta| {
            assert!(delta.pairs.is_empty());
            assert_eq!(cost.terms(state).pair_violations, 1);
        },
    );
    check(&state, &written(&model, &state));
}

#[test]
fn a_measured_pair_is_the_screens_count() {
    let model = demo_with_pairs();
    let cost = cost_of(&model, None);
    assert!(cost.pairs.len() == 8 && cost.unmeasured_pairs == 0);
    let mut state = as_read(&cost);
    let mut rng = SplitMix64(0x5eed_0007);
    let mut counts = BTreeSet::new();
    for _ in 0..20 {
        random_walk(
            &cost,
            &mut state,
            &mut rng,
            15,
            &[0, 1, 2, 3, 4],
            |_, _, _| {},
        );
        let text = written(&model, &state);
        let count = cost.terms(&state).pair_violations;
        assert_eq!(count, pair_violations(&parse(&text), &model.intent.pairs));
        counts.insert(count);
    }
    assert!(counts.len() > 1, "{counts:?}");
}

// ─── The soft terms ──────────────────────────────────────────────────────────

/// The length of net `net`'s tree on the model's board, by the score's tree.
fn tree_of(model: &BoardModel, net: &str) -> f64 {
    let sites: Vec<Site<'_>> = model
        .index
        .pads_of_net(net)
        .iter()
        .map(|pad| Site {
            reference: &pad.reference,
            pad: &pad.pad_number,
            at: pad.at,
        })
        .collect();
    objectives::spanning_tree(&sites).unwrap().0
}

/// The demo with a zone on `net`.
fn demo_with_pour(net: &str) -> BoardModel {
    let zone = format!(
        "(zone (net \"{net}\") (layer \"F.Cu\") (uuid \"77777777-0000-4000-8000-000000000001\") (polygon (pts (xy 0 0) (xy 60 0) (xy 60 40) (xy 0 40))))\n)\n"
    );
    let text = demo_text();
    let text = format!("{}{zone}", text.trim_end().strip_suffix(')').unwrap());
    model_of(&text, &[])
}

#[test]
fn a_rail_with_a_pour_weighs_nothing_and_one_without_weighs_the_rail_weight() {
    // The demo's GND (11 references) and VCC (9) are rails at fanout 8.
    let plain = demo_model();
    let (gnd, vcc) = (tree_of(&plain, "GND"), tree_of(&plain, "VCC"));
    assert!(gnd > 0.0 && vcc > 0.0);
    let rails = |model: &BoardModel, params: &ConvergeParams| {
        let cost = CostModel::build(model, None, params).unwrap();
        let terms = cost.terms(&as_read(&cost));
        let rest = terms.airwire_mm
            + params.crossing_mm * terms.crossings as f64
            + terms.pair_mm
            + params.label_mm * terms.label_conflicts as f64;
        (terms.rail_airwire_mm, terms.soft - rest)
    };
    let params = converge_params();
    let (counted, weighed) = rails(&plain, &params);
    assert!((counted - (gnd + vcc)).abs() < 1e-9, "no pour: both rails");
    assert!((weighed - 0.3 * (gnd + vcc)).abs() < 1e-9);
    let poured = demo_with_pour("GND");
    let (counted, weighed) = rails(&poured, &params);
    assert!(
        (counted - vcc).abs() < 1e-9,
        "a pour on GND: VCC alone, {counted}"
    );
    assert!((weighed - 0.3 * vcc).abs() < 1e-9);
    let mut none = params;
    none.rail_weight = 0.0;
    assert_eq!(rails(&plain, &none), (0.0, 0.0));
    // A move of a part on the poured rail only costs nothing in length.
    let cost = cost_of(&poured, None);
    let h1 = index_of(&poured, "H1");
    assert!(cost.part_nets[h1].is_empty(), "H1 is on GND alone");
}

#[test]
fn the_neighbour_term_is_the_schematic_measures_sum() {
    let model = demo_model();
    let hoods = demo_hoods();
    let cost = cost_of(&model, Some(&hoods));
    let mut state = as_read(&cost);
    let sum = |text: &str| -> f64 { part_distances(&parse(text), &hoods).unwrap().values().sum() };
    let measured = cost.terms(&state).neighbour_mm.unwrap();
    assert!((measured - sum(&model.text)).abs() < 1e-9, "{measured}");
    let mut rng = SplitMix64(0x5eed_0008);
    for round in 0..10 {
        random_walk(
            &cost,
            &mut state,
            &mut rng,
            30,
            &[0, 1, 2, 3, 4],
            |_, _, _| {},
        );
        let text = written(&model, &state);
        let again = cost.terms(&rebuilt(&cost, &text)).neighbour_mm.unwrap();
        assert!(
            (again - sum(&text)).abs() < 1e-9,
            "round {round}: {again} against {}",
            sum(&text)
        );
        // The state the walk kept, to the 1 µm the anchors are rounded to.
        let kept = cost.terms(&state).neighbour_mm.unwrap();
        assert!(
            (kept - again).abs() < 1e-4,
            "round {round}: {kept} against {again}"
        );
    }
    // Without neighbourhoods there is no term.
    let none = cost_of(&model, None);
    assert_eq!(none.terms(&as_read(&none)).neighbour_mm, None);
    // A name that is not on the board is refused.
    let stranger = hoods_of(&[("C1", &["X9"])]);
    assert!(matches!(
        CostModel::build(&model, Some(&stranger), &converge_params()),
        Err(GenerateFailure::Invalid(_))
    ));
}

// ─── Labels ──────────────────────────────────────────────────────────────────

/// `text` with the `(at …)` and layer of `reference`'s Reference property replaced by
/// `rest` (for instance `(at 10 0 0) (layer "F.SilkS")`), and `extra` footprint items
/// before it.
fn relabel(text: &str, reference: &str, rest: &str, extra: &str) -> String {
    let old = format!("(property \"Reference\" \"{reference}\" (at 0 -1.5 0) (layer \"F.SilkS\")");
    assert_eq!(
        text.matches(&old).count(),
        1,
        "{reference} has its Reference label"
    );
    text.replace(
        &old,
        &format!("{extra}(property \"Reference\" \"{reference}\" {rest}"),
    )
}

/// R1 at (20, 20) and R2 at (30, 20), each 2 x 1 mm, with their labels moved.
fn labelled(r1: &str, r2: &str, extra: &str) -> CostModel<'static> {
    let parts = vec![
        Fp::new("R1", 20.0, 20.0).two_pad("A", "B"),
        Fp::new("R2", 30.0, 20.0).two_pad("A", "B"),
    ];
    let text = relabel(&board(OUTLINE, &parts), "R1", r1, extra);
    let text = relabel(&text, "R2", r2, "");
    let model: &'static BoardModel = Box::leak(Box::new(model_of(&text, &[])));
    cost_of(model, None)
}

fn conflicts(cost: &CostModel<'_>) -> usize {
    cost.terms(&as_read(cost)).label_conflicts
}

/// R2's label out of the way: 3 mm above it.
const R2_AWAY: &str = "(at 0 -3 0) (layer \"F.SilkS\")";

#[test]
fn a_label_over_another_part_is_a_conflict_and_over_its_own_is_not() {
    assert_eq!(
        conflicts(&labelled("(at 0 0 0) (layer \"F.SilkS\")", R2_AWAY, "")),
        0,
        "over its own"
    );
    assert_eq!(
        conflicts(&labelled("(at 10 0 0) (layer \"F.SilkS\")", R2_AWAY, "")),
        1,
        "over R2"
    );
    // The builder's labels, 1.5 mm above each part, 10 mm apart: none.
    let parts = vec![
        Fp::new("R1", 20.0, 20.0).two_pad("A", "B"),
        Fp::new("R2", 30.0, 20.0).two_pad("A", "B"),
    ];
    let model = model_of(&board(OUTLINE, &parts), &[]);
    assert_eq!(conflicts(&cost_of(&model, None)), 0);
}

#[test]
fn two_parts_labels_over_each_other_are_one_conflict() {
    let cost = labelled(
        "(at 5 -4 0) (layer \"F.SilkS\")",
        "(at -5 -4 0) (layer \"F.SilkS\")",
        "",
    );
    assert_eq!(conflicts(&cost), 1);
}

#[test]
fn a_hidden_label_counts_nothing() {
    let font = "(effects (font (size 1 1) (thickness 0.15)))";
    let away = "(at 0 -3 0) (layer \"F.SilkS\")";
    // The same text over R2, visible: one conflict.
    let shown = format!("(fp_text user \"XX\" (at 10 0) (layer \"F.SilkS\") {font})\n");
    assert_eq!(conflicts(&labelled(away, R2_AWAY, &shown)), 1);
    let hidden = [
        format!("(property \"Note\" \"XX\" (at 10 0 0) (layer \"F.SilkS\") (hide yes) {font})\n"),
        format!("(fp_text user \"XX\" (at 10 0) (layer \"F.SilkS\") hide {font})\n"),
        "(fp_text user \"XX\" (at 10 0) (layer \"F.SilkS\") (effects (font (size 1 1) (thickness 0.15)) hide))\n".to_string(),
        "(fp_text user \"XX\" (at 10 0) (layer \"F.SilkS\") (effects (font (size 1 1) (thickness 0.15)) (hide yes)))\n".to_string(),
    ];
    for extra in &hidden {
        assert_eq!(conflicts(&labelled(away, R2_AWAY, extra)), 0, "{extra}");
    }
    // `(hide no)` is shown; a text on a layer that is not silk is no label.
    let not_hidden =
        format!("(property \"Note\" \"XX\" (at 10 0 0) (layer \"F.SilkS\") (hide no) {font})\n");
    assert_eq!(conflicts(&labelled(away, R2_AWAY, &not_hidden)), 1);
    let fab = format!("(fp_text user \"XX\" (at 10 0) (layer \"F.Fab\") {font})\n");
    assert_eq!(conflicts(&labelled(away, R2_AWAY, &fab)), 0);
}

#[test]
fn a_label_on_the_other_side_of_the_board_counts_nothing() {
    // R1 on top; R3 and R4 on the back; T1 a through-hole part (both sides) on top.
    let parts = vec![
        Fp::new("R1", 20.0, 20.0).two_pad("A", "B"),
        Fp::new("R3", 30.0, 20.0).back().two_pad("A", "B"),
        Fp::new("R4", 40.0, 20.0).back().two_pad("A", "B"),
        Fp::new("T1", 20.0, 30.0).two_pad("A", "B"),
    ];
    let base = board(OUTLINE, &parts);
    let through = |text: &str| {
        // T1's pads go through the board.
        let block = text.find("\"R4\"").unwrap();
        let t1 = block + text[block..].find("(footprint").unwrap();
        let (head, tail) = text.split_at(t1);
        format!("{head}{}", tail.replace("smd rect", "thru_hole rect"))
    };
    let count = |text: String| {
        let model: &'static BoardModel = Box::leak(Box::new(model_of(&text, &[])));
        conflicts(&cost_of(model, None))
    };
    let away = "(at 0 -3 0) (layer \"F.SilkS\")";
    let back_away = "(at 0 -3 0) (layer \"B.SilkS\")";
    let quiet = |text: &str| {
        let text = relabel(text, "R3", back_away, "");
        let text = relabel(&text, "R4", back_away, "");
        relabel(&text, "T1", away, "")
    };
    assert_eq!(
        count(through(&quiet(&relabel(&base, "R1", away, "")))),
        0,
        "nothing over anything"
    );
    // R1's top label over R3 on the back.
    let r1_over_r3 = relabel(&base, "R1", "(at 10 0 0) (layer \"F.SilkS\")", "");
    assert_eq!(
        count(through(&quiet(&r1_over_r3))),
        0,
        "a top label over a back part"
    );
    // R3 on the back with a label on the top silk (an odd footprint) over R1: the two
    // parts share no side.
    let r3_odd = relabel(&base, "R3", "(at -10 0 0) (layer \"F.SilkS\")", "");
    let r3_odd = relabel(&r3_odd, "R4", back_away, "");
    let r3_odd = relabel(&relabel(&r3_odd, "T1", away, ""), "R1", away, "");
    assert_eq!(count(through(&r3_odd)), 0, "parts that share no side");
    // T1 (both sides) with a top label over R3: R3 is not on the top.
    let t1_over_r3 = relabel(&base, "T1", "(at 10 -10 0) (layer \"F.SilkS\")", "");
    let t1_over_r3 = relabel(
        &relabel(&t1_over_r3, "R3", back_away, ""),
        "R4",
        back_away,
        "",
    );
    let t1_over_r3 = relabel(&t1_over_r3, "R1", away, "");
    assert_eq!(
        count(through(&t1_over_r3)),
        0,
        "a top label over a part on the back only"
    );
    // R3's back label over R4 on the back: a conflict, on its own side.
    let r3_over_r4 = relabel(&base, "R3", "(at 10 0 0) (layer \"B.SilkS\")", "");
    let r3_over_r4 = relabel(&relabel(&r3_over_r4, "R4", back_away, ""), "T1", away, "");
    let r3_over_r4 = relabel(&r3_over_r4, "R1", away, "");
    assert_eq!(
        count(through(&r3_over_r4)),
        1,
        "a back label over a back part"
    );
}

#[test]
fn a_label_is_placed_through_its_footprints_transform_and_turns_with_its_part() {
    // R1 at (20, 20) turned 90 degrees, its label at (4, 0) in the footprint, stored at
    // 90 degrees (absolute): on the board it is at (20, 16), 1.85 wide and 2.75 tall.
    let parts = vec![
        Fp::new("R1", 20.0, 20.0).rot(90.0).two_pad("A", "B"),
        Fp::new("R2", 50.0, 30.0).two_pad("A", "B"),
    ];
    let text = relabel(
        &board(OUTLINE, &parts),
        "R1",
        "(at 4 0 90) (layer \"F.SilkS\")",
        "",
    );
    let text = relabel(&text, "R2", R2_AWAY, "");
    let model = model_of(&text, &[]);
    let cost = cost_of(&model, None);
    let r1 = index_of(&model, "R1");
    let centre = cost.read_centres[r1];
    let close = |a: Rect, b: Rect| {
        [a.0 - b.0, a.1 - b.1, a.2 - b.2, a.3 - b.3]
            .iter()
            .all(|d| d.abs() < 1e-9)
    };
    let label = cost.labels[r1][0];
    let on_board = shifted(label.boxes[0], centre);
    assert!(
        close(
            on_board,
            (20.0 - 0.925, 16.0 - 1.375, 20.0 + 0.925, 16.0 + 1.375)
        ),
        "{on_board:?}"
    );

    // Turned once more, the label goes round the part's centre with it, as the written
    // board has it.
    let mut state = as_read(&cost);
    let mv = Move {
        parts: vec![(r1, centre, 1)],
    };
    assert!(cost.legal(&state, &mv));
    let delta = cost.delta(&state, &mv);
    cost.apply(&mut state, &mv, delta);
    let turned = shifted(label.boxes[1], state.centres[r1]);
    let again = model_of(&written(&model, &state), &[]);
    let rewritten = cost_of(&again, None);
    let r1_again = index_of(&again, "R1");
    let read_back = shifted(
        rewritten.labels[r1_again][0].boxes[0],
        rewritten.read_centres[r1_again],
    );
    let near = |a: Rect, b: Rect| {
        [a.0 - b.0, a.1 - b.1, a.2 - b.2, a.3 - b.3]
            .iter()
            .all(|d| d.abs() < 1e-6)
    };
    assert!(near(turned, read_back), "{turned:?} against {read_back:?}");
    assert!(!near(turned, on_board), "the label turned");

    // And conflicts follow: R2 under the label as read, then under the turned label.
    let under = |at: (f64, f64)| {
        let mut state = as_read(&cost);
        let r2 = index_of(&model, "R2");
        let mv = Move {
            parts: vec![(r2, at, 0)],
        };
        let delta = cost.delta(&state, &mv);
        cost.apply(&mut state, &mv, delta);
        let before = state.label_conflicts;
        let mv = Move {
            parts: vec![(r1, centre, 1)],
        };
        let delta = cost.delta(&state, &mv);
        cost.apply(&mut state, &mv, delta);
        (before, state.label_conflicts)
    };
    let turned_centre = ((turned.0 + turned.2) / 2.0, (turned.1 + turned.3) / 2.0);
    assert_eq!(under((20.0, 16.0)), (1, 0));
    assert_eq!(under(turned_centre).1, 1);
}

#[test]
fn a_label_weight_of_0_keeps_the_count_out_of_the_cost() {
    let over = "(at 10 0 0) (layer \"F.SilkS\")";
    let parts = vec![
        Fp::new("R1", 20.0, 20.0).two_pad("A", "B"),
        Fp::new("R2", 30.0, 20.0).two_pad("A", "B"),
    ];
    let text = relabel(&board(OUTLINE, &parts), "R1", over, "");
    let text = relabel(&text, "R2", R2_AWAY, "");
    let model = model_of(&text, &[]);
    let on = cost_of(&model, None);
    let mut params = converge_params();
    params.label_mm = 0.0;
    let off = CostModel::build(&model, None, &params).unwrap();
    let (on_terms, off_terms) = (on.terms(&as_read(&on)), off.terms(&as_read(&off)));
    assert_eq!(
        (on_terms.label_conflicts, off_terms.label_conflicts),
        (1, 1)
    );
    assert!((on_terms.cost - off_terms.cost - 0.2).abs() < 1e-9);
    // A move that clears the label: the same change but the label's 0.2 mm.
    let r2 = index_of(&model, "R2");
    let mv = Move {
        parts: vec![(r2, (40.0, 30.0), 0)],
    };
    let (d_on, d_off) = (on.delta(&as_read(&on), &mv), off.delta(&as_read(&off), &mv));
    assert_eq!((d_on.labels, d_off.labels), (-1, -1));
    assert!((d_off.cost - d_on.cost - 0.2).abs() < 1e-9);
}

// ─── Speed (not a guard) ─────────────────────────────────────────────────────

/// The time of one `delta` on the demo, for stage 3 and the measurement: 100,000 legal
/// moves of every kind, costed against the state as read. Run by hand:
/// `cargo test -p konnect-core --lib -- place_converge::tests::time_one_delta_on_the_demo --exact --ignored --nocapture`.
#[test]
#[ignore]
fn time_one_delta_on_the_demo() {
    let model = demo_with_pairs();
    let hoods = demo_hoods();
    time_deltas("demo", &cost_of(&model, Some(&hoods)));
    // 96 two-pad parts in a 12 x 8 grid on a 100 x 70 board, each on two signal nets
    // and every fourth one on a rail: about the size of a real board.
    let parts: Vec<Fp> = (0..96)
        .map(|k| {
            let (i, j) = (k % 12, k / 12);
            let fp = Fp::new(
                &format!("R{}", k + 1),
                6.0 + 8.0 * i as f64,
                6.0 + 8.0 * j as f64,
            )
            .pad(pad("1", -0.5, 0.0, Some(&format!("N{k}"))))
            .pad(pad("2", 0.5, 0.0, Some(&format!("N{}", (k * 7 + 3) % 96))));
            if k % 4 == 0 {
                fp.pad(pad("3", 0.0, 0.4, Some("GND")))
            } else {
                fp
            }
        })
        .collect();
    let model = model_of(&board(Shape::Rect(0.0, 0.0, 100.0, 70.0), &parts), &[]);
    time_deltas("96 parts", &cost_of(&model, None));
}

fn time_deltas(name: &str, cost: &CostModel<'_>) {
    let state = as_read(cost);
    let mut rng = SplitMix64(0x5eed_0009);
    let mut moves = Vec::new();
    while moves.len() < 100_000 {
        let kind = [0, 0, 0, 1, 2][rng.below(5) as usize];
        let mv = random_move(cost, &state, &mut rng, kind);
        if cost.legal(&state, &mv) {
            moves.push(mv);
        }
    }
    let started = std::time::Instant::now();
    let mut sum = 0.0;
    for mv in &moves {
        sum += cost.delta(&state, mv).cost;
    }
    let delta = started.elapsed();
    let started = std::time::Instant::now();
    let mut legal = 0;
    for mv in &moves {
        legal += usize::from(cost.legal(&state, mv));
    }
    let legality = started.elapsed();
    println!(
        "{name}: {} deltas: {:.2} µs each; legal: {:.2} µs each ({legal}, {sum:.3})",
        moves.len(),
        delta.as_secs_f64() * 1e6 / moves.len() as f64,
        legality.as_secs_f64() * 1e6 / moves.len() as f64
    );
}
