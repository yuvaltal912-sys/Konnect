//! Tests of `place_converge`, the convergent placement search (family C): the cost model
//! (stage 2), and the search, its moves, its rules and whole runs (stage 3).

use super::*;
use crate::place_generators::tests::{
    converge_params, demo_model, demo_parts, demo_text, model_of,
};
use crate::place_generators::{
    board_model, generate, placements_from, recipes, unit, ConvergeRecipe, Family, Matrix, Params,
    Recipe,
};
use crate::place_held::tests::{board, evaluate_text, pad, parse, Fp, Shape};
use crate::place_held::{
    check_held, decoupling_pairs, groups, held_set, placements_of, DecouplingPair,
};
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
/// three or four parts at once (3), a part back where and as it was read (4), a proposal
/// of the search's own (`propose`, 5), making room for a broken pair (`make_room`, 6).
/// `None` when the search's own has nothing to propose.
fn random_move(
    cost: &CostModel<'_>,
    state: &State,
    rng: &mut SplitMix64,
    kind: u64,
) -> Option<Move> {
    let movable = &cost.movable;
    match kind {
        5 => {
            let p = movable[rng.below(movable.len() as u64) as usize];
            let radius = 0.5 + 6.0 * unit(rng);
            return cost.propose(state, p, radius, rng);
        }
        6 => {
            let broken: Vec<usize> = (0..cost.pairs.len())
                .filter(|&k| state.pair_mm[k] > cost.pairs[k].limit_mm)
                .collect();
            if broken.is_empty() {
                return None;
            }
            let k = broken[rng.below(broken.len() as u64) as usize];
            return cost.make_room(state, k);
        }
        _ => {}
    }
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
    Some(Move { parts })
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
    let mut made = vec![0; 7];
    let mut tries = 0;
    while made.iter().sum::<usize>() < count {
        tries += 1;
        assert!(tries < 200 * count, "too few legal moves: {made:?}");
        let kind = kinds[rng.below(kinds.len() as u64) as usize];
        let Some(mv) = random_move(cost, state, rng, kind) else {
            continue;
        };
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
        &[0, 0, 1, 2, 3, 3, 4, 5, 5, 5, 6],
        |state, mv, _| {
            let fresh = cost.state(state.centres.clone(), state.turns.clone());
            assert_same_caches(state, &fresh, &format!("after {mv:?}"));
            seen.0 = seen.0.max(state.crossings);
            seen.1 = seen.1.max(state.label_conflicts);
            seen.2 = seen.2.max(cost.terms(state).pair_violations);
        },
    );
    assert!(
        made[..6].iter().all(|&n| n > 300),
        "every kind of move was made: {made:?}"
    );
    assert!(made[6] >= 20, "making room was made: {made:?}");
    // `propose`'s four kinds, each made: a walk of its own moves, classified by the draw
    // `propose` makes first.
    let mut kinds = [0usize; 4];
    let mut tries = 0;
    while kinds.iter().any(|&n| n < 100) {
        tries += 1;
        assert!(tries < 200_000, "propose's kinds: {kinds:?}");
        let p = cost.movable[rng.below(cost.movable.len() as u64) as usize];
        let draw = unit(&mut rng.clone());
        let Some(mv) = cost.propose(&state, p, 4.0, &mut rng) else {
            continue;
        };
        if !cost.legal(&state, &mv) {
            continue;
        }
        let kind = match draw {
            d if d < SHIFT_SHARE => 0,
            d if d < SHIFT_SHARE + TURN_SHARE => 1,
            d if d < SHIFT_SHARE + TURN_SHARE + SWAP_SHARE => 2,
            _ => 3,
        };
        kinds[kind] += 1;
        let delta = cost.delta(&state, &mv);
        cost.apply(&mut state, &mv, delta);
        let fresh = cost.state(state.centres.clone(), state.turns.clone());
        assert_same_caches(&state, &fresh, &format!("after the proposal {mv:?}"));
    }

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
    let model = ninety_six_parts();
    time_deltas("96 parts", &cost_of(&model, None));
}

/// 96 two-pad parts in a 12 x 8 grid on a 100 x 70 board, each on two signal nets and
/// every fourth one on a rail: about the size of a real board.
fn ninety_six_parts() -> BoardModel {
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
    model_of(&board(Shape::Rect(0.0, 0.0, 100.0, 70.0), &parts), &[])
}

fn time_deltas(name: &str, cost: &CostModel<'_>) {
    let state = as_read(cost);
    let mut rng = SplitMix64(0x5eed_0009);
    let mut moves = Vec::new();
    while moves.len() < 100_000 {
        let kind = [0, 0, 0, 1, 2][rng.below(5) as usize];
        let mv = random_move(cost, &state, &mut rng, kind).unwrap();
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

// ─── The rules, read alone ───────────────────────────────────────────────────

#[test]
fn quiet_enough_needs_the_quiet_stages_and_the_cold() {
    // (quiet stages in a row, uphill share, level, ends)
    let table: [(usize, f64, u32, bool); 10] = [
        (5, 0.0, 0, true),
        (5, 0.009, 0, true),
        (9, 0.005, 0, true),
        (4, 0.0, 0, false),
        (0, 0.0, 0, false),
        (5, 0.01, 0, false),
        // The control: 5 quiet stages at an uphill share of 3 % do not end the run.
        (5, 0.03, 0, false),
        (5, 0.0, 1, false),
        (10, 0.0, 1, true),
        (20, 0.005, 2, true),
    ];
    for (run, share, level, ends) in table {
        assert_eq!(
            quiet_enough(run, share, &ConvergeParams::measured(level)),
            ends,
            "{run} quiet stages at an uphill share of {share}, level {level}"
        );
    }
}

#[test]
fn the_shift_radius_follows_the_temperature_at_every_level() {
    // The radius part of `a_level_scales_the_cooling_the_patience_and_the_quiet_gain`:
    // at one temperature, the same radius at levels 0 to 3; and level k reaches level
    // 0's temperature after one stage in 2^k stages, with the same radius.
    let (r0, first) = (20.0, 300.0);
    let base = ConvergeParams::measured(0);
    for t in [300.0, 120.0, 17.5, 0.3, 1e-4] {
        let at_0 = shift_radius(&base, r0, t, first);
        for level in 1..=3u32 {
            let params = ConvergeParams::measured(level);
            assert_eq!(
                shift_radius(&params, r0, t, first),
                at_0,
                "T {t}, level {level}"
            );
        }
    }
    assert_eq!(shift_radius(&base, r0, first, first), r0);
    assert_eq!(shift_radius(&base, r0, first * 1e-9, first), 1.0);
    for level in 0..=3u32 {
        let params = ConvergeParams::measured(level);
        let stages = 1i32 << level;
        let t = first * params.cooling_at_level().powi(stages * 10);
        let t0 = first * base.cooling.powi(10);
        assert!((t / t0 - 1.0).abs() < 1e-12, "level {level}");
        let (r, r_0) = (
            shift_radius(&params, r0, t, first),
            shift_radius(&base, r0, t0, first),
        );
        assert!((r - r_0).abs() < 1e-9, "level {level}: {r} against {r_0}");
    }
}

#[test]
fn starts_stop_reads_the_costs_in_start_order() {
    let rule = OuterRule::measured();
    let inf = f64::INFINITY;
    // The experiment's 8 generated starts in start order (design, "What was measured"):
    // the first ended best, at 1195; the other seven between 1255 and 2201, and 3260 for
    // the run that ended with two broken pairs. The design gives no more of their order,
    // so the filler values and their orders are this test's: any order of the seven
    // gives 4, because the first start is the best and three starts in a row then fail
    // to pay.
    let experiment = [
        1195.0, 1255.0, 2201.0, 3260.0, 1400.0, 1600.0, 1800.0, 2000.0,
    ];
    assert_eq!(starts_stop(&experiment, &rule), Some(4));
    let mut reversed = experiment;
    reversed[1..].reverse();
    assert_eq!(starts_stop(&reversed, &rule), Some(4));
    let mut sorted = experiment;
    sorted[1..].sort_by(f64::total_cmp);
    assert_eq!(starts_stop(&sorted, &rule), Some(4));
    // The best of the 4 is the best of all 8.
    let best = |costs: &[f64]| costs.iter().copied().fold(inf, f64::min);
    assert_eq!(best(&experiment[..4]), best(&experiment));

    // (costs in start order, the n the rule gives)
    let table: Vec<(Vec<f64>, Option<usize>)> = vec![
        (vec![], None),
        (vec![100.0], None),
        (vec![100.0, 101.0, 102.0], None),
        (vec![100.0, 101.0, 102.0, 103.0], Some(4)),
        // A start that pays (below 0.99 x the best of the starts before the window)
        // keeps the search going.
        (vec![100.0, 101.0, 98.0, 103.0], None),
        (vec![100.0, 101.0, 98.0, 103.0, 104.0], None),
        (vec![100.0, 101.0, 98.0, 103.0, 104.0, 105.0], Some(6)),
        (vec![100.0, 99.5, 99.2, 99.1], Some(4)),
        (vec![100.0, 90.0, 80.0, 70.0, 69.9, 69.8, 69.7], Some(7)),
        // A failed start is infinity: three failures in a row stop the search, and a
        // first finite cost pays against every failure before it.
        (vec![inf, inf, inf, inf], Some(4)),
        (vec![inf, inf, inf, 500.0], None),
        (vec![inf, inf, inf, 500.0, inf, inf, inf], Some(7)),
        (vec![100.0, inf, inf, inf], Some(4)),
    ];
    for (costs, n) in &table {
        assert_eq!(starts_stop(costs, &rule), *n, "{costs:?}");
    }

    // The guard: at `max_starts` whatever the costs say.
    let falling: Vec<f64> = (0..100).map(|k| 1000.0 * 0.9f64.powi(k)).collect();
    assert_eq!(starts_stop(&falling, &rule), Some(64));
    assert_eq!(starts_stop(&falling[..63], &rule), None);
    let short = OuterRule {
        max_starts: 2,
        ..rule
    };
    assert_eq!(starts_stop(&[100.0, 50.0, 25.0], &short), Some(2));

    // The control: a window of 1 stops after 2 on the experiment's list.
    let one = OuterRule { window: 1, ..rule };
    assert_eq!(starts_stop(&experiment, &one), Some(2));
}

#[test]
fn effort_stop_ends_at_the_first_level_that_does_not_pay() {
    let rule = OuterRule::measured();
    // (cost at levels 0, 1, ..., the levels the rule gives)
    let table: Vec<(Vec<f64>, Option<usize>)> = vec![
        (vec![], None),
        (vec![1000.0], None),
        // The experiment's three starts at levels 0 to 2: each doubling paid.
        (vec![2212.0, 1208.0, 1124.0], None),
        (vec![1241.0, 1139.0, 1109.0], None),
        (vec![2046.0, 1994.0, 1919.0], None),
        (vec![1000.0, 995.0], Some(2)),
        (vec![1000.0, 1010.0], Some(2)),
        (vec![1000.0, 980.0, 975.0], Some(3)),
        // Read against the best of the levels below, not the last one.
        (vec![1000.0, 1100.0], Some(2)),
        (vec![1000.0, 980.0, 1200.0, 900.0], Some(3)),
        // A failed level 0 is paid by any finite level 1.
        (vec![f64::INFINITY, 900.0, 880.0], None),
    ];
    for (costs, n) in &table {
        assert_eq!(effort_stop(costs, &rule), *n, "{costs:?}");
    }
    // The guard: after `max_level` (level 5, the sixth cost) whatever the costs say.
    let halving: Vec<f64> = (0..10).map(|k| 1000.0 * 0.5f64.powi(k)).collect();
    assert_eq!(effort_stop(&halving, &rule), Some(6));
    assert_eq!(effort_stop(&halving[..5], &rule), None);
    let flat = OuterRule {
        max_level: 0,
        ..rule
    };
    assert_eq!(effort_stop(&[1000.0], &flat), Some(1));

    // The control: a gain of 0 never stops before the guard on levels that each gain
    // 0.5 %, where the measured gain stops after level 1.
    let creeping: Vec<f64> = (0..10).map(|k| 1000.0 * 0.995f64.powi(k)).collect();
    assert_eq!(effort_stop(&creeping, &rule), Some(2));
    let any = OuterRule { gain: 0.0, ..rule };
    assert_eq!(effort_stop(&creeping, &any), Some(6));
}

// ─── Whole runs ──────────────────────────────────────────────────────────────

/// The measured parameters at `level` with 20 proposals per part and stage, not 200:
/// whole-run tests in a debug build take seconds, not minutes. Only the stage length
/// differs; the rules and every weight are the measured ones.
fn quick(level: u32) -> ConvergeParams {
    ConvergeParams {
        stage_per_part: 20,
        ..ConvergeParams::measured(level)
    }
}

/// One run from the board as read (an empty start).
fn run(
    model: &BoardModel,
    hoods: Option<&Neighbourhoods>,
    seed: u64,
    params: &ConvergeParams,
) -> (Candidate, Trace) {
    converge(model, hoods, &[], seed, params).expect("a run")
}

/// The board a candidate writes.
fn board_of(model: &BoardModel, candidate: &Candidate) -> String {
    placements_applied_to(&model.text, &candidate.placements).expect("the board is written")
}

/// The hard failures of `text` that are courtyard overlaps or parts outside the outline
/// and name a movable part.
fn movable_hard_failures(model: &BoardModel, text: &str) -> Vec<serde_json::Value> {
    let scored = evaluated(model, text);
    scored["hard_failures"]
        .as_array()
        .expect("the score lists its hard failures")
        .iter()
        .filter(|f| {
            matches!(
                f["kind"].as_str(),
                Some("courtyard_overlap" | "outside_outline")
            )
        })
        .filter(|f| {
            f["references"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| model.held.movable.contains(r.as_str().unwrap()))
        })
        .cloned()
        .collect()
}

/// A crowded board: U1 and 21 resistors in a chain on a 22 x 12 mm board, a part gap of
/// 0.3 mm and an edge clearance of 0.5 mm, so that most moves are refused.
fn crowded() -> BoardModel {
    let mut parts = vec![Fp::new("U1", 11.0, 7.0)
        .court(1.5, 1.5)
        .pad(pad("1", -1.0, -1.0, Some("N0")))
        .pad(pad("2", 1.0, 0.0, Some("N10")))
        .pad(pad("3", -1.0, 1.0, Some("N21")))];
    for k in 0..21 {
        let (row, column) = (k / 7, k % 7);
        let y = [1.5, 3.5, 10.5][row];
        parts.push(
            Fp::new(&format!("R{}", k + 1), 2.0 + 3.0 * column as f64, y)
                .two_pad(&format!("N{k}"), &format!("N{}", (k * 5 + 3) % 22)),
        );
    }
    let mut model = model_of(&board(Shape::Rect(0.0, 0.0, 22.0, 12.0), &parts), &[]);
    model.clearances.part_gap_mm = 0.3;
    model.clearances.edge_mm = 0.5;
    model
}

#[test]
fn a_converged_board_has_no_new_hard_failure() {
    let model = crowded();
    assert!(movable_hard_failures(&model, &model.text).is_empty());
    for seed in [1, 2] {
        let (candidate, trace) = run(&model, None, seed, &quick(0));
        assert!(trace.converged(), "seed {seed}: {trace:?}");
        let text = board_of(&model, &candidate);
        let failures = movable_hard_failures(&model, &text);
        assert!(failures.is_empty(), "seed {seed}: {failures:?}");
        // Every moved part keeps C's gap on the written board too.
        let cost = cost_of(&model, None);
        let state = rebuilt(&cost, &text);
        for &p in &cost.movable {
            let mv = Move {
                parts: vec![(p, state.centres[p], state.turns[p])],
            };
            let moved = state.centres[p] != cost.read_centres[p] || state.turns[p] != 0;
            assert!(
                !moved || cost.legal(&state, &mv),
                "seed {seed}: {} is legal where it ended",
                model.references[p]
            );
        }
        assert!(candidate.placements.len() > 10, "the run moved parts");
    }
}

/// The demo with a part of every hold reason: the connector J1, the mounting hole H1, a
/// part with no netted pad (TP1), a KiCad lock (L1) and a caller's part (C8).
fn every_hold_reason() -> BoardModel {
    let mut parts = demo_parts();
    parts.push(Fp::new("TP1", 50.0, 6.0).pad(pad("1", 0.0, 0.0, None)));
    parts.push(Fp::new("L1", 50.0, 34.0).locked().two_pad("SIG1", "SIG2"));
    model_of(&board(OUTLINE, &parts), &["C8"])
}

#[test]
fn family_c_never_lists_a_held_part() {
    let model = every_hold_reason();
    let reasons: BTreeSet<crate::place_held::HoldReason> = model
        .held
        .held
        .values()
        .flat_map(|part| part.reasons.iter().copied())
        .collect();
    assert_eq!(reasons.len(), 4, "{reasons:?}");
    for held in ["J1", "H1", "TP1", "L1", "C8"] {
        assert!(model.held.held.contains_key(held), "{held}");
    }
    for seed in [1, 2] {
        let (candidate, _) = run(&model, None, seed, &quick(0));
        assert!(!candidate.placements.is_empty());
        for placement in &candidate.placements {
            assert!(
                model.held.movable.contains(&placement.reference),
                "seed {seed}: {} is held",
                placement.reference
            );
        }
        let text = board_of(&model, &candidate);
        assert_eq!(
            check_held(&model.held, &parse(&text)),
            Ok(()),
            "seed {seed}"
        );
    }
    // Inside the search too: `placements_from` never writes a held part, so the board
    // alone would not show one that moved in the search's own state.
    let cost = CostModel::build(&model, None, &quick(0)).unwrap();
    let (mut state, mut rng, mut evaluations) = annealed(&cost, 1);
    descend(&cost, &mut state, &mut rng, &mut evaluations);
    for p in (0..model.parts.len()).filter(|&p| !model.movable[p]) {
        assert_eq!(
            (state.centres[p], state.turns[p]),
            (cost.read_centres[p], 0),
            "{} stayed where and as it was read",
            model.references[p]
        );
    }
}

#[test]
fn no_part_changes_side_in_a_whole_run() {
    let model = demo_with_pairs();
    let (candidate, _) = run(&model, Some(&demo_hoods()), 3, &quick(0));
    let text = board_of(&model, &candidate);
    let scan = konnect_sexp::board::footprint_courtyards(&parse(&text));
    for item in &scan.items {
        let reference = item.reference.as_deref().unwrap();
        let p = index_of(&model, reference);
        assert_eq!(item.layer_side, model.parts[p].layer_side, "{reference}");
    }
    let r6 = index_of(&model, "R6");
    assert_eq!(model.parts[r6].layer_side, konnect_sexp::board::Side::Back);
    assert!(
        candidate.placements.iter().any(|p| p.reference == "R6"),
        "the back part moved"
    );
}

#[test]
fn a_part_read_off_the_quarter_turns_is_never_turned_in_a_whole_run() {
    let mut parts = vec![Fp::new("R1", 20.0, 20.0).rot(45.0).two_pad("A", "B")];
    for k in 2..=7 {
        let x = 8.0 + 6.0 * k as f64;
        parts.push(Fp::new(&format!("R{k}"), x, 30.0).two_pad(&format!("N{k}"), "A"));
    }
    parts.push(Fp::new("R8", 10.0, 10.0).two_pad("B", "N4"));
    let model = model_of(&board(OUTLINE, &parts), &[]);
    let mut turned = 0;
    for seed in [1, 2] {
        let (candidate, _) = run(&model, None, seed, &quick(0));
        for placement in &candidate.placements {
            let p = index_of(&model, &placement.reference);
            let read = model.parts[p].rotation_deg;
            if placement.reference == "R1" {
                assert_eq!(placement.rotation, read, "seed {seed}: R1 kept 45 degrees");
            } else if placement.rotation != read {
                turned += 1;
            }
        }
    }
    // The control, in the same runs: a part read at 0 degrees does turn.
    assert!(turned > 0, "a part read at 0 degrees turned");
}

#[test]
fn with_turn_parts_off_every_part_keeps_its_rotation() {
    let model = demo_with_pairs();
    let off = ConvergeParams {
        turn_parts: false,
        ..quick(0)
    };
    let (candidate, trace) = run(&model, Some(&demo_hoods()), 4, &off);
    assert!(trace.converged());
    assert!(!candidate.placements.is_empty());
    for placement in &candidate.placements {
        let p = index_of(&model, &placement.reference);
        assert_eq!(
            placement.rotation, model.parts[p].rotation_deg,
            "{}",
            placement.reference
        );
    }
    // With turns on, the same run turns parts.
    let (on, _) = run(&model, Some(&demo_hoods()), 4, &quick(0));
    assert!(on
        .placements
        .iter()
        .any(|p| p.rotation != model.parts[index_of(&model, &p.reference)].rotation_deg));
}

/// `jobs` run on a pool of `threads` scoped threads from one atomic counter, each the
/// written board of its run, in job order.
fn pooled_runs(
    model: &BoardModel,
    hoods: &Neighbourhoods,
    jobs: &[(u64, ConvergeParams)],
    threads: usize,
) -> Vec<(String, Trace)> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<(String, Trace)>>> =
        jobs.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= jobs.len() {
                    break;
                }
                let (seed, params) = jobs[i];
                let (candidate, trace) = run(model, Some(hoods), seed, &params);
                *slots[i].lock().unwrap() = Some((board_of(model, &candidate), trace));
            });
        }
    });
    slots
        .into_iter()
        .map(|slot| slot.into_inner().unwrap().unwrap())
        .collect()
}

#[test]
fn the_same_run_gives_the_same_board_bytes() {
    let model = demo_with_pairs();
    let hoods = demo_hoods();
    let jobs: Vec<(u64, ConvergeParams)> = (1..=4).map(|seed| (seed, quick(0))).collect();
    let alone = pooled_runs(&model, &hoods, &jobs, 1);
    let again = pooled_runs(&model, &hoods, &jobs, 1);
    let wide = pooled_runs(&model, &hoods, &jobs, 8);
    assert_eq!(alone, again, "twice");
    assert_eq!(alone, wide, "from 1 and from 8 threads");
    // Another seed gives another board; so does level 1 from the same seed.
    assert_ne!(alone[0].0, alone[1].0, "seeds 1 and 2");
    let level_1 = pooled_runs(&model, &hoods, &[(1, quick(1))], 1);
    assert_ne!(alone[0].0, level_1[0].0, "levels 0 and 1");
    assert_eq!(level_1[0].1.level, 1);
    // The levels of one start do not repeat each other's stream: the first temperature
    // reads the stream and not the level, so it differs only because the stream was
    // advanced by the level.
    assert_ne!(
        alone[0].1.first_temperature, level_1[0].1.first_temperature,
        "the stream of level 1 is advanced"
    );
}

#[test]
fn a_run_of_generate_is_the_same_twice() {
    // `generate` with a recipe of family C, as `rerun` will call it: the same candidate,
    // its trace included.
    let mut model = demo_with_pairs();
    model.hoods = Some(demo_hoods());
    let start = recipes(&Matrix::bench())
        .into_iter()
        .find(|r| r.family == Family::Force && r.seed == 2)
        .unwrap();
    let recipe = Recipe {
        family: Family::Converge,
        seed: start.seed,
        params: Params::Converge(ConvergeRecipe {
            start: Box::new(start),
            params: quick(0),
        }),
    };
    let first = generate(&model, &recipe).expect("a run");
    let second = generate(&model, &recipe).expect("a run");
    assert_eq!(first, second);
    let trace = first.trace.as_ref().expect("a trace");
    assert_eq!(first.converged, Some(trace.converged()));
    assert_eq!(first.iterations_run, Some(trace.stages));
    assert!(trace.hoods_sha256.is_some());
}

#[test]
fn the_trace_holds_the_start_as_given_and_the_end_as_written() {
    // An F candidate as the start, on the demo with a part gap of 0.7 mm (above the
    // 0.5 mm grid step): F legalizes with courtyards that may touch, so C must move some
    // of its parts first.
    let mut model = demo_with_pairs();
    model.clearances.part_gap_mm = 0.7;
    let hoods = demo_hoods();
    let recipe = recipes(&Matrix::bench())
        .into_iter()
        .find(|r| r.family == Family::Force && r.seed == 3)
        .unwrap();
    let start = generate(&model, &recipe)
        .expect("an F candidate")
        .placements;
    let (candidate, trace) = converge(&model, Some(&hoods), &start, 3, &quick(0)).unwrap();

    // The start as given, before it was made legal, under the run's R.
    let cost = CostModel::build(&model, Some(&hoods), &quick(0))
        .unwrap()
        .with_requirement(trace.requirement_mm);
    let (centres, turns) = centres_of(&cost, &start).unwrap();
    let given = cost.state(centres, turns);
    assert_eq!(trace.start, cost.terms(&given));
    let legal = state_from(&cost, &start).unwrap();
    assert_ne!(legal.centres, given.centres, "C moved parts of the start");
    assert!(trace.requirement_mm >= 1000.0);

    // The end as written: the screen's own airwire length, crossings, pair and connector
    // counts of the written board.
    let text = board_of(&model, &candidate);
    let scored = evaluated(&model, &text);
    let metrics = &scored["routing_metrics"];
    let length = metrics["wirelength_mm"].as_f64().unwrap();
    assert!(
        (trace.end.airwire_mm - length).abs() <= 1e-6,
        "{} against {length}",
        trace.end.airwire_mm
    );
    assert_eq!(
        trace.end.crossings as u64,
        metrics["proper_crossings"].as_u64().unwrap()
    );
    assert_eq!(
        trace.end.pair_violations,
        pair_violations(&parse(&text), &model.intent.pairs)
    );
    assert_eq!(
        trace.end.connector_violations,
        screen(&scored, &model.held, &[], 0, None)
            .unwrap()
            .connector_violations
    );
    assert_eq!(trace.end, cost.terms(&rebuilt(&cost, &text)));
    // The curve ends at the end's cost, to the rounding of the written anchors.
    let last = *trace.best_by_step.last().unwrap();
    assert!(
        (last - trace.end.cost).abs() < 1e-2,
        "{last} against {}",
        trace.end.cost
    );
}

/// Three fixtures for the stop rules: the demo with pairs and neighbourhoods, the stock
/// rectangle and the rounded board.
fn three_fixtures() -> Vec<(&'static str, BoardModel, Option<Neighbourhoods>)> {
    vec![
        ("demo", demo_with_pairs(), Some(demo_hoods())),
        (
            "rectangle",
            model_of(&std::fs::read_to_string(RECTANGLE).unwrap(), &[]),
            None,
        ),
        (
            "rounded",
            model_of(&std::fs::read_to_string(ROUNDED).unwrap(), &[]),
            None,
        ),
    ]
}

#[test]
fn a_run_stops_by_its_rules_on_the_fixtures() {
    for (name, model, hoods) in three_fixtures() {
        for level in [0, 1] {
            let (candidate, trace) = run(&model, hoods.as_ref(), 5, &quick(level));
            let what = format!("{name} at level {level}");
            assert_eq!(trace.anneal_stop, AnnealStop::Quiet, "{what}");
            assert!(
                matches!(
                    trace.descent_stop,
                    DescentStop::FixedPoint | DescentStop::Quiet
                ),
                "{what}: {:?}",
                trace.descent_stop
            );
            assert!(
                trace.converged() && candidate.converged == Some(true),
                "{what}"
            );
            assert!(trace.stages < quick(level).max_stages_at_level(), "{what}");
            // The curve: one best cost per stage, then one per sweep, never rising.
            assert_eq!(
                trace.best_by_step.len(),
                trace.stages + trace.sweeps,
                "{what}"
            );
            assert!(
                trace.best_by_step[..trace.stages]
                    .windows(2)
                    .all(|w| w[1] <= w[0] + 1e-6),
                "{what}: annealing's best rose"
            );
            assert!(
                trace.best_by_step[trace.stages..]
                    .windows(2)
                    .all(|w| w[1] <= w[0] + 1e-6),
                "{what}: descent rose"
            );
            assert!(trace.end.cost <= trace.start.cost + 1e-6, "{what}");
            assert!(
                trace.evaluations > 0 && trace.first_temperature > 0.0,
                "{what}"
            );
        }
    }
    // The control: a guard of 3 stages ends annealing on the guard, and the run is
    // recorded as not converged.
    let (_, model, hoods) = three_fixtures().remove(0);
    let guarded = ConvergeParams {
        max_stages: 3,
        ..quick(0)
    };
    let (candidate, trace) = run(&model, hoods.as_ref(), 5, &guarded);
    assert_eq!((trace.stages, trace.anneal_stop), (3, AnnealStop::Guard));
    assert!(!trace.converged());
    assert_eq!(candidate.converged, Some(false));
}

#[test]
fn the_first_temperature_trials_do_not_move_the_start() {
    let model = demo_with_pairs();
    let cost = cost_of(&model, None);
    let state = state_from(&cost, &[]).unwrap();
    let before = state.clone();
    let mut rng = SplitMix64(7);
    let t = first_temperature(&cost, &state, &mut rng);
    assert_eq!(state, before, "the start is as it was");
    assert!(t.is_finite() && t > 0.0, "{t}");
    // The trials walk on: each from where the one before left the parts. Replayed here
    // by hand, they give the same temperature and move parts on the copy.
    let mut rng = SplitMix64(7);
    let mut copy = state.clone();
    let (mut ups, mut sum, mut made) = (0, 0.0, 0);
    for _ in 0..400 {
        let p = cost.movable[rng.below(cost.movable.len() as u64) as usize];
        let (dx, dy) = (grid_shift(&mut rng, 10.0), grid_shift(&mut rng, 10.0));
        let at = copy.centres[p];
        let mv = Move {
            parts: vec![(p, (at.0 + dx, at.1 + dy), copy.turns[p])],
        };
        if cost.legal(&copy, &mv) {
            let delta = cost.delta(&copy, &mv);
            if delta.cost > 0.0 {
                ups += 1;
                sum += delta.cost;
            }
            cost.apply(&mut copy, &mv, delta);
            made += 1;
        }
    }
    assert!(made > 100 && ups > 10, "{made} trials made, {ups} uphill");
    assert_ne!(
        copy.centres, state.centres,
        "the trials moved parts on the copy"
    );
    assert!((t - sum / ups as f64 / std::f64::consts::LN_2).abs() < 1e-9);
}

/// U1, its decoupling capacitor C1 beside it (locked), and 60 locked single-pad parts
/// far to the right, each on a net of its own with one of U1's pins: the soft cost of
/// the start is about 20 m, and U1 beside the far parts saves almost all of it.
fn far_pins() -> (BoardModel, usize) {
    let mut u1 = Fp::new("U1", 20.0, 50.0)
        .court(2.0, 15.5)
        .pad(pad("61", -1.5, 0.0, Some("VCC")));
    let mut parts = Vec::new();
    for k in 1..=60 {
        let y = -14.75 + 0.5 * (k - 1) as f64;
        let net = format!("S{k}");
        u1 = u1.pad(pad(&k.to_string(), 1.0, y, Some(&net)));
        parts.push(
            Fp::new(&format!("R{k}"), 350.0, 50.0 + y)
                .locked()
                .court(0.2, 0.2)
                .pad(pad("1", 0.0, 0.0, Some(&net))),
        );
    }
    parts.push(u1);
    parts.push(Fp::new("C1", 16.0, 50.0).locked().two_pad("VCC", "GND"));
    let text = board(Shape::Rect(0.0, 0.0, 400.0, 100.0), &parts);
    let set = held_set(&text, &BTreeSet::new(), &HoldRules::default()).unwrap();
    let pairs = vec![DecouplingPair {
        cap: "C1".to_string(),
        cap_pad: "1".to_string(),
        ic: "U1".to_string(),
        ic_pad: "61".to_string(),
        net: "VCC".to_string(),
        limit_mm: 5.0,
        basis: "test".to_string(),
    }];
    let model = board_model(&text, set, Intent::new(Vec::new(), pairs)).unwrap();
    let u1 = index_of(&model, "U1");
    (model, u1)
}

#[test]
fn a_broken_requirement_costs_at_least_the_starts_soft_cost() {
    let (model, u1) = far_pins();
    assert_eq!(model.movable.iter().filter(|&&m| m).count(), 1);
    let fixed = cost_of(&model, None);
    let start = state_from(&fixed, &[]).unwrap();
    let soft = fixed.terms(&start).soft;
    assert!(soft > 10_000.0, "the start's soft cost is {soft}");
    assert_eq!(fixed.terms(&start).pair_violations, 0);
    // U1 beside the far parts: the pair breaks, the airwire falls by about 19 m.
    let trade = Move {
        parts: vec![(u1, (340.0, 50.0), 0)],
    };
    assert!(fixed.legal(&start, &trade));

    // The run's R is the start's soft cost, and under it the trade costs more than it
    // saves.
    let short = ConvergeParams {
        stage_per_part: 1,
        max_stages: 1,
        max_sweeps: 1,
        ..converge_params()
    };
    let (_, trace) = converge(&model, None, &[], 1, &short).unwrap();
    assert_eq!(trace.requirement_mm, soft);
    let followed = cost_of(&model, None).with_requirement(trace.requirement_mm);
    let start = followed.state(start.centres.clone(), start.turns.clone());
    let delta = followed.delta(&start, &trade);
    assert!(delta.cost > 0.0, "under R = {soft}: {}", delta.cost);

    // The contrast on the same fixture: under a fixed 1000 mm the move trades the pair
    // for length.
    let start = fixed.state(start.centres.clone(), start.turns.clone());
    let delta = fixed.delta(&start, &trade);
    assert!(delta.cost < 0.0, "under R = 1000: {}", delta.cost);

    // R is never below the parameter: on the demo (soft cost far below 1000 mm) it is
    // 1000 mm.
    let (_, trace) = converge(&demo_with_pairs(), None, &[], 1, &short).unwrap();
    assert_eq!(trace.requirement_mm, 1000.0);
}

/// The state a run reaches before descent: the start, its first temperature and
/// annealing, as `converge` runs them.
fn annealed(cost: &CostModel<'_>, seed: u64) -> (State, SplitMix64, u64) {
    let mut state = state_from(cost, &[]).unwrap();
    let mut rng = SplitMix64(seed);
    let t = first_temperature(cost, &state, &mut rng);
    let mut evaluations = 0;
    anneal(cost, &mut state, t, &mut rng, &mut evaluations);
    (state, rng, evaluations)
}

/// The lowest cost change of any legal move of descent's set on `state`.
fn best_single_move(cost: &CostModel<'_>, state: &State) -> f64 {
    let mut best = f64::INFINITY;
    for &p in &cost.movable {
        for mv in cost.descent_moves(state, p) {
            if cost.legal(state, &mv) {
                best = best.min(cost.delta(state, &mv).cost);
            }
        }
    }
    best
}

#[test]
fn after_descent_no_single_move_lowers_the_cost() {
    let model = demo_with_pairs();
    let cost = CostModel::build(&model, Some(&demo_hoods()), &quick(0)).unwrap();
    let mut fixed_points = 0;
    for seed in [1, 2, 3] {
        let (mut state, mut rng, mut evaluations) = annealed(&cost, seed);
        // The control: annealing alone leaves a single move that lowers the cost.
        let after_anneal = best_single_move(&cost, &state);
        assert!(after_anneal < -1e-9, "seed {seed}: {after_anneal}");
        let (_, stop, _) = descend(&cost, &mut state, &mut rng, &mut evaluations);
        if stop == DescentStop::FixedPoint {
            fixed_points += 1;
            let after = best_single_move(&cost, &state);
            assert!(
                after >= -1e-9,
                "seed {seed}: a move lowers the cost by {after}"
            );
        }
    }
    assert!(fixed_points > 0, "descent reached a fixed point");
}

// ─── Making room ─────────────────────────────────────────────────────────────

/// U1 at (30, 20) boxed in by 24 movable 2.4 mm parts B1 to B24 (0.1 to 0.35 mm apart),
/// and its capacitor C1 far off at `cap_at`: every cell within C1's 3 mm limit of U1's
/// pad 1 is taken. `locked` locks the blockers it names. The blockers sit on a ground
/// rail with a pour, and the board's labels are on `F.Fab`, not on silk: nothing in the
/// cost moves a blocker, so no single move frees a cell for C1. U1 is locked, so it
/// cannot go to C1 instead.
fn boxed(locked: &[usize], outline: Shape, cap_at: (f64, f64), extra: Vec<Fp>) -> BoardModel {
    let offsets = [-5.25, -2.75, 0.0, 2.75, 5.25];
    let mut parts = vec![
        Fp::new("U1", 30.0, 20.0)
            .locked()
            .court(1.5, 1.5)
            .pad(pad("1", -1.0, 0.0, Some("VCC")))
            .pad(pad("2", 1.0, 0.0, Some("GND"))),
        Fp::new("C1", cap_at.0, cap_at.1).two_pad("VCC", "GND"),
    ];
    let mut n = 0;
    for &dx in &offsets {
        for &dy in &offsets {
            if dx == 0.0 && dy == 0.0 {
                continue;
            }
            n += 1;
            let mut blocker = Fp::new(&format!("B{n}"), 30.0 + dx, 20.0 + dy)
                .court(1.2, 1.2)
                .pad(pad("1", 0.0, 0.0, Some("GND")));
            if locked.contains(&n) {
                blocker = blocker.locked();
            }
            parts.push(blocker);
        }
    }
    parts.extend(extra);
    let text = board(outline, &parts).replace(
        "(at 0 -1.5 0) (layer \"F.SilkS\")",
        "(at 0 -1.5 0) (layer \"F.Fab\")",
    );
    let zone = "(zone (net \"GND\") (layer \"F.Cu\") (uuid \"77777777-0000-4000-8000-000000000002\") (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1) (xy 0 1))))\n)\n";
    let text = format!("{}{zone}", text.trim_end().strip_suffix(')').unwrap());
    let set = held_set(&text, &BTreeSet::new(), &HoldRules::default()).unwrap();
    let pairs = vec![DecouplingPair {
        cap: "C1".to_string(),
        cap_pad: "1".to_string(),
        ic: "U1".to_string(),
        ic_pad: "1".to_string(),
        net: "VCC".to_string(),
        limit_mm: 3.0,
        basis: "test".to_string(),
    }];
    board_model(&text, set, Intent::new(Vec::new(), pairs)).unwrap()
}

/// Two locked fillers that leave a slot 1.6 mm tall around (50, 34): C1 fits there, a
/// blocker does not, so no swap of C1 with a blocker is legal.
fn slot_at_50_34() -> Vec<Fp> {
    vec![
        Fp::new("F1", 50.0, 32.2)
            .locked()
            .court(3.0, 1.0)
            .pad(pad("1", 0.0, 0.0, Some("GND"))),
        Fp::new("F2", 50.0, 35.8)
            .locked()
            .court(3.0, 1.0)
            .pad(pad("1", 0.0, 0.0, Some("GND"))),
    ]
}

fn boxed_in() -> BoardModel {
    boxed(&[], OUTLINE, (50.0, 34.0), slot_at_50_34())
}

/// Every part of `state` that moved is legal where it stands.
fn assert_nothing_overlaps(cost: &CostModel<'_>, state: &State, what: &str) {
    for &p in &cost.movable {
        if state.centres[p] == cost.read_centres[p] && state.turns[p] == 0 {
            continue;
        }
        let mv = Move {
            parts: vec![(p, state.centres[p], state.turns[p])],
        };
        assert!(
            cost.legal(state, &mv),
            "{what}: {} overlaps",
            cost.model.references[p]
        );
    }
}

#[test]
fn a_capacitor_boxed_out_of_range_gets_a_cell_by_its_ic() {
    let model = boxed_in();
    let cost = cost_of(&model, None);
    let c1 = index_of(&model, "C1");
    assert!(cost.movable.len() == 25 && cost.part_nets[c1].len() == 1);
    let start = state_from(&cost, &[]).unwrap();
    assert_eq!(cost.terms(&start).pair_violations, 1);
    // No single move of C1 brings it within its limit: every cell there is taken.
    let mut rng = SplitMix64(11);
    for _ in 0..20_000 {
        if let Some(mv) = cost.propose(&start, c1, 30.0, &mut rng) {
            if cost.legal(&start, &mv) && mv.parts.iter().any(|&(p, ..)| p == c1) {
                let mut after = start.clone();
                let delta = cost.delta(&after, &mv);
                cost.apply(&mut after, &mv, delta);
                assert_eq!(cost.terms(&after).pair_violations, 1, "{mv:?}");
            }
        }
    }

    // `repair` makes room: the pair within its limit, nothing overlapping.
    let mut state = start.clone();
    let mut evaluations = 0;
    assert_eq!(repair(&cost, &mut state, &mut evaluations), (1, 1));
    assert_eq!(cost.terms(&state).pair_violations, 0);
    assert_same_caches(
        &state,
        &cost.state(state.centres.clone(), state.turns.clone()),
        "after repair",
    );
    assert_nothing_overlaps(&cost, &state, "after repair");
    let written = written(&model, &state);
    assert!(movable_hard_failures(&model, &written).is_empty());
    assert_eq!(pair_violations(&parse(&written), &model.intent.pairs), 0);

    // Descent alone, from the boxed start: it runs `repair` first and ends with the pair
    // within its limit (the control: without `repair` no move of its set frees a cell).
    let mut state = start.clone();
    let (_, stop, _) = descend(&cost, &mut state, &mut SplitMix64(3), &mut evaluations);
    assert_eq!(cost.terms(&state).pair_violations, 0, "descent: {stop:?}");
    assert_nothing_overlaps(&cost, &state, "after descent");

    // A whole run ends with the pair within its limit.
    let (candidate, trace) = run(&model, None, 1, &quick(0));
    assert_eq!(trace.end.pair_violations, 0, "{trace:?}");
    assert!(trace.repairs_tried >= trace.repairs_made);
    let text = board_of(&model, &candidate);
    assert!(movable_hard_failures(&model, &text).is_empty());
}

#[test]
fn making_room_never_moves_a_held_part() {
    // The six blockers left of U1 locked: C1 must go elsewhere, and only movable
    // blockers make room for it.
    let left = [6, 7, 8, 9, 10, 11];
    let model = boxed(&left, OUTLINE, (50.0, 34.0), slot_at_50_34());
    let cost = cost_of(&model, None);
    for k in left {
        assert!(!model.movable[index_of(&model, &format!("B{k}"))], "B{k}");
    }
    let state = state_from(&cost, &[]).unwrap();
    let mv = cost
        .make_room(&state, 0)
        .expect("room on a side of movable parts");
    assert!(mv.parts.len() >= 2, "{mv:?}");
    for &(p, ..) in &mv.parts {
        assert!(model.movable[p], "{} is held", model.references[p]);
    }
    assert!(cost.legal(&state, &mv));
    // Every blocker held: no cell within the limit is legal against the held parts.
    let all: Vec<usize> = (1..=24).collect();
    let model = boxed(&all, OUTLINE, (50.0, 34.0), slot_at_50_34());
    let cost = cost_of(&model, None);
    let state = state_from(&cost, &[]).unwrap();
    assert_eq!(cost.make_room(&state, 0), None);
}

#[test]
fn making_room_is_refused_when_a_displaced_part_finds_no_cell() {
    // The board ends 0.05 mm outside the blockers on the left and 1.05 mm above and below
    // them; right of them two locked fillers leave C1 a slot 2.2 mm tall. A displaced
    // 2.4 mm blocker finds no cell anywhere.
    let fillers = || {
        vec![
            Fp::new("F1", 39.25, 15.75)
                .locked()
                .court(2.65, 3.15)
                .pad(pad("1", 0.0, 0.0, Some("GND"))),
            Fp::new("F2", 39.25, 24.25)
                .locked()
                .court(2.65, 3.15)
                .pad(pad("1", 0.0, 0.0, Some("GND"))),
        ]
    };
    let tight = boxed(
        &[],
        Shape::Rect(23.5, 12.5, 42.0, 27.5),
        (38.0, 20.0),
        fillers(),
    );
    let cost = cost_of(&tight, None);
    let state = state_from(&cost, &[]).unwrap();
    assert_eq!(cost.terms(&state).pair_violations, 1);
    assert_eq!(cost.make_room(&state, 0), None);
    // The same board with a strip below the blockers: the displaced blocker finds a cell
    // there, and the move is legal as a whole.
    let roomy = boxed(
        &[],
        Shape::Rect(23.5, 9.0, 42.0, 27.5),
        (38.0, 20.0),
        fillers(),
    );
    let cost = cost_of(&roomy, None);
    let state = state_from(&cost, &[]).unwrap();
    let mv = cost.make_room(&state, 0).expect("room in the strip");
    assert!(mv.parts.len() >= 2 && cost.legal(&state, &mv), "{mv:?}");
}

// ─── The cost model's run-level halves ───────────────────────────────────────

#[test]
fn an_unmeasured_pair_is_a_constant_violation_and_the_run_still_ends() {
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
    let (candidate, trace) = run(&model, None, 1, &quick(0));
    assert!(trace.converged(), "{trace:?}");
    assert_eq!(trace.start.pair_violations, 1);
    assert_eq!(trace.end.pair_violations, 1);
    let text = board_of(&model, &candidate);
    assert_eq!(pair_violations(&parse(&text), &pairs), 1);
    assert!((trace.end.cost - trace.end.soft - trace.requirement_mm).abs() < 1e-9);
}

/// The demo with an indicator LED D1 on the two rails only, both rails poured: nothing
/// in the nets pulls D1 anywhere.
fn led_on_rails() -> BoardModel {
    let mut parts = demo_parts();
    parts.push(Fp::new("D1", 6.0, 20.0).two_pad("VCC", "GND"));
    let text = board(OUTLINE, &parts);
    let zones: String = ["VCC", "GND"]
        .iter()
        .enumerate()
        .map(|(k, net)| {
            format!(
                "(zone (net \"{net}\") (layer \"F.Cu\") (uuid \"77777777-0000-4000-8000-00000000001{k}\") (polygon (pts (xy 0 0) (xy 60 0) (xy 60 40) (xy 0 40))))\n"
            )
        })
        .collect();
    let text = format!("{}{zones})\n", text.trim_end().strip_suffix(')').unwrap());
    model_of(&text, &[])
}

#[test]
fn a_part_on_rails_only_ends_by_its_schematic_neighbours() {
    let model = led_on_rails();
    let (d1, j1) = (index_of(&model, "D1"), index_of(&model, "J1"));
    let cost = cost_of(&model, None);
    assert!(cost.part_nets[d1].is_empty(), "D1 is on poured rails only");
    let hoods = hoods_of(&[("D1", &["J1"]), ("R1", &["R2"])]);
    let distance = |candidate: &Candidate| {
        let text = board_of(&model, candidate);
        let state = rebuilt(&cost, &text);
        let (a, b) = (state.centres[d1], state.centres[j1]);
        (a.0 - b.0).hypot(a.1 - b.1)
    };
    let (pulled, trace) = run(&model, Some(&hoods), 1, &quick(0));
    assert!(trace.converged() && trace.end.neighbour_mm.is_some());
    let near = distance(&pulled);
    // The control: without the term the same part ends farther away.
    let off = ConvergeParams {
        neighbour_weight: 0.0,
        ..quick(0)
    };
    let (loose, _) = run(&model, Some(&hoods), 1, &off);
    let far = distance(&loose);
    assert!(near < far, "with the term {near} mm, without {far} mm");
    assert!(near < 6.0, "D1 ends by J1: {near} mm");
    // With no schematic the run still ends, and the trace has no neighbour term.
    let (_, trace) = run(&model, None, 1, &quick(0));
    assert!(trace.converged());
    assert_eq!(trace.end.neighbour_mm, None);
    assert_eq!(trace.hoods_sha256, None);
}

#[test]
fn the_hoods_hash_is_of_their_sorted_json() {
    let hoods = demo_hoods();
    let text = r#"{"alone":0,"measured":5,"neighbours":{"C1":["U1","C2"],"C5":["R4"],"R1":["R2","R3","C1"],"R6":["J1"],"U1":["C1"]},"unmatched":0}"#;
    assert_eq!(
        hoods_sha256(&hoods),
        format!("{:x}", Sha256::digest(text.as_bytes()))
    );
    let mut other = hoods.clone();
    other.alone = 1;
    assert_ne!(hoods_sha256(&hoods), hoods_sha256(&other));
}

// ─── Speed (not a guard) ─────────────────────────────────────────────────────

/// One run of `converge` on the demo with decoupling pairs and neighbourhoods, from the
/// board as read; its trace and wall time.
fn timed_run(params: &ConvergeParams, seed: u64) -> (Trace, std::time::Duration) {
    let model = demo_with_pairs();
    let hoods = demo_hoods();
    let started = std::time::Instant::now();
    let (_, trace) = converge(&model, Some(&hoods), &[], seed, params).expect("a run");
    (trace, started.elapsed())
}

/// The cost of whole runs on the demo at levels 0 and 1, with the measured parameters,
/// for the measurement. Run by hand in a release build:
/// `cargo test --release -p konnect-core --locked --lib place_converge::tests::time_a_run_on_the_demo -- --exact --ignored --nocapture`.
#[test]
#[ignore]
fn time_a_run_on_the_demo() {
    for level in [0, 1] {
        for seed in 1..=3 {
            let (trace, took) = timed_run(&ConvergeParams::measured(level), seed);
            println!(
                "level {level} seed {seed}: {} stages ({:?}), {} sweeps ({:?}), {} evaluations, \
                 repairs {} made of {} tried, first T {:.3}, R {:.1}, start cost {:.3} (airwire {:.2}, \
                 crossings {}, pairs broken {}), end cost {:.3} (airwire {:.2}, crossings {}, pairs \
                 broken {}, labels {}), {:.2} s, {:.2} µs per evaluation",
                trace.stages,
                trace.anneal_stop,
                trace.sweeps,
                trace.descent_stop,
                trace.evaluations,
                trace.repairs_made,
                trace.repairs_tried,
                trace.first_temperature,
                trace.requirement_mm,
                trace.start.cost,
                trace.start.airwire_mm,
                trace.start.crossings,
                trace.start.pair_violations,
                trace.end.cost,
                trace.end.airwire_mm,
                trace.end.crossings,
                trace.end.pair_violations,
                trace.end.label_conflicts,
                took.as_secs_f64(),
                took.as_secs_f64() * 1e6 / trace.evaluations as f64,
            );
        }
    }
}

/// A whole run at levels 0 and 1 on the 96-part board, with the measured parameters: what
/// to expect on a board of the owner's size. Run by hand in a release build:
/// `cargo test --release -p konnect-core --locked --lib place_converge::tests::time_a_run_on_96_parts -- --exact --ignored --nocapture`.
#[test]
#[ignore]
fn time_a_run_on_96_parts() {
    let model = ninety_six_parts();
    for level in [0, 1] {
        let started = std::time::Instant::now();
        let (_, trace) = run(&model, None, 1, &ConvergeParams::measured(level));
        let took = started.elapsed();
        println!(
            "96 parts, level {level}: {} stages ({:?}), {} sweeps ({:?}), {} evaluations, \
             repairs {} made of {} tried, first T {:.3}, start cost {:.3} (airwire {:.2}, \
             crossings {}), end cost {:.3} (airwire {:.2}, crossings {}, labels {}), {:.2} s, \
             {:.2} µs per evaluation",
            trace.stages,
            trace.anneal_stop,
            trace.sweeps,
            trace.descent_stop,
            trace.evaluations,
            trace.repairs_made,
            trace.repairs_tried,
            trace.first_temperature,
            trace.start.cost,
            trace.start.airwire_mm,
            trace.start.crossings,
            trace.end.cost,
            trace.end.airwire_mm,
            trace.end.crossings,
            trace.end.label_conflicts,
            took.as_secs_f64(),
            took.as_secs_f64() * 1e6 / trace.evaluations as f64,
        );
    }
}
