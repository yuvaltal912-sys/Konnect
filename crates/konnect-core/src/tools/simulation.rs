//! SPICE simulation tools.

use super::{cli, get_path, sch_export, ToolContext, ToolDef};
use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::CallToolResult;
use crate::simulation::kicad_netlist::{self, ElementEntry, NetEntry, SpiceNetlist};
use crate::simulation::snapshot;
use crate::simulation::worker_client::{probe_engine, EngineStatus};
use crate::tool;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

const EXPORT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_LIMIT: u64 = 25;
const MAX_LIMIT: u64 = 200;
const DETAIL_LIMIT: usize = 800;

pub fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "check_simulation_readiness",
        "Report whether a schematic can be simulated, and why not, without running a simulation. Works from the saved files on a private copy, so the project is not touched and unsaved editor changes are not seen.",
        json!({
            "type": "object",
            "properties": {
                "schematic": { "type": "string", "description": "Root .kicad_sch path" },
                "net_filter": { "type": "string", "description": "Only list nets whose name contains this text" },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_LIMIT,
                    "default": DEFAULT_LIMIT,
                    "description": "Most nets and elements to list; totals are always complete"
                }
            },
            "required": ["schematic"]
        }),
        |args, ctx| async move { handle_check_simulation_readiness(args, ctx).await }
    )]
}

async fn handle_check_simulation_readiness(
    args: &Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let schematic = get_path(args, "schematic")?;
    let net_filter = args["net_filter"].as_str().unwrap_or("");
    let limit = args["limit"]
        .as_u64()
        .unwrap_or(DEFAULT_LIMIT)
        .min(MAX_LIMIT) as usize;

    if !schematic.is_file() {
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::FileNotFound {
                path: schematic.display().to_string(),
            },
            format!("Schematic not found: {}", schematic.display()),
        ));
    }
    match sch_export::owning_project_root(&schematic) {
        Ok(None) => {}
        Ok(Some(root)) => {
            let reason = format!(
                "{} is a sheet inside the project rooted at {}. KiCad would treat it as the \
                 root and simulate only part of the design: use {}.",
                schematic.display(),
                root.display(),
                root.display()
            );
            return Ok(CallToolResult::error_kind(
                ToolErrorKind::InvalidArgument {
                    field: "schematic".to_string(),
                    reason: reason.clone(),
                },
                reason,
            ));
        }
        Err(error) => return Ok(error.into_tool_result()),
    }

    let engine = probe_engine(
        ctx.simulation.get().map(|r| r.as_ref()),
        &ctx.config.kicad_cli,
    )
    .await;
    let design = inspect_design(&ctx.config.kicad_cli, &schematic).await;
    Ok(CallToolResult::json(&readiness_report(
        &engine, &design, net_filter, limit,
    )))
}

/// The design as `kicad-cli` sees it, from a private copy of the saved files.
struct Design {
    nets: Vec<NetEntry>,
    elements: Vec<ElementEntry>,
    locks_present: Vec<String>,
}

async fn inspect_design(kicad_cli: &str, schematic: &Path) -> anyhow::Result<Design> {
    let scratch = tempfile::Builder::new()
        .prefix("konnect-sim-export-")
        .tempdir()?;
    let source = scratch.path().join("src");
    std::fs::create_dir(&source)?;
    let snapshot = snapshot::take_snapshot(schematic, &source)?;
    let netlists = cli::export_simulation_netlists(
        kicad_cli,
        &snapshot.root_sheet,
        scratch.path(),
        EXPORT_TIMEOUT,
    )
    .await?;
    let kicad = kicad_netlist::parse(&netlists.kicad)?;
    let spice = SpiceNetlist::parse(&netlists.spice);
    Ok(Design {
        nets: kicad_netlist::net_entries(&kicad, &spice),
        elements: kicad_netlist::element_entries(&kicad, &spice),
        locks_present: snapshot
            .locks_present
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
    })
}

#[derive(Serialize)]
struct Listing<'a, T> {
    total: usize,
    shown: usize,
    truncated: bool,
    items: Vec<&'a T>,
}

/// Counted before the cut, as `run_drc` does, so a short list never reads as a
/// small design.
fn listing<'a, T>(items: impl Iterator<Item = &'a T>, limit: usize) -> Listing<'a, T> {
    let items: Vec<&T> = items.collect();
    let total = items.len();
    let items: Vec<&T> = items.into_iter().take(limit).collect();
    Listing {
        total,
        shown: items.len(),
        truncated: items.len() < total,
        items,
    }
}

/// `ready` is true only when every check ran and passed, false when one failed,
/// and null when one could not run. A check that did not run is never a pass.
fn readiness_report(
    engine: &EngineStatus,
    design: &anyhow::Result<Design>,
    net_filter: &str,
    limit: usize,
) -> Value {
    let engine_ran = engine.probe_status != "not_available";
    let engine_check = if engine_ran {
        json!({ "name": "engine", "ran": true, "passed": engine.available })
    } else {
        json!({ "name": "engine", "ran": false, "reason": engine.detail })
    };
    let export_check = match design {
        Ok(_) => json!({ "name": "netlist_export", "ran": true, "passed": true }),
        Err(error) => json!({
            "name": "netlist_export",
            "ran": true,
            "passed": false,
            "detail": export_detail(&format!("{error:#}")),
        }),
    };
    let checks = [
        engine_check,
        export_check,
        json!({
            "name": "preflight",
            "ran": false,
            "reason": "Preflight checks are not implemented in this build."
        }),
    ];
    let failed = (engine_ran && !engine.available) || design.is_err();
    let ready = if failed { json!(false) } else { Value::Null };

    let mut report = json!({
        "ready": ready,
        "checks": checks,
        "engine": engine,
        "sources": ["saved_schematic_snapshot"],
    });
    if let Ok(design) = design {
        let filter = net_filter.to_lowercase();
        let nets = design.nets.iter().filter(|net| {
            net.kicad_name.to_lowercase().contains(&filter)
                || net.spice_name.to_lowercase().contains(&filter)
        });
        report["nets"] = json!(listing(nets, limit));
        report["elements"] = json!(listing(design.elements.iter(), limit));
        report["source_evidence"] = json!({
            "excludes_unsaved_editor_state": true,
            "locks_present": design.locks_present,
        });
    }
    report
}

/// Keep KiCad's own words and drop library noise that says nothing about the
/// design. Exported text is data from the project, so it stays bounded.
fn export_detail(message: &str) -> String {
    let lines: Vec<&str> = message
        .lines()
        .filter(|line| !line.contains("Fontconfig"))
        .collect();
    lines.join("\n").chars().take(DETAIL_LIMIT).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::ToolRouter;
    use crate::tools::ServerConfig;
    use std::sync::Arc;

    const NET: &str = include_str!("../../tests/fixtures/simulation/rectifier_unmodelled.net");
    const CIR: &str = include_str!("../../tests/fixtures/simulation/rectifier_unmodelled.cir");

    fn engine(probe_status: &'static str, available: bool) -> EngineStatus {
        EngineStatus {
            probe_status,
            available,
            version: available.then(|| "44.2".to_string()),
            library_path: None,
            code_models_dir: None,
            code_model_count: 0,
            code_models_failed: Vec::new(),
            searched_locations: Vec::new(),
            detail: None,
        }
    }

    fn design() -> anyhow::Result<Design> {
        let kicad = kicad_netlist::parse(NET)?;
        let spice = SpiceNetlist::parse(CIR);
        Ok(Design {
            nets: kicad_netlist::net_entries(&kicad, &spice),
            elements: kicad_netlist::element_entries(&kicad, &spice),
            locks_present: vec!["/p/~top.kicad_sch.lck".to_string()],
        })
    }

    fn context(kicad_cli: &str) -> ToolContext {
        ToolContext::new(
            ServerConfig {
                kicad_cli: kicad_cli.to_string(),
                ..Default::default()
            },
            Arc::new(ToolRouter::new()),
        )
    }

    fn sheet() -> &'static str {
        "(kicad_sch (version 20250114) (generator \"eeschema\") \
         (uuid \"00000000-0000-0000-0000-000000000001\") (paper \"A4\"))"
    }

    #[test]
    fn a_working_engine_and_export_are_not_ready_while_preflight_has_not_run() {
        let report = readiness_report(&engine("ok", true), &design(), "", 25);
        assert!(report["ready"].is_null());
        assert_eq!(report["checks"][0]["passed"], true);
        assert_eq!(report["checks"][1]["passed"], true);
        assert_eq!(report["checks"][2]["ran"], false);
    }

    #[test]
    fn a_missing_engine_is_not_ready() {
        let report = readiness_report(&engine("not_found", false), &design(), "", 25);
        assert_eq!(report["ready"], false);
        assert_eq!(report["checks"][0]["passed"], false);
    }

    #[test]
    fn an_engine_check_that_could_not_run_is_unknown_not_a_failure() {
        let report = readiness_report(&engine("not_available", false), &design(), "", 25);
        assert!(report["ready"].is_null());
        assert_eq!(report["checks"][0]["ran"], false);
    }

    #[test]
    fn a_failed_export_is_not_ready_and_lists_no_nets() {
        let report = readiness_report(
            &engine("ok", true),
            &Err(anyhow::anyhow!("kicad-cli exited with 2")),
            "",
            25,
        );
        assert_eq!(report["ready"], false);
        assert_eq!(report["checks"][1]["passed"], false);
        assert!(report["checks"][1]["detail"]
            .as_str()
            .unwrap()
            .contains("exited with 2"));
        assert!(report.get("nets").is_none());
        assert!(report.get("source_evidence").is_none());
    }

    #[test]
    fn totals_are_counted_before_the_cut() {
        let report = readiness_report(&engine("ok", true), &design(), "", 2);
        assert_eq!(report["elements"]["total"], 5);
        assert_eq!(report["elements"]["shown"], 2);
        assert_eq!(report["elements"]["truncated"], true);
        assert_eq!(report["elements"]["items"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn nets_can_be_filtered_by_name() {
        let report = readiness_report(&engine("ok", true), &design(), "RECT_", 25);
        assert_eq!(report["nets"]["total"], 1);
        assert_eq!(report["nets"]["items"][0]["kicad_name"], "/rect_out");
    }

    #[test]
    fn results_state_they_come_from_saved_files_and_name_any_locks() {
        let report = readiness_report(&engine("ok", true), &design(), "", 25);
        assert_eq!(report["sources"][0], "saved_schematic_snapshot");
        assert_eq!(
            report["source_evidence"]["excludes_unsaved_editor_state"],
            true
        );
        assert_eq!(
            report["source_evidence"]["locks_present"][0],
            "/p/~top.kicad_sch.lck"
        );
    }

    #[test]
    fn export_details_drop_font_noise_and_stay_bounded() {
        let noisy = format!(
            "Fontconfig warning: x\nSimulation model library not found\n{}",
            "y".repeat(5000)
        );
        let detail = export_detail(&noisy);
        assert!(!detail.contains("Fontconfig"));
        assert!(detail.contains("library not found"));
        assert!(detail.chars().count() <= DETAIL_LIMIT);
    }

    #[tokio::test]
    async fn a_missing_schematic_is_a_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let args = json!({ "schematic": dir.path().join("absent.kicad_sch") });
        let result = handle_check_simulation_readiness(&args, &context(""))
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn a_child_sheet_is_refused_with_the_root_named() {
        let dir = tempfile::tempdir().unwrap();
        let root_sheet = "(kicad_sch (version 20250114) (generator \"eeschema\") \
             (uuid \"00000000-0000-0000-0000-000000000001\") (paper \"A4\") \
             (sheet (at 0 0) (size 10 10) (uuid \"00000000-0000-0000-0000-000000000002\") \
             (property \"Sheetname\" \"C\" (at 0 0 0)) \
             (property \"Sheetfile\" \"child.kicad_sch\" (at 0 0 0))))";
        std::fs::write(dir.path().join("top.kicad_sch"), root_sheet).unwrap();
        std::fs::write(dir.path().join("top.kicad_pro"), "{}").unwrap();
        let child = dir.path().join("child.kicad_sch");
        std::fs::write(&child, sheet()).unwrap();
        let result =
            handle_check_simulation_readiness(&json!({ "schematic": child }), &context(""))
                .await
                .unwrap();
        assert!(result.is_error);
    }

    #[cfg(unix)]
    mod with_fake_cli {
        use super::*;
        use crate::tools::cli::test_support::write_script;

        /// Scans argv, so it survives argument reordering, and logs each format.
        fn exporting_cli(dir: &Path) -> std::path::PathBuf {
            let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/simulation/rectifier_unmodelled");
            let body = format!(
                "format=''; output=''\n\
                 while [ $# -gt 0 ]; do\n\
                   case \"$1\" in\n\
                     --output) output=\"$2\"; shift ;;\n\
                     --format) format=\"$2\"; shift ;;\n\
                   esac\n\
                   shift\n\
                 done\n\
                 if read -r line; then echo stdin-open >> '{log}'; fi\n\
                 echo \"$format\" >> '{log}'\n\
                 case \"$format\" in\n\
                   spice) cp '{fixture}.cir' \"$output\" ;;\n\
                   *) cp '{fixture}.net' \"$output\" ;;\n\
                 esac\n",
                log = dir.join("calls.log").display(),
                fixture = fixtures.display(),
            );
            write_script(dir, "fake-kicad-cli", &body, "")
        }

        fn project(dir: &Path) -> std::path::PathBuf {
            let root = dir.join("top.kicad_sch");
            std::fs::write(&root, sheet()).unwrap();
            std::fs::write(dir.join("top.kicad_pro"), "{}").unwrap();
            root
        }

        #[tokio::test]
        async fn readiness_exports_kicad_format_first_and_lists_the_design() {
            let tools = tempfile::tempdir().unwrap();
            let project_dir = tempfile::tempdir().unwrap();
            let root = project(project_dir.path());
            let fake = exporting_cli(tools.path());

            let design = inspect_design(fake.to_str().unwrap(), &root).await.unwrap();
            assert_eq!(design.elements.len(), 5);
            assert!(design.nets.iter().any(|net| net.kicad_name == "/rect_out"));

            let calls = std::fs::read_to_string(tools.path().join("calls.log")).unwrap();
            assert_eq!(calls, "kicadsexpr\nspice\n");
        }

        #[tokio::test]
        async fn the_export_never_writes_into_the_project() {
            use std::os::unix::fs::PermissionsExt;
            let tools = tempfile::tempdir().unwrap();
            let project_dir = tempfile::tempdir().unwrap();
            let root = project(project_dir.path());
            let fake = exporting_cli(tools.path());
            let read_only = std::fs::Permissions::from_mode(0o555);
            std::fs::set_permissions(project_dir.path(), read_only).unwrap();

            let result = inspect_design(fake.to_str().unwrap(), &root).await;

            let writable = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(project_dir.path(), writable).unwrap();
            assert!(result.is_ok(), "{:?}", result.err());
            assert_eq!(std::fs::read_dir(project_dir.path()).unwrap().count(), 2);
        }

        #[tokio::test]
        async fn a_nonzero_exit_fails_even_when_a_file_was_written() {
            let tools = tempfile::tempdir().unwrap();
            let project_dir = tempfile::tempdir().unwrap();
            let root = project(project_dir.path());
            let fake = write_script(
                tools.path(),
                "failing-cli",
                "while [ $# -gt 1 ]; do\n\
                   if [ \"$1\" = --output ]; then echo written > \"$2\"; fi\n\
                   shift\n\
                 done\n\
                 echo 'Simulation model library not found'\n\
                 exit 2\n",
                "",
            );
            let error = inspect_design(fake.to_str().unwrap(), &root)
                .await
                .err()
                .unwrap();
            assert!(
                format!("{error:#}").contains("library not found"),
                "{error:#}"
            );
        }

        #[tokio::test]
        async fn an_empty_export_is_a_failure() {
            let tools = tempfile::tempdir().unwrap();
            let project_dir = tempfile::tempdir().unwrap();
            let root = project(project_dir.path());
            let fake = write_script(
                tools.path(),
                "silent-cli",
                "while [ $# -gt 1 ]; do\n\
                   if [ \"$1\" = --output ]; then : > \"$2\"; fi\n\
                   shift\n\
                 done\n",
                "",
            );
            assert!(inspect_design(fake.to_str().unwrap(), &root).await.is_err());
        }

        #[tokio::test]
        async fn a_hung_export_is_killed_at_the_timeout() {
            let tools = tempfile::tempdir().unwrap();
            let pid_file = tools.path().join("pid");
            let fake = write_script(
                tools.path(),
                "hung-cli",
                &format!("echo $$ > '{}'\nexec /bin/sleep 30\n", pid_file.display()),
                "",
            );
            let result = cli::export_simulation_netlists(
                fake.to_str().unwrap(),
                Path::new("/x.kicad_sch"),
                tools.path(),
                Duration::from_millis(500),
            )
            .await;
            assert!(result.is_err());
            let pid = std::fs::read_to_string(&pid_file).unwrap();
            let pid = pid.trim();
            let mut gone = false;
            for _ in 0..50 {
                let alive = std::process::Command::new("kill")
                    .args(["-0", pid])
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap()
                    .success();
                if !alive {
                    gone = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(gone, "the hung export is still running");
        }
    }
}
