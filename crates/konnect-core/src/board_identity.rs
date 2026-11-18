//! One board, two texts: KiCad's in-memory board as IPC serializes it, and the
//! file KiCad saves. They differ in ways that are not the board: layout and
//! whitespace, and the standalone-library metadata (`version`, `generator`,
//! `generator_version`) IPC writes inside each embedded footprint and a saved
//! board leaves out. A byte hash of one never matches the other, so a check that
//! the saved board is the board a DSN was exported from needs an identity both
//! texts share.
//!
//! [`canonical_sha256`] is that identity: the SHA-256 of the parsed tree after
//! [`normalize_snapshot_metadata`], written in an unambiguous framing. Any
//! geometry, net, UUID, model or unknown field still changes it.

use anyhow::{Context, Result};
use konnect_sexp::SexpNode;
use sha2::{Digest, Sha256};

/// Drop what IPC writes and a saved board omits: the `version`, `generator` and
/// `generator_version` children of every `footprint` list. Nothing else is
/// touched. Never discard geometry, models, connectivity, UUIDs or arbitrary
/// unknown fields to make a comparison pass.
pub(crate) fn normalize_snapshot_metadata(node: &mut SexpNode) {
    let footprint = node.head() == Some("footprint");
    if let SexpNode::List(children) = node {
        if footprint {
            children.retain(|child| {
                !matches!(
                    child.head(),
                    Some("version" | "generator" | "generator_version")
                )
            });
        }
        for child in children {
            normalize_snapshot_metadata(child);
        }
    }
}

/// The SHA-256 (lowercase hex) of a board text as a tree: parsed, normalized by
/// [`normalize_snapshot_metadata`], and framed so that no two different trees
/// hash the same input. KiCad's in-memory text of a board and the file it saves
/// for that board give the same value; any change to the board itself does not.
///
/// Errors when the text does not parse.
pub(crate) fn canonical_sha256(board_text: &str) -> Result<String> {
    let mut tree = konnect_sexp::parse_sexp(board_text).context("the board text does not parse")?;
    normalize_snapshot_metadata(&mut tree);
    let mut hasher = Sha256::new();
    feed(&mut hasher, &tree);
    Ok(format!("{:x}", hasher.finalize()))
}

/// Writes a node as a tag byte, then a length, then its content: `A` and `S`
/// carry the byte length and bytes of an atom or a quoted string, `L` the child
/// count and then each child. An atom and a string of the same text differ.
fn feed(hasher: &mut Sha256, node: &SexpNode) {
    match node {
        SexpNode::Atom(text) => {
            hasher.update(b"A");
            hasher.update((text.len() as u64).to_le_bytes());
            hasher.update(text.as_bytes());
        }
        SexpNode::Str(text) => {
            hasher.update(b"S");
            hasher.update((text.len() as u64).to_le_bytes());
            hasher.update(text.as_bytes());
        }
        SexpNode::List(children) => {
            hasher.update(b"L");
            hasher.update((children.len() as u64).to_le_bytes());
            for child in children {
                feed(hasher, child);
            }
        }
    }
}

#[cfg(test)]
#[path = "board_identity_tests.rs"]
pub(crate) mod tests;
