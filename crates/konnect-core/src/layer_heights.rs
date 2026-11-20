//! Copper layer heights for fastroute's `--layer-heights`
//! (docs/LENGTH_TUNING_DESIGN.md, L7), so that fastroute counts a via's length close
//! to how KiCad does when it tunes.
//!
//! KiCad's via length between copper layers A and B (`BOARD_STACKUP::
//! GetLayerDistance`) runs from A through B, both copper thicknesses included.
//! fastroute adds `height[B] − height[A]`. With height(top) = 0 and height(i) =
//! everything above layer i plus its own copper, a via from the top layer is exact
//! and a via between two lower layers is one copper thickness short. The judge
//! measures with KiCad itself (`length_probe.rs`), so this only moves fastroute's
//! aim.
//!
//! Read from the saved board's text, as `trace_current.rs` reads copper thickness:
//! the route call works on files, and IPC is not available there.

// Stage 1 of Row LT builds this module; the route call uses it from Stage 2 on.
#![allow(dead_code)]

use konnect_sexp::{parse_sexp, SexpNode};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// KiCad's default copper thickness, mm (`BOARD_STACKUP_ITEM::
/// GetCopperDefaultThickness`).
pub(crate) const DEFAULT_COPPER_MM: f64 = 0.035;
/// KiCad's default solder mask thickness, mm.
pub(crate) const DEFAULT_MASK_MM: f64 = 0.01;
/// KiCad's default board thickness when `(general (thickness …))` is missing, mm.
pub(crate) const DEFAULT_BOARD_MM: f64 = 1.6;

/// Where the heights came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeightSource {
    /// The board's own `(setup (stackup …))`.
    BoardStackup,
    /// KiCad's default stackup for a board that defines none
    /// (`BOARD_STACKUP::BuildDefaultStackupList`).
    KicadDefault,
}

impl HeightSource {
    /// The name the result carries.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::BoardStackup => "board_stackup",
            Self::KicadDefault => "kicad_default",
        }
    }
}

/// One copper or dielectric item of a stackup, top to bottom. Mask, silk and paste
/// are not items: KiCad's distance skips them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StackItem {
    /// The copper layer's name (`F.Cu`, `In1.Cu`, …); `None` for a dielectric.
    pub layer: Option<String>,
    /// The item's thickness, mm, every sublayer summed.
    pub thickness_mm: f64,
}

/// What the call sends as `--layer-heights`, and why.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LayerHeights {
    /// One height per DSN layer, in DSN order, mm; `None` when nothing is sent.
    pub heights_mm: Option<Vec<f64>>,
    pub source: Option<HeightSource>,
    /// Why nothing is sent (the project does not count via heights; a DSN layer is
    /// not in the stackup; the board does not parse), or what was assumed.
    pub note: Option<String>,
}

/// A height in mm rounded to the micrometre, without trailing zeros (`0`, `0.55`,
/// `1.065`).
fn micrometres(mm: f64) -> String {
    let text = format!("{mm:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

impl LayerHeights {
    /// The switch value, `0,0.55,1.065,1.58`: each height in mm, rounded to the
    /// micrometre, without trailing zeros; `None` without heights.
    pub(crate) fn switch_value(&self) -> Option<String> {
        let heights = self.heights_mm.as_ref()?;
        Some(
            heights
                .iter()
                .map(|height| micrometres(*height))
                .collect::<Vec<_>>()
                .join(","),
        )
    }

    /// The result's `length_matching.layer_heights`: `sent`, `source`
    /// (`board_stackup` or `kicad_default`), `heights_mm`, and `note` when set.
    pub(crate) fn json(&self) -> Value {
        let rounded: Option<Vec<f64>> = self.heights_mm.as_ref().map(|heights| {
            heights
                .iter()
                .map(|height| (height * 1000.0).round() / 1000.0)
                .collect()
        });
        let mut value = json!({
            "sent": self.heights_mm.is_some(),
            "source": self.source.map(HeightSource::name),
            "heights_mm": rounded,
        });
        if let Some(note) = &self.note {
            value["note"] = json!(note);
        }
        value
    }
}

/// Whether KiCad counts via heights in lengths: the project's
/// `board.design_settings.rules.use_height_for_length_calcs`; `true` when the project,
/// the key or the file is missing (KiCad's default, `m_UseHeightForLengthCalcs`).
pub(crate) fn use_height_for_length_calcs(project_text: Option<&str>) -> bool {
    project_text
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .and_then(|project| {
            project["board"]["design_settings"]["rules"]["use_height_for_length_calcs"].as_bool()
        })
        .unwrap_or(true)
}

/// A thickness that can be used: finite and above zero.
fn usable(thickness: f64) -> bool {
    thickness.is_finite() && thickness > 0.0
}

/// The copper and dielectric items of `(setup (stackup …))`, top to bottom, in file
/// order; each item's thickness is the sum of its `thickness` entries (a dielectric
/// with `addsublayer` has several). `None` when the board has no stackup, or a
/// copper or dielectric item has no positive finite thickness (then KiCad's default
/// is not used either: the caller sends no heights and says why).
pub(crate) fn stackup_items(board: &SexpNode) -> Option<Vec<StackItem>> {
    let stackup = board.find("setup")?.find("stackup")?;
    let mut items = Vec::new();
    for layer in stackup.find_all("layer") {
        let name = layer.get(1).and_then(SexpNode::as_str)?;
        let kind = layer.find_str("type").unwrap_or("").to_ascii_lowercase();
        let copper = kind == "copper";
        if !copper
            && ["mask", "silk", "paste"]
                .iter()
                .any(|skipped| kind.contains(skipped))
        {
            continue;
        }
        let mut thickness = 0.0;
        let mut any = false;
        for entry in layer.find_all("thickness") {
            let value = entry.get_f64(1)?;
            if !usable(value) {
                return None;
            }
            thickness += value;
            any = true;
        }
        if !any {
            return None;
        }
        items.push(StackItem {
            layer: copper.then(|| name.to_string()),
            thickness_mm: thickness,
        });
    }
    items
        .iter()
        .any(|item| item.layer.is_some())
        .then_some(items)
}

/// A copper layer's place in the stack: F.Cu first, the inner layers by number, B.Cu
/// last; `None` for a name that is not a copper layer.
fn copper_rank(name: &str) -> Option<usize> {
    match name {
        "F.Cu" => Some(0),
        "B.Cu" => Some(usize::MAX),
        _ => name
            .strip_prefix("In")
            .and_then(|rest| rest.strip_suffix(".Cu"))
            .and_then(|number| number.parse::<usize>().ok())
            .filter(|number| *number >= 1),
    }
}

/// KiCad's default stackup for `board`: its copper layers from `(layers …)` in
/// stack order (F.Cu, In1.Cu, …, B.Cu), each 35 µm; between each two, a dielectric
/// of `(board thickness − 35 µm × copper count − 10 µm × masks) / (copper count −
/// 1)`, masks counted from `F.Mask` and `B.Mask` in `(layers …)`, board thickness
/// from `(general (thickness …))` else 1.6 mm. An error when the board has no
/// copper layer or the dielectric would not be positive.
pub(crate) fn default_stackup(board: &SexpNode) -> anyhow::Result<Vec<StackItem>> {
    let mut copper: Vec<(usize, String)> = Vec::new();
    let mut masks = 0usize;
    if let Some(layers) = board.find("layers") {
        for entry in layers.children().unwrap_or(&[]).iter().skip(1) {
            let Some(name) = entry.get(1).and_then(SexpNode::as_str) else {
                continue;
            };
            if let Some(rank) = copper_rank(name) {
                copper.push((rank, name.to_string()));
            } else if name == "F.Mask" || name == "B.Mask" {
                masks += 1;
            }
        }
    }
    if copper.is_empty() {
        anyhow::bail!("the board lists no copper layer");
    }
    copper.sort();
    copper.dedup();
    let board_mm = board
        .find("general")
        .and_then(|general| general.find_f64("thickness"))
        .unwrap_or(DEFAULT_BOARD_MM);
    let count = copper.len();
    let mut items = Vec::new();
    let dielectric = if count > 1 {
        let thickness =
            (board_mm - DEFAULT_COPPER_MM * count as f64 - DEFAULT_MASK_MM * masks as f64)
                / (count - 1) as f64;
        if !usable(thickness) {
            anyhow::bail!(
                "a board {board_mm} mm thick leaves no room for dielectric between {count} \
                 copper layers in KiCad's default stackup"
            );
        }
        Some(thickness)
    } else {
        None
    };
    for (index, (_, name)) in copper.into_iter().enumerate() {
        if index > 0 {
            items.push(StackItem {
                layer: None,
                thickness_mm: dielectric.unwrap_or_default(),
            });
        }
        items.push(StackItem {
            layer: Some(name),
            thickness_mm: DEFAULT_COPPER_MM,
        });
    }
    Ok(items)
}

/// Each copper layer's height, mm (L7): the top copper layer 0, every other the
/// sum of the items above it plus its own thickness.
pub(crate) fn copper_heights_mm(items: &[StackItem]) -> BTreeMap<String, f64> {
    let mut heights = BTreeMap::new();
    let mut above = 0.0;
    let mut started = false;
    for item in items {
        if let Some(name) = &item.layer {
            if started {
                heights.insert(name.clone(), above + item.thickness_mm);
            } else {
                // The top copper layer is the origin.
                heights.insert(name.clone(), 0.0);
                started = true;
            }
        }
        if started {
            above += item.thickness_mm;
        }
    }
    heights
}

/// KiCad's exact via distance between two copper layers of `items`, mm: from the
/// upper one through the lower one, both copper thicknesses included; 0 for the same
/// layer; `None` when either is not a copper item. For the tests and the
/// verification's residual report (how far fastroute's via count is from KiCad's).
pub(crate) fn kicad_layer_distance_mm(
    items: &[StackItem],
    first: &str,
    second: &str,
) -> Option<f64> {
    let position = |name: &str| {
        items
            .iter()
            .position(|item| item.layer.as_deref() == Some(name))
    };
    let (a, b) = (position(first)?, position(second)?);
    let (upper, lower) = (a.min(b), a.max(b));
    if upper == lower {
        return Some(0.0);
    }
    Some(
        items[upper..=lower]
            .iter()
            .map(|item| item.thickness_mm)
            .sum(),
    )
}

/// The heights of a call. Nothing is sent, with a note, when the project does not
/// count via heights, the board does not parse, or a DSN layer is not a copper
/// layer of the stackup used. The stackup is the board's own, else KiCad's
/// default (with a note saying so). The result is in `dsn_layers` order
/// (`specctra::dsn_layer_names`).
pub(crate) fn layer_heights(
    board_text: &str,
    project_text: Option<&str>,
    dsn_layers: &[String],
) -> LayerHeights {
    if !use_height_for_length_calcs(project_text) {
        return LayerHeights {
            note: Some(
                "the project does not count via heights in lengths \
                 (use_height_for_length_calcs is off), so no layer heights are sent"
                    .to_string(),
            ),
            ..LayerHeights::default()
        };
    }
    let board = match parse_sexp(board_text) {
        Ok(board) => board,
        Err(error) => {
            return LayerHeights {
                note: Some(format!(
                    "the board does not parse ({error}), so no layer heights are sent"
                )),
                ..LayerHeights::default()
            }
        }
    };
    let has_stackup = board
        .find("setup")
        .and_then(|setup| setup.find("stackup"))
        .is_some();
    let (items, source, mut note) = match stackup_items(&board) {
        Some(items) => (items, HeightSource::BoardStackup, None),
        None if has_stackup => {
            return LayerHeights {
                note: Some(
                    "the board's stackup has no usable thickness for a copper or dielectric \
                     item (or no copper layer), so no layer heights are sent"
                        .to_string(),
                ),
                ..LayerHeights::default()
            }
        }
        None => match default_stackup(&board) {
            Ok(items) => (
                items,
                HeightSource::KicadDefault,
                Some(
                    "the board defines no stackup; KiCad's default stackup was assumed".to_string(),
                ),
            ),
            Err(error) => {
                return LayerHeights {
                    note: Some(format!(
                        "the board defines no stackup and KiCad's default cannot be built \
                         ({error}), so no layer heights are sent"
                    )),
                    ..LayerHeights::default()
                }
            }
        },
    };
    let by_layer = copper_heights_mm(&items);
    let mut heights = Vec::with_capacity(dsn_layers.len());
    for layer in dsn_layers {
        match by_layer.get(layer) {
            Some(height) => heights.push(*height),
            None => {
                return LayerHeights {
                    source: Some(source),
                    note: Some(format!(
                        "the DSN layer '{layer}' is not a copper layer of the stackup used, so \
                         no layer heights are sent"
                    )),
                    ..LayerHeights::default()
                }
            }
        }
    }
    if heights.is_empty() {
        note = Some("the DSN lists no layer, so no layer heights are sent".to_string());
        return LayerHeights {
            source: Some(source),
            note,
            ..LayerHeights::default()
        };
    }
    LayerHeights {
        heights_mm: Some(heights),
        source: Some(source),
        note,
    }
}

#[cfg(test)]
#[path = "layer_heights_tests.rs"]
mod tests;
