//! `score_placement`'s judgement of a parsed board, pulled out of
//! `handle_score_placement` so that a search can score thousands of candidates on
//! threads with no file, no async runtime and no call to KiCad. The handler reads the
//! board (live or saved) as before, calls [`evaluate`] and adds `source`; its response
//! is unchanged (`score_placement_output_is_unchanged_by_the_pull_out`).

use super::{
    ambiguous_pad_references, bbox_center, courtyard_reference, decoupling, footprint_values,
    objectives, ref_prefix, round3, side_name, CONNECTOR_CAP, CONNECTOR_EDGE_LIMIT_MM,
    CONNECTOR_POINTS, DECOUPLING_CAP, DECOUPLING_POINTS, OUTSIDE_CAP, OUTSIDE_POINTS, OVERLAP_CAP,
    OVERLAP_POINTS,
};
use konnect_sexp::board::{
    board_outline_shape, footprint_courtyards, CourtyardSource, OutlineShape, PcbConnectivityIndex,
};
use konnect_sexp::board_geometry::Allowlist;
use konnect_sexp::outline::{Outline, OutlineStatus};
use konnect_sexp::parser::SexpNode;
use serde_json::json;
use std::collections::BTreeSet;

/// Every field of `score_placement`'s response except `source`, computed from `tree`
/// alone, by the code that follows `parse_sexp` in the handler: `verdict`, `score`,
/// `score_incomplete`, `routing_metrics`, `decoupling_coverage`, the outline fields,
/// `edge_mount_listed`, `hard_failures`, `deductions`, `connector_edges`,
/// `uncoupled_caps`, `unproven_decoupling_caps`, `decoupling_associations`,
/// `interface_filter_caps`, `footprints_scored` and `footprints_skipped`.
/// `max_net_references` is already validated (2..=256). Errs only where the handler's
/// `?` did (a coverage record that does not serialize). No I/O.
pub(crate) fn evaluate(
    tree: &SexpNode,
    edge_mount: &Allowlist,
    max_net_references: usize,
) -> anyhow::Result<serde_json::Value> {
    let scan = footprint_courtyards(tree);
    let outline_shape = board_outline_shape(tree);
    // Containment is judged against the true Edge.Cuts outline: its lines,
    // arcs, circles and polygons joined into rings, cut-outs included. That
    // replaces the #594 rule that only a provably axis-aligned rectangle could
    // be checked. The outline is usable when it closes; an open or unreadable
    // one proves nothing and the verdict says so instead of passing.
    let true_outline = Outline::from_tree(tree);
    let outline_status = true_outline.status();
    let outline_closed = outline_status == OutlineStatus::Closed;
    let threshold = true_outline.containment_threshold_mm();
    let index = PcbConnectivityIndex::build(tree);
    let values = footprint_values(tree);

    // ── Hard failure (a): same-side courtyard-bbox overlaps ─────────────────
    // Opposite sides never collide (a Front part and a Back part share x/y but
    // not space), and AnchorOnly bboxes are position-only stand-ins with zero
    // area — a pair involving one is skipped, not scored as touching.
    let mut hard_failures: Vec<serde_json::Value> = Vec::new();
    let mut overlap_refs: BTreeSet<String> = BTreeSet::new();
    let mut overlap_pairs = 0usize;
    for (i, a) in scan.items.iter().enumerate() {
        for b in &scan.items[i + 1..] {
            if a.bbox_source == CourtyardSource::AnchorOnly
                || b.bbox_source == CourtyardSource::AnchorOnly
                || a.layer_side != b.layer_side
            {
                continue;
            }
            let ix = a.bbox.2.min(b.bbox.2) - a.bbox.0.max(b.bbox.0);
            let iy = a.bbox.3.min(b.bbox.3) - a.bbox.1.max(b.bbox.1);
            if ix > 0.0 && iy > 0.0 {
                overlap_pairs += 1;
                let (ra, rb) = (courtyard_reference(a), courtyard_reference(b));
                overlap_refs.insert(ra.to_string());
                overlap_refs.insert(rb.to_string());
                hard_failures.push(json!({
                    "kind": "courtyard_overlap",
                    "references": [ra, rb],
                    "detail": format!(
                        "{ra} and {rb} courtyard bboxes overlap by {} mm² on the {} side",
                        round3(ix * iy),
                        side_name(a.layer_side),
                    ),
                }));
            }
        }
    }

    // ── Hard failure (b): parts outside the board outline ───────────────────
    // No usable outline means this check is blocked, not passed: the response
    // carries `outline_missing` or `outline_unproven` and the verdict can never
    // be "pass". A part named in `edge_mount_references` is reported, not
    // failed.
    let mut outside_refs: Vec<String> = Vec::new();
    let mut edge_mount_listed: Vec<serde_json::Value> = Vec::new();
    if outline_closed {
        for c in &scan.items {
            let (x0, y0, x1, y1) = c.bbox;
            let over = true_outline.overhang(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
            if over.depth_mm <= threshold {
                continue;
            }
            let reference = courtyard_reference(c);
            if edge_mount.allows(reference) {
                edge_mount_listed.push(json!({
                    "reference": reference,
                    "overhang_mm": round3(over.depth_mm),
                }));
                continue;
            }
            outside_refs.push(reference.to_string());
            hard_failures.push(json!({
                "kind": "outside_outline",
                "references": [reference],
                "overhang_mm": round3(over.depth_mm),
                "relation": if over.touches_board { "straddles" } else { "outside" },
                "detail": format!(
                    "{reference} courtyard bbox ({}, {})..({}, {}) is not fully inside \
                     the board outline: it overhangs it by {} mm",
                    round3(x0), round3(y0), round3(x1), round3(y1), round3(over.depth_mm),
                ),
            }));
        }
    }

    // ── Connector-edge check (J*) ───────────────────────────────────────────
    // A connector whose bbox center is further than the limit from *every*
    // outline edge cannot be reached by a cable without a tunnel. Blocked
    // without an outline, like the outside check.
    let mut connector_edges: Vec<serde_json::Value> = Vec::new();
    let mut connector_violations: Vec<(String, f64)> = Vec::new();
    if outline_closed {
        for c in &scan.items {
            let Some(reference) = c.reference.as_deref() else {
                continue;
            };
            if ref_prefix(reference) != "J" {
                continue;
            }
            let centre = bbox_center(c.bbox);
            // Distance to the nearest edge of the true outline (a cut-out's
            // edge counts), negative when the centre is off the board.
            let edge_distance = if true_outline.contains(centre) {
                true_outline.boundary_distance(centre)
            } else {
                -true_outline.boundary_distance(centre)
            };
            connector_edges.push(json!({
                "reference": reference,
                "edge_distance_mm": round3(edge_distance),
            }));
            if edge_distance > CONNECTOR_EDGE_LIMIT_MM {
                connector_violations.push((reference.to_string(), edge_distance));
            }
        }
    }

    // ── Decoupling check (C* near a shared-net U*) ──────────────────────────
    let ambiguous_references = ambiguous_pad_references(tree);
    let board_diagonal = true_outline
        .bbox()
        .filter(|_| outline_closed)
        .map(|(x0, y0, x1, y1)| (x1 - x0).hypot(y1 - y0));
    let routing = objectives::assess(
        &index,
        max_net_references,
        board_diagonal,
        &ambiguous_references,
    );
    let decoupling = decoupling::check(
        &scan.items,
        &index,
        &values,
        max_net_references,
        &ambiguous_references,
    );
    let decoupling_coverage = serde_json::to_value(&decoupling.coverage)?;
    let decoupling::Check {
        violations: decoupling_violations,
        uncoupled_caps,
        interface_filter_caps,
        associations: decoupling_associations,
        unproven_caps: unproven_decoupling_caps,
        ..
    } = decoupling;
    let score_incomplete = !routing.complete
        || routing.wire_deduction.is_none()
        || routing.crossing_deduction.is_none()
        || decoupling_coverage["status"] != "complete"
        || !unproven_decoupling_caps.is_empty()
        || scan.skipped > 0;

    // ── Soft score (ported weight table — see the module docs) ──────────────
    let overlap_deduction = (overlap_pairs as i64 * OVERLAP_POINTS).min(OVERLAP_CAP);
    let outside_deduction = (outside_refs.len() as i64 * OUTSIDE_POINTS).min(OUTSIDE_CAP);
    let connector_deduction =
        (connector_violations.len() as i64 * CONNECTOR_POINTS).min(CONNECTOR_CAP);
    let decoupling_deduction =
        (decoupling_violations.len() as i64 * DECOUPLING_POINTS).min(DECOUPLING_CAP);
    let score = (100
        - overlap_deduction
        - outside_deduction
        - connector_deduction
        - decoupling_deduction
        - routing.wire_deduction.unwrap_or(0)
        - routing.crossing_deduction.unwrap_or(0))
    .max(0);

    let mut deductions: Vec<serde_json::Value> = Vec::new();
    if overlap_deduction > 0 {
        deductions.push(json!({
            "kind": "courtyard_overlap",
            "points": overlap_deduction,
            "references": overlap_refs.iter().collect::<Vec<_>>(),
            "detail": format!(
                "{overlap_pairs} same-side courtyard overlap pair(s) — also a hard failure"
            ),
        }));
    }
    if outside_deduction > 0 {
        deductions.push(json!({
            "kind": "outside_outline",
            "points": outside_deduction,
            "references": outside_refs,
            "detail": format!(
                "{} part(s) not fully inside the board outline — also a hard failure",
                hard_failures.iter().filter(|f| f["kind"] == "outside_outline").count(),
            ),
        }));
    }
    if connector_deduction > 0 {
        deductions.push(json!({
            "kind": "connector_edge",
            "points": connector_deduction,
            "references": connector_violations.iter().map(|(r, _)| r).collect::<Vec<_>>(),
            "detail": connector_violations
                .iter()
                .map(|(r, d)| format!(
                    "{r} center is {} mm from the nearest board edge (limit {CONNECTOR_EDGE_LIMIT_MM} mm)",
                    round3(*d),
                ))
                .collect::<Vec<_>>()
                .join("; "),
        }));
    }
    if decoupling_deduction > 0 {
        deductions.push(json!({
            "kind": "decoupling",
            "points": decoupling_deduction,
            "references": decoupling_violations.iter().map(|v| &v.cap).collect::<Vec<_>>(),
            "detail": decoupling_violations
                .iter()
                .map(|v| format!(
                    "{} ({}, limit {} mm) is {} mm from IC {} via {} ({} references)",
                    v.cap,
                    v.value,
                    v.limit_mm,
                    round3(v.distance_mm),
                    v.ic,
                    v.qualifying_net,
                    v.net_reference_count,
                ))
                .collect::<Vec<_>>()
                .join("; "),
        }));
    }

    if let Some(points) = routing.wire_deduction.filter(|points| *points > 0) {
        deductions.push(json!({"kind":"wirelength", "points":points, "detail":"Advisory mean Euclidean MST edge length / board diagonal, scaled by20 and rounded up (cap20); not routed copper length"}));
    }
    if let Some(points) = routing.crossing_deduction.filter(|points| *points > 0) {
        deductions.push(json!({"kind":"airwire_crossings", "points":points, "detail":"Projected proper crossings of different-net straight MST edges, 2 points each (cap20); not shorts or DRC"}));
    }

    // The verdict derives ONLY from hard failures and outline shape — the
    // numeric score never decides it. Precedence: hard failures →
    // outline_missing → outline_unproven → pass.
    let verdict = if !hard_failures.is_empty() {
        "hard_fail"
    } else if outline_status == OutlineStatus::Missing {
        "outline_missing"
    } else if !outline_closed {
        "outline_unproven"
    } else {
        "pass"
    };

    Ok(json!({
        "verdict": verdict,
        "score": score,
        "score_incomplete": score_incomplete,
        "routing_metrics": routing.to_json(),
        "decoupling_coverage": decoupling_coverage,
        "outline_missing": outline_status == OutlineStatus::Missing,
        "outline_unproven": !outline_closed && outline_status != OutlineStatus::Missing,
        "outline_shape": match (outline_status, outline_shape) {
            (OutlineStatus::Missing, _) => "missing",
            (OutlineStatus::Closed, OutlineShape::Rectangular { .. }) => "rectangular",
            (OutlineStatus::Closed, _) => "closed",
            _ => "unproven",
        },
        "outline": crate::tools::board_geometry::outline_json(&true_outline),
        "edge_mount_listed": edge_mount_listed,
        "hard_failures": hard_failures,
        "deductions": deductions,
        "connector_edges": connector_edges,
        "uncoupled_caps": uncoupled_caps,
        "unproven_decoupling_caps": unproven_decoupling_caps
            .iter()
            .map(|cap| json!({
                "reference": cap.cap,
                "value": cap.value,
                "reason": cap.reason,
            }))
            .collect::<Vec<_>>(),
        "decoupling_associations": decoupling_associations
            .iter()
            .map(|association| json!({
                "reference": association.cap,
                "value": association.value,
                "ic": association.ic,
                "qualifying_net": association.qualifying_net,
                "net_reference_count": association.net_reference_count,
                "distance_mm": round3(association.distance_mm),
                "limit_mm": association.limit_mm,
                "status": association.status,
                "association_basis": association.association_basis,
                "metric": "pad_anchor_euclidean_xy",
                "cap_pad": association.cap_pad,
                "ic_pad": association.ic_pad,
                "cap_position": {"x":association.cap_at.0,"y":association.cap_at.1},
                "ic_position": {"x":association.ic_at.0,"y":association.ic_at.1},
            }))
            .collect::<Vec<_>>(),
        "interface_filter_caps": interface_filter_caps
            .iter()
            .map(|f| json!({
                "reference": f.cap,
                "value": f.value,
                "connector": f.connector,
                "connector_distance_mm": round3(f.distance_mm),
                "metric": "worst_nearest_terminal_pad_euclidean_xy",
                "matched_pairs": f.matched_pairs,
                "matched_pair_count": f.matched_pair_count,
                "matched_pairs_truncated": f.matched_pairs_truncated,
                "limit_mm": f.limit_mm,
            }))
            .collect::<Vec<_>>(),
        "footprints_scored": scan.items.len(),
        "footprints_skipped": scan.skipped,
    }))
}
