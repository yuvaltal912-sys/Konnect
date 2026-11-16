//! Tests of the scratch project copy and the file-only routing writer.

use super::*;
use crate::specctra_ses::{SesArc, SesImportPlan, SesTrack, SesVia};
use konnect_sexp::board::{tracks, vias};
use serde_json::json;
use std::collections::BTreeSet;

/// A KiCad 10 board: nets are named in place; four copper layers.
const KICAD_10: &str = r#"(kicad_pcb
    (version 20260206)
    (generator "pcbnew")
    (layers
        (0 "F.Cu" signal)
        (4 "In1.Cu" signal)
        (6 "In2.Cu" signal)
        (2 "B.Cu" signal)
        (25 "Edge.Cuts" user)
    )
    (footprint "R"
        (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND"))
        (pad "2" smd rect (at 3 0) (size 1 1) (layers "F.Cu") (net "VCC"))
    )
)
"#;

/// A legacy board: a net table with codes, `(net N)` on items; two copper layers.
const LEGACY: &str = r#"(kicad_pcb
    (version 20241229)
    (generator "pcbnew")
    (layers
        (0 "F.Cu" signal)
        (31 "B.Cu" signal)
        (44 "Edge.Cuts" user)
    )
    (net 0 "")
    (net 1 "GND")
    (net 2 "VCC")
    (footprint "R"
        (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "GND"))
    )
)
"#;

fn plan(tracks: Vec<SesTrack>, arcs: Vec<SesArc>, vias: Vec<SesVia>) -> SesImportPlan {
    SesImportPlan {
        routing_context: None,
        board_path: String::new(),
        source_sha256: String::new(),
        session_id: "session".to_string(),
        locked_track_count: 0,
        locked_via_count: 0,
        tracks,
        arcs,
        vias,
        duplicates_skipped: 0,
    }
}

fn track(net: &str, layer: &str, width: f64, from: (f64, f64), to: (f64, f64)) -> SesTrack {
    SesTrack {
        net_name: net.to_string(),
        layer: layer.to_string(),
        width_mm: width,
        x1_mm: from.0,
        y1_mm: from.1,
        x2_mm: to.0,
        y2_mm: to.1,
    }
}

fn via(net: &str, at: (f64, f64), size: f64, drill: f64) -> SesVia {
    SesVia {
        net_name: net.to_string(),
        padstack_name: "Via".to_string(),
        x_mm: at.0,
        y_mm: at.1,
        drill_mm: drill,
        size_mm: size,
    }
}

fn arc(net: &str, layer: &str) -> SesArc {
    SesArc {
        net_name: net.to_string(),
        layer: layer.to_string(),
        width_mm: 0.2,
        start_x_mm: 10.0,
        start_y_mm: 10.0,
        mid_x_mm: 11.0,
        mid_y_mm: 11.0,
        end_x_mm: 12.0,
        end_y_mm: 10.0,
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn everything() -> SesImportPlan {
    plan(
        vec![track("GND", "F.Cu", 0.25, (1.0, 2.0), (3.5, 2.0))],
        vec![arc("VCC", "B.Cu")],
        vec![via("GND", (5.0, 6.0), 0.6, 0.3)],
    )
}

// ---------------------------------------------------------------- write_routing

#[test]
fn kicad_10_board_names_the_net_in_place_and_reads_back() {
    let out = write_routing(KICAD_10, &everything()).unwrap();
    let tree = parse_sexp(&out).expect("the output parses");
    assert_eq!(tree.head(), Some("kicad_pcb"));

    let read = tracks(&tree);
    assert_eq!(read.skipped, 0);
    assert_eq!(read.items.len(), 1);
    let t = &read.items[0];
    assert_eq!(t.net.as_deref(), Some("GND"));
    assert_eq!(t.layer, "F.Cu");
    assert!(close(t.width, 0.25));
    assert!(close(t.x1, 1.0) && close(t.y1, 2.0) && close(t.x2, 3.5) && close(t.y2, 2.0));

    let read = vias(&tree);
    assert_eq!(read.skipped, 0);
    assert_eq!(read.items.len(), 1);
    let v = &read.items[0];
    assert_eq!(v.net.as_deref(), Some("GND"));
    assert!(close(v.x, 5.0) && close(v.y, 6.0));
    assert!(close(v.size, 0.6) && close(v.drill, 0.3));
    // Four copper layers: the via spans the first and the last of the table.
    assert_eq!(v.layers, vec!["F.Cu".to_string(), "B.Cu".to_string()]);

    // The name is written in the KiCad 10 form, never a code.
    assert!(out.contains("(net \"GND\")"));
    assert!(out.contains("(net \"VCC\")"));
    assert!(!out.contains("(net 1)"));
}

#[test]
fn legacy_board_gets_the_net_code_and_reads_back_by_name() {
    let plan = plan(
        vec![track("GND", "F.Cu", 0.25, (1.0, 2.0), (3.0, 2.0))],
        vec![],
        vec![via("VCC", (5.0, 6.0), 0.6, 0.3)],
    );
    let out = write_routing(LEGACY, &plan).unwrap();
    assert!(out.contains("(net 1)"), "{out}");
    assert!(out.contains("(net 2)"), "{out}");
    assert!(
        !out.contains("(net 0)"),
        "net 0 is the unconnected net: {out}"
    );
    // The item blocks name no net by string.
    let appended = &out[LEGACY.rfind(')').unwrap()..];
    assert!(!appended.contains("(net \""), "{appended}");

    let tree = parse_sexp(&out).unwrap();
    let read = tracks(&tree);
    assert_eq!(read.items.len(), 1);
    assert_eq!(read.items[0].net.as_deref(), Some("GND"));
    let read = vias(&tree);
    assert_eq!(read.items.len(), 1);
    assert_eq!(read.items[0].net.as_deref(), Some("VCC"));
    // Two copper layers: F.Cu and B.Cu.
    assert_eq!(
        read.items[0].layers,
        vec!["F.Cu".to_string(), "B.Cu".to_string()]
    );
}

#[test]
fn legacy_board_missing_the_net_is_refused() {
    // The honest case, with the net declared, is the test above.
    for plan in [
        plan(
            vec![track("SIG", "F.Cu", 0.25, (0.0, 0.0), (1.0, 0.0))],
            vec![],
            vec![],
        ),
        plan(vec![], vec![arc("SIG", "F.Cu")], vec![]),
        plan(vec![], vec![], vec![via("SIG", (0.0, 0.0), 0.6, 0.3)]),
    ] {
        let error = write_routing(LEGACY, &plan).unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("SIG"),
            "the refusal names the net: {message}"
        );
    }
}

#[test]
fn an_item_with_no_net_name_is_refused_never_net_0() {
    for board in [LEGACY, KICAD_10] {
        let plan = plan(
            vec![track("", "F.Cu", 0.25, (0.0, 0.0), (1.0, 0.0))],
            vec![],
            vec![],
        );
        assert!(write_routing(board, &plan).is_err());
    }
}

#[test]
fn only_the_closing_parenthesis_of_the_root_moves() {
    for board in [KICAD_10, LEGACY] {
        let out = write_routing(board, &everything()).unwrap();
        let close = board.rfind(')').unwrap();
        assert!(
            out.starts_with(&board[..close]),
            "the original text is a prefix"
        );
        assert!(out.ends_with(&board[close..]), "the tail is kept");
        assert!(out.len() > board.len());
        // Whole blocks, tab-indented, one item at a time.
        let added = &out[close..out.len() - (board.len() - close)];
        assert!(
            added.starts_with("\t(segment\n\t\t(start 1 2)\n\t\t(end 3.5 2)\n"),
            "{added}"
        );
        assert!(
            added.contains("\n\t\t(width 0.25)\n\t\t(layer \"F.Cu\")\n"),
            "{added}"
        );
        assert!(
            added.contains("\t(arc\n\t\t(start 10 10)\n\t\t(mid 11 11)\n\t\t(end 12 10)\n"),
            "{added}"
        );
        assert!(added.contains("\t(via\n\t\t(at 5 6)\n\t\t(size 0.6)\n\t\t(drill 0.3)\n\t\t(layers \"F.Cu\" \"B.Cu\")\n"), "{added}");
        assert!(added.ends_with("\t)\n"), "{added}");
        assert_eq!(added.matches("(uuid \"").count(), 3);
    }
}

#[test]
fn arcs_are_written_with_start_mid_and_end() {
    let out = write_routing(KICAD_10, &everything()).unwrap();
    let tree = parse_sexp(&out).unwrap();
    let arcs = tree.find_all("arc");
    assert_eq!(arcs.len(), 1);
    let a = arcs[0];
    let point = |tag: &str| -> (f64, f64) {
        let node = a.find(tag).unwrap();
        (
            node.get(1).unwrap().as_str().unwrap().parse().unwrap(),
            node.get(2).unwrap().as_str().unwrap().parse().unwrap(),
        )
    };
    assert_eq!(point("start"), (10.0, 10.0));
    assert_eq!(point("mid"), (11.0, 11.0));
    assert_eq!(point("end"), (12.0, 10.0));
    assert_eq!(a.find_f64("width"), Some(0.2));
    assert_eq!(a.find_str("layer"), Some("B.Cu"));
    assert_eq!(a.find("net").unwrap().get(1).unwrap().as_str(), Some("VCC"));
}

#[test]
fn fresh_v4_uuids_each_time() {
    let one = write_routing(KICAD_10, &everything()).unwrap();
    let two = write_routing(KICAD_10, &everything()).unwrap();
    let ids = |text: &str| -> Vec<String> {
        let tree = parse_sexp(text).unwrap();
        ["segment", "arc", "via"]
            .iter()
            .flat_map(|tag| tree.find_all(tag))
            .map(|item| item.find_str("uuid").unwrap().to_string())
            .collect()
    };
    let (a, b) = (ids(&one), ids(&two));
    assert_eq!(a.len(), 3);
    let all: BTreeSet<&String> = a.iter().chain(b.iter()).collect();
    assert_eq!(all.len(), 6, "no uuid repeats");
    for id in a.iter().chain(b.iter()) {
        let parsed = uuid::Uuid::parse_str(id).expect("a uuid");
        assert_eq!(parsed.get_version_num(), 4);
    }
}

#[test]
fn a_root_that_is_not_a_board_is_refused() {
    let sch = "(kicad_sch (version 20260206)\n)\n";
    assert!(write_routing(sch, &everything()).is_err());
    assert!(write_routing("(footprint \"R\")", &everything()).is_err());
    assert!(write_routing("not a board", &everything()).is_err());
    // The same refusal on the stitching writer.
    assert!(write_planned(sch, &Planned::default()).is_err());
    // A board is accepted (the pair).
    assert!(write_routing(KICAD_10, &plan(vec![], vec![], vec![])).is_ok());
}

#[test]
fn an_empty_plan_changes_nothing() {
    let out = write_routing(KICAD_10, &plan(vec![], vec![], vec![])).unwrap();
    assert_eq!(out, KICAD_10);
    assert_eq!(
        write_planned(KICAD_10, &Planned::default()).unwrap(),
        KICAD_10
    );
}

#[test]
fn the_root_is_found_with_strings_read_as_text() {
    // A parenthesis and an escaped quote inside the last strings must not move
    // the insertion point.
    let board = "(kicad_pcb (layers (0 \"F.Cu\" signal) (2 \"B.Cu\" signal)) (net 0 \"\") (net 1 \"G)ND(\") (gr_text \"say \\\"hi\\\" )) \" (at 0 0)))";
    let plan = plan(
        vec![track("G)ND(", "F.Cu", 0.25, (0.0, 0.0), (1.0, 0.0))],
        vec![],
        vec![],
    );
    let out = write_routing(board, &plan).unwrap();
    let close = board.rfind(')').unwrap();
    assert!(out.starts_with(&board[..close]));
    assert!(out.ends_with(&board[close..]));
    let tree = parse_sexp(&out).unwrap();
    assert_eq!(tracks(&tree).items.len(), 1);
    assert_eq!(
        tree.find_all("gr_text").len(),
        1,
        "the text is still a child of the root"
    );
    assert_eq!(tracks(&tree).items[0].net.as_deref(), Some("G)ND("));
}

#[test]
fn quotes_and_backslashes_in_a_net_name_are_escaped() {
    let name = "A\"B\\C";
    let board = "(kicad_pcb (version 20260206) (layers (0 \"F.Cu\" signal) (2 \"B.Cu\" signal)) (footprint \"R\" (pad \"1\" smd rect (net \"X\"))))";
    let plan = plan(
        vec![track(name, "F.Cu", 0.25, (0.0, 0.0), (1.0, 0.0))],
        vec![],
        vec![],
    );
    let out = write_routing(board, &plan).unwrap();
    assert!(out.contains(r#"(net "A\"B\\C")"#), "{out}");
    let tree = parse_sexp(&out).unwrap();
    assert_eq!(tracks(&tree).items[0].net.as_deref(), Some(name));
}

#[test]
fn numbers_have_at_most_six_decimals_without_exponent_or_negative_zero() {
    assert_eq!(num(0.1 + 0.2).unwrap(), "0.3");
    assert_eq!(num(12.5).unwrap(), "12.5");
    assert_eq!(num(3.0).unwrap(), "3");
    assert_eq!(num(-2.25).unwrap(), "-2.25");
    assert_eq!(num(0.1234567).unwrap(), "0.123457");
    assert_eq!(num(1e10).unwrap(), "10000000000");
    assert_eq!(num(-0.0).unwrap(), "0");
    assert_eq!(num(-1e-9).unwrap(), "0");
    assert_eq!(num(1e-9).unwrap(), "0");
    assert!(num(f64::NAN).is_err());
    assert!(num(f64::INFINITY).is_err());

    // In a written item.
    let plan = plan(
        vec![track("GND", "F.Cu", 0.25, (-0.0, 1e-9), (0.1 + 0.2, 7.0))],
        vec![],
        vec![],
    );
    let out = write_routing(KICAD_10, &plan).unwrap();
    assert!(out.contains("(start 0 0)"), "{out}");
    assert!(out.contains("(end 0.3 7)"), "{out}");
    // A coordinate that is not finite is refused, never written.
    let bad = self::plan(
        vec![track("GND", "F.Cu", 0.25, (f64::NAN, 0.0), (1.0, 0.0))],
        vec![],
        vec![],
    );
    assert!(write_routing(KICAD_10, &bad).is_err());
}

#[test]
fn a_via_needs_two_copper_layers_in_the_table_but_a_track_does_not() {
    let no_table =
        "(kicad_pcb (version 20260206) (footprint \"R\" (pad \"1\" smd rect (net \"GND\"))))";
    let one_layer = "(kicad_pcb (version 20260206) (layers (0 \"F.Cu\" signal) (25 \"Edge.Cuts\" user)) (footprint \"R\" (pad \"1\" smd rect (net \"GND\"))))";
    let via_plan = plan(vec![], vec![], vec![via("GND", (1.0, 1.0), 0.6, 0.3)]);
    assert!(write_routing(no_table, &via_plan).is_err());
    assert!(write_routing(one_layer, &via_plan).is_err());
    // The honest case: two copper layers.
    assert!(write_routing(KICAD_10, &via_plan).is_ok());
    // No via, no span needed.
    let track_plan = plan(
        vec![track("GND", "F.Cu", 0.25, (0.0, 0.0), (1.0, 0.0))],
        vec![],
        vec![],
    );
    assert!(write_routing(no_table, &track_plan).is_ok());
}

#[test]
fn the_via_span_is_the_first_and_last_copper_layer_of_the_table() {
    // Table order, not layer numbers, decides; a non-copper layer in between is skipped.
    let board = "(kicad_pcb (version 20260206) (layers (0 \"F.Cu\" signal) (9 \"F.Adhes\" user) (4 \"In1.Cu\" signal) (6 \"In2.Cu\" signal) (31 \"B.Cu\" signal) (25 \"Edge.Cuts\" user)) (footprint \"R\" (pad \"1\" smd rect (net \"GND\"))))";
    let out = write_routing(
        board,
        &plan(vec![], vec![], vec![via("GND", (1.0, 1.0), 0.6, 0.3)]),
    )
    .unwrap();
    assert!(out.contains("(layers \"F.Cu\" \"B.Cu\")"), "{out}");
    assert!(!out.contains("In1.Cu\" \"In2.Cu"), "{out}");
}

// ---------------------------------------------------------------- planned_items

/// A planner result shaped as `route_refine.rs` writes it (its `route_batch`
/// operation: tracks as polylines, vias with drill and pad size).
fn planner_result() -> Value {
    json!({
        "schema_version": 1,
        "planner": "route_refinement",
        "mutated": false,
        "status": "complete",
        "proposals": [
            {
                "id": "a",
                "kind": "stitching",
                "operations": [{
                    "tool": "route_batch",
                    "arguments": {
                        "board": "/x/board.kicad_pcb",
                        "tracks": [{
                            "net_name": "GND", "layer": "F.Cu", "width": 0.25,
                            "points": [{"x": 1.0, "y": 2.0}, {"x": 3.0, "y": 2.0}, {"x": 3.0, "y": 4.5}]
                        }],
                        "vias": [{"net_name": "GND", "x": 5.0, "y": 6.0, "drill": 0.3, "pad_size": 0.6}]
                    }
                }]
            },
            {
                "id": "b",
                "kind": "stitching",
                "operations": [{
                    "tool": "route_batch",
                    "arguments": {
                        "board": "/x/board.kicad_pcb",
                        "tracks": [],
                        "vias": [{"net_name": "VCC", "x": 7.0, "y": 8.0, "drill": 0.4, "pad_size": 0.8}]
                    }
                }]
            }
        ]
    })
}

#[test]
fn planner_shaped_result_is_read_in_order() {
    let items = planned_items(&planner_result()).unwrap();
    // A polyline of three points is two legs.
    assert_eq!(
        items.tracks,
        vec![
            PlannedTrack {
                net: "GND".into(),
                layer: "F.Cu".into(),
                width_mm: 0.25,
                x1_mm: 1.0,
                y1_mm: 2.0,
                x2_mm: 3.0,
                y2_mm: 2.0,
            },
            PlannedTrack {
                net: "GND".into(),
                layer: "F.Cu".into(),
                width_mm: 0.25,
                x1_mm: 3.0,
                y1_mm: 2.0,
                x2_mm: 3.0,
                y2_mm: 4.5,
            },
        ]
    );
    assert_eq!(
        items.vias,
        vec![
            PlannedVia {
                net: "GND".into(),
                x_mm: 5.0,
                y_mm: 6.0,
                size_mm: 0.6,
                drill_mm: 0.3,
            },
            PlannedVia {
                net: "VCC".into(),
                x_mm: 7.0,
                y_mm: 8.0,
                size_mm: 0.8,
                drill_mm: 0.4,
            },
        ]
    );
}

#[test]
fn a_proposal_that_modifies_copper_is_refused() {
    let mut plan = planner_result();
    plan["proposals"][1]["operations"].as_array_mut().unwrap().push(json!({
        "tool": "modify_trace",
        "arguments": {"board": "/x/board.kicad_pcb", "uuid": "u", "x1": 0, "y1": 0, "x2": 1, "y2": 1}
    }));
    let error = planned_items(&plan).unwrap_err();
    assert!(format!("{error:#}").contains("modify_trace"), "{error:#}");
    // Without it the plan is read (the pair).
    assert!(planned_items(&planner_result()).is_ok());
}

#[test]
fn an_unreadable_planner_result_is_refused() {
    assert!(planned_items(&json!({})).is_err(), "no proposals list");
    assert!(
        planned_items(&json!({"proposals": [{"id": "a"}]})).is_err(),
        "no operations"
    );
    assert_eq!(
        planned_items(&json!({"proposals": []})).unwrap(),
        Planned::default()
    );

    let mut one_point = planner_result();
    one_point["proposals"][0]["operations"][0]["arguments"]["tracks"][0]["points"] =
        json!([{"x": 1.0, "y": 2.0}]);
    assert!(planned_items(&one_point).is_err());

    let mut no_drill = planner_result();
    no_drill["proposals"][1]["operations"][0]["arguments"]["vias"][0]
        .as_object_mut()
        .unwrap()
        .remove("drill");
    assert!(planned_items(&no_drill).is_err());

    let mut no_width = planner_result();
    no_width["proposals"][0]["operations"][0]["arguments"]["tracks"][0]
        .as_object_mut()
        .unwrap()
        .remove("width");
    assert!(planned_items(&no_width).is_err());

    let mut zero_size = planner_result();
    zero_size["proposals"][1]["operations"][0]["arguments"]["vias"][0]["pad_size"] = json!(0.0);
    assert!(planned_items(&zero_size).is_err());

    let mut no_net = planner_result();
    no_net["proposals"][1]["operations"][0]["arguments"]["vias"][0]["net_name"] = json!("");
    assert!(planned_items(&no_net).is_err());
}

// ---------------------------------------------------------------- write_planned

#[test]
fn planned_items_are_written_and_read_back() {
    let items = planned_items(&planner_result()).unwrap();
    let out = write_planned(KICAD_10, &items).unwrap();
    let tree = parse_sexp(&out).unwrap();
    let read = tracks(&tree);
    assert_eq!(read.items.len(), 2);
    assert!(read
        .items
        .iter()
        .all(|t| t.net.as_deref() == Some("GND") && t.layer == "F.Cu"));
    assert!(close(read.items[1].y2, 4.5));
    let read = vias(&tree);
    assert_eq!(read.items.len(), 2);
    assert_eq!(read.items[0].net.as_deref(), Some("GND"));
    assert_eq!(read.items[1].net.as_deref(), Some("VCC"));
    assert!(close(read.items[1].size, 0.8) && close(read.items[1].drill, 0.4));
    // The original text is a prefix, as for the session writer.
    let end = KICAD_10.rfind(')').unwrap();
    assert!(out.starts_with(&KICAD_10[..end]));
    assert!(out.ends_with(&KICAD_10[end..]));
}

#[test]
fn planned_items_on_a_legacy_board_use_codes_and_refuse_an_undeclared_net() {
    let items = planned_items(&planner_result()).unwrap();
    let out = write_planned(LEGACY, &items).unwrap();
    assert!(!out.contains("(net 0)"));
    let tree = parse_sexp(&out).unwrap();
    assert_eq!(tracks(&tree).items[0].net.as_deref(), Some("GND"));
    assert_eq!(vias(&tree).items[1].net.as_deref(), Some("VCC"));

    let mut undeclared = items;
    undeclared.vias[1].net = "NOPE".to_string();
    let error = write_planned(LEGACY, &undeclared).unwrap_err();
    assert!(format!("{error:#}").contains("NOPE"));
}

// ---------------------------------------------------------------- copy_project

/// Writes `text` at `folder/relative`, creating folders.
fn put(folder: &Path, relative: &str, text: &str) {
    let path = folder.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Every file below `folder`, as sorted relative paths with `/`.
fn listing(folder: &Path) -> Vec<String> {
    fn walk(base: &Path, at: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                out.push(
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(folder, folder, &mut out);
    out.sort();
    out
}

/// A project folder with what a judged copy needs and what it must leave behind.
fn project(folder: &Path) -> PathBuf {
    for name in [
        "b.kicad_pcb",
        "b.kicad_pro",
        "b.kicad_prl",
        "b.kicad_dru",
        "fp-lib-table",
        "sym-lib-table",
        // Left behind:
        "b.kicad_sch",
        "other.kicad_pcb",
        "other.kicad_pro",
        "b.kicad_pcb-bak",
        "b.drc.json",
        "notes.txt",
    ] {
        put(folder, name, name);
    }
    put(folder, ".konnect/design_intent.json", "{}");
    put(folder, ".konnect/deep/more.json", "{}");
    put(folder, "lib.pretty/a.kicad_mod", "a");
    put(folder, "lib.pretty/sub/b.step", "b");
    put(folder, "plain/x.txt", "x");
    folder.join("b.kicad_pcb")
}

#[test]
fn copies_the_listed_files_and_folders_only() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let board = project(source.path());
    let into = target.path().join("scratch");

    let copied = copy_project(&board, &into).unwrap();
    assert_eq!(copied, into.join("b.kicad_pcb"));
    assert!(!copied.exists(), "the board itself is not copied");
    assert_eq!(
        listing(&into),
        vec![
            ".konnect/deep/more.json",
            ".konnect/design_intent.json",
            "b.kicad_dru",
            "b.kicad_prl",
            "b.kicad_pro",
            "fp-lib-table",
            "lib.pretty/a.kicad_mod",
            "lib.pretty/sub/b.step",
            "sym-lib-table",
        ]
    );
    // Contents are the source's.
    assert_eq!(
        std::fs::read_to_string(into.join("b.kicad_pro")).unwrap(),
        "b.kicad_pro"
    );
    // The source is untouched.
    assert!(board.exists());
    assert!(source.path().join("b.kicad_sch").exists());
}

#[test]
fn a_project_with_only_a_board_copies_nothing_but_still_names_the_path() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    put(source.path(), "solo.kicad_pcb", "(kicad_pcb)");
    let into = target.path().join("a/b/scratch");
    let copied = copy_project(&source.path().join("solo.kicad_pcb"), &into).unwrap();
    assert_eq!(copied, into.join("solo.kicad_pcb"));
    assert!(into.is_dir());
    assert!(listing(&into).is_empty());
}

#[test]
fn a_copy_onto_the_boards_own_folder_is_refused() {
    let source = tempfile::tempdir().unwrap();
    let board = project(source.path());
    assert!(copy_project(&board, source.path()).is_err());
    // Every file is as it was.
    assert_eq!(
        std::fs::read_to_string(source.path().join("b.kicad_pro")).unwrap(),
        "b.kicad_pro"
    );
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_anywhere_is_refused() {
    use std::os::unix::fs::symlink;
    let outside = tempfile::tempdir().unwrap();
    put(outside.path(), "secret.txt", "secret");

    // A linked project file.
    let source = tempfile::tempdir().unwrap();
    let board = project(source.path());
    std::fs::remove_file(source.path().join("b.kicad_prl")).unwrap();
    symlink(
        outside.path().join("secret.txt"),
        source.path().join("b.kicad_prl"),
    )
    .unwrap();
    let target = tempfile::tempdir().unwrap();
    assert!(copy_project(&board, &target.path().join("one")).is_err());

    // A link inside the design-intent folder.
    let source = tempfile::tempdir().unwrap();
    let board = project(source.path());
    symlink(
        outside.path().join("secret.txt"),
        source.path().join(".konnect/deep/linked.json"),
    )
    .unwrap();
    assert!(copy_project(&board, &target.path().join("two")).is_err());

    // A linked library folder.
    let source = tempfile::tempdir().unwrap();
    let board = project(source.path());
    symlink(outside.path(), source.path().join("linked.pretty")).unwrap();
    assert!(copy_project(&board, &target.path().join("three")).is_err());

    // A link elsewhere in the folder, that the copy never looks at, is no reason.
    let source = tempfile::tempdir().unwrap();
    let board = project(source.path());
    symlink(
        outside.path().join("secret.txt"),
        source.path().join("unrelated.txt"),
    )
    .unwrap();
    let copied = copy_project(&board, &target.path().join("four")).unwrap();
    assert_eq!(copied, target.path().join("four/b.kicad_pcb"));
    assert!(!target.path().join("four/unrelated.txt").exists());
}
