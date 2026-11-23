//! Tests of ZR0b's units, text cuts, clauses and loop: pure, no KiCad. The loop's tests
//! script a [`Checker`].

use super::*;
use serde_json::json;
use std::sync::Mutex;

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn idx(indices: &[usize]) -> BTreeSet<usize> {
    indices.iter().copied().collect()
}

// --------------------------------------------------------------- units and text

/// A session with a `(string_quote ")` parser block, a quoted net name, a net in two
/// blocks, repeats (one via three times, two padstacks; a wire and its reversed path), a
/// quarter arc, a wire that mixes a path and a qarc, and a net that is not asked for.
const SESSION: &str = r#"(session "board"
  (base_design "board")
  (placement
    (resolution um 10)
  )
  (was_is)
  (routes
    (resolution um 10)
    (parser
      (string_quote ")
      (space_in_quoted_tokens on)
      (host_cad "KiCad's Pcbnew")
    )
    (library_out
      (padstack "v1" (shape (circle F.Cu 6000 0 0)))
      (padstack v2 (shape (circle F.Cu 6000 0 0)))
    )
    (network_out
      (net GND
        (wire (path F.Cu 2000 0 0 10000 0))
        (via "v1" 10000 0)
        (wire (path B.Cu 2000 10000 0 10000 30000 40000 30000))
      )
      (net "Net-(R1-Pad1)"
        (wire (path F.Cu 2000 0 0 0 10000))
      )
      (net VCC
        (via "v1" 50000 50000)
        (wire (path F.Cu 2000 50000 50000 60000 50000))
      )
      (net GND
        (via v2 10000 0)
        (wire (qarc F.Cu 2000 0 0 10000 10000 0 10000))
        (wire (path B.Cu 2000 40000 30000 10000 30000 10000 0))
        (via "v1" 10000 0)
        (wire (path F.Cu 2000 70000 0 80000 0) (qarc F.Cu 2000 80000 0 90000 10000 80000 10000))
        (wire (path F.Cu 2000 0 5 1000.5 5))
      )
    )
  )
)
"#;

fn span_text(text: &str, span: (usize, usize)) -> &str {
    &text[span.0..span.1]
}

#[test]
fn units_are_the_wires_and_vias_of_the_given_nets_in_text_order() {
    let units = units(SESSION, &set(&["GND", "Net-(R1-Pad1)"])).unwrap();
    let kinds: Vec<(UnitKind, &str)> = units.iter().map(|u| (u.kind, u.net.as_str())).collect();
    assert_eq!(
        kinds,
        vec![
            (UnitKind::Wire, "GND"),
            (UnitKind::Via, "GND"),
            (UnitKind::Wire, "GND"),
            (UnitKind::Wire, "Net-(R1-Pad1)"),
            (UnitKind::Wire, "GND"),
            (UnitKind::Wire, "GND"),
            (UnitKind::Wire, "GND"),
        ],
        "VCC is not asked for; the second GND block's new units come after the quoted net"
    );
    assert_eq!(units[0].ends, Some([[0, 0], [10000, 0]]));
    assert_eq!(units[0].at, None);
    assert_eq!(units[1].at, Some([10000, 0]));
    assert_eq!(units[1].ends, None);
    assert_eq!(units[1].length_mm, 0.0);
    assert_eq!(units[2].ends, Some([[10000, 0], [40000, 30000]]));
    assert_eq!(units[3].ends, Some([[0, 0], [0, 10000]]));
    assert_eq!(
        span_text(SESSION, units[0].spans[0]),
        "(wire (path F.Cu 2000 0 0 10000 0))"
    );
}

#[test]
fn a_bare_and_a_quoted_net_name_are_both_compared_unquoted() {
    let quoted = units(SESSION, &set(&["Net-(R1-Pad1)"])).unwrap();
    assert_eq!(quoted.len(), 1);
    assert_eq!(quoted[0].net, "Net-(R1-Pad1)");
    let bare = units(SESSION, &set(&["VCC"])).unwrap();
    assert_eq!(bare.len(), 2);
    assert_eq!(bare[0].kind, UnitKind::Via);
    assert!(units(SESSION, &set(&["\"Net-(R1-Pad1)\""]))
        .unwrap()
        .is_empty());
}

#[test]
fn a_repeated_via_is_one_unit_whatever_the_padstack_and_a_reversed_path_is_a_repeat() {
    let units = units(SESSION, &set(&["GND"])).unwrap();
    let via = &units[1];
    assert_eq!(via.kind, UnitKind::Via);
    let texts: Vec<&str> = via.spans.iter().map(|s| span_text(SESSION, *s)).collect();
    assert_eq!(
        texts,
        vec![
            "(via \"v1\" 10000 0)",
            "(via v2 10000 0)",
            "(via \"v1\" 10000 0)"
        ],
        "the same via twice and once with another padstack, in text order"
    );
    let wire = &units[2];
    assert_eq!(wire.spans.len(), 2);
    assert_eq!(
        span_text(SESSION, wire.spans[1]),
        "(wire (path B.Cu 2000 40000 30000 10000 30000 10000 0))"
    );
    assert_eq!(
        wire.ends,
        Some([[10000, 0], [40000, 30000]]),
        "the first block's ends"
    );
    assert_eq!(units.iter().map(|u| u.spans.len()).sum::<usize>(), 9);
}

#[test]
fn a_wire_with_a_qarc_and_a_path_that_is_not_whole_have_no_ends() {
    let units = units(SESSION, &set(&["GND"])).unwrap();
    let arc = &units[3];
    assert!(span_text(SESSION, arc.spans[0]).starts_with("(wire (qarc"));
    assert_eq!(arc.ends, None);
    let mixed = &units[4];
    assert!(span_text(SESSION, mixed.spans[0]).contains("(path F.Cu 2000 70000 0 80000 0)"));
    assert_eq!(mixed.ends, None, "a path with a qarc beside it has no ends");
    let fraction = &units[5];
    assert_eq!(fraction.ends, None);
    assert!((fraction.length_mm - 0.10005).abs() < 1e-12);
}

#[test]
fn the_resolution_is_applied_to_the_length() {
    let units = units(SESSION, &set(&["GND"])).unwrap();
    // 10 units a micrometre.
    assert!((units[0].length_mm - 1.0).abs() < 1e-12);
    assert!((units[2].length_mm - 6.0).abs() < 1e-12);
    // A quarter circle of radius 1 mm.
    assert!((units[3].length_mm - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    // 1 mm of path and a quarter circle of radius 1 mm.
    assert!((units[4].length_mm - (1.0 + std::f64::consts::FRAC_PI_2)).abs() < 1e-12);

    let with = |resolution: &str| {
        let text = format!(
            "(session s (routes (resolution {resolution}) (network_out (net GND (wire (path F.Cu 1 0 0 3000 4000))))))"
        );
        super::units(&text, &set(&["GND"])).unwrap()[0].length_mm
    };
    assert!((with("mm 1000") - 5.0).abs() < 1e-12);
    assert!((with("um 1") - 5.0).abs() < 1e-12);
    assert!((with("mil 10") - 500.0 * 0.0254).abs() < 1e-9);
    assert!((with("inch 1000") - 5.0 * 25.4).abs() < 1e-9);
}

#[test]
fn a_session_without_routes_or_network_out_is_an_error_and_no_block_of_the_nets_is_empty() {
    let error = units(
        "(session s (routes (resolution um 10) (library_out)))",
        &set(&["GND"]),
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("network_out"), "{error:#}");
    let error = units("(session s (placement))", &set(&["GND"])).unwrap_err();
    assert!(format!("{error:#}").contains("routes"), "{error:#}");
    let error = units("(session s (routes (network_out)))", &set(&["GND"])).unwrap_err();
    assert!(format!("{error:#}").contains("resolution"), "{error:#}");
    let error = units("(session s (routes", &set(&["GND"])).unwrap_err();
    assert!(
        format!("{error:#}").starts_with("scan the session text"),
        "{error:#}"
    );
    assert!(units(SESSION, &set(&["AGND"])).unwrap().is_empty());
}

// --- without

const SHORT: &str = "(session s
  (routes
    (resolution um 10)
    (network_out
      (net GND
        (wire (path F.Cu 2000 0 0 10000 0))
        (via v 10000 0)
      )
      (net VCC (wire (path F.Cu 2000 0 0 0 5)) (via v 0 5))
    )
  )
)
";

#[test]
fn removed_empty_gives_the_text_byte_for_byte() {
    let units = units(SESSION, &set(&["GND"])).unwrap();
    assert_eq!(without(SESSION, &units, &BTreeSet::new()), SESSION);
}

#[test]
fn a_cut_takes_the_block_its_indent_and_its_line_when_nothing_else_is_on_it() {
    let units = units(SHORT, &set(&["GND", "VCC"])).unwrap();
    assert_eq!(units.len(), 4);
    let expected = "(session s
  (routes
    (resolution um 10)
    (network_out
      (net GND
        (via v 10000 0)
      )
      (net VCC (wire (path F.Cu 2000 0 0 0 5)) (via v 0 5))
    )
  )
)
";
    assert_eq!(without(SHORT, &units, &idx(&[0])), expected);
    let expected = "(session s
  (routes
    (resolution um 10)
    (network_out
      (net GND
        (wire (path F.Cu 2000 0 0 10000 0))
        (via v 10000 0)
      )
      (net VCC (wire (path F.Cu 2000 0 0 0 5)))
    )
  )
)
";
    assert_eq!(
        without(SHORT, &units, &idx(&[3])),
        expected,
        "a block with other text on its line takes only the spaces before it"
    );
}

#[test]
fn cutting_every_unit_of_a_net_cuts_the_net_block() {
    let units = units(SHORT, &set(&["GND", "VCC"])).unwrap();
    let expected = "(session s
  (routes
    (resolution um 10)
    (network_out
      (net VCC (wire (path F.Cu 2000 0 0 0 5)) (via v 0 5))
    )
  )
)
";
    assert_eq!(without(SHORT, &units, &idx(&[0, 1])), expected);
    let expected = "(session s
  (routes
    (resolution um 10)
    (network_out
      (net GND
        (wire (path F.Cu 2000 0 0 10000 0))
        (via v 10000 0)
      )
    )
  )
)
";
    assert_eq!(without(SHORT, &units, &idx(&[2, 3])), expected);
}

/// The fixture's session with a GND via at the end of its GND wire (twice, once with
/// the padstack name quoted), and a second GND wire from that via, written twice, the
/// second time reversed. Each block starts on its own line.
fn rich_session() -> String {
    let base = include_str!("../tests/fixtures/specctra_rounded_board.freerouting-2.3.0.ses");
    let extra = "      (net GND
        (via \"konnect_via_0001\" 1095000 -500000
        )
        (wire
          (path F.Cu 2000
            1095000 -500000
            1095000 -520000
          )
        )
        (via konnect_via_0001 1095000 -500000)
        (wire
          (path F.Cu 2000
            1095000 -520000
            1095000 -500000
          )
        )
";
    assert_eq!(base.matches("      (net GND\n").count(), 1);
    base.replacen("      (net GND\n", extra, 1)
}

/// `text` without each span's lines: from the start of the line it begins on to after
/// the newline that follows it. Right only for a text whose blocks each start their
/// own line and end it.
fn without_lines(text: &str, spans: &[(usize, usize)]) -> String {
    let mut spans = spans.to_vec();
    spans.sort();
    let mut out = String::new();
    let mut from = 0;
    for (start, end) in spans {
        let line = text[..start].rfind('\n').map_or(0, |p| p + 1);
        assert!(text[line..start].bytes().all(|c| c == b' '));
        assert_eq!(&text[end..end + 1], "\n");
        out.push_str(&text[from..line]);
        from = end + 1;
    }
    out.push_str(&text[from..]);
    out
}

#[test]
fn every_byte_outside_the_cut_blocks_and_their_line_whitespace_is_kept() {
    let ses = rich_session();
    let units = units(&ses, &set(&["GND", "VCC"])).unwrap();
    assert_eq!(units.len(), 6);
    for k in 0..units.len() {
        assert_eq!(
            without(&ses, &units, &idx(&[k])),
            without_lines(&ses, &units[k].spans),
            "unit {k}"
        );
    }
    assert_eq!(
        without(&ses, &units, &idx(&[0, 2, 4])),
        without_lines(
            &ses,
            &[
                units[0].spans.clone(),
                units[2].spans.clone(),
                units[4].spans.clone()
            ]
            .concat()
        )
    );
}

/// The import plan of `ses` against the judge fixture's board and manifest.
struct Importer {
    _dir: tempfile::TempDir,
    board: std::path::PathBuf,
    board_text: String,
    manifest: String,
}

impl Importer {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let exported = crate::route_judge::tests::exported(dir.path());
        let board = dir.path().join("specctra_rounded_board.kicad_pcb");
        let board_text = std::fs::read_to_string(&board).unwrap();
        let manifest =
            std::fs::read_to_string(format!("{}.konnect.json", exported.dsn.display())).unwrap();
        Importer {
            _dir: dir,
            board,
            board_text,
            manifest,
        }
    }

    fn plan(&self, ses: &str) -> crate::specctra_ses::SesImportPlan {
        crate::specctra_ses::parse_import_plan_with(
            &self.board,
            &self.board_text,
            &self.manifest,
            ses,
            crate::specctra_ses::SourceCheck::Bytes,
        )
        .unwrap()
    }
}

/// A unit's point as the importer writes it, mm, by the importer's own arithmetic
/// (resolution 10 a micrometre, then micrometres to mm, y flipped).
fn mm(point: [i64; 2]) -> (f64, f64) {
    (
        point[0] as f64 / 10.0 / 1000.0,
        -(point[1] as f64 / 10.0) / 1000.0,
    )
}

/// How many path segments the block at `span` writes: its numbers less the width,
/// paired, less one.
fn segments(text: &str, span: (usize, usize)) -> usize {
    let numbers = span_text(text, span)
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .filter(|word| word.parse::<i64>().is_ok())
        .count();
    (numbers - 1) / 2 - 1
}

#[test]
fn cutting_one_unit_takes_exactly_its_segments_or_its_via_out_of_the_import_plan() {
    let importer = Importer::new();
    let ses = rich_session();
    let plan = importer.plan(&ses);
    assert_eq!(plan.vias.len(), 1);
    assert_eq!(plan.duplicates_skipped, 2, "the via and the reversed wire");
    let units = units(&ses, &set(&["GND", "VCC"])).unwrap();
    for (k, unit) in units.iter().enumerate() {
        let cut = importer.plan(&without(&ses, &units, &idx(&[k])));
        assert!(
            cut.tracks.iter().all(|t| plan.tracks.contains(t)),
            "unit {k}"
        );
        let gone: Vec<_> = plan
            .tracks
            .iter()
            .filter(|t| !cut.tracks.contains(t))
            .collect();
        match unit.kind {
            UnitKind::Via => {
                assert_eq!(cut.tracks, plan.tracks, "unit {k}");
                assert!(cut.vias.is_empty(), "unit {k}");
                let (x, y) = mm(unit.at.unwrap());
                assert_eq!((plan.vias[0].x_mm, plan.vias[0].y_mm), (x, y));
            }
            UnitKind::Wire => {
                assert_eq!(cut.vias, plan.vias, "unit {k}");
                assert_eq!(gone.len(), segments(&ses, unit.spans[0]), "unit {k}");
                assert_eq!(cut.tracks.len() + gone.len(), plan.tracks.len(), "unit {k}");
                assert!(gone.iter().all(|t| t.net_name == unit.net), "unit {k}");
                let ends = unit.ends.unwrap();
                for end in [mm(ends[0]), mm(ends[1])] {
                    assert!(
                        gone.iter()
                            .any(|t| (t.x1_mm, t.y1_mm) == end || (t.x2_mm, t.y2_mm) == end),
                        "unit {k}: an end of the cut wire is an end of a cut segment"
                    );
                }
            }
        }
    }
}

#[test]
fn cutting_a_repeat_unit_removes_its_item_from_the_import_plan() {
    let importer = Importer::new();
    let ses = rich_session();
    let units = units(&ses, &set(&["GND"])).unwrap();
    let via = units.iter().position(|u| u.kind == UnitKind::Via).unwrap();
    assert_eq!(units[via].spans.len(), 2);
    let wire = units
        .iter()
        .position(|u| u.spans.len() == 2 && u.kind == UnitKind::Wire);
    let wire = wire.unwrap();
    let plan = importer.plan(&ses);
    let cut = importer.plan(&without(&ses, &units, &idx(&[via])));
    assert!(cut.vias.is_empty(), "both blocks of the via are cut");
    let cut = importer.plan(&without(&ses, &units, &idx(&[wire])));
    assert_eq!(cut.tracks.len(), plan.tracks.len() - 1);
    assert_eq!(cut.duplicates_skipped, 1, "only the via's repeat is left");
}

#[test]
fn cutting_every_unit_of_a_net_leaves_the_net_out_of_the_import_plan() {
    let importer = Importer::new();
    let ses = rich_session();
    let units = units(&ses, &set(&["GND"])).unwrap();
    let all: BTreeSet<usize> = (0..units.len()).collect();
    let cut_text = without(&ses, &units, &all);
    assert!(!cut_text.contains("(net GND"));
    let cut = importer.plan(&cut_text);
    assert!(cut.vias.is_empty());
    assert!(cut.tracks.iter().all(|t| t.net_name == "VCC"));
    assert!(!cut.tracks.is_empty());
}

// --- wires_ending_at

#[test]
fn wires_ending_at_take_ends_of_the_same_net_that_are_not_removed() {
    let text = "(session s (routes (resolution um 10) (network_out
      (net GND
        (via v 10000 0)
        (wire (path F.Cu 2000 0 0 10000 0))
        (wire (path F.Cu 2000 10000 0 10000 9000 20000 9000))
        (wire (path F.Cu 2000 5000 0 10000 0 15000 0))
        (wire (path F.Cu 2000 20000 0 30000 0))
      )
      (net VCC
        (wire (path F.Cu 2000 10000 0 10000 -5000))
      ))))";
    let units = units(text, &set(&["GND", "VCC"])).unwrap();
    assert_eq!(units.len(), 6);
    assert_eq!(
        wires_ending_at(&units, 0, &BTreeSet::new()),
        vec![1, 2],
        "a path through the via's point is not taken, nor another net's wire there"
    );
    assert_eq!(wires_ending_at(&units, 0, &idx(&[1])), vec![2]);
    assert!(
        wires_ending_at(&units, 1, &BTreeSet::new()).is_empty(),
        "not a via"
    );
}

// --- audit_items

fn audit() -> Value {
    json!({
        "net_roles": { "ground": { "count": 2, "nets": { "GND": "name", "AGND": "design_intent" }, "listed": 2 } },
        "metrics": {
            "3.2.unstitched_layer_changes": 2,
            "4.1.worst_loop_mm": 12.5,
            "4.1.median_loop_mm": 6.25,
            "4.1.mean_loop_area_mm2": 3.5,
        },
        "checks": [
            { "item": "3.2", "score": 0.5, "metrics": {} },
            { "item": "4.1", "score": 0.75, "metrics": {
                "measured": 3,
                "pins": [
                    { "pin": "U1.3", "net": "VCC", "loop": { "loop_mm": 12.5, "area_mm2": 4.0 } },
                    { "pin": "U1.7", "net": "VCC", "loop": null, "why": "no capacitor" },
                    { "pin": "U2.1", "net": "VCC", "loop": { "loop_mm": 6.25, "area_mm2": 3.0 } },
                ],
            } },
            { "item": "5.3", "score": null, "metrics": {} },
        ],
    })
}

#[test]
fn audit_items_read_each_value_from_its_place() {
    let items = audit_items(&audit());
    assert_eq!(items.ground, set(&["AGND", "GND"]));
    assert_eq!(items.unstitched_layer_changes, Some(2.0));
    assert_eq!(items.loops_measured, Some(3));
    assert_eq!(items.worst_loop_mm, Some(12.5));
    assert_eq!(items.median_loop_mm, Some(6.25));
    assert_eq!(items.mean_loop_area_mm2, Some(3.5));
    assert_eq!(
        items.pin_loops,
        BTreeMap::from([
            ("U1.3".to_string(), (12.5, 4.0)),
            ("U2.1".to_string(), (6.25, 3.0)),
        ])
    );
    assert_eq!(items.scored, set(&["3.2", "4.1"]), "5.3 has no score");
}

#[test]
fn audit_items_that_the_audit_did_not_give_are_none() {
    let items = audit_items(&json!({}));
    assert_eq!(
        items,
        AuditItems {
            ground: BTreeSet::new(),
            unstitched_layer_changes: None,
            loops_measured: None,
            worst_loop_mm: None,
            median_loop_mm: None,
            mean_loop_area_mm2: None,
            pin_loops: BTreeMap::new(),
            scored: BTreeSet::new(),
        }
    );
    let list = audit_items(&json!({ "net_roles": { "ground": { "nets": ["GND"] } } }));
    assert_eq!(list.ground, set(&["GND"]));
}

// --- drc_refusals and refusals

fn rules(counts: &[(&str, usize)]) -> BTreeMap<String, usize> {
    counts.iter().map(|(k, n)| (k.to_string(), *n)).collect()
}

fn rest() -> CheckedRest {
    CheckedRest {
        fills: vec![
            ZoneLayerFill {
                net: "GND".into(),
                layer: "B.Cu".into(),
                zone_uuid: Some("z1".into()),
                pieces_mm2: vec![100.0, 2.0],
                islands: vec![],
            },
            ZoneLayerFill {
                net: "GND".into(),
                layer: "F.Cu".into(),
                zone_uuid: Some("z2".into()),
                pieces_mm2: vec![50.0],
                islands: vec![],
            },
        ],
        measures: crate::route_judge::tests::measures(),
        gates: Gates {
            g1: vec![],
            g2: vec![],
            g3: vec![],
            g4: vec![],
        },
        audit: AuditItems {
            ground: set(&["GND"]),
            unstitched_layer_changes: Some(2.0),
            loops_measured: Some(3),
            worst_loop_mm: Some(10.0),
            median_loop_mm: Some(5.0),
            mean_loop_area_mm2: Some(4.0),
            pin_loops: BTreeMap::from([("U1.3".to_string(), (5.0, 2.0))]),
            scored: set(&["3.2", "4.1", "5.3"]),
        },
    }
}

fn checked(unconnected: usize, violations: &[(&str, usize)]) -> Checked {
    Checked {
        unconnected,
        violations: rules(violations),
        rest: Some(rest()),
    }
}

/// The base state with its rest changed by `change`.
fn with(change: impl FnOnce(&mut CheckedRest)) -> Checked {
    let mut state = checked(0, &[("x", 1)]);
    change(state.rest.as_mut().unwrap());
    state
}

fn refused(candidate: &Checked) -> Vec<String> {
    let base = checked(0, &[("x", 1)]);
    refusals(&base, &base, candidate, crate::pour_rank::RankMode::Routing)
}

#[test]
fn equal_is_kept() {
    let base = checked(0, &[("x", 1)]);
    assert!(drc_refusals(&base, 0, &rules(&[("x", 1)])).is_empty());
    assert!(refusals(&base, &base, &base, crate::pour_rank::RankMode::Routing).is_empty());
}

#[test]
fn drc_refusals_name_unconnected_then_every_higher_rule_in_name_order() {
    let base = checked(1, &[("b", 1), ("c", 2)]);
    assert_eq!(drc_refusals(&base, 2, &rules(&[])), vec!["unconnected"]);
    assert_eq!(
        drc_refusals(&base, 2, &rules(&[("a", 1), ("b", 2), ("c", 2)])),
        vec!["unconnected", "rule:a", "rule:b"],
        "every refusing rule is named; a rule the current state does not have counts as 0"
    );
    assert!(drc_refusals(&base, 0, &rules(&[("a", 0), ("b", 0)])).is_empty());
}

#[test]
fn a_drc_refusal_returns_only_its_names() {
    let mut candidate = with(|r| r.measures.acute = Some(5.0));
    candidate.unconnected = 1;
    assert_eq!(refused(&candidate), vec!["unconnected"]);
    let candidate = Checked {
        unconnected: 0,
        violations: rules(&[("x", 2), ("track_dangling", 1)]),
        rest: None,
    };
    assert_eq!(refused(&candidate), vec!["rule:track_dangling", "rule:x"]);
}

#[test]
fn a_candidate_that_passes_the_drc_without_the_rest_is_unchecked() {
    let candidate = Checked {
        unconnected: 0,
        violations: rules(&[("x", 1)]),
        rest: None,
    };
    assert_eq!(refused(&candidate), vec!["unchecked"]);
}

#[test]
fn a_higher_key_entry_refuses_by_its_position_and_seconds_never_decide() {
    assert_eq!(
        refused(&with(|r| r.measures.acute = Some(1.0))),
        vec!["key:6"]
    );
    assert_eq!(
        refused(&with(|r| r.measures.copper_quality = Some(80.0))),
        vec!["key:11"]
    );
    assert_eq!(
        refused(&with(|r| r.gates.g3.push("narrow".into()))),
        vec!["key:2"]
    );
    assert!(refused(&with(|r| r.measures.route_s = 1000.0)).is_empty());
    assert!(refused(&with(|r| r.measures.copper_quality = Some(95.0))).is_empty());
}

#[test]
fn an_entry_that_improves_does_not_excuse_one_that_worsens() {
    let candidate = with(|r| {
        r.measures.acute = Some(2.0);
        r.measures.copper_quality = Some(99.0);
        r.measures.right_angles = Some(3.0);
    });
    assert_eq!(refused(&candidate), vec!["key:6", "key:9"]);
}

#[test]
fn more_unstitched_layer_changes_refuse_and_none_on_one_side_refuses() {
    assert_eq!(
        refused(&with(|r| r.audit.unstitched_layer_changes = Some(3.0))),
        vec!["stitching"]
    );
    assert!(refused(&with(|r| r.audit.unstitched_layer_changes = Some(1.0))).is_empty());
    assert_eq!(
        refused(&with(|r| r.audit.unstitched_layer_changes = None)),
        vec!["stitching"]
    );
    let none = with(|r| r.audit.unstitched_layer_changes = None);
    assert!(
        refusals(&none, &none, &none, crate::pour_rank::RankMode::Routing).is_empty(),
        "None on both sides passes"
    );
    let some = with(|_| {});
    assert_eq!(
        refusals(&none, &none, &some, crate::pour_rank::RankMode::Routing),
        vec!["stitching"]
    );
}

#[test]
fn a_check_that_stops_scoring_refuses() {
    assert_eq!(
        refused(&with(|r| {
            r.audit.scored.remove("5.3");
        })),
        vec!["unscored"]
    );
    assert!(refused(&with(|r| {
        r.audit.scored.insert("8.1".into());
    }))
    .is_empty());
}

#[test]
fn a_fill_below_the_original_beyond_the_tolerance_refuses_and_a_lost_zone_layer_is_zero() {
    let inside = with(|r| r.fills[0].pieces_mm2 = vec![100.0, 1.995]);
    assert!(refused(&inside).is_empty());
    let outside = with(|r| r.fills[0].pieces_mm2 = vec![100.0, 1.985]);
    assert_eq!(refused(&outside), vec!["fill"]);
    let lost_piece = with(|r| r.fills[0].pieces_mm2 = vec![100.0]);
    assert_eq!(refused(&lost_piece), vec!["fill"]);
    let lost_layer = with(|r| {
        r.fills.remove(1);
    });
    assert_eq!(refused(&lost_layer), vec!["fill"]);
    let other_uuid = with(|r| r.fills[1].zone_uuid = Some("z3".into()));
    assert_eq!(
        refused(&other_uuid),
        vec!["fill"],
        "the zone layer is (net, layer, UUID)"
    );
    let more = with(|r| r.fills[1].pieces_mm2 = vec![60.0]);
    assert!(refused(&more).is_empty());
}

#[test]
fn each_decoupling_loop_measure_refuses_beyond_its_tolerance() {
    let refuses = |change: fn(&mut AuditItems)| refused(&with(|r| change(&mut r.audit)));
    assert!(refuses(|a| a.worst_loop_mm = Some(10.04)).is_empty());
    assert_eq!(refuses(|a| a.worst_loop_mm = Some(10.06)), vec!["loops"]);
    assert!(refuses(|a| a.median_loop_mm = Some(5.04)).is_empty());
    assert_eq!(refuses(|a| a.median_loop_mm = Some(5.06)), vec!["loops"]);
    assert!(refuses(|a| a.mean_loop_area_mm2 = Some(4.09)).is_empty());
    assert_eq!(
        refuses(|a| a.mean_loop_area_mm2 = Some(4.11)),
        vec!["loops"]
    );
    assert_eq!(refuses(|a| a.loops_measured = Some(2)), vec!["loops"]);
    assert!(refuses(|a| a.loops_measured = Some(4)).is_empty());
    assert!(refuses(|a| {
        a.pin_loops.insert("U1.3".into(), (5.04, 2.09));
    })
    .is_empty());
    assert_eq!(
        refuses(|a| {
            a.pin_loops.insert("U1.3".into(), (5.06, 2.0));
        }),
        vec!["loops"]
    );
    assert_eq!(
        refuses(|a| {
            a.pin_loops.insert("U1.3".into(), (5.0, 2.11));
        }),
        vec!["loops"]
    );
    assert!(
        refuses(|a| {
            a.pin_loops.clear();
            a.pin_loops.insert("U9.1".into(), (90.0, 90.0));
        })
        .is_empty(),
        "only a pin of both lists is compared"
    );
    assert_eq!(refuses(|a| a.worst_loop_mm = None), vec!["loops"]);
    assert_eq!(refuses(|a| a.median_loop_mm = None), vec!["loops"]);
    assert_eq!(refuses(|a| a.mean_loop_area_mm2 = None), vec!["loops"]);
    assert_eq!(refuses(|a| a.loops_measured = None), vec!["loops"]);
}

#[test]
fn the_tolerances_do_not_add_up_over_accepted_states() {
    let original = checked(0, &[("x", 1)]);
    let step = |area: f64| with(|r| r.audit.mean_loop_area_mm2 = Some(area));
    let first = step(4.06);
    let second = step(4.12);
    assert!(refusals(
        &original,
        &original,
        &first,
        crate::pour_rank::RankMode::Routing
    )
    .is_empty());
    assert_eq!(
        refusals(
            &original,
            &first,
            &second,
            crate::pour_rank::RankMode::Routing
        ),
        vec!["loops"],
        "0.06 above the state before it, 0.12 above the original"
    );
    let fill = |area: f64| with(|r| r.fills[1].pieces_mm2 = vec![area]);
    let first = fill(49.994);
    assert!(refusals(
        &original,
        &original,
        &first,
        crate::pour_rank::RankMode::Routing
    )
    .is_empty());
    assert_eq!(
        refusals(
            &original,
            &first,
            &fill(49.988),
            crate::pour_rank::RankMode::Routing
        ),
        vec!["fill"]
    );
}

#[test]
fn the_clauses_against_the_current_state_take_its_values() {
    let original = checked(2, &[("x", 3)]);
    let current = checked(1, &[("x", 2)]);
    assert_eq!(
        refusals(
            &original,
            &current,
            &checked(2, &[("x", 2)]),
            crate::pour_rank::RankMode::Routing
        ),
        vec!["unconnected"]
    );
    assert_eq!(
        refusals(
            &original,
            &current,
            &checked(1, &[("x", 3)]),
            crate::pour_rank::RankMode::Routing
        ),
        vec!["rule:x"]
    );
    let worse = with(|r| r.audit.unstitched_layer_changes = Some(1.0));
    let mut now = worse.clone();
    now.rest.as_mut().unwrap().audit.unstitched_layer_changes = Some(0.0);
    assert_eq!(
        refusals(&worse, &now, &worse, crate::pour_rank::RankMode::Routing),
        vec!["stitching"]
    );
}

// ------------------------------------------------------------------- the loop

/// What a scripted check answers.
#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
enum Answer {
    Checked(Checked),
    Importer,
    Error,
}

type Rule = Box<dyn Fn(&BTreeSet<usize>) -> Answer + Send + Sync>;

/// A checker that answers by the set of units the asked text lacks, and records every
/// asked set with the `current` it was given.
struct Script {
    units: Vec<Unit>,
    answer: Rule,
    asked: Mutex<Vec<(BTreeSet<usize>, Option<Checked>)>>,
}

/// What tells a unit apart in the loop sessions: via or not, net, ends, point.
type Signature = (bool, String, Option<[[i64; 2]; 2]>, Option<[i64; 2]>);

fn signature(unit: &Unit) -> Signature {
    (
        unit.kind == UnitKind::Via,
        unit.net.clone(),
        unit.ends,
        unit.at,
    )
}

impl Script {
    fn new(ses: &str, answer: impl Fn(&BTreeSet<usize>) -> Answer + Send + Sync + 'static) -> Self {
        Script {
            units: units(ses, &set(&["GND"])).unwrap(),
            answer: Box::new(answer),
            asked: Mutex::new(Vec::new()),
        }
    }

    fn removed_of(&self, text: &str) -> BTreeSet<usize> {
        let left: BTreeSet<_> = units(text, &set(&["GND"]))
            .unwrap()
            .iter()
            .map(signature)
            .collect();
        self.units
            .iter()
            .enumerate()
            .filter(|(_, unit)| !left.contains(&signature(unit)))
            .map(|(index, _)| index)
            .collect()
    }

    fn asked(&self) -> Vec<BTreeSet<usize>> {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .map(|(s, _)| s.clone())
            .collect()
    }
}

impl Checker for Script {
    fn check<'a>(&'a self, ses_text: &'a str, current: Option<&'a Checked>) -> CheckFuture<'a> {
        let removed = self.removed_of(ses_text);
        let answer = (self.answer)(&removed);
        self.asked.lock().unwrap().push((removed, current.cloned()));
        Box::pin(async move {
            match answer {
                Answer::Checked(checked) => Ok(CheckOutcome::Checked(checked)),
                Answer::Importer => Ok(CheckOutcome::ImporterRefused("refused".into())),
                Answer::Error => Err(anyhow!("the board changed on disk")),
            }
        })
    }
}

/// A session of GND vias, via `i` at (100000·i, 0) with `wires[i]` wires that end at
/// it. Units: via `i`, then its wires.
fn loop_session(wires: &[usize]) -> String {
    let mut net = String::new();
    for (i, &count) in wires.iter().enumerate() {
        let x = 100_000 * (i + 1);
        net.push_str(&format!("        (via v {x} 0)\n"));
        for j in 0..count {
            let y = 1000 * (j + 1);
            net.push_str(&format!(
                "        (wire (path F.Cu 2000 {x} 0 {x} {y} {} {y}))\n",
                x + 500
            ));
        }
    }
    format!(
        "(session s\n  (routes\n    (resolution um 10)\n    (network_out\n      (net GND\n{net}      )\n    )\n  )\n)\n"
    )
}

/// The original of the loop tests.
fn original() -> Checked {
    checked(0, &[("x", 5)])
}

/// A state the clauses accept: the original with other route seconds (which never
/// decide), so that an accepted state is told apart from the original.
fn better_state(n: usize) -> Checked {
    let mut state = original();
    state.rest.as_mut().unwrap().measures.route_s = 10.0 + n as f64;
    state
}

fn better(n: usize) -> Answer {
    Answer::Checked(better_state(n))
}

fn same() -> Answer {
    Answer::Checked(original())
}

fn rule(name: &str) -> Answer {
    Answer::Checked(Checked {
        unconnected: 0,
        violations: rules(&[("x", 5), (name, 1)]),
        rest: None,
    })
}

fn open() -> Answer {
    Answer::Checked(Checked {
        unconnected: 1,
        violations: rules(&[("x", 5)]),
        rest: None,
    })
}

fn fill_lost() -> Answer {
    let mut state = original();
    state.rest.as_mut().unwrap().fills[0].pieces_mm2 = vec![100.0];
    Answer::Checked(state)
}

fn vias_of(units: &[Unit]) -> Vec<usize> {
    (0..units.len())
        .filter(|&i| units[i].kind == UnitKind::Via)
        .collect()
}

async fn run(ses: &str, script: &Script) -> Pruned {
    prune_loop(
        ses,
        &script.units,
        &original(),
        script,
        crate::pour_rank::RankMode::Routing,
    )
    .await
}

/// The returned `removed` is a set the checker accepted whole, and `last` is that
/// check; nothing removed leaves the original. Every check was given the state
/// accepted before it.
fn assert_whole(script: &Script, pruned: &Pruned) {
    if pruned.removed.is_empty() {
        assert_eq!(pruned.last, original());
    } else {
        let asked = script.asked.lock().unwrap();
        let (_, current) = asked
            .iter()
            .rev()
            .find(|(s, _)| s == &pruned.removed)
            .unwrap_or_else(|| panic!("{:?} was never checked", pruned.removed));
        match (script.answer)(&pruned.removed) {
            Answer::Checked(checked) => {
                assert_eq!(pruned.last, checked);
                let current = current.as_ref().expect("checked against a state");
                assert!(
                    refusals(
                        &original(),
                        current,
                        &checked,
                        crate::pour_rank::RankMode::Routing
                    )
                    .is_empty(),
                    "{:?} was refused when it was checked",
                    pruned.removed
                );
            }
            _ => panic!("{:?} was not a measured check", pruned.removed),
        }
    }
    for (_, current) in script.asked.lock().unwrap().iter() {
        assert!(current.is_some(), "a candidate is checked against a state");
    }
}

#[tokio::test]
async fn every_via_redundant_with_its_wires_takes_one_check_a_via() {
    let ses = loop_session(&[1, 2, 1]);
    let script = Script::new(&ses, |_| same());
    let pruned = run(&ses, &script).await;
    let all: BTreeSet<usize> = (0..script.units.len()).collect();
    assert_eq!(pruned.removed, all);
    assert_eq!(pruned.checks, 3);
    assert_eq!(pruned.ended_by, "done");
    assert_eq!(pruned.error, None);
    assert!(pruned.kept_with.is_empty() && pruned.kept_alone.is_empty());
    assert_eq!(
        script.asked(),
        vec![
            idx(&[0, 1]),
            idx(&[0, 1, 2, 3, 4]),
            idx(&[0, 1, 2, 3, 4, 5, 6])
        ]
    );
    assert_whole(&script, &pruned);
}

#[tokio::test]
async fn a_via_refused_with_its_wires_is_tried_alone_and_one_refused_both_ways_stays() {
    // Via 0 (unit 0, wire 1): refused with its wire, removed alone. Via 1 (unit 2,
    // wire 3): a dangling track with its wire, open alone.
    let ses = loop_session(&[1, 1]);
    let script = Script::new(&ses, |s| {
        if s.contains(&1) || s.contains(&3) {
            rule("track_dangling")
        } else if s.contains(&2) {
            open()
        } else {
            better(1)
        }
    });
    let pruned = run(&ses, &script).await;
    assert_eq!(pruned.removed, idx(&[0]));
    assert_eq!(
        script.asked(),
        vec![idx(&[0, 1]), idx(&[0]), idx(&[0, 2, 3]), idx(&[0, 2])],
        "the open via is final: the second sweep asks nothing"
    );
    assert_eq!(pruned.checks, 4);
    assert_eq!(pruned.ended_by, "done");
    assert_eq!(
        pruned.kept_with,
        BTreeMap::from([("rule:track_dangling".to_string(), 1)]),
        "the with-wires clause of a via whose alone test failed `unconnected`"
    );
    assert_eq!(
        pruned.kept_alone,
        BTreeMap::from([("unconnected".to_string(), 1)])
    );
    assert_whole(&script, &pruned);
}

#[tokio::test]
async fn an_alone_test_refused_by_unconnected_or_fill_is_never_asked_again() {
    // Via 0 is open alone, via 1 loses fill alone, via 2 is refused by a rule alone,
    // via 3 goes: so a second sweep runs, and asks via 2 only.
    let ses = loop_session(&[0, 0, 0, 0]);
    let script = Script::new(&ses, |s| {
        if s.contains(&0) {
            open()
        } else if s.contains(&1) {
            fill_lost()
        } else if s.contains(&2) {
            rule("track_dangling")
        } else {
            same()
        }
    });
    let pruned = run(&ses, &script).await;
    assert_eq!(pruned.removed, idx(&[3]));
    assert_eq!(
        script.asked(),
        vec![idx(&[0]), idx(&[1]), idx(&[2]), idx(&[3]), idx(&[2, 3])]
    );
    assert_eq!(
        pruned.kept_alone,
        BTreeMap::from([
            ("fill".to_string(), 1),
            ("rule:track_dangling".to_string(), 1),
            ("unconnected".to_string(), 1),
        ])
    );
    assert_whole(&script, &pruned);
}

#[tokio::test]
async fn a_second_sweep_removes_a_via_the_first_refused_by_a_rule() {
    let ses = loop_session(&[1, 1]);
    // Via 0 (unit 0, wire 1) goes only once via 1 (unit 2, wire 3) has gone.
    let script = Script::new(&ses, |s| {
        if s.contains(&0) && !s.contains(&2) {
            rule("track_dangling")
        } else {
            same()
        }
    });
    let pruned = run(&ses, &script).await;
    assert_eq!(pruned.removed, idx(&[0, 1, 2, 3]));
    assert_eq!(
        script.asked(),
        vec![idx(&[0, 1]), idx(&[0]), idx(&[2, 3]), idx(&[0, 1, 2, 3])]
    );
    assert!(pruned.kept_with.is_empty() && pruned.kept_alone.is_empty());
    assert_whole(&script, &pruned);
}

#[tokio::test]
async fn no_sweep_follows_one_that_removed_nothing() {
    let ses = loop_session(&[1, 0]);
    let script = Script::new(&ses, |_| rule("track_dangling"));
    let pruned = run(&ses, &script).await;
    assert!(pruned.removed.is_empty());
    assert_eq!(script.asked(), vec![idx(&[0, 1]), idx(&[0]), idx(&[2])]);
    assert_eq!(
        pruned.kept_with,
        BTreeMap::from([("rule:track_dangling".to_string(), 1)])
    );
    assert_eq!(
        pruned.kept_alone,
        BTreeMap::from([("rule:track_dangling".to_string(), 2)])
    );
    assert_whole(&script, &pruned);
}

#[tokio::test]
async fn there_are_at_most_three_sweeps() {
    // Via i goes only when via i+1 has gone; via 3 goes at once. One via a sweep goes,
    // from the last, so the fourth sweep that would take via 0 never runs.
    let ses = loop_session(&[0, 0, 0, 0]);
    let script = Script::new(&ses, |s| {
        if s.iter().all(|&via| via == 3 || s.contains(&(via + 1))) {
            same()
        } else {
            rule("track_dangling")
        }
    });
    let pruned = run(&ses, &script).await;
    assert_eq!(pruned.removed, idx(&[1, 2, 3]));
    assert_eq!(pruned.checks, 4 + 3 + 2);
    assert_eq!(pruned.ended_by, "done");
    assert_eq!(
        pruned.kept_alone,
        BTreeMap::from([("rule:track_dangling".to_string(), 1)])
    );
    assert_whole(&script, &pruned);
}

#[tokio::test]
async fn an_importer_refusal_is_a_refused_attempt_and_never_final() {
    let ses = loop_session(&[1, 0]);
    let script = Script::new(&ses, |s| {
        if s.contains(&0) {
            Answer::Importer
        } else {
            better(1)
        }
    });
    let pruned = run(&ses, &script).await;
    assert_eq!(pruned.removed, idx(&[2]));
    assert_eq!(
        script.asked(),
        vec![
            idx(&[0, 1]),
            idx(&[0]),
            idx(&[2]),
            idx(&[0, 1, 2]),
            idx(&[0, 2])
        ],
        "the refused via is asked again in the second sweep"
    );
    assert_eq!(pruned.ended_by, "done");
    assert_eq!(
        pruned.kept_with,
        BTreeMap::from([("importer".to_string(), 1)])
    );
    assert_eq!(
        pruned.kept_alone,
        BTreeMap::from([("importer".to_string(), 1)])
    );
    assert_whole(&script, &pruned);
}

#[tokio::test]
async fn an_error_ends_the_loop_with_the_state_accepted_so_far() {
    let ses = loop_session(&[1, 1, 1]);
    let script = Script::new(&ses, |s| {
        if s.contains(&2) {
            Answer::Error
        } else {
            better(1)
        }
    });
    let pruned = run(&ses, &script).await;
    assert_eq!(pruned.removed, idx(&[0, 1]));
    assert_eq!(pruned.last, better_state(1));
    assert_eq!(pruned.checks, 2);
    assert_eq!(pruned.ended_by, "check_error");
    assert!(pruned
        .error
        .as_deref()
        .unwrap()
        .contains("the board changed on disk"));
    assert_eq!(script.asked(), vec![idx(&[0, 1]), idx(&[0, 1, 2, 3])]);
    assert_whole(&script, &pruned);
}

#[tokio::test]
async fn the_check_cap_ends_the_loop_with_the_state_accepted_so_far() {
    let ses = loop_session(&[0; MAX_CHECKS + 1]);
    let script = Script::new(&ses, |s| {
        if s == &BTreeSet::from([0]) {
            better(1)
        } else {
            rule("track_dangling")
        }
    });
    let pruned = run(&ses, &script).await;
    assert_eq!(pruned.checks, MAX_CHECKS);
    assert_eq!(pruned.ended_by, "check_cap");
    assert_eq!(pruned.removed, idx(&[0]));
    assert_eq!(pruned.last, better_state(1));
    assert_eq!(
        pruned.kept_alone,
        BTreeMap::from([("rule:track_dangling".to_string(), MAX_CHECKS - 1)]),
        "the via the cap stopped before has no clause"
    );
    assert_whole(&script, &pruned);
}

#[tokio::test]
async fn each_check_is_given_the_state_accepted_before_it() {
    let ses = loop_session(&[0, 0, 0]);
    let script = Script::new(&ses, |s| match s.len() {
        1 if s.contains(&0) => better(1),
        2 if s.contains(&1) => better(2),
        _ => rule("track_dangling"),
    });
    let pruned = run(&ses, &script).await;
    assert_eq!(pruned.removed, idx(&[0, 1]));
    let currents: Vec<Option<Checked>> = script
        .asked
        .lock()
        .unwrap()
        .iter()
        .map(|(_, current)| current.clone())
        .collect();
    let states = [original(), better_state(1), better_state(2)];
    assert_eq!(
        currents[..3],
        [
            Some(states[0].clone()),
            Some(states[1].clone()),
            Some(states[2].clone())
        ]
    );
    assert_whole(&script, &pruned);
}

#[tokio::test]
async fn two_runs_ask_the_same_sets_in_the_same_order() {
    let ses = loop_session(&[2, 0, 1, 1, 0]);
    let answer = |s: &BTreeSet<usize>| {
        if s.len().is_multiple_of(3) {
            rule("track_dangling")
        } else if s.contains(&4) && !s.contains(&6) {
            open()
        } else {
            same()
        }
    };
    let first = Script::new(&ses, answer);
    let second = Script::new(&ses, answer);
    let a = run(&ses, &first).await;
    let b = run(&ses, &second).await;
    assert_eq!(first.asked(), second.asked());
    assert_eq!(a, b);
    assert!(!a.removed.is_empty());
    assert_whole(&first, &a);
    assert_eq!(vias_of(&first.units).len(), 5);
}

// --- replace_winner (builder B)

/// A judged run whose audit gave `quality` as its copper quality.
fn judged_at(quality: f64) -> Judged {
    let mut measures = crate::route_judge::tests::measures();
    measures.copper_quality = Some(quality);
    Judged {
        measures,
        gates: Gates {
            g1: vec![],
            g2: vec![],
            g3: vec![],
            g4: vec![],
        },
        board: std::path::PathBuf::from("/nowhere/board.kicad_pcb"),
        rounds: vec![],
        tune: None,
    }
}

fn run_record(index: usize, judged: Judged) -> crate::route_search::RunRecord {
    crate::route_search::RunRecord {
        index,
        shape: crate::route_search::Shape::AsExported,
        seed: index as u64,
        launched_s: 0.0,
        routed_s: Some(1.0),
        judged_s: Some(2.0),
        route_s: Some(1.0),
        judge_s: Some(1.0),
        outcome: "judged",
        reason: None,
        routed: Some(Routed {
            index,
            shape: crate::route_search::Shape::AsExported,
            seed: index as u64,
            dsn: PathBuf::from("/nowhere/variant.dsn"),
            held: crate::fastroute::Held {
                ses: PathBuf::from(format!("/nowhere/run-{index}/full.ses")),
                unrouted: 0,
                kept: "full",
                passes: vec![],
                seconds: 1.0,
                untuned: None,
                tune: Default::default(),
                group_search: None,
                planes: None,
            },
            planes: vec![],
            folder: PathBuf::from(format!("/nowhere/run-{index}")),
        }),
        judged: Some(judged),
    }
}

/// Two judged runs: run 0 won at copper quality 90, run 1 has 85.
fn two_runs() -> SearchOutcome {
    let runs = vec![
        run_record(0, judged_at(90.0)),
        run_record(1, judged_at(85.0)),
    ];
    let refs: Vec<&Judged> = runs.iter().filter_map(|r| r.judged.as_ref()).collect();
    let keys = crate::route_judge::rank(&refs)
        .into_iter()
        .map(Some)
        .collect();
    let pooled: Vec<(&Measures, &Gates, bool)> = refs
        .iter()
        .map(|judged| (&judged.measures, &judged.gates, true))
        .collect();
    let pour_keys = crate::pour_rank::pour_keys(&pooled)
        .into_iter()
        .map(Some)
        .collect();
    SearchOutcome {
        runs,
        winner: Some(0),
        keys,
        pour_keys,
        rank: crate::pour_rank::RankMode::Routing,
        elapsed: std::time::Duration::from_secs(1),
        budget_overrun: None,
        stopped_launching: None,
        baseline: Ok(Baseline {
            violations: BTreeMap::new(),
            unconnected: 0,
            pour_fills: None,
            pour_fills_error: None,
        }),
        pour_prune: None,
    }
}

/// The winner is replaced only while it still ranks first over every judged run; else
/// nothing changes. Control: the check removed, a pruned winner that ranks below run 1
/// is delivered.
#[test]
fn a_pruned_winner_that_would_not_rank_first_replaces_nothing() {
    let mut outcome = two_runs();
    let keys = outcome.keys.clone();
    let refused = replace_winner(
        &mut outcome,
        0,
        PathBuf::from("/nowhere/run-0/pruned.ses"),
        judged_at(80.0),
        crate::pour_rank::RankMode::Routing,
    );
    assert_eq!(
        refused,
        Err("the pruned winner would no longer rank first among the judged runs".to_string())
    );
    assert_eq!(outcome.keys, keys);
    let winner = outcome.runs[0].routed.as_ref().unwrap();
    assert_eq!(winner.held.ses, PathBuf::from("/nowhere/run-0/full.ses"));
    assert_eq!(outcome.runs[0].judged, Some(judged_at(90.0)));

    // Still first: replaced, and the keys are taken again.
    let kept = replace_winner(
        &mut outcome,
        0,
        PathBuf::from("/nowhere/run-0/pruned.ses"),
        judged_at(95.0),
        crate::pour_rank::RankMode::Routing,
    )
    .unwrap();
    assert_eq!(kept["session"], "/nowhere/run-0/full.ses");
    assert_eq!(kept["judged_board"], Value::Null, "the board is not there");
    let winner = outcome.runs[0].routed.as_ref().unwrap();
    assert_eq!(winner.held.ses, PathBuf::from("/nowhere/run-0/pruned.ses"));
    assert_eq!(outcome.runs[0].judged, Some(judged_at(95.0)));
    let refs: Vec<&Judged> = outcome
        .runs
        .iter()
        .filter_map(|r| r.judged.as_ref())
        .collect();
    let again: Vec<Option<Key>> = crate::route_judge::rank(&refs)
        .into_iter()
        .map(Some)
        .collect();
    assert_eq!(outcome.keys, again);
    assert_ne!(outcome.keys, keys);
}

/// Row ZR, ZR1: a kept prune keeps the original run's pad comparison (the pruned judge
/// makes none), as it keeps its Tune choice; the pruned board's own pour measures stay.
/// Control: the carry left out, the counts read `None`.
#[test]
fn a_pruned_winner_keeps_the_runs_pad_comparison() {
    let pours = |pieces, false_connected, false_open| crate::zone_fill::PourMeasures {
        open_before_stitching: 0,
        open_after_stitching: 0,
        pieces,
        largest_piece_share: Some(1.0),
        unfilled: 0,
        starved_thermals: 0,
        false_connected,
        false_open,
        damage: None,
    };
    let mut outcome = two_runs();
    {
        let original = outcome.runs[0].judged.as_mut().unwrap();
        original.measures.pours = Some(pours(3, Some(1), Some(2)));
    }
    let mut pruned = judged_at(95.0);
    pruned.measures.pours = Some(pours(2, None, None));
    replace_winner(
        &mut outcome,
        0,
        PathBuf::from("/nowhere/run-0/pruned.ses"),
        pruned,
        crate::pour_rank::RankMode::Routing,
    )
    .unwrap();
    let measures = &outcome.runs[0].judged.as_ref().unwrap().measures;
    let taken = measures.pours.as_ref().unwrap();
    assert_eq!(
        (taken.false_connected, taken.false_open),
        (Some(1), Some(2))
    );
    assert_eq!(taken.pieces, 2, "the pruned board's own fill");
    assert_eq!(measures.pours_error, None);

    // A comparison that failed on the run keeps its reason.
    let mut outcome = two_runs();
    {
        let original = outcome.runs[0].judged.as_mut().unwrap();
        original.measures.pours = Some(pours(3, None, None));
        original.measures.pours_error = Some("KiCad's open pads could not be read".into());
    }
    let mut pruned = judged_at(95.0);
    pruned.measures.pours = Some(pours(2, None, None));
    replace_winner(
        &mut outcome,
        0,
        PathBuf::from("/p.ses"),
        pruned,
        crate::pour_rank::RankMode::Routing,
    )
    .unwrap();
    let measures = &outcome.runs[0].judged.as_ref().unwrap().measures;
    assert_eq!(
        measures.pours_error.as_deref(),
        Some("KiCad's open pads could not be read")
    );
}

// --- review S3 (builder B)

/// (e) and (f) on a board where 4.1 measured nothing and 3.2 gave no count: nothing to
/// compare, nothing refused, so such a board still prunes. Controls: `(None, _) => true`
/// in either arm of `loops_worse`.
#[test]
fn a_board_with_no_decoupling_loops_and_no_stitching_count_refuses_nothing() {
    let none = with(|r| {
        r.audit.loops_measured = None;
        r.audit.worst_loop_mm = None;
        r.audit.median_loop_mm = None;
        r.audit.mean_loop_area_mm2 = None;
        r.audit.pin_loops.clear();
        r.audit.unstitched_layer_changes = None;
    });
    assert!(refusals(&none, &none, &none, crate::pour_rank::RankMode::Routing).is_empty());
    assert!(!loops_worse(
        &none.rest.as_ref().unwrap().audit,
        &none.rest.as_ref().unwrap().audit
    ));
}

/// S7 in the loop itself: every accepted state is 0.06 mm² of mean loop area above the
/// one before. The second via passes against the state before it and is refused against
/// the original, so it stays with `loops`. Control: the loop gives `refusals` the current
/// state in place of the original.
#[tokio::test]
async fn the_loop_compares_the_tolerance_clauses_with_the_original() {
    let ses = loop_session(&[0, 0]);
    let script = Script::new(&ses, |removed| {
        let mut state = original();
        state.rest.as_mut().unwrap().audit.mean_loop_area_mm2 =
            Some(4.0 + 0.06 * removed.len() as f64);
        Answer::Checked(state)
    });
    let pruned = run(&ses, &script).await;
    let vias = vias_of(&script.units);
    assert_eq!(pruned.removed, idx(&[vias[0]]));
    assert_eq!(
        pruned.kept_alone,
        BTreeMap::from([("loops".to_string(), 1)])
    );
    assert_whole(&script, &pruned);
}

// --- cargo mutants (builder B): the edges and the seconds

/// (d) and (f) at their edge: a value exactly its tolerance worse than the original's is
/// kept; only beyond it refuses. Controls (the mutants it was written for): `>` to `>=` in
/// `loops_worse`'s aggregate and pin comparisons, `<` to `<=` in the fill clause.
#[test]
fn a_value_exactly_at_its_tolerance_is_kept() {
    type Change<'a> = &'a dyn Fn(&mut CheckedRest);
    let base = checked(0, &[("x", 1)]);
    let kept = |change: Change| {
        let candidate = with(|r| change(r));
        refusals(
            &base,
            &base,
            &candidate,
            crate::pour_rank::RankMode::Routing,
        )
    };
    // The base's 4.1: worst 10, median 5, mean area 4, pin U1.3 (5 mm, 2 mm²); its fill
    // on F.Cu is one 50 mm² piece.
    let at: [(&str, Change); 6] = [
        ("worst", &|r| {
            r.audit.worst_loop_mm = Some(10.0 + LOOP_TOLERANCE_MM)
        }),
        ("median", &|r| {
            r.audit.median_loop_mm = Some(5.0 + LOOP_TOLERANCE_MM)
        }),
        ("mean area", &|r| {
            r.audit.mean_loop_area_mm2 = Some(4.0 + LOOP_AREA_TOLERANCE_MM2)
        }),
        ("pin length", &|r| {
            r.audit
                .pin_loops
                .insert("U1.3".into(), (5.0 + LOOP_TOLERANCE_MM, 2.0));
        }),
        ("pin area", &|r| {
            r.audit
                .pin_loops
                .insert("U1.3".into(), (5.0, 2.0 + LOOP_AREA_TOLERANCE_MM2));
        }),
        ("fill", &|r| {
            r.fills[1].pieces_mm2 = vec![50.0 - FILL_TOLERANCE_MM2]
        }),
    ];
    for (what, change) in at {
        assert!(kept(change).is_empty(), "{what} at its tolerance");
    }
    // A little beyond refuses, each by its clause.
    assert_eq!(
        kept(&|r| {
            r.audit
                .pin_loops
                .insert("U1.3".into(), (5.0 + 2.0 * LOOP_TOLERANCE_MM, 2.0));
        }),
        vec!["loops"]
    );
    assert_eq!(
        kept(&|r| r.fills[1].pieces_mm2 = vec![50.0 - 2.0 * FILL_TOLERANCE_MM2]),
        vec!["fill"]
    );
}

/// P9: every entry of the key but the last (the route seconds) decides the full judge of
/// the pruned session. Both full judges read the same run's route seconds today, so the
/// last entry is equal in every real call; this pins the rule itself. Control (the mutant
/// it was written for): the range `0..KEY_LEN - 1` taken as `0..KEY_LEN`.
#[test]
fn the_route_seconds_never_refuse_the_pruned_session() {
    let original = judged_at(90.0);
    let mut slower = judged_at(90.0);
    slower.measures.route_s = original.measures.route_s + 100.0;
    assert_eq!(
        full_judge_refusal(&original, &slower, crate::pour_rank::RankMode::Routing),
        None
    );
    // Control of the test itself: a lower copper quality refuses at its position.
    let worse = judged_at(80.0);
    assert!(
        full_judge_refusal(&original, &worse, crate::pour_rank::RankMode::Routing)
            .is_some_and(|reason| reason.contains("at position 11")),
        "{:?}",
        full_judge_refusal(&original, &worse, crate::pour_rank::RankMode::Routing)
    );
}
