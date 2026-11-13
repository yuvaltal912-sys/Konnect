//! S6 first half: pour ownership suppresses intent widening, not class rules.
use super::*;
use konnect_ipc::IpcRoutingRules;
use serde_json::json;

const TWO: &str = include_str!("../tests/fixtures/specctra_rounded_board.kicad_pcb");
const FOUR: &str = include_str!("../tests/fixtures/specctra_four_layer_back.kicad_pcb");

fn rules() -> IpcEffectiveRoutingRules {
    ["GND", "VCC"]
        .into_iter()
        .map(|net| {
            (
                net.to_string(),
                IpcRoutingRules {
                    class_name: "Default".into(),
                    constituents: vec!["Default".into()],
                    track_width_mm: Some(0.25),
                    clearance_mm: Some(0.2),
                    via_diameter_mm: Some(0.6),
                    via_drill_mm: Some(0.3),
                },
            )
        })
        .collect()
}

fn with_intent(source: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.kicad_pcb");
    std::fs::write(&board, source).unwrap();
    let intent = crate::tools::design_intent::design_intent_path(dir.path());
    std::fs::create_dir_all(intent.parent().unwrap()).unwrap();
    std::fs::write(
        intent,
        json!({"schema_version": 1, "currents": [
            {"net": "GND", "max_current_a": 3.0}, {"net": "VCC", "max_current_a": 3.0}
        ]})
        .to_string(),
    )
    .unwrap();
    (dir, board)
}

#[test]
fn pour_nets_keep_their_class_width_even_when_pours_are_omitted() {
    for source in [TWO, FOUR] {
        let (_dir, board) = with_intent(source);
        let snapshot = crate::routing_context::Snapshot::read(&board).unwrap();
        for pours in [PourPolicy::default(), PourPolicy::omit()] {
            let bundle = export_dsn_with_context(
                &board,
                source,
                &rules(),
                &snapshot,
                ExportOptions {
                    widths_from_intent: true,
                    pours,
                    ..Default::default()
                },
            )
            .unwrap();
            let report = &bundle.intent_widths;
            let pour = report.iter().find(|r| r["net"] == "GND").unwrap();
            assert_eq!(pour["status"], "not_widened");
            assert_eq!(pour["from_mm"], 0.25);
            assert_eq!(pour["to_mm"], 0.25);
            assert!(pour["reason"].as_str().unwrap().contains("copper pour"));
            let power = report.iter().find(|r| r["net"] == "VCC").unwrap();
            assert_eq!(power["status"], "widened");
            assert!(power["to_mm"].as_f64().unwrap() > 0.25);
            let manifest: serde_json::Value = serde_json::from_str(&bundle.manifest).unwrap();
            assert_eq!(manifest["intent_widths"], json!(report));
            let net = manifest["nets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["name"] == "GND")
                .unwrap();
            let class = manifest["classes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["name"] == net["class_name"])
                .unwrap();
            assert_eq!(class["track_width_um"], 250);
            assert!(net["min_track_width_um"].is_null());
        }
    }
}

#[test]
fn pour_ownership_uses_resolved_net_identity_not_the_ground_name() {
    // KiCad 8/9 numbered nets and a supply pour, rather than a name heuristic.
    let source = FOUR
        .replace(
            "(net \"GND\")\n\t\t(layer \"In2.Cu\")",
            "(net 7)\n\t\t(net_name \"VCC\")\n\t\t(layer \"In2.Cu\")",
        )
        .replacen("(kicad_pcb", "(kicad_pcb (net 7 \"VCC\")", 1);
    let (_dir, board) = with_intent(&source);
    let (out, report) = widen_for_intent(&board, &source, &rules()).unwrap();
    assert_eq!(out["VCC"].track_width_mm, Some(0.25));
    assert!(out["GND"].track_width_mm.unwrap() > 0.25);
    assert_eq!(
        report.iter().find(|r| r["net"] == "VCC").unwrap()["status"],
        "not_widened"
    );
}

#[test]
fn rule_areas_and_non_copper_zones_do_not_suppress_widening() {
    let hook = "(net \"GND\")\n\t\t(layer \"In2.Cu\")";
    for replacement in [
        "(net \"GND\")\n\t\t(layer \"In2.Cu\") (keepout (tracks not_allowed) (vias not_allowed))",
        "(net \"GND\")\n\t\t(layer \"Dwgs.User\")",
    ] {
        let source = FOUR.replace(hook, replacement);
        assert_ne!(source, FOUR);
        let (_dir, board) = with_intent(&source);
        let (out, _) = widen_for_intent(&board, &source, &rules()).unwrap();
        assert!(out["GND"].track_width_mm.unwrap() > 0.25);
    }
}

#[test]
fn pour_ownership_does_not_fill_in_a_missing_class_width() {
    let (_dir, board) = with_intent(FOUR);
    let mut incomplete = rules();
    incomplete.get_mut("GND").unwrap().track_width_mm = None;
    let (out, _) = widen_for_intent(&board, FOUR, &incomplete).unwrap();
    assert_eq!(out["GND"].track_width_mm, None);
    assert!(export_dsn(&board, FOUR, &out).is_err());
}

#[test]
fn no_intent_and_explicit_opt_out_keep_the_dsn_identical() {
    for source in [TWO, FOUR] {
        let (dir, board) = with_intent(source);
        let snapshot = crate::routing_context::Snapshot::read(&board).unwrap();
        let options = ExportOptions::default();
        let before = export_dsn_with_options(&board, source, &rules(), options.clone()).unwrap();
        let opted_out =
            export_dsn_with_context(&board, source, &rules(), &snapshot, options.clone()).unwrap();
        assert_eq!(before.dsn, opted_out.dsn);
        assert!(opted_out.intent_widths.is_empty());
        std::fs::remove_file(crate::tools::design_intent::design_intent_path(dir.path())).unwrap();
        let snapshot = crate::routing_context::Snapshot::read(&board).unwrap();
        let absent = export_dsn_with_context(
            &board,
            source,
            &rules(),
            &snapshot,
            ExportOptions {
                widths_from_intent: true,
                ..options
            },
        )
        .unwrap();
        assert_eq!(before.dsn, absent.dsn);
        assert!(absent.intent_widths.is_empty());
    }
}

// ─── S6 second half: a per-zone pour policy ─────────────────────────────────
//
// `specctra_pours_policy.kicad_pcb` has five pour layers and one rule area:
// GND_TOP (GND, F.Cu), GND_BOTTOM (GND, B.Cu), VCC_REGION (VCC, F.Cu),
// GND_BOTH (GND, F.Cu and B.Cu) and the rule area `no_route_keepout`.

const POLICY: &str = include_str!("../tests/fixtures/specctra_pours_policy.kicad_pcb");
const GND_TOP: &str = "55555555-5555-4555-8555-555555555501";
const GND_BOTH: &str = "55555555-5555-4555-8555-555555555503";
const KEEPOUT: &str = "44444444-4444-4444-8444-444444444442";

fn policy(value: serde_json::Value) -> Result<PourPolicy, String> {
    PourPolicy::from_argument(Some(&value))
}

fn export_policy(pours: PourPolicy) -> Result<ExportBundle> {
    export_dsn_with_options(
        Path::new("board.kicad_pcb"),
        POLICY,
        &rules(),
        ExportOptions {
            pours,
            ..Default::default()
        },
    )
}

/// The planes the router is given, as (net, layer), sorted.
fn planes_of(dsn: &str) -> Vec<(String, String)> {
    let mut tokenizer = ListTokenizer::new(BufReader::new(Cursor::new(dsn.as_bytes())));
    let file = dsn::DsnFile::read_dsn(&mut tokenizer).expect("the DSN parses");
    let mut planes: Vec<(String, String)> = file
        .pcb
        .structure
        .planes
        .iter()
        .map(|plane| (plane.net.clone(), plane.polygon.layer.clone()))
        .collect();
    planes.sort();
    planes
}

fn pair(net: &str, layer: &str) -> (String, String) {
    (net.to_string(), layer.to_string())
}

#[test]
fn the_pours_argument_still_reads_the_two_whole_board_strings() {
    assert_eq!(PourPolicy::from_argument(None), Ok(PourPolicy::default()));
    assert_eq!(policy(json!("fixed")), Ok(PourPolicy::default()));
    assert_eq!(policy(json!("omit")), Ok(PourPolicy::omit()));
    assert_eq!(PourPolicy::default().default, PourMode::Fixed);
    for wrong in [json!("plane"), json!(""), json!(1), json!(true), json!([])] {
        let error = policy(wrong.clone()).unwrap_err();
        assert!(
            error.contains("'fixed'") && error.contains("'omit'"),
            "{wrong}: {error}"
        );
    }
}

#[test]
fn a_per_zone_policy_reads_a_default_and_rules_with_selectors() {
    let read = policy(json!({
        "default": "omit",
        "zones": [
            { "net": "GND", "layer": "B.Cu", "policy": "fixed" },
            { "uuid": GND_TOP, "name": "GND_TOP", "policy": "omit" }
        ]
    }))
    .unwrap();
    assert_eq!(read.default, PourMode::Omit);
    assert_eq!(
        read.zones,
        vec![
            PourRule {
                select: PourSelector {
                    net: Some("GND".into()),
                    layer: Some("B.Cu".into()),
                    ..Default::default()
                },
                policy: PourMode::Fixed,
            },
            PourRule {
                select: PourSelector {
                    uuid: Some(GND_TOP.into()),
                    name: Some("GND_TOP".into()),
                    ..Default::default()
                },
                policy: PourMode::Omit,
            },
        ]
    );
    // `default` is optional and is `fixed`; an empty list is allowed.
    assert_eq!(policy(json!({ "zones": [] })), Ok(PourPolicy::default()));
    assert_eq!(policy(json!({ "default": "omit" })), Ok(PourPolicy::omit()));
}

#[test]
fn a_per_zone_policy_refuses_what_it_does_not_know() {
    let refused = |value: serde_json::Value, needle: &str| {
        let error = policy(value.clone()).unwrap_err();
        assert!(error.contains(needle), "{value}: {error}");
    };
    // An unknown key, at the top or in a rule, is named.
    refused(json!({ "mode": "omit" }), "mode");
    refused(
        json!({ "zones": [{ "zone": "GND_TOP", "policy": "omit" }] }),
        "zone",
    );
    refused(
        json!({ "zones": [{ "net": "GND", "policy": "omit", "priority": 1 }] }),
        "priority",
    );
    // Policies are `fixed` and `omit` only.
    refused(json!({ "default": "plane" }), "default");
    refused(
        json!({ "zones": [{ "net": "GND", "policy": "plane" }] }),
        "zones[0]",
    );
    refused(json!({ "zones": [{ "net": "GND" }] }), "policy");
    // A rule picks something: the board-wide choice is `default`.
    refused(json!({ "zones": [{ "policy": "omit" }] }), "selector");
    // Selectors are non-empty strings.
    refused(json!({ "zones": [{ "net": "", "policy": "omit" }] }), "net");
    refused(json!({ "zones": [{ "net": 3, "policy": "omit" }] }), "net");
    refused(
        json!({ "zones": [{ "layer": null, "policy": "omit" }] }),
        "layer",
    );
    // The list is a list, and bounded.
    refused(
        json!({ "zones": { "net": "GND", "policy": "omit" } }),
        "zones",
    );
    refused(json!({ "zones": ["GND"] }), "zones[0]");
    let many: Vec<serde_json::Value> = (0..=MAX_POUR_RULES)
        .map(|i| json!({ "name": format!("Z{i}"), "policy": "omit" }))
        .collect();
    refused(json!({ "zones": many }), "256");
    // Exactly the bound is allowed.
    let most = &many[..MAX_POUR_RULES];
    assert_eq!(
        policy(json!({ "zones": most })).unwrap().zones.len(),
        MAX_POUR_RULES
    );
}

/// Only a policy that can leave a pour out needs the Rust exporter.
#[test]
fn only_a_policy_that_can_leave_a_pour_out_may_omit() {
    assert!(!PourPolicy::default().may_omit());
    assert!(PourPolicy::omit().may_omit());
    assert!(
        !policy(json!({ "zones": [{ "net": "GND", "policy": "fixed" }] }))
            .unwrap()
            .may_omit()
    );
    assert!(
        policy(json!({ "zones": [{ "net": "GND", "policy": "omit" }] }))
            .unwrap()
            .may_omit()
    );
    assert!(
        policy(json!({ "default": "omit", "zones": [{ "net": "GND", "policy": "fixed" }] }))
            .unwrap()
            .may_omit()
    );
}

#[test]
fn an_object_with_no_rules_writes_the_same_dsn_as_its_string() {
    for (string, object) in [
        ("fixed", json!({ "zones": [] })),
        ("fixed", json!({ "default": "fixed" })),
        ("omit", json!({ "default": "omit", "zones": [] })),
    ] {
        let by_string = export_policy(policy(json!(string)).unwrap()).unwrap();
        let by_object = export_policy(policy(object.clone()).unwrap()).unwrap();
        assert_eq!(by_string.dsn, by_object.dsn, "{object}");
        assert_eq!(by_string.manifest, by_object.manifest, "{object}");
    }
}

#[test]
fn a_rule_leaves_out_only_the_pours_it_picks() {
    let all = planes_of(&export_policy(PourPolicy::default()).unwrap().dsn);
    assert_eq!(
        all,
        vec![
            pair("GND", "B.Cu"),
            pair("GND", "B.Cu"),
            pair("GND", "F.Cu"),
            pair("GND", "F.Cu"),
            pair("VCC", "F.Cu"),
        ]
    );
    // By name: the top GND pour goes, everything else stays.
    let by_name = export_policy(
        policy(json!({ "zones": [{ "name": "GND_TOP", "policy": "omit" }] })).unwrap(),
    )
    .unwrap();
    assert_eq!(
        planes_of(&by_name.dsn),
        vec![
            pair("GND", "B.Cu"),
            pair("GND", "B.Cu"),
            pair("GND", "F.Cu"),
            pair("VCC", "F.Cu"),
        ]
    );
    // By net and layer: every GND pour layer on F.Cu goes, including that
    // layer of the two-layer zone; the VCC pour on F.Cu stays.
    let by_net_layer = export_policy(
        policy(json!({ "zones": [{ "net": "GND", "layer": "F.Cu", "policy": "omit" }] })).unwrap(),
    )
    .unwrap();
    assert_eq!(
        planes_of(&by_net_layer.dsn),
        vec![
            pair("GND", "B.Cu"),
            pair("GND", "B.Cu"),
            pair("VCC", "F.Cu")
        ]
    );
    // The other way round: everything left out but the bottom GND planes.
    let only_bottom = export_policy(
        policy(json!({
            "default": "omit",
            "zones": [{ "net": "GND", "layer": "B.Cu", "policy": "fixed" }]
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        planes_of(&only_bottom.dsn),
        vec![pair("GND", "B.Cu"), pair("GND", "B.Cu")]
    );
    // Rule areas always stay.
    for bundle in [&by_name, &by_net_layer, &only_bottom] {
        assert!(bundle.dsn.contains("(keepout"), "{}", bundle.dsn);
    }
}

#[test]
fn a_layer_selector_splits_a_zone_that_covers_two_layers() {
    let bundle = export_policy(
        policy(json!({ "zones": [{ "uuid": GND_BOTH, "layer": "F.Cu", "policy": "omit" }] }))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        planes_of(&bundle.dsn),
        vec![
            pair("GND", "B.Cu"),
            pair("GND", "B.Cu"),
            pair("GND", "F.Cu"),
            pair("VCC", "F.Cu"),
        ]
    );
    let manifest: serde_json::Value = serde_json::from_str(&bundle.manifest).unwrap();
    let both: Vec<(&str, &str)> = manifest["pours"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["uuid"] == GND_BOTH)
        .map(|row| {
            (
                row["layer"].as_str().unwrap(),
                row["policy"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(both, vec![("B.Cu", "fixed"), ("F.Cu", "omit")]);
}

#[test]
fn rules_that_disagree_about_one_pour_are_refused() {
    let error = export_policy(
        policy(json!({ "zones": [
            { "net": "GND", "policy": "omit" },
            { "name": "GND_BOTTOM", "policy": "fixed" }
        ] }))
        .unwrap(),
    )
    .unwrap_err()
    .to_string();
    for needle in [
        "zones[0] gives 'omit'",
        "zones[1] gives 'fixed'",
        "GND_BOTTOM",
        "B.Cu",
    ] {
        assert!(error.contains(needle), "{needle}: {error}");
    }
    // Rules that agree are fine.
    let agreeing = export_policy(
        policy(json!({ "zones": [
            { "net": "GND", "policy": "omit" },
            { "name": "GND_BOTTOM", "policy": "omit" }
        ] }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(planes_of(&agreeing.dsn), vec![pair("VCC", "F.Cu")]);
}

#[test]
fn a_rule_that_picks_no_pour_is_refused() {
    let refused = |rule: serde_json::Value, needles: &[&str]| {
        let error = export_policy(policy(json!({ "zones": [rule.clone()] })).unwrap())
            .unwrap_err()
            .to_string();
        for needle in needles {
            assert!(error.contains(needle), "{rule}: {needle}: {error}");
        }
        // A rule area is named only when the rule picked one.
        let names_an_area = needles.contains(&"rule area");
        assert_eq!(
            error.contains("rule area"),
            names_an_area,
            "{rule}: {error}"
        );
    };
    refused(
        json!({ "uuid": "00000000-0000-4000-8000-000000000000", "policy": "omit" }),
        &["zones[0]", "no pour"],
    );
    refused(
        json!({ "name": "gnd_top", "policy": "omit" }),
        &["zones[0]", "no pour"],
    );
    // VCC has a pour on F.Cu only.
    refused(
        json!({ "net": "VCC", "layer": "B.Cu", "policy": "omit" }),
        &["zones[0]", "no pour"],
    );
    // A rule area is not a pour: rule areas always stay.
    refused(
        json!({ "uuid": KEEPOUT, "policy": "omit" }),
        &["zones[0]", "rule area"],
    );
    // A layer the board does not have.
    refused(
        json!({ "layer": "In1.Cu", "policy": "omit" }),
        &["zones[0]", "In1.Cu", "copper layer"],
    );
}

#[test]
fn the_manifest_states_what_was_done_with_each_pour() {
    let bundle = export_policy(
        policy(json!({ "zones": [{ "name": "GND_TOP", "policy": "omit" }] })).unwrap(),
    )
    .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&bundle.manifest).unwrap();
    let mut rows: Vec<serde_json::Value> = manifest["pours"].as_array().unwrap().clone();
    rows.sort_by_key(|row| (row["uuid"].to_string(), row["layer"].to_string()));
    assert_eq!(
        rows,
        vec![
            json!({ "uuid": "44444444-4444-4444-8444-444444444441", "name": "GND_BOTTOM", "net": "GND", "layer": "B.Cu", "policy": "fixed", "chosen_by": "default" }),
            json!({ "uuid": GND_TOP, "name": "GND_TOP", "net": "GND", "layer": "F.Cu", "policy": "omit", "chosen_by": "zones[0]" }),
            json!({ "uuid": "55555555-5555-4555-8555-555555555502", "name": "VCC_REGION", "net": "VCC", "layer": "F.Cu", "policy": "fixed", "chosen_by": "default" }),
            json!({ "uuid": GND_BOTH, "name": "GND_BOTH", "net": "GND", "layer": "B.Cu", "policy": "fixed", "chosen_by": "default" }),
            json!({ "uuid": GND_BOTH, "name": "GND_BOTH", "net": "GND", "layer": "F.Cu", "policy": "fixed", "chosen_by": "default" }),
        ]
    );
    // The inventory the importer checks still counts every pour.
    assert_eq!(manifest["supported_profile"]["zone_plane_count"], 5);
    assert_eq!(manifest["supported_profile"]["zone_keepout_count"], 1);
    // Some pours reached the router, so they were not all left out.
    assert_eq!(manifest["supported_profile"]["pours_omitted"], false);
    // Every pour left out by rules, with a `fixed` default, counts as omitted.
    let none = export_policy(
        policy(json!({ "zones": [
            { "net": "GND", "policy": "omit" }, { "net": "VCC", "policy": "omit" }
        ] }))
        .unwrap(),
    )
    .unwrap();
    let none: serde_json::Value = serde_json::from_str(&none.manifest).unwrap();
    assert_eq!(none["supported_profile"]["pours_omitted"], true);
}

#[test]
fn the_importer_accepts_a_per_zone_manifest() {
    // The rounded board's recorded session: the same parts and identifiers,
    // with 0.2 mm tracks, so the classes here are 0.2 mm.
    let ses = include_str!("../tests/fixtures/specctra_rounded_board.freerouting-2.3.0.ses");
    let mut narrow = rules();
    for rule in narrow.values_mut() {
        rule.track_width_mm = Some(0.2);
    }
    let temp = tempfile::tempdir().unwrap();
    let board = temp.path().join("specctra_rounded_board.kicad_pcb");
    std::fs::write(&board, POLICY).unwrap();
    let bundle = export_dsn_with_options(
        &board,
        POLICY,
        &narrow,
        ExportOptions {
            pours: policy(json!({ "zones": [{ "name": "GND_TOP", "policy": "omit" }] })).unwrap(),
            ..Default::default()
        },
    )
    .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&bundle.manifest).unwrap();
    assert_eq!(manifest["pours"].as_array().unwrap().len(), 5);
    let plan =
        crate::specctra_ses::parse_import_plan(&board, POLICY, &bundle.manifest, ses).unwrap();
    assert!(!plan.tracks.is_empty());
}

#[test]
fn the_native_exporter_is_not_adopted_when_any_pour_is_left_out() {
    let native = include_str!("../tests/fixtures/specctra_rounded_board.native-kicad-10.dsn");
    let bundle = export_policy(
        policy(json!({ "zones": [{ "name": "GND_TOP", "policy": "omit" }] })).unwrap(),
    )
    .unwrap();
    let error = adopt_native_dsn(bundle, native.to_string())
        .unwrap_err()
        .to_string();
    assert!(error.contains("left out"), "{error}");
}

#[test]
fn the_planes_a_dsn_gives_the_router_are_read_from_the_dsn() {
    let bundle = export_policy(
        policy(json!({ "zones": [{ "net": "GND", "layer": "F.Cu", "policy": "omit" }] })).unwrap(),
    )
    .unwrap();
    let planes = dsn_planes(&bundle.dsn).unwrap();
    assert_eq!(
        planes,
        vec![
            DsnPlane {
                net: "GND".into(),
                layer: "B.Cu".into()
            },
            DsnPlane {
                net: "VCC".into(),
                layer: "F.Cu".into()
            },
        ],
        "one row per net and layer"
    );
    let none = export_policy(PourPolicy::omit()).unwrap();
    assert!(dsn_planes(&none.dsn).unwrap().is_empty());
    assert!(dsn_planes("not a dsn").is_err());
}

/// The synthetic routing fixture exports in both shapes the measurement
/// uses: every pour out, or the top pour out and the bottom one as a plane.
#[test]
fn the_pour_routing_fixture_exports_with_and_without_its_bottom_plane() {
    let board = include_str!("../tests/fixtures/specctra_pours_routing.kicad_pcb");
    let rules: IpcEffectiveRoutingRules = std::iter::once("GND".to_string())
        .chain((1..=6).map(|k| format!("S{k}")))
        .map(|net| {
            (
                net,
                IpcRoutingRules {
                    class_name: "Default".into(),
                    constituents: vec!["Default".into()],
                    track_width_mm: Some(0.25),
                    clearance_mm: Some(0.2),
                    via_diameter_mm: Some(0.6),
                    via_drill_mm: Some(0.3),
                },
            )
        })
        .collect();
    let export = |pours: serde_json::Value| {
        export_dsn_with_options(
            Path::new("specctra_pours_routing.kicad_pcb"),
            board,
            &rules,
            ExportOptions {
                pours: policy(pours).unwrap(),
                ..Default::default()
            },
        )
        .unwrap()
    };
    assert!(planes_of(&export(json!("omit")).dsn).is_empty());
    let plane = export(json!({ "zones": [{ "name": "GND_TOP", "policy": "omit" }] }));
    assert_eq!(planes_of(&plane.dsn), vec![pair("GND", "B.Cu")]);
    let manifest: serde_json::Value = serde_json::from_str(&plane.manifest).unwrap();
    assert_eq!(manifest["components"].as_array().unwrap().len(), 12);
    assert_eq!(manifest["nets"].as_array().unwrap().len(), 7);
}
