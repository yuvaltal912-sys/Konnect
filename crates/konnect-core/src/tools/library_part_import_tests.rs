use super::*;
use crate::mcp::protocol::ToolContent;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn ctx() -> ToolContext {
    ToolContext::new(
        crate::tools::ServerConfig::default(),
        std::sync::Arc::new(crate::router::ToolRouter::new()),
    )
}

fn text(result: &CallToolResult) -> String {
    match &result.content[0] {
        ToolContent::Text { text } => text.clone(),
        other => panic!("unexpected content {other:?}"),
    }
}

fn ok(result: CallToolResult) -> Value {
    assert!(!result.is_error, "{}", text(&result));
    serde_json::from_str(&text(&result)).unwrap()
}

fn err_kind(result: &CallToolResult) -> String {
    assert!(result.is_error, "expected an error, got {}", text(result));
    let body: Value = serde_json::from_str(&text(result)).unwrap_or(Value::Null);
    body["error"]["kind"]
        .as_str()
        .or_else(|| body["kind"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| text(result))
}

struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

fn project() -> Project {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap().join("board");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("board.kicad_pro"), "{}\n").unwrap();
    Project { _dir: dir, root }
}

impl Project {
    fn pro(&self) -> String {
        self.root.join("board.kicad_pro").display().to_string()
    }
}

fn ldo_args(project: &Project) -> Value {
    json!({
        "project": project.pro(),
        "symbol_file": fixture("part_import/TEST_LDO.kicad_sym").display().to_string(),
        "footprint_file": fixture("part_import/SOT-23_TEST.kicad_mod").display().to_string(),
    })
}

async fn call(args: Value) -> CallToolResult {
    handle_import_library_part(&args, &ctx()).await.unwrap()
}

async fn preview(args: &Value) -> Value {
    ok(call(args.clone()).await)
}

async fn apply(args: &Value, revision: &str) -> CallToolResult {
    let mut args = args.clone();
    args["dry_run"] = json!(false);
    args["expected_revision"] = json!(revision);
    call(args).await
}

/// Every file under the project, with its bytes; a link is recorded by its
/// target, not followed.
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_symlink() {
                out.insert(
                    format!("{} ->", path.strip_prefix(base).unwrap().display()),
                    std::fs::read_link(&path)
                        .unwrap()
                        .display()
                        .to_string()
                        .into_bytes(),
                );
            } else if path.is_dir() {
                out.insert(
                    format!("{}/", path.strip_prefix(base).unwrap().display()),
                    Vec::new(),
                );
                walk(base, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(base).unwrap().display().to_string(),
                    std::fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[tokio::test]
async fn preview_shows_targets_pins_pads_and_check_without_writing() {
    let project = project();
    let before = snapshot(&project.root);
    let mut args = ldo_args(&project);
    args["model_file"] = json!(fixture("part_import/SOT-23_TEST.step")
        .display()
        .to_string());
    args["lcsc"] = json!("c6186");
    let body = preview(&args).await;

    assert_eq!(
        snapshot(&project.root),
        before,
        "a preview must write nothing"
    );
    assert_eq!(body["dry_run"], true);
    assert_eq!(body["can_apply"], true, "{body:#}");
    assert!(body["revision"].as_str().unwrap().starts_with("sha256:"));
    assert_eq!(
        body["network"],
        "none: the tool reads only the local files given"
    );

    let kinds: Vec<&str> = body["writes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "model",
            "footprint",
            "symbol_library",
            "symbol_lib_table",
            "footprint_lib_table"
        ]
    );
    assert!(body["writes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|w| w["action"] == "create"));

    assert_eq!(body["symbol"]["name"], "TEST_LDO_3V3");
    assert_eq!(body["symbol"]["lib_id"], "imported_parts:TEST_LDO_3V3");
    assert_eq!(body["symbol"]["pin_count"], 3);
    let pins: Vec<(&str, &str, &str)> = body["symbol"]["pins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["number"].as_str().unwrap(),
                p["name"].as_str().unwrap(),
                p["type"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        pins,
        [
            ("1", "GND", "power_in"),
            ("2", "VOUT", "power_out"),
            ("3", "VIN", "power_in")
        ]
    );
    assert_eq!(
        body["symbol"]["footprint_field_before"],
        "vendor_lib:SOT-23-3_L2.9-W1.3-P1.90-LS2.4-BR"
    );
    assert_eq!(
        body["symbol"]["fields_set"]["Footprint"],
        "imported_parts:SOT-23_TEST"
    );
    assert_eq!(body["symbol"]["fields_set"]["LCSC"], "C6186");

    assert_eq!(body["footprint"]["pad_count"], 3);
    let pad = &body["footprint"]["pads"][0];
    assert_eq!(pad["number"], "1");
    assert_eq!(pad["type"], "smd");
    assert_eq!(pad["shape"], "roundrect");
    assert_eq!(pad["x"], -0.95);
    assert_eq!(pad["width"], 0.6);
    assert_eq!(body["footprint"]["model"]["action"], "create");
    assert_eq!(
        body["footprint"]["model"]["reference"],
        "${KIPRJMOD}/imported_parts.3dshapes/SOT-23_TEST.step"
    );

    assert_eq!(body["pin_pad_check"]["result"], "pass");
    assert_eq!(body["pin_pad_check"]["source"], "candidate");
    assert_eq!(
        body["registrations"]["symbol"]["uri"],
        "${KIPRJMOD}/imported_parts.kicad_sym"
    );
    assert_eq!(
        body["registrations"]["footprint"]["uri"],
        "${KIPRJMOD}/imported_parts.pretty"
    );
    assert_eq!(body["registrations"]["symbol"]["state"], "inserted");
}

#[tokio::test]
async fn apply_writes_registers_keeps_model_transform_and_reads_back() {
    let project = project();
    let mut args = ldo_args(&project);
    args["model_file"] = json!(fixture("part_import/SOT-23_TEST.step")
        .display()
        .to_string());
    args["lcsc"] = json!("C6186");
    args["mpn"] = json!("TEST-LDO-33");
    args["manufacturer"] = json!("Example Semi");
    let revision = preview(&args).await["revision"]
        .as_str()
        .unwrap()
        .to_string();
    let body = ok(apply(&args, &revision).await);

    assert_eq!(body["dry_run"], false);
    assert_eq!(body["pin_pad_check"]["result"], "pass");
    assert_eq!(body["pin_pad_check"]["source"], "saved_file");
    assert_eq!(body["written"].as_array().unwrap().len(), 5);
    assert!(body["next"]
        .as_str()
        .unwrap()
        .contains("imported_parts:TEST_LDO_3V3"));

    let library = std::fs::read_to_string(project.root.join("imported_parts.kicad_sym")).unwrap();
    let root = correctness::library(&library).unwrap();
    let symbol = root.find_all("symbol")[0];
    let fields = properties_of(symbol);
    assert_eq!(fields["Footprint"], "imported_parts:SOT-23_TEST");
    assert_eq!(fields["LCSC"], "C6186");
    assert_eq!(fields["MPN"], "TEST-LDO-33");
    assert_eq!(fields["Manufacturer"], "Example Semi");
    assert_eq!(fields["Value"], "TEST_LDO_3V3", "other fields are kept");
    assert_eq!(
        version_of(&root),
        Some(20241209),
        "a new library keeps the source format"
    );

    let footprint = std::fs::read_to_string(
        project
            .root
            .join("imported_parts.pretty/SOT-23_TEST.kicad_mod"),
    )
    .unwrap();
    let footprint = parse_sexp(&footprint).unwrap();
    let models = footprint.find_all("model");
    assert_eq!(models.len(), 1);
    assert_eq!(
        models[0].get(1).and_then(SexpNode::as_str),
        Some("${KIPRJMOD}/imported_parts.3dshapes/SOT-23_TEST.step")
    );
    assert_eq!(xyz_of(models[0], "offset", [9.0; 3]), [0.0, 0.0, 0.05]);
    assert_eq!(xyz_of(models[0], "rotate", [9.0; 3]), [-90.0, 0.0, 180.0]);
    assert_eq!(
        std::fs::read(
            project
                .root
                .join("imported_parts.3dshapes/SOT-23_TEST.step")
        )
        .unwrap(),
        std::fs::read(fixture("part_import/SOT-23_TEST.step")).unwrap()
    );

    for (table, uri) in [
        ("sym-lib-table", "${KIPRJMOD}/imported_parts.kicad_sym"),
        ("fp-lib-table", "${KIPRJMOD}/imported_parts.pretty"),
    ] {
        let entries = parse_lib_table(&std::fs::read_to_string(project.root.join(table)).unwrap());
        assert_eq!(entries.len(), 1, "{table}");
        assert_eq!(entries[0]["nickname"], "imported_parts");
        assert_eq!(entries[0]["uri"], uri);
    }
    assert!(!journal_pending(&project.root));

    // The shipped check tool agrees with the saved result.
    let check = ok(correctness_check(&project).await);
    assert_eq!(check["result"], "pass");
}

async fn correctness_check(project: &Project) -> CallToolResult {
    let tool = super::super::tools()
        .into_iter()
        .find(|t| t.name == "check_symbol_footprint")
        .unwrap();
    let args = json!({
        "library_path": project.root.join("imported_parts.kicad_sym").display().to_string(),
        "symbol_name": "TEST_LDO_3V3",
        "footprint_path": project.root.join("imported_parts.pretty/SOT-23_TEST.kicad_mod").display().to_string(),
    });
    (tool.handler)(&args, std::sync::Arc::new(ctx()))
        .await
        .unwrap()
}

#[tokio::test]
async fn apply_needs_the_preview_revision_and_refuses_a_stale_one() {
    let project = project();
    let args = ldo_args(&project);
    let before = snapshot(&project.root);

    let mut missing = args.clone();
    missing["dry_run"] = json!(false);
    assert_eq!(err_kind(&call(missing).await), "invalid_argument");

    assert_eq!(
        err_kind(&apply(&args, "sha256:not-the-revision").await),
        "stale_target"
    );
    assert_eq!(
        snapshot(&project.root),
        before,
        "a refused apply writes nothing"
    );

    // A revision for one request does not apply another.
    let revision = preview(&args).await["revision"]
        .as_str()
        .unwrap()
        .to_string();
    let mut other = args.clone();
    other["lcsc"] = json!("C1");
    assert_eq!(err_kind(&apply(&other, &revision).await), "stale_target");

    // A table edited between preview and apply invalidates the revision.
    std::fs::write(
        project.root.join("sym-lib-table"),
        "(sym_lib_table\n  (version 7)\n  (lib (name \"other\") (type \"KiCad\") (uri \"${KIPRJMOD}/other.kicad_sym\") (options \"\") (descr \"\"))\n)\n",
    )
    .unwrap();
    assert_eq!(err_kind(&apply(&args, &revision).await), "stale_target");
    assert!(!project.root.join("imported_parts.kicad_sym").exists());
    assert!(!project.root.join("imported_parts.pretty").exists());

    // A fresh preview over the edited table keeps the other entry.
    let body = preview(&args).await;
    let revision = body["revision"].as_str().unwrap().to_string();
    assert_eq!(body["writes"][2]["action"], "register");
    ok(apply(&args, &revision).await);
    let entries =
        parse_lib_table(&std::fs::read_to_string(project.root.join("sym-lib-table")).unwrap());
    let names: Vec<_> = entries
        .iter()
        .map(|e| e["nickname"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["other", "imported_parts"]);
}

#[tokio::test]
async fn a_second_part_appends_to_the_library_and_reuses_the_registration() {
    let project = project();
    let args = ldo_args(&project);
    let revision = preview(&args).await["revision"]
        .as_str()
        .unwrap()
        .to_string();
    ok(apply(&args, &revision).await);
    let tables_before = (
        std::fs::read(project.root.join("sym-lib-table")).unwrap(),
        std::fs::read(project.root.join("fp-lib-table")).unwrap(),
    );

    // Real KiCad 10 files: tab-indented, newer format than the first part.
    let second = json!({
        "project": project.root.display().to_string(),
        "symbol_file": fixture("reference_prefix_kicad10/Device.kicad_sym").display().to_string(),
        "footprint_file": fixture("c_0603_1608metric_kicad10.kicad_mod").display().to_string(),
    });
    let body = preview(&second).await;
    assert_eq!(body["can_apply"], true, "{body:#}");
    assert_eq!(body["registrations"]["symbol"]["state"], "unchanged");
    assert_eq!(body["registrations"]["footprint"]["state"], "unchanged");
    let actions: Vec<_> = body["writes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| (w["kind"].as_str().unwrap(), w["action"].as_str().unwrap()))
        .collect();
    assert_eq!(
        actions,
        [("footprint", "create"), ("symbol_library", "append_symbol")]
    );
    assert!(body["warnings"].as_array().unwrap().iter().any(|w| w
        .as_str()
        .unwrap()
        .contains("rises from 20241209 to 20251024")));
    // The KiCad library model path uses a variable this tool cannot check.
    assert_eq!(
        body["footprint"]["model"]["kept_references"][0]["state"],
        "uses_a_path_variable"
    );

    ok(apply(&second, body["revision"].as_str().unwrap()).await);
    let library = std::fs::read_to_string(project.root.join("imported_parts.kicad_sym")).unwrap();
    let root = correctness::library(&library).unwrap();
    let names: Vec<_> = top_level_symbols(&library)
        .into_iter()
        .map(|s| s.0)
        .collect();
    assert_eq!(names, ["TEST_LDO_3V3", "R"]);
    assert_eq!(version_of(&root), Some(20251024));
    assert_eq!(
        properties_of(root.find_all("symbol")[1])["Footprint"],
        "imported_parts:C_0603_1608Metric"
    );
    assert_eq!(
        (
            std::fs::read(project.root.join("sym-lib-table")).unwrap(),
            std::fs::read(project.root.join("fp-lib-table")).unwrap(),
        ),
        tables_before,
        "an existing registration is not rewritten"
    );
}

#[tokio::test]
async fn an_existing_symbol_footprint_or_model_blocks_and_is_never_overwritten() {
    let project = project();
    let mut args = ldo_args(&project);
    args["model_file"] = json!(fixture("part_import/SOT-23_TEST.step")
        .display()
        .to_string());
    let revision = preview(&args).await["revision"]
        .as_str()
        .unwrap()
        .to_string();
    ok(apply(&args, &revision).await);
    let after_first = snapshot(&project.root);

    let body = preview(&args).await;
    assert_eq!(body["can_apply"], false);
    let blockers: Vec<&str> = body["blocked_by"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_str().unwrap())
        .collect();
    assert_eq!(blockers.len(), 1, "{blockers:?}");
    assert!(blockers[0].starts_with("symbol_exists:"), "{blockers:?}");
    // The same footprint and model bytes are reused, not a conflict.
    assert_eq!(body["footprint"]["action"], "reuse_identical");
    assert_eq!(body["footprint"]["model"]["action"], "reuse_identical");
    let refused = apply(&args, body["revision"].as_str().unwrap()).await;
    assert_eq!(err_kind(&refused), "plan_blocked");
    assert_eq!(snapshot(&project.root), after_first);

    // A different footprint with the same name blocks and is kept.
    let scratch = tempfile::tempdir().unwrap();
    let changed_footprint = scratch.path().join("changed.kicad_mod");
    std::fs::write(
        &changed_footprint,
        std::fs::read_to_string(fixture("part_import/SOT-23_TEST.kicad_mod"))
            .unwrap()
            .replace("(at 0 -1.1)", "(at 0 -1.2)"),
    )
    .unwrap();
    let mut changed = args.clone();
    changed["footprint_file"] = json!(changed_footprint.display().to_string());
    let body = preview(&changed).await;
    assert_eq!(body["footprint"]["action"], "blocked");
    assert!(
        body["blocked_by"].as_array().unwrap().iter().any(|b| b
            .as_str()
            .unwrap()
            .starts_with("footprint_exists:")
            && b.as_str().unwrap().contains("different footprint")),
        "{body:#}"
    );
    let refused = apply(&changed, body["revision"].as_str().unwrap()).await;
    assert_eq!(err_kind(&refused), "plan_blocked");
    assert_eq!(snapshot(&project.root), after_first);

    // A different model with the same name blocks too.
    let scratch = tempfile::tempdir().unwrap();
    let other_model = scratch.path().join("SOT-23_TEST.step");
    std::fs::write(
        &other_model,
        "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n/* other */\nENDSEC;\nEND-ISO-10303-21;\n",
    )
    .unwrap();
    let other_footprint = scratch.path().join("other.kicad_mod");
    std::fs::write(
        &other_footprint,
        std::fs::read_to_string(fixture("part_import/SOT-23_TEST.kicad_mod"))
            .unwrap()
            .replace("(footprint \"SOT-23_TEST\"", "(footprint \"SOT-23_OTHER\""),
    )
    .unwrap();
    let mut other = args.clone();
    other["model_file"] = json!(other_model.display().to_string());
    other["footprint_file"] = json!(other_footprint.display().to_string());
    let body = preview(&other).await;
    assert!(body["blocked_by"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b.as_str().unwrap().starts_with("model_exists:")));
    assert_eq!(body["footprint"]["model"]["action"], "blocked");
}

/// The fixture LDO under another name, so a second import is a new symbol.
fn renamed_ldo(dir: &Path) -> String {
    let path = dir.join("TEST_LDO_5V0.kicad_sym");
    std::fs::write(
        &path,
        std::fs::read_to_string(fixture("part_import/TEST_LDO.kicad_sym"))
            .unwrap()
            .replace("TEST_LDO_3V3", "TEST_LDO_5V0"),
    )
    .unwrap();
    path.display().to_string()
}

#[tokio::test]
async fn a_second_part_sharing_an_identical_footprint_reuses_it() {
    let project = project();
    let mut args = ldo_args(&project);
    args["model_file"] = json!(fixture("part_import/SOT-23_TEST.step")
        .display()
        .to_string());
    let revision = preview(&args).await["revision"]
        .as_str()
        .unwrap()
        .to_string();
    ok(apply(&args, &revision).await);
    let footprint_path = project
        .root
        .join("imported_parts.pretty/SOT-23_TEST.kicad_mod");
    let footprint_before = std::fs::read(&footprint_path).unwrap();

    let scratch = tempfile::tempdir().unwrap();
    let mut second = args.clone();
    second["symbol_file"] = json!(renamed_ldo(scratch.path()));
    let body = preview(&second).await;
    assert_eq!(body["can_apply"], true, "{body:#}");
    assert_eq!(body["footprint"]["action"], "reuse_identical");
    assert_eq!(body["footprint"]["model"]["action"], "reuse_identical");
    let kinds: Vec<&str> = body["writes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["symbol_library"], "only the symbol is written");

    // A change to the reused footprint is caught: it becomes a
    // footprint_exists blocker, so the revision differs.
    let revision = body["revision"].as_str().unwrap().to_string();
    std::fs::write(&footprint_path, b"(footprint \"SOT-23_TEST\")\n").unwrap();
    assert_eq!(err_kind(&apply(&second, &revision).await), "stale_target");
    std::fs::write(&footprint_path, &footprint_before).unwrap();

    let saved = ok(apply(&second, &revision).await);
    assert_eq!(
        saved["footprint_reused"],
        footprint_path.display().to_string()
    );
    assert_eq!(saved["pin_pad_check"]["result"], "pass");
    assert_eq!(std::fs::read(&footprint_path).unwrap(), footprint_before);
    let library = std::fs::read_to_string(project.root.join("imported_parts.kicad_sym")).unwrap();
    let root = correctness::library(&library).unwrap();
    let footprints: Vec<_> = root
        .find_all("symbol")
        .into_iter()
        .map(|symbol| {
            (
                symbol
                    .get(1)
                    .and_then(SexpNode::as_str)
                    .unwrap()
                    .to_string(),
                properties_of(symbol)["Footprint"].clone(),
            )
        })
        .collect();
    assert_eq!(
        footprints,
        [
            (
                "TEST_LDO_3V3".to_string(),
                "imported_parts:SOT-23_TEST".to_string()
            ),
            (
                "TEST_LDO_5V0".to_string(),
                "imported_parts:SOT-23_TEST".to_string()
            )
        ]
    );
}

#[tokio::test]
async fn a_nickname_bound_to_another_library_blocks() {
    let project = project();
    std::fs::write(
        project.root.join("fp-lib-table"),
        "(fp_lib_table\n  (version 7)\n  (lib (name \"imported_parts\") (type \"KiCad\") (uri \"/somewhere/else.pretty\") (options \"\") (descr \"\"))\n)\n",
    )
    .unwrap();
    let body = preview(&ldo_args(&project)).await;
    assert_eq!(body["can_apply"], false);
    assert_eq!(body["registrations"]["footprint"]["state"], "blocked");
    assert!(body["blocked_by"][0]
        .as_str()
        .unwrap()
        .starts_with("nickname_taken:"));

    // Another nickname goes through.
    let mut args = ldo_args(&project);
    args["library"] = json!("my_parts");
    let body = preview(&args).await;
    assert_eq!(body["can_apply"], true, "{body:#}");
    assert_eq!(body["symbol"]["lib_id"], "my_parts:TEST_LDO_3V3");
}

#[tokio::test]
async fn a_pin_pad_mismatch_is_reported_not_blocking() {
    let project = project();
    let scratch = tempfile::tempdir().unwrap();
    let footprint = scratch.path().join("SOT-23_EP.kicad_mod");
    std::fs::write(
        &footprint,
        std::fs::read_to_string(fixture("part_import/SOT-23_TEST.kicad_mod"))
            .unwrap()
            .replace("(footprint \"SOT-23_TEST\"", "(footprint \"SOT-23_EP\"")
            .replace(
                "  (pad \"3\"",
                "  (pad \"EP\" smd rect (at 0 0) (size 1 1) (layers \"F.Cu\" \"F.Mask\"))\n  (pad \"4\"",
            ),
    )
    .unwrap();
    let mut args = ldo_args(&project);
    args["footprint_file"] = json!(footprint.display().to_string());
    let body = preview(&args).await;
    assert_eq!(body["can_apply"], true);
    assert_eq!(body["pin_pad_check"]["result"], "fail");
    assert_eq!(body["pin_pad_check"]["missing_pads"], json!(["3"]));
    assert_eq!(body["pin_pad_check"]["extra_pads"], json!(["4", "EP"]));
    assert!(body["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap().starts_with("Pin-pad check: fail")));
    // The kept vendor model reference does not resolve in this project.
    assert_eq!(
        body["footprint"]["model"]["kept_references"][0]["state"],
        "missing"
    );

    let saved = ok(apply(&args, body["revision"].as_str().unwrap()).await);
    assert_eq!(saved["pin_pad_check"]["result"], "fail");
}

#[tokio::test]
async fn unusable_inputs_are_refused_before_anything_is_planned() {
    let project = project();
    let scratch = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| {
        let path = scratch.path().join(name);
        std::fs::write(&path, text).unwrap();
        path.display().to_string()
    };
    let two = write(
        "two.kicad_sym",
        "(kicad_symbol_lib (version 20241209) (generator \"t\")\n  (symbol \"A\" (property \"Reference\" \"U\" (at 0 0 0) (effects (font (size 1.27 1.27)))))\n  (symbol \"B\" (extends \"A\") (property \"Reference\" \"U\" (at 0 0 0) (effects (font (size 1.27 1.27)))))\n)\n",
    );
    let module = write(
        "old.kicad_mod",
        "(module \"OLD\" (layer F.Cu) (pad 1 smd rect (at 0 0) (size 1 1) (layers F.Cu)))\n",
    );
    let traversal = write(
        "bad.kicad_mod",
        "(footprint \"parts/../../escape\" (layer \"F.Cu\") (pad \"1\" smd rect (at 0 0) (size 1 1) (layers \"F.Cu\")))\n",
    );
    let not_step = write("model.step", "solid not_a_step\n");
    let base = ldo_args(&project);
    let with = |key: &str, value: Value| {
        let mut args = base.clone();
        args[key] = value;
        args
    };
    let mut cases = vec![
        (
            with("symbol_file", json!(two.clone())),
            "invalid_argument",
            "symbol_name",
        ),
        (
            {
                let mut a = with("symbol_file", json!(two.clone()));
                a["symbol_name"] = json!("B");
                a
            },
            "invalid_argument",
            "derived",
        ),
        (
            {
                let mut a = with("symbol_file", json!(two));
                a["symbol_name"] = json!("C");
                a
            },
            "invalid_argument",
            "it holds: A, B",
        ),
        (
            with("footprint_file", json!(module)),
            "invalid_argument",
            "fp upgrade",
        ),
        (
            with("footprint_file", json!(traversal)),
            "invalid_argument",
            "must not contain",
        ),
        (
            with("model_file", json!(not_step)),
            "invalid_argument",
            "ISO-10303-21",
        ),
        (
            with(
                "model_file",
                json!(fixture("part_import/README.md").display().to_string()),
            ),
            "invalid_argument",
            ".step, .stp or .wrl",
        ),
        (
            with("lcsc", json!("25804")),
            "invalid_argument",
            "LCSC part number",
        ),
        (
            with("library", json!("../up")),
            "invalid_argument",
            "1-64 characters",
        ),
        (
            with("project", json!(scratch.path().display().to_string())),
            "invalid_argument",
            "no .kicad_pro",
        ),
        (
            with(
                "symbol_file",
                json!(scratch
                    .path()
                    .join("missing.kicad_sym")
                    .display()
                    .to_string()),
            ),
            "file_not_found",
            "missing.kicad_sym",
        ),
        (with("dry_run", json!("no")), "invalid_argument", "boolean"),
    ];
    for (args, kind, needle) in cases.drain(..) {
        let result = call(args.clone()).await;
        assert_eq!(err_kind(&result), kind, "{args}");
        assert!(
            text(&result).contains(needle),
            "{needle}: {}",
            text(&result)
        );
    }
    assert_eq!(
        snapshot(&project.root).keys().collect::<Vec<_>>(),
        ["board.kicad_pro"],
        "refused inputs write nothing"
    );
}

#[test]
fn the_tool_is_registered_in_the_integration_toolset_with_a_closed_schema() {
    // With `render_library_item`, `library` reaches the 20-tool cap, so the
    // importer lives in `integration`.
    assert!(!super::super::tools()
        .iter()
        .any(|t| t.name == "import_library_part"));
    let tools = crate::tools::integration::tools();
    let tool = tools
        .iter()
        .find(|t| t.name == "import_library_part")
        .expect("registered");
    assert_eq!(tool.input_schema["additionalProperties"], false);
    assert_eq!(
        tool.input_schema["required"],
        json!(["project", "symbol_file", "footprint_file"])
    );
}

#[test]
fn model_replacement_inserts_one_reference_when_the_footprint_has_none() {
    let source = "(footprint \"X\"\n  (layer \"F.Cu\")\n  (pad \"1\" smd rect (at 0 0) (size 1 1) (layers \"F.Cu\"))\n)\n";
    let root = parse_sexp(source).unwrap();
    let replaced = replace_models(source, &root, "${KIPRJMOD}/lib.3dshapes/x.step").unwrap();
    let parsed = parse_sexp(&replaced).unwrap();
    let model = parsed.find("model").unwrap();
    assert_eq!(
        model.get(1).and_then(SexpNode::as_str),
        Some("${KIPRJMOD}/lib.3dshapes/x.step")
    );
    assert_eq!(xyz_of(model, "scale", [0.0; 3]), [1.0, 1.0, 1.0]);

    let two =
        "(footprint \"X\"\n  (model \"a.step\" (offset (xyz 1 2 3)))\n  (model \"b.wrl\")\n)\n";
    let root = parse_sexp(two).unwrap();
    let replaced = replace_models(two, &root, "new.step").unwrap();
    let parsed = parse_sexp(&replaced).unwrap();
    assert_eq!(parsed.find_all("model").len(), 1);
    assert_eq!(
        xyz_of(parsed.find("model").unwrap(), "offset", [0.0; 3]),
        [1.0, 2.0, 3.0]
    );
}

#[test]
fn model_replacement_keeps_visibility_and_opacity() {
    for (model, visible, opacity) in [
        (
            "(model \"a.step\" (hide yes) (opacity 0.4) (offset (xyz 1 2 3)))",
            false,
            0.4,
        ),
        ("(model \"a.step\" hide (offset (xyz 1 2 3)))", false, 1.0),
        ("(model \"a.step\" (hide true))", false, 1.0),
        ("(model \"a.step\" (hide no) (opacity 0.25))", true, 0.25),
        ("(model \"a.step\" (offset (xyz 1 2 3)))", true, 1.0),
    ] {
        let source = format!("(footprint \"X\"\n  {model}\n)\n");
        let root = parse_sexp(&source).unwrap();
        let replaced = replace_models(&source, &root, "new.step").unwrap();
        let models =
            crate::tools::library_footprint::models(&parse_sexp(&replaced).unwrap()).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].filename, "new.step");
        assert_eq!(models[0].visible, visible, "{model}");
        assert_eq!(models[0].opacity, opacity, "{model}");
    }
}

/// The fixture footprint with its model embedded the way KiCad 9+ saves it,
/// plus `extra` more embedded files.
fn embedded_footprint(dir: &Path, reference: &str, extra: &str) -> String {
    let path = dir.join("SOT-23_TEST.kicad_mod");
    let source = std::fs::read_to_string(fixture("part_import/SOT-23_TEST.kicad_mod")).unwrap();
    let body = source
        .replace("${KIPRJMOD}/vendor_models/SOT-23-3.step", reference)
        .trim_end()
        .strip_suffix(')')
        .unwrap()
        .to_string();
    std::fs::write(
        &path,
        format!(
            "{body}  (embedded_files\n    (file\n      (name \"SOT-23-3.step\")\n      (type model)\n      (data |KLUv/QBYAQAA|)\n      (checksum \"00\")\n    ){extra}\n  )\n)\n"
        ),
    )
    .unwrap();
    path.display().to_string()
}

#[tokio::test]
async fn an_embedded_model_is_reported_as_embedded_and_dropped_when_replaced() {
    let project = project();
    let scratch = tempfile::tempdir().unwrap();
    let mut args = ldo_args(&project);
    args["footprint_file"] = json!(embedded_footprint(
        scratch.path(),
        "kicad-embed://SOT-23-3.step",
        ""
    ));
    let body = preview(&args).await;
    assert_eq!(
        body["footprint"]["model"]["kept_references"][0]["state"],
        "embedded"
    );
    assert!(
        !body["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("3D model reference")),
        "{body:#}"
    );

    // A reference to an embedded file that is not there is still reported.
    let absent = tempfile::tempdir().unwrap();
    args["footprint_file"] = json!(embedded_footprint(
        absent.path(),
        "kicad-embed://other.step",
        ""
    ));
    let body = preview(&args).await;
    assert_eq!(
        body["footprint"]["model"]["kept_references"][0]["state"],
        "embedded_file_absent"
    );
    assert!(body["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap().contains("embedded_file_absent")));

    // With model_file, the embedded model goes with its reference; another
    // embedded file stays.
    let datasheet = "\n    (file\n      (name \"ds.pdf\")\n      (type datasheet)\n      (data |AAAA|)\n      (checksum \"01\")\n    )";
    let with_datasheet = tempfile::tempdir().unwrap();
    args["footprint_file"] = json!(embedded_footprint(
        with_datasheet.path(),
        "kicad-embed://SOT-23-3.step",
        datasheet
    ));
    args["model_file"] = json!(fixture("part_import/SOT-23_TEST.step")
        .display()
        .to_string());
    let body = preview(&args).await;
    assert_eq!(
        body["footprint"]["model"]["embedded_models_removed"],
        json!(["SOT-23-3.step"])
    );
    ok(apply(&args, body["revision"].as_str().unwrap()).await);
    let saved = std::fs::read_to_string(
        project
            .root
            .join("imported_parts.pretty/SOT-23_TEST.kicad_mod"),
    )
    .unwrap();
    let saved = parse_sexp(&saved).unwrap();
    assert!(embedded_model_names(&saved).is_empty());
    let files = saved.find("embedded_files").unwrap().find_all("file");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].find_str("name"), Some("ds.pdf"));
}

#[test]
fn an_embedded_files_block_left_empty_is_removed() {
    let source = "(footprint \"X\"\n  (model \"kicad-embed://a.step\" (offset (xyz 0 0 0)))\n  (embedded_files\n    (file (name \"a.step\") (type model) (data |AAAA|) (checksum \"0\"))\n  )\n)\n";
    let root = parse_sexp(source).unwrap();
    let replaced = replace_models(source, &root, "new.step").unwrap();
    assert!(!replaced.contains("embedded_files"), "{replaced}");
    assert!(!replaced.contains("kicad-embed://"), "{replaced}");
    assert_eq!(parse_sexp(&replaced).unwrap().find_all("model").len(), 1);
}

#[tokio::test]
async fn a_step_model_with_a_byte_order_mark_is_copied_verbatim() {
    let project = project();
    let scratch = tempfile::tempdir().unwrap();
    let model = scratch.path().join("BOM.step");
    std::fs::write(
        &model,
        "\u{feff}ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n",
    )
    .unwrap();
    let mut args = ldo_args(&project);
    args["model_file"] = json!(model.display().to_string());
    let body = preview(&args).await;
    assert_eq!(body["can_apply"], true, "{body:#}");
    assert_eq!(body["footprint"]["model"]["action"], "create");
    ok(apply(&args, body["revision"].as_str().unwrap()).await);
    let saved = std::fs::read(project.root.join("imported_parts.3dshapes/BOM.step")).unwrap();
    assert_eq!(saved, std::fs::read(&model).unwrap());
    assert!(saved.starts_with(&[0xef, 0xbb, 0xbf]));
    // The reuse comparison is over the same verbatim bytes.
    let body = preview(&args).await;
    assert_eq!(body["footprint"]["model"]["action"], "reuse_identical");
}

#[tokio::test]
async fn a_non_utf8_step_model_is_refused_with_a_re_export_hint() {
    let project = project();
    let scratch = tempfile::tempdir().unwrap();
    let model = scratch.path().join("LATIN1.step");
    std::fs::write(
        &model,
        b"ISO-10303-21;\nHEADER;\nFILE_NAME('M\xfcller');\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n",
    )
    .unwrap();
    let mut args = ldo_args(&project);
    args["model_file"] = json!(model.display().to_string());
    let result = call(args).await;
    assert_eq!(err_kind(&result), "invalid_argument");
    assert!(
        text(&result).contains("re-export it as an ISO 10303-21 conforming (ASCII) STEP"),
        "{}",
        text(&result)
    );
    assert_eq!(
        snapshot(&project.root).keys().collect::<Vec<_>>(),
        ["board.kicad_pro"]
    );
}

#[tokio::test]
async fn a_library_registered_under_another_nickname_blocks() {
    let project = project();
    let library = project.root.join("imported_parts.kicad_sym");
    std::fs::write(
        &library,
        "(kicad_symbol_lib\n  (version 20241209)\n  (generator \"t\")\n)\n",
    )
    .unwrap();
    // The same file by the tool's own URI text, then by an absolute path.
    for uri in [
        "${KIPRJMOD}/imported_parts.kicad_sym".to_string(),
        library.display().to_string(),
    ] {
        std::fs::write(
            project.root.join("sym-lib-table"),
            format!("(sym_lib_table\n  (version 7)\n  (lib (name \"MyParts\") (type \"KiCad\") (uri \"{uri}\") (options \"\") (descr \"\"))\n)\n"),
        )
        .unwrap();
        let body = preview(&ldo_args(&project)).await;
        assert_eq!(body["can_apply"], false, "{uri}");
        assert_eq!(body["registrations"]["symbol"]["state"], "blocked");
        assert!(
            body["blocked_by"].as_array().unwrap().iter().any(|b| {
                let b = b.as_str().unwrap();
                b.starts_with("library_already_registered:") && b.contains("'MyParts'")
            }),
            "{body:#}"
        );
    }
    // Before the file exists, a row that spells the path another way is
    // still the same library.
    std::fs::remove_file(&library).unwrap();
    std::fs::write(
        project.root.join("sym-lib-table"),
        "(sym_lib_table\n  (version 7)\n  (lib (name \"MyParts\") (type \"KiCad\") (uri \"${KIPRJMOD}/./sub/../imported_parts.kicad_sym\") (options \"\") (descr \"\"))\n)\n",
    )
    .unwrap();
    let body = preview(&ldo_args(&project)).await;
    assert_eq!(body["can_apply"], false, "{body:#}");
    assert!(
        body["blocked_by"].as_array().unwrap().iter().any(|b| b
            .as_str()
            .unwrap()
            .starts_with("library_already_registered:")),
        "{body:#}"
    );
}

#[tokio::test]
async fn a_library_spelled_in_another_case_blocks() {
    let project = project();
    std::fs::create_dir(project.root.join("imported_parts.pretty")).unwrap();
    let mut args = ldo_args(&project);
    args["library"] = json!("Imported_Parts");
    let body = preview(&args).await;
    assert_eq!(body["can_apply"], false, "{body:#}");
    assert!(
        body["blocked_by"].as_array().unwrap().iter().any(|b| {
            let b = b.as_str().unwrap();
            b.starts_with("target_case_mismatch:") && b.contains("'imported_parts.pretty'")
        }),
        "{body:#}"
    );
}

/// Device.kicad_sym's R with KiCad 10's C_0603: a second, unrelated part.
fn other_part(project: &Project) -> Value {
    json!({
        "project": project.pro(),
        "symbol_file": fixture("reference_prefix_kicad10/Device.kicad_sym").display().to_string(),
        "footprint_file": fixture("c_0603_1608metric_kicad10.kicad_mod").display().to_string(),
    })
}

#[tokio::test]
async fn a_disabled_registration_blocks_and_a_hidden_one_warns() {
    let project = project();
    let args = ldo_args(&project);
    let revision = preview(&args).await["revision"]
        .as_str()
        .unwrap()
        .to_string();
    ok(apply(&args, &revision).await);
    let table = project.root.join("sym-lib-table");
    let registered = std::fs::read_to_string(&table).unwrap();
    assert_eq!(registered.matches("(descr \"\"))").count(), 1);

    std::fs::write(
        &table,
        registered.replace("(descr \"\"))", "(descr \"\") (disabled))"),
    )
    .unwrap();
    let body = preview(&other_part(&project)).await;
    assert_eq!(body["can_apply"], false);
    assert_eq!(body["registrations"]["symbol"]["state"], "blocked");
    assert!(
        body["blocked_by"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b.as_str().unwrap().starts_with("library_disabled:")),
        "{body:#}"
    );

    std::fs::write(
        &table,
        registered.replace("(descr \"\"))", "(descr \"\") (hidden))"),
    )
    .unwrap();
    let body = preview(&other_part(&project)).await;
    assert_eq!(body["can_apply"], true, "{body:#}");
    assert_eq!(body["registrations"]["symbol"]["state"], "unchanged");
    assert!(body["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap().contains("hidden")));
}

#[tokio::test]
async fn a_registration_under_an_equivalent_absolute_uri_is_reused() {
    let project = project();
    let args = ldo_args(&project);
    let revision = preview(&args).await["revision"]
        .as_str()
        .unwrap()
        .to_string();
    ok(apply(&args, &revision).await);
    for (table, file) in [
        ("sym-lib-table", "imported_parts.kicad_sym"),
        ("fp-lib-table", "imported_parts.pretty"),
    ] {
        let path = project.root.join(table);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            text.replace(
                &format!("${{KIPRJMOD}}/{file}"),
                &project.root.join(file).display().to_string(),
            ),
        )
        .unwrap();
    }
    let tables_before = (
        std::fs::read(project.root.join("sym-lib-table")).unwrap(),
        std::fs::read(project.root.join("fp-lib-table")).unwrap(),
    );
    let second = other_part(&project);
    let body = preview(&second).await;
    assert_eq!(body["can_apply"], true, "{body:#}");
    assert_eq!(body["registrations"]["symbol"]["state"], "unchanged");
    assert_eq!(body["registrations"]["footprint"]["state"], "unchanged");
    ok(apply(&second, body["revision"].as_str().unwrap()).await);
    assert_eq!(
        (
            std::fs::read(project.root.join("sym-lib-table")).unwrap(),
            std::fs::read(project.root.join("fp-lib-table")).unwrap(),
        ),
        tables_before
    );
}

/// Previews `args`, expects a blocker starting with `prefix` and no write
/// planned onto `target`, and checks that applying the blocked revision writes
/// nothing.
async fn assert_blocked(project: &Project, args: &Value, prefix: &str, target: &str) {
    let before = snapshot(&project.root);
    let body = preview(args).await;
    assert_eq!(body["can_apply"], false, "{prefix}: {body:#}");
    assert!(
        body["blocked_by"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b.as_str().unwrap().starts_with(prefix)),
        "{prefix}: {body:#}"
    );
    let target = project.root.join(target).display().to_string();
    assert!(
        !body["writes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["path"].as_str().unwrap().starts_with(&target)),
        "{prefix}: a write is planned onto the blocked {target}: {body:#}"
    );
    let refused = apply(args, body["revision"].as_str().unwrap()).await;
    assert_eq!(err_kind(&refused), "plan_blocked");
    assert!(text(&refused).contains("nothing written"));
    assert_eq!(snapshot(&project.root), before, "{prefix}");
}

#[tokio::test]
async fn targets_of_the_wrong_kind_or_unusable_block() {
    let case = project();
    std::fs::write(case.root.join("imported_parts.pretty"), "not a folder\n").unwrap();
    assert_blocked(
        &case,
        &ldo_args(&case),
        "target_has_wrong_type:",
        "imported_parts.pretty",
    )
    .await;

    let case = project();
    std::fs::create_dir(case.root.join("imported_parts.kicad_sym")).unwrap();
    assert_blocked(
        &case,
        &ldo_args(&case),
        "target_has_wrong_type:",
        "imported_parts.kicad_sym",
    )
    .await;

    let case = project();
    std::fs::write(
        case.root.join("imported_parts.kicad_sym"),
        "(kicad_symbol_lib (version\n",
    )
    .unwrap();
    assert_blocked(
        &case,
        &ldo_args(&case),
        "symbol_library_unusable:",
        "imported_parts.kicad_sym",
    )
    .await;

    let case = project();
    std::fs::write(
        case.root.join("sym-lib-table"),
        "(sym_lib_table\n  (version 7)\n  (lib (name \"x\")\n",
    )
    .unwrap();
    assert_blocked(
        &case,
        &ldo_args(&case),
        "lib_table_unusable:",
        "sym-lib-table",
    )
    .await;

    let case = project();
    let row = "  (lib (name \"imported_parts\") (type \"KiCad\") (uri \"${KIPRJMOD}/imported_parts.pretty\") (options \"\") (descr \"\"))\n";
    std::fs::write(
        case.root.join("fp-lib-table"),
        format!("(fp_lib_table\n  (version 7)\n{row}{row})\n"),
    )
    .unwrap();
    assert_blocked(
        &case,
        &ldo_args(&case),
        "lib_table_unusable:",
        "fp-lib-table",
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_targets_and_tables_block_in_the_preview() {
    let outside = tempfile::tempdir().unwrap();

    let case = project();
    std::os::unix::fs::symlink(outside.path(), case.root.join("imported_parts.pretty")).unwrap();
    assert_blocked(
        &case,
        &ldo_args(&case),
        "target_is_a_symlink:",
        "imported_parts.pretty",
    )
    .await;

    // A dangling link where the footprint file would go.
    let case = project();
    std::fs::create_dir(case.root.join("imported_parts.pretty")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("gone.kicad_mod"),
        case.root
            .join("imported_parts.pretty/SOT-23_TEST.kicad_mod"),
    )
    .unwrap();
    assert_blocked(
        &case,
        &ldo_args(&case),
        "target_is_a_symlink:",
        "imported_parts.pretty/SOT-23_TEST.kicad_mod",
    )
    .await;

    // A library table linked to a file inside the project.
    let case = project();
    std::fs::create_dir(case.root.join("tables")).unwrap();
    std::fs::write(
        case.root.join("tables/sym-lib-table"),
        "(sym_lib_table\n  (version 7)\n)\n",
    )
    .unwrap();
    std::os::unix::fs::symlink("tables/sym-lib-table", case.root.join("sym-lib-table")).unwrap();
    let body = preview(&ldo_args(&case)).await;
    assert_eq!(body["registrations"]["symbol"]["state"], "blocked");
    assert_blocked(
        &case,
        &ldo_args(&case),
        "target_is_a_symlink:",
        "sym-lib-table",
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_commit_refused_before_writing_says_nothing_was_written() {
    use std::os::unix::fs::PermissionsExt;
    let project = project();
    // The footprint folder exists, so apply needs to create no folder and
    // the transaction itself is what fails.
    std::fs::create_dir(project.root.join("imported_parts.pretty")).unwrap();
    let args = ldo_args(&project);
    let body = preview(&args).await;
    assert_eq!(body["can_apply"], true, "{body:#}");
    let before = snapshot(&project.root);

    let set_mode = |mode| {
        std::fs::set_permissions(&project.root, std::fs::Permissions::from_mode(mode)).unwrap()
    };
    set_mode(0o555);
    let probe = project.root.join("probe");
    if std::fs::write(&probe, "").is_ok() {
        // Running as root: permissions do not refuse anything here.
        let _ = std::fs::remove_file(&probe);
        set_mode(0o755);
        return;
    }
    let refused = apply(&args, body["revision"].as_str().unwrap()).await;
    set_mode(0o755);

    assert_eq!(err_kind(&refused), "plan_blocked", "{}", text(&refused));
    let message = text(&refused);
    assert!(message.contains("transaction_refused:"), "{message}");
    assert!(message.contains("nothing written"), "{message}");
    assert!(!message.contains("may be written"), "{message}");
    assert_eq!(snapshot(&project.root), before);

    // Without the folder, creating it is what fails: refused the same way.
    std::fs::remove_dir(project.root.join("imported_parts.pretty")).unwrap();
    let body = preview(&args).await;
    let before = snapshot(&project.root);
    set_mode(0o555);
    let refused = apply(&args, body["revision"].as_str().unwrap()).await;
    set_mode(0o755);
    assert_eq!(err_kind(&refused), "plan_blocked", "{}", text(&refused));
    assert!(text(&refused).contains("cannot_create_folder:"));
    assert_eq!(snapshot(&project.root), before);
}

#[test]
fn only_an_active_journal_counts_as_pending() {
    let project = project();
    let id = "00000000-0000-4000-8000-000000000000";
    std::fs::write(
        project
            .root
            .join(format!(".konnect-transaction-{id}.abandoned.json")),
        "{}",
    )
    .unwrap();
    assert!(
        !journal_pending(&project.root),
        "abandoned evidence is not pending"
    );
    assert!(
        journal_pending(&project.root.join("missing")),
        "a folder that cannot be inspected counts as pending"
    );
    std::fs::write(
        project.root.join(format!(".konnect-transaction-{id}.json")),
        "{}",
    )
    .unwrap();
    assert!(
        journal_pending(&project.root),
        "an active journal is pending"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_project_path_relative_to_the_working_folder_applies() {
    let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
    let depth = cwd.components().count();
    let dir = tempfile::tempdir().unwrap();
    // Nested deeper than the working folder, so a relative target joined onto
    // the project folder a second time cannot climb back to the same place.
    let mut root = dir.path().canonicalize().unwrap();
    for _ in 0..depth {
        root.push("d");
    }
    root.push("board");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("board.kicad_pro"), "{}\n").unwrap();
    let mut relative = PathBuf::new();
    for _ in 1..depth {
        relative.push("..");
    }
    let relative = relative
        .join(root.strip_prefix("/").unwrap())
        .join("board.kicad_pro");
    assert!(relative.is_relative());
    let project = Project { _dir: dir, root };

    let mut args = ldo_args(&project);
    args["project"] = json!(relative.display().to_string());
    let body = preview(&args).await;
    assert_eq!(body["can_apply"], true, "{body:#}");
    assert_eq!(
        body["library"]["symbol_library"],
        project
            .root
            .join("imported_parts.kicad_sym")
            .display()
            .to_string()
    );
    let saved = ok(apply(&args, body["revision"].as_str().unwrap()).await);
    assert_eq!(saved["pin_pad_check"]["result"], "pass");
    assert!(project.root.join("imported_parts.kicad_sym").is_file());
    assert!(project
        .root
        .join("imported_parts.pretty/SOT-23_TEST.kicad_mod")
        .is_file());
    assert_eq!(
        parse_lib_table(&std::fs::read_to_string(project.root.join("sym-lib-table")).unwrap())[0]
            ["uri"],
        "${KIPRJMOD}/imported_parts.kicad_sym"
    );
}
