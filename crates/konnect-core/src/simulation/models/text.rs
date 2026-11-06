//! The text of a fitted model file, and the strict check every such file
//! passes before Konnect writes it, attaches it, or calls it fitted.
//!
//! KiCad's GUI simulator includes a model file directly into ngspice, with no
//! vetting and no dropped rights, and ngspice runs a comment such as `*#shell`
//! as a command. So a fitted file holds no caller text at all: one marker line
//! and cards in a small grammar Konnect writes, with numbers it formats. The
//! check below accepts that grammar and nothing else, so a file someone edited
//! into anything more is no longer a fitted file. Where the numbers came from
//! goes in a JSON sidecar, which no simulator reads.

use sha2::{Digest, Sha256};

/// Starts the first line of every fitted file.
pub(crate) const MARKER_PREFIX: &str = "* konnect-fitted-model v1 ";
pub(crate) const MAX_NAME: usize = 32;
/// The longest kind name a marker carries (`tvs_bidirectional` is 17).
const MAX_KIND: usize = 24;
/// Every fitted model's name ends so, so it never reads as a vendor's model in
/// KiCad, a BOM or a deck.
pub(crate) const NAME_SUFFIX: &str = "_fit";
/// Parameters on one continuation line.
const PARAMETERS_PER_LINE: usize = 4;
/// What a fitted file may hold, at most.
pub(crate) const MAX_FILE_BYTES: usize = 64 * 1024;
const MAX_LINES: usize = 400;
/// Parameters a fitted diode card may set.
pub(crate) const DIODE_PARAMETERS: [&str; 12] = [
    "IS", "N", "RS", "BV", "IBV", "NBV", "CJO", "VJ", "M", "EG", "XTI", "TNOM",
];

/// A model name every parser reads alike, that says it was fitted.
pub(crate) fn check_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let starts_with_letter = chars.next().is_some_and(|first| first.is_ascii_lowercase());
    let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !starts_with_letter || !rest_ok || name.len() > MAX_NAME {
        return Err(format!(
            "model_name must start with a lowercase letter and hold only lowercase letters, digits \
             and _, at most {MAX_NAME} characters"
        ));
    }
    if !name.ends_with(NAME_SUFFIX) || name.len() == NAME_SUFFIX.len() {
        return Err(format!(
            "model_name must end in {NAME_SUFFIX}, so the fitted model never reads as a vendor's"
        ));
    }
    // The suffix already keeps a name off every device type and every name
    // Windows reserves for a file.
    if name.contains("vdmos") {
        return Err("model_name must not contain vdmos, which ngspice misreads".to_string());
    }
    Ok(())
}

/// A number every parser reads the same way: plain scientific notation, never a
/// scale letter, because `M` is milli to ngspice and mega in KiCad's fields.
pub(crate) fn number(value: f64) -> String {
    format!("{value:.6e}")
}

/// Whether a token is a number as [`number`] writes it.
fn is_number(token: &str) -> bool {
    let unsigned = token.strip_prefix('-').unwrap_or(token);
    let Some((mantissa, exponent)) = unsigned.split_once('e') else {
        return false;
    };
    let exponent = exponent.strip_prefix('-').unwrap_or(exponent);
    let mantissa_ok = mantissa.len() == 8
        && mantissa.as_bytes()[0].is_ascii_digit()
        && mantissa.as_bytes()[1] == b'.'
        && mantissa[2..].bytes().all(|byte| byte.is_ascii_digit());
    mantissa_ok
        && !exponent.is_empty()
        && exponent.len() <= 3
        && exponent.bytes().all(|byte| byte.is_ascii_digit())
}

/// `.model NAME D` with its parameters on continuation lines.
pub(crate) fn diode_card(name: &str, parameters: &[(&'static str, f64)]) -> String {
    let mut card = format!(".model {name} D\n");
    for chunk in parameters.chunks(PARAMETERS_PER_LINE) {
        let line: Vec<String> = chunk
            .iter()
            .map(|(key, value)| format!("{key}={}", number(*value)))
            .collect();
        card.push_str("+ ");
        card.push_str(&line.join(" "));
        card.push('\n');
    }
    card
}

/// The whole file: the marker, whose hash covers every byte after it, then
/// the cards.
pub(crate) fn file(kind: &str, name: &str, body: &str) -> String {
    let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
    format!("{MARKER_PREFIX}kind={kind} name={name} body-sha256={hash} verdict=pass\n{body}")
}

/// The top-level card a fitted file defines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Card {
    /// `.model NAME D`, with pins A and K.
    Diode,
    /// `.subckt NAME ports...`.
    Subcircuit { ports: Vec<String> },
}

/// What the strict check read from a fitted file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fitted {
    pub kind: String,
    pub name: String,
    pub card: Card,
    pub body_sha256: String,
}

impl Fitted {
    /// The model's pins, in the order an instance lists its nodes.
    pub(crate) fn pins(&self) -> Vec<String> {
        match &self.card {
            Card::Diode => vec!["A".to_string(), "K".to_string()],
            Card::Subcircuit { ports } => ports.clone(),
        }
    }

    /// KiCad's `Sim.Device` for this library model, as its dialog writes it.
    pub(crate) fn sim_device(&self) -> &'static str {
        match self.card {
            Card::Diode => "D",
            Card::Subcircuit { .. } => "SUBCKT",
        }
    }
}

/// What a consumed model file says of itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FittedInput {
    pub path: std::path::PathBuf,
    /// The model's name, from the marker. A marker that names nothing a model
    /// can be called is read as unnamed.
    pub name: Option<String>,
    /// The file is still exactly what Konnect wrote.
    pub verified: bool,
    /// The names its `.model` and `.subckt` cards define, lowercase, read
    /// loosely so an edited file still names what it defines. For a verified
    /// file this is its name.
    pub defines: Vec<String>,
}

/// More names than one fitted file could define are not read.
const MAX_DEFINED: usize = 32;

/// The `.model` and `.subckt` names in a file, lowercase, read loosely.
fn defined_names(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut names: Vec<String> = Vec::new();
    for line in text.split(['\n', '\r']) {
        let mut words = line.split_whitespace();
        let card = words.next().map(str::to_ascii_lowercase);
        if let (Some(".model" | ".subckt"), Some(name)) = (card.as_deref(), words.next()) {
            let name = name.to_lowercase();
            if names.len() < MAX_DEFINED && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// A file starting with Konnect's marker, read from the bytes the deck
/// consumed. None for any other file.
pub(crate) fn fitted_input(path: &std::path::Path, bytes: &[u8]) -> Option<FittedInput> {
    // Konnect writes its marker as line 1. An edit may put a comment, a blank
    // line or a byte order mark above it, and ngspice reads on, so the file
    // still counts as fitted, though no longer as checked.
    let marker = bytes
        .split(|byte| matches!(byte, b'\n' | b'\r'))
        .map(bare_line)
        .find(|line| line.starts_with(MARKER_PREFIX.as_bytes()))?;
    let path = path.to_path_buf();
    if bytes.starts_with(MARKER_PREFIX.as_bytes()) {
        if let Ok(fitted) = check_fitted(bytes) {
            return Some(FittedInput {
                path,
                defines: vec![fitted.name.clone()],
                name: Some(fitted.name),
                verified: true,
            });
        }
    }
    let name = String::from_utf8_lossy(marker)
        .split(' ')
        .find_map(|part| part.strip_prefix("name=").map(str::to_string))
        .filter(|name| check_name(name).is_ok());
    Some(FittedInput {
        path,
        name,
        verified: false,
        defines: defined_names(bytes),
    })
}

/// A line without the byte order marks and blanks a reader skips before it.
fn bare_line(mut line: &[u8]) -> &[u8] {
    loop {
        if let Some(rest) = line.strip_prefix(b"\xef\xbb\xbf".as_slice()) {
            line = rest;
        } else if let Some((first, rest)) = line.split_first() {
            if first.is_ascii_whitespace() {
                line = rest;
            } else {
                return line;
            }
        } else {
            return line;
        }
    }
}

fn is_identifier(token: &str, max: usize) -> bool {
    !token.is_empty()
        && token.len() <= max
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_node(token: &str) -> bool {
    is_identifier(token, 8)
}

/// Accept a fitted file exactly as Konnect writes one, and nothing else.
pub(crate) fn check_fitted(bytes: &[u8]) -> Result<Fitted, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("larger than a fitted file can be".to_string());
    }
    if let Some(position) = bytes
        .iter()
        .position(|&byte| byte != b'\n' && !(0x20..=0x7e).contains(&byte))
    {
        return Err(format!(
            "byte {position} is not printable ASCII or a line feed"
        ));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "not text".to_string())?;
    if !text.ends_with('\n') {
        return Err("does not end with a line feed".to_string());
    }
    let (marker, body) = text
        .split_once('\n')
        .ok_or_else(|| "has no marker line".to_string())?;
    let fields = marker
        .strip_prefix(MARKER_PREFIX)
        .ok_or_else(|| "does not start with Konnect's marker".to_string())?;
    let field = |key: &str| -> Result<&str, String> {
        fields
            .split(' ')
            .find_map(|part| part.strip_prefix(key)?.strip_prefix('='))
            .ok_or_else(|| format!("the marker has no {key}"))
    };
    let (kind, name, hash, verdict) = (
        field("kind")?,
        field("name")?,
        field("body-sha256")?,
        field("verdict")?,
    );
    if fields.split(' ').count() != 4 || verdict != "pass" || !is_identifier(kind, MAX_KIND) {
        return Err("the marker is not one Konnect writes".to_string());
    }
    check_name(name)?;
    let actual = format!("{:x}", Sha256::digest(body.as_bytes()));
    if hash != actual {
        return Err("its cards were changed after Konnect wrote them".to_string());
    }
    let card = check_cards(body, name)?;
    Ok(Fitted {
        kind: kind.to_string(),
        name: name.to_string(),
        card,
        body_sha256: actual,
    })
}

/// The cards after the marker. Konnect writes either one diode card, or one
/// subcircuit holding R, L, C, D and sense lines and its own diode cards.
fn check_cards(body: &str, name: &str) -> Result<Card, String> {
    let lines: Vec<&str> = body.lines().collect();
    if lines.is_empty() || lines.len() > MAX_LINES {
        return Err("does not hold one card".to_string());
    }
    let first: Vec<&str> = lines[0].split(' ').collect();
    match first.as_slice() {
        [".model", card_name, "D"] if *card_name == name => {
            let parameter_lines = check_continuations(&lines[1..], 0)?;
            if 1 + parameter_lines != lines.len() {
                return Err(format!(
                    "line {} follows the diode card, where Konnect writes nothing",
                    parameter_lines + 3
                ));
            }
            Ok(Card::Diode)
        }
        [".subckt", card_name, ports @ ..] if *card_name == name => {
            if ports.is_empty() || ports.len() > 8 || !ports.iter().all(|port| is_node(port)) {
                return Err("its subcircuit ports are not ones Konnect writes".to_string());
            }
            let Some((last, inner)) = lines[1..].split_last() else {
                return Err("its subcircuit is not closed".to_string());
            };
            if *last != format!(".ends {name}") {
                return Err("its subcircuit is not closed by its own .ends".to_string());
            }
            check_subcircuit_body(inner, name)?;
            Ok(Card::Subcircuit {
                ports: ports.iter().map(|port| port.to_string()).collect(),
            })
        }
        _ => Err(format!(
            "its first card is not a {name} card Konnect writes"
        )),
    }
}

/// `+ KEY=number` lines after a `.model` card, from line `start` on.
fn check_continuations(lines: &[&str], start: usize) -> Result<usize, String> {
    let mut count = 0;
    for line in lines {
        let Some(rest) = line.strip_prefix("+ ") else {
            break;
        };
        let pairs: Vec<&str> = rest.split(' ').collect();
        if pairs.is_empty() || pairs.len() > PARAMETERS_PER_LINE {
            return Err(format!(
                "line {} is not a parameter line Konnect writes",
                start + count + 2
            ));
        }
        for pair in pairs {
            let ok = pair
                .split_once('=')
                .is_some_and(|(key, value)| DIODE_PARAMETERS.contains(&key) && is_number(value));
            if !ok {
                return Err(format!(
                    "line {} sets something a fitted diode does not",
                    start + count + 2
                ));
            }
        }
        count += 1;
    }
    if count == 0 {
        return Err("a model card has no parameters".to_string());
    }
    Ok(count)
}

fn check_subcircuit_body(lines: &[&str], name: &str) -> Result<(), String> {
    let inner_models: Vec<String> = lines
        .iter()
        .filter_map(|line| line.strip_prefix(".model "))
        .filter_map(|rest| rest.strip_suffix(" D"))
        .map(str::to_string)
        .collect();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let tokens: Vec<&str> = line.split(' ').collect();
        let number_line = |tokens: &[&str]| matches!(tokens, [_, a, b, value] if is_node(a) && is_node(b) && is_number(value));
        let ok = match tokens.first().copied() {
            Some(".model") => {
                let model = tokens.get(1).copied().unwrap_or_default();
                let named = model
                    .strip_prefix(name)
                    .is_some_and(|rest| matches!(rest, "_f" | "_b"));
                if !(tokens.len() == 3 && tokens[2] == "D" && named) {
                    return Err(format!("line {} is not a model Konnect writes", index + 2));
                }
                index += 1 + check_continuations(&lines[index + 1..], index + 1)?;
                continue;
            }
            Some("VKONNECT") => {
                matches!(tokens.as_slice(), [_, a, b, "0"] if is_node(a) && is_node(b))
            }
            Some("R1" | "R2" | "RL" | "L1" | "C1") => number_line(&tokens),
            Some("DF" | "DB" | "DG" | "DH") => matches!(
                tokens.as_slice(),
                [_, a, b, model] if is_node(a) && is_node(b) && inner_models.iter().any(|m| m == model)
            ),
            _ => false,
        };
        if !ok {
            return Err(format!("line {} is not one Konnect writes", index + 2));
        }
        index += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diode_file() -> String {
        let card = diode_card(
            "bat54_fit",
            &[
                ("IS", 1e-8),
                ("N", 1.05),
                ("RS", 0.5),
                ("BV", 30.0),
                ("TNOM", 25.0),
            ],
        );
        file("schottky_diode", "bat54_fit", &card)
    }

    fn zener_file() -> String {
        let body = format!(
            ".subckt z5v1_fit A K\nDF A K z5v1_fit_f\nDB A K z5v1_fit_b\n{}{}.ends z5v1_fit\n",
            diode_card("z5v1_fit_f", &[("IS", 1e-14), ("N", 1.0)]),
            diode_card("z5v1_fit_b", &[("IS", 1e-20), ("BV", 5.0)])
        );
        file("zener", "z5v1_fit", &body)
    }

    fn fuse_file() -> String {
        file(
            "resistance",
            "fuse_fit",
            ".subckt fuse_fit 1 2\nVKONNECT 1 n1 0\nR1 n1 2 1.200000e-2\n.ends fuse_fit\n",
        )
    }

    #[test]
    fn a_marker_below_an_edit_still_marks_the_file_as_fitted_but_unchecked() {
        let body = diode_card("bat54_fit", &[("IS", 1e-9), ("N", 1.05)]);
        let clean = file("diode", "bat54_fit", &body);
        let path = std::path::Path::new("bat54_fit.lib");
        let read = fitted_input(path, clean.as_bytes()).unwrap();
        assert!(read.verified);
        for edited in [
            format!("* bench note\n{clean}"),
            format!("\u{feff}{clean}"),
            format!("\n  {clean}"),
            format!("* bench note\r{clean}"),
        ] {
            let read = fitted_input(path, edited.as_bytes()).unwrap();
            assert!(!read.verified, "{edited:?}");
            assert_eq!(read.name.as_deref(), Some("bat54_fit"));
            assert_eq!(read.defines, ["bat54_fit"]);
        }
        assert!(fitted_input(path, body.as_bytes()).is_none());
    }

    #[test]
    fn names_say_they_were_fitted() {
        assert!(check_name("bat54_fit").is_ok());
        assert!(check_name("fuse_f1_fit").is_ok());
        for bad in [
            "",
            "_fit",
            "BAT54_fit",
            "bat54",
            "1n4148_fit",
            "a b_fit",
            "x-y_fit",
            "irfvdmos_fit",
            "bat54_fit\n",
            "bat54_fit_b",
        ] {
            assert!(check_name(bad).is_err(), "{bad:?} passed");
        }
        assert!(check_name(&format!("{}_fit", "a".repeat(MAX_NAME - 4))).is_ok());
        assert!(check_name(&format!("{}_fit", "a".repeat(MAX_NAME - 3))).is_err());
    }

    #[test]
    fn numbers_carry_no_scale_letters() {
        assert_eq!(number(1.5e-9), "1.500000e-9");
        assert_eq!(number(-12.0), "-1.200000e1");
        assert_eq!(number(0.0), "0.000000e0");
        for value in [1e6, 2.2e-3, 47e3, -1e-300, 1e300] {
            assert!(is_number(&number(value)), "{}", number(value));
        }
        for bad in [
            "1m",
            "1.5",
            "1e3",
            "1.000000e",
            "1.000000eM",
            "1.000000e1k",
            "NaN",
        ] {
            assert!(!is_number(bad), "{bad}");
        }
    }

    #[test]
    fn every_file_konnect_writes_passes_its_own_check() {
        let diode = check_fitted(diode_file().as_bytes()).unwrap();
        assert_eq!(diode.card, Card::Diode);
        assert_eq!(diode.pins(), ["A", "K"]);
        assert_eq!(diode.sim_device(), "D");
        let zener = check_fitted(zener_file().as_bytes()).unwrap();
        assert_eq!(zener.pins(), ["A", "K"]);
        assert_eq!(zener.sim_device(), "SUBCKT");
        let fuse = check_fitted(fuse_file().as_bytes()).unwrap();
        assert_eq!(fuse.pins(), ["1", "2"]);
        assert_eq!(fuse.kind, "resistance");
    }

    /// Every way a line becomes a command, or a file stops being the one
    /// Konnect wrote, is refused. The flattener alone removes `*#` lines
    /// rather than refusing them, and KiCad's GUI never runs the flattener.
    #[test]
    fn a_fitted_file_that_could_run_a_command_is_refused() {
        let good = diode_file();
        let (_, body) = good.split_once('\n').unwrap();
        // Re-hash the body, so only the grammar can refuse it.
        let rehash = |body: &str| file("schottky_diode", "bat54_fit", body);
        for hostile in [
            "*#shell echo KMARK\n",
            "  *#shell echo KMARK\n",
            ";#echo KMARK\n",
            "=#echo KMARK\n",
            "(#echo KMARK\n",
            "r#echo KMARK\n",
            ".control\nshell echo KMARK\n.endc\n",
            ".include \"/etc/passwd\"\n",
            ".lib x.lib tt\n",
            ".title *ng_script\n",
            ".end\n",
            ".param x=1\n",
            ".options interp\n",
            "* a comment\n",
            "\n",
        ] {
            let text = rehash(&format!("{body}{hostile}"));
            assert!(check_fitted(text.as_bytes()).is_err(), "{hostile:?} passed");
        }
        for byte in ["\r", "\u{c}", "\u{b}", "\u{feff}", "\0", "é"] {
            let text = rehash(&body.replacen("IS=", &format!("{byte}IS="), 1));
            assert!(check_fitted(text.as_bytes()).is_err(), "{byte:?} passed");
        }
    }

    #[test]
    fn an_edited_or_foreign_file_is_not_a_fitted_one() {
        let good = diode_file();
        assert!(check_fitted(good.replace("N=1.050000e0", "N=1.100000e0").as_bytes()).is_err());
        assert!(check_fitted(good.replace("verdict=pass", "verdict=fail").as_bytes()).is_err());
        assert!(check_fitted(b".model bat54_fit D\n+ IS=1.000000e-8\n").is_err());
        assert!(check_fitted(good.trim_end().as_bytes()).is_err());
        // A parameter a fitted diode never sets, though the hash agrees.
        let body = diode_card("bat54_fit", &[("IS", 1e-8)]).replace("IS=", "LEVEL=");
        assert!(check_fitted(file("diode", "bat54_fit", &body).as_bytes()).is_err());
        // A subcircuit element Konnect never writes.
        let body = ".subckt fuse_fit 1 2\nB1 1 2 I=1\n.ends fuse_fit\n";
        assert!(check_fitted(file("resistance", "fuse_fit", body).as_bytes()).is_err());
        // A card under another name than the marker's.
        let body = diode_card("other_fit", &[("IS", 1e-8)]);
        assert!(check_fitted(file("diode", "bat54_fit", &body).as_bytes()).is_err());
    }
}
