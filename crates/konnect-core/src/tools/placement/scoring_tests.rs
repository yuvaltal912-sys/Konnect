//! Parent-handler integration: metric policy, penalties, coverage and refusal
//! must survive the actual scoring path, not just the mathematical helpers.
use super::*;
use serde_json::Value;

const TWO_R: &str = include_str!("../../../tests/fixtures/specctra_two_resistors.kicad_pcb");

fn ctx() -> ToolContext {
    ToolContext::new(
        crate::tools::ServerConfig::default(),
        std::sync::Arc::new(crate::router::ToolRouter::new()),
    )
}
fn response(result: CallToolResult) -> Value {
    assert!(!result.is_error, "{result:?}");
    let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
        panic!("text expected")
    };
    serde_json::from_str(text).unwrap()
}
fn scene() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("two-resistors.kicad_pcb");
    std::fs::write(&path, TWO_R).unwrap();
    (dir, path)
}

#[tokio::test]
async fn fanout_policy_is_strict_before_any_board_read() {
    for value in [
        json!(null),
        json!(true),
        json!("8"),
        json!(1),
        json!(257),
        json!(-1),
        json!(3.5),
    ] {
        let result = handle_score_placement(
            &json!({"board":"nonexistent-pr8-board.kicad_pcb","max_net_references":value}),
            &ctx(),
        )
        .await
        .unwrap();
        assert!(result.is_error, "{value}: {result:?}");
        let text = serde_json::to_string(&result).unwrap();
        assert!(
            text.contains("invalid_argument") && text.contains("max_net_references"),
            "{text}"
        );
    }
    assert_eq!(metric_fanout_limit(&json!({})).unwrap(), 8);
    for limit in [2, 256] {
        assert_eq!(
            metric_fanout_limit(&json!({"max_net_references":limit})).unwrap(),
            limit
        );
    }
}

#[test]
fn published_fanout_schema_matches_runtime_policy() {
    let tool = tools()
        .into_iter()
        .find(|t| t.name == "score_placement")
        .unwrap();
    let validator = jsonschema::validator_for(&tool.input_schema).unwrap();
    for limit in [2, 8, 256] {
        assert!(validator.is_valid(&json!({"board":"x","max_net_references":limit})));
    }
    for value in [
        json!(null),
        json!(true),
        json!("8"),
        json!(1),
        json!(257),
        json!(3.5),
    ] {
        assert!(
            !validator.is_valid(&json!({"board":"x","max_net_references":value})),
            "{value}"
        );
    }
}

#[tokio::test]
async fn hand_computed_native_board_penalty_is_wired_into_the_score() {
    let (_dir, path) = scene();
    let before = std::fs::read(&path).unwrap();
    let report = response(
        handle_score_placement(&json!({"board":path}), &ctx())
            .await
            .unwrap(),
    );
    // Native outline is50x40mm; VCC11mm +GND9mm, two MST edges.
    // ceil(20*20/(2*sqrt(50^2+40^2)))=4, crossings0, no old penalties.
    assert_eq!(report["routing_metrics"]["wirelength_mm"], 20.0);
    assert_eq!(report["routing_metrics"]["proper_crossings"], 0);
    assert_eq!(report["routing_metrics"]["wire_deduction"], 4);
    assert_eq!(report["routing_metrics"]["crossing_deduction"], 0);
    assert_eq!(report["score"], 96);
    assert_eq!(report["score_incomplete"], false);
    assert_eq!(report["verdict"], "pass");
    assert_eq!(report["source"], "saved_file");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "scoring is read-only"
    );
}

#[tokio::test]
async fn a_shorter_topology_and_fewer_projected_crossings_improve_the_score() {
    let (_dir, path) = scene();
    let original = response(
        handle_score_placement(&json!({"board":path}), &ctx())
            .await
            .unwrap(),
    );
    let placements = [konnect_ipc::types::IpcFootprintPlacement {
        reference: "R2".into(),
        x: 110.0,
        y: 60.0,
        rotation: 0.0,
    }];
    let changed = apply_placements_to_content(TWO_R, &placements).unwrap();
    std::fs::write(&path, changed).unwrap();
    let longer = response(
        handle_score_placement(&json!({"board":path}), &ctx())
            .await
            .unwrap(),
    );
    // Independent endpoints produce sqrt221 +sqrt181 and a crossing at105,55.
    let expected = 221.0_f64.sqrt() + 181.0_f64.sqrt();
    assert!((longer["routing_metrics"]["wirelength_mm"].as_f64().unwrap() - expected).abs() < 1e-8);
    assert_eq!(longer["routing_metrics"]["proper_crossings"], 1);
    assert_eq!(longer["routing_metrics"]["crossing_deduction"], 2);
    assert_eq!(longer["score"], 93);
    assert!(original["score"].as_i64().unwrap() > longer["score"].as_i64().unwrap());
}

#[test]
fn incomplete_before_or_after_score_blocks_a_claimed_plan_improvement() {
    for (before, after) in [
        (
            json!({"score_incomplete":true}),
            json!({"score_incomplete":false}),
        ),
        (
            json!({"score_incomplete":false}),
            json!({"score_incomplete":true}),
        ),
        (json!({}), json!({"score_incomplete":false})),
    ] {
        let mut applicability = PlanApplicability::new();
        require_complete_score_comparison(&mut applicability, &before, &after);
        assert!(applicability.is_blocked());
    }
    let mut applicability = PlanApplicability::new();
    require_complete_score_comparison(
        &mut applicability,
        &json!({"score_incomplete":false}),
        &json!({"score_incomplete":false}),
    );
    assert!(!applicability.is_blocked());
}

fn changed_reference(source: &str, from: &str, to: &str) -> String {
    use konnect_sexp::writer::{apply_edits, find_direct_child_blocks, SexpEdit};
    let (start, end) = find_direct_child_blocks(source, "kicad_pcb")
        .into_iter()
        .find(|(a, b)| {
            let n = konnect_sexp::parse_sexp(&source[*a..*b]).unwrap();
            n.head() == Some("footprint")
                && n.find_all("property").iter().any(|p| {
                    p.get(1).and_then(SexpNode::as_str) == Some("Reference")
                        && p.get(2).and_then(SexpNode::as_str) == Some(from)
                })
        })
        .unwrap();
    let part = &source[start..end];
    let (a, b) = find_direct_child_blocks(part, "footprint")
        .into_iter()
        .find(|(a, b)| {
            let n = konnect_sexp::parse_sexp(&part[*a..*b]).unwrap();
            n.head() == Some("property") && n.get(1).and_then(SexpNode::as_str) == Some("Reference")
        })
        .unwrap();
    let old = &part[a..b];
    let replacement = old.replacen(&format!("\"{from}\""), &format!("\"{to}\""), 1);
    apply_edits(
        source.into(),
        vec![SexpEdit::replace(start + a, start + b, replacement)],
    )
}

#[tokio::test]
async fn duplicate_pad_bearing_references_are_not_a_trustworthy_zero() {
    let (_dir, path) = scene();
    let source = changed_reference(TWO_R, "R2", "R1");
    assert_eq!(
        ambiguous_pad_references(&konnect_sexp::parse_sexp(&source).unwrap()),
        vec!["R1"]
    );
    std::fs::write(&path, source).unwrap();
    let report = response(
        handle_score_placement(&json!({"board":path}), &ctx())
            .await
            .unwrap(),
    );
    assert_eq!(
        report["verdict"], "pass",
        "geometry remains the hard verdict"
    );
    assert_eq!(report["score_incomplete"], true);
    assert_eq!(report["routing_metrics"]["status"], "incomplete");
    assert!(report["routing_metrics"]["wirelength_mm"].is_null());
}

#[tokio::test]
async fn unavailable_wire_normalization_is_visible_even_with_valid_pad_metrics() {
    use konnect_sexp::writer::{apply_edits, find_direct_child_blocks, SexpEdit};
    let (_dir, path) = scene();
    let edits = find_direct_child_blocks(TWO_R, "kicad_pcb")
        .into_iter()
        .filter_map(|(a, b)| {
            let n = konnect_sexp::parse_sexp(&TWO_R[a..b]).unwrap();
            (n.head().is_some_and(|h| h.starts_with("gr_"))
                && n.find_str("layer") == Some("Edge.Cuts"))
            .then(|| SexpEdit::delete(a, b))
        })
        .collect();
    std::fs::write(&path, apply_edits(TWO_R.into(), edits)).unwrap();
    let report = response(
        handle_score_placement(&json!({"board":path}), &ctx())
            .await
            .unwrap(),
    );
    assert_eq!(report["verdict"], "outline_missing");
    assert_eq!(report["routing_metrics"]["wirelength_mm"], 20.0);
    assert!(report["routing_metrics"]["wire_deduction"].is_null());
    assert_eq!(report["score_incomplete"], true);
}

#[tokio::test]
async fn fanout_option_reaches_both_electrical_metric_families() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("placement.kicad_pcb");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../konnect-sexp/tests/fixtures/placement/placement_fixture.kicad_pcb"
        ),
        &path,
    )
    .unwrap();
    let report = response(
        handle_score_placement(&json!({"board":path,"max_net_references":2}), &ctx())
            .await
            .unwrap(),
    );
    assert_eq!(report["routing_metrics"]["max_net_references"], 2);
    assert!(
        report["routing_metrics"]["filtered"]["above_reference_limit"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(report["unproven_decoupling_caps"].as_array().unwrap().len() >= 2);
    assert_eq!(report["score_incomplete"], true);
}

#[test]
fn error_results_cannot_masquerade_as_high_quality_scores() {
    let payload = r#"{"score":100,"score_incomplete":false}"#;
    assert!(decode_score_result(&CallToolResult::error(payload)).is_err());
    assert_eq!(
        decode_score_result(&CallToolResult::text(payload)).unwrap()["score"],
        100
    );
    assert!(decode_score_result(&CallToolResult::image("QUJD", "image/png")).is_err());
}
