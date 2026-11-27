//! `delete_all_routing`: remove every track, arc and via from the live board in one
//! KiCad undo step, after a preview whose revision the apply must name (Row DR).
//!
//! The plan is read from the board text KiCad holds (`SaveDocumentToString`), the same
//! text `export_specctra_dsn` reads, so "locked" means here exactly what the export
//! accepts as fixed wiring: `(locked yes)`. Only top-level `segment`, `arc` and `via`
//! items are routing; an `(arc …)` inside a zone or polygon outline is geometry, not a
//! track. Zones, footprints, graphics and everything else are never sent to KiCad for
//! deletion, and the read-back proves they are unchanged, apart from what KiCad itself
//! updates with the tracks it belongs to (teardrop zones, tuning patterns, groups of
//! routing only), which is counted before and after and reported.
use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::CallToolResult;
use crate::tool;
use crate::tools::pcb_board::{attempt_ipc_write, BoardWrite};
use crate::tools::{get_path, invalid_arg, ToolContext, ToolDef};
use anyhow::{Context, Result};
use konnect_sexp::SexpNode;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Nets listed by name in a response; the rest are counted in `nets_total`.
const NETS_LISTED: usize = 100;
/// Item identities quoted in a read-back failure.
const IDS_QUOTED: usize = 20;

pub(super) fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "delete_all_routing",
        "Remove every track, arc and via from the live board in one KiCad undo step, for \
         example before export_specctra_dsn, which refuses a board that holds unlocked \
         routing. dry_run (the default) changes nothing: it counts what would go by kind, \
         net and layer, what is kept, and returns plan_revision. The apply \
         (dry_run=false) needs that expected_plan_revision and refuses when the board \
         changed in any way since the preview. Locked tracks, arcs and vias are kept \
         unless include_locked is true. Only tracks, arcs and vias are sent to KiCad for \
         deletion: zones, footprints, graphics and text never are, but KiCad itself may \
         remove the teardrop zones, tuning patterns and routing-only groups that belong \
         to deleted tracks (counted in linked_objects). The apply reads the board back \
         and reports what was removed, what was kept, what KiCad updated, and any other \
         object that changed. The board is not \
         saved: call save_project before an export. One Ctrl+Z in KiCad undoes the whole \
         delete. Live KiCad only, never a file edit.",
        json!({
            "type": "object",
            "properties": {
                "board": { "type": "string", "description": "The .kicad_pcb open in KiCad's PCB editor" },
                "dry_run": { "type": "boolean", "default": true, "description": "Preview only (default). false deletes, and needs expected_plan_revision" },
                "expected_plan_revision": { "type": "string", "description": "plan_revision from the preview, with the same include_locked" },
                "include_locked": { "type": "boolean", "default": false, "description": "Also delete locked tracks, arcs and vias. Off by default: export_specctra_dsn keeps locked straight tracks and locked through vias as fixed wiring" }
            },
            "required": ["board"],
            "additionalProperties": false
        }),
        |args, ctx| async move { handle_delete_all_routing(args, ctx).await }
    )
    .with_board_access(crate::tools::BoardAccess::LiveOnly)]
}

// ─── The plan (pure) ─────────────────────────────────────────────────────────

/// What a routing item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Track,
    Arc,
    Via,
}

impl Kind {
    fn of(head: &str) -> Option<Self> {
        match head {
            "segment" => Some(Self::Track),
            "arc" => Some(Self::Arc),
            "via" => Some(Self::Via),
            _ => None,
        }
    }
}

/// One top-level track, arc or via of the board text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RoutingItem {
    pub kind: Kind,
    /// The item's KIID, or `None` when the text carries none (it cannot be deleted by id).
    pub uuid: Option<String>,
    pub net: String,
    /// A track's or arc's layer; a via's span, its two outer layers joined by `-`.
    pub layer: String,
    /// `(locked yes)`, or a bare `locked` atom in an older file. The Specctra export
    /// reads only the first; [`Self::export_refusal`] names the second.
    pub locked: bool,
    /// A member of a locked group, directly or through nested groups. KiCad treats it as
    /// locked; the text marks only the group, so the Specctra export reads it as unlocked.
    pub in_locked_group: bool,
    /// Why `export_specctra_dsn` would refuse this item even when it is locked (read only
    /// for kept items).
    pub export_refusal: Option<&'static str>,
}

impl RoutingItem {
    /// Kept unless `include_locked`: locked itself, or held by a locked group.
    fn held(&self) -> bool {
        self.locked || self.in_locked_group
    }
}

/// Tracks, arcs and vias, counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub tracks: usize,
    pub arcs: usize,
    pub vias: usize,
}

impl Counts {
    fn add(&mut self, kind: Kind) {
        match kind {
            Kind::Track => self.tracks += 1,
            Kind::Arc => self.arcs += 1,
            Kind::Via => self.vias += 1,
        }
    }
    pub(crate) fn total(&self) -> usize {
        self.tracks + self.arcs + self.vias
    }
    fn json(&self) -> Value {
        json!({ "total": self.total(), "tracks": self.tracks, "arcs": self.arcs, "vias": self.vias })
    }
}

/// The board as the tool reads it: its routing, and everything else in a form the
/// read-back compares.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BoardReading {
    pub routing: Vec<RoutingItem>,
    /// Every other top-level object that must come back unchanged, as a sorted list of
    /// fingerprints: nets named, not numbered (a pre-KiCad-10 board renumbers its net
    /// table when a net loses its last track), a zone without its fill, a group without
    /// its routing members; anything else whole. The net table itself is left out.
    pub fixed: Vec<String>,
    /// Objects KiCad itself may update or remove when their tracks go: teardrop zones,
    /// generated objects (tuning patterns) and groups that hold only routing. This tool
    /// never asks KiCad to delete them; they are counted before and after, not compared.
    pub linked: BTreeMap<&'static str, usize>,
}

/// Whether a node carries `(locked yes)` or, in older files, a bare `locked` atom.
fn is_locked(node: &SexpNode) -> bool {
    node.find_str("locked") == Some("yes")
        || node
            .children()
            .unwrap_or(&[])
            .iter()
            .any(|child| matches!(child, SexpNode::Atom(atom) if atom == "locked"))
}

fn uuid_of(node: &SexpNode) -> Option<&str> {
    node.find_str("uuid")
        .or_else(|| node.find_str("tstamp"))
        .or_else(|| node.find_str("id"))
        .filter(|id| !id.is_empty())
}

fn members(group: &SexpNode) -> Vec<&str> {
    group
        .find("members")
        .and_then(SexpNode::children)
        .unwrap_or(&[])
        .iter()
        .skip(1)
        .filter_map(SexpNode::as_str)
        .collect()
}

/// Every KIID a locked group holds, through nested groups too.
fn locked_group_members<'a>(groups: &[&'a SexpNode]) -> BTreeSet<&'a str> {
    let by_id: BTreeMap<&str, &SexpNode> = groups
        .iter()
        .filter_map(|group| Some((uuid_of(group)?, *group)))
        .collect();
    let mut held = BTreeSet::new();
    let mut open: Vec<&SexpNode> = groups.iter().copied().filter(|g| is_locked(g)).collect();
    while let Some(group) = open.pop() {
        for member in members(group) {
            if held.insert(member) {
                if let Some(inner) = by_id.get(member) {
                    open.push(inner);
                }
            }
        }
    }
    held
}

impl BoardReading {
    pub(crate) fn of(text: &str) -> Result<Self> {
        let tree = konnect_sexp::parse_sexp(text).context("the board text does not parse")?;
        anyhow::ensure!(
            tree.head() == Some("kicad_pcb"),
            "the document KiCad returned is not a board"
        );
        // A pre-KiCad-10 board names a net by number and keeps a `(net N "name")` table.
        let net_names: BTreeMap<String, String> = tree
            .find_all("net")
            .into_iter()
            .filter_map(|net| {
                Some((
                    net.get(1)?.as_str()?.to_string(),
                    net.get(2)?.as_str()?.to_string(),
                ))
            })
            .collect();
        let top = tree.children().unwrap_or(&[]).get(1..).unwrap_or(&[]);
        let groups: Vec<&SexpNode> = top.iter().filter(|n| n.head() == Some("group")).collect();
        let held = locked_group_members(&groups);
        let linked_at = linked_objects(top);
        // What a group's members are compared without: routing, linked objects, and
        // identities that name nothing on the board (KiCad drops a deleted track from
        // its group; an older file may still list it).
        let other_ids: BTreeSet<&str> = top
            .iter()
            .enumerate()
            .filter(|(i, n)| n.head().and_then(Kind::of).is_none() && !linked_at.contains_key(i))
            .filter_map(|(_, n)| uuid_of(n))
            .collect();
        let mut routing = Vec::new();
        let mut fixed = Vec::new();
        let mut linked = BTreeMap::new();
        for (index, node) in top.iter().enumerate() {
            let Some(head) = node.head() else { continue };
            if let Some(kind) = Kind::of(head) {
                routing.push(routing_item(node, kind, &net_names, &held));
            } else if head == "net" {
                // The net table: renumbered by KiCad 9 as nets lose their last item.
            } else if let Some(name) = linked_at.get(&index) {
                *linked.entry(*name).or_insert(0) += 1;
            } else {
                fixed.push(fingerprint(node, &net_names, &other_ids));
            }
        }
        fixed.sort();
        Ok(Self {
            routing,
            fixed,
            linked,
        })
    }
}

/// The top-level objects counted in [`BoardReading::linked`], by position: teardrop
/// zones, generated objects, and groups whose members are all routing, linked objects
/// or identities that name nothing (to a fixed point, so a group of such groups is
/// linked too). An empty group is linked as well: it is what a group of routing only
/// becomes after the delete.
fn linked_objects(top: &[SexpNode]) -> BTreeMap<usize, &'static str> {
    let routing_ids: BTreeSet<&str> = top
        .iter()
        .filter(|n| n.head().and_then(Kind::of).is_some())
        .filter_map(uuid_of)
        .collect();
    let all_ids: BTreeSet<&str> = top.iter().filter_map(uuid_of).collect();
    let mut linked: BTreeMap<usize, &'static str> = BTreeMap::new();
    let mut linked_ids: BTreeSet<&str> = BTreeSet::new();
    for (index, node) in top.iter().enumerate() {
        let name = match node.head() {
            Some("zone") if crate::specctra::is_teardrop(node) => "teardrop_zones",
            Some("generated") => "generated",
            _ => continue,
        };
        linked.insert(index, name);
        linked_ids.extend(uuid_of(node));
    }
    loop {
        let mut grew = false;
        for (index, node) in top.iter().enumerate() {
            if node.head() != Some("group") || linked.contains_key(&index) {
                continue;
            }
            let list = members(node);
            let tied = |m: &&str| routing_ids.contains(m) || linked_ids.contains(m);
            // Empty or all-dangling counts too: a group whose routing KiCad removed is
            // empty (or dangling) after the delete, and it was linked before.
            if list.iter().all(|m| tied(m) || !all_ids.contains(m)) {
                linked.insert(index, "groups_of_routing_only");
                linked_ids.extend(uuid_of(node));
                grew = true;
            }
        }
        if !grew {
            return linked;
        }
    }
}

fn routing_item(
    node: &SexpNode,
    kind: Kind,
    net_names: &BTreeMap<String, String>,
    held: &BTreeSet<&str>,
) -> RoutingItem {
    let uuid = uuid_of(node).map(str::to_string);
    let raw_net = node.find_str("net").unwrap_or("");
    let net = net_names
        .get(raw_net)
        .cloned()
        .unwrap_or_else(|| raw_net.to_string());
    let layer = match kind {
        Kind::Via => node
            .find("layers")
            .and_then(SexpNode::children)
            .map(|layers| {
                layers
                    .iter()
                    .skip(1)
                    .filter_map(SexpNode::as_str)
                    .collect::<Vec<_>>()
                    .join("-")
            })
            .unwrap_or_default(),
        Kind::Track | Kind::Arc => node.find_str("layer").unwrap_or("").to_string(),
    };
    // What the export refuses in locked wiring (`specctra::locked_routing`): a lock it
    // does not read, no net, arcs, vias with a type (blind, micro), other than the
    // through span, or a drill it cannot take, and zero-length tracks.
    let size = node.find_f64("size");
    let drill = node.find("drill");
    let drill_mm = drill
        .and_then(|d| d.get(1))
        .and_then(SexpNode::as_str)
        .and_then(|d| d.parse::<f64>().ok());
    let export_refusal = match kind {
        _ if is_locked(node) && node.find_str("locked") != Some("yes") => {
            Some("a lock written the old way (the export reads it as unlocked)")
        }
        _ if net.is_empty() => Some("a locked item with no net"),
        Kind::Arc => Some("a locked routed arc"),
        Kind::Via if node.find("type").is_some() => Some("a locked blind or micro via"),
        Kind::Via if layer != "F.Cu-B.Cu" => Some("a locked via that is not a through via"),
        Kind::Via if drill.is_none() => Some("a locked via with no drill"),
        Kind::Via
            if drill.is_some_and(|d| {
                d.children()
                    .unwrap_or(&[])
                    .iter()
                    .any(|c| c.as_str() == Some("oval"))
            }) =>
        {
            Some("a locked via with an oval drill")
        }
        Kind::Via if matches!((drill_mm, size), (Some(d), Some(s)) if d >= s) => {
            Some("a locked via whose drill is not smaller than its size")
        }
        Kind::Track if point(node, "start") == point(node, "end") => {
            Some("a locked zero-length track")
        }
        _ => None,
    };
    RoutingItem {
        in_locked_group: uuid.as_deref().is_some_and(|id| held.contains(id)),
        kind,
        uuid,
        net,
        layer,
        locked: is_locked(node),
        export_refusal,
    }
}

/// A track end's coordinates as written.
fn point<'a>(node: &'a SexpNode, tag: &str) -> Option<&'a [SexpNode]> {
    node.find(tag)?.children()?.get(1..)
}

/// Nets by name, not number, anywhere in `node`.
fn named_nets(node: &SexpNode, net_names: &BTreeMap<String, String>) -> SexpNode {
    match node {
        SexpNode::List(children) if node.head() == Some("net") => {
            let number = children.get(1).and_then(SexpNode::as_str).unwrap_or("");
            let name = children
                .get(2)
                .and_then(SexpNode::as_str)
                .map(str::to_string)
                .or_else(|| net_names.get(number).cloned())
                .unwrap_or_else(|| number.to_string());
            SexpNode::List(vec![children[0].clone(), SexpNode::Str(name)])
        }
        SexpNode::List(children) => SexpNode::List(
            children
                .iter()
                .map(|child| named_nets(child, net_names))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The form of an object the read-back compares (see [`BoardReading::fixed`]).
fn fingerprint(
    node: &SexpNode,
    net_names: &BTreeMap<String, String>,
    other_ids: &BTreeSet<&str>,
) -> String {
    let node = named_nets(node, net_names);
    let SexpNode::List(children) = &node else {
        return format!("{node:?}");
    };
    let kept: Vec<SexpNode> = match node.head() {
        Some("zone") => children
            .iter()
            .filter(|child| !matches!(child.head(), Some("filled_polygon" | "fill_segments")))
            .cloned()
            .collect(),
        Some("group") => children
            .iter()
            .map(|child| match child {
                SexpNode::List(list) if child.head() == Some("members") => SexpNode::List(
                    list.iter()
                        .enumerate()
                        .filter(|(i, m)| {
                            *i == 0 || m.as_str().is_none_or(|id| other_ids.contains(id))
                        })
                        .map(|(_, m)| m.clone())
                        .collect(),
                ),
                other => other.clone(),
            })
            .collect(),
        _ => return format!("{node:?}"),
    };
    format!("{:?}", SexpNode::List(kept))
}

/// What one call would delete and keep.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Plan {
    pub plan_revision: String,
    pub include_locked: bool,
    pub delete: Vec<RoutingItem>,
    pub keep_locked: Vec<RoutingItem>,
    /// Routing the text gives no KIID for: it cannot be deleted by id, so the apply refuses.
    pub unidentified: usize,
}

impl Plan {
    pub(crate) fn of(
        board: &Path,
        text: &str,
        include_locked: bool,
    ) -> Result<(Self, BoardReading)> {
        let reading = BoardReading::of(text)?;
        let (delete, keep_locked): (Vec<_>, Vec<_>) = reading
            .routing
            .iter()
            .cloned()
            .partition(|item| include_locked || !item.held());
        let unidentified = delete.iter().filter(|item| item.uuid.is_none()).count();
        Ok((
            Self {
                plan_revision: plan_revision(board, include_locked, text),
                include_locked,
                delete,
                keep_locked,
                unidentified,
            },
            reading,
        ))
    }

    fn ids(&self) -> Vec<String> {
        self.delete
            .iter()
            .filter_map(|item| item.uuid.clone())
            .collect()
    }
}

/// The board path, the locked choice and the whole live text: any change to the board
/// between the preview and the apply changes it.
pub(crate) fn plan_revision(board: &Path, include_locked: bool, text: &str) -> String {
    let mut hash = Sha256::new();
    let flag = [include_locked as u8];
    for part in [
        b"delete_all_routing/1".as_slice(),
        board.as_os_str().as_encoded_bytes(),
        flag.as_slice(),
        text.as_bytes(),
    ] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    format!("{:x}", hash.finalize())
}

/// Counts by kind, by net (largest first, at most [`NETS_LISTED`]) and by layer.
pub(crate) fn breakdown(items: &[RoutingItem]) -> Value {
    let mut all = Counts::default();
    let mut by_net: BTreeMap<&str, Counts> = BTreeMap::new();
    let mut by_layer: BTreeMap<&str, Counts> = BTreeMap::new();
    for item in items {
        all.add(item.kind);
        by_net.entry(item.net.as_str()).or_default().add(item.kind);
        by_layer
            .entry(item.layer.as_str())
            .or_default()
            .add(item.kind);
    }
    let mut nets: Vec<(&str, Counts)> = by_net.into_iter().collect();
    nets.sort_by(|a, b| b.1.total().cmp(&a.1.total()).then(a.0.cmp(b.0)));
    let nets_total = nets.len();
    let listed = |name: &str, counts: &Counts, key: &str| {
        let mut row = counts.json();
        row[key] = json!(if name.is_empty() { "(no net)" } else { name });
        row
    };
    json!({
        "total": all.total(),
        "tracks": all.tracks,
        "arcs": all.arcs,
        "vias": all.vias,
        "by_net": nets.iter().take(NETS_LISTED).map(|(n, c)| listed(n, c, "net")).collect::<Vec<_>>(),
        "nets_total": nets_total,
        "nets_listed": nets_total.min(NETS_LISTED),
        // A via's layer is its span, its two outer layers joined by "-".
        "by_layer": by_layer.iter().map(|(l, c)| {
            let mut row = c.json();
            row["layer"] = json!(l);
            row
        }).collect::<Vec<_>>(),
    })
}

/// What is kept, and what of it `export_specctra_dsn` will still refuse.
fn kept_note(plan: &Plan) -> Value {
    let mut kept = Counts::default();
    let mut grouped = 0;
    let mut refused: BTreeMap<&str, usize> = BTreeMap::new();
    for item in &plan.keep_locked {
        kept.add(item.kind);
        if item.in_locked_group && !item.locked {
            grouped += 1;
            *refused
                .entry("routing held only by a locked group (the export reads it as unlocked)")
                .or_insert(0) += 1;
        } else if let Some(reason) = item.export_refusal {
            *refused.entry(reason).or_insert(0) += 1;
        }
    }
    let mut value = kept.json();
    value["held_only_by_locked_groups"] = json!(grouped);
    value["why"] = json!(if plan.include_locked {
        "include_locked is true: locked routing, and routing in locked groups, is deleted too"
    } else {
        "locked, or in a locked group; pass include_locked=true to delete it as well"
    });
    if !refused.is_empty() {
        value["export_will_refuse"] = json!(refused
            .iter()
            .map(|(reason, n)| json!({ "count": n, "what": reason }))
            .collect::<Vec<_>>());
        value["export_note"] = json!(
            "export_specctra_dsn will still refuse the board for these kept items: unlock them (or the group) and delete them too, or pass include_locked=true"
        );
    }
    value
}

const NEVER_DELETED: &str = "This tool sends KiCad only top-level tracks, arcs and vias to delete. Zones, footprints, graphics, text, dimensions and groups are never sent. KiCad itself may update or remove what belongs to deleted tracks: teardrop zones, tuning patterns (generated) and groups that hold only routing; those are counted in linked_objects.";

fn never_touched(reading: &BoardReading) -> Value {
    json!({
        "what": NEVER_DELETED,
        "linked_objects": reading.linked,
    })
}

// ─── The handler ─────────────────────────────────────────────────────────────

/// The arguments, checked before KiCad is asked anything.
struct Call {
    board: PathBuf,
    dry_run: bool,
    include_locked: bool,
    expected: Option<String>,
}

fn read_call(args: &Value) -> std::result::Result<Call, CallToolResult> {
    let board =
        get_path(args, "board").map_err(|error| invalid_arg("board", &format!("{error:#}")))?;
    if board.extension().is_none_or(|e| e != "kicad_pcb") {
        return Err(invalid_arg("board", "must name a .kicad_pcb file"));
    }
    let flag = |key: &str, default: bool| match args.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| invalid_arg(key, "must be a boolean")),
    };
    let dry_run = flag("dry_run", true)?;
    let include_locked = flag("include_locked", false)?;
    let expected = match args.get("expected_plan_revision") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .filter(|text| !text.is_empty())
                .ok_or_else(|| invalid_arg("expected_plan_revision", "must be a non-empty string"))?
                .to_string(),
        ),
    };
    if !dry_run && expected.is_none() {
        return Err(invalid_arg(
            "expected_plan_revision",
            "required when dry_run is false: run the preview first and pass its plan_revision",
        ));
    }
    Ok(Call {
        board,
        dry_run,
        include_locked,
        expected,
    })
}

pub(crate) async fn handle_delete_all_routing(
    args: &Value,
    ctx: &ToolContext,
) -> Result<CallToolResult> {
    let call = match read_call(args) {
        Ok(call) => call,
        Err(refusal) => return Ok(refusal),
    };
    let board = call.board.clone();
    let what = if call.dry_run {
        "routing delete preview"
    } else {
        "routing delete"
    };
    let result = attempt_ipc_write(ctx, &board, what, move |client| run(client, &call)).await?;
    Ok(match result {
        BoardWrite::Ipc(result) | BoardWrite::Refused(result) => result,
        BoardWrite::File(reason) => CallToolResult::error_kind(
            ToolErrorKind::EditorUnavailable {
                editor: "pcb".to_string(),
                reason: reason.premise(),
            },
            format!(
                "{} delete_all_routing works on the board KiCad holds open and never edits the \
                 file. Open '{}' in KiCad's PCB editor and call again. Nothing was deleted.",
                reason.premise(),
                board.display()
            ),
        ),
    })
}

/// The preview or the apply, on one IPC session. An error before the commit is a
/// refusal with nothing changed; the commit's own failure is classified by the shared
/// helpers (dropped, recovered or unknown); a read-back failure after a commit is an
/// uncertain outcome, never a refusal.
fn run(client: &konnect_ipc::client::KiCadIpcClient, call: &Call) -> Result<CallToolResult> {
    let document = client.find_open_board(&call.board).map_err(before_commit)?;
    let before_text = client
        .save_document_to_string_in(document.clone())
        .map_err(before_commit)?;
    let (plan, before) = Plan::of(&call.board, &before_text, call.include_locked)?;
    let mut body = json!({
        "board": call.board.display().to_string(),
        "source": "live_kicad_ipc",
        "plan_revision": plan.plan_revision,
        "include_locked": plan.include_locked,
        "kept_locked": kept_note(&plan),
        "never_touched": never_touched(&before),
    });
    if plan.unidentified > 0 {
        body["status"] = json!("blocked");
        body["will_delete"] = breakdown(&plan.delete);
        body["unidentified"] = json!(plan.unidentified);
        body["reason"] = json!(format!(
            "{} routing item(s) carry no KIID in the board text, so they cannot be deleted by \
             id. Nothing was deleted.",
            plan.unidentified
        ));
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::PlanBlocked {
                operation: "delete_all_routing".to_string(),
                reasons: vec![body["reason"].as_str().unwrap_or_default().to_string()],
            },
            serde_json::to_string_pretty(&body)?,
        ));
    }
    if call.dry_run {
        body["dry_run"] = json!(true);
        body["readback"] = json!("not_written");
        body["will_delete"] = breakdown(&plan.delete);
        if plan.delete.is_empty() {
            body["status"] = json!("noop");
            body["next_step"] = json!("Nothing to delete.");
        } else {
            body["status"] = json!("ready");
            body["next_step"] = json!(format!(
                "To delete, call delete_all_routing with dry_run=false, include_locked={} and \
                 expected_plan_revision=\"{}\". It refuses if the board changes first.",
                plan.include_locked, plan.plan_revision
            ));
        }
        return Ok(CallToolResult::json(&body));
    }
    if call.expected.as_deref() != Some(plan.plan_revision.as_str()) {
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::StaleTarget {
                target: call.board.display().to_string(),
                reason: "stale_plan_revision".to_string(),
            },
            "expected_plan_revision does not match the board KiCad holds now: the board \
             changed since the preview, or include_locked differs from it. Nothing was \
             deleted. Run the preview again and apply its new plan_revision.",
        ));
    }
    body["dry_run"] = json!(false);
    if plan.delete.is_empty() {
        body["status"] = json!("noop");
        body["removed"] = Counts::default().json();
        body["readback"] = json!("nothing_to_delete");
        return Ok(CallToolResult::json(&body));
    }
    let ids = plan.ids();
    client.run_commit_recovering_in(document.clone(), "Delete all routing", |client| {
        client.delete_items_in(document.clone(), ids.clone())
    })?;
    match verify(client, document, &plan, &before) {
        Ok(Ok(readback)) => {
            body["status"] = json!("applied");
            for (key, value) in readback {
                body[key] = value;
            }
            body["readback"] = json!("board_text_after_commit");
            body["undo"] = json!("one KiCad undo step (Ctrl+Z) restores everything deleted");
            body["next_step"] =
                json!("The board is not saved. Call save_project before export_specctra_dsn.");
            Ok(CallToolResult::json(&body))
        }
        Ok(Err(differences)) => Ok(uncertain(
            &call.board,
            format!("KiCad committed the delete, but the read-back differs: {differences}"),
        )),
        Err(error) => Ok(uncertain(
            &call.board,
            format!("KiCad committed the delete, but the read-back failed: {error:#}"),
        )),
    }
}

/// A read that timed out before any change is a refusal, not an unknown outcome: the
/// shared classifier would report `ipc_outcome_unknown` ("do not repeat the mutation")
/// for it. Every other error keeps its type, so a board that is not open stays that.
fn before_commit(error: anyhow::Error) -> anyhow::Error {
    match error.downcast_ref::<konnect_ipc::client::IpcOutcomeUnknown>() {
        Some(unknown) => anyhow::anyhow!(
            "KiCad did not answer {} ({}), a read before any change: nothing was deleted. \
             Call again when KiCad responds",
            unknown.command,
            unknown.reason
        ),
        None => error,
    }
}

fn uncertain(board: &Path, reason: String) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::MutationOutcomeUncertain {
            operation: "delete_all_routing".to_string(),
            path: board.display().to_string(),
            reason: reason.clone(),
        },
        format!(
            "{reason}. Inspect the open board before calling again: one Ctrl+Z in KiCad \
             undoes the whole delete. The board was not saved."
        ),
    )
}

/// The read-back: every planned item is gone, every kept locked item is still there,
/// and every other object is unchanged. `Ok(Err(..))` names what differs.
fn verify(
    client: &konnect_ipc::client::KiCadIpcClient,
    document: konnect_ipc::gen::kiapi::common::types::DocumentSpecifier,
    plan: &Plan,
    before: &BoardReading,
) -> Result<std::result::Result<Vec<(&'static str, Value)>, String>> {
    let after_text = client.save_document_to_string_in(document)?;
    let after = BoardReading::of(&after_text)?;
    Ok(compare(plan, before, &after))
}

pub(crate) fn compare(
    plan: &Plan,
    before: &BoardReading,
    after: &BoardReading,
) -> std::result::Result<Vec<(&'static str, Value)>, String> {
    let after_ids: BTreeSet<&str> = after
        .routing
        .iter()
        .filter_map(|item| item.uuid.as_deref())
        .collect();
    let quote = |ids: Vec<&str>| {
        let shown = ids.iter().take(IDS_QUOTED).copied().collect::<Vec<_>>();
        format!(
            "{} ({}{})",
            ids.len(),
            shown.join(", "),
            if ids.len() > IDS_QUOTED { ", …" } else { "" }
        )
    };
    let mut differences = Vec::new();
    let remaining: Vec<&str> = plan
        .delete
        .iter()
        .filter_map(|item| item.uuid.as_deref())
        .filter(|id| after_ids.contains(id))
        .collect();
    if !remaining.is_empty() {
        differences.push(format!(
            "planned items still on the board: {}",
            quote(remaining)
        ));
    }
    let lost: Vec<&str> = plan
        .keep_locked
        .iter()
        .filter_map(|item| item.uuid.as_deref())
        .filter(|id| !after_ids.contains(id))
        .collect();
    if !lost.is_empty() {
        differences.push(format!(
            "locked items that were to stay are gone: {}",
            quote(lost)
        ));
    }
    let planned_ids: BTreeSet<&str> = before
        .routing
        .iter()
        .filter_map(|item| item.uuid.as_deref())
        .collect();
    let new: Vec<&str> = after_ids
        .iter()
        .copied()
        .filter(|id| !planned_ids.contains(id))
        .collect();
    if !new.is_empty() {
        differences.push(format!(
            "routing items that were not on the board before: {}",
            quote(new)
        ));
    }
    if before.fixed != after.fixed {
        differences.push(format!(
            "other objects changed: {}",
            changed_heads(&before.fixed, &after.fixed)
        ));
    }
    if !differences.is_empty() {
        return Err(differences.join("; "));
    }
    let removed: Vec<RoutingItem> = plan
        .delete
        .iter()
        .filter(|item| {
            item.uuid
                .as_deref()
                .is_some_and(|id| !after_ids.contains(id))
        })
        .cloned()
        .collect();
    let mut left = Counts::default();
    for item in &after.routing {
        left.add(item.kind);
    }
    Ok(vec![
        ("removed", breakdown(&removed)),
        ("routing_left", left.json()),
        (
            "never_touched",
            json!({
                "what": NEVER_DELETED,
                "unchanged_objects": after.fixed.len(),
                "linked_objects_before": before.linked,
                "linked_objects_after": after.linked,
                "kicad_updated": linked_changes(&before.linked, &after.linked),
            }),
        ),
    ])
}

/// The linked objects whose count KiCad changed, said plainly (for example "KiCad
/// removed 4 teardrop_zones with their tracks").
fn linked_changes(before: &BTreeMap<&str, usize>, after: &BTreeMap<&str, usize>) -> Vec<String> {
    let names: BTreeSet<&str> = before.keys().chain(after.keys()).copied().collect();
    names
        .into_iter()
        .filter_map(|name| {
            let (b, a) = (
                *before.get(name).unwrap_or(&0),
                *after.get(name).unwrap_or(&0),
            );
            match b.cmp(&a) {
                std::cmp::Ordering::Greater => {
                    Some(format!("KiCad removed {} {name} with their tracks", b - a))
                }
                std::cmp::Ordering::Less => Some(format!("KiCad added {} {name}", a - b)),
                std::cmp::Ordering::Equal => None,
            }
        })
        .collect()
}

/// Which kinds of object differ between two fingerprint lists, with how many each side
/// holds that the other does not.
fn changed_heads(before: &[String], after: &[String]) -> String {
    fn count(list: &[String]) -> BTreeMap<&str, usize> {
        let mut map = BTreeMap::new();
        for entry in list {
            *map.entry(entry.as_str()).or_insert(0) += 1;
        }
        map
    }
    let (b, a) = (count(before), count(after));
    let mut heads: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (entry, n) in &b {
        let extra = n.saturating_sub(*a.get(entry).unwrap_or(&0));
        if extra > 0 {
            heads.entry(head_of(entry)).or_default().0 += extra;
        }
    }
    for (entry, n) in &a {
        let extra = n.saturating_sub(*b.get(entry).unwrap_or(&0));
        if extra > 0 {
            heads.entry(head_of(entry)).or_default().1 += extra;
        }
    }
    heads
        .iter()
        .map(|(head, (gone, came))| {
            format!("{head}: {gone} gone or changed, {came} new or changed")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The head of a fingerprint (`List([Atom("footprint"), …` → `footprint`).
fn head_of(fingerprint: &str) -> String {
    fingerprint
        .split('"')
        .nth(1)
        .unwrap_or("object")
        .to_string()
}

#[cfg(test)]
#[path = "pcb_routing_clear_tests.rs"]
mod tests;
