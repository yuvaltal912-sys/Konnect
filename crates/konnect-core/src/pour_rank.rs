//! Row ZR, stage ZR5, part 2: the pour key and its setting (docs/POUR_RANK_DESIGN.md).
//!
//! Pure. The pour key is today's rank key (`route_judge::rank_measures`) with four
//! entries added from part 1's pour record (`pour_damage`), and a setting says which of
//! the two keys ranks. Nothing here changes today's key (PK1), reads a file or starts a
//! process.
//!
//! The three steps and the places below are the owner's: part 2 was built before ZR3's
//! numbers with candidate values (2026-11-23), and on 2026-11-24, judged on the replay of
//! the recorded runs, he moved the ground track before the lengths in 5 % steps (PK19) and
//! set the plane-loss step at 2.5 % (PK20); the area-loss step stays 1 % after the vias.

use crate::route_judge::{Gates, Judged, Key, Measures};
use serde::Serialize;
use serde_json::{json, Value};

/// The step of the plane loss in the pour key (PK4, PK20): 2.5 % of the planes' area.
pub(crate) const PLANE_STEP: f64 = 0.025;

/// The step of the area loss in the pour key (PK6): 1 % of the pour copper.
pub(crate) const AREA_STEP: f64 = 0.01;

/// The step of the pour track in the pour key (PK19): 5 % of the pool's shortest signal
/// length.
pub(crate) const POUR_TRACK_STEP: f64 = 0.05;

/// The length of the pour key: today's thirteen entries and four more (PK2).
pub(crate) const POUR_KEY_LEN: usize = 17;

/// A run's pour key; the least is the best. Entries 0 to 6 are today's 0 to 6 (the four
/// gates, the intent's length miss, KiCad's own length violations, acute corners); 10 to
/// 13 are today's 7 to 10 (the signal and power length steps, right angles, vias off the
/// pour nets); 15 and 16 are today's 11 and 12 (minus copper quality, the route seconds).
/// The four others are [`POUR_FAULTS`] (7), [`PLANE_LOSS`] (8), [`POUR_TRACK`] (9) and
/// [`AREA_LOSS`] (14).
pub(crate) type PourKey = [i64; POUR_KEY_LEN];

/// Where each of today's thirteen entries (`route_judge::Key`, in its order) sits in the
/// pour key (PK2, PK19): 0 to 6, then 10 to 13, then 15 and 16. Inserting keeps their
/// order among themselves.
pub(crate) const TODAY_AT: [usize; crate::route_judge::KEY_LEN] =
    [0, 1, 2, 3, 4, 5, 6, 10, 11, 12, 13, 15, 16];

/// Entry 7: KiCad's `starved_thermal` violations on the judged board plus the damage's
/// `lost_fills` (PK5).
pub(crate) const POUR_FAULTS: usize = 7;

/// Entry 8: [`plane_step`] of the damage's `plane_loss` (PK4).
pub(crate) const PLANE_LOSS: usize = 8;

/// Entry 9: [`pour_track_step`] of the run's track length on pour nets (PK7, PK19): after
/// the plane-loss step, before the signal length step.
pub(crate) const POUR_TRACK: usize = 9;

/// Entry 14: [`area_step`] of the damage's `area_loss` (PK6).
pub(crate) const AREA_LOSS: usize = 14;

// ─── The pour key ────────────────────────────────────────────────────────────

/// [`PourKey`]'s entry 8 for a plane loss: `(plane_loss / PLANE_STEP).round()`, as the
/// length steps round (half away from zero). A loss that is not a finite number of 0 or
/// more gives `i64::MAX` (unmeasured sorts last).
pub(crate) fn plane_step(plane_loss: f64) -> i64 {
    share_step(plane_loss, PLANE_STEP)
}

/// [`PourKey`]'s entry 14 for an area loss: `(area_loss / AREA_STEP).round()`, with the
/// same rule for a loss that is not a finite number of 0 or more.
pub(crate) fn area_step(area_loss: f64) -> i64 {
    share_step(area_loss, AREA_STEP)
}

/// `(share / step).round()`, half away from zero; `i64::MAX` for a share that is not a
/// finite number of 0 or more.
fn share_step(share: f64, step: f64) -> i64 {
    if share.is_finite() && share >= 0.0 {
        (share / step).round() as i64
    } else {
        i64::MAX
    }
}

/// [`PourKey`]'s entry 9 (PK19): the track length on pour nets in units of
/// [`POUR_TRACK_STEP`] of the pool's shortest signal length:
/// `(pour_track_mm / shortest_signal_mm / POUR_TRACK_STEP).round()`, half away from zero.
/// 0 when the run has no track on a pour net, and 0 when the pool has no signal length to
/// scale by (`None`): a step with no unit cannot rank.
pub(crate) fn pour_track_step(pour_track_mm: f64, shortest_signal_mm: Option<f64>) -> i64 {
    match shortest_signal_mm {
        Some(shortest) if pour_track_mm > 0.0 => {
            (pour_track_mm / shortest / POUR_TRACK_STEP).round() as i64
        }
        _ => 0,
    }
}

/// The pour key of every run at once (the length steps need the pool), in `runs`' order.
/// `runs` is what `route_judge::rank_measures` takes: the measures, the gates, and
/// whether the run is in the pool.
/// - Entries 0 to 6, 10 to 13, 15 and 16 are the entries of
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
///   against (the runs in the pool that pass G1, else the pool, else every run;
///   `route_judge::shortest_length`, which both keys call), in its own step
///   ([`POUR_TRACK_STEP`], PK19).
///
/// With no pour net in the call the four entries are 0 for every run, so the pour key
/// orders the runs exactly as today's key does, ties included (PK2).
pub(crate) fn pour_keys(runs: &[(&Measures, &Gates, bool)]) -> Vec<PourKey> {
    let shortest_signal = crate::route_judge::shortest_length(runs, |m| m.signal_mm);
    crate::route_judge::rank_measures(runs)
        .into_iter()
        .zip(runs)
        .map(|(today, &(measures, _, _))| {
            let mut key: PourKey = [0; POUR_KEY_LEN];
            for (entry, &place) in TODAY_AT.iter().enumerate() {
                key[place] = today[entry];
            }
            let [faults, plane, area] = damage_entries(measures);
            key[POUR_FAULTS] = faults;
            key[PLANE_LOSS] = plane;
            key[AREA_LOSS] = area;
            key[POUR_TRACK] = pour_track_step(measures.pour_track_mm, shortest_signal);
            key
        })
        .collect()
}

/// Entries [`POUR_FAULTS`], [`PLANE_LOSS`] and [`AREA_LOSS`] of one run (PK8): 0 in all
/// three when the judge took no pour record; the record's numbers when it has a damage;
/// `i64::MAX` in all three otherwise.
fn damage_entries(measures: &Measures) -> [i64; 3] {
    match (&measures.pours, &measures.pours_error) {
        (None, None) => [0, 0, 0],
        (Some(pours), None) => match &pours.damage {
            Some(damage) => [
                count(pours.starved_thermals).saturating_add(count(damage.lost_fills)),
                plane_step(damage.plane_loss),
                area_step(damage.area_loss),
            ],
            None => [i64::MAX; 3],
        },
        _ => [i64::MAX; 3],
    }
}

/// A count as a key entry.
fn count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

// ─── The setting ─────────────────────────────────────────────────────────────

/// Which key ranks (PK9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RankMode {
    /// Today's key (`route_judge::rank_measures`), with `routing.rank_key: "routing"`.
    Routing,
    /// The pour key ([`pour_keys`]). The default ([`DEFAULT_RANK_MODE`], Row ZR, ZR6).
    Pours,
}

impl RankMode {
    /// `routing` or `pours`: the word of the user config and of a result.
    pub(crate) fn name(self) -> &'static str {
        match self {
            RankMode::Routing => "routing",
            RankMode::Pours => "pours",
        }
    }

    /// The mode a word names; `None` for any other text (the match is exact).
    pub(crate) fn parse(text: &str) -> Option<RankMode> {
        match text {
            "routing" => Some(RankMode::Routing),
            "pours" => Some(RankMode::Pours),
            _ => None,
        }
    }

    /// The other mode.
    pub(crate) fn other(self) -> RankMode {
        match self {
            RankMode::Routing => RankMode::Pours,
            RankMode::Pours => RankMode::Routing,
        }
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

/// The mode of a call with no `routing.rank_key` in the user config (Row ZR, ZR6: the
/// owner's fold of the switch into ZR6's PR, "כן, לעשות את שלושתם", 2026-11-24; the
/// switch itself waits for his yes on the PR text): the pour key.
pub(crate) const DEFAULT_RANK_MODE: RankMode = RankMode::Pours;

/// The mode of a call that reads the user config: `routing.rank_key` (`config` is that
/// value when the config has it), else [`DEFAULT_RANK_MODE`]. A JSON null is "not set".
/// A value that is not a string naming one of the two modes is refused; the message
/// names `routing.rank_key`, the value and both names, as `route_search::resolve_effort`
/// does for the effort. There is no tool argument (PK9).
pub(crate) fn resolve_rank_mode(
    config: Option<&Value>,
) -> std::result::Result<(RankMode, RankSource), String> {
    let Some(value) = config.filter(|value| !value.is_null()) else {
        return Ok((DEFAULT_RANK_MODE, RankSource::Default));
    };
    let source = "the user config's `routing.rank_key`";
    let Some(text) = value.as_str() else {
        return Err(format!(
            "{source} must be a string, one of routing or pours; it was {value}"
        ));
    };
    RankMode::parse(text)
        .map(|mode| (mode, RankSource::UserConfig))
        .ok_or_else(|| format!("{source} must be one of routing or pours; it was {text:?}"))
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
    match mode {
        RankMode::Routing => crate::route_judge::rank_measures(runs)
            .iter()
            .map(|key| key.to_vec())
            .collect(),
        RankMode::Pours => pour_keys(runs).iter().map(|key| key.to_vec()).collect(),
    }
}

/// [`ranking`] of judged runs that are all in the pool, as `route_judge::rank` is of
/// `rank_measures`.
pub(crate) fn ranking_of(mode: RankMode, judged: &[&Judged]) -> Vec<Vec<i64>> {
    let runs: Vec<(&Measures, &Gates, bool)> = judged
        .iter()
        .map(|run| (&run.measures, &run.gates, true))
        .collect();
    ranking(mode, &runs)
}

/// The positions at which `next` is above `now`, the last entry (the route seconds) left
/// out: what the prune's clause (c) refuses on. Both are keys of one [`ranking`] call, so
/// they have one length; keys of two lengths are a bug of the caller (a `debug_assert`).
pub(crate) fn entries_above(now: &[i64], next: &[i64]) -> Vec<usize> {
    debug_assert_eq!(
        now.len(),
        next.len(),
        "two keys of one ranking have one length"
    );
    now.iter()
        .zip(next)
        .take(now.len().saturating_sub(1))
        .enumerate()
        .filter(|(_, (now, next))| next > now)
        .map(|(position, _)| position)
        .collect()
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
/// - `same_winner`: whether that index is `winner`; `false` when there is no such index.
pub(crate) fn rank_report(
    mode: RankMode,
    source: RankSource,
    keys: &[Option<Key>],
    pour_keys: &[Option<PourKey>],
    winner: Option<usize>,
) -> Value {
    let other = mode.other();
    let other_keys: Vec<Option<Vec<i64>>> = match other {
        RankMode::Routing => keys.iter().map(|key| key.map(|key| key.to_vec())).collect(),
        RankMode::Pours => pour_keys
            .iter()
            .map(|key| key.map(|key| key.to_vec()))
            .collect(),
    };
    // The least key; on a tie the lower index (`min` of (key, index)).
    let other_winner = other_keys
        .iter()
        .enumerate()
        .filter_map(|(index, key)| key.as_ref().map(|key| (key, index)))
        .min()
        .map(|(_, index)| index);
    json!({
        "ranked_by": mode.name(),
        "source": source,
        "other_key": other.name(),
        "other_winner": other_winner,
        "same_winner": other_winner.is_some() && other_winner == winner,
    })
}

#[cfg(test)]
#[path = "pour_rank_tests.rs"]
pub(crate) mod tests;
