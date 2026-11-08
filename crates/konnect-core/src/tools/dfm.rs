//! The geometric checks behind `audit_manufacturing`.
//!
//! Each check reads the saved board ([`BoardGeometry`]) and the fab limits,
//! and says what it covered. What KiCad's own DRC already proves (silkscreen
//! over copper, courtyard overlap, copper clearance, hole-to-hole) is named as
//! delegated instead of re-implemented, and what nothing checks is named as
//! not checked, so a clean result never claims more than it looked at.

use std::collections::{BTreeMap, HashMap};

use konnect_sexp::board_geometry::{BoardGeometry, Entry, Kind, Shape};
use serde_json::{json, Value};

use super::design_intent::FabTarget;

/// Tolerance for "below the minimum": KiCad stores nanometres, files print
/// them in mm, and 0.127 must not fail against 0.127.
const EPS_MM: f64 = 1e-6;

/// How many example UUIDs a grouped finding carries.
const EXAMPLE_UUIDS: usize = 5;

/// One fab capability limit and where its value came from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limit {
    pub mm: f64,
    /// `built_in_table`, `design_intent` or `generic_default`.
    pub source: &'static str,
}

/// The limits a board is checked against.
#[derive(Debug, Clone)]
pub(crate) struct FabLimits {
    pub house: String,
    pub min_track: Limit,
    pub min_clearance: Limit,
    pub min_via_drill: Limit,
    pub min_via_diameter: Limit,
    pub min_annular: Limit,
    pub min_pth_drill: Limit,
    pub notes: Vec<String>,
}

impl FabLimits {
    pub fn to_json(&self) -> Value {
        let one = |l: Limit| json!({ "mm": l.mm, "source": l.source });
        json!({
            "house": self.house,
            "min_track_mm": one(self.min_track),
            "min_clearance_mm": one(self.min_clearance),
            "min_via_drill_mm": one(self.min_via_drill),
            "min_via_diameter_mm": one(self.min_via_diameter),
            "min_annular_ring_mm": one(self.min_annular),
            "min_pth_drill_mm": one(self.min_pth_drill),
            "notes": self.notes,
        })
    }
}

/// The houses with built-in values: (track, clearance, drill, annular), mm.
/// Standard (not advanced) capability; confirm against the fab's current page
/// before relying on a margin this thin.
fn table(house: &str) -> Option<(f64, f64, f64, f64)> {
    Some(match house {
        "jlcpcb" => (0.127, 0.127, 0.3, 0.13),
        "pcbway" => (0.1, 0.1, 0.2, 0.1),
        "oshpark" => (0.152, 0.152, 0.254, 0.127),
        _ => return None,
    })
}

/// The limits for `requested` (the caller's `fab_house`, if given), with the
/// design-intent fab target's own minimums taking precedence when it targets
/// the same house (or names none).
pub(crate) fn fab_limits(requested: Option<&str>, intent: Option<&FabTarget>) -> FabLimits {
    let mut notes = Vec::new();
    let intent_house = intent
        .and_then(|t| t.house.as_deref())
        .map(|h| h.trim().to_lowercase());
    let house = match (requested, intent_house.as_deref()) {
        (Some(r), _) => r.trim().to_lowercase(),
        (None, Some(h)) if table(h).is_some() => h.to_string(),
        (None, Some(h)) => {
            notes.push(format!(
                "design intent names fab house '{h}', which has no built-in limits; using generic defaults"
            ));
            h.to_string()
        }
        (None, None) => "jlcpcb".to_string(),
    };
    let (values, source) = match table(&house) {
        Some(v) => (v, "built_in_table"),
        None => {
            if requested.is_some() {
                notes.push(format!(
                    "'{house}' has no built-in limits; using generic defaults"
                ));
            }
            ((0.15, 0.15, 0.3, 0.13), "generic_default")
        }
    };
    let (track, clearance, drill, annular) = values;
    let limit = |mm: f64| Limit { mm, source };
    let mut limits = FabLimits {
        house: house.clone(),
        min_track: limit(track),
        min_clearance: limit(clearance),
        min_via_drill: limit(drill),
        min_via_diameter: limit(drill + 2.0 * annular),
        min_annular: limit(annular),
        min_pth_drill: limit(drill),
        notes,
    };

    if let Some(target) = intent {
        let same_house = intent_house.as_deref().is_none_or(|h| h == house);
        if !same_house {
            limits.notes.push(format!(
                "design intent targets '{}', not '{house}': its minimums were not applied",
                intent_house.unwrap_or_default()
            ));
        } else {
            let from_intent = |mm: f64| Limit {
                mm,
                source: "design_intent",
            };
            if let Some(v) = target.min_track_mm {
                limits.min_track = from_intent(v);
            }
            if let Some(v) = target.min_clearance_mm {
                limits.min_clearance = from_intent(v);
            }
            if let Some(v) = target.min_via_drill_mm {
                limits.min_via_drill = from_intent(v);
            }
            if let Some(v) = target.min_via_diameter_mm {
                limits.min_via_diameter = from_intent(v);
            }
        }
    }
    limits
}

/// One check's coverage line.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Coverage {
    pub status: &'static str,
    pub checked: usize,
    pub failed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Coverage {
    fn checked(checked: usize, failed: usize) -> Self {
        Coverage {
            status: "checked",
            checked,
            failed,
            reason: None,
        }
    }

    pub fn not_checked(reason: impl Into<String>) -> Self {
        Coverage {
            status: "not_checked",
            checked: 0,
            failed: 0,
            reason: Some(reason.into()),
        }
    }
}

/// A finding before it becomes the audit's JSON.
#[derive(Debug, Clone)]
pub(crate) struct DfmIssue {
    pub severity: &'static str,
    pub component: Option<String>,
    pub issue: String,
    pub fix: String,
    /// Where it is: example UUIDs and a position, so the reader can go there.
    pub at: Value,
}

/// What every check produced.
#[derive(Debug, Default)]
pub(crate) struct DfmReport {
    pub issues: Vec<DfmIssue>,
    pub checks: BTreeMap<&'static str, Coverage>,
}

fn group_uuids<'a>(items: impl Iterator<Item = &'a Entry>) -> Vec<String> {
    items
        .filter_map(|e| e.uuid.clone())
        .take(EXAMPLE_UUIDS)
        .collect()
}

fn prop(entry: &Entry, key: &str) -> Option<f64> {
    entry.props.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn pt_json(p: (f64, f64)) -> Value {
    json!({ "x": round_um(p.0), "y": round_um(p.1) })
}

fn round_um(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Run every geometric check against `limits`.
pub(crate) fn check_board(geometry: &BoardGeometry, limits: &FabLimits) -> DfmReport {
    let mut report = DfmReport::default();
    check_track_widths(geometry, limits, &mut report);
    check_vias(geometry, limits, &mut report);
    check_pad_holes(geometry, limits, &mut report);
    check_via_in_pad(geometry, &mut report);
    check_acid_traps(geometry, &mut report);
    report
}

fn check_track_widths(geometry: &BoardGeometry, limits: &FabLimits, report: &mut DfmReport) {
    let tracks: Vec<&Entry> = geometry
        .others
        .iter()
        .filter(|e| e.kind == Kind::Track)
        .collect();
    let mut narrow: BTreeMap<(String, String), Vec<&Entry>> = BTreeMap::new();
    for t in &tracks {
        if prop(t, "width").is_some_and(|w| w < limits.min_track.mm - EPS_MM) {
            let key = (
                t.net.clone().unwrap_or_default(),
                t.layers.first().cloned().unwrap_or_default(),
            );
            narrow.entry(key).or_default().push(t);
        }
    }
    let mut failed = 0;
    for ((net, layer), items) in &narrow {
        failed += items.len();
        let narrowest = items
            .iter()
            .filter_map(|t| prop(t, "width"))
            .fold(f64::INFINITY, f64::min);
        report.issues.push(DfmIssue {
            severity: "error",
            component: None,
            issue: format!(
                "{} track segment(s) on net '{}', layer {}, are below the {} minimum track \
                 width {:.3} mm (narrowest {:.3} mm)",
                items.len(),
                net_label(net),
                layer,
                limits.house,
                limits.min_track.mm,
                narrowest
            ),
            fix: format!("Widen them to at least {:.3} mm", limits.min_track.mm),
            at: json!({ "net": net, "layer": layer, "uuids": group_uuids(items.iter().copied()) }),
        });
    }
    report
        .checks
        .insert("track_width", Coverage::checked(tracks.len(), failed));
}

fn net_label(net: &str) -> &str {
    if net.is_empty() {
        "(no net)"
    } else {
        net
    }
}

fn check_vias(geometry: &BoardGeometry, limits: &FabLimits, report: &mut DfmReport) {
    let vias: Vec<&Entry> = geometry
        .others
        .iter()
        .filter(|e| e.kind == Kind::Via)
        .collect();
    // Group by the defect and the via size, so 400 identical vias are one line.
    let mut groups: BTreeMap<(&'static str, i64, i64), Vec<&Entry>> = BTreeMap::new();
    for v in &vias {
        let (Some(size), Some(drill)) = (prop(v, "size"), prop(v, "drill")) else {
            continue;
        };
        let key = |what| (what, (size * 1e6) as i64, (drill * 1e6) as i64);
        if drill < limits.min_via_drill.mm - EPS_MM {
            groups.entry(key("drill")).or_default().push(v);
        }
        if size < limits.min_via_diameter.mm - EPS_MM {
            groups.entry(key("diameter")).or_default().push(v);
        }
        if (size - drill) / 2.0 < limits.min_annular.mm - EPS_MM {
            groups.entry(key("annular")).or_default().push(v);
        }
    }
    let mut failed = std::collections::BTreeSet::new();
    for ((what, size_nm, drill_nm), items) in &groups {
        let (size, drill) = (*size_nm as f64 / 1e6, *drill_nm as f64 / 1e6);
        for item in items {
            failed.insert(
                item.uuid
                    .clone()
                    .unwrap_or_else(|| format!("{:?}", item.at)),
            );
        }
        let (issue, fix) = match *what {
            "drill" => (
                format!(
                    "{} via(s) of {:.3}/{:.3} mm (diameter/drill) have a drill below the {} \
                     minimum {:.3} mm",
                    items.len(),
                    size,
                    drill,
                    limits.house,
                    limits.min_via_drill.mm
                ),
                format!("Use a drill of at least {:.3} mm", limits.min_via_drill.mm),
            ),
            "diameter" => (
                format!(
                    "{} via(s) of {:.3}/{:.3} mm (diameter/drill) are below the {} minimum via \
                     diameter {:.3} mm",
                    items.len(),
                    size,
                    drill,
                    limits.house,
                    limits.min_via_diameter.mm
                ),
                format!(
                    "Use a via diameter of at least {:.3} mm",
                    limits.min_via_diameter.mm
                ),
            ),
            _ => (
                format!(
                    "{} via(s) of {:.3}/{:.3} mm (diameter/drill) have a {:.3} mm annular ring, \
                     below the {} minimum {:.3} mm",
                    items.len(),
                    size,
                    drill,
                    (size - drill) / 2.0,
                    limits.house,
                    limits.min_annular.mm
                ),
                format!(
                    "Grow the diameter to at least {:.3} mm for this drill",
                    drill + 2.0 * limits.min_annular.mm
                ),
            ),
        };
        report.issues.push(DfmIssue {
            severity: "error",
            component: None,
            issue,
            fix,
            at: json!({ "uuids": group_uuids(items.iter().copied()), "first": pt_json(items[0].at) }),
        });
    }
    report
        .checks
        .insert("via_size", Coverage::checked(vias.len(), failed.len()));
}

fn check_pad_holes(geometry: &BoardGeometry, limits: &FabLimits, report: &mut DfmReport) {
    let mut checked = 0;
    let mut failed = 0;
    for fp in &geometry.footprints {
        let reference = fp.reference.clone().unwrap_or_else(|| "?".to_string());
        let mut small_drill = Vec::new();
        let mut thin_ring = Vec::new();
        for pad in fp.pads.iter().filter(|p| p.through_hole) {
            let Some(drill) = pad.drill else { continue };
            checked += 1;
            if drill < limits.min_pth_drill.mm - EPS_MM {
                small_drill.push((pad.number.clone(), drill));
            }
            // A plated pad's ring; a custom pad's size is not its copper.
            if pad.kind == "thru_hole" && !pad.approximate {
                let ring = (pad.size.0.min(pad.size.1) - drill) / 2.0;
                if ring < limits.min_annular.mm - EPS_MM {
                    thin_ring.push((pad.number.clone(), ring));
                }
            }
        }
        if !small_drill.is_empty() {
            failed += small_drill.len();
            let smallest = small_drill
                .iter()
                .map(|p| p.1)
                .fold(f64::INFINITY, f64::min);
            report.issues.push(DfmIssue {
                severity: "error",
                component: Some(reference.clone()),
                issue: format!(
                    "{reference}: {} hole(s) below the {} minimum drill {:.3} mm (smallest \
                     {smallest:.3} mm, pads {})",
                    small_drill.len(),
                    limits.house,
                    limits.min_pth_drill.mm,
                    pad_list(&small_drill)
                ),
                fix: "Use a footprint with larger holes, or confirm the fab's small-hole option"
                    .to_string(),
                at: pt_json(fp.at),
            });
        }
        if !thin_ring.is_empty() {
            failed += thin_ring.len();
            let thinnest = thin_ring.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
            report.issues.push(DfmIssue {
                severity: "error",
                component: Some(reference.clone()),
                issue: format!(
                    "{reference}: {} plated pad(s) have an annular ring below the {} minimum \
                     {:.3} mm (thinnest {thinnest:.3} mm, pads {})",
                    thin_ring.len(),
                    limits.house,
                    limits.min_annular.mm,
                    pad_list(&thin_ring)
                ),
                fix: "Grow the pad or shrink the drill".to_string(),
                at: pt_json(fp.at),
            });
        }
    }
    report
        .checks
        .insert("pad_holes", Coverage::checked(checked, failed));
}

fn pad_list(pads: &[(String, f64)]) -> String {
    let mut names: Vec<&str> = pads.iter().map(|p| p.0.as_str()).collect();
    names.dedup();
    if names.len() > 8 {
        format!("{}, … {} more", names[..8].join(", "), names.len() - 8)
    } else {
        names.join(", ")
    }
}

/// Via-in-pad: a via whose copper touches an SMD pad on a shared layer.
/// Same net: solder wicks down the barrel unless the via is filled and capped.
/// Another net: a short, which `run_drc` reports too.
fn check_via_in_pad(geometry: &BoardGeometry, report: &mut DfmReport) {
    let vias: Vec<&Entry> = geometry
        .others
        .iter()
        .filter(|e| e.kind == Kind::Via)
        .collect();
    let mut failed = 0;
    for fp in &geometry.footprints {
        let reference = fp.reference.clone().unwrap_or_else(|| "?".to_string());
        // pad number → (same-net via uuids, other-net)
        let mut hits: BTreeMap<String, (Vec<String>, bool)> = BTreeMap::new();
        for pad in fp.pads.iter().filter(|p| p.kind == "smd") {
            for via in &vias {
                let shares_layer = pad
                    .layers
                    .iter()
                    .any(|l| l.ends_with(".Cu") && via_on_layer(via, l));
                if !shares_layer || !via.shape.intersects(&pad.shape) {
                    continue;
                }
                let entry = hits.entry(pad.number.clone()).or_default();
                let same = pad.net.is_some() && pad.net == via.net;
                if !same {
                    entry.1 = true;
                }
                entry
                    .0
                    .push(via.uuid.clone().unwrap_or_else(|| format!("{:?}", via.at)));
            }
        }
        for (pad, (uuids, other_net)) in hits {
            failed += 1;
            let (severity, issue, fix) = if other_net {
                (
                    "error",
                    format!("{reference} pad {pad}: a via of another net overlaps the pad copper (a short)"),
                    "Move the via off the pad; run_drc reports this short as well".to_string(),
                )
            } else {
                (
                    "warning",
                    format!("{reference} pad {pad}: via-in-pad ({} via(s))", uuids.len()),
                    "Order filled and capped vias (VIPPO) or move the via beside the pad, or \
                     solder will wick into the barrel"
                        .to_string(),
                )
            };
            report.issues.push(DfmIssue {
                severity,
                component: Some(reference.clone()),
                issue,
                fix,
                at: json!({ "pad": pad, "via_uuids": uuids.into_iter().take(EXAMPLE_UUIDS).collect::<Vec<_>>() }),
            });
        }
    }
    let smd_pads = geometry
        .footprints
        .iter()
        .flat_map(|f| &f.pads)
        .filter(|p| p.kind == "smd")
        .count();
    report
        .checks
        .insert("via_in_pad", Coverage::checked(smd_pads, failed));
}

/// Whether a via has copper on `layer`. A through via names only its outer
/// layers, so any copper layer between them counts.
fn via_on_layer(via: &Entry, layer: &str) -> bool {
    via.layers.iter().any(|l| l == layer)
        || (via.layers.iter().any(|l| l == "F.Cu") && via.layers.iter().any(|l| l == "B.Cu"))
}

/// Acid traps: two tracks of one net on one layer that meet end to end at an
/// interior angle under 90°. Junctions inside a pad or via of that net are
/// skipped (the pad's copper fills the wedge), and a track meeting another
/// mid-segment is not a shared endpoint and is not examined.
/// (layer, net, x µm, y µm): where track ends meet.
type JunctionKey = (String, String, i64, i64);

fn check_acid_traps(geometry: &BoardGeometry, report: &mut DfmReport) {
    // (layer, net, x µm, y µm) → unit directions leaving the junction
    let mut ends: HashMap<JunctionKey, Vec<(f64, f64)>> = HashMap::new();
    for t in geometry.others.iter().filter(|e| e.kind == Kind::Track) {
        let pts = &t.shape.pts;
        if pts.len() < 2 {
            continue;
        }
        let layer = t.layers.first().cloned().unwrap_or_default();
        let net = t.net.clone().unwrap_or_default();
        let n = pts.len();
        for (end, next) in [(pts[0], pts[1]), (pts[n - 1], pts[n - 2])] {
            let (dx, dy) = (next.0 - end.0, next.1 - end.1);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-9 {
                continue;
            }
            let key = (
                layer.clone(),
                net.clone(),
                (end.0 * 1000.0).round() as i64,
                (end.1 * 1000.0).round() as i64,
            );
            ends.entry(key).or_default().push((dx / len, dy / len));
        }
    }

    let copper_of_net = |net: &str, layer: &str, p: (f64, f64)| -> bool {
        let point = Shape::disc(p, 0.0);
        geometry.footprints.iter().flat_map(|f| &f.pads).any(|pad| {
            pad.net.as_deref() == Some(net) && pad.on_layer(layer) && pad.shape.intersects(&point)
        }) || geometry.others.iter().any(|e| {
            e.kind == Kind::Via
                && e.net.as_deref() == Some(net)
                && via_on_layer(e, layer)
                && e.shape.intersects(&point)
        })
    };

    let mut junctions = 0;
    let mut skipped = 0;
    let mut traps: Vec<(JunctionKey, f64)> = Vec::new();
    for (key, dirs) in &ends {
        if dirs.len() < 2 {
            continue;
        }
        junctions += 1;
        let mut sharpest = 180.0f64;
        for i in 0..dirs.len() {
            for j in (i + 1)..dirs.len() {
                let dot = (dirs[i].0 * dirs[j].0 + dirs[i].1 * dirs[j].1).clamp(-1.0, 1.0);
                sharpest = sharpest.min(dot.acos().to_degrees());
            }
        }
        if sharpest < 90.0 - 1e-6 {
            let p = (key.2 as f64 / 1000.0, key.3 as f64 / 1000.0);
            if copper_of_net(&key.1, &key.0, p) {
                skipped += 1;
                continue;
            }
            traps.push((key.clone(), sharpest));
        }
    }
    traps.sort_by(|a, b| a.0.cmp(&b.0));
    for ((layer, net, x, y), angle) in &traps {
        report.issues.push(DfmIssue {
            severity: "warning",
            component: None,
            issue: format!(
                "Acid trap: tracks of net '{}' meet at {angle:.0}° on {layer}",
                net_label(net)
            ),
            fix: "Re-route the junction so the tracks meet at 90° or more (a 45° chamfer or a \
                  teardrop)"
                .to_string(),
            at: json!({ "net": net, "layer": layer, "x": *x as f64 / 1000.0, "y": *y as f64 / 1000.0 }),
        });
    }
    let mut coverage = Coverage::checked(junctions, traps.len());
    if skipped > 0 {
        coverage.reason = Some(format!(
            "{skipped} acute junction(s) inside a pad or via of the same net were not counted"
        ));
    }
    report.checks.insert("acid_trap", coverage);
}

/// The project's own DRC minimums (`board.design_settings.rules` in the
/// `.kicad_pro`) set below what the fab can make: DRC would then pass copper
/// the fab cannot build.
pub(crate) fn check_rule_minimums(project: &Value, limits: &FabLimits, report: &mut DfmReport) {
    let rules = &project["board"]["design_settings"]["rules"];
    let mut checked = 0;
    let mut failed = 0;
    for (key, what, limit) in [
        ("min_track_width", "minimum track width", limits.min_track),
        ("min_clearance", "minimum clearance", limits.min_clearance),
        (
            "min_via_diameter",
            "minimum via diameter",
            limits.min_via_diameter,
        ),
        (
            "min_through_hole_diameter",
            "minimum through-hole drill",
            limits.min_via_drill,
        ),
        (
            "min_via_annular_width",
            "minimum via annular width",
            limits.min_annular,
        ),
    ] {
        let Some(value) = rules[key].as_f64() else {
            continue;
        };
        checked += 1;
        if value < limit.mm - EPS_MM {
            failed += 1;
            report.issues.push(DfmIssue {
                severity: "warning",
                component: None,
                issue: format!(
                    "The project's {what} rule ({value:.3} mm) is below the {} capability \
                     ({:.3} mm), so DRC will pass copper the fab cannot make",
                    limits.house, limit.mm
                ),
                fix: format!("Raise {key} to at least {:.3} mm", limit.mm),
                at: json!({ "rule": key }),
            });
        }
    }
    report
        .checks
        .insert("project_rule_minimums", Coverage::checked(checked, failed));
}

/// What `run_drc` proves and this audit therefore does not re-check.
pub(crate) const DELEGATED_TO_DRC: [&str; 6] = [
    "silk_over_copper",
    "silk_overlap",
    "courtyards_overlap",
    "clearance",
    "hole_to_hole",
    "solder_mask_bridge",
];

/// What nothing in this audit or DRC's delegated list checks.
pub(crate) const NOT_CHECKED: [&str; 4] = [
    "acid traps between a track and a pad edge",
    "acid traps where a track meets another mid-segment",
    "solder-mask slivers between pads",
    "filled/capped status of vias (not stored in the board file)",
];

#[cfg(test)]
#[path = "dfm_tests.rs"]
pub(crate) mod tests;
