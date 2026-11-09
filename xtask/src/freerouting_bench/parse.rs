//! Reading what Freerouting leaves behind: the SES it writes, the statistics
//! block it prints on stdout, and the version line in its log.
//!
//! None of this needs Java, so all of it is unit-tested. The xtask has no JSON
//! or S-expression dependency of its own (adding one would touch `Cargo.lock`),
//! so both readers here are small and only as general as Freerouting's output
//! needs.

use std::collections::BTreeSet;

/// A JSON value, as much of JSON as Freerouting's statistics block uses.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    /// The value at a dotted path such as `connections.incomplete_count`.
    pub fn at(&self, path: &str) -> Option<&Json> {
        path.split('.').try_fold(self, |value, key| match value {
            Json::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        })
    }

    pub fn number_at(&self, path: &str) -> Option<f64> {
        match self.at(path)? {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn text_at(&self, path: &str) -> Option<&str> {
        match self.at(path)? {
            Json::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// Parse one JSON value from the start of `text`; returns it and the bytes
/// consumed.
pub fn parse_json_prefix(text: &str) -> Option<(Json, usize)> {
    let bytes = text.as_bytes();
    let mut at = 0;
    let value = json_value(bytes, &mut at)?;
    Some((value, at))
}

fn skip_ws(b: &[u8], at: &mut usize) {
    while *at < b.len() && b[*at].is_ascii_whitespace() {
        *at += 1;
    }
}

fn json_value(b: &[u8], at: &mut usize) -> Option<Json> {
    skip_ws(b, at);
    match *b.get(*at)? {
        b'{' => {
            *at += 1;
            let mut fields = Vec::new();
            skip_ws(b, at);
            if b.get(*at) == Some(&b'}') {
                *at += 1;
                return Some(Json::Object(fields));
            }
            loop {
                skip_ws(b, at);
                let Json::Text(key) = json_string(b, at)? else {
                    return None;
                };
                skip_ws(b, at);
                if b.get(*at) != Some(&b':') {
                    return None;
                }
                *at += 1;
                fields.push((key, json_value(b, at)?));
                skip_ws(b, at);
                match b.get(*at)? {
                    b',' => *at += 1,
                    b'}' => {
                        *at += 1;
                        return Some(Json::Object(fields));
                    }
                    _ => return None,
                }
            }
        }
        b'[' => {
            *at += 1;
            let mut items = Vec::new();
            skip_ws(b, at);
            if b.get(*at) == Some(&b']') {
                *at += 1;
                return Some(Json::Array(items));
            }
            loop {
                items.push(json_value(b, at)?);
                skip_ws(b, at);
                match b.get(*at)? {
                    b',' => *at += 1,
                    b']' => {
                        *at += 1;
                        return Some(Json::Array(items));
                    }
                    _ => return None,
                }
            }
        }
        b'"' => json_string(b, at),
        b't' if b[*at..].starts_with(b"true") => {
            *at += 4;
            Some(Json::Bool(true))
        }
        b'f' if b[*at..].starts_with(b"false") => {
            *at += 5;
            Some(Json::Bool(false))
        }
        b'n' if b[*at..].starts_with(b"null") => {
            *at += 4;
            Some(Json::Null)
        }
        _ => {
            let start = *at;
            while *at < b.len() && matches!(b[*at], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
            {
                *at += 1;
            }
            std::str::from_utf8(&b[start..*at])
                .ok()?
                .parse()
                .ok()
                .map(Json::Number)
        }
    }
}

fn json_string(b: &[u8], at: &mut usize) -> Option<Json> {
    if b.get(*at) != Some(&b'"') {
        return None;
    }
    *at += 1;
    let mut out = Vec::new();
    while *at < b.len() {
        match b[*at] {
            b'"' => {
                *at += 1;
                return String::from_utf8(out).ok().map(Json::Text);
            }
            b'\\' => {
                let escaped = *b.get(*at + 1)?;
                *at += 2;
                match escaped {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'u' => {
                        let hex = std::str::from_utf8(b.get(*at..*at + 4)?).ok()?;
                        let c = char::from_u32(u32::from_str_radix(hex, 16).ok()?)?;
                        *at += 4;
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                    }
                    other => out.push(other),
                }
            }
            other => {
                out.push(other);
                *at += 1;
            }
        }
    }
    None
}

/// The statistics block Freerouting prints on stdout after a batch run: the
/// first line that is exactly `{`, parsed as one JSON object. Log lines around
/// it are ignored.
pub fn statistics_block(stdout: &str) -> Option<Json> {
    let mut offset = 0;
    for line in stdout.split_inclusive('\n') {
        if line.trim_end() == "{" {
            let (value, _) = parse_json_prefix(&stdout[offset..])?;
            return matches!(value, Json::Object(_)).then_some(value);
        }
        offset += line.len();
    }
    None
}

/// The last unrouted and violation counts Freerouting 2.3 to 2.5 log: `… (3
/// unrouted and 1 violation)` or `incomplete connections: 3, clearance
/// violations: 1`. The last line naming either is the final state.
pub fn final_counts_from_log(log: &str) -> Option<(u64, Option<u64>)> {
    fn number_before(text: &str, word: &str) -> Option<u64> {
        let at = text.rfind(word)?;
        let digits: String = text[..at]
            .trim_end()
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.chars().rev().collect::<String>().parse().ok()
    }
    fn number_after(text: &str, word: &str) -> Option<u64> {
        let at = text.rfind(word)? + word.len();
        let digits: String = text[at..]
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    }
    log.lines().rev().find_map(|line| {
        if let Some(unrouted) = number_after(line, "incomplete connections:") {
            return Some((unrouted, number_after(line, "clearance violations:")));
        }
        let unrouted = number_before(line, " unrouted and ")?;
        // "1 violation)" or "2 violations)"
        Some((unrouted, number_after(line, " unrouted and ")))
    })
}

/// `2.1.0` from a log line such as `INFO   Freerouting v2.1.0 (build-date: …)`.
pub fn version_from_log(log: &str) -> Option<String> {
    log.lines().find_map(|line| {
        let rest = &line[line.find("Freerouting v")? + "Freerouting v".len()..];
        let version: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let version = version.trim_end_matches('.');
        (version.split('.').count() >= 2).then(|| version.to_string())
    })
}

/// `2.3.0` from `freerouting-2.3.0.jar`.
pub fn version_from_file_name(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".jar").unwrap_or(name);
    let digits = stem.trim_start_matches(|c: char| !c.is_ascii_digit());
    let version: String = digits
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let version = version.trim_end_matches('.');
    (version.split('.').count() >= 2).then(|| version.to_string())
}

/// One Specctra S-expression node.
#[derive(Debug, Clone, PartialEq)]
pub enum Sexp {
    Atom(String),
    List(Vec<Sexp>),
}

impl Sexp {
    pub fn head(&self) -> Option<&str> {
        match self {
            Sexp::List(items) => match items.first()? {
                Sexp::Atom(a) => Some(a),
                Sexp::List(_) => None,
            },
            Sexp::Atom(_) => None,
        }
    }

    pub fn items(&self) -> &[Sexp] {
        match self {
            Sexp::List(items) => items,
            Sexp::Atom(_) => &[],
        }
    }

    pub fn atom(&self) -> Option<&str> {
        match self {
            Sexp::Atom(a) => Some(a),
            Sexp::List(_) => None,
        }
    }

    /// Every descendant list (self included) whose head is `name`.
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Sexp>) {
        if self.head() == Some(name) {
            out.push(self);
        }
        for child in self.items() {
            child.find_all(name, out);
        }
    }

    pub fn first(&self, name: &str) -> Option<&Sexp> {
        let mut found = Vec::new();
        self.find_all(name, &mut found);
        found.into_iter().next()
    }
}

/// Parse one Specctra S-expression. Quoted atoms use `"`, which is what
/// Freerouting writes in a session file.
pub fn parse_sexp(text: &str) -> Option<Sexp> {
    let mut stack: Vec<Vec<Sexp>> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '(' => stack.push(Vec::new()),
            ')' => {
                let list = Sexp::List(stack.pop()?);
                match stack.last_mut() {
                    Some(parent) => parent.push(list),
                    None => return Some(list),
                }
            }
            // `(string_quote ")` names the quote character itself.
            '"' if matches!(
                stack.last().map(Vec::as_slice),
                Some([Sexp::Atom(head)]) if head == "string_quote"
            ) =>
            {
                stack.last_mut()?.push(Sexp::Atom("\"".to_string()));
            }
            '"' => {
                let mut atom = String::new();
                for c in chars.by_ref() {
                    if c == '"' {
                        break;
                    }
                    atom.push(c);
                }
                stack.last_mut()?.push(Sexp::Atom(atom));
            }
            c if c.is_whitespace() => {}
            c => {
                let mut atom = c.to_string();
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() || next == '(' || next == ')' {
                        break;
                    }
                    atom.push(next);
                    chars.next();
                }
                stack.last_mut()?.push(Sexp::Atom(atom));
            }
        }
    }
    None
}

/// What a session file says about the routing.
#[derive(Debug, Clone, PartialEq)]
pub struct SesSummary {
    pub nets_with_wiring: usize,
    pub vias: usize,
    pub wire_segments: usize,
    /// Segments that repeat one already counted (same layer, same two ends).
    pub duplicate_segments: usize,
    /// Sum of the unique segments' lengths.
    pub wirelength_mm: f64,
    /// A fingerprint of the routing itself: every unique segment (net, layer,
    /// width, ends in either order) and every via (net, padstack, position),
    /// sorted. Two runs that wrote the same copper in a different order, or
    /// drew a segment the other way round, have the same one.
    pub routing_fingerprint: String,
}

/// Millimetres per Specctra unit.
pub fn unit_mm(unit: &str) -> Option<f64> {
    match unit.to_ascii_lowercase().as_str() {
        "um" => Some(0.001),
        "mm" => Some(1.0),
        "cm" => Some(10.0),
        "mil" => Some(0.0254),
        "inch" => Some(25.4),
        _ => None,
    }
}

/// Count vias and measure wiring in a session file's `routes` section.
pub fn summarize_ses(text: &str) -> Result<SesSummary, String> {
    let root = parse_sexp(text).ok_or("the SES is not one balanced S-expression")?;
    if root.head() != Some("session") {
        return Err("the SES does not start with (session".to_string());
    }
    let routes = root
        .first("routes")
        .ok_or("the SES has no routes section")?;
    let resolution = routes
        .first("resolution")
        .or_else(|| root.first("resolution"))
        .ok_or("the SES has no resolution")?;
    let unit = resolution.items().get(1).and_then(Sexp::atom).unwrap_or("");
    let per_unit: f64 = resolution
        .items()
        .get(2)
        .and_then(Sexp::atom)
        .and_then(|v| v.parse().ok())
        .filter(|v: &f64| *v > 0.0)
        .ok_or("the SES resolution has no positive value")?;
    let scale = unit_mm(unit).ok_or_else(|| format!("unknown SES unit {unit:?}"))? / per_unit;

    let mut summary = SesSummary {
        nets_with_wiring: 0,
        vias: 0,
        wire_segments: 0,
        duplicate_segments: 0,
        wirelength_mm: 0.0,
        routing_fingerprint: fnv1a(b""),
    };
    let Some(network) = routes.first("network_out") else {
        return Ok(summary);
    };
    let mut seen: BTreeSet<(String, [i64; 4])> = BTreeSet::new();
    let mut copper: BTreeSet<String> = BTreeSet::new();
    for net in network.items().iter().filter(|n| n.head() == Some("net")) {
        let mut wired = false;
        let net_name = net.items().get(1).and_then(Sexp::atom).unwrap_or("");
        for item in net.items() {
            match item.head() {
                Some("via") => {
                    summary.vias += 1;
                    wired = true;
                    let fields: Vec<&str> =
                        item.items().iter().skip(1).filter_map(Sexp::atom).collect();
                    copper.insert(format!("via {net_name:?} {}", fields.join(" ")));
                }
                Some("wire") => {
                    wired = true;
                    for path in item.items().iter().filter(|p| p.head() == Some("path")) {
                        let fields = path.items();
                        let layer = fields.get(1).and_then(Sexp::atom).unwrap_or("").to_string();
                        let width = fields.get(2).and_then(Sexp::atom).unwrap_or("");
                        // (path layer width x0 y0 x1 y1 …)
                        let coords: Vec<i64> = fields
                            .iter()
                            .skip(3)
                            .filter_map(Sexp::atom)
                            .filter_map(|v| v.parse::<f64>().ok())
                            .map(|v| v.round() as i64)
                            .collect();
                        for pair in coords.chunks_exact(2).collect::<Vec<_>>().windows(2) {
                            let (a, b) = ((pair[0][0], pair[0][1]), (pair[1][0], pair[1][1]));
                            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                            summary.wire_segments += 1;
                            copper.insert(format!(
                                "wire {net_name:?} {layer} {width} {} {} {} {}",
                                lo.0, lo.1, hi.0, hi.1
                            ));
                            if !seen.insert((layer.clone(), [lo.0, lo.1, hi.0, hi.1])) {
                                summary.duplicate_segments += 1;
                                continue;
                            }
                            let dx = (hi.0 - lo.0) as f64;
                            let dy = (hi.1 - lo.1) as f64;
                            summary.wirelength_mm += dx.hypot(dy) * scale;
                        }
                    }
                }
                _ => {}
            }
        }
        if wired {
            summary.nets_with_wiring += 1;
        }
    }
    summary.routing_fingerprint =
        fnv1a(copper.into_iter().collect::<Vec<_>>().join("\n").as_bytes());
    Ok(summary)
}

/// 64-bit FNV-1a, as 16 hex digits: stable across Rust releases, unlike
/// `DefaultHasher`.
fn fnv1a(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// A stable fingerprint of a session file's content: its tokens joined by one
/// space, hashed with 64-bit FNV-1a. Whitespace and line endings do not change
/// it; any routed coordinate does.
pub fn ses_fingerprint(text: &str) -> String {
    let normalized = text
        .replace('(', " ( ")
        .replace(')', " ) ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    fnv1a(normalized.as_bytes())
}

/// Connections (pin-to-pin) a DSN asks for: for each net in `network`, its pin
/// count minus one. Used only to report the corpus size.
pub fn dsn_connection_count(text: &str) -> Option<usize> {
    let root = parse_sexp(text)?;
    let network = root.first("network")?;
    Some(
        network
            .items()
            .iter()
            .filter(|n| n.head() == Some("net"))
            .map(|net| {
                net.items()
                    .iter()
                    .filter(|p| p.head() == Some("pins"))
                    .map(|p| p.items().len().saturating_sub(1))
                    .sum::<usize>()
                    .saturating_sub(1)
            })
            .sum(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const STDOUT: &str = "2026-11-08 14:21:05.148 INFO   Freerouting v2.1.0 (build-date: 2025-04-12)\n\
2026-11-08 14:21:06.939 INFO   [5A84] Auto-router pass #1 was completed in 0.09 seconds with the score of 992.39.\n\
{\n  \"host\": \"KiCad's Pcbnew,10.0.8\",\n  \"unit\": \"mm\",\n  \"connections\": {\n    \"maximum_count\": 10,\n    \"incomplete_count\": 2\n  },\n  \"traces\": { \"total_length\": 164.56873, \"names\": [\"a\\\"b\", \"\\u00b5\"] },\n  \"vias\": { \"total_count\": 2 },\n  \"clearance_violations\": { \"total_count\": 0 },\n  \"flags\": [true, false, null, -1.5e2]\n}\n2026-11-08 14:21:07 INFO   done\n";

    #[test]
    fn the_statistics_block_is_found_between_log_lines() {
        let stats = statistics_block(STDOUT).expect("block");
        assert_eq!(stats.number_at("connections.maximum_count"), Some(10.0));
        assert_eq!(stats.number_at("connections.incomplete_count"), Some(2.0));
        assert_eq!(stats.number_at("vias.total_count"), Some(2.0));
        assert_eq!(stats.number_at("traces.total_length"), Some(164.56873));
        assert_eq!(stats.text_at("unit"), Some("mm"));
        assert_eq!(stats.text_at("host"), Some("KiCad's Pcbnew,10.0.8"));
        assert_eq!(
            stats.at("traces.names"),
            Some(&Json::Array(vec![
                Json::Text("a\"b".into()),
                Json::Text("µ".into())
            ]))
        );
        assert_eq!(
            stats.at("flags"),
            Some(&Json::Array(vec![
                Json::Bool(true),
                Json::Bool(false),
                Json::Null,
                Json::Number(-150.0)
            ]))
        );
        assert_eq!(stats.at("connections.nope"), None);
        assert_eq!(stats.number_at("unit"), None);
    }

    #[test]
    fn output_without_a_block_or_with_a_broken_one_has_no_statistics() {
        assert_eq!(statistics_block("INFO only log lines\n"), None);
        assert_eq!(statistics_block("{\n  \"a\": 1,\n"), None);
        assert_eq!(
            statistics_block("x {\n"),
            None,
            "the brace must open a line"
        );
    }

    #[test]
    fn freerouting_2_5_logs_its_final_counts_in_prose() {
        let log = "INFO   Auto-routing pass #1 was completed in 0.12 seconds with score 1000.00 (3 unrouted and 2 violations).\n\
INFO   Auto-routing stage completed: started with 3 unrouted nets, completed in 0.19 seconds, final score: 1000.00 (1 unrouted and 0 violations), using 0.00 total CPU seconds\n\
INFO   Optimizer pass #1: optimizer score 625.00 -> 625.00 (UNCHANGED, 0.0000%), router score: 1000.00, incomplete connections: 0, clearance violations: 4.\n\
INFO   Job 'B251CF' finished with state: COMPLETED (elapsed: 0.31 seconds)\n";
        assert_eq!(final_counts_from_log(log), Some((0, Some(4))));
        let routing_only = log.lines().take(2).collect::<Vec<_>>().join("\n");
        assert_eq!(final_counts_from_log(&routing_only), Some((1, Some(0))));
        assert_eq!(
            final_counts_from_log("INFO   started with 3 unrouted nets"),
            None
        );
        // 2.3 and 2.4 write the singular, and no optimizer summary.
        let singular = "INFO   Optimization stage completed: started with score 998.41 (0 unrouted and 1 violation), completed in 0.01 seconds, final score: 998.41 (0 unrouted and 1 violation), using 0.01 total";
        assert_eq!(final_counts_from_log(singular), Some((0, Some(1))));
        assert_eq!(final_counts_from_log(""), None);
    }

    #[test]
    fn versions_come_from_the_log_or_the_file_name() {
        assert_eq!(version_from_log(STDOUT).as_deref(), Some("2.1.0"));
        assert_eq!(
            version_from_log("INFO   Freerouting v2.5.0-SNAPSHOT").as_deref(),
            Some("2.5.0")
        );
        assert_eq!(version_from_log("INFO   Freerouting started"), None);
        assert_eq!(
            version_from_file_name("freerouting-2.3.0.jar").as_deref(),
            Some("2.3.0")
        );
        assert_eq!(
            version_from_file_name("freerouting-2.4.1-executable.jar").as_deref(),
            Some("2.4.1")
        );
        assert_eq!(version_from_file_name("freerouting.jar"), None);
        assert_eq!(version_from_file_name("freerouting-2.jar"), None);
    }

    const SES: &str = r#"(session b
  (base_design b)
  (placement (resolution um 10) (component R (place R1 1000000 -500000 front 0)))
  (routes
    (resolution um 10)
    (parser (host_cad "KiCad's Pcbnew") (host_version 10.0.8))
    (library_out (padstack "Via[0-1]_600:300_um" (shape (circle F.Cu 6000 0 0))))
    (network_out
      (net GND
        (wire (path F.Cu 2000 0 0 30000 40000))
        (wire (path F.Cu 2000 30000 40000 0 0))
        (wire (path B.Cu 2000 0 0 30000 40000))
        (via "Via[0-1]_600:300_um" 30000 40000)
        (via "Via[0-1]_600:300_um" 0 0)
      )
      (net "VCC 3V3"
        (wire (path F.Cu 2000 0 0 10000 0 10000 10000))
      )
      (net EMPTY)
    )
  )
)"#;

    #[test]
    fn a_session_file_gives_vias_and_unique_wirelength() {
        let summary = summarize_ses(SES).unwrap();
        assert_eq!(summary.vias, 2);
        assert_eq!(summary.nets_with_wiring, 2);
        // GND: 5 mm on F.Cu (its reverse is a duplicate) + 5 mm on B.Cu;
        // VCC: 1 mm + 1 mm.
        assert_eq!(summary.wire_segments, 5);
        assert_eq!(summary.duplicate_segments, 1);
        assert!((summary.wirelength_mm - 12.0).abs() < 1e-9, "{summary:?}");
    }

    #[test]
    fn units_and_resolution_scale_the_wirelength() {
        let ses = "(session s (routes (resolution mil 1) (network_out (net A (wire (path Top 10 0 0 1000 0))))))";
        let summary = summarize_ses(ses).unwrap();
        assert!((summary.wirelength_mm - 25.4).abs() < 1e-9);
        let ses = "(session s (routes (resolution mm 1000) (network_out (net A (wire (path Top 10 0 0 0 2500))))))";
        assert!((summarize_ses(ses).unwrap().wirelength_mm - 2.5).abs() < 1e-9);
        let unrouted = "(session s (routes (resolution um 10)))";
        assert_eq!(summarize_ses(unrouted).unwrap().vias, 0);
    }

    #[test]
    fn a_broken_session_file_is_an_error_not_zero() {
        assert!(summarize_ses("(session s (routes").is_err());
        assert!(summarize_ses("(pcb x)").is_err());
        assert!(summarize_ses("(session s (routes (resolution parsec 1)))").is_err());
        assert!(summarize_ses("(session s (routes (resolution um 0)))").is_err());
        assert!(summarize_ses("(session s (placement))").is_err());
    }

    #[test]
    fn the_fingerprint_ignores_layout_and_sees_coordinates() {
        let a = ses_fingerprint(SES);
        let reflowed = SES.replace("\n", " ").replace("  ", "\t");
        assert_eq!(a, ses_fingerprint(&reflowed));
        assert_eq!(a, ses_fingerprint(&SES.replace(")", " )")));
        assert_ne!(
            a,
            ses_fingerprint(&SES.replace("30000 40000 0 0", "30000 40001 0 0"))
        );
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn the_routing_fingerprint_ignores_order_and_direction_only() {
        let a = summarize_ses(SES).unwrap().routing_fingerprint;
        // The two GND wires swapped, and the VCC path drawn the other way.
        let reordered = SES
            .replace(
                "(wire (path F.Cu 2000 0 0 30000 40000))\n        (wire (path F.Cu 2000 30000 40000 0 0))\n        (wire (path B.Cu 2000 0 0 30000 40000))",
                "(wire (path B.Cu 2000 0 0 30000 40000))\n        (wire (path F.Cu 2000 30000 40000 0 0))",
            )
            .replace("0 0 10000 0 10000 10000", "10000 10000 10000 0 0 0");
        assert_ne!(ses_fingerprint(SES), ses_fingerprint(&reordered));
        assert_eq!(a, summarize_ses(&reordered).unwrap().routing_fingerprint);
        for changed in [
            SES.replace("10000 0 10000 10000", "10000 0 10000 10001"),
            SES.replace("(path B.Cu 2000", "(path In1.Cu 2000"),
            SES.replace("(path B.Cu 2000", "(path B.Cu 2500"),
            SES.replace("30000 40000)\n        (via", "30000 40001)\n        (via"),
            SES.replace("(net \"VCC 3V3\"", "(net VCC"),
        ] {
            assert_ne!(
                a,
                summarize_ses(&changed).unwrap().routing_fingerprint,
                "{changed}"
            );
        }
    }

    #[test]
    fn a_dsn_that_names_its_quote_character_parses() {
        let dsn = r#"(pcb "a b" (parser (string_quote ") (host_cad "KiCad's Pcbnew")) (network (net "N 1" (pins R1-1 R2-1))))"#;
        let root = parse_sexp(dsn).expect("balanced");
        assert_eq!(root.items()[1], Sexp::Atom("a b".into()));
        let quote = root.first("string_quote").unwrap();
        assert_eq!(quote.items()[1], Sexp::Atom("\"".into()));
        assert_eq!(
            root.first("host_cad").unwrap().items()[1],
            Sexp::Atom("KiCad's Pcbnew".into())
        );
        assert_eq!(dsn_connection_count(dsn), Some(1));
    }

    #[test]
    fn a_dsn_names_its_connections() {
        let dsn = "(pcb x (network (net GND (pins R1-1 R2-1 C1-2)) (net VCC (pins R1-2 R2-2)) (net NC (pins U1-3))))";
        assert_eq!(dsn_connection_count(dsn), Some(3));
        assert_eq!(dsn_connection_count("(pcb x)"), None);
    }

    #[test]
    fn real_fixture_sessions_parse() {
        let root = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/konnect-core/tests/fixtures/"
        );
        for name in [
            "specctra_rounded_board.freerouting-2.3.0.ses",
            "specctra_two_resistors_locked.freerouting-2.3.0.ses",
        ] {
            let text = std::fs::read_to_string(format!("{root}{name}")).unwrap();
            let summary = summarize_ses(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(summary.wirelength_mm > 0.0, "{name}: {summary:?}");
        }
        for name in [
            "specctra_rounded_board.native-kicad-10.dsn",
            "specctra_two_resistors.native-kicad-10.dsn",
        ] {
            let text = std::fs::read_to_string(format!("{root}{name}")).unwrap();
            assert!(dsn_connection_count(&text).unwrap() > 0, "{name}");
        }
    }
}
