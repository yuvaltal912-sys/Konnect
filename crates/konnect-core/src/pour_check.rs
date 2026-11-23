//! Row ZR, stage ZR4: KiCad's own fill of a pour plan (docs/POUR_PLAN_DESIGN.md, PP11).
//!
//! A plan is returned only after KiCad has filled it. The board as it was read, and the
//! same board with the proposals, are written to scratch copies below Konnect's state
//! folder, refilled and checked by kicad-cli, and compared. Nothing here writes to the
//! user's board or project, and kicad-cli runs with a private copy of KiCad's settings.

use crate::pour_plan::ProposedZone;
use crate::tools::cli::DrcReport;
use anyhow::Result;
use serde::Serialize;
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
        todo!()
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
    todo!()
}

/// The folder of one call: `<state>/pour-plans/<stamp>-<the first 8 characters of the
/// SHA-256 of board_text>`, created with its parents. One that exists already is an
/// error: two calls never share a folder.
pub(crate) fn run_folder(state: &Path, board_text: &str, stamp: &str) -> Result<PathBuf> {
    todo!()
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
    todo!()
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
    todo!()
}

#[cfg(test)]
#[path = "pour_check_tests.rs"]
mod tests;
