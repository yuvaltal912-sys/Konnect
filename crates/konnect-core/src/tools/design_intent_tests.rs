use super::*;
use crate::mcp::error::extract_error_kind;
use crate::mcp::protocol::ToolContent;

fn body(result: &CallToolResult) -> Value {
    match result.content.first() {
        Some(ToolContent::Text { text }) => serde_json::from_str(text).unwrap(),
        other => panic!("expected text content, got {other:?}"),
    }
}

fn kind(result: &CallToolResult) -> Option<String> {
    extract_error_kind(result)
}

/// A temporary KiCad project directory.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("board.kicad_pro"), "{}").unwrap();
    dir
}

fn rails() -> Value {
    json!([
        { "name": "3V3", "nets": ["+3V3"], "voltage_v": 3.3, "max_current_a": 0.5,
          "tolerance_pct": 5, "source_ref": "U2" },
        { "name": "VBUS", "nets": ["VBUS", "VBUS_F"], "voltage_v": 5.0, "max_current_a": 2.0 }
    ])
}

fn set(dir: &Path, section: &str, value: Value, revision: Option<&Value>) -> CallToolResult {
    let mut args = json!({ "section": section, "value": value });
    if let Some(revision) = revision {
        args["expected_revision"] = revision.clone();
    }
    set_design_intent_at(dir, &args)
}

fn current_revision(dir: &Path) -> Value {
    body(&get_design_intent_at(dir))["revision"].clone()
}

fn file_text(dir: &Path) -> Option<String> {
    std::fs::read_to_string(design_intent_path(dir)).ok()
}

#[test]
fn an_absent_record_reads_as_absent_with_every_section_missing() {
    let dir = project();
    let result = get_design_intent_at(dir.path());
    assert!(!result.is_error);
    let body = body(&result);
    assert_eq!(body["source"], "absent", "{body}");
    assert!(body["revision"].is_null(), "{body}");
    assert!(body["intent"].is_null(), "{body}");
    assert_eq!(body["missing_sections"], json!(SECTIONS), "{body}");
    assert_eq!(body["net_names_checked"], false, "{body}");
    assert!(file_text(dir.path()).is_none(), "a read writes nothing");
}

#[test]
fn a_first_write_creates_the_record_and_reads_back() {
    let dir = project();
    let result = set(dir.path(), "rails", rails(), None);
    assert!(!result.is_error, "{:?}", body(&result));
    let written = body(&result);
    assert_eq!(written["status"], "applied", "{written}");
    assert_eq!(written["created"], true, "{written}");
    assert!(written["previous_revision"].is_null(), "{written}");

    let read = body(&get_design_intent_at(dir.path()));
    assert_eq!(read["source"], "file", "{read}");
    assert_eq!(read["revision"], written["revision"], "{read}");
    assert_eq!(read["intent"]["rails"][0]["source_ref"], "U2", "{read}");
    assert_eq!(read["intent"]["schema_version"], SCHEMA_VERSION, "{read}");
    assert_eq!(
        read["missing_sections"],
        json!(["currents", "interfaces", "fab_target"]),
        "{read}"
    );
    assert_eq!(
        read["revision"].as_str().unwrap(),
        revision_of(&file_text(dir.path()).unwrap()),
        "the revision is the hash of the file as written"
    );
}

#[test]
fn setting_one_section_keeps_the_others() {
    let dir = project();
    assert!(!set(dir.path(), "rails", rails(), None).is_error);
    let revision = current_revision(dir.path());
    let result = set(
        dir.path(),
        "fab_target",
        json!({ "house": "JLCPCB", "layer_count": 4, "outer_copper_oz": 1, "inner_copper_oz": 0.5 }),
        Some(&revision),
    );
    assert!(!result.is_error, "{:?}", body(&result));
    let read = body(&get_design_intent_at(dir.path()));
    assert_eq!(
        read["intent"]["rails"].as_array().unwrap().len(),
        2,
        "{read}"
    );
    assert_eq!(read["intent"]["fab_target"]["layer_count"], 4, "{read}");
    assert_eq!(body(&result)["previous_revision"], revision);
}

#[test]
fn a_write_over_an_existing_record_needs_its_current_revision() {
    let dir = project();
    assert!(!set(dir.path(), "rails", rails(), None).is_error);
    let before = file_text(dir.path()).unwrap();
    let currents = json!([{ "net": "MOTOR_A", "max_current_a": 1.5 }]);

    let missing = set(dir.path(), "currents", currents.clone(), None);
    assert_eq!(kind(&missing).as_deref(), Some("stale_target"));

    let stale = set(
        dir.path(),
        "currents",
        currents.clone(),
        Some(&json!("sha256:00")),
    );
    assert_eq!(kind(&stale).as_deref(), Some("stale_target"));
    assert_eq!(
        file_text(dir.path()).unwrap(),
        before,
        "a refusal writes nothing"
    );

    // A revision for a record that does not exist is also stale.
    let empty = project();
    let phantom = set(
        empty.path(),
        "currents",
        currents,
        Some(&json!("sha256:00")),
    );
    assert_eq!(kind(&phantom).as_deref(), Some("stale_target"));
    assert!(file_text(empty.path()).is_none());
}

#[test]
fn a_dry_run_validates_and_writes_nothing() {
    let dir = project();
    let args = json!({ "section": "rails", "value": rails(), "dry_run": true });
    let preview = body(&set_design_intent_at(dir.path(), &args));
    assert_eq!(preview["status"], "preview", "{preview}");
    assert_eq!(preview["written"], false, "{preview}");
    assert!(preview["candidate_revision"].is_string(), "{preview}");
    assert!(file_text(dir.path()).is_none());

    assert!(!set(dir.path(), "rails", rails(), None).is_error);
    assert_eq!(
        current_revision(dir.path()),
        preview["candidate_revision"],
        "the preview names the revision the write produces"
    );
}

#[test]
fn an_identical_write_reports_unchanged_and_leaves_the_file() {
    let dir = project();
    assert!(!set(dir.path(), "rails", rails(), None).is_error);
    let revision = current_revision(dir.path());
    let before = file_text(dir.path()).unwrap();
    let again = body(&set(dir.path(), "rails", rails(), Some(&revision)));
    assert_eq!(again["status"], "unchanged", "{again}");
    assert_eq!(again["revision"], revision, "{again}");
    assert_eq!(file_text(dir.path()).unwrap(), before);
}

#[test]
fn sections_clear_with_an_empty_list_or_null() {
    let dir = project();
    assert!(!set(dir.path(), "rails", rails(), None).is_error);
    let r = current_revision(dir.path());
    assert!(
        !set(
            dir.path(),
            "fab_target",
            json!({ "layer_count": 2 }),
            Some(&r)
        )
        .is_error
    );

    let r = current_revision(dir.path());
    assert!(!set(dir.path(), "fab_target", Value::Null, Some(&r)).is_error);
    let r = current_revision(dir.path());
    assert!(!set(dir.path(), "rails", json!([]), Some(&r)).is_error);

    let read = body(&get_design_intent_at(dir.path()));
    assert_eq!(read["missing_sections"], json!(SECTIONS), "{read}");
    assert!(read["intent"].get("fab_target").is_none(), "{read}");
}

/// Each invalid section is refused with a reason that names where it is, and
/// nothing reaches the disk.
#[test]
fn invalid_sections_are_refused_with_their_location() {
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "rails",
            json!([{ "name": "A", "nets": ["N"], "voltage_v": 1 },
                   { "name": "A", "nets": ["M"], "voltage_v": 1 }]),
            "rails[1].name: 'A' is used twice",
        ),
        (
            "rails",
            json!([{ "name": "A", "nets": ["N"], "voltage_v": 1 },
                   { "name": "B", "nets": ["N"], "voltage_v": 2 }]),
            "rails[1].nets[0]: 'N' already belongs to a rail",
        ),
        (
            "rails",
            json!([{ "name": "A", "nets": [], "voltage_v": 1 }]),
            "rails[0].nets: a rail needs at least one net",
        ),
        (
            "rails",
            json!([{ "name": "A", "nets": ["N"], "voltage_v": 1, "max_current_a": -1 }]),
            "rails[0].max_current_a",
        ),
        (
            "rails",
            json!([{ "name": "A", "nets": [" N"], "voltage_v": 1 }]),
            "leading or trailing whitespace",
        ),
        (
            "rails",
            json!([{ "name": "A", "nets": ["N"], "voltage_v": 1, "tolerance_pct": 150 }]),
            "rails[0].tolerance_pct",
        ),
        (
            "rails",
            json!([{ "name": "A", "nets": ["N"], "voltage_v": 1, "volts": 3 }]),
            "unknown field `volts`",
        ),
        (
            "rails",
            json!([{ "name": "A", "nets": ["N"] }]),
            "missing field `voltage_v`",
        ),
        (
            "currents",
            json!([{ "net": "X", "max_current_a": 0 }]),
            "currents[0].max_current_a",
        ),
        (
            "currents",
            json!([{ "net": "X", "max_current_a": 1 }, { "net": "X", "max_current_a": 2 }]),
            "currents[1].net: 'X' is listed twice",
        ),
        (
            "interfaces",
            json!([{ "name": "USB", "kind": "usb2", "nets": ["DP"],
                     "differential_pairs": [{ "positive": "DP", "negative": "DN" }] }]),
            "'DN' is not one of this interface's nets",
        ),
        (
            "interfaces",
            json!([{ "name": "USB", "kind": "usb2", "nets": ["DP", "DN"],
                     "differential_impedance_ohm": 90 }]),
            "has no differential_pairs",
        ),
        (
            "interfaces",
            json!([{ "name": "USB", "kind": "usb2", "nets": ["DP", "DN"],
                     "differential_pairs": [{ "positive": "DP", "negative": "DP" }] }]),
            "positive and negative are both 'DP'",
        ),
        (
            "fab_target",
            json!({ "layer_count": 3 }),
            "fab_target.layer_count: 3",
        ),
        (
            "fab_target",
            json!({ "layer_count": 2, "inner_copper_oz": 0.5 }),
            "a 2-layer board has no inner layers",
        ),
        (
            "fab_target",
            json!({ "min_via_drill_mm": 0.3, "min_via_diameter_mm": 0.3 }),
            "is not larger than min_via_drill_mm",
        ),
        ("fab_target", json!([]), "invalid type"),
        ("rails", Value::Null, "invalid type"),
        ("rails", json!([["A", ["N"], 1.0]]), "invalid type"),
    ];
    for (section, value, expected) in cases {
        let dir = project();
        let result = set(dir.path(), section, value.clone(), None);
        assert!(result.is_error, "{section} {value} was accepted");
        assert_eq!(
            kind(&result).as_deref(),
            Some("invalid_argument"),
            "{value}"
        );
        let reason = body(&result)["error"]["reason"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            reason.contains(expected),
            "{value}: '{reason}' lacks '{expected}'"
        );
        assert!(file_text(dir.path()).is_none(), "{value} reached the disk");
    }
}

#[test]
fn a_net_in_a_rail_cannot_also_carry_a_separate_current() {
    let dir = project();
    assert!(!set(dir.path(), "rails", rails(), None).is_error);
    let r = current_revision(dir.path());
    let result = set(
        dir.path(),
        "currents",
        json!([{ "net": "VBUS_F", "max_current_a": 3 }]),
        Some(&r),
    );
    assert_eq!(kind(&result).as_deref(), Some("invalid_argument"));
    let reason = body(&result)["error"]["reason"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(reason.contains("belongs to a rail"), "{reason}");
}

#[test]
fn an_unusable_file_is_refused_and_never_replaced() {
    let unusable = [
        r#"{"schema_version": 1, "rails": ["#,
        r#"{"schema_version": 2}"#,
        r#"{"schema_version": 1, "colour": "green"}"#,
        r#"{"schema_version": 1, "currents": [{"net": "X", "max_current_a": -2}]}"#,
    ];
    for text in unusable {
        let dir = project();
        let path = design_intent_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();

        let read = get_design_intent_at(dir.path());
        assert_eq!(
            kind(&read).as_deref(),
            Some("invalid_configuration"),
            "{text}"
        );

        let write = set(
            dir.path(),
            "rails",
            rails(),
            Some(&json!(revision_of(text))),
        );
        assert_eq!(
            kind(&write).as_deref(),
            Some("invalid_configuration"),
            "{text}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
}

#[test]
fn a_directory_without_a_kicad_project_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let result = set(dir.path(), "rails", rails(), None);
    assert_eq!(kind(&result).as_deref(), Some("invalid_argument"));
    assert!(!dir.path().join(".konnect").exists(), "no stray folder");

    let missing = dir.path().join("nowhere");
    let result = set(&missing, "rails", rails(), None);
    assert_eq!(kind(&result).as_deref(), Some("invalid_argument"));
    assert!(!missing.exists());
}

#[test]
fn a_record_that_appears_during_creation_is_a_conflict() {
    let dir = project();
    let path = design_intent_path(dir.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "someone else's record").unwrap();
    let refusal = persist(&path, None, "mine").unwrap_err();
    assert_eq!(kind(&refusal).as_deref(), Some("conflict"));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "someone else's record"
    );
}

#[test]
fn a_full_record_round_trips_through_the_typed_model() {
    let text = json!({
        "schema_version": 1,
        "rails": rails(),
        "currents": [{ "net": "MOTOR_A", "max_current_a": 1.5, "max_temp_rise_c": 10 }],
        "interfaces": [{
            "name": "USB", "kind": "usb2", "nets": ["USB_DP", "USB_DN"],
            "differential_pairs": [{ "positive": "USB_DP", "negative": "USB_DN" }],
            "differential_impedance_ohm": 90
        }],
        "fab_target": { "house": "JLCPCB", "layer_count": 2, "board_thickness_mm": 1.6,
                        "outer_copper_oz": 1, "min_track_mm": 0.127,
                        "min_via_drill_mm": 0.3, "min_via_diameter_mm": 0.6 }
    })
    .to_string();
    let IntentRead::Loaded { intent, .. } = ({
        let dir = project();
        let path = design_intent_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();
        read_design_intent(&path)
    }) else {
        panic!("a valid record must load")
    };
    assert!(intent.missing_sections().is_empty());
    assert_eq!(
        intent.interfaces[0].differential_pairs[0].negative,
        "USB_DN"
    );
}

#[test]
fn every_section_the_schema_advertises_is_handled() {
    let defs = tools();
    let set_tool = defs.iter().find(|t| t.name == "set_design_intent").unwrap();
    let advertised = set_tool.input_schema["properties"]["section"]["enum"]
        .as_array()
        .unwrap();
    for section in advertised {
        let mut intent = DesignIntent::empty();
        let empty = if section == "fab_target" {
            Value::Null
        } else {
            json!([])
        };
        assert!(
            replace_section(&mut intent, section.as_str().unwrap(), &empty).is_ok(),
            "{section}"
        );
    }
}

/// Unit calls bypass dispatch; this checks the advertised schema lets every
/// section shape through to the typed model, which is where it is judged.
#[test]
fn the_dispatch_schema_admits_every_section_shape() {
    let defs = tools();
    let set_tool = defs.iter().find(|t| t.name == "set_design_intent").unwrap();
    for (section, value) in [
        ("fab_target", json!({ "house": "JLCPCB", "layer_count": 2 })),
        ("fab_target", Value::Null),
        ("rails", rails()),
        (
            "interfaces",
            json!([{ "name": "I2C", "kind": "i2c", "nets": ["SDA", "SCL"] }]),
        ),
    ] {
        let args = json!({ "section": section, "value": value, "expected_revision": null });
        assert!(
            set_tool.input_validator.is_valid(&args),
            "dispatch refuses {section} {value}"
        );
    }
    let unknown = json!({ "section": "colour", "value": [] });
    assert!(!set_tool.input_validator.is_valid(&unknown));
}

#[test]
fn a_write_that_does_not_read_back_as_sent_is_uncertain() {
    let dir = project();
    let args = json!({ "section": "rails", "value": rails() });
    let result = set_design_intent_with(dir.path(), &args, |path, _, _| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, r#"{"schema_version": 1}"#).unwrap();
        Ok(())
    });
    assert_eq!(kind(&result).as_deref(), Some("mutation_outcome_uncertain"));
}
