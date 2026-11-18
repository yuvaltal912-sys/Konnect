//! Placement candidates. Each generator is a pure function of the board model and a
//! recipe (family, seed, parameters): the same pair gives the same candidate on any
//! thread, in any order, in any run of the same build. Randomness is
//! `route_search::SplitMix64`, seeded by the recipe alone.

use crate::place_held::{HeldSet, Intent};
use crate::place_judge::FailureKey;
use crate::route_search::SplitMix64;
use crate::tools::placement::force_core::{
    self, FdExtras, FdParams, FdPart, GroupSpring, PadSpring, Rect, Relaxed, Springs,
};
use crate::tools::placement::net_weight;
use konnect_ipc::types::IpcFootprintPlacement;
use konnect_sexp::board::{
    footprint_courtyards, footprints, FootprintCourtyard, PcbConnectivityIndex,
};
use konnect_sexp::outline::{Outline, OutlineStatus};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Bumped whenever any generator's arithmetic changes, so that a record says whether
/// this build can reproduce it.
pub(crate) const GENERATOR_VERSION: u32 = 2;

/// The grid every generator snaps centres to, and legalize searches on (the force
/// core's legacy grid).
pub(crate) const GRID_MM: f64 = 0.5;

/// Rings legalize searches before it gives up on a part.
pub(crate) const LEGALIZE_RINGS: usize = 400;

/// The margins between cells of the schematic grid, mm, in the order tried: the first is
/// the legacy placer's. `schematic_grid` uses the first one whose rows fit the board.
const CELL_MARGINS_MM: [f64; 4] = [2.0, 1.0, 0.5, 0.25];

/// A part a generator placed must clear a neighbour by at least this, so that no
/// rounding of the written board can turn "touching" into a courtyard overlap. Anchors
/// are rounded to 1 µm (`round_um`), so each part's courtyard moves by at most 0.5 µm
/// on an axis, and two parts close the gap by at most 1 µm; 2 µm keeps a margin.
const MOVED_CLEARANCE_MM: f64 = 2e-3;

/// How far to the right of the board S0's rows start, mm.
const NEUTRAL_GAP_MM: f64 = 5.0;

/// The gap between parts of S0, mm.
const NEUTRAL_SPACING_MM: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Family {
    /// F: the force core from a seeded scatter. Like S, it takes from O only what S0
    /// keeps: the held parts, every side and rotation, and the brief.
    Force,
    /// S: the schematic placer's clusters as grids, optionally polished by the core.
    Schematic,
    /// P: seeded perturbations of the input placement (start O).
    Perturb,
}

/// One candidate's full description: with the model's input, enough to rebuild it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Recipe {
    pub family: Family,
    pub seed: u64,
    pub params: Params,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Params {
    Force(ForceParams),
    Schematic(SchematicParams),
    Perturb(PerturbParams),
}

/// The force core's spring graph and run for one recipe.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct ForceParams {
    /// The weight of a net with more than `fanout_limit` distinct references (a
    /// board-wide rail). Other nets keep the legacy `net_weight`. With
    /// `fanout_limit` = `usize::MAX` no net is a rail, so any `rail_weight` gives the
    /// legacy graph; 3.0 is the legacy weight of a power name.
    pub rail_weight: f64,
    pub fanout_limit: usize,
    /// Weight of each decoupling pair's pad spring (`PadSpring`).
    pub pair_weight: f64,
    /// Weight of each functional group's centre spring (`GroupSpring`).
    pub group_weight: f64,
    pub iterations: usize,
}

/// The schematic placer, with the fanout filter the legacy lacks.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct SchematicParams {
    /// Union-find joins parts only over nets with at most this many distinct
    /// references. The legacy joined over every net, so one ground net made the whole
    /// board one cluster.
    pub fanout_limit: usize,
    /// When set, the force core runs from the grid with these parameters.
    pub polish: Option<ForceParams>,
}

/// Seeded edits of the input placement.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct PerturbParams {
    pub moves: usize,
    /// A shift moves a part's centre by up to this far.
    pub radius_mm: f64,
    /// Swaps of two movable parts with the same footprint ID are allowed.
    pub swaps: bool,
    /// Quarter and half turns of a part with exactly 2 pads are allowed.
    pub turns: bool,
}

/// Everything a generator reads, built once per board from its saved text.
#[derive(Debug, Clone)]
pub(crate) struct BoardModel {
    pub text: String,
    /// Every footprint's courtyard, sorted by reference (a part's index everywhere).
    pub parts: Vec<FootprintCourtyard>,
    pub references: Vec<String>,
    pub index: PcbConnectivityIndex,
    pub outline: Outline,
    /// The outline's bbox: the force core's bounds.
    pub bounds: Rect,
    pub movable: Vec<bool>,
    /// Distinct references per net, for the fanout filters.
    pub net_fanout: BTreeMap<String, usize>,
    pub held: HeldSet,
    pub intent: Intent,
    /// Each part's footprint library ID (a swap needs two parts with the same one).
    /// Added to the skeleton: `perturb` needs it.
    pub footprint_ids: Vec<String>,
    /// Each part's pad count (a turn needs exactly 2). Added to the skeleton for the
    /// same reason.
    pub pad_counts: Vec<usize>,
    /// The input board's own hard failures that name a movable part (kind and sorted
    /// references), P1's yardstick: a candidate fails P1 only on one that is not among
    /// them. Empty from `board_model`, which does not evaluate; `place_search::search`
    /// sets it from the input board's score, so a model of another origin judges every
    /// failure as new.
    pub input_failures: Vec<FailureKey>,
}

/// Builds the model. Refuses a text whose SHA-256 is not `held.board_sha256`, a scan
/// that skipped a footprint, and an outline that is not closed (both are refused by
/// `held_set` too; checked again because the model may be built from another call).
pub(crate) fn board_model(text: &str, held: HeldSet, intent: Intent) -> anyhow::Result<BoardModel> {
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    anyhow::ensure!(
        hash == held.board_sha256,
        "the board text is not the one the held set was read from"
    );
    let tree = konnect_sexp::parse_sexp(text)?;
    let scan = footprint_courtyards(&tree);
    anyhow::ensure!(
        scan.skipped == 0,
        "the courtyard scan skipped {} footprint(s)",
        scan.skipped
    );
    let outline = Outline::from_tree(&tree);
    anyhow::ensure!(
        outline.status() == OutlineStatus::Closed,
        "the board outline is not closed"
    );
    let bounds = outline
        .bbox()
        .ok_or_else(|| anyhow::anyhow!("the board outline has no extent"))?;

    // The scan and `footprints` list the same footprints in file order when nothing was
    // skipped, so a courtyard and its node are the same position.
    let nodes = footprints(&tree);
    anyhow::ensure!(
        nodes.len() == scan.items.len(),
        "the footprint scan and the footprint list disagree"
    );
    // A reference shared by pad-less footprints (the logos) is keyed `<reference>#<n>`
    // (`place_held::shared_key`); those parts are held, so the key is only ever an
    // identity here, never written to a board.
    let raw_references: Vec<Option<&str>> =
        scan.items.iter().map(|c| c.reference.as_deref()).collect();
    let keys = crate::place_held::footprint_keys(&raw_references);
    let mut rows: Vec<(String, FootprintCourtyard, String, usize)> = Vec::new();
    for ((node, courtyard), key) in nodes.iter().zip(scan.items.iter()).zip(keys) {
        let reference = key.ok_or_else(|| anyhow::anyhow!("a footprint has no reference"))?;
        let library_id = node
            .get(1)
            .and_then(konnect_sexp::parser::SexpNode::as_str)
            .unwrap_or_default()
            .to_string();
        rows.push((
            reference,
            courtyard.clone(),
            library_id,
            node.find_all("pad").len(),
        ));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    for pair in rows.windows(2) {
        anyhow::ensure!(
            pair[0].0 != pair[1].0,
            "the reference {} is on the board more than once",
            pair[0].0
        );
    }
    let mut references = Vec::with_capacity(rows.len());
    let mut parts = Vec::with_capacity(rows.len());
    let mut footprint_ids = Vec::with_capacity(rows.len());
    let mut pad_counts = Vec::with_capacity(rows.len());
    let mut movable = Vec::with_capacity(rows.len());
    for (reference, courtyard, library_id, pads) in rows {
        let is_movable = held.movable.contains(&reference);
        anyhow::ensure!(
            is_movable || held.held.contains_key(&reference),
            "{reference} is in neither the held nor the movable list"
        );
        movable.push(is_movable);
        references.push(reference);
        parts.push(courtyard);
        footprint_ids.push(library_id);
        pad_counts.push(pads);
    }

    let index = PcbConnectivityIndex::build(&tree);
    let mut net_fanout = BTreeMap::new();
    for net in index.nets() {
        let distinct: std::collections::BTreeSet<&str> = index
            .pads_of_net(net)
            .iter()
            .map(|pad| pad.reference.as_str())
            .collect();
        net_fanout.insert(net.to_string(), distinct.len());
    }
    Ok(BoardModel {
        text: text.to_string(),
        parts,
        references,
        index,
        outline,
        bounds,
        movable,
        net_fanout,
        held,
        intent,
        footprint_ids,
        pad_counts,
        input_failures: Vec::new(),
    })
}

/// One generated placement: an absolute anchor and rotation for every movable part that
/// moved or turned, in reference order. Held parts are never listed, and neither is a
/// movable part that stayed as it was (its footprint block stays byte for byte).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Candidate {
    pub placements: Vec<IpcFootprintPlacement>,
    /// The force core's, when the recipe ran it.
    pub converged: Option<bool>,
    pub iterations_run: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub(crate) enum GenerateFailure {
    /// Legalize found no legal cell for these parts.
    NoRoom(Vec<String>),
    /// The recipe does not fit the model: a parameter out of range, say.
    Invalid(String),
    /// The generator panicked; its message. Set by the pool, never by a generator.
    // Set by the search's pool, which no product code runs (see `place_search`).
    #[allow(dead_code)]
    Panicked(String),
}

// ─── Small geometry helpers ──────────────────────────────────────────────────

fn bbox_centre((x0, y0, x1, y1): Rect) -> (f64, f64) {
    ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
}

fn bbox_size((x0, y0, x1, y1): Rect) -> (f64, f64) {
    (x1 - x0, y1 - y0)
}

fn snap(value: f64) -> f64 {
    (value / GRID_MM).round() * GRID_MM
}

/// Rounds to 1 µm, as the legacy handler rounds anchors.
fn round_um(value: f64) -> f64 {
    (value * 1e3).round() / 1e3
}

/// A vector turned by `quarter_turns` x 90° in KiCad's Y-down, counter-clockwise-on-
/// screen sense (`konnect_sexp::geometry::rotate_about`: `x' = x cos + y sin`,
/// `y' = -x sin + y cos`), exactly.
fn turn_vector((x, y): (f64, f64), quarter_turns: i32) -> (f64, f64) {
    match quarter_turns.rem_euclid(4) {
        0 => (x, y),
        1 => (y, -x),
        2 => (-x, -y),
        _ => (-y, x),
    }
}

/// A courtyard bbox turned by `quarter_turns` x 90° about `anchor`, in KiCad's
/// Y-down, counter-clockwise-on-screen sense (`konnect_sexp::geometry`'s). Exact for a
/// bbox the scan made at a cardinal angle; a superset otherwise, as the scan's own is.
// Used only by tests.
#[allow(dead_code)]
pub(crate) fn turned_bbox(bbox: Rect, anchor: (f64, f64), quarter_turns: i32) -> Rect {
    let (x0, y0, x1, y1) = bbox;
    let mut hull: Option<Rect> = None;
    for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
        let (dx, dy) = turn_vector((x - anchor.0, y - anchor.1), quarter_turns);
        let (px, py) = (anchor.0 + dx, anchor.1 + dy);
        hull = Some(match hull {
            None => (px, py, px, py),
            Some((hx0, hy0, hx1, hy1)) => (hx0.min(px), hy0.min(py), hx1.max(px), hy1.max(py)),
        });
    }
    hull.expect("four corners")
}

/// A seeded value in `[0, 1)`: the top 53 bits of the next draw.
pub(crate) fn unit(rng: &mut SplitMix64) -> f64 {
    (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
}

/// The quarter turns from `from_deg` to `to_deg`, as a whole number.
fn quarter_turns_between(from_deg: f64, to_deg: f64) -> i32 {
    (((to_deg - from_deg) / 90.0).round() as i64).rem_euclid(4) as i32
}

/// KiCad's spelling of an angle: in (-180, 180].
fn normalised_angle(degrees: f64) -> f64 {
    let wrapped = degrees.rem_euclid(360.0);
    if wrapped > 180.0 {
        wrapped - 360.0
    } else {
        wrapped
    }
}

/// Each part's courtyard size as read.
fn as_read_sizes(model: &BoardModel) -> Vec<(f64, f64)> {
    model.parts.iter().map(|p| bbox_size(p.bbox)).collect()
}

/// Each part's courtyard centre as read.
fn as_read_centres(model: &BoardModel) -> Vec<(f64, f64)> {
    model.parts.iter().map(|p| bbox_centre(p.bbox)).collect()
}

// ─── The generators ──────────────────────────────────────────────────────────

fn validate_force(params: &ForceParams) -> Result<(), GenerateFailure> {
    let weights = [params.rail_weight, params.pair_weight, params.group_weight];
    if weights.iter().any(|w| !w.is_finite() || *w < 0.0) {
        return Err(GenerateFailure::Invalid(
            "a spring weight is not a finite number of at least 0".to_string(),
        ));
    }
    if params.iterations == 0 || params.iterations > 10_000 {
        return Err(GenerateFailure::Invalid(format!(
            "iterations {} is outside 1..=10000",
            params.iterations
        )));
    }
    Ok(())
}

/// The force core clamps a movable part inside `bounds`, which needs the part to fit.
fn ensure_parts_fit(model: &BoardModel) -> Result<(), GenerateFailure> {
    let (bx0, by0, bx1, by1) = model.bounds;
    for (i, size) in as_read_sizes(model).iter().enumerate() {
        if model.movable[i] && (size.0 > bx1 - bx0 || size.1 > by1 - by0) {
            return Err(GenerateFailure::Invalid(format!(
                "{} is larger than the board outline's bounding box",
                model.references[i]
            )));
        }
    }
    Ok(())
}

/// One candidate for `recipe`:
/// - Force: `scatter`, then `force`, then `legalize`.
/// - Schematic: `schematic_grid`, then `force` when `polish` is set, then `legalize`.
/// - Perturb: `perturb` of the input, then `legalize`.
///
/// Centres become anchors by each part's own offset (anchor minus courtyard centre,
/// turned with the part), rounded to 1 µm as the legacy handler rounds them. A part that
/// neither moved nor turned is not listed. Pure.
pub(crate) fn generate(model: &BoardModel, recipe: &Recipe) -> Result<Candidate, GenerateFailure> {
    let mut rng = SplitMix64(recipe.seed);
    let read_sizes = as_read_sizes(model);
    let mut converged = None;
    let mut iterations_run = None;
    let (mut centres, rotations) = match (&recipe.family, &recipe.params) {
        (Family::Force, Params::Force(params)) => {
            validate_force(params)?;
            ensure_parts_fit(model)?;
            let start = scatter(model, &mut rng);
            let relaxed = force(model, params, &start);
            converged = Some(relaxed.converged);
            iterations_run = Some(relaxed.iterations_run);
            (
                relaxed.centres,
                model.parts.iter().map(|p| p.rotation_deg).collect(),
            )
        }
        (Family::Schematic, Params::Schematic(params)) => {
            if let Some(polish) = &params.polish {
                validate_force(polish)?;
                ensure_parts_fit(model)?;
            }
            let grid = schematic_grid(model, params, &mut rng);
            let centres = match &params.polish {
                Some(polish) => {
                    let relaxed = force(model, polish, &grid);
                    converged = Some(relaxed.converged);
                    iterations_run = Some(relaxed.iterations_run);
                    relaxed.centres
                }
                None => grid,
            };
            (
                centres,
                model.parts.iter().map(|p| p.rotation_deg).collect(),
            )
        }
        (Family::Perturb, Params::Perturb(params)) => {
            if !params.radius_mm.is_finite() || params.radius_mm <= 0.0 || params.moves == 0 {
                return Err(GenerateFailure::Invalid(
                    "a perturbation needs at least one move and a radius above 0".to_string(),
                ));
            }
            if !model.movable.iter().any(|m| *m) {
                return Err(GenerateFailure::Invalid("no part may move".to_string()));
            }
            perturb(model, params, &mut rng)
        }
        _ => {
            return Err(GenerateFailure::Invalid(
                "the recipe's family and its parameters are of different kinds".to_string(),
            ))
        }
    };

    let turns: Vec<i32> = (0..model.parts.len())
        .map(|i| quarter_turns_between(model.parts[i].rotation_deg, rotations[i]))
        .collect();
    let sizes: Vec<(f64, f64)> = read_sizes
        .iter()
        .zip(&turns)
        .map(|(size, q)| if q % 2 == 0 { *size } else { (size.1, size.0) })
        .collect();
    legalize(model, &sizes, &mut centres)?;

    let read_centres = as_read_centres(model);
    let mut placements = Vec::new();
    for (i, part) in model.parts.iter().enumerate() {
        if !model.movable[i] {
            continue;
        }
        // A part that stayed is not listed at all: a placement would re-spell its text
        // and pad angles (`Set` rewrites them even by 0 degrees), and the candidate
        // would no longer be the input in everything it did not change.
        if centres[i] == read_centres[i] && turns[i] == 0 {
            continue;
        }
        let offset = turn_vector(
            (part.at.0 - read_centres[i].0, part.at.1 - read_centres[i].1),
            turns[i],
        );
        let rotation = if turns[i] == 0 {
            part.rotation_deg
        } else {
            normalised_angle(part.rotation_deg + 90.0 * f64::from(turns[i]))
        };
        placements.push(IpcFootprintPlacement {
            reference: model.references[i].clone(),
            x: round_um(centres[i].0 + offset.0),
            y: round_um(centres[i].1 + offset.1),
            rotation,
        });
    }
    Ok(Candidate {
        placements,
        converged,
        iterations_run,
    })
}

/// S0, the AI arm's start: every movable part in rows to the right of `bounds`, outside
/// the outline, in reference order, 1 mm apart with no overlap; held parts, every side
/// and every rotation as read. F and S start from their own seeded layouts but know no
/// more than S0 does.
///
/// The rows start 5 mm to the right of the rightmost edge of the board's bounds and of
/// every footprint (a held part may stand outside the outline), so a row never meets one.
pub(crate) fn neutral_start(model: &BoardModel) -> Vec<IpcFootprintPlacement> {
    let (bx0, by0, bx1, by1) = model.bounds;
    let right = model.parts.iter().map(|p| p.bbox.2).fold(bx1, f64::max);
    let row_start = right + NEUTRAL_GAP_MM;
    let row_width = (bx1 - bx0).max(40.0);
    let read_centres = as_read_centres(model);
    let sizes = as_read_sizes(model);
    let mut placements = Vec::new();
    let (mut cursor_x, mut cursor_y, mut row_height) = (row_start, by0.min(by1), 0.0f64);
    for (i, part) in model.parts.iter().enumerate() {
        if !model.movable[i] {
            continue;
        }
        let (w, h) = sizes[i];
        if cursor_x > row_start && cursor_x + w > row_start + row_width {
            cursor_x = row_start;
            cursor_y += row_height + NEUTRAL_SPACING_MM;
            row_height = 0.0;
        }
        let centre = (cursor_x + w / 2.0, cursor_y + h / 2.0);
        let offset = (part.at.0 - read_centres[i].0, part.at.1 - read_centres[i].1);
        placements.push(IpcFootprintPlacement {
            reference: model.references[i].clone(),
            x: round_um(centre.0 + offset.0),
            y: round_um(centre.1 + offset.1),
            rotation: part.rotation_deg,
        });
        cursor_x += w + NEUTRAL_SPACING_MM;
        row_height = row_height.max(h);
    }
    placements
}

/// Each movable centre drawn uniformly in `bounds` shrunk by its half-size; held
/// parts at their own centres.
fn scatter(model: &BoardModel, rng: &mut SplitMix64) -> Vec<(f64, f64)> {
    let (bx0, by0, bx1, by1) = model.bounds;
    let sizes = as_read_sizes(model);
    let mut centres = as_read_centres(model);
    for i in 0..centres.len() {
        if !model.movable[i] {
            continue;
        }
        let (w, h) = sizes[i];
        let x = bx0 + w / 2.0 + unit(rng) * ((bx1 - bx0) - w).max(0.0);
        let y = by0 + h / 2.0 + unit(rng) * ((by1 - by0) - h).max(0.0);
        centres[i] = (x, y);
    }
    centres
}

fn find_root(parent: &mut [usize], i: usize) -> usize {
    let mut root = i;
    while parent[root] != root {
        root = parent[root];
    }
    let mut at = i;
    while parent[at] != root {
        let next = parent[at];
        parent[at] = root;
        at = next;
    }
    root
}

/// The movable parts joined by union-find over the nets with at most `fanout_limit`
/// distinct references, as lists of part indices: clusters ordered by their smallest
/// member, members in index order.
fn clusters(model: &BoardModel, fanout_limit: usize) -> Vec<Vec<usize>> {
    let count = model.parts.len();
    let mut parent: Vec<usize> = (0..count).collect();
    let position_of: BTreeMap<&str, usize> = model
        .references
        .iter()
        .enumerate()
        .filter(|(i, _)| model.movable[*i])
        .map(|(i, r)| (r.as_str(), i))
        .collect();
    for net in model.index.nets() {
        if model.net_fanout.get(net).copied().unwrap_or(0) > fanout_limit {
            continue;
        }
        let mut previous: Option<usize> = None;
        for pad in model.index.pads_of_net(net) {
            if let Some(&i) = position_of.get(pad.reference.as_str()) {
                if let Some(p) = previous {
                    let (a, b) = (find_root(&mut parent, p), find_root(&mut parent, i));
                    parent[a.max(b)] = a.min(b);
                }
                previous = Some(i);
            }
        }
    }
    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..count {
        if model.movable[i] {
            let root = find_root(&mut parent, i);
            clusters.entry(root).or_default().push(i);
        }
    }
    clusters.into_values().collect()
}

/// The schematic placer with the fanout filter: union-find over the nets with at most
/// `fanout_limit` distinct references, clusters in a seeded order, members by courtyard
/// area (ties by reference, as the legacy), near-square grids flowing left to right in
/// `bounds` and wrapping into rows, cells of the largest member plus a margin. The
/// margin is the first of `CELL_MARGINS_MM` (2, 1, 0.5, 0.25 mm; the legacy placer's
/// is 2) whose rows fit inside `bounds`; when none does, 0.25 mm, and `legalize`
/// does what it can (containment in the true outline is its job). The margin is a
/// function of the board and the recipe, so a rerun finds the same one, and it is not
/// recorded. Held parts at their own centres.
fn schematic_grid(
    model: &BoardModel,
    params: &SchematicParams,
    rng: &mut SplitMix64,
) -> Vec<(f64, f64)> {
    let mut clusters = clusters(model, params.fanout_limit);
    for i in (1..clusters.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        clusters.swap(i, j);
    }
    let sizes = as_read_sizes(model);
    let ordered: Vec<Vec<usize>> = clusters
        .iter()
        .map(|members| {
            let mut ordered = members.clone();
            ordered.sort_by(|&a, &b| {
                let area = |i: usize| sizes[i].0 * sizes[i].1;
                area(b)
                    .total_cmp(&area(a))
                    .then_with(|| model.references[a].cmp(&model.references[b]))
            });
            ordered
        })
        .collect();
    let mut last = Vec::new();
    for margin in CELL_MARGINS_MM {
        let (centres, fits) = grid_with_margin(model, &sizes, &ordered, margin);
        if fits {
            return centres;
        }
        last = centres;
    }
    last
}

/// The grid of `schematic_grid` for one cell `margin`, and whether its rows fit inside
/// `bounds`: no cluster wider than the board, and the last row ends above the bottom
/// margin.
fn grid_with_margin(
    model: &BoardModel,
    sizes: &[(f64, f64)],
    clusters: &[Vec<usize>],
    margin: f64,
) -> (Vec<(f64, f64)>, bool) {
    let (bx0, by0, bx1, by1) = model.bounds;
    let mut centres = as_read_centres(model);
    let mut cursor_x = bx0 + margin;
    let mut cursor_y = by0 + margin;
    let mut row_height = 0.0f64;
    let mut fits = true;
    for ordered in clusters {
        let n = ordered.len();
        let cols = (n as f64).sqrt().ceil() as usize;
        let rows = n.div_ceil(cols);
        let cell_w = ordered.iter().map(|&i| sizes[i].0).fold(0.0, f64::max) + margin;
        let cell_h = ordered.iter().map(|&i| sizes[i].1).fold(0.0, f64::max) + margin;
        let cluster_w = cell_w * cols as f64;
        let cluster_h = cell_h * rows as f64;
        if cursor_x > bx0 + margin && cursor_x + cluster_w > bx1 - margin {
            cursor_x = bx0 + margin;
            cursor_y += row_height + margin;
            row_height = 0.0;
        }
        if cursor_x + cluster_w > bx1 - margin {
            fits = false;
        }
        row_height = row_height.max(cluster_h);
        for (slot, &i) in ordered.iter().enumerate() {
            let (col, row) = (slot % cols, slot / cols);
            centres[i] = (
                snap(cursor_x + cell_w * (col as f64 + 0.5)),
                snap(cursor_y + cell_h * (row as f64 + 0.5)),
            );
        }
        cursor_x += cluster_w + margin;
    }
    if cursor_y + row_height > by1 - margin {
        fits = false;
    }
    (centres, fits)
}

/// `params.moves` seeded edits of the input placement, each one of: a shift by up to
/// `radius_mm`; a swap of two movable parts with the same footprint ID (`swaps`); a
/// quarter or half turn of a 2-pad part (`turns`). Returns centres and rotations.
///
/// A swap exchanges the two parts' centres (each keeps its own rotation); a part with
/// the same footprint ID on the other side is never a partner (no flips). A turn is
/// about the part's courtyard centre.
fn perturb(
    model: &BoardModel,
    params: &PerturbParams,
    rng: &mut SplitMix64,
) -> (Vec<(f64, f64)>, Vec<f64>) {
    let mut centres = as_read_centres(model);
    let mut rotations: Vec<f64> = model.parts.iter().map(|p| p.rotation_deg).collect();
    let movable: Vec<usize> = (0..model.parts.len())
        .filter(|&i| model.movable[i])
        .collect();
    // Movable parts that share a footprint ID and a side, groups of 2 or more.
    let mut by_footprint: BTreeMap<(&str, bool), Vec<usize>> = BTreeMap::new();
    for &i in &movable {
        by_footprint
            .entry((
                model.footprint_ids[i].as_str(),
                model.parts[i].layer_side == konnect_sexp::board::Side::Back,
            ))
            .or_default()
            .push(i);
    }
    let swap_groups: Vec<Vec<usize>> = by_footprint
        .into_values()
        .filter(|members| members.len() >= 2)
        .collect();
    let two_pad: Vec<usize> = movable
        .iter()
        .copied()
        .filter(|&i| model.pad_counts[i] == 2)
        .collect();
    #[derive(Clone, Copy)]
    enum Edit {
        Shift,
        Swap,
        Turn,
    }
    let mut edits = vec![Edit::Shift];
    if params.swaps && !swap_groups.is_empty() {
        edits.push(Edit::Swap);
    }
    if params.turns && !two_pad.is_empty() {
        edits.push(Edit::Turn);
    }
    for _ in 0..params.moves {
        match edits[rng.below(edits.len() as u64) as usize] {
            Edit::Shift => {
                let i = movable[rng.below(movable.len() as u64) as usize];
                let dx = (2.0 * unit(rng) - 1.0) * params.radius_mm;
                let dy = (2.0 * unit(rng) - 1.0) * params.radius_mm;
                centres[i] = (centres[i].0 + dx, centres[i].1 + dy);
            }
            Edit::Swap => {
                let group = &swap_groups[rng.below(swap_groups.len() as u64) as usize];
                let first = rng.below(group.len() as u64) as usize;
                let mut second = rng.below(group.len() as u64 - 1) as usize;
                if second >= first {
                    second += 1;
                }
                let (a, b) = (group[first], group[second]);
                centres.swap(a, b);
            }
            Edit::Turn => {
                let i = two_pad[rng.below(two_pad.len() as u64) as usize];
                let quarter_turns = rng.below(2) as f64 + 1.0;
                rotations[i] = normalised_angle(rotations[i] + 90.0 * quarter_turns);
            }
        }
    }
    (centres, rotations)
}

/// The force core from `start` with `spring_graph`, the pairs' pad springs and the
/// groups' centre springs, on `model.bounds`; then its collision pass.
fn force(model: &BoardModel, params: &ForceParams, start: &[(f64, f64)]) -> Relaxed {
    let sizes = as_read_sizes(model);
    let parts = core_parts(model, &sizes);
    let springs = spring_graph(model, params);
    let extras = extras(model, params);
    let core = FdParams::legacy(params.iterations);
    let mut relaxed = force_core::relax(&parts, &springs, &extras, model.bounds, start, &core);
    force_core::resolve_collisions(&parts, model.bounds, &mut relaxed.centres, &core);
    relaxed
}

/// The spring graph of `params`: `force_core::springs` with the weight
/// `rail_weight` for a net above `fanout_limit` distinct references, else `net_weight`.
fn spring_graph(model: &BoardModel, params: &ForceParams) -> Springs {
    let references: Vec<&str> = model.references.iter().map(String::as_str).collect();
    let weight = |net: &str| {
        if model.net_fanout.get(net).copied().unwrap_or(0) > params.fanout_limit {
            params.rail_weight
        } else {
            net_weight(net)
        }
    };
    force_core::springs(&model.index, &references, &weight)
}

/// The pairs' pad springs and the groups' centre springs, with the recipe's weights.
fn extras(model: &BoardModel, params: &ForceParams) -> FdExtras {
    let mut extras = FdExtras::default();
    let centres = as_read_centres(model);
    if params.pair_weight > 0.0 {
        // Every pad site of the pairs' references, by (reference, pad number).
        let mut sites: BTreeMap<(&str, &str), Vec<(f64, f64)>> = BTreeMap::new();
        let wanted: std::collections::BTreeSet<&str> = model
            .intent
            .pairs
            .iter()
            .flat_map(|pair| [pair.cap.as_str(), pair.ic.as_str()])
            .collect();
        let netted = model
            .index
            .nets()
            .into_iter()
            .flat_map(|net| model.index.pads_of_net(net).iter());
        for pad in netted.chain(model.index.pads_without_net()) {
            if wanted.contains(pad.reference.as_str()) {
                sites
                    .entry((pad.reference.as_str(), pad.pad_number.as_str()))
                    .or_default()
                    .push(pad.at);
            }
        }
        for pair in &model.intent.pairs {
            let (Ok(a), Ok(b)) = (
                model.references.binary_search(&pair.cap),
                model.references.binary_search(&pair.ic),
            ) else {
                continue;
            };
            let found = |reference: &str, pad: &str| match sites.get(&(reference, pad)) {
                Some(at) if at.len() == 1 => Some(at[0]),
                _ => None,
            };
            let (Some(cap_at), Some(ic_at)) = (
                found(pair.cap.as_str(), pair.cap_pad.as_str()),
                found(pair.ic.as_str(), pair.ic_pad.as_str()),
            ) else {
                continue;
            };
            extras.pads.push(PadSpring {
                a,
                b,
                a_offset: (cap_at.0 - centres[a].0, cap_at.1 - centres[a].1),
                b_offset: (ic_at.0 - centres[b].0, ic_at.1 - centres[b].1),
                weight: params.pair_weight,
            });
        }
    }
    if params.group_weight > 0.0 {
        for group in &model.intent.groups {
            let members: Vec<usize> = group
                .members
                .iter()
                .filter_map(|reference| model.references.binary_search(reference).ok())
                .collect();
            if members.len() >= 2 {
                extras.groups.push(GroupSpring {
                    members,
                    weight: params.group_weight,
                });
            }
        }
    }
    extras
}

/// The core's view of every part: size (after any turn), side, movable.
fn core_parts(model: &BoardModel, sizes: &[(f64, f64)]) -> Vec<FdPart> {
    model
        .parts
        .iter()
        .enumerate()
        .map(|(i, part)| FdPart {
            size: sizes[i],
            side: part.layer_side,
            movable: model.movable[i],
        })
        .collect()
}

/// The cells of ring `ring` around a centre cell, as grid steps, nearest first (ties
/// by step): the centre cell alone for ring 0, else the square ring's `8 * ring` cells.
fn ring_cells(ring: i64) -> Vec<(i64, i64)> {
    if ring == 0 {
        return vec![(0, 0)];
    }
    let mut cells = Vec::with_capacity(8 * ring as usize);
    for i in -ring..=ring {
        cells.push((i, -ring));
        cells.push((i, ring));
    }
    for j in (-ring + 1)..ring {
        cells.push((-ring, j));
        cells.push((ring, j));
    }
    cells.sort_by_key(|&(i, j)| (i * i + j * j, i, j));
    cells
}

/// Containment and overlap repair against the true outline. Held parts settle first.
/// Then each movable part in reference order: when its courtyard bbox (`sizes[i]` at
/// `centres[i]`) overhangs the outline (`Outline::overhang` deeper than
/// `containment_threshold_mm`) or overlaps a settled part on its side, it moves to the
/// nearest legal cell, spiralling on `GRID_MM` for up to `LEGALIZE_RINGS` rings. Every
/// part is placed, or the result is `NoRoom` naming the parts that were not.
///
/// A part at exactly its input position and rotation (its centre and size are the ones
/// read) is legal as it is: neither its containment nor its overlaps with other unmoved
/// parts are checked, because the input may hold them on purpose (a socket over its
/// neighbours, a part at the edge), and P1 judges a candidate against the input's own
/// hard failures. All of them settle first, with the held parts. Only a part that
/// moved or turned must be inside the true outline and clear of every settled part,
/// moved or not, held or not, by 2 µm (`MOVED_CLEARANCE_MM`), so that rounding of the
/// anchors to 1 µm in the written board cannot turn a touch into an overlap. Cells are
/// ordered by distance, the ring's cells in a fixed order, so the result is the same on
/// every run.
pub(crate) fn legalize(
    model: &BoardModel,
    sizes: &[(f64, f64)],
    centres: &mut [(f64, f64)],
) -> Result<(), GenerateFailure> {
    let count = model.parts.len();
    if sizes.len() != count || centres.len() != count {
        return Err(GenerateFailure::Invalid(
            "legalize needs one size and one centre per part".to_string(),
        ));
    }
    let threshold = model.outline.containment_threshold_mm();
    let (bx0, by0, bx1, by1) = model.bounds;
    let read_centres = as_read_centres(model);
    let read_sizes = as_read_sizes(model);

    // `gate` is how much the two boxes must overlap, per axis, to count as overlapping:
    // above minus `MOVED_CLEARANCE_MM` (touching, or nearer than 2 µm, is overlapping).
    let legal_at =
        |i: usize, at: (f64, f64), gate: f64, settled: &[usize], centres: &[(f64, f64)]| {
            let (w, h) = sizes[i];
            let (x0, y0, x1, y1) = (
                at.0 - w / 2.0,
                at.1 - h / 2.0,
                at.0 + w / 2.0,
                at.1 + h / 2.0,
            );
            if x0 < bx0 - threshold
                || y0 < by0 - threshold
                || x1 > bx1 + threshold
                || y1 > by1 + threshold
            {
                return false;
            }
            if model
                .outline
                .overhang(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
                .depth_mm
                > threshold
            {
                return false;
            }
            !settled.iter().any(|&j| {
                model.parts[j].layer_side == model.parts[i].layer_side && {
                    let ix = (w + sizes[j].0) / 2.0 - (at.0 - centres[j].0).abs();
                    let iy = (h + sizes[j].1) / 2.0 - (at.1 - centres[j].1).abs();
                    ix > gate && iy > gate
                }
            })
        };

    let unmoved = |i: usize, centres: &[(f64, f64)]| {
        centres[i] == read_centres[i] && sizes[i] == read_sizes[i]
    };
    // The held parts and every part that stayed settle first, as they are.
    let mut settled: Vec<usize> = (0..count)
        .filter(|&i| !model.movable[i] || unmoved(i, centres))
        .collect();
    let queue: Vec<usize> = (0..count)
        .filter(|&i| model.movable[i] && !unmoved(i, centres))
        .collect();
    let mut failed: Vec<String> = Vec::new();
    for i in queue {
        let strict = -MOVED_CLEARANCE_MM;
        if !legal_at(i, centres[i], strict, &settled, centres) {
            let base = (snap(centres[i].0), snap(centres[i].1));
            let mut found = None;
            'rings: for ring in 0..=LEGALIZE_RINGS as i64 {
                for (di, dj) in ring_cells(ring) {
                    let cell = (base.0 + di as f64 * GRID_MM, base.1 + dj as f64 * GRID_MM);
                    if legal_at(i, cell, strict, &settled, centres) {
                        found = Some(cell);
                        break 'rings;
                    }
                }
            }
            match found {
                Some(cell) => centres[i] = cell,
                None => failed.push(model.references[i].clone()),
            }
        }
        settled.push(i);
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(GenerateFailure::NoRoom(failed))
    }
}

/// The fixed recipe matrix of one board.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Matrix {
    pub force_seeds: Vec<u64>,
    pub force: Vec<ForceParams>,
    pub schematic_seeds: Vec<u64>,
    pub schematic: Vec<SchematicParams>,
    pub perturb_seeds: Vec<u64>,
    pub perturb: Vec<PerturbParams>,
}

impl Matrix {
    /// The bench's matrix (design decision D14): F seeds 1..=32 x rail weight
    /// {0, 0.5, 3} on nets above 8 references (pair weight 10, group weight 0.5, 600
    /// iterations);
    /// S seeds 1..=32 x {raw, polished with rail weight 0.5}; P seeds 1..=64 x
    /// {4 edits within 2 mm, 12 edits within 5 mm}, swaps and turns on. 288 recipes.
    // Used only by the bench (an ignored test): no product code calls the search.
    #[allow(dead_code)]
    pub(crate) fn bench() -> Self {
        let force = |rail_weight: f64| ForceParams {
            rail_weight,
            fanout_limit: 8,
            pair_weight: 10.0,
            group_weight: 0.5,
            iterations: 600,
        };
        Matrix {
            force_seeds: (1..=32).collect(),
            force: vec![force(0.0), force(0.5), force(3.0)],
            schematic_seeds: (1..=32).collect(),
            schematic: vec![
                SchematicParams {
                    fanout_limit: 8,
                    polish: None,
                },
                SchematicParams {
                    fanout_limit: 8,
                    polish: Some(force(0.5)),
                },
            ],
            perturb_seeds: (1..=64).collect(),
            perturb: vec![
                PerturbParams {
                    moves: 4,
                    radius_mm: 2.0,
                    swaps: true,
                    turns: true,
                },
                PerturbParams {
                    moves: 12,
                    radius_mm: 5.0,
                    swaps: true,
                    turns: true,
                },
            ],
        }
    }
}

/// Every recipe of `matrix`, in a fixed order: F, then S, then P; within a family,
/// seed-major, then the parameter sets in the order given.
pub(crate) fn recipes(matrix: &Matrix) -> Vec<Recipe> {
    let mut out = Vec::new();
    for &seed in &matrix.force_seeds {
        for params in &matrix.force {
            out.push(Recipe {
                family: Family::Force,
                seed,
                params: Params::Force(*params),
            });
        }
    }
    for &seed in &matrix.schematic_seeds {
        for params in &matrix.schematic {
            out.push(Recipe {
                family: Family::Schematic,
                seed,
                params: Params::Schematic(*params),
            });
        }
    }
    for &seed in &matrix.perturb_seeds {
        for params in &matrix.perturb {
            out.push(Recipe {
                family: Family::Perturb,
                seed,
                params: Params::Perturb(*params),
            });
        }
    }
    out
}

#[cfg(test)]
#[path = "place_generators_tests.rs"]
mod tests;
