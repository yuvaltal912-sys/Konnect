//! The judge's length check (docs/LENGTH_TUNING_DESIGN.md, L8 to L10): KiCad
//! measures every checked net on a copy of the judged board, and Konnect compares
//! those lengths with the design intent's groups and pair skews.
//!
//! Stage 2 of Row LT holds only the data types the length plan
//! (`tools/route_lengths.rs`) builds and reports: what is checked, and what a check
//! found. Stage 3 adds the probe, the check itself and G4's lines.

// Stage 2 of Row LT builds these types; the judge uses them from Stage 3 on.
#![allow(dead_code)]

use crate::tools::route_lengths::Window;
use serde::{Deserialize, Serialize};

/// KiCad's rule keys of length and skew violations: G4 counts them, G2 does not
/// (L9).
pub(crate) const LENGTH_RULE_KEYS: [&str; 2] = ["length_out_of_range", "skew_out_of_range"];

/// A group the judge checks, by board net names.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheckedGroup {
    pub interface: String,
    pub name: String,
    pub nets: Vec<String>,
    pub window: Window,
}

/// A pair skew the judge checks, by board net names.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheckedPair {
    pub positive: String,
    pub negative: String,
    pub max_skew_mm: f64,
}

/// Everything one call checks (`route_lengths::LengthPlan::checks`).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LengthChecks {
    pub groups: Vec<CheckedGroup>,
    pub pairs: Vec<CheckedPair>,
}

impl LengthChecks {
    /// Every net to probe, each once, sorted.
    pub(crate) fn nets(&self) -> Vec<String> {
        let mut nets: Vec<String> = self
            .groups
            .iter()
            .flat_map(|group| group.nets.iter().cloned())
            .chain(
                self.pairs
                    .iter()
                    .flat_map(|pair| [pair.positive.clone(), pair.negative.clone()]),
            )
            .collect();
        nets.sort();
        nets.dedup();
        nets
    }

    /// True when nothing is checked (then the judge runs no probe).
    pub(crate) fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.pairs.is_empty()
    }
}

/// One net of a checked group, measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct NetLength {
    pub net: String,
    /// KiCad's length, mm; `None` when the probe gave no entry.
    pub kicad_mm: Option<f64>,
    /// The window this net had to meet, mm (`Window::range_mm`).
    pub low_mm: f64,
    pub high_mm: f64,
    /// How far outside the window, mm; 0 inside.
    pub miss_mm: f64,
}

/// One checked group, measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct GroupLengths {
    pub interface: String,
    pub name: String,
    /// The longest measured net, mm (skew mode's reference).
    pub longest_mm: Option<f64>,
    pub nets: Vec<NetLength>,
}

/// One checked pair, measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PairSkew {
    pub positive: String,
    pub negative: String,
    /// |L₊ − L₋|, mm; `None` when either net is unmeasured.
    pub skew_mm: Option<f64>,
    pub max_skew_mm: f64,
    pub miss_mm: f64,
}

/// What the length check found on one judged run (`Measures.lengths`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct LengthMeasures {
    pub groups: Vec<GroupLengths>,
    pub pairs: Vec<PairSkew>,
    /// Checked nets the probe gave no entry for.
    pub unmeasured: Vec<String>,
    /// The sum of every measured net's and pair's miss, mm.
    pub miss_mm: f64,
    /// The probe's own DRC time, seconds.
    pub probe_s: f64,
}
