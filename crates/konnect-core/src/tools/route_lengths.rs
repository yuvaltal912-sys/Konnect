//! The length groups of a fastroute call (docs/LENGTH_TUNING_DESIGN.md, Row LT).
//!
//! The groups come from the design intent alone (`interfaces[].length_groups`, and
//! `differential_pairs[].max_skew_mm` for a pair's own skew), never from KiCad's
//! custom rules, which KiCad's own DRC checks on every judged run (L3).
//!
//! Stage 1 of the row holds only what the pairs file needs: the share of a tolerance
//! fastroute is given, and the skew it is sent for a declared maximum. Stage 2 adds
//! the plan, the tune file and the refusals here.

/// The share of an intent tolerance fastroute is given (L5, L13): fastroute leaves
/// alone every net inside its own window, so the rest is the margin for the gap
/// between its measure and KiCad's.
pub(crate) const FASTROUTE_WINDOW_SHARE: f64 = 0.5;

/// The skew fastroute is given for a declared maximum (L13): `max_skew_mm` ×
/// [`FASTROUTE_WINDOW_SHARE`]. DP's pairs file writes it as `skew=`, and a trailing
/// pair group as its `tolerance=`.
pub(crate) fn fastroute_skew_mm(max_skew_mm: f64) -> f64 {
    max_skew_mm * FASTROUTE_WINDOW_SHARE
}

#[cfg(test)]
#[path = "route_lengths_tests.rs"]
mod tests;
