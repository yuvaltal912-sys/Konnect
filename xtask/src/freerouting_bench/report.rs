//! Turning runs into a per-board quality table, per-version totals, a ranking
//! and a recommendation.
//!
//! Completion is a gate, not a score: a board passes when every run wrote a
//! readable SES with no unrouted connection, no clearance violation and
//! without stopping at the job budget. Versions are then compared on quality
//! over the boards that every competing version passed.
//!
//! With `--score`, every run is also judged on the real board against its
//! existing routing (`board_score`), and each config on each DSN is a
//! candidate, ranked by the gates and then the owner's order of measures.

use super::board_score::{BoardScore, Category, Compared, ScoreContext, Verdict};
use super::parse::{Json, SesSummary};
use super::quality::Quality;
use super::{Options, SettingsEvidence};
use std::cmp::Ordering;
use std::path::PathBuf;

/// One Freerouting run on one board.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunRecord {
    pub exit_code: Option<i32>,
    /// Killed at the hard limit (`--timeout-secs`): no result.
    pub timed_out: bool,
    /// Freerouting's own job budget ran out; the SES holds what it routed.
    pub hit_job_budget: bool,
    pub runtime_s: f64,
    /// The version Freerouting printed in its log.
    pub reported_version: Option<String>,
    /// Freerouting logged "Analytics are disabled".
    pub analytics_disabled: bool,
    /// Freerouting's own count of connections left unrouted.
    pub unrouted: Option<u64>,
    /// `connections.maximum_count` (2.1 only).
    pub connections: Option<u64>,
    pub stats_vias: Option<u64>,
    pub clearance_violations: Option<u64>,
    pub ses: Option<SesSummary>,
    pub ses_fingerprint: Option<String>,
    pub quality: Option<Quality>,
    pub quality_error: Option<String>,
    pub error: Option<String>,
    /// What is known about the settings the run really used.
    pub settings: Option<SettingsEvidence>,
    /// The run measured on the real board (`--score`).
    pub board_score: Option<BoardScore>,
    /// That score against the baseline's.
    pub compared: Option<Compared>,
    /// Why the run could not be scored on the board.
    pub score_error: Option<String>,
}

impl RunRecord {
    /// A run that wrote a readable SES and reported its completion.
    pub fn succeeded(&self) -> bool {
        self.error.is_none() && self.ses.is_some() && self.unrouted.is_some()
    }

    /// Keep the run's board score, judged against the context's baseline.
    pub fn judge(&mut self, score: BoardScore, context: &ScoreContext) {
        self.compared = context.baseline.as_ref().ok().map(|baseline| {
            super::board_score::compare(
                &score,
                baseline,
                &context.baseline_roles(),
                &context.spec.thresholds,
            )
        });
        self.board_score = Some(score);
        self.score_error = None;
    }

    /// The board score in a few words, for the progress line.
    fn score_words(&self) -> String {
        match (&self.compared, &self.score_error) {
            (Some(compared), _) => {
                let gates: Vec<String> = compared
                    .gates
                    .iter()
                    .map(|gate| {
                        let short = gate.name.split(' ').next().unwrap_or(gate.name);
                        format!("{short} {}", if gate.pass() { "ok" } else { "FAIL" })
                    })
                    .collect();
                format!(
                    "; board: {}, signals x{}, power x{}",
                    gates.join(" "),
                    ratio_cell(compared.signal_ratio),
                    ratio_cell(compared.power_ratio)
                )
            }
            (None, Some(error)) => format!("; board: not scored ({error})"),
            (None, None) => String::new(),
        }
    }

    pub fn one_line(&self) -> String {
        match (&self.error, &self.ses) {
            (Some(error), _) => format!("FAILED ({error}) after {:.1} s", self.runtime_s),
            (None, Some(ses)) => format!(
                "{} unrouted, {} violations, {} vias, {:.1} mm{}, {:.1} s{}, ses {}{}",
                self.unrouted
                    .map_or("? (no statistics)".to_string(), |n| n.to_string()),
                self.clearance_violations
                    .map_or("?".to_string(), |n| n.to_string()),
                ses.vias,
                ses.wirelength_mm,
                self.quality
                    .as_ref()
                    .and_then(Quality::wire_ratio)
                    .map_or(String::new(), |r| format!(" ({r:.2}x HPWL)")),
                self.runtime_s,
                if self.hit_job_budget {
                    " (job budget hit)"
                } else {
                    ""
                },
                self.ses_fingerprint.as_deref().unwrap_or("-"),
                self.score_words()
            ),
            (None, None) => "no result".to_string(),
        }
    }
}

/// `label`, and `(version)` after it when the label does not already say it
/// (a config's label ends in `@<version>`) and the version is known.
fn display_name(label: &str, version: &str) -> String {
    let said = label == version
        || label
            .rsplit_once('@')
            .is_some_and(|(_, jar_label)| jar_label == version);
    if said || version == "unknown" {
        label.to_string()
    } else {
        format!("{label} ({version})")
    }
}

/// Every run of one JAR on one board.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardResult {
    pub board: String,
    /// The folder this JAR's runs went in, different for every JAR: the key
    /// totals and the ranking group by.
    pub dir: String,
    pub version: String,
    pub jar: PathBuf,
    /// The `--config` these runs used, by name, and its arguments.
    pub config: Option<String>,
    pub config_args: Vec<String>,
    pub dsn_connections: Option<usize>,
    pub components: Option<usize>,
    pub signal_layers: Option<usize>,
    pub runs: Vec<RunRecord>,
}

impl BoardResult {
    /// The JAR as named in the report: its folder label, with the version
    /// Freerouting reported when that differs.
    pub fn name(&self) -> String {
        display_name(&self.dir, &self.version)
    }

    pub fn all_succeeded(&self) -> bool {
        !self.runs.is_empty() && self.runs.iter().all(RunRecord::succeeded)
    }

    /// The completion gate: every run complete, clean and within budget.
    pub fn passes_gate(&self) -> bool {
        self.all_succeeded()
            && self.unrouted() == Some(0)
            && self.violations() == Some(0)
            && !self.budget_limited()
            && self.quality().is_some()
    }

    /// Why the gate failed, in a few words.
    pub fn gate_failure(&self) -> Option<String> {
        if self.passes_gate() {
            return None;
        }
        if let Some(error) = self.runs.iter().find_map(|run| run.error.clone()) {
            return Some(error);
        }
        let mut reasons = Vec::new();
        if let Some(n) = self.unrouted().filter(|n| *n > 0) {
            reasons.push(format!("{n} unrouted"));
        }
        match self.violations() {
            Some(n) if n > 0 => reasons.push(format!("{n} clearance violations")),
            None if self.unrouted().is_some() => {
                reasons.push("clearance violations not reported".to_string())
            }
            _ => {}
        }
        if self.budget_limited() {
            reasons.push("stopped by the job budget".to_string());
        }
        if let Some(error) = self.runs.iter().find_map(|run| run.quality_error.clone()) {
            reasons.push(format!("not measured: {error}"));
        }
        if reasons.is_empty() {
            reasons.push("no result".to_string());
        }
        Some(reasons.join(", "))
    }

    /// Every run wrote the same SES, token for token. `None` with fewer than
    /// two runs or when a run failed: there is nothing to compare.
    pub fn same_ses(&self) -> Option<bool> {
        self.all_agree(|r| r.ses_fingerprint.clone())
    }

    /// Every run laid the same copper: the same segments and vias, in any
    /// order. This is what the ranking calls deterministic; `same_ses` can
    /// be false while this is true when only the order of wires changed.
    pub fn deterministic(&self) -> Option<bool> {
        self.all_agree(|r| r.ses.as_ref().map(|s| s.routing_fingerprint.clone()))
    }

    /// Some run stopped at Freerouting's job budget. Its result depends on how
    /// far the clock let it get, so it is not compared for determinism.
    pub fn budget_limited(&self) -> bool {
        self.runs.iter().any(|r| r.hit_job_budget)
    }

    /// The worst run's clearance violations. `None` when a run did not report
    /// them: that fails the gate, it is not taken for none.
    pub fn violations(&self) -> Option<u64> {
        self.runs
            .iter()
            .map(|r| r.clearance_violations)
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .max()
    }

    fn all_agree(&self, key: impl Fn(&RunRecord) -> Option<String>) -> Option<bool> {
        if self.runs.len() < 2 || !self.all_succeeded() || self.budget_limited() {
            return None;
        }
        let first = key(&self.runs[0]);
        Some(self.runs.iter().all(|r| key(r) == first))
    }

    /// The worst run's unrouted count.
    pub fn unrouted(&self) -> Option<u64> {
        self.runs
            .iter()
            .map(|r| r.unrouted)
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .max()
    }

    /// The first run's quality: the one quoted for the board.
    pub fn quality(&self) -> Option<&Quality> {
        self.runs.first()?.quality.as_ref()
    }

    pub fn median_runtime_s(&self) -> f64 {
        let mut times: Vec<f64> = self.runs.iter().map(|r| r.runtime_s).collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        match times.len() {
            0 => 0.0,
            n if n % 2 == 1 => times[n / 2],
            n => (times[n / 2 - 1] + times[n / 2]) / 2.0,
        }
    }
}

/// One JAR's totals: the gate over every board, quality over the boards
/// every competing JAR passed (`compared_boards`). JARs are told apart by
/// `label` (their folder), not by version, so a rebuild of one version stays
/// a row of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct VersionTotals {
    pub label: String,
    pub version: String,
    pub boards: usize,
    pub passed_boards: usize,
    pub compared_boards: usize,
    pub wirelength_mm: f64,
    pub hpwl_mm: f64,
    pub bound_mm: f64,
    pub vias: usize,
    pub nets_routed: usize,
    pub layer_changes: usize,
    pub right_corners: usize,
    pub acute_corners: usize,
    pub segments: usize,
    pub narrow_segments: usize,
    pub nondeterministic_boards: usize,
    pub deterministic_boards: usize,
    pub runtime_s: f64,
}

impl VersionTotals {
    /// The JAR as named in the ranking: its label, and its version when that differs.
    pub fn name(&self) -> String {
        display_name(&self.label, &self.version)
    }

    pub fn gate_failures(&self) -> usize {
        self.boards - self.passed_boards
    }

    pub fn wire_ratio(&self) -> Option<f64> {
        (self.bound_mm > 0.0).then(|| self.wirelength_mm / self.bound_mm)
    }

    pub fn hpwl_ratio(&self) -> Option<f64> {
        (self.hpwl_mm > 0.0).then(|| self.wirelength_mm / self.hpwl_mm)
    }

    pub fn defects(&self) -> usize {
        self.acute_corners + self.narrow_segments
    }
}

/// The JAR labels, in the order they first appear.
fn labels_in_order(results: &[BoardResult]) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for result in results {
        if !labels.contains(&result.dir) {
            labels.push(result.dir.clone());
        }
    }
    labels
}

/// Boards that every competing JAR passed. A JAR that passed no board
/// competes on the gate alone, so it cannot empty this set.
pub fn compared_boards(results: &[BoardResult]) -> Vec<String> {
    let competing: Vec<String> = labels_in_order(results)
        .into_iter()
        .filter(|v| results.iter().any(|r| &r.dir == v && r.passes_gate()))
        .collect();
    let mut boards: Vec<String> = Vec::new();
    for result in results {
        if !boards.contains(&result.board) {
            boards.push(result.board.clone());
        }
    }
    boards
        .into_iter()
        .filter(|board| {
            !competing.is_empty()
                && competing.iter().all(|label| {
                    results
                        .iter()
                        .any(|r| &r.board == board && &r.dir == label && r.passes_gate())
                })
        })
        .collect()
}

pub fn totals(results: &[BoardResult]) -> Vec<VersionTotals> {
    let compared = compared_boards(results);
    labels_in_order(results)
        .into_iter()
        .map(|label| {
            let mine: Vec<&BoardResult> = results.iter().filter(|r| r.dir == label).collect();
            let passed = mine.iter().filter(|r| r.passes_gate()).count();
            let measured: Vec<(&BoardResult, &Quality)> = mine
                .iter()
                .filter(|r| r.passes_gate() && compared.contains(&r.board))
                .filter_map(|r| Some((*r, r.quality()?)))
                .collect();
            let sum = |f: &dyn Fn(&Quality) -> usize| measured.iter().map(|(_, q)| f(q)).sum();
            let sum_f = |f: &dyn Fn(&Quality) -> f64| {
                measured.iter().map(|(_, q)| f(q)).fold(0.0, |a, b| a + b)
            };
            VersionTotals {
                boards: mine.len(),
                passed_boards: passed,
                compared_boards: measured.len(),
                wirelength_mm: sum_f(&|q| q.wirelength_mm),
                hpwl_mm: sum_f(&|q| q.hpwl_mm),
                bound_mm: sum_f(&|q| q.bound_mm),
                vias: sum(&|q| q.vias),
                nets_routed: sum(&|q| q.nets_routed),
                layer_changes: sum(&|q| q.layer_changes),
                right_corners: sum(&|q| q.right_corners),
                acute_corners: sum(&|q| q.acute_corners),
                segments: sum(&|q| q.segments),
                narrow_segments: sum(&|q| q.narrow_segments),
                nondeterministic_boards: measured
                    .iter()
                    .filter(|(r, _)| r.deterministic() == Some(false))
                    .count(),
                deterministic_boards: measured
                    .iter()
                    .filter(|(r, _)| r.deterministic() == Some(true))
                    .count(),
                runtime_s: measured
                    .iter()
                    .map(|(r, _)| r.median_runtime_s())
                    .fold(0.0, |a, b| a + b),
                version: mine[0].version.clone(),
                label,
            }
        })
        .collect()
}

/// Best first: fewest boards failing the gate; then, on the compared boards,
/// fewest defects (acid traps, segments below their class width), wiring
/// closest to its lower bound (in steps of 1 %, so a negligible
/// difference does not hide the next measure), fewest vias, fewest boards
/// whose runs laid different copper, least time.
pub fn rank(mut versions: Vec<VersionTotals>) -> Vec<VersionTotals> {
    let ratio_step = |t: &VersionTotals| {
        t.wire_ratio()
            .map_or(u64::MAX, |r| (r * 100.0).round() as u64)
    };
    versions.sort_by(|a, b| {
        (
            a.gate_failures(),
            a.defects(),
            ratio_step(a),
            a.vias,
            a.nondeterministic_boards,
        )
            .cmp(&(
                b.gate_failures(),
                b.defects(),
                ratio_step(b),
                b.vias,
                b.nondeterministic_boards,
            ))
            .then(
                a.runtime_s
                    .partial_cmp(&b.runtime_s)
                    .unwrap_or(Ordering::Equal),
            )
    });
    versions
}

pub fn describe(t: &VersionTotals) -> String {
    let ratio = t
        .wire_ratio()
        .map_or("-".to_string(), |r| format!("{r:.3}"));
    let hpwl = t
        .hpwl_ratio()
        .map_or("-".to_string(), |r| format!("{r:.3}"));
    format!(
        "{}: passed the completion gate on {} of {} boards. On the {} compared boards: wiring {ratio} x its lower bound and {hpwl} x HPWL ({:.1} mm), {} vias ({} layer changes) over {} nets, {} acute and {} right-angle corners, {} of {} segments below their class width, same routing every run on {} (differs on {}), {:.1} s",
        t.name(),
        t.passed_boards,
        t.boards,
        t.compared_boards,
        t.wirelength_mm,
        t.vias,
        t.layer_changes,
        t.nets_routed,
        t.acute_corners,
        t.right_corners,
        t.narrow_segments,
        t.segments,
        t.deterministic_boards,
        t.nondeterministic_boards,
        t.runtime_s
    )
}

/// One config on one DSN, over all its runs, judged on the real board. A
/// candidate is only as good as its worst run: the gates must hold in every
/// run, and each measure is quoted at its worst.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub name: String,
    pub runs: usize,
    /// Runs with a board score.
    pub scored: usize,
    /// Runs that failed on their own account (no SES, unrouted count not
    /// reported, settings not in effect).
    pub failed_runs: usize,
    /// A run that failed, or was not scored: the candidate cannot be chosen.
    pub incomplete: bool,
    /// Some run was stopped by Freerouting's job budget: its routing is what
    /// the clock allowed, said beside the candidate but not a gate.
    pub budget_limited: bool,
    /// G1 completion, G2 DRC, G3 width: failed in some run.
    pub failed_gates: [bool; 3],
    pub acute: Option<f64>,
    pub signal_ratio: Option<f64>,
    pub power_ratio: Option<f64>,
    pub total_ratio: Option<f64>,
    pub pour_track_mm: Option<f64>,
    pub right_angles: Option<f64>,
    pub vias: Option<usize>,
    pub copper_quality: Option<f64>,
    pub verdict: Verdict,
    pub runtime_s: f64,
}

impl Candidate {
    pub fn passes(&self) -> bool {
        !self.incomplete && self.failed_gates == [false; 3]
    }
}

fn worst(values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    values
        .collect::<Option<Vec<f64>>>()?
        .into_iter()
        .fold(None, |worst: Option<f64>, v| {
            Some(worst.map_or(v, |w| w.max(v)))
        })
}

/// The candidates of a scored run: every entry with a board score in any run.
pub fn candidates(results: &[BoardResult]) -> Vec<Candidate> {
    let several_boards = results.iter().any(|r| r.board != results[0].board);
    results
        .iter()
        .filter(|r| {
            r.runs
                .iter()
                .any(|run| run.board_score.is_some() || run.score_error.is_some())
        })
        .map(|r| {
            let scored: Vec<(&BoardScore, &Compared)> = r
                .runs
                .iter()
                .filter_map(|run| Some((run.board_score.as_ref()?, run.compared.as_ref()?)))
                .collect();
            let metric = |key: &str| worst(scored.iter().map(|(score, _)| score.audit.metric(key)));
            let mut failed_gates = [false; 3];
            for (_, compared) in &scored {
                for (failed, gate) in failed_gates.iter_mut().zip(&compared.gates) {
                    *failed |= !gate.pass();
                }
            }
            let verdicts: Vec<Verdict> = scored.iter().map(|(_, c)| c.verdict()).collect();
            Candidate {
                name: if several_boards {
                    format!("{} on {}", r.name(), r.board)
                } else {
                    r.name()
                },
                runs: r.runs.len(),
                scored: scored.len(),
                failed_runs: r.runs.iter().filter(|run| !run.succeeded()).count(),
                incomplete: scored.len() < r.runs.len() || !r.all_succeeded(),
                budget_limited: r.budget_limited(),
                failed_gates,
                acute: metric("5.3.acute_corners"),
                signal_ratio: worst(scored.iter().map(|(_, c)| c.signal_ratio)),
                power_ratio: worst(scored.iter().map(|(_, c)| c.power_ratio)),
                total_ratio: worst(scored.iter().map(|(_, c)| c.total_ratio)),
                pour_track_mm: worst(scored.iter().map(|(s, _)| Some(s.length(Category::Pour)))),
                right_angles: metric("5.3.right_angle_corners"),
                vias: scored.iter().map(|(s, _)| s.non_pour_vias()).max(),
                copper_quality: worst(
                    scored
                        .iter()
                        .map(|(s, _)| s.audit.copper_quality.map(|q| -q)),
                )
                .map(|q| -q),
                verdict: [Verdict::Below, Verdict::Matches, Verdict::Beats]
                    .into_iter()
                    .find(|v| verdicts.contains(v))
                    .unwrap_or(Verdict::NotMeasured),
                runtime_s: r.median_runtime_s(),
            }
        })
        .collect()
}

/// Best first, in the owner's order: the gates (a candidate with a failed or
/// unscored run last, then completion before DRC before width), then acute
/// corners, signal length, power length (both in steps of 1 %, since two runs
/// of one config differ by about that), right-angle corners, vias off the
/// pour nets, copper quality (in whole points), runtime.
pub fn rank_candidates(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    let whole = |value: Option<f64>| value.map_or(u64::MAX, |v| v.round().max(0.0) as u64);
    let step = |ratio: Option<f64>| ratio.map_or(u64::MAX, |r| (r * 100.0).round() as u64);
    let key = |c: &Candidate| {
        (
            (c.incomplete, c.failed_gates),
            whole(c.acute),
            step(c.signal_ratio),
            step(c.power_ratio),
            whole(c.right_angles),
            c.vias.unwrap_or(usize::MAX),
            std::cmp::Reverse(c.copper_quality.map_or(0, |q| q.round().max(0.0) as u64)),
        )
    };
    candidates.sort_by(|a, b| {
        key(a).cmp(&key(b)).then(
            a.runtime_s
                .partial_cmp(&b.runtime_s)
                .unwrap_or(Ordering::Equal),
        )
    });
    candidates
}

fn ratio_cell(ratio: Option<f64>) -> String {
    ratio.map_or("-".to_string(), |r| format!("{r:.3}"))
}

fn gate_cell(failed: bool) -> &'static str {
    if failed {
        "FAIL"
    } else {
        "pass"
    }
}

pub fn describe_candidate(c: &Candidate) -> String {
    let gates = if c.passes() {
        "passes every gate in every run".to_string()
    } else {
        let mut why = Vec::new();
        if c.scored < c.runs {
            why.push(format!("{} of {} runs scored", c.scored, c.runs));
        }
        if c.failed_runs > 0 {
            why.push(format!("{} of {} runs failed", c.failed_runs, c.runs));
        }
        for (failed, name) in c
            .failed_gates
            .iter()
            .zip(["G1 completion", "G2 DRC", "G3 width"])
        {
            if *failed {
                why.push(format!("{name} failed"));
            }
        }
        why.join(", ")
    };
    format!(
        "{}: {gates}; {} acute corners, signals x{}, power x{}, {} right angles, {} vias off the pour nets, copper quality {}, {:.1} s; {} the baseline{}",
        c.name,
        cell(c.acute, |v| format!("{v:.0}")),
        ratio_cell(c.signal_ratio),
        ratio_cell(c.power_ratio),
        cell(c.right_angles, |v| format!("{v:.0}")),
        cell(c.vias, |v| v.to_string()),
        cell(c.copper_quality, |v| format!("{v:.1}")),
        c.runtime_s,
        c.verdict.word(),
        if c.budget_limited {
            " (a run was stopped by the job budget)"
        } else {
            ""
        }
    )
}

/// The board-score section of the report: the baseline row, every scored
/// run, the ranking, and why each gate that failed did.
fn render_board_scores(results: &[BoardResult], context: &ScoreContext) -> String {
    let mut out = String::new();
    let t = &context.spec.thresholds;
    out.push_str(&format!(
        "\n## Board score\n\n\
         Every run was put into a copy of the board, refilled and checked by kicad-cli, audited with `audit_layout_quality` (konnect {}), and measured by net category, as was the existing routing (the baseline). Intent currents: {}.\n\n\
         **Gates** (every run of a candidate must pass all three): G1 completion: 0 unconnected in KiCad and a routed share of 1 in the audit. G2 DRC: no violation type above the baseline's count. G3 width: no track below the narrowest class width, 0 `track_width` violations, each power net (a recorded current, no pour) below its intent width on at most {:.0} % of its length and for no stretch over {} mm, and no more undersized nets or bottlenecks in the audit (4.2, 4.3) than the baseline.\n\n\
         **Quality** is against the baseline: lengths are ratios over the nets that have track on both boards; pour nets are the nets that own a pour (the current is in the pour, so their track is quoted in mm). Matching bands: signals x{}, power x{}, pour-net track +{} mm, right angles +{}, scores -{}, loops +{:.0} %.\n\n",
        context.konnect_commit.as_deref().map_or("unknown", |c| &c[..c.len().min(12)]),
        context.spec.currents,
        t.below_intent_share * 100.0,
        t.below_intent_section_mm,
        t.signal_match,
        t.power_match,
        t.pour_track_match_mm,
        t.right_angle_match,
        t.score_match,
        t.loop_match * 100.0,
    ));
    out.push_str("| run | G1 | G2 | G3 | signals x | power x | all x | pour-net track mm | vias pour / other | acute | 90° | copper quality | unconnected | against the baseline |\n");
    out.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    let row = |name: &str, score: &BoardScore, compared: Option<&Compared>| {
        let gate = |i: usize| compared.map_or("-", |c| gate_cell(!c.gates[i].pass()));
        format!(
            "| {} | {} | {} | {} | {} | {} | {} | {:.1} | {} / {} | {} | {} | {} | {} | {} |\n",
            name.replace('|', "/"),
            gate(0),
            gate(1),
            gate(2),
            compared.map_or("1.000".to_string(), |c| ratio_cell(c.signal_ratio)),
            compared.map_or("1.000".to_string(), |c| ratio_cell(c.power_ratio)),
            compared.map_or("1.000".to_string(), |c| ratio_cell(c.total_ratio)),
            score.length(Category::Pour),
            score.vias[Category::Pour as usize],
            score.non_pour_vias(),
            cell(score.audit.metric("5.3.acute_corners"), |v| format!(
                "{v:.0}"
            )),
            cell(score.audit.metric("5.3.right_angle_corners"), |v| format!(
                "{v:.0}"
            )),
            cell(score.audit.copper_quality, |v| format!("{v:.1}")),
            score.drc.unconnected,
            compared.map_or("-", |c| c.verdict().word()),
        )
    };
    match &context.baseline {
        Ok(baseline) => out.push_str(&row("baseline (existing routing)", baseline, None)),
        Err(error) => out.push_str(&format!("\nThe baseline was not scored: {error}\n\n")),
    }
    let several_boards = results.iter().any(|r| r.board != results[0].board);
    let run_name = |r: &BoardResult, index: usize| {
        if several_boards {
            format!("{} on {}, run {}", r.name(), r.board, index + 1)
        } else {
            format!("{}, run {}", r.name(), index + 1)
        }
    };
    let mut problems = String::new();
    for r in results {
        for (index, run) in r.runs.iter().enumerate() {
            let name = run_name(r, index);
            match (&run.board_score, &run.score_error) {
                (Some(score), _) => out.push_str(&row(&name, score, run.compared.as_ref())),
                (None, Some(error)) => {
                    problems.push_str(&format!("- {name}: not scored: {error}\n"))
                }
                (None, None) => {}
            }
            if let Some(error) = &run.error {
                problems.push_str(&format!("- {name}: run failed: {error}\n"));
            }
            for gate in run.compared.iter().flat_map(|c| &c.gates) {
                for failure in &gate.failures {
                    problems.push_str(&format!("- {name}: {}: {failure}\n", gate.name));
                }
            }
        }
    }
    let ranked = rank_candidates(candidates(results));
    out.push_str("\n### Ranking of the candidates\n\nEach config on each DSN is a candidate, quoted at its worst run.\n\n");
    for (i, candidate) in ranked.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, describe_candidate(candidate)));
    }
    match ranked.first() {
        Some(best) if best.passes() => out.push_str(&format!(
            "\nChosen: {} (ranked by the gates, then acute corners, signal length and power length in 1 % steps, right angles, vias, copper quality, runtime).\n",
            best.name
        )),
        Some(best) => out.push_str(&format!(
            "\nNo candidate passes every gate in every run. Nearest: {}.\n",
            best.name
        )),
        None => {}
    }
    if !problems.is_empty() {
        out.push_str("\n### Failed gates and runs\n\n");
        out.push_str(&problems);
    }
    let mut below = String::new();
    for r in results {
        for (index, run) in r.runs.iter().enumerate() {
            let Some(compared) = &run.compared else {
                continue;
            };
            let listed: Vec<String> = compared
                .bands
                .iter()
                .filter(|band| band.verdict == Verdict::Below)
                .map(|band| {
                    format!(
                        "{} {} (baseline {})",
                        band.measure,
                        cell(band.value, |v| format!("{v:.3}")),
                        cell(band.baseline, |v| format!("{v:.3}"))
                    )
                })
                .collect();
            if !listed.is_empty() {
                below.push_str(&format!(
                    "- {}: {}\n",
                    run_name(r, index),
                    listed.join("; ")
                ));
            }
        }
    }
    if !below.is_empty() {
        out.push_str("\n### Measures below the baseline\n\n");
        out.push_str(&below);
    }
    let unsure: Vec<String> = results
        .iter()
        .flat_map(|r| r.runs.iter().enumerate().map(move |(i, run)| (r, i, run)))
        .filter_map(|(r, i, run)| {
            let settings = run.settings.as_ref()?;
            (!settings.unconfirmed.is_empty()).then(|| {
                format!(
                    "- {}: {}\n",
                    run_name(r, i),
                    settings.unconfirmed.join(", ")
                )
            })
        })
        .collect();
    if !unsure.is_empty() {
        out.push_str("\n### Settings Freerouting did not save back\n\nGiven on the command line and not refused, but absent from the `freerouting.json` the run wrote, so not confirmed.\n\n");
        out.push_str(&unsure.concat());
    }
    out
}

fn yes_no(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "-",
    }
}

fn cell<T>(value: Option<T>, f: impl FnOnce(T) -> String) -> String {
    value.map_or("-".to_string(), f)
}

pub fn render_markdown(
    results: &[BoardResult],
    options: &Options,
    java: Option<&str>,
    context: Option<&ScoreContext>,
) -> String {
    let mut out = String::new();
    out.push_str("# Freerouting benchmark\n\n");
    out.push_str(&format!(
        "Java {}, {} runs per board, -mp {}, -mt {}, job budget {} s, hard limit {} s.\n\n\
         **Gate:** every run wrote a readable SES with 0 unrouted connections (Freerouting's own count) and 0 clearance violations, within the job budget. Only boards that pass are measured for quality.\n\n\
         **Quality** (first run): wire/bound is the wiring of the routed nets (leaving out nets that also have a pour or plane, or wiring already in the DSN) over half the perimeter of their pins' convex hull, a lower bound no wiring at any angle beats (1.00 is the unreachable ideal). wire/HPWL is the same wiring over the half-perimeter of the pins' bounding box, the usual reference; it is a bound only for 90° routing, so 45° wiring can go below 1.00. Vias/net is over nets with wiring. Layer changes are vias where wires on two layers meet. Corners are counted away from pads and vias: acute (under 89.5°, acid traps) and right (90 ± 0.5°). Tracks are unique segments; narrow ones are below their net class's width. Runtime is wall clock per run with JVM start (median). Same routing: every run laid the same segments and vias in any order.\n\n",
        java.unwrap_or("unknown"),
        options.runs,
        options.max_passes,
        options.threads,
        options.job_timeout.as_secs(),
        options.timeout.as_secs()
    ));
    out.push_str("| board | parts | layers | version | gate | wire/bound | wire/HPWL | wire mm | vias | vias/net | layer changes | acute | 90° | tracks | narrow | runtime s | same routing |\n");
    out.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for r in results {
        let q = r.quality().filter(|_| r.passes_gate());
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {:.1} | {} |\n",
            r.board.replace('|', "/"),
            cell(r.components, |n| n.to_string()),
            cell(r.signal_layers, |n| n.to_string()),
            r.name().replace('|', "/"),
            r.gate_failure()
                .map_or("pass".to_string(), |why| format!("fail: {}", why.replace('|', "/"))),
            cell(q.and_then(Quality::wire_ratio), |v| format!("{v:.3}")),
            cell(q.and_then(Quality::hpwl_ratio), |v| format!("{v:.3}")),
            cell(q, |q| format!("{:.1}", q.wirelength_mm)),
            cell(q, |q| q.vias.to_string()),
            cell(q.and_then(Quality::vias_per_net), |v| format!("{v:.2}")),
            cell(q, |q| q.layer_changes.to_string()),
            cell(q, |q| q.acute_corners.to_string()),
            cell(q, |q| q.right_corners.to_string()),
            cell(q, |q| q.segments.to_string()),
            cell(q, |q| q.narrow_segments.to_string()),
            r.median_runtime_s(),
            yes_no(r.deterministic()),
        ));
    }
    let compared = compared_boards(results);
    let ranked = rank(totals(results));
    out.push_str(&format!(
        "\n## Ranking\n\nQuality is compared on the {} boards every competing version passed{}.\n\n",
        compared.len(),
        if compared.is_empty() {
            String::new()
        } else {
            format!(": {}", compared.join(", "))
        }
    ));
    for (i, t) in ranked.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, describe(t)));
    }
    if let Some(best) = ranked.first() {
        out.push_str(&format!(
            "\nRecommended: {} (ranked by boards failing the gate, then defects, wire/bound in 1 % steps, vias, determinism, runtime).\n",
            best.name()
        ));
    }
    if let Some(context) = context {
        out.push_str(&render_board_scores(results, context));
    }
    out
}

fn optional<T>(value: Option<T>, f: impl FnOnce(T) -> Json) -> Json {
    value.map_or(Json::Null, f)
}

fn number(n: impl Into<f64>) -> Json {
    Json::Number(n.into())
}

fn count(n: usize) -> Json {
    Json::Number(n as f64)
}

fn quality_json(q: &Quality) -> Json {
    Json::Object(vec![
        ("nets_routed".into(), count(q.nets_routed)),
        ("wirelength_mm".into(), number(q.wirelength_mm)),
        ("hpwl_mm".into(), number(q.hpwl_mm)),
        ("bound_mm".into(), number(q.bound_mm)),
        ("hpwl_ratio".into(), optional(q.hpwl_ratio(), number)),
        ("plane_nets_routed".into(), count(q.plane_nets_routed)),
        ("prewired_nets_routed".into(), count(q.prewired_nets_routed)),
        ("wire_ratio".into(), optional(q.wire_ratio(), number)),
        ("vias".into(), count(q.vias)),
        ("vias_per_net".into(), optional(q.vias_per_net(), number)),
        ("layer_changes".into(), count(q.layer_changes)),
        ("right_corners".into(), count(q.right_corners)),
        ("acute_corners".into(), count(q.acute_corners)),
        ("segments".into(), count(q.segments)),
        ("narrow_segments".into(), count(q.narrow_segments)),
        ("unknown_nets".into(), count(q.unknown_nets)),
        (
            "widths".into(),
            Json::Array(
                q.widths
                    .iter()
                    .map(|w| {
                        Json::Object(vec![
                            ("class".into(), Json::Text(w.class.clone())),
                            ("width_mm".into(), number(w.width_mm)),
                            ("segments".into(), count(w.segments)),
                            ("narrow_segments".into(), count(w.narrow_segments)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn texts(values: &[String]) -> Json {
    Json::Array(values.iter().cloned().map(Json::Text).collect())
}

fn settings_json(settings: &SettingsEvidence) -> Json {
    Json::Object(vec![
        ("failures".into(), texts(&settings.failures)),
        ("differing".into(), texts(&settings.differing)),
        ("unconfirmed".into(), texts(&settings.unconfirmed)),
        (
            "saved".into(),
            Json::Object(
                settings
                    .saved
                    .iter()
                    .map(|(path, value)| (path.clone(), Json::Text(value.clone())))
                    .collect(),
            ),
        ),
    ])
}

/// A board score as numbers. Per-net lengths stay out: `--rescore` reads them
/// again from the run's `score/` files.
fn score_json(score: &BoardScore) -> Json {
    let by_category = |values: [f64; 3]| {
        Json::Object(vec![
            ("pour".into(), number(values[Category::Pour as usize])),
            ("power".into(), number(values[Category::Power as usize])),
            ("signal".into(), number(values[Category::Signal as usize])),
        ])
    };
    let numbers = |map: &std::collections::BTreeMap<String, f64>| {
        Json::Object(map.iter().map(|(k, v)| (k.clone(), number(*v))).collect())
    };
    Json::Object(vec![
        ("placement".into(), Json::Text(score.placement.clone())),
        ("segments".into(), count(score.segments)),
        ("length_mm".into(), by_category(score.length_mm)),
        ("total_mm".into(), number(score.total_mm())),
        ("vias".into(), by_category(score.vias.map(|v| v as f64))),
        (
            "below_base_segments".into(),
            count(score.below_base_segments),
        ),
        (
            "widths".into(),
            Json::Array(
                score
                    .widths
                    .iter()
                    .map(|w| {
                        Json::Object(vec![
                            ("net".into(), Json::Text(w.net.clone())),
                            ("intent_mm".into(), number(w.intent_mm)),
                            ("length_mm".into(), number(w.length_mm)),
                            ("below_mm".into(), number(w.below_mm)),
                            ("below_share".into(), number(w.below_share())),
                            ("longest_below_mm".into(), number(w.longest_below_mm)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "drc".into(),
            Json::Object(
                score
                    .drc
                    .by_type
                    .iter()
                    .map(|(kind, n)| (kind.clone(), count(*n)))
                    .collect(),
            ),
        ),
        ("unconnected".into(), count(score.drc.unconnected)),
        (
            "audit".into(),
            Json::Object(vec![
                (
                    "copper_quality".into(),
                    optional(score.audit.copper_quality, number),
                ),
                ("score".into(), optional(score.audit.score, number)),
                (
                    "routed_share".into(),
                    optional(score.audit.routed_share, number),
                ),
                (
                    "complete".into(),
                    optional(score.audit.complete, Json::Bool),
                ),
                ("categories".into(), numbers(&score.audit.categories)),
                ("metrics".into(), numbers(&score.audit.metrics)),
            ]),
        ),
    ])
}

fn compared_json(compared: &Compared) -> Json {
    Json::Object(vec![
        ("passes".into(), Json::Bool(compared.passes())),
        (
            "gates".into(),
            Json::Array(
                compared
                    .gates
                    .iter()
                    .map(|gate| {
                        Json::Object(vec![
                            ("gate".into(), Json::Text(gate.name.to_string())),
                            ("pass".into(), Json::Bool(gate.pass())),
                            ("failures".into(), texts(&gate.failures)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "signal_ratio".into(),
            optional(compared.signal_ratio, number),
        ),
        ("signal_nets".into(), count(compared.signal_nets)),
        ("power_ratio".into(), optional(compared.power_ratio, number)),
        ("power_nets".into(), count(compared.power_nets)),
        ("total_ratio".into(), optional(compared.total_ratio, number)),
        (
            "against_baseline".into(),
            Json::Text(compared.verdict().word().to_string()),
        ),
        (
            "bands".into(),
            Json::Array(
                compared
                    .bands
                    .iter()
                    .map(|band| {
                        Json::Object(vec![
                            ("measure".into(), Json::Text(band.measure.clone())),
                            ("value".into(), optional(band.value, number)),
                            ("baseline".into(), optional(band.baseline, number)),
                            (
                                "verdict".into(),
                                Json::Text(band.verdict.word().to_string()),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

pub fn results_json(
    results: &[BoardResult],
    options: &Options,
    java: Option<&str>,
    context: Option<&ScoreContext>,
) -> Json {
    let runs = |r: &BoardResult| {
        Json::Array(
            r.runs
                .iter()
                .map(|run| {
                    Json::Object(vec![
                        ("exit_code".into(), optional(run.exit_code, number)),
                        ("timed_out".into(), Json::Bool(run.timed_out)),
                        ("hit_job_budget".into(), Json::Bool(run.hit_job_budget)),
                        ("runtime_s".into(), number(run.runtime_s)),
                        (
                            "reported_version".into(),
                            optional(run.reported_version.clone(), Json::Text),
                        ),
                        (
                            "analytics_disabled".into(),
                            Json::Bool(run.analytics_disabled),
                        ),
                        (
                            "unrouted".into(),
                            optional(run.unrouted, |n| number(n as f64)),
                        ),
                        (
                            "connections".into(),
                            optional(run.connections, |n| number(n as f64)),
                        ),
                        (
                            "stats_vias".into(),
                            optional(run.stats_vias, |n| number(n as f64)),
                        ),
                        (
                            "clearance_violations".into(),
                            optional(run.clearance_violations, |n| number(n as f64)),
                        ),
                        (
                            "ses_vias".into(),
                            optional(run.ses.as_ref(), |s| count(s.vias)),
                        ),
                        (
                            "duplicate_segments".into(),
                            optional(run.ses.as_ref(), |s| count(s.duplicate_segments)),
                        ),
                        (
                            "ses_fingerprint".into(),
                            optional(run.ses_fingerprint.clone(), Json::Text),
                        ),
                        (
                            "routing_fingerprint".into(),
                            optional(run.ses.as_ref(), |s| {
                                Json::Text(s.routing_fingerprint.clone())
                            }),
                        ),
                        (
                            "quality".into(),
                            optional(run.quality.as_ref(), quality_json),
                        ),
                        (
                            "quality_error".into(),
                            optional(run.quality_error.clone(), Json::Text),
                        ),
                        ("error".into(), optional(run.error.clone(), Json::Text)),
                        (
                            "settings".into(),
                            optional(run.settings.as_ref(), settings_json),
                        ),
                        (
                            "board_score".into(),
                            optional(run.board_score.as_ref(), score_json),
                        ),
                        (
                            "compared".into(),
                            optional(run.compared.as_ref(), compared_json),
                        ),
                        (
                            "score_error".into(),
                            optional(run.score_error.clone(), Json::Text),
                        ),
                    ])
                })
                .collect(),
        )
    };
    let ranked = rank(totals(results));
    let ranked_candidates = rank_candidates(candidates(results));
    let score = context.map(|context| {
        Json::Object(vec![
            ("currents".into(), Json::Text(context.spec.currents.clone())),
            ("thresholds".into(), context.spec.thresholds.to_json()),
            (
                "konnect_commit".into(),
                optional(context.konnect_commit.clone(), Json::Text),
            ),
            (
                "baseline".into(),
                match &context.baseline {
                    Ok(baseline) => score_json(baseline),
                    Err(error) => Json::Object(vec![("error".into(), Json::Text(error.clone()))]),
                },
            ),
            (
                "ranking".into(),
                Json::Array(
                    ranked_candidates
                        .iter()
                        .map(|c| Json::Text(describe_candidate(c)))
                        .collect(),
                ),
            ),
            (
                "chosen".into(),
                optional(
                    ranked_candidates
                        .first()
                        .filter(|c| c.passes())
                        .map(|c| c.name.clone()),
                    Json::Text,
                ),
            ),
        ])
    });
    Json::Object(vec![
        (
            "java".into(),
            optional(java.map(str::to_string), Json::Text),
        ),
        ("runs_per_board".into(), number(options.runs)),
        ("max_passes".into(), number(options.max_passes)),
        ("threads".into(), number(options.threads)),
        (
            "job_timeout_s".into(),
            number(options.job_timeout.as_secs() as f64),
        ),
        ("timeout_s".into(), number(options.timeout.as_secs() as f64)),
        ("fr_args".into(), texts(&options.extra_args)),
        (
            "score_spec".into(),
            optional(
                context.map(|c| c.spec.source.display().to_string()),
                Json::Text,
            ),
        ),
        ("score".into(), optional(score, |score| score)),
        (
            "boards".into(),
            Json::Array(
                results
                    .iter()
                    .map(|r| {
                        Json::Object(vec![
                            ("board".into(), Json::Text(r.board.clone())),
                            ("dir".into(), Json::Text(r.dir.clone())),
                            ("version".into(), Json::Text(r.version.clone())),
                            ("jar".into(), Json::Text(r.jar.display().to_string())),
                            ("config".into(), optional(r.config.clone(), Json::Text)),
                            ("config_args".into(), texts(&r.config_args)),
                            ("components".into(), optional(r.components, count)),
                            ("signal_layers".into(), optional(r.signal_layers, count)),
                            ("dsn_connections".into(), optional(r.dsn_connections, count)),
                            ("passes_gate".into(), Json::Bool(r.passes_gate())),
                            (
                                "gate_failure".into(),
                                optional(r.gate_failure(), Json::Text),
                            ),
                            (
                                "deterministic".into(),
                                optional(r.deterministic(), Json::Bool),
                            ),
                            ("same_ses".into(), optional(r.same_ses(), Json::Bool)),
                            ("runs".into(), runs(r)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "compared_boards".into(),
            Json::Array(
                compared_boards(results)
                    .into_iter()
                    .map(Json::Text)
                    .collect(),
            ),
        ),
        (
            "ranking".into(),
            Json::Array(ranked.iter().map(|t| Json::Text(describe(t))).collect()),
        ),
        (
            "recommended".into(),
            optional(ranked.first().map(VersionTotals::name), Json::Text),
        ),
    ])
}

pub fn to_json(value: &Json) -> String {
    let mut out = String::new();
    write_json(value, 0, &mut out);
    out.push('\n');
    out
}

fn write_json(value: &Json, indent: usize, out: &mut String) {
    let pad = |n: usize| "  ".repeat(n);
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Number(n) if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 => {
            out.push_str(&format!("{}", *n as i64))
        }
        Json::Number(n) if n.is_finite() => out.push_str(&format!("{n}")),
        Json::Number(_) => out.push_str("null"),
        Json::Text(s) => {
            out.push('"');
            for c in s.chars() {
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
        }
        Json::Array(items) if items.is_empty() => out.push_str("[]"),
        Json::Array(items) => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                out.push_str(&pad(indent + 1));
                write_json(item, indent + 1, out);
                out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(indent));
            out.push(']');
        }
        Json::Object(fields) if fields.is_empty() => out.push_str("{}"),
        Json::Object(fields) => {
            out.push_str("{\n");
            for (i, (key, item)) in fields.iter().enumerate() {
                out.push_str(&pad(indent + 1));
                write_json(&Json::Text(key.clone()), indent + 1, out);
                out.push_str(": ");
                write_json(item, indent + 1, out);
                out.push_str(if i + 1 < fields.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(indent));
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse::{parse_json_prefix, statistics_block};
    use super::*;

    fn quality(mm: f64, hpwl: f64, vias: usize) -> Quality {
        Quality {
            nets_routed: 2,
            wirelength_mm: mm,
            hpwl_mm: hpwl,
            bound_mm: hpwl,
            vias,
            segments: 4,
            ..Quality::default()
        }
    }

    fn run(unrouted: u64, vias: usize, mm: f64, secs: f64, fingerprint: &str) -> RunRecord {
        RunRecord {
            exit_code: Some(0),
            runtime_s: secs,
            unrouted: Some(unrouted),
            clearance_violations: Some(0),
            ses: Some(SesSummary {
                nets_with_wiring: 2,
                vias,
                wire_segments: 4,
                duplicate_segments: 0,
                wirelength_mm: mm,
                routing_fingerprint: fingerprint.to_string(),
            }),
            ses_fingerprint: Some(fingerprint.to_string()),
            quality: Some(quality(mm, 10.0, vias)),
            ..RunRecord::default()
        }
    }

    fn failed() -> RunRecord {
        RunRecord {
            exit_code: Some(1),
            error: Some("no SES written (exit Some(1))".into()),
            runtime_s: 2.0,
            ..RunRecord::default()
        }
    }

    fn board(board: &str, version: &str, runs: Vec<RunRecord>) -> BoardResult {
        BoardResult {
            board: board.into(),
            dir: version.into(),
            version: version.into(),
            jar: PathBuf::from(format!("/j/freerouting-{version}.jar")),
            config: None,
            config_args: Vec::new(),
            dsn_connections: Some(10),
            components: Some(40),
            signal_layers: Some(2),
            runs,
        }
    }

    fn options() -> Options {
        super::super::parse_options(&["--jar".to_string(), "a.jar".to_string()]).unwrap()
    }

    #[test]
    fn the_gate_needs_complete_clean_runs_within_budget() {
        let good = board(
            "a",
            "v",
            vec![run(0, 1, 11.0, 1.0, "x"), run(0, 1, 11.0, 1.0, "x")],
        );
        assert!(good.passes_gate());
        assert_eq!(good.gate_failure(), None);
        let open = board(
            "a",
            "v",
            vec![run(2, 1, 11.0, 1.0, "x"), run(0, 1, 11.0, 1.0, "x")],
        );
        assert_eq!(open.gate_failure().as_deref(), Some("2 unrouted"));
        let mut dirty = run(0, 1, 11.0, 1.0, "x");
        dirty.clearance_violations = Some(3);
        assert_eq!(
            board("a", "v", vec![dirty]).gate_failure().as_deref(),
            Some("3 clearance violations")
        );
        let mut slow = run(0, 1, 11.0, 61.0, "x");
        slow.hit_job_budget = true;
        assert_eq!(
            board("a", "v", vec![slow]).gate_failure().as_deref(),
            Some("stopped by the job budget")
        );
        let broken = board("a", "v", vec![run(0, 1, 11.0, 1.0, "x"), failed()]);
        assert!(broken.gate_failure().unwrap().starts_with("no SES written"));
        let mut unmeasured = run(0, 1, 11.0, 1.0, "x");
        unmeasured.quality = None;
        unmeasured.quality_error = Some("bad DSN".into());
        assert!(board("a", "v", vec![unmeasured])
            .gate_failure()
            .unwrap()
            .contains("not measured: bad DSN"));
    }

    #[test]
    fn violations_a_run_did_not_report_fail_the_gate() {
        let mut unknown = run(0, 1, 11.0, 1.0, "x");
        unknown.clearance_violations = None;
        let result = board("a", "v", vec![unknown.clone(), unknown]);
        assert_eq!(result.violations(), None);
        assert!(!result.passes_gate());
        assert_eq!(
            result.gate_failure().as_deref(),
            Some("clearance violations not reported")
        );
        // One run unreported is enough.
        let mut one = run(0, 1, 11.0, 1.0, "x");
        one.clearance_violations = None;
        assert!(!board("a", "v", vec![run(0, 1, 11.0, 1.0, "x"), one]).passes_gate());
    }

    #[test]
    fn a_label_shows_its_version_only_when_it_adds_something() {
        assert_eq!(display_name("2.5.0", "2.5.0"), "2.5.0");
        assert_eq!(display_name("2.5.0-2", "2.5.0"), "2.5.0-2 (2.5.0)");
        assert_eq!(display_name("unknown-2", "unknown"), "unknown-2");
        // A config's label carries the JAR's label after the `@`.
        assert_eq!(display_name("S0@2.5.0", "2.5.0"), "S0@2.5.0");
        assert_eq!(display_name("S0@2.5.0-2", "2.5.0"), "S0@2.5.0-2 (2.5.0)");
    }

    #[test]
    fn two_jars_of_one_version_stay_two_rows() {
        let mut rebuild = board(
            "b",
            "2.5.0",
            vec![run(0, 1, 11.0, 1.0, "x"), run(0, 1, 11.0, 1.0, "x")],
        );
        rebuild.dir = "2.5.0-2".into();
        rebuild.jar = PathBuf::from("/b/freerouting-2.5.0.jar");
        let release = board(
            "b",
            "2.5.0",
            vec![run(0, 1, 12.0, 1.0, "y"), run(0, 1, 12.0, 1.0, "y")],
        );
        let results = vec![release, rebuild];
        let rows = totals(&results);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|t| t.boards == 1 && t.version == "2.5.0"));
        let names: Vec<String> = rows.iter().map(VersionTotals::name).collect();
        assert_eq!(names, ["2.5.0", "2.5.0-2 (2.5.0)"]);
        let markdown = render_markdown(&results, &options(), None, None);
        assert!(markdown.contains("| 2.5.0-2 (2.5.0) |"), "{markdown}");
        let text = to_json(&results_json(&results, &options(), None, None));
        let (parsed, _) = parse_json_prefix(&text).unwrap();
        let Some(Json::Array(boards)) = parsed.at("boards") else {
            panic!("boards")
        };
        assert_eq!(boards[0].text_at("dir"), Some("2.5.0"));
        assert_eq!(boards[1].text_at("dir"), Some("2.5.0-2"));
    }

    #[test]
    fn determinism_needs_two_successful_runs_with_one_fingerprint() {
        let same = board(
            "a",
            "v",
            vec![run(0, 1, 1.0, 1.0, "x"), run(0, 1, 1.0, 3.0, "x")],
        );
        assert_eq!(same.deterministic(), Some(true));
        assert_eq!(same.median_runtime_s(), 2.0);
        let differ = board(
            "a",
            "v",
            vec![run(0, 1, 1.0, 1.0, "x"), run(1, 1, 1.0, 1.0, "y")],
        );
        assert_eq!(differ.deterministic(), Some(false));
        assert_eq!(differ.unrouted(), Some(1), "the worst run counts");
        assert_eq!(
            board("a", "v", vec![run(0, 1, 1.0, 1.0, "x")]).deterministic(),
            None
        );
        let broken = board("a", "v", vec![run(0, 1, 1.0, 1.0, "x"), failed()]);
        assert_eq!(broken.deterministic(), None);
        assert_eq!(broken.unrouted(), None);
    }

    #[test]
    fn wires_written_in_another_order_are_the_same_routing() {
        let mut second = run(0, 1, 1.0, 1.0, "x");
        second.ses_fingerprint = Some("reordered".into());
        let result = board("a", "v", vec![run(0, 1, 1.0, 1.0, "x"), second]);
        assert_eq!(result.deterministic(), Some(true));
        assert_eq!(result.same_ses(), Some(false));
    }

    #[test]
    fn a_run_stopped_by_the_job_budget_is_not_compared_for_determinism() {
        let mut first = run(12, 4, 50.0, 61.0, "p");
        first.hit_job_budget = true;
        let result = board("big", "v", vec![first, run(10, 4, 52.0, 61.0, "q")]);
        assert!(result.budget_limited());
        assert_eq!(result.deterministic(), None);
        assert!(!result.passes_gate());
    }

    #[test]
    fn quality_is_compared_only_on_boards_every_competing_version_passed() {
        let results = vec![
            board(
                "easy",
                "a",
                vec![run(0, 2, 12.0, 1.0, "x"), run(0, 2, 12.0, 1.0, "x")],
            ),
            board(
                "easy",
                "b",
                vec![run(0, 2, 15.0, 1.0, "y"), run(0, 2, 15.0, 1.0, "y")],
            ),
            // b fails "hard"; a's routing of it must not count against a.
            board(
                "hard",
                "a",
                vec![run(0, 9, 90.0, 9.0, "z"), run(0, 9, 90.0, 9.0, "z")],
            ),
            board(
                "hard",
                "b",
                vec![run(3, 1, 5.0, 1.0, "w"), run(3, 1, 5.0, 1.0, "w")],
            ),
            // c passes nothing: it competes on the gate alone.
            board("easy", "c", vec![failed(), failed()]),
            board("hard", "c", vec![failed(), failed()]),
        ];
        assert_eq!(compared_boards(&results), vec!["easy".to_string()]);
        let ranked = rank(totals(&results));
        let order: Vec<&str> = ranked.iter().map(|t| t.version.as_str()).collect();
        assert_eq!(order, ["a", "b", "c"]);
        let a = &ranked[0];
        assert_eq!((a.passed_boards, a.compared_boards, a.vias), (2, 1, 2));
        assert!((a.wire_ratio().unwrap() - 1.2).abs() < 1e-9);
        assert_eq!(ranked[1].gate_failures(), 1);
        assert_eq!((ranked[2].passed_boards, ranked[2].compared_boards), (0, 0));
        let text = describe(a);
        assert!(text.starts_with("a: passed the completion gate on 2 of 2 boards. On the 1 compared boards: wiring 1.200 x"), "{text}");
    }

    #[test]
    fn defects_outrank_wirelength_which_outranks_vias_then_determinism() {
        let with = |mm: f64, vias: usize, acute: usize, fp: &str| {
            let mut r = run(0, vias, mm, 1.0, fp);
            r.quality.as_mut().unwrap().acute_corners = acute;
            r
        };
        let results = vec![
            // shortest, but an acid trap
            board(
                "x",
                "trap",
                vec![with(10.0, 0, 1, "t"), with(10.0, 0, 1, "t")],
            ),
            // 0.4 % longer than "lean" (same 1 % step), more vias
            board(
                "x",
                "viaful",
                vec![with(11.04, 6, 0, "v"), with(11.04, 6, 0, "v")],
            ),
            board(
                "x",
                "lean",
                vec![with(11.0, 2, 0, "l"), with(11.0, 2, 0, "m")],
            ),
            // longer by a full step
            board(
                "x",
                "long",
                vec![with(13.0, 0, 0, "g"), with(13.0, 0, 0, "g")],
            ),
        ];
        let order: Vec<String> = rank(totals(&results))
            .into_iter()
            .map(|t| t.version)
            .collect();
        assert_eq!(order, ["lean", "viaful", "long", "trap"]);
    }

    #[test]
    fn ties_on_quality_go_to_the_repeatable_then_the_faster_version() {
        let results = vec![
            board(
                "a",
                "slow",
                vec![run(0, 2, 5.0, 9.0, "x"), run(0, 2, 5.0, 9.0, "x")],
            ),
            board(
                "a",
                "quick",
                vec![run(0, 2, 5.0, 2.0, "y"), run(0, 2, 5.0, 2.0, "y")],
            ),
            board(
                "a",
                "wobbly",
                vec![run(0, 2, 5.0, 1.0, "u"), run(0, 2, 5.0, 1.0, "w")],
            ),
        ];
        let order: Vec<String> = rank(totals(&results))
            .into_iter()
            .map(|t| t.version)
            .collect();
        assert_eq!(order, ["quick", "slow", "wobbly"]);
    }

    #[test]
    fn the_report_and_its_json_say_the_same_thing() {
        let results = vec![
            board(
                "demo|1",
                "2.5.0",
                vec![run(0, 2, 12.5, 3.0, "x"), run(0, 2, 12.5, 3.0, "x")],
            ),
            board("demo|1", "2.1.0", vec![failed(), failed()]),
        ];
        let markdown = render_markdown(&results, &options(), Some("25.0.5"), None);
        assert!(
            markdown.contains("| demo/1 | 40 | 2 | 2.5.0 | pass | 1.250 | 1.250 | 12.5 | 2 | 1.00 | 0 | 0 | 0 | 4 | 0 | 3.0 | yes |"),
            "{markdown}"
        );
        assert!(
            markdown
                .contains("| demo/1 | 40 | 2 | 2.1.0 | fail: no SES written (exit Some(1)) | - |"),
            "{markdown}"
        );
        assert!(markdown.contains("Recommended: 2.5.0"), "{markdown}");
        assert!(
            markdown.contains("compared on the 1 boards every competing version passed: demo|1"),
            "{markdown}"
        );
        assert!(!markdown.contains("-0.0"), "{markdown}");

        let text = to_json(&results_json(&results, &options(), Some("25.0.5"), None));
        let (parsed, used) = parse_json_prefix(&text).expect("valid JSON");
        assert_eq!(text[used..].trim(), "");
        assert_eq!(parsed.text_at("recommended"), Some("2.5.0"));
        assert_eq!(parsed.number_at("job_timeout_s"), Some(60.0));
        let Some(Json::Array(boards)) = parsed.at("boards") else {
            panic!("boards")
        };
        assert_eq!(boards[0].text_at("board"), Some("demo|1"));
        assert_eq!(boards[0].at("passes_gate"), Some(&Json::Bool(true)));
        assert_eq!(boards[1].at("passes_gate"), Some(&Json::Bool(false)));
        let Some(Json::Array(runs)) = boards[0].at("runs") else {
            panic!("runs")
        };
        assert_eq!(runs[0].number_at("quality.wire_ratio"), Some(1.25));
        assert_eq!(runs[0].number_at("quality.vias"), Some(2.0));
        assert!(statistics_block(&format!("log line\n{text}")).is_some());
    }

    fn candidate(name: &str) -> Candidate {
        Candidate {
            name: name.into(),
            runs: 2,
            scored: 2,
            failed_runs: 0,
            incomplete: false,
            budget_limited: false,
            failed_gates: [false; 3],
            acute: Some(0.0),
            signal_ratio: Some(1.05),
            power_ratio: Some(1.05),
            total_ratio: Some(1.05),
            pour_track_mm: Some(80.0),
            right_angles: Some(10.0),
            vias: Some(100),
            copper_quality: Some(60.0),
            verdict: Verdict::Below,
            runtime_s: 100.0,
        }
    }

    fn order(candidates: Vec<Candidate>) -> Vec<String> {
        rank_candidates(candidates)
            .into_iter()
            .map(|c| c.name)
            .collect()
    }

    #[test]
    fn candidates_rank_by_the_gates_then_the_owners_order_of_measures() {
        let with = |name: &str, edit: &dyn Fn(&mut Candidate)| {
            let mut c = candidate(name);
            edit(&mut c);
            c
        };
        // Each line: the first beats the second on one measure although the
        // second is better on every measure after it.
        let better_later = |c: &mut Candidate| {
            c.signal_ratio = Some(1.0);
            c.power_ratio = Some(1.0);
            c.right_angles = Some(0.0);
            c.vias = Some(1);
            c.copper_quality = Some(99.0);
            c.runtime_s = 1.0;
        };
        // A failed gate loses to any pass, whatever the quality.
        let failed = with("fails G3", &|c| {
            better_later(c);
            c.acute = Some(0.0);
            c.failed_gates = [false, false, true];
        });
        assert_eq!(
            order(vec![failed.clone(), candidate("passes")]),
            ["passes", "fails G3"]
        );
        // Completion is worse to fail than DRC, and DRC worse than width.
        let g1 = with("fails G1", &|c| c.failed_gates = [true, false, false]);
        let g2 = with("fails G2", &|c| c.failed_gates = [false, true, false]);
        assert_eq!(
            order(vec![g1.clone(), failed.clone(), g2.clone()]),
            ["fails G3", "fails G2", "fails G1"]
        );
        // A run that failed or was not scored is last of all.
        let unscored = with("unscored", &|c| {
            better_later(c);
            c.incomplete = true;
        });
        assert_eq!(order(vec![unscored, g1.clone()]), ["fails G1", "unscored"]);
        // Then acute corners.
        let acute = with("one acute", &|c| {
            better_later(c);
            c.acute = Some(1.0);
        });
        assert_eq!(order(vec![acute, candidate("none")]), ["none", "one acute"]);
        // Then signal length, in steps of 1 %.
        let longer = with("signals 1.07", &|c| {
            c.signal_ratio = Some(1.07);
            c.power_ratio = Some(1.0);
            c.right_angles = Some(0.0);
        });
        assert_eq!(
            order(vec![longer, candidate("signals 1.05")]),
            ["signals 1.05", "signals 1.07"]
        );
        // Less than a step is no difference: the next measure decides.
        let a_hair = with("signals 1.054", &|c| {
            c.signal_ratio = Some(1.054);
            c.power_ratio = Some(1.02);
        });
        assert_eq!(
            order(vec![candidate("signals 1.05"), a_hair]),
            ["signals 1.054", "signals 1.05"]
        );
        // Then power length, right angles, vias, copper quality, runtime.
        for (name, worse) in [
            (
                "power",
                &(|c: &mut Candidate| {
                    c.power_ratio = Some(1.08);
                    c.right_angles = Some(0.0);
                }) as &dyn Fn(&mut Candidate),
            ),
            ("right angles", &|c| {
                c.right_angles = Some(11.0);
                c.vias = Some(1);
            }),
            ("vias", &|c| {
                c.vias = Some(101);
                c.copper_quality = Some(99.0);
            }),
            ("quality", &|c| {
                c.copper_quality = Some(58.0);
                c.runtime_s = 1.0;
            }),
            ("runtime", &|c| c.runtime_s = 101.0),
        ] {
            assert_eq!(
                order(vec![with(name, worse), candidate("reference")]),
                ["reference", name],
                "{name}"
            );
        }
        // A measure that is missing is never the better one.
        let unmeasured = with("no signal ratio", &|c| c.signal_ratio = None);
        assert_eq!(
            order(vec![unmeasured, candidate("measured")]),
            ["measured", "no signal ratio"]
        );
    }

    fn scored(signal_mm: f64, acute: f64, quality: f64) -> BoardScore {
        use super::super::board_score::Audit;
        BoardScore {
            placement: "same".into(),
            pour_nets: ["GND".to_string()].into(),
            segments: 3,
            length_mm: [8.0, 20.0, signal_mm],
            vias: [2, 1, 2],
            net_length_mm: [("GND", 8.0), ("+5V", 20.0), ("SIG", signal_mm)]
                .map(|(net, mm)| (net.to_string(), mm))
                .into(),
            audit: Audit {
                copper_quality: Some(quality),
                score: Some(quality),
                routed_share: Some(1.0),
                complete: Some(true),
                categories: [("routing".to_string(), quality)].into(),
                metrics: [
                    ("5.3.acute_corners", acute),
                    ("5.3.right_angle_corners", 3.0),
                    ("4.2.nets_undersized", 0.0),
                    ("4.3.narrow_paths", 0.0),
                    ("3.3.floating_ground_islands", 0.0),
                ]
                .map(|(key, value)| (key.to_string(), value))
                .into(),
            },
            ..BoardScore::default()
        }
    }

    fn judged(context: &ScoreContext, score: BoardScore) -> RunRecord {
        let mut record = run(0, 2, 11.0, 30.0, "x");
        record.judge(score, context);
        record
    }

    #[test]
    fn a_candidate_is_only_as_good_as_its_worst_run() {
        let context = ScoreContext::for_tests(scored(30.0, 0.0, 60.0), &["GND", "+5V"]);
        let mut entry = board(
            "V1",
            "S2@2.5.0",
            vec![
                judged(&context, scored(30.0, 0.0, 62.0)),
                judged(&context, scored(33.0, 2.0, 59.0)),
            ],
        );
        entry.config = Some("S2".into());
        let c = &candidates(std::slice::from_ref(&entry))[0];
        assert_eq!(c.name, "S2@2.5.0");
        assert_eq!((c.runs, c.scored, c.incomplete), (2, 2, false));
        assert_eq!(c.failed_gates, [false; 3]);
        // The longer run, the run with more acute corners, the lower score.
        assert!((c.signal_ratio.unwrap() - 1.1).abs() < 1e-9);
        assert_eq!(c.acute, Some(2.0));
        assert_eq!(c.copper_quality, Some(59.0));
        assert_eq!(c.verdict, Verdict::Below);
        assert!(c.passes());
        // One run failing a gate fails the candidate.
        let mut open = scored(30.0, 0.0, 60.0);
        open.drc.unconnected = 1;
        entry.runs[0] = judged(&context, open);
        let c = &candidates(std::slice::from_ref(&entry))[0];
        assert_eq!(c.failed_gates, [true, false, false]);
        assert!(!c.passes());
        // A run that could not be scored leaves the candidate incomplete.
        entry.runs[0] = run(0, 2, 11.0, 30.0, "x");
        entry.runs[0].score_error = Some("kicad-cli refill and DRC: exit Some(1)".into());
        let c = &candidates(std::slice::from_ref(&entry))[0];
        assert_eq!((c.scored, c.incomplete), (1, true));
        assert!(!c.passes());
        // So does a run whose settings were not in effect, scored or not.
        entry.runs[0] = judged(&context, scored(30.0, 0.0, 60.0));
        entry.runs[0].error = Some("settings not in effect: x".into());
        assert!(candidates(std::slice::from_ref(&entry))[0].incomplete);
        // A run the job budget stopped is said so, and is not a gate.
        entry.runs[0].error = None;
        entry.runs[1].hit_job_budget = true;
        let c = &candidates(std::slice::from_ref(&entry))[0];
        assert!(c.budget_limited && c.passes());
        assert!(describe_candidate(c).ends_with("(a run was stopped by the job budget)"));
        // Entries of a bench that scored nothing are not candidates.
        assert!(candidates(&[board("V1", "v", vec![run(0, 1, 11.0, 1.0, "x")])]).is_empty());
    }

    #[test]
    fn the_board_score_is_in_the_report_and_in_its_json() {
        let context = ScoreContext::for_tests(scored(30.0, 0.0, 60.0), &["GND", "+5V"]);
        let mut good = board(
            "V1",
            "S2@2.5.0",
            vec![
                judged(&context, scored(30.3, 0.0, 61.0)),
                judged(&context, scored(30.6, 0.0, 61.0)),
            ],
        );
        good.config = Some("S2".into());
        good.config_args = vec!["--router.fanout.enabled=false".into()];
        good.runs[0].settings = Some(SettingsEvidence {
            unconfirmed: vec!["router.scoring.via_costs=25".into()],
            ..SettingsEvidence::default()
        });
        let mut open = scored(29.0, 3.0, 61.0);
        open.drc.unconnected = 4;
        let bad = board("V0", "S0@2.5.0", vec![judged(&context, open)]);
        let results = vec![bad, good];
        let markdown = render_markdown(&results, &options(), None, Some(&context));
        assert!(markdown.contains("## Board score"), "{markdown}");
        assert!(
            markdown.contains("Intent currents: test values."),
            "{markdown}"
        );
        assert!(markdown.contains("| baseline (existing routing) | - | - | - | 1.000 | 1.000 | 1.000 | 8.0 | 2 / 3 | 0 | 3 | 60.0 | 0 | - |"), "{markdown}");
        assert!(
            markdown.contains("| S2@2.5.0 on V1, run 2 | pass | pass | pass | 1.020 |"),
            "{markdown}"
        );
        assert!(
            markdown.contains("| S0@2.5.0 on V0, run 1 | FAIL | pass | pass | 0.967 |"),
            "{markdown}"
        );
        assert!(
            markdown.contains("\n1. S2@2.5.0 on V1: passes every gate in every run;"),
            "{markdown}"
        );
        assert!(
            markdown.contains("\n2. S0@2.5.0 on V0: G1 completion failed;"),
            "{markdown}"
        );
        assert!(markdown.contains("\nChosen: S2@2.5.0 on V1 "), "{markdown}");
        assert!(
            markdown.contains("- S0@2.5.0 on V0, run 1: G1 completion: 4 unconnected (KiCad)"),
            "{markdown}"
        );
        assert!(
            markdown.contains("- S0@2.5.0 on V0, run 1: acute corners 3.000 (baseline 0.000)"),
            "{markdown}"
        );
        assert!(
            markdown.contains("- S2@2.5.0 on V1, run 1: router.scoring.via_costs=25"),
            "{markdown}"
        );
        // Without a context there is no such section.
        assert!(!render_markdown(&results, &options(), None, None).contains("Board score"));

        let text = to_json(&results_json(&results, &options(), None, Some(&context)));
        let (json, _) = parse_json_prefix(&text).unwrap();
        assert_eq!(json.text_at("score_spec"), Some("/work/spec.json"));
        assert_eq!(json.text_at("score.currents"), Some("test values"));
        assert_eq!(json.text_at("score.chosen"), Some("S2@2.5.0 on V1"));
        assert_eq!(json.number_at("score.thresholds.signal_match"), Some(1.03));
        assert_eq!(
            json.number_at("score.baseline.length_mm.signal"),
            Some(30.0)
        );
        let Some(Json::Array(boards)) = json.at("boards") else {
            panic!("boards in {text}")
        };
        assert_eq!(boards[1].text_at("config"), Some("S2"));
        let Some(Json::Array(runs)) = boards[1].at("runs") else {
            panic!("runs in {text}")
        };
        assert_eq!(runs[1].number_at("board_score.total_mm"), Some(58.6));
        assert!((runs[1].number_at("compared.signal_ratio").unwrap() - 1.02).abs() < 1e-9);
        assert_eq!(
            runs[1].text_at("compared.against_baseline"),
            Some("matches")
        );
        assert!(
            matches!(runs[0].at("settings.unconfirmed"), Some(Json::Array(list)) if list.len() == 1)
        );
        // No candidate passing means none is chosen.
        let only_bad = &results[..1];
        let markdown = render_markdown(only_bad, &options(), None, Some(&context));
        assert!(
            markdown.contains("No candidate passes every gate in every run. Nearest: S0@2.5.0."),
            "{markdown}"
        );
        let text = to_json(&results_json(only_bad, &options(), None, Some(&context)));
        let (json, _) = parse_json_prefix(&text).unwrap();
        assert_eq!(json.at("score.chosen"), Some(&Json::Null));
    }

    #[test]
    fn json_text_is_escaped() {
        let value = Json::Object(vec![("k\"".into(), Json::Text("a\\b\n\u{1}µ".into()))]);
        let text = to_json(&value);
        assert!(text.contains(r#""k\"": "a\\b\n\u0001µ""#), "{text}");
        assert_eq!(parse_json_prefix(&text).unwrap().0, value);
        assert_eq!(to_json(&Json::Number(f64::NAN)).trim(), "null");
        assert_eq!(to_json(&Json::Number(2.5)).trim(), "2.5");
        assert_eq!(to_json(&Json::Number(-3.0)).trim(), "-3");
    }
}
