use super::*;
use konnect_sexp::{parse_sexp, SexpNode};

// KiCad10.0.5's own resave; its provenance README is beside the file.
const TWO_R: &str = include_str!("../../../tests/fixtures/specctra_two_resistors.kicad_pcb");

fn tree() -> SexpNode {
    parse_sexp(TWO_R).unwrap()
}
fn index(tree: &SexpNode) -> PcbConnectivityIndex {
    PcbConnectivityIndex::build(tree)
}
fn diagonal() -> f64 {
    50.0_f64.hypot(40.0)
}
fn objective(tree: &SexpNode) -> RoutingObjective {
    assess(&index(tree), 8, Some(diagonal()), &[])
}

fn footprint_mut<'a>(tree: &'a mut SexpNode, reference: &str) -> &'a mut Vec<SexpNode> {
    let SexpNode::List(nodes) = tree else {
        panic!("root")
    };
    let fp = nodes
        .iter_mut()
        .find(|n| {
            n.head() == Some("footprint")
                && n.find_all("property").iter().any(|p| {
                    p.get(1).and_then(SexpNode::as_str) == Some("Reference")
                        && p.get(2).and_then(SexpNode::as_str) == Some(reference)
                })
        })
        .unwrap();
    let SexpNode::List(items) = fp else {
        panic!("footprint")
    };
    items
}
fn move_to(tree: &mut SexpNode, reference: &str, x: f64, y: f64) {
    let nodes = footprint_mut(tree, reference);
    let at = nodes.iter_mut().find(|n| n.head() == Some("at")).unwrap();
    *at = SexpNode::List(vec![
        SexpNode::Atom("at".into()),
        SexpNode::Atom(x.to_string()),
        SexpNode::Atom(y.to_string()),
    ]);
}
fn pad_mut<'a>(tree: &'a mut SexpNode, reference: &str, number: &str) -> &'a mut Vec<SexpNode> {
    let pad = footprint_mut(tree, reference)
        .iter_mut()
        .find(|n| n.head() == Some("pad") && n.get(1).and_then(SexpNode::as_str) == Some(number))
        .unwrap();
    let SexpNode::List(nodes) = pad else {
        panic!("pad")
    };
    nodes
}
fn replace(nodes: &mut [SexpNode], head: &str, node: SexpNode) {
    *nodes.iter_mut().find(|n| n.head() == Some(head)).unwrap() = node;
}
fn unavailable(result: &RoutingObjective) {
    assert!(!result.complete, "{}", result.to_json());
    assert!(result.wirelength_mm.is_none() && result.crossings.is_none());
    assert!(result.wire_deduction.is_none() && result.crossing_deduction.is_none());
}

#[test]
fn native_two_resistors_have_twenty_mm_and_no_proper_crossing() {
    let result = objective(&tree());
    // VCC: (99.5,50)..(110.5,50)=11; GND:(100.5,50)..(109.5,50)=9.
    assert!(result.complete);
    assert_eq!(result.wirelength_mm, Some(20.0));
    assert_eq!(result.crossings, Some(0));
    assert_eq!(result.edge_count, 2);
    assert_eq!(result.eligible_nets, 2);
    assert_eq!(result.wire_deduction, Some(4));
    assert_eq!(result.crossing_deduction, Some(0));
    assert_eq!(result.to_json()["existing_copper_considered"], false);
}

#[test]
fn moving_the_native_second_resistor_has_hand_computed_lengths_and_one_crossing() {
    let mut data = tree();
    move_to(&mut data, "R2", 110.0, 60.0);
    let result = objective(&data);
    // Both lines intersect strictly at (105,55). Their lengths are sqrt221
    // and sqrt181. This expected geometry is independent of the MST code.
    let expected = 221.0_f64.sqrt() + 181.0_f64.sqrt();
    assert!((result.wirelength_mm.unwrap() - expected).abs() < 1e-10);
    assert_eq!(result.crossings, Some(1));
    assert_eq!(result.wire_deduction, Some(5));
    assert_eq!(result.crossing_deduction, Some(2));
    assert!(result.wire_deduction > objective(&tree()).wire_deduction);
}

#[test]
fn distinct_reference_fanout_limit_is_inclusive_and_filtering_is_explicit() {
    let mut data = tree();
    let mut third = footprint_mut(&mut data, "R1").clone();
    let property = third
        .iter_mut()
        .find(|n| {
            n.head() == Some("property") && n.get(1).and_then(SexpNode::as_str) == Some("Reference")
        })
        .unwrap();
    let SexpNode::List(values) = property else {
        panic!("property")
    };
    values[2] = SexpNode::Str("R3".into());
    let SexpNode::List(root) = &mut data else {
        panic!("root")
    };
    root.push(SexpNode::List(third));
    move_to(&mut data, "R3", 120.0, 50.0);
    let ix = index(&data);
    let filtered = assess(&ix, 2, Some(diagonal()), &[]);
    assert!(filtered.complete);
    assert_eq!(filtered.filtered_fanout, 2);
    assert_eq!(filtered.eligible_nets, 0);
    assert_eq!(filtered.wirelength_mm, Some(0.0));
    let included = assess(&ix, 3, Some(diagonal()), &[]);
    assert_eq!(included.eligible_nets, 2);
    assert_eq!(included.wirelength_mm, Some(40.0));
}

#[test]
fn single_site_and_single_reference_nets_do_not_become_external_airwires() {
    let mut data = tree();
    replace(
        pad_mut(&mut data, "R2", "1"),
        "net",
        SexpNode::List(vec![
            SexpNode::Atom("net".into()),
            SexpNode::Str("SINGLE".into()),
        ]),
    );
    let result = objective(&data);
    assert_eq!(result.filtered_fewer_sites, 2);
    assert_eq!(result.eligible_nets, 1);
    assert_eq!(result.wirelength_mm, Some(11.0));
    // Put every named site on R1's own net. Moving no other footprint can
    // improve this internal connection, so it is not a placement objective.
    let mut data = tree();
    let r2 = footprint_mut(&mut data, "R2");
    r2.retain(|n| n.head() != Some("pad"));
    replace(
        pad_mut(&mut data, "R1", "2"),
        "net",
        SexpNode::List(vec![
            SexpNode::Atom("net".into()),
            SexpNode::Str("VCC".into()),
        ]),
    );
    let result = objective(&data);
    assert_eq!(result.filtered_internal, 1);
    assert_eq!(result.edge_count, 0);
}

#[test]
fn physically_separate_same_number_pads_are_retained() {
    let mut data = tree();
    let mut extra = SexpNode::List(pad_mut(&mut data, "R1", "1").clone());
    let SexpNode::List(nodes) = &mut extra else {
        panic!("pad")
    };
    replace(
        nodes,
        "at",
        SexpNode::List(vec![
            SexpNode::Atom("at".into()),
            SexpNode::Atom("0.5".into()),
            SexpNode::Atom("0".into()),
        ]),
    );
    footprint_mut(&mut data, "R1").push(extra);
    let result = objective(&data);
    assert_eq!(result.edge_count, 3);
    assert_eq!(result.pad_sites, 5);
    assert_eq!(result.wirelength_mm, Some(20.0));
}

#[test]
fn reordered_native_forms_and_net_code_renumbering_keep_the_same_result() {
    let data = tree();
    let expected = objective(&data).to_json();
    let mut reordered = data.clone();
    let SexpNode::List(nodes) = &mut reordered else {
        panic!("root")
    };
    nodes[1..].reverse();
    for node in nodes {
        if node.head() == Some("footprint") {
            let SexpNode::List(children) = node else {
                unreachable!()
            };
            children[1..].reverse();
        }
    }
    assert_eq!(objective(&reordered).to_json(), expected);
    let mut coded = data;
    for (reference, pad, code) in [
        ("R1", "1", 41),
        ("R1", "2", 97),
        ("R2", "1", 97),
        ("R2", "2", 41),
    ] {
        replace(
            pad_mut(&mut coded, reference, pad),
            "net",
            SexpNode::List(vec![
                SexpNode::Atom("net".into()),
                SexpNode::Atom(code.to_string()),
            ]),
        );
    }
    let SexpNode::List(root) = &mut coded else {
        panic!("root")
    };
    for (code, name) in [(41, "VCC"), (97, "GND")] {
        root.push(SexpNode::List(vec![
            SexpNode::Atom("net".into()),
            SexpNode::Atom(code.to_string()),
            SexpNode::Str(name.into()),
        ]));
    }
    assert_eq!(objective(&coded).to_json(), expected);
}

#[test]
fn equal_weight_mst_choices_are_deterministic_and_use_all_sites() {
    let mut sites = vec![
        Site {
            reference: "A",
            pad: "1",
            at: (0.0, 0.0),
        },
        Site {
            reference: "B",
            pad: "1",
            at: (0.0, 1.0),
        },
        Site {
            reference: "C",
            pad: "1",
            at: (1.0, 0.0),
        },
        Site {
            reference: "D",
            pad: "1",
            at: (1.0, 1.0),
        },
    ];
    let (length, a) = mst("N", &sites, &mut 100).unwrap();
    sites.reverse();
    let (again, b) = mst("N", &sites, &mut 100).unwrap();
    assert_eq!(length, 3.0);
    assert_eq!(again, length);
    assert_eq!(a.len(), 3);
    let endpoints = |edges: Vec<Edge<'_>>| {
        edges
            .into_iter()
            .map(|e| (e.a.at, e.b.at))
            .collect::<Vec<_>>()
    };
    assert_eq!(endpoints(a), endpoints(b));

    // D is equally far from B and C. C is reached first, but stable geometry
    // ordering prefers B when its equal candidate is considered later.
    let sites = [
        Site {
            reference: "A",
            pad: "1",
            at: (-2.0, -10.0),
        },
        Site {
            reference: "B",
            pad: "1",
            at: (-1.0, 0.0),
        },
        Site {
            reference: "C",
            pad: "1",
            at: (0.0, -1.0),
        },
        Site {
            reference: "D",
            pad: "1",
            at: (3.0, 3.0),
        },
    ];
    let (_, edges) = mst("N", &sites, &mut 100).unwrap();
    assert_eq!(
        edges
            .iter()
            .find(|e| e.b.reference == "D")
            .unwrap()
            .a
            .reference,
        "B"
    );
}

#[test]
fn only_proper_interior_crossings_count() {
    assert!(proper_crossing((0.0, 0.0), (4.0, 4.0), (0.0, 4.0), (4.0, 0.0)).unwrap());
    assert!(!proper_crossing((0.0, 0.0), (2.0, 0.0), (2.0, 0.0), (2.0, 2.0)).unwrap());
    assert!(!proper_crossing((0.0, 0.0), (4.0, 0.0), (2.0, 0.0), (2.0, 2.0)).unwrap());
    assert!(!proper_crossing((0.0, 0.0), (4.0, 0.0), (1.0, 0.0), (3.0, 0.0)).unwrap());
    assert!(!proper_crossing((0.0, 0.0), (0.0, 0.0), (-1.0, 0.0), (1.0, 0.0)).unwrap());
    assert!(!proper_crossing(
        (-f64::MAX, 0.0),
        (-f64::MAX, 1.0),
        (f64::MAX, 0.0),
        (f64::MAX, 1.0)
    )
    .unwrap());
}

#[test]
fn same_net_segments_are_not_foreign_crossings_and_points_are_capped() {
    fn site(reference: &'static str, at: (f64, f64)) -> Site<'static> {
        Site {
            reference,
            pad: "1",
            at,
        }
    }
    let a = Edge {
        net: "A",
        a: site("R1", (0.0, 0.0)),
        b: site("R2", (4.0, 4.0)),
    };
    let mut b = Edge {
        net: "A",
        a: site("R3", (0.0, 4.0)),
        b: site("R4", (4.0, 0.0)),
    };
    assert_eq!(
        count_crossings(
            &[a.clone(), b.clone()],
            &mut 10,
            &mut Samples::new(Limits::default())
        )
        .unwrap(),
        0
    );
    b.net = "B";
    assert_eq!(
        count_crossings(&[a, b], &mut 10, &mut Samples::new(Limits::default())).unwrap(),
        1
    );
    assert_eq!(crossing_points(0), 0);
    assert_eq!(crossing_points(1), 2);
    assert_eq!(crossing_points(10), 20);
    assert_eq!(crossing_points(100), 20);
    assert_eq!(crossing_points(usize::MAX), 20);
}

#[test]
fn incomplete_index_nonfinite_sites_and_ambiguous_references_never_report_zero() {
    let mut missing = tree();
    pad_mut(&mut missing, "R1", "1").retain(|n| n.head() != Some("at"));
    let result = objective(&missing);
    unavailable(&result);
    assert_eq!(result.skipped_pads, 1);
    for (x, y) in [(f64::MAX, 0.0), (0.0, f64::MAX)] {
        let mut nonfinite = tree();
        // Inputs are finite-checked, but finite roots plus local offsets can
        // still overflow either transformed world coordinate.
        move_to(&mut nonfinite, "R1", x, y);
        replace(
            pad_mut(&mut nonfinite, "R1", "1"),
            "at",
            SexpNode::List(vec![
                SexpNode::Atom("at".into()),
                SexpNode::Atom(x.to_string()),
                SexpNode::Atom(y.to_string()),
            ]),
        );
        let result = objective(&nonfinite);
        unavailable(&result);
        assert!(result.reasons.iter().any(|s| s.contains("pad sites")));
    }
    let result = assess(&index(&tree()), 8, Some(diagonal()), &["R1".into()]);
    unavailable(&result);
    let mut unnamed = tree();
    let property = footprint_mut(&mut unnamed, "R1")
        .iter_mut()
        .find(|n| {
            n.head() == Some("property") && n.get(1).and_then(SexpNode::as_str) == Some("Reference")
        })
        .unwrap();
    let SexpNode::List(values) = property else {
        panic!("property")
    };
    values[2] = SexpNode::Str(String::new());
    unavailable(&objective(&unnamed));
}

#[test]
fn missing_diagonal_only_blocks_normalized_wire_points() {
    let mut data = tree();
    move_to(&mut data, "R2", 110.0, 60.0);
    let ix = index(&data);
    for diagonal in [
        None,
        Some(0.0),
        Some(-1.0),
        Some(f64::NAN),
        Some(f64::INFINITY),
    ] {
        let result = assess(&ix, 8, diagonal, &[]);
        assert!(result.complete);
        assert!(result.wirelength_mm.is_some());
        assert_eq!(result.crossings, Some(1));
        assert_eq!(result.wire_deduction, None);
        assert_eq!(result.crossing_deduction, Some(2));
    }
    let empty = parse_sexp("(kicad_pcb)").unwrap();
    let result = assess(&index(&empty), 8, None, &[]);
    assert!(result.complete);
    assert_eq!(result.wire_deduction, Some(0));
}

#[test]
fn work_limits_return_unavailable_totals_rather_than_a_prefix() {
    let ix = index(&tree());
    for limits in [
        Limits {
            sites: 3,
            ..Default::default()
        },
        Limits {
            distances: 0,
            ..Default::default()
        },
        Limits {
            pairs: 0,
            ..Default::default()
        },
    ] {
        let result = assess_with_limits(&ix, 8, Some(diagonal()), &[], limits);
        unavailable(&result);
        assert!(result.reasons.iter().any(|s| s.contains("limit exceeded")));
    }
}

#[test]
fn bounded_samples_do_not_turn_exact_totals_into_partial_metrics() {
    let mut data = tree();
    move_to(&mut data, "R2", 110.0, 60.0);
    let ix = index(&data);
    for limits in [
        Limits {
            samples: 0,
            ..Default::default()
        },
        Limits {
            sample_bytes: 1,
            ..Default::default()
        },
    ] {
        let result = assess_with_limits(&ix, 8, Some(diagonal()), &[], limits);
        assert!(result.complete && result.details_truncated);
        assert_eq!(result.crossings, Some(1));
        assert!(result.net_samples.is_empty() && result.crossing_samples.is_empty());
    }
}

#[test]
fn very_long_net_names_cannot_expand_new_details_beyond_their_budget() {
    let mut data = tree();
    let name = "V".repeat(50_000);
    for reference in ["R1", "R2"] {
        let number = if reference == "R1" { "1" } else { "2" };
        replace(
            pad_mut(&mut data, reference, number),
            "net",
            SexpNode::List(vec![
                SexpNode::Atom("net".into()),
                SexpNode::Str(name.clone()),
            ]),
        );
    }
    let result = objective(&data);
    assert!(result.complete && result.details_truncated);
    assert_eq!(result.wirelength_mm, Some(20.0));
    assert!(serde_json::to_vec(&result.to_json()).unwrap().len() < SAMPLE_BYTES + 4_000);
}

#[test]
fn nonfinite_computed_geometry_is_unavailable() {
    let sites = [
        Site {
            reference: "A",
            pad: "1",
            at: (-f64::MAX, 0.0),
        },
        Site {
            reference: "B",
            pad: "1",
            at: (f64::MAX, 0.0),
        },
    ];
    assert!(mst("N", &sites, &mut 10).unwrap_err().contains("nonfinite"));
    assert!(
        proper_crossing((0.0, 0.0), (1e200, 1e200), (0.0, 1e200), (1e200, 0.0))
            .unwrap_err()
            .contains("nonfinite")
    );
    let sites = [
        Site {
            reference: "A",
            pad: "1",
            at: (0.0, 0.0),
        },
        Site {
            reference: "B",
            pad: "1",
            at: (1e308, 0.0),
        },
        Site {
            reference: "C",
            pad: "1",
            at: (0.0, 1e308),
        },
        Site {
            reference: "D",
            pad: "1",
            at: (1e308, 1e308),
        },
    ];
    assert!(mst("N", &sites, &mut 100)
        .unwrap_err()
        .contains("overflowed"));
}

#[test]
fn overflow_across_individually_finite_nets_is_not_a_zero_objective() {
    let mut data = tree();
    move_to(&mut data, "R1", 0.0, 0.0);
    move_to(&mut data, "R2", 1e308, 0.0);
    for reference in ["R1", "R2"] {
        for number in ["1", "2"] {
            replace(
                pad_mut(&mut data, reference, number),
                "at",
                SexpNode::List(vec![
                    SexpNode::Atom("at".into()),
                    SexpNode::Atom("0".into()),
                    SexpNode::Atom("0".into()),
                ]),
            );
        }
    }
    let result = objective(&data);
    unavailable(&result);
    assert!(result
        .reasons
        .iter()
        .any(|s| s.contains("accumulation overflowed")));
}

#[test]
fn wire_points_are_bounded_and_normalized_by_edges_and_diagonal() {
    let ix = index(&tree());
    assert_eq!(assess(&ix, 8, Some(1.0), &[]).wire_deduction, Some(20));
    assert_eq!(assess(&ix, 8, Some(100.0), &[]).wire_deduction, Some(2));
    assert_eq!(
        assess(&ix, 8, Some(f64::MIN_POSITIVE), &[]).wire_deduction,
        Some(20)
    );
}
