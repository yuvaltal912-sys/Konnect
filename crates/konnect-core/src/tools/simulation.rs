//! SPICE simulation tools.

use super::{cli, get_path, sch_export, ToolContext, ToolDef};
use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::CallToolResult;
use crate::simulation::flatten::{self, Compatibility, DeckInputs, FlatDeck, Refusal};
use crate::simulation::kicad_netlist::{self, ElementEntry, KiCadNetlist, NetEntry, SpiceNetlist};
use crate::simulation::preflight::{self, Preflight};
use crate::simulation::snapshot;
use crate::simulation::worker_client::{probe_engine, EngineStatus};
use crate::tool;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

const EXPORT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_LIMIT: u64 = 25;
const MAX_LIMIT: u64 = 200;
const DETAIL_LIMIT: usize = 800;
const MAX_SPICE_LINES: usize = 200;

pub fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "check_simulation_readiness",
        "Report whether a schematic can be simulated, and why not, without running a simulation. Works from the saved files on a private copy, so the project is not touched and unsaved editor changes are not seen.",
        json!({
            "type": "object",
            "properties": {
                "schematic": { "type": "string", "description": "Root .kicad_sch path" },
                "testbench": {
                    "type": "object",
                    "description": "Changes to the deck for simulation only. The schematic is not touched",
                    "properties": {
                        "exclude_references": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "References to leave out, or prefix wildcards such as J*"
                        },
                        "spice_lines": {
                            "type": "array",
                            "maxItems": MAX_SPICE_LINES,
                            "items": { "type": "string" },
                            "description": "Elements, models and includes to add. Use spice_name values from this tool"
                        }
                    }
                },
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
    let model_dirs = ctx
        .simulation
        .get()
        .map(|runtime| runtime.model_dirs().to_vec())
        .unwrap_or_default();
    let testbench = Testbench::from_args(&args["testbench"]);
    let allow_digital_models = ctx
        .simulation
        .get()
        .is_none_or(|runtime| runtime.allows_digital_models());
    let inputs = DesignInputs {
        model_dirs: &model_dirs,
        testbench: &testbench,
        allow_digital_models,
        code_model_count: engine.code_model_count,
    };
    let design = inspect_design(&ctx.config.kicad_cli, &schematic, &inputs).await;
    Ok(CallToolResult::json(&readiness_report(
        &engine, &design, net_filter, limit,
    )))
}

#[derive(Default)]
struct Testbench {
    exclude_references: Vec<String>,
    spice_lines: Vec<String>,
}

impl Testbench {
    fn from_args(value: &Value) -> Self {
        let strings = |field: &str| -> Vec<String> {
            value[field]
                .as_array()
                .map(|items| {
                    let text = items.iter().filter_map(Value::as_str);
                    text.map(str::to_owned).collect()
                })
                .unwrap_or_default()
        };
        Self {
            exclude_references: strings("exclude_references"),
            spice_lines: strings("spice_lines"),
        }
    }
}

struct DesignInputs<'a> {
    model_dirs: &'a [PathBuf],
    testbench: &'a Testbench,
    allow_digital_models: bool,
    code_model_count: usize,
}

/// References each pattern selects, and the patterns that select nothing. A
/// trailing `*` matches a prefix.
fn excluded_references(kicad: &KiCadNetlist, patterns: &[String]) -> (Vec<String>, Vec<String>) {
    let mut references = Vec::new();
    let mut unmatched = Vec::new();
    for pattern in patterns {
        let matches = |reference: &str| match pattern.strip_suffix('*') {
            Some(prefix) => reference.starts_with(prefix),
            None => reference == pattern,
        };
        let selected: Vec<String> = kicad
            .components
            .iter()
            .filter(|component| matches(&component.reference))
            .map(|component| component.reference.clone())
            .collect();
        if selected.is_empty() {
            unmatched.push(pattern.clone());
        }
        references.extend(selected);
    }
    references.sort();
    references.dedup();
    (references, unmatched)
}

/// The design as `kicad-cli` sees it, from a private copy of the saved files.
struct Design {
    nets: Vec<NetEntry>,
    elements: Vec<ElementEntry>,
    locks_present: Vec<String>,
    compatibility: Compatibility,
    deck: Result<FlatDeck, Refusal>,
    ignored_model_dirs: Vec<String>,
    excluded_references: Vec<String>,
    preflight: Option<Preflight>,
}

/// KiCad wipes its per-user IBIS cache on every SPICE export, from any project,
/// so an export and the read of its includes must not interleave with another.
static EXPORT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Folders an include may read from, canonical to compare with resolved paths,
/// and any configured folder that was left out. A root that holds the user's
/// home folder would stop the check meaning anything, so it is never used.
fn allowed_roots(project_dir: &Path, model_dirs: &[PathBuf]) -> (Vec<PathBuf>, Vec<String>) {
    let home = dirs::home_dir().and_then(|home| home.canonicalize().ok());
    let usable = |path: &Path| {
        path.parent().is_some() && !home.as_ref().is_some_and(|home| home.starts_with(path))
    };
    let cache = dirs::cache_dir()
        .into_iter()
        .chain(std::env::var_os("KICAD_CACHE_HOME").map(PathBuf::from))
        .flat_map(|dir| [dir.join("KiCad"), dir.join("kicad")]);
    let mut roots: Vec<PathBuf> = std::iter::once(project_dir.to_path_buf())
        .chain(crate::kicad_install::share_roots())
        .chain(cache)
        .filter_map(|path| path.canonicalize().ok())
        .filter(|path| usable(path))
        .collect();
    let mut ignored = Vec::new();
    for dir in model_dirs {
        match dir.canonicalize() {
            Ok(path) if dir.is_absolute() && usable(&path) => roots.push(path),
            _ => ignored.push(dir.display().to_string()),
        }
    }
    (roots, ignored)
}

async fn inspect_design(
    kicad_cli: &str,
    schematic: &Path,
    inputs: &DesignInputs<'_>,
) -> anyhow::Result<Design> {
    let scratch = tempfile::Builder::new()
        .prefix("konnect-sim-export-")
        .tempdir()?;
    let source = scratch.path().join("src");
    std::fs::create_dir(&source)?;
    let snapshot = snapshot::take_snapshot(schematic, &source)?;
    let project_dir = schematic
        .canonicalize()?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let compatibility = Compatibility::from_project_file(&schematic.with_extension("kicad_pro"));
    let _export_guard = EXPORT_LOCK.lock().await;
    let netlists = cli::export_simulation_netlists(
        kicad_cli,
        &snapshot.root_sheet,
        scratch.path(),
        EXPORT_TIMEOUT,
    )
    .await?;
    let kicad = kicad_netlist::parse(&netlists.kicad)?;
    let spice = SpiceNetlist::parse(&netlists.spice);
    let (allowed_roots, ignored_model_dirs) = allowed_roots(&project_dir, inputs.model_dirs);
    let (excluded_references, unmatched_exclusions) =
        excluded_references(&kicad, &inputs.testbench.exclude_references);
    let nets = kicad_netlist::net_entries(&kicad, &spice);
    let elements = kicad_netlist::element_entries(&kicad, &spice, &excluded_references);
    let excluded_elements: Vec<String> = elements
        .iter()
        .filter(|element| element.excluded)
        .filter_map(|element| element.element_name.as_deref())
        .map(str::to_lowercase)
        .collect();
    let deck = flatten::flatten(&DeckInputs {
        netlist: &netlists.spice,
        testbench: &inputs.testbench.spice_lines,
        project_dir: &project_dir,
        allowed_roots: &allowed_roots,
        compatibility,
        excluded_elements: &excluded_elements,
    });
    let preflight = deck.as_ref().ok().map(|deck| {
        preflight::run(&preflight::Inputs {
            deck,
            compatibility,
            kicad: &kicad,
            nets: &nets,
            elements: &elements,
            allow_digital_models: inputs.allow_digital_models,
            code_model_count: inputs.code_model_count,
            unmatched_exclusions: &unmatched_exclusions,
            has_testbench_lines: !inputs.testbench.spice_lines.is_empty(),
        })
    });
    Ok(Design {
        compatibility,
        deck,
        ignored_model_dirs,
        excluded_references,
        preflight,
        nets,
        elements,
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
    let deck_check = match design.as_ref().map(|design| &design.deck) {
        Ok(Ok(_)) => json!({ "name": "deck_vetting", "ran": true, "passed": true }),
        Ok(Err(refusal)) => json!({
            "name": "deck_vetting",
            "ran": true,
            "passed": false,
            "refusal": refusal,
        }),
        Err(_) => json!({
            "name": "deck_vetting",
            "ran": false,
            "reason": "The netlist could not be exported.",
        }),
    };
    let preflight = design
        .as_ref()
        .ok()
        .and_then(|design| design.preflight.as_ref());
    let mut checks = vec![engine_check, export_check, deck_check];
    match preflight {
        Some(preflight) => checks.extend(
            preflight
                .checks
                .iter()
                .map(|check| check_report(check, limit)),
        ),
        None => checks.push(json!({
            "name": "preflight",
            "ran": false,
            "reason": "Preflight runs on a deck that passed vetting."
        })),
    }
    let ready = if checks.iter().any(|check| check["passed"] == false) {
        json!(false)
    } else if checks.iter().any(|check| check["ran"] == false) {
        Value::Null
    } else {
        json!(true)
    };

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
        report["engine"]["compatibility"] = json!(design.compatibility.ngbehavior);
        if !design.ignored_model_dirs.is_empty() {
            report["ignored_model_dirs"] = json!(design.ignored_model_dirs);
        }
        if let Ok(deck) = &design.deck {
            report["deck"] = json!({
                "line_count": deck.line_count(),
                "inlined_file_count": deck.inlined_files.len(),
                "dropped_directives": deck.dropped_directives,
            });
        }
        if let Some(preflight) = &design.preflight {
            report["unmodelled_symbols"] = unmodelled_report(&preflight.unmodelled_symbols, limit);
            let warnings = preflight.warnings.iter();
            report["warnings"] = json!(warnings
                .map(|warning| warning_report(warning, limit))
                .collect::<Vec<_>>());
        }
        if !design.excluded_references.is_empty() {
            report["excluded_references"] =
                json!(listing(design.excluded_references.iter(), limit));
        }
        report["source_evidence"] = json!({
            "excludes_unsaved_editor_state": true,
            "locks_present": design.locks_present,
        });
    }
    report
}

/// A check with its items cut to the limit and counted before the cut.
fn check_report(check: &preflight::Check, limit: usize) -> Value {
    let mut report = json!(check);
    add_items(&mut report, &check.items, limit);
    report
}

fn add_items(report: &mut Value, items: &[String], limit: usize) {
    let shown: Vec<&String> = items.iter().take(limit).collect();
    report["total"] = json!(items.len());
    report["shown"] = json!(shown.len());
    report["truncated"] = json!(shown.len() < items.len());
    report["items"] = json!(shown);
}

fn warning_report(warning: &preflight::Warning, limit: usize) -> Value {
    let mut report = json!(warning);
    add_items(&mut report, &warning.items, limit);
    report
}

/// Listed like the other lists, with each symbol's pins cut to the same limit.
fn unmodelled_report(symbols: &[preflight::UnmodelledSymbol], limit: usize) -> Value {
    let mut report = json!(listing(symbols.iter(), limit));
    if let Some(items) = report["items"].as_array_mut() {
        for (item, symbol) in items.iter_mut().zip(symbols) {
            item["pin_total"] = json!(symbol.pins.len());
            item["pins"] = json!(symbol.pins.iter().take(limit).collect::<Vec<_>>());
        }
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

    static NO_TESTBENCH: Testbench = Testbench {
        exclude_references: Vec::new(),
        spice_lines: Vec::new(),
    };

    fn no_inputs() -> DesignInputs<'static> {
        DesignInputs {
            model_dirs: &[],
            testbench: &NO_TESTBENCH,
            allow_digital_models: true,
            code_model_count: 1,
        }
    }

    #[test]
    fn exclusions_take_exact_references_and_prefix_wildcards() {
        let kicad = kicad_netlist::parse(NET).unwrap();
        let (references, unmatched) = excluded_references(
            &kicad,
            &["D1".to_string(), "R*".to_string(), "J*".to_string()],
        );
        assert_eq!(references, ["D1", "R1", "R2"]);
        assert_eq!(unmatched, ["J*"]);
    }

    #[test]
    fn a_symbol_with_many_pins_is_cut_to_the_limit() {
        let pin = |number: usize| preflight::SymbolPin {
            pin: number.to_string(),
            pin_name: None,
            pin_type: None,
            net: "/a".to_string(),
            spice_net: None,
        };
        let symbol = preflight::UnmodelledSymbol {
            reference: "U1".to_string(),
            value: String::new(),
            lib_id: None,
            pins: (0..5).map(pin).collect(),
            suggested_fix: "exclude",
        };
        let report = unmodelled_report(&[symbol], 2);
        assert_eq!(report["items"][0]["pin_total"], 5);
        assert_eq!(report["items"][0]["pins"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_passing_design_is_ready_only_when_every_check_ran() {
        let mut passing = design().unwrap();
        passing.preflight = Some(Preflight {
            checks: Vec::new(),
            warnings: Vec::new(),
            unmodelled_symbols: Vec::new(),
        });
        let report = readiness_report(&engine("ok", true), &Ok(passing), "", 25);
        assert_eq!(report["ready"], true, "{report}");
        let report = readiness_report(&engine("not_available", false), &design(), "", 25);
        assert_eq!(report["ready"], Value::Null, "{report}");
    }

    #[test]
    fn check_items_are_cut_to_the_limit_and_counted_first() {
        let kicad = kicad_netlist::parse(NET).unwrap();
        let mut design = design().unwrap();
        design.preflight = Some(preflight::run(&preflight::Inputs {
            deck: &flatten::flatten(&DeckInputs {
                netlist: CIR,
                testbench: &[],
                project_dir: Path::new("/"),
                allowed_roots: &[],
                compatibility: Compatibility::DEFAULT,
                excluded_elements: &[],
            })
            .unwrap(),
            compatibility: Compatibility::DEFAULT,
            kicad: &kicad,
            nets: &design.nets,
            elements: &design.elements,
            allow_digital_models: true,
            code_model_count: 1,
            unmatched_exclusions: &[],
            has_testbench_lines: false,
        }));
        let report = readiness_report(&engine("ok", true), &Ok(design), "", 1);
        assert_eq!(report["ready"], false);
        let coverage = report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["name"] == "model_coverage")
            .unwrap();
        assert_eq!(coverage["total"], 2, "{report}");
        assert_eq!(coverage["items"].as_array().unwrap().len(), 1);
        assert_eq!(report["unmodelled_symbols"]["total"], 2);
        assert_eq!(report["unmodelled_symbols"]["truncated"], true);
    }

    fn design() -> anyhow::Result<Design> {
        let kicad = kicad_netlist::parse(NET)?;
        let spice = SpiceNetlist::parse(CIR);
        Ok(Design {
            nets: kicad_netlist::net_entries(&kicad, &spice),
            elements: kicad_netlist::element_entries(&kicad, &spice, &[]),
            locks_present: vec!["/p/~top.kicad_sch.lck".to_string()],
            compatibility: Compatibility::DEFAULT,
            deck: Ok(FlatDeck::default()),
            ignored_model_dirs: Vec::new(),
            excluded_references: Vec::new(),
            preflight: None,
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
        assert_eq!(report["checks"][2]["passed"], true);
        assert_eq!(report["checks"][3]["ran"], false);
    }

    #[test]
    fn a_refused_deck_is_not_ready_and_says_where() {
        let mut refused = design().unwrap();
        refused.deck = Err(Refusal {
            code: flatten::RefusalCode::EncryptedModel,
            origin: "/p/tps.lib".to_string(),
            line: Some(2),
            detail: "encrypted".to_string(),
        });
        let report = readiness_report(&engine("ok", true), &Ok(refused), "", 25);
        assert_eq!(report["ready"], false);
        assert_eq!(report["checks"][2]["passed"], false);
        assert_eq!(report["checks"][2]["refusal"]["code"], "encrypted_model");
        assert_eq!(report["checks"][2]["refusal"]["line"], 2);
        assert!(report.get("deck").is_none());
    }

    #[test]
    fn a_model_folder_that_cannot_be_used_is_reported_rather_than_lost() {
        let project = tempfile::tempdir().unwrap();
        let extra = tempfile::tempdir().unwrap();
        let home = dirs::home_dir().unwrap();
        let dirs = [
            extra.path().to_path_buf(),
            PathBuf::from("/"),
            home.clone(),
            PathBuf::from("~/spice"),
            PathBuf::from("relative"),
            PathBuf::from("."),
            extra.path().join("absent"),
        ];
        let (roots, ignored) = allowed_roots(project.path(), &dirs);
        assert!(roots.contains(&project.path().canonicalize().unwrap()));
        assert!(roots.contains(&extra.path().canonicalize().unwrap()));
        assert!(!roots.iter().any(|root| root.parent().is_none()));
        assert_eq!(ignored.len(), 6, "{ignored:?}");
    }

    #[test]
    fn a_project_that_holds_the_home_folder_is_not_a_root() {
        let home = dirs::home_dir().unwrap().canonicalize().unwrap();
        let (roots, _) = allowed_roots(&home, &[]);
        assert!(!roots.contains(&home));
    }

    #[test]
    fn a_failed_export_cannot_claim_the_deck_was_vetted() {
        let report = readiness_report(&engine("ok", true), &Err(anyhow::anyhow!("failed")), "", 25);
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

            let design = inspect_design(fake.to_str().unwrap(), &root, &no_inputs())
                .await
                .unwrap();
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

            let result = inspect_design(fake.to_str().unwrap(), &root, &no_inputs()).await;

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
            let error = inspect_design(fake.to_str().unwrap(), &root, &no_inputs())
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
            assert!(inspect_design(fake.to_str().unwrap(), &root, &no_inputs())
                .await
                .is_err());
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
