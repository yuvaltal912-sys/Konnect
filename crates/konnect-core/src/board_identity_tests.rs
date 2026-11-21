//! The board identity: one value for KiCad's in-memory text of a board and the
//! file it saves, and a different value for any other board.

use super::*;
use konnect_ipc::{IpcEffectiveRoutingRules, IpcRoutingRules};

/// A board as KiCad saves it.
pub(crate) const SAVED: &str = include_str!("../tests/fixtures/specctra_four_layer_back.kicad_pcb");

/// The same board as IPC serializes it: one line, and each footprint carrying the
/// standalone library metadata a saved board leaves out.
pub(crate) fn ipc_form(saved: &str) -> String {
    let one_line = saved.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    let mut rest = one_line.as_str();
    while let Some(at) = rest.find("(footprint \"") {
        let name_start = at + "(footprint \"".len();
        let name_end = name_start + rest[name_start..].find('"').expect("a closed name");
        out.push_str(&rest[..=name_end]);
        out.push_str(" (version 20260206) (generator \"pcbnew\") (generator_version \"10.0\")");
        rest = &rest[name_end + 1..];
    }
    out.push_str(rest);
    out
}

#[test]
fn the_saved_file_and_the_ipc_text_of_one_board_share_the_identity() {
    let live = ipc_form(SAVED);
    assert!(
        live.contains("(generator_version \"10.0\")"),
        "fixture has footprints"
    );
    assert_ne!(live, SAVED, "the two texts differ byte for byte");
    assert_eq!(
        canonical_sha256(SAVED).unwrap(),
        canonical_sha256(&live).unwrap()
    );
}

#[test]
fn a_change_to_the_board_changes_the_identity() {
    let base = canonical_sha256(SAVED).unwrap();
    // A moved footprint.
    let moved = SAVED.replacen("(at ", "(at 1", 1);
    assert_ne!(moved, SAVED);
    assert_ne!(canonical_sha256(&moved).unwrap(), base);
    // The board's own generator is the board's, not a footprint's: it counts.
    let generator = SAVED.replacen("(generator \"pcbnew\")", "(generator \"other\")", 1);
    assert_ne!(generator, SAVED, "fixture has a board-level generator");
    assert_ne!(canonical_sha256(&generator).unwrap(), base);
    // An unknown field inside a footprint counts.
    let unknown = SAVED.replacen(
        "(footprint \"",
        "(footprint (unknown_field must_count) \"",
        1,
    );
    assert_ne!(unknown, SAVED);
    assert_ne!(canonical_sha256(&unknown).unwrap(), base);
}

#[test]
fn an_atom_and_a_string_of_one_text_differ() {
    assert_ne!(
        canonical_sha256("(kicad_pcb (a b))").unwrap(),
        canonical_sha256("(kicad_pcb (a \"b\"))").unwrap()
    );
    // Framing: without lengths, `aAb c` and `a bAc` (the tag byte inside an
    // atom) would write the same bytes.
    assert_ne!(
        canonical_sha256("(kicad_pcb (x aAb c))").unwrap(),
        canonical_sha256("(kicad_pcb (x a bAc))").unwrap()
    );
}

#[test]
fn a_text_that_does_not_parse_has_no_identity() {
    assert!(canonical_sha256("(kicad_pcb (unclosed").is_err());
}

fn rules() -> IpcEffectiveRoutingRules {
    ["GND", "VCC"]
        .into_iter()
        .map(|net| {
            (
                net.to_string(),
                IpcRoutingRules {
                    class_name: "Default".to_string(),
                    constituents: vec!["Default".to_string()],
                    track_width_mm: Some(0.25),
                    clearance_mm: Some(0.2),
                    via_diameter_mm: Some(0.6),
                    via_drill_mm: Some(0.3),
                },
            )
        })
        .collect()
}

/// End to end: a DSN exported from the IPC text of a board is judged against the
/// file KiCad saved for it. The byte hash cannot match; the canonical one does.
#[test]
fn the_judge_accepts_the_saved_file_of_a_board_exported_over_ipc() {
    let live = ipc_form(SAVED);
    let bundle =
        crate::specctra::export_dsn(std::path::Path::new("board.kicad_pcb"), &live, &rules())
            .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&bundle.manifest).unwrap();
    assert_eq!(
        manifest["source_canonical_sha256"].as_str(),
        Some(canonical_sha256(&live).unwrap().as_str())
    );
    assert_ne!(
        manifest["source_sha256"].as_str(),
        Some(format!("{:x}", Sha256::digest(SAVED.as_bytes())).as_str()),
        "the byte hash is of the IPC text, not the saved file"
    );
    assert!(crate::route_judge::saved_board_matches(
        SAVED.as_bytes(),
        &manifest
    ));
    // A saved board that is not the exported one is still refused.
    let moved = SAVED.replacen("(at ", "(at 1", 1);
    assert!(!crate::route_judge::saved_board_matches(
        moved.as_bytes(),
        &manifest
    ));
}

/// The one rule the judge's preflight and its import-plan call share.
#[test]
fn matches_export_takes_the_bytes_or_the_tree_and_nothing_else() {
    let live = ipc_form(SAVED);
    let byte_hash = |text: &str| format!("{:x}", Sha256::digest(text.as_bytes()));
    let tree = canonical_sha256(SAVED).unwrap();

    // The bytes of the recorded text, in either letter case, without a tree hash.
    assert!(matches_export(
        live.as_bytes(),
        Some(&byte_hash(&live)),
        None
    ));
    assert!(matches_export(
        live.as_bytes(),
        Some(&byte_hash(&live).to_uppercase()),
        None
    ));
    // The saved file of that board: other bytes, same tree.
    assert!(matches_export(
        SAVED.as_bytes(),
        Some(&byte_hash(&live)),
        Some(&tree)
    ));
    assert!(matches_export(
        SAVED.as_bytes(),
        Some(&byte_hash(&live)),
        Some(&tree.to_uppercase())
    ));
    // The tree hash alone, with no byte hash recorded.
    assert!(matches_export(SAVED.as_bytes(), None, Some(&tree)));

    // Controls. Other bytes and no tree hash: refused.
    assert!(!matches_export(
        SAVED.as_bytes(),
        Some(&byte_hash(&live)),
        None
    ));
    assert!(!matches_export(SAVED.as_bytes(), None, None));
    // A changed board: refused by both hashes.
    let moved = SAVED.replacen("(at ", "(at 1", 1);
    assert!(!matches_export(
        moved.as_bytes(),
        Some(&byte_hash(&live)),
        Some(&tree)
    ));
    // Text that does not parse, and bytes that are not text, match by bytes only.
    assert!(!matches_export(
        b"(kicad_pcb (unclosed",
        Some(&byte_hash(&live)),
        Some(&tree)
    ));
    assert!(!matches_export(
        &[0xff, 0xfe, 0x28],
        Some(&byte_hash(&live)),
        Some(&tree)
    ));
    assert!(matches_export(
        b"(kicad_pcb (unclosed",
        Some(&byte_hash("(kicad_pcb (unclosed")),
        Some(&tree)
    ));
}
