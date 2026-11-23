//! The judge's length check (docs/LENGTH_TUNING_DESIGN.md, L8 to L10): KiCad
//! measures every checked net on a copy of the judged board, and Konnect compares
//! those lengths with the design intent's groups and pair skews.
//!
//! The copy's `.kicad_dru` holds probe rules only, one `length (min 2000mm)` rule
//! per net, so that KiCad reports every checked net with its actual length; every
//! other DRC check the project lists is set to `ignore`, which KiCad skips. The
//! board's own rules are checked by the judge's ordinary DRC (G4(c)), never here: a
//! probe rule would override a board rule for the same net.

use crate::tools::cli::DrcReport;
use crate::tools::route_lengths::Window;
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The probe rules' name prefix; rule `n` (from 1) is `__konnect_len_<n, 4 digits>`.
pub(crate) const PROBE_RULE_PREFIX: &str = "__konnect_len_";
/// The probe rules' minimum length, mm: no real net reaches it, so every probed net
/// is reported, and the actual length is the other number of the message. KiCad
/// holds a constraint as a 32-bit count of nanometres, so the largest minimum it
/// keeps is 2147.48 mm (`KiROUND` clamps above that); this is below it.
pub(crate) const PROBE_MIN_MM: f64 = 2000.0;
/// The probe's folder in a run's judge folder.
pub(crate) const PROBE_FOLDER: &str = "lengths";
/// The rank key's step for the intent miss, mm (L10).
pub(crate) const MISS_STEP_MM: f64 = 0.01;
/// How far past a window's end a length may be and still pass, mm: half of the
/// last digit KiCad prints (4 decimals), so that a net printed on the bound passes.
pub(crate) const PRINTED_HALF_DIGIT_MM: f64 = 0.000_05;

/// KiCad's rule keys of length and skew violations: G4 counts them, G2 does not
/// (L9).
pub(crate) const LENGTH_RULE_KEYS: [&str; 2] = ["length_out_of_range", "skew_out_of_range"];

/// A group the judge checks, by board net names.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheckedGroup {
    pub interface: String,
    pub name: String,
    pub nets: Vec<String>,
    pub window: Window,
}

/// A pair skew the judge checks, by board net names.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheckedPair {
    pub positive: String,
    pub negative: String,
    pub max_skew_mm: f64,
}

/// Everything one call checks (`route_lengths::LengthPlan::checks`).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LengthChecks {
    pub groups: Vec<CheckedGroup>,
    pub pairs: Vec<CheckedPair>,
    /// The board's own `.kicad_dru` has length or skew rules (G4(c)): the judge's DRC
    /// copy of the project is made to report them whatever the project's severities
    /// say ([`forced_severities_text`]). Not a group or a pair: it does not make
    /// [`LengthChecks::is_empty`] false, so it starts no probe.
    pub board_rules: bool,
}

impl LengthChecks {
    /// Every net to probe, each once, sorted.
    pub(crate) fn nets(&self) -> Vec<String> {
        let mut nets: Vec<String> = self
            .groups
            .iter()
            .flat_map(|group| group.nets.iter().cloned())
            .chain(
                self.pairs
                    .iter()
                    .flat_map(|pair| [pair.positive.clone(), pair.negative.clone()]),
            )
            .collect();
        nets.sort();
        nets.dedup();
        nets
    }

    /// True when nothing is checked (then the judge runs no probe).
    pub(crate) fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.pairs.is_empty()
    }
}

/// One net of a checked group, measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct NetLength {
    pub net: String,
    /// KiCad's length, mm; `None` when the probe gave no entry.
    pub kicad_mm: Option<f64>,
    /// The window this net had to meet, mm (`Window::range_mm`).
    pub low_mm: f64,
    pub high_mm: f64,
    /// How far outside the window, mm; 0 inside.
    pub miss_mm: f64,
}

/// One checked group, measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct GroupLengths {
    pub interface: String,
    pub name: String,
    /// The longest measured net, mm (skew mode's reference).
    pub longest_mm: Option<f64>,
    pub nets: Vec<NetLength>,
}

/// One checked pair, measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PairSkew {
    pub positive: String,
    pub negative: String,
    /// |L₊ − L₋|, mm; `None` when either net is unmeasured.
    pub skew_mm: Option<f64>,
    pub max_skew_mm: f64,
    pub miss_mm: f64,
}

/// Row LT, L22: why a checked net's judged copper is not a clean path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PathFault {
    /// Crossings and overlaps of its session copper (touches are not gated).
    pub contacts: usize,
    /// DRC entries of the judged board that name it: `track_dangling` and
    /// `unconnected_items`.
    pub dangling: usize,
}

/// What the length check found on one judged run (`Measures.lengths`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct LengthMeasures {
    pub groups: Vec<GroupLengths>,
    pub pairs: Vec<PairSkew>,
    /// Checked nets the probe gave no entry for.
    pub unmeasured: Vec<String>,
    /// Row LT, L22 (G4(d)): the checked nets whose judged copper is not a clean path,
    /// by net. A net in it has its window's lower bound as its miss in `groups`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub paths: BTreeMap<String, PathFault>,
    /// The sum of every measured net's and pair's miss, mm.
    pub miss_mm: f64,
    /// The probe's own DRC time, seconds.
    pub probe_s: f64,
}

/// The probe's `.kicad_dru`: `(version 1)`, then for net `n` (from 1, in `nets`
/// order) `(rule "__konnect_len_<n>" (severity error) (constraint length (min
/// 2000mm)) (condition "A.NetName == '<net>'"))`. The names are
/// `route_lengths::tune_token`s and no two board nets are equal ignoring case (L4),
/// so none needs escaping and each rule catches one net.
pub(crate) fn probe_rules_text(nets: &[String]) -> String {
    let mut text = String::from("(version 1)\n");
    for (position, net) in nets.iter().enumerate() {
        text.push_str(&format!(
            "(rule \"{PROBE_RULE_PREFIX}{:04}\"\n  (severity error)\n  (constraint length (min {PROBE_MIN_MM}mm))\n  (condition \"A.NetName == '{net}'\"))\n",
            position + 1
        ));
    }
    text
}

/// The probe copy's `.kicad_pro`: the project's own text with every key of
/// `board.design_settings.rule_severities` set to `"ignore"` except
/// `length_out_of_range`, which is `"error"` (added when missing, with the object
/// that holds it). A project that sets that key to `ignore` hides every probe
/// violation whatever a rule's own `severity` says, so it is forced. A check the
/// project does not list runs at KiCad's default severity: slower, never wrong.
/// Everything else, `use_height_for_length_calcs` included, is kept. An error when
/// the text is not a JSON object.
pub(crate) fn probe_project_text(project_text: &str) -> Result<String> {
    let mut project = parse_project(project_text)?;
    let severities = rule_severities(&mut project)?;
    for severity in severities.values_mut() {
        *severity = json!("ignore");
    }
    severities.insert("length_out_of_range".to_string(), json!("error"));
    serde_json::to_string_pretty(&project).context("write the probe's project file")
}

fn parse_project(project_text: &str) -> Result<Value> {
    let project: Value =
        serde_json::from_str(project_text).context("the project file is not JSON")?;
    ensure!(project.is_object(), "the project file is not a JSON object");
    Ok(project)
}

/// `board.design_settings.rule_severities` of a project, with the objects that hold
/// it added when missing. A key that is present but not an object is replaced: the
/// copy's own settings must be exact.
fn rule_severities(project: &mut Value) -> Result<&mut serde_json::Map<String, Value>> {
    let object_at = |value: &mut Value, key: &str| {
        if !value[key].is_object() {
            value[key] = json!({});
        }
    };
    object_at(project, "board");
    object_at(&mut project["board"], "design_settings");
    object_at(&mut project["board"]["design_settings"], "rule_severities");
    project["board"]["design_settings"]["rule_severities"]
        .as_object_mut()
        .context("the project's rule severities are not an object")
}

/// The judge's DRC copy's `.kicad_pro` when the board has its own length or skew
/// rules (G4(c)): the project's own text with [`LENGTH_RULE_KEYS`] set to
/// `"error"`, everything else kept. A project that sets either key to `ignore`
/// would hide the board's own rules from KiCad's report, and G4 would pass on a
/// check that never ran. G2 does not count these keys, so forcing them changes
/// nothing there. Only ever applied to a scratch copy, never to the owner's project.
pub(crate) fn forced_severities_text(project_text: &str) -> Result<String> {
    let mut project = parse_project(project_text)?;
    let severities = rule_severities(&mut project)?;
    for key in LENGTH_RULE_KEYS {
        severities.insert(key.to_string(), json!("error"));
    }
    serde_json::to_string_pretty(&project).context("write the judge copy's project file")
}

/// [`forced_severities_text`] applied to the `.kicad_pro` beside `board` (a judge's
/// scratch copy); left alone when the copy has none (KiCad's default is `error`).
pub(crate) fn force_length_severities(board: &Path) -> Result<()> {
    let project = board.with_extension("kicad_pro");
    if !project.is_file() {
        return Ok(());
    }
    let own = std::fs::read_to_string(&project)
        .with_context(|| format!("read the project file '{}'", project.display()))?;
    let forced = forced_severities_text(&own).with_context(|| {
        format!(
            "the project file '{}' cannot be made to report the board's length rules",
            project.display()
        )
    })?;
    std::fs::write(&project, forced)
        .with_context(|| format!("write the project file '{}'", project.display()))
}

/// The rules whose findings G4(d) reads, with the least severity the judge gives each in
/// a call that checks lengths (KiCad's own defaults): a project that sets
/// `track_dangling` or `unconnected_items` to `ignore` would hide a net's stub or break
/// from `path_faults`, and from `tune_harm` and G1.
pub(crate) const CHECK_SEVERITIES: [(&str, &str); 2] = [
    ("track_dangling", "warning"),
    ("unconnected_items", "error"),
];

/// KiCad's severities, least first; any other word counts as `ignore`.
fn severity_level(severity: &str) -> u8 {
    match severity {
        "warning" => 1,
        "error" => 2,
        _ => 0,
    }
}

/// The judge copy's `.kicad_pro` for a call with checks (S3): the project's own text
/// with each of [`CHECK_SEVERITIES`] at least at its level. A key at that level or higher
/// (or `error` for a `warning` floor) stays as it is; a key that is missing or lower is
/// set to the level. Everything else is kept. Only ever applied to a scratch copy.
pub(crate) fn check_severities_text(project_text: &str) -> Result<String> {
    let mut project = parse_project(project_text)?;
    let severities = rule_severities(&mut project)?;
    for (key, floor) in CHECK_SEVERITIES {
        let now = severities.get(key).and_then(Value::as_str).unwrap_or("");
        if severity_level(now) < severity_level(floor) {
            severities.insert(key.to_string(), json!(floor));
        }
    }
    serde_json::to_string_pretty(&project).context("write the judge copy's project file")
}

/// Row ZR, ZR0b (P11): the rules a removed ground via or wire can break that the
/// prune's DRC clause must see, each at least a warning in the prune's own scratch
/// project. A project that ignores one would let a dangling stub, a floating via, an
/// isolated piece of copper or a pad with too few spokes through.
pub(crate) const PRUNE_SEVERITIES: [&str; 4] = [
    "track_dangling",
    "via_dangling",
    "isolated_copper",
    "starved_thermal",
];

/// The prune's scratch copy's `.kicad_pro` (P11): the project's own text with each of
/// [`PRUNE_SEVERITIES`] at least `warning`. A key at `warning` or `error` keeps its
/// severity; a key that is missing, `ignore` or any other word is set to `warning`.
/// Everything else is kept. Only ever applied to a scratch copy.
pub(crate) fn prune_severities_text(project_text: &str) -> Result<String> {
    let mut project = parse_project(project_text)?;
    let severities = rule_severities(&mut project)?;
    for key in PRUNE_SEVERITIES {
        let now = severities.get(key).and_then(Value::as_str).unwrap_or("");
        if severity_level(now) < severity_level("warning") {
            severities.insert(key.to_string(), json!("warning"));
        }
    }
    serde_json::to_string_pretty(&project).context("write the prune copy's project file")
}

/// [`prune_severities_text`] applied to the `.kicad_pro` beside `board` (the prune's
/// scratch copy); left alone when the copy has none (KiCad's defaults report all four).
pub(crate) fn force_prune_severities(board: &Path) -> Result<()> {
    let project = board.with_extension("kicad_pro");
    if !project.is_file() {
        return Ok(());
    }
    let own = std::fs::read_to_string(&project)
        .with_context(|| format!("read the project file '{}'", project.display()))?;
    let forced = prune_severities_text(&own).with_context(|| {
        format!(
            "the project file '{}' cannot be made to report dangling, isolated and starved copper",
            project.display()
        )
    })?;
    std::fs::write(&project, forced)
        .with_context(|| format!("write the project file '{}'", project.display()))
}

/// [`check_severities_text`] applied to the `.kicad_pro` beside `board` (a judge's
/// scratch copy); left alone when the copy has none.
pub(crate) fn force_check_severities(board: &Path) -> Result<()> {
    let project = board.with_extension("kicad_pro");
    if !project.is_file() {
        return Ok(());
    }
    let own = std::fs::read_to_string(&project)
        .with_context(|| format!("read the project file '{}'", project.display()))?;
    let forced = check_severities_text(&own).with_context(|| {
        format!(
            "the project file '{}' cannot be made to report dangling and unconnected items",
            project.display()
        )
    })?;
    std::fs::write(&project, forced)
        .with_context(|| format!("write the project file '{}'", project.display()))
}

/// `kicad-cli pcb drc --output <out> --format json --units mm <board>`: no refill,
/// no save, no schematic parity.
pub(crate) fn probe_drc_args<'a>(out: &'a str, board: &'a str) -> Vec<&'a str> {
    vec![
        "pcb", "drc", "--output", out, "--format", "json", "--units", "mm", board,
    ]
}

/// The actual length in a probe violation's description, mm: every number followed
/// by `mm` (a decimal comma read as a point), less the ones within 0.001 of
/// [`PROBE_MIN_MM`]; `None` unless exactly one is left (a net at 2000 mm or longer
/// reads `None` and is unmeasured). The words around the numbers are never read, so
/// KiCad's language does not matter. Its tests use a report captured from a real
/// `kicad-cli` run, not a hand-written message.
pub(crate) fn actual_mm(description: &str) -> Option<f64> {
    let chars: Vec<char> = description.chars().collect();
    let mut found: Vec<f64> = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        if !chars[at].is_ascii_digit() {
            at += 1;
            continue;
        }
        let start = at;
        while at < chars.len() && chars[at].is_ascii_digit() {
            at += 1;
        }
        if at + 1 < chars.len() && matches!(chars[at], '.' | ',') && chars[at + 1].is_ascii_digit()
        {
            at += 1;
            while at < chars.len() && chars[at].is_ascii_digit() {
                at += 1;
            }
        }
        // A number inside a word or a name (`len_0001`) is not a length.
        if start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
            continue;
        }
        let mut unit = at;
        while unit < chars.len() && chars[unit].is_whitespace() {
            unit += 1;
        }
        let is_mm = unit + 1 < chars.len()
            && chars[unit] == 'm'
            && chars[unit + 1] == 'm'
            && !chars.get(unit + 2).is_some_and(|ch| ch.is_alphabetic());
        if !is_mm {
            continue;
        }
        let number: String = chars[start..at]
            .iter()
            .map(|ch| if *ch == ',' { '.' } else { *ch })
            .collect();
        if let Ok(value) = number.parse::<f64>() {
            found.push(value);
        }
    }
    found.retain(|mm| (mm - PROBE_MIN_MM).abs() >= 0.001);
    match found[..] {
        // KiCad reports only a length below the minimum, so a number above it is
        // not an actual length.
        [mm] if mm < PROBE_MIN_MM => Some(mm),
        _ => None,
    }
}

/// The probe's lengths by net, and the nets it gave no entry for (in `nets` order).
/// A violation counts when its rule is `length_out_of_range`, its description names
/// a probe rule, and one of its items names that rule's net as `[<net>]`. Two
/// entries for one net, an entry whose items name another net, or a rule number
/// that was never written, is an error (the probe is not trusted).
pub(crate) fn parse_probe(
    report: &DrcReport,
    nets: &[String],
) -> Result<(BTreeMap<String, f64>, Vec<String>)> {
    let mut lengths: BTreeMap<String, f64> = BTreeMap::new();
    let mut seen: Vec<bool> = vec![false; nets.len()];
    for violation in report
        .violations
        .iter()
        .filter(|violation| violation.rule == "length_out_of_range")
    {
        let Some(at) = violation.description.find(PROBE_RULE_PREFIX) else {
            continue;
        };
        let digits: String = violation.description[at + PROBE_RULE_PREFIX.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let Ok(number) = digits.parse::<usize>() else {
            continue;
        };
        let Some(position) = number.checked_sub(1).filter(|p| *p < nets.len()) else {
            bail!(
                "the length probe's report names rule {PROBE_RULE_PREFIX}{digits}, but only {} rule(s) were written",
                nets.len()
            );
        };
        let net = &nets[position];
        let tag = format!("[{net}]");
        let names_net = violation
            .items
            .iter()
            .any(|item| item.description.contains(&tag));
        if !names_net {
            ensure!(
                violation.items.is_empty(),
                "the length probe's rule {PROBE_RULE_PREFIX}{digits} is for net '{net}', but its items name another net ({})",
                violation
                    .items
                    .iter()
                    .map(|item| item.description.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            );
            continue;
        }
        ensure!(
            !seen[position],
            "the length probe reports net '{net}' twice"
        );
        seen[position] = true;
        if let Some(mm) = actual_mm(&violation.description) {
            lengths.insert(net.clone(), mm);
        }
    }
    let unmeasured = nets
        .iter()
        .filter(|net| !lengths.contains_key(*net))
        .cloned()
        .collect();
    Ok((lengths, unmeasured))
}

/// Runs the probe for `nets` on the judged board `judged`: `scratch_board::
/// copy_project` into `folder`, the board's text copied there, the `.kicad_dru`
/// replaced by [`probe_rules_text`], the `.kicad_pro` by [`probe_project_text`]
/// (left as copied when there is none), then kicad-cli with [`probe_drc_args`] and
/// `KICAD_CONFIG_HOME` = `config_home` when given. Returns the lengths, the
/// unmeasured nets and the DRC's seconds. kicad-cli failing, or a report that does
/// not read, is an error: the judge fails.
pub(crate) async fn probe_lengths(
    kicad_cli: &str,
    config_home: Option<&Path>,
    judged: &Path,
    folder: &Path,
    nets: &[String],
) -> Result<(BTreeMap<String, f64>, Vec<String>, f64)> {
    if folder.exists() {
        std::fs::remove_dir_all(folder)
            .with_context(|| format!("clear the old probe folder '{}'", folder.display()))?;
    }
    let board = crate::scratch_board::copy_project(judged, folder)?;
    std::fs::copy(judged, &board).with_context(|| {
        format!(
            "copy the judged board '{}' to '{}'",
            judged.display(),
            board.display()
        )
    })?;
    std::fs::write(board.with_extension("kicad_dru"), probe_rules_text(nets))
        .context("write the probe's rules")?;
    let project = board.with_extension("kicad_pro");
    if project.is_file() {
        let own = std::fs::read_to_string(&project)
            .with_context(|| format!("read the project file '{}'", project.display()))?;
        let probe = probe_project_text(&own).with_context(|| {
            format!("the project file '{}' cannot be probed", project.display())
        })?;
        std::fs::write(&project, probe).context("write the probe's project file")?;
    }

    let out = board.with_extension("length.json");
    let args = probe_drc_args(
        out.to_str().context("scratch path is not UTF-8")?,
        board.to_str().context("scratch path is not UTF-8")?,
    );
    let started = std::time::Instant::now();
    let report = crate::tools::cli::drc_report_from_args(kicad_cli, &args, &out, config_home)
        .await
        .context("kicad-cli's length probe failed")?;
    let seconds = started.elapsed().as_secs_f64();
    let (lengths, unmeasured) = parse_probe(&report, nets)?;
    Ok((lengths, unmeasured, seconds))
}

/// The measures of a run (L9): each group's nets against `Window::range_mm` (skew
/// mode against the longest measured net), each pair's skew against its maximum, a
/// net past a window's end by at most [`PRINTED_HALF_DIGIT_MM`] counted inside; the
/// misses summed. A net of `faults` (L22, [`path_faults`]) has its window's lower
/// bound as its miss, as if no length were usable: a finite number, so that the rank
/// still orders the runs (an unmeasured net would set every such run to the same
/// largest key). A faulted net in no group has no window: it adds the longer measured
/// length of its pair as its miss (also as if no length were usable), and counts as
/// unmeasured when its pair is not measured.
pub(crate) fn check(
    checks: &LengthChecks,
    lengths: &BTreeMap<String, f64>,
    unmeasured: Vec<String>,
    faults: BTreeMap<String, PathFault>,
    probe_s: f64,
) -> LengthMeasures {
    // How far `value` is outside `low..=high`, mm: 0 on a bound, within half a
    // printed digit.
    let outside = |value: f64, low: f64, high: f64| -> f64 {
        if value < low - PRINTED_HALF_DIGIT_MM {
            low - value
        } else if value > high + PRINTED_HALF_DIGIT_MM {
            value - high
        } else {
            0.0
        }
    };
    let mut miss_mm = 0.0;
    let mut groups = Vec::new();
    for group in &checks.groups {
        let longest_mm = group
            .nets
            .iter()
            .filter_map(|net| lengths.get(net).copied())
            .fold(None, |most: Option<f64>, mm| {
                Some(most.map_or(mm, |m| m.max(mm)))
            });
        let (low_mm, high_mm) = group.window.range_mm(longest_mm.unwrap_or(0.0));
        let nets = group
            .nets
            .iter()
            .map(|net| {
                let kicad_mm = lengths.get(net).copied();
                let miss = if faults.contains_key(net) {
                    low_mm.max(0.0)
                } else {
                    kicad_mm.map_or(0.0, |mm| outside(mm, low_mm, high_mm))
                };
                miss_mm += miss;
                NetLength {
                    net: net.clone(),
                    kicad_mm,
                    low_mm,
                    high_mm,
                    miss_mm: miss,
                }
            })
            .collect();
        groups.push(GroupLengths {
            interface: group.interface.clone(),
            name: group.name.clone(),
            longest_mm,
            nets,
        });
    }
    let pairs = checks
        .pairs
        .iter()
        .map(|pair| {
            let skew_mm = match (lengths.get(&pair.positive), lengths.get(&pair.negative)) {
                (Some(positive), Some(negative)) => Some((positive - negative).abs()),
                _ => None,
            };
            let miss = skew_mm.map_or(0.0, |skew| outside(skew, 0.0, pair.max_skew_mm));
            miss_mm += miss;
            PairSkew {
                positive: pair.positive.clone(),
                negative: pair.negative.clone(),
                skew_mm,
                max_skew_mm: pair.max_skew_mm,
                miss_mm: miss,
            }
        })
        .collect();
    // A checked net with no length is unmeasured whatever the caller listed.
    let mut unmeasured = unmeasured;
    // A faulted net in no group has no window (S6): its pair's longer length is its miss.
    let grouped: BTreeSet<&String> = checks.groups.iter().flat_map(|g| &g.nets).collect();
    let mut charged: BTreeSet<&String> = BTreeSet::new();
    for pair in &checks.pairs {
        for net in [&pair.positive, &pair.negative] {
            if !faults.contains_key(net) || grouped.contains(net) || !charged.insert(net) {
                continue;
            }
            match (lengths.get(&pair.positive), lengths.get(&pair.negative)) {
                (Some(positive), Some(negative)) => miss_mm += positive.max(*negative),
                _ => unmeasured.push(net.clone()),
            }
        }
    }
    unmeasured.extend(
        checks
            .nets()
            .into_iter()
            .filter(|net| !lengths.contains_key(net)),
    );
    unmeasured.sort();
    unmeasured.dedup();
    LengthMeasures {
        groups,
        pairs,
        unmeasured,
        paths: faults,
        miss_mm,
        probe_s,
    }
}

/// Row LT, L22: the faults of the checked nets (`checks.nets()`), from the judged
/// session's self contacts (`fastroute::self_contacts`; `Touch` left out: the
/// router's own jogs touch) and the judged board's DRC report: the entries of
/// `track_dangling` and of `unconnected_items` that name the net as `[<net>]` in an
/// item description. Nets without a fault are left out.
pub(crate) fn path_faults(
    checks: &LengthChecks,
    contacts: &BTreeMap<String, Vec<crate::fastroute::SelfContact>>,
    drc: &DrcReport,
) -> BTreeMap<String, PathFault> {
    use crate::fastroute::ContactKind;
    let names = |violation: &crate::tools::cli::DrcViolation, tag: &str| {
        violation
            .items
            .iter()
            .any(|item| item.description.contains(tag))
    };
    let mut faults = BTreeMap::new();
    for net in checks.nets() {
        let tag = format!("[{net}]");
        let crossing = contacts.get(&net).map_or(0, |found| {
            found
                .iter()
                .filter(|contact| {
                    matches!(contact.kind, ContactKind::Crossing | ContactKind::Overlap)
                })
                .count()
        });
        let dangling = drc
            .violations
            .iter()
            .filter(|violation| violation.rule == "track_dangling")
            .chain(drc.unconnected_items.iter().flatten())
            .filter(|violation| names(violation, &tag))
            .count();
        if crossing > 0 || dangling > 0 {
            faults.insert(
                net,
                PathFault {
                    contacts: crossing,
                    dangling,
                },
            );
        }
    }
    faults
}

/// Row LT, L22: G4(d)'s lines, one for each cause of each net of `faults`: "<net>'s
/// copper crosses or overlaps itself <n> time(s)" and "<net> has <n> dangling or
/// unconnected item(s)", each ending "so KiCad's length counts copper the signal
/// does not travel".
pub(crate) fn contact_lines(faults: &BTreeMap<String, PathFault>) -> Vec<String> {
    const WHY: &str = "so KiCad's length counts copper the signal does not travel";
    let mut lines = Vec::new();
    for (net, fault) in faults {
        if fault.contacts > 0 {
            lines.push(format!(
                "{net}'s copper crosses or overlaps itself {} time(s), {WHY}",
                fault.contacts
            ));
        }
        if fault.dangling > 0 {
            lines.push(format!(
                "{net} has {} dangling or unconnected item(s), {WHY}",
                fault.dangling
            ));
        }
    }
    lines
}

/// Whether G2 counts a KiCad rule key: every key but [`LENGTH_RULE_KEYS`].
pub(crate) fn counts_in_g2(rule: &str) -> bool {
    !LENGTH_RULE_KEYS.contains(&rule)
}

/// KiCad's own length and skew violations on the judged board (`Measures.
/// violations`), summed.
pub(crate) fn kicad_rule_violations(violations: &BTreeMap<String, usize>) -> usize {
    LENGTH_RULE_KEYS
        .iter()
        .map(|key| violations.get(*key).copied().unwrap_or(0))
        .sum()
}

/// G4's lines (L9), one per failure: each net outside its window ("group 'dq'
/// (ram): /DQ7 is 41.20 mm, 0.62 mm below its window 41.82–42.32 mm"), each
/// unmeasured net, each pair over its skew, and KiCad's own violations by key
/// ("KiCad: 2 length_out_of_range violation(s) of the board's own rules"). Empty
/// when everything passes; `measures` is `None` when the call checks nothing.
pub(crate) fn g4_lines(
    measures: Option<&LengthMeasures>,
    violations: &BTreeMap<String, usize>,
) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(measures) = measures {
        for group in &measures.groups {
            // A net with a path fault has its own lines (G4(d), `contact_lines`): its
            // miss is its window's lower bound, not a distance from KiCad's length.
            for net in group
                .nets
                .iter()
                .filter(|net| net.miss_mm > 0.0 && !measures.paths.contains_key(&net.net))
            {
                let Some(mm) = net.kicad_mm else { continue };
                // A miss that rounds to 0.00 is shown with the digits KiCad prints.
                let digits = if net.miss_mm < 0.005 { 4 } else { 2 };
                let way = if mm < net.low_mm { "below" } else { "above" };
                lines.push(format!(
                    "group '{}' ({}): {} is {mm:.digits$} mm, {:.digits$} mm {way} its window {:.digits$}\u{2013}{:.digits$} mm",
                    group.name, group.interface, net.net, net.miss_mm, net.low_mm, net.high_mm
                ));
            }
        }
        for pair in measures.pairs.iter().filter(|pair| pair.miss_mm > 0.0) {
            let Some(skew) = pair.skew_mm else { continue };
            let digits = if pair.miss_mm < 0.005 { 4 } else { 2 };
            lines.push(format!(
                "pair {} / {}: skew {skew:.digits$} mm, {:.digits$} mm over its maximum {:.digits$} mm",
                pair.positive, pair.negative, pair.miss_mm, pair.max_skew_mm
            ));
        }
        for net in &measures.unmeasured {
            lines.push(format!(
                "{net}: KiCad gave no length for this net (not on the board, or 2000 mm or longer)"
            ));
        }
    }
    for key in LENGTH_RULE_KEYS {
        let count = violations.get(key).copied().unwrap_or(0);
        if count > 0 {
            lines.push(format!(
                "KiCad: {count} {key} violation(s) of the board's own rules"
            ));
        }
    }
    lines
}

/// The rank key's length part (L10), positions 3 to 5: G4 failed (0 or 1), the
/// intent miss in [`MISS_STEP_MM`] steps (`round`; `i64::MAX` when a net is
/// unmeasured; 0 without measures), KiCad's own violations.
pub(crate) fn key_part(
    g4_failed: bool,
    measures: Option<&LengthMeasures>,
    violations: &BTreeMap<String, usize>,
) -> [i64; 3] {
    let miss = match measures {
        None => 0,
        Some(measures) if !measures.unmeasured.is_empty() => i64::MAX,
        Some(measures) => (measures.miss_mm / MISS_STEP_MM).round() as i64,
    };
    [
        i64::from(g4_failed),
        miss,
        kicad_rule_violations(violations) as i64,
    ]
}

#[cfg(test)]
#[path = "length_probe_tests.rs"]
mod tests;
