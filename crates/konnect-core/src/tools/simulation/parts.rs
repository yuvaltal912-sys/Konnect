//! The parts that need a model, grouped by what they are.
//!
//! A board repeats parts: thirty fuses of one type need one model. So each
//! group is one kind of part, with the references that use it, the library's
//! pins and its own `Sim.Pins`, and what the project says of the part. All of
//! that is project text, so it is bounded and never resolved outside the
//! project: a datasheet path is reported as a file only when it is a plain
//! file inside the project.

use super::Design;
use crate::simulation::kicad_netlist::{Component, LibraryPart};
use crate::simulation::preflight::Preflight;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::path::Path;

const MAX_TEXT: usize = 200;
const MAX_URL: usize = 500;
const DOCUMENT_EXTENSIONS: [&str; 5] = ["pdf", "htm", "html", "txt", "md"];

/// Project text, cut short and with every control character made a space.
fn bounded(text: &str, max: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max)
        .collect()
}

/// A datasheet as the project names it: a URL, a document inside the project,
/// or text this does not resolve.
fn datasheet(value: &str, project_dir: &Path) -> Value {
    let value = value.trim();
    if value.starts_with("https://") || value.starts_with("http://") {
        return json!({ "url": bounded(value, MAX_URL) });
    }
    let relative = ["${KIPRJMOD}/", "$(KIPRJMOD)/"]
        .iter()
        .find_map(|prefix| value.strip_prefix(prefix))
        .or_else(|| {
            let plain = !value.starts_with(['/', '\\', '~', '$'])
                && !value.contains(':')
                && !value.split(['/', '\\']).any(|part| part == "..");
            plain.then_some(value)
        });
    let inside = relative.and_then(|relative| {
        let root = project_dir.canonicalize().ok()?;
        let file = root.join(relative).canonicalize().ok()?;
        let document = file
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                DOCUMENT_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
            });
        let plain = std::fs::symlink_metadata(&file).is_ok_and(|meta| meta.is_file());
        (file.starts_with(&root) && document && plain).then(|| {
            file.strip_prefix(&root)
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default()
        })
    });
    match inside {
        Some(file) => json!({ "project_file": file }),
        None => json!({ "text": bounded(value, MAX_TEXT) }),
    }
}

struct Group<'a> {
    reason: &'static str,
    component: &'a Component,
    references: Vec<String>,
    sheet_files: BTreeSet<String>,
}

/// What makes two parts the same part.
fn same_part(a: &Component, b: &Component) -> bool {
    a.lib_id == b.lib_id
        && a.value == b.value
        && a.part_fields == b.part_fields
        && a.datasheet == b.datasheet
}

/// Every part that needs a model: those KiCad leaves unmodelled, and those it
/// runs on a default model that says nothing about the part.
pub(super) fn report(design: &Design, preflight: &Preflight, limit: usize) -> Value {
    let mut needing: Vec<(String, &'static str)> = preflight
        .unmodelled_symbols
        .iter()
        .map(|symbol| (symbol.reference.clone(), "no_model"))
        .collect();
    let defaults = preflight
        .warnings
        .iter()
        .filter(|warning| warning.code == "generic_default_model")
        .flat_map(|warning| warning.items.iter());
    for item in defaults {
        // KiCad names a default model after its part: `__D3` is D3's.
        let reference = item.strip_prefix("__").unwrap_or(item);
        let known = design.components.iter().any(|c| c.reference == reference);
        if known && !needing.iter().any(|(seen, _)| seen == reference) {
            needing.push((reference.to_string(), "kicad_default_model"));
        }
    }
    let mut groups: Vec<Group<'_>> = Vec::new();
    for (reference, reason) in &needing {
        let Some(component) = design.components.iter().find(|c| &c.reference == reference) else {
            continue;
        };
        let sheet = component
            .sheet_file
            .as_deref()
            .map(|sheet| bounded(sheet, MAX_TEXT));
        match groups
            .iter_mut()
            .find(|group| group.reason == *reason && same_part(group.component, component))
        {
            Some(group) => {
                group.references.push(reference.clone());
                group.sheet_files.extend(sheet);
            }
            None => groups.push(Group {
                reason,
                component,
                references: vec![reference.clone()],
                sheet_files: sheet.into_iter().collect(),
            }),
        }
    }
    groups.sort_by(|a, b| {
        b.references
            .len()
            .cmp(&a.references.len())
            .then_with(|| a.references[0].cmp(&b.references[0]))
    });
    let total = groups.len();
    let items: Vec<Value> = groups
        .iter()
        .take(limit)
        .map(|group| entry(group, design, limit))
        .collect();
    json!({
        "total": total,
        "shown": items.len(),
        "truncated": items.len() < total,
        "items": items,
    })
}

fn entry(group: &Group<'_>, design: &Design, limit: usize) -> Value {
    let component = group.component;
    let library: Option<&LibraryPart> = design
        .library_parts
        .iter()
        .find(|part| component.lib_id.as_deref() == Some(part.lib_id.as_str()));
    let part_fields: Map<String, Value> = component
        .part_fields
        .iter()
        .map(|(name, value)| (bounded(name, 40), json!(bounded(value, MAX_TEXT))))
        .collect();
    let pins = library.map_or(&[][..], |part| part.pins.as_slice());
    json!({
        "reason": group.reason,
        "lib_id": component.lib_id.as_deref().map(|lib_id| bounded(lib_id, MAX_TEXT)),
        "value": bounded(&component.value, MAX_TEXT),
        "part_fields": part_fields,
        "datasheet": component
            .datasheet
            .as_deref()
            .map(|value| datasheet(value, &design.project_dir)),
        "description": component.description.as_deref().map(|text| bounded(text, MAX_TEXT)),
        "keywords": component.keywords.as_deref().map(|text| bounded(text, MAX_TEXT)),
        "reference_count": group.references.len(),
        "references": group.references.iter().take(limit).collect::<Vec<_>>(),
        "sheet_files": group.sheet_files,
        "library_pin_count": pins.len(),
        "library_pins": pins.iter().take(limit).collect::<Vec<_>>(),
        "library_sim_pins": library
            .and_then(|part| part.sim_pins.as_deref())
            .map(|pins| bounded(pins, MAX_TEXT)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_datasheet_is_a_file_only_inside_the_project() {
        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join("datasheets")).unwrap();
        std::fs::write(project.path().join("datasheets/bat54.pdf"), "%PDF").unwrap();
        std::fs::write(project.path().join("notes.key"), "x").unwrap();
        std::fs::write(outside.path().join("secret.pdf"), "%PDF").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            outside.path().join("secret.pdf"),
            project.path().join("datasheets/link.pdf"),
        )
        .unwrap();
        let dir = project.path();
        assert_eq!(
            datasheet("${KIPRJMOD}/datasheets/bat54.pdf", dir),
            json!({ "project_file": "datasheets/bat54.pdf" })
        );
        assert_eq!(
            datasheet("datasheets/bat54.pdf", dir),
            json!({ "project_file": "datasheets/bat54.pdf" })
        );
        assert_eq!(
            datasheet("https://x.test/a.pdf", dir),
            json!({ "url": "https://x.test/a.pdf" })
        );
        let outside_path = outside.path().join("secret.pdf").display().to_string();
        for raw in [
            outside_path.as_str(),
            "../x.pdf",
            "datasheets/../../x.pdf",
            "${HOME}/x.pdf",
            "~/.ssh/id_rsa",
            "file:///etc/hosts",
            "notes.key",
            "datasheets/link.pdf",
            "javascript:alert(1)",
        ] {
            let read = datasheet(raw, dir);
            assert!(
                read.get("project_file").is_none() && read.get("url").is_none(),
                "{raw}: {read}"
            );
        }
    }

    #[test]
    fn project_text_is_bounded_and_has_no_control_characters() {
        let long = format!("a\nb\u{1b}[31m{}", "x".repeat(1000));
        let text = bounded(&long, MAX_TEXT);
        assert_eq!(text.chars().count(), MAX_TEXT);
        assert!(!text.chars().any(char::is_control));
    }
}
