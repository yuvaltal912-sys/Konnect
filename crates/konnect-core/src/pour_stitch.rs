//! Row ZR, stage ZR6: a ground via at every reference change (checklist 3.2), on every
//! judged run before the rank, and in the delivered session
//! (docs/ZONE_ROUTING_DESIGN.md, "ZR6").
//!
//! Nothing here writes to the user's board or project: every board is a scratch copy under
//! the search folder's `judge` folder, and the delivered session is a new file beside the
//! winner's.

use crate::pour_prune::{self, Checked, CheckedRest};
use crate::pour_rank::{self, RankMode};
use crate::route_judge::{self, Baseline, Gates, JudgeContext, Judged, ToolCaller};
use crate::route_search::{Routed, SearchOutcome};
use crate::scratch_board::{Planned, PlannedVia};
use crate::tools::cli::DrcReport;
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Reference rounds a judged run gets after Z9's rounds (S4); S5's one retry is inside it.
pub(crate) const REFERENCE_ROUNDS: usize = 1;
/// S5 `reference`: the tolerance of 3.1's length off the reference, mm.
pub(crate) const LENGTH_OFF_REFERENCE_TOLERANCE_MM: f64 = 0.1;
/// S3: the slack of a kept via's distance to its target, mm (1 µm: the audit writes a
/// finding's point rounded to 1 µm).
pub(crate) const KEEP_SLACK_MM: f64 = 0.001;
/// S5's retry: a written via is named by a DRC item when the item's position lies within
/// the via's radius plus this, mm. A via's own violation names its centre; the other item
/// of a clearance violation lies within one clearance of its edge.
pub(crate) const RETRY_REACH_MM: f64 = 0.5;
/// S5's retry: a DRC item's position that is a vertex of a saved fill piece, within this,
/// names that piece (KiCad places an `isolated_copper` item at a vertex of the island), mm.
pub(crate) const VERTEX_MATCH_MM: f64 = 0.001;
/// S2: the findings the round's second audit lists (the audit's maximum).
pub(crate) const FULL_FINDINGS: usize = 500;
/// The audit's default `stitch_via_max_mm`, for a 3.2 finding that does not name its limit.
const DEFAULT_LIMIT_MM: f64 = 3.0;
/// The two folders of the round in `<run dir>`: the first board and the retry's.
const FIRST_FOLDER: &str = "s1";
const RETRY_FOLDER: &str = "s1-retry";

/// Whether the judge runs the reference round and the delivery writes its vias (S1, S13):
/// on for `route_specctra_dsn`, off for the placement tools' routing calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferenceStitch {
    On,
    Off,
}

/// One unstitched layer change the round may serve (S2): a 3.2 finding whose reference nets
/// are one net the audit lists as ground, over two or more reference layers.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Target {
    /// The reference net, the net the new via belongs to.
    pub net: String,
    /// The signal via's point, mm (the finding's `at`).
    pub at: (f64, f64),
    /// The finding's `measured.reference_layers`.
    pub reference_layers: Vec<String>,
    /// The finding's `threshold.stitch_via_max_mm`.
    pub max_mm: f64,
    /// The finding's `subject` (signal net and via), for the record.
    pub subject: String,
}

/// A 3.2 finding the round leaves alone, and why (S2): `two_reference_nets` (a capacitor
/// between the planes is the fix, not a via), `not_ground` (a power plane, or no reference
/// net), or `no_position` (a finding without a point; the audit always gives one).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Skipped {
    pub subject: String,
    pub reason: &'static str,
}

/// A target the round did not serve, and why (S6): the planner's `blocked` reason of the
/// `stitching` kind, or `refused` when the guard sent the round back.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Remaining {
    pub subject: String,
    pub net: String,
    pub reason: String,
}

/// What [`targets`] read.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Targets {
    pub targets: Vec<Target>,
    pub skipped: Vec<Skipped>,
    pub complete: bool,
}

/// The audit's count of unstitched layer changes (`metrics["3.2.unstitched_layer_changes"]`).
pub(crate) fn unstitched(audit: &Value) -> Option<f64> {
    audit["metrics"]["3.2.unstitched_layer_changes"].as_f64()
}

/// S2: the targets and the skipped findings of the audit's 3.2 check (`findings[]` whose
/// `item` is `3.2`), in the audit's order. `ground` is `net_roles.ground.nets`
/// (`pour_prune::audit_items`). A finding is a target only when its `reference_nets` is
/// one name in `ground`. `complete` is false when targets plus skipped are fewer than
/// `metrics["3.2.unstitched_layer_changes"]` (the audit lists at most `max_findings`):
/// the round then asks [`full_findings`] for the whole list before it plans.
pub(crate) fn targets(audit: &Value, ground: &BTreeSet<String>) -> Targets {
    let mut out = Targets {
        targets: Vec::new(),
        skipped: Vec::new(),
        complete: true,
    };
    let findings = audit["findings"].as_array().map_or(&[][..], Vec::as_slice);
    for finding in findings.iter().filter(|finding| finding["item"] == "3.2") {
        let subject = finding["subject"].as_str().unwrap_or_default().to_string();
        let skip = |reason| Skipped {
            subject: subject.clone(),
            reason,
        };
        let measured = &finding["measured"];
        let nets: Vec<&str> = measured["reference_nets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if nets.len() > 1 {
            out.skipped.push(skip("two_reference_nets"));
            continue;
        }
        let Some(net) = nets.first().filter(|net| ground.contains(**net)) else {
            out.skipped.push(skip("not_ground"));
            continue;
        };
        let (Some(x), Some(y)) = (finding["at"]["x"].as_f64(), finding["at"]["y"].as_f64()) else {
            out.skipped.push(skip("no_position"));
            continue;
        };
        out.targets.push(Target {
            net: net.to_string(),
            at: (x, y),
            reference_layers: measured["reference_layers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            max_mm: finding["threshold"]["stitch_via_max_mm"]
                .as_f64()
                .unwrap_or(DEFAULT_LIMIT_MM),
            subject,
        });
    }
    let listed = (out.targets.len() + out.skipped.len()) as f64;
    out.complete = unstitched(audit).is_none_or(|count| listed >= count);
    out
}

/// S2: the audit of `board` again, in-process (no KiCad run), with `max_findings: 500`, so
/// every 3.2 finding is listed; a `max_findings` changes no score.
pub(crate) async fn full_findings(tools: &dyn ToolCaller, board: &Path) -> Result<Value> {
    let args = json!({
        "board": board.display().to_string(),
        "max_findings": FULL_FINDINGS,
    });
    let (refused, text) = tools
        .call("audit_layout_quality", &args)
        .await
        .context("the full audit failed")?;
    ensure!(!refused, "the full audit refused: {}", clipped(&text));
    serde_json::from_str(&text).context("the full audit result is not JSON")
}

/// A message cut to a readable length.
fn clipped(text: &str) -> String {
    const SHOWN: usize = 300;
    match text.char_indices().nth(SHOWN) {
        Some((cut, _)) => format!("{}...", &text[..cut]),
        None => text.to_string(),
    }
}

/// Whether `via` serves `target`: the same net, and within its limit (with
/// [`KEEP_SLACK_MM`]).
fn serves(via: &PlannedVia, target: &Target) -> bool {
    via.net == target.net
        && (via.x_mm - target.at.0).hypot(via.y_mm - target.at.1) <= target.max_mm + KEEP_SLACK_MM
}

/// S3: the vias of `items` that belong to a target's net and lie within that target's
/// `max_mm` of its `at` (1 µm of slack). Every track and every other via is dropped (those
/// join copper groups, which is Z9's work). Returns the kept items and the dropped count.
/// Removing copper raises no clearance, but a kept via the planner accepted only because an
/// earlier, dropped proposal anchored its fill group can sit in an island KiCad removes:
/// the DRC (`isolated_copper`) then names it, and S5's one retry drops it.
pub(crate) fn keep_reference_vias(items: &Planned, targets: &[Target]) -> (Planned, usize) {
    let vias: Vec<PlannedVia> = items
        .vias
        .iter()
        .filter(|via| targets.iter().any(|target| serves(via, target)))
        .cloned()
        .collect();
    let dropped = items.tracks.len() + items.vias.len() - vias.len();
    (
        Planned {
            tracks: Vec::new(),
            vias,
        },
        dropped,
    )
}

/// `above` of S5's tolerance clauses: a value that was given and is no longer, or that
/// rose by more than `tolerance`.
fn above(was: Option<f64>, now: Option<f64>, tolerance: f64) -> bool {
    match (was, now) {
        (Some(_), None) => true,
        (Some(was), Some(now)) => now > was + tolerance,
        (None, _) => false,
    }
}

/// Whether the call took a pour record for this board: it has one or an error of one
/// (`judge_one` takes it whenever the call has pour nets).
fn pour_record_due(rest: &CheckedRest) -> bool {
    rest.measures.pours.is_some() || rest.measures.pours_error.is_some()
}

/// The gates by name, each with whether it passed.
fn gate_verdicts(gates: &Gates) -> [(&'static str, bool); 4] {
    [
        ("g1", gates.g1.is_empty()),
        ("g2", gates.g2.is_empty()),
        ("g3", gates.g3.is_empty()),
        ("g4", gates.g4.is_empty()),
    ]
}

/// Both keys of the two boards, as one pool (`pour_rank::ranking`).
fn pair_keys(before: &CheckedRest, after: &CheckedRest) -> [(Vec<i64>, Vec<i64>); 2] {
    let pool = [
        (&before.measures, &before.gates, true),
        (&after.measures, &after.gates, true),
    ];
    let routing = pour_rank::ranking(RankMode::Routing, &pool);
    let pours = pour_rank::ranking(RankMode::Pours, &pool);
    [
        (routing[0].clone(), pours[0].clone()),
        (routing[1].clone(), pours[1].clone()),
    ]
}

/// S5: every clause that refuses `stitched` against `before` (both boards of the same run:
/// the post-Z9 one and the stitched one); empty when it may replace it. Both `rest`s must
/// hold the pour record (`measures.pours`) when the call has pour nets. In order, with
/// `pour_prune`'s helpers:
/// - `unconnected`: KiCad's unconnected count is not EQUAL (a reference via may not close
///   an open: Z9 does that, and its vias are delivered differently, S10);
/// - `rule:<name>`: a KiCad rule's count is higher (`pour_prune::drc_refusals`); when
///   either of these two refuses, only these;
/// - `unchecked`: either side has no `rest`, or no pour record where one is due;
/// - `gate:<g>`: a gate's verdict differs (passed and failed alike, so the status the
///   judge reports cannot depend on a via the session may not carry);
/// - `key:routing:<position>` and `key:pours:<position>`: an entry of that key above
///   `before`'s, the two boards as the pool (`pour_rank::ranking`, `entries_above`), the
///   seconds left out. Both modes, whatever the call's, so the rank-key setting does not
///   change what is safe;
/// - `not_better`: 3.2's unstitched count is not lower (or either side lacks it);
/// - `fill:<net>/<layer>`: a zone layer of a net not in `stitched_nets` has less total
///   fill than `before`'s minus `pour_prune::FILL_TOLERANCE_MM2`; the stitched nets' own
///   zones are judged by the pour entries of the pours key instead (a same-net via may
///   change their outline);
/// - `unscored`: a check that scored in `before` does not;
/// - `loops`: 4.1 worse than `before`'s by `pour_prune`'s loop clause and tolerances
///   (`loops_worse`);
/// - `reference`: 3.1's length off the reference higher by more than
///   [`LENGTH_OFF_REFERENCE_TOLERANCE_MM`] (an inner-plane antipad can lower 3.1's
///   coverage, and copper quality is rounded to whole points, so a 3.2 gain could hide it);
/// - `ground`: 3.3's floating ground area or islands higher.
pub(crate) fn stitch_refusals(
    before: &Checked,
    stitched: &Checked,
    stitched_nets: &BTreeSet<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    if stitched.unconnected != before.unconnected {
        out.push("unconnected".to_string());
    }
    out.extend(
        pour_prune::drc_refusals(before, before.unconnected, &stitched.violations)
            .into_iter()
            .filter(|clause| clause != "unconnected"),
    );
    if !out.is_empty() {
        return out;
    }
    let (Some(was), Some(now)) = (&before.rest, &stitched.rest) else {
        return vec!["unchecked".to_string()];
    };
    let due = pour_record_due(was) || pour_record_due(now);
    if due && (was.measures.pours.is_none() || now.measures.pours.is_none()) {
        return vec!["unchecked".to_string()];
    }

    for ((gate, passed), (_, passes)) in gate_verdicts(&was.gates)
        .into_iter()
        .zip(gate_verdicts(&now.gates))
    {
        if passed != passes {
            out.push(format!("gate:{gate}"));
        }
    }

    let [(routing_was, pours_was), (routing_now, pours_now)] = pair_keys(was, now);
    for position in pour_rank::entries_above(&routing_was, &routing_now) {
        out.push(format!("key:routing:{position}"));
    }
    for position in pour_rank::entries_above(&pours_was, &pours_now) {
        out.push(format!("key:pours:{position}"));
    }

    let better = match (
        was.measures.checklist.unstitched_layer_changes,
        now.measures.checklist.unstitched_layer_changes,
    ) {
        (Some(was), Some(now)) => now < was,
        _ => false,
    };
    if !better {
        out.push("not_better".to_string());
    }

    let had = pour_prune::fill_by_zone_layer(&was.fills);
    let has = pour_prune::fill_by_zone_layer(&now.fills);
    let mut lost: BTreeSet<String> = BTreeSet::new();
    for (&(net, layer, zone), &area) in &had {
        if stitched_nets.contains(net) {
            continue;
        }
        let now_area = has.get(&(net, layer, zone)).copied().unwrap_or(0.0);
        if now_area < area - pour_prune::FILL_TOLERANCE_MM2 {
            lost.insert(format!("fill:{net}/{layer}"));
        }
    }
    out.extend(lost);

    if !was.audit.scored.is_subset(&now.audit.scored) {
        out.push("unscored".to_string());
    }
    if pour_prune::loops_worse(&was.audit, &now.audit) {
        out.push("loops".to_string());
    }
    let (first, next) = (&was.measures.checklist, &now.measures.checklist);
    if above(
        first.length_off_reference_mm,
        next.length_off_reference_mm,
        LENGTH_OFF_REFERENCE_TOLERANCE_MM,
    ) {
        out.push("reference".to_string());
    }
    if above(first.floating_ground_mm2, next.floating_ground_mm2, 0.0)
        || above(
            first.floating_ground_islands,
            next.floating_ground_islands,
            0.0,
        )
    {
        out.push("ground".to_string());
    }
    out
}

/// One via the round wrote and kept (S6), in board millimetres.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct StitchVia {
    pub net: String,
    pub x_mm: f64,
    pub y_mm: f64,
    pub size_mm: f64,
    pub drill_mm: f64,
}

impl StitchVia {
    fn of(via: &PlannedVia) -> Self {
        Self {
            net: via.net.clone(),
            x_mm: via.x_mm,
            y_mm: via.y_mm,
            size_mm: via.size_mm,
            drill_mm: via.drill_mm,
        }
    }

    fn planned(&self) -> PlannedVia {
        PlannedVia {
            net: self.net.clone(),
            x_mm: self.x_mm,
            y_mm: self.y_mm,
            size_mm: self.size_mm,
            drill_mm: self.drill_mm,
        }
    }
}

/// The round's record (S6): `Judged::reference_stitch`, each run's `run.json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct StitchRecord {
    /// `kept` (the stitched board is the judged board), `refused` (S5), `none_needed`
    /// (no target), `no_proposal`, `planner_refused` (with `reason`), `failed` (with
    /// `reason`; the post-Z9 board stays, the judge does not fail).
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub targets: usize,
    pub skipped: Vec<Skipped>,
    pub proposals: usize,
    /// Vias kept by S3 and written.
    pub written_vias: usize,
    /// Proposed items S3 dropped.
    pub dropped: usize,
    /// The kept vias (`kept` only): what the delivery writes into the session (S10).
    pub vias: Vec<StitchVia>,
    /// Both boards' checks, as S5 read them, for the independent replay (S6).
    pub before: CheckedSummary,
    pub after: Option<CheckedSummary>,
    /// S5's one retry: the vias it dropped, and the clauses that named them.
    pub retried: Option<Retry>,
    /// S5's clauses when `refused`.
    pub refused_by: Vec<String>,
    pub remaining: Vec<Remaining>,
    /// KiCad refills and DRCs, and the round's seconds (inside the search's budget: the
    /// judge lanes run it).
    pub kicad_runs: usize,
    pub seconds: f64,
}

impl StitchRecord {
    pub(crate) fn new(before: CheckedSummary) -> Self {
        Self {
            outcome: "failed",
            reason: None,
            targets: 0,
            skipped: Vec::new(),
            proposals: 0,
            written_vias: 0,
            dropped: 0,
            vias: Vec::new(),
            before,
            after: None,
            retried: None,
            refused_by: Vec::new(),
            remaining: Vec::new(),
            kicad_runs: 0,
            seconds: 0.0,
        }
    }
}

/// One board's check as S5 reads it, serialized into `run.json` (S6): DRC counts, the
/// gates' verdicts, both keys (`routing`, `pours`; the two boards as the pool), the fill
/// per zone layer, 3.1, 3.2, 3.3 and 4.1, the scored checks.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(crate) struct CheckedSummary {
    pub unconnected: usize,
    pub violations: BTreeMap<String, usize>,
    pub gates_passed: Vec<String>,
    pub key_routing: Vec<i64>,
    pub key_pours: Vec<i64>,
    /// Total fill by `net/layer/zone UUID` (`-` for no UUID), mm².
    pub fill_mm2: BTreeMap<String, f64>,
    pub checklist: ChecklistItems,
    pub scored: Vec<String>,
}

/// `checked` as S5 read it, with the two keys given.
fn summary(checked: &Checked, keys: Option<(Vec<i64>, Vec<i64>)>) -> CheckedSummary {
    let (key_routing, key_pours) = keys.unwrap_or_default();
    let mut out = CheckedSummary {
        unconnected: checked.unconnected,
        violations: checked.violations.clone(),
        key_routing,
        key_pours,
        ..CheckedSummary::default()
    };
    if let Some(rest) = &checked.rest {
        out.gates_passed = gate_verdicts(&rest.gates)
            .into_iter()
            .filter(|(_, passed)| *passed)
            .map(|(gate, _)| gate.to_string())
            .collect();
        out.fill_mm2 = pour_prune::fill_by_zone_layer(&rest.fills)
            .into_iter()
            .map(|((net, layer, zone), area)| {
                (
                    format!("{net}/{layer}/{}", zone.unwrap_or("-")),
                    (area * 1000.0).round() / 1000.0,
                )
            })
            .collect();
        out.checklist = rest.measures.checklist.clone();
        out.scored = rest.audit.scored.iter().cloned().collect();
    }
    out
}

/// The summaries of `before` alone (the round stopped before a stitched board), with its
/// keys of a pool of one.
fn summary_alone(before: &Checked) -> CheckedSummary {
    let keys = before.rest.as_ref().map(|rest| {
        let pool = [(&rest.measures, &rest.gates, true)];
        (
            pour_rank::ranking(RankMode::Routing, &pool)[0].clone(),
            pour_rank::ranking(RankMode::Pours, &pool)[0].clone(),
        )
    });
    summary(before, keys)
}

/// The summaries of both boards, their keys taken over the two as one pool, as S5 reads
/// them.
fn summaries(before: &Checked, after: &Checked) -> (CheckedSummary, CheckedSummary) {
    match (&before.rest, &after.rest) {
        (Some(was), Some(now)) => {
            let [first, second] = pair_keys(was, now);
            (summary(before, Some(first)), summary(after, Some(second)))
        }
        _ => (summary_alone(before), summary_alone(after)),
    }
}

/// S5's one retry (S11 in the design's table): when a refusal names copper (a DRC item's
/// position, or a zone layer for `fill`), the vias within that item's reach are dropped
/// and the rest is checked once more. A refusal that names no via ends the round
/// `refused`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Retry {
    pub dropped_vias: usize,
    pub because: Vec<String>,
}

/// What [`reference_round`] hands back when it kept the stitched board: judge_one goes on
/// with this board, report, audit and check, and the vias go to the delivery (S10).
#[derive(Debug, Clone)]
pub(crate) struct Stitched {
    pub board: PathBuf,
    pub drc: DrcReport,
    pub audit: Value,
    pub checked: Checked,
    pub vias: Vec<PlannedVia>,
}

/// What judge_one already holds that the stitched board's measures, gates and pour record
/// need (`route_judge::measure`, `gates`, `zone_fill::pour_record`).
pub(crate) struct RoundInputs<'a> {
    pub baseline: &'a Baseline,
    pub routed: &'a Routed,
    pub plan: &'a crate::specctra_ses::SesImportPlan,
    pub widths: &'a crate::intent_width_report::WidthReport,
    /// Z9's vias, as `measure` counts them.
    pub stitching_vias: usize,
    /// The first refilled board's report (`zone_fill::pour_record`'s `first_refill`);
    /// `None` when the call has no pour net.
    pub first_drc: Option<&'a DrcReport>,
}

/// A future the reference planner returns.
pub(crate) type PlanFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>>;

/// How the round reaches the planner (S8): in-process, with the profile that accepts local
/// pad overrides and teardrop zones with no UUID. The registry's `plan_route_refinement`
/// and Z9 keep the strict profile, so the switch off changes nothing. The production
/// implementation calls `tools::route_refine`'s new in-process entry; the tests script it.
pub(crate) trait ReferencePlanner: Send + Sync {
    fn plan<'a>(&'a self, board: &'a Path, nets: &'a [String], radius_mm: f64) -> PlanFuture<'a>;
}

/// The production [`ReferencePlanner`]: `tools::route_refine::plan_reference_stitching`.
pub(crate) struct InProcessPlanner;

impl ReferencePlanner for InProcessPlanner {
    fn plan<'a>(&'a self, board: &'a Path, nets: &'a [String], radius_mm: f64) -> PlanFuture<'a> {
        Box::pin(crate::tools::route_refine::plan_reference_stitching(
            board, nets, radius_mm,
        ))
    }
}

/// One board's [`Checked`] as judge_one would take it: KiCad's counts of `drc`, the pour
/// zones' fill of `board` (`pour_prune::fills_of`), `route_judge::measure` from `audit` and
/// the run's plan and widths (Z9's vias, the route seconds), the pour record against the
/// first refill (`zone_fill::pour_record`, when the call has pour nets), the gates, and the
/// audit's items (`pour_prune::audit_items`). No length probe: both boards S5 compares
/// are taken this way.
pub(crate) async fn check_board(
    ctx: &JudgeContext,
    inputs: &RoundInputs<'_>,
    board: &Path,
    drc: &DrcReport,
    audit: &Value,
) -> Result<Checked> {
    let unconnected = route_judge::unconnected_total(drc)?;
    let violations = route_judge::violations_by_rule(drc);
    let fills = pour_prune::fills_of(board, &ctx.pour_nets).await?;
    let mut measures = route_judge::measure(
        ctx,
        drc,
        audit,
        inputs.plan,
        inputs.widths,
        inputs.stitching_vias,
        inputs.routed.held.seconds,
    )?;
    if let Some(first) = inputs.first_drc {
        let taken = crate::zone_fill::pour_record(
            board,
            first.clone(),
            drc.clone(),
            &ctx.pour_nets,
            inputs.baseline.pour_fills.as_deref(),
        )
        .await;
        crate::zone_fill::record_pours(&mut measures, taken);
    }
    let gates = route_judge::gates(&measures, inputs.baseline);
    Ok(Checked {
        unconnected,
        violations,
        rest: Some(CheckedRest {
            fills,
            measures,
            gates,
            audit: pour_prune::audit_items(audit),
        }),
    })
}

/// judge_one's step: the post-Z9 board's [`Checked`] ([`check_board`]), then
/// [`reference_round`] with the tools' planner (`ToolCaller::reference_planner`). When the
/// post-Z9 check cannot be taken the round is `failed` and the board stays.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn judged_round(
    ctx: &JudgeContext,
    tools: &dyn ToolCaller,
    inputs: &RoundInputs<'_>,
    config: Option<&Path>,
    run_dir: &Path,
    board: &Path,
    drc: &DrcReport,
    audit: &Value,
) -> (Option<Stitched>, StitchRecord) {
    let started = std::time::Instant::now();
    match check_board(ctx, inputs, board, drc, audit).await {
        Ok(before) => {
            reference_round(
                ctx,
                tools,
                tools.reference_planner(),
                inputs,
                config,
                run_dir,
                board,
                audit,
                &before,
            )
            .await
        }
        Err(error) => {
            let mut record = StitchRecord::new(CheckedSummary::default());
            record.reason = Some(format!("the post-Z9 board's check failed: {error:#}"));
            record.seconds = pour_prune::round1(started.elapsed().as_secs_f64());
            (None, record)
        }
    }
}

/// Removes one of the round's own folders (`<run dir>/s1`, `<run dir>/s1-retry`); a link
/// is never followed or removed, and a folder that is already gone is fine.
fn remove_round_folder(run_dir: &Path, folder: &Path) {
    let ours = folder.parent() == Some(run_dir)
        && folder
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == FIRST_FOLDER || name == RETRY_FOLDER);
    if ours {
        pour_prune::remove_own(folder);
    }
}

/// The targets no via of `vias` serves, each with `reason`.
fn remaining(targets: &[Target], vias: &[PlannedVia], reason: &str) -> Vec<Remaining> {
    targets
        .iter()
        .filter(|target| !vias.iter().any(|via| serves(via, target)))
        .map(|target| Remaining {
            subject: target.subject.clone(),
            net: target.net.clone(),
            reason: reason.to_string(),
        })
        .collect()
}

/// The planner's reason for the reference changes it left: its `blocked` row of the
/// `stitching` kind that names them, else a reason of the round's own.
fn planner_reason(plan: &Value) -> String {
    plan["blocked"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["kind"] == "stitching")
        .filter_map(|row| row["reason"].as_str())
        .find(|reason| reason.contains("reference changes remain"))
        .map_or_else(
            || "no proposed via of the net lies within the limit".to_string(),
            str::to_string,
        )
}

/// The nets of `vias`.
fn nets_of(vias: &[PlannedVia]) -> BTreeSet<String> {
    vias.iter().map(|via| via.net.clone()).collect()
}

/// Whether `point` lies inside the closed ring `ring` (even-odd rule).
fn inside(ring: &[(f64, f64)], point: (f64, f64)) -> bool {
    let mut odd = false;
    for (i, &(x0, y0)) in ring.iter().enumerate() {
        let (x1, y1) = ring[(i + 1) % ring.len()];
        if (y0 > point.1) != (y1 > point.1) {
            let x = x0 + (point.1 - y0) * (x1 - x0) / (y1 - y0);
            if point.0 < x {
                odd = !odd;
            }
        }
    }
    odd
}

/// The distance from `point` to the segment `a`-`b`.
fn to_segment(point: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((point.0 - a.0) * dx + (point.1 - a.1) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (point.0 - (a.0 + t * dx)).hypot(point.1 - (a.1 + t * dy))
}

/// Whether a via at `at` with radius `radius` reaches the ring: inside it, or within
/// `radius + RETRY_REACH_MM` of its edge.
fn reaches_ring(ring: &[(f64, f64)], at: (f64, f64), radius: f64) -> bool {
    if ring.len() < 3 {
        return false;
    }
    inside(ring, at)
        || (0..ring.len())
            .any(|i| to_segment(at, ring[i], ring[(i + 1) % ring.len()]) <= radius + RETRY_REACH_MM)
}

/// S5's retry: the indices of the `vias` a refusal names.
/// - `unconnected` and `rule:<name>`: every item position of the stitched board's DRC
///   entries of that category. A position that is a vertex of a saved fill piece of the
///   stitched board (within [`VERTEX_MATCH_MM`]) names the vias that reach that piece (an
///   island KiCad reports at one of its vertices); any other position names the vias
///   whose centre lies within their radius plus [`RETRY_REACH_MM`] of it.
/// - `fill:<net>/<layer>`: the vias that reach a fill piece of that net and layer on the
///   post-Z9 board (`before_text`): the antipad a through via cuts lies there.
/// - Every other clause names no via.
pub(crate) fn named_vias(
    refused: &[String],
    vias: &[PlannedVia],
    drc: &DrcReport,
    before_text: &str,
    stitched_text: &str,
) -> BTreeSet<usize> {
    let mut positions: Vec<(f64, f64)> = Vec::new();
    for clause in refused {
        let entries: Vec<&crate::tools::cli::DrcViolation> = if clause == "unconnected" {
            drc.unconnected_items.iter().flatten().collect()
        } else if let Some(rule) = clause.strip_prefix("rule:") {
            drc.violations.iter().filter(|v| v.rule == rule).collect()
        } else {
            continue;
        };
        for entry in entries {
            positions.extend(entry.pos.iter().map(|pos| (pos.x, pos.y)));
            positions.extend(
                entry
                    .items
                    .iter()
                    .filter_map(|item| item.pos.as_ref())
                    .map(|pos| (pos.x, pos.y)),
            );
        }
    }
    let mut named = BTreeSet::new();
    if !positions.is_empty() {
        let rings = crate::zone_fill::fill_rings(stitched_text).unwrap_or_default();
        for position in positions {
            let pieces: Vec<&crate::zone_fill::FillRing> = rings
                .iter()
                .filter(|ring| {
                    ring.points.iter().any(|vertex| {
                        (vertex.0 - position.0).hypot(vertex.1 - position.1) <= VERTEX_MATCH_MM
                    })
                })
                .collect();
            for (index, via) in vias.iter().enumerate() {
                let at = (via.x_mm, via.y_mm);
                let radius = via.size_mm / 2.0;
                let hit = if pieces.is_empty() {
                    (at.0 - position.0).hypot(at.1 - position.1) <= radius + RETRY_REACH_MM
                } else {
                    pieces
                        .iter()
                        .any(|piece| reaches_ring(&piece.points, at, radius))
                };
                if hit {
                    named.insert(index);
                }
            }
        }
    }
    let fills: Vec<(&str, &str)> = refused
        .iter()
        .filter_map(|clause| clause.strip_prefix("fill:"))
        .filter_map(|rest| rest.rsplit_once('/'))
        .collect();
    if !fills.is_empty() {
        let rings = crate::zone_fill::fill_rings(before_text).unwrap_or_default();
        for (net, layer) in fills {
            for ring in rings
                .iter()
                .filter(|ring| ring.net.as_deref() == Some(net) && ring.layer == layer)
            {
                for (index, via) in vias.iter().enumerate() {
                    if reaches_ring(&ring.points, (via.x_mm, via.y_mm), via.size_mm / 2.0) {
                        named.insert(index);
                    }
                }
            }
        }
    }
    named
}

/// One stitched board: `vias` written over `board_text` (`scratch_board::write_planned`)
/// into `<folder>/<board file name>` (the project copied as a Z9 round copies it), then
/// KiCad's refill and DRC on `config`.
async fn write_stitched(
    ctx: &JudgeContext,
    config: Option<&Path>,
    board: &Path,
    board_text: &str,
    vias: &[PlannedVia],
    run_dir: &Path,
    folder: &Path,
) -> Result<(PathBuf, DrcReport)> {
    let items = Planned {
        tracks: Vec::new(),
        vias: vias.to_vec(),
    };
    let text = crate::scratch_board::write_planned(board_text, &items)
        .context("the reference vias could not be written")?;
    remove_round_folder(run_dir, folder);
    let next = crate::scratch_board::copy_project(board, folder)?;
    route_judge::write_board(&next, &text)?;
    let drc = route_judge::refill(ctx, config, &next)
        .await
        .context("refill and DRC of the stitched board")?;
    Ok((next, drc))
}

/// S1 to S5 on the post-Z9 board of one judged run, in `<run dir>/s1` (project copied as a
/// Z9 round is). Called only when the post-Z9 audit's 3.2 count is above 0 (judge_one
/// builds `before` only then, so a run with nothing to serve pays nothing). `before` is
/// that board's [`Checked`] with the pour record; `audit` its audit JSON (S2's input).
/// Steps: [`targets`] (and [`full_findings`] when not complete); none, `none_needed`.
/// `planner.plan` (`nets`: the targets' nets, radius: the targets' largest `max_mm`); a
/// refusal, `planner_refused`. `scratch_board::planned_items`, [`keep_reference_vias`];
/// nothing kept, `no_proposal`. `scratch_board::write_planned`, KiCad's refill and DRC,
/// the audit, the fills, `measure`, the pour record and `gates` of the stitched board from
/// `inputs`, then [`stitch_refusals`], with one retry (S5) in `<run dir>/s1-retry`. Never
/// an `Err`: a step's error is `failed` with the reason, and the post-Z9 board stays. The
/// round's folders go unless the round is kept.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn reference_round(
    ctx: &JudgeContext,
    tools: &dyn ToolCaller,
    planner: &dyn ReferencePlanner,
    inputs: &RoundInputs<'_>,
    config: Option<&Path>,
    run_dir: &Path,
    board: &Path,
    audit: &Value,
    before: &Checked,
) -> (Option<Stitched>, StitchRecord) {
    let started = std::time::Instant::now();
    let mut record = StitchRecord::new(summary_alone(before));
    let round = Round {
        ctx,
        tools,
        planner,
        inputs,
        config,
        run_dir,
        board,
        audit,
        before,
    };
    let kept = match round.steps(&mut record).await {
        Ok(kept) => kept,
        Err(error) => {
            record.outcome = "failed";
            record.reason = Some(format!("{error:#}"));
            None
        }
    };
    if kept.is_none() {
        remove_round_folder(run_dir, &run_dir.join(FIRST_FOLDER));
        remove_round_folder(run_dir, &run_dir.join(RETRY_FOLDER));
    }
    record.seconds = pour_prune::round1(started.elapsed().as_secs_f64());
    (kept, record)
}

/// The arguments of one [`reference_round`].
struct Round<'r, 'i> {
    ctx: &'r JudgeContext,
    tools: &'r dyn ToolCaller,
    planner: &'r dyn ReferencePlanner,
    inputs: &'r RoundInputs<'i>,
    config: Option<&'r Path>,
    run_dir: &'r Path,
    board: &'r Path,
    audit: &'r Value,
    before: &'r Checked,
}

impl Round<'_, '_> {
    async fn steps(&self, record: &mut StitchRecord) -> Result<Option<Stitched>> {
        // S2: the targets, from every 3.2 finding.
        let ground = pour_prune::audit_items(self.audit).ground;
        let mut found = targets(self.audit, &ground);
        if !found.complete {
            let full = full_findings(self.tools, self.board).await?;
            found = targets(&full, &ground);
        }
        record.targets = found.targets.len();
        record.skipped = found.skipped.clone();
        if found.targets.is_empty() {
            record.outcome = "none_needed";
            return Ok(None);
        }

        // S3: one planner call for every target.
        let nets: Vec<String> = found
            .targets
            .iter()
            .map(|target| target.net.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let radius = found
            .targets
            .iter()
            .map(|target| target.max_mm)
            .fold(0.0, f64::max);
        let plan = match self.planner.plan(self.board, &nets, radius).await {
            Ok(plan) => plan,
            Err(error) => {
                let reason = format!("{error:#}");
                record.outcome = "planner_refused";
                record.remaining = remaining(&found.targets, &[], &reason);
                record.reason = Some(reason);
                return Ok(None);
            }
        };
        let reason = planner_reason(&plan);
        record.proposals = plan["proposals"].as_array().map_or(0, Vec::len);
        let items = crate::scratch_board::planned_items(&plan)
            .context("the planner's proposals are not usable")?;
        let (kept, dropped) = keep_reference_vias(&items, &found.targets);
        record.dropped = dropped;
        if kept.vias.is_empty() {
            record.outcome = "no_proposal";
            record.remaining = remaining(&found.targets, &[], &reason);
            return Ok(None);
        }

        // S4 and S5: the stitched board, checked; one retry.
        let board_text = tokio::fs::read_to_string(self.board)
            .await
            .with_context(|| format!("read the post-Z9 board '{}'", self.board.display()))?;
        let mut vias = kept.vias;
        for attempt in 0..=REFERENCE_ROUNDS {
            let folder = self.run_dir.join(if attempt == 0 {
                FIRST_FOLDER
            } else {
                RETRY_FOLDER
            });
            let (path, drc) = write_stitched(
                self.ctx,
                self.config,
                self.board,
                &board_text,
                &vias,
                self.run_dir,
                &folder,
            )
            .await?;
            record.kicad_runs += 1;
            let audit = route_judge::audit(self.tools, &path).await?;
            let checked = check_board(self.ctx, self.inputs, &path, &drc, &audit).await?;
            let refused = stitch_refusals(self.before, &checked, &nets_of(&vias));
            let (before, after) = summaries(self.before, &checked);
            record.before = before;
            record.after = Some(after);
            if refused.is_empty() {
                record.outcome = "kept";
                record.written_vias = vias.len();
                record.vias = vias.iter().map(StitchVia::of).collect();
                record.refused_by = Vec::new();
                record.remaining = remaining(&found.targets, &vias, &reason);
                return Ok(Some(Stitched {
                    board: path,
                    drc,
                    audit,
                    checked,
                    vias,
                }));
            }
            if attempt < REFERENCE_ROUNDS {
                let stitched_text = tokio::fs::read_to_string(&path).await.unwrap_or_default();
                let named = named_vias(&refused, &vias, &drc, &board_text, &stitched_text);
                if !named.is_empty() {
                    record.retried = Some(Retry {
                        dropped_vias: named.len(),
                        because: refused.clone(),
                    });
                    vias = vias
                        .into_iter()
                        .enumerate()
                        .filter(|(index, _)| !named.contains(index))
                        .map(|(_, via)| via)
                        .collect();
                    remove_round_folder(self.run_dir, &folder);
                    if !vias.is_empty() {
                        continue;
                    }
                }
            }
            record.outcome = "refused";
            record.refused_by = refused;
            record.remaining = remaining(&found.targets, &[], "refused");
            return Ok(None);
        }
        Ok(None)
    }
}

/// S6: 3.1, 3.2 and 4.1 of every judged run, from its audit (`metrics`
/// `3.1.length_off_reference_mm`, `3.2.unstitched_layer_changes`, `4.1.worst_loop_mm`).
/// Recorded in `Measures::checklist`; no key reads it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct ChecklistItems {
    pub length_off_reference_mm: Option<f64>,
    pub unstitched_layer_changes: Option<f64>,
    /// 3.3's floating ground area, mm², and islands (S5 `ground`).
    pub floating_ground_mm2: Option<f64>,
    pub floating_ground_islands: Option<f64>,
    pub worst_loop_mm: Option<f64>,
}

/// [`ChecklistItems`] of an audit: `3.1.length_off_reference_mm`,
/// `3.2.unstitched_layer_changes`, `3.3.floating_ground_area_mm2`,
/// `3.3.floating_ground_islands` and `4.1.worst_loop_mm` of its `metrics`; a value the
/// audit did not give is `None`.
pub(crate) fn checklist_items(audit: &Value) -> ChecklistItems {
    let metric = |key: &str| audit["metrics"][key].as_f64();
    ChecklistItems {
        length_off_reference_mm: metric("3.1.length_off_reference_mm"),
        unstitched_layer_changes: metric("3.2.unstitched_layer_changes"),
        floating_ground_mm2: metric("3.3.floating_ground_area_mm2"),
        floating_ground_islands: metric("3.3.floating_ground_islands"),
        worst_loop_mm: metric("4.1.worst_loop_mm"),
    }
}

/// S10: the via padstack of the export manifest (`padstacks`, `purpose == "via"`: what
/// the importer resolves a session via by) whose diameter and drill equal `size_mm` and
/// `drill_mm` within 1 µm; `None` when none matches (the delivery is then `no_padstack`).
/// Only a round through via the importer takes is matched (`shape` `circle`, equal sizes,
/// a drill). The planner's via is the largest class size raised to the rule minimums, so
/// on a board with several via sizes this can be `None`: the measurement records it per
/// board. A manifest with no `padstacks` list is an error.
pub(crate) fn manifest_padstack(
    manifest: &Value,
    size_mm: f64,
    drill_mm: f64,
) -> Result<Option<String>> {
    let rows = manifest["padstacks"]
        .as_array()
        .context("the manifest has no 'padstacks' list")?;
    const TOLERANCE_MM: f64 = 0.001;
    let um = |row: &Value, key: &str| row[key].as_i64().map(|value| value as f64 / 1000.0);
    for row in rows {
        if row["purpose"] != "via" || row["shape"] != "circle" {
            continue;
        }
        let (Some(name), Some(x), Some(y), Some(drill)) = (
            row["name"].as_str(),
            um(row, "size_x_um"),
            um(row, "size_y_um"),
            um(row, "drill_um"),
        ) else {
            continue;
        };
        if x == y && (x - size_mm).abs() <= TOLERANCE_MM && (drill - drill_mm).abs() <= TOLERANCE_MM
        {
            return Ok(Some(name.to_string()));
        }
    }
    Ok(None)
}

/// A name as a quoted session atom; a name the atom cannot hold is refused.
fn quoted(name: &str) -> Result<String> {
    ensure!(
        !name.is_empty() && !name.contains('"') && !name.contains('\\'),
        "the name {name:?} cannot be written into the session"
    );
    Ok(format!("\"{name}\""))
}

/// S10 ("ZR0b in reverse"): `ses_text` with one `(via "<padstack>" <x> <y>)` block a via,
/// in the session's units and axes (the scanner `pour_prune` uses), appended to the via's
/// net block in `network_out`; a `(net "<name>" …)` block is added when the net has none.
/// Each point is snapped to the session's grid (the planner rounds to 1 nm; a session unit
/// is coarser); the full judge of the written session checks the snapped copper.
pub(crate) fn write_vias(ses_text: &str, vias: &[PlannedVia], padstack: &str) -> Result<String> {
    let network = pour_prune::network(ses_text)?;
    let padstack = quoted(padstack)?;
    let unit = network.mm_per_unit;
    let mut by_net: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for via in vias {
        let (x, y) = ((via.x_mm / unit).round(), (-via.y_mm / unit).round());
        ensure!(
            x.is_finite() && y.is_finite() && x.abs() < 1e15 && y.abs() < 1e15,
            "the via of {} at ({}, {}) is outside the session's range",
            via.net,
            via.x_mm,
            via.y_mm
        );
        by_net
            .entry(via.net.as_str())
            .or_default()
            .push(format!("(via {padstack} {} {})", x as i64, y as i64));
    }
    // (position, text), applied from the end so that earlier positions hold.
    let mut inserts: Vec<(usize, String)> = Vec::new();
    let mut new_nets = String::new();
    for (net, blocks) in by_net {
        match network.nets.iter().find(|block| block.name == net) {
            Some(block) => {
                let at = block
                    .routes
                    .last()
                    .map_or(block.block.end - 1, |route| route.end);
                let text: String = blocks
                    .iter()
                    .map(|via| format!("\n        {via}"))
                    .collect();
                inserts.push((at, text));
            }
            None => {
                new_nets.push_str(&format!("\n      (net {}", quoted(net)?));
                for via in blocks {
                    new_nets.push_str(&format!("\n        {via}"));
                }
                new_nets.push_str("\n      )");
            }
        }
    }
    if !new_nets.is_empty() {
        let at = network
            .nets
            .last()
            .map_or(network.out.end - 1, |net| net.block.end);
        inserts.push((at, new_nets));
    }
    inserts.sort_by_key(|insert| std::cmp::Reverse(insert.0));
    let mut out = ses_text.to_string();
    for (at, text) in inserts {
        out.insert_str(at, &text);
    }
    Ok(out)
}

/// The delivery's record (`search.reference_stitch`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DeliveryRecord {
    /// `delivered` (the delivered session holds the vias), `nothing_to_deliver`,
    /// `rejected` (the full judge of the written session failed S5 against the ranked
    /// judged state, or its 3.2 count is higher, or the winner would no longer rank first;
    /// with `refused_by`), `no_padstack`, `failed` (with `reason`). Anything but
    /// `delivered` and `nothing_to_deliver` leaves the session as before and adds the
    /// planner call to `next_steps`.
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub vias: usize,
    pub refused_by: Vec<String>,
    pub seconds: f64,
}

impl DeliveryRecord {
    fn new(outcome: &'static str, reason: Option<String>, vias: usize) -> Self {
        Self {
            outcome,
            reason,
            vias,
            refused_by: Vec::new(),
            seconds: 0.0,
        }
    }
}

/// The judge folder of the full judge of run `index`'s delivered session.
fn stitched_folder(index: usize) -> String {
    format!("run-{index}-stitched")
}

/// The winner's state the delivery works on, owned, so that it runs as a task of its own.
struct Delivery {
    index: usize,
    routed: Routed,
    baseline: Baseline,
    ranked: Checked,
    vias: Vec<PlannedVia>,
}

/// What [`deliver`] decided: the record, the session and its judge when they may replace
/// the winner's, and the files it wrote (they go unless the session is delivered).
struct Delivered {
    record: DeliveryRecord,
    kept: Option<(PathBuf, Judged)>,
    written: Vec<PathBuf>,
}

/// The winner's kept round, or the record of why there is nothing to deliver.
fn delivery_of(outcome: &SearchOutcome) -> std::result::Result<Delivery, DeliveryRecord> {
    let nothing =
        |reason: &str| DeliveryRecord::new("nothing_to_deliver", Some(reason.to_string()), 0);
    let Some(record) = outcome
        .winner
        .and_then(|position| outcome.runs.get(position))
    else {
        return Err(nothing("no run won"));
    };
    let (Some(routed), Some(judged)) = (&record.routed, &record.judged) else {
        return Err(nothing(
            "the winning record has no routed session or no judged board",
        ));
    };
    let Some(round) = judged
        .reference_stitch
        .as_ref()
        .filter(|round| round.outcome == "kept" && !round.vias.is_empty())
    else {
        return Err(nothing("the winner's judge kept no reference via"));
    };
    let vias: Vec<PlannedVia> = round.vias.iter().map(StitchVia::planned).collect();
    let count = vias.len();
    let Some(ranked) = judged.reference_checked.clone() else {
        return Err(DeliveryRecord::new(
            "failed",
            Some("the kept round's check is not in memory".to_string()),
            count,
        ));
    };
    let baseline = outcome.baseline.as_ref().map_err(|error| {
        DeliveryRecord::new(
            "failed",
            Some(format!("the unrouted board's baseline failed: {error}")),
            count,
        )
    })?;
    Ok(Delivery {
        index: record.index,
        routed: routed.clone(),
        baseline: baseline.clone(),
        ranked,
        vias,
    })
}

/// A delivered session's [`Checked`]: its full judge's counts, measures and gates, the
/// fill of its judged board and its audit again (in-process).
async fn checked_of(
    ctx: &JudgeContext,
    tools: &dyn ToolCaller,
    judged: &Judged,
) -> Result<Checked> {
    let audit = route_judge::audit(tools, &judged.board).await?;
    let fills = pour_prune::fills_of(&judged.board, &ctx.pour_nets).await?;
    Ok(Checked {
        unconnected: judged.measures.unconnected,
        violations: judged.measures.violations.clone(),
        rest: Some(CheckedRest {
            fills,
            measures: judged.measures.clone(),
            gates: judged.gates.clone(),
            audit: pour_prune::audit_items(&audit),
        }),
    })
}

/// S10: the clauses that refuse the delivered session's check against the ranked state:
/// [`stitch_refusals`] without `not_better` (the two boards hold the same vias), and
/// `stitching` when its 3.2 count is higher or either side lacks it.
pub(crate) fn delivery_refusals(
    ranked: &Checked,
    delivered: &Checked,
    nets: &BTreeSet<String>,
) -> Vec<String> {
    let mut out: Vec<String> = stitch_refusals(ranked, delivered, nets)
        .into_iter()
        .filter(|clause| clause != "not_better")
        .collect();
    let count = |checked: &Checked| {
        checked
            .rest
            .as_ref()
            .and_then(|rest| rest.measures.checklist.unstitched_layer_changes)
    };
    let higher = match (count(ranked), count(delivered)) {
        (Some(was), Some(now)) => now > was,
        _ => true,
    };
    if higher && !out.iter().any(|clause| clause == "unchecked") {
        out.push("stitching".to_string());
    }
    out
}

/// The delivery's steps on an owned winner: the padstack, the written session
/// (`stitched.ses` beside the winner's), its full judge with the round off, S5 against the
/// ranked state.
async fn deliver(ctx: Arc<JudgeContext>, tools: Arc<dyn ToolCaller>, job: Delivery) -> Delivered {
    let count = job.vias.len();
    let mut written = Vec::new();
    let ended = |outcome, reason: String, refused_by: Vec<String>, written: Vec<PathBuf>| {
        let mut record = DeliveryRecord::new(outcome, Some(reason), count);
        record.refused_by = refused_by;
        Delivered {
            record,
            kept: None,
            written,
        }
    };
    let first = &job.vias[0];
    let (size, drill) = (first.size_mm, first.drill_mm);
    if job
        .vias
        .iter()
        .any(|via| (via.size_mm - size).abs() > 1e-9 || (via.drill_mm - drill).abs() > 1e-9)
    {
        return ended(
            "failed",
            "the kept vias are not of one size".to_string(),
            Vec::new(),
            written,
        );
    }
    let padstack =
        match manifest_padstack(&ctx.manifest, size, drill) {
            Ok(Some(name)) => name,
            Ok(None) => return ended(
                "no_padstack",
                format!(
                    "the export manifest has no via padstack of {size} mm with a {drill} mm drill"
                ),
                Vec::new(),
                written,
            ),
            Err(error) => return ended("failed", format!("{error:#}"), Vec::new(), written),
        };
    let text = match tokio::fs::read_to_string(&job.routed.held.ses).await {
        Ok(text) => text,
        Err(error) => {
            return ended(
                "failed",
                format!(
                    "the winner's session '{}' could not be read: {error}",
                    job.routed.held.ses.display()
                ),
                Vec::new(),
                written,
            )
        }
    };
    let stitched_text = match write_vias(&text, &job.vias, &padstack) {
        Ok(text) => text,
        Err(error) => return ended("failed", format!("{error:#}"), Vec::new(), written),
    };
    let session = job.routed.folder.join("stitched.ses");
    if let Err(error) = konnect_sexp::write_new_atomic(&session, &stitched_text) {
        return ended(
            "failed",
            format!(
                "the stitched session '{}' could not be written: {error}",
                session.display()
            ),
            Vec::new(),
            written,
        );
    }
    let folder = stitched_folder(job.index);
    written.push(session.clone());
    written.push(ctx.root.join(&folder));
    // The full judge with the round off: its board holds the session's copper and Z9's.
    let mut off = (*ctx).clone();
    off.reference_stitch = ReferenceStitch::Off;
    let tools: &dyn ToolCaller = &*tools;
    let judged = match route_judge::judge_session(
        &off,
        &job.routed,
        &job.baseline,
        tools,
        &session,
        &folder,
    )
    .await
    {
        Ok(judged) => judged,
        Err(error) => {
            return ended(
                "failed",
                format!("the full judge of the stitched session failed: {error:#}"),
                Vec::new(),
                written,
            )
        }
    };
    let delivered = match checked_of(&off, tools, &judged).await {
        Ok(checked) => checked,
        Err(error) => {
            return ended(
                "failed",
                format!("the stitched session's check failed: {error:#}"),
                Vec::new(),
                written,
            )
        }
    };
    let refused = delivery_refusals(&job.ranked, &delivered, &nets_of(&job.vias));
    if !refused.is_empty() {
        return ended(
            "rejected",
            "the stitched session's full judge is not the ranked board".to_string(),
            refused,
            written,
        );
    }
    Delivered {
        record: DeliveryRecord::new("delivered", None, count),
        kept: Some((session, judged)),
        written,
    }
}

/// S10 and S11, after `pour_prune::prune_winner` on the same outcome: the delivered
/// winner's kept reference vias (its `Judged::reference_stitch`, `kept`; the PRUNED
/// winner's when the prune replaced it, since P9's full judge runs the round too) are
/// written into its delivered session ([`manifest_padstack`], [`write_vias`]) as
/// `stitched.ses` beside it. That session is judged in full with the round OFF
/// (`route_judge::judge_session`, folder `run-<index>-stitched`), so its judged board holds
/// exactly the session's copper plus Z9's. Its [`Checked`] is compared with the ranked
/// state's (kept in memory as `Judged::reference_checked`) by [`delivery_refusals`]. Then
/// the keys are taken again (`pour_prune::replace_winner`: `outcome.keys`, `pour_keys`) and
/// the winner must still rank first; then `held.ses` is `stitched.ses`, and the winner's
/// judged is the delivered session's, its round's record carried over and its
/// `reference_vias` 0. On any other outcome the session goes out as before, the written
/// files go, and the status and `next_steps` count the reference vias it lacks as they
/// count Z9's (the winner's `Measures::reference_vias`). Runs in a task of its own like
/// the prune: never fails the call; a panic or an error keeps the session as before, with
/// the reason recorded.
pub(crate) async fn deliver_winner(
    ctx: &JudgeContext,
    tools: Arc<dyn ToolCaller>,
    outcome: &mut SearchOutcome,
) -> DeliveryRecord {
    let started = std::time::Instant::now();
    let mut record = match delivery_of(outcome) {
        Err(record) => record,
        Ok(job) => {
            let count = job.vias.len();
            let fresh: Vec<PathBuf> = [
                job.routed.folder.join("stitched.ses"),
                ctx.root.join(stitched_folder(job.index)),
            ]
            .into_iter()
            .filter(|path| path.symlink_metadata().is_err())
            .collect();
            let task = tokio::spawn(deliver(Arc::new(ctx.clone()), tools, job));
            let _abort = pour_prune::AbortOnDrop(task.abort_handle());
            let done = match task.await {
                Ok(done) => done,
                Err(error) => Delivered {
                    record: DeliveryRecord::new(
                        "failed",
                        Some(format!("the delivery stopped: {error}")),
                        count,
                    ),
                    kept: None,
                    written: fresh,
                },
            };
            let Delivered {
                mut record,
                kept,
                written,
            } = done;
            match (kept, outcome.winner) {
                (Some((session, mut judged)), Some(winner)) => {
                    let original = outcome
                        .runs
                        .get(winner)
                        .and_then(|run| run.judged.as_ref())
                        .and_then(|judged| judged.reference_stitch.clone());
                    judged.reference_stitch = original;
                    judged.reference_checked = None;
                    judged.measures.reference_vias = 0;
                    if let Err(reason) =
                        pour_prune::replace_winner(outcome, winner, session, judged, ctx.rank)
                    {
                        record = DeliveryRecord::new("rejected", Some(reason), count);
                        record.refused_by = vec!["rank".to_string()];
                        written.iter().for_each(|path| pour_prune::remove_own(path));
                    }
                }
                _ => written.iter().for_each(|path| pour_prune::remove_own(path)),
            }
            record
        }
    };
    record.seconds = pour_prune::round1(started.elapsed().as_secs_f64());
    record
}

/// The registry's tools behind [`deliver_winner`] (production).
pub(crate) async fn deliver_winner_registry(
    ctx: &JudgeContext,
    tools: Arc<crate::tools::ToolContext>,
    outcome: &mut SearchOutcome,
) -> DeliveryRecord {
    deliver_winner(ctx, Arc::new(route_judge::RegistryTools(tools)), outcome).await
}

/// The nets of the winner's kept reference vias that its delivered session lacks, for
/// `next_steps` (S10): empty when there is none, or when the session holds them.
pub(crate) fn undelivered_nets(judged: &Judged) -> Vec<String> {
    if judged.measures.reference_vias == 0 {
        return Vec::new();
    }
    judged
        .reference_stitch
        .as_ref()
        .map(|round| {
            round
                .vias
                .iter()
                .map(|via| via.net.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "pour_stitch_tests.rs"]
mod tests;
