//! Row ZR, stage ZR5: pours in the rank (docs/POUR_RANK_DESIGN.md).
//!
//! Pure. Three things live here: what the routing did to the pours, measured against
//! KiCad's own fill of the board before it was routed (the damage); the pour key, which
//! is today's rank key (`route_judge::rank_measures`) with four entries added; and the
//! setting that says which of the two keys ranks. Nothing here changes today's key (PK1),
//! reads a file or starts a process.

use crate::route_judge::{Gates, Judged, Key, Measures};
use crate::zone_fill::ZoneLayerFill;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The step of the plane loss in the pour key (PK4): 5 % of a pour net's plane.
pub(crate) const PLANE_STEP: f64 = 0.05;

/// The length of the pour key: today's thirteen entries and four more (PK2).
pub(crate) const POUR_KEY_LEN: usize = 17;

/// A run's pour key; the least is the best. Entries 0 to 6 are today's 0 to 6 (the four
/// gates, the intent's length miss, KiCad's own length violations, acute corners); 9 to
/// 12 are today's 7 to 10 (the signal and power length steps, right angles, vias off the
/// pour nets); 15 and 16 are today's 11 and 12 (minus copper quality, the route seconds).
/// The four others are [`POUR_FAULTS`], [`PLANE_LOSS`], [`EXTRA_PIECES`] and
/// [`POUR_TRACK`].
pub(crate) type PourKey = [i64; POUR_KEY_LEN];

/// Entry 7: KiCad's `starved_thermal` violations on the judged board plus
/// [`PourDamage::lost_fills`] (PK5).
pub(crate) const POUR_FAULTS: usize = 7;

/// Entry 8: [`plane_step`] of [`PourDamage::plane_loss`] (PK4).
pub(crate) const PLANE_LOSS: usize = 8;

/// Entry 13: [`PourDamage::extra_pieces`] (PK6).
pub(crate) const EXTRA_PIECES: usize = 13;

/// Entry 14: [`pour_track_step`] of the run's track length on pour nets (PK7).
pub(crate) const POUR_TRACK: usize = 14;

// ─── The damage ──────────────────────────────────────────────────────────────

/// One pour net's fill on one copper layer, over every zone of the net on that layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct LayerFill {
    pub net: String,
    pub layer: String,
    /// Fill pieces. 0: the net has a zone on this layer and KiCad saved no fill for it.
    pub pieces: usize,
    /// The largest piece's area, mm²; 0 without fill.
    pub largest_mm2: f64,
    /// The area of all pieces, mm².
    pub area_mm2: f64,
}

/// The layer fills of `pour_fills`' rows (`zone_fill::pour_fills`: one row per zone and
/// layer): the rows of one net on one layer are joined, their pieces counted together,
/// the largest piece the largest of them all, their areas added. Ordered by net, then
/// layer. A net's zone layer with no piece still gives its row, with 0 pieces.
pub(crate) fn layer_fills(fills: &[ZoneLayerFill]) -> Vec<LayerFill> {
    todo!()
}

/// One pour net's plane after routing, for the reader of a result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct NetPlane {
    pub net: String,
    /// The layer whose share is `intact_share`: the best of the net's layers that had
    /// fill before routing, ties to the first in `before`'s order. `None` when the net
    /// had no fill before routing on any layer.
    pub layer: Option<String>,
    /// The largest piece after routing over the largest piece before, on that layer, at
    /// most 1. 1 when `layer` is `None` (nothing to lose).
    pub intact_share: f64,
}

/// What the routing did to the pours, against KiCad's own fill of the board before it
/// was routed (PK3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PourDamage {
    /// Layer fills that had a piece before routing and have none after.
    pub lost_fills: usize,
    /// The sum over the pour nets of `1 - intact_share` (PK4). 0: every pour net has a
    /// layer whose largest piece is as large as before.
    pub plane_loss: f64,
    /// The sum over the layer fills that had a piece before routing of the pieces after
    /// routing beyond the pieces before, never below 0 for a layer (PK6).
    pub extra_pieces: usize,
    /// One row per pour net of `before`, by name.
    pub nets: Vec<NetPlane>,
}

/// The damage of `after` against `before` (both from [`layer_fills`]; `before` is the
/// judge's baseline, `after` the judged board):
/// - A layer fill is matched by net and layer. One that is in `before` and not in
///   `after` counts as 0 pieces after (the zone is gone from the judged board).
/// - `lost_fills`: `before.pieces > 0` and no piece after.
/// - A net's `intact_share`: over its layer fills with `before.largest_mm2 > 0`, the
///   largest of `min(1, after.largest_mm2 / before.largest_mm2)`. A piece that grew is not
///   a gain: the share is never above 1.
/// - `plane_loss`: the sum of `1 - intact_share` over the nets of `before`. A net with no
///   fill before on any layer adds 0.
/// - `extra_pieces`: over the layer fills with `before.pieces > 0`, the sum of
///   `after.pieces.saturating_sub(before.pieces)`.
/// - A layer fill that is in `after` and not in `before` (a zone the unrouted board did
///   not have) is not read: the judged board is the unrouted board with a session written
///   into it, so it has the same zones.
///
/// `after == before` gives 0, 0.0 and 0, whatever the zones look like.
pub(crate) fn pour_damage(before: &[LayerFill], after: &[LayerFill]) -> PourDamage {
    todo!()
}

// ─── The pour key ────────────────────────────────────────────────────────────

/// [`PourKey`]'s entry 8 for a plane loss: `(plane_loss / PLANE_STEP).round()`, as the
/// length steps round (half away from zero). A loss that is not a finite number of 0 or
/// more gives `i64::MAX` (unmeasured sorts last).
pub(crate) fn plane_step(plane_loss: f64) -> i64 {
    todo!()
}

/// [`PourKey`]'s entry 14: the track length on pour nets in units of
/// `route_judge::LENGTH_STEP` of the pool's shortest signal length:
/// `(pour_track_mm / shortest_signal_mm / LENGTH_STEP).round()`. 0 when the run has no
/// track on a pour net, and 0 when the pool has no signal length to scale by (`None`): a
/// step with no unit cannot rank.
pub(crate) fn pour_track_step(pour_track_mm: f64, shortest_signal_mm: Option<f64>) -> i64 {
    todo!()
}

/// The pour key of every run at once (the length steps need the pool), in `runs`' order.
/// `runs` is what `route_judge::rank_measures` takes: the measures, the gates, and
/// whether the run is in the pool.
/// - Entries 0 to 6, 9 to 12, 15 and 16 are the entries of
///   `route_judge::rank_measures(runs)` (see [`PourKey`]), unchanged.
/// - Entries [`POUR_FAULTS`], [`PLANE_LOSS`] and [`EXTRA_PIECES`], from the run's pour
///   measures (`Measures::pours`) and their `damage` (the field this stage adds to
///   `zone_fill::PourMeasures`):
///   - no pour measures and no `pours_error`: this judge took no pour record, and all
///     three are 0 (PK8). That is a call with no pour net, and it is the prune's light
///     judge (`route_judge::judge_light`), which takes none for any of the three boards
///     it compares; the prune guards the pours in its loop with its own clauses (b) and
///     (d);
///   - pour measures with a damage: `starved_thermals + lost_fills`,
///     [`plane_step`]`(plane_loss)`, `extra_pieces`;
///   - anything else (a `pours_error`, or pour measures with no damage): `i64::MAX` in
///     all three. A run whose pours could not be measured never outranks one whose pours
///     were.
/// - Entry [`POUR_TRACK`]: [`pour_track_step`] of `Measures::pour_track_mm`, against the
///   shortest signal length of the same pool that `rank_measures` steps the signal length
///   against (the runs in the pool that pass G1, else the pool, else every run; this
///   stage makes that rule one helper in `route_judge` that both keys call).
///
/// With no pour net in the call the four entries are 0 for every run, so the pour key
/// orders the runs exactly as today's key does (PK2).
pub(crate) fn pour_keys(runs: &[(&Measures, &Gates, bool)]) -> Vec<PourKey> {
    todo!()
}

// ─── The setting ─────────────────────────────────────────────────────────────

/// Which key ranks (PK9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RankMode {
    /// Today's key (`route_judge::rank_measures`). The default.
    Routing,
    /// The pour key ([`pour_keys`]).
    Pours,
}

impl RankMode {
    /// `routing` or `pours`: the word of the user config and of a result.
    pub(crate) fn name(self) -> &'static str {
        todo!()
    }

    /// The mode a word names; `None` for any other text (the match is exact).
    pub(crate) fn parse(text: &str) -> Option<RankMode> {
        todo!()
    }

    /// The other mode.
    pub(crate) fn other(self) -> RankMode {
        todo!()
    }
}

/// Where a call's mode came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RankSource {
    UserConfig,
    Default,
}

/// The mode of a call: the user config's `routing.rank_key` (`config` is that value when
/// the config has it), else [`RankMode::Routing`]. A JSON null is "not set". A value that
/// is not a string naming one of the two modes is refused; the message names
/// `routing.rank_key`, the value and both names, as `route_search::resolve_effort` does
/// for the effort. There is no tool argument (PK9).
pub(crate) fn resolve_rank_mode(
    config: Option<&Value>,
) -> std::result::Result<(RankMode, RankSource), String> {
    todo!()
}

// ─── What every ranking site calls ───────────────────────────────────────────

/// The keys that rank `runs` under `mode`, in `runs`' order; the least is the best.
/// [`RankMode::Routing`]: each run's key of `route_judge::rank_measures(runs)`, entry for
/// entry. [`RankMode::Pours`]: each run's key of [`pour_keys`]. The keys of one call have
/// one length. Every site that ranks calls this or [`ranking_of`] (PK12): the search's
/// winner and the runs it releases, the choice among a run's tuned candidates, the
/// prune's clause (c) and its final comparison, and the result's listing.
pub(crate) fn ranking(mode: RankMode, runs: &[(&Measures, &Gates, bool)]) -> Vec<Vec<i64>> {
    todo!()
}

/// [`ranking`] of judged runs that are all in the pool, as `route_judge::rank` is of
/// `rank_measures`.
pub(crate) fn ranking_of(mode: RankMode, judged: &[&Judged]) -> Vec<Vec<i64>> {
    todo!()
}

/// The positions at which `next` is above `now`, the last entry (the route seconds) left
/// out: what the prune's clause (c) refuses on. Both are keys of one [`ranking`] call, so
/// they have one length; with different lengths only the positions both have, less the
/// last of the shorter, are compared.
pub(crate) fn entries_above(now: &[i64], next: &[i64]) -> Vec<usize> {
    todo!()
}

/// `search.rank` of a result (PK10):
/// `{ranked_by, source, other_key, other_winner, same_winner}`.
/// - `ranked_by` and `other_key`: the mode's name and the other mode's.
/// - `source`: where the mode came from.
/// - `other_winner`: the index of the run with the least key of the other mode among the
///   runs that have one, ties to the lower index; `null` when no run has one. `keys` are
///   today's keys by run index and `pour_keys` the pour keys, each `None` for a run that
///   was not judged. A judged run the search released still has both.
/// - `same_winner`: whether that index is `winner`.
pub(crate) fn rank_report(
    mode: RankMode,
    source: RankSource,
    keys: &[Option<Key>],
    pour_keys: &[Option<PourKey>],
    winner: Option<usize>,
) -> Value {
    todo!()
}

#[cfg(test)]
#[path = "pour_rank_tests.rs"]
mod tests;
