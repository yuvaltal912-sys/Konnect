//! Scratch copies of a board for the best-of-N judge: the project files a judged
//! copy needs, and a file-only writer that adds tracks, arcs and vias to a board's
//! text. Only for copies under a search folder: nothing here writes to a user's
//! board, and the live path (`apply_specctra_ses`) is not touched.

use crate::specctra_ses::SesImportPlan;
use anyhow::{bail, ensure, Context, Result};
use konnect_sexp::net::{net_ref_for_write, NetRef};
use konnect_sexp::{layers as board_layers, parse_sexp, SexpNode};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Copies into `into` (created if missing) the files a judged copy of `board`
/// needs, from the board's own folder: `<stem>.kicad_pro`, `<stem>.kicad_prl`,
/// `<stem>.kicad_dru`, `fp-lib-table`, `sym-lib-table` (each when present, as a
/// regular file), the `.konnect` folder (the design intent) and every `*.pretty`
/// folder, recursively. No schematic, no other board, no backups. Symbolic links
/// are refused, not followed. Returns the path the board's copy must be written to:
/// `into/<board file name>`. The board itself is not copied: the caller writes its
/// text.
pub(crate) fn copy_project(board: &Path, into: &Path) -> Result<PathBuf> {
    let file_name = board
        .file_name()
        .with_context(|| format!("'{}' names no board file", board.display()))?;
    let stem = board
        .file_stem()
        .and_then(|stem| stem.to_str())
        .with_context(|| format!("board '{}' has no UTF-8 file stem", board.display()))?;
    let folder = match board.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(into)
        .with_context(|| format!("create scratch folder '{}'", into.display()))?;
    // A copy onto itself would truncate the very files it reads.
    let source_dir = std::fs::canonicalize(folder)
        .with_context(|| format!("resolve board folder '{}'", folder.display()))?;
    let target_dir = std::fs::canonicalize(into)
        .with_context(|| format!("resolve scratch folder '{}'", into.display()))?;
    ensure!(
        source_dir != target_dir,
        "the scratch folder '{}' is the board's own folder",
        into.display()
    );

    let mut wanted: Vec<String> = ["kicad_pro", "kicad_prl", "kicad_dru"]
        .iter()
        .map(|extension| format!("{stem}.{extension}"))
        .collect();
    wanted.push("fp-lib-table".to_string());
    wanted.push("sym-lib-table".to_string());
    // Only the design intent: the rest of `.konnect` (baselines, checkpoints,
    // call logs) is the live project's own state.
    wanted.push(".konnect/design_intent.json".to_string());
    // Every `*.pretty` library folder, in name order so that a copy is repeatable.
    let mut pretty: Vec<String> = std::fs::read_dir(folder)
        .with_context(|| format!("read board folder '{}'", folder.display()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(".pretty"))
        .collect();
    pretty.sort();
    wanted.extend(pretty);

    for name in wanted {
        // A file inside a folder (`.konnect/design_intent.json`) is looked up through
        // the folder, so the folder must not be a link either.
        if let Some((folder_name, _)) = name.split_once('/') {
            let link = std::fs::symlink_metadata(folder.join(folder_name))
                .is_ok_and(|meta| meta.file_type().is_symlink());
            if link {
                bail!(
                    "'{}' is a symbolic link; the scratch copy does not follow links",
                    folder.join(folder_name).display()
                );
            }
        }
        let from = folder.join(&name);
        let Ok(kind) = std::fs::symlink_metadata(&from).map(|meta| meta.file_type()) else {
            continue; // not present
        };
        if kind.is_symlink() {
            bail!(
                "'{}' is a symbolic link; the scratch copy does not follow links",
                from.display()
            );
        }
        let to = into.join(&name);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create '{}'", parent.display()))?;
        }
        if kind.is_dir() {
            copy_folder(&from, &to)?;
        } else if kind.is_file() {
            std::fs::copy(&from, &to)
                .with_context(|| format!("copy '{}' to '{}'", from.display(), to.display()))?;
        }
        // Anything else (a socket, a device) is no project file.
    }
    Ok(into.join(file_name))
}

/// `from` and everything below it, copied to `to`. A symbolic link anywhere is
/// refused, not followed.
pub(crate) fn copy_folder(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).with_context(|| format!("create '{}'", to.display()))?;
    let mut entries: Vec<_> = std::fs::read_dir(from)
        .with_context(|| format!("read '{}'", from.display()))?
        .collect::<std::io::Result<_>>()
        .with_context(|| format!("read '{}'", from.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let source = entry.path();
        let kind = entry
            .file_type()
            .with_context(|| format!("inspect '{}'", source.display()))?;
        let target = to.join(entry.file_name());
        if kind.is_symlink() {
            bail!(
                "'{}' is a symbolic link; the scratch copy does not follow links",
                source.display()
            );
        } else if kind.is_dir() {
            copy_folder(&source, &target)?;
        } else if kind.is_file() {
            std::fs::copy(&source, &target).with_context(|| {
                format!("copy '{}' to '{}'", source.display(), target.display())
            })?;
        } else {
            bail!("'{}' is neither a file nor a folder", source.display());
        }
    }
    Ok(())
}

/// `board_text` with every track, arc and via of `plan` added before the closing
/// parenthesis of the root `(kicad_pcb …)`, one item a line block, tab-indented as
/// KiCad writes them:
///
/// ```text
/// \t(segment\n\t\t(start x1 y1)\n\t\t(end x2 y2)\n\t\t(width w)\n\t\t(layer "L")\n\t\t<net>\n\t\t(uuid "…")\n\t)
/// \t(arc\n\t\t(start …)\n\t\t(mid …)\n\t\t(end …)\n\t\t(width w)\n\t\t(layer "L")\n\t\t<net>\n\t\t(uuid "…")\n\t)
/// \t(via\n\t\t(at x y)\n\t\t(size s)\n\t\t(drill d)\n\t\t(layers "F.Cu" "B.Cu")\n\t\t<net>\n\t\t(uuid "…")\n\t)
/// ```
///
/// `<net>` is the board's own form, from `konnect_sexp::net::net_ref_for_write`:
/// `(net "NAME")` on a KiCad 10 board, `(net N)` on a legacy one. A net a legacy
/// board does not declare is refused (never net 0). A via spans the board's first
/// and last copper layers (the board's `layers` table, as `F.Cu` and `B.Cu` on a
/// two-layer board). Coordinates in mm with at most 6 decimals, no exponent.
/// Fresh v4 uuids. Names are written with `"` and `\` escaped. Nothing else in the
/// text changes. Refuses a text whose root is not `kicad_pcb`.
pub(crate) fn write_routing(board_text: &str, plan: &SesImportPlan) -> Result<String> {
    let tree = parse_root(board_text)?;
    let mut writer = ItemWriter::new(&tree);
    for track in &plan.tracks {
        writer.segment(
            &track.net_name,
            &track.layer,
            track.width_mm,
            [track.x1_mm, track.y1_mm],
            [track.x2_mm, track.y2_mm],
        )?;
    }
    for arc in &plan.arcs {
        writer.arc(
            &arc.net_name,
            &arc.layer,
            arc.width_mm,
            [arc.start_x_mm, arc.start_y_mm],
            [arc.mid_x_mm, arc.mid_y_mm],
            [arc.end_x_mm, arc.end_y_mm],
        )?;
    }
    for via in &plan.vias {
        writer.via(&via.net_name, via.x_mm, via.y_mm, via.size_mm, via.drill_mm)?;
    }
    insert_items(board_text, &writer.text)
}

/// The text's tree, which must be a `kicad_pcb`.
fn parse_root(board_text: &str) -> Result<SexpNode> {
    let tree = parse_sexp(board_text).context("parse the board text")?;
    ensure!(
        tree.head() == Some("kicad_pcb"),
        "the text is not a kicad_pcb board"
    );
    Ok(tree)
}

/// `board_text` with `items` just before the closing parenthesis of its root. Items
/// are whole lines; one is started on a new line when the root's last child does not
/// end its line.
fn insert_items(board_text: &str, items: &str) -> Result<String> {
    let close = root_close(board_text)?;
    let before = &board_text[..close];
    let mut out = String::with_capacity(board_text.len() + items.len() + 1);
    out.push_str(before);
    if !items.is_empty() && !before.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(items);
    out.push_str(&board_text[close..]);
    Ok(out)
}

/// The byte index of the parenthesis that closes the first `(` of the text, with
/// strings (`"…"`, backslash escapes) read as text.
fn root_close(text: &str) -> Result<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut opened = false;
    for (index, byte) in text.bytes().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'(' => {
                depth += 1;
                opened = true;
            }
            b')' => {
                ensure!(
                    opened && depth > 0,
                    "unbalanced parentheses in the board text"
                );
                depth -= 1;
                if depth == 0 {
                    return Ok(index);
                }
            }
            _ => {}
        }
    }
    bail!("the board text has no closing parenthesis for its root")
}

/// A coordinate or size in mm: at most 6 decimals, trailing zeros trimmed, no
/// exponent, `-0` as `0`.
fn num(value: f64) -> Result<String> {
    ensure!(value.is_finite(), "a coordinate or size is not finite");
    let mut text = format!("{value:.6}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" {
        text = "0".to_string();
    }
    Ok(text)
}

/// `"text"` with `\` and `"` escaped.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        if ch == '\\' || ch == '"' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// Builds the item text for one board: remembers how the board names its nets and
/// which layers its vias span.
struct ItemWriter<'a> {
    tree: &'a SexpNode,
    nets: BTreeMap<String, String>,
    span: Option<(String, String)>,
    text: String,
}

impl<'a> ItemWriter<'a> {
    fn new(tree: &'a SexpNode) -> Self {
        ItemWriter {
            tree,
            nets: BTreeMap::new(),
            span: None,
            text: String::new(),
        }
    }

    /// The `(net …)` node of `name`, in the board's own form.
    fn net(&mut self, name: &str) -> Result<String> {
        if let Some(node) = self.nets.get(name) {
            return Ok(node.clone());
        }
        // The empty name is the unconnected pseudo-net: never copper of a route.
        ensure!(!name.is_empty(), "an item has no net name");
        let node = match net_ref_for_write(self.tree, name) {
            Some(NetRef::ByName(name)) => format!("(net {})", quoted(&name)),
            Some(NetRef::ById { id, .. }) => {
                ensure!(
                    id != "0",
                    "net '{name}' resolves to net 0, the unconnected net"
                );
                format!("(net {id})")
            }
            None => bail!("the board does not declare net '{name}'"),
        };
        self.nets.insert(name.to_string(), node.clone());
        Ok(node)
    }

    /// The first and last copper layers of the board's `layers` table.
    fn via_span(&mut self) -> Result<(String, String)> {
        if let Some(span) = &self.span {
            return Ok(span.clone());
        }
        let stack = board_layers::layers(self.tree);
        let copper = board_layers::copper(&stack);
        ensure!(
            copper.len() >= 2,
            "the board's layer table lists {} copper layer(s); a via needs two",
            copper.len()
        );
        let span = (
            copper[0].name.clone(),
            copper[copper.len() - 1].name.clone(),
        );
        self.span = Some(span.clone());
        Ok(span)
    }

    fn segment(
        &mut self,
        net: &str,
        layer: &str,
        width: f64,
        start: [f64; 2],
        end: [f64; 2],
    ) -> Result<()> {
        let net = self.net(net)?;
        self.text.push_str(&format!(
            "\t(segment\n\t\t(start {} {})\n\t\t(end {} {})\n\t\t(width {})\n\t\t(layer {})\n\t\t{}\n\t\t(uuid \"{}\")\n\t)\n",
            num(start[0])?,
            num(start[1])?,
            num(end[0])?,
            num(end[1])?,
            num(width)?,
            quoted(layer),
            net,
            uuid::Uuid::new_v4()
        ));
        Ok(())
    }

    fn arc(
        &mut self,
        net: &str,
        layer: &str,
        width: f64,
        start: [f64; 2],
        mid: [f64; 2],
        end: [f64; 2],
    ) -> Result<()> {
        let net = self.net(net)?;
        self.text.push_str(&format!(
            "\t(arc\n\t\t(start {} {})\n\t\t(mid {} {})\n\t\t(end {} {})\n\t\t(width {})\n\t\t(layer {})\n\t\t{}\n\t\t(uuid \"{}\")\n\t)\n",
            num(start[0])?,
            num(start[1])?,
            num(mid[0])?,
            num(mid[1])?,
            num(end[0])?,
            num(end[1])?,
            num(width)?,
            quoted(layer),
            net,
            uuid::Uuid::new_v4()
        ));
        Ok(())
    }

    fn via(&mut self, net: &str, x: f64, y: f64, size: f64, drill: f64) -> Result<()> {
        let net = self.net(net)?;
        let (first, last) = self.via_span()?;
        self.text.push_str(&format!(
            "\t(via\n\t\t(at {} {})\n\t\t(size {})\n\t\t(drill {})\n\t\t(layers {} {})\n\t\t{}\n\t\t(uuid \"{}\")\n\t)\n",
            num(x)?,
            num(y)?,
            num(size)?,
            num(drill)?,
            quoted(&first),
            quoted(&last),
            net,
            uuid::Uuid::new_v4()
        ));
        Ok(())
    }
}

/// A track a stitching proposal creates.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedTrack {
    pub net: String,
    pub layer: String,
    pub width_mm: f64,
    pub x1_mm: f64,
    pub y1_mm: f64,
    pub x2_mm: f64,
    pub y2_mm: f64,
}

/// A via a stitching proposal creates.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedVia {
    pub net: String,
    pub x_mm: f64,
    pub y_mm: f64,
    pub size_mm: f64,
    pub drill_mm: f64,
}

/// What stitching proposals create.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Planned {
    pub tracks: Vec<PlannedTrack>,
    pub vias: Vec<PlannedVia>,
}

/// The tracks and vias of every proposal of a `plan_route_refinement` result, in
/// order, read from each proposal's `operations`: a `route_batch` operation's
/// `arguments.tracks` and `arguments.vias`, in the shapes `route_batch` accepts
/// (read the planner's code in `tools/route_refine.rs` for the exact field names).
/// Any other operation (such as `modify_trace`) is refused: stitching proposals
/// only create.
pub(crate) fn planned_items(plan: &Value) -> Result<Planned> {
    let proposals = plan
        .get("proposals")
        .and_then(Value::as_array)
        .context("the planner result has no 'proposals' list")?;
    let mut items = Planned::default();
    for (number, proposal) in proposals.iter().enumerate() {
        let operations = proposal
            .get("operations")
            .and_then(Value::as_array)
            .with_context(|| format!("proposal {number} has no 'operations' list"))?;
        for operation in operations {
            let tool = operation.get("tool").and_then(Value::as_str).unwrap_or("");
            ensure!(
                tool == "route_batch",
                "proposal {number} holds a '{tool}' operation; stitching proposals only create copper"
            );
            let arguments = operation
                .get("arguments")
                .with_context(|| format!("proposal {number}: a route_batch has no arguments"))?;
            for track in list(arguments, "tracks", number)? {
                read_track(track, number, &mut items)?;
            }
            for via in list(arguments, "vias", number)? {
                read_via(via, number, &mut items)?;
            }
        }
    }
    Ok(items)
}

/// `arguments[key]` as a list; absent is empty, anything else that is not a list is
/// refused.
fn list<'a>(arguments: &'a Value, key: &str, proposal: usize) -> Result<&'a [Value]> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(rows)) => Ok(rows.as_slice()),
        Some(_) => bail!("proposal {proposal}: route_batch '{key}' is not a list"),
    }
}

fn field_text<'a>(row: &'a Value, key: &str, proposal: usize) -> Result<&'a str> {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .with_context(|| format!("proposal {proposal}: an item has no '{key}'"))
}

fn field_number(row: &Value, key: &str, proposal: usize) -> Result<f64> {
    row.get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .with_context(|| format!("proposal {proposal}: an item has no number '{key}'"))
}

fn field_size(row: &Value, key: &str, proposal: usize) -> Result<f64> {
    let value = field_number(row, key, proposal)?;
    ensure!(value > 0.0, "proposal {proposal}: '{key}' is not positive");
    Ok(value)
}

/// One `route_batch` track: a polyline, written as one segment per leg.
fn read_track(track: &Value, proposal: usize, items: &mut Planned) -> Result<()> {
    let net = field_text(track, "net_name", proposal)?;
    let layer = field_text(track, "layer", proposal)?;
    let width = field_size(track, "width", proposal)?;
    let points = track
        .get("points")
        .and_then(Value::as_array)
        .with_context(|| format!("proposal {proposal}: a track has no 'points'"))?;
    ensure!(
        points.len() >= 2,
        "proposal {proposal}: a track holds fewer than two points"
    );
    let corners: Vec<(f64, f64)> = points
        .iter()
        .map(|point| {
            Ok((
                field_number(point, "x", proposal)?,
                field_number(point, "y", proposal)?,
            ))
        })
        .collect::<Result<_>>()?;
    for leg in corners.windows(2) {
        items.tracks.push(PlannedTrack {
            net: net.to_string(),
            layer: layer.to_string(),
            width_mm: width,
            x1_mm: leg[0].0,
            y1_mm: leg[0].1,
            x2_mm: leg[1].0,
            y2_mm: leg[1].1,
        });
    }
    Ok(())
}

/// One `route_batch` via; the planner always gives its drill and pad size.
fn read_via(via: &Value, proposal: usize, items: &mut Planned) -> Result<()> {
    items.vias.push(PlannedVia {
        net: field_text(via, "net_name", proposal)?.to_string(),
        x_mm: field_number(via, "x", proposal)?,
        y_mm: field_number(via, "y", proposal)?,
        size_mm: field_size(via, "pad_size", proposal)?,
        drill_mm: field_size(via, "drill", proposal)?,
    });
    Ok(())
}

/// `board_text` with the planned tracks and vias added, exactly as
/// [`write_routing`] writes its items.
pub(crate) fn write_planned(board_text: &str, items: &Planned) -> Result<String> {
    let tree = parse_root(board_text)?;
    let mut writer = ItemWriter::new(&tree);
    for track in &items.tracks {
        writer.segment(
            &track.net,
            &track.layer,
            track.width_mm,
            [track.x1_mm, track.y1_mm],
            [track.x2_mm, track.y2_mm],
        )?;
    }
    for via in &items.vias {
        writer.via(&via.net, via.x_mm, via.y_mm, via.size_mm, via.drill_mm)?;
    }
    insert_items(board_text, &writer.text)
}

#[cfg(test)]
#[path = "scratch_board_tests.rs"]
mod tests;
