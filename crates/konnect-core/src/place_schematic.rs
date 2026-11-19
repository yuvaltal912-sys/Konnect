//! The schematic-neighbour measure of the placement search (design D23): does a candidate
//! keep the parts that the schematic draws next to each other near each other on the
//! board, compared with the owner's placement? It is reported, never ranked.
//!
//! The neighbourhoods are fixed once, from the board's own schematic project: each
//! measured part's `NEIGHBOURS` nearest parts on the same sheet instance. A candidate is
//! then measured against those same neighbourhoods, on its board text.

// Only `place_search` (itself used by the bench alone) reaches this module.
#![allow(dead_code)]

use konnect_sexp::board::{footprint_courtyards, footprints};
use konnect_sexp::parser::SexpNode;
use konnect_sexp::schematic::extract_symbol_instances;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How many nearest parts of the same sheet instance make a part's neighbourhood.
pub(crate) const NEIGHBOURS: usize = 3;

/// A part is pulled away when its mean distance to its neighbours is more than this factor
/// times the input's ...
pub(crate) const PULLED_FACTOR: f64 = 2.0;

/// ... and more than this many millimetres above it. Both must hold.
pub(crate) const PULLED_MM: f64 = 10.0;

/// The deepest sheet nesting read; KiCad's own limit is far below it.
const MAX_DEPTH: usize = 32;

// ─── The schematic side ──────────────────────────────────────────────────────

/// One placed symbol (one unit of a part) of a sheet instance.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SheetSymbol {
    /// The symbol's own `(uuid …)`; it is what the last component of a footprint's path names.
    pub uuid: Option<String>,
    /// Its reference on this sheet instance: the `(instances … (reference …))` entry of the
    /// sheet path, else the `Reference` property.
    pub reference: String,
    /// Its `(at x y)`, in schematic millimetres.
    pub at: (f64, f64),
}

/// One sheet instance: the root, or a `(sheet …)` entry (so a file used twice is two).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SheetInstance {
    /// The sheet uuids below the root, each as `/<uuid>`: `""` for the root. This is the
    /// part of a footprint's `(path …)` before the symbol's uuid.
    pub chain: String,
    pub symbols: Vec<SheetSymbol>,
}

/// `uuid -> reference` of the symbols of `tree` on the sheet path `instance_path`
/// (`/<root uuid><chain>`), from each symbol's `(instances (project … (path … (reference …))))`.
fn instance_references(tree: &SexpNode, instance_path: &str) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    for node in tree.find_all("symbol") {
        if node.find("lib_id").is_none() {
            continue;
        }
        let Some(uuid) = node.find_str("uuid") else {
            continue;
        };
        let Some(instances) = node.find("instances") else {
            continue;
        };
        for project in instances.find_all("project") {
            for path in project.find_all("path") {
                if path.get(1).and_then(SexpNode::as_str) == Some(instance_path) {
                    if let Some(reference) = path.find_str("reference") {
                        found.insert(uuid.to_string(), reference.to_string());
                    }
                }
            }
        }
    }
    found
}

/// Reads the sheet instances below the root schematic `root`: the root, then each `(sheet …)`
/// followed through its `uuid` and its `Sheetfile` property (relative to the including file's
/// folder, and never outside the root's folder). Errs, with a reason, on a file that cannot
/// be read, a sheet without a uuid or a file, a file that includes itself, or a path that
/// leaves the root's folder.
pub(crate) fn read_sheets(root: &Path) -> Result<Vec<SheetInstance>, String> {
    let base = root
        .parent()
        .and_then(|folder| std::fs::canonicalize(folder).ok())
        .ok_or_else(|| format!("the folder of '{}' cannot be read", root.display()))?;
    let tree = read_tree(root)?;
    let root_uuid = tree
        .find_str("uuid")
        .ok_or_else(|| format!("the schematic '{}' has no uuid", root.display()))?
        .to_string();
    let mut out = Vec::new();
    let mut ancestors = vec![std::fs::canonicalize(root)
        .map_err(|error| format!("'{}' cannot be resolved: {error}", root.display()))?];
    collect(&tree, root, "", &root_uuid, &base, &mut ancestors, &mut out)?;
    Ok(out)
}

/// Reads and parses one schematic file. Plain reads, not `schematic::read_schematic`: that
/// takes KiCad-style document locks in Konnect's state folder, and the search touches no
/// file but its own.
fn read_tree(path: &Path) -> Result<SexpNode, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("the schematic '{}' cannot be read: {error}", path.display()))?;
    konnect_sexp::parse_sexp(&text)
        .map_err(|error| format!("the schematic '{}' does not parse: {error}", path.display()))
}

fn property<'a>(node: &'a SexpNode, name: &str) -> Option<&'a str> {
    node.find_all("property")
        .into_iter()
        .find(|p| p.get(1).and_then(SexpNode::as_str) == Some(name))
        .and_then(|p| p.get(2))
        .and_then(SexpNode::as_str)
}

fn collect(
    tree: &SexpNode,
    file: &Path,
    chain: &str,
    root_uuid: &str,
    base: &Path,
    ancestors: &mut Vec<PathBuf>,
    out: &mut Vec<SheetInstance>,
) -> Result<(), String> {
    if ancestors.len() > MAX_DEPTH {
        return Err(format!(
            "the sheets nest deeper than {MAX_DEPTH} levels at '{}'",
            file.display()
        ));
    }
    let references = instance_references(tree, &format!("/{root_uuid}{chain}"));
    let symbols = extract_symbol_instances(tree)
        .into_iter()
        .map(|symbol| SheetSymbol {
            reference: symbol
                .uuid
                .as_ref()
                .and_then(|uuid| references.get(uuid))
                .cloned()
                .unwrap_or(symbol.reference),
            uuid: symbol.uuid,
            at: (symbol.x, symbol.y),
        })
        .collect();
    out.push(SheetInstance {
        chain: chain.to_string(),
        symbols,
    });
    for sheet in tree.find_all("sheet") {
        let uuid = sheet
            .find_str("uuid")
            .ok_or_else(|| format!("a sheet of '{}' has no uuid", file.display()))?;
        let name = property(sheet, "Sheetfile")
            .ok_or_else(|| format!("sheet {uuid} of '{}' has no Sheetfile", file.display()))?;
        let path = file.parent().unwrap_or(Path::new("")).join(name);
        let resolved = std::fs::canonicalize(&path).map_err(|error| {
            format!(
                "the sheet file '{}' cannot be resolved: {error}",
                path.display()
            )
        })?;
        if !resolved.starts_with(base) {
            return Err(format!(
                "the sheet file '{}' lies outside the schematic's folder",
                path.display()
            ));
        }
        if ancestors.contains(&resolved) {
            return Err(format!(
                "the sheet file '{}' includes itself",
                path.display()
            ));
        }
        let child = read_tree(&path)?;
        ancestors.push(resolved);
        let result = collect(
            &child,
            &path,
            &format!("{chain}/{uuid}"),
            root_uuid,
            base,
            ancestors,
            out,
        );
        ancestors.pop();
        result?;
    }
    Ok(())
}

// ─── The neighbourhoods ──────────────────────────────────────────────────────

/// The neighbourhoods of a board against a schematic, fixed once.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Neighbourhoods {
    /// For each measured part that has at least one neighbour: its nearest parts of the same
    /// sheet instance, nearest first, ties by reference (at most `NEIGHBOURS`).
    pub neighbours: BTreeMap<String, Vec<String>>,
    /// Footprints with a pad and a matched symbol.
    pub measured: usize,
    /// Footprints with a pad and no matched symbol (pad-less ones are skipped uncounted).
    pub unmatched: usize,
    /// Measured parts with no neighbour: alone on their sheet instance, so not in the mean.
    pub alone: usize,
}

/// The sheet instance and the schematic points of the part that footprint `node`
/// (reference `reference`) stands for, or `None`. The footprint's `(path "/<sheets>/<uuid>")`
/// names its sheet instance and its symbol; where no symbol of that instance has that uuid
/// (or the path names no known instance), the part is matched by its reference on the
/// instance: the instance the path names, else the only instance that has the reference.
/// Every unit of the part on the instance gives a point.
fn match_part(
    node: &SexpNode,
    reference: &str,
    sheets: &[SheetInstance],
) -> Option<(usize, Vec<(f64, f64)>)> {
    let path = node.find_str("path").unwrap_or("");
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let named = parts.split_last().and_then(|(uuid, sheet_uuids)| {
        let chain: String = sheet_uuids.iter().map(|s| format!("/{s}")).collect();
        let at = sheets.iter().position(|sheet| sheet.chain == chain)?;
        Some((at, *uuid))
    });
    let (at, reference_on_sheet) = match named {
        Some((at, uuid)) => {
            let by_uuid = sheets[at]
                .symbols
                .iter()
                .find(|symbol| symbol.uuid.as_deref() == Some(uuid))
                .map(|symbol| symbol.reference.clone());
            let found = by_uuid.or_else(|| {
                sheets[at]
                    .symbols
                    .iter()
                    .any(|symbol| symbol.reference == reference)
                    .then(|| reference.to_string())
            })?;
            (at, found)
        }
        None => {
            let mut holders = sheets.iter().enumerate().filter(|(_, sheet)| {
                sheet
                    .symbols
                    .iter()
                    .any(|symbol| symbol.reference == reference)
            });
            let (at, _) = holders.next()?;
            if holders.next().is_some() {
                return None;
            }
            (at, reference.to_string())
        }
    };
    let points: Vec<(f64, f64)> = sheets[at]
        .symbols
        .iter()
        .filter(|symbol| symbol.reference == reference_on_sheet)
        .map(|symbol| symbol.at)
        .collect();
    (!points.is_empty()).then_some((at, points))
}

/// The least distance between a point of `a` and a point of `b`.
fn least_distance(a: &[(f64, f64)], b: &[(f64, f64)]) -> f64 {
    a.iter()
        .flat_map(|p| b.iter().map(move |q| (p.0 - q.0).hypot(p.1 - q.1)))
        .fold(f64::INFINITY, f64::min)
}

/// The neighbourhoods of `board` (the input board's tree) against `sheets`. Errs when the
/// courtyard scan skipped a footprint (the centres would not line up with the footprints).
pub(crate) fn neighbourhoods(
    board: &SexpNode,
    sheets: &[SheetInstance],
) -> Result<Neighbourhoods, String> {
    let nodes = footprints(board);
    let scan = footprint_courtyards(board);
    if scan.skipped > 0 || scan.items.len() != nodes.len() {
        return Err(format!(
            "the courtyard scan skipped {} footprint(s)",
            scan.skipped
        ));
    }
    let mut unmatched = 0usize;
    // (reference, sheet instance, schematic points)
    let mut parts: Vec<(String, usize, Vec<(f64, f64)>)> = Vec::new();
    for (node, courtyard) in nodes.iter().zip(&scan.items) {
        if node.find_all("pad").is_empty() {
            continue;
        }
        let reference = courtyard.reference.as_deref().filter(|r| !r.is_empty());
        match reference.and_then(|r| match_part(node, r, sheets).map(|m| (r, m))) {
            Some((reference, (sheet, points))) => {
                parts.push((reference.to_string(), sheet, points));
            }
            None => unmatched += 1,
        }
    }
    let mut neighbours = BTreeMap::new();
    for (reference, sheet, points) in &parts {
        let mut others: Vec<(f64, &str)> = parts
            .iter()
            .filter(|(other, other_sheet, _)| other_sheet == sheet && other != reference)
            .map(|(other, _, other_points)| (least_distance(points, other_points), other.as_str()))
            .collect();
        others.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(b.1)));
        others.truncate(NEIGHBOURS);
        if !others.is_empty() {
            neighbours.insert(
                reference.clone(),
                others.into_iter().map(|(_, r)| r.to_string()).collect(),
            );
        }
    }
    Ok(Neighbourhoods {
        measured: parts.len(),
        alone: parts.len() - neighbours.len(),
        neighbours,
        unmatched,
    })
}

// ─── The board side ──────────────────────────────────────────────────────────

/// For each part of `hoods`, the mean board distance to its neighbours: between courtyard
/// bbox centres, on `board`'s own text. Errs when a part or a neighbour is not on the board
/// or the courtyard scan skipped a footprint.
pub(crate) fn part_distances(
    board: &SexpNode,
    hoods: &Neighbourhoods,
) -> Result<BTreeMap<String, f64>, String> {
    let scan = footprint_courtyards(board);
    if scan.skipped > 0 {
        return Err(format!(
            "the courtyard scan skipped {} footprint(s)",
            scan.skipped
        ));
    }
    let centres: BTreeMap<&str, (f64, f64)> = scan
        .items
        .iter()
        .filter_map(|c| {
            let (x0, y0, x1, y1) = c.bbox;
            Some((c.reference.as_deref()?, ((x0 + x1) / 2.0, (y0 + y1) / 2.0)))
        })
        .collect();
    let mut own = BTreeMap::new();
    for (part, near) in &hoods.neighbours {
        let from = centres
            .get(part.as_str())
            .ok_or_else(|| "a measured part is not on the board".to_string())?;
        let mut sum = 0.0;
        for other in near {
            let to = centres
                .get(other.as_str())
                .ok_or_else(|| "a neighbour is not on the board".to_string())?;
            sum += (from.0 - to.0).hypot(from.1 - to.1);
        }
        own.insert(part.clone(), sum / near.len() as f64);
    }
    Ok(own)
}

/// Whether a part is pulled away: its mean distance to its neighbours is more than
/// `PULLED_FACTOR` times the input's and more than `PULLED_MM` above it.
pub(crate) fn is_pulled_away(input_mm: f64, own_mm: f64) -> bool {
    own_mm > PULLED_FACTOR * input_mm && own_mm - input_mm > PULLED_MM
}

/// The measure of one candidate, as `search.json` records it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SchematicMeasure {
    /// The mean over the parts that have a neighbour of their mean neighbour distance.
    pub mean_mm: f64,
    /// `mean_mm` over the input board's; `None` when the input's is 0.
    pub ratio_to_input: Option<f64>,
    /// Parts with a mean neighbour distance both more than twice the input's and more than
    /// 10 mm above it.
    pub parts_pulled_away: usize,
    /// Footprints with a pad and a matched symbol.
    pub parts_measured: usize,
    /// Footprints with a pad and no matched symbol.
    pub parts_unmatched: usize,
    /// Measured parts with no neighbour (not in the mean).
    pub parts_alone: usize,
}

/// The fixed side of the measure: the neighbourhoods and the input board's own distances.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SchematicBaseline {
    pub hoods: Neighbourhoods,
    pub input_own: BTreeMap<String, f64>,
    pub input_mean: f64,
}

fn mean(values: &BTreeMap<String, f64>) -> f64 {
    values.values().sum::<f64>() / values.len() as f64
}

impl SchematicBaseline {
    /// The baseline of the input board `board_path` (parsed as `board`): its project's root
    /// schematic is the `.kicad_sch` with the board's file stem in the same folder. Errs with
    /// the reason when the schematic cannot be read or no part has a neighbour.
    pub(crate) fn read(board_path: &Path, board: &SexpNode) -> Result<Self, String> {
        let sheets = read_sheets(&board_path.with_extension("kicad_sch"))?;
        Self::from_sheets(&sheets, board)
    }

    pub(crate) fn from_sheets(sheets: &[SheetInstance], board: &SexpNode) -> Result<Self, String> {
        let hoods = neighbourhoods(board, sheets)?;
        if hoods.neighbours.is_empty() {
            return Err(format!(
                "no part of the board has a schematic neighbour ({} measured, {} unmatched)",
                hoods.measured, hoods.unmatched
            ));
        }
        let input_own = part_distances(board, &hoods)?;
        Ok(SchematicBaseline {
            input_mean: mean(&input_own),
            input_own,
            hoods,
        })
    }

    /// The measure of a candidate's board against the fixed neighbourhoods.
    pub(crate) fn measure(&self, board: &SexpNode) -> Result<SchematicMeasure, String> {
        let own = part_distances(board, &self.hoods)?;
        let mean_mm = mean(&own);
        let pulled = own
            .iter()
            .filter(|(part, mm)| {
                self.input_own
                    .get(*part)
                    .is_some_and(|input| is_pulled_away(*input, **mm))
            })
            .count();
        Ok(SchematicMeasure {
            mean_mm,
            ratio_to_input: (self.input_mean > 0.0).then(|| mean_mm / self.input_mean),
            parts_pulled_away: pulled,
            parts_measured: self.hoods.measured,
            parts_unmatched: self.hoods.unmatched,
            parts_alone: self.hoods.alone,
        })
    }
}

#[cfg(test)]
#[path = "place_schematic_tests.rs"]
mod tests;
