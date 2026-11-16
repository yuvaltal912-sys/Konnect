//! Best-of-N routing: effort levels and budgets, the DSN variants a search tries,
//! the lanes it runs on, and the timed search itself (docs/ROUTE_SEARCH_DESIGN.md,
//! decisions 1 to 9).
//!
//! The scheduler knows nothing of fastroute or KiCad: it drives a [`Runner`], so
//! tests run it with fakes on tokio's paused clock.

use crate::route_judge::{Baseline, Judged, Key};
use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::{Id, JoinError, JoinSet};
use tokio::time::Instant;

/// A boxed, sendable future, as the runner's jobs return.
pub(crate) type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// The effort level the user picks (owner, 2026-11-16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Effort {
    Fast,
    M,
    H,
    Xh,
}

/// What an effort level allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// The per-board time budget; `None` for Fast.
    pub budget: Option<Duration>,
    /// A fixed number of runs, all launched at once; `Some(2)` for Fast only.
    pub runs: Option<usize>,
}

impl Effort {
    /// `fast`, `m`, `h` or `xh`, exactly (lower case); anything else is `None`.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text {
            "fast" => Some(Self::Fast),
            "m" => Some(Self::M),
            "h" => Some(Self::H),
            "xh" => Some(Self::Xh),
            _ => None,
        }
    }

    /// The name `parse` reads.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::M => "m",
            Self::H => "h",
            Self::Xh => "xh",
        }
    }

    /// The owner's budgets (2026-11-16, "double the first proposal", runs in
    /// parallel): Fast = 2 runs at once and no budget; M = 240 s; H = 720 s;
    /// XH = 2400 s, per board.
    pub(crate) fn limits(self) -> Limits {
        let minutes = |m: u64| Some(Duration::from_secs(m * 60));
        match self {
            Self::Fast => Limits {
                budget: None,
                runs: Some(2),
            },
            Self::M => Limits {
                budget: minutes(4),
                runs: None,
            },
            Self::H => Limits {
                budget: minutes(12),
                runs: None,
            },
            Self::Xh => Limits {
                budget: minutes(40),
                runs: None,
            },
        }
    }
}

/// Where the effort level came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EffortSource {
    Argument,
    UserConfig,
    Default,
}

/// The effort level of a call: the `effort` argument when present, else the user
/// config's `routing.effort` (`config` is that value when the config has it), else
/// M. A value that is not a string naming one of the four levels is refused, from
/// either place; the message names where it came from and the four names.
pub(crate) fn resolve_effort(
    argument: Option<&Value>,
    config: Option<&Value>,
) -> std::result::Result<(Effort, EffortSource), String> {
    fn read(value: &Value, source: &str) -> std::result::Result<Effort, String> {
        let Some(text) = value.as_str() else {
            return Err(format!(
                "{source} must be a string, one of fast, m, h or xh; it was {value}"
            ));
        };
        Effort::parse(text)
            .ok_or_else(|| format!("{source} must be one of fast, m, h or xh; it was {text:?}"))
    }
    // A JSON null is "not given", from either place.
    let argument = argument.filter(|v| !v.is_null());
    let config = config.filter(|v| !v.is_null());
    if let Some(value) = argument {
        return read(value, "the `effort` argument").map(|e| (e, EffortSource::Argument));
    }
    if let Some(value) = config {
        return read(value, "the user config's `routing.effort`")
            .map(|e| (e, EffortSource::UserConfig));
    }
    Ok((Effort::M, EffortSource::Default))
}

/// One way of giving the board to the router (decision 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Shape {
    /// The DSN as exported, its planes included.
    AsExported,
    /// Every `(plane …)` of the DSN's structure removed.
    Omitted,
}

/// The shapes this DSN offers: `AsExported` always, then `Omitted` when the DSN's
/// structure holds at least one `(plane …)`.
pub(crate) fn shapes(dsn_text: &str) -> Result<Vec<Shape>> {
    let structure = structure_block(dsn_text)?;
    let has_plane = child_blocks(dsn_text, structure)?
        .into_iter()
        .any(|c| has_head(dsn_text, c, "plane"));
    Ok(if has_plane {
        vec![Shape::AsExported, Shape::Omitted]
    } else {
        vec![Shape::AsExported]
    })
}

/// The DSN text of one variant: for `Omitted`, [`strip_planes`]; for a seed of 1 or
/// more, [`reorder_placement`] (seed 0 keeps the order); and always
/// [`stub_rule_off`]. Every byte outside those edits is kept.
pub(crate) fn variant_text(dsn_text: &str, shape: Shape, seed: u64) -> Result<String> {
    let shaped = match shape {
        Shape::AsExported => dsn_text.to_string(),
        Shape::Omitted => strip_planes(dsn_text)?.0,
    };
    let ordered = if seed >= 1 {
        reorder_placement(&shaped, seed)?
    } else {
        shaped
    };
    stub_rule_off(&ordered)
}

/// Removes every direct `(plane …)` child of the top-level `(structure …)` block,
/// with the whitespace that precedes it on its line; returns the text and how many
/// it removed. Block scanning is quote-aware (a `"` opens and closes a string; a
/// parenthesis inside one is text). A DSN with no structure block is refused.
pub(crate) fn strip_planes(text: &str) -> Result<(String, usize)> {
    let structure = structure_block(text)?;
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut kept_from = 0;
    let mut removed = 0;
    for plane in child_blocks(text, structure)?
        .into_iter()
        .filter(|c| has_head(text, *c, "plane"))
    {
        let mut from = plane.start;
        while from > kept_from && matches!(bytes[from - 1], b' ' | b'\t') {
            from -= 1;
        }
        out.push_str(&text[kept_from..from]);
        kept_from = plane.end;
        removed += 1;
    }
    out.push_str(&text[kept_from..]);
    Ok((out, removed))
}

/// Reorders the `(component …)` children of the top-level `(placement …)` block
/// with a Fisher-Yates shuffle driven by [`SplitMix64`] seeded with `seed`. The
/// gaps between blocks (whitespace, newlines) stay where they were; only the blocks
/// move, each byte for byte. Seed 0 returns the text unchanged. A placement child
/// that is not a `component` is refused; so is a DSN with no placement block.
pub(crate) fn reorder_placement(text: &str, seed: u64) -> Result<String> {
    if seed == 0 {
        return Ok(text.to_string());
    }
    let root = root_block(text)?;
    let placement = child_blocks(text, root)?
        .into_iter()
        .find(|c| has_head(text, *c, "placement"))
        .ok_or_else(|| anyhow!("the DSN has no top-level (placement ...) block"))?;
    let blocks = child_blocks(text, placement)?;
    for block in &blocks {
        if !has_head(text, *block, "component") {
            bail!(
                "the DSN's placement holds a ({} ...) block that is not a component; \
                 it cannot be reordered safely",
                head_of(text, *block)
            );
        }
    }
    if blocks.len() < 2 {
        return Ok(text.to_string());
    }
    // Fisher-Yates over the block indices: position `k` gets block `order[k]`.
    let mut order: Vec<usize> = (0..blocks.len()).collect();
    let mut rng = SplitMix64(seed);
    for i in (1..order.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        order.swap(i, j);
    }
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..blocks[0].start]);
    for (pos, &source) in order.iter().enumerate() {
        let block = blocks[source];
        out.push_str(&text[block.start..block.end]);
        if pos + 1 < blocks.len() {
            // The gap that followed the block that stood here stays here.
            out.push_str(&text[blocks[pos].end..blocks[pos + 1].start]);
        }
    }
    out.push_str(&text[blocks[blocks.len() - 1].end..]);
    Ok(out)
}

/// Turns the pad-exit stub rule off (research CR1) in the structure's own top-level
/// `(rule …)` (the direct child of `(structure …)`, not a class rule): an existing
/// `(clearance <n> (type smd_to_turn_gap))` inside it gets 0 as its value; else
/// `(clearance 0 (type smd_to_turn_gap))` is added as the rule's last child, on its
/// own line, indented as the rule's other children are. A structure with no rule is
/// refused.
pub(crate) fn stub_rule_off(text: &str) -> Result<String> {
    let structure = structure_block(text)?;
    let rules: Vec<Block> = child_blocks(text, structure)?
        .into_iter()
        .filter(|c| has_head(text, *c, "rule"))
        .collect();
    let Some(first_rule) = rules.first().copied() else {
        bail!(
            "the DSN's structure has no (rule ...) block, so the pad-exit stub rule \
             cannot be turned off"
        );
    };

    // An existing `(clearance <n> (type smd_to_turn_gap))`: its value becomes 0.
    let mut values: Vec<(usize, usize)> = Vec::new();
    for rule in &rules {
        for clearance in child_blocks(text, *rule)?
            .into_iter()
            .filter(|c| has_head(text, *c, "clearance"))
        {
            let parts = items(text, clearance)?;
            let mut is_stub = false;
            for part in parts.iter().filter(|p| p.list) {
                let typed = Block {
                    start: part.start,
                    end: part.end,
                };
                if has_head(text, typed, "type") {
                    let inner = items(text, typed)?;
                    is_stub |= inner
                        .get(1)
                        .is_some_and(|a| !a.list && &text[a.start..a.end] == STUB_TYPE);
                }
            }
            if is_stub {
                if let Some(value) = parts.get(1).filter(|v| !v.list) {
                    values.push((value.start, value.end));
                }
            }
        }
    }
    if !values.is_empty() {
        values.sort_unstable();
        let mut out = String::with_capacity(text.len());
        let mut kept_from = 0;
        for (start, end) in values {
            out.push_str(&text[kept_from..start]);
            out.push('0');
            kept_from = end;
        }
        out.push_str(&text[kept_from..]);
        return Ok(out);
    }

    // Else add it as the first rule's last child.
    let parts = items(text, first_rule)?;
    let last = parts
        .last()
        .copied()
        .ok_or_else(|| anyhow!("the structure's (rule ...) block is empty"))?;
    let line_start = text[..last.start].rfind('\n').map_or(0, |p| p + 1);
    let indent = &text[line_start..last.start];
    let own_line = last.list && indent.bytes().all(|c| c == b' ' || c == b'\t');
    let added = if own_line {
        format!("\n{indent}{STUB_RULE_OFF}")
    } else {
        format!(" {STUB_RULE_OFF}")
    };
    let mut out = String::with_capacity(text.len() + added.len());
    out.push_str(&text[..last.end]);
    out.push_str(&added);
    out.push_str(&text[last.end..]);
    Ok(out)
}

/// The text the stub rule gets (research CR1), and the clearance type it names.
const STUB_TYPE: &str = "smd_to_turn_gap";
const STUB_RULE_OFF: &str = "(clearance 0 (type smd_to_turn_gap))";

/// A parenthesised block of the DSN text: `start` at its `(`, `end` after its `)`.
#[derive(Debug, Clone, Copy)]
struct Block {
    start: usize,
    end: usize,
}

/// One direct child of a block: an atom (a bare word or a quoted string) or a list.
#[derive(Debug, Clone, Copy)]
struct Item {
    start: usize,
    end: usize,
    list: bool,
}

fn skip_whitespace(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// The end of the bare word that starts at `i`.
fn atom_end(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'(' | b')' | b'"') {
        i += 1;
    }
    i
}

/// The end of the quoted string whose opening `"` is at `open`.
fn string_end(b: &[u8], open: usize) -> Result<usize> {
    match b[open + 1..].iter().position(|&c| c == b'"') {
        Some(p) => Ok(open + 1 + p + 1),
        None => bail!("the DSN has a quoted string that is never closed (byte {open})"),
    }
}

/// Where scanning continues inside the list that opens at `open`. Normally right
/// after the `(`; for `(string_quote ")` it is after the quote character, which is
/// that block's content and not the start of a string.
fn enter(b: &[u8], open: usize) -> usize {
    const HEAD: &[u8] = b"string_quote";
    let inner = open + 1;
    if b[inner..].starts_with(HEAD) {
        let after = inner + HEAD.len();
        if after < b.len() && b[after].is_ascii_whitespace() {
            let quote = skip_whitespace(b, after);
            if quote < b.len() && matches!(b[quote], b'"' | b'\'') {
                return quote + 1;
            }
        }
    }
    inner
}

/// The index after the `)` that closes the list opening at `open`.
fn list_end(b: &[u8], open: usize) -> Result<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'(' => {
                depth += 1;
                i = enter(b, i);
            }
            b')' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
            b'"' => i = string_end(b, i)?,
            _ => i += 1,
        }
    }
    bail!("the DSN has a parenthesis that is never closed (byte {open})")
}

/// The direct children of `block`, in order.
fn items(text: &str, block: Block) -> Result<Vec<Item>> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = enter(b, block.start);
    loop {
        i = skip_whitespace(b, i);
        if i >= block.end.saturating_sub(1) {
            return Ok(out);
        }
        match b[i] {
            b')' => return Ok(out),
            b'(' => {
                let end = list_end(b, i)?;
                out.push(Item {
                    start: i,
                    end,
                    list: true,
                });
                i = end;
            }
            b'"' => {
                let end = string_end(b, i)?;
                out.push(Item {
                    start: i,
                    end,
                    list: false,
                });
                i = end;
            }
            _ => {
                let end = atom_end(b, i);
                out.push(Item {
                    start: i,
                    end,
                    list: false,
                });
                i = end;
            }
        }
    }
}

/// The nested lists directly inside `block`.
fn child_blocks(text: &str, block: Block) -> Result<Vec<Block>> {
    Ok(items(text, block)?
        .into_iter()
        .filter(|i| i.list)
        .map(|i| Block {
            start: i.start,
            end: i.end,
        })
        .collect())
}

/// The first word of a block (`plane` for `(plane GND ...)`), or "".
fn head_of(text: &str, block: Block) -> &str {
    let b = text.as_bytes();
    let i = skip_whitespace(b, block.start + 1);
    if i < block.end.saturating_sub(1) && !matches!(b[i], b'(' | b')' | b'"') {
        &text[i..atom_end(b, i)]
    } else {
        ""
    }
}

fn has_head(text: &str, block: Block, name: &str) -> bool {
    head_of(text, block).eq_ignore_ascii_case(name)
}

/// The DSN's root block, `(pcb ...)`.
fn root_block(text: &str) -> Result<Block> {
    let b = text.as_bytes();
    let start = skip_whitespace(b, 0);
    if start >= b.len() || b[start] != b'(' {
        bail!("the DSN text does not start with a ( block");
    }
    Ok(Block {
        start,
        end: list_end(b, start)?,
    })
}

/// The `(structure ...)` block that is a direct child of the root.
fn structure_block(text: &str) -> Result<Block> {
    let root = root_block(text)?;
    child_blocks(text, root)?
        .into_iter()
        .find(|c| has_head(text, *c, "structure"))
        .ok_or_else(|| anyhow!("the DSN has no top-level (structure ...) block"))
}

/// SplitMix64 (Steele, Lea and Flood, 2014): a small, fixed, portable generator, so
/// that a seed names the same order on every machine and in every build.
#[derive(Debug, Clone)]
pub(crate) struct SplitMix64(pub u64);

impl SplitMix64 {
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..bound` (`bound` > 0), by Lemire's multiply-shift with
    /// rejection, so there is no modulo bias.
    pub(crate) fn below(&mut self, bound: u64) -> u64 {
        debug_assert!(bound > 0, "SplitMix64::below needs a bound above 0");
        if bound == 0 {
            return 0;
        }
        let mut product = u128::from(self.next_u64()) * u128::from(bound);
        let mut low = product as u64;
        if low < bound {
            // The few low values that would make the result slightly more likely.
            let threshold = bound.wrapping_neg() % bound;
            while low < threshold {
                product = u128::from(self.next_u64()) * u128::from(bound);
                low = product as u64;
            }
        }
        (product >> 64) as u64
    }
}

/// The most runs one call starts (decision 9).
pub(crate) const MAX_RUNS: usize = 240;

/// The longest one run (every pass of it) may take before it is killed (decision 8).
pub(crate) const HANG_CAP: Duration = Duration::from_secs(900);

/// The order runs are launched in (decision 3): seed 1 in every shape (in the order
/// given), then seed 2 in every shape, and so on; `limits.runs` entries when set,
/// else [`MAX_RUNS`].
pub(crate) fn launch_order(shapes: &[Shape], limits: Limits) -> Vec<(Shape, u64)> {
    let wanted = limits.runs.unwrap_or(MAX_RUNS);
    let mut order = Vec::with_capacity(wanted.min(MAX_RUNS));
    if shapes.is_empty() {
        return order;
    }
    let mut seed = 1u64;
    while order.len() < wanted {
        for shape in shapes {
            if order.len() == wanted {
                break;
            }
            order.push((*shape, seed));
        }
        seed += 1;
    }
    order
}

/// How many runs route at once, and how many are judged at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct Lanes {
    pub routers: usize,
    pub judges: usize,
}

/// Decision 5: routers = clamp(cores / 2, 1, 7), judges = clamp(cores / 4, 1, 3).
/// For Fast, routers is its run count (2), whatever the cores.
pub(crate) fn lanes(cores: usize, effort: Effort) -> Lanes {
    let routers = match effort {
        Effort::Fast => 2,
        _ => (cores / 2).clamp(1, 7),
    };
    Lanes {
        routers,
        judges: (cores / 4).clamp(1, 3),
    }
}

/// What a router lane hands to a judge lane.
#[derive(Debug, Clone)]
pub(crate) struct Routed {
    pub index: usize,
    pub shape: Shape,
    pub seed: u64,
    /// The variant's DSN, in the run's folder.
    pub dsn: PathBuf,
    pub held: crate::fastroute::Held,
    /// The nets the variant gave the router as planes (none for `Omitted`).
    pub planes: Vec<String>,
    /// The run's own folder.
    pub folder: PathBuf,
}

/// Why a run gave no session.
#[derive(Debug, Clone)]
pub(crate) enum RouteFailure {
    /// Killed at its deadline (the hang cap, or the budget's cut).
    Killed,
    Failed(String),
}

/// The jobs a search runs. The production runner routes with fastroute and judges
/// with `route_judge`; tests use fakes.
pub(crate) trait Runner: Send + Sync + 'static {
    /// Route one variant; the run must end, killed, by `deadline`.
    fn route(
        &self,
        index: usize,
        shape: Shape,
        seed: u64,
        deadline: tokio::time::Instant,
    ) -> BoxFuture<std::result::Result<Routed, RouteFailure>>;

    /// Judge one routed run against the unrouted board's baseline.
    fn judge(&self, routed: Routed, baseline: Arc<Baseline>) -> BoxFuture<Result<Judged>>;

    /// The unrouted board's DRC, G2's baseline. Started at t0, beside the first
    /// wave; every judge waits for it.
    fn baseline(&self) -> BoxFuture<Result<Baseline>>;
}

/// One run of a search, as the result reports it.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunRecord {
    pub index: usize,
    pub shape: Shape,
    pub seed: u64,
    /// Seconds from t0.
    pub launched_s: f64,
    pub routed_s: Option<f64>,
    pub judged_s: Option<f64>,
    /// How long the route and the judge took.
    pub route_s: Option<f64>,
    pub judge_s: Option<f64>,
    /// `judged`, `route_failed`, `hung`, `judge_failed`, `cut_at_budget`, or
    /// `running` (only inside the scheduler; never in a finished outcome).
    pub outcome: &'static str,
    pub reason: Option<String>,
    #[serde(skip)]
    pub routed: Option<Routed>,
    #[serde(skip)]
    pub judged: Option<Judged>,
}

/// A search to run.
#[derive(Debug, Clone)]
pub(crate) struct SearchPlan {
    pub limits: Limits,
    pub lanes: Lanes,
    pub order: Vec<(Shape, u64)>,
    pub hang_cap: Duration,
}

/// How a search ended.
#[derive(Debug)]
pub(crate) struct SearchOutcome {
    /// Every launched run, in launch order (`index` is its position).
    pub runs: Vec<RunRecord>,
    /// The index of the winning run, if any run was judged.
    pub winner: Option<usize>,
    /// Each judged run's rank key, by index (`None` for a run not judged).
    pub keys: Vec<Option<Key>>,
    pub elapsed: Duration,
    /// When no run was judged by the budget: how far past it the first was (D7).
    pub budget_overrun: Option<Duration>,
    /// Why the last launch was the last: `order exhausted`, `the budget`,
    /// `the launch rule`, `fixed run count`, or the MAX_RUNS cap.
    pub stopped_launching: Option<String>,
    pub baseline: std::result::Result<Baseline, String>,
}

/// The timed search (decisions 6 to 9):
/// - t0 = now. The baseline job starts at once. The first wave launches
///   `min(lanes.routers, order.len())` runs at once (for Fast, its fixed count).
/// - Each route gets `deadline = min(launch + hang_cap, the budget's cut instant)`;
///   the cut instant is `None` until the cut is decided (see below), so routes are
///   bounded by the hang cap until then. A route that returns `Killed` is `hung`
///   when its hang cap was reached, `cut_at_budget` when the cut was.
/// - A routed run goes to a judge lane at once, or waits in FIFO order. No judge
///   starts before the baseline is done; if the baseline failed, every judge
///   records `judge_failed` with the baseline's reason.
/// - After every event (a route or judge finishing, or the budget instant), while a
///   router lane is free and the order has runs left: launch the next run when
///   [`may_launch`] allows (never after the first wave for Fast, never at or after
///   the budget). Medians are over finished routes (routed or failed, not killed)
///   and finished judges.
/// - At the budget, if at least one run is judged: every running route and judge
///   is dropped (their processes die with them) and recorded `cut_at_budget`; the
///   search ends. If none is judged: no new launch, and the search waits for the
///   first judged run, then cuts everything else; `budget_overrun` is how far past
///   the budget that was.
/// - Without a budget (Fast) the search ends when every run is routed and judged.
/// - The search also ends when nothing runs, nothing waits for a judge and nothing
///   more may launch.
/// - The winner is the least [`crate::route_judge::rank`] key over the judged runs,
///   ties to the lower index.
pub(crate) async fn search(plan: &SearchPlan, runner: Arc<dyn Runner>) -> SearchOutcome {
    let t0 = Instant::now();
    let budget_at = plan.limits.budget.map(|b| t0 + b);
    let mut sched = Sched::new(plan, runner, t0);
    let baseline_job = sched.runner.baseline();
    sched.base.spawn(baseline_job);

    let ended;
    loop {
        let now = Instant::now();
        let passed = budget_at.is_some_and(|b| now >= b);
        if passed {
            sched.budget_hit = true;
            if sched.any_judged() {
                ended = now;
                sched.cut();
                sched.stop_tasks().await;
                break;
            }
        }
        sched.start_judges(now);
        sched.launch_ready(now, passed);
        if sched.is_idle() {
            ended = now;
            sched.stop_tasks().await;
            break;
        }
        let wake = budget_at.filter(|_| !passed);
        let Some(event) = sched.next_event(wake).await else {
            ended = now;
            sched.stop_tasks().await;
            break;
        };
        let at = Instant::now();
        match event {
            Event::Baseline(result) => sched.on_baseline(result),
            Event::Judge(result) => sched.on_judge(at, result),
            Event::Route(result) => sched.on_route(at, result),
            Event::Budget => {}
        }
    }
    sched.finish(ended, budget_at)
}

/// A finished job, as the scheduler's `select!` hands it over.
enum Event {
    Baseline(std::result::Result<Result<Baseline>, JoinError>),
    Judge(std::result::Result<(Id, Result<Judged>), JoinError>),
    Route(std::result::Result<(Id, std::result::Result<Routed, RouteFailure>), JoinError>),
    Budget,
}

/// The state of one search.
struct Sched<'a> {
    plan: &'a SearchPlan,
    runner: Arc<dyn Runner>,
    t0: Instant,
    runs: Vec<RunRecord>,
    launched_at: Vec<Instant>,
    judge_started: Vec<Option<Instant>>,
    base: JoinSet<Result<Baseline>>,
    routes: JoinSet<std::result::Result<Routed, RouteFailure>>,
    route_ids: HashMap<Id, usize>,
    judges: JoinSet<Result<Judged>>,
    judge_ids: HashMap<Id, usize>,
    /// Routed runs waiting for a judge lane, first in first out.
    queue: VecDeque<(usize, Routed)>,
    /// `None` until the baseline job has finished.
    baseline: Option<std::result::Result<Arc<Baseline>, String>>,
    /// Durations of finished routes (routed or failed, not killed) and judges.
    route_times: Vec<Duration>,
    judge_times: Vec<Duration>,
    started: bool,
    budget_hit: bool,
    first_judged_at: Option<Instant>,
}

impl<'a> Sched<'a> {
    fn new(plan: &'a SearchPlan, runner: Arc<dyn Runner>, t0: Instant) -> Self {
        Self {
            plan,
            runner,
            t0,
            runs: Vec::new(),
            launched_at: Vec::new(),
            judge_started: Vec::new(),
            base: JoinSet::new(),
            routes: JoinSet::new(),
            route_ids: HashMap::new(),
            judges: JoinSet::new(),
            judge_ids: HashMap::new(),
            queue: VecDeque::new(),
            baseline: None,
            route_times: Vec::new(),
            judge_times: Vec::new(),
            started: false,
            budget_hit: false,
            first_judged_at: None,
        }
    }

    fn since_t0(&self, at: Instant) -> f64 {
        at.saturating_duration_since(self.t0).as_secs_f64()
    }

    fn any_judged(&self) -> bool {
        self.runs.iter().any(|r| r.outcome == "judged")
    }

    /// Nothing runs, nothing waits for a judge: only a launch could continue the
    /// search, and `launch_ready` has just declined to make one.
    fn is_idle(&self) -> bool {
        self.routes.is_empty() && self.judges.is_empty() && self.queue.is_empty()
    }

    /// The next finished job, or the budget instant (`budget_wake`, only while it is
    /// still ahead). `None` when there is nothing to wait for.
    async fn next_event(&mut self, budget_wake: Option<Instant>) -> Option<Event> {
        let baseline_pending = self.baseline.is_none();
        let judging = !self.judges.is_empty();
        let routing = !self.routes.is_empty();
        let sleeping = budget_wake.is_some();
        let wake = budget_wake.unwrap_or_else(Instant::now);
        tokio::select! {
            biased;
            Some(result) = self.base.join_next(), if baseline_pending => {
                Some(Event::Baseline(result))
            }
            Some(result) = self.judges.join_next_with_id(), if judging => {
                Some(Event::Judge(result))
            }
            Some(result) = self.routes.join_next_with_id(), if routing => {
                Some(Event::Route(result))
            }
            _ = tokio::time::sleep_until(wake), if sleeping => Some(Event::Budget),
            else => None,
        }
    }

    fn on_baseline(&mut self, result: std::result::Result<Result<Baseline>, JoinError>) {
        self.baseline = Some(match result {
            Ok(Ok(baseline)) => Ok(Arc::new(baseline)),
            Ok(Err(e)) => Err(format!("{e:#}")),
            Err(e) => Err(format!("the baseline job did not finish: {e}")),
        });
    }

    fn on_route(
        &mut self,
        at: Instant,
        result: std::result::Result<(Id, std::result::Result<Routed, RouteFailure>), JoinError>,
    ) {
        let (index, result) = match result {
            Ok((id, result)) => (self.route_ids.remove(&id), result),
            Err(e) => (
                self.route_ids.remove(&e.id()),
                Err(RouteFailure::Failed(format!(
                    "the route job did not finish: {e}"
                ))),
            ),
        };
        let Some(index) = index else { return };
        let took = at.saturating_duration_since(self.launched_at[index]);
        let routed_s = self.since_t0(at);
        let hang_cap_s = self.plan.hang_cap.as_secs();
        let record = &mut self.runs[index];
        record.route_s = Some(took.as_secs_f64());
        match result {
            Ok(routed) => {
                record.routed_s = Some(routed_s);
                record.routed = Some(routed.clone());
                self.route_times.push(took);
                self.queue.push_back((index, routed));
            }
            Err(RouteFailure::Failed(message)) => {
                record.outcome = "route_failed";
                record.reason = Some(message);
                self.route_times.push(took);
            }
            Err(RouteFailure::Killed) => {
                record.outcome = "hung";
                record.reason = Some(format!(
                    "killed at the hang cap ({hang_cap_s} s); never retried"
                ));
            }
        }
    }

    fn on_judge(
        &mut self,
        at: Instant,
        result: std::result::Result<(Id, Result<Judged>), JoinError>,
    ) {
        let (index, result) = match result {
            Ok((id, result)) => (self.judge_ids.remove(&id), result),
            Err(e) => (
                self.judge_ids.remove(&e.id()),
                Err(anyhow!("the judge job did not finish: {e}")),
            ),
        };
        let Some(index) = index else { return };
        let took = at.saturating_duration_since(
            self.judge_started[index].unwrap_or(self.launched_at[index]),
        );
        let judged_s = self.since_t0(at);
        self.judge_times.push(took);
        let record = &mut self.runs[index];
        record.judged_s = Some(judged_s);
        record.judge_s = Some(took.as_secs_f64());
        match result {
            Ok(judged) => {
                record.outcome = "judged";
                record.judged = Some(judged);
                self.first_judged_at.get_or_insert(at);
            }
            Err(e) => {
                record.outcome = "judge_failed";
                record.reason = Some(format!("{e:#}"));
            }
        }
    }

    /// Hands queued runs to free judge lanes. No judge starts before the baseline is
    /// done; if the baseline failed, every queued run fails with its reason.
    fn start_judges(&mut self, now: Instant) {
        let baseline = match &self.baseline {
            None => return,
            Some(Err(message)) => {
                let reason = format!("the unrouted board's baseline failed: {message}");
                let judged_s = self.since_t0(now);
                while let Some((index, _)) = self.queue.pop_front() {
                    let record = &mut self.runs[index];
                    record.outcome = "judge_failed";
                    record.reason = Some(reason.clone());
                    record.judged_s = Some(judged_s);
                }
                return;
            }
            Some(Ok(baseline)) => Arc::clone(baseline),
        };
        while self.judges.len() < self.plan.lanes.judges.max(1) {
            let Some((index, routed)) = self.queue.pop_front() else {
                break;
            };
            self.judge_started[index] = Some(now);
            let job = self.runner.judge(routed, Arc::clone(&baseline));
            let handle = self.judges.spawn(job);
            self.judge_ids.insert(handle.id(), index);
        }
    }

    /// The first wave (every router lane at once, or the fixed count), then, after
    /// each event, further runs while a lane is free and the launch rule allows.
    fn launch_ready(&mut self, now: Instant, passed: bool) {
        let order_len = self.plan.order.len();
        let routers = self.plan.lanes.routers.max(1);
        if !self.started {
            self.started = true;
            for _ in 0..routers.min(order_len) {
                self.launch_one(now);
            }
            return;
        }
        if self.plan.limits.runs.is_some() || passed {
            return;
        }
        while self.routes.len() < routers && self.runs.len() < order_len {
            let queued = self.runs.iter().filter(|r| r.outcome == "running").count();
            if !may_launch(
                now.saturating_duration_since(self.t0),
                self.plan.limits.budget,
                median(&self.route_times),
                median(&self.judge_times),
                queued,
                self.plan.lanes.judges,
            ) {
                break;
            }
            self.launch_one(now);
        }
    }

    fn launch_one(&mut self, now: Instant) {
        let index = self.runs.len();
        let (shape, seed) = self.plan.order[index];
        let deadline = now + self.plan.hang_cap;
        let job = self.runner.route(index, shape, seed, deadline);
        let launched_s = self.since_t0(now);
        let handle = self.routes.spawn(job);
        self.route_ids.insert(handle.id(), index);
        self.launched_at.push(now);
        self.judge_started.push(None);
        self.runs.push(RunRecord {
            index,
            shape,
            seed,
            launched_s,
            routed_s: None,
            judged_s: None,
            route_s: None,
            judge_s: None,
            outcome: "running",
            reason: None,
            routed: None,
            judged: None,
        });
    }

    /// The budget's cut: every run still running is recorded `cut_at_budget`.
    fn cut(&mut self) {
        self.routes.abort_all();
        self.judges.abort_all();
        self.queue.clear();
        let budget_s = self.plan.limits.budget.map_or(0, |b| b.as_secs());
        for record in self.runs.iter_mut().filter(|r| r.outcome == "running") {
            record.outcome = "cut_at_budget";
            record.reason = Some(format!(
                "cut at the budget ({budget_s} s) before it was judged"
            ));
        }
    }

    /// Ends every job still alive (their processes die with their futures) and waits
    /// until they have.
    async fn stop_tasks(&mut self) {
        self.routes.shutdown().await;
        self.judges.shutdown().await;
        self.base.shutdown().await;
    }

    fn finish(self, ended: Instant, budget_at: Option<Instant>) -> SearchOutcome {
        let elapsed = ended.saturating_duration_since(self.t0);
        let budget_overrun = match (budget_at, self.first_judged_at) {
            (Some(budget), Some(first)) if first > budget => Some(first - budget),
            _ => None,
        };

        // One rank call over the judged runs, in index order.
        let judged: Vec<(usize, &Judged)> = self
            .runs
            .iter()
            .filter_map(|r| r.judged.as_ref().map(|j| (r.index, j)))
            .collect();
        let mut keys: Vec<Option<Key>> = vec![None; self.runs.len()];
        if !judged.is_empty() {
            let refs: Vec<&Judged> = judged.iter().map(|(_, j)| *j).collect();
            for ((index, _), key) in judged.iter().zip(crate::route_judge::rank(&refs)) {
                keys[*index] = Some(key);
            }
        }
        let winner = judged
            .iter()
            .map(|(index, _)| *index)
            .filter(|index| keys[*index].is_some())
            .min_by(|a, b| keys[*a].cmp(&keys[*b]).then(a.cmp(b)));

        let stopped_launching = if self.plan.order.is_empty() {
            None
        } else if self.plan.limits.runs.is_some() {
            Some("fixed run count".to_string())
        } else if self.runs.len() >= self.plan.order.len() {
            Some(if self.plan.order.len() >= MAX_RUNS {
                format!("MAX_RUNS ({MAX_RUNS})")
            } else {
                "order exhausted".to_string()
            })
        } else if self.budget_hit {
            Some("the budget".to_string())
        } else {
            Some("the launch rule".to_string())
        };

        let baseline = match &self.baseline {
            Some(Ok(baseline)) => Ok((**baseline).clone()),
            Some(Err(message)) => Err(message.clone()),
            None => Err(
                "the unrouted board's baseline had not finished when the search ended".to_string(),
            ),
        };
        SearchOutcome {
            runs: self.runs,
            winner,
            keys,
            elapsed,
            budget_overrun,
            stopped_launching,
            baseline,
        }
    }
}

/// The launch rule of decision 6, alone so it is tested alone: a run started at
/// `now` (since t0) is expected to be judged by `budget` when
/// `now + route + ceil(queued / judges) * judge + judge <= budget`, `queued` being
/// the runs routed or routing that are not yet judged. False when either median is
/// `None` (nothing finished yet) or there is no budget.
pub(crate) fn may_launch(
    now: Duration,
    budget: Option<Duration>,
    route_median: Option<Duration>,
    judge_median: Option<Duration>,
    queued: usize,
    judges: usize,
) -> bool {
    let (Some(budget), Some(route), Some(judge)) = (budget, route_median, judge_median) else {
        return false;
    };
    let waves = queued.div_ceil(judges.max(1));
    let waves = u32::try_from(waves).unwrap_or(u32::MAX);
    let finish = now
        .saturating_add(route)
        .saturating_add(judge.saturating_mul(waves))
        .saturating_add(judge);
    finish <= budget
}

/// The median of a list of durations (the lower middle of an even count); `None`
/// for an empty list.
pub(crate) fn median(values: &[Duration]) -> Option<Duration> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    Some(sorted[(sorted.len() - 1) / 2])
}

#[cfg(test)]
#[path = "route_search_tests.rs"]
mod tests;
