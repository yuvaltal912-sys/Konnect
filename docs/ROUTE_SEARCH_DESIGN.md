# Best-of-N routing with fastroute: design

Status: design approved by the owner on 2026-11-16 ("כן, מאושר"), with decision 2 changed
at his word (personal use: the GPL is not a design constraint). Claude Opus 5.5 (xhigh)
designed it down to the skeletons; Sonnet 5.5 (high) agents write the bodies. Branch
`claude/route-best-of-n`, from `fork/main` `40517d4c` (merge of #69); it does not need
PR #71's export changes.

## What the owner asked for

- Best-of-N routing is the default of `route_specctra_dsn` in this fork.
- The engine is the fastroute candidate (C\*\* + BR1 + W8P). Build x18 since 2026-11-17 (x14 plus
  C1: wires and vias read back from a session keep their own net class's clearance; C1b: an
  imported via takes its via rule's clearance class; C1c: with no or an empty via rule, class
  1, as the autorouter does; W1: a wire that collapses to one point is not written to the
  session, which Konnect's importer refused; engine commit 1e94751); the switches are
  unchanged from x14. x17 (engine commit 5e7c0348) was the build from 2026-11-16.
- The user picks an effort level: Fast, M, H, XH. Budgets per board, runs in parallel:
  Fast = 2 runs at once, M = 4 min, H = 12 min, XH = 40 min (the owner's "double the first
  proposal" of 1 run / 2 / 6 / 20 min; he confirmed this reading on 2026-11-16).
- Konnect judges every run itself and keeps the best by gates that need no human routing.
- Freerouting stays available as an engine.

The evidence is in the router-design research report (stages 3, 4 and 7, and its section
"Konnect changes", items 2, 3, 5 and 7). It is git-ignored and private; this document
repeats no board data.

## The flow of one call

```
route_specctra_dsn(dsn_path, ses_output_path, effort?, engine? = "fastroute", fastroute_path?)
 1. arguments: engine, effort (argument, else user config routing.effort, else "m")
 2. fastroute: locate (argument, env KONNECT_FASTROUTE, user config routing.fastroute_path,
    a 1 Kopper package's own build next to the executable, the default place), then probe that it is the candidate build
 3. judge preflight: manifest beside the DSN, the saved board equals the export's snapshot,
    kicad-cli present
 4. variants: shapes {as exported, every plane omitted} x seeds 1, 2, 3, ...
    each variant = the DSN with planes stripped (omitted shape), placement reordered by the
    seed, and the pad-exit stub rule off
 5. the timed search: router lanes route variants (hold sequence, sandboxed fastroute),
    judge lanes judge them (scratch copy: apply, refill + DRC, stitching, audit)
 6. the winner by the rank key; its session is written to ses_output_path
 7. the result: today's fields (status, completion, intent_widths, width_exceptions) for the
    winner, plus engine, effort and search
```

Nothing is written to the user's board. The winner's session is applied later with
`apply_specctra_ses` against the same manifest (the importer reads the manifest's pour
rows as information only, so one manifest serves both shapes), and the stitching steps stay
next steps (`completion.next_steps`), as row S6 decided: no hidden write path.

## Decisions taken in this design (with the reason)

| # | Decision | Reason |
|---|---|---|
| D1 | The pour axis is "as exported" plus "every plane omitted", the second derived from the given DSN by removing its `(plane …)` entries. A DSN with no plane has one shape | Stage 3/5 of the research: the pour shape alone separates "no complete board" from "6 of 14 pass every gate" on one board. The export widens no pour net in either shape (row S6), so removing the planes is the omitted export; the importer checks the zone inventory against the board, not the DSN's planes |
| D2 | Seeds reorder the `(component …)` blocks of `(placement …)`; every other byte is kept. A seeded SplitMix64 Fisher-Yates, not the research's Python orders | The research: 98 distinct routings from 98 orders, while thread count gives 3. The importer matches placement by reference, so order cannot change what is applied |
| D3 | Launch order: seed 1 in every shape, then seed 2 in every shape, … | Both shapes are tried from the first wave |
| D4 | Every run uses the research's hold sequence: Full (retry off, optimizer on); only if connections stay open and the manifest gives one floor width, Fallback (retry at the floor, optimizer off, from Full's session) kept only if it closes at least one; then Polish (retry off, optimizer on, from Fallback's session), kept when it leaves no more open than Fallback. Later passes start from `--initial-session` | The candidate was measured on this sequence with `--initial-session`; BR1 fires only on a session import. "Fallback kept only if it closes one" is the product's S9 rule and is stricter than the research driver, which kept a Fallback that closed nothing. A Fallback or Polish that fails (an error, not the 900 s cap) is recorded `failed` and the run keeps the session it had; a failed Full fails the run | A later pass only ever improves on a session that already exists, so its failure must not throw that session away |
| D5 | Router lanes = clamp(cores / 2, 1, 7), 2 threads each; judge lanes = clamp(cores / 4, 1, 3). Fast: its 2 runs at once | The research's throughput table: 7 x 2 was the best on 14 cores (5.78 routings a minute), with 3 judges; peak memory at most 571 MB a run |
| D6 | The budget clock starts when the search starts (the unrouted board's DRC runs in it, beside the first wave). The first wave fills every router lane at once. After that a run starts only when a router lane is free and, by the medians so far, it could be routed and judged before the budget: `now + route + ceil(queued / judges) x judge + judge <= budget`, `queued` counting the runs routing or waiting for a judge. No run starts after the first wave until one route and one judge have finished (both medians are needed) | The task's rule ("a route starts only while its judge can still finish"), as stage 4 measured it, without the fixed first guesses that stopped stage 4's B3 early. Waiting for the first judge costs throughput (lanes idle for one judge's time), never quality |
| D7 | At the budget: if a run is judged, every running route and judge is cut and recorded `cut_at_budget`. If none is judged yet, the search waits for the first judged run (no new launches) and reports the overrun. Before the cut, every route and judge that has already finished is collected, so a run that ended at the budget instant is kept | The owner's top value is quality, so a call never ends with nothing because one board routes slowly (one research run took 622 s); the overrun is reported as information |
| D8 | A route that runs past 900 s (all its passes together) is killed (SIGKILL to its process, then waited for) and recorded `hung`, never retried. `sandbox-exec` replaces itself with the binary and the profile lets no other program start, so that process is the whole run; dropping the call kills it the same way (`kill_on_drop`). Fast has no budget, only this cap | The research's largest hang cap; its hangs were endless loops, and a killed run is never counted as a routing |
| D9 | No run cap: under a budget (M, H, XH) the order of D3 has no end, and runs launch until the budget, by D6's rule. Fast keeps its 2 runs. (Until 2026-11-16 this was "at most 240 runs a call": on the public fixtures M reached 240 runs in 94 s, so H and XH would have stopped at the same point and not used their time.) | The owner, 2026-11-16, choosing between "time decides, no cap" and "the later of the two": "הזמן קובע, בלי תקרה". Each level now means its time on every board; on a large board nothing changes, the budget ends it first. D18 to D20 keep the scratch boards, the failures and the result bounded |
| D10 | Absolute gates (research change 7), no baseline: G1 complete, G2 DRC, G3 width (section below). Audit 4.2's via need stays as the audit counts it today (a net's total vias against one layer change); "per layer change" is an audit change of its own and is not in this PR | 4.2 lives in `audit_trace_current`; changing what it counts changes every audit, and needs its own measurement |
| D11 | T1 (spoke tracks for starved thermals, research change 1) is not in the judge | Not in the requested judge; it is its own Konnect change for either router. Without it some runs fail G2 by a starved thermal, which the rank still orders |
| D12 | The rank key is the research's (`exp20.key`, which reproduces the bench's own order), with the lengths stepped against the shortest run that passes G1 (the shortest of all judged runs when none does) instead of a human routing, so that a degenerate run that is not complete cannot make the steps finer than the real runs | Keeps the measured order; a product user has no baseline |
| D13 | fastroute runs only on macOS for now, inside `sandbox-exec` with a profile written per run: no network, writes only to the run folder and the null devices, no process but the binary itself, and an environment of `HOME`, `TMPDIR` and `PATH` only. Everything else stays allowed (`allow default`): reads anywhere, and Mach and XPC service lookups. It contains the network and writes; it is not a full jail. On Windows and Linux the engine refuses and names `engine: "freerouting"` | The research's isolation rule; macOS first, other systems validated at the end of the series. A deny-default profile would have to list every system service the binary needs, and would break on a macOS update |
| D14 | Each run's folder is reduced to its record once the search ends; the winner's folder is kept whole, with its judged board when it still has one (D19) | Thousands of runs of board copies would fill the disk |
| D15 | `status` is the router's value (today's meaning, from its count and the planes) only when the winner passes all three gates. If G1 fails it is `incomplete`; if only G2 or G3 fails it is the new `complete_with_gate_failures`. The verdict itself is beside it (`completion.judged`, `search.winner.gates`) | A caller that checks `status == "complete"` must never accept a board the judge failed. On the owner's boards fastroute breaks a netclass clearance that Freerouting keeps (research session, 2026-11-16); G2 catches it, and this status makes it visible |
| D16 | If 3 judges have failed (an error, not a gate) and none has succeeded, no further run starts; the runs in flight finish, and `stopped_launching` says "the judge failed on every run so far". A judge that then succeeds on a run in flight lifts the stop | A judge that fails on every run (a broken kicad-cli, a refused board) would otherwise spend the whole budget routing runs nobody can judge |
| D17 | The judge's kicad-cli never runs on the user's KiCad settings. The call copies them once (`<search folder>/kicad-config`, from `$KICAD_CONFIG_HOME` or the user's KiCad settings folder; reported as `search.kicad_config_home`), and the baseline and each run's judge get their own copy of that, removed when they are done. A link inside the settings is left out of the copy and named in `search.kicad_config_skipped_links`; it never stops the call | The baseline and up to 3 judges run kicad-cli at once, and kicad-cli re-saves its settings on exit: one shared folder, the user's or the call's, would race. A copied link would let kicad-cli write through it |
| D18 | Five routes in a row that fail (an error, not the hang cap) stop launching; a route in flight that succeeds lifts the stop. `stopped_launching`: "5 routes in a row failed" | With no cap, routes that all start failing (a full disk, a broken sandbox) would launch failing runs until the budget. When no route has succeeded at all the launch rule already stops after the first wave (it has no judge time) |
| D19 | During the search, every run that finished without a verdict, and every judged run outside the 8 best by the rank key so far, is released at once: its judge folder (the scratch boards) and its router logs and sandbox folders are removed; its variant, sessions and reports stay. A released run can still win (the key's length steps are relative to the shortest run, so the order of close runs can change); its session is then the result and `judged_board` is null with a note | Keeps the scratch boards to about 8 copies however long the search runs, without ever losing the winner's session. The small files that stay (variant, sessions, reports) grow with the run count until D14 reduces them at the end; the largest search folder on the public fixtures was 200 MB at its peak (pic, XH, 2290 runs) |
| D20 | The result's `search.runs` lists the 20 best judged runs by the rank key and the first 20 runs that were not judged; `search.outcomes` counts every outcome, and `search.json` in the search folder lists every run | Thousands of rows would not fit a tool response |
| D21 | When the winner passes all three gates and the judge added no stitching via, `status` is the judge's, not the router's count (amends D15): `complete`, or `complete_with_width_exceptions` or `unverified` from the winner's width report, or `stopped`. The router's `incomplete` and its plane `unverified` give way. When the judge needed stitching vias, those vias are not in the session, so the router's status stands. The router's count stays in `completion.router_unrouted`, the via count in `completion.judged.stitching_vias`, and `completion.reason` says why the status differs from the count, or why it stands | The router counts a connection a pour makes as one it left. On the public `specctra_pours_routing` board, 4 of the 6 recorded routes, and both 1 Kopper smoke runs, read `incomplete` while the judge found 0 unconnected with every gate passing (2026-11-17). G1 is KiCad's own count after the refill and the stitching, which is what the router's count cannot show |

- **D22 (2026-11-18): the judge checks the saved board as a tree, not only as bytes.** The export hashes KiCad's in-memory text of the board (IPC). KiCad saves that board in another layout and without each footprint's `version`, `generator` and `generator_version`, so the saved bytes never hashed to `source_sha256`, and the judge refused every live board. The export now also writes `source_canonical_sha256`: the board as a parsed tree, with that footprint metadata dropped (the rule DRC's live-to-saved check already used), hashed with lengths and atom/string tags. The judge accepts the saved board when its bytes match `source_sha256` or its tree matches `source_canonical_sha256`. Any other change to the board still refuses. A manifest from before D22 gets the byte check only.

- **D23 (2026-11-20): differential pairs declared in the design intent are routed coupled.** fastroute has `--pairs=FILE` (the pairs are routed and coupled first, then fixed while the rest routes); Konnect never passed it. `route_specctra_dsn` now builds the file from the design intent and passes `--pairs=<search folder>/pairs.txt` on every pass of every run of the search (every shape and seed). `--pairs` is a per-call input, so it is not one of `CANDIDATE_SWITCHES`; the sandbox profile (D13) allows reads anywhere, so a run reads the file from the call's folder.
  - **Source: the design intent only.** Every `interfaces[].differential_pairs` of `<project>/.konnect/design_intent.json`, in file order, the project being the folder of the board the DSN was exported from (the manifest's `board_path`). Pairs are never guessed from net names. No intent file, an intent with no pair, or an intent that cannot be used: no `--pairs`, and the command line is as before pairs existed. An intent that exists and cannot be used adds `differential_pairs_note` to the result.
  - **Names.** fastroute matches names as the DSN writes them, and a net of a sub-sheet carries its sheet path (`/sheet/CAN_H`). Each intent name is resolved against the nets of the DSN's `network`: the net with exactly that name, else the one net whose name ends with `/` and the name, else unresolved (none, or more than one). A pair is left out, and routed as two ordinary nets, when a name is unresolved, when both names are one net, when a resolved name holds white space (a pairs file is split at white space), or when a net is already in an earlier pair. The result says which pair and why (`differential_pairs_unresolved`).
  - **Gap.** `gap=` is the `diff_pair_gap` (mm) of the positive net's netclass, in `<board>.kicad_pro`. The class is resolved as KiCad does (`NET_SETTINGS::GetEffectiveNetClass`, `makeEffectiveNetclass`): the classes that name the net by assignment or pattern, ordered by `priority` (lowest number first, equal by name), then `Default`; the first that sets the value decides, so a class that leaves it unset takes the Default's. A value of 0 is KiCad's "not set": no `gap=`, and fastroute uses the clearance. Skew stays at fastroute's default.
  - **Result.** `differential_pairs` (`positive`, `negative`, `dsn_positive`, `dsn_negative`, `gap_mm` or null) and `differential_pairs_unresolved` (`positive`, `negative`, `reason`), both empty lists when the intent has none.
  - **The placement bench** routes through the same call. `scratch_board::copy_project` now copies `.konnect/design_intent.json` when it exists and nothing else from `.konnect`, so a candidate's copy declares the same pairs as the board.

  Reason: the owner asked for CAN_H/CAN_L, and both halves existed: fastroute's `--pairs` and the intent's pair list. The design intent is the one place the owner states which nets are a pair, so a guess from net names would route a pair he never declared. A wrong guess would couple nets he never paired; a missing pair costs only the coupling.

## Decisions for the owner

1. **The budget reading**: Fast = 2 runs at once, M = 240 s, H = 720 s, XH = 2400 s per board.
2. **How Konnect finds fastroute.** This fork is for the owner's personal use only, so the
   GPL is not a design constraint (owner, 2026-11-16). Konnect looks for the binary itself at
   one fixed place, `~/.local/share/fastroute/candidate/fastroute`; the `fastroute_path`
   argument, the environment variable `KONNECT_FASTROUTE` and the user config key
   `routing.fastroute_path` override it, in that order. Since 1 Kopper K2 (2026-11-17), the
   `fastroute` next to the `konnect` executable (its resolved path) comes after the user
   config and before the fixed place, when that executable is inside a 1 Kopper package
   (`1-kopper.json` one folder up), so a package finds its own build; when that file is
   there but unusable, the call is refused rather than routed with another build. Before routing, a probe proves the
   binary is the candidate build (every candidate switch is accepted). On this Mac the x18
   build is copied once to that place (it lives in a research worktree today). fastroute
   still runs as its own process, for technical reasons only: a hung run (2 of 100 seeds in
   the research) can be killed only as a process, the no-network sandbox applies to a
   process, and parallel runs keep their memory apart.
3. **When fastroute is not there**: the call refuses with the setup steps and says that
   `engine: "freerouting"` routes with Freerouting. No silent fallback to Freerouting.
4. **The default effort** when the call and the user config give none: M.

## Modules

New files in `crates/konnect-core/src/`:

| File | What |
|---|---|
| `fastroute.rs` | the engine: locate, probe, the command line, the sandbox profile, one pass, the hold sequence |
| `route_search.rs` | effort and budgets, DSN variants, lanes, the timed search (the scheduler) |
| `route_judge.rs` | judge preflight, the unrouted board's baseline, judging one run, gates, rank key |
| `scratch_board.rs` | the scratch project copy and the file-only writer of tracks, arcs and vias into it |
| `tools/route_pairs.rs` (D23) | the differential pairs of a call: the intent's pairs, resolved against the DSN's nets, with their gaps and the `--pairs` file |
| `fastroute_tests.rs`, `route_search_tests.rs`, `route_judge_tests.rs`, `scratch_board_tests.rs` | tests |

Changed: `tools/integration.rs` (schema, the engine switch, the fastroute handler),
`docs/API_MIGRATIONS.md`, `tool-directory.md`, the `kicad-pcb` skill and the agents' routing
guidance (ask once for an effort level, save it as `routing.effort`), `docs/CREATION_HANDOFF.md`.
No new tool: the registry count is unchanged. Not touched: `specctra.rs`, `specctra_ses.rs`,
`tools/pcb_export.rs`, `tools/layout_quality*.rs` (PR #71's files), `freerouting_mcp.rs`.

## Skeletons

Bodies say what they must do. `Result` is `anyhow::Result` unless named.

### `fastroute.rs`

```rust
//! fastroute (a Rust port of Freerouting) as a separate executable: a command line, a DSN
//! in, a session and a JSON report out. A process, not a library, so that a hung run can be
//! killed, the run can be sandboxed off the network, and parallel runs keep their memory
//! apart.

/// The candidate the owner chose on 2026-11-16 (C** + BR1 + W8P; build x18, engine commit
/// 1e94751, the same switches as x14). Sent on every pass, in this order.
pub(crate) const CANDIDATE_SWITCHES: &[&str] = &[
    "--no-time-limits", "--x.no_clock", "--x.par_vias=340", "--x.work_cap=2000000",
    "--x.par_vias_noacute", "--x.bridge_full_width", "--x.par_vias_net",
    "--x.par_vias_strict", "--x.par_via_each_a=0.8975",
];
pub(crate) const CANDIDATE_NAME: &str = "C** + BR1 + W8P";
/// Threads per run (the research's 7 x 2).
pub(crate) const THREADS: u32 = 2;
/// The probe's switch no build knows: a candidate build names it as unknown only after it
/// has accepted every candidate switch before it.
const SENTINEL_SWITCH: &str = "--x.konnect_probe_unknown";

/// route_specctra_dsn's measured preset as the research gave it to fastroute, without the
/// per-board values (retry width, edge clearance), which come from the manifest.
const PRESET: &[&str] = &[
    "--router.autorouter.max_passes=20",
    "--router.optimizer.improvement_threshold=0.0025",
    "--router.optimizer.max_consecutive_failures=1000000",
    "--router.optimizer.max_consecutive_failures_pass1=1000000",
    "--router.scoring.via_costs=10",
    "--router.scoring.default_undesired_direction_trace_cost=1.0",
    "--router.fanout.enabled=false",
    "--router.automatic_neckdown=false",
    "--router.job_timeout=00:10:00",
    "--router.optimizer.timeout=00:10:00",
];

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Binary {
    pub path: PathBuf,       // canonical
    pub sha256: String,
    pub version: String,     // first line of `--version`
    pub source: &'static str // "argument" | "environment" | "user_config" | "default"
}

pub(crate) enum Located {
    Found(Binary),
    /// Nothing named a binary and none is at the default place: the setup steps say where
    /// to put one.
    NotConfigured,
    /// A path was named but is not a file, not executable, or not the candidate build.
    Refused { path: PathBuf, source: &'static str, reason: String },
    /// This system cannot isolate the run (D13).
    Unsupported { reason: String },
}

/// The first of: `argument`, `KONNECT_FASTROUTE`, `config` (user config
/// `routing.fastroute_path`), `bundled` (next to the executable, since K2), the default
/// `~/.local/share/fastroute/candidate/fastroute`.
/// No other folder is searched, nothing is downloaded. Refuses off macOS and when
/// `/usr/bin/sandbox-exec` is missing. Then `probe`.
pub(crate) const DEFAULT_PATH: &str = ".local/share/fastroute/candidate/fastroute"; // under HOME
pub(crate) async fn locate(argument: Option<&Path>, config: Option<PathBuf>, scratch: &Path) -> Located;

/// Proves `path` is the candidate build, each step sandboxed (`sandbox_profile`) with a
/// cleared environment: `--version` must print a first line starting `fastroute `; then
/// `-de <scratch>/absent.dsn` + every CANDIDATE_SWITCHES entry + SENTINEL_SWITCH must print
/// `unknown experimental switch --x.konnect_probe_unknown` (a build that lacks a candidate
/// switch names that one; a build with no experimental switches names none). Hashes the
/// file. Each step is killed after 10 s.
async fn probe(path: &Path, scratch: &Path) -> Result<Binary>;

/// One pass's own settings.
#[derive(Debug, Clone)]
pub(crate) struct Pass {
    pub name: &'static str,             // "full" | "fallback" | "polish"
    pub neck_width_um: f64,             // 0 = the retry is off
    pub optimizer: bool,
    pub initial_session: Option<PathBuf>,
}

/// The command line of one pass, each setting once: `-de dsn -do ses --report=report`,
/// PRESET, `--router.optimizer.enabled`, `--router.neck_width_um`,
/// `--router.copper_to_edge_clearance_um` (from `hints`, 250 sent as 250.001 as the
/// Freerouting path does), both `max_threads` at THREADS, CANDIDATE_SWITCHES,
/// `--x.par_via_amps=<amps>` when given, `--initial-session=<file>` as one token. Never
/// `--live`, `--multi-start` or `--max-time`.
pub(crate) fn pass_arguments(dsn: &Path, ses: &Path, report: &Path, hints: &RoutingHints,
                             amps: Option<&Path>, pass: &Pass) -> Vec<String>;

/// The `--x.par_via_amps` file from the manifest's `intent_widths` rows that carry a
/// `current_a` and are routed (`not_routed` rows left out): one `<amps> <net>` line each,
/// sorted by net. `None` when no row has a current.
pub(crate) fn amps_file_text(manifest: &serde_json::Value) -> Option<String>;

/// The run's sandbox profile: `(deny network*)`, `(deny file-write*)` then allow writes
/// under `folder` and to /dev/null, /dev/zero, /dev/tty, /dev/dtracehelper only,
/// `(deny process-exec)` then allow only `binary`. Paths are quoted for the profile
/// language; a path with a quote or backslash is refused.
pub(crate) fn sandbox_profile(folder: &Path, binary: &Path) -> Result<String>;

#[derive(Debug, Clone)]
pub(crate) struct PassOutcome {
    pub ses: PathBuf,
    pub unrouted: usize,              // report.json `unrouted`, a list
    pub clearance_violations: usize,  // report.json `clearance_violations`
    pub seconds: f64,
}

/// Runs one pass in its own folder: writes the profile, runs
/// `/usr/bin/sandbox-exec -f profile binary args…` with only
/// HOME=folder/home, TMPDIR=folder/tmp, PATH=/usr/bin:/bin (no FASTROUTE_* or
/// FREEROUTING__* variable), stdout and stderr to folder/fastroute.log. Killed at
/// `deadline` (SIGKILL to the child, then waited for), then `Err(Killed)`. Dropping the
/// future kills it too (`kill_on_drop`). Err on a non-zero exit, a missing or empty SES, or a report that
/// does not parse.
pub(crate) async fn run_pass(binary: &Binary, folder: &Path, dsn: &Path, hints: &RoutingHints,
                             amps: Option<&Path>, pass: Pass, deadline: tokio::time::Instant)
                             -> Result<PassOutcome, PassError>;

pub(crate) enum PassError { Killed, Failed(anyhow::Error) }

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Held {
    pub ses: PathBuf,                 // the kept pass's session
    pub unrouted: usize,
    pub kept: &'static str,
    pub passes: Vec<serde_json::Value>, // name, outcome (kept|discarded|skipped), unrouted, seconds, why
    pub seconds: f64,
}

/// D4's sequence in `folder/full`, `folder/fallback`, `folder/polish`. The floor is
/// `hints.retry_width_um`; with none, or `fallback == WidthFallback::None`, Fallback is
/// skipped with the reason in its row. One `deadline` bounds every pass (D8); a pass killed
/// by it ends the run as `Killed`.
pub(crate) async fn route_held(binary: &Binary, folder: &Path, dsn: &Path, hints: &RoutingHints,
                               amps: Option<&Path>, fallback: WidthFallback,
                               deadline: tokio::time::Instant) -> Result<Held, PassError>;
```

### `route_search.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Effort { Fast, M, H, Xh }

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Limits {
    pub budget: Option<Duration>, // None for Fast
    pub runs: Option<usize>,      // Some(2) for Fast
}

impl Effort {
    /// "fast", "m", "h", "xh"; anything else is None.
    pub(crate) fn parse(text: &str) -> Option<Self>;
    pub(crate) fn name(self) -> &'static str;
    /// The owner's budgets (2026-11-16): Fast 2 runs, M 240 s, H 720 s, XH 2400 s.
    pub(crate) fn limits(self) -> Limits;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EffortSource { Argument, UserConfig, Default }

/// `effort` argument, else the user config's `routing.effort`, else M. A value that is not
/// one of the four is refused, from either place, naming where it came from.
pub(crate) fn resolve_effort(argument: Option<&Value>, config: Option<&Value>)
    -> Result<(Effort, EffortSource), String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Shape { AsExported, Omitted }

/// AsExported always; Omitted too when the DSN's structure holds a plane.
pub(crate) fn shapes(dsn_text: &str) -> Result<Vec<Shape>>;

/// A variant's DSN text: Omitted strips every `(plane …)` of the structure; seed >= 1
/// reorders the placement's component blocks (seed 0 keeps them); the stub rule is always
/// off (`stub_rule_off`). Bytes outside those edits are kept. Quote-aware block scanning.
pub(crate) fn variant_text(dsn_text: &str, shape: Shape, seed: u64) -> Result<String>;

/// Removes each direct `(plane …)` child of `(structure …)` with the whitespace before it;
/// returns the text and how many it removed.
fn strip_planes(text: &str) -> Result<(String, usize)>;

/// Fisher-Yates over the `(component …)` children of `(placement …)` with SplitMix64(seed);
/// the gaps between blocks stay where they were. Refuses a placement child that is not a
/// component.
fn reorder_placement(text: &str, seed: u64) -> Result<String>;

/// In the structure's own `(rule …)`: replaces the value of an existing
/// `(clearance N (type smd_to_turn_gap))`, else adds `(clearance 0 (type smd_to_turn_gap))`
/// as its last child (research CR1). Refuses a structure with no rule.
fn stub_rule_off(text: &str) -> Result<String>;

/// D3: (shape, seed) for seed 1, 2, … and every shape; Fast's fixed count, else
/// with no end (D9, as changed on 2026-11-16).
pub(crate) fn launch_order(shapes: &[Shape], limits: Limits) -> Order;
pub(crate) const HANG_CAP: Duration = Duration::from_secs(900);

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub(crate) struct Lanes { pub routers: usize, pub judges: usize }

/// D5. Fast: routers = its run count.
pub(crate) fn lanes(cores: usize, effort: Effort) -> Lanes;

/// What a router lane hands to a judge lane.
#[derive(Debug, Clone)]
pub(crate) struct Routed {
    pub index: usize, pub shape: Shape, pub seed: u64,
    pub dsn: PathBuf, pub held: fastroute::Held,
    pub planes: Vec<String>,          // the nets the variant gave the router as planes
    pub folder: PathBuf,
}

/// The two jobs the scheduler runs, so tests drive it with fakes on a paused clock.
pub(crate) trait Runner: Send + Sync + 'static {
    fn route(&self, index: usize, shape: Shape, seed: u64, deadline: tokio::time::Instant)
        -> BoxFuture<'static, Result<Routed, RouteFailure>>;
    fn judge(&self, routed: Routed) -> BoxFuture<'static, Result<route_judge::Judged>>;
    /// The unrouted board's DRC (G2's baseline); the first judge waits for it.
    fn baseline(&self) -> BoxFuture<'static, Result<route_judge::Baseline>>;
}

pub(crate) enum RouteFailure { Hung, Failed(String) }

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct RunRecord {
    pub index: usize, pub shape: Shape, pub seed: u64,
    pub launched_s: f64, pub routed_s: Option<f64>, pub judged_s: Option<f64>,
    pub route_s: Option<f64>, pub judge_s: Option<f64>,
    pub outcome: &'static str, // judged | route_failed | hung | judge_failed | cut_at_budget
    pub reason: Option<String>,
    #[serde(skip)] pub judged: Option<route_judge::Judged>,
}

pub(crate) struct SearchPlan {
    pub limits: Limits, pub lanes: Lanes, pub order: Vec<(Shape, u64)>, pub hang_cap: Duration,
}

pub(crate) struct SearchOutcome {
    pub runs: Vec<RunRecord>,
    pub winner: Option<usize>,            // index into runs
    pub keys: Vec<Option<route_judge::Key>>,
    pub elapsed: Duration,
    pub budget_overrun: Option<Duration>, // D7, when the first judged run came after the budget
    pub stopped_launching: Option<String>,// why the last launch was the last
    pub baseline: Result<route_judge::Baseline, String>,
}

/// The timed search (D6 to D9). One tokio task set for routes (at most `lanes.routers` at
/// once), one for judges (at most `lanes.judges`), a FIFO of routed runs, the baseline task
/// started at t0. Loop on whichever finishes first: a route (record it; queue it for a
/// judge), a judge (record it; update the medians), the budget instant, or the hang cap of
/// a route (its own deadline makes it return Hung). After each event, launch while the
/// rule of D6 allows. Ends when nothing runs, nothing is queued and nothing more may launch;
/// or at the budget per D7. Cut tasks are dropped (their processes die with them). If the
/// baseline fails, every judge fails with its reason. The winner: least `route_judge::rank`
/// key over the judged runs, ties to the earlier index.
pub(crate) async fn search(plan: &SearchPlan, runner: Arc<dyn Runner>) -> SearchOutcome;

/// The launch rule of D6, alone so it is tested alone: true when a run started `now` is
/// expected to be judged by `budget`. None medians (nothing finished) is false.
pub(crate) fn may_launch(now: Duration, budget: Option<Duration>, route_median: Option<Duration>,
                         judge_median: Option<Duration>, queued: usize, judges: usize) -> bool;
```

### `route_judge.rs`

```rust
/// What the judge needs, read once per call.
pub(crate) struct JudgeContext {
    pub board: PathBuf,              // manifest board_path, canonical
    pub board_text: String,          // the saved board: same bytes or same canonical tree (D22)
    pub manifest: serde_json::Value,
    pub manifest_text: String,
    pub kicad_cli: String,
    pub pour_nets: BTreeSet<String>, // nets of the manifest's pour rows
    pub power_nets: BTreeSet<String>,// nets of intent_widths rows widened or already wide enough
    pub floors_um: BTreeMap<String, i64>, // per net: min_track_width_um, else its class width
    pub root: PathBuf,               // <search>/judge
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Baseline { pub violations: BTreeMap<String, usize>, pub unconnected: usize }

/// Refusals, each with the next step, before anything routes: no manifest beside the DSN;
/// the manifest's board is missing; the saved board is not the exported one, by bytes
/// (`source_sha256`) or as a tree (`source_canonical_sha256`, D22) ("save the board in
/// KiCad, export the DSN again"); no kicad-cli.
pub(crate) fn prepare(dsn: &Path, root: &Path, kicad_cli: &str) -> Result<JudgeContext, Refusal>;

/// G2's baseline: the unrouted board (the saved text, unchanged) in a scratch project,
/// `refill_drc_on_private_copy`, violations counted by type, unconnected counted.
pub(crate) async fn baseline(ctx: &JudgeContext) -> Result<Baseline>;

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Measures {
    pub unconnected: usize,                     // KiCad, after refill and stitching
    pub violations: BTreeMap<String, usize>,    // KiCad, by type, same board
    pub routed_share: Option<f64>, pub audit_complete: Option<bool>,
    pub acute: Option<f64>, pub right_angles: Option<f64>,  // audit 5.3
    pub copper_quality: Option<f64>,            // audit quality.copper_quality
    pub nets_undersized: Option<f64>,           // audit 4.2.nets_undersized
    pub narrow_paths: Option<f64>,              // audit 4.3.narrow_paths
    pub signal_mm: f64, pub power_mm: f64, pub pour_track_mm: f64, // from the session
    pub vias_off_pour: usize,                   // session vias not on a pour net
    pub below_floor: usize,                     // session segments below their net's floor
    pub width_rows: Vec<serde_json::Value>,     // intent_width_report rows for this session
    pub stitching_vias: usize,
    pub route_s: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Gates { pub g1: Vec<String>, pub g2: Vec<String>, pub g3: Vec<String> }

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Judged { pub measures: Measures, pub gates: Gates, pub board: PathBuf,
                           pub rounds: Vec<serde_json::Value> }

/// One run, judged by Konnect itself on scratch copies under `ctx.root/<index>`:
/// 1. `specctra_ses::parse_import_plan(board, board_text, manifest, ses)`: an SES the
///    importer would refuse is a failed judge.
/// 2. `scratch_board::copy_project` + `scratch_board::write_routing` into `r0`.
/// 3. `cli::refill_drc_on_private_copy` on `r0`.
/// 4. Only for `routed.planes` nets with unconnected items: up to 4 rounds of
///    `plan_route_refinement` (through the tool registry: kinds stitching, those nets,
///    search_radius_mm 3.0, effort standard); no proposal or a refusal stops; else every
///    proposed item is written (`scratch_board::write_planned`) into `r<n>` and refilled +
///    DRC'd (research `post`, which adopted every listed via).
/// 5. `audit_layout_quality` on the last board (through the tool registry).
/// 6. `measure`, then `gates`. Earlier rounds' boards are removed; the last is kept.
pub(crate) async fn judge(ctx: &JudgeContext, routed: &Routed, baseline: &Baseline,
                          tools: &ToolContext) -> Result<Judged>;

/// G1 complete: 0 unconnected; audit routed share 1; audit completion complete.
/// G2 DRC: every violation type at most the baseline's count.
/// G3 width: no segment below its floor; 0 `track_width` violations; each intent-width row
/// with at most 5 % of its length and no stretch longer than 2.0 mm below its intent width
/// (the bench's thresholds); audit 4.2 nets undersized 0 and 4.3 narrow paths 0, when the
/// audit ran them. Each failure is one readable line.
pub(crate) fn gates(m: &Measures, baseline: &Baseline) -> Gates;

/// The rank key of D12, for every judged run at once (the steps need the shortest):
/// (G1 failed, G2 failed, G3 failed) as 0 or 1, acute corners, signal length / the
/// shortest judged signal length in steps of 0.02, power the same, right angles, vias off
/// the pour nets, minus copper quality in whole points, route seconds. A measure the audit
/// did not give sorts last.
pub(crate) fn rank(judged: &[&Judged]) -> Vec<Key>;
pub(crate) type Key = (u8, u8, u8, i64, i64, i64, i64, i64, i64, i64);
```

### `scratch_board.rs`

```rust
/// The files a judged copy needs beside the board, from the board's own folder:
/// `<stem>.kicad_pro`, `.kicad_prl`, `.kicad_dru`, `fp-lib-table`, `sym-lib-table`, the
/// `.konnect` folder (design intent) and `*.pretty` folders. No schematic. Returns the
/// copy's board path (same file name).
pub(crate) fn copy_project(board: &Path, into: &Path) -> Result<PathBuf>;

/// `board_text` with every track, arc and via of `plan` added before the root's closing
/// parenthesis: `(segment (start) (end) (width) (layer) (net …) (uuid))`, `(arc …)` the
/// same with `(mid)`, `(via (at) (size) (drill) (layers "F.Cu" "B.Cu") (net …) (uuid))`.
/// Nets the board's way (`konnect_sexp::net::net_ref_for_write`): a net a legacy board does
/// not declare is refused, never net 0. Fresh uuids. Nothing else changes.
pub(crate) fn write_routing(board_text: &str, plan: &SesImportPlan) -> Result<String>;

/// The items of `plan_route_refinement` stitching proposals, written the same way.
pub(crate) fn write_planned(board_text: &str, items: &Planned) -> Result<String>;
pub(crate) struct Planned { pub tracks: Vec<PlannedTrack>, pub vias: Vec<PlannedVia> }
/// The `route_batch` tracks and vias of every proposal's `operations`, in order. Any other
/// operation (`modify_trace`) is refused: stitching proposals create, never move.
pub(crate) fn planned_items(plan: &serde_json::Value) -> Result<Planned>;
```

### `tools/integration.rs`

```rust
// Schema: `engine` {"fastroute" (default), "freerouting"}; `effort` {"fast","m","h","xh"};
// `fastroute_path`. Descriptions say what each engine reads.

/// Which engine, and the arguments the other engine owns refused by name:
/// fastroute refuses jar_path, java_path, max_passes, optimizer_enabled,
/// job_timeout_seconds, overall_timeout_seconds, allow_neckdown true, width_policy "retry";
/// freerouting refuses effort and fastroute_path.
fn route_engine(args: &Value) -> Result<Engine, CallToolResult>;

async fn handle_route_specctra_dsn(args, ctx) {
    match route_engine(args) {
        Engine::Freerouting => route_with_freerouting(args, ctx).await, // today's body, unchanged
        Engine::Fastroute => route_with_fastroute(args, ctx).await,
    }
}

/// 1. effort (resolve_effort with the user config), `check_dsn_input`, a new `.ses`
///    output and a `<ses>.search` folder that does not exist yet.
/// 2. `fastroute::locate`; a refusal carries `setup` (install, name the path, or use
///    engine "freerouting").
/// 3. `route_judge::prepare`.
/// 4. shapes, the variants' DSNs are written by each run in its own folder, the plan.
/// 5. `route_search::search` with the production Runner.
/// 6. no winner: an error result with every run's record; nothing written to ses_output.
/// 7. the winner's session to ses_output (`write_new_atomic`); every other run folder
///    reduced to its record (D14); `search.json` written.
/// 8. the result: status and completion from `completion_report(dsn, ses_output, winner
///    planes, winner unrouted, width)`, with `completion.judged`; intent_widths and
///    width_exceptions from `width_report(dsn, ses_output)`; engine {name, path, sha256,
///    version, candidate, switches}; effort {level, source, budget_s, runs}; search {lanes,
///    shapes, runs, winner {index, shape, seed, gates, measures, judged_board, passes},
///    elapsed_s, budget_overrun_s, stopped_launching}; artifacts.
async fn route_with_fastroute(args: &Value, ctx: &ToolContext) -> anyhow::Result<CallToolResult>;
```

## Tests, each guard with a negative control

| Guard | Test | Negative control |
|---|---|---|
| probe refuses a build that is not the candidate | fake binaries: one that ignores `--x.*`, one that names a candidate switch as unknown | the real candidate's answer accepted (fake that names only the sentinel) |
| sandbox: no network, no writes outside the run folder, no other process | the profile text; a live run on macOS of a fake that tries `/usr/bin/nc 127.0.0.1`, a write to the home folder, and `exec /bin/ls` | the same fake without the profile does all three |
| each setting sent once, never `--live` | `pass_arguments` for every pass | — |
| variant transforms keep every other byte | strip, reorder (multiset of bytes equal; seed 0 identity), stub rule (insert and replace) | a DSN with no structure rule and a placement with a non-component child are refused |
| shapes | a DSN with planes gives two, without gives one | |
| launch rule | `may_launch` table: at, below and above the budget; no medians | |
| scheduler (paused clock, fake runner) | first wave fills lanes; no launch past the rule; cut at the budget once one is judged; wait past the budget when none is; hung run recorded and not retried; no run cap under a budget; 5 route failures in a row stop launching and a success lifts it; runs outside the 8 best released once each; Fast launches exactly 2 and has no budget | removing the cut, or the wait, fails its test |
| file writer | both net formats; arcs; vias; a net a legacy board lacks is refused | net 0 is never written |
| judge preflight | saved board differs from the snapshot; no manifest; no kicad-cli | the matching board is accepted |
| gates | each gate passes and fails on one changed measure | |
| rank | the research key's order on a hand-made set; length steps against the shortest | |
| engine arguments | each foreign argument refused by name, both engines | `engine: "freerouting"` routes exactly as today (the existing Freerouting tests, which gain `engine: "freerouting"`) |
| effort | argument, user config, default; a bad value from each place | |
| end to end (ignored test, real fastroute and kicad-cli) | a public fixture at Fast: a session is written, every run has a record, the winner passes the importer | |

Scoped `cargo llvm-cov` on the four new modules, and targeted `cargo mutants` on the guards
(probe, profile, preflight, gates, the launch rule and the cut), in an isolated checkout, as
`AGENTS.md` says.

## Measurement (public fixtures only)

Boards: `pic_programmer` and `ecc83-pp` (KiCad demos) and `specctra_pours_routing` (a
repository fixture), tracks stripped, exported file-based by the existing probe
(`freerouting_routes_a_folder`, pours fixed). Each effort level once per board, through this
checkout's `target/release/konnect` by pipe, with a scratch `KICAD_CONFIG_HOME` (library
tables copied, iterative refill off) and `KONNECT_STATE_DIR`, under the lane locks. Recorded
per call: wall time, runs launched, judged, hung and cut, the winner's shape, seed, gates and
measures, and the overrun. Expected cost: about 3 h of machine time (XH 40 min x 3).

## What this design does not do

- T1 spokes (research change 1), the stitching planner's local `zone_connect` refusal
  (change 6), the per-net stitching via (change 8), the unrounded per-via capacity (change 9).
- The per-layer-change via need in audit 4.2 (D10).
- Windows and Linux (D13).
- A check that KiCad's IPC snapshot text equals the saved file after a save: the judge
  requires it (preflight), and this was not tested against a live KiCad.
