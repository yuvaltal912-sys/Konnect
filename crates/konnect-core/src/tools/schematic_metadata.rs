//! Saved-sheet title blocks. Targeted spans preserve every unrelated byte;
//! prospective and saved observations establish what the edit actually did.

use crate::mcp::protocol::CallToolResult;
use crate::tool;
use crate::tools::{get_path, invalid_arg, mutation_outcome_uncertain, ToolContext, ToolDef};
use konnect_sexp::writer::{
    apply_edits, find_direct_child_blocks, read_consistent, write_atomic_if_unchanged, SexpEdit,
};
use konnect_sexp::{parse_sexp, SexpNode};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

type Values = BTreeMap<String, String>;

pub(super) fn tool() -> ToolDef {
    tool!(
        "set_schematic_title_block",
        "Edit the named saved sheet's title, date, revision, company or comments. Omitted \
         fields stay unchanged; an empty string clears a field. Comment keys are 1 through \
         9. Preserves unrelated content and reports independently read-back saved values. \
         Writes refuse an editor lock or concurrent file change. Source with unsupported \
         line-leading '#' comments is refused rather than treating comments as metadata. \
         Does not edit other sheets or PCB metadata.",
        json!({
            "type": "object", "additionalProperties": false,
            "properties": {
                "schematic": {"type": "string"},
                "title": {"type": "string"},
                "date": {"type": "string", "description": "Free-form date text; no automatic date is inserted"},
                "revision": {"type": "string"},
                "company": {"type": "string"},
                "comments": {"type": "object", "minProperties": 1,
                    "patternProperties": {"^[1-9]$": {"type": "string"}}, "additionalProperties": false}
            },
            "required": ["schematic"],
            "anyOf": [{"required":["title"]},{"required":["date"]},{"required":["revision"]},{"required":["company"]},{"required":["comments"]}]
        }),
        |args, ctx| async move { handle_set_schematic_title_block(args, ctx).await }
    )
}

struct Layout {
    values: Values,
    title_block: Option<(usize, usize)>,
    fields: BTreeMap<String, (usize, usize)>,
    children: Vec<(usize, usize)>,
}

fn keys() -> Vec<String> {
    ["title", "date", "revision", "company"]
        .into_iter()
        .map(str::to_owned)
        .chain((1..=9).map(|n| format!("comment:{n}")))
        .collect()
}

fn parse_update(args: &Value) -> Result<Values, CallToolResult> {
    let Some(object) = args.as_object() else {
        return Err(invalid_arg("arguments", "must be an object"));
    };
    let mut values = Values::new();
    for (key, value) in object {
        match key.as_str() {
            "schematic" => {}
            "title" | "date" | "revision" | "company" => {
                values.insert(key.clone(), string_value(value, key)?);
            }
            "comments" => {
                let Some(comments) = value.as_object().filter(|m| !m.is_empty()) else {
                    return Err(invalid_arg(
                        "comments",
                        "must be a nonempty object with keys 1 through 9",
                    ));
                };
                for (number, value) in comments {
                    if number.len() != 1 || !matches!(number.as_bytes()[0], b'1'..=b'9') {
                        return Err(invalid_arg("comments", "keys must be 1 through 9"));
                    }
                    values.insert(
                        format!("comment:{number}"),
                        string_value(value, "comments")?,
                    );
                }
            }
            _ => return Err(invalid_arg(key, "unknown title-block argument")),
        }
    }
    if values.is_empty() {
        return Err(invalid_arg(
            "metadata",
            "supply at least one title-block field",
        ));
    }
    Ok(values)
}

fn string_value(value: &Value, field: &str) -> Result<String, CallToolResult> {
    let Some(value) = value.as_str() else {
        return Err(invalid_arg(field, "must be a string when supplied"));
    };
    if value.contains('\0') {
        return Err(invalid_arg(field, "must not contain NUL characters"));
    }
    Ok(value.into())
}

fn layout(source: &str) -> Result<Layout, String> {
    if has_line_comment(source) {
        return Err("title-block edits do not support line-leading '#' comments".into());
    }
    // parse_sexp also accepts an implicit list of several top-level forms.
    // Header editing requires one explicit, fully consumed document root.
    let trimmed = source.trim();
    if !trimmed.starts_with('(')
        || konnect_sexp::writer::find_balanced_block(trimmed, 0)
            .is_none_or(|(start, end)| start != 0 || end != trimmed.len())
    {
        return Err("file must contain exactly one explicit schematic root".into());
    }
    let root = parse_sexp(source).map_err(|e| format!("unreadable schematic: {e}"))?;
    if root.head() != Some("kicad_sch") {
        return Err("file must have one kicad_sch root".into());
    }
    let children = find_direct_child_blocks(source, "kicad_sch");
    let blocks: Vec<_> = children
        .iter()
        .copied()
        .filter(|(start, end)| {
            parse_sexp(&source[*start..*end]).is_ok_and(|n| n.head() == Some("title_block"))
        })
        .collect();
    if blocks.len() > 1 {
        return Err("duplicate title_block nodes are ambiguous".into());
    }
    let mut values: Values = keys().into_iter().map(|key| (key, String::new())).collect();
    let mut fields = BTreeMap::new();
    let title_block = blocks.first().copied();
    if let Some((start, end)) = title_block {
        let block = parse_sexp(&source[start..end]).map_err(|error| error.to_string())?;
        if !block.children().is_some_and(|children| {
            children.iter().skip(1).all(|child| {
                matches!(child, SexpNode::List(items) if matches!(items.first(), Some(SexpNode::Atom(_))))
            })
        }) {
            return Err("title_block fields must be nonempty lists with unquoted field names".into());
        }
        for (relative_start, relative_end) in
            find_direct_child_blocks(&source[start..end], "title_block")
        {
            let range = (start + relative_start, start + relative_end);
            let node = parse_sexp(&source[range.0..range.1]).map_err(|e| e.to_string())?;
            let (key, value_index) = match node.head() {
                Some("title" | "date" | "company") => (node.head().unwrap().to_owned(), 1),
                Some("rev") => ("revision".into(), 1),
                Some("comment") => {
                    let number = node
                        .get(1)
                        .and_then(SexpNode::as_str)
                        .and_then(|v| v.parse::<u8>().ok())
                        .filter(|n| (1..=9).contains(n))
                        .ok_or("invalid saved comment index")?;
                    (format!("comment:{number}"), 2)
                }
                _ => continue,
            };
            let value = node
                .get(value_index)
                .and_then(SexpNode::as_str)
                .ok_or("invalid saved title-block value")?;
            if node.children().map_or(0, |v| v.len()) != value_index + 1 {
                return Err("invalid saved title-block field structure".into());
            }
            if fields.insert(key.clone(), range).is_some() {
                return Err(format!("duplicate saved {key} fields are ambiguous"));
            }
            values.insert(key, value.to_owned());
        }
    }
    Ok(Layout {
        values,
        title_block,
        fields,
        children,
    })
}

/// KiCad treats a line-leading `#` outside a quoted string as a comment.
/// The current Konnect parsers do not model comments, so refuse instead of
/// accidentally editing a balanced title_block written inside a comment.
fn has_line_comment(source: &str) -> bool {
    let mut in_string = false;
    let mut escaped = false;
    let mut line_start = true;
    for c in source.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '\n' || c == '\r' {
            line_start = true;
        } else if line_start && c == '#' {
            return true;
        } else if c == '"' {
            in_string = true;
            line_start = false;
        } else if !c.is_ascii_whitespace() {
            line_start = false;
        }
    }
    false
}

fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn field_node(key: &str, value: &str) -> String {
    if let Some(number) = key.strip_prefix("comment:") {
        format!("(comment {number} {})", quote(value))
    } else {
        format!(
            "({} {})",
            if key == "revision" { "rev" } else { key },
            quote(value)
        )
    }
}

fn indent(source: &str, position: usize) -> Option<&str> {
    let line = source[..position].rfind('\n').map_or(0, |n| n + 1);
    source[line..position]
        .chars()
        .all(|c| c == ' ' || c == '\t')
        .then_some(&source[line..position])
}

fn prepare(source: &str, update: &Values) -> Result<(String, Values, Vec<String>), String> {
    let before = layout(source)?;
    let mut expected = before.values.clone();
    expected.extend(update.clone());
    let changed: Vec<_> = keys()
        .into_iter()
        .filter(|key| before.values.get(key) != expected.get(key))
        .collect();
    if changed.is_empty() {
        return Ok((source.into(), expected, changed));
    }
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let root_indent = before
        .children
        .iter()
        .find_map(|(start, _)| indent(source, *start).filter(|v| !v.is_empty()))
        .unwrap_or("  ");
    let mut edits = Vec::new();
    let mut insertions = Vec::new();
    for key in &changed {
        let node = field_node(key, &expected[key]);
        if let Some(&(start, end)) = before.fields.get(key) {
            edits.push(SexpEdit::replace(start, end, node));
        } else {
            insertions.push(node);
        }
    }
    if !insertions.is_empty() {
        if let Some((start, end)) = before.title_block {
            let parent_indent = indent(source, start).unwrap_or(root_indent);
            let fallback = format!("{parent_indent}{root_indent}");
            let child_indent = before
                .fields
                .values()
                .find_map(|(start, _)| indent(source, *start))
                .unwrap_or(&fallback);
            let close = end - 1;
            let line_start = source[..close].rfind('\n').map_or(0, |n| n + 1);
            let nodes = insertions.join(&format!("{newline}{child_indent}"));
            if source[line_start..close]
                .chars()
                .all(|c| c == ' ' || c == '\t')
            {
                edits.push(SexpEdit::insert(
                    line_start,
                    format!("{child_indent}{nodes}{newline}"),
                ));
            } else {
                edits.push(SexpEdit::insert(
                    close,
                    format!("{newline}{child_indent}{nodes}{newline}{parent_indent}"),
                ));
            }
        } else {
            let child_indent = format!("{root_indent}{root_indent}");
            let block = format!(
                "(title_block{newline}{child_indent}{}{newline}{root_indent})",
                insertions.join(&format!("{newline}{child_indent}"))
            );
            let anchor = before
                .children
                .iter()
                .find(|(start, end)| {
                    parse_sexp(&source[*start..*end]).is_ok_and(|n| {
                        !matches!(
                            n.head(),
                            Some("version" | "generator" | "generator_version" | "uuid" | "paper")
                        )
                    })
                })
                .map(|range| range.0);
            if let Some(anchor) = anchor {
                edits.push(SexpEdit::insert(
                    anchor,
                    format!("{block}{newline}{root_indent}"),
                ));
            } else {
                let close = konnect_sexp::writer::find_balanced_block(
                    source,
                    source.find('(').ok_or("missing root")?,
                )
                .ok_or("unbalanced root")?
                .1 - 1;
                edits.push(SexpEdit::insert(
                    close,
                    format!("{newline}{root_indent}{block}{newline}"),
                ));
            }
        }
    }
    Ok((apply_edits(source.into(), edits), expected, changed))
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    Prospective,
    Readback,
}
#[cfg(test)]
thread_local! { static FAULT: std::cell::Cell<Fault> = const { std::cell::Cell::new(Fault::None) }; }

async fn handle_set_schematic_title_block(
    args: &Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let update = match parse_update(args) {
        Ok(v) => v,
        Err(e) => return Ok(e),
    };
    let path = get_path(args, "schematic")?;
    let source = read_consistent(&path)?;
    let (candidate, expected, changed) = match prepare(&source, &update) {
        Ok(v) => v,
        Err(e) => return Ok(invalid_arg("schematic", &e)),
    };
    #[cfg(test)]
    let candidate = FAULT.with(|fault| {
        if fault.get() == Fault::Prospective {
            fault.set(Fault::None);
            candidate.replacen("(title \"", "(title \"corrupted ", 1)
        } else {
            candidate
        }
    });
    if let Err(error) = verify(&path, &candidate, &expected) {
        return Ok(invalid_arg(
            "metadata",
            &format!("prospective title block does not match: {error}"),
        ));
    }
    if candidate != source {
        write_atomic_if_unchanged(&path, &source, &candidate)?;
    }
    let saved = match read_consistent(&path) {
        Ok(v) => v,
        Err(e) => {
            return Ok(mutation_outcome_uncertain(
                &path,
                "set_schematic_title_block",
                e.to_string(),
            ))
        }
    };
    #[cfg(test)]
    let saved = FAULT.with(|fault| {
        if fault.get() == Fault::Readback {
            fault.set(Fault::None);
            saved.replacen("(title \"", "(title \"corrupted ", 1)
        } else {
            saved
        }
    });
    let observed = match verify(&path, &saved, &expected) {
        Ok(v) => v,
        Err(e) => {
            return Ok(mutation_outcome_uncertain(
                &path,
                "set_schematic_title_block",
                e,
            ))
        }
    };
    Ok(CallToolResult::json(&json!({
        "schematic": path, "source": "saved_file", "updated": !changed.is_empty(), "changed_fields": changed,
        "title_block": metadata_json(&observed)
    })))
}

fn verify(path: &Path, source: &str, expected: &Values) -> Result<Values, String> {
    konnect_schematic_editor::Schematic::from_source(path, source).map_err(|e| e.to_string())?;
    let observed = layout(source)?.values;
    if &observed != expected {
        return Err("observed title-block values differ from the intended edit".into());
    }
    Ok(observed)
}

fn metadata_json(values: &Values) -> Value {
    let comments: BTreeMap<_, _> = (1..=9)
        .map(|n| (n.to_string(), values[&format!("comment:{n}")].clone()))
        .collect();
    json!({"title": values["title"], "date": values["date"], "revision": values["revision"], "company": values["company"], "comments": comments})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::protocol::ToolContent;

    const KICAD9: &str =
        include_str!("../../../konnect-schematic-editor/tests/fixtures/labels_kicad9.kicad_sch");
    const KICAD7: &str =
        include_str!("../../../konnect-schematic-editor/tests/fixtures/labels_kicad7.kicad_sch");

    fn context() -> ToolContext {
        ToolContext::new(
            crate::tools::ServerConfig::default(),
            std::sync::Arc::new(crate::router::ToolRouter::new()),
        )
    }
    fn body(result: &CallToolResult) -> Value {
        let ToolContent::Text { text } = &result.content[0] else {
            panic!("expected JSON")
        };
        serde_json::from_str(text).unwrap()
    }
    async fn call(path: &Path, mut args: Value) -> CallToolResult {
        args["schematic"] = json!(path);
        handle_set_schematic_title_block(&args, &context())
            .await
            .unwrap()
    }
    fn scene(source: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sheet.kicad_sch");
        std::fs::write(&path, source).unwrap();
        (dir, path)
    }

    #[tokio::test]
    async fn partial_update_on_real_kicad9_preserves_other_fields_and_all_other_bytes() {
        let (_dir, path) = scene(KICAD9);
        let result = call(
            &path,
            json!({"title":"Edited title", "revision":"B", "comments":{"2":"updated"}}),
        )
        .await;
        assert!(!result.is_error, "{result:?}");
        let observed = body(&result);
        assert_eq!(observed["title_block"]["title"], "Edited title");
        assert_eq!(observed["title_block"]["date"], "2023-11-22");
        assert_eq!(
            observed["title_block"]["comments"]["1"],
            "See LICENSE.txt for details"
        );
        assert_eq!(observed["title_block"]["comments"]["2"], "updated");
        let expected = KICAD9
            .replace("TinyTapeout 4/5 Demo Board PRELIM", "Edited title")
            .replace("(rev \"1.0.4\")", "(rev \"B\")")
            .replace("Licensed under Apache 2.0", "updated");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
    }

    #[tokio::test]
    async fn real_kicad7_preserves_comment_gaps_and_crlf_when_inserting_fields() {
        let (_dir, path) = scene(KICAD7);
        let result = call(
            &path,
            json!({"date":"Sat 7 Nov 2026","comments":{"9":"last"}}),
        )
        .await;
        assert!(!result.is_error, "{result:?}");
        let actual = std::fs::read_to_string(&path).unwrap();
        assert!(actual.contains("\r\n    (date \"Sat 7 Nov 2026\")\r\n"));
        assert!(actual.contains("(comment 6 \"Paul PERONNARD\")"));
        assert!(actual.contains("(comment 9 \"last\")"));
        assert!(!actual.replace("\r\n", "").contains('\n'));
        assert_eq!(body(&result)["title_block"]["company"], "CERN");
    }

    #[tokio::test]
    async fn creates_a_header_before_drawing_items_and_empty_strings_clear_without_erasing_comments(
    ) {
        let source = KICAD9;
        let l = layout(source).unwrap();
        let (start, end) = l.title_block.unwrap();
        let without = format!("{}{}", &source[..start], &source[end..]);
        let (_dir, path) = scene(&without);
        let result = call(
            &path,
            json!({"title":"New header", "comments":{"1":"first","9":"last"}}),
        )
        .await;
        assert!(!result.is_error);
        let actual = std::fs::read_to_string(&path).unwrap();
        assert!(actual.find("(title_block").unwrap() < actual.find("(lib_symbols").unwrap());
        let clear = call(&path, json!({"title":"", "comments":{"1":""}})).await;
        assert!(!clear.is_error);
        assert_eq!(body(&clear)["title_block"]["title"], "");
        assert_eq!(body(&clear)["title_block"]["comments"]["1"], "");
        assert_eq!(body(&clear)["title_block"]["comments"]["9"], "last");
        let before = std::fs::read(&path).unwrap();
        let repeated = call(&path, json!({"title":"", "comments":{"1":""}})).await;
        assert_eq!(body(&repeated)["updated"], false);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[tokio::test]
    async fn strings_round_trip_quotes_backslashes_controls_unicode_and_fake_forms() {
        let (_dir, path) = scene(KICAD9);
        let text = "Test \"quote\" \\path\tline\nnext\rעברית (title_block (title \"fake\"))";
        let result = call(&path, json!({"title":text,"comments":{"1":text}})).await;
        assert!(!result.is_error, "{result:?}");
        assert_eq!(body(&result)["title_block"]["title"], text);
        assert_eq!(body(&result)["title_block"]["comments"]["1"], text);
        assert_eq!(
            parse_sexp(&std::fs::read_to_string(path).unwrap())
                .unwrap()
                .find_all("title_block")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn preserves_unknown_children_and_does_not_match_nested_or_quoted_titles() {
        let source = KICAD9.replace(
            "(title_block\r\n",
            "(title_block\r\n\t\t(future_field \"leave (title \\\"hidden\\\") alone\")\r\n",
        );
        let (_dir, path) = scene(&source);
        let result = call(&path, json!({"company":"New company"})).await;
        assert!(!result.is_error, "{result:?}");
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            source.replace("Psychogenic Technologies", "New company")
        );
    }

    #[tokio::test]
    async fn invalid_argument_classes_are_refused_without_writing() {
        let (_dir, path) = scene(KICAD9);
        for args in [
            json!({}),
            json!({"title":"valid", "comments":{}}),
            json!({"title":null}),
            json!({"date":true}),
            json!({"revision":1}),
            json!({"company":[]}),
            json!({"title":"valid", "comments":[]}),
            json!({"comments":{"1":false}}),
            json!({"comments":{"0":"x"}}),
            json!({"comments":{"10":"x"}}),
            json!({"comments":{"01":"x"}}),
            json!({"comments":{"x":"x"}}),
            json!({"title":"nul\u{0}"}),
            json!({"title":"valid", "unexpected":"x"}),
        ] {
            assert!(
                parse_update(&args).is_err(),
                "argument guard accepted {args}"
            );
            let result = call(&path, args.clone()).await;
            assert!(result.is_error, "{args}: {result:?}");
            assert!(serde_json::to_string(&result)
                .unwrap()
                .contains("invalid_argument"));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), KICAD9);
        }
    }

    #[tokio::test]
    async fn duplicate_and_malformed_saved_metadata_is_refused_without_writing() {
        let (start, end) = layout(KICAD9).unwrap().title_block.unwrap();
        for source in [
            format!(
                "{}{}{}",
                &KICAD9[..start],
                &KICAD9[start..end],
                &KICAD9[start..]
            ),
            KICAD9.replace("(title_block\r\n", "(title_block (title \"duplicate\")\r\n"),
            KICAD9.replace(
                "(title_block\r\n",
                "(title_block (comment 1 \"duplicate\")\r\n",
            ),
            KICAD9.replace(
                "(title_block\r\n",
                "(title_block (comment 10 \"invalid\")\r\n",
            ),
            KICAD9.replace("(company \"Psychogenic Technologies\")", "(company)"),
            KICAD9.replace(
                "(company \"Psychogenic Technologies\")",
                "(company (nested))",
            ),
            KICAD9.replace(
                "(company \"Psychogenic Technologies\")",
                "(company \"bad\" \"extra\")",
            ),
            KICAD9.replace("(kicad_sch", "(not_a_schematic"),
            format!("{KICAD9}(other_root)"),
            format!("{KICAD9} trailing)"),
        ] {
            assert!(
                layout(&source).is_err(),
                "saved metadata guard accepted {source}"
            );
            let (_dir, path) = scene(&source);
            let result = call(&path, json!({"title":"Changed"})).await;
            assert!(result.is_error, "{result:?}");
            assert_eq!(std::fs::read_to_string(path).unwrap(), source);
        }
    }

    #[tokio::test]
    async fn malformed_title_block_child_shapes_are_refused_without_writing() {
        for child in ["stray", "\"stray\"", "()", "(\"title\" \"x\")"] {
            // Replace the existing title, so a quoted title head is not
            // independently refused by the duplicate-field guard.
            let source = KICAD9.replace("(title \"TinyTapeout 4/5 Demo Board PRELIM\")", child);
            assert!(layout(&source).is_err(), "accepted malformed field {child}");
            let (_dir, path) = scene(&source);
            let result = call(&path, json!({"title":"New title"})).await;
            assert!(result.is_error, "{child}: {result:?}");
            assert_eq!(std::fs::read_to_string(path).unwrap(), source);
        }
    }

    #[test]
    fn implicit_and_trailing_roots_are_not_header_documents() {
        for source in [
            "kicad_sch (title_block (title \"fake\"))",
            "(kicad_sch) (other_root)",
            "(kicad_sch) trailing)",
            "(not_a_schematic)",
        ] {
            assert!(layout(source).is_err(), "{source}");
        }
    }

    #[tokio::test]
    async fn commented_fake_title_blocks_are_refused_without_writing() {
        let (start, end) = layout(KICAD9).unwrap().title_block.unwrap();
        let without = format!("{}{}", &KICAD9[..start], &KICAD9[end..]);
        let source = without.replace(
            "(lib_symbols)",
            "# (title_block (title \"commented fake\"))\r\n\t(lib_symbols)",
        );
        let (_dir, path) = scene(&source);
        let result = call(&path, json!({"title":"Real title"})).await;
        assert!(result.is_error, "{result:?}");
        assert!(serde_json::to_string(&result).unwrap().contains("comments"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    }

    #[tokio::test]
    async fn hashes_inside_quoted_multiline_values_remain_metadata() {
        assert!(!has_line_comment(
            "(title \"Line 1\n# this is inside the title\")"
        ));
        assert!(!has_line_comment("(title \"Escaped \\\"# quote\\\"\")"));
        let (_dir, path) = scene(KICAD9);
        let title = "# heading\n# second line";
        let result = call(&path, json!({"title":title})).await;
        assert!(!result.is_error, "{result:?}");
        assert_eq!(body(&result)["title_block"]["title"], title);
    }

    #[test]
    fn parse_update_refuses_nonobjects_and_nul_before_document_access() {
        for args in [json!(null), json!([]), json!(true)] {
            let refusal = parse_update(&args).unwrap_err();
            assert_eq!(body(&refusal)["error"]["field"], "arguments");
        }
        assert!(parse_update(&json!({"title":"nul\u{0}"})).is_err());
    }

    #[test]
    fn published_schema_accepts_partial_updates_and_rejects_malformed_metadata() {
        let definition = tool();
        for args in [
            json!({"schematic":"sheet.kicad_sch","title":""}),
            json!({"schematic":"sheet.kicad_sch","date":"free-form date","revision":"B","company":"Company"}),
            json!({"schematic":"sheet.kicad_sch","comments":{"1":"first","9":""}}),
        ] {
            assert!(definition.input_validator.is_valid(&args), "{args}");
        }
        for args in [
            json!({"schematic":"sheet.kicad_sch"}),
            json!({"schematic":"sheet.kicad_sch","title":null}),
            json!({"schematic":"sheet.kicad_sch","title":"valid","unknown":"x"}),
            json!({"schematic":"sheet.kicad_sch","comments":{}}),
            json!({"schematic":"sheet.kicad_sch","comments":{"0":"x"}}),
            json!({"schematic":"sheet.kicad_sch","comments":{"10":"x"}}),
            json!({"schematic":"sheet.kicad_sch","comments":{"1":23}}),
        ] {
            assert!(!definition.input_validator.is_valid(&args), "{args}");
        }
    }

    #[tokio::test]
    async fn prospective_guard_refuses_corrupt_candidate_before_writing() {
        let (_dir, path) = scene(KICAD9);
        FAULT.with(|v| v.set(Fault::Prospective));
        let result = call(&path, json!({"title":"New title"})).await;
        assert!(result.is_error);
        assert_eq!(std::fs::read_to_string(path).unwrap(), KICAD9);
    }

    #[tokio::test]
    async fn saved_guard_reports_uncertain_outcome_after_write() {
        let (_dir, path) = scene(KICAD9);
        FAULT.with(|v| v.set(Fault::Readback));
        let result = call(&path, json!({"title":"New title"})).await;
        assert!(result.is_error);
        assert!(serde_json::to_string(&result)
            .unwrap()
            .contains("mutation_outcome_uncertain"));
        assert_eq!(
            layout(&std::fs::read_to_string(path).unwrap())
                .unwrap()
                .values["title"],
            "New title"
        );
    }

    #[test]
    fn stale_source_and_editor_lock_cannot_be_overwritten() {
        let (_dir, path) = scene(KICAD9);
        let (candidate, _, _) =
            prepare(KICAD9, &parse_update(&json!({"title":"Changed"})).unwrap()).unwrap();
        let conflict = write_atomic_if_unchanged(&path, "stale source", &candidate).unwrap_err();
        assert!(matches!(conflict, konnect_sexp::SexpError::Conflict { .. }));
        let lock = konnect_sexp::writer::kicad_editor_lock_path(&path).unwrap();
        std::fs::write(lock, b"editor owns this sheet").unwrap();
        assert!(matches!(
            write_atomic_if_unchanged(&path, KICAD9, &candidate).unwrap_err(),
            konnect_sexp::SexpError::KiCadEditorLocked { .. }
        ));
        assert_eq!(std::fs::read_to_string(path).unwrap(), KICAD9);
    }

    #[tokio::test]
    #[ignore = "requires real kicad-cli; exports a scratch copy only"]
    async fn real_kicad_exports_the_saved_title_block() {
        let source = include_str!("../../tests/fixtures/power_rail.kicad_sch");
        let (dir, path) = scene(source);
        let result = call(&path,json!({"title":"PR6 verified title","revision":"B","company":"PR6 company","comments":{"1":"PR6 verified comment","9":"Last valid comment"}})).await;
        assert!(!result.is_error, "{result:?}");
        let cli = std::env::var("KICAD_CLI")
            .unwrap_or_else(|_| "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli".into());
        let output = dir.path().join("export");
        std::fs::create_dir(&output).unwrap();
        let svg =
            super::super::cli::export_schematic_svg(&cli, &path, &output, &Default::default())
                .await
                .unwrap();
        let text = std::fs::read_to_string(svg).unwrap();
        assert!(
            text.contains("PR6 verified title"),
            "title not in KiCad SVG"
        );
        assert!(
            text.contains("PR6 verified comment"),
            "comment not in KiCad SVG"
        );
    }
}
