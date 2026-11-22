//! The placement search of one board: generate many candidates on threads, screen
//! them, route the references and the best of each family one call at a time, rank
//! everything under one judge, and record every candidate so that it can be rebuilt.
//! It writes only below its own folder, and never the input board.

// No product code calls the search: the bench (`place_bench_tests`, an ignored test) is its
// only consumer (design D1: no tool, no registry change, no apply path). So the library
// build sees it as unused, and so everything only it reaches. This one allowance covers the
// module; the other `place_*` modules carry item-level allowances only where something
// is used by the bench test alone or by nothing (each says which).
#![allow(dead_code)]

use crate::place_converge::{
    effort_stop, hoods_sha256, starts_stop, ConvergeParams, OuterRule, Trace,
};
use crate::place_generators::{
    board_model, generate, neutral_start, recipes, BoardModel, Candidate, Clearances,
    ConvergeRecipe, Family, GenerateFailure, Matrix, Params, Recipe, GENERATOR_VERSION,
};
use crate::place_held::{
    check_held, decoupling_pairs, groups, held_set, placements_of, same_board, sha256_hex, HeldSet,
    HoldRules, Intent,
};
use crate::place_judge::{
    movable_failure_keys, order as judged_order, pair_violations, placement_gates, rank, screen,
    screen_key, Judged, Key, PlacementGates, RouteResult, Screen, ScreenKey, SILK,
};
use crate::place_route::{export_candidate, route_candidate, unrouted_check, RouteInputs};
use crate::place_schematic::{SchematicBaseline, SchematicMeasure};
use crate::route_judge::Gates;
use crate::route_search::{median, BoxFuture, Effort, SplitMix64, HANG_CAP};
use crate::tools::pcb_components::{placements_applied_to, placements_moved_applied_to};
use crate::tools::placement::evaluate::evaluate;
use crate::tools::ToolContext;
use konnect_sexp::board::footprint_courtyards;
use konnect_sexp::board_geometry::Allowlist;
use konnect_sexp::parser::SexpNode;
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::time::Instant;

/// `score_placement`'s own fanout limit, used for every candidate's score.
const MAX_NET_REFERENCES: usize = 8;

/// Where a candidate came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Origin {
    /// O: the board as the owner placed it, routed `Plan::reference_calls` times;
    /// `call` counts from 1. Ids `0..reference_calls`.
    Owner { call: usize },
    /// A: the AI's placement from S0, read from the bench folder and admitted by
    /// `admit_ai`, routed the same number of times. The ids after O's.
    Ai { board: PathBuf, call: usize },
    Generated {
        recipe: Recipe,
        generator_version: u32,
    },
}

/// Wall times of one candidate's stages.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(crate) struct Timings {
    pub generate_ms: Option<f64>,
    pub write_ms: Option<f64>,
    pub evaluate_ms: Option<f64>,
    pub export_s: Option<f64>,
    pub route_s: Option<f64>,
}

/// One candidate, as `search.json` lists it.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Record {
    pub id: usize,
    pub origin: Origin,
    /// SHA-256 of the candidate's board text; `None` when no board was made.
    pub board_sha256: Option<String>,
    pub input_sha256: String,
    pub held_sha256: String,
    pub intent_sha256: String,
    /// `generated`, `generate_failed`, `held_moved`, `write_failed`, `screen_failed`,
    /// `duplicate` (its board equals an earlier candidate's, named in `reason`),
    /// `ai_refused`, `screened_out`, `routed`, `no_winner`, `export_refused`,
    /// `route_failed`, `not_routed_time_limit` or `not_routed_stopped`.
    pub outcome: &'static str,
    pub reason: Option<String>,
    pub converged: Option<bool>,
    pub screen: Option<Screen>,
    pub screen_key: Option<ScreenKey>,
    pub key: Option<Key>,
    /// 1 is the best, over the judged candidates of this board.
    pub rank: Option<usize>,
    pub timings: Timings,
    /// The routing call as recorded: its folder, engine (path, SHA-256, version),
    /// effort, lanes, outcome counts, elapsed and overrun, the winner's shape and seed,
    /// the host's 1-minute load at its start.
    pub route: Option<serde_json::Value>,
    /// P1 and P2 lines of the judged call (`placement_gates`). In `search.json` only:
    /// the lines can name parts.
    pub placement_gates: Option<PlacementGates>,
    /// The candidate's unrouted check, counts by DRC type (`search.baseline`).
    pub unrouted: Option<BTreeMap<String, usize>>,
    /// The routing winner's G1 to G4 lines. In `search.json` only: they can name nets.
    pub route_gates: Option<Gates>,
    /// How near this board keeps the parts that the schematic draws next to each other,
    /// against the input's (design D23). Reported only: it is not in the rank key. `None`
    /// when no board was made or the measure could not be taken (then
    /// `schematic_neighbours_reason` says why).
    pub schematic_neighbours: Option<SchematicMeasure>,
    pub schematic_neighbours_reason: Option<String>,
    /// Family C's trace (design C11). `None` for every other origin, and then not in
    /// `search.json` at all: with family C off the file is what it was before family C.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub converge: Option<Trace>,
    #[serde(skip)]
    pub board_text: Option<String>,
    #[serde(skip)]
    pub judged: Option<Judged>,
}

impl Record {
    /// A record of `model`'s board with nothing else known yet.
    fn bare(model: &BoardModel, id: usize, origin: Origin, outcome: &'static str) -> Self {
        Record {
            id,
            origin,
            board_sha256: None,
            input_sha256: model.held.board_sha256.clone(),
            held_sha256: model.held.held_sha256.clone(),
            intent_sha256: model.intent.sha256.clone(),
            outcome,
            reason: None,
            converged: None,
            screen: None,
            screen_key: None,
            key: None,
            rank: None,
            timings: Timings::default(),
            route: None,
            placement_gates: None,
            unrouted: None,
            route_gates: None,
            schematic_neighbours: None,
            schematic_neighbours_reason: None,
            converge: None,
            board_text: None,
            judged: None,
        }
    }

    /// Files the schematic-neighbour measure of this record's board text, when it has one:
    /// the measure, or the reason it could not be taken (no schematic, a board that does
    /// not parse). Reported only. `pub(crate)` for the bench's `converge_check`, which
    /// measures family C's runs the same way.
    pub(crate) fn file_schematic(&mut self, baseline: &Result<SchematicBaseline, String>) {
        let Some(text) = &self.board_text else {
            return;
        };
        let measured = baseline.as_ref().map_err(Clone::clone).and_then(|base| {
            let tree = konnect_sexp::parse_sexp(text)
                .map_err(|error| format!("the board text does not parse: {error}"))?;
            base.measure(&tree)
        });
        match measured {
            Ok(measure) => self.schematic_neighbours = Some(measure),
            Err(reason) => self.schematic_neighbours_reason = Some(reason),
        }
    }

    /// The family letter of the record's group: `O`, `A`, `F`, `S`, `P` or `C`.
    fn group(&self) -> &'static str {
        match &self.origin {
            Origin::Owner { .. } => "O",
            Origin::Ai { .. } => "A",
            Origin::Generated { recipe, .. } => family_letter(recipe.family),
        }
    }

    /// A short label for the table: no reference, no net, only the origin and numbers. A
    /// run of family C names its start (the start's family, seed and parameter-set index
    /// in `matrix`, `-` when the set is not one of the matrix's) and its effort level: the
    /// starts of F and S share seeds, and the levels of one start share its seed.
    fn label(&self, matrix: &Matrix) -> String {
        match &self.origin {
            Origin::Owner { call } => format!("O call {call}"),
            Origin::Ai { call, .. } => format!("A call {call}"),
            Origin::Generated { recipe, .. } => match &recipe.params {
                Params::Converge(run) => format!(
                    "C from {} seed {} set {} level {}",
                    family_letter(run.start.family),
                    run.start.seed,
                    start_set(&run.start, matrix).map_or("-".to_string(), |set| set.to_string()),
                    run.params.level
                ),
                _ => format!("{} seed {}", self.group(), recipe.seed),
            },
        }
    }

    /// What a rule of family C reads for this run (design C7): its trace's end cost when
    /// the record is `generated`, the board as written; infinity for a run that failed or
    /// whose board did not get through the screen.
    fn run_cost(&self) -> f64 {
        match (&self.converge, self.outcome) {
            (Some(trace), "generated") => trace.end.cost,
            _ => f64::INFINITY,
        }
    }
}

/// The letter of a family: `F`, `S`, `P` or `C`.
fn family_letter(family: Family) -> &'static str {
    match family {
        Family::Force => "F",
        Family::Schematic => "S",
        Family::Perturb => "P",
        Family::Converge => "C",
    }
}

/// The index of a start recipe's parameter set among the matrix's sets of its family
/// (F's or S's); `None` for another family or a set the matrix does not have.
fn start_set(start: &Recipe, matrix: &Matrix) -> Option<usize> {
    match &start.params {
        Params::Force(params) => matrix.force.iter().position(|p| p == params),
        Params::Schematic(params) => matrix.schematic.iter().position(|p| p == params),
        _ => None,
    }
}

/// One board's search.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    /// The board's bench name (`B4`, `B5`, `B6`, or a public fixture's).
    pub name: String,
    /// The board's bench folder; everything is written below it.
    pub folder: PathBuf,
    pub matrix: Matrix,
    /// Generation threads: the machine's cores.
    pub threads: usize,
    /// Routing calls for each reference, O and A (3): the spread of a reference's own
    /// calls is the routing noise a family's best must clear.
    pub reference_calls: usize,
    /// Per family, this many best by the screen key are routed ...
    pub best_per_family: usize,
    /// ... and this many drawn, with `draw_seed`, from the family's rest that passes P1.
    pub drawn_per_family: usize,
    pub draw_seed: u64,
    /// The board's whole time limit, generation included.
    pub time_limit: Duration,
    pub route: RouteInputs,
    /// A's board, when the bench has one.
    pub ai_board: Option<PathBuf>,
    /// The caller's held list (the bench's `held.json`): references held besides the
    /// rules. Added to the skeleton: `held_set` takes it and the plan is the only
    /// place the bench can put it.
    pub caller_held: BTreeSet<String>,
    /// The rules that hold a part besides KiCad's lock and the caller's list
    /// (`HoldRules::default()`, or `HoldRules::locks_only()` in the owner's flow).
    pub hold: HoldRules,
}

/// How one board's search ended.
#[derive(Debug, Clone)]
pub(crate) struct Outcome {
    pub records: Vec<Record>,
    pub elapsed: Duration,
    /// Why the routing phase stopped early, when it did, in full (it can hold a path or a
    /// tool's own output): `search.json` only.
    pub stopped_routing: Option<String>,
    /// The same as one fixed word, the only form `table.md` prints: `time_limit`,
    /// `input_check_failed` or `calls_failed_in_a_row`.
    pub stopped_code: Option<&'static str>,
    /// The clearances the generators kept (`BoardModel::clearances`).
    pub clearances: Clearances,
    /// P2's yardstick: the input board's unrouted check.
    pub input_unrouted: BTreeMap<String, usize>,
    /// Family C's outer loops (phase 1b); `None` when the matrix leaves the family off.
    pub converge: Option<OuterTrace>,
}

/// The routing phase's two jobs, so that the scheduler is tested with fakes on a
/// paused clock. The production one wraps `place_route` with an isolated context.
pub(crate) trait RouteCall: Send + Sync + 'static {
    /// Export and route one candidate in its own new `folder`.
    fn route(
        &self,
        id: usize,
        board_text: String,
        folder: PathBuf,
    ) -> BoxFuture<(RouteResult, Timings)>;
    /// The input board's unrouted check (`place_route::unrouted_check`).
    fn unrouted(
        &self,
        board_text: String,
        folder: PathBuf,
    ) -> BoxFuture<anyhow::Result<BTreeMap<String, usize>>>;
}

// ─── The production RouteCall ────────────────────────────────────────────────

/// The production [`RouteCall`]: `place_route`'s export and routing call under an
/// isolated context. Its folder handling is `place_route`'s: the candidate's folder
/// must not exist yet.
pub(crate) struct PlaceRouteCall {
    pub inputs: RouteInputs,
    pub ctx: Arc<ToolContext>,
}

/// The host's 1-minute load average, recorded at the start of each routing call (a
/// loaded host changes routing throughput, design D12). `None` where it cannot be read.
fn host_load_1m() -> Option<f64> {
    if let Ok(text) = std::fs::read_to_string("/proc/loadavg") {
        return text.split_whitespace().next()?.parse().ok();
    }
    let output = std::process::Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .ok()?;
    String::from_utf8(output.stdout)
        .ok()?
        .split_whitespace()
        .find_map(|word| word.parse::<f64>().ok())
}

impl RouteCall for PlaceRouteCall {
    fn route(
        &self,
        _id: usize,
        board_text: String,
        folder: PathBuf,
    ) -> BoxFuture<(RouteResult, Timings)> {
        let inputs = self.inputs.clone();
        let ctx = Arc::clone(&self.ctx);
        Box::pin(async move {
            let mut timings = Timings::default();
            let load = host_load_1m();
            let started = std::time::Instant::now();
            let exported = {
                let (inputs, folder) = (inputs.clone(), folder.clone());
                tokio::task::spawn_blocking(move || export_candidate(&inputs, &folder, &board_text))
                    .await
            };
            timings.export_s = Some(started.elapsed().as_secs_f64());
            let dsn = match exported {
                Ok(Ok(dsn)) => dsn,
                Ok(Err(error)) => {
                    return (
                        RouteResult::Failed {
                            stage: "export",
                            reason: format!("{error:#}"),
                        },
                        timings,
                    )
                }
                Err(error) => {
                    return (
                        RouteResult::Failed {
                            stage: "export",
                            reason: format!("the export job did not finish: {error}"),
                        },
                        timings,
                    )
                }
            };
            let routing_started = std::time::Instant::now();
            let mut result = route_candidate(ctx, &inputs, &dsn, &folder).await;
            timings.route_s = Some(routing_started.elapsed().as_secs_f64());
            if let RouteResult::Routed { call, .. } = &mut result {
                call["host_load_1m"] = json!(load);
            }
            (result, timings)
        })
    }

    fn unrouted(
        &self,
        board_text: String,
        folder: PathBuf,
    ) -> BoxFuture<anyhow::Result<BTreeMap<String, usize>>> {
        let inputs = self.inputs.clone();
        Box::pin(async move {
            // `unrouted_check` reads the input board itself; the text given must be that
            // board, or the yardstick would measure another one.
            let on_disk = std::fs::read_to_string(&inputs.board)?;
            anyhow::ensure!(
                on_disk == board_text,
                "the text given is not the input board '{}'",
                inputs.board.display()
            );
            unrouted_check(&inputs, &folder).await
        })
    }
}

// ─── Phase 1: generation ─────────────────────────────────────────────────────

/// A generator as the pool calls it. Production is [`generate`]; a test passes its own
/// (a panicking one, say) without changing what a recipe means.
pub(crate) type GenerateFn<'a> =
    &'a (dyn Fn(&BoardModel, &Recipe) -> Result<Candidate, GenerateFailure> + Sync);

/// The references of `set.held` that are real footprints of the board. A reference
/// shared only by pad-less footprints is held under `<reference>#<n>`; such a key names
/// no footprint, so it is never an edge-mount reference.
fn real_held_references(set: &HeldSet) -> Vec<String> {
    set.held
        .keys()
        .filter(|key| {
            !key.rsplit_once('#')
                .is_some_and(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .cloned()
        .collect()
}

fn milliseconds(started: std::time::Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

/// The mean distance of each group's members to the group's centre over the board
/// diagonal, averaged over the groups; `None` without a group that has 2 members on the
/// board, or without extent.
fn group_spread(model: &BoardModel, tree: &SexpNode) -> Option<f64> {
    let (x0, y0, x1, y1) = model.bounds;
    let diagonal = (x1 - x0).hypot(y1 - y0);
    if diagonal <= 0.0 {
        return None;
    }
    let scan = footprint_courtyards(tree);
    let centres: BTreeMap<&str, (f64, f64)> = scan
        .items
        .iter()
        .filter_map(|c| {
            let (a, b, c2, d) = c.bbox;
            Some((c.reference.as_deref()?, ((a + c2) / 2.0, (b + d) / 2.0)))
        })
        .collect();
    let mut spreads = Vec::new();
    for group in &model.intent.groups {
        let points: Vec<(f64, f64)> = group
            .members
            .iter()
            .filter_map(|member| centres.get(member.as_str()).copied())
            .collect();
        if points.len() < 2 {
            continue;
        }
        let n = points.len() as f64;
        let centre = (
            points.iter().map(|p| p.0).sum::<f64>() / n,
            points.iter().map(|p| p.1).sum::<f64>() / n,
        );
        let mean = points
            .iter()
            .map(|p| (p.0 - centre.0).hypot(p.1 - centre.1))
            .sum::<f64>()
            / n;
        spreads.push(mean / diagonal);
    }
    if spreads.is_empty() {
        None
    } else {
        Some(spreads.iter().sum::<f64>() / spreads.len() as f64)
    }
}

/// Held check, evaluation and screen of one board text, with every real held part as an
/// edge mount. Errs with the record outcome and its reason.
fn screen_text(model: &BoardModel, text: &str) -> Result<(Screen, f64), (&'static str, String)> {
    let tree = konnect_sexp::parse_sexp(text).map_err(|error| {
        (
            "write_failed",
            format!("the board text does not parse: {error}"),
        )
    })?;
    check_held(&model.held, &tree).map_err(|lines| ("held_moved", lines.join("; ")))?;
    let started = std::time::Instant::now();
    let edge_mount = Allowlist::new(real_held_references(&model.held));
    let evaluated = evaluate(&tree, &edge_mount, MAX_NET_REFERENCES)
        .map_err(|error| ("screen_failed", format!("the evaluation failed: {error:#}")))?;
    let violations = pair_violations(&tree, &model.intent.pairs);
    let screened = screen(
        &evaluated,
        &model.held,
        &model.input_failures,
        violations,
        group_spread(model, &tree),
    )
    .map_err(|error| ("screen_failed", format!("{error:#}")))?;
    Ok((screened, milliseconds(started)))
}

fn describe(failure: &GenerateFailure) -> String {
    match failure {
        GenerateFailure::NoRoom(parts) => {
            format!("no room for {} part(s): {}", parts.len(), parts.join(", "))
        }
        GenerateFailure::Invalid(why) => why.clone(),
        GenerateFailure::Panicked(message) => message.clone(),
    }
}

/// One generated candidate's record: generate, write, prove the held set, evaluate,
/// measure the pairs, screen.
fn generated_record(
    model: &BoardModel,
    id: usize,
    recipe: &Recipe,
    generator: GenerateFn<'_>,
) -> Record {
    let mut record = Record::bare(
        model,
        id,
        Origin::Generated {
            recipe: recipe.clone(),
            generator_version: GENERATOR_VERSION,
        },
        "generate_failed",
    );
    let started = std::time::Instant::now();
    let generated = generator(model, recipe);
    record.timings.generate_ms = Some(milliseconds(started));
    let mut candidate = match generated {
        Ok(candidate) => candidate,
        Err(failure) => {
            record.reason = Some(describe(&failure));
            return record;
        }
    };
    record.converged = candidate.converged;
    // Family C's run files its trace (design C11); every other family has none.
    record.converge = candidate.trace.take();
    let started = std::time::Instant::now();
    let written = placements_applied_to(&model.text, &candidate.placements);
    record.timings.write_ms = Some(milliseconds(started));
    let text = match written {
        Ok(text) => text,
        Err(error) => {
            record.outcome = "write_failed";
            record.reason = Some(format!("{error:?}"));
            return record;
        }
    };
    record.board_sha256 = Some(sha256_hex(text.as_bytes()));
    match screen_text(model, &text) {
        Ok((screened, evaluate_ms)) => {
            record.timings.evaluate_ms = Some(evaluate_ms);
            record.screen_key = Some(screen_key(&screened));
            record.screen = Some(screened);
            record.outcome = "generated";
        }
        Err((outcome, reason)) => {
            record.outcome = outcome;
            record.reason = Some(reason);
        }
    }
    record.board_text = Some(text);
    record
}

/// The text of a panic payload.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "the generator panicked without a message".to_string()
    }
}

/// `generated_record` with a panic in the job caught (`std::panic::catch_unwind`) and
/// recorded `generate_failed` with its message, so that no job's failure stops another.
fn caught_record(
    model: &BoardModel,
    id: usize,
    recipe: &Recipe,
    generator: GenerateFn<'_>,
) -> Record {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        generated_record(model, id, recipe, generator)
    })) {
        Ok(record) => record,
        Err(payload) => {
            let mut record = Record::bare(
                model,
                id,
                Origin::Generated {
                    recipe: recipe.clone(),
                    generator_version: GENERATOR_VERSION,
                },
                "generate_failed",
            );
            record.reason = Some(format!(
                "the generator panicked: {}",
                panic_message(payload.as_ref())
            ));
            record
        }
    }
}

/// Marks as `duplicate` each `generated` record whose board equals the board of an
/// earlier record of `records` (in slice order, which is id order) or of `earlier`
/// (`None` names the input board). The first holder of a board keeps it.
pub(crate) fn mark_duplicates(records: &mut [Record], earlier: &[(Option<usize>, String)]) {
    let mut seen: BTreeMap<String, Option<usize>> = BTreeMap::new();
    for (id, sha) in earlier {
        seen.entry(sha.clone()).or_insert(*id);
    }
    for record in records.iter_mut() {
        if record.outcome != "generated" {
            continue;
        }
        let Some(sha) = record.board_sha256.clone() else {
            continue;
        };
        match seen.get(&sha) {
            Some(holder) => {
                record.outcome = "duplicate";
                record.reason = Some(match holder {
                    Some(id) => format!("same board as candidate {id}"),
                    None => "same board as the input board".to_string(),
                });
            }
            None => {
                seen.insert(sha, Some(record.id));
            }
        }
    }
}

/// Phase 1, generation. `threads` scoped threads pull the next recipe index from one
/// atomic counter; each job generates (`place_generators::generate`), writes the board
/// text (`pcb_components::placements_applied_to`), proves the held set
/// (`place_held::check_held`), evaluates (`evaluate::evaluate`, every held part as
/// `edge_mount_references`), measures the intent's pairs (`place_judge::pair_violations`)
/// and screens. A panic in a job is caught (`std::panic::catch_unwind`) and recorded
/// `generate_failed` with its message; no job's failure stops another. Each result
/// lands in its recipe's slot, so the order in which jobs finish changes nothing. Ids
/// are `first_id + slot`. After the pool, a record whose board equals the input board
/// or a lower id's becomes `duplicate` (`search` adds A's board to those it compares
/// against).
pub(crate) fn generate_all(
    model: &BoardModel,
    recipes: &[Recipe],
    threads: usize,
    first_id: usize,
) -> Vec<Record> {
    generate_all_with(model, recipes, threads, first_id, &generate)
}

/// [`generate_all`] with the generator given: the production one is [`generate`].
pub(crate) fn generate_all_with(
    model: &BoardModel,
    recipes: &[Recipe],
    threads: usize,
    first_id: usize,
    generator: GenerateFn<'_>,
) -> Vec<Record> {
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<Record>>> = recipes.iter().map(|_| Mutex::new(None)).collect();
    let workers = threads.max(1).min(recipes.len());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let slot = next.fetch_add(1, Ordering::SeqCst);
                let Some(recipe) = recipes.get(slot) else {
                    break;
                };
                let record = caught_record(model, first_id + slot, recipe, generator);
                *slots[slot].lock().unwrap_or_else(PoisonError::into_inner) = Some(record);
            });
        }
    });
    let mut records: Vec<Record> = slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .unwrap_or_else(PoisonError::into_inner)
                .expect("every slot is filled once the scope has ended")
        })
        .collect();
    mark_duplicates(&mut records, &[(None, model.held.board_sha256.clone())]);
    records
}

// ─── Phase 1b: family C ──────────────────────────────────────────────────────

/// The outer loops' own record, `search.json`'s `converge`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct OuterTrace {
    pub params: ConvergeParams,
    pub rule: OuterRule,
    /// The ids of the starts taken, in start order, and the best cost after each;
    /// `None` while every run so far failed.
    pub starts: Vec<usize>,
    pub best_by_start: Vec<Option<f64>>,
    /// `rule`, `guard`, or `no_more_starts` (every start was taken).
    pub starts_stop: &'static str,
    /// The id of the start the effort was doubled on, and its cost at levels 0, 1, ...
    pub best_start: Option<usize>,
    pub cost_by_level: Vec<f64>,
    /// `rule` or `guard`; `None` when no run of a start succeeded.
    pub effort_stop: Option<&'static str>,
    /// Wall time of the whole phase.
    pub seconds: f64,
}

/// A record's start cost (design C8): the screen's airwire length plus
/// `params.crossing_mm` per crossing. The requirement terms are left out: every run
/// repairs them. `None` without both numbers.
fn start_cost(record: &Record, params: &ConvergeParams) -> Option<f64> {
    let screen = record.screen.as_ref()?;
    Some(screen.wirelength_mm? + params.crossing_mm * screen.crossings? as f64)
}

/// Family C's starts (design C8): the records of F and S with outcome `generated` that
/// pass P1, the lowest start cost first, a record without one last, ties to the lower id.
fn converge_pool<'r>(starts: &'r [Record], params: &ConvergeParams) -> Vec<&'r Record> {
    let mut pool: Vec<(&Record, Option<f64>)> = starts
        .iter()
        .filter(|r| {
            matches!(r.group(), "F" | "S")
                && r.outcome == "generated"
                && r.screen_key.is_some_and(|key| key[0] == 0)
        })
        .map(|r| (r, start_cost(r, params)))
        .collect();
    pool.sort_by(|(a, cost_a), (b, cost_b)| {
        let by_cost = match (cost_a, cost_b) {
            (Some(x), Some(y)) => x.total_cmp(y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        by_cost.then(a.id.cmp(&b.id))
    });
    pool.into_iter().map(|(record, _)| record).collect()
}

/// The recipe of a run of family C from `start` with `params`: the start recipe's seed.
fn converge_recipe(start: &Recipe, params: ConvergeParams) -> Recipe {
    Recipe {
        family: Family::Converge,
        seed: start.seed,
        params: Params::Converge(ConvergeRecipe {
            start: Box::new(start.clone()),
            params,
        }),
    }
}

/// The recipe a record of F or S was made from.
fn recipe_of(record: &Record) -> &Recipe {
    match &record.origin {
        Origin::Generated { recipe, .. } => recipe,
        _ => unreachable!("a start of family C is a generated record of F or S"),
    }
}

/// Phase 1b, family C. `starts` are the records of families F and S as phase 1 left
/// them. The pool is those with outcome `generated` that pass P1 (`screen_key[0] == 0`),
/// ordered by start cost (the screen's `wirelength_mm` + `params.crossing_mm` x
/// `crossings`; a record without both sorts last), ties to the lower id.
///
/// Starts. The pool's starts are handed in that order to `threads` scoped threads from
/// one atomic counter. Each job is `generated_record` with the recipe
/// `Recipe { family: Converge, seed: <the start recipe's seed>, params: Params::Converge(
/// ConvergeRecipe { start, params: *params }) }` (generate, write, held check, evaluate,
/// screen, as for every family) and files the candidate's trace on its record. After
/// each job the costs of the longest finished prefix are read by
/// `place_converge::starts_stop`; once it gives n, no new start is handed out, the
/// running jobs end, and exactly the records of the first n starts are kept.
///
/// Effort. The start whose record has the lowest cost (ties to the earlier start) is
/// run again with `params.level` 1, 2, ..., one level at a time, each a record like the
/// others; after each, `place_converge::effort_stop` reads the costs of levels 0 up, and
/// the loop ends when it gives a count. Every level's record is kept, the one that did
/// not pay too.
///
/// Ids are `first_id` upward: the starts in start order, then the levels. A panic is a
/// `generate_failed` record and costs infinity. The result is the same for any number of
/// threads. The starts run at level 0 whatever `params.level` holds, and the outer trace
/// records the parameters so.
pub(crate) fn converge_all(
    model: &BoardModel,
    starts: &[Record],
    params: &ConvergeParams,
    rule: &OuterRule,
    threads: usize,
    first_id: usize,
) -> (Vec<Record>, OuterTrace) {
    converge_all_with(model, starts, params, rule, threads, first_id, &generate)
}

/// The progress of the starts' pool: each start's record in its slot, and the count
/// `starts_stop` gave once it gave one.
struct StartsProgress {
    slots: Vec<Option<Record>>,
    stop: Option<usize>,
}

/// [`converge_all`] with the generator given: the production one is [`generate`].
pub(crate) fn converge_all_with(
    model: &BoardModel,
    starts: &[Record],
    params: &ConvergeParams,
    rule: &OuterRule,
    threads: usize,
    first_id: usize,
    generator: GenerateFn<'_>,
) -> (Vec<Record>, OuterTrace) {
    let started = std::time::Instant::now();
    // The starts run at level 0 whatever level the parameters hold, and the effort levels
    // count from 1 (design C7).
    let params = &ConvergeParams {
        level: 0,
        ..*params
    };
    let pool = converge_pool(starts, params);
    let recipes: Vec<Recipe> = pool
        .iter()
        .map(|start| converge_recipe(recipe_of(start), *params))
        .collect();
    // `starts_stop` stops at `max_starts` whatever the costs say, so no start past it is
    // ever kept: none is handed out (at least one, as the rule reads at least one).
    let handed = recipes.len().min(rule.max_starts.max(1));
    let progress = Mutex::new(StartsProgress {
        slots: vec![None; handed],
        stop: None,
    });
    let next = AtomicUsize::new(0);
    let workers = threads.max(1).min(handed);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let slot = next.fetch_add(1, Ordering::SeqCst);
                if slot >= handed {
                    break;
                }
                // Once the rule has stopped, no new start is handed out. Every start
                // before the stop was handed out already: the rule read their records.
                if progress
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .stop
                    .is_some()
                {
                    break;
                }
                let record = caught_record(model, first_id + slot, &recipes[slot], generator);
                let mut progress = progress.lock().unwrap_or_else(PoisonError::into_inner);
                progress.slots[slot] = Some(record);
                if progress.stop.is_none() {
                    let costs: Vec<f64> = progress
                        .slots
                        .iter()
                        .map_while(Option::as_ref)
                        .map(Record::run_cost)
                        .collect();
                    progress.stop = starts_stop(&costs, rule);
                }
            });
        }
    });
    let progress = progress
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    // Exactly the first n starts: a job that was running when the rule stopped ends, and
    // its record is not kept, so the records do not depend on the number of threads.
    let kept = progress.stop.unwrap_or(handed);
    let mut records: Vec<Record> = progress
        .slots
        .into_iter()
        .take(kept)
        .map(|slot| slot.expect("every start before the stop has its record"))
        .collect();
    let costs: Vec<f64> = records.iter().map(Record::run_cost).collect();
    let starts_word = match progress.stop {
        None => "no_more_starts",
        // The same function without its guard: did the rule itself stop at n?
        Some(n) => {
            let unguarded = OuterRule {
                max_starts: usize::MAX,
                ..*rule
            };
            if starts_stop(&costs, &unguarded) == Some(n) {
                "rule"
            } else {
                "guard"
            }
        }
    };
    let mut best_by_start = Vec::new();
    let mut best_so_far = f64::INFINITY;
    for &cost in &costs {
        best_so_far = best_so_far.min(cost);
        best_by_start.push(best_so_far.is_finite().then_some(best_so_far));
    }

    // Effort: the best start again, one level at a time.
    let best = (0..costs.len())
        .filter(|&k| costs[k].is_finite())
        .min_by(|&a, &b| costs[a].total_cmp(&costs[b]).then(a.cmp(&b)));
    let mut cost_by_level = Vec::new();
    let mut effort_word = None;
    if let Some(best) = best {
        cost_by_level.push(costs[best]);
        let start = recipe_of(pool[best]);
        let mut level = 0u32;
        loop {
            if let Some(count) = effort_stop(&cost_by_level, rule) {
                let unguarded = OuterRule {
                    max_level: u32::MAX,
                    ..*rule
                };
                effort_word = Some(if effort_stop(&cost_by_level, &unguarded) == Some(count) {
                    "rule"
                } else {
                    "guard"
                });
                break;
            }
            level += 1;
            let recipe = converge_recipe(start, ConvergeParams { level, ..*params });
            let record = caught_record(model, first_id + records.len(), &recipe, generator);
            cost_by_level.push(record.run_cost());
            records.push(record);
        }
    }
    let trace = OuterTrace {
        params: *params,
        rule: *rule,
        starts: pool.iter().take(kept).map(|start| start.id).collect(),
        best_by_start,
        starts_stop: starts_word,
        best_start: best.map(|best| pool[best].id),
        cost_by_level,
        effort_stop: effort_word,
        seconds: started.elapsed().as_secs_f64(),
    };
    (records, trace)
}

/// Checks family C's records for duplicates like any candidate's (design C1): a board
/// equal to the input board, to the board of an earlier record that holds one (A's and
/// the lower ids' are among `records`), or to an earlier run's of C is `duplicate`.
fn mark_converge_duplicates(model: &BoardModel, records: &[Record], converged: &mut [Record]) {
    let mut holders: Vec<(Option<usize>, String)> = vec![(None, model.held.board_sha256.clone())];
    holders.extend(
        records
            .iter()
            .filter(|r| r.outcome == "generated")
            .filter_map(|r| Some((Some(r.id), r.board_sha256.clone()?))),
    );
    mark_duplicates(converged, &holders);
}

// ─── Phase 2: selection ──────────────────────────────────────────────────────

/// Phase 2, selection, as candidate ids in routing order: O's calls, then A's; then,
/// per family, among the candidates that pass P1 (a candidate that fails it is never
/// routed), its `best` best by the screen key (ties to the lower id) and `drawn`
/// drawn uniformly with `SplitMix64(draw_seed)` from its other candidates that pass P1.
/// The families are interleaved round by round: each family's best, then each
/// family's drawn one, then the second best, then the rest, and round r starts at
/// family r mod 3 (round 0: F, S, P; round 1: S, P, F; round 2: P, F, S; ...), so a time
/// limit cuts every family alike over the rounds, and neither the last family nor the
/// screen's check is always the first thing cut. A candidate whose board equals an
/// earlier one's (`board_sha256`) was recorded `duplicate` in phase 1 and is never
/// selected; nor is a record that failed before the screen.
///
/// Family C (design C12) has its own order: its candidates that pass P1, by the screen's
/// requirement fields (score incomplete, decoupling violations, connector violations),
/// then by the cost the family converged on (`Trace::end.cost`), ties to the lower id; its
/// best and drawn ones as for the others, drawn after P's, so F's, S's and P's draws do
/// not change. In round r, C's call goes in at place 3 - (r mod 4) among the three
/// families' slots of that round (last in round 0, first in round 3), which leaves F's, S's
/// and P's order as it is without C, and is the rotation over F, S, P, C for the
/// bench's four rounds.
pub(crate) fn routing_order(
    records: &[Record],
    best: usize,
    drawn: usize,
    draw_seed: u64,
) -> Vec<usize> {
    let mut order = Vec::new();
    for reference in ["O", "A"] {
        order.extend(
            records
                .iter()
                .filter(|r| r.group() == reference && r.outcome == "generated")
                .map(|r| r.id),
        );
    }
    // Only a candidate that passes P1 is a candidate for a routing call: the best of a
    // pool that held the ones that fail it would be a call spent on an illegal placement.
    let passing = |family: &str| -> Vec<&Record> {
        records
            .iter()
            .filter(|r| {
                r.group() == family
                    && r.outcome == "generated"
                    && r.screen_key.is_some_and(|key| key[0] == 0)
            })
            .collect()
    };
    let mut rng = SplitMix64(draw_seed);
    let mut sequences: Vec<Vec<usize>> = Vec::new();
    for family in ["F", "S", "P"] {
        let mut pool = passing(family);
        pool.sort_by(|a, b| a.screen_key.cmp(&b.screen_key).then(a.id.cmp(&b.id)));
        sequences.push(family_sequence(&pool, best, drawn, &mut rng));
    }
    let mut converged = passing("C");
    let requirements = |r: &Record| r.screen_key.map(|key| [key[1], key[2], key[3]]);
    converged.sort_by(|a, b| {
        requirements(a)
            .cmp(&requirements(b))
            .then(a.run_cost().total_cmp(&b.run_cost()))
            .then(a.id.cmp(&b.id))
    });
    let c_sequence = family_sequence(&converged, best, drawn, &mut rng);
    let longest = sequences
        .iter()
        .chain([&c_sequence])
        .map(Vec::len)
        .max()
        .unwrap_or(0);
    for round in 0..longest {
        let mut slots: Vec<Option<usize>> = (0..sequences.len())
            .map(|step| {
                sequences[(round + step) % sequences.len()]
                    .get(round)
                    .copied()
            })
            .collect();
        let place = sequences.len() - round % (sequences.len() + 1);
        slots.insert(place, c_sequence.get(round).copied());
        order.extend(slots.into_iter().flatten());
    }
    order
}

/// One family's calls in order from its pool, sorted best first: its best, then its
/// `drawn` ones drawn uniformly with `rng` from the rest in id order (so the draw is a
/// function of the seed and the records alone), then its other best ones.
fn family_sequence(
    pool: &[&Record],
    best: usize,
    drawn: usize,
    rng: &mut SplitMix64,
) -> Vec<usize> {
    let picked: Vec<usize> = pool.iter().take(best).map(|r| r.id).collect();
    let mut rest: Vec<usize> = pool.iter().skip(best).map(|r| r.id).collect();
    rest.sort_unstable();
    let mut lucky = Vec::new();
    for _ in 0..drawn {
        if rest.is_empty() {
            break;
        }
        let at = rng.below(rest.len() as u64) as usize;
        lucky.push(rest.remove(at));
    }
    let mut sequence = Vec::new();
    sequence.extend(picked.first().copied());
    sequence.extend(lucky);
    sequence.extend(picked.iter().skip(1).copied());
    sequence
}

// ─── Phase 3: the launch rule ────────────────────────────────────────────────

/// What a routing call is expected to take before any has finished: the effort's
/// budget plus 60 s, a margin this design chooses for the judge's tail after the
/// budget (route design D7 lets a call that has no judged run yet wait past its
/// budget without a bound, so the median takes over as soon as one call ends). Fast
/// has no budget: the hang cap.
pub(crate) fn first_call_estimate(effort: Effort) -> Duration {
    effort.limits().budget.unwrap_or(HANG_CAP) + Duration::from_secs(60)
}

/// The launch rule: a routing call started at `elapsed` is expected to end by
/// `limit`: `elapsed + expected <= limit`, `expected` the median wall time of the calls
/// finished so far, or `first_call_estimate` before any. A call is never cut once
/// started: its own budget ends it (route design D7).
pub(crate) fn may_route(
    elapsed: Duration,
    limit: Duration,
    call_walls: &[Duration],
    effort: Effort,
) -> bool {
    let expected = median(call_walls).unwrap_or_else(|| first_call_estimate(effort));
    elapsed.saturating_add(expected) <= limit
}

/// The time the launch rule reads (design C13): the time since the search began less
/// the time phase 1b (family C) took. The board's limit was sized for the routing calls,
/// and a search that takes its own time must not cost a reference or a family a call.
pub(crate) fn launch_elapsed(since_start: Duration, phase_1b: Duration) -> Duration {
    since_start.saturating_sub(phase_1b)
}

/// Calls in a row that gave no routed candidate (`export_refused`, `route_failed` and
/// `no_winner`), with no routed candidate yet, that stop the routing phase. A routed
/// candidate resets the count, and after the first one the phase never stops this way.
pub(crate) const ROUTE_FAILURES_BEFORE_STOP: usize = 3;

// ─── The search ──────────────────────────────────────────────────────────────

/// The board text of S0 (the AI arm's start): `neutral_start` applied to the input.
pub(crate) fn s0_text(model: &BoardModel) -> anyhow::Result<String> {
    placements_applied_to(&model.text, &neutral_start(model))
        .map_err(|error| anyhow::anyhow!("S0 could not be written: {error:?}"))
}

/// A's board admitted as a reference: the anchors and rotations of the movable parts
/// read from `ai_text` (`place_held::placements_of`), applied to S0's text with
/// `pcb_components::placements_moved_applied_to` (a part whose rotation is S0's is moved,
/// as the move tool moves it, and only a turned part is set; a set would re-spell the
/// text and pad angles of a part the AI only moved), and the result must be the same
/// board as `ai_text` (`place_held::same_board`). So A differs from S0 only in where
/// movable parts are and how they turn: a flip, a changed held part, net, zone, outline
/// or rule is refused, and the refusal (`ai_refused`, with the first difference) is a
/// record. Returns the rebuilt text, which is what is judged.
pub(crate) fn admit_ai(
    s0_text: &str,
    ai_text: &str,
    movable: &BTreeSet<String>,
) -> Result<String, String> {
    let ai_tree = konnect_sexp::parse_sexp(ai_text)
        .map_err(|error| format!("the AI's board does not parse: {error}"))?;
    let placements = placements_of(&ai_tree, movable)
        .map_err(|error| format!("the AI's movable parts cannot be read: {error:#}"))?;
    let rebuilt = placements_moved_applied_to(s0_text, &placements)
        .map_err(|error| format!("the AI's placements do not apply to S0: {error:?}"))?;
    let rebuilt_tree = konnect_sexp::parse_sexp(&rebuilt)
        .map_err(|error| format!("the rebuilt board does not parse: {error}"))?;
    same_board(&rebuilt_tree, &ai_tree).map_err(|difference| {
        format!("the AI's board differs from S0 beyond placement: {difference}")
    })?;
    Ok(rebuilt)
}

/// The record of a reference call (`origin` O or A) from its board's screen.
fn reference_record(
    model: &BoardModel,
    id: usize,
    origin: Origin,
    text: &str,
    screened: &Result<(Screen, f64), (&'static str, String)>,
) -> Record {
    let mut record = Record::bare(model, id, origin, "generated");
    record.board_sha256 = Some(sha256_hex(text.as_bytes()));
    record.board_text = Some(text.to_string());
    match screened {
        Ok((screened, evaluate_ms)) => {
            record.timings.evaluate_ms = Some(*evaluate_ms);
            record.screen_key = Some(screen_key(screened));
            record.screen = Some(screened.clone());
        }
        Err((outcome, reason)) => {
            record.outcome = outcome;
            record.reason = Some(reason.clone());
        }
    }
    record
}

/// The silk entries of an unrouted check: its counts of the `SILK` kinds, summed (the
/// table's `silk` column and the rank key's silk field). Shared with the bench's
/// `converge_check`, so the two cannot count differently.
pub(crate) fn silk_entries(counts: &BTreeMap<String, usize>) -> usize {
    SILK.iter()
        .map(|kind| counts.get(*kind).copied().unwrap_or(0))
        .sum()
}

/// The judge's view of one called record, filed on the record: the placement gates
/// from the call's own unrouted baseline, the silk count, and the route result.
fn file_judgement(
    record: &mut Record,
    route: RouteResult,
    input_unrouted: &BTreeMap<String, usize>,
) {
    let Some(screened) = record.screen.clone() else {
        return;
    };
    let unrouted = match &route {
        RouteResult::Routed { unrouted, .. } => Some(unrouted),
        RouteResult::NoWinner { unrouted, .. } => unrouted.as_ref(),
        _ => None,
    };
    let placement = placement_gates(&screened, unrouted, input_unrouted);
    let silk = unrouted.map(silk_entries);
    record.placement_gates = Some(placement.clone());
    record.unrouted = unrouted.cloned();
    if let RouteResult::Routed { gates, .. } = &route {
        record.route_gates = Some(gates.clone());
    }
    record.judged = Some(Judged {
        id: record.id,
        screen: screened,
        placement,
        route,
        silk,
    });
}

/// Keys and ranks of every judged record (1 is the best).
fn assign_ranks(records: &mut [Record]) {
    let judged: Vec<&Judged> = records.iter().filter_map(|r| r.judged.as_ref()).collect();
    if judged.is_empty() {
        return;
    }
    let keys: BTreeMap<usize, Key> = judged.iter().map(|j| j.id).zip(rank(&judged)).collect();
    let positions: BTreeMap<usize, usize> = judged_order(&judged)
        .into_iter()
        .enumerate()
        .map(|(position, id)| (id, position + 1))
        .collect();
    for record in records.iter_mut() {
        if record.judged.is_some() {
            record.key = keys.get(&record.id).copied();
            record.rank = positions.get(&record.id).copied();
        }
    }
}

/// Files a finished call on its record: the outcome, the reason, the route summary and
/// the timings. Returns whether the call gave no routed candidate (a failure or no
/// winner: it counts toward `ROUTE_FAILURES_BEFORE_STOP`) and whether it routed one (its
/// wall time then tells the launch rule something; a call that judged no run waited
/// out its own limits and says nothing about the next call).
fn file_call(
    record: &mut Record,
    result: &RouteResult,
    timings: Timings,
    folder: &Path,
) -> (bool, bool) {
    record.timings.export_s = timings.export_s;
    record.timings.route_s = timings.route_s;
    let folder_text = folder.display().to_string();
    match result {
        RouteResult::Routed { call, .. } => {
            record.outcome = "routed";
            let mut summary = call.clone();
            if summary.is_object() {
                summary["folder"] = json!(folder_text);
            } else {
                summary = json!({ "folder": folder_text, "call": call });
            }
            record.route = Some(summary);
            (false, true)
        }
        RouteResult::NoWinner { reason, .. } => {
            record.outcome = "no_winner";
            record.reason = Some(reason.clone());
            record.route = Some(json!({ "folder": folder_text, "reason": reason }));
            (true, false)
        }
        RouteResult::Failed { stage, reason } => {
            record.outcome = if *stage == "export" {
                "export_refused"
            } else {
                "route_failed"
            };
            record.reason = Some(reason.clone());
            record.route = Some(json!({ "folder": folder_text, "stage": stage, "reason": reason }));
            (true, false)
        }
        RouteResult::NotRouted { why } => {
            record.outcome = match *why {
                "time_limit" => "not_routed_time_limit",
                "stopped" => "not_routed_stopped",
                _ => "screened_out",
            };
            (false, false)
        }
    }
}

/// The whole search of one board (flow steps 1 to 8): the held set and the intent,
/// the model, the input's unrouted check, the references and phase 1, phase 1b (family
/// C, `converge_all`) when the matrix has it, selection, then the routing phase one call
/// at a time in `routing_order` while `may_route` allows (on `launch_elapsed`),
/// each candidate's P gates and route result, the keys, and `search.json` and
/// `table.md` in `plan.folder`. Errs only before phase 1 (a held set that cannot be
/// proved, an input that does not parse); every later failure is a record.
pub(crate) async fn search(plan: &Plan, routing: Arc<dyn RouteCall>) -> anyhow::Result<Outcome> {
    let started = Instant::now();
    let (model, schematic) = search_model(plan)?;
    std::fs::create_dir_all(plan.folder.join("candidates"))?;

    // References: O, then A.
    let mut records: Vec<Record> = Vec::new();
    let input_screen = screen_text(&model, &model.text);
    for call in 1..=plan.reference_calls {
        let id = records.len();
        records.push(reference_record(
            &model,
            id,
            Origin::Owner { call },
            &model.text,
            &input_screen,
        ));
    }
    let mut earlier: Vec<(Option<usize>, String)> = Vec::new();
    if let Some(ai_path) = &plan.ai_board {
        let admitted = std::fs::read_to_string(ai_path)
            .map_err(|error| format!("the AI's board cannot be read: {error}"))
            .and_then(|ai_text| {
                let s0 = s0_text(&model).map_err(|error| format!("{error:#}"))?;
                admit_ai(&s0, &ai_text, &model.held.movable)
            });
        match admitted {
            Err(reason) => {
                let id = records.len();
                let mut record = Record::bare(
                    &model,
                    id,
                    Origin::Ai {
                        board: ai_path.clone(),
                        call: 1,
                    },
                    "ai_refused",
                );
                record.reason = Some(reason);
                records.push(record);
            }
            Ok(ai_text) => {
                let screened = screen_text(&model, &ai_text);
                for call in 1..=plan.reference_calls {
                    let id = records.len();
                    records.push(reference_record(
                        &model,
                        id,
                        Origin::Ai {
                            board: ai_path.clone(),
                            call,
                        },
                        &ai_text,
                        &screened,
                    ));
                }
                if plan.reference_calls > 0 {
                    earlier.push((
                        Some(records.len() - plan.reference_calls),
                        sha256_hex(ai_text.as_bytes()),
                    ));
                }
            }
        }
    }

    // Phase 1.
    let first_id = records.len();
    let mut generated = generate_all(&model, &recipes(&plan.matrix), plan.threads, first_id);
    mark_duplicates(&mut generated, &earlier);
    records.extend(generated);

    // Phase 1b, family C (design C1): after every record of F, S and P exists, so their
    // ids and their draws in the selection do not change. Its time is its own (C13).
    let mut outer: Option<OuterTrace> = None;
    let mut phase_1b = Duration::ZERO;
    if let Some((params, rule)) = &plan.matrix.converge {
        let phase_started = Instant::now();
        let (mut converged, trace) =
            converge_all(&model, &records, params, rule, plan.threads, records.len());
        mark_converge_duplicates(&model, &records, &mut converged);
        records.extend(converged);
        phase_1b = phase_started.elapsed();
        outer = Some(trace);
    }

    for record in &mut records {
        record.file_schematic(&schematic);
    }

    // Phase 2.
    let order = routing_order(
        &records,
        plan.best_per_family,
        plan.drawn_per_family,
        plan.draw_seed,
    );
    let chosen: BTreeSet<usize> = order.iter().copied().collect();
    for record in &mut records {
        if record.outcome == "generated" && !chosen.contains(&record.id) {
            record.outcome = "screened_out";
        }
        // A board nobody will route is never read again (`rerun` rebuilds a generated
        // one from its recipe): keeping the text of every candidate would hold a copy
        // of the board per recipe, 288 of them, for the rest of the search.
        if !chosen.contains(&record.id) {
            record.board_text = None;
        }
    }

    // Phase 3.
    let mut stopped_routing: Option<String> = None;
    let mut stopped_code: Option<&'static str> = None;
    let mut input_unrouted = BTreeMap::new();
    match routing
        .unrouted(model.text.clone(), plan.folder.join("input-check"))
        .await
    {
        Ok(counts) => input_unrouted = counts,
        Err(error) => {
            stopped_routing = Some(format!(
                "the input board's unrouted check failed, so no candidate's P2 can be judged: {error:#}"
            ));
            stopped_code = Some("input_check_failed");
        }
    }
    let mut walls: Vec<Duration> = Vec::new();
    let mut misses_in_a_row = 0usize;
    let mut any_routed = false;
    let mut out_of_time = false;
    let mut halted = stopped_routing.is_some();
    for id in order {
        let Some(at) = records.iter().position(|r| r.id == id) else {
            continue;
        };
        if out_of_time {
            records[at].outcome = "not_routed_time_limit";
            continue;
        }
        if halted {
            records[at].outcome = "not_routed_stopped";
            continue;
        }
        if !may_route(
            launch_elapsed(started.elapsed(), phase_1b),
            plan.time_limit,
            &walls,
            plan.route.effort,
        ) {
            out_of_time = true;
            stopped_routing =
                Some("the launch rule: the next call would not end by the time limit".to_string());
            stopped_code = Some("time_limit");
            records[at].outcome = "not_routed_time_limit";
            continue;
        }
        let Some(board_text) = records[at].board_text.clone() else {
            records[at].outcome = "not_routed_stopped";
            continue;
        };
        let folder = plan.folder.join("candidates").join(format!("c{id:04}"));
        let call_started = Instant::now();
        let (result, timings) = routing.route(id, board_text, folder.clone()).await;
        let wall = call_started.elapsed();
        let (missed, routed) = file_call(&mut records[at], &result, timings, &folder);
        if routed {
            // Only a call that routed says how long the next one takes.
            walls.push(wall);
            misses_in_a_row = 0;
            any_routed = true;
        } else if missed {
            misses_in_a_row += 1;
        }
        file_judgement(&mut records[at], result, &input_unrouted);
        if misses_in_a_row >= ROUTE_FAILURES_BEFORE_STOP && !any_routed {
            halted = true;
            stopped_routing = Some(format!(
                "{ROUTE_FAILURES_BEFORE_STOP} routing calls in a row ended in a failure or with no winner, and none had routed a candidate"
            ));
            stopped_code = Some("calls_failed_in_a_row");
        }
    }
    assign_ranks(&mut records);

    let outcome = Outcome {
        records,
        elapsed: started.elapsed(),
        stopped_routing,
        stopped_code,
        clearances: model.clearances,
        input_unrouted,
        converge: outer,
    };
    std::fs::write(
        plan.folder.join("search.json"),
        serde_json::to_string_pretty(&search_json(plan, &outcome))?,
    )?;
    std::fs::write(plan.folder.join("table.md"), table(plan, &outcome))?;
    Ok(outcome)
}

/// The board model as `search` builds it, and the schematic baseline (design D23): the
/// held set and the intent from the input board's own score, its failures as P1's
/// yardstick, the clearances from the routing inputs, and the schematic neighbourhoods
/// that family C reads (`None` when the schematic cannot be read; the baseline then
/// carries the reason). Writes nothing. Errs as `search` does before phase 1: a held set
/// that cannot be proved, an input that does not parse.
pub(crate) fn search_model(
    plan: &Plan,
) -> anyhow::Result<(BoardModel, Result<SchematicBaseline, String>)> {
    let text = std::fs::read_to_string(&plan.route.board).map_err(|error| {
        anyhow::anyhow!(
            "the input board '{}' cannot be read: {error}",
            plan.route.board.display()
        )
    })?;
    let held = held_set(&text, &plan.caller_held, &plan.hold).map_err(|refusal| {
        anyhow::anyhow!("{} Next step: {}", refusal.reason, refusal.next_step)
    })?;
    let tree = konnect_sexp::parse_sexp(&text)?;
    let edge_mount = Allowlist::new(real_held_references(&held));
    let evaluated = evaluate(&tree, &edge_mount, MAX_NET_REFERENCES)?;
    let pairs = decoupling_pairs(&evaluated)?;
    let intent = Intent::new(groups(&tree, &held, &[]), pairs);
    let input_failures = movable_failure_keys(&evaluated, &held)?;
    // The schematic neighbourhoods, fixed once from the input board's project (D23). A
    // missing or unreadable schematic is a reason on every record, not an error.
    let schematic = SchematicBaseline::read(&plan.route.board, &tree);
    let mut model = board_model(&text, held, intent)?;
    model.input_failures = input_failures;
    model.clearances = Clearances::from_inputs(&plan.route.rules, &plan.route.options);
    // Family C reads the neighbourhoods the measure reads (design C11), and none when
    // the schematic cannot be read.
    model.hoods = schematic
        .as_ref()
        .ok()
        .map(|baseline| baseline.hoods.clone());
    Ok((model, schematic))
}

/// Rebuilds a generated record's board text and proves it is the recorded one. Errs
/// when the record is not a generated one, when its `generator_version`, input,
/// held or intent SHA-256 differ from this build's model, or when the rebuilt text's
/// SHA-256 is not `board_sha256`.
pub(crate) fn rerun(model: &BoardModel, record: &Record) -> anyhow::Result<String> {
    let Origin::Generated {
        recipe,
        generator_version,
    } = &record.origin
    else {
        anyhow::bail!("candidate {} is not a generated one", record.id);
    };
    anyhow::ensure!(
        *generator_version == GENERATOR_VERSION,
        "candidate {} was made by generator version {generator_version}, this build is {GENERATOR_VERSION}",
        record.id
    );
    // Family C (design C11): the start is generated again from its own recipe inside
    // `generate`, so the start recipe must be one of F or S; and the run read the
    // schematic neighbourhoods its trace names, so a model with other ones would make
    // another board.
    if recipe.family == Family::Converge {
        let Params::Converge(run) = &recipe.params else {
            anyhow::bail!(
                "candidate {} is of family C and its recipe is not a run of family C",
                record.id
            );
        };
        anyhow::ensure!(
            run.start.family != Family::Converge,
            "candidate {} is of family C and starts from a recipe of family C, which is no start",
            record.id
        );
        let trace = record.converge.as_ref().ok_or_else(|| {
            anyhow::anyhow!("candidate {} is of family C and has no trace", record.id)
        })?;
        anyhow::ensure!(
            model.hoods.as_ref().map(hoods_sha256) == trace.hoods_sha256,
            "candidate {} was made with other schematic neighbourhoods",
            record.id
        );
    }
    anyhow::ensure!(
        record.input_sha256 == model.held.board_sha256,
        "candidate {} was made from another input board",
        record.id
    );
    anyhow::ensure!(
        record.held_sha256 == model.held.held_sha256,
        "candidate {} was made with another held list",
        record.id
    );
    anyhow::ensure!(
        record.intent_sha256 == model.intent.sha256,
        "candidate {} was made with another intent",
        record.id
    );
    let recorded = record
        .board_sha256
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("candidate {} has no board to compare with", record.id))?;
    let candidate = generate(model, recipe).map_err(|failure| {
        anyhow::anyhow!("the recipe no longer generates: {}", describe(&failure))
    })?;
    let text = placements_applied_to(&model.text, &candidate.placements)
        .map_err(|error| anyhow::anyhow!("the placements no longer apply: {error:?}"))?;
    anyhow::ensure!(
        sha256_hex(text.as_bytes()) == recorded,
        "the rebuilt board of candidate {} is not the recorded one",
        record.id
    );
    Ok(text)
}

// ─── The records on disk and the table ───────────────────────────────────────

/// `search.json`: the plan (matrix with family C's parameters and outer rule, limits,
/// effort, fastroute, threads, host cores), every record (a run of C with its trace), the
/// input's unrouted check, family C's outer trace, and how the search ended. With family
/// C off no `converge` key is written, in the plan, a record or at the top.
pub(crate) fn search_json(plan: &Plan, outcome: &Outcome) -> serde_json::Value {
    let matrix = &plan.matrix;
    let fastroute_sha256 = std::fs::read(&plan.route.fastroute)
        .ok()
        .map(|bytes| sha256_hex(&bytes));
    let mut file = json!({
        "plan": {
            "name": plan.name,
            "generator_version": GENERATOR_VERSION,
            "matrix": {
                "force_seeds": matrix.force_seeds,
                "force": matrix.force,
                "schematic_seeds": matrix.schematic_seeds,
                "schematic": matrix.schematic,
                "perturb_seeds": matrix.perturb_seeds,
                "perturb": matrix.perturb,
            },
            "recipes": recipes(matrix).len(),
            "hold_rules": plan.hold,
            "threads": plan.threads,
            "host_cores": std::thread::available_parallelism().map(|n| n.get()).ok(),
            "reference_calls": plan.reference_calls,
            "best_per_family": plan.best_per_family,
            "drawn_per_family": plan.drawn_per_family,
            "draw_seed": plan.draw_seed,
            "time_limit_s": plan.time_limit.as_secs_f64(),
            "clearances": outcome.clearances,
            "effort": plan.route.effort.name(),
            "fastroute": plan.route.fastroute.display().to_string(),
            "fastroute_sha256": fastroute_sha256,
            "ai_board": plan.ai_board.as_ref().map(|path| path.display().to_string()),
        },
        "records": outcome.records,
        "input_unrouted": outcome.input_unrouted,
        "elapsed_s": outcome.elapsed.as_secs_f64(),
        "stopped_routing": outcome.stopped_routing,
        "stopped_code": outcome.stopped_code,
    });
    // Family C's keys only when the family is on: with it off the file is what it was
    // before family C.
    if let Some((params, rule)) = matrix.converge {
        file["plan"]["matrix"]["converge"] = json!({ "params": params, "rule": rule });
    }
    if let Some(outer) = &outcome.converge {
        file["converge"] = json!(outer);
    }
    file
}

/// A family's verdict against one reference, by rank (1 is best).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Verdict {
    /// The family's best ranks above every one of the reference's calls.
    Beats,
    /// It ranks between the reference's best and worst call: inside its routing noise.
    Matches,
    /// It ranks below every one of the reference's calls.
    Below,
    /// The family or the reference has no routed candidate.
    Unknown,
}

impl Verdict {
    fn label(self) -> &'static str {
        match self {
            Verdict::Beats => "beats",
            Verdict::Matches => "matches",
            Verdict::Below => "below",
            Verdict::Unknown => "unknown",
        }
    }
}

/// `Verdict` from the family's best rank and the ranks of the reference's calls.
pub(crate) fn verdict(family_best: Option<usize>, reference_calls: &[usize]) -> Verdict {
    let (Some(best), Some(&top), Some(&bottom)) = (
        family_best,
        reference_calls.iter().min(),
        reference_calls.iter().max(),
    ) else {
        return Verdict::Unknown;
    };
    if best < top {
        Verdict::Beats
    } else if best <= bottom {
        Verdict::Matches
    } else {
        Verdict::Below
    }
}

fn pass_word(passed: bool) -> &'static str {
    if passed {
        "pass"
    } else {
        "fail"
    }
}

fn number(value: Option<f64>) -> String {
    value.map_or("-".to_string(), |v| format!("{v:.1}"))
}

/// `table.md`'s line on family C's outer loops, counts only: the starts taken and how
/// the rule for starts stopped, the effort levels run on the best start and how the rule
/// for effort stopped, the runs a guard ended, and phase 1b's wall time (C13: not counted
/// against the board's limit).
fn outer_loops_line(outer: &OuterTrace, records: &[Record]) -> String {
    let runs: Vec<&Record> = records.iter().filter(|r| r.group() == "C").collect();
    let guarded = runs
        .iter()
        .filter(|r| r.converge.as_ref().is_some_and(|trace| !trace.converged()))
        .count();
    let levels = match outer.cost_by_level.len() {
        0 => "no effort level (no start's run succeeded)".to_string(),
        n => format!(
            "effort levels 0 to {} on the best start, the rule for effort stopped by `{}`",
            n - 1,
            outer.effort_stop.unwrap_or("-")
        ),
    };
    format!(
        "Family C's outer loops: {} start(s) taken, the rule for starts stopped by `{}`; {levels}; \
         {guarded} of {} run(s) ended by a guard; {:.0} s, not counted against the time limit.\n",
        outer.starts.len(),
        outer.starts_stop,
        runs.len(),
        outer.seconds
    )
}

/// `table.md` of one board, counts only, no net or part names: one row per routed
/// candidate in rank order (rank, origin, family and seed, P1, P2, G1, unconnected,
/// G2, G3, G4, decoupling, connector, acute, signal and power mm, vias off the pours,
/// copper quality, silk, placement score, and the two reported-only schematic-neighbour
/// columns of D23), then one row per family and reference (best
/// rank, median rank of its routed candidates, how many pass every gate, how many were
/// generated, failed, duplicates and screened out), family C's one line of counts on its
/// outer loops when the matrix has it, then each family's verdict against O and against
/// A (`verdict`).
pub(crate) fn table(plan: &Plan, outcome: &Outcome) -> String {
    // Only the families the matrix makes recipes for: a family the bench left out
    // (`families` in `bench.json`) has no row and no verdict.
    let all_recipes = recipes(&plan.matrix);
    let mut families: Vec<&str> = [
        (Family::Force, "F"),
        (Family::Schematic, "S"),
        (Family::Perturb, "P"),
    ]
    .into_iter()
    .filter(|(family, _)| all_recipes.iter().any(|r| r.family == *family))
    .map(|(_, letter)| letter)
    .collect();
    // Family C has no fixed recipes: it is in when the matrix has its parameters.
    if plan.matrix.converge.is_some() {
        families.push("C");
    }
    let mut text = format!("# Placement search: {}\n\n", plan.name);
    text.push_str(&format!(
        "Effort {}, {} recipes (families {}), {} candidates, {:.0} s elapsed.",
        plan.route.effort.name(),
        all_recipes.len(),
        families.join(", "),
        outcome.records.len(),
        outcome.elapsed.as_secs_f64()
    ));
    // A fixed word only: the full reason can hold a path or a tool's own output, and
    // lives in `search.json`.
    if let Some(code) = outcome.stopped_code {
        text.push_str(&format!(" Routing stopped early: {code}."));
    }
    text.push_str("\n\n## Routed candidates, best first\n\n");
    text.push_str(
        "The last two columns (`sch. nbr mm`: the mean distance of each part to its nearest \
         schematic neighbours, and `pulled away`: the parts that moved away from them, against \
         the input) are reported only; they do not rank.\n\n",
    );
    text.push_str("| rank | origin | P1 | P2 | G1 | unconnected | G2 | G3 | G4 | decoupling | connector | acute | signal mm | power mm | vias off pours | copper quality | silk | score | sch. nbr mm | pulled away |\n");
    text.push_str(
        "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n",
    );
    let mut routed: Vec<&Record> = outcome
        .records
        .iter()
        .filter(|r| r.outcome == "routed" && r.rank.is_some())
        .collect();
    routed.sort_by_key(|r| (r.rank, r.id));
    for record in &routed {
        let Some(judged) = &record.judged else {
            continue;
        };
        let RouteResult::Routed {
            gates, measures, ..
        } = &judged.route
        else {
            continue;
        };
        text.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {:.1} | {:.1} | {} | {} | {} | {} | {} | {} |\n",
            record.rank.unwrap_or_default(),
            record.label(&plan.matrix),
            pass_word(judged.placement.p1.is_empty()),
            pass_word(judged.placement.p2.is_empty()),
            pass_word(gates.g1.is_empty()),
            measures.unconnected,
            pass_word(gates.g2.is_empty()),
            pass_word(gates.g3.is_empty()),
            pass_word(gates.g4.is_empty()),
            judged.screen.decoupling_violations,
            judged.screen.connector_violations,
            number(measures.acute),
            measures.signal_mm,
            measures.power_mm,
            measures.vias_off_pour,
            number(measures.copper_quality),
            judged.silk.map_or("-".to_string(), |silk| silk.to_string()),
            judged.screen.score,
            number(record.schematic_neighbours.as_ref().map(|m| m.mean_mm)),
            record
                .schematic_neighbours
                .as_ref()
                .map_or("-".to_string(), |m| m.parts_pulled_away.to_string()),
        ));
    }

    text.push_str("\n## Per family and reference\n\n");
    text.push_str("| group | best rank | median rank | pass every gate | made | failed | duplicates | screened out | routed |\n");
    text.push_str("|---|---|---|---|---|---|---|---|---|\n");
    let groups: Vec<&str> = ["O", "A"]
        .into_iter()
        .chain(families.iter().copied())
        .collect();
    let mut ranks_of: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    // The ranks of every judged call of a reference, a routed one or one with no winner:
    // both carry a key, and a no-winner call is the reference's worst rank, not a call
    // that did not happen.
    let mut reference_ranks: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for group in groups {
        let members: Vec<&Record> = outcome
            .records
            .iter()
            .filter(|r| r.group() == group)
            .collect();
        let mut ranks: Vec<usize> = members
            .iter()
            .filter(|r| r.outcome == "routed")
            .filter_map(|r| r.rank)
            .collect();
        ranks.sort_unstable();
        let mut judged_calls: Vec<usize> = members
            .iter()
            .filter(|r| r.outcome == "routed" || r.outcome == "no_winner")
            .filter_map(|r| r.rank)
            .collect();
        judged_calls.sort_unstable();
        reference_ranks.insert(group, judged_calls);
        let all_pass = members
            .iter()
            .filter(|r| r.outcome == "routed")
            .filter(|r| {
                r.judged.as_ref().is_some_and(|j| {
                    j.placement.pass()
                        && matches!(&j.route, RouteResult::Routed { gates, .. } if gates.all_pass())
                })
            })
            .count();
        let count = |outcomes: &[&str]| {
            members
                .iter()
                .filter(|r| outcomes.contains(&r.outcome))
                .count()
        };
        text.push_str(&format!(
            "| {group} | {} | {} | {all_pass} | {} | {} | {} | {} | {} |\n",
            ranks.first().map_or("-".to_string(), |r| r.to_string()),
            ranks
                .get(ranks.len().saturating_sub(1) / 2)
                .filter(|_| !ranks.is_empty())
                .map_or("-".to_string(), |r| r.to_string()),
            members.iter().filter(|r| r.screen.is_some()).count(),
            count(&[
                "generate_failed",
                "held_moved",
                "write_failed",
                "screen_failed",
                "ai_refused"
            ]),
            count(&["duplicate"]),
            count(&["screened_out"]),
            ranks.len(),
        ));
        ranks_of.insert(group, ranks);
    }
    if let Some(outer) = &outcome.converge {
        text.push('\n');
        text.push_str(&outer_loops_line(outer, &outcome.records));
    }

    text.push_str("\n## Verdicts\n\n");
    text.push_str(
        "A family's best is its best routed candidate. A reference's calls are all its judged \
         calls, routed or with no winner; a no-winner call ranks as the reference's worst.\n\n",
    );
    text.push_str("| family | against O | against A |\n|---|---|---|\n");
    for &family in &families {
        let best = ranks_of
            .get(family)
            .and_then(|ranks| ranks.first().copied());
        let against = |reference: &str| {
            verdict(
                best,
                reference_ranks
                    .get(reference)
                    .map_or(&[][..], Vec::as_slice),
            )
            .label()
        };
        text.push_str(&format!(
            "| {family} | {} | {} |\n",
            against("O"),
            against("A")
        ));
    }
    text
}

#[cfg(test)]
#[path = "place_search_tests.rs"]
mod tests;
