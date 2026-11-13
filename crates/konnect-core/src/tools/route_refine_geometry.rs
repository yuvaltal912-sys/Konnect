//! Conservative geometry profile and candidate checks for S7; no I/O or mutation.
use super::distance_kernel::Prepared;
use super::*;
use std::rc::Rc;

pub(super) struct Rules {
    pub clearance: f64,
    pub edge: f64,
    pub hole: f64,
    pub hole_to_hole: f64,
    pub diameter: f64,
    pub drill: f64,
    pub annular: f64,
    pub min_width: f64,
    classes: BTreeMap<String, f64>,
    assignments: BTreeMap<String, Vec<String>>,
    /// Wildcard pattern and the class it gives to the nets it matches.
    patterns: Vec<(String, String)>,
}
impl Rules {
    pub fn read(s: &crate::routing_context::Snapshot) -> Result<Self> {
        let p = super::super::verification::project_json_guard::parse(
            s.project
                .as_deref()
                .context("saved project required for planning rules")?,
        )?;
        let r = &p["board"]["design_settings"]["rules"];
        let value = |v: &Value, key: &str, required: bool| -> Result<f64> {
            match v.get(key) {
                None | Some(Value::Null) if !required => Ok(0.),
                Some(n) => {
                    let n = n.as_f64().context("rule must be numeric")?;
                    ensure!(
                        n.is_finite() && (0.0..=100.).contains(&n),
                        "invalid rule {key}"
                    );
                    Ok(n)
                }
                _ => bail!("required saved rule {key} unavailable"),
            }
        };
        let mut out = Self {
            clearance: value(r, "min_clearance", true)?,
            edge: value(r, "min_copper_edge_clearance", true)?,
            hole: value(r, "min_hole_clearance", true)?,
            hole_to_hole: value(r, "min_hole_to_hole", true)?,
            diameter: value(r, "min_via_diameter", true)?,
            drill: value(r, "min_through_hole_diameter", true)?,
            annular: value(r, "min_via_annular_width", true)?,
            min_width: value(r, "min_track_width", true)?,
            classes: BTreeMap::new(),
            assignments: BTreeMap::new(),
            patterns: Vec::new(),
        };
        let mut unset = Vec::new();
        for c in p["net_settings"]["classes"]
            .as_array()
            .context("saved netclasses unavailable")?
        {
            let name = c["name"].as_str().context("netclass lacks name")?;
            ensure!(
                !name.is_empty() && !out.classes.contains_key(name),
                "duplicate/empty class"
            );
            out.classes
                .insert(name.into(), value(c, "track_width", false)?);
            if c.get("track_width").is_none_or(Value::is_null) {
                unset.push(name.to_string());
            }
            out.clearance = out.clearance.max(value(c, "clearance", false)?);
            out.diameter = out.diameter.max(value(c, "via_diameter", false)?);
            out.drill = out.drill.max(value(c, "via_drill", false)?);
        }
        ensure!(
            out.classes.contains_key("Default"),
            "Default netclass unavailable"
        );
        // KiCad gives a class that sets no track width the Default class's width.
        let default_width = out.classes["Default"];
        for name in unset {
            out.classes.insert(name, default_width);
        }
        // KiCad saves `null` when a project has no assignments or patterns.
        let present = |key: &str| p["net_settings"].get(key).filter(|v| !v.is_null());
        if let Some(map) = present("netclass_assignments") {
            for (net, c) in map.as_object().context("invalid class assignments")? {
                let list = if let Some(s) = c.as_str() {
                    vec![s.to_string()]
                } else {
                    c.as_array()
                        .context("invalid assigned class")?
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(str::to_string)
                                .context("invalid assigned class")
                        })
                        .collect::<Result<Vec<_>>>()?
                };
                ensure!(
                    list.iter().all(|c| out.classes.contains_key(c)),
                    "unknown assigned class"
                );
                out.assignments.insert(net.clone(), list);
            }
        }
        if let Some(list) = present("netclass_patterns") {
            for v in list.as_array().context("invalid class patterns")? {
                let c = v["netclass"].as_str().context("missing pattern class")?;
                let pat = v["pattern"].as_str().context("missing class pattern")?;
                ensure!(out.classes.contains_key(c), "unknown pattern class");
                out.patterns.push((pat.into(), c.into()));
            }
        }
        if let Some(rules) = &s.rules {
            let b = crate::custom_rules::routing_bounds(rules)?;
            out.clearance = out.clearance.max(b.clearance_mm);
            out.min_width = out.min_width.max(b.width_mm);
        }
        out.drill = (out.drill * 1e6).ceil() / 1e6;
        out.diameter = (out.diameter.max(out.drill + 2. * out.annular) * 1e6).ceil() / 1e6;
        ensure!(
            out.drill > 0. && out.diameter > out.drill && out.diameter <= 100.,
            "invalid via dimensions"
        );
        Ok(out)
    }
    pub fn width(&self, net: &str) -> f64 {
        // KiCad puts a copper item without a net in the Default class.
        if net.is_empty() {
            return self.min_width.max(self.classes["Default"]);
        }
        // A net follows the classes that name it, exactly or by a wildcard
        // pattern: the widest of them when several do. Default is the class
        // of a net that nothing names.
        let assigned = self.assignments.get(net).into_iter().flatten();
        let matched = self
            .patterns
            .iter()
            .filter(|(pattern, _)| glob(pattern, net))
            .map(|(_, class)| class);
        let class = assigned
            .chain(matched)
            .map(|class| self.classes[class])
            .reduce(f64::max)
            .unwrap_or(self.classes["Default"]);
        self.min_width.max(class)
    }
    pub fn json(&self) -> Value {
        json!({"policy":"conservative saved project/class maxima for clearance and via sizes; width floor from the widest class that names the net exactly or by wildcard pattern, else the Default class","clearance_mm":self.clearance,"edge_clearance_mm":self.edge,"hole_clearance_mm":self.hole,"hole_to_hole_mm":self.hole_to_hole,"via_diameter_mm":self.diameter,"via_drill_mm":self.drill,"min_annular_mm":self.annular,"sources":["saved project","saved netclasses","supported unconditional custom-rule minima"]})
    }
}
/// KiCad's wildcard reading of a netclass pattern: `*` is any run, `?` is any
/// one character, every other character is literal; the whole name must match
/// and case counts (KiCad 10.0.1 `common/eda_pattern_match.cpp:207-244`,
/// compiled without `wxRE_ICASE` at `:105`, applied to net names at
/// `common/project/net_settings.cpp:756-758`). KiCad's second, regular
/// expression reading of the same pattern (`:108-119`) is not evaluated.
/// Bounded: O(pattern * name), no backtracking.
fn glob(pattern: &str, text: &str) -> bool {
    let text: Vec<_> = text.chars().collect();
    let mut prev = vec![false; text.len() + 1];
    prev[0] = true;
    for p in pattern.chars() {
        let mut next = vec![false; text.len() + 1];
        if p == '*' {
            next[0] = prev[0];
        }
        for i in 1..=text.len() {
            next[i] = if p == '*' {
                prev[i] || next[i - 1]
            } else {
                (p == '?' || p == text[i - 1]) && prev[i - 1]
            };
        }
        prev = next;
    }
    prev[text.len()]
}

#[derive(Clone)]
pub(super) struct Copper {
    pub id: String,
    pub net: Option<String>,
    pub shape: Shape,
    pub mask: u64,
    pub kind: &'static str,
    pub locked: bool,
    pub drill: Option<f64>,
    pub layer: String,
    /// The shape is the copper itself, not a chord approximation of a curve.
    /// Only exact obstacles can pass a saved clearance on to replacement copper.
    pub exact: bool,
}
impl Copper {
    pub fn track(
        id: String,
        net: String,
        layer: String,
        mask: u64,
        a: Pt,
        b: Pt,
        width: f64,
    ) -> Self {
        Self {
            id,
            net: Some(net),
            shape: Shape::path(vec![a, b], width / 2.),
            mask,
            kind: "track",
            locked: false,
            drill: None,
            layer,
            exact: true,
        }
    }
    pub fn preimage(&self) -> Value {
        json!({"uuid":self.id,"net":self.net,"layer":self.layer,"points":self.shape.pts.iter().map(|p|point(*p)).collect::<Vec<_>>(),"width":2.*self.shape.radius,"drill":self.drill})
    }
    /// Whether this straight track lies inside `original`'s copper: the same
    /// width, net and layers, both ends on the original centreline segment
    /// within the 1 nm coordinate resolution. Such copper adds nothing new.
    pub fn within(&self, original: &Copper) -> bool {
        self.kind == "track"
            && original.kind == "track"
            && self.net == original.net
            && self.mask == original.mask
            && self.shape.radius == original.shape.radius
            && self.shape.pts.len() == 2
            && original.shape.pts.len() == 2
            && self.shape.pts.iter().all(|p| {
                konnect_sexp::planar::segment_distance(
                    *p,
                    *p,
                    original.shape.pts[0],
                    original.shape.pts[1],
                ) <= COORDINATE
            })
    }
}

/// Coordinates are rounded to 1 nm; nothing finer is distinguishable on a board.
const COORDINATE: f64 = 0.000001;
/// Shapes at least this large get a cached edge tree and point index.
const PREPARE_MIN: usize = 16;
/// Search this far past a distance threshold, so the threshold is never the cap.
const SLACK: f64 = 0.001;
/// Copper this close is in contact.
const TOUCH: f64 = 1e-9;

type Bounds = (f64, f64, f64, f64);

#[derive(Clone)]
struct Hole {
    shape: Shape,
    mask: u64,
    plated_net: Option<String>,
}

/// The caller's computational allowance. Exhaustion is a `SearchLimit`, never
/// a geometry verdict.
pub(super) struct Meter {
    pub pairs: usize,
    pub work: usize,
    pub limits: effort::Limits,
    pub started: std::time::Instant,
}
impl Meter {
    pub fn charge(&mut self, work: usize) -> Result<()> {
        let next = self
            .work
            .checked_add(work)
            .context("geometry work overflow")?;
        if (self.work == 0 || next / 1024 != self.work / 1024)
            && self.started.elapsed().as_millis() >= u128::from(self.limits.max_elapsed_ms)
        {
            return Err(SearchLimit("elapsed search time limit reached").into());
        }
        if next > self.limits.max_geometry_work {
            return Err(SearchLimit("geometry work budget exhausted").into());
        }
        self.work = next;
        Ok(())
    }
    fn pair(&mut self) -> Result<()> {
        if self.pairs >= self.limits.max_pair_checks {
            return Err(SearchLimit("pair-check budget exhausted").into());
        }
        self.pairs += 1;
        Ok(())
    }
    /// The distance between two shapes, exact wherever it is below `limit`;
    /// otherwise some value at or above `limit`. Every caller compares against
    /// a threshold no larger than its limit, so decisions are those of the true
    /// distance while distant copper costs nothing. The micrometre of slack
    /// keeps rounding at the limit itself out of the comparison.
    fn distance(
        &mut self,
        a: &Shape,
        pa: Option<&Prepared>,
        b: &Shape,
        pb: Option<&Prepared>,
        limit: f64,
    ) -> Result<f64> {
        self.pair()?;
        super::distance_kernel::distance_with(a, pa, b, pb, limit + SLACK, &mut |work| {
            self.charge(work)
        })
    }
}

pub(super) struct Scene {
    pub items: Vec<Copper>,
    boxes: Vec<Bounds>,
    prepared: Vec<Option<Rc<Prepared>>>,
    holes: Vec<Hole>,
    hole_boxes: Vec<Bounds>,
    pub layers: Vec<String>,
    pub outline: konnect_sexp::outline::Outline,
    outline_rings: Vec<(Shape, Option<Rc<Prepared>>)>,
    pub rules: Rules,
    pub meter: Meter,
}
impl Scene {
    pub fn read(
        tree: &SexpNode,
        m: &audit::Model,
        mut rules: Rules,
        limits: effort::Limits,
        started: std::time::Instant,
    ) -> Result<Self> {
        ensure!(
            (2..=4).contains(&m.layers.len()) && m.layers.len() != 3,
            "only two/four copper layers supported"
        );
        ensure!(
            m.layers.first().map(String::as_str) == Some("F.Cu")
                && m.layers.last().map(String::as_str) == Some("B.Cu"),
            "invalid layer stack"
        );
        ensure!(
            m.layers.iter().collect::<BTreeSet<_>>().len() == m.layers.len(),
            "duplicate layers"
        );
        ensure!(
            m.geo.outline.status() == konnect_sexp::outline::OutlineStatus::Closed,
            "closed, fully readable outline required"
        );
        ensure!(
            m.geo.skipped.is_empty(),
            "geometry has skipped items: {:?}",
            m.geo.skipped
        );
        let mut locks = BTreeSet::new();
        let mut ids = BTreeSet::new();
        let mut nodes = 0;
        let mut groups = Vec::new();
        inspect(tree, false, &mut ids, &mut locks, &mut groups, &mut nodes)?;
        resolve_locks(&mut locks, &groups);
        for z in tree.find_all("zone") {
            if let Some(c) = z.find("connect_pads").and_then(|n| n.find("clearance")) {
                let c = c
                    .get_f64(1)
                    .filter(|v| v.is_finite() && (0.0..=100.).contains(v))
                    .context("invalid zone clearance")?;
                rules.clearance = rules.clearance.max(c);
            }
        }
        let mut net_codes = BTreeSet::new();
        let mut net_names = BTreeSet::new();
        for n in tree.find_all("net") {
            let code = n
                .get(1)
                .and_then(SexpNode::as_str)
                .context("net code absent")?;
            let name = n
                .get(2)
                .and_then(SexpNode::as_str)
                .context("net name absent")?;
            ensure!(
                net_codes.insert(code) && net_names.insert(name),
                "ambiguous net table"
            );
        }
        let all = (1u64 << m.layers.len()) - 1;
        let mask = |layers: &[String]| -> Result<u64> {
            let mut mask = 0;
            for l in layers {
                if l == "*.Cu" {
                    mask |= all;
                } else if l.ends_with(".Cu") {
                    mask |= 1 << m.layer_ix(l).context("unknown copper layer")?;
                }
            }
            Ok(mask)
        };
        let mut items = Vec::new();
        let mut holes = Vec::new();
        for f in &m.geo.footprints {
            for p in &f.pads {
                ensure!(!p.approximate, "unsupported approximate pad");
                let id = p.uuid.clone().context("pad lacks UUID")?;
                let pmask = mask(&p.layers)?;
                if let Some(d) = p.drill {
                    ensure!(d.is_finite() && d > 0., "invalid pad drill");
                    holes.push(Hole {
                        shape: Shape::disc(p.at, d / 2.),
                        mask: all,
                        plated_net: if p.kind == "thru_hole" {
                            p.net.clone()
                        } else {
                            None
                        },
                    });
                }
                // An unplated hole's pad can still carry copper wider than its
                // drill; it stays an obstacle with no net of its own.
                if pmask != 0 {
                    items.push(Copper {
                        id,
                        net: if p.kind == "np_thru_hole" {
                            None
                        } else {
                            p.net.clone()
                        },
                        shape: p.shape.clone(),
                        mask: pmask,
                        kind: "pad",
                        locked: f.locked,
                        drill: p.drill,
                        layer: p.layers.join(","),
                        // Round, oval and rounded pads are chord approximations.
                        exact: matches!(p.form.as_str(), "rect" | "trapezoid"),
                    });
                }
            }
        }
        for e in &m.geo.others {
            let mut mask = mask(&e.layers)?;
            if mask == 0 {
                continue;
            }
            let kind = match e.kind {
                Kind::Track => {
                    if e.form == "segment" {
                        "track"
                    } else {
                        "arc"
                    }
                }
                Kind::Via => "via",
                Kind::Graphic => "graphic",
                Kind::Zone if e.form == "keepout" => "keepout",
                Kind::Zone => continue,
                _ => bail!("unsupported copper item"),
            };
            let id = e.uuid.clone().context("copper item lacks UUID")?;
            let drill = e.props.iter().find(|(k, _)| *k == "drill").map(|x| x.1);
            if kind == "via" {
                ensure!(
                    e.layers == vec!["F.Cu".to_string(), "B.Cu".to_string()],
                    "only through vias supported"
                );
                mask = all;
                holes.push(Hole {
                    shape: Shape::disc(e.at, drill.context("via drill absent")? / 2.),
                    mask: all,
                    plated_net: e.net.clone(),
                });
            }
            items.push(Copper {
                id,
                net: e.net.clone(),
                // A straight segment is a stadium and a via a disc: both exact.
                exact: (kind == "track" && e.shape.pts.len() == 2)
                    || (kind == "via" && e.shape.pts.len() == 1),
                shape: e.shape.clone(),
                mask,
                kind,
                locked: locks.contains(e.uuid.as_deref().unwrap_or("")),
                drill,
                layer: e.layers.first().cloned().unwrap_or_default(),
            });
        }
        for f in &m.fills {
            for (i, r) in f.rings.iter().enumerate() {
                let layer = m.layer_ix(&f.layer).context("fill on unknown layer")?;
                items.push(Copper {
                    id: format!(
                        "{}:fill:{i}:{}",
                        f.zone_uuid.as_deref().context("zone UUID absent")?,
                        f.layer
                    ),
                    net: f.net.clone(),
                    shape: Shape::polygon(r.pts.clone()),
                    mask: 1 << layer,
                    kind: "fill",
                    locked: true,
                    drill: None,
                    layer: f.layer.clone(),
                    // The saved polygon is the copper KiCad itself checks.
                    exact: true,
                });
            }
        }
        ensure!(items.len() <= 50_000, "copper inventory exceeds bound");
        ensure!(
            items.iter().filter(|i| i.kind == "track").count() <= 5000,
            "audit track-count bound exceeded"
        );
        ensure!(
            items.iter().map(|i| i.shape.pts.len()).sum::<usize>() <= 500_000,
            "total geometry vertex bound exceeded"
        );
        for item in &items {
            ensure!(
                !item.shape.pts.is_empty()
                    && item.shape.pts.len() <= 20_000
                    && item.shape.radius.is_finite()
                    && item.shape.radius >= 0.
                    && item.shape.pts.iter().all(|p| p.0.is_finite()
                        && p.1.is_finite()
                        && p.0.abs() <= 2000.
                        && p.1.abs() <= 2000.),
                "invalid/unbounded geometry"
            );
        }
        Self::assemble(
            items,
            holes,
            m.layers.clone(),
            m.geo.outline.clone(),
            rules,
            limits,
            started,
        )
    }

    fn assemble(
        items: Vec<Copper>,
        holes: Vec<Hole>,
        layers: Vec<String>,
        outline: konnect_sexp::outline::Outline,
        rules: Rules,
        limits: effort::Limits,
        started: std::time::Instant,
    ) -> Result<Self> {
        let boxes = items
            .iter()
            .map(|i| i.shape.bbox().context("geometry has no bounds"))
            .collect::<Result<Vec<_>>>()?;
        let hole_boxes = holes
            .iter()
            .map(|h| h.shape.bbox().context("hole has no bounds"))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            prepared: vec![None; items.len()],
            items,
            boxes,
            holes,
            hole_boxes,
            layers,
            outline_rings: outline
                .loops
                .iter()
                .map(|r| (Shape::ring(r.clone(), 0.), None))
                .collect(),
            outline,
            rules,
            meter: Meter {
                pairs: 0,
                work: 0,
                limits,
                started,
            },
        })
    }

    pub fn charge(&mut self, work: usize) -> Result<()> {
        self.meter.charge(work)
    }

    /// Build item `i`'s reusable indexes on first use; small shapes need none.
    fn prepare(&mut self, i: usize) -> Result<()> {
        if self.prepared[i].is_none() && self.items[i].shape.pts.len() >= PREPARE_MIN {
            let built = super::distance_kernel::prepare(&self.items[i].shape, &mut |work| {
                self.meter.charge(work)
            })?;
            self.prepared[i] = Some(Rc::new(built));
        }
        Ok(())
    }
    /// Distance from a free shape to item `i`, exact below `limit`.
    fn gap_to_item(&mut self, shape: &Shape, i: usize, limit: f64) -> Result<f64> {
        self.prepare(i)?;
        self.meter.distance(
            shape,
            None,
            &self.items[i].shape,
            self.prepared[i].as_deref(),
            limit,
        )
    }
    /// Distance between two items, exact below `limit`.
    fn between(&mut self, a: usize, b: usize, limit: f64) -> Result<f64> {
        self.prepare(a)?;
        self.prepare(b)?;
        self.meter.distance(
            &self.items[a].shape,
            self.prepared[a].as_deref(),
            &self.items[b].shape,
            self.prepared[b].as_deref(),
            limit,
        )
    }
    #[cfg(test)]
    pub fn distance(&mut self, a: &Shape, b: &Shape) -> Result<f64> {
        self.meter.distance(a, None, b, None, f64::INFINITY)
    }

    /// The scene item of every ring of every saved fill, in the audit model's
    /// order. Fills are read in that order and no proposal removes one.
    pub fn fill_rings(&mut self, m: &audit::Model) -> Result<Vec<Vec<usize>>> {
        self.charge(self.items.len())?;
        let mut fills = (0..self.items.len()).filter(|&i| self.items[i].kind == "fill");
        let mut table = Vec::with_capacity(m.fills.len());
        for fill in &m.fills {
            let mut row = Vec::with_capacity(fill.rings.len());
            for ring in &fill.rings {
                let i = fills.next().context("saved fill inventory changed")?;
                ensure!(
                    self.items[i].shape.pts.len() == ring.pts.len(),
                    "saved fill inventory changed"
                );
                row.push(i);
            }
            table.push(row);
        }
        ensure!(fills.next().is_none(), "saved fill inventory changed");
        Ok(table)
    }
    /// Whether `p` is inside filled item `i`, by `point_in_ring`'s rule.
    fn item_contains(&mut self, i: usize, p: Pt) -> Result<bool> {
        self.prepare(i)?;
        super::distance_kernel::contains(
            &self.items[i].shape,
            self.prepared[i].as_deref(),
            p,
            &mut |work| self.meter.charge(work),
        )
    }
    /// Indices of the net's saved fill rings. Proposals never remove a fill.
    pub fn fills_of(&mut self, net: &str) -> Result<Vec<usize>> {
        self.charge(self.items.len())?;
        Ok((0..self.items.len())
            .filter(|&i| self.items[i].kind == "fill" && self.items[i].net.as_deref() == Some(net))
            .collect())
    }
    /// The listed fills that contain the whole via with the geometry margin.
    pub fn containing_fills(&mut self, fills: &[usize], via: &Shape) -> Result<Vec<usize>> {
        let mut found = Vec::new();
        let bounds = via.bbox().context("candidate has no bounds")?;
        for &i in fills {
            self.charge(1)?;
            if !near_bounds(bounds, self.boxes[i], 0.) {
                continue;
            }
            self.prepare(i)?;
            let (fill, prepared) = (&self.items[i].shape, self.prepared[i].as_deref());
            if !super::distance_kernel::contains(fill, prepared, via.pts[0], &mut |work| {
                self.meter.charge(work)
            })? {
                continue;
            }
            self.meter.pair()?;
            if super::distance_kernel::boundary_gap(
                fill,
                prepared,
                via,
                None,
                MARGIN + SLACK,
                &mut |work| self.meter.charge(work),
            )? >= MARGIN
            {
                found.push(i);
            }
        }
        Ok(found)
    }
    pub fn touches_pad(
        &mut self,
        at: Pt,
        radius: f64,
        net: &Option<String>,
        mask: u64,
    ) -> Result<bool> {
        let shape = Shape::disc(at, radius);
        let bounds = shape.bbox().unwrap();
        for i in 0..self.items.len() {
            self.charge(1)?;
            let pad = &self.items[i];
            if pad.kind == "pad"
                && pad.net.is_some()
                && &pad.net == net
                && pad.mask & mask != 0
                && near_bounds(bounds, self.boxes[i], 0.)
                && self.gap_to_item(&shape, i, TOUCH)? <= TOUCH
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// Certify one piece of proposed copper against everything that stays.
    ///
    /// `replaced` lists the saved straight tracks this proposal replaces. They
    /// matter in one case only: an exact obstacle may sit inside the extra
    /// geometry margin when the new copper is no closer to it than that saved
    /// copper already was and still meets the nominal clearance. The proposal
    /// then brings nothing nearer than the saved board had it. Returns how many
    /// obstacles were accepted that way.
    pub fn safe(
        &mut self,
        new: &Copper,
        removed: &BTreeSet<String>,
        created: &[Copper],
        replaced: &[usize],
    ) -> Result<usize> {
        self.charge(
            2 * self.outline.loops.iter().map(Vec::len).sum::<usize>() * new.shape.pts.len(),
        )?;
        ensure!(
            new.shape.pts.iter().all(|p| self.outline.contains(*p)),
            "outside outline"
        );
        for r in 0..self.outline_rings.len() {
            if self.outline_rings[r].1.is_none() && self.outline_rings[r].0.pts.len() >= PREPARE_MIN
            {
                let built =
                    super::distance_kernel::prepare(&self.outline_rings[r].0, &mut |work| {
                        self.meter.charge(work)
                    })?;
                self.outline_rings[r].1 = Some(Rc::new(built));
            }
            let (ring, prepared) = (&self.outline_rings[r].0, self.outline_rings[r].1.as_deref());
            let edge = self.rules.edge + MARGIN;
            ensure!(
                self.meter
                    .distance(&new.shape, None, ring, prepared, edge)?
                    >= edge,
                "board edge/cutout clearance"
            );
        }
        if new.kind == "track" {
            ensure!(
                2. * new.shape.radius + 1e-9 >= self.rules.width(new.net.as_deref().unwrap_or("")),
                "existing width below supported saved floor"
            );
        }
        if let Some(d) = new.drill {
            ensure!(
                d > 0. && 2. * new.shape.radius - d + 1e-12 >= 2. * self.rules.annular,
                "via annular width"
            );
        }
        let new_bounds = new.shape.bbox().context("candidate bounds unavailable")?;
        let reach = self.rules.clearance + MARGIN;
        // KiCad also holds a new via's own hole to the hole clearance against
        // other-net copper. A thin annulus does not make the copper check imply it.
        let drill = new
            .drill
            .filter(|_| new.kind == "via")
            .map(|d| Shape::disc(new.shape.pts[0], d / 2.));
        let drill_bounds = drill.as_ref().and_then(Shape::bbox).unwrap_or(new_bounds);
        let hole_limit = self.rules.hole + MARGIN;
        let mut inherited = 0;
        for i in 0..self.items.len() {
            self.charge(1)?;
            let old = &self.items[i];
            if removed.contains(&old.id) || old.mask & new.mask == 0 {
                continue;
            }
            if old.kind != "keepout" && old.net.is_some() && old.net == new.net {
                // Same-net copper may touch, except that a new via never lands
                // on a pad: an open via in a pad wicks solder away from the joint.
                if new.kind == "via"
                    && old.kind == "pad"
                    && near_bounds(new_bounds, self.boxes[i], MARGIN)
                {
                    let id = old.id.clone();
                    ensure!(
                        self.gap_to_item(&new.shape, i, MARGIN)? >= MARGIN,
                        "via-in-pad overlap with {}",
                        id
                    );
                }
                continue;
            }
            // The copper check comes first, so a place that fails both is named
            // by its copper clearance.
            'copper: {
                if !near_bounds(new_bounds, self.boxes[i], reach) {
                    break 'copper;
                }
                // Curve approximations, keep-outs and graphics always keep the margin.
                let (id, exact) = (
                    old.id.clone(),
                    old.exact && matches!(old.kind, "track" | "via" | "pad" | "fill"),
                );
                let distance = self.gap_to_item(&new.shape, i, reach)?;
                if distance >= reach {
                    break 'copper;
                }
                // Inside the margin band. Board coordinates are whole nanometres, so
                // 1e-9 mm absorbs only floating-point noise, never a real shortfall.
                let mut saved = f64::INFINITY;
                if exact && new.kind == "track" && distance + 1e-9 >= self.rules.clearance {
                    for &o in replaced {
                        let original = &self.items[o];
                        if original.kind == "track"
                            && original.exact
                            && original.net == new.net
                            && original.mask == new.mask
                            && removed.contains(&original.id)
                        {
                            saved = saved.min(self.between(o, i, reach)?);
                        }
                    }
                }
                ensure!(distance + 1e-9 >= saved, "clearance to {}", id);
                inherited += 1;
            }
            if let Some(drill) = &drill {
                if self.items[i].kind != "keepout"
                    && near_bounds(drill_bounds, self.boxes[i], hole_limit)
                {
                    let id = self.items[i].id.clone();
                    ensure!(
                        self.gap_to_item(drill, i, hole_limit)? >= hole_limit,
                        "hole clearance to {}",
                        id
                    );
                }
            }
        }
        for other in created {
            self.charge(1)?;
            if other.mask & new.mask == 0 || (other.net.is_some() && other.net == new.net) {
                continue;
            }
            ensure!(
                self.meter
                    .distance(&new.shape, None, &other.shape, None, reach)?
                    >= reach,
                "clearance to {}",
                other.id
            );
            if let Some(drill) = &drill {
                ensure!(
                    self.meter
                        .distance(drill, None, &other.shape, None, hole_limit)?
                        >= hole_limit,
                    "hole clearance to {}",
                    other.id
                );
            }
        }
        let hole_reach = self.rules.hole.max(self.rules.hole_to_hole) + MARGIN;
        for h in 0..self.holes.len() {
            self.charge(1)?;
            if self.holes[h].mask & new.mask == 0
                || !near_bounds(new_bounds, self.hole_boxes[h], hole_reach)
            {
                continue;
            }
            let hole = self.holes[h].clone();
            // A same-net plated barrel is intentional copper contact. NPTH and
            // other-net holes retain copper clearance; all drills retain spacing.
            let plated_contact = hole.plated_net.is_some() && hole.plated_net == new.net;
            if !plated_contact {
                let limit = self.rules.hole + MARGIN;
                ensure!(
                    self.meter
                        .distance(&new.shape, None, &hole.shape, None, limit)?
                        >= limit,
                    "copper to hole clearance"
                );
            }
            if let Some(d) = new.drill {
                let limit = self.rules.hole_to_hole + MARGIN;
                ensure!(
                    self.meter.distance(
                        &Shape::disc(new.shape.pts[0], d / 2.),
                        None,
                        &hole.shape,
                        None,
                        limit
                    )? >= limit,
                    "hole to hole clearance"
                );
            }
        }
        Ok(inherited)
    }
    /// Indices of the saved items a proposal removes or rewrites.
    pub fn indices(&self, removed: &BTreeSet<String>) -> Vec<usize> {
        (0..self.items.len())
            .filter(|&i| removed.contains(&self.items[i].id))
            .collect()
    }
    pub fn attachments(&mut self, removed: &BTreeSet<String>, created: &[Copper]) -> Result<()> {
        let old = self.indices(removed);
        ensure!(
            old.iter()
                .all(|&i| !self.items[i].locked && self.items[i].kind == "track"),
            "locked or unsupported edit"
        );
        for &a in &old {
            for b in 0..self.items.len() {
                self.charge(1)?;
                let (x, y) = (&self.items[a], &self.items[b]);
                if removed.contains(&y.id)
                    || y.kind == "keepout"
                    || x.net != y.net
                    || x.net.is_none()
                    || x.mask & y.mask == 0
                    || !near_bounds(self.boxes[a], self.boxes[b], 0.01)
                {
                    continue;
                }
                if self.between(a, b, 0.01)? <= 0.01 {
                    let (x, y) = (&self.items[a], &self.items[b]);
                    let id = y.id.clone();
                    ensure!(
                        y.kind != "arc"
                            || created.iter().any(|n| n.id == x.id && n.shape == x.shape),
                        "changed attachment to curved copper is not proven"
                    );
                    let mut retained = false;
                    for n in created {
                        if n.net == self.items[b].net
                            && n.mask & self.items[b].mask != 0
                            && self.gap_to_item(&n.shape, b, TOUCH)? <= TOUCH
                        {
                            retained = true;
                            break;
                        }
                    }
                    ensure!(retained, "lost attachment to {}", id);
                }
            }
        }
        // Each replacement is a connected subgraph, so all preserved attachments stay joined.
        let mut reached = BTreeSet::from([0usize]);
        loop {
            let mut changed = false;
            for i in 0..created.len() {
                let mut connected = false;
                if !reached.contains(&i) {
                    for &j in &reached {
                        if created[i].mask & created[j].mask != 0
                            && created[i].net == created[j].net
                            && self.meter.distance(
                                &created[i].shape,
                                None,
                                &created[j].shape,
                                None,
                                TOUCH,
                            )? <= TOUCH
                        {
                            connected = true;
                            break;
                        }
                    }
                }
                if connected {
                    reached.insert(i);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        ensure!(
            reached.len() == created.len(),
            "replacement copper is disconnected"
        );
        Ok(())
    }
    /// The bookkeeping cost of `accept`, payable before a proposal is recorded.
    pub fn accept_cost(&self, created: &[Copper]) -> usize {
        self.items.len() + created.iter().map(|i| i.shape.pts.len()).sum::<usize>()
    }
    /// Replace the removed items by the created ones. Infallible once its cost
    /// is paid, so a recorded proposal and the scene can never disagree.
    pub fn accept(&mut self, removed: &BTreeSet<String>, created: Vec<Copper>) {
        let keep: Vec<bool> = self
            .items
            .iter()
            .map(|i| !removed.contains(&i.id))
            .collect();
        let mut flags = keep.iter();
        self.items.retain(|_| *flags.next().unwrap());
        let mut flags = keep.iter();
        self.boxes.retain(|_| *flags.next().unwrap());
        let mut flags = keep.iter();
        self.prepared.retain(|_| *flags.next().unwrap());
        for i in created {
            if let Some(d) = i.drill {
                let shape = Shape::disc(i.shape.pts[0], d / 2.);
                self.hole_boxes
                    .push(shape.bbox().expect("validated geometry"));
                self.holes.push(Hole {
                    shape,
                    mask: i.mask,
                    plated_net: i.net.clone(),
                });
            }
            self.boxes.push(i.shape.bbox().expect("validated geometry"));
            self.prepared.push(None);
            self.items.push(i);
        }
    }
    pub fn components(&mut self, net: &str) -> Result<(Vec<usize>, BTreeSet<usize>)> {
        let mut groups: Vec<_> = (0..self.items.len()).collect();
        let indices: Vec<_> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| i.net.as_deref() == Some(net) && i.kind != "keepout")
            .map(|(i, _)| i)
            .collect();
        for (pos, &a) in indices.iter().enumerate() {
            for &b in &indices[pos + 1..] {
                self.charge(1)?;
                if self.items[a].mask & self.items[b].mask == 0
                    || !near_bounds(self.boxes[a], self.boxes[b], 0.)
                {
                    continue;
                }
                let (ra, rb) = (root(&mut groups, a), root(&mut groups, b));
                if ra == rb {
                    continue;
                }
                if self.between(a, b, TOUCH)? <= TOUCH {
                    groups[ra] = rb;
                }
            }
        }
        for &i in &indices {
            groups[i] = root(&mut groups, i);
        }
        let anchored = indices
            .iter()
            .filter(|&&i| self.items[i].kind == "pad")
            .map(|&i| groups[i])
            .collect();
        Ok((groups, anchored))
    }
}
/// Meters the audit's plane analysis and answers its ring tests from the
/// scene's cached point indexes instead of rescanning each pour.
pub(super) struct PlaneProbe<'a> {
    scene: &'a mut Scene,
    rings: Vec<Vec<usize>>,
}
impl<'a> PlaneProbe<'a> {
    pub fn new(scene: &'a mut Scene, m: &audit::Model) -> Result<Self> {
        let rings = scene.fill_rings(m)?;
        Ok(Self { scene, rings })
    }
}
impl audit::PlaneProbe for PlaneProbe<'_> {
    fn charge(&mut self, work: usize) -> Result<()> {
        self.scene.charge(work)
    }
    fn ring_contains(&mut self, _: &audit::Model, fill: usize, ring: usize, p: Pt) -> Result<bool> {
        self.scene.item_contains(self.rings[fill][ring], p)
    }
}

fn root(groups: &mut [usize], mut i: usize) -> usize {
    while groups[i] != i {
        groups[i] = groups[groups[i]];
        i = groups[i];
    }
    i
}
fn near_bounds(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64), gap: f64) -> bool {
    a.0 <= b.2 + gap && a.2 + gap >= b.0 && a.1 <= b.3 + gap && a.3 + gap >= b.1
}

/// KiCad locks every item inside a locked group, however deep the nesting.
/// Returns how many groups were expanded.
fn resolve_locks(locks: &mut BTreeSet<String>, groups: &[(String, Vec<String>)]) -> usize {
    // Two groups can share an id in a damaged file: both member lists count.
    let mut members: BTreeMap<&str, Vec<&String>> = BTreeMap::new();
    for (id, list) in groups {
        members.entry(id.as_str()).or_default().extend(list);
    }
    // Walk down from the locked groups; a group is queued once, when it is
    // first locked, so the work is linear in the membership lists.
    let mut queue: Vec<String> = locks
        .iter()
        .filter(|id| members.contains_key(id.as_str()))
        .cloned()
        .collect();
    let mut visits = 0;
    while let Some(group) = queue.pop() {
        visits += 1;
        for &member in &members[group.as_str()] {
            if locks.insert(member.clone()) && members.contains_key(member.as_str()) {
                queue.push(member.clone());
            }
        }
    }
    visits
}

fn inspect(
    n: &SexpNode,
    in_footprint: bool,
    ids: &mut BTreeSet<String>,
    locks: &mut BTreeSet<String>,
    groups: &mut Vec<(String, Vec<String>)>,
    count: &mut usize,
) -> Result<()> {
    *count += 1;
    ensure!(*count <= 500_000, "source node bound exceeded");
    let head = n.head().unwrap_or("");
    let footprint = in_footprint || head == "footprint";
    let locked = n
        .find("locked")
        .is_some_and(|v| v.get(1).and_then(SexpNode::as_str) != Some("no"))
        || n.children()
            .unwrap_or(&[])
            .iter()
            .any(|v| v.as_str() == Some("locked"));
    if let Some(id) = n.find_str("uuid") {
        ensure!(
            !id.is_empty() && ids.insert(id.into()),
            "missing/duplicate UUID"
        );
        if locked {
            locks.insert(id.into());
        }
    }
    // KiCad treats every member of a locked group as locked. A group can be a
    // member of another group, so membership is kept and resolved by the caller.
    if head == "group" {
        let members: Vec<String> = n
            .find("members")
            .and_then(SexpNode::children)
            .unwrap_or(&[])
            .iter()
            .skip(1)
            .filter_map(SexpNode::as_str)
            .map(str::to_string)
            .collect();
        if locked {
            locks.extend(members.iter().cloned());
        }
        if let Some(id) = n.find_str("uuid").or_else(|| n.find_str("id")) {
            groups.push((id.into(), members));
        }
    }
    // A local override changes the rule a check would have to use.
    const OVERRIDES: [&str; 5] = [
        "clearance",
        "zone_connect",
        "thermal_width",
        "thermal_bridge_width",
        "thermal_gap",
    ];
    if head == "pad" || head == "footprint" {
        for field in OVERRIDES {
            ensure!(
                n.find(field).is_none(),
                "unsupported local {field} override"
            );
        }
    }
    // KiCad 9 and 10 can give a via or pad other copper on each layer. One
    // size per pad or via is modelled, so anything but `normal` is refused,
    // and so is an override placed on one layer of a padstack.
    if head == "padstack" {
        ensure!(
            n.find_str("mode").is_none_or(|mode| mode == "normal"),
            "unsupported padstack mode"
        );
        for layer in n.find_all("layer") {
            for field in OVERRIDES {
                ensure!(
                    layer.find(field).is_none(),
                    "unsupported local {field} override"
                );
            }
        }
        // With no mode, a layer entry still replaces that layer's copper.
        ensure!(
            n.find("layer").is_none(),
            "unsupported padstack layer copper"
        );
    }
    // KiCad holds copper to the edge clearance against Margin-layer shapes
    // too. Every item on that layer is refused, text and dimensions included.
    ensure!(
        n.find_str("layer") != Some("Margin"),
        "items on the Margin layer are outside planner profile"
    );
    let copper = n.find_str("layer").is_some_and(|l| l.ends_with(".Cu"))
        || n.find("layers").is_some_and(|v| {
            v.children()
                .unwrap_or(&[])
                .iter()
                .filter_map(SexpNode::as_str)
                .any(|l| l.ends_with(".Cu"))
        });
    if copper {
        ensure!(
            !footprint || matches!(head, "footprint" | "pad"),
            "unsupported footprint copper item {head}"
        );
        ensure!(
            matches!(
                head,
                "footprint"
                    | "pad"
                    | "segment"
                    | "arc"
                    | "via"
                    | "zone"
                    | "filled_polygon"
                    | "gr_line"
                    | "gr_rect"
                    | "gr_circle"
                    | "gr_arc"
                    | "gr_poly"
                    | "gr_curve"
            ),
            "unsupported copper item {head}"
        );
    }
    if head == "pad" {
        let size = n.find("size").context("missing pad size")?;
        ensure!(
            (1..=2).all(|i| size
                .get_f64(i)
                .is_some_and(|v| v.is_finite() && v > 0. && v <= 200.)),
            "pad dimensions outside curve/size profile"
        );
        if let Some(d) = n.find("drill") {
            ensure!(
                d.get(1).and_then(SexpNode::as_str) != Some("oval") && d.find("offset").is_none(),
                "slotted/offset drill is outside planner profile"
            );
        }
    }
    if matches!(head, "arc" | "gr_arc" | "fp_arc") {
        let p = |tag: &str| -> Result<Pt> {
            let p = n.find(tag).context("unreadable arc")?;
            Ok((
                p.get_f64(1).context("unreadable arc")?,
                p.get_f64(2).context("unreadable arc")?,
            ))
        };
        if let Some((_, r)) = konnect_sexp::planar::circumcircle(p("start")?, p("mid")?, p("end")?)
        {
            ensure!(
                r.is_finite() && r <= 100.,
                "arc radius exceeds proven approximation profile"
            );
        }
    }
    if matches!(head, "gr_circle" | "fp_circle") {
        let p = |tag: &str| -> Result<Pt> {
            let p = n.find(tag).context("unreadable circle")?;
            Ok((
                p.get_f64(1).context("unreadable circle")?,
                p.get_f64(2).context("unreadable circle")?,
            ))
        };
        ensure!(
            dist(p("center")?, p("end")?) <= 100.,
            "circle radius exceeds proven approximation profile"
        );
    }
    ensure!(
        !matches!(head, "gr_curve" | "fp_curve")
            || (!copper && n.find_str("layer") != Some("Edge.Cuts")),
        "Bezier safety geometry is outside proven approximation profile"
    );
    if head == "via" {
        ensure!(
            n.get(1)
                .and_then(SexpNode::as_str)
                .is_none_or(|s| !matches!(s, "blind" | "micro")),
            "unsupported via type"
        );
    }
    if head == "zone" && copper && n.find("keepout").is_none() {
        ensure!(
            !n.find_all("filled_polygon").is_empty(),
            "zone has no saved filled polygons"
        );
    }
    if head == "filled_polygon" {
        ensure!(
            n.find_str("layer").is_some_and(|l| l.ends_with(".Cu")),
            "fill layer unavailable"
        );
        let pts = n
            .find("pts")
            .and_then(SexpNode::children)
            .context("fill points missing")?;
        ensure!(pts.len() >= 4, "fill has too few vertices");
        for p in &pts[1..] {
            ensure!(
                p.head() == Some("xy")
                    && p.get_f64(1).is_some_and(f64::is_finite)
                    && p.get_f64(2).is_some_and(f64::is_finite),
                "unreadable fill vertex"
            );
        }
    }
    if let Some(children) = n.children() {
        for c in children {
            if matches!(c, SexpNode::List(_)) {
                inspect(c, footprint, ids, locks, groups, count)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn test_scene(items: Vec<Copper>) -> Scene {
    Scene::assemble(
        items,
        Vec::new(),
        vec!["F.Cu".into(), "B.Cu".into()],
        konnect_sexp::outline::Outline {
            loops: vec![vec![(0., 0.), (30., 0.), (30., 30.), (0., 30.)]],
            open_chains: 0,
            unreadable: 0,
            graphics: 1,
            curved: false,
            tolerance_mm: 0.005,
        },
        Rules {
            clearance: 0.2,
            edge: 0.2,
            hole: 0.2,
            hole_to_hole: 0.2,
            diameter: 0.6,
            drill: 0.3,
            annular: 0.1,
            min_width: 0.1,
            classes: BTreeMap::from([("Default".into(), 0.2)]),
            assignments: BTreeMap::new(),
            patterns: Vec::new(),
        },
        effort::Limits::standard(),
        std::time::Instant::now(),
    )
    .unwrap()
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    fn scene(items: Vec<Copper>) -> Scene {
        super::test_scene(items)
    }
    #[test]
    fn route_refine_chord_contact_cannot_certify_contact_with_a_curved_track() {
        let theta = std::f64::consts::PI / 100.;
        let u = (theta.cos(), theta.sin());
        let at = |r: f64| (15. + r * u.0, 15. + r * u.1);
        let old = Copper::track(
            "old".into(),
            "SIG".into(),
            "F.Cu".into(),
            1,
            at(5.),
            at(9.8),
            0.2,
        );
        let arc = Copper {
            id: "arc".into(),
            net: Some("SIG".into()),
            shape: Shape::path(
                konnect_sexp::planar::tessellate_arc(
                    (25., 15.),
                    (15. + 10. / 2f64.sqrt(), 15. + 10. / 2f64.sqrt()),
                    (15., 25.),
                    0.005,
                ),
                0.1,
            ),
            mask: 1,
            kind: "arc",
            locked: false,
            drill: None,
            layer: "F.Cu".into(),
            exact: false,
        };
        let mut new = old.clone();
        new.shape.pts[1] = at(10. * theta.cos() - 0.2);
        let mut scene = scene(vec![old, arc]);
        assert!(
            scene
                .attachments(&BTreeSet::from(["old".into()]), &[new])
                .is_err(),
            "chord touch leaves a real 0.0049344mm gap"
        );
    }
    #[test]
    fn route_refine_geometry_budget_counts_segment_work_not_only_shape_pairs() {
        let complex = Shape::polygon(
            (0..10001)
                .map(|i| {
                    let a = i as f64 * std::f64::consts::TAU / 10001.;
                    (10. + a.cos(), 10. + a.sin())
                })
                .collect(),
        );
        let mut scene = scene(Vec::new());
        assert_eq!(
            scene.distance(&complex, &complex).unwrap(),
            0.0,
            "obvious overlap must short-circuit within the work budget"
        );
        scene.meter.work = scene.meter.limits.max_geometry_work - 1;
        let a = Shape::path(vec![(1., 1.), (2., 1.), (2., 2.)], 0.1);
        let b = Shape::path(vec![(5., 5.), (6., 5.), (6., 6.)], 0.1);
        assert!(
            scene.distance(&a, &b).is_err(),
            "actual segment comparisons must consume the remaining work budget"
        );
    }

    #[test]
    fn route_refine_elapsed_limit_stops_search_without_relaxing_geometry() {
        let mut scene = scene(Vec::new());
        scene.meter.limits.max_elapsed_ms = 1;
        scene.meter.started = std::time::Instant::now() - std::time::Duration::from_millis(5);
        let error = scene.charge(1).unwrap_err();
        assert_eq!(
            error.downcast_ref::<SearchLimit>().unwrap().0,
            "elapsed search time limit reached"
        );
        assert_eq!(scene.meter.work, 0);
    }

    fn via(at: Pt, diameter: f64, drill: f64) -> Copper {
        Copper {
            id: "new".into(),
            net: Some("GND".into()),
            shape: Shape::disc(at, diameter / 2.),
            mask: 15,
            kind: "via",
            locked: false,
            drill: Some(drill),
            layer: "F.Cu,B.Cu".into(),
            exact: true,
        }
    }

    #[test]
    fn route_refine_via_checks_inner_layers_and_clearance_thresholds() {
        for layer in [2, 4] {
            for (y, expected) in [(10.60, false), (10.62, true)] {
                let obstacle = Copper::track(
                    "obstacle".into(),
                    "OTHER".into(),
                    "inner".into(),
                    layer,
                    (8., y),
                    (12., y),
                    0.2,
                );
                let mut scene = scene(vec![obstacle]);
                assert_eq!(
                    scene
                        .safe(&via((10., 10.), 0.6, 0.3), &BTreeSet::new(), &[], &[])
                        .is_ok(),
                    expected
                );
            }
        }
    }

    #[test]
    fn route_refine_annular_width_is_coupled_after_rounding() {
        for (diameter, drill, expected) in [
            (0.6, 0.3, true),
            (0.59, 0.3, false),
            (0.6, 0.4, false),
            (0.6, nm(0.3000006), false),
        ] {
            let mut scene = scene(Vec::new());
            scene.rules.annular = 0.15;
            assert_eq!(
                scene
                    .safe(
                        &via((10., 10.), diameter, drill),
                        &BTreeSet::new(),
                        &[],
                        &[]
                    )
                    .is_ok(),
                expected
            );
        }
    }

    #[test]
    fn route_refine_same_net_drills_still_require_hole_spacing() {
        for (x, expected) in [(10.5, false), (10.6, true)] {
            let mut scene = scene(Vec::new());
            scene.holes.push(Hole {
                shape: Shape::disc((x, 10.), 0.15),
                mask: 15,
                plated_net: Some("GND".into()),
            });
            scene
                .hole_boxes
                .push((x - 0.15, 10. - 0.15, x + 0.15, 10. + 0.15));
            assert_eq!(
                scene
                    .safe(&via((10., 10.), 0.6, 0.3), &BTreeSet::new(), &[], &[])
                    .is_ok(),
                expected
            );
        }
    }

    #[test]
    fn route_refine_complete_via_envelope_must_clear_edges_and_keepouts() {
        let mut scene = scene(Vec::new());
        assert!(scene
            .safe(&via((0.3, 10.), 0.6, 0.3), &BTreeSet::new(), &[], &[])
            .is_err());
        assert!(scene
            .safe(&via((0.52, 10.), 0.6, 0.3), &BTreeSet::new(), &[], &[])
            .is_ok());
        let mut keepout = via((10., 10.), 0.6, 0.3);
        keepout.kind = "keepout";
        scene.items.push(keepout);
        scene.boxes.push((9.7, 9.7, 10.3, 10.3));
        scene.prepared.push(None);
        assert!(scene
            .safe(&via((10., 10.), 0.6, 0.3), &BTreeSet::new(), &[], &[])
            .is_err());
    }

    fn straight(id: &str, net: &str, a: Pt, b: Pt, width: f64) -> Copper {
        Copper::track(id.into(), net.into(), "F.Cu".into(), 1, a, b, width)
    }

    // A shortened track adds no copper; anything moved, widened or re-netted does.
    #[test]
    fn route_refine_only_a_shortened_track_counts_as_saved_copper() {
        let original = straight("t", "SIG", (4., 6.), (6., 6.), 0.2);
        let shorter = |a: Pt, b: Pt| straight("t", "SIG", a, b, 0.2);
        assert!(shorter((4., 6.), (5.5, 6.)).within(&original));
        assert!(shorter((4.25, 6.), (5.5, 6.)).within(&original));
        assert!(shorter((4., 6.), (6., 6.)).within(&original));
        for (name, moved) in [
            ("extended", shorter((4., 6.), (6.000002, 6.))),
            ("shifted sideways", shorter((4., 6.000002), (5.5, 6.000002))),
            ("rotated", shorter((4., 6.), (5.5, 6.01))),
            (
                "widened",
                straight("t", "SIG", (4., 6.), (5.5, 6.), 0.2000001),
            ),
            (
                "another net",
                straight("t", "OTHER", (4., 6.), (5.5, 6.), 0.2),
            ),
            ("another layer", {
                let mut t = shorter((4., 6.), (5.5, 6.));
                t.mask = 2;
                t
            }),
            ("not a track", {
                let mut t = shorter((4., 6.), (5.5, 6.));
                t.kind = "via";
                t
            }),
        ] {
            assert!(!moved.within(&original), "{name} is new copper");
        }
    }

    /// A corner whose saved leg runs exactly 0.2 mm from a foreign track, with
    /// the chamfer connector that replaces it. `gap` moves the foreign track.
    fn band(gap: f64, obstacle: impl FnOnce(&mut Copper)) -> (Scene, Copper, BTreeSet<String>) {
        let mut foreign = straight("foreign", "OTHER", (4.8, 5.8 - gap), (7.5, 5.8 - gap), 0.2);
        obstacle(&mut foreign);
        let scene = scene(vec![
            straight("leg", "SIG", (4., 6.), (6., 6.), 0.2),
            foreign,
        ]);
        let connector = straight("new", "SIG", (5.5, 6.), (6., 6.5), 0.2);
        (scene, connector, BTreeSet::from(["leg".to_string()]))
    }

    // The 0.01 mm margin may be waived only for an exact obstacle the replaced
    // saved copper already faced at that distance, and never below the rule.
    #[test]
    fn route_refine_new_copper_stays_inside_the_outline_at_the_class_width() {
        let track =
            |a, b, width| Copper::track("new".into(), "GND".into(), "F.Cu".into(), 1, a, b, width);
        let none = BTreeSet::new();
        let mut scene = test_scene(Vec::new());
        let inside = track((5., 5.), (8., 5.), 0.2);
        assert_eq!(scene.safe(&inside, &none, &[], &[]).unwrap(), 0);
        // Far outside: no edge is near, so only the containment check refuses it.
        let outside = track((45., 45.), (48., 45.), 0.2);
        let refused = scene.safe(&outside, &none, &[], &[]).unwrap_err();
        assert_eq!(format!("{refused:#}"), "outside outline");
        let thin = track((5., 5.), (8., 5.), 0.15);
        let refused = scene.safe(&thin, &none, &[], &[]).unwrap_err();
        assert_eq!(
            format!("{refused:#}"),
            "existing width below supported saved floor"
        );
    }

    #[test]
    fn route_refine_copper_created_together_keeps_clearance_between_nets() {
        let track = |id: &str, net: &str, y: f64| {
            Copper::track(
                id.into(),
                net.into(),
                "F.Cu".into(),
                1,
                (5., y),
                (8., y),
                0.2,
            )
        };
        let none = BTreeSet::new();
        let mut scene = test_scene(Vec::new());
        let new = track("new", "GND", 5.);
        assert!(scene
            .safe(&new, &none, &[track("same", "GND", 5.3)], &[])
            .is_ok());
        let refused = scene
            .safe(&new, &none, &[track("near", "SIG", 5.3)], &[])
            .unwrap_err();
        assert_eq!(format!("{refused:#}"), "clearance to near");
    }

    /// KiCad holds a via's own hole to the hole clearance against other-net
    /// copper. With a small annulus the copper clearance does not imply it.
    #[test]
    fn route_refine_new_via_hole_keeps_hole_clearance_to_foreign_copper() {
        let scene_with = |y: f64, net: &str| {
            let mut scene = test_scene(vec![Copper::track(
                "foreign".into(),
                net.into(),
                "F.Cu".into(),
                1,
                (8., y),
                (12., y),
                0.2,
            )]);
            scene.rules.clearance = 0.15;
            scene.rules.hole = 0.25;
            scene.rules.annular = 0.075;
            scene
        };
        let via = Copper {
            id: "new".into(),
            net: Some("GND".into()),
            shape: Shape::disc((10., 10.), 0.225),
            mask: 3,
            kind: "via",
            locked: false,
            drill: Some(0.3),
            layer: "F.Cu,B.Cu".into(),
            exact: true,
        };
        let none = BTreeSet::new();
        // Copper gap 0.170 clears 0.15 + margin; drill gap 0.245 is under 0.25.
        let refused = scene_with(10.495, "OTHER")
            .safe(&via, &none, &[], &[])
            .unwrap_err();
        assert_eq!(format!("{refused:#}"), "hole clearance to foreign");
        // Drill gap 0.27 clears 0.25 + margin.
        assert!(scene_with(10.52, "OTHER")
            .safe(&via, &none, &[], &[])
            .is_ok());
        // Same-net copper may touch the via, hole included.
        assert!(scene_with(10.495, "GND")
            .safe(&via, &none, &[], &[])
            .is_ok());
        // Copper created in the same proposal on another net is held to it too.
        let beside = Copper::track(
            "beside".into(),
            "OTHER".into(),
            "F.Cu".into(),
            1,
            (8., 10.495),
            (12., 10.495),
            0.2,
        );
        let mut empty = scene_with(25., "OTHER");
        let refused = empty.safe(&via, &none, &[beside], &[]).unwrap_err();
        assert_eq!(format!("{refused:#}"), "hole clearance to beside");
        // A keep-out restricts copper, not drill spacing: no hole rule against it.
        let mut zone = scene_with(10.495, "OTHER");
        zone.items[0].kind = "keepout";
        zone.items[0].net = None;
        assert!(zone.safe(&via, &none, &[], &[]).is_ok());
    }

    #[test]
    fn route_refine_edge_clearance_includes_the_extra_margin() {
        // The test outline is the square 0..30 with a 0.2 mm edge rule.
        let track = |gap: f64| {
            let y = 0.1 + gap;
            Copper::track(
                "new".into(),
                "GND".into(),
                "F.Cu".into(),
                1,
                (5., y),
                (8., y),
                0.2,
            )
        };
        let none = BTreeSet::new();
        let refused = scene(Vec::new())
            .safe(&track(0.205), &none, &[], &[])
            .unwrap_err();
        assert_eq!(format!("{refused:#}"), "board edge/cutout clearance");
        assert!(scene(Vec::new())
            .safe(&track(0.215), &none, &[], &[])
            .is_ok());
    }

    #[test]
    fn route_refine_clearance_applies_only_between_copper_on_a_shared_layer() {
        let track = |id: &str, net: &str, mask: u64| {
            let layer = if mask == 1 { "F.Cu" } else { "B.Cu" };
            Copper::track(
                id.into(),
                net.into(),
                layer.into(),
                mask,
                (5., 5.),
                (8., 5.),
                0.2,
            )
        };
        let none = BTreeSet::new();
        let new = track("new", "GND", 1);
        // Directly under the new track, on the other layer: no rule applies.
        assert!(scene(vec![track("under", "SIG", 2)])
            .safe(&new, &none, &[], &[])
            .is_ok());
        assert!(scene(vec![track("same", "SIG", 1)])
            .safe(&new, &none, &[], &[])
            .is_err());
        assert!(scene(Vec::new())
            .safe(&new, &none, &[track("under", "SIG", 2)], &[])
            .is_ok());
        assert!(scene(Vec::new())
            .safe(&new, &none, &[track("same", "SIG", 1)], &[])
            .is_err());
    }

    #[test]
    fn route_refine_only_a_same_net_plated_hole_may_be_touched() {
        let near = |plated: Option<&str>| {
            let mut scene = scene(Vec::new());
            scene.holes.push(Hole {
                shape: Shape::disc((10., 10.3), 0.15),
                mask: 15,
                plated_net: plated.map(str::to_string),
            });
            scene.hole_boxes.push((9.85, 10.15, 10.15, 10.45));
            let new = Copper::track(
                "new".into(),
                "GND".into(),
                "F.Cu".into(),
                1,
                (8., 10.),
                (12., 10.),
                0.2,
            );
            scene
                .safe(&new, &BTreeSet::new(), &[], &[])
                .map_err(|e| format!("{e:#}"))
        };
        assert!(near(Some("GND")).is_ok());
        assert_eq!(near(Some("OTHER")).unwrap_err(), "copper to hole clearance");
        assert_eq!(near(None).unwrap_err(), "copper to hole clearance");
    }

    #[test]
    fn route_refine_attachment_and_continuity_are_per_layer() {
        let track = |id: &str, mask: u64, a, b| {
            let layer = if mask == 1 { "F.Cu" } else { "B.Cu" };
            Copper::track(id.into(), "GND".into(), layer.into(), mask, a, b, 0.2)
        };
        let leg = track("leg", 1, (5., 5.), (10., 5.));
        let removed = BTreeSet::from(["leg".to_string()]);
        let short = track("leg", 1, (5., 5.), (8., 5.));
        // A same-net track on the other layer that crosses the leg's end is
        // not attached to it, so shortening the leg loses nothing.
        let under = track("under", 2, (10., 4.), (10., 6.));
        assert!(scene(vec![leg.clone(), under])
            .attachments(&removed, std::slice::from_ref(&short))
            .is_ok());
        // A branch on the leg's own layer is attached, and copper on the other
        // layer does not keep it attached.
        let branch = track("branch", 1, (10., 5.), (10., 8.));
        let below = track("new", 2, (8., 5.), (10., 5.));
        let refused = scene(vec![leg.clone(), branch])
            .attachments(&removed, &[short.clone(), below.clone()])
            .unwrap_err();
        assert_eq!(format!("{refused:#}"), "lost attachment to branch");
        // Two pieces that meet only across layers are not one piece.
        let refused = scene(vec![leg])
            .attachments(&removed, &[short, below])
            .unwrap_err();
        assert_eq!(format!("{refused:#}"), "replacement copper is disconnected");
    }

    #[test]
    fn route_refine_replacement_copper_must_be_one_connected_piece() {
        let track =
            |id: &str, a, b| Copper::track(id.into(), "GND".into(), "F.Cu".into(), 1, a, b, 0.2);
        let leg = track("leg", (5., 5.), (10., 5.));
        let removed = BTreeSet::from(["leg".to_string()]);
        let joined = [
            track("leg", (5., 5.), (8., 5.)),
            track("new", (8., 5.), (10., 5.)),
        ];
        assert!(test_scene(vec![leg.clone()])
            .attachments(&removed, &joined)
            .is_ok());
        let apart = [
            track("leg", (5., 5.), (7., 5.)),
            track("new", (8., 5.), (10., 5.)),
        ];
        let refused = test_scene(vec![leg])
            .attachments(&removed, &apart)
            .unwrap_err();
        assert_eq!(format!("{refused:#}"), "replacement copper is disconnected");
    }

    /// The classes of a net are those that name it exactly and those whose
    /// pattern matches it under KiCad's wildcard reading; the widest sets the
    /// floor, and Default is the class of a net that nothing names.
    #[test]
    fn route_refine_width_floor_follows_the_classes_that_name_the_net() {
        let mut rules = test_scene(Vec::new()).rules;
        rules.min_width = 0.1;
        rules.classes = BTreeMap::from([
            ("Default".to_string(), 0.25),
            ("Narrow".to_string(), 0.15),
            ("Wide".to_string(), 0.4),
            ("Widest".to_string(), 0.6),
        ]);
        rules.assignments = BTreeMap::from([("SIG".to_string(), vec!["Narrow".to_string()])]);
        rules.patterns = vec![("PWR*".to_string(), "Wide".to_string())];
        assert_eq!(rules.width("OTHER"), 0.25, "no class names it: Default");
        assert_eq!(rules.width("SIG"), 0.15, "assigned class replaces Default");
        assert_eq!(rules.width("PWR_5V"), 0.4, "pattern class replaces Default");
        assert_eq!(rules.width("XPWR"), 0.25, "the whole name must match");
        assert_eq!(rules.width("pwr_5v"), 0.25, "case counts");
        rules
            .assignments
            .insert("PWR_5V".into(), vec!["Narrow".into()]);
        assert_eq!(rules.width("PWR_5V"), 0.4, "widest of its classes");
        rules.patterns.push(("PWR_?V".into(), "Widest".into()));
        assert_eq!(rules.width("PWR_5V"), 0.6, "widest of its classes");
        assert_eq!(rules.width("PWR_12V"), 0.4, "? is exactly one character");
        // Every character but * and ? is literal.
        rules.patterns = vec![
            ("+5V".into(), "Wide".into()),
            ("Net-(R1-Pad1)".into(), "Wide".into()),
            ("D[0]".into(), "Wide".into()),
            ("A\\B".into(), "Wide".into()),
            ("^USB.*$".into(), "Narrow".into()),
        ];
        for net in ["+5V", "Net-(R1-Pad1)", "D[0]", "A\\B"] {
            assert_eq!(rules.width(net), 0.4, "{net}");
        }
        for net in [
            "5V",
            "++5V",
            "55V",
            "Net-R1-Pad1",
            "D0",
            "AB",
            "USB_DP",
            "USB",
        ] {
            assert_eq!(rules.width(net), 0.25, "{net}");
        }
        assert_eq!(rules.width("^USB.x$"), 0.15, "the literal reading");
        // KiCad puts copper without a net in the Default class.
        rules.patterns = vec![("*".into(), "Wide".into())];
        assert_eq!(rules.width("ANY"), 0.4);
        assert_eq!(rules.width(""), 0.25);
        rules.min_width = 0.5;
        assert_eq!(rules.width("ANY"), 0.5, "never below the board minimum");
        assert_eq!(rules.width(""), 0.5);
    }

    /// Lock resolution expands each group at most once, in any listing order,
    /// and ends on a cycle.
    #[test]
    fn route_refine_nested_locks_resolve_in_one_pass_over_the_groups() {
        let name = |i: usize| format!("g{i}");
        // A chain 2,000 deep, innermost group listed first, only the outermost locked.
        let mut groups = vec![(name(0), vec!["track".to_string(), "other".to_string()])];
        groups.extend((1..2000).map(|i| (name(i), vec![name(i - 1)])));
        groups.push(("free".into(), vec!["untouched".into()]));
        let mut locks = BTreeSet::from([name(1999)]);
        let visits = super::resolve_locks(&mut locks, &groups);
        assert!(locks.contains("track") && locks.contains("other"));
        assert!(!locks.contains("untouched") && !locks.contains("free"));
        assert_eq!(locks.len(), 2000 + 2);
        assert_eq!(visits, 2000, "each locked group is expanded exactly once");
        // Two groups that contain each other.
        let cycle = vec![
            ("a".to_string(), vec!["b".to_string(), "x".to_string()]),
            ("b".to_string(), vec!["a".to_string(), "y".to_string()]),
        ];
        let mut locks = BTreeSet::from(["a".to_string()]);
        assert_eq!(super::resolve_locks(&mut locks, &cycle), 2);
        assert_eq!(locks.len(), 4);
        // Two groups that share an id, inside a locked group.
        let shared = vec![
            ("x".to_string(), vec!["first".to_string()]),
            ("x".to_string(), vec!["second".to_string()]),
            ("outer".to_string(), vec!["x".to_string()]),
        ];
        let mut locks = BTreeSet::from(["outer".to_string()]);
        super::resolve_locks(&mut locks, &shared);
        assert!(locks.contains("first") && locks.contains("second"));
    }

    #[test]
    fn route_refine_locked_copper_is_never_rewritten() {
        let mut leg = Copper::track(
            "leg".into(),
            "GND".into(),
            "F.Cu".into(),
            1,
            (5., 5.),
            (8., 5.),
            0.2,
        );
        let removed = BTreeSet::from(["leg".to_string()]);
        let mut scene = test_scene(vec![leg.clone()]);
        assert!(scene.attachments(&removed, &[leg.clone()]).is_ok());
        leg.locked = true;
        let mut scene = test_scene(vec![leg.clone()]);
        let refused = scene.attachments(&removed, &[leg]).unwrap_err();
        assert_eq!(format!("{refused:#}"), "locked or unsupported edit");
    }

    #[test]
    fn route_refine_margin_band_is_inherited_only_from_replaced_exact_copper() {
        let (mut scene, connector, removed) = band(0.2, |_| {});
        assert_eq!(
            scene.safe(&connector, &removed, &[], &[0]).unwrap(),
            1,
            "the saved leg already ran at this exact clearance"
        );
        let (mut scene, connector, removed) = band(0.2, |_| {});
        assert!(
            scene.safe(&connector, &removed, &[], &[]).is_err(),
            "nothing replaced: the full margin applies"
        );
        let (mut scene, connector, _) = band(0.2, |_| {});
        assert!(
            scene.safe(&connector, &BTreeSet::new(), &[], &[0]).is_err(),
            "copper that stays on the board passes nothing on"
        );
        for (name, change) in [
            (
                "a curve approximation",
                (|c| c.exact = false) as fn(&mut Copper),
            ),
            ("a keep-out", |c| c.kind = "keepout"),
            ("copper graphics", |c| c.kind = "graphic"),
        ] {
            let (mut scene, connector, removed) = band(0.2, change);
            assert!(
                scene.safe(&connector, &removed, &[], &[0]).is_err(),
                "{name} keeps the margin"
            );
        }
        let (mut scene, connector, removed) = band(0.199, |_| {});
        assert!(
            scene.safe(&connector, &removed, &[], &[0]).is_err(),
            "an existing shortfall is never extended to new copper"
        );
        let (mut scene, mut via, removed) = band(0.2, |_| {});
        via.kind = "via";
        via.shape = Shape::disc((5.5, 6.), 0.1);
        via.drill = Some(0.05);
        scene.rules.annular = 0.01;
        assert!(
            scene.safe(&via, &removed, &[], &[0]).is_err(),
            "only replacement track copper inherits"
        );
        // Closer than the saved leg was, though still at the nominal rule.
        let mut scene = super::test_scene(vec![
            straight("leg", "SIG", (4., 6.), (6., 6.), 0.2),
            straight("foreign", "OTHER", (5.4, 6.6), (5.35, 6.65), 0.38),
        ]);
        let removed = BTreeSet::from(["leg".to_string()]);
        let near = straight("new", "SIG", (5.5, 6.), (6., 6.5), 0.2);
        let gap = scene
            .distance(&near.shape, &scene.items[1].shape.clone())
            .unwrap();
        assert!(
            (0.2..0.21).contains(&gap),
            "fixture must sit in the band: {gap}"
        );
        assert!(
            scene.safe(&near, &removed, &[], &[0]).is_err(),
            "new copper may not come nearer than the saved copper was"
        );
        let far = straight("new", "SIG", (5.75, 6.), (6., 6.25), 0.2);
        assert_eq!(scene.safe(&far, &removed, &[], &[0]).unwrap(), 0);
    }

    // An open via in a pad wicks solder; same-net contact is no excuse.
    #[test]
    fn route_refine_new_via_never_lands_on_a_same_net_pad() {
        let pad = || Copper {
            id: "pad".into(),
            net: Some("GND".into()),
            shape: Shape::polygon(vec![(9.6, 9.6), (10.4, 9.6), (10.4, 10.4), (9.6, 10.4)]),
            mask: 1,
            kind: "pad",
            locked: false,
            drill: None,
            layer: "F.Cu".into(),
            exact: true,
        };
        for (x, expected) in [(10., false), (10.6, false), (10.705, false), (10.72, true)] {
            let mut scene = scene(vec![pad()]);
            let result = scene.safe(&via((x, 10.), 0.6, 0.3), &BTreeSet::new(), &[], &[]);
            assert_eq!(result.is_ok(), expected, "via at x={x}: {result:?}");
            if !expected {
                assert!(result
                    .unwrap_err()
                    .to_string()
                    .starts_with("via-in-pad overlap"));
            }
        }
        // A same-net track may still end on its own pad.
        let mut scene = scene(vec![pad()]);
        let track = straight("t", "GND", (10., 10.), (14., 10.), 0.2);
        assert!(scene.safe(&track, &BTreeSet::new(), &[], &[]).is_ok());
    }

    // KiCad's fractured ring: the double bridge contributes no copper area, and
    // a stitch needs its whole annulus plus the margin inside the fill.
    #[test]
    fn route_refine_fill_holes_require_the_entire_via_annulus_to_fit() {
        let hole = vec![
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
        // Densify so the cached point index and edge tree are the code under test.
        let mut dense = Vec::new();
        for w in hole.windows(2) {
            for k in 0..6 {
                let t = f64::from(k) / 6.;
                dense.push((
                    w[0].0 + t * (w[1].0 - w[0].0),
                    w[0].1 + t * (w[1].1 - w[0].1),
                ));
            }
        }
        for ring in [hole, dense] {
            let mut scene = scene(vec![Copper {
                id: "fill".into(),
                net: Some("GND".into()),
                shape: Shape::polygon(ring),
                mask: 2,
                kind: "fill",
                locked: true,
                drill: None,
                layer: "B.Cu".into(),
                exact: true,
            }]);
            let fills = scene.fills_of("GND").unwrap();
            assert_eq!(fills, [0]);
            for (x, inside) in [
                (10., false),
                (8.8, false),
                (8.695, false),
                (8.68, true),
                (8.6, true),
                (2.305, false),
                (2.32, true),
            ] {
                let found = scene
                    .containing_fills(&fills, &Shape::disc((x, 10.), 0.3))
                    .unwrap();
                assert_eq!(!found.is_empty(), inside, "via centre x={x}");
            }
            assert!(scene.fills_of("OTHER").unwrap().is_empty());
        }
    }

    // Catches a hole prefilter that is narrower than the rule it guards.
    #[test]
    fn route_refine_hole_prefilter_never_hides_a_hole_inside_the_rule() {
        for (x, expected) in [(10.5, false), (10.659, false), (10.661, true)] {
            let mut scene = scene(Vec::new());
            scene.rules.hole = 0.2;
            scene.rules.hole_to_hole = 0.05;
            scene.holes.push(Hole {
                shape: Shape::disc((x, 10.), 0.15),
                mask: 15,
                plated_net: None,
            });
            scene
                .hole_boxes
                .push((x - 0.15, 10. - 0.15, x + 0.15, 10. + 0.15));
            assert_eq!(
                scene
                    .safe(&via((10., 10.), 0.6, 0.3), &BTreeSet::new(), &[], &[])
                    .is_ok(),
                expected,
                "unplated hole at x={x}"
            );
        }
    }

    // After a proposal is accepted, every per-item table must still line up.
    #[test]
    fn route_refine_accept_keeps_items_boxes_and_caches_aligned() {
        let ring = |cx: f64| -> Vec<Pt> {
            (0..64)
                .map(|i| {
                    let a = f64::from(i) * std::f64::consts::TAU / 64.;
                    (cx + a.cos(), 20. + a.sin())
                })
                .collect()
        };
        let fill = |id: &str, cx: f64| Copper {
            id: id.into(),
            net: Some("OTHER".into()),
            shape: Shape::polygon(ring(cx)),
            mask: 1,
            kind: "fill",
            locked: true,
            drill: None,
            layer: "F.Cu".into(),
            exact: true,
        };
        let mut scene = scene(vec![
            fill("left", 5.),
            straight("gone", "SIG", (10., 10.), (12., 10.), 0.2),
            fill("right", 25.),
        ]);
        // Warm both caches, then drop the item between them.
        for x in [5., 25.] {
            let probe = straight("probe", "SIG", (x - 1., 21.15), (x + 1., 21.15), 0.2);
            assert!(scene.safe(&probe, &BTreeSet::new(), &[], &[]).is_err());
        }
        assert!(scene.prepared[0].is_some() && scene.prepared[2].is_some());
        let created = vec![via((15., 5.), 0.6, 0.3)];
        scene.accept(&BTreeSet::from(["gone".to_string()]), created);
        let ids: Vec<_> = scene.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["left", "right", "new"]);
        assert_eq!(scene.boxes.len(), 3);
        assert_eq!(scene.prepared.len(), 3);
        assert_eq!(scene.holes.len(), scene.hole_boxes.len());
        for (i, item) in scene.items.iter().enumerate() {
            assert_eq!(scene.boxes[i], item.shape.bbox().unwrap(), "{}", item.id);
        }
        // The cache that moved from slot 2 to slot 1 must still describe "right".
        let near_right = straight("probe", "SIG", (23.7, 20.), (23.8, 20.), 0.2);
        let near_left = straight("probe", "SIG", (3.7, 20.), (3.8, 20.), 0.2);
        for (probe, name) in [(near_right, "right"), (near_left, "left")] {
            let error = scene
                .safe(&probe, &BTreeSet::new(), &[], &[])
                .unwrap_err()
                .to_string();
            assert_eq!(error, format!("clearance to {name}"));
        }
        assert!(scene
            .safe(
                &straight("probe", "SIG", (14., 20.), (16., 20.), 0.2),
                &BTreeSet::new(),
                &[],
                &[]
            )
            .is_ok());
    }
}
