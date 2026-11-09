//! Creation PR 5: the shipped workflow must name executable tools and retain
//! the evidence boundaries that prevent a partial check becoming a pass.
//! These are asset-contract checks; independent dry runs assess agent behavior.

use konnect_core::router::registry;
use std::path::Path;
use std::process::Command;

fn asset(path: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join(path),
    )
    .unwrap()
}

#[test]
fn compiled_creation_skills_match_the_sources() {
    let home = tempfile::tempdir().unwrap();
    for name in [
        "kicad-schematic",
        "kicad-pcb",
        "kicad-review",
        "kicad-manufacture",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_konnect"))
            .args(["skill", name])
            .env("HOME", home.path())
            .env("KONNECT_STATE_DIR", home.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{name}: {:?}", output.stderr);
        assert_eq!(
            output.stdout,
            asset(&format!("skills/{name}/SKILL.md")).as_bytes(),
            "compiled {name} is stale"
        );
    }
}

fn require(text: &str, tokens: &[&str], context: &str) {
    for token in tokens {
        assert!(
            text.contains(token),
            "{context} omits contract item {token}"
        );
    }
}

#[test]
fn schematic_completion_uses_saved_connectivity_and_the_required_toolsets() {
    for file in [
        "skills/kicad-schematic/SKILL.md",
        "agents/kicad-schematic-build-agent.md",
    ] {
        let text = asset(file);
        // save_project is PCB-only. A prose warning may name it, a schematic
        // workflow must never prescribe invoking it to save its sheets.
        assert!(
            !text.contains("save_project("),
            "{file} prescribes PCB-only saving"
        );
        assert!(
            text.contains("scope: \"complete\"") || text.contains("scope=\"complete\""),
            "{file} omits complete-scope acceptance"
        );
        require(
            &text,
            &[
                "library",
                "sch_hierarchy",
                "sch_bus",
                "compare_netlist",
                "complete",
                "get_schematic_pin_locations",
                "add_power_symbol",
                "INCOMPLETE",
            ],
            file,
        );
    }
}

#[test]
fn review_evidence_has_a_page_cursor_and_an_independent_intent_boundary() {
    for file in [
        "skills/kicad-review/SKILL.md",
        "agents/kicad-design-review-agent.md",
    ] {
        let text = asset(file);
        assert!(
            text.contains("scope: \"complete\"") || text.contains("scope=\"complete\""),
            "{file} omits complete-scope acceptance"
        );
        require(
            &text,
            &[
                "next_offset",
                "truncated",
                "output",
                "INCOMPLETE",
                "compare_netlist",
                "complete",
                "live_board_synced",
            ],
            file,
        );
    }
    let review = asset("skills/kicad-review/SKILL.md");
    require(
        &review,
        &[
            "does not measure PCB proximity",
            "does not establish current capacity",
        ],
        "heuristic audit coverage",
    );
}

#[test]
fn pcb_routing_exposes_the_whole_supported_route_and_failure_boundary() {
    let text = asset("skills/kicad-pcb/SKILL.md");
    require(
        &text,
        &[
            "set_component_placements",
            "query_board_region",
            "check_freerouting",
            "export_specctra_dsn",
            "route_specctra_dsn",
            "plan_specctra_ses_import",
            "apply_specctra_ses",
            "candidate_output_path",
            "unconnected_items",
            "INCOMPLETE",
            "2 to 32 copper layers",
            "either side",
            "thermal reliefs",
        ],
        "PCB workflow",
    );
}

#[test]
fn pcb_layout_says_which_tools_honour_a_footprint_lock() {
    // Only the two planners read KiCad's lock; the explicit move tools do not,
    // so the skill must not promise that a lock protects a part from them.
    let text = asset("skills/kicad-pcb/SKILL.md");
    let outline_step = text
        .split("1. **Board outline**")
        .nth(1)
        .expect("missing board outline step")
        .split("2. **Update from schematic**")
        .next()
        .unwrap()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    require(
        &outline_step,
        &[
            "set_footprint_lock",
            "Only `auto_place_from_schematic` and `refine_placement_force_directed` hold",
            "`move_component`",
            "`set_component_placements`",
            "do not check the lock",
        ],
        "PCB board outline step",
    );
    assert!(!text.contains("so placement and routing respect them"));
}

#[test]
fn blocked_routing_has_an_explicit_stop_and_incomplete_handoff() {
    let text = asset("skills/kicad-pcb/SKILL.md");
    let failure = text
        .split("If DSN export, Freerouting, or SES import fails,")
        .nth(1)
        .expect("missing route failure policy")
        .split("### Routing Tools")
        .next()
        .unwrap();
    require(
        failure,
        &[
            "stop",
            "blocker",
            "Do not hand-roll",
            "KiCad PCB Editor",
            "unrouted",
            "INCOMPLETE",
        ],
        "route failure policy",
    );
}

#[test]
fn guidance_tools_belong_to_the_toolsets_the_workflows_use() {
    // A tool name existing somewhere in the catalogue does not mean loading
    // the workflow's toolset exposes it. Verify the actual registry ownership.
    for (toolset, names) in [
        (
            "sch_export",
            &["compare_netlist", "get_schematic_netlist", "run_erc"][..],
        ),
        (
            "sch_hierarchy",
            &["add_hierarchical_sheet", "validate_sheet_pins"][..],
        ),
        ("sch_bus", &["add_bus", "connect_pins_to_bus"][..]),
        (
            "pcb_components",
            &["set_component_placements", "flip_component"][..],
        ),
        ("placement", &["query_board_region"][..]),
        (
            "integration",
            &["check_freerouting", "route_specctra_dsn"][..],
        ),
        ("pcb_export", &["export_specctra_dsn"][..]),
        (
            "pcb_routing",
            &["plan_specctra_ses_import", "apply_specctra_ses"][..],
        ),
    ] {
        let tools = registry::tools_for(toolset).unwrap();
        for name in names {
            assert!(
                tools.iter().any(|t| t.name == *name),
                "{name} is not in {toolset}"
            );
        }
    }
}
