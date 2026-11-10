//! Scoring a routed result on the board itself, against the board's existing
//! routing: `cargo xtask freerouting-bench --score <spec.json>`.
//!
//! `quality` measures a session file against its DSN, so it cannot see a pour,
//! a refill, KiCad's own DRC or the layout audit. Here every result goes
//! through one pipeline, the same for the existing routing (the baseline row)
//! and for each Freerouting run:
//!
//! 1. copy the project folder (board, `.kicad_pro`, libraries, design intent);
//! 2. put the session's wires and vias into the copy (`score_board.py apply`,
//!    KiCad's Python); the baseline's board is copied in instead;
//! 3. refill and check with `kicad-cli pcb drc --refill-zones --save-board`;
//! 4. read the tracks, vias, pads and zones back (`score_board.py measure`);
//! 5. run `audit_layout_quality` through this checkout's own `konnect` by pipe.
//!
//! Everything after that is arithmetic over three JSON files, unit-tested
//! without KiCad, Java or Konnect: track length by net category, the share of
//! a power net below its intent width, DRC against the baseline's counts, the
//! gates and the bands. The files stay in the run's `score/` folder, so
//! `--rescore` redoes the arithmetic without running anything.
//!
//! Only scratch copies are ever written. The source project and the baseline
//! board are read, never opened by KiCad.

use super::parse::{parse_json_prefix, Json};
use super::quality::DsnBoard;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const KICAD_PYTHON: &str =
    "/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3";
const KICAD_CLI: &str = "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli";

// ─── JSON access ─────────────────────────────────────────────────────────────

fn items(value: Option<&Json>) -> &[Json] {
    match value {
        Some(Json::Array(items)) => items,
        _ => &[],
    }
}

fn fields(value: Option<&Json>) -> &[(String, Json)] {
    match value {
        Some(Json::Object(fields)) => fields,
        _ => &[],
    }
}

fn text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) => Some(text),
        _ => None,
    }
}

fn number(value: Option<&Json>) -> Option<f64> {
    match value {
        Some(Json::Number(n)) => Some(*n),
        _ => None,
    }
}

fn read_json(path: &Path) -> Result<Json, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    parse_json_prefix(&text)
        .map(|(json, _)| json)
        .ok_or_else(|| format!("{} is not JSON", path.display()))
}

/// `text` as a JSON string, quotes included.
fn json_text(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ─── Thresholds ──────────────────────────────────────────────────────────────

/// The limits a result is judged by. The defaults are the owner's plan; the
/// spec's `thresholds` object replaces any of them by name.
#[derive(Debug, Clone, PartialEq)]
pub struct Thresholds {
    /// Gate G3: the largest share of a power net's length that may be below
    /// its intent width.
    pub below_intent_share: f64,
    /// Gate G3: the longest stretch of one power net below its intent width.
    pub below_intent_section_mm: f64,
    /// Signal length over the baseline's that still matches it.
    pub signal_match: f64,
    /// Power length over the baseline's that still matches it.
    pub power_match: f64,
    /// Track on pour nets above the baseline's that still matches it, in mm.
    pub pour_track_match_mm: f64,
    /// Right-angle corners above the baseline's that still match it.
    pub right_angle_match: f64,
    /// How far an audit score may fall below the baseline's and still match.
    pub score_match: f64,
    /// How much a decoupling loop may grow over the baseline's and still
    /// match, as a fraction.
    pub loop_match: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            below_intent_share: 0.05,
            below_intent_section_mm: 2.0,
            signal_match: 1.03,
            power_match: 1.05,
            pour_track_match_mm: 20.0,
            right_angle_match: 5.0,
            score_match: 1.0,
            loop_match: 0.05,
        }
    }
}

type ThresholdField = fn(&mut Thresholds) -> &mut f64;

impl Thresholds {
    const FIELDS: [(&'static str, ThresholdField); 8] = [
        ("below_intent_share", |t| &mut t.below_intent_share),
        ("below_intent_section_mm", |t| {
            &mut t.below_intent_section_mm
        }),
        ("signal_match", |t| &mut t.signal_match),
        ("power_match", |t| &mut t.power_match),
        ("pour_track_match_mm", |t| &mut t.pour_track_match_mm),
        ("right_angle_match", |t| &mut t.right_angle_match),
        ("score_match", |t| &mut t.score_match),
        ("loop_match", |t| &mut t.loop_match),
    ];

    /// The defaults with the named values of `json` (an object) put in.
    /// A name that is not a threshold is refused, so a typo cannot leave a
    /// default silently in force.
    pub fn from_json(json: Option<&Json>) -> Result<Self, String> {
        let mut thresholds = Thresholds::default();
        for (name, value) in fields(json) {
            let Some((_, field)) = Self::FIELDS.iter().find(|(known, _)| known == name) else {
                return Err(format!("unknown threshold {name:?}"));
            };
            match value {
                Json::Number(n) if n.is_finite() && *n >= 0.0 => *field(&mut thresholds) = *n,
                _ => return Err(format!("threshold {name:?} must be a number of at least 0")),
            }
        }
        Ok(thresholds)
    }

    pub fn to_json(&self) -> Json {
        let mut copy = self.clone();
        Json::Object(
            Self::FIELDS
                .iter()
                .map(|(name, field)| (name.to_string(), Json::Number(*field(&mut copy))))
                .collect(),
        )
    }
}

// ─── The spec ────────────────────────────────────────────────────────────────

/// What `--score` needs to know, read from a JSON file kept under `target/`
/// (it names a private board).
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreSpec {
    /// The spec file itself, saved in `results.json` for `--rescore`.
    pub source: PathBuf,
    /// A folder holding the unrouted board, its `.kicad_pro`, its libraries
    /// and `.konnect/design_intent.json`. It is copied whole for every run.
    pub project: PathBuf,
    /// The unrouted board's file name inside `project`.
    pub board: String,
    /// The existing routing: a routed copy of the same board and placement.
    pub baseline: PathBuf,
    /// The DSN whose class widths are the intent widths. Without it each
    /// run's own DSN gives them.
    pub intent_dsn: Option<PathBuf>,
    pub kicad_python: PathBuf,
    pub kicad_cli: PathBuf,
    /// A scratch `KICAD_CONFIG_HOME`, so no run reads or writes the owner's
    /// KiCad settings. `<it>/10.0/` holds `kicad_advanced` and copies of the
    /// global `fp-lib-table` and `sym-lib-table`.
    pub kicad_config_home: PathBuf,
    /// This checkout's `konnect` binary, never the installed one.
    pub konnect: PathBuf,
    /// Where the intent currents came from, quoted with every result.
    pub currents: String,
    pub thresholds: Thresholds,
}

impl ScoreSpec {
    const KEYS: [&'static str; 10] = [
        "project",
        "board",
        "baseline",
        "intent_dsn",
        "kicad_python",
        "kicad_cli",
        "kicad_config_home",
        "konnect",
        "currents",
        "thresholds",
    ];

    /// Parse a spec. Relative paths are taken from the spec file's folder.
    pub fn parse(spec: &str, source: &Path, repo: &Path) -> Result<Self, String> {
        let (json, _) = parse_json_prefix(spec).ok_or("the score spec is not JSON")?;
        let Json::Object(entries) = &json else {
            return Err("the score spec must be a JSON object".to_string());
        };
        if let Some((key, _)) = entries
            .iter()
            .find(|(key, _)| !Self::KEYS.contains(&key.as_str()))
        {
            return Err(format!(
                "unknown key {key:?} in the score spec (known: {})",
                Self::KEYS.join(", ")
            ));
        }
        let base = source.parent().unwrap_or(Path::new("."));
        let path = |key: &str| text(json.at(key)).map(|p| base.join(p));
        let required = |key: &str| path(key).ok_or_else(|| format!("the score spec needs {key:?}"));
        let board = text(json.at("board"))
            .filter(|name| !name.is_empty() && !name.contains('/'))
            .ok_or("the score spec needs \"board\": the unrouted board's file name in the project folder")?
            .to_string();
        let currents = text(json.at("currents"))
            .filter(|label| !label.trim().is_empty())
            .ok_or("the score spec needs \"currents\": where the intent currents came from (the owner, or test values)")?
            .to_string();
        Ok(ScoreSpec {
            source: source.to_path_buf(),
            project: required("project")?,
            board,
            baseline: required("baseline")?,
            intent_dsn: path("intent_dsn"),
            kicad_python: path("kicad_python").unwrap_or_else(|| KICAD_PYTHON.into()),
            kicad_cli: path("kicad_cli").unwrap_or_else(|| KICAD_CLI.into()),
            kicad_config_home: required("kicad_config_home")?,
            konnect: path("konnect").unwrap_or_else(|| repo.join("target/release/konnect")),
            currents,
            thresholds: Thresholds::from_json(json.at("thresholds"))?,
        })
    }

    /// Read a spec file. Its paths become absolute: the scoring tools run
    /// with a run's folder as their own, where a relative path (a relative
    /// `KICAD_CONFIG_HOME` above all) would name something else.
    pub fn read(path: &Path, repo: &Path) -> Result<Self, String> {
        let path = &path
            .canonicalize()
            .map_err(|e| format!("cannot read the score spec {}: {e}", path.display()))?;
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read the score spec {}: {e}", path.display()))?;
        Self::parse(&text, path, repo)
    }
}

// ─── What a board holds ──────────────────────────────────────────────────────

/// One straight track, in nanometres.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub net: String,
    pub layer: String,
    pub width_nm: i64,
    pub a: (i64, i64),
    pub b: (i64, i64),
    pub length_nm: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Via {
    pub net: String,
    pub at: (i64, i64),
}

/// A board as `score_board.py measure` wrote it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Measure {
    /// Net → pads on copper.
    pub pads: BTreeMap<String, usize>,
    pub segments: Vec<Segment>,
    pub vias: Vec<Via>,
    /// Nets that own a copper pour.
    pub pour_nets: BTreeSet<String>,
    /// A digest of every footprint's reference, position, angle and side.
    pub placement: String,
}

impl Measure {
    pub fn from_json(json: &Json) -> Result<Self, String> {
        let whole = |value: Option<&Json>| number(value).map(|n| n as i64);
        let mut measure = Measure {
            placement: text(json.at("placement"))
                .ok_or("the measurement has no placement digest")?
                .to_string(),
            ..Measure::default()
        };
        for (net, count) in fields(json.at("pads")) {
            measure
                .pads
                .insert(net.clone(), number(Some(count)).unwrap_or(0.0) as usize);
        }
        for track in items(json.at("tracks")) {
            let row = items(Some(track));
            let (Some(net), Some(layer)) = (text(row.first()), text(row.get(1))) else {
                return Err(
                    "a track row is not [net, layer, width, x1, y1, x2, y2, length]".into(),
                );
            };
            let n: Option<Vec<i64>> = (2..8).map(|i| whole(row.get(i))).collect();
            let Some(n) = n else {
                return Err(
                    "a track row is not [net, layer, width, x1, y1, x2, y2, length]".into(),
                );
            };
            measure.segments.push(Segment {
                net: net.to_string(),
                layer: layer.to_string(),
                width_nm: n[0],
                a: (n[1], n[2]),
                b: (n[3], n[4]),
                length_nm: n[5],
            });
        }
        for via in items(json.at("vias")) {
            let row = items(Some(via));
            let (Some(net), Some(x), Some(y)) =
                (text(row.first()), whole(row.get(1)), whole(row.get(2)))
            else {
                return Err("a via row is not [net, x, y, diameter, drill]".into());
            };
            measure.vias.push(Via {
                net: net.to_string(),
                at: (x, y),
            });
        }
        for zone in items(json.at("zones")) {
            if let Some(net) = text(zone.at("net")).filter(|net| !net.is_empty()) {
                measure.pour_nets.insert(net.to_string());
            }
        }
        Ok(measure)
    }
}

/// Which nets carry a recorded current, and how wide each must be.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Roles {
    /// Nets with a recorded current: the design-intent record's `currents`
    /// and the nets of its `rails`.
    pub intent_nets: BTreeSet<String>,
    /// The width such a net must have, in mm: its class width in the DSN
    /// that was exported with intent widening.
    pub intent_width_mm: BTreeMap<String, f64>,
    /// The narrowest class width of that DSN. No track may be below it.
    pub base_width_mm: Option<f64>,
}

impl Roles {
    /// The nets a design-intent record gives a current to.
    pub fn intent_nets_of(record: &Json) -> BTreeSet<String> {
        let mut nets = BTreeSet::new();
        for current in items(record.at("currents")) {
            if let Some(net) = text(current.at("net")) {
                nets.insert(net.to_string());
            }
        }
        for rail in items(record.at("rails")) {
            if number(rail.at("max_current_a")).is_some() {
                for net in items(rail.at("nets")) {
                    if let Some(net) = text(Some(net)) {
                        nets.insert(net.to_string());
                    }
                }
            }
        }
        nets
    }

    /// The same nets with the widths `dsn` gives them.
    pub fn with_widths_from(&self, dsn: &DsnBoard) -> Roles {
        let mut roles = Roles {
            intent_nets: self.intent_nets.clone(),
            ..Roles::default()
        };
        roles.base_width_mm = dsn
            .net_width
            .values()
            .map(|(_, width)| *width)
            .chain(dsn.default_width_mm)
            .fold(None, |least: Option<f64>, width| {
                Some(least.map_or(width, |l| l.min(width)))
            });
        for net in &self.intent_nets {
            if let Some((_, width)) = dsn.net_width.get(net) {
                roles.intent_width_mm.insert(net.clone(), *width);
            }
        }
        roles
    }
}

/// The three kinds of net lengths are reported for. A net that owns a pour is
/// a pour net whatever else is recorded for it: its current is in the pour.
/// On the boards this is for, those are the ground nets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Pour = 0,
    Power = 1,
    Signal = 2,
}

pub fn category(net: &str, pour_nets: &BTreeSet<String>, roles: &Roles) -> Category {
    if pour_nets.contains(net) {
        Category::Pour
    } else if roles.intent_nets.contains(net) {
        Category::Power
    } else {
        Category::Signal
    }
}

/// How much of one power net is narrower than its intent width.
#[derive(Debug, Clone, PartialEq)]
pub struct WidthCheck {
    pub net: String,
    pub intent_mm: f64,
    pub length_mm: f64,
    /// Length of the segments below the intent width.
    pub below_mm: f64,
    /// The longest connected stretch of such segments.
    pub longest_below_mm: f64,
}

impl WidthCheck {
    pub fn below_share(&self) -> f64 {
        if self.length_mm > 0.0 {
            self.below_mm / self.length_mm
        } else {
            0.0
        }
    }
}

/// A width this much under the wanted one is rounding, not a narrow track.
const WIDTH_SLACK_NM: i64 = 1_000;

fn below(width_nm: i64, wanted_mm: f64) -> bool {
    width_nm + WIDTH_SLACK_NM < (wanted_mm * 1e6).round() as i64
}

/// For every power net (a recorded current, no pour) with an intent width:
/// how much of it is below that width, and the longest stretch that is.
/// A stretch is segments below the width that join end to end, on one layer or
/// through a via of the net.
pub fn width_checks(measure: &Measure, roles: &Roles) -> Vec<WidthCheck> {
    let mut checks = Vec::new();
    for (net, intent_mm) in &roles.intent_width_mm {
        if category(net, &measure.pour_nets, roles) != Category::Power {
            continue;
        }
        let mine: Vec<&Segment> = measure.segments.iter().filter(|s| &s.net == net).collect();
        let via_points: BTreeSet<(i64, i64)> = measure
            .vias
            .iter()
            .filter(|v| &v.net == net)
            .map(|v| v.at)
            .collect();
        let narrow: Vec<&Segment> = mine
            .iter()
            .copied()
            .filter(|s| below(s.width_nm, *intent_mm))
            .collect();
        // Union-find over the narrow segments, joined at shared ends.
        let mut parent: Vec<usize> = (0..narrow.len()).collect();
        fn find(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        let mut at_point: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, segment) in narrow.iter().enumerate() {
            at_point.entry(segment.a).or_default().push(i);
            at_point.entry(segment.b).or_default().push(i);
        }
        for (point, members) in &at_point {
            // Every pair at the point, not only neighbours in the list: two
            // pieces on one layer may have a piece on another layer between
            // them.
            for (n, &first) in members.iter().enumerate() {
                for &second in &members[n + 1..] {
                    let joined =
                        narrow[first].layer == narrow[second].layer || via_points.contains(point);
                    if joined {
                        let (a, b) = (find(&mut parent, first), find(&mut parent, second));
                        parent[a] = b;
                    }
                }
            }
        }
        let mut stretch: HashMap<usize, i64> = HashMap::new();
        for (i, segment) in narrow.iter().enumerate() {
            *stretch.entry(find(&mut parent, i)).or_default() += segment.length_nm;
        }
        let mm = |nm: i64| nm as f64 / 1e6;
        checks.push(WidthCheck {
            net: net.clone(),
            intent_mm: *intent_mm,
            length_mm: mm(mine.iter().map(|s| s.length_nm).sum()),
            below_mm: mm(narrow.iter().map(|s| s.length_nm).sum()),
            longest_below_mm: mm(stretch.values().copied().max().unwrap_or(0)),
        });
    }
    checks
}

/// KiCad's DRC report, as counts.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Drc {
    /// Violations by type.
    pub by_type: BTreeMap<String, usize>,
    pub unconnected: usize,
}

impl Drc {
    pub fn from_json(json: &Json) -> Result<Self, String> {
        let (Some(Json::Array(violations)), Some(Json::Array(unconnected))) =
            (json.at("violations"), json.at("unconnected_items"))
        else {
            return Err("the DRC report has no violations and unconnected_items lists".to_string());
        };
        let mut by_type = BTreeMap::new();
        for violation in violations {
            let kind = text(violation.at("type")).unwrap_or("unknown");
            *by_type.entry(kind.to_string()).or_insert(0) += 1;
        }
        Ok(Drc {
            by_type,
            unconnected: unconnected.len(),
        })
    }

    pub fn count(&self, kind: &str) -> usize {
        self.by_type.get(kind).copied().unwrap_or(0)
    }

    /// Every violation type this report has more of than `baseline`, as
    /// `type n (baseline m)`.
    pub fn above(&self, baseline: &Drc) -> Vec<String> {
        self.by_type
            .iter()
            .filter(|(kind, count)| **count > baseline.count(kind))
            .map(|(kind, count)| format!("{kind} {count} (baseline {})", baseline.count(kind)))
            .collect()
    }
}

/// The numbers of one `audit_layout_quality` result that runs are compared on.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Audit {
    pub copper_quality: Option<f64>,
    pub score: Option<f64>,
    pub routed_share: Option<f64>,
    /// `None` when a net could not be judged (a pour with no saved fill).
    pub complete: Option<bool>,
    /// Category → score, for the categories that measured something.
    pub categories: BTreeMap<String, f64>,
    /// The audit's flat `metrics`, numbers only (`5.3.acute_corners`, …).
    pub metrics: BTreeMap<String, f64>,
}

impl Audit {
    pub fn from_json(json: &Json) -> Result<Self, String> {
        if text(json.at("audit")) != Some("layout_quality") || json.at("quality").is_none() {
            return Err(format!(
                "not an audit_layout_quality result{}",
                text(json.at("error"))
                    .or(text(json.at("message")))
                    .map_or(String::new(), |why| format!(": {why}"))
            ));
        }
        let mut audit = Audit {
            copper_quality: json.number_at("quality.copper_quality"),
            score: json.number_at("quality.score"),
            routed_share: json.number_at("quality.routed_share"),
            complete: match json.at("completion.complete") {
                Some(Json::Bool(complete)) => Some(*complete),
                _ => None,
            },
            ..Audit::default()
        };
        for entry in items(json.at("quality.categories")) {
            if let (Some(name), Some(score)) =
                (text(entry.at("category")), number(entry.at("score")))
            {
                audit.categories.insert(name.to_string(), score);
            }
        }
        for (key, value) in fields(json.at("metrics")) {
            if let Some(n) = number(Some(value)) {
                audit.metrics.insert(key.clone(), n);
            }
        }
        Ok(audit)
    }

    pub fn metric(&self, key: &str) -> Option<f64> {
        self.metrics.get(key).copied()
    }
}

/// One routed board, measured.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BoardScore {
    pub placement: String,
    pub pour_nets: BTreeSet<String>,
    pub segments: usize,
    /// Track length in mm by [`Category`].
    pub length_mm: [f64; 3],
    /// Vias by [`Category`].
    pub vias: [usize; 3],
    /// Track length per net, in mm, for nets with any track.
    pub net_length_mm: BTreeMap<String, f64>,
    /// Segments narrower than the narrowest class width.
    pub below_base_segments: usize,
    pub widths: Vec<WidthCheck>,
    pub drc: Drc,
    pub audit: Audit,
}

impl BoardScore {
    pub fn from_parts(measure: &Measure, drc: Drc, audit: Audit, roles: &Roles) -> Self {
        let mut score = BoardScore {
            placement: measure.placement.clone(),
            pour_nets: measure.pour_nets.clone(),
            segments: measure.segments.len(),
            widths: width_checks(measure, roles),
            drc,
            audit,
            ..BoardScore::default()
        };
        for segment in &measure.segments {
            let mm = segment.length_nm as f64 / 1e6;
            score.length_mm[category(&segment.net, &measure.pour_nets, roles) as usize] += mm;
            *score.net_length_mm.entry(segment.net.clone()).or_default() += mm;
            if roles
                .base_width_mm
                .is_some_and(|base| below(segment.width_nm, base))
            {
                score.below_base_segments += 1;
            }
        }
        for via in &measure.vias {
            score.vias[category(&via.net, &measure.pour_nets, roles) as usize] += 1;
        }
        score
    }

    /// Read the three files a scored run left in `score_dir`.
    pub fn read(score_dir: &Path, roles: &Roles) -> Result<Self, String> {
        let measure = Measure::from_json(&read_json(&score_dir.join("measure.json"))?)?;
        let drc = Drc::from_json(&read_json(&score_dir.join("drc.json"))?)?;
        let audit = Audit::from_json(&read_json(&score_dir.join("audit.json"))?)?;
        Ok(Self::from_parts(&measure, drc, audit, roles))
    }

    pub fn total_mm(&self) -> f64 {
        self.length_mm.iter().sum()
    }

    pub fn length(&self, category: Category) -> f64 {
        self.length_mm[category as usize]
    }

    pub fn non_pour_vias(&self) -> usize {
        self.vias[Category::Power as usize] + self.vias[Category::Signal as usize]
    }
}

// ─── A result against the baseline ───────────────────────────────────────────

/// One gate: it passes when nothing is listed.
#[derive(Debug, Clone, PartialEq)]
pub struct Gate {
    pub name: &'static str,
    pub failures: Vec<String>,
}

impl Gate {
    pub fn pass(&self) -> bool {
        self.failures.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Beats,
    Matches,
    Below,
    NotMeasured,
}

impl Verdict {
    pub fn word(self) -> &'static str {
        match self {
            Verdict::Beats => "beats",
            Verdict::Matches => "matches",
            Verdict::Below => "below",
            Verdict::NotMeasured => "not measured",
        }
    }
}

/// One quality measure of a result beside the baseline's.
#[derive(Debug, Clone, PartialEq)]
pub struct Band {
    pub measure: String,
    pub value: Option<f64>,
    pub baseline: Option<f64>,
    pub verdict: Verdict,
}

/// A result judged against the baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct Compared {
    /// G1 completion, G2 DRC, G3 width.
    pub gates: [Gate; 3],
    /// Signal track over the baseline's, on the nets routed in both.
    pub signal_ratio: Option<f64>,
    pub signal_nets: usize,
    pub power_ratio: Option<f64>,
    pub power_nets: usize,
    /// All track over the baseline's, on the nets routed in both.
    pub total_ratio: Option<f64>,
    pub bands: Vec<Band>,
}

impl Compared {
    pub fn passes(&self) -> bool {
        self.gates.iter().all(Gate::pass)
    }

    /// The worst band: a result beats the baseline only when every measure
    /// does, and matches it when none is below.
    pub fn verdict(&self) -> Verdict {
        let measured: Vec<Verdict> = self
            .bands
            .iter()
            .map(|band| band.verdict)
            .filter(|verdict| *verdict != Verdict::NotMeasured)
            .collect();
        if measured.is_empty() {
            Verdict::NotMeasured
        } else if measured.contains(&Verdict::Below) {
            Verdict::Below
        } else if measured.contains(&Verdict::Matches) {
            Verdict::Matches
        } else {
            Verdict::Beats
        }
    }
}

/// Track of the nets of one category that have track on both boards: the
/// result's over the baseline's, and how many nets that is. Categories are
/// the baseline's, so both boards are split the same way.
fn ratio_on_common(
    candidate: &BoardScore,
    baseline: &BoardScore,
    roles: &Roles,
    wanted: Option<Category>,
) -> (Option<f64>, usize) {
    let (mut mine, mut theirs, mut nets) = (0.0, 0.0, 0);
    for (net, base_mm) in &baseline.net_length_mm {
        let Some(mm) = candidate.net_length_mm.get(net) else {
            continue;
        };
        if wanted.is_some_and(|w| category(net, &baseline.pour_nets, roles) != w) {
            continue;
        }
        if *base_mm > 0.0 && *mm > 0.0 {
            mine += mm;
            theirs += base_mm;
            nets += 1;
        }
    }
    ((theirs > 0.0).then(|| mine / theirs), nets)
}

/// A measure where smaller is better: beats at or under `beat`, matches at or
/// under `matched`.
fn at_most(
    measure: &str,
    value: Option<f64>,
    baseline: Option<f64>,
    beat: f64,
    matched: f64,
) -> Band {
    let verdict = match value {
        None => Verdict::NotMeasured,
        Some(v) if v <= beat + 1e-9 => Verdict::Beats,
        Some(v) if v <= matched + 1e-9 => Verdict::Matches,
        Some(_) => Verdict::Below,
    };
    Band {
        measure: measure.to_string(),
        value,
        baseline,
        verdict,
    }
}

/// A measure where larger is better.
fn at_least(
    measure: &str,
    value: Option<f64>,
    baseline: Option<f64>,
    beat: f64,
    matched: f64,
) -> Band {
    let mut band = at_most(measure, value.map(|v| -v), baseline, -beat, -matched);
    band.value = value;
    band
}

/// Judge `candidate` against `baseline`: the three gates, then each quality
/// measure as beats, matches or below.
pub fn compare(
    candidate: &BoardScore,
    baseline: &BoardScore,
    roles: &Roles,
    t: &Thresholds,
) -> Compared {
    // G1: every connection made, by KiCad's count and by the audit's.
    let mut g1 = Vec::new();
    if candidate.placement != baseline.placement {
        g1.push("the placement is not the baseline's, so the routing cannot be compared".into());
    }
    if candidate.pour_nets != baseline.pour_nets {
        g1.push("the pours are not the baseline's".into());
    }
    if candidate.drc.unconnected > 0 {
        g1.push(format!("{} unconnected (KiCad)", candidate.drc.unconnected));
    }
    match candidate.audit.routed_share {
        Some(share) if share >= 1.0 - 1e-9 => {}
        Some(share) => g1.push(format!("routed share {share:.3} (audit)")),
        None => g1.push("the audit gave no routed share".into()),
    }
    if candidate.audit.complete != Some(true) {
        g1.push("the audit does not call the routing complete".into());
    }

    // G2: no DRC violation type above the baseline's count.
    let g2 = candidate.drc.above(&baseline.drc);

    // G3: widths.
    let mut g3 = Vec::new();
    if candidate.below_base_segments > 0 {
        g3.push(format!(
            "{} segment(s) below the narrowest class width",
            candidate.below_base_segments
        ));
    }
    if candidate.drc.count("track_width") > 0 {
        g3.push(format!(
            "{} track_width violation(s) (KiCad)",
            candidate.drc.count("track_width")
        ));
    }
    for check in &candidate.widths {
        if check.below_share() > t.below_intent_share + 1e-9 {
            g3.push(format!(
                "'{}': {:.0} % of its length below its {:.3} mm intent width (limit {:.0} %)",
                check.net,
                check.below_share() * 100.0,
                check.intent_mm,
                t.below_intent_share * 100.0
            ));
        }
        if check.longest_below_mm > t.below_intent_section_mm + 1e-9 {
            g3.push(format!(
                "'{}': a {:.1} mm stretch below its {:.3} mm intent width (limit {:.1} mm)",
                check.net, check.longest_below_mm, check.intent_mm, t.below_intent_section_mm
            ));
        }
    }
    for (key, what) in [
        (
            "4.2.nets_undersized",
            "nets undersized for their current (audit 4.2)",
        ),
        (
            "4.3.narrow_paths",
            "bottlenecks on a recorded current (audit 4.3)",
        ),
    ] {
        match (candidate.audit.metric(key), baseline.audit.metric(key)) {
            (Some(mine), Some(theirs)) if mine > theirs => {
                g3.push(format!("{mine:.0} {what}, baseline {theirs:.0}"))
            }
            (None, Some(_)) => g3.push(format!("the audit did not measure {key}")),
            _ => {}
        }
    }

    let (signal_ratio, signal_nets) =
        ratio_on_common(candidate, baseline, roles, Some(Category::Signal));
    let (power_ratio, power_nets) =
        ratio_on_common(candidate, baseline, roles, Some(Category::Power));
    let (total_ratio, _) = ratio_on_common(candidate, baseline, roles, None);

    let metric = |score: &BoardScore, key: &str| score.audit.metric(key);
    let mut bands = vec![
        at_most(
            "signal length x",
            signal_ratio,
            Some(1.0),
            1.0,
            t.signal_match,
        ),
        at_most("power length x", power_ratio, Some(1.0), 1.0, t.power_match),
    ];
    let pour = baseline.length(Category::Pour);
    bands.push(at_most(
        "track on pour nets, mm",
        Some(candidate.length(Category::Pour)),
        Some(pour),
        pour,
        pour + t.pour_track_match_mm,
    ));
    let vias = baseline.non_pour_vias() as f64;
    bands.push(at_most(
        "vias off the pour nets",
        Some(candidate.non_pour_vias() as f64),
        Some(vias),
        vias,
        vias,
    ));
    bands.push(at_most(
        "acute corners",
        metric(candidate, "5.3.acute_corners"),
        metric(baseline, "5.3.acute_corners"),
        0.0,
        0.0,
    ));
    let right = metric(baseline, "5.3.right_angle_corners");
    bands.push(at_most(
        "right-angle corners",
        metric(candidate, "5.3.right_angle_corners"),
        right,
        right.unwrap_or(0.0),
        right.unwrap_or(0.0) + t.right_angle_match,
    ));
    let quality = baseline.audit.copper_quality;
    bands.push(match quality {
        Some(base) => at_least(
            "copper quality",
            candidate.audit.copper_quality,
            quality,
            base,
            base - t.score_match,
        ),
        None => at_least("copper quality", None, None, 0.0, 0.0),
    });
    for (name, base) in &baseline.audit.categories {
        bands.push(at_least(
            &format!("{name} score"),
            candidate.audit.categories.get(name).copied(),
            Some(*base),
            *base,
            base - t.score_match,
        ));
    }
    // A loop a raster step longer is the same loop.
    const RASTER_MM: f64 = 0.1;
    for (key, name) in [
        ("4.1.worst_loop_mm", "worst decoupling loop, mm"),
        ("4.1.median_loop_mm", "median decoupling loop, mm"),
    ] {
        if let Some(base) = metric(baseline, key) {
            bands.push(at_most(
                name,
                metric(candidate, key),
                Some(base),
                base + RASTER_MM,
                base * (1.0 + t.loop_match),
            ));
        }
    }
    if let Some(base) = metric(baseline, "3.1.reference_coverage") {
        bands.push(at_least(
            "reference-plane coverage",
            metric(candidate, "3.1.reference_coverage"),
            Some(base),
            base,
            base,
        ));
    }
    if let Some(base) = metric(baseline, "3.2.unstitched_layer_changes") {
        bands.push(at_most(
            "unstitched layer changes",
            metric(candidate, "3.2.unstitched_layer_changes"),
            Some(base),
            base,
            base,
        ));
    }
    bands.push(at_most(
        "floating ground islands",
        metric(candidate, "3.3.floating_ground_islands"),
        metric(baseline, "3.3.floating_ground_islands"),
        0.0,
        0.0,
    ));

    Compared {
        gates: [
            Gate {
                name: "G1 completion",
                failures: g1,
            },
            Gate {
                name: "G2 DRC",
                failures: g2,
            },
            Gate {
                name: "G3 width",
                failures: g3,
            },
        ],
        signal_ratio,
        signal_nets,
        power_ratio,
        power_nets,
        total_ratio,
        bands,
    }
}

// ─── The pipeline ────────────────────────────────────────────────────────────

/// Everything a scored invocation shares: the spec, the roles and the
/// baseline's score.
#[derive(Debug, Clone)]
pub struct ScoreContext {
    pub spec: ScoreSpec,
    /// The record's intent nets, without widths: each board adds its own.
    intent_nets: BTreeSet<String>,
    /// Roles with the widths of the spec's intent DSN, when it names one.
    fixed_roles: Option<Roles>,
    pub baseline: Result<BoardScore, String>,
    /// The `konnect` that ran the audits, as it reported itself.
    pub konnect_commit: Option<String>,
}

impl ScoreContext {
    /// The roles a run is judged with: the intent DSN's widths, else the
    /// run's own DSN's.
    pub fn roles(&self, own_dsn: Option<&DsnBoard>) -> Roles {
        match (&self.fixed_roles, own_dsn) {
            (Some(roles), _) => roles.clone(),
            (None, Some(dsn)) => Roles {
                intent_nets: self.intent_nets.clone(),
                ..Roles::default()
            }
            .with_widths_from(dsn),
            (None, None) => Roles {
                intent_nets: self.intent_nets.clone(),
                ..Roles::default()
            },
        }
    }

    /// The baseline's roles: the intent DSN's when there is one.
    pub fn baseline_roles(&self) -> Roles {
        self.roles(None)
    }

    /// Read the spec and score the baseline into `out/baseline/score`. With
    /// `reuse`, a baseline already scored there is read back instead.
    pub fn prepare(spec_path: &Path, repo: &Path, out: &Path, reuse: bool) -> Result<Self, String> {
        let spec = ScoreSpec::read(spec_path, repo)?;
        let record = spec.project.join(".konnect/design_intent.json");
        let intent_nets = if record.is_file() {
            Roles::intent_nets_of(&read_json(&record)?)
        } else {
            BTreeSet::new()
        };
        let fixed_roles = match &spec.intent_dsn {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("cannot read the intent DSN {}: {e}", path.display()))?;
                let dsn = DsnBoard::parse(&text)
                    .map_err(|e| format!("the intent DSN {}: {e}", path.display()))?;
                Some(
                    Roles {
                        intent_nets: intent_nets.clone(),
                        ..Roles::default()
                    }
                    .with_widths_from(&dsn),
                )
            }
            None => None,
        };
        let mut context = ScoreContext {
            spec,
            intent_nets,
            fixed_roles,
            baseline: Err("not scored".to_string()),
            konnect_commit: None,
        };
        let score_dir = out.join("baseline").join("score");
        let roles = context.baseline_roles();
        let ready = reuse && score_dir.join("audit.json").is_file();
        context.baseline = if ready {
            BoardScore::read(&score_dir, &roles)
        } else {
            run_pipeline(&context.spec, &score_dir, None)
                .and_then(|()| BoardScore::read(&score_dir, &roles))
        };
        context.konnect_commit = read_json(&score_dir.join("konnect.json"))
            .ok()
            .and_then(|json| text(json.at("commit")).map(str::to_string));
        Ok(context)
    }

    /// The same context for `--rescore`: nothing runs, and the baseline is
    /// read from the files a scored run left in `out`.
    pub fn reload(spec_path: &Path, repo: &Path, out: &Path) -> Result<Self, String> {
        if !out.join("baseline/score/audit.json").is_file() {
            return Err(format!(
                "{} has no scored baseline to re-measure",
                out.display()
            ));
        }
        Self::prepare(spec_path, repo, out, true)
    }

    /// Score one run's session files into `score_dir` and read the result.
    pub fn score_run(
        &self,
        score_dir: &Path,
        sessions: &[PathBuf],
        own_dsn: Option<&DsnBoard>,
    ) -> Result<BoardScore, String> {
        run_pipeline(&self.spec, score_dir, Some(sessions))?;
        BoardScore::read(score_dir, &self.roles(own_dsn))
    }
}

#[cfg(test)]
impl ScoreContext {
    /// A context around a given baseline, for tests that run no tool.
    pub fn for_tests(baseline: BoardScore, intent_nets: &[&str]) -> Self {
        let spec = ScoreSpec::parse(
            r#"{"project":"p","board":"b.kicad_pcb","baseline":"r","kicad_config_home":"c","currents":"test values"}"#,
            Path::new("/work/spec.json"),
            Path::new("/repo"),
        )
        .expect("test spec");
        ScoreContext {
            spec,
            intent_nets: intent_nets.iter().map(|net| net.to_string()).collect(),
            fixed_roles: None,
            baseline: Ok(baseline),
            konnect_commit: Some("0123456789abcdef".to_string()),
        }
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Run one tool with its output in `<log>.out` and `<log>.err`, and stop it
/// at `limit`. `stdin`, when given, is the file it reads.
fn run_logged(
    command: &mut Command,
    what: &str,
    log: &Path,
    stdin: Option<&Path>,
    limit: Duration,
) -> Result<(), String> {
    let file = |extension: &str| {
        std::fs::File::create(log.with_extension(extension))
            .map_err(|e| format!("{what}: cannot write its log: {e}"))
    };
    let input = match stdin {
        Some(path) => Stdio::from(
            std::fs::File::open(path).map_err(|e| format!("{what}: cannot read its input: {e}"))?,
        ),
        None => Stdio::null(),
    };
    let mut child = command
        .stdin(input)
        .stdout(file("out")?)
        .stderr(file("err")?)
        .spawn()
        .map_err(|e| format!("{what}: cannot start {:?}: {e}", command.get_program()))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{what}: stopped after {} s", limit.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("{what}: {e}")),
        }
    };
    if status.success() {
        return Ok(());
    }
    let last = std::fs::read_to_string(log.with_extension("err"))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(|line| line.chars().take(240).collect::<String>())
        .unwrap_or_default();
    Err(format!("{what}: exit {:?}: {last}", status.code()))
}

/// The pipeline for one board, leaving `drc.json`, `measure.json` and
/// `audit.json` in `score_dir`. `sessions` are the SES files to put into a
/// copy of the unrouted board; `None` scores the baseline board as it is.
pub fn run_pipeline(
    spec: &ScoreSpec,
    score_dir: &Path,
    sessions: Option<&[PathBuf]>,
) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(score_dir);
    copy_dir(&spec.project, score_dir).map_err(|e| {
        format!(
            "cannot copy {} to {}: {e}",
            spec.project.display(),
            score_dir.display()
        )
    })?;
    let board = score_dir.join(&spec.board);
    if !board.is_file() {
        return Err(format!(
            "the project folder has no {} (the spec's \"board\")",
            spec.board
        ));
    }
    let script = crate::repo_root().join("xtask/freerouting-bench/score_board.py");
    let python = |arguments: &[&std::ffi::OsStr]| {
        let mut command = Command::new(&spec.kicad_python);
        command
            .args(["-I", "-B"])
            .arg(&script)
            .args(arguments)
            .env("KICAD_CONFIG_HOME", &spec.kicad_config_home)
            .current_dir(score_dir);
        command
    };
    let minutes = |n: u64| Duration::from_secs(60 * n);
    match sessions {
        Some(sessions) => {
            let mut arguments = vec!["apply".as_ref(), board.as_os_str()];
            arguments.extend(sessions.iter().map(|s| s.as_os_str()));
            run_logged(
                &mut python(&arguments),
                "putting the session into the board copy",
                &score_dir.join("apply"),
                None,
                minutes(5),
            )?;
        }
        None => {
            std::fs::copy(&spec.baseline, &board).map_err(|e| {
                format!(
                    "cannot copy the baseline board {}: {e}",
                    spec.baseline.display()
                )
            })?;
        }
    }
    // Never piped: a killed kicad-cli leaves a lock behind.
    let drc = score_dir.join("drc.json");
    run_logged(
        Command::new(&spec.kicad_cli)
            .args([
                "pcb",
                "drc",
                "--format",
                "json",
                "--refill-zones",
                "--save-board",
                "-o",
            ])
            .arg(&drc)
            .arg(&board)
            .env("KICAD_CONFIG_HOME", &spec.kicad_config_home)
            .current_dir(score_dir),
        "kicad-cli refill and DRC",
        &score_dir.join("drc"),
        None,
        minutes(15),
    )?;
    if !drc.is_file() {
        return Err("kicad-cli wrote no DRC report".to_string());
    }
    let measured = score_dir.join("measure.json");
    run_logged(
        &mut python(&["measure".as_ref(), board.as_os_str(), measured.as_os_str()]),
        "reading the board back",
        &score_dir.join("measure"),
        None,
        minutes(5),
    )?;
    audit_by_pipe(spec, &board, score_dir)
}

/// The installed Konnect lives here. It is never run or touched.
const INSTALLED_KONNECT: &str = ".local/share/konnect";

/// Run `audit_layout_quality` on `board` through `spec.konnect` over stdio,
/// with its state in `target/dogfood-home`, and write the result to
/// `score_dir/audit.json`. The binary must report itself as the one asked for.
fn audit_by_pipe(spec: &ScoreSpec, board: &Path, score_dir: &Path) -> Result<(), String> {
    let konnect = spec
        .konnect
        .canonicalize()
        .map_err(|e| format!("no konnect binary at {} (build it with `cargo build --release --locked -p konnect`): {e}", spec.konnect.display()))?;
    if konnect.to_string_lossy().contains(INSTALLED_KONNECT) {
        return Err(format!(
            "{} is the installed Konnect; score with this checkout's own build",
            konnect.display()
        ));
    }
    let state = crate::repo_root().join("target/dogfood-home");
    std::fs::create_dir_all(&state)
        .map_err(|e| format!("cannot create {}: {e}", state.display()))?;
    let call = |id: u32, tool: &str, arguments: &str| {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}","arguments":{arguments}}}}}"#
        )
    };
    let requests = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"xtask-freerouting-bench","version":"0"}}}"#.to_string(),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_string(),
        call(2, "get_installation_info", "{}"),
        call(3, "load_toolset", r#"{"name":"design_review"}"#),
        call(
            4,
            "audit_layout_quality",
            &format!(r#"{{"board":{},"max_findings":1}}"#, json_text(&board.to_string_lossy())),
        ),
    ];
    let input = score_dir.join("konnect.in");
    std::fs::write(&input, requests.join("\n") + "\n")
        .map_err(|e| format!("cannot write {}: {e}", input.display()))?;
    run_logged(
        Command::new(&konnect)
            .env("HOME", &state)
            .env("KONNECT_STATE_DIR", &state)
            .env("KONNECT_CALL_LOG", "off")
            .env("KICAD_CONFIG_HOME", &spec.kicad_config_home)
            .current_dir(score_dir),
        "the layout audit",
        &score_dir.join("konnect"),
        Some(&input),
        Duration::from_secs(900),
    )?;
    let answers = std::fs::read_to_string(score_dir.join("konnect.out")).unwrap_or_default();
    let answer = |id: f64| -> Result<Json, String> {
        let reply = answers
            .lines()
            .filter_map(|line| parse_json_prefix(line).map(|(json, _)| json))
            .find(|json| json.number_at("id") == Some(id))
            .ok_or_else(|| format!("konnect gave no answer to request {id}"))?;
        let body = text(
            items(reply.at("result.content"))
                .first()
                .and_then(|c| c.at("text")),
        )
        .ok_or_else(|| format!("konnect's answer to request {id} has no text"))?;
        if matches!(reply.at("result.isError"), Some(Json::Bool(true))) {
            return Err(format!(
                "konnect refused request {id}: {}",
                body.chars().take(400).collect::<String>()
            ));
        }
        parse_json_prefix(body)
            .map(|(json, _)| json)
            .ok_or_else(|| format!("konnect's answer to request {id} is not JSON"))
    };
    let info = answer(2.0)?;
    let reported = text(info.at("runtime.executable_path")).unwrap_or_default();
    if Path::new(reported).canonicalize().ok().as_deref() != Some(konnect.as_path()) {
        return Err(format!(
            "the audit ran in {reported:?}, not in {}",
            konnect.display()
        ));
    }
    let commit = text(info.at("build.commit")).unwrap_or("unknown");
    std::fs::write(
        score_dir.join("konnect.json"),
        format!(
            "{{\"executable\":{},\"commit\":{}}}\n",
            json_text(reported),
            json_text(commit)
        ),
    )
    .map_err(|e| e.to_string())?;
    let audit = answer(4.0)?;
    Audit::from_json(&audit)?;
    // The audit as konnect wrote it, for --rescore.
    let reply = answers
        .lines()
        .filter_map(|line| parse_json_prefix(line).map(|(json, _)| json))
        .find(|json| json.number_at("id") == Some(4.0))
        .ok_or("konnect gave no answer to the audit")?;
    let body = text(
        items(reply.at("result.content"))
            .first()
            .and_then(|c| c.at("text")),
    )
    .unwrap_or_default();
    std::fs::write(score_dir.join("audit.json"), body).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(text: &str) -> Json {
        parse_json_prefix(text).expect("test JSON").0
    }

    fn segment(net: &str, layer: &str, width_um: i64, a: (i64, i64), b: (i64, i64)) -> Segment {
        let mm = |v: i64| v * 1_000_000;
        let (a, b) = ((mm(a.0), mm(a.1)), (mm(b.0), mm(b.1)));
        Segment {
            net: net.to_string(),
            layer: layer.to_string(),
            width_nm: width_um * 1000,
            a,
            b,
            length_nm: (((b.0 - a.0) as f64).hypot((b.1 - a.1) as f64)).round() as i64,
        }
    }

    fn roles() -> Roles {
        Roles {
            intent_nets: ["GND", "+5V", "+12V"].map(String::from).into(),
            intent_width_mm: [("GND", 1.5), ("+5V", 0.5), ("+12V", 0.35)]
                .map(|(net, width)| (net.to_string(), width))
                .into(),
            base_width_mm: Some(0.2),
        }
    }

    fn audit(quality: f64, share: f64, metrics: &[(&str, f64)]) -> Audit {
        Audit {
            copper_quality: Some(quality),
            score: Some(quality * share),
            routed_share: Some(share),
            complete: Some(share >= 1.0),
            categories: [("routing".to_string(), quality)].into(),
            metrics: metrics.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    /// A board with a ground pour, one power net and two signals.
    fn board(power_width_um: i64, signal_len: i64, ground_len: i64) -> Measure {
        Measure {
            pads: [("GND", 6), ("+5V", 3), ("SIG_A", 2), ("SIG_B", 2)]
                .map(|(net, pads)| (net.to_string(), pads))
                .into(),
            segments: vec![
                segment("GND", "F.Cu", 200, (0, 0), (ground_len, 0)),
                segment("+5V", "F.Cu", power_width_um, (0, 5), (10, 5)),
                segment("+5V", "F.Cu", 500, (10, 5), (20, 5)),
                segment("SIG_A", "F.Cu", 200, (0, 10), (signal_len, 10)),
                segment("SIG_B", "B.Cu", 200, (0, 20), (10, 20)),
            ],
            vias: vec![
                Via {
                    net: "GND".into(),
                    at: (0, 0),
                },
                Via {
                    net: "GND".into(),
                    at: (1, 0),
                },
                Via {
                    net: "+5V".into(),
                    at: (0, 5),
                },
                Via {
                    net: "SIG_A".into(),
                    at: (0, 10),
                },
                Via {
                    net: "SIG_B".into(),
                    at: (0, 20),
                },
            ],
            pour_nets: ["GND".to_string()].into(),
            placement: "same".to_string(),
        }
    }

    fn clean_drc() -> Drc {
        Drc {
            by_type: [
                ("silk_overlap".to_string(), 25),
                ("lib_footprint_mismatch".to_string(), 96),
            ]
            .into(),
            unconnected: 0,
        }
    }

    const METRICS: [(&str, f64); 9] = [
        ("5.3.acute_corners", 0.0),
        ("5.3.right_angle_corners", 3.0),
        ("4.1.worst_loop_mm", 30.0),
        ("4.1.median_loop_mm", 9.0),
        ("4.2.nets_undersized", 1.0),
        ("4.3.narrow_paths", 2.0),
        ("3.1.reference_coverage", 0.5),
        ("3.2.unstitched_layer_changes", 10.0),
        ("3.3.floating_ground_islands", 0.0),
    ];

    fn scored(measure: &Measure) -> BoardScore {
        BoardScore::from_parts(measure, clean_drc(), audit(60.0, 1.0, &METRICS), &roles())
    }

    #[test]
    fn a_spec_names_its_files_and_refuses_what_it_does_not_know() {
        let repo = Path::new("/repo");
        let spec = ScoreSpec::parse(
            r#"{"project":"project","board":"b.kicad_pcb","baseline":"/abs/routed.kicad_pcb",
                "kicad_config_home":"cfg","currents":"test values","thresholds":{"signal_match":1.05}}"#,
            Path::new("/work/spec.json"),
            repo,
        )
        .unwrap();
        assert_eq!(spec.project, Path::new("/work/project"));
        assert_eq!(spec.baseline, Path::new("/abs/routed.kicad_pcb"));
        assert_eq!(spec.kicad_config_home, Path::new("/work/cfg"));
        assert_eq!(spec.konnect, Path::new("/repo/target/release/konnect"));
        assert_eq!(spec.kicad_cli, Path::new(KICAD_CLI));
        assert_eq!(spec.intent_dsn, None);
        assert_eq!(spec.currents, "test values");
        assert_eq!(spec.thresholds.signal_match, 1.05);
        assert_eq!(
            spec.thresholds.power_match,
            Thresholds::default().power_match
        );
        let parse = |text: &str| ScoreSpec::parse(text, Path::new("/work/spec.json"), repo);
        let full = |extra: &str| {
            format!(
                r#"{{"project":"p","board":"b.kicad_pcb","baseline":"r","kicad_config_home":"c","currents":"test"{extra}}}"#
            )
        };
        assert!(parse(&full("")).is_ok());
        // A typo must not leave a default in force unnoticed.
        assert!(parse(&full(r#","baselin":"x""#))
            .unwrap_err()
            .contains("unknown key"));
        assert!(parse(&full(r#","thresholds":{"signal_mach":1.1}"#))
            .unwrap_err()
            .contains("unknown threshold"));
        assert!(parse(&full(r#","thresholds":{"signal_match":"1.1"}"#)).is_err());
        // Which currents were used is said with every result, so it is required.
        assert!(
            parse(r#"{"project":"p","board":"b","baseline":"r","kicad_config_home":"c"}"#)
                .unwrap_err()
                .contains("currents")
        );
        // No scratch config home, no run: KiCad would use the owner's settings.
        assert!(
            parse(r#"{"project":"p","board":"b","baseline":"r","currents":"test"}"#)
                .unwrap_err()
                .contains("kicad_config_home")
        );
        assert!(parse(r#"{"project":"p","board":"../b","baseline":"r","kicad_config_home":"c","currents":"t"}"#).is_err());
    }

    /// The scoring tools run with a run's folder as their own, so a spec
    /// reached by a relative path must still name absolute files: a relative
    /// `KICAD_CONFIG_HOME` would send KiCad to a fresh default config.
    #[test]
    fn a_spec_file_is_read_with_absolute_paths() {
        let root = std::env::temp_dir().join(format!("konnect-score-spec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("inner")).unwrap();
        std::fs::write(
            root.join("spec.json"),
            r#"{"project":"project","board":"b.kicad_pcb","baseline":"r.kicad_pcb","kicad_config_home":"cfg","currents":"test"}"#,
        )
        .unwrap();
        let roundabout = root.join("inner").join("..").join("spec.json");
        let spec = ScoreSpec::read(&roundabout, Path::new("/repo")).unwrap();
        let real = root.canonicalize().unwrap();
        assert_eq!(spec.source, real.join("spec.json"));
        assert_eq!(spec.kicad_config_home, real.join("cfg"));
        assert_eq!(spec.project, real.join("project"));
        assert!(
            ScoreSpec::read(&root.join("absent.json"), Path::new("/repo"))
                .unwrap_err()
                .contains("cannot read the score spec")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn thresholds_round_trip_through_json() {
        let custom = Thresholds {
            below_intent_share: 0.1,
            right_angle_match: 8.0,
            ..Thresholds::default()
        };
        assert_eq!(
            Thresholds::from_json(Some(&custom.to_json())).unwrap(),
            custom
        );
        assert_eq!(Thresholds::from_json(None).unwrap(), Thresholds::default());
    }

    #[test]
    fn a_measurement_is_read_with_its_pour_nets() {
        let measure = Measure::from_json(&json(
            r#"{"placement":"abc","pads":{"GND":4,"A":2},
                "tracks":[["A","F.Cu",200000,0,0,3000000,4000000,5000000]],
                "vias":[["GND",1000000,2000000,600000,300000]],
                "zones":[{"net":"GND","layers":["B.Cu"]},{"net":"","layers":["F.Cu"]}]}"#,
        ))
        .unwrap();
        assert_eq!(measure.placement, "abc");
        assert_eq!(measure.pads["GND"], 4);
        assert_eq!(measure.segments[0].b, (3_000_000, 4_000_000));
        assert_eq!(measure.segments[0].length_nm, 5_000_000);
        assert_eq!(measure.vias[0].at, (1_000_000, 2_000_000));
        // A zone with no net is not a pour of any net.
        assert_eq!(measure.pour_nets, ["GND".to_string()].into());
        assert!(
            Measure::from_json(&json(r#"{"placement":"x","tracks":[["A","F.Cu",1]]}"#)).is_err()
        );
        assert!(Measure::from_json(&json(r#"{"tracks":[]}"#)).is_err());
    }

    #[test]
    fn intent_nets_come_from_currents_and_from_rails_with_a_current() {
        let nets = Roles::intent_nets_of(&json(
            r#"{"schema_version":1,
                "currents":[{"net":"MOTOR","max_current_a":2}],
                "rails":[{"name":"5V","nets":["+5V","+5V_SW"],"voltage_v":5,"max_current_a":1},
                         {"name":"REF","nets":["VREF"],"voltage_v":2.5}]}"#,
        ));
        let expected: BTreeSet<String> = ["MOTOR", "+5V", "+5V_SW"].map(String::from).into();
        assert_eq!(nets, expected);
    }

    #[test]
    fn intent_widths_are_the_class_widths_of_the_dsn() {
        let dsn = DsnBoard::parse(
            "(pcb x (resolution um 10) (unit um)
              (structure (layer F.Cu (type signal)) (rule (width 200)))
              (network (net GND (pins R1-1 R2-1)) (net +5V (pins R1-2 R2-2)) (net A (pins R3-1 R3-2))
                (class base A (rule (width 200)))
                (class wide GND (rule (width 1500)))
                (class mid +5V (rule (width 500)))))",
        )
        .unwrap();
        let roles = Roles {
            intent_nets: ["GND", "+5V", "ABSENT"].map(String::from).into(),
            ..Roles::default()
        }
        .with_widths_from(&dsn);
        assert_eq!(roles.base_width_mm, Some(0.2));
        assert_eq!(roles.intent_width_mm["GND"], 1.5);
        assert_eq!(roles.intent_width_mm["+5V"], 0.5);
        // A net the DSN does not name gets no width, and no width is invented.
        assert!(!roles.intent_width_mm.contains_key("ABSENT"));
        assert!(!roles.intent_width_mm.contains_key("A"));
    }

    #[test]
    fn a_net_with_a_pour_is_a_pour_net_whatever_its_current() {
        let pours: BTreeSet<String> = ["GND".to_string()].into();
        let roles = roles();
        assert_eq!(category("GND", &pours, &roles), Category::Pour);
        assert_eq!(category("+5V", &pours, &roles), Category::Power);
        assert_eq!(category("SIG_A", &pours, &roles), Category::Signal);
        // Without its pour, the same net is a power net.
        assert_eq!(category("GND", &BTreeSet::new(), &roles), Category::Power);
    }

    #[test]
    fn length_and_vias_are_split_by_category() {
        let score = scored(&board(500, 30, 8));
        assert_eq!(score.length(Category::Pour), 8.0);
        assert_eq!(score.length(Category::Power), 20.0);
        assert_eq!(score.length(Category::Signal), 40.0);
        assert_eq!(score.total_mm(), 68.0);
        assert_eq!(score.vias, [2, 1, 2]);
        assert_eq!(score.non_pour_vias(), 3);
        assert_eq!(score.segments, 5);
        assert_eq!(score.net_length_mm["SIG_A"], 30.0);
        assert_eq!(score.below_base_segments, 0);
    }

    #[test]
    fn the_share_below_intent_width_is_per_power_net() {
        // +5V: 10 mm at 0.2 and 10 mm at 0.5, against an intent of 0.5.
        let measure = board(200, 30, 8);
        let checks = width_checks(&measure, &roles());
        // GND owns a pour, so it is not judged by length; +12V has no track.
        let nets: Vec<&str> = checks.iter().map(|c| c.net.as_str()).collect();
        assert_eq!(nets, ["+12V", "+5V"]);
        let five = &checks[1];
        assert_eq!((five.length_mm, five.below_mm), (20.0, 10.0));
        assert_eq!(five.below_share(), 0.5);
        assert_eq!(five.longest_below_mm, 10.0);
        assert_eq!(checks[0].below_share(), 0.0);
        // At its width, nothing is below.
        assert_eq!(width_checks(&board(500, 30, 8), &roles())[1].below_mm, 0.0);
        // A micrometre of rounding is not a narrow track.
        let mut rounded = board(500, 30, 8);
        rounded.segments[1].width_nm = 499_500;
        assert_eq!(width_checks(&rounded, &roles())[1].below_mm, 0.0);
    }

    #[test]
    fn a_stretch_below_width_joins_end_to_end_and_through_a_via_only() {
        let mut measure = board(500, 30, 8);
        measure.segments = vec![
            // Three narrow pieces in a row on F.Cu: one 3 mm stretch.
            segment("+5V", "F.Cu", 200, (0, 0), (1, 0)),
            segment("+5V", "F.Cu", 200, (1, 0), (2, 0)),
            segment("+5V", "F.Cu", 200, (2, 0), (3, 0)),
            // A narrow piece on B.Cu from the same point, with no via: apart.
            segment("+5V", "B.Cu", 200, (3, 0), (3, 2)),
            // A wide piece between two narrow ones keeps them apart.
            segment("+5V", "F.Cu", 500, (10, 0), (11, 0)),
            segment("+5V", "F.Cu", 200, (11, 0), (12, 0)),
        ];
        measure.vias.clear();
        let check = &width_checks(&measure, &roles())[1];
        assert_eq!(check.below_mm, 6.0);
        assert_eq!(check.longest_below_mm, 3.0);
        // A via at the shared point joins the two layers: 3 + 2.
        measure.vias.push(Via {
            net: "+5V".into(),
            at: (3_000_000, 0),
        });
        assert_eq!(width_checks(&measure, &roles())[1].longest_below_mm, 5.0);
        // Another net's via there joins nothing.
        measure.vias[0].net = "GND".into();
        assert_eq!(width_checks(&measure, &roles())[1].longest_below_mm, 3.0);
        // Two pieces on one layer meet at a point where a piece on another
        // layer also ends (a through-hole pad, no via): the two still join.
        measure.vias.clear();
        measure.segments = vec![
            segment("+5V", "F.Cu", 200, (0, 0), (4, 0)),
            segment("+5V", "B.Cu", 200, (4, 0), (4, 1)),
            segment("+5V", "F.Cu", 200, (4, 0), (9, 0)),
        ];
        assert_eq!(width_checks(&measure, &roles())[1].longest_below_mm, 9.0);
    }

    #[test]
    fn drc_is_counted_by_type_and_compared_with_the_baseline() {
        let report = json(
            r#"{"violations":[{"type":"silk_overlap","severity":"warning"},{"type":"silk_overlap"},
                              {"type":"clearance","severity":"error"}],
                "unconnected_items":[{"type":"unconnected_items"}],"schematic_parity":[]}"#,
        );
        let drc = Drc::from_json(&report).unwrap();
        assert_eq!((drc.count("silk_overlap"), drc.count("clearance")), (2, 1));
        assert_eq!(drc.unconnected, 1);
        let baseline = Drc {
            by_type: [
                ("silk_overlap".to_string(), 2),
                ("solder_mask_bridge".to_string(), 1),
            ]
            .into(),
            unconnected: 6,
        };
        // Only the type above the baseline's count is named.
        assert_eq!(drc.above(&baseline), ["clearance 1 (baseline 0)"]);
        // Fewer than the baseline, or the same, is not a failure.
        assert!(baseline.above(&baseline).is_empty());
        assert!(Drc::default().above(&baseline).is_empty());
        assert!(Drc::from_json(&json(r#"{"violations":[]}"#)).is_err());
    }

    #[test]
    fn the_audit_is_read_by_its_numbers() {
        let audit = Audit::from_json(&json(
            r#"{"audit":"layout_quality","status":"checked",
                "quality":{"score":60.3,"copper_quality":61.34,"routed_share":0.983,
                  "categories":[{"category":"routing","score":69.6},{"category":"switching_loops","score":null}]},
                "completion":{"complete":false,"unrouted_nets":1},
                "metrics":{"5.3.acute_corners":0,"5.1.wire_to_hpwl":1.013,"3.2.note":"text","4.1.worst_loop_mm":null}}"#,
        ))
        .unwrap();
        assert_eq!(audit.copper_quality, Some(61.34));
        assert_eq!(audit.routed_share, Some(0.983));
        assert_eq!(audit.complete, Some(false));
        assert_eq!(audit.categories.len(), 1);
        assert_eq!(audit.metric("5.1.wire_to_hpwl"), Some(1.013));
        assert_eq!(audit.metric("4.1.worst_loop_mm"), None);
        assert_eq!(audit.metric("3.2.note"), None);
        // A refusal is not an audit with empty numbers.
        let refused = Audit::from_json(&json(r#"{"error":"board not found"}"#)).unwrap_err();
        assert!(refused.contains("board not found"), "{refused}");
    }

    #[test]
    fn a_result_like_the_baseline_passes_every_gate_and_beats_or_matches() {
        let baseline = scored(&board(500, 30, 8));
        let same = compare(&baseline, &baseline, &roles(), &Thresholds::default());
        assert!(same.passes(), "{:?}", same.gates);
        assert_eq!(same.signal_ratio, Some(1.0));
        assert_eq!((same.signal_nets, same.power_nets), (2, 1));
        assert_eq!(same.verdict(), Verdict::Beats);
        // 2 % more signal track still matches; 5 % does not.
        let longer = |signal| {
            let mut measure = board(500, 30, 8);
            measure.segments[3] = Segment {
                length_nm: signal,
                ..measure.segments[3].clone()
            };
            compare(
                &scored(&measure),
                &baseline,
                &roles(),
                &Thresholds::default(),
            )
        };
        let near = longer(30_800_000);
        assert!((near.signal_ratio.unwrap() - 1.02).abs() < 1e-9);
        assert_eq!(near.bands[0].verdict, Verdict::Matches);
        assert_eq!(near.verdict(), Verdict::Matches);
        let far = longer(32_000_000);
        assert_eq!(far.bands[0].verdict, Verdict::Below);
        assert_eq!(far.verdict(), Verdict::Below);
        // The gates are about defects, not length: both still pass.
        assert!(near.passes() && far.passes());
    }

    #[test]
    fn ratios_are_over_the_nets_routed_on_both_boards() {
        let baseline = scored(&board(500, 30, 8));
        // The result leaves SIG_B without track: it is out of the ratio, not a
        // zero that would flatter the result.
        let mut measure = board(500, 33, 8);
        measure.segments.remove(4);
        let compared = compare(
            &scored(&measure),
            &baseline,
            &roles(),
            &Thresholds::default(),
        );
        assert_eq!(compared.signal_nets, 1);
        assert!((compared.signal_ratio.unwrap() - 1.1).abs() < 1e-9);
        // 8 + 20 + 33 over 8 + 20 + 30.
        assert!((compared.total_ratio.unwrap() - 61.0 / 58.0).abs() < 1e-9);
    }

    #[test]
    fn the_completion_gate_needs_kicad_and_the_audit_to_agree() {
        let baseline = scored(&board(500, 30, 8));
        let judge = |edit: &dyn Fn(&mut BoardScore)| {
            let mut score = baseline.clone();
            edit(&mut score);
            compare(&score, &baseline, &roles(), &Thresholds::default()).gates[0].clone()
        };
        assert!(judge(&|_| {}).pass());
        let open = judge(&|s| s.drc.unconnected = 2);
        assert_eq!(open.failures, ["2 unconnected (KiCad)"]);
        assert!(!judge(&|s| s.audit.routed_share = Some(0.983)).pass());
        assert!(!judge(&|s| s.audit.routed_share = None).pass());
        assert!(!judge(&|s| s.audit.complete = None).pass());
        // Another placement makes the comparison unfair, whatever the routing.
        assert!(judge(&|s| s.placement = "moved".into()).failures[0].contains("placement"));
        assert!(!judge(&|s| s.pour_nets.clear()).pass());
    }

    #[test]
    fn the_drc_gate_fails_on_any_type_above_the_baseline() {
        let baseline = scored(&board(500, 30, 8));
        let mut worse = baseline.clone();
        worse.drc.by_type.insert("clearance".into(), 2);
        let compared = compare(&worse, &baseline, &roles(), &Thresholds::default());
        assert_eq!(compared.gates[1].failures, ["clearance 2 (baseline 0)"]);
        assert!(compared.gates[0].pass() && compared.gates[2].pass());
        // One more of a type the baseline already has also fails.
        let mut more = baseline.clone();
        more.drc.by_type.insert("silk_overlap".into(), 26);
        assert!(!compare(&more, &baseline, &roles(), &Thresholds::default()).gates[1].pass());
    }

    #[test]
    fn the_width_gate_holds_power_nets_to_their_intent_width() {
        let baseline = scored(&board(500, 30, 8));
        let gate = |measure: &Measure, t: &Thresholds| {
            compare(&scored(measure), &baseline, &roles(), t).gates[2].clone()
        };
        let t = Thresholds::default();
        assert!(gate(&board(500, 30, 8), &t).pass());
        // Half of +5V at 0.2 mm: over the 5 % share and over the 2 mm stretch.
        let narrow = gate(&board(200, 30, 8), &t);
        assert_eq!(narrow.failures.len(), 2, "{:?}", narrow.failures);
        assert!(narrow.failures[0].contains("50 % of its length"));
        assert!(narrow.failures[1].contains("10.0 mm stretch"));
        // The limits are the thresholds, not constants.
        let loose = Thresholds {
            below_intent_share: 0.6,
            below_intent_section_mm: 12.0,
            ..Thresholds::default()
        };
        assert!(gate(&board(200, 30, 8), &loose).pass());
        // A track below the narrowest class width fails whatever its net.
        let mut thin = board(500, 30, 8);
        thin.segments[3].width_nm = 150_000;
        assert!(gate(&thin, &t).failures[0].contains("below the narrowest class width"));
        // The ground net is wide in the DSN but owns a pour: its 0.2 mm
        // track is not judged by length.
        assert!(gate(&board(500, 30, 80), &t).pass());
    }

    #[test]
    fn the_width_gate_also_reads_kicads_width_check_and_the_audits_bottlenecks() {
        let baseline = scored(&board(500, 30, 8));
        let gate = |edit: &dyn Fn(&mut BoardScore)| {
            let mut score = baseline.clone();
            edit(&mut score);
            compare(&score, &baseline, &roles(), &Thresholds::default()).gates[2].clone()
        };
        assert!(gate(&|_| {}).pass());
        assert!(!gate(&|s| {
            s.drc.by_type.insert("track_width".into(), 1);
        })
        .pass());
        assert!(!gate(&|s| {
            s.audit.metrics.insert("4.3.narrow_paths".into(), 3.0);
        })
        .pass());
        assert!(!gate(&|s| {
            s.audit.metrics.remove("4.2.nets_undersized");
        })
        .pass());
        // Fewer bottlenecks than the baseline is fine.
        assert!(gate(&|s| {
            s.audit.metrics.insert("4.3.narrow_paths".into(), 0.0);
        })
        .pass());
    }

    #[test]
    fn each_quality_measure_is_banded_against_the_baseline() {
        let baseline = scored(&board(500, 30, 8));
        let band = |edit: &dyn Fn(&mut BoardScore), measure: &str| {
            let mut score = baseline.clone();
            edit(&mut score);
            compare(&score, &baseline, &roles(), &Thresholds::default())
                .bands
                .into_iter()
                .find(|b| b.measure == measure)
                .unwrap_or_else(|| panic!("band {measure}"))
                .verdict
        };
        let set = |key: &'static str, value: f64| {
            move |s: &mut BoardScore| {
                s.audit.metrics.insert(key.into(), value);
            }
        };
        // An acute corner is never matched: the baseline has none.
        assert_eq!(
            band(&set("5.3.acute_corners", 1.0), "acute corners"),
            Verdict::Below
        );
        // Right angles: up to the baseline beats, five more match.
        assert_eq!(
            band(&set("5.3.right_angle_corners", 3.0), "right-angle corners"),
            Verdict::Beats
        );
        assert_eq!(
            band(&set("5.3.right_angle_corners", 8.0), "right-angle corners"),
            Verdict::Matches
        );
        assert_eq!(
            band(&set("5.3.right_angle_corners", 9.0), "right-angle corners"),
            Verdict::Below
        );
        // Track on the pour nets: 20 mm over the baseline still matches.
        assert_eq!(
            band(&|s| s.length_mm[0] = 28.0, "track on pour nets, mm"),
            Verdict::Matches
        );
        assert_eq!(
            band(&|s| s.length_mm[0] = 28.5, "track on pour nets, mm"),
            Verdict::Below
        );
        // Vias off the pour nets: not one more.
        assert_eq!(
            band(&|s| s.vias[2] += 1, "vias off the pour nets"),
            Verdict::Below
        );
        assert_eq!(
            band(&|s| s.vias[0] += 9, "vias off the pour nets"),
            Verdict::Beats
        );
        // Scores: larger is better, one point down still matches.
        assert_eq!(
            band(&|s| s.audit.copper_quality = Some(61.0), "copper quality"),
            Verdict::Beats
        );
        assert_eq!(
            band(&|s| s.audit.copper_quality = Some(59.0), "copper quality"),
            Verdict::Matches
        );
        assert_eq!(
            band(&|s| s.audit.copper_quality = Some(58.9), "copper quality"),
            Verdict::Below
        );
        assert_eq!(
            band(&|s| s.audit.copper_quality = None, "copper quality"),
            Verdict::NotMeasured
        );
        assert_eq!(
            band(
                &|s| {
                    s.audit.categories.insert("routing".into(), 58.0);
                },
                "routing score"
            ),
            Verdict::Below
        );
        // Loops: a raster step is the same loop, 5 % matches.
        assert_eq!(
            band(&set("4.1.worst_loop_mm", 30.1), "worst decoupling loop, mm"),
            Verdict::Beats
        );
        assert_eq!(
            band(&set("4.1.worst_loop_mm", 31.5), "worst decoupling loop, mm"),
            Verdict::Matches
        );
        assert_eq!(
            band(&set("4.1.worst_loop_mm", 31.6), "worst decoupling loop, mm"),
            Verdict::Below
        );
        // Return path: no worse than the baseline, and no island at all.
        assert_eq!(
            band(
                &set("3.1.reference_coverage", 0.49),
                "reference-plane coverage"
            ),
            Verdict::Below
        );
        assert_eq!(
            band(
                &set("3.2.unstitched_layer_changes", 11.0),
                "unstitched layer changes"
            ),
            Verdict::Below
        );
        assert_eq!(
            band(
                &set("3.3.floating_ground_islands", 1.0),
                "floating ground islands"
            ),
            Verdict::Below
        );
    }

    #[test]
    fn the_installed_konnect_is_never_the_scorer() {
        let home =
            std::env::temp_dir().join(format!("konnect-score-installed-{}", std::process::id()));
        let installed = home.join(INSTALLED_KONNECT).join("bin");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(installed.join("konnect"), "").unwrap();
        let spec = ScoreSpec::parse(
            &format!(
                r#"{{"project":"p","board":"b.kicad_pcb","baseline":"r","kicad_config_home":"c","currents":"test","konnect":{}}}"#,
                json_text(&installed.join("konnect").to_string_lossy())
            ),
            &home.join("spec.json"),
            Path::new("/repo"),
        )
        .unwrap();
        let error = audit_by_pipe(&spec, Path::new("/nowhere/b.kicad_pcb"), &home).unwrap_err();
        assert!(error.contains("installed Konnect"), "{error}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_json_string_is_escaped_for_the_request_line() {
        assert_eq!(json_text("a \"b\"\\c\n"), r#""a \"b\"\\c\n""#);
        let (back, _) = parse_json_prefix(&json_text("/tmp/my board/b.kicad_pcb")).unwrap();
        assert_eq!(back, Json::Text("/tmp/my board/b.kicad_pcb".into()));
    }
}
