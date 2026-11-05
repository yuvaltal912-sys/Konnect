//! Stored input provenance must name original models, not disposable exports.
//!
//! KiCad deletes every `ibis/*.cache` on the next SPICE export. Its exporter
//! names those generated files after the component reference; the KiCad-format
//! netlist names that component's actual IBIS `Sim.Library`.

use crate::simulation::flatten::ModelInput;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const MAX_MODEL_BYTES: u64 = 64 * 1024 * 1024;

/// Candidate original models named by the private sheet snapshot, hashed before
/// the exporter reads them. Only explicitly named, allowed plain files are read.
pub(crate) struct OriginalModels(HashMap<PathBuf, ModelInput>);

impl OriginalModels {
    pub(crate) fn capture(source: &Path, project: &Path, allowed: &[PathBuf]) -> Self {
        fn properties(node: &konnect_sexp::SexpNode, values: &mut Vec<String>) {
            if node.get(0).and_then(|n| n.as_str()) == Some("property")
                && node.get(1).and_then(|n| n.as_str()) == Some("Sim.Library")
            {
                if let Some(value) = node.get(2).and_then(|n| n.as_str()) {
                    values.push(value.to_string());
                }
            }
            if let Some(children) = node.children() {
                for child in children {
                    properties(child, values);
                }
            }
        }
        fn visit(dir: &Path, values: &mut Vec<String>) -> std::io::Result<()> {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    visit(&entry.path(), values)?;
                } else if entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "kicad_sch")
                {
                    if let Ok(root) =
                        konnect_sexp::parse_sexp(&std::fs::read_to_string(entry.path())?)
                    {
                        properties(&root, values);
                    }
                }
            }
            Ok(())
        }
        let mut values = Vec::new();
        if visit(source, &mut values).is_err() {
            return Self(HashMap::new());
        }
        let mut files: Vec<_> = values
            .iter()
            .filter_map(|v| original_library(v, project, allowed))
            .collect();
        files.sort();
        files.dedup();
        let mut remaining = MAX_MODEL_BYTES;
        let mut hashes = HashMap::new();
        let mut cache = ModelHashCache::default();
        let mut seen = HashSet::new();
        for (path, resolved) in files {
            let cap = if seen.contains(&resolved) {
                MAX_MODEL_BYTES
            } else {
                remaining
            };
            if let Some((bytes, sha256)) = cache.fingerprint(&resolved, cap) {
                if seen.insert(resolved.clone()) {
                    remaining -= bytes;
                }
                hashes.insert(
                    path.clone(),
                    ModelInput {
                        path,
                        resolved,
                        bytes,
                        sha256,
                    },
                );
            }
        }
        Self(hashes)
    }
}

/// A listing owns one cache; a later request creates a new one. Only successful
/// bounded reads are cached: a small remaining budget must not poison a later
/// record with more room, and cached lengths still count against each record.
#[derive(Default)]
pub(crate) struct ModelHashCache {
    files: HashMap<PathBuf, (u64, String)>,
    #[cfg(test)]
    reads: usize,
}

impl ModelHashCache {
    fn fingerprint(&mut self, path: &Path, cap: u64) -> Option<(u64, String)> {
        if let Some((bytes, hash)) = self.files.get(path) {
            return (*bytes <= cap).then(|| (*bytes, hash.clone()));
        }
        #[cfg(test)]
        {
            self.reads += 1;
        }
        let bytes = super::read_plain(path, cap).ok()?;
        if bytes.len() as u64 > cap {
            return None;
        }
        let fingerprint = (bytes.len() as u64, format!("{:x}", Sha256::digest(&bytes)));
        self.files.insert(path.to_path_buf(), fingerprint.clone());
        Some(fingerprint)
    }

    fn unchanged(&mut self, models: &[ModelInput]) -> bool {
        let mut remaining = MAX_MODEL_BYTES;
        let mut seen = HashSet::new();
        for model in models {
            if model.path.canonicalize().ok().as_ref() != Some(&model.resolved) {
                return false;
            }
            let cap = if seen.contains(&model.resolved) {
                MAX_MODEL_BYTES
            } else {
                remaining
            };
            let Some((bytes, hash)) = self.fingerprint(&model.resolved, cap) else {
                return false;
            };
            if bytes != model.bytes || hash != model.sha256 {
                return false;
            }
            if seen.insert(model.resolved.clone()) {
                remaining -= bytes;
            }
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct InputProvenance {
    sources_hash: String,
    /// None when origins cannot be proved, including older provenance records
    /// containing only model_files/models_hash and no logical path evidence.
    #[serde(default)]
    model_inputs: Option<Vec<ModelInput>>,
}

impl InputProvenance {
    pub(crate) fn capture(
        sources: Sha256,
        kicad_netlist: &str,
        inlined_models: &[ModelInput],
        project_dir: &Path,
        allowed_roots: &[PathBuf],
        original_models: &OriginalModels,
    ) -> Self {
        Self::capture_with_roots(
            sources,
            kicad_netlist,
            inlined_models,
            project_dir,
            allowed_roots,
            &cache_roots(),
            original_models,
        )
    }

    fn capture_with_roots(
        sources: Sha256,
        kicad_netlist: &str,
        inlined_models: &[ModelInput],
        project_dir: &Path,
        allowed_roots: &[PathBuf],
        cache_roots: &[PathBuf],
        original_models: &OriginalModels,
    ) -> Self {
        let libraries = ibis_libraries(kicad_netlist);
        let mut complete = true;
        let mut models = Vec::new();
        let mut ibis_origins = Vec::new();
        for model in inlined_models {
            let origin = match ibis_reference(&model.resolved, cache_roots) {
                Some(reference) => libraries
                    .get(&reference)
                    .and_then(Option::as_ref)
                    .and_then(|library| original_library(library, project_dir, allowed_roots))
                    .and_then(|(path, resolved)| {
                        original_models
                            .0
                            .get(&path)
                            .filter(|before| before.resolved == resolved)
                            .cloned()
                    })
                    .inspect(|origin| ibis_origins.push(origin.clone())),
                // Do not reopen normal models: these fingerprints describe the
                // exact bytes already consumed by the flattened deck.
                None => Some(model.clone()),
            };
            match origin {
                Some(model) if !models.contains(&model) => models.push(model),
                Some(_) => {}
                None => complete = false,
            }
        }
        // IBIS generation reads originals outside the flattening reader. Its
        // logical path and bytes must still match the pre-export snapshot.
        complete &= ModelHashCache::default().unchanged(&ibis_origins);
        // Original IBIS bytes can be larger than their generated cache. Keep
        // the complete set within the same budget used by later comparisons.
        let mut seen = HashSet::new();
        let mut remaining = MAX_MODEL_BYTES;
        for model in &models {
            if seen.insert(&model.resolved) {
                match remaining.checked_sub(model.bytes) {
                    Some(room) => remaining = room,
                    None => complete = false,
                }
            }
        }
        Self {
            sources_hash: format!("{:x}", sources.finalize()),
            model_inputs: complete.then_some(models),
        }
    }

    #[cfg(test)]
    pub(crate) fn changed_since(&self, sources: Sha256) -> Option<bool> {
        self.changed_since_with_cache(sources, &mut ModelHashCache::default())
    }

    pub(crate) fn changed_since_with_cache(
        &self,
        sources: Sha256,
        cache: &mut ModelHashCache,
    ) -> Option<bool> {
        if format!("{:x}", sources.finalize()) != self.sources_hash {
            return Some(true);
        }
        Some(!cache.unchanged(self.model_inputs.as_ref()?))
    }
}

fn original_library(
    value: &str,
    project: &Path,
    allowed: &[PathBuf],
) -> Option<(PathBuf, PathBuf)> {
    // Embedded/variable-based libraries need KiCad's resolver. Do not guess and
    // then report a false "unchanged" result for a different file.
    if value.is_empty() || value.contains('$') || value.contains("://") {
        return None;
    }
    let path = Path::new(value);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project.join(path)
    };
    let resolved = path.canonicalize().ok()?;
    if !allowed.iter().any(|root| resolved.starts_with(root)) {
        return None;
    }
    Some((path, resolved))
}

fn ibis_libraries(netlist: &str) -> HashMap<String, Option<String>> {
    let mut libraries = HashMap::new();
    let Ok(root) = konnect_sexp::parse_sexp(netlist) else {
        return libraries;
    };
    let Some(components) = root.find("components") else {
        return libraries;
    };
    for component in components.find_all("comp") {
        let Some(reference) = component.find_str("ref") else {
            continue;
        };
        let Some(fields) = component.find("fields") else {
            continue;
        };
        let get = |name| {
            let mut matching = fields
                .find_all("field")
                .into_iter()
                .filter(|field| field.find_str("name") == Some(name));
            let value = matching.next()?.children()?.last()?.as_str()?;
            if matching.next().is_some() {
                None
            } else {
                Some(value)
            }
        };
        if get("Sim.Device") != Some("IBIS") {
            continue;
        }
        let library = get("Sim.Library").map(str::to_string);
        libraries
            .entry(reference.to_string())
            .and_modify(|entry| *entry = None)
            .or_insert(library);
    }
    libraries
}

fn cache_roots() -> Vec<PathBuf> {
    dirs::cache_dir()
        .into_iter()
        .chain(std::env::var_os("KICAD_CACHE_HOME").map(PathBuf::from))
        .flat_map(|base| [base.join("KiCad"), base.join("kicad")])
        .map(|root| root.canonicalize().unwrap_or(root))
        .collect()
}

fn ibis_reference(path: &Path, roots: &[PathBuf]) -> Option<String> {
    // Canonicalize the parent so a missing generated file is still recognized,
    // including macOS /var -> /private/var aliases. No arbitrary path ending in
    // /ibis/*.cache is accepted: it must be inside a configured KiCad cache.
    let path = path
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .and_then(|parent| path.file_name().map(|name| parent.join(name)))
        .unwrap_or_else(|| path.to_path_buf());
    for root in roots {
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let parts: Vec<_> = relative.components().collect();
        if parts.len() != 3 {
            continue;
        }
        let version = parts[0].as_os_str().to_str()?;
        let Some((major, minor)) = version.split_once('.') else {
            continue;
        };
        if major.is_empty()
            || minor.is_empty()
            || !major
                .bytes()
                .chain(minor.bytes())
                .all(|c| c.is_ascii_digit())
            || parts[1].as_os_str() != "ibis"
            || relative.extension().is_none_or(|ext| ext != "cache")
        {
            continue;
        }
        return relative.file_stem()?.to_str().map(str::to_string);
    }
    None
}

pub(crate) fn is_generated_ibis_cache(path: &Path) -> bool {
    ibis_reference(path, &cache_roots()).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NETLIST: &str =
        include_str!("../../../tests/fixtures/simulation/ibis_provenance/ibis.net");
    const MODEL: &str =
        include_str!("../../../tests/fixtures/simulation/ibis_provenance/ibis_v1_1.ibs");

    fn scene() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/simulation/ibis_provenance/ibis.kicad_sch"
            ),
            project.join("ibis.kicad_sch"),
        )
        .unwrap();
        let model = project.join("ibis_v1_1.ibs");
        std::fs::write(&model, MODEL).unwrap();
        let cache_root = dir.path().join("KiCad");
        let cache = cache_root.join("10.0/ibis/U1.cache");
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        std::fs::write(&cache, "generated SPICE; not an original input").unwrap();
        (
            dir,
            project.canonicalize().unwrap(),
            cache_root.canonicalize().unwrap(),
            cache.canonicalize().unwrap(),
        )
    }

    fn originals(project: &Path) -> OriginalModels {
        OriginalModels::capture(project, project, &[project.to_path_buf()])
    }

    fn consumed(paths: &[PathBuf]) -> Vec<ModelInput> {
        paths
            .iter()
            .map(|path| {
                ModelInput::from_bytes(
                    path.clone(),
                    path.canonicalize().unwrap_or_else(|_| path.clone()),
                    &std::fs::read(path).unwrap_or_default(),
                )
            })
            .collect()
    }

    fn capture(project: &Path, cache_root: &Path, cache: &Path) -> InputProvenance {
        InputProvenance::capture_with_roots(
            Sha256::new(),
            NETLIST,
            &consumed(&[cache.to_path_buf()]),
            project,
            &[project.to_path_buf()],
            &[cache_root.to_path_buf()],
            &originals(project),
        )
    }

    #[test]
    fn ibis_original_input_provenance_survives_generated_cache_deletion() {
        let (_dir, project, cache_root, cache) = scene();
        let provenance = capture(&project, &cache_root, &cache);
        assert_eq!(
            provenance.model_inputs.as_ref().unwrap()[0].resolved,
            project.join("ibis_v1_1.ibs")
        );
        assert!(provenance.model_inputs.is_some());
        assert_eq!(provenance.changed_since(Sha256::new()), Some(false));
        std::fs::remove_file(&cache).unwrap();
        assert_eq!(provenance.changed_since(Sha256::new()), Some(false));
        // Another project's export may recreate the same reference's cache.
        std::fs::write(&cache, "unrelated project's generated model").unwrap();
        assert_eq!(provenance.changed_since(Sha256::new()), Some(false));
        std::fs::write(project.join("ibis_v1_1.ibs"), MODEL.replace("5.0V", "4.0V")).unwrap();
        assert_eq!(provenance.changed_since(Sha256::new()), Some(true));
        let mut edited_sheets = Sha256::new();
        edited_sheets.update(b"sheet edit");
        assert_eq!(provenance.changed_since(edited_sheets), Some(true));
    }

    #[test]
    fn unavailable_original_ibis_input_is_unknown_and_never_assumed_unchanged() {
        let (_dir, project, cache_root, cache) = scene();
        let unknown = cache.with_file_name("UNKNOWN.cache");
        for provenance in [
            capture(&project, &cache_root, &unknown),
            InputProvenance::capture_with_roots(
                Sha256::new(),
                NETLIST,
                &consumed(std::slice::from_ref(&cache)),
                &project,
                &[],
                std::slice::from_ref(&cache_root),
                &originals(&project),
            ),
            InputProvenance::capture_with_roots(
                Sha256::new(),
                "broken export",
                &consumed(std::slice::from_ref(&cache)),
                &project,
                std::slice::from_ref(&project),
                std::slice::from_ref(&cache_root),
                &originals(&project),
            ),
        ] {
            assert!(provenance.model_inputs.is_none());
            assert_eq!(provenance.changed_since(Sha256::new()), None);
            let mut changed = Sha256::new();
            changed.update(b"a changed source");
            assert_eq!(provenance.changed_since(changed), Some(true));
        }
        let provenance = capture(&project, &cache_root, &cache);
        std::fs::remove_file(project.join("ibis_v1_1.ibs")).unwrap();
        assert_eq!(provenance.changed_since(Sha256::new()), Some(true));
    }

    #[test]
    fn ordinary_models_and_ibis_lookalikes_are_still_inputs() {
        let (_dir, project, cache_root, _cache) = scene();
        let ordinary = project.join("ibis/U1.cache");
        std::fs::create_dir(ordinary.parent().unwrap()).unwrap();
        std::fs::write(&ordinary, "ordinary user model").unwrap();
        let provenance = capture(&project, &cache_root, &ordinary);
        assert_eq!(
            provenance.model_inputs.as_ref().unwrap()[0].resolved,
            ordinary
        );
        assert_eq!(provenance.changed_since(Sha256::new()), Some(false));
        std::fs::remove_file(ordinary).unwrap();
        assert_eq!(provenance.changed_since(Sha256::new()), Some(true));
    }

    #[test]
    fn original_model_symlinks_and_oversized_files_are_not_read_from_records() {
        let (_dir, project, cache_root, cache) = scene();
        let provenance = capture(&project, &cache_root, &cache);
        let model = project.join("ibis_v1_1.ibs");
        std::fs::File::options()
            .write(true)
            .open(&model)
            .unwrap()
            .set_len(MAX_MODEL_BYTES + 1)
            .unwrap();
        assert_eq!(provenance.changed_since(Sha256::new()), Some(true));
        #[cfg(unix)]
        {
            std::fs::remove_file(&model).unwrap();
            let other = project.join("other.ibs");
            std::fs::write(&other, MODEL).unwrap();
            std::os::unix::fs::symlink(other, &model).unwrap();
            assert_eq!(provenance.changed_since(Sha256::new()), Some(true));
        }
    }

    #[test]
    fn duplicate_ibis_references_do_not_prove_an_original_model() {
        let (_dir, project, cache_root, cache) = scene();
        let netlist = NETLIST.replace("(ref \"U2\")", "(ref \"U1\")");
        assert_ne!(netlist, NETLIST);
        let provenance = InputProvenance::capture_with_roots(
            Sha256::new(),
            &netlist,
            &consumed(&[cache]),
            &project,
            std::slice::from_ref(&project),
            &[cache_root],
            &originals(&project),
        );
        assert_eq!(provenance.changed_since(Sha256::new()), None);
    }

    #[test]
    fn an_ibis_input_changed_during_export_is_not_recorded_as_proven() {
        let (_dir, project, cache_root, cache) = scene();
        let before_export = originals(&project);
        assert_eq!(before_export.0.len(), 1);
        std::fs::write(project.join("ibis_v1_1.ibs"), MODEL.replace("5.0V", "4.0V")).unwrap();
        let provenance = InputProvenance::capture_with_roots(
            Sha256::new(),
            NETLIST,
            &consumed(&[cache]),
            &project,
            std::slice::from_ref(&project),
            &[cache_root],
            &before_export,
        );
        assert_eq!(provenance.changed_since(Sha256::new()), None);
    }

    #[cfg(unix)]
    #[test]
    fn ibis_provenance_detects_retargeted_library_and_parent_symlinks() {
        for parent_link in [false, true] {
            let (_dir, project, cache_root, cache) = scene();
            let library = project.join("ibis_v1_1.ibs");
            for name in ["v1", "v2"] {
                std::fs::create_dir(project.join(name)).unwrap();
                std::fs::write(
                    project.join(name).join("model.ibs"),
                    if name == "v1" {
                        MODEL.to_string()
                    } else {
                        MODEL.replace("5.0V", "4.0V")
                    },
                )
                .unwrap();
            }
            std::fs::remove_file(&library).unwrap();
            let switched = if parent_link {
                let parent = project.join("models");
                std::os::unix::fs::symlink("v1", &parent).unwrap();
                std::os::unix::fs::symlink("models/model.ibs", &library).unwrap();
                parent
            } else {
                std::os::unix::fs::symlink("v1/model.ibs", &library).unwrap();
                library.clone()
            };
            let provenance = capture(&project, &cache_root, &cache);
            let mut hashes = ModelHashCache::default();
            assert_eq!(
                provenance.changed_since_with_cache(Sha256::new(), &mut hashes),
                Some(false)
            );
            std::fs::remove_file(&switched).unwrap();
            std::os::unix::fs::symlink(if parent_link { "v2" } else { "v2/model.ibs" }, &switched)
                .unwrap();
            // Logical resolution must still be checked when the old target's
            // bytes are already cached by another run in the same listing.
            assert_eq!(
                provenance.changed_since_with_cache(Sha256::new(), &mut hashes),
                Some(true)
            );
            assert_eq!(provenance.changed_since(Sha256::new()), Some(true));
        }
    }

    #[test]
    fn ordinary_model_provenance_uses_bytes_consumed_before_a_concurrent_edit() {
        use crate::simulation::flatten::{self, Compatibility, DeckInputs};
        let (_dir, project, cache_root, _cache) = scene();
        let model = project.join("resistor.lib");
        std::fs::write(&model, "R1 out 0 1k\n").unwrap();
        let before = originals(&project);
        let deck = flatten::flatten(&DeckInputs {
            netlist: ".title race\n.include resistor.lib\nV1 out 0 1\n",
            testbench: &[],
            project_dir: &project,
            allowed_roots: std::slice::from_ref(&project),
            compatibility: Compatibility::DEFAULT,
            excluded_elements: &[],
        })
        .unwrap();
        assert!(deck.render().contains("R1 out 0 1k"));
        std::fs::write(&model, "R1 out 0 2k\n").unwrap();
        let provenance = InputProvenance::capture_with_roots(
            Sha256::new(),
            "(export)",
            &deck.inlined_models,
            &project,
            std::slice::from_ref(&project),
            &[cache_root],
            &before,
        );
        assert_eq!(provenance.changed_since(Sha256::new()), Some(true));
        std::fs::write(&model, "R1 out 0 1k\n").unwrap();
        assert_eq!(provenance.changed_since(Sha256::new()), Some(false));
    }

    #[test]
    fn model_hashes_are_shared_only_within_one_request() {
        let (_dir, project, cache_root, _cache) = scene();
        let model = project.join("ordinary.lib");
        std::fs::write(&model, "R1 out 0 1k\n").unwrap();
        let provenance = capture(&project, &cache_root, &model);
        let mut request = ModelHashCache::default();
        for record in [provenance.clone(), provenance.clone()] {
            assert_eq!(
                record.changed_since_with_cache(Sha256::new(), &mut request),
                Some(false)
            );
        }
        assert_eq!(request.reads, 1);
        std::fs::write(&model, "R1 out 0 2k\n").unwrap();
        // A new request gets a fresh cache and notices the edit.
        let mut next_request = ModelHashCache::default();
        assert_eq!(
            provenance.changed_since_with_cache(Sha256::new(), &mut next_request),
            Some(true)
        );
        assert_eq!(next_request.reads, 1);
    }

    #[test]
    fn cached_hashes_respect_each_records_budget_and_failed_reads_do_not_poison_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.lib");
        std::fs::write(&path, b"12345678").unwrap();
        let mut hashes = ModelHashCache::default();
        assert!(hashes.fingerprint(&path, 4).is_none());
        assert!(hashes.fingerprint(&path, 8).is_some());
        assert!(hashes.fingerprint(&path, 4).is_none());
        assert_eq!(hashes.reads, 2);
        let missing = dir.path().join("missing.lib");
        assert!(hashes.fingerprint(&missing, 8).is_none());
        std::fs::write(&missing, b"model").unwrap();
        assert!(hashes.fingerprint(&missing, 8).is_some());

        let mut models = Vec::new();
        for name in ["one.lib", "two.lib"] {
            let path = dir.path().join(name);
            std::fs::File::create(&path)
                .unwrap()
                .set_len(MAX_MODEL_BYTES / 2 + 1)
                .unwrap();
            let (bytes, sha256) = hashes.fingerprint(&path, MAX_MODEL_BYTES).unwrap();
            models.push(ModelInput {
                path: path.clone(),
                resolved: path.canonicalize().unwrap(),
                bytes,
                sha256,
            });
        }
        assert!(
            !hashes.unchanged(&models),
            "cached models must not bypass the aggregate budget"
        );
    }

    #[test]
    fn old_provenance_without_logical_paths_is_unknown_but_changed_sources_are_known() {
        let old = serde_json::json!({
            "sources_hash": format!("{:x}", Sha256::new().finalize()),
            "model_files": ["/old/resolved/model.ibs"],
            "models_hash": "old-combined-hash"
        });
        let provenance: InputProvenance = serde_json::from_value(old).unwrap();
        assert_eq!(provenance.changed_since(Sha256::new()), None);
        let mut changed = Sha256::new();
        changed.update(b"changed sheet");
        assert_eq!(provenance.changed_since(changed), Some(true));
    }

    #[test]
    fn combined_original_and_inlined_model_budget_is_unknown_at_capture() {
        let (_dir, project, cache_root, cache) = scene();
        let before = originals(&project);
        let mut models = consumed(&[cache]);
        // The exact ordinary digest is irrelevant here: its recorded size plus
        // the independently read IBIS original already exceeds the read budget.
        models.push(ModelInput {
            path: project.join("large.lib"),
            resolved: project.join("large.lib"),
            bytes: MAX_MODEL_BYTES,
            sha256: "consumed-fingerprint".to_string(),
        });
        let provenance = InputProvenance::capture_with_roots(
            Sha256::new(),
            NETLIST,
            &models,
            &project,
            std::slice::from_ref(&project),
            &[cache_root],
            &before,
        );
        assert_eq!(provenance.changed_since(Sha256::new()), None);
    }
}
