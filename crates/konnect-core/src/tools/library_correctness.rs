//! Metadata-only library edits and identifier compatibility. Geometry is not rewritten.
use super::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn tools() -> Vec<ToolDef> {
    vec![
 tool!("edit_symbol","Partially edit a library symbol's Value, Reference, Footprint, Description, Datasheet or keywords. Preserves geometry and field placement; verifies saved metadata. Does not rename symbols or refresh placed copies.",json!({"type":"object","properties":{"library_path":{"type":"string"},"symbol_name":{"type":"string"},"value":{"type":"string"},"reference_prefix":{"type":"string"},"footprint":{"type":"string"},"description":{"type":"string"},"datasheet":{"type":"string"},"keywords":{"type":"string"}},"required":["library_path","symbol_name"],"additionalProperties":false}),|args,ctx|async move{handle_edit_symbol(args,ctx).await}),
 tool!("check_symbol_footprint","Read-only comparison of effective symbol pin numbers with footprint pad numbers, including inherited pins and repeated pads. Reports pass/fail/incomplete; not physical pin-map or datasheet proof.",json!({"type":"object","properties":{"library_path":{"type":"string"},"symbol_name":{"type":"string"},"footprint_path":{"type":"string"}},"required":["library_path","symbol_name","footprint_path"]}),|args,ctx|async move{handle_check_symbol_footprint(args,ctx).await})]
}

pub(super) fn library(source: &str) -> anyhow::Result<SexpNode> {
    let (mut quoted, mut escaped, mut line_start) = (false, false, true);
    for ch in source.chars() {
        anyhow::ensure!(
            quoted || !line_start || ch != '#',
            "unsupported library comment line"
        );
        if escaped {
            escaped = false;
        } else if ch == '\\' && quoted {
            escaped = true;
        } else if ch == '"' {
            quoted = !quoted;
        }
        if ch == '\n' {
            line_start = true;
        } else if !ch.is_whitespace() {
            line_start = false;
        }
    }

    let root = parse_sexp(source)?;
    anyhow::ensure!(
        source.trim_start().starts_with("(kicad_symbol_lib")
            && root.head() == Some("kicad_symbol_lib"),
        "expected one explicit kicad_symbol_lib root"
    );
    let mut names = BTreeSet::new();
    for n in root.find_all("symbol") {
        let name = n
            .get(1)
            .and_then(SexpNode::as_str)
            .ok_or_else(|| anyhow::anyhow!("unnamed symbol"))?;
        anyhow::ensure!(!name.is_empty(), "empty symbol name");
        anyhow::ensure!(names.insert(name), "duplicate symbol name: {name}");
    }
    Ok(root)
}
fn symbol<'a>(root: &'a SexpNode, name: &str) -> anyhow::Result<&'a SexpNode> {
    root.find_all("symbol")
        .into_iter()
        .find(|n| n.get(1).and_then(SexpNode::as_str) == Some(name))
        .ok_or_else(|| anyhow::anyhow!("symbol not found: {name}"))
}
fn chain<'a>(root: &'a SexpNode, name: &str) -> anyhow::Result<Vec<&'a SexpNode>> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut current = name;
    loop {
        anyhow::ensure!(seen.insert(current), "cyclic symbol inheritance");
        let node = symbol(root, current)?;
        out.push(node);
        match node.find_str("extends") {
            Some(base) => current = base,
            None => break,
        }
    }
    Ok(out)
}
fn properties(node: &SexpNode) -> anyhow::Result<BTreeMap<String, String>> {
    let mut fields = BTreeMap::new();
    for p in node.find_all("property") {
        let key = p
            .get(1)
            .and_then(SexpNode::as_str)
            .ok_or_else(|| anyhow::anyhow!("invalid property name"))?;
        let value = p
            .get(2)
            .and_then(SexpNode::as_str)
            .ok_or_else(|| anyhow::anyhow!("invalid property value"))?;
        anyhow::ensure!(
            fields.insert(key.into(), value.into()).is_none(),
            "duplicate property: {key}"
        );
    }
    Ok(fields)
}
pub(super) fn metadata(args: &Value) -> Result<BTreeMap<String, String>, CallToolResult> {
    let mut result = BTreeMap::new();
    for (arg, key) in [
        ("value", "Value"),
        ("reference_prefix", "Reference"),
        ("footprint", "Footprint"),
        ("description", "Description"),
        ("datasheet", "Datasheet"),
        ("keywords", "ki_keywords"),
    ] {
        if let Some(value) = args.get(arg) {
            let Some(value) = value.as_str() else {
                return Err(invalid_library_argument(arg, "must be a string"));
            };
            if value.contains('\0') {
                return Err(invalid_library_argument(arg, "must not contain NUL"));
            }
            if arg == "reference_prefix" && value.is_empty() {
                return Err(invalid_library_argument(arg, "must be nonempty"));
            }
            result.insert(
                key.into(),
                if key == "Datasheet" && value == "~" {
                    String::new()
                } else {
                    value.into()
                },
            );
        }
    }
    Ok(result)
}
fn span(source: &str, name: &str) -> anyhow::Result<(usize, usize)> {
    library(source)?;
    find_direct_child_blocks(source, "kicad_symbol_lib")
        .into_iter()
        .find(|(a, b)| {
            parse_sexp(&source[*a..*b]).is_ok_and(|n| {
                n.head() == Some("symbol") && n.get(1).and_then(SexpNode::as_str) == Some(name)
            })
        })
        .ok_or_else(|| anyhow::anyhow!("symbol not found: {name}"))
}
fn strings(source: &str) -> Vec<(usize, usize)> {
    let b = source.as_bytes();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' {
            let start = i;
            i += 1;
            while i < b.len() {
                if b[i] == b'\\' {
                    i += 2;
                } else if b[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            ranges.push((start, i.min(b.len())));
        } else {
            i += 1;
        }
    }
    ranges
}
pub(super) fn prepare(
    source: &str,
    name: &str,
    fields: &BTreeMap<String, String>,
) -> anyhow::Result<String> {
    let (start, end) = span(source, name)?;
    let root = library(source)?;
    properties(symbol(&root, name)?)?;
    let body = &source[start..end];
    let mut edits = Vec::new();
    let mut pending = fields.clone();
    for (a, b) in find_direct_child_blocks(body, "symbol") {
        let node = parse_sexp(&body[a..b])?;
        if node.head() != Some("property") {
            continue;
        }
        let Some(key) = node.get(1).and_then(SexpNode::as_str) else {
            continue;
        };
        if let Some(value) = pending.remove(key) {
            let ranges = strings(&body[a..b]);
            anyhow::ensure!(ranges.len() >= 2, "property value must be quoted");
            let (x, y) = ranges[1];
            edits.push(SexpEdit::replace(
                start + a + x,
                start + a + y,
                serde_json::to_string(&value)?,
            ));
        }
    }
    if !pending.is_empty() {
        let newline = if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let nodes = pending
            .into_iter()
            .map(|(k, v)| {
                let visibility=if matches!(k.as_str(),"Value"|"Reference"){""}else{" (hide yes)"};
                format!(
                    "{newline}    (property {} {} (at 0 0 0){visibility} (effects (font (size 1.27 1.27))))",
                    serde_json::to_string(&k).unwrap(),
                    serde_json::to_string(&v).unwrap()
                )
            })
            .collect::<String>();
        edits.push(SexpEdit::insert(end - 1, nodes));
    }
    let candidate = apply_edits(source.into(), edits);
    let parsed = library(&candidate)?;
    let observed = properties(symbol(&parsed, name)?)?;
    anyhow::ensure!(
        fields.iter().all(|(k, v)| observed.get(k) == Some(v)),
        "prospective metadata differs"
    );
    Ok(candidate)
}
async fn handle_edit_symbol(args: &Value, _ctx: &ToolContext) -> anyhow::Result<CallToolResult> {
    let path = get_path(args, "library_path")?;
    let name = match require_str(args, "symbol_name") {
        Ok(v) => v,
        Err(e) => return Ok(e),
    };
    let fields = match metadata(args) {
        Ok(v) if !v.is_empty() => v,
        Ok(_) => {
            return Ok(invalid_library_argument(
                "metadata",
                "supply a metadata field",
            ))
        }
        Err(e) => return Ok(e),
    };
    let source = read_consistent(&path)?;
    let candidate = match prepare(&source, name, &fields) {
        Ok(v) => v,
        Err(e) => return Ok(CallToolResult::error(e.to_string())),
    };
    if candidate != source {
        write_atomic_if_unchanged(&path, &source, &candidate)?;
    }
    let observed = saved_metadata(&path, name, &fields);
    match observed {
        Ok(p) => Ok(CallToolResult::json(
            &json!({"symbol":name,"updated":candidate!=source,"properties":p,"source":"saved_file"}),
        )),
        Err(e) => Ok(crate::tools::mutation_outcome_uncertain(
            &path,
            "edit_symbol",
            e.to_string(),
        )),
    }
}
pub(super) fn delete(source: &str, name: &str) -> anyhow::Result<String> {
    let (a, b) = span(source, name)?;
    let root = library(source)?;
    anyhow::ensure!(
        !root
            .find_all("symbol")
            .iter()
            .any(|s| s.find_str("extends") == Some(name)),
        "symbol is inherited by another definition"
    );
    let result = apply_edits(source.into(), vec![SexpEdit::delete(a, b)]);
    library(&result)?;
    Ok(result)
}
async fn handle_check_symbol_footprint(
    args: &Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let path = get_path(args, "library_path")?;
    let fp = get_path(args, "footprint_path")?;
    let name = match require_str(args, "symbol_name") {
        Ok(v) => v,
        Err(e) => return Ok(e),
    };
    let root = library(&read_consistent(&path)?)?;
    if let Err(e) = chain(&root, name) {
        return Ok(CallToolResult::json(
            &json!({"result":"incomplete","passed":false,"reason":e.to_string()}),
        ));
    }
    let footprint = parse_sexp(&read_consistent(&fp)?)?;
    anyhow::ensure!(
        matches!(footprint.head(), Some("footprint" | "module")),
        "not a footprint"
    );
    Ok(CallToolResult::json(&pin_pad_check(
        &root, name, &footprint,
    )))
}

/// The identifier-set comparison behind `check_symbol_footprint`, over parsed
/// documents so a caller can check a candidate before it is written.
pub(super) fn pin_pad_check(root: &SexpNode, name: &str, footprint: &SexpNode) -> Value {
    let nodes = match chain(root, name) {
        Ok(v) => v,
        Err(e) => return json!({"result":"incomplete","passed":false,"reason":e.to_string()}),
    };
    let mut pins = BTreeSet::new();
    let mut unreadable_pins = 0;
    for node in nodes {
        for p in descendants_with_head(node, "pin") {
            if let Some(n) = p.find_str("number") {
                if !n.is_empty() {
                    pins.insert(n.to_string());
                } else {
                    unreadable_pins += 1;
                }
            } else {
                unreadable_pins += 1;
            }
        }
    }
    let pad_nodes = footprint.find_all("pad");
    let unreadable_pads = pad_nodes
        .iter()
        .filter(|p| p.get(1).and_then(SexpNode::as_str).is_none())
        .count();
    let pads: BTreeSet<String> = pad_nodes
        .into_iter()
        .filter_map(|p| p.get(1).and_then(SexpNode::as_str))
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let missing: Vec<_> = pins.difference(&pads).cloned().collect();
    let extra: Vec<_> = pads.difference(&pins).cloned().collect();
    let result = if pins.is_empty() || pads.is_empty() || unreadable_pins > 0 || unreadable_pads > 0
    {
        "incomplete"
    } else if missing.is_empty() && extra.is_empty() {
        "pass"
    } else {
        "fail"
    };
    json!({"result":result,"passed":result=="pass","pins":pins,"pads":pads,"missing_pads":missing,"extra_pads":extra,"scope":"identifier sets only; not datasheet or physical pin-map proof","source":"saved_file"})
}
fn rank(name: &str, text: &str, query: &str) -> Option<u32> {
    let n = name.to_lowercase();
    let t = text.to_lowercase();
    if !query
        .split_whitespace()
        .all(|q| n.contains(q) || t.contains(q))
    {
        return None;
    }
    Some(if n == query {
        100
    } else if n.starts_with(query) {
        80
    } else if n.contains(query) {
        60
    } else {
        20
    })
}
pub(super) fn symbol_hits(nick: &str, source: &str, query: &str) -> anyhow::Result<Vec<Value>> {
    let root = library(source)?;
    let mut hits = Vec::new();
    for node in root.find_all("symbol") {
        let name = node.get(1).and_then(SexpNode::as_str).unwrap();
        let mut p = BTreeMap::new();
        for parent in chain(&root, name)?.into_iter().rev() {
            p.extend(properties(parent)?);
        }
        let description = p.get("Description").cloned().unwrap_or_default();
        let keywords = p
            .get("ki_keywords")
            .cloned()
            .unwrap_or_else(|| node.find_str("ki_keywords").unwrap_or("").into());
        if let Some(score) = rank(name, &format!("{description} {keywords}"), query) {
            hits.push(json!({"library":nick,"name":name,"id":format!("{nick}:{name}"),"description":description,"keywords":keywords,"rank":score}));
        }
    }
    Ok(hits)
}
pub(super) async fn search(
    args: &Value,
    ctx: &ToolContext,
    footprints: bool,
) -> anyhow::Result<CallToolResult> {
    let query = match require_str(args, "query") {
        Ok(v) if !v.trim().is_empty() => v.trim().to_lowercase(),
        _ => return Ok(invalid_library_argument("query", "must be nonempty")),
    };
    let (offset, limit) = match crate::tools::response_budget::window(args, 50) {
        Ok(v) => v,
        Err(e) => return Ok(e),
    };
    let project = args["project_dir"]
        .as_str()
        .map(PathBuf::from)
        .or_else(|| ctx.config.project_dir.clone());
    let table_name = if footprints {
        "fp-lib-table"
    } else {
        "sym-lib-table"
    };
    let mut tables = vec![if footprints {
        global_fp_lib_table()
    } else {
        global_sym_lib_table()
    }];
    if let Some(p) = project {
        tables.push(p.join(table_name));
    }
    let mut entries = BTreeMap::new();
    for table in tables {
        for lib in read_flat_lib_table(&table) {
            if let (Some(n), Some(p)) = (lib["nickname"].as_str(), lib["path"].as_str()) {
                entries.insert(n.to_string(), PathBuf::from(p));
            }
        }
    }
    let mut hits = Vec::new();
    let mut skipped = Vec::new();
    for (nick, path) in entries {
        if footprints {
            match std::fs::read_dir(&path) {
                Err(e) => skipped.push(json!({"library":nick,"reason":e.to_string()})),
                Ok(rd) => {
                    for entry in rd {
                        let entry = match entry {
                            Ok(v) => v,
                            Err(e) => {
                                skipped.push(json!({"library":nick,"reason":e.to_string()}));
                                continue;
                            }
                        };
                        if entry.path().extension().and_then(|v| v.to_str()) != Some("kicad_mod") {
                            continue;
                        }
                        let load = (|| -> anyhow::Result<_> {
                            let source = read_consistent(&entry.path())?;
                            let root = parse_sexp(&source)?;
                            anyhow::ensure!(
                                matches!(root.head(), Some("footprint" | "module")),
                                "not a footprint"
                            );
                            Ok(root)
                        })();
                        match load {
                            Err(e) => skipped.push(json!({"library":nick,"reason":e.to_string()})),
                            Ok(root) => {
                                let name = entry
                                    .path()
                                    .file_stem()
                                    .unwrap()
                                    .to_string_lossy()
                                    .into_owned();
                                let description = root.find_str("descr").unwrap_or("");
                                let tags = root.find_str("tags").unwrap_or("");
                                if let Some(rank) =
                                    rank(&name, &format!("{description} {tags}"), &query)
                                {
                                    hits.push(json!({"library":nick,"name":name,"id":format!("{nick}:{name}"),"description":description,"tags":tags,"rank":rank}));
                                }
                            }
                        }
                    }
                }
            }
        } else {
            match read_consistent(&path)
                .map_err(anyhow::Error::from)
                .and_then(|s| symbol_hits(&nick, &s, &query))
            {
                Ok(v) => hits.extend(v),
                Err(e) => skipped.push(json!({"library":nick,"reason":e.to_string()})),
            }
        }
    }
    hits.sort_by(|a, b| {
        b["rank"]
            .as_u64()
            .cmp(&a["rank"].as_u64())
            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
    });
    let mut response =
        crate::tools::response_budget::list_response("results", &hits, offset, limit);
    response["total_matches"] = json!(hits.len());
    response["query"] = json!(query);
    response["search_complete"] = json!(skipped.is_empty());
    response["skipped_library_count"] = json!(skipped.len());
    response["skipped_libraries"] = json!(skipped.into_iter().take(20).collect::<Vec<_>>());
    Ok(CallToolResult::json(&response))
}

#[cfg(test)]
mod tests {
    use super::*;
    const REAL: &str =
        include_str!("../../tests/fixtures/reference_prefix_kicad10/Device.kicad_sym");
    fn ctx() -> ToolContext {
        ToolContext::new(
            crate::tools::ServerConfig::default(),
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        )
    }
    fn body(r: CallToolResult) -> Value {
        assert!(!r.is_error, "{r:?}");
        let crate::mcp::protocol::ToolContent::Text { text } = &r.content[0] else {
            panic!()
        };
        serde_json::from_str(text).unwrap()
    }
    #[test]
    fn metadata_partial_preserves_geometry_and_handles_escaping() {
        let fields = BTreeMap::from([
            (
                "Description".into(),
                "quoted \" (symbol fake) \\ Unicode שלום".into(),
            ),
            ("ki_keywords".into(), "search tag".into()),
        ]);
        let out = prepare(REAL, "R", &fields).unwrap();
        let old = library(REAL).unwrap();
        let new = library(&out).unwrap();
        assert_eq!(
            descendants_with_head(symbol(&old, "R").unwrap(), "pin"),
            descendants_with_head(symbol(&new, "R").unwrap(), "pin")
        );
        assert_eq!(
            properties(symbol(&new, "R").unwrap()).unwrap()["Description"],
            fields["Description"]
        );
        assert_eq!(prepare(&out, "R", &fields).unwrap(), out);
    }
    #[test]
    fn duplicate_names_and_wrong_roots_refuse() {
        assert!(library("(other)").is_err());
        assert!(library("(kicad_symbol_lib) (other)").is_err());
        assert!(library("(kicad_symbol_lib (symbol \"\"))").is_err());
        let root = library("(kicad_symbol_lib (symbol \"A\" (property \"Value\" \"x\") (property \"Value\" \"y\")))").unwrap();
        assert!(properties(symbol(&root, "A").unwrap()).is_err());
        let duplicate = REAL.replacen("\n)", "\n(symbol \"R\")\n)", 1);
        assert!(library(&duplicate).is_err());
    }
    #[test]
    fn deletion_is_structural_for_tabs_quotes_and_nested_symbols() {
        let out = delete(REAL, "R").unwrap();
        assert!(!library(&out)
            .unwrap()
            .find_all("symbol")
            .iter()
            .any(|s| s.get(1).and_then(SexpNode::as_str) == Some("R")));
        assert!(delete(REAL, "missing").is_err());
        let inherited = "(kicad_symbol_lib (symbol \"A\") (symbol \"B\" (extends \"A\")))";
        assert!(delete(inherited, "A").is_err());
    }
    #[test]
    fn inheritance_missing_and_cycle_are_not_partial_passes() {
        for s in [
            "(kicad_symbol_lib (symbol \"A\" (extends \"B\")))",
            "(kicad_symbol_lib (symbol \"A\" (extends \"A\")))",
        ] {
            assert!(chain(&library(s).unwrap(), "A").is_err());
        }
    }
    #[tokio::test]
    async fn inherited_repeated_identifiers_and_incomplete_inputs() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("pins.kicad_sym");
        let f = d.path().join("pads.kicad_mod");
        let base = "(kicad_symbol_lib (symbol \"Base\" (symbol \"Base_1_1\" (pin passive line (number \"1\")))) (symbol \"Part\" (extends \"Base\")))";
        std::fs::write(&p, base).unwrap();
        std::fs::write(
            &f,
            "(footprint \"Part\" (pad \"1\") (pad \"1\") (pad \"\"))",
        )
        .unwrap();
        let args = json!({"library_path":p,"symbol_name":"Part","footprint_path":f});
        assert_eq!(
            body(handle_check_symbol_footprint(&args, &ctx()).await.unwrap())["result"],
            "pass"
        );
        for pins in [
            base.replace("(number \"1\")", "(number \"1\")) (pin passive line"),
            base.replace("(number \"1\")", "(number \"\")"),
            base.replace("(number \"1\")", ""),
            "(kicad_symbol_lib (symbol \"Part\"))".into(),
        ] {
            std::fs::write(&p, pins).unwrap();
            assert_eq!(
                body(handle_check_symbol_footprint(&args, &ctx()).await.unwrap())["result"],
                "incomplete"
            );
        }
        std::fs::write(&p, base).unwrap();
        for pads in [
            "(footprint \"Part\" (pad \"1\") (pad))",
            "(footprint \"Part\")",
        ] {
            std::fs::write(&f, pads).unwrap();
            assert_eq!(
                body(handle_check_symbol_footprint(&args, &ctx()).await.unwrap())["result"],
                "incomplete"
            );
        }
    }
    #[test]
    fn rank_uses_all_words_and_prioritizes_names() {
        assert_eq!(rank("ADC", "precision converter", "adc"), Some(100));
        assert_eq!(rank("ADC123", "", "adc"), Some(80));
        assert_eq!(rank("X_ADC", "", "adc"), Some(60));
        assert_eq!(
            rank("Part", "low power converter", "low converter"),
            Some(20)
        );
        assert_eq!(rank("Part", "low", "low missing"), None);
        let hits = symbol_hits("Device", REAL, "resistor").unwrap();
        assert!(hits.iter().any(|h| h["name"] == "R"));
    }
    #[tokio::test]
    async fn edit_refuses_bad_types_without_writing_and_reads_saved_fields() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.kicad_sym");
        std::fs::write(&p, REAL).unwrap();
        assert!(
            handle_edit_symbol(&json!({"library_path":p,"symbol_name":"R"}), &ctx())
                .await
                .unwrap()
                .is_error
        );
        for value in [json!(null), json!(false), json!([]), json!("bad\0value")] {
            let r = handle_edit_symbol(
                &json!({"library_path":p,"symbol_name":"R","description":value}),
                &ctx(),
            )
            .await
            .unwrap();
            assert!(r.is_error);
            assert_eq!(std::fs::read_to_string(&p).unwrap(), REAL);
        }
        let r = body(
            handle_edit_symbol(
                &json!({"library_path":p,"symbol_name":"R","description":"Updated"}),
                &ctx(),
            )
            .await
            .unwrap(),
        );
        assert_eq!(r["properties"]["Description"], "Updated");
    }
    #[tokio::test]
    async fn check_identifier_sets_including_missing_and_extra_pads() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.kicad_sym");
        let f = d.path().join("a.kicad_mod");
        std::fs::write(&p, REAL).unwrap();
        let fixture = include_str!("../../tests/fixtures/c_0603_1608metric_kicad10.kicad_mod");
        std::fs::write(&f, fixture).unwrap();
        let r = body(
            handle_check_symbol_footprint(
                &json!({"library_path":p,"symbol_name":"R","footprint_path":f}),
                &ctx(),
            )
            .await
            .unwrap(),
        );
        assert_eq!(r["result"], "pass", "{r}");
        let variant = fixture.replacen("(pad \"2\"", "(pad \"9\"", 1);
        assert_ne!(variant, fixture);
        std::fs::write(&f, variant).unwrap();
        let r = body(
            handle_check_symbol_footprint(
                &json!({"library_path":p,"symbol_name":"R","footprint_path":f}),
                &ctx(),
            )
            .await
            .unwrap(),
        );
        assert_eq!(r["result"], "fail");
        assert_eq!(r["missing_pads"], json!(["2"]));
        assert_eq!(r["extra_pads"], json!(["9"]));
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;
    #[tokio::test]
    async fn project_footprints_are_ranked_paged_and_missing_entries_disclosed() {
        let d = tempfile::tempdir().unwrap();
        let lib = d.path().join("Parts.pretty");
        std::fs::create_dir(&lib).unwrap();
        let source = include_str!("../../tests/fixtures/c_0603_1608metric_kicad10.kicad_mod");
        for name in ["PR10_UNIQUE", "PR10_UNIQUE_more"] {
            std::fs::write(lib.join(format!("{name}.kicad_mod")), source).unwrap();
        }
        let table=format!("(fp_lib_table (lib (name \"LocalPR10\") (type \"KiCad\") (uri {}) (options \"\") (descr \"\")) (lib (name \"BrokenPR10\") (type \"KiCad\") (uri {}) (options \"\") (descr \"\")))",serde_json::to_string(&lib.to_string_lossy()).unwrap(),serde_json::to_string(&d.path().join("missing.pretty").to_string_lossy()).unwrap());
        std::fs::write(d.path().join("fp-lib-table"), table).unwrap();
        let ctx = ToolContext::new(
            crate::tools::ServerConfig::default(),
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        let r = search(
            &json!({"query":"PR10_UNIQUE","project_dir":d.path(),"limit":1}),
            &ctx,
            true,
        )
        .await
        .unwrap();
        let crate::mcp::protocol::ToolContent::Text { text } = &r.content[0] else {
            panic!()
        };
        let value: Value = serde_json::from_str(text).unwrap();
        assert_eq!(value["results"][0]["name"], "PR10_UNIQUE");
        assert_eq!(value["total_matches"], 2);
        assert_eq!(value["next_offset"], 1);
        assert_eq!(value["truncated"], true);
        assert_eq!(value["search_complete"], false);
        assert!(value["skipped_library_count"].as_u64().unwrap() > 0);
    }
}

pub(super) fn saved_metadata(
    path: &Path,
    name: &str,
    expected: &BTreeMap<String, String>,
) -> anyhow::Result<BTreeMap<String, String>> {
    let source = read_consistent(path)?;
    let root = library(&source)?;
    let fields = properties(symbol(&root, name)?)?;
    anyhow::ensure!(
        expected.iter().all(|(k, v)| fields.get(k) == Some(v)),
        "saved metadata differs"
    );
    Ok(fields)
}

#[cfg(test)]
mod write_tests {
    use super::*;
    #[tokio::test]
    async fn create_metadata_is_saved_and_duplicate_refuses_without_overwrite() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("new.kicad_sym");
        let ctx = ToolContext::new(
            crate::tools::ServerConfig::default(),
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        );
        let args = json!({"library_path":p,"name":"Test","reference_prefix":"U","description":"metadata only","footprint":"Package:Part","keywords":"sensor custom","pins":[]});
        for name in ["", "bad\0name"] {
            let mut invalid = args.clone();
            invalid["name"] = json!(name);
            assert!(handle_create_symbol(&invalid, &ctx).await.unwrap().is_error);
            assert!(!p.exists());
        }
        let r = handle_create_symbol(&args, &ctx).await.unwrap();
        assert!(!r.is_error, "{r:?}");
        let bytes = std::fs::read_to_string(&p).unwrap();
        let root = library(&bytes).unwrap();
        let values = properties(symbol(&root, "Test").unwrap()).unwrap();
        assert_eq!(values["Description"], "metadata only");
        assert_eq!(values["Footprint"], "Package:Part");
        assert_eq!(values["ki_keywords"], "sensor custom");
        assert!(handle_create_symbol(&args, &ctx).await.unwrap().is_error);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), bytes);
    }
    #[test]
    fn saved_verification_does_not_accept_wrong_metadata() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("part.kicad_sym");
        std::fs::write(
            &p,
            include_str!("../../tests/fixtures/reference_prefix_kicad10/Device.kicad_sym"),
        )
        .unwrap();
        assert!(saved_metadata(
            &p,
            "R",
            &BTreeMap::from([("Description".into(), "not the fixture".into())])
        )
        .is_err());
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    #[test]
    fn nul_and_types_are_rejected_before_edit_planning() {
        assert!(metadata(&json!({"reference_prefix":""})).is_err());
        for v in [json!(null), json!(false), json!("bad\0value")] {
            assert!(metadata(&json!({"description":v})).is_err());
        }
    }
    #[test]
    fn commented_fake_definitions_are_refused() {
        assert!(library("(kicad_symbol_lib\n# (symbol \"FAKE\")\n)").is_err());
        assert!(library(
            "(kicad_symbol_lib (symbol \"A\" (property \"Description\" \"#literal\")))"
        )
        .is_ok());
    }
}
