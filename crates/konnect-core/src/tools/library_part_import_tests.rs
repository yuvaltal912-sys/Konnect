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

/// Every file under the project, with its bytes.
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
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
    assert!(
        blockers.iter().any(|b| b.starts_with("symbol_exists:")),
        "{blockers:?}"
    );
    assert!(
        blockers
            .iter()
            .any(|b| b.starts_with("footprint_exists:")
                && b.contains("already holds this footprint")),
        "{blockers:?}"
    );
    // The same model bytes are reused, not a conflict.
    assert_eq!(body["footprint"]["model"]["action"], "reuse_identical");
    let refused = apply(&args, body["revision"].as_str().unwrap()).await;
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
fn the_tool_is_registered_in_the_library_toolset_with_a_closed_schema() {
    let tools = super::super::tools();
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
