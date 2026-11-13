//! Exact primitive distances with bounded spatial acceleration.
//!
//! The caller owns the effort allowance. Every containment edge, materialized edge,
//! sort comparison/move, node build/visit and primitive comparison is charged before
//! it runs. Exhaustion returns the caller's error, never a partial distance. An edge
//! BVH only removes pairs whose boxes cannot improve the current primitive minimum.
use anyhow::Result;
use konnect_sexp::board_geometry::Shape;
use konnect_sexp::planar::{point_in_ring, segment_distance, Pt};

/// Shapes with more edges than one BVH leaf get a reusable tree.
const LEAF: usize = 8;
/// Rings below this size are scanned directly for point containment.
const SLAB_MIN: usize = 32;

/// The unprepared entry: both shapes are indexed for this one call.
#[cfg(test)]
pub(super) fn distance(
    a: &Shape,
    b: &Shape,
    charge: &mut impl FnMut(usize) -> Result<()>,
) -> Result<f64> {
    distance_with(a, None, b, None, f64::INFINITY, charge)
}

/// Reusable edge tree and even-odd point index for one shape. Building is
/// charged once; later queries charge only the nodes and edges they visit.
/// The answers are those of the unprepared primitives, bit for bit.
pub(super) struct Prepared {
    edges: Vec<Edge>,
    nodes: Vec<Node>,
    slabs: Option<Slabs>,
}

pub(super) fn prepare(
    shape: &Shape,
    charge: &mut impl FnMut(usize) -> Result<()>,
) -> Result<Prepared> {
    let mut edges = edges(shape, charge)?;
    let mut nodes = Vec::new();
    if edges.len() > LEAF {
        charge(edges.len())?;
        let mut scratch = edges.clone();
        build(&mut edges, &mut scratch, 0, &mut nodes, charge)?;
    }
    let slabs = if shape.filled {
        Slabs::build(&shape.pts, charge)?
    } else {
        None
    };
    Ok(Prepared {
        edges,
        nodes,
        slabs,
    })
}

/// Whether `point` lies in a filled shape's interior, by `point_in_ring`'s rule.
pub(super) fn contains(
    polygon: &Shape,
    prepared: Option<&Prepared>,
    point: Pt,
    charge: &mut impl FnMut(usize) -> Result<()>,
) -> Result<bool> {
    if !polygon.filled {
        return Ok(false);
    }
    match prepared.and_then(|p| p.slabs.as_ref()) {
        Some(slabs) => slabs.contains(&polygon.pts, point, charge),
        None => {
            charge(polygon.pts.len())?;
            Ok(point_in_ring(point, &polygon.pts))
        }
    }
}

/// The distance between two shapes, exact wherever it is below `cap`. A result
/// at or above `cap` only says that nothing is nearer than `cap`: edges that
/// cannot come closer are never visited. Pass infinity for the true distance.
pub(super) fn distance_with(
    a: &Shape,
    pa: Option<&Prepared>,
    b: &Shape,
    pb: Option<&Prepared>,
    cap: f64,
    charge: &mut impl FnMut(usize) -> Result<()>,
) -> Result<f64> {
    for (polygon, prepared, point) in [(a, pa, b.pts.first()), (b, pb, a.pts.first())] {
        if let Some(&point) = point {
            if contains(polygon, prepared, point, charge)? {
                return Ok(0.0);
            }
        }
    }
    boundary_gap(a, pa, b, pb, cap, charge)
}

/// Distance between the two shapes' edges and strokes; interiors are ignored.
/// `cap` bounds the search as in [`distance_with`].
pub(super) fn boundary_gap(
    a: &Shape,
    pa: Option<&Prepared>,
    b: &Shape,
    pb: Option<&Prepared>,
    cap: f64,
    charge: &mut impl FnMut(usize) -> Result<()>,
) -> Result<f64> {
    let finish = |best: f64| (best - a.radius - b.radius).max(0.0);
    // Centreline distance beyond which a pair cannot matter. A NaN or negative
    // cap disables the bound instead of hiding edges.
    let reach = if cap >= 0.0 {
        cap + a.radius + b.radius
    } else {
        f64::INFINITY
    };
    let tree_a = pa.filter(|p| !p.nodes.is_empty());
    let tree_b = pb.filter(|p| !p.nodes.is_empty());
    if tree_a.is_some() || tree_b.is_some() {
        let owned;
        let (tree, queries): (&Prepared, &[Edge]) = match (tree_a, tree_b) {
            (Some(x), Some(y)) if x.edges.len() >= y.edges.len() => (x, &y.edges),
            (Some(x), Some(y)) => (y, &x.edges),
            (Some(x), None) => match pb {
                Some(small) => (x, &small.edges),
                None => {
                    owned = edges(b, charge)?;
                    (x, &owned)
                }
            },
            (None, Some(y)) => match pa {
                Some(small) => (y, &small.edges),
                None => {
                    owned = edges(a, charge)?;
                    (y, &owned)
                }
            },
            (None, None) => unreachable!("a prepared tree was selected"),
        };
        if queries.is_empty() {
            return Ok(finish(f64::INFINITY));
        }
        charge(1)?;
        let seed = segment_distance(queries[0].a, queries[0].b, tree.edges[0].a, tree.edges[0].b);
        if finish(seed) == 0.0 {
            return Ok(0.0);
        }
        return search(
            queries,
            &tree.edges,
            &tree.nodes,
            seed.min(reach),
            &finish,
            charge,
        );
    }
    let aa = match pa {
        Some(small) => small.edges.clone(),
        None => edges(a, charge)?,
    };
    let bb = match pb {
        Some(small) => small.edges.clone(),
        None => edges(b, charge)?,
    };
    if aa.is_empty() || bb.is_empty() {
        return Ok(finish(f64::INFINITY));
    }
    let mut best = f64::INFINITY;
    // A point/disc or a short path needs only a linear scan; a tree would cost more.
    if aa.len().min(bb.len()) <= 4 || aa.len().saturating_mul(bb.len()) <= 256 {
        for p in &aa {
            for q in &bb {
                charge(1)?;
                best = best.min(segment_distance(p.a, p.b, q.a, q.b));
                if finish(best) == 0.0 {
                    return Ok(0.0);
                }
            }
        }
        return Ok(finish(best));
    }
    let (queries, mut indexed) = if aa.len() <= bb.len() {
        (aa, bb)
    } else {
        (bb, aa)
    };
    charge(1)?;
    best = segment_distance(queries[0].a, queries[0].b, indexed[0].a, indexed[0].b);
    if finish(best) == 0.0 {
        return Ok(0.0);
    }
    charge(indexed.len())?;
    let mut scratch = indexed.clone();
    let mut nodes = Vec::new();
    build(&mut indexed, &mut scratch, 0, &mut nodes, charge)?;
    search(&queries, &indexed, &nodes, best.min(reach), &finish, charge)
}

/// Exact minimum over every query/indexed edge pair. A node or edge is skipped
/// only when its box gap already exceeds the best primitive distance found.
fn search(
    queries: &[Edge],
    indexed: &[Edge],
    nodes: &[Node],
    mut best: f64,
    finish: &impl Fn(f64) -> f64,
    charge: &mut impl FnMut(usize) -> Result<()>,
) -> Result<f64> {
    let mut stack = Vec::new();
    for query in queries {
        charge(1)?;
        stack.push(0usize);
        while let Some(index) = stack.pop() {
            charge(1)?;
            let node = nodes[index];
            if query.bounds.lower_gap(node.bounds) > best {
                continue;
            }
            if let Some((left, right)) = node.children {
                charge(2)?;
                let (near, far) = if query.bounds.lower_gap(nodes[left].bounds)
                    <= query.bounds.lower_gap(nodes[right].bounds)
                {
                    (left, right)
                } else {
                    (right, left)
                };
                stack.push(far);
                stack.push(near);
            } else {
                for other in &indexed[node.start..node.end] {
                    charge(1)?;
                    if query.bounds.lower_gap(other.bounds) > best {
                        continue;
                    }
                    charge(1)?;
                    best = best.min(segment_distance(query.a, query.b, other.a, other.b));
                    if finish(best) == 0.0 {
                        return Ok(0.0);
                    }
                }
            }
        }
    }
    Ok(finish(best))
}

/// Even-odd point location over one ring. Each non-horizontal edge is listed in
/// every horizontal slab its closed y-range touches, so the slab holding a query
/// holds every edge `point_in_ring` would toggle on; parity is order-free.
struct Slabs {
    lo: f64,
    hi: f64,
    scale: f64,
    starts: Vec<u32>,
    edges: Vec<u32>,
}

impl Slabs {
    fn slab(lo: f64, scale: f64, count: usize, y: f64) -> usize {
        let s = ((y - lo) * scale).floor();
        if s >= count as f64 {
            count - 1
        } else if s > 0.0 {
            s as usize
        } else {
            0
        }
    }

    fn build(ring: &[Pt], charge: &mut impl FnMut(usize) -> Result<()>) -> Result<Option<Self>> {
        let n = ring.len();
        if n < SLAB_MIN || n > u32::MAX as usize {
            return Ok(None);
        }
        charge(n)?;
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in ring {
            if !p.0.is_finite() || !p.1.is_finite() {
                return Ok(None);
            }
            lo = lo.min(p.1);
            hi = hi.max(p.1);
        }
        if hi.partial_cmp(&lo) != Some(std::cmp::Ordering::Greater) || !(hi - lo).is_finite() {
            return Ok(None);
        }
        let previous = |i: usize| if i == 0 { n - 1 } else { i - 1 };
        let mut count = (n / 4).clamp(1, 4096);
        let (scale, total) = loop {
            let scale = count as f64 / (hi - lo);
            if !scale.is_finite() {
                return Ok(None);
            }
            charge(n)?;
            let mut total = 0usize;
            for i in 0..n {
                let (a, b) = (ring[i].1, ring[previous(i)].1);
                if a != b {
                    total += Self::slab(lo, scale, count, a.max(b))
                        - Self::slab(lo, scale, count, a.min(b))
                        + 1;
                }
            }
            if total <= 4 * n + 64 || count == 1 {
                break (scale, total);
            }
            count /= 2;
        };
        charge(total.saturating_add(count).saturating_add(n))?;
        let mut starts = vec![0u32; count + 1];
        for i in 0..n {
            let (a, b) = (ring[i].1, ring[previous(i)].1);
            if a != b {
                let (first, last) = (
                    Self::slab(lo, scale, count, a.min(b)),
                    Self::slab(lo, scale, count, a.max(b)),
                );
                for s in first..=last {
                    starts[s + 1] += 1;
                }
            }
        }
        for s in 0..count {
            starts[s + 1] += starts[s];
        }
        let mut next = starts.clone();
        let mut edges = vec![0u32; total];
        for i in 0..n {
            let (a, b) = (ring[i].1, ring[previous(i)].1);
            if a != b {
                let (first, last) = (
                    Self::slab(lo, scale, count, a.min(b)),
                    Self::slab(lo, scale, count, a.max(b)),
                );
                for s in first..=last {
                    edges[next[s] as usize] = i as u32;
                    next[s] += 1;
                }
            }
        }
        Ok(Some(Self {
            lo,
            hi,
            scale,
            starts,
            edges,
        }))
    }

    fn contains(
        &self,
        ring: &[Pt],
        p: Pt,
        charge: &mut impl FnMut(usize) -> Result<()>,
    ) -> Result<bool> {
        charge(1)?;
        // Outside the ring's y-range no edge satisfies point_in_ring's crossing test.
        if p.1.partial_cmp(&self.lo) == Some(std::cmp::Ordering::Less)
            || p.1.partial_cmp(&self.hi) != Some(std::cmp::Ordering::Less)
        {
            return Ok(false);
        }
        let slab = Self::slab(self.lo, self.scale, self.starts.len() - 1, p.1);
        let listed = &self.edges[self.starts[slab] as usize..self.starts[slab + 1] as usize];
        charge(listed.len())?;
        let n = ring.len();
        let mut inside = false;
        for &i in listed {
            let i = i as usize;
            let (a, b) = (ring[i], ring[if i == 0 { n - 1 } else { i - 1 }]);
            if (a.1 > p.1) != (b.1 > p.1) {
                let x = (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0;
                if p.0 < x {
                    inside = !inside;
                }
            }
        }
        Ok(inside)
    }
}

#[derive(Clone, Copy)]
struct Bounds {
    lo: Pt,
    hi: Pt,
}

impl Bounds {
    fn edge(a: Pt, b: Pt) -> Self {
        Self {
            lo: (a.0.min(b.0), a.1.min(b.1)),
            hi: (a.0.max(b.0), a.1.max(b.1)),
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            lo: (self.lo.0.min(other.lo.0), self.lo.1.min(other.lo.1)),
            hi: (self.hi.0.max(other.hi.0), self.hi.1.max(other.hi.1)),
        }
    }

    fn centre(self, axis: usize) -> f64 {
        if axis == 0 {
            self.lo.0 / 2.0 + self.hi.0 / 2.0
        } else {
            self.lo.1 / 2.0 + self.hi.1 / 2.0
        }
    }

    /// An L-infinity box gap is a lower bound on Euclidean segment distance.
    /// Round down and retain a roundoff band, so near-equal candidates are examined
    /// by the original primitive. Extreme/nonfinite arithmetic disables pruning.
    fn lower_gap(self, other: Self) -> f64 {
        let scale = [
            self.lo.0, self.lo.1, self.hi.0, self.hi.1, other.lo.0, other.lo.1, other.hi.0,
            other.hi.1,
        ]
        .into_iter()
        .map(f64::abs)
        .fold(0.0, f64::max);
        if !scale.is_finite() || scale > 1e100 {
            return 0.0;
        }
        let gap = (other.lo.0 - self.hi.0)
            .max(self.lo.0 - other.hi.0)
            .max(other.lo.1 - self.hi.1)
            .max(self.lo.1 - other.hi.1)
            .max(0.0);
        (gap.next_down() - 64.0 * f64::EPSILON * (scale + 1.0))
            .next_down()
            .max(0.0)
    }
}

#[derive(Clone, Copy)]
struct Edge {
    a: Pt,
    b: Pt,
    bounds: Bounds,
}

fn edges(shape: &Shape, charge: &mut impl FnMut(usize) -> Result<()>) -> Result<Vec<Edge>> {
    charge(shape.pts.len())?;
    let edge = |a, b| Edge {
        a,
        b,
        bounds: Bounds::edge(a, b),
    };
    let mut result: Vec<_> = shape.pts.windows(2).map(|p| edge(p[0], p[1])).collect();
    match shape.pts.len() {
        1 => result.push(edge(shape.pts[0], shape.pts[0])),
        n if n > 2 && shape.closed => result.push(edge(shape.pts[n - 1], shape.pts[0])),
        _ => {}
    }
    Ok(result)
}

#[derive(Clone, Copy)]
struct Node {
    bounds: Bounds,
    start: usize,
    end: usize,
    children: Option<(usize, usize)>,
}

fn build(
    edges: &mut [Edge],
    scratch: &mut [Edge],
    start: usize,
    nodes: &mut Vec<Node>,
    charge: &mut impl FnMut(usize) -> Result<()>,
) -> Result<usize> {
    charge(edges.len())?;
    let bounds = edges
        .iter()
        .skip(1)
        .fold(edges[0].bounds, |b, e| b.union(e.bounds));
    charge(1)?;
    let index = nodes.len();
    nodes.push(Node {
        bounds,
        start,
        end: start + edges.len(),
        children: None,
    });
    if edges.len() > 8 {
        let axis = usize::from(bounds.hi.1 - bounds.lo.1 > bounds.hi.0 - bounds.lo.0);
        sort(edges, scratch, axis, charge)?;
        let mid = edges.len() / 2;
        let (a, b) = edges.split_at_mut(mid);
        let (sa, sb) = scratch.split_at_mut(mid);
        let left = build(a, sa, start, nodes, charge)?;
        let right = build(b, sb, start + mid, nodes, charge)?;
        nodes[index].children = Some((left, right));
    }
    Ok(index)
}

/// Fallible bottom-up mergesort: no comparator can hide a budget error. Reusing
/// the scratch slice bounds allocation; balanced splits bound BVH recursion depth.
fn sort(
    edges: &mut [Edge],
    scratch: &mut [Edge],
    axis: usize,
    charge: &mut impl FnMut(usize) -> Result<()>,
) -> Result<()> {
    let n = edges.len();
    let mut width = 1;
    while width < n {
        let mut start = 0;
        while start < n {
            let mid = start.saturating_add(width).min(n);
            let end = mid.saturating_add(width).min(n);
            let (mut a, mut b) = (start, mid);
            for output in &mut scratch[start..end] {
                let take_left = if a == mid {
                    false
                } else if b == end {
                    true
                } else {
                    charge(1)?;
                    edges[a]
                        .bounds
                        .centre(axis)
                        .total_cmp(&edges[b].bounds.centre(axis))
                        .then_with(|| {
                            edges[a]
                                .bounds
                                .centre(1 - axis)
                                .total_cmp(&edges[b].bounds.centre(1 - axis))
                        })
                        .is_le()
                };
                charge(1)?;
                *output = if take_left {
                    let e = edges[a];
                    a += 1;
                    e
                } else {
                    let e = edges[b];
                    b += 1;
                    e
                };
            }
            start = end;
        }
        charge(n)?;
        edges.copy_from_slice(&scratch[..n]);
        width = width.saturating_mul(2);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured(a: &Shape, b: &Shape, limit: usize) -> (Result<f64>, usize) {
        let mut spent = 0usize;
        let result = distance(a, b, &mut |work| {
            anyhow::ensure!(
                work <= limit.saturating_sub(spent),
                "work allowance exhausted"
            );
            spent += work;
            Ok(())
        });
        (result, spent)
    }

    fn assert_same(a: &Shape, b: &Shape) {
        let expected = a.distance(b);
        let (actual, _) = measured(a, b, 10_000_000);
        let actual = actual.unwrap();
        assert!(
            actual == expected || (actual - expected).abs() <= 1e-12 * expected.abs().max(1.0),
            "expected {expected}, got {actual}; a={a:?}; b={b:?}"
        );
    }

    // Catches lost stroke radii, omitted closing edges, containment and degenerate edges.
    #[test]
    fn literal_point_path_polygon_and_stroke_distances() {
        let square = Shape::polygon(vec![(0., 0.), (4., 0.), (4., 4.), (0., 4.)]);
        let cases = [
            (Shape::disc((0., 0.), 1.), Shape::disc((5., 0.), 1.), 3.),
            (Shape::disc((2., 2.), 0.), square.clone(), 0.),
            (Shape::disc((6., 2.), 0.5), square.clone(), 1.5),
            (
                Shape::path(vec![(0., 0.), (4., 0.)], 0.5),
                Shape::disc((2., 2.), 0.5),
                1.,
            ),
            (
                Shape::path(vec![(0., 0.), (4., 4.)], 0.),
                Shape::path(vec![(0., 4.), (4., 0.)], 0.),
                0.,
            ),
            (
                Shape::ring(vec![(0., 0.), (4., 0.), (4., 4.)], 0.),
                Shape::disc((1., 1.), 0.),
                0.,
            ),
            (
                Shape::path(vec![(1., 1.), (1., 1.)], 0.25),
                Shape::disc((1., 3.), 0.25),
                1.5,
            ),
            (Shape::path(Vec::new(), 0.), square, f64::INFINITY),
        ];
        for (a, b, want) in cases {
            for (a, b) in [(&a, &b), (&b, &a)] {
                let (actual, _) = measured(a, b, 100_000);
                assert_eq!(actual.unwrap(), want);
            }
        }
    }

    fn dense_l(parts: usize) -> Shape {
        let corners = [
            (0., 0.),
            (10., 0.),
            (10., 1.),
            (1., 1.),
            (1., 10.),
            (0., 10.),
        ];
        let mut points = Vec::with_capacity(parts * corners.len());
        for i in 0..corners.len() {
            let (a, b) = (corners[i], corners[(i + 1) % corners.len()]);
            for j in 0..parts {
                let t = j as f64 / parts as f64;
                points.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
            }
        }
        Shape::polygon(points)
    }

    fn dense_circle(parts: usize) -> Shape {
        Shape::polygon(
            (0..parts)
                .map(|i| {
                    let angle = std::f64::consts::TAU * i as f64 / parts as f64;
                    (6. + 2. * angle.cos(), 6. + 2. * angle.sin())
                })
                .collect(),
        )
    }

    // Catches a return to exhaustive edge pairs: 6000 * 4000 exceeds the allowance.
    #[test]
    fn dense_concave_fill_and_nested_bbox_circle_finish_below_ten_million_work() {
        let a = dense_l(1000);
        let b = dense_circle(4000);
        for (a, b) in [(&a, &b), (&b, &a)] {
            let (result, spent) = measured(a, b, 10_000_000);
            let actual =
                result.expect("disjoint dense fills must finish within the work allowance");
            assert!((actual - 3.0).abs() <= 1e-12, "gap={actual}");
            eprintln!("dense concave/circle work: {spent}");
        }
    }

    // Catches an unchecked/ignored budget callback or returning a partial minimum as success.
    #[test]
    fn budget_errors_propagate_without_any_followup_charge() {
        let a = dense_l(20);
        let b = dense_circle(80);
        for fail_at in [0, 1, 2, 8, 64, 256, 1024] {
            let mut calls = 0;
            let result = distance(&a, &b, &mut |_| {
                let current = calls;
                calls += 1;
                if current >= fail_at {
                    anyhow::bail!("sentinel budget refusal");
                }
                Ok(())
            });
            assert_eq!(result.unwrap_err().to_string(), "sentinel budget refusal");
            assert_eq!(calls, fail_at + 1, "work continued after refusal");
        }
    }

    // The independent exhaustive oracle catches pruning, segment enumeration and winding mistakes.
    #[test]
    fn matches_exhaustive_shapes_including_closed_degenerate_and_self_intersecting_paths() {
        let mut seed = 0x8a5c_470d_e31f_92b6u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 32) as u32
        };
        for _ in 0..800 {
            let mut shape = || {
                let count = (next() % 18) as usize;
                let pts = (0..count)
                    .map(|_| {
                        (
                            (next() % 129) as f64 / 8. - 8.,
                            (next() % 129) as f64 / 8. - 8.,
                        )
                    })
                    .collect();
                Shape {
                    pts,
                    closed: next().is_multiple_of(2),
                    filled: next().is_multiple_of(3),
                    radius: (next() % 5) as f64 / 8.,
                }
            };
            let (a, b) = (shape(), shape());
            assert_same(&a, &b);
            assert_same(&b, &a);
        }
    }

    // Catches treating a fractured fill bridge as a filled hole or skipping containment.
    #[test]
    fn bridged_hole_and_boundary_tangency_follow_existing_ring_semantics() {
        let fill = Shape::polygon(vec![
            (2., 2.),
            (18., 2.),
            (18., 18.),
            (10., 18.),
            (10., 11.),
            (11., 11.),
            (11., 9.),
            (9., 9.),
            (9., 11.),
            (10., 11.),
            (10., 18.),
            (2., 18.),
        ]);
        for disc in [
            Shape::disc((10., 10.), 0.3),
            Shape::disc((8.8, 10.), 0.3),
            Shape::disc((8.6, 10.), 0.3),
            Shape::disc((10., 14.), 0.),
        ] {
            assert_same(&fill, &disc);
            assert_same(&disc, &fill);
        }
        for x in [2_f64.next_down(), 2., 2_f64.next_up()] {
            assert_same(&Shape::disc((0., 0.), 1.), &Shape::disc((x, 0.), 1.));
        }
    }

    // Catches uncharged containment, including the early return without constructing edges.
    #[test]
    fn containment_consumes_its_edge_allowance_before_returning_overlap() {
        let fill = dense_l(100);
        let point = Shape::disc((0.5, 5.0), 0.0);
        assert!(measured(&fill, &point, fill.pts.len() - 1).0.is_err());
        assert_eq!(measured(&fill, &point, fill.pts.len()).0.unwrap(), 0.0);
    }

    // Catches doing tree work after the first exact primitive already proves zero distance.
    #[test]
    fn a_proven_overlap_short_circuits_before_tree_building() {
        let mut a = vec![(0.0, 0.0), (2.0, 2.0)];
        let mut b = vec![(0.0, 2.0), (2.0, 0.0)];
        for i in 2..1000 {
            a.push((i as f64, 2.0));
            b.push((i as f64, 0.0));
        }
        let (result, spent) = measured(&Shape::path(a, 0.0), &Shape::path(b, 0.0), 2001);
        assert_eq!(result.unwrap(), 0.0);
        assert_eq!(spent, 2001);
    }

    // Forces BVH searches on disjoint shapes; tiny/huge cases exercise conservative pruning.
    #[test]
    fn disjoint_bvh_queries_match_exhaustive_at_different_scales_and_translations() {
        let a = dense_l(12);
        let b = dense_circle(64);
        for (scale, offset) in [
            (1.0, 0.0),
            (0.000001, 2000.0),
            (1.0, 1e9),
            (1e-120, 0.0),
            (1e140, 0.0),
        ] {
            let transform = |shape: &Shape| Shape {
                pts: shape
                    .pts
                    .iter()
                    .map(|p| (p.0 * scale + offset, p.1 * scale - offset))
                    .collect(),
                closed: shape.closed,
                filled: false,
                radius: 0.0,
            };
            let (a, b) = (transform(&a), transform(&b));
            assert_same(&a, &b);
            assert_same(&b, &a);
        }
    }

    fn unlimited() -> impl FnMut(usize) -> Result<()> {
        |_| Ok(())
    }

    fn random_shape(next: &mut impl FnMut() -> u32, most: u32) -> Shape {
        let count = (next() % most) as usize;
        let pts = (0..count)
            .map(|_| {
                (
                    (next() % 129) as f64 / 8. - 8.,
                    (next() % 129) as f64 / 8. - 8.,
                )
            })
            .collect();
        Shape {
            pts,
            closed: next().is_multiple_of(2),
            filled: next().is_multiple_of(3),
            radius: (next() % 5) as f64 / 8.,
        }
    }

    // A cached tree or point index that drops, repeats or misorders an edge, or
    // prunes a nearer one, disagrees with the exhaustive distance somewhere here.
    #[test]
    fn prepared_shapes_match_the_exhaustive_oracle_in_every_pairing() {
        let mut seed = 0x51f1_5e3a_9c0d_77b1u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 32) as u32
        };
        let (mut trees, mut indexes) = (0, 0);
        for round in 0..400 {
            // Large enough for trees and slab indexes, small enough to overlap often.
            let most = if round % 2 == 0 { 90 } else { 18 };
            let (a, b) = (random_shape(&mut next, most), random_shape(&mut next, 90));
            let (pa, pb) = (
                prepare(&a, &mut unlimited()).unwrap(),
                prepare(&b, &mut unlimited()).unwrap(),
            );
            trees += usize::from(!pa.nodes.is_empty()) + usize::from(!pb.nodes.is_empty());
            indexes += usize::from(pa.slabs.is_some()) + usize::from(pb.slabs.is_some());
            let expected = a.distance(&b);
            for (x, y) in [
                (None, None),
                (Some(&pa), None),
                (None, Some(&pb)),
                (Some(&pa), Some(&pb)),
            ] {
                let actual = distance_with(&a, x, &b, y, f64::INFINITY, &mut unlimited()).unwrap();
                assert!(
                    actual == expected
                        || (actual - expected).abs() <= 1e-12 * expected.abs().max(1.0),
                    "expected {expected}, got {actual}; a={a:?}; b={b:?}"
                );
            }
        }
        assert!(
            trees > 300 && indexes > 60,
            "the random shapes must exercise both caches: {trees} trees, {indexes} point indexes"
        );
    }

    // The slab index must answer exactly as point_in_ring does, including on
    // vertices, horizontal edges, repeated points and KiCad's bridged holes.
    #[test]
    fn slab_point_location_equals_point_in_ring_on_and_off_the_boundary() {
        let mut seed = 0x0dd5_ba11_c0ff_ee42u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 32) as u32
        };
        let mut rings: Vec<Vec<Pt>> = vec![dense_l(40).pts, dense_circle(500).pts];
        // A bridged hole, densified so the index engages.
        let bridged = [
            (2., 2.),
            (18., 2.),
            (18., 18.),
            (2., 18.),
            (2., 2.),
            (9., 9.),
            (9., 11.),
            (11., 11.),
            (11., 9.),
            (9., 9.),
            (2., 2.),
        ];
        let mut dense = Vec::new();
        for w in bridged.windows(2) {
            for k in 0..8 {
                let t = k as f64 / 8.;
                dense.push((
                    w[0].0 + t * (w[1].0 - w[0].0),
                    w[0].1 + t * (w[1].1 - w[0].1),
                ));
            }
        }
        rings.push(dense);
        for _ in 0..60 {
            let count = 32 + (next() % 200) as usize;
            // A coarse lattice forces shared heights, horizontal edges and repeats.
            rings.push(
                (0..count)
                    .map(|_| ((next() % 33) as f64 / 2., (next() % 17) as f64 / 2.))
                    .collect(),
            );
        }
        for ring in rings {
            let shape = Shape::polygon(ring.clone());
            let prepared = prepare(&shape, &mut unlimited()).unwrap();
            assert!(
                prepared.slabs.is_some(),
                "ring of {} must be indexed",
                ring.len()
            );
            let mut points: Vec<Pt> = ring.clone();
            for w in ring.windows(2) {
                points.push(((w[0].0 + w[1].0) / 2., (w[0].1 + w[1].1) / 2.));
            }
            for _ in 0..400 {
                points.push((
                    (next() % 400) as f64 / 20. - 1.,
                    (next() % 400) as f64 / 20. - 1.,
                ));
            }
            for y in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e300, 1e300] {
                points.push((5., y));
                points.push((y, 5.));
            }
            for point in points {
                assert_eq!(
                    contains(&shape, Some(&prepared), point, &mut unlimited()).unwrap(),
                    point_in_ring(point, &ring),
                    "point {point:?} in a ring of {}",
                    ring.len()
                );
            }
        }
    }

    /// A ring whose every edge spans the whole height would fill every slab
    /// with every edge. The index must stay linear in the ring's size.
    #[test]
    fn slab_index_stays_linear_when_every_edge_spans_the_whole_height() {
        let n = 2000;
        let zigzag = Shape::polygon(
            (0..n)
                .map(|i| (i as f64 * 0.01, if i % 2 == 0 { 0. } else { 100. }))
                .collect(),
        );
        let prepared = prepare(&zigzag, &mut |_| Ok(())).unwrap();
        let slabs = prepared.slabs.as_ref().expect("a large ring is indexed");
        assert!(
            slabs.edges.len() <= 4 * n + 64,
            "{} entries",
            slabs.edges.len()
        );
        for p in [
            (5.003, 50.),
            (-1., 50.),
            (10., 0.5),
            (19.985, 99.),
            (7.5, 100.),
        ] {
            assert_eq!(
                slabs.contains(&zigzag.pts, p, &mut |_| Ok(())).unwrap(),
                point_in_ring(p, &zigzag.pts),
                "{p:?}"
            );
        }
    }

    // Catches losing the cache: a query that re-walks or re-indexes the fill
    // costs thousands of units; a reused tree and slab index cost a few dozen.
    #[test]
    fn a_prepared_fill_answers_small_queries_without_rescanning_its_edges() {
        let fill = dense_circle(4000);
        let mut build = 0usize;
        let prepared = prepare(&fill, &mut |work| {
            build += work;
            Ok(())
        })
        .unwrap();
        assert!(build >= 4000, "building is charged: {build}");
        for (centre, inside) in [((6.3, 6.4), true), ((6.5, 7.), true), ((12., 6.), false)] {
            let via = Shape::disc(centre, 0.3);
            let radius = (centre.0 - 6f64).hypot(centre.1 - 6.);
            let expected = (2. - radius).abs() - 0.3;
            // Unprepared, each query walks all 4000 edges twice. From the tree a
            // full exact answer costs less than one walk, a capped one almost nothing.
            for (cap, most) in [(f64::INFINITY, 4000), (0.011, 80)] {
                let mut spent = 0usize;
                let mut charge = |work: usize| {
                    spent += work;
                    Ok(())
                };
                assert_eq!(
                    contains(&fill, Some(&prepared), centre, &mut charge).unwrap(),
                    inside
                );
                let gap =
                    boundary_gap(&fill, Some(&prepared), &via, None, cap, &mut charge).unwrap();
                if cap.is_infinite() {
                    assert!((gap - expected).abs() < 1e-5, "gap {gap} at {centre:?}");
                } else {
                    assert!(gap >= cap - 1e-12, "capped gap {gap} at {centre:?}");
                }
                assert!(
                    spent < most,
                    "query at {centre:?} with cap {cap} cost {spent} units"
                );
            }
        }
    }

    // A cap may skip distant edges, never a nearer one: below the cap the
    // answer is the exhaustive distance, and at the cap nothing is nearer.
    #[test]
    fn a_distance_cap_never_hides_a_nearer_edge() {
        let mut seed = 0x7a11_0c4b_95e3_d2f6u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 32) as u32
        };
        let (mut exact, mut capped) = (0, 0);
        for _ in 0..300 {
            let (a, b) = (random_shape(&mut next, 90), random_shape(&mut next, 90));
            let (pa, pb) = (
                prepare(&a, &mut unlimited()).unwrap(),
                prepare(&b, &mut unlimited()).unwrap(),
            );
            let expected = a.distance(&b);
            for cap in [0.0, 0.01, 0.21, 1.0, 4.0, f64::NAN, -1.0] {
                for (x, y) in [(None, None), (Some(&pa), None), (Some(&pa), Some(&pb))] {
                    let actual = distance_with(&a, x, &b, y, cap, &mut unlimited()).unwrap();
                    let tolerance = 1e-12 * expected.abs().max(1.0);
                    if expected < cap || cap.is_nan() || cap < 0.0 {
                        exact += 1;
                        assert!(
                            (actual - expected).abs() <= tolerance || actual == expected,
                            "cap {cap}: expected {expected}, got {actual}; a={a:?}; b={b:?}"
                        );
                    } else {
                        capped += 1;
                        assert!(
                            actual >= cap - tolerance && actual <= expected + tolerance,
                            "cap {cap}: {actual} hides or invents copper; true {expected}"
                        );
                    }
                }
            }
        }
        assert!(
            exact > 1000 && capped > 500,
            "{exact} exact, {capped} capped"
        );
    }

    // Catches an index that swallows a refusal and returns a half-built cache.
    #[test]
    fn a_refused_charge_never_yields_a_partial_index_or_answer() {
        let fill = dense_circle(600);
        let mut total = 0usize;
        let prepared = prepare(&fill, &mut |work| {
            total += work;
            Ok(())
        })
        .unwrap();
        for limit in [0, 1, 599, 600, 601, total / 2, total - 1] {
            let mut spent = 0usize;
            let refused = prepare(&fill, &mut |work| {
                anyhow::ensure!(work <= limit.saturating_sub(spent), "sentinel");
                spent += work;
                Ok(())
            });
            assert!(
                refused.is_err(),
                "allowance {limit} of {total} must not build"
            );
        }
        let mut calls = 0;
        assert!(contains(&fill, Some(&prepared), (6., 6.), &mut |_| {
            calls += 1;
            anyhow::bail!("sentinel")
        })
        .is_err());
        assert_eq!(calls, 1, "work continued after the refusal");
    }
}
