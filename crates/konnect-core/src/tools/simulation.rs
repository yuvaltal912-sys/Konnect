//! SPICE simulation tools.

use super::{get_path, ToolContext, ToolDef};
use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::CallToolResult;
use crate::simulation::worker_client::{probe_engine, EngineStatus};
use crate::tool;
use serde_json::{json, Value};

pub fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "check_simulation_readiness",
        "Report whether a schematic can be simulated, and why not, without running a simulation.",
        json!({
            "type": "object",
            "properties": {
                "schematic": { "type": "string", "description": "Root .kicad_sch path" }
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
    if !schematic.is_file() {
        return Ok(CallToolResult::error_kind(
            ToolErrorKind::FileNotFound {
                path: schematic.display().to_string(),
            },
            format!("Schematic not found: {}", schematic.display()),
        ));
    }
    let engine = probe_engine(
        ctx.simulation.get().map(|r| r.as_ref()),
        &ctx.config.kicad_cli,
    )
    .await;
    Ok(CallToolResult::json(&readiness_report(&engine)))
}

/// `ready` is true only when every check ran and passed, false when one failed,
/// and null when one could not run. A check that did not run is never a pass.
fn readiness_report(engine: &EngineStatus) -> Value {
    let engine_ran = engine.probe_status != "not_available";
    let engine_check = if engine_ran {
        json!({ "name": "engine", "ran": true, "passed": engine.available })
    } else {
        json!({ "name": "engine", "ran": false, "reason": engine.detail })
    };
    let checks = [
        engine_check,
        json!({
            "name": "schematic",
            "ran": false,
            "reason": "Schematic checks are not implemented in this build."
        }),
    ];
    let ready = if engine_ran && !engine.available {
        json!(false)
    } else {
        Value::Null
    };
    json!({ "ready": ready, "checks": checks, "engine": engine })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::ToolRouter;
    use crate::tools::ServerConfig;
    use std::sync::Arc;

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

    fn context() -> ToolContext {
        ToolContext::new(ServerConfig::default(), Arc::new(ToolRouter::new()))
    }

    #[test]
    fn a_working_engine_alone_is_not_ready_while_other_checks_have_not_run() {
        let report = readiness_report(&engine("ok", true));
        assert!(report["ready"].is_null());
        assert_eq!(report["checks"][0]["passed"], true);
        assert_eq!(report["checks"][1]["ran"], false);
        assert_eq!(report["engine"]["version"], "44.2");
    }

    #[test]
    fn a_missing_engine_is_not_ready() {
        let report = readiness_report(&engine("not_found", false));
        assert_eq!(report["ready"], false);
        assert_eq!(report["checks"][0]["ran"], true);
        assert_eq!(report["checks"][0]["passed"], false);
    }

    #[test]
    fn an_engine_check_that_could_not_run_is_unknown_not_a_failure() {
        let report = readiness_report(&engine("not_available", false));
        assert!(report["ready"].is_null());
        assert_eq!(report["checks"][0]["ran"], false);
    }

    #[tokio::test]
    async fn a_missing_schematic_is_a_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.kicad_sch");
        let args = json!({ "schematic": path });
        let result = handle_check_simulation_readiness(&args, &context())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn an_embedded_context_reports_unknown_readiness() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.kicad_sch");
        std::fs::write(&path, "(kicad_sch)").unwrap();
        let args = json!({ "schematic": path });
        let result = handle_check_simulation_readiness(&args, &context())
            .await
            .unwrap();
        assert!(!result.is_error);
    }
}
