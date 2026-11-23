//! Row ZR, stage ZR4: the pours a board should have (docs/POUR_PLAN_DESIGN.md).
//!
//! Pure: a plan is a function of the board's text, the project file's text and the
//! design-intent record. Nothing here reads a file, starts a process or changes a board.
//! A plan is a proposal: a zone enters a board only through `add_zone` (PP1). KiCad's own
//! fill of a plan is `pour_check`'s; the tool that joins the two is
//! `tools::pour_plan_tool`.

use crate::tools::design_intent::{DesignIntent, FabTarget, PourIntent};
use crate::tools::pcb_board::ZoneSettings;
use crate::tools::SupplyRoles;
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// KiCad's default minimum width of a new zone, mm (`zones.h`, `ZONE_THICKNESS_MM`).
pub(crate) const MIN_WIDTH_MM: f64 = 0.25;

/// The clearance of KiCad's own Default netclass, mm: what a net has when the project
/// file holds no class (`pcb_routing::kicad_default_class`).
pub(crate) const SEEDED_CLASS_CLEARANCE_MM: f64 = 0.2;

/// What a net is to the board's supply (PP4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SupplyRole {
    Ground,
    Power,
}

/// One supply net of the board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SupplyNet {
    pub net: String,
    pub role: SupplyRole,
    /// Where the role comes from, in the layout audit's words: a design-intent rail, a
    /// netclass, the net's name; or "asked for in the design intent" for a net of the
    /// record's `pours` that has no role.
    pub source: String,
    /// The net's copper pads on the board, as the Specctra export counts them.
    pub pads: usize,
}

/// One copper layer of the board's stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CopperLayer {
    pub name: String,
    /// `signal` or `power`, as the export reads the board's layer table (KiCad's `mixed`
    /// is `signal`). Reported only: no rule reads it (PP6).
    pub kind: String,
}

/// A pour the board already has, on one copper layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExistingPour {
    pub net: String,
    pub layer: String,
    pub priority: u32,
}

/// What a plan reads off a board.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BoardFacts {
    /// The copper layers, outermost first (`F.Cu`, `In1.Cu` …, `B.Cu`).
    pub stack: Vec<CopperLayer>,
    /// The board's outline in KiCad's own coordinates, mm (Y down): one closed loop, the
    /// first corner not repeated.
    pub outline_mm: Vec<(f64, f64)>,
    /// Copper pads per net; a net with no pad has no entry.
    pub pads: BTreeMap<String, usize>,
    /// The copper pours of the board, one row per zone and copper layer. Rule areas and
    /// teardrops are not pours.
    pub existing: Vec<ExistingPour>,
}

/// The facts of a board's text:
/// - `stack` from `specctra::board_copper_stack`;
/// - `outline_mm` from `specctra::board_outline_um`: each corner `(x / 1000, -y / 1000)`,
///   the repeated last corner dropped;
/// - `pads` from `specctra::pad_pins`, counted by net;
/// - `existing` from `specctra::pour_fill_rows`, each row's priority read from its
///   `(zone …)` node (`(priority N)`, 0 when absent).
///
/// An error wherever the Specctra export itself refuses the board (an outline that is not
/// one closed loop, a copper stack it does not support, a zone or a footprint it cannot
/// read), with the export's own reason: pours are defined before the export (Z8), so a
/// board that cannot be exported gets no plan (PP10).
pub(crate) fn board_facts(board_text: &str) -> Result<BoardFacts> {
    todo!()
}

/// The project's numbers a pour's settings depend on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProjectRules {
    /// `board.design_settings.rules.min_clearance`, mm; 0 without one.
    pub min_clearance_mm: f64,
    /// `board.design_settings.rules.min_track_width`, mm; 0 without one.
    pub min_track_width_mm: f64,
    /// `net_settings.classes[].clearance` by class name, mm.
    pub class_clearance_mm: BTreeMap<String, f64>,
}

/// The rules of a project file's text. `None` (no project file beside the board) gives
/// KiCad's own: both minima 0 and no class.
///
/// `Err(reason)` when the text is not JSON, or one of the values read is present and is
/// not a finite number of 0 or more: a pour's clearance is never taken from a file that
/// does not read (P4).
pub(crate) fn project_rules(
    project_text: Option<&str>,
) -> std::result::Result<ProjectRules, String> {
    todo!()
}

/// The supply nets on the board, in the order the rules read them: the ground nets by
/// pads (most first), then by name; then the power nets the same way.
/// - A net with a role and no pad on the board is left out.
/// - A net that `asked` names and that has no role is added as power, with the source
///   "asked for in the design intent".
/// - A ground net that `asked` names without layers is moved to the head of the ground
///   nets, and a power net that `asked` names without layers to the head of the power
///   nets, in the record's order (PP7).
pub(crate) fn supply_nets(
    roles: &SupplyRoles,
    pads: &BTreeMap<String, usize>,
    asked: &[PourIntent],
) -> Vec<SupplyNet> {
    todo!()
}

/// One copper layer given to one net.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slot {
    pub layer: String,
    pub net: String,
    /// Why this net on this layer, in one sentence.
    pub reason: String,
    /// Whether the design intent's `pours` asked for this net on this layer.
    pub asked: bool,
}

/// A pour that was considered and is not proposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NotProposed {
    /// `None` for a reason that is about the board and not about one net (a stack with no
    /// rule, no ground net known).
    pub net: Option<String>,
    pub layer: Option<String>,
    pub reason: String,
    /// Whether the design intent's `pours` asked for it: an unmet request.
    pub asked: bool,
}

/// The copper layers divided among the supply nets, and what was left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Division {
    /// In stack order.
    pub slots: Vec<Slot>,
    pub left_out: Vec<NotProposed>,
}

/// The division (PP5 to PP7). One net a layer, whole layers only.
/// 1. **Asked, with layers.** Each entry of `asked` that names layers takes each of them
///    for its net, and that net gets no other layer. A layer the board's stack does not
///    have, and a net with no pad on the board, are left out as unmet requests.
/// 2. **The rule,** for the layers step 1 left free. `ground` is the first ground net of
///    `nets` that no entry of `asked` names with layers, and `power` the first such power
///    net (`supply_nets` has put a net asked without layers first):
///    - 2 layers: `ground` on both;
///    - 4 layers: `ground` on `F.Cu`, `In1.Cu` and `B.Cu`; `power` on `In2.Cu`, and
///      `ground` there when there is no `power`;
///    - any other count: nothing, and one `left_out` row with no net and no layer says
///      that this stack has no rule and that the design intent's `pours` can name layers.
///    A layer the rule gives to `ground` when there is none stays free, with one
///    `left_out` row for that layer and no net. Its reason says which it is: no ground
///    net is known (and the design intent's `rails` can record one), or the only ground
///    net was asked for on other layers.
/// 3. **Left out,** each with its reason: every supply net of `nets` that got no layer
///    ("each layer takes one whole-layer pour, and every layer has one"), and an asked
///    net without layers that the rule gave no layer (an unmet request).
pub(crate) fn division(
    stack: &[CopperLayer],
    nets: &[SupplyNet],
    asked: &[PourIntent],
) -> Division {
    todo!()
}

/// Whether `slot` can be added to a board that already has `existing` (PP8). `Err(reason)`:
/// - the net already has a pour on the layer ("already poured; existing zones are never
///   changed");
/// - another net has a pour on the layer at priority 0: a whole-layer pour must sit below
///   it, and KiCad has no priority below 0, so the reason names that zone's net and says
///   to raise its priority.
/// Another net's pours at priority 1 or more are fine: the proposal, at priority 0, fills
/// what they leave.
pub(crate) fn against_existing(
    slot: &Slot,
    existing: &[ExistingPour],
) -> std::result::Result<(), String> {
    todo!()
}

/// The settings of a pour of `net` (PP9):
/// - clearance: the largest of the clearances of the net's classes (`classes`, the names
///   the audit gives the net) and of the Default class (the file's, else
///   [`SEEDED_CLASS_CLEARANCE_MM`]), `rules.min_clearance_mm` and the fab target's
///   `min_clearance_mm`;
/// - minimum width: the largest of [`MIN_WIDTH_MM`], `rules.min_track_width_mm` and the
///   fab target's `min_track_mm`;
/// - thermal pads, a gap of `konnect_ipc::builders::ZONE_THERMAL_RELIEF_MM`, a bridge of
///   the larger of that and the minimum width; islands always removed; priority 0.
/// The result passes `ZoneSettings::check`.
pub(crate) fn settings_for(
    classes: &[String],
    rules: &ProjectRules,
    fab: Option<&FabTarget>,
) -> ZoneSettings {
    todo!()
}

/// One proposed zone: every value `add_zone` takes but the board.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProposedZone {
    pub net: String,
    pub layer: String,
    pub points_mm: Vec<(f64, f64)>,
    pub settings: ZoneSettings,
    pub reason: String,
    pub asked: bool,
}

/// A plan: what to add, and what was left out.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PourPlan {
    pub stack: Vec<CopperLayer>,
    /// The supply nets considered, in the rules' order.
    pub nets: Vec<SupplyNet>,
    /// In stack order.
    pub proposals: Vec<ProposedZone>,
    pub not_proposed: Vec<NotProposed>,
}

/// The plan of a board: [`supply_nets`], [`division`], then each slot through
/// [`against_existing`] (a refused slot is `not_proposed` with the reason), then
/// [`settings_for`] and the board's outline for every slot that stays. `intent` is the
/// project's record when it has one: its `pours` are the requests, its `fab_target` the
/// fab's minima.
pub(crate) fn plan(
    facts: &BoardFacts,
    roles: &SupplyRoles,
    rules: &ProjectRules,
    intent: Option<&DesignIntent>,
) -> PourPlan {
    todo!()
}

/// The `add_zone` arguments that add one proposal to `board`: `board`, `net_name`,
/// `layer`, `points` and every setting (`ZoneSettings::to_args`), the defaults included,
/// so that the zone KiCad checked is the zone that is added (PP14).
pub(crate) fn add_zone_arguments(board: &Path, zone: &ProposedZone) -> Value {
    todo!()
}

/// `board_text` with each proposal's zone added before the closing parenthesis of its
/// root, written by `add_zone`'s own file writer (`tools::pcb_board::zone_sexp`), unnamed.
/// A net that a legacy board does not declare is refused (never net 0), and so is a text
/// whose root is not `kicad_pcb`. Nothing else in the text changes.
pub(crate) fn with_zones(board_text: &str, zones: &[ProposedZone]) -> Result<String> {
    todo!()
}

#[cfg(test)]
#[path = "pour_plan_tests.rs"]
mod tests;
