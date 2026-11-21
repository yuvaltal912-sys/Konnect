//! Tests of `place_schematic`: the schematic is read from small hand-written files, the
//! board from the placement tests' fixtures. Nothing here touches a real project.

use super::*;
use crate::place_held::tests::{board, parse, Fp, Shape};

const OUTLINE: Shape = Shape::Rect(0.0, 0.0, 200.0, 100.0);

// ─── Fixtures ────────────────────────────────────────────────────────────────

/// One placed symbol of a hand-written schematic.
struct Sym {
    reference: &'static str,
    uuid: &'static str,
    at: (f64, f64),
    unit: u32,
    /// `(sheet path, reference)` of its `(instances …)`.
    instances: Vec<(&'static str, &'static str)>,
}

fn sym(reference: &'static str, uuid: &'static str, x: f64, y: f64) -> Sym {
    Sym {
        reference,
        uuid,
        at: (x, y),
        unit: 1,
        instances: Vec::new(),
    }
}

impl Sym {
    fn unit(mut self, unit: u32) -> Self {
        self.unit = unit;
        self
    }
    fn on(mut self, sheet_path: &'static str, reference: &'static str) -> Self {
        self.instances.push((sheet_path, reference));
        self
    }
}

fn symbol_text(symbol: &Sym) -> String {
    let instances = if symbol.instances.is_empty() {
        String::new()
    } else {
        let paths: String = symbol
            .instances
            .iter()
            .map(|(path, reference)| {
                format!(
                    "(path \"{path}\" (reference \"{reference}\") (unit {}))",
                    symbol.unit
                )
            })
            .collect();
        format!("(instances (project \"p\" {paths}))")
    };
    format!(
        "(symbol (lib_id \"Dev:Part\") (at {} {} 0) (unit {}) (uuid \"{}\") (property \"Reference\" \"{}\" (at 0 0 0)) {instances})\n",
        symbol.at.0, symbol.at.1, symbol.unit, symbol.uuid, symbol.reference
    )
}

/// A schematic with `uuid`, the symbols, and one `(sheet …)` per `(uuid, file)`. Its
/// `lib_symbols` holds a symbol the extraction must not mistake for a placed one.
fn schematic_text(uuid: &str, symbols: &[Sym], sheets: &[(&str, &str)]) -> String {
    let mut text = format!(
        "(kicad_sch (version 20260101) (generator \"test\") (uuid \"{uuid}\") (paper \"A4\")\n(lib_symbols (symbol \"Dev:Part\" (property \"Reference\" \"LIB\" (at 0 0 0))))\n"
    );
    for symbol in symbols {
        text.push_str(&symbol_text(symbol));
    }
    for (sheet_uuid, file) in sheets {
        text.push_str(&format!(
            "(sheet (at 0 0) (size 10 10) (uuid \"{sheet_uuid}\") (property \"Sheetname\" \"s\" (at 0 0 0)) (property \"Sheetfile\" \"{file}\" (at 0 0 0)) (pin \"P\" input (at 0 0 0) (uuid \"pin-{sheet_uuid}\")))\n"
        ));
    }
    text.push_str(")\n");
    text
}

fn write(folder: &Path, name: &str, text: &str) -> PathBuf {
    let path = folder.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

/// A part of a test board: a courtyard of 1 mm around its anchor, two pads.
fn part(reference: &str, x: f64, y: f64) -> Fp {
    Fp::new(reference, x, y).court(0.5, 0.5).two_pad("A", "B")
}

fn board_of(parts: &[Fp]) -> SexpNode {
    parse(&board(OUTLINE, parts))
}

/// One flat sheet by hand, for the tests that do not read files.
fn flat(symbols: &[(&'static str, &'static str, f64, f64)]) -> Vec<SheetInstance> {
    vec![SheetInstance {
        chain: String::new(),
        symbols: symbols
            .iter()
            .map(|(reference, uuid, x, y)| SheetSymbol {
                uuid: Some((*uuid).to_string()),
                reference: (*reference).to_string(),
                at: (*x, *y),
            })
            .collect(),
    }]
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn list(hoods: &Neighbourhoods, reference: &str) -> Vec<String> {
    hoods
        .neighbours
        .get(reference)
        .unwrap_or_else(|| panic!("{reference} has no neighbourhood: {hoods:?}"))
        .clone()
}

// ─── Reading the sheets ──────────────────────────────────────────────────────

#[test]
fn a_flat_sheet_gives_each_part_its_three_nearest_ties_by_reference() {
    let dir = tempfile::tempdir().unwrap();
    let root = write(
        dir.path(),
        "p.kicad_sch",
        &schematic_text(
            "root",
            &[
                sym("R1", "u1", 0.0, 0.0),
                sym("R2", "u2", 10.0, 0.0),
                sym("R3", "u3", 20.0, 0.0),
                sym("R4", "u4", 30.0, 0.0),
                sym("R5", "u5", 40.0, 0.0),
            ],
            &[],
        ),
    );
    let sheets = read_sheets(&root).unwrap();
    assert_eq!(sheets.len(), 1);
    assert_eq!(
        sheets[0].symbols.len(),
        5,
        "the lib_symbols entry is no symbol"
    );
    // File order R5..R1, so that a missing tie-break would show: R3 is 10 mm from R2 and R4
    // alike, and 20 mm from R1 and R5 alike.
    let parts: Vec<Fp> = (1..=5)
        .rev()
        .map(|k| part(&format!("R{k}"), 10.0 * k as f64, 20.0).path(&format!("/u{k}")))
        .collect();
    let hoods = neighbourhoods(&board_of(&parts), &sheets).unwrap();
    assert_eq!(list(&hoods, "R1"), ["R2", "R3", "R4"], "k is 3");
    assert_eq!(list(&hoods, "R2"), ["R1", "R3", "R4"]);
    assert_eq!(list(&hoods, "R3"), ["R2", "R4", "R1"]);
    assert_eq!(list(&hoods, "R5"), ["R4", "R3", "R2"]);
    assert_eq!((hoods.measured, hoods.unmatched, hoods.alone), (5, 0, 0));
}

#[test]
fn a_sheet_with_fewer_parts_gives_fewer_neighbours_and_a_lone_part_none() {
    let sheets = flat(&[("R1", "u1", 0.0, 0.0), ("R2", "u2", 5.0, 0.0)]);
    let board = board_of(&[
        part("R1", 10.0, 10.0).path("/u1"),
        part("R2", 20.0, 10.0).path("/u2"),
    ]);
    let hoods = neighbourhoods(&board, &sheets).unwrap();
    assert_eq!(list(&hoods, "R1"), ["R2"]);
    assert_eq!(list(&hoods, "R2"), ["R1"]);
    // One part on its sheet: measured, alone, in nobody's list.
    let sheets = flat(&[("R1", "u1", 0.0, 0.0)]);
    let board = board_of(&[part("R1", 10.0, 10.0).path("/u1")]);
    let hoods = neighbourhoods(&board, &sheets).unwrap();
    assert!(hoods.neighbours.is_empty());
    assert_eq!((hoods.measured, hoods.alone), (1, 1));
}

#[test]
fn a_hierarchical_sheet_is_followed_and_its_parts_are_neighbours_only_of_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let root = write(
        dir.path(),
        "p.kicad_sch",
        &schematic_text(
            "root",
            &[sym("R1", "ua", 0.0, 0.0)],
            &[("s1", "sub.kicad_sch")],
        ),
    );
    write(
        dir.path(),
        "sub.kicad_sch",
        &schematic_text(
            "file-uuid-not-the-instance",
            &[sym("R2", "ub", 500.0, 0.0), sym("R3", "uc", 600.0, 0.0)],
            &[],
        ),
    );
    let sheets = read_sheets(&root).unwrap();
    let chains: Vec<&str> = sheets.iter().map(|s| s.chain.as_str()).collect();
    assert_eq!(chains, ["", "/s1"]);
    // R1 is the nearest to R2 on the board and 500 mm away in the schematic: another sheet
    // instance, so it is no neighbour.
    let board = board_of(&[
        part("R1", 10.0, 10.0).path("/ua"),
        part("R2", 11.0, 10.0).path("/s1/ub"),
        part("R3", 60.0, 10.0).path("/s1/uc"),
    ]);
    let hoods = neighbourhoods(&board, &sheets).unwrap();
    assert_eq!(list(&hoods, "R2"), ["R3"]);
    assert_eq!(list(&hoods, "R3"), ["R2"]);
    assert!(!hoods.neighbours.contains_key("R1"));
    assert_eq!((hoods.measured, hoods.unmatched, hoods.alone), (3, 0, 1));
}

#[test]
fn one_sheet_file_used_twice_is_two_instances_with_no_neighbours_across() {
    let dir = tempfile::tempdir().unwrap();
    let root = write(
        dir.path(),
        "p.kicad_sch",
        &schematic_text(
            "root",
            &[],
            &[("s1", "ch.kicad_sch"), ("s2", "ch.kicad_sch")],
        ),
    );
    // The file's own Reference property is the last one saved (R10, R11); each instance
    // has its own reference in its instances entry.
    write(
        dir.path(),
        "ch.kicad_sch",
        &schematic_text(
            "ch",
            &[
                sym("R10", "up", 0.0, 0.0)
                    .on("/root/s1", "R10")
                    .on("/root/s2", "R20"),
                sym("R11", "uq", 5.0, 0.0)
                    .on("/root/s1", "R11")
                    .on("/root/s2", "R21"),
            ],
            &[],
        ),
    );
    let sheets = read_sheets(&root).unwrap();
    let chains: Vec<&str> = sheets.iter().map(|s| s.chain.as_str()).collect();
    assert_eq!(chains, ["", "/s1", "/s2"]);
    // All four parts are within a few mm of each other on the board.
    let board = board_of(&[
        part("R10", 10.0, 10.0).path("/s1/up"),
        part("R11", 12.0, 10.0).path("/s1/uq"),
        part("R20", 14.0, 10.0).path("/s2/up"),
        part("R21", 16.0, 10.0).path("/s2/uq"),
    ]);
    let hoods = neighbourhoods(&board, &sheets).unwrap();
    assert_eq!(list(&hoods, "R10"), ["R11"]);
    assert_eq!(list(&hoods, "R11"), ["R10"]);
    assert_eq!(list(&hoods, "R20"), ["R21"]);
    assert_eq!(list(&hoods, "R21"), ["R20"]);
    assert_eq!((hoods.measured, hoods.unmatched), (4, 0));
    // A path that names no sheet instance falls back to the reference on the instance: R20
    // is a reference of the second instance only, and the file's own property says R10.
    let board = board_of(&[
        part("R20", 14.0, 10.0).path("/gone/up"),
        part("R21", 16.0, 10.0).path("/s2/uq"),
    ]);
    let hoods = neighbourhoods(&board, &sheets).unwrap();
    assert_eq!(list(&hoods, "R20"), ["R21"], "{hoods:?}");
    assert_eq!(hoods.unmatched, 0);
}

#[test]
fn every_unit_of_a_part_gives_a_point_and_the_least_distance_counts() {
    let dir = tempfile::tempdir().unwrap();
    let root = write(
        dir.path(),
        "p.kicad_sch",
        &schematic_text(
            "root",
            &[
                // U1's units are 100 mm apart; the footprint's path names unit 1.
                sym("U1", "u1a", 0.0, 0.0),
                sym("U1", "u1b", 100.0, 0.0).unit(2),
                sym("R1", "ur1", 95.0, 0.0),
                sym("R2", "ur2", -20.0, 0.0),
            ],
            &[],
        ),
    );
    let sheets = read_sheets(&root).unwrap();
    let board = board_of(&[
        part("U1", 50.0, 50.0).path("/u1a"),
        part("R1", 60.0, 50.0).path("/ur1"),
        part("R2", 70.0, 50.0).path("/ur2"),
    ]);
    let hoods = neighbourhoods(&board, &sheets).unwrap();
    // Unit 2 is 5 mm from R1: R1 is nearer than R2 (20 mm from unit 1), not 95 mm away.
    assert_eq!(list(&hoods, "U1"), ["R1", "R2"]);
    assert_eq!(list(&hoods, "R1"), ["U1", "R2"]);
    assert_eq!(list(&hoods, "R2"), ["U1", "R1"]);
    assert_eq!(hoods.measured, 3, "one part for the two units");
}

#[test]
fn an_unmatched_footprint_is_counted_and_a_padless_one_is_skipped() {
    let sheets = flat(&[
        ("R1", "u1", 0.0, 0.0),
        ("R2", "u2", 5.0, 0.0),
        ("FID1", "uf", 2.0, 0.0),
    ]);
    let board = board_of(&[
        part("R1", 10.0, 10.0).path("/u1"),
        part("R2", 20.0, 10.0).path("/u2"),
        // A symbol nobody has, and a board-only footprint with no path at all.
        part("X1", 30.0, 10.0).path("/nosuch"),
        part("BO1", 40.0, 10.0),
        // A fiducial with a symbol but no pad: neither measured nor unmatched, and nobody's
        // neighbour although it is the nearest symbol to both.
        Fp::new("FID1", 15.0, 10.0).court(0.5, 0.5).path("/uf"),
    ]);
    let hoods = neighbourhoods(&board, &sheets).unwrap();
    assert_eq!((hoods.measured, hoods.unmatched, hoods.alone), (2, 2, 0));
    assert_eq!(list(&hoods, "R1"), ["R2"]);
    assert_eq!(list(&hoods, "R2"), ["R1"]);
}

#[test]
fn a_part_whose_path_does_not_resolve_is_matched_by_its_reference() {
    let dir = tempfile::tempdir().unwrap();
    let root = write(
        dir.path(),
        "p.kicad_sch",
        &schematic_text(
            "root",
            &[sym("R1", "u1", 0.0, 0.0), sym("R2", "u2", 5.0, 0.0)],
            &[("s1", "sub.kicad_sch")],
        ),
    );
    write(
        dir.path(),
        "sub.kicad_sch",
        &schematic_text(
            "sub",
            &[sym("R7", "u7", 0.0, 0.0), sym("R8", "u8", 5.0, 0.0)],
            &[],
        ),
    );
    let sheets = read_sheets(&root).unwrap();
    let board = board_of(&[
        // The symbol's uuid is stale, but the path names the root instance: R1 is there.
        part("R1", 10.0, 10.0).path("/stale-uuid"),
        part("R2", 20.0, 10.0).path("/u2"),
        // The path names a sheet instance that does not exist: R7 is on one instance only.
        part("R7", 30.0, 10.0).path("/gone/u7"),
        part("R8", 40.0, 10.0).path("/s1/u8"),
    ]);
    let hoods = neighbourhoods(&board, &sheets).unwrap();
    assert_eq!(hoods.unmatched, 0, "{hoods:?}");
    assert_eq!(list(&hoods, "R1"), ["R2"]);
    assert_eq!(list(&hoods, "R7"), ["R8"]);
}

#[test]
fn a_reference_on_two_instances_is_not_guessed_when_the_path_does_not_resolve() {
    let dir = tempfile::tempdir().unwrap();
    let root = write(
        dir.path(),
        "p.kicad_sch",
        &schematic_text(
            "root",
            &[],
            &[("s1", "ch.kicad_sch"), ("s2", "ch.kicad_sch")],
        ),
    );
    // No instances entries: both instances read R9 and R8 from the property.
    write(
        dir.path(),
        "ch.kicad_sch",
        &schematic_text(
            "ch",
            &[sym("R9", "u9", 0.0, 0.0), sym("R8", "u8", 5.0, 0.0)],
            &[],
        ),
    );
    let sheets = read_sheets(&root).unwrap();
    let board = board_of(&[
        part("R9", 10.0, 10.0).path("/nowhere/u9"),
        part("R8", 20.0, 10.0).path("/s1/u8"),
    ]);
    let hoods = neighbourhoods(&board, &sheets).unwrap();
    assert_eq!((hoods.measured, hoods.unmatched), (1, 1), "{hoods:?}");
}

#[test]
fn a_schematic_that_cannot_be_read_is_an_error_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    // No file at all.
    let error = read_sheets(&dir.path().join("p.kicad_sch")).unwrap_err();
    assert!(error.contains("cannot be read"), "{error}");
    // A sheet whose file is missing.
    let root = write(
        dir.path(),
        "p.kicad_sch",
        &schematic_text("root", &[], &[("s1", "gone.kicad_sch")]),
    );
    let error = read_sheets(&root).unwrap_err();
    assert!(error.contains("gone.kicad_sch"), "{error}");
    // A sheet with no Sheetfile.
    let text = schematic_text("root", &[], &[("s1", "x")]).replace("Sheetfile", "Other");
    write(dir.path(), "p.kicad_sch", &text);
    assert!(read_sheets(&root).unwrap_err().contains("no Sheetfile"));
    // A file that includes itself.
    write(
        dir.path(),
        "p.kicad_sch",
        &schematic_text("root", &[], &[("s1", "loop.kicad_sch")]),
    );
    write(
        dir.path(),
        "loop.kicad_sch",
        &schematic_text("loop", &[], &[("s2", "loop.kicad_sch")]),
    );
    let error = read_sheets(&root).unwrap_err();
    assert!(error.contains("includes itself"), "{error}");
    // A sheet file outside the schematic's folder, which exists.
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    write(
        dir.path(),
        "outside.kicad_sch",
        &schematic_text("outside", &[], &[]),
    );
    let root = write(
        &project,
        "p.kicad_sch",
        &schematic_text("root", &[], &[("s1", "../outside.kicad_sch")]),
    );
    let error = read_sheets(&root).unwrap_err();
    assert!(error.contains("outside"), "{error}");
    // Negative control: the same sheet inside the folder reads.
    write(
        &project,
        "inside.kicad_sch",
        &schematic_text("in", &[], &[]),
    );
    write(
        &project,
        "p.kicad_sch",
        &schematic_text("root", &[], &[("s1", "inside.kicad_sch")]),
    );
    assert_eq!(read_sheets(&root).unwrap().len(), 2);
}

#[test]
fn the_schematic_distance_uses_both_axes() {
    // P and Q are 4 mm apart, P and R 5 mm; all three sit well off the y axis, so a sum of
    // the y offsets in place of their difference would put R first.
    let sheets = flat(&[
        ("P", "up", 0.0, 10.0),
        ("Q", "uq", 0.0, 14.0),
        ("R", "ur", 5.0, 10.0),
    ]);
    let board = board_of(&[
        part("P", 10.0, 10.0).path("/up"),
        part("Q", 20.0, 10.0).path("/uq"),
        part("R", 30.0, 10.0).path("/ur"),
    ]);
    let hoods = neighbourhoods(&board, &sheets).unwrap();
    assert_eq!(list(&hoods, "P"), ["Q", "R"]);
}

#[test]
fn a_board_whose_courtyard_scan_skips_a_footprint_is_a_reason() {
    let sheets = flat(&[("P", "up", 0.0, 0.0), ("Q", "uq", 5.0, 0.0)]);
    let good = board_of(&[
        part("P", 10.0, 10.0).path("/up"),
        part("Q", 20.0, 10.0).path("/uq"),
    ]);
    let broken = board_of(&[
        part("P", 10.0, 10.0).path("/up"),
        part("Q", 20.0, 10.0).path("/uq"),
        part("Z", 30.0, 10.0).broken_courtyard(),
    ]);
    let error = neighbourhoods(&broken, &sheets).unwrap_err();
    assert!(error.contains("skipped 1"), "{error}");
    let base = SchematicBaseline::from_sheets(&sheets, &good).unwrap();
    let error = base.measure(&broken).unwrap_err();
    assert!(error.contains("skipped 1"), "{error}");
    // Negative control: the good board measures.
    assert!(base.measure(&good).is_ok());
}

#[test]
fn sheets_nest_to_a_bounded_depth() {
    // `n` files below the root, each including the next.
    let nested = |n: usize| {
        let dir = tempfile::tempdir().unwrap();
        let name = |k: usize| format!("f{k}.kicad_sch");
        let root = write(
            dir.path(),
            "p.kicad_sch",
            &schematic_text("root", &[], &[("s0", &name(1))]),
        );
        for k in 1..=n {
            let child = if k < n {
                vec![("sx", name(k + 1))]
            } else {
                Vec::new()
            };
            let refs: Vec<(&str, &str)> = child.iter().map(|(u, f)| (*u, f.as_str())).collect();
            write(dir.path(), &name(k), &schematic_text("f", &[], &refs));
        }
        (dir, read_sheets(&root))
    };
    // 31 files below the root is 32 files in all: the deepest allowed.
    let (_keep, ok) = nested(MAX_DEPTH - 1);
    assert_eq!(ok.unwrap().len(), MAX_DEPTH);
    let (_keep, deep) = nested(MAX_DEPTH);
    assert!(deep.unwrap_err().contains("deeper"));
}

// ─── The measure ─────────────────────────────────────────────────────────────

/// Three parts that are each other's neighbours (A, B, C), one alone on another sheet
/// instance (D), and one pad-bearing footprint with no symbol (X).
fn cluster_sheets() -> Vec<SheetInstance> {
    let mut sheets = flat(&[
        ("A", "ua", 0.0, 0.0),
        ("B", "ub", 4.0, 0.0),
        ("C", "uc", 0.0, 4.0),
    ]);
    sheets.push(SheetInstance {
        chain: "/s2".to_string(),
        symbols: vec![SheetSymbol {
            uuid: Some("ud".to_string()),
            reference: "D".to_string(),
            at: (0.0, 0.0),
        }],
    });
    sheets
}

fn cluster_board(c: (f64, f64), d: (f64, f64), x: (f64, f64)) -> SexpNode {
    board_of(&[
        part("A", 10.0, 10.0).path("/ua"),
        part("B", 14.0, 10.0).path("/ub"),
        part("C", c.0, c.1).path("/uc"),
        part("D", d.0, d.1).path("/s2/ud"),
        part("X", x.0, x.1),
    ])
}

#[test]
fn the_input_measures_itself_as_one() {
    let input = cluster_board((10.0, 14.0), (100.0, 50.0), (150.0, 80.0));
    let base = SchematicBaseline::from_sheets(&cluster_sheets(), &input).unwrap();
    let measure = base.measure(&input).unwrap();
    // A: (4 + 4) / 2; B and C: (4 + 4 * sqrt 2) / 2; the mean of the three.
    let diagonal = 4.0 * 2f64.sqrt();
    let expected = (4.0 + 2.0 * (4.0 + diagonal) / 2.0) / 3.0;
    assert!(near(measure.mean_mm, expected), "{}", measure.mean_mm);
    assert!(near(measure.ratio_to_input.unwrap(), 1.0));
    assert_eq!(measure.parts_pulled_away, 0);
    assert_eq!(
        (
            measure.parts_measured,
            measure.parts_unmatched,
            measure.parts_alone
        ),
        (4, 1, 1)
    );
}

#[test]
fn moving_a_schematic_neighbour_far_away_raises_the_mean_and_counts_it_pulled() {
    let input = cluster_board((10.0, 14.0), (100.0, 50.0), (150.0, 80.0));
    let base = SchematicBaseline::from_sheets(&cluster_sheets(), &input).unwrap();
    let near_mean = base.measure(&input).unwrap();
    // C goes from 4 mm below A to 40 mm below it.
    let moved = cluster_board((10.0, 50.0), (100.0, 50.0), (150.0, 80.0));
    let measure = base.measure(&moved).unwrap();
    assert!(measure.mean_mm > near_mean.mean_mm);
    assert!(measure.ratio_to_input.unwrap() > 1.0);
    // By hand: A (4 + 40) / 2 = 22 against 4; B (4 + hypot(4, 40)) / 2 = 22.1 against 4.83;
    // C (40 + hypot(4, 40)) / 2 = 40.1 against 4.83: all more than twice and 10 mm more.
    let own = part_distances(&moved, &base.hoods).unwrap();
    assert!(near(own["A"], 22.0));
    assert!(near(own["B"], (4.0 + 4f64.hypot(40.0)) / 2.0));
    assert!(near(own["C"], (40.0 + 4f64.hypot(40.0)) / 2.0));
    assert_eq!(measure.parts_pulled_away, 3);
}

#[test]
fn moving_a_part_that_is_no_ones_neighbour_and_has_none_changes_nothing() {
    let input = cluster_board((10.0, 14.0), (100.0, 50.0), (150.0, 80.0));
    let base = SchematicBaseline::from_sheets(&cluster_sheets(), &input).unwrap();
    // D is alone on its sheet instance; X has no symbol. Both go to the other corner.
    let moved = cluster_board((10.0, 14.0), (190.0, 90.0), (5.0, 95.0));
    assert_eq!(base.measure(&moved).unwrap(), base.measure(&input).unwrap());
    // Negative control: the same move of C changes the measure.
    let moved = cluster_board((190.0, 90.0), (100.0, 50.0), (150.0, 80.0));
    assert_ne!(base.measure(&moved).unwrap(), base.measure(&input).unwrap());
}

/// Two parts that are each other's only neighbour, `before` mm apart on the input and
/// `after` on the candidate: the parts pulled away (each part's own distance is the gap).
fn pulled_by(before: f64, after: f64) -> usize {
    let sheets = flat(&[("P", "up", 0.0, 0.0), ("Q", "uq", 5.0, 0.0)]);
    let two = |gap: f64| {
        board_of(&[
            part("P", 10.0, 10.0).path("/up"),
            part("Q", 10.0 + gap, 10.0).path("/uq"),
        ])
    };
    let base = SchematicBaseline::from_sheets(&sheets, &two(before)).unwrap();
    base.measure(&two(after)).unwrap().parts_pulled_away
}

#[test]
fn a_part_is_pulled_away_only_when_both_the_factor_and_the_distance_are_exceeded() {
    // More than 2x but not more than 10 mm above: 3 -> 7 (2.3x, +4 mm).
    assert_eq!(pulled_by(3.0, 7.0), 0);
    // More than 10 mm above but not 2x: 30 -> 45 (1.5x, +15 mm).
    assert_eq!(pulled_by(30.0, 45.0), 0);
    // Both: 5 -> 20 (4x, +15 mm), for P and for Q.
    assert_eq!(pulled_by(5.0, 20.0), 2);
    // The edges are not exceeded: exactly 2x (20 -> 40, +20 mm) and exactly +10 mm
    // (5 -> 15, 3x); a hair over either edge, with the other met, is.
    assert_eq!(pulled_by(20.0, 40.0), 0);
    assert_eq!(pulled_by(20.0, 40.5), 2);
    assert_eq!(pulled_by(5.0, 15.0), 0);
    assert_eq!(pulled_by(5.0, 15.5), 2);
    // The predicate alone, with an input of 0 (everything above 10 mm is 'more than 2x').
    assert!(!is_pulled_away(0.0, 10.0));
    assert!(is_pulled_away(0.0, 10.5));
}

#[test]
fn distances_run_between_courtyard_centres_not_anchors() {
    let sheets = flat(&[("P", "up", 0.0, 0.0), ("Q", "uq", 5.0, 0.0)]);
    // P's courtyard is off-centre: its box (9, 9) to (15, 13) has its centre at (12, 11),
    // 2 mm right of and 1 mm below the anchor (10, 10). Q's centre is its anchor.
    let board = board_of(&[
        Fp::new("P", 10.0, 10.0)
            .court_rect(-1.0, -1.0, 5.0, 3.0)
            .two_pad("A", "B")
            .path("/up"),
        part("Q", 20.0, 10.0).path("/uq"),
    ]);
    let hoods = neighbourhoods(&board, &sheets).unwrap();
    let own = part_distances(&board, &hoods).unwrap();
    assert!(near(own["P"], 8.0f64.hypot(1.0)), "{own:?}");
    assert!(near(own["Q"], 8.0f64.hypot(1.0)));
}

#[test]
fn an_input_with_no_neighbour_or_no_distance_is_a_reason_not_a_number() {
    // Nothing matches: no neighbourhood, so no baseline.
    let sheets = flat(&[("Z", "uz", 0.0, 0.0)]);
    let board = board_of(&[part("R1", 10.0, 10.0).path("/u1")]);
    let error = SchematicBaseline::from_sheets(&sheets, &board).unwrap_err();
    assert!(error.contains("no part of the board has a schematic neighbour"));
    // Two neighbours on the same spot: the input's mean is 0, so no ratio.
    let sheets = flat(&[("P", "up", 0.0, 0.0), ("Q", "uq", 5.0, 0.0)]);
    let stacked = board_of(&[
        part("P", 10.0, 10.0).path("/up"),
        part("Q", 10.0, 10.0).path("/uq"),
    ]);
    let base = SchematicBaseline::from_sheets(&sheets, &stacked).unwrap();
    let measure = base.measure(&stacked).unwrap();
    assert_eq!(measure.mean_mm, 0.0);
    assert_eq!(measure.ratio_to_input, None);
}

#[test]
fn a_candidate_that_lacks_a_measured_part_is_a_reason() {
    let input = cluster_board((10.0, 14.0), (100.0, 50.0), (150.0, 80.0));
    let base = SchematicBaseline::from_sheets(&cluster_sheets(), &input).unwrap();
    let without_c = board_of(&[
        part("A", 10.0, 10.0).path("/ua"),
        part("B", 14.0, 10.0).path("/ub"),
        part("D", 100.0, 50.0).path("/s2/ud"),
    ]);
    assert!(base
        .measure(&without_c)
        .unwrap_err()
        .contains("not on the board"));
}

#[test]
fn the_baseline_reads_the_schematic_named_like_the_board() {
    let dir = tempfile::tempdir().unwrap();
    let board_path = dir.path().join("my_board.kicad_pcb");
    let board = board_of(&[
        part("R1", 10.0, 10.0).path("/u1"),
        part("R2", 20.0, 10.0).path("/u2"),
    ]);
    // Only another project's schematic is there: a reason.
    write(
        dir.path(),
        "other.kicad_sch",
        &schematic_text("o", &[sym("R1", "u1", 0.0, 0.0)], &[]),
    );
    let error = SchematicBaseline::read(&board_path, &board).unwrap_err();
    assert!(error.contains("my_board.kicad_sch"), "{error}");
    // Negative control: the one named like the board reads.
    write(
        dir.path(),
        "my_board.kicad_sch",
        &schematic_text(
            "r",
            &[sym("R1", "u1", 0.0, 0.0), sym("R2", "u2", 5.0, 0.0)],
            &[],
        ),
    );
    let base = SchematicBaseline::read(&board_path, &board).unwrap();
    assert_eq!(base.hoods.measured, 2);
    assert!(near(base.input_mean, 10.0));
}
