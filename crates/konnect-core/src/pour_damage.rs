//! Row ZR, stage ZR5, part 1: the pour record (docs/POUR_RANK_DESIGN.md).
//!
//! Pure. What the routing did to the pours, measured against KiCad's own fill of the
//! board before it was routed (PK3). Recorded only: no gate and no entry of the rank key
//! reads it (PK1), and nothing here reads a file or starts a process.

use crate::zone_fill::ZoneLayerFill;
use serde::{Deserialize, Serialize};

/// One pour net's connected fill on one copper layer, over every zone of the net on that
/// layer. A piece KiCad saved as an island (`(island yes)`: kept by the zone's own island
/// rule, touching nothing of its net) is not fill here (PK3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct LayerFill {
    pub net: String,
    pub layer: String,
    /// The fill pieces that are not islands. 0: the net has a zone on this layer and no
    /// connected fill.
    pub pieces: usize,
    /// The largest of those pieces, mm²; 0 without one.
    pub largest_mm2: f64,
    /// The area of all of those pieces, mm².
    pub area_mm2: f64,
    /// The pieces KiCad saved as islands. They are in none of the three numbers above.
    pub islands: usize,
}

/// The layer fills of `pour_fills`' rows (`zone_fill::pour_fills`: one row per zone and
/// layer, each with its pieces' areas and, from this stage on, one island flag per piece
/// in the same order, `ZoneLayerFill::islands`). The rows of one net on one layer are
/// joined: the pieces that are not islands are counted together, the largest of them is
/// the largest of all the rows, their areas are added, and the islands are counted
/// apart. Ordered by net, then layer. A zone layer with no piece still gives its row,
/// with zeros. Zones of one net that overlap on a layer are not merged: the largest
/// piece is the largest saved piece of any of them.
pub(crate) fn layer_fills(fills: &[ZoneLayerFill]) -> Vec<LayerFill> {
    todo!()
}

/// One pour net's plane before and after routing, for the reader of a result (PK4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct NetPlane {
    pub net: String,
    /// The layer of the net's largest piece before routing; ties go to the first layer in
    /// `before`'s order. `None` when the net had no connected fill before routing.
    pub plane_layer: Option<String>,
    /// That piece's area, mm²: the net's plane. 0 with no fill before.
    pub plane_mm2: f64,
    /// The layer of what is kept; ties go to the first layer in `before`'s order. `None`
    /// when nothing is kept.
    pub kept_layer: Option<String>,
    /// The largest of, over the net's layers, the smaller of the largest piece after
    /// routing and the largest piece before on that layer, mm².
    pub kept_mm2: f64,
    /// `kept_mm2 / plane_mm2`, from 0 to 1; 1 when the plane is 0 (nothing to lose).
    pub kept_share: f64,
}

/// What the routing did to the pours, against KiCad's own fill of the board before it
/// was routed (PK3 to PK6), with the rows it was made from (PK18).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PourDamage {
    /// Layer fills that had a piece before routing and have none after (PK5).
    pub lost_fills: usize,
    /// The plane area that is no longer kept, over the area of all the nets' planes: a
    /// share from 0 to 1 (PK4). 0 when no net had fill before.
    pub plane_loss: f64,
    /// The fill area that is gone, over the fill area before: a share from 0 to 1 (PK6).
    /// 0 when no layer had fill before.
    pub area_loss: f64,
    /// One row per pour net of `before`, by name.
    pub nets: Vec<NetPlane>,
    /// One row per layer fill of `before`, in its order, holding the values after
    /// routing (zeros for a layer fill that is not in `after`).
    pub layers: Vec<LayerFill>,
}

/// The damage of `after` against `before` (both from [`layer_fills`]; `before` is the
/// judge's baseline, `after` the judged board):
/// - A layer fill is matched by net and layer. One that is in `before` and not in `after`
///   has no piece and no area after. One that is in `after` and not in `before` is not
///   read: the judged board is the baseline's board with a session written into it, so it
///   has the same zones.
/// - `lost_fills`: `before.pieces > 0` and no piece after.
/// - A net's plane is the largest `before.largest_mm2` over its layers. What it keeps is
///   the largest, over its layers, of `min(after.largest_mm2, before.largest_mm2)`: on a
///   layer, never more than that layer had (a piece that grew is not a gain).
/// - `plane_loss`: the sum over the nets of `plane - kept`, over the sum of the planes;
///   0 when that sum is 0. Weighted by area, so a small pour counts as a small part.
/// - `area_loss`: 1 minus the sum, over the layer fills with `before.area_mm2 > 0`, of
///   `min(after.area_mm2, before.area_mm2)`, over the sum of their `before.area_mm2`;
///   0 when there is none.
///
/// `after == before` gives 0, 0.0 and 0.0, whatever the zones look like. Every share is a
/// finite number from 0 to 1.
pub(crate) fn pour_damage(before: &[LayerFill], after: &[LayerFill]) -> PourDamage {
    todo!()
}

#[cfg(test)]
#[path = "pour_damage_tests.rs"]
mod tests;
