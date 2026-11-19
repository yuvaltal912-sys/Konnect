//! The force-directed placer's arithmetic, pulled out of `handle_force_directed`
//! (`refine_placement_force_directed`) so that a search can run it on threads, from
//! other starts and with other parameters. Pure: no I/O, no clock, no randomness of
//! its own.
//!
//! With [`FdParams::legacy`], the legacy spring weights, an empty [`FdExtras`] and the
//! parts' courtyard centres as the start, [`relax`] and then [`resolve_collisions`]
//! perform the same floating-point operations, in the same order, as the handler did
//! before the pull-out. `force_directed_output_is_unchanged_by_the_pull_out` holds them
//! to it.

use konnect_sexp::board::{PcbConnectivityIndex, Side};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Ported spring-embedder constants (attributed in THIRD_PARTY.md).
const FD_K_SPRING: f64 = 0.4;
const FD_K_REPEL: f64 = 80.0;
const FD_K_WALL: f64 = 5.0;
const FD_DAMPING: f64 = 0.85;
const FD_MIN_DIST: f64 = 0.5;
const FD_GRID_MM: f64 = 0.5;
const FD_PATIENCE: usize = 4;

/// An axis-aligned rectangle `(x0, y0, x1, y1)` in board mm, `x0 < x1`, `y0 < y1`.
pub(crate) type Rect = (f64, f64, f64, f64);

/// The spring embedder's constants. Every field is finite; all but
/// `temperature_floor_mm` are above 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FdParams {
    pub k_spring: f64,
    pub k_repel: f64,
    pub k_wall: f64,
    pub damping: f64,
    pub min_dist_mm: f64,
    pub grid_mm: f64,
    /// Rounds the snapped layout must stay unchanged to count as converged.
    pub patience: usize,
    /// The iteration ceiling (the tool's `iterations`, at most 10 000).
    pub iterations: usize,
    /// The first temperature as a share of the bounds' shorter side (legacy 0.05).
    pub step_share: f64,
    /// Added to the falling temperature every round (legacy 0.1 mm).
    pub temperature_floor_mm: f64,
    /// The ring beyond the two half-widths inside which repulsion is boosted
    /// (legacy 1.0 mm). Like the legacy, only the widths (`size.0`) enter it.
    pub clear_ring_mm: f64,
    /// The collision pass searches rings `1..ring_end` (end exclusive) before it leaves
    /// a part where it snapped. Legacy 200: the handler's loop is `for ring in 1..200`,
    /// so 199 rings.
    pub ring_end: usize,
}

impl FdParams {
    /// The tool's constants (`FD_K_SPRING`, `FD_K_REPEL`, `FD_K_WALL`, `FD_DAMPING`,
    /// `FD_MIN_DIST`, `FD_GRID_MM`, `FD_PATIENCE`, which live in this file now), the
    /// legacy literals above, and `iterations` from the tool's argument.
    pub(crate) fn legacy(iterations: usize) -> Self {
        Self {
            k_spring: FD_K_SPRING,
            k_repel: FD_K_REPEL,
            k_wall: FD_K_WALL,
            damping: FD_DAMPING,
            min_dist_mm: FD_MIN_DIST,
            grid_mm: FD_GRID_MM,
            patience: FD_PATIENCE,
            iterations,
            step_share: 0.05,
            temperature_floor_mm: 0.1,
            clear_ring_mm: 1.0,
            ring_end: 200,
        }
    }
}

/// One part as the core sees it, in the caller's order (the handler's: sorted by
/// reference).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FdPart {
    /// Courtyard bbox width and height, mm.
    pub size: (f64, f64),
    pub side: Side,
    /// False for a KiCad-locked, caller-locked or unselected part: it exerts force and
    /// is an obstacle, but never moves.
    pub movable: bool,
}

/// Accumulated spring weight per part pair `(a, b)`, `a < b`, every weight above 0.
/// A `BTreeMap`, so iteration is in pair order, as the handler's per-round `collect`
/// into a `BTreeMap` made it.
pub(crate) type Springs = BTreeMap<(usize, usize), f64>;

/// The handler's spring graph, generalized only in where the weight comes from: for
/// every net of `index`, in its sorted order, with `weight(net) > 0`, every pair of
/// distinct parts of `references` with a pad on that net gains `weight(net)`, added in
/// the handler's order. A pad whose reference is not in `references` is ignored.
/// `references` is sorted and unique; a reference's position is its part index. The
/// handler passes `net_weight`.
pub(crate) fn springs(
    index: &PcbConnectivityIndex,
    references: &[&str],
    weight: &dyn Fn(&str) -> f64,
) -> Springs {
    let ref_index: HashMap<&str, usize> = references
        .iter()
        .enumerate()
        .map(|(i, reference)| (*reference, i))
        .collect();
    let mut springs = Springs::new();
    for net in index.nets() {
        let weight = weight(net);
        if weight.is_nan() || weight <= 0.0 {
            continue;
        }
        let members: Vec<usize> = index
            .pads_of_net(net)
            .iter()
            .filter_map(|p| ref_index.get(p.reference.as_str()).copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for (ai, &a) in members.iter().enumerate() {
            for &b in &members[ai + 1..] {
                *springs.entry((a.min(b), a.max(b))).or_insert(0.0) += weight;
            }
        }
    }
    springs
}

/// A spring between two pads instead of two centres: it pulls the pad of part `a` at
/// `a_offset` (from `a`'s centre) toward the pad of part `b` at `b_offset`, with force
/// `weight * k_spring * distance`, on both parts. Decoupling pairs use it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PadSpring {
    pub a: usize,
    pub b: usize,
    pub a_offset: (f64, f64),
    pub b_offset: (f64, f64),
    pub weight: f64,
}

/// A spring from every member of a group toward the members' mean centre, the mean
/// taken afresh every round, with force `weight * k_spring * distance`. Functional
/// groups use it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GroupSpring {
    pub members: Vec<usize>,
    pub weight: f64,
}

/// Forces the tool never had. Empty (the default) adds no operation at all, so the
/// legacy arithmetic is untouched; otherwise they are added after the walls.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct FdExtras {
    pub pads: Vec<PadSpring>,
    pub groups: Vec<GroupSpring>,
}

/// What the cooling loop ends with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Relaxed {
    /// Unsnapped centres, one per part; an immovable part's is its start.
    pub centres: Vec<(f64, f64)>,
    pub iterations_run: usize,
    /// The layout snapped to `grid_mm` stayed unchanged for `patience` rounds.
    pub converged: bool,
}

/// The cooling loop of `handle_force_directed`, unchanged. Per round: the temperature
/// `shorter side of bounds * step_share * (1 - round / iterations) + floor`; pairwise
/// repulsion, boosted inside the clearance ring; the springs in pair order; the walls
/// of `bounds`; then `extras`. Velocities are damped and clamped to the temperature;
/// each centre is clamped so its half-size stays inside `bounds`. Converged when the
/// layout snapped to `grid_mm` is unchanged for `patience` rounds. Immovable parts keep
/// velocity 0 and their start.
///
/// Requires `parts.len() == start.len()`, every index in `springs` and `extras` below
/// `parts.len()`, and `bounds` at least as large as every part. It panics otherwise:
/// that is a caller bug, not an input condition, since every caller builds all four
/// from one courtyard scan.
pub(crate) fn relax(
    parts: &[FdPart],
    springs: &Springs,
    extras: &FdExtras,
    bounds: Rect,
    start: &[(f64, f64)],
    params: &FdParams,
) -> Relaxed {
    let count = parts.len();
    assert_eq!(count, start.len(), "one start centre per part");
    assert!(
        springs.keys().all(|&(a, b)| a < count && b < count),
        "a spring names a part that does not exist"
    );
    assert!(
        extras.pads.iter().all(|s| s.a < count && s.b < count)
            && extras
                .groups
                .iter()
                .all(|g| g.members.iter().all(|&m| m < count)),
        "an extra spring names a part that does not exist"
    );
    let (ox0, oy0, ox1, oy1) = bounds;
    let (board_w, board_h) = (ox1 - ox0, oy1 - oy0);
    let sizes: Vec<(f64, f64)> = parts.iter().map(|p| p.size).collect();
    let mut pos: Vec<(f64, f64)> = start.to_vec();
    let mut vel: Vec<(f64, f64)> = vec![(0.0, 0.0); count];
    let iterations = params.iterations;
    let step = board_w.min(board_h) * params.step_share;
    let snap = |v: f64| (v / params.grid_mm).round() * params.grid_mm;

    let mut last_snapshot: Vec<(f64, f64)> = Vec::new();
    let mut stable = 0usize;
    let mut iterations_run = 0usize;
    let mut converged = false;
    for iteration in 0..iterations {
        iterations_run = iteration + 1;
        let temperature =
            step * (1.0 - iteration as f64 / iterations as f64) + params.temperature_floor_mm;
        let mut force: Vec<(f64, f64)> = vec![(0.0, 0.0); count];

        // Pairwise repulsion, boosted sharply inside the clearance ring.
        for i in 0..count {
            for j in (i + 1)..count {
                let dx = pos[i].0 - pos[j].0;
                let dy = pos[i].1 - pos[j].1;
                let dist = dx.hypot(dy).max(params.min_dist_mm);
                let min_clear = (sizes[i].0 + sizes[j].0) * 0.5 + params.clear_ring_mm;
                let mut k = params.k_repel;
                if dist < min_clear {
                    k *= (min_clear / dist).powi(2);
                }
                let magnitude = k / (dist * dist);
                let (ux, uy) = (dx / dist, dy / dist);
                force[i].0 += ux * magnitude;
                force[i].1 += uy * magnitude;
                force[j].0 -= ux * magnitude;
                force[j].1 -= uy * magnitude;
            }
        }
        // Spring attraction along shared nets, in pair order.
        for (&(a, b), &weight) in springs {
            let dx = pos[b].0 - pos[a].0;
            let dy = pos[b].1 - pos[a].1;
            let dist = dx.hypot(dy).max(params.min_dist_mm);
            let magnitude = params.k_spring * weight * dist;
            let (ux, uy) = (dx / dist, dy / dist);
            force[a].0 += ux * magnitude;
            force[a].1 += uy * magnitude;
            force[b].0 -= ux * magnitude;
            force[b].1 -= uy * magnitude;
        }
        // Walls push inward.
        for i in 0..count {
            force[i].0 += params.k_wall / (pos[i].0 - ox0).max(0.01);
            force[i].0 -= params.k_wall / (ox1 - pos[i].0).max(0.01);
            force[i].1 += params.k_wall / (pos[i].1 - oy0).max(0.01);
            force[i].1 -= params.k_wall / (oy1 - pos[i].1).max(0.01);
        }
        // Forces the tool never had: pad-to-pad springs, then group springs. With
        // empty extras neither loop runs, so the legacy arithmetic is untouched.
        for spring in &extras.pads {
            let (a, b) = (spring.a, spring.b);
            let dx = (pos[b].0 + spring.b_offset.0) - (pos[a].0 + spring.a_offset.0);
            let dy = (pos[b].1 + spring.b_offset.1) - (pos[a].1 + spring.a_offset.1);
            let dist = dx.hypot(dy).max(params.min_dist_mm);
            let magnitude = params.k_spring * spring.weight * dist;
            let (ux, uy) = (dx / dist, dy / dist);
            force[a].0 += ux * magnitude;
            force[a].1 += uy * magnitude;
            force[b].0 -= ux * magnitude;
            force[b].1 -= uy * magnitude;
        }
        for group in &extras.groups {
            if group.members.is_empty() {
                continue;
            }
            let n = group.members.len() as f64;
            let mean = (
                group.members.iter().map(|&m| pos[m].0).sum::<f64>() / n,
                group.members.iter().map(|&m| pos[m].1).sum::<f64>() / n,
            );
            for &m in &group.members {
                let dx = mean.0 - pos[m].0;
                let dy = mean.1 - pos[m].1;
                let dist = dx.hypot(dy).max(params.min_dist_mm);
                let magnitude = params.k_spring * group.weight * dist;
                force[m].0 += dx / dist * magnitude;
                force[m].1 += dy / dist * magnitude;
            }
        }

        // Integrate: damped velocity clamped to the cooling temperature;
        // locked/unselected parts exert force but never move.
        for i in 0..count {
            if !parts[i].movable {
                vel[i] = (0.0, 0.0);
                continue;
            }
            vel[i].0 = (vel[i].0 + force[i].0) * params.damping;
            vel[i].1 = (vel[i].1 + force[i].1) * params.damping;
            let speed = vel[i].0.hypot(vel[i].1);
            if speed > temperature {
                let scale = temperature / speed;
                vel[i].0 *= scale;
                vel[i].1 *= scale;
            }
            pos[i].0 = (pos[i].0 + vel[i].0).clamp(ox0 + sizes[i].0 / 2.0, ox1 - sizes[i].0 / 2.0);
            pos[i].1 = (pos[i].1 + vel[i].1).clamp(oy0 + sizes[i].1 / 2.0, oy1 - sizes[i].1 / 2.0);
        }

        // Convergence: the SNAPPED layout unchanged for `patience` rounds.
        let snapshot: Vec<(f64, f64)> = pos.iter().map(|p| (snap(p.0), snap(p.1))).collect();
        if snapshot == last_snapshot {
            stable += 1;
            if stable >= params.patience {
                converged = true;
                break;
            }
        } else {
            stable = 0;
            last_snapshot = snapshot;
        }
    }

    Relaxed {
        centres: pos,
        iterations_run,
        converged,
    }
}

/// The handler's collision pass, unchanged. Immovable parts are settled first; then
/// each movable part, in index order, is snapped to `grid_mm` and, while its bbox
/// overlaps a settled part on its own side, spirals outward ring by ring (8 directions
/// per ring, each clamped into `bounds`) to the first free cell, over the rings
/// `1..ring_end`. A part with no free cell stays where it snapped: the
/// placement score's hard failure then says so. Mutates `centres` in place.
///
/// Forces cluster connected parts, but nothing in the physics forbids courtyard
/// overlap: without this pass the springs simply crush a net's members into one spot.
/// Walking the parts in index order and spiralling on the snap grid keeps it
/// deterministic by construction.
pub(crate) fn resolve_collisions(
    parts: &[FdPart],
    bounds: Rect,
    centres: &mut [(f64, f64)],
    params: &FdParams,
) {
    assert_eq!(parts.len(), centres.len(), "one centre per part");
    let (ox0, oy0, ox1, oy1) = bounds;
    let sizes: Vec<(f64, f64)> = parts.iter().map(|p| p.size).collect();
    let snap = |v: f64| (v / params.grid_mm).round() * params.grid_mm;
    let overlaps = |a: (f64, f64), sa: (f64, f64), b: (f64, f64), sb: (f64, f64)| {
        (a.0 - b.0).abs() < (sa.0 + sb.0) * 0.5 && (a.1 - b.1).abs() < (sa.1 + sb.1) * 0.5
    };
    // Immovable parts are obstacles from the start — settle them all before
    // any movable part looks for a free cell, or a cap processed earlier in
    // reference order would never see the locked IC it is about to overlap.
    let mut settled: Vec<usize> = (0..parts.len()).filter(|&i| !parts[i].movable).collect();
    for i in 0..parts.len() {
        if !parts[i].movable {
            continue;
        }
        centres[i] = (snap(centres[i].0), snap(centres[i].1));
        let collides = |p: (f64, f64), settled: &[usize], pos: &[(f64, f64)]| {
            settled
                .iter()
                .any(|&j| parts[j].side == parts[i].side && overlaps(p, sizes[i], pos[j], sizes[j]))
        };
        if collides(centres[i], &settled, centres) {
            'search: for ring in 1..params.ring_end {
                let r = ring as f64 * params.grid_mm;
                for (dx, dy) in [
                    (r, 0.0),
                    (0.0, r),
                    (-r, 0.0),
                    (0.0, -r),
                    (r, r),
                    (-r, r),
                    (r, -r),
                    (-r, -r),
                ] {
                    let candidate = (
                        (centres[i].0 + dx).clamp(ox0 + sizes[i].0 / 2.0, ox1 - sizes[i].0 / 2.0),
                        (centres[i].1 + dy).clamp(oy0 + sizes[i].1 / 2.0, oy1 - sizes[i].1 / 2.0),
                    );
                    if !collides(candidate, &settled, centres) {
                        centres[i] = candidate;
                        break 'search;
                    }
                }
            }
        }
        settled.push(i);
    }
}
