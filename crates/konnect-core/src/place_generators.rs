//! Placement candidates. Each generator is a pure function of the board model and a
//! recipe (family, seed, parameters): the same pair gives the same candidate on any
//! thread, in any order, in any run of the same build. Randomness is
//! `route_search::SplitMix64`, seeded by the recipe alone.

use crate::place_converge::{converge, ConvergeParams, OuterRule, Trace};
use crate::place_held::{HeldSet, Intent};
use crate::place_judge::FailureKey;
use crate::place_schematic::Neighbourhoods;
use crate::route_search::SplitMix64;
use crate::tools::placement::force_core::{
    self, FdExtras, FdParams, FdPart, GroupSpring, PadSpring, Rect, Relaxed, Springs,
};
use crate::tools::placement::net_weight;
use konnect_ipc::types::IpcFootprintPlacement;
use konnect_sexp::board::{
    footprint_courtyards, footprints, FootprintCourtyard, PcbConnectivityIndex, Side,
};
use konnect_sexp::geometry::arc_bbox;
use konnect_sexp::layers::is_copper_name;
use konnect_sexp::outline::{Outline, OutlineStatus};
use konnect_sexp::parser::SexpNode;
use konnect_sexp::planar::{bbox_of, dist};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Bumped whenever any generator's arithmetic changes, so that a record says whether
/// this build can reproduce it. 10: family C's output for one recipe changed with the
/// amendments A1 and A2 (the attraction's grid and the first temperature); a record of
/// version 9 made before them is refused by its version, not by a mismatch of hashes.
pub(crate) const GENERATOR_VERSION: u32 = 10;

/// The grid every generator snaps centres to, and legalize searches on (the force
/// core's legacy grid).
pub(crate) const GRID_MM: f64 = 0.5;

/// Rings legalize searches before it gives up on a part.
pub(crate) const LEGALIZE_RINGS: usize = 400;

/// The margins between parts of the schematic shelves, mm, in the order tried: the first
/// is the legacy placer's cell margin. `schematic_grid` uses the first whose shelves fit.
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
    /// S: the schematic placer's clusters as shelf-packed rows, optionally polished by the core.
    Schematic,
    /// P: seeded perturbations of the input placement (start O).
    Perturb,
    /// C: the convergent search (`place_converge`), one run per start and level.
    Converge,
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
    // Made by `place_search::converge_all` (the bench alone reaches it) and by tests.
    #[allow(dead_code)]
    Converge(ConvergeRecipe),
}

/// A run of family C, enough to rebuild it: the recipe of the candidate it starts from
/// (family F or S, never C) and its parameters, the level among them. The run's seed is
/// the start recipe's.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ConvergeRecipe {
    pub start: Box<Recipe>,
    pub params: ConvergeParams,
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
    /// The clearances `legalize` keeps beyond the courtyards; 0 from `board_model`, set by
    /// `place_search::search` from its routing inputs.
    pub clearances: Clearances,
    /// The copper sides each part occupies (`part_sides_of`), by part index.
    pub part_sides: Vec<Sides>,
    /// The boxes of the board's own fixed copper (`copper_obstacles`): copper text and
    /// graphics, and rule areas that keep footprints or pads out. A moved part stays
    /// clear of the ones on its sides.
    pub copper: Vec<CopperObstacle>,
    /// The schematic neighbourhoods (`place_schematic::Neighbourhoods`). `None` from
    /// `board_model`; `place_search::search` sets it from its `SchematicBaseline`. `None`
    /// when the schematic cannot be read: family C then runs without the term N, and
    /// its trace has `neighbour_mm: null`.
    pub hoods: Option<Neighbourhoods>,
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
    let mut rows: Vec<(String, FootprintCourtyard, String, usize, Sides)> = Vec::new();
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
            part_sides_of(node, courtyard.layer_side),
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
    let mut part_sides = Vec::with_capacity(rows.len());
    for (reference, courtyard, library_id, pads, sides) in rows {
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
        part_sides.push(sides);
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
        clearances: Clearances::default(),
        part_sides,
        copper: copper_obstacles(&tree),
        hoods: None,
    })
}

/// The copper sides of the board an item occupies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct Sides {
    pub top: bool,
    pub bottom: bool,
}

impl Sides {
    const NONE: Sides = Sides {
        top: false,
        bottom: false,
    };
    const TOP: Sides = Sides {
        top: true,
        bottom: false,
    };
    const BOTTOM: Sides = Sides {
        top: false,
        bottom: true,
    };
    const BOTH: Sides = Sides {
        top: true,
        bottom: true,
    };

    fn of(side: Side) -> Sides {
        match side {
            Side::Front => Sides::TOP,
            Side::Back => Sides::BOTTOM,
        }
    }

    fn union(self, other: Sides) -> Sides {
        Sides {
            top: self.top || other.top,
            bottom: self.bottom || other.bottom,
        }
    }

    pub(crate) fn intersects(self, other: Sides) -> bool {
        (self.top && other.top) || (self.bottom && other.bottom)
    }

    fn is_empty(self) -> bool {
        self == Sides::NONE
    }
}

/// The sides a footprint occupies: a footprint with a `thru_hole` or `np_thru_hole` pad
/// has copper (or a drilled hole through the copper) on every layer, so it occupies both
/// sides; any other footprint occupies its own.
fn part_sides_of(footprint: &SexpNode, own: Side) -> Sides {
    let through = footprint.find_all("pad").iter().any(|pad| {
        matches!(
            pad.get(2).and_then(SexpNode::as_str),
            Some("thru_hole" | "np_thru_hole")
        )
    });
    if through {
        Sides::BOTH
    } else {
        Sides::of(own)
    }
}

/// A box of the board's fixed copper that a moved part must keep clear of.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CopperObstacle {
    /// A superset of the item's copper, never a subset.
    pub bbox: Rect,
    pub sides: Sides,
}

/// The sides of the copper layers an item names, by `(layer …)` and `(layers …)`: `F.Cu`
/// the top, `B.Cu` the bottom, any other copper layer (an inner one, `*.Cu`, `F&B.Cu`)
/// both. A layer that is not copper adds nothing.
fn copper_sides(item: &SexpNode) -> Sides {
    let mut names: Vec<&str> = Vec::new();
    if let Some(name) = item.find_str("layer") {
        names.push(name);
    }
    if let Some(list) = item.find("layers").and_then(SexpNode::children) {
        names.extend(list.iter().skip(1).filter_map(SexpNode::as_str));
    }
    names.into_iter().fold(Sides::NONE, |sides, name| {
        sides.union(match name {
            "F.Cu" => Sides::TOP,
            "B.Cu" => Sides::BOTTOM,
            other if is_copper_name(other) => Sides::BOTH,
            _ => Sides::NONE,
        })
    })
}

fn xy(node: &SexpNode) -> Option<(f64, f64)> {
    let (x, y) = (node.get_f64(1)?, node.get_f64(2)?);
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

fn xy_of(item: &SexpNode, tag: &str) -> Option<(f64, f64)> {
    xy(item.find(tag)?)
}

/// The exact box of a three-point arc, bulge included (`konnect_sexp::geometry::arc_bbox`,
/// which falls back to the box of the three points when they are collinear); `None` when a
/// coordinate is not finite.
fn arc_box(start: (f64, f64), mid: (f64, f64), end: (f64, f64)) -> Option<Rect> {
    let (x0, y0, x1, y1) = arc_bbox(start, mid, end);
    [x0, y0, x1, y1]
        .iter()
        .all(|v| v.is_finite())
        .then_some((x0, y0, x1, y1))
}

/// The points that bound a `(pts …)` path: each `(xy …)`, and each `(arc (start …) (mid …)
/// (end …))` by the two corners of its exact box. `None` when an element is unreadable
/// or of another kind.
fn path_points(pts: &SexpNode) -> Option<Vec<(f64, f64)>> {
    let mut points = Vec::new();
    for item in pts.children()?.iter().skip(1) {
        match item.head()? {
            "xy" => points.push(xy(item)?),
            "arc" => {
                let (x0, y0, x1, y1) = arc_box(
                    xy_of(item, "start")?,
                    xy_of(item, "mid")?,
                    xy_of(item, "end")?,
                )?;
                points.extend([(x0, y0), (x1, y1)]);
            }
            _ => return None,
        }
    }
    Some(points)
}

/// The bounding points of the `(pts …)` of `node` (a polygon or a curve); `None` when one
/// is unreadable.
fn pts_of(node: &SexpNode) -> Option<Vec<(f64, f64)>> {
    path_points(node.find("pts")?)
}

fn grow((x0, y0, x1, y1): Rect, by: f64) -> Rect {
    (x0 - by, y0 - by, x1 + by, y1 + by)
}

/// The stroke's half width, as `(stroke (width w))` or the legacy `(width w)`; 0 when
/// absent or unreadable.
fn half_stroke(item: &SexpNode) -> f64 {
    item.find("stroke")
        .and_then(|stroke| stroke.find_f64("width"))
        .or_else(|| item.find_f64("width"))
        .filter(|width| width.is_finite() && *width > 0.0)
        .map_or(0.0, |width| width / 2.0)
}

/// The box of a board graphic's copper: its geometry grown by half the stroke.
fn graphic_box(item: &SexpNode, head: &str) -> Option<Rect> {
    let geometry = match head {
        "gr_line" | "gr_rect" => bbox_of(&[xy_of(item, "start")?, xy_of(item, "end")?])?,
        "gr_circle" => {
            let (centre, edge) = (xy_of(item, "center")?, xy_of(item, "end")?);
            let r = dist(centre, edge);
            (centre.0 - r, centre.1 - r, centre.0 + r, centre.1 + r)
        }
        "gr_arc" => arc_box(
            xy_of(item, "start")?,
            xy_of(item, "mid")?,
            xy_of(item, "end")?,
        )?,
        // A curve stays inside the hull of its control points.
        "gr_poly" | "gr_curve" => bbox_of(&pts_of(item)?)?,
        "gr_text_box" => match pts_of(item) {
            Some(corners) if !corners.is_empty() => bbox_of(&corners)?,
            _ => bbox_of(&[xy_of(item, "start")?, xy_of(item, "end")?])?,
        },
        _ => return None,
    };
    Some(grow(geometry, half_stroke(item)))
}

/// The stroke font's width of one character per unit of size, as a ceiling: KiCad's own
/// boxes of the demo's 19 copper texts gave at most 1.22 (width less thickness, over
/// characters times `size_x`).
const TEXT_WIDTH_PER_CHAR: f64 = 1.3;

/// The stroke font's height of one line per unit of size: KiCad's boxes gave 1.70
/// including the thickness, so 1.7 plus the thickness again is a ceiling.
const TEXT_HEIGHT_PER_LINE: f64 = 1.7;

/// The size of a text that gives none readable, mm: KiCad's default text size.
const DEFAULT_TEXT_SIZE_MM: f64 = 1.524;

/// The pen width of a text that gives none, mm.
const DEFAULT_TEXT_THICKNESS_MM: f64 = 0.15;

/// The box of a board text's copper. With a `render_cache`, the box of its polygons.
/// Without, the stroke-font estimate: width the longest line's characters times
/// `size_x` times `TEXT_WIDTH_PER_CHAR` plus the thickness, height the lines times `size_y`
/// times `TEXT_HEIGHT_PER_LINE` plus the thickness, centred on `at`; a `justify` side
/// makes the box span the anchor by the whole extent on that axis, a superset whichever
/// way the text is mirrored. Turned by `at`'s angle about the anchor: exact for a
/// cardinal angle, the axis-aligned hull otherwise.
pub(crate) fn text_box(item: &SexpNode) -> Option<Rect> {
    // The cache gives the box only when every one of its polygons is readable: a partial
    // box would be a subset, so any unreadable polygon sends the whole text to the estimate.
    if let Some(cache) = item.find("render_cache") {
        let polygons: Option<Vec<Vec<(f64, f64)>>> =
            cache.find_all("polygon").into_iter().map(pts_of).collect();
        if let Some(bbox) = polygons.and_then(|all| bbox_of(&all.concat())) {
            return Some(bbox);
        }
    }

    let text = item.get(1)?.as_str()?;
    if text.is_empty() {
        return None;
    }
    let at = item.find("at")?;
    let (x, y) = xy(at)?;
    let rotation = at.get_f64(3).filter(|r| r.is_finite()).unwrap_or(0.0);
    let effects = item.find("effects");
    let font = effects.and_then(|e| e.find("font"));
    let (size_x, size_y) = font
        .and_then(|f| f.find("size"))
        .and_then(xy)
        .map_or((DEFAULT_TEXT_SIZE_MM, DEFAULT_TEXT_SIZE_MM), |(w, h)| {
            (w.abs(), h.abs())
        });
    let thickness = font
        .and_then(|f| f.find_f64("thickness"))
        .filter(|t| t.is_finite() && *t > 0.0)
        .unwrap_or(DEFAULT_TEXT_THICKNESS_MM);
    let (mut anchored_x, mut anchored_y) = (false, false);
    if let Some(justify) = effects.and_then(|e| e.find("justify")) {
        for word in justify
            .children()
            .unwrap_or(&[])
            .iter()
            .filter_map(SexpNode::as_str)
        {
            match word {
                "left" | "right" => anchored_x = true,
                "top" | "bottom" => anchored_y = true,
                _ => {}
            }
        }
    }

    // `(line_spacing s)` stretches the pitch of the lines; a value below 1 is not trusted.
    let spacing = [font, effects, Some(item)]
        .into_iter()
        .flatten()
        .find_map(|node| node.find_f64("line_spacing"))
        .filter(|s| s.is_finite())
        .map_or(1.0, |s| s.max(1.0));
    let lines = text.split('\n').count() as f64;
    let longest = text
        .split('\n')
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0) as f64;
    let width = longest * size_x * TEXT_WIDTH_PER_CHAR + thickness;
    let height = lines * size_y * TEXT_HEIGHT_PER_LINE * spacing + thickness;
    let half_w = if anchored_x { width } else { width / 2.0 };
    let half_h = if anchored_y { height } else { height / 2.0 };

    let quarter = (rotation / 90.0).round();
    let (half_w, half_h) = if (rotation - quarter * 90.0).abs() < 1e-9 {
        if quarter.rem_euclid(2.0) == 1.0 {
            (half_h, half_w)
        } else {
            (half_w, half_h)
        }
    } else {
        let (sin, cos) = rotation.to_radians().sin_cos();
        (
            half_w * cos.abs() + half_h * sin.abs(),
            half_w * sin.abs() + half_h * cos.abs(),
        )
    };
    // Knockout text is copper around the glyphs, inside a margin KiCad sets to at most
    // max(thickness / 2, size_y / 9); the box grows by max(thickness, size_y / 4).
    let knockout = item
        .find("layer")
        .and_then(SexpNode::children)
        .is_some_and(|words| words.iter().any(|w| w.as_str() == Some("knockout")));
    let margin = if knockout {
        thickness.max(size_y / 4.0)
    } else {
        0.0
    };
    Some((
        x - half_w - margin,
        y - half_h - margin,
        x + half_w + margin,
        y + half_h + margin,
    ))
}

/// The box of a rule area that keeps footprints or pads out (`(footprints not_allowed)`
/// or `(pads not_allowed)` in its `keepout`): its polygon's box, a three-point arc by its
/// exact extent. A keepout that forbids only tracks, vias or pours is no obstacle to a part.
fn keepout_box(zone: &SexpNode) -> Option<Rect> {
    let keepout = zone.find("keepout")?;
    let forbids = |what: &str| keepout.find_str(what) == Some("not_allowed");
    if !forbids("footprints") && !forbids("pads") {
        return None;
    }
    let mut corners: Vec<(f64, f64)> = Vec::new();
    for polygon in zone.find_all("polygon") {
        corners.extend(pts_of(polygon)?);
    }
    bbox_of(&corners)
}

/// The boxes of the board's fixed copper, from its top-level items, each with the sides
/// it is on: graphics and texts on a copper layer, and rule areas that keep footprints or
/// pads out. Silkscreen, fabrication and `Edge.Cuts` items, tracks, vias and pours are
/// not listed, and neither is an item that cannot be read (nothing is guessed).
fn copper_obstacles(tree: &SexpNode) -> Vec<CopperObstacle> {
    let mut found = Vec::new();
    for item in tree.children().unwrap_or(&[]) {
        let Some(head) = item.head() else { continue };
        let sides = copper_sides(item);
        if sides.is_empty() {
            continue;
        }
        let bbox = match head {
            "gr_text" => text_box(item),
            "zone" => keepout_box(item),
            _ => graphic_box(item, head),
        };
        if let Some(bbox) = bbox.filter(|b| [b.0, b.1, b.2, b.3].iter().all(|v| v.is_finite())) {
            found.push(CopperObstacle { bbox, sides });
        }
    }
    found
}

/// The two clearances `legalize` keeps beyond the courtyards, set by the search from the
/// project's rules (`Clearances::from_inputs`) and 0 where nobody set them (tests, a
/// model of another origin).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub(crate) struct Clearances {
    /// What a moved part keeps from a settled one when either box is not a real
    /// courtyard (`CourtyardSource::Courtyard`): the largest `clearance_mm` of the
    /// board's rules plus `SOLDER_MASK_MARGIN_MM`. Two real courtyards keep only
    /// `MOVED_CLEARANCE_MM`: a courtyard already carries its pad clearance, and
    /// courtyards are meant to touch.
    pub part_gap_mm: f64,
    /// How far inside the true outline a moved part's box must lie: the project's
    /// copper-to-edge clearance.
    pub edge_mm: f64,
}

/// Added to the largest clearance for the solder-mask bridge between pads of different
/// nets.
pub(crate) const SOLDER_MASK_MARGIN_MM: f64 = 0.1;

impl Clearances {
    /// From the routing inputs: `part_gap_mm` the largest `clearance_mm` of `rules` plus
    /// `SOLDER_MASK_MARGIN_MM` (0 when no rule gives one), `edge_mm` the options'
    /// `copper_edge_clearance_um` in mm (0 when absent).
    pub(crate) fn from_inputs(
        rules: &konnect_ipc::types::IpcEffectiveRoutingRules,
        options: &crate::specctra::ExportOptions,
    ) -> Self {
        let largest = rules
            .values()
            .filter_map(|rule| rule.clearance_mm)
            .filter(|mm| mm.is_finite() && *mm > 0.0)
            .fold(None, |acc: Option<f64>, mm| {
                Some(acc.map_or(mm, |a| a.max(mm)))
            });
        Clearances {
            part_gap_mm: largest.map_or(0.0, |mm| mm + SOLDER_MASK_MARGIN_MM),
            edge_mm: options
                .copper_edge_clearance_um
                .map_or(0.0, |um| (um as f64 / 1000.0).max(0.0)),
        }
    }

    /// The gap a moved part `a` keeps from the settled part `b`.
    fn gap_between(&self, a: &FootprintCourtyard, b: &FootprintCourtyard) -> f64 {
        use konnect_sexp::board::CourtyardSource;
        if a.bbox_source == CourtyardSource::Courtyard
            && b.bbox_source == CourtyardSource::Courtyard
        {
            MOVED_CLEARANCE_MM
        } else {
            MOVED_CLEARANCE_MM.max(self.part_gap_mm)
        }
    }
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
    /// Family C's run; `None` for the other families.
    pub trace: Option<Trace>,
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
pub(crate) fn turn_vector((x, y): (f64, f64), quarter_turns: i32) -> (f64, f64) {
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
/// - Converge: the start recipe's candidate (family F or S, else `Invalid`), then
///   `place_converge::converge` from it with `model.hoods`; the candidate carries the
///   run's trace.
///
/// Centres become anchors by each part's own offset (anchor minus courtyard centre,
/// turned with the part), rounded to 1 µm as the legacy handler rounds them
/// (`placements_from`). A part that neither moved nor turned is not listed. Pure.
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
        (Family::Converge, Params::Converge(run)) => {
            // Family C refines a candidate of F or S (design C8); a start of P or of C
            // itself is no start.
            if !matches!(run.start.family, Family::Force | Family::Schematic) {
                return Err(GenerateFailure::Invalid(
                    "a run of family C starts from a recipe of family F or S".to_string(),
                ));
            }
            let start = generate(model, &run.start)?;
            let (mut candidate, trace) = converge(
                model,
                model.hoods.as_ref(),
                &start.placements,
                recipe.seed,
                &run.params,
            )?;
            candidate.trace = Some(trace);
            return Ok(candidate);
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
    legalize_turned(model, &sizes, &turns, &mut centres)?;
    Ok(Candidate {
        placements: placements_from(model, &centres, &turns),
        converged,
        iterations_run,
        trace: None,
    })
}

/// The absolute anchor and rotation of every movable part that is not exactly where and
/// as it was read: the tail of `generate`, for `converge` too. A part that stayed is not
/// listed (D11).
pub(crate) fn placements_from(
    model: &BoardModel,
    centres: &[(f64, f64)],
    turns: &[i32],
) -> Vec<IpcFootprintPlacement> {
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
    placements
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
/// `fanout_limit` distinct references, clusters in a seeded order, each cluster's
/// members by courtyard area descending (ties by reference, as the legacy), packed in
/// shelves: parts go left to right inside `bounds`, each at its own width plus a
/// margin, a new shelf opens below (as tall as its tallest part plus the margin) when
/// the next part does not fit the row, and a cluster may continue onto the next shelf.
/// Clusters follow each other in order with one more margin between them. The margin is
/// the first of `CELL_MARGINS_MM` (2, 1, 0.5, 0.25 mm) whose shelves fit inside
/// `bounds`; when none does, the 0.25 mm layout with every centre clamped into `bounds`
/// (inset by half the part's size), so that `legalize` starts on the board and does
/// what it can (containment in the true outline is its job). The margin is a function
/// of the board and the recipe, so a rerun finds the same one, and it is not recorded.
/// Held parts at their own centres.
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
    let (bx0, by0, bx1, by1) = model.bounds;
    for members in &ordered {
        for &i in members {
            let (w, h) = sizes[i];
            last[i] = (
                clamp_inside(last[i].0, bx0 + w / 2.0, bx1 - w / 2.0),
                clamp_inside(last[i].1, by0 + h / 2.0, by1 - h / 2.0),
            );
        }
    }
    last
}

/// `value` within `low..=high`; the midpoint when the range is empty (a part larger
/// than the bounds).
fn clamp_inside(value: f64, low: f64, high: f64) -> f64 {
    if low > high {
        (low + high) / 2.0
    } else {
        value.clamp(low, high)
    }
}

/// The shelves of `schematic_grid` for one `margin`, and whether they fit inside
/// `bounds`: no part wider than the usable width, and the last shelf ends above the
/// bottom margin.
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
    let mut shelf_height = 0.0f64;
    let mut fits = true;
    for ordered in clusters {
        for &i in ordered {
            let (w, h) = sizes[i];
            if cursor_x > bx0 + margin && cursor_x + w > bx1 - margin {
                cursor_x = bx0 + margin;
                cursor_y += shelf_height + margin;
                shelf_height = 0.0;
            }
            if cursor_x + w > bx1 - margin {
                fits = false;
            }
            centres[i] = (cursor_x + w / 2.0, cursor_y + h / 2.0);
            cursor_x += w + margin;
            shelf_height = shelf_height.max(h);
        }
        // One more margin between clusters.
        cursor_x += margin;
    }
    if cursor_y + shelf_height > by1 - margin {
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

/// A decoupling pair whose two parts are on the board and whose two pads were found once
/// each: part indices, and each pad's offset from its part's courtyard centre as read.
#[derive(Debug, Clone, Copy, PartialEq)]
struct MeasuredPair {
    cap: usize,
    ic: usize,
    cap_offset: (f64, f64),
    ic_offset: (f64, f64),
}

/// The pairs of `model.intent` that can be measured, in `intent.pairs` order. A pair whose
/// part is not on the board, or whose pad is missing or found twice, is left out.
fn measured_pairs(model: &BoardModel) -> Vec<MeasuredPair> {
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
    let centres = as_read_centres(model);
    let mut measured = Vec::new();
    for pair in &model.intent.pairs {
        let (Ok(cap), Ok(ic)) = (
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
        measured.push(MeasuredPair {
            cap,
            ic,
            cap_offset: (cap_at.0 - centres[cap].0, cap_at.1 - centres[cap].1),
            ic_offset: (ic_at.0 - centres[ic].0, ic_at.1 - centres[ic].1),
        });
    }
    measured
}

/// The pairs' pad springs and the groups' centre springs, with the recipe's weights.
fn extras(model: &BoardModel, params: &ForceParams) -> FdExtras {
    let mut extras = FdExtras::default();
    if params.pair_weight > 0.0 {
        for pair in measured_pairs(model) {
            extras.pads.push(PadSpring {
                a: pair.cap,
                b: pair.ic,
                a_offset: pair.cap_offset,
                b_offset: pair.ic_offset,
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
/// Family C's nearest-legal-cell search walks the same rings (`place_converge`), so a
/// cell is the nearest by one order in both.
pub(crate) fn ring_cells(ring: i64) -> Vec<(i64, i64)> {
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

/// The order the moved parts settle in, each with the index of its decoupling pair when it
/// is a decoupling cap. `largest` is the moved parts largest first. A moved cap with a
/// measured pair leaves that list and settles right after its IC, as a designer places it;
/// a cap with several pairs uses the first in `pairs` order, and several caps of one IC
/// follow `pairs` order. The caps of an IC that does not move (held or unmoved) come
/// first. A cap whose IC is not a measured part (it is in no pair) keeps its place.
fn settling_order(largest: &[usize], pairs: &[MeasuredPair]) -> Vec<(usize, Option<usize>)> {
    let queued: std::collections::BTreeSet<usize> = largest.iter().copied().collect();
    // Each moved cap's first pair.
    let mut pair_of: BTreeMap<usize, usize> = BTreeMap::new();
    for (k, pair) in pairs.iter().enumerate() {
        if queued.contains(&pair.cap) {
            pair_of.entry(pair.cap).or_insert(k);
        }
    }
    let mut order: Vec<(usize, Option<usize>)> = Vec::with_capacity(largest.len());
    let mut done = std::collections::BTreeSet::new();
    // Place `part`, then the caps whose first pair is with it.
    fn emit(
        part: usize,
        pairs: &[MeasuredPair],
        pair_of: &BTreeMap<usize, usize>,
        done: &mut std::collections::BTreeSet<usize>,
        order: &mut Vec<(usize, Option<usize>)>,
    ) {
        if !done.insert(part) {
            return;
        }
        order.push((part, pair_of.get(&part).copied()));
        for (k, pair) in pairs.iter().enumerate() {
            if pair.ic == part && pair_of.get(&pair.cap) == Some(&k) {
                emit(pair.cap, pairs, pair_of, done, order);
            }
        }
    }
    for (k, pair) in pairs.iter().enumerate() {
        if pair_of.get(&pair.cap) == Some(&k) && !queued.contains(&pair.ic) {
            emit(pair.cap, pairs, &pair_of, &mut done, &mut order);
        }
    }
    for &part in largest {
        if !pair_of.contains_key(&part) {
            emit(part, pairs, &pair_of, &mut done, &mut order);
        }
    }
    // A cap whose IC is itself a cap in a loop of pairs: nothing is left out.
    for &part in largest {
        emit(part, pairs, &pair_of, &mut done, &mut order);
    }
    order
}

/// Containment and overlap repair against the true outline. Held parts settle first.
/// Then each moved part, the largest courtyard box first (`sizes[i]`, width times height;
/// equal areas by reference): a big part needs a free strip that the small ones, settled
/// first, would have scattered across the board. A decoupling cap is the exception: it
/// settles right after its IC (before the other moved parts when the IC is held or
/// unmoved, see `settling_order`). Its search starts at the IC pad, not at the cap's own
/// centre, and takes among the cells of a ring the one that puts the cap's pad nearest the
/// IC pad (`MeasuredPair`). When its courtyard bbox (`sizes[i]` at
/// `centres[i]`) overhangs the outline (`Outline::overhang` deeper than
/// `containment_threshold_mm`) or overlaps a settled part that shares a side with it
/// (`BoardModel::part_sides`: a through-hole part is on both) or a copper obstacle on its
/// sides (`BoardModel::copper`), it moves to the
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
/// anchors to 1 µm in the written board cannot turn a touch into an overlap, and by
/// `part_gap_mm` when either of the two boxes is not a real courtyard (a footprint
/// without one is bounded by its pads, which a 2 µm gap leaves touching: see
/// `Clearances`). A moved part's box also keeps the larger of 2 µm and `part_gap_mm` from
/// every copper obstacle on its sides (the board's own copper text and graphics, and
/// rule areas that keep footprints or pads out); an unmoved part is not tested against
/// them, like the rest. A moved part's box also lies `edge_mm` inside the true outline: the
/// box grown by `edge_mm` on every side must have no overhang (a square growth, a little
/// more than the Euclidean distance at a rounded corner). Cells are ordered by distance,
/// the ring's cells in a fixed order, so the result is the same on every run.
// Used by tests; the generators know the turns and call `legalize_turned`.
#[allow(dead_code)]
pub(crate) fn legalize(
    model: &BoardModel,
    sizes: &[(f64, f64)],
    centres: &mut [(f64, f64)],
) -> Result<(), GenerateFailure> {
    legalize_turned(model, sizes, &vec![0; model.parts.len()], centres)
}

/// `legalize` of parts turned by `turns[i]` quarter turns from their input rotation (which
/// `sizes[i]` already shows); the turn moves a pad offset of the decoupling search.
pub(crate) fn legalize_turned(
    model: &BoardModel,
    sizes: &[(f64, f64)],
    turns: &[i32],
    centres: &mut [(f64, f64)],
) -> Result<(), GenerateFailure> {
    legalize_with(model, sizes, turns, centres, GapRule::CourtyardsMayTouch)
}

/// `legalize_turned` with the gap rule given: `GapRule::CourtyardsMayTouch` is
/// `legalize_turned` itself, and family C makes its starts legal with
/// `GapRule::Everywhere` (design C3, C8). The settling order and the ring search are the
/// same under both; only the gap a moved part keeps from another part's box differs.
pub(crate) fn legalize_with(
    model: &BoardModel,
    sizes: &[(f64, f64)],
    turns: &[i32],
    centres: &mut [(f64, f64)],
    rule: GapRule,
) -> Result<(), GenerateFailure> {
    let count = model.parts.len();
    if sizes.len() != count || centres.len() != count || turns.len() != count {
        return Err(GenerateFailure::Invalid(
            "legalize needs one size, one turn and one centre per part".to_string(),
        ));
    }
    let read_centres = as_read_centres(model);
    let read_sizes = as_read_sizes(model);
    let legality = Legality::new(model, rule);
    let legal_at = |i: usize, at: (f64, f64), settled: &[usize], centres: &[(f64, f64)]| {
        legality.legal_at(i, at, sizes[i], settled.iter().copied(), centres, sizes)
    };

    let unmoved = |i: usize, centres: &[(f64, f64)]| {
        centres[i] == read_centres[i] && sizes[i] == read_sizes[i]
    };
    // The held parts and every part that stayed settle first, as they are.
    let mut settled: Vec<usize> = (0..count)
        .filter(|&i| !model.movable[i] || unmoved(i, centres))
        .collect();
    let mut largest: Vec<usize> = (0..count)
        .filter(|&i| model.movable[i] && !unmoved(i, centres))
        .collect();
    // Largest first, as greedy legalization does; the part index is the reference order.
    largest.sort_unstable_by(|&a, &b| {
        let area = |i: usize| sizes[i].0 * sizes[i].1;
        area(b).total_cmp(&area(a)).then(a.cmp(&b))
    });
    let pairs: Vec<MeasuredPair> = measured_pairs(model)
        .into_iter()
        .filter(|pair| pair.cap != pair.ic)
        .collect();
    let mut failed: Vec<String> = Vec::new();
    for (i, pair) in settling_order(&largest, &pairs) {
        // A decoupling cap searches from the IC pad (the IC settled first, or did not
        // move), and keeps nothing of its own proposed centre.
        let pad_target = pair.map(|k| {
            let (ic, offset) = (
                pairs[k].ic,
                turn_vector(pairs[k].ic_offset, turns[pairs[k].ic]),
            );
            (centres[ic].0 + offset.0, centres[ic].1 + offset.1)
        });
        let legal_as_it_is = pad_target.is_none() && legal_at(i, centres[i], &settled, centres);
        if !legal_as_it_is {
            let origin = pad_target.unwrap_or(centres[i]);
            let base = (snap(origin.0), snap(origin.1));
            let mut found = None;
            'rings: for ring in 0..=LEGALIZE_RINGS as i64 {
                let mut cells = ring_cells(ring);
                if let (Some(target), Some(k)) = (pad_target, pair) {
                    // The cap's own pad, from the cell's centre, nearest the IC pad first;
                    // equal distances keep the ring's order.
                    let own = turn_vector(pairs[k].cap_offset, turns[i]);
                    let away = |&(di, dj): &(i64, i64)| {
                        let x = base.0 + di as f64 * GRID_MM + own.0;
                        let y = base.1 + dj as f64 * GRID_MM + own.1;
                        (x - target.0).hypot(y - target.1)
                    };
                    cells.sort_by(|a, b| away(a).total_cmp(&away(b)));
                }
                for (di, dj) in cells {
                    let cell = (base.0 + di as f64 * GRID_MM, base.1 + dj as f64 * GRID_MM);
                    if legal_at(i, cell, &settled, centres) {
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

// ─── The legality of one moved part ──────────────────────────────────────────

/// Which gap a moved part keeps from another part's box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GapRule {
    /// `legalize` as it is: 2 µm between two real courtyards, and the larger of 2 µm and
    /// `part_gap_mm` when either box is not one (`Clearances::gap_between`).
    CourtyardsMayTouch,
    /// Family C: the larger of 2 µm and `part_gap_mm` between any two boxes.
    // Made by family C (`place_converge`, which the bench alone reaches) and by tests.
    #[allow(dead_code)]
    Everywhere,
}

/// The legality test of one moved part: `legalize_turned`'s `legal_at`, pulled out so that
/// family C tests a move by the same rule that legalizes F and S (the outline, the edge
/// clearance, the part sides, the copper obstacles), with its own gap rule.
pub(crate) struct Legality<'a> {
    model: &'a BoardModel,
    rule: GapRule,
    /// `Outline::containment_threshold_mm`, read once.
    threshold_mm: f64,
}

impl<'a> Legality<'a> {
    pub(crate) fn new(model: &'a BoardModel, rule: GapRule) -> Self {
        Legality {
            model,
            rule,
            threshold_mm: model.outline.containment_threshold_mm(),
        }
    }

    /// Whether part `i`, its box of `size` centred at `at`, is legal there: the box grown
    /// by `edge_mm` on every side lies inside the true outline (first its bbox, then
    /// `Outline::overhang` against the containment threshold); it is clear of every part
    /// of `others` whose sides intersect its own (`clear_of`), each at `centres[j]` with
    /// `sizes[j]`; and it keeps the larger of 2 µm and `part_gap_mm` from every copper
    /// obstacle on its sides. With `GapRule::CourtyardsMayTouch` this is the closure
    /// `legalize_turned` has today, term for term.
    pub(crate) fn legal_at(
        &self,
        i: usize,
        at: (f64, f64),
        size: (f64, f64),
        others: impl IntoIterator<Item = usize>,
        centres: &[(f64, f64)],
        sizes: &[(f64, f64)],
    ) -> bool {
        let model = self.model;
        let threshold = self.threshold_mm;
        let (bx0, by0, bx1, by1) = model.bounds;
        let edge = model.clearances.edge_mm;
        let (w, h) = size;
        // The courtyard box grown by the copper-to-edge clearance on every side: it must
        // be inside the outline, so the box itself is `edge_mm` inside.
        let (x0, y0, x1, y1) = (
            at.0 - w / 2.0 - edge,
            at.1 - h / 2.0 - edge,
            at.0 + w / 2.0 + edge,
            at.1 + h / 2.0 + edge,
        );
        if x0 < bx0 - threshold
            || y0 < by0 - threshold
            || x1 > bx1 + threshold
            || y1 > by1 + threshold
        {
            return false;
        }
        // The four tests are pure and the answer is their conjunction, so their order
        // changes nothing but the time: the cheap box tests first, `Outline::overhang`
        // (the dearest, and the one a crowded board rarely fails) last.
        if others
            .into_iter()
            .any(|j| !self.clear_of(i, at, size, j, centres[j], sizes[j]))
        {
            return false;
        }
        let sides = model.part_sides[i];
        let hits_copper = model.copper.iter().any(|obstacle| {
            sides.intersects(obstacle.sides) && {
                let (ox0, oy0, ox1, oy1) = obstacle.bbox;
                let ix = (w + (ox1 - ox0)) / 2.0 - (at.0 - (ox0 + ox1) / 2.0).abs();
                let iy = (h + (oy1 - oy0)) / 2.0 - (at.1 - (oy0 + oy1) / 2.0).abs();
                let gate = -MOVED_CLEARANCE_MM.max(model.clearances.part_gap_mm);
                ix > gate && iy > gate
            }
        });
        if hits_copper {
            return false;
        }
        // Written as before, so that a depth that is not a number answers as it did.
        if model
            .outline
            .overhang(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
            .depth_mm
            > threshold
        {
            return false;
        }
        true
    }

    /// Whether parts `i` and `j`, at the given centres and sizes, keep the rule's gap, or
    /// share no side. The two parts of a swap are tested against each other with it.
    pub(crate) fn clear_of(
        &self,
        i: usize,
        at_i: (f64, f64),
        size_i: (f64, f64),
        j: usize,
        at_j: (f64, f64),
        size_j: (f64, f64),
    ) -> bool {
        let model = self.model;
        if !model.part_sides[i].intersects(model.part_sides[j]) {
            return true;
        }
        // Two boxes overlap when, on both axes, they overlap by more than minus the gap
        // they must keep (at least 2 µm, so touching is overlapping).
        let ix = (size_i.0 + size_j.0) / 2.0 - (at_i.0 - at_j.0).abs();
        let iy = (size_i.1 + size_j.1) / 2.0 - (at_i.1 - at_j.1).abs();
        let gate = -self.gap(i, j);
        !(ix > gate && iy > gate)
    }

    /// The gap the rule asks between the boxes of parts `i` and `j`.
    fn gap(&self, i: usize, j: usize) -> f64 {
        let clearances = &self.model.clearances;
        match self.rule {
            GapRule::CourtyardsMayTouch => {
                clearances.gap_between(&self.model.parts[i], &self.model.parts[j])
            }
            GapRule::Everywhere => MOVED_CLEARANCE_MM.max(clearances.part_gap_mm),
        }
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
    /// Family C: its level-0 parameters and its outer rules. `None`: the family is off.
    /// C has no fixed list of recipes, so `recipes` does not list it; `converge_all`
    /// makes them.
    pub converge: Option<(ConvergeParams, OuterRule)>,
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
            converge: None,
        }
    }
}

impl Matrix {
    /// This matrix with only the recipes of `families` left: the others get no seed, so
    /// `recipes` makes none of them, and family C is off unless it is named. What is kept
    /// is unchanged, so the F and S recipes are the same ones with or without P or C.
    // Used only by the bench (an ignored test): no product code calls the search.
    #[allow(dead_code)]
    pub(crate) fn only(mut self, families: &[Family]) -> Self {
        if !families.contains(&Family::Force) {
            self.force_seeds.clear();
        }
        if !families.contains(&Family::Schematic) {
            self.schematic_seeds.clear();
        }
        if !families.contains(&Family::Perturb) {
            self.perturb_seeds.clear();
        }
        if !families.contains(&Family::Converge) {
            self.converge = None;
        }
        self
    }
}

/// Every recipe of `matrix`, in a fixed order: F, then S, then P; within a family,
/// seed-major, then the parameter sets in the order given. Family C has no fixed recipes
/// (its starts are F's and S's candidates), so none is listed whatever `converge` holds.
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
pub(crate) mod tests;
