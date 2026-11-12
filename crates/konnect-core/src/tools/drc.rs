//! One ordering and source-evidence contract for both CLI DRC surfaces.

use super::{cli, ipc_target_error_result, with_board_ipc_classified, ToolContext};
use crate::mcp::{error::ToolErrorKind, protocol::CallToolResult};
use konnect_ipc::IpcFailure;
use serde_json::{json, Value};
use std::{path::Path, time::Duration};

#[path = "zone_fill_defects.rs"]
mod zone_fill_defects;

const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// Publish by sibling-file replacement, never by truncating a destination inode.
/// This also preserves the source board if an output path is a distinct hard link.
pub(crate) async fn write_report(output: &str, contents: &str) -> anyhow::Result<()> {
    use anyhow::Context;
    let path = std::path::PathBuf::from(output);
    let contents = contents.to_string();
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("could not create report directory {}", parent.display())
            })?;
        }
        konnect_sexp::writer::write_atomic(&path, &contents)
            .with_context(|| format!("could not write report to {}", path.display()))
    })
    .await?
}

/// The violations of a DRC result as one page, with the fields that say what the
/// page is and a tally of every violation by rule. Both CLI DRC tools use it, so
/// a large board cannot overflow the client in either (#C3).
pub(crate) fn violations_page(
    filtered: &[&cli::DrcViolation],
    offset: usize,
    limit: usize,
) -> serde_json::Map<String, Value> {
    use super::response_budget as budget;
    let all: Vec<Value> = filtered
        .iter()
        .map(|v| {
            json!({
                "severity": v.severity,
                "rule": v.rule,
                "description": v.description,
                "pos": v.pos.as_ref().map(|p| json!({ "x": p.x, "y": p.y })),
                "items": v.items
            })
        })
        .collect();
    let page = budget::page(&all, offset, limit, budget::BYTE_BUDGET);
    let mut fields = serde_json::Map::new();
    page.describe(&mut fields);
    fields.insert("by_rule".into(), json!(budget::tally(&all, "rule")));
    fields.insert("violations".into(), json!(page.items));
    fields
}

#[derive(serde::Serialize)]
pub(crate) struct DrcSourceEvidence {
    pub source: &'static str,
    pub live_board_synced: bool,
    pub zones_refilled: bool,
    pub zone_refill_source: Option<&'static str>,
    /// Whether the zone fills this DRC read were current (#C24). See
    /// [`zone_fill_check`].
    pub zone_fill: Value,
}

impl DrcSourceEvidence {
    /// `zone_fill_state` and its evidence, for a DRC result's top level.
    pub(crate) fn zone_fill_fields(&self) -> serde_json::Map<String, Value> {
        let mut fields = serde_json::Map::new();
        fields.insert("zone_fill_state".into(), self.zone_fill["state"].clone());
        fields.insert("zone_fill_check".into(), self.zone_fill.clone());
        fields
    }
}

/// Violations listed when a refill changes the DRC result, per direction.
const ZONE_FILL_EXAMPLES: usize = 10;
/// Zones named when their fills differ from a refill.
const ZONE_FILL_ZONES_LISTED: usize = 10;

/// A copper zone of a board (not a rule area) and its saved fill.
struct ZoneFill {
    /// The zone's `uuid`, or its `tstamp` on a KiCad 6/7 board, lowercased.
    id: Option<String>,
    /// Net, layers and outline, written one way: what pairs a zone whose id
    /// changed when kicad-cli saved the refilled copy.
    shape: String,
    /// Inside a footprint. kicad-cli's refill fills board zones only
    /// (`ZONE_FILLER_TOOL::FillAllZones` walks `BOARD::Zones()`), so these
    /// cannot be compared.
    in_footprint: bool,
    name: Option<String>,
    net: Option<String>,
    /// Each `filled_polygon` as its layer and points, sorted, so two fills
    /// compare equal when they hold the same polygons in any order, however
    /// the numbers are written and whatever `island` marks a newer KiCad adds.
    polygons: Vec<String>,
}

/// A node written one way: numbers to six decimals, KiCad's 1 nm, so `1`,
/// `1.0` and `1.000000` agree and no real difference is rounded away.
fn canonical(node: &konnect_sexp::SexpNode) -> String {
    match node.children() {
        Some(children) => format!(
            "({})",
            children.iter().map(canonical).collect::<Vec<_>>().join(" ")
        ),
        None => {
            let atom = node.as_str().unwrap_or_default();
            match atom.parse::<f64>() {
                // -0 and 0 are one coordinate.
                Ok(0.0) => "0.000000".into(),
                Ok(number) => format!("{number:.6}"),
                Err(_) => atom.to_owned(),
            }
        }
    }
}

fn zone_fill(zone: &konnect_sexp::SexpNode, in_footprint: bool) -> ZoneFill {
    let mut layers: Vec<&str> = ["layer", "layers"]
        .into_iter()
        .filter_map(|tag| zone.find(tag))
        .flat_map(|node| node.children().unwrap_or(&[]).iter().skip(1))
        .filter_map(konnect_sexp::SexpNode::as_str)
        .collect();
    layers.sort_unstable();
    layers.dedup();
    let mut polygons: Vec<String> = zone
        .find_all("filled_polygon")
        .into_iter()
        .map(|polygon| {
            // KiCad 5 wrote no layer on a single-layer zone's fill.
            let layer = polygon
                .find_str("layer")
                .or_else(|| (layers.len() == 1).then(|| layers[0]))
                .unwrap_or_default();
            let points = polygon.find("pts").map(canonical).unwrap_or_default();
            format!("{layer}|{points}")
        })
        .collect();
    polygons.sort();
    let net = zone.find_str("net_name").map(str::to_owned).or_else(|| {
        zone.find("net")
            .and_then(|net| net.get(1))
            .and_then(|name| name.as_str())
            .filter(|name| name.parse::<i64>().is_err())
            .map(str::to_owned)
    });
    let outline = zone
        .find("polygon")
        .and_then(|polygon| polygon.find("pts"))
        .map(canonical)
        .unwrap_or_default();
    ZoneFill {
        id: zone
            .find_str("uuid")
            .or_else(|| zone.find_str("tstamp"))
            .map(|id| id.trim().to_ascii_lowercase())
            .filter(|id| !id.is_empty()),
        shape: format!(
            "{}|{}|{outline}",
            net.as_deref().unwrap_or_default(),
            layers.join(",")
        ),
        in_footprint,
        name: zone
            .find_str("name")
            .map(str::to_owned)
            .filter(|n| !n.is_empty()),
        net,
        polygons,
    }
}

/// The board's copper zones, then those inside footprints (`module` on a
/// KiCad 5 board).
fn copper_zone_fills(source: &str) -> anyhow::Result<Vec<ZoneFill>> {
    let tree = konnect_sexp::parse_sexp(source)?;
    let copper = |zone: &&konnect_sexp::SexpNode| zone.find("keepout").is_none();
    let board = tree
        .find_all("zone")
        .into_iter()
        .filter(copper)
        .map(|zone| zone_fill(zone, false));
    let footprints = ["footprint", "module"]
        .into_iter()
        .flat_map(|tag| tree.find_all(tag))
        .flat_map(|footprint| footprint.find_all("zone"))
        .filter(copper)
        .map(|zone| zone_fill(zone, true));
    Ok(board.chain(footprints).collect())
}

fn zone_id(zone: &ZoneFill) -> Option<&str> {
    zone.id.as_deref()
}

fn zone_shape(zone: &ZoneFill) -> Option<&str> {
    Some(zone.shape.as_str())
}

/// Index of each key held by exactly one zone; `None` for a shared key.
fn unique_keys(
    zones: &[&ZoneFill],
    key: fn(&ZoneFill) -> Option<&str>,
) -> std::collections::HashMap<String, Option<usize>> {
    let mut seen: std::collections::HashMap<String, Option<usize>> = Default::default();
    for (index, zone) in zones.iter().enumerate() {
        if let Some(key) = key(zone) {
            seen.entry(key.to_owned())
                .and_modify(|slot| *slot = None)
                .or_insert(Some(index));
        }
    }
    seen
}

/// Pair each saved zone with its refilled self: by id where exactly one zone
/// on each side has it, then by shape the same way. A zone nothing pairs is
/// returned apart, never guessed at.
fn pair_zones<'s, 'r>(
    saved: &[&'s ZoneFill],
    refilled: &[&'r ZoneFill],
) -> (Vec<(&'s ZoneFill, &'r ZoneFill)>, Vec<&'s ZoneFill>) {
    let mut pairs = Vec::new();
    let mut taken = vec![false; refilled.len()];
    let mut left: Vec<&'s ZoneFill> = saved.to_vec();
    for key in [zone_id as fn(&ZoneFill) -> Option<&str>, zone_shape] {
        let free: Vec<usize> = (0..refilled.len()).filter(|&i| !taken[i]).collect();
        let free_zones: Vec<&ZoneFill> = free.iter().map(|&i| refilled[i]).collect();
        let theirs = unique_keys(&free_zones, key);
        let ours = unique_keys(&left, key);
        let mut unpaired = Vec::new();
        for zone in left {
            let found = key(zone).and_then(|k| match (ours.get(k), theirs.get(k)) {
                (Some(Some(_)), Some(Some(index))) => Some(free[*index]),
                _ => None,
            });
            match found {
                Some(index) => {
                    taken[index] = true;
                    pairs.push((zone, refilled[index]));
                }
                None => unpaired.push(zone),
            }
        }
        left = unpaired;
    }
    (pairs, left)
}

/// One violation as the fill comparison counts it: what and where, not the
/// ownership enrichment only the main run gets.
fn violation_key(violation: &cli::DrcViolation) -> String {
    let pos = violation
        .pos
        .as_ref()
        .map_or_else(String::new, |p| format!("{:.4},{:.4}", p.x, p.y));
    format!(
        "{}|{}|{}|{pos}",
        violation.severity, violation.rule, violation.description
    )
}

/// Rules a refill cannot change and the private copy cannot reproduce: it
/// carries no library tables, so a footprint from a project library reads as
/// missing there. They are left out of the comparison, and its result says so.
const RULES_NOT_COMPARED: [&str; 2] = ["lib_footprint_issues", "lib_footprint_mismatch"];

fn fill_sensitive(report: &cli::DrcReport) -> Vec<&cli::DrcViolation> {
    report
        .violations
        .iter()
        .chain(report.unconnected_items.iter().flatten())
        .filter(|violation| !RULES_NOT_COMPARED.contains(&violation.rule.as_str()))
        .collect()
}

/// The violations of `from` that `other` lacks, counted with multiplicity.
fn only_in<'a>(
    from: &[&'a cli::DrcViolation],
    other: &[&cli::DrcViolation],
) -> Vec<&'a cli::DrcViolation> {
    let mut remaining: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for violation in other {
        *remaining.entry(violation_key(violation)).or_default() += 1;
    }
    from.iter()
        .copied()
        .filter(
            |violation| match remaining.get_mut(&violation_key(violation)) {
                Some(count) if *count > 0 => {
                    *count -= 1;
                    false
                }
                _ => true,
            },
        )
        .collect()
}

fn violation_json(violation: &cli::DrcViolation) -> Value {
    json!({
        "severity": violation.severity,
        "rule": violation.rule,
        "description": violation.description,
        "pos": violation.pos.as_ref().map(|p| json!({ "x": p.x, "y": p.y })),
    })
}

/// Whether the zone fills `report` was computed from were current (#C24).
///
/// `state` is `fresh` when the zones were refilled for this run, when the
/// board has no copper zones, or when a refill of a private copy produced the
/// same fills for every zone; `stale` when that refill changed any zone's
/// fill; `unknown` when no comparison was made, it failed, or some zones could
/// not be compared, with `reason`. Zones are paired by id (`uuid`, or `tstamp`
/// on a KiCad 6/7 board), then by net, layers and outline; one nothing pairs
/// makes the state `unknown`, never `stale`. Zones inside footprints count but
/// cannot be compared, since kicad-cli's refill fills board zones only. The
/// violations a refill clears or adds are counted and the first few listed,
/// leaving out [`RULES_NOT_COMPARED`]. The comparison is a second,
/// analysis-only DRC on a scratch copy of the board and its project rules;
/// the board itself is never written.
async fn zone_fill_check(
    ctx: &ToolContext,
    board: &Path,
    refilled: bool,
    check: bool,
    report: &cli::DrcReport,
    defect_comparison: &mut Option<Result<zone_fill_defects::Comparison, &'static str>>,
) -> Value {
    let unknown = |method: &str, zones: Option<usize>, reason: String| {
        json!({
            "state": "unknown",
            "method": method,
            "copper_zones": zones,
            "reason": reason,
            "hint": "Pass refill_zones: true to run DRC on fresh fills, or leave check_zone_fills on.",
        })
    };
    let saved = match tokio::fs::read_to_string(board)
        .await
        .map_err(anyhow::Error::from)
        .and_then(|source| copper_zone_fills(&source))
    {
        Ok(zones) => zones,
        Err(error) => {
            return unknown(
                "check_failed",
                None,
                format!("could not read the board's zones: {error:#}"),
            )
        }
    };
    let in_footprints = saved.iter().filter(|zone| zone.in_footprint).count();
    let footprint_reason = format!(
        "{in_footprints} copper zone(s) inside footprints: kicad-cli's refill fills board \
         zones only, so their fills were not checked"
    );
    let footprint_hint = "Zones inside footprints are not refilled by kicad-cli, with or \
                          without refill_zones; check their fills in KiCad.";
    if refilled {
        if in_footprints > 0 {
            let mut result = unknown("refilled_for_this_run", Some(saved.len()), footprint_reason);
            result["footprint_zones"] = json!(in_footprints);
            result["hint"] = json!(footprint_hint);
            return result;
        }
        return json!({ "state": "fresh", "method": "refilled_for_this_run" });
    }
    if saved.is_empty() {
        return json!({ "state": "fresh", "method": "no_copper_zones", "copper_zones": 0 });
    }
    if !check {
        return unknown(
            "not_checked",
            Some(saved.len()),
            "check_zone_fills was false and the zones were not refilled".into(),
        );
    }
    let board_zones: Vec<&ZoneFill> = saved.iter().filter(|zone| !zone.in_footprint).collect();
    if board_zones.is_empty() {
        // A refill would change none of them, so no second DRC runs.
        let mut result = unknown("not_comparable", Some(saved.len()), footprint_reason);
        result["footprint_zones"] = json!(in_footprints);
        result["hint"] = json!(footprint_hint);
        return result;
    }
    let comparison = async {
        let scratch = tempfile::tempdir()?;
        let name = board.file_name().context("board path has no file name")?;
        let copy = scratch.path().join(name);
        tokio::fs::copy(board, &copy).await?;
        // The fill depends on the project's clearances, so the rules travel
        // with the copy. Neither file is required.
        for extension in ["kicad_pro", "kicad_dru"] {
            let rules = board.with_extension(extension);
            if rules.is_file() {
                tokio::fs::copy(&rules, copy.with_extension(extension)).await?;
            }
        }
        let before = zone_fill_defects::Snapshot::capture(&copy).await;
        let refilled_report = cli::refill_drc_on_private_copy(&ctx.config.kicad_cli, &copy).await?;
        let after = zone_fill_defects::Snapshot::capture(&copy).await;
        *defect_comparison = Some(Ok(zone_fill_defects::Comparison {
            before,
            after,
            report: refilled_report.clone(),
        }));
        let refilled = copper_zone_fills(&tokio::fs::read_to_string(&copy).await?)?;
        anyhow::Ok((refilled_report, refilled))
    };
    use anyhow::Context as _;
    let (refilled_report, refilled) = match comparison.await {
        Ok(result) => result,
        Err(error) => {
            // Evidence captured before a later failure is kept; otherwise the
            // diagnostic names the failed comparison (review A, F5).
            defect_comparison.get_or_insert(Err("comparison_failed"));
            return unknown(
                "check_failed",
                Some(saved.len()),
                format!("the refill comparison did not complete: {error:#}"),
            );
        }
    };
    let refilled_board: Vec<&ZoneFill> =
        refilled.iter().filter(|zone| !zone.in_footprint).collect();
    let (pairs, unpaired) = pair_zones(&board_zones, &refilled_board);
    let differing: Vec<&ZoneFill> = pairs
        .iter()
        .filter(|(saved, refilled)| saved.polygons != refilled.polygons)
        .map(|(saved, _)| *saved)
        .collect();
    let before = fill_sensitive(report);
    let after = fill_sensitive(&refilled_report);
    let cleared = only_in(&before, &after);
    let added = only_in(&after, &before);
    let state = if !differing.is_empty() {
        "stale"
    } else if unpaired.is_empty() && in_footprints == 0 {
        "fresh"
    } else {
        "unknown"
    };
    let mut result = json!({
        "state": state,
        "method": "compared_with_refill",
        "copper_zones": saved.len(),
        "zones_compared": pairs.len(),
        "zones_differing": differing.len(),
        "violations_cleared_by_refill": cleared.len(),
        "violations_added_by_refill": added.len(),
        "rules_not_compared": RULES_NOT_COMPARED,
    });
    let mut reasons = Vec::new();
    if !unpaired.is_empty() {
        result["zones_unpaired"] = json!(unpaired.len());
        reasons.push(format!(
            "{} zone(s) could not be paired with the refilled copy by id or by net, layers \
             and outline, so their fills were not compared",
            unpaired.len()
        ));
    }
    if in_footprints > 0 {
        result["footprint_zones"] = json!(in_footprints);
        reasons.push(footprint_reason);
    }
    if !reasons.is_empty() {
        result["reason"] = json!(reasons.join("; "));
    }
    if !differing.is_empty() {
        result["differing_zones"] = json!(differing
            .iter()
            .take(ZONE_FILL_ZONES_LISTED)
            .map(|zone| json!({ "id": zone.id, "name": zone.name, "net": zone.net }))
            .collect::<Vec<_>>());
        result["hint"] = json!(if cleared.is_empty() && added.is_empty() {
            "The saved zone fills are out of date, but a refill does not change these DRC results."
        } else {
            "The saved zone fills are out of date and a refill changes these DRC results. \
             Refill the zones (refill_zones: true, or the refill_zones tool) and run DRC again."
        });
    } else if state == "unknown" {
        result["hint"] = json!(if unpaired.is_empty() {
            footprint_hint
        } else {
            "Pass refill_zones: true to run DRC on fresh fills."
        });
    }
    if !cleared.is_empty() {
        result["cleared_by_refill"] = json!(cleared
            .iter()
            .take(ZONE_FILL_EXAMPLES)
            .map(|v| violation_json(v))
            .collect::<Vec<_>>());
    }
    if !added.is_empty() {
        result["added_by_refill"] = json!(added
            .iter()
            .take(ZONE_FILL_EXAMPLES)
            .map(|v| violation_json(v))
            .collect::<Vec<_>>());
    }
    result
}

/// Preserve completed work when optional report publication fails afterwards.
pub(crate) fn report_write_failure(
    evidence: &DrcSourceEvidence,
    board: &Path,
    output: &str,
    error: anyhow::Error,
) -> anyhow::Result<CallToolResult> {
    if !evidence.live_board_synced {
        return Err(error);
    }
    let body = json!({
        "error": ToolErrorKind::HandlerError { reason: format!("could not write DRC report to {output}: {error:#}") },
        "source_evidence": evidence,
        "board": board.display().to_string(),
        "message": "Board synchronization and CLI DRC completed, but optional report publication failed. Do not repeat refill/save blindly; correct the output destination and recover the report from the already-saved board.",
    });
    let mut result = CallToolResult::json(&body);
    result.is_error = true;
    Ok(result)
}

fn uncertain(board: &Path, operation: &str, reason: impl std::fmt::Display) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::MutationOutcomeUncertain {
            operation: operation.into(),
            path: board.display().to_string(),
            reason: reason.to_string(),
        },
        format!("{operation}: {reason}. The operation was not accepted as verified. Inspect the requested board in KiCad and its saved file before retrying; do not repeat possibly applied work blindly."),
    )
}

fn target_failure(failure: IpcFailure) -> CallToolResult {
    match failure {
        IpcFailure::Target { error, .. } => ipc_target_error_result(&error),
        other => CallToolResult::error_kind(
            ToolErrorKind::EditorUnavailable {
                editor: "pcb".into(),
                reason: other.to_string(),
            },
            format!("Cannot synchronize the requested board: {other}. CLI DRC was not started; open that board with IPC enabled and retry."),
        ),
    }
}

/// Complete a requested live refill without saving or switching to another board.
pub(crate) async fn refill(
    ctx: &ToolContext,
    board: &Path,
) -> anyhow::Result<Result<(), CallToolResult>> {
    let path = board.to_path_buf();
    let result = with_board_ipc_classified(ctx, board, move |client, _| {
        Ok(client
            .refill_zones()
            .and_then(|()| client.wait_for_board_ready(READY_TIMEOUT))
            .map_err(|error| uncertain(&path, "refill_zones", error)))
    })
    .await?;
    Ok(match result {
        Ok(result) => result,
        Err(error) => Err(target_failure(error)),
    })
}

pub(crate) async fn run(
    ctx: &ToolContext,
    board: &Path,
    args: &Value,
) -> anyhow::Result<Result<(cli::DrcReport, DrcSourceEvidence), CallToolResult>> {
    let option = |field: &str| -> Result<bool, CallToolResult> {
        match args.get(field) {
            None => Ok(false),
            Some(value) => value.as_bool().ok_or_else(|| {
                CallToolResult::error_kind(
                    ToolErrorKind::InvalidArgument {
                        field: field.into(),
                        reason: "must be a boolean".into(),
                    },
                    format!("{field} must be a boolean; no work was applied."),
                )
            }),
        }
    };
    let (sync, refill) = match (option("sync_live_board"), option("refill_zones")) {
        (Ok(sync), Ok(refill)) => (sync, refill),
        (Err(error), _) | (_, Err(error)) => return Ok(Err(error)),
    };
    // On unless the caller turns it off: a DRC on stale fills misleads (#C24).
    let check_fills = match args.get("check_zone_fills") {
        None => true,
        Some(_) => match option("check_zone_fills") {
            Ok(check) => check,
            Err(error) => return Ok(Err(error)),
        },
    };
    // Refuse obvious publication errors before any live save/refill. Later
    // permission changes or filesystem races still need applied-state recovery.
    if let Some(output) = args["output"].as_str() {
        let output_path = Path::new(output);
        let same_board = match (output_path.canonicalize(), board.canonicalize()) {
            (Ok(output), Ok(board)) => output == board,
            _ => output_path == board,
        };
        let invalid = same_board
            || output_path.is_dir()
            || output_path
                .ancestors()
                .skip(1)
                .any(|parent| parent.is_file());
        if invalid {
            return Ok(Err(CallToolResult::error_kind(
                ToolErrorKind::InvalidArgument {
                    field: "output".into(),
                        reason: "destination aliases the source board, is a directory or has a non-directory ancestor".into(),
                },
                "Choose a writable report file path. No refill, save or CLI DRC was started.",
            )));
        }
    }
    let mut saved_snapshot = None;
    if sync {
        let path = board.to_path_buf();
        let synchronized = with_board_ipc_classified(ctx, board, move |client, _| {
            let operation = || -> anyhow::Result<String> {
                if refill {
                    client.refill_zones()?;
                    client.wait_for_board_ready(READY_TIMEOUT)?;
                }
                client.save_board()?;
                client.wait_for_board_ready(READY_TIMEOUT)?;
                let live = client.save_document_to_string()?;
                let saved = std::fs::read_to_string(&path)?;
                let mut live_tree = konnect_sexp::parse_sexp(&live)?;
                let mut saved_tree = konnect_sexp::parse_sexp(&saved)?;
                normalize_snapshot_metadata(&mut live_tree);
                normalize_snapshot_metadata(&mut saved_tree);
                if live_tree != saved_tree {
                    anyhow::bail!("saved PCB does not match the observed live document");
                }
                Ok(saved)
            };
            Ok(operation().map_err(|error| uncertain(&path, "synchronize_drc_source", error)))
        })
        .await?;
        match synchronized {
            Ok(Ok(snapshot)) => saved_snapshot = Some(snapshot),
            Ok(Err(error)) => return Ok(Err(error)),
            Err(error) => return Ok(Err(target_failure(error))),
        }
    }
    // CLI refill is analysis-only. In synchronized mode the persisted IPC fill
    // is authoritative, so do not refill a second time in a different process.
    let defect_source = zone_fill_defects::Snapshot::capture(board).await;
    let report = match cli::run_drc(&ctx.config.kicad_cli, board, refill && !sync).await {
        Ok(report) => report,
        Err(error) if sync => return Ok(Err(uncertain(board, "drc_after_save", error))),
        Err(error) => return Err(error),
    };
    if let Some(snapshot) = saved_snapshot {
        match tokio::fs::read_to_string(board).await {
            Ok(current) if current == snapshot => {}
            Ok(_) => {
                return Ok(Err(uncertain(
                    board,
                    "verify_drc_source",
                    "saved PCB changed during CLI DRC",
                )))
            }
            Err(error) => return Ok(Err(uncertain(board, "verify_drc_source", error))),
        }
    }
    let mut defect_comparison = None;
    let mut zone_fill = zone_fill_check(
        ctx,
        board,
        refill,
        check_fills,
        &report,
        &mut defect_comparison,
    )
    .await;
    zone_fill["suspected_fill_defects"] = zone_fill_defects::finish(
        defect_source,
        board,
        &report,
        if refill {
            if sync {
                "saved_ipc_refill"
            } else {
                "kicad_cli_refill"
            }
        } else {
            "saved_fills"
        },
        defect_comparison,
    )
    .await;
    Ok(Ok((
        report,
        DrcSourceEvidence {
            source: "saved_file",
            live_board_synced: sync,
            zones_refilled: refill,
            zone_refill_source: if refill {
                Some(if sync { "ipc" } else { "kicad_cli" })
            } else {
                None
            },
            zone_fill,
        },
    )))
}

// IPC serializes embedded footprints with standalone library format metadata;
// SaveDocument omits that metadata in a board. Never discard geometry, models,
// connectivity, UUIDs, or arbitrary unknown fields to make a comparison pass.
fn normalize_snapshot_metadata(node: &mut konnect_sexp::SexpNode) {
    use konnect_sexp::SexpNode;
    let footprint = node.head() == Some("footprint");
    if let SexpNode::List(children) = node {
        if footprint {
            children.retain(|child| {
                !matches!(
                    child.head(),
                    Some("version" | "generator" | "generator_version")
                )
            });
        }
        for child in children {
            normalize_snapshot_metadata(child);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mcp::handler::McpHandler, test_support::MockIpcServer, tools::ServerConfig};
    use konnect_ipc::gen::kiapi;
    use prost::Message;
    use std::sync::{Arc, Mutex};

    const OLD: &str = include_str!("../../tests/fixtures/drc_ownership_j1.kicad_pcb");
    const LIVE: &str = include_str!("../../../konnect-ipc/tests/fixtures/live_ipc.kicad_pcb");

    fn response(message: Option<prost_types::Any>, busy: bool) -> kiapi::common::ApiResponse {
        kiapi::common::ApiResponse {
            status: Some(kiapi::common::ApiResponseStatus {
                status: if busy {
                    kiapi::common::ApiStatusCode::AsBusy
                } else {
                    kiapi::common::ApiStatusCode::AsOk
                } as i32,
                error_message: String::new(),
            }),
            header: None,
            message,
        }
    }

    fn packed(message: impl Message, name: &str) -> Option<prost_types::Any> {
        Some(prost_types::Any {
            type_url: format!("type.googleapis.com/{name}"),
            value: message.encode_to_vec(),
        })
    }

    async fn call(
        tool: &str,
        board: &Path,
        address: &str,
        executable: &Path,
        extra: Value,
    ) -> Value {
        let handler = McpHandler::new(ServerConfig {
            kicad_cli: executable.display().to_string(),
            kicad_binary: String::new(),
            ipc_address: address.into(),
            project_dir: None,
            jlcpcb_db_path: None,
            auto_load_toolsets: true,
            eager_toolsets: false,
        })
        .await
        .unwrap();
        let mut args = json!({"board": board.display().to_string()});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let result = handler
            .handle_message(json!({"jsonrpc":"2.0", "id":408,
            "method":"tools/call", "params":{"name":tool,"arguments":args}}))
            .await
            .unwrap()
            .result
            .unwrap();
        let mut body: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        body["isError"] = result["isError"].clone();
        body
    }

    fn cli_fixture(dir: &Path, board: &Path) -> std::path::PathBuf {
        std::fs::write(
            board.with_extension("drc.json"),
            r#"{"violations":[],"unconnected_items":[],"schematic_parity":[]}"#,
        )
        .unwrap();
        cli::test_support::write_script(dir, "drc-probe",
            "#!/bin/sh\nfor last; do :; done\ncp \"$last\" \"$last.observed\"\nexit 0\n",
            "@echo off\r\n:loop\r\nset last=%1\r\nshift\r\nif not \"%1\"==\"\" goto loop\r\ncopy /y %last% %last%.observed >nul\r\nexit /b 0\r\n")
    }

    fn mock(board: &Path, commands: Arc<Mutex<Vec<String>>>, mode: &'static str) -> MockIpcServer {
        let path = board.to_path_buf();
        let busy = Arc::new(Mutex::new(false));
        MockIpcServer::spawn("drc-sync", move |request| {
            let command = request.message.unwrap();
            let name = command.type_url.rsplit('/').next().unwrap().to_string();
            commands.lock().unwrap().push(name.clone());
            if *busy.lock().unwrap() && name == "kiapi.common.commands.GetOpenDocuments" {
                if mode != "busy-forever" {
                    *busy.lock().unwrap() = false;
                }
                return response(None, true);
            }
            let document = |filename: &str| kiapi::common::types::DocumentSpecifier {
                r#type: kiapi::common::types::DocumentType::DoctypePcb as i32,
                identifier: Some(
                    kiapi::common::types::document_specifier::Identifier::BoardFilename(
                        filename.into(),
                    ),
                ),
                project: Some(kiapi::common::types::ProjectSpecifier {
                    name: "clock".into(),
                    path: path.parent().unwrap().display().to_string(),
                }),
            };
            match name.as_str() {
                "kiapi.common.commands.GetOpenDocuments" => response(
                    packed(
                        kiapi::common::commands::GetOpenDocumentsResponse {
                            documents: if mode == "wrong" {
                                vec![document("other.kicad_pcb")]
                            } else {
                                vec![
                                    document("other.kicad_pcb"),
                                    document(path.file_name().unwrap().to_str().unwrap()),
                                ]
                            },
                        },
                        "kiapi.common.commands.GetOpenDocumentsResponse",
                    ),
                    false,
                ),
                "kiapi.board.commands.RefillZones" => {
                    let cmd = kiapi::board::commands::RefillZones::decode(command.value.as_slice())
                        .unwrap();
                    assert_eq!(
                        cmd.board.unwrap(),
                        document(path.file_name().unwrap().to_str().unwrap())
                    );
                    if mode == "refill-fail" {
                        let mut rejected = response(None, false);
                        rejected.status.as_mut().unwrap().status =
                            kiapi::common::ApiStatusCode::AsBadRequest as i32;
                        return rejected;
                    }
                    *busy.lock().unwrap() = true;
                    response(None, false)
                }
                "kiapi.common.commands.SaveDocument" => {
                    let cmd =
                        kiapi::common::commands::SaveDocument::decode(command.value.as_slice())
                            .unwrap();
                    assert_eq!(
                        cmd.document.unwrap(),
                        document(path.file_name().unwrap().to_str().unwrap())
                    );
                    if mode == "save-fail" {
                        let mut rejected = response(None, false);
                        rejected.status.as_mut().unwrap().status =
                            kiapi::common::ApiStatusCode::AsBadRequest as i32;
                        rejected
                    } else {
                        if mode != "stale" {
                            std::fs::write(&path, LIVE).unwrap();
                        }
                        if mode == "report-fail" {
                            let report = path.with_extension("report");
                            if !report.exists() {
                                std::fs::create_dir(report).unwrap();
                            }
                        }
                        response(None, false)
                    }
                }
                "kiapi.common.commands.SaveDocumentToString" => response(
                    packed(
                        kiapi::common::commands::SavedDocumentResponse {
                            contents: LIVE.into(),
                            document: Some(document(path.file_name().unwrap().to_str().unwrap())),
                        },
                        "kiapi.common.commands.SavedDocumentResponse",
                    ),
                    false,
                ),
                _ => panic!("unexpected IPC command: {name}"),
            }
        })
    }

    #[tokio::test]
    async fn served_drc_synchronizes_exact_board_and_waits_before_save() {
        for tool in ["run_drc", "get_drc_violations"] {
            let dir = tempfile::tempdir().unwrap();
            let board = dir.path().join("clock.kicad_pcb");
            std::fs::write(&board, OLD).unwrap();
            let executable = cli_fixture(dir.path(), &board);
            let commands = Arc::new(Mutex::new(Vec::new()));
            let server = mock(&board, commands.clone(), "ok");
            let result = call(
                tool,
                &board,
                server.address(),
                &executable,
                json!({"sync_live_board":true,"refill_zones":true}),
            )
            .await;
            assert_eq!(result["isError"], false, "{result}");
            assert_eq!(result["source"], "saved_file");
            assert_eq!(result["live_board_synced"], true);
            assert_eq!(result["zone_refill_source"], "ipc");
            // Review A F7: the IPC-saved fill keeps its own provenance label.
            assert_eq!(
                result["zone_fill_check"]["suspected_fill_defects"]["primary_report"]["provenance"],
                "saved_ipc_refill",
                "{result}"
            );
            assert_eq!(
                std::fs::read_to_string(board.with_extension("kicad_pcb.observed")).unwrap(),
                LIVE
            );
            let seen = commands.lock().unwrap();
            let fill = seen
                .iter()
                .position(|s| s.ends_with("RefillZones"))
                .unwrap();
            let save = seen
                .iter()
                .position(|s| s.ends_with("SaveDocument"))
                .unwrap();
            assert!(
                save > fill + 2,
                "refill must be followed by busy/readiness observations: {seen:?}"
            );
            assert_eq!(
                seen.iter().filter(|s| s.ends_with("RefillZones")).count(),
                1
            );
            assert_eq!(
                seen.iter().filter(|s| s.ends_with("SaveDocument")).count(),
                1
            );
        }
    }

    #[tokio::test]
    async fn served_drc_refuses_wrong_target_save_failure_and_unproven_snapshot_before_cli() {
        for tool in ["run_drc", "get_drc_violations"] {
            for mode in ["wrong", "refill-fail", "save-fail", "stale"] {
                let dir = tempfile::tempdir().unwrap();
                let board = dir.path().join("clock.kicad_pcb");
                std::fs::write(&board, OLD).unwrap();
                let executable = cli_fixture(dir.path(), &board);
                let commands = Arc::new(Mutex::new(Vec::new()));
                let server = mock(&board, commands.clone(), mode);
                let result = call(
                    tool,
                    &board,
                    server.address(),
                    &executable,
                    json!({"sync_live_board":true,"refill_zones": mode == "refill-fail"}),
                )
                .await;
                assert_eq!(result["isError"], true, "{mode}: {result}");
                assert!(!board.with_extension("kicad_pcb.observed").exists());
                assert_eq!(std::fs::read_to_string(&board).unwrap(), OLD);
                if mode == "wrong" {
                    assert!(!commands
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|s| s.ends_with("SaveDocument")));
                }
            }
        }
    }

    #[tokio::test]
    async fn served_file_only_drc_does_not_require_ipc_or_save() {
        for tool in ["run_drc", "get_drc_violations"] {
            let dir = tempfile::tempdir().unwrap();
            let board = dir.path().join("clock.kicad_pcb");
            std::fs::write(&board, OLD).unwrap();
            let executable = cli_fixture(dir.path(), &board);
            let result = call(tool, &board, "", &executable, json!({})).await;
            assert_eq!(result["isError"], false, "{result}");
            assert_eq!(result["live_board_synced"], false);
            assert_eq!(result["zones_refilled"], false);
            assert_eq!(std::fs::read_to_string(&board).unwrap(), OLD);
        }
    }

    #[tokio::test]
    async fn served_refill_waits_without_saving_and_refuses_wrong_board() {
        for mode in ["ok", "wrong"] {
            let dir = tempfile::tempdir().unwrap();
            let board = dir.path().join("clock.kicad_pcb");
            std::fs::write(&board, OLD).unwrap();
            let commands = Arc::new(Mutex::new(Vec::new()));
            let server = mock(&board, commands.clone(), mode);
            let result = call(
                "refill_zones",
                &board,
                server.address(),
                Path::new(""),
                json!({}),
            )
            .await;
            assert_eq!(result["isError"], mode == "wrong", "{result}");
            let seen = commands.lock().unwrap();
            assert!(!seen.iter().any(|s| s.ends_with("SaveDocument")));
            if mode == "ok" {
                assert_eq!(result["zones_refilled"], true);
                assert_eq!(result["saved"], false);
            }
            assert_eq!(std::fs::read_to_string(&board).unwrap(), OLD);
        }
    }

    #[test]
    fn readiness_deadline_does_not_replay_refill() {
        let dir = tempfile::tempdir().unwrap();
        let board = dir.path().join("clock.kicad_pcb");
        std::fs::write(&board, OLD).unwrap();
        let commands = Arc::new(Mutex::new(Vec::new()));
        let server = mock(&board, commands.clone(), "busy-forever");
        let client = konnect_ipc::KiCadIpcClient::new(server.address());
        client.find_open_board(&board).unwrap();
        client.refill_zones().unwrap();
        assert!(client.wait_for_board_ready(Duration::ZERO).is_err());
        assert_eq!(
            commands
                .lock()
                .unwrap()
                .iter()
                .filter(|s| s.ends_with("RefillZones"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn served_drc_rejects_malformed_options_and_missing_ipc_without_writes() {
        for tool in ["run_drc", "get_drc_violations"] {
            for extra in [
                json!({"sync_live_board":"true"}),
                json!({"refill_zones":1}),
                json!({"sync_live_board":true}),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let board = dir.path().join("clock.kicad_pcb");
                std::fs::write(&board, OLD).unwrap();
                let executable = cli_fixture(dir.path(), &board);
                let result = call(tool, &board, "", &executable, extra).await;
                assert_eq!(result["isError"], true, "{result}");
                assert_eq!(std::fs::read_to_string(&board).unwrap(), OLD);
                assert!(!board.with_extension("kicad_pcb.observed").exists());
            }
        }
    }

    #[test]
    fn snapshot_normalization_only_ignores_embedded_library_metadata() {
        let mut a = konnect_sexp::parse_sexp(OLD).unwrap();
        let mut b = a.clone();
        if let konnect_sexp::SexpNode::List(children) = &mut b {
            let footprint = children
                .iter_mut()
                .find(|node| node.head() == Some("footprint"))
                .unwrap();
            if let konnect_sexp::SexpNode::List(children) = footprint {
                children.push(konnect_sexp::parse_sexp("(version 20260206)").unwrap());
            }
        }
        normalize_snapshot_metadata(&mut a);
        normalize_snapshot_metadata(&mut b);
        assert_eq!(a, b);
        if let konnect_sexp::SexpNode::List(children) = &mut b {
            children
                .push(konnect_sexp::parse_sexp("(unknown_content must_not_be_ignored)").unwrap());
        }
        normalize_snapshot_metadata(&mut b);
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn served_drc_does_not_accept_changed_source_or_cli_failure_after_save() {
        for tool in ["run_drc", "get_drc_violations"] {
            for fail in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let board = dir.path().join("clock.kicad_pcb");
                std::fs::write(&board, OLD).unwrap();
                std::fs::write(board.with_extension("kicad_pcb.replacement"), OLD).unwrap();
                cli_fixture(dir.path(), &board);
                let executable = if fail {
                    cli::test_support::write_script(
                        dir.path(),
                        "failing-drc",
                        "#!/bin/sh\nexit 1\n",
                        "@exit /b 1\r\n",
                    )
                } else {
                    cli::test_support::write_script(dir.path(), "changing-drc",
                        "#!/bin/sh\nfor last; do :; done\ncp \"$last.replacement\" \"$last\"\nexit 0\n",
                        "@echo off\r\n:loop\r\nset last=%1\r\nshift\r\nif not \"%1\"==\"\" goto loop\r\ncopy /y %last%.replacement %last% >nul\r\nexit /b 0\r\n")
                };
                let commands = Arc::new(Mutex::new(Vec::new()));
                let server = mock(&board, commands.clone(), "ok");
                let result = call(
                    tool,
                    &board,
                    server.address(),
                    &executable,
                    json!({"sync_live_board":true}),
                )
                .await;
                assert_eq!(result["isError"], true, "{result}");
                assert_eq!(result["error"]["kind"], "mutation_outcome_uncertain");
                assert_eq!(
                    result["error"]["operation"],
                    if fail {
                        "drc_after_save"
                    } else {
                        "verify_drc_source"
                    }
                );
                assert_eq!(
                    commands
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|s| s.ends_with("SaveDocument"))
                        .count(),
                    1
                );
            }
        }
    }

    #[tokio::test]
    async fn served_report_failure_preserves_completed_sync_receipt_and_preflights_obvious_errors()
    {
        for tool in ["run_drc", "get_drc_violations"] {
            for preexisting in [true, false] {
                let dir = tempfile::tempdir().unwrap();
                let board = dir.path().join("clock.kicad_pcb");
                std::fs::write(&board, OLD).unwrap();
                let executable = cli_fixture(dir.path(), &board);
                let output = board.with_extension("report");
                if preexisting {
                    std::fs::create_dir(&output).unwrap();
                }
                let commands = Arc::new(Mutex::new(Vec::new()));
                let server = mock(&board, commands.clone(), "report-fail");
                let result = call(
                    tool,
                    &board,
                    server.address(),
                    &executable,
                    json!({"sync_live_board":true,"output":output.display().to_string()}),
                )
                .await;
                assert_eq!(result["isError"], true, "{result}");
                if preexisting {
                    assert_eq!(result["error"]["kind"], "invalid_argument");
                    assert!(commands.lock().unwrap().is_empty());
                    assert_eq!(std::fs::read_to_string(&board).unwrap(), OLD);
                } else {
                    assert_eq!(result["error"]["kind"], "handler_error");
                    assert_eq!(result["source_evidence"]["live_board_synced"], true);
                    assert_eq!(result["source_evidence"]["source"], "saved_file");
                    assert_eq!(std::fs::read_to_string(&board).unwrap(), LIVE);
                    assert_eq!(
                        commands
                            .lock()
                            .unwrap()
                            .iter()
                            .filter(|s| s.ends_with("SaveDocument"))
                            .count(),
                        1
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn report_output_cannot_replace_the_source_board() {
        for tool in ["run_drc", "get_drc_violations"] {
            let dir = tempfile::tempdir().unwrap();
            let board = dir.path().join("clock.kicad_pcb");
            std::fs::write(&board, OLD).unwrap();
            let executable = cli_fixture(dir.path(), &board);
            let commands = Arc::new(Mutex::new(Vec::new()));
            let server = mock(&board, commands.clone(), "ok");
            let result = call(
                tool,
                &board,
                server.address(),
                &executable,
                json!({"sync_live_board":true,"output":board.display().to_string()}),
            )
            .await;
            assert_eq!(result["isError"], true);
            assert_eq!(result["error"]["kind"], "invalid_argument");
            assert!(commands.lock().unwrap().is_empty());
            assert_eq!(std::fs::read_to_string(&board).unwrap(), OLD);
        }
    }

    #[tokio::test]
    async fn report_publication_does_not_truncate_a_hard_linked_board() {
        for tool in ["run_drc", "get_drc_violations"] {
            let dir = tempfile::tempdir().unwrap();
            let board = dir.path().join("clock.kicad_pcb");
            std::fs::write(&board, OLD).unwrap();
            let executable = cli_fixture(dir.path(), &board);
            let output = dir.path().join("report.json");
            std::fs::hard_link(&board, &output).unwrap();
            let result = call(
                tool,
                &board,
                "",
                &executable,
                json!({"output":output.display().to_string()}),
            )
            .await;
            assert_eq!(result["isError"], false, "{result}");
            assert_eq!(std::fs::read_to_string(&board).unwrap(), OLD);
            let report: Value =
                serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
            assert!(report["violations"].as_array().unwrap().is_empty());
        }
    }

    /// Requires a disposable, already-open KiCad 10 board, never a working design.
    #[tokio::test]
    #[ignore = "requires disposable live KiCad board and KICAD_API_SOCKET"]
    async fn live_drc_drops_deleted_via_uuid_after_refill_and_save() {
        let board = std::path::PathBuf::from(
            std::env::var("KONNECT_LIVE_KICAD_BOARD").expect("disposable board path"),
        );
        let address = std::env::var("KICAD_API_SOCKET").expect("live IPC endpoint");
        let executable = std::path::PathBuf::from(
            std::env::var("KONNECT_TEST_KICAD_CLI").expect("CLI executable"),
        );
        let client = konnect_ipc::KiCadIpcClient::new(&address);
        client.find_open_board(&board).unwrap();
        let net = client
            .get_nets()
            .unwrap()
            .into_iter()
            .find(|net| !net.name.is_empty())
            .expect("named net");
        client.add_via(&net.name, 100.0, 120.0, 0.4, 0.8).unwrap();
        client.save_board().unwrap();
        let saved = std::fs::read_to_string(&board).unwrap();
        let tree = konnect_sexp::parse_sexp(&saved).unwrap();
        let via = tree
            .find_all("via")
            .into_iter()
            .find(|node| {
                node.find("at")
                    .is_some_and(|at| at.get_f64(1) == Some(100.0) && at.get_f64(2) == Some(120.0))
            })
            .expect("created via");
        let uuid = via
            .find("uuid")
            .unwrap()
            .get(1)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        client.delete_items(vec![uuid.clone()]).unwrap();
        // Add an unfilled native copper zone as a second observable outcome:
        // a refill acknowledgement alone must not pass the live regression.
        let corners: Vec<_> = tree
            .find_all("gr_line")
            .into_iter()
            .filter(|node| {
                node.find("layer")
                    .and_then(|layer| layer.get(1))
                    .and_then(konnect_sexp::SexpNode::as_str)
                    == Some("Edge.Cuts")
            })
            .flat_map(|node| [node.find("start"), node.find("end")])
            .flatten()
            .map(|point| (point.get_f64(1).unwrap(), point.get_f64(2).unwrap()))
            .collect();
        assert!(
            !corners.is_empty(),
            "native fixture must have a line-based board outline"
        );
        let left = corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min) + 0.5;
        let right = corners
            .iter()
            .map(|p| p.0)
            .fold(f64::NEG_INFINITY, f64::max)
            - 0.5;
        let top = corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min) + 0.5;
        let bottom = corners
            .iter()
            .map(|p| p.1)
            .fold(f64::NEG_INFINITY, f64::max)
            - 0.5;
        let points = [(left, top), (right, top), (right, bottom), (left, bottom)];
        let zone = konnect_ipc::builders::build_zone(
            &konnect_ipc::builders::ZoneSpec {
                layer: "F.Cu",
                net_name: &net.name,
                points: &points,
                clearance_mm: 0.3,
                min_thickness_mm: 0.25,
                name: "issue408-live-refill",
                priority: 1,
                connection: kiapi::board::types::ZoneConnectionStyle::ZcsFull,
            },
            net.netcode,
        );
        assert!(!zone.filled && zone.filled_polygons.is_empty());
        let created = client
            .create_items_returning(vec![konnect_ipc::builders::pack_any(
                &zone,
                "kiapi.board.types.Zone",
            )])
            .unwrap();
        let zone_id = created
            .iter()
            .map(|item| kiapi::board::types::Zone::decode(item.value.as_slice()).unwrap())
            .next()
            .unwrap()
            .id
            .unwrap()
            .value;
        assert!(
            std::fs::read_to_string(&board).unwrap().contains(&uuid),
            "deletion must still be unsaved for this regression"
        );
        let stale = call(
            "run_drc",
            &board,
            &address,
            &executable,
            json!({"severity":"info","limit":10000}),
        )
        .await;
        assert_eq!(stale["isError"], false, "{stale}");
        assert!(
            stale.to_string().contains(&uuid),
            "file-only control must actually report the stale UUID: {stale}"
        );
        for tool in ["run_drc", "get_drc_violations"] {
            let result = call(
                tool,
                &board,
                &address,
                &executable,
                json!({"sync_live_board":true,"refill_zones":true,"severity":"info"}),
            )
            .await;
            assert_eq!(result["isError"], false, "{result}");
            assert_eq!(result["live_board_synced"], true);
            assert_eq!(result["zones_refilled"], true);
            assert!(
                !result.to_string().contains(&uuid),
                "deleted UUID survived synchronized DRC"
            );
            assert!(!std::fs::read_to_string(&board).unwrap().contains(&uuid));
            let zones = client
                .get_items(kiapi::common::types::KiCadObjectType::KotPcbZone)
                .unwrap();
            let observed = zones
                .iter()
                .map(|item| kiapi::board::types::Zone::decode(item.value.as_slice()).unwrap())
                .find(|zone| zone.id.as_ref().is_some_and(|id| id.value == zone_id))
                .expect("created zone");
            assert!(
                observed.filled && !observed.filled_polygons.is_empty(),
                "refill must actually compute copper"
            );
        }
    }
}

/// C24: `zone_fill_state` says whether the fills a DRC read were current, by
/// refilling a private copy, and never writes the board.
#[cfg(all(test, unix))]
mod zone_fill_tests {
    use super::*;
    use crate::tools::{ServerConfig, ToolContext};
    use serde_json::json;

    fn board(fill: &str) -> String {
        format!(
            r#"(kicad_pcb (version 20260206) (generator "test")
  (net 0 "")
  (net 1 "GND")
  (zone (net 1) (net_name "GND") (layer "F.Cu") (uuid "z-pour") (name "pour")
    (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10)))
    (filled_polygon (layer "F.Cu") (pts {fill})))
  (zone (net 0) (net_name "") (layer "F.Cu") (uuid "z-keepout")
    (keepout (tracks not_allowed) (vias allowed) (pads allowed) (copperpour allowed) (footprints allowed))
    (polygon (pts (xy 20 20) (xy 30 20) (xy 30 30))))
)
"#
        )
    }
    const FILL: &str = "(xy 0.5 0.5) (xy 9.5 0.5) (xy 9.5 9.5) (xy 0.5 9.5)";
    const REFILL: &str = "(xy 0.5 0.5) (xy 9.5 0.5) (xy 9.5 6) (xy 0.5 6)";
    const STALE_REPORT: &str = r#"{"violations":[{"type":"clearance","severity":"error","description":"Clearance violation (zone pour and track)","items":[{"description":"Zone pour","pos":{"x":9.5,"y":9.5},"uuid":"z-pour"}]},{"type":"silk_overlap","severity":"warning","description":"Silkscreen overlap","items":[{"description":"Text","pos":{"x":1,"y":1}}]}],"unconnected_items":[],"schematic_parity":[]}"#;
    const REFILLED_REPORT: &str = r#"{"violations":[{"type":"silk_overlap","severity":"warning","description":"Silkscreen overlap","items":[{"description":"Text","pos":{"x":1,"y":1}}]}],"unconnected_items":[]}"#;

    struct Scene {
        dir: tempfile::TempDir,
        board: std::path::PathBuf,
        ctx: ToolContext,
    }

    impl Scene {
        /// A stand-in kicad-cli: the main DRC writes `main.json`; a run with
        /// `--save-board` writes `refill.json` and saves `refilled.kicad_pcb`
        /// over the board it was given. Every call is logged.
        fn new(saved: &str, refilled: Option<&str>) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let board = dir.path().join("demo.kicad_pcb");
            std::fs::write(&board, saved).unwrap();
            std::fs::write(board.with_extension("kicad_pro"), "{}").unwrap();
            std::fs::write(dir.path().join("main.json"), STALE_REPORT).unwrap();
            std::fs::write(dir.path().join("refill.json"), REFILLED_REPORT).unwrap();
            let refill_step = match refilled {
                Some(text) => {
                    std::fs::write(dir.path().join("refilled.kicad_pcb"), text).unwrap();
                    format!(
                        "cp '{0}/refilled.kicad_pcb' \"$last\"; cp '{0}/refill.json' \"$out\"",
                        dir.path().display()
                    )
                }
                None => "echo refill failed >&2; exit 3".into(),
            };
            let script = format!(
                "#!/bin/sh\nout=''; save=0; prev=''\nfor a; do\n  [ \"$prev\" = --output ] && out=\"$a\"\n  [ \"$a\" = --save-board ] && save=1\n  prev=\"$a\"; last=\"$a\"\ndone\necho \"$*\" >> '{0}/calls'\nif [ $save = 1 ]; then\n  [ -f \"$(dirname \"$last\")/demo.kicad_pro\" ] || exit 4\n  {refill_step}\nelse\n  cp '{0}/main.json' \"$out\"\nfi\nexit 0\n",
                dir.path().display()
            );
            let cli = cli::test_support::write_script(dir.path(), "kicad-cli", &script, "");
            let ctx = ToolContext::new(
                ServerConfig {
                    kicad_cli: cli.display().to_string(),
                    ..Default::default()
                },
                std::sync::Arc::new(crate::router::ToolRouter::new()),
            );
            Scene { dir, board, ctx }
        }

        /// What the `--save-board` run reports, in place of [`REFILLED_REPORT`].
        fn with_refill_report(self, report: &str) -> Self {
            std::fs::write(self.dir.path().join("refill.json"), report).unwrap();
            self
        }

        fn calls(&self) -> Vec<String> {
            std::fs::read_to_string(self.dir.path().join("calls"))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        }

        async fn run(&self, tool: &str, extra: Value) -> (bool, Value) {
            let mut args = json!({ "board": self.board.display().to_string(), "severity": "info" });
            args.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let result = match tool {
                "run_drc" => super::super::verification::handle_run_drc(&args, &self.ctx).await,
                _ => super::super::pcb_export::handle_get_drc_violations(&args, &self.ctx).await,
            }
            .unwrap();
            let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
                panic!("expected text");
            };
            (result.is_error, serde_json::from_str(text).unwrap())
        }
    }

    fn defect_board(fill: &str) -> String {
        board(fill)
            .replacen(
                "(net 0",
                "(layers (0 \"F.Cu\" signal) (2 \"B.Cu\" signal)) (net 0",
                1,
            )
            .replacen(
                "(net 1 \"GND\")",
                r#"(net 1 "GND") (net 2 "SIGNAL")
  (footprint "Test" (layer "F.Cu") (uuid "fp-test") (at 5 5)
    (pad "1" thru_hole circle (at 0 0) (size 2 2) (drill 1)
      (layers "*.Cu" "*.Mask") (net 2 "SIGNAL") (uuid "pad-test")))"#,
                1,
            )
    }

    fn defect_report() -> Value {
        json!({
            "violations": [{
                "type": "clearance", "severity": "error",
                "description": "Clearance violation (netclass 'Default' clearance 0.2000 mm; actual 0.0000 mm)",
                "items": [
                    {"uuid": "z-pour", "description": "untrusted description", "pos": {"x": 5, "y": 5}},
                    {"uuid": "pad-test", "description": "also untrusted", "pos": {"x": 5, "y": 5}}
                ]
            }],
            "unconnected_items": [], "schematic_parity": []
        })
    }

    #[tokio::test]
    async fn suspected_fill_defects_preserve_the_primary_drc_and_do_not_add_a_run() {
        // Removing the detector or trusting the report's prose instead of the
        // board's net identities must break this observable result.
        let saved = defect_board(FILL);
        let report = defect_report();
        for tool in ["run_drc", "get_drc_violations"] {
            let scene = Scene::new(&saved, Some(&board(REFILL)));
            std::fs::write(scene.dir.path().join("main.json"), report.to_string()).unwrap();
            let (error, body) = scene.run(tool, json!({"check_zone_fills": false})).await;
            assert!(!error, "{body}");
            let diagnostic = &body["zone_fill_check"]["suspected_fill_defects"];
            assert_eq!(diagnostic["status"], "suspected", "{tool}: {body}");
            assert_eq!(diagnostic["primary_report"]["matching_entries"], 1);
            assert_eq!(diagnostic["primary_report"]["distinct_pairs"], 1);
            assert!(diagnostic["newly_reported_after_refill"].is_null());
            assert_eq!(body["violations"].as_array().unwrap().len(), 1);
            assert_eq!(body["violations"][0]["severity"], "error");
            assert_eq!(scene.calls().len(), 1);
            assert_eq!(std::fs::read_to_string(&scene.board).unwrap(), saved);
        }
    }

    #[tokio::test]
    async fn suspected_fill_defects_attribute_only_a_new_pair_after_private_refill() {
        let saved = defect_board(FILL);
        let after = defect_board(REFILL);
        for tool in ["run_drc", "get_drc_violations"] {
            let scene =
                Scene::new(&saved, Some(&after)).with_refill_report(&defect_report().to_string());
            std::fs::write(
                scene.dir.path().join("main.json"),
                r#"{"violations":[],"unconnected_items":[],"schematic_parity":[]}"#,
            )
            .unwrap();
            let (error, body) = scene.run(tool, json!({})).await;
            assert!(!error, "{body}");
            let d = &body["zone_fill_check"]["suspected_fill_defects"];
            assert_eq!(d["status"], "suspected", "{body}");
            assert_eq!(d["primary_report"]["matching_entries"], 0);
            assert_eq!(d["comparison_report"]["matching_entries"], 1);
            assert_eq!(d["newly_reported_after_refill"], 1);
            assert_eq!(body["violations"].as_array().unwrap().len(), 0);
            assert_eq!(scene.calls().len(), 2);
            assert_eq!(std::fs::read_to_string(&scene.board).unwrap(), saved);
        }
    }

    #[tokio::test]
    async fn suspected_fill_defects_keep_provenance_when_refill_is_requested_or_fails() {
        let saved = defect_board(FILL);
        for refill in [false, true] {
            let scene = Scene::new(&saved, None);
            std::fs::write(
                scene.dir.path().join("main.json"),
                defect_report().to_string(),
            )
            .unwrap();
            let (_, body) = scene.run("run_drc", json!({"refill_zones": refill})).await;
            let d = &body["zone_fill_check"]["suspected_fill_defects"];
            assert_eq!(d["status"], "suspected");
            assert_eq!(d["primary_report"]["matching_entries"], 1);
            assert_eq!(
                d["primary_report"]["provenance"],
                if refill {
                    "kicad_cli_refill"
                } else {
                    "saved_fills"
                }
            );
            assert!(d["comparison_report"]["matching_entries"].is_null());
            assert!(d["newly_reported_after_refill"].is_null());
            // A requested refill runs no comparison; a failed one is named.
            let reason = if refill {
                "not_compared"
            } else {
                "comparison_failed"
            };
            assert_eq!(d["comparison_report"]["reason"], reason, "{body}");
            assert_eq!(d["attribution_reason"], reason);
            assert_eq!(scene.calls().len(), if refill { 1 } else { 2 });
            assert_eq!(body["errors"], 1);
        }
    }

    #[tokio::test]
    async fn suspected_fill_defects_do_not_read_a_failed_private_refill_as_not_compared() {
        // Review A F5: the --save-board run exits 3 in this scene.
        let saved = defect_board(FILL);
        let empty = r#"{"violations":[],"unconnected_items":[],"schematic_parity":[]}"#;
        for tool in ["run_drc", "get_drc_violations"] {
            let scene = Scene::new(&saved, None);
            std::fs::write(scene.dir.path().join("main.json"), empty).unwrap();
            let (error, body) = scene.run(tool, json!({})).await;
            assert!(!error, "{body}");
            assert_eq!(body["zone_fill_check"]["method"], "check_failed");
            let d = &body["zone_fill_check"]["suspected_fill_defects"];
            assert_eq!(d["status"], "incomplete", "{tool}: {body}");
            assert_eq!(d["comparison_report"]["reason"], "comparison_failed");
            assert_eq!(d["comparison_report"]["coverage"], "unavailable");
            assert_eq!(d["attribution_reason"], "comparison_failed");
            assert_eq!(scene.calls().len(), 2);
            // A deliberate skip still reads as not compared, with nothing found.
            let scene = Scene::new(&saved, None);
            std::fs::write(scene.dir.path().join("main.json"), empty).unwrap();
            let (_, body) = scene.run(tool, json!({"check_zone_fills": false})).await;
            let d = &body["zone_fill_check"]["suspected_fill_defects"];
            assert_eq!(d["status"], "none_detected", "{body}");
            assert_eq!(d["comparison_report"]["reason"], "not_compared");
            assert_eq!(scene.calls().len(), 1);
        }
    }

    #[tokio::test]
    async fn suspected_fill_defects_refuse_attribution_beside_a_footprint_zone() {
        // Review A F7: the served path, not only the pure comparison.
        let with_footprint_zone = |fill: &str| {
            defect_board(fill).replacen(
                "(pad \"1\"",
                "(zone (net 2) (net_name \"SIGNAL\") (layer \"F.Cu\") (uuid \"fp-zone\")\n      (polygon (pts (xy 4 4) (xy 6 4) (xy 6 6))))\n    (pad \"1\"",
                1,
            )
        };
        let saved = with_footprint_zone(FILL);
        let scene = Scene::new(&saved, Some(&with_footprint_zone(REFILL)))
            .with_refill_report(&defect_report().to_string());
        std::fs::write(
            scene.dir.path().join("main.json"),
            r#"{"violations":[],"unconnected_items":[],"schematic_parity":[]}"#,
        )
        .unwrap();
        let (error, body) = scene.run("run_drc", json!({})).await;
        assert!(!error, "{body}");
        let d = &body["zone_fill_check"]["suspected_fill_defects"];
        assert_eq!(d["status"], "suspected", "{body}");
        assert_eq!(d["comparison_report"]["matching_entries"], 1);
        assert!(d["newly_reported_after_refill"].is_null());
        assert_eq!(
            d["attribution_reason"],
            "footprint_zone_comparison_unsupported"
        );
        assert_eq!(scene.calls().len(), 2);
        assert_eq!(std::fs::read_to_string(&scene.board).unwrap(), saved);
    }

    #[tokio::test]
    async fn suspected_fill_defects_bind_before_the_primary_cli_and_reach_baselines() {
        let saved = defect_board(FILL);
        let scene = Scene::new(&saved, None);
        std::fs::write(
            scene.dir.path().join("main.json"),
            defect_report().to_string(),
        )
        .unwrap();
        let baseline = super::super::check_baselines::handle_save_check_baseline(
            &json!({"board": scene.board.display().to_string(), "name": "defect", "check_zone_fills": false}),
            &scene.ctx,
        ).await.unwrap();
        let crate::mcp::protocol::ToolContent::Text { text } = &baseline.content[0] else {
            panic!("text")
        };
        assert!(!baseline.is_error, "{text}");
        let baseline: Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            baseline["source_evidence"]["zone_fill"]["suspected_fill_defects"]["primary_report"]
                ["matching_entries"],
            1
        );
        let script = std::fs::read_to_string(&scene.ctx.config.kicad_cli).unwrap();
        std::fs::write(
            &scene.ctx.config.kicad_cli,
            script.replace(
                "exit 0",
                &format!(
                    "printf ' ' >> '{}'\nexit 0",
                    scene.board.with_extension("kicad_pro").display()
                ),
            ),
        )
        .unwrap();
        let (_, body) = scene
            .run("run_drc", json!({"check_zone_fills": false}))
            .await;
        let d = &body["zone_fill_check"]["suspected_fill_defects"];
        assert_eq!(d["status"], "incomplete");
        assert_eq!(
            d["primary_report"]["reason"],
            "source_changed_or_unavailable"
        );
        assert_eq!(body["errors"], 1);
    }

    #[tokio::test]
    async fn suspected_fill_defects_do_not_drop_a_report_with_an_unreadable_refilled_board() {
        let saved = defect_board(FILL);
        for tool in ["run_drc", "get_drc_violations"] {
            for primary_has_candidate in [false, true] {
                let scene = Scene::new(&saved, Some("(kicad_pcb broken"))
                    .with_refill_report(&defect_report().to_string());
                let primary = if primary_has_candidate {
                    defect_report()
                } else {
                    json!({"violations":[],"unconnected_items":[],"schematic_parity":[]})
                };
                std::fs::write(scene.dir.path().join("main.json"), primary.to_string()).unwrap();
                let (error, body) = scene.run(tool, json!({})).await;
                assert!(!error, "{body}");
                let d = &body["zone_fill_check"]["suspected_fill_defects"];
                assert_eq!(
                    d["status"],
                    if primary_has_candidate {
                        "suspected"
                    } else {
                        "incomplete"
                    }
                );
                assert_eq!(d["comparison_report"]["coverage"], "unavailable");
                assert_eq!(d["comparison_report"]["reason"], "board_did_not_parse");
                assert!(d["comparison_report"]["matching_entries"].is_null());
                assert!(d["newly_reported_after_refill"].is_null());
                assert_eq!(
                    body["violations"].as_array().unwrap().len(),
                    usize::from(primary_has_candidate)
                );
                assert_eq!(scene.calls().len(), 2);
                assert_eq!(std::fs::read_to_string(&scene.board).unwrap(), saved);
            }
        }
    }

    #[tokio::test]
    async fn stale_fills_are_reported_with_the_violations_a_refill_clears() {
        for tool in ["run_drc", "get_drc_violations"] {
            let saved = board(FILL);
            let scene = Scene::new(&saved, Some(&board(REFILL)));
            let (error, body) = scene.run(tool, json!({})).await;
            assert!(!error, "{body}");
            assert_eq!(body["zone_fill_state"], "stale", "{tool}: {body}");
            let check = &body["zone_fill_check"];
            assert_eq!(check["method"], "compared_with_refill");
            assert_eq!(check["copper_zones"], 1, "the keepout is not a copper zone");
            assert_eq!(check["zones_differing"], 1);
            assert_eq!(check["differing_zones"][0]["id"], "z-pour");
            assert_eq!(check["zones_compared"], 1);
            assert!(check.get("reason").is_none(), "{check}");
            assert_eq!(check["differing_zones"][0]["net"], "GND");
            assert_eq!(check["violations_cleared_by_refill"], 1);
            assert_eq!(check["violations_added_by_refill"], 0);
            assert_eq!(check["cleared_by_refill"][0]["rule"], "clearance");
            assert!(check["hint"].as_str().unwrap().contains("refill"));
            // The main result is still the saved fills' result, whole.
            assert_eq!(body["design_rule_violations"], 2);
            assert_eq!(std::fs::read_to_string(&scene.board).unwrap(), saved);
            let calls = scene.calls();
            assert_eq!(calls.len(), 2, "{calls:?}");
            assert!(!calls[0].contains("--save-board"));
            assert!(calls[1].contains("--refill-zones --save-board"));
            let board_arg = calls[1].split(' ').next_back().unwrap();
            assert_ne!(
                std::path::Path::new(board_arg),
                scene.board.as_path(),
                "the refill must run on a copy"
            );
        }
    }

    #[tokio::test]
    async fn fills_a_refill_reproduces_are_fresh() {
        let saved = board(FILL);
        let scene = Scene::new(&saved, Some(&saved));
        let (_, body) = scene.run("run_drc", json!({})).await;
        assert_eq!(body["zone_fill_state"], "fresh", "{body}");
        assert_eq!(body["zone_fill_check"]["zones_differing"], 0);
        assert!(body["zone_fill_check"].get("differing_zones").is_none());
    }

    /// A KiCad 7 board (`version 20221018`): the zone's id is a `tstamp`, its
    /// net a number beside `net_name`. kicad-cli 10 saves the refilled copy
    /// with `uuid`, `(net "GND")`, an `island` mark and other number spellings.
    fn kicad7_board(id: &str, fill: &str) -> String {
        format!(
            r#"(kicad_pcb (version 20221018) (generator pcbnew)
  (net 0 "")
  (net 1 "GND")
  (zone (net 1) (net_name "GND") (layer "F.Cu") {id} (name "pour")
    (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10)))
    (filled_polygon (layer "F.Cu") (pts {fill})))
)
"#
        )
    }

    fn kicad10_board(id: &str, fill: &str) -> String {
        format!(
            r#"(kicad_pcb (version 20260206) (generator "pcbnew")
  (net 0 "")
  (net 1 "GND")
  (zone (net "GND") (layers "F.Cu") {id} (name "pour")
    (polygon (pts (xy 0.0 0.0) (xy 10.000 0) (xy 10 10) (xy 0 10)))
    (filled_polygon (layer "F.Cu") (island no) (pts {fill})))
)
"#
        )
    }

    const FILL_RESPELT: &str = "(xy 0.50 0.5) (xy 9.5 0.500) (xy 9.5 9.5) (xy 0.5 9.50)";

    /// Review #49: matched by `uuid` alone, a `tstamp` zone missed its
    /// refilled self and every KiCad 6/7 board read stale.
    #[tokio::test]
    async fn tstamp_zones_with_identical_fills_are_fresh() {
        let saved = kicad7_board("(tstamp Z-POUR)", FILL);
        let scene = Scene::new(
            &saved,
            Some(&kicad10_board("(uuid \"z-pour\")", FILL_RESPELT)),
        );
        let (_, body) = scene.run("run_drc", json!({})).await;
        assert_eq!(body["zone_fill_state"], "fresh", "{body}");
        let check = &body["zone_fill_check"];
        assert_eq!(check["zones_compared"], 1);
        assert_eq!(check["zones_differing"], 0);
        assert!(check.get("zones_unpaired").is_none(), "{check}");

        // The id pairs the zone even when its outline is written from another
        // corner, which the shape key would not match.
        let rotated = kicad10_board("(uuid \"z-pour\")", FILL_RESPELT).replace(
            "(xy 0.0 0.0) (xy 10.000 0) (xy 10 10) (xy 0 10)",
            "(xy 10 10) (xy 0 10) (xy 0 0) (xy 10 0)",
        );
        let scene = Scene::new(&saved, Some(&rotated));
        let (_, body) = scene.run("run_drc", json!({})).await;
        assert_eq!(body["zone_fill_state"], "fresh", "{body}");

        // A real change on the same zone is still stale, and named by its id.
        let scene = Scene::new(&saved, Some(&kicad10_board("(uuid \"z-pour\")", REFILL)));
        let (_, body) = scene.run("run_drc", json!({})).await;
        assert_eq!(body["zone_fill_state"], "stale", "{body}");
        assert_eq!(
            body["zone_fill_check"]["differing_zones"][0]["id"],
            "z-pour"
        );
    }

    /// An id KiCad rewrites on load (a KiCad 5 timestamp, a missing id) pairs
    /// by net, layers and outline; a zone nothing pairs is unknown, never stale.
    #[tokio::test]
    async fn zones_whose_ids_change_pair_by_shape_or_are_unknown() {
        let saved = kicad7_board("(tstamp 5C1234AB)", FILL);
        let rewritten = "(uuid \"00000000-0000-0000-0000-00005c1234ab\")";
        let scene = Scene::new(&saved, Some(&kicad10_board(rewritten, FILL_RESPELT)));
        let (_, body) = scene.run("run_drc", json!({})).await;
        assert_eq!(body["zone_fill_state"], "fresh", "{body}");
        assert_eq!(body["zone_fill_check"]["zones_compared"], 1);

        let scene = Scene::new(&saved, Some(&kicad10_board(rewritten, REFILL)));
        let (_, body) = scene.run("run_drc", json!({})).await;
        assert_eq!(body["zone_fill_state"], "stale", "{body}");

        // A new id and a moved outline: no evidence either way.
        let moved = kicad10_board(rewritten, REFILL).replace("(xy 10 10)", "(xy 10 11)");
        let scene = Scene::new(&saved, Some(&moved));
        let (_, body) = scene.run("run_drc", json!({})).await;
        assert_eq!(body["zone_fill_state"], "unknown", "{body}");
        let check = &body["zone_fill_check"];
        assert_eq!(check["method"], "compared_with_refill");
        assert_eq!(check["zones_unpaired"], 1);
        assert_eq!(check["zones_compared"], 0);
        assert!(
            check["reason"].as_str().unwrap().contains("paired"),
            "{check}"
        );
    }

    /// Review #49: the private copy has no library tables, so a footprint from
    /// a project library is "missing" there. A refill cannot change that, so
    /// it is not counted as the refill's doing.
    #[tokio::test]
    async fn library_checks_the_copy_cannot_reproduce_are_not_counted() {
        let refill_report = r#"{"violations":[{"type":"silk_overlap","severity":"warning","description":"Silkscreen overlap","items":[{"description":"Text","pos":{"x":1,"y":1}}]},{"type":"lib_footprint_issues","severity":"warning","description":"The current configuration does not include the footprint library 'Local'","items":[{"description":"Footprint R1","pos":{"x":10,"y":10}}]}],"unconnected_items":[]}"#;
        let saved = board(FILL);
        let scene = Scene::new(&saved, Some(&saved)).with_refill_report(refill_report);
        let (_, body) = scene.run("run_drc", json!({})).await;
        let check = &body["zone_fill_check"];
        assert_eq!(check["violations_added_by_refill"], 0, "{check}");
        assert!(check.get("added_by_refill").is_none(), "{check}");
        assert_eq!(
            check["rules_not_compared"],
            json!(["lib_footprint_issues", "lib_footprint_mismatch"])
        );
    }

    fn footprint_zone(fill: &str) -> String {
        format!(
            r#"  (footprint "Local:RF" (layer "F.Cu") (uuid "fp-1")
    (zone (net "GND") (layers "F.Cu") (uuid "z-fp")
      (polygon (pts (xy 40 40) (xy 45 40) (xy 45 45)))
      (filled_polygon (layer "F.Cu") (pts {fill})))
    (zone (layers "F.Cu") (uuid "z-fp-keepout")
      (keepout (tracks not_allowed) (vias allowed) (pads allowed) (copperpour not_allowed) (footprints allowed))
      (polygon (pts (xy 50 50) (xy 55 50) (xy 55 55)))))
"#
        )
    }

    #[test]
    fn copper_zones_inside_footprints_are_read() {
        let source = format!(
            "(kicad_pcb (version 20260206)\n{})\n",
            footprint_zone("(xy 40 40) (xy 45 40) (xy 45 45)")
        );
        let zones = copper_zone_fills(&source).unwrap();
        assert_eq!(zones.len(), 1, "the keepout is not a copper zone");
        assert!(zones[0].in_footprint);
        assert_eq!(zones[0].id.as_deref(), Some("z-fp"));
    }

    /// Review #49 follow-up: fills are compared to KiCad's 1 nm. Respelt
    /// numbers agree, but a vertex moved by 20 nm is a different fill.
    #[test]
    fn fills_compare_to_the_nanometre_and_ignore_spelling() {
        let polygons = |fill: &str| copper_zone_fills(&board(fill)).unwrap()[0].polygons.clone();
        let respelt = "(xy 0.500000 0.5) (xy 9.5 0.50) (xy 9.5000 9.5) (xy 0.5 9.5)";
        assert_eq!(polygons(FILL), polygons(respelt));
        let moved = "(xy 0.50002 0.5) (xy 9.5 0.5) (xy 9.5 9.5) (xy 0.5 9.5)";
        assert_ne!(polygons(FILL), polygons(moved));
    }

    /// Review #49: zones inside footprints were not read, so a board whose only
    /// copper zones sat in footprints read fresh. kicad-cli's refill does not
    /// fill them, so they cannot be compared: the state is unknown.
    #[tokio::test]
    async fn footprint_zones_are_counted_and_make_the_state_unknown() {
        let fp = footprint_zone("(xy 40 40) (xy 45 40) (xy 45 45)");
        let only = format!("(kicad_pcb (version 20260206) (generator \"test\")\n{fp})\n");
        let scene = Scene::new(&only, Some(&only));
        let (_, body) = scene.run("run_drc", json!({})).await;
        assert_eq!(body["zone_fill_state"], "unknown", "{body}");
        let check = &body["zone_fill_check"];
        assert_eq!(check["method"], "not_comparable");
        assert_eq!(check["copper_zones"], 1);
        assert_eq!(check["footprint_zones"], 1);
        assert_eq!(scene.calls().len(), 1, "no refill could tell, so none ran");

        let both = board(FILL).replacen("  (zone", &format!("{fp}  (zone"), 1);
        let scene = Scene::new(&both, Some(&both));
        let (_, body) = scene.run("run_drc", json!({})).await;
        assert_eq!(body["zone_fill_state"], "unknown", "{body}");
        let check = &body["zone_fill_check"];
        assert_eq!(check["method"], "compared_with_refill");
        assert_eq!(check["zones_compared"], 1);
        assert_eq!(check["footprint_zones"], 1);
        assert!(check["reason"].as_str().unwrap().contains("footprints"));

        let (_, body) = scene.run("run_drc", json!({ "refill_zones": true })).await;
        assert_eq!(body["zone_fill_state"], "unknown", "{body}");
        assert_eq!(body["zone_fill_check"]["method"], "refilled_for_this_run");
    }

    #[tokio::test]
    async fn refilled_runs_unzoned_boards_and_opt_outs_need_no_second_pass() {
        let scene = Scene::new(&board(FILL), Some(&board(REFILL)));
        let (_, refilled) = scene.run("run_drc", json!({ "refill_zones": true })).await;
        assert_eq!(refilled["zone_fill_state"], "fresh");
        assert_eq!(
            refilled["zone_fill_check"]["method"],
            "refilled_for_this_run"
        );
        let (_, skipped) = scene
            .run("run_drc", json!({ "check_zone_fills": false }))
            .await;
        assert_eq!(skipped["zone_fill_state"], "unknown");
        assert_eq!(skipped["zone_fill_check"]["method"], "not_checked");
        assert_eq!(scene.calls().len(), 2, "one DRC each, no refill copy");

        let unzoned = "(kicad_pcb (version 20260206) (generator \"test\"))\n";
        let scene = Scene::new(unzoned, Some(unzoned));
        let (_, body) = scene.run("get_drc_violations", json!({})).await;
        assert_eq!(body["zone_fill_state"], "fresh");
        assert_eq!(body["zone_fill_check"]["method"], "no_copper_zones");
        assert_eq!(scene.calls().len(), 1);
    }

    #[tokio::test]
    async fn a_failed_comparison_is_unknown_and_keeps_the_drc_result() {
        let scene = Scene::new(&board(FILL), None);
        let (error, body) = scene.run("run_drc", json!({})).await;
        assert!(!error, "{body}");
        assert_eq!(body["zone_fill_state"], "unknown");
        assert_eq!(body["zone_fill_check"]["method"], "check_failed");
        assert!(body["zone_fill_check"]["reason"]
            .as_str()
            .unwrap()
            .contains("refill"));
        assert_eq!(body["design_rule_violations"], 2);
    }

    /// Review #49: the DRC baseline tools dropped `check_zone_fills`, so every
    /// save and compare on a board with zones ran DRC twice.
    #[tokio::test]
    async fn drc_baselines_forward_the_zone_fill_opt_out() {
        use super::super::check_baselines as baselines;
        let scene = Scene::new(&board(FILL), Some(&board(REFILL)));
        let board = scene.board.display().to_string();
        let parse = |result: CallToolResult| -> Value {
            let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
                panic!("expected text");
            };
            assert!(!result.is_error, "{text}");
            serde_json::from_str(text).unwrap()
        };
        let saved = parse(
            baselines::handle_save_check_baseline(
                &json!({ "board": board, "name": "b", "check_zone_fills": false }),
                &scene.ctx,
            )
            .await
            .unwrap(),
        );
        assert_eq!(
            saved["source_evidence"]["zone_fill"]["method"], "not_checked",
            "{saved}"
        );
        assert_eq!(scene.calls().len(), 1, "one DRC, no refill copy");
        let compared = parse(
            baselines::handle_compare_check_baseline(
                &json!({ "board": board, "name": "b", "check_zone_fills": false }),
                &scene.ctx,
            )
            .await
            .unwrap(),
        );
        assert_eq!(
            compared["source_evidence"]["zone_fill"]["method"], "not_checked",
            "{compared}"
        );
        assert_eq!(scene.calls().len(), 2);
        // The default still compares.
        let compared = parse(
            baselines::handle_compare_check_baseline(
                &json!({ "board": board, "name": "b" }),
                &scene.ctx,
            )
            .await
            .unwrap(),
        );
        assert_eq!(
            compared["source_evidence"]["zone_fill"]["state"], "stale",
            "{compared}"
        );
        assert_eq!(scene.calls().len(), 4, "a DRC and its refill copy");
        // An ERC baseline has no zones to skip.
        let schematic = scene.dir.path().join("demo.kicad_sch");
        std::fs::write(&schematic, "(kicad_sch (version 20250610))\n").unwrap();
        let refused = baselines::handle_save_check_baseline(
            &json!({ "schematic": schematic.display().to_string(), "name": "e", "check_zone_fills": false }),
            &scene.ctx,
        )
        .await
        .unwrap();
        let crate::mcp::protocol::ToolContent::Text { text } = &refused.content[0] else {
            panic!("expected text");
        };
        assert!(
            refused.is_error && text.contains("check_zone_fills"),
            "{text}"
        );
        assert_eq!(scene.calls().len(), 4, "refused before ERC ran");
    }

    #[tokio::test]
    async fn a_malformed_check_flag_is_refused_before_any_drc() {
        let scene = Scene::new(&board(FILL), Some(&board(REFILL)));
        let (error, body) = scene
            .run("run_drc", json!({ "check_zone_fills": "yes" }))
            .await;
        assert!(error, "{body}");
        assert!(scene.calls().is_empty());
    }

    /// Real KiCad on a scratch copy of a demo board: its fills were made by an
    /// older KiCad, so this one's refill differs (stale); once refilled and
    /// saved, a second check finds them fresh. Needs `KICAD_CLI`.
    #[tokio::test]
    #[ignore = "runs the real kicad-cli; set KICAD_CLI"]
    async fn real_kicad_finds_old_fills_stale_and_refilled_ones_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let board = dir.path().join("pic_programmer.kicad_pcb");
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../konnect-sexp/tests/fixtures/pic_programmer.kicad_pcb"
            ),
            &board,
        )
        .unwrap();
        let ctx = ToolContext::new(
            ServerConfig {
                kicad_cli: std::env::var("KICAD_CLI").expect("KICAD_CLI"),
                ..Default::default()
            },
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        let args = json!({ "board": board.display().to_string() });
        let check = |result: CallToolResult| -> Value {
            let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
                panic!("expected text");
            };
            serde_json::from_str(text).unwrap()
        };
        let before = std::fs::read(&board).unwrap();
        let old = check(
            super::super::verification::handle_run_drc(&args, &ctx)
                .await
                .unwrap(),
        );
        assert_eq!(
            old["zone_fill_state"], "stale",
            "{}",
            old["zone_fill_check"]
        );
        assert_eq!(
            std::fs::read(&board).unwrap(),
            before,
            "the board is never written"
        );
        // Refill and save the scratch board the way the owner would.
        cli::refill_drc_on_private_copy(&ctx.config.kicad_cli, &board)
            .await
            .unwrap();
        let new = check(
            super::super::verification::handle_run_drc(&args, &ctx)
                .await
                .unwrap(),
        );
        assert_eq!(
            new["zone_fill_state"], "fresh",
            "{}",
            new["zone_fill_check"]
        );
        // The same fills with KiCad 6/7 zone ids: kicad-cli writes them back
        // as `uuid`, and the zones still pair (review #49).
        let mut in_zone = false;
        let tstamped: Vec<String> = std::fs::read_to_string(&board)
            .unwrap()
            .lines()
            .map(|line| {
                if line == "\t(zone" {
                    in_zone = true;
                } else if in_zone && line.trim_start().starts_with("(uuid ") {
                    in_zone = false;
                    return line.replacen("(uuid ", "(tstamp ", 1);
                }
                line.to_owned()
            })
            .collect();
        assert!(tstamped.iter().any(|line| line.contains("(tstamp ")));
        std::fs::write(&board, tstamped.join("\n") + "\n").unwrap();
        let old_ids = check(
            super::super::verification::handle_run_drc(&args, &ctx)
                .await
                .unwrap(),
        );
        assert_eq!(
            old_ids["zone_fill_state"], "fresh",
            "{}",
            old_ids["zone_fill_check"]
        );
        assert_eq!(
            old_ids["zone_fill_check"]["zones_compared"],
            old_ids["zone_fill_check"]["copper_zones"]
        );
    }
}
