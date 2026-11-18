//! Tests of `place_held`, and the small boards every placement-search test builds
//! (the builder is `pub(crate)` so that `place_generators_tests` and
//! `place_judge_tests` use the same one).

use super::*;
use konnect_sexp::board_geometry::Allowlist;

// ─── The board builder ───────────────────────────────────────────────────────

/// One pad of a test footprint: number, local position, net (`None` is unconnected).
#[derive(Clone)]
pub(crate) struct Pad {
    number: String,
    x: f64,
    y: f64,
    net: Option<String>,
}

pub(crate) fn pad(number: &str, x: f64, y: f64, net: Option<&str>) -> Pad {
    Pad {
        number: number.to_string(),
        x,
        y,
        net: net.map(str::to_string),
    }
}

/// One footprint of a test board.
#[derive(Clone)]
pub(crate) struct Fp {
    pub reference: String,
    lib_id: String,
    value: String,
    pub x: f64,
    pub y: f64,
    rotation: f64,
    back: bool,
    locked: bool,
    /// Half-width and half-height of a courtyard rectangle centred on the anchor.
    court: Option<(f64, f64)>,
    /// An off-centre courtyard rectangle `(x0, y0, x1, y1)`, local, instead of `court`.
    court_rect: Option<(f64, f64, f64, f64)>,
    pads: Vec<Pad>,
    path: Option<String>,
    broken_courtyard: bool,
}

impl Fp {
    pub(crate) fn new(reference: &str, x: f64, y: f64) -> Self {
        Fp {
            reference: reference.to_string(),
            lib_id: "Test:Part".to_string(),
            value: "10k".to_string(),
            x,
            y,
            rotation: 0.0,
            back: false,
            locked: false,
            court: Some((1.0, 0.5)),
            court_rect: None,
            pads: Vec::new(),
            path: None,
            broken_courtyard: false,
        }
    }
    pub(crate) fn lib(mut self, id: &str) -> Self {
        self.lib_id = id.to_string();
        self
    }
    pub(crate) fn value(mut self, value: &str) -> Self {
        self.value = value.to_string();
        self
    }
    pub(crate) fn rot(mut self, degrees: f64) -> Self {
        self.rotation = degrees;
        self
    }
    pub(crate) fn back(mut self) -> Self {
        self.back = true;
        self
    }
    pub(crate) fn locked(mut self) -> Self {
        self.locked = true;
        self
    }
    pub(crate) fn court(mut self, half_w: f64, half_h: f64) -> Self {
        self.court = Some((half_w, half_h));
        self
    }
    pub(crate) fn court_rect(mut self, x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        self.court_rect = Some((x0, y0, x1, y1));
        self
    }
    pub(crate) fn no_court(mut self) -> Self {
        self.court = None;
        self
    }
    pub(crate) fn path(mut self, path: &str) -> Self {
        self.path = Some(path.to_string());
        self
    }
    pub(crate) fn pad(mut self, pad: Pad) -> Self {
        self.pads.push(pad);
        self
    }
    /// A courtyard line with a missing coordinate: the courtyard scan skips it.
    pub(crate) fn broken_courtyard(mut self) -> Self {
        self.broken_courtyard = true;
        self
    }
    /// Pads "1" and "2" at local (-0.5, 0) and (0.5, 0).
    pub(crate) fn two_pad(self, net1: &str, net2: &str) -> Self {
        self.pad(pad("1", -0.5, 0.0, Some(net1)))
            .pad(pad("2", 0.5, 0.0, Some(net2)))
    }
}

/// The Edge.Cuts outline of a test board.
#[derive(Clone, Copy)]
pub(crate) enum Shape {
    Rect(f64, f64, f64, f64),
    /// A rectangle with arcs of radius `r` at its four corners.
    Rounded(f64, f64, f64, f64, f64),
    /// A rectangle with one side missing: an open chain.
    Open(f64, f64, f64, f64),
}

fn line(a: (f64, f64), b: (f64, f64), n: usize) -> String {
    format!(
        "(gr_line (start {} {}) (end {} {}) (stroke (width 0.05) (type default)) (layer \"Edge.Cuts\") (uuid \"11111111-0000-4000-8000-{n:012}\"))\n",
        a.0, a.1, b.0, b.1
    )
}

fn arc(a: (f64, f64), m: (f64, f64), b: (f64, f64), n: usize) -> String {
    format!(
        "(gr_arc (start {} {}) (mid {} {}) (end {} {}) (stroke (width 0.05) (type default)) (layer \"Edge.Cuts\") (uuid \"11111111-0000-4000-8000-{n:012}\"))\n",
        a.0, a.1, m.0, m.1, b.0, b.1
    )
}

fn outline_text(shape: Shape) -> String {
    match shape {
        Shape::Rect(x0, y0, x1, y1) => [
            line((x0, y0), (x1, y0), 1),
            line((x1, y0), (x1, y1), 2),
            line((x1, y1), (x0, y1), 3),
            line((x0, y1), (x0, y0), 4),
        ]
        .concat(),
        Shape::Open(x0, y0, x1, y1) => [
            line((x0, y0), (x1, y0), 1),
            line((x1, y0), (x1, y1), 2),
            line((x1, y1), (x0, y1), 3),
        ]
        .concat(),
        Shape::Rounded(x0, y0, x1, y1, r) => {
            let k = r * (1.0 - std::f64::consts::FRAC_1_SQRT_2);
            [
                line((x0 + r, y0), (x1 - r, y0), 1),
                arc((x1 - r, y0), (x1 - k, y0 + k), (x1, y0 + r), 2),
                line((x1, y0 + r), (x1, y1 - r), 3),
                arc((x1, y1 - r), (x1 - k, y1 - k), (x1 - r, y1), 4),
                line((x1 - r, y1), (x0 + r, y1), 5),
                arc((x0 + r, y1), (x0 + k, y1 - k), (x0, y1 - r), 6),
                line((x0, y1 - r), (x0, y0 + r), 7),
                arc((x0, y0 + r), (x0 + k, y0 + k), (x0 + r, y0), 8),
            ]
            .concat()
        }
    }
}

/// A uuid for footprint number `n` (0-based).
pub(crate) fn uuid_of(n: usize) -> String {
    format!("00000000-0000-4000-8000-{:012}", n + 1)
}

fn footprint_text(n: usize, fp: &Fp) -> String {
    let (cu, mask, paste, court_layer) = if fp.back {
        ("B.Cu", "B.Mask", "B.Paste", "B.CrtYd")
    } else {
        ("F.Cu", "F.Mask", "F.Paste", "F.CrtYd")
    };
    let at = if fp.rotation == 0.0 {
        format!("(at {} {})", fp.x, fp.y)
    } else {
        format!("(at {} {} {})", fp.x, fp.y, fp.rotation)
    };
    let mut text = String::new();
    text.push_str(&format!("(footprint \"{}\"\n", fp.lib_id));
    text.push_str(&format!("(layer \"{cu}\")\n"));
    text.push_str(&format!("(uuid \"{}\")\n", uuid_of(n)));
    text.push_str(&format!("{at}\n"));
    if fp.locked {
        text.push_str("(locked yes)\n");
    }
    if let Some(path) = &fp.path {
        text.push_str(&format!("(path \"{path}\")\n"));
    }
    if !fp.reference.is_empty() {
        text.push_str(&format!(
            "(property \"Reference\" \"{}\" (at 0 -1.5 0) (layer \"F.SilkS\") (uuid \"22222222-0000-4000-8000-{n:012}\") (effects (font (size 1 1) (thickness 0.15))))\n",
            fp.reference
        ));
    }
    text.push_str(&format!(
        "(property \"Value\" \"{}\" (at 0 1.5 0) (layer \"F.Fab\") (uuid \"33333333-0000-4000-8000-{n:012}\") (effects (font (size 1 1) (thickness 0.15))))\n",
        fp.value
    ));
    if fp.broken_courtyard {
        text.push_str(&format!(
            "(fp_line (start 1) (end 2 3) (stroke (width 0.05) (type default)) (layer \"{court_layer}\"))\n"
        ));
    } else if let Some((x0, y0, x1, y1)) = fp.court_rect {
        text.push_str(&format!(
            "(fp_rect (start {x0} {y0}) (end {x1} {y1}) (stroke (width 0.05) (type default)) (fill no) (layer \"{court_layer}\") (uuid \"44444444-0000-4000-8000-{n:012}\"))\n"
        ));
    } else if let Some((hw, hh)) = fp.court {
        text.push_str(&format!(
            "(fp_rect (start {} {}) (end {hw} {hh}) (stroke (width 0.05) (type default)) (fill no) (layer \"{court_layer}\") (uuid \"44444444-0000-4000-8000-{n:012}\"))\n",
            -hw, -hh
        ));
    }
    for (k, pad) in fp.pads.iter().enumerate() {
        let net = pad
            .net
            .as_ref()
            .map(|net| format!("(net \"{net}\")"))
            .unwrap_or_default();
        text.push_str(&format!(
            "(pad \"{}\" smd rect (at {} {}) (size 0.6 0.5) (layers \"{cu}\" \"{mask}\" \"{paste}\") {net} (uuid \"55555555-{k:04}-4000-8000-{n:012}\"))\n",
            pad.number, pad.x, pad.y
        ));
    }
    text.push_str(")\n");
    text
}

/// A KiCad 10 board of `parts` inside `shape`, with `(group …)` blocks naming
/// references.
pub(crate) fn board_with_groups(shape: Shape, parts: &[Fp], groups: &[(&str, &[&str])]) -> String {
    let mut text = String::from(
        "(kicad_pcb\n(version 20260206)\n(generator \"pcbnew\")\n(generator_version \"10.0\")\n(general (thickness 1.6) (legacy_teardrops no))\n(paper \"A4\")\n(layers (0 \"F.Cu\" signal) (2 \"B.Cu\" signal) (5 \"F.SilkS\" user \"F.Silkscreen\") (1 \"F.Mask\" user) (3 \"B.Mask\" user) (25 \"Edge.Cuts\" user) (31 \"F.CrtYd\" user \"F.Courtyard\") (29 \"B.CrtYd\" user \"B.Courtyard\"))\n(setup (pad_to_mask_clearance 0.05))\n",
    );
    for (n, fp) in parts.iter().enumerate() {
        text.push_str(&footprint_text(n, fp));
    }
    text.push_str(&outline_text(shape));
    for (k, (name, members)) in groups.iter().enumerate() {
        let uuids: Vec<String> = members
            .iter()
            .map(|member| {
                // `@name` is another group of the list, by name.
                if let Some(inner) = member.strip_prefix('@') {
                    let j = groups
                        .iter()
                        .position(|(other, _)| *other == inner)
                        .unwrap_or_else(|| panic!("{inner} is not a group of the test board"));
                    return format!("\"66666666-0000-4000-8000-{j:012}\"");
                }
                let n = parts
                    .iter()
                    .position(|fp| fp.reference == *member)
                    .unwrap_or_else(|| panic!("{member} is not on the test board"));
                format!("\"{}\"", uuid_of(n))
            })
            .collect();
        text.push_str(&format!(
            "(group \"{name}\" (uuid \"66666666-0000-4000-8000-{k:012}\") (members {}))\n",
            uuids.join(" ")
        ));
    }
    text.push_str(")\n");
    text
}

pub(crate) fn board(shape: Shape, parts: &[Fp]) -> String {
    board_with_groups(shape, parts, &[])
}

pub(crate) fn parse(text: &str) -> SexpNode {
    konnect_sexp::parse_sexp(text).expect("the test board parses")
}

/// `evaluate::evaluate` of `text` with the given held references as edge mounts.
pub(crate) fn evaluate_text(text: &str, edge_mount: &[&str]) -> serde_json::Value {
    crate::tools::placement::evaluate::evaluate(
        &parse(text),
        &Allowlist::new(edge_mount.iter().copied()),
        8,
    )
    .expect("the test board evaluates")
}

// ─── Fixtures of this file's tests ───────────────────────────────────────────

const OUTLINE: Shape = Shape::Rect(0.0, 0.0, 60.0, 40.0);

fn no_caller() -> BTreeSet<String> {
    BTreeSet::new()
}

/// A clean board: U1 and two passives that all connect, one connector at the edge.
fn clean_parts() -> Vec<Fp> {
    vec![
        Fp::new("U1", 20.0, 20.0)
            .court(3.0, 3.0)
            .pad(pad("1", -2.0, 0.0, Some("VCC")))
            .pad(pad("2", 2.0, 0.0, Some("GND"))),
        Fp::new("C1", 12.0, 20.0).two_pad("VCC", "GND"),
        Fp::new("R1", 28.0, 20.0).two_pad("VCC", "SIG"),
        Fp::new("J1", 56.0, 20.0)
            .court(2.0, 2.0)
            .pad(pad("1", 0.0, 0.0, Some("SIG"))),
    ]
}

fn set_of(parts: &[Fp]) -> HeldSet {
    held_set(&board(OUTLINE, parts), &no_caller(), &HoldRules::default()).expect("the set holds")
}

#[test]
fn lock_caller_connector_and_mechanical_parts_are_held() {
    let parts = vec![
        Fp::new("U1", 20.0, 20.0)
            .court(3.0, 3.0)
            .pad(pad("1", -2.0, 0.0, Some("VCC")))
            .pad(pad("2", 2.0, 0.0, Some("GND"))),
        Fp::new("R1", 30.0, 20.0).two_pad("VCC", "SIG"),
        // An interior connector: held because of its prefix, wherever it sits.
        Fp::new("J1", 35.0, 10.0).pad(pad("1", 0.0, 0.0, Some("SIG"))),
        // A mounting hole with a netted pad and a mechanical prefix.
        Fp::new("H1", 5.0, 5.0).pad(pad("1", 0.0, 0.0, Some("GND"))),
        // A mounting hole by footprint name only: another prefix, a netted pad.
        Fp::new("X1", 5.0, 35.0)
            .lib("MountingHole:MountingHole_3.2mm_M3")
            .pad(pad("1", 0.0, 0.0, Some("GND"))),
        // A fiducial by footprint name, in mixed case.
        Fp::new("X2", 55.0, 35.0)
            .lib("Misc:fiducial_1mm")
            .pad(pad("1", 0.0, 0.0, Some("GND"))),
        // No pad on any net.
        Fp::new("TP9", 40.0, 30.0).pad(pad("1", 0.0, 0.0, None)),
        // KiCad's own lock.
        Fp::new("L1", 45.0, 10.0).two_pad("VCC", "SIG").locked(),
        // Named by the caller.
        Fp::new("C1", 15.0, 30.0).two_pad("VCC", "GND"),
        Fp::new("C2", 15.0, 10.0).two_pad("VCC", "GND"),
    ];
    let caller: BTreeSet<String> = ["C1".to_string()].into();
    let set = held_set(&board(OUTLINE, &parts), &caller, &HoldRules::default()).unwrap();
    let reasons = |reference: &str| {
        set.held[reference]
            .reasons
            .iter()
            .copied()
            .collect::<Vec<_>>()
    };
    assert_eq!(reasons("J1"), vec![HoldReason::Connector]);
    assert_eq!(reasons("H1"), vec![HoldReason::Mechanical]);
    assert_eq!(reasons("X1"), vec![HoldReason::Mechanical]);
    assert_eq!(reasons("X2"), vec![HoldReason::Mechanical]);
    assert_eq!(reasons("TP9"), vec![HoldReason::Mechanical]);
    assert_eq!(reasons("L1"), vec![HoldReason::KicadLock]);
    assert_eq!(reasons("C1"), vec![HoldReason::Caller]);
    let movable: Vec<&str> = set.movable.iter().map(String::as_str).collect();
    assert_eq!(movable, vec!["C2", "R1", "U1"]);
    // The held part keeps exactly what the text has.
    assert_eq!(set.held["J1"].at, (35.0, 10.0));
    assert_eq!(set.held["J1"].side, Side::Front);

    // The same footprints under ordinary names, with a netted pad, are movable.
    let renamed = vec![
        Fp::new("U1", 20.0, 20.0)
            .court(3.0, 3.0)
            .pad(pad("1", -2.0, 0.0, Some("VCC")))
            .pad(pad("2", 2.0, 0.0, Some("GND"))),
        Fp::new("R7", 35.0, 10.0).pad(pad("1", 0.0, 0.0, Some("SIG"))),
        Fp::new("R8", 5.0, 5.0).pad(pad("1", 0.0, 0.0, Some("GND"))),
        Fp::new("R9", 5.0, 35.0)
            .lib("Resistor_SMD:R_0402")
            .pad(pad("1", 0.0, 0.0, Some("GND"))),
    ];
    let set = held_set(
        &board(OUTLINE, &renamed),
        &no_caller(),
        &HoldRules::default(),
    )
    .unwrap();
    assert!(set.held.is_empty(), "{:?}", set.held);
    assert_eq!(set.movable.len(), 4);
}

#[test]
fn the_prefix_is_the_letters_before_the_number() {
    // `JP1` (a jumper) is not a `J` connector, and `HS1` (a heatsink) is not `H`.
    let parts = vec![
        Fp::new("JP1", 10.0, 10.0).two_pad("A", "B"),
        Fp::new("HS1", 20.0, 10.0).two_pad("A", "B"),
        Fp::new("MH2", 30.0, 10.0).two_pad("A", "B"),
        Fp::new("FID1", 40.0, 10.0).two_pad("A", "B"),
    ];
    let set = set_of(&parts);
    assert!(set.movable.contains("JP1") && set.movable.contains("HS1"));
    assert_eq!(
        set.held.keys().cloned().collect::<Vec<_>>(),
        vec!["FID1", "MH2"]
    );
}

#[test]
fn the_held_hash_is_stable_and_changes_with_the_list() {
    let parts = clean_parts();
    let a = set_of(&parts);
    let b = set_of(&parts);
    assert_eq!(a.held_sha256, b.held_sha256);
    assert_eq!(
        a.board_sha256,
        sha256_hex(board(OUTLINE, &parts).as_bytes())
    );
    let mut moved = parts.clone();
    moved[3].x -= 1.0;
    assert_ne!(set_of(&moved).held_sha256, a.held_sha256);
}

#[test]
fn same_board_ignores_formatting_not_values() {
    let parts = clean_parts();
    let original = board(OUTLINE, &parts);
    let respaced = original.replace("\n", "\n\n   ").replace("(at ", "(at  ");
    assert_eq!(same_board(&parse(&original), &parse(&respaced)), Ok(()));

    // 1.5 and 1.50 are the same number.
    let a = parse("(kicad_pcb (at 1.5 2))");
    let b = parse("(kicad_pcb (at 1.50 2.0))");
    assert_eq!(same_board(&a, &b), Ok(()));
    // A 0.01 mm shift is not.
    let shifted = parse("(kicad_pcb (at 1.51 2.0))");
    let error = same_board(&a, &shifted).unwrap_err();
    assert!(error.contains("at"), "{error}");
    // A change below 1e-6 is the same board.
    assert_eq!(
        same_board(&a, &parse("(kicad_pcb (at 1.5000004 2))")),
        Ok(())
    );
    // A string is compared exactly, even when it reads as a number.
    let error = same_board(&parse("(x (pad \"1\"))"), &parse("(x (pad \"1.0\"))")).unwrap_err();
    assert!(error.contains("differs"), "{error}");
    // A different number of children, a different kind of node.
    assert!(same_board(&parse("(x 1 2)"), &parse("(x 1)")).is_err());
    assert!(same_board(&parse("(x 1)"), &parse("(x (1))")).is_err());
    // The path of the difference names the footprint.
    let mut moved = parts.clone();
    moved[1].x += 0.01;
    let error = same_board(&parse(&original), &parse(&board(OUTLINE, &moved))).unwrap_err();
    assert!(error.contains("footprint(C1)"), "{error}");
}

#[test]
fn held_set_refuses_what_it_cannot_prove() {
    let rules = HoldRules::default();
    let clean = clean_parts();
    // The clean fixture is accepted.
    assert!(held_set(&board(OUTLINE, &clean), &no_caller(), &rules).is_ok());

    let mut cases: Vec<(&str, Result<HeldSet, HeldRefusal>)> = Vec::new();
    cases.push((
        "open outline",
        held_set(
            &board(Shape::Open(0.0, 0.0, 60.0, 40.0), &clean),
            &no_caller(),
            &rules,
        ),
    ));
    cases.push((
        "text that is not a board",
        held_set("(not_a_board)", &no_caller(), &rules),
    ));
    cases.push((
        "text that does not parse",
        held_set("(kicad_pcb", &no_caller(), &rules),
    ));
    let unknown: BTreeSet<String> = ["Z99".to_string()].into();
    cases.push((
        "unknown caller reference",
        held_set(&board(OUTLINE, &clean), &unknown, &rules),
    ));
    let mut shared = clean.clone();
    shared.push(Fp::new("C1", 40.0, 30.0).two_pad("VCC", "GND"));
    cases.push((
        "shared reference",
        held_set(&board(OUTLINE, &shared), &no_caller(), &rules),
    ));
    let mut skipped = clean.clone();
    skipped.push(
        Fp::new("R2", 40.0, 30.0)
            .two_pad("VCC", "SIG")
            .broken_courtyard(),
    );
    cases.push((
        "skipped courtyard",
        held_set(&board(OUTLINE, &skipped), &no_caller(), &rules),
    ));
    let mut nameless = clean.clone();
    nameless.push(Fp::new("", 40.0, 30.0).two_pad("VCC", "SIG"));
    cases.push((
        "missing reference",
        held_set(&board(OUTLINE, &nameless), &no_caller(), &rules),
    ));
    let only_connectors = vec![Fp::new("J1", 20.0, 20.0).pad(pad("1", 0.0, 0.0, Some("SIG")))];
    cases.push((
        "nothing movable",
        held_set(&board(OUTLINE, &only_connectors), &no_caller(), &rules),
    ));

    for (name, result) in cases {
        let refusal = result.expect_err(name);
        assert!(!refusal.reason.is_empty(), "{name}: no reason");
        assert!(!refusal.next_step.is_empty(), "{name}: no next step");
    }
    // The reason names the thing that is wrong.
    let refusal = held_set(&board(OUTLINE, &clean), &unknown, &rules).unwrap_err();
    assert!(refusal.reason.contains("Z99"), "{refusal:?}");
}

#[test]
fn a_candidate_that_moves_a_held_part_is_refused() {
    let parts = clean_parts();
    let set = set_of(&parts);
    let ok = |parts: &[Fp]| check_held(&set, &parse(&board(OUTLINE, parts)));
    // The unchanged text passes, and so does moving a movable part.
    assert_eq!(ok(&parts), Ok(()));
    let mut moved_movable = parts.clone();
    moved_movable[1].x += 3.0;
    assert_eq!(ok(&moved_movable), Ok(()));

    // J1 is the held connector (index 3). A 1 µm shift.
    let mut shifted = parts.clone();
    shifted[3].x += 0.001;
    let lines = ok(&shifted).unwrap_err();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("J1") && lines[0].contains("moved"),
        "{lines:?}"
    );
    // A turn.
    let mut turned = parts.clone();
    turned[3] = turned[3].clone().rot(90.0);
    let lines = ok(&turned).unwrap_err();
    assert!(lines.iter().any(|l| l.contains("turned")), "{lines:?}");
    // A flip.
    let mut flipped = parts.clone();
    flipped[3] = flipped[3].clone().back();
    let lines = ok(&flipped).unwrap_err();
    assert!(
        lines.iter().any(|l| l.contains("changed side")),
        "{lines:?}"
    );
    // A dropped footprint, held or movable.
    let mut dropped = parts.clone();
    dropped.remove(3);
    let lines = ok(&dropped).unwrap_err();
    assert!(
        lines.iter().any(|l| l.contains("J1 is missing")),
        "{lines:?}"
    );
    let mut dropped_movable = parts.clone();
    dropped_movable.remove(1);
    let lines = ok(&dropped_movable).unwrap_err();
    assert!(
        lines.iter().any(|l| l.contains("C1 is missing")),
        "{lines:?}"
    );
    // An added one.
    let mut added = parts.clone();
    added.push(Fp::new("R9", 40.0, 30.0).two_pad("VCC", "SIG"));
    let lines = ok(&added).unwrap_err();
    assert!(lines.iter().any(|l| l.contains("R9")), "{lines:?}");
    // One twice.
    let mut twice = parts.clone();
    twice.push(parts[1].clone());
    let lines = ok(&twice).unwrap_err();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("C1") && l.contains("2 times")),
        "{lines:?}"
    );
}

#[test]
fn placements_of_reads_anchors_and_names_what_is_missing() {
    let parts = vec![
        Fp::new("U1", 20.0, 20.0)
            .court(3.0, 3.0)
            .two_pad("A", "B")
            .rot(90.0),
        Fp::new("R1", 28.5, 21.25).two_pad("A", "B"),
    ];
    let tree = parse(&board(OUTLINE, &parts));
    let both: BTreeSet<String> = ["R1".to_string(), "U1".to_string()].into();
    let placements = placements_of(&tree, &both).unwrap();
    assert_eq!(placements.len(), 2);
    assert_eq!(placements[0].reference, "R1");
    assert_eq!(
        (placements[0].x, placements[0].y, placements[0].rotation),
        (28.5, 21.25, 0.0)
    );
    assert_eq!(placements[1].rotation, 90.0);
    let missing: BTreeSet<String> = ["R1".to_string(), "Z1".to_string()].into();
    let error = placements_of(&tree, &missing).unwrap_err().to_string();
    assert!(error.contains("Z1"), "{error}");
}

fn association(cap: &str, ic: &str, net: &str, status: &str, basis: &str) -> serde_json::Value {
    serde_json::json!({
        "reference": cap, "ic": ic, "cap_pad": "1", "ic_pad": "3",
        "qualifying_net": net, "limit_mm": 2.5, "status": status,
        "association_basis": basis,
    })
}

#[test]
fn pairs_come_from_associations_never_from_ground() {
    // C2 shares only GND with U1; C1 shares VCC.
    let parts = vec![
        Fp::new("U1", 20.0, 20.0)
            .court(3.0, 3.0)
            .pad(pad("1", -2.0, 0.0, Some("VCC")))
            .pad(pad("2", 2.0, 0.0, Some("GND"))),
        Fp::new("C1", 14.0, 20.0)
            .value("100nF")
            .two_pad("VCC", "GND"),
        Fp::new("C2", 26.0, 20.0)
            .value("100nF")
            .pad(pad("1", -0.5, 0.0, Some("GND")))
            .pad(pad("2", 0.5, 0.0, Some("OTHER"))),
    ];
    let evaluated = evaluate_text(&board(OUTLINE, &parts), &[]);
    let pairs = decoupling_pairs(&evaluated).unwrap();
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    assert_eq!((pairs[0].cap.as_str(), pairs[0].ic.as_str()), ("C1", "U1"));
    assert_eq!(pairs[0].net, "VCC");
    assert_eq!(pairs[0].limit_mm, 2.5);
    assert!(
        !pairs[0].basis.is_empty() && !pairs[0].cap_pad.is_empty() && !pairs[0].ic_pad.is_empty()
    );

    // Sharing a non-ground rail gives the pair.
    let mut shared = parts.clone();
    shared[2] = Fp::new("C2", 26.0, 20.0)
        .value("100nF")
        .pad(pad("1", -0.5, 0.0, Some("VCC")))
        .pad(pad("2", 0.5, 0.0, Some("GND")));
    let evaluated = evaluate_text(&board(OUTLINE, &shared), &[]);
    assert_eq!(decoupling_pairs(&evaluated).unwrap().len(), 2);

    // Every entry counts, whatever its status: a far capacitor is a pair too.
    let mut far = parts.clone();
    far[1] = Fp::new("C1", 4.0, 5.0).value("100nF").two_pad("VCC", "GND");
    let evaluated = evaluate_text(&board(OUTLINE, &far), &[]);
    let entries = evaluated["decoupling_associations"].as_array().unwrap();
    assert_eq!(entries[0]["status"], "too_far");
    assert_eq!(decoupling_pairs(&evaluated).unwrap().len(), 1);
}

#[test]
fn a_decoupling_entry_that_lacks_a_field_is_an_error() {
    let good = association("C1", "U1", "VCC", "within_limit", "pin_adjacent");
    assert!(
        decoupling_pairs(&serde_json::json!({"decoupling_associations": [good.clone()]})).is_ok()
    );
    for field in [
        "reference",
        "ic",
        "cap_pad",
        "ic_pad",
        "qualifying_net",
        "limit_mm",
        "association_basis",
    ] {
        let mut entry = good.clone();
        entry.as_object_mut().unwrap().remove(field);
        let error = decoupling_pairs(&serde_json::json!({"decoupling_associations": [entry]}))
            .unwrap_err()
            .to_string();
        assert!(error.contains(field), "{field}: {error}");
    }
    assert!(decoupling_pairs(&serde_json::json!({})).is_err());
}

#[test]
fn groups_drop_held_members_and_groups_of_one() {
    let parts = vec![
        Fp::new("U1", 10.0, 10.0).court(3.0, 3.0).two_pad("A", "B"),
        Fp::new("C1", 20.0, 10.0).two_pad("A", "B"),
        Fp::new("C2", 30.0, 10.0).two_pad("A", "B"),
        Fp::new("J1", 50.0, 10.0).pad(pad("1", 0.0, 0.0, Some("A"))),
        Fp::new("R1", 10.0, 30.0).two_pad("A", "B"),
    ];
    let text = board_with_groups(
        OUTLINE,
        &parts,
        &[
            ("power", &["U1", "C1", "J1"]),
            ("lonely", &["R1", "J1"]),
            ("all", &["U1", "C1", "C2"]),
        ],
    );
    let set = held_set(&text, &no_caller(), &HoldRules::default()).unwrap();
    assert!(set.held.contains_key("J1"));
    let caller = vec![Group {
        name: "mine".to_string(),
        source: GroupSource::Caller,
        members: ["R1", "C2", "J1"].iter().map(|s| s.to_string()).collect(),
    }];
    let found = groups(&parse(&text), &set, &caller);
    let names: Vec<&str> = found.iter().map(|g| g.name.as_str()).collect();
    // The caller's group first; "lonely" is left with R1 alone and is dropped.
    assert_eq!(names, vec!["mine", "power", "all"], "{found:?}");
    assert_eq!(found[0].source, GroupSource::Caller);
    assert_eq!(
        found[0].members.iter().cloned().collect::<Vec<_>>(),
        vec!["C2", "R1"]
    );
    assert_eq!(found[1].source, GroupSource::KicadGroup);
    assert_eq!(
        found[1].members.iter().cloned().collect::<Vec<_>>(),
        vec!["C1", "U1"]
    );
    assert!(found
        .iter()
        .all(|g| !g.members.contains("J1") && g.members.len() >= 2));
}

#[test]
fn sheet_groups_follow_the_footprint_paths() {
    let two_sheets = vec![
        Fp::new("U1", 10.0, 10.0)
            .court(3.0, 3.0)
            .two_pad("A", "B")
            .path("/aaaa/u1"),
        Fp::new("C1", 20.0, 10.0).two_pad("A", "B").path("/aaaa/c1"),
        Fp::new("R1", 30.0, 10.0).two_pad("A", "B").path("/bbbb/r1"),
        Fp::new("R2", 40.0, 10.0).two_pad("A", "B").path("/bbbb/r2"),
        Fp::new("R3", 50.0, 10.0).two_pad("A", "B").path("/cccc/r3"),
        // A root-sheet symbol: its parent path is the root.
        Fp::new("R4", 10.0, 30.0).two_pad("A", "B").path("/r4"),
        Fp::new("R5", 20.0, 30.0).two_pad("A", "B"),
    ];
    let text = board(OUTLINE, &two_sheets);
    let set = held_set(&text, &no_caller(), &HoldRules::default()).unwrap();
    let found = groups(&parse(&text), &set, &[]);
    let by_name: BTreeMap<&str, Vec<&str>> = found
        .iter()
        .map(|g| {
            (
                g.name.as_str(),
                g.members.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    assert_eq!(
        by_name.get("sheet /aaaa"),
        Some(&vec!["C1", "U1"]),
        "{found:?}"
    );
    assert_eq!(
        by_name.get("sheet /bbbb"),
        Some(&vec!["R1", "R2"]),
        "{found:?}"
    );
    // /cccc and the root hold one part each, and a group of one is dropped.
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found.iter().all(|g| g.source == GroupSource::Sheet));

    // With one sheet instance only, there is no sheet grouping at all.
    let one_sheet = vec![
        Fp::new("U1", 10.0, 10.0)
            .court(3.0, 3.0)
            .two_pad("A", "B")
            .path("/aaaa/u1"),
        Fp::new("C1", 20.0, 10.0).two_pad("A", "B").path("/aaaa/c1"),
    ];
    let text = board(OUTLINE, &one_sheet);
    let set = held_set(&text, &no_caller(), &HoldRules::default()).unwrap();
    assert!(groups(&parse(&text), &set, &[]).is_empty());
}

#[test]
fn a_kicad_group_inside_a_group_is_followed() {
    let parts = vec![
        Fp::new("U1", 10.0, 10.0).court(3.0, 3.0).two_pad("A", "B"),
        Fp::new("C1", 20.0, 10.0).two_pad("A", "B"),
        Fp::new("R1", 30.0, 10.0).two_pad("A", "B"),
    ];
    let text = board_with_groups(
        OUTLINE,
        &parts,
        &[("inner", &["C1", "R1"]), ("outer", &["U1", "@inner"])],
    );
    let set = held_set(&text, &no_caller(), &HoldRules::default()).unwrap();
    let found = groups(&parse(&text), &set, &[]);
    let outer = found
        .iter()
        .find(|g| g.name == "outer")
        .expect("outer group");
    assert_eq!(outer.members.len(), 3, "{found:?}");
}

#[test]
fn the_intent_hash_follows_its_content() {
    let pair = DecouplingPair {
        cap: "C1".into(),
        cap_pad: "1".into(),
        ic: "U1".into(),
        ic_pad: "3".into(),
        net: "VCC".into(),
        limit_mm: 2.5,
        basis: "pin_adjacent".into(),
    };
    let a = Intent::new(Vec::new(), vec![pair.clone()]);
    let b = Intent::new(Vec::new(), vec![pair.clone()]);
    assert_eq!(a, b);
    assert_eq!(a.sha256.len(), 64);
    let mut other = pair;
    other.limit_mm = 5.0;
    assert_ne!(Intent::new(Vec::new(), vec![other]).sha256, a.sha256);
}

#[test]
fn a_rounded_outline_and_a_part_without_courtyard_artwork_are_provable() {
    // Four lines and four arcs close; a part with no courtyard is read from its pads.
    let parts = vec![
        Fp::new("U1", 20.0, 20.0).no_court().two_pad("A", "B"),
        Fp::new("J1", 5.0, 20.0).pad(pad("1", 0.0, 0.0, Some("A"))),
    ];
    let text = board(Shape::Rounded(0.0, 0.0, 60.0, 40.0, 5.0), &parts);
    let set = held_set(&text, &no_caller(), &HoldRules::default()).unwrap();
    assert_eq!(set.movable.iter().cloned().collect::<Vec<_>>(), vec!["U1"]);
    assert_eq!(set.held.len(), 1);
}
