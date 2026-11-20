//! Tests of `place_generators`, on small boards built by the shared test builder.

use super::*;
use crate::place_held::tests::{board, evaluate_text, pad, parse, Fp, Shape};
use crate::place_held::{self, decoupling_pairs, groups, held_set, DecouplingPair, HoldRules};
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
        let count = candidate.placements.len();
        assert_eq!(listed.len(), count, "no part is listed twice");
        assert!(
            count > 0 && count <= model.held.movable.len(),
            "a part that moved or turned is listed once"
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
    // A recipe moved every movable part (by a micron: a part exactly where the input has
    // it is legal as it is, whatever the outline says).
    for (i, centre) in centres.iter_mut().enumerate() {
        if model.movable[i] {
            centre.0 += 1e-3;
        }
    }
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
    // Every part moved (by a micron): a part left where the input has it is legal as is.
    for centre in &mut centres {
        centre.0 += 1e-3;
    }
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
        // R1 neither moved nor turned: it is not listed, and the board is the input.
        let Some(placement) = candidate.placements.first() else {
            assert_eq!(written, model.text);
            continue;
        };
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

// ─── A part that stays stays byte for byte ───────────────────────────────────

/// The text of the footprint of `reference`, from its `(footprint` to its closing line.
fn block_of(text: &str, reference: &str) -> String {
    let at = text
        .find(&format!("(property \"Reference\" \"{reference}\""))
        .unwrap_or_else(|| panic!("{reference} is not on the board"));
    let start = text[..at].rfind("(footprint \"").unwrap();
    let end = start + text[start..].find("\n)\n").unwrap() + 3;
    text[start..end].to_string()
}

#[test]
fn a_part_that_did_not_move_keeps_its_footprint_block_byte_for_byte() {
    // R9 sits at 180 degrees with texts at angles a set would re-spell (100 degrees
    // becomes -80). A candidate that moves one part must leave R9 exactly as it was.
    let mut parts = demo_parts();
    parts.push(
        Fp::new("R9", 50.0, 30.0)
            .lib("Test:R")
            .two_pad("SIG8", "SIG9")
            .rot(180.0),
    );
    let input = board(OUTLINE, &parts);
    let input = input.replacen(
        "(property \"Reference\" \"R9\" (at 0 -1.5 0)",
        "(property \"Reference\" \"R9\" (at 0 -1.5 100)",
        1,
    );
    let model = model_of(&input, &[]);
    let one_move = |seed: u64| Recipe {
        family: Family::Perturb,
        seed,
        params: Params::Perturb(PerturbParams {
            moves: 1,
            radius_mm: 2.0,
            swaps: false,
            turns: false,
        }),
    };
    let mut left_alone = 0;
    for seed in 1..=40u64 {
        let candidate = generate(&model, &one_move(seed)).unwrap();
        assert_eq!(candidate.placements.len(), 1, "seed {seed}: one part moved");
        let written = placements_applied_to(&model.text, &candidate.placements).unwrap();
        let moved = &candidate.placements[0].reference;
        for reference in &model.references {
            if reference == moved {
                continue;
            }
            assert_eq!(
                block_of(&written, reference),
                block_of(&input, reference),
                "seed {seed}: {reference} was not touched and must not change"
            );
        }
        if moved != "R9" {
            left_alone += 1;
        }
    }
    assert!(left_alone > 20, "R9 was left alone in {left_alone} of 40");
    // R9 itself, when it is the one that moves, is written.
    let moved_r9 = (1..=200u64).find_map(|seed| {
        let candidate = generate(&model, &one_move(seed)).unwrap();
        (candidate.placements[0].reference == "R9").then_some(candidate)
    });
    let written = placements_applied_to(&model.text, &moved_r9.unwrap().placements).unwrap();
    assert_ne!(block_of(&written, "R9"), block_of(&input, "R9"));
}

// ─── Clearance after the anchors are rounded ─────────────────────────────────

#[test]
fn legalized_parts_do_not_overlap_once_their_anchors_are_rounded_and_written() {
    // R1 and R2 have courtyards a hair under 1.5 mm wide, centred a fraction of a micron
    // off their anchors, so that after `round_um` R1's courtyard moves toward R2 and R2's
    // toward R1. Legalize puts R2 on the grid next to R1, 0.1 to 1 µm from touching: a
    // clearance of 1 nm would call that legal, and the written board would overlap.
    for (width, off) in [
        (1.4995, 0.0004),
        (1.4995, 0.00033),
        (1.4999, 0.0002),
        (1.4999, 0.00045),
        (1.4998, 0.00035),
        (1.4990, 0.00049),
    ] {
        let rect = |centre: f64| (centre - width / 2.0, -0.5, centre + width / 2.0, 0.5);
        let (a0, a1, a2, a3) = rect(off);
        let (b0, b1, b2, b3) = rect(-off);
        let parts = vec![
            Fp::new("R1", 20.0, 20.0)
                .court_rect(a0, a1, a2, a3)
                .two_pad("N1", "N2"),
            Fp::new("R2", 40.0, 20.0)
                .court_rect(b0, b1, b2, b3)
                .two_pad("N1", "N2"),
        ];
        let model = model_of(&board(OUTLINE, &parts), &[]);
        let sizes = as_read_sizes(&model);
        let read = as_read_centres(&model);
        let mut centres = read.clone();
        // R2 proposed right up against R1: the centre distance of exactly the width.
        centres[1] = (read[0].0 + width, read[0].1);
        legalize(&model, &sizes, &mut centres).unwrap();
        assert_ne!(centres[1], read[1], "R2 moved");
        // The anchors exactly as `generate` writes them.
        let mut placements = Vec::new();
        for i in 0..2 {
            if centres[i] == read[i] {
                continue;
            }
            let part = &model.parts[i];
            let offset = (part.at.0 - read[i].0, part.at.1 - read[i].1);
            placements.push(IpcFootprintPlacement {
                reference: model.references[i].clone(),
                x: round_um(centres[i].0 + offset.0),
                y: round_um(centres[i].1 + offset.1),
                rotation: part.rotation_deg,
            });
        }
        let written = placements_applied_to(&model.text, &placements).unwrap();
        let evaluated = evaluate_text(&written, &[]);
        assert_eq!(
            evaluated["hard_failures"].as_array().unwrap().len(),
            0,
            "width {width}, offset {off}: {:?}",
            evaluated["hard_failures"]
        );
        let scan = konnect_sexp::board::footprint_courtyards(&parse(&written));
        let (a, b) = (&scan.items[0], &scan.items[1]);
        let gap = b.bbox.0 - a.bbox.2;
        assert!(gap >= 0.0, "width {width}, offset {off}: gap {gap}");
    }
}

// ─── Legalize, pinned ────────────────────────────────────────────────────────

/// A board with an outline whose corner is not at the origin (x 10..70, y 20..60), one
/// movable resistor of 2 x 1 mm (`R1`, read far from the place the tests drop it) and
/// the held `extra`.
fn pinned_model(extra: Vec<Fp>) -> BoardModel {
    pinned_model_of(extra, (1.0, 0.5))
}

/// [`pinned_model`] with R1's courtyard half-width and half-height given.
fn pinned_model_of(extra: Vec<Fp>, half: (f64, f64)) -> BoardModel {
    let mut parts = extra;
    parts.push(
        Fp::new("R1", 20.0, 30.0)
            .court(half.0, half.1)
            .two_pad("N1", "N2"),
    );
    model_of(&board(Shape::Rect(10.0, 20.0, 70.0, 60.0), &parts), &[])
}

fn legalized_r1(model: &BoardModel, at: (f64, f64)) -> (f64, f64) {
    let sizes = as_read_sizes(model);
    let mut centres = as_read_centres(model);
    let r1 = model.references.binary_search(&"R1".to_string()).unwrap();
    centres[r1] = at;
    legalize(model, &sizes, &mut centres).unwrap();
    centres[r1]
}

#[test]
fn legalize_needs_one_size_and_one_centre_per_part() {
    let model = pinned_model(Vec::new());
    let sizes = as_read_sizes(&model);
    let centres = as_read_centres(&model);
    assert_eq!(sizes.len(), 1);
    let mut longer = centres.clone();
    longer.push((0.0, 0.0));
    // Either list alone with the wrong length is refused (both wrong is not the only case).
    let mut too_many_centres = longer.clone();
    assert!(matches!(
        legalize(&model, &sizes, &mut too_many_centres),
        Err(GenerateFailure::Invalid(_))
    ));
    let mut too_many_sizes = sizes.clone();
    too_many_sizes.push((1.0, 1.0));
    let mut ok_centres = centres.clone();
    assert!(matches!(
        legalize(&model, &too_many_sizes, &mut ok_centres),
        Err(GenerateFailure::Invalid(_))
    ));
    let mut ok_centres = centres.clone();
    assert!(legalize(&model, &sizes, &mut ok_centres).is_ok());
}

#[test]
fn legalize_keeps_a_part_flush_with_the_outline_and_moves_one_that_sticks_out() {
    let model = pinned_model(Vec::new());
    // R1 is 2 x 1 mm; the outline is x 10..70, y 20..60. Flush on each side: legal as
    // it is, to the last digit.
    for flush in [(11.0, 40.0), (69.0, 40.0), (40.0, 20.5), (40.0, 59.5)] {
        assert_eq!(legalized_r1(&model, flush), flush, "flush at {flush:?}");
    }
    // Past the outline's bounding box by 0.05 mm on one side: nearer the edge than the
    // outline's own "on the edge" distance (0.1 µm) is not, but 50 nm is, so the
    // containment check below the bbox check lets these through, and the bbox check
    // is what moves them back to the flush cell.
    for (past, flush) in [
        ((10.99995, 40.0), (11.0, 40.0)),
        ((69.00005, 40.0), (69.0, 40.0)),
        ((40.0, 20.49995), (40.0, 20.5)),
        ((40.0, 59.50005), (40.0, 59.5)),
    ] {
        assert_eq!(
            legalized_r1(&model, past),
            flush,
            "past by 50 nm at {past:?}"
        );
    }
    // Well outside: the nearest legal cell.
    assert_eq!(legalized_r1(&model, (10.5, 40.0)), (11.0, 40.0));
    assert_eq!(legalized_r1(&model, (69.5, 40.0)), (69.0, 40.0));
    assert_eq!(legalized_r1(&model, (40.0, 20.0)), (40.0, 20.5));
    assert_eq!(legalized_r1(&model, (40.0, 60.0)), (40.0, 59.5));
}

#[test]
fn legalize_measures_overlap_by_both_sizes_on_both_axes() {
    // J1, 2 x 2 mm, held at (40, 40); R1 is 1 x 1 here, so half the two sizes together is
    // 1.5 mm on each axis (their product over two would be 1.0, and their sum times two 6).
    let model = pinned_model_of(
        vec![Fp::new("J1", 40.0, 40.0)
            .court(1.0, 1.0)
            .pad(pad("1", 0.0, 0.0, Some("N1")))],
        (0.5, 0.5),
    );
    // Clear in y only (|dy| 1.6 > 1.5), clear in x only (|dx| 1.6 > 1.5): both stay.
    assert_eq!(legalized_r1(&model, (40.0, 41.6)), (40.0, 41.6));
    assert_eq!(legalized_r1(&model, (41.6, 40.0)), (41.6, 40.0));
    assert_eq!(legalized_r1(&model, (38.4, 40.0)), (38.4, 40.0));
    assert_eq!(legalized_r1(&model, (40.0, 38.4)), (40.0, 38.4));
    // On top of J1: 3 steps of 0.5 mm leave R1 touching it (1.5 mm), so the nearest
    // clear cells are 4 steps (2.0 mm) away, and of the four, the left one comes first.
    assert_eq!(legalized_r1(&model, (40.0, 40.0)), (38.0, 40.0));
}

#[test]
fn legalize_takes_equal_cells_in_a_fixed_order() {
    // Two parts of 2 x 2 mm: R1 dropped on J1 must clear it by 2.002 mm, so the nearest
    // cells are 2.5 mm away, four of them equally near: left first (smaller step in x),
    // then above, below and right.
    let square = |model_parts: Vec<Fp>| {
        let mut parts = model_parts;
        parts.push(
            Fp::new("R1", 20.0, 30.0)
                .court(1.0, 1.0)
                .two_pad("N1", "N2"),
        );
        model_of(&board(Shape::Rect(10.0, 20.0, 70.0, 60.0), &parts), &[])
    };
    let j1 = || {
        Fp::new("J1", 40.0, 40.0)
            .court(1.0, 1.0)
            .pad(pad("1", 0.0, 0.0, Some("N1")))
    };
    let model = square(vec![j1()]);
    assert_eq!(legalized_r1(&model, (40.0, 40.0)), (37.5, 40.0));
    // With the left cell taken, above comes next.
    let j2 = Fp::new("J2", 37.5, 40.0)
        .court(1.0, 1.0)
        .pad(pad("1", 0.0, 0.0, Some("N1")));
    let model = square(vec![j1(), j2]);
    assert_eq!(legalized_r1(&model, (40.0, 40.0)), (40.0, 37.5));
    // And below when above is taken too.
    let j3 = Fp::new("J3", 40.0, 37.5)
        .court(1.0, 1.0)
        .pad(pad("1", 0.0, 0.0, Some("N1")));
    let j2 = Fp::new("J2", 37.5, 40.0)
        .court(1.0, 1.0)
        .pad(pad("1", 0.0, 0.0, Some("N1")));
    let model = square(vec![j1(), j2, j3]);
    assert_eq!(legalized_r1(&model, (40.0, 40.0)), (40.0, 42.5));
}

// ─── Unmoved parts are legal as they are ─────────────────────────────────────

#[test]
fn legalize_leaves_unmoved_parts_alone_even_where_the_input_overlaps_or_overhangs() {
    // R1 and R2 overlap each other in the input, R3 hangs over the right edge, as a
    // socket may on purpose. None of them moves in the recipe: all stay, no NoRoom.
    // R4 is moved onto R1: it must land clear of all three and inside the outline.
    let parts = vec![
        Fp::new("R1", 20.0, 20.0).two_pad("N1", "N2"),
        Fp::new("R2", 20.5, 20.0).two_pad("N2", "N3"),
        Fp::new("R3", 59.8, 30.0).two_pad("N3", "N4"),
        Fp::new("R4", 40.0, 10.0).two_pad("N4", "N5"),
    ];
    let model = model_of(&board(OUTLINE, &parts), &[]);
    let sizes = as_read_sizes(&model);
    let read = as_read_centres(&model);
    let r4 = 3;
    // Nothing moves.
    let mut centres = read.clone();
    legalize(&model, &sizes, &mut centres).expect("an input as it is needs no room");
    assert_eq!(centres, read);
    // R4 dropped on R1.
    let mut centres = read.clone();
    centres[r4] = read[0];
    legalize(&model, &sizes, &mut centres).expect("R4 finds a cell");
    assert_eq!(&centres[..3], &read[..3], "the unmoved parts stay");
    assert_ne!(centres[r4], read[0]);
    let (x, y) = centres[r4];
    let (w, h) = sizes[r4];
    for (k, other) in read.iter().enumerate().take(3) {
        let ix = (w + sizes[k].0) / 2.0 - (x - other.0).abs();
        let iy = (h + sizes[k].1) / 2.0 - (y - other.1).abs();
        assert!(ix <= 0.0 || iy <= 0.0, "R4 overlaps R{}", k + 1);
    }
    let (bx0, by0, bx1, by1) = model.bounds;
    assert!(x - w / 2.0 >= bx0 && x + w / 2.0 <= bx1 && y - h / 2.0 >= by0 && y + h / 2.0 <= by1);
    // A moved part may not overhang either: R4 dropped over the right edge comes back in.
    let mut centres = read.clone();
    centres[r4] = (59.9, 10.0);
    legalize(&model, &sizes, &mut centres).unwrap();
    assert!(centres[r4].0 + sizes[r4].0 / 2.0 <= bx1);
    // And a part that turned is moved, not unmoved: R1 with its size swapped is judged.
    let mut turned_sizes = sizes.clone();
    turned_sizes[0] = (sizes[0].1, sizes[0].0);
    let mut centres = read.clone();
    legalize(&model, &turned_sizes, &mut centres).unwrap();
    let (tw, th) = turned_sizes[0];
    for (k, other) in read.iter().enumerate().skip(1).take(2) {
        let ix = (tw + sizes[k].0) / 2.0 - (centres[0].0 - other.0).abs();
        let iy = (th + sizes[k].1) / 2.0 - (centres[0].1 - other.1).abs();
        assert!(ix <= 0.0 || iy <= 0.0, "turned R1 overlaps R{}", k + 1);
    }
}

// ─── The schematic shelves fit the board ─────────────────────────────────────

/// The S recipe of `params` and `seed`.
fn schematic_of(params: SchematicParams, seed: u64) -> Recipe {
    Recipe {
        family: Family::Schematic,
        seed,
        params: Params::Schematic(params),
    }
}

/// Whether every part's courtyard (`centres`, `sizes`) lies inside `bounds`.
fn all_inside(model: &BoardModel, sizes: &[(f64, f64)], centres: &[(f64, f64)]) -> bool {
    let (bx0, by0, bx1, by1) = model.bounds;
    centres.iter().zip(sizes).all(|((x, y), (w, h))| {
        x - w / 2.0 >= bx0 && x + w / 2.0 <= bx1 && y - h / 2.0 >= by0 && y + h / 2.0 <= by1
    })
}

#[test]
fn the_schematic_shelves_take_a_smaller_margin_when_the_wide_one_overflows() {
    // 40 parts of 2 x 1 mm, each alone in its cluster, on a 30 x 20 mm board. At a 2 mm
    // margin a part takes 2 + 2 mm and a cluster 2 mm more: five to a shelf, eight shelves
    // of 1 mm and 2 mm margins, 24 mm of shelves on a 20 mm board. At 1 mm: seven to a
    // shelf, six shelves, 12 mm. It fits.
    let parts: Vec<Fp> = (0..40)
        .map(|k| {
            Fp::new(
                &format!("R{k:02}"),
                3.0 + (k % 9) as f64 * 3.0,
                3.0 + (k / 9) as f64 * 3.0,
            )
            .two_pad(&format!("A{k}"), &format!("B{k}"))
        })
        .collect();
    let model = model_of(&board(Shape::Rect(0.0, 0.0, 30.0, 20.0), &parts), &[]);
    let sizes = as_read_sizes(&model);
    let found = clusters(&model, 8);
    assert_eq!(found.len(), 40);
    assert!(
        !grid_with_margin(&model, &sizes, &found, 2.0).1,
        "2 mm overflows"
    );
    assert!(grid_with_margin(&model, &sizes, &found, 1.0).1);

    let params = SchematicParams {
        fanout_limit: 8,
        polish: None,
    };
    let centres = schematic_grid(&model, &params, &mut SplitMix64(7));
    assert!(all_inside(&model, &sizes, &centres));
    // The shelves are the 1 mm ones, not a clamped heap: every two parts are at least 1 mm apart.
    for i in 0..centres.len() {
        for j in (i + 1)..centres.len() {
            let gap_x = (centres[i].0 - centres[j].0).abs() - (sizes[i].0 + sizes[j].0) / 2.0;
            let gap_y = (centres[i].1 - centres[j].1).abs() - (sizes[i].1 + sizes[j].1) / 2.0;
            assert!(
                gap_x.max(gap_y) >= 1.0 - 1e-9,
                "parts {i} and {j} are too close"
            );
        }
    }
    for seed in 1..=8u64 {
        generate(&model, &schematic_of(params, seed))
            .unwrap_or_else(|e| panic!("seed {seed}: {e:?}"));
    }
}

#[test]
fn a_cluster_with_one_large_part_fits_a_board_the_cell_grid_overflowed() {
    // One chain of a 20 x 10 mm socket and 30 parts of 2 x 1 mm on a 40 x 30 mm board.
    // Cells of the cluster's largest member (22 x 12 mm, six to a row) are 130 mm wide
    // and overflow it; shelves with each part at its own size fit at the first margin.
    let mut parts = vec![Fp::new("J9", 10.0, 10.0)
        .court(10.0, 5.0)
        .two_pad("N0", "N1")];
    for k in 1..=30 {
        parts.push(
            Fp::new(
                &format!("R{k:02}"),
                2.0 + ((k - 1) % 12) as f64 * 3.0,
                20.0 + ((k - 1) / 12) as f64 * 3.0,
            )
            .two_pad(&format!("N{k}"), &format!("N{}", k + 1)),
        );
    }
    let rules = HoldRules {
        connector_prefixes: Vec::new(),
        ..HoldRules::default()
    };
    let text = board(Shape::Rect(0.0, 0.0, 40.0, 30.0), &parts);
    let set = held_set(&text, &BTreeSet::new(), &rules).expect("the held set");
    let model = board_model(&text, set, Intent::default()).expect("the model");
    assert_eq!(model.held.movable.len(), 31, "the socket is movable here");
    let sizes = as_read_sizes(&model);
    let found = clusters(&model, 8);
    assert_eq!(found.len(), 1, "one chain, one cluster");
    let (centres, fits) = grid_with_margin(&model, &sizes, &found, 2.0);
    assert!(fits, "the shelves fit at the first margin");
    assert!(all_inside(&model, &sizes, &centres));

    let params = SchematicParams {
        fanout_limit: 8,
        polish: None,
    };
    for seed in 1..=4u64 {
        let candidate = generate(&model, &schematic_of(params, seed))
            .unwrap_or_else(|e| panic!("seed {seed}: no room? {e:?}"));
        let written = placements_applied_to(&model.text, &candidate.placements).unwrap();
        let evaluated = evaluate_text(&written, &[]);
        assert_eq!(evaluated["hard_failures"].as_array().unwrap().len(), 0);
    }
}

#[test]
fn shelves_keep_a_cluster_together_and_clamp_when_nothing_fits() {
    // A board too small for the parts at any margin: the layout of the last margin has
    // every part's centre clamped so that its courtyard is inside the bounds.
    let parts: Vec<Fp> = (0..12)
        .map(|k| {
            Fp::new(&format!("R{k:02}"), 3.0 + k as f64, 5.0)
                .court(2.0, 1.5)
                .two_pad(&format!("A{k}"), &format!("B{k}"))
        })
        .collect();
    let model = model_of(&board(Shape::Rect(0.0, 0.0, 12.0, 8.0), &parts), &[]);
    let sizes = as_read_sizes(&model);
    let found = clusters(&model, 8);
    for margin in CELL_MARGINS_MM {
        assert!(
            !grid_with_margin(&model, &sizes, &found, margin).1,
            "{margin}"
        );
    }
    let params = SchematicParams {
        fanout_limit: 8,
        polish: None,
    };
    let centres = schematic_grid(&model, &params, &mut SplitMix64(1));
    assert!(all_inside(&model, &sizes, &centres));

    // The members of a cluster follow each other in area order on the shelves, and the
    // next cluster starts after them: two chains of three parts on a roomy board.
    let mut chained = Vec::new();
    for (chain, base) in [("X", 0), ("Y", 10)] {
        for k in 0..3 {
            chained.push(
                Fp::new(
                    &format!("{chain}{k}"),
                    5.0 + 5.0 * k as f64,
                    5.0 + base as f64,
                )
                .two_pad(&format!("{chain}{k}"), &format!("{chain}{}", k + 1)),
            );
        }
    }
    let model = model_of(&board(OUTLINE, &chained), &[]);
    let sizes = as_read_sizes(&model);
    let found = clusters(&model, 8);
    assert_eq!(found.len(), 2);
    let (centres, fits) = grid_with_margin(&model, &sizes, &found, 2.0);
    assert!(fits);
    for members in &found {
        let xs: Vec<f64> = members.iter().map(|&i| centres[i].0).collect();
        let ys: Vec<f64> = members.iter().map(|&i| centres[i].1).collect();
        // One shelf (the board is wide), each part a part-width and a margin from the last.
        assert!(ys.iter().all(|y| *y == ys[0]), "{ys:?}");
        let mut sorted = xs.clone();
        sorted.sort_by(f64::total_cmp);
        assert!(
            sorted.windows(2).all(|w| (w[1] - w[0] - 4.0).abs() < 1e-9),
            "{sorted:?}"
        );
    }
    // The second cluster starts a part width, the margin and one more margin after the first.
    let first_end = found[0]
        .iter()
        .map(|&i| centres[i].0)
        .fold(f64::MIN, f64::max)
        + 1.0;
    let second_start = found[1]
        .iter()
        .map(|&i| centres[i].0)
        .fold(f64::MAX, f64::min)
        - 1.0;
    assert!(
        (second_start - first_end - 4.0).abs() < 1e-9,
        "{first_end} {second_start}"
    );
}

// ─── Clearances beyond the courtyards ────────────────────────────────────────

fn with_clearances(mut model: BoardModel, part_gap_mm: f64, edge_mm: f64) -> BoardModel {
    model.clearances = Clearances {
        part_gap_mm,
        edge_mm,
    };
    model
}

/// The larger of the two axis gaps between the boxes of parts `a` and `b`.
fn gap_between_parts(sizes: &[(f64, f64)], centres: &[(f64, f64)], a: usize, b: usize) -> f64 {
    let gap_x = (centres[a].0 - centres[b].0).abs() - (sizes[a].0 + sizes[b].0) / 2.0;
    let gap_y = (centres[a].1 - centres[b].1).abs() - (sizes[a].1 + sizes[b].1) / 2.0;
    gap_x.max(gap_y)
}

#[test]
fn parts_without_a_courtyard_are_legalized_apart_by_the_part_gap() {
    // Two pads-only footprints (no courtyard: the box is the pads', 1.6 x 0.5 mm). R2 is
    // dropped on R1. Without a gap the nearest legal cell leaves 0.5 mm between them;
    // with 0.7 mm it must leave at least that.
    let parts = vec![
        Fp::new("R1", 30.0, 20.0).no_court().two_pad("N1", "N2"),
        Fp::new("R2", 10.0, 10.0).no_court().two_pad("N2", "N3"),
    ];
    let text = board(OUTLINE, &parts);
    let place = |gap: f64| {
        let model = with_clearances(model_of(&text, &[]), gap, 0.0);
        assert_eq!(
            model.parts[0].bbox_source,
            konnect_sexp::board::CourtyardSource::PadsFallback
        );
        let sizes = as_read_sizes(&model);
        let mut centres = as_read_centres(&model);
        centres[1] = centres[0];
        legalize(&model, &sizes, &mut centres).unwrap();
        gap_between_parts(&sizes, &centres, 0, 1)
    };
    let none = place(0.0);
    assert!((0.0..0.7).contains(&none), "{none}");
    let kept = place(0.7);
    assert!(kept >= 0.7 - 1e-9, "{kept}");
}

#[test]
fn two_real_courtyards_may_still_touch_closely() {
    // Real courtyards keep 2 µm, not the part gap: R2 at 3 µm from R1's courtyard stays.
    // The same distance between a courtyard and a pads-only part is moved.
    let courtyards = vec![
        Fp::new("R1", 30.0, 20.0).two_pad("N1", "N2"),
        Fp::new("R2", 10.0, 10.0).two_pad("N2", "N3"),
    ];
    let model = with_clearances(model_of(&board(OUTLINE, &courtyards), &[]), 0.7, 0.0);
    let sizes = as_read_sizes(&model);
    let mut centres = as_read_centres(&model);
    centres[1] = (centres[0].0 + sizes[0].0 + 0.003, centres[0].1);
    let proposed = centres[1];
    legalize(&model, &sizes, &mut centres).unwrap();
    assert_eq!(centres[1], proposed, "courtyards 3 µm apart are legal");
    // Closer than 2 µm is not.
    let mut centres = as_read_centres(&model);
    centres[1] = (centres[0].0 + sizes[0].0 + 0.001, centres[0].1);
    let proposed = centres[1];
    legalize(&model, &sizes, &mut centres).unwrap();
    assert_ne!(centres[1], proposed);

    let mixed = vec![
        Fp::new("R1", 30.0, 20.0).two_pad("N1", "N2"),
        Fp::new("R2", 10.0, 10.0).no_court().two_pad("N2", "N3"),
    ];
    let model = with_clearances(model_of(&board(OUTLINE, &mixed), &[]), 0.7, 0.0);
    let sizes = as_read_sizes(&model);
    let mut centres = as_read_centres(&model);
    centres[1] = (
        centres[0].0 + (sizes[0].0 + sizes[1].0) / 2.0 + 0.003,
        centres[0].1,
    );
    let proposed = centres[1];
    legalize(&model, &sizes, &mut centres).unwrap();
    assert_ne!(centres[1], proposed, "a pads-only part keeps the part gap");
    assert!(gap_between_parts(&sizes, &centres, 0, 1) >= 0.7 - 1e-9);
}

#[test]
fn a_moved_part_keeps_the_edge_clearance_inside_a_rounded_outline() {
    let parts = vec![Fp::new("R1", 20.0, 20.0).two_pad("N1", "N2")];
    let text = board(Shape::Rounded(0.0, 0.0, 60.0, 40.0, 10.0), &parts);
    let threshold = model_of(&text, &[]).outline.containment_threshold_mm();
    // R1 is 2 x 1 mm. Straight edge: its left side 0.3 mm from the edge. Corner: its
    // top-left corner 0.2 mm inside the arc of radius 10 about (10, 10).
    let straight = (1.3, 20.0);
    let corner = (4.07, 3.57);
    for at in [straight, corner] {
        let model = with_clearances(model_of(&text, &[]), 0.0, 0.0);
        let sizes = as_read_sizes(&model);
        let mut centres = as_read_centres(&model);
        centres[0] = at;
        legalize(&model, &sizes, &mut centres).unwrap();
        assert_eq!(
            centres[0], at,
            "inside the outline, so legal with no edge clearance"
        );

        let model = with_clearances(model_of(&text, &[]), 0.0, 0.5);
        let mut centres = as_read_centres(&model);
        centres[0] = at;
        legalize(&model, &sizes, &mut centres).unwrap();
        assert_ne!(centres[0], at, "{at:?} is within 0.5 mm of the edge");
        let (w, h) = sizes[0];
        let (x, y) = centres[0];
        let grown = [
            (x - w / 2.0 - 0.5, y - h / 2.0 - 0.5),
            (x + w / 2.0 + 0.5, y - h / 2.0 - 0.5),
            (x + w / 2.0 + 0.5, y + h / 2.0 + 0.5),
            (x - w / 2.0 - 0.5, y + h / 2.0 + 0.5),
        ];
        assert!(
            model.outline.overhang(&grown).depth_mm <= threshold,
            "the grown box of {at:?} -> {:?} is not inside",
            centres[0]
        );
    }
}

// ─── Copper obstacles and the sides a part occupies ──────────────────────────

/// `text` with `items` added before its closing parenthesis.
fn with_items(text: &str, items: &str) -> String {
    let end = text.rfind(')').expect("a board ends with a parenthesis");
    format!("{}{items}\n)\n", &text[..end])
}

/// A board text, an `(at …)`, a layer and effects, as a board-level `gr_text`.
fn gr_text(text: &str, at: &str, layer: &str, effects: &str) -> String {
    format!("(gr_text \"{text}\" (at {at}) (layer \"{layer}\") {effects})\n")
}

const PLAIN: &str = "(effects (font (size 1 1) (thickness 0.15)))";

/// The pinned board (x 10..70, y 20..60) with R1 (2 x 1 mm, on top) and `items` of copper.
fn copper_model(items: &str) -> BoardModel {
    copper_model_of(
        vec![Fp::new("R1", 20.0, 30.0).two_pad("N1", "N2")],
        &[],
        items,
    )
}

/// A board of `parts` in the pinned outline and `items`, with the footprints named in
/// `through_hole` given a `thru_hole` pad.
fn copper_model_of(parts: Vec<Fp>, through_hole: &[&str], items: &str) -> BoardModel {
    let mut text = board(Shape::Rect(10.0, 20.0, 70.0, 60.0), &parts);
    for reference in through_hole {
        let block = block_of(&text, reference);
        text = text.replace(&block, &block.replace(" smd rect", " thru_hole circle"));
    }
    model_of(&with_items(&text, items), &[])
}

fn r1_of(model: &BoardModel) -> usize {
    model.references.binary_search(&"R1".to_string()).unwrap()
}

fn close(a: Rect, b: Rect) -> bool {
    [a.0 - b.0, a.1 - b.1, a.2 - b.2, a.3 - b.3]
        .iter()
        .all(|d| d.abs() < 1e-6)
}

/// The only obstacle of `items` (one copper item), asserting there is exactly one.
fn only_obstacle(items: &str) -> CopperObstacle {
    let model = copper_model(items);
    assert_eq!(model.copper.len(), 1, "{items}: {:?}", model.copper);
    model.copper[0]
}

/// The gap between R1's box at `at` and `bbox`: the larger of the two axis gaps.
fn gap_to(model: &BoardModel, at: (f64, f64), bbox: Rect) -> f64 {
    let (w, h) = as_read_sizes(model)[r1_of(model)];
    let gap_x = (at.0 - (bbox.0 + bbox.2) / 2.0).abs() - (w + bbox.2 - bbox.0) / 2.0;
    let gap_y = (at.1 - (bbox.1 + bbox.3) / 2.0).abs() - (h + bbox.3 - bbox.1) / 2.0;
    gap_x.max(gap_y)
}

#[test]
fn a_moved_part_is_moved_off_copper_text_and_not_off_silkscreen_text() {
    // "ABCD", size 1, thickness 0.15: 4 x 1.3 + 0.15 = 5.35 wide, 1.7 + 0.15 = 1.85 high,
    // centred on (40, 40): x 37.325..42.675, y 39.075..40.925.
    let on_copper = copper_model(&gr_text("ABCD", "40 40", "F.Cu", PLAIN));
    let moved = legalized_r1(&on_copper, (40.0, 40.0));
    // R1 is 2 x 1 mm: the nearest cells clear of the box are 1.5 mm up or down (3 steps),
    // and of the two, above comes first.
    assert_eq!(moved, (40.0, 38.5));
    assert!(gap_to(&on_copper, moved, on_copper.copper[0].bbox) >= MOVED_CLEARANCE_MM);
    // The same text on the silkscreen, the fabrication layer or the back silkscreen is no
    // obstacle.
    for layer in ["F.SilkS", "F.Fab", "B.SilkS", "Cmts.User"] {
        let model = copper_model(&gr_text("ABCD", "40 40", layer, PLAIN));
        assert!(model.copper.is_empty(), "{layer}");
        assert_eq!(legalized_r1(&model, (40.0, 40.0)), (40.0, 40.0), "{layer}");
    }
    // A text that is no obstacle is the only difference: the copper text on a far corner
    // does not move R1 either.
    let far = copper_model(&gr_text("ABCD", "60 55", "F.Cu", PLAIN));
    assert_eq!(legalized_r1(&far, (40.0, 40.0)), (40.0, 40.0));
}

#[test]
fn a_moved_part_keeps_two_micrometres_from_copper_on_every_side() {
    // The text box is x 37.325..42.675, y 39.075..40.925 and R1 is 2 x 1 mm. At 3 µm from
    // a side R1 is legal where dropped; at 1 µm it is moved. Beside the text, level with it
    // or above and below it, at a distance, it stays: the axes are judged apart.
    let model = copper_model(&gr_text("ABCD", "40 40", "F.Cu", PLAIN));
    let (right, left, below, above) = (43.678, 36.322, 41.428, 38.572);
    for (at, near) in [
        ((right, 40.0), (right - 0.002, 40.0)),
        ((left, 40.0), (left + 0.002, 40.0)),
        ((40.0, below), (40.0, below - 0.002)),
        ((40.0, above), (40.0, above + 0.002)),
    ] {
        assert_eq!(legalized_r1(&model, at), at, "{at:?} is 3 µm off");
        assert_ne!(legalized_r1(&model, near), near, "{near:?} is 1 µm off");
    }
    for beside in [(46.0, 40.0), (34.0, 40.0), (40.0, 44.0), (40.0, 36.0)] {
        assert_eq!(legalized_r1(&model, beside), beside, "{beside:?}");
    }
    // Clear in one axis and over the other's range: the corner cases of the product test.
    for diagonal in [(46.0, 38.0), (34.0, 42.0), (45.0, 44.0), (35.0, 36.0)] {
        assert_eq!(legalized_r1(&model, diagonal), diagonal, "{diagonal:?}");
    }
    // A text turned a quarter is as tall as it was wide: R1 beside it moves sideways.
    let turned = copper_model(&gr_text("ABCD", "40 40 90", "F.Cu", PLAIN));
    assert_eq!(legalized_r1(&turned, (40.0, 40.0)), (38.0, 40.0));
}

#[test]
fn touching_the_part_gap_exactly_is_legal() {
    // A bar from (30, 30) to (34, 30), no width: its box is x 30..34, y 30..30. With a part
    // gap of 0.5 mm, R1 (2 x 1 mm) centred 3.5 mm from the bar's centre in x (a gap of
    // exactly 0.5 mm) or 1 mm from it in y (the same) is legal as dropped; closer is not.
    // Every number here is exact in binary, so "exactly" is exact.
    let bar = "(gr_line (start 30 30) (end 34 30) (layer \"F.Cu\"))";
    let model = with_clearances(copper_model(bar), 0.5, 0.0);
    for exact in [(35.5, 30.0), (28.5, 30.0), (32.0, 31.0), (32.0, 29.0)] {
        assert_eq!(legalized_r1(&model, exact), exact, "{exact:?}");
    }
    for closer in [(35.0, 30.0), (29.0, 30.0), (32.0, 30.5), (32.0, 29.5)] {
        assert_ne!(legalized_r1(&model, closer), closer, "{closer:?}");
    }
}

#[test]
fn a_moved_part_keeps_the_part_gap_from_copper() {
    let model = with_clearances(
        copper_model(&gr_text("ABCD", "40 40", "F.Cu", PLAIN)),
        0.7,
        0.0,
    );
    let moved = legalized_r1(&model, (40.0, 40.0));
    let gap = gap_to(&model, moved, model.copper[0].bbox);
    assert!(gap >= 0.7 - 1e-9, "{gap}");
    // With no gap a cell 0.575 mm from the text is legal; with 0.7 mm it is not.
    let none = copper_model(&gr_text("ABCD", "40 40", "F.Cu", PLAIN));
    assert_eq!(legalized_r1(&none, (40.0, 38.0)), (40.0, 38.0));
    assert_ne!(legalized_r1(&model, (40.0, 38.0)), (40.0, 38.0));
}

#[test]
fn a_copper_text_box_follows_its_size_lines_thickness_and_justification() {
    // No effects at all: KiCad's default size 1.524, thickness 0.15.
    let bare = only_obstacle(&gr_text("ABCD", "40 40", "F.Cu", ""));
    let (w, h) = (4.0 * 1.524 * 1.3 + 0.15, 1.524 * 1.7 + 0.15);
    assert!(
        close(
            bare.bbox,
            (
                40.0 - w / 2.0,
                40.0 - h / 2.0,
                40.0 + w / 2.0,
                40.0 + h / 2.0
            )
        ),
        "{bare:?}"
    );
    assert_eq!(bare.sides, Sides::TOP);
    // Size 2 x 3, thickness 0.3: width 2 x 2 x 1.3 + 0.3 = 5.5, height 3 x 1.7 + 0.3 = 5.4.
    let sized = only_obstacle(&gr_text(
        "AB",
        "40 40",
        "F.Cu",
        "(effects (font (size 2 3) (thickness 0.3)))",
    ));
    assert!(close(sized.bbox, (37.25, 37.3, 42.75, 42.7)), "{sized:?}");
    // Two lines, the longer of four characters: 4 x 1.3 + 0.15 wide, 2 x 1.7 + 0.15 high.
    let lines = only_obstacle(&gr_text("AB\\nCDEF", "40 40", "F.Cu", PLAIN));
    assert!(
        close(lines.bbox, (37.325, 38.225, 42.675, 41.775)),
        "{lines:?}"
    );
    // The longest line counts, not the last or the first: "ABCDE\nA".
    let first_longer = only_obstacle(&gr_text("ABCDE\\nA", "40 40", "F.Cu", PLAIN));
    assert!(
        close(first_longer.bbox, (36.675, 38.225, 43.325, 41.775)),
        "{first_longer:?}"
    );
    // Justified left: the box spans the anchor by the whole width in x, and is centred in y.
    let left = only_obstacle(&gr_text(
        "ABCD",
        "40 40",
        "F.Cu",
        "(effects (font (size 1 1) (thickness 0.15)) (justify left))",
    ));
    assert!(close(left.bbox, (34.65, 39.075, 45.35, 40.925)), "{left:?}");
    // Justified top: the whole height in y, centred in x. Mirrored right bottom: both.
    let top = only_obstacle(&gr_text(
        "ABCD",
        "40 40",
        "F.Cu",
        "(effects (font (size 1 1) (thickness 0.15)) (justify top))",
    ));
    assert!(close(top.bbox, (37.325, 38.15, 42.675, 41.85)), "{top:?}");
    let both = only_obstacle(&gr_text(
        "ABCD",
        "40 40",
        "B.Cu",
        "(effects (font (size 1 1) (thickness 0.15)) (justify right bottom mirror))",
    ));
    assert!(close(both.bbox, (34.65, 38.15, 45.35, 41.85)), "{both:?}");
    assert_eq!(both.sides, Sides::BOTTOM);
    // Empty text has no copper.
    assert!(copper_model(&gr_text("", "40 40", "F.Cu", PLAIN))
        .copper
        .is_empty());
}

#[test]
fn a_turned_copper_text_box_is_turned_about_its_anchor() {
    // A quarter turn swaps the half extents: 0.925 wide each side, 2.675 high.
    let quarter = only_obstacle(&gr_text("ABCD", "40 40 90", "F.Cu", PLAIN));
    assert!(
        close(quarter.bbox, (39.075, 37.325, 40.925, 42.675)),
        "{quarter:?}"
    );
    // A three-quarter turn, and a negative one, the same; a half turn leaves it.
    for angle in ["270", "-90"] {
        let turned = only_obstacle(&gr_text("ABCD", &format!("40 40 {angle}"), "F.Cu", PLAIN));
        assert!(close(turned.bbox, quarter.bbox), "{angle}: {turned:?}");
    }
    let half = only_obstacle(&gr_text("ABCD", "40 40 180", "F.Cu", PLAIN));
    assert!(
        close(half.bbox, (37.325, 39.075, 42.675, 40.925)),
        "{half:?}"
    );
    // 45 degrees: the hull of the turned box, (2.675 + 0.925) x sin 45 = 2.5456 each way.
    let slanted = only_obstacle(&gr_text("ABCD", "40 40 45", "F.Cu", PLAIN));
    let reach = (2.675 + 0.925) * std::f64::consts::FRAC_1_SQRT_2;
    assert!(
        close(
            slanted.bbox,
            (40.0 - reach, 40.0 - reach, 40.0 + reach, 40.0 + reach)
        ),
        "{slanted:?}"
    );
    // 30 degrees: 2.675 cos 30 + 0.925 sin 30 across, 2.675 sin 30 + 0.925 cos 30 high.
    let thirty = only_obstacle(&gr_text("ABCD", "40 40 30", "F.Cu", PLAIN));
    let (sin, cos) = 30f64.to_radians().sin_cos();
    let (rw, rh) = (2.675 * cos + 0.925 * sin, 2.675 * sin + 0.925 * cos);
    assert!(
        close(thirty.bbox, (40.0 - rw, 40.0 - rh, 40.0 + rw, 40.0 + rh)),
        "{thirty:?}"
    );
    // A justified, turned text turns the whole span: left, 90 degrees is 5.35 up and down.
    let justified = only_obstacle(&gr_text(
        "ABCD",
        "40 40 90",
        "F.Cu",
        "(effects (font (size 1 1) (thickness 0.15)) (justify left))",
    ));
    assert!(
        close(justified.bbox, (39.075, 34.65, 40.925, 45.35)),
        "{justified:?}"
    );
}

#[test]
fn a_quarter_turned_copper_text_box_is_exact_to_the_last_digit() {
    // At the origin, where a residue of 1e-16 from cos 90 would still show: the cardinal
    // angles swap the half extents, they do not multiply them by sines and cosines.
    let width = 4.0 * TEXT_WIDTH_PER_CHAR + 0.15;
    let height = TEXT_HEIGHT_PER_LINE + 0.15;
    let (half_w, half_h) = (width / 2.0, height / 2.0);
    for (angle, swapped) in [
        ("90", true),
        ("270", true),
        ("-90", true),
        ("-270", true),
        ("180", false),
        ("0", false),
    ] {
        let found = only_obstacle(&gr_text("ABCD", &format!("0 0 {angle}"), "F.Cu", PLAIN));
        let expected = if swapped {
            (-half_h, -half_w, half_h, half_w)
        } else {
            (-half_w, -half_h, half_w, half_h)
        };
        assert_eq!(found.bbox, expected, "{angle}");
    }
}

#[test]
fn unreadable_numbers_and_pens_do_not_make_a_box_smaller_or_unbounded() {
    // A cache polygon with an infinite coordinate is unreadable: the whole text takes the
    // estimate (2 characters: 2.75 x 1.85), never the box of the polygon that was readable.
    let cache = "(render_cache \"AB\" 0 (polygon (pts (xy 38 39) (xy inf 39) (xy 41 41))) \
                 (polygon (pts (xy 36 40.5) (xy 37 42) (xy 39 40))))";
    let item = gr_text(
        "AB",
        "40 40",
        "F.Cu",
        &format!("(effects (font (size 1 1) (thickness 0.15))) {cache}"),
    );
    assert!(close(
        only_obstacle(&item).bbox,
        (38.625, 39.075, 41.375, 40.925)
    ));
    // A negative or infinite stroke width is no width.
    for width in ["-0.4", "inf", "nan"] {
        let item = format!(
            "(gr_line (start 30 30) (end 34 32) (stroke (width {width}) (type default)) (layer \"F.Cu\"))"
        );
        assert!(
            close(only_obstacle(&item).bbox, (30.0, 30.0, 34.0, 32.0)),
            "{width}"
        );
    }
    // A pen of 0, a negative or an infinite one is the default 0.15.
    for thickness in ["0", "-1", "inf", "nan"] {
        let found = only_obstacle(&gr_text(
            "ABCD",
            "40 40",
            "F.Cu",
            &format!("(effects (font (size 1 1) (thickness {thickness})))"),
        ));
        assert!(
            close(found.bbox, (37.325, 39.075, 42.675, 40.925)),
            "{thickness}: {:?}",
            found.bbox
        );
    }
}

#[test]
fn a_text_box_takes_its_corners_over_its_start_and_end() {
    let stroke = "(stroke (width 0.2) (type default))";
    // The corners are the box as turned; start and end are not.
    let turned = format!(
        "(gr_text_box \"ab\" (start 60 30) (end 64 33) (pts (xy 58 29) (xy 62 29) (xy 62 35) (xy 58 35)) {stroke} (layer \"F.Cu\"))"
    );
    assert!(close(only_obstacle(&turned).bbox, (57.9, 28.9, 62.1, 35.1)));
    // No corners listed: start and end.
    let empty =
        format!("(gr_text_box \"ab\" (start 60 30) (end 64 33) (pts) {stroke} (layer \"F.Cu\"))");
    assert!(close(only_obstacle(&empty).bbox, (59.9, 29.9, 64.1, 33.1)));
}

#[test]
fn a_text_estimate_follows_line_spacing_a_default_size_and_knockout() {
    let at = "0 0";
    // Two lines at 1.5 spacing: 2 x 1.7 x 1.5 + 0.15 high, 2 x 1.3 + 0.15 wide. In the font
    // or in the effects; below 1 is not trusted (the pitch is never tighter than the font's).
    let tall = |spacing: &str| {
        only_obstacle(&gr_text(
            "AB\\nCD",
            at,
            "F.Cu",
            &format!("(effects (font (size 1 1) (thickness 0.15) (line_spacing {spacing})))"),
        ))
        .bbox
    };
    let (w, h) = (2.0 * 1.3 + 0.15, 2.0 * 1.7 * 1.5 + 0.15);
    assert_eq!(tall("1.5"), (-w / 2.0, -h / 2.0, w / 2.0, h / 2.0));
    let plain_h = 2.0 * 1.7 + 0.15;
    for ignored in ["0.5", "1", "inf", "nan"] {
        assert_eq!(
            tall(ignored),
            (-w / 2.0, -plain_h / 2.0, w / 2.0, plain_h / 2.0),
            "{ignored}"
        );
    }
    let in_effects = only_obstacle(&gr_text(
        "AB\\nCD",
        at,
        "F.Cu",
        "(effects (font (size 1 1) (thickness 0.15)) (line_spacing 1.5))",
    ));
    assert_eq!(in_effects.bbox, (-w / 2.0, -h / 2.0, w / 2.0, h / 2.0));
    // No readable size is 1.524 mm; a readable one is its own.
    let no_size = only_obstacle(&gr_text(
        "A",
        at,
        "F.Cu",
        "(effects (font (thickness 0.15)))",
    ));
    let (uw, uh) = (1.524 * 1.3 + 0.15, 1.524 * 1.7 + 0.15);
    assert!(close(
        no_size.bbox,
        (-uw / 2.0, -uh / 2.0, uw / 2.0, uh / 2.0)
    ));
    let broken = only_obstacle(&gr_text(
        "A",
        at,
        "F.Cu",
        "(effects (font (size x 1) (thickness 0.15)))",
    ));
    assert!(close(broken.bbox, no_size.bbox));
    // Knockout grows the box by max(thickness, size_y / 4) on every side: 0.25 here (size 1),
    // 0.5 with a pen of 0.5.
    let knockout = |effects: &str| {
        only_obstacle(&format!(
            "(gr_text \"A\" (at 0 0) (layer \"F.Cu\" knockout) {effects})"
        ))
    };
    let thin = knockout("(effects (font (size 1 1) (thickness 0.15)))");
    let (kw, kh) = (1.3 + 0.15, 1.7 + 0.15);
    assert!(close(
        thin.bbox,
        (
            -kw / 2.0 - 0.25,
            -kh / 2.0 - 0.25,
            kw / 2.0 + 0.25,
            kh / 2.0 + 0.25
        )
    ));
    assert_eq!(thin.sides, Sides::TOP);
    let thick = knockout("(effects (font (size 1 1) (thickness 0.5)))");
    let (tw, th) = (1.3 + 0.5, 1.7 + 0.5);
    assert!(close(
        thick.bbox,
        (
            -tw / 2.0 - 0.5,
            -th / 2.0 - 0.5,
            tw / 2.0 + 0.5,
            th / 2.0 + 0.5
        )
    ));
    // Not knockout, no margin: the layer's other words do not count.
    let other = only_obstacle(
        "(gr_text \"A\" (at 0 0) (layer \"F.Cu\" mirror) (effects (font (size 1 1) (thickness 0.15))))",
    );
    assert!(close(
        other.bbox,
        (-kw / 2.0, -kh / 2.0, kw / 2.0, kh / 2.0)
    ));
}

#[test]
fn a_polygon_box_includes_the_extent_of_its_arcs() {
    // The arc through (40, 30), (42, 35), (40, 40) about (35.25, 35) reaches x = 42, which
    // none of the polygon's corners does.
    let poly = |arc: &str| {
        format!(
            "(gr_poly (pts (xy 30 30) (xy 40 30) {arc} (xy 30 40)) (stroke (width 0.2) (type default)) (fill no) (layer \"F.Cu\"))"
        )
    };
    let found = only_obstacle(&poly("(arc (start 40 30) (mid 42 35) (end 40 40))"));
    assert!(
        close(found.bbox, (29.9, 29.9, 42.1, 40.1)),
        "{:?}",
        found.bbox
    );
    // An arc that is missing a point is unreadable: no obstacle, nothing guessed.
    let broken = copper_model(&poly("(arc (start 40 30) (end 40 40))"));
    assert!(broken.copper.is_empty());
    // The same arc in a rule area.
    let zone = "(zone (net 0) (net_name \"\") (layers \"F.Cu\") (name \"ra\") (hatch edge 0.5) \
                (keepout (tracks allowed) (vias allowed) (pads allowed) (copperpour allowed) (footprints not_allowed)) \
                (polygon (pts (xy 30 30) (xy 40 30) (arc (start 40 30) (mid 42 35) (end 40 40)) (xy 30 40))))";
    assert!(close(only_obstacle(zone).bbox, (30.0, 30.0, 42.0, 40.0)));
}

#[test]
fn a_near_straight_arc_is_a_thin_box_and_does_not_block_a_part_beside_it() {
    // 20 mm long and 0.01 mm high: the circle through its points is 5 m across, the arc is
    // a hair.
    let arc = "(gr_arc (start 30 40) (mid 40 40.01) (end 50 40) (stroke (width 0.1) (type default)) (layer \"F.Cu\"))";
    let found = only_obstacle(arc);
    assert!(
        found.bbox.0 > 29.9 && found.bbox.2 < 50.1,
        "{:?}",
        found.bbox
    );
    assert!(
        found.bbox.1 > 39.9 && found.bbox.3 < 40.1,
        "{:?}",
        found.bbox
    );
    let model = copper_model(arc);
    // R1 beside it, 1 mm below, stays; on it, it moves.
    assert_eq!(legalized_r1(&model, (40.0, 41.0)), (40.0, 41.0));
    assert_eq!(legalized_r1(&model, (40.0, 39.0)), (40.0, 39.0));
    assert_ne!(legalized_r1(&model, (40.0, 40.0)), (40.0, 40.0));
}

#[test]
fn a_copper_text_with_a_render_cache_is_boxed_by_the_cache() {
    // The estimate would be 37.325..42.675; the cache's two polygons say otherwise.
    let cache = "(render_cache \"ABCD\" 0 (polygon (pts (xy 38 39) (xy 41 39) (xy 41 41))) \
                 (polygon (pts (xy 36 40.5) (xy 37 42) (xy 39 40))))";
    let item = gr_text(
        "ABCD",
        "40 40",
        "F.Cu",
        &format!("(effects (font (size 1 1) (thickness 0.15))) {cache}"),
    );
    let cached = only_obstacle(&item);
    assert!(close(cached.bbox, (36.0, 39.0, 41.0, 42.0)), "{cached:?}");
    // A cache with no points falls back to the estimate.
    let empty = only_obstacle(&gr_text(
        "ABCD",
        "40 40",
        "F.Cu",
        &format!("{PLAIN} (render_cache \"ABCD\" 0)"),
    ));
    assert!(
        close(empty.bbox, (37.325, 39.075, 42.675, 40.925)),
        "{empty:?}"
    );
}

#[test]
fn copper_graphics_are_boxed_by_their_geometry_grown_by_half_the_stroke() {
    let stroke = |width: f64| format!("(stroke (width {width}) (type default))");
    let cases: Vec<(String, Rect)> = vec![
        (
            format!("(gr_line (start 30 30) (end 34 32) {} (layer \"F.Cu\"))", stroke(0.4)),
            (29.8, 29.8, 34.2, 32.2),
        ),
        (
            // The corners are given in the other order.
            format!("(gr_rect (start 24 28) (end 20 25) {} (fill no) (layer \"F.Cu\"))", stroke(0.2)),
            (19.9, 24.9, 24.1, 28.1),
        ),
        (
            format!("(gr_circle (center 50 40) (end 53 40) {} (fill no) (layer \"F.Cu\"))", stroke(0.2)),
            (46.9, 36.9, 53.1, 43.1),
        ),
        (
            format!(
                "(gr_poly (pts (xy 30 30) (xy 35 31) (xy 33 36)) {} (fill no) (layer \"F.Cu\"))",
                stroke(0.2)
            ),
            (29.9, 29.9, 35.1, 36.1),
        ),
        (
            format!(
                "(gr_curve (pts (xy 20 30) (xy 22 25) (xy 26 35) (xy 28 30)) {} (layer \"F.Cu\"))",
                stroke(0.2)
            ),
            (19.9, 24.9, 28.1, 35.1),
        ),
        (
            // The legacy spelling of the width.
            "(gr_line (start 30 30) (end 34 32) (width 0.4) (layer \"F.Cu\"))".to_string(),
            (29.8, 29.8, 34.2, 32.2),
        ),
        (
            // No width: the geometry itself.
            "(gr_line (start 30 30) (end 34 32) (layer \"F.Cu\"))".to_string(),
            (30.0, 30.0, 34.0, 32.0),
        ),
        (
            format!(
                "(gr_text_box \"ab\" (start 60 30) (end 64 33) (pts (xy 60 30) (xy 64 30) (xy 64 33) (xy 60 33)) {} (layer \"F.Cu\"))",
                stroke(0.2)
            ),
            (59.9, 29.9, 64.1, 33.1),
        ),
        (
            format!("(gr_text_box \"ab\" (start 60 30) (end 64 33) {} (layer \"F.Cu\"))", stroke(0.2)),
            (59.9, 29.9, 64.1, 33.1),
        ),
    ];
    for (item, expected) in cases {
        let found = only_obstacle(&item);
        assert!(close(found.bbox, expected), "{item}: {:?}", found.bbox);
        assert_eq!(found.sides, Sides::TOP, "{item}");
    }
}

#[test]
fn an_arc_box_covers_the_bulge() {
    // The circle about (40, 40) of radius 10; the arc runs from (50, 40) through
    // (47.071, 32.929) to (30, 40), over the top, so it reaches y = 30, which none of its
    // three points does.
    let mid = 40.0 + 10.0 * std::f64::consts::FRAC_1_SQRT_2;
    let arc = format!(
        "(gr_arc (start 50 40) (mid {mid} {}) (end 30 40) (stroke (width 0.1) (type default)) (layer \"F.Cu\"))",
        80.0 - mid
    );
    let found = only_obstacle(&arc);
    assert!(found.bbox.1 <= 30.0 - 0.05 + 1e-9, "{:?}", found.bbox);
    assert!(found.bbox.0 <= 30.0 - 0.05 + 1e-9 && found.bbox.2 >= 50.0 + 0.05 - 1e-9);
    // The exact extent of the arc, not of its whole circle: it stays above y = 40.
    assert!(
        close(found.bbox, (29.95, 29.95, 50.05, 40.05)),
        "{:?}",
        found.bbox
    );
    // Collinear points: the box of the points.
    let straight = only_obstacle(
        "(gr_arc (start 30 30) (mid 32 31) (end 34 32) (stroke (width 0.2) (type default)) (layer \"F.Cu\"))",
    );
    assert!(
        close(straight.bbox, (29.9, 29.9, 34.1, 32.1)),
        "{:?}",
        straight.bbox
    );
}

#[test]
fn copper_items_take_the_sides_of_their_layers() {
    let line = |layers: &str| format!("(gr_line (start 30 30) (end 34 30) {layers})");
    // The tree alone, so that a line on `Edge.Cuts` does not open the outline.
    let sides = |layers: &str| {
        let tree = parse(&format!("(kicad_pcb {})", line(layers)));
        copper_obstacles(&tree).first().map(|o| o.sides)
    };
    assert_eq!(sides("(layer \"F.Cu\")"), Some(Sides::TOP));
    assert_eq!(sides("(layer \"B.Cu\")"), Some(Sides::BOTTOM));
    assert_eq!(sides("(layer \"In1.Cu\")"), Some(Sides::BOTH));
    assert_eq!(sides("(layers \"F.Cu\" \"B.Cu\")"), Some(Sides::BOTH));
    assert_eq!(sides("(layers \"F&B.Cu\")"), Some(Sides::BOTH));
    assert_eq!(sides("(layers \"*.Cu\")"), Some(Sides::BOTH));
    assert_eq!(sides("(layers \"F.Cu\" \"F.Mask\")"), Some(Sides::TOP));
    assert_eq!(sides("(layers \"B.Cu\" \"B.Mask\")"), Some(Sides::BOTTOM));
    // Not copper: never an obstacle.
    for layer in [
        "F.SilkS",
        "F.Fab",
        "Edge.Cuts",
        "F.Mask",
        "F.CrtYd",
        "User.1",
    ] {
        assert_eq!(sides(&format!("(layer \"{layer}\")")), None, "{layer}");
    }
    assert_eq!(sides("(layers \"F.Mask\" \"F.Paste\")"), None);
}

#[test]
fn a_part_is_moved_off_copper_graphics_on_its_side() {
    // A bar of F.Cu line 0.4 wide across (40, 40): y 39.8..40.2, x 36..44.
    let bar =
        "(gr_line (start 36 40) (end 44 40) (stroke (width 0.4) (type default)) (layer \"F.Cu\"))";
    let model = copper_model(bar);
    let moved = legalized_r1(&model, (40.0, 40.0));
    assert_ne!(moved, (40.0, 40.0));
    assert!(gap_to(&model, moved, model.copper[0].bbox) >= MOVED_CLEARANCE_MM);
    // The same bar on the back does not touch a top part.
    let back = bar.replace("F.Cu", "B.Cu");
    assert_eq!(
        legalized_r1(&copper_model(&back), (40.0, 40.0)),
        (40.0, 40.0)
    );
}

fn keepout(layers: &str, forbids: &str) -> String {
    // `forbids` names the one kind the rule area keeps out; the others are allowed.
    let rule = |kind: &str| {
        format!(
            "({kind} {})",
            if kind == forbids {
                "not_allowed"
            } else {
                "allowed"
            }
        )
    };
    format!(
        "(zone (net 0) (net_name \"\") {layers} (name \"ra\") (hatch edge 0.5) \
         (keepout {} {} {} {} {}) (fill (thermal_gap 0.5) (thermal_bridge_width 0.5)) \
         (polygon (pts (xy 35 35) (xy 45 35) (xy 45 45) (xy 35 45))))",
        rule("tracks"),
        rule("vias"),
        rule("pads"),
        rule("copperpour"),
        rule("footprints")
    )
}

#[test]
fn a_rule_area_that_keeps_parts_out_blocks_a_moved_part() {
    for kind in ["footprints", "pads"] {
        let model = copper_model(&keepout("(layers \"F.Cu\")", kind));
        assert_eq!(model.copper.len(), 1, "{kind}");
        assert!(close(model.copper[0].bbox, (35.0, 35.0, 45.0, 45.0)));
        assert_eq!(model.copper[0].sides, Sides::TOP);
        let moved = legalized_r1(&model, (40.0, 40.0));
        // The edge is 5 mm above the centre; R1's box is 0.5 mm half high, so the first
        // cell above that clears it by 2 µm is 12 steps up (a cell at 34.5 would touch).
        assert_eq!(moved, (40.0, 34.0), "{kind}");
        // The same rule area on the back leaves a top part alone.
        let back = copper_model(&keepout("(layers \"B.Cu\")", kind));
        assert_eq!(legalized_r1(&back, (40.0, 40.0)), (40.0, 40.0), "{kind}");
        // The legacy single layer.
        let single = copper_model(&keepout("(layer \"F.Cu\")", kind));
        assert_eq!(single.copper.len(), 1, "{kind}");
    }
    // A rule area on both layers blocks a part on either.
    let both = copper_model(&keepout("(layers \"F&B.Cu\")", "footprints"));
    assert_eq!(both.copper[0].sides, Sides::BOTH);
}

#[test]
fn a_rule_area_that_keeps_out_only_tracks_vias_or_pours_is_no_obstacle() {
    for kind in ["tracks", "vias", "copperpour"] {
        let model = copper_model(&keepout("(layers \"F.Cu\")", kind));
        assert!(model.copper.is_empty(), "{kind}");
        assert_eq!(legalized_r1(&model, (40.0, 40.0)), (40.0, 40.0), "{kind}");
    }
    // A pour is no obstacle either: a zone without a keepout.
    let pour = "(zone (net 1) (net_name \"N1\") (layers \"F.Cu\") (name \"\") (hatch edge 0.5) \
                (fill yes) (polygon (pts (xy 35 35) (xy 45 35) (xy 45 45) (xy 35 45))))";
    assert!(copper_model(pour).copper.is_empty());
}

#[test]
fn a_rule_area_with_an_arc_is_boxed_by_the_arc_extent() {
    // A shape whose top edge bulges as an arc over a circle about (40, 40) of radius 10:
    // it reaches y = 30 but not y = 50.
    let mid = 40.0 + 10.0 * std::f64::consts::FRAC_1_SQRT_2;
    let zone = format!(
        "(zone (net 0) (net_name \"\") (layers \"F.Cu\") (name \"ra\") (hatch edge 0.5) \
         (keepout (tracks allowed) (vias allowed) (pads allowed) (copperpour allowed) (footprints not_allowed)) \
         (polygon (pts (xy 50 40) (arc (start 50 40) (mid {mid} {}) (end 30 40)) (xy 30 45) (xy 50 45))))",
        80.0 - mid
    );
    let found = only_obstacle(&zone);
    assert!(
        close(found.bbox, (30.0, 30.0, 50.0, 45.0)),
        "{:?}",
        found.bbox
    );
}

#[test]
fn a_through_hole_part_occupies_both_sides_and_another_its_own() {
    let parts = vec![
        Fp::new("R1", 20.0, 30.0).two_pad("N1", "N2"),
        Fp::new("R2", 25.0, 30.0).two_pad("N1", "N2").back(),
        Fp::new("R3", 30.0, 30.0).two_pad("N1", "N2"),
        Fp::new("R4", 35.0, 30.0).two_pad("N1", "N2").back(),
        Fp::new("R5", 40.0, 30.0).two_pad("N1", "N2"),
    ];
    let model = copper_model_of(parts, &["R3", "R4"], "");
    let sides = |reference: &str| {
        model.part_sides[model
            .references
            .binary_search(&reference.to_string())
            .unwrap()]
    };
    assert_eq!(sides("R1"), Sides::TOP);
    assert_eq!(sides("R2"), Sides::BOTTOM);
    assert_eq!(sides("R3"), Sides::BOTH);
    assert_eq!(sides("R4"), Sides::BOTH);
    assert_eq!(sides("R5"), Sides::TOP);
    // An unplated hole goes through the board as well.
    let text = board(
        Shape::Rect(10.0, 20.0, 70.0, 60.0),
        &[Fp::new("R1", 20.0, 30.0).two_pad("N1", "N2")],
    );
    let punched = text.replace(" smd rect", " np_thru_hole circle");
    assert_eq!(model_of(&punched, &[]).part_sides[0], Sides::BOTH);
}

#[test]
fn a_through_hole_part_and_a_bottom_part_that_overlap_are_not_both_legal() {
    // B1 on the back at (40, 40), 2 x 1 mm. R1 on top over it is fine, as two SMD parts
    // on opposite sides may overlap.
    let back = || Fp::new("B1", 40.0, 40.0).two_pad("N3", "N4").back();
    let smd = copper_model_of(
        vec![back(), Fp::new("R1", 20.0, 30.0).two_pad("N1", "N2")],
        &[],
        "",
    );
    assert_eq!(legalized_r1(&smd, (40.0, 40.0)), (40.0, 40.0));
    // R1 through-hole is on the back too: it is moved.
    let tht = copper_model_of(
        vec![back(), Fp::new("R1", 20.0, 30.0).two_pad("N1", "N2")],
        &["R1"],
        "",
    );
    let moved = legalized_r1(&tht, (40.0, 40.0));
    assert_ne!(moved, (40.0, 40.0));
    // A moved bottom part over a settled through-hole part is moved as well.
    let settled = copper_model_of(
        vec![
            Fp::new("T1", 40.0, 40.0).two_pad("N3", "N4"),
            Fp::new("R1", 20.0, 30.0).two_pad("N1", "N2").back(),
        ],
        &["T1"],
        "",
    );
    assert_ne!(legalized_r1(&settled, (40.0, 40.0)), (40.0, 40.0));
    // And two parts on the same side overlapping are still moved.
    let same = copper_model_of(
        vec![
            Fp::new("T1", 40.0, 40.0).two_pad("N3", "N4"),
            Fp::new("R1", 20.0, 30.0).two_pad("N1", "N2"),
        ],
        &[],
        "",
    );
    assert_ne!(legalized_r1(&same, (40.0, 40.0)), (40.0, 40.0));
}

#[test]
fn a_part_meets_copper_text_only_on_a_side_it_occupies() {
    let top_text = |layer: &str| gr_text("ABCD", "40 40", layer, PLAIN);
    // A top SMD part over B.Cu text: allowed.
    let model = copper_model(&top_text("B.Cu"));
    assert_eq!(legalized_r1(&model, (40.0, 40.0)), (40.0, 40.0));
    // A through-hole part over B.Cu text: blocked, and over F.Cu text too.
    for layer in ["B.Cu", "F.Cu", "In1.Cu"] {
        let model = copper_model_of(
            vec![Fp::new("R1", 20.0, 30.0).two_pad("N1", "N2")],
            &["R1"],
            &top_text(layer),
        );
        assert_ne!(legalized_r1(&model, (40.0, 40.0)), (40.0, 40.0), "{layer}");
    }
    // A bottom part: blocked by B.Cu, not by F.Cu; an inner layer blocks both.
    for (layer, blocked) in [("B.Cu", true), ("F.Cu", false), ("In1.Cu", true)] {
        let model = copper_model_of(
            vec![Fp::new("R1", 20.0, 30.0).two_pad("N1", "N2").back()],
            &[],
            &top_text(layer),
        );
        let stays = legalized_r1(&model, (40.0, 40.0)) == (40.0, 40.0);
        assert_eq!(stays, !blocked, "{layer}");
    }
    // A top part under an inner-layer text is blocked.
    let inner = copper_model(&top_text("In2.Cu"));
    assert_ne!(legalized_r1(&inner, (40.0, 40.0)), (40.0, 40.0));
}

#[test]
fn an_unmoved_part_on_copper_text_stays_with_no_room_trouble() {
    // R1 sits on the text in the input and does not move: legal as it is. R2 moves from
    // somewhere else onto the same text, and is moved off it. The text is 15.75 x 5.25 mm
    // (size 3): without the obstacle test R2 would stop 3 steps above R1, still on the
    // text; clear of the text it is 7 steps up.
    let parts = vec![
        Fp::new("R1", 40.0, 40.0).two_pad("N1", "N2"),
        Fp::new("R2", 20.0, 30.0).two_pad("N2", "N3"),
    ];
    let big = "(effects (font (size 3 3) (thickness 0.15)))";
    let model = copper_model_of(parts, &[], &gr_text("ABCD", "40 40", "F.Cu", big));
    let sizes = as_read_sizes(&model);
    let read = as_read_centres(&model);
    let mut centres = read.clone();
    legalize(&model, &sizes, &mut centres).expect("an input as it is needs no room");
    assert_eq!(centres, read);
    // R2 dropped on the text and on R1 lands off both.
    let mut centres = read.clone();
    centres[1] = (40.0, 40.0);
    legalize(&model, &sizes, &mut centres).expect("R2 finds a cell");
    assert_eq!(centres[0], read[0], "R1 stays");
    assert_eq!(centres[1], (40.0, 36.5));
    assert!(gap_to_part(&model, &sizes, &centres) >= MOVED_CLEARANCE_MM - 1e-9);
    assert!(gap_to_text(&model, &sizes, centres[1]) >= MOVED_CLEARANCE_MM - 1e-9);
}

/// The gap between R2's box and R1's, from `centres`.
fn gap_to_part(_model: &BoardModel, sizes: &[(f64, f64)], centres: &[(f64, f64)]) -> f64 {
    gap_between_parts(sizes, centres, 0, 1)
}

/// The gap between a part of the size of R2's and the first obstacle's box.
fn gap_to_text(model: &BoardModel, sizes: &[(f64, f64)], at: (f64, f64)) -> f64 {
    let bbox = model.copper[0].bbox;
    let (w, h) = sizes[1];
    let gap_x = (at.0 - (bbox.0 + bbox.2) / 2.0).abs() - (w + bbox.2 - bbox.0) / 2.0;
    let gap_y = (at.1 - (bbox.1 + bbox.3) / 2.0).abs() - (h + bbox.3 - bbox.1) / 2.0;
    gap_x.max(gap_y)
}

#[test]
fn the_generator_version_is_eight() {
    // The obstacles, the part sides, the exact boxes and the largest-first order change what
    // every recipe gives, so a record of an earlier version is not reproducible.
    assert_eq!(GENERATOR_VERSION, 8);
}

// ─── Largest first ───────────────────────────────────────────────────────────

/// The pinned outline (x 10..70, y 20..60) with `parts`, each `(reference, half width, half
/// height)` read far from (40, 40); the centres of all of them are then put at `at`.
fn dropped_parts(parts: &[(&str, f64, f64)], at: (f64, f64)) -> (BoardModel, Vec<(f64, f64)>) {
    let fps: Vec<Fp> = parts
        .iter()
        .enumerate()
        .map(|(k, (reference, hw, hh))| {
            Fp::new(reference, 12.0 + 3.0 * k as f64, 22.0)
                .court(*hw, *hh)
                .two_pad("N1", "N2")
        })
        .collect();
    let model = model_of(&board(Shape::Rect(10.0, 20.0, 70.0, 60.0), &fps), &[]);
    let centres = vec![at; model.parts.len()];
    (model, centres)
}

fn index_of(model: &BoardModel, reference: &str) -> usize {
    model
        .references
        .binary_search(&reference.to_string())
        .unwrap()
}

#[test]
fn a_big_part_fits_because_it_settles_before_the_small_one() {
    // The board is 60 x 40 mm. Z1's courtyard is 38 x 38: it fits only where nothing small
    // sits in the middle. A1 (2 x 1 mm) is first by reference, and both are dropped on the
    // centre. In reference order A1 would stay there and Z1 would find no room at all.
    let (model, mut centres) = dropped_parts(&[("A1", 1.0, 0.5), ("Z1", 19.0, 19.0)], (40.0, 40.0));
    let sizes = as_read_sizes(&model);
    legalize(&model, &sizes, &mut centres).expect("the big part settles first");
    let (a1, z1) = (index_of(&model, "A1"), index_of(&model, "Z1"));
    assert_eq!(centres[z1], (40.0, 40.0), "the big part keeps the centre");
    // The small one goes to the nearest free cell: 20.5 mm to the left (41 steps), the
    // left cell before the right one.
    assert_eq!(centres[a1], (19.5, 40.0));
    assert!(gap_between_parts(&sizes, &centres, a1, z1) >= MOVED_CLEARANCE_MM - 1e-9);
}

#[test]
fn parts_settle_by_decreasing_area_and_equal_areas_by_reference() {
    // Three parts dropped on one cell. The largest keeps it, the next takes the nearest
    // cell that is free of it, the smallest the nearest free of both: not in reference order.
    let (model, mut centres) = dropped_parts(
        &[("A1", 1.0, 0.5), ("B1", 2.0, 1.0), ("C1", 3.0, 1.5)],
        (40.0, 40.0),
    );
    let sizes = as_read_sizes(&model);
    legalize(&model, &sizes, &mut centres).unwrap();
    let at = |r: &str| centres[index_of(&model, r)];
    assert_eq!(at("C1"), (40.0, 40.0), "the largest settles first");
    // B1 (4 x 2) clears C1 (6 x 3) by 2.502 mm in y: 3 mm, up before down.
    assert_eq!(at("B1"), (40.0, 37.0));
    for (a, b) in [("A1", "B1"), ("A1", "C1"), ("B1", "C1")] {
        let gap = gap_between_parts(&sizes, &centres, index_of(&model, a), index_of(&model, b));
        assert!(gap >= MOVED_CLEARANCE_MM - 1e-9, "{a} {b} {gap}");
    }
    // Equal areas, different shapes: the smaller reference first. B1 4 x 2 and C1 2 x 4 both
    // 8 mm2; B1 keeps the cell.
    let (model, mut centres) = dropped_parts(&[("B1", 2.0, 1.0), ("C1", 1.0, 2.0)], (40.0, 40.0));
    let sizes = as_read_sizes(&model);
    legalize(&model, &sizes, &mut centres).unwrap();
    assert_eq!(centres[index_of(&model, "B1")], (40.0, 40.0));
    assert_ne!(centres[index_of(&model, "C1")], (40.0, 40.0));
}

#[test]
fn held_and_unmoved_parts_still_settle_first_whatever_their_size() {
    // J1 is held (a connector) and 2 x 1 mm; A1 is movable and does not move, 2 x 1 mm.
    // Z1 is moved onto each of them and is the largest part: it moves off them, they stay.
    let parts = vec![
        Fp::new("J1", 30.0, 30.0)
            .court(1.0, 0.5)
            .pad(pad("1", 0.0, 0.0, Some("N1"))),
        Fp::new("A1", 50.0, 30.0)
            .court(1.0, 0.5)
            .two_pad("N1", "N2"),
        Fp::new("Z1", 15.0, 50.0)
            .court(4.0, 3.0)
            .two_pad("N2", "N3"),
    ];
    let model = model_of(&board(Shape::Rect(10.0, 20.0, 70.0, 60.0), &parts), &[]);
    let (j1, a1, z1) = (
        index_of(&model, "J1"),
        index_of(&model, "A1"),
        index_of(&model, "Z1"),
    );
    assert!(!model.movable[j1] && model.movable[a1]);
    let sizes = as_read_sizes(&model);
    let read = as_read_centres(&model);
    // Z1 dropped between them, overlapping both.
    let mut centres = read.clone();
    centres[z1] = (40.0, 30.0);
    legalize(&model, &sizes, &mut centres).unwrap();
    assert_eq!(centres[j1], read[j1]);
    assert_eq!(centres[a1], read[a1]);
    assert_eq!(centres[z1], (40.0, 30.0), "clear of both, so it stays");
    centres[z1] = (30.0, 30.0);
    legalize(&model, &sizes, &mut centres).unwrap();
    assert_eq!((centres[j1], centres[a1]), (read[j1], read[a1]));
    assert_ne!(
        centres[z1],
        (30.0, 30.0),
        "dropped on the held J1, Z1 moves"
    );
    assert!(gap_between_parts(&sizes, &centres, z1, j1) >= MOVED_CLEARANCE_MM - 1e-9);
    centres[z1] = (50.0, 30.0);
    legalize(&model, &sizes, &mut centres).unwrap();
    assert_eq!((centres[j1], centres[a1]), (read[j1], read[a1]));
    assert_ne!(
        centres[z1],
        (50.0, 30.0),
        "dropped on the unmoved A1, Z1 moves"
    );
    assert!(gap_between_parts(&sizes, &centres, z1, a1) >= MOVED_CLEARANCE_MM - 1e-9);
}

#[test]
fn the_order_is_by_area_not_by_perimeter() {
    // P1 is 10 x 1 mm (area 10, sides sum 11), Q1 is 4 x 4 mm (area 16, sides sum 8). Both
    // are dropped on one cell: the larger area, Q1, keeps it, though P1 is first by
    // reference and by the sum of its sides.
    let (model, mut centres) = dropped_parts(&[("P1", 5.0, 0.5), ("Q1", 2.0, 2.0)], (40.0, 40.0));
    let sizes = as_read_sizes(&model);
    legalize(&model, &sizes, &mut centres).unwrap();
    assert_eq!(centres[index_of(&model, "Q1")], (40.0, 40.0));
    assert_ne!(centres[index_of(&model, "P1")], (40.0, 40.0));
}

// ─── Decoupling caps settle after their IC, from its pad ─────────────────────

fn pair_of(cap: &str, cap_pad: &str, ic: &str, ic_pad: &str, limit_mm: f64) -> DecouplingPair {
    DecouplingPair {
        cap: cap.to_string(),
        cap_pad: cap_pad.to_string(),
        ic: ic.to_string(),
        ic_pad: ic_pad.to_string(),
        net: "VCC".to_string(),
        limit_mm,
        basis: "test".to_string(),
    }
}

/// The pinned board with `parts` and `pairs` as its intent.
fn decoupling_model(parts: Vec<Fp>, pairs: Vec<DecouplingPair>) -> BoardModel {
    let mut model = model_of(&board(Shape::Rect(10.0, 20.0, 70.0, 60.0), &parts), &[]);
    model.intent = Intent::new(Vec::new(), pairs);
    model
}

/// U1, a 4 x 4 mm part at (40, 40), held by a lock, with pad "1" on its left side at
/// (38.2, 40).
fn ic() -> Fp {
    Fp::new("U1", 40.0, 40.0)
        .court(2.0, 2.0)
        .locked()
        .pad(pad("1", -1.8, 0.0, Some("VCC")))
}

/// A 2 x 1 mm capacitor read at `at`: pad "1" at the left, "2" at the right.
fn cap(reference: &str, at: (f64, f64)) -> Fp {
    Fp::new(reference, at.0, at.1).two_pad("VCC", "GND")
}

/// The distance between the pads of `reference`'s pair, the parts at `centres`.
fn pad_distance(model: &BoardModel, reference: &str, centres: &[(f64, f64)]) -> f64 {
    let pair = measured_pairs(model)
        .into_iter()
        .find(|p| p.cap == index_of(model, reference))
        .expect("a measured pair");
    let (c, i) = (centres[pair.cap], centres[pair.ic]);
    let own = (c.0 + pair.cap_offset.0, c.1 + pair.cap_offset.1);
    let ic_at = (i.0 + pair.ic_offset.0, i.1 + pair.ic_offset.1);
    (own.0 - ic_at.0).hypot(own.1 - ic_at.1)
}

/// A crowd of big parts, all read far away, and two caps of U1.
fn crowded_ic() -> BoardModel {
    let mut parts = vec![ic(), cap("C1", (60.0, 50.0)), cap("C2", (60.0, 52.0))];
    for k in 0..4 {
        parts.push(
            Fp::new(&format!("B{k}"), 20.0 + 5.0 * k as f64, 55.0)
                .court(2.0, 1.0)
                .two_pad("N1", "N2"),
        );
    }
    decoupling_model(
        parts,
        vec![
            pair_of("C1", "2", "U1", "1", 4.0),
            pair_of("C2", "2", "U1", "1", 4.0),
        ],
    )
}

/// Everything movable dropped where a designer would want the caps.
fn dropped_on_the_pin(model: &BoardModel) -> (Vec<(f64, f64)>, Vec<(f64, f64)>) {
    let sizes = as_read_sizes(model);
    let mut centres = as_read_centres(model);
    for (i, centre) in centres.iter_mut().enumerate() {
        if model.movable[i] {
            *centre = (34.5, 40.0);
        }
    }
    (sizes, centres)
}

#[test]
fn an_ics_caps_settle_right_after_it_and_land_within_their_limit() {
    let model = crowded_ic();
    let (sizes, mut centres) = dropped_on_the_pin(&model);
    legalize(&model, &sizes, &mut centres).expect("room for all");
    for reference in ["C1", "C2"] {
        let d = pad_distance(&model, reference, &centres);
        assert!(d <= 4.0, "{reference} is {d} mm from the pin");
    }
    // Where the pad is: C1 takes the cell 1.5 mm left of the pin's column, C2 the next.
    assert_eq!(centres[index_of(&model, "C1")], (36.5, 40.0));
    assert_eq!(centres[index_of(&model, "C2")], (36.5, 38.5));
    // The old order settled the big parts first: with no pair known, at least one cap is out.
    let mut plain = model.clone();
    plain.intent = Intent::new(Vec::new(), Vec::new());
    let (sizes, mut centres) = dropped_on_the_pin(&plain);
    legalize(&plain, &sizes, &mut centres).expect("room for all");
    let out = ["C1", "C2"]
        .iter()
        .filter(|r| pad_distance(&model, r, &centres) > 4.0)
        .count();
    assert!(out >= 1, "the unpaired order left both caps in range");
}

#[test]
fn a_caps_search_starts_at_the_ic_pad_not_at_its_own_centre() {
    // C1 is read, and proposed, far away at (60, 50), where it is legal. It still lands
    // 1.5 mm left of the pin: the cell 3 steps out whose pad is nearest the IC pad.
    let model = decoupling_model(
        vec![ic(), cap("C1", (60.0, 50.0))],
        vec![pair_of("C1", "2", "U1", "1", 2.0)],
    );
    let sizes = as_read_sizes(&model);
    let mut centres = as_read_centres(&model);
    legalize(&model, &sizes, &mut centres).unwrap();
    // The cap did not move in the input, so it is unmoved and stays: drop it elsewhere first.
    assert_eq!(centres[index_of(&model, "C1")], (60.0, 50.0));
    let mut centres = as_read_centres(&model);
    centres[index_of(&model, "C1")] = (60.0, 51.0);
    legalize(&model, &sizes, &mut centres).unwrap();
    let c1 = centres[index_of(&model, "C1")];
    assert_eq!(c1, (36.5, 40.0));
    assert!((pad_distance(&model, "C1", &centres) - 1.2).abs() < 1e-9);
    // Without the pair it keeps the legal cell it was dropped on.
    let mut plain = model.clone();
    plain.intent = Intent::new(Vec::new(), Vec::new());
    let mut centres = as_read_centres(&plain);
    centres[index_of(&plain, "C1")] = (60.0, 51.0);
    legalize(&plain, &sizes, &mut centres).unwrap();
    assert_eq!(centres[index_of(&plain, "C1")], (60.0, 51.0));
}

#[test]
fn a_cap_without_a_measured_pair_keeps_its_place() {
    // The pair names an IC that is not on the board, a pad that does not exist, and a pad
    // that is on the part twice: none can be measured, so C1, C2 and C3 stay where dropped.
    let parts = vec![
        ic(),
        cap("C1", (60.0, 50.0)),
        cap("C2", (60.0, 53.0)),
        cap("C3", (60.0, 56.0)).pad(pad("2", 0.0, 0.3, Some("GND"))),
    ];
    let model = decoupling_model(
        parts,
        vec![
            pair_of("C1", "2", "U9", "1", 2.0),
            pair_of("C2", "9", "U1", "1", 2.0),
            pair_of("C3", "2", "U1", "1", 2.0),
        ],
    );
    assert!(measured_pairs(&model).is_empty());
    let sizes = as_read_sizes(&model);
    let mut centres = as_read_centres(&model);
    let dropped = [(60.0, 51.0), (60.0, 54.0), (60.0, 57.0)];
    for (reference, at) in ["C1", "C2", "C3"].iter().zip(dropped) {
        centres[index_of(&model, reference)] = at;
    }
    legalize(&model, &sizes, &mut centres).unwrap();
    for (reference, at) in ["C1", "C2", "C3"].iter().zip(dropped) {
        assert_eq!(centres[index_of(&model, reference)], at, "{reference}");
    }
}

#[test]
fn the_caps_of_a_moved_ic_land_by_its_final_pad() {
    // U2 moves (dropped on the left of the board), and the big parts settle before it. Its
    // caps follow it and end within 2 mm of the pad where it finally is.
    let u2 = Fp::new("U2", 50.0, 30.0)
        .court(2.0, 2.0)
        .pad(pad("1", -1.8, 0.0, Some("VCC")));
    let mut parts = vec![u2, cap("C1", (15.0, 55.0)), cap("C2", (20.0, 55.0))];
    for k in 0..3 {
        parts.push(
            Fp::new(&format!("B{k}"), 12.0 + 5.0 * k as f64, 22.0)
                .court(3.0, 2.0)
                .two_pad("N1", "N2"),
        );
    }
    let model = decoupling_model(
        parts,
        vec![
            pair_of("C1", "2", "U2", "1", 2.0),
            pair_of("C2", "2", "U2", "1", 2.0),
        ],
    );
    let sizes = as_read_sizes(&model);
    let mut centres = as_read_centres(&model);
    for (i, centre) in centres.iter_mut().enumerate() {
        if model.movable[i] {
            *centre = (30.0, 40.0);
        }
    }
    legalize(&model, &sizes, &mut centres).unwrap();
    for reference in ["C1", "C2"] {
        let d = pad_distance(&model, reference, &centres);
        assert!(d <= 2.0, "{reference} is {d} mm from the pad of U2");
    }
}

#[test]
fn unmoved_and_held_parts_stay_where_a_cap_search_runs() {
    // U1 is held, U3 is movable and unmoved; C3 is an unmoved cap of U1. C1 (U1) and C4 (U3)
    // are dropped far away: they come to their pins, and nothing else moves.
    let u3 = Fp::new("U3", 20.0, 30.0)
        .court(2.0, 2.0)
        .pad(pad("1", 1.8, 0.0, Some("VCC")));
    let parts = vec![
        ic(),
        u3,
        cap("C1", (60.0, 50.0)),
        cap("C3", (36.5, 40.0)),
        cap("C4", (60.0, 55.0)),
    ];
    let model = decoupling_model(
        parts,
        vec![
            pair_of("C1", "2", "U1", "1", 2.0),
            pair_of("C3", "2", "U1", "1", 2.0),
            pair_of("C4", "1", "U3", "1", 2.0),
        ],
    );
    let sizes = as_read_sizes(&model);
    let read = as_read_centres(&model);
    let mut centres = read.clone();
    centres[index_of(&model, "C1")] = (60.0, 51.0);
    centres[index_of(&model, "C4")] = (60.0, 56.0);
    legalize(&model, &sizes, &mut centres).unwrap();
    for reference in ["U1", "U3", "C3"] {
        let i = index_of(&model, reference);
        assert_eq!(centres[i], read[i], "{reference}");
    }
    for reference in ["C1", "C4"] {
        let d = pad_distance(&model, reference, &centres);
        assert!(d <= 2.0, "{reference} {d}");
    }
    // C1 does not take C3's cell: it lands clear of it.
    let (c1, c3) = (index_of(&model, "C1"), index_of(&model, "C3"));
    assert!(gap_between_parts(&sizes, &centres, c1, c3) >= MOVED_CLEARANCE_MM - 1e-9);
    // And the whole thing is the same twice.
    let mut again = read.clone();
    again[index_of(&model, "C1")] = (60.0, 51.0);
    again[index_of(&model, "C4")] = (60.0, 56.0);
    legalize(&model, &sizes, &mut again).unwrap();
    assert_eq!(again, centres);
}

#[test]
fn a_turned_cap_measures_its_pad_from_the_turned_offset() {
    // C1 turned a quarter: 1 x 2 mm, its right pad (0.5, 0) now 0.5 mm above the centre.
    // Left of the pin, the cell whose pad is nearest the IC pad is 0.5 mm below the pin's
    // row, which puts the pad on it: (37, 40.5).
    let model = decoupling_model(
        vec![ic(), cap("C1", (60.0, 50.0))],
        vec![pair_of("C1", "2", "U1", "1", 2.0)],
    );
    let mut sizes = as_read_sizes(&model);
    let c1 = index_of(&model, "C1");
    sizes[c1] = (sizes[c1].1, sizes[c1].0);
    let mut turns = vec![0; model.parts.len()];
    turns[c1] = 1;
    let mut centres = as_read_centres(&model);
    centres[c1] = (60.0, 51.0);
    legalize_turned(&model, &sizes, &turns, &mut centres).unwrap();
    assert_eq!(centres[c1], (37.0, 40.5));
    assert!(matches!(
        legalize_turned(&model, &sizes, &[0], &mut centres),
        Err(GenerateFailure::Invalid(_))
    ));
}

fn measured(cap: usize, ic: usize) -> MeasuredPair {
    MeasuredPair {
        cap,
        ic,
        cap_offset: (0.0, 0.0),
        ic_offset: (0.0, 0.0),
    }
}

fn parts_of(order: &[(usize, Option<usize>)]) -> Vec<usize> {
    order.iter().map(|(part, _)| *part).collect()
}

#[test]
fn the_settling_order_puts_a_cap_after_its_ic_and_the_caps_of_a_fixed_ic_first() {
    // Moved parts largest first: 5, 4, 3, 2, 1, 0. Caps 1 and 0 belong to the moved 4;
    // cap 3 belongs to 9, which is not moved; 7 is no cap and no IC.
    let largest = [5, 4, 3, 2, 1, 0];
    let pairs = [measured(0, 4), measured(3, 9), measured(1, 4)];
    let order = settling_order(&largest, &pairs);
    // 3 first (its IC is fixed), then 5, then 4 with its caps in pair order (0, then 1), then 2.
    assert_eq!(parts_of(&order), vec![3, 5, 4, 0, 1, 2]);
    // The pair index rides along for the caps.
    assert_eq!(
        order,
        vec![
            (3, Some(1)),
            (5, None),
            (4, None),
            (0, Some(0)),
            (1, Some(2)),
            (2, None)
        ]
    );
    // A cap in two pairs uses the first; a part with no pair keeps its place.
    let pairs = [measured(1, 5), measured(1, 4)];
    assert_eq!(parts_of(&settling_order(&[5, 4, 1], &pairs)), vec![5, 1, 4]);
    // Caps of a fixed IC in pair order, whatever their sizes: 0 is the largest.
    let pairs = [measured(2, 9), measured(0, 9)];
    assert_eq!(parts_of(&settling_order(&[3, 0, 2], &pairs)), vec![2, 0, 3]);
    // A pair whose cap is not moved is no cap here.
    let pairs = [measured(8, 4)];
    assert_eq!(parts_of(&settling_order(&[4, 1], &pairs)), vec![4, 1]);
    // A loop of pairs leaves no part out and none twice.
    let pairs = [measured(1, 2), measured(2, 1)];
    let looped = parts_of(&settling_order(&[1, 2, 3], &pairs));
    assert_eq!(looped.len(), 3);
    assert!(looped.contains(&1) && looped.contains(&2) && looped.contains(&3));
    // A chain: the cap of a cap follows it.
    let pairs = [measured(2, 1), measured(3, 2)];
    assert_eq!(
        parts_of(&settling_order(&[4, 1, 2, 3], &pairs)),
        vec![4, 1, 2, 3]
    );
}
