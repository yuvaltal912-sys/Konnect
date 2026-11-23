//! Row ZR, stage ZR1: the zone plan, the zones file, the engine's `planes` object and
//! the pad comparison (docs/ZONE_ROUTING_DESIGN.md, "Tests", the Konnect rows). Each
//! guard's test carries its negative control.

use super::*;
use konnect_ipc::{IpcEffectiveRoutingRules, IpcRoutingRules};

/// Two 0402 resistors and a GND pour on B.Cu, with a track-and-via rule area on F.Cu.
const ROUNDED: &str = include_str!("../tests/fixtures/specctra_rounded_board.kicad_pcb");

/// The GND pour of the fixture on B.Cu, then the same pour on F.Cu: the two planes.
const B_ZONE_UUID: &str = "44444444-4444-4444-8444-444444444441";
const F_ZONE_UUID: &str = "55555555-5555-4555-8555-555555555555";

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

/// The fixture with a second GND pour on F.Cu (the fixture's B.Cu pour copied, after
/// it), R2's pad 2 on GND (so GND has three pads), and, with `round`, R1's GND pad a
/// circle.
fn board(round: bool) -> String {
    let b_zone_start = ROUNDED.find("\t(zone\n\t\t(net \"GND\")").unwrap();
    let b_zone_end = ROUNDED[b_zone_start..].find("\n\t)\n").unwrap() + b_zone_start + 4;
    let f_zone = ROUNDED[b_zone_start..b_zone_end]
        .replace("(layer \"B.Cu\")", "(layer \"F.Cu\")")
        .replace(B_ZONE_UUID, F_ZONE_UUID);
    let mut text = format!(
        "{}{}{}",
        &ROUNDED[..b_zone_end],
        f_zone,
        &ROUNDED[b_zone_end..]
    );
    text = text.replace(
        "(net \"VCC\")\n\t\t\t(uuid \"10e20b3d-431e-4a39-a976-77389855efa1\")",
        "(net \"GND\")\n\t\t\t(uuid \"10e20b3d-431e-4a39-a976-77389855efa1\")",
    );
    if round {
        let r1_pad2 = "(pad \"2\" smd roundrect\n\t\t\t(at 0.5 0)\n\t\t\t(size 0.6 0.5)\n\t\t\t(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")\n\t\t\t(roundrect_rratio 0.25)\n\t\t\t(net \"GND\")\n\t\t\t(uuid \"80cbeaee-444b-4a13-95c2-2c277db1ac17\")";
        assert_eq!(text.matches(r1_pad2).count(), 1);
        text = text.replace(
            r1_pad2,
            "(pad \"2\" smd circle\n\t\t\t(at 0.5 0)\n\t\t\t(size 0.5 0.5)\n\t\t\t(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")\n\t\t\t(net \"GND\")\n\t\t\t(uuid \"80cbeaee-444b-4a13-95c2-2c277db1ac17\")",
        );
    }
    text
}

/// The export's DSN and manifest of `text`.
fn exported(text: &str) -> (String, Value) {
    let bundle = crate::specctra::export_dsn(
        Path::new("/scratch/specctra_rounded_board.kicad_pcb"),
        text,
        &rules(),
    )
    .unwrap();
    (bundle.dsn, serde_json::from_str(&bundle.manifest).unwrap())
}

/// The plan of `planned` (the board at route time) against the export of `exported_from`.
fn plan_with(exported_from: &str, planned: &str, rules_text: Option<&str>) -> ZonePlan {
    let (dsn, manifest) = exported(exported_from);
    zone_plan(planned, Some(&manifest), &dsn, None, rules_text).unwrap()
}

fn plan(text: &str) -> ZonePlan {
    plan_with(text, text, None)
}

fn modelled(plan: &ZonePlan) -> Vec<(usize, &str, &str)> {
    plan.zones
        .iter()
        .map(|zone| (zone.plane_index, zone.net.as_str(), zone.layer.as_str()))
        .collect()
}

fn reasons(plan: &ZonePlan) -> Vec<&str> {
    plan.legacy
        .iter()
        .map(|zone| zone.reason.as_str())
        .collect()
}

/// The F.Cu pour's `(fill …)` replaced by `fill`.
fn with_f_fill(text: &str, fill: &str) -> String {
    let at = text.find(F_ZONE_UUID).unwrap();
    let start = text[at..].find("(fill yes").unwrap() + at;
    let end = text[start..].find("\n\t\t)").unwrap() + start + 4;
    format!("{}{fill}{}", &text[..start], &text[end..])
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| name.to_string()).collect()
}

#[test]
fn a_manifest_and_dsn_that_agree_give_rows_in_plane_order() {
    let text = board(false);
    let plan = plan(&text);
    assert_eq!(modelled(&plan), [(0, "GND", "B.Cu"), (1, "GND", "F.Cu")]);
    assert!(plan.legacy.is_empty(), "{:?}", plan.legacy);
    assert_eq!(plan.zones[0].uuid.as_deref(), Some(B_ZONE_UUID));
    assert_eq!(
        plan.zones[0].settings,
        FillSettings {
            clearance_mm: 0.5,
            min_width_mm: 0.25,
            connect: PadConnect::Thermal,
            thermal_gap_mm: 0.5,
            thermal_bridge_mm: 0.5,
            priority: 0,
            islands: IslandRule::Remove,
        }
    );
}

#[test]
fn a_manifest_and_dsn_that_disagree_model_nothing() {
    let text = board(false);
    let (dsn, manifest) = exported(&text);
    // The DSN's second plane removed: one plane for two fixed pours.
    let second = dsn.match_indices("(plane GND").nth(1).unwrap().0;
    let end = dsn[second..].find("\n    )").unwrap() + second + 6;
    let short = format!("{}{}", &dsn[..second], &dsn[end..]);
    assert_eq!(crate::specctra::dsn_plane_order(&short).unwrap().len(), 1);
    let plan = zone_plan(&text, Some(&manifest), &short, None, None).unwrap();
    assert!(plan.zones.is_empty(), "{:?}", plan.zones);
    assert_eq!(plan.legacy.len(), 2);
    for reason in reasons(&plan) {
        assert!(
            reason.contains("gives 2 pour plane(s) and the DSN has 1"),
            "{reason}"
        );
    }
    assert_eq!(file_text(&plan), None);
    // No manifest: every pour legacy, with the reason.
    let none = zone_plan(&text, None, &dsn, None, None).unwrap();
    assert!(none.zones.is_empty());
    assert!(reasons(&none)
        .iter()
        .all(|reason| reason.contains("no manifest")));
    // The first plane on another layer: it no longer matches the manifest's order.
    let swapped = dsn.replacen("(polygon B.Cu", "(polygon X.Cu", 1);
    assert_ne!(swapped, dsn);
    let plan = zone_plan(&text, Some(&manifest), &swapped, None, None).unwrap();
    assert!(plan.zones.is_empty());
    assert!(
        reasons(&plan)[0].contains("the DSN's plane 0 is net 'GND' on X.Cu"),
        "{:?}",
        reasons(&plan)
    );
    // A manifest row that is not the board's pour.
    let mut other = manifest.clone();
    other["pours"][1]["uuid"] = json!("not-the-zone");
    let plan = zone_plan(&text, Some(&other), &dsn, None, None).unwrap();
    assert!(plan.zones.is_empty());
    assert!(reasons(&plan)[0].contains("the manifest's pour 1 is not the board's pour 1"));
    // Control: the same inputs, untouched, give rows in order.
    let plan = zone_plan(&text, Some(&manifest), &dsn, None, None).unwrap();
    assert_eq!(modelled(&plan), [(0, "GND", "B.Cu"), (1, "GND", "F.Cu")]);
}

#[test]
fn an_omitted_pour_has_no_plane_and_shifts_no_index() {
    let text = board(false);
    let (dsn, mut manifest) = exported(&text);
    // As if the export had left the B.Cu pour out: its row says omit, the DSN keeps the
    // F.Cu plane only.
    manifest["pours"][0]["policy"] = json!("omit");
    let first = dsn.find("(plane GND").unwrap();
    let end = dsn[first..].find("\n    )").unwrap() + first + 6;
    let one = format!("{}{}", &dsn[..first], &dsn[end..]);
    let plan = zone_plan(&text, Some(&manifest), &one, None, None).unwrap();
    assert_eq!(modelled(&plan), [(0, "GND", "F.Cu")]);
    assert_eq!(plan.legacy.len(), 1);
    assert_eq!(plan.legacy[0].layer, "B.Cu");
    assert!(plan.legacy[0].reason.contains("left out of the DSN"));
}

#[test]
fn an_uncovered_zone_is_not_modelled() {
    let text = board(false);
    let hatch = with_f_fill(
        &text,
        "(fill yes (mode hatch) (thermal_gap 0.5) (thermal_bridge_width 0.5) (island_removal_mode 0) (hatch_thickness 1) (hatch_gap 1.5) (hatch_orientation 0))",
    );
    let rounded = with_f_fill(
        &text,
        "(fill yes (thermal_gap 0.5) (thermal_bridge_width 0.5) (smoothing fillet) (radius 1) (island_removal_mode 0))",
    );
    let chamfered = with_f_fill(
        &text,
        "(fill yes (thermal_gap 0.5) (thermal_bridge_width 0.5) (smoothing chamfer) (radius 0.5) (island_removal_mode 0))",
    );
    let unreadable = with_f_fill(
        &text,
        "(fill yes (thermal_gap -1) (thermal_bridge_width 0.5) (island_removal_mode 0))",
    );
    for (variant, words) in [
        (&hatch, "a hatch fill"),
        (&rounded, "rounded corners"),
        (&chamfered, "chamfered corners"),
        (&unreadable, "thermal_gap '-1'"),
    ] {
        let plan = plan_with(&text, variant, None);
        assert_eq!(modelled(&plan), [(0, "GND", "B.Cu")], "{words}");
        assert_eq!(plan.legacy.len(), 1, "{words}");
        assert_eq!(plan.legacy[0].layer, "F.Cu");
        assert!(plan.legacy[0].reason.contains(words), "{:?}", plan.legacy);
    }
    // Control: the same zone solid and square (a smoothing of radius 0 changes nothing).
    let square = with_f_fill(
        &text,
        "(fill yes (thermal_gap 0.5) (thermal_bridge_width 0.5) (smoothing fillet) (island_removal_mode 0))",
    );
    let plan = plan_with(&text, &square, None);
    assert_eq!(modelled(&plan), [(0, "GND", "B.Cu"), (1, "GND", "F.Cu")]);
}

#[test]
fn fill_settings_read_each_value_and_kicads_defaults() {
    let zone = |inner: &str| konnect_sexp::parse_sexp(&format!("(zone {inner})")).unwrap();
    let all = zone(
        "(priority 3) (connect_pads yes (clearance 0.3)) (min_thickness 0.2) \
         (fill yes (thermal_gap 0.4) (thermal_bridge_width 0.6) (island_removal_mode 2) (island_area_min 4.5))",
    );
    assert_eq!(
        fill_settings(&all).unwrap(),
        FillSettings {
            clearance_mm: 0.3,
            min_width_mm: 0.2,
            connect: PadConnect::Solid,
            thermal_gap_mm: 0.4,
            thermal_bridge_mm: 0.6,
            priority: 3,
            islands: IslandRule::RemoveBelow { area_mm2: 4.5 },
        }
    );
    let bare = fill_settings(&zone("(net 1)")).unwrap();
    assert_eq!(bare.clearance_mm, 0.5);
    assert_eq!(bare.min_width_mm, 0.25);
    assert_eq!(bare.connect, PadConnect::Thermal);
    assert_eq!(bare.islands, IslandRule::Remove);
    for (inner, connect) in [
        ("(connect_pads no (clearance 0.2))", PadConnect::None),
        (
            "(connect_pads thru_hole_only (clearance 0.2))",
            PadConnect::ThermalThroughHole,
        ),
        ("(connect_pads (clearance 0.2))", PadConnect::Thermal),
    ] {
        assert_eq!(fill_settings(&zone(inner)).unwrap().connect, connect);
    }
    assert_eq!(
        fill_settings(&zone("(fill yes (island_removal_mode 1))"))
            .unwrap()
            .islands,
        IslandRule::Keep
    );
    assert_eq!(
        fill_settings(&zone("(fill yes (island_removal_mode 2))"))
            .unwrap()
            .islands,
        IslandRule::RemoveBelow { area_mm2: 10.0 }
    );
    for (inner, words) in [
        ("(min_thickness 0)", "a minimum width of 0"),
        (
            "(fill yes (island_removal_mode 7))",
            "island_removal_mode '7'",
        ),
        ("(priority -1)", "priority '-1'"),
        ("(connect_pads maybe)", "connect_pads 'maybe'"),
        ("(filled_areas_thickness yes)", "a stroked fill"),
        ("(min_thickness inf)", "min_thickness 'inf'"),
    ] {
        let reason = fill_settings(&zone(inner)).unwrap_err();
        assert!(reason.contains(words), "{inner}: {reason}");
    }
}

#[test]
fn an_uncovered_board_is_not_modelled() {
    let text = board(false);
    let via = |extra: &str| {
        text.replacen(
            "\t(zone\n",
            &format!(
                "\t(via (at 105 45) (size 0.6) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") {extra} (net \"GND\") (uuid \"66666666-6666-4666-8666-666666666666\"))\n\t(zone\n"
            ),
            1,
        )
    };
    for (variant, words) in [
        (
            via("(remove_unused_layers yes)"),
            "remove copper on unused layers",
        ),
        (
            via("(remove_unused_layers)"),
            "remove copper on unused layers",
        ),
        (
            via("(start_end_only yes)"),
            "remove copper on unused layers",
        ),
        (
            via("(zone_layer_connections \"F.Cu\")"),
            "override their zone connection per layer",
        ),
        (
            via("(backdrill (size 0.4) (layers \"B.Cu\" \"F.Cu\"))"),
            "backdrilled or post-machined",
        ),
    ] {
        let plan = plan_with(&text, &variant, None);
        assert!(plan.zones.is_empty(), "{words}: {:?}", plan.zones);
        assert_eq!(plan.legacy.len(), 2, "{words}");
        assert!(
            reasons(&plan).iter().all(|reason| reason.contains(words)),
            "{words}: {:?}",
            reasons(&plan)
        );
        assert_eq!(file_text(&plan), None);
    }
    // A custom rule that can change a zone's clearance.
    let clearance = "(version 1)\n(rule \"wide\" (constraint clearance (min 0.4)) (condition \"A.NetName == 'GND'\"))\n";
    let plan = plan_with(&text, &text, Some(clearance));
    assert!(plan.zones.is_empty());
    assert!(
        reasons(&plan)[0].contains("'wide' (clearance)"),
        "{:?}",
        reasons(&plan)
    );
    // Controls: the via plain, and a rule that cannot change a zone.
    let plan = plan_with(&text, &via("(remove_unused_layers no)"), None);
    assert_eq!(modelled(&plan).len(), 2, "{:?}", plan.legacy);
    let width = "(version 1)\n(rule \"thick\" (constraint track_width (min 0.3)) (condition \"A.NetName == 'VCC'\"))\n";
    let plan = plan_with(&text, &text, Some(width));
    assert_eq!(modelled(&plan).len(), 2, "{:?}", plan.legacy);
}

#[test]
fn a_custom_pad_with_its_own_spokes_on_a_pour_net_uncovers_the_board() {
    let text = board(false);
    let pad = "(pad \"1\" smd roundrect\n\t\t\t(at -0.5 0)\n\t\t\t(size 0.6 0.5)\n\t\t\t(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")\n\t\t\t(roundrect_rratio 0.25)\n\t\t\t(net \"GND\")";
    assert_eq!(text.matches(pad).count(), 1);
    let custom = |primitive: &str| {
        text.replace(
            pad,
            &format!(
                "(pad \"1\" smd custom\n\t\t\t(at -0.5 0)\n\t\t\t(size 0.5 0.5)\n\t\t\t(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")\n\t\t\t(options (clearance outline) (anchor circle))\n\t\t\t(primitives {primitive})\n\t\t\t(net \"GND\")"
            ),
        )
    };
    let spokes = custom("(gr_vector (start 0 0) (end 1 0) (width 0.2))");
    let plan = plan_with(&text, &spokes, None);
    assert!(plan.zones.is_empty());
    assert!(
        reasons(&plan)[0].contains("spoke shapes of their own"),
        "{:?}",
        reasons(&plan)
    );
    // Control: the same custom pad with no spoke shape is not what uncovers the board.
    let plain = custom("(gr_line (start -0.2 0) (end 0.2 0) (width 0.2))");
    let plan = plan_with(&text, &plain, None);
    assert!(
        reasons(&plan)
            .iter()
            .all(|reason| !reason.contains("spoke shapes")),
        "{:?}",
        reasons(&plan)
    );
}

#[test]
fn a_pads_own_values_reach_the_engine() {
    let text = board(true);
    let plan = plan(&text);
    assert_eq!(modelled(&plan).len(), 2);
    // R1's round GND pad: angle 45 under the F.Cu pour; R2's rectangular GND pads,
    // with the zone's values at angle 0, get no line.
    assert_eq!(
        plan.pads,
        [PadRow {
            component: "R1".into(),
            pin: "2".into(),
            connect: PadConnect::Thermal,
            thermal_gap_mm: 0.5,
            thermal_bridge_mm: 0.5,
            angle_deg: 45.0,
        }]
    );
    assert!(file_text(&plan)
        .unwrap()
        .contains("\npad R1 2 connect=thermal gap_mm=0.5 bridge_mm=0.5 angle_deg=45\n"));
    // Control: the pad rectangular again, with the zone's values: no line at all.
    let plain = self::plan(&board(false));
    assert!(plain.pads.is_empty(), "{:?}", plain.pads);
    assert!(!file_text(&plain).unwrap().contains("\npad "));
}

#[test]
fn a_pads_overrides_resolve_pad_then_footprint_then_zone() {
    let text = board(false);
    let r2_pad1_net = "(net \"GND\")\n\t\t\t(uuid \"611a5e55-7fda-4eb1-9177-2b7af8a58dba\")";
    assert_eq!(text.matches(r2_pad1_net).count(), 1);
    let pad_override = text.replace(
        r2_pad1_net,
        &format!("(zone_connect 2) (thermal_gap 0.3) (thermal_bridge_width 0.1) (thermal_bridge_angle 30)\n\t\t\t{r2_pad1_net}"),
    );
    let plan = plan(&pad_override);
    assert_eq!(
        plan.pads,
        [PadRow {
            component: "R2".into(),
            pin: "1".into(),
            connect: PadConnect::Solid,
            thermal_gap_mm: 0.3,
            // raised to the zone's minimum width, as KiCad raises a pad's override
            thermal_bridge_mm: 0.25,
            angle_deg: 30.0,
        }]
    );
    // The footprint's zone_connect is inherited by its pads that set none.
    let r2 = "(uuid \"22222222-2222-4222-8222-222222222222\")";
    let footprint = text.replace(r2, &format!("{r2}\n\t\t(zone_connect 0)"));
    let plan = self::plan(&footprint);
    let connects: Vec<(&str, &str, PadConnect)> = plan
        .pads
        .iter()
        .map(|pad| (pad.component.as_str(), pad.pin.as_str(), pad.connect))
        .collect();
    assert_eq!(
        connects,
        [("R2", "1", PadConnect::None), ("R2", "2", PadConnect::None)]
    );
    // A zone of thru_hole_only: these SMD pads are solid.
    let tht = text.replace(
        &format!("(uuid \"{F_ZONE_UUID}\")\n\t\t(hatch edge 0.5)\n\t\t(connect_pads\n"),
        &format!(
            "(uuid \"{F_ZONE_UUID}\")\n\t\t(hatch edge 0.5)\n\t\t(connect_pads thru_hole_only\n"
        ),
    );
    assert_ne!(tht, text);
    let plan = self::plan(&tht);
    assert_eq!(
        plan.zones[1].settings.connect,
        PadConnect::ThermalThroughHole
    );
    assert_eq!(plan.pads.len(), 3);
    assert!(plan.pads.iter().all(|pad| pad.connect == PadConnect::Solid));
    assert!(file_text(&plan)
        .unwrap()
        .contains("\nzone 1 clearance_mm=0.5 min_width_mm=0.25 connect=thermal "));
}

#[test]
fn a_pad_that_resolves_differently_under_two_zones_demotes_its_net() {
    // A through-hole pad under both GND pours, with the pours' gaps different and its
    // own angle 45: one line cannot say both gaps.
    let text = board(false);
    let r2_pad2 = "(pad \"2\" smd roundrect\n\t\t\t(at 0.5 0)\n\t\t\t(size 0.6 0.5)\n\t\t\t(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")\n\t\t\t(roundrect_rratio 0.25)\n\t\t\t(net \"GND\")\n\t\t\t(uuid \"10e20b3d-431e-4a39-a976-77389855efa1\")";
    assert_eq!(text.matches(r2_pad2).count(), 1);
    let through = text.replace(
        r2_pad2,
        "(pad \"2\" thru_hole circle\n\t\t\t(at 0.5 0)\n\t\t\t(size 0.8 0.8)\n\t\t\t(drill 0.4)\n\t\t\t(layers \"*.Cu\" \"*.Mask\")\n\t\t\t(net \"GND\")\n\t\t\t(uuid \"10e20b3d-431e-4a39-a976-77389855efa1\")",
    );
    let gaps = with_f_fill(
        &through,
        "(fill yes (thermal_gap 0.3) (thermal_bridge_width 0.5) (island_removal_mode 0))",
    );
    let plan = plan_with(&gaps, &gaps, None);
    assert!(plan.zones.is_empty(), "{:?}", plan.zones);
    assert!(
        reasons(&plan)
            .iter()
            .all(|reason| reason.contains("pad R2-2 resolves to other thermal values")),
        "{:?}",
        reasons(&plan)
    );
    // Control: the same gaps under both pours: one line, angle 45.
    let plan = plan_with(&through, &through, None);
    assert_eq!(modelled(&plan).len(), 2);
    assert_eq!(plan.pads.len(), 1);
    assert_eq!(plan.pads[0].angle_deg, 45.0);
}

#[test]
fn a_dsn_that_names_the_pins_otherwise_models_nothing() {
    let text = board(false);
    let (dsn, manifest) = exported(&text);
    assert!(dsn.contains("R2-2"));
    let renamed = dsn.replace("R2-2", "R2-7");
    let plan = zone_plan(&text, Some(&manifest), &renamed, None, None).unwrap();
    assert!(plan.zones.is_empty());
    assert!(
        reasons(&plan)[0].contains("does not name the pins of net 'GND'"),
        "{:?}",
        reasons(&plan)
    );
    // Control: the DSN as exported.
    let plan = zone_plan(&text, Some(&manifest), &dsn, None, None).unwrap();
    assert_eq!(modelled(&plan).len(), 2);
}

#[test]
fn a_rule_area_that_keeps_pours_out_is_a_keepout_line() {
    let text = board(false).replace("(copperpour allowed)", "(copperpour not_allowed)");
    let plan = plan(&text);
    assert_eq!(
        plan.keepouts,
        [KeepoutRow {
            layer: "F.Cu".into(),
            corners: vec![
                (112000, -52000),
                (120000, -52000),
                (120000, -62000),
                (112000, -62000)
            ],
        }]
    );
    assert!(file_text(&plan)
        .unwrap()
        .ends_with("\nkeepout F.Cu 112000 -52000 120000 -52000 120000 -62000 112000 -62000\n"));
    // Control: the rule area lets pours in: no keepout.
    assert!(self::plan(&board(false)).keepouts.is_empty());
}

#[test]
fn the_header_comes_from_the_project_else_kicads_defaults() {
    let text = board(false);
    let (dsn, manifest) = exported(&text);
    let project = json!({"board": {"design_settings": {"rules": {
        "min_resolved_spokes": 3, "min_copper_edge_clearance": 0.3, "min_hole_clearance": 0.2
    }}}})
    .to_string();
    let plan = zone_plan(&text, Some(&manifest), &dsn, Some(&project), None).unwrap();
    assert_eq!(
        (
            plan.min_spokes,
            plan.edge_clearance_mm,
            plan.hole_clearance_mm
        ),
        (3, 0.3, 0.2)
    );
    let plan = zone_plan(&text, Some(&manifest), &dsn, None, None).unwrap();
    assert_eq!(
        (
            plan.min_spokes,
            plan.edge_clearance_mm,
            plan.hole_clearance_mm,
            plan.teardrop_mm
        ),
        (2, 0.5, 0.25, 0.0)
    );
    // KiCad's legacy edge sentinel (-0.01) is not guessed at.
    let sentinel = json!({"board": {"design_settings": {"rules": {
        "min_copper_edge_clearance": -0.01
    }}}})
    .to_string();
    let plan = zone_plan(&text, Some(&manifest), &dsn, Some(&sentinel), None).unwrap();
    assert!(plan.zones.is_empty());
    assert!(reasons(&plan)[0].contains("min_copper_edge_clearance"));
    // A teardrop that is on sets the header's length; one that is off does not.
    let r1 = "(uuid \"fb481b08-0972-41e5-a5d9-a4bab9f165c1\")";
    let teardrop = |enabled: &str| {
        text.replace(
            r1,
            &format!("(teardrops (best_length_ratio 0.5) (max_length 1.5) (enabled {enabled}))\n\t\t\t{r1}"),
        )
    };
    let on = plan_with(&text, &teardrop("yes"), None);
    assert_eq!(on.teardrop_mm, 1.5);
    let off = plan_with(&text, &teardrop("no"), None);
    assert_eq!(off.teardrop_mm, 0.0);
}

#[test]
fn file_text_writes_exactly_the_contract() {
    let settings = |connect, priority, islands| FillSettings {
        clearance_mm: 0.2,
        min_width_mm: 0.25,
        connect,
        thermal_gap_mm: 0.5,
        thermal_bridge_mm: 0.5,
        priority,
        islands,
    };
    let plan = ZonePlan {
        zones: vec![
            ZoneRow {
                plane_index: 1,
                net: "GND".into(),
                layer: "F.Cu".into(),
                uuid: None,
                settings: settings(
                    PadConnect::Solid,
                    1,
                    IslandRule::RemoveBelow { area_mm2: 10.0 },
                ),
            },
            ZoneRow {
                plane_index: 0,
                net: "GND".into(),
                layer: "B.Cu".into(),
                uuid: None,
                settings: settings(PadConnect::Thermal, 0, IslandRule::Remove),
            },
        ],
        legacy: Vec::new(),
        pads: vec![
            PadRow {
                component: "U3".into(),
                pin: "5".into(),
                connect: PadConnect::Thermal,
                thermal_gap_mm: 0.5,
                thermal_bridge_mm: 0.5,
                angle_deg: 45.0,
            },
            PadRow {
                component: "J 1".into(),
                pin: "a=\"b\\".into(),
                connect: PadConnect::None,
                thermal_gap_mm: 0.125,
                thermal_bridge_mm: 0.3,
                angle_deg: 22.5,
            },
        ],
        keepouts: vec![KeepoutRow {
            layer: "B.Cu".into(),
            corners: vec![(1000, -2000), (3000, -2000), (3000, -4000)],
        }],
        min_spokes: 2,
        edge_clearance_mm: 0.3,
        hole_clearance_mm: 0.25,
        teardrop_mm: 0.0,
        names: PadNames::default(),
    };
    assert_eq!(
        file_text(&plan).unwrap(),
        "zones 1\n\
         cell_mm 0.05\n\
         min_spokes 2\n\
         edge_clearance_mm 0.3\n\
         hole_clearance_mm 0.25\n\
         teardrop_mm 0\n\
         zone 0 clearance_mm=0.2 min_width_mm=0.25 connect=thermal gap_mm=0.5 bridge_mm=0.5 priority=0 islands=remove\n\
         zone 1 clearance_mm=0.2 min_width_mm=0.25 connect=solid gap_mm=0.5 bridge_mm=0.5 priority=1 islands=below:10\n\
         pad U3 5 connect=thermal gap_mm=0.5 bridge_mm=0.5 angle_deg=45\n\
         pad \"J 1\" \"a=\\\"b\\\\\" connect=none gap_mm=0.125 bridge_mm=0.3 angle_deg=22.5\n\
         keepout B.Cu 1000 -2000 3000 -2000 3000 -4000\n"
    );
    // A plan that models no pour writes nothing, whatever else it holds.
    let nothing = ZonePlan {
        zones: Vec::new(),
        ..plan
    };
    assert_eq!(file_text(&nothing), None);
}

/// The contract's §2 example object.
fn planes_example() -> Value {
    json!({
        "version": 1,
        "error": null,
        "cell_mm": 0.05,
        "grid": [4100, 3300],
        "zones": [
            {"plane_index": 0, "net": "GND", "layer": "B.Cu", "cells": 1234567,
             "pieces": 3, "pieces_before_islands": 4}
        ],
        "legacy_planes": [1],
        "nets": ["GND"],
        "connected_pads": ["C1-2", "U1-5"],
        "open_pads": ["U2-3"],
        "starved_pads": ["C3-1"],
        "pieces": 3,
        "closing_routed": 0,
        "redraws": 1,
        "redraw_s": 0.42
    })
}

#[test]
fn engine_planes_reads_the_contracts_object() {
    let report = json!({"unrouted": [], "planes": planes_example()});
    assert_eq!(
        engine_planes(&report),
        Some(EnginePlanes {
            error: None,
            nets: set(&["GND"]),
            connected_pads: set(&["C1-2", "U1-5"]),
            open_pads: set(&["U2-3"]),
            starved_pads: set(&["C3-1"]),
            closing_routed: 0,
            pieces: 3,
            redraws: 1,
            redraw_s: 0.42,
        })
    );
    // None without the section, or with a null one.
    assert_eq!(engine_planes(&json!({"unrouted": []})), None);
    assert_eq!(engine_planes(&json!({"planes": null})), None);
    // The engine's own error is kept.
    let mut failed = planes_example();
    failed["error"] = json!("zone 2 has no plane");
    assert_eq!(
        planes_object(&failed).unwrap().error.as_deref(),
        Some("zone 2 has no plane")
    );
    // An object that does not read is an error, never a quiet prediction.
    let mut v2 = planes_example();
    v2["version"] = json!(2);
    assert!(planes_object(&v2).is_err());
    let mut both = planes_example();
    both["open_pads"] = json!(["U1-5"]);
    assert!(planes_object(&both)
        .unwrap_err()
        .to_string()
        .contains("both connected and open"));
    let mut short = planes_example();
    short.as_object_mut().unwrap().remove("connected_pads");
    assert!(planes_object(&short).is_err());
}

#[test]
fn the_record_counts_the_pad_lists() {
    let full = engine_planes_record(&planes_example(), true);
    assert_eq!(full["connected_pad_count"], 2);
    assert_eq!(full["open_pads"], json!(["U2-3"]));
    assert!(full.get("connected_pads").is_none());
    let short = engine_planes_record(&planes_example(), false);
    assert_eq!(short["open_pad_count"], 1);
    assert_eq!(short["starved_pad_count"], 1);
    assert!(short.get("open_pads").is_none());
    assert_eq!(short["redraw_s"], 0.42);
}

/// The base board, refilled: the F.Cu GND pour filled over R2's two pads only, the B.Cu
/// pour filled away from every pad.
pub(crate) fn refilled() -> String {
    let text = board(false);
    let fill = |layer: &str, (x0, y0, x1, y1): (f64, f64, f64, f64)| {
        format!(
            "\n\t\t(filled_polygon (layer \"{layer}\") (pts (xy {x0} {y0}) (xy {x1} {y0}) (xy {x1} {y1}) (xy {x0} {y1})))"
        )
    };
    let into = |text: &str, uuid: &str, polygon: String| {
        let at = text.find(uuid).unwrap();
        let end = text[at..].find("\n\t)\n").unwrap() + at;
        format!("{}{polygon}{}", &text[..end], &text[end..])
    };
    let text = into(&text, F_ZONE_UUID, fill("F.Cu", (108.0, 48.0, 112.0, 52.0)));
    into(&text, B_ZONE_UUID, fill("B.Cu", (86.0, 60.0, 90.0, 64.0)))
}

/// The base board, as [`refilled`] names its pads.
pub(crate) fn names_of_the_base_board() -> PadNames {
    PadNames::from_board(&board(false)).unwrap()
}

#[test]
fn kicads_side_is_the_audits_pads_off_the_main_group_named_as_the_dsn() {
    let names = names_of_the_base_board();
    let at = Path::new("/scratch/r0/board.kicad_pcb");
    let open = kicad_open_pads(at, &refilled(), &names, &set(&["GND"])).unwrap();
    assert_eq!(open, set(&["R1-2"]));
    // A net not asked for is not read.
    assert!(kicad_open_pads(at, &refilled(), &names, &set(&["VCC"]))
        .unwrap()
        .is_empty());
    // A pad the names do not hold cannot be named: an error, not a guess.
    let err = kicad_open_pads(at, &refilled(), &PadNames::default(), &set(&["GND"]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("not a pad the export wrote"), "{err}");
    // A pour of the net with no saved fill: its pads would read as cut off.
    let unfilled = board(false).replace("(fill yes", "(fill");
    let err = kicad_open_pads(at, &unfilled, &names, &set(&["GND"]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("has no saved fill"), "{err}");
}

#[test]
fn a_false_connection_is_counted() {
    let names = names_of_the_base_board();
    let kicad_open = kicad_open_pads(
        Path::new("/scratch/r0/board.kicad_pcb"),
        &refilled(),
        &names,
        &set(&["GND"]),
    )
    .unwrap();
    let mut object = planes_example();
    object["connected_pads"] = json!(["R1-2", "R2-1", "R2-2"]);
    object["open_pads"] = json!([]);
    let engine = planes_object(&object).unwrap();
    // KiCad leaves R1-2 open (one unconnected item); the model called it connected.
    assert_eq!(compare_pads(&engine, &kicad_open, 1), (Some(1), Some(0)));
    // Control: the pad open in both.
    object["connected_pads"] = json!(["R2-1", "R2-2"]);
    object["open_pads"] = json!(["R1-2"]);
    let engine = planes_object(&object).unwrap();
    assert_eq!(compare_pads(&engine, &kicad_open, 1), (Some(0), Some(0)));
    // The model leaving open a pad KiCad connects is a false open.
    object["connected_pads"] = json!(["R2-1"]);
    object["open_pads"] = json!(["R1-2", "R2-2"]);
    let engine = planes_object(&object).unwrap();
    assert_eq!(compare_pads(&engine, &kicad_open, 1), (Some(0), Some(1)));
}

#[test]
fn sides_that_cannot_be_compared_are_not_compared() {
    let engine = planes_object(&planes_example()).unwrap();
    // KiCad at 0 unconnected, while the audit's grouping finds a pad off the main group.
    assert_eq!(compare_pads(&engine, &set(&["U2-3"]), 0), (None, None));
    // Control: both empty compares, and counts what the model left open.
    assert_eq!(compare_pads(&engine, &set(&[]), 0), (Some(0), Some(1)));
    // The engine's own error: nothing to compare.
    let mut failed = planes_example();
    failed["error"] = json!("no model");
    let failed = planes_object(&failed).unwrap();
    assert_eq!(compare_pads(&failed, &set(&[]), 0), (None, None));
}

#[test]
fn the_comparison_is_recorded_beside_the_zr0_measures_never_as_a_failure() {
    let pours = PourMeasures {
        open_before_stitching: 1,
        open_after_stitching: 1,
        pieces: 2,
        largest_piece_share: Some(1.0),
        unfilled: 0,
        starved_thermals: 0,
        false_connected: None,
        false_open: None,
    };
    let base = || {
        let mut measures = crate::route_judge::Measures {
            unconnected: 0,
            violations: BTreeMap::new(),
            routed_share: None,
            audit_complete: None,
            acute: None,
            right_angles: None,
            copper_quality: None,
            nets_undersized: None,
            narrow_paths: None,
            signal_mm: 0.0,
            power_mm: 0.0,
            pour_track_mm: 0.0,
            vias_off_pour: 0,
            below_floor: 0,
            width_rows: vec![],
            longest_below_mm: BTreeMap::new(),
            stitching_vias: 0,
            route_s: 0.0,
            lengths: None,
            pours: None,
            pours_error: None,
        };
        measures.pours = Some(pours.clone());
        measures
    };
    let engine = || planes_object(&planes_example());
    let mut measures = base();
    record_comparison(&mut measures, engine(), Some(Ok(set(&["U1-5"]))), 1);
    let taken = measures.pours.as_ref().unwrap();
    assert_eq!(
        (taken.false_connected, taken.false_open),
        (Some(1), Some(1))
    );
    assert_eq!(measures.pours_error, None);
    // KiCad's side failed: the ZR0 measures stay, the counts are None, the reason kept.
    let mut measures = base();
    record_comparison(
        &mut measures,
        engine(),
        Some(Err(anyhow::anyhow!("no UUID"))),
        1,
    );
    assert_eq!(measures.pours.as_ref().unwrap().pieces, 2);
    assert_eq!(measures.pours.as_ref().unwrap().false_connected, None);
    assert!(measures.pours_error.as_deref().unwrap().contains("no UUID"));
    // The engine's object does not read.
    let mut measures = base();
    record_comparison(&mut measures, Err(anyhow::anyhow!("version 2")), None, 1);
    assert!(measures
        .pours_error
        .as_deref()
        .unwrap()
        .contains("version 2"));
    // No pour measures: nothing to add to.
    let mut measures = base();
    measures.pours = None;
    record_comparison(&mut measures, engine(), Some(Ok(set(&[]))), 0);
    assert_eq!(measures.pours, None);
}

/// A plan's one-line count summary: zones modelled, legacy rows and their reasons, pad
/// lines, keepouts, the header.
fn summary(name: &str, plan: &ZonePlan) -> String {
    let mut reasons: Vec<String> = plan.legacy.iter().map(|z| z.reason.clone()).collect();
    reasons.sort();
    reasons.dedup();
    format!(
        "{name}: {} zone(s) modelled, {} legacy{}, {} pad line(s), {} keepout(s); header min_spokes {} edge {} hole {} teardrop {}",
        plan.zones.len(),
        plan.legacy.len(),
        if reasons.is_empty() {
            String::new()
        } else {
            format!(" ({})", reasons.join(" | "))
        },
        plan.pads.len(),
        plan.keepouts.len(),
        plan.min_spokes,
        plan.edge_clearance_mm,
        plan.hole_clearance_mm,
        plan.teardrop_mm,
    )
}

/// The plan of the board saved in `folder` (`board.kicad_pcb`, its export `board.dsn`
/// and `board.dsn.konnect.json`, and `board.kicad_pro` / `board.kicad_dru` when there),
/// checked for what every plan must hold, with its summary.
fn plan_of_folder(name: &str, folder: &Path) -> (ZonePlan, String) {
    let read = |file: &str| std::fs::read_to_string(folder.join(file)).ok();
    let text = read("board.kicad_pcb").unwrap();
    let dsn = read("board.dsn").unwrap();
    let manifest: Value = serde_json::from_str(&read("board.dsn.konnect.json").unwrap()).unwrap();
    let plan = zone_plan(
        &text,
        Some(&manifest),
        &dsn,
        read("board.kicad_pro").as_deref(),
        read("board.kicad_dru").as_deref(),
    )
    .unwrap_or_else(|error| panic!("{name}: {error:#}"));
    let planes = crate::specctra::dsn_plane_order(&dsn).unwrap().len();
    assert_eq!(
        plan.zones.len() + plan.legacy.len(),
        manifest["pours"].as_array().map_or(0, Vec::len),
        "{name}: every pour is a row"
    );
    assert!(
        plan.zones.iter().all(|zone| zone.plane_index < planes),
        "{name}"
    );
    let line = summary(name, &plan);
    (plan, line)
}

/// The repository's pour fixture, with its tracked export: both GND pours modelled.
#[test]
fn the_pours_fixture_gets_a_plan() {
    let dir = tempfile::tempdir().unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for (from, to) in [
        ("specctra_pours_routing.kicad_pcb", "board.kicad_pcb"),
        ("specctra_pours_routing.kicad_pro", "board.kicad_pro"),
        ("specctra_pours_routing.dsn", "board.dsn"),
        (
            "specctra_pours_routing.dsn.konnect.json",
            "board.dsn.konnect.json",
        ),
    ] {
        std::fs::copy(fixtures.join(from), dir.path().join(to)).unwrap();
    }
    let (plan, line) = plan_of_folder("pours", dir.path());
    assert_eq!(
        modelled(&plan),
        [(0, "GND", "F.Cu"), (1, "GND", "B.Cu")],
        "{line}"
    );
    assert!(plan.legacy.is_empty(), "{line}");
    assert!(file_text(&plan)
        .unwrap()
        .starts_with("zones 1\ncell_mm 0.05\n"));
}

/// The zone plans of the boards under `KONNECT_ZR1_PLAN_IN` (one sub-folder a board, as
/// [`plan_of_folder`] reads it), each written to `KONNECT_ZR1_PLANS` as
/// `<name>.zones.txt`, `<name>.plan.json` and `<name>.summary.txt`. Reads and writes
/// files only: no KiCad, no router.
#[test]
#[ignore = "needs KONNECT_ZR1_PLAN_IN and KONNECT_ZR1_PLANS"]
fn zone_plans_of_a_folder() {
    let input = std::path::PathBuf::from(std::env::var_os("KONNECT_ZR1_PLAN_IN").unwrap());
    let out = std::path::PathBuf::from(std::env::var_os("KONNECT_ZR1_PLANS").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let mut boards: Vec<_> = std::fs::read_dir(&input)
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().join("board.kicad_pcb").is_file())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    boards.sort();
    assert!(!boards.is_empty(), "no board under {}", input.display());
    for name in boards {
        let (plan, line) = plan_of_folder(&name, &input.join(&name));
        println!("{line}");
        std::fs::write(
            out.join(format!("{name}.zones.txt")),
            file_text(&plan).unwrap_or_default(),
        )
        .unwrap();
        let mut body = plan.result_json(None);
        body["pads"] = json!(plan.pads);
        body["keepout_rows"] = json!(plan.keepouts);
        std::fs::write(
            out.join(format!("{name}.plan.json")),
            serde_json::to_string_pretty(&body).unwrap(),
        )
        .unwrap();
        std::fs::write(out.join(format!("{name}.summary.txt")), format!("{line}\n")).unwrap();
    }
}
