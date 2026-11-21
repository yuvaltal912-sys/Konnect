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
            json!([{ "name": " A", "nets": ["N"], "voltage_v": 1 }]),
            "leading or trailing whitespace",
        ),
        (
            "rails",
            json!([{ "name": "A", "nets": ["  "], "voltage_v": 1 }]),
            "is only whitespace",
        ),
        (
            "currents",
            json!([{ "net": " ", "max_current_a": 1 }]),
            "is only whitespace",
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

/// KiCad keeps a net name's outer spaces, so a recorded net may carry them: a
/// board can have `'+5V '` next to `'+5V'`, and the record must be able to name
/// either. The name is stored and read back byte for byte. Negative control: a
/// rail *name* (not a net) with an outer space is still refused.
#[test]
fn a_net_name_with_outer_spaces_is_kept_as_written() {
    let dir = project();
    let currents =
        json!([{ "net": "+5V ", "max_current_a": 2.0 }, { "net": " SD", "max_current_a": 1.0 }]);
    let result = set(dir.path(), "currents", currents, None);
    assert!(!result.is_error, "{:?}", body(&result));
    let read = body(&get_design_intent_at(dir.path()));
    assert_eq!(read["intent"]["currents"][0]["net"], "+5V ", "{read}");
    assert_eq!(read["intent"]["currents"][1]["net"], " SD", "{read}");
    // A net may sit in a rail or in 'currents', not both, so the rail names another.
    let rail = json!([{ "name": "12V", "nets": ["+12V "], "voltage_v": 12.0 }]);
    let revision = current_revision(dir.path());
    let set_rail = set(dir.path(), "rails", rail, Some(&revision));
    assert!(
        !set_rail.is_error,
        "a rail net may carry outer spaces too: {:?}",
        body(&set_rail)
    );
    let read = body(&get_design_intent_at(dir.path()));
    assert_eq!(read["intent"]["rails"][0]["nets"][0], "+12V ", "{read}");
    let interface = json!([{ "name": "I2C", "kind": "i2c", "nets": [" SDA", "SCL "] }]);
    let revision = current_revision(dir.path());
    let set_interface = set(dir.path(), "interfaces", interface, Some(&revision));
    assert!(
        !set_interface.is_error,
        "an interface net too: {:?}",
        body(&set_interface)
    );
    let read = body(&get_design_intent_at(dir.path()));
    assert_eq!(
        read["intent"]["interfaces"][0]["nets"],
        json!([" SDA", "SCL "]),
        "{read}"
    );
    let other = project();
    let bad = set(
        other.path(),
        "rails",
        json!([{ "name": "5V ", "nets": ["N"], "voltage_v": 5.0 }]),
        None,
    );
    assert!(
        bad.is_error,
        "control: a rail name with an outer space is refused"
    );
    assert!(file_text(other.path()).is_none(), "and nothing is written");
}

// ─── Row LT: length groups and pair skews ────────────────────────────────────

/// One interface of six nets, with the given length groups and pairs.
fn ram(groups: Value, pairs: Value) -> Value {
    json!([{
        "name": "ram", "kind": "ddr",
        "nets": ["D0", "D1", "D2", "D3", "DQS", "DQSN"],
        "differential_pairs": pairs,
        "length_groups": groups
    }])
}

fn all_nets() -> Value {
    json!(["D0", "D1", "D2", "D3", "DQS", "DQSN"])
}

fn good_pairs() -> Value {
    json!([{ "positive": "DQS", "negative": "DQSN", "max_skew_mm": 0.1 }])
}

fn good_groups() -> Value {
    json!([{ "name": "g", "nets": all_nets(), "tolerance_mm": 0.5 }])
}

/// The reason a refused interfaces section gives.
fn refusal_reason(value: Value) -> Option<String> {
    let dir = project();
    let result = set(dir.path(), "interfaces", value, None);
    if !result.is_error {
        return None;
    }
    assert_eq!(kind(&result).as_deref(), Some("invalid_argument"));
    assert!(file_text(dir.path()).is_none(), "a refusal wrote the file");
    Some(
        body(&result)["error"]["reason"]
            .as_str()
            .unwrap()
            .to_string(),
    )
}

/// A pre-Row-LT record, in the exact text the writer produces: it has none of the new
/// keys.
const OLD_RECORD: &str = r#"{
  "schema_version": 1,
  "rails": [],
  "currents": [],
  "interfaces": [
    {
      "name": "USB",
      "kind": "usb2",
      "nets": [
        "USB_DP",
        "USB_DN"
      ],
      "differential_pairs": [
        {
          "positive": "USB_DP",
          "negative": "USB_DN"
        }
      ],
      "differential_impedance_ohm": 90.0
    }
  ]
}
"#;

fn read_text(text: &str) -> IntentRead {
    let dir = project();
    let path = design_intent_path(dir.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    read_design_intent(&path)
}

#[test]
fn a_record_without_the_length_fields_reads_and_writes_back_byte_for_byte() {
    let IntentRead::Loaded { intent, text } = read_text(OLD_RECORD) else {
        panic!("a record from before Row LT must load")
    };
    assert_eq!(text, OLD_RECORD);
    assert!(intent.interfaces[0].length_groups.is_empty());
    assert_eq!(intent.interfaces[0].differential_pairs[0].max_skew_mm, None);
    let written = serde_json::to_string_pretty(&*intent).unwrap() + "\n";
    assert_eq!(written, OLD_RECORD, "the new fields must not appear unset");
    assert!(!written.contains("length_groups") && !written.contains("max_skew_mm"));
}

#[test]
fn a_record_with_the_length_fields_round_trips() {
    // The text is the typed record's own, so key order is the struct's.
    let typed: DesignIntent = serde_json::from_value(json!({
        "schema_version": 1,
        "rails": [],
        "currents": [],
        "interfaces": ram(
            json!([
                { "name": "low", "nets": ["D0", "D1"], "tolerance_mm": 0.5 },
                { "name": "high", "nets": ["D2", "D3", "DQS", "DQSN"], "tolerance_mm": 0.25,
                  "target_mm": 40.0, "notes": "byte lane" }
            ]),
            good_pairs()
        )
    }))
    .unwrap();
    let text = serde_json::to_string_pretty(&typed).unwrap() + "\n";
    let IntentRead::Loaded { intent, text: read } = read_text(&text) else {
        panic!("a valid record with length groups must load")
    };
    let group = &intent.interfaces[0].length_groups[1];
    assert_eq!(group.target_mm, Some(40.0));
    assert_eq!(group.notes.as_deref(), Some("byte lane"));
    assert_eq!(
        intent.interfaces[0].differential_pairs[0].max_skew_mm,
        Some(0.1)
    );
    let written = serde_json::to_string_pretty(&*intent).unwrap() + "\n";
    assert_eq!(written, read, "the record writes back as it was read");

    // The same record through the tool reads back too.
    let dir = project();
    let value = ram(
        json!([{ "name": "g", "nets": all_nets(), "tolerance_mm": 0.5, "target_mm": 30 }]),
        good_pairs(),
    );
    let result = set(dir.path(), "interfaces", value, None);
    assert!(!result.is_error, "{:?}", body(&result));
    let read = body(&get_design_intent_at(dir.path()));
    assert_eq!(
        read["intent"]["interfaces"][0]["length_groups"][0]["target_mm"], 30.0,
        "{read}"
    );
}

#[test]
fn an_unknown_key_inside_a_length_group_or_a_pair_is_refused() {
    // Negative control of the schema row: the new structs are closed.
    for value in [
        ram(
            json!([{ "name": "g", "nets": all_nets(), "tolerance_mm": 0.5, "slack_mm": 1 }]),
            good_pairs(),
        ),
        ram(
            good_groups(),
            json!([{ "positive": "DQS", "negative": "DQSN", "max_skew": 0.1 }]),
        ),
    ] {
        let reason =
            refusal_reason(value.clone()).unwrap_or_else(|| panic!("{value} was accepted"));
        assert!(reason.contains("unknown field"), "{reason}");
    }
    // The control: the record without the stray key is accepted.
    assert!(refusal_reason(ram(good_groups(), good_pairs())).is_none());

    // A file with a stray key inside a group is unusable, and a read says so.
    let text = serde_json::to_string(&json!({
        "schema_version": 1,
        "interfaces": ram(
            json!([{ "name": "g", "nets": all_nets(), "tolerance_mm": 0.5, "slack_mm": 1 }]),
            json!([])
        )
    }))
    .unwrap();
    assert!(matches!(read_text(&text), IntentRead::Unusable { .. }));
}

/// A tolerance of zero is reported once, on the tolerance: the target is not also
/// blamed for not exceeding a tolerance that is already refused.
#[test]
fn a_tolerance_that_is_refused_is_the_only_problem_of_its_group() {
    for tolerance in [json!(0), json!(-0.5)] {
        for target in [json!(0), json!(-3), json!(0.5)] {
            let reason = refusal_reason(ram(
                json!([{ "name": "g", "nets": ["D0", "D1"], "tolerance_mm": tolerance,
                         "target_mm": target }]),
                json!([]),
            ))
            .expect("a tolerance that is not positive is refused");
            assert!(
                reason.contains("length_groups[0].tolerance_mm"),
                "{tolerance} {target}: {reason}"
            );
            assert!(
                !reason.contains("greater than tolerance_mm"),
                "{tolerance} {target}: {reason}"
            );
        }
    }
}

/// Each length rule of L1a refuses with the path of what is wrong, and the same
/// record with the rule met is accepted.
#[test]
fn every_length_rule_refuses_with_its_path_and_accepts_when_met() {
    let all = all_nets();
    // (bad groups, bad pairs, expected text, good groups, good pairs)
    let cases: Vec<(Value, Value, &str, Value, Value)> = vec![
        (
            json!([{ "name": "g", "nets": ["D0", "D1", "X"], "tolerance_mm": 0.5 }]),
            json!([]),
            "interfaces[0].length_groups[0].nets[2]: 'X' is not one of this interface's nets",
            json!([{ "name": "g", "nets": ["D0", "D1", "D2"], "tolerance_mm": 0.5 }]),
            json!([]),
        ),
        (
            json!([{ "name": "a", "nets": ["D0", "D1"], "tolerance_mm": 0.5 },
                   { "name": "b", "nets": ["D1", "D2"], "tolerance_mm": 0.5 }]),
            json!([]),
            "interfaces[0].length_groups[1].nets[0]: 'D1' is already in length group \
             interfaces[0].length_groups[0]",
            json!([{ "name": "a", "nets": ["D0", "D1"], "tolerance_mm": 0.5 },
                   { "name": "b", "nets": ["D2", "D3"], "tolerance_mm": 0.5 }]),
            json!([]),
        ),
        (
            json!([{ "name": "g", "nets": ["D0", "D0"], "tolerance_mm": 0.5 }]),
            json!([]),
            "interfaces[0].length_groups[0].nets[1]: 'D0' is listed twice in this group",
            json!([{ "name": "g", "nets": ["D0", "D1"], "tolerance_mm": 0.5 }]),
            json!([]),
        ),
        (
            json!([{ "name": "g", "nets": ["D0"], "tolerance_mm": 0.5 }]),
            json!([]),
            "interfaces[0].length_groups[0].nets: a group without target_mm",
            json!([{ "name": "g", "nets": ["D0"], "tolerance_mm": 0.5, "target_mm": 30 }]),
            json!([]),
        ),
        (
            json!([{ "name": "g", "nets": [], "tolerance_mm": 0.5, "target_mm": 30 }]),
            json!([]),
            "interfaces[0].length_groups[0].nets: a length group needs at least one net",
            json!([{ "name": "g", "nets": ["D0"], "tolerance_mm": 0.5, "target_mm": 30 }]),
            json!([]),
        ),
        (
            json!([{ "name": "g", "nets": ["D0", "D1"], "tolerance_mm": 0 }]),
            json!([]),
            "interfaces[0].length_groups[0].tolerance_mm: 0 must be greater than zero",
            json!([{ "name": "g", "nets": ["D0", "D1"], "tolerance_mm": 0.01 }]),
            json!([]),
        ),
        (
            json!([{ "name": "g", "nets": ["D0", "D1"], "tolerance_mm": -0.5 }]),
            json!([]),
            "interfaces[0].length_groups[0].tolerance_mm: -0.5 must be greater than zero",
            json!([{ "name": "g", "nets": ["D0", "D1"], "tolerance_mm": 0.5 }]),
            json!([]),
        ),
        (
            json!([{ "name": "g", "nets": ["D0", "D1"], "tolerance_mm": 0.5, "target_mm": 0.5 }]),
            json!([]),
            "interfaces[0].length_groups[0].target_mm: 0.5 must be greater than tolerance_mm 0.5",
            json!([{ "name": "g", "nets": ["D0", "D1"], "tolerance_mm": 0.5, "target_mm": 0.6 }]),
            json!([]),
        ),
        (
            json!([{ "name": "g", "nets": ["D0", "D1"], "tolerance_mm": 0.5, "target_mm": -3 }]),
            json!([]),
            "interfaces[0].length_groups[0].target_mm",
            json!([{ "name": "g", "nets": ["D0", "D1"], "tolerance_mm": 0.5, "target_mm": 3 }]),
            json!([]),
        ),
        (
            json!([{ "name": "a", "nets": ["D0", "D1"], "tolerance_mm": 0.5 },
                   { "name": "a", "nets": ["D2", "D3"], "tolerance_mm": 0.5 }]),
            json!([]),
            "interfaces[0].length_groups[1].name: 'a' is used twice",
            json!([{ "name": "a", "nets": ["D0", "D1"], "tolerance_mm": 0.5 },
                   { "name": "b", "nets": ["D2", "D3"], "tolerance_mm": 0.5 }]),
            json!([]),
        ),
        (
            json!([{ "name": " g", "nets": ["D0", "D1"], "tolerance_mm": 0.5 }]),
            json!([]),
            "interfaces[0].length_groups[0].name: ' g' has leading or trailing whitespace",
            json!([{ "name": "g", "nets": ["D0", "D1"], "tolerance_mm": 0.5 }]),
            json!([]),
        ),
        (
            json!([{ "name": "g", "nets": ["D0", " "], "tolerance_mm": 0.5 }]),
            json!([]),
            "interfaces[0].length_groups[0].nets[1]: ' ' is only whitespace",
            json!([{ "name": "g", "nets": ["D0", "D1"], "tolerance_mm": 0.5 }]),
            json!([]),
        ),
        // A pair split across two groups.
        (
            json!([{ "name": "a", "nets": ["D0", "DQS"], "tolerance_mm": 0.5 },
                   { "name": "b", "nets": ["D1", "DQSN"], "tolerance_mm": 0.5 }]),
            good_pairs(),
            "interfaces[0].differential_pairs[0]: 'DQS' and 'DQSN' must be in the same length group",
            json!([{ "name": "a", "nets": ["D0", "DQS", "DQSN"], "tolerance_mm": 0.5 },
                   { "name": "b", "nets": ["D1", "D2"], "tolerance_mm": 0.5 }]),
            good_pairs(),
        ),
        // A pair with one net in a group and the other in none.
        (
            json!([{ "name": "a", "nets": ["D0", "DQS"], "tolerance_mm": 0.5 }]),
            good_pairs(),
            "interfaces[0].differential_pairs[0]: 'DQS' and 'DQSN' must be in the same length group",
            json!([{ "name": "a", "nets": ["D0", "DQS", "DQSN"], "tolerance_mm": 0.5 }]),
            good_pairs(),
        ),
        // A pair in no group stays allowed.
        (
            json!([{ "name": "a", "nets": ["D0", "DQSN"], "tolerance_mm": 0.5 }]),
            good_pairs(),
            "must be in the same length group",
            json!([{ "name": "a", "nets": ["D0", "D1"], "tolerance_mm": 0.5 }]),
            good_pairs(),
        ),
        (
            json!([]),
            json!([{ "positive": "DQS", "negative": "DQSN", "max_skew_mm": 0 }]),
            "interfaces[0].differential_pairs[0].max_skew_mm: 0 must be greater than zero",
            json!([]),
            json!([{ "positive": "DQS", "negative": "DQSN", "max_skew_mm": 0.001 }]),
        ),
        (
            json!([]),
            json!([{ "positive": "DQS", "negative": "DQSN", "max_skew_mm": -1 }]),
            "interfaces[0].differential_pairs[0].max_skew_mm: -1 must be greater than zero",
            json!([]),
            json!([{ "positive": "DQS", "negative": "DQSN" }]),
        ),
    ];
    for (bad_groups, bad_pairs, expected, good_groups, good_pairs) in cases {
        let bad = ram(bad_groups, bad_pairs);
        let reason = refusal_reason(bad.clone())
            .unwrap_or_else(|| panic!("{bad} was accepted, wanted '{expected}'"));
        assert!(
            reason.contains(expected),
            "{bad}: '{reason}' lacks '{expected}'"
        );
        let good = ram(good_groups, good_pairs);
        assert!(
            refusal_reason(good.clone()).is_none(),
            "control: {good} must be accepted"
        );
    }
    // A net listed in a group of another interface is refused too (one group per net).
    let two = json!([
        { "name": "a", "kind": "x", "nets": ["N0", "N1"],
          "length_groups": [{ "name": "g", "nets": ["N0", "N1"], "tolerance_mm": 0.5 }] },
        { "name": "b", "kind": "x", "nets": ["N1", "N2"],
          "length_groups": [{ "name": "g", "nets": ["N1", "N2"], "tolerance_mm": 0.5 }] }
    ]);
    let reason = refusal_reason(two).expect("a net in two groups of two interfaces");
    assert!(
        reason.contains(
            "interfaces[1].length_groups[0].nets[0]: 'N1' is already in length group \
             interfaces[0].length_groups[0]"
        ),
        "{reason}"
    );
    let _ = all;
}

#[test]
fn the_set_design_intent_description_names_the_two_new_fields() {
    let tools = tools();
    let tool = tools
        .iter()
        .find(|tool| tool.name == "set_design_intent")
        .expect("set_design_intent is registered");
    assert!(tool.description.contains("length_groups"));
    assert!(tool.description.contains("max_skew_mm"));
}

#[test]
fn non_finite_lengths_are_refused_by_the_validator() {
    // JSON cannot carry NaN or infinity, so the typed record is built by hand.
    let mut intent: DesignIntent = serde_json::from_value(json!({
        "schema_version": 1,
        "interfaces": ram(good_groups(), good_pairs())
    }))
    .unwrap();
    assert!(
        validate_lengths(&intent).is_empty(),
        "control: the record is valid"
    );
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut broken = intent.clone();
        broken.interfaces[0].length_groups[0].tolerance_mm = value;
        let problems = validate_lengths(&broken);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("length_groups[0].tolerance_mm")),
            "tolerance {value}: {problems:?}"
        );
        let mut broken = intent.clone();
        broken.interfaces[0].length_groups[0].target_mm = Some(value);
        let problems = validate_lengths(&broken);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("length_groups[0].target_mm")),
            "target {value}: {problems:?}"
        );
        let mut broken = intent.clone();
        broken.interfaces[0].differential_pairs[0].max_skew_mm = Some(value);
        let problems = validate_lengths(&broken);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("differential_pairs[0].max_skew_mm")),
            "skew {value}: {problems:?}"
        );
    }
    intent.interfaces[0].length_groups[0].target_mm = Some(f64::NAN);
    assert!(
        !validate(&intent).is_empty(),
        "validate includes the length rules"
    );
}
