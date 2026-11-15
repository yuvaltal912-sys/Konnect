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
//!
//! Keywords are read in any letter case, as the Specctra grammar has them;
//! names are compared as they are written. The DSN's text is scanned byte by
//! byte and never cut inside a character: a board file, a net or a pin may be
//! named in any script. A file this module cannot read is an error, never a
//! panic.

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
    /// Wires and vias that carry a `(type ...)`. A seeded pass's session
    /// must hold none.
    pub typed: usize,
}

/// What a `wire` may carry beside its shape. Anything else in a wire is a
/// shape: one this code reads, or one it says it does not.
pub(crate) const WIRE_DESCRIPTORS: [&str; 9] = [
    "net",
    "turret",
    "type",
    "attr",
    "shield",
    "window",
    "connect",
    "supply",
    "clearance_class",
];

/// The most of a shape's name an error or a result repeats.
pub(crate) const MAX_FORM_CHARS: usize = 40;

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
pub(crate) fn parse_specctra(source: &str, what: &str) -> Result<SexpNode> {
    let readable = source.replace("(string_quote \")", "(string_quote \"\\\"\")");
    konnect_sexp::parse_sexp(&readable)
        .map_err(|_| anyhow::anyhow!("the {what} is not a readable S-expression"))
}

fn atom<'a>(node: &'a SexpNode, index: usize, what: &str) -> Result<&'a str> {
    node.get(index)
        .and_then(SexpNode::as_str)
        .with_context(|| format!("{what} is missing"))
}

/// Whether `name` is the keyword `keyword`, in any letter case.
fn is_keyword(name: Option<&str>, keyword: &str) -> bool {
    name.is_some_and(|name| name.eq_ignore_ascii_case(keyword))
}

/// The child lists of `node` whose head is the keyword `head`, in any
/// letter case.
pub(crate) fn lists<'a>(
    node: &'a SexpNode,
    head: &'static str,
) -> impl Iterator<Item = &'a SexpNode> + 'a {
    node.children()
        .unwrap_or(&[])
        .iter()
        .filter(move |child| is_keyword(child.head(), head))
}

/// The first of them.
pub(crate) fn first<'a>(node: &'a SexpNode, head: &'static str) -> Option<&'a SexpNode> {
    lists(node, head).next()
}

/// A name as a DSN token: bare when it was bare, quoted when it was quoted.
/// A DSN has no escape inside a quoted token and the reader has, so a quoted
/// name that holds a quote, a backslash or a control character would be
/// written as a different name: it is refused.
fn token(node: &SexpNode) -> Result<String> {
    match node {
        SexpNode::Atom(text) => Ok(text.clone()),
        SexpNode::Str(text) if text.contains(['"', '\\']) || text.chars().any(char::is_control) => {
            bail!("a session name holds a quote character, a backslash or a control character, and cannot be written back as it was")
        }
        SexpNode::Str(text) => Ok(format!("\"{text}\"")),
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

/// A session number in DSN units.
fn scaled(node: &SexpNode, scale: f64, what: &str) -> Result<f64> {
    let text = node
        .as_str()
        .with_context(|| format!("the session's {what} is not a number"))?;
    let value: f64 = text
        .parse()
        .map_err(|_| anyhow::anyhow!("the session's {what} is not a number"))?;
    let value = value * scale;
    if !value.is_finite() {
        bail!("the session's {what} is not finite");
    }
    Ok(value)
}

/// The routes of a session and micrometres per session unit.
pub(crate) fn session_routes(session: &SexpNode) -> Result<(&SexpNode, f64)> {
    if !is_keyword(session.head(), "session") {
        bail!("the file is not a Specctra session");
    }
    let routes = first(session, "routes").context("the session has no routes")?;
    let resolution =
        first(routes, "resolution").context("the session's routes have no resolution")?;
    let unit = atom(resolution, 1, "the session's resolution unit")?;
    let per_unit: f64 = atom(resolution, 2, "the session's resolution")?
        .parse()
        .map_err(|_| anyhow::anyhow!("the session's resolution is not a number"))?;
    let um = unit_um(unit).with_context(|| format!("the session's unit '{unit}' is not known"))?;
    if !per_unit.is_finite() || per_unit <= 0.0 {
        bail!("the session's resolution is not positive");
    }
    let um_per_unit = um / per_unit;
    if !um_per_unit.is_finite() {
        bail!("the session's resolution is too small to scale by");
    }
    Ok((routes, um_per_unit))
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
                    if first(item, "type").is_some() {
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
///
/// The scan is over bytes. Every byte it acts on (a parenthesis, a quote) is
/// ASCII, and an ASCII byte is never part of a longer character, so text in
/// any script passes through untouched and the offsets returned are always
/// on a character boundary.
fn dsn_ends(text: &str) -> Result<DsnEnds> {
    const QUOTE_TOKEN: &[u8] = b"(string_quote \")";
    let bytes = text.as_bytes();
    let (mut at, mut depth) = (0usize, 0usize);
    let mut top_start: Option<usize> = None;
    let mut wiring_close = None;
    let mut root_close = None;
    while at < bytes.len() {
        if bytes[at..].starts_with(QUOTE_TOKEN) {
            at += QUOTE_TOKEN.len();
            continue;
        }
        match bytes[at] {
            b'"' => {
                let end = bytes[at + 1..]
                    .iter()
                    .position(|byte| *byte == b'"')
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
                    let head = bytes[start + 1..at]
                        .split(|byte| byte.is_ascii_whitespace() || matches!(byte, b'(' | b')'))
                        .next()
                        .unwrap_or(&[]);
                    if head.eq_ignore_ascii_case(b"wiring") {
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
    if bytes[root_close + 1..]
        .iter()
        .any(|byte| !byte.is_ascii_whitespace())
    {
        bail!("the DSN holds text after its root scope");
    }
    Ok(DsnEnds {
        wiring_close,
        root_close,
    })
}

/// `dsn` with the wires and vias of `ses` added to its wiring as ordinary
/// wiring. Refused: a session net, layer or via padstack the DSN does not
/// have; a session item that carries a type; a wire that is anything but
/// plain paths (an arc, a polyline, an area), or that has no width; a name
/// that cannot be written back as it was; a DSN with no `unit`. A session
/// with no copper gives the DSN back unchanged.
pub(crate) fn seed_dsn(dsn: &str, ses: &str) -> Result<Seeded> {
    let pcb = parse_specctra(dsn, "DSN")?;
    if !is_keyword(pcb.head(), "pcb") {
        bail!("the DSN's root is not 'pcb'");
    }
    let unit = first(&pcb, "unit").context("the DSN names no unit")?;
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
                if first(wire, "type").is_some() {
                    bail!("the session holds a typed wire on net '{name}'; only ordinary wiring is seeded");
                }
                let mut paths = 0usize;
                for shape in wire.children().unwrap_or(&[]) {
                    // An atom (the keyword itself) is not a shape.
                    let Some(head) = shape.head() else { continue };
                    let form = head.to_ascii_lowercase();
                    if WIRE_DESCRIPTORS.contains(&form.as_str()) {
                        continue;
                    }
                    if form == "qarc" {
                        bail!(
                            "the session holds an arc on net '{name}', which seeding does not support"
                        );
                    }
                    if form != "path" {
                        let form: String = form.chars().take(MAX_FORM_CHARS).collect();
                        bail!(
                            "the session holds a '{form}' wire on net '{name}', which seeding does not support"
                        );
                    }
                    let children = shape.children().unwrap_or(&[]);
                    // The head, a layer, a width and whole points: an odd count.
                    if children.len() < 7 || children.len().is_multiple_of(2) {
                        bail!("a session path on net '{name}' does not hold complete point pairs");
                    }
                    let layer = atom(shape, 1, "a session path's layer")?;
                    if !layers.contains(layer) {
                        bail!("the session uses layer '{layer}', which the DSN does not have");
                    }
                    let values: Vec<f64> = children[2..]
                        .iter()
                        .map(|value| scaled(value, scale, "path value"))
                        .collect::<Result<_>>()?;
                    if values[0] <= 0.0 {
                        bail!("a session path on net '{name}' has no width");
                    }
                    let values: Vec<String> = values.into_iter().map(dsn_number).collect();
                    items.push_str(&format!(
                        "    (wire\n      (path {} {})\n      (net {net_token})\n    )\n",
                        token(&children[1])?,
                        values.join(" ")
                    ));
                    paths += 1;
                    seeded.wires += 1;
                    seeded.nets.insert(name.to_string());
                }
                if paths == 0 {
                    bail!("the session holds a wire with no path on net '{name}'");
                }
            }
            for via in lists(net, "via") {
                if first(via, "type").is_some() {
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
                    dsn_number(scaled(&children[2], scale, "via x")?),
                    dsn_number(scaled(&children[3], scale, "via y")?),
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
    // The scan's offsets are of ASCII parentheses, so both cuts are on a
    // character boundary; a cut that were not is an error, not a panic.
    let (close, new_scope) = match ends.wiring_close {
        Some(close) => (close, false),
        None => (ends.root_close, true),
    };
    let (before, after) = dsn
        .split_at_checked(close)
        .context("the DSN's closing parenthesis is not where the scan found it")?;
    // `(wiring)` on one line, or a scope that already ends on its own line:
    // either way the new items start on a fresh line.
    let before = before.trim_end_matches([' ', '\t']);
    let newline = if before.ends_with('\n') { "" } else { "\n" };
    seeded.dsn = if new_scope {
        format!("{before}{newline}  (wiring\n{items}  )\n{after}")
    } else {
        format!("{before}{newline}{items}  {after}")
    };
    // What is written must read back, with exactly the new items added.
    let wiring_items = |text: &str| -> Result<usize> {
        let tree = parse_specctra(text, "seeded DSN")?;
        Ok(lists(&tree, "wiring")
            .map(|wiring| {
                wiring
                    .children()
                    .map_or(0, |children| children.len().saturating_sub(1))
            })
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

    // ─── Review A, M1: text outside ASCII, and odd files ───────────────────

    /// A DSN and its session with text outside ASCII in every token the seed
    /// writer reads or steps over: the design name (U-umlaut and Hebrew), a
    /// quoted host string, a layer, a padstack (micro sign), a bare and a
    /// quoted net name (micro sign, omega, Hebrew), pins (degree sign) and a
    /// class (CJK). Konnect's own export writes such tokens bare.
    const WORLD_DSN: &str = "(pcb \u{dc}bung_\u{5dc}\u{5d5}\u{5d7}.kicad_pcb\n  (parser\n    (string_quote \")\n    (space_in_quoted_tokens on)\n    (host_cad \"KiCad \u{2013} \u{fc}\")\n  )\n  (resolution um 10)\n  (unit um)\n  (structure\n    (layer \u{d6}nska.Cu\n      (type signal)\n    )\n  )\n  (library\n    (padstack via_\u{b5}_600\n      (shape (circle \u{d6}nska.Cu 600))\n    )\n  )\n  (network\n    (net \u{b5}VBUS\n      (pins \u{dc}1-1 J\u{b0}1-1)\n    )\n    (net \"\u{3a9} (odd) \u{5d0}\u{5d5}\u{5ea}\"\n      (pins \u{dc}1-2 J\u{b0}1-2)\n    )\n    (class \u{96fb}\u{6e90} \u{b5}VBUS\n      (rule\n        (width 1000)\n      )\n    )\n  )\n  (wiring)\n)\n";

    const WORLD_SES: &str = "(session \u{dc}bung_\u{5dc}\u{5d5}\u{5d7} (routes (resolution um 10) (network_out (net \u{b5}VBUS (wire (path \u{d6}nska.Cu 10000 0 0 30000 0)) (via via_\u{b5}_600 30000 0)) (net \"\u{3a9} (odd) \u{5d0}\u{5d5}\u{5ea}\" (wire (path \u{d6}nska.Cu 2000 1 2 3 4))))))";

    #[test]
    fn text_outside_ascii_in_every_token_is_seeded_byte_for_byte() {
        let seeded = seed_dsn(WORLD_DSN, WORLD_SES).unwrap();
        assert_eq!((seeded.wires, seeded.vias), (2, 1));
        let expected = "  (wiring\n    (wire\n      (path \u{d6}nska.Cu 1000 0 0 3000 0)\n      (net \u{b5}VBUS)\n    )\n    (via via_\u{b5}_600 3000 0\n      (net \u{b5}VBUS)\n    )\n    (wire\n      (path \u{d6}nska.Cu 200 0.1 0.2 0.3 0.4)\n      (net \"\u{3a9} (odd) \u{5d0}\u{5d5}\u{5ea}\")\n    )\n  )\n)\n";
        assert!(seeded.dsn.ends_with(expected), "{}", seeded.dsn);
        // Every byte before the wiring scope is the export's own.
        let head = WORLD_DSN.strip_suffix("  (wiring)\n)\n").unwrap();
        assert_eq!(seeded.dsn.strip_suffix(expected).unwrap(), head);
        let ends = dsn_ends(WORLD_DSN).unwrap();
        let close = ends.wiring_close.unwrap();
        assert!(WORLD_DSN[..close].ends_with("(wiring"));
        assert_eq!(ends.root_close, WORLD_DSN.len() - 2);
        let back = session_wiring(WORLD_SES).unwrap();
        assert_eq!((back.wires, back.vias, back.typed), (2, 1, 0));
        assert_eq!(back.nets, seeded.nets);
    }

    #[test]
    fn a_character_outside_ascii_in_any_one_token_does_not_stop_the_seed() {
        const TOKENS: [&str; 10] = [
            "board.kicad_pcb",
            "F.Cu",
            "B.Cu",
            "konnect_via_0001",
            "VBUS",
            "A (odd) net",
            "U1-1",
            "J1-2",
            "power",
            "signal",
        ];
        // Two, three and four bytes long.
        for ch in ['\u{e9}', '\u{5d0}', '\u{20ac}', '\u{96fb}', '\u{1f600}'] {
            let put = |text: &str, token: &str| text.replace(token, &format!("{ch}{token}{ch}"));
            let mut cases: Vec<(String, String, &str)> = TOKENS
                .iter()
                .map(|token| (put(DSN, token), put(SES, token), *token))
                .collect();
            // The session's own design name, and every token at once.
            cases.push((
                DSN.to_string(),
                SES.replace("(session board", &format!("(session {ch}board{ch}")),
                "the session's name",
            ));
            cases.push(TOKENS.iter().fold(
                (DSN.to_string(), SES.to_string(), "every token"),
                |(dsn, ses, what), token| (put(&dsn, token), put(&ses, token), what),
            ));
            for (dsn, ses, what) in cases {
                let seeded = seed_dsn(&dsn, &ses)
                    .unwrap_or_else(|error| panic!("{ch:?} in {what}: {error:#}"));
                assert_eq!((seeded.wires, seeded.vias), (2, 1), "{ch:?} in {what}");
                assert!(
                    seeded
                        .dsn
                        .starts_with(dsn.strip_suffix("  (wiring)\n)\n").unwrap()),
                    "{ch:?} in {what}"
                );
                assert!(seeded.dsn.ends_with("    )\n  )\n)\n"), "{ch:?} in {what}");
                let ends = dsn_ends(&dsn).unwrap();
                assert!(
                    dsn[..ends.wiring_close.unwrap()].ends_with("(wiring"),
                    "{ch:?} in {what}"
                );
                assert_eq!(ends.root_close, dsn.len() - 2, "{ch:?} in {what}");
            }
        }
        // Beside the quote token, a parenthesis and a quote.
        for text in [
            "(pcb \u{e9}(string_quote \")\u{e9}(wiring)\u{e9})",
            "(pcb \u{1f600} (wiring \"\u{1f600})\" \u{1f600}) \u{1f600})",
        ] {
            let ends = dsn_ends(text).unwrap();
            let close = ends.wiring_close.unwrap();
            assert_eq!(text.as_bytes()[close], b')', "{text}");
            assert_eq!(ends.root_close, text.len() - 1, "{text}");
        }
    }

    #[test]
    fn a_cut_or_odd_file_is_an_error_never_a_panic() {
        // The DSN cut at every character: never whole, never a panic.
        for end in (0..WORLD_DSN.len() - 1).filter(|end| WORLD_DSN.is_char_boundary(*end)) {
            let cut = &WORLD_DSN[..end];
            assert!(dsn_ends(cut).is_err(), "{end}");
            assert!(seed_dsn(cut, WORLD_SES).is_err(), "{end}");
        }
        for end in (0..WORLD_SES.len()).filter(|end| WORLD_SES.is_char_boundary(*end)) {
            let cut = &WORLD_SES[..end];
            assert!(seed_dsn(WORLD_DSN, cut).is_err(), "{end}");
            assert!(session_wiring(cut).is_err(), "{end}");
        }
        for odd in [
            "",
            "(",
            ")",
            "\"",
            "\u{e9}",
            "(\u{e9}",
            "\u{e9})",
            "(pcb \u{e9}",
            "(pcb \"\u{e9}",
            "(pcb b (wiring \u{1f600}",
            "(string_quote \")",
            "(pcb b (string_quote \")\u{e9})",
            "((((",
            "))))",
            "(pcb (unit) (wiring))",
            "(pcb b (unit um) (wiring (wire)))",
            "(session)",
            "(session b (routes (resolution) (network_out (net))))",
            "(session b (routes (resolution um 10) (network_out (net) (net (wire) (via)))))",
            "(session b (routes (resolution um 10) (network_out (net VBUS (wire (path)) (via x)))))",
        ] {
            let _ = dsn_ends(odd);
            let _ = seed_dsn(odd, WORLD_SES);
            let _ = seed_dsn(WORLD_DSN, odd);
            let _ = seed_dsn(DSN, odd);
            let _ = session_wiring(odd);
        }
    }

    // ─── Review A, N4: letter case, wire forms, names, widths ──────────────

    #[test]
    fn keywords_are_read_in_any_letter_case() {
        let dsn = "(PCB board\n  (PARSER\n    (string_quote \")\n  )\n  (UNIT UM)\n  (STRUCTURE\n    (LAYER F.Cu)\n  )\n  (LIBRARY\n    (PADSTACK via0)\n  )\n  (NETWORK\n    (NET VBUS)\n  )\n  (WIRING)\n)\n";
        let ses = "(SESSION b (ROUTES (RESOLUTION UM 10) (NETWORK_OUT (NET VBUS (WIRE (PATH F.Cu 10000 0 0 30000 0)) (VIA via0 30000 0)))))";
        let seeded = seed_dsn(dsn, ses).unwrap();
        assert_eq!((seeded.wires, seeded.vias), (1, 1));
        // The seed went into the scope that was there, not into a second one.
        assert_eq!(seeded.dsn.to_lowercase().matches("(wiring").count(), 1);
        assert!(
            seeded
                .dsn
                .ends_with("  (WIRING\n    (wire\n      (path F.Cu 1000 0 0 3000 0)\n      (net VBUS)\n    )\n    (via via0 3000 0\n      (net VBUS)\n    )\n  )\n)\n"),
            "{}",
            seeded.dsn
        );
        let found = session_wiring(ses).unwrap();
        assert_eq!((found.wires, found.vias, found.typed), (1, 1, 0));
        // A type is a type in any case: counted, and never seeded.
        let typed = ses.replace("30000 0))", "30000 0) (TYPE fix))");
        assert_eq!(session_wiring(&typed).unwrap().typed, 1);
        let error = seed_dsn(dsn, &typed).unwrap_err();
        assert!(format!("{error:#}").contains("typed wire"), "{error:#}");
        let arc = ses.replace(
            "(PATH F.Cu 10000 0 0 30000 0)",
            "(QARC F.Cu 10000 0 0 10 10 0 10)",
        );
        let error = seed_dsn(dsn, &arc).unwrap_err();
        assert!(format!("{error:#}").contains("an arc"), "{error:#}");
    }

    #[test]
    fn a_wire_that_is_not_plain_paths_is_refused_not_dropped() {
        let ses =
            |body: &str| format!("(session b (routes (resolution um 10) (network_out {body})))");
        for (body, needle) in [
            (
                "(net VBUS (wire (polyline_path F.Cu 2000 0 0 0 10 0 0 10 0 10 0 10 10)))",
                "a 'polyline_path' wire",
            ),
            (
                "(net VBUS (wire (polygon F.Cu 0 0 0 10 10 0 10)))",
                "a 'polygon' wire",
            ),
            (
                "(net VBUS (wire (path F.Cu 2000 0 0 10 10) (rect F.Cu 0 0 10 10)))",
                "a 'rect' wire",
            ),
            ("(net VBUS (wire))", "a wire with no path"),
            ("(net VBUS (wire (net VBUS)))", "a wire with no path"),
            ("(net VBUS (wire (path F.Cu 0 0 0 10 10)))", "has no width"),
            (
                "(net VBUS (wire (path F.Cu -2000 0 0 10 10)))",
                "has no width",
            ),
        ] {
            let error = seed_dsn(DSN, &ses(body)).unwrap_err();
            assert!(format!("{error:#}").contains(needle), "{body}: {error:#}");
        }
        // What a wire may carry beside its path is stepped over.
        let seeded = seed_dsn(
            DSN,
            &ses(
                "(net VBUS (wire (path F.Cu 2000 0 0 10 10) (net VBUS) (clearance_class default)))",
            ),
        )
        .unwrap();
        assert_eq!(seeded.wires, 1);
    }

    #[test]
    fn a_name_that_cannot_be_written_back_as_it_was_is_refused() {
        // A DSN has no escape inside a quoted token; the reader has. A name
        // with a backslash, or one that reads as a control character, would
        // be written as a different name.
        for name in ["\"A\\tB x\"", "\"A\\\\B x\"", "\"A\\nB\""] {
            let dsn = DSN.replace("\"A (odd) net\"", name);
            let ses = SES.replace("\"A (odd) net\"", name);
            let error = seed_dsn(&dsn, &ses).unwrap_err();
            assert!(
                format!("{error:#}").contains("cannot be written back"),
                "{name}: {error:#}"
            );
        }
    }
}
