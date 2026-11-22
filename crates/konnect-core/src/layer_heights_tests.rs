//! Tests of the layer heights of a fastroute call (`layer_heights.rs`, L7). Each
//! guard has a test that it holds and a counterpart on the honest case: that
//! counterpart is the guard's negative control.

use super::*;

const FOUR_LAYER: &str =
    include_str!("../../konnect-ipc/tests/fixtures/four_layer_stackup_kicad10.kicad_pcb");

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| name.to_string()).collect()
}

fn parsed(text: &str) -> SexpNode {
    parse_sexp(text).unwrap()
}

/// One stackup layer: `(layer "name" (type "kind") (thickness t) …)`, each further
/// thickness as a KiCad sublayer.
fn item(name: &str, kind: &str, thicknesses: &[f64]) -> String {
    let mut text = format!("(layer \"{name}\" (type \"{kind}\")");
    for (index, thickness) in thicknesses.iter().enumerate() {
        if index > 0 {
            text.push_str(" addsublayer");
        }
        text.push_str(&format!(" (thickness {thickness})"));
    }
    text.push(')');
    text
}

/// A board with `(general (thickness t))`, the given copper layers (KiCad's ids:
/// F.Cu 0, B.Cu 2, inner layers from 4), and the given masks and stackup text.
fn board(thickness: Option<f64>, copper: &[&str], masks: bool, stackup: Option<&str>) -> String {
    let mut text = String::from("(kicad_pcb (version 20240108)\n");
    if let Some(thickness) = thickness {
        text.push_str(&format!("  (general (thickness {thickness}))\n"));
    }
    text.push_str("  (layers\n");
    for (index, name) in copper.iter().enumerate() {
        text.push_str(&format!("    ({} \"{name}\" signal)\n", index * 2));
    }
    text.push_str("    (13 \"F.Paste\" user)\n");
    if masks {
        text.push_str("    (1 \"F.Mask\" user)\n    (3 \"B.Mask\" user)\n");
    }
    text.push_str("  )\n");
    if let Some(stackup) = stackup {
        text.push_str(&format!("  (setup (stackup {stackup}))\n"));
    }
    text.push_str(")\n");
    text
}

/// A four-layer stackup with silk, paste and mask around it, and sublayers in the
/// second dielectric.
fn stackup_text() -> String {
    [
        item("F.SilkS", "Top Silk Screen", &[]),
        item("F.Paste", "Top Solder Paste", &[]),
        item("F.Mask", "Top Solder Mask", &[0.01]),
        item("F.Cu", "copper", &[0.035]),
        item("dielectric 1", "prepreg", &[0.1]),
        item("In1.Cu", "copper", &[0.0152]),
        item("dielectric 2", "core", &[0.2, 0.3]),
        item("In2.Cu", "copper", &[0.0152]),
        item("dielectric 3", "prepreg", &[0.1]),
        item("B.Cu", "copper", &[0.035]),
        item("B.Mask", "Bottom Solder Mask", &[0.01]),
    ]
    .join("\n")
}

fn four_layer_board() -> String {
    board(
        Some(1.0),
        &["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"],
        true,
        Some(&stackup_text()),
    )
}

fn heights_of(result: &LayerHeights) -> Vec<f64> {
    result.heights_mm.clone().expect("heights are sent")
}

fn close(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-9
}

// ─── The stackup ────────────────────────────────────────────────────────────

#[test]
fn sublayers_are_summed_and_mask_silk_and_paste_are_skipped() {
    let items = stackup_items(&parsed(&four_layer_board())).unwrap();
    let kinds: Vec<(Option<&str>, f64)> = items
        .iter()
        .map(|item| (item.layer.as_deref(), item.thickness_mm))
        .collect();
    assert_eq!(
        kinds.iter().map(|(layer, _)| *layer).collect::<Vec<_>>(),
        vec![
            Some("F.Cu"),
            None,
            Some("In1.Cu"),
            None,
            Some("In2.Cu"),
            None,
            Some("B.Cu")
        ],
        "mask, silk and paste are not items"
    );
    // The second dielectric has two sublayers: 0.2 + 0.3.
    assert!(close(kinds[3].1, 0.5), "{kinds:?}");
    assert!(close(kinds[0].1, 0.035) && close(kinds[2].1, 0.0152));
}

#[test]
fn a_board_without_a_stackup_or_with_an_unusable_item_gives_none() {
    // No stackup at all.
    assert_eq!(
        stackup_items(&parsed(&board(None, &["F.Cu", "B.Cu"], false, None))),
        None
    );
    // A copper item without a thickness, a zero, a negative one and a dielectric
    // without one are each unusable.
    for broken in [
        item("F.Cu", "copper", &[]),
        item("F.Cu", "copper", &[0.0]),
        item("F.Cu", "copper", &[-0.035]),
    ] {
        let stackup = [
            broken,
            item("dielectric 1", "core", &[1.5]),
            item("B.Cu", "copper", &[0.035]),
        ]
        .join("\n");
        let text = board(None, &["F.Cu", "B.Cu"], false, Some(&stackup));
        assert_eq!(stackup_items(&parsed(&text)), None);
    }
    let stackup = [
        item("F.Cu", "copper", &[0.035]),
        item("dielectric 1", "core", &[]),
        item("B.Cu", "copper", &[0.035]),
    ]
    .join("\n");
    let text = board(None, &["F.Cu", "B.Cu"], false, Some(&stackup));
    assert_eq!(stackup_items(&parsed(&text)), None);
    // Control: the same stackup with every thickness is read.
    let stackup = [
        item("F.Cu", "copper", &[0.035]),
        item("dielectric 1", "core", &[1.5]),
        item("B.Cu", "copper", &[0.035]),
    ]
    .join("\n");
    let text = board(None, &["F.Cu", "B.Cu"], false, Some(&stackup));
    assert_eq!(stackup_items(&parsed(&text)).unwrap().len(), 3);
}

#[test]
fn a_real_kicad_stackup_is_read_as_kicad_wrote_it() {
    let heights = layer_heights(
        FOUR_LAYER,
        None,
        &names(&["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]),
    );
    assert_eq!(heights.source, Some(HeightSource::BoardStackup));
    assert_eq!(heights.note, None);
    // 0.035 + 0.2104 + 0.0152, then + 0.25 + 0.0152, then + 0.2104 + 0.035.
    let expected = [0.0, 0.2606, 0.5258, 0.7712];
    for (got, want) in heights_of(&heights).iter().zip(expected) {
        assert!(close(*got, want), "{got} vs {want}");
    }
}

// ─── KiCad's default stackup ────────────────────────────────────────────────

#[test]
fn a_board_without_a_stackup_gets_kicads_default_formula() {
    // Four copper layers, 1.6 mm, both masks: (1.6 - 4 x 0.035 - 2 x 0.01) / 3 = 0.48.
    let text = board(Some(1.6), &["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"], true, None);
    let items = default_stackup(&parsed(&text)).unwrap();
    assert_eq!(items.len(), 7);
    assert!(close(items[1].thickness_mm, 0.48), "{items:?}");
    let heights = layer_heights(&text, None, &names(&["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]));
    assert_eq!(heights.source, Some(HeightSource::KicadDefault));
    assert!(heights.note.as_deref().unwrap().contains("default stackup"));
    assert_eq!(heights.switch_value().as_deref(), Some("0,0.55,1.065,1.58"));
    // Two layers: 1.51 of dielectric, 1.58 to the bottom (the step-0 value).
    let two = board(Some(1.6), &["F.Cu", "B.Cu"], true, None);
    assert_eq!(
        layer_heights(&two, None, &names(&["F.Cu", "B.Cu"]))
            .switch_value()
            .as_deref(),
        Some("0,1.58")
    );
    // Without masks the 10 µm are not taken off; without `general` the board is 1.6.
    let bare = board(None, &["F.Cu", "B.Cu"], false, None);
    let items = default_stackup(&parsed(&bare)).unwrap();
    assert!(close(items[1].thickness_mm, 1.6 - 0.07), "{items:?}");
    // A thinner board: 0.8 mm, two layers, both masks.
    let thin = board(Some(0.8), &["F.Cu", "B.Cu"], true, None);
    let items = default_stackup(&parsed(&thin)).unwrap();
    assert!(close(items[1].thickness_mm, 0.8 - 0.07 - 0.02), "{items:?}");
}

#[test]
fn the_default_stackup_puts_the_copper_layers_in_stack_order() {
    // The ids of KiCad's `(layers …)` put B.Cu before the inner layers; the stack
    // does not. Ten inner layers show the numeric order (In10 after In9).
    let mut text = String::from(
        "(kicad_pcb (general (thickness 3.2)) (layers (0 \"F.Cu\" signal) (2 \"B.Cu\" signal)",
    );
    for number in (1..=10).rev() {
        text.push_str(&format!(" ({} \"In{number}.Cu\" signal)", number * 2 + 2));
    }
    text.push_str("))");
    let items = default_stackup(&parsed(&text)).unwrap();
    let copper: Vec<&str> = items
        .iter()
        .filter_map(|item| item.layer.as_deref())
        .collect();
    assert_eq!(copper.first(), Some(&"F.Cu"));
    assert_eq!(copper.last(), Some(&"B.Cu"));
    assert_eq!(copper[1], "In1.Cu");
    assert_eq!(copper[9], "In9.Cu");
    assert_eq!(copper[10], "In10.Cu");
    assert_eq!(copper.len(), 12);
}

#[test]
fn a_default_stackup_that_cannot_be_built_is_an_error() {
    // No copper layer.
    assert!(default_stackup(&parsed("(kicad_pcb (layers (13 \"F.Paste\" user)))")).is_err());
    assert!(default_stackup(&parsed("(kicad_pcb)")).is_err());
    // A board too thin for its copper: the dielectric would not be positive.
    let thin = board(Some(0.05), &["F.Cu", "B.Cu"], true, None);
    assert!(default_stackup(&parsed(&thin)).is_err());
    let result = layer_heights(&thin, None, &names(&["F.Cu", "B.Cu"]));
    assert_eq!(result.heights_mm, None);
    assert!(result.note.as_deref().unwrap().contains("default"));
    // Control: the same board at 1.6 mm builds.
    let fine = board(Some(1.6), &["F.Cu", "B.Cu"], true, None);
    assert!(default_stackup(&parsed(&fine)).is_ok());
    // One copper layer needs no dielectric.
    let single = board(Some(1.6), &["F.Cu"], false, None);
    let items = default_stackup(&parsed(&single)).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        layer_heights(&single, None, &names(&["F.Cu"])).heights_mm,
        Some(vec![0.0])
    );
}

// ─── The heights ────────────────────────────────────────────────────────────

#[test]
fn the_heights_follow_the_dsn_order_not_the_stack_order() {
    let text = four_layer_board();
    let stack = layer_heights(&text, None, &names(&["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]));
    let reversed = layer_heights(&text, None, &names(&["B.Cu", "In2.Cu", "In1.Cu", "F.Cu"]));
    let mut flipped = heights_of(&stack);
    flipped.reverse();
    assert_eq!(heights_of(&reversed), flipped);
    // The top copper layer is 0 wherever it sits in the DSN.
    assert_eq!(*heights_of(&reversed).last().unwrap(), 0.0);
    // Heights are cumulative: F 0, In1 0.035+0.1+0.0152, In2 + 0.5 + 0.0152, B + 0.1 + 0.035.
    let expected = [0.0, 0.1502, 0.6654, 0.8004];
    for (got, want) in heights_of(&stack).iter().zip(expected) {
        assert!(close(*got, want), "{got} vs {want}");
    }
}

#[test]
fn a_project_that_does_not_count_via_heights_sends_none() {
    let text = four_layer_board();
    let layers = names(&["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]);
    let off = r#"{"board":{"design_settings":{"rules":{"use_height_for_length_calcs":false}}}}"#;
    let result = layer_heights(&text, Some(off), &layers);
    assert_eq!(result.heights_mm, None);
    assert_eq!(result.switch_value(), None);
    assert!(result
        .note
        .as_deref()
        .unwrap()
        .contains("use_height_for_length_calcs"));
    assert_eq!(result.json()["sent"], false);
    // Controls: on, absent, a project without the key, a project that does not parse,
    // and no project at all each send heights.
    let on = r#"{"board":{"design_settings":{"rules":{"use_height_for_length_calcs":true}}}}"#;
    for project in [Some(on), Some("{}"), Some("not json"), None] {
        assert!(use_height_for_length_calcs(project), "{project:?}");
        assert!(
            layer_heights(&text, project, &layers).heights_mm.is_some(),
            "{project:?}"
        );
    }
    assert!(!use_height_for_length_calcs(Some(off)));
}

#[test]
fn a_dsn_layer_missing_from_the_stackup_sends_none_with_a_note() {
    let text = four_layer_board();
    let result = layer_heights(&text, None, &names(&["F.Cu", "In9.Cu", "B.Cu"]));
    assert_eq!(result.heights_mm, None);
    assert_eq!(result.source, Some(HeightSource::BoardStackup));
    let note = result.note.as_deref().unwrap();
    assert!(note.contains("'In9.Cu'"), "{note}");
    // A DSN that names a layer of the stackup only partly is fine: a subset is sent.
    let subset = layer_heights(&text, None, &names(&["F.Cu", "B.Cu"]));
    assert_eq!(subset.heights_mm.as_ref().map(Vec::len), Some(2));
    // A DSN with no layer sends nothing and says so.
    let empty = layer_heights(&text, None, &[]);
    assert_eq!(empty.heights_mm, None);
    assert!(empty.note.is_some());
}

#[test]
fn a_board_that_does_not_parse_or_has_a_broken_stackup_sends_none_with_a_note() {
    let layers = names(&["F.Cu", "B.Cu"]);
    let result = layer_heights("(kicad_pcb (", None, &layers);
    assert_eq!(result.heights_mm, None);
    assert!(result.note.as_deref().unwrap().contains("does not parse"));
    // A stackup that exists but has a copper layer without a thickness does not fall
    // back to the default: KiCad's own stackup would differ from it.
    let stackup = [
        item("F.Cu", "copper", &[]),
        item("dielectric 1", "core", &[1.5]),
        item("B.Cu", "copper", &[0.035]),
    ]
    .join("\n");
    let text = board(Some(1.6), &["F.Cu", "B.Cu"], true, Some(&stackup));
    let result = layer_heights(&text, None, &layers);
    assert_eq!(result.heights_mm, None);
    assert_eq!(result.source, None);
    assert!(result.note.as_deref().unwrap().contains("stackup"));
}

// ─── What fastroute counts against what KiCad counts ────────────────────────

#[test]
fn a_via_from_the_top_layer_is_kicads_distance_and_a_lower_pair_is_short_by_the_upper_copper() {
    let text = four_layer_board();
    let layers = ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"];
    let items = stackup_items(&parsed(&text)).unwrap();
    let heights = heights_of(&layer_heights(&text, None, &names(&layers)));
    let copper = |name: &str| {
        items
            .iter()
            .find(|item| item.layer.as_deref() == Some(name))
            .unwrap()
            .thickness_mm
    };
    let mut checked = 0;
    for (upper, a) in layers.iter().enumerate() {
        for (lower, b) in layers.iter().enumerate().skip(upper + 1) {
            let kicad = kicad_layer_distance_mm(&items, a, b).unwrap();
            let fastroute = heights[lower] - heights[upper];
            if upper == 0 {
                assert!(
                    close(kicad, fastroute),
                    "{a} to {b}: {kicad} vs {fastroute}"
                );
            } else {
                assert!(
                    close(kicad - fastroute, copper(a)),
                    "{a} to {b}: short by {} not {}",
                    kicad - fastroute,
                    copper(a)
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 6);
}

#[test]
fn kicads_layer_distance_counts_both_copper_layers_either_way_round() {
    let items = stackup_items(&parsed(&four_layer_board())).unwrap();
    // F.Cu to In1.Cu: 0.035 + 0.1 + 0.0152.
    let down = kicad_layer_distance_mm(&items, "F.Cu", "In1.Cu").unwrap();
    assert!(close(down, 0.1502), "{down}");
    assert_eq!(
        kicad_layer_distance_mm(&items, "In1.Cu", "F.Cu"),
        Some(down)
    );
    // The same layer is 0; a layer that is not copper, or not there, is none.
    assert_eq!(
        kicad_layer_distance_mm(&items, "In1.Cu", "In1.Cu"),
        Some(0.0)
    );
    assert_eq!(
        kicad_layer_distance_mm(&items, "F.Cu", "dielectric 1"),
        None
    );
    assert_eq!(kicad_layer_distance_mm(&items, "F.Cu", "In9.Cu"), None);
    assert_eq!(kicad_layer_distance_mm(&items, "Nope", "F.Cu"), None);
}

// ─── The switch value and the result ────────────────────────────────────────

#[test]
fn the_switch_value_is_rounded_to_the_micrometre_without_trailing_zeros() {
    let heights = LayerHeights {
        heights_mm: Some(vec![0.0, 0.550_000_000_1, 1.064_999_999, 1.58, 2.0, 0.0004]),
        source: Some(HeightSource::KicadDefault),
        note: None,
    };
    assert_eq!(
        heights.switch_value().as_deref(),
        Some("0,0.55,1.065,1.58,2,0")
    );
    assert_eq!(LayerHeights::default().switch_value(), None);
}

#[test]
fn the_result_json_says_what_was_sent_and_where_it_came_from() {
    let sent = layer_heights(
        &board(Some(1.6), &["F.Cu", "B.Cu"], true, None),
        None,
        &names(&["F.Cu", "B.Cu"]),
    )
    .json();
    assert_eq!(sent["sent"], true);
    assert_eq!(sent["source"], "kicad_default");
    assert_eq!(sent["heights_mm"], json!([0.0, 1.58]));
    assert!(sent["note"].as_str().unwrap().contains("default"));
    let own = layer_heights(
        &four_layer_board(),
        None,
        &names(&["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]),
    )
    .json();
    assert_eq!(own["source"], "board_stackup");
    assert!(own.get("note").is_none(), "no note without a reason: {own}");
    let none = LayerHeights::default().json();
    assert_eq!(none["sent"], false);
    assert!(none["source"].is_null() && none["heights_mm"].is_null());
}

#[test]
fn a_stackup_without_a_copper_item_gives_none() {
    let stackup = [
        item("dielectric 1", "core", &[1.5]),
        item("F.Mask", "Top Solder Mask", &[0.01]),
    ]
    .join("\n");
    let text = board(None, &["F.Cu", "B.Cu"], false, Some(&stackup));
    assert_eq!(stackup_items(&parsed(&text)), None);
}
