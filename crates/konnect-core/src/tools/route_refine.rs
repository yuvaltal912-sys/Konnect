//! Q1 second opinion: a bounded plan against saved copper, never a writer.
use anyhow::{bail, ensure, Context, Result};
use konnect_sexp::board_geometry::{BoardGeometry, Kind, Shape};
use konnect_sexp::parser::{parse_sexp, SexpNode};
use konnect_sexp::planar::Pt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::{get_path, layout_quality as audit, ToolContext, ToolDef};
use crate::mcp::protocol::CallToolResult;
use crate::tool;

#[path = "route_refine_distance.rs"]
mod distance_kernel;
#[path = "route_refine_effort.rs"]
mod effort;
#[path = "route_refine_geometry.rs"]
mod geometry;
use geometry::{Copper, Rules, Scene};

const MARGIN: f64 = 0.01;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const RESPONSE_BYTES: usize = 48 * 1024;
/// Unresolved findings listed in detail; every one is still counted.
const BLOCKED_DETAIL: usize = 200;
/// Copper objects one plan may create or rewrite: the `route_batch` bound.
const MAX_OPERATIONS: usize = 200;
const OPERATION_LIMIT: &str = "copper operation limit reached";
/// Search order. Corner repairs come first: stitching then sees their copper.
const KINDS: [&str; 3] = ["t_junction", "chamfer", "stitching"];
const RESPONSE_LIMIT: &str = "response size limit reached";
const BLOCKED_LIMIT: &str = "blocked detail limit reached";

#[derive(Debug)]
struct SearchLimit(&'static str);
impl std::fmt::Display for SearchLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for SearchLimit {}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn nm(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}
fn dist(a: Pt, b: Pt) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}
fn point(p: Pt) -> Value {
    json!({"x":p.0,"y":p.1})
}

pub(super) fn tools() -> Vec<ToolDef> {
    vec![tool!("plan_route_refinement", "Read-only second opinion on the saved board: proposes same-net stitching vias, perpendicular acute-T repairs and pad-exit chamfers. Refill and save first. Returns exact preimages, ordinary-tool operations and conservative geometry coverage; never applies, saves or refills. Saved fills may be stale; native DRC and a new audit remain required after selective adoption.", json!({
        "type":"object","properties":{
            "board":{"type":"string"},
            "kinds":{"type":"array","minItems":1,"uniqueItems":true,"items":{"type":"string","enum":["stitching","t_junction","chamfer"]}},
            "nets":{"type":"array","uniqueItems":true,"items":{"type":"string","minLength":1}},
            "max_proposals":{"type":"integer","minimum":1,"maximum":200,"default":50},
            "effort":{"type":"string","enum":["quick","standard","deep"],"default":"standard","description":"Bounded computational effort, mutually exclusive with budget. Safety thresholds are unchanged."},
            "budget":effort::budget_schema(),
            "search_radius_mm":{"type":"number","minimum":0.1,"maximum":20,"default":3.0},
            "grid_step_mm":{"type":"number","minimum":0.05,"maximum":1,"default":0.25},
            "chamfer_mm":{"type":"number","minimum":0.01,"maximum":5,"default":0.5}
        },"required":["board"],"additionalProperties":false
    }), |args,ctx|async move { handle(args,ctx).await })]
}

#[derive(Clone)]
struct Options {
    kinds: BTreeSet<String>,
    nets: BTreeSet<String>,
    max_proposals: usize,
    max_candidates: usize,
    effort: effort::Effort,
    radius: f64,
    step: f64,
    chamfer: f64,
}
impl Options {
    fn read(v: &Value) -> Result<Self> {
        ensure!(v.is_object(), "arguments must be an object");
        for k in v.as_object().unwrap().keys() {
            ensure!(
                [
                    "board",
                    "kinds",
                    "nets",
                    "max_proposals",
                    "effort",
                    "budget",
                    "search_radius_mm",
                    "grid_step_mm",
                    "chamfer_mm"
                ]
                .contains(&k.as_str()),
                "unknown argument {k}"
            );
        }
        let size = |key: &str, default, max| -> Result<usize> {
            match v.get(key) {
                None => Ok(default),
                Some(n) => {
                    let n = effort::whole(n).context("limit must be an integer")?;
                    ensure!(n > 0 && n <= max as u64, "{key} outside supported bounds");
                    Ok(n as usize)
                }
            }
        };
        let number = |key: &str, default, min, max| -> Result<f64> {
            let n = v
                .get(key)
                .map(|n| n.as_f64().context("dimension must be a number"))
                .transpose()?
                .unwrap_or(default);
            ensure!(
                n.is_finite() && n >= min && n <= max,
                "{key} outside supported bounds"
            );
            Ok(n)
        };
        let strings = |key: &str| -> Result<BTreeSet<String>> {
            let mut r = BTreeSet::new();
            if let Some(a) = v.get(key) {
                for n in a.as_array().context("filter must be an array")? {
                    let n = n
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .context("empty/non-string filter")?;
                    ensure!(r.insert(n.to_string()), "duplicate filter");
                }
            }
            Ok(r)
        };
        let mut kinds = strings("kinds")?;
        if v.get("kinds").is_none() {
            kinds = KINDS.map(str::to_string).into();
        }
        ensure!(
            !kinds.is_empty() && kinds.iter().all(|k| KINDS.contains(&k.as_str())),
            "unsupported/empty proposal kind"
        );
        let effort = effort::Effort::read(v)?;
        Ok(Self {
            kinds,
            nets: strings("nets")?,
            max_proposals: size("max_proposals", 50, 200)?,
            max_candidates: effort.limits.max_candidates,
            effort,
            radius: number("search_radius_mm", 3., 0.1, 20.)?,
            step: number("grid_step_mm", 0.25, 0.05, 1.)?,
            chamfer: number("chamfer_mm", 0.5, 0.01, 5.)?,
        })
    }
    fn includes(&self, net: &str) -> bool {
        self.nets.is_empty() || self.nets.contains(net)
    }
    fn json(&self) -> Value {
        json!({"kinds":self.kinds,"nets":self.nets,"max_proposals":self.max_proposals,"max_candidates":self.max_candidates,"search_radius_mm":self.radius,"grid_step_mm":self.step,"chamfer_mm":self.chamfer,"geometry_margin_mm":MARGIN,"coordinate_resolution_mm":0.000001,"max_pair_checks":self.effort.limits.max_pair_checks,"max_copper_operations":MAX_OPERATIONS,"max_response_bytes":RESPONSE_BYTES,"max_blocked_listed":BLOCKED_DETAIL})
    }
}

struct Snapshot {
    board: PathBuf,
    content: String,
    sidecars: crate::routing_context::Snapshot,
    intent: Option<String>,
}
impl Snapshot {
    fn model<T>(
        &self,
        tree: &SexpNode,
        geometry: &BoardGeometry,
        params: &audit::Params,
        visit: impl FnOnce(&audit::Model<'_>) -> Result<T>,
    ) -> Result<T> {
        audit::with_model_inputs(
            &self.board,
            tree,
            geometry,
            params,
            Some(&audit::CapturedInputs {
                project: self.sidecars.project.as_deref(),
                intent: self.intent.as_deref(),
            }),
            visit,
        )
    }
    fn read(board: &Path) -> Result<Self> {
        let m = std::fs::symlink_metadata(board)?;
        ensure!(
            m.file_type().is_file() && m.len() <= MAX_BYTES as u64,
            "board must be a regular file at most 64 MiB"
        );
        let content = konnect_sexp::writer::read_consistent(board)?;
        ensure!(content.len() <= MAX_BYTES, "board exceeds 64 MiB");
        let sidecars = crate::routing_context::Snapshot::read(board)?;
        let intent_path =
            super::design_intent::design_intent_path(board.parent().context("no parent")?);
        let intent = optional(&intent_path)?;
        if let Some(source) = &intent {
            match super::design_intent::read_design_intent(&intent_path) {
                super::design_intent::IntentRead::Loaded { text, .. } if &text == source => {}
                _ => bail!("invalid or changed design intent"),
            }
        }
        Ok(Self {
            board: board.to_path_buf(),
            content,
            sidecars,
            intent,
        })
    }
    fn ensure_unchanged(&self) -> Result<()> {
        ensure!(
            std::fs::symlink_metadata(&self.board)?
                .file_type()
                .is_file(),
            "board changed type during planning"
        );
        ensure!(
            konnect_sexp::writer::read_consistent(&self.board)? == self.content,
            "board changed during planning"
        );
        self.sidecars.ensure_unchanged(&self.board)?;
        ensure!(
            optional(&super::design_intent::design_intent_path(
                self.board.parent().unwrap()
            ))? == self.intent,
            "design intent changed during planning"
        );
        Ok(())
    }
    fn source(&self) -> Value {
        json!({"kind":"saved_board","board":self.board,"board_sha256":digest(self.content.as_bytes()),"project_sha256":self.sidecars.project.as_ref().map(|v|digest(v.as_bytes())),"rules_sha256":self.sidecars.rules.as_ref().map(|v|digest(v.as_bytes())),"intent_sha256":self.intent.as_ref().map(|v|digest(v.as_bytes())),"unsaved_editor_changes":"excluded","fill_freshness":"not_established; refill and save before planning"})
    }
}
fn optional(path: &Path) -> Result<Option<String>> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
        Ok(m) => {
            ensure!(
                m.file_type().is_file() && m.len() <= 4_194_304,
                "sidecar must be a regular file at most 4 MiB"
            );
            Ok(Some(konnect_sexp::writer::read_consistent(path)?))
        }
    }
}

async fn handle(args: &Value, _ctx: &ToolContext) -> Result<CallToolResult> {
    let args = args.clone();
    let started = std::time::Instant::now();
    tokio::task::spawn_blocking(move||{
    let result=(||{let options=Options::read(&args)?;let board=get_path(&args,"board")?;let snapshot=Snapshot::read(&board)?;plan(&snapshot,&options,started)})();
    match result {Ok(v)=>Ok(CallToolResult::text(serde_json::to_string(&v)?)),Err(e)=>{let mut r=CallToolResult::text(json!({"schema_version":1,"planner":"route_refinement","mutated":false,"status":"blocked","proposals":[],"blocked":[{"reason":bounded(&format!("{e:#}"))}],"coverage":{"complete":false},"elapsed_ms":started.elapsed().as_millis() as u64,"effort":effort::Effort::read(&args).ok().map(|e|e.json())}).to_string());r.is_error=true;Ok(r)}}
}).await?
}

/// A refusal can repeat the caller's own argument, net name or a token of the
/// board file. It is cut to a fixed length so no input can grow the response.
fn bounded(reason: &str) -> String {
    const SHOWN: usize = 160;
    match reason.char_indices().nth(SHOWN) {
        Some((cut, _)) => format!("{}…", &reason[..cut]),
        None => reason.to_string(),
    }
}

fn plan(snapshot: &Snapshot, options: &Options, started: std::time::Instant) -> Result<Value> {
    let tree = parse_sexp(&snapshot.content)?;
    ensure!(tree.head() == Some("kicad_pcb"), "not a board");
    let geo = BoardGeometry::from_tree(&tree);
    let params = audit::Params::from_args(&json!({}))
        .map_err(|_| anyhow::anyhow!("audit defaults invalid"))?;
    let mut result = snapshot.model(&tree, &geo, &params, |m| {
        let rules = Rules::read(&snapshot.sidecars)?;
        let mut scene = Scene::read(&tree, m, rules, options.effort.limits, started)?;
        for n in &options.nets {
            ensure!(
                scene.items.iter().any(|i| i.net.as_deref() == Some(n)),
                "unknown net {n}"
            );
        }
        let before = audit::planner_corners(m).metrics;
        let preparation_ms = started.elapsed().as_millis() as u64;
        let mut planner = Planner::new(&snapshot.board, options);
        let mut phases = Vec::new();
        for kind in KINDS {
            if !options.kinds.contains(kind) {
                continue;
            }
            // Every limit is shared, so a search that stopped leaves nothing
            // for a later kind. Say so instead of implying it was examined.
            if planner.stopped {
                phases.push(json!({"kind":kind,"state":"not_started"}));
                continue;
            }
            let before = (planner.considered, scene.meter.pairs, scene.meter.work);
            let result = match kind {
                "t_junction" => planner.corners(&mut scene, m, true),
                "chamfer" => planner.corners(&mut scene, m, false),
                _ => planner.stitching(&mut scene, m),
            };
            planner.phase_result(kind, result)?;
            phases.push(json!({
                "kind":kind,
                "state":if planner.stopped {"stopped"} else {"complete"},
                "candidates":planner.considered - before.0,
                "pair_checks":scene.meter.pairs - before.1,
                "geometry_work":scene.meter.work - before.2}));
        }
        validate_proposal_prefix(&snapshot.board, &planner.proposals, options.max_proposals)?;
        let total = planner.proposals.len();
        let blocked_listed = planner.blocked.len();
        // Counts the whole checked plan, including proposals a response
        // has no room to list.
        let mut by_kind = BTreeMap::new();
        for proposal in &planner.proposals {
            *by_kind
                .entry(proposal["kind"].as_str().unwrap_or_default().to_string())
                .or_insert(0usize) += 1;
        }
        let mut result = json!({
            "schema_version":1,"planner":"route_refinement","mutated":false,
            "source":snapshot.source(),
            "status":"planned",
            "parameters":options.json(),
            "before":{"corners":before},
            "proposals":planner.proposals,
            "blocked":planner.blocked,
            "coverage":{
                "complete":!planner.stopped,
                "phases":phases,
                "candidates_considered":planner.considered,
                "candidates_rejected":planner.rejected,
                "pair_checks":scene.meter.pairs,
                "geometry_work":scene.meter.work,
                "max_geometry_work":options.effort.limits.max_geometry_work,
                "proposals_total":total,"proposals_listed":total,
                "proposals_by_kind":by_kind,
                "blocked_total":planner.blocked_total,"blocked_listed":blocked_listed,
                "rule_policy":scene.rules.json()},
            "truncated":false,
            "not_checked":["saved fill freshness","KiCad general conditional rules","post-refill connectivity/thermal relief","current capacity","unsaved live editor changes"],
            "adoption":"Compare independently with audit_layout_quality under the same settings. Proposals form one ordered cumulative plan: each depends on its predecessor, and a subset needs a fresh geometry check. Re-observe the exact source fingerprints and every affected object's preimage before selective ordinary-tool edits. Calls span multiple undo commits; inspect partial application. Save/refill, native DRC and audit after each adopted group."});
        let mut effort = options.effort.json();
        effort["used"] = json!({"candidates":planner.considered,"pair_checks":scene.meter.pairs,"geometry_work":scene.meter.work,"preparation_ms":preparation_ms});
        result["effort"] = effort;
        let mut exhaustion = planner.exhaustion;
        while serde_json::to_vec(&result)?.len() > RESPONSE_BYTES - 512 {
            if result["blocked"].as_array().is_some_and(|v| !v.is_empty()) {
                result["blocked"].as_array_mut().unwrap().pop();
            } else if !result["proposals"].as_array().unwrap().is_empty() {
                result["proposals"].as_array_mut().unwrap().pop();
            } else {
                bail!("response envelope exceeds budget");
            }
            if !exhaustion.iter().any(|s| s == RESPONSE_LIMIT) {
                exhaustion.push(RESPONSE_LIMIT.into());
            }
        }
        let listed = result["proposals"].as_array().unwrap().len();
        let blocked_listed = result["blocked"].as_array().unwrap().len();
        result["coverage"]["proposals_listed"] = json!(listed);
        result["coverage"]["blocked_listed"] = json!(blocked_listed);
        // Partial covers every way work can remain: a stopped search, an
        // unresolved finding, or detail this response had no room to list.
        let unlisted = listed < total || blocked_listed < planner.blocked_total;
        result["truncated"] = json!(planner.stopped || unlisted);
        if planner.stopped || unlisted || planner.blocked_total > 0 {
            result["status"] = json!("partial");
        }
        result["exhaustion"] = json!(exhaustion);
        Ok(result)
    })?;
    snapshot.ensure_unchanged()?;
    let elapsed = started.elapsed().as_millis() as u64;
    result["elapsed_ms"] = json!(elapsed);
    result["effort"]["used"]["elapsed_ms"] = json!(elapsed);
    Ok(result)
}

fn validate_proposal_prefix(board: &Path, proposals: &[Value], max: usize) -> Result<()> {
    ensure!(proposals.len() <= max, "proposal count invariant");
    let tools = crate::router::registry::tools_for("pcb_routing")
        .context("routing catalogue unavailable")?;
    let mut ids = BTreeSet::new();
    let mut previous = Value::Null;
    let mut objects = 0usize;
    for proposal in proposals {
        let id = proposal["id"]
            .as_str()
            .context("proposal identity missing")?;
        ensure!(ids.insert(id), "duplicate proposal identity");
        ensure!(
            proposal["depends_on"] == previous,
            "proposal dependency is not an earlier prefix"
        );
        for op in proposal["operations"]
            .as_array()
            .context("operations missing")?
        {
            let name = op["tool"].as_str().context("operation name missing")?;
            ensure!(
                matches!(name, "modify_trace" | "route_batch"),
                "unexpected operation"
            );
            ensure!(
                op["arguments"]["board"] == json!(board),
                "operation board differs from snapshot"
            );
            let tool = tools
                .iter()
                .find(|t| t.name == name)
                .context("operation tool unavailable")?;
            ensure!(
                tool.input_validator.is_valid(&op["arguments"]),
                "operation does not match normal tool schema"
            );
            if name == "modify_trace" {
                objects += 1;
            } else {
                objects += op["arguments"]["vias"].as_array().map_or(0, Vec::len);
                for track in op["arguments"]["tracks"].as_array().into_iter().flatten() {
                    objects += track["points"]
                        .as_array()
                        .context("track points missing")?
                        .len()
                        - 1;
                }
            }
        }
        previous = json!(id);
    }
    ensure!(objects <= MAX_OPERATIONS, "copper operation bound exceeded");
    Ok(())
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn route_refine_model_uses_captured_sidecars_even_if_the_path_changes() {
        let dir = tempfile::tempdir().unwrap();
        let board = dir.path().join("b.kicad_pcb");
        std::fs::write(&board,"(kicad_pcb (layers (0 \"F.Cu\" signal) (31 \"B.Cu\" signal)) (net 1 \"SIG\") (segment (start 2 2) (end 8 2) (width 0.2) (layer \"F.Cu\") (net 1) (uuid abc)))").unwrap();
        std::fs::write(
            board.with_extension("kicad_pro"),
            json!({"net_settings":{"netclass_assignments":{"SIG":"Power"}}}).to_string(),
        )
        .unwrap();
        let snapshot = Snapshot::read(&board).unwrap();
        std::fs::write(board.with_extension("kicad_pro"), "{}").unwrap();
        let tree = parse_sexp(&snapshot.content).unwrap();
        let geo = BoardGeometry::from_tree(&tree);
        let params = audit::Params::from_args(&json!({})).unwrap_or_else(|_| panic!("params"));
        let classes = snapshot
            .model(&tree, &geo, &params, |m| {
                Ok(m.roles.netclass.get("SIG").cloned())
            })
            .unwrap();
        assert_eq!(
            classes,
            Some(vec!["Power".to_string()]),
            "model reread a different sidecar revision"
        );
        assert!(
            snapshot.ensure_unchanged().is_err(),
            "changed path must invalidate returned plan"
        );
    }

    #[test]
    fn route_refine_model_uses_the_captured_intent_even_if_the_path_changes() {
        let dir = tempfile::tempdir().unwrap();
        let board = dir.path().join("b.kicad_pcb");
        std::fs::write(&board,"(kicad_pcb (layers (0 \"F.Cu\" signal) (31 \"B.Cu\" signal)) (net 1 \"SIG\") (segment (start 2 2) (end 8 2) (width 0.2) (layer \"F.Cu\") (net 1) (uuid abc)))").unwrap();
        let intent = super::super::design_intent::design_intent_path(dir.path());
        std::fs::create_dir_all(intent.parent().unwrap()).unwrap();
        let read_as = |snapshot: &Snapshot| {
            let tree = parse_sexp(&snapshot.content).unwrap();
            let geo = BoardGeometry::from_tree(&tree);
            let params = audit::Params::from_args(&json!({})).unwrap_or_else(|_| panic!("params"));
            snapshot
                .model(&tree, &geo, &params, |m| Ok(m.roles.intent))
                .unwrap()
        };
        // Captured while absent, written afterwards.
        let absent = Snapshot::read(&board).unwrap();
        std::fs::write(&intent, json!({"schema_version":1}).to_string()).unwrap();
        assert_eq!(read_as(&absent), "absent", "model read a later intent");
        assert!(absent.ensure_unchanged().is_err());
        // Captured while valid, replaced afterwards.
        let loaded = Snapshot::read(&board).unwrap();
        assert!(loaded.ensure_unchanged().is_ok());
        std::fs::write(&intent, "{").unwrap();
        assert_eq!(read_as(&loaded), "loaded", "model reread a changed intent");
        assert!(loaded.ensure_unchanged().is_err());
        // An intent that is already unusable refuses the snapshot itself.
        assert!(Snapshot::read(&board).is_err());
    }

    /// The planner answers the audit's ring tests from its cached point
    /// indexes. The plane analysis must not notice, apart from the cost.
    #[test]
    fn route_refine_metered_plane_probe_matches_the_unmetered_ring_scan() {
        use super::super::route_refine_tests::{reference_change, Fixture};
        let fixture = Fixture::new(&reference_change());
        let snapshot = Snapshot::read(&fixture.board).unwrap();
        let tree = parse_sexp(&snapshot.content).unwrap();
        let geo = BoardGeometry::from_tree(&tree);
        let params = audit::Params::from_args(&json!({})).unwrap_or_else(|_| panic!("params"));
        snapshot
            .model(&tree, &geo, &params, |m| {
                let rules = Rules::read(&snapshot.sidecars)?;
                let mut scene = Scene::read(
                    &tree,
                    m,
                    rules,
                    effort::Limits::standard(),
                    std::time::Instant::now(),
                )?;
                let facts = |checks: &[audit::CheckOut]| {
                    checks
                        .iter()
                        .map(|c| {
                            (
                                c.id,
                                c.findings
                                    .iter()
                                    .map(|f| (f.measured.clone(), f.at))
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect::<Vec<_>>()
                };
                let mut scanned = 0usize;
                let plain = audit::planner_planes(m, &mut |work| {
                    scanned += work;
                    Ok(())
                })?;
                let indexed =
                    audit::planner_planes(m, &mut geometry::PlaneProbe::new(&mut scene, m)?)?;
                assert_eq!(facts(&indexed), facts(&plain));
                assert_eq!(facts(&plain)[0].1.len(), 1, "one unstitched layer change");
                assert!(
                    scene.meter.work > 0 && scene.meter.work < scanned,
                    "indexed {} against scanned {scanned}",
                    scene.meter.work
                );
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn route_refine_search_limit_retains_checked_prefix_but_not_invalid_geometry() {
        let options = Options::read(&json!({"board":"unused"})).unwrap();
        let mut planner = Planner::new(Path::new("unused"), &options);
        planner.proposals = vec![json!({"id":"checked-prefix"})];
        planner.operations = 1;
        planner
            .phase_result(
                "stitching",
                Err(SearchLimit("geometry work budget exhausted").into()),
            )
            .unwrap();
        assert!(planner.stopped);
        assert!(!planner.room());
        assert_eq!(planner.proposals[0]["id"], "checked-prefix");
        assert_eq!(planner.exhaustion, ["geometry work budget exhausted"]);
        assert!(planner
            .phase_result("stitching", Err(anyhow::anyhow!("unreadable copper")))
            .is_err());
    }

    #[test]
    fn route_refine_operation_bound_stops_the_search_and_counts_only_real_operations() {
        let options = Options::read(&json!({"board":"unused"})).unwrap();
        let track =
            |id: &str, a, b| Copper::track(id.into(), "GND".into(), "F.Cu".into(), 1, a, b, 0.2);
        let kept = track("kept", (5., 5.), (10., 5.));
        let mut p = Planner::new(Path::new("unused"), &options);
        let mut scene = geometry::test_scene(vec![kept.clone()]);
        p.operations = MAX_OPERATIONS - 1;
        // One unchanged leg plus one new track is one operation: it still fits.
        let fits = p
            .candidate(
                &mut scene,
                "chamfer",
                vec![kept.clone()],
                vec![kept.clone(), track("new", (10., 5.), (13., 5.))],
            )
            .unwrap();
        assert!(matches!(fits, Outcome::Accepted), "{fits:?}");
        assert_eq!(p.operations, MAX_OPERATIONS);
        validate_proposal_prefix(p.board, &p.proposals, options.max_proposals).unwrap();
        // Two new tracks do not fit: a stop, never a refusal of this copper.
        let mut p = Planner::new(Path::new("unused"), &options);
        let mut scene = geometry::test_scene(Vec::new());
        p.operations = MAX_OPERATIONS - 1;
        let full = p
            .candidate(
                &mut scene,
                "chamfer",
                Vec::new(),
                vec![
                    track("a", (5., 5.), (6., 5.)),
                    track("b", (6., 5.), (7., 5.)),
                ],
            )
            .unwrap();
        assert!(matches!(full, Outcome::Full), "{full:?}");
        assert!(p.stopped && p.proposals.is_empty() && p.rejected.is_empty());
        assert_eq!((p.considered, p.blocked_total), (0, 0));
        assert_eq!(p.exhaustion, [OPERATION_LIMIT]);
    }

    /// The stitch search visits each grid ring once. It must visit exactly the
    /// perimeter cells of the square, in the order a full scan would meet them.
    #[test]
    fn route_refine_stitch_ring_is_the_square_perimeter_in_scan_order() {
        for radius in 0..=24i32 {
            let mut expected = Vec::new();
            for dx in -radius..=radius {
                for dy in -radius..=radius {
                    if dx.abs() == radius || dy.abs() == radius {
                        expected.push((dx, dy));
                    }
                }
            }
            let cells = ring(radius);
            assert_eq!(cells, expected, "radius {radius}");
            assert_eq!(
                cells.len(),
                if radius == 0 { 1 } else { 8 * radius as usize }
            );
        }
    }

    #[test]
    fn route_refine_copper_changed_by_an_earlier_proposal_is_not_edited_again() {
        let options = Options::read(&json!({"board":"unused"})).unwrap();
        let track =
            |id: &str, a, b| Copper::track(id.into(), "GND".into(), "F.Cu".into(), 1, a, b, 0.2);
        let kept = track("kept", (5., 5.), (10., 5.));
        let mut p = Planner::new(Path::new("unused"), &options);
        let mut scene = geometry::test_scene(vec![kept.clone()]);
        p.changed.insert("kept".into());
        let outcome = p
            .candidate(
                &mut scene,
                "chamfer",
                vec![kept.clone()],
                vec![kept, track("new", (10., 5.), (13., 5.))],
            )
            .unwrap();
        let reason = "conflicts with an earlier proposal";
        assert!(
            matches!(&outcome, Outcome::Rejected(r) if r == reason),
            "{outcome:?}"
        );
        assert!(p.proposals.is_empty());
        assert_eq!(p.rejected.get(reason), Some(&1));
    }

    /// Accepting a proposal costs work. It is charged before the proposal is
    /// recorded, so a search one unit short records nothing.
    #[test]
    fn route_refine_accepting_a_proposal_is_paid_for_before_it_is_recorded() {
        let options = Options::read(&json!({"board":"unused"})).unwrap();
        let track =
            |id: &str, a, b| Copper::track(id.into(), "GND".into(), "F.Cu".into(), 1, a, b, 0.2);
        let scene = || geometry::test_scene(vec![track("far", (20., 20.), (25., 20.))]);
        let new = track("new", (5., 5.), (8., 5.));
        let cost = scene().accept_cost(std::slice::from_ref(&new));
        assert_eq!(cost, 1 + 2, "one saved item and the new track's two points");
        let mut probe = scene();
        let start = probe.meter.work;
        probe.safe(&new, &BTreeSet::new(), &[], &[]).unwrap();
        let checks = probe.meter.work - start;
        let run = |spent: usize| {
            let mut p = Planner::new(Path::new("unused"), &options);
            let mut scene = scene();
            scene.meter.work += spent;
            let start = scene.meter.work;
            let outcome = p.candidate(&mut scene, "chamfer", Vec::new(), vec![new.clone()]);
            (outcome, scene.meter.work - start, p.proposals.len())
        };
        let (outcome, total, recorded) = run(0);
        assert!(matches!(outcome.unwrap(), Outcome::Accepted));
        assert_eq!((total, recorded), (checks + cost, 1), "acceptance unpaid");
        let room = options.effort.limits.max_geometry_work - scene().meter.work;
        let (outcome, _, recorded) = run(room - total + 1);
        assert!(outcome.unwrap_err().downcast_ref::<SearchLimit>().is_some());
        assert_eq!(recorded, 0, "an unpaid proposal was recorded");
        let (outcome, _, recorded) = run(room - total);
        assert!(matches!(outcome.unwrap(), Outcome::Accepted));
        assert_eq!(recorded, 1);
    }

    /// Before a plan is returned, every proposal is checked once more against
    /// the ordinary tools' schemas, the dependency order and the object bound.
    #[test]
    fn route_refine_final_self_check_refuses_a_malformed_plan() {
        let board = Path::new("/scratch/b.kicad_pcb");
        let modify = json!({"tool":"modify_trace","arguments":{"board":board,"uuid":"u","x1":0.0,"y1":0.0,"x2":1.0,"y2":0.0}});
        let batch = |vias: usize, points: usize| {
            let via = json!({"net_name":"GND","x":1.0,"y":1.0,"drill":0.3,"pad_size":0.6});
            let tracks = if points == 0 {
                json!([])
            } else {
                let points: Vec<Value> =
                    (0..points).map(|i| json!({"x":i as f64,"y":0.0})).collect();
                json!([{"net_name":"GND","layer":"F.Cu","width":0.2,"points":points}])
            };
            json!({"tool":"route_batch","arguments":{"board":board,"tracks":tracks,"vias":vec![via; vias]}})
        };
        let chain = |plans: Vec<Vec<Value>>| -> Vec<Value> {
            let mut previous = Value::Null;
            plans
                .into_iter()
                .enumerate()
                .map(|(i, operations)| {
                    let id = format!("p{i}");
                    let proposal = json!({"id":id,"depends_on":previous,"operations":operations});
                    previous = json!(id);
                    proposal
                })
                .collect()
        };
        let check = |proposals: &[Value], max: usize| {
            validate_proposal_prefix(board, proposals, max).map_err(|e| format!("{e:#}"))
        };
        let good = chain(vec![
            vec![modify.clone()],
            vec![batch(1, 2), modify.clone()],
        ]);
        assert_eq!(check(&good, 2), Ok(()));
        assert_eq!(check(&good, 1).unwrap_err(), "proposal count invariant");
        // The object bound, by each way an operation adds copper objects.
        let bound = "copper operation bound exceeded";
        let half = MAX_OPERATIONS / 2;
        for (fits, over) in [
            (
                vec![modify.clone(); MAX_OPERATIONS],
                vec![modify.clone(); MAX_OPERATIONS + 1],
            ),
            // One batch is capped by the tool's own schema, so the bound is
            // reached across two operations.
            (
                vec![batch(half, 0), batch(half, 0)],
                vec![batch(half, 0), batch(half + 1, 0)],
            ),
            (
                vec![batch(0, half + 1), batch(0, half + 1)],
                vec![batch(0, half + 1), batch(0, half + 2)],
            ),
        ] {
            assert_eq!(check(&chain(vec![fits]), 200), Ok(()));
            assert_eq!(check(&chain(vec![over]), 200).unwrap_err(), bound);
        }
        // Structure.
        let mut twice = good.clone();
        twice[1]["id"] = json!("p0");
        assert_eq!(check(&twice, 2).unwrap_err(), "duplicate proposal identity");
        let mut unordered = good.clone();
        unordered[1]["depends_on"] = Value::Null;
        assert_eq!(
            check(&unordered, 2).unwrap_err(),
            "proposal dependency is not an earlier prefix"
        );
        let mut foreign = good.clone();
        foreign[0]["operations"][0]["tool"] = json!("delete_trace");
        assert_eq!(check(&foreign, 2).unwrap_err(), "unexpected operation");
        let mut elsewhere = good.clone();
        elsewhere[0]["operations"][0]["arguments"]["board"] = json!("/scratch/other.kicad_pcb");
        assert_eq!(
            check(&elsewhere, 2).unwrap_err(),
            "operation board differs from snapshot"
        );
        let mut invalid = good.clone();
        invalid[0]["operations"][0]["arguments"]["x1"] = json!("left");
        assert_eq!(
            check(&invalid, 2).unwrap_err(),
            "operation does not match normal tool schema"
        );
    }

    /// The sources are read once. A board that changes while the plan is made
    /// invalidates the plan, however valid its proposals were.
    #[test]
    fn route_refine_a_board_changed_during_planning_refuses_the_plan() {
        use super::super::route_refine_tests::{dogleg, Fixture};
        let fixture = Fixture::new(&dogleg());
        let options = Options::read(&json!({"board":fixture.board,"kinds":["chamfer"]})).unwrap();
        let snapshot = Snapshot::read(&fixture.board).unwrap();
        let planned = plan(&snapshot, &options, std::time::Instant::now()).unwrap();
        assert_eq!(planned["proposals"].as_array().unwrap().len(), 1);
        let text = std::fs::read_to_string(&fixture.board).unwrap();
        std::fs::write(&fixture.board, text + "\n").unwrap();
        let refused = plan(&snapshot, &options, std::time::Instant::now()).unwrap_err();
        assert_eq!(format!("{refused:#}"), "board changed during planning");
    }

    #[test]
    fn route_refine_late_budget_failure_never_appends_a_partly_checked_candidate() {
        let options = Options::read(&json!({"board":"unused"})).unwrap();
        let mut p = Planner::new(Path::new("unused"), &options);
        let track = |id: &str, a, b, net: &str| {
            Copper::track(id.into(), net.into(), "F.Cu".into(), 1, a, b, 0.2)
        };
        let mut scene = geometry::test_scene(vec![
            track("far", (20., 20.), (25., 20.), "OTHER"),
            track("danger", (11., 9.), (11., 11.), "OTHER"),
        ]);
        assert!(matches!(
            p.candidate(
                &mut scene,
                "chamfer",
                Vec::new(),
                vec![track("safe", (3., 3.), (4., 3.), "GND")]
            )
            .unwrap(),
            Outcome::Accepted
        ));
        let prefix = p.proposals.clone();
        scene.meter.work = scene.meter.limits.max_geometry_work - 21;
        let failed = p.candidate(
            &mut scene,
            "chamfer",
            Vec::new(),
            vec![track("unsafe", (10., 10.), (12., 10.), "GND")],
        );
        assert!(failed
            .as_ref()
            .unwrap_err()
            .downcast_ref::<SearchLimit>()
            .is_some());
        p.phase_result("chamfer", failed.map(|_| ())).unwrap();
        assert_eq!(p.proposals, prefix, "half-validated candidate escaped");
        validate_proposal_prefix(p.board, &p.proposals, options.max_proposals).unwrap();
    }
}

/// Grid cells at Chebyshev distance `radius` from the origin: the perimeter of
/// the square, in the order a column-by-column scan of the square meets them.
fn ring(radius: i32) -> Vec<(i32, i32)> {
    let mut cells = Vec::with_capacity(8 * radius.max(1) as usize);
    for dx in -radius..=radius {
        if dx.abs() == radius {
            cells.extend((-radius..=radius).map(|dy| (dx, dy)));
        } else {
            cells.extend([(dx, -radius), (dx, radius)]);
        }
    }
    cells.dedup();
    cells
}

/// What became of one candidate that there was still room to consider.
#[derive(Debug)]
enum Outcome {
    Accepted,
    /// Failed a check; the reason names the first guard that refused it.
    Rejected(String),
    /// A count limit was reached first. The candidate was never examined.
    Full,
}

/// The guard that refused a candidate, without the object it named.
fn rejection_class(reason: &str) -> &str {
    [
        ("clearance to ", "clearance"),
        ("lost attachment to ", "lost attachment"),
        ("via-in-pad overlap with ", "via-in-pad overlap"),
        ("hole clearance to ", "hole clearance"),
    ]
    .iter()
    .find(|(prefix, _)| reason.starts_with(prefix))
    .map_or(reason, |(_, class)| class)
}

struct Planner<'a> {
    board: &'a Path,
    options: &'a Options,
    proposals: Vec<Value>,
    /// Unresolved findings listed in detail; `blocked_total` counts them all.
    blocked: Vec<Value>,
    blocked_total: usize,
    considered: usize,
    /// Candidates that failed a check, by the guard that refused them.
    rejected: BTreeMap<String, usize>,
    /// A limit ended the search early: later candidates were not examined.
    stopped: bool,
    changed: BTreeSet<String>,
    operations: usize,
    exhaustion: Vec<String>,
}

impl<'a> Planner<'a> {
    fn new(board: &'a Path, options: &'a Options) -> Self {
        Self {
            board,
            options,
            proposals: Vec::new(),
            blocked: Vec::new(),
            blocked_total: 0,
            considered: 0,
            rejected: BTreeMap::new(),
            stopped: false,
            changed: BTreeSet::new(),
            operations: 0,
            exhaustion: Vec::new(),
        }
    }
    fn exhaust(&mut self, reason: &str) {
        if !self.exhaustion.iter().any(|s| s == reason) {
            self.exhaustion.push(reason.into());
        }
    }
    /// A work, pair or time limit ends the search but keeps every proposal
    /// already checked in full. Any other error refuses the whole plan.
    fn phase_result(&mut self, _kind: &str, result: Result<()>) -> Result<()> {
        match result {
            Err(e) if e.downcast_ref::<SearchLimit>().is_some() => {
                self.stopped = true;
                self.exhaust(e.downcast_ref::<SearchLimit>().unwrap().0);
                Ok(())
            }
            other => other,
        }
    }
    fn room(&mut self) -> bool {
        let room = !self.stopped
            && self.considered < self.options.max_candidates
            && self.proposals.len() < self.options.max_proposals
            && self.operations < MAX_OPERATIONS;
        if !room {
            if self.considered >= self.options.max_candidates {
                self.exhaust("candidate limit reached");
            }
            if self.proposals.len() >= self.options.max_proposals {
                self.exhaust("proposal response limit reached");
            }
            if self.operations >= MAX_OPERATIONS {
                self.exhaust(OPERATION_LIMIT);
            }
            self.stopped = true;
        }
        room
    }
    /// Record an unresolved finding. The detail list is bounded; the count is
    /// not, and a full list never ends the search.
    fn block(&mut self, kind: &str, reason: String, ids: Vec<String>) {
        self.blocked_total += 1;
        if self.blocked.len() < BLOCKED_DETAIL {
            self.blocked
                .push(json!({"kind":kind,"reason":reason,"objects":ids}));
        } else {
            self.exhaust(BLOCKED_LIMIT);
        }
    }
    fn candidate(
        &mut self,
        scene: &mut Scene,
        kind: &str,
        old: Vec<Copper>,
        created: Vec<Copper>,
    ) -> Result<Outcome> {
        if !self.room() {
            return Ok(Outcome::Full);
        }
        // Copper listed unchanged costs no operation. The plan's object bound
        // is a stop, not a finding about this copper: nothing is examined.
        let objects = created
            .iter()
            .filter(|n| !old.iter().any(|o| o.id == n.id && o.shape == n.shape))
            .count();
        if self.operations + objects > MAX_OPERATIONS {
            self.exhaust(OPERATION_LIMIT);
            self.stopped = true;
            return Ok(Outcome::Full);
        }
        self.considered += 1;
        let removed: BTreeSet<_> = old.iter().map(|i| i.id.clone()).collect();
        let replaced = scene.indices(&removed);
        let check = (|| {
            ensure!(
                !removed.iter().any(|id| self.changed.contains(id)),
                "conflicts with an earlier proposal"
            );
            if !old.is_empty() {
                scene.attachments(&removed, &created)?;
            }
            let mut inherited = 0;
            for n in &created {
                // Copper that is unchanged, or lies inside the track it
                // shortens, is saved copper: it brings nothing new to certify.
                if old
                    .iter()
                    .any(|o| o.id == n.id && (o.shape == n.shape || n.within(o)))
                {
                    continue;
                }
                inherited += scene.safe(
                    n,
                    &removed,
                    &created
                        .iter()
                        .filter(|c| c.id != n.id)
                        .cloned()
                        .collect::<Vec<_>>(),
                    &replaced,
                )?;
            }
            // Paid here, so nothing can fail between recording and applying.
            let cost = scene.accept_cost(&created);
            scene.charge(cost)?;
            Ok(inherited)
        })();
        let inherited = match check {
            Ok(inherited) => inherited,
            Err(e) if e.downcast_ref::<SearchLimit>().is_some() => return Err(e),
            Err(e) => {
                let reason = format!("{e:#}");
                *self
                    .rejected
                    .entry(rejection_class(&reason).into())
                    .or_default() += 1;
                return Ok(Outcome::Rejected(reason));
            }
        };
        let mut tracks = Vec::new();
        let mut vias = Vec::new();
        let mut modifications = Vec::new();
        for n in &created {
            if let Some(previous) = old.iter().find(|o| o.id == n.id) {
                if previous.shape != n.shape {
                    let (a, b) = (n.shape.pts[0], n.shape.pts[1]);
                    modifications.push(json!({"tool":"modify_trace","arguments":{"board":self.board,"uuid":n.id,"x1":a.0,"y1":a.1,"x2":b.0,"y2":b.1}}));
                }
            } else if n.kind == "via" {
                vias.push(json!({"net_name":n.net,"x":n.shape.pts[0].0,"y":n.shape.pts[0].1,"drill":n.drill,"pad_size":2.*n.shape.radius}));
            } else {
                tracks.push(json!({"net_name":n.net,"layer":n.layer,"width":2.*n.shape.radius,"points":n.shape.pts.iter().map(|p|point(*p)).collect::<Vec<_>>()}));
            }
        }
        let mut operations = Vec::new();
        if !tracks.is_empty() || !vias.is_empty() {
            operations.push(json!({"tool":"route_batch","arguments":{"board":self.board,"tracks":tracks,"vias":vias}}));
        }
        operations.extend(modifications);
        let old_length: f64 = old
            .iter()
            .filter(|i| i.kind == "track")
            .map(|i| dist(i.shape.pts[0], i.shape.pts[1]))
            .sum();
        let new_length: f64 = created
            .iter()
            .filter(|i| i.kind == "track")
            .map(|i| dist(i.shape.pts[0], i.shape.pts[1]))
            .sum();
        let id=digest(serde_json::to_string(&json!({"kind":kind,"before":old.iter().map(Copper::preimage).collect::<Vec<_>>(),"operations":operations}))?.as_bytes());
        self.proposals.push(json!({"id":id,"kind":kind,"preconditions":old.iter().map(Copper::preimage).collect::<Vec<_>>(),"operations":operations,"checks":{"full_envelope_clearance":true,"existing_attachments_preserved":true,"replacement_connected":true,"geometry_margin_mm":MARGIN,"saved_clearances_inherited":inherited},"predicted":{"track_length_delta_mm":nm(new_length-old_length),"native_post_refill_result":"unverified"},"depends_on":self.proposals.last().map(|p|p["id"].clone())}));
        self.operations += objects;
        self.changed.extend(removed.iter().cloned());
        scene.accept(&removed, created);
        Ok(Outcome::Accepted)
    }
    fn corners(&mut self, scene: &mut Scene, m: &audit::Model, acute: bool) -> Result<()> {
        let kind = if acute { "t_junction" } else { "chamfer" };
        let mut rows = if acute {
            let report = super::dfm::check_board(m.geo, &super::dfm::fab_limits(None, None));
            report
                .issues
                .into_iter()
                .filter(|i| i.issue.starts_with("Acid trap"))
                .filter_map(|i| {
                    Some((
                        i.at["net"].as_str()?.to_string(),
                        i.at["layer"].as_str()?.to_string(),
                        (i.at["x"].as_f64()?, i.at["y"].as_f64()?),
                        Vec::new(),
                    ))
                })
                .collect()
        } else {
            audit::planner_junctions(m)
        };
        // Audit findings come from hash maps; candidate order affects cumulative
        // conflicts and IDs, so impose a total order before planning any edit.
        rows.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.cmp(&b.1))
                .then(a.2 .0.total_cmp(&b.2 .0))
                .then(a.2 .1.total_cmp(&b.2 .1))
        });
        let mut visited = BTreeSet::new();
        for (net, layer, at, _) in rows {
            if !self.room() {
                break;
            }
            if !self.options.includes(&net)
                || !visited.insert((
                    net.clone(),
                    layer.clone(),
                    (at.0 * 1000.).round() as i64,
                    (at.1 * 1000.).round() as i64,
                ))
            {
                continue;
            }
            scene.charge(scene.items.len())?;
            let legs: Vec<_> = scene
                .items
                .iter()
                .filter(|i| {
                    i.kind == "track"
                        && i.net.as_deref() == Some(&net)
                        && i.layer == layer
                        && i.shape.pts.iter().any(|p| dist(*p, at) <= 0.001)
                })
                .cloned()
                .collect();
            // The audit groups endpoints on a 1-um key. Use an actual source
            // endpoint for geometry; the rounded display position can turn an
            // exact perpendicular junction into an apparently oblique one.
            let at = legs
                .first()
                .and_then(|leg| {
                    leg.shape
                        .pts
                        .iter()
                        .min_by(|a, b| dist(**a, at).total_cmp(&dist(**b, at)))
                })
                .copied()
                .unwrap_or(at);
            let far = |t: &Copper| {
                if dist(t.shape.pts[0], at) < dist(t.shape.pts[1], at) {
                    t.shape.pts[1]
                } else {
                    t.shape.pts[0]
                }
            };
            let direction = |t: &Copper| {
                let p = far(t);
                let len = dist(p, at);
                ((p.0 - at.0) / len, (p.1 - at.1) / len)
            };
            let ids = legs.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
            let wanted = if acute { 3 } else { 2 };
            let unsupported = if legs.len() != wanted {
                Some(format!(
                    "unsupported junction: {} straight track ends meet here, {wanted} needed",
                    legs.len()
                ))
            } else if legs.iter().any(|t| t.locked) {
                Some("junction includes locked copper".into())
            } else if legs
                .iter()
                .any(|t| t.shape.pts.len() != 2 || dist(t.shape.pts[0], t.shape.pts[1]) <= 1e-6)
            {
                // A zero-length track has no direction to measure an angle from.
                Some("junction includes a degenerate track".into())
            } else {
                None
            };
            if let Some(reason) = unsupported {
                self.block(kind, reason, ids);
                continue;
            }
            if !acute {
                let mut pad_exit = false;
                for t in &legs {
                    if scene.touches_pad(far(t), t.shape.radius, &t.net, t.mask)? {
                        pad_exit = true;
                        break;
                    }
                }
                if !pad_exit {
                    self.block(
                        kind,
                        "right-angle bend is not a pad-exit dogleg".into(),
                        ids,
                    );
                    continue;
                }
            }
            let trunk = if acute {
                (0..3)
                    .flat_map(|i| (i + 1..3).map(move |j| (i, j)))
                    .find(|&(i, j)| {
                        let (a, b) = (direction(&legs[i]), direction(&legs[j]));
                        a.0 * b.0 + a.1 * b.1 < -0.999999
                    })
            } else {
                None
            };
            if acute && trunk.is_none() {
                self.block(kind, "acute junction has no straight trunk".into(), ids);
                continue;
            }
            // One row per junction: every setback tried and why each failed.
            let mut reasons: Vec<String> = Vec::new();
            let note = |reasons: &mut Vec<String>, reason: String| {
                if !reasons.contains(&reason) {
                    reasons.push(reason);
                }
            };
            let mut settled = false;
            for scale in [1., 0.5, 0.25, 0.125] {
                if !self.room() {
                    settled = true;
                    break;
                }
                let setback = self.options.chamfer * scale;
                let mut created = legs.clone();
                let replace_end = |t: &mut Copper, p: Pt| {
                    let n = if dist(t.shape.pts[0], at) < dist(t.shape.pts[1], at) {
                        0
                    } else {
                        1
                    };
                    t.shape.pts[n] = (nm(p.0), nm(p.1));
                };
                let (a, b, connector_width) = if let Some((i, j)) = trunk {
                    let k = (0..3).find(|&k| k != i && k != j).unwrap();
                    let u = direction(&legs[k]);
                    if dist(at, far(&legs[k])) <= setback + 0.01 {
                        note(&mut reasons, "branch shorter than the setback".into());
                        continue;
                    }
                    let a = (nm(at.0 + u.0 * setback), nm(at.1 + u.1 * setback));
                    let t = direction(&legs[i]);
                    let projected = (a.0 - at.0) * t.0 + (a.1 - at.1) * t.1;
                    if projected > dist(at, far(&legs[i])) || -projected > dist(at, far(&legs[j])) {
                        note(
                            &mut reasons,
                            "perpendicular foot is beyond the trunk".into(),
                        );
                        continue;
                    }
                    // A branch square to the trunk has its foot at the junction:
                    // the repair would move nothing. A trunk that counts as
                    // straight may still bend a little, so either leg counts.
                    let other = direction(&legs[j]);
                    let along = (a.0 - at.0) * other.0 + (a.1 - at.1) * other.1;
                    if projected.abs().min(along.abs()) <= 0.001 * setback {
                        note(
                            &mut reasons,
                            "branch already meets the trunk at a right angle".into(),
                        );
                        continue;
                    }
                    let b = (nm(at.0 + projected * t.0), nm(at.1 + projected * t.1));
                    let v = (b.0 - a.0, b.1 - a.1);
                    if dist(a, b) < 0.00001 || (v.0 * t.0 + v.1 * t.1).abs() > 0.000002 {
                        note(
                            &mut reasons,
                            "retermination is not perpendicular after 1 nm rounding".into(),
                        );
                        continue;
                    }
                    replace_end(&mut created[k], a);
                    (a, b, 2. * legs[k].shape.radius)
                } else {
                    if legs.iter().any(|t| dist(at, far(t)) <= setback + 0.01) {
                        note(&mut reasons, "segment shorter than the setback".into());
                        continue;
                    }
                    let (u, v) = (direction(&legs[0]), direction(&legs[1]));
                    if (u.0 * v.0 + u.1 * v.1).abs() > 0.000001 {
                        note(
                            &mut reasons,
                            "segments are not exactly perpendicular".into(),
                        );
                        continue;
                    }
                    let a = (nm(at.0 + u.0 * setback), nm(at.1 + u.1 * setback));
                    let b = (nm(at.0 + v.0 * setback), nm(at.1 + v.1 * setback));
                    replace_end(&mut created[0], a);
                    replace_end(&mut created[1], b);
                    (a, b, 2. * legs[0].shape.radius.max(legs[1].shape.radius))
                };
                let new_id = format!("proposal:{}:connector", self.proposals.len());
                created.push(Copper::track(
                    new_id,
                    net.clone(),
                    layer.clone(),
                    legs[0].mask,
                    a,
                    b,
                    connector_width,
                ));
                match self.candidate(scene, kind, legs.clone(), created)? {
                    Outcome::Accepted | Outcome::Full => {
                        settled = true;
                        break;
                    }
                    Outcome::Rejected(reason) => note(&mut reasons, reason),
                }
            }
            if !settled {
                self.block(kind, reasons.join("; "), ids);
            }
        }
        Ok(())
    }
    fn stitching(&mut self, scene: &mut Scene, m: &audit::Model) -> Result<()> {
        let checks = audit::planner_planes(m, &mut geometry::PlaneProbe::new(scene, m)?)?;
        let mut targets: Vec<(String, Pt, u64)> = Vec::new();
        if let Some(check) = checks.iter().find(|c| c.id == "3.2") {
            for f in &check.findings {
                let nets = f.measured["reference_nets"].as_array();
                let layers = f.measured["reference_layers"].as_array();
                if let (Some(nets), Some(layers), Some(at)) = (nets, layers, f.at) {
                    if nets.len() == 1 {
                        if let Some(net) = nets[0].as_str() {
                            let mut mask = 0u64;
                            for l in layers {
                                if let Some(i) = l.as_str().and_then(|l| m.layer_ix(l)) {
                                    mask |= 1 << i;
                                }
                            }
                            if mask.count_ones() >= 2 {
                                targets.push((net.into(), at, mask));
                            }
                        }
                    }
                }
            }
        }
        targets.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1 .0.total_cmp(&b.1 .0))
                .then(a.1 .1.total_cmp(&b.1 .1))
                .then(a.2.cmp(&b.2))
        });
        let nets: BTreeSet<_> = m
            .roles
            .ground
            .keys()
            .cloned()
            .chain(targets.iter().map(|t| t.0.clone()))
            .filter(|n| self.options.includes(n))
            .collect();
        for net in nets {
            if self.stopped {
                break;
            }
            let fills = scene.fills_of(&net)?;
            let mut components = scene.components(&net)?;
            // A via can only help where it joins two copper groups of the net
            // or serves a reference change. With neither left, stop looking.
            let pending = |components: &(Vec<usize>, BTreeSet<usize>),
                           targets: &[(String, Pt, u64)]| {
                fills
                    .iter()
                    .map(|&i| components.0[i])
                    .collect::<BTreeSet<_>>()
                    .len()
                    > 1
                    || targets.iter().any(|t| t.0 == net)
            };
            if pending(&components, &targets) {
                // Search near reference changes first, then near copper cut off
                // from the net's largest pad group, then everywhere else.
                let pad_items: BTreeMap<&str, usize> = scene
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| i.kind == "pad" && i.net.as_deref() == Some(&net))
                    .map(|(index, i)| (i.id.as_str(), index))
                    .collect();
                let mut sizes: BTreeMap<usize, usize> = BTreeMap::new();
                for &index in pad_items.values() {
                    *sizes.entry(components.0[index]).or_default() += 1;
                }
                let main = sizes
                    .iter()
                    .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                    .map(|(group, _)| *group);
                let rank = |group: Option<usize>| {
                    if group.is_some() && group == main {
                        2u8
                    } else {
                        1
                    }
                };
                let mut origins: Vec<(u8, Pt)> = targets
                    .iter()
                    .filter(|t| t.0 == net)
                    .map(|t| (0, t.1))
                    .collect();
                for &p in m.pads.get(&net).into_iter().flatten() {
                    let pad = m.pad(p).1;
                    let group = pad
                        .uuid
                        .as_deref()
                        .and_then(|id| pad_items.get(id))
                        .map(|&index| components.0[index]);
                    origins.push((rank(group), pad.at));
                }
                for &i in &fills {
                    if let Some(b) = scene.items[i].shape.bbox() {
                        origins.push((
                            rank(Some(components.0[i])),
                            ((b.0 + b.2) / 2., (b.1 + b.3) / 2.),
                        ));
                    }
                }
                drop(pad_items);
                origins.sort_by_key(|o| o.0);
                let mut seen = BTreeSet::new();
                let steps = (self.options.radius / self.options.step).ceil() as i32;
                'origins: for (_, origin) in origins {
                    for radius in 0..=steps {
                        // Only the ring's own cells are visited and paid for.
                        for (dx, dy) in ring(radius) {
                            {
                                scene.charge(1)?;
                                if !self.room() {
                                    break 'origins;
                                }
                                let at = (
                                    nm(origin.0 + f64::from(dx) * self.options.step),
                                    nm(origin.1 + f64::from(dy) * self.options.step),
                                );
                                if dist(origin, at) > self.options.radius
                                    || !seen.insert((
                                        (at.0 * 1e6).round() as i64,
                                        (at.1 * 1e6).round() as i64,
                                    ))
                                {
                                    continue;
                                }
                                let via = Shape::disc(at, scene.rules.diameter / 2.);
                                let inside = scene.containing_fills(&fills, &via)?;
                                let mask = inside
                                    .iter()
                                    .fold(0u64, |mask, &i| mask | scene.items[i].mask);
                                let groups: BTreeSet<_> =
                                    inside.iter().map(|&i| components.0[i]).collect();
                                let reference = targets.iter().any(|(n, p, span)| {
                                    n == &net && mask & span == *span && dist(*p, at) <= 3.
                                });
                                if mask.count_ones() < 2
                                    || !groups.iter().any(|g| components.1.contains(g))
                                    || (groups.len() < 2 && !reference)
                                {
                                    // Not a place a stitch could help: counted, not a candidate edit.
                                    self.considered += 1;
                                    continue;
                                }
                                let n = Copper {
                                    id: format!("proposal:{}:via", self.proposals.len()),
                                    net: Some(net.clone()),
                                    shape: via,
                                    mask: (1 << scene.layers.len()) - 1,
                                    kind: "via",
                                    locked: false,
                                    drill: Some(scene.rules.drill),
                                    layer: "F.Cu,B.Cu".into(),
                                    exact: true,
                                };
                                match self.candidate(scene, "stitching", Vec::new(), vec![n])? {
                                    Outcome::Accepted => {
                                        targets.retain(|(n, p, span)| {
                                            n != &net || mask & span != *span || dist(*p, at) > 3.
                                        });
                                        components = scene.components(&net)?;
                                        if !pending(&components, &targets) {
                                            break 'origins;
                                        }
                                    }
                                    Outcome::Rejected(_) => {}
                                    Outcome::Full => break 'origins,
                                }
                            }
                        }
                    }
                }
            }
            let (groups, anchored) = scene.components(&net)?;
            let floating: BTreeSet<_> = fills
                .iter()
                .map(|&i| groups[i])
                .filter(|g| !anchored.contains(g))
                .collect();
            if anchored.len() > 1 || !floating.is_empty() {
                let cause = if self.stopped {
                    "the search stopped before every position was tried"
                } else {
                    "no safe stitch position within the search bounds"
                };
                self.block("stitching",format!("ground {net}: {} pad-anchored groups and {} floating fill components remain; {cause}",anchored.len(),floating.len()),Vec::new());
            }
        }
        if !targets.is_empty() {
            let cause = if self.stopped {
                "the search stopped before every position was tried"
            } else {
                "no safe candidate within the search bounds"
            };
            self.block(
                "stitching",
                format!("{} reference changes remain; {cause}", targets.len()),
                Vec::new(),
            );
        }
        Ok(())
    }
}
