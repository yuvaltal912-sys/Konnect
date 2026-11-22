//! Row LT: `dsn_layer_names` and `dsn_class_names` (docs/LENGTH_TUNING_DESIGN.md, L6
//! and L7). Each guard has a test that it holds and a counterpart on the honest case:
//! that counterpart is the guard's negative control.

use super::*;

const POURS: &str = include_str!("../tests/fixtures/specctra_pours_routing.dsn");

/// A DSN with the given layer blocks in its structure and the given class names in
/// its network.
fn dsn(layers: &[&str], classes: &[&str]) -> String {
    let mut text = String::from("(pcb board.kicad_pcb\n  (structure\n");
    for layer in layers {
        text.push_str(&format!("    {layer}\n"));
    }
    text.push_str("    (boundary (path pcb 0 0 0 10 0 10 10 0 10 0 0))\n  )\n  (network\n");
    for class in classes {
        text.push_str(&format!(
            "    (class {class} GND\n      (rule (width 250))\n    )\n"
        ));
    }
    text.push_str("  )\n)\n");
    text
}

fn layer(name: &str, index: usize) -> String {
    format!("(layer {name} (type signal) (property (index {index})))")
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| name.to_string()).collect()
}

#[test]
fn the_layers_are_read_in_file_order_with_the_index_beside_each() {
    let text = dsn(
        &[
            &layer("F.Cu", 0),
            &layer("In1.Cu", 1),
            &layer("In2.Cu", 2),
            &layer("B.Cu", 3),
        ],
        &[],
    );
    assert_eq!(
        dsn_layer_names(&text).unwrap(),
        names(&["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"])
    );
    // A real export: two layers.
    assert_eq!(dsn_layer_names(POURS).unwrap(), names(&["F.Cu", "B.Cu"]));
    // Keywords in any letter case; a quoted name is read without its quotes.
    let upper = "(PCB b (STRUCTURE (LAYER \"Top Layer\" (TYPE signal) (PROPERTY (INDEX 0))) \
                 (LAYER Bottom (type signal) (property (index 1)))))";
    assert_eq!(
        dsn_layer_names(upper).unwrap(),
        names(&["Top Layer", "Bottom"])
    );
    // No layer: none (as `dsn_net_names` gives no net).
    assert!(dsn_layer_names("(pcb b (structure))").unwrap().is_empty());
}

#[test]
fn a_dsn_whose_file_order_is_not_its_index_order_is_refused() {
    // The guard: heights read against the wrong layers would be silently wrong.
    let swapped = dsn(&[&layer("B.Cu", 1), &layer("F.Cu", 0)], &[]);
    let error = dsn_layer_names(&swapped).unwrap_err().to_string();
    assert!(
        error.contains("'B.Cu'") && error.contains("index order"),
        "{error}"
    );
    // An index that skips, or does not start at 0, is not the file order either.
    for indexes in [[1usize, 2], [0, 2], [0, 0]] {
        let text = dsn(
            &[&layer("F.Cu", indexes[0]), &layer("B.Cu", indexes[1])],
            &[],
        );
        assert!(dsn_layer_names(&text).is_err(), "{indexes:?}");
    }
    // Control: the same layers in index order are accepted.
    let ordered = dsn(&[&layer("F.Cu", 0), &layer("B.Cu", 1)], &[]);
    assert_eq!(dsn_layer_names(&ordered).unwrap(), names(&["F.Cu", "B.Cu"]));
}

#[test]
fn a_layer_without_an_index_or_with_a_bad_one_is_refused() {
    for bad in [
        "(layer F.Cu (type signal))",
        "(layer F.Cu (type signal) (property))",
        "(layer F.Cu (type signal) (property (index)))",
        "(layer F.Cu (type signal) (property (index one)))",
        "(layer F.Cu (type signal) (property (index -1)))",
    ] {
        let error = dsn_layer_names(&dsn(&[bad], &[])).unwrap_err().to_string();
        assert!(
            error.contains("F.Cu") || error.contains("layer"),
            "{bad}: {error}"
        );
    }
    // Control: the same layer with an index is read.
    assert_eq!(
        dsn_layer_names(&dsn(&[&layer("F.Cu", 0)], &[])).unwrap(),
        names(&["F.Cu"])
    );
    // A name used twice is refused.
    let twice = dsn(&[&layer("F.Cu", 0), &layer("F.Cu", 1)], &[]);
    assert!(dsn_layer_names(&twice).is_err());
    // Not a DSN, or not a pcb.
    assert!(dsn_layer_names("(session x)").is_err());
    assert!(dsn_layer_names("not a dsn (").is_err());
}

#[test]
fn the_classes_are_the_network_classes_in_order_each_once() {
    let text = dsn(
        &[],
        &[
            "konnect_class_0001",
            "ddr",
            "konnect_class_0001",
            "\"my class\"",
        ],
    );
    assert_eq!(
        dsn_class_names(&text).unwrap(),
        names(&["konnect_class_0001", "ddr", "my class"])
    );
    assert_eq!(
        dsn_class_names(POURS).unwrap(),
        names(&["konnect_class_0001"])
    );
    // A class named in the wiring or elsewhere is not read; no network gives none.
    assert!(dsn_class_names("(pcb b (structure) (wiring (class x y)))")
        .unwrap()
        .is_empty());
    assert!(dsn_class_names("(session x)").is_err());
    assert!(dsn_class_names("not a dsn (").is_err());
}

#[test]
fn a_class_name_with_a_comma_is_left_out() {
    // fastroute splits the ignored-class list at commas, so such a name cannot be listed.
    let text = dsn(&[], &["\"a,b\"", "plain"]);
    assert_eq!(dsn_class_names(&text).unwrap(), names(&["plain"]));
    // Control: a name without one is kept.
    let text = dsn(&[], &["\"a b\"", "plain"]);
    assert_eq!(dsn_class_names(&text).unwrap(), names(&["a b", "plain"]));
}

/// The exporter and the Tune pass's via-stub guard (`via_disc_radii`) agree: a real export
/// with two via sizes gives each via padstack of the DSN's `(via ...)` list an entry, at half
/// its diameter, and nothing else is a via.
#[test]
fn the_via_discs_of_a_real_export_are_half_the_diameter_of_each_via_padstack() {
    use konnect_ipc::{IpcEffectiveRoutingRules, IpcRoutingRules};
    let rule = |via_diameter_mm: f64| IpcRoutingRules {
        class_name: "Default".to_string(),
        constituents: vec!["Default".to_string()],
        track_width_mm: Some(0.25),
        clearance_mm: Some(0.2),
        via_diameter_mm: Some(via_diameter_mm),
        via_drill_mm: Some(0.3),
    };
    let rules: IpcEffectiveRoutingRules = [
        ("GND".to_string(), rule(0.6)),
        ("VCC".to_string(), rule(0.8)),
    ]
    .into_iter()
    .collect();
    let source = include_str!("../tests/fixtures/specctra_two_resistors.kicad_pcb");
    let dsn = export_dsn(Path::new("board.kicad_pcb"), source, &rules)
        .unwrap()
        .dsn;
    let radii = via_disc_radii(&dsn);
    // The padstacks the structure lists as vias.
    let listed: Vec<String> = dsn
        .lines()
        .find_map(|line| line.trim().strip_prefix("(via konnect_via_"))
        .map(|rest| {
            format!("konnect_via_{rest}")
                .trim_end_matches(')')
                .split_whitespace()
                .map(str::to_string)
                .collect()
        })
        .expect("the structure lists its via padstacks");
    assert_eq!(listed.len(), 2, "{listed:?}");
    for name in &listed {
        assert!(radii.contains_key(name), "{name} has no disc: {radii:?}");
    }
    let mut found: Vec<i64> = listed.iter().map(|name| radii[name]).collect();
    found.sort_unstable();
    assert_eq!(found, [300, 400], "half of 600 um and of 800 um");
}
