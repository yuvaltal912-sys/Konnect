//! Row S9: start a routing pass from an earlier pass's copper.
//!
//! [`seed_dsn`] adds a session's wires and vias to a DSN's `wiring` as
//! ordinary wiring, with no `(type ...)` at all, so Freerouting may move, rip
//! up and optimize them like copper it laid itself. Nothing is ever seeded as
//! fixed: Freerouting neither optimizes fixed wiring nor writes it back to
//! its session, and the session of a seeded pass must be whole.
//!
//! The DSN's own bytes are kept: the new items are inserted before the
//! closing parenthesis of its `wiring` scope (or a `wiring` scope is added
//! before the end of the file) and nothing else is rewritten.

use anyhow::{bail, Context, Result};
use konnect_sexp::SexpNode;
use std::collections::BTreeSet;

/// A DSN with a session's copper added as ordinary wiring.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Seeded {
    pub dsn: String,
    pub wires: usize,
    pub vias: usize,
    /// The nets the seeded copper belongs to.
    pub nets: BTreeSet<String>,
}

/// What a session holds, for checking a seeded pass's own session.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SessionWiring {
    pub wires: usize,
    pub vias: usize,
    /// The nets that have at least one wire or via.
    pub nets: BTreeSet<String>,
    /// Wires and vias that carry a `(type ...)`. The importer refuses them.
    pub typed: usize,
}

/// Micrometres per unit, for the units a DSN or a session may name.
fn unit_um(unit: &str) -> Option<f64> {
    match unit.to_ascii_lowercase().as_str() {
        "um" => Some(1.0),
        "mm" => Some(1000.0),
        "mil" => Some(25.4),
        "inch" => Some(25_400.0),
        _ => None,
    }
}

/// The DSN's one quote delimiter is written as the bare token
/// `(string_quote ")`, which no S-expression reader balances. For reading
/// only, that token is given a closed form; the text that is written keeps
/// the original bytes.
fn parse_specctra(source: &str, what: &str) -> Result<SexpNode> {
    let readable = source.replace("(string_quote \")", "(string_quote \"\\\"\")");
    konnect_sexp::parse_sexp(&readable)
        .map_err(|_| anyhow::anyhow!("the {what} is not a readable S-expression"))
}

fn atom<'a>(node: &'a SexpNode, index: usize, what: &str) -> Result<&'a str> {
    node.get(index)
        .and_then(SexpNode::as_str)
        .with_context(|| format!("{what} is missing"))
}

fn lists<'a>(node: &'a SexpNode, head: &'static str) -> impl Iterator<Item = &'a SexpNode> + 'a {
    node.children()
        .unwrap_or(&[])
        .iter()
        .filter(move |child| child.head() == Some(head))
}

/// A name as a DSN token: bare when it was bare, quoted when it was quoted.
fn token(node: &SexpNode) -> Result<String> {
    match node {
        SexpNode::Atom(text) => Ok(text.clone()),
        SexpNode::Str(text) if !text.contains('"') => Ok(format!("\"{text}\"")),
        SexpNode::Str(_) => bail!("a session name holds a quote character"),
        SexpNode::List(_) => bail!("the session holds a list where a name belongs"),
    }
}

/// A coordinate or width in DSN units: at most four decimals, none trailing.
fn dsn_number(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    // "-0.0000" trims to "-0"; nothing trims to an empty text, because the
    // digit before the point stays.
    if text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

fn scaled(node: &SexpNode, scale: f64, what: &str) -> Result<String> {
    let text = node
        .as_str()
        .with_context(|| format!("the session's {what} is not a number"))?;
    let value: f64 = text
        .parse()
        .map_err(|_| anyhow::anyhow!("the session's {what} is not a number"))?;
    if !value.is_finite() {
        bail!("the session's {what} is not finite");
    }
    Ok(dsn_number(value * scale))
}

/// The routes of a session and micrometres per session unit.
fn session_routes(session: &SexpNode) -> Result<(&SexpNode, f64)> {
    if session.head() != Some("session") {
        bail!("the file is not a Specctra session");
    }
    let routes = session
        .find("routes")
        .context("the session has no routes")?;
    let resolution = routes
        .find("resolution")
        .context("the session's routes have no resolution")?;
    let unit = atom(resolution, 1, "the session's resolution unit")?;
    let per_unit: f64 = atom(resolution, 2, "the session's resolution")?
        .parse()
        .map_err(|_| anyhow::anyhow!("the session's resolution is not a number"))?;
    let um = unit_um(unit).with_context(|| format!("the session's unit '{unit}' is not known"))?;
    if !per_unit.is_finite() || per_unit <= 0.0 {
        bail!("the session's resolution is not positive");
    }
    Ok((routes, um / per_unit))
}

/// What a session holds: its wires, vias, the nets that have any, and how
/// many items carry a type.
pub(crate) fn session_wiring(ses: &str) -> Result<SessionWiring> {
    let session = parse_specctra(ses, "session")?;
    let (routes, _) = session_routes(&session)?;
    let mut found = SessionWiring::default();
    for network in lists(routes, "network_out") {
        for net in lists(network, "net") {
            let name = atom(net, 1, "a session net's name")?;
            for (head, count) in [("wire", &mut found.wires), ("via", &mut found.vias)] {
                for item in lists(net, head) {
                    *count += 1;
                    found.nets.insert(name.to_string());
                    if item.find("type").is_some() {
                        found.typed += 1;
                    }
                }
            }
        }
    }
    Ok(found)
}

/// Where the DSN's `wiring` scope closes, or failing that where the root
/// closes: byte offsets of those closing parentheses in the original text.
struct DsnEnds {
    wiring_close: Option<usize>,
    root_close: usize,
}

/// Scan the DSN's own text for the ends of its top-level scopes. Quoted
/// tokens are skipped (a DSN has no escape inside a quoted token), and so is
/// the bare `(string_quote ")`.
fn dsn_ends(text: &str) -> Result<DsnEnds> {
    const QUOTE_TOKEN: &str = "(string_quote \")";
    let bytes = text.as_bytes();
    let (mut at, mut depth) = (0usize, 0usize);
    let mut top_start: Option<usize> = None;
    let mut wiring_close = None;
    let mut root_close = None;
    while at < bytes.len() {
        if text[at..].starts_with(QUOTE_TOKEN) {
            at += QUOTE_TOKEN.len();
            continue;
        }
        match bytes[at] {
            b'"' => {
                let end = text[at + 1..]
                    .find('"')
                    .context("the DSN holds an unclosed quoted token")?;
                at += end + 1;
            }
            b'(' => {
                depth += 1;
                if depth == 2 {
                    top_start = Some(at);
                }
            }
            b')' => {
                if depth == 0 {
                    bail!("the DSN's parentheses do not balance");
                }
                if depth == 2 {
                    let start = top_start.take().context("the DSN's scopes do not nest")?;
                    let head = text[start + 1..at]
                        .split(|c: char| c.is_whitespace() || c == '(' || c == ')')
                        .next()
                        .unwrap_or("");
                    if head == "wiring" {
                        if wiring_close.is_some() {
                            bail!("the DSN holds more than one wiring scope");
                        }
                        wiring_close = Some(at);
                    }
                }
                depth -= 1;
                if depth == 0 {
                    if root_close.is_some() {
                        bail!("the DSN holds more than one root");
                    }
                    root_close = Some(at);
                }
            }
            _ => {}
        }
        at += 1;
    }
    if depth != 0 {
        bail!("the DSN's parentheses do not balance");
    }
    let root_close = root_close.context("the DSN has no root scope")?;
    if !text[root_close + 1..].trim().is_empty() {
        bail!("the DSN holds text after its root scope");
    }
    Ok(DsnEnds {
        wiring_close,
        root_close,
    })
}

/// `dsn` with the wires and vias of `ses` added to its wiring as ordinary
/// wiring. Refused: a session net, layer or via padstack the DSN does not
/// have; a session item that carries a type; an arc; a DSN with no `unit`.
/// A session with no copper gives the DSN back unchanged.
pub(crate) fn seed_dsn(dsn: &str, ses: &str) -> Result<Seeded> {
    let pcb = parse_specctra(dsn, "DSN")?;
    if pcb.head() != Some("pcb") {
        bail!("the DSN's root is not 'pcb'");
    }
    let unit = lists(&pcb, "unit")
        .next()
        .context("the DSN names no unit")?;
    let unit = atom(unit, 1, "the DSN's unit")?;
    let dsn_um = unit_um(unit).with_context(|| format!("the DSN's unit '{unit}' is not known"))?;
    fn names<'a>(pcb: &'a SexpNode, scope: &'static str, item: &'static str) -> BTreeSet<&'a str> {
        lists(pcb, scope)
            .flat_map(|scope| lists(scope, item))
            .filter_map(|node| node.get(1).and_then(SexpNode::as_str))
            .collect()
    }
    let layers = names(&pcb, "structure", "layer");
    let padstacks = names(&pcb, "library", "padstack");
    let nets = names(&pcb, "network", "net");

    let session = parse_specctra(ses, "session")?;
    let (routes, session_um) = session_routes(&session)?;
    let scale = session_um / dsn_um;

    let mut items = String::new();
    let mut seeded = Seeded {
        dsn: String::new(),
        wires: 0,
        vias: 0,
        nets: BTreeSet::new(),
    };
    for network in lists(routes, "network_out") {
        for net in lists(network, "net") {
            let net_node = net.get(1).context("a session net has no name")?;
            let name = atom(net, 1, "a session net's name")?;
            if !nets.contains(name) {
                bail!("the session routes net '{name}', which the DSN does not have");
            }
            let net_token = token(net_node)?;
            for wire in lists(net, "wire") {
                if wire.find("type").is_some() {
                    bail!("the session holds a typed wire on net '{name}'; only ordinary wiring is seeded");
                }
                if wire.find("qarc").is_some() {
                    bail!(
                        "the session holds an arc on net '{name}', which seeding does not support"
                    );
                }
                for path in lists(wire, "path") {
                    let children = path.children().unwrap_or(&[]);
                    // The head, a layer, a width and whole points: an odd count.
                    if children.len() < 7 || children.len().is_multiple_of(2) {
                        bail!("a session path on net '{name}' does not hold complete point pairs");
                    }
                    let layer = atom(path, 1, "a session path's layer")?;
                    if !layers.contains(layer) {
                        bail!("the session uses layer '{layer}', which the DSN does not have");
                    }
                    let values: Vec<String> = children[2..]
                        .iter()
                        .map(|value| scaled(value, scale, "path value"))
                        .collect::<Result<_>>()?;
                    items.push_str(&format!(
                        "    (wire\n      (path {} {})\n      (net {net_token})\n    )\n",
                        token(&children[1])?,
                        values.join(" ")
                    ));
                    seeded.wires += 1;
                    seeded.nets.insert(name.to_string());
                }
            }
            for via in lists(net, "via") {
                if via.find("type").is_some() {
                    bail!("the session holds a typed via on net '{name}'; only ordinary wiring is seeded");
                }
                let children = via.children().unwrap_or(&[]);
                if children.len() < 4 {
                    bail!("a session via on net '{name}' has no position");
                }
                let padstack = atom(via, 1, "a session via's padstack")?;
                if !padstacks.contains(padstack) {
                    bail!(
                        "the session uses via '{padstack}', which the DSN's library does not have"
                    );
                }
                items.push_str(&format!(
                    "    (via {} {} {}\n      (net {net_token})\n    )\n",
                    token(&children[1])?,
                    scaled(&children[2], scale, "via x")?,
                    scaled(&children[3], scale, "via y")?,
                ));
                seeded.vias += 1;
                seeded.nets.insert(name.to_string());
            }
        }
    }
    if items.is_empty() {
        seeded.dsn = dsn.to_string();
        return Ok(seeded);
    }
    let ends = dsn_ends(dsn)?;
    seeded.dsn = match ends.wiring_close {
        Some(close) => {
            // `(wiring)` on one line, or a scope that already ends on its own
            // line: either way the new items start on a fresh line.
            let before = dsn[..close].trim_end_matches([' ', '\t']);
            let newline = if before.ends_with('\n') { "" } else { "\n" };
            format!("{before}{newline}{items}  {}", &dsn[close..])
        }
        None => {
            let before = dsn[..ends.root_close].trim_end_matches([' ', '\t']);
            let newline = if before.ends_with('\n') { "" } else { "\n" };
            format!(
                "{before}{newline}  (wiring\n{items}  )\n{}",
                &dsn[ends.root_close..]
            )
        }
    };
    // What is written must read back, with exactly the new items added.
    let wiring_items = |text: &str| -> Result<usize> {
        let tree = parse_specctra(text, "seeded DSN")?;
        Ok(lists(&tree, "wiring")
            .map(|wiring| wiring.children().map_or(0, |children| children.len() - 1))
            .sum())
    };
    if wiring_items(&seeded.dsn)? != wiring_items(dsn)? + seeded.wires + seeded.vias {
        bail!("the seeded DSN does not read back with the session's items added");
    }
    Ok(seeded)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DSN: &str = "(pcb board.kicad_pcb\n  (parser\n    (string_quote \")\n    (space_in_quoted_tokens on)\n  )\n  (resolution um 10)\n  (unit um)\n  (structure\n    (layer F.Cu\n      (type signal)\n    )\n    (layer B.Cu\n      (type signal)\n    )\n  )\n  (library\n    (padstack konnect_via_0001\n      (shape (circle F.Cu 600))\n    )\n  )\n  (network\n    (net VBUS\n      (pins U1-1 J1-1)\n    )\n    (net \"A (odd) net\"\n      (pins U1-2 J1-2)\n    )\n    (class power VBUS\n      (rule\n        (width 1000)\n      )\n    )\n  )\n  (wiring)\n)\n";

    const SES: &str = "(session board (routes (resolution um 10) (network_out (net VBUS (wire (path F.Cu 10000 0 0 30000 0 30000 -25004)) (via \"konnect_via_0001\" 30000 -25004)) (net \"A (odd) net\" (wire (path B.Cu 2000 1 2 3 4))))))";

    #[test]
    fn a_sessions_copper_becomes_ordinary_wiring_and_nothing_else_changes() {
        let seeded = seed_dsn(DSN, SES).unwrap();
        assert_eq!((seeded.wires, seeded.vias), (2, 1));
        assert_eq!(
            seeded.nets.iter().map(String::as_str).collect::<Vec<_>>(),
            ["A (odd) net", "VBUS"]
        );
        // Nothing is ever seeded as fixed, or with any type.
        let added = seeded.dsn.split("  (wiring").nth(1).unwrap();
        assert!(!added.contains("(type"), "{added}");
        // Session units (a tenth of a micrometre) become DSN units (um).
        let expected = "  (wiring\n    (wire\n      (path F.Cu 1000 0 0 3000 0 3000 -2500.4)\n      (net VBUS)\n    )\n    (via \"konnect_via_0001\" 3000 -2500.4\n      (net VBUS)\n    )\n    (wire\n      (path B.Cu 200 0.1 0.2 0.3 0.4)\n      (net \"A (odd) net\")\n    )\n  )\n)\n";
        assert!(seeded.dsn.ends_with(expected), "{}", seeded.dsn);
        // Every byte before the wiring scope is the export's own.
        let head = DSN.strip_suffix("  (wiring)\n)\n").unwrap();
        assert_eq!(seeded.dsn.strip_suffix(expected).unwrap(), head);
        // And it reads back as what was written.
        let back = session_wiring(SES).unwrap();
        assert_eq!((back.wires, back.vias, back.typed), (2, 1, 0));
        assert_eq!(back.nets, seeded.nets);
    }

    #[test]
    fn wiring_the_dsn_already_holds_is_kept_and_the_seed_follows_it() {
        let fixed = "  (wiring\n    (wire\n      (path F.Cu 250 5 5 6 6)\n      (net VBUS)\n      (type fix)\n    )\n  )\n)\n";
        let dsn = DSN.replace("  (wiring)\n)\n", fixed);
        let seeded = seed_dsn(&dsn, SES).unwrap();
        // The board's own locked copper stays fixed; the seed has no type.
        assert_eq!(seeded.dsn.matches("(type fix)").count(), 1);
        let kept = seeded.dsn.find("(path F.Cu 250 5 5 6 6)").unwrap();
        let first_new = seeded.dsn.find("(path F.Cu 1000 0 0 3000 0").unwrap();
        assert!(kept < first_new);
        assert!(seeded
            .dsn
            .starts_with(dsn.strip_suffix("  )\n)\n").unwrap()));
        assert!(seeded
            .dsn
            .ends_with("      (net \"A (odd) net\")\n    )\n  )\n)\n"));
        // The seed went into the scope that was there, not into a second one.
        assert_eq!(seeded.dsn.matches("(wiring").count(), 1, "{}", seeded.dsn);
    }

    #[test]
    fn a_dsn_without_a_wiring_scope_gets_one() {
        let dsn = DSN.replace("  (wiring)\n", "");
        let seeded = seed_dsn(&dsn, SES).unwrap();
        assert!(seeded.dsn.starts_with(dsn.strip_suffix(")\n").unwrap()));
        assert!(seeded.dsn.contains("\n  (wiring\n    (wire\n"));
        assert!(seeded.dsn.ends_with("    )\n  )\n)\n"));
        assert_eq!(seeded.wires, 2);
    }

    #[test]
    fn a_session_with_no_copper_gives_the_dsn_back_unchanged() {
        let empty = "(session board (routes (resolution um 10) (network_out)))";
        let seeded = seed_dsn(DSN, empty).unwrap();
        assert_eq!(seeded.dsn, DSN);
        assert_eq!((seeded.wires, seeded.vias), (0, 0));
        assert!(seeded.nets.is_empty());
    }

    #[test]
    fn the_units_of_both_files_set_the_scale() {
        // A session in mm at 1000 per mm is in micrometres; the DSN in mm.
        let dsn = DSN.replace("(unit um)", "(unit mm)");
        let ses = "(session b (routes (resolution mm 1000) (network_out (net VBUS (wire (path F.Cu 1000 0 0 3000 -2500))))))";
        let seeded = seed_dsn(&dsn, ses).unwrap();
        assert!(
            seeded.dsn.contains("(path F.Cu 1 0 0 3 -2.5)"),
            "{}",
            seeded.dsn
        );
        // The same session into the DSN in micrometres.
        let seeded = seed_dsn(DSN, ses).unwrap();
        assert!(
            seeded.dsn.contains("(path F.Cu 1000 0 0 3000 -2500)"),
            "{}",
            seeded.dsn
        );
        // Mils and inches: 25.4 um and 25400 um.
        let ses = "(session b (routes (resolution mil 10) (network_out (net VBUS (wire (path F.Cu 100 0 0 1000 -500))))))";
        assert!(seed_dsn(DSN, ses)
            .unwrap()
            .dsn
            .contains("(path F.Cu 254 0 0 2540 -1270)"));
        let ses = "(session b (routes (resolution inch 1000) (network_out (net VBUS (wire (path F.Cu 10 0 0 100 -50))))))";
        assert!(seed_dsn(DSN, ses)
            .unwrap()
            .dsn
            .contains("(path F.Cu 254 0 0 2540 -1270)"));
        let inches = DSN.replace("(unit um)", "(unit inch)");
        assert!(seed_dsn(&inches, ses)
            .unwrap()
            .dsn
            .contains("(path F.Cu 0.01 0 0 0.1 -0.05)"));
        // A value that rounds to nothing is written as 0, never "-0".
        let ses = "(session b (routes (resolution um 10) (network_out (net VBUS (wire (path F.Cu 2000 -0.0001 0 3000 -0))))))";
        assert!(seed_dsn(DSN, ses)
            .unwrap()
            .dsn
            .contains("(path F.Cu 200 0 0 300 0)"));
        // A session at one unit per micrometre is copied as it is.
        let ses = "(session b (routes (resolution um 1) (network_out (net VBUS (wire (path F.Cu 1000 0 0 3000 -2500))))))";
        assert!(seed_dsn(DSN, ses)
            .unwrap()
            .dsn
            .contains("(path F.Cu 1000 0 0 3000 -2500)"));
    }

    #[test]
    fn what_the_dsn_does_not_have_is_refused() {
        let ses =
            |body: &str| format!("(session b (routes (resolution um 10) (network_out {body})))");
        for (body, needle) in [
            (
                "(net OTHER (wire (path F.Cu 2000 0 0 10 10)))",
                "net 'OTHER', which the DSN does not have",
            ),
            (
                "(net VBUS (wire (path In1.Cu 2000 0 0 10 10)))",
                "layer 'In1.Cu', which the DSN does not have",
            ),
            (
                "(net VBUS (via other_via 0 0))",
                "via 'other_via', which the DSN's library does not have",
            ),
            (
                "(net VBUS (wire (path F.Cu 2000 0 0 10 10) (type fix)))",
                "typed wire",
            ),
            (
                "(net VBUS (wire (path F.Cu 2000 0 0 10 10) (type route)))",
                "typed wire",
            ),
            (
                "(net VBUS (via konnect_via_0001 0 0 (type fix)))",
                "typed via",
            ),
            (
                "(net VBUS (wire (qarc F.Cu 2000 0 0 10 10 0 10)))",
                "an arc",
            ),
            (
                "(net VBUS (wire (path F.Cu 2000 0 0 10)))",
                "complete point pairs",
            ),
            (
                "(net VBUS (wire (path F.Cu 2000 0 0)))",
                "complete point pairs",
            ),
            (
                "(net VBUS (wire (path F.Cu 2000 0 0 10 10 20)))",
                "complete point pairs",
            ),
            (
                "(net VBUS (wire (path F.Cu wide 0 0 10 10)))",
                "not a number",
            ),
            ("(net VBUS (via konnect_via_0001 0))", "no position"),
        ] {
            let error = seed_dsn(DSN, &ses(body)).unwrap_err();
            assert!(format!("{error:#}").contains(needle), "{body}: {error:#}");
        }
        for (ses, needle) in [
            ("(pcb b)", "not a Specctra session"),
            ("(session b)", "no routes"),
            ("(session b (routes (network_out)))", "no resolution"),
            (
                "(session b (routes (resolution furlong 10) (network_out)))",
                "unit 'furlong' is not known",
            ),
            (
                "(session b (routes (resolution um 0) (network_out)))",
                "not positive",
            ),
            ("(session b (routes", "readable S-expression"),
        ] {
            let error = seed_dsn(DSN, ses).unwrap_err();
            assert!(format!("{error:#}").contains(needle), "{ses}: {error:#}");
        }
        for (dsn, needle) in [
            (DSN.replace("  (unit um)\n", ""), "names no unit"),
            (
                DSN.replace("(unit um)", "(unit furlong)"),
                "unit 'furlong' is not known",
            ),
            (DSN.replace("(pcb ", "(board "), "root is not 'pcb'"),
        ] {
            let error = seed_dsn(&dsn, SES).unwrap_err();
            assert!(format!("{error:#}").contains(needle), "{needle}: {error:#}");
        }
    }

    #[test]
    fn a_name_that_holds_a_quote_is_refused_not_written() {
        // A DSN has no escape inside a quoted token, so such a name cannot
        // be written back.
        let dsn = DSN.replace("\"A (odd) net\"", "\"A\\\"B\"");
        let ses = SES.replace("\"A (odd) net\"", "\"A\\\"B\"");
        let error = seed_dsn(&dsn, &ses).unwrap_err();
        assert!(
            format!("{error:#}").contains("holds a quote character"),
            "{error:#}"
        );
    }

    #[test]
    fn a_wiring_scope_is_found_whatever_follows_its_name() {
        for (text, close) in [
            ("(pcb b (structure)(wiring(wire x)))", 33),
            ("(pcb b (wiring))", 14),
            ("(pcb b (wiring\n(wire x)))", 23),
            ("(pcb b (wiring\t(wire x)))", 23),
        ] {
            let ends = dsn_ends(text).unwrap();
            assert_eq!(ends.wiring_close, Some(close), "{text:?}");
            assert_eq!(&text[close..close + 1], ")");
        }
        // A scope whose name only starts with the word is not the wiring.
        assert_eq!(
            dsn_ends("(pcb b (wiring_rules x))").unwrap().wiring_close,
            None
        );
    }

    #[test]
    fn a_dsn_with_two_roots_is_refused() {
        assert!(seed_dsn(&format!("{DSN}(pcb again)"), SES).is_err());
    }

    #[test]
    fn quoted_names_and_the_quote_token_do_not_confuse_the_scan() {
        // A quoted net name that holds parentheses and the word wiring.
        let dsn = DSN.replace("\"A (odd) net\"", "\"A (wiring) ) net\"");
        let ses = SES.replace("\"A (odd) net\"", "\"A (wiring) ) net\"");
        let seeded = seed_dsn(&dsn, &ses).unwrap();
        assert_eq!(seeded.wires, 2);
        assert!(seeded.dsn.contains("      (net \"A (wiring) ) net\")\n"));
        assert!(seeded.dsn.ends_with("    )\n  )\n)\n"));
        let ends = dsn_ends(&dsn).unwrap();
        assert_eq!(
            &dsn[ends.wiring_close.unwrap() - 8..=ends.wiring_close.unwrap()],
            " (wiring)"
        );
        assert_eq!(ends.root_close, dsn.trim_end().len() - 1);
        // Unbalanced text is refused, not guessed at.
        for (text, needle) in [
            ("(pcb b (wiring)", "do not balance"),
            ("(pcb b (wiring)))", "do not balance"),
            ("(pcb b \"open (wiring))", "unclosed quoted token"),
            ("(pcb b (wiring) (wiring))", "more than one wiring scope"),
            ("(pcb b (wiring)) trailing", "text after its root scope"),
        ] {
            let error = dsn_ends(text).err().unwrap();
            assert!(format!("{error:#}").contains(needle), "{text}: {error:#}");
        }
    }

    #[test]
    fn a_sessions_typed_items_and_nets_are_counted() {
        let ses = "(session b (routes (resolution um 10) (network_out (net VBUS (wire (path F.Cu 2000 0 0 10 10)) (wire (path F.Cu 2000 0 0 10 10) (type fix)) (via v 0 0)) (net EMPTY) (net SIG (via v 1 1 (type route))))))";
        let found = session_wiring(ses).unwrap();
        assert_eq!((found.wires, found.vias, found.typed), (2, 2, 2));
        assert_eq!(
            found.nets.iter().map(String::as_str).collect::<Vec<_>>(),
            ["SIG", "VBUS"],
            "a net with no copper is not counted"
        );
        assert!(session_wiring("(pcb b)").is_err());
    }
}
