//! Saved sidecars and live routing preferences bound to an export revision.
use anyhow::{ensure, Context, Result};
use konnect_ipc::IpcEffectiveRoutingRules;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoutingContext {
    pub project_sha256: Option<String>,
    pub custom_rules_sha256: Option<String>,
    pub effective_rules_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub project: Option<String>,
    pub rules: Option<String>,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    read_optional_with(path, |p| Ok(konnect_sexp::writer::read_consistent(p)?))
}
fn read_optional_with(
    path: &Path,
    read: impl FnOnce(&Path) -> Result<String>,
) -> Result<Option<String>> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
        Ok(meta) => {
            ensure!(
                meta.file_type().is_file(),
                "routing sidecar must be a regular file, not a symlink or directory"
            );
            let source = read(path)?;
            ensure!(source.len() <= 4_194_304, "routing sidecar exceeds 4 MiB");
            ensure!(
                std::fs::symlink_metadata(path)?.file_type().is_file(),
                "routing sidecar changed to a non-regular file"
            );
            Ok(Some(source))
        }
    }
}

/// The netclass IPC command has no document header. Keep its authority
/// unambiguous even when board snapshots themselves can be selected by path.
pub(crate) fn require_single_board(client: &konnect_ipc::KiCadIpcClient) -> Result<()> {
    ensure!(
        client.get_open_board_paths()?.len() == 1,
        "routing export/import requires exactly one PCB open in KiCad"
    );
    Ok(())
}

impl Snapshot {
    pub fn read(board: &Path) -> Result<Self> {
        let project = read_optional(&board.with_extension("kicad_pro"))?;
        let rules = read_optional(&board.with_extension("kicad_dru"))?;
        if let Some(project) = &project {
            crate::tools::verification::project_json_guard::parse(project)
                .context("invalid saved routing project")?;
        }
        if let Some(rules) = &rules {
            crate::custom_rules::parse(rules).context("invalid custom DRC rules")?;
        }
        Ok(Self { project, rules })
    }
    pub fn seal(&self, effective: &IpcEffectiveRoutingRules) -> Result<RoutingContext> {
        Ok(RoutingContext {
            project_sha256: self.project.as_ref().map(|s| digest(s.as_bytes())),
            custom_rules_sha256: self.rules.as_ref().map(|s| digest(s.as_bytes())),
            effective_rules_sha256: digest(&serde_json::to_vec(effective)?),
        })
    }
    pub fn verify_saved(&self, context: Option<&RoutingContext>) -> Result<()> {
        match context {
            Some(context) => {
                ensure!(
                    self.project.as_ref().map(|s| digest(s.as_bytes())) == context.project_sha256,
                    "saved project changed since DSN export; export again"
                );
                ensure!(
                    self.rules.as_ref().map(|s| digest(s.as_bytes()))
                        == context.custom_rules_sha256,
                    "custom DRC rules changed since DSN export; export again"
                );
            }
            None => ensure!(
                self.project.is_none() && self.rules.is_none(),
                "legacy manifest has no project/custom DRC revision; export again"
            ),
        }
        // Refuse a caller-forged manifest that tries to bless an unsupported
        // custom-rule profile just by supplying a matching file hash.
        if let Some(rules) = &self.rules {
            crate::custom_rules::routing_bounds(rules)?;
        }
        Ok(())
    }
    pub fn verify_live(
        &self,
        context: Option<&RoutingContext>,
        effective: &IpcEffectiveRoutingRules,
    ) -> Result<()> {
        self.verify_saved(context)?;
        if let Some(context) = context {
            ensure!(
                self.seal(effective)?.effective_rules_sha256 == context.effective_rules_sha256,
                "live effective routing rules changed since DSN export; export again"
            );
        }
        Ok(())
    }
    pub fn ensure_unchanged(&self, board: &Path) -> Result<()> {
        ensure!(
            &Self::read(board)? == self,
            "routing project or custom rules changed during operation"
        );
        Ok(())
    }
    pub fn project_rules(
        &self,
        effective: &IpcEffectiveRoutingRules,
    ) -> Result<IpcEffectiveRoutingRules> {
        let mut adjusted = effective.clone();
        if let Some(source) = &self.rules {
            let bounds = crate::custom_rules::routing_bounds(source)?;
            for rule in adjusted.values_mut() {
                // Missing live fields remain missing so the existing exporter
                // refuses incomplete evidence instead of inventing defaults.
                ensure!(
                    rule.clearance_mm.is_none_or(|v| v.is_finite() && v >= 0.0),
                    "live clearance is invalid"
                );
                ensure!(
                    rule.track_width_mm.is_none_or(|v| v.is_finite() && v > 0.0),
                    "live width is invalid"
                );
                rule.clearance_mm = rule.clearance_mm.map(|v| v.max(bounds.clearance_mm));
                rule.track_width_mm = rule.track_width_mm.map(|v| v.max(bounds.width_mm));
            }
        }
        Ok(adjusted)
    }
}

/// Never let a caller-chosen candidate basename select unrelated .pro/.dru
/// siblings. Native DRC sees only this captured context in a private directory.
#[derive(Debug)]
pub(crate) struct CandidateDrc {
    pub ignored_check_keys: serde_json::Value,
    pub included_severities: serde_json::Value,
    pub violations: usize,
    pub unconnected_items: usize,
}

pub(crate) fn require_custom_check_coverage(
    report: &serde_json::Value,
    snapshot: &Snapshot,
) -> Result<()> {
    if let Some(source) = &snapshot.rules {
        let ignored = report["ignored_check_keys"]
            .as_array()
            .context("KiCad did not disclose ignored checks")?;
        let ignored = ignored
            .iter()
            .map(|key| key.as_str().context("ignored check key is unreadable"))
            .collect::<Result<Vec<_>>>()?;
        let included = report["included_severities"]
            .as_array()
            .context("KiCad did not disclose included severities")?;
        ensure!(
            ["error", "warning", "exclusion"]
                .iter()
                .all(|key| included.iter().any(|v| v.as_str() == Some(key))),
            "candidate DRC did not include every severity"
        );
        for rule in crate::custom_rules::parse(source)? {
            for constraint in rule.node.find_all("constraint") {
                let key = constraint
                    .get(1)
                    .and_then(konnect_sexp::SexpNode::as_str)
                    .context("custom constraint type missing")?;
                ensure!(
                    !ignored.contains(&key),
                    "required custom-rule check is disabled in the saved project: {key}"
                );
            }
        }
    }
    Ok(())
}

pub(crate) async fn candidate_drc(
    cli: &str,
    board_source: &str,
    snapshot: &Snapshot,
) -> Result<CandidateDrc> {
    let report = crate::tools::verification::drc_rule_writer::target_drc(
        cli,
        board_source,
        snapshot.project.as_deref(),
        snapshot.rules.as_deref().unwrap_or("(version 1)\n"),
    )
    .await?;
    require_custom_check_coverage(&report, snapshot)?;
    Ok(CandidateDrc {
        ignored_check_keys: report["ignored_check_keys"].clone(),
        included_severities: report["included_severities"].clone(),
        violations: report["violations"]
            .as_u64()
            .context("missing candidate violation count")? as usize,
        unconnected_items: report["unconnected_items"]
            .as_u64()
            .context("missing candidate unconnected count")? as usize,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_complete_reports_with_enabled_required_checks_can_validate_custom_routes() {
        let snapshot=Snapshot{project:None,rules:Some("(version 1)(rule \"X\" (constraint clearance (min 0.2mm)) (constraint track_width (min 0.3mm)))".into())};
        let complete = serde_json::json!({"ignored_check_keys":["missing_courtyard"],"included_severities":["error","warning","exclusion"]});
        require_custom_check_coverage(&complete, &snapshot).unwrap();
        for check in ["clearance", "track_width"] {
            let mut r = complete.clone();
            r["ignored_check_keys"] = serde_json::json!([check]);
            assert!(require_custom_check_coverage(&r, &snapshot).is_err());
        }
        for field in ["ignored_check_keys", "included_severities"] {
            let mut r = complete.clone();
            r.as_object_mut().unwrap().remove(field);
            assert!(require_custom_check_coverage(&r, &snapshot).is_err());
        }
        let mut r = complete.clone();
        r["ignored_check_keys"] = serde_json::json!([null]);
        assert!(require_custom_check_coverage(&r, &snapshot).is_err());
        for severity in ["error", "warning", "exclusion"] {
            let mut r = complete.clone();
            r["included_severities"] = serde_json::json!(["error", "warning", "exclusion"]
                .into_iter()
                .filter(|s| *s != severity)
                .collect::<Vec<_>>());
            assert!(require_custom_check_coverage(&r, &snapshot).is_err());
        }
    }
    #[test]
    fn projection_never_weakens_live_values_or_invents_missing_fields() {
        let mut effective = IpcEffectiveRoutingRules::new();
        effective.insert(
            "VCC".into(),
            konnect_ipc::IpcRoutingRules {
                class_name: "Default".into(),
                constituents: vec![],
                track_width_mm: Some(0.5),
                clearance_mm: Some(0.1),
                via_diameter_mm: None,
                via_drill_mm: None,
            },
        );
        let original = effective.clone();
        let snapshot=Snapshot{project:None,rules:Some("(version 1)(rule \"X\" (constraint clearance (min 0.200001mm)) (constraint track_width (min 0.3mm)))".into())};
        let adjusted = snapshot.project_rules(&effective).unwrap();
        assert_eq!(adjusted["VCC"].track_width_mm, Some(0.5));
        assert_eq!(adjusted["VCC"].clearance_mm, Some(0.201));
        assert_eq!(adjusted["VCC"].via_diameter_mm, None);
        assert_eq!(effective, original);
        for (width, clearance) in [
            (Some(-1.0), Some(0.2)),
            (Some(f64::NAN), Some(0.2)),
            (Some(0.2), Some(-1.0)),
            (Some(0.2), Some(f64::INFINITY)),
        ] {
            let mut bad = original.clone();
            let rule = bad.get_mut("VCC").unwrap();
            rule.track_width_mm = width;
            rule.clearance_mm = clearance;
            assert!(snapshot.project_rules(&bad).is_err());
        }

        effective.get_mut("VCC").unwrap().track_width_mm = Some(0.1);
        effective.get_mut("VCC").unwrap().clearance_mm = Some(0.7);
        let projected = snapshot.project_rules(&effective).unwrap();
        assert_eq!(projected["VCC"].track_width_mm, Some(0.3));
        assert_eq!(projected["VCC"].clearance_mm, Some(0.7));
        effective.get_mut("VCC").unwrap().track_width_mm = None;
        assert_eq!(
            snapshot.project_rules(&effective).unwrap()["VCC"].track_width_mm,
            None
        );
    }
    #[tokio::test]
    #[ignore = "requires real KiCad CLI"]
    async fn native_candidate_drc_uses_captured_rules_and_refuses_disabled_checks() {
        let cli = "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli";
        let board = include_str!("../tests/fixtures/specctra_two_resistors.kicad_pcb");
        let rules = "(version 1)(rule \"CapturedClearance\" (constraint clearance (min 2mm)))";
        let mut project: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/multichannel_mixer.kicad_pro"
        ))
        .unwrap();
        project["board"]["design_settings"]["rule_severities"]["clearance"] =
            serde_json::json!("error");
        let snapshot = Snapshot {
            project: Some(project.to_string()),
            rules: Some(rules.into()),
        };
        let report = candidate_drc(cli, board, &snapshot).await.unwrap();
        let baseline = candidate_drc(
            cli,
            board,
            &Snapshot {
                project: snapshot.project.clone(),
                rules: None,
            },
        )
        .await
        .unwrap();
        assert!(
            report.violations > baseline.violations,
            "captured custom clearance must change the native result"
        );
        project["board"]["design_settings"]["rule_severities"]["clearance"] =
            serde_json::json!("ignore");
        let snapshot = Snapshot {
            project: Some(project.to_string()),
            rules: Some(rules.into()),
        };
        let error = candidate_drc(cli, board, &snapshot).await.err().unwrap();
        assert!(error.to_string().contains("disabled"), "{error:#}");
    }
    #[test]
    fn routing_sidecars_are_size_bounded_and_rechecked_after_reading() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("sidecar");
        std::fs::write(&path, " ".repeat(4_194_305)).unwrap();
        assert!(read_optional(&path).is_err());
        #[cfg(unix)]
        {
            std::fs::write(&path, "{}").unwrap();
            let other = d.path().join("other");
            std::fs::write(&other, "{}").unwrap();
            let result = read_optional_with(&path, |p| {
                let s = std::fs::read_to_string(p)?;
                std::fs::remove_file(p)?;
                std::os::unix::fs::symlink(&other, p)?;
                Ok(s)
            });
            assert!(result.is_err());
        }
    }
    #[test]
    fn class_authority_requires_exactly_one_open_board() {
        let d = tempfile::tempdir().unwrap();
        let first = d.path().join("a.kicad_pcb");
        let second = d.path().join("b.kicad_pcb");
        std::fs::write(&first, "board").unwrap();
        std::fs::write(&second, "board").unwrap();
        for boards in [
            vec![],
            vec![first.as_path()],
            vec![first.as_path(), second.as_path()],
        ] {
            let count = boards.len();
            let server =
                crate::tools::pcb_board::board_mock::spawn_kicad_holding_boards(&boards, |_| None);
            let client = konnect_ipc::KiCadIpcClient::new(server.address());
            assert_eq!(require_single_board(&client).is_ok(), count == 1);
        }
    }
    fn empty() -> Snapshot {
        Snapshot {
            project: None,
            rules: None,
        }
    }
    #[test]
    fn saved_revision_distinguishes_presence_changes_and_legacy_manifests() {
        let effective = IpcEffectiveRoutingRules::new();
        let original = empty();
        let seal = original.seal(&effective).unwrap();
        original.verify_live(Some(&seal), &effective).unwrap();
        original.verify_saved(None).unwrap();
        let project = Snapshot {
            project: Some("{}".into()),
            rules: None,
        };
        assert!(project.verify_saved(Some(&seal)).is_err());
        assert!(project.verify_saved(None).is_err());
        let rules = Snapshot {
            project: None,
            rules: Some("(version 1)".into()),
        };
        assert!(rules.verify_saved(Some(&seal)).is_err());
        assert!(rules.verify_saved(None).is_err());
        let seal = rules.seal(&effective).unwrap();
        assert!(original.verify_saved(Some(&seal)).is_err());
    }
    #[test]
    fn live_classes_and_unrepresentable_rule_semantics_cannot_be_blessed_by_a_hash() {
        let effective = IpcEffectiveRoutingRules::new();
        let mut seal = empty().seal(&effective).unwrap();
        seal.effective_rules_sha256 = "changed".into();
        assert!(empty().verify_live(Some(&seal), &effective).is_err());
        let snapshot=Snapshot{project:None,rules:Some("(version 1)(rule \"X\" (condition \"A.NetName == 'GND'\") (constraint clearance (min 0.2mm)))".into())};
        assert!(snapshot
            .verify_saved(Some(&snapshot.seal(&effective).unwrap()))
            .is_err());
    }
    #[test]
    fn snapshot_detects_sidecar_changes_without_touching_board_bytes() {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        std::fs::write(&board, "unchanged").unwrap();
        let snap = Snapshot::read(&board).unwrap();
        std::fs::write(board.with_extension("kicad_pro"), "{}").unwrap();
        assert!(snap.ensure_unchanged(&board).is_err());
        let snap = Snapshot::read(&board).unwrap();
        std::fs::write(board.with_extension("kicad_pro"), "{\"x\":1}").unwrap();
        assert!(snap.ensure_unchanged(&board).is_err());
        std::fs::write(board.with_extension("kicad_pro"), "{\"x\":1,\"x\":2}").unwrap();
        assert!(Snapshot::read(&board).is_err());
        std::fs::write(board.with_extension("kicad_pro"), "{}").unwrap();
        std::fs::write(board.with_extension("kicad_dru"), "(version 1) trailing").unwrap();
        assert!(Snapshot::read(&board).is_err());
        assert_eq!(std::fs::read_to_string(&board).unwrap(), "unchanged");
    }
    #[cfg(unix)]
    #[test]
    fn linked_or_nonregular_sidecars_refuse_even_when_the_target_would_parse() {
        let d = tempfile::tempdir().unwrap();
        let board = d.path().join("board.kicad_pcb");
        let file = d.path().join("target");
        std::fs::write(&file, "{}").unwrap();
        let project = board.with_extension("kicad_pro");
        std::os::unix::fs::symlink(&file, &project).unwrap();
        assert!(Snapshot::read(&board).is_err());
        assert!(read_optional_with(&project, |_| panic!(
            "an existing sidecar link must not be followed"
        ))
        .is_err());
        std::fs::remove_file(&project).unwrap();
        std::fs::create_dir(&project).unwrap();
        assert!(Snapshot::read(&board).is_err());
    }
}
