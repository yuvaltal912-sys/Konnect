//! Every placement path copies the library symbol's `Sim.*` fields onto the
//! instance as hidden fields. KiCad's simulator reads the instance only, so a
//! part placed without them exports as `D1 __D1` and does not simulate.
//! `replace_component` swaps them along with the `lib_id`.

use super::*;
use crate::mcp::protocol::ToolContent;
use crate::router::ToolRouter;
use serde_json::json;
use std::sync::Arc;

/// A diode with the fields KiCad's Device library gives it (`Sim.Params`
/// carries quotes, to prove they survive), a resistor with none, a
/// `ki_keywords` that must stay in the library, and a `power:` symbol.
pub(crate) const SIM_LIBRARY: &str = "(kicad_sch\n  (version 20250610)\n  (generator \"konnect\")\n  (uuid \"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee\")\n  (paper \"A4\")\n  (lib_symbols\n    (symbol \"Device:D\"\n      (property \"Reference\" \"D\" (at 0 0 0))\n      (property \"Value\" \"D\" (at 0 0 0))\n      (property \"Sim.Device\" \"D\" (at 0 0 0) (hide yes))\n      (property \"Sim.Pins\" \"1=K 2=A\" (at 0 0 0) (hide yes))\n      (property \"Sim.Params\" \"m=\\\"x\\\" is=1n\" (at 0 0 0) (hide yes))\n      (property \"ki_keywords\" \"diode\" (at 0 0 0) (hide yes))\n    )\n    (symbol \"Device:DCOPY\"\n      (property \"Reference\" \"D\" (at 0 0 0))\n      (property \"Value\" \"DCOPY\" (at 0 0 0))\n      (property \"Sim.Device\" \"D\" (at 0 0 0) (hide yes))\n      (property \"Sim.Pins\" \"1=K 2=A\" (at 0 0 0) (hide yes))\n      (property \"Sim.Params\" \"m=\\\"x\\\" is=1n\" (at 0 0 0) (hide yes))\n    )\n    (symbol \"Device:R\"\n      (property \"Reference\" \"R\" (at 0 0 0))\n      (property \"Value\" \"R\" (at 0 0 0))\n    )\n    (symbol \"power:VSIM\"\n      (property \"Reference\" \"#PWR\" (at 0 0 0))\n      (property \"Value\" \"VSIM\" (at 0 0 0))\n      (property \"Sim.Device\" \"V\" (at 0 0 0) (hide yes))\n    )\n  )\n)\n";

/// What a placed `Device:D` must carry, in library order.
pub(crate) const DIODE_SIM: [(&str, &str); 3] = [
    ("Sim.Device", "D"),
    ("Sim.Pins", "1=K 2=A"),
    ("Sim.Params", "m=\"x\" is=1n"),
];

fn test_ctx() -> Arc<ToolContext> {
    Arc::new(ToolContext::new(
        ServerConfig {
            kicad_cli: String::new(),
            kicad_binary: String::new(),
            ipc_address: String::new(),
            project_dir: None,
            jlcpcb_db_path: None,
            auto_load_toolsets: false,
            eager_toolsets: false,
        },
        Arc::new(ToolRouter::new()),
    ))
}

async fn call(definitions: Vec<ToolDef>, name: &str, args: serde_json::Value) -> CallToolResult {
    let tool = definitions
        .into_iter()
        .find(|tool| tool.name == name)
        .expect("registered tool");
    let result = (tool.handler)(&args, test_ctx()).await.unwrap();
    assert!(!result.is_error, "{name}: {result:?}");
    result
}

fn seeded() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sim.kicad_sch");
    std::fs::write(&path, SIM_LIBRARY).unwrap();
    (dir, path)
}

/// The `Sim.*` fields of `reference` in the saved file as `(name, value,
/// hidden)`, and whether the instance carries a `ki_*` field.
pub(crate) fn saved_sim_fields(
    path: &std::path::Path,
    reference: &str,
) -> (Vec<(String, String, bool)>, bool) {
    let sch = konnect_schematic_editor::Schematic::load(path).unwrap();
    let sym = sch
        .symbols
        .iter()
        .find(|s| s.reference() == Some(reference))
        .unwrap_or_else(|| panic!("no {reference} in the saved file"));
    let sim = sym
        .properties
        .iter()
        .filter(|p| p.name.starts_with("Sim."))
        .map(|p| {
            let written = konnect_schematic_editor::sexp::writer::write(&p.to_sexp());
            (
                p.name.clone(),
                p.value.clone(),
                written.contains("(hide yes)"),
            )
        })
        .collect();
    let ki = sym.properties.iter().any(|p| p.name.starts_with("ki_"));
    (sim, ki)
}

/// `expected` as `saved_sim_fields` reports a copied, hidden set.
pub(crate) fn hidden(expected: &[(&str, &str)]) -> Vec<(String, String, bool)> {
    expected
        .iter()
        .map(|(n, v)| (n.to_string(), v.to_string(), true))
        .collect()
}

#[tokio::test]
async fn add_schematic_component_copies_library_sim_fields() {
    let (_dir, path) = seeded();
    for (lib_id, reference, x) in [("Device:D", "D1", 100.0), ("Device:R", "R1", 120.0)] {
        call(
            sch_components::tools(),
            "add_schematic_component",
            json!({"schematic": path.display().to_string(), "lib_id": lib_id,
                   "reference": reference, "x": x, "y": 50.0}),
        )
        .await;
    }
    let (diode, diode_ki) = saved_sim_fields(&path, "D1");
    assert_eq!(
        diode,
        hidden(&DIODE_SIM),
        "library order, hidden, quotes intact"
    );
    assert!(!diode_ki, "ki_* fields are library-only and are not copied");
    assert!(saved_sim_fields(&path, "R1").0.is_empty());
}

#[tokio::test]
async fn batch_place_components_copies_library_sim_fields() {
    let (_dir, path) = seeded();
    call(
        sch_batch::tools(),
        "batch_place_components",
        json!({"schematic": path.display().to_string(), "components": [
            {"lib_id": "Device:D", "x": 100.0, "y": 100.0, "reference": "D1"},
            {"lib_id": "Device:R", "x": 110.0, "y": 100.0, "reference": "R1"},
            {"lib_id": "Device:D", "x": 120.0, "y": 100.0, "reference": "D2"}]}),
    )
    .await;
    for reference in ["D1", "D2"] {
        let (sim, ki) = saved_sim_fields(&path, reference);
        assert_eq!(sim, hidden(&DIODE_SIM), "{reference}");
        assert!(!ki);
    }
    assert!(saved_sim_fields(&path, "R1").0.is_empty());
}

#[tokio::test]
async fn add_power_symbol_copies_library_sim_fields() {
    let (_dir, path) = seeded();
    call(
        sch_wiring::tools(),
        "add_power_symbol",
        json!({"schematic": path.display().to_string(), "power_net": "VSIM",
               "x": 100.33, "y": 80.01}),
    )
    .await;
    let (sim, _) = saved_sim_fields(&path, "#PWR01");
    assert_eq!(sim, hidden(&[("Sim.Device", "V")]));
}

// ---- replace_component ------------------------------------------------------

async fn place(path: &std::path::Path, lib_id: &str, reference: &str, x: f64) {
    call(
        sch_components::tools(),
        "add_schematic_component",
        json!({"schematic": path.display().to_string(), "lib_id": lib_id,
               "reference": reference, "x": x, "y": 50.0}),
    )
    .await;
}

async fn replace(path: &std::path::Path, reference: &str, new_lib_id: &str) -> serde_json::Value {
    let result = call(
        sch_components::tools(),
        "replace_component",
        json!({"schematic": path.display().to_string(), "reference": reference,
               "new_lib_id": new_lib_id}),
    )
    .await;
    let ToolContent::Text { text } = &result.content[0] else {
        panic!("expected a text result");
    };
    serde_json::from_str(text).unwrap()
}

fn names(listed: &serde_json::Value) -> Vec<(String, String)> {
    listed
        .as_array()
        .unwrap()
        .iter()
        .map(|field| {
            (
                field["name"].as_str().unwrap().to_owned(),
                field["value"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn pairs(fields: &[(&str, &str)]) -> Vec<(String, String)> {
    fields
        .iter()
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect()
}

#[tokio::test]
async fn replacing_a_diode_with_a_resistor_drops_the_diode_model() {
    let (_dir, path) = seeded();
    place(&path, "Device:D", "D1", 100.0).await;
    let result = replace(&path, "D1", "Device:R").await;
    assert_eq!(
        saved_sim_fields(&path, "D1").0,
        Vec::new(),
        "a resistor must not keep Sim.Device=D"
    );
    assert_eq!(names(&result["sim_fields_dropped"]), pairs(&DIODE_SIM));
    assert!(names(&result["sim_fields_added"]).is_empty());
}

#[tokio::test]
async fn replacing_a_resistor_with_a_diode_copies_the_diode_model() {
    let (_dir, path) = seeded();
    place(&path, "Device:R", "R1", 100.0).await;
    let result = replace(&path, "R1", "Device:D").await;
    let (sim, ki) = saved_sim_fields(&path, "R1");
    assert_eq!(sim, hidden(&DIODE_SIM), "library order, hidden");
    assert!(!ki);
    assert!(names(&result["sim_fields_dropped"]).is_empty());
    assert_eq!(names(&result["sim_fields_added"]), pairs(&DIODE_SIM));
}

#[tokio::test]
async fn the_new_symbols_fields_replace_the_old_ones_not_merge_with_them() {
    let (_dir, path) = seeded();
    place(&path, "Device:D", "D1", 100.0).await;
    replace(&path, "D1", "power:VSIM").await;
    assert_eq!(
        saved_sim_fields(&path, "D1").0,
        hidden(&[("Sim.Device", "V")]),
        "the diode's Sim.Pins and Sim.Params must not survive"
    );
}

#[tokio::test]
async fn a_hand_edited_sim_field_goes_with_the_old_part_and_is_reported() {
    let (_dir, path) = seeded();
    place(&path, "Device:D", "D1", 100.0).await;
    call(
        sch_components::tools(),
        "edit_schematic_component",
        json!({"schematic": path.display().to_string(), "reference": "D1",
               "fields": {"Sim.Params": "is=5n", "MPN": "1N4148"}}),
    )
    .await;
    let result = replace(&path, "D1", "Device:R").await;
    assert!(saved_sim_fields(&path, "D1").0.is_empty());
    assert!(
        names(&result["sim_fields_dropped"]).contains(&("Sim.Params".into(), "is=5n".into())),
        "the hand-edited value is what the caller needs to restore: {result}"
    );
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains("(property \"MPN\" \"1N4148\""),
        "only Sim.* fields are swapped:\n{saved}"
    );
}

#[tokio::test]
async fn replacing_a_symbol_with_itself_keeps_the_instance_fields() {
    let (_dir, path) = seeded();
    place(&path, "Device:D", "D1", 100.0).await;
    call(
        sch_components::tools(),
        "edit_schematic_component",
        json!({"schematic": path.display().to_string(), "reference": "D1",
               "fields": {"Sim.Params": "is=5n"}}),
    )
    .await;
    let result = replace(&path, "D1", "Device:D").await;
    let (sim, _) = saved_sim_fields(&path, "D1");
    assert!(
        sim.contains(&("Sim.Params".into(), "is=5n".into(), true)),
        "nothing was replaced, so nothing is reset: {sim:?}"
    );
    assert!(names(&result["sim_fields_dropped"]).is_empty());
    assert!(names(&result["sim_fields_added"]).is_empty());
}

#[tokio::test]
async fn the_swap_writes_no_blank_line_and_keeps_a_crlf_tab_file_consistent() {
    let (_dir, path) = seeded();
    place(&path, "Device:D", "D1", 100.0).await;
    let saved = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        saved
            .replace("\r\n", "\n")
            .replace("  ", "\t")
            .replace('\n', "\r\n"),
    )
    .unwrap();
    replace(&path, "D1", "Device:R").await;
    assert!(saved_sim_fields(&path, "D1").0.is_empty());
    replace(&path, "D1", "Device:D").await;
    let saved = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        saved.matches('\n').count(),
        saved.matches("\r\n").count(),
        "every line break stays CRLF"
    );
    assert!(!saved.contains("\r\r"), "no stray carriage return");
    assert!(!saved.contains("\r\n\r\n"), "no blank line left behind");
    let at = saved.find("(lib_id \"Device:D\")").unwrap();
    let start = saved[..at].rfind("(symbol").unwrap();
    let (_, end) = konnect_sexp::writer::find_balanced_block(&saved, start).unwrap();
    assert!(
        !saved[start..end].lines().any(|line| line.trim().is_empty()),
        "no whitespace-only line in the instance:\n{}",
        &saved[start..end]
    );
    assert_eq!(saved_sim_fields(&path, "D1").0, hidden(&DIODE_SIM));
}

/// A two-unit part whose units disagree on `Sim.*` (unit 2 has none, as a
/// hand-edited or older sheet can), replaced by a two-unit part that has two.
const TWO_UNITS: &str = r#"(kicad_sch
  (version 20260306)
  (uuid "11111111-1111-4111-8111-111111111111")
  (lib_symbols
    (symbol "Test:DUAL"
      (symbol "DUAL_1_1" (pin input line (at 0 0 0) (length 0) (name "A") (number "1")))
      (symbol "DUAL_2_1" (pin output line (at 0 0 0) (length 0) (name "Y") (number "2")))
    )
    (symbol "Test:DUAL_NEW"
      (property "Sim.Device" "SUBCKT" (at 0 0 0) (hide yes))
      (property "Sim.Name" "dual_new" (at 0 0 0) (hide yes))
      (symbol "DUAL_NEW_1_1" (pin input line (at 0 0 0) (length 0) (name "A") (number "1")))
      (symbol "DUAL_NEW_2_1" (pin output line (at 0 0 0) (length 0) (name "Y") (number "2")))
    )
  )
  (symbol
    (lib_id "Test:DUAL")
    (at 100 100 0)
    (unit 1)
    (uuid "22222222-2222-4222-8222-222222222222")
    (property "Reference" "U1" (at 100 98 0))
    (property "Value" "OLD" (at 100 102 0))
    (property "Sim.Device" "D" (at 100 100 0) (hide yes))
    (property "Sim.Pins" "1=K 2=A" (at 100 100 0) (hide yes))
    (instances (project "multi" (path "/11111111-1111-4111-8111-111111111111" (reference "U1") (unit 1))))
  )
  (symbol
    (lib_id "Test:DUAL")
    (at 100 120 180)
    (unit 2)
    (uuid "33333333-3333-4333-8333-333333333333")
    (property "Reference" "U1" (at 100 118 0))
    (property "Value" "OLD" (at 100 122 0))
    (instances (project "multi" (path "/11111111-1111-4111-8111-111111111111" (reference "U1") (unit 2))))
  )
  (sheet_instances (path "/" (page "1")))
)
"#;

#[tokio::test]
async fn every_placed_unit_gets_the_swap() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("multi.kicad_sch");
    std::fs::write(&path, TWO_UNITS).unwrap();
    let result = replace(&path, "U1", "Test:DUAL_NEW").await;
    assert_eq!(result["units_replaced"], 2);
    let wanted = [("Sim.Device", "SUBCKT"), ("Sim.Name", "dual_new")];
    let saved = konnect_schematic_editor::Schematic::load(&path).unwrap();
    let units: Vec<_> = saved
        .symbols
        .iter()
        .filter(|s| s.reference() == Some("U1"))
        .collect();
    assert_eq!(units.len(), 2);
    for unit in units {
        let sim: Vec<_> = unit
            .properties
            .iter()
            .filter(|p| p.name.starts_with("Sim."))
            .map(|p| (p.name.as_str(), p.value.as_str()))
            .collect();
        assert_eq!(sim, wanted.to_vec(), "unit {}", unit.unit);
    }
    assert_eq!(
        names(&result["sim_fields_dropped"]),
        pairs(&[("Sim.Device", "D"), ("Sim.Pins", "1=K 2=A")])
    );
}

#[tokio::test]
async fn only_sim_dot_fields_are_swapped_and_the_new_ones_land_after_the_last_property() {
    let (_dir, path) = seeded();
    place(&path, "Device:D", "D1", 100.0).await;
    call(
        sch_components::tools(),
        "edit_schematic_component",
        json!({"schematic": path.display().to_string(), "reference": "D1",
               "fields": {"Simulation": "keep", "sim.device": "keep", "SimX": "keep"}}),
    )
    .await;
    replace(&path, "D1", "power:VSIM").await;
    let saved = std::fs::read_to_string(&path).unwrap();
    let at = saved.find("(lib_id \"power:VSIM\")").unwrap();
    let start = saved[..at].rfind("(symbol").unwrap();
    let (_, end) = konnect_sexp::writer::find_balanced_block(&saved, start).unwrap();
    let instance = &saved[start..end];
    for name in ["Simulation", "sim.device", "SimX"] {
        assert!(
            instance.contains(&format!("(property \"{name}\" \"keep\"")),
            "{name} is not a Sim.* field:\n{instance}"
        );
    }
    let last = instance.rfind("(property \"").unwrap();
    assert!(
        instance[last..].starts_with("(property \"Sim.Device\" \"V\""),
        "the new field follows the last property:\n{instance}"
    );
}

#[tokio::test]
async fn a_field_the_new_symbol_repeats_exactly_is_in_neither_list() {
    let (_dir, path) = seeded();
    place(&path, "Device:D", "D1", 100.0).await;
    // power:VSIM's Sim.Device is V; give the diode's the same value first.
    call(
        sch_components::tools(),
        "edit_schematic_component",
        json!({"schematic": path.display().to_string(), "reference": "D1",
               "fields": {"Sim.Device": "V"}}),
    )
    .await;
    let result = replace(&path, "D1", "power:VSIM").await;
    assert_eq!(
        names(&result["sim_fields_dropped"]),
        pairs(&[("Sim.Pins", "1=K 2=A"), ("Sim.Params", "m=\"x\" is=1n")])
    );
    assert!(names(&result["sim_fields_added"]).is_empty(), "{result}");
    assert_eq!(
        saved_sim_fields(&path, "D1").0,
        hidden(&[("Sim.Device", "V")])
    );
}

#[tokio::test]
async fn a_unit_already_carrying_the_new_fields_is_not_rewritten() {
    let (_dir, path) = seeded();
    place(&path, "Device:R", "R1", 100.0).await;
    place(&path, "Device:D", "D1", 120.0).await;
    // A resistor replaced by a diode, and a diode by a diode, both through
    // the swap; only the first changes the file's Sim text.
    replace(&path, "R1", "Device:D").await;
    let before = std::fs::read_to_string(&path).unwrap();
    let result = replace(&path, "D1", "Device:D").await;
    assert!(names(&result["sim_fields_dropped"]).is_empty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[tokio::test]
async fn an_instance_block_written_on_one_line_is_not_corrupted() {
    let (_dir, path) = seeded();
    place(&path, "Device:R", "R1", 100.0).await;
    let saved = std::fs::read_to_string(&path).unwrap();
    // Fold the placed instance onto one line.
    let at = saved.find("(lib_id \"Device:R\")").unwrap();
    let start = saved[..at].rfind("(symbol").unwrap();
    let (_, end) = konnect_sexp::writer::find_balanced_block(&saved, start).unwrap();
    let folded = saved[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    std::fs::write(
        &path,
        format!("{}{}{}", &saved[..start], folded, &saved[end..]),
    )
    .unwrap();
    replace(&path, "R1", "Device:D").await;
    assert_eq!(saved_sim_fields(&path, "R1").0, hidden(&DIODE_SIM));
}

#[test]
fn the_readback_names_a_unit_still_carrying_an_old_sim_field() {
    let expected = |name: &str| name == "Sim.Device";
    let clean = json!({"units": [{"unit": 2, "fields": {"Sim.Device": "D", "Value": "x"}}]});
    assert_eq!(
        sch_components::unit_with_stale_sim_field(&clean, expected),
        None
    );
    let stale = json!({"units": [{"unit": 2, "fields": {"Sim.Device": "D", "Sim.Pins": "1=K"}}]});
    assert_eq!(
        sch_components::unit_with_stale_sim_field(&stale, expected),
        Some(json!(2))
    );
}

#[tokio::test]
async fn a_unit_already_carrying_the_new_fields_in_another_layout_is_left_alone() {
    let (_dir, path) = seeded();
    place(&path, "Device:D", "D1", 100.0).await;
    // The same three fields, each folded onto one line as a hand edit might.
    let mut saved = std::fs::read_to_string(&path).unwrap();
    let start = saved.rfind("(lib_id \"Device:D\")").unwrap();
    for name in ["Sim.Device", "Sim.Pins", "Sim.Params"] {
        let at = start
            + saved[start..]
                .find(&format!("(property \"{name}\""))
                .unwrap();
        let (_, end) = konnect_sexp::writer::find_balanced_block(&saved, at).unwrap();
        let folded = saved[at..end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        saved.replace_range(at..end, &folded);
    }
    std::fs::write(&path, &saved).unwrap();
    // Device:DCOPY carries exactly the diode's fields.
    let result = replace(&path, "D1", "Device:DCOPY").await;
    assert!(names(&result["sim_fields_dropped"]).is_empty(), "{result}");
    assert!(names(&result["sim_fields_added"]).is_empty(), "{result}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        saved.replace("(lib_id \"Device:D\")", "(lib_id \"Device:DCOPY\")"),
        "only the lib_id changes"
    );
}
