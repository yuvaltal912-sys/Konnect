//! A raster of one net's copper, per layer, for the layout-quality audit.
//!
//! Connectivity, copper area, shortest copper paths (decoupling and hot
//! loops) and the narrowest section on a path (bottlenecks) all come from the
//! same picture of the copper, so they cannot disagree with each other. The
//! price is resolution: every length is good to about one cell, and the grid
//! says which step it used.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use konnect_sexp::board_geometry::Shape;
use konnect_sexp::planar::Pt;

/// Cells in one layer at most; the step grows to stay under it.
const MAX_CELLS_PER_LAYER: f64 = 2_500_000.0;
const NO_LINK: u8 = u8::MAX;

/// How a shape marks cells.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cover {
    /// Cells whose centre is inside the shape (widths stay true).
    Centre,
    /// Every cell the shape touches, a half step more (connectivity).
    Touch,
}

pub struct Grid {
    pub x0: f64,
    pub y0: f64,
    pub step: f64,
    pub nx: usize,
    pub ny: usize,
    /// Copper layer names in stack order.
    pub layers: Vec<String>,
    /// One bitmap per layer, row-major.
    pub copper: Vec<Vec<bool>>,
    /// Per cell, the lowest and highest layer index a via or through pad joins
    /// there (`NO_LINK` when none).
    link_lo: Vec<u8>,
    link_hi: Vec<u8>,
}

pub type Cell = (usize, usize, usize); // layer, ix, iy

impl Grid {
    /// A grid over `bbox` (min x, min y, max x, max y), margin included.
    pub fn new(bbox: (f64, f64, f64, f64), step_hint: f64, layers: Vec<String>) -> Grid {
        let (x0, y0, x1, y1) = bbox;
        let (w, h) = ((x1 - x0).max(step_hint), (y1 - y0).max(step_hint));
        let step = step_hint.max(((w * h) / MAX_CELLS_PER_LAYER).sqrt());
        let nx = (w / step).ceil() as usize + 1;
        let ny = (h / step).ceil() as usize + 1;
        let n = nx * ny;
        Grid {
            x0,
            y0,
            step,
            nx,
            ny,
            copper: vec![vec![false; n]; layers.len()],
            layers,
            link_lo: vec![NO_LINK; n],
            link_hi: vec![NO_LINK; n],
        }
    }

    pub fn idx(&self, ix: usize, iy: usize) -> usize {
        iy * self.nx + ix
    }

    pub fn centre(&self, ix: usize, iy: usize) -> Pt {
        (
            self.x0 + (ix as f64 + 0.5) * self.step,
            self.y0 + (iy as f64 + 0.5) * self.step,
        )
    }

    pub fn cell_of(&self, p: Pt) -> Option<(usize, usize)> {
        let fx = (p.0 - self.x0) / self.step;
        let fy = (p.1 - self.y0) / self.step;
        if fx < 0.0 || fy < 0.0 {
            return None;
        }
        let (ix, iy) = (fx as usize, fy as usize);
        (ix < self.nx && iy < self.ny).then_some((ix, iy))
    }

    fn mark(&mut self, layer: usize, ix: usize, iy: usize) {
        let i = self.idx(ix, iy);
        self.copper[layer][i] = true;
    }

    /// Mark a stroked polyline or a disc.
    pub fn add_path(&mut self, layer: usize, pts: &[Pt], half_width: f64, cover: Cover) {
        let reach = half_width
            + match cover {
                Cover::Centre => 0.0,
                Cover::Touch => self.step * 0.75,
            };
        if pts.len() == 1 {
            self.add_disc(layer, pts[0], reach);
            return;
        }
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (minx, maxx) = (a.0.min(b.0) - reach, a.0.max(b.0) + reach);
            let (miny, maxy) = (a.1.min(b.1) - reach, a.1.max(b.1) + reach);
            let (ix0, ix1, iy0, iy1) = self.cell_range(minx, miny, maxx, maxy);
            for iy in iy0..=iy1 {
                for ix in ix0..=ix1 {
                    let c = self.centre(ix, iy);
                    if konnect_sexp::planar::point_segment_distance(c, a, b) <= reach {
                        self.mark(layer, ix, iy);
                    }
                }
            }
        }
    }

    pub fn add_disc(&mut self, layer: usize, c: Pt, r: f64) {
        let (ix0, ix1, iy0, iy1) = self.cell_range(c.0 - r, c.1 - r, c.0 + r, c.1 + r);
        for iy in iy0..=iy1 {
            for ix in ix0..=ix1 {
                let p = self.centre(ix, iy);
                if (p.0 - c.0).hypot(p.1 - c.1) <= r {
                    self.mark(layer, ix, iy);
                }
            }
        }
    }

    fn cell_range(&self, minx: f64, miny: f64, maxx: f64, maxy: f64) -> (usize, usize, usize, usize) {
        let clamp = |v: f64, n: usize| (v.floor().max(0.0) as usize).min(n - 1);
        (
            clamp((minx - self.x0) / self.step, self.nx),
            clamp((maxx - self.x0) / self.step, self.nx),
            clamp((miny - self.y0) / self.step, self.ny),
            clamp((maxy - self.y0) / self.step, self.ny),
        )
    }

    /// Mark the inside of a ring (even-odd, so a fractured fill with its
    /// bridged holes comes out right).
    pub fn add_ring(&mut self, layer: usize, ring: &[Pt]) {
        if ring.len() < 3 {
            return;
        }
        let (minx, miny, maxx, maxy) = match konnect_sexp::planar::bbox_of(ring) {
            Some(b) => b,
            None => return,
        };
        let (_, _, iy0, iy1) = self.cell_range(minx, miny, maxx, maxy);
        let n = ring.len();
        let mut xs: Vec<f64> = Vec::new();
        for iy in iy0..=iy1 {
            let y = self.y0 + (iy as f64 + 0.5) * self.step;
            xs.clear();
            for i in 0..n {
                let (p, q) = (ring[i], ring[(i + 1) % n]);
                if (p.1 <= y) != (q.1 <= y) {
                    xs.push(p.0 + (y - p.1) / (q.1 - p.1) * (q.0 - p.0));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            for pair in xs.chunks(2) {
                if let [a, b] = pair {
                    let lo_f = ((a - self.x0) / self.step - 0.5).ceil().max(0.0);
                    let hi_f = ((b - self.x0) / self.step - 0.5).floor();
                    if hi_f < lo_f || lo_f >= self.nx as f64 {
                        continue;
                    }
                    let hi = (hi_f as usize).min(self.nx - 1);
                    for ix in (lo_f as usize)..=hi {
                        self.mark(layer, ix, iy);
                    }
                }
            }
        }
    }

    /// Mark a pad or other shape.
    pub fn add_shape(&mut self, layer: usize, shape: &Shape, cover: Cover) {
        if shape.filled && shape.closed && shape.pts.len() >= 3 {
            self.add_ring(layer, &shape.pts);
            if cover == Cover::Touch {
                let mut ring = shape.pts.clone();
                ring.push(ring[0]);
                self.add_path(layer, &ring, self.step * 0.75, Cover::Centre);
            }
        } else {
            self.add_path(layer, &shape.pts, shape.radius, cover);
        }
    }

    /// A via or through pad at `c` joining layers `lo..=hi` over radius `r`.
    pub fn add_link(&mut self, c: Pt, r: f64, lo: usize, hi: usize) {
        let (ix0, ix1, iy0, iy1) = self.cell_range(c.0 - r, c.1 - r, c.0 + r, c.1 + r);
        let reach = r.max(self.step * 0.75);
        for iy in iy0..=iy1 {
            for ix in ix0..=ix1 {
                let p = self.centre(ix, iy);
                if (p.0 - c.0).hypot(p.1 - c.1) <= reach {
                    let i = self.idx(ix, iy);
                    self.link_lo[i] = self.link_lo[i].min(lo as u8);
                    self.link_hi[i] = if self.link_hi[i] == NO_LINK {
                        hi as u8
                    } else {
                        self.link_hi[i].max(hi as u8)
                    };
                    for l in lo..=hi {
                        self.mark(l, ix, iy);
                    }
                }
            }
        }
    }

    pub fn is_copper(&self, layer: usize, ix: usize, iy: usize) -> bool {
        self.copper[layer][self.idx(ix, iy)]
    }

    /// Copper area over all layers, mm².
    pub fn area_mm2(&self) -> f64 {
        let cells: usize = self
            .copper
            .iter()
            .map(|l| l.iter().filter(|c| **c).count())
            .sum();
        cells as f64 * self.step * self.step
    }

    /// Cells of `shape` that are copper on `layers`, for use as path ends.
    pub fn cells_under(&self, shape: &Shape, layers: &[usize]) -> Vec<Cell> {
        let Some((minx, miny, maxx, maxy)) = shape.bbox() else {
            return Vec::new();
        };
        let (ix0, ix1, iy0, iy1) = self.cell_range(minx, miny, maxx, maxy);
        let mut out = Vec::new();
        for &l in layers {
            for iy in iy0..=iy1 {
                for ix in ix0..=ix1 {
                    if !self.is_copper(l, ix, iy) {
                        continue;
                    }
                    let c = self.centre(ix, iy);
                    let inside = if shape.filled && shape.closed {
                        konnect_sexp::planar::point_in_ring(c, &shape.pts)
                    } else {
                        shape
                            .pts
                            .windows(2)
                            .map(|w| konnect_sexp::planar::point_segment_distance(c, w[0], w[1]))
                            .chain(shape.pts.first().map(|p| (c.0 - p.0).hypot(c.1 - p.1)))
                            .fold(f64::INFINITY, f64::min)
                            <= shape.radius + self.step * 0.75
                    };
                    if inside {
                        out.push((l, ix, iy));
                    }
                }
            }
        }
        out
    }

    /// Connected copper groups: a label per copper cell, 0 for none.
    pub fn components(&self) -> (Vec<Vec<u32>>, u32) {
        let n = self.nx * self.ny;
        let mut labels = vec![vec![0u32; n]; self.layers.len()];
        let mut next = 0u32;
        let mut stack: Vec<Cell> = Vec::new();
        for l in 0..self.layers.len() {
            for iy in 0..self.ny {
                for ix in 0..self.nx {
                    let i = self.idx(ix, iy);
                    if !self.copper[l][i] || labels[l][i] != 0 {
                        continue;
                    }
                    next += 1;
                    labels[l][i] = next;
                    stack.push((l, ix, iy));
                    while let Some((cl, cx, cy)) = stack.pop() {
                        for (nl, nx, ny) in self.neighbours4(cl, cx, cy) {
                            let j = self.idx(nx, ny);
                            if self.copper[nl][j] && labels[nl][j] == 0 {
                                labels[nl][j] = next;
                                stack.push((nl, nx, ny));
                            }
                        }
                    }
                }
            }
        }
        (labels, next)
    }

    fn neighbours4(&self, l: usize, ix: usize, iy: usize) -> Vec<Cell> {
        let mut out = Vec::with_capacity(8);
        if ix > 0 {
            out.push((l, ix - 1, iy));
        }
        if ix + 1 < self.nx {
            out.push((l, ix + 1, iy));
        }
        if iy > 0 {
            out.push((l, ix, iy - 1));
        }
        if iy + 1 < self.ny {
            out.push((l, ix, iy + 1));
        }
        let i = self.idx(ix, iy);
        if self.link_lo[i] != NO_LINK {
            let (lo, hi) = (self.link_lo[i] as usize, self.link_hi[i] as usize);
            if (lo..=hi).contains(&l) {
                for k in lo..=hi {
                    if k != l {
                        out.push((k, ix, iy));
                    }
                }
            }
        }
        out
    }

    /// Distance, in mm, from each copper cell to the nearest empty cell
    /// (chamfer transform, within a few percent). Empty cells are 0.
    pub fn distance_transform(&self) -> Vec<Vec<f32>> {
        let (nx, ny) = (self.nx, self.ny);
        let big = 1.0e6f32;
        let (a, b) = (self.step as f32, (self.step * std::f64::consts::SQRT_2) as f32);
        let mut out = Vec::with_capacity(self.layers.len());
        for l in 0..self.layers.len() {
            let mut d: Vec<f32> = self.copper[l]
                .iter()
                .map(|c| if *c { big } else { 0.0 })
                .collect();
            // Cells at the grid edge border empty space.
            for iy in 0..ny {
                for ix in 0..nx {
                    if ix == 0 || iy == 0 || ix + 1 == nx || iy + 1 == ny {
                        let i = iy * nx + ix;
                        if d[i] > a {
                            d[i] = a;
                        }
                    }
                }
            }
            for iy in 0..ny {
                for ix in 0..nx {
                    let i = iy * nx + ix;
                    if d[i] == 0.0 {
                        continue;
                    }
                    let mut v = d[i];
                    if ix > 0 {
                        v = v.min(d[i - 1] + a);
                    }
                    if iy > 0 {
                        v = v.min(d[i - nx] + a);
                        if ix > 0 {
                            v = v.min(d[i - nx - 1] + b);
                        }
                        if ix + 1 < nx {
                            v = v.min(d[i - nx + 1] + b);
                        }
                    }
                    d[i] = v;
                }
            }
            for iy in (0..ny).rev() {
                for ix in (0..nx).rev() {
                    let i = iy * nx + ix;
                    if d[i] == 0.0 {
                        continue;
                    }
                    let mut v = d[i];
                    if ix + 1 < nx {
                        v = v.min(d[i + 1] + a);
                    }
                    if iy + 1 < ny {
                        v = v.min(d[i + nx] + a);
                        if ix + 1 < nx {
                            v = v.min(d[i + nx + 1] + b);
                        }
                        if ix > 0 {
                            v = v.min(d[i + nx - 1] + b);
                        }
                    }
                    d[i] = v;
                }
            }
            out.push(d);
        }
        out
    }
}

// ─── Shortest copper path ────────────────────────────────────────────────────

#[derive(PartialEq)]
struct Item {
    cost: f64,
    cell: Cell,
}
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Item {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap on cost.
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
    }
}

/// The shortest path through copper from any of `from` to any of `to`.
pub struct ShortestPath {
    pub length_mm: f64,
    pub vias: usize,
    /// Cell centres along the path, source first.
    pub points: Vec<Pt>,
}

/// `via_cost_mm` is added for every layer change.
pub fn shortest_path(
    grid: &Grid,
    from: &[Cell],
    to: &[Cell],
    via_cost_mm: f64,
) -> Option<ShortestPath> {
    if from.is_empty() || to.is_empty() {
        return None;
    }
    let n = grid.nx * grid.ny;
    let layers = grid.layers.len();
    let mut dist = vec![f64::INFINITY; n * layers];
    let mut prev: Vec<u32> = vec![u32::MAX; n * layers];
    let key = |c: Cell| c.0 * n + grid.idx(c.1, c.2);
    let mut target = vec![false; n * layers];
    for &t in to {
        target[key(t)] = true;
    }
    let mut heap = BinaryHeap::new();
    for &s in from {
        dist[key(s)] = 0.0;
        heap.push(Item { cost: 0.0, cell: s });
    }
    let diag = grid.step * std::f64::consts::SQRT_2;
    let mut reached: Option<Cell> = None;
    while let Some(Item { cost, cell }) = heap.pop() {
        let (l, ix, iy) = cell;
        if cost > dist[key(cell)] {
            continue;
        }
        if target[key(cell)] {
            reached = Some(cell);
            break;
        }
        let mut try_move = |nl: usize, nx: usize, ny: usize, step: f64, heap: &mut BinaryHeap<Item>| {
            if !grid.is_copper(nl, nx, ny) {
                return;
            }
            let c = (nl, nx, ny);
            let nc = cost + step;
            if nc < dist[key(c)] {
                dist[key(c)] = nc;
                prev[key(c)] = key(cell) as u32;
                heap.push(Item { cost: nc, cell: c });
            }
        };
        for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let (nx, ny) = (ix as i64 + dx, iy as i64 + dy);
                if nx < 0 || ny < 0 || nx >= grid.nx as i64 || ny >= grid.ny as i64 {
                    continue;
                }
                let step = if dx != 0 && dy != 0 { diag } else { grid.step };
                try_move(l, nx as usize, ny as usize, step, &mut heap);
            }
        }
        let i = grid.idx(ix, iy);
        if grid.link_lo[i] != NO_LINK {
            let (lo, hi) = (grid.link_lo[i] as usize, grid.link_hi[i] as usize);
            if (lo..=hi).contains(&l) {
                for k in lo..=hi {
                    if k != l {
                        try_move(k, ix, iy, via_cost_mm, &mut heap);
                    }
                }
            }
        }
    }
    let end = reached?;
    let mut points = Vec::new();
    let mut vias = 0;
    let mut k = key(end);
    let mut last_layer = end.0;
    loop {
        let (l, i) = (k / n, k % n);
        points.push(grid.centre(i % grid.nx, i / grid.nx));
        if l != last_layer {
            vias += 1;
            last_layer = l;
        }
        if prev[k] == u32::MAX {
            break;
        }
        k = prev[k] as usize;
    }
    points.reverse();
    Some(ShortestPath {
        length_mm: dist[key(end)],
        vias,
        points,
    })
}

// ─── Widest copper path ──────────────────────────────────────────────────────

/// The narrowest section on the best path.
pub struct WidestPath {
    /// Local width at that cell, mm.
    pub width_mm: f64,
    pub at: Pt,
    pub layer: usize,
}

/// The path from `from` to `to` whose narrowest point, measured as local
/// width over `required_mm[layer]`, is widest. Cells in `exempt` (the end
/// pads) do not limit the path. `dt` is [`Grid::distance_transform`].
pub fn widest_path(
    grid: &Grid,
    dt: &[Vec<f32>],
    from: &[Cell],
    to: &[Cell],
    required_mm: &[f64],
    exempt: &[Cell],
) -> Option<WidestPath> {
    if from.is_empty() || to.is_empty() {
        return None;
    }
    let n = grid.nx * grid.ny;
    let layers = grid.layers.len();
    let key = |c: Cell| c.0 * n + grid.idx(c.1, c.2);
    let mut skip = vec![false; n * layers];
    for &e in exempt.iter().chain(from).chain(to) {
        skip[key(e)] = true;
    }
    let mut target = vec![false; n * layers];
    for &t in to {
        target[key(t)] = true;
    }
    let local = |c: Cell| -> (f64, f64) {
        if skip[key(c)] {
            return (f64::INFINITY, f64::INFINITY);
        }
        let d = dt[c.0][grid.idx(c.1, c.2)] as f64;
        let width = (2.0 * d - grid.step).max(0.0);
        (width / required_mm[c.0].max(1e-9), width)
    };
    // Best (ratio, width of the limiting cell, its cell) reaching each node.
    let mut best = vec![f64::NEG_INFINITY; n * layers];
    let mut at: Vec<(f64, Cell)> = vec![(f64::INFINITY, (0, 0, 0)); n * layers];
    // Heap order: the better path first, then the cell farther from the copper edge, so ties
    // follow the middle of the copper and the reported neck is where it really is.
    #[derive(PartialEq)]
    struct Max(f64, f64, Cell);
    impl Eq for Max {}
    impl PartialOrd for Max {
        fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
            Some(self.cmp(o))
        }
    }
    impl Ord for Max {
        fn cmp(&self, o: &Self) -> Ordering {
            self.0
                .partial_cmp(&o.0)
                .unwrap_or(Ordering::Equal)
                .then(self.1.partial_cmp(&o.1).unwrap_or(Ordering::Equal))
        }
    }
    let mut heap = BinaryHeap::new();
    for &s in from {
        best[key(s)] = f64::INFINITY;
        heap.push(Max(f64::INFINITY, f64::INFINITY, s));
    }
    let mut reached: Option<Cell> = None;
    while let Some(Max(r, _, cell)) = heap.pop() {
        if r < best[key(cell)] {
            continue;
        }
        if target[key(cell)] {
            reached = Some(cell);
            break;
        }
        for (nl, nx, ny) in grid.neighbours4(cell.0, cell.1, cell.2) {
            if !grid.is_copper(nl, nx, ny) {
                continue;
            }
            let c = (nl, nx, ny);
            let (ratio, width) = local(c);
            let through = r.min(ratio);
            if through > best[key(c)] {
                best[key(c)] = through;
                at[key(c)] = if ratio < r { (width, c) } else { at[key(cell)] };
                heap.push(Max(through, dt[nl][grid.idx(nx, ny)] as f64, c));
            }
        }
    }
    let end = reached?;
    let (width, cell) = at[key(end)];
    Some(WidestPath {
        width_mm: if width.is_finite() { width } else { f64::INFINITY },
        at: grid.centre(cell.1, cell.2),
        layer: cell.0,
    })
}
