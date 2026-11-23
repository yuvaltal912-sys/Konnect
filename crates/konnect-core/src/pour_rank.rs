//! Row ZR, stage ZR5, part 2: the pour key and its setting (docs/POUR_RANK_DESIGN.md).
//!
//! Pure. The pour key is today's rank key (`route_judge::rank_measures`) with four
//! entries added from part 1's pour record (`pour_damage`), and a setting says which of
//! the two keys ranks. Nothing here changes today's key (PK1), reads a file or starts a
//! process.
//!
//! Part 2 is built after ZR3's numbers: the two steps and the two places below are the
//! candidate, confirmed on recorded runs before this file gets its bodies.

use crate::route_judge::{Gates, Judged, Key, Measures};
use serde::Serialize;
use serde_json::Value;

/// The step of the plane loss in the pour key (PK4): 5 % of the planes' area.
pub(crate) const PLANE_STEP: f64 = 0.05;

/// The step of the area loss in the pour key (PK6): 1 % of the pour copper.
pub(crate) const AREA_STEP: f64 = 0.01;

/// The length of the pour key: today's thirteen entries and four more (PK2).
pub(crate) const POUR_KEY_LEN: usize = 17;

/// A run's pour key; the least is the best. Entries 0 to 6 are today's 0 to 6 (the four
/// gates, the intent's length miss, KiCad's own length violations, acute corners); 9 to
/// 12 are today's 7 to 10 (the signal and power length steps, right angles, vias off the
/// pour nets); 15 and 16 are today's 11 and 12 (minus copper quality, the route seconds).
/// The four others are [`POUR_FAULTS`], [`PLANE_LOSS`], [`AREA_LOSS`] and
/// [`POUR_TRACK`].
pub(crate) type PourKey = [i64; POUR_KEY_LEN];

/// Entry 7: KiCad's `starved_thermal` violations on the judged board plus the damage's
/// `lost_fills` (PK5).
pub(crate) const POUR_FAULTS: usize = 7;

/// Entry 8: [`plane_step`] of the damage's `plane_loss` (PK4).
pub(crate) const PLANE_LOSS: usize = 8;

/// Entry 13: [`area_step`] of the damage's `area_loss` (PK6).
pub(crate) const AREA_LOSS: usize = 13;

/// Entry 14: [`pour_track_step`] of the run's track length on pour nets (PK7).
pub(crate) const POUR_TRACK: usize = 14;

// ─── The pour key ────────────────────────────────────────────────────────────

/// [`PourKey`]'s entry 8 for a plane loss: `(plane_loss / PLANE_STEP).round()`, as the
/// length steps round (half away from zero). A loss that is not a finite number of 0 or
/// more gives `i64::MAX` (unmeasured sorts last).
pub(crate) fn plane_step(plane_loss: f64) -> i64 {
    todo!()
}

/// [`PourKey`]'s entry 13 for an area loss: `(area_loss / AREA_STEP).round()`, with the
/// same rule for a loss that is not a finite number of 0 or more.
pub(crate) fn area_step(area_loss: f64) -> i64 {
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
/// - Entries [`POUR_FAULTS`], [`PLANE_LOSS`] and [`AREA_LOSS`], from the run's pour
///   measures (`Measures::pours`) and their `damage` (part 1's record):
///   - no pour measures and no `pours_error`: this judge took no pour record, and all
///     three are 0 (PK8). That is a call with no pour net, and it is the prune's light
///     judge (`route_judge::judge_light`), which takes none for any of the three boards
///     it compares; the prune guards the pours in its loop with its own clauses (b) and
///     (d);
///   - pour measures with a damage: `starved_thermals + lost_fills`,
///     [`plane_step`]`(plane_loss)`, [`area_step`]`(area_loss)`;
///   - anything else (a `pours_error`, or pour measures with no damage): `i64::MAX` in
///     all three. A run whose pours could not be measured never outranks one whose pours
///     were.
/// - Entry [`POUR_TRACK`]: [`pour_track_step`] of `Measures::pour_track_mm`, against the
///   shortest signal length of the same pool that `rank_measures` steps the signal length
///   against (the runs in the pool that pass G1, else the pool, else every run; part 2
///   makes that rule and the step two `pub(crate)` helpers of `route_judge` that both
///   keys call, and `rank_measures` returns what it returned).
///
/// With no pour net in the call the four entries are 0 for every run, so the pour key
/// orders the runs exactly as today's key does, ties included (PK2).
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
    /// The user config's `routing.rank_key`.
    UserConfig,
    /// No setting: [`RankMode::Routing`].
    Default,
    /// The caller fixed it ([`RankChoice::Fixed`]).
    Fixed,
}

/// Where a routing call takes its mode from: what `route_with_fastroute` is given by its
/// caller, beside the prune's switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RankChoice {
    /// The user config's `routing.rank_key`, read on every call whatever the call's
    /// arguments (PK9). The route tool.
    UserConfig,
    /// This mode, and the user config is not read for it. The placement tools' routing
    /// calls fix [`RankMode::Routing`] (`place_route::CANDIDATE_RANK`, PK13).
    Fixed(RankMode),
}

/// The mode of a call that reads the user config: `routing.rank_key` (`config` is that
/// value when the config has it), else [`RankMode::Routing`]. A JSON null is "not set".
/// A value that is not a string naming one of the two modes is refused; the message
/// names `routing.rank_key`, the value and both names, as `route_search::resolve_effort`
/// does for the effort. There is no tool argument (PK9).
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
/// prune's clause (c), its final comparison and the keys it takes again, and the
/// result's listing.
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
/// they have one length; keys of two lengths are a bug of the caller (a `debug_assert`).
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
///   was not judged. A judged run the search released still has both. Both lists are of
///   the boards this call judged and kept: a run's tuned choice was made by the ranking
///   key, and the winner's keys are the ones taken after the prune.
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
