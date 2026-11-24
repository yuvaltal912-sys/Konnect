//! Row ZR, ZR6: the reference round inside the judge and the delivery of its vias
//! (`pour_stitch::reference_round`, `deliver_winner`), with the prune tests' fake
//! kicad-cli and judge harness, a scripted audit and a scripted reference planner (no
//! KiCad). Each guard has a negative control in the stage's records.

use super::*;
use crate::pour_stitch::{
    deliver_winner, undelivered_nets, DeliveryRecord, PlanFuture, ReferencePlanner,
    ReferenceStitch, StitchRecord,
};

/// The fixture's GND wire runs from (105, 50) to (109.5, 50) on F.Cu; the signal via the
/// scripted audit names sits beside it, and the planner's via at (106, 52) serves it.
const SIGNAL_AT: (f64, f64) = (105.5, 51.0);

/// A 3.2 finding on `nets` at `at`.
fn finding(subject: &str, nets: &[&str], at: (f64, f64)) -> Value {
    json!({
        "item": "3.2",
        "subject": subject,
        "measured": {
            "reference_nets": nets,
            "reference_layers": ["F.Cu", "B.Cu"],
            "nearest_stitch_mm": 5.5,
        },
        "threshold": { "stitch_via_max_mm": 3.0 },
        "at": { "x": at.0, "y": at.1 },
    })
}

/// The prune tests' clean audit (GND ground, quality `quality`), with `count` unstitched
/// layer changes and `findings` listed.
fn audit(count: usize, findings: Vec<Value>, quality: f64) -> String {
    let mut body: Value = serde_json::from_str(&audit_with(quality, 1.0, &["GND"])).unwrap();
    body["metrics"]["3.2.unstitched_layer_changes"] = json!(count);
    body["metrics"]["3.1.length_off_reference_mm"] = json!(12.5);
    body["findings"] = json!(findings);
    body.to_string()
}

/// What the scripted audit answers for a board: one unstitched GND change at
/// [`SIGNAL_AT`] unless the board holds a via at (106, 52).
fn served(board: &Path) -> String {
    let text = std::fs::read_to_string(board).unwrap_or_default();
    if text.contains("(at 106 52)") {
        audit(0, vec![], 80.0)
    } else {
        audit(1, vec![finding("SIG via 1", &["GND"], SIGNAL_AT)], 80.0)
    }
}

/// A planner result proposing GND vias at `points`, each `size` mm wide.
fn proposal(points: &[(f64, f64)], size: f64) -> Value {
    let vias: Vec<Value> = points
        .iter()
        .map(|(x, y)| json!({"net_name": "GND", "x": x, "y": y, "drill": 0.3, "pad_size": size}))
        .collect();
    json!({
        "proposals": [{
            "id": "p",
            "kind": "stitching",
            "operations": [{ "tool": "route_batch", "arguments": { "board": "b", "tracks": [], "vias": vias } }]
        }],
        "blocked": [{ "kind": "stitching", "reason": "2 reference changes remain; no safe candidate within the search bounds", "objects": [] }],
    })
}

type Audit = Box<dyn Fn(usize, &Path, &Value) -> (bool, String) + Send + Sync>;
type Plan = Box<dyn Fn(usize) -> Result<Value> + Send + Sync>;

/// The tools the round sees: a scripted audit (`audit(n, board, args)`), no registry
/// planner (Z9 is not called: the harness's run has no plane net), and a scripted
/// reference planner (`plan(n)`); every audit's arguments and every planner call are kept.
struct Calls {
    audit: Audit,
    plan: Plan,
    audits: Mutex<Vec<Value>>,
    plans: Mutex<Vec<(PathBuf, Vec<String>, f64)>>,
}

impl Calls {
    fn new(
        audit: impl Fn(usize, &Path, &Value) -> (bool, String) + Send + Sync + 'static,
        plan: impl Fn(usize) -> Result<Value> + Send + Sync + 'static,
    ) -> Calls {
        Calls {
            audit: Box::new(audit),
            plan: Box::new(plan),
            audits: Mutex::new(Vec::new()),
            plans: Mutex::new(Vec::new()),
        }
    }

    /// The served audit and one via at (106, 52).
    fn served() -> Calls {
        Calls::new(
            |_, board, _| (false, served(board)),
            |_| Ok(proposal(&[(106.0, 52.0)], 0.6)),
        )
    }

    fn plans(&self) -> Vec<(PathBuf, Vec<String>, f64)> {
        self.plans.lock().unwrap().clone()
    }
}

impl ToolCaller for Calls {
    fn call<'a>(&'a self, name: &'a str, args: &'a Value) -> ToolCall<'a> {
        Box::pin(async move {
            match name {
                "audit_layout_quality" => {
                    let number = {
                        let mut audits = self.audits.lock().unwrap();
                        audits.push(args.clone());
                        audits.len() - 1
                    };
                    let board = PathBuf::from(args["board"].as_str().unwrap_or_default());
                    Ok((self.audit)(number, &board, args))
                }
                other => anyhow::bail!("the scripted tools have no '{other}'"),
            }
        })
    }

    fn reference_planner(&self) -> &dyn ReferencePlanner {
        self
    }
}

impl ReferencePlanner for Calls {
    fn plan<'a>(&'a self, board: &'a Path, nets: &'a [String], radius_mm: f64) -> PlanFuture<'a> {
        Box::pin(async move {
            let number = {
                let mut plans = self.plans.lock().unwrap();
                plans.push((board.to_path_buf(), nets.to_vec(), radius_mm));
                plans.len() - 1
            };
            (self.plan)(number)
        })
    }
}

fn on(default: String) -> H {
    let mut h = H::new(default);
    h.ctx.reference_stitch = ReferenceStitch::On;
    h
}

async fn judge(h: &H, tools: &Calls) -> Judged {
    judge_session(
        &h.ctx,
        &h.routed,
        &clean_baseline(),
        tools,
        &h.routed.held.ses,
        "run-0",
    )
    .await
    .unwrap()
}

fn record(judged: &Judged) -> &StitchRecord {
    judged.reference_stitch.as_ref().expect("the round ran")
}

// ─── The round in the judge (S1 to S7) ───────────────────────────────────────

#[tokio::test]
async fn a_kept_round_is_the_judged_board_and_its_record_holds_both_checks() {
    let h = on(drc(&[], &[]));
    let tools = Calls::served();
    let judged = judge(&h, &tools).await;
    let round = record(&judged);
    assert_eq!(round.outcome, "kept", "{round:?}");
    assert_eq!(
        (
            round.targets,
            round.proposals,
            round.written_vias,
            round.dropped
        ),
        (1, 1, 1, 0)
    );
    assert_eq!(round.kicad_runs, 1);
    assert_eq!(round.retried, None);
    assert!(round.remaining.is_empty(), "{:?}", round.remaining);
    assert_eq!(round.vias.len(), 1);
    assert_eq!((round.vias[0].x_mm, round.vias[0].y_mm), (106.0, 52.0));
    assert_eq!(round.before.checklist.unstitched_layer_changes, Some(1.0));
    let after = round.after.as_ref().unwrap();
    assert_eq!(after.checklist.unstitched_layer_changes, Some(0.0));
    assert_eq!(after.key_routing, round.before.key_routing);
    // The judged board is the stitched one; the post-Z9 board's folder went.
    assert!(
        judged
            .board
            .ends_with("run-0/s1/specctra_rounded_board.kicad_pcb"),
        "{}",
        judged.board.display()
    );
    assert!(!h.ctx.root.join("run-0/r0").exists());
    assert_eq!(judged.measures.reference_vias, 1);
    assert_eq!(
        judged.measures.checklist.unstitched_layer_changes,
        Some(0.0)
    );
    assert!(judged.rounds.is_empty(), "the round's rows are not Z9's");
    // The kept check carries the run's final measures and gates.
    let checked = judged.reference_checked.as_ref().unwrap();
    assert_eq!(checked.rest.as_ref().unwrap().measures, judged.measures);
    assert_eq!(checked.rest.as_ref().unwrap().gates, judged.gates);
    // One planner call: the target's net, its limit, the post-Z9 board.
    let plans = tools.plans();
    assert_eq!(plans.len(), 1);
    assert!(plans[0]
        .0
        .ends_with("run-0/r0/specctra_rounded_board.kicad_pcb"));
    assert_eq!(
        (plans[0].1.clone(), plans[0].2),
        (vec!["GND".to_string()], 3.0)
    );
    // The refill of r0 and of s1.
    assert_eq!(h.calls().len(), 2);
    // The record is in the run's JSON.
    let body = json!(judged);
    assert_eq!(body["reference_stitch"]["outcome"], "kept");
    assert_eq!(body["measures"]["reference_vias"], 1);
    assert!(body.get("reference_checked").is_none());
}

#[tokio::test]
async fn with_the_switch_off_the_judge_runs_no_round_and_records_the_checklist() {
    let h = H::new(drc(&[], &[]));
    assert_eq!(
        h.ctx.reference_stitch,
        ReferenceStitch::Off,
        "prepare gives off"
    );
    let tools = Calls::new(
        |_, board, _| (false, served(board)),
        |_| panic!("the planner must not be called"),
    );
    let judged = judge(&h, &tools).await;
    assert_eq!(judged.reference_stitch, None);
    assert_eq!(judged.reference_checked, None);
    assert_eq!(judged.measures.reference_vias, 0);
    assert_eq!(
        judged.measures.checklist.unstitched_layer_changes,
        Some(1.0)
    );
    assert_eq!(
        judged.measures.checklist.length_off_reference_mm,
        Some(12.5)
    );
    assert!(judged
        .board
        .ends_with("run-0/r0/specctra_rounded_board.kicad_pcb"));
    assert_eq!(h.calls().len(), 1);
    assert_eq!(tools.audits.lock().unwrap().len(), 1);
    assert!(json!(judged).get("reference_stitch").is_none());
}

#[tokio::test]
async fn a_board_with_nothing_unstitched_pays_nothing() {
    let h = on(drc(&[], &[]));
    let tools = Calls::new(
        |_, _, _| (false, audit(0, vec![], 80.0)),
        |_| panic!("the planner must not be called"),
    );
    let judged = judge(&h, &tools).await;
    assert_eq!(judged.reference_stitch, None);
    assert_eq!(h.calls().len(), 1);
}

#[tokio::test]
async fn a_refusal_that_names_no_via_keeps_the_post_z9_board() {
    let h = on(drc(&[], &[]));
    // The stitched board's DRC has a clearance item far from the via.
    let mut far: Value = serde_json::from_str(&drc(&[], &["clearance"])).unwrap();
    far["violations"][0]["items"] =
        json!([{ "description": "a track", "pos": { "x": 0.0, "y": 0.0 } }]);
    h.canned(2, far.to_string());
    let tools = Calls::served();
    let judged = judge(&h, &tools).await;
    let round = record(&judged);
    assert_eq!(round.outcome, "refused", "{round:?}");
    assert_eq!(round.refused_by, vec!["rule:clearance"]);
    assert_eq!(round.retried, None);
    assert_eq!(round.kicad_runs, 1);
    assert_eq!(round.remaining.len(), 1);
    assert_eq!(round.remaining[0].reason, "refused");
    assert!(judged
        .board
        .ends_with("run-0/r0/specctra_rounded_board.kicad_pcb"));
    assert!(!h.ctx.root.join("run-0/s1").exists());
    assert_eq!(judged.measures.reference_vias, 0);
    assert_eq!(judged.measures.violations.get("clearance"), None);
    assert_eq!(judged.reference_checked, None);
}

#[tokio::test]
async fn the_retry_drops_the_via_a_drc_item_names_and_keeps_the_rest() {
    let h = on(drc(&[], &[]));
    let mut near: Value = serde_json::from_str(&drc(&[], &["clearance"])).unwrap();
    near["violations"][0]["items"] =
        json!([{ "description": "a via", "pos": { "x": 104.0, "y": 52.0 } }]);
    h.canned(2, near.to_string());
    let tools = Calls::new(
        |_, board, _| (false, served(board)),
        |_| Ok(proposal(&[(106.0, 52.0), (104.0, 52.0)], 0.6)),
    );
    let judged = judge(&h, &tools).await;
    let round = record(&judged);
    assert_eq!(round.outcome, "kept", "{round:?}");
    assert_eq!(
        round.retried,
        Some(crate::pour_stitch::Retry {
            dropped_vias: 1,
            because: vec!["rule:clearance".to_string()],
        })
    );
    assert_eq!((round.written_vias, round.kicad_runs), (1, 2));
    assert_eq!((round.vias[0].x_mm, round.vias[0].y_mm), (106.0, 52.0));
    assert!(judged
        .board
        .ends_with("run-0/s1-retry/specctra_rounded_board.kicad_pcb"));
    assert!(!h.ctx.root.join("run-0/s1").exists());
    assert_eq!(h.calls().len(), 3);
}

#[tokio::test]
async fn a_planner_refusal_and_a_proposal_off_target_change_nothing() {
    let h = on(drc(&[], &[]));
    let tools = Calls::new(
        |_, board, _| (false, served(board)),
        |_| anyhow::bail!("unsupported padstack layer copper"),
    );
    let judged = judge(&h, &tools).await;
    let round = record(&judged);
    assert_eq!(round.outcome, "planner_refused");
    assert_eq!(
        round.reason.as_deref(),
        Some("unsupported padstack layer copper")
    );
    assert_eq!(
        round.remaining[0].reason,
        "unsupported padstack layer copper"
    );
    assert_eq!(h.calls().len(), 1);
    assert!(judged
        .board
        .ends_with("run-0/r0/specctra_rounded_board.kicad_pcb"));

    let h = on(drc(&[], &[]));
    // 4 mm from the change: a joiner, not a reference via.
    let tools = Calls::new(
        |_, board, _| (false, served(board)),
        |_| Ok(proposal(&[(109.5, 51.0)], 0.6)),
    );
    let judged = judge(&h, &tools).await;
    let round = record(&judged);
    assert_eq!(round.outcome, "no_proposal");
    assert_eq!((round.proposals, round.dropped), (1, 1));
    assert_eq!(
        round.remaining[0].reason,
        "2 reference changes remain; no safe candidate within the search bounds"
    );
    assert_eq!(h.calls().len(), 1);
}

#[tokio::test]
async fn findings_that_are_not_ground_are_skipped_and_a_short_list_is_read_in_full() {
    let h = on(drc(&[], &[]));
    let tools = Calls::new(
        |_, _, _| {
            (
                false,
                audit(
                    1,
                    vec![finding("SIG via 1", &["GND", "VCC"], SIGNAL_AT)],
                    80.0,
                ),
            )
        },
        |_| panic!("the planner must not be called"),
    );
    let judged = judge(&h, &tools).await;
    let round = record(&judged);
    assert_eq!(round.outcome, "none_needed");
    assert_eq!(round.skipped[0].reason, "two_reference_nets");

    // The audit counts 2 and lists 1: the round asks for the whole list.
    let h = on(drc(&[], &[]));
    let tools = Calls::new(
        |_, board, args| {
            if args.get("max_findings").is_some() {
                let both = vec![
                    finding("SIG via 1", &["GND"], SIGNAL_AT),
                    finding("SIG via 2", &["VCC"], (100.0, 40.0)),
                ];
                return (false, audit(2, both, 80.0));
            }
            let text = std::fs::read_to_string(board).unwrap_or_default();
            if text.contains("(at 106 52)") {
                (
                    false,
                    audit(1, vec![finding("SIG via 2", &["VCC"], (100.0, 40.0))], 80.0),
                )
            } else {
                (
                    false,
                    audit(2, vec![finding("SIG via 2", &["VCC"], (100.0, 40.0))], 80.0),
                )
            }
        },
        |_| Ok(proposal(&[(106.0, 52.0)], 0.6)),
    );
    let judged = judge(&h, &tools).await;
    let round = record(&judged);
    assert_eq!(round.outcome, "kept", "{round:?}");
    assert_eq!((round.targets, round.skipped.len()), (1, 1));
    let audits = tools.audits.lock().unwrap().clone();
    assert_eq!(audits[1]["max_findings"], 500, "{audits:?}");
    assert_eq!(
        audits.len(),
        3,
        "the post-Z9 board, its full list, the stitched board"
    );
}

#[tokio::test]
async fn a_failed_step_records_the_reason_and_the_judge_goes_on() {
    let h = on(drc(&[], &[]));
    let tools = Calls::new(
        |number, board, _| {
            if number == 1 {
                (true, "the audit refused".to_string())
            } else {
                (false, served(board))
            }
        },
        |_| Ok(proposal(&[(106.0, 52.0)], 0.6)),
    );
    let judged = judge(&h, &tools).await;
    let round = record(&judged);
    assert_eq!(round.outcome, "failed");
    assert!(
        round
            .reason
            .as_deref()
            .unwrap()
            .contains("the audit refused"),
        "{round:?}"
    );
    assert!(judged
        .board
        .ends_with("run-0/r0/specctra_rounded_board.kicad_pcb"));
    assert!(!h.ctx.root.join("run-0/s1").exists());
}

// ─── The delivery (S10, S11) ─────────────────────────────────────────────────

/// A finished search of the harness's one run, judged with the round on.
async fn won(h: &H, tools: &Calls) -> SearchOutcome {
    let base = clean_baseline();
    let judged = judge(h, tools).await;
    let key = rank(&[&judged])[0];
    let pour_key = crate::pour_rank::pour_keys(&[(&judged.measures, &judged.gates, true)])[0];
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
        pour_keys: vec![Some(pour_key)],
        rank: crate::pour_rank::RankMode::Routing,
        elapsed: Duration::from_secs(5),
        budget_overrun: None,
        stopped_launching: Some("fixed run count".to_string()),
        baseline: Ok(base),
        pour_prune: None,
        reference_stitch: None,
    }
}

fn stitched_ses(h: &H) -> PathBuf {
    h.routed.folder.join("stitched.ses")
}

async fn deliver(h: &H, tools: Arc<Calls>, outcome: &mut SearchOutcome) -> DeliveryRecord {
    deliver_winner(&h.ctx, tools, outcome).await
}

#[tokio::test]
async fn the_delivered_session_holds_the_vias_and_its_full_judge_is_the_ranked_board() {
    let h = on(drc(&[], &[]));
    // Two changes, one the planner can serve: the delivered board still has one, so a
    // round in its judge would run (and plan) again.
    let tools = Arc::new(Calls::new(
        |_, board, _| {
            let text = std::fs::read_to_string(board).unwrap_or_default();
            let mut findings = vec![finding("SIG via 2", &["GND"], (100.0, 40.0))];
            if !text.contains("(at 106 52)") {
                findings.push(finding("SIG via 1", &["GND"], SIGNAL_AT));
            }
            (false, audit(findings.len(), findings, 80.0))
        },
        |_| Ok(proposal(&[(106.0, 52.0)], 0.6)),
    ));
    let mut outcome = won(&h, &tools).await;
    let record = deliver(&h, tools.clone(), &mut outcome).await;
    assert_eq!(record.outcome, "delivered", "{record:?}");
    assert_eq!((record.vias, record.refused_by.len()), (1, 0));
    let run = &outcome.runs[0];
    let held = &run.routed.as_ref().unwrap().held;
    assert_eq!(held.ses, stitched_ses(&h));
    // The via in the GND block, snapped to the session's units (y mirrored).
    let text = std::fs::read_to_string(&held.ses).unwrap();
    let gnd = text.find("(net GND").unwrap();
    let at = text
        .find("(via \"konnect_via_0001\" 1060000 -520000)")
        .unwrap();
    assert!(gnd < at, "{text}");
    // The importer read it back: the judged board of the delivered session holds the via.
    let judged = run.judged.as_ref().unwrap();
    assert!(judged.board.starts_with(h.ctx.root.join("run-0-stitched")));
    let board = std::fs::read_to_string(&judged.board).unwrap();
    assert!(
        board.contains("(at 106 52)"),
        "the session's via is on the board"
    );
    // Its judge ran with the round off; the round's record stays with the run.
    assert_eq!(tools.plans().len(), 1, "only the search's judge planned");
    assert_eq!(judged.measures.reference_vias, 0);
    assert_eq!(judged.measures.checklist.unstitched_layer_changes, Some(1.0));
    assert_eq!(record_of(judged).outcome, "kept");
    assert_eq!(record_of(judged).remaining.len(), 1);
    assert!(undelivered_nets(judged).is_empty());
    assert_eq!(outcome.keys[0], Some(rank(&[judged])[0]));
}

fn record_of(judged: &Judged) -> &StitchRecord {
    judged.reference_stitch.as_ref().unwrap()
}

#[tokio::test]
async fn a_delivered_board_that_ranks_worse_is_not_delivered_and_its_vias_are_counted() {
    let h = on(drc(&[], &[]));
    // The delivered session's board audits at a lower copper quality.
    let tools = Arc::new(Calls::new(
        |_, board, _| {
            let delivered = board.to_string_lossy().contains("run-0-stitched");
            let text = std::fs::read_to_string(board).unwrap_or_default();
            let count = usize::from(!text.contains("(at 106 52)"));
            let findings = if count == 1 {
                vec![finding("SIG via 1", &["GND"], SIGNAL_AT)]
            } else {
                vec![]
            };
            (
                false,
                audit(count, findings, if delivered { 70.0 } else { 80.0 }),
            )
        },
        |_| Ok(proposal(&[(106.0, 52.0)], 0.6)),
    ));
    let mut outcome = won(&h, &tools).await;
    let before = outcome.runs[0].routed.as_ref().unwrap().held.ses.clone();
    let record = deliver(&h, tools.clone(), &mut outcome).await;
    assert_eq!(record.outcome, "rejected", "{record:?}");
    assert!(
        record.refused_by.contains(&"key:routing:11".to_string()),
        "{record:?}"
    );
    assert_eq!(outcome.runs[0].routed.as_ref().unwrap().held.ses, before);
    assert!(!stitched_ses(&h).exists());
    assert!(!h.ctx.root.join("run-0-stitched").exists());
    // The session lacks the via: the status and next steps count it.
    let judged = outcome.runs[0].judged.as_ref().unwrap();
    assert_eq!(judged.measures.reference_vias, 1);
    assert_eq!(undelivered_nets(judged), vec!["GND"]);
}

#[tokio::test]
async fn a_via_no_manifest_padstack_fits_and_a_winner_with_no_round_deliver_nothing() {
    let h = on(drc(&[], &[]));
    let tools = Arc::new(Calls::new(
        |_, board, _| (false, served(board)),
        |_| Ok(proposal(&[(106.0, 52.0)], 0.8)),
    ));
    let mut outcome = won(&h, &tools).await;
    assert_eq!(
        record_of(outcome.runs[0].judged.as_ref().unwrap()).outcome,
        "kept"
    );
    let record = deliver(&h, tools.clone(), &mut outcome).await;
    assert_eq!(record.outcome, "no_padstack", "{record:?}");
    assert!(!stitched_ses(&h).exists());

    let mut h = H::new(drc(&[], &[]));
    h.ctx.reference_stitch = ReferenceStitch::Off;
    let tools = Arc::new(Calls::served());
    let mut outcome = won(&h, &tools).await;
    let record = deliver(&h, tools.clone(), &mut outcome).await;
    assert_eq!(record.outcome, "nothing_to_deliver");
    assert_eq!(record.vias, 0);
}

// ─── The prune keeps them (S11) ──────────────────────────────────────────────

#[test]
fn the_prunes_kicad_count_holds_the_rounds_refills() {
    let mut judged = Judged {
        measures: measures(),
        gates: Gates {
            g1: vec![],
            g2: vec![],
            g3: vec![],
            g4: vec![],
        },
        board: PathBuf::new(),
        rounds: vec![],
        tune: None,
        reference_stitch: None,
        reference_checked: None,
    };
    let before = pour_prune::kicad_runs_of(&judged);
    assert_eq!(before, 1);
    let mut round = StitchRecord::new(Default::default());
    round.kicad_runs = 2;
    judged.reference_stitch = Some(round);
    assert_eq!(pour_prune::kicad_runs_of(&judged), before + 2);
}

/// The mutants batch's miss in `Round::steps` (shard 4): there is ONE retry. A retry that
/// is refused again, even by a DRC item that names a via, ends the round `refused`.
#[tokio::test]
async fn a_retry_refused_again_ends_the_round_refused() {
    let h = on(drc(&[], &[]));
    let item = |x: f64, y: f64| {
        let mut report: Value = serde_json::from_str(&drc(&[], &["clearance"])).unwrap();
        report["violations"][0]["items"] =
            json!([{ "description": "a via", "pos": { "x": x, "y": y } }]);
        report.to_string()
    };
    h.canned(2, item(104.0, 52.0));
    h.canned(3, item(105.0, 49.0));
    let tools = Calls::new(
        |_, board, _| (false, served(board)),
        |_| Ok(proposal(&[(106.0, 52.0), (104.0, 52.0), (105.0, 49.0)], 0.6)),
    );
    let judged = judge(&h, &tools).await;
    let round = record(&judged);
    assert_eq!(round.outcome, "refused", "{round:?}");
    assert_eq!(round.refused_by, vec!["rule:clearance"]);
    assert_eq!(round.retried.as_ref().map(|retry| retry.dropped_vias), Some(1));
    assert_eq!(round.kicad_runs, 2);
    assert_eq!(h.calls().len(), 3, "no third stitched board");
    assert!(judged.board.ends_with("run-0/r0/specctra_rounded_board.kicad_pcb"));
    assert!(!h.ctx.root.join("run-0/s1-retry").exists());
}
