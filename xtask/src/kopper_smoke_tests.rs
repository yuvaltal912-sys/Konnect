//! The smoke's verdicts on canned answers, and its staging; no binary is started.

use super::*;
use std::os::unix::fs::PermissionsExt;

fn package(dir: &Path) -> PathBuf {
    let prefix = dir.join("1-kopper");
    std::fs::create_dir_all(prefix.join("bin")).unwrap();
    for name in ["konnect", "fastroute"] {
        let path = prefix.join("bin").join(name);
        std::fs::write(&path, format!("{name} stand-in")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let sha = |name: &str| kopper_package::sha256_of(&prefix.join("bin").join(name)).unwrap();
    let manifest = json!({
        "name": kopper_package::NAME,
        "konnect": {"sha256": sha("konnect")},
        "fastroute": {"sha256": sha("fastroute"), "build": "x18"},
    });
    std::fs::write(prefix.join(MANIFEST), manifest.to_string()).unwrap();
    prefix
}

fn manifest(prefix: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(prefix.join(MANIFEST)).unwrap()).unwrap()
}

fn passes(checks: &[Check]) -> Vec<bool> {
    checks.iter().map(|check| check.pass).collect()
}

#[test]
fn a_package_whose_binary_changed_fails() {
    let dir = tempfile::tempdir().unwrap();
    let prefix = package(dir.path());
    assert_eq!(passes(&package_checks(&prefix)), [true, true, true]);
    std::fs::write(prefix.join("bin/fastroute"), "another build").unwrap();
    assert_eq!(passes(&package_checks(&prefix)), [true, true, false]);
    assert_eq!(passes(&package_checks(&dir.path().join("none"))), [false]);
}

#[test]
fn simulation_listed_anywhere_fails() {
    let simulation = konnect_core::router::registry::tools_for("simulation").unwrap()[0].name;
    let list = |names: &[&str]| json!({"tools": names.iter().map(|n| json!({"name": n})).collect::<Vec<_>>()});
    assert!(tools_list_check(&list(&["route_trace", "run_drc"])).pass);
    assert!(!tools_list_check(&list(&["route_trace", simulation])).pass);
    assert!(
        !tools_list_check(&list(&[])).pass,
        "an empty list proves nothing"
    );

    let boxes = |names: &[&str]| json!({"toolsets": names.iter().map(|n| json!({"name": n})).collect::<Vec<_>>()});
    assert!(toolboxes_check(&boxes(&["project", "pcb_routing"])).pass);
    assert!(!toolboxes_check(&boxes(&["project", "simulation"])).pass);
    assert!(!toolboxes_check(&json!({})).pass);
}

#[test]
fn the_server_must_be_the_package_and_carry_no_simulation_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let prefix = package(dir.path());
    let exe = prefix.join("bin/konnect");
    let info = |path: &Path, ngspice: &str| json!({"runtime": {"executable_path": path}, "ngspice": {"probe_status": ngspice}});
    assert_eq!(
        passes(&installation_checks(&info(&exe, "not_available"), &exe)),
        [true, true]
    );
    assert_eq!(
        passes(&installation_checks(
            &info(&prefix.join("bin/fastroute"), "ready"),
            &exe
        )),
        [false, false]
    );
}

#[test]
fn a_route_with_another_fastroute_or_a_failed_gate_fails() {
    let dir = tempfile::tempdir().unwrap();
    let prefix = package(dir.path());
    let manifest = manifest(&prefix);
    let ses = dir.path().join("board.ses");
    std::fs::write(&ses, "(session board)").unwrap();
    let body = |source: &str, path: &Path, sha: &Value, gates: bool| {
        json!({
            "success": true,
            "status": if gates { "complete" } else { "complete_with_gate_failures" },
            "engine": {"name": "fastroute", "source": source, "path": path, "sha256": sha},
            "completion": {"judged": {"all_gates_pass": gates, "unconnected": 0}},
        })
    };
    let bundled = prefix.join("bin/fastroute");
    let sha = &manifest["fastroute"]["sha256"];
    let wait = Duration::from_secs(1);
    let good = route_checks(
        false,
        &body("bundled", &bundled, sha, true),
        &prefix,
        &manifest,
        &ses,
        wait,
    );
    assert_eq!(passes(&good), [true, true, true, true, true]);

    let other = dir.path().join("other-fastroute");
    std::fs::write(&other, "x").unwrap();
    for (source, path, sha) in [
        ("default", &bundled, sha.clone()),
        ("bundled", &other, sha.clone()),
        ("bundled", &bundled, json!("0".repeat(64))),
    ] {
        let checks = route_checks(
            false,
            &body(source, path, &sha, true),
            &prefix,
            &manifest,
            &ses,
            wait,
        );
        assert_eq!(
            passes(&checks),
            [true, false, true, true, true],
            "{source} {path:?}"
        );
    }
    let failed_gate = route_checks(
        false,
        &body("bundled", &bundled, sha, false),
        &prefix,
        &manifest,
        &ses,
        wait,
    );
    assert_eq!(passes(&failed_gate), [true, true, true, false, false]);
    // Every gate passed, but the status is not `complete`: the router's count
    // standing over the judge's verdict (fixed with D21) fails the smoke.
    let mut counted = body("bundled", &bundled, sha, true);
    counted["status"] = json!("incomplete");
    let counted = route_checks(false, &counted, &prefix, &manifest, &ses, wait);
    assert_eq!(passes(&counted), [true, true, true, true, false]);
    let error = route_checks(
        true,
        &body("bundled", &bundled, sha, true),
        &prefix,
        &manifest,
        &ses,
        wait,
    );
    assert!(!error[0].pass);
    let no_session = route_checks(
        false,
        &body("bundled", &bundled, sha, true),
        &prefix,
        &manifest,
        &dir.path().join("absent.ses"),
        wait,
    );
    assert!(!no_session[2].pass);
}

#[test]
fn staging_points_the_manifest_at_its_own_board_copy() {
    let dir = tempfile::tempdir().unwrap();
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/konnect-core/tests/fixtures");
    let staged = stage(&fixtures, dir.path(), Path::new("/x/kicad-cli")).unwrap();
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(staged.dsn.with_extension("dsn.konnect.json")).unwrap(),
    )
    .unwrap();
    let board = dir
        .path()
        .join("board/specctra_pours_routing.kicad_pcb")
        .canonicalize()
        .unwrap();
    assert_eq!(manifest["board_path"], json!(board.to_string_lossy()));
    assert_eq!(
        std::fs::read(&board).unwrap(),
        std::fs::read(fixtures.join("specctra_pours_routing.kicad_pcb")).unwrap(),
        "the board copy is the fixture, byte for byte (the judge checks its hash)"
    );
    assert!(staged.dsn.is_file());
    assert!(!staged.ses.exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("konnect.toml")).unwrap(),
        "kicad_cli = \"/x/kicad-cli\"\n",
        "only kicad-cli: simulation stays at its default"
    );
    for folder in ["home", "tmp", "state", "kicad-config"] {
        assert!(dir.path().join(folder).is_dir(), "{folder}");
    }
}

#[test]
fn a_route_that_did_not_succeed_or_left_a_connection_open_fails() {
    let dir = tempfile::tempdir().unwrap();
    let prefix = package(dir.path());
    let manifest = manifest(&prefix);
    let ses = dir.path().join("board.ses");
    std::fs::write(&ses, "(session board)").unwrap();
    let bundled = prefix.join("bin/fastroute");
    let base = json!({
        "success": true,
        "status": "complete",
        "engine": {"name": "fastroute", "source": "bundled", "path": bundled,
                   "sha256": manifest["fastroute"]["sha256"]},
        "completion": {"judged": {"all_gates_pass": true, "unconnected": 0}},
    });
    let wait = Duration::from_secs(1);
    assert!(route_checks(false, &base, &prefix, &manifest, &ses, wait)
        .iter()
        .all(|check| check.pass));
    let mut failed = base.clone();
    failed["success"] = json!(false);
    assert!(!route_checks(false, &failed, &prefix, &manifest, &ses, wait)[0].pass);
    let mut open = base.clone();
    open["completion"]["judged"]["unconnected"] = json!(2);
    assert!(!route_checks(false, &open, &prefix, &manifest, &ses, wait)[3].pass);
}

#[test]
fn any_protected_change_or_a_missing_installed_konnect_fails() {
    let account = tempfile::tempdir().unwrap();
    let installed = account.path().join(INSTALLED_KONNECT);
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::write(&installed, "installed").unwrap();
    let settings = account.path().join(KICAD_SETTINGS).join("11.0");
    std::fs::create_dir_all(&settings).unwrap();
    std::fs::write(settings.join("kicad_common.json"), "{}").unwrap();
    let before = protected_hashes(account.path());
    assert_eq!(before.len(), 2, "{before:?}");
    assert!(protected_check(&before, &protected_hashes(account.path())).pass);

    std::fs::write(settings.join("kicad_common.json"), "{\"changed\": 1}").unwrap();
    let check = protected_check(&before, &protected_hashes(account.path()));
    assert!(
        !check.pass && check.detail.contains("kicad_common.json"),
        "{check:?}"
    );

    std::fs::write(settings.join("kicad_common.json"), "{}").unwrap();
    std::fs::write(settings.join("pcbnew.json"), "{}").unwrap();
    let check = protected_check(&before, &protected_hashes(account.path()));
    assert!(
        !check.pass && check.detail.contains("pcbnew.json"),
        "a new file: {check:?}"
    );

    let empty = tempfile::tempdir().unwrap();
    let none = protected_hashes(empty.path());
    assert!(
        !protected_check(&none, &none).pass,
        "nothing to check is not a pass"
    );
}

#[test]
fn staging_without_a_fixture_file_fails() {
    let dir = tempfile::tempdir().unwrap();
    let fixtures = tempfile::tempdir().unwrap();
    assert!(stage(fixtures.path(), dir.path(), Path::new("/x/kicad-cli")).is_err());
}
