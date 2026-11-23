//! Row ZR, ZR0b: the judge parts of the ground-via prune (`routed_scratch`, `judge_light`),
//! its production checker (`JudgeChecker`) and `prune_winner`, with a fake kicad-cli and a
//! scripted audit (no KiCad). A child of the judge's tests, so that it reuses their
//! export of the public rounded board.

use super::*;
use crate::pour_prune::{
    self, prune_winner_with, units, without, CheckOutcome, Checked, Checker, JudgeChecker,
    PruneRecord,
};
use crate::route_search::{RunRecord, SearchOutcome, Shape};
use std::sync::Mutex;
use std::time::Duration;

/// Stands in for `kicad-cli pcb drc --output <json> --format json --refill-zones
/// --save-board <board>`: records `KICAD_CONFIG_HOME` and its arguments in `calls.log`,
/// counts its calls in `counter`, and writes the call's report to the `--output` path:
/// - with an `emulate_vias` file beside it, one `via_dangling` violation per via of the
///   board, none when the board's own project sets `via_dangling` to `ignore` (KiCad
///   reports a check the project does not ignore);
/// - else the call's canned report (`drc.<n>.json`, else `drc.default.json`).
///
/// The board is left as it is, or, when the call has a `board.<n>` file, that is saved
/// over it (as `--save-board` saves the refilled board).
const FAKE_CLI: &str = r#"#!/bin/sh
dir="$(cd "$(dirname "$0")" && pwd)"
echo "KICAD_CONFIG_HOME=${KICAD_CONFIG_HOME-unset} $*" >> "$dir/calls.log"
out=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--output" ]; then out="$a"; fi
  prev="$a"
done
n=$(cat "$dir/counter" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" > "$dir/counter"
board="$prev"
if [ -f "$dir/emulate_vias" ]; then
  project="${board%.kicad_pcb}.kicad_pro"
  vias=$(grep -E -c '^[[:space:]]*\(via([[:space:]]|$)' "$board")
  if [ -f "$project" ] && grep -q '"via_dangling": *"ignore"' "$project"; then vias=0; fi
  items=""
  i=0
  while [ "$i" -lt "$vias" ]; do
    if [ -n "$items" ]; then items="$items,"; fi
    items="$items{\"type\":\"via_dangling\",\"severity\":\"warning\",\"description\":\"a via\",\"items\":[]}"
    i=$((i + 1))
  done
  echo "{\"violations\":[$items],\"unconnected_items\":[]}" > "$out"
elif [ -f "$dir/drc.$n.json" ]; then cp "$dir/drc.$n.json" "$out"; else cp "$dir/drc.default.json" "$out"; fi
if [ -f "$dir/board.$n" ]; then cp "$dir/board.$n" "$board"; fi
exit 0
"#;

/// A KiCad DRC report: one unconnected item per net in `open`, one violation per rule in
/// `violations`.
fn drc(open: &[&str], violations: &[&str]) -> String {
    json!({
        "violations": violations
            .iter()
            .map(|rule| json!({
                "type": rule, "severity": "error", "description": "a violation", "items": []
            }))
            .collect::<Vec<_>>(),
        "unconnected_items": open
            .iter()
            .map(|net| json!({
                "type": "unconnected_items",
                "severity": "error",
                "description": "Missing connection between items",
                "items": [{
                    "description": format!("Pad 1 [{net}] of R1 on F.Cu"),
                    "pos": {"x": 1.0, "y": 2.0}
                }]
            }))
            .collect::<Vec<_>>(),
    })
    .to_string()
}

/// A report without the unconnected-items category: the judge's counts refuse it.
fn no_category() -> String {
    json!({ "violations": [] }).to_string()
}

const FIXTURE_SES: &str =
    include_str!("../tests/fixtures/specctra_rounded_board.freerouting-2.3.0.ses");

/// The fixture's session with `extra` written first in its GND net.
fn with_gnd(extra: &str) -> String {
    assert_eq!(FIXTURE_SES.matches("      (net GND\n").count(), 1);
    FIXTURE_SES.replacen("      (net GND\n", &format!("      (net GND\n{extra}"), 1)
}

/// A GND via at the start of the fixture's GND wire, with a 2 mm stub wire from it: one
/// via whose unit takes two wires (4.5 mm and 2 mm).
fn via_session() -> String {
    with_gnd(
        "        (via konnect_via_0001 1050000 -500000)
        (wire
          (path F.Cu 2000
            1050000 -500000
            1050000 -520000
          )
        )
",
    )
}

/// GND vias that no wire ends at, at `points` (session units).
fn lone_vias(points: &[(i64, i64)]) -> String {
    let extra: String = points
        .iter()
        .map(|(x, y)| format!("        (via konnect_via_0001 {x} {y})\n"))
        .collect();
    with_gnd(&extra)
}

/// The judge harness of these tests: the exported rounded board, the fake kicad-cli, a
/// private settings copy, and one routed run whose session is in its run folder.
struct H {
    _dir: tempfile::TempDir,
    ctx: JudgeContext,
    routed: Routed,
    fake: PathBuf,
}

impl H {
    fn new(default: String) -> H {
        let dir = tempfile::tempdir().unwrap();
        let Exported { dsn, ses } = exported(dir.path());
        let fake = dir.path().join("fake");
        std::fs::create_dir_all(&fake).unwrap();
        let cli = crate::tools::cli::test_support::write_script(&fake, "kicad-cli", FAKE_CLI, "");
        std::fs::write(fake.join("drc.default.json"), default).unwrap();
        let settings = dir.path().join("settings/10.0");
        put(&settings, "kicad_common.json", "{}");
        let config = copy_kicad_config(
            Some((settings, "10.0".to_string())),
            &dir.path().join("kicad-config"),
        )
        .unwrap()
        .home;
        let search = dir.path().join("search");
        let root = search.join("judge");
        let mut ctx = prepare(&dsn, &root, &cli.display().to_string()).expect("preflight");
        std::fs::create_dir_all(&root).unwrap();
        ctx.kicad_config_home = Some(config);
        let folder = search.join("run-000-as_exported-s0");
        std::fs::create_dir_all(&folder).unwrap();
        let held_ses = folder.join("full.ses");
        std::fs::copy(&ses, &held_ses).unwrap();
        let routed = Routed {
            index: 0,
            shape: Shape::AsExported,
            seed: 0,
            dsn,
            held: crate::fastroute::Held {
                ses: held_ses,
                unrouted: 0,
                kept: "full",
                passes: vec![],
                seconds: 2.5,
                untuned: None,
                tune: Default::default(),
                group_search: None,
            },
            planes: vec![],
            folder,
        };
        H {
            _dir: dir,
            ctx,
            routed,
            fake,
        }
    }

    fn session(&mut self, text: &str) -> &mut Self {
        std::fs::write(&self.routed.held.ses, text).unwrap();
        self
    }

    fn text(&self) -> String {
        std::fs::read_to_string(&self.routed.held.ses).unwrap()
    }

    /// The fake's report of its `call`-th run (from 1).
    fn canned(&self, call: usize, report: String) -> &Self {
        std::fs::write(self.fake.join(format!("drc.{call}.json")), report).unwrap();
        self
    }

    fn emulate_vias(&self) {
        std::fs::write(self.fake.join("emulate_vias"), "").unwrap();
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.fake.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Writes `own` as the board's project, with the hash the export records.
    fn project(&mut self, own: &str) -> PathBuf {
        let source = self.ctx.board.with_extension("kicad_pro");
        std::fs::write(&source, own).unwrap();
        let mut manifest: Value = serde_json::from_str(&self.ctx.manifest_text).unwrap();
        manifest["routing_context"] = json!({
            "project_sha256": format!("{:x}", Sha256::digest(own.as_bytes())),
            "custom_rules_sha256": null,
            "effective_rules_sha256": "0",
        });
        self.ctx.manifest_text = manifest.to_string();
        source
    }
}

fn clean_baseline() -> Baseline {
    Baseline {
        violations: BTreeMap::new(),
        unconnected: 0,
    }
}

/// An audit that passes G1 and scores, with `quality` as its copper quality and GND as
/// its one ground net.
fn audit_json(quality: f64) -> String {
    audit_with(quality, 1.0, &["GND"])
}

fn audit_with(quality: f64, routed_share: f64, ground: &[&str]) -> String {
    let ground: serde_json::Map<String, Value> = ground
        .iter()
        .map(|net| (net.to_string(), json!("name")))
        .collect();
    json!({
        "quality": { "routed_share": routed_share, "copper_quality": quality },
        "completion": { "complete": true },
        "metrics": {
            "5.3.acute_corners": 0,
            "5.3.right_angle_corners": 0,
            "3.2.unstitched_layer_changes": 0,
            "4.1.worst_loop_mm": 5.0,
            "4.1.median_loop_mm": 4.0,
            "4.1.mean_loop_area_mm2": 2.0,
        },
        "net_roles": { "ground": { "count": ground.len(), "nets": ground, "listed": ground.len() } },
        "checks": [
            { "item": "3.2", "score": 1.0, "metrics": {} },
            { "item": "4.1", "score": 1.0, "metrics": { "measured": 2, "pins": [] } },
        ],
    })
    .to_string()
}

type AuditScript = Box<dyn Fn(usize, &Path) -> (bool, String) + Send + Sync>;
type PlanScript = Box<dyn Fn(usize) -> (bool, String) + Send + Sync>;

/// The registry's tools as the prune sees them: `audit_layout_quality` answers from a
/// script (`audit(n, board)` for the n-th audit, from 0), or from the real registry when
/// `real` is set; `plan_route_refinement` answers `plan(n)`. Every call's tool name is
/// kept, in order.
struct Tools {
    audit: AuditScript,
    plan: PlanScript,
    real: Option<RegistryTools>,
    seen: Mutex<Vec<String>>,
}

impl Tools {
    fn audit(audit: impl Fn(usize, &Path) -> (bool, String) + Send + Sync + 'static) -> Tools {
        Tools {
            audit: Box::new(audit),
            plan: Box::new(|_| panic!("the planner must not be called")),
            real: None,
            seen: Mutex::new(Vec::new()),
        }
    }

    /// Every audit passes with copper quality 80.
    fn clean() -> Tools {
        Tools::audit(|_, _| (false, audit_json(80.0)))
    }

    fn planner(mut self, plan: impl Fn(usize) -> (bool, String) + Send + Sync + 'static) -> Self {
        self.plan = Box::new(plan);
        self
    }

    fn count(&self, name: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|seen| *seen == name)
            .count()
    }
}

impl ToolCaller for Tools {
    fn call<'a>(&'a self, name: &'a str, args: &'a Value) -> ToolCall<'a> {
        Box::pin(async move {
            let number = {
                let mut seen = self.seen.lock().unwrap();
                seen.push(name.to_string());
                seen.iter().filter(|seen| *seen == name).count() - 1
            };
            match name {
                "audit_layout_quality" => match &self.real {
                    Some(real) => real.call(name, args).await,
                    None => {
                        let board = PathBuf::from(args["board"].as_str().unwrap_or_default());
                        Ok((self.audit)(number, &board))
                    }
                },
                "plan_route_refinement" => Ok((self.plan)(number)),
                other => anyhow::bail!("the scripted tools have no '{other}'"),
            }
        })
    }
}

/// A planner result that proposes one stitching via on `net`.
fn one_via(net: &str) -> (bool, String) {
    (
        false,
        json!({
            "proposals": [{
                "id": "p",
                "kind": "stitching",
                "operations": [{
                    "tool": "route_batch",
                    "arguments": {
                        "board": "b",
                        "tracks": [],
                        "vias": [{"net_name": net, "x": 5.0, "y": 5.0, "drill": 0.3, "pad_size": 0.6}]
                    }
                }]
            }]
        })
        .to_string(),
    )
}

/// The severities of the scratch project beside `board`.
fn severities_beside(board: &Path) -> Value {
    let text = std::fs::read_to_string(board.with_extension("kicad_pro")).unwrap();
    serde_json::from_str::<Value>(&text).unwrap()["board"]["design_settings"]["rule_severities"]
        .clone()
}

// ─── routed_scratch and judge_light ──────────────────────────────────────────

#[tokio::test]
async fn routed_scratch_forces_the_prune_rules_only_when_asked() {
    // (the project's severities, what the prune's copy gets)
    let own = json!({"board": {"design_settings": {"rule_severities": {
        "track_dangling": "ignore",
        "via_dangling": "ignore",
        "starved_thermal": "error",
        "clearance": "ignore"
    }}}})
    .to_string();
    let mut h = H::new(drc(&[], &[]));
    let source = h.project(&own);
    let text = h.text();
    let dir = h.ctx.root.join("scratch-test");
    let scratch = routed_scratch(&h.ctx, &text, &dir, None, true)
        .await
        .unwrap_or_else(|error| panic!("{:#}", error.into_error()));
    assert_eq!(
        scratch.board,
        dir.join("r0/specctra_rounded_board.kicad_pcb")
    );
    let forced = severities_beside(&scratch.board);
    for key in ["track_dangling", "via_dangling", "isolated_copper"] {
        assert_eq!(forced[key], "warning", "{key}: {forced}");
    }
    // A rule already an error stays one; another check keeps the project's setting.
    assert_eq!(forced["starved_thermal"], "error");
    assert_eq!(forced["clearance"], "ignore");
    // The owner's project is never written.
    assert_eq!(std::fs::read_to_string(&source).unwrap(), own);

    // Without the flag the project is as judge_one leaves it (here: as the owner's).
    let scratch = routed_scratch(&h.ctx, &text, &dir, None, false)
        .await
        .unwrap_or_else(|error| panic!("{:#}", error.into_error()));
    let kept = severities_beside(&scratch.board);
    assert_eq!(kept["track_dangling"], "ignore");
    assert_eq!(kept["via_dangling"], "ignore");
    assert!(kept.get("isolated_copper").is_none(), "{kept}");
    // One refill a scratch, and the folder was cleared between them.
    assert_eq!(h.calls().len(), 2);
}

#[tokio::test]
async fn routed_scratch_tells_the_importers_refusal_from_every_other_error() {
    let h = H::new(drc(&[], &[]));
    let dir = h.ctx.root.join("scratch-test");
    std::fs::create_dir_all(dir.join("kept")).unwrap();
    match routed_scratch(&h.ctx, "(session broken)", &dir, None, true).await {
        Err(ScratchError::Importer(error)) => assert!(
            format!("{error:#}").contains("the importer refuses the run's session"),
            "{error:#}"
        ),
        Err(ScratchError::Other(error)) => panic!("not an importer error: {error:#}"),
        Ok(_) => panic!("a broken session was written"),
    }
    // Refused before anything was written: no kicad-cli, the folder as it was.
    assert!(h.calls().is_empty());
    assert!(dir.join("kept").is_dir());

    // A refill whose report cannot be counted is not the importer's.
    h.canned(1, "not json".to_string());
    match routed_scratch(&h.ctx, &h.text(), &dir, None, true).await {
        Err(ScratchError::Other(error)) => assert!(
            format!("{error:#}").contains("refill and DRC of the routed board"),
            "{error:#}"
        ),
        Err(ScratchError::Importer(error)) => panic!("not the importer's: {error:#}"),
        Ok(_) => panic!("an unreadable report was taken"),
    }
}

#[tokio::test]
async fn judge_light_audits_once_and_runs_no_stitching_and_no_probe() {
    let mut h = H::new(drc(&["GND"], &[]));
    // A call with planes and length checks: the full judge would stitch and probe.
    h.routed.planes = vec!["GND".to_string()];
    h.ctx.lengths = Some(Arc::new(crate::length_probe::LengthChecks {
        board_rules: true,
        ..Default::default()
    }));
    let text = h.text();
    let dir = h.ctx.root.join("light");
    let scratch = routed_scratch(&h.ctx, &text, &dir, None, true)
        .await
        .unwrap_or_else(|error| panic!("{:#}", error.into_error()));
    let tools = Tools::clean();
    let (measures, gates, audit) = judge_light(&h.ctx, &clean_baseline(), &tools, &scratch, 7.5)
        .await
        .unwrap();
    assert_eq!(tools.count("audit_layout_quality"), 1);
    assert_eq!(tools.count("plan_route_refinement"), 0);
    assert_eq!(h.calls().len(), 1, "the scratch's refill only: no probe");
    assert!(!dir.join(crate::length_probe::PROBE_FOLDER).exists());
    assert_eq!(measures.stitching_vias, 0);
    assert_eq!(measures.route_s, 7.5);
    assert!(measures.lengths.is_none() && measures.pours.is_none());
    assert_eq!(measures.unconnected, 1, "the scratch's own report");
    assert_eq!(gates, super::gates(&measures, &clean_baseline()));
    assert_eq!(audit["quality"]["copper_quality"], 80.0);
}

// ─── JudgeChecker ────────────────────────────────────────────────────────────

fn checked(outcome: CheckOutcome) -> Checked {
    match outcome {
        CheckOutcome::Checked(checked) => checked,
        CheckOutcome::ImporterRefused(why) => panic!("the importer refused: {why}"),
    }
}

/// P7. Control: the DRC stop removed, the audit runs on the refused candidate.
#[tokio::test]
async fn the_audit_is_not_run_after_a_drc_refusal() {
    let mut h = H::new(drc(&[], &[]));
    h.session(&via_session());
    // Call 2 is the first candidate: one clearance violation more than the original.
    h.canned(2, drc(&[], &["clearance"]));
    let base = clean_baseline();
    let tools = Tools::clean();
    let text = h.text();
    let checker = JudgeChecker::new(&h.ctx, &base, &tools, &text, 0, 2.5).unwrap();
    let original = checked(checker.check(&text, None).await.unwrap());
    assert!(original.rest.is_some(), "the original's check is whole");
    assert_eq!(tools.count("audit_layout_quality"), 1);

    let candidate = FIXTURE_SES;
    let refused = checked(checker.check(candidate, Some(&original)).await.unwrap());
    assert_eq!(refused.violations.get("clearance"), Some(&1));
    assert!(refused.rest.is_none(), "stopped at the DRC");
    assert_eq!(
        tools.count("audit_layout_quality"),
        1,
        "no audit after a DRC refusal"
    );

    let allowed = checked(checker.check(candidate, Some(&original)).await.unwrap());
    assert!(allowed.rest.is_some());
    assert_eq!(tools.count("audit_layout_quality"), 2);
    assert_eq!((checker.kicad_runs(), checker.audits()), (3, 2));
    // Every check on the checker's own settings copy, in the prune's scratch folder.
    let own = h
        .ctx
        .root
        .join("kicad-config/run-0-prune")
        .display()
        .to_string();
    for call in h.calls() {
        assert!(
            call.starts_with(&format!("KICAD_CONFIG_HOME={own} ")),
            "{call}"
        );
        assert!(call.contains("/judge/run-0-prune/c/r0/"), "{call}");
    }
    drop(checker);
    assert!(!h.ctx.root.join("kicad-config/run-0-prune").exists());
}

/// P10: a candidate TEXT the importer refuses is a refused candidate; when the original
/// no longer passes either, it is an error.
#[tokio::test]
async fn an_importer_refusal_is_a_refused_candidate_unless_the_original_fails_too() {
    let mut h = H::new(drc(&[], &[]));
    h.project(r#"{"board": {}}"#);
    let base = clean_baseline();
    let tools = Tools::clean();
    let text = h.text();
    let checker = JudgeChecker::new(&h.ctx, &base, &tools, &text, 0, 2.5).unwrap();
    let original = checked(checker.check(&text, None).await.unwrap());
    match checker
        .check("(session broken)", Some(&original))
        .await
        .unwrap()
    {
        CheckOutcome::ImporterRefused(why) => {
            assert!(why.contains("the importer refuses"), "{why}")
        }
        CheckOutcome::Checked(_) => panic!("a broken text was checked"),
    }
    assert_eq!(h.calls().len(), 1, "no kicad-cli for a refused text");

    // The board's project changes on disk: the original fails too, and that is an error.
    std::fs::write(
        h.ctx.board.with_extension("kicad_pro"),
        r#"{"board": {"x": 1}}"#,
    )
    .unwrap();
    let error = checker
        .check("(session broken)", Some(&original))
        .await
        .expect_err("an error");
    assert!(
        format!("{error:#}").contains("changed during the prune"),
        "{error:#}"
    );
}

/// P10 in the loop: a board that changes on disk mid-prune ends the loop `check_error`
/// with the state accepted so far.
#[tokio::test]
async fn a_board_that_changes_mid_prune_ends_the_loop_with_the_state_accepted_so_far() {
    let mut h = H::new(drc(&[], &[]));
    h.session(&lone_vias(&[(1000000, -550000), (1100000, -550000)]));
    let project = h.project(r#"{"board": {}}"#);
    let base = clean_baseline();
    // The second audit (the first candidate's) changes the project after its check.
    let tools = Tools::audit(move |n, _| {
        if n == 1 {
            std::fs::write(&project, r#"{"board": {"changed": true}}"#).unwrap();
        }
        (false, audit_json(80.0))
    });
    let text = h.text();
    let nets = set(&["GND"]);
    let units = units(&text, &nets).unwrap();
    let checker = JudgeChecker::new(&h.ctx, &base, &tools, &text, 0, 2.5).unwrap();
    let original = checked(checker.check(&text, None).await.unwrap());
    let pruned = pour_prune::prune_loop(&text, &units, &original, &checker).await;
    assert_eq!(pruned.ended_by, "check_error");
    assert!(
        pruned
            .error
            .as_deref()
            .is_some_and(|error| error.contains("changed during the prune")),
        "{:?}",
        pruned.error
    );
    let first_via = units
        .iter()
        .position(|unit| unit.kind == pour_prune::UnitKind::Via)
        .unwrap();
    assert_eq!(pruned.removed, BTreeSet::from([first_via]));
    assert_eq!(pruned.checks, 2);
}

/// P11: the original has one dangling via in a project that ignores `via_dangling`; the
/// prune's copy reports it, so a candidate that removes it is accepted and one that adds
/// another is refused. Control: the prune rules not forced, the added via goes through.
#[tokio::test]
async fn the_prunes_project_reports_dangling_vias_whatever_the_project_says() {
    let mut h = H::new(drc(&[], &[]));
    h.session(&lone_vias(&[(1000000, -550000)]));
    h.project(r#"{"board": {"design_settings": {"rule_severities": {"via_dangling": "ignore"}}}}"#);
    h.emulate_vias();
    let base = clean_baseline();
    let tools = Tools::clean();
    let text = h.text();
    let checker = JudgeChecker::new(&h.ctx, &base, &tools, &text, 0, 2.5).unwrap();
    let original = checked(checker.check(&text, None).await.unwrap());
    assert_eq!(original.violations.get("via_dangling"), Some(&1));

    let units = units(&text, &set(&["GND"])).unwrap();
    let pruned = pour_prune::prune_loop(&text, &units, &original, &checker).await;
    let via = units
        .iter()
        .position(|unit| unit.kind == pour_prune::UnitKind::Via)
        .unwrap();
    assert!(pruned.removed.contains(&via), "{pruned:?}");
    assert_eq!(pruned.last.violations.get("via_dangling"), None);

    let two = lone_vias(&[(1000000, -550000), (1000000, -560000)]);
    let added = checked(checker.check(&two, Some(&original)).await.unwrap());
    assert_eq!(
        pour_prune::refusals(&original, &original, &added),
        vec!["rule:via_dangling"]
    );
}

// ─── prune_winner ────────────────────────────────────────────────────────────

/// A finished search whose one run, the harness's, was judged in full with `tools` and
/// won.
async fn searched(h: &H, tools: &Tools) -> SearchOutcome {
    let base = clean_baseline();
    let judged = judge_session(&h.ctx, &h.routed, &base, tools, &h.routed.held.ses, "run-0")
        .await
        .unwrap();
    assert!(judged.gates.all_pass(), "{:?}", judged.gates);
    let key = rank(&[&judged])[0];
    SearchOutcome {
        runs: vec![RunRecord {
            index: 0,
            shape: Shape::AsExported,
            seed: 0,
            launched_s: 0.0,
            routed_s: Some(1.0),
            judged_s: Some(2.0),
            route_s: Some(1.0),
            judge_s: Some(1.0),
            outcome: "judged",
            reason: None,
            routed: Some(h.routed.clone()),
            judged: Some(judged),
        }],
        winner: Some(0),
        keys: vec![Some(key)],
        elapsed: Duration::from_secs(5),
        budget_overrun: None,
        stopped_launching: Some("fixed run count".to_string()),
        baseline: Ok(base),
        pour_prune: None,
    }
}

/// What the prune may change of an outcome: every run's record, session and judge, the
/// winner and the keys.
fn outcome_json(outcome: &SearchOutcome) -> Value {
    json!({
        "runs": outcome.runs.iter().map(|record| json!({
            "record": record,
            "ses": record.routed.as_ref().map(|routed| routed.held.ses.clone()),
            "judged": record.judged,
        })).collect::<Vec<_>>(),
        "winner": outcome.winner,
        "keys": outcome.keys,
    })
}

async fn prune(h: &H, tools: &Arc<Tools>, outcome: &mut SearchOutcome) -> PruneRecord {
    prune_winner_with(&h.ctx, tools.clone(), outcome).await;
    outcome.pour_prune.clone().expect("always set")
}

fn pruned_ses(h: &H) -> PathBuf {
    h.routed.folder.join("pruned.ses")
}

/// The files of a prune that was not kept are gone, and so is its scratch folder.
fn assert_no_prune_files(h: &H) {
    assert!(!pruned_ses(h).exists(), "pruned.ses was left");
    assert!(
        !h.ctx.root.join("run-0-pruned").exists(),
        "the -pruned judge was left"
    );
    assert!(
        !h.ctx.root.join("run-0-prune").exists(),
        "the scratch folder was left"
    );
}

#[tokio::test]
async fn a_kept_prune_replaces_the_winners_session_and_judge() {
    let mut h = H::new(drc(&[], &[]));
    h.session(&via_session());
    let tools = Arc::new(Tools::clean());
    let mut outcome = searched(&h, &tools).await;
    let original_ses = h.routed.held.ses.clone();
    let original_board = outcome.runs[0].judged.as_ref().unwrap().board.clone();
    let tune = TuneChoice {
        chosen: None,
        rows: vec![json!({ "variant": "free", "reason": "carried" })],
        untuned: measures(),
    };
    outcome.runs[0].judged.as_mut().unwrap().tune = Some(tune.clone());
    // A stale key: the prune takes the keys again.
    let stale: Key = [7; KEY_LEN];
    outcome.keys = vec![Some(stale)];
    let text = h.text();

    let record = prune(&h, &tools, &mut outcome).await;
    assert_eq!(record.outcome, "pruned", "{record:?}");
    assert_eq!(record.reason, None);
    assert_eq!(record.nets, vec!["GND"]);
    assert_eq!(
        (
            record.vias_before,
            record.vias_removed,
            record.wires_removed
        ),
        (1, 1, 2)
    );
    assert_eq!(record.track_mm_removed, 6.5);
    assert_eq!(record.ended_by, Some("done"));
    // The original's check, one candidate, the full judge.
    assert_eq!((record.checks, record.audits), (3, 3));
    assert_eq!(h.calls().len(), 4, "the setup judge and the prune's three");

    let routed = outcome.runs[0].routed.as_ref().unwrap();
    assert_eq!(routed.held.ses, pruned_ses(&h));
    let units = units(&text, &set(&["GND"])).unwrap();
    let every: BTreeSet<usize> = (0..units.len()).collect();
    assert_eq!(
        std::fs::read_to_string(pruned_ses(&h)).unwrap(),
        without(&text, &units, &every)
    );
    assert_eq!(
        std::fs::read_to_string(&original_ses).unwrap(),
        text,
        "the original stays"
    );
    let judged = outcome.runs[0].judged.as_ref().unwrap();
    assert!(judged.board.starts_with(h.ctx.root.join("run-0-pruned")));
    assert!(judged.board.is_file());
    assert_eq!(judged.tune, Some(tune));
    assert_eq!(outcome.keys, vec![Some(rank(&[judged])[0])]);
    assert_eq!(outcome.winner, Some(0));
    assert!(
        !h.ctx.root.join("run-0-prune").exists(),
        "the scratch folder goes"
    );
    assert!(
        original_board.is_file(),
        "the original's judged board stays"
    );
    assert_eq!(
        record.before,
        Some(json!({
            "session": original_ses.to_string_lossy(),
            "key": stale,
            "judged_board": original_board.to_string_lossy(),
        }))
    );
    let shown = serde_json::to_value(&record).unwrap();
    assert!(shown.get("reason").is_none(), "{shown}");
    assert_eq!(shown["vias_kept_with"], json!({}));
}

/// P9, each clause: a pruned session whose full judge fails a gate, writes a stitching
/// round, or has a higher key entry is `rejected`; its files go and the outcome is as it
/// was. The control of each clause is the clause removed (controls.md).
#[tokio::test]
async fn a_pruned_session_the_full_judge_refuses_is_rejected_and_changes_nothing() {
    // Audits: 0 the setup judge, 1 the original's check, 2 the candidate, 3 the full judge.
    let cases: Vec<(&str, Tools, Vec<String>)> = vec![
        (
            "fails a gate",
            Tools::audit(|n, _| {
                (
                    false,
                    audit_with(80.0, if n == 3 { 0.5 } else { 1.0 }, &["GND"]),
                )
            }),
            vec![],
        ),
        (
            "rank key is above the original's at position 11",
            Tools::audit(|n, _| (false, audit_json(if n == 3 { 70.0 } else { 80.0 }))),
            vec![],
        ),
        (
            "wrote 1 stitching round",
            Tools::clean().planner(|_| one_via("GND")),
            // The full judge's first refill (call 4) leaves GND open; the stitched board
            // (call 5) closes it.
            vec![drc(&["GND"], &[])],
        ),
    ];
    for (wanted, tools, full_refill) in cases {
        let mut h = H::new(drc(&[], &[]));
        h.session(&via_session());
        h.routed.planes = vec!["GND".to_string()];
        for report in full_refill {
            h.canned(4, report);
        }
        let tools = Arc::new(tools);
        let mut outcome = searched(&h, &tools).await;
        let before = outcome_json(&outcome);
        let record = prune(&h, &tools, &mut outcome).await;
        assert_eq!(record.outcome, "rejected", "{wanted}: {record:?}");
        assert!(
            record
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains(wanted)),
            "{wanted}: {:?}",
            record.reason
        );
        assert_eq!(
            record.vias_removed, 1,
            "what the loop removed is still said"
        );
        assert_eq!(record.before, None);
        assert_eq!(outcome_json(&outcome), before, "{wanted}");
        assert_no_prune_files(&h);
    }
}

/// P8: a winner with planes and no stitching round is pruned; one the judge stitched is
/// skipped. Control: the round clause removed, the stitched winner is pruned.
#[tokio::test]
async fn a_winner_with_planes_is_pruned_and_a_stitched_winner_is_skipped() {
    let mut h = H::new(drc(&[], &[]));
    h.session(&via_session());
    h.routed.planes = vec!["GND".to_string()];
    let tools = Arc::new(Tools::clean());
    let mut outcome = searched(&h, &tools).await;
    assert!(outcome.runs[0].judged.as_ref().unwrap().rounds.is_empty());
    let record = prune(&h, &tools, &mut outcome).await;
    assert_eq!(record.outcome, "pruned", "{record:?}");

    let mut h = H::new(drc(&[], &[]));
    h.session(&via_session());
    h.routed.planes = vec!["GND".to_string()];
    let mut outcome = searched(&h, &tools).await;
    outcome.runs[0].judged.as_mut().unwrap().rounds = vec![
        json!({ "round": 1, "open_before": 1, "proposals": 1, "written_vias": 1, "open_after": 0 }),
    ];
    let before = outcome_json(&outcome);
    let calls = h.calls().len();
    let record = prune(&h, &tools, &mut outcome).await;
    assert_eq!(record.outcome, "skipped");
    assert_eq!(
        record.reason.as_deref(),
        Some("the judge wrote 1 stitching round(s) for the winner")
    );
    assert_eq!(h.calls().len(), calls, "a skip runs no kicad-cli");
    assert_eq!(outcome_json(&outcome), before);
    assert_eq!(record.checks, 0);
}

#[tokio::test]
async fn the_other_skips_say_why_and_change_nothing() {
    // (why, how the harness or the outcome is changed)
    type Change = Box<dyn Fn(&mut H, &mut SearchOutcome)>;
    let cases: Vec<(&str, Change)> = vec![
        (
            "the call has no pour net",
            Box::new(|h: &mut H, _: &mut SearchOutcome| h.ctx.pour_nets.clear()),
        ),
        (
            "the winner fails a gate",
            Box::new(|_: &mut H, outcome: &mut SearchOutcome| {
                outcome.runs[0].judged.as_mut().unwrap().gates.g2 = vec!["clearance".to_string()];
            }),
        ),
        (
            "no run won",
            Box::new(|_: &mut H, outcome: &mut SearchOutcome| outcome.winner = None),
        ),
        (
            "no via on a ground pour net",
            Box::new(|h: &mut H, _: &mut SearchOutcome| {
                h.session(FIXTURE_SES);
            }),
        ),
    ];
    for (why, change) in cases {
        let mut h = H::new(drc(&[], &[]));
        h.session(&via_session());
        let tools = Arc::new(Tools::clean());
        let mut outcome = searched(&h, &tools).await;
        change(&mut h, &mut outcome);
        let before = outcome_json(&outcome);
        let record = prune(&h, &tools, &mut outcome).await;
        assert_eq!(record.outcome, "skipped", "{why}: {record:?}");
        assert_eq!(record.reason.as_deref(), Some(why));
        assert_eq!(outcome_json(&outcome), before, "{why}");
        assert_no_prune_files(&h);
    }
}

/// P3: the audit's role list decides, not the name. A pour net named like a ground (VSS)
/// that the design intent makes a rail at -5 V is left alone and named. Control: without
/// the design intent the audit calls it ground by its name and its vias are tried.
#[tokio::test]
async fn a_pour_net_the_design_intent_makes_power_is_left_alone() {
    for (intent, tried) in [(true, false), (false, true)] {
        let mut h = H::renamed("GND", "VSS", drc(&[], &[]));
        h.session(&via_session().replace("(net GND", "(net VSS"));
        if intent {
            let folder = h.ctx.board.parent().unwrap().join(".konnect");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(
                folder.join("design_intent.json"),
                json!({ "schema_version": 1, "currents": [], "interfaces": [],
                    "rails": [{ "name": "NEG5", "nets": ["VSS"], "voltage_v": -5.0 }] })
                .to_string(),
            )
            .unwrap();
        }
        assert!(h.ctx.pour_nets.contains("VSS"), "{:?}", h.ctx.pour_nets);
        let tools = Arc::new(Tools::clean());
        let mut outcome = searched(&h, &tools).await;
        // The prune's checks get the real audit's roles.
        let real = Tools {
            real: Some(RegistryTools(tool_context(&h.ctx.kicad_cli))),
            ..Tools::clean()
        };
        let real = Arc::new(real);
        let record = prune(&h, &real, &mut outcome).await;
        if tried {
            assert_eq!(record.nets, vec!["VSS"], "{record:?}");
            assert_eq!(record.vias_before, 1);
        } else {
            assert_eq!(record.outcome, "skipped", "{record:?}");
            assert_eq!(
                record.reason.as_deref(),
                Some("no pour net of the call is ground by the audit's net roles")
            );
            assert!(record.nets.is_empty());
            assert!(
                record.nets_left_alone.contains(&"VSS".to_string()),
                "{record:?}"
            );
        }
    }
}

#[tokio::test]
async fn a_failed_original_check_delivers_the_original() {
    let mut h = H::new(drc(&[], &[]));
    h.session(&via_session());
    // Call 1 is the setup judge, 2 the original's check.
    h.canned(2, no_category());
    let tools = Arc::new(Tools::clean());
    let mut outcome = searched(&h, &tools).await;
    let before = outcome_json(&outcome);
    let record = prune(&h, &tools, &mut outcome).await;
    assert_eq!(record.outcome, "failed", "{record:?}");
    assert!(
        record.reason.as_deref().is_some_and(
            |reason| reason.starts_with("the check of the winner's own session failed")
        ),
        "{:?}",
        record.reason
    );
    assert_eq!(outcome_json(&outcome), before);
    assert_no_prune_files(&h);
    assert_eq!(
        (record.checks, record.audits),
        (1, 0),
        "the refill ran, its report could not be counted"
    );
}

#[tokio::test]
async fn a_loop_that_errs_before_any_removal_removes_nothing_and_says_why() {
    let mut h = H::new(drc(&[], &[]));
    h.session(&via_session());
    // Call 3 is the first candidate.
    h.canned(3, no_category());
    let tools = Arc::new(Tools::clean());
    let mut outcome = searched(&h, &tools).await;
    let before = outcome_json(&outcome);
    let record = prune(&h, &tools, &mut outcome).await;
    assert_eq!(record.outcome, "nothing_removed", "{record:?}");
    assert_eq!(record.ended_by, Some("check_error"));
    assert!(
        record
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("unconnected-items category")),
        "{:?}",
        record.reason
    );
    assert_eq!(outcome_json(&outcome), before);
    assert_no_prune_files(&h);
}

/// Never a panic for the call. Control: the prune run in place of its own task, the
/// panic is the call's.
#[tokio::test]
async fn a_panic_in_the_prune_is_a_failed_record_and_the_original_is_delivered() {
    let mut h = H::new(drc(&[], &[]));
    h.session(&via_session());
    let tools = Arc::new(Tools::audit(|n, _| {
        assert!(n < 2, "a scripted audit that breaks on the candidate");
        (false, audit_json(80.0))
    }));
    let mut outcome = searched(&h, &tools).await;
    let before = outcome_json(&outcome);
    let record = prune(&h, &tools, &mut outcome).await;
    assert_eq!(record.outcome, "failed", "{record:?}");
    assert!(
        record
            .reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("the prune stopped")),
        "{:?}",
        record.reason
    );
    assert_eq!(outcome_json(&outcome), before);
    assert_no_prune_files(&h);
}

impl H {
    /// [`H::new`] on the rounded board with the net `from` renamed `to` everywhere: the
    /// board, its export and the recorded session.
    fn renamed(from: &str, to: &str, default: String) -> H {
        let mut h = H::new(default);
        let folder = h.ctx.board.parent().unwrap().to_path_buf();
        let board_text = include_str!("../tests/fixtures/specctra_pours_policy.kicad_pcb")
            .replace(&format!("\"{from}\""), &format!("\"{to}\""));
        assert_ne!(
            board_text,
            include_str!("../tests/fixtures/specctra_pours_policy.kicad_pcb")
        );
        export_renamed(&folder, &board_text, &[to, "VCC"]);
        let cli = h.ctx.kicad_cli.clone();
        let config = h.ctx.kicad_config_home.clone();
        let mut ctx = prepare(
            &folder.join("specctra_rounded_board.dsn"),
            &h.ctx.root,
            &cli,
        )
        .expect("preflight");
        ctx.kicad_config_home = config;
        h.ctx = ctx;
        h
    }
}

/// [`exported`]'s export of `board_text` (saved in `folder` as the rounded board) with
/// routing rules for `nets`.
fn export_renamed(folder: &Path, board_text: &str, nets: &[&str]) {
    use crate::specctra::{export_dsn_with_options, ExportOptions, PourPolicy};
    use konnect_ipc::{IpcEffectiveRoutingRules, IpcRoutingRules};
    let board = folder.join("specctra_rounded_board.kicad_pcb");
    std::fs::write(&board, board_text).unwrap();
    let rules: IpcEffectiveRoutingRules = nets
        .iter()
        .map(|net| {
            (
                net.to_string(),
                IpcRoutingRules {
                    class_name: "Default".into(),
                    constituents: vec!["Default".into()],
                    track_width_mm: Some(0.2),
                    clearance_mm: Some(0.2),
                    via_diameter_mm: Some(0.6),
                    via_drill_mm: Some(0.3),
                },
            )
        })
        .collect();
    let bundle = export_dsn_with_options(
        &board,
        board_text,
        &rules,
        ExportOptions {
            pours: PourPolicy::from_argument(Some(&json!({
                "zones": [{ "name": "GND_TOP", "policy": "omit" }]
            })))
            .unwrap(),
            ..Default::default()
        },
    )
    .unwrap();
    std::fs::write(folder.join("specctra_rounded_board.dsn"), &bundle.dsn).unwrap();
    std::fs::write(
        folder.join("specctra_rounded_board.dsn.konnect.json"),
        &bundle.manifest,
    )
    .unwrap();
}
