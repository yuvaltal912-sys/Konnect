//! The force core on its own: its constants, its determinism, the forces the tool never
//! had, the spring graph and the collision pass. That the tool's output did not change
//! is `force_directed_output_is_unchanged_by_the_pull_out`'s job, in `placement.rs`.
use super::force_core::*;
use konnect_sexp::board::{PcbConnectivityIndex, Side};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../konnect-sexp/tests/fixtures/placement/placement_fixture.kicad_pcb"
);
const BOUNDS: Rect = (0.0, 0.0, 60.0, 45.0);

fn part(size: (f64, f64), movable: bool) -> FdPart {
    FdPart {
        size,
        side: Side::Front,
        movable,
    }
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

#[test]
fn the_legacy_params_are_the_tools_constants() {
    let params = FdParams::legacy(123);
    assert_eq!(
        params,
        FdParams {
            k_spring: 0.4,
            k_repel: 80.0,
            k_wall: 5.0,
            damping: 0.85,
            min_dist_mm: 0.5,
            grid_mm: 0.5,
            patience: 4,
            iterations: 123,
            step_share: 0.05,
            temperature_floor_mm: 0.1,
            clear_ring_mm: 1.0,
            ring_end: 200,
        }
    );
}

#[test]
fn relax_is_deterministic_and_an_immovable_part_keeps_its_start() {
    let parts = [
        part((4.0, 4.0), true),
        part((4.0, 4.0), false),
        part((2.0, 2.0), true),
    ];
    let start = [(20.0, 20.0), (30.0, 22.0), (31.0, 21.0)];
    let springs = Springs::from([((0, 2), 3.0), ((1, 2), 1.0)]);
    let params = FdParams::legacy(300);
    let run = || {
        relax(
            &parts,
            &springs,
            &FdExtras::default(),
            BOUNDS,
            &start,
            &params,
        )
    };
    let (a, b) = (run(), run());
    assert_eq!(a, b);
    assert_eq!(a.centres[1], start[1], "the immovable part never moves");
    assert!(a.iterations_run >= 1 && a.iterations_run <= 300);
    assert_ne!(a.centres[0], start[0], "a movable part does");
}

#[test]
fn relax_with_no_iterations_changes_nothing() {
    let parts = [part((2.0, 2.0), true)];
    let relaxed = relax(
        &parts,
        &Springs::new(),
        &FdExtras::default(),
        BOUNDS,
        &[(10.0, 10.0)],
        &FdParams::legacy(0),
    );
    assert_eq!(relaxed.centres, vec![(10.0, 10.0)]);
    assert_eq!((relaxed.iterations_run, relaxed.converged), (0, false));
}

#[test]
#[should_panic(expected = "one start centre per part")]
fn relax_panics_when_the_start_does_not_match_the_parts() {
    let parts = [part((2.0, 2.0), true), part((2.0, 2.0), true)];
    relax(
        &parts,
        &Springs::new(),
        &FdExtras::default(),
        BOUNDS,
        &[(10.0, 10.0)],
        &FdParams::legacy(10),
    );
}

#[test]
fn a_pad_spring_pulls_the_two_pads_together_and_none_changes_nothing() {
    let parts = [part((2.0, 2.0), true), part((2.0, 2.0), true)];
    let start = [(20.0, 22.0), (40.0, 22.0)];
    let params = FdParams::legacy(300);
    let (a_offset, b_offset) = ((1.0, 0.0), (-1.0, 0.0));
    let pad_gap = |centres: &[(f64, f64)]| {
        distance(
            (centres[0].0 + a_offset.0, centres[0].1 + a_offset.1),
            (centres[1].0 + b_offset.0, centres[1].1 + b_offset.1),
        )
    };
    let plain = relax(
        &parts,
        &Springs::new(),
        &FdExtras::default(),
        BOUNDS,
        &start,
        &params,
    );
    let extras = FdExtras {
        pads: vec![PadSpring {
            a: 0,
            b: 1,
            a_offset,
            b_offset,
            weight: 10.0,
        }],
        groups: vec![],
    };
    let pulled = relax(&parts, &Springs::new(), &extras, BOUNDS, &start, &params);
    assert!(
        pad_gap(&pulled.centres) < pad_gap(&plain.centres),
        "{} against {}",
        pad_gap(&pulled.centres),
        pad_gap(&plain.centres)
    );
    assert!(pad_gap(&pulled.centres) < pad_gap(&start));
    // Extras that are present but empty add no operation.
    let empty = FdExtras {
        pads: vec![],
        groups: vec![],
    };
    assert_eq!(
        relax(&parts, &Springs::new(), &empty, BOUNDS, &start, &params),
        plain
    );
}

#[test]
fn a_group_spring_pulls_its_members_toward_their_mean() {
    let parts = [part((2.0, 2.0), true); 4];
    let start = [(10.0, 10.0), (50.0, 10.0), (10.0, 35.0), (50.0, 35.0)];
    let params = FdParams::legacy(300);
    let spread = |centres: &[(f64, f64)]| {
        let mut widest = 0.0f64;
        for (i, a) in centres.iter().enumerate() {
            for b in &centres[i + 1..] {
                widest = widest.max(distance(*a, *b));
            }
        }
        widest
    };
    let plain = relax(
        &parts,
        &Springs::new(),
        &FdExtras::default(),
        BOUNDS,
        &start,
        &params,
    );
    let extras = FdExtras {
        pads: vec![],
        groups: vec![GroupSpring {
            members: vec![0, 1, 2, 3],
            weight: 0.5,
        }],
    };
    let grouped = relax(&parts, &Springs::new(), &extras, BOUNDS, &start, &params);
    assert!(spread(&grouped.centres) < spread(&plain.centres));
    // A group of nobody is no force.
    let nobody = FdExtras {
        pads: vec![],
        groups: vec![GroupSpring {
            members: vec![],
            weight: 9.0,
        }],
    };
    assert_eq!(
        relax(&parts, &Springs::new(), &nobody, BOUNDS, &start, &params),
        plain
    );
}

#[test]
fn springs_join_the_parts_on_a_net_by_its_weight_and_ignore_other_references() {
    let content = std::fs::read_to_string(FIXTURE).unwrap();
    let tree = konnect_sexp::parse_sexp(&content).unwrap();
    let index = PcbConnectivityIndex::build(&tree);
    let mut references: Vec<&str> = vec!["C1", "C2", "J1", "R1", "TP1", "TP2", "U1", "U2"];
    references.sort_unstable();
    let on_vcc: std::collections::BTreeSet<&str> = index
        .pads_of_net("VCC")
        .iter()
        .map(|pad| pad.reference.as_str())
        .collect();
    assert!(on_vcc.len() >= 3, "the fixture's VCC joins several parts");

    let only_vcc = |net: &str| if net == "VCC" { 2.0 } else { 0.0 };
    let graph = springs(&index, &references, &only_vcc);
    let position = |reference: &str| references.iter().position(|r| *r == reference).unwrap();
    let expected: Vec<(usize, usize)> = {
        let mut members: Vec<usize> = on_vcc.iter().map(|r| position(r)).collect();
        members.sort_unstable();
        let mut pairs = vec![];
        for (i, &a) in members.iter().enumerate() {
            for &b in &members[i + 1..] {
                pairs.push((a, b));
            }
        }
        pairs
    };
    assert_eq!(graph.keys().copied().collect::<Vec<_>>(), expected);
    assert!(graph.values().all(|weight| *weight == 2.0));

    // Weights add over the nets two parts share: C1 and U1 share VCC and GND.
    let both = springs(&index, &references, &|net: &str| match net {
        "VCC" => 2.0,
        "GND" => 3.0,
        _ => 0.0,
    });
    assert_eq!(both[&(position("C1"), position("U1"))], 5.0);

    // A pad whose reference the caller left out is ignored, not an error.
    let without_u1: Vec<&str> = references.iter().copied().filter(|r| *r != "U1").collect();
    let fewer = springs(&index, &without_u1, &only_vcc);
    let remaining = on_vcc.iter().filter(|r| **r != "U1").count();
    assert_eq!(fewer.len(), remaining * (remaining - 1) / 2);

    // A weight of zero or less makes no spring.
    assert!(springs(&index, &references, &|_: &str| 0.0).is_empty());
    assert!(springs(&index, &references, &|_: &str| -1.0).is_empty());
}

#[test]
fn collisions_move_an_overlapping_part_to_a_free_cell_and_leave_the_obstacle() {
    let parts = [
        part((4.0, 4.0), false),
        part((4.0, 4.0), true),
        FdPart {
            side: Side::Back,
            ..part((4.0, 4.0), true)
        },
    ];
    let mut centres = vec![(10.0, 10.0), (10.2, 10.1), (10.2, 10.1)];
    let params = FdParams::legacy(1);
    resolve_collisions(&parts, BOUNDS, &mut centres, &params);
    assert_eq!(centres[0], (10.0, 10.0), "the obstacle stays");
    assert!(
        (centres[1].0 - centres[0].0).abs() >= 4.0 || (centres[1].1 - centres[0].1).abs() >= 4.0,
        "no longer overlaps: {centres:?}"
    );
    assert_eq!(centres[1].0 % 0.5, 0.0, "on the grid");
    assert_eq!(
        centres[2],
        (10.0, 10.0),
        "the other side is no obstacle: snapped only"
    );
}

#[test]
fn a_part_with_no_free_cell_stays_where_it_snapped() {
    let parts = [part((4.0, 4.0), false), part((4.0, 4.0), true)];
    let mut centres = vec![(10.0, 10.0), (10.2, 10.1)];
    let mut params = FdParams::legacy(1);
    params.ring_end = 1; // no ring at all
    resolve_collisions(&parts, BOUNDS, &mut centres, &params);
    assert_eq!(centres[1], (10.0, 10.0));
}
