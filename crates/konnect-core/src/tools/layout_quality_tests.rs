use super::*;
use serde_json::json;

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/layout_quality")
        .join(name)
}

struct Fx {
    _dir: tempfile::TempDir,
    board: std::path::PathBuf,
}

/// A scratch copy of a fixture board, with an optional design-intent record
/// and project file beside it.
fn fx(name: &str, intent: Option<Value>, pro: Option<Value>) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("b.kicad_pcb");
    std::fs::copy(fixture_path(name), &board).unwrap();
    std::fs::write(
        dir.path().join("b.kicad_pro"),
        pro.map(|p| p.to_string()).unwrap_or_else(|| "{}".into()),
    )
    .unwrap();
    if let Some(i) = intent {
        let path = design_intent_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_string_pretty(&i).unwrap()).unwrap();
    }
    Fx { _dir: dir, board }
}

fn run_with(f: &Fx, args: Value) -> Value {
    let params = Params::from_args(&args).unwrap_or_else(|_| panic!("bad args"));
    let result = audit(&f.board, &params).unwrap();
    assert!(!result.is_error, "{result:?}");
    match result.content.first() {
        Some(crate::mcp::protocol::ToolContent::Text { text }) => {
            serde_json::from_str(text).unwrap()
        }
        other => panic!("expected text, got {other:?}"),
    }
}

fn run(f: &Fx) -> Value {
    run_with(f, json!({}))
}

fn check<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["item"] == id)
        .unwrap_or_else(|| panic!("no check {id}: {report}"))
}

fn findings<'a>(report: &'a Value, item: &str) -> Vec<&'a Value> {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["item"] == item)
        .collect()
}

/// Prints one line per board for `LQ_PROBE_BOARDS` (paths separated by `:`), and writes the
/// full result beside it when `LQ_PROBE_OUT` names a folder. Ignored: it needs real boards.
#[test]
#[ignore = "needs LQ_PROBE_BOARDS"]
fn probe_boards() {
    let boards = std::env::var("LQ_PROBE_BOARDS").expect("LQ_PROBE_BOARDS");
    for path in boards.split(':').filter(|p| !p.is_empty()) {
        let dir = tempfile::tempdir().unwrap();
        let name = std::path::Path::new(path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let board = dir.path().join("b.kicad_pcb");
        std::fs::copy(path, &board).unwrap();
        if let Some(pro) = std::path::Path::new(path)
            .with_extension("kicad_pro")
            .exists()
            .then(|| std::path::Path::new(path).with_extension("kicad_pro"))
        {
            std::fs::copy(pro, dir.path().join("b.kicad_pro")).unwrap();
        }
        let started = std::time::Instant::now();
        let params = Params::from_args(&json!({})).unwrap_or_else(|_| panic!("args"));
        let result = audit(&board, &params).unwrap();
        let Some(crate::mcp::protocol::ToolContent::Text { text }) = result.content.first() else {
            panic!()
        };
        let r: Value = serde_json::from_str(text).unwrap();
        if r["status"] != "checked" {
            println!("PROBE {name}: {}", r);
            continue;
        }
        let cats: Vec<String> = r["quality"]["categories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| format!("{}={}", c["category"].as_str().unwrap(), c["score"]))
            .collect();
        println!(
            "PROBE {name}: score {} | {} | unrouted {} | err {} warn {} | {:.1}s | bytes {}",
            r["quality"]["score"],
            cats.join(" "),
            r["completion"]["unrouted_nets"],
            r["counts"]["error"],
            r["counts"]["warning"],
            started.elapsed().as_secs_f64(),
            text.len()
        );
        println!(
            "PROBE   metrics {}",
            serde_json::to_string(&r["metrics"]).unwrap()
        );
        if let Ok(out) = std::env::var("LQ_PROBE_OUT") {
            std::fs::write(
                std::path::Path::new(&out).join(format!("{name}.json")),
                serde_json::to_string_pretty(&r).unwrap(),
            )
            .unwrap();
        }
    }
}

// ─── Helpers for the assertions ──────────────────────────────────────────────

/// A fixture's text with every saved zone fill removed (zones left unfilled).
fn without_fills(name: &str) -> String {
    let text = std::fs::read_to_string(fixture_path(name)).unwrap();
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(i) = rest.find("(filled_polygon") {
        out.push_str(&rest[..i]);
        let mut depth = 0;
        let mut end = i;
        for (k, c) in rest[i..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + k + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.replace("(fill yes", "(fill")
}

/// A fixture's text with every track and via removed.
fn without_copper_routing(name: &str) -> String {
    let text = std::fs::read_to_string(fixture_path(name)).unwrap();
    let mut out = String::new();
    let mut rest = text.as_str();
    loop {
        let next = ["\n\t(segment", "\n\t(via", "\n\t(arc"]
            .iter()
            .filter_map(|t| rest.find(t))
            .min();
        let Some(i) = next else { break };
        out.push_str(&rest[..i]);
        let mut depth = 0;
        let mut end = i + 1;
        for (k, c) in rest[i + 1..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1 + k + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn fx_text(text: &str, intent: Option<Value>) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("b.kicad_pcb");
    std::fs::write(&board, text).unwrap();
    std::fs::write(dir.path().join("b.kicad_pro"), "{}").unwrap();
    if let Some(i) = intent {
        let path = design_intent_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, i.to_string()).unwrap();
    }
    Fx { _dir: dir, board }
}

fn buck_intent() -> Value {
    json!({ "schema_version": 1, "rails": [
        { "name": "OUT1", "nets": ["VOUT1"], "voltage_v": 3.3, "source_ref": "U1" },
        { "name": "OUT2", "nets": ["VOUT2"], "voltage_v": 3.3, "source_ref": "U2" }] })
}

fn neck_intent(amps: f64) -> Value {
    json!({ "schema_version": 1, "currents": [
        { "net": "P5V_NECK", "max_current_a": amps }, { "net": "P5V_WIDE", "max_current_a": amps }] })
}

fn score(report: &Value, id: &str) -> f64 {
    check(report, id)["score"]
        .as_f64()
        .unwrap_or_else(|| panic!("no score for {id}: {}", check(report, id)))
}

fn f_for<'a>(report: &'a Value, item: &str, needle: &str) -> Vec<&'a Value> {
    findings(report, item)
        .into_iter()
        .filter(|f| f["subject"].as_str().unwrap_or("").contains(needle))
        .collect()
}

// ─── §3 reference planes ─────────────────────────────────────────────────────

#[test]
fn a_track_over_a_plane_split_and_a_slot_is_measured_and_named() {
    let r = run(&fx("plane_split.kicad_pcb", None, None));
    let c = check(&r, "3.1");
    assert_eq!(c["status"], "checked");
    let nets = c["metrics"]["nets"].as_array().unwrap();
    let ok = nets.iter().find(|n| n["net"] == "CLK_OK").unwrap();
    let bad = nets.iter().find(|n| n["net"] == "CLK_SPLIT").unwrap();
    // The clean net sits over solid ground its whole length.
    assert!(ok["coverage"].as_f64().unwrap() > 0.98, "{ok}");
    assert_eq!(ok["runs_off_reference"], 0);
    // The other crosses a 3 mm slot (x 20..23) and the 3V3 region (x 40..54): 17 of 48 mm.
    let cov = bad["coverage"].as_f64().unwrap();
    assert!((0.58..0.72).contains(&cov), "{bad}");
    let other = bad["other_net_plane_mm"].as_f64().unwrap();
    assert!((13.0..15.0).contains(&other), "3V3 stretch {other}");
    let void = bad["void_mm"].as_f64().unwrap();
    assert!((2.0..4.5).contains(&void), "slot {void}");
    // The slot, the 3V3 region, and the sliver of clearance between that region and ground.
    assert!(bad["runs_off_reference"].as_u64().unwrap() >= 2, "{bad}");
    // One finding, for the bad net only, with the measured value, the threshold and a UUID.
    let fs = findings(&r, "3.1");
    assert_eq!(fs.len(), 1, "{fs:?}");
    let f = fs[0];
    assert_eq!(f["subject"], "CLK_SPLIT");
    assert!(f["issue"].as_str().unwrap().contains("'3V3' copper"));
    assert_eq!(f["threshold"]["min_reference_coverage"], 0.9);
    assert_eq!(f["threshold"]["max_void_run_mm"], 2.0);
    assert!(f["checklist"].as_str().unwrap().starts_with("§3"));
    assert!(!f["uuids"].as_array().unwrap().is_empty());
    // Ground in the same board is connected: no §3.3 finding.
    assert!(findings(&r, "3.3").is_empty(), "{:?}", findings(&r, "3.3"));
}

#[test]
fn a_looser_threshold_clears_the_split_finding() {
    let f = fx("plane_split.kicad_pcb", None, None);
    let r = run_with(
        &f,
        json!({ "min_reference_coverage": 0.5, "max_void_run_mm": 20.0 }),
    );
    assert!(findings(&r, "3.1").is_empty());
    assert_eq!(
        r["thresholds"]["min_reference_coverage"]["source"],
        "argument"
    );
    assert_eq!(r["thresholds"]["max_void_run_mm"]["value"], 20.0);
    assert_eq!(r["thresholds"]["sample_mm"]["source"], "default");
}

#[test]
fn zones_without_a_saved_fill_are_not_judged_as_voids() {
    let r = run(&fx_text(&without_fills("plane_split.kicad_pcb"), None));
    let c = check(&r, "3.1");
    assert_eq!(c["status"], "not_run", "{c}");
    assert!(
        c["reason"].as_str().unwrap().contains("no saved fill"),
        "{c}"
    );
    assert!(findings(&r, "3.1").is_empty());
    assert!(r["quality"]["not_scored"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["item"] == "3.1"));
}

#[test]
fn a_layer_change_between_reference_planes_needs_a_stitching_via() {
    let r = run(&fx("layer_change.kicad_pcb", None, None));
    let c = check(&r, "3.2");
    assert_eq!(c["status"], "checked", "{c}");
    // Two layer changes per net, four in all; only the stitched net's two are covered.
    assert_eq!(c["metrics"]["layer_changes"], 4);
    assert_eq!(c["metrics"]["needing_stitch"], 4);
    assert_eq!(c["metrics"]["stitched"], 2);
    assert_eq!(score(&r, "3.2"), 0.5);
    let fs = findings(&r, "3.2");
    assert_eq!(fs.len(), 2, "{fs:?}");
    assert!(fs
        .iter()
        .all(|f| f["subject"].as_str().unwrap().starts_with("CLK_BARE")));
    let nearest = fs[0]["measured"]["nearest_stitch_mm"].as_f64().unwrap();
    assert!(nearest > 7.0, "{nearest}");
    assert_eq!(fs[0]["threshold"]["stitch_via_max_mm"], 3.0);
    assert_eq!(fs[0]["uuids"].as_array().unwrap().len(), 2);
    assert!(f_for(&r, "3.2", "CLK_STITCHED").is_empty());
}

#[test]
fn a_wide_stitch_limit_accepts_a_distant_via() {
    let r = run_with(
        &fx("layer_change.kicad_pcb", None, None),
        json!({ "stitch_via_max_mm": 30.0 }),
    );
    assert!(findings(&r, "3.2").is_empty());
    assert_eq!(score(&r, "3.2"), 1.0);
}

#[test]
fn ground_cut_off_from_its_pads_is_an_error_and_a_floating_island_a_warning() {
    // Remove the ground vias: the pads reach no plane.
    let text = std::fs::read_to_string(fixture_path("plane_split.kicad_pcb")).unwrap();
    let mut stripped = String::new();
    let mut rest = text.as_str();
    while let Some(i) = rest.find("\n\t(via") {
        stripped.push_str(&rest[..i]);
        let mut depth = 0;
        let mut end = i + 1;
        for (k, c) in rest[i + 1..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1 + k + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest = &rest[end..];
    }
    stripped.push_str(rest);
    let r = run(&fx_text(&stripped, None));
    let errors: Vec<_> = findings(&r, "3.3")
        .into_iter()
        .filter(|f| f["severity"] == "error")
        .collect();
    assert_eq!(errors.len(), 1, "{:?}", findings(&r, "3.3"));
    assert!(errors[0]["issue"]
        .as_str()
        .unwrap()
        .contains("separate copper groups"));
    assert!(findings(&r, "3.3")
        .iter()
        .any(|f| f["severity"] == "warning"
            && f["issue"]
                .as_str()
                .unwrap()
                .contains("touches no pad of the net")));
    assert!(score(&r, "3.3") < 0.5);
}

// ─── §4 decoupling and current ───────────────────────────────────────────────

#[test]
fn a_tight_decoupling_loop_passes_and_a_large_one_is_measured_on_both_sides() {
    let r = run(&fx("decoupling.kicad_pcb", None, None));
    let c = check(&r, "4.1");
    assert_eq!(c["status"], "checked", "{c}");
    let pins = c["metrics"]["pins"].as_array().unwrap();
    let u1 = pins.iter().find(|p| p["pin"] == "U1.1").unwrap();
    let u2 = pins.iter().find(|p| p["pin"] == "U2.1").unwrap();
    let tight = u1["loop"]["loop_mm"].as_f64().unwrap();
    let loose = u2["loop"]["loop_mm"].as_f64().unwrap();
    assert!(tight < 6.0, "{tight}");
    assert!(loose > 35.0, "{loose}");
    assert_eq!(u1["loop"]["capacitor"], "C1");
    assert_eq!(u2["loop"]["capacitor"], "C2");
    // The ground side is part of the loop: its leg runs through a plane and costs most of the excess.
    let legs = u2["loop"]["legs"].as_array().unwrap();
    assert_eq!(legs.len(), 2);
    assert_eq!(legs[1]["net"], "GND");
    assert!(legs[1]["copper_mm"].as_f64().unwrap() > 15.0, "{legs:?}");
    assert!(legs[1]["layer_changes"].as_u64().unwrap() >= 2);
    assert!(u2["loop"]["area_mm2"].as_f64().unwrap() > 20.0);
    assert!(u1["loop"]["area_mm2"].as_f64().unwrap() < 10.0);
    // Only the loose one is a finding, with the threshold and UUIDs.
    let fs = findings(&r, "4.1");
    assert_eq!(fs.len(), 1, "{fs:?}");
    assert!(fs[0]["subject"].as_str().unwrap().starts_with("U2.1"));
    assert_eq!(fs[0]["severity"], "error");
    assert_eq!(fs[0]["threshold"]["decap_max_loop_mm"], 10.0);
    assert!(fs[0]["uuids"].as_array().unwrap().len() >= 2);
    assert_eq!(score(&r, "4.1"), (1.0 + 0.0) / 2.0);
}

#[test]
fn a_pin_with_no_capacitor_in_range_is_named() {
    let r = run_with(
        &fx("decoupling.kicad_pcb", None, None),
        json!({ "decap_search_mm": 1.0 }),
    );
    let fs = findings(&r, "4.1");
    assert_eq!(fs.len(), 2, "{fs:?}");
    assert!(fs
        .iter()
        .all(|f| f["issue"].as_str().unwrap().contains("no capacitor")));
    assert_eq!(score(&r, "4.1"), 0.0);
    assert_eq!(check(&r, "4.1")["metrics"]["pins_without_capacitor"], 2);
}

#[test]
fn a_neck_down_on_a_recorded_current_is_found_where_it_is() {
    let r = run(&fx("neck_down.kicad_pcb", Some(neck_intent(3.0)), None));
    let c = check(&r, "4.3");
    assert_eq!(c["status"], "checked", "{c}");
    let fs = findings(&r, "4.3");
    assert_eq!(fs.len(), 1, "{fs:?}");
    let f = fs[0];
    assert!(f["subject"].as_str().unwrap().starts_with("P5V_NECK"));
    // The 0.2 mm stretch at x 25..28 on y = 10, within the raster's one-step accuracy.
    let w = f["measured"]["narrowest_mm"].as_f64().unwrap();
    assert!((0.1..0.35).contains(&w), "{w}");
    assert!(f["threshold"]["required_mm"].as_f64().unwrap() > 1.3);
    assert_eq!(f["threshold"]["current_from"], "currents");
    let (x, y) = (
        f["at"]["x"].as_f64().unwrap(),
        f["at"]["y"].as_f64().unwrap(),
    );
    assert!(
        (24.5..28.5).contains(&x) && (9.0..11.0).contains(&y),
        "{x},{y}"
    );
    assert!(!f["uuids"].as_array().unwrap().is_empty());
    // The wide net is clean and measured at about its 2 mm.
    let paths = c["metrics"]["paths"].as_array().unwrap();
    let wide = paths.iter().find(|p| p["net"] == "P5V_WIDE").unwrap();
    assert!(
        (1.8..2.3).contains(&wide["narrowest_mm"].as_f64().unwrap()),
        "{wide}"
    );
    assert!(score(&r, "4.3") < 1.0);
    // The existing current audit is reused: same net, an error naming the segment's UUID.
    let sizing = findings(&r, "4.2");
    assert_eq!(sizing.len(), 1, "{sizing:?}");
    assert_eq!(sizing[0]["severity"], "error");
    assert_eq!(sizing[0]["subject"], "P5V_NECK");
    assert_eq!(sizing[0]["uuids"].as_array().unwrap().len(), 1);
}

#[test]
fn a_small_current_needs_no_neck_down_finding() {
    let r = run_with(
        &fx("neck_down.kicad_pcb", Some(neck_intent(0.3)), None),
        json!({ "bottleneck_min_current_a": 0.1 }),
    );
    assert_eq!(check(&r, "4.3")["status"], "checked");
    assert!(findings(&r, "4.3").is_empty(), "{:?}", findings(&r, "4.3"));
    assert!(findings(&r, "4.2").is_empty());
    assert_eq!(score(&r, "4.3"), 1.0);
}

#[test]
fn without_a_record_the_current_checks_say_why_they_did_not_run() {
    let r = run(&fx("neck_down.kicad_pcb", None, None));
    for id in ["4.2", "4.3", "3.4"] {
        let c = check(&r, id);
        assert_eq!(c["status"], "not_run", "{id}: {c}");
        assert!(
            c["reason"].as_str().unwrap().contains("current")
                || c["reason"].as_str().unwrap().contains("design-intent"),
            "{c}"
        );
        assert!(c["score"].is_null());
    }
}

// ─── §8 switching regulators ─────────────────────────────────────────────────

#[test]
fn a_bucks_input_loop_is_measured_and_the_far_capacitor_is_flagged() {
    let r = run(&fx("buck.kicad_pcb", Some(buck_intent()), None));
    let c = check(&r, "8.1");
    assert_eq!(c["status"], "checked", "{c}");
    let nodes = c["metrics"]["switch_nodes"].as_array().unwrap();
    let n1 = nodes.iter().find(|n| n["switch_net"] == "SW1").unwrap();
    let n2 = nodes.iter().find(|n| n["switch_net"] == "SW2").unwrap();
    assert_eq!(n1["topology"], "buck");
    assert!(n1["topology_basis"]
        .as_str()
        .unwrap()
        .contains("supplied by U1"));
    assert!(n1["loop"]["loop_mm"].as_f64().unwrap() < 8.0);
    assert!(n2["loop"]["loop_mm"].as_f64().unwrap() > 25.0);
    assert_eq!(n1["loop"]["capacitor"], "C11");
    assert_eq!(n2["loop"]["capacitor"], "C21");
    let fs = findings(&r, "8.1");
    assert_eq!(fs.len(), 1, "{fs:?}");
    assert!(fs[0]["subject"].as_str().unwrap().starts_with("SW2 buck"));
    assert_eq!(fs[0]["threshold"]["hot_loop_max_mm"], 12.0);
    assert!(fs[0]["issue"].as_str().unwrap().contains("input loop"));
    assert_eq!(score(&r, "8.1"), 0.5);
    // The same pin is also an ordinary decoupling finding, not a second hot-loop one.
    assert!(findings(&r, "4.1")
        .iter()
        .any(|f| f["subject"].as_str().unwrap().starts_with("U2.1")));
}

#[test]
fn an_undecided_topology_is_needs_human_and_an_argument_decides_it() {
    let f = fx("buck.kicad_pcb", None, None);
    let r = run(&f);
    let c = check(&r, "8.1");
    assert_eq!(c["status"], "not_run", "{c}");
    assert!(r["needs_human"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["item"] == "8.1"
            && n["why"]
                .as_str()
                .unwrap()
                .contains("topology not determined")));
    let r = run_with(
        &f,
        json!({ "switching_regulators": [{ "switch_net": "SW1", "topology": "buck" }, { "switch_net": "SW2", "topology": "buck" }] }),
    );
    assert_eq!(check(&r, "8.1")["status"], "checked");
    assert_eq!(findings(&r, "8.1").len(), 1);
}

#[test]
fn switch_node_copper_and_a_feedback_route_beside_it_are_measured() {
    let r = run(&fx("buck.kicad_pcb", Some(buck_intent()), None));
    let c = check(&r, "8.2");
    assert_eq!(c["status"], "checked", "{c}");
    let nodes = c["metrics"]["switch_nodes"].as_array().unwrap();
    let a1 = nodes.iter().find(|n| n["switch_net"] == "SW1").unwrap();
    let a2 = nodes.iter().find(|n| n["switch_net"] == "SW2").unwrap();
    // 6.5 mm × 0.8 mm track plus the pads: about 6.5 mm².
    assert!(
        (5.0..9.0).contains(&a1["copper_area_mm2"].as_f64().unwrap()),
        "{a1}"
    );
    // Feedback found from the pin called FB; far from the node on U1, 0.4 mm from it on U2.
    assert_eq!(a1["feedback_nets"][0]["net"], "FB1");
    assert!(a1["feedback_min_gap_mm"].as_f64().unwrap() > 2.0);
    assert!(a2["feedback_min_gap_mm"].as_f64().unwrap() < 0.6);
    let fs = findings(&r, "8.2");
    assert_eq!(fs.len(), 1, "{fs:?}");
    assert!(fs[0]["subject"].as_str().unwrap().starts_with("SW2"));
    assert_eq!(fs[0]["threshold"]["min_switch_feedback_gap_mm"], 1.0);
    assert!(!fs[0]["uuids"].as_array().unwrap().is_empty());
    // A tighter area limit flags the copper itself.
    let r = run_with(
        &fx("buck.kicad_pcb", Some(buck_intent()), None),
        json!({ "max_switch_node_area_mm2": 3.0 }),
    );
    assert_eq!(
        findings(&r, "8.2")
            .iter()
            .filter(|f| f["measured"].get("copper_area_mm2").is_some())
            .count(),
        2
    );
}

#[test]
fn a_generous_hot_loop_limit_clears_the_finding() {
    let r = run_with(
        &fx("buck.kicad_pcb", Some(buck_intent()), None),
        json!({ "hot_loop_max_mm": 100.0, "hot_loop_max_area_mm2": 500.0 }),
    );
    assert!(findings(&r, "8.1").is_empty());
    assert!(score(&r, "8.1") > 0.9);
}

// ─── §5 routing ──────────────────────────────────────────────────────────────

#[test]
fn detours_vias_and_stubs_are_measured_against_the_pin_to_pin_tree() {
    let r = run(&fx("routing.kicad_pcb", None, None));
    let c = check(&r, "5.1");
    assert_eq!(c["status"], "checked", "{c}");
    let rows = c["metrics"]["worst_detours"].as_array().unwrap();
    let row = |n: &str| {
        rows.iter()
            .find(|r| r["net"] == n)
            .unwrap_or_else(|| panic!("{n}: {rows:?}"))
    };
    let d = row("N_DETOUR");
    assert!(
        (2.9..3.1).contains(&d["detour_ratio"].as_f64().unwrap()),
        "{d}"
    );
    assert_eq!(d["vias"], 2);
    assert_eq!(row("N_STUB")["dangling_ends"], 1);
    assert!((0.99..1.01).contains(&row("N_CLEAN")["detour_ratio"].as_f64().unwrap()));
    let fs = findings(&r, "5.1");
    let subjects: Vec<&str> = fs.iter().map(|f| f["subject"].as_str().unwrap()).collect();
    assert!(
        subjects.contains(&"N_DETOUR")
            && subjects.contains(&"N_STUB")
            && subjects.contains(&"N_CORNER"),
        "{subjects:?}"
    );
    assert!(!subjects.contains(&"N_CLEAN"));
    let stub = fs.iter().find(|f| f["subject"] == "N_STUB").unwrap();
    assert_eq!(stub["measured"]["dangling_ends"], 1);
    assert_eq!(stub["uuids"].as_array().unwrap().len(), 1);
    assert_eq!(stub["threshold"]["max_detour_ratio"], 1.5);
    assert!(r["metrics"]["5.1.wire_to_hpwl"].as_f64().unwrap() > 1.0);
}

#[test]
fn unrelated_tracks_running_close_together_are_reported_and_spaced_ones_are_not() {
    let r = run(&fx("routing.kicad_pcb", None, None));
    let fs = findings(&r, "5.2");
    assert_eq!(fs.len(), 1, "{fs:?}");
    assert_eq!(fs[0]["subject"], "N_PAR_A / N_PAR_B");
    assert!((19.0..21.0).contains(&fs[0]["measured"]["parallel_mm"].as_f64().unwrap()));
    assert!(fs[0]["measured"]["closest_gap_mm"].as_f64().unwrap() < 0.3);
    assert!(fs[0]["uuids"].as_array().unwrap().len() == 2);
    assert!(score(&r, "5.2") < 0.8);
}

#[test]
fn acute_corners_reuse_the_acid_trap_check_and_right_angles_are_counted() {
    let r = run(&fx("routing.kicad_pcb", None, None));
    let m = &check(&r, "5.3")["metrics"];
    assert_eq!(m["acute_corners"], 1, "{m}");
    assert_eq!(m["acute_found_by_this_scan"], 1, "the two scans agree: {m}");
    assert!(m["right_angle_corners"].as_u64().unwrap() >= 6);
    let acute = findings(&r, "5.3")
        .into_iter()
        .find(|f| f["issue"].as_str().unwrap().starts_with("Acid trap"))
        .unwrap();
    assert_eq!(acute["subject"], "N_CORNER");
    assert_eq!(acute["severity"], "warning");
    assert!(findings(&r, "5.3").iter().any(
        |f| f["severity"] == "info" && f["issue"].as_str().unwrap().starts_with("Right-angle")
    ));
}

#[test]
fn a_pair_that_drifts_apart_is_flagged_and_a_constant_gap_pair_is_not() {
    let r = run(&fx("routing.kicad_pcb", None, None));
    let c = check(&r, "5.4");
    assert_eq!(c["status"], "checked", "{c}");
    let pairs = c["metrics"]["pairs"].as_array().unwrap();
    let good = pairs.iter().find(|p| p["pair"][0] == "DG_P").unwrap();
    let bad = pairs.iter().find(|p| p["pair"][0] == "DB_P").unwrap();
    assert!(good["coupled_fraction"].as_f64().unwrap() > 0.95, "{good}");
    assert!(bad["coupled_fraction"].as_f64().unwrap() < 0.5, "{bad}");
    assert_eq!(good["source"], "name heuristic");
    let fs: Vec<_> = findings(&r, "5.4")
        .into_iter()
        .filter(|f| f["severity"] == "warning")
        .collect();
    assert_eq!(fs.len(), 1, "{fs:?}");
    assert!(fs[0]["subject"].as_str().unwrap().starts_with("DB_P"));
    // Length mismatch is information only: matching needs the interface's skew limit.
    assert!(findings(&r, "5.4")
        .iter()
        .any(|f| f["severity"] == "info" && f["subject"].as_str().unwrap().ends_with("length")));
    assert!(r["needs_human"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["item"] == "5.5"));
}

#[test]
fn a_recorded_pair_replaces_the_name_guess() {
    let intent = json!({ "schema_version": 1, "interfaces": [{
        "name": "USBX", "kind": "usb2", "nets": ["DG_P", "DG_N"],
        "differential_pairs": [{ "positive": "DG_P", "negative": "DG_N" }] }] });
    let r = run(&fx("routing.kicad_pcb", Some(intent), None));
    let pairs = r["net_roles"]["differential_pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0]["source"], "design_intent interface 'USBX'");
    // The record's interface nets are the critical ones now; DB_* is no longer judged.
    assert_eq!(r["net_roles"]["critical_basis"], "recorded_or_named");
    assert!(r["net_roles"]["critical"]["nets"].get("DG_P").is_some());
}

// ─── Roles, summary and refusals ─────────────────────────────────────────────

#[test]
fn nets_are_judged_critical_from_netclasses_and_the_result_says_why() {
    let pro = json!({ "net_settings": { "classes": [], "netclass_patterns": [
        { "netclass": "HighSpeed", "pattern": "N_CLEAN" },
        { "netclass": "Power", "pattern": "N_STUB" }] } });
    let r = run(&fx("routing.kicad_pcb", None, Some(pro)));
    let crit = &r["net_roles"]["critical"]["nets"];
    assert_eq!(crit["N_CLEAN"], "netclass 'HighSpeed'");
    assert!(r["net_roles"]["power"]["nets"].get("N_STUB").is_some());
    assert_eq!(r["net_roles"]["netclass_assigned_nets"], 2);
}

#[test]
fn without_any_record_every_long_signal_is_judged_and_it_is_stated() {
    let r = run(&fx("neck_down.kicad_pcb", None, None));
    assert_eq!(r["net_roles"]["design_intent"], "absent");
    assert_eq!(r["net_roles"]["critical_basis"], "fallback_all_signals");
    let why = r["net_roles"]["critical"]["nets"]["P5V_NECK"]
        .as_str()
        .unwrap_or("");
    assert!(
        why.starts_with("fallback") || r["net_roles"]["critical"]["count"].as_u64().unwrap() == 0,
        "{why}"
    );
    let r = run_with(
        &fx("plane_split.kicad_pcb", None, None),
        json!({ "signal_scope": "all_signals" }),
    );
    assert_eq!(r["net_roles"]["critical_basis"], "all_signals");
}

#[test]
fn a_board_with_nets_left_unrouted_scores_below_the_routed_one() {
    let full = run(&fx("routing.kicad_pcb", None, None));
    let open = run(&fx_text(&without_copper_routing("routing.kicad_pcb"), None));
    assert_eq!(full["completion"]["complete"], true);
    assert_eq!(open["completion"]["complete"], false);
    assert!(open["completion"]["unrouted_nets"].as_u64().unwrap() >= 10);
    let (a, b) = (
        full["quality"]["score"].as_f64().unwrap(),
        open["quality"]["score"].as_f64().unwrap(),
    );
    assert!(b < a * 0.5, "routed {a}, stripped {b}");
    assert!(open["summary"].as_str().unwrap().contains("INCOMPLETE"));
    assert!(open["quality"]["routed_share"].as_f64().unwrap() < 0.2);
}

#[test]
fn good_routing_scores_above_poor_routing_of_the_same_kind() {
    // Same checks, same thresholds: the tight buck beats the loose one, per pin and per node.
    let r = run(&fx("buck.kicad_pcb", Some(buck_intent()), None));
    let pins = check(&r, "4.1")["metrics"]["pins"]
        .as_array()
        .unwrap()
        .clone();
    let loop_of = |p: &str| {
        pins.iter().find(|x| x["pin"] == p).unwrap()["loop"]["loop_mm"]
            .as_f64()
            .unwrap()
    };
    assert!(loop_of("U1.1") * 4.0 < loop_of("U2.1"));
    assert!(r["quality"]["score"].as_f64().unwrap() > 0.0);
    let cats = r["quality"]["categories"].as_array().unwrap();
    assert!(cats
        .iter()
        .all(|c| c["score"].is_null() || (0.0..=100.0).contains(&c["score"].as_f64().unwrap())));
    assert!(r["quality"]["weight_measured"].as_f64().unwrap() <= 1.0);
}

#[test]
fn every_threshold_is_listed_with_its_source_and_every_finding_names_its_item() {
    let r = run(&fx("routing.kicad_pcb", None, None));
    for Knob(name, default, ..) in KNOBS {
        assert_eq!(r["thresholds"][*name]["default"], *default, "{name}");
        assert!(r["thresholds"][*name]["limits"].as_str().unwrap().len() > 10);
    }
    for f in r["findings"].as_array().unwrap() {
        assert!(!f["checklist"].as_str().unwrap().is_empty(), "{f}");
        assert!(f["measured"].is_object(), "{f}");
        assert!(f["threshold"].is_object(), "{f}");
    }
    assert!(r["needs_human"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["item"] == "11"));
    assert!(r["not_measured"].as_array().unwrap().len() >= 3);
}

#[test]
fn bad_arguments_are_refused_with_the_field_named() {
    for args in [
        json!({ "sample_mm": 0.0 }),
        json!({ "max_detour_ratio": "x" }),
        json!({ "signal_scope": "some" }),
        json!({ "critical_nets": "CLK" }),
        json!({ "max_findings": 0 }),
        json!({ "switching_regulators": [{ "switch_net": "SW", "topology": "flyback" }] }),
    ] {
        assert!(Params::from_args(&args).is_err(), "{args}");
    }
    assert!(Params::from_args(&json!({ "sample_mm": 0.5, "critical_nets": ["A"] })).is_ok());
}

#[test]
fn the_tool_is_registered_with_a_schema_that_forbids_unknown_arguments() {
    let defs = tools();
    assert_eq!(defs.len(), 1);
    let t = &defs[0];
    assert_eq!(t.name, "audit_layout_quality");
    assert!(t
        .input_validator
        .is_valid(&json!({ "board": "x.kicad_pcb" })));
    assert!(!t
        .input_validator
        .is_valid(&json!({ "board": "x.kicad_pcb", "surprise": 1 })));
    assert!(!t
        .input_validator
        .is_valid(&json!({ "board": "x.kicad_pcb", "sample_mm": 99 })));
    assert!(!t.input_validator.is_valid(&json!({})));
    for Knob(name, ..) in KNOBS {
        assert!(t.input_schema["properties"].get(*name).is_some(), "{name}");
    }
}

#[test]
fn a_missing_file_is_an_error_and_a_board_without_copper_layers_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let p = Params::from_args(&json!({})).unwrap_or_else(|_| panic!());
    assert!(audit(&dir.path().join("nope.kicad_pcb"), &p).is_err());
    let f = fx_text(
        "(kicad_pcb (version 20260206) (layers (25 \"Edge.Cuts\" user)))",
        None,
    );
    let r = run(&f);
    assert_eq!(r["status"], "not_run");
    assert!(r["reason"].as_str().unwrap().contains("no copper layers"));
}

// ─── Name rules, scores and the raster ───────────────────────────────────────

#[test]
fn name_rules_find_ground_power_and_critical_nets() {
    for g in ["GND", "/Power/AGND", "PGND", "GNDA", "VSS", "+0V", "0V"] {
        assert!(ground_by_name(g), "{g}");
        assert!(!power_by_name(g), "{g}");
    }
    for p in [
        "+3V3",
        "5V",
        "12V",
        "/sheet/+1V8",
        "VCC",
        "VDD_IO",
        "VBUS",
        "VIN",
    ] {
        assert!(power_by_name(p), "{p}");
    }
    for n in ["SDA", "N_CLEAN", "CLK_OK", "TX", "VREF_NEG", "GATE1"] {
        assert!(!ground_by_name(n), "{n}");
    }
    assert!(
        critical_by_name("/mcu/SPI_SCLK") && critical_by_name("USB_DP") && !critical_by_name("SDA")
    );
    assert!(glob("N_*", "N_CLEAN") && glob("A?C", "ABC") && !glob("N_*", "X_N"));
}

#[test]
fn score_curves_have_the_documented_shape() {
    assert_eq!(margin_score(0.0, 10.0), 1.0);
    assert_eq!(margin_score(5.0, 10.0), 1.0);
    assert!((margin_score(10.0, 10.0) - 0.5).abs() < 1e-12);
    assert!((margin_score(15.0, 10.0) - 0.25).abs() < 1e-12);
    assert_eq!(margin_score(20.0, 10.0), 0.0);
    assert_eq!(excess_score(1.5, 1.5), 1.0);
    assert!((excess_score(2.25, 1.5) - 0.5).abs() < 1e-12);
    assert_eq!(excess_score(9.0, 1.5), 0.0);
}

#[test]
fn the_raster_measures_widths_lengths_and_components() {
    use grid::{shortest_path, widest_path, Cover, Grid};
    // A 2 mm bar with a 0.4 mm neck in the middle, and an unconnected 1 mm dot.
    let mut g = Grid::new((0.0, 0.0, 40.0, 10.0), 0.05, vec!["F.Cu".into()]);
    g.add_path(0, &[(2.0, 5.0), (18.0, 5.0)], 1.0, Cover::Centre);
    g.add_path(0, &[(18.0, 5.0), (22.0, 5.0)], 0.2, Cover::Centre);
    g.add_path(0, &[(22.0, 5.0), (38.0, 5.0)], 1.0, Cover::Centre);
    g.add_disc(0, (20.0, 9.0), 0.5);
    let (_, count) = g.components();
    assert_eq!(count, 2);
    let dt = g.distance_transform();
    let from = g.cells_under(
        &konnect_sexp::board_geometry::Shape::disc((3.0, 5.0), 0.3),
        &[0],
    );
    let to = g.cells_under(
        &konnect_sexp::board_geometry::Shape::disc((37.0, 5.0), 0.3),
        &[0],
    );
    let p = shortest_path(&g, &from, &to, 0.5).unwrap();
    assert!((p.length_mm - 34.0).abs() < 0.6, "{}", p.length_mm);
    let w = widest_path(&g, &dt, &from, &to, &[1.0], &[]).unwrap();
    assert!((w.width_mm - 0.4).abs() < 0.12, "{}", w.width_mm);
    assert!((w.at.0 - 20.0).abs() < 2.4, "{:?}", w.at);
    // A ring with a bridged hole fills by the even-odd rule.
    let mut h = Grid::new((0.0, 0.0, 12.0, 12.0), 0.1, vec!["F.Cu".into()]);
    h.add_ring(
        0,
        &[
            (1.0, 1.0),
            (11.0, 1.0),
            (11.0, 11.0),
            (1.0, 11.0),
            (1.0, 6.0),
            (4.0, 4.0),
            (4.0, 8.0),
            (1.0, 6.0),
        ],
    );
    assert!(
        h.area_mm2() > 80.0 && h.area_mm2() < 100.0,
        "{}",
        h.area_mm2()
    );
}

// ─── Review follow-up: boards written by hand ────────────────────────────────

/// One pad of a hand-written part.
struct Pd {
    num: String,
    at: (f64, f64),
    net: String,
    size: (f64, f64),
    layer: &'static str,
}

fn pd(num: &str, dx: f64, dy: f64, net: &str) -> Pd {
    Pd {
        num: num.into(),
        at: (dx, dy),
        net: net.into(),
        size: (0.6, 0.6),
        layer: "F.Cu",
    }
}

impl Pd {
    fn size(mut self, w: f64, h: f64) -> Pd {
        self.size = (w, h);
        self
    }
    fn on(mut self, layer: &'static str) -> Pd {
        self.layer = layer;
        self
    }
}

/// A small `.kicad_pcb` written as text: parts, tracks, vias and zones, nothing else. The
/// six `pcbnew` fixtures cover the shapes the checks were built on; these boards isolate one
/// situation each.
struct Bd {
    layers: Vec<&'static str>,
    body: String,
    n: u32,
}

impl Bd {
    fn new(layers: &[&'static str]) -> Bd {
        Bd {
            layers: layers.to_vec(),
            body: String::new(),
            n: 0,
        }
    }

    fn uuid(&mut self) -> String {
        self.n += 1;
        format!("00000000-0000-4000-8000-{:012}", self.n)
    }

    fn part(&mut self, reference: &str, at: (f64, f64), pads: Vec<Pd>) -> &mut Bd {
        let (u, ur, uv) = (self.uuid(), self.uuid(), self.uuid());
        self.body.push_str(&format!(
            "(footprint \"\" (layer \"F.Cu\") (uuid \"{u}\") (at {} {})\n\
             (property \"Reference\" \"{reference}\" (at 0 0 0) (layer \"F.SilkS\") (uuid \"{ur}\") (effects (font (size 1 1))))\n\
             (property \"Value\" \"{reference}\" (at 0 0 0) (layer \"F.Fab\") (uuid \"{uv}\") (effects (font (size 1 1))))\n",
            at.0, at.1
        ));
        for p in pads {
            let pu = self.uuid();
            let layers = if p.layer == "*.Cu" {
                "\"*.Cu\" \"*.Mask\"".to_string()
            } else {
                format!("\"{}\"", p.layer)
            };
            let kind = if p.layer == "*.Cu" {
                "thru_hole circle"
            } else {
                "smd rect"
            };
            let drill = if p.layer == "*.Cu" { "(drill 0.5)" } else { "" };
            self.body.push_str(&format!(
                "(pad \"{}\" {kind} (at {} {}) (size {} {}) {drill} (layers {layers}) (net \"{}\") (uuid \"{pu}\"))\n",
                p.num, p.at.0, p.at.1, p.size.0, p.size.1, p.net
            ));
        }
        self.body.push_str(")\n");
        self
    }

    fn seg(&mut self, layer: &str, a: (f64, f64), b: (f64, f64), width: f64, net: &str) -> &mut Bd {
        let u = self.uuid();
        self.body.push_str(&format!(
            "(segment (start {} {}) (end {} {}) (width {width}) (layer \"{layer}\") (net \"{net}\") (uuid \"{u}\"))\n",
            a.0, a.1, b.0, b.1
        ));
        self
    }

    fn via(&mut self, at: (f64, f64), net: &str) -> &mut Bd {
        let u = self.uuid();
        let (first, last) = (self.layers[0], self.layers[self.layers.len() - 1]);
        self.body.push_str(&format!(
            "(via (at {} {}) (size 0.6) (drill 0.3) (layers \"{first}\" \"{last}\") (net \"{net}\") (uuid \"{u}\"))\n",
            at.0, at.1
        ));
        self
    }

    /// A rectangular zone; `filled` writes its saved fill, otherwise it is left unfilled.
    fn zone(&mut self, net: &str, layer: &str, r: (f64, f64, f64, f64), filled: bool) -> &mut Bd {
        let u = self.uuid();
        let pts = format!(
            "(xy {} {}) (xy {} {}) (xy {} {}) (xy {} {})",
            r.0, r.1, r.2, r.1, r.2, r.3, r.0, r.3
        );
        let fill = if filled {
            format!(
                "(fill yes (thermal_gap 0.5) (thermal_bridge_width 0.5))\n(filled_polygon (layer \"{layer}\") (pts {pts}))"
            )
        } else {
            "(fill (thermal_gap 0.5) (thermal_bridge_width 0.5))".to_string()
        };
        self.body.push_str(&format!(
            "(zone (net \"{net}\") (layer \"{layer}\") (uuid \"{u}\") (hatch edge 0.5) (connect_pads (clearance 0.3)) (min_thickness 0.2)\n{fill}\n(polygon (pts {pts})))\n"
        ));
        self
    }

    fn text(&self) -> String {
        let layers: String = self
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let n = match *l {
                    "F.Cu" => 0,
                    "B.Cu" => 2,
                    _ => 4 + 2 * (i as i32 - 1),
                };
                format!("({n} \"{l}\" signal)")
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "(kicad_pcb (version 20260206) (generator \"pcbnew\") (general (thickness 1.6)) (paper \"A4\")\n(layers {layers} (25 \"Edge.Cuts\" user))\n{}\n)\n",
            self.body
        )
    }

    fn fx(&self, intent: Option<Value>) -> Fx {
        fx_text(&self.text(), intent)
    }
}

const TWO: [&str; 2] = ["F.Cu", "B.Cu"];
const FOUR: [&str; 4] = ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"];

fn subjects<'a>(fs: &[&'a Value]) -> Vec<&'a str> {
    fs.iter().map(|f| f["subject"].as_str().unwrap()).collect()
}

/// A buck (U1, L1) whose output rail feeds a load IC (U2) and a capacitor, and a second load IC
/// (U3) on another rail.
fn buck_feeding_a_load() -> Bd {
    let mut b = Bd::new(&TWO);
    b.part(
        "U1",
        (5.0, 10.0),
        vec![pd("1", -0.5, 0.0, "SW"), pd("2", 0.5, 0.0, "GND")],
    );
    b.part(
        "L1",
        (10.0, 10.0),
        vec![pd("1", -0.5, 0.0, "SW"), pd("2", 0.5, 0.0, "+3V3")],
    );
    b.part(
        "U2",
        (20.0, 10.0),
        vec![pd("1", -0.5, 0.0, "+3V3"), pd("2", 0.5, 0.0, "GND")],
    );
    b.part(
        "C5",
        (20.0, 12.0),
        vec![pd("1", -0.5, 0.0, "+3V3"), pd("2", 0.5, 0.0, "GND")],
    );
    b.part(
        "U3",
        (30.0, 10.0),
        vec![pd("1", -0.5, 0.0, "+5V"), pd("2", 0.5, 0.0, "GND")],
    );
    b.part(
        "C6",
        (30.0, 12.0),
        vec![pd("1", -0.5, 0.0, "+5V"), pd("2", 0.5, 0.0, "GND")],
    );
    b.seg("F.Cu", (4.5, 10.0), (9.5, 10.0), 0.4, "SW");
    b.seg("F.Cu", (10.5, 10.0), (19.5, 10.0), 0.4, "+3V3");
    b.seg("F.Cu", (19.5, 10.0), (19.5, 12.0), 0.4, "+3V3");
    b.seg("F.Cu", (20.5, 10.0), (20.5, 12.0), 0.4, "GND");
    b.seg("F.Cu", (29.5, 10.0), (29.5, 12.0), 0.4, "+5V");
    b.seg("F.Cu", (30.5, 10.0), (30.5, 12.0), 0.4, "GND");
    b
}

#[test]
fn a_power_rail_behind_an_inductor_is_a_filter_not_a_switch_node() {
    // No design intent, only the default netclass: +3V3 is a power rail by its name alone.
    let r = run(&buck_feeding_a_load().fx(None));
    // The load IC's supply pin is still measured for decoupling.
    let pins = check(&r, "4.1")["metrics"]["pins"]
        .as_array()
        .unwrap()
        .clone();
    assert!(pins.iter().any(|p| p["pin"] == "U2.1"), "{pins:?}");
    assert_eq!(check(&r, "4.1")["metrics"]["supply_pins"], 2);
    // Only SW is a switch node: 8.1 asks about it alone, and 8.2 measures only its copper.
    let hot = check(&r, "8.1");
    assert!(hot["reason"].as_str().unwrap().contains("'SW'"), "{hot}");
    assert!(!hot["reason"].as_str().unwrap().contains("+3V3"), "{hot}");
    assert!(r["needs_human"]
        .as_array()
        .unwrap()
        .iter()
        .all(|n| { n["item"] != "8.1" || !n["why"].as_str().unwrap().contains("+3V3") }));
    let nodes = check(&r, "8.2")["metrics"]["switch_nodes"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(nodes.len(), 1, "{nodes:?}");
    assert_eq!(nodes[0]["switch_net"], "SW");
    assert!(f_for(&r, "8.2", "+3V3").is_empty());
}

#[test]
fn a_switch_node_the_caller_names_leaves_its_supply_pins_to_8_1_and_is_counted() {
    let r = run_with(
        &buck_feeding_a_load().fx(None),
        json!({ "switching_regulators": [{ "switch_net": "+3V3", "topology": "buck" }] }),
    );
    let c = check(&r, "4.1");
    assert_eq!(c["metrics"]["supply_pins"], 1, "{c}");
    let left = &c["metrics"]["switch_node_pins_left_to_8_1_8_2"];
    assert_eq!(left["count"], 1, "{left}");
    assert_eq!(left["pins"][0], "U2.1");
    // The one switch node 8.2 measures is the named one.
    let nodes = check(&r, "8.2")["metrics"]["switch_nodes"]
        .as_array()
        .unwrap()
        .clone();
    assert!(nodes.iter().any(|n| n["switch_net"] == "+3V3"), "{nodes:?}");
}

#[test]
fn stitching_vias_on_ground_do_not_count_against_5_1() {
    let mut b = Bd::new(&TWO);
    b.part(
        "J1",
        (5.0, 5.0),
        vec![pd("1", 0.0, 0.0, "GND"), pd("2", 10.0, 0.0, "GND")],
    );
    b.seg("F.Cu", (5.0, 5.0), (15.0, 5.0), 0.3, "GND");
    b.zone("GND", "B.Cu", (1.0, 1.0, 40.0, 20.0), true);
    for i in 0..10 {
        b.via((6.0 + i as f64 * 1.5, 8.0), "GND");
    }
    // A signal net that hops layers three times for one connection is still judged.
    b.part("S1", (5.0, 15.0), vec![pd("1", 0.0, 0.0, "SIG")]);
    b.part(
        "S2",
        (15.0, 15.0),
        vec![pd("1", 0.0, 0.0, "SIG").on("B.Cu")],
    );
    b.seg("F.Cu", (5.0, 15.0), (8.0, 15.0), 0.2, "SIG");
    b.via((8.0, 15.0), "SIG");
    b.seg("B.Cu", (8.0, 15.0), (10.0, 15.0), 0.2, "SIG");
    b.via((10.0, 15.0), "SIG");
    b.seg("F.Cu", (10.0, 15.0), (12.0, 15.0), 0.2, "SIG");
    b.via((12.0, 15.0), "SIG");
    b.seg("B.Cu", (12.0, 15.0), (15.0, 15.0), 0.2, "SIG");
    let r = run(&b.fx(None));
    let fs = findings(&r, "5.1");
    assert_eq!(subjects(&fs), vec!["SIG"], "{fs:?}");
    assert!(fs[0]["issue"].as_str().unwrap().contains("3 via(s)"));
    // The metric says which nets got no via ratio, and why.
    let listed = check(&r, "5.1")["metrics"]["nets_without_a_via_ratio"]["listed"].clone();
    assert!(
        listed.as_array().unwrap().iter().any(|n| n["net"] == "GND"),
        "{listed}"
    );
    // Ground's vias leave the score alone: only SIG is scored, and it scores badly.
    assert_eq!(score(&r, "5.1"), 0.5);
}

#[test]
fn stitching_vias_on_a_ground_net_without_a_pour_do_not_count_against_5_1() {
    // No zone at all: the role alone keeps ground's vias out of the via ratio.
    let mut b = Bd::new(&TWO);
    b.part(
        "J1",
        (5.0, 5.0),
        vec![pd("1", 0.0, 0.0, "GND"), pd("2", 10.0, 0.0, "GND")],
    );
    b.seg("F.Cu", (5.0, 5.0), (15.0, 5.0), 0.3, "GND");
    for i in 0..6 {
        b.via((6.0 + i as f64 * 1.5, 5.0), "GND");
    }
    let r = run(&b.fx(None));
    let fs = findings(&r, "5.1");
    assert!(fs.is_empty(), "{fs:?}");
    let listed = check(&r, "5.1")["metrics"]["nets_without_a_via_ratio"]["listed"].clone();
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["net"] == "GND"
                && n["why"].as_str().unwrap_or_default().contains("stitching")),
        "{listed}"
    );
}

fn parallel_pair_board(y: f64) -> Bd {
    let mut b = Bd::new(&TWO);
    b.part(
        "A1",
        (0.0, y),
        vec![pd("1", 0.0, 0.0, "SIG_A"), pd("2", 20.0, 0.0, "SIG_A")],
    );
    b.part(
        "B1",
        (0.0, y + 0.4),
        vec![pd("1", 0.0, 0.0, "SIG_B"), pd("2", 20.0, 0.0, "SIG_B")],
    );
    b.seg("F.Cu", (0.0, y), (20.0, y), 0.2, "SIG_A");
    b.seg("F.Cu", (0.0, y + 0.4), (20.0, y + 0.4), 0.2, "SIG_B");
    b
}

#[test]
fn a_parallel_pair_across_a_cell_boundary_is_still_reported() {
    // 2.9 and 3.3 fall either side of a 3 mm cell line; 1.9 and 2.3 do not.
    for y in [1.9, 2.9] {
        let r = run(&parallel_pair_board(y).fx(None));
        let fs = findings(&r, "5.2");
        assert_eq!(subjects(&fs), vec!["SIG_A / SIG_B"], "y = {y}: {fs:?}");
        assert!((19.0..21.0).contains(&fs[0]["measured"]["parallel_mm"].as_f64().unwrap()));
    }
}

#[test]
fn a_ground_island_held_only_by_a_dangling_via_is_floating() {
    let mut b = Bd::new(&FOUR);
    b.part(
        "J1",
        (5.0, 5.0),
        vec![pd("1", 0.0, 0.0, "GND"), pd("2", 5.0, 0.0, "GND")],
    );
    b.seg("F.Cu", (5.0, 5.0), (10.0, 5.0), 0.3, "GND");
    b.via((7.5, 5.0), "GND");
    b.zone("GND", "In1.Cu", (0.0, 0.0, 30.0, 20.0), true);
    // A pour on In2.Cu away from every other ground copper, with one via inside it.
    b.zone("GND", "In2.Cu", (40.0, 0.0, 45.0, 5.0), true);
    b.via((42.5, 2.5), "GND");
    let r = run(&b.fx(None));
    let fs = findings(&r, "3.3");
    assert_eq!(fs.len(), 1, "{fs:?}");
    assert_eq!(fs[0]["severity"], "warning");
    assert!(fs[0]["subject"].as_str().unwrap().contains("island"));
    assert!(fs[0]["measured"]["area_mm2"].as_f64().unwrap() > 20.0);
    assert_eq!(
        check(&r, "3.3")["metrics"]["compare"]["floating_ground_islands"],
        1
    );
    assert!(score(&r, "3.3") < 1.0);
    // The control: the same board without the island has nothing to report.
    let mut c = Bd::new(&FOUR);
    c.part(
        "J1",
        (5.0, 5.0),
        vec![pd("1", 0.0, 0.0, "GND"), pd("2", 5.0, 0.0, "GND")],
    );
    c.seg("F.Cu", (5.0, 5.0), (10.0, 5.0), 0.3, "GND");
    c.via((7.5, 5.0), "GND");
    c.zone("GND", "In1.Cu", (0.0, 0.0, 30.0, 20.0), true);
    let r = run(&c.fx(None));
    assert!(findings(&r, "3.3").is_empty(), "{:?}", findings(&r, "3.3"));
    assert_eq!(score(&r, "3.3"), 1.0);
}

/// Two 1 mm pads 10 mm apart on one net, joined by a 0.25 mm track that stops `end_x` along.
fn gap_board(end_x: f64) -> Bd {
    let mut b = Bd::new(&TWO);
    b.part(
        "P1",
        (0.0, 0.0),
        vec![pd("1", 0.0, 0.0, "SIG").size(1.0, 1.0)],
    );
    b.part(
        "P2",
        (10.0, 0.0),
        vec![pd("1", 0.0, 0.0, "SIG").size(1.0, 1.0)],
    );
    b.seg("F.Cu", (0.0, 0.0), (end_x, 0.0), 0.25, "SIG");
    b
}

#[test]
fn a_track_that_stops_short_of_its_pad_is_unrouted() {
    // Copper ends at end_x + 0.125; the far pad's edge is at 9.5. Gaps of 0.12 and 0.2 mm
    // joined on the grown raster; they are open for KiCad.
    for end_x in [9.255, 9.175] {
        let r = run(&gap_board(end_x).fx(None));
        assert_eq!(
            r["completion"]["unrouted_nets"], 1,
            "end {end_x}: {}",
            r["completion"]
        );
        assert_eq!(r["completion"]["complete"], false);
        assert_eq!(r["quality"]["routed_share"], 0.0);
    }
    // A track that reaches the pad is routed.
    let r = run(&gap_board(9.6).fx(None));
    assert_eq!(r["completion"]["unrouted_nets"], 0, "{}", r["completion"]);
    assert_eq!(r["completion"]["complete"], true);
}

#[test]
fn a_ground_pad_a_gap_away_from_its_copper_is_cut_off() {
    // Ground pads 10 mm apart; the track stops 0.12 mm short of the second one.
    let mut b = Bd::new(&TWO);
    b.part(
        "P1",
        (0.0, 0.0),
        vec![pd("1", 0.0, 0.0, "GND").size(1.0, 1.0)],
    );
    b.part(
        "P2",
        (10.0, 0.0),
        vec![pd("1", 0.0, 0.0, "GND").size(1.0, 1.0)],
    );
    b.seg("F.Cu", (0.0, 0.0), (9.255, 0.0), 0.25, "GND");
    let r = run(&b.fx(None));
    let errors: Vec<_> = findings(&r, "3.3")
        .into_iter()
        .filter(|f| f["severity"] == "error")
        .collect();
    assert_eq!(errors.len(), 1, "{:?}", findings(&r, "3.3"));
    assert!(errors[0]["issue"]
        .as_str()
        .unwrap()
        .contains("2 separate copper groups"));
    // Reaching the pad joins them.
    let mut c = Bd::new(&TWO);
    c.part(
        "P1",
        (0.0, 0.0),
        vec![pd("1", 0.0, 0.0, "GND").size(1.0, 1.0)],
    );
    c.part(
        "P2",
        (10.0, 0.0),
        vec![pd("1", 0.0, 0.0, "GND").size(1.0, 1.0)],
    );
    c.seg("F.Cu", (0.0, 0.0), (9.6, 0.0), 0.25, "GND");
    assert!(findings(&run(&c.fx(None)), "3.3").is_empty());
}

#[test]
fn a_net_with_two_current_problems_counts_once() {
    let mut b = Bd::new(&TWO);
    b.part(
        "J1",
        (0.0, 0.0),
        vec![pd("1", 0.0, 0.0, "P_NECK").size(2.0, 2.0)],
    );
    b.part(
        "J2",
        (20.0, 0.0),
        vec![pd("1", 0.0, 0.0, "P_NECK").size(2.0, 2.0)],
    );
    b.seg("F.Cu", (0.0, 0.0), (20.0, 0.0), 0.2, "P_NECK");
    b.via((10.0, 0.0), "P_NECK");
    b.part(
        "J3",
        (0.0, 10.0),
        vec![pd("1", 0.0, 0.0, "P_WIDE").size(2.0, 2.0)],
    );
    b.part(
        "J4",
        (20.0, 10.0),
        vec![pd("1", 0.0, 0.0, "P_WIDE").size(2.0, 2.0)],
    );
    b.seg("F.Cu", (0.0, 10.0), (20.0, 10.0), 2.0, "P_WIDE");
    let intent = json!({ "schema_version": 1, "currents": [
        { "net": "P_NECK", "max_current_a": 3.0 }, { "net": "P_WIDE", "max_current_a": 1.0 }] });
    let r = run(&b.fx(Some(intent)));
    let fs = findings(&r, "4.2");
    assert_eq!(fs.len(), 2, "two problems on one net: {fs:?}");
    assert!(fs.iter().all(|f| f["subject"] == "P_NECK"));
    let c = check(&r, "4.2");
    assert_eq!(c["metrics"]["nets_checked"], 2, "{c}");
    assert_eq!(c["metrics"]["nets_with_problems"], 1, "{c}");
    assert_eq!(score(&r, "4.2"), 0.5);
}

#[test]
fn a_track_over_a_solid_power_plane_is_referenced_not_split() {
    let mut b = Bd::new(&FOUR);
    b.part(
        "J1",
        (5.0, 5.0),
        vec![pd("1", 0.0, 0.0, "GND"), pd("2", 2.0, 0.0, "+3V3")],
    );
    b.part("A1", (5.0, 15.0), vec![pd("1", 0.0, 0.0, "CLK").on("B.Cu")]);
    b.part(
        "A2",
        (45.0, 15.0),
        vec![pd("1", 0.0, 0.0, "CLK").on("B.Cu")],
    );
    b.seg("B.Cu", (5.0, 15.0), (45.0, 15.0), 0.2, "CLK");
    b.zone("GND", "In1.Cu", (0.0, 0.0, 50.0, 30.0), true);
    b.zone("+3V3", "In2.Cu", (0.0, 0.0, 50.0, 30.0), true);
    let r = run_with(&b.fx(None), json!({ "critical_nets": ["CLK"] }));
    let c = check(&r, "3.1");
    let clk = c["metrics"]["nets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["net"] == "CLK")
        .unwrap();
    assert!(clk["coverage"].as_f64().unwrap() > 0.98, "{clk}");
    assert_eq!(clk["layers"][0]["reference_net"], "+3V3");
    assert_eq!(clk["layers"][0]["reference_kind"], "power");
    assert!(findings(&r, "3.1").is_empty(), "{:?}", findings(&r, "3.1"));
    assert_eq!(score(&r, "3.1"), c["score"].as_f64().unwrap());
    assert!(score(&r, "3.1") > 0.98);
}

#[test]
fn a_split_in_a_power_reference_plane_is_still_a_split() {
    // The power plane is half +3V3 and half +5V: a track over both crosses a split.
    let mut b = Bd::new(&FOUR);
    b.part(
        "J1",
        (5.0, 5.0),
        vec![pd("1", 0.0, 0.0, "+3V3"), pd("2", 2.0, 0.0, "+5V")],
    );
    b.part("A1", (5.0, 15.0), vec![pd("1", 0.0, 0.0, "CLK").on("B.Cu")]);
    b.part(
        "A2",
        (45.0, 15.0),
        vec![pd("1", 0.0, 0.0, "CLK").on("B.Cu")],
    );
    b.seg("B.Cu", (5.0, 15.0), (45.0, 15.0), 0.2, "CLK");
    b.zone("+3V3", "In2.Cu", (0.0, 0.0, 30.0, 30.0), true);
    b.zone("+5V", "In2.Cu", (30.5, 0.0, 50.0, 30.0), true);
    let r = run_with(&b.fx(None), json!({ "critical_nets": ["CLK"] }));
    let fs = findings(&r, "3.1");
    assert_eq!(fs.len(), 1, "{fs:?}");
    assert!(fs[0]["issue"].as_str().unwrap().contains("'+5V' copper"));
}

#[test]
fn zones_without_a_saved_fill_leave_ground_and_completion_unjudged() {
    let r = run(&fx_text(&without_fills("plane_split.kicad_pcb"), None));
    // 3.3 does not call the pads of an unfilled pour "cut off".
    let c = check(&r, "3.3");
    assert_eq!(c["status"], "not_run", "{c}");
    assert!(
        c["reason"].as_str().unwrap().contains("no saved fill"),
        "{c}"
    );
    assert!(findings(&r, "3.3").iter().all(|f| f["severity"] != "error"));
    // The gate does not count those nets as unrouted, and says it did not judge them.
    let comp = &r["completion"];
    assert_eq!(comp["unrouted_nets"], 0, "{comp}");
    assert!(comp["nets_not_judged"].as_u64().unwrap() >= 1, "{comp}");
    assert_eq!(comp["status"], "partial");
    assert!(comp["complete"].is_null());
    assert!(comp["reason"].as_str().unwrap().contains("no saved fill"));
    assert_eq!(r["quality"]["routed_share"], 1.0);
    assert!(r["summary"].as_str().unwrap().contains("NOT FULLY JUDGED"));
}

#[test]
fn nets_not_judged_for_an_unfilled_zone_are_left_out_of_the_routed_share() {
    // GND joins its pads only through a zone that was never filled; SIG is really open.
    let mut b = gap_board(9.255);
    b.part(
        "G1",
        (0.0, 10.0),
        vec![pd("1", 0.0, 0.0, "GND").size(1.0, 1.0)],
    );
    b.part(
        "G2",
        (10.0, 10.0),
        vec![pd("1", 0.0, 0.0, "GND").size(1.0, 1.0)],
    );
    b.zone("GND", "B.Cu", (-2.0, 8.0, 12.0, 12.0), false);
    let r = run(&b.fx(None));
    let comp = &r["completion"];
    assert_eq!(comp["unrouted_nets"], 1, "{comp}");
    assert_eq!(comp["nets_not_judged"], 1, "{comp}");
    assert_eq!(comp["complete"], false);
    // One judged net and it is open: the share is 0, not 1 of 2.
    assert_eq!(r["quality"]["routed_share"], 0.0);
}

#[test]
fn nets_left_out_of_5_1_are_named_and_a_short_tree_is_still_judged() {
    let mut b = Bd::new(&TWO);
    // Two pads 0.8 mm apart joined by 8.8 mm of copper: judged against a 1 mm floor.
    b.part("P1", (10.0, 10.0), vec![pd("1", 0.0, 0.0, "SHORT")]);
    b.part("P2", (10.8, 10.0), vec![pd("1", 0.0, 0.0, "SHORT")]);
    b.seg("F.Cu", (10.0, 10.0), (10.0, 14.0), 0.2, "SHORT");
    b.seg("F.Cu", (10.0, 14.0), (10.8, 14.0), 0.2, "SHORT");
    b.seg("F.Cu", (10.8, 14.0), (10.8, 10.0), 0.2, "SHORT");
    // A straight net as the control.
    b.part("P3", (20.0, 10.0), vec![pd("1", 0.0, 0.0, "LONG")]);
    b.part("P4", (30.0, 10.0), vec![pd("1", 0.0, 0.0, "LONG")]);
    b.seg("F.Cu", (20.0, 10.0), (30.0, 10.0), 0.2, "LONG");
    // Ground carried by a pour: no detour ratio and no via ratio, so it is named, not scored.
    b.part("G1", (10.0, 20.0), vec![pd("1", 0.0, 0.0, "GND")]);
    b.part("G2", (20.0, 20.0), vec![pd("1", 0.0, 0.0, "GND")]);
    b.seg("F.Cu", (10.0, 20.0), (12.0, 20.0), 0.3, "GND");
    b.zone("GND", "F.Cu", (5.0, 18.0, 25.0, 22.0), true);
    let r = run(&b.fx(None));
    let fs = findings(&r, "5.1");
    assert_eq!(subjects(&fs), vec!["SHORT"], "{fs:?}");
    assert!(fs[0]["measured"]["detour_ratio"].as_f64().unwrap() > 5.0);
    let m = &check(&r, "5.1")["metrics"];
    assert_eq!(
        m["nets_without_a_detour_ratio"]["listed"][0]["net"], "GND",
        "{m}"
    );
    assert_eq!(m["nets_without_a_detour_ratio"]["count"], 1);
    assert_eq!(m["nets_not_scored"]["listed"][0]["net"], "GND", "{m}");
    assert_eq!(m["nets_without_a_via_ratio"]["listed"][0]["net"], "GND");
}

/// A source pad and `loads` load pads (one part each) on a 2 mm bar along `y`, net `net`.
fn rail(b: &mut Bd, net: &str, y: f64, loads: usize) {
    b.part("J", (0.0, y), vec![pd("1", 0.0, 0.0, net).size(3.0, 3.0)]);
    for k in 0..loads {
        b.part(
            &format!("R{k}"),
            (6.0 + 2.0 * k as f64, y),
            vec![pd("1", 0.0, 0.0, net)],
        );
    }
    b.seg(
        "F.Cu",
        (0.0, y),
        (6.0 + 2.0 * (loads as f64 - 1.0), y),
        2.0,
        net,
    );
}

#[test]
fn loads_beyond_the_per_net_limit_are_said_and_the_farthest_is_measured() {
    let mut b = Bd::new(&TWO);
    rail(&mut b, "VZ", 0.0, 14);
    let intent =
        json!({ "schema_version": 1, "currents": [{ "net": "VZ", "max_current_a": 3.0 }] });
    let r = run(&b.fx(Some(intent)));
    let c = check(&r, "4.3");
    assert_eq!(c["status"], "partial", "{c}");
    assert!(c["reason"].as_str().unwrap().contains("more loads"), "{c}");
    let left = &c["metrics"]["paths_not_measured"]["loads_beyond_the_per_net_limit"];
    assert_eq!(left[0]["net"], "VZ");
    assert_eq!(left[0]["loads_not_measured"], 2);
    // The loads kept run from the nearest to the farthest (R13 is 28 mm along).
    let paths = c["metrics"]["paths"].as_array().unwrap();
    assert!(
        paths
            .iter()
            .any(|p| p["path"].as_str().unwrap().ends_with("R13.1")),
        "{paths:?}"
    );
    assert!(
        paths
            .iter()
            .any(|p| p["path"].as_str().unwrap().ends_with("R0.1")),
        "{paths:?}"
    );
}

#[test]
fn a_ground_return_with_no_ground_pad_is_named_when_3_4_does_not_run() {
    let mut b = Bd::new(&TWO);
    rail(&mut b, "VZ", 0.0, 3);
    let intent =
        json!({ "schema_version": 1, "currents": [{ "net": "VZ", "max_current_a": 1.0 }] });
    let r = run(&b.fx(Some(intent)));
    let c = check(&r, "3.4");
    assert_eq!(c["status"], "not_run", "{c}");
    assert!(
        c["reason"].as_str().unwrap().contains("had no ground pad"),
        "{c}"
    );
}

#[test]
fn paths_past_the_global_cap_are_counted_by_net() {
    let mut b = Bd::new(&TWO);
    let mut currents = Vec::new();
    for i in 0..41 {
        let net = format!("VN{i:02}");
        rail(&mut b, &net, 5.0 * i as f64, 2);
        currents.push(json!({ "net": net, "max_current_a": 1.0 }));
    }
    let r = run(&b.fx(Some(json!({ "schema_version": 1, "currents": currents }))));
    let c = check(&r, "4.3");
    assert_eq!(c["status"], "partial", "{c}");
    assert!(c["reason"].as_str().unwrap().contains("cap of 80"), "{c}");
    let capped = c["metrics"]["paths_not_measured"]["paths_past_the_global_cap_of_80"]
        .as_array()
        .unwrap();
    assert_eq!(capped.len(), 1, "{capped:?}");
    assert_eq!(capped[0]["net"], "VN40");
    assert_eq!(capped[0]["paths_not_measured"], 2);
}

#[test]
fn an_explicit_netclass_assignment_and_a_pattern_both_apply_as_in_kicad() {
    // KiCad joins every assigned and every matching pattern class into one effective class.
    let pro = json!({ "net_settings": { "classes": [],
        "netclass_patterns": [{ "netclass": "Analog", "pattern": "*" }],
        "netclass_assignments": { "N_STUB": ["Power"] } } });
    let r = run(&fx("routing.kicad_pcb", None, Some(pro)));
    assert!(
        r["net_roles"]["power"]["nets"].get("N_STUB").is_some(),
        "{}",
        r["net_roles"]
    );
    // The same net with two matching patterns, Analog first.
    let pro = json!({ "net_settings": { "classes": [], "netclass_patterns": [
        { "netclass": "Analog", "pattern": "N_STUB" },
        { "netclass": "Power", "pattern": "N_*" }] } });
    let r = run(&fx("routing.kicad_pcb", None, Some(pro)));
    let why = r["net_roles"]["power"]["nets"]["N_STUB"].as_str().unwrap();
    assert!(why.contains("Analog") && why.contains("Power"), "{why}");
}

fn without_vias_of(name: &str, net: &str) -> String {
    let text = std::fs::read_to_string(fixture_path(name)).unwrap();
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(i) = rest.find("\n\t(via") {
        out.push_str(&rest[..i]);
        let mut depth = 0;
        let mut end = i + 1;
        for (k, c) in rest[i + 1..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1 + k + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        let block = &rest[i..end];
        if !block.contains(&format!("(net \"{net}\")")) {
            out.push_str(block);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

#[test]
fn no_stitching_candidate_is_null_not_a_million_millimetres() {
    let r = run(&fx_text(
        &without_vias_of("layer_change.kicad_pcb", "GND"),
        None,
    ));
    let c = check(&r, "3.2");
    assert!(!findings(&r, "3.2").is_empty(), "{c}");
    assert!(c["metrics"]["worst_nearest_mm"].is_null(), "{c}");
    assert!(c["metrics"]["worst_nearest_note"]
        .as_str()
        .unwrap()
        .contains("no distance to report"));
    assert!(
        c["metrics"]["unstitched_with_no_candidate_on_board"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(findings(&r, "3.2")
        .iter()
        .all(|f| f["measured"]["nearest_stitch_mm"].is_null()));
}

#[test]
fn findings_name_the_items_they_are_about() {
    let r = run(&fx("routing.kicad_pcb", None, None));
    // Acute corners: the tracks that meet at the corner.
    let acute = findings(&r, "5.3")
        .into_iter()
        .find(|f| f["issue"].as_str().unwrap().starts_with("Acid trap"))
        .unwrap();
    assert!(!acute["uuids"].as_array().unwrap().is_empty(), "{acute}");
    // A pair's length mismatch: tracks of both nets.
    let skew = f_for(&r, "5.4", "length");
    assert!(!skew.is_empty());
    assert!(skew
        .iter()
        .all(|f| !f["uuids"].as_array().unwrap().is_empty()));
    // Ground cut off from its pads: the pads that are cut off.
    let text = without_vias_of("plane_split.kicad_pcb", "GND");
    let r = run(&fx_text(&text, None));
    let cut: Vec<_> = findings(&r, "3.3")
        .into_iter()
        .filter(|f| f["severity"] == "error")
        .collect();
    assert_eq!(cut.len(), 1, "{cut:?}");
    assert!(
        !cut[0]["uuids"].as_array().unwrap().is_empty(),
        "{}",
        cut[0]
    );
}

// Build the public layer-change fixture directly so cancellation exercises the analysis,
// without running the unrelated audit checks first.
fn with_stitching_model(test: impl FnOnce(&mut Model)) {
    let tree =
        parse_sexp(&std::fs::read_to_string(fixture_path("layer_change.kicad_pcb")).unwrap())
            .unwrap();
    let geometry = BoardGeometry::from_tree(&tree);
    let params = Params::from_args(&json!({})).unwrap_or_else(|_| panic!("params"));
    let (fills, unfilled) = read_fills(&tree, &geometry);
    let mut tracks: HashMap<String, Vec<usize>> = HashMap::new();
    let mut vias: HashMap<String, Vec<usize>> = HashMap::new();
    let mut pads: HashMap<String, Vec<PadIx>> = HashMap::new();
    for (i, e) in geometry.others.iter().enumerate() {
        if let Some(net) = &e.net {
            match e.kind {
                Kind::Track => tracks.entry(net.clone()).or_default().push(i),
                Kind::Via => vias.entry(net.clone()).or_default().push(i),
                _ => {}
            }
        }
    }
    for (fi, f) in geometry.footprints.iter().enumerate() {
        for (pi, p) in f.pads.iter().enumerate() {
            if let Some(net) = &p.net {
                pads.entry(net.clone()).or_default().push((fi, pi));
            }
        }
    }
    let mut roles = Roles::default();
    roles.ground.insert("GND".into(), "fixture".into());
    for net in tracks.keys().filter(|n| n.as_str() != "GND") {
        roles.critical.insert(net.clone(), "fixture".into());
    }
    let intent = serde_json::from_value(json!({ "schema_version": SCHEMA_VERSION })).unwrap();
    let mut model = Model {
        geo: &geometry,
        layers: copper_layers(&tree),
        fills,
        unfilled,
        roles,
        params: &params,
        copper: trace_current::copper_for(&tree, &intent),
        tracks,
        vias,
        pads,
        conn: std::cell::OnceCell::new(),
    };
    test(&mut model);
}

#[test]
fn planner_stitching_matches_existing_layer_change_audit() {
    with_stitching_model(|m| {
        let (checks, _) = planes::run(m);
        let expected = checks.iter().find(|c| c.id == "3.2").unwrap();
        let mut charged = 0;
        let actual = planes::planner_stitching(m, &mut |n| {
            charged += n;
            Ok(())
        })
        .unwrap();
        assert_eq!(actual.metrics["layer_changes"], 4);
        assert_eq!(actual.metrics["needing_stitch"], 4);
        assert_eq!(actual.metrics["stitched"], 2);
        assert_eq!(actual.score, Some(0.5));
        assert_eq!(actual.to_json(), expected.to_json());
        assert_eq!(
            actual
                .findings
                .iter()
                .map(Finding::to_json)
                .collect::<Vec<_>>(),
            expected
                .findings
                .iter()
                .map(Finding::to_json)
                .collect::<Vec<_>>()
        );
        assert!(charged > 100, "sample and ring work must be charged");
    });
}

#[test]
fn planner_stitching_propagates_cancellation_at_every_charge() {
    with_stitching_model(|m| {
        let mut checkpoints = 0;
        planes::planner_stitching(m, &mut |_| {
            checkpoints += 1;
            Ok(())
        })
        .unwrap();
        assert!(
            checkpoints > 100,
            "must check inside the reference analysis"
        );
        for stop in 1..=checkpoints {
            let mut calls = 0;
            let result = planes::planner_stitching(m, &mut |_| {
                calls += 1;
                if calls == stop {
                    anyhow::bail!("reference analysis cancelled")
                }
                Ok(())
            });
            let error = result.err().expect("cancellation must abort analysis");
            assert_eq!(error.to_string(), "reference analysis cancelled");
            assert_eq!(calls, stop, "must stop immediately at charge {stop}");
        }
    });
}

#[test]
fn planner_stitching_skips_reference_samples_without_signal_vias() {
    with_stitching_model(|m| {
        m.vias.retain(|net, _| net == "GND");
        // Empty entries also mean no actual signal vias.
        for net in m.roles.critical.keys() {
            m.vias.insert(net.clone(), Vec::new());
        }
        let mut charges = 0;
        let result = planes::planner_stitching(m, &mut |n| {
            charges += n;
            Ok(())
        })
        .unwrap();
        assert_eq!(result.status, "not_run");
        assert!(
            charges <= 20,
            "no sample, fill or ground raster work: {charges}"
        );
    });
}

#[test]
fn planner_stitching_charges_ring_edges_before_point_containment() {
    with_stitching_model(|m| {
        for fill in &mut m.fills {
            for ring in &mut fill.rings {
                // Keep each ring's original bounding box, but make edge work expensive.
                ring.pts = ring.pts.iter().copied().cycle().take(10_000).collect();
            }
        }
        let result = planes::planner_stitching(m, &mut |n| {
            if n >= 10_000 {
                anyhow::bail!("ring exceeds work budget")
            }
            Ok(())
        });
        let error = result
            .err()
            .expect("ring scan must be charged before it runs");
        assert_eq!(error.to_string(), "ring exceeds work budget");
    });
}

// ─── S7b: corners read with the corrected definition ─────────────────────────

const JN: (f64, f64) = (20.0, 20.0);

/// A straight track from one point to another.
type Track = ((f64, f64), (f64, f64));

fn leg_at(degrees: f64, length: f64) -> (f64, f64) {
    let r = degrees.to_radians();
    (
        ((JN.0 + length * r.cos()) * 1e6).round() / 1e6,
        ((JN.1 + length * r.sin()) * 1e6).round() / 1e6,
    )
}

/// A board holding only the given SIG tracks on F.Cu, 0.2 mm wide.
fn corner_board(tracks: &[Track]) -> Fx {
    let mut segs = String::new();
    for (i, (a, b)) in tracks.iter().enumerate() {
        segs += &format!(
            "  (segment (start {} {}) (end {} {}) (width 0.2) (layer \"F.Cu\") (net \"SIG\") (uuid \"t{i}\"))\n",
            a.0, a.1, b.0, b.1
        );
    }
    fx_text(
        &format!(
            r#"(kicad_pcb (version 20260206) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 60 40) (stroke (width 0.05) (type solid)) (fill no) (layer "Edge.Cuts") (uuid "e1"))
{segs})"#
        ),
        None,
    )
}

fn corner_metrics(tracks: &[Track]) -> Value {
    let r = run(&corner_board(tracks));
    check(&r, "5.3")["metrics"].clone()
}

fn corner_legs(angles: &[f64]) -> Vec<Track> {
    angles.iter().map(|a| (JN, leg_at(*a, 3.0))).collect()
}

/// A 0.1-um track at the junction: its two ends share one key.
fn corner_stub() -> Track {
    (JN, (JN.0, JN.1 + 0.0001))
}

/// The audit read four directions here and no bend; the real legs make a
/// right angle, and it is counted as one. The short track is counted too.
#[test]
fn a_short_track_at_a_right_angle_bend_counts_as_that_right_angle() {
    let mut t = corner_legs(&[180., 90.]);
    t.push(corner_stub());
    let m = corner_metrics(&t);
    assert_eq!(m["acute_corners"], 0, "{m}");
    assert_eq!(m["right_angle_corners"], 1, "{m}");
    assert_eq!(m["acute_found_by_this_scan"], 0, "{m}");
    assert_eq!(m["short_tracks"], 1, "{m}");
    assert_eq!(m["junctions_with_short_tracks"], 1, "{m}");
}

#[test]
fn the_corner_scan_reads_the_real_legs_of_a_junction() {
    let mut bend = corner_legs(&[180., 45.]);
    bend.push(corner_stub());
    let m = corner_metrics(&bend);
    assert_eq!(
        (m["acute_corners"].as_u64(), m["bends"].as_u64()),
        (Some(0), Some(1)),
        "{m}"
    );
    assert_eq!(m["right_angle_corners"], 0, "{m}");
    // Three real legs are a tee, not a bend.
    let j1 = (JN.0 + 0.002, JN.1 + 0.002);
    let tee = vec![
        (JN, leg_at(90., 3.)),
        (JN, leg_at(270., 3.)),
        (j1, (j1.0 + 3., j1.1)),
        (JN, j1),
    ];
    let m = corner_metrics(&tee);
    assert_eq!(
        (m["acute_corners"].as_u64(), m["bends"].as_u64()),
        (Some(0), Some(0)),
        "{m}"
    );
    assert_eq!(m["junctions_with_short_tracks"], 1, "{m}");
}

/// At the line both corner readings agree: 88.99 degrees is acute for the
/// acid-trap check and for the scan, 89.5 degrees is a right angle for both.
#[test]
fn the_two_corner_readings_agree_at_eighty_nine_degrees() {
    let m = corner_metrics(&corner_legs(&[0., 88.99]));
    assert_eq!(m["acute_corners"], 1, "{m}");
    assert_eq!(m["acute_found_by_this_scan"], 1, "{m}");
    assert_eq!(m["right_angle_corners"], 0, "{m}");
    let m = corner_metrics(&corner_legs(&[0., 89.5]));
    assert_eq!(m["acute_corners"], 0, "{m}");
    assert_eq!(m["acute_found_by_this_scan"], 0, "{m}");
    assert_eq!(m["right_angle_corners"], 1, "{m}");
}

/// A right-angle bend whose 9-um track reaches within 0.01 mm of a pad of the
/// net while the bend itself stays 15 um clear: the bend is counted. Only a
/// junction whose legs all end near that copper is left out.
#[test]
fn a_bend_is_left_out_only_where_its_legs_meet_near_a_pad() {
    let mut t = corner_legs(&[180., 270.]);
    t.push((JN, (JN.0, JN.1 + 0.009)));
    let pad = format!(
        "(footprint \"Fixture:P\" (layer \"F.Cu\") (uuid \"fp\") (at {} {} 0) (property \"Reference\" \"P1\" (at 0 0 0) (layer \"F.SilkS\") (uuid \"fp-r\") (effects (font (size 1 1)))) (pad \"1\" smd circle (at 0 0 0) (size 0.8 0.8) (layers \"F.Cu\" \"F.Mask\") (net \"SIG\") (uuid \"pp\")))",
        JN.0,
        JN.1 + 0.415
    );
    let mut segs = String::new();
    for (i, (a, b)) in t.iter().enumerate() {
        segs += &format!(
            "  (segment (start {} {}) (end {} {}) (width 0.2) (layer \"F.Cu\") (net \"SIG\") (uuid \"t{i}\"))\n",
            a.0, a.1, b.0, b.1
        );
    }
    let text = format!(
        r#"(kicad_pcb (version 20260206) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 60 40) (stroke (width 0.05) (type solid)) (fill no) (layer "Edge.Cuts") (uuid "e1"))
{segs}{pad})"#
    );
    let r = run(&fx_text(&text, None));
    let m = &check(&r, "5.3")["metrics"];
    assert_eq!(m["right_angle_corners"], 1, "{m}");
}

/// A right-angle bend whose legs end at two keys 9 um apart, joined by a
/// short track: leg A runs left from the first key, leg B up from the second.
/// A pad of the net centred straight above has its edge 5 um from the second
/// key and 14 um from the first: one leg end near the pad, so the bend is
/// counted. With the edge at 0 and 9 um, both are near and it is left out.
#[test]
fn a_bend_is_left_out_only_when_every_leg_end_is_near_a_pad() {
    let j1 = (JN.0, JN.1 + 0.009);
    let t = [(JN, leg_at(180., 3.)), (JN, j1), (j1, (j1.0, j1.1 + 3.))];
    let bend = |gap: f64| {
        let pad = format!(
            "(footprint \"Fixture:P\" (layer \"F.Cu\") (uuid \"fp\") (at {} {} 0) (property \"Reference\" \"P1\" (at 0 0 0) (layer \"F.SilkS\") (uuid \"fp-r\") (effects (font (size 1 1)))) (pad \"1\" smd circle (at 0 0 0) (size 0.8 0.8) (layers \"F.Cu\" \"F.Mask\") (net \"SIG\") (uuid \"pp\")))",
            j1.0,
            j1.1 + 0.4 + gap
        );
        let mut segs = String::new();
        for (i, (a, b)) in t.iter().enumerate() {
            segs += &format!(
                "  (segment (start {} {}) (end {} {}) (width 0.2) (layer \"F.Cu\") (net \"SIG\") (uuid \"t{i}\"))\n",
                a.0, a.1, b.0, b.1
            );
        }
        let text = format!(
            r#"(kicad_pcb (version 20260206) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 60 40) (stroke (width 0.05) (type solid)) (fill no) (layer "Edge.Cuts") (uuid "e1"))
{segs}{pad})"#
        );
        let r = run(&fx_text(&text, None));
        check(&r, "5.3")["metrics"]["right_angle_corners"].clone()
    };
    assert_eq!(bend(0.005), 1);
    assert_eq!(bend(0.0), 0);
}

/// The scan at a via where the router snapped a wire end to the centre and
/// left a 2.8-um piece: a right-angle bend and a 45-degree V there lie inside
/// the via and are left out; without the via each is counted.
#[test]
fn a_corner_at_a_via_centre_is_left_out_with_its_snapped_piece() {
    let q = (JN.0 + 0.0024, JN.1 + 0.0015);
    for (angle, metric) in [(90f64, "right_angle_corners"), (135f64, "acute_corners")] {
        let r = angle.to_radians();
        let t = [
            (JN, leg_at(180., 3.)),
            (q, JN),
            (q, (q.0 + 3. * r.cos(), q.1 + 3. * r.sin())),
        ];
        let mut segs = String::new();
        for (i, (a, b)) in t.iter().enumerate() {
            segs += &format!(
                "  (segment (start {} {}) (end {} {}) (width 0.2) (layer \"F.Cu\") (net \"SIG\") (uuid \"t{i}\"))\n",
                a.0, a.1, b.0, b.1
            );
        }
        let board = |via: &str| {
            format!(
                r#"(kicad_pcb (version 20260206) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 60 40) (stroke (width 0.05) (type solid)) (fill no) (layer "Edge.Cuts") (uuid "e1"))
{segs}{via})"#
            )
        };
        let via = format!(
            "  (via (at {} {}) (size 0.6) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") (net \"SIG\") (uuid \"v1\"))\n",
            JN.0, JN.1
        );
        let inside = run(&fx_text(&board(&via), None));
        let m = &check(&inside, "5.3")["metrics"];
        assert_eq!(m[metric], 0, "{angle} degrees at the via: {m}");
        assert_eq!(m["junctions_with_short_tracks"], 1, "{m}");
        let open = run(&fx_text(&board(""), None));
        let m = &check(&open, "5.3")["metrics"];
        assert_eq!(m[metric], 1, "{angle} degrees without the via: {m}");
    }
}

/// The router's jog at a 135-degree bend: it turns 90 degrees, runs a short
/// piece and turns 45 degrees back. Under 10 um the piece belongs to the
/// junction and the bend reads as the 135 degrees it is; from 10 um the piece
/// is a leg and its first end reads as a right angle. Neither is acute.
#[test]
fn a_jog_at_a_135_degree_bend_is_a_right_angle_only_from_ten_micrometres() {
    for (length, right, bends) in [(0.00999, 0, 1), (0.01001, 1, 2)] {
        let q = (JN.0, JN.1 + length);
        let r = 45f64.to_radians();
        let t = [
            (leg_at(180., 3.), JN),
            (JN, q),
            (q, (q.0 + 3. * r.cos(), q.1 + 3. * r.sin())),
        ];
        let m = corner_metrics(&t);
        assert_eq!(m["acute_corners"], 0, "jog of {length} mm: {m}");
        assert_eq!(m["right_angle_corners"], right, "jog of {length} mm: {m}");
        assert_eq!(m["bends"], bends, "jog of {length} mm: {m}");
        // Two junctions from 10 um, neither holding a short track.
        assert_eq!(m["junctions_with_short_tracks"], 1 - right, "{m}");
    }
}

/// An acute finding names the tracks that meet at its junction, at every key:
/// a real 45-degree V whose legs end 9 um apart, joined by a short piece. A
/// track of the same net and layer elsewhere on the board is not named.
#[test]
fn an_acute_finding_names_the_tracks_at_every_key_of_its_junction() {
    let j1 = (JN.0 + 0.009, JN.1);
    let t = [
        (JN, leg_at(180., 3.)),
        (JN, j1),
        (j1, (j1.0 - 2.12132, j1.1 + 2.12132)),
        ((40., 30.), (43., 30.)),
    ];
    let r = run(&corner_board(&t));
    let fs: Vec<_> = findings(&r, "5.3")
        .into_iter()
        .filter(|f| f["severity"] == "warning")
        .collect();
    assert_eq!(fs.len(), 1, "{fs:?}");
    assert_eq!(fs[0]["uuids"], json!(["t0", "t1", "t2"]), "{fs:?}");
    // The stated threshold is the rule applied: acute below 89 degrees.
    assert_eq!(
        fs[0]["threshold"]["minimum_interior_angle_deg"], 89.0,
        "{fs:?}"
    );
}

/// The 5.3 score from the corrected counts: an acute corner costs a whole
/// bend, a right angle three tenths, out of every bend. Two right-angle
/// bends of a signal net, one of the ground net and one acute V: 4 bends,
/// 3 right angles, 1 acute corner, so 1 - (1 + 0.9) / 4. Right-angle
/// findings name signal nets only.
#[test]
fn the_corner_score_weighs_the_corrected_counts() {
    let bend = |at: (f64, f64), b: f64, net: &str, i: usize| {
        let r = b.to_radians();
        format!(
            "  (segment (start {x} {y}) (end {} {y}) (width 0.2) (layer \"F.Cu\") (net \"{net}\") (uuid \"a{i}\"))\n  (segment (start {x} {y}) (end {} {}) (width 0.2) (layer \"F.Cu\") (net \"{net}\") (uuid \"b{i}\"))\n",
            at.0 - 3.,
            ((at.0 + 3. * r.cos()) * 1e6).round() / 1e6,
            ((at.1 + 3. * r.sin()) * 1e6).round() / 1e6,
            x = at.0,
            y = at.1,
        )
    };
    let segs = bend((10., 10.), 90., "SIG", 0)
        + &bend((10., 25.), 90., "SIG", 1)
        + &bend((30., 10.), 90., "GND", 2)
        + &bend((30., 25.), 135., "SIG", 3);
    let text = format!(
        r#"(kicad_pcb (version 20260206) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 60 40) (stroke (width 0.05) (type solid)) (fill no) (layer "Edge.Cuts") (uuid "e1"))
{segs})"#
    );
    let r = run(&fx_text(&text, None));
    let m = &check(&r, "5.3")["metrics"];
    assert_eq!(
        (
            m["bends"].as_u64(),
            m["right_angle_corners"].as_u64(),
            m["acute_corners"].as_u64()
        ),
        (Some(4), Some(3), Some(1)),
        "{m}"
    );
    assert!(
        (score(&r, "5.3") - (1. - 1.9 / 4.)).abs() < 1e-9,
        "{}",
        score(&r, "5.3")
    );
    let right: Vec<_> = findings(&r, "5.3")
        .into_iter()
        .filter(|f| f["severity"] == "info")
        .map(|f| f["subject"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(right, ["SIG", "SIG"]);
}

// ─── S7b review M1: a run of short pieces is real copper beyond 20 um ──────

fn on_nm(p: (f64, f64)) -> (f64, f64) {
    ((p.0 * 1e6).round() / 1e6, (p.1 * 1e6).round() / 1e6)
}

/// A 90-degree fillet of 0.25 mm radius drawn in 1-degree pieces between two
/// straight legs is a rounded corner: neither acute nor a right angle.
#[test]
fn a_fillet_drawn_as_short_pieces_is_neither_acute_nor_a_right_angle() {
    let r = 0.25;
    let c = (JN.0, JN.1 + r);
    let p = |deg: f64| {
        let t = deg.to_radians();
        on_nm((c.0 + r * t.sin(), c.1 - r * t.cos()))
    };
    let mut t = vec![(JN, leg_at(180., 3.))];
    t.extend((0..90).map(|i| (p(i as f64), p(i as f64 + 1.))));
    let f = p(90.);
    t.push((f, (f.0, f.1 + 3.)));
    let m = corner_metrics(&t);
    assert_eq!(m["acute_corners"], 0, "{m}");
    assert_eq!(m["right_angle_corners"], 0, "{m}");
    assert_eq!(m["junctions_with_short_tracks"], 0, "{m}");
}

/// A 45-degree corner whose second leg is a run of 40 pieces of 9 um is one
/// acute corner for the scan too.
#[test]
fn a_corner_drawn_as_a_run_of_short_pieces_is_acute_for_the_scan() {
    let r = 135f64.to_radians();
    let p = |i: usize| {
        on_nm((
            JN.0 + i as f64 * 0.009 * r.cos(),
            JN.1 + i as f64 * 0.009 * r.sin(),
        ))
    };
    let mut t = vec![(JN, leg_at(180., 3.))];
    t.extend((0..40).map(|i| (p(i), p(i + 1))));
    let m = corner_metrics(&t);
    assert_eq!(m["acute_corners"], 1, "{m}");
    assert_eq!(m["acute_found_by_this_scan"], 1, "{m}");
}

// ─── Netclass pattern matcher (row A1) ───────────────────────────────────────

/// The matcher this module used before row A1, kept here only as the oracle of the
/// differential test below: for each `*` it tries every split of the rest of the name,
/// so its cost is exponential in the number of `*`. Never call it on a long name.
fn glob_recursive(pattern: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') => (0..=t.len()).any(|i| go(&p[1..], &t[i..])),
            Some('?') => !t.is_empty() && go(&p[1..], &t[1..]),
            Some(c) => t.first() == Some(c) && go(&p[1..], &t[1..]),
        }
    }
    go(
        &pattern.chars().collect::<Vec<_>>(),
        &text.chars().collect::<Vec<_>>(),
    )
}

/// Every string over `alphabet` with 0 to `max_len` characters, shortest first.
fn strings_up_to(alphabet: &[char], max_len: usize) -> Vec<String> {
    let mut all = vec![String::new()];
    let mut layer = vec![String::new()];
    for _ in 0..max_len {
        layer = layer
            .iter()
            .flat_map(|s| {
                alphabet.iter().map(move |c| {
                    let mut longer = s.clone();
                    longer.push(*c);
                    longer
                })
            })
            .collect();
        all.extend(layer.iter().cloned());
    }
    all
}

/// Runs `work` on its own thread and returns its result, or `None` if it has not finished within
/// `limit`. A matcher that is too slow therefore fails a test instead of hanging the run; the
/// abandoned thread dies with the test process.
fn finishes_within<T: Send + 'static>(
    limit: std::time::Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.recv_timeout(limit).ok()
}

/// The linear matcher gives the recursive matcher's answer for every input. Patterns and names
/// are all strings up to a small length over an alphabet that holds the wildcards, a non-ASCII
/// character, characters that are only literal, and (in the names) a literal `*`; that covers
/// the empty pattern and name, leading, trailing, doubled and adjacent `*`, and `?` at both
/// ends. A seeded pseudo-random set of longer pairs follows.
#[test]
fn glob_gives_the_same_answer_as_the_recursive_matcher() {
    let patterns = strings_up_to(&['a', 'b', '*', '?', 'é'], 5);
    let names = strings_up_to(&['a', 'b', 'é', '*'], 4);
    let (mut matched, mut refused) = (0usize, 0usize);
    for pattern in &patterns {
        for name in &names {
            let expected = glob_recursive(pattern, name);
            assert_eq!(
                glob(pattern, name),
                expected,
                "{pattern:?} against {name:?}"
            );
            if expected {
                matched += 1;
            } else {
                refused += 1;
            }
        }
    }
    // The set is not one-sided: a matcher that always said yes, or always no, would fail it.
    assert!(
        matched > 10_000 && refused > 10_000,
        "{matched} / {refused}"
    );

    // Longer pairs from a fixed generator (a 64-bit LCG), a few characters of alphabet so that
    // matches stay common.
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = |n: usize| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 33) as usize) % n
    };
    // Upper case, a combining accent and a character outside the basic plane join the letters.
    let pattern_alphabet = [
        'a',
        'A',
        'b',
        'é',
        '\u{301}',
        '\u{1d11e}',
        '*',
        '*',
        '?',
        '?',
    ];
    let name_alphabet = ['a', 'A', 'b', 'é', '\u{301}', '\u{1d11e}', '*', '?'];
    let mut long_matched = 0usize;
    for _ in 0..20_000 {
        let pattern: String = (0..next(13))
            .map(|_| pattern_alphabet[next(pattern_alphabet.len())])
            .collect();
        let name: String = (0..next(13))
            .map(|_| name_alphabet[next(name_alphabet.len())])
            .collect();
        let expected = glob_recursive(&pattern, &name);
        assert_eq!(
            glob(&pattern, &name),
            expected,
            "{pattern:?} against {name:?}"
        );
        long_matched += usize::from(expected);
    }
    assert!(long_matched > 500, "{long_matched}");
}

/// Case counts, as in KiCad (its pattern is compiled without `wxRE_ICASE`): a matcher that folded
/// case would classify `n_clean` into the class of `N_*`, and the differential test above cannot
/// say so for a matcher that folds both sides the same way.
#[test]
fn glob_counts_case() {
    assert!(glob("N_*", "N_CLEAN") && glob("n_*", "n_clean") && glob("A?c", "Abc"));
    assert!(!glob("n_*", "N_CLEAN") && !glob("N_*", "n_clean"));
    assert!(!glob("a", "A") && !glob("A", "a") && !glob("*a*", "XAX"));
    assert!(!glob("É*", "école") && glob("é*", "école"));
}

/// The cost of a match is the pattern length times the name length. The recursive matcher needed
/// a number of steps that grows exponentially with the `*` count, so twelve `*` against a 400
/// character name never finished: this test catches any return to it. The bounds are generous
/// (the linear matcher takes microseconds here) so a slow machine cannot fail it.
#[test]
fn glob_cost_does_not_grow_with_the_number_of_wildcards() {
    let limit = std::time::Duration::from_secs(5);
    let crafted = "*?".repeat(12) + "Q~";
    let name = "a".repeat(400);
    let answer = finishes_within(limit, move || glob(&crafted, &name));
    assert_eq!(answer, Some(false), "many wildcards against a long name");

    // Linear in both lengths, not just in the wildcard count: thousands of characters each way.
    let long_pattern = "*a?".repeat(1_500) + "Q";
    let long_name = "a".repeat(3_000);
    let answer = finishes_within(limit, move || {
        (
            glob(&long_pattern, &long_name),
            glob(&long_pattern, &(long_name.clone() + "Q")),
        )
    });
    assert_eq!(
        answer,
        Some((false, true)),
        "long pattern against long name"
    );
}

/// The call site: `netclasses_from_source` runs every pattern against every net, so crafted
/// patterns in a project file must not make it slow, and a real pattern beside them still
/// classifies its nets.
#[test]
fn netclasses_from_source_survives_crafted_patterns() {
    let crafted = "*?".repeat(12) + "Q~";
    let mut patterns: Vec<Value> = (0..40)
        .map(|i| json!({ "netclass": format!("Crafted{i}"), "pattern": crafted }))
        .collect();
    patterns.push(json!({ "netclass": "Clean", "pattern": "N_*" }));
    let source = json!({ "net_settings": { "netclass_patterns": patterns } }).to_string();
    let nets: BTreeSet<String> = (0..8)
        .map(|i| format!("{}{i}", "a".repeat(300)))
        .chain(["N_CLEAN".to_string()])
        .collect();
    let classes = finishes_within(std::time::Duration::from_secs(10), move || {
        netclasses_from_source(&source, &nets)
    })
    .expect("the crafted patterns must not stall the audit model");
    assert_eq!(classes.len(), 1, "{classes:?}");
    assert_eq!(classes["N_CLEAN"], vec!["Clean".to_string()]);
}
