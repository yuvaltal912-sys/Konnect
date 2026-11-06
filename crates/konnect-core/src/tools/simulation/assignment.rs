//! Check a planned model assignment the way KiCad will read it.
//!
//! A field readback proves the text that was written, not the deck KiCad
//! builds from it. So before anything is committed, the planned sheets go into
//! a private copy of the project, `kicad-cli` exports it, and each part must
//! come out as an element of the model, with each model pin on the net of the
//! symbol pin mapped to it. A symbol pin left unmapped must not be wired to
//! another part, because KiCad drops it from the deck in silence.

use super::{cli, export_detail, simulation_failure, EXPORT_LOCK, EXPORT_TIMEOUT};
use crate::mcp::protocol::CallToolResult;
use crate::simulation::kicad_netlist::{self, SpiceNetlist};
use crate::simulation::snapshot;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// What one part must become.
pub(crate) struct Expected<'a> {
    pub reference: &'a str,
    /// Its symbol's pin numbers.
    pub pins: Vec<String>,
}

fn refuse(reason: &'static str, items: Vec<String>, message: String) -> CallToolResult {
    simulation_failure("assignment", reason, None, items, message)
}

/// Export the project with `edited` sheets in place, and confirm each part.
/// `pin_map` pairs a symbol pin number with the model pin it drives, and
/// `ports` lists the model pins in the order an element names its nodes.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn check(
    kicad_cli: &str,
    root: &Path,
    project_dir: &Path,
    edited: &[(PathBuf, String)],
    parts: &[Expected<'_>],
    model_name: &str,
    ports: &[String],
    pin_map: &[(String, String)],
) -> Result<Vec<Value>, CallToolResult> {
    let unreadable = |error: String| {
        refuse(
            "export_failed",
            Vec::new(),
            format!("The planned assignment could not be checked, so nothing was written: {error}"),
        )
    };
    let scratch = tempfile::Builder::new()
        .prefix("konnect-assign-")
        .tempdir()
        .map_err(|error| unreadable(error.to_string()))?;
    let source = scratch.path().join("src");
    std::fs::create_dir(&source).map_err(|error| unreadable(error.to_string()))?;
    let snapshot =
        snapshot::take_snapshot(root, &source).map_err(|error| unreadable(format!("{error:#}")))?;
    for (sheet, content) in edited {
        let relative = sheet
            .strip_prefix(project_dir)
            .map_err(|_| unreadable(format!("{} is outside the project", sheet.display())))?;
        std::fs::write(
            source.join(relative),
            snapshot::absolute_sim_libraries(content, project_dir),
        )
        .map_err(|error| unreadable(error.to_string()))?;
    }
    let netlists = {
        let _export_guard = EXPORT_LOCK.lock().await;
        cli::export_simulation_netlists(
            kicad_cli,
            &snapshot.root_sheet,
            scratch.path(),
            EXPORT_TIMEOUT,
        )
        .await
        .map_err(|error| {
            refuse(
                "export_failed",
                Vec::new(),
                format!(
                    "KiCad could not export the circuit with the model attached, so nothing \
                         was written: {}",
                    export_detail(&format!("{error:#}"))
                ),
            )
        })?
    };
    let kicad =
        kicad_netlist::parse(&netlists.kicad).map_err(|error| unreadable(format!("{error:#}")))?;
    let spice = SpiceNetlist::parse(&netlists.spice);
    let nets = kicad_netlist::net_entries(&kicad, &spice);
    let net_of = |reference: &str, pin: &str| {
        kicad.nets.iter().find(|net| {
            net.pins
                .iter()
                .any(|node| node.reference == reference && node.pin == pin)
        })
    };
    let mut checked = Vec::new();
    for part in parts {
        let reference = part.reference;
        let tokens = spice.element_for(reference).unwrap_or_default();
        let unmodelled =
            matches!(tokens, [name, model] if model.strip_prefix("__") == Some(name.as_str()));
        if tokens.is_empty() || unmodelled {
            return Err(refuse(
                "still_unmodelled",
                vec![reference.to_string()],
                format!("KiCad still leaves {reference} without a model, so nothing was written."),
            ));
        }
        let uses_model = tokens
            .last()
            .is_some_and(|model| model.eq_ignore_ascii_case(model_name));
        if !uses_model || tokens.len() != ports.len() + 2 {
            return Err(refuse(
                "model_not_used",
                vec![reference.to_string()],
                format!(
                    "KiCad wrote {reference} as `{}`, which is not an instance of {model_name} with \
                     {} pins, so nothing was written.",
                    tokens.join(" ").chars().take(200).collect::<String>(),
                    ports.len()
                ),
            ));
        }
        for (index, port) in ports.iter().enumerate() {
            let Some((pin, _)) = pin_map.iter().find(|(_, mapped)| mapped == port) else {
                continue;
            };
            let expected = net_of(reference, pin).and_then(|net| {
                nets.iter()
                    .find(|entry| entry.kicad_name == net.name)
                    .map(|entry| entry.spice_name.clone())
            });
            let node = &tokens[index + 1];
            if expected.as_deref() != Some(node.as_str()) {
                return Err(refuse(
                    "pin_mismatch",
                    vec![reference.to_string()],
                    format!(
                        "KiCad put model pin {port} of {reference} on {node}, not on the net of \
                         symbol pin {pin}, so nothing was written. Check pin_map against the \
                         symbol's pins."
                    ),
                ));
            }
        }
        let mapped_nets: Vec<&str> = pin_map
            .iter()
            .filter_map(|(pin, _)| net_of(reference, pin).map(|net| net.name.as_str()))
            .collect();
        for pin in part
            .pins
            .iter()
            .filter(|pin| !pin_map.iter().any(|(mapped, _)| mapped == *pin))
        {
            let net = net_of(reference, pin);
            // A stacked pin, such as a second cathode on the tab, shares its net
            // with a mapped pin, so nothing is lost when KiCad drops it.
            let stacked = net.is_some_and(|net| mapped_nets.contains(&net.name.as_str()));
            let shared =
                net.is_some_and(|net| net.pins.iter().any(|node| node.reference != reference));
            if shared && !stacked {
                return Err(refuse(
                    "unmapped_pin_connected",
                    vec![format!("{reference}:{pin}")],
                    format!(
                        "Symbol pin {pin} of {reference} is wired to another part, but pin_map \
                         gives it no model pin, and KiCad would drop it from the simulation. \
                         Map it, or use a model with a pin for it."
                    ),
                ));
            }
        }
        // A coupling names its inductors by element, and a fitted inductor is a
        // subcircuit, which a coupling cannot name.
        let coupled = netlists.spice.lines().map(str::trim).any(|line| {
            line.starts_with(['K', 'k'])
                && line.split_whitespace().skip(1).any(|word| {
                    word.eq_ignore_ascii_case(reference)
                        || word.eq_ignore_ascii_case(&format!("L{reference}"))
                })
        });
        if coupled {
            return Err(refuse(
                "coupled_inductor",
                vec![reference.to_string()],
                format!(
                    "{reference} is coupled to another inductor, and a coupling cannot name a \
                     fitted subcircuit, so nothing was written."
                ),
            ));
        }
        checked.push(json!({
            "reference": reference,
            "element": tokens[0],
            "nodes": &tokens[1..=ports.len()],
        }));
    }
    Ok(checked)
}
