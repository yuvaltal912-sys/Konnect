//! Tests of `place_generators`, on small boards built by the shared test builder.

use super::*;
use crate::place_held::tests::{board, evaluate_text, pad, parse, Fp, Shape};
use crate::place_held::{self, decoupling_pairs, groups, held_set, HoldRules};
use crate::tools::pcb_components::placements_applied_to;
use konnect_sexp::board::Side;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

const OUTLINE: Shape = Shape::Rect(0.0, 0.0, 60.0, 40.0);

/// The model of `text`, as the search builds it: the held set, the intent from the
/// input board's own score, then the model.
fn model_of(text: &str, caller: &[&str]) -> BoardModel {
    let caller: BTreeSet<String> = caller.iter().map(|r| r.to_string()).collect();
    let set = held_set(text, &caller, &HoldRules::default()).expect("the held set");
    let held: Vec<&str> = set.held.keys().map(String::as_str).collect();
    let evaluated = evaluate_text(text, &held);
    let pairs = decoupling_pairs(&evaluated).unwrap();
    let found = groups(&parse(text), &set, &[]);
    board_model(text, set, Intent::new(found, pairs)).expect("the model")
}

/// A roomy board: U1, eight capacitors beside their pins, six resistors in a chain, one
/// of them turned and one on the back; a held connector and a mounting hole.
fn demo_parts() -> Vec<Fp> {
    let mut parts = vec![
        Fp::new("U1", 30.0, 20.0)
            .lib("Test:U")
            .court(3.0, 3.0)
            .pad(pad("1", -2.0, -1.0, Some("VCC")))
            .pad(pad("2", -2.0, 1.0, Some("GND")))
            .pad(pad("3", 2.0, 0.0, Some("SIG0"))),
        Fp::new("J1", 58.0, 20.0)
            .lib("Test:J")
            .court(1.5, 3.0)
            .pad(pad("1", 0.0, -1.0, Some("SIG0")))
            .pad(pad("2", 0.0, 1.0, Some("GND"))),
        Fp::new("H1", 4.0, 4.0)
            .lib("Test:H")
            .court(2.0, 2.0)
            .pad(pad("1", 0.0, 0.0, Some("GND"))),
    ];
    for k in 0..8 {
        parts.push(
            Fp::new(&format!("C{}", k + 1), 8.0 + 5.0 * k as f64, 8.0)
                .lib("Test:C")
                .value("100nF")
                .two_pad("VCC", "GND"),
        );
    }
    for k in 0..6 {
        let mut resistor = Fp::new(&format!("R{}", k + 1), 8.0 + 6.0 * k as f64, 32.0)
            .lib("Test:R")
            .two_pad(&format!("SIG{k}"), &format!("SIG{}", k + 1));
        if k == 4 {
            resistor = resistor.rot(90.0);
        }
        if k == 5 {
            resistor = resistor.back();
        }
        parts.push(resistor);
    }
    parts
}

fn demo_text() -> String {
    board(OUTLINE, &demo_parts())
}

fn demo_model() -> BoardModel {
    model_of(&demo_text(), &[])
}

fn bytes_of(model: &BoardModel, recipe: &Recipe) -> String {
    match generate(model, recipe) {
        Ok(candidate) => placements_applied_to(&model.text, &candidate.placements).unwrap(),
        Err(failure) => format!("failure {failure:?}"),
    }
}

fn force_recipe(seed: u64) -> Recipe {
    recipes(&Matrix::bench())
        .into_iter()
        .find(|r| r.family == Family::Force && r.seed == seed)
        .unwrap()
}

fn schematic_recipe(seed: u64, polished: bool) -> Recipe {
    recipes(&Matrix::bench())
        .into_iter()
        .find(|r| {
            r.family == Family::Schematic
                && r.seed == seed
                && matches!(&r.params, Params::Schematic(p) if p.polish.is_some() == polished)
        })
        .unwrap()
}

fn perturb_recipe(seed: u64, twelve: bool) -> Recipe {
    recipes(&Matrix::bench())
        .into_iter()
        .find(|r| {
            r.family == Family::Perturb
                && r.seed == seed
                && matches!(&r.params, Params::Perturb(p) if (p.moves == 12) == twelve)
        })
        .unwrap()
}

fn pooled(model: &BoardModel, recipes: &[&Recipe], threads: usize) -> Vec<String> {
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<String>>> = recipes.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= recipes.len() {
                    break;
                }
                *slots[i].lock().unwrap() = Some(bytes_of(model, recipes[i]));
            });
        }
    });
    slots
        .into_iter()
        .map(|slot| slot.into_inner().unwrap().unwrap())
        .collect()
}

#[test]
fn the_same_recipe_gives_the_same_board_bytes() {
    let model = demo_model();
    let all = recipes(&Matrix::bench());
    // A spread across the matrix: every 12th recipe is 24, three of each kind or more.
    let sample: Vec<&Recipe> = all.iter().step_by(12).collect();
    assert!(sample.iter().any(|r| r.family == Family::Force));
    assert!(sample.iter().any(|r| r.family == Family::Schematic));
    assert!(sample.iter().any(|r| r.family == Family::Perturb));
    let first: Vec<String> = sample.iter().map(|r| bytes_of(&model, r)).collect();
    let second: Vec<String> = sample.iter().map(|r| bytes_of(&model, r)).collect();
    assert_eq!(first, second, "twice in one thread");
    for threads in [1, 8] {
        assert_eq!(
            pooled(&model, &sample, threads),
            first,
            "a pool of {threads} thread(s)"
        );
    }
    assert!(
        first.iter().all(|text| !text.starts_with("failure")),
        "a roomy board gives a board for every recipe of the sample"
    );

    // The seed reaches the generator: seed 2 is not seed 1, in each family.
    for (one, two) in [
        (force_recipe(1), force_recipe(2)),
        (schematic_recipe(1, true), schematic_recipe(2, true)),
        (perturb_recipe(1, true), perturb_recipe(2, true)),
    ] {
        assert_ne!(
            bytes_of(&model, &one),
            bytes_of(&model, &two),
            "{:?}",
            one.family
        );
    }
    // The raw schematic grid orders its clusters by seed: some seed differs from seed 1.
    let raw: BTreeSet<String> = (1..=8)
        .map(|seed| bytes_of(&model, &schematic_recipe(seed, false)))
        .collect();
    assert!(
        raw.len() > 1,
        "the cluster order never changes with the seed"
    );
}

#[test]
fn generators_never_list_a_held_part() {
    // Every hold reason on one board: a connector, a mounting hole, a part with no
    // netted pad, a KiCad lock and a caller's part.
    let mut parts = demo_parts();
    parts.push(Fp::new("TP1", 50.0, 6.0).pad(pad("1", 0.0, 0.0, None)));
    parts.push(Fp::new("L1", 50.0, 34.0).locked().two_pad("SIG1", "SIG2"));
    let text = board(OUTLINE, &parts);
    let model = model_of(&text, &["C8"]);
    let reasons: BTreeSet<place_held::HoldReason> = model
        .held
        .held
        .values()
        .flat_map(|part| part.reasons.iter().copied())
        .collect();
    assert_eq!(reasons.len(), 4, "{reasons:?}");
    for held in ["J1", "H1", "TP1", "L1", "C8"] {
        assert!(model.held.held.contains_key(held), "{held}");
    }
    let recipes: Vec<Recipe> = vec![
        force_recipe(3),
        schematic_recipe(3, false),
        schematic_recipe(3, true),
        perturb_recipe(3, false),
        perturb_recipe(3, true),
    ];
    for recipe in &recipes {
        let candidate = generate(&model, recipe).expect("a roomy board");
        let listed: BTreeSet<&str> = candidate
            .placements
            .iter()
            .map(|p| p.reference.as_str())
            .collect();
        assert!(
            listed.iter().all(|r| model.held.movable.contains(*r)),
            "{:?} lists a held part: {listed:?}",
            recipe.family
        );
        assert_eq!(
            listed.len(),
            model.held.movable.len(),
            "every movable part is listed once"
        );
        let in_order: Vec<&str> = candidate
            .placements
            .iter()
            .map(|p| p.reference.as_str())
            .collect();
        let mut sorted = in_order.clone();
        sorted.sort_unstable();
        assert_eq!(in_order, sorted, "reference order");
        // The written board keeps every held part exactly.
        let written = placements_applied_to(&model.text, &candidate.placements).unwrap();
        assert_eq!(
            place_held::check_held(&model.held, &parse(&written)),
            Ok(())
        );
    }
}

#[test]
fn every_family_gives_a_legal_candidate_on_a_roomy_board() {
    let model = demo_model();
    for recipe in recipes(&Matrix::bench()).iter().step_by(7) {
        let candidate = generate(&model, recipe).unwrap_or_else(|e| panic!("{recipe:?}: {e:?}"));
        let written = placements_applied_to(&model.text, &candidate.placements).unwrap();
        let held: Vec<&str> = model.held.held.keys().map(String::as_str).collect();
        let evaluated = evaluate_text(&written, &held);
        assert_eq!(
            evaluated["hard_failures"].as_array().unwrap().len(),
            0,
            "{recipe:?}: {:?}",
            evaluated["hard_failures"]
        );
        if recipe.family == Family::Perturb {
            assert!(candidate.converged.is_none() && candidate.iterations_run.is_none());
        }
        if recipe.family == Family::Force {
            assert!(candidate.converged.is_some() && candidate.iterations_run.is_some());
        }
    }
}

#[test]
fn legalize_puts_every_part_inside_the_true_outline() {
    // Rounded corners of radius 10: a part in a corner of the bbox is outside the arc
    // although it is inside the rectangle that the force core works in.
    let corners = [
        [(1.2, 0.8), (3.7, 2.3)],
        [(58.8, 0.8), (56.3, 2.3)],
        [(1.2, 39.2), (3.7, 37.7)],
        [(58.8, 39.2), (56.3, 37.7)],
    ];
    let mut parts =
        vec![Fp::new("J1", 30.0, 20.0)
            .court(2.0, 2.0)
            .pad(pad("1", 0.0, 0.0, Some("A")))];
    for (k, pair) in corners.iter().enumerate() {
        for (m, (x, y)) in pair.iter().enumerate() {
            parts.push(Fp::new(&format!("R{}", 2 * k + m + 1), *x, *y).two_pad("A", "B"));
        }
    }
    let text = board(Shape::Rounded(0.0, 0.0, 60.0, 40.0, 10.0), &parts);
    let model = model_of(&text, &[]);
    let sizes = as_read_sizes(&model);
    let mut centres = as_read_centres(&model);
    let threshold = model.outline.containment_threshold_mm();
    let box_of = |i: usize, centres: &[(f64, f64)]| {
        let (w, h) = sizes[i];
        let (x, y) = centres[i];
        [
            (x - w / 2.0, y - h / 2.0),
            (x + w / 2.0, y - h / 2.0),
            (x + w / 2.0, y + h / 2.0),
            (x - w / 2.0, y + h / 2.0),
        ]
    };
    let depth =
        |i: usize, centres: &[(f64, f64)]| model.outline.overhang(&box_of(i, centres)).depth_mm;
    // The fixture tests the arcs: inside the bounds, outside the outline.
    let (bx0, by0, bx1, by1) = model.bounds;
    for i in 0..centres.len() {
        let corners = box_of(i, &centres);
        assert!(corners
            .iter()
            .all(|(x, y)| *x >= bx0 && *x <= bx1 && *y >= by0 && *y <= by1));
    }
    let overhanging = (0..centres.len())
        .filter(|&i| depth(i, &centres) > threshold)
        .count();
    assert!(
        overhanging >= 4,
        "only {overhanging} part(s) overhang an arc"
    );

    legalize(&model, &sizes, &mut centres).expect("a roomy board has room");
    for i in 0..centres.len() {
        assert!(
            depth(i, &centres) <= threshold,
            "{} still overhangs by {}",
            model.references[i],
            depth(i, &centres)
        );
    }
    // And no two parts on a side overlap.
    for i in 0..centres.len() {
        for j in (i + 1)..centres.len() {
            let ix = (sizes[i].0 + sizes[j].0) / 2.0 - (centres[i].0 - centres[j].0).abs();
            let iy = (sizes[i].1 + sizes[j].1) / 2.0 - (centres[i].1 - centres[j].1).abs();
            assert!(
                !(ix > 0.0 && iy > 0.0),
                "{} overlaps {}",
                model.references[i],
                model.references[j]
            );
        }
    }
    // The held connector did not move; the score agrees with the repair.
    assert_eq!(centres[0], as_read_centres(&model)[0]);
    let placements: Vec<IpcFootprintPlacement> = (0..centres.len())
        .filter(|&i| model.movable[i])
        .map(|i| IpcFootprintPlacement {
            reference: model.references[i].clone(),
            x: centres[i].0,
            y: centres[i].1,
            rotation: 0.0,
        })
        .collect();
    let written = placements_applied_to(&model.text, &placements).unwrap();
    let evaluated = evaluate_text(&written, &["J1"]);
    assert_eq!(
        evaluated["verdict"], "pass",
        "{:?}",
        evaluated["hard_failures"]
    );
}

#[test]
fn legalize_leaves_the_input_placement_alone() {
    // Two courtyards that touch exactly, as the input has them: nothing moves.
    let parts = vec![
        Fp::new("R1", 10.0, 10.0).two_pad("A", "B"),
        Fp::new("R2", 12.0, 10.0).two_pad("A", "B"),
        Fp::new("R3", 20.0, 10.0).two_pad("A", "B"),
    ];
    let model = model_of(&board(OUTLINE, &parts), &[]);
    let sizes = as_read_sizes(&model);
    let mut centres = as_read_centres(&model);
    let before = centres.clone();
    legalize(&model, &sizes, &mut centres).unwrap();
    assert_eq!(centres, before);
    // But a part that moved to touch a neighbour is moved off it.
    let mut moved = before.clone();
    moved[2] = (before[1].0 + 2.0, before[1].1);
    let touching = moved[2];
    legalize(&model, &sizes, &mut moved).unwrap();
    assert_ne!(moved[2], touching);
    assert_eq!(moved[0], before[0]);
    assert_eq!(moved[1], before[1]);
}

#[test]
fn legalize_says_no_room_instead_of_guessing() {
    // 7 parts of 4 x 3 mm on a 10 x 6 mm board: at most 2 or 4 can fit.
    let mut parts = Vec::new();
    for k in 0..7 {
        parts.push(
            Fp::new(&format!("R{}", k + 1), 5.0, 3.0)
                .court(2.0, 1.5)
                .two_pad("A", "B"),
        );
    }
    let text = board(Shape::Rect(0.0, 0.0, 10.0, 6.0), &parts);
    let model = model_of(&text, &[]);
    let sizes = as_read_sizes(&model);
    let mut centres = as_read_centres(&model);
    let error = legalize(&model, &sizes, &mut centres).unwrap_err();
    let GenerateFailure::NoRoom(names) = error else {
        panic!("expected NoRoom, got {error:?}");
    };
    assert!(!names.is_empty() && names.len() < 7, "{names:?}");
    assert!(names.iter().all(|n| model.references.contains(n)));
    // The parts that were placed are legal.
    let placed: Vec<usize> = (0..7)
        .filter(|&i| !names.contains(&model.references[i]))
        .collect();
    for &i in &placed {
        for &j in &placed {
            if i < j {
                let ix = (sizes[i].0 + sizes[j].0) / 2.0 - (centres[i].0 - centres[j].0).abs();
                let iy = (sizes[i].1 + sizes[j].1) / 2.0 - (centres[i].1 - centres[j].1).abs();
                assert!(!(ix > 0.0 && iy > 0.0));
            }
        }
    }
    // And generate reports it as a failure of the recipe, never a board.
    let recipe = perturb_recipe(1, false);
    assert!(matches!(
        generate(&model, &recipe),
        Err(GenerateFailure::NoRoom(_))
    ));
}

#[test]
fn the_schematic_grid_does_not_join_over_a_rail() {
    // GND on every part; two chains of parts, A and B, each with its own signal nets.
    let mut with_ground = Vec::new();
    for k in 0..5 {
        let chain = |name: &str, y: f64| {
            Fp::new(&format!("{name}{}", k + 1), 5.0 + 4.0 * k as f64, y)
                .two_pad(&format!("{name}{k}"), &format!("{name}{}", k + 1))
                .pad(pad("3", 0.0, 0.8, Some("GND")))
        };
        with_ground.push(chain("R", 8.0));
        with_ground.push(chain("C", 30.0));
    }
    let model = model_of(&board(OUTLINE, &with_ground), &[]);
    assert!(model.net_fanout["GND"] > 8);
    let limited = clusters(&model, 8);
    assert_eq!(limited.len(), 2, "{limited:?}");
    assert!(limited.iter().all(|c| c.len() == 5));
    // Without the filter the legacy behaviour: one cluster.
    assert_eq!(clusters(&model, usize::MAX).len(), 1);
}

#[test]
fn the_legacy_graph_is_rail_weight_three_without_a_limit() {
    let model = demo_model();
    let references: Vec<&str> = model.references.iter().map(String::as_str).collect();
    let legacy = force_core::springs(&model.index, &references, &net_weight);
    assert!(!legacy.is_empty());
    for rail_weight in [0.0, 0.5, 3.0, 9.0] {
        let params = ForceParams {
            rail_weight,
            fanout_limit: usize::MAX,
            pair_weight: 10.0,
            group_weight: 0.5,
            iterations: 10,
        };
        assert_eq!(
            spring_graph(&model, &params),
            legacy,
            "rail weight {rail_weight}"
        );
    }
    // With a limit, the rails (GND and VCC here, above 8 references) take the weight.
    let railed = ForceParams {
        rail_weight: 0.0,
        fanout_limit: 8,
        pair_weight: 10.0,
        group_weight: 0.5,
        iterations: 10,
    };
    assert_ne!(spring_graph(&model, &railed), legacy);
    let zero_rails: Springs = spring_graph(&model, &railed);
    // No capacitor is pulled to U1 any more: only the signal chain remains.
    let c1 = model.references.iter().position(|r| r == "C1").unwrap();
    let u1 = model.references.iter().position(|r| r == "U1").unwrap();
    assert!(!zero_rails.contains_key(&(c1.min(u1), c1.max(u1))));
    assert!(legacy.contains_key(&(c1.min(u1), c1.max(u1))));
}

#[test]
fn the_pairs_and_groups_become_springs() {
    let mut parts = demo_parts();
    // A capacitor beside U1's VCC pin, so that the score pairs them.
    parts[3] = Fp::new("C1", 25.8, 19.0)
        .lib("Test:C")
        .value("100nF")
        .two_pad("GND", "VCC");
    parts[0] = parts[0].clone().path("/aaaa/u1");
    for fp in parts.iter_mut() {
        if fp.reference.starts_with('C') {
            *fp = fp.clone().path(&format!("/aaaa/{}", fp.reference));
        }
        if fp.reference.starts_with('R') {
            *fp = fp.clone().path(&format!("/bbbb/{}", fp.reference));
        }
    }
    let model = model_of(&board(OUTLINE, &parts), &[]);
    assert!(!model.intent.pairs.is_empty());
    assert!(model.intent.groups.len() >= 2, "{:?}", model.intent.groups);
    let params = ForceParams {
        rail_weight: 0.5,
        fanout_limit: 8,
        pair_weight: 10.0,
        group_weight: 0.5,
        iterations: 10,
    };
    let extras = extras(&model, &params);
    assert_eq!(extras.pads.len(), model.intent.pairs.len());
    assert!(extras.pads.iter().all(|s| s.weight == 10.0 && s.a != s.b));
    assert_eq!(extras.groups.len(), model.intent.groups.len());
    // A weight of 0 adds no spring at all.
    let none = ForceParams {
        pair_weight: 0.0,
        group_weight: 0.0,
        ..params
    };
    assert_eq!(super::extras(&model, &none), FdExtras::default());
}

#[test]
fn turned_bbox_matches_the_scan_of_a_turned_footprint() {
    // An off-centre courtyard, so a wrong sense or a wrong pivot shows.
    for (rotation, back) in [(0.0, false), (90.0, false), (-90.0, false), (0.0, true)] {
        let mut part = Fp::new("U1", 20.0, 15.0)
            .court_rect(-1.0, -0.5, 3.0, 1.5)
            .two_pad("A", "B")
            .rot(rotation);
        if back {
            part = part.back();
        }
        let text = board(OUTLINE, &[part]);
        let before = konnect_sexp::board::footprint_courtyards(&parse(&text)).items[0].clone();
        for quarter_turns in 0..4 {
            let placement = IpcFootprintPlacement {
                reference: "U1".into(),
                x: 20.0,
                y: 15.0,
                rotation: rotation + 90.0 * f64::from(quarter_turns),
            };
            let turned = placements_applied_to(&text, &[placement]).unwrap();
            let after = konnect_sexp::board::footprint_courtyards(&parse(&turned)).items[0].clone();
            let expected = turned_bbox(before.bbox, before.at, quarter_turns);
            let (a, b) = (after.bbox, expected);
            assert!(
                (a.0 - b.0).abs() < 1e-9
                    && (a.1 - b.1).abs() < 1e-9
                    && (a.2 - b.2).abs() < 1e-9
                    && (a.3 - b.3).abs() < 1e-9,
                "rotation {rotation}, {quarter_turns} turn(s): the scan has {a:?}, turned_bbox {b:?}"
            );
        }
    }
    // Four quarter turns are the identity; a turn is exact on a cardinal box.
    let bbox = (1.0, 2.0, 5.0, 3.0);
    assert_eq!(turned_bbox(bbox, (2.0, 2.5), 4), bbox);
    assert_eq!(turned_bbox(bbox, (2.0, 2.5), 0), bbox);
    assert_eq!(turned_bbox(bbox, (2.0, 2.5), 1), (1.5, -0.5, 2.5, 3.5));
}

#[test]
fn perturb_swaps_only_same_footprints_and_turns_only_two_pad_parts() {
    // Two kinds of two-pad part, two 3-pad parts of one kind, and a lone 3-pad part.
    let mut parts = vec![
        Fp::new("J1", 58.0, 20.0).pad(pad("1", 0.0, 0.0, Some("A"))),
        Fp::new("U1", 10.0, 10.0)
            .lib("Test:U3")
            .court(2.0, 2.0)
            .pad(pad("1", -1.0, 0.0, Some("A")))
            .pad(pad("2", 0.0, 1.0, Some("B")))
            .pad(pad("3", 1.0, 0.0, Some("C"))),
        Fp::new("U2", 20.0, 10.0)
            .lib("Test:U3")
            .court(2.0, 2.0)
            .pad(pad("1", -1.0, 0.0, Some("A")))
            .pad(pad("2", 0.0, 1.0, Some("B")))
            .pad(pad("3", 1.0, 0.0, Some("C"))),
        Fp::new("U3", 30.0, 10.0)
            .lib("Test:U3b")
            .court(2.0, 2.0)
            .pad(pad("1", -1.0, 0.0, Some("A")))
            .pad(pad("2", 0.0, 1.0, Some("B")))
            .pad(pad("3", 1.0, 0.0, Some("C"))),
    ];
    for k in 0..4 {
        parts.push(
            Fp::new(&format!("R{}", k + 1), 8.0 + 6.0 * k as f64, 25.0)
                .lib("Test:R")
                .two_pad("A", "B"),
        );
        parts.push(
            Fp::new(&format!("C{}", k + 1), 8.0 + 6.0 * k as f64, 32.0)
                .lib("Test:C")
                .two_pad("A", "B"),
        );
    }
    parts.push(
        Fp::new("R9", 50.0, 32.0)
            .lib("Test:R")
            .two_pad("A", "B")
            .back(),
    );
    let model = model_of(&board(OUTLINE, &parts), &[]);
    let read = as_read_centres(&model);
    let read_rotation: Vec<f64> = model.parts.iter().map(|p| p.rotation_deg).collect();
    let params = PerturbParams {
        moves: 12,
        radius_mm: 1e-5,
        swaps: true,
        turns: true,
    };
    let (mut swapped, mut turned) = (0usize, 0usize);
    for seed in 1..=200u64 {
        let (centres, rotations) = perturb(&model, &params, &mut SplitMix64(seed));
        for i in 0..model.parts.len() {
            if !model.movable[i] {
                assert_eq!(centres[i], read[i]);
                assert_eq!(rotations[i], read_rotation[i]);
                continue;
            }
            // A part that moved more than a shift can carry it took another part's place,
            // and that part has its footprint, on its side.
            let (dx, dy) = (centres[i].0 - read[i].0, centres[i].1 - read[i].1);
            assert!(dx.abs() <= 1.3e-4 + 1e-12 || dx.abs() > 1e-3, "{dx}");
            if dx.hypot(dy) > 1e-3 {
                swapped += 1;
                let partner = (0..model.parts.len()).find(|&j| {
                    j != i
                        && model.movable[j]
                        && (centres[i].0 - read[j].0).abs() < 1e-3
                        && (centres[i].1 - read[j].1).abs() < 1e-3
                });
                let j = partner
                    .unwrap_or_else(|| panic!("{} moved to no part's place", model.references[i]));
                assert_eq!(
                    model.footprint_ids[i], model.footprint_ids[j],
                    "{} took {}'s place",
                    model.references[i], model.references[j]
                );
                assert_eq!(model.parts[i].layer_side, model.parts[j].layer_side);
            }
            if rotations[i] != read_rotation[i] {
                turned += 1;
                assert_eq!(model.pad_counts[i], 2, "{} was turned", model.references[i]);
            }
        }
    }
    assert!(swapped > 20, "swaps never happened: {swapped}");
    assert!(turned > 20, "turns never happened: {turned}");

    // With swaps and turns off, only shifts: nothing but a small move, no turn.
    let shifts = PerturbParams {
        swaps: false,
        turns: false,
        ..params
    };
    for seed in 1..=20u64 {
        let (centres, rotations) = perturb(&model, &shifts, &mut SplitMix64(seed));
        assert_eq!(rotations, read_rotation);
        for i in 0..centres.len() {
            assert!((centres[i].0 - read[i].0).abs() < 1e-3);
        }
    }
}

#[test]
fn a_turned_part_is_written_turned_and_its_courtyard_follows() {
    // A 2-pad part turned by `perturb` is legalized with its turned size, and the
    // written board has it at the turned rotation.
    let parts = vec![
        Fp::new("J1", 58.0, 20.0).pad(pad("1", 0.0, 0.0, Some("A"))),
        Fp::new("R1", 20.0, 20.0)
            .court_rect(-1.0, -0.5, 3.0, 0.5)
            .two_pad("A", "B"),
    ];
    let model = model_of(&board(OUTLINE, &parts), &[]);
    let turning = PerturbParams {
        moves: 12,
        radius_mm: 1e-4,
        swaps: false,
        turns: true,
    };
    let mut seen = 0;
    for seed in 1..=30u64 {
        let recipe = Recipe {
            family: Family::Perturb,
            seed,
            params: Params::Perturb(turning),
        };
        let candidate = generate(&model, &recipe).unwrap();
        let written = placements_applied_to(&model.text, &candidate.placements).unwrap();
        let scan = konnect_sexp::board::footprint_courtyards(&parse(&written));
        let r1 = scan
            .items
            .iter()
            .find(|c| c.reference.as_deref() == Some("R1"))
            .unwrap();
        let placement = &candidate.placements[0];
        assert_eq!(
            r1.rotation_deg, placement.rotation,
            "the rotation is written"
        );
        // A turn is about the courtyard centre, which the anchor offset (turned with
        // the part) must put back where the generator asked.
        let index = model.references.binary_search(&"R1".to_string()).unwrap();
        let read = bbox_centre(model.parts[index].bbox);
        let now = bbox_centre(r1.bbox);
        assert!(
            (now.0 - read.0).abs() < 5e-3 && (now.1 - read.1).abs() < 5e-3,
            "seed {seed}: the courtyard centre went from {read:?} to {now:?}"
        );
        if placement.rotation != 0.0 {
            seen += 1;
        }
        // The legalizer judged the box the scan sees: no hard failure.
        let evaluated = evaluate_text(&written, &["J1"]);
        assert_eq!(evaluated["hard_failures"].as_array().unwrap().len(), 0);
    }
    assert!(seen > 5, "turns never happened");
}

#[test]
fn neutral_start_is_outside_the_outline_and_overlap_free() {
    let model = demo_model();
    let placements = neutral_start(&model);
    assert_eq!(placements.len(), model.held.movable.len());
    let written = placements_applied_to(&model.text, &placements).unwrap();
    let tree = parse(&written);
    // Held parts unchanged; sides and rotations as read.
    assert_eq!(place_held::check_held(&model.held, &tree), Ok(()));
    for placement in &placements {
        let i = model
            .references
            .binary_search(&placement.reference)
            .unwrap();
        assert_eq!(placement.rotation, model.parts[i].rotation_deg);
        assert!(model.movable[i]);
    }
    let scan = konnect_sexp::board::footprint_courtyards(&tree);
    let threshold = model.outline.containment_threshold_mm();
    let (_, _, bounds_right, _) = model.bounds;
    let mut movable_boxes = Vec::new();
    for item in &scan.items {
        let reference = item.reference.as_deref().unwrap();
        let i = model
            .references
            .binary_search(&reference.to_string())
            .unwrap();
        let (x0, y0, x1, y1) = item.bbox;
        if model.movable[i] {
            let over = model
                .outline
                .overhang(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
            assert!(
                !over.touches_board && over.depth_mm > threshold,
                "{reference} is on the board"
            );
            assert!(
                x0 > bounds_right,
                "{reference} is not to the right of the bounds"
            );
            assert_eq!(item.layer_side, model.parts[i].layer_side);
            movable_boxes.push(item.bbox);
        }
    }
    // Every pair of parts is a millimetre apart on one axis at least.
    for (a, boxa) in movable_boxes.iter().enumerate() {
        for boxb in movable_boxes.iter().skip(a + 1) {
            let gap_x = (boxa.0 - boxb.2).max(boxb.0 - boxa.2);
            let gap_y = (boxa.1 - boxb.3).max(boxb.1 - boxa.3);
            assert!(
                gap_x >= 0.999 || gap_y >= 0.999,
                "{boxa:?} and {boxb:?} are too close"
            );
        }
    }
    // Nothing of the board is touched but where the movable parts sit.
    let mut again = placements.clone();
    again.reverse();
    assert_eq!(placements_applied_to(&model.text, &again).unwrap(), written);
}

#[test]
fn the_bench_matrix_has_288_recipes_in_a_fixed_order() {
    let matrix = Matrix::bench();
    let all = recipes(&matrix);
    assert_eq!(all.len(), 288);
    assert_eq!(all, recipes(&Matrix::bench()));
    let families: Vec<Family> = all.iter().map(|r| r.family).collect();
    assert!(families[..96].iter().all(|f| *f == Family::Force));
    assert!(families[96..160].iter().all(|f| *f == Family::Schematic));
    assert!(families[160..].iter().all(|f| *f == Family::Perturb));
    // Seed-major, then the parameter sets in the order given.
    let rail = |recipe: &Recipe| match &recipe.params {
        Params::Force(p) => p.rail_weight,
        _ => panic!("not a force recipe"),
    };
    assert_eq!((all[0].seed, rail(&all[0])), (1, 0.0));
    assert_eq!((all[1].seed, rail(&all[1])), (1, 0.5));
    assert_eq!((all[2].seed, rail(&all[2])), (1, 3.0));
    assert_eq!((all[3].seed, rail(&all[3])), (2, 0.0));
    assert_eq!(all[95].seed, 32);
    let Params::Schematic(raw) = &all[96].params else {
        panic!()
    };
    let Params::Schematic(polished) = &all[97].params else {
        panic!()
    };
    assert_eq!((all[96].seed, raw.polish.is_none()), (1, true));
    assert_eq!(
        (all[97].seed, polished.polish.map(|p| p.rail_weight)),
        (1, Some(0.5))
    );
    assert_eq!(all[159].seed, 32);
    let Params::Perturb(small) = &all[160].params else {
        panic!()
    };
    let Params::Perturb(large) = &all[161].params else {
        panic!()
    };
    assert_eq!((all[160].seed, small.moves, small.radius_mm), (1, 4, 2.0));
    assert_eq!((all[161].seed, large.moves, large.radius_mm), (1, 12, 5.0));
    assert!(small.swaps && small.turns && large.swaps && large.turns);
    assert_eq!(all[287].seed, 64);
    // Every force recipe carries the design's constants.
    for recipe in &all[..96] {
        let Params::Force(p) = &recipe.params else {
            panic!()
        };
        assert_eq!(
            (p.fanout_limit, p.pair_weight, p.group_weight, p.iterations),
            (8, 10.0, 0.5, 600)
        );
    }
}

#[test]
fn invalid_recipes_are_failures_not_panics() {
    let model = demo_model();
    let mut mismatched = force_recipe(1);
    mismatched.family = Family::Perturb;
    assert!(matches!(
        generate(&model, &mismatched),
        Err(GenerateFailure::Invalid(_))
    ));
    let mut no_iterations = force_recipe(1);
    if let Params::Force(p) = &mut no_iterations.params {
        p.iterations = 0;
    }
    assert!(matches!(
        generate(&model, &no_iterations),
        Err(GenerateFailure::Invalid(_))
    ));
    let mut negative = force_recipe(1);
    if let Params::Force(p) = &mut negative.params {
        p.pair_weight = -1.0;
    }
    assert!(matches!(
        generate(&model, &negative),
        Err(GenerateFailure::Invalid(_))
    ));
    let mut still = perturb_recipe(1, false);
    if let Params::Perturb(p) = &mut still.params {
        p.radius_mm = 0.0;
    }
    assert!(matches!(
        generate(&model, &still),
        Err(GenerateFailure::Invalid(_))
    ));
    // A part larger than the board cannot be relaxed inside it.
    let big = board(
        Shape::Rect(0.0, 0.0, 10.0, 10.0),
        &[
            Fp::new("J1", 5.0, 5.0)
                .court(1.0, 1.0)
                .pad(pad("1", 0.0, 0.0, Some("A"))),
            Fp::new("U1", 5.0, 5.0).court(8.0, 2.0).two_pad("A", "B"),
        ],
    );
    let model = model_of(&big, &[]);
    assert!(matches!(
        generate(&model, &force_recipe(1)),
        Err(GenerateFailure::Invalid(_))
    ));
}

#[test]
fn the_model_refuses_a_text_that_is_not_the_held_sets() {
    let text = demo_text();
    let set = held_set(&text, &BTreeSet::new(), &HoldRules::default()).unwrap();
    let other = board(OUTLINE, &demo_parts()[..6]);
    assert!(board_model(&other, set.clone(), Intent::default()).is_err());
    let model = board_model(&text, set, Intent::default()).unwrap();
    assert_eq!(model.references.len(), demo_parts().len());
    let mut sorted = model.references.clone();
    sorted.sort();
    assert_eq!(model.references, sorted);
    assert_eq!(
        model.parts[0].reference.as_deref(),
        Some(model.references[0].as_str())
    );
    assert_eq!(model.bounds, (0.0, 0.0, 60.0, 40.0));
    let u1 = model.references.iter().position(|r| r == "U1").unwrap();
    assert_eq!(model.pad_counts[u1], 3);
    assert_eq!(model.footprint_ids[u1], "Test:U");
    assert_eq!(model.parts[u1].layer_side, Side::Front);
}

#[test]
fn ring_cells_walk_outward_nearest_first() {
    assert_eq!(ring_cells(0), vec![(0, 0)]);
    for ring in 1..=5i64 {
        let cells = ring_cells(ring);
        assert_eq!(cells.len() as i64, 8 * ring);
        assert!(cells.iter().all(|&(i, j)| i.abs().max(j.abs()) == ring));
        let distances: Vec<i64> = cells.iter().map(|&(i, j)| i * i + j * j).collect();
        let mut sorted = distances.clone();
        sorted.sort_unstable();
        assert_eq!(distances, sorted);
        let unique: BTreeSet<(i64, i64)> = cells.iter().copied().collect();
        assert_eq!(unique.len(), cells.len());
    }
}

#[test]
fn unit_is_in_the_half_open_unit_interval() {
    let mut rng = SplitMix64(7);
    for _ in 0..1000 {
        let value = unit(&mut rng);
        assert!((0.0..1.0).contains(&value));
    }
    assert_eq!(unit(&mut SplitMix64(7)), unit(&mut SplitMix64(7)));
}
