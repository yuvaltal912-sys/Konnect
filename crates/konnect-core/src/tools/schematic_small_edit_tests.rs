//! PR 6: handler checks on real KiCad saves, plus corruption of prospective
//! writes to prove that requested flags and mirror state are independently bound.
use super::*;
use std::path::PathBuf;

const MIRROR: &str = include_str!("../../tests/fixtures/mirror_fields_kicad10.kicad_sch");
const MULTI: &str = include_str!("../../tests/fixtures/ecc83_multiunit.kicad_sch");

fn scene(source: &str) -> (tempfile::TempDir, PathBuf, ToolContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ecc83-pp.kicad_sch");
    std::fs::write(&path, source).unwrap();
    let context = ToolContext::new(
        super::super::ServerConfig::default(),
        std::sync::Arc::new(crate::router::ToolRouter::new()),
    );
    (dir, path, context)
}

fn body(result: CallToolResult) -> serde_json::Value {
    assert!(!result.is_error, "{result:?}");
    let ToolContent::Text { text } = &result.content[0] else {
        panic!("expected JSON")
    };
    serde_json::from_str(text).unwrap()
}

#[test]
fn new_transform_schema_matches_the_direct_argument_contract() {
    let definitions = tools();
    let rotation = &definitions
        .iter()
        .find(|t| t.name == "rotate_schematic_component")
        .unwrap()
        .input_schema;
    // Clients inspect this metadata even though the enum independently
    // implies strings. Its removal control is recorded as structural proof.
    assert_eq!(rotation["properties"]["mirror"]["type"], "string");
    let validator = jsonschema::validator_for(rotation).unwrap();
    for extra in [
        json!({"mirror":"x"}),
        json!({"mirror":"none"}),
        json!({"rotation":90}),
        json!({"rotation":270,"mirror":"y"}),
    ] {
        let mut args = extra;
        args["schematic"] = json!("saved.kicad_sch");
        args["reference"] = json!("U1");
        assert!(validator.is_valid(&args), "{args}");
    }
    for extra in [
        json!({}),
        json!({"mirror":"X"}),
        json!({"mirror":null}),
        json!({"mirror":true}),
        json!({"rotation":"90"}),
    ] {
        let mut args = extra;
        args["schematic"] = json!("saved.kicad_sch");
        args["reference"] = json!("U1");
        assert!(!validator.is_valid(&args), "{args}");
    }
    let edits = &definitions
        .iter()
        .find(|t| t.name == "edit_schematic_component")
        .unwrap()
        .input_schema;
    let validator = jsonschema::validator_for(edits).unwrap();
    for key in ["dnp", "in_bom", "on_board", "exclude_from_sim"] {
        for value in [
            json!(true),
            json!(false),
            json!(null),
            json!("true"),
            json!(1),
        ] {
            let mut args = json!({"schematic":"saved.kicad_sch","reference":"U1"});
            args[key] = value.clone();
            assert_eq!(validator.is_valid(&args), value.is_boolean(), "{args}");
        }
    }
}

#[tokio::test]
async fn mirror_handler_matches_kicad_written_twins_and_can_clear_it() {
    let (_dir, path, ctx) = scene(MIRROR);
    for (reference, axis, twin) in [
        ("D1", "x", "D2"),
        ("U1", "y", "U2"),
        ("U3", "x", "U4"),
        ("U5", "y", "U6"),
        ("U7", "none", "U8"),
    ] {
        let result = body(
            handle_rotate_schematic_component(
                &json!({"schematic": path, "reference": reference, "mirror": axis}),
                &ctx,
            )
            .await
            .unwrap(),
        );
        assert_eq!(result["mirror_x"], axis == "x");
        assert_eq!(result["mirror_y"], axis == "y");
        let written = std::fs::read_to_string(&path).unwrap();
        let actual = super::rotate_field_text_tests::field_positions(&written, reference);
        let expected = super::rotate_field_text_tests::field_positions(MIRROR, twin)
            .map(|(x, y, r)| (x, y - 25.4, r));
        assert_eq!(
            super::rotate_field_text_tests::to_nanometres(actual),
            super::rotate_field_text_tests::to_nanometres(expected),
            "{reference}"
        );
        assert!(!written.contains("(mirror none)"));
    }
}

#[tokio::test]
async fn mirror_changes_all_units_and_omitted_transform_fields_are_preserved() {
    let (_dir, path, ctx) = scene(MULTI);
    let before = cse::Schematic::load(&path).unwrap();
    let rotations: BTreeMap<_, _> = before
        .symbols
        .iter()
        .filter(|s| s.reference() == Some("U1"))
        .map(|s| (s.uuid.clone(), s.at.rotation.unwrap_or(0.0)))
        .collect();
    assert!(rotations.len() > 1);
    let mirrored = body(
        handle_rotate_schematic_component(
            &json!({"schematic":path,"reference":"U1","mirror":"y"}),
            &ctx,
        )
        .await
        .unwrap(),
    );
    for unit in mirrored["units"].as_array().unwrap() {
        assert_eq!(
            unit["rotation"].as_f64().unwrap(),
            rotations[unit["uuid"].as_str().unwrap()]
        );
        assert_eq!(unit["mirror_y"], true);
    }
    let rotated = body(
        handle_rotate_schematic_component(
            &json!({"schematic":path,"reference":"U1","rotation":90}),
            &ctx,
        )
        .await
        .unwrap(),
    );
    assert!(rotated["units"]
        .as_array()
        .unwrap()
        .iter()
        .all(|u| u["mirror_y"] == true));
    let combined = body(
        handle_rotate_schematic_component(
            &json!({"schematic":path,"reference":"U1","rotation":180,"mirror":"x"}),
            &ctx,
        )
        .await
        .unwrap(),
    );
    assert_eq!(combined["rotation"], 180.0);
    assert!(combined["units"]
        .as_array()
        .unwrap()
        .iter()
        .all(|u| u["mirror_x"] == true && u["mirror_y"] == false));
}

#[tokio::test]
async fn malformed_transform_or_flags_refuse_before_any_write() {
    let (_dir, path, ctx) = scene(MULTI);
    let before = std::fs::read(&path).unwrap();
    for extra in [
        json!({}),
        json!({"mirror":null}),
        json!({"mirror":"X"}),
        json!({"mirror":true}),
        json!({"rotation":"90"}),
        json!({"rotation":null}),
    ] {
        let mut args = extra;
        args["schematic"] = json!(path);
        args["reference"] = json!("U1");
        assert!(
            handle_rotate_schematic_component(&args, &ctx)
                .await
                .unwrap()
                .is_error
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    for field in ["dnp", "in_bom", "on_board", "exclude_from_sim"] {
        for value in [json!(null), json!(1), json!("true")] {
            let mut args = json!({"schematic":path,"reference":"U1","value":"must-not-land"});
            args[field] = value;
            assert!(
                handle_edit_schematic_component(&args, &ctx)
                    .await
                    .unwrap()
                    .is_error
            );
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
    }
}

#[tokio::test]
async fn flags_are_component_wide_and_get_and_list_report_saved_state() {
    let (_dir, path, ctx) = scene(MULTI);
    let old = read_consistent(&path).unwrap();
    let indexed = indexed_uuid_items(&path, &old).unwrap();
    let targets = component_target_from_source(&path, &old, "U1")
        .unwrap()
        .uuids();
    let response=body(handle_edit_schematic_component(&json!({"schematic":path,"reference":"U1","dnp":true,"in_bom":false,"on_board":false,"exclude_from_sim":true,"unit":2}),&ctx).await.unwrap());
    let expected = json!({"dnp":true,"in_bom":false,"on_board":false,"exclude_from_sim":true});
    assert_eq!(response["flags"], expected);
    assert!(response["units"]
        .as_array()
        .unwrap()
        .iter()
        .all(|u| u["flags"] == expected));
    let get = body(
        handle_get_schematic_component(&json!({"schematic":path,"reference":"U1"}), &ctx)
            .await
            .unwrap(),
    );
    assert_eq!(get["flags"], expected);
    assert!(get["units"]
        .as_array()
        .unwrap()
        .iter()
        .all(|u| u["flags"] == expected));
    let list = body(
        handle_list_schematic_components(&json!({"schematic":path}), &ctx)
            .await
            .unwrap(),
    );
    let items = list["components"].as_array().unwrap();
    let units: Vec<_> = items.iter().filter(|u| u["reference"] == "U1").collect();
    assert_eq!(units.len(), 3);
    assert!(units.iter().all(|u| u["flags"] == expected));
    let after = read_consistent(&path).unwrap();
    let after_index = indexed_uuid_items(&path, &after).unwrap();
    for (uuid, item) in indexed {
        if !targets.contains(&uuid) {
            assert_eq!(after_index[&uuid].source, item.source, "unrelated {uuid}");
        }
    }
    let cleared = body(
        handle_edit_schematic_component(
            &json!({"schematic":path,"reference":"U1","dnp":false}),
            &ctx,
        )
        .await
        .unwrap(),
    );
    assert_eq!(cleared["flags"]["dnp"], false);
    assert_eq!(cleared["flags"]["in_bom"], false);
    assert_eq!(cleared["flags"]["on_board"], false);
    assert_eq!(cleared["flags"]["exclude_from_sim"], true);
}

#[tokio::test]
async fn omitted_simulation_flag_keeps_older_sheets_without_the_token() {
    let (_dir, path, ctx) = scene(MIRROR);
    // A typed legacy variation of the real KiCad fixture: missing means
    // missing, rather than an implicit request to add a modern token.
    let mut before = cse::Schematic::load(&path).unwrap();
    let reference = "D1";
    before
        .symbols
        .iter_mut()
        .find(|s| s.reference() == Some(reference))
        .unwrap()
        .exclude_from_sim = None;
    before.overwrite().unwrap();
    let response = body(
        handle_edit_schematic_component(
            &json!({"schematic":path,"reference":reference,"dnp":true}),
            &ctx,
        )
        .await
        .unwrap(),
    );
    assert!(response["flags"]["exclude_from_sim"].is_null());
    assert!(cse::Schematic::load(&path)
        .unwrap()
        .symbols
        .iter()
        .filter(|s| s.reference() == Some(reference))
        .all(|s| s.exclude_from_sim.is_none()));
}

#[tokio::test]
async fn flags_and_mirror_prospective_corruption_refuse_without_writing() {
    let (_dir, path, ctx) = scene(MULTI);
    let before = std::fs::read(&path).unwrap();
    COMPONENT_PROSPECTIVE_FAULT
        .with(|f| *f.borrow_mut() = Some(("(dnp yes)".into(), "(dnp no)".into())));
    let result = handle_edit_schematic_component(
        &json!({"schematic":path,"reference":"U1","dnp":true}),
        &ctx,
    )
    .await
    .unwrap();
    assert!(result.is_error);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    COMPONENT_PROSPECTIVE_FAULT.with(|f| {
        *f.borrow_mut() = Some((
            "(at 160.02 64.77 0)\n\t\t(mirror y)".into(),
            "(at 160.02 64.77 0)\n\t\t(mirror x)".into(),
        ))
    });
    let result = handle_rotate_schematic_component(
        &json!({"schematic":path,"reference":"U1","mirror":"y"}),
        &ctx,
    )
    .await
    .unwrap();
    assert!(result.is_error);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

fn embedded_mut<'a>(
    schematic: &'a mut cse::Schematic,
    lib_id: &str,
) -> &'a mut cse::sexp::SexpNode {
    let lib = schematic
        .raw_other
        .iter_mut()
        .find(|n| n.tag() == Some("lib_symbols"))
        .unwrap();
    let cse::sexp::SexpNode::List(children) = lib else {
        unreachable!()
    };
    children
        .iter_mut()
        .find(|n| n.tag() == Some("symbol") && n.value() == Some(lib_id))
        .unwrap()
}

#[test]
fn placement_copies_explicit_library_flags_and_keeps_missing_defaults() {
    let (_dir, path, _) = scene(MIRROR);
    let mut sch = cse::Schematic::load(&path).unwrap();
    let lib = embedded_mut(&mut sch, "Device:LED");
    let cse::sexp::SexpNode::List(children) = lib else {
        unreachable!()
    };
    children.retain(|n| {
        !matches!(
            n.tag(),
            Some("in_bom" | "on_board" | "dnp" | "exclude_from_sim")
        )
    });
    for (key, value) in [
        ("in_bom", "no"),
        ("on_board", "no"),
        ("dnp", "yes"),
        ("exclude_from_sim", "yes"),
    ] {
        children.push(cse::sexp::tagged(key, vec![cse::sexp::atom(value)]));
    }
    let src = |_: &str| Vec::<PathBuf>::new();
    let placed = place_one_component(
        &mut sch,
        &[],
        "ecc83-pp",
        "Device:LED",
        30.0,
        30.0,
        0.0,
        None,
        Some("D99"),
        None,
        None,
        1,
        &src,
    )
    .unwrap();
    let actual = sch.symbols.iter().find(|s| s.uuid == placed.uuid).unwrap();
    let expected = SymbolFlags {
        dnp: true,
        in_bom: false,
        on_board: false,
        exclude_from_sim: Some(true),
    };
    assert_eq!(symbol_flags(actual), expected);
    assert_eq!(placed.fields.flags, expected);
    let lib = embedded_mut(&mut sch, "Device:LED");
    let cse::sexp::SexpNode::List(children) = lib else {
        unreachable!()
    };
    children.retain(|n| {
        !matches!(
            n.tag(),
            Some("in_bom" | "on_board" | "dnp" | "exclude_from_sim")
        )
    });
    let placed = place_one_component(
        &mut sch,
        &[],
        "ecc83-pp",
        "Device:LED",
        40.0,
        30.0,
        0.0,
        None,
        Some("D100"),
        None,
        None,
        1,
        &src,
    )
    .unwrap();
    let actual = sch.symbols.iter().find(|s| s.uuid == placed.uuid).unwrap();
    assert_eq!(symbol_flags(actual), SymbolFlags::default());
}

#[test]
fn adding_a_unit_inherits_current_package_flags_and_refuses_inconsistent_units() {
    let (dir, path, _) = scene(MULTI);
    let mut sch = cse::Schematic::load(&path).unwrap();
    let mut definition = embedded_mut(&mut sch, "ecc83-pp:ECC83").clone();
    let cse::sexp::SexpNode::List(children) = &mut definition else {
        unreachable!()
    };
    children[1] = cse::sexp::qstr("ECC83");
    let library = dir.path().join("ecc83-pp.kicad_sym");
    let lib = cse::sexp::tagged("kicad_symbol_lib", vec![definition]);
    std::fs::write(&library, cse::sexp::writer::write(&lib)).unwrap();
    let src = move |_: &str| vec![library.clone()];
    let power_uuid = sch
        .symbols
        .iter()
        .find(|s| s.reference() == Some("U1") && s.unit == 3)
        .unwrap()
        .uuid
        .clone();
    sch.symbols.remove_by_uuid(&power_uuid).unwrap();
    let expected = SymbolFlags {
        dnp: true,
        in_bom: false,
        on_board: false,
        exclude_from_sim: Some(true),
    };
    for s in sch
        .symbols
        .iter_mut()
        .filter(|s| s.reference() == Some("U1"))
    {
        apply_symbol_flags(s, expected);
    }
    let placed = place_one_component(
        &mut sch,
        &[],
        "ecc83-pp",
        "ecc83-pp:ECC83",
        30.0,
        30.0,
        0.0,
        None,
        Some("U1"),
        None,
        None,
        3,
        &src,
    )
    .unwrap();
    assert_eq!(placed.fields.flags, expected);
    assert_eq!(
        symbol_flags(sch.symbols.iter().find(|s| s.uuid == placed.uuid).unwrap()),
        expected
    );
    sch.symbols.remove_by_uuid(&placed.uuid).unwrap();
    sch.symbols
        .iter_mut()
        .find(|s| s.reference() == Some("U1") && s.unit == 2)
        .unwrap()
        .dnp = false;
    let count = sch.symbols.len();
    let result = place_one_component(
        &mut sch,
        &[],
        "ecc83-pp",
        "ecc83-pp:ECC83",
        30.0,
        30.0,
        0.0,
        None,
        Some("U1"),
        None,
        None,
        3,
        &src,
    );
    assert!(result.is_err());
    assert_eq!(sch.symbols.len(), count);
}

#[test]
fn additional_unit_accepts_equivalent_legacy_simulation_flags() {
    let (dir, path, _) = scene(MULTI);
    let mut sch = cse::Schematic::load(&path).unwrap();
    let mut definition = embedded_mut(&mut sch, "ecc83-pp:ECC83").clone();
    let cse::sexp::SexpNode::List(children) = &mut definition else {
        unreachable!()
    };
    children[1] = cse::sexp::qstr("ECC83");
    let library = dir.path().join("ecc83-pp.kicad_sym");
    std::fs::write(
        &library,
        cse::sexp::writer::write(&cse::sexp::tagged("kicad_symbol_lib", vec![definition])),
    )
    .unwrap();
    let src = move |_: &str| vec![library.clone()];
    let power_uuid = sch
        .symbols
        .iter()
        .find(|s| s.reference() == Some("U1") && s.unit == 3)
        .unwrap()
        .uuid
        .clone();
    sch.symbols.remove_by_uuid(&power_uuid).unwrap();
    for s in sch
        .symbols
        .iter_mut()
        .filter(|s| s.reference() == Some("U1"))
    {
        s.exclude_from_sim = if s.unit == 1 { None } else { Some(false) };
    }
    let placed = place_one_component(
        &mut sch,
        &[],
        "ecc83-pp",
        "ecc83-pp:ECC83",
        30.0,
        30.0,
        0.0,
        None,
        Some("U1"),
        None,
        None,
        3,
        &src,
    )
    .unwrap();
    assert_eq!(
        placed.fields.flags.exclude_from_sim, None,
        "new unit follows the lowest-numbered existing unit's representation"
    );
    assert_eq!(
        sch.symbols
            .iter()
            .find(|s| s.reference() == Some("U1") && s.unit == 2)
            .unwrap()
            .exclude_from_sim,
        Some(false),
        "an untouched unit keeps its explicit token"
    );
    // Effective false and true remain different even though absent and
    // explicit false are interchangeable for consistency checking.
    sch.symbols.remove_by_uuid(&placed.uuid).unwrap();
    sch.symbols
        .iter_mut()
        .find(|s| s.reference() == Some("U1") && s.unit == 2)
        .unwrap()
        .exclude_from_sim = Some(true);
    let count = sch.symbols.len();
    assert!(place_one_component(
        &mut sch,
        &[],
        "ecc83-pp",
        "ecc83-pp:ECC83",
        30.0,
        30.0,
        0.0,
        None,
        Some("U1"),
        None,
        None,
        3,
        &src
    )
    .is_err());
    assert_eq!(sch.symbols.len(), count);
}

#[tokio::test]
async fn mirror_carries_the_no_connect_to_its_reflected_pin() {
    const SOURCE: &str = include_str!("../../tests/fixtures/no_connect_carry_kicad10.kicad_sch");
    let (_dir, path, ctx) = scene(SOURCE);
    let result = body(
        handle_rotate_schematic_component(
            &json!({"schematic":path,"reference":"R2","mirror":"x"}),
            &ctx,
        )
        .await
        .unwrap(),
    );
    assert_eq!(result["no_connects_moved_count"], 1);
    let marker = &result["no_connects_moved"][0];
    assert_eq!(marker["uuid"], "1a251b9a-30be-43fa-bf5f-04706ed82788");
    assert_eq!(marker["to"], json!({"x":158.75,"y":163.83}));
    let saved = cse::Schematic::load(&path).unwrap();
    let marker = saved
        .no_connects
        .iter()
        .find(|n| n.uuid == "1a251b9a-30be-43fa-bf5f-04706ed82788")
        .unwrap();
    assert_eq!((marker.x, marker.y), (158.75, 163.83));
}

#[tokio::test]
async fn post_commit_readback_failure_reports_uncertainty_for_flags_and_mirror() {
    for mirror in [false, true] {
        let (_dir, path, ctx) = scene(MULTI);
        COMPONENT_COMMITTED_LOAD_FAULT
            .with(|f| *f.borrow_mut() = Some("controlled saved-file failure".into()));
        let result = if mirror {
            handle_rotate_schematic_component(
                &json!({"schematic":path,"reference":"U1","mirror":"y"}),
                &ctx,
            )
            .await
            .unwrap()
        } else {
            handle_edit_schematic_component(
                &json!({"schematic":path,"reference":"U1","dnp":true}),
                &ctx,
            )
            .await
            .unwrap()
        };
        assert!(result.is_error);
        assert_eq!(
            crate::mcp::error::extract_error_kind(&result).as_deref(),
            Some("mutation_outcome_uncertain")
        );
        let saved = cse::Schematic::load(&path).unwrap();
        assert!(saved
            .symbols
            .iter()
            .filter(|s| s.reference() == Some("U1"))
            .all(|s| if mirror {
                s.mirror.as_deref() == Some("y")
            } else {
                s.dnp
            }));
    }
}
