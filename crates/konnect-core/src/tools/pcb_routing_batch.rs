//! Explicit routing edits, never an autorouter. Live-only, one undo commit.
use super::*;
use crate::outcome::{self, OutcomeStatus};
use crate::tools::pcb_board::{attempt_ipc_write, BoardWrite};
use anyhow::{bail, ensure, Result};
use kiapi::{board::types as pcb, common::types as common};
use konnect_ipc::{
    builders::{any_is, pack_any},
    gen::kiapi,
    KiCadIpcClient,
};
use serde_json::Value;
use std::collections::BTreeSet;
type Document = common::DocumentSpecifier;
const MAX_ITEMS: usize = 200;

pub(super) fn tools() -> Vec<ToolDef> {
    let point = json!({"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"}},"required":["x","y"],"additionalProperties":false});
    let track = json!({"type":"object","properties":{"net_name":{"type":"string"},"layer":{"type":"string"},"width":{"type":"number"},"points":{"type":"array","minItems":2,"maxItems":201,"items":point}},"required":["net_name","layer","points"],"additionalProperties":false});
    let via = json!({"type":"object","properties":{"net_name":{"type":"string"},"x":{"type":"number"},"y":{"type":"number"},"drill":{"type":"number"},"pad_size":{"type":"number"}},"required":["net_name","x","y"],"additionalProperties":false});
    vec![
        tool!("route_batch","Create explicit polylines and through vias in one live KiCad undo commit (max 200 copper objects). Defaults come from live effective netclasses; widths below preferred class width refuse. Returns readback KIIDs. Does not route around obstacles, check clearance, refill or save.",json!({"type":"object","properties":{"board":{"type":"string"},"tracks":{"type":"array","maxItems":200,"items":track},"vias":{"type":"array","maxItems":200,"items":via}},"required":["board"],"additionalProperties":false}),|args,ctx|async move{handle_route_batch(args,ctx).await}).with_board_access(crate::tools::BoardAccess::LiveOnly),
        tool!("query_vias","Read live vias with KIIDs, net, position, span, diameter and drill. Paged and byte-bounded; never reads a saved fallback.",json!({"type":"object","properties":{"board":{"type":"string"},"net_name":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":10000},"offset":{"type":"integer","minimum":0}},"required":["board"]}),|args,ctx|async move{handle_query_vias(args,ctx).await}).with_board_access(crate::tools::BoardAccess::LiveOnly),
        tool!("delete_via","Delete an observed, unlocked live via by KIID in one undo commit and verify absence. Refuses other object types and stale IDs.",json!({"type":"object","properties":{"board":{"type":"string"},"uuid":{"type":"string"}},"required":["board","uuid"],"additionalProperties":false}),|args,ctx|async move{handle_delete_via(args,ctx).await}).with_board_access(crate::tools::BoardAccess::LiveOnly),
    ]
}
async fn handle_route_batch(args: &Value, ctx: &ToolContext) -> Result<CallToolResult> {
    handle("route_batch", args, ctx).await
}
async fn handle_query_vias(args: &Value, ctx: &ToolContext) -> Result<CallToolResult> {
    handle("query_vias", args, ctx).await
}
async fn handle_delete_via(args: &Value, ctx: &ToolContext) -> Result<CallToolResult> {
    handle("delete_via", args, ctx).await
}
fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    let value = args
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("{key} must be a string"))?;
    ensure!(
        !value.trim().is_empty() && !value.contains('\0'),
        "{key} must be nonempty without NUL"
    );
    Ok(value)
}
fn number(args: &Value, key: &str) -> Result<f64> {
    let value = args
        .get(key)
        .and_then(Value::as_f64)
        .with_context(|| format!("{key} must be a finite number"))?;
    Ok(value)
}
fn optional(args: &Value, key: &str) -> Result<Option<f64>> {
    args.get(key).map(|_| number(args, key)).transpose()
}
fn coordinate(args: &Value, key: &str) -> Result<f64> {
    let value = number(args, key)?;
    ensure!(
        value.abs() <= 2000.0,
        "{key} exceeds the supported +/-2000 mm coordinate range"
    );
    Ok(value)
}
fn size(value: f64) -> Result<f64> {
    ensure!(
        (0.000001..=100.0).contains(&value),
        "copper dimension must be between 1 nm and 100 mm"
    );
    Ok(value)
}
fn field_error(reason: impl ToString) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::InvalidArgument {
            field: "routing".into(),
            reason: reason.to_string(),
        },
        reason.to_string(),
    )
}
fn envelope(mut result: CallToolResult, board: &Path) -> CallToolResult {
    if let Some(crate::mcp::protocol::ToolContent::Text { text }) = result.content.first_mut() {
        if serde_json::from_str::<Value>(text).is_err() {
            *text = json!({"error":text}).to_string();
        }
    }
    let kind = crate::mcp::error::extract_error_kind(&result);
    let uncertain = matches!(
        kind.as_deref(),
        Some("mutation_outcome_uncertain" | "ipc_outcome_unknown" | "ipc_batch_recovered")
    );
    let status = if uncertain {
        OutcomeStatus::Uncertain
    } else if result.is_error {
        OutcomeStatus::Failed
    } else {
        OutcomeStatus::Complete
    };
    let success = status == OutcomeStatus::Complete;
    outcome::attach(
        result,
        outcome::summary(
            status,
            board.display().to_string(),
            if success || uncertain {
                "live_kicad_ipc"
            } else {
                "not_established"
            },
            1,
            usize::from(success),
            usize::from(!success),
            if success {
                None
            } else {
                Some(if uncertain {
                    outcome::inspect_before_retry()
                } else {
                    outcome::retry_whole_request()
                })
            },
        ),
    )
}
pub(super) async fn handle(
    name: &'static str,
    args: &Value,
    ctx: &ToolContext,
) -> Result<CallToolResult> {
    let board = match get_path(args, "board") {
        Ok(p) => p,
        Err(e) => return Ok(envelope(field_error(e), Path::new("unknown"))),
    };
    let owned = args.clone();
    let requested = board.clone();
    let result = attempt_ipc_write(ctx, &board, name, move |client| {
        let document = client.find_open_board(&requested)?;
        // All fallible preflight happens before a commit. Post-write failures
        // are caught separately and preserve uncertainty rather than refusal.
        match prepare(name, &owned, client, &document) {
            Ok(prepared) => execute(name, &owned, client, &document, prepared, &requested),
            Err(error) => Ok(field_error(format!("preflight refused: {error:#}"))),
        }
    })
    .await?;
    Ok(envelope(
        match result {
            BoardWrite::Ipc(r) | BoardWrite::Refused(r) => r,
            BoardWrite::File(_) => {
                field_error("requested board must be open in KiCad; no file fallback")
            }
        },
        &board,
    ))
}
#[derive(Default)]
struct Prepared {
    before: BTreeMap<String, prost_types::Any>,
    creates: Vec<prost_types::Any>,
    updates: Vec<prost_types::Any>,
    deletes: Vec<String>,
    checks: Vec<Value>,
    ends: Option<Value>,
}
fn object_id(item: &prost_types::Any) -> Result<String> {
    let id = if any_is(item, "kiapi.board.types.Track") {
        pcb::Track::decode(item.value.as_slice())?.id
    } else if any_is(item, "kiapi.board.types.Via") {
        pcb::Via::decode(item.value.as_slice())?.id
    } else {
        bail!("unexpected routing object type")
    };
    let id = id.context("routing item lacks KIID")?.value;
    ensure!(!id.is_empty(), "routing item has empty KIID");
    Ok(id)
}
fn inventory(
    client: &KiCadIpcClient,
    document: &Document,
) -> Result<BTreeMap<String, prost_types::Any>> {
    let mut result = BTreeMap::new();
    for (kind, url) in [
        (
            common::KiCadObjectType::KotPcbTrace,
            "kiapi.board.types.Track",
        ),
        (common::KiCadObjectType::KotPcbVia, "kiapi.board.types.Via"),
    ] {
        for item in client.get_items_in(document.clone(), kind)? {
            ensure!(
                any_is(&item, url),
                "KiCad returned the wrong routing object type"
            );
            let id = object_id(&item)?;
            ensure!(result.insert(id, item).is_none(), "duplicate routing KIID");
        }
    }
    Ok(result)
}
fn normalized(item: &prost_types::Any) -> Result<prost_types::Any> {
    if any_is(item, "kiapi.board.types.Track") {
        let mut t = pcb::Track::decode(item.value.as_slice())?;
        t.id = None;
        if let Some(n) = t.net.as_mut() {
            n.code = None;
        }
        Ok(pack_any(&t, "kiapi.board.types.Track"))
    } else if any_is(item, "kiapi.board.types.Via") {
        let mut v = pcb::Via::decode(item.value.as_slice())?;
        v.id = None;
        if let Some(n) = v.net.as_mut() {
            n.code = None;
        }
        Ok(pack_any(&v, "kiapi.board.types.Via"))
    } else {
        bail!("unexpected routing object")
    }
}
fn view(item: &prost_types::Any) -> Result<Value> {
    let uuid = object_id(item)?;
    if any_is(item, "kiapi.board.types.Track") {
        let t = pcb::Track::decode(item.value.as_slice())?;
        let p = |p: Option<common::Vector2>| {
            p.map(|v|json!({"x":konnect_ipc::builders::nm_to_mm(v.x_nm),"y":konnect_ipc::builders::nm_to_mm(v.y_nm)}))
        };
        Ok(
            json!({"uuid":uuid,"kind":"track","net":t.net.map(|n|n.name),"layer":layer_name(t.layer),"width":t.width.map(|w|konnect_ipc::builders::nm_to_mm(w.value_nm)),"from":p(t.start),"to":p(t.end)}),
        )
    } else {
        let v = pcb::Via::decode(item.value.as_slice())?;
        let stack = v.pad_stack.as_ref();
        Ok(
            json!({"uuid":uuid,"kind":"via","net":v.net.map(|n|n.name),"x":v.position.as_ref().map(|p|konnect_ipc::builders::nm_to_mm(p.x_nm)),"y":v.position.as_ref().map(|p|konnect_ipc::builders::nm_to_mm(p.y_nm)),"drill":stack.and_then(|s|s.drill.as_ref()).and_then(|d|d.diameter.as_ref()).map(|d|konnect_ipc::builders::nm_to_mm(d.x_nm)),"pad_size":stack.and_then(|s|s.copper_layers.first()).and_then(|l|l.size.as_ref()).map(|d|konnect_ipc::builders::nm_to_mm(d.x_nm)),"layers":stack.map(|s|s.layers.iter().map(|l|layer_name(*l)).collect::<Vec<_>>()),"locked":match v.locked {v if v==common::LockedState::LsLocked as i32=>Some(true),v if v==common::LockedState::LsUnlocked as i32=>Some(false),_=>None}}),
        )
    }
}
struct Rules {
    nets: BTreeMap<String, i32>,
    rules: konnect_ipc::IpcEffectiveRoutingRules,
    layers: BTreeSet<i32>,
}
impl Rules {
    fn read(client: &KiCadIpcClient, document: &Document) -> Result<Self> {
        let nets = client
            .get_nets_in(document.clone())?
            .into_iter()
            .map(|n| (n.name, n.netcode))
            .collect();
        let rules = client.get_effective_routing_rules_in(document.clone())?;
        let layers = client
            .get_enabled_layers_in(document.clone())?
            .layers
            .into_iter()
            .map(|l| l.id)
            .collect();
        Ok(Self {
            nets,
            rules,
            layers,
        })
    }
    fn net(&self, name: &str) -> Result<(i32, &konnect_ipc::IpcRoutingRules)> {
        Ok((
            *self
                .nets
                .get(name)
                .context("net is not on the live board")?,
            self.rules
                .get(name)
                .context("effective live netclass is unavailable")?,
        ))
    }
    fn layer(&self, name: &str) -> Result<i32> {
        ensure!(name.ends_with(".Cu"), "routing requires a copper layer");
        let layer = konnect_ipc::builders::try_layer_from_name(name)? as i32;
        ensure!(
            self.layers.contains(&layer),
            "requested copper layer is not enabled"
        );
        Ok(layer)
    }
    fn track(
        &self,
        args: &Value,
        points: &[(f64, f64)],
        checks: &mut Vec<Value>,
    ) -> Result<Vec<prost_types::Any>> {
        let net = text(args, "net_name")?;
        let layer = text(args, "layer")?;
        self.layer(layer)?;
        let (code, class) = self.net(net)?;
        let preferred = class
            .track_width_mm
            .context("live class has no preferred track width")?;
        let width_nm = nm(size(optional(args, "width")?.unwrap_or(preferred))?);
        let width = konnect_ipc::builders::nm_to_mm(width_nm);
        // A Konnect policy using the preferred width, not a KiCad DRC minimum.
        ensure!(
            width_nm >= nm(preferred),
            "width is below the live netclass preferred width"
        );
        checks.push(json!({"net":net,"netclass":class.class_name,"preferred_width":preferred,"width":width,"policy":"at_least_preferred_width","source":"live_effective_netclass"}));
        let mut out = Vec::new();
        for pair in points.windows(2) {
            let mut t = konnect_ipc::builders::build_track(
                net, code, layer, width, pair[0].0, pair[0].1, pair[1].0, pair[1].1,
            );
            t.width = Some(common::Distance { value_nm: width_nm });
            ensure!(
                t.start != t.end,
                "zero-length segment after nanometer conversion"
            );
            out.push(pack_any(&t, "kiapi.board.types.Track"));
        }
        Ok(out)
    }
    fn via(&self, args: &Value) -> Result<prost_types::Any> {
        let net = text(args, "net_name")?;
        let (code, class) = self.net(net)?;
        self.layer("F.Cu")?;
        self.layer("B.Cu")?;
        let x = coordinate(args, "x")?;
        let y = coordinate(args, "y")?;
        let drill = size(
            optional(args, "drill")?
                .or(class.via_drill_mm)
                .context("live class has no via drill")?,
        )?;
        let diameter = size(
            optional(args, "pad_size")?
                .or(class.via_diameter_mm)
                .context("live class has no via diameter")?,
        )?;
        let (drill_nm, diameter_nm) = (nm(drill), nm(diameter));
        ensure!(
            diameter_nm > drill_nm,
            "via diameter must exceed drill after nanometer rounding"
        );
        let mut via = konnect_ipc::builders::build_via(net, code, x, y, drill, diameter);
        let stack = via.pad_stack.as_mut().unwrap();
        stack.drill.as_mut().unwrap().diameter = Some(common::Vector2 {
            x_nm: drill_nm,
            y_nm: drill_nm,
        });
        stack.copper_layers[0].size = Some(common::Vector2 {
            x_nm: diameter_nm,
            y_nm: diameter_nm,
        });
        Ok(pack_any(&via, "kiapi.board.types.Via"))
    }
}
fn points(args: &Value) -> Result<Vec<(f64, f64)>> {
    let list = args
        .get("points")
        .and_then(Value::as_array)
        .context("points must be an array")?;
    ensure!(
        (2..=201).contains(&list.len()),
        "polyline needs 2..201 points"
    );
    list.iter()
        .map(|p| Ok((coordinate(p, "x")?, coordinate(p, "y")?)))
        .collect()
}
fn live_pad(
    client: &KiCadIpcClient,
    document: &Document,
    reference: &str,
    number: &str,
    net: &str,
    layer: &str,
) -> Result<(f64, f64)> {
    let pads = client
        .get_footprint_pads_in(document.clone(), reference)?
        .with_context(|| format!("Footprint '{reference}' not found on the board open in KiCad"))?;
    let pads = pads
        .iter()
        .filter(|p| p.number == number)
        .collect::<Vec<_>>();
    ensure!(
        !pads.is_empty(),
        "Pad '{number}' not found on '{reference}'"
    );
    ensure!(
        pads.len() == 1,
        "pad number has multiple physical anchors; provide explicit coordinates instead"
    );
    let p = pads[0];
    ensure!(
        p.net == net,
        "pad '{reference}.{number}' does not belong to requested net"
    );
    ensure!(
        p.layers.iter().any(|l| l == layer),
        "pad has no known copper on requested layer"
    );
    ensure!(
        p.x.is_finite() && p.y.is_finite() && p.x.abs() <= 2000.0 && p.y.abs() <= 2000.0,
        "pad position outside supported coordinates"
    );
    Ok((p.x, p.y))
}
fn prepare(
    name: &str,
    args: &Value,
    client: &KiCadIpcClient,
    document: &Document,
) -> Result<Prepared> {
    let mut p = Prepared::default();
    // Pad lookup precedes routing-rule lookup, so missing targets are named.
    let pad_ends = if name == "route_pad_to_pad" {
        let net = text(args, "net_name")?;
        let layer = match args.get("layer") {
            Some(_) => text(args, "layer")?,
            None => "F.Cu",
        };
        let a = live_pad(
            client,
            document,
            text(args, "ref1")?,
            text(args, "pad1")?,
            net,
            layer,
        )?;
        let b = live_pad(
            client,
            document,
            text(args, "ref2")?,
            text(args, "pad2")?,
            net,
            layer,
        )?;
        p.ends = Some(
            json!({"from":{"ref":args["ref1"],"pad":args["pad1"],"x":a.0,"y":a.1},"to":{"ref":args["ref2"],"pad":args["pad2"],"x":b.0,"y":b.1}}),
        );
        Some((a, b, layer.to_string()))
    } else {
        None
    };
    p.before = inventory(client, document)?;
    if name == "query_vias" {
        if args.get("net_name").is_some() {
            text(args, "net_name")?;
        }
        return Ok(p);
    }
    if name == "delete_via" {
        let id = text(args, "uuid")?;
        let item = p
            .before
            .get(id)
            .context("via KIID not found on requested board")?;
        ensure!(any_is(item, "kiapi.board.types.Via"), "KIID is not a via");
        ensure!(
            pcb::Via::decode(item.value.as_slice())?.locked
                == common::LockedState::LsUnlocked as i32,
            "via is locked or lock state is unknown"
        );
        p.deletes.push(id.into());
        return Ok(p);
    }
    let rules = Rules::read(client, document)?;
    match name {
        "route_trace" => {
            p.creates = rules.track(
                args,
                &[
                    (coordinate(args, "x1")?, coordinate(args, "y1")?),
                    (coordinate(args, "x2")?, coordinate(args, "y2")?),
                ],
                &mut p.checks,
            )?;
        }
        "add_via" => p.creates.push(rules.via(args)?),
        "route_pad_to_pad" => {
            let (a, b, layer) = pad_ends.unwrap();
            let mut data = args.clone();
            data["layer"] = json!(layer);
            let points = if a.0 == b.0 || a.1 == b.1 {
                vec![a, b]
            } else {
                vec![a, (b.0, a.1), b]
            };
            p.creates = rules.track(&data, &points, &mut p.checks)?;
        }
        "route_batch" => {
            for (key, via) in [("tracks", false), ("vias", true)] {
                if let Some(value) = args.get(key) {
                    let list = value
                        .as_array()
                        .with_context(|| format!("{key} must be an array"))?;
                    for item in list {
                        if via {
                            p.creates.push(rules.via(item)?);
                        } else {
                            p.creates
                                .extend(rules.track(item, &points(item)?, &mut p.checks)?);
                        }
                        ensure!(
                            p.creates.len() <= MAX_ITEMS,
                            "batch exceeds 200 copper objects"
                        );
                    }
                }
            }
        }
        "modify_trace" => {
            let id = text(args, "uuid")?;
            let old = p
                .before
                .get(id)
                .context("trace KIID not found on requested board")?;
            ensure!(
                any_is(old, "kiapi.board.types.Track"),
                "KIID is not a straight trace"
            );
            let mut t = pcb::Track::decode(old.value.as_slice())?;
            ensure!(
                t.locked == common::LockedState::LsUnlocked as i32,
                "trace is locked or lock state is unknown"
            );
            ensure!(
                ["net_name", "layer", "width", "x1", "y1", "x2", "y2"]
                    .iter()
                    .any(|k| args.get(*k).is_some()),
                "supply at least one trace field"
            );
            if args.get("net_name").is_some() {
                let net = text(args, "net_name")?;
                let (code, _) = rules.net(net)?;
                t.net = Some(konnect_ipc::builders::net(net, code));
            }
            if args.get("layer").is_some() {
                t.layer = rules.layer(text(args, "layer")?)?;
            }
            if let Some(width) = optional(args, "width")? {
                t.width = Some(common::Distance {
                    value_nm: nm(size(width)?),
                });
            }
            for (field, end, x) in [
                ("x1", true, true),
                ("y1", true, false),
                ("x2", false, true),
                ("y2", false, false),
            ] {
                if args.get(field).is_some() {
                    let pos = if end {
                        t.start.as_mut()
                    } else {
                        t.end.as_mut()
                    }
                    .context("trace has no endpoint")?;
                    let value = konnect_ipc::builders::mm_to_nm(coordinate(args, field)?);
                    if x {
                        pos.x_nm = value
                    } else {
                        pos.y_nm = value
                    }
                }
            }
            let a = t.start.as_ref().context("trace has no start")?;
            let b = t.end.as_ref().context("trace has no end")?;
            let spec = json!({"net_name":t.net.as_ref().context("trace has no net")?.name,"layer":layer_name(t.layer),"width":konnect_ipc::builders::nm_to_mm(t.width.as_ref().context("trace has no width")?.value_nm)});
            rules.track(
                &spec,
                &[
                    (
                        konnect_ipc::builders::nm_to_mm(a.x_nm),
                        konnect_ipc::builders::nm_to_mm(a.y_nm),
                    ),
                    (
                        konnect_ipc::builders::nm_to_mm(b.x_nm),
                        konnect_ipc::builders::nm_to_mm(b.y_nm),
                    ),
                ],
                &mut p.checks,
            )?;
            p.updates.push(pack_any(&t, "kiapi.board.types.Track"));
        }
        _ => bail!("unknown routing operation"),
    }
    ensure!(
        !p.creates.is_empty() || !p.updates.is_empty(),
        "routing batch must contain at least one copper object"
    );
    Ok(p)
}
fn execute(
    name: &str,
    args: &Value,
    client: &KiCadIpcClient,
    document: &Document,
    p: Prepared,
    board: &Path,
) -> Result<CallToolResult> {
    if name == "query_vias" {
        let (offset, limit) = crate::tools::response_budget::window(args, 100)
            .map_err(|_| anyhow::anyhow!("invalid pagination"))?;
        let items = p
            .before
            .values()
            .filter(|i| any_is(i, "kiapi.board.types.Via"))
            .map(view)
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|v| args.get("net_name").is_none_or(|n| v["net"] == *n))
            .collect::<Vec<_>>();
        let mut value = crate::tools::response_budget::list_response("vias", &items, offset, limit);
        value["source"] = json!("live_kicad_ipc");
        return Ok(CallToolResult::json(&value));
    }
    let committed =
        client.run_commit_recovering_in(document.clone(), "Explicit routing edit", |c| {
            let created = if p.creates.is_empty() {
                Vec::new()
            } else {
                c.create_items_in_returning(document.clone(), p.creates.clone())?
            };
            if !p.updates.is_empty() {
                c.update_items_in(document.clone(), p.updates.clone())?;
            }
            if !p.deletes.is_empty() {
                c.delete_items_in(document.clone(), p.deletes.clone())?;
            }
            Ok(created)
        });
    let created = match committed {
        Ok(created) => created,
        Err(error) => {
            return Ok(uncertain(
                name,
                board,
                format!(
                    "routing commit did not complete; backend recovery may have run: {error:#}"
                ),
            ))
        }
    };
    let verification = (|| -> Result<Value> {
        let after = inventory(client, document)?;
        ensure!(
            created.len() == p.creates.len(),
            "creation receipts are incomplete"
        );
        let mut ids = BTreeSet::new();
        let mut expected = p
            .creates
            .iter()
            .map(normalized)
            .collect::<Result<Vec<_>>>()?;
        for item in &created {
            let id = object_id(item)?;
            ensure!(
                !p.before.contains_key(&id) && ids.insert(id.clone()),
                "creation returned duplicate or pre-existing KIID"
            );
            let shape = normalized(item)?;
            let index = expected
                .iter()
                .position(|e| *e == shape)
                .context("created object differs from requested copper")?;
            expected.remove(index);
            ensure!(
                normalized(
                    after
                        .get(&id)
                        .context("created KIID is absent on readback")?
                )? == shape,
                "created copper readback differs"
            );
        }
        for (id, old) in &p.before {
            if p.deletes.contains(id) {
                ensure!(!after.contains_key(id), "deleted via remains on readback");
                continue;
            }
            let want = p
                .updates
                .iter()
                .find(|i| object_id(i).ok().as_deref() == Some(id))
                .unwrap_or(old);
            ensure!(
                normalized(after.get(id).context("existing copper disappeared")?)?
                    == normalized(want)?,
                "existing or modified copper readback differs"
            );
        }
        ensure!(
            after.len() == p.before.len() + p.creates.len() - p.deletes.len(),
            "routing inventory changed unexpectedly"
        );
        let created = ids
            .iter()
            .map(|id| view(&after[id]))
            .collect::<Result<Vec<_>>>()?;
        let modified = p
            .updates
            .iter()
            .map(|item| {
                let id = object_id(item)?;
                view(&after[&id])
            })
            .collect::<Result<Vec<_>>>()?;
        let mut result = json!({"status":"applied","source":"live_kicad_ipc","created":created,"modified":modified,"deleted":p.deletes,"created_count":p.creates.len(),"modified_count":p.updates.len(),"deleted_count":p.deletes.len(),"checks":p.checks,"clearance_checked":false,"saved":false,"undo":"one KiCad undo commit"});
        let primary = result["created"]
            .as_array()
            .and_then(|items| items.first())
            .or_else(|| {
                result["modified"]
                    .as_array()
                    .and_then(|items| items.first())
            })
            .cloned();
        if let Some(primary) = &primary {
            for field in [
                "uuid", "net", "layer", "width", "from", "to", "x", "y", "drill", "pad_size",
            ] {
                if let Some(value) = primary.get(field) {
                    result[field] = value.clone();
                }
            }
        }
        if let Some(ends) = p.ends {
            result["from"] = ends["from"].clone();
            result["to"] = ends["to"].clone();
            result["routed"] = json!(true);
        }
        if name == "modify_trace" {
            result["modified_uuid"] = result["modified"][0]["uuid"].clone();
        }
        Ok(result)
    })();
    Ok(match verification {
        Ok(value) => CallToolResult::json(&value),
        Err(error) => uncertain(
            name,
            board,
            format!("commit completed but verification failed: {error:#}"),
        ),
    })
}

fn layer_name(layer: i32) -> String {
    pcb::BoardLayer::try_from(layer)
        .map(|l| l.as_str_name().trim_start_matches("BL_").replace('_', "."))
        .unwrap_or_else(|_| format!("unknown_{layer}"))
}

fn nm(mm: f64) -> i64 {
    (mm * 1_000_000.0).round() as i64
}

fn uncertain(name: &str, board: &Path, reason: String) -> CallToolResult {
    CallToolResult::error_kind(
        ToolErrorKind::MutationOutcomeUncertain {
            operation: name.into(),
            path: board.display().to_string(),
            reason: reason.clone(),
        },
        format!(
            "Routing persistence is unproven: {reason}. Inspect the open board before retrying."
        ),
    )
}
#[cfg(test)]
#[path = "pcb_routing_batch_tests.rs"]
mod tests;
