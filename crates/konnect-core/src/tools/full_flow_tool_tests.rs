//! Row FF: `run_full_flow`, with a fake `FlowIo` that applies the writes to a text.
//! The scratch steps (P2 to P4) need KiCad and fastroute: the ignored end-to-end test
//! and the live smoke cover them. Everything from gate A on runs here.

use super::*;
use crate::router::ToolRouter;
use crate::tools::ServerConfig;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};

const BOARD_TEXT: &str =
    "(kicad_pcb (version 20240108) (generator \"pcbnew\") (net 0 \"\") (net 1 \"GND\"))";

fn ctx() -> Arc<ToolContext> {
    Arc::new(ToolContext::new(
        ServerConfig {
            kicad_cli: "kicad-cli-not-used".to_string(),
            kicad_binary: String::new(),
            ipc_address: String::new(),
            project_dir: None,
            jlcpcb_db_path: None,
            auto_load_toolsets: false,
            eager_toolsets: false,
        },
        Arc::new(ToolRouter::new()),
    ))
}

/// The tools that change the board (or the project), for the fake.
const WRITERS: [&str; 7] = [
    "set_component_placements",
    "add_zone",
    "refill_zones",
    "apply_specctra_ses",
    "route_batch",
    "run_drc",
    "create_custom_drc_rule",
];

/// A fake board: a live text and a saved text, every call logged, scripted answers.
#[derive(Default)]
struct Fake {
    live: Mutex<String>,
    saved: Mutex<String>,
    log: Mutex<Vec<String>>,
    scripted: Mutex<BTreeMap<String, VecDeque<Called>>>,
    /// Live reads that fail before one succeeds.
    live_failures: AtomicUsize,
    live_reads: AtomicUsize,
    /// `save_project` copies the live text to the saved file unless this is set.
    save_breaks: Mutex<bool>,
    checkpoints: AtomicUsize,
    writes: AtomicUsize,
    /// The flow file, to see the intent recorded before a write.
    flow_file: Mutex<Option<PathBuf>>,
    intents: Mutex<Vec<String>>,
    route: Mutex<Value>,
    deleted: Mutex<bool>,
    sent: Mutex<Vec<(String, Value)>>,
    pauses: AtomicUsize,
}

fn ok(body: Value) -> Called {
    Called::Answered(CallToolResult::json(&body))
}

impl Fake {
    fn new() -> Arc<Fake> {
        let fake = Fake::default();
        *fake.live.lock().unwrap() = BOARD_TEXT.to_string();
        *fake.saved.lock().unwrap() = BOARD_TEXT.to_string();
        *fake.route.lock().unwrap() = route_body();
        Arc::new(fake)
    }

    fn script(&self, tool: &str, answer: Called) {
        self.scripted
            .lock()
            .unwrap()
            .entry(tool.to_string())
            .or_default()
            .push_back(answer);
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn calls(&self) -> Vec<String> {
        self.log()
            .into_iter()
            .filter(|e| !e.starts_with("read:"))
            .collect()
    }

    fn change_live(&self) {
        let n = self.writes.fetch_add(1, Ordering::SeqCst);
        let mut live = self.live.lock().unwrap();
        let end = live.rfind(')').unwrap();
        live.insert_str(end, &format!(" (fake_item {n})"));
    }

    fn answer(&self, tool: &str, args: &Value) -> Called {
        if let Some(answer) = self
            .scripted
            .lock()
            .unwrap()
            .get_mut(tool)
            .and_then(VecDeque::pop_front)
        {
            if WRITERS.contains(&tool) && matches!(&answer, Called::Answered(r) if !r.is_error) {
                self.change_live();
            }
            return answer;
        }
        match tool {
            "delete_all_routing" if args["dry_run"] == json!(true) => {
                let gone = *self.deleted.lock().unwrap();
                ok(json!({
                    "status": if gone { "noop" } else { "ready" },
                    "plan_revision": "rev-delete",
                    "will_delete": if gone { counts(0, 0) } else { counts(10, 2) },
                    "kept_locked": counts(0, 0),
                }))
            }
            "delete_all_routing" => {
                *self.deleted.lock().unwrap() = true;
                self.change_live();
                ok(json!({ "status": "applied", "removed": counts(10, 2) }))
            }
            "route_specctra_dsn" => ok(self.route.lock().unwrap().clone()),
            "plan_route_refinement" => ok(json!({ "proposals": [{
                "kind": "stitching",
                "operations": [{ "tool": "route_batch", "arguments": { "board": "b", "tracks": [], "vias": [{ "x": 1.0 }] } }],
            }]})),
            "run_drc" => {
                self.change_live();
                ok(
                    json!({ "by_rule": [{ "rule": "unconnected_items", "severity": "error", "count": 0 }] }),
                )
            }
            "create_custom_drc_rule" if args["dry_run"] == json!(true) => {
                ok(json!({ "status": "ready", "plan_revision": "rev-rules" }))
            }
            "create_custom_drc_rule" => {
                self.change_live();
                ok(
                    json!({ "status": "applied", "rules": [{ "name": "len", "action": "created" }] }),
                )
            }
            "save_project" => {
                if !*self.save_breaks.lock().unwrap() {
                    let live = self.live.lock().unwrap().clone();
                    *self.saved.lock().unwrap() = live;
                }
                ok(json!({ "message": "Board saved successfully." }))
            }
            _ => {
                if WRITERS.contains(&tool) {
                    self.change_live();
                }
                ok(json!({ "ok": true }))
            }
        }
    }
}

impl FlowIo for Arc<Fake> {
    fn call(&self, tool: &'static str, args: Value) -> BoxFuture<Called> {
        let dry = if args["dry_run"] == json!(true) {
            "(dry)"
        } else {
            ""
        };
        self.log.lock().unwrap().push(format!("{tool}{dry}"));
        self.sent
            .lock()
            .unwrap()
            .push((tool.to_string(), args.clone()));
        if let Some(file) = self.flow_file.lock().unwrap().as_ref() {
            if let Ok(flow) = read_json(file) {
                let at = flow["at"].as_str().unwrap_or_default().to_string();
                let calling = flow["records"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["step"] == json!(at) && r["state"] == "calling");
                self.intents
                    .lock()
                    .unwrap()
                    .push(format!("{tool}:{at}:{calling}"));
            }
        }
        let answer = self.answer(tool, &args);
        Box::pin(std::future::ready(answer))
    }

    fn live_text(&self) -> BoxFuture<Result<String, String>> {
        self.log.lock().unwrap().push("read:live".to_string());
        self.live_reads.fetch_add(1, Ordering::SeqCst);
        let failing = self.live_failures.load(Ordering::SeqCst);
        if failing > 0 {
            self.live_failures.store(failing - 1, Ordering::SeqCst);
            return Box::pin(std::future::ready(Err(
                "KiCad cannot be reached".to_string()
            )));
        }
        let live = self.live.lock().unwrap().clone();
        Box::pin(std::future::ready(Ok(live)))
    }

    fn saved_text(&self) -> BoxFuture<Result<String, String>> {
        let saved = self.saved.lock().unwrap().clone();
        Box::pin(std::future::ready(Ok(saved)))
    }

    fn checkpoint(&self, _label: String) -> BoxFuture<Result<String, String>> {
        let n = self.checkpoints.fetch_add(1, Ordering::SeqCst);
        self.log.lock().unwrap().push("checkpoint".to_string());
        Box::pin(std::future::ready(Ok(format!("cp-{n}"))))
    }

    fn pause(&self, _pause: Duration) -> BoxFuture<()> {
        self.pauses.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::ready(()))
    }
}

fn counts(tracks: u64, vias: u64) -> Value {
    json!({ "total": tracks + vias, "tracks": tracks, "arcs": 0, "vias": vias })
}

/// A route result with every follow-up the flow runs, as `route_fastroute` lays them out.
fn route_body() -> Value {
    let drc =
        json!({ "board": "/b/board.kicad_pcb", "sync_live_board": true, "refill_zones": true });
    json!({
        "success": true,
        "status": "incomplete",
        "summary": { "pours": { "line": "Pours: as exported." }, "lengths": { "line": "Lengths: one group." } },
        "completion": {
            "judged": { "all_gates_pass": true, "gates": { "g1": [] } },
            "not_checked": [],
            "next_steps": [
                { "step": 1, "tool": "save_project", "arguments": {} },
                { "step": 2, "tool": "create_checkpoint", "arguments": { "project": "/b/board.kicad_pro" } },
                { "step": 3, "tool": "plan_specctra_ses_import", "arguments": { "board": "/b/board.kicad_pcb", "ses_path": "/r/routed.ses", "manifest_path": "/r/board.manifest.json" } },
                { "step": 4, "tool": "apply_specctra_ses", "arguments": { "board": "/b/board.kicad_pcb", "ses_path": "/r/routed.ses", "manifest_path": "/r/board.manifest.json", "candidate_output_path": "/r/routed.candidate.kicad_pcb" } },
                { "step": 5, "tool": "run_drc", "arguments": drc },
                { "step": 6, "tool": "plan_route_refinement", "arguments": { "board": "/b/board.kicad_pcb", "kinds": ["stitching"], "nets": ["GND"], "search_radius_mm": 3.0 } },
                { "step": 7, "tool": "route_batch", "arguments": { "board": "/b/board.kicad_pcb" }, "arguments_from": { "vias": "…" } },
                { "step": 8, "tool": "run_drc", "arguments": drc, "repeat": "Repeat steps 6 to 8 …" },
                { "step": 9, "tool": "plan_route_refinement", "arguments": { "board": "/b/board.kicad_pcb", "kinds": ["stitching"], "nets": ["GND"], "search_radius_mm": 3.0 } },
                { "step": 10, "tool": "route_batch", "arguments": { "board": "/b/board.kicad_pcb" } },
                { "step": 11, "tool": "run_drc", "arguments": drc },
                { "step": 12, "manual": "Widen the window …" },
                { "step": 13, "manual": "To keep KiCad checking these lengths …", "tool": "create_custom_drc_rule",
                  "arguments": { "board": "/b/board.kicad_pcb", "rules": [{ "name": "len", "rule": "(rule len)" }], "replace_existing": true } },
            ],
        },
    })
}

/// A flow at gate A's far side: P0 to P4 done, their files written, every key approved.
struct Planned {
    _root: tempfile::TempDir,
    state: PathBuf,
    flow: FlowRecord,
}

fn planned() -> Planned {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let board = root.path().join("board.kicad_pcb");
    std::fs::write(&board, BOARD_TEXT).unwrap();
    let mut flow = FlowRecord {
        flow_id: "1700000000000-0123456789ab".to_string(),
        board: board.display().to_string(),
        project: board.with_extension("kicad_pro").display().to_string(),
        state: FlowState::Running,
        board_state: BoardState::Unchanged,
        steps_asked: Steps::default(),
        approvals: Approvals {
            placement: Some(PlacementAnswer::Apply),
            delete_routing: Some(DeleteAnswer::Delete),
            pours: Some(PoursAnswer::Add),
            pour_roles: Some(RolesAnswer::AsNamed(AsNamed::AsNamed)),
            routing: Some(RoutingAnswer::ApplyIfComplete),
        },
        start_approvals: Approvals::default(),
        effort: "m".to_string(),
        placement_effort: "m".to_string(),
        fastroute: json!({ "path": "/fr/fastroute" }),
        planned_from: crate::board_identity::canonical_sha256(BOARD_TEXT).unwrap(),
        routing_items: 12,
        nets: ["GND".to_string()].into_iter().collect(),
        rails: BTreeMap::new(),
        gate: None,
        gate_body: None,
        plan_revision: None,
        facts: PlanFacts {
            change: Some("placement".to_string()),
            unlocked_routing: 12,
            zones: 2,
            fill_clean: true,
            p4_on_winner: true,
            ..PlanFacts::default()
        },
        route: None,
        at: None,
        attempt: 1,
        route_attempt: 1,
        records: Vec::new(),
        legs: Vec::new(),
        start_checkpoint: None,
        stop_reason: None,
        checkpoints: Vec::new(),
    };
    for step in [
        Step::Preflight,
        Step::CountRouting,
        Step::PoursBefore,
        Step::Placement,
        Step::PoursAfter,
    ] {
        flow.put(StepRecord::new(step, StepState::Done));
    }
    let attempt = attempt_folder(&state, &flow);
    write_value(
        &attempt.join(P1_FILE),
        &json!({ "will_delete": counts(10, 2), "kept_locked": counts(0, 0) }),
    )
    .unwrap();
    write_value(&attempt.join(P3_FILE), &json!({
        "status": "planned",
        "winner": { "change": "placement", "moves": [], "moved_parts": 1 },
        "apply_with": { "tool": "set_component_placements", "arguments": { "board": flow.board, "placements": [{ "reference": "R1", "x": 10.0, "y": 5.0, "rotation": 90.0 }] } },
    })).unwrap();
    write_value(&attempt.join(P4_FILE), &json!({
        "status": "planned",
        "proposals": [
            { "net": "GND", "layer": "F.Cu", "add_zone": { "board": flow.board, "net_name": "GND", "layer": "F.Cu", "points": [] } },
            { "net": "GND", "layer": "B.Cu", "add_zone": { "board": flow.board, "net_name": "GND", "layer": "B.Cu", "points": [] } },
        ],
    })).unwrap();
    std::fs::create_dir_all(flow_folder(&state, &flow.flow_id).join("steps")).unwrap();
    save_flow(&state, &flow).unwrap();
    Planned {
        _root: root,
        state,
        flow,
    }
}

async fn leg(planned: &mut Planned, fake: &Arc<Fake>) -> FlowRecord {
    *fake.flow_file.lock().unwrap() =
        Some(flow_folder(&planned.state, &planned.flow.flow_id).join(FLOW_FILE));
    let io: Arc<dyn FlowIo> = Arc::new(Arc::clone(fake));
    let flow = run_leg(
        &planned.state,
        planned.flow.clone(),
        io,
        ctx(),
        Arc::new(FlowProgress::default()),
    )
    .await
    .unwrap();
    planned.flow = flow.clone();
    flow
}

// ─── The calls ───────────────────────────────────────────────────────────────

#[test]
fn each_form_alone_is_accepted() {
    let id = "1700000000000-0123456789ab";
    assert!(matches!(
        read_request(&json!({ "board": "/b/x.kicad_pcb" })),
        Ok(Request::Start(_))
    ));
    assert!(matches!(
        read_request(&json!({ "run_id": id, "wait_s": 300 })),
        Ok(Request::Report { .. })
    ));
    assert!(matches!(
        read_request(
            &json!({ "run_id": id, "answer": { "placement": "apply" }, "expected_plan_revision": "r" })
        ),
        Ok(Request::Answer { .. })
    ));
    assert!(matches!(
        read_request(&json!({ "run_id": id, "resume": true })),
        Ok(Request::Resume { .. })
    ));
    assert!(matches!(
        read_request(&json!({ "run_id": id, "abandon": true })),
        Ok(Request::Abandon { .. })
    ));
}

fn refused_field(args: Value) -> String {
    let refusal = read_request(&args).expect_err("refused");
    body_of(&refusal).to_string()
}

#[test]
fn a_mix_of_forms_is_refused_naming_the_argument() {
    let id = "1700000000000-0123456789ab";
    assert!(refused_field(json!({ "board": "/b/x.kicad_pcb", "wait_s": 1 })).contains("wait_s"));
    assert!(refused_field(json!({ "run_id": id, "board": "/b/x.kicad_pcb" })).contains("board"));
    assert!(
        refused_field(json!({ "run_id": id, "resume": true, "abandon": true })).contains("abandon")
    );
    assert!(
        refused_field(json!({ "run_id": id, "answer": {}, "resume": true })).contains("resume")
    );
    assert!(refused_field(json!({ "run_id": id, "resume": true, "wait_s": 5 })).contains("wait_s"));
    assert!(
        refused_field(json!({ "run_id": id, "answer": { "pours": "add" } }))
            .contains("expected_plan_revision")
    );
    assert!(
        refused_field(json!({ "run_id": id, "expected_plan_revision": "r" }))
            .contains("expected_plan_revision")
    );
    assert!(refused_field(json!({ "run_id": "../x" })).contains("run_id"));
}

#[test]
fn wait_s_above_600_and_unknown_values_are_refused() {
    let id = "1700000000000-0123456789ab";
    assert!(refused_field(json!({ "run_id": id, "wait_s": 601 })).contains("wait_s"));
    assert!(
        refused_field(json!({ "board": "/b/x.kicad_pcb", "steps": { "pours": "run" } }))
            .contains("steps")
    );
    assert!(
        refused_field(json!({ "board": "/b/x.kicad_pcb", "approve": { "routing": "yes" } }))
            .contains("approve")
    );
    assert!(
        refused_field(json!({ "board": "/b/x.kicad_pcb", "effort": "max" })).contains("effort")
    );
    assert!(refused_field(json!({ "board": "/b/x.pcb" })).contains("board"));
}

#[test]
fn the_schema_is_closed_and_registered_in_the_integration_toolset() {
    let def = find_tool("run_full_flow").expect("registered");
    assert!(def
        .input_validator
        .validate(
            &json!({ "board": "/b/x.kicad_pcb", "approve": { "pour_roles": { "GND": "ground" } } })
        )
        .is_ok());
    assert!(def
        .input_validator
        .validate(&json!({ "board": "/b/x.kicad_pcb", "zones": 1 }))
        .is_err());
    assert!(def
        .input_validator
        .validate(&json!({ "run_id": "1-0", "wait_s": 900 }))
        .is_err());
    let integration = crate::router::registry::tools_for("integration").unwrap();
    assert!(integration.iter().any(|tool| tool.name == "run_full_flow"));
}

// ─── P0 and the marker ───────────────────────────────────────────────────────

fn temp_board(project: bool) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let board = root.path().join("board.kicad_pcb");
    std::fs::write(&board, BOARD_TEXT).unwrap();
    if project {
        std::fs::write(board.with_extension("kicad_pro"), "{}").unwrap();
    }
    let state = root.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    (root, board, state)
}

fn start_call(board: &Path) -> StartCall {
    StartCall {
        board: board.to_path_buf(),
        effort: None,
        placement_effort: None,
        fastroute_path: None,
        steps: Steps::default(),
        approve: Approvals::default(),
    }
}

fn leaves_nothing(state: &Path) {
    let flows: Vec<_> = std::fs::read_dir(flows_root(state))
        .map(|dir| {
            dir.filter_map(Result::ok)
                .map(|e| e.file_name())
                .filter(|n| n != ".boards")
                .collect()
        })
        .unwrap_or_default();
    assert!(flows.is_empty(), "{flows:?}");
}

#[tokio::test]
async fn no_project_file_is_refused_and_leaves_nothing() {
    let (_root, board, state) = temp_board(false);
    let refused = preflight(&ctx(), &state, &board, &start_call(&board))
        .await
        .unwrap()
        .err()
        .unwrap();
    assert!(body_of(&refused).to_string().contains("no project file"));
    leaves_nothing(&state);
    assert!(open_flow(&state, &board).unwrap().is_none());
}

#[tokio::test]
async fn a_flow_open_on_the_board_is_refused() {
    let (_root, board, state) = temp_board(true);
    let mut flow = planned().flow;
    flow.board = board.display().to_string();
    std::fs::create_dir_all(flow_folder(&state, &flow.flow_id)).unwrap();
    save_flow(&state, &flow).unwrap();
    mark_open(&state, &board, &flow.flow_id).unwrap().unwrap();
    let refused = preflight(&ctx(), &state, &board, &start_call(&board))
        .await
        .unwrap()
        .err()
        .unwrap();
    assert!(body_of(&refused).to_string().contains(&flow.flow_id));
}

#[tokio::test]
async fn a_placement_run_on_the_board_is_refused() {
    let (_root, board, state) = temp_board(true);
    let claim = claim_board(
        &state,
        &PLACEMENT_RUNS,
        &board,
        "1700000000000-aaaaaaaaaaaa",
    )
    .unwrap()
    .unwrap();
    let refused = preflight(&ctx(), &state, &board, &start_call(&board))
        .await
        .unwrap()
        .err()
        .unwrap();
    assert!(body_of(&refused)
        .to_string()
        .contains("plan_convergent_placement"));
    drop(claim);
}

#[test]
fn only_one_of_two_starts_makes_the_marker() {
    let (_root, board, state) = temp_board(true);
    let mut flow = planned().flow;
    flow.board = board.display().to_string();
    std::fs::create_dir_all(flow_folder(&state, &flow.flow_id)).unwrap();
    save_flow(&state, &flow).unwrap();
    assert!(mark_open(&state, &board, &flow.flow_id).unwrap().is_ok());
    assert_eq!(
        mark_open(&state, &board, "1700000000001-bbbbbbbbbbbb").unwrap(),
        Err(flow.flow_id.clone())
    );
    // A flow that has ended frees its board.
    flow.state = FlowState::Done;
    save_flow(&state, &flow).unwrap();
    assert!(open_flow(&state, &board).unwrap().is_none());
    assert!(mark_open(&state, &board, "1700000000001-bbbbbbbbbbbb")
        .unwrap()
        .is_ok());
    clear_open(&state, &board, "someone-else");
    assert!(
        marker_path(&state, &board).is_file(),
        "a marker of another flow stays"
    );
}

// ─── The order and the protocol ──────────────────────────────────────────────

#[tokio::test]
async fn everything_approved_runs_the_writes_in_order_with_a_save_after_each() {
    let mut planned = planned();
    let fake = Fake::new();
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.state, FlowState::Done, "{:?}", flow.stop_reason);
    let expected = [
        "checkpoint",
        "delete_all_routing(dry)",
        "delete_all_routing",
        "save_project",
        "checkpoint",
        "set_component_placements",
        "save_project",
        "checkpoint",
        "add_zone",
        "add_zone",
        "refill_zones",
        "save_project",
        "export_specctra_dsn",
        "route_specctra_dsn",
        "plan_specctra_ses_import",
        "checkpoint",
        "apply_specctra_ses",
        "save_project",
        "checkpoint",
        "plan_route_refinement",
        "route_batch",
        "run_drc",
        "save_project",
        "checkpoint",
        "plan_route_refinement",
        "route_batch",
        "run_drc",
        "save_project",
        "checkpoint",
        "create_custom_drc_rule(dry)",
        "create_custom_drc_rule",
        "save_project",
        "checkpoint",
        "run_drc",
        "save_project",
    ];
    assert_eq!(fake.calls(), expected);
    for record in &flow.records {
        if record.step.writes() {
            assert_eq!(record.state, StepState::Done, "{:?}", record.step);
            assert!(
                record.after.is_some() && record.before.is_some() && record.checkpoint.is_some()
            );
        }
    }
    assert_eq!(flow.start_checkpoint.as_deref(), Some("cp-0"));
    assert!(open_flow(&planned.state, Path::new(&flow.board))
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn the_intent_is_recorded_before_every_write() {
    let mut planned = planned();
    let fake = Fake::new();
    leg(&mut planned, &fake).await;
    let intents = fake.intents.lock().unwrap().clone();
    for intent in intents
        .iter()
        .filter(|i| WRITERS.iter().any(|w| i.starts_with(&format!("{w}:"))))
    {
        assert!(intent.ends_with(":true"), "{intent}");
    }
}

#[tokio::test]
async fn gate_b_stops_when_a_judge_gate_failed_and_writes_nothing_more() {
    let mut planned = planned();
    let fake = Fake::new();
    fake.route.lock().unwrap()["completion"]["judged"]["all_gates_pass"] = json!(false);
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.state, FlowState::AwaitingAnswer);
    assert_eq!(flow.gate, Some(Gate::Routing));
    assert_eq!(
        fake.calls().last().map(String::as_str),
        Some("plan_specctra_ses_import")
    );
    let body = flow.gate_body.unwrap();
    assert_eq!(body["all_gates_pass"], false);
    assert_eq!(body["needs"][0]["key"], "routing");
    assert_eq!(body["needs"][0]["recommended"], Value::Null);
}

#[tokio::test]
async fn a_board_changed_since_the_plan_stops_at_l1() {
    let mut planned = planned();
    let fake = Fake::new();
    fake.change_live();
    let saved = fake.live.lock().unwrap().clone();
    *fake.saved.lock().unwrap() = saved;
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.state, FlowState::Stopped);
    assert_eq!(flow.at, Some(Step::Unchanged));
    assert!(flow.stop_reason.unwrap().starts_with("board_changed"));
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());
}

#[tokio::test]
async fn a_write_that_moved_the_board_elsewhere_stops_the_next_step() {
    let mut planned = planned();
    let fake = Fake::new();
    // L2 done; then something else changes and saves the board before L3.
    fake.script("delete_all_routing", ok(json!({ "status": "ready", "plan_revision": "r", "will_delete": counts(10, 2), "kept_locked": counts(0, 0) })));
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.state, FlowState::Done);
    // Replay: a record whose `after` is not the saved board.
    let mut planned = self::planned();
    let mut deleted = StepRecord::new(Step::DeleteRouting, StepState::Done);
    deleted.after = Some("not-the-board".to_string());
    deleted.before = Some(planned.flow.planned_from.clone());
    planned
        .flow
        .put(StepRecord::new(Step::Unchanged, StepState::Done));
    planned.flow.put(deleted);
    let fake = Fake::new();
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.at, Some(Step::ApplyPlacement));
    assert!(flow.stop_reason.unwrap().starts_with("board_changed"));
    assert!(!fake
        .calls()
        .contains(&"set_component_placements".to_string()));
}

#[tokio::test]
async fn ff9_a_fresh_count_that_differs_stops_before_the_delete() {
    let mut planned = planned();
    let fake = Fake::new();
    fake.script("delete_all_routing", ok(json!({ "status": "ready", "plan_revision": "r", "will_delete": counts(11, 2), "kept_locked": counts(0, 0) })));
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.state, FlowState::Stopped);
    assert_eq!(flow.at, Some(Step::DeleteRouting));
    assert_eq!(fake.calls(), ["checkpoint", "delete_all_routing(dry)"]);
    assert_eq!(flow.board_state, BoardState::Unchanged);
}

#[tokio::test]
async fn a_refused_write_ends_the_step_as_failed() {
    let mut planned = planned();
    let fake = Fake::new();
    fake.script(
        "set_component_placements",
        Called::Answered(CallToolResult::error("no such footprint")),
    );
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.at, Some(Step::ApplyPlacement));
    assert_eq!(
        flow.record(Step::ApplyPlacement).unwrap().state,
        StepState::Failed
    );
    assert!(!fake.calls().contains(&"add_zone".to_string()));
}

#[tokio::test]
async fn a_handler_error_from_a_write_is_uncertain() {
    let mut planned = planned();
    let fake = Fake::new();
    fake.script(
        "set_component_placements",
        Called::Erred("IPC dropped".to_string()),
    );
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(
        flow.record(Step::ApplyPlacement).unwrap().state,
        StepState::Uncertain
    );
    assert_eq!(flow.board_state, BoardState::Uncertain);
    let view = view(&planned.state, &flow, None);
    assert!(view["next_steps"][0]
        .as_str()
        .unwrap()
        .contains("Inspect the board"));
    assert!(view["way_back"]["start"]["preview"]["arguments"]["dry_run"] == json!(true));
}

#[tokio::test]
async fn an_uncertain_kind_from_a_write_is_uncertain() {
    let mut planned = planned();
    let fake = Fake::new();
    fake.script(
        "set_component_placements",
        Called::Answered(CallToolResult::error_kind(
            crate::mcp::error::ToolErrorKind::MutationOutcomeUncertain {
                operation: "set_component_placements".into(),
                path: "b".into(),
                reason: "r".into(),
            },
            "may have changed",
        )),
    );
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(
        flow.record(Step::ApplyPlacement).unwrap().state,
        StepState::Uncertain
    );
}

#[tokio::test]
async fn a_save_that_does_not_verify_is_unsaved() {
    let mut planned = planned();
    let fake = Fake::new();
    *fake.save_breaks.lock().unwrap() = true;
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.at, Some(Step::DeleteRouting));
    assert_eq!(
        flow.record(Step::DeleteRouting).unwrap().state,
        StepState::Unsaved
    );
    assert_eq!(flow.board_state, BoardState::Unsaved);
}

#[tokio::test]
async fn a_failed_read_is_tried_three_times_and_a_write_never_again() {
    let mut planned = planned();
    let fake = Fake::new();
    fake.live_failures.store(2, Ordering::SeqCst);
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(
        flow.state,
        FlowState::Done,
        "two failures then a read: the flow goes on"
    );
    let mut planned = self::planned();
    let fake = Fake::new();
    fake.live_failures.store(3, Ordering::SeqCst);
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.state, FlowState::Stopped);
    assert!(flow.stop_reason.unwrap().starts_with("kicad_unreachable"));
    assert_eq!(fake.live_reads.load(Ordering::SeqCst), 3);
    // A write that errs is not repeated.
    let mut planned = self::planned();
    let fake = Fake::new();
    fake.script("refill_zones", Called::Erred("dropped".to_string()));
    leg(&mut planned, &fake).await;
    assert_eq!(
        fake.calls().iter().filter(|c| *c == "refill_zones").count(),
        1
    );
}

#[tokio::test]
async fn a_refused_combination_approved_in_advance_goes_back_to_gate_a_before_any_write() {
    let mut planned = planned();
    planned.flow.approvals.delete_routing = Some(DeleteAnswer::Keep);
    let fake = Fake::new();
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.state, FlowState::AwaitingAnswer);
    assert_eq!(flow.gate, Some(Gate::Plan));
    let body = flow.gate_body.clone().unwrap();
    assert!(body["refused"].as_str().unwrap().contains("delete_routing"));
    assert!(fake.calls().is_empty());
    // M4: the answer there may change a key set in advance; one still refused stays.
    let revision = flow.plan_revision.clone().unwrap();
    let mut again = flow.clone();
    let still = Approvals {
        placement: Some(PlacementAnswer::Apply),
        ..Approvals::default()
    };
    assert!(apply_answer(&mut again, Gate::Plan, &still, &revision).is_err());
    assert_eq!(again, flow, "a refused answer changes nothing");
    let fixed = Approvals {
        delete_routing: Some(DeleteAnswer::Delete),
        ..Approvals::default()
    };
    apply_answer(&mut again, Gate::Plan, &fixed, &revision).unwrap();
    assert_eq!(again.approvals.delete_routing, Some(DeleteAnswer::Delete));
    planned.flow = again;
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.state, FlowState::Done, "{:?}", flow.stop_reason);
}

#[tokio::test]
async fn the_reference_planners_refusal_is_left_for_the_user() {
    let mut planned = planned();
    let fake = Fake::new();
    fake.script("plan_route_refinement", ok(json!({ "proposals": [] })));
    fake.script(
        "plan_route_refinement",
        Called::Answered(CallToolResult::error("local pad overrides")),
    );
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.state, FlowState::Done, "{:?}", flow.stop_reason);
    let reference = flow.record(Step::ReferenceVias).unwrap();
    assert_eq!(reference.state, StepState::Done);
    assert!(reference.not_checked[0].starts_with("left for you"));
}

// ─── Resume (FF15) ───────────────────────────────────────────────────────────

/// A flow stopped at `step`: everything before it done through the fake.
async fn stopped_at(step: Step, fail: Called, tool: &str) -> (Planned, Arc<Fake>) {
    let mut planned = planned();
    let fake = Fake::new();
    fake.script(tool, fail);
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.at, Some(step), "{:?}", flow.stop_reason);
    (planned, fake)
}

#[tokio::test]
async fn resume_records_a_placement_that_landed() {
    let (mut planned, fake) = stopped_at(
        Step::ApplyPlacement,
        Called::Erred("dropped".into()),
        "set_component_placements",
    )
    .await;
    // The placement did land: R1 is at its target on the board.
    let landed = "(kicad_pcb (version 20240108) (footprint \"R\" (at 10 5 90) (property \"Reference\" \"R1\")))";
    *fake.live.lock().unwrap() = landed.to_string();
    *fake.saved.lock().unwrap() = landed.to_string();
    assert!(
        already_done(Step::ApplyPlacement, &planned.state, &planned.flow, &fake)
            .await
            .unwrap()
    );
    resume_step(&planned.state, &mut planned.flow, &fake)
        .await
        .unwrap()
        .unwrap();
    let record = planned.flow.record(Step::ApplyPlacement).unwrap();
    assert_eq!(record.state, StepState::Done);
    assert_eq!(
        record.after.as_deref(),
        Some(
            crate::board_identity::canonical_sha256(landed)
                .unwrap()
                .as_str()
        )
    );
    let calls_before = fake.calls().len();
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.state, FlowState::Done, "{:?}", flow.stop_reason);
    assert!(!fake.calls()[calls_before..].contains(&"set_component_placements".to_string()));
}

#[tokio::test]
async fn resume_runs_a_step_again_only_at_its_start() {
    let (mut planned, fake) = stopped_at(
        Step::ApplyPlacement,
        Called::Answered(CallToolResult::error("refused")),
        "set_component_placements",
    )
    .await;
    resume_step(&planned.state, &mut planned.flow, &fake)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        planned.flow.record(Step::ApplyPlacement).map(|r| r.state),
        Some(StepState::NotRun),
        "it runs again"
    );
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.state, FlowState::Done);
    // Elsewhere, and without the step's work: refused.
    let (mut planned, fake) = stopped_at(
        Step::ApplyPlacement,
        Called::Erred("dropped".into()),
        "set_component_placements",
    )
    .await;
    fake.change_live();
    let saved = fake.live.lock().unwrap().clone();
    *fake.saved.lock().unwrap() = saved;
    let refused = resume_step(&planned.state, &mut planned.flow, &fake)
        .await
        .unwrap()
        .unwrap_err();
    assert!(refused.starts_with("board_changed"), "{refused}");
}

#[tokio::test]
async fn resume_only_saves_again_an_unsaved_step() {
    let mut planned = planned();
    let fake = Fake::new();
    *fake.save_breaks.lock().unwrap() = true;
    leg(&mut planned, &fake).await;
    *fake.save_breaks.lock().unwrap() = false;
    let before = fake.calls().len();
    resume_step(&planned.state, &mut planned.flow, &fake)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fake.calls()[before..], ["save_project"]);
    assert_eq!(
        planned.flow.record(Step::DeleteRouting).unwrap().state,
        StepState::Done
    );
}

#[tokio::test]
async fn already_done_reads_each_step_from_the_board() {
    let planned = planned();
    let fake = Fake::new();
    // L2: the fresh dry run says noop only once the routing is gone.
    assert!(
        !already_done(Step::DeleteRouting, &planned.state, &planned.flow, &fake)
            .await
            .unwrap()
    );
    *fake.deleted.lock().unwrap() = true;
    assert!(
        already_done(Step::DeleteRouting, &planned.state, &planned.flow, &fake)
            .await
            .unwrap()
    );
    // L4: both zones present.
    let one = "(kicad_pcb (zone (net 1) (net_name \"GND\") (layer \"F.Cu\")))";
    *fake.live.lock().unwrap() = one.to_string();
    assert!(
        !already_done(Step::AddZones, &planned.state, &planned.flow, &fake)
            .await
            .unwrap()
    );
    let both = "(kicad_pcb (zone (net_name \"GND\") (layer \"F.Cu\")) (zone (net_name \"GND\") (layers \"B.Cu\")))";
    *fake.live.lock().unwrap() = both.to_string();
    assert!(
        already_done(Step::AddZones, &planned.state, &planned.flow, &fake)
            .await
            .unwrap()
    );
    // F1: routing beyond the locked routing kept.
    *fake.live.lock().unwrap() = "(kicad_pcb (segment (net 1)))".to_string();
    assert!(
        already_done(Step::ApplySession, &planned.state, &planned.flow, &fake)
            .await
            .unwrap()
    );
    *fake.live.lock().unwrap() = BOARD_TEXT.to_string();
    assert!(
        !already_done(Step::ApplySession, &planned.state, &planned.flow, &fake)
            .await
            .unwrap()
    );
    // F3: every batch noop.
    write_value(
        &route_folder(&planned.state, &planned.flow).join(ROUTE_FILE),
        &route_body(),
    )
    .unwrap();
    assert!(
        !already_done(Step::LengthRules, &planned.state, &planned.flow, &fake)
            .await
            .unwrap()
    );
    fake.script("create_custom_drc_rule", ok(json!({ "status": "noop" })));
    assert!(
        already_done(Step::LengthRules, &planned.state, &planned.flow, &fake)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn a_resume_of_the_route_uses_a_new_route_folder() {
    let mut planned = planned();
    let fake = Fake::new();
    fake.script(
        "route_specctra_dsn",
        Called::Answered(CallToolResult::error("no winner")),
    );
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.at, Some(Step::Route));
    assert_eq!(flow.route_attempt, 1);
    resume_step(&planned.state, &mut planned.flow, &fake)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(planned.flow.route_attempt, 2);
    assert!(planned.flow.record(Step::Export).is_none());
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.state, FlowState::Done);
    assert!(route_folder(&planned.state, &flow).ends_with("routes/2"));
}

// ─── Answers ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn gate_a_answer_with_is_complete_and_bound_to_the_plan() {
    let mut planned = planned();
    planned.flow.approvals = Approvals::default();
    let fake = Fake::new();
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.gate, Some(Gate::Plan));
    let view = view(&planned.state, &flow, None);
    let answer: Approvals = serde_json::from_value(view["answer_with"]["answer"].clone()).unwrap();
    let merged = flow.approvals.merged(&answer, Gate::Plan).unwrap();
    assert!(needs_at_plan(&flow.facts, &flow.steps_asked, &merged).is_empty());
    assert_eq!(
        view["answer_with"]["expected_plan_revision"],
        json!(flow.plan_revision)
    );
    assert_eq!(
        view["gate"]["routing_that_goes"]["will_delete"]["total"],
        12
    );
    assert_eq!(view["gate"]["zones"].as_array().unwrap().len(), 2);
    assert!(
        fake.calls().is_empty(),
        "nothing is written before the answer"
    );
}

// ─── WithPours (FF4) ─────────────────────────────────────────────────────────

#[derive(Default)]
struct SeenRoute {
    texts: Mutex<Vec<String>>,
}

impl RouteCall for SeenRoute {
    fn route(
        &self,
        _id: usize,
        text: String,
        _folder: PathBuf,
    ) -> BoxFuture<(
        crate::place_judge::RouteResult,
        crate::place_search::Timings,
    )> {
        self.texts.lock().unwrap().push(text);
        Box::pin(std::future::ready((
            crate::place_judge::RouteResult::Failed {
                stage: "export",
                reason: "seen".into(),
            },
            crate::place_search::Timings::default(),
        )))
    }

    fn unrouted(
        &self,
        text: String,
        _folder: PathBuf,
    ) -> BoxFuture<anyhow::Result<BTreeMap<String, usize>>> {
        self.texts.lock().unwrap().push(text);
        Box::pin(std::future::ready(Ok(BTreeMap::new())))
    }
}

fn zone() -> ProposedZone {
    let text = crate::pour_plan::tests::pours_without_pours();
    let facts = crate::pour_plan::board_facts(&text).unwrap();
    let nets: BTreeSet<String> = facts.pads.keys().cloned().collect();
    let roles = crate::tools::supply_roles(&nets, None, None);
    let rules = crate::pour_plan::project_rules(None).unwrap();
    crate::pour_plan::plan(&facts, &roles, &rules, None)
        .proposals
        .remove(0)
}

#[tokio::test]
async fn with_pours_routes_every_candidate_with_the_zones_and_measures_the_zoned_yardstick() {
    let plain = crate::pour_plan::tests::pours_without_pours();
    let zones = vec![zone()];
    let zoned_text = crate::pour_plan::with_zones(&plain, &zones).unwrap();
    let seen = Arc::new(SeenRoute::default());
    let with = WithPours {
        zoned: Arc::clone(&seen) as Arc<dyn RouteCall>,
        zones: zones.clone(),
        plain_text: plain.clone(),
        zoned_text: zoned_text.clone(),
    };
    with.route(1, plain.clone(), PathBuf::from("/x")).await;
    assert!(with
        .unrouted("another board".into(), PathBuf::from("/y"))
        .await
        .is_err());
    with.unrouted(plain.clone(), PathBuf::from("/y"))
        .await
        .unwrap();
    let texts = seen.texts.lock().unwrap().clone();
    assert_eq!(texts.len(), 2);
    let zone_count = |text: &str| {
        konnect_sexp::parse_sexp(text)
            .unwrap()
            .find_all("zone")
            .len()
    };
    assert_eq!(zone_count(&texts[0]), zone_count(&plain) + 1);
    assert_eq!(texts[1], zoned_text);
    assert_eq!(
        zone_count(&plain),
        zone_count(&crate::pour_plan::tests::pours_without_pours()),
        "the model's text holds no zone of the plan"
    );
}

// ─── Follow-ups and adapters ─────────────────────────────────────────────────

#[test]
fn route_follow_ups_takes_the_three_groups_and_leaves_the_rest() {
    let follow = route_follow_ups(&route_body());
    assert_eq!(follow.plane.as_ref().unwrap()[0]["step"], 6);
    assert_eq!(follow.reference.as_ref().unwrap()[0]["step"], 9);
    assert_eq!(follow.rules.len(), 1);
    assert_eq!(follow.rules[0]["replace_existing"], true);
    assert_eq!(follow.left, ["Widen the window …"]);
    let none = route_follow_ups(&json!({ "completion": { "next_steps": [] } }));
    assert!(none.plane.is_none() && none.reference.is_none() && none.rules.is_empty());
}

#[test]
fn stitch_batches_never_exceed_the_cap() {
    let vias: Vec<Value> = (0..450).map(|n| json!({ "x": n })).collect();
    let planned = json!({ "proposals": [
        { "kind": "stitching", "operations": [{ "tool": "route_batch", "arguments": { "vias": vias } }] },
        { "kind": "shorten", "operations": [{ "tool": "route_batch", "arguments": { "vias": [{ "x": -1 }] } }] },
    ]});
    let batches = stitch_batches(&planned, &json!("/b/board.kicad_pcb"));
    assert_eq!(batches.len(), 3);
    let sizes: Vec<usize> = batches
        .iter()
        .map(|b| b["vias"].as_array().unwrap().len())
        .collect();
    assert_eq!(sizes, [200, 200, 50]);
    assert!(batches.iter().all(|b| b["board"] == "/b/board.kicad_pcb"));
}

#[test]
fn same_deletion_compares_the_approved_counts() {
    let approved = json!({ "will_delete": counts(10, 2), "kept_locked": counts(1, 1) });
    assert!(same_deletion(
        &approved,
        &json!({ "will_delete": counts(10, 2), "kept_locked": counts(1, 1) }),
        false
    )
    .is_ok());
    assert!(same_deletion(
        &approved,
        &json!({ "will_delete": counts(10, 3), "kept_locked": counts(1, 1) }),
        false
    )
    .is_err());
    assert!(same_deletion(
        &approved,
        &json!({ "will_delete": counts(10, 2), "kept_locked": counts(0, 1) }),
        false
    )
    .is_err());
    assert!(same_deletion(
        &approved,
        &json!({ "will_delete": counts(11, 3), "kept_locked": counts(0, 0) }),
        true
    )
    .is_ok());
    assert!(same_deletion(
        &approved,
        &json!({ "will_delete": counts(10, 2), "kept_locked": counts(0, 0) }),
        true
    )
    .is_err());
}

#[test]
fn outcome_of_call_reads_errors_as_the_design_says() {
    let fine = Called::Answered(CallToolResult::json(&json!({})));
    let refused = Called::Answered(CallToolResult::error("no"));
    let before = Called::Before(CallToolResult::error("missing"));
    let erred = Called::Erred("dropped".into());
    assert_eq!(outcome_of_call(&fine, true), Outcome::Succeeded);
    assert_eq!(outcome_of_call(&refused, true), Outcome::Failed);
    assert_eq!(outcome_of_call(&before, true), Outcome::Failed);
    assert_eq!(outcome_of_call(&erred, true), Outcome::MayHaveChanged);
    assert_eq!(
        outcome_of_call(&erred, false),
        Outcome::Failed,
        "a read changes nothing"
    );
}

/// Every adapter's arguments against the registry's own validator, so a change in a
/// tool's schema fails here first.
#[test]
fn every_adapter_matches_its_tools_schema() {
    let board = Path::new("/b/board.kicad_pcb");
    let route = Path::new("/b/route");
    let valid = |tool: &str, args: &Value| {
        let def = find_tool(tool).unwrap_or_else(|| panic!("{tool} is registered"));
        assert!(
            crate::mcp::handler::first_missing_required(&def.input_schema, args).is_none(),
            "{tool}: {args}"
        );
        if let Err(error) = def.input_validator.validate(args) {
            panic!("{tool}: {args}: {error}");
        }
    };
    valid("delete_all_routing", &delete_preview_args(board, true));
    valid(
        "delete_all_routing",
        &delete_apply_args(board, false, "rev"),
    );
    valid("export_specctra_dsn", &export_args(board, route));
    valid(
        "route_specctra_dsn",
        &route_args(route, "m", Path::new("/fr/fastroute")),
    );
    valid("save_project", &json!({}));
    valid("refill_zones", &json!({ "board": board }));
    valid(
        "run_drc",
        &json!({ "board": board, "sync_live_board": true, "refill_zones": true }),
    );
    valid(
        "add_zone",
        &crate::pour_plan::add_zone_arguments(board, &zone()),
    );
    let placement = json!({ "apply_with": { "tool": "set_component_placements", "arguments": { "board": board, "placements": [{ "reference": "R1", "x": 1.0, "y": 2.0, "rotation": 0.0 }] } } });
    valid(
        "set_component_placements",
        &placement_args(&placement).unwrap(),
    );
    let body = route_body();
    for tool in ["plan_specctra_ses_import", "apply_specctra_ses"] {
        valid(tool, &step_arguments(&body, tool).unwrap());
    }
    let follow = route_follow_ups(&body);
    valid(
        "plan_route_refinement",
        &follow.plane.unwrap()[0]["arguments"],
    );
    let mut rules = follow.rules[0].clone();
    rules["dry_run"] = json!(true);
    valid("create_custom_drc_rule", &rules);
    rules["dry_run"] = json!(false);
    rules["expected_plan_revision"] = json!("rev");
    valid("create_custom_drc_rule", &rules);
    for batch in stitch_batches(
        &json!({ "proposals": [{ "kind": "stitching", "operations": [{ "tool": "route_batch", "arguments": { "vias": [{ "net_name": "GND", "x": 1.0, "y": 2.0, "drill": 0.3, "pad_size": 0.6 }] } }] }] }),
        &json!(board),
    ) {
        valid("route_batch", &batch);
    }
}

#[test]
fn placements_at_reads_position_and_turn() {
    let live = "(kicad_pcb (footprint \"R\" (at 10 5 -270) (property \"Reference\" \"R1\")))";
    let at = |x: f64, rotation: f64| json!({ "placements": [{ "reference": "R1", "x": x, "y": 5.0, "rotation": rotation }] });
    assert!(placements_at(live, &at(10.0, 90.0)));
    assert!(!placements_at(live, &at(10.01, 90.0)));
    assert!(!placements_at(live, &at(10.0, 0.0)));
    assert!(!placements_at(
        live,
        &json!({ "placements": [{ "reference": "R9", "x": 10.0, "y": 5.0, "rotation": 90.0 }] })
    ));
}

#[test]
fn locked_routing_on_a_moved_parts_net_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let plan = dir.path().join("p3.json");
    write_value(
        &plan,
        &json!({ "apply_with": { "arguments": { "placements": [{ "reference": "R1" }] } } }),
    )
    .unwrap();
    let live = |locked: &str| {
        format!(
        "(kicad_pcb (net 1 \"GND\") (net 2 \"SIG\") (footprint \"R\" (property \"Reference\" \"R1\") (pad \"1\" smd (net 1 \"GND\"))) (segment (net 2) {locked}) (via (net 1) {locked}))"
    )
    };
    assert!(locked_on_moved(&live("(locked yes)"), &plan));
    assert!(!locked_on_moved(&live(""), &plan));
    let other = "(kicad_pcb (net 2 \"SIG\") (footprint \"R\" (property \"Reference\" \"R1\") (pad \"1\" smd (net 1 \"GND\"))) (segment (net 2) (locked yes)))";
    assert!(!locked_on_moved(other, &plan));
}

#[tokio::test]
async fn registry_calls_refuse_bad_arguments_before_the_handler() {
    let ctx = ctx();
    for (tool, args) in [
        ("delete_all_routing", json!({})),
        (
            "delete_all_routing",
            json!({ "board": "/b/x.kicad_pcb", "dry_run": "yes" }),
        ),
        (
            "delete_all_routing",
            json!({ "board": "/b/x.kicad_pcb", "unknown": 1 }),
        ),
        ("no_such_tool", json!({})),
    ] {
        let called = registry_call(&ctx, tool, &args).await;
        assert!(
            matches!(called, Called::Before(_)),
            "{tool} {args}: {called:?}"
        );
        assert_eq!(outcome_of_call(&called, true), Outcome::Failed);
    }
    assert_eq!(
        ctx.observer.snapshot().await.total_calls,
        0,
        "a call refused before its handler is not recorded"
    );
}

#[test]
fn a_route_with_only_the_reference_group_has_no_plane_stitching() {
    let mut body = route_body();
    let steps = body["completion"]["next_steps"].as_array_mut().unwrap();
    // Drop the plane group (steps 6 to 8): a board with no plane nets.
    steps.retain(|step| !matches!(step["step"].as_u64(), Some(6..=8)));
    let follow = route_follow_ups(&body);
    assert!(follow.plane.is_none());
    assert_eq!(follow.reference.as_ref().unwrap()[0]["step"], 9);
}

#[test]
fn a_noop_result_is_a_success_for_the_flow() {
    let noop = Called::Answered(CallToolResult::json(&json!({ "status": "noop" })));
    assert_eq!(outcome_of_call(&noop, true), Outcome::Unchanged);
}

/// #131: `delete_all_routing`'s apply takes B's checkpoint too. F's own checkpoint before
/// L2 holds the same saved files, so the guard reuses it: one checkpoint, not two.
#[test]
fn the_guards_checkpoint_after_the_flows_is_the_same_one() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.kicad_pcb");
    std::fs::write(&board, BOARD_TEXT).unwrap();
    std::fs::write(board.with_extension("kicad_pro"), "{}").unwrap();
    let project = board.with_extension("kicad_pro");
    let flow =
        crate::tools::checkpoints::ensure_checkpoint(&project, "full flow: before L2").unwrap();
    let guard = crate::tools::checkpoints::ensure_checkpoint(
        &project,
        &crate::tools::board_safety::checkpoint_label("delete_all_routing"),
    )
    .unwrap();
    assert!(flow.made_now);
    assert_eq!(guard.id, flow.id);
    assert!(!guard.made_now);
}

// ─── FF4, owner decision 4: the search with and without the pours (ignored) ──

/// The placement search on the public placement fixture at effort m, once with each
/// candidate routed as it is and once with P2's pours (`WithPours`). Each winner then
/// gets its own P4 pours and one final route at m, and the two final routes are compared
/// by the judge's pour key (lower is better). Needs `KONNECT_FASTROUTE`, and scratch
/// `KICAD_CONFIG_HOME` and `KONNECT_STATE_DIR`. Writes `ff4.json` into the state folder.
#[test]
#[ignore = "starts fastroute and kicad-cli: needs KONNECT_FASTROUTE and scratch KICAD_CONFIG_HOME and KONNECT_STATE_DIR"]
fn ff4_the_search_with_and_without_the_pours_on_the_public_fixture() {
    use crate::tools::placement_converge::{run_folder, LiveInput};
    use konnect_ipc::types::{IpcEffectiveRoutingRules, IpcRoutingRules};
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../konnect-sexp/tests/fixtures/placement")
        .canonicalize()
        .unwrap();
    let user = tempfile::tempdir().unwrap();
    let user = user.path().canonicalize().unwrap();
    for name in [
        "placement_fixture.kicad_pcb",
        "placement_fixture.kicad_pro",
        "placement_fixture.kicad_sch",
    ] {
        std::fs::copy(fixture.join(name), user.join(name)).unwrap();
    }
    let board = user.join("placement_fixture.kicad_pcb");
    let text = std::fs::read_to_string(&board).unwrap();
    let tree = konnect_sexp::parse_sexp(&text).unwrap();
    let index = konnect_sexp::board::PcbConnectivityIndex::build(&tree);
    let rules: IpcEffectiveRoutingRules = index
        .nets()
        .into_iter()
        .filter(|net| !net.is_empty() && !index.pads_of_net(net).is_empty())
        .map(|net| {
            (
                net.to_string(),
                IpcRoutingRules {
                    class_name: "Default".to_string(),
                    constituents: Vec::new(),
                    track_width_mm: Some(0.2),
                    clearance_mm: Some(0.2),
                    via_diameter_mm: Some(0.6),
                    via_drill_mm: Some(0.3),
                },
            )
        })
        .collect();
    let state =
        PathBuf::from(std::env::var("KONNECT_STATE_DIR").expect("a scratch KONNECT_STATE_DIR"));
    let kicad_cli = crate::kicad_install::find_cli("")
        .expect("kicad-cli is installed")
        .display()
        .to_string();
    let ctx = Arc::new(ToolContext::new(
        ServerConfig {
            kicad_cli,
            kicad_binary: String::new(),
            ipc_address: String::new(),
            project_dir: None,
            jlcpcb_db_path: None,
            auto_load_toolsets: false,
            eager_toolsets: false,
        },
        Arc::new(ToolRouter::new()),
    ));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let as_named = RolesAnswer::AsNamed(AsNamed::AsNamed);
    let pours = |text: &str, folder: &Path| -> Vec<ProposedZone> {
        std::fs::create_dir_all(folder).unwrap();
        let (result, plan) = runtime
            .block_on(crate::tools::pour_plan_tool::plan_text(
                &ctx,
                &board,
                text,
                Some(&as_named),
                Some(folder),
            ))
            .unwrap();
        assert!(!result.is_error, "{:?}", body_of(&result));
        plan.map(|plan| plan.proposals).unwrap_or_default()
    };
    let stripped = crate::place_route::strip_routing(&text).unwrap();
    let root = state.join(format!("ff4-{}", stamp_ms()));
    let before = pours(&stripped, &root.join("p2"));
    assert!(!before.is_empty(), "the fixture's ground pour is planned");
    let live = LiveInput {
        text: text.clone(),
        rules,
    };
    let mut record = serde_json::Map::new();
    let mut finals = Vec::new();
    for with in [false, true] {
        let name = if with { "with_pours" } else { "without_pours" };
        let folder =
            run_folder(&root, &text, &format!("{}{}", stamp_ms(), u8::from(with))).unwrap();
        let fastroute = runtime
            .block_on(locate_fastroute(None, &folder))
            .unwrap()
            .unwrap_or_else(|refusal| panic!("fastroute: {refusal:?}"));
        let prepared = prepare_search(
            &ctx,
            &folder,
            &board,
            &live,
            crate::route_search::Effort::M,
            &fastroute.path,
        )
        .unwrap();
        let routing: Arc<dyn RouteCall> = if with {
            let plain_text = std::fs::read_to_string(&prepared.inputs.board).unwrap();
            let zoned_text = crate::pour_plan::with_zones(&plain_text, &before).unwrap();
            let zoned_board =
                crate::scratch_board::copy_project(&board, &folder.join("zoned")).unwrap();
            std::fs::write(&zoned_board, &zoned_text).unwrap();
            let mut inputs = prepared.inputs.clone();
            inputs.options = crate::place_route::export_options(&zoned_board);
            inputs.board = zoned_board;
            Arc::new(WithPours {
                zoned: Arc::new(crate::place_search::PlaceRouteCall {
                    inputs,
                    ctx: Arc::clone(&prepared.isolated),
                }),
                zones: before.clone(),
                plain_text,
                zoned_text,
            })
        } else {
            Arc::clone(&prepared.routing)
        };
        let started = std::time::Instant::now();
        let planned = crate::tools::placement_converge::Planned::of(&text).unwrap();
        let (body, outcome, winner) = runtime
            .block_on(plan_with_outcome(
                &prepared.plan,
                routing,
                &board,
                &planned,
                &SearchProgress::default(),
            ))
            .unwrap();
        let search_s = started.elapsed().as_secs_f64();
        let id = winner.expect("a winner");
        let found = outcome.records.iter().find(|r| r.id == id).unwrap();
        let (_, winner_text) = crate::place_search::rebuild(&outcome.model, found).unwrap();
        let after = pours(&winner_text, &folder.join("p4"));
        let final_text = crate::pour_plan::with_zones(&winner_text, &after).unwrap();
        let route = crate::place_search::PlaceRouteCall {
            inputs: prepared.inputs.clone(),
            ctx: Arc::clone(&prepared.isolated),
        };
        let (result, _) = runtime.block_on(route.route(9_999, final_text, folder.join("final")));
        let crate::place_judge::RouteResult::Routed {
            gates,
            measures,
            status,
            ..
        } = result
        else {
            panic!("{name}: the final route did not route: {result:?}");
        };
        record.insert(
            name.to_string(),
            json!({
                "search_s": (search_s * 10.0).round() / 10.0,
                "status": body["status"],
                "candidates": body["candidates"],
                "winner_change": body["winner"]["change"],
                "zones_p4": after.len(),
                "final": {
                    "status": status,
                    "gates_pass": gates.all_pass(),
                    "unconnected": measures.unconnected,
                    "signal_mm": measures.signal_mm,
                    "power_mm": measures.power_mm,
                    "vias_off_pour": measures.vias_off_pour,
                    "copper_quality": measures.copper_quality,
                    "stitching_vias": measures.stitching_vias,
                },
                "folder": folder.display().to_string(),
            }),
        );
        finals.push((gates, measures));
    }
    let keys = crate::pour_rank::pour_keys(&[
        (&finals[0].1, &finals[0].0, true),
        (&finals[1].1, &finals[1].0, true),
    ]);
    record.insert(
        "pour_key".into(),
        json!({ "without_pours": keys[0].to_vec(), "with_pours": keys[1].to_vec() }),
    );
    let verdict = if keys[1] <= keys[0] {
        "with_pours_at_least_as_good"
    } else {
        "without_pours_better"
    };
    record.insert("verdict".into(), json!(verdict));
    std::fs::write(
        root.join("ff4.json"),
        serde_json::to_string_pretty(&Value::Object(record)).unwrap(),
    )
    .unwrap();
    println!("FF4: {verdict}; record {}", root.join("ff4.json").display());
}

// ─── The review's fixes ──────────────────────────────────────────────────────

#[tokio::test]
async fn m1_a_stop_before_the_intent_is_resumed() {
    let mut planned = planned();
    let fake = Fake::new();
    // L1 reads fine; then KiCad drops for L2's three reads.
    fake.script("delete_all_routing", ok(json!({ "status": "ready", "plan_revision": "r", "will_delete": counts(10, 2), "kept_locked": counts(0, 0) })));
    planned
        .flow
        .put(StepRecord::new(Step::Unchanged, StepState::Done));
    fake.live_failures.store(3, Ordering::SeqCst);
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.at, Some(Step::DeleteRouting));
    assert!(flow
        .stop_reason
        .as_deref()
        .unwrap()
        .starts_with("kicad_unreachable"));
    assert_eq!(flow.record(Step::DeleteRouting).unwrap().before, None);
    resume_step(&planned.state, &mut planned.flow, &fake)
        .await
        .unwrap()
        .unwrap();
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.state, FlowState::Done, "{:?}", flow.stop_reason);
}

#[tokio::test]
async fn m2_every_apply_attempt_gets_a_new_candidate_path() {
    let (mut planned, fake) = stopped_at(
        Step::ApplySession,
        Called::Answered(CallToolResult::error("rolled back")),
        "apply_specctra_ses",
    )
    .await;
    resume_step(&planned.state, &mut planned.flow, &fake)
        .await
        .unwrap()
        .unwrap();
    leg(&mut planned, &fake).await;
    let files: Vec<PathBuf> =
        std::fs::read_dir(flow_folder(&planned.state, &planned.flow.flow_id).join("steps"))
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().contains("apply_specctra_ses"))
            .collect();
    let paths: BTreeSet<String> = files
        .iter()
        .map(|f| {
            read_json(f).unwrap()["arguments"]["candidate_output_path"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(paths.len(), files.len(), "{paths:?}");
    assert!(files.len() >= 2);
    assert!(
        paths
            .iter()
            .all(|p| p.starts_with("/r/candidate-") && p.ends_with(".kicad_pcb")),
        "{paths:?}"
    );
}

#[test]
fn m3_the_pours_signature_sees_every_change_of_the_zones() {
    let pours = |net: &str, clean: bool| {
        json!({ "status": "planned", "check": { "clean": clean },
        "proposals": [{ "net": net, "layer": "F.Cu", "add_zone": { "net_name": net, "layer": "F.Cu" }, "fill": { "pieces": 1 } }] })
    };
    let base = pours_signature(&pours("VBUS", true));
    assert_eq!(base, pours_signature(&pours("VBUS", true)));
    assert_ne!(
        base,
        pours_signature(&pours("5V", true)),
        "same count, another net"
    );
    assert_ne!(
        base,
        pours_signature(&pours("VBUS", false)),
        "same zone, another check"
    );
}

#[tokio::test]
async fn m5_a_refusal_after_an_earlier_write_of_the_step_is_uncertain() {
    let mut planned = planned();
    let fake = Fake::new();
    fake.script("add_zone", ok(json!({ "ok": true })));
    fake.script(
        "add_zone",
        Called::Answered(CallToolResult::error("no outline")),
    );
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.at, Some(Step::AddZones));
    assert_eq!(
        flow.record(Step::AddZones).unwrap().state,
        StepState::Uncertain
    );
    assert_eq!(flow.board_state, BoardState::Uncertain);
}

#[tokio::test]
async fn m6_no_save_is_sent_when_the_board_cannot_be_read() {
    let fake = Fake::new();
    fake.live_failures.store(3, Ordering::SeqCst);
    assert!(verified_save(&fake).await.is_err());
    assert!(!fake.calls().contains(&"save_project".to_string()));
}

#[test]
fn m1_a_stitching_step_resumes_from_its_last_round() {
    let mut record = StepRecord::new(Step::PlaneStitch, StepState::Uncertain);
    record.before = Some("start".into());
    record.rounds = vec!["round-1".into(), "round-2".into()];
    assert_eq!(may_resume(&record, false, "round-2"), Ok(Resume::RunAgain));
    assert!(
        may_resume(&record, false, "round-1").is_err(),
        "only the last round's board"
    );
    let mut flow = planned().flow;
    let mut again = StepRecord::new(Step::PlaneStitch, StepState::NotRun);
    again.before = Some("round-2".into());
    flow.put(again);
    assert_eq!(flow.expected_before(Step::PlaneStitch), "round-2");
}

#[tokio::test]
async fn m2_a_leg_ended_between_steps_keeps_the_done_record() {
    let mut planned = planned();
    let fake = Fake::new();
    fake.route.lock().unwrap()["completion"]["judged"]["all_gates_pass"] = json!(false);
    leg(&mut planned, &fake).await;
    // As if the server stopped right after R3 was saved.
    planned.flow.state = FlowState::Running;
    planned.flow.at = Some(Step::ImportCheck);
    save_flow(&planned.state, &planned.flow).unwrap();
    let mut flow = planned.flow.clone();
    leg_ended(&planned.state, &mut flow, "stopped");
    assert_eq!(flow.state, FlowState::Stopped);
    assert_eq!(
        flow.record(Step::ImportCheck).unwrap().state,
        StepState::Done
    );
    resume_step(&planned.state, &mut flow, &fake)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(flow.state, FlowState::Running);
}

#[test]
fn m3_gate_r_needs_its_roles_and_m5_keeping_the_placement_plans_the_pours_again() {
    let mut flow = planned().flow;
    flow.approvals.pour_roles = None;
    flow.facts.named_only = vec!["VCC".to_string()];
    flow.gate = Some(Gate::Roles);
    flow.plan_revision = Some("rev".into());
    assert!(apply_answer(&mut flow.clone(), Gate::Roles, &Approvals::default(), "rev").is_err());
    let roles = Approvals {
        pour_roles: Some(RolesAnswer::AsNamed(AsNamed::AsNamed)),
        ..Approvals::default()
    };
    apply_answer(&mut flow, Gate::Roles, &roles, "rev").unwrap();
    assert!(flow.facts.named_only.is_empty());

    let mut flow = planned().flow;
    flow.approvals = Approvals::default();
    flow.facts.change = Some("small".into());
    flow.facts.p4_on_winner = true;
    flow.plan_revision = Some("rev".into());
    let keep = Approvals {
        placement: Some(PlacementAnswer::KeepCurrent),
        delete_routing: Some(DeleteAnswer::Delete),
        pours: Some(PoursAnswer::Add),
        routing: Some(RoutingAnswer::ApplyIfComplete),
        ..Approvals::default()
    };
    apply_answer(&mut flow, Gate::Plan, &keep, "rev").unwrap();
    assert!(
        flow.record(Step::PoursAfter).is_none(),
        "P4 runs again on the current placement"
    );
    assert!(apply_answer(&mut planned().flow, Gate::Plan, &keep, "stale").is_err());
}

// ─── Review round 2 ──────────────────────────────────────────────────────────

#[tokio::test]
async fn a_resume_that_plans_again_keeps_only_the_starts_approvals() {
    let mut flow = planned().flow;
    flow.start_approvals = Approvals {
        routing: Some(RoutingAnswer::ApplyBest),
        ..Approvals::default()
    };
    flow.state = FlowState::Stopped;
    flow.at = Some(Step::Placement);
    flow.records.retain(|r| r.step < Step::Placement);
    flow.stop_reason = Some("failed: the search failed".into());
    let fake = Fake::new();
    let state = tempfile::tempdir().unwrap();
    resume_step(state.path(), &mut flow, &fake)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(flow.attempt, 2);
    assert_eq!(flow.approvals.routing, Some(RoutingAnswer::ApplyBest));
    assert_eq!(
        flow.approvals.placement, None,
        "gate A asks the new plan again"
    );
    assert_eq!(flow.approvals.pours, None);
    assert!(
        flow.approvals.pour_roles.is_some(),
        "the roles answered at gate R stay"
    );
}

#[test]
fn switching_to_apply_after_the_pours_were_planned_on_the_current_board_plans_again() {
    let mut flow = planned().flow;
    flow.facts.p4_on_winner = false;
    flow.approvals.placement = Some(PlacementAnswer::KeepCurrent);
    flow.plan_revision = Some("rev".into());
    flow.gate_body = Some(json!({ "refused": "delete_routing" }));
    let apply = Approvals {
        placement: Some(PlacementAnswer::Apply),
        ..Approvals::default()
    };
    apply_answer(&mut flow, Gate::Plan, &apply, "rev").unwrap();
    assert!(flow.record(Step::PoursAfter).is_none());
}

#[tokio::test]
async fn the_refused_gate_names_the_key_and_offers_no_plain_yes() {
    let mut planned = planned();
    planned.flow.approvals.delete_routing = Some(DeleteAnswer::Keep);
    let fake = Fake::new();
    let flow = leg(&mut planned, &fake).await;
    let view = view(&planned.state, &flow, None);
    assert_eq!(view["answer_with"]["answer"], Value::Null);
    assert!(view["next_steps"][0]
        .as_str()
        .unwrap()
        .contains("delete_routing"));
}

#[tokio::test]
async fn a_leg_ended_after_a_read_step_does_not_redo_it() {
    let mut planned = planned();
    let fake = Fake::new();
    fake.route.lock().unwrap()["completion"]["judged"]["all_gates_pass"] = json!(false);
    leg(&mut planned, &fake).await;
    let mut flow = planned.flow.clone();
    flow.state = FlowState::Stopped;
    flow.at = Some(Step::Route);
    resume_step(&planned.state, &mut flow, &fake)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(flow.route_attempt, 1, "the route is not run again");
    assert!(flow.done(Step::Route));
}

#[tokio::test]
async fn a_read_refused_after_a_saved_round_is_not_uncertain() {
    let mut planned = planned();
    let fake = Fake::new();
    // F2a: round 1 adopts a via and DRC still finds one unconnected; round 2's planner
    // read is refused.
    fake.script(
        "run_drc",
        ok(json!({ "by_rule": [{ "rule": "unconnected_items", "count": 1 }] })),
    );
    fake.script("plan_route_refinement", ok(json!({ "proposals": [{ "kind": "stitching", "operations": [{ "tool": "route_batch", "arguments": { "vias": [{ "x": 1.0 }] } }] }] })));
    fake.script(
        "plan_route_refinement",
        Called::Answered(CallToolResult::error("busy")),
    );
    let flow = leg(&mut planned, &fake).await;
    assert_eq!(flow.at, Some(Step::PlaneStitch));
    let record = flow.record(Step::PlaneStitch).unwrap();
    assert_eq!(record.state, StepState::Failed, "{:?}", record.why);
    assert_eq!(record.rounds.len(), 1);
}

// ─── The mutants' misses ─────────────────────────────────────────────────────

#[test]
fn wait_s_of_600_is_accepted() {
    assert!(matches!(
        read_request(&json!({ "run_id": "1700000000000-0123456789ab", "wait_s": 600 })),
        Ok(Request::Report { .. })
    ));
}

#[tokio::test]
async fn a_read_pauses_only_between_tries() {
    let fake = Fake::new();
    fake.live_failures.store(2, Ordering::SeqCst);
    with_tries(&fake, || fake.live_text()).await.unwrap();
    assert_eq!(fake.pauses.load(Ordering::SeqCst), 2);
    let fine = Fake::new();
    with_tries(&fine, || fine.live_text()).await.unwrap();
    assert_eq!(fine.pauses.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_refused_save_does_not_verify_even_on_an_unchanged_board() {
    let fake = Fake::new();
    fake.script(
        "save_project",
        Called::Answered(CallToolResult::error("no board")),
    );
    assert!(verified_save(&fake)
        .await
        .unwrap_err()
        .contains("save_project refused"));
}

#[tokio::test]
async fn already_done_sends_its_own_arguments_and_reads_refusals_as_not_done() {
    let mut planned = planned();
    planned.flow.approvals.delete_routing = Some(DeleteAnswer::DeleteIncludingLocked);
    let fake = Fake::new();
    already_done(Step::DeleteRouting, &planned.state, &planned.flow, &fake)
        .await
        .unwrap();
    let sent = fake.sent.lock().unwrap().last().cloned().unwrap();
    assert_eq!(sent.1["include_locked"], true);
    // A refused preview is not "done", whatever its body says.
    let fake = Fake::new();
    let mut noop_error = CallToolResult::json(&json!({ "status": "noop" }));
    noop_error.is_error = true;
    fake.script("delete_all_routing", Called::Answered(noop_error.clone()));
    assert!(
        !already_done(Step::DeleteRouting, &planned.state, &planned.flow, &fake)
            .await
            .unwrap()
    );
    write_value(
        &route_folder(&planned.state, &planned.flow).join(ROUTE_FILE),
        &route_body(),
    )
    .unwrap();
    fake.script("create_custom_drc_rule", Called::Answered(noop_error));
    assert!(
        !already_done(Step::LengthRules, &planned.state, &planned.flow, &fake)
            .await
            .unwrap()
    );
}

#[test]
fn a_leg_ended_mid_call_is_uncertain_only_for_a_writing_step() {
    let planned = planned();
    for (step, state, uncertain) in [
        (Step::ApplyPlacement, StepState::Calling, true),
        (Step::ApplyPlacement, StepState::Failed, false),
        (Step::Route, StepState::Calling, false),
    ] {
        let mut flow = planned.flow.clone();
        flow.state = FlowState::Running;
        flow.at = Some(step);
        flow.records.retain(|r| r.step < step);
        let mut record = StepRecord::new(step, state);
        record.before = Some("b".into());
        flow.put(record);
        save_flow(&planned.state, &flow).unwrap();
        leg_ended(&planned.state, &mut flow, "stopped");
        assert_eq!(flow.state, FlowState::Stopped);
        assert_eq!(
            flow.board_state == BoardState::Uncertain,
            uncertain,
            "{step:?} {state:?}"
        );
    }
}

#[tokio::test]
async fn an_answer_waits_for_the_leg_before_it_and_a_failed_leg_stops_the_flow() {
    let planned = planned();
    let jobs = Jobs::default();
    let board = PathBuf::from(&planned.flow.board);
    let leg = |id: &str| -> PathBuf {
        let folder = run_folder_of(&planned.state, &FLOW_LEGS, id);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    };
    // A leg that has not ended.
    let running = "1700000000001-aaaaaaaaaaaa";
    let claim = claim_board(&planned.state, &FLOW_LEGS, &board, running)
        .unwrap()
        .unwrap();
    let (release, wait) = tokio::sync::oneshot::channel::<()>();
    jobs.start(
        &FLOW_LEGS,
        claim,
        &board,
        leg(running),
        Arc::new(FlowProgress::default()),
        async move {
            let _ = wait.await;
            Ok(json!({}))
        },
    )
    .unwrap();
    let mut flow = planned.flow.clone();
    flow.legs = vec![running.to_string()];
    assert!(last_leg_ended(&jobs, &planned.state, &mut flow)
        .await
        .is_err());
    let _ = release.send(());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(last_leg_ended(&jobs, &planned.state, &mut flow)
        .await
        .is_ok());
    // A leg that failed without recording the flow's end.
    let failed = "1700000000002-bbbbbbbbbbbb";
    let claim = claim_board(&planned.state, &FLOW_LEGS, &board, failed)
        .unwrap()
        .unwrap();
    jobs.start(
        &FLOW_LEGS,
        claim,
        &board,
        leg(failed),
        Arc::new(FlowProgress::default()),
        async move { anyhow::bail!("the leg broke") },
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut flow = planned.flow.clone();
    flow.state = FlowState::Running;
    flow.at = Some(Step::ApplyPlacement);
    flow.legs = vec![failed.to_string()];
    save_flow(&planned.state, &flow).unwrap();
    last_leg_ended(&jobs, &planned.state, &mut flow)
        .await
        .unwrap();
    assert_eq!(flow.state, FlowState::Stopped);
    assert!(flow.stop_reason.unwrap().contains("the leg broke"));
}

fn gated(gate: Gate) -> FlowRecord {
    let mut flow = planned().flow;
    flow.gate = Some(gate);
    flow.plan_revision = Some("rev".into());
    flow
}

#[test]
fn apply_answer_keeps_each_gates_own_rules() {
    // A key already set cannot change at gate R (only a refused gate A allows it).
    let mut flow = gated(Gate::Roles);
    let other = Approvals {
        placement: Some(PlacementAnswer::KeepCurrent),
        ..Approvals::default()
    };
    assert!(apply_answer(&mut flow, Gate::Roles, &other, "rev").is_err());
    // Gate A with keys missing is refused.
    let mut flow = gated(Gate::Plan);
    flow.approvals = Approvals::default();
    let partial = Approvals {
        placement: Some(PlacementAnswer::Apply),
        ..Approvals::default()
    };
    assert!(
        apply_answer(&mut flow, Gate::Plan, &partial, "rev")
            .unwrap_err()
            .is_error
    );
    // Roles changed at gate A: P4 again, and the placement says it was judged with the
    // earlier roles.
    let mut flow = gated(Gate::Plan);
    let roles = Approvals {
        pour_roles: Some(RolesAnswer::Map(BTreeMap::from([(
            "GND".to_string(),
            GivenRole::Ground,
        )]))),
        ..Approvals::default()
    };
    apply_answer(&mut flow, Gate::Plan, &roles, "rev").unwrap();
    assert!(flow.record(Step::PoursAfter).is_none());
    assert!(flow
        .record(Step::Placement)
        .unwrap()
        .not_checked
        .iter()
        .any(|n| n.contains("earlier pour roles")));
}

#[tokio::test]
async fn resume_keeps_the_right_records() {
    let fake = Fake::new();
    let state = tempfile::tempdir().unwrap();
    // A plan step: only the pre-flight stays.
    let mut flow = planned().flow;
    flow.state = FlowState::Stopped;
    flow.at = Some(Step::PoursAfter);
    flow.records.retain(|r| r.step < Step::PoursAfter);
    resume_step(state.path(), &mut flow, &fake)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        flow.records.iter().map(|r| r.step).collect::<Vec<_>>(),
        [Step::Preflight]
    );
    // L1: its own record goes, the plan stays.
    let mut flow = planned().flow;
    flow.state = FlowState::Stopped;
    flow.at = Some(Step::Unchanged);
    flow.put(StepRecord::new(Step::Unchanged, StepState::Failed));
    resume_step(state.path(), &mut flow, &fake)
        .await
        .unwrap()
        .unwrap();
    assert!(flow.record(Step::Unchanged).is_none());
    assert!(flow.done(Step::PoursAfter));
    // A read step stopped as board_changed is refused.
    let mut flow = planned().flow;
    flow.state = FlowState::Stopped;
    flow.at = Some(Step::Unchanged);
    flow.put(StepRecord::new(Step::Unchanged, StepState::Failed));
    flow.stop_reason = Some("board_changed: elsewhere".into());
    assert!(resume_step(state.path(), &mut flow, &fake)
        .await
        .unwrap()
        .is_err());
    // A writing step stopped as board_changed before its intent runs again once the
    // board is back where the step before left it.
    let mut flow = planned().flow;
    flow.state = FlowState::Stopped;
    flow.at = Some(Step::DeleteRouting);
    flow.put(StepRecord::new(Step::Unchanged, StepState::Done));
    flow.put(StepRecord::new(Step::DeleteRouting, StepState::Failed));
    flow.stop_reason = Some("board_changed: restored since".into());
    resume_step(state.path(), &mut flow, &fake)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(flow.state, FlowState::Running);
}
