//! Row RW: the messages a live netclass write sends, and the checks on what
//! KiCad holds afterwards. The served path is in `project_file_writer_tests`.

use super::*;

fn class(name: &str, priority: i32, fields: &[(Field, i64)]) -> NetClass {
    let mut class = NetClass {
        name: name.to_string(),
        priority: Some(priority),
        // As KiCad 10.0.1 serialises every class (module docs).
        r#type: NetClassType::NctImplicit as i32,
        constituents: vec![name.to_string()],
        ..Default::default()
    };
    for (field, nm) in fields {
        field.set(&mut class, *nm);
    }
    class
}

fn defaults() -> Vec<(Field, i64)> {
    vec![
        (Field::Clearance, 200_000),
        (Field::TrackWidth, 250_000),
        (Field::ViaDrill, 400_000),
        (Field::ViaDiameter, 800_000),
    ]
}

#[test]
fn every_field_reads_back_what_it_sets() {
    for (index, field) in ALL_FIELDS.into_iter().enumerate() {
        let mut message = NetClass::default();
        assert_eq!(field.get(&message), None, "{field:?}");
        let nm = 100_000 + index as i64;
        field.set(&mut message, nm);
        assert_eq!(field.get(&message), Some(nm), "{field:?}");
        // Setting one field leaves every other one unset.
        for other in ALL_FIELDS.into_iter().filter(|other| *other != field) {
            assert_eq!(other.get(&message), None, "{field:?} set {other:?}");
        }
    }
}

#[test]
fn an_existing_class_is_sent_with_only_the_named_fields_and_its_own_priority() {
    let kicad = vec![
        class("Default", i32::MAX, &defaults()),
        class(
            "Power",
            3,
            &[(Field::TrackWidth, 400_000), (Field::Clearance, 300_000)],
        ),
    ];
    let (message, existed) = class_message(
        &kicad,
        "Power",
        &[(Field::TrackWidth, 500_000)],
        &defaults(),
    );
    assert!(existed);
    assert_eq!(message.priority, Some(3));
    assert_eq!(Field::TrackWidth.get(&message), Some(500_000));
    for field in ALL_FIELDS
        .into_iter()
        .filter(|field| *field != Field::TrackWidth)
    {
        assert_eq!(field.get(&message), None, "{field:?} must not be sent");
    }
}

#[test]
fn a_class_kicad_lacks_is_sent_sparse_with_creation_defaults() {
    let kicad = vec![class("Default", i32::MAX, &defaults())];
    let (message, existed) =
        class_message(&kicad, "HV", &[(Field::Clearance, 1_000_000)], &defaults());
    assert!(!existed);
    assert_eq!(message.priority, Some(-1));
    assert_eq!(Field::Clearance.get(&message), Some(1_000_000));
    assert_eq!(Field::TrackWidth.get(&message), Some(250_000));
    assert_eq!(Field::DiffPairGap.get(&message), None);
}

/// The guard against KiCad's silent skip: a class sent as NCT_IMPLICIT is not
/// applied (`netclass.cpp:220`).
#[test]
fn a_sent_class_is_never_implicit_and_names_no_constituents() {
    let kicad = vec![class("Default", i32::MAX, &defaults())];
    for name in ["Default", "New"] {
        let (message, _) = class_message(&kicad, name, &[(Field::TrackWidth, 1)], &defaults());
        assert_eq!(message.r#type, NetClassType::NctExplicit as i32, "{name}");
        assert!(message.constituents.is_empty(), "{name}");
    }
}

#[test]
fn verify_accepts_what_was_sent_and_refuses_what_kicad_did_not_take() {
    let before = vec![
        class("Default", i32::MAX, &defaults()),
        class("Power", 3, &[(Field::TrackWidth, 400_000)]),
    ];
    let named = [(Field::TrackWidth, 500_000)];
    let (message, existed) = class_message(&before, "Power", &named, &defaults());
    let mut after = before.clone();
    Field::TrackWidth.set(&mut after[1], 500_000);
    assert_eq!(
        verify(&before, &after, &message, &named, &defaults(), existed),
        Ok(())
    );

    // KiCad answered, but its copy still holds the old width.
    let unchanged = before.clone();
    let refusal = verify(&before, &unchanged, &message, &named, &defaults(), existed).unwrap_err();
    assert!(refusal.contains("track_width"), "{refusal}");

    // The class is gone.
    let gone = vec![before[0].clone()];
    assert!(verify(&before, &gone, &message, &named, &defaults(), existed).is_err());

    // Its priority moved.
    let mut moved = after.clone();
    moved[1].priority = Some(0);
    assert!(
        verify(&before, &moved, &message, &named, &defaults(), existed)
            .unwrap_err()
            .contains("priority")
    );

    // Another class changed at the same time.
    let mut other = after.clone();
    Field::Clearance.set(&mut other[0], 1);
    assert!(
        verify(&before, &other, &message, &named, &defaults(), existed)
            .unwrap_err()
            .contains("other netclasses")
    );
}

#[test]
fn verify_checks_a_new_class_holds_its_defaults_too() {
    let before = vec![class("Default", i32::MAX, &defaults())];
    let named = [(Field::Clearance, 1_000_000)];
    let (message, existed) = class_message(&before, "HV", &named, &defaults());
    let mut held = message.clone();
    held.r#type = NetClassType::NctImplicit as i32;
    let after = vec![before[0].clone(), held.clone()];
    assert_eq!(
        verify(&before, &after, &message, &named, &defaults(), existed),
        Ok(())
    );
    // KiCad took the named clearance but not the default width.
    let mut partial = held;
    partial.board.as_mut().unwrap().track_width = None;
    let after = vec![before[0].clone(), partial];
    assert!(
        verify(&before, &after, &message, &named, &defaults(), existed)
            .unwrap_err()
            .contains("track_width")
    );
}

#[test]
fn unsaved_differences_name_every_field_and_class_the_file_lacks() {
    let kicad = vec![
        class("Default", i32::MAX, &[(Field::TrackWidth, 400_000)]),
        class("OnlyKiCad", 1, &[]),
    ];
    let file = vec![
        json!({ "name": "Default", "track_width": 0.2 }),
        json!({ "name": "OnlyFile", "track_width": 0.3 }),
    ];
    let differences = unsaved_differences(&kicad, &file);
    assert_eq!(
        differences,
        vec![
            json!({ "netclass": "Default", "field": "track_width", "kicad_mm": 0.4, "project_file_mm": 0.2 }),
            json!({ "netclass": "OnlyKiCad", "only_in": "kicad" }),
            json!({ "netclass": "OnlyFile", "only_in": "project_file" }),
        ]
    );
    assert!(unsaved_note(&differences).unwrap().contains("3 netclass"));

    // The same values on both sides, written as KiCad writes mm.
    let file = vec![
        json!({ "name": "Default", "track_width": 0.4 }),
        json!({ "name": "OnlyKiCad" }),
    ];
    let equal = unsaved_differences(&kicad, &file);
    assert!(equal.is_empty(), "{equal:?}");
    assert_eq!(unsaved_note(&equal), None);
}

#[test]
fn values_kicad_cannot_hold_are_refused() {
    assert!(check_mm("trace_width", Field::TrackWidth, 0.5).is_ok());
    assert!(check_mm("trace_width", Field::TrackWidth, 100.0).is_ok());
    assert!(check_mm("clearance", Field::Clearance, 0.0).is_ok());
    for bad in [0.0, -0.1, 100.001, f64::NAN, f64::INFINITY] {
        assert!(
            check_mm("trace_width", Field::TrackWidth, bad).is_err(),
            "{bad}"
        );
    }
    assert!(check_mm("clearance", Field::Clearance, -0.001).is_err());
    assert_eq!(mm_to_nm(0.25), 250_000);
    assert_eq!(mm_to_nm(0.1 + 0.2), 300_000);
}

#[test]
fn a_priority_the_file_writes_differently_is_named() {
    let kicad = vec![class("Power", 3, &[])];
    let file = vec![json!({ "name": "Power", "priority": 5 })];
    assert_eq!(
        unsaved_differences(&kicad, &file),
        vec![json!({ "netclass": "Power", "field": "priority", "kicad": 3, "project_file": 5 })]
    );
    // The control: the same priority, or a file that writes none.
    assert!(unsaved_differences(&kicad, &[json!({ "name": "Power", "priority": 3 })]).is_empty());
    assert!(unsaved_differences(&kicad, &[json!({ "name": "Power" })]).is_empty());
}
