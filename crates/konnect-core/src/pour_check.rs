//! Row ZR, stage ZR4: KiCad's own fill of a pour plan (docs/POUR_PLAN_DESIGN.md, PP11).
//!
//! A plan is returned only after KiCad has filled it. The board as it was read, and the
//! same board with the proposals, are written to scratch copies below Konnect's state
//! folder, refilled and checked by kicad-cli, and compared. Nothing here writes to the
//! user's board or project, and kicad-cli runs with a private copy of KiCad's settings.

use crate::pour_plan::ProposedZone;
use crate::tools::cli::DrcReport;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

/// The folder below Konnect's state folder that holds one folder per call.
pub(crate) const PLANS_FOLDER: &str = "pour-plans";

/// The scratch project of the board as it was read.
pub(crate) const WITHOUT: &str = "without";

/// The scratch project of the board with the proposals.
pub(crate) const WITH: &str = "with";

/// One refill and DRC in flight.
pub(crate) type RefillCall<'a> = Pin<Box<dyn Future<Output = Result<DrcReport>> + Send + 'a>>;

/// How the check has KiCad refill a scratch board and report on it. The one production
/// implementation is [`KicadCli`]; the tests of the check put a scripted one in its
/// place (as `route_judge::ToolCaller` does for the judge's tools).
pub(crate) trait Refill: Send + Sync {
    /// Refills the zones of the scratch board at `board`, saves the refilled board over
    /// that file and returns its DRC.
    fn refill<'a>(&'a self, board: &'a Path) -> RefillCall<'a>;
}

/// kicad-cli, with `config_home` as its `KICAD_CONFIG_HOME`
/// (`tools::cli::refill_drc_on_private_copy_with_config`), so that it never writes the
/// owner's own KiCad settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KicadCli {
    pub kicad_cli: String,
    /// A private copy of KiCad's settings (`route_judge::private_kicad_config`).
    pub config_home: PathBuf,
}

impl Refill for KicadCli {
    fn refill<'a>(&'a self, board: &'a Path) -> RefillCall<'a> {
        Box::pin(async move {
            crate::tools::cli::refill_drc_on_private_copy_with_config(
                &self.kicad_cli,
                board,
                Some(&self.config_home),
            )
            .await
        })
    }
}

/// The fill of one proposal, as KiCad saved it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ProposalFill {
    pub net: String,
    pub layer: String,
    /// Fill pieces of the proposal's zone on its layer. 0: KiCad filled nothing of it.
    pub pieces: usize,
    pub area_mm2: f64,
    /// The largest piece's share of the area; `None` without fill.
    pub largest_piece_share: Option<f64>,
}

/// KiCad's unconnected items that name one net, without and with the proposals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NetOpen {
    pub net: String,
    pub before: usize,
    pub after: usize,
}

/// A DRC rule that KiCad reports more often with the proposals than without.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Risen {
    /// KiCad's rule key (`starved_thermal`, `isolated_copper`, …).
    pub rule: String,
    /// `error` or `warning`, as the report gives it.
    pub severity: String,
    pub before: usize,
    pub after: usize,
}

/// What KiCad's refill and DRC say about a plan.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct FillCheck {
    /// One row per proposal, in the plan's order.
    pub fills: Vec<ProposalFill>,
    /// One row per proposed net, by name.
    pub open: Vec<NetOpen>,
    /// By rule, then severity.
    pub risen: Vec<Risen>,
    /// KiCad's `starved_thermal` violations without the proposals, at any severity.
    pub starved_thermals_before: usize,
    /// KiCad's `starved_thermal` violations with the proposals, at any severity.
    pub starved_thermals: usize,
    /// No rule of severity `error` rose, and `starved_thermal` did not rise at any
    /// severity: a project may report it as a warning, and a pad of the pour's net with
    /// too few spokes is a finding all the same. A proposal KiCad filled nothing of is
    /// not part of this: the tool withdraws it.
    pub clean: bool,
}

/// The verdict of two reports and the refilled board (pure):
/// - `fills`: for each proposal, the row of its net and layer in
///   `zone_fill::pour_fills(refilled_text, the proposed nets)`. The plan proposes a net
///   only on a layer where it has no pour yet (PP8), so that row is the proposal's own.
///   A proposal with no row is an error (a zone that was written is always read back).
/// - `open`: `route_judge::unconnected_on` of each proposed net in `without` and in
///   `with`.
/// - `risen`: the violations of each report counted by rule and severity; every pair
///   whose count in `with` is above its count in `without`.
/// - `starved_thermals_before` and `starved_thermals`: the violations of
///   `zone_fill::STARVED_THERMAL` in `without` and in `with`.
/// - `clean`: no `risen` row has the severity `error`, and `starved_thermals` is not
///   above `starved_thermals_before`.
///
/// An error when either report has no unconnected-items category: an unrouted board
/// could then not be told from a routed one (`route_judge::unconnected_total`).
pub(crate) fn verdict(
    zones: &[ProposedZone],
    without: &DrcReport,
    with: &DrcReport,
    refilled_text: &str,
) -> Result<FillCheck> {
    // Only that the category is there: `open` counts it per net.
    crate::route_judge::unconnected_total(without)
        .context("the DRC of the board without the proposals")?;
    crate::route_judge::unconnected_total(with)
        .context("the DRC of the board with the proposals")?;
    let nets: BTreeSet<String> = zones.iter().map(|zone| zone.net.clone()).collect();
    let rows = crate::zone_fill::pour_fills(refilled_text, &nets)?;
    let mut fills = Vec::with_capacity(zones.len());
    for zone in zones {
        let mut own = rows
            .iter()
            .filter(|row| row.net == zone.net && row.layer == zone.layer);
        let Some(row) = own.next() else {
            bail!(
                "the refilled board has no zone of {} on {}, though the check wrote one",
                zone.net,
                zone.layer
            );
        };
        if own.next().is_some() {
            bail!(
                "the refilled board has more than one zone of {} on {}, so the proposal's own \
                 fill cannot be told apart",
                zone.net,
                zone.layer
            );
        }
        fills.push(ProposalFill {
            net: zone.net.clone(),
            layer: zone.layer.clone(),
            pieces: row.pieces_mm2.len(),
            area_mm2: row.pieces_mm2.iter().sum(),
            largest_piece_share: row.largest_share(),
        });
    }
    let open = nets
        .iter()
        .map(|net| {
            let one = std::slice::from_ref(net);
            NetOpen {
                net: net.clone(),
                before: crate::route_judge::unconnected_on(without, one),
                after: crate::route_judge::unconnected_on(with, one),
            }
        })
        .collect();
    let counts = |report: &DrcReport| {
        let mut by_rule: BTreeMap<(String, String), usize> = BTreeMap::new();
        for violation in report.all() {
            *by_rule
                .entry((violation.rule.clone(), violation.severity.clone()))
                .or_default() += 1;
        }
        by_rule
    };
    let (counts_without, counts_with) = (counts(without), counts(with));
    let risen: Vec<Risen> = counts_with
        .iter()
        .filter_map(|((rule, severity), &after)| {
            let before = counts_without
                .get(&(rule.clone(), severity.clone()))
                .copied()
                .unwrap_or(0);
            (after > before).then(|| Risen {
                rule: rule.clone(),
                severity: severity.clone(),
                before,
                after,
            })
        })
        .collect();
    let starved = |report: &DrcReport| {
        report
            .all()
            .filter(|violation| violation.rule == crate::zone_fill::STARVED_THERMAL)
            .count()
    };
    let (starved_thermals_before, starved_thermals) = (starved(without), starved(with));
    let clean = !risen.iter().any(|row| row.severity == "error")
        && starved_thermals <= starved_thermals_before;
    Ok(FillCheck {
        fills,
        open,
        risen,
        starved_thermals_before,
        starved_thermals,
        clean,
    })
}

/// The folder of one call: `<state>/pour-plans/<stamp>-<the first 8 characters of the
/// SHA-256 of board_text>`, created with its parents. One that exists already is an
/// error: two calls never share a folder.
pub(crate) fn run_folder(state: &Path, board_text: &str, stamp: &str) -> Result<PathBuf> {
    let plans = state.join(PLANS_FOLDER);
    std::fs::create_dir_all(&plans).with_context(|| format!("create '{}'", plans.display()))?;
    let digest = crate::place_held::sha256_hex(board_text.as_bytes());
    let folder = plans.join(format!("{stamp}-{}", &digest[..8]));
    std::fs::create_dir(&folder)
        .with_context(|| format!("create the plan's folder '{}'", folder.display()))?;
    Ok(folder)
}

/// A scratch project in `folder/<name>`: the project files beside `board`
/// (`scratch_board::copy_project`) and `text` written as the board. The scratch copy's
/// project reports what a pour changes whatever the project's own severities say, in the
/// scratch copy only: the four rules a pour can break at least as warnings
/// (`length_probe::force_prune_severities`: isolated copper and starved thermals among
/// them), and unconnected items as errors (`length_probe::force_check_severities`), so
/// that `open` counts real items on a project that ignores them. Returns the scratch
/// board's path.
fn scratch(folder: &Path, name: &str, board: &Path, text: &str) -> Result<PathBuf> {
    let copy = crate::scratch_board::copy_project(board, &folder.join(name))?;
    std::fs::write(&copy, text).with_context(|| format!("write '{}'", copy.display()))?;
    crate::length_probe::force_prune_severities(&copy)?;
    crate::length_probe::force_check_severities(&copy)?;
    Ok(copy)
}

/// The check of one plan, in `folder` (made by [`run_folder`]):
/// 1. [`scratch`] of [`WITHOUT`] with `board_text`, and of [`WITH`] with
///    `pour_plan::with_zones(board_text, zones)`.
/// 2. `refill` on each, one after the other: the two runs share one settings folder,
///    and kicad-cli saves its settings again when it exits.
/// 3. [`verdict`] of the two reports and the refilled `with` board, which is read and
///    parsed off the async threads.
///
/// A step that fails is an error that names the step. The folder is kept either way: it
/// is the record of what KiCad was shown.
pub(crate) async fn check(
    folder: &Path,
    board: &Path,
    board_text: &str,
    zones: &[ProposedZone],
    refill: &dyn Refill,
) -> Result<FillCheck> {
    let without = scratch(folder, WITHOUT, board, board_text)
        .context("step 1, the scratch project of the board as it was read")?;
    let with_text = crate::pour_plan::with_zones(board_text, zones)
        .context("step 1, the board with the proposals")?;
    let with = scratch(folder, WITH, board, &with_text)
        .context("step 1, the scratch project of the board with the proposals")?;
    let report_without = refill
        .refill(&without)
        .await
        .context("step 2, kicad-cli's refill and DRC of the board as it was read")?;
    let report_with = refill
        .refill(&with)
        .await
        .context("step 2, kicad-cli's refill and DRC of the board with the proposals")?;
    let zones = zones.to_vec();
    tokio::task::spawn_blocking(move || {
        let refilled = std::fs::read_to_string(&with)
            .with_context(|| format!("read the refilled board '{}'", with.display()))?;
        verdict(&zones, &report_without, &report_with, &refilled)
    })
    .await
    .context("step 3, the verdict")?
    .context("step 3, the verdict")
}

#[cfg(test)]
#[path = "pour_check_tests.rs"]
mod tests;
