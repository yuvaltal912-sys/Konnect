use super::*;
use crate::tools::cli::{DrcViolation, ReportItem};

fn square(x: f64, y: f64, side: f64) -> String {
    format!(
        "(pts (xy {x} {y}) (xy {} {y}) (xy {} {}) (xy {x} {}))",
        x + side,
        x + side,
        y + side,
        y + side
    )
}

/// A zone of `net` on `layer` with one `filled_polygon` per given square.
fn zone(net: &str, layer: &str, uuid: &str, squares: &[(f64, f64, f64)]) -> String {
    let fills: String = squares
        .iter()
        .map(|&(x, y, side)| {
            format!(
                "\n    (filled_polygon (layer \"{layer}\") {})",
                square(x, y, side)
            )
        })
        .collect();
    format!(
        "  (zone (net \"{net}\") (layer \"{layer}\") (uuid \"{uuid}\") (hatch edge 0.5)\n    (connect_pads (clearance 0.2)) (min_thickness 0.25)\n    (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.5))\n    (polygon {}){fills})\n",
        square(0.0, 0.0, 100.0)
    )
}

fn board(zones: &[String]) -> String {
    format!(
        "(kicad_pcb (version 20250114) (generator \"pcbnew\")\n  (net 0 \"\")\n  (net 1 \"GND\")\n  (net 2 \"VCC\")\n{})",
        zones.concat()
    )
}

fn nets(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn violation(rule: &str, items: &[&str]) -> DrcViolation {
    DrcViolation {
        severity: "error".to_string(),
        description: "a description".to_string(),
        rule: rule.to_string(),
        pos: None,
        items: items
            .iter()
            .map(|description| ReportItem {
                description: description.to_string(),
                ..Default::default()
            })
            .collect(),
    }
}

fn report(violations: Vec<DrcViolation>, unconnected: Vec<DrcViolation>) -> DrcReport {
    DrcReport {
        violations,
        unconnected_items: Some(unconnected),
        schematic_parity: None,
        ownership_diagnostic: None,
        schematic_parity_diagnostic: None,
    }
}

fn open_on(net: &str) -> DrcViolation {
    violation(
        "unconnected_items",
        &[
            &format!("Pad 1 [{net}] of U1"),
            &format!("Pad 2 [{net}] of U2"),
        ],
    )
}

#[test]
fn every_filled_polygon_of_a_pour_net_is_one_piece() {
    let text = board(&[zone(
        "GND",
        "B.Cu",
        "z1",
        // the small piece first in the file: the pieces come back largest first
        &[(20.0, 0.0, 2.0), (0.0, 0.0, 10.0)],
    )]);
    let fills = pour_fills(&text, &nets(&["GND"])).unwrap();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].pieces_mm2, vec![100.0, 4.0], "largest first");
    // control: the same board with one fill polygon is one piece
    let one = board(&[zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0)])]);
    assert_eq!(
        pour_fills(&one, &nets(&["GND"])).unwrap()[0]
            .pieces_mm2
            .len(),
        1
    );
}

#[test]
fn only_zones_of_pour_nets_are_read() {
    let text = board(&[
        zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0)]),
        zone("VCC", "F.Cu", "z2", &[(0.0, 0.0, 10.0), (20.0, 0.0, 2.0)]),
    ]);
    let fills = pour_fills(&text, &nets(&["GND"])).unwrap();
    assert_eq!(
        fills.iter().map(|f| f.net.as_str()).collect::<Vec<_>>(),
        ["GND"]
    );
    // control: naming the other net reads its zone too
    assert_eq!(pour_fills(&text, &nets(&["GND", "VCC"])).unwrap().len(), 2);
}

#[test]
fn a_keepout_or_teardrop_is_no_pour() {
    let real = zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0)]);
    let keepout = zone("GND", "B.Cu", "k1", &[(0.0, 0.0, 10.0)]).replace(
        "(hatch edge 0.5)",
        "(hatch edge 0.5) (keepout (tracks not_allowed))",
    );
    let teardrop = zone("GND", "B.Cu", "t1", &[(0.0, 0.0, 10.0)]).replace(
        "(hatch edge 0.5)",
        "(hatch edge 0.5) (attr (teardrop (type padvia)))",
    );
    let text = board(&[real.clone(), keepout, teardrop]);
    let fills = pour_fills(&text, &nets(&["GND"])).unwrap();
    assert_eq!(
        fills
            .iter()
            .map(|f| f.zone_uuid.as_deref())
            .collect::<Vec<_>>(),
        [Some("z1")]
    );
    // control: the same zones without the keepout and teardrop marks are all pours
    let plain = board(&[
        real,
        zone("GND", "B.Cu", "k1", &[(0.0, 0.0, 10.0)]),
        zone("GND", "B.Cu", "t1", &[(0.0, 0.0, 10.0)]),
    ]);
    assert_eq!(pour_fills(&plain, &nets(&["GND"])).unwrap().len(), 3);
}

#[test]
fn a_zone_net_is_read_in_each_board_file_form() {
    let kicad10 = board(&[zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0)])]);
    let by_id = kicad10.replacen("(zone (net \"GND\")", "(zone (net 1)", 1);
    let by_name = kicad10.replacen("(zone (net \"GND\")", "(zone (net 1) (net_name \"GND\")", 1);
    for text in [&kicad10, &by_id, &by_name] {
        assert_eq!(
            pour_fills(text, &nets(&["GND"])).unwrap().len(),
            1,
            "{text}"
        );
    }
    // control: an id the net table names otherwise is that other net
    let other = kicad10.replacen("(zone (net \"GND\")", "(zone (net 2)", 1);
    assert!(pour_fills(&other, &nets(&["GND"])).unwrap().is_empty());
}

#[test]
fn a_pour_layer_with_no_saved_fill_is_unfilled_not_whole() {
    let text = board(&[zone("GND", "B.Cu", "z1", &[])]);
    let m = pour_measures(
        &report(vec![], vec![]),
        &text,
        &report(vec![], vec![]),
        &nets(&["GND"]),
    )
    .unwrap();
    assert_eq!((m.pieces, m.unfilled, m.largest_piece_share), (0, 1, None));
    // control: one fill polygon makes it filled and whole
    let filled = board(&[zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0)])]);
    let m = pour_measures(
        &report(vec![], vec![]),
        &filled,
        &report(vec![], vec![]),
        &nets(&["GND"]),
    )
    .unwrap();
    assert_eq!(
        (m.pieces, m.unfilled, m.largest_piece_share),
        (1, 0, Some(1.0))
    );
}

#[test]
fn the_largest_share_is_the_worst_zone_layers() {
    let text = board(&[
        zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0)]),
        // 64 of 64 + 16 + 16 + 4 = 100: the worst share
        zone(
            "GND",
            "F.Cu",
            "z2",
            &[
                (0.0, 0.0, 8.0),
                (20.0, 0.0, 4.0),
                (30.0, 0.0, 4.0),
                (40.0, 0.0, 2.0),
            ],
        ),
    ]);
    let m = pour_measures(
        &report(vec![], vec![]),
        &text,
        &report(vec![], vec![]),
        &nets(&["GND"]),
    )
    .unwrap();
    assert_eq!(m.pieces, 5);
    assert!((m.largest_piece_share.unwrap() - 0.64).abs() < 1e-12);
}

#[test]
fn open_before_stitching_is_the_first_refill_and_on_pour_nets_only() {
    let text = board(&[zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0)])]);
    let first = report(vec![], vec![open_on("GND"), open_on("GND"), open_on("SIG")]);
    let last = report(vec![], vec![open_on("GND")]);
    let m = pour_measures(&first, &text, &last, &nets(&["GND"])).unwrap();
    assert_eq!((m.open_before_stitching, m.open_after_stitching), (2, 1));
    // control: with the reports swapped the counts swap
    let m = pour_measures(&last, &text, &first, &nets(&["GND"])).unwrap();
    assert_eq!((m.open_before_stitching, m.open_after_stitching), (1, 2));
}

#[test]
fn starved_thermals_are_counted_on_the_last_board() {
    let text = board(&[zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0)])]);
    let starved = report(
        vec![
            violation(STARVED_THERMAL, &["Pad 1 [GND] of U1"]),
            violation("clearance", &["x"]),
        ],
        vec![],
    );
    let m = pour_measures(&report(vec![], vec![]), &text, &starved, &nets(&["GND"])).unwrap();
    assert_eq!(m.starved_thermals, 1);
    // control: the same report on the first refill only counts nothing
    let m = pour_measures(&starved, &text, &report(vec![], vec![]), &nets(&["GND"])).unwrap();
    assert_eq!(m.starved_thermals, 0);
}

#[test]
fn the_engine_side_is_not_measured_before_zr1() {
    let text = board(&[zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0)])]);
    let m = pour_measures(
        &report(vec![], vec![]),
        &text,
        &report(vec![], vec![]),
        &nets(&["GND"]),
    )
    .unwrap();
    assert_eq!((m.false_connected, m.false_open), (None, None));
}

#[test]
fn a_board_that_does_not_parse_is_an_error() {
    assert!(pour_fills("(kicad_pcb (zone", &nets(&["GND"])).is_err());
}

#[test]
fn the_ring_area_takes_either_winding_and_needs_three_points() {
    let ccw = [(0.0, 0.0), (2.0, 0.0), (2.0, 3.0), (0.0, 3.0)];
    let cw: Vec<(f64, f64)> = ccw.iter().rev().copied().collect();
    assert_eq!(ring_area_mm2(&ccw), 6.0);
    assert_eq!(ring_area_mm2(&cw), 6.0);
    assert_eq!(ring_area_mm2(&ccw[..2]), 0.0);
}

#[test]
fn every_layer_a_zone_names_is_counted_filled_or_not() {
    // a two-layer zone, filled on one layer only
    let text = board(&[zone("GND", "F.Cu", "z1", &[(0.0, 0.0, 10.0)])
        .replace("(layer \"F.Cu\") (uuid", "(layers \"F.Cu\" \"B.Cu\") (uuid")]);
    let m = pour_measures(
        &report(vec![], vec![]),
        &text,
        &report(vec![], vec![]),
        &nets(&["GND"]),
    )
    .unwrap();
    assert_eq!((m.pieces, m.unfilled), (1, 1));
    // control: the same zone on one layer has nothing unfilled
    let one = board(&[zone("GND", "F.Cu", "z1", &[(0.0, 0.0, 10.0)])]);
    let m = pour_measures(
        &report(vec![], vec![]),
        &one,
        &report(vec![], vec![]),
        &nets(&["GND"]),
    )
    .unwrap();
    assert_eq!((m.pieces, m.unfilled), (1, 0));
}

#[test]
fn a_fractured_piece_is_its_outline_less_its_hole() {
    // KiCad saves a hole joined to its outline by a zero-width cut: one ring
    let ring = "(pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10) (xy 0 5) (xy 3 5) (xy 3 7) (xy 7 7) (xy 7 3) (xy 3 3) (xy 3 5) (xy 0 5))";
    let text = board(&[
        zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0)]).replace(&square(0.0, 0.0, 10.0), ring)
    ]);
    let fills = pour_fills(&text, &nets(&["GND"])).unwrap();
    assert_eq!(
        fills[0].pieces_mm2,
        vec![84.0],
        "100 less the 16 of the hole"
    );
    // control: the outline alone
    let plain = board(&[zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0)])]);
    assert_eq!(
        pour_fills(&plain, &nets(&["GND"])).unwrap()[0].pieces_mm2,
        vec![100.0]
    );
}

#[test]
fn the_net_name_wins_over_the_id() {
    // an older file: an id the table names otherwise, and the zone's own `net_name`
    let text = board(&[zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0)])]).replacen(
        "(zone (net \"GND\")",
        "(zone (net 2) (net_name \"GND\")",
        1,
    );
    assert_eq!(pour_fills(&text, &nets(&["GND"])).unwrap().len(), 1);
    // control: without the `net_name` the id names the other net
    let id_only = text.replacen(" (net_name \"GND\")", "", 1);
    assert!(pour_fills(&id_only, &nets(&["GND"])).unwrap().is_empty());
}

#[test]
fn a_quoted_name_is_a_name_and_net_0_is_no_net() {
    // KiCad 10 writes `(net "2")` for a net named 2: never looked up as an id
    let named_2 = board(&[zone("2", "B.Cu", "z1", &[(0.0, 0.0, 10.0)])]);
    assert_eq!(pour_fills(&named_2, &nets(&["2"])).unwrap().len(), 1);
    assert!(pour_fills(&named_2, &nets(&["VCC"])).unwrap().is_empty());
    // an unquoted 0 is no net, even when a net is named "0"
    let zero = board(&[zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0)])]).replacen(
        "(zone (net \"GND\")",
        "(zone (net 0)",
        1,
    );
    assert!(pour_fills(&zero, &nets(&["0", ""])).unwrap().is_empty());
}

#[test]
fn a_fill_polygon_with_no_area_is_no_piece() {
    let degenerate = "(pts (xy 0 0) (xy 5 0) (xy 10 0))";
    let text = board(&[
        zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0), (20.0, 0.0, 2.0)])
            .replace(&square(20.0, 0.0, 2.0), degenerate),
    ]);
    assert_eq!(
        pour_fills(&text, &nets(&["GND"])).unwrap()[0].pieces_mm2,
        vec![100.0]
    );
}

#[test]
fn a_failed_pour_measure_is_recorded_not_raised() {
    let mut m = crate::route_judge::Measures {
        unconnected: 0,
        violations: Default::default(),
        routed_share: None,
        audit_complete: None,
        acute: None,
        right_angles: None,
        copper_quality: None,
        nets_undersized: None,
        narrow_paths: None,
        signal_mm: 0.0,
        power_mm: 0.0,
        pour_track_mm: 0.0,
        vias_off_pour: 0,
        below_floor: 0,
        width_rows: vec![],
        longest_below_mm: Default::default(),
        stitching_vias: 0,
        route_s: 0.0,
        lengths: None,
        pours: None,
        pours_error: None,
    };
    record_pours(&mut m, Err(anyhow::anyhow!("the board went away")));
    assert_eq!(m.pours, None);
    assert_eq!(m.pours_error.as_deref(), Some("the board went away"));
    // control: a measure that was taken replaces the error
    let text = board(&[zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0)])]);
    let taken = pour_measures(
        &report(vec![], vec![]),
        &text,
        &report(vec![], vec![]),
        &nets(&["GND"]),
    );
    record_pours(&mut m, taken);
    assert_eq!(m.pours.as_ref().map(|p| p.pieces), Some(1));
    assert_eq!(m.pours_error, None);
}

#[tokio::test]
async fn the_record_reads_the_board_at_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("judged.kicad_pcb");
    let empty = || report(vec![], vec![]);
    let missing = pour_record(&path, empty(), empty(), &nets(&["GND"])).await;
    assert!(format!("{:#}", missing.unwrap_err()).contains("read the judged board"));
    // control: once the board is there it is measured
    std::fs::write(
        &path,
        board(&[zone("GND", "B.Cu", "z1", &[(0.0, 0.0, 10.0)])]),
    )
    .unwrap();
    let m = pour_record(&path, empty(), empty(), &nets(&["GND"]))
        .await
        .unwrap();
    assert_eq!(m.pieces, 1);
}
