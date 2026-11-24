//! Tests of `plan_pours`: one test and one negative control for each guard in
//! docs/POUR_PLAN_DESIGN.md, "Tests, each guard with a negative control".

use super::*;
use crate::pour_check::{NetOpen, ProposalFill, Risen};
use crate::pour_plan::tests::{pours_without_pours, with_node, POURS};
use crate::pour_plan::{
    project_rules, settings_for, CopperLayer, OpenRuleArea, ProposedZone, SupplyNet, SupplyRole,
};
use crate::router::ToolRouter;
use crate::tools::ServerConfig;
use std::sync::Arc;

const PROJECT: &str = include_str!("../../tests/fixtures/specctra_pours_routing.kicad_pro");

/// Stands in for `kicad-cli pcb drc --output <json> … --save-board <board>`: records
/// `KICAD_CONFIG_HOME`, gives every zone `zone_sexp` wrote one fill piece on its own
/// layer (unless `empty` is in its folder), and writes `with.json` or `without.json` as
/// the report. With `fail` in its folder it fails and writes nothing.
const FAKE_CLI: &str = r#"#!/bin/sh
dir="$(cd "$(dirname "$0")" && pwd)"
echo "KICAD_CONFIG_HOME=${KICAD_CONFIG_HOME-unset}" >> "$dir/calls.log"
if [ -f "$dir/fail" ]; then echo "Error: no KiCad" >&2; exit 1; fi
out=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--output" ]; then out="$a"; fi
  prev="$a"
done
board="$prev"
if [ ! -f "$dir/empty" ]; then
  /usr/bin/awk '{
    if (match($0, /\(zone \(net "[^"]*"\) \(layers "[^"]*"\)/)) {
      s = substr($0, RSTART, RLENGTH); sub(/.*\(layers "/, "", s); sub(/"\).*/, "", s); layer = s
    }
    if ($0 ~ /^    \(polygon \(pts/ && layer != "") {
      print "    (filled_polygon (layer \"" layer "\") (pts (xy 100 100) (xy 150 100) (xy 150 140) (xy 100 140)))"
      layer = ""
    }
    print
  }' "$board" > "$board.tmp" && mv "$board.tmp" "$board"
fi
case "$board" in
  */with/*) cp "$dir/with.json" "$out" ;;
  */with-thermal-1/*) if [ -f "$dir/second.json" ]; then cp "$dir/second.json" "$out"; else cp "$dir/with.json" "$out"; fi ;;
  *) cp "$dir/without.json" "$out" ;;
esac
exit 0
"#;

fn report_json(open_gnd: usize, violations: &[(&str, &str)]) -> String {
    json!({
        "violations": violations
            .iter()
            .map(|(rule, severity)| json!({
                "type": rule, "severity": severity, "description": "a violation", "items": []
            }))
            .collect::<Vec<_>>(),
        "unconnected_items": (0..open_gnd)
            .map(|_| json!({
                "type": "unconnected_items", "severity": "error", "description": "Missing connection",
                "items": [
                    { "description": "Pad 2 [GND] of R1", "uuid": "a", "pos": { "x": 1.0, "y": 1.0 } },
                    { "description": "Pad 2 [GND] of R2", "uuid": "b", "pos": { "x": 2.0, "y": 2.0 } }
                ]
            }))
            .collect::<Vec<_>>(),
    })
    .to_string()
}

/// A user's project with the fixture's board and project file, a fake kicad-cli and a
/// state folder.
struct Bench {
    _root: tempfile::TempDir,
    board: PathBuf,
    fake: PathBuf,
    state: PathBuf,
    ctx: ToolContext,
}

#[cfg(unix)]
fn bench(board_text: &str) -> Bench {
    let root = tempfile::tempdir().unwrap();
    let user = root.path().join("project");
    std::fs::create_dir_all(&user).unwrap();
    let board = user.join("board.kicad_pcb");
    std::fs::write(&board, board_text).unwrap();
    std::fs::write(user.join("board.kicad_pro"), PROJECT).unwrap();
    let fake = root.path().join("fake");
    std::fs::create_dir_all(&fake).unwrap();
    let cli = crate::tools::cli::test_support::write_script(&fake, "kicad-cli", FAKE_CLI, "");
    std::fs::write(fake.join("without.json"), report_json(11, &[])).unwrap();
    std::fs::write(
        fake.join("with.json"),
        report_json(0, &[("isolated_copper", "warning")]),
    )
    .unwrap();
    let state = root.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let ctx = ToolContext::new(
        ServerConfig {
            kicad_cli: cli.display().to_string(),
            kicad_binary: String::new(),
            ipc_address: String::new(),
            project_dir: None,
            jlcpcb_db_path: None,
            auto_load_toolsets: false,
            eager_toolsets: false,
        },
        Arc::new(ToolRouter::new()),
    );
    Bench {
        _root: root,
        board,
        fake,
        state,
        ctx,
    }
}

impl Bench {
    async fn run(&self) -> CallToolResult {
        plan_pours_in(&self.ctx, &self.board, BoardSource::Auto, Some(&self.state))
            .await
            .unwrap()
    }

    fn user_folder(&self) -> &Path {
        self.board.parent().unwrap()
    }
}

fn text_of(result: &CallToolResult) -> String {
    match result.content.first() {
        Some(crate::mcp::protocol::ToolContent::Text { text }) => text.clone(),
        other => panic!("expected text, got {other:?}"),
    }
}

fn body_of(result: &CallToolResult) -> Value {
    serde_json::from_str(&text_of(result)).expect("tool results are JSON")
}

fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(folder) = stack.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

// ─── The planner writes nothing; the result is the documented one ───────────────

#[cfg(unix)]
#[tokio::test]
async fn the_planner_writes_nothing_and_returns_the_documented_result() {
    let bench = bench(&pours_without_pours());
    let intent_dir = bench.user_folder().join(".konnect");
    std::fs::create_dir_all(&intent_dir).unwrap();
    std::fs::write(
        intent_dir.join("design_intent.json"),
        "{\"schema_version\": 1}\n",
    )
    .unwrap();
    let before = snapshot(bench.user_folder());
    let result = bench.run().await;
    assert!(!result.is_error, "{}", text_of(&result));
    assert_eq!(
        snapshot(bench.user_folder()),
        before,
        "nothing of the user's changed"
    );

    let body = body_of(&result);
    let keys: BTreeSet<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "status",
            "board",
            "board_sha256",
            "stack",
            "nets",
            "proposals",
            "not_proposed",
            "rule_areas_filled",
            "check",
            "next_steps",
            "sources",
            "source_evidence"
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(body["status"], PLANNED, "{body}");
    assert_eq!(body["sources"]["board"], "saved_board");
    assert_eq!(body["sources"]["check"], FROM_KICAD_CLI);
    assert_eq!(body["source_evidence"]["board_state"], "saved_board");
    assert_eq!(
        body["board_sha256"],
        crate::place_held::sha256_hex(pours_without_pours().as_bytes())
    );
    assert_eq!(
        body["stack"],
        json!([{"name": "F.Cu", "kind": "signal"}, {"name": "B.Cu", "kind": "signal"}])
    );
    assert_eq!(
        body["nets"],
        json!([{"net": "GND", "role": "ground", "source": "name", "pads": 12}])
    );
    let proposals = body["proposals"].as_array().unwrap();
    assert_eq!(proposals.len(), 2);
    for (proposal, layer) in proposals.iter().zip(["F.Cu", "B.Cu"]) {
        let keys: BTreeSet<&str> = proposal
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            ["net", "layer", "asked", "reason", "add_zone", "fill"]
                .into_iter()
                .collect()
        );
        assert_eq!(proposal["layer"], layer);
        assert_eq!(proposal["asked"], false);
        assert_eq!(
            proposal["add_zone"]["board"],
            bench.board.display().to_string()
        );
        assert_eq!(proposal["add_zone"]["clearance"], 0.2);
        assert_eq!(
            proposal["fill"],
            json!({"pieces": 1, "area_mm2": 2000.0, "largest_piece_share": 1.0})
        );
    }
    let check = &body["check"];
    let keys: BTreeSet<&str> = check
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "clean",
            "open",
            "risen",
            "starved_thermals_before",
            "starved_thermals",
            "unreached",
            "thermal_steps",
            "folder"
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(check["clean"], true);
    assert_eq!(
        check["open"],
        json!([{"net": "GND", "before": 11, "after": 0}])
    );
    assert_eq!(
        check["risen"],
        json!([{"rule": "isolated_copper", "severity": "warning", "before": 0, "after": 1}])
    );
    let folder = PathBuf::from(check["folder"].as_str().unwrap());
    assert!(folder.starts_with(bench.state.join(crate::pour_check::PLANS_FOLDER)));
    // The settings copy is not kept; the scratch projects are.
    assert!(!folder.join("kicad-config").exists());
    assert!(folder.join("with").join("board.kicad_pcb").is_file());
    let calls = std::fs::read_to_string(bench.fake.join("calls.log")).unwrap();
    let private = format!(
        "KICAD_CONFIG_HOME={}",
        folder.join("kicad-config").display()
    );
    assert_eq!(calls.lines().collect::<Vec<_>>(), [private.as_str(); 2]);
    // And the arguments read back as add_zone's settings.
    let settings = crate::tools::pcb_board::ZoneSettings::from_args(&proposals[0]["add_zone"]);
    assert!(settings.is_ok(), "{settings:?}");
}

// ─── No check, no plan ─────────────────────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn no_check_no_plan() {
    let bench = bench(&pours_without_pours());
    std::fs::write(bench.fake.join("fail"), "").unwrap();
    let result = bench.run().await;
    assert!(result.is_error, "{}", text_of(&result));
    let text = text_of(&result);
    assert!(!text.contains("proposals"), "{text}");
    assert!(text.contains("nothing is proposed"), "{text}");
    assert!(text.contains("step 2"), "{text}");
    // The settings copy is not kept after a failed check either.
    let plans = bench.state.join(crate::pour_check::PLANS_FOLDER);
    let folders: Vec<_> = std::fs::read_dir(&plans).unwrap().flatten().collect();
    assert_eq!(folders.len(), 1);
    assert!(!folders[0].path().join("kicad-config").exists());
    assert!(
        text.contains(&folders[0].path().display().to_string()),
        "{text}"
    );
}

// ─── A record or a project file that cannot be used is refused ─────────────────

#[cfg(unix)]
#[tokio::test]
async fn a_record_that_cannot_be_used_is_refused() {
    let bench = bench(&pours_without_pours());
    let intent_dir = bench.user_folder().join(".konnect");
    std::fs::create_dir_all(&intent_dir).unwrap();
    std::fs::write(intent_dir.join("design_intent.json"), "{ not json").unwrap();
    let result = bench.run().await;
    assert!(result.is_error);
    assert_eq!(
        crate::mcp::error::extract_error_kind(&result).as_deref(),
        Some("invalid_configuration")
    );
    assert!(!text_of(&result).contains("\"proposals\""));
    assert!(!bench.state.join(crate::pour_check::PLANS_FOLDER).exists());
}

#[cfg(unix)]
#[tokio::test]
async fn a_project_file_that_cannot_be_read_is_refused() {
    let bench = bench(&pours_without_pours());
    std::fs::write(
        bench.user_folder().join("board.kicad_pro"),
        [0xff, 0xfe, 0x00],
    )
    .unwrap();
    let result = bench.run().await;
    assert!(result.is_error, "{}", text_of(&result));
    assert!(!text_of(&result).contains("\"proposals\""));
    assert!(!bench.state.join(crate::pour_check::PLANS_FOLDER).exists());

    // A project file that reads and holds a bad number is refused as well.
    std::fs::write(
        bench.user_folder().join("board.kicad_pro"),
        json!({ "board": { "design_settings": { "rules": { "min_clearance": -1 } } } }).to_string(),
    )
    .unwrap();
    let result = bench.run().await;
    assert!(result.is_error, "{}", text_of(&result));
    assert!(text_of(&result).contains("min_clearance"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_board_the_export_refuses_gets_no_plan() {
    let bench = bench(&POURS.replace("(2 \"B.Cu\" signal)", "(2 \"B.Cu\" jumper)"));
    let result = bench.run().await;
    assert!(result.is_error, "{}", text_of(&result));
    assert!(text_of(&result).contains("Nothing was planned"));
}

#[tokio::test]
async fn the_arguments_are_checked_first() {
    let ctx = crate::tools::pcb_board::board_mock::ctx_talking_to(String::new());
    let result = handle_plan_pours(&json!({ "board": "/nowhere/board.kicad_sch" }), &ctx)
        .await
        .unwrap();
    assert!(result.is_error);
    let result = handle_plan_pours(&json!({ "board": "/nowhere/board.kicad_pcb" }), &ctx)
        .await
        .unwrap();
    assert!(result.is_error);
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.kicad_pcb");
    std::fs::write(&board, POURS).unwrap();
    let result = handle_plan_pours(
        &json!({ "board": board.to_str().unwrap(), "board_source": "sideways" }),
        &ctx,
    )
    .await
    .unwrap();
    assert!(result.is_error);
}

// ─── The real handler on a board that has its pours: nothing to propose ────────

#[cfg(unix)]
#[tokio::test]
async fn a_board_with_its_pours_has_nothing_to_propose_and_no_check() {
    let bench = bench(POURS);
    let result = bench.run().await;
    assert!(!result.is_error, "{}", text_of(&result));
    let body = body_of(&result);
    assert_eq!(body["status"], NOTHING_TO_PROPOSE);
    assert!(body.get("check").is_none(), "{body}");
    assert!(
        !bench.fake.join("calls.log").exists(),
        "kicad-cli did not run"
    );
    assert!(body["not_proposed"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["reason"].as_str().unwrap().contains("already poured")));
}

// ─── Pure pieces: withdraw, status, the result, the next steps ─────────────────

fn zone(net: &str, layer: &str, asked: bool) -> ProposedZone {
    ProposedZone {
        net: net.into(),
        layer: layer.into(),
        points_mm: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)],
        settings: settings_for(&[], &project_rules(None).unwrap(), None),
        reason: "by rule".into(),
        asked,
    }
}

fn fill(net: &str, layer: &str, pieces: usize) -> ProposalFill {
    ProposalFill {
        net: net.into(),
        layer: layer.into(),
        pieces,
        area_mm2: pieces as f64 * 12.3456,
        largest_piece_share: (pieces > 0).then_some(1.0),
    }
}

fn area(layer: &str) -> OpenRuleArea {
    OpenRuleArea {
        layer: layer.into(),
        name: Some("antenna".into()),
        uuid: Some("u-1".into()),
        forbids: vec!["tracks".into(), "vias".into()],
    }
}

fn plan_with(proposals: Vec<ProposedZone>) -> PourPlan {
    PourPlan {
        stack: vec![
            CopperLayer {
                name: "F.Cu".into(),
                kind: "signal".into(),
            },
            CopperLayer {
                name: "B.Cu".into(),
                kind: "signal".into(),
            },
        ],
        nets: vec![SupplyNet {
            net: "GND".into(),
            role: SupplyRole::Ground,
            source: "design_intent rail 'GND' at 0 V".into(),
            pads: 12,
        }],
        proposals,
        not_proposed: Vec::new(),
        filled_rule_areas: Vec::new(),
    }
}

fn clean_check(fills: Vec<ProposalFill>) -> FillCheck {
    FillCheck {
        fills,
        open: vec![NetOpen {
            net: "GND".into(),
            before: 3,
            after: 0,
        }],
        risen: Vec::new(),
        starved_thermals_before: 0,
        starved_thermals: 0,
        unreached: Vec::new(),
        thermal_steps: Vec::new(),
        clean: true,
    }
}

#[test]
fn a_proposal_kicad_fills_nothing_of_is_withdrawn() {
    let mut plan = plan_with(vec![zone("GND", "F.Cu", true), zone("GND", "B.Cu", false)]);
    plan.filled_rule_areas = vec![area("F.Cu"), area("B.Cu")];
    let mut check = clean_check(vec![fill("GND", "F.Cu", 0), fill("GND", "B.Cu", 2)]);
    withdraw_unfilled(&mut plan, &mut check);
    assert_eq!(plan.proposals.len(), 1);
    assert_eq!(plan.proposals[0].layer, "B.Cu");
    assert_eq!(check.fills.len(), 1);
    assert_eq!(check.fills[0].layer, "B.Cu");
    let row = plan
        .not_proposed
        .iter()
        .find(|r| r.layer.as_deref() == Some("F.Cu"))
        .unwrap();
    assert_eq!(row.net.as_deref(), Some("GND"));
    assert!(row.asked, "asked is kept");
    assert!(row.reason.contains("fills nothing"), "{}", row.reason);
    assert_eq!(plan.filled_rule_areas, [area("B.Cu")]);
    assert!(check.clean, "clean is not changed");
}

#[test]
fn the_status_tells_the_truth() {
    let plan = plan_with(vec![zone("GND", "F.Cu", false)]);
    let check = clean_check(vec![fill("GND", "F.Cu", 1)]);
    assert_eq!(status(&plan, Some(&check)), PLANNED);

    let mut unclean = check.clone();
    unclean.clean = false;
    assert_eq!(status(&plan, Some(&unclean)), PLANNED_WITH_FINDINGS);

    let mut unmet = plan.clone();
    unmet.not_proposed.push(NotProposed {
        net: Some("+5V".into()),
        layer: None,
        reason: "no pad".into(),
        asked: true,
    });
    assert_eq!(status(&unmet, Some(&check)), PLANNED_WITH_FINDINGS);
    let mut other = plan.clone();
    other.not_proposed.push(NotProposed {
        net: Some("+5V".into()),
        layer: None,
        reason: "not asked".into(),
        asked: false,
    });
    assert_eq!(status(&other, Some(&check)), PLANNED);

    let mut filled = plan.clone();
    filled.filled_rule_areas.push(area("F.Cu"));
    assert_eq!(status(&filled, Some(&check)), PLANNED_WITH_FINDINGS);

    assert_eq!(status(&plan_with(vec![]), None), NOTHING_TO_PROPOSE);
}

#[test]
fn no_proposal_comes_first() {
    let mut plan = plan_with(vec![zone("GND", "F.Cu", false)]);
    plan.not_proposed.push(NotProposed {
        net: Some("+12V".into()),
        layer: None,
        reason: "the board has no pad of that net".into(),
        asked: true,
    });
    let mut check = clean_check(vec![fill("GND", "F.Cu", 0)]);
    check.clean = false;
    withdraw_unfilled(&mut plan, &mut check);
    let body = result_json(
        Path::new("/b/board.kicad_pcb"),
        "abc",
        &plan,
        Some((&check, Path::new("/s/f"))),
    );
    assert_eq!(body["status"], NOTHING_TO_PROPOSE);
    assert!(body.get("check").is_some(), "{body}");
    assert!(body["not_proposed"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["net"] == "+12V" && row["asked"] == true));
    let steps = body["next_steps"].to_string();
    assert!(steps.contains("+12V"), "{steps}");
    assert!(body["proposals"].as_array().unwrap().is_empty());
}

#[test]
fn a_filled_rule_area_is_said_out_loud() {
    let mut plan = plan_with(vec![zone("GND", "F.Cu", false)]);
    plan.filled_rule_areas.push(area("F.Cu"));
    let check = clean_check(vec![fill("GND", "F.Cu", 1)]);
    let body = result_json(
        Path::new("/b/board.kicad_pcb"),
        "abc",
        &plan,
        Some((&check, Path::new("/s/f"))),
    );
    assert_eq!(
        body["rule_areas_filled"],
        json!([{"layer": "F.Cu", "name": "antenna", "uuid": "u-1", "forbids": ["tracks", "vias"]}])
    );
    let steps: Vec<String> = serde_json::from_value(body["next_steps"].clone()).unwrap();
    let line = steps
        .iter()
        .find(|s| s.contains("antenna"))
        .expect("a line names the area");
    assert!(
        line.contains("delete_zone") && line.contains("add_rule_area"),
        "{line}"
    );
    assert!(line.contains("keepout.copperpour true"), "{line}");
    assert_eq!(body["proposals"][0]["fill"]["area_mm2"], 12.346);
}

/// Every word of `steps` that reads like a tool (a snake_case word) is a registered tool,
/// a documented key of this result, or a known argument of the tool it is named with.
#[test]
fn the_next_steps_name_real_tools() {
    let mut plan = plan_with(vec![zone("GND", "F.Cu", false)]);
    plan.nets.push(SupplyNet {
        net: "VCC3".into(),
        role: SupplyRole::Power,
        source: "name".into(),
        pads: 2,
    });
    plan.filled_rule_areas.push(area("F.Cu"));
    plan.not_proposed.push(NotProposed {
        net: Some("+5V".into()),
        layer: Some("In1.Cu".into()),
        reason: "asked for on In1.Cu, which this board's copper stack does not have".into(),
        asked: true,
    });
    let check = FillCheck {
        risen: vec![Risen {
            rule: "clearance".into(),
            severity: "error".into(),
            before: 0,
            after: 1,
        }],
        clean: false,
        starved_thermals: 1,
        ..clean_check(vec![fill("GND", "F.Cu", 1)])
    };
    let mut all = next_steps(&plan, Some(&check));
    all.extend(next_steps(&plan_with(vec![]), None));
    let mut empty = plan_with(vec![]);
    empty.nets.clear();
    all.extend(next_steps(&empty, None));

    let tools: BTreeSet<String> = crate::router::registry::ALL_TOOLSETS
        .iter()
        .flat_map(|set| crate::router::registry::tools_for(set.name).unwrap_or_default())
        .map(|tool| tool.name.to_string())
        .collect();
    assert!(tools.contains("plan_pours"), "the tool is registered");
    // Words that are not tools: the result's own keys and KiCad's rule keys.
    let not_tools = ["not_proposed", "starved_thermals"];
    let mut named = BTreeSet::new();
    for step in &all {
        for word in step.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')) {
            let word = word.trim_matches('.');
            if !word.contains('_') || not_tools.contains(&word) {
                continue;
            }
            if let Some((tool, argument)) = word.split_once('.') {
                // `keepout.copperpour`: an argument of add_rule_area.
                let def = crate::router::registry::tools_for("pcb_board")
                    .unwrap()
                    .into_iter()
                    .find(|t| t.name == "add_rule_area")
                    .unwrap();
                assert!(
                    def.input_schema["properties"][tool]["properties"][argument].is_object(),
                    "{word} is no argument of add_rule_area"
                );
                continue;
            }
            assert!(
                tools.contains(word),
                "`{word}` in '{step}' is no registered tool"
            );
            named.insert(word.to_string());
        }
    }
    for tool in [
        "add_zone",
        "refill_zones",
        "save_project",
        "export_specctra_dsn",
        "delete_zone",
        "add_rule_area",
        "plan_pours",
        "set_design_intent",
    ] {
        assert!(named.contains(tool), "{tool} is named: {named:?}");
    }
    // `section 'rails'` and `section 'pours'` are sections of set_design_intent.
    let def = crate::router::registry::tools_for("design_intent")
        .or_else(|| {
            crate::router::registry::ALL_TOOLSETS
                .iter()
                .find_map(|set| {
                    crate::router::registry::tools_for(set.name)
                        .filter(|tools| tools.iter().any(|t| t.name == "set_design_intent"))
                })
        })
        .unwrap()
        .into_iter()
        .find(|t| t.name == "set_design_intent")
        .unwrap();
    let sections = def.input_schema["properties"]["section"]["enum"].to_string();
    assert!(
        sections.contains("rails") && sections.contains("pours"),
        "{sections}"
    );
}

#[test]
fn a_role_from_a_name_is_named_in_the_next_steps() {
    let mut plan = plan_with(vec![zone("GND", "F.Cu", false)]);
    plan.nets[0].source = "name".into();
    let steps = next_steps(&plan, None).join(" ");
    assert!(steps.contains("GND come from their names"), "{steps}");
    let plan = plan_with(vec![zone("GND", "F.Cu", false)]);
    assert!(!next_steps(&plan, None).join(" ").contains("names only"));
}

#[test]
fn an_unclean_check_names_what_rose() {
    let plan = plan_with(vec![zone("GND", "F.Cu", false)]);
    let check = FillCheck {
        risen: vec![Risen {
            rule: "clearance".into(),
            severity: "error".into(),
            before: 0,
            after: 2,
        }],
        clean: false,
        starved_thermals: 3,
        starved_thermals_before: 1,
        ..clean_check(vec![fill("GND", "F.Cu", 1)])
    };
    let steps = next_steps(&plan, Some(&check)).join(" ");
    assert!(steps.contains("clearance (error) 0 to 2"), "{steps}");
    assert!(steps.contains("1 to 3"), "{steps}");
    assert!(!next_steps(&plan, Some(&clean_check(vec![])))
        .join(" ")
        .contains("not clean"));
}

// ─── The tool is where the documents say ───────────────────────────────────────

#[test]
fn the_tool_is_in_pcb_board_and_the_count_is_18() {
    let tools = crate::router::registry::tools_for("pcb_board").unwrap();
    assert!(tools.iter().any(|t| t.name == "plan_pours"));
    assert_eq!(tools.len(), 18);
    let meta = crate::router::registry::ALL_TOOLSETS
        .iter()
        .find(|set| set.name == "pcb_board")
        .unwrap();
    assert_eq!(meta.tool_count, 18);
}

// ─── The real thing (ignored, needs kicad-cli) ─────────────────────────────────

/// kicad-cli of this Mac, or the one `KONNECT_KICAD_CLI` names.
fn real_kicad_cli() -> String {
    std::env::var("KONNECT_KICAD_CLI")
        .unwrap_or_else(|_| "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli".to_string())
}

fn real_ctx() -> ToolContext {
    ToolContext::new(
        ServerConfig {
            kicad_cli: real_kicad_cli(),
            kicad_binary: String::new(),
            ipc_address: String::new(),
            project_dir: None,
            jlcpcb_db_path: None,
            auto_load_toolsets: false,
            eager_toolsets: false,
        },
        Arc::new(ToolRouter::new()),
    )
}

fn real_project(board_text: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let user = root.path().join("project");
    std::fs::create_dir_all(&user).unwrap();
    let board = user.join("board.kicad_pcb");
    std::fs::write(&board, board_text).unwrap();
    std::fs::write(user.join("board.kicad_pro"), PROJECT).unwrap();
    let state = root.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    (root, board, state)
}

/// Run with `--ignored` and `KICAD_CONFIG_HOME` set to a scratch folder. The public
/// `pours` fixture with its pours removed: two proposals, KiCad fills both, clean.
#[tokio::test]
#[ignore = "needs kicad-cli; run with KICAD_CONFIG_HOME set to a scratch folder"]
async fn the_real_thing_fills_clean() {
    let (_root, board, state) = real_project(&pours_without_pours());
    let result = plan_pours_in(&real_ctx(), &board, BoardSource::Saved, Some(&state))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", text_of(&result));
    let body = body_of(&result);
    assert_eq!(body["status"], PLANNED, "{body}");
    assert_eq!(body["check"]["clean"], true, "{body}");
    let proposals = body["proposals"].as_array().unwrap();
    assert_eq!(proposals.len(), 2);
    for proposal in proposals {
        assert!(
            proposal["fill"]["pieces"].as_u64().unwrap() >= 1,
            "{proposal}"
        );
    }
    assert_eq!(body["check"]["starved_thermals"], 0, "{body}");
    // The pours join every GND pad. KiCad reports a pad with no spoke at all as
    // unconnected, not as a starved thermal (`drc_test_provider_zone_connections.cpp:176`),
    // so `clean` alone would not see a plan whose spokes KiCad cannot make.
    let gnd = body["check"]["open"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["net"] == "GND")
        .unwrap()
        .clone();
    assert!(gnd["before"].as_u64().unwrap() > 0, "{gnd}");
    assert_eq!(gnd["after"], 0, "{body}");
    assert_eq!(body["check"]["unreached"], json!([]), "{body}");
    assert_eq!(body["check"]["thermal_steps"].as_array().unwrap().len(), 1);

    // PP17: the minimum width and the spoke raised above the 0805 pads' 1.0 mm short side
    // (the fab target's minimum track), so that KiCad makes no spoke at either step.
    let (_root, board, state) = real_project(&pours_without_pours());
    let intent = board.parent().unwrap().join(".konnect");
    std::fs::create_dir_all(&intent).unwrap();
    std::fs::write(
        intent.join("design_intent.json"),
        json!({ "schema_version": 1, "fab_target": { "min_track_mm": 1.2 } }).to_string(),
    )
    .unwrap();
    let result = plan_pours_in(&real_ctx(), &board, BoardSource::Saved, Some(&state))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", text_of(&result));
    let body = body_of(&result);
    assert_eq!(body["check"]["clean"], false, "{body}");
    let unreached = body["check"]["unreached"].as_array().unwrap();
    assert_eq!(unreached.len(), 1, "{body}");
    assert_eq!(unreached[0]["net"], "GND");
    assert!(!unreached[0]["pads"].as_array().unwrap().is_empty());
    // Both steps ran; the bridge stayed at the minimum width.
    let steps = body["check"]["thermal_steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2, "{body}");
    assert_eq!(steps[1]["thermal_bridge_width"], 1.2);
}

/// PP16 through the tool: the first fill starves a thermal, the second is clean, and every
/// proposal's add_zone arguments carry the second's thermal values.
#[cfg(unix)]
#[tokio::test]
async fn the_kept_fills_values_are_what_is_returned() {
    let bench = bench(&pours_without_pours());
    std::fs::write(
        bench.fake.join("with.json"),
        report_json(0, &[("starved_thermal", "error")]),
    )
    .unwrap();
    std::fs::write(bench.fake.join("second.json"), report_json(0, &[])).unwrap();
    let result = bench.run().await;
    assert!(!result.is_error, "{}", text_of(&result));
    let body = body_of(&result);
    assert_eq!(body["status"], PLANNED, "{body}");
    for proposal in body["proposals"].as_array().unwrap() {
        assert_eq!(proposal["add_zone"]["thermal_gap"], 0.3, "{proposal}");
        assert_eq!(
            proposal["add_zone"]["thermal_bridge_width"], 0.4,
            "{proposal}"
        );
    }
    let steps = body["check"]["thermal_steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0]["kept"], false);
    assert_eq!(steps[1]["kept"], true);
    let lines = body["next_steps"].to_string();
    assert!(lines.contains("smaller thermal values"), "{lines}");
    let folder = PathBuf::from(body["check"]["folder"].as_str().unwrap());
    assert!(folder
        .join(crate::pour_check::WITH_THERMAL_1)
        .join("board.kicad_pcb")
        .is_file());
}

/// The same fixture with a rule area over part of the board that keeps tracks and vias
/// out and lets pours in: KiCad fills inside it, and with pours forbidden too the fill
/// is smaller (PP15's fact of KiCad).
#[tokio::test]
#[ignore = "needs kicad-cli; run with KICAD_CONFIG_HOME set to a scratch folder"]
async fn kicad_fills_inside_a_rule_area_that_lets_pours_in() {
    let area = |pours: &str| {
        format!(
            "\t(zone (layer \"F.Cu\") (uuid \"dddddddd-0000-4000-8000-000000000001\") (name \"keep\") (hatch edge 0.5) (keepout (tracks not_allowed) (vias not_allowed) (pads allowed) (copperpour {pours}) (footprints allowed)) (polygon (pts (xy 102 102) (xy 112 102) (xy 112 112) (xy 102 112))))"
        )
    };
    let mut areas = Vec::new();
    for pours in ["allowed", "not_allowed"] {
        let (_root, board, state) = real_project(&with_node(&pours_without_pours(), &area(pours)));
        let result = plan_pours_in(&real_ctx(), &board, BoardSource::Saved, Some(&state))
            .await
            .unwrap();
        assert!(!result.is_error, "{}", text_of(&result));
        let body = body_of(&result);
        let front = body["proposals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["layer"] == "F.Cu")
            .unwrap()
            .clone();
        areas.push(front["fill"]["area_mm2"].as_f64().unwrap());
        if pours == "allowed" {
            assert_eq!(
                body["rule_areas_filled"].as_array().unwrap().len(),
                1,
                "{body}"
            );
            assert_eq!(body["status"], PLANNED_WITH_FINDINGS);
        } else {
            assert!(body["rule_areas_filled"].as_array().unwrap().is_empty());
        }
    }
    assert!(
        areas[1] < areas[0] - 50.0,
        "KiCad keeps a fill out of an area only when it forbids pours: {areas:?}"
    );
}

/// The review of phase A, item 1: a request the board already meets is no finding. The
/// fixture with its F.Cu pour removed (GND is poured on B.Cu at priority 0), and the
/// record asking for GND without layers, then on B.Cu only.
#[test]
fn a_request_the_board_already_meets_is_no_finding() {
    let first = POURS.find("\t(zone\n").unwrap();
    let second = first + 1 + POURS[first + 1..].find("\t(zone\n").unwrap();
    let text = format!("{}{}", &POURS[..first], &POURS[second..]);
    let facts = crate::pour_plan::board_facts(&text).unwrap();
    assert_eq!(facts.existing.len(), 1);
    assert_eq!(facts.existing[0].layer, "B.Cu");
    let nets: BTreeSet<String> = facts.pads.keys().cloned().collect();
    let roles = crate::tools::supply_roles(&nets, None, Some(PROJECT));
    let rules = project_rules(Some(PROJECT)).unwrap();
    let record = |pours: Value| -> DesignIntent {
        serde_json::from_value(json!({ "schema_version": 1, "pours": pours })).unwrap()
    };

    let plan = crate::pour_plan::plan(
        &facts,
        &roles,
        &rules,
        Some(&record(json!([{ "net": "GND" }]))),
    );
    assert_eq!(plan.proposals.len(), 1);
    let check = clean_check(vec![fill("GND", "F.Cu", 1)]);
    assert_eq!(status(&plan, Some(&check)), PLANNED);
    assert!(!next_steps(&plan, Some(&check))
        .join(" ")
        .contains("not met"));

    let plan = crate::pour_plan::plan(
        &facts,
        &roles,
        &rules,
        Some(&record(json!([{ "net": "GND", "layers": ["B.Cu"] }]))),
    );
    assert!(plan.proposals.is_empty());
    assert_eq!(status(&plan, None), NOTHING_TO_PROPOSE);
    assert!(!next_steps(&plan, None).join(" ").contains("not met"));
}
