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

#[test]
fn every_line_of_every_tested_deck_is_in_the_skill() {
    let skill = skill();
    let mut decks = 0;
    for entry in std::fs::read_dir(fixtures()).unwrap().flatten() {
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "sp")
        {
            continue;
        }
        decks += 1;
        let text = std::fs::read_to_string(entry.path()).unwrap();
        for line in text.lines() {
            assert!(
                skill.lines().any(|shown| shown == line),
                "{} is not in the skill: {line}",
                entry.path().display()
            );
        }
    }
    assert_eq!(decks, 6, "the five scenarios and the spliced example");
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
    // No stand-in adds a card that Konnect refuses or that changes the circuit.
    for line in section
        .lines()
        .filter(|line| line.starts_with('.') || line.starts_with("K"))
    {
        let line = line.to_lowercase();
        for refused in [".probe", ".ic ", ".nodeset", ".options"] {
            assert!(!line.starts_with(refused), "{line}");
        }
    }
}
