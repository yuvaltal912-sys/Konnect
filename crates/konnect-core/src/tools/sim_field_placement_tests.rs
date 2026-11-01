//! Every placement path copies the library symbol's `Sim.*` fields onto the
//! instance as hidden fields. KiCad's simulator reads the instance only, so a
//! part placed without them exports as `D1 __D1` and does not simulate.

use super::*;
use crate::router::ToolRouter;
use serde_json::json;
use std::sync::Arc;

/// A diode with the fields KiCad's Device library gives it (`Sim.Params`
/// carries quotes, to prove they survive), a resistor with none, a
/// `ki_keywords` that must stay in the library, and a `power:` symbol.
pub(crate) const SIM_LIBRARY: &str = "(kicad_sch\n  (version 20250610)\n  (generator \"konnect\")\n  (uuid \"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee\")\n  (paper \"A4\")\n  (lib_symbols\n    (symbol \"Device:D\"\n      (property \"Reference\" \"D\" (at 0 0 0))\n      (property \"Value\" \"D\" (at 0 0 0))\n      (property \"Sim.Device\" \"D\" (at 0 0 0) (hide yes))\n      (property \"Sim.Pins\" \"1=K 2=A\" (at 0 0 0) (hide yes))\n      (property \"Sim.Params\" \"m=\\\"x\\\" is=1n\" (at 0 0 0) (hide yes))\n      (property \"ki_keywords\" \"diode\" (at 0 0 0) (hide yes))\n    )\n    (symbol \"Device:R\"\n      (property \"Reference\" \"R\" (at 0 0 0))\n      (property \"Value\" \"R\" (at 0 0 0))\n    )\n    (symbol \"power:VSIM\"\n      (property \"Reference\" \"#PWR\" (at 0 0 0))\n      (property \"Value\" \"VSIM\" (at 0 0 0))\n      (property \"Sim.Device\" \"V\" (at 0 0 0) (hide yes))\n    )\n  )\n)\n";

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
