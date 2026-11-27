//! Tests of the best-of-N search: effort levels, the DSN variants, the lanes, the
//! launch rule and the scheduler itself, which runs here on tokio's paused clock with
//! a fake [`Runner`].
//!
//! Every guard has a test that it holds and a counterpart that shows the honest case
//! passes; the counterpart is the guard's negative control.
//!
//! The scheduler tests end by calling `route_judge::rank` (the winner), so they need
//! its body. Which of them depend on its *order* is said on each test.

use super::*;
use crate::fastroute::Held;
use crate::route_judge::{Gates, Measures};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Fixtures: a small DSN in the shape Konnect exports it.
// ---------------------------------------------------------------------------

const GND_PLANE: &str = "    (plane GND\n      (polygon B.Cu 0 1 1 9 1 9 9 1 9)\n    )";
const VCC_PLANE: &str = "    (plane VCC\n      (polygon F.Cu 0 1 1 9 1 9 9 1 9)\n    )";
const STUB: &str = "(clearance 0 (type smd_to_turn_gap))";

const TOP: &str = "(pcb board.kicad_pcb\n  (parser\n    (string_quote \")\n    \
(space_in_quoted_tokens on)\n    (host_cad Konnect)\n  )\n  (resolution um 10)\n  (unit um)\n";
const STRUCTURE_HEAD: &str = "  (structure\n    (layer F.Cu\n      (type signal)\n    )\n    \
(boundary\n      (path pcb 0 0 0 100 0 100 100 0 100)\n    )\n";
const STRUCTURE_TAIL: &str = "    (via konnect_via_0001)\n    (rule\n      (width 200)\n      \
(clearance 200)\n    )\n  )\n";
const REST: &str =
    "  (library\n    (image img_C1\n      (pin pad_1 1 0 0)\n    )\n  )\n  (network\n    \
(net GND\n      (pins C1-1 C2-1)\n    )\n    (class k1 GND \"Net-(P1-PM)\"\n      (rule\n        \
(width 250)\n        (clearance 200)\n      )\n    )\n  )\n  (wiring\n  )\n)\n";

/// The structure rule's last lines, which several tests edit.
const RULE_END: &str = "      (clearance 200)\n    )\n  )\n  (placement";
/// The class rule's last lines.
const CLASS_RULE_END: &str = "        (clearance 200)\n      )\n    )\n  )\n  (wiring";

fn placement(count: usize) -> String {
    let mut text = String::from("  (placement\n");
    for n in 1..=count {
        text.push_str(&format!(
            "    (component img_C{n}\n      (place C{n} {} -1000 front 90)\n    )\n",
            n * 1000
        ));
    }
    text.push_str("  )\n");
    text
}

/// A DSN with the given planes (each already indented, no newline) and `count`
/// components. An empty plane text leaves just its newline, which is exactly what
/// removing a plane leaves behind.
fn dsn(planes: &[&str], count: usize) -> String {
    let mut text = String::from(TOP);
    text.push_str(STRUCTURE_HEAD);
    for plane in planes {
        text.push_str(plane);
        text.push('\n');
    }
    text.push_str(STRUCTURE_TAIL);
    text.push_str(&placement(count));
    text.push_str(REST);
    text
}

/// The text before the first component, the component blocks, the gaps between
/// them, and the text after the last one.
fn placement_parts(text: &str) -> (String, Vec<String>, Vec<String>, String) {
    let root = root_block(text).unwrap();
    let placement = child_blocks(text, root)
        .unwrap()
        .into_iter()
        .find(|c| has_head(text, *c, "placement"))
        .unwrap();
    let blocks = child_blocks(text, placement).unwrap();
    let before = text[..blocks[0].start].to_string();
    let after = text[blocks.last().unwrap().end..].to_string();
    let gaps = blocks
        .windows(2)
        .map(|w| text[w[0].end..w[1].start].to_string())
        .collect();
    let texts = blocks
        .iter()
        .map(|b| text[b.start..b.end].to_string())
        .collect();
    (before, texts, gaps, after)
}

fn refdes(block: &str) -> String {
    block
        .split("(place ")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .to_string()
}

fn order_of(text: &str) -> Vec<String> {
    placement_parts(text).1.iter().map(|b| refdes(b)).collect()
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// Effort
// ---------------------------------------------------------------------------

#[test]
fn effort_names_round_trip() {
    for (effort, name) in [
        (Effort::Fast, "fast"),
        (Effort::M, "m"),
        (Effort::H, "h"),
        (Effort::Xh, "xh"),
    ] {
        assert_eq!(effort.name(), name);
        assert_eq!(Effort::parse(name), Some(effort));
    }
    // Exactly the lower-case names, nothing near them.
    for bad in ["", "M", "Fast", "XH", "medium", " m", "m ", "xh2", "slow"] {
        assert_eq!(Effort::parse(bad), None, "{bad:?} must not parse");
    }
}

#[test]
fn effort_limits_are_the_owners_budgets() {
    assert_eq!(
        Effort::Fast.limits(),
        Limits {
            budget: None,
            runs: Some(2)
        }
    );
    for (effort, seconds) in [(Effort::M, 240), (Effort::H, 720), (Effort::Xh, 1320)] {
        assert_eq!(
            effort.limits(),
            Limits {
                budget: Some(Duration::from_secs(seconds)),
                runs: None
            },
            "{effort:?}"
        );
    }
}

#[test]
fn resolve_effort_takes_the_argument_then_the_config_then_m() {
    let arg = json!("h");
    let cfg = json!("xh");
    assert_eq!(
        resolve_effort(Some(&arg), Some(&cfg)),
        Ok((Effort::H, EffortSource::Argument))
    );
    assert_eq!(
        resolve_effort(None, Some(&cfg)),
        Ok((Effort::Xh, EffortSource::UserConfig))
    );
    assert_eq!(
        resolve_effort(None, None),
        Ok((Effort::M, EffortSource::Default))
    );
    // A JSON null is "not given".
    let null = Value::Null;
    assert_eq!(
        resolve_effort(Some(&null), Some(&cfg)),
        Ok((Effort::Xh, EffortSource::UserConfig))
    );
    assert_eq!(
        resolve_effort(Some(&null), Some(&null)),
        Ok((Effort::M, EffortSource::Default))
    );
}

#[test]
fn resolve_effort_refuses_a_bad_value_and_names_its_source() {
    let bad = json!("turbo");
    let good = json!("fast");
    // A bad argument is refused even when the config is fine.
    let err = resolve_effort(Some(&bad), Some(&good)).unwrap_err();
    assert!(err.contains("`effort` argument"), "{err}");
    assert!(err.contains("turbo"), "{err}");
    for name in ["fast", "m", "h", "xh"] {
        assert!(err.contains(name), "{err} must name {name}");
    }
    // A bad config value is refused, and says it came from the config.
    let err = resolve_effort(None, Some(&bad)).unwrap_err();
    assert!(err.contains("routing.effort"), "{err}");
    assert!(!err.contains("`effort` argument"), "{err}");
    // A bad config value does not matter when the argument is given.
    assert_eq!(
        resolve_effort(Some(&good), Some(&bad)),
        Ok((Effort::Fast, EffortSource::Argument))
    );
}

#[test]
fn resolve_effort_refuses_a_value_that_is_not_a_string() {
    for value in [json!(4), json!(true), json!(["m"]), json!({"effort": "m"})] {
        let err = resolve_effort(Some(&value), None).unwrap_err();
        assert!(err.contains("`effort` argument"), "{err}");
        let err = resolve_effort(None, Some(&value)).unwrap_err();
        assert!(err.contains("routing.effort"), "{err}");
    }
}

// ---------------------------------------------------------------------------
// Shapes and the DSN transforms
// ---------------------------------------------------------------------------

#[test]
fn shapes_offers_omitted_only_when_a_plane_exists() {
    assert_eq!(
        shapes(&dsn(&[GND_PLANE], 2)).unwrap(),
        vec![Shape::AsExported, Shape::Omitted]
    );
    assert_eq!(
        shapes(&dsn(&[GND_PLANE, VCC_PLANE], 2)).unwrap(),
        vec![Shape::AsExported, Shape::Omitted]
    );
    assert_eq!(shapes(&dsn(&[], 2)).unwrap(), vec![Shape::AsExported]);
    // A plane that is not a direct child of the structure does not count.
    let elsewhere = "(pcb t (structure (layer F.Cu (plane N)) (rule (width 1))) \
                     (library (plane A)) (network (plane B)))";
    assert_eq!(shapes(elsewhere).unwrap(), vec![Shape::AsExported]);
    // No structure at all is refused.
    assert!(shapes("(pcb t (placement))").is_err());
}

#[test]
fn strip_planes_removes_both_planes_and_nothing_else() {
    let (out, removed) = strip_planes(&dsn(&[GND_PLANE, VCC_PLANE], 6)).unwrap();
    assert_eq!(removed, 2);
    // Each plane and the indentation before it are gone; their newlines stay.
    assert_eq!(out, dsn(&["", ""], 6));
    assert!(!out.contains("(plane"));
    // The honest counterpart: a DSN with no plane comes back byte for byte.
    let plain = dsn(&[], 6);
    assert_eq!(strip_planes(&plain).unwrap(), (plain.clone(), 0));
}

#[test]
fn strip_planes_keeps_a_plane_that_is_inside_a_quoted_string() {
    // A direct child of the structure that is a string holding "(plane ...)" text,
    // then a real plane.
    let text = [
        "(pcb t\n  (parser\n    (string_quote \")\n  )\n  (structure\n",
        "    \"(plane decoy)\"\n",
        "    (layer F.Cu\n      (property (note \"(plane fake\"))\n    )\n",
        GND_PLANE,
        "\n    (rule (width 200))\n  )\n)\n",
    ]
    .concat();
    let text = text.as_str();
    let (out, removed) = strip_planes(text).unwrap();
    assert_eq!(removed, 1);
    assert_eq!(out, text.replace(GND_PLANE, ""));
    assert!(out.contains("\"(plane decoy)\""));
    assert!(out.contains("\"(plane fake\""));
    assert!(!out.contains("(plane GND"));
}

#[test]
fn strip_planes_leaves_a_plane_like_block_outside_the_structure() {
    let text = "(pcb t\n  (structure\n    (layer F.Cu (plane N))\n    (rule (width 1))\n  )\n  \
(library (image i (plane A (polygon B.Cu 0 0 0 1 1 1))))\n  (network (plane B))\n)\n";
    assert_eq!(strip_planes(text).unwrap(), (text.to_string(), 0));
    // The honest case: the same text with a real plane in the structure loses it.
    let with_plane = text.replace(
        "    (rule (width 1))\n",
        "    (plane GND (polygon B.Cu 0 0 0 1 1 1))\n    (rule (width 1))\n",
    );
    let (out, removed) = strip_planes(&with_plane).unwrap();
    assert_eq!(removed, 1);
    assert_eq!(
        out,
        text.replace("    (rule (width 1))\n", "\n    (rule (width 1))\n")
    );
}

#[test]
fn strip_planes_refuses_a_dsn_with_no_structure() {
    assert!(strip_planes("(pcb t (placement) (library))").is_err());
    assert!(strip_planes("not a dsn").is_err());
    // An unclosed quote is an error, not a silent pass.
    assert!(strip_planes("(pcb t (structure (layer \"F.Cu)))").is_err());
}

#[test]
fn reorder_placement_with_seed_zero_is_the_identity() {
    let text = dsn(&[GND_PLANE], 6);
    assert_eq!(reorder_placement(&text, 0).unwrap(), text);
}

#[test]
fn reorder_placement_moves_only_the_blocks() {
    let text = dsn(&[GND_PLANE, VCC_PLANE], 6);
    let (before, blocks, gaps, after) = placement_parts(&text);
    for seed in [1u64, 2, 3] {
        let out = reorder_placement(&text, seed).unwrap();
        let (b2, blocks2, gaps2, a2) = placement_parts(&out);
        // The rest of the file and the gaps are byte-identical.
        assert_eq!(b2, before, "seed {seed}");
        assert_eq!(a2, after, "seed {seed}");
        assert_eq!(gaps2, gaps, "seed {seed}");
        assert_eq!(out.len(), text.len());
        // The same blocks, each byte for byte, in another order.
        let mut sorted_in = blocks.clone();
        let mut sorted_out = blocks2.clone();
        sorted_in.sort();
        sorted_out.sort();
        assert_eq!(sorted_out, sorted_in, "seed {seed}");
        assert_ne!(blocks2, blocks, "seed {seed} must not be the identity here");
    }
}

#[test]
fn reorder_placement_keeps_uneven_gaps_in_place() {
    let text =
        "(pcb t (structure (rule (width 1))) (placement (component a (place C1 1 1 front 0))\n\n  \
(component b (place C2 2 2 front 0)) (component c (place C3 3 3 front 0))\t\
(component d (place C4 4 4 front 0))) )";
    let (_, _, gaps, _) = placement_parts(text);
    assert_eq!(gaps, vec!["\n\n  ", " ", "\t"]);
    let out = reorder_placement(text, 3).unwrap();
    let (_, _, gaps_out, _) = placement_parts(&out);
    assert_eq!(gaps_out, gaps);
    // SplitMix64 seed 3 on four blocks (mirrored in Python): positions take 3, 1, 2, 0.
    assert_eq!(order_of(&out), names(&["C4", "C2", "C3", "C1"]));
}

#[test]
fn reorder_placement_is_the_same_for_the_same_seed_and_pinned_to_splitmix() {
    let text = dsn(&[], 6);
    let first = reorder_placement(&text, 7).unwrap();
    assert_eq!(reorder_placement(&text, 7).unwrap(), first);
    // Pinned orders for seeds 1, 2 and 3 (an independent Python SplitMix64 and
    // Fisher-Yates from the last position down), so a seed names the same order on
    // every machine and in every build.
    assert_eq!(
        order_of(&reorder_placement(&text, 1).unwrap()),
        names(&["C3", "C1", "C2", "C5", "C6", "C4"])
    );
    assert_eq!(
        order_of(&reorder_placement(&text, 2).unwrap()),
        names(&["C2", "C1", "C5", "C3", "C6", "C4"])
    );
    assert_eq!(
        order_of(&reorder_placement(&text, 3).unwrap()),
        names(&["C2", "C5", "C6", "C3", "C4", "C1"])
    );
}

#[test]
fn reorder_placement_gives_different_orders_for_different_seeds() {
    let text = dsn(&[], 6);
    let orders: Vec<Vec<String>> = (1..=3)
        .map(|seed| order_of(&reorder_placement(&text, seed).unwrap()))
        .collect();
    assert_ne!(orders[0], orders[1]);
    assert_ne!(orders[0], orders[2]);
    assert_ne!(orders[1], orders[2]);
    let natural = order_of(&text);
    for order in &orders {
        assert_ne!(order, &natural);
    }
}

#[test]
fn reorder_placement_refuses_a_non_component_child_and_a_missing_placement() {
    let ok = dsn(&[], 3);
    assert!(reorder_placement(&ok, 1).is_ok());
    let foreign = ok.replace(
        "  (placement\n",
        "  (placement\n    (keepout hole (circle F.Cu 100))\n",
    );
    let err = reorder_placement(&foreign, 1).unwrap_err().to_string();
    assert!(err.contains("not a component"), "{err}");
    assert!(err.contains("keepout"), "{err}");
    assert!(reorder_placement("(pcb t (structure (rule (width 1))))", 1).is_err());
}

#[test]
fn stub_rule_off_adds_the_clearance_as_the_rules_last_child() {
    let text = dsn(&[GND_PLANE], 2);
    let out = stub_rule_off(&text).unwrap();
    let inserted = format!("      (clearance 200)\n      {STUB}\n    )\n  )\n  (placement");
    assert_eq!(out, text.replace(RULE_END, &inserted));
    // The rule stays well formed: applying it again only rewrites the 0 it added.
    assert_eq!(stub_rule_off(&out).unwrap(), out);
}

#[test]
fn stub_rule_off_adds_to_a_one_line_rule_inline() {
    assert_eq!(
        stub_rule_off("(pcb t (structure (rule (width 200))))").unwrap(),
        format!("(pcb t (structure (rule (width 200) {STUB})))")
    );
}

#[test]
fn stub_rule_off_replaces_an_existing_value() {
    let with_stub = dsn(&[], 2).replace(
        RULE_END,
        "      (clearance 200)\n      (clearance 50 (type smd_to_turn_gap))\n      \
(clearance 100 (type wire_wire))\n    )\n  )\n  (placement",
    );
    let out = stub_rule_off(&with_stub).unwrap();
    assert_eq!(
        out,
        with_stub.replace("(clearance 50 (type smd_to_turn_gap))", STUB)
    );
    // The other typed clearance, and the plain one, are untouched; nothing was added.
    assert!(out.contains("(clearance 100 (type wire_wire))"));
    assert_eq!(out.matches("smd_to_turn_gap").count(), 1);
    assert_eq!(stub_rule_off(&out).unwrap(), out);
}

#[test]
fn stub_rule_off_does_not_touch_a_class_rule() {
    // Only the class rule carries the stub clearance.
    let classy = dsn(&[], 2).replace(
        CLASS_RULE_END,
        "        (clearance 200)\n        (clearance 50 (type smd_to_turn_gap))\n      )\n    )\n  )\n  (wiring",
    );
    let out = stub_rule_off(&classy).unwrap();
    // The class's 50 is still there; the structure's own rule got the new one.
    assert!(out.contains("        (clearance 50 (type smd_to_turn_gap))"));
    let inserted = format!("      (clearance 200)\n      {STUB}\n    )\n  )\n  (placement");
    assert_eq!(out, classy.replace(RULE_END, &inserted));
}

#[test]
fn stub_rule_off_refuses_a_structure_with_no_rule() {
    // Only a class has a rule: it must not be used in the structure's place.
    let text = "(pcb t (structure (layer F.Cu)) (network (class k (rule (width 1)))))";
    let err = stub_rule_off(text).unwrap_err().to_string();
    assert!(err.contains("no (rule"), "{err}");
    assert!(stub_rule_off("(pcb t (placement))").is_err());
}

#[test]
fn variant_text_omitted_with_a_seed_applies_all_three_edits() {
    let base = dsn(&[GND_PLANE, VCC_PLANE], 6);
    let out = variant_text(&base, Shape::Omitted, 3).unwrap();
    assert!(!out.contains("(plane"));
    assert_eq!(out.matches(STUB).count(), 1);
    assert_eq!(order_of(&out), names(&["C2", "C5", "C6", "C3", "C4", "C1"]));
    // Everything the three edits do not touch is as it was.
    assert!(out.ends_with(REST));
    assert!(out.starts_with(TOP));
}

#[test]
fn variant_text_as_exported_with_seed_zero_applies_only_the_stub_rule() {
    let base = dsn(&[GND_PLANE, VCC_PLANE], 6);
    let out = variant_text(&base, Shape::AsExported, 0).unwrap();
    let inserted = format!("      (clearance 200)\n      {STUB}\n    )\n  )\n  (placement");
    assert_eq!(out, base.replace(RULE_END, &inserted));
    assert!(out.contains("(plane GND") && out.contains("(plane VCC"));
    assert_eq!(order_of(&out), order_of(&base));
}

// ---------------------------------------------------------------------------
// SplitMix64, launch order, lanes, the launch rule, the median
// ---------------------------------------------------------------------------

#[test]
fn splitmix64_matches_the_reference_outputs() {
    let mut rng = SplitMix64(0);
    assert_eq!(rng.next_u64(), 0xE220_A839_7B1D_CDAF);
    assert_eq!(rng.next_u64(), 0x6E78_9E6A_A1B9_65F4);
    assert_eq!(rng.next_u64(), 0x06C4_5D18_8009_454F);
    let mut rng = SplitMix64(5);
    assert_eq!(rng.next_u64(), 7_134_611_160_154_358_618);
    assert_eq!(rng.next_u64(), 13_877_614_986_023_876_344);
}

#[test]
fn below_stays_in_range_and_reaches_every_value() {
    for bound in [1u64, 2, 7, 1 << 63] {
        let mut rng = SplitMix64(bound);
        for _ in 0..2000 {
            assert!(rng.below(bound) < bound, "bound {bound}");
        }
    }
    let mut rng = SplitMix64(11);
    let mut seen = [0usize; 7];
    for _ in 0..2000 {
        seen[rng.below(7) as usize] += 1;
    }
    assert!(seen.iter().all(|&n| n > 100), "{seen:?}");
    // Pinned values (Python mirror): bound 1 is always 0; bound 7 from seed 1.
    let mut rng = SplitMix64(9);
    assert_eq!((rng.below(1), rng.below(1), rng.below(1)), (0, 0, 0));
    let mut rng = SplitMix64(9);
    let _ = (rng.below(1), rng.below(1), rng.below(1));
    let twos: Vec<u64> = (0..6).map(|_| rng.below(2)).collect();
    assert_eq!(twos, vec![1, 0, 0, 1, 1, 0]);
    let mut rng = SplitMix64(1);
    let sevens: Vec<u64> = (0..5).map(|_| rng.below(7)).collect();
    assert_eq!(sevens, vec![3, 5, 6, 3, 3]);
}

#[test]
fn below_rejects_the_biased_draws() {
    // For this bound, seed 0's first draw has a low half under the threshold, so
    // Lemire's method must discard it. Without the rejection the answer would be
    // 8147104208329303768 (the Python mirror gives both).
    let bound = (1u64 << 63) + 1;
    let mut rng = SplitMix64(0);
    assert_eq!(rng.below(bound), 243_808_509_735_772_839);
    let mut rng = SplitMix64(5);
    let drawn: Vec<u64> = (0..6).map(|_| rng.below(bound)).collect();
    assert_eq!(
        drawn,
        vec![
            6_938_807_493_011_938_172,
            2_146_363_211_429_306_531,
            916_244_348_587_400_354,
            1_733_626_130_553_941_730,
            4_154_899_361_148_330_835,
            4_175_487_192_854_586_758
        ]
    );
}

/// The first `n` runs of an order.
fn first(order: &Order, n: usize) -> Vec<(Shape, u64)> {
    (0..n).map_while(|index| order.get(index)).collect()
}

#[test]
fn launch_order_interleaves_seeds_over_shapes() {
    let both = [Shape::AsExported, Shape::Omitted];
    let order = launch_order(&both, Effort::M.limits());
    assert_eq!(
        first(&order, 5),
        vec![
            (Shape::AsExported, 1),
            (Shape::Omitted, 1),
            (Shape::AsExported, 2),
            (Shape::Omitted, 2),
            (Shape::AsExported, 3),
        ]
    );
    // One shape: seeds in turn.
    let one = launch_order(&[Shape::AsExported], Effort::H.limits());
    assert_eq!(
        first(&one, 3),
        vec![
            (Shape::AsExported, 1),
            (Shape::AsExported, 2),
            (Shape::AsExported, 3)
        ]
    );
    // The shapes' own order is kept.
    let swapped = launch_order(&[Shape::Omitted, Shape::AsExported], Effort::M.limits());
    assert_eq!(swapped.get(0), Some((Shape::Omitted, 1)));
    assert_eq!(swapped.get(1), Some((Shape::AsExported, 1)));
    assert!(launch_order(&[], Effort::M.limits()).is_empty());
    assert!(launch_order(&[], Effort::Fast.limits()).is_empty());
}

#[test]
fn launch_order_gives_two_for_fast_and_has_no_end_under_a_budget() {
    let both = [Shape::AsExported, Shape::Omitted];
    assert_eq!(
        launch_order(&both, Effort::Fast.limits()),
        Order::Fixed(vec![(Shape::AsExported, 1), (Shape::Omitted, 1)])
    );
    assert_eq!(
        launch_order(&[Shape::AsExported], Effort::Fast.limits()),
        Order::Fixed(vec![(Shape::AsExported, 1), (Shape::AsExported, 2)])
    );
    assert_eq!(launch_order(&both, Effort::Fast.limits()).get(2), None);
    // Under a budget there is no run cap (owner, 2026-11-16): the order goes on.
    for effort in [Effort::M, Effort::H, Effort::Xh] {
        let order = launch_order(&both, effort.limits());
        assert_eq!(order, Order::Endless(both.to_vec()), "{effort:?}");
        assert_eq!(order.get(239), Some((Shape::Omitted, 120)));
        assert_eq!(
            order.get(240),
            Some((Shape::AsExported, 121)),
            "past the old cap"
        );
        assert_eq!(order.get(99_999), Some((Shape::Omitted, 50_000)));
        let single = launch_order(&[Shape::AsExported], effort.limits());
        assert_eq!(single.get(9_999), Some((Shape::AsExported, 10_000)));
    }
    // An odd count ends in the middle of a seed.
    let odd = launch_order(
        &both,
        Limits {
            budget: None,
            runs: Some(5),
        },
    );
    assert_eq!(odd.get(4), Some((Shape::AsExported, 3)));
    assert_eq!(odd.get(5), None);
}

#[test]
fn lanes_follow_the_core_count() {
    // cores: routers = clamp(cores / 2, 1, 7), judges = clamp(cores / 4, 1, 3).
    for (cores, routers, judges) in [
        (1, 1, 1),
        (2, 1, 1),
        (4, 2, 1),
        (8, 4, 2),
        (14, 7, 3),
        (32, 7, 3),
    ] {
        assert_eq!(
            lanes(cores, Effort::M),
            Lanes { routers, judges },
            "{cores} cores"
        );
        assert_eq!(lanes(cores, Effort::Xh), lanes(cores, Effort::M));
    }
}

#[test]
fn fast_lanes_give_routers_two_whatever_the_cores() {
    for cores in [1, 2, 4, 8, 14, 32] {
        assert_eq!(lanes(cores, Effort::Fast).routers, 2, "{cores} cores");
    }
    // Judges still follow the cores.
    assert_eq!(lanes(1, Effort::Fast).judges, 1);
    assert_eq!(lanes(14, Effort::Fast).judges, 3);
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[test]
fn may_launch_allows_a_finish_exactly_at_the_budget() {
    // now 10 + route 20 + 3 queued x judge 10 (one judge) + judge 10 = 70.
    let launch =
        |budget: Duration| may_launch(secs(10), Some(budget), Some(secs(20)), Some(secs(10)), 3, 1);
    assert!(launch(secs(70)), "equal to the budget is allowed");
    assert!(launch(secs(71)));
    assert!(
        !launch(Duration::from_millis(69_999)),
        "one millisecond over is not"
    );
    assert!(!launch(secs(40)));
}

#[test]
fn may_launch_is_false_without_medians_or_a_budget() {
    let budget = Some(secs(10_000));
    assert!(may_launch(
        secs(0),
        budget,
        Some(secs(1)),
        Some(secs(1)),
        0,
        1
    ));
    assert!(!may_launch(secs(0), budget, None, Some(secs(1)), 0, 1));
    assert!(!may_launch(secs(0), budget, Some(secs(1)), None, 0, 1));
    assert!(!may_launch(secs(0), budget, None, None, 0, 1));
    // No budget (Fast) never launches by the rule.
    assert!(!may_launch(
        secs(0),
        None,
        Some(secs(1)),
        Some(secs(1)),
        0,
        1
    ));
}

#[test]
fn may_launch_rounds_the_queue_up_by_the_judges() {
    // now 10 + route 20 + judge 10 = 40, plus ceil(queued / judges) waves of 10.
    let launch = |budget: u64, queued: usize, judges: usize| {
        may_launch(
            secs(10),
            Some(secs(budget)),
            Some(secs(20)),
            Some(secs(10)),
            queued,
            judges,
        )
    };
    // 3 queued over 2 judges is 2 waves (60), not 1 (50): rounding up.
    assert!(launch(60, 3, 2));
    assert!(!launch(55, 3, 2));
    // 2 queued over 2 judges is 1 wave (50).
    assert!(launch(50, 2, 2));
    assert!(!launch(49, 2, 2));
    // Nothing queued: just the run's own judge (40).
    assert!(launch(40, 0, 3));
    assert!(!launch(39, 0, 3));
    // Many judges still need one wave for a queue that fits them.
    assert!(launch(50, 3, 3));
    // A judge count of 0 is read as 1 (no division by zero).
    assert_eq!(launch(60, 2, 0), launch(60, 2, 1));
}

#[test]
fn median_takes_the_lower_middle() {
    assert_eq!(median(&[]), None);
    assert_eq!(median(&[secs(7)]), Some(secs(7)));
    assert_eq!(median(&[secs(9), secs(1), secs(5)]), Some(secs(5)));
    assert_eq!(
        median(&[secs(4), secs(1), secs(3), secs(2)]),
        Some(secs(2)),
        "the lower middle of an even count"
    );
    assert_eq!(median(&[secs(8), secs(8)]), Some(secs(8)));
}

// ---------------------------------------------------------------------------
// The scheduler, on a paused clock with a fake runner
// ---------------------------------------------------------------------------

fn close(actual: Option<f64>, expected: f64) {
    let actual = actual.expect("a time was recorded");
    assert!(
        (actual - expected).abs() < 2e-3,
        "{actual} is not {expected}"
    );
}

fn close_duration(actual: Duration, expected_s: u64) {
    close(Some(actual.as_secs_f64()), expected_s as f64);
}

fn baseline() -> Baseline {
    Baseline {
        violations: BTreeMap::new(),
        unconnected: 0,
        pour_fills: None,
        pour_fills_error: None,
    }
}

/// A judged run whose rank is obvious: it differs from another only in its acute
/// corners (fewer is better) or in a failed G1 (worse than any pass).
fn judged(acute: f64, g1_failed: bool) -> Judged {
    Judged {
        measures: Measures {
            unconnected: 0,
            violations: BTreeMap::new(),
            routed_share: Some(1.0),
            audit_complete: Some(true),
            acute: Some(acute),
            right_angles: Some(0.0),
            copper_quality: Some(90.0),
            nets_undersized: Some(0.0),
            narrow_paths: Some(0.0),
            signal_mm: 100.0,
            power_mm: 10.0,
            pour_track_mm: 0.0,
            vias_off_pour: 0,
            below_floor: 0,
            width_rows: Vec::new(),
            longest_below_mm: BTreeMap::new(),
            stitching_vias: 0,
            route_s: 1.0,
            lengths: None,
            pours: None,
            pours_error: None,
            reference_vias: 0,
            checklist: Default::default(),
        },
        gates: Gates {
            g1: if g1_failed {
                vec!["the board is not complete".to_string()]
            } else {
                Vec::new()
            },
            g2: Vec::new(),
            g3: Vec::new(),
            g4: Vec::new(),
        },
        board: PathBuf::from("/nonexistent/judged.kicad_pcb"),
        rounds: Vec::new(),
        tune: None,
        reference_stitch: None,
        reference_checked: None,
    }
}

fn routed(index: usize, shape: Shape, seed: u64) -> Routed {
    Routed {
        index,
        shape,
        seed,
        dsn: PathBuf::from("variant.dsn"),
        held: Held {
            ses: PathBuf::from("variant.ses"),
            unrouted: 0,
            kept: "full",
            passes: Vec::new(),
            seconds: 1.0,
            untuned: None,
            tune: Default::default(),
            group_search: None,
            planes: None,
        },
        planes: Vec::new(),
        folder: PathBuf::from("run"),
    }
}

/// A routed run with Row LT's Tune candidates: the untuned session `variant.ses` kept
/// in `held.ses`, a `Free` and a `PourSafe` candidate beside it.
fn tuned(routed: &mut Routed) {
    use crate::fastroute::{TuneCandidate, TuneVariant};
    routed.held.untuned = Some(routed.held.ses.clone());
    for (at, variant) in [TuneVariant::Free, TuneVariant::PourSafe]
        .into_iter()
        .enumerate()
    {
        routed.held.tune.candidates.push(TuneCandidate {
            variant,
            ses: PathBuf::from(format!("{}.ses", variant.folder_name(1))),
            unrouted: at,
            dropped: Vec::new(),
            passes: 1,
        });
        routed.held.tune.rows.push(json!({
            "name": "tune", "outcome": "candidate", "unrouted": at, "seconds": 1.0,
            "variant": variant, "round": 1
        }));
    }
    routed.held.passes.push(routed.held.tune.rows[0].clone());
}

/// How many jobs of one kind ran at once.
struct Level {
    now: AtomicUsize,
    max: AtomicUsize,
}

struct Hold(Arc<Level>);

impl Hold {
    fn take(level: &Arc<Level>) -> Hold {
        let now = level.now.fetch_add(1, Ordering::SeqCst) + 1;
        level.max.fetch_max(now, Ordering::SeqCst);
        Hold(Arc::clone(level))
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.0.now.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Seconds a route takes, and the message it fails with (if it does).
type RouteSpec = (u64, Option<&'static str>);
/// Seconds a judge takes, and what it returns.
type JudgeSpec = (u64, std::result::Result<Judged, &'static str>);

struct Shared {
    route: Box<dyn Fn(usize) -> RouteSpec + Send + Sync>,
    judge: Box<dyn Fn(usize) -> JudgeSpec + Send + Sync>,
    baseline: (u64, std::result::Result<Baseline, &'static str>),
    routes_at_once: Arc<Level>,
    judges_at_once: Arc<Level>,
    deadlines: Mutex<Vec<(usize, Instant)>>,
    judge_starts: Mutex<Vec<(usize, Instant)>>,
    released: Mutex<Vec<usize>>,
    /// Row LT: every run is routed with two Tune candidates.
    tuned: std::sync::atomic::AtomicBool,
    /// Row ZR, ZR5 part 2: the search ranks by the pour key (else today's).
    pours: std::sync::atomic::AtomicBool,
}

#[derive(Clone)]
struct Fake(Arc<Shared>);

fn fake_with_baseline(
    route: impl Fn(usize) -> RouteSpec + Send + Sync + 'static,
    judge: impl Fn(usize) -> JudgeSpec + Send + Sync + 'static,
    baseline: (u64, std::result::Result<Baseline, &'static str>),
) -> Fake {
    let level = || {
        Arc::new(Level {
            now: AtomicUsize::new(0),
            max: AtomicUsize::new(0),
        })
    };
    Fake(Arc::new(Shared {
        route: Box::new(route),
        judge: Box::new(judge),
        baseline,
        routes_at_once: level(),
        judges_at_once: level(),
        deadlines: Mutex::new(Vec::new()),
        judge_starts: Mutex::new(Vec::new()),
        released: Mutex::new(Vec::new()),
        tuned: std::sync::atomic::AtomicBool::new(false),
        pours: std::sync::atomic::AtomicBool::new(false),
    }))
}

fn fake(
    route: impl Fn(usize) -> RouteSpec + Send + Sync + 'static,
    judge: impl Fn(usize) -> JudgeSpec + Send + Sync + 'static,
) -> Fake {
    fake_with_baseline(route, judge, (0, Ok(baseline())))
}

/// A judge that takes `seconds` and judges the run as an ordinary good one.
fn good(seconds: u64) -> JudgeSpec {
    (seconds, Ok(judged(1.0, false)))
}

impl Fake {
    fn most_routes_at_once(&self) -> usize {
        self.0.routes_at_once.max.load(Ordering::SeqCst)
    }

    fn most_judges_at_once(&self) -> usize {
        self.0.judges_at_once.max.load(Ordering::SeqCst)
    }

    fn deadline_of(&self, index: usize) -> Instant {
        self.0
            .deadlines
            .lock()
            .unwrap()
            .iter()
            .find(|(i, _)| *i == index)
            .expect("the route was started")
            .1
    }

    fn judge_calls(&self) -> usize {
        self.0.judge_starts.lock().unwrap().len()
    }

    /// The runs released, sorted.
    fn released(&self) -> Vec<usize> {
        let mut released = self.0.released.lock().unwrap().clone();
        released.sort_unstable();
        released
    }
}

impl Runner for Fake {
    fn route(
        &self,
        index: usize,
        shape: Shape,
        seed: u64,
        deadline: Instant,
    ) -> BoxFuture<std::result::Result<Routed, RouteFailure>> {
        let shared = Arc::clone(&self.0);
        Box::pin(async move {
            let _hold = Hold::take(&shared.routes_at_once);
            shared.deadlines.lock().unwrap().push((index, deadline));
            let (seconds, failure) = (shared.route)(index);
            tokio::select! {
                _ = tokio::time::sleep(secs(seconds)) => match failure {
                    Some(message) => Err(RouteFailure::Failed(message.to_string())),
                    None => {
                        let mut routed = routed(index, shape, seed);
                        if shared.tuned.load(Ordering::SeqCst) {
                            tuned(&mut routed);
                        }
                        Ok(routed)
                    }
                },
                _ = tokio::time::sleep_until(deadline) => Err(RouteFailure::Killed),
            }
        })
    }

    fn judge(&self, routed: Routed, baseline: Arc<Baseline>) -> BoxFuture<Result<Judged>> {
        let shared = Arc::clone(&self.0);
        Box::pin(async move {
            let _hold = Hold::take(&shared.judges_at_once);
            let _ = baseline;
            shared
                .judge_starts
                .lock()
                .unwrap()
                .push((routed.index, Instant::now()));
            let (seconds, result) = (shared.judge)(routed.index);
            tokio::time::sleep(secs(seconds)).await;
            result.map_err(|message| anyhow!(message))
        })
    }

    fn release(&self, index: usize, _shape: Shape, _seed: u64) {
        let mut released = self.0.released.lock().unwrap();
        assert!(!released.contains(&index), "run {index} released twice");
        released.push(index);
    }

    fn rank(&self) -> crate::pour_rank::RankMode {
        if self.0.pours.load(Ordering::SeqCst) {
            crate::pour_rank::RankMode::Pours
        } else {
            crate::pour_rank::RankMode::Routing
        }
    }

    fn baseline(&self) -> BoxFuture<Result<Baseline>> {
        let shared = Arc::clone(&self.0);
        Box::pin(async move {
            tokio::time::sleep(secs(shared.baseline.0)).await;
            shared
                .baseline
                .1
                .clone()
                .map_err(|message| anyhow!(message))
        })
    }
}

fn plan(routers: usize, judges: usize, budget: Option<u64>, order_len: usize) -> SearchPlan {
    SearchPlan {
        limits: Limits {
            budget: budget.map(secs),
            runs: None,
        },
        lanes: Lanes { routers, judges },
        order: (0..order_len)
            .map(|i| (Shape::AsExported, i as u64 + 1))
            .collect(),
        hang_cap: HANG_CAP,
    }
}

async fn run(plan: &SearchPlan, fake: &Fake) -> SearchOutcome {
    search(plan, Arc::new(fake.clone())).await
}

fn outcomes(outcome: &SearchOutcome) -> Vec<&'static str> {
    outcome.runs.iter().map(|r| r.outcome).collect()
}

fn launches(outcome: &SearchOutcome) -> Vec<f64> {
    outcome.runs.iter().map(|r| r.launched_s).collect()
}

fn assert_launches(outcome: &SearchOutcome, expected: &[u64]) {
    let actual = launches(outcome);
    assert_eq!(actual.len(), expected.len(), "{actual:?}");
    for (a, e) in actual.iter().zip(expected) {
        close(Some(*a), *e as f64);
    }
}

// (a) the first wave fills exactly the router lanes (needs rank: a winner is chosen)
#[tokio::test(start_paused = true)]
async fn the_first_wave_fills_exactly_the_router_lanes() {
    let fake = fake(|_| (100, None), |_| good(10));
    let outcome = run(&plan(3, 2, Some(150), 10), &fake).await;
    // Three at once, nothing more: the rule has no judge median to go by until the
    // first judge ends, and by then the budget no longer fits another run.
    assert_launches(&outcome, &[0, 0, 0]);
    assert_eq!(fake.most_routes_at_once(), 3);
    assert_eq!(fake.most_judges_at_once(), 2);
    assert_eq!(outcomes(&outcome), vec!["judged"; 3]);
    close_duration(outcome.elapsed, 120);
    assert_eq!(
        outcome.stopped_launching.as_deref(),
        Some("the launch rule")
    );
    assert!(outcome.budget_overrun.is_none());
}

#[tokio::test(start_paused = true)]
async fn the_first_wave_is_the_order_when_the_order_is_shorter_than_the_lanes() {
    let fake = fake(|_| (10, None), |_| good(10));
    let outcome = run(&plan(5, 1, Some(500), 2), &fake).await;
    assert_launches(&outcome, &[0, 0]);
    assert_eq!(fake.most_routes_at_once(), 2);
    assert_eq!(
        outcome.stopped_launching.as_deref(),
        Some("order exhausted")
    );
}

// (b) no launch until a route and a judge have finished, then only while the rule holds
#[tokio::test(start_paused = true)]
async fn no_run_starts_before_the_first_judge_has_ended_then_the_rule_decides() {
    // Routes take 100 s and judges 50 s, one judge lane, two router lanes.
    let route = |_| (100, None);
    let judge = |_| good(50);

    // A budget of 1000: run 2 and 3 start at 150, when the first judge ends (and not
    // at 100, when the first route ended).
    let fake_a = fake(route, judge);
    let outcome = run(&plan(2, 1, Some(1000), 4), &fake_a).await;
    assert_launches(&outcome, &[0, 0, 150, 150]);
    assert_eq!(outcomes(&outcome), vec!["judged"; 4]);
    assert_eq!(
        outcome.stopped_launching.as_deref(),
        Some("order exhausted")
    );
    close_duration(outcome.elapsed, 350);
    // The run's own records.
    close(outcome.runs[0].routed_s, 100.0);
    close(outcome.runs[0].route_s, 100.0);
    close(outcome.runs[0].judge_s, 50.0);
    // Both routes end at 100 s; the one judge lane takes them in turn (which one
    // goes first is the order the two completions are seen in).
    let mut first_two: Vec<f64> = outcome.runs[..2]
        .iter()
        .map(|r| r.judged_s.unwrap())
        .collect();
    first_two.sort_by(f64::total_cmp);
    close(Some(first_two[0]), 150.0);
    close(Some(first_two[1]), 200.0);
    // Equal runs: the tie goes to the lower index.
    assert_eq!(outcome.winner, Some(0));
}

#[tokio::test(start_paused = true)]
async fn the_launch_rule_allows_a_finish_exactly_at_the_budget_and_refuses_one_second_over() {
    let route = |_| (100, None);
    let judge = |_| good(50);
    // At 150: run 2 needs 150 + 100 + 1 x 50 + 50 = 350; run 3 would need 400.
    let at_the_edge = fake(route, judge);
    let outcome = run(&plan(2, 1, Some(350), 10), &at_the_edge).await;
    assert_launches(&outcome, &[0, 0, 150]);
    assert_eq!(
        outcome.stopped_launching.as_deref(),
        Some("the launch rule")
    );
    assert_eq!(outcomes(&outcome), vec!["judged"; 3]);
    close_duration(outcome.elapsed, 300);

    // One second less and run 2 does not fit either: only the first wave runs.
    let just_short = fake(route, judge);
    let outcome = run(&plan(2, 1, Some(349), 10), &just_short).await;
    assert_launches(&outcome, &[0, 0]);
    assert_eq!(
        outcome.stopped_launching.as_deref(),
        Some("the launch rule")
    );
    close_duration(outcome.elapsed, 200);
}

// (c) the cut at the budget, with a run judged
#[tokio::test(start_paused = true)]
async fn at_the_budget_with_a_judged_run_everything_running_is_cut() {
    // Runs 0 and 1 route in 20 s and 21 s; run 0 is judged in 10 s, run 1's judge is slow
    // (300 s); runs 2 and 3 start at 30 s and route for 500 s.
    let fake = fake(
        |i| match i {
            0 => (20, None),
            1 => (21, None),
            _ => (500, None),
        },
        |i| if i == 1 { good(300) } else { good(10) },
    );
    let outcome = run(&plan(2, 1, Some(200), 10), &fake).await;
    assert_launches(&outcome, &[0, 0, 30, 30]);
    assert_eq!(
        outcomes(&outcome),
        vec!["judged", "cut_at_budget", "cut_at_budget", "cut_at_budget"]
    );
    // The search ends at the budget instant, not when the slow jobs would have.
    close_duration(outcome.elapsed, 200);
    assert!(outcome.budget_overrun.is_none());
    assert_eq!(outcome.stopped_launching.as_deref(), Some("the budget"));
    assert_eq!(outcome.winner, Some(0));
    assert!(outcome.keys[0].is_some());
    assert!(outcome.keys[1..].iter().all(Option::is_none));
    // A run cut while its judge ran keeps what it had (its route), and every cut
    // run says why.
    close(outcome.runs[1].routed_s, 21.0);
    assert!(outcome.runs[1].judged.is_none());
    for record in &outcome.runs[1..] {
        assert!(
            record.reason.as_deref().unwrap_or("").contains("budget"),
            "{record:?}"
        );
    }
}

// (d) nothing judged at the budget: wait for the first judge, report the overrun, cut the rest
#[tokio::test(start_paused = true)]
async fn at_the_budget_with_nothing_judged_the_search_waits_for_the_first_judge() {
    // Runs 0 and 1 route in 60 s and 61 s, judges take 80 s, run 2 routes for 500 s.
    let fake = fake(
        |i| match i {
            0 => (60, None),
            1 => (61, None),
            _ => (500, None),
        },
        |_| good(80),
    );
    let outcome = run(&plan(3, 1, Some(100), 3), &fake).await;
    // Run 0 is judged at 140 s, 40 s after the budget; then everything else is cut.
    assert_eq!(
        outcomes(&outcome),
        vec!["judged", "cut_at_budget", "cut_at_budget"]
    );
    close_duration(outcome.elapsed, 140);
    let overrun = outcome.budget_overrun.expect("the budget was overrun");
    close_duration(overrun, 40);
    assert_eq!(outcome.winner, Some(0));
    // The run that was routed but not yet judged keeps its route time.
    close(outcome.runs[1].routed_s, 61.0);
    assert_launches(&outcome, &[0, 0, 0]);
}

#[tokio::test(start_paused = true)]
async fn at_the_budget_with_a_judged_run_the_search_does_not_wait() {
    // The counterpart of the test above: the first judge ends at 90 s, before the
    // budget of 100 s, so the cut is at 100 s and there is no overrun.
    let fake = fake(
        |i| match i {
            0 => (60, None),
            1 => (61, None),
            _ => (500, None),
        },
        |_| good(30),
    );
    let outcome = run(&plan(3, 1, Some(100), 3), &fake).await;
    assert_eq!(
        outcomes(&outcome),
        vec!["judged", "cut_at_budget", "cut_at_budget"]
    );
    close_duration(outcome.elapsed, 100);
    assert!(outcome.budget_overrun.is_none());
    assert_eq!(outcome.winner, Some(0));
}

// (e) the hang cap
#[tokio::test(start_paused = true)]
async fn a_route_past_the_hang_cap_is_hung_and_is_not_retried() {
    // Run 1 would take 400 s; the cap is 100 s.
    let fake = fake(
        |i| match i {
            0 => (30, None),
            1 => (400, None),
            _ => (31, None),
        },
        |_| good(10),
    );
    let base = Instant::now();
    let mut p = plan(2, 1, Some(1000), 5);
    p.hang_cap = secs(100);
    let outcome = run(&p, &fake).await;

    assert_eq!(
        outcomes(&outcome),
        vec!["judged", "hung", "judged", "judged", "judged"]
    );
    close(outcome.runs[1].route_s, 100.0);
    assert!(outcome.runs[1].judged.is_none());
    assert!(outcome.runs[1]
        .reason
        .as_deref()
        .unwrap()
        .contains("hang cap"));
    // Each route was handed its own launch time plus the cap as its deadline.
    for record in &outcome.runs {
        let deadline = fake.deadline_of(record.index);
        close(
            Some(deadline.saturating_duration_since(base).as_secs_f64()),
            record.launched_s + 100.0,
        );
    }
    // The lane the hung route freed was used for the next run, at 100 s, and the
    // hung run's variant was never started again.
    close(Some(outcome.runs[4].launched_s), 100.0);
    assert_eq!(outcome.runs.len(), 5);
    let mut variants: Vec<(Shape, u64)> = outcome.runs.iter().map(|r| (r.shape, r.seed)).collect();
    variants.sort_by_key(|v| v.1);
    variants.dedup();
    assert_eq!(variants.len(), 5);
    assert_eq!(outcome.winner, Some(0));
}

#[tokio::test(start_paused = true)]
async fn a_route_that_ends_under_the_hang_cap_is_judged() {
    // The counterpart: the same 400 s route with a 500 s cap is an ordinary run.
    let fake = fake(
        |i| if i == 1 { (400, None) } else { (30, None) },
        |_| good(10),
    );
    let mut p = plan(2, 1, Some(5000), 3);
    p.hang_cap = secs(500);
    let outcome = run(&p, &fake).await;
    assert_eq!(outcomes(&outcome), vec!["judged"; 3]);
    close(outcome.runs[1].route_s, 400.0);
}

// (f) Fast
#[tokio::test(start_paused = true)]
async fn fast_launches_exactly_two_at_once_and_waits_for_both_judges() {
    let limits = Effort::Fast.limits();
    for order_len in [2usize, 5] {
        let fake = fake(
            |i| if i == 0 { (20, None) } else { (50, None) },
            |_| good(30),
        );
        let p = SearchPlan {
            limits,
            lanes: lanes(14, Effort::Fast),
            order: (0..order_len)
                .map(|i| (Shape::AsExported, i as u64 + 1))
                .collect(),
            hang_cap: HANG_CAP,
        };
        assert_eq!(p.lanes.routers, 2);
        let outcome = run(&p, &fake).await;
        // Two runs at 0, never a third, both judged, nothing cut.
        assert_launches(&outcome, &[0, 0]);
        assert_eq!(outcomes(&outcome), vec!["judged", "judged"]);
        close_duration(outcome.elapsed, 80);
        close(outcome.runs[1].judged_s, 80.0);
        assert!(outcome.budget_overrun.is_none());
        assert_eq!(
            outcome.stopped_launching.as_deref(),
            Some("fixed run count")
        );
        assert_eq!(outcome.winner, Some(0));
    }
}

// (g) the baseline
#[tokio::test(start_paused = true)]
async fn a_failing_baseline_fails_every_judge_with_its_reason() {
    let fake = fake_with_baseline(|_| (10, None), |_| good(10), (5, Err("kicad-cli died")));
    let outcome = run(&plan(2, 2, Some(100), 2), &fake).await;
    assert_eq!(outcomes(&outcome), vec!["judge_failed", "judge_failed"]);
    for record in &outcome.runs {
        let reason = record.reason.as_deref().unwrap();
        assert!(reason.contains("kicad-cli died"), "{reason}");
        assert!(record.judged.is_none());
        // The route itself finished; the run keeps its route record.
        close(record.routed_s, 10.0);
    }
    assert_eq!(outcome.winner, None);
    assert!(outcome.keys.iter().all(Option::is_none));
    assert_eq!(outcome.baseline.as_ref().unwrap_err(), "kicad-cli died");
    // No judge was ever started.
    assert_eq!(fake.judge_calls(), 0);
    close_duration(outcome.elapsed, 10);
}

#[tokio::test(start_paused = true)]
async fn a_working_baseline_lets_the_same_runs_be_judged() {
    let fake = fake(|_| (10, None), |_| good(10));
    let outcome = run(&plan(2, 2, Some(100), 2), &fake).await;
    assert_eq!(outcomes(&outcome), vec!["judged", "judged"]);
    assert!(outcome.baseline.is_ok());
    assert_eq!(outcome.winner, Some(0));
    assert_eq!(fake.judge_calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn no_judge_starts_before_the_baseline_is_done() {
    // Routes end at 10 s; the baseline at 50 s; judges take 5 s.
    let base = Instant::now();
    let fake = fake_with_baseline(|_| (10, None), |_| good(5), (50, Ok(baseline())));
    let outcome = run(&plan(2, 2, Some(1000), 2), &fake).await;
    assert_eq!(outcomes(&outcome), vec!["judged", "judged"]);
    close(outcome.runs[0].routed_s, 10.0);
    close(outcome.runs[0].judged_s, 55.0);
    close(outcome.runs[0].judge_s, 5.0);
    for (_, started) in fake.0.judge_starts.lock().unwrap().iter() {
        assert!(*started >= base + secs(50), "a judge started too early");
    }
    close_duration(outcome.elapsed, 55);
}

// (h) the winner
#[tokio::test(start_paused = true)]
async fn the_winner_is_the_least_key_and_ties_go_to_the_lower_index() {
    // Depends on rank: fewer acute corners is better, a failed G1 is worse than any
    // pass, and equal runs get equal keys. Run 0 has 5 acute corners, runs 1 and 2
    // have 1, run 3 has none but fails G1.
    let fake = fake(
        |_| (10, None),
        |i| {
            let run = match i {
                0 => judged(5.0, false),
                1 | 2 => judged(1.0, false),
                _ => judged(0.0, true),
            };
            (10, Ok(run))
        },
    );
    let outcome = run(&plan(4, 4, Some(1000), 4), &fake).await;
    assert_eq!(outcomes(&outcome), vec!["judged"; 4]);
    assert_eq!(outcome.winner, Some(1));
    let key = |i: usize| outcome.keys[i].unwrap();
    assert_eq!(key(1), key(2));
    assert!(key(1) < key(0));
    assert!(key(1) < key(3));
    assert!(outcome.keys.iter().all(Option::is_some));
}

#[tokio::test(start_paused = true)]
async fn a_failed_route_and_a_failed_judge_are_recorded_and_never_win() {
    let fake = fake(
        |i| {
            if i == 0 {
                (10, Some("boom"))
            } else {
                (10, None)
            }
        },
        |i| match i {
            1 => (10, Err("audit refused")),
            _ => good(10),
        },
    );
    let outcome = run(&plan(4, 4, Some(1000), 4), &fake).await;
    assert_eq!(
        outcomes(&outcome),
        vec!["route_failed", "judge_failed", "judged", "judged"]
    );
    assert_eq!(outcome.runs[0].reason.as_deref(), Some("boom"));
    assert!(outcome.runs[0].routed.is_none());
    assert!(outcome.runs[1]
        .reason
        .as_deref()
        .unwrap()
        .contains("audit refused"));
    // The failed judge's route is kept, its judged run is not.
    assert!(outcome.runs[1].routed.is_some());
    assert!(outcome.runs[1].judged.is_none());
    assert_eq!(outcome.winner, Some(2));
    assert!(outcome.keys[0].is_none() && outcome.keys[1].is_none());
}

// (i) the order runs out
#[tokio::test(start_paused = true)]
async fn the_search_ends_when_the_order_is_exhausted_and_everything_is_judged() {
    let fake = fake(|_| (10, None), |_| good(10));
    let outcome = run(&plan(2, 1, Some(1000), 3), &fake).await;
    assert_eq!(outcomes(&outcome), vec!["judged"; 3]);
    assert_eq!(
        outcome.stopped_launching.as_deref(),
        Some("order exhausted")
    );
    // Run 2 starts when the first judge ends (20 s) and ends at 40 s.
    assert_launches(&outcome, &[0, 0, 20]);
    close_duration(outcome.elapsed, 40);
    assert!(outcome.budget_overrun.is_none());
    assert_eq!(outcome.winner, Some(0));
}

#[tokio::test(start_paused = true)]
async fn under_a_budget_runs_launch_past_the_old_cap_until_the_budget() {
    // Routes of 1 s, judges of 1 s, 3 judge lanes: about 3 runs a second for 400 s.
    // The old cap of 240 runs no longer stops the search; the budget does.
    let limits = Limits {
        budget: Some(secs(400)),
        runs: None,
    };
    let p = SearchPlan {
        limits,
        lanes: Lanes {
            routers: 7,
            judges: 3,
        },
        order: launch_order(&[Shape::AsExported, Shape::Omitted], limits),
        hang_cap: HANG_CAP,
    };
    let fake = fake(|_| (1, None), |_| good(1));
    let outcome = run(&p, &fake).await;
    assert!(outcome.runs.len() > 1000, "{} runs", outcome.runs.len());
    // The launch rule stops launching just before the budget (each run must still
    // be judged in time), or the budget's cut does: never the order, never a cap.
    assert!(
        matches!(
            outcome.stopped_launching.as_deref(),
            Some("the budget" | "the launch rule")
        ),
        "{:?}",
        outcome.stopped_launching
    );
    let elapsed = outcome.elapsed.as_secs_f64();
    assert!((390.0..=400.001).contains(&elapsed), "{elapsed}");
    assert!(fake.most_routes_at_once() <= 7);
    assert!(fake.most_judges_at_once() <= 3);
    // Indexes are launch positions, seeds interleave over the shapes.
    for (i, record) in outcome.runs.iter().enumerate() {
        assert_eq!(record.index, i);
        assert_eq!(Some((record.shape, record.seed)), p.order.get(i));
    }
    assert_eq!(outcome.winner, Some(0));
    // Every judged run but the best KEPT_WHOLE was released; the winner never was.
    let judged: Vec<usize> = outcome
        .runs
        .iter()
        .filter(|r| r.outcome == "judged")
        .map(|r| r.index)
        .collect();
    let released = fake.released();
    assert_eq!(released.len(), judged.len() - KEPT_WHOLE);
    assert!(!released.contains(&0));
}

// The route-failure stop: an endless order whose routes all start failing would
// otherwise launch failing runs until the budget.
#[tokio::test(start_paused = true)]
async fn five_route_failures_in_a_row_stop_launching() {
    // One router lane. Run 0 routes (10 s) and is judged (15 s); from then on every
    // route fails after 10 s: runs at 15, 25, 35, 45, 55, and the fifth failure in a
    // row (at 65 s) stops launching; the search ends there.
    let fake = fake(
        |i| {
            if i == 0 {
                (10, None)
            } else {
                (10, Some("no space left on device"))
            }
        },
        |_| good(5),
    );
    let p = SearchPlan {
        order: launch_order(&[Shape::AsExported], Effort::M.limits()),
        ..plan(1, 1, Some(100_000), 0)
    };
    let outcome = run(&p, &fake).await;
    let mut expected = vec!["judged"];
    expected.extend(["route_failed"; 5]);
    assert_eq!(outcomes(&outcome), expected);
    assert_launches(&outcome, &[0, 15, 25, 35, 45, 55]);
    assert_eq!(
        outcome.stopped_launching.as_deref(),
        Some("5 routes in a row failed")
    );
    close_duration(outcome.elapsed, 65);
    assert_eq!(outcome.winner, Some(0));
    // Each failed run was released at once.
    assert_eq!(fake.released(), vec![1, 2, 3, 4, 5]);
}

#[tokio::test(start_paused = true)]
async fn failures_that_are_not_five_in_a_row_keep_launching() {
    // The negative control: every fourth route succeeds, so at most three fail in
    // a row, and the whole fixed order is launched.
    let fake = fake(
        |i| {
            if i % 4 == 0 {
                (10, None)
            } else {
                (10, Some("the router crashed"))
            }
        },
        |_| good(5),
    );
    let outcome = run(&plan(1, 1, Some(100_000), 12), &fake).await;
    assert_eq!(outcome.runs.len(), 12);
    assert_eq!(
        outcome.stopped_launching.as_deref(),
        Some("order exhausted")
    );
}

#[tokio::test(start_paused = true)]
async fn a_route_in_flight_that_succeeds_lifts_the_stop() {
    // Two router lanes. Run 1 routes for 100 s; meanwhile runs 2 to 6 fail in a row
    // (the fifth at 65 s) and launching stops. Run 1 lands at 100 s, the stop is
    // lifted, and the rest of the order launches from 100 s on.
    let fake = fake(
        |i| match i {
            0 => (10, None),
            1 => (100, None),
            _ => (10, Some("the router crashed")),
        },
        |_| good(5),
    );
    let outcome = run(&plan(2, 1, Some(100_000), 12), &fake).await;
    assert_eq!(outcome.runs.len(), 12);
    for (at, expected) in launches(&outcome)[..8]
        .iter()
        .zip([0, 0, 15, 25, 35, 45, 55, 100])
    {
        close(Some(*at), expected as f64);
    }
    assert_eq!(outcome.runs[1].outcome, "judged");
    assert_eq!(outcome.winner, Some(0));
}

/// Row ZR, ZR5 part 2 (PK12): the search's winner and the runs it releases follow the
/// runner's mode. Twelve runs judged one by one: run i has a signal length that grows
/// with i (today's key prefers the early runs) and a plane loss that shrinks with i
/// (the pour key prefers the late ones). Both keys are kept for every judged run.
/// Control: `finish` or `release_beyond_the_best` left on `route_judge::rank`.
#[tokio::test(start_paused = true)]
async fn the_winner_and_the_released_runs_follow_the_mode() {
    use crate::pour_rank::RankMode;
    let pool = |i: usize| {
        crate::pour_rank::tests::split_judged(100.0 + 2.0 * i as f64, 0.05 * (11 - i) as f64)
    };
    for (mode, winner, released) in [
        (RankMode::Routing, 0, vec![8, 9, 10, 11]),
        (RankMode::Pours, 11, vec![0, 1, 2, 3]),
    ] {
        let fake = fake(|_| (10, None), move |i| (5, Ok(pool(i))));
        fake.0
            .pours
            .store(mode == RankMode::Pours, Ordering::SeqCst);
        let outcome = run(&plan(1, 1, Some(100_000), 12), &fake).await;
        assert_eq!(outcomes(&outcome), vec!["judged"; 12], "{mode:?}");
        assert_eq!(outcome.rank, mode);
        assert_eq!(outcome.winner, Some(winner), "{mode:?}");
        assert_eq!(fake.released(), released, "{mode:?}");
        // Both keys, by index, whatever ranked.
        let judged: Vec<Judged> = (0..12).map(pool).collect();
        let refs: Vec<&Judged> = judged.iter().collect();
        let pooled: Vec<(&Measures, &Gates, bool)> =
            refs.iter().map(|j| (&j.measures, &j.gates, true)).collect();
        let today: Vec<Option<Key>> = crate::route_judge::rank(&refs)
            .into_iter()
            .map(Some)
            .collect();
        let poured: Vec<Option<crate::pour_rank::PourKey>> = crate::pour_rank::pour_keys(&pooled)
            .into_iter()
            .map(Some)
            .collect();
        assert_eq!(outcome.keys, today, "{mode:?}");
        assert_eq!(outcome.pour_keys, poured, "{mode:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn runs_outside_the_best_are_released_and_the_best_are_kept() {
    // Twelve runs judged one by one, each with more acute corners than the last but
    // run 11, the best of all. The best KEPT_WHOLE (8) by the key so far keep their
    // folders; a run pushed out of them is released then.
    let fake = fake(
        |_| (10, None),
        |i| {
            let acute = if i == 11 { 0.0 } else { 1.0 + i as f64 };
            (5, Ok(judged(acute, false)))
        },
    );
    let outcome = run(&plan(1, 1, Some(100_000), 12), &fake).await;
    assert_eq!(outcomes(&outcome), vec!["judged"; 12]);
    assert_eq!(outcome.winner, Some(11));
    // Runs 0..=7 are the best eight until run 8 comes; then 8, 9 and 10 are
    // released as they are judged, and run 11 pushes run 7 out.
    assert_eq!(fake.released(), vec![7, 8, 9, 10]);
}

#[tokio::test(start_paused = true)]
async fn an_empty_order_ends_at_once_with_nothing() {
    let fake = fake(|_| (10, None), |_| good(10));
    let outcome = run(&plan(3, 1, Some(100), 0), &fake).await;
    assert!(outcome.runs.is_empty());
    assert_eq!(outcome.winner, None);
    assert_eq!(outcome.stopped_launching, None);
    assert!(outcome.baseline.is_err());
    close_duration(outcome.elapsed, 0);
}

#[tokio::test(start_paused = true)]
async fn judge_lanes_are_never_exceeded() {
    // Seven routes end together; one judge lane judges them one after another.
    let fake = fake(|_| (10, None), |_| good(5));
    let outcome = run(&plan(7, 1, Some(1000), 7), &fake).await;
    assert_eq!(outcomes(&outcome), vec!["judged"; 7]);
    assert_eq!(fake.most_judges_at_once(), 1);
    assert_eq!(fake.most_routes_at_once(), 7);
    // One at a time, 5 s each, from 10 s on.
    let mut done: Vec<f64> = outcome.runs.iter().map(|r| r.judged_s.unwrap()).collect();
    done.sort_by(f64::total_cmp);
    for (i, at) in done.iter().enumerate() {
        close(Some(*at), 15.0 + 5.0 * i as f64);
    }
}

// The cut keeps jobs that finished at the very instant of the budget.
#[tokio::test(start_paused = true)]
async fn the_cut_keeps_a_judge_and_a_route_that_finished_at_the_budget_instant() {
    // Run 0 is judged at 20 s. Runs 1 and 2 are judged from 10 s on, 90 s each, so
    // both judges end at 100 s, the budget. Run 3 routes for exactly 100 s.
    // Without collecting finished jobs before the abort, the judges that ended at
    // the budget are recorded cut and their results are lost.
    let fake = fake(
        |i| if i == 3 { (100, None) } else { (10, None) },
        |i| if i == 0 { good(10) } else { good(90) },
    );
    let outcome = run(&plan(4, 3, Some(100), 4), &fake).await;
    assert_eq!(
        outcomes(&outcome),
        vec!["judged", "judged", "judged", "cut_at_budget"]
    );
    close_duration(outcome.elapsed, 100);
    assert!(outcome.budget_overrun.is_none());
    close(outcome.runs[1].judged_s, 100.0);
    close(outcome.runs[2].judged_s, 100.0);
    assert!(outcome.runs[1].judged.is_some() && outcome.runs[2].judged.is_some());
    // The route that ended at the budget is kept as routed, then cut: no judge starts.
    close(outcome.runs[3].routed_s, 100.0);
    assert!(outcome.runs[3].routed.is_some());
    assert!(outcome.runs[3].judged.is_none());
    assert_eq!(outcome.winner, Some(0));
    assert!(outcome.keys[..3].iter().all(Option::is_some));
}

// The judge that fails on every run.
fn failing(seconds: u64) -> JudgeSpec {
    (seconds, Err("the importer refused the session"))
}

#[tokio::test(start_paused = true)]
async fn three_judge_failures_and_no_success_stop_launching() {
    // One lane each: route 10 s, judge 5 s. Run 1 waits for the first judge (15 s);
    // run 2 starts when run 1's route ends (25 s), run 3 when run 2's does (35 s).
    // The third failure comes at 40 s, so run 3, already routing, is the last.
    let fake = fake(|_| (10, None), |_| failing(5));
    let outcome = run(&plan(1, 1, Some(100_000), 10), &fake).await;
    assert_eq!(outcomes(&outcome), vec!["judge_failed"; 4]);
    assert_launches(&outcome, &[0, 15, 25, 35]);
    assert_eq!(
        outcome.stopped_launching.as_deref(),
        Some("the judge failed on every run so far")
    );
    // No cut: the search simply ended with the last judge.
    close_duration(outcome.elapsed, 50);
    assert_eq!(outcome.winner, None);
    for record in &outcome.runs {
        assert!(record
            .reason
            .as_deref()
            .unwrap()
            .contains("importer refused"));
    }
}

#[tokio::test(start_paused = true)]
async fn a_late_success_lifts_the_stop_and_launching_resumes() {
    // Run 1 routes for 200 s and is judged well; the other runs fail their judges.
    // The third failure (at 40 s) stops launching while run 1 is still routing; run
    // 4, already routing, finishes. Run 1's success at 205 s lifts the stop, both
    // router lanes fill at once, and the rest of the order is launched (failures
    // after a success never stop it again).
    let fake = fake(
        |i| if i == 1 { (200, None) } else { (10, None) },
        |i| if i == 1 { good(5) } else { failing(5) },
    );
    let outcome = run(&plan(2, 1, Some(100_000), 10), &fake).await;
    assert_eq!(outcome.runs.len(), 10);
    let launched = launches(&outcome);
    // Nothing launched between the stop (40 s) and the success (205 s).
    for (at, expected) in launched[..7].iter().zip([0, 0, 15, 25, 35, 205, 205]) {
        close(Some(*at), expected as f64);
    }
    assert_eq!(outcome.runs[1].outcome, "judged");
    assert!(outcome
        .runs
        .iter()
        .enumerate()
        .all(|(i, r)| i == 1 || r.outcome == "judge_failed"));
    assert_eq!(
        outcome.stopped_launching.as_deref(),
        Some("order exhausted")
    );
    assert_eq!(outcome.winner, Some(1));
}

#[tokio::test(start_paused = true)]
async fn two_failures_then_a_success_keep_launching() {
    // The negative control: runs 0 and 1 fail, run 2 succeeds, so the rule never
    // trips and the whole order is launched.
    let fake = fake(|_| (10, None), |i| if i < 2 { failing(5) } else { good(5) });
    let outcome = run(&plan(1, 1, Some(100_000), 6), &fake).await;
    assert_eq!(outcome.runs.len(), 6);
    assert_eq!(outcome.runs[0].outcome, "judge_failed");
    assert_eq!(outcome.runs[1].outcome, "judge_failed");
    assert!(outcome.runs[2..].iter().all(|r| r.outcome == "judged"));
    assert_eq!(
        outcome.stopped_launching.as_deref(),
        Some("order exhausted")
    );
    assert_eq!(outcome.winner, Some(2));
}

#[tokio::test(start_paused = true)]
async fn failures_after_a_success_do_not_stop_launching() {
    // A success first, then three failures in a row: a judge that works on some
    // runs is not a deterministic failure.
    let fake = fake(
        |_| (10, None),
        |i| {
            if i == 0 || i >= 4 {
                good(5)
            } else {
                failing(5)
            }
        },
    );
    let outcome = run(&plan(1, 1, Some(100_000), 6), &fake).await;
    assert_eq!(outcome.runs.len(), 6);
    assert_eq!(
        outcome.stopped_launching.as_deref(),
        Some("order exhausted")
    );
    assert_eq!(outcome.winner, Some(0));
}

// Row LT, L23: the scheduler rewrites a judged run's `held` from the judge's choice, so
// the session the call writes and the pass it reports are the judged board's.
#[tokio::test(start_paused = true)]
async fn a_judged_runs_held_session_follows_the_judges_choice_among_its_tune_candidates() {
    use crate::fastroute::TuneVariant;
    use crate::route_judge::TuneChoice;
    for (chosen, ses, kept, unrouted) in [
        (Some(TuneVariant::PourSafe), "tune-pour-safe.ses", "tune", 1),
        (Some(TuneVariant::Free), "tune.ses", "tune", 0),
        // None: the untuned session, as routing left it.
        (None, "variant.ses", "full", 0),
    ] {
        let fake = fake(
            |_| (10, None),
            move |_| {
                let mut run = judged(1.0, false);
                run.tune = Some(TuneChoice {
                    chosen,
                    rows: Vec::new(),
                    untuned: run.measures.clone(),
                });
                (10, Ok(run))
            },
        );
        fake.0.tuned.store(true, Ordering::SeqCst);
        let outcome = run(&plan(1, 1, None, 1), &fake).await;
        assert_eq!(outcomes(&outcome), vec!["judged"]);
        let held = &outcome.runs[0].routed.as_ref().unwrap().held;
        assert_eq!(held.ses, PathBuf::from(ses), "{chosen:?}");
        assert_eq!((held.kept, held.unrouted), (kept, unrouted), "{chosen:?}");
    }
    // Control: a run with no Tune candidates keeps its session.
    let fake = fake(|_| (10, None), |_| good(10));
    let outcome = run(&plan(1, 1, None, 1), &fake).await;
    let held = &outcome.runs[0].routed.as_ref().unwrap().held;
    assert_eq!(
        (held.ses.clone(), held.kept),
        (PathBuf::from("variant.ses"), "full")
    );
}

// Row LT, N1: a choice the run cannot follow fails the judge, so a session that differs from
// the judged board is never written.
#[tokio::test(start_paused = true)]
async fn a_choice_naming_a_candidate_the_run_does_not_hold_fails_the_judge() {
    use crate::fastroute::TuneVariant;
    use crate::route_judge::TuneChoice;
    // The runs are routed without Tune candidates, and the judge names one.
    let fake = fake(
        |_| (10, None),
        |_| {
            let mut run = judged(1.0, false);
            run.tune = Some(TuneChoice {
                chosen: Some(TuneVariant::PourSafe),
                rows: Vec::new(),
                untuned: run.measures.clone(),
            });
            (10, Ok(run))
        },
    );
    let outcome = run(&plan(1, 1, None, 1), &fake).await;
    assert_eq!(outcomes(&outcome), vec!["judge_failed"]);
    let reason = outcome.runs[0].reason.as_deref().unwrap();
    assert!(reason.contains("does not hold"), "{reason}");
    assert!(outcome.runs[0].judged.is_none());
    assert_eq!(outcome.winner, None);
}
