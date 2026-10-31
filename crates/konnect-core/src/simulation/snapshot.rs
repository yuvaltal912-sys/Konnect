//! Copy a saved project into a private folder for `kicad-cli`.
//!
//! `kicad-cli` takes a project lock on every run, strands it if killed, and on
//! Windows holds sheets open without write sharing. Running it on a copy keeps
//! all of that out of the user's project. The copy holds only the sheets and
//! the `.kicad_pro`; KiCad needs no `sym-lib-table` to export a netlist.

use anyhow::{bail, Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const SIM_LIBRARY_PROPERTY: &str = "(property \"Sim.Library\" \"";

/// A sub-sheet the snapshot cannot place inside its private copy.
#[derive(Debug, thiserror::Error)]
#[error("sheet {} is outside the project folder {}", sheet.display(), project_dir.display())]
pub(crate) struct SheetOutsideProject {
    sheet: PathBuf,
    project_dir: PathBuf,
}

#[derive(Debug)]
pub(crate) struct Snapshot {
    /// The copied root sheet, for `kicad-cli`.
    pub root_sheet: PathBuf,
    /// Editor locks seen on the live project. They are disclosed, not refused:
    /// the snapshot reads saved files, so a lock means they may be stale.
    pub locks_present: Vec<PathBuf>,
}

/// Copy `root` and its sub-sheets into `into`, keeping their relative layout.
/// Reads the project only; nothing is written beside it.
pub(crate) fn take_snapshot(root: &Path, into: &Path) -> Result<Snapshot> {
    let project_dir = root
        .canonicalize()
        .with_context(|| format!("cannot resolve schematic {}", root.display()))?
        .parent()
        .context("schematic has no parent folder")?
        .to_path_buf();
    let root_name = root.file_name().context("schematic has no file name")?;

    let mut locks_present = Vec::new();
    for sheet in hierarchy_files(&project_dir.join(root_name))? {
        let relative = sheet
            .strip_prefix(&project_dir)
            .map_err(|_| SheetOutsideProject {
                sheet: sheet.clone(),
                project_dir: project_dir.clone(),
            })?
            .to_path_buf();
        let source = std::fs::read_to_string(&sheet)
            .with_context(|| format!("cannot read sheet {}", sheet.display()))?;
        let destination = into.join(&relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&destination, absolute_sim_libraries(&source, &project_dir))
            .with_context(|| format!("cannot copy sheet {}", sheet.display()))?;
        if let Some(lock) = konnect_sexp::writer::kicad_editor_lock_path(&sheet) {
            if lock.exists() {
                locks_present.push(lock);
            }
        }
    }

    let project_file = project_dir.join(root_name).with_extension("kicad_pro");
    if project_file.is_file() {
        let name = project_file
            .file_name()
            .context("project file has no name")?;
        std::fs::copy(&project_file, into.join(name))
            .with_context(|| format!("cannot copy {}", project_file.display()))?;
        let lock = project_dir.join(format!("~{}.lck", name.to_string_lossy()));
        if lock.exists() {
            locks_present.push(lock);
        }
    }

    Ok(Snapshot {
        root_sheet: into.join(root_name),
        locks_present,
    })
}

fn hierarchy_files(root: &Path) -> Result<Vec<PathBuf>> {
    fn visit(
        path: &Path,
        seen: &mut HashSet<PathBuf>,
        active: &mut HashSet<PathBuf>,
        files: &mut Vec<PathBuf>,
    ) -> Result<()> {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("cannot resolve sheet {}", path.display()))?;
        if active.contains(&canonical) {
            bail!(
                "sheet hierarchy contains a cycle at {}",
                canonical.display()
            );
        }
        if !seen.insert(canonical.clone()) {
            return Ok(());
        }
        active.insert(canonical.clone());
        let schematic = konnect_schematic_editor::Schematic::load(&canonical)
            .with_context(|| format!("cannot load sheet {}", canonical.display()))?;
        files.push(canonical.clone());
        let parent = canonical.parent().unwrap_or_else(|| Path::new("."));
        for sheet in schematic.sheets.iter() {
            visit(&parent.join(sheet.file()), seen, active, files)?;
        }
        active.remove(&canonical);
        Ok(())
    }

    let mut files = Vec::new();
    visit(root, &mut HashSet::new(), &mut HashSet::new(), &mut files)?;
    Ok(files)
}

/// Rewrite project-relative `Sim.Library` values to live absolute paths, so
/// KiCad's resolver finds model files the snapshot does not carry. 15 of
/// KiCad's 18 simulation demos exit 2 without this.
fn absolute_sim_libraries(source: &str, project_dir: &Path) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find(SIM_LIBRARY_PROPERTY) {
        let value_start = start + SIM_LIBRARY_PROPERTY.len();
        out.push_str(&rest[..value_start]);
        rest = &rest[value_start..];
        let Some(end) = closing_quote(rest) else {
            break;
        };
        let raw = &rest[..end];
        match live_library_path(&unescape(raw), project_dir) {
            Some(path) => out.push_str(&escape(&path.to_string_lossy())),
            None => out.push_str(raw),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn live_library_path(value: &str, project_dir: &Path) -> Option<PathBuf> {
    if value.is_empty() {
        return None;
    }
    for variable in ["${KIPRJMOD}", "$(KIPRJMOD)"] {
        if let Some(relative) = value.strip_prefix(variable) {
            return Some(project_dir.join(relative.trim_start_matches(['/', '\\'])));
        }
    }
    // Other variables are KiCad's own, and survive the move.
    if value.starts_with('$') || Path::new(value).is_absolute() {
        return None;
    }
    Some(project_dir.join(value))
}

fn closing_quote(text: &str) -> Option<usize> {
    let mut escaped = false;
    for (index, c) in text.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => return Some(index),
            _ => {}
        }
    }
    None
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            _ => out.push(c),
        }
    }
    out
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECT: &str = "/live/project";

    fn rewrite(value: &str) -> String {
        let source = format!("(property \"Sim.Library\" \"{value}\" (at 0 0 0))");
        absolute_sim_libraries(&source, Path::new(PROJECT))
    }

    fn sheet(children: &[(&str, &str)]) -> String {
        let sheets: String = children
            .iter()
            .map(|(name, file)| {
                format!(
                    "(sheet (at 0 0) (size 10 10) (uuid \"00000000-0000-0000-0000-000000000002\") \
                     (property \"Sheetname\" \"{name}\" (at 0 0 0)) \
                     (property \"Sheetfile\" \"{file}\" (at 0 0 0)))"
                )
            })
            .collect();
        format!(
            "(kicad_sch (version 20250114) (generator \"eeschema\") \
             (uuid \"00000000-0000-0000-0000-000000000001\") (paper \"A4\") {sheets})"
        )
    }

    #[test]
    fn a_bare_library_name_becomes_a_live_path() {
        assert!(rewrite("diode.mod").contains("/live/project/diode.mod\" (at"));
    }

    #[test]
    fn a_project_variable_becomes_a_live_path() {
        assert!(rewrite("${KIPRJMOD}/models/a.lib").contains("/live/project/models/a.lib\""));
        assert!(rewrite("$(KIPRJMOD)/a.lib").contains("/live/project/a.lib\""));
    }

    #[test]
    fn other_variables_absolute_paths_and_blanks_are_left_alone() {
        for value in [
            "${KICAD10_SYMBOL_DIR}/Simulation_SPICE.sp",
            "/abs/a.lib",
            "",
        ] {
            assert!(
                rewrite(value).contains(&format!("\"{value}\" (at")),
                "{value}"
            );
        }
    }

    #[test]
    fn every_library_in_a_sheet_is_rewritten_and_the_rest_is_untouched() {
        let source = "(a) (property \"Sim.Library\" \"x.lib\") (b) \
                      (property \"Sim.Library\" \"y.lib\") (property \"Sim.Name\" \"z.lib\")";
        let out = absolute_sim_libraries(source, Path::new(PROJECT));
        assert!(out.contains("/live/project/x.lib"));
        assert!(out.contains("/live/project/y.lib"));
        assert!(out.contains("(property \"Sim.Name\" \"z.lib\")"));
        assert!(out.starts_with("(a) "));
    }

    #[test]
    fn quotes_in_values_survive_the_rewrite() {
        let out = absolute_sim_libraries(
            "(property \"Sim.Library\" \"a\\\"b.lib\")",
            Path::new("/p\"q"),
        );
        assert!(out.contains("/p\\\"q/a\\\"b.lib"), "{out}");
    }

    #[test]
    fn an_unterminated_value_is_returned_unchanged() {
        let source = "(property \"Sim.Library\" \"never closed";
        assert_eq!(absolute_sim_libraries(source, Path::new(PROJECT)), source);
    }

    #[test]
    fn a_hierarchy_keeps_its_layout_and_the_project_stays_untouched() {
        let project = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join("sub")).unwrap();
        let root = project.path().join("top.kicad_sch");
        std::fs::write(&root, sheet(&[("Child", "sub/child.kicad_sch")])).unwrap();
        std::fs::write(project.path().join("sub/child.kicad_sch"), sheet(&[])).unwrap();
        std::fs::write(project.path().join("top.kicad_pro"), "{}").unwrap();
        let before: Vec<_> = std::fs::read_dir(project.path())
            .unwrap()
            .flatten()
            .collect();

        let snapshot = take_snapshot(&root, out.path()).unwrap();

        assert_eq!(snapshot.root_sheet, out.path().join("top.kicad_sch"));
        assert!(out.path().join("sub/child.kicad_sch").is_file());
        assert!(out.path().join("top.kicad_pro").is_file());
        assert!(snapshot.locks_present.is_empty());
        let after: Vec<_> = std::fs::read_dir(project.path())
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(before.len(), after.len());
    }

    #[test]
    fn a_loose_schematic_has_no_project_file_to_copy() {
        let project = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let root = project.path().join("loose.kicad_sch");
        std::fs::write(&root, sheet(&[])).unwrap();
        take_snapshot(&root, out.path()).unwrap();
        assert!(!out.path().join("loose.kicad_pro").exists());
    }

    #[test]
    fn locks_are_reported_and_do_not_stop_the_copy() {
        let project = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let root = project.path().join("top.kicad_sch");
        std::fs::write(&root, sheet(&[])).unwrap();
        std::fs::write(project.path().join("top.kicad_pro"), "{}").unwrap();
        std::fs::write(project.path().join("~top.kicad_sch.lck"), "{}").unwrap();
        std::fs::write(project.path().join("~top.kicad_pro.lck"), "{}").unwrap();

        let snapshot = take_snapshot(&root, out.path()).unwrap();
        assert_eq!(snapshot.locks_present.len(), 2);
        assert!(!out.path().join("~top.kicad_pro.lck").exists());
    }

    #[test]
    fn a_sheet_outside_the_project_is_refused() {
        let base = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        std::fs::create_dir(base.path().join("project")).unwrap();
        let root = base.path().join("project/top.kicad_sch");
        std::fs::write(&root, sheet(&[("Shared", "../shared.kicad_sch")])).unwrap();
        std::fs::write(base.path().join("shared.kicad_sch"), sheet(&[])).unwrap();
        let error = take_snapshot(&root, out.path()).unwrap_err();
        assert!(
            format!("{error:#}").contains("outside the project"),
            "{error:#}"
        );
    }

    #[test]
    fn a_missing_sheet_is_an_error() {
        let project = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let root = project.path().join("top.kicad_sch");
        std::fs::write(&root, sheet(&[("Gone", "gone.kicad_sch")])).unwrap();
        assert!(take_snapshot(&root, out.path()).is_err());
    }

    #[test]
    fn a_sheet_cycle_is_an_error() {
        let project = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let root = project.path().join("a.kicad_sch");
        std::fs::write(&root, sheet(&[("B", "b.kicad_sch")])).unwrap();
        std::fs::write(
            project.path().join("b.kicad_sch"),
            sheet(&[("A", "a.kicad_sch")]),
        )
        .unwrap();
        let error = take_snapshot(&root, out.path()).unwrap_err();
        assert!(format!("{error:#}").contains("cycle"), "{error:#}");
    }

    #[test]
    fn a_reused_sheet_is_copied_once_without_being_a_cycle() {
        let project = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let root = project.path().join("top.kicad_sch");
        std::fs::write(
            &root,
            sheet(&[("A", "same.kicad_sch"), ("B", "same.kicad_sch")]),
        )
        .unwrap();
        std::fs::write(project.path().join("same.kicad_sch"), sheet(&[])).unwrap();
        take_snapshot(&root, out.path()).unwrap();
        assert!(out.path().join("same.kicad_sch").is_file());
    }
}
