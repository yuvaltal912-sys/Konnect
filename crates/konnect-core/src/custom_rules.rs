//! Complete rule-file framing and lossless named edits. KiCad compiles expressions.
use anyhow::{bail, ensure, Context, Result};
use konnect_sexp::{parse_sexp, SexpNode};
use std::collections::BTreeSet;
use std::ops::Range;

pub(crate) const PROBE: &str = "__konnect_rule_validation_probe__";

#[derive(Debug)]
pub(crate) struct Rule {
    pub name: String,
    pub span: Range<usize>,
    pub expressions: Vec<String>,
    pub node: SexpNode,
}

/// Unlike parse_sexp alone, this consumes every byte and rejects trailing junk.
fn frames(source: &str) -> Result<Vec<Range<usize>>> {
    ensure!(source.len() <= 524_288, "rule document exceeds 512 KiB");
    ensure!(!source.contains('\0'), "rule document contains NUL");
    let mut spans = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    let (mut quoted, mut escaped, mut comment) = (false, false, false);
    for (i, b) in source.bytes().enumerate() {
        if comment {
            if b == b'\n' {
                comment = false;
            }
            continue;
        }
        if quoted {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                quoted = false;
            }
            continue;
        }
        match b {
            b'#' => comment = true,
            b'"' if depth > 0 => quoted = true,
            b'(' => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
                ensure!(depth <= 64, "rule nesting exceeds 64");
            }
            b')' => {
                ensure!(depth > 0, "unmatched rule closing parenthesis");
                depth -= 1;
                if depth == 0 {
                    spans.push(start..i + 1);
                }
            }
            _ if depth == 0 && !b.is_ascii_whitespace() => {
                bail!("unexpected text outside rule statements")
            }
            _ => {}
        }
    }
    ensure!(depth == 0 && !quoted, "incomplete rule statement or string");
    Ok(spans)
}

fn node(source: &str) -> Result<SexpNode> {
    let mut bytes = source.as_bytes().to_vec();
    let (mut quoted, mut escaped, mut comment) = (false, false, false);
    for b in &mut bytes {
        if comment {
            if *b == b'\n' {
                comment = false;
            } else {
                *b = b' ';
            }
        } else if quoted {
            if escaped {
                escaped = false;
            } else if *b == b'\\' {
                escaped = true;
            } else if *b == b'"' {
                quoted = false;
            }
        } else if *b == b'"' {
            quoted = true;
        } else if *b == b'#' {
            comment = true;
            *b = b' ';
        }
    }
    parse_sexp(std::str::from_utf8(&bytes)?).context("parse framed custom rule")
}

pub(crate) fn name_valid(name: &str) -> bool {
    !name.trim().is_empty()
        && name.len() <= 256
        && !name.chars().any(char::is_control)
        && !name.contains(PROBE)
}

fn rule(source: &str, span: Range<usize>) -> Result<Rule> {
    let node = node(&source[span.clone()])?;
    ensure!(
        node.head() == Some("rule"),
        "only version and rule statements are supported"
    );
    let children = node.children().context("rule must be a list")?;
    let name = children
        .get(1)
        .and_then(SexpNode::as_str)
        .context("rule lacks a name")?
        .to_owned();
    ensure!(name_valid(&name), "invalid or reserved rule name");
    let (mut fields, mut expressions, mut constraints) = (BTreeSet::new(), Vec::new(), 0);
    for child in children.iter().skip(2) {
        match child.head() {
            Some("condition" | "layer" | "severity") => {
                let key = child.head().unwrap();
                ensure!(fields.insert(key), "duplicate rule selector");
                let args = child.children().unwrap();
                ensure!(args.len() == 2, "rule selector must have one value");
                let value = args[1]
                    .as_str()
                    .context("rule selector value must be text")?;
                if key == "condition" {
                    expressions.push(value.to_owned());
                }
            }
            Some("constraint") => {
                let args = child.children().unwrap();
                ensure!(args.len() >= 3, "constraint needs a type and value");
                let kind = args[1].as_str().context("constraint type must be text")?;
                if kind == "assertion" {
                    ensure!(args.len() == 3, "assertion needs one expression");
                    expressions.push(
                        args[2]
                            .as_str()
                            .context("assertion expression must be text")?
                            .to_owned(),
                    );
                }
                constraints += 1;
            }
            _ => bail!("unsupported rule clause"),
        }
    }
    ensure!(constraints > 0, "rule needs a constraint");
    ensure!(
        !source[span.clone()].contains("${"),
        "unresolved text variables are not supported in custom rules"
    );
    Ok(Rule {
        name,
        span,
        expressions,
        node,
    })
}

pub(crate) fn parse(source: &str) -> Result<Vec<Rule>> {
    let spans = frames(source)?;
    let version = spans.first().context("rule document needs version 1")?;
    let version = node(&source[version.clone()])?;
    ensure!(
        version.head() == Some("version")
            && version.children().is_some_and(|v| v.len() == 2)
            && version.get(1).and_then(SexpNode::as_str) == Some("1"),
        "rule document needs version 1 first"
    );
    ensure!(spans.len() <= 129, "rule document exceeds 128 rules");
    let mut names = BTreeSet::new();
    spans
        .into_iter()
        .skip(1)
        .map(|span| {
            let r = rule(source, span)?;
            ensure!(names.insert(r.name.clone()), "duplicate rule name");
            Ok(r)
        })
        .collect()
}

#[derive(Clone, Copy)]
pub(crate) enum Operation {
    Create,
    Edit,
    Delete,
}

pub(crate) fn change(
    source: &str,
    name: &str,
    replacement: Option<&str>,
    operation: Operation,
) -> Result<String> {
    let rules = parse(source)?;
    let existing = rules.iter().find(|r| r.name == name);
    let mut replacement_node = None;
    let text = if let Some(raw) = replacement {
        let spans = frames(raw)?;
        ensure!(
            spans.len() == 1,
            "replacement must contain exactly one rule"
        );
        let r = rule(raw, spans[0].clone())?;
        ensure!(
            r.name == name,
            "replacement rule name differs from requested name"
        );
        replacement_node = Some(r.node);
        Some(raw)
    } else {
        None
    };
    let result = match operation {
        Operation::Create => {
            format!("{source}\n{}\n", text.context("create requires a rule")?)
        }
        Operation::Edit | Operation::Delete => {
            let r = existing.context("rule name does not exist")?;
            let text = match operation {
                Operation::Edit => text.context("edit requires a rule")?,
                _ => "",
            };
            format!(
                "{}{}{}",
                &source[..r.span.start],
                text,
                &source[r.span.end..]
            )
        }
    };
    let mut expected = Vec::new();
    for r in &rules {
        if r.name == name && matches!(operation, Operation::Delete) {
            continue;
        }
        if r.name == name && matches!(operation, Operation::Edit) {
            expected.push(
                replacement_node
                    .clone()
                    .context("missing replacement node")?,
            );
        } else {
            expected.push(r.node.clone());
        }
    }
    if matches!(operation, Operation::Create) {
        expected.push(replacement_node.context("missing new rule node")?);
    }
    let actual = parse(&result)?
        .into_iter()
        .map(|r| r.node)
        .collect::<Vec<_>>();
    ensure!(
        actual == expected,
        "rule edit changes the meaning or order of unrelated rules"
    );
    Ok(result)
}

/// KiCad's assertion parser can ignore a failed Compile(). Recompile every
/// condition/assertion as a condition that must evaluate true on the probe.
pub(crate) fn probe_condition(source: &str) -> Result<String> {
    let expressions = parse(source)?
        .into_iter()
        .flat_map(|r| r.expressions)
        .collect::<Vec<_>>();
    Ok(if expressions.is_empty() {
        "1 == 1".to_owned()
    } else {
        expressions
            .into_iter()
            .map(|e| format!("(({e}) || (1 == 1))"))
            .collect::<Vec<_>>()
            .join(" && ")
    })
}

pub(crate) fn quote(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
    )
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RoutingBounds {
    pub clearance_mm: f64,
    pub width_mm: f64,
}

/// The DSN exporter only translates unconditional, explicit-mm minima.
pub(crate) fn routing_bounds(source: &str) -> Result<RoutingBounds> {
    let mut out = RoutingBounds::default();
    for r in parse(source)? {
        let clauses = &r.node.children().unwrap()[2..];
        ensure!(
            clauses.iter().all(|c| c.head() == Some("constraint")),
            "custom DRC rules contain selectors outside the DSN profile"
        );
        for clause in clauses.iter().filter(|c| c.head() == Some("constraint")) {
            let args = clause.children().unwrap();
            let kind = args[1].as_str().unwrap();
            ensure!(
                matches!(kind, "clearance" | "track_width"),
                "custom DRC constraint is outside the DSN profile"
            );
            ensure!(
                args.len() == 3,
                "custom DRC bounds are outside the DSN profile"
            );
            let bound = args[2]
                .children()
                .context("custom DRC minimum must be a list")?;
            ensure!(
                bound.len() == 2 && bound[0].as_str() == Some("min"),
                "DSN needs exactly one minimum"
            );
            let raw = bound[1]
                .as_str()
                .and_then(|s| s.strip_suffix("mm"))
                .context("DSN needs an explicit numeric mm minimum")?;
            let min: f64 = raw
                .parse()
                .context("DSN cannot translate bound expressions")?;
            ensure!(
                min.is_finite() && (0.000001..=100.0).contains(&min),
                "DSN custom minimum is out of range"
            );
            let upper = (min * 1000.0).ceil() / 1000.0;
            if kind == "clearance" {
                out.clearance_mm = out.clearance_mm.max(upper);
            } else {
                out.width_mm = out.width_mm.max(upper);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIRST: &str = "(rule \"One\" (condition \"A.NetName == 'GND#1'\") (constraint clearance (min 0.2mm + 0.1mm)))";
    #[test]
    fn lossless_edit_and_delete_preserve_comments_and_rule_order() {
        let src = format!("# header\r\n(version 1)\r\n# before\r\n{FIRST}\r\n# middle\r\n(rule \"Two\" (constraint track_width (min 0.3mm)))\r\n# tail\r\n");
        let replacement = FIRST.replace("0.2mm + 0.1mm", "0.4mm");
        let edited = change(&src, "One", Some(&replacement), Operation::Edit).unwrap();
        assert_eq!(edited, src.replacen(FIRST, &replacement, 1));
        let deleted = change(&src, "One", None, Operation::Delete).unwrap();
        assert_eq!(deleted, src.replacen(FIRST, "", 1));
        assert_eq!(parse(&deleted).unwrap()[0].name, "Two");
        let created = change(&deleted, "One", Some(FIRST), Operation::Create).unwrap();
        let parsed = parse(&created).unwrap();
        assert_eq!(
            parsed.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["Two", "One"]
        );
    }
    #[test]
    fn strict_complete_framing_refuses_ambiguous_or_trailing_input() {
        for src in ["", "(version 2)", "(version 1) garbage", "(version 1))", "(version 1)(rule \"X\"", "(version 1)(rule \"X\" (condition \"bad)", "(version 1)\0", "(version 1)(unknown x)", "(version 1)(version 1)", "(version 1)(rule \"X\" (constraint clearance))", "(version 1)(rule \"X\" (condition \"A\") (condition \"B\") (constraint clearance (min 1mm)))"] {
            assert!(parse(src).is_err(),"{src}");
        }
        let duplicate = format!("(version 1)\n{FIRST}\n{FIRST}");
        assert!(parse(&duplicate).is_err());
        assert!(parse("(version 1)(rule \"__konnect_rule_validation_probe__\" (constraint clearance (min 1mm)))").is_err());
        assert!(parse("(version 1)(rule \"X\" (constraint assertion \"${UNRESOLVED}\"))").is_err());
    }
    #[test]
    fn malformed_rule_fields_and_names_are_not_guessed() {
        for statement in [
            "(other \"X\" (constraint clearance (min 1mm)))",
            "(rule \"\" (constraint clearance (min 1mm)))",
            "(rule (name) (constraint clearance (min 1mm)))",
            "(rule \"X\")",
            "(rule \"X\" (note \"unknown\") (constraint clearance (min 1mm)))",
            "(rule \"X\" (condition \"1 == 1\" \"extra\") (constraint clearance (min 1mm)))",
            "(rule \"X\" (condition (value)) (constraint clearance (min 1mm)))",
            "(rule \"X\" (constraint (type) (min 1mm)))",
            "(rule \"X\" (constraint assertion (value)))",
            "(rule \"X\" (constraint assertion \"1 == 1\" \"extra\"))",
        ] {
            assert!(
                parse(&format!("(version 1){statement}")).is_err(),
                "{statement}"
            );
        }
        assert!(parse("(version 1)# bad\0").is_err());
        assert!(parse(&format!(
            "(version 1)(rule \"{}\" (constraint clearance (min 1mm)))",
            "x".repeat(257)
        ))
        .is_err());
    }
    #[test]
    fn operations_require_exact_existing_identity_and_one_replacement() {
        let src = format!("(version 1)\n{FIRST}");
        assert!(change(&src, "One", Some(FIRST), Operation::Create).is_err());
        assert!(change(&src, "Missing", None, Operation::Delete).is_err());
        assert!(change(&src, "Missing", Some(FIRST), Operation::Edit).is_err());
        assert!(change(
            &src,
            "One",
            Some(&format!("{FIRST}\n{FIRST}")),
            Operation::Edit
        )
        .is_err());
        assert!(change(&src, "One", None, Operation::Edit).is_err());
        assert!(change(&src, "One", Some(""), Operation::Edit).is_err());
        assert!(change("(version 1)", "New", None, Operation::Create).is_err());
    }
    #[test]
    fn trailing_replacement_comments_cannot_hide_an_adjacent_rule() {
        let old = "(rule \"One\" (constraint clearance (min 0.2mm)))";
        let other = "(rule \"Other\" (constraint track_width (min 0.3mm)))";
        let source = format!("(version 1){old}{other}");
        let replacement = format!("{old} # would hide the adjacent rule");
        assert!(change(&source, "One", Some(&replacement), Operation::Edit).is_err());
        let valid = format!("{old} # terminated comment\n");
        let result = change(&source, "One", Some(&valid), Operation::Edit).unwrap();
        assert_eq!(parse(&result).unwrap().len(), 2);
        assert!(result.contains(other));
    }
    #[test]
    fn document_limits_refuse_before_unbounded_native_work() {
        assert!(parse(&format!("(version 1){}", " ".repeat(524_289))).is_err());
        assert!(frames(&format!("{}{}", "(".repeat(65), ")".repeat(65))).is_err());
        let mut src = "(version 1)\n".to_owned();
        for n in 0..129 {
            src.push_str(&format!(
                "(rule \"R{n}\" (constraint clearance (min 1mm)))\n"
            ));
        }
        assert!(parse(&src).is_err());
    }
    #[test]
    fn assertions_and_conditions_both_enter_the_compilation_witness() {
        let src=format!("(version 1)\n{FIRST}\n(rule \"Assert\" (constraint assertion \"A.NoSuchProperty == 1\"))");
        let condition = probe_condition(&src).unwrap();
        assert!(condition.contains("A.NoSuchProperty"));
        assert!(condition.contains("A.NetName"));
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }
    #[test]
    fn routing_projection_refuses_selectors_units_multiple_bounds_and_nonfinite_values() {
        for statement in [
            "(rule \"X\" (condition \"1 == 1\") (constraint clearance (min 0.2mm)))",
            "(rule \"X\" (constraint clearance (min 0.2mm) (max 0.3mm)))",
            "(rule \"X\" (constraint clearance (min 0.2)))",
            "(rule \"X\" (constraint clearance (min NaNmm)))",
            "(rule \"X\" (constraint clearance (min garbagemm)))",
            "(rule \"X\" (constraint clearance 0.2mm))",
            "(rule \"X\" (constraint clearance (min 100.1mm)))",
        ] {
            assert!(
                routing_bounds(&format!("(version 1){statement}")).is_err(),
                "{statement}"
            );
        }
    }
    #[test]
    fn routing_projection_rounds_minima_up_and_refuses_nonrepresentable_rules() {
        let src="(version 1)(rule \"X\" (constraint clearance (min 0.200001mm)) (constraint track_width (min 0.300001mm)))";
        let b = routing_bounds(src).unwrap();
        assert_eq!(b.clearance_mm, 0.201);
        assert_eq!(b.width_mm, 0.301);
        for rule in [
            FIRST,
            "(rule \"X\" (constraint clearance (max 0.2mm)))",
            "(rule \"X\" (constraint clearance (min 0.1mm + 0.1mm)))",
            "(rule \"X\" (constraint annular_width (min 0.2mm)))",
            "(rule \"X\" (constraint clearance (min -1mm)))",
        ] {
            assert!(
                routing_bounds(&format!("(version 1){rule}")).is_err(),
                "{rule}"
            );
        }
    }
}
