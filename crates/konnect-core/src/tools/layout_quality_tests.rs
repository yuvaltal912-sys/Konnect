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
        Some(crate::mcp::protocol::ToolContent::Text { text }) => serde_json::from_str(text).unwrap(),
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
        let name = std::path::Path::new(path).file_name().unwrap().to_string_lossy().to_string();
        let board = dir.path().join("b.kicad_pcb");
        std::fs::copy(path, &board).unwrap();
        if let Some(pro) = std::path::Path::new(path).with_extension("kicad_pro").exists().then(|| std::path::Path::new(path).with_extension("kicad_pro")) {
            std::fs::copy(pro, dir.path().join("b.kicad_pro")).unwrap();
        }
        let started = std::time::Instant::now();
        let params = Params::from_args(&json!({})).unwrap_or_else(|_| panic!("args"));
        let result = audit(&board, &params).unwrap();
        let Some(crate::mcp::protocol::ToolContent::Text { text }) = result.content.first() else { panic!() };
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
        println!("PROBE   metrics {}", serde_json::to_string(&r["metrics"]).unwrap());
        if let Ok(out) = std::env::var("LQ_PROBE_OUT") {
            std::fs::write(std::path::Path::new(&out).join(format!("{name}.json")), serde_json::to_string_pretty(&r).unwrap()).unwrap();
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
    check(report, id)["score"].as_f64().unwrap_or_else(|| panic!("no score for {id}: {}", check(report, id)))
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
    assert_eq!(ok["runs_off_ground"], 0);
    // The other crosses a 3 mm slot (x 20..23) and the 3V3 region (x 40..54): 17 of 48 mm.
    let cov = bad["coverage"].as_f64().unwrap();
    assert!((0.58..0.72).contains(&cov), "{bad}");
    let other = bad["other_net_plane_mm"].as_f64().unwrap();
    assert!((13.0..15.0).contains(&other), "3V3 stretch {other}");
    let void = bad["void_mm"].as_f64().unwrap();
    assert!((2.0..4.5).contains(&void), "slot {void}");
    // The slot, the 3V3 region, and the sliver of clearance between that region and ground.
    assert!(bad["runs_off_ground"].as_u64().unwrap() >= 2, "{bad}");
    // One finding, for the bad net only, with the measured value, the threshold and a UUID.
    let fs = findings(&r, "3.1");
    assert_eq!(fs.len(), 1, "{fs:?}");
    let f = fs[0];
    assert_eq!(f["subject"], "CLK_SPLIT");
    assert!(f["issue"].as_str().unwrap().contains("'3V3' plane"));
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
    let r = run_with(&f, json!({ "min_reference_coverage": 0.5, "max_void_run_mm": 20.0 }));
    assert!(findings(&r, "3.1").is_empty());
    assert_eq!(r["thresholds"]["min_reference_coverage"]["source"], "argument");
    assert_eq!(r["thresholds"]["max_void_run_mm"]["value"], 20.0);
    assert_eq!(r["thresholds"]["sample_mm"]["source"], "default");
}

#[test]
fn zones_without_a_saved_fill_are_not_judged_as_voids() {
    let r = run(&fx_text(&without_fills("plane_split.kicad_pcb"), None));
    let c = check(&r, "3.1");
    assert_eq!(c["status"], "not_run", "{c}");
    assert!(c["reason"].as_str().unwrap().contains("no saved fill"), "{c}");
    assert!(findings(&r, "3.1").is_empty());
    assert!(r["quality"]["not_scored"].as_array().unwrap().iter().any(|n| n["item"] == "3.1"));
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
    assert!(fs.iter().all(|f| f["subject"].as_str().unwrap().starts_with("CLK_BARE")));
    let nearest = fs[0]["measured"]["nearest_stitch_mm"].as_f64().unwrap();
    assert!(nearest > 7.0, "{nearest}");
    assert_eq!(fs[0]["threshold"]["stitch_via_max_mm"], 3.0);
    assert_eq!(fs[0]["uuids"].as_array().unwrap().len(), 2);
    assert!(f_for(&r, "3.2", "CLK_STITCHED").is_empty());
}

#[test]
fn a_wide_stitch_limit_accepts_a_distant_via() {
    let r = run_with(&fx("layer_change.kicad_pcb", None, None), json!({ "stitch_via_max_mm": 30.0 }));
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
    let errors: Vec<_> = findings(&r, "3.3").into_iter().filter(|f| f["severity"] == "error").collect();
    assert_eq!(errors.len(), 1, "{:?}", findings(&r, "3.3"));
    assert!(errors[0]["issue"].as_str().unwrap().contains("separate copper groups"));
    assert!(findings(&r, "3.3").iter().any(|f| f["severity"] == "warning" && f["issue"].as_str().unwrap().contains("touches no pad or via")));
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
    let r = run_with(&fx("decoupling.kicad_pcb", None, None), json!({ "decap_search_mm": 1.0 }));
    let fs = findings(&r, "4.1");
    assert_eq!(fs.len(), 2, "{fs:?}");
    assert!(fs.iter().all(|f| f["issue"].as_str().unwrap().contains("no capacitor")));
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
    let (x, y) = (f["at"]["x"].as_f64().unwrap(), f["at"]["y"].as_f64().unwrap());
    assert!((24.5..28.5).contains(&x) && (9.0..11.0).contains(&y), "{x},{y}");
    assert!(!f["uuids"].as_array().unwrap().is_empty());
    // The wide net is clean and measured at about its 2 mm.
    let paths = c["metrics"]["paths"].as_array().unwrap();
    let wide = paths.iter().find(|p| p["net"] == "P5V_WIDE").unwrap();
    assert!((1.8..2.3).contains(&wide["narrowest_mm"].as_f64().unwrap()), "{wide}");
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
    let r = run_with(&fx("neck_down.kicad_pcb", Some(neck_intent(0.3)), None), json!({ "bottleneck_min_current_a": 0.1 }));
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
        assert!(c["reason"].as_str().unwrap().contains("current") || c["reason"].as_str().unwrap().contains("design-intent"), "{c}");
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
    assert!(n1["topology_basis"].as_str().unwrap().contains("supplied by U1"));
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
    assert!(findings(&r, "4.1").iter().any(|f| f["subject"].as_str().unwrap().starts_with("U2.1")));
}

#[test]
fn an_undecided_topology_is_needs_human_and_an_argument_decides_it() {
    let f = fx("buck.kicad_pcb", None, None);
    let r = run(&f);
    let c = check(&r, "8.1");
    assert_eq!(c["status"], "not_run", "{c}");
    assert!(r["needs_human"].as_array().unwrap().iter().any(|n| n["item"] == "8.1" && n["why"].as_str().unwrap().contains("topology not determined")));
    let r = run_with(&f, json!({ "switching_regulators": [{ "switch_net": "SW1", "topology": "buck" }, { "switch_net": "SW2", "topology": "buck" }] }));
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
    assert!((5.0..9.0).contains(&a1["copper_area_mm2"].as_f64().unwrap()), "{a1}");
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
    let r = run_with(&fx("buck.kicad_pcb", Some(buck_intent()), None), json!({ "max_switch_node_area_mm2": 3.0 }));
    assert_eq!(findings(&r, "8.2").iter().filter(|f| f["measured"].get("copper_area_mm2").is_some()).count(), 2);
}

#[test]
fn a_generous_hot_loop_limit_clears_the_finding() {
    let r = run_with(&fx("buck.kicad_pcb", Some(buck_intent()), None), json!({ "hot_loop_max_mm": 100.0, "hot_loop_max_area_mm2": 500.0 }));
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
    let row = |n: &str| rows.iter().find(|r| r["net"] == n).unwrap_or_else(|| panic!("{n}: {rows:?}"));
    let d = row("N_DETOUR");
    assert!((2.9..3.1).contains(&d["detour_ratio"].as_f64().unwrap()), "{d}");
    assert_eq!(d["vias"], 2);
    assert_eq!(row("N_STUB")["dangling_ends"], 1);
    assert!((0.99..1.01).contains(&row("N_CLEAN")["detour_ratio"].as_f64().unwrap()));
    let fs = findings(&r, "5.1");
    let subjects: Vec<&str> = fs.iter().map(|f| f["subject"].as_str().unwrap()).collect();
    assert!(subjects.contains(&"N_DETOUR") && subjects.contains(&"N_STUB") && subjects.contains(&"N_CORNER"), "{subjects:?}");
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
    let acute = findings(&r, "5.3").into_iter().find(|f| f["issue"].as_str().unwrap().starts_with("Acid trap")).unwrap();
    assert_eq!(acute["subject"], "N_CORNER");
    assert_eq!(acute["severity"], "warning");
    assert!(findings(&r, "5.3").iter().any(|f| f["severity"] == "info" && f["issue"].as_str().unwrap().starts_with("Right-angle")));
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
    let fs: Vec<_> = findings(&r, "5.4").into_iter().filter(|f| f["severity"] == "warning").collect();
    assert_eq!(fs.len(), 1, "{fs:?}");
    assert!(fs[0]["subject"].as_str().unwrap().starts_with("DB_P"));
    // Length mismatch is information only: matching needs the interface's skew limit.
    assert!(findings(&r, "5.4").iter().any(|f| f["severity"] == "info" && f["subject"].as_str().unwrap().ends_with("length")));
    assert!(r["needs_human"].as_array().unwrap().iter().any(|n| n["item"] == "5.5"));
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
    let why = r["net_roles"]["critical"]["nets"]["P5V_NECK"].as_str().unwrap_or("");
    assert!(why.starts_with("fallback") || r["net_roles"]["critical"]["count"].as_u64().unwrap() == 0, "{why}");
    let r = run_with(&fx("plane_split.kicad_pcb", None, None), json!({ "signal_scope": "all_signals" }));
    assert_eq!(r["net_roles"]["critical_basis"], "all_signals");
}

#[test]
fn a_board_with_nets_left_unrouted_scores_below_the_routed_one() {
    let full = run(&fx("routing.kicad_pcb", None, None));
    let open = run(&fx_text(&without_copper_routing("routing.kicad_pcb"), None));
    assert_eq!(full["completion"]["complete"], true);
    assert_eq!(open["completion"]["complete"], false);
    assert!(open["completion"]["unrouted_nets"].as_u64().unwrap() >= 10);
    let (a, b) = (full["quality"]["score"].as_f64().unwrap(), open["quality"]["score"].as_f64().unwrap());
    assert!(b < a * 0.5, "routed {a}, stripped {b}");
    assert!(open["summary"].as_str().unwrap().contains("INCOMPLETE"));
    assert!(open["quality"]["routed_share"].as_f64().unwrap() < 0.2);
}

#[test]
fn good_routing_scores_above_poor_routing_of_the_same_kind() {
    // Same checks, same thresholds: the tight buck beats the loose one, per pin and per node.
    let r = run(&fx("buck.kicad_pcb", Some(buck_intent()), None));
    let pins = check(&r, "4.1")["metrics"]["pins"].as_array().unwrap().clone();
    let loop_of = |p: &str| pins.iter().find(|x| x["pin"] == p).unwrap()["loop"]["loop_mm"].as_f64().unwrap();
    assert!(loop_of("U1.1") * 4.0 < loop_of("U2.1"));
    assert!(r["quality"]["score"].as_f64().unwrap() > 0.0);
    let cats = r["quality"]["categories"].as_array().unwrap();
    assert!(cats.iter().all(|c| c["score"].is_null() || (0.0..=100.0).contains(&c["score"].as_f64().unwrap())));
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
    assert!(r["needs_human"].as_array().unwrap().iter().any(|n| n["item"] == "11"));
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
    assert!(t.input_validator.is_valid(&json!({ "board": "x.kicad_pcb" })));
    assert!(!t.input_validator.is_valid(&json!({ "board": "x.kicad_pcb", "surprise": 1 })));
    assert!(!t.input_validator.is_valid(&json!({ "board": "x.kicad_pcb", "sample_mm": 99 })));
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
    let f = fx_text("(kicad_pcb (version 20260206) (layers (25 \"Edge.Cuts\" user)))", None);
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
    for p in ["+3V3", "5V", "12V", "/sheet/+1V8", "VCC", "VDD_IO", "VBUS", "VIN"] {
        assert!(power_by_name(p), "{p}");
    }
    for n in ["SDA", "N_CLEAN", "CLK_OK", "TX", "VREF_NEG", "GATE1"] {
        assert!(!ground_by_name(n), "{n}");
    }
    assert!(critical_by_name("/mcu/SPI_SCLK") && critical_by_name("USB_DP") && !critical_by_name("SDA"));
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
    let from = g.cells_under(&konnect_sexp::board_geometry::Shape::disc((3.0, 5.0), 0.3), &[0]);
    let to = g.cells_under(&konnect_sexp::board_geometry::Shape::disc((37.0, 5.0), 0.3), &[0]);
    let p = shortest_path(&g, &from, &to, 0.5).unwrap();
    assert!((p.length_mm - 34.0).abs() < 0.6, "{}", p.length_mm);
    let w = widest_path(&g, &dt, &from, &to, &[1.0], &[]).unwrap();
    assert!((w.width_mm - 0.4).abs() < 0.12, "{}", w.width_mm);
    assert!((w.at.0 - 20.0).abs() < 2.4, "{:?}", w.at);
    // A ring with a bridged hole fills by the even-odd rule.
    let mut h = Grid::new((0.0, 0.0, 12.0, 12.0), 0.1, vec!["F.Cu".into()]);
    h.add_ring(0, &[(1.0, 1.0), (11.0, 1.0), (11.0, 11.0), (1.0, 11.0), (1.0, 6.0), (4.0, 4.0), (4.0, 8.0), (1.0, 6.0)]);
    assert!(h.area_mm2() > 80.0 && h.area_mm2() < 100.0, "{}", h.area_mm2());
}

