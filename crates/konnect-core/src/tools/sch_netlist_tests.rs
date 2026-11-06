//! Tests for `get_schematic_netlist` and `compare_netlist`.
//!
//! The expected connectivity is KiCad's, not this module's: `single_pin_nets.net`
//! is the output of `kicad-cli sch export netlist` for the committed
//! `single_pin_nets` hierarchy, and the README beside the fixture lists what
//! each net is meant to hold. Handler tests run the real handlers against a
//! stand-in `kicad-cli` that serves that file; the ignored tests at the end run
//! the real KiCad on edited copies of the hierarchy.

use super::*;
use crate::mcp::protocol::ToolContent;
use serde_json::json;
use std::path::Path;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn fixture_text(name: &str) -> String {
    std::fs::read_to_string(Path::new(FIXTURES).join(name)).unwrap()
}

fn fixture_design() -> Connectivity {
    connectivity(&kicad_netlist::parse(&fixture_text("single_pin_nets.net")).unwrap())
}

fn intent(nets: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
    nets.iter()
        .map(|(name, pins)| {
            (
                (*name).to_owned(),
                pins.iter().map(|pin| (*pin).to_owned()).collect(),
            )
        })
        .collect()
}

fn names<T: AsRef<str>>(items: &[T]) -> Vec<&str> {
    items.iter().map(AsRef::as_ref).collect()
}

// ─── KiCad's netlist, read ───────────────────────────────────────────────────

#[test]
fn the_hierarchy_wide_netlist_keeps_sheet_paths_and_kicads_own_net_names() {
    let design = fixture_design();
    assert_eq!(design.components, 12);
    // Across the sheet boundary: R8 is on the root sheet, R10 on the child.
    assert_eq!(names(&design.nets["/SHEET_NET"]), ["R8.2", "R10.2"]);
    // A net local to the child carries the sheet's path.
    assert_eq!(
        names(&design.nets["/Child/CHILD_LOCAL"]),
        ["R11.2", "R12.1"]
    );
    // A pin KiCad names itself, because nothing names its net.
    assert_eq!(names(&design.nets["unconnected-(R2-Pad2)"]), ["R2.2"]);
    // Power: nine pins on GND, listed in natural order (R3 before R10).
    assert_eq!(
        names(&design.nets["GND"]),
        ["R1.1", "R3.1", "R4.1", "R6.1", "R7.1", "R8.1", "R9.1", "R10.1", "R11.1"]
    );
    assert_eq!(design.nets.len(), 12);
    assert_eq!(design.pin_net.len(), 24);
    assert!(design.ambiguous.is_empty());
    assert!(design.unannotated.is_empty());
}

#[test]
fn pins_sort_the_way_a_person_counts() {
    let mut ids = vec!["R10.1", "R2.10", "R2.2", "U1.A10", "U1.A2", "R2.1", "C1.1"];
    ids.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(
        ids,
        ["C1.1", "R2.1", "R2.2", "R2.10", "R10.1", "U1.A2", "U1.A10"]
    );
}

#[test]
fn a_reference_used_twice_makes_its_pins_ambiguous() {
    let netlist = kicad_netlist::parse(
        r#"(export (components
            (comp (ref "R1") (value "1k")) (comp (ref "R1") (value "2k")) (comp (ref "R?") (value "1k")))
          (nets
            (net (code "1") (name "A") (node (ref "R1") (pin "1")) (node (ref "R1") (pin "1")))
            (net (code "2") (name "B") (node (ref "R1") (pin "1")) (node (ref "R?") (pin "2")))))"#,
    )
    .unwrap();
    let design = connectivity(&netlist);
    assert_eq!(design.ambiguous["R1.1"], ["A", "B"]);
    assert_eq!(design.unannotated, ["R?"]);
    // The same pin repeated on one net is one pin, not two.
    assert_eq!(names(&design.nets["A"]), ["R1.1"]);

    let problems = ambiguity(
        &design,
        &intent(&[("X", &["R1.1", "R1.2"])]),
        Scope::ListedPins,
    );
    assert_eq!(problems.len(), 1);
    // A pin the intent never names cannot make a listed comparison unreliable.
    assert!(ambiguity(&design, &intent(&[("X", &["R3.1"])]), Scope::ListedPins).is_empty());
    assert_eq!(
        ambiguity(&design, &intent(&[("X", &["R3.1"])]), Scope::Complete).len(),
        1
    );
}

// ─── Comparison ──────────────────────────────────────────────────────────────

/// The design's own partition under invented names.
fn renamed_copy_of_the_design() -> BTreeMap<String, BTreeSet<String>> {
    fixture_design()
        .nets
        .values()
        .enumerate()
        .map(|(index, pins)| (format!("INTENT_{index}"), pins.iter().cloned().collect()))
        .collect()
}

#[test]
fn the_same_partition_under_other_names_passes_even_when_complete() {
    let design = fixture_design();
    let intended = renamed_copy_of_the_design();
    for scope in [Scope::ListedPins, Scope::Complete] {
        let comparison = compare(&design, &intended, scope);
        assert!(comparison.is_empty(), "{scope:?}: {comparison:?}");
    }
}

#[test]
fn a_partial_intent_passes_when_the_listed_pins_are_joined_as_drawn() {
    let design = fixture_design();
    let intended = intent(&[
        ("A_NAME_THE_SCHEMATIC_DOES_NOT_USE", &["R1.2", "R2.1"]),
        ("ACROSS_SHEETS", &["R10.2", "R8.2"]),
    ]);
    assert!(compare(&design, &intended, Scope::ListedPins).is_empty());
}

#[test]
fn a_pin_the_design_does_not_have_is_missing() {
    let design = fixture_design();
    let intended = intent(&[("TWO_PIN", &["R1.2", "R2.1", "R99.1"])]);
    let comparison = compare(&design, &intended, Scope::ListedPins);
    assert_eq!(
        comparison.missing,
        [("TWO_PIN".to_owned(), "R99.1".to_owned())]
    );
    assert!(comparison.split.is_empty(), "{comparison:?}");
}

#[test]
fn an_intended_net_spread_over_two_nets_is_split() {
    // R4.2 is on /SPLIT and R6.2 on VCC: the intent wants them joined.
    let design = fixture_design();
    let comparison = compare(
        &design,
        &intent(&[("WANTED", &["R4.2", "R6.2"])]),
        Scope::ListedPins,
    );
    assert_eq!(comparison.split.len(), 1);
    let split = &comparison.split[0];
    assert_eq!(split.intended_net, "WANTED");
    let found: Vec<(&str, Vec<&str>)> = split
        .groups
        .iter()
        .map(|(net, pins)| (net.as_str(), names(pins)))
        .collect();
    assert_eq!(found, [("/SPLIT", vec!["R4.2"]), ("VCC", vec!["R6.2"])]);
    assert!(comparison.merged.is_empty() && comparison.missing.is_empty());
}

#[test]
fn a_design_net_holding_two_intended_nets_is_merged() {
    // The intent keeps R1.1 and R3.1 apart; the design ties both to GND.
    let design = fixture_design();
    let comparison = compare(
        &design,
        &intent(&[("RAIL_A", &["R1.1"]), ("RAIL_B", &["R3.1"])]),
        Scope::ListedPins,
    );
    assert_eq!(comparison.merged.len(), 1);
    let merged = &comparison.merged[0];
    assert_eq!(merged.design_net, "GND");
    let joined: Vec<&str> = merged.joins.iter().map(|(net, _)| net.as_str()).collect();
    assert_eq!(joined, ["RAIL_A", "RAIL_B"]);
    assert!(comparison.split.is_empty() && comparison.missing.is_empty());
}

#[test]
fn unlisted_pins_are_extra_only_when_the_scope_is_complete() {
    let design = fixture_design();
    // GND really holds nine pins; the intent names two.
    let intended = intent(&[("GND", &["R1.1", "R3.1"])]);
    assert!(compare(&design, &intended, Scope::ListedPins).is_empty());
    let comparison = compare(&design, &intended, Scope::Complete);
    let extras: Vec<&Extra> = comparison
        .extra
        .iter()
        .filter(|extra| extra.design_net == "GND")
        .collect();
    assert_eq!(extras.len(), 1);
    assert_eq!(extras[0].intended_nets, ["GND"]);
    assert_eq!(
        names(&extras[0].pins),
        ["R4.1", "R6.1", "R7.1", "R8.1", "R9.1", "R10.1", "R11.1"]
    );
    // Nets of one pin join nothing, so listing none of them is not a finding.
    assert!(comparison
        .extra
        .iter()
        .all(|extra| !extra.design_net.starts_with("unconnected")));
}

#[test]
fn pins_the_intent_never_lists_but_the_design_joins_are_extra_with_no_intended_net() {
    let design = fixture_design();
    let comparison = compare(&design, &intent(&[("ONLY", &["R6.2"])]), Scope::Complete);
    let hidden = comparison
        .extra
        .iter()
        .find(|extra| extra.design_net == "/TWO_PIN")
        .expect("R1.2 and R2.1 are joined and the intent says nothing about them");
    assert!(hidden.intended_nets.is_empty());
    assert_eq!(names(&hidden.pins), ["R1.2", "R2.1"]);
}

// ─── Intent validation ───────────────────────────────────────────────────────

fn refusal_text(result: CallToolResult) -> String {
    assert!(result.is_error);
    let ToolContent::Text { text } = &result.content[0] else {
        panic!("expected text");
    };
    text.clone()
}

#[test]
fn a_malformed_intent_is_refused_naming_the_field() {
    for (value, needle) in [
        (json!([]), "object"),
        (json!({}), "no nets"),
        (json!({ "A": [] }), "no pins"),
        (json!({ "A": "R1.1" }), "list"),
        (json!({ "A": ["R1"] }), "REF.PIN"),
        (json!({ "A": [".1"] }), "REF.PIN"),
        (json!({ "A": ["R1."] }), "REF.PIN"),
        (json!({ "A": [1] }), "not a string"),
        (json!({ "A": ["R1.1"], "B": ["R1.1"] }), "both"),
    ] {
        let text = refusal_text(parse_intended(&value).unwrap_err());
        assert!(text.contains("intended"), "{value}: {text}");
        assert!(text.contains(needle), "{value}: {text}");
    }
    // Spaces around a pin are not part of it; repeating a pin in one net is harmless.
    let parsed = parse_intended(&json!({ "A": [" R1.1 ", "R1.1", "J1.A.3"] })).unwrap();
    assert_eq!(
        parsed["A"].iter().map(String::as_str).collect::<Vec<_>>(),
        ["J1.A.3", "R1.1"]
    );
}

// ─── Responses ───────────────────────────────────────────────────────────────

fn fake_loaded() -> Loaded {
    Loaded {
        root: PathBuf::from("/design/top.kicad_sch"),
        netlist: kicad_netlist::parse(&fixture_text("single_pin_nets.net")).unwrap(),
        locks_present: Vec::new(),
    }
}

#[test]
fn a_page_of_nets_says_what_is_left_and_where_to_resume() {
    let loaded = fake_loaded();
    let design = connectivity(&loaded.netlist);
    let first = netlist_response(&design, &loaded, 5, 0, None);
    assert_eq!(first["returned"], 5);
    assert_eq!(first["net_count"], 12);
    assert_eq!(first["truncated"], true);
    assert_eq!(first["next_offset"], 5);
    let second = netlist_response(&design, &loaded, 5, 5, None);
    let third = netlist_response(&design, &loaded, 5, 10, None);
    assert_eq!(third["returned"], 2);
    assert_eq!(third["truncated"], false);
    assert!(third.get("next_offset").is_none());
    // The pages together are every net, once.
    let mut seen: Vec<String> = Vec::new();
    for page in [&first, &second, &third] {
        seen.extend(page["nets"].as_object().unwrap().keys().cloned());
    }
    let mut all: Vec<String> = design.nets.keys().cloned().collect();
    seen.sort();
    all.sort();
    assert_eq!(seen, all);
}

#[test]
fn a_name_filter_narrows_the_nets_and_the_counts() {
    let loaded = fake_loaded();
    let design = connectivity(&loaded.netlist);
    let response = netlist_response(&design, &loaded, 200, 0, Some("unconnected"));
    assert_eq!(response["matching_nets"], 3);
    assert_eq!(response["net_count"], 12);
    assert_eq!(response["truncated"], false);
    assert_eq!(response["nets"]["unconnected-(R5-Pad2)"], json!(["R5.2"]));
}

#[test]
fn a_huge_design_stays_under_the_size_cap_and_a_giant_net_is_shortened() {
    let mut design = Connectivity::default();
    for index in 0..3000 {
        design.nets.insert(
            format!("/Sheet_{index:04}/SOME_LONG_SIGNAL_NAME"),
            vec![format!("R{index}.1"), format!("C{index}.2")],
        );
    }
    design.nets.insert(
        "GND".to_owned(),
        (1..=1000).map(|index| format!("C{index}.1")).collect(),
    );
    let mut loaded = fake_loaded();
    loaded
        .locks_present
        .push(PathBuf::from("/design/~top.kicad_sch.lck"));
    let response = netlist_response(&design, &loaded, MAX_NET_LIMIT, 0, None);
    let text = serde_json::to_string(&response).unwrap();
    assert!(text.len() < 50_000, "{} characters", text.len());
    assert_eq!(response["truncated"], true);
    let returned = response["returned"].as_u64().unwrap();
    assert!(returned > 100 && returned < 1000, "{returned}");
    assert_eq!(response["next_offset"], returned);
    // The giant net, wherever it fell, is cut with its true size kept.
    let rest = netlist_response(&design, &loaded, 1, 0, Some("GND"));
    assert_eq!(rest["pins_truncated"]["nets"]["GND"], 1000);
    assert_eq!(
        rest["nets"]["GND"].as_array().unwrap().len(),
        MAX_PINS_PER_NET
    );
    // A lock is disclosed: unsaved editor changes are not in the copy.
    assert_eq!(
        response["source"]["editor_locks"],
        json!(["/design/~top.kicad_sch.lck"])
    );
    assert!(response["source"]["warning"]
        .as_str()
        .unwrap()
        .contains("not yet saved"));
}

#[test]
fn the_verdict_is_fail_pass_or_incomplete_and_never_a_silent_pass() {
    let loaded = fake_loaded();
    let design = connectivity(&loaded.netlist);
    let render = |intended: &BTreeMap<String, BTreeSet<String>>, problems: &[String]| {
        let reference = Reference {
            intended: intended.clone(),
            scope: Scope::ListedPins,
            description: json!({ "kind": "intended" }),
            problems: Vec::new(),
        };
        compare_response(
            &design,
            &loaded,
            &reference,
            &compare(&design, intended, Scope::ListedPins),
            problems,
            25,
        )
    };

    let good = render(&intent(&[("X", &["R1.2", "R2.1"])]), &[]);
    assert_eq!(
        (good["result"].as_str(), good["passed"].as_bool()),
        (Some("pass"), Some(true))
    );

    let bad = render(&intent(&[("X", &["R1.2", "R6.2"])]), &[]);
    assert_eq!(
        (bad["result"].as_str(), bad["passed"].as_bool()),
        (Some("fail"), Some(false))
    );
    assert_eq!(
        bad["counts"],
        json!({ "missing": 0, "split": 1, "merged": 0, "extra": 0 })
    );
    assert_eq!(bad["split"][0]["found_on"].as_array().unwrap().len(), 2);

    // Nothing wrong was found, but the evidence cannot tell pins apart.
    let unprovable = render(
        &intent(&[("X", &["R1.2", "R2.1"])]),
        &["R1.1 is listed on 2 nets".to_owned()],
    );
    assert_eq!(unprovable["result"], "incomplete");
    assert_eq!(unprovable["passed"], false);
    assert_eq!(
        unprovable["incomplete_because"][0],
        "R1.1 is listed on 2 nets"
    );
}

#[test]
fn findings_are_listed_up_to_the_limit_and_counted_in_full() {
    let loaded = fake_loaded();
    let design = connectivity(&loaded.netlist);
    let pins: Vec<String> = (100..160).map(|index| format!("X{index}.1")).collect();
    let intended: BTreeMap<String, BTreeSet<String>> = pins
        .iter()
        .map(|pin| (format!("N_{pin}"), BTreeSet::from([pin.clone()])))
        .collect();
    let reference = Reference {
        intended: intended.clone(),
        scope: Scope::ListedPins,
        description: json!({ "kind": "intended" }),
        problems: Vec::new(),
    };
    let response = compare_response(
        &design,
        &loaded,
        &reference,
        &compare(&design, &intended, Scope::ListedPins),
        &[],
        10,
    );
    assert_eq!(response["counts"]["missing"], 60);
    assert_eq!(response["missing"].as_array().unwrap().len(), 10);
    assert_eq!(response["truncated"], true);
    assert_eq!(response["result"], "fail");
}

// ─── The handlers, against a stand-in kicad-cli ──────────────────────────────

#[cfg(unix)]
mod handlers {
    use super::*;
    use crate::tools::cli::test_support::write_script;

    /// A project on disk and a `kicad-cli` that serves the fixture netlist and
    /// logs the schematic it was asked to export.
    struct Scene {
        _dir: tempfile::TempDir,
        root: PathBuf,
        log: PathBuf,
        ctx: ToolContext,
    }

    fn scene(netlist_fixture: &str) -> Scene {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        for name in [
            "single_pin_nets.kicad_sch",
            "single_pin_nets_child.kicad_sch",
        ] {
            std::fs::copy(Path::new(FIXTURES).join(name), project.join(name)).unwrap();
        }
        std::fs::write(project.join("single_pin_nets.kicad_pro"), "{}").unwrap();
        let log = dir.path().join("cli.log");
        let script = write_script(
            dir.path(),
            "fake-kicad-cli",
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\nout=''\nwhile [ $# -gt 0 ]; do\n  \
                 if [ \"$1\" = '--output' ]; then out=\"$2\"; fi\n  shift\ndone\n\
                 cp '{fixture}' \"$out\"\n",
                log = log.display(),
                fixture = Path::new(FIXTURES).join(netlist_fixture).display(),
            ),
            "",
        );
        let ctx = ToolContext::new(
            crate::tools::ServerConfig {
                kicad_cli: script.to_str().unwrap().to_owned(),
                ..crate::tools::ServerConfig::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        Scene {
            root: project.join("single_pin_nets.kicad_sch"),
            _dir: dir,
            log,
            ctx,
        }
    }

    fn body(result: &CallToolResult) -> Value {
        let ToolContent::Text { text } = &result.content[0] else {
            panic!("expected text");
        };
        serde_json::from_str(text).unwrap_or_else(|_| panic!("not JSON: {text}"))
    }

    #[tokio::test]
    async fn the_netlist_is_read_from_a_private_copy_never_the_live_files() {
        let scene = scene("single_pin_nets.net");
        let before = std::fs::read(&scene.root).unwrap();
        let result = handle_get_netlist(
            &json!({ "schematic": scene.root.to_string_lossy() }),
            &scene.ctx,
        )
        .await
        .unwrap();
        assert!(!result.is_error, "{result:?}");
        let response = body(&result);
        assert_eq!(response["net_count"], 12);
        assert_eq!(response["nets"]["/SHEET_NET"], json!(["R8.2", "R10.2"]));
        assert_eq!(response["source"]["saved_files_only"], true);

        let log = std::fs::read_to_string(&scene.log).unwrap();
        assert_eq!(log.lines().count(), 1, "{log}");
        let exported = log.lines().next().unwrap().split(' ').next_back().unwrap();
        assert!(
            !Path::new(exported).starts_with(scene.root.parent().unwrap().canonicalize().unwrap()),
            "kicad-cli was given the live file: {exported}"
        );
        assert!(log.contains("sch export netlist"), "{log}");
        assert!(log.contains("--format kicadsexpr"), "{log}");
        assert_eq!(std::fs::read(&scene.root).unwrap(), before);
    }

    #[tokio::test]
    async fn a_project_file_names_its_root_sheet_and_a_sub_sheet_is_refused() {
        let scene = scene("single_pin_nets.net");
        let project_file = scene.root.with_extension("kicad_pro");
        let by_project = handle_get_netlist(
            &json!({ "schematic": project_file.to_string_lossy(), "limit": 1 }),
            &scene.ctx,
        )
        .await
        .unwrap();
        assert!(!by_project.is_error, "{by_project:?}");
        assert_eq!(body(&by_project)["returned"], 1);

        let child = scene.root.with_file_name("single_pin_nets_child.kicad_sch");
        let refused =
            handle_get_netlist(&json!({ "schematic": child.to_string_lossy() }), &scene.ctx)
                .await
                .unwrap();
        let text = refusal_text(refused);
        assert!(text.contains("sheet inside the project"), "{text}");
        assert!(text.contains("single_pin_nets.kicad_sch"), "{text}");
        // Refused before kicad-cli ran at all.
        assert_eq!(
            std::fs::read_to_string(&scene.log).unwrap().lines().count(),
            1
        );
    }

    #[tokio::test]
    async fn bad_arguments_are_refused_before_kicad_cli_runs() {
        let scene = scene("single_pin_nets.net");
        let root = scene.root.to_string_lossy().to_string();
        for (tool, args, needle) in [
            ("get", json!({ "schematic": root, "limit": 0 }), "limit"),
            ("get", json!({ "schematic": root, "limit": 5000 }), "limit"),
            ("get", json!({ "schematic": root, "offset": -1 }), "offset"),
            ("get", json!({ "schematic": root, "limit": 1.5 }), "limit"),
            (
                "get",
                json!({ "schematic": "/nowhere/top.kicad_sch" }),
                "not found",
            ),
            (
                "get",
                json!({ "schematic": "/nowhere/top.txt" }),
                "kicad_sch",
            ),
            ("compare", json!({ "schematic": root }), "exactly one"),
            (
                "compare",
                json!({ "schematic": root, "intended": { "A": ["R1.1"] }, "reference_netlist": "x.net" }),
                "exactly one",
            ),
            (
                "compare",
                json!({ "schematic": root, "intended": { "A": ["R1.1"] }, "scope": "all" }),
                "scope",
            ),
            (
                "compare",
                json!({ "schematic": root, "reference_netlist": "/nowhere/x.net", "scope": "listed_pins" }),
                "scope",
            ),
            (
                "compare",
                json!({ "schematic": root, "reference_netlist": "/nowhere/x.net" }),
                "not found",
            ),
            (
                "compare",
                json!({ "schematic": root, "intended": { "A": ["R1"] } }),
                "ref.pin",
            ),
        ] {
            let result = if tool == "get" {
                handle_get_netlist(&args, &scene.ctx).await.unwrap()
            } else {
                handle_compare_netlist(&args, &scene.ctx).await.unwrap()
            };
            let text = refusal_text(result);
            assert!(text.to_lowercase().contains(needle), "{args}: {text}");
        }
        assert!(
            !scene.log.exists(),
            "kicad-cli ran for a request that should have been refused"
        );
    }

    #[tokio::test]
    async fn compare_passes_a_matching_intent_and_fails_a_wrong_one() {
        let scene = scene("single_pin_nets.net");
        let root = scene.root.to_string_lossy().to_string();
        let pass = handle_compare_netlist(
            &json!({ "schematic": root, "intended": {
                "SIG": ["R1.2", "R2.1"], "ACROSS": ["R8.2", "R10.2"],
                "CHILD": ["R11.2", "R12.1"] } }),
            &scene.ctx,
        )
        .await
        .unwrap();
        let pass = body(&pass);
        assert_eq!(pass["result"], "pass", "{pass}");
        assert_eq!(pass["compared"]["scope"], "listed_pins");
        assert_eq!(pass["compared"]["design_components"], 12);

        let fail = handle_compare_netlist(
            &json!({ "schematic": root, "intended": {
                "SIG": ["R1.2", "R2.1", "R5.1"], "RAIL_A": ["R1.1"], "RAIL_B": ["R3.1"],
                "GHOST": ["R42.1"] } }),
            &scene.ctx,
        )
        .await
        .unwrap();
        let fail = body(&fail);
        assert_eq!(fail["result"], "fail");
        assert_eq!(fail["passed"], false);
        assert_eq!(
            fail["counts"],
            json!({ "missing": 1, "split": 1, "merged": 1, "extra": 0 }),
            "{fail}"
        );
        assert_eq!(
            fail["missing"][0],
            json!({ "intended_net": "GHOST", "pin": "R42.1" })
        );
        assert_eq!(fail["merged"][0]["design_net"], "GND");
        assert_eq!(fail["split"][0]["intended_net"], "SIG");
    }

    #[tokio::test]
    async fn a_netlist_file_is_compared_completely_and_names_do_not_matter() {
        let scene = scene("single_pin_nets.net");
        let root = scene.root.to_string_lossy().to_string();
        let reference = Path::new(FIXTURES).join("single_pin_nets.net");
        let same = body(
            &handle_compare_netlist(
                &json!({ "schematic": root, "reference_netlist": reference.to_string_lossy() }),
                &scene.ctx,
            )
            .await
            .unwrap(),
        );
        assert_eq!(same["result"], "pass", "{same}");
        assert_eq!(same["compared"]["scope"], "complete");
        assert_eq!(same["compared"]["against"]["kind"], "reference_netlist");

        // The same circuit with every net renamed still passes.
        let renamed = scene.log.with_file_name("renamed.net");
        let text = fixture_text("single_pin_nets.net")
            .replace("\"/TWO_PIN\"", "\"Net-(R1-Pad2)\"")
            .replace("\"/SHEET_NET\"", "\"/Child/IN\"")
            .replace("\"VCC\"", "\"+5V\"");
        std::fs::write(&renamed, text).unwrap();
        let renamed = body(
            &handle_compare_netlist(
                &json!({ "schematic": root, "reference_netlist": renamed.to_string_lossy() }),
                &scene.ctx,
            )
            .await
            .unwrap(),
        );
        assert_eq!(renamed["result"], "pass", "{renamed}");
    }

    #[tokio::test]
    async fn a_reference_netlist_that_is_not_a_netlist_is_refused() {
        let scene = scene("single_pin_nets.net");
        let junk = scene.log.with_file_name("junk.net");
        std::fs::write(&junk, "(not a netlist").unwrap();
        let result = handle_compare_netlist(
            &json!({ "schematic": scene.root.to_string_lossy(), "reference_netlist": junk.to_string_lossy() }),
            &scene.ctx,
        )
        .await
        .unwrap();
        let text = refusal_text(result);
        assert!(text.contains("reference_netlist"), "{text}");
        assert!(text.contains("netlist"), "{text}");
    }

    #[tokio::test]
    async fn a_failed_export_is_an_error_and_never_an_empty_design() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("p");
        std::fs::create_dir(&project).unwrap();
        let sheet = project.join("top.kicad_sch");
        std::fs::copy(
            Path::new(FIXTURES).join("annotate_duplicates.kicad_sch"),
            &sheet,
        )
        .unwrap();
        let script = write_script(dir.path(), "failing-cli", "#!/bin/sh\nexit 3\n", "");
        let ctx = ToolContext::new(
            crate::tools::ServerConfig {
                kicad_cli: script.to_str().unwrap().to_owned(),
                ..crate::tools::ServerConfig::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        let error = handle_compare_netlist(
            &json!({ "schematic": sheet.to_string_lossy(), "intended": { "A": ["R1.1"] } }),
            &ctx,
        )
        .await
        .expect_err("a failing kicad-cli must not read as a verdict");
        assert!(format!("{error:#}").contains("kicad-cli"), "{error:#}");
    }

    #[test]
    fn the_tools_are_in_the_schematic_export_toolset() {
        let names: Vec<&str> = tools().iter().map(|tool| tool.name).collect();
        assert_eq!(names, ["get_schematic_netlist", "compare_netlist"]);
        let exported: Vec<&str> = crate::tools::sch_export::tools()
            .iter()
            .map(|tool| tool.name)
            .collect();
        assert!(exported.contains(&"get_schematic_netlist"));
        assert!(exported.contains(&"compare_netlist"));
    }
}

// ─── Real KiCad, on edited copies of the hierarchy ───────────────────────────

/// Remove (`replacement` `None`) or rename the `nth` `(label "NAME" …)` block.
fn edit_label(text: &str, name: &str, nth: usize, replacement: Option<&str>) -> String {
    let marker = format!("(label \"{name}\"");
    let start = text
        .match_indices(&marker)
        .nth(nth)
        .unwrap_or_else(|| panic!("no label {name} #{nth}"))
        .0;
    match replacement {
        Some(new) => format!(
            "{}(label \"{new}\"{}",
            &text[..start],
            &text[start + marker.len()..]
        ),
        None => {
            let mut depth = 0usize;
            let mut end = start;
            for (offset, character) in text[start..].char_indices() {
                match character {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = start + offset + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            format!(
                "{}{}",
                &text[..start],
                text[end..].trim_start_matches(['\n', '\t'])
            )
        }
    }
}

/// A copy of the hierarchy in a new folder, with the root sheet's text changed.
fn project_copy(dir: &Path, edit: impl Fn(&str) -> String) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    for name in [
        "single_pin_nets.kicad_sch",
        "single_pin_nets_child.kicad_sch",
    ] {
        std::fs::copy(Path::new(FIXTURES).join(name), dir.join(name)).unwrap();
    }
    let root = dir.join("single_pin_nets.kicad_sch");
    let edited = edit(&std::fs::read_to_string(&root).unwrap());
    std::fs::write(&root, edited).unwrap();
    root
}

/// A context for the real KiCad: `KICAD_CLI`, or `kicad-cli` found as Konnect finds it.
fn live_ctx() -> ToolContext {
    ToolContext::new(
        crate::tools::ServerConfig {
            kicad_cli: std::env::var("KICAD_CLI").unwrap_or_else(|_| "kicad-cli".to_owned()),
            ..crate::tools::ServerConfig::default()
        },
        std::sync::Arc::new(crate::router::ToolRouter::new()),
    )
}

async fn live_compare(actual: &Path, reference: &Path) -> Value {
    let ctx = live_ctx();
    let result = handle_compare_netlist(
        &json!({
            "schematic": actual.to_string_lossy(),
            "reference_schematic": reference.to_string_lossy(),
        }),
        &ctx,
    )
    .await
    .unwrap();
    let ToolContent::Text { text } = &result.content[0] else {
        panic!("expected text");
    };
    assert!(!result.is_error, "{text}");
    serde_json::from_str(text).unwrap()
}

/// Needs the real KiCad. Run: `cargo test -p konnect-core --lib sch_netlist -- --ignored`
#[tokio::test]
#[ignore = "needs kicad-cli"]
async fn real_kicad_reads_the_hierarchy_and_compares_edited_copies() {
    let dir = tempfile::tempdir().unwrap();
    let original = project_copy(&dir.path().join("original"), str::to_owned);
    let ctx = live_ctx();

    // The read: the hierarchy, KiCad's own names, and a sheet-crossing net.
    let read = handle_get_netlist(&json!({ "schematic": original.to_string_lossy() }), &ctx)
        .await
        .unwrap();
    let ToolContent::Text { text } = &read.content[0] else {
        panic!("expected text");
    };
    assert!(!read.is_error, "{text}");
    let read: Value = serde_json::from_str(text).unwrap();
    assert_eq!(read["net_count"], 12, "{read}");
    assert_eq!(read["nets"]["/SHEET_NET"], json!(["R8.2", "R10.2"]));
    assert_eq!(
        read["nets"]["/Child/CHILD_LOCAL"],
        json!(["R11.2", "R12.1"])
    );
    assert_eq!(read["nets"]["unconnected-(R12-Pad2)"], json!(["R12.2"]));
    assert_eq!(read["nets"]["GND"].as_array().unwrap().len(), 9, "{read}");
    // Reading must match the committed copy of KiCad's own export.
    let committed = fixture_design();
    for (name, pins) in &committed.nets {
        assert_eq!(read["nets"][name], json!(pins), "{name}");
    }

    // An unchanged copy of the project is the same circuit.
    let same = project_copy(&dir.path().join("same"), str::to_owned);
    assert_eq!(live_compare(&same, &original).await["result"], "pass");

    // Take the only label off /TWO_PIN: KiCad now names that net itself, and
    // the circuit is unchanged.
    let unnamed = project_copy(&dir.path().join("unnamed"), |text| {
        edit_label(text, "TWO_PIN", 0, None)
    });
    let unnamed_read = handle_get_netlist(
        &json!({ "schematic": unnamed.to_string_lossy(), "name_contains": "Net-(" }),
        &ctx,
    )
    .await
    .unwrap();
    let ToolContent::Text { text } = &unnamed_read.content[0] else {
        panic!("expected text");
    };
    let unnamed_read: Value = serde_json::from_str(text).unwrap();
    let auto_named = unnamed_read["nets"].as_object().unwrap();
    assert_eq!(auto_named.len(), 1, "{unnamed_read}");
    let (auto_name, pins) = auto_named.iter().next().unwrap();
    assert!(auto_name.starts_with("Net-("), "{auto_name}");
    assert_eq!(pins, &json!(["R1.2", "R2.1"]));
    let renamed = live_compare(&unnamed, &original).await;
    assert_eq!(
        renamed["result"], "pass",
        "a rename is not a change: {renamed}"
    );

    // Take one of /SPLIT's two labels off: R4.2 and R5.1 stop being one net.
    let split = project_copy(&dir.path().join("split"), |text| {
        edit_label(text, "SPLIT", 1, None)
    });
    let split = live_compare(&split, &original).await;
    assert_eq!(split["result"], "fail", "{split}");
    assert_eq!(
        split["counts"],
        json!({ "missing": 0, "split": 1, "merged": 0, "extra": 0 })
    );
    assert_eq!(split["split"][0]["intended_net"], "/SPLIT");

    // Rename the second /SPLIT label to TWO_PIN: R5.1 leaves /SPLIT and joins
    // R1.2 and R2.1, so one net is split and another merged.
    let merged = project_copy(&dir.path().join("merged"), |text| {
        edit_label(text, "SPLIT", 1, Some("TWO_PIN"))
    });
    let merged = live_compare(&merged, &original).await;
    assert_eq!(merged["result"], "fail", "{merged}");
    assert_eq!(merged["counts"]["merged"], 1, "{merged}");
    assert_eq!(merged["counts"]["split"], 1, "{merged}");
    assert_eq!(merged["merged"][0]["design_net"], "/TWO_PIN");
}
