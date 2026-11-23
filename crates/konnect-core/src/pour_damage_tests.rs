//! Tests of `pour_damage`: one test and one negative control for each guard in
//! docs/POUR_RANK_DESIGN.md, "Part 1: tests, each guard with a negative control".
//! The two guards that need the judge's fake kicad-cli ("before" is the refilled
//! baseline; a fill that cannot be read fails nothing) are in `route_judge_tests.rs`,
//! beside that harness; `baseline_json`'s is in `tools/route_fastroute_tests.rs`.

use super::*;
use crate::zone_fill::{pour_fills, pour_measures, PourMeasures, ZoneLayerFill};
use std::collections::{BTreeMap, BTreeSet};

/// One layer fill row, as `layer_fills` gives it.
fn row(net: &str, layer: &str, pieces: usize, largest: f64, area: f64) -> LayerFill {
    LayerFill {
        net: net.to_string(),
        layer: layer.to_string(),
        pieces,
        largest_mm2: largest,
        area_mm2: area,
        islands: 0,
    }
}

/// A row of one whole piece.
fn whole(net: &str, layer: &str, area: f64) -> LayerFill {
    row(net, layer, 1, area, area)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-12
}

fn zone_layer(
    net: &str,
    layer: &str,
    uuid: &str,
    pieces: &[f64],
    islands: &[bool],
) -> ZoneLayerFill {
    ZoneLayerFill {
        net: net.to_string(),
        layer: layer.to_string(),
        zone_uuid: Some(uuid.to_string()),
        pieces_mm2: pieces.to_vec(),
        islands: islands.to_vec(),
    }
}

// ------------------------------------------------------------------ the numbers

#[test]
fn an_undamaged_board_scores_nothing() {
    // A zone that is two pieces by design (4000 and 1000 on B.Cu), and a second net.
    let before = vec![
        row("GND", "B.Cu", 2, 4000.0, 5000.0),
        whole("GND", "F.Cu", 300.0),
        whole("VCC", "F.Cu", 30.0),
    ];
    let damage = pour_damage(&before, &before);
    assert_eq!(
        (damage.lost_fills, damage.plane_loss, damage.area_loss),
        (0, 0.0, 0.0)
    );
    assert!(
        damage.nets.iter().all(|net| net.kept_share == 1.0),
        "{damage:?}"
    );
}

#[test]
fn the_best_single_layer_counts_by_area() {
    let before = vec![whole("GND", "F.Cu", 4000.0), whole("GND", "B.Cu", 4000.0)];
    // The front cut to 2000, the back whole: the back is still a whole plane.
    let after = vec![
        row("GND", "F.Cu", 2, 2000.0, 3900.0),
        whole("GND", "B.Cu", 4000.0),
    ];
    assert_eq!(pour_damage(&before, &after).plane_loss, 0.0);
    // Both cut, to 2400 and 3200: the best is 3200 of 4000.
    let after = vec![
        row("GND", "F.Cu", 2, 2400.0, 3900.0),
        row("GND", "B.Cu", 2, 3200.0, 3900.0),
    ];
    let damage = pour_damage(&before, &after);
    assert!(close(damage.plane_loss, 0.2), "{damage:?}");
    assert_eq!(damage.nets[0].kept_layer.as_deref(), Some("B.Cu"));
}

#[test]
fn a_small_whole_layer_does_not_hide_a_destroyed_plane() {
    let before = vec![whole("GND", "B.Cu", 4000.0), whole("GND", "F.Cu", 20.0)];
    let after = vec![
        row("GND", "B.Cu", 3, 1000.0, 3000.0),
        whole("GND", "F.Cu", 20.0),
    ];
    let damage = pour_damage(&before, &after);
    assert!(close(damage.plane_loss, 0.75), "{damage:?}");
    assert_eq!(damage.nets[0].kept_layer.as_deref(), Some("B.Cu"));
}

#[test]
fn the_nets_are_weighted_by_area() {
    let before = vec![whole("GND", "B.Cu", 4000.0), whole("VCC", "F.Cu", 30.0)];
    let after = vec![whole("GND", "B.Cu", 3600.0), whole("VCC", "F.Cu", 24.0)];
    let damage = pour_damage(&before, &after);
    assert!(close(damage.plane_loss, 406.0 / 4030.0), "{damage:?}");
    // Equal weights would give (0.1 + 0.2) / 2.
    assert!(!close(damage.plane_loss, 0.15));
}

#[test]
fn a_piece_that_grew_is_not_a_gain() {
    // One layer: the largest piece and the area larger after than before.
    let before = vec![whole("GND", "B.Cu", 1000.0)];
    let after = vec![whole("GND", "B.Cu", 1200.0)];
    let damage = pour_damage(&before, &after);
    assert_eq!(
        (damage.plane_loss, damage.area_loss),
        (0.0, 0.0),
        "{damage:?}"
    );
    assert_eq!(damage.nets[0].kept_mm2, 1000.0);
    // Two layers: the back cut to 2000, the front grew from 1000 to 3000. The front
    // keeps at most its own 1000, so the back's 2000 is what is kept.
    let before = vec![whole("GND", "B.Cu", 4000.0), whole("GND", "F.Cu", 1000.0)];
    let after = vec![
        row("GND", "B.Cu", 2, 2000.0, 3900.0),
        whole("GND", "F.Cu", 3000.0),
    ];
    let damage = pour_damage(&before, &after);
    assert!(close(damage.plane_loss, 0.5), "{damage:?}");
    // The area: the front counts 1000 of its 1000, the back 3900 of 4000.
    assert!(close(damage.area_loss, 1.0 - 4900.0 / 5000.0), "{damage:?}");
}

#[test]
fn a_fill_that_vanished_is_a_fault() {
    // Fill before, none after, on the net's only layer.
    let before = vec![whole("GND", "B.Cu", 4000.0)];
    let after = vec![row("GND", "B.Cu", 0, 0.0, 0.0)];
    let damage = pour_damage(&before, &after);
    assert_eq!(
        (damage.lost_fills, damage.plane_loss, damage.area_loss),
        (1, 1.0, 1.0)
    );
    assert_eq!(damage.nets[0].kept_layer, None);
    // A second layer that is whole (and the plane): one lost fill, no plane loss.
    let before = vec![whole("GND", "B.Cu", 4000.0), whole("GND", "F.Cu", 1000.0)];
    let after = vec![
        whole("GND", "B.Cu", 4000.0),
        row("GND", "F.Cu", 0, 0.0, 0.0),
    ];
    let damage = pour_damage(&before, &after);
    assert_eq!((damage.lost_fills, damage.plane_loss), (1, 0.0));
    // Islands only after is still no fill.
    let mut islands_only = row("GND", "F.Cu", 0, 0.0, 0.0);
    islands_only.islands = 2;
    let after = vec![whole("GND", "B.Cu", 4000.0), islands_only];
    assert_eq!(pour_damage(&before, &after).lost_fills, 1);
}

#[test]
fn a_cut_off_piece_kept_alive_is_not_worse_than_one_that_is_gone() {
    let before = vec![whole("GND", "B.Cu", 4000.0)];
    // The same cut twice: the cut-off 900 mm² kept (a via feeds it), and gone.
    let kept = pour_damage(&before, &[row("GND", "B.Cu", 2, 3000.0, 3900.0)]);
    let gone = pour_damage(&before, &[row("GND", "B.Cu", 1, 3000.0, 3000.0)]);
    assert!(kept.area_loss < gone.area_loss, "{kept:?} {gone:?}");
    assert!(close(kept.area_loss, 0.025) && close(gone.area_loss, 0.25));
    assert_eq!(kept.plane_loss, gone.plane_loss);
}

#[test]
fn a_layer_fill_with_no_row_after_is_zero_after() {
    let before = vec![whole("GND", "B.Cu", 4000.0), whole("GND", "F.Cu", 1000.0)];
    // F.Cu has no row after; VCC/In1.Cu is only in "after" and is not read.
    let after = vec![whole("GND", "B.Cu", 4000.0), whole("VCC", "In1.Cu", 500.0)];
    let damage = pour_damage(&before, &after);
    assert_eq!(damage.lost_fills, 1);
    assert!(close(damage.area_loss, 1000.0 / 5000.0), "{damage:?}");
    assert_eq!(damage.layers[1], row("GND", "F.Cu", 0, 0.0, 0.0));
    assert_eq!(damage.layers.len(), 2);
    assert_eq!(
        damage
            .nets
            .iter()
            .map(|n| n.net.as_str())
            .collect::<Vec<_>>(),
        ["GND"]
    );
    // A row only in "after" changes nothing.
    assert_eq!(
        pour_damage(&before, &after[..1]),
        damage,
        "the after-only row is not read"
    );
}

#[test]
fn a_net_with_no_fill_before_adds_nothing() {
    // Only a net with no fill before: nothing in the sums.
    let before = vec![row("VCC", "F.Cu", 0, 0.0, 0.0)];
    let damage = pour_damage(&before, &before);
    assert_eq!(
        (damage.plane_loss, damage.area_loss),
        (0.0, 0.0),
        "{damage:?}"
    );
    let vcc = &damage.nets[0];
    assert_eq!(
        (
            vcc.plane_layer.as_deref(),
            vcc.plane_mm2,
            vcc.kept_layer.as_deref(),
            vcc.kept_mm2
        ),
        (None, 0.0, None, 0.0)
    );
    assert_eq!(vcc.kept_share, 1.0);
    // Beside a plane cut to 3000 of 4000: the loss is that plane's alone.
    let before = vec![
        whole("GND", "B.Cu", 4000.0),
        row("VCC", "F.Cu", 0, 0.0, 0.0),
    ];
    let after = vec![
        whole("GND", "B.Cu", 3000.0),
        row("VCC", "F.Cu", 0, 0.0, 0.0),
    ];
    let damage = pour_damage(&before, &after);
    assert!(close(damage.plane_loss, 0.25), "{damage:?}");
    assert_eq!(damage.lost_fills, 0, "no piece before is no lost fill");
}

#[test]
fn the_rows_are_in_the_record() {
    // "Before" in an order that is not the names' order: ties go to the first layer
    // of "before", F.Cu.
    let before = vec![
        whole("GND", "F.Cu", 4000.0),
        whole("GND", "B.Cu", 4000.0),
        whole("AGND", "B.Cu", 50.0),
    ];
    let after = vec![
        whole("AGND", "B.Cu", 50.0),
        row("GND", "B.Cu", 2, 3000.0, 3900.0),
        row("GND", "F.Cu", 2, 3000.0, 3800.0),
    ];
    let damage = pour_damage(&before, &after);
    // The "after" rows, in "before"'s order.
    assert_eq!(
        damage.layers,
        vec![after[2].clone(), after[1].clone(), after[0].clone()]
    );
    // One row per net, by name.
    assert_eq!(
        damage.nets,
        vec![
            NetPlane {
                net: "AGND".into(),
                plane_layer: Some("B.Cu".into()),
                plane_mm2: 50.0,
                kept_layer: Some("B.Cu".into()),
                kept_mm2: 50.0,
                kept_share: 1.0,
            },
            NetPlane {
                net: "GND".into(),
                plane_layer: Some("F.Cu".into()),
                plane_mm2: 4000.0,
                kept_layer: Some("F.Cu".into()),
                kept_mm2: 3000.0,
                kept_share: 0.75,
            },
        ]
    );
}

#[test]
fn every_share_is_from_zero_to_one() {
    let before = vec![
        row("GND", "B.Cu", 3, 4000.0, 4500.0),
        whole("GND", "F.Cu", 700.0),
        whole("VCC", "F.Cu", 30.0),
    ];
    for after in [
        vec![],
        before.clone(),
        vec![row("GND", "B.Cu", 9, 9000.0, 9000.0)],
        vec![row("GND", "B.Cu", 1, 1.0, 1.0), whole("VCC", "F.Cu", 1e6)],
    ] {
        let damage = pour_damage(&before, &after);
        for share in [damage.plane_loss, damage.area_loss] {
            assert!(
                share.is_finite() && (0.0..=1.0).contains(&share),
                "{damage:?}"
            );
        }
    }
}

// ------------------------------------------------------------- the layer fills

#[test]
fn an_island_kicad_kept_is_not_fill() {
    let fills = vec![zone_layer(
        "GND",
        "B.Cu",
        "z1",
        &[500.0, 300.0, 20.0],
        &[true, false, false],
    )];
    assert_eq!(
        layer_fills(&fills),
        vec![LayerFill {
            islands: 1,
            ..row("GND", "B.Cu", 2, 300.0, 320.0)
        }]
    );
}

#[test]
fn a_piece_saved_as_an_island_is_flagged_in_the_board_read() {
    let piece = |x: f64, side: f64, mark: &str| {
        format!(
            "(filled_polygon (layer \"B.Cu\"){mark} (pts (xy {x} 0) (xy {} 0) (xy {} {side}) (xy {x} {side})))",
            x + side,
            x + side
        )
    };
    let text = format!(
        "(kicad_pcb (net 0 \"\") (net 1 \"GND\") (zone (net \"GND\") (layer \"B.Cu\") (uuid \"z1\") {} {} {} {}))",
        piece(0.0, 10.0, ""),
        piece(20.0, 20.0, " (island yes)"),
        piece(50.0, 3.0, " (island)"),
        piece(60.0, 2.0, " (island no)"),
    );
    let gnd: BTreeSet<String> = ["GND".to_string()].into();
    let fills = pour_fills(&text, &gnd).unwrap();
    // Largest first, each flag with its piece.
    assert_eq!(fills[0].pieces_mm2, vec![400.0, 100.0, 9.0, 4.0]);
    assert_eq!(fills[0].islands, vec![true, false, true, false]);
    assert_eq!(
        layer_fills(&fills),
        vec![LayerFill {
            islands: 2,
            ..row("GND", "B.Cu", 2, 100.0, 104.0)
        }]
    );
    // KiCad also reads `true` and `false` (`parseMaybeAbsentBool`).
    let text = format!(
        "(kicad_pcb (net 0 \"\") (net 1 \"GND\") (zone (net \"GND\") (layer \"B.Cu\") (uuid \"z1\") {} {}))",
        piece(0.0, 10.0, " (island true)"),
        piece(20.0, 3.0, " (island false)"),
    );
    let fills = pour_fills(&text, &gnd).unwrap();
    assert_eq!(fills[0].pieces_mm2, vec![100.0, 9.0]);
    assert_eq!(fills[0].islands, vec![true, false]);
}

#[test]
fn several_zones_of_a_net_on_a_layer_are_one_fill() {
    let fills = vec![
        zone_layer("GND", "B.Cu", "z1", &[100.0, 4.0], &[false, false]),
        zone_layer("GND", "B.Cu", "z2", &[300.0], &[false]),
        zone_layer("GND", "F.Cu", "z3", &[], &[]),
        zone_layer("AGND", "F.Cu", "z4", &[7.0], &[]),
    ];
    assert_eq!(
        layer_fills(&fills),
        vec![
            whole("AGND", "F.Cu", 7.0),
            row("GND", "B.Cu", 3, 300.0, 404.0),
            // A zone layer with no piece still gives its row, with zeros.
            row("GND", "F.Cu", 0, 0.0, 0.0),
        ]
    );
}

// --------------------------------------------------------------- the record

#[test]
fn an_older_record_still_reads() {
    let older = serde_json::json!({
        "open_before_stitching": 1, "open_after_stitching": 0, "pieces": 2,
        "largest_piece_share": 0.9, "unfilled": 0, "starved_thermals": 0,
        "false_connected": null, "false_open": null
    });
    let read: PourMeasures = serde_json::from_value(older.clone()).unwrap();
    assert_eq!(read.damage, None);
    // And a record with no damage writes none.
    assert_eq!(serde_json::to_value(&read).unwrap(), older);
    // Control in the test: a damage that is there is read back.
    let mut with = read.clone();
    with.damage = Some(pour_damage(&[whole("GND", "B.Cu", 10.0)], &[]));
    let back: PourMeasures = serde_json::from_value(serde_json::to_value(&with).unwrap()).unwrap();
    assert_eq!(back, with);
}

#[test]
fn the_record_changes_no_rank() {
    use crate::route_judge::{rank_measures, Gates};
    let gates = Gates {
        g1: vec![],
        g2: vec![],
        g3: vec![],
        g4: vec![],
    };
    let pours = |damage| PourMeasures {
        open_before_stitching: 0,
        open_after_stitching: 0,
        pieces: 1,
        largest_piece_share: Some(1.0),
        unfilled: 0,
        starved_thermals: 0,
        false_connected: None,
        false_open: None,
        damage,
    };
    let mut whole_run = crate::route_judge::tests::measures();
    whole_run.pours = Some(pours(Some(pour_damage(
        &[whole("GND", "B.Cu", 4000.0)],
        &[whole("GND", "B.Cu", 4000.0)],
    ))));
    let mut cut_run = whole_run.clone();
    cut_run.pours = Some(pours(Some(pour_damage(
        &[whole("GND", "B.Cu", 4000.0)],
        &[row("GND", "B.Cu", 0, 0.0, 0.0)],
    ))));
    let mut no_record = whole_run.clone();
    no_record.pours = Some(pours(None));
    let keys = rank_measures(&[
        (&whole_run, &gates, true),
        (&cut_run, &gates, true),
        (&no_record, &gates, true),
    ]);
    assert_eq!(keys[0], keys[1]);
    assert_eq!(keys[0], keys[2]);
}

// ------------------------------------------------- the fills' present readers

/// A board with one GND zone on B.Cu: a 100 mm² piece and a 4 mm² piece, the second
/// saved as an island when `island`.
fn island_board(island: bool, with_small: bool) -> String {
    let mark = if island { " (island yes)" } else { "" };
    let small = if with_small {
        format!(
            "(filled_polygon (layer \"B.Cu\"){mark} (pts (xy 20 0) (xy 22 0) (xy 22 2) (xy 20 2)))"
        )
    } else {
        String::new()
    };
    format!(
        "(kicad_pcb (net 0 \"\") (net 1 \"GND\") (zone (net \"GND\") (layer \"B.Cu\") (uuid \"z1\") (filled_polygon (layer \"B.Cu\") (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10))) {small}))"
    )
}

#[test]
fn the_zr0_measures_read_an_island_as_a_piece() {
    use crate::tools::cli::DrcReport;
    let empty = || DrcReport {
        violations: vec![],
        unconnected_items: Some(vec![]),
        schematic_parity: None,
        ownership_diagnostic: None,
        schematic_parity_diagnostic: None,
    };
    let gnd: BTreeSet<String> = ["GND".to_string()].into();
    let measures = |text: &str| pour_measures(&empty(), text, &empty(), &gnd).unwrap();
    let flagged = measures(&island_board(true, true));
    assert_eq!(flagged.pieces, 2, "ZR0's pieces count the island");
    assert!(close(flagged.largest_piece_share.unwrap(), 100.0 / 104.0));
    assert_eq!(flagged.damage, None);
    // The same PourMeasures as the board with no island mark.
    assert_eq!(flagged, measures(&island_board(false, true)));
    // And `pieces_mm2` still holds the island.
    assert_eq!(
        pour_fills(&island_board(true, true), &gnd).unwrap()[0].pieces_mm2,
        vec![100.0, 4.0]
    );
}

#[test]
fn the_prunes_fill_clause_reads_an_island_as_fill() {
    use crate::pour_prune::{refusals, AuditItems, Checked, CheckedRest};
    use crate::route_judge::Gates;
    let gnd: BTreeSet<String> = ["GND".to_string()].into();
    let checked = |text: &str| Checked {
        unconnected: 0,
        violations: BTreeMap::new(),
        rest: Some(CheckedRest {
            fills: pour_fills(text, &gnd).unwrap(),
            measures: crate::route_judge::tests::measures(),
            gates: Gates {
                g1: vec![],
                g2: vec![],
                g3: vec![],
                g4: vec![],
            },
            audit: AuditItems {
                ground: gnd.clone(),
                unstitched_layer_changes: None,
                loops_measured: None,
                worst_loop_mm: None,
                median_loop_mm: None,
                mean_loop_area_mm2: None,
                pin_loops: BTreeMap::new(),
                scored: BTreeSet::new(),
            },
        }),
    };
    // A candidate that loses the 4 mm² island: refused for its fill, exactly as when
    // the piece is not marked as an island.
    let lost = checked(&island_board(true, false));
    for original in [
        checked(&island_board(true, true)),
        checked(&island_board(false, true)),
    ] {
        assert_eq!(refusals(&original, &original, &lost), vec!["fill"]);
        assert!(refusals(&original, &original, &original).is_empty());
    }
}
