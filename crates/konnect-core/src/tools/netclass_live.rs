//! Netclass writes while KiCad holds the board (Row RW).
//!
//! KiCad keeps the project's netclasses in memory and writes them to
//! `.kicad_pro` on every board save (`pcbnew/files.cpp:1049`, through
//! `SETTINGS_MANAGER::SaveProject`). A write to the file alone is therefore
//! undone by KiCad's next save (#791). KiCad 10's IPC API has `SetNetClasses`
//! (`common/api/api_handler_common.cpp:123`), which changes KiCad's own copy.
//! So a netclass change goes to KiCad first, is read back, and then goes to the
//! project file as well: both copies hold it, and KiCad's next save writes the
//! same value again.
//!
//! KiCad 10.0.1 to 10.0.6 must not be written this way. Their
//! `NETCLASS::Deserialize` empties the class's constituents
//! (`netclass.cpp:223`, `SetConstituentNetclasses( {} )`), nothing refills
//! them, and `NETCLASS::GetName()` then reads the first element of that empty
//! list (`netclass.cpp:328-333`). KiCad calls `GetName()` on every class when it
//! saves the project (`net_settings.cpp:76`), so the next save after such a
//! write reads past an empty vector. KiCad 10.0.7 fixed it (its `Deserialize`
//! sets `{ this }`, and `Serialize` marks a class explicit when it has at most
//! one constituent); 10.0.6 has neither change. So the live write needs a KiCad
//! of at least [`FIXED_IN`], read with `GetVersion`; an older one is refused
//! with the steps in KiCad's own dialog.
//!
//! Three facts of KiCad 10.0.1 shape the messages sent. The second and third
//! hold on 10.0.7 and 10.0.8 too; the first holds up to 10.0.6 only (10.0.7
//! lists a class explicit, and answers a skipped one with `AS_BAD_REQUEST`), and
//! the message is built to work either way:
//! - `NETCLASS::Serialize` marks every class `NCT_IMPLICIT` (its constructor
//!   puts the class in its own constituents, `netclass.cpp:56-59` and `:142`),
//!   and `NETCLASS::Deserialize` returns early on `NCT_IMPLICIT` after setting
//!   only the name and priority (`netclass.cpp:216-221`). A class read with
//!   `GetNetClasses` and sent back unchanged is therefore silently not applied.
//!   Every class sent here is built fresh and marked `NCT_EXPLICIT`.
//! - `Deserialize` sets the priority from the message whether or not it is
//!   present (`netclass.cpp:217`), so the priority KiCad holds is always sent.
//!   Every other field is set only when present, on the existing object, so a
//!   message that names one field changes that field alone.
//! - The board's nets keep the effective class they resolved at the last
//!   `BOARD::SynchronizeNetsAndNetClasses` (`board.cpp:2693`), which runs at the
//!   start of every DRC (`drc_engine.cpp:435`) and on every board save
//!   (`files.cpp:1075`). Until one of those runs, KiCad's interactive router and
//!   a live routing export still see the old values.

use crate::mcp::protocol::CallToolResult;
use crate::tools::live_board::{self, LiveBoard};
use crate::tools::ToolContext;
use konnect_ipc::gen::kiapi;
use konnect_ipc::gen::kiapi::common::project::{NetClass, NetClassBoardSettings, NetClassType};
use serde_json::{json, Value};
use std::path::Path;

/// The first KiCad whose `SetNetClasses` leaves a class it can save (module
/// docs): (major, minor, patch).
pub(crate) const FIXED_IN: (u32, u32, u32) = (10, 0, 7);

/// The steps that make a netclass change in KiCad itself, for every refusal
/// that leaves the change to the user.
pub(crate) fn dialog_steps(name: &str) -> String {
    format!(
        "In KiCad: File > Board Setup > Design Rules > Net Classes, set the values of '{name}' \
         in its row (add the row if it is new), press OK, then File > Save. Then call \
         get_netclasses: 'kicad_unsaved' is empty once the file has KiCad's values."
    )
}

/// The netclass settings this module reads and writes, by the key KiCad's
/// project file uses for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Field {
    Clearance,
    TrackWidth,
    ViaDiameter,
    ViaDrill,
    DiffPairWidth,
    DiffPairGap,
    DiffPairViaGap,
}

/// Every [`Field`], in the order results list them.
pub(crate) const ALL_FIELDS: [Field; 7] = [
    Field::Clearance,
    Field::TrackWidth,
    Field::ViaDiameter,
    Field::ViaDrill,
    Field::DiffPairWidth,
    Field::DiffPairGap,
    Field::DiffPairViaGap,
];

impl Field {
    /// The key in `.kicad_pro`'s `net_settings.classes[]`.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Field::Clearance => "clearance",
            Field::TrackWidth => "track_width",
            Field::ViaDiameter => "via_diameter",
            Field::ViaDrill => "via_drill",
            Field::DiffPairWidth => "diff_pair_width",
            Field::DiffPairGap => "diff_pair_gap",
            Field::DiffPairViaGap => "diff_pair_via_gap",
        }
    }

    /// The value a class holds itself, in nm; `None` when it inherits it.
    /// Via sizes are read as `Deserialize` reads them: the first copper layer's
    /// width and the drill's x diameter (`netclass.cpp:240-247`).
    pub(crate) fn get(self, class: &NetClass) -> Option<i64> {
        let board = class.board.as_ref()?;
        match self {
            Field::Clearance => board.clearance.as_ref().map(|d| d.value_nm),
            Field::TrackWidth => board.track_width.as_ref().map(|d| d.value_nm),
            Field::DiffPairWidth => board.diff_pair_track_width.as_ref().map(|d| d.value_nm),
            Field::DiffPairGap => board.diff_pair_gap.as_ref().map(|d| d.value_nm),
            Field::DiffPairViaGap => board.diff_pair_via_gap.as_ref().map(|d| d.value_nm),
            Field::ViaDiameter => board
                .via_stack
                .as_ref()?
                .copper_layers
                .first()?
                .size
                .as_ref()
                .map(|v| v.x_nm),
            Field::ViaDrill => board
                .via_stack
                .as_ref()?
                .drill
                .as_ref()?
                .diameter
                .as_ref()
                .map(|v| v.x_nm),
        }
    }

    fn set(self, class: &mut NetClass, nm: i64) {
        use kiapi::board::types as board_types;
        let distance = Some(kiapi::common::types::Distance { value_nm: nm });
        let square = Some(kiapi::common::types::Vector2 { x_nm: nm, y_nm: nm });
        let board = class.board.get_or_insert_with(Default::default);
        match self {
            Field::Clearance => board.clearance = distance,
            Field::TrackWidth => board.track_width = distance,
            Field::DiffPairWidth => board.diff_pair_track_width = distance,
            Field::DiffPairGap => board.diff_pair_gap = distance,
            Field::DiffPairViaGap => board.diff_pair_via_gap = distance,
            Field::ViaDiameter => {
                let stack = board.via_stack.get_or_insert_with(Default::default);
                if stack.copper_layers.is_empty() {
                    // The layer `Serialize` writes (`netclass.cpp:160-164`).
                    stack.copper_layers.push(board_types::PadStackLayer {
                        layer: board_types::BoardLayer::BlFCu as i32,
                        shape: board_types::PadStackShape::PssCircle as i32,
                        ..Default::default()
                    });
                }
                stack.copper_layers[0].size = square;
            }
            Field::ViaDrill => {
                let stack = board.via_stack.get_or_insert_with(Default::default);
                stack.drill.get_or_insert_with(Default::default).diameter = square;
            }
        }
    }
}

/// Millimetres as KiCad's integer nanometres.
pub(crate) fn mm_to_nm(mm: f64) -> i64 {
    (mm * 1_000_000.0).round() as i64
}

fn nm_to_mm(nm: i64) -> f64 {
    nm as f64 / 1_000_000.0
}

/// A value given in mm must be one KiCad can hold: finite, within 100 mm, and
/// above zero (a clearance may be zero).
pub(crate) fn check_mm(argument: &str, field: Field, mm: f64) -> Result<(), String> {
    let low_ok = if field == Field::Clearance {
        mm >= 0.0
    } else {
        mm > 0.0
    };
    if mm.is_finite() && low_ok && mm <= 100.0 {
        Ok(())
    } else {
        Err(format!(
            "{argument} must be {} 100 mm; got {mm}",
            if field == Field::Clearance {
                "from 0 to"
            } else {
                "above 0 and at most"
            }
        ))
    }
}

/// The message that makes KiCad's class `name` hold `named` (nm) and changes
/// nothing else in it. A class KiCad does not have yet is new: it takes
/// `new_defaults` for what `named` leaves out, and the priority KiCad's own
/// constructor gives a class (-1, `netclass.cpp:59`), as the file path writes.
pub(crate) fn class_message(
    kicad: &[NetClass],
    name: &str,
    named: &[(Field, i64)],
    new_defaults: &[(Field, i64)],
) -> (NetClass, bool) {
    let existing = kicad.iter().find(|class| class.name == name);
    let mut message = NetClass {
        name: name.to_string(),
        priority: Some(existing.map_or(-1, |class| class.priority.unwrap_or(0))),
        board: Some(NetClassBoardSettings::default()),
        // Never NCT_IMPLICIT, which KiCad skips without a word (module docs).
        r#type: NetClassType::NctExplicit as i32,
        ..Default::default()
    };
    if existing.is_none() {
        for (field, nm) in new_defaults {
            field.set(&mut message, *nm);
        }
    }
    for (field, nm) in named {
        field.set(&mut message, *nm);
    }
    (message, existing.is_some())
}

/// What KiCad holds after the write, checked against what was sent: every
/// named field and the priority of the class, and every other class exactly as
/// before. `Err` names what differs.
pub(crate) fn verify(
    before: &[NetClass],
    after: &[NetClass],
    sent: &NetClass,
    named: &[(Field, i64)],
    new_defaults: &[(Field, i64)],
    existed: bool,
) -> Result<(), String> {
    let Some(class) = after.iter().find(|class| class.name == sent.name) else {
        return Err(format!(
            "KiCad does not list netclass '{}' after the write",
            sent.name
        ));
    };
    let mut expected: Vec<(Field, i64)> = Vec::new();
    if !existed {
        expected.extend_from_slice(new_defaults);
    }
    for (field, nm) in named {
        expected.retain(|(other, _)| other != field);
        expected.push((*field, *nm));
    }
    for (field, nm) in &expected {
        let held = field.get(class);
        if held != Some(*nm) {
            return Err(format!(
                "KiCad holds {} = {} for netclass '{}' after the write, not the {} mm sent",
                field.key(),
                held.map_or("nothing".to_string(), |v| format!("{} mm", nm_to_mm(v))),
                sent.name,
                nm_to_mm(*nm)
            ));
        }
    }
    if class.priority != sent.priority {
        return Err(format!(
            "KiCad holds priority {:?} for netclass '{}' after the write, not the {:?} sent",
            class.priority, sent.name, sent.priority
        ));
    }
    let others = |classes: &[NetClass]| -> Vec<NetClass> {
        let mut rest: Vec<NetClass> = classes
            .iter()
            .filter(|class| class.name != sent.name)
            .cloned()
            .collect();
        rest.sort_by(|a, b| a.name.cmp(&b.name));
        rest
    };
    if others(before) != others(after) {
        return Err(
            "KiCad's other netclasses changed during the write, and this call changed only one: \
             something else edited them at the same time"
                .to_string(),
        );
    }
    Ok(())
}

/// Where KiCad's copy and the project file disagree, per class and field:
/// what KiCad holds and has not saved yet. Classes only one side has are listed
/// with `only_in`. An empty list means the file holds what KiCad holds, for the
/// settings this module knows.
pub(crate) fn unsaved_differences(kicad: &[NetClass], file_classes: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    let file_named = |name: &str| {
        file_classes
            .iter()
            .find(|class| class["name"].as_str() == Some(name))
    };
    for class in kicad {
        let Some(saved) = file_named(&class.name) else {
            out.push(json!({ "netclass": class.name, "only_in": "kicad" }));
            continue;
        };
        // The priority decides which class wins a net; compared where the file
        // writes one (KiCad always does, and so does create_netclass).
        if let Some(written) = saved.get("priority").and_then(Value::as_i64) {
            if class.priority.map(i64::from) != Some(written) {
                out.push(json!({
                    "netclass": class.name,
                    "field": "priority",
                    "kicad": class.priority,
                    "project_file": written,
                }));
            }
        }
        for field in ALL_FIELDS {
            let held = field.get(class);
            let written = saved.get(field.key()).and_then(Value::as_f64).map(mm_to_nm);
            if held != written {
                out.push(json!({
                    "netclass": class.name,
                    "field": field.key(),
                    "kicad_mm": held.map(nm_to_mm),
                    "project_file_mm": written.map(nm_to_mm),
                }));
            }
        }
    }
    for saved in file_classes {
        let name = saved["name"].as_str().unwrap_or_default();
        if !kicad.iter().any(|class| class.name == name) {
            out.push(json!({ "netclass": name, "only_in": "project_file" }));
        }
    }
    out
}

/// The one-line meaning of [`unsaved_differences`], for a result's note.
pub(crate) fn unsaved_note(differences: &[Value]) -> Option<String> {
    (!differences.is_empty()).then(|| {
        format!(
            "KiCad's copy of the project and its project file differ in {} netclass \
             setting(s) (`kicad_unsaved`). Routing reads the file, and KiCad's next save \
             writes its own copy over the file. To keep KiCad's values, save the board in \
             KiCad before routing (File > Save). To keep the file's: for a field that \
             differs, set it again with create_netclass; for a class only one copy has \
             (`only_in`), close and reopen the project in KiCad, which loads the file.",
            differences.len()
        )
    })
}

/// What the IPC write did.
pub(crate) struct Written {
    /// KiCad's classes after the write, as read back.
    pub after: Vec<NetClass>,
    /// Whether KiCad had the class before.
    pub existed: bool,
    /// Whether `SetNetClasses` was sent (not when nothing was named).
    pub sent: bool,
}

/// The outcome of offering a netclass write to a KiCad that may hold the board.
pub(crate) enum LiveWrite {
    /// No KiCad holds this board: the file is the only copy.
    NotHeld,
    /// KiCad holds it, and its copy now has the change, read back.
    Written(Written),
    /// The call must stop here, with this result. Nothing was written to the
    /// file; what KiCad's copy holds is stated in the result.
    Refused(CallToolResult),
}

/// Why a live write did not complete, kept apart from transport errors so that
/// `observe` never mistakes a failed write for a refusal it may write past.
enum LiveFailure {
    /// Nothing was sent to KiCad.
    Before(String),
    /// `SetNetClasses` was sent; KiCad's copy may or may not hold the change.
    After(String),
}

/// Sends the change to KiCad when it holds `board`, and reads it back.
///
/// `schematic_locked` is whether the board's sibling schematic has a KiCad
/// lock file. A schematic editor in another KiCad process holds its own copy of
/// the project and would write it back over this change, so the write goes
/// ahead only when the KiCad holding the board also serves that schematic
/// (one process, one project in memory).
pub(crate) async fn write_if_held(
    ctx: &ToolContext,
    board: &Path,
    name: &str,
    named: Vec<(Field, i64)>,
    new_defaults: Vec<(Field, i64)>,
    schematic_locked: bool,
    file_has_class: bool,
) -> anyhow::Result<LiveWrite> {
    let class_name = name.to_string();
    let schematic_name = board
        .with_extension("kicad_sch")
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let observed = live_board::observe(ctx, board, move |client, _document| {
        Ok(write_live(
            client,
            &class_name,
            &named,
            &new_defaults,
            schematic_locked.then_some(schematic_name.as_str()),
            file_has_class,
        ))
    })
    .await?;
    match observed {
        LiveBoard::Answered(Ok(written)) => Ok(LiveWrite::Written(written)),
        LiveBoard::Answered(Err(LiveFailure::Before(reason))) => {
            Ok(LiveWrite::Refused(CallToolResult::error(format!(
                "KiCad holds this board open, and the netclass was not changed: {reason}. \
                 Nothing was sent to KiCad and the project file was not written. {}",
                dialog_steps(name)
            ))))
        }
        LiveBoard::Answered(Err(LiveFailure::After(reason))) => {
            Ok(LiveWrite::Refused(CallToolResult::error(format!(
                "KiCad holds this board open. The netclass change was sent to KiCad, but it \
                 could not be confirmed: {reason}. The project file was not written. Call \
                 get_netclasses to see what KiCad holds (`kicad_unsaved`) before retrying."
            ))))
        }
        // Not held, or not provably: the established gate decides, exactly as
        // it did before this path existed.
        _ => Ok(
            match crate::tools::pcb_board::refuse_if_board_open_in_kicad(
                ctx,
                board,
                "netclass change",
            )
            .await?
            {
                None => LiveWrite::NotHeld,
                Some(refusal) => LiveWrite::Refused(refusal),
            },
        ),
    }
}

fn write_live(
    client: &konnect_ipc::client::KiCadIpcClient,
    name: &str,
    named: &[(Field, i64)],
    new_defaults: &[(Field, i64)],
    schematic: Option<&str>,
    file_has_class: bool,
) -> Result<Written, LiveFailure> {
    let version = client.get_kicad_version().map_err(|e| {
        LiveFailure::Before(format!("KiCad did not say which version it is ({e:#})"))
    })?;
    // A release candidate of the first fixed version may predate the fix.
    let candidate_of_fix = (version.major, version.minor, version.patch) == FIXED_IN
        && version.full_version.to_ascii_lowercase().contains("rc");
    if (version.major, version.minor, version.patch) < FIXED_IN || candidate_of_fix {
        let (major, minor, patch) = FIXED_IN;
        return Err(LiveFailure::Before(format!(
            "this KiCad is {} ({}.{}.{}), and a KiCad before {major}.{minor}.{patch} leaves a \
             netclass written over its IPC API in a state its next save reads past the end of \
             (fixed in {major}.{minor}.{patch}). Update KiCad, or make the change in KiCad itself",
            version.full_version, version.major, version.minor, version.patch
        )));
    }
    if let Some(schematic) = schematic {
        let served = client
            .get_open_documents_for(konnect_ipc::types::IpcEditorKind::Schematic)
            .map_err(|e| {
                LiveFailure::Before(format!(
                    "the schematic has a KiCad lock, and KiCad could not say whether it holds \
                     that schematic itself ({e:#})"
                ))
            })?;
        // KiCad names an open schematic by its bare file name
        // (`api_handler_sch.cpp:83`), in the field the board answer uses.
        let same_process = served.iter().any(|document| {
            matches!(
                &document.identifier,
                Some(kiapi::common::types::document_specifier::Identifier::BoardFilename(file))
                    if Path::new(file).file_name().is_some_and(|name| name == schematic)
            )
        });
        if !same_process {
            return Err(LiveFailure::Before(format!(
                "the schematic {schematic} has a KiCad lock, but the KiCad that holds the board \
                 does not have it open. A schematic editor in another KiCad keeps its own copy of \
                 the project and would write it back over this change. Close that schematic \
                 editor (or open the schematic from the same KiCad as the board) and retry"
            )));
        }
    }
    let before = client
        .get_net_classes()
        .map_err(|e| LiveFailure::Before(format!("KiCad did not list its netclasses ({e:#})")))?;
    let (message, existed) = class_message(&before, name, named, new_defaults);
    // A class one copy has and the other lacks is an unsaved change in KiCad
    // (or a file edited behind it): writing would give the two copies
    // different values for it, and KiCad's next save would keep its own.
    if existed && !file_has_class {
        return Err(LiveFailure::Before(format!(
            "KiCad's copy of the project has netclass '{name}' and the project file does not: \
             KiCad holds a netclass it has not saved. Save the board in KiCad (File > Save) and retry"
        )));
    }
    if !existed && file_has_class {
        return Err(LiveFailure::Before(format!(
            "the project file has netclass '{name}' and KiCad's copy of the project does not: \
             the file was edited after KiCad loaded it, or the class was removed in KiCad and not \
             saved. To keep the file's class, close and reopen the project in KiCad, then retry; \
             a save in KiCad now would remove it from the file"
        )));
    }
    // Nothing named on a class both copies have: nothing to send.
    if named.is_empty() && existed {
        return Ok(Written {
            after: before,
            existed,
            sent: false,
        });
    }
    client
        .merge_net_classes(vec![message.clone()])
        .map_err(|e| LiveFailure::After(format!("KiCad did not accept SetNetClasses ({e:#})")))?;
    let after = client.get_net_classes().map_err(|e| {
        LiveFailure::After(format!(
            "KiCad did not list its netclasses after the write ({e:#})"
        ))
    })?;
    verify(&before, &after, &message, named, new_defaults, existed).map_err(LiveFailure::After)?;
    Ok(Written {
        after,
        existed,
        sent: true,
    })
}

/// Whether KiCad answers that it holds `board` open. Only a proof counts;
/// every other answer is `false`, for the caller's own gate to decide.
pub(crate) async fn held_by_kicad(ctx: &ToolContext, board: &Path) -> anyhow::Result<bool> {
    Ok(matches!(
        live_board::observe(ctx, board, |_, _| Ok(())).await?,
        LiveBoard::Answered(())
    ))
}

/// The refusal of a netclass-pattern write while KiCad holds the board, with
/// the steps that make the same change in KiCad itself. KiCad 10's IPC API has
/// no netclass-pattern command (`api/proto/common/commands/project_commands.proto`
/// has only `GetNetClasses` and `SetNetClasses`, and its `NetClass` message
/// carries no patterns), so the change cannot be sent, and the file alone would
/// be overwritten by KiCad's next save.
pub(crate) fn pattern_refusal(net: &str, netclass: &str) -> CallToolResult {
    CallToolResult::error(format!(
        "KiCad holds this board open, and KiCad 10's IPC API has no command for netclass \
         patterns, so this assignment cannot reach KiCad's copy of the project; written to the \
         file alone, KiCad's next save would remove it. Nothing was written. Make it in KiCad: \
         File > Board Setup > Design Rules > Net Classes, under 'Netclass Assignments' add a row \
         with Pattern '{net}' and Net Class '{netclass}', press OK, then File > Save. Then call \
         get_netclasses: the pattern is listed once KiCad has saved the project."
    ))
}

/// KiCad's netclasses for a read, or `None` when KiCad cannot list them.
pub(crate) fn read_kicad_classes(
    client: &konnect_ipc::client::KiCadIpcClient,
) -> Option<Vec<NetClass>> {
    client.get_net_classes().ok()
}

/// The facts a live write's result carries, as JSON.
pub(crate) fn live_json(written: &Written, differences: &[Value]) -> Value {
    json!({
        "kicad_copy": match (written.sent, written.existed) {
            (false, _) => "unchanged",
            (true, true) => "updated",
            (true, false) => "created",
        },
        "board_setup_open": "If KiCad's Board Setup was open, close it with Cancel: OK writes the dialog's own copy of every netclass over this change.",
        "read_back": if written.sent { "kicad_netclasses_match" } else { "not_needed" },
        "board_nets_resync": "at_next_drc_or_save",
        "kicad_unsaved": differences,
    })
}

#[cfg(test)]
#[path = "netclass_live_tests.rs"]
mod tests;
