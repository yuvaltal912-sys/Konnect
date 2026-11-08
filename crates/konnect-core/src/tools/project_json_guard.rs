//! Reject duplicate JSON keys before a whole-project rewrite.
use serde::de::{Deserialize, Deserializer, Error, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt;
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(UniqueVisitor)
    }
}
struct UniqueVisitor;
impl<'de> Visitor<'de> for UniqueVisitor {
    type Value = Unique;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JSON without duplicate object keys")
    }
    fn visit_bool<E: Error>(self, v: bool) -> Result<Unique, E> {
        Ok(Unique(Value::Bool(v)))
    }
    fn visit_i64<E: Error>(self, v: i64) -> Result<Unique, E> {
        Ok(Unique(Value::Number(v.into())))
    }
    fn visit_u64<E: Error>(self, v: u64) -> Result<Unique, E> {
        Ok(Unique(Value::Number(v.into())))
    }
    fn visit_f64<E: Error>(self, v: f64) -> Result<Unique, E> {
        Ok(Unique(Value::Number(
            Number::from_f64(v).ok_or_else(|| E::custom("non-finite JSON number"))?,
        )))
    }
    fn visit_str<E: Error>(self, v: &str) -> Result<Unique, E> {
        Ok(Unique(Value::String(v.into())))
    }
    fn visit_string<E: Error>(self, v: String) -> Result<Unique, E> {
        Ok(Unique(Value::String(v)))
    }
    fn visit_none<E: Error>(self) -> Result<Unique, E> {
        Ok(Unique(Value::Null))
    }
    fn visit_unit<E: Error>(self) -> Result<Unique, E> {
        Ok(Unique(Value::Null))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Unique, A::Error> {
        let mut out = Vec::new();
        while let Some(v) = sequence.next_element::<Unique>()? {
            out.push(v.0);
        }
        Ok(Unique(Value::Array(out)))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Unique, A::Error> {
        let mut out = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if out.contains_key(&key) {
                return Err(A::Error::custom(format!(
                    "duplicate project JSON key: {key}"
                )));
            }
            let value = map.next_value::<Unique>()?;
            out.insert(key, value.0);
        }
        Ok(Unique(Value::Object(out)))
    }
}
pub(crate) fn parse(source: &str) -> anyhow::Result<Value> {
    let mut deserializer = serde_json::Deserializer::from_str(source);
    let value = Unique::deserialize(&mut deserializer)?.0;
    deserializer.end()?;
    anyhow::ensure!(value.is_object(), "project root must be an object");
    Ok(value)
}

/// Read a saved project without treating missing, unreadable or linked files as defaults.
pub(crate) fn project_source(path: &std::path::Path) -> anyhow::Result<String> {
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.file_type().is_file(),
        "project must be a regular file, not a symlink or directory"
    );
    Ok(konnect_sexp::writer::read_consistent(path)?)
}
pub(super) fn write_verified(
    path: &std::path::Path,
    before: &str,
    after: &str,
) -> anyhow::Result<()> {
    project_source(path)?;
    konnect_sexp::writer::write_atomic_if_unchanged(path, before, after)?;
    anyhow::ensure!(
        project_source(path)? == after,
        "saved project differs from the planned bytes"
    );
    Ok(())
}
pub(super) fn uncertain(
    path: &std::path::Path,
    operation: &str,
    reason: String,
) -> crate::mcp::protocol::CallToolResult {
    crate::mcp::protocol::CallToolResult::error_kind(crate::mcp::error::ToolErrorKind::MutationOutcomeUncertain{operation:operation.into(),path:path.display().to_string(),reason:reason.clone()},format!("The project write could not be verified: {reason}. Inspect the saved project before retrying."))
}
/// A saved project may also be owned by its root schematic editor.
pub(super) fn schematic_lock_refusal(
    board: &std::path::Path,
) -> Option<crate::mcp::protocol::CallToolResult> {
    let schematic = board.with_extension("kicad_sch");
    let lock = konnect_sexp::writer::kicad_editor_lock_path(&schematic)?;
    match std::fs::symlink_metadata(&lock) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Ok(_) => Some(crate::mcp::protocol::CallToolResult::error(format!(
            "KiCad schematic lock exists at {}; close it before editing project settings",
            lock.display()
        ))),
        Err(error) => Some(crate::mcp::protocol::CallToolResult::error(format!(
            "cannot inspect schematic lock {}: {error}",
            lock.display()
        ))),
    }
}
/// Supported project-level minima from KiCad 10 board_design_settings.cpp.
pub(super) fn minima(args: &Value) -> anyhow::Result<Vec<(&'static str, f64)>> {
    let mut out = Vec::new();
    for (key, lo, hi) in [
        ("min_copper_edge_clearance", -0.01, 25.0),
        ("min_silk_clearance", -10.0, 100.0),
        ("min_via_annular_width", 0.0, 25.0),
    ] {
        if let Some(raw) = args.get(key) {
            let value = raw
                .as_f64()
                .ok_or_else(|| anyhow::anyhow!("{key} must be numeric"))?;
            anyhow::ensure!(
                value.is_finite() && (lo..=hi).contains(&value),
                "{key} is outside KiCad's supported range"
            );
            anyhow::ensure!(
                (value * 1_000_000.0 - (value * 1_000_000.0).round()).abs() < 0.000001
                    && (value == 0.0 || value.abs() >= 0.000001),
                "{key} must be a whole number of nanometers"
            );
            out.push((key, (value * 1_000_000.0).round() / 1_000_000.0));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_keys_are_refused_at_every_depth() {
        assert!(parse(r#"{"board":{},"board":{}}"#).is_err());
        assert!(parse(r#"{"board":{"rules":{"a":1,"a":2}}}"#).is_err());
        assert!(parse(r#"{"array":[{"x":1,"x":2}]}"#).is_err());
        assert!(parse("[]").is_err());
        assert!(parse("{} garbage").is_err());
        assert_eq!(
            parse(r#"{"a":1,"v":[null,true,1.25,"x"]}"#).unwrap(),
            serde_json::json!({"a":1,"v":[null,true,1.25,"x"]})
        );
    }
    #[test]
    fn minima_validate_types_ranges_and_nanometer_precision() {
        assert_eq!(minima(&serde_json::json!({"min_copper_edge_clearance":-0.01,"min_silk_clearance":-10.0,"min_via_annular_width":0})).unwrap().len(),3);
        for (key, value) in [
            ("min_copper_edge_clearance", serde_json::json!(-0.02)),
            ("min_copper_edge_clearance", serde_json::json!(25.1)),
            ("min_silk_clearance", serde_json::json!(-10.1)),
            ("min_silk_clearance", serde_json::json!(100.1)),
            ("min_via_annular_width", serde_json::json!(-0.1)),
            ("min_via_annular_width", serde_json::json!(25.1)),
            ("min_via_annular_width", serde_json::json!(0.0000011)),
            ("min_via_annular_width", serde_json::json!(null)),
        ] {
            let mut args = serde_json::json!({});
            args[key] = value;
            assert!(minima(&args).is_err(), "{args}");
        }
    }
    #[test]
    fn conditional_project_write_does_not_clobber_concurrent_bytes() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("board.kicad_pro");
        std::fs::write(&p, "old").unwrap();
        std::fs::write(&p, "concurrent").unwrap();
        assert!(write_verified(&p, "old", "new").is_err());
        assert_eq!(std::fs::read_to_string(p).unwrap(), "concurrent");
    }
    #[cfg(unix)]
    #[test]
    fn project_symlinks_and_directories_refuse() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("target");
        let link = d.path().join("link");
        std::fs::write(&target, "{}\n").unwrap();
        symlink(&target, &link).unwrap();
        assert!(project_source(&link).is_err());
        assert!(project_source(d.path()).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "{}\n");
    }
    #[test]
    fn a_real_kicad_project_round_trips_semantically() {
        let source = include_str!("../../tests/fixtures/multichannel_mixer.kicad_pro");
        let value = parse(source).unwrap();
        let baseline: Value = serde_json::from_str(source).unwrap();
        assert_eq!(value, baseline);
    }
}
