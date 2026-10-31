//! The protection stand-ins in the `kicad-simulation` skill are the decks the live
//! tests run, not a copy that can drift from them.

use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../konnect-core/tests/fixtures/simulation/protection")
}

fn skill() -> String {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/skills/kicad-simulation/SKILL.md");
    std::fs::read_to_string(path).unwrap()
}

/// The text of every fenced block in a document, one string per block.
fn code_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut open: Option<Vec<&str>> = None;
    for line in text.lines() {
        if line.starts_with("```") {
            match open.take() {
                Some(lines) => blocks.push(lines.join("\n")),
                None => open = Some(Vec::new()),
            }
        } else if let Some(lines) = open.as_mut() {
            lines.push(line);
        }
    }
    blocks
}

fn decks() -> Vec<(String, String)> {
    let mut decks = Vec::new();
    for entry in std::fs::read_dir(fixtures()).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "sp") {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            decks.push((
                name,
                std::fs::read_to_string(&path)
                    .unwrap()
                    .trim_end()
                    .to_owned(),
            ));
        }
    }
    decks.sort();
    decks
}

#[test]
fn every_tested_deck_is_one_whole_block_of_the_skill() {
    let blocks = code_blocks(&skill());
    let decks = decks();
    assert_eq!(decks.len(), 6, "the five scenarios and the spliced example");
    for (name, deck) in &decks {
        assert!(
            blocks.iter().any(|block| block == deck),
            "{name}.sp is not a block of the skill, line for line and in order"
        );
    }
}

#[test]
fn a_deck_adds_no_card_that_konnect_refuses_or_that_changes_the_circuit() {
    for (name, deck) in decks() {
        for line in deck.lines().filter(|line| !line.starts_with('*')) {
            let line = line.to_lowercase();
            let refused = [".probe", ".ic ", ".nodeset", ".options", ".control"];
            let coupling = line.starts_with('k');
            assert!(
                !coupling && !refused.iter().any(|card| line.starts_with(card)),
                "{name}: {line}"
            );
        }
    }
}

#[test]
fn the_skill_says_what_a_stand_in_is_and_is_not() {
    let skill = skill();
    let section = &skill[skill.find("## Protection Scenarios").unwrap()..];
    let section = &section[..section.find("\n## Fixing").unwrap()];
    for needed in [
        "approximation",
        "not a model of a specific part",
        "no thermal behaviour",
        "simulated_circuit: \"modified\"",
        "**",
    ] {
        assert!(
            section.contains(needed),
            "the section does not say {needed:?}"
        );
    }
    for scenario in [
        "### Output short",
        "### I²t fuse",
        "### eFuse current limit",
        "### Undervoltage lockout",
        "### Reversed battery",
    ] {
        assert!(section.contains(scenario), "no {scenario}");
    }
}
