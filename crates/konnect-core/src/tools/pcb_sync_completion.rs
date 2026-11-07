//! Conservative sync replacement and whole-net copper updates. No routing.
use super::*;
use konnect_ipc::{
    builders::{any_is, pack_any},
    gen::kiapi,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct CopperChange {
    pub kiid: String,
    kind: &'static str,
    from: String,
    pub to: String,
}

/// Require complete copper coverage and unanimous saved intent for every old
/// terminal. A split, a skipped/board-only terminal or an unknown zone refuses.
pub(super) fn whole_net_target(
    design: &ExportedDesign,
    board: &BoardState,
    old: &str,
    new: &str,
) -> bool {
    if board.unresolved_copper || old.is_empty() || new.is_empty() {
        return false;
    }
    let known = board.copper.iter().filter(|c| c.net == old).count();
    if board.routed_nets.get(old).copied().unwrap_or(0) != known {
        return false;
    }
    board.footprints.iter().all(|fp| {
        fp.pad_nets
            .iter()
            .filter(|(_, net)| net.as_str() == old)
            .all(|(number, _)| {
                !fp.not_in_schematic
                    && design
                        .components
                        .iter()
                        .find(|c| {
                            fp.symbol_path
                                .as_deref()
                                .map(|path| path == c.symbol_path)
                                .unwrap_or(fp.reference == c.reference)
                        })
                        .is_some_and(|c| c.pad_nets.get(number).is_some_and(|net| net == new))
            })
    })
}

pub(super) fn copper_changes(design: &ExportedDesign, board: &BoardState) -> Vec<CopperChange> {
    board
        .copper
        .iter()
        .filter_map(|item| {
            let target = board
                .footprints
                .iter()
                .flat_map(|fp| {
                    fp.pad_nets
                        .iter()
                        .filter(|(_, n)| **n == item.net)
                        .map(move |(p, _)| (fp, p))
                })
                .find_map(|(fp, p)| {
                    design
                        .components
                        .iter()
                        .find(|c| {
                            fp.symbol_path
                                .as_deref()
                                .map(|path| path == c.symbol_path)
                                .unwrap_or(fp.reference == c.reference)
                        })
                        .and_then(|c| c.pad_nets.get(p))
                })?;
            (target != &item.net && whole_net_target(design, board, &item.net, target)).then(|| {
                CopperChange {
                    kiid: item.kiid.clone(),
                    kind: item.kind,
                    from: item.net.clone(),
                    to: target.clone(),
                }
            })
        })
        .collect()
}

/// Change only the net. Track endpoints, arc midpoint, via stack and all other
/// protobuf fields, including KIID, survive unchanged. Codes are KiCad-local.
pub(super) fn reassign_copper(item: &prost_types::Any, target: &str) -> Result<prost_types::Any> {
    macro_rules! update {
        ($t:ty, $name:literal) => {
            if any_is(item, $name) {
                let mut value = <$t>::decode(item.value.as_slice())?;
                value.net = Some(kiapi::board::types::Net {
                    name: target.into(),
                    code: None,
                });
                return Ok(pack_any(&value, $name));
            }
        };
    }
    update!(kiapi::board::types::Track, "kiapi.board.types.Track");
    update!(kiapi::board::types::Arc, "kiapi.board.types.Arc");
    update!(kiapi::board::types::Via, "kiapi.board.types.Via");
    bail!("unsupported copper item")
}

fn pads(item: &prost_types::Any) -> Result<BTreeMap<String, Vec<Vec<u8>>>> {
    let fp = kiapi::board::types::FootprintInstance::decode(item.value.as_slice())?;
    let mut result: BTreeMap<String, Vec<Vec<u8>>> = BTreeMap::new();
    for item in fp.definition.context("missing definition")?.items {
        if any_is(&item, "kiapi.board.types.Pad") {
            let mut pad = kiapi::board::types::Pad::decode(item.value.as_slice())?;
            pad.id = None;
            pad.net = None;
            result
                .entry(pad.number.clone())
                .or_default()
                .push(pad.encode_to_vec());
        }
    }
    for values in result.values_mut() {
        values.sort();
    }
    Ok(result)
}

pub(super) fn prepare_replacements(
    plan: &mut SyncPlan,
    prepared: &BTreeMap<String, PreparedFootprint>,
    snapshot: &LiveSnapshot,
    design: &ExportedDesign,
) -> Vec<SyncDiagnostic> {
    let mut diagnostics = Vec::new();
    for change in &plan.changes {
        let PlannedChange::Update {
            kiid,
            reference,
            replace_with: Some(id),
            ..
        } = change
        else {
            continue;
        };
        let build = (|| -> Result<_> {
            let source = &prepared
                .get(id)
                .context("replacement library unavailable")?
                .source;
            let old = snapshot
                .items
                .get(kiid)
                .context("replacement target missing")?;
            let updated = super::super::pcb_footprint_update::replacement_for_sync(
                old,
                id,
                source,
                &snapshot.net_codes,
            )?;
            let mut instance =
                kiapi::board::types::FootprintInstance::decode(updated.value.as_slice())?;
            let component = design
                .components
                .iter()
                .find(|component| component.reference == *reference)
                .context("replacement symbol missing")?;
            write_symbol_fields(&mut instance, &component.fields)?;
            let updated = pack_any(&instance, "kiapi.board.types.FootprintInstance");
            let old_pads = pads(old)?;
            let new_pads = pads(&updated)?;
            let before = snapshot
                .state
                .footprints
                .iter()
                .find(|fp| fp.kiid == *kiid)
                .context("replacement target missing")?;
            for (number, net) in &before.pad_nets {
                if (snapshot.state.unresolved_copper
                    || snapshot.state.routed_nets.contains_key(net))
                    && old_pads.get(number) != new_pads.get(number)
                {
                    bail!("replacement changes routed pad {number}; detach or reroute its copper explicitly first");
                }
            }
            // Apply symbol fields in the preview too, before any IPC mutation.
            update_footprint_item(&updated, change, &snapshot.net_codes)
        })();
        match build {
            Ok(item) => {
                plan.replacements.insert(kiid.clone(), item);
            }
            Err(e) => diagnostics.push(footprint_conflict(
                "footprint_replacement_unsafe",
                format!("{reference}: {e:#}"),
                id,
                vec![reference.clone()],
            )),
        }
    }
    diagnostics
}

/// Bind every raw live footprint/copper object and every prepared library byte.
/// This catches same-KIID moves, same-name library edits and changed pad shapes.
pub(super) fn seal_revision(
    plan: &mut SyncPlan,
    snapshot: &LiveSnapshot,
    prepared: &BTreeMap<String, PreparedFootprint>,
) {
    let mut hash = Sha256::new();
    hash.update(plan.plan_revision.as_bytes());
    for (id, item) in &snapshot.items {
        for bytes in [
            id.as_bytes(),
            item.type_url.as_bytes(),
            item.value.as_slice(),
        ] {
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
    }
    for (id, part) in prepared {
        for bytes in [id.as_bytes(), part.source.as_bytes()] {
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
    }
    plan.plan_revision = format!("{:x}", hash.finalize());
}

pub(super) fn record_readback(
    plan: &mut SyncPlan,
    snapshot: &LiveSnapshot,
    readback: &[prost_types::Any],
    client: &konnect_ipc::KiCadIpcClient,
) -> Result<()> {
    let present: BTreeMap<_, _> = readback
        .iter()
        .filter_map(|item| {
            let fp = kiapi::board::types::FootprintInstance::decode(item.value.as_slice()).ok()?;
            validate_terminals(&fp).ok()?;
            let state = board_footprint_from_instance(&fp).ok()?;
            Some((state.kiid.clone(), (state, item)))
        })
        .collect();
    for change in &plan.changes {
        let PlannedChange::Update {
            kiid,
            reference,
            value,
            symbol_path,
            dnp,
            pad_nets,
            preserve,
            replace_with,
            field_changes,
            ..
        } = change
        else {
            continue;
        };
        let actual = present.get(kiid);
        let state_matches = actual.is_some_and(|(fp, _)| {
            fp.reference == *reference
                && fp.value == *value
                && fp.symbol_path.as_deref() == Some(symbol_path.as_str())
                && fp.dnp == *dnp
                && fp.pad_nets == *pad_nets
                && fp.position == preserve.position
                && fp.rotation == preserve.rotation
                && fp.layer == preserve.layer
                && fp.locked == preserve.locked
                && field_changes.iter().all(|field| {
                    fp.fields.get(&field.name).map(String::as_str).unwrap_or("") == field.to
                })
        });
        if let Some((fp, _)) = actual {
            if let Some(before) = snapshot.state.footprints.iter().find(|f| f.kiid == *kiid) {
                let numbers: BTreeSet<_> = before.pad_nets.keys().chain(pad_nets.keys()).collect();
                for number in numbers {
                    if before.pad_nets.get(number) != pad_nets.get(number)
                        && fp.pad_nets.get(number) == pad_nets.get(number)
                    {
                        plan.counts.pads_reassigned.applied += 1;
                    }
                }
            }
        }
        let replacement_matches = if let Some(id) = replace_with {
            actual.is_some_and(|(fp, item)| {
                fp.footprint_id == *id
                    && plan.replacements.get(kiid).is_some_and(|sent| {
                        let metadata_matches =
                            kiapi::board::types::FootprintInstance::decode(sent.value.as_slice())
                                .ok()
                                .and_then(|fp| board_footprint_from_instance(&fp).ok())
                                .is_some_and(|expected| expected.fields == fp.fields);
                        if !metadata_matches {
                            return false;
                        }

                        pads(sent)
                            .ok()
                            .zip(pads(item).ok())
                            .is_some_and(|(a, b)| a == b)
                    })
            })
        } else {
            true
        };
        plan.counts.updated.applied += usize::from(state_matches && replacement_matches);
        if replace_with.is_some() && state_matches && replacement_matches {
            plan.counts.footprints_replaced.applied += 1;
        }
        if !state_matches || !replacement_matches {
            plan.diagnostics.push(conflict("sync_update_readback_differs",format!("{reference} ({kiid}) did not retain the planned identity, state, pad nets or replacement geometry"),Some(reference)));
        }
    }
    if !plan.copper_changes.is_empty() {
        let mut items = BTreeMap::new();
        for kind in [
            kiapi::common::types::KiCadObjectType::KotPcbTrace,
            kiapi::common::types::KiCadObjectType::KotPcbArc,
            kiapi::common::types::KiCadObjectType::KotPcbVia,
        ] {
            for item in client.get_items_in(snapshot.document.clone(), kind)? {
                if let Some(id) = copper_id(&item)? {
                    items.insert(id, item);
                }
            }
        }
        for change in &plan.copper_changes {
            let expected = reassign_copper(
                snapshot.items.get(&change.kiid).context("copper missing")?,
                &change.to,
            )?;
            let landed = items.get(&change.kiid).is_some_and(|item| {
                // Removing only the assigned code still compares actual net name.
                normalized_copper(item)
                    .ok()
                    .zip(normalized_copper(&expected).ok())
                    .is_some_and(|(a, b)| a == b)
            });
            if landed {
                plan.counts.copper_reassigned.applied += 1;
            } else {
                plan.diagnostics.push(conflict(
                    "copper_readback_differs",
                    format!(
                        "copper {} did not retain the planned net and geometry",
                        change.kiid
                    ),
                    None,
                ));
            }
        }
    }
    Ok(())
}
fn normalized_copper(item: &prost_types::Any) -> Result<prost_types::Any> {
    macro_rules! norm {
        ($t:ty, $name:literal) => {
            if any_is(item, $name) {
                let mut v = <$t>::decode(item.value.as_slice())?;
                if let Some(n) = v.net.as_mut() {
                    n.code = None;
                }
                return Ok(pack_any(&v, $name));
            }
        };
    }
    norm!(kiapi::board::types::Track, "kiapi.board.types.Track");
    norm!(kiapi::board::types::Arc, "kiapi.board.types.Arc");
    norm!(kiapi::board::types::Via, "kiapi.board.types.Via");
    bail!("unsupported copper")
}
fn copper_id(item: &prost_types::Any) -> Result<Option<String>> {
    macro_rules! id {
        ($t:ty, $name:literal) => {
            if any_is(item, $name) {
                return Ok(<$t>::decode(item.value.as_slice())?.id.map(|id| id.value));
            }
        };
    }
    id!(kiapi::board::types::Track, "kiapi.board.types.Track");
    id!(kiapi::board::types::Arc, "kiapi.board.types.Arc");
    id!(kiapi::board::types::Via, "kiapi.board.types.Via");
    bail!("unexpected copper response")
}

pub(super) fn capture_copper(
    items: &mut BTreeMap<String, prost_types::Any>,
    item: &prost_types::Any,
    id: Option<&kiapi::common::types::Kiid>,
) -> Result<()> {
    let id = id
        .filter(|id| !id.value.is_empty())
        .context("copper has no KIID")?;
    anyhow::ensure!(!items.contains_key(&id.value), "duplicate live copper KIID");
    items.insert(id.value.clone(), item.clone());
    Ok(())
}

pub(super) fn validate_terminals(fp: &kiapi::board::types::FootprintInstance) -> Result<()> {
    let mut nets = BTreeMap::new();
    for item in &fp.definition.as_ref().context("missing definition")?.items {
        if !any_is(item, "kiapi.board.types.Pad") {
            continue;
        }
        let pad =
            kiapi::board::types::Pad::decode(item.value.as_slice()).context("invalid live pad")?;
        if let Some(net) = pad.net.filter(|n| !n.name.is_empty()) {
            anyhow::ensure!(!pad.number.is_empty(), "netted live pad has no number");
            if let Some(old) = nets.insert(pad.number, net.name.clone()) {
                anyhow::ensure!(old == net.name, "repeated live pad carries multiple nets");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_and_unnumbered_netted_pads_are_not_valid_terminals() {
        let mut fp = kiapi::board::types::FootprintInstance {
            definition: Some(Default::default()),
            ..Default::default()
        };
        let pad = kiapi::board::types::Pad {
            net: Some(kiapi::board::types::Net {
                name: "NET".into(),
                code: None,
            }),
            ..Default::default()
        };
        fp.definition
            .as_mut()
            .unwrap()
            .items
            .push(pack_any(&pad, "kiapi.board.types.Pad"));
        assert!(validate_terminals(&fp).is_err());
        fp.definition.as_mut().unwrap().items = vec![prost_types::Any {
            type_url: "type.googleapis.com/kiapi.board.types.Pad".into(),
            value: vec![255],
        }];
        assert!(validate_terminals(&fp).is_err());
    }
    #[test]
    fn capture_requires_present_unique_copper_identity() {
        let mut items = BTreeMap::new();
        let item = prost_types::Any {
            type_url: "test".into(),
            value: vec![],
        };
        assert!(capture_copper(&mut items, &item, None).is_err());
        assert!(capture_copper(
            &mut items,
            &item,
            Some(&kiapi::common::types::Kiid {
                value: String::new()
            })
        )
        .is_err());
        let id = kiapi::common::types::Kiid {
            value: "unique".into(),
        };
        capture_copper(&mut items, &item, Some(&id)).unwrap();
        assert!(capture_copper(&mut items, &item, Some(&id)).is_err());
    }
}
