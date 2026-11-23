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
    // The engine's own error: no counts, and its text kept as the engine's.
    let mut measures = base();
    let mut failed = planes_example();
    failed["error"] = json!("zone 2 has no plane");
    record_comparison(&mut measures, planes_object(&failed), None, 1);
    assert_eq!(measures.pours.as_ref().unwrap().false_connected, None);
    assert_eq!(
        measures.pours_error.as_deref(),
        Some("the engine's model reported an error: zone 2 has no plane")
    );
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

// ─── Fix round (review of 5168952d) ──────────────────────────────────────────

/// The footprint block of `text` whose uuid is `uuid` (`\t(footprint …` to its `\t)`).
fn footprint_block<'a>(text: &'a str, uuid: &str) -> &'a str {
    let at = text.find(uuid).unwrap();
    let start = text[..at].rfind("\t(footprint ").unwrap();
    let end = text[start..].find("\n\t)\n").unwrap() + start + 4;
    &text[start..end]
}

/// The base board with R1 and R2 copied as R3 and R4, 8 mm lower: GND has six pads, three
/// on each pair.
fn six_pads() -> String {
    let text = board(false);
    let r1 = footprint_block(&text, "11111111-1111-4111-8111-111111111111");
    let r2 = footprint_block(&text, "22222222-2222-4222-8222-222222222222");
    let r3 = r1
        .replace(
            "11111111-1111-4111-8111-111111111111",
            "77777777-7777-4777-8777-777777777777",
        )
        .replacen("(at 100 50)", "(at 100 58)", 1)
        .replace("\"Reference\" \"R1\"", "\"Reference\" \"R3\"")
        .replace("fb481b08-", "fb481b09-")
        .replace("80cbeaee-", "80cbeaef-");
    let r4 = r2
        .replace(
            "22222222-2222-4222-8222-222222222222",
            "88888888-8888-4888-8888-888888888888",
        )
        .replacen("(at 110 50)", "(at 110 58)", 1)
        .replace("\"Reference\" \"R2\"", "\"Reference\" \"R4\"")
        .replace("611a5e55-", "611a5e56-")
        .replace("10e20b3d-", "10e20b3e-");
    let end = text.find(r2).unwrap() + r2.len();
    format!("{}{r3}{r4}{}", &text[..end], &text[end..])
}

/// `text` with the F.Cu GND pour filled by one rectangle per `(x0, y0, x1, y1)`.
fn with_f_fills(text: &str, boxes: &[(f64, f64, f64, f64)]) -> String {
    let polygons: String = boxes
        .iter()
        .map(|(x0, y0, x1, y1)| {
            format!("\n\t\t(filled_polygon (layer \"F.Cu\") (pts (xy {x0} {y0}) (xy {x1} {y0}) (xy {x1} {y1}) (xy {x0} {y1})))")
        })
        .collect();
    let at = text.find(F_ZONE_UUID).unwrap();
    let end = text[at..].find("\n\t)\n").unwrap() + at;
    format!("{}{polygons}{}", &text[..end], &text[end..])
}

#[test]
fn a_tie_leaves_every_pad_of_the_net_open_on_both_sides() {
    let text = six_pads();
    let names = PadNames::from_board(&text).unwrap();
    let all = set(&["R1-2", "R2-1", "R2-2", "R3-2", "R4-1", "R4-2"]);
    let at = Path::new("/scratch/r0/board.kicad_pcb");
    // 3 and 3: no group holds a strict majority, so KiCad's side opens all six.
    let split = with_f_fills(
        &text,
        &[(99.8, 48.0, 111.0, 52.0), (99.8, 56.0, 111.0, 60.0)],
    );
    let open = kicad_open_pads(at, &split, &names, &set(&["GND"])).unwrap();
    assert_eq!(open, all);
    // The engine's rule gives the same: every pin open, none connected.
    let mut object = planes_example();
    object["connected_pads"] = json!([]);
    object["open_pads"] = json!(all.iter().collect::<Vec<_>>());
    let engine = planes_object(&object).unwrap();
    assert_eq!(compare_pads(&engine, &open, 1), (Some(0), Some(0)));
    // Control: 4 and 2 (a strip joins R3's GND pad to the top group): a majority, so
    // only the two are open.
    let four = with_f_fills(
        &text,
        &[
            (99.8, 48.0, 111.0, 52.0),
            (100.2, 48.0, 100.8, 60.0),
            (108.0, 56.0, 111.0, 60.0),
        ],
    );
    let open = kicad_open_pads(at, &four, &names, &set(&["GND"])).unwrap();
    assert_eq!(open, set(&["R4-1", "R4-2"]));
}

#[test]
fn the_board_minimum_clearance_raises_each_zone() {
    let text = board(false).replace(
        "(connect_pads\n\t\t\t(clearance 0.5)",
        "(connect_pads\n\t\t\t(clearance 0.2)",
    );
    let (dsn, manifest) = exported(&text);
    let project = |min: f64| {
        json!({"board": {"design_settings": {"rules": {"min_clearance": min}}}}).to_string()
    };
    let clearances = |plan: &ZonePlan| -> Vec<f64> {
        plan.zones
            .iter()
            .map(|zone| zone.settings.clearance_mm)
            .collect()
    };
    let raised = zone_plan(&text, Some(&manifest), &dsn, Some(&project(0.3)), None).unwrap();
    assert_eq!(clearances(&raised), [0.3, 0.3]);
    assert!(file_text(&raised)
        .unwrap()
        .contains("zone 0 clearance_mm=0.3 "));
    // Controls: a lower board minimum, and none, leave the zone's own 0.2.
    let lower = zone_plan(&text, Some(&manifest), &dsn, Some(&project(0.1)), None).unwrap();
    assert_eq!(clearances(&lower), [0.2, 0.2]);
    let none = zone_plan(&text, Some(&manifest), &dsn, None, None).unwrap();
    assert_eq!(clearances(&none), [0.2, 0.2]);
}

/// R1's VCC pad (not a pour net) as a custom pad with `options`.
fn custom_vcc_pad(text: &str, options: &str) -> String {
    let pad = "(pad \"1\" smd roundrect\n\t\t\t(at -0.5 0)\n\t\t\t(size 0.6 0.5)\n\t\t\t(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")\n\t\t\t(roundrect_rratio 0.25)\n\t\t\t(net \"VCC\")";
    assert_eq!(text.matches(pad).count(), 1);
    text.replace(
        pad,
        &format!(
            "(pad \"1\" smd custom\n\t\t\t(at -0.5 0)\n\t\t\t(size 0.4 0.4)\n\t\t\t(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")\n\t\t\t(options {options} (anchor circle))\n\t\t\t(primitives (gr_poly (pts (xy -0.3 -0.2) (xy 0.3 -0.2) (xy 0.3 0.2)) (width 0) (fill yes)))\n\t\t\t(net \"VCC\")"
        ),
    )
}

#[test]
fn a_convex_hull_custom_pad_on_a_modelled_layer_uncovers_the_board() {
    let text = board(false);
    const HULL: &str = "convex hull on a modelled zone's layer";
    let hull = custom_vcc_pad(&text, "(clearance convexhull)");
    let plan = plan_with(&text, &hull, None);
    assert!(plan.zones.is_empty(), "{:?}", plan.zones);
    assert!(
        reasons(&plan).iter().all(|r| r.contains(HULL)),
        "{:?}",
        reasons(&plan)
    );
    // Controls: the same pad knocked out by its outline; and the hull pad when F.Cu's
    // pour is not modelled (a hatch fill), so no modelled zone is on its layer.
    let outline = custom_vcc_pad(&text, "(clearance outline)");
    let plan = plan_with(&text, &outline, None);
    assert!(
        reasons(&plan).iter().all(|r| !r.contains(HULL)),
        "{:?}",
        reasons(&plan)
    );
    let hatch = with_f_fill(
        &hull,
        "(fill yes (mode hatch) (thermal_gap 0.5) (thermal_bridge_width 0.5) (island_removal_mode 0))",
    );
    let plan = plan_with(&text, &hatch, None);
    assert!(
        reasons(&plan).iter().all(|r| !r.contains(HULL)),
        "{:?}",
        reasons(&plan)
    );
}

/// `text` with `extra` added to the pad whose uuid is `uuid`.
fn pad_with(text: &str, uuid: &str, extra: &str) -> String {
    let anchor = format!("(uuid \"{uuid}\")");
    assert_eq!(text.matches(&anchor).count(), 1);
    text.replace(&anchor, &format!("{extra}\n\t\t\t{anchor}"))
}

const R1_VCC: &str = "fb481b08-0972-41e5-a5d9-a4bab9f165c1";
const R2_GND: &str = "611a5e55-7fda-4eb1-9177-2b7af8a58dba";

#[test]
fn a_pads_layered_values_override_or_backdrill_uncovers_the_board() {
    let text = board(false);
    let thermal = "(padstack (mode front_inner_back) (layer \"Inner\" (thermal_gap 0.3)))";
    let clearance = "(padstack (mode front_inner_back) (layer \"Inner\" (clearance 0.3)))";
    for (variant, words) in [
        (
            pad_with(&text, R2_GND, thermal),
            "pour net with layer-specific connection or thermal values",
        ),
        (
            pad_with(&text, R1_VCC, clearance),
            "pads with a layer-specific clearance",
        ),
        (
            pad_with(&text, R2_GND, "(zone_layer_connections \"F.Cu\")"),
            "override their zone connection per layer",
        ),
        (
            pad_with(
                &text,
                R2_GND,
                "(backdrill (size 0.4) (layers \"B.Cu\" \"F.Cu\"))",
            ),
            "backdrilled or post-machined",
        ),
    ] {
        let plan = plan_with(&text, &variant, None);
        assert!(plan.zones.is_empty(), "{words}: {:?}", plan.zones);
        assert!(
            reasons(&plan).iter().all(|r| r.contains(words)),
            "{words}: {:?}",
            reasons(&plan)
        );
    }
    // Control: layered thermal values on a pad of a net with no pour change no zone.
    let plan = plan_with(&text, &pad_with(&text, R1_VCC, thermal), None);
    assert_eq!(modelled(&plan).len(), 2, "{:?}", reasons(&plan));
}

#[test]
fn a_teardrop_or_keepout_that_does_not_read_uncovers_the_board() {
    let text = board(false);
    let teardrop = pad_with(&text, R1_VCC, "(teardrops (max_length abc) (enabled yes))");
    let plan = plan_with(&text, &teardrop, None);
    assert!(plan.zones.is_empty());
    assert!(
        reasons(&plan)[0].contains("a teardrop setting cannot be read"),
        "{:?}",
        reasons(&plan)
    );
    // A rule area that keeps pours out, with two outlines the file cannot carry as one.
    let area = |polygons: &str| {
        text.replacen(
            "\t(zone\n",
            &format!(
                "\t(zone (layer \"B.Cu\") (uuid \"99999999-9999-4999-8999-999999999999\") (hatch edge 0.5) (connect_pads (clearance 0)) (min_thickness 0.25) (keepout (tracks allowed) (vias allowed) (pads allowed) (copperpour not_allowed) (footprints allowed)) {polygons})\n\t(zone\n"
            ),
            1,
        )
    };
    let square = "(polygon (pts (xy 90 40) (xy 92 40) (xy 92 42) (xy 90 42)))";
    let two = area(&format!(
        "{square} (polygon (pts (xy 95 40) (xy 97 40) (xy 97 42) (xy 95 42)))"
    ));
    let plan = plan_with(&text, &two, None);
    assert!(plan.zones.is_empty());
    assert!(
        reasons(&plan)[0].contains("outline the zones file cannot carry"),
        "{:?}",
        reasons(&plan)
    );
    // Controls: a teardrop that reads, and the rule area with one outline: a keepout line.
    let plan = plan_with(
        &text,
        &pad_with(&text, R1_VCC, "(teardrops (max_length 0.8) (enabled yes))"),
        None,
    );
    assert_eq!((modelled(&plan).len(), plan.teardrop_mm), (2, 0.8));
    let plan = plan_with(&text, &area(square), None);
    assert_eq!(modelled(&plan).len(), 2);
    assert_eq!(plan.keepouts.len(), 1);
}

/// One pad and its footprint, parsed.
fn pad_node(pad: &str, footprint: &str) -> (SexpNode, SexpNode) {
    (
        konnect_sexp::parse_sexp(pad).unwrap(),
        konnect_sexp::parse_sexp(footprint).unwrap(),
    )
}

fn thermal_zone() -> FillSettings {
    fill_settings(&konnect_sexp::parse_sexp("(zone (min_thickness 0.25))").unwrap()).unwrap()
}

#[test]
fn a_custom_pad_with_a_round_anchor_takes_45_degrees_after_kicad_6() {
    let (pad, footprint) = pad_node(
        "(pad \"1\" smd custom (at 0 0) (size 1 1) (layers \"F.Cu\") (options (clearance outline) (anchor circle)))",
        "(footprint \"x\")",
    );
    let angle = |version| {
        pad_values(&pad, &footprint, &thermal_zone(), version)
            .unwrap()
            .3
    };
    assert_eq!(angle(Some(20211014)), 0.0, "KiCad 6.0 files: 90, written 0");
    assert_eq!(angle(Some(20211015)), 45.0);
    assert_eq!(angle(None), 45.0);
    // Control: a rectangular anchor is 90 (0) in any version.
    let (rect, _) = pad_node(
        "(pad \"1\" smd custom (at 0 0) (size 1 1) (layers \"F.Cu\") (options (clearance outline) (anchor rect)))",
        "(footprint \"x\")",
    );
    assert_eq!(
        pad_values(&rect, &footprint, &thermal_zone(), Some(20211015))
            .unwrap()
            .3,
        0.0
    );
}

#[test]
fn the_legacy_thermal_width_token_is_a_pads_bridge() {
    let footprint = konnect_sexp::parse_sexp("(footprint \"x\")").unwrap();
    let bridge = |pad: &str| {
        pad_values(
            &konnect_sexp::parse_sexp(pad).unwrap(),
            &footprint,
            &thermal_zone(),
            None,
        )
        .unwrap()
        .2
    };
    assert_eq!(
        bridge("(pad \"1\" smd rect (at 0 0) (size 1 1) (layers \"F.Cu\") (thermal_width 0.4))"),
        0.4
    );
    assert_eq!(
        bridge("(pad \"1\" smd rect (at 0 0) (size 1 1) (layers \"F.Cu\") (thermal_bridge_width 0.3) (thermal_width 0.4))"),
        0.3,
        "the newer token wins"
    );
    // Control: neither token, the zone's bridge.
    assert_eq!(
        bridge("(pad \"1\" smd rect (at 0 0) (size 1 1) (layers \"F.Cu\"))"),
        0.5
    );
}

/// The base board with R2 also called R1 (exported as `R1_DUP2`) and both its pads
/// numbered 1 (pins `1@1` and `1@2`), its second pad with its own spoke angle.
fn repeated_names() -> String {
    let text = board(false).replace("\"Reference\" \"R2\"", "\"Reference\" \"R1\"");
    let r2 = footprint_block(&text, "22222222-2222-4222-8222-222222222222");
    let renumbered = r2.replace("(pad \"2\" smd", "(pad \"1\" smd");
    let text = text.replace(r2, &renumbered);
    pad_with(
        &text,
        "10e20b3d-431e-4a39-a976-77389855efa1",
        "(thermal_bridge_angle 30)",
    )
}

#[test]
fn repeated_references_and_pad_numbers_keep_the_dsn_names() {
    let text = repeated_names();
    let bundle = crate::specctra::export_dsn_with_options(
        Path::new("/scratch/specctra_rounded_board.kicad_pcb"),
        &text,
        &rules(),
        crate::specctra::ExportOptions {
            allow_duplicate_references: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(bundle.dsn.contains("R1_DUP2-1@2"), "{}", bundle.dsn);
    let manifest: Value = serde_json::from_str(&bundle.manifest).unwrap();
    let plan = zone_plan(&text, Some(&manifest), &bundle.dsn, None, None).unwrap();
    assert_eq!(modelled(&plan).len(), 2, "{:?}", reasons(&plan));
    let lines: Vec<(&str, &str)> = plan
        .pads
        .iter()
        .map(|pad| (pad.component.as_str(), pad.pin.as_str()))
        .collect();
    assert_eq!(lines, [("R1_DUP2", "1@2")]);
    assert!(file_text(&plan)
        .unwrap()
        .contains("\npad R1_DUP2 1@2 connect=thermal gap_mm=0.5 bridge_mm=0.5 angle_deg=30\n"));
    // The comparison names the same pads the same way: R2's two pads on the fill, R1's
    // GND pad off it.
    let refilled = with_f_fills(&text, &[(108.0, 48.0, 112.0, 52.0)]);
    let open = kicad_open_pads(
        Path::new("/scratch/r0/board.kicad_pcb"),
        &refilled,
        &plan.names,
        &set(&["GND"]),
    )
    .unwrap();
    assert_eq!(open, set(&["R1-2"]));
    // R1's GND pad joined to R2's first pad: R2's second (`1@2`) is the one left.
    let refilled = with_f_fills(&text, &[(100.2, 48.0, 109.8, 52.0)]);
    let open = kicad_open_pads(
        Path::new("/scratch/r0/board.kicad_pcb"),
        &refilled,
        &plan.names,
        &set(&["GND"]),
    )
    .unwrap();
    assert_eq!(open, set(&["R1_DUP2-1@2"]));
}

#[test]
fn a_reference_with_a_space_is_written_quoted() {
    let text = pad_with(
        &board(false).replace("\"Reference\" \"R2\"", "\"Reference\" \"R 2\""),
        R2_GND,
        "(thermal_bridge_angle 30)",
    );
    let plan = plan(&text);
    assert_eq!(modelled(&plan).len(), 2, "{:?}", reasons(&plan));
    assert!(
        file_text(&plan)
            .unwrap()
            .contains("\npad \"R 2\" 1 connect=thermal gap_mm=0.5 bridge_mm=0.5 angle_deg=30\n"),
        "{}",
        file_text(&plan).unwrap()
    );
}

#[test]
fn a_pad_line_with_an_empty_name_is_never_written() {
    let text = board(true);
    let tree = konnect_sexp::parse_sexp(&text).unwrap();
    let copper = crate::specctra::board_copper_layers(&tree).unwrap();
    let zones = plan(&text).zones;
    let pins = crate::specctra::pad_pins(&tree).unwrap();
    // Control: as exported, R1's round pad gets its line and nothing is demoted.
    let (lines, demoted) = pad_lines(&tree, &pins, &zones, &copper);
    assert_eq!(lines.len(), 1);
    assert!(demoted.is_empty());
    for empty in ["component", "pin"] {
        let mut named = pins.clone();
        let round = named
            .iter_mut()
            .find(|pin| pin.component == "R1" && pin.pin == "2")
            .unwrap();
        if empty == "component" {
            round.component.clear();
        } else {
            round.pin.clear();
        }
        let (lines, demoted) = pad_lines(&tree, &named, &zones, &copper);
        assert!(lines.is_empty(), "{empty}: {lines:?}");
        assert!(
            demoted["GND"].contains("empty component or pin name"),
            "{empty}: {demoted:?}"
        );
    }
}

/// The rule area of `keepout_arc_board` and its arc: from (112, 52) bulging to x 121 at
/// y 57 and back to (112, 62), the rest of the square straight.
fn keepout_arc_board() -> String {
    board(false)
        .replace("(copperpour allowed)", "(copperpour not_allowed)")
        .replace(
            "(xy 112 52) (xy 120 52) (xy 120 62) (xy 112 62)",
            "(xy 112 52) (arc (start 117 52) (mid 122 57) (end 117 62)) (xy 112 62)",
        )
}

#[test]
fn a_keepout_arc_is_circumscribed() {
    let text = keepout_arc_board();
    assert!(text.contains("(mid 122 57)"));
    let plan = plan(&text);
    assert_eq!(plan.keepouts.len(), 1);
    let corners = &plan.keepouts[0].corners;
    // In DSN micrometres the arc's centre is (117000, -57000), its radius 5000.
    let inside = |(px, py): (f64, f64)| {
        let mut inside = false;
        for i in 0..corners.len() {
            let (x0, y0) = (corners[i].0 as f64, corners[i].1 as f64);
            let (x1, y1) = {
                let next = corners[(i + 1) % corners.len()];
                (next.0 as f64, next.1 as f64)
            };
            if (y0 > py) != (y1 > py) && px < x0 + (py - y0) * (x1 - x0) / (y1 - y0) {
                inside = !inside;
            }
        }
        inside
    };
    // Every point of the arc's open half, 0.1 um inside it, is inside the keepout.
    for k in 1..400 {
        let theta = -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * k as f64 / 400.0;
        let r = 5000.0 - 0.1;
        let p = (117000.0 + r * theta.cos(), -57000.0 + r * theta.sin());
        assert!(
            inside(p),
            "the arc point {p:?} is outside the keepout {corners:?}"
        );
    }
    // And nothing goes more than the tolerance and the margin beyond the arc.
    for &(x, y) in corners {
        let distance = ((x as f64 - 117000.0).powi(2) + (y as f64 + 57000.0).powi(2)).sqrt();
        if x > 117000 {
            assert!(distance <= 5000.0 + 1.0 + 5.0 + 1.0, "{distance}");
        }
    }
}

#[test]
fn a_pad_the_dsn_and_kicad_see_differently_round_demotes_its_net() {
    let text = board(false);
    let r1_gnd = "(pad \"2\" smd roundrect\n\t\t\t(at 0.5 0)\n\t\t\t(size 0.6 0.5)\n\t\t\t(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")\n\t\t\t(roundrect_rratio 0.25)\n\t\t\t(net \"GND\")\n\t\t\t(uuid \"80cbeaee";
    assert_eq!(text.matches(r1_gnd).count(), 1);
    let oval = |size: &str| {
        text.replace(
            r1_gnd,
            &format!("(pad \"2\" smd oval\n\t\t\t(at 0.5 0)\n\t\t\t(size {size})\n\t\t\t(layers \"F.Cu\" \"F.Mask\" \"F.Paste\")\n\t\t\t(net \"GND\")\n\t\t\t(uuid \"80cbeaee"),
        )
    };
    // 0.5 by 0.5001 mm: one size in whole micrometres, so the DSN writes a circle, and
    // not equal sides to KiCad's filler.
    let plan = plan_with(&text, &oval("0.5 0.5001"), None);
    assert!(plan.zones.is_empty(), "{:?}", plan.zones);
    assert!(
        reasons(&plan)
            .iter()
            .all(|r| r.contains("pad R1-2: the DSN writes it as a circle")),
        "{:?}",
        reasons(&plan)
    );
    // The other way: a rectangular pad whose F.Cu padstack layer is a circle.
    let layered = pad_with(
        &text,
        R2_GND,
        "(padstack (mode front_inner_back) (layer \"F.Cu\" (shape circle) (size 0.5 0.5)))",
    );
    let plan = plan_with(&text, &layered, None);
    assert!(plan.zones.is_empty());
    assert!(
        reasons(&plan)
            .iter()
            .all(|r| r.contains("KiCad's filler takes it as round")),
        "{:?}",
        reasons(&plan)
    );
    // Control: an oval of exactly equal sides is round to both.
    let plan = plan_with(&text, &oval("0.5 0.5"), None);
    assert_eq!(modelled(&plan).len(), 2, "{:?}", reasons(&plan));
}

#[test]
fn track_to_track_teardrops_uncover_the_board() {
    let text = board(false);
    let (dsn, manifest) = exported(&text);
    const WORDS: &str = "track-to-track teardrops";
    let project = |on: bool| {
        json!({"board": {"design_settings": {"teardrop_options": [
            {"td_onvia": true, "td_onpthpad": true, "td_onsmdpad": true, "td_ontrackend": on}
        ]}}})
        .to_string()
    };
    let plan = zone_plan(&text, Some(&manifest), &dsn, Some(&project(true)), None).unwrap();
    assert!(plan.zones.is_empty());
    assert!(
        reasons(&plan).iter().all(|r| r.contains(WORDS)),
        "{:?}",
        reasons(&plan)
    );
    // A track-end teardrop zone already on the board says the same.
    let teardrop = |kind: &str| {
        text.replacen(
            "\t(zone\n",
            &format!(
                "\t(zone (net \"GND\") (layer \"F.Cu\") (uuid \"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\") (hatch edge 0.5) (connect_pads yes (clearance 0)) (min_thickness 0.0254) (attr (teardrop (type {kind}))) (fill yes) (polygon (pts (xy 100 51) (xy 100.2 51) (xy 100.2 51.2))))\n\t(zone\n"
            ),
            1,
        )
    };
    let plan = plan_with(&text, &teardrop("track_end"), None);
    assert!(plan.zones.is_empty());
    assert!(
        reasons(&plan).iter().all(|r| r.contains(WORDS)),
        "{:?}",
        reasons(&plan)
    );
    // Controls: the option off, and a pad teardrop zone.
    let plan = zone_plan(&text, Some(&manifest), &dsn, Some(&project(false)), None).unwrap();
    assert_eq!(modelled(&plan).len(), 2, "{:?}", reasons(&plan));
    let plan = plan_with(&text, &teardrop("padvia"), None);
    assert_eq!(modelled(&plan).len(), 2, "{:?}", reasons(&plan));
}
