//! Family C, the convergent placement search (`docs/CONVERGENT_PLACEMENT_DESIGN.md`): one
//! scalar cost of a placement, a search that moves parts while the cost falls, and two
//! stop rules that read the cost and nothing else (no clock, no run count).
//!
//! One run refines one start (a candidate of family F or S): annealing stage by stage
//! until the best cost stops falling, then descent to a placement that no single move
//! improves. The same model, start, seed and parameters give the same placement on any
//! thread. The search around the runs (which starts, how much effort, when to stop) is
//! `place_search::converge_all`, by the rules of [`OuterRule`].

// Only the bench reaches this module (`place_search`, design C1), as with `place_search`.
#![allow(dead_code)]

use crate::place_generators::{
    legalize_with, placements_from, ring_cells, text_box, turn_vector, unit, BoardModel, Candidate,
    GapRule, GenerateFailure, Legality, GRID_MM,
};
use crate::place_held::footprint_keys;
use crate::place_schematic::Neighbourhoods;
use crate::route_search::SplitMix64;
use crate::tools::placement::force_core::Rect;
use crate::tools::placement::objectives::{self, Site};
use crate::tools::placement::{ref_prefix, CONNECTOR_EDGE_LIMIT_MM};
use konnect_ipc::types::IpcFootprintPlacement;
use konnect_sexp::board::{footprint_courtyards, footprints, zones};
use konnect_sexp::geometry::{rotate_about, transform_pad};
use konnect_sexp::parser::SexpNode;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;

/// The parameters of one run. They are part of the recipe, so a record says exactly what
/// ran, and `rerun` rebuilds the board from it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct ConvergeParams {
    /// The cost of one proper crossing between the airwire trees of two nets, mm.
    pub crossing_mm: f64,
    /// A net with more than this many distinct references is a rail (the score's limit).
    pub fanout_limit: usize,
    /// The weight of a rail's airwire length when the board has no pour on that net. A
    /// rail with a pour weighs 0: the pour connects it wherever the part sits.
    pub rail_weight: f64,
    /// The least a broken requirement costs before its excess, mm (a decoupling pair
    /// past its limit, a connector too far from the edge). A run uses the larger of
    /// this and the soft cost of its own start (`Trace::requirement_mm`), so one broken
    /// requirement always costs more than everything else the run can still gain.
    pub requirement_mm: f64,
    /// The cost of each mm a broken requirement is past its limit.
    pub requirement_slope: f64,
    /// The weight of the schematic-neighbour term, per mm; 0 turns the term off.
    pub neighbour_weight: f64,
    /// The cost of one label conflict, mm; 0 turns the term off.
    pub label_mm: f64,
    /// Whether a part may turn by quarter turns (owner decision 1). Off: no turn is
    /// proposed and every part keeps the rotation read.
    pub turn_parts: bool,
    /// The effort level k, 0 upward. Level k cools 2^k times as slowly as level 0 and
    /// scales the inner stop rule with it, so that the rule reads the same fall of the
    /// temperature at every level (design C5, C6): see `cooling_at_level`,
    /// `quiet_stages_at_level` and `quiet_gain_at_level`.
    pub level: u32,
    /// Proposals in one annealing stage, per movable part.
    pub stage_per_part: usize,
    /// Level 0's factor of the temperature from one stage to the next.
    pub cooling: f64,
    /// The shift radius is `radius_start x (T / first T)^radius_exponent`, never below
    /// 1 mm: it follows the temperature, not the stage count, so every level shrinks it
    /// over the same range of temperatures.
    pub radius_exponent: f64,
    /// At level 0, a stage that lowers the best cost by less than this share is quiet;
    /// a sweep of descent too, at every level.
    pub quiet_gain: f64,
    /// At level 0, annealing ends after this many quiet stages in a row ...
    pub quiet_stages: usize,
    /// ... once a stage accepts fewer uphill moves than this share of its proposals.
    pub uphill_floor: f64,
    /// Descent also ends after this many quiet sweeps in a row.
    pub quiet_sweeps: usize,
    /// Guards against a run that never settles. Never the expected stop: a run they end
    /// is recorded `converged: false`.
    pub max_stages: usize,
    pub max_sweeps: usize,
}

impl ConvergeParams {
    /// The values the experiment of 2026-11-20 ran with (design C2, C5, C6), at
    /// `level`: parts may turn, crossing 4 mm, fanout 8, rail weight 0.3, requirement
    /// 1000 mm and slope 50, 200 proposals per part and stage, cooling 0.95, radius exponent 0.594, quiet
    /// gain 0.001, 5 quiet stages, uphill floor 0.01, 3 quiet sweeps; and the two terms
    /// the experiment did not have, neighbour weight 0.1 and label 0.2 mm; guards 2000
    /// stages (at level 0; `max_stages_at_level` scales it) and 200 sweeps.
    pub(crate) fn measured(level: u32) -> Self {
        ConvergeParams {
            crossing_mm: 4.0,
            fanout_limit: 8,
            rail_weight: 0.3,
            requirement_mm: 1000.0,
            requirement_slope: 50.0,
            neighbour_weight: 0.1,
            label_mm: 0.2,
            turn_parts: true,
            level,
            stage_per_part: 200,
            cooling: 0.95,
            radius_exponent: 0.594,
            quiet_gain: 0.001,
            quiet_stages: 5,
            uphill_floor: 0.01,
            quiet_sweeps: 3,
            max_stages: 2000,
            max_sweeps: 200,
        }
    }

    /// 2^level, as a factor.
    fn slowdown(&self) -> f64 {
        2f64.powi(i32::try_from(self.level).unwrap_or(i32::MAX))
    }

    /// 2^level, as a count; `usize::MAX` when it does not fit.
    fn times(&self) -> usize {
        1usize.checked_shl(self.level).unwrap_or(usize::MAX)
    }

    /// `cooling` to the power 1 / 2^level: level 1 takes two stages for the fall of the
    /// temperature that level 0 makes in one.
    pub(crate) fn cooling_at_level(&self) -> f64 {
        self.cooling.powf(1.0 / self.slowdown())
    }

    /// `quiet_stages` x 2^level: the same fall of the temperature as at level 0.
    pub(crate) fn quiet_stages_at_level(&self) -> usize {
        self.quiet_stages.saturating_mul(self.times())
    }

    /// `quiet_gain` / 2^level: a stage of level k is 1 / 2^k of a stage of level 0.
    pub(crate) fn quiet_gain_at_level(&self) -> f64 {
        self.quiet_gain / self.slowdown()
    }

    /// `max_stages` x 2^level.
    pub(crate) fn max_stages_at_level(&self) -> usize {
        self.max_stages.saturating_mul(self.times())
    }

    /// Refuses a value that is not finite, a weight below 0, a cooling outside (0, 1), a
    /// quiet gain outside (0, 1), a radius exponent that is not above 0, a level above
    /// 16, and a count of 0.
    pub(crate) fn validate(&self) -> Result<(), GenerateFailure> {
        let refuse = |what: String| Err(GenerateFailure::Invalid(what));
        // The weights, and the uphill floor (a share): finite and at least 0.
        for (name, value) in [
            ("crossing_mm", self.crossing_mm),
            ("rail_weight", self.rail_weight),
            ("requirement_mm", self.requirement_mm),
            ("requirement_slope", self.requirement_slope),
            ("neighbour_weight", self.neighbour_weight),
            ("label_mm", self.label_mm),
            ("uphill_floor", self.uphill_floor),
        ] {
            if !value.is_finite() || value < 0.0 {
                return refuse(format!(
                    "{name} {value} is not a finite number of at least 0"
                ));
            }
        }
        // Written so that a value that is not a number fails too.
        for (name, value) in [("cooling", self.cooling), ("quiet_gain", self.quiet_gain)] {
            if !(value > 0.0 && value < 1.0) {
                return refuse(format!("{name} {value} is not inside (0, 1)"));
            }
        }
        if !(self.radius_exponent.is_finite() && self.radius_exponent > 0.0) {
            return refuse(format!(
                "radius_exponent {} is not a finite number above 0",
                self.radius_exponent
            ));
        }
        if self.level > 16 {
            return refuse(format!("level {} is above 16", self.level));
        }
        for (name, value) in [
            ("fanout_limit", self.fanout_limit),
            ("stage_per_part", self.stage_per_part),
            ("quiet_stages", self.quiet_stages),
            ("quiet_sweeps", self.quiet_sweeps),
            ("max_stages", self.max_stages),
            ("max_sweeps", self.max_sweeps),
        ] {
            if value == 0 {
                return refuse(format!("{name} is 0"));
            }
        }
        Ok(())
    }
}

/// The outer rules: when to stop taking new starts, and when to stop doubling the effort
/// (design C7).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct OuterRule {
    /// What counts as paying: a best cost below (1 - `gain`) times the best cost before.
    pub gain: f64,
    /// Starts: the search stops taking starts at the first count n >= `window` + 1 at
    /// which the last `window` starts did not pay.
    pub window: usize,
    /// Guards, recorded as a guard's stop: the most starts, and the highest effort level.
    pub max_starts: usize,
    pub max_level: u32,
}

impl OuterRule {
    /// The measured rule (design C7): gain 0.01, window 3, at most 64 starts, levels up
    /// to 5.
    pub(crate) fn measured() -> Self {
        OuterRule {
            gain: 0.01,
            window: 3,
            max_starts: 64,
            max_level: 5,
        }
    }

    /// Whether a best cost of `after` pays against `before`: `after` < (1 - `gain`) x
    /// `before`. An infinite `before` is paid by any finite `after`.
    pub(crate) fn pays(&self, before: f64, after: f64) -> bool {
        if before == f64::INFINITY {
            return after.is_finite();
        }
        after < (1.0 - self.gain) * before
    }
}

/// Why annealing ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnnealStop {
    /// `quiet_stages` quiet stages in a row and uphill moves below `uphill_floor`.
    Quiet,
    /// `max_stages` reached.
    Guard,
}

/// Why descent ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DescentStop {
    /// A whole sweep made no move: no single move of the set lowers the cost.
    FixedPoint,
    /// `quiet_sweeps` quiet sweeps in a row.
    Quiet,
    /// `max_sweeps` reached.
    Guard,
}

/// The cost of a placement, term by term. `cost` is their weighted sum (design C2).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct Terms {
    pub cost: f64,
    /// The score's own airwire measure: the eligible nets only, weight 1.
    pub airwire_mm: f64,
    /// The rails that count (no pour), unweighted.
    pub rail_airwire_mm: f64,
    /// The score's own proper-crossing count.
    pub crossings: usize,
    /// Decoupling pairs past their limit, and pairs that cannot be measured (the
    /// screen's own count, `place_judge::pair_violations`).
    pub pair_violations: usize,
    /// The sum of the pairs' pad-to-pad distances.
    pub pair_mm: f64,
    /// Connectors farther from the outline than the score's limit, held ones included
    /// (the screen's own count).
    pub connector_violations: usize,
    /// The sum over the parts that have schematic neighbours of their mean centre
    /// distance to them; `None` when the board has no neighbourhoods.
    pub neighbour_mm: Option<f64>,
    /// Label conflicts between parts that share a side: a silk label of one part over
    /// another part's courtyard box, and a label of one part over a label of another
    /// (each pair of labels once).
    pub label_conflicts: usize,
    /// The soft cost: `cost` without the requirement terms of the pairs and connectors.
    pub soft: f64,
}

/// What one run did, for its record.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Trace {
    /// The effort level the run was made at.
    pub level: u32,
    /// What a broken requirement cost in this run: the larger of
    /// `ConvergeParams::requirement_mm` and the start's soft cost.
    pub requirement_mm: f64,
    /// SHA-256 of the schematic neighbourhoods the run read (their JSON, keys sorted);
    /// `None` when it had none. `rerun` refuses a model whose neighbourhoods differ.
    pub hoods_sha256: Option<String>,
    /// The start as given, before it was made legal under family C's gap rule.
    pub start: Terms,
    /// The result as written: the terms of the state rebuilt from the placements with
    /// their anchors rounded to 1 µm, so the airwire length, the crossings and the two
    /// violation counts are the screen's own of the written board.
    pub end: Terms,
    /// Compound moves that made room for a broken requirement (design C4), tried and made.
    pub repairs_tried: u64,
    pub repairs_made: u64,
    pub first_temperature: f64,
    pub stages: usize,
    pub anneal_stop: AnnealStop,
    pub sweeps: usize,
    pub descent_stop: DescentStop,
    /// Legal moves whose cost change was computed.
    pub evaluations: u64,
    /// The best cost at the end of each stage, then of each sweep: the curve.
    pub best_by_step: Vec<f64>,
}

impl Trace {
    /// Both stops are a rule's, not a guard's.
    pub(crate) fn converged(&self) -> bool {
        self.anneal_stop == AnnealStop::Quiet && self.descent_stop != DescentStop::Guard
    }
}

/// A net as the cost reads it.
#[derive(Debug, Clone, PartialEq)]
struct Net<'a> {
    /// 1 for an eligible net, `rail_weight` for a rail that counts.
    weight: f64,
    /// Counted by the score's measure, and in the crossings.
    eligible: bool,
    /// Its pad sites, in the score's order (the index's): (part, index into the part's
    /// `pads`).
    sites: Vec<(usize, usize)>,
    /// Each site's reference and pad number as the index has them: the score's tree
    /// breaks ties between sites at one position by them.
    names: Vec<(&'a str, &'a str)>,
}

/// A decoupling pair as the cost reads it: (part, pad index) of the capacitor and of the
/// IC, and the limit.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pair {
    cap: (usize, usize),
    ic: (usize, usize),
    limit_mm: f64,
}

/// Where a part's pads are, as the score's own index finds them on the written board.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Frame {
    /// The footprint's anchor as read (`(at x y)`), which a part that did not move keeps.
    anchor: (f64, f64),
    /// The anchor less the courtyard box centre, as read: `placements_from` turns it with
    /// the part and adds it to the centre, rounded to 1 µm.
    offset: (f64, f64),
    /// cos and sin of the rotation as read, for a part that did not move.
    read_turn: (f64, f64),
    /// cos and sin of the rotation the written board has after 0 to 3 quarter turns
    /// (normalised and written to 6 decimals, as `placements_applied_to` writes it).
    turned: [(f64, f64); 4],
}

/// One visible silk label of a part.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Label {
    /// Its box relative to the part's centre, at 0 to 3 quarter turns of the part.
    boxes: [Rect; 4],
    /// On `F.SilkS` (the top), or on `B.SilkS`.
    top: bool,
}

/// One net's airwire tree: its length, its edges as the score's tree takes them, and the
/// box of the edges (no box when it has none).
#[derive(Debug, Clone, PartialEq)]
struct NetTree {
    mm: f64,
    edges: Vec<Edge>,
    bbox: Rect,
}

/// Everything that does not change while parts move, built once per board and run.
/// Parts are the model's, by its index. A part's position is its courtyard box centre and
/// its quarter turns from the rotation read. Box offsets below are from that centre, as
/// read, and are turned with the part (`place_generators::turn_vector`). A pad is placed
/// as the written board places it: through the anchor `placements_from` writes (1 µm)
/// and the rotation it writes, by the arithmetic of `konnect_sexp`'s pad transform, so
/// that the airwire length and the crossings are the score's of the written board to the
/// last bit.
pub(crate) struct CostModel<'a> {
    model: &'a BoardModel,
    params: ConvergeParams,
    legality: Legality<'a>,
    /// What a broken requirement costs before its excess, mm (`with_requirement`).
    requirement_mm: f64,
    /// Movable parts, in index order.
    movable: Vec<usize>,
    /// A movable part read at a multiple of 90 degrees may turn, when `turn_parts` is on;
    /// another keeps its turns.
    may_turn: Vec<bool>,
    /// Courtyard box size as read.
    sizes: Vec<(f64, f64)>,
    /// Courtyard box centre as read.
    read_centres: Vec<(f64, f64)>,
    frames: Vec<Frame>,
    /// Each indexed pad's local position (its `(at x y)` in the footprint), in file order.
    pads: Vec<Vec<(f64, f64)>>,
    /// Each visible silk label.
    labels: Vec<Vec<Label>>,
    /// The box around a part's courtyard and labels, relative to its centre, at 0 to 3
    /// quarter turns: two parts whose extents do not overlap have no label conflict.
    extents: Vec<[Rect; 4]>,
    nets: Vec<Net<'a>>,
    /// The eligible nets, in net order.
    eligible: Vec<usize>,
    /// The nets each part is on.
    part_nets: Vec<Vec<usize>>,
    pairs: Vec<Pair>,
    part_pairs: Vec<Vec<usize>>,
    /// Pairs that cannot be measured: each a violation in every state.
    unmeasured_pairs: usize,
    /// Every part the score's connector rule reads (reference prefix `J`), held ones
    /// included (their term is a constant).
    connectors: Vec<usize>,
    /// Each part's position in `connectors`.
    part_connector: Vec<Option<usize>>,
    /// Each part's schematic neighbours, by part index; empty without a neighbourhood.
    hoods: Vec<Vec<usize>>,
    /// The parts that list each part as a neighbour.
    hood_of: Vec<Vec<usize>>,
    /// Whether the board has neighbourhoods at all (`Terms::neighbour_mm`).
    has_hoods: bool,
    /// The compound moves of making room this run tried and made (`repair` adds to it,
    /// `converge` reads it into the trace): the run's one model is the one place both
    /// annealing and descent reach.
    repairs: Cell<(u64, u64)>,
}

/// One edge of a net's airwire tree: (from, to), as the score's tree gives it.
type Edge = objectives::TreeEdge;

/// Every part's centre and quarter turns.
type Placement = (Vec<(f64, f64)>, Vec<u8>);

/// Every part's box centre and size, as a legality test reads them.
type Boxes<'s> = (&'s [(f64, f64)], &'s [(f64, f64)]);

/// A placement and the caches that make a move's cost change cheap.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct State {
    pub centres: Vec<(f64, f64)>,
    /// Quarter turns from the rotation read, 0 to 3.
    pub turns: Vec<u8>,
    /// Each part's courtyard box size at its turns.
    sizes: Vec<(f64, f64)>,
    /// Each net's airwire length, tree edges and the box of its edges.
    net_mm: Vec<f64>,
    net_edges: Vec<Vec<Edge>>,
    net_box: Vec<Rect>,
    crossings: usize,
    /// Each measured pair's pad-to-pad distance.
    pair_mm: Vec<f64>,
    /// Each connector's box centre distance to the outline's edge, negative off the board.
    connector_mm: Vec<f64>,
    /// Each part's mean centre distance to its schematic neighbours; 0 without any.
    neighbour_mm: Vec<f64>,
    label_conflicts: usize,
    cost: f64,
}

impl State {
    pub(crate) fn cost(&self) -> f64 {
        self.cost
    }
}

/// One proposed move: the new centre and turns of every part it names (one part, the two
/// of a swap, or a capacitor and the parts it displaces).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Move {
    pub parts: Vec<(usize, (f64, f64), u8)>,
}

/// The cost change of a legal move and what `apply` needs to make it without computing
/// it again.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Delta {
    pub cost: f64,
    /// The change of the soft cost (`Terms::soft`): `cost` without the requirement terms
    /// of the pairs and connectors (no R and no excess slope). Summed on its own, beside
    /// `cost`, so that it does not depend on R or the slope to the bit; the first
    /// temperature reads it (amendment A2), annealing and descent read `cost`.
    soft: f64,
    nets: Vec<(usize, NetTree)>,
    crossings: i64,
    pairs: Vec<(usize, f64)>,
    connectors: Vec<(usize, f64)>,
    neighbours: Vec<(usize, f64)>,
    labels: i64,
}

/// Rounds to 1 µm: `placements_from`'s rounding of an anchor, to the same bit.
fn round_um(value: f64) -> f64 {
    (value * 1e3).round() / 1e3
}

/// KiCad's spelling of an angle, in (-180, 180]: `placements_from`'s and the writer's.
fn normalised_angle(degrees: f64) -> f64 {
    let wrapped = degrees.rem_euclid(360.0);
    if wrapped > 180.0 {
        wrapped - 360.0
    } else {
        wrapped
    }
}

/// The rotation a board written by `placements_applied_to` gives back when it is read:
/// the angle normalised and written to 6 decimals (`(at x y)` without one reads as 0).
fn written_angle(degrees: f64) -> f64 {
    let normal = normalised_angle(degrees);
    let read: f64 = format!("{normal:.6}").parse().unwrap_or(normal);
    if read == 0.0 {
        0.0
    } else {
        read
    }
}

/// cos and sin of `degrees`, by the arithmetic of `konnect_sexp::geometry::rotate_about`.
fn turn_of(degrees: f64) -> (f64, f64) {
    let theta = degrees * PI / 180.0;
    (theta.cos(), theta.sin())
}

/// The quarter turns from `from_deg` to `to_deg`, 0 to 3.
fn quarter_turns_between(from_deg: f64, to_deg: f64) -> u8 {
    (((to_deg - from_deg) / 90.0).round() as i64).rem_euclid(4) as u8
}

/// Whether `degrees` is a whole number of quarter turns.
fn on_quarter_turns(degrees: f64) -> bool {
    let quarters = degrees / 90.0;
    (quarters - quarters.round()).abs() < 1e-9
}

/// A rect turned by `turns` quarter turns about the origin: exact, a rect again.
fn turned_rect((x0, y0, x1, y1): Rect, turns: u8) -> Rect {
    let corners = [(x0, y0), (x1, y1)].map(|corner| turn_vector(corner, i32::from(turns)));
    (
        corners[0].0.min(corners[1].0),
        corners[0].1.min(corners[1].1),
        corners[0].0.max(corners[1].0),
        corners[0].1.max(corners[1].1),
    )
}

fn shifted((x0, y0, x1, y1): Rect, (dx, dy): (f64, f64)) -> Rect {
    (x0 + dx, y0 + dy, x1 + dx, y1 + dy)
}

fn union((ax0, ay0, ax1, ay1): Rect, (bx0, by0, bx1, by1): Rect) -> Rect {
    (ax0.min(bx0), ay0.min(by0), ax1.max(bx1), ay1.max(by1))
}

/// Whether two boxes overlap by more than nothing on both axes.
fn overlap(a: Rect, b: Rect) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

/// Whether two boxes touch or overlap: when they do not, no edge in one crosses an edge
/// in the other by the score's test (its box test is the same, strict).
fn boxes_meet(a: Rect, b: Rect) -> bool {
    !(a.0 > b.2 || b.0 > a.2 || a.1 > b.3 || b.1 > a.3)
}

/// The box with no point, which meets nothing.
const NO_BOX: Rect = (
    f64::INFINITY,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::NEG_INFINITY,
);

/// The box of a tree's edges.
fn edges_box(edges: &[Edge]) -> Rect {
    edges.iter().fold(NO_BOX, |bbox, &(a, b)| {
        union(
            bbox,
            (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)),
        )
    })
}

/// The score's crossings between the edges of two different nets, `lo` the net that
/// comes first in net order (the score passes its edge first).
fn crossings_between(lo: &[Edge], lo_box: Rect, hi: &[Edge], hi_box: Rect) -> i64 {
    if !boxes_meet(lo_box, hi_box) {
        return 0;
    }
    let mut count = 0;
    for &(a, b) in lo {
        for &(c, d) in hi {
            if objectives::crosses(a, b, c, d).expect("the pad sites are finite") {
                count += 1;
            }
        }
    }
    count
}

/// Whether a text is hidden: `(hide yes)` or a bare `hide` among its own children, or
/// either in its `effects`. `(hide no)` is shown.
fn is_hidden(item: &SexpNode) -> bool {
    fn says_hide(node: &SexpNode, skip: usize) -> bool {
        node.children()
            .unwrap_or(&[])
            .iter()
            .skip(skip)
            .any(|child| match child {
                SexpNode::Atom(word) => word == "hide",
                SexpNode::List(_) => {
                    child.head() == Some("hide")
                        && child.get(1).and_then(SexpNode::as_str) != Some("no")
                }
                SexpNode::Str(_) => false,
            })
    }
    // The head, then a property's name and value, or an `fp_text`'s kind and text.
    says_hide(item, 3)
        || item
            .find("effects")
            .is_some_and(|effects| says_hide(effects, 1))
}

/// The label of one footprint text `item` (a `property` or an `fp_text`), or `None` when
/// it is not a visible text on a silkscreen layer. Its centre is its `(at x y)` taken
/// through the footprint's transform; its box is `text_box`'s stroke-font estimate for
/// its text (`${REFERENCE}` spelled as `reference`) laid along its stored angle; the box
/// is kept relative to `centre`, the part's courtyard centre as read.
fn label_of(
    item: &SexpNode,
    reference: &str,
    (root, rotation): ((f64, f64), f64),
    centre: (f64, f64),
) -> Option<Label> {
    let top = match item.find_str("layer")? {
        "F.SilkS" => true,
        "B.SilkS" => false,
        _ => return None,
    };
    if is_hidden(item) {
        return None;
    }
    let text = item.get(2)?.as_str()?.replace("${REFERENCE}", reference);
    if text.is_empty() {
        return None;
    }
    let at = item.find("at")?;
    let (x, y) = (at.get_f64(1)?, at.get_f64(2)?);
    let angle = at.get_f64(3).unwrap_or(0.0);
    if !(x.is_finite() && y.is_finite() && angle.is_finite()) {
        return None;
    }
    let (ax, ay) = rotate_about(x, y, root.0, root.1, rotation);
    // `text_box` reads a board text: the same text, effects and layer at the origin.
    let mut estimate = vec![
        SexpNode::Atom("gr_text".to_string()),
        SexpNode::Str(text),
        SexpNode::List(vec![
            SexpNode::Atom("at".to_string()),
            SexpNode::Atom("0".to_string()),
            SexpNode::Atom("0".to_string()),
            SexpNode::Atom(format!("{angle}")),
        ]),
    ];
    estimate.extend(item.find("effects").cloned());
    estimate.extend(item.find("layer").cloned());
    let bbox = text_box(&SexpNode::List(estimate))?;
    let base = shifted(bbox, (ax - centre.0, ay - centre.1));
    Some(Label {
        boxes: [0, 1, 2, 3].map(|turns| turned_rect(base, turns)),
        top,
    })
}

/// What `build` reads from the footprints of the board text.
struct Footprints {
    frames: Vec<Frame>,
    pads: Vec<Vec<(f64, f64)>>,
    /// Each indexed pad by (part, pad number, the bits of its board position): the
    /// key the index's pad sites are matched by.
    sites: BTreeMap<(usize, String, u64, u64), Vec<usize>>,
    /// Each indexed pad by (part, pad number): a pair's pad must be found once.
    numbers: BTreeMap<(usize, String), Vec<usize>>,
    labels: Vec<Vec<Label>>,
    connector: Vec<bool>,
}

/// The footprints of `tree` (the model's own text) matched to the model's parts, as
/// `board_model` matches them: in file order, by `place_held::footprint_keys`.
fn read_footprints(
    model: &BoardModel,
    tree: &SexpNode,
    read_centres: &[(f64, f64)],
) -> Result<Footprints, GenerateFailure> {
    let invalid = |what: &str| GenerateFailure::Invalid(what.to_string());
    let count = model.parts.len();
    let nodes = footprints(tree);
    let scan = footprint_courtyards(tree);
    if nodes.len() != count || scan.items.len() != count || scan.skipped != 0 {
        return Err(invalid("the board text's footprints are not the model's"));
    }
    let raw: Vec<Option<&str>> = scan.items.iter().map(|c| c.reference.as_deref()).collect();
    let keys = footprint_keys(&raw);
    let mut frames: Vec<Option<Frame>> = vec![None; count];
    let mut read = Footprints {
        frames: Vec::new(),
        pads: vec![Vec::new(); count],
        sites: BTreeMap::new(),
        numbers: BTreeMap::new(),
        labels: vec![Vec::new(); count],
        connector: vec![false; count],
    };
    for (n, node) in nodes.iter().enumerate() {
        let key = keys[n]
            .as_deref()
            .ok_or_else(|| invalid("a footprint has no reference"))?;
        let p = model
            .references
            .binary_search_by(|r| r.as_str().cmp(key))
            .map_err(|_| invalid("a footprint is not one of the model's parts"))?;
        if frames[p].is_some() {
            return Err(invalid("two footprints are one part of the model"));
        }
        let at = node
            .find("at")
            .ok_or_else(|| invalid("a footprint has no anchor"))?;
        let (fx, fy) = (at.get_f64(1), at.get_f64(2));
        let rotation = at.get_f64(3).unwrap_or(0.0);
        let (Some(fx), Some(fy)) = (fx, fy) else {
            return Err(invalid("a footprint's anchor cannot be read"));
        };
        if !(fx.is_finite() && fy.is_finite() && rotation.is_finite()) {
            return Err(invalid("a footprint's anchor is not finite"));
        }
        let part = &model.parts[p];
        frames[p] = Some(Frame {
            anchor: (fx, fy),
            offset: (part.at.0 - read_centres[p].0, part.at.1 - read_centres[p].1),
            read_turn: turn_of(rotation),
            turned: [0u8, 1, 2, 3]
                .map(|q| turn_of(written_angle(part.rotation_deg + 90.0 * f64::from(q)))),
        });
        let reference = raw[n].unwrap_or(key);
        read.connector[p] = raw[n].is_some_and(|r| ref_prefix(r) == "J");
        // The index places the pads of a footprint with a reference and a side, and of
        // those the pads with a number and a position.
        let indexed = raw[n].is_some() && matches!(node.find_str("layer"), Some("F.Cu" | "B.Cu"));
        if indexed {
            for pad in node.find_all("pad") {
                let Some(number) = pad.get(1).and_then(SexpNode::as_str) else {
                    continue;
                };
                let Some(local) = pad.find("at").and_then(|at| {
                    let (x, y) = (at.get_f64(1)?, at.get_f64(2)?);
                    (x.is_finite() && y.is_finite()).then_some((x, y))
                }) else {
                    continue;
                };
                let board = transform_pad(local.0, local.1, fx, fy, rotation);
                let slot = read.pads[p].len();
                read.pads[p].push(local);
                read.sites
                    .entry((p, number.to_string(), board.0.to_bits(), board.1.to_bits()))
                    .or_default()
                    .push(slot);
                read.numbers
                    .entry((p, number.to_string()))
                    .or_default()
                    .push(slot);
            }
        }
        for item in node.children().unwrap_or(&[]) {
            if matches!(item.head(), Some("property" | "fp_text")) {
                read.labels[p].extend(label_of(
                    item,
                    reference,
                    ((fx, fy), rotation),
                    read_centres[p],
                ));
            }
        }
    }
    read.frames = frames
        .into_iter()
        .map(|frame| frame.ok_or_else(|| invalid("a part of the model has no footprint")))
        .collect::<Result<_, _>>()?;
    Ok(read)
}

impl<'a> CostModel<'a> {
    /// Reads what the cost needs from `model` (its text is parsed once more for the pad
    /// sites, the pour nets and the silk labels) and from `hoods`:
    /// - nets: the score's own filter (`objectives`: at least 2 pad sites, 2 to
    ///   `fanout_limit` distinct references) at weight 1; a net above the limit at
    ///   `rail_weight` unless a zone on the board carries its name, or the weight is 0;
    /// - pairs: `model.intent.pairs`; a pair whose two pads cannot each be found once is
    ///   not measured: it counts as a violation that no move changes, as the screen
    ///   counts it, and adds the requirement cost to every state alike;
    /// - connectors: every part whose reference prefix is `J` (the score's rule); a held
    ///   one's term is a constant;
    /// - labels: every visible text of a footprint on a silkscreen layer (`F.SilkS` or
    ///   `B.SilkS`): its `property` entries and its `fp_text` entries, a text with
    ///   `(hide yes)`, a bare `hide`, or `hide` in its `effects` left out. A label's
    ///   centre is its `(at x y)` taken through the footprint's transform
    ///   (`konnect_sexp::geometry::rotate_about`, as a pad's is); its box is the
    ///   stroke-font estimate `place_generators::text_box` makes for copper text (the
    ///   reference's own length for `${REFERENCE}`), laid along the text's stored angle,
    ///   which is absolute, and its hull at an angle that is not a quarter turn. The box
    ///   is kept relative to the part's centre and turns with the part;
    /// - hoods: `hoods.neighbours` by part index; a name that is not on the board is
    ///   `Invalid`.
    ///
    /// Errs with `Invalid` when no part may move or a parameter is refused.
    /// The requirement cost is `params.requirement_mm` until `with_requirement` sets it.
    pub(crate) fn build(
        model: &'a BoardModel,
        hoods: Option<&Neighbourhoods>,
        params: &ConvergeParams,
    ) -> Result<Self, GenerateFailure> {
        params.validate()?;
        let invalid = |what: String| GenerateFailure::Invalid(what);
        let count = model.parts.len();
        let movable: Vec<usize> = (0..count).filter(|&i| model.movable[i]).collect();
        if movable.is_empty() {
            return Err(invalid("no part may move".to_string()));
        }
        let read_centres: Vec<(f64, f64)> = model
            .parts
            .iter()
            .map(|p| ((p.bbox.0 + p.bbox.2) / 2.0, (p.bbox.1 + p.bbox.3) / 2.0))
            .collect();
        let sizes: Vec<(f64, f64)> = model
            .parts
            .iter()
            .map(|p| (p.bbox.2 - p.bbox.0, p.bbox.3 - p.bbox.1))
            .collect();
        let may_turn: Vec<bool> = (0..count)
            .map(|i| {
                params.turn_parts
                    && model.movable[i]
                    && on_quarter_turns(model.parts[i].rotation_deg)
            })
            .collect();
        let tree = konnect_sexp::parse_sexp(&model.text)
            .map_err(|error| invalid(format!("the board text does not parse: {error}")))?;
        let read = read_footprints(model, &tree, &read_centres)?;
        let find = |reference: &str| {
            model
                .references
                .binary_search_by(|r| r.as_str().cmp(reference))
                .ok()
        };

        // The nets, in the score's order, each site matched to its pad.
        let pours: BTreeSet<String> = zones(&tree)
            .items
            .into_iter()
            .filter_map(|zone| zone.net)
            .collect();
        let mut sites = read.sites;
        let mut nets = Vec::new();
        for name in model.index.nets() {
            let pads = model.index.pads_of_net(name);
            let references = pads
                .iter()
                .map(|pad| pad.reference.as_str())
                .collect::<BTreeSet<_>>()
                .len();
            let eligible = objectives::counts_net(pads.len(), references, params.fanout_limit);
            let rail = !eligible && pads.len() >= 2 && references > params.fanout_limit;
            let weight = if eligible {
                1.0
            } else if rail && params.rail_weight > 0.0 && !pours.contains(name) {
                params.rail_weight
            } else {
                continue;
            };
            let mut members = Vec::with_capacity(pads.len());
            let mut names = Vec::with_capacity(pads.len());
            for pad in pads {
                let slot = find(&pad.reference).and_then(|p| {
                    let key = (
                        p,
                        pad.pad_number.clone(),
                        pad.at.0.to_bits(),
                        pad.at.1.to_bits(),
                    );
                    let slots = sites.get_mut(&key)?;
                    (!slots.is_empty()).then(|| (p, slots.remove(0)))
                });
                let slot = slot.ok_or_else(|| {
                    invalid(format!(
                        "pad {} of {} is not found on its footprint",
                        pad.pad_number, pad.reference
                    ))
                })?;
                members.push(slot);
                names.push((pad.reference.as_str(), pad.pad_number.as_str()));
            }
            nets.push(Net {
                weight,
                eligible,
                sites: members,
                names,
            });
        }
        let eligible: Vec<usize> = (0..nets.len()).filter(|&n| nets[n].eligible).collect();
        let mut part_nets: Vec<Vec<usize>> = vec![Vec::new(); count];
        for (n, net) in nets.iter().enumerate() {
            let parts: BTreeSet<usize> = net.sites.iter().map(|&(p, _)| p).collect();
            for p in parts {
                part_nets[p].push(n);
            }
        }

        // The pairs: each pad found once on the board, as the screen finds it.
        let once = |reference: &str, number: &str| {
            let p = find(reference)?;
            match read.numbers.get(&(p, number.to_string())) {
                Some(slots) if slots.len() == 1 => Some((p, slots[0])),
                _ => None,
            }
        };
        let mut pairs = Vec::new();
        let mut unmeasured_pairs = 0;
        let mut part_pairs: Vec<Vec<usize>> = vec![Vec::new(); count];
        for pair in &model.intent.pairs {
            match (once(&pair.cap, &pair.cap_pad), once(&pair.ic, &pair.ic_pad)) {
                (Some(cap), Some(ic)) => {
                    let k = pairs.len();
                    pairs.push(Pair {
                        cap,
                        ic,
                        limit_mm: pair.limit_mm,
                    });
                    part_pairs[cap.0].push(k);
                    if ic.0 != cap.0 {
                        part_pairs[ic.0].push(k);
                    }
                }
                _ => unmeasured_pairs += 1,
            }
        }

        let connectors: Vec<usize> = (0..count).filter(|&p| read.connector[p]).collect();
        let mut part_connector = vec![None; count];
        for (c, &p) in connectors.iter().enumerate() {
            part_connector[p] = Some(c);
        }

        let mut hood_lists: Vec<Vec<usize>> = vec![Vec::new(); count];
        let mut hood_of: Vec<Vec<usize>> = vec![Vec::new(); count];
        if let Some(hoods) = hoods {
            for (name, near) in &hoods.neighbours {
                let p = find(name).ok_or_else(|| {
                    invalid(format!("the schematic part {name} is not on the board"))
                })?;
                hood_lists[p] = near
                    .iter()
                    .map(|other| {
                        find(other).ok_or_else(|| {
                            invalid(format!(
                                "the schematic neighbour {other} is not on the board"
                            ))
                        })
                    })
                    .collect::<Result<_, _>>()?;
            }
            for (p, near) in hood_lists.iter().enumerate() {
                for &q in near {
                    if !hood_of[q].contains(&p) {
                        hood_of[q].push(p);
                    }
                }
            }
        }

        let extents: Vec<[Rect; 4]> = (0..count)
            .map(|p| {
                let (w, h) = sizes[p];
                let court = (-w / 2.0, -h / 2.0, w / 2.0, h / 2.0);
                [0u8, 1, 2, 3].map(|q| {
                    read.labels[p]
                        .iter()
                        .fold(turned_rect(court, q), |extent, label| {
                            union(extent, label.boxes[usize::from(q)])
                        })
                })
            })
            .collect();

        Ok(CostModel {
            model,
            params: *params,
            legality: Legality::new(model, GapRule::Everywhere),
            requirement_mm: params.requirement_mm,
            movable,
            may_turn,
            sizes,
            read_centres,
            frames: read.frames,
            pads: read.pads,
            labels: read.labels,
            extents,
            nets,
            eligible,
            part_nets,
            pairs,
            part_pairs,
            unmeasured_pairs,
            connectors,
            part_connector,
            hoods: hood_lists,
            hood_of,
            has_hoods: hoods.is_some(),
            repairs: Cell::new((0, 0)),
        })
    }

    /// The same model with `mm` as the cost of a broken requirement (`converge` gives
    /// the larger of the parameter and the start's soft cost). A state's cost is cached,
    /// so a state of the model before is built again with `state`.
    pub(crate) fn with_requirement(self, mm: f64) -> Self {
        CostModel {
            requirement_mm: mm,
            ..self
        }
    }

    /// Part `p`'s courtyard box size at `turns`.
    fn size_at(&self, p: usize, turns: u8) -> (f64, f64) {
        let (w, h) = self.sizes[p];
        if turns.is_multiple_of(2) {
            (w, h)
        } else {
            (h, w)
        }
    }

    /// The anchor and the rotation (cos, sin) part `p` has on the written board at
    /// `centre` and `turns`: as read when it is where and as it was read (it is not
    /// written), else `placements_from`'s anchor and the written rotation.
    fn frame_at(&self, p: usize, centre: (f64, f64), turns: u8) -> ((f64, f64), (f64, f64)) {
        let frame = &self.frames[p];
        if turns == 0 && centre == self.read_centres[p] {
            (frame.anchor, frame.read_turn)
        } else {
            let offset = turn_vector(frame.offset, i32::from(turns));
            (
                (round_um(centre.0 + offset.0), round_um(centre.1 + offset.1)),
                frame.turned[usize::from(turns)],
            )
        }
    }

    /// Pad `slot` of part `p` at `centre` and `turns`, by `rotate_about`'s arithmetic.
    fn pad_at(&self, p: usize, slot: usize, centre: (f64, f64), turns: u8) -> (f64, f64) {
        let ((ax, ay), (cos, sin)) = self.frame_at(p, centre, turns);
        let (dx, dy) = self.pads[p][slot];
        (ax + dx * cos + dy * sin, ay - dx * sin + dy * cos)
    }

    /// Net `n`'s tree with each part where `place` says.
    fn tree(&self, n: usize, place: &impl Fn(usize) -> ((f64, f64), u8)) -> NetTree {
        let net = &self.nets[n];
        let sites: Vec<Site<'a>> = net
            .sites
            .iter()
            .zip(&net.names)
            .map(|(&(p, slot), &(reference, pad))| {
                let (centre, turns) = place(p);
                Site {
                    reference,
                    pad,
                    at: self.pad_at(p, slot, centre, turns),
                }
            })
            .collect();
        let (mm, edges) = objectives::spanning_tree(&sites).expect("the pad sites are finite");
        NetTree {
            mm,
            bbox: edges_box(&edges),
            edges,
        }
    }

    /// The crossings between nets `a` and `b` (different), with the edges and boxes given.
    fn crossings_of(
        &self,
        (a, a_edges, a_box): (usize, &[Edge], Rect),
        (b, b_edges, b_box): (usize, &[Edge], Rect),
    ) -> i64 {
        if a < b {
            crossings_between(a_edges, a_box, b_edges, b_box)
        } else {
            crossings_between(b_edges, b_box, a_edges, a_box)
        }
    }

    /// Pair `k`'s pad-to-pad distance with each part where `place` says, as the screen
    /// measures it.
    fn pair_distance(&self, k: usize, place: &impl Fn(usize) -> ((f64, f64), u8)) -> f64 {
        let pair = &self.pairs[k];
        let at = |(p, slot): (usize, usize)| {
            let (centre, turns) = place(p);
            self.pad_at(p, slot, centre, turns)
        };
        let (cap, ic) = (at(pair.cap), at(pair.ic));
        (cap.0 - ic.0).hypot(cap.1 - ic.1)
    }

    /// What a pair at `d` costs: the distance, and the requirement past the limit.
    fn pair_cost(&self, k: usize, d: f64) -> f64 {
        let limit = self.pairs[k].limit_mm;
        d + if d > limit {
            self.requirement_mm + self.params.requirement_slope * (d - limit)
        } else {
            0.0
        }
    }

    /// The score's distance of a connector's box centre to the outline's edge (a
    /// cut-out's edge counts), negative off the board.
    fn edge_distance(&self, centre: (f64, f64)) -> f64 {
        let outline = &self.model.outline;
        if outline.contains(centre) {
            outline.boundary_distance(centre)
        } else {
            -outline.boundary_distance(centre)
        }
    }

    /// What a connector at `distance` from the edge costs.
    fn connector_cost(&self, distance: f64) -> f64 {
        if distance > CONNECTOR_EDGE_LIMIT_MM {
            self.requirement_mm
                + self.params.requirement_slope * (distance - CONNECTOR_EDGE_LIMIT_MM)
        } else {
            0.0
        }
    }

    /// Part `p`'s mean centre distance to its schematic neighbours, as
    /// `place_schematic::part_distances` sums it; 0 without any.
    fn neighbour_distance(&self, p: usize, place: &impl Fn(usize) -> ((f64, f64), u8)) -> f64 {
        let near = &self.hoods[p];
        if near.is_empty() {
            return 0.0;
        }
        let from = place(p).0;
        let mut sum = 0.0;
        for &q in near {
            let to = place(q).0;
            sum += (from.0 - to.0).hypot(from.1 - to.1);
        }
        sum / near.len() as f64
    }

    /// The label conflicts between parts `a` and `b` (different) where they are given: a
    /// visible label of one over the other's courtyard box on its own silk side, and a
    /// label of one over a label of the other on the same silk side, each pair of labels
    /// once. Parts that share no side have none.
    fn label_conflicts_between(
        &self,
        (a, at_a, turns_a): (usize, (f64, f64), u8),
        (b, at_b, turns_b): (usize, (f64, f64), u8),
    ) -> usize {
        let sides = &self.model.part_sides;
        if (self.labels[a].is_empty() && self.labels[b].is_empty())
            || !sides[a].intersects(sides[b])
        {
            return 0;
        }
        let (qa, qb) = (usize::from(turns_a), usize::from(turns_b));
        if !overlap(
            shifted(self.extents[a][qa], at_a),
            shifted(self.extents[b][qb], at_b),
        ) {
            return 0;
        }
        let court = |p: usize, at: (f64, f64), turns: u8| {
            let (w, h) = self.size_at(p, turns);
            (
                at.0 - w / 2.0,
                at.1 - h / 2.0,
                at.0 + w / 2.0,
                at.1 + h / 2.0,
            )
        };
        let on_side = |label: &Label, p: usize| {
            if label.top {
                sides[p].top
            } else {
                sides[p].bottom
            }
        };
        let (court_a, court_b) = (court(a, at_a, turns_a), court(b, at_b, turns_b));
        let mut count = 0;
        for label in &self.labels[a] {
            let over = shifted(label.boxes[qa], at_a);
            if on_side(label, b) && overlap(over, court_b) {
                count += 1;
            }
            for other in &self.labels[b] {
                if label.top == other.top && overlap(over, shifted(other.boxes[qb], at_b)) {
                    count += 1;
                }
            }
        }
        for label in &self.labels[b] {
            if on_side(label, a) && overlap(shifted(label.boxes[qb], at_b), court_a) {
                count += 1;
            }
        }
        count
    }

    /// The state of a placement, every cache computed from nothing. `apply` keeps the
    /// caches equal to this (the test `a_states_caches_equal_a_full_evaluation`).
    pub(crate) fn state(&self, centres: Vec<(f64, f64)>, turns: Vec<u8>) -> State {
        let count = self.model.parts.len();
        assert!(
            centres.len() == count && turns.len() == count,
            "a state has one centre and one turn per part"
        );
        let place = |p: usize| (centres[p], turns[p]);
        let trees: Vec<NetTree> = (0..self.nets.len()).map(|n| self.tree(n, &place)).collect();
        let mut crossings = 0;
        for (k, &a) in self.eligible.iter().enumerate() {
            for &b in &self.eligible[k + 1..] {
                crossings += crossings_between(
                    &trees[a].edges,
                    trees[a].bbox,
                    &trees[b].edges,
                    trees[b].bbox,
                );
            }
        }
        let mut label_conflicts = 0;
        for a in 0..count {
            for b in a + 1..count {
                label_conflicts += self
                    .label_conflicts_between((a, centres[a], turns[a]), (b, centres[b], turns[b]));
            }
        }
        let mut state = State {
            sizes: (0..count).map(|p| self.size_at(p, turns[p])).collect(),
            net_mm: trees.iter().map(|tree| tree.mm).collect(),
            net_box: trees.iter().map(|tree| tree.bbox).collect(),
            net_edges: trees.into_iter().map(|tree| tree.edges).collect(),
            crossings: usize::try_from(crossings).expect("a count is at least 0"),
            pair_mm: (0..self.pairs.len())
                .map(|k| self.pair_distance(k, &place))
                .collect(),
            connector_mm: self
                .connectors
                .iter()
                .map(|&p| self.edge_distance(centres[p]))
                .collect(),
            neighbour_mm: (0..count)
                .map(|p| self.neighbour_distance(p, &place))
                .collect(),
            label_conflicts,
            cost: 0.0,
            centres,
            turns,
        };
        state.cost = self.terms(&state).cost;
        state
    }

    /// The terms of `state`, for the trace and the record.
    pub(crate) fn terms(&self, state: &State) -> Terms {
        let params = &self.params;
        let (mut airwire_mm, mut rail_airwire_mm) = (0.0, 0.0);
        for (n, net) in self.nets.iter().enumerate() {
            if net.eligible {
                airwire_mm += state.net_mm[n];
            } else {
                rail_airwire_mm += state.net_mm[n];
            }
        }
        // The requirement terms: an unmeasured pair costs the requirement in every state.
        let mut requirement = self.unmeasured_pairs as f64 * self.requirement_mm;
        let mut pair_violations = self.unmeasured_pairs;
        let mut pair_mm = 0.0;
        for (k, &d) in state.pair_mm.iter().enumerate() {
            pair_mm += d;
            if d > self.pairs[k].limit_mm {
                pair_violations += 1;
            }
            requirement += self.pair_cost(k, d) - d;
        }
        let mut connector_violations = 0;
        for &distance in &state.connector_mm {
            if distance > CONNECTOR_EDGE_LIMIT_MM {
                connector_violations += 1;
            }
            requirement += self.connector_cost(distance);
        }
        let neighbour_mm = self
            .has_hoods
            .then(|| state.neighbour_mm.iter().sum::<f64>());
        let soft = airwire_mm
            + params.rail_weight * rail_airwire_mm
            + params.crossing_mm * state.crossings as f64
            + pair_mm
            + params.neighbour_weight * neighbour_mm.unwrap_or(0.0)
            + params.label_mm * state.label_conflicts as f64;
        Terms {
            cost: soft + requirement,
            airwire_mm,
            rail_airwire_mm,
            crossings: state.crossings,
            pair_violations,
            pair_mm,
            connector_violations,
            neighbour_mm,
            label_conflicts: state.label_conflicts,
            soft,
        }
    }

    /// Whether `mv` is legal: every part it names is legal at its new centre and turns
    /// against every other part where it would be (`Legality::legal_at`, with
    /// `GapRule::Everywhere`), and the two parts of a swap against each other.
    ///
    /// A move is also illegal when it names no part, a part twice, a held part (held
    /// parts never move), a centre that is not finite, turns above 3, or new turns for a
    /// part that may not turn.
    pub(crate) fn legal(&self, state: &State, mv: &Move) -> bool {
        let count = self.model.parts.len();
        if mv.parts.is_empty() {
            return false;
        }
        for (k, &(p, at, turns)) in mv.parts.iter().enumerate() {
            if p >= count
                || !self.model.movable[p]
                || turns > 3
                || !(at.0.is_finite() && at.1.is_finite())
                || (turns != state.turns[p] && !self.may_turn[p])
                || mv.parts[..k].iter().any(|&(other, ..)| other == p)
            {
                return false;
            }
        }
        let named = |j: usize| mv.parts.iter().any(|&(p, ..)| p == j);
        for (k, &(p, at, turns)) in mv.parts.iter().enumerate() {
            let size = self.size_at(p, turns);
            let others = (0..count).filter(|&j| j != p && !named(j));
            if !self
                .legality
                .legal_at(p, at, size, others, &state.centres, &state.sizes)
            {
                return false;
            }
            // The parts the move names, against each other where they would be.
            for &(other, at_other, turns_other) in &mv.parts[k + 1..] {
                let size_other = self.size_at(other, turns_other);
                if !self
                    .legality
                    .clear_of(p, at, size, other, at_other, size_other)
                {
                    return false;
                }
            }
        }
        true
    }

    /// The cost change of the legal move `mv`: the airwire trees of the nets of its parts
    /// rebuilt, their crossings counted again against every other eligible net and among
    /// themselves, the pairs and connectors of its parts, the neighbour distances of its
    /// parts and of the parts that list them, and the label conflicts its parts take
    /// part in. Reads `state`, changes nothing.
    pub(crate) fn delta(&self, state: &State, mv: &Move) -> Delta {
        let params = &self.params;
        let place = |p: usize| {
            mv.parts
                .iter()
                .find(|&&(q, ..)| q == p)
                .map_or((state.centres[p], state.turns[p]), |&(_, at, turns)| {
                    (at, turns)
                })
        };
        // `soft` takes every change `cost` takes but the requirement terms.
        let (mut cost, mut soft) = (0.0, 0.0);

        // The nets of the moved parts, each rebuilt once.
        let mut changed: Vec<usize> = mv
            .parts
            .iter()
            .flat_map(|&(p, ..)| self.part_nets[p].iter().copied())
            .collect();
        changed.sort_unstable();
        changed.dedup();
        let trees: Vec<(usize, NetTree)> =
            changed.iter().map(|&n| (n, self.tree(n, &place))).collect();
        for (n, tree) in &trees {
            let change = self.nets[*n].weight * (tree.mm - state.net_mm[*n]);
            cost += change;
            soft += change;
        }

        // Their crossings: against every eligible net that did not change, and among
        // themselves, each pair of nets once.
        let old = |n: usize| (n, state.net_edges[n].as_slice(), state.net_box[n]);
        let mut crossings = 0i64;
        for (k, (n, tree)) in trees.iter().enumerate() {
            let n = *n;
            if !self.nets[n].eligible {
                continue;
            }
            let new = (n, tree.edges.as_slice(), tree.bbox);
            for &m in &self.eligible {
                if m == n || changed.binary_search(&m).is_ok() {
                    continue;
                }
                crossings += self.crossings_of(new, old(m)) - self.crossings_of(old(n), old(m));
            }
            for (m, other) in &trees[k + 1..] {
                let m = *m;
                if !self.nets[m].eligible {
                    continue;
                }
                crossings += self.crossings_of(new, (m, other.edges.as_slice(), other.bbox))
                    - self.crossings_of(old(n), old(m));
            }
        }
        let crossing = params.crossing_mm * crossings as f64;
        cost += crossing;
        soft += crossing;

        let mut touched: Vec<usize> = mv
            .parts
            .iter()
            .flat_map(|&(p, ..)| self.part_pairs[p].iter().copied())
            .collect();
        touched.sort_unstable();
        touched.dedup();
        let pairs: Vec<(usize, f64)> = touched
            .into_iter()
            .map(|k| (k, self.pair_distance(k, &place)))
            .collect();
        for &(k, d) in &pairs {
            cost += self.pair_cost(k, d) - self.pair_cost(k, state.pair_mm[k]);
            // The pair's plain distance is soft; its requirement past the limit is not.
            soft += d - state.pair_mm[k];
        }

        let connectors: Vec<(usize, f64)> = mv
            .parts
            .iter()
            .filter_map(|&(p, at, _)| self.part_connector[p].map(|c| (c, self.edge_distance(at))))
            .collect();
        for &(c, distance) in &connectors {
            cost += self.connector_cost(distance) - self.connector_cost(state.connector_mm[c]);
        }

        // The moved parts that have neighbours, and the parts that list a moved part.
        let mut near: Vec<usize> = mv
            .parts
            .iter()
            .flat_map(|&(p, ..)| {
                (!self.hoods[p].is_empty())
                    .then_some(p)
                    .into_iter()
                    .chain(self.hood_of[p].iter().copied())
            })
            .collect();
        near.sort_unstable();
        near.dedup();
        let neighbours: Vec<(usize, f64)> = near
            .into_iter()
            .map(|p| (p, self.neighbour_distance(p, &place)))
            .collect();
        for &(p, mm) in &neighbours {
            let change = params.neighbour_weight * (mm - state.neighbour_mm[p]);
            cost += change;
            soft += change;
        }

        // The label conflicts of every pair of parts with a moved part in it, once: a pair
        // of two moved parts is counted at the first of them in the move.
        let count = self.model.parts.len();
        let mut labels = 0i64;
        for (k, &(p, at, turns)) in mv.parts.iter().enumerate() {
            for o in 0..count {
                if o == p || mv.parts[..k].iter().any(|&(earlier, ..)| earlier == o) {
                    continue;
                }
                let (at_o, turns_o) = place(o);
                let after = self.label_conflicts_between((p, at, turns), (o, at_o, turns_o));
                let before = self.label_conflicts_between(
                    (p, state.centres[p], state.turns[p]),
                    (o, state.centres[o], state.turns[o]),
                );
                labels += after as i64 - before as i64;
            }
        }
        let label = params.label_mm * labels as f64;
        cost += label;
        soft += label;

        Delta {
            cost,
            soft,
            nets: trees,
            crossings,
            pairs,
            connectors,
            neighbours,
            labels,
        }
    }

    /// Makes the move.
    pub(crate) fn apply(&self, state: &mut State, mv: &Move, delta: Delta) {
        for &(p, at, turns) in &mv.parts {
            state.centres[p] = at;
            state.turns[p] = turns;
            state.sizes[p] = self.size_at(p, turns);
        }
        for (n, tree) in delta.nets {
            state.net_mm[n] = tree.mm;
            state.net_edges[n] = tree.edges;
            state.net_box[n] = tree.bbox;
        }
        state.crossings = usize::try_from(state.crossings as i64 + delta.crossings)
            .expect("the crossings stay at 0 or above");
        for (k, d) in delta.pairs {
            state.pair_mm[k] = d;
        }
        for (c, distance) in delta.connectors {
            state.connector_mm[c] = distance;
        }
        for (p, mm) in delta.neighbours {
            state.neighbour_mm[p] = mm;
        }
        state.label_conflicts = usize::try_from(state.label_conflicts as i64 + delta.labels)
            .expect("the label conflicts stay at 0 or above");
        state.cost += delta.cost;
    }

    /// A random proposal for the movable part `part` (design C4), or `None` when the kind
    /// drawn has nothing to propose: a shift by up to `radius_mm` on each axis, snapped to
    /// the 0.25 mm grid (55 in 100); a turn by 1 to 3 quarter turns about the centre (12;
    /// nothing when the part may not turn); a swap of centres with another movable part
    /// drawn uniformly (15; nothing when it draws the part itself); or the attraction
    /// with a jitter of `radius_mm` / 4 (18). A proposal that would leave the part where
    /// and as it is counts as nothing. A stage's proposals are the draws that named a
    /// move, legal or not; the uphill share is the uphill moves made over that count.
    fn propose(
        &self,
        state: &State,
        part: usize,
        radius_mm: f64,
        rng: &mut SplitMix64,
    ) -> Option<Move> {
        let (at, turns) = (state.centres[part], state.turns[part]);
        let kind = unit(rng);
        if kind < SHIFT_SHARE {
            let (dx, dy) = (grid_shift(rng, radius_mm), grid_shift(rng, radius_mm));
            if dx == 0.0 && dy == 0.0 {
                return None;
            }
            Some(Move {
                parts: vec![(part, (at.0 + dx, at.1 + dy), turns)],
            })
        } else if kind < SHIFT_SHARE + TURN_SHARE {
            if !self.may_turn[part] {
                return None;
            }
            let by = 1 + rng.below(3) as u8;
            Some(Move {
                parts: vec![(part, at, (turns + by) % 4)],
            })
        } else if kind < SHIFT_SHARE + TURN_SHARE + SWAP_SHARE {
            let other = self.movable[rng.below(self.movable.len() as u64) as usize];
            if other == part {
                return None;
            }
            Some(Move {
                parts: vec![
                    (part, state.centres[other], turns),
                    (other, at, state.turns[other]),
                ],
            })
        } else {
            let mv = self.attraction(state, part, radius_mm / 4.0, rng)?;
            let (_, to, to_turns) = mv.parts[0];
            (to != at || to_turns != turns).then_some(mv)
        }
    }

    /// Making room (design C4), for a decoupling pair past its limit: the capacitor at the
    /// cell nearest the IC's pad (0.5 mm grid, within the pair's limit) that is legal
    /// against the outline, the copper obstacles, the held parts and the IC, whatever
    /// movable parts stand there; each movable part it then lies over is moved to its own
    /// nearest legal cell, largest first, against everything as it then stands. `None`
    /// when no such cell exists or a displaced part finds none within 25 rings. The move
    /// names the capacitor and every displaced part.
    ///
    /// "Nearest the IC's pad" is the capacitor's own pad of the pair: the cells are its
    /// centre on the grid, ordered by how far they put that pad from the IC's pad (ties by
    /// grid step), and the capacitor keeps its turns. A held capacitor, or a pair whose
    /// two pads are on one part, gets no move. A displaced part keeps its turns and
    /// searches from its own centre, snapped to the grid.
    fn make_room(&self, state: &State, pair: usize) -> Option<Move> {
        let Pair {
            cap: (cap, cap_slot),
            ic: (ic, ic_slot),
            limit_mm,
        } = self.pairs[pair];
        if cap == ic || !self.model.movable[cap] {
            return None;
        }
        let target = self.pad_at(ic, ic_slot, state.centres[ic], state.turns[ic]);
        let turns = state.turns[cap];
        let size = self.size_at(cap, turns);
        // The cap's pad from its centre: the search is laid around the centre that puts
        // the pad on the IC's pad.
        let here = state.centres[cap];
        let own = self.pad_at(cap, cap_slot, here, turns);
        let base = (
            snap_to(target.0 - (own.0 - here.0), GRID_MM),
            snap_to(target.1 - (own.1 - here.1), GRID_MM),
        );
        let reach = (limit_mm / GRID_MM).ceil() as i64 + 1;
        let mut cells: Vec<(f64, i64, i64)> = Vec::new();
        for di in -reach..=reach {
            for dj in -reach..=reach {
                let cell = (base.0 + di as f64 * GRID_MM, base.1 + dj as f64 * GRID_MM);
                let pad = self.pad_at(cap, cap_slot, cell, turns);
                let d = (pad.0 - target.0).hypot(pad.1 - target.1);
                if d <= limit_mm {
                    cells.push((d, di, dj));
                }
            }
        }
        cells.sort_by(|a, b| a.0.total_cmp(&b.0).then((a.1, a.2).cmp(&(b.1, b.2))));
        // Legal against the outline, the copper, the held parts and the IC only.
        let count = self.model.parts.len();
        let fixed: Vec<usize> = (0..count)
            .filter(|&j| j != cap && (j == ic || !self.model.movable[j]))
            .collect();
        let cell = cells.into_iter().find_map(|(_, di, dj)| {
            let cell = (base.0 + di as f64 * GRID_MM, base.1 + dj as f64 * GRID_MM);
            self.legality
                .legal_at(
                    cap,
                    cell,
                    size,
                    fixed.iter().copied(),
                    &state.centres,
                    &state.sizes,
                )
                .then_some(cell)
        })?;

        // The movable parts the capacitor now lies over, largest first.
        let mut displaced: Vec<usize> = (0..count)
            .filter(|&j| {
                j != cap
                    && j != ic
                    && self.model.movable[j]
                    && !self
                        .legality
                        .clear_of(cap, cell, size, j, state.centres[j], state.sizes[j])
            })
            .collect();
        displaced.sort_by(|&a, &b| {
            let area = |p: usize| state.sizes[p].0 * state.sizes[p].1;
            area(b).total_cmp(&area(a)).then(a.cmp(&b))
        });
        let mut centres = state.centres.clone();
        centres[cap] = cell;
        let mut parts = vec![(cap, cell, turns)];
        for d in displaced {
            let from = (
                snap_to(centres[d].0, GRID_MM),
                snap_to(centres[d].1, GRID_MM),
            );
            let others: Vec<usize> = (0..count).filter(|&j| j != d).collect();
            let found = self.first_legal_cell(
                d,
                state.sizes[d],
                from,
                others.iter().copied(),
                (&centres, &state.sizes),
                |at| {
                    self.legality.legal_at(
                        d,
                        at,
                        state.sizes[d],
                        others.iter().copied(),
                        &centres,
                        &state.sizes,
                    )
                },
            )?;
            centres[d] = found;
            parts.push((d, found, state.turns[d]));
        }
        Some(Move { parts })
    }

    /// The attraction of `part`: the mean of the pads its nets reach on other parts (a
    /// pair's pad counts three times), jittered by up to `jitter_mm` and snapped to the
    /// 0.25 mm grid of the shifts; then the nearest legal cell among that point and the
    /// points around it in steps of 0.5 mm, within 25 rings (amendment A1). `None` when
    /// the part reaches no other part or no cell is legal.
    ///
    /// A pad counts by its net's weight in the cost (a rail's by `rail_weight`), as the
    /// experiment weighed it. A jitter of 0 draws nothing from `rng`. The search's centre
    /// is on the 0.25 mm grid and not on the fixed 0.5 mm lattice, so a part that an
    /// attraction placed before does not get its own cell back whenever the mean has moved
    /// by a quarter millimetre (the experiment's `prop_attract`).
    fn attraction(
        &self,
        state: &State,
        part: usize,
        jitter_mm: f64,
        rng: &mut SplitMix64,
    ) -> Option<Move> {
        let jitter = if jitter_mm > 0.0 {
            let dx = (2.0 * unit(rng) - 1.0) * jitter_mm;
            let dy = (2.0 * unit(rng) - 1.0) * jitter_mm;
            (dx, dy)
        } else {
            (0.0, 0.0)
        };
        self.attraction_at(state, part, jitter)
    }

    /// The attraction of `part` with the jitter given (see `attraction`).
    fn attraction_at(&self, state: &State, part: usize, jitter: (f64, f64)) -> Option<Move> {
        let pad =
            |(q, slot): (usize, usize)| self.pad_at(q, slot, state.centres[q], state.turns[q]);
        let (mut sx, mut sy, mut weight) = (0.0, 0.0, 0.0);
        for &n in &self.part_nets[part] {
            let net = &self.nets[n];
            for &site in &net.sites {
                if site.0 != part {
                    let (x, y) = pad(site);
                    sx += net.weight * x;
                    sy += net.weight * y;
                    weight += net.weight;
                }
            }
        }
        for &k in &self.part_pairs[part] {
            let pair = &self.pairs[k];
            let other = if pair.cap.0 == part {
                pair.ic
            } else {
                pair.cap
            };
            if other.0 != part {
                let (x, y) = pad(other);
                sx += PAIR_PULL * x;
                sy += PAIR_PULL * y;
                weight += PAIR_PULL;
            }
        }
        if weight <= 0.0 {
            return None;
        }
        // The target on the shifts' 0.25 mm grid (amendment A1); the rings step 0.5 mm.
        let base = (
            snap_to(sx / weight + jitter.0, SHIFT_GRID_MM),
            snap_to(sy / weight + jitter.1, SHIFT_GRID_MM),
        );
        let mut mv = Move {
            parts: vec![(part, base, state.turns[part])],
        };
        // `legal` tests the part against every other part where it stands, so a cell that
        // one of them blocks is one `legal` refuses.
        let size = self.size_at(part, state.turns[part]);
        let cell = self.first_legal_cell(
            part,
            size,
            base,
            (0..self.model.parts.len()).filter(|&j| j != part),
            (&state.centres, &state.sizes),
            |cell| {
                mv.parts[0].1 = cell;
                self.legal(state, &mv)
            },
        )?;
        mv.parts[0].1 = cell;
        Some(mv)
    }

    /// The first cell of `nearest_cells` around `base` (rings 0 to 24 in steps of
    /// `GRID_MM`, from `base` wherever it lies: the attraction's base is on the 0.25 mm
    /// grid, making room's on the 0.5 mm grid) at which `legal` holds, or `None`. It is the cell
    /// `nearest_cells().iter().map(cell).find(legal)` gives, found with fewer calls of
    /// `legal`, on one condition: `legal(cell)` is false whenever part `part`, its box of
    /// `size` at the cell, is not `Legality::clear_of` some part `j` of `blockers` at
    /// `centres[j]` with `sizes[j]` (the caller's `legal` tests at least these parts).
    ///
    /// Why it is the same cell: `clear_of` is false exactly when the overlap on x and the
    /// overlap on y both pass the gate, and the overlap on x reads only the cell's x (on y,
    /// only its y). So for each blocker the columns whose x passes are found once, by
    /// `clear_of` itself at the blocker's own y (where the y overlap is half the two
    /// heights, which passes any gate of at least 0), and the rows likewise. A cell in
    /// both is one where `clear_of` is false, so `legal` is false and it is skipped; every
    /// other cell is decided by `legal` itself, in the same order as before. A blocker
    /// too far for any cell to reach is not tested at all; that only saves time, since a
    /// cell it does not mark is decided by `legal`.
    fn first_legal_cell(
        &self,
        part: usize,
        size: (f64, f64),
        base: (f64, f64),
        blockers: impl IntoIterator<Item = usize>,
        (centres, sizes): Boxes<'_>,
        mut legal: impl FnMut((f64, f64)) -> bool,
    ) -> Option<(f64, f64)> {
        const SPAN: usize = 2 * (CELL_RINGS as usize - 1) + 1;
        let reach = (CELL_RINGS - 1) as f64 * GRID_MM;
        // A blocker farther than this from the search's centre on an axis blocks no cell:
        // the reach, the two half sizes, the widest gap and a millimetre more.
        let slack = self.model.clearances.part_gap_mm.max(0.0) + 1.0;
        let offset = CELL_RINGS - 1;
        // A cell's coordinates, by the same arithmetic as the search's own.
        let column = |k: usize| base.0 + (k as i64 - offset) as f64 * GRID_MM;
        let row = |k: usize| base.1 + (k as i64 - offset) as f64 * GRID_MM;
        // `blocked[column]` has bit `row` set when a blocker is not clear of that cell.
        let mut blocked = [0u64; SPAN];
        for j in blockers {
            let (at, other) = (centres[j], sizes[j]);
            if (at.0 - base.0).abs() > reach + (size.0 + other.0) / 2.0 + slack
                || (at.1 - base.1).abs() > reach + (size.1 + other.1) / 2.0 + slack
            {
                continue;
            }
            let mut rows = 0u64;
            for k in 0..SPAN {
                if !self
                    .legality
                    .clear_of(part, (at.0, row(k)), size, j, at, other)
                {
                    rows |= 1 << k;
                }
            }
            if rows == 0 {
                continue;
            }
            for (k, columns) in blocked.iter_mut().enumerate() {
                if !self
                    .legality
                    .clear_of(part, (column(k), at.1), size, j, at, other)
                {
                    *columns |= rows;
                }
            }
        }
        nearest_cells().iter().find_map(|&(di, dj)| {
            let (ci, cj) = ((di + offset) as usize, (dj + offset) as usize);
            if blocked[ci] & (1 << cj) != 0 {
                return None;
            }
            let cell = (base.0 + di as f64 * GRID_MM, base.1 + dj as f64 * GRID_MM);
            legal(cell).then_some(cell)
        })
    }

    /// The moves descent tries for `part`, in its fixed order (design C6): shifts of
    /// 0.25, 0.5, 1, 2 and 4 mm in the 8 compass directions, the 3 turns (when the part
    /// may turn), the attraction without jitter (when it goes somewhere), and swaps with
    /// the part's 6 nearest movable parts (by centre distance, ties by index). Lazy: the
    /// attraction and the nearest parts are found only when the moves before them are
    /// reached.
    fn descent_moves<'s>(
        &'s self,
        state: &'s State,
        part: usize,
    ) -> impl Iterator<Item = Move> + 's {
        let (at, turns) = (state.centres[part], state.turns[part]);
        let shifts = DESCENT_STEPS_MM.iter().flat_map(move |&step| {
            DIRECTIONS.iter().map(move |&(dx, dy)| Move {
                parts: vec![(part, (at.0 + dx * step, at.1 + dy * step), turns)],
            })
        });
        let may_turn = self.may_turn[part];
        let turned = (1..=3u8).filter(move |_| may_turn).map(move |by| Move {
            parts: vec![(part, at, (turns + by) % 4)],
        });
        let attracted = std::iter::once_with(move || self.attraction_at(state, part, (0.0, 0.0)))
            .flatten()
            .filter(move |mv| mv.parts[0].1 != at);
        let swaps = std::iter::once_with(move || {
            let mut near: Vec<(f64, usize)> = self
                .movable
                .iter()
                .filter(|&&o| o != part)
                .map(|&o| {
                    let c = state.centres[o];
                    ((c.0 - at.0).powi(2) + (c.1 - at.1).powi(2), o)
                })
                .collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            near.truncate(DESCENT_SWAPS);
            near
        })
        .flatten()
        .map(move |(_, other)| Move {
            parts: vec![
                (part, state.centres[other], turns),
                (other, at, state.turns[other]),
            ],
        });
        shifts.chain(turned).chain(attracted).chain(swaps)
    }

    /// `state` with every cache computed from nothing again (design C9), the placement
    /// unchanged.
    fn rebuild(&self, state: &mut State) {
        let centres = std::mem::take(&mut state.centres);
        let turns = std::mem::take(&mut state.turns);
        *state = self.state(centres, turns);
    }
}

/// The shares of the four drawn moves (design C4): shift 55, turn 12, swap 15, and the
/// attraction the rest (18).
const SHIFT_SHARE: f64 = 0.55;
const TURN_SHARE: f64 = 0.12;
const SWAP_SHARE: f64 = 0.15;

/// The grid of a drawn shift's offset, mm (design C4).
const SHIFT_GRID_MM: f64 = 0.25;

/// A decoupling pair's pad counts this many times in the attraction (design C4).
const PAIR_PULL: f64 = 3.0;

/// The rings of a nearest-legal-cell search in steps of `GRID_MM`: ring 0 and 24 more
/// (design C4, amendment A1).
const CELL_RINGS: i64 = 25;

/// The first temperature's trials (design C5, amendment A2): this many shifts of up to
/// this far on each axis, each tried from the start and none made.
const TRIAL_SHIFTS: usize = 4_000;
const TRIAL_RADIUS_MM: f64 = 10.0;
/// The share of times an average uphill shift is accepted at the first temperature
/// (design C5, amendment A2): the run starts melted.
const FIRST_ACCEPTANCE: f64 = 0.95;
/// The first temperature when no trial went uphill.
const COLD_START_TEMPERATURE: f64 = 50.0;

/// Descent's shifts (design C6), and its swaps with this many nearest parts.
const DESCENT_STEPS_MM: [f64; 5] = [0.25, 0.5, 1.0, 2.0, 4.0];
const DIRECTIONS: [(f64, f64); 8] = [
    (1.0, 0.0),
    (-1.0, 0.0),
    (0.0, 1.0),
    (0.0, -1.0),
    (1.0, 1.0),
    (1.0, -1.0),
    (-1.0, 1.0),
    (-1.0, -1.0),
];
const DESCENT_SWAPS: usize = 6;

/// Descent makes a move only when it lowers the cost by more than this, mm; annealing
/// keeps a new best only when it is lower by more than this.
const LOWER_BY_MM: f64 = 1e-9;

/// `value` on the grid of `step`.
fn snap_to(value: f64, step: f64) -> f64 {
    (value / step).round() * step
}

/// A drawn shift of up to `radius_mm` on one axis, on the 0.25 mm grid.
fn grid_shift(rng: &mut SplitMix64, radius_mm: f64) -> f64 {
    snap_to((2.0 * unit(rng) - 1.0) * radius_mm, SHIFT_GRID_MM)
}

/// The cells of rings 0 to 24 around a centre cell, as grid steps: ring by ring, each
/// ring nearest first and ties by step. Built from `legalize`'s own ring function
/// (`place_generators::ring_cells`), so the two searches walk one order.
fn nearest_cells() -> &'static [(i64, i64)] {
    static CELLS: std::sync::OnceLock<Vec<(i64, i64)>> = std::sync::OnceLock::new();
    CELLS.get_or_init(|| (0..CELL_RINGS).flat_map(ring_cells).collect())
}

/// The fall from `before` to `after` as a share of `before`; 0 when `before` is not
/// above 0 (a cost of 0 cannot fall).
fn gain_of(before: f64, after: f64) -> f64 {
    if before > 0.0 {
        (before - after) / before
    } else {
        0.0
    }
}

/// The shift radius at `temperature` (design C5): `r0 x (T / first T)^radius_exponent`,
/// never below 1 mm. It reads the temperature and not the level or the stage count, so
/// every level shrinks it over the same range of temperatures.
fn shift_radius(params: &ConvergeParams, r0: f64, temperature: f64, first: f64) -> f64 {
    (r0 * (temperature / first).powf(params.radius_exponent)).max(MIN_RADIUS_MM)
}

/// The least shift radius, mm (design C5).
const MIN_RADIUS_MM: f64 = 1.0;

/// SHA-256 of the neighbourhoods' JSON, keys sorted (design C11): the keys are written in
/// sorted order and the map is a `BTreeMap`, so the text does not depend on how
/// `serde_json` orders a map. `place_search::rerun` reads it too, to refuse a model whose
/// neighbourhoods are not the ones a trace names.
pub(crate) fn hoods_sha256(hoods: &Neighbourhoods) -> String {
    let json = serde_json::json!({
        "alone": hoods.alone,
        "measured": hoods.measured,
        "neighbours": hoods.neighbours,
        "unmatched": hoods.unmatched,
    });
    let text = serde_json::to_string(&json).expect("the neighbourhoods are JSON");
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// The state a run starts from (design C8): `start`, the placements of a candidate of
/// family F or S (a part it does not list is where and as the model read it), made legal
/// under `GapRule::Everywhere` by `place_generators::legalize_with` (a part that already
/// keeps the gap stays; one that does not moves to its nearest legal cell). `NoRoom` when
/// a part finds none; `Invalid` when a placement names a part that is not movable or a
/// rotation that is not the part's own plus quarter turns.
fn state_from(
    cost: &CostModel<'_>,
    start: &[IpcFootprintPlacement],
) -> Result<State, GenerateFailure> {
    let (mut centres, turns) = centres_of(cost, start)?;
    let sizes: Vec<(f64, f64)> = (0..centres.len())
        .map(|p| cost.size_at(p, turns[p]))
        .collect();
    let quarter_turns: Vec<i32> = turns.iter().map(|&q| i32::from(q)).collect();
    legalize_with(
        cost.model,
        &sizes,
        &quarter_turns,
        &mut centres,
        GapRule::Everywhere,
    )?;
    Ok(cost.state(centres, turns))
}

/// The centres and turns of `placements` (absolute anchors and rotations, as a candidate
/// lists them), every part they do not list where and as the model read it. A placement
/// exactly at the anchor and rotation read is the part as read. `Invalid` when a
/// placement names a part that is not on the board, a held part, a part twice, a value
/// that is not finite, or a rotation that is not the part's own plus quarter turns.
fn centres_of(
    cost: &CostModel<'_>,
    placements: &[IpcFootprintPlacement],
) -> Result<Placement, GenerateFailure> {
    let model = cost.model;
    let invalid = |what: String| Err(GenerateFailure::Invalid(what));
    let mut centres = cost.read_centres.clone();
    let mut turns = vec![0u8; centres.len()];
    let mut named = vec![false; centres.len()];
    for placement in placements {
        let reference = &placement.reference;
        let Ok(p) = model.references.binary_search(reference) else {
            return invalid(format!("{reference} is not on the board"));
        };
        if !model.movable[p] {
            return invalid(format!("{reference} is held and never moves"));
        }
        if std::mem::replace(&mut named[p], true) {
            return invalid(format!("{reference} is placed twice"));
        }
        if !(placement.x.is_finite() && placement.y.is_finite() && placement.rotation.is_finite()) {
            return invalid(format!("{reference} has a placement that is not finite"));
        }
        let part = &model.parts[p];
        if (placement.x, placement.y) == part.at && placement.rotation == part.rotation_deg {
            continue;
        }
        let q = quarter_turns_between(part.rotation_deg, placement.rotation);
        let off = normalised_angle(placement.rotation - part.rotation_deg - 90.0 * f64::from(q));
        if off.abs() > 1e-6 {
            return invalid(format!(
                "{reference} at {} degrees is not its own rotation plus quarter turns",
                placement.rotation
            ));
        }
        let offset = turn_vector(cost.frames[p].offset, i32::from(q));
        centres[p] = (placement.x - offset.0, placement.y - offset.1);
        turns[p] = q;
    }
    Ok((centres, turns))
}

/// The first temperature (amendment A2): 4,000 trial shifts of up to 10 mm on each axis
/// (0.25 mm grid) of parts drawn uniformly, each tried from `state` itself and none
/// made: `state` is not changed. Over the legal trials whose soft cost rises (the cost
/// change without the requirement terms of the pairs and connectors: no R and no
/// excess), the mean rise over ln(1 / 0.95), so that an average uphill shift is
/// accepted 19 times in 20 (50 when no trial was uphill).
///
/// The soft rise is `Delta::soft`, computed by `delta` beside the whole change. Each
/// trial draws from `rng`, in this order and whether or not it is legal: the part
/// (`below` the count of movable parts), then the offset on x, then on y (one `unit`
/// each, `grid_shift`).
fn first_temperature(cost: &CostModel<'_>, state: &State, rng: &mut SplitMix64) -> f64 {
    let count = cost.movable.len() as u64;
    let (mut uphill, mut sum) = (0usize, 0.0);
    for _ in 0..TRIAL_SHIFTS {
        let part = cost.movable[rng.below(count) as usize];
        let (dx, dy) = (
            grid_shift(rng, TRIAL_RADIUS_MM),
            grid_shift(rng, TRIAL_RADIUS_MM),
        );
        let at = state.centres[part];
        let mv = Move {
            parts: vec![(part, (at.0 + dx, at.1 + dy), state.turns[part])],
        };
        if !cost.legal(state, &mv) {
            continue;
        }
        let rise = cost.delta(state, &mv).soft;
        if rise > 0.0 {
            uphill += 1;
            sum += rise;
        }
    }
    if uphill == 0 {
        COLD_START_TEMPERATURE
    } else {
        sum / uphill as f64 / (1.0 / FIRST_ACCEPTANCE).ln()
    }
}

/// Whether annealing ends after a stage (design C6): `quiet_run` stages in a row were
/// quiet, at least `quiet_stages_at_level`, and the stage's uphill share is below
/// `uphill_floor`. Both are needed.
pub(crate) fn quiet_enough(quiet_run: usize, uphill_share: f64, params: &ConvergeParams) -> bool {
    quiet_run >= params.quiet_stages_at_level() && uphill_share < params.uphill_floor
}

/// Annealing (design C5, C6). A stage is `stage_per_part` proposals per movable part at
/// one temperature; a proposal that is legal is made when it lowers the cost, and
/// otherwise with probability exp(-change / temperature). After each stage: the gain is
/// the fall of the best cost over the stage as a share of the best cost at its start; a
/// stage that gains less than `quiet_gain_at_level` is quiet. Annealing ends when
/// `quiet_enough` says so, or at `max_stages_at_level`.
/// Between stages the temperature is multiplied by `cooling_at_level`, and the shift
/// radius is set from the temperature: `r0 x (T / first T)^radius_exponent`, never below
/// 1 mm, `r0` half the shorter side of the outline's bbox. At each stage's end `repair`
/// runs and the caches are rebuilt from nothing, so rounding does not add up. `state`
/// ends as the best placement seen. Returns the stages run, why it ended, and the best
/// cost after each stage.
fn anneal(
    cost: &CostModel<'_>,
    state: &mut State,
    temperature: f64,
    rng: &mut SplitMix64,
    evaluations: &mut u64,
) -> (usize, AnnealStop, Vec<f64>) {
    let params = &cost.params;
    let first = temperature;
    let (bx0, by0, bx1, by1) = cost.model.bounds;
    let r0 = (bx1 - bx0).min(by1 - by0) / 2.0;
    let count = cost.movable.len();
    let per_stage = params.stage_per_part.saturating_mul(count);
    let (cooling, quiet_gain, most) = (
        params.cooling_at_level(),
        params.quiet_gain_at_level(),
        params.max_stages_at_level(),
    );
    let mut t = first;
    let mut radius = shift_radius(params, r0, t, first);
    let mut best = state.cost;
    let mut best_at: Placement = (state.centres.clone(), state.turns.clone());
    let mut curve = Vec::new();
    let (mut stages, mut quiet_run) = (0usize, 0usize);
    let stop = loop {
        stages += 1;
        let at_start = best;
        let (mut proposals, mut uphill) = (0u64, 0u64);
        for _ in 0..per_stage {
            let part = cost.movable[rng.below(count as u64) as usize];
            let Some(mv) = cost.propose(state, part, radius, rng) else {
                continue;
            };
            proposals += 1;
            if !cost.legal(state, &mv) {
                continue;
            }
            let delta = cost.delta(state, &mv);
            *evaluations += 1;
            if delta.cost > 0.0 {
                // Uphill: made with probability exp(-change / T).
                if unit(rng) >= (-delta.cost / t).exp() {
                    continue;
                }
                uphill += 1;
            }
            cost.apply(state, &mv, delta);
            if state.cost < best - LOWER_BY_MM {
                best = state.cost;
                best_at = (state.centres.clone(), state.turns.clone());
            }
        }
        repair(cost, state, evaluations);
        cost.rebuild(state);
        if state.cost < best - LOWER_BY_MM {
            best = state.cost;
            best_at = (state.centres.clone(), state.turns.clone());
        }
        curve.push(best);
        quiet_run = if gain_of(at_start, best) < quiet_gain {
            quiet_run + 1
        } else {
            0
        };
        let uphill_share = uphill as f64 / proposals.max(1) as f64;
        if quiet_enough(quiet_run, uphill_share, params) {
            break AnnealStop::Quiet;
        }
        if stages >= most {
            break AnnealStop::Guard;
        }
        t *= cooling;
        radius = shift_radius(params, r0, t, first);
    };
    *state = cost.state(best_at.0, best_at.1);
    (stages, stop, curve)
}

/// Descent (design C6). A sweep visits every movable part once, in an order drawn by
/// `rng`; for a part it tries, in this fixed order, shifts of 0.25, 0.5, 1, 2 and 4 mm in
/// the 8 compass directions, the 3 turns, the attraction without jitter, and swaps with
/// its 6 nearest movable parts, makes the first move that lowers the cost by more than
/// 1e-9, and tries again from the top until none does. `repair` runs before the first
/// sweep. Descent ends when a sweep made no move (`FixedPoint`), after `quiet_sweeps`
/// quiet sweeps in a row, or at `max_sweeps`.
fn descend(
    cost: &CostModel<'_>,
    state: &mut State,
    rng: &mut SplitMix64,
    evaluations: &mut u64,
) -> (usize, DescentStop, Vec<f64>) {
    let params = &cost.params;
    repair(cost, state, evaluations);
    cost.rebuild(state);
    let mut curve = Vec::new();
    let (mut sweeps, mut quiet) = (0usize, 0usize);
    loop {
        sweeps += 1;
        let before = state.cost;
        let mut moved = 0usize;
        let mut order = cost.movable.clone();
        // Fisher-Yates, by the seeded stream.
        for i in (1..order.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            order.swap(i, j);
        }
        for &part in &order {
            loop {
                let found = cost.descent_moves(state, part).find_map(|mv| {
                    if !cost.legal(state, &mv) {
                        return None;
                    }
                    *evaluations += 1;
                    let delta = cost.delta(state, &mv);
                    (delta.cost < -LOWER_BY_MM).then_some((mv, delta))
                });
                let Some((mv, delta)) = found else {
                    break;
                };
                cost.apply(state, &mv, delta);
                moved += 1;
            }
        }
        cost.rebuild(state);
        curve.push(state.cost);
        quiet = if gain_of(before, state.cost) < params.quiet_gain {
            quiet + 1
        } else {
            0
        };
        if moved == 0 {
            return (sweeps, DescentStop::FixedPoint, curve);
        }
        if quiet >= params.quiet_sweeps {
            return (sweeps, DescentStop::Quiet, curve);
        }
        if sweeps >= params.max_sweeps {
            return (sweeps, DescentStop::Guard, curve);
        }
    }
}

/// The repair of broken requirements: for every decoupling pair past its limit, in pair
/// order, `CostModel::make_room`, made when it lowers the cost. Returns how many were
/// tried and how many made.
///
/// A pair is read as the state stands after the repairs before it. "Tried" is a compound
/// move that `make_room` gave; it is made when it is legal as a whole and lowers the
/// cost. The counts are also added to the cost model's run totals (`Trace`).
fn repair(cost: &CostModel<'_>, state: &mut State, evaluations: &mut u64) -> (u64, u64) {
    let (mut tried, mut made) = (0u64, 0u64);
    for k in 0..cost.pairs.len() {
        if state.pair_mm[k] <= cost.pairs[k].limit_mm {
            continue;
        }
        let Some(mv) = cost.make_room(state, k) else {
            continue;
        };
        tried += 1;
        if !cost.legal(state, &mv) {
            continue;
        }
        let delta = cost.delta(state, &mv);
        *evaluations += 1;
        if delta.cost < 0.0 {
            cost.apply(state, &mv, delta);
            made += 1;
        }
    }
    let (all_tried, all_made) = cost.repairs.get();
    cost.repairs.set((all_tried + tried, all_made + made));
    (tried, made)
}

/// One run of family C from `start`: `CostModel::build`, `state_from`,
/// `with_requirement` (the larger of the parameter and the start's soft cost),
/// `first_temperature`, `anneal`, `descend`; then the placements of every movable part
/// that is not exactly where and as it was read (`place_generators::placements_from`,
/// the tail of `generate`). The random stream is `SplitMix64(seed)` advanced by
/// `params.level` draws, so the levels of one start do not repeat each other.
/// `Candidate::converged` is `Trace::converged`, `iterations_run` the stages.
/// Pure: the same model, hoods, start, seed and parameters give the same result.
pub(crate) fn converge(
    model: &BoardModel,
    hoods: Option<&Neighbourhoods>,
    start: &[IpcFootprintPlacement],
    seed: u64,
    params: &ConvergeParams,
) -> Result<(Candidate, Trace), GenerateFailure> {
    let cost = CostModel::build(model, hoods, params)?;
    let given = centres_of(&cost, start)?;
    let legal = state_from(&cost, start)?;
    // R follows the start the run works from: the start made legal (its soft cost does
    // not depend on R). Every state of the model before is built again under it.
    let requirement = params.requirement_mm.max(cost.terms(&legal).soft);
    let cost = cost.with_requirement(requirement);
    let start_terms = cost.terms(&cost.state(given.0, given.1));
    let mut state = cost.state(legal.centres, legal.turns);

    let mut rng = SplitMix64(seed);
    for _ in 0..params.level {
        rng.next_u64();
    }
    let mut evaluations = 0u64;
    let first = first_temperature(&cost, &state, &mut rng);
    let (stages, anneal_stop, mut best_by_step) =
        anneal(&cost, &mut state, first, &mut rng, &mut evaluations);
    let (sweeps, descent_stop, by_sweep) = descend(&cost, &mut state, &mut rng, &mut evaluations);
    best_by_step.extend(by_sweep);

    let turns: Vec<i32> = state.turns.iter().map(|&q| i32::from(q)).collect();
    let placements = placements_from(model, &state.centres, &turns);
    // The end as written: the state rebuilt from the anchors `placements_from` rounded.
    let (centres, turns) = centres_of(&cost, &placements)?;
    let end = cost.terms(&cost.state(centres, turns));
    let (repairs_tried, repairs_made) = cost.repairs.get();
    let trace = Trace {
        level: params.level,
        requirement_mm: requirement,
        hoods_sha256: hoods.map(hoods_sha256),
        start: start_terms,
        end,
        repairs_tried,
        repairs_made,
        first_temperature: first,
        stages,
        anneal_stop,
        sweeps,
        descent_stop,
        evaluations,
        best_by_step,
    };
    let candidate = Candidate {
        placements,
        converged: Some(trace.converged()),
        iterations_run: Some(stages),
        trace: None,
    };
    Ok((candidate, trace))
}

/// The rule for starts, on the best costs of the runs of starts 1, 2, 3, ... in start
/// order: the number of starts after which the search stops taking starts, or `None`
/// when it has not stopped within `costs`. It stops at the first n >= `window` + 1 at
/// which min(costs[..n]) does not pay against min(costs[..n - window]) (`OuterRule::pays`),
/// and at `max_starts` whatever the costs say. A start whose run failed is
/// `f64::INFINITY`.
pub(crate) fn starts_stop(costs: &[f64], rule: &OuterRule) -> Option<usize> {
    let best = |n: usize| costs[..n].iter().copied().fold(f64::INFINITY, f64::min);
    for n in 1..=costs.len() {
        if n > rule.window && !rule.pays(best(n - rule.window), best(n)) {
            return Some(n);
        }
        if n >= rule.max_starts {
            return Some(n);
        }
    }
    None
}

/// The rule for effort, on the cost of the best start's run at levels 0, 1, 2, ...: the
/// number of levels after which the search stops doubling, or `None` when it has not
/// stopped within `costs`. It stops after the first level k >= 1 whose cost does not pay
/// against the best of the levels below it, and after `max_level` whatever the costs say.
pub(crate) fn effort_stop(costs: &[f64], rule: &OuterRule) -> Option<usize> {
    let mut below = f64::INFINITY;
    for (k, &cost) in costs.iter().enumerate() {
        if k >= 1 && !rule.pays(below, cost) {
            return Some(k + 1);
        }
        if k >= rule.max_level as usize {
            return Some(k + 1);
        }
        below = below.min(cost);
    }
    None
}

#[cfg(test)]
#[path = "place_converge_tests.rs"]
mod tests;
