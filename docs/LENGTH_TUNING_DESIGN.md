# Length tuning (serpentine routing) with fastroute: design (Row LT)

Status: design only. Claude Opus 5.5 (xhigh) designed it down to the skeletons on 2026-11-20,
on branch `claude/length-tuning-design-674041` from `fork/main` `8795de82`. It builds on Row DP
(differential pairs to fastroute, branch `claude/fastroute-pairs` at `5448e46d`, whose code is
that of `98d1f837`; not merged yet) as if DP were merged. Sonnet 5.5 (high) agents write the bodies on the owner's word. Nothing is
built, and no board was written.

## What the owner asked for

- On 2026-11-20 he asked whether 1 Kopper has serpentine (meander) routing. fastroute has it
  (`--tune`, and pair skew in `--pairs`), but Konnect passes neither.
- A design down to function skeletons: where length groups come from, measuring length as
  KiCad does, the judge, pairs, refusals, tests and a verification plan.
- His standing values apply: result quality over run time, measurement first, one clear
  recommendation per decision.

## What the code says today

Checked in `fork/main` `8795de82`, Row DP at `98d1f837`, fastroute x18 (engine `1e94751`; its
source is in the router-design worktree, and `tuning.rs`, `diffpair.rs` and `main.rs` are
unchanged since that commit), and KiCad 10.0.1's source.

**fastroute's tuning** (`crates/fr-engine/src/tuning.rs`, `crates/fastroute/src/main.rs`):

- **Tune file.** A `group NAME tolerance=T [target=L]` line, then net patterns. `*` is a
  wildcard, `#` starts a comment, and words are split at white space.
- **Target.** The target is `target=`, or the group's longest net. Every net shorter than
  `target − tolerance` gets meanders, aimed at `target − tolerance/2`.
- **It only lengthens.** A net longer than the target is never shortened. A net of length 0
  (unrouted) is skipped.
- **Meanders.** They are square serpentines with 90° corners, amplitude 4 mm down to 0.3 mm,
  on the longest straight segments first. Each one is kept only if a trace 5 µm wider has no
  clearance violation.
- **What it never touches.** Locked tracks (KiCad `fix`) and footprint copper.
- **It reports to the log only.** Tuning writes nothing to the `--report` JSON.
- **When it runs.** Tuning, and pair-skew matching, run only if no stop was requested. Konnect
  never sends `--max-time`, and `--no-time-limits` (a candidate switch) turns the job timeout
  off, so tuning runs at the end of every pass that ends normally. A pass Konnect kills at its
  deadline writes no tuned session.
- **Via lengths.** With `--layer-heights` a via adds
  `heights[highest layer] − heights[lowest layer]`, over the layers its traces use there.
  One difference of heights cannot equal KiCad's distance for every layer pair (L7).
- **Lengths end at pad centres.** fastroute measures a net from pad centre to pad centre.

**KiCad 10.0.1:**

- **What a length is** (`length_delay_calculation.cpp`; `drc_test_provider_matched_length.cpp`,
  lines 407 to 418): tracks and arcs, plus each via's stackup height when
  `use_height_for_length_calcs` is on, plus pad-to-die lengths. Track parts inside a pad are cut
  off (`OptimiseTracesInPads`), and a via counts only between the layers its tracks use
  (`OptimiseViaLayers`).
- **Via height** (`board_stackup.cpp`, `GetLayerDistance`): from the first copper layer through
  the second, both copper thicknesses included. A board without a stackup gets KiCad's default
  (`BuildDefaultStackupList`): 35 µm copper, 10 µm mask, and the rest of the board thickness
  split evenly between the dielectrics.
- **Its DRC checks `.kicad_dru` rules** for `length` and `skew` (`within_diff_pairs` too), as
  `length_out_of_range` and `skew_out_of_range`.
- **What a violation says.** Its text names the rule and holds the actual length, printed with
  4 decimals in mm: `formatMsg`, `"(%s min length %s; actual %s)"`.
- **Ignored checks are skipped.** A check set to `ignore` gets error limit 0, so DRC skips it
  (`drc_engine.cpp`, lines 915 to 923).

**Konnect:**

- **Gates and the rank key** (`route_judge.rs`).
  - G2 lets a run pass while a `length (min …)` rule is still broken: the unrouted baseline
    already breaks it once per net.
  - The rank key counts meander length in `signal_mm`, and meander corners in `right_angles`
    and `copper_quality`.
  - `place_judge.rs` copies `route[3..9]` out of the routing key.
- **Konnect never evaluates KiCad's expression language.** `custom_rules.rs` opens with
  "KiCad compiles expressions", and `routing_bounds` refuses conditioned rules.
- **So a board with a KiCad length rule cannot be routed today.** `routing_bounds` admits
  only unconditioned `clearance` and `track_width` minima. `export_specctra_dsn` refuses any
  other custom rule (`pcb_export.rs`, lines 909 to 914), and so does the apply (L19).
- **Reading the stackup from the saved file has a precedent:** `trace_current.rs`,
  `stackup_copper`. `board_stackup.rs` reads it only through IPC.
- **The DSN export refuses unlocked routing.** Locked tracks and vias go out as fixed wiring.
- **The design intent** is closed (`deny_unknown_fields`) at `schema_version` 1. Its
  interfaces hold `differential_pairs`, which Row DP sends to fastroute.

**Public boards with a real bus:**

| Board | Layers | What it has |
|---|---|---|
| `video` | 4 | the 32-bit RAM data bus `/DQ0`…`/DQ31`; a stackup of 35 µm copper and 0.48 mm dielectrics |
| `cm5_minima` | 6 | MIPI D-PHY lanes: 4 data pairs plus a clock pair, per lane |

`vme-wren` has DDR4 length rules, but they are `fromTo` rules (fastroute has no from-to length),
and the board is 70 MB. It is not used.

## The flow of one call (what changes)

```
route_specctra_dsn(dsn_path, ses_output_path, effort?, engine? = "fastroute", fastroute_path?)
 ... as today (ROUTE_SEARCH_DESIGN.md), then, in search_and_write:
 a. pairs: route_pairs::plan (D23), now with each pair's skew= from max_skew_mm (L13)
 b. lengths: route_lengths::plan (intent groups + pair skews, resolved against the DSN;
    refusal before routing if a group cannot be applied; layer heights; KiCad's own
    length/skew rules listed); tune.txt written when there is a group
 c. every routing pass: as today, plus --layer-heights when the call has a resolved pair
 d. a Tune pass after the hold sequence when there is a group: from the held session,
    autorouter and optimizer off, --tune; kept only if it changes no other net (L6)
 e. the judge: as today, then a KiCad length probe on a copy of the last board when
    anything is checked (L8); gate G4 (L9); rank key with G4 after G3 (L10)
 f. the result: length_matching (L15), status with G4 (L11), next steps (L16)
```

A call whose intent declares no length group and no `max_skew_mm` has no Tune pass, no
probe, and no `tune.txt`. Its command lines are DP's, plus `--layer-heights` when it has a
pair. Its rank changes only where the board's own `.kicad_dru` has `length` or `skew` rules
(L9, L18).

## Decisions taken in this design (with the reason)

| # | Decision | Reason |
|---|---|---|
| L1 | **Schema.** Two optional fields, still `schema_version` 1. `interfaces[].length_groups[]` = `{name, nets[], tolerance_mm, target_mm?, notes?}`. `interfaces[].differential_pairs[].max_skew_mm?`. A record without them is written byte for byte as before | Like D23, lengths live where the interface's nets live. A group belongs to an interface (a byte lane, a bus). Optional fields keep every existing record valid. The installed build refuses a record that uses them (`deny_unknown_fields`), which is correct: it cannot honour them |
| L1a | **Validation**, refused before any write. Each group net is one of its interface's nets. A net is in at most one length group of the record. A group without a target has at least 2 nets. `tolerance_mm > 0`. `target_mm > tolerance_mm`. `max_skew_mm > 0`. A differential pair has both nets in the same group, or neither | A net in two groups has two windows, and a half-listed pair has an unmatched partner. Both are mistakes better caught when written than after a 40-minute search |
| L2 | **Meaning.** Lengths are KiCad's: tracks, plus vias by stackup when the project counts them, plus pad-to-die, minus track parts inside pads. Without a target (skew mode), every net is within `tolerance_mm` of the group's longest net: KiCad's `skew (max …)`. With a target, every net is within `target_mm ± tolerance_mm`: KiCad's `length (min …) (max …)`. A pair's `max_skew_mm` is \|L₊ − L₋\| ≤ max | These are KiCad's own two rule forms, so a group can be written back as a KiCad rule (L16), and KiCad's DRC agrees with the judge. A symmetric window is all KiCad's DDR-style rules need (min / opt / max with opt in the middle); a tighter in-group skew is a smaller tolerance |
| L3 | **Source** (owner decision 1). Tuning comes from the design intent only. KiCad's own `length` and `skew` rules are not translated. KiCad's DRC checks them on every judged run (G4), and the result lists them (`length_matching.kicad_rules`). The `kicad-pcb` skill tells the AI to mirror each one into a length group. No precedence is needed: both kinds are gates, so a net under both must meet both, and a conflict shows as a G4 failure naming both | Konnect never evaluates KiCad's expression language. A partial reader (net class, net name, `inDiffPair`, as fastroute's own KiCad plugin does) would tune a different net set from the one KiCad checks, and silently, for conditions it misreads. The gate is exact whichever source tunes |
| L4 | **Names** are resolved as D23 resolves a pair's (`route_pairs::resolve_name`): the exact DSN net, else the one net ending in `/name`. Unlike D23, **a group or a declared skew that cannot be applied refuses the call before anything is routed.** The causes: an unresolved or ambiguous name; a name with white space, `#`, `*`, `?`, a quote or a backslash; a DSN name that is not a net of the saved board; a checked net equal to another board net when letter case is ignored; a `max_skew_mm` pair that DP left out. A design intent that exists but cannot be used also refuses when its text holds `"length_groups"` or `"max_skew_mm"`; otherwise D23's note stands. The refusal names the group, the net and the fix | A pair left out still routes as two working nets. A length group left out is a requirement silently not met. With no routing done, a refusal costs nothing, and the AI fixes one name and calls again. The excluded characters are what the tune file (split at white space, `#` comment, `*` wildcard) and KiCad's `==` (`*` and `?` wildcards, quoted string) cannot carry exactly. KiCad's `==` also ignores letter case (`libeval_compiler.cpp`, `VALUE::EqualTo`), so a probe rule for `/Clk` would also catch `/CLK` |
| L5 | **The tune file** holds `lt1`, `lt2`, … (intent order) and DSN names only, never intent text. fastroute leaves alone every net inside its own window, so Konnect gives it **half** the intent's tolerance (`FASTROUTE_WINDOW_SHARE` = 0.5). Skew mode: `tolerance=T/2` (fastroute lengthens every net more than T/2 below the longest, aiming at T/4 below). Target mode: `target=L+T/2 tolerance=T` (it lengthens every net below `L − T/2`, aiming at `L`, the middle of the window). After the groups comes one trailing group per pair inside a group, `lt<g>p<k>`, with `tolerance=` half the pair's `max_skew_mm`, else 0.1 mm | fastroute's window is one-sided: it acts below target − tolerance and aims at target − tolerance/2. A net it leaves just inside its own window can fall outside KiCad's, because the two measures differ by the track inside the pads (L8, owner decision 3). Half the tolerance is that margin; the price is a few more meanders. Tuning a group lengthens each net of a pair on its own, so the pair's skew can grow up to the group's tolerance. The trailing group restores it, as fastroute processes groups in file order |
| L6 | **A Tune pass** comes after the hold sequence (Full → Fallback → Polish), only when the call has a group. It starts from the held session (`--initial-session`) with the optimizer off. `--router.autorouter.ignore_net_classes=` lists every DSN class, so the autorouter routes nothing. It sends `--tune`, and `--layer-heights` when there are heights, and never `--pairs`. It is kept when it succeeds, leaves no more connections open than its input, and changes the copper of no net outside the tune file. Copper is compared as geometry, not text: per net, segments by layer and width, collinear touching pieces merged and ends rounded to 1 µm, and vias by padstack and position. Otherwise the pass is `discarded` or `failed` with its reason, and the run keeps its input session. A Tune pass killed at the deadline ends the run (D4). Routing passes never get `--tune`. Before any body is written, step 0 of the verification proves on the real binary that a Tune pass routes nothing and keeps untouched nets equal under this comparison | Routing stays exactly as measured in Row BN. Tuning runs once, on the final wiring: a tune inside Full would be undone by Polish's optimizer, and repeated on every pass. The other-net check is the guard that the pass did only tuning. It compares geometry because fastroute's session import may split or merge segments it never moved, and a text compare would then discard every Tune pass. A later calibration, if the verification asks for one (owner decision 3), changes only this pass's file |
| L7 | **Layer heights.** Konnect computes them from the saved board: the `(setup (stackup …))` copper and dielectric thicknesses, sublayers summed, mask, silk and paste skipped; or KiCad's default stackup when there is none. They follow the DSN's layer order. Height(top copper) = 0, and height(layer i) = everything above it plus its own copper. They are sent only when `use_height_for_length_calcs` is on (absent: on, KiCad's default): on the Tune pass, and on every routing pass when the call has a resolved pair. A DSN layer missing from the stackup sends none, with a note | With these heights, a via from the top layer is exactly KiCad's distance. A via between two lower layers is short by one copper thickness (35 µm at 1 oz): one difference of heights cannot match KiCad's distance for every pair of layers. The judge measures KiCad's exact value, so this error only moves fastroute's aim. fastroute reads the heights only when it measures a net (`net_length`: pair skew and tuning), so a routing pass of a call without a pair keeps DP's exact command line. The precedent for reading the saved file is `trace_current.rs`; the route call works on files, so IPC is not available |
| L8 | **The judge measures with KiCad** (the length probe). After the audit, on a copy of the last judged board in `run-<i>/lengths`, Konnect writes probe rules only to the `.kicad_dru`. Each probe rule is `__konnect_len_NNNN`: `length (min 2000mm)`, `severity error`, condition `A.NetName == '<net>'`, one per checked net. Every DRC severity the project lists, except `length_out_of_range`, is set to `ignore`, and KiCad skips those tests (a key the project does not list runs at its default: slower, never wrong). It runs `kicad-cli pcb drc --units mm` with no refill and no save, on the run's own KiCad settings copy. Each net's length is the probe violation's actual value; the violation is found by its rule name and its items' `[net]`. A net with no entry, or one at 2000 mm or longer, is `unmeasured` (a G4 line). A probe that cannot run fails the judge | "Measure as KiCad does" is best done by KiCad. Pad clipping, via layer optimisation, pad-to-die, arcs and the default stackup are KiCad's code, not a copy that can drift. KiCad keeps a constraint as a 32-bit count of nanometres (`drc_rule.h`, `MINOPTMAX<int>`; `KiROUND` clamps), so the largest minimum it can hold is 2147.48 mm; 2000 mm is below that and above any real net. The actual is the number of the message that is not 2000, whatever language KiCad prints the words in. A separate copy keeps the board's own rules out of the probe: a probe rule would override a board rule for the same net |
| L9 | **Gate G4, length.** (a) Every intent group meets its window on the probe's lengths. (b) Every pair with `max_skew_mm` meets it. (c) KiCad's own DRC on the judged board has 0 `length_out_of_range` and 0 `skew_out_of_range`, absolute, not against the baseline. G2 stops counting those two keys | "Best-of-N never prefers a run that breaks a length rule": a KiCad length rule is one too. Against the baseline, G2 let a still-short net pass, because the unrouted board breaks every `min` rule already. One breach is reported by one gate |
| L10 | **Rank key** (13 entries, was 10): G1, G2, G3, **G4 failed**, **intent miss** (the sum of every measured net's distance outside its window, in 0.01 mm steps; an unmeasured net sorts last), **KiCad's own length and skew violations**, then today's measures: acute corners, signal length steps, power length steps, right angles, vias off the pours, minus copper quality, route seconds. `KEY_QUALITY = 6..12` names the measures the placement key copies. The placement key gains G4 failed, the intent miss and the KiCad count after G3 (16 → 19 entries) | Within the same earlier gates, a run that meets every length rule beats one that misses. Among runs that all miss, the smallest miss wins. The KiCad rules come after the intent's because Konnect tuned only the intent's. Tuned nets stay in the signal length: in skew mode a shorter longest net means less meander everywhere, and in target mode every run carries the same length. Meander corners stay counted: they rise with meander length, so they agree with the length steps |
| L11 | **Status.** G4 joins D15 and D21. A failed G4 (with G1 passing) gives `complete_with_gate_failures`. The judged status of D21 needs all four gates | A caller that checks `status == "complete"` must never accept a board whose lengths fail |
| L12 | **An impossible target is refused before routing.** If `target_mm + tolerance_mm` is below a net's shortest possible length, nothing can be done, because fastroute never shortens a net. The shortest possible length is the largest pad-to-pad centre distance of the net, minus both pads' circumradii (half the pad diagonal), never below 0. A net that owns a zone on the board is not checked here: a pour can join its pads with no track. A target too long for the free space is not predicted. It shows after the search as a G4 miss per net | The bound is conservative: KiCad cuts track parts inside pads, and a net's tracks span its farthest pads unless a zone joins them. So it never refuses a target that can be met. Whether meanders fit cannot be known before routing |
| L13 | **Pairs.** Half of a pair's `max_skew_mm` becomes its `skew=` in DP's pairs file (fastroute's coupled-aware skew matching on every routing pass; the margin of L5), and the judge checks the full value. A pair without one keeps fastroute's default of 0.1 mm (D23), and is not probed or checked. A pair inside a group (owner decision 2) is routed coupled (DP). The Tune pass then lengthens each of its nets to the group's window, and the trailing pair group restores its skew. The judge checks the window, and the skew if declared. The result marks the pair `tuned net by net` | fastroute has no coupled meander, so there are two choices: tune net by net, or fail the group on every run. A byte lane's strobe pair, or a D-PHY clock pair, must meet the lane's length. fastroute leaves alone a pair whose skew is within its `skew=`, so sending the full value would leave no room for the pad-edge gap |
| L14 | **`engine: "freerouting"`** with groups or skews: it routes as today, and the result carries `length_matching_note` ("Freerouting does not tune lengths; the groups were neither tuned nor checked; route with engine fastroute"). It is not refused | Freerouting stays available as an engine (Row BN). The note keeps the gap visible |
| L15 | **The result's `length_matching`.** Present when the intent declares a group or a skew, or the board's `.kicad_dru` has a `length` or `skew` rule. Its fields: `groups` (interface, name, file name, window, each net with its DSN name, its KiCad length, its range and its miss, the longest net, pass), `pairs` (skew, max, pass, group), `layer_heights` (sent, source, values, note), `kicad_rules` (name, constraints, condition, listed and not evaluated), `kicad_violations`, the winner's `tune_pass` row, `pairs_tuned_net_by_net`, and `note`. `search.runs` rows carry G4 and the miss | The AI needs to know what was asked, what was measured, and what it did not check, without reading `search.json` |
| L16 | **Next steps, no write.** When a group or skew is checked, `completion.next_steps` gives the KiCad rule text for each one, named by the group's file name: `(rule "1kopper_len_lt1" (constraint skew (max 0.5mm)) (condition "A.NetName == '/DQ0' \|\| …"))`, or `length (min …) (max …) (opt …)` in target mode. It suggests adding the rules with `create_custom_drc_rule` after `apply_specctra_ses`, so KiCad keeps checking them after hand edits; L19 lets such rules stay in a board that is exported again. On a G4 miss: relax the tolerance, give the short nets room, or route at a higher effort | Row S6 rule: no hidden write path. The judge's checks end with the call; a KiCad rule keeps checking afterwards. After the apply, because the apply refuses a project whose rules changed since the export |
| L17 | **No probe change** for `--tune` or `--layer-heights`. The candidate x18 has both (its `--help`). A build without them fails its passes with fastroute's own "unknown option" error, which the result quotes | D23 added `--pairs` the same way |
| L18 | **What changes for a call without groups.** A board whose `.kicad_dru` holds `length` or `skew` rules (and nothing else the DSN cannot project) can now be exported (L19), where it was refused. Its runs are then gated by G4(c), so one can get `complete_with_gate_failures`, and the result lists its rules (L15). A call with resolved pairs also gets `--layer-heights` (L7). Every other call is unchanged | These follow from L7, L9 and L19. A run that breaks a length rule of the board is not called complete |
| L19 | **KiCad length and skew rules no longer block the DSN.** `custom_rules::routing_bounds`, which `export_specctra_dsn`, `apply_specctra_ses` (`Snapshot::verify_saved`) and the S7 planner share, skips a rule whose every constraint is `length` or `skew`; that rule's `condition`, `layer` and `severity` clauses are then not read. Every other rule is projected or refused as today, including a rule that mixes a length constraint with a clearance one | Today any conditioned rule refuses the export (`routing_bounds` admits only unconditioned `clearance` and `track_width` minima; `pcb_export.rs`, lines 909 to 914), so a board with a KiCad length rule cannot be routed through Konnect at all, and G4(c) and L16 could never run. A length or skew rule changes nothing the DSN carries (no clearance, no width), so skipping it keeps the projection exact. Konnect still never evaluates the condition (L3) |

## Decisions for the owner

1. **Where length groups come from.** **Recommended:** the design intent alone says what is
   tuned. KiCad's own length and skew rules are checked by KiCad's DRC on every run (a run that
   breaks one fails G4), but never translated. Today such a rule blocks the DSN export
   altogether; L19 lets it through, because it changes nothing the DSN carries. The result lists them, and the AI mirrors each
   into an intent group. *Alternative:* Konnect also reads simple `.kicad_dru` conditions
   (`A.NetClass ==`, `A.NetName ==`, `A.inDiffPair`) and tunes them itself, as fastroute's
   KiCad plugin does. Reason for the recommendation: Konnect never evaluates KiCad's rule
   language, so a partial reader could tune different nets from the ones KiCad checks.
2. **A differential pair inside a length group** (a DDR strobe, a D-PHY clock). **Recommended:**
   allowed. fastroute lengthens each net of the pair on its own (it has no coupled meander), a
   trailing pair group restores the pair's skew, and the judge checks both. The result marks
   the pair "tuned net by net", which means coupling is not held along the meanders.
   *Alternative:* refused until fastroute can tune a pair as one. Reason: refused, such a group
   fails its length on every run.
3. **fastroute as it is, or changed first.** fastroute measures to pad centres, while KiCad
   measures to pad edges, so its aim can be off by a fraction of a pad on each end (L7, L8).
   Konnect gives fastroute only half of each tolerance, keeping the other half as margin for
   this (L5). Its meanders also have square corners. **Recommended:** build Row LT on
   fastroute x18 unchanged.
   The verification measures the pad-edge gap per net and the corner count. A fastroute row
   (lengths cut at pad edges as KiCad does, 45° meander corners) starts only if the measurement
   shows a tolerance missed on a board where it fits, or on his word for the corners.
   *Alternative:* change fastroute first. Reason: measurement first. The judge's gate is exact,
   so the gap can only cost passing runs, never pick a wrong board.

## Modules

New files in `crates/konnect-core/src/`:

| File | What |
|---|---|
| `tools/route_lengths.rs` | the call's length plan: the intent's groups and skews, resolved against the DSN; refusals; the tune file; KiCad's own rules listed; the result's fields and next steps |
| `layer_heights.rs` | copper layer heights from the saved board's stackup or KiCad's default stackup, in DSN layer order |
| `length_probe.rs` | the judge's KiCad length probe, the length check, G4's lines and the key's length part |
| `tools/route_lengths_tests.rs`, `layer_heights_tests.rs`, `length_probe_tests.rs` | tests |

Changes to existing items (each shown in the skeletons where it compiles as an addition, else
listed here):

| Where | Change |
|---|---|
| `tools/design_intent.rs` | `DiffPair.max_skew_mm`, `Interface.length_groups`, `LengthGroup`, `validate_lengths` called from `validate`; the `set_design_intent` description names the two fields |
| `tools/route_pairs.rs` (DP) | `DeclaredPair` and `ResolvedPair` gain `max_skew_mm: Option<f64>`; `file_text` writes ` skew=<route_lengths::fastroute_skew_mm>` after `gap=` |
| `custom_rules.rs` | `routing_bounds` skips a rule for which `is_dsn_neutral` holds (L19); its tests gain the length-rule cases |
| `specctra.rs` | `dsn_layer_names`, `dsn_class_names` |
| `fastroute.rs` | `CallFiles` gains `layer_heights: Option<&'a str>`, sent by `pass_arguments` as `--layer-heights=` after `--pairs`; the runner fills it for routing passes only when the call has a resolved pair (L7). `run_pass`'s process part moves, unchanged, into `run_arguments`, which `run_tune_pass` shares. `route_held` gains `tune: Option<TuneInputs<'_>>`, and calls `tune_held` after the hold sequence. `Held.kept` may be `tune` |
| `route_judge.rs` | `JudgeContext.lengths: Option<Arc<LengthChecks>>`; `Measures.lengths: Option<LengthMeasures>`; `Gates.g4` and `all_pass` with it. `gates` skips `length_probe::counts_in_g2`-false keys in G2 and fills G4 from `length_probe::g4_lines`. `judge_with` runs `judge_lengths` after the audit. `Key = [i64; KEY_LEN]`; `rank_measures` builds the L10 layout |
| `place_judge.rs` | the key copies `route[KEY_QUALITY]`, and adds G4, the miss and the KiCad count after G3 (`Key = [i64; 19]`) |
| `tools/route_fastroute.rs` | `search_and_write` builds the length plan after the pairs plan, refuses with `route_lengths::refusal_result`, writes `tune.txt`, sets `judge.lengths`, and gives the runner `tune`, `classes`, `tune_nets` and `layer_heights`. `result_json` and `no_winner_result` add `length_matching` and the next steps. `judged_status` reads the four gates. `run_rows` adds G4 and the miss |
| `tools/integration.rs` | the Freerouting path adds `length_matching_note` (L14) |
| `crates/konnect/assets/skills/kicad-pcb/SKILL.md`, `docs/API_MIGRATIONS.md`, `1-KOPPER.md`, `docs/ROUTE_SEARCH_DESIGN.md` (a D24 line pointing here), `docs/CREATION_HANDOFF.md` | guidance: declare groups in the intent; mirror KiCad's length rules; read `length_matching` |

No new tool and no new argument: the registry count is unchanged.

## Skeletons

Compiled with `cargo check -p konnect-core --tests` on a scratch checkout of DP's `5448e46d`
with these blocks pasted in (see "Verification of this document"), then removed. Bodies are
`todo!()`.

### `tools/design_intent.rs` (changed items)

```rust
/// Two nets of one interface routed as a pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffPair {
    pub positive: String,
    pub negative: String,
    /// Row LT: the largest difference allowed between the two nets' lengths, in mm,
    /// measured as KiCad measures them. Sent to fastroute as the pair's `skew=` and
    /// checked by the judge. Absent: fastroute's own default (0.1 mm), not checked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_skew_mm: Option<f64>,
}

/// Row LT: nets of one interface routed to matched lengths
/// (docs/LENGTH_TUNING_DESIGN.md, L1 and L2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LengthGroup {
    /// Unique within its interface. Free text: the tune file never holds it.
    pub name: String,
    /// Nets of this interface; each net is in at most one length group of the
    /// record.
    pub nets: Vec<String>,
    /// mm, greater than zero. Without `target_mm`: every net within this of the
    /// group's longest net (KiCad's `skew (max …)`). With it: every net within
    /// `target_mm` ± this (KiCad's `length (min …) (max …)`).
    pub tolerance_mm: f64,
    /// mm, greater than `tolerance_mm`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

// `Interface` gains, after `differential_pairs`:
//     #[serde(default, skip_serializing_if = "Vec::is_empty")]
//     pub length_groups: Vec<LengthGroup>,

/// Every reason the record's length groups and pair skews cannot be used, each
/// prefixed with where it is (`interfaces[i].length_groups[j].nets[k]: …`), as
/// [`validate`] reports. `validate` appends these to its own list:
/// - a group's `name` passes `name_ok` and is unique within its interface;
/// - `nets` is not empty, each net passes `net_ok`, is one of the interface's
///   `nets`, appears once in the group and in no other length group of the record;
/// - a group without `target_mm` has at least two nets;
/// - `tolerance_mm` is finite and greater than zero; `target_mm`, when set, is
///   finite and greater than `tolerance_mm`;
/// - each pair's `max_skew_mm`, when set, is finite and greater than zero;
/// - for every pair of the record, its two nets are in the same length group, or
///   neither is in one.
fn validate_lengths(intent: &DesignIntent) -> Vec<String> {
    let _ = intent;
    todo!()
}
```

### `specctra.rs` (added)

```rust
/// Row LT: the layers of a DSN's `structure`, as the DSN names them, in file order,
/// which is how fastroute numbers them. Refused when a layer has no
/// `(property (index N))` or the file order is not the index order: a height list
/// read against the wrong layers would count vias wrongly without any error.
/// Keywords match in any letter case; a DSN that does not parse is an error.
pub(crate) fn dsn_layer_names(dsn: &str) -> Result<Vec<String>> {
    let _ = dsn;
    todo!()
}

/// Row LT: the net classes of a DSN's `network` (`(class NAME net …)`), in file
/// order, each once: what the Tune pass lists in
/// `--router.autorouter.ignore_net_classes`. A class name holding a comma cannot be
/// listed (fastroute splits the value at commas); it is left out, and the Tune
/// pass's other-net check (L6) still guards its nets.
pub(crate) fn dsn_class_names(dsn: &str) -> Result<Vec<String>> {
    let _ = dsn;
    todo!()
}
```

### `custom_rules.rs` (added)

```rust
/// Row LT (docs/LENGTH_TUNING_DESIGN.md, L19): the constraints that change nothing
/// the DSN carries. A rule whose every constraint is one of these is skipped by
/// `routing_bounds`, its `condition`, `layer` and `severity` clauses unread; a rule
/// that mixes one of these with any other constraint is projected or refused as
/// before.
pub(crate) const DSN_NEUTRAL_CONSTRAINTS: [&str; 2] = ["length", "skew"];

/// Whether `rule` is skipped by `routing_bounds` (L19): it has at least one
/// `constraint` clause, every one of them is a [`DSN_NEUTRAL_CONSTRAINTS`] kind, and
/// its other clauses are only `condition`, `layer` and `severity`.
pub(crate) fn is_dsn_neutral(rule: &Rule) -> bool {
    let _ = rule;
    todo!()
}
```

### `layer_heights.rs`

```rust
//! Copper layer heights for fastroute's `--layer-heights`
//! (docs/LENGTH_TUNING_DESIGN.md, L7), so that fastroute counts a via's length close
//! to how KiCad does when it tunes.
//!
//! KiCad's via length between copper layers A and B (`BOARD_STACKUP::
//! GetLayerDistance`) runs from A through B, both copper thicknesses included.
//! fastroute adds `height[B] − height[A]`. With height(top) = 0 and height(i) =
//! everything above layer i plus its own copper, a via from the top layer is exact
//! and a via between two lower layers is one copper thickness short. The judge
//! measures with KiCad itself (`length_probe.rs`), so this only moves fastroute's
//! aim.
//!
//! Read from the saved board's text, as `trace_current.rs` reads copper thickness:
//! the route call works on files, and IPC is not available there.

use konnect_sexp::SexpNode;
use serde_json::Value;
use std::collections::BTreeMap;

/// KiCad's default copper thickness, mm (`BOARD_STACKUP_ITEM::
/// GetCopperDefaultThickness`).
pub(crate) const DEFAULT_COPPER_MM: f64 = 0.035;
/// KiCad's default solder mask thickness, mm.
pub(crate) const DEFAULT_MASK_MM: f64 = 0.01;
/// KiCad's default board thickness when `(general (thickness …))` is missing, mm.
pub(crate) const DEFAULT_BOARD_MM: f64 = 1.6;

/// Where the heights came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeightSource {
    /// The board's own `(setup (stackup …))`.
    BoardStackup,
    /// KiCad's default stackup for a board that defines none
    /// (`BOARD_STACKUP::BuildDefaultStackupList`).
    KicadDefault,
}

/// One copper or dielectric item of a stackup, top to bottom. Mask, silk and paste
/// are not items: KiCad's distance skips them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StackItem {
    /// The copper layer's name (`F.Cu`, `In1.Cu`, …); `None` for a dielectric.
    pub layer: Option<String>,
    /// The item's thickness, mm, every sublayer summed.
    pub thickness_mm: f64,
}

/// What the call sends as `--layer-heights`, and why.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LayerHeights {
    /// One height per DSN layer, in DSN order, mm; `None` when nothing is sent.
    pub heights_mm: Option<Vec<f64>>,
    pub source: Option<HeightSource>,
    /// Why nothing is sent (the project does not count via heights; a DSN layer is
    /// not in the stackup; the board does not parse), or what was assumed.
    pub note: Option<String>,
}

impl LayerHeights {
    /// The switch value, `0,0.55,1.065,1.58`: each height in mm, rounded to the
    /// micrometre, without trailing zeros; `None` without heights.
    pub(crate) fn switch_value(&self) -> Option<String> {
        todo!()
    }

    /// The result's `length_matching.layer_heights`: `sent`, `source`
    /// (`board_stackup` or `kicad_default`), `heights_mm`, and `note` when set.
    pub(crate) fn json(&self) -> Value {
        todo!()
    }
}

/// Whether KiCad counts via heights in lengths: the project's
/// `board.design_settings.rules.use_height_for_length_calcs`; `true` when the project,
/// the key or the file is missing (KiCad's default, `m_UseHeightForLengthCalcs`).
pub(crate) fn use_height_for_length_calcs(project_text: Option<&str>) -> bool {
    let _ = project_text;
    todo!()
}

/// The copper and dielectric items of `(setup (stackup …))`, top to bottom, in file
/// order; each item's thickness is the sum of its `thickness` entries (a dielectric
/// with `addsublayer` has several). `None` when the board has no stackup, or a
/// copper or dielectric item has no positive finite thickness (then KiCad's default
/// is not used either: the caller sends no heights and says why).
pub(crate) fn stackup_items(board: &SexpNode) -> Option<Vec<StackItem>> {
    let _ = board;
    todo!()
}

/// KiCad's default stackup for `board`: its copper layers from `(layers …)` in
/// stack order (F.Cu, In1.Cu, …, B.Cu), each 35 µm; between each two, a dielectric
/// of `(board thickness − 35 µm × copper count − 10 µm × masks) / (copper count −
/// 1)`, masks counted from `F.Mask` and `B.Mask` in `(layers …)`, board thickness
/// from `(general (thickness …))` else 1.6 mm. An error when the board has no
/// copper layer or the dielectric would not be positive.
pub(crate) fn default_stackup(board: &SexpNode) -> anyhow::Result<Vec<StackItem>> {
    let _ = board;
    todo!()
}

/// Each copper layer's height, mm (L7): the top copper layer 0, every other the
/// sum of the items above it plus its own thickness.
pub(crate) fn copper_heights_mm(items: &[StackItem]) -> BTreeMap<String, f64> {
    let _ = items;
    todo!()
}

/// KiCad's exact via distance between two copper layers of `items`, mm: from the
/// upper one through the lower one, both copper thicknesses included; 0 for the same
/// layer; `None` when either is not a copper item. For the tests and the
/// verification's residual report (how far fastroute's via count is from KiCad's).
pub(crate) fn kicad_layer_distance_mm(items: &[StackItem], first: &str, second: &str) -> Option<f64> {
    let _ = (items, first, second);
    todo!()
}

/// The heights of a call. Nothing is sent, with a note, when the project does not
/// count via heights, the board does not parse, or a DSN layer is not a copper
/// layer of the stackup used. The stackup is the board's own, else KiCad's
/// default (with a note saying so). The result is in `dsn_layers` order
/// (`specctra::dsn_layer_names`).
pub(crate) fn layer_heights(board_text: &str, project_text: Option<&str>, dsn_layers: &[String]) -> LayerHeights {
    let _ = (board_text, project_text, dsn_layers);
    todo!()
}

#[cfg(test)]
#[path = "layer_heights_tests.rs"]
mod tests;
```

### `tools/route_lengths.rs`

```rust
//! The length groups of a fastroute call (docs/LENGTH_TUNING_DESIGN.md, Row LT).
//!
//! The groups come from the design intent alone (`interfaces[].length_groups`, and
//! `differential_pairs[].max_skew_mm` for a pair's own skew), never from KiCad's
//! custom rules, which KiCad's own DRC checks on every judged run (L3). Each name is
//! resolved against the exported DSN's nets as D23 resolves a pair's; a group that
//! cannot be applied refuses the call before anything is routed (L4). The plan
//! writes the `--tune` file (L5), carries the layer heights (L7), and gives the
//! judge what it checks (`length_probe::LengthChecks`).

use crate::layer_heights::LayerHeights;
use crate::length_probe::{LengthChecks, LengthMeasures};
use crate::mcp::protocol::CallToolResult;
use crate::tools::design_intent::DesignIntent;
use crate::tools::route_pairs::PairsPlan;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The tune file's name in the call's folder.
pub(crate) const TUNE_FILE: &str = "tune.txt";

/// fastroute's own `--pair-skew` default, mm: the tolerance of a trailing pair group
/// for a pair whose intent gives no `max_skew_mm` (L5, L13).
pub(crate) const FASTROUTE_PAIR_SKEW_MM: f64 = 0.1;

/// The share of an intent tolerance fastroute is given (L5, L13): fastroute leaves
/// alone every net inside its own window, so the rest is the margin for the gap
/// between its measure and KiCad's.
pub(crate) const FASTROUTE_WINDOW_SHARE: f64 = 0.5;

/// What a net name in the tune file or a probe condition may not hold (L4):
/// fastroute splits a tune line at white space, cuts it at `#` and reads `*` as a
/// wildcard; KiCad's `==` reads `*` and `?` as wildcards, and a quote or a backslash
/// would end or escape the condition's string.
pub(crate) const UNSAFE_NAME_CHARS: &[char] = &['#', '*', '?', '\'', '"', '\\'];

/// A group as the design intent declares it, with its interface's name.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DeclaredGroup {
    pub interface: String,
    pub name: String,
    pub nets: Vec<String>,
    pub tolerance_mm: f64,
    pub target_mm: Option<f64>,
}

/// A pair's declared skew, as the intent names its nets.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DeclaredSkew {
    pub positive: String,
    pub negative: String,
    pub max_skew_mm: f64,
}

/// What a group's nets must meet, in KiCad's length (L2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Window {
    /// Every net within `tolerance_mm` of the group's longest net.
    Skew { tolerance_mm: f64 },
    /// Every net within `target_mm` ± `tolerance_mm`.
    Target { target_mm: f64, tolerance_mm: f64 },
}

impl Window {
    /// The group's `target=` (none in skew mode) and `tolerance=` in the tune file
    /// (L5), with s = [`FASTROUTE_WINDOW_SHARE`]: skew `(None, s·T)`; target
    /// `(Some(L + s·T), 2s·T)`. fastroute lengthens every net below its target −
    /// tolerance and aims at target − tolerance / 2, so in target mode it lengthens
    /// every net below `L − s·T` and aims at `L`.
    pub(crate) fn fastroute_values(&self) -> (Option<f64>, f64) {
        todo!()
    }

    /// The range a net's KiCad length must lie in, mm, both ends inclusive: skew
    /// `[longest − T, longest]`, target `[L − T, L + T]`. `longest_mm` is the group's
    /// longest measured net (read only in skew mode).
    pub(crate) fn range_mm(&self, longest_mm: f64) -> (f64, f64) {
        let _ = longest_mm;
        todo!()
    }
}

/// A net of a group: its intent name and its DSN name, which is also the board's
/// (the SES importer maps names one to one).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GroupNet {
    pub intent: String,
    pub dsn: String,
}

/// A group fastroute tunes and the judge checks.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedGroup {
    pub interface: String,
    pub name: String,
    /// `lt<N>`, N from 1 in intent order: the only group name the tune file holds.
    pub file_name: String,
    pub nets: Vec<GroupNet>,
    pub window: Window,
    /// Each resolved pair whose two nets are in this group: DSN names and the
    /// tolerance of its trailing pair group ([`fastroute_skew_mm`] of its
    /// `max_skew_mm`, else [`FASTROUTE_PAIR_SKEW_MM`]).
    pub pairs: Vec<(String, String, f64)>,
}

/// A pair whose skew the judge checks (it has `max_skew_mm`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GatedPair {
    pub positive: String,
    pub negative: String,
    pub dsn_positive: String,
    pub dsn_negative: String,
    pub max_skew_mm: f64,
    /// The group both nets are in, if any (intent name).
    pub group: Option<String>,
}

/// A `length` or `skew` rule of the board's own `.kicad_dru`: listed, never
/// evaluated (L3).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct KicadLengthRule {
    pub name: String,
    /// Each `length` or `skew` constraint as the file writes it, white space folded.
    pub constraints: Vec<String>,
    /// The rule's condition text as written, when it has one.
    pub condition: Option<String>,
}

/// Why the call is refused before routing, with the step that fixes it (L4, L12).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LengthRefusal {
    pub reason: String,
    pub next_step: String,
}

/// What a call does about lengths.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LengthPlan {
    pub groups: Vec<ResolvedGroup>,
    pub pairs: Vec<GatedPair>,
    pub heights: LayerHeights,
    pub kicad_rules: Vec<KicadLengthRule>,
    /// Why the board's `.kicad_dru` could not be listed, when it exists and does not
    /// parse.
    pub kicad_rules_note: Option<String>,
    /// Set when the design intent exists and cannot be used: nothing was read.
    pub note: Option<String>,
}

impl LengthPlan {
    /// True when nothing is tuned or checked from the intent (no group, no gated
    /// pair): then there is no tune file, no Tune pass and no probe.
    pub(crate) fn is_empty(&self) -> bool {
        todo!()
    }

    /// The `--tune` file (L5), or `None` without a group: a header comment, then
    /// each group as `group lt<N> tolerance=<t>[ target=<l>]` (`t` and `l` from
    /// `Window::fastroute_values`) with its DSN nets on indented lines, then each
    /// group's trailing pair groups (`group lt<N>p<K> tolerance=<skew>` with the
    /// pair's two nets). Numbers in mm without a trailing `.0`. No intent text
    /// appears in the file.
    pub(crate) fn tune_file_text(&self) -> Option<String> {
        todo!()
    }

    /// The DSN names of every net the tune file lists: the nets the Tune pass may
    /// change (L6).
    pub(crate) fn tune_nets(&self) -> BTreeSet<String> {
        todo!()
    }

    /// What the judge checks: every group's DSN nets with its window, every gated
    /// pair with its DSN names.
    pub(crate) fn checks(&self) -> LengthChecks {
        todo!()
    }

    /// True when the result carries `length_matching` (L15): a group, a gated pair,
    /// a listed KiCad rule, or a note.
    pub(crate) fn reported(&self) -> bool {
        todo!()
    }

    /// The result's `length_matching` (L15), with the winner's measures when there
    /// is a judged winner: `groups`, `pairs`, `layer_heights`, `kicad_rules`,
    /// `kicad_violations` (from `winner_violations`, KiCad's own DRC counts of
    /// `length_out_of_range` and `skew_out_of_range`), `tune_pass` (the winner's
    /// row), `pairs_tuned_net_by_net`, and `note` and `kicad_rules_note` when set.
    pub(crate) fn fields(
        &self,
        winner: Option<&LengthMeasures>,
        winner_violations: Option<&BTreeMap<String, usize>>,
        tune_pass: Option<&Value>,
    ) -> Vec<(&'static str, Value)> {
        let _ = (winner, winner_violations, tune_pass);
        todo!()
    }

    /// `completion.next_steps` lines (L16): the KiCad rule text of every checked
    /// group and pair ([`kicad_rule_suggestion`], [`kicad_pair_rule_suggestion`])
    /// with `create_custom_drc_rule`; and, on a miss, what can change it.
    pub(crate) fn next_steps(&self, winner: Option<&LengthMeasures>) -> Vec<String> {
        let _ = winner;
        todo!()
    }
}

/// Every length group of every interface, in file order.
pub(crate) fn declared_groups(intent: &DesignIntent) -> Vec<DeclaredGroup> {
    let _ = intent;
    todo!()
}

/// Every pair with `max_skew_mm`, in file order.
pub(crate) fn declared_skews(intent: &DesignIntent) -> Vec<DeclaredSkew> {
    let _ = intent;
    todo!()
}

/// The skew fastroute is given for a declared maximum (L13): `max_skew_mm` ×
/// [`FASTROUTE_WINDOW_SHARE`]. DP's pairs file writes it as `skew=`, and a trailing
/// pair group as its `tolerance=`.
pub(crate) fn fastroute_skew_mm(max_skew_mm: f64) -> f64 {
    let _ = max_skew_mm;
    todo!()
}

/// Whether an unusable design intent's raw text declares lengths: it holds the key
/// `"length_groups"` or `"max_skew_mm"`. Then the call is refused (L4); otherwise
/// D23's note stands and the call routes without the intent.
pub(crate) fn declares_lengths(raw: &str) -> bool {
    let _ = raw;
    todo!()
}

/// Whether a resolved DSN name can be written to the tune file and a probe
/// condition: not empty, no white space or control character, none of
/// [`UNSAFE_NAME_CHARS`].
pub(crate) fn tune_token(name: &str) -> bool {
    let _ = name;
    todo!()
}

/// Resolves the groups and skews against the DSN's nets (`route_pairs::resolve_name`),
/// in order. `Err` on the first group or skew that cannot be applied (L4): a name
/// resolving to no net or several, a name that is not a [`tune_token`], a DSN name
/// not in `board_nets`, a checked net equal to another of `board_nets` when letter
/// case is ignored (KiCad's `==` ignores it), or a skew whose pair is not among
/// `pairs.resolved`. Each
/// group gets `lt<N>` and its pairs (from `pairs.resolved`, matched by intent
/// names); each skew becomes a [`GatedPair`] with the group both nets are in.
pub(crate) fn resolve(
    groups: &[DeclaredGroup],
    skews: &[DeclaredSkew],
    dsn_nets: &[String],
    board_nets: &BTreeSet<String>,
    pairs: &PairsPlan,
) -> Result<(Vec<ResolvedGroup>, Vec<GatedPair>), LengthRefusal> {
    let _ = (groups, skews, dsn_nets, board_nets, pairs);
    todo!()
}

/// The net names of the saved board's net table (`(net N "name")`), for the
/// one-to-one check of L4.
pub(crate) fn board_net_names(board_text: &str) -> anyhow::Result<BTreeSet<String>> {
    let _ = board_text;
    todo!()
}

/// Each net's shortest possible KiCad length, mm (L12): the largest distance between
/// two of its pads' centres, minus both pads' circumradii (half the pad diagonal),
/// never below 0; 0 for a net with fewer than two pads. From the saved board
/// (`konnect_sexp::board_geometry::BoardGeometry`). Only `nets` are computed, and a
/// net that owns a zone is left out (a pour can join its pads with no track).
pub(crate) fn shortest_possible_mm(board_text: &str, nets: &BTreeSet<String>) -> anyhow::Result<BTreeMap<String, f64>> {
    let _ = (board_text, nets);
    todo!()
}

/// The first target-mode group with a net whose shortest possible length is above
/// `target + tolerance`, as a refusal naming the group, the net, the shortest length
/// and the window (L12). Skew-mode groups are never refused here.
pub(crate) fn impossible_target(groups: &[ResolvedGroup], shortest: &BTreeMap<String, f64>) -> Option<LengthRefusal> {
    let _ = (groups, shortest);
    todo!()
}

/// The `length` and `skew` rules of a `.kicad_dru` (`custom_rules::parse`), in file
/// order, listed with their constraints and condition, not evaluated.
pub(crate) fn kicad_length_rules(dru_text: &str) -> anyhow::Result<Vec<KicadLengthRule>> {
    let _ = dru_text;
    todo!()
}

/// The KiCad rule that checks `group` as the judge did (L16), named by its file
/// name: `(rule "1kopper_len_lt1" (constraint skew (max <T>mm)) (condition
/// "A.NetName == '<net>' || …"))` in skew mode, `(constraint length (min <L−T>mm)
/// (max <L+T>mm) (opt <L>mm))` in target mode. The full intent tolerance, not
/// fastroute's share.
pub(crate) fn kicad_rule_suggestion(group: &ResolvedGroup) -> String {
    let _ = group;
    todo!()
}

/// The KiCad rule that checks a gated pair's skew: `(constraint skew (max <S>mm))`
/// on its two nets.
pub(crate) fn kicad_pair_rule_suggestion(pair: &GatedPair) -> String {
    let _ = pair;
    todo!()
}

/// Writes the plan's tune file to `<folder>/tune.txt` and returns its path, or
/// `None` (and writes nothing) when the plan has no group.
pub(crate) fn write_file(folder: &Path, plan: &LengthPlan) -> anyhow::Result<Option<PathBuf>> {
    let _ = (folder, plan);
    todo!()
}

/// The plan of a call for the board the DSN was exported from (`board`, its saved
/// text `board_text`), after the pairs plan:
/// 1. the design intent beside the board (absent: an empty plan; unusable: refused
///    when [`declares_lengths`] reads its raw text, else an empty plan with `note`,
///    as D23);
/// 2. `resolve` against `specctra::dsn_net_names` and [`board_net_names`];
/// 3. [`impossible_target`] with [`shortest_possible_mm`];
/// 4. heights: `layer_heights::layer_heights` with the board's `.kicad_pro` and
///    `specctra::dsn_layer_names`, only when the plan has a group or the pairs plan
///    has a resolved pair (else none, no note); routing passes send them only with a
///    resolved pair, the Tune pass whenever there are any (L7);
/// 5. [`kicad_length_rules`] of `<board>.kicad_dru` when it exists.
pub(crate) fn plan(board: &Path, board_text: &str, dsn_text: &str, pairs: &PairsPlan) -> Result<LengthPlan, LengthRefusal> {
    let _ = (board, board_text, dsn_text, pairs);
    todo!()
}

/// The refusal as a tool result: `invalid_configuration` on the design intent's
/// path, "Nothing was routed." plus the reason and the next step.
pub(crate) fn refusal_result(refusal: &LengthRefusal, intent: &Path) -> CallToolResult {
    let _ = (refusal, intent);
    todo!()
}

/// L14: the note the Freerouting path adds when the board's design intent declares a
/// length group or a pair skew; `None` otherwise (also when the intent is absent or
/// unusable).
pub(crate) fn freerouting_note(board: &Path) -> Option<String> {
    let _ = board;
    todo!()
}

#[cfg(test)]
#[path = "route_lengths_tests.rs"]
mod tests;
```

### `fastroute.rs` (added)

```rust
/// Row LT: the Tune pass's name and folder (docs/LENGTH_TUNING_DESIGN.md, L6).
pub(crate) const TUNE_PASS: &str = "tune";

/// What the Tune pass needs beyond a routing pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TuneInputs<'a> {
    /// The call's `--tune` file (`tools/route_lengths.rs`).
    pub tune: &'a Path,
    /// Every net class of the DSN (`specctra::dsn_class_names`): the autorouter
    /// ignores them all, so it routes nothing.
    pub classes: &'a [String],
    /// The DSN names of the nets the tune file lists: the only nets whose wiring the
    /// pass may change.
    pub nets: &'a std::collections::BTreeSet<String>,
}

/// `--layer-heights=<value>` from `layer_heights::LayerHeights::switch_value`.
pub(crate) fn layer_heights_argument(value: &str) -> String {
    let _ = value;
    todo!()
}

/// The Tune pass's command line, every setting once, in this order: `-de <dsn>`,
/// `-do <ses>`, `--report=<report>`, [`PRESET`], `--router.optimizer.enabled=false`,
/// `--router.autorouter.ignore_net_classes=<classes, comma separated>` (left out
/// when there is none), `--router.copper_to_edge_clearance_um=` when `hints` has
/// one, the two thread settings at [`THREADS`], [`CANDIDATE_SWITCHES`],
/// `--x.par_via_amps=` when `files` has it, `--tune=<file>`, `--layer-heights=`
/// when `files` has it, and `--initial-session=<initial>`. Never `--pairs`,
/// `--live`, `--multi-start`, `--max-time`, `--diagnose` or a neck width.
pub(crate) fn tune_pass_arguments(
    dsn: &Path,
    ses: &Path,
    report: &Path,
    hints: &RoutingHints,
    files: CallFiles<'_>,
    tune: &TuneInputs<'_>,
    initial: &Path,
) -> Vec<String> {
    let _ = (dsn, ses, report, hints, files, tune, initial);
    todo!()
}

/// Runs the Tune pass in `folder` exactly as [`run_pass`] runs a pass (the same
/// sandbox, environment, log and deadline; `run_arguments` after the split), with
/// [`tune_pass_arguments`].
#[allow(clippy::too_many_arguments)] // the parts of one pass, each read once
pub(crate) async fn run_tune_pass(
    binary: &Binary,
    folder: &Path,
    dsn: &Path,
    hints: &RoutingHints,
    files: CallFiles<'_>,
    tune: &TuneInputs<'_>,
    initial: &Path,
    deadline: Instant,
) -> std::result::Result<PassOutcome, PassError> {
    let _ = (binary, folder, dsn, hints, files, tune, initial, deadline);
    todo!()
}

/// One net's copper in a session, as geometry (L6): its straight pieces by layer and
/// width, collinear pieces that touch merged into one, each piece's ends rounded to
/// 1 µm and ordered; its vias by padstack and rounded position. Both lists sorted.
/// Two sessions that split one trace into different pieces compare equal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct NetCopper {
    /// (layer, width in µm, [x1, y1, x2, y2] in µm).
    pub pieces: Vec<(String, i64, [i64; 4])>,
    /// (padstack, [x, y] in µm).
    pub vias: Vec<(String, [i64; 2])>,
}

/// A session's copper by net: every `(net NAME (wire …) (via …))` of
/// `(routes (network_out …))`, read with the session's own `resolution`, as
/// [`NetCopper`]. Keywords in any letter case; a session that does not parse, or a
/// wire that is not a `path`, is an error.
pub(crate) fn copper_by_net(ses_text: &str) -> Result<BTreeMap<String, NetCopper>> {
    let _ = ses_text;
    todo!()
}

/// The nets outside `nets` whose copper differs between two sessions (one appears
/// in only one of them, or their [`NetCopper`] differ), sorted: the Tune pass's
/// guard (L6).
pub(crate) fn changed_outside(
    before: &str,
    after: &str,
    nets: &std::collections::BTreeSet<String>,
) -> Result<Vec<String>> {
    let _ = (before, after, nets);
    todo!()
}

/// The Tune pass after the hold sequence (L6), in `folder/tune`: from `held.ses`.
/// - Killed by `deadline`: `PassError::Killed` ends the run (D4).
/// - Failed (an error, no session, an unreadable report): a `failed` row with its
///   reason; the run keeps `held`.
/// - It leaves more open than `held.unrouted`: `discarded` ("opened N").
/// - [`changed_outside`] names a net, or cannot read either session: `discarded`
///   ("changed the copper of <nets>", at most 5 named, then "and N more").
/// - Else `kept`: `held.ses` becomes the Tune pass's session, `held.kept` is
///   [`TUNE_PASS`], and `held.unrouted` its count.
///
/// The row joins `held.passes` with `name`, `outcome`, `unrouted`, `seconds` and
/// `reason`, as every pass row; `held.seconds` adds the pass's time.
#[allow(clippy::too_many_arguments)] // the parts of one pass, each read once
pub(crate) async fn tune_held(
    binary: &Binary,
    folder: &Path,
    dsn: &Path,
    hints: &RoutingHints,
    files: CallFiles<'_>,
    tune: &TuneInputs<'_>,
    held: Held,
    deadline: Instant,
) -> std::result::Result<Held, PassError> {
    let _ = (binary, folder, dsn, hints, files, tune, held, deadline);
    todo!()
}
```

### `length_probe.rs`

```rust
//! The judge's length check (docs/LENGTH_TUNING_DESIGN.md, L8 to L10): KiCad
//! measures every checked net on a copy of the judged board, and Konnect compares
//! those lengths with the design intent's groups and pair skews.
//!
//! The copy's `.kicad_dru` holds probe rules only, one `length (min 2000mm)` rule
//! per net, so that KiCad reports every checked net with its actual length; every
//! other DRC check the project lists is set to `ignore`, which KiCad skips. The board's own rules are
//! checked by the judge's ordinary DRC (G4(c)), never here: a probe rule would
//! override a board rule for the same net.

use crate::tools::cli::DrcReport;
use crate::tools::route_lengths::Window;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
/// KiCad's rule keys of length and skew violations: G4 counts them, G2 does not
/// (L9).
pub(crate) const LENGTH_RULE_KEYS: [&str; 2] = ["length_out_of_range", "skew_out_of_range"];
/// The rank key's step for the intent miss, mm (L10).
pub(crate) const MISS_STEP_MM: f64 = 0.01;
/// How far past a window's end a length may be and still pass, mm: half of the
/// last digit KiCad prints (4 decimals), so that a net printed on the bound passes.
pub(crate) const PRINTED_HALF_DIGIT_MM: f64 = 0.000_05;

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
}

impl LengthChecks {
    /// Every net to probe, each once, sorted.
    pub(crate) fn nets(&self) -> Vec<String> {
        todo!()
    }

    /// True when nothing is checked (then the judge runs no probe).
    pub(crate) fn is_empty(&self) -> bool {
        todo!()
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

/// What the length check found on one judged run (`Measures.lengths`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct LengthMeasures {
    pub groups: Vec<GroupLengths>,
    pub pairs: Vec<PairSkew>,
    /// Checked nets the probe gave no entry for.
    pub unmeasured: Vec<String>,
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
    let _ = nets;
    todo!()
}

/// The probe copy's `.kicad_pro`: the project's own text with every key of
/// `board.design_settings.rule_severities` set to `"ignore"` except
/// `length_out_of_range`, which is `"error"` (added when missing). A check the
/// project does not list runs at KiCad's default severity: slower, never wrong.
/// Everything else, `use_height_for_length_calcs` included, is kept. An error when
/// the text is not a JSON object.
pub(crate) fn probe_project_text(project_text: &str) -> anyhow::Result<String> {
    let _ = project_text;
    todo!()
}

/// `kicad-cli pcb drc --output <out> --format json --units mm <board>`: no refill,
/// no save, no schematic parity.
pub(crate) fn probe_drc_args<'a>(out: &'a str, board: &'a str) -> Vec<&'a str> {
    let _ = (out, board);
    todo!()
}

/// The actual length in a probe violation's description, mm: every number followed
/// by `mm` (a decimal comma read as a point), less the one within 0.001 of
/// [`PROBE_MIN_MM`]; `None` unless exactly one is left (a net at 2000 mm or longer
/// reads `None` and is unmeasured). The words around the numbers are never read, so
/// KiCad's language does not matter. Its tests use a report captured from a real
/// `kicad-cli` run, not a hand-written message.
pub(crate) fn actual_mm(description: &str) -> Option<f64> {
    let _ = description;
    todo!()
}

/// The probe's lengths by net, and the nets it gave no entry for. A violation counts
/// when its rule is `length_out_of_range`, its description names a probe rule, and
/// one of its items names that rule's net as `[<net>]`. Two entries for one net, or
/// an entry whose item names another net, is an error (the probe is not trusted).
pub(crate) fn parse_probe(report: &DrcReport, nets: &[String]) -> anyhow::Result<(BTreeMap<String, f64>, Vec<String>)> {
    let _ = (report, nets);
    todo!()
}

/// Runs the probe for `nets` on the judged board `judged`: `scratch_board::
/// copy_project` into `folder`, the board's text written there, the `.kicad_dru`
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
) -> anyhow::Result<(BTreeMap<String, f64>, Vec<String>, f64)> {
    let _ = (kicad_cli, config_home, judged, folder, nets);
    todo!()
}

/// The measures of a run (L9): each group's nets against `Window::range_mm` (skew
/// mode against the longest measured net), each pair's skew against its maximum, a
/// net past a window's end by at most [`PRINTED_HALF_DIGIT_MM`] counted inside; the
/// misses summed.
pub(crate) fn check(
    checks: &LengthChecks,
    lengths: &BTreeMap<String, f64>,
    unmeasured: Vec<String>,
    probe_s: f64,
) -> LengthMeasures {
    let _ = (checks, lengths, unmeasured, probe_s);
    todo!()
}

/// Whether G2 counts a KiCad rule key: every key but [`LENGTH_RULE_KEYS`].
pub(crate) fn counts_in_g2(rule: &str) -> bool {
    let _ = rule;
    todo!()
}

/// KiCad's own length and skew violations on the judged board (`Measures.
/// violations`), summed.
pub(crate) fn kicad_rule_violations(violations: &BTreeMap<String, usize>) -> usize {
    let _ = violations;
    todo!()
}

/// G4's lines (L9), one per failure: each net outside its window ("group 'dq'
/// (ram): /DQ7 is 41.20 mm, 0.62 mm below its window 41.82–42.32 mm"), each
/// unmeasured net, each pair over its skew, and KiCad's own violations by key
/// ("KiCad: 2 length_out_of_range violation(s) of the board's own rules"). Empty
/// when everything passes; `measures` is `None` when the call checks nothing.
pub(crate) fn g4_lines(measures: Option<&LengthMeasures>, violations: &BTreeMap<String, usize>) -> Vec<String> {
    let _ = (measures, violations);
    todo!()
}

/// The rank key's length part (L10), positions 3 to 5: G4 failed (0 or 1), the
/// intent miss in [`MISS_STEP_MM`] steps (`round`; `i64::MAX` when a net is
/// unmeasured; 0 without measures), KiCad's own violations.
pub(crate) fn key_part(
    g4_failed: bool,
    measures: Option<&LengthMeasures>,
    violations: &BTreeMap<String, usize>,
) -> [i64; 3] {
    let _ = (g4_failed, measures, violations);
    todo!()
}

#[cfg(test)]
#[path = "length_probe_tests.rs"]
mod tests;
```

### `route_judge.rs` (added)

```rust
/// Row LT: the length of the rank key (docs/LENGTH_TUNING_DESIGN.md, L10): G1, G2,
/// G3, G4, intent miss, KiCad's length violations, then the ten-entry key's
/// measures from acute corners on. `Key` becomes `[i64; KEY_LEN]`.
pub(crate) const KEY_LEN: usize = 13;

/// Where the quality measures sit in the key (acute corners to minus copper
/// quality): what the placement key copies (it read `3..9` before Row LT).
pub(crate) const KEY_QUALITY: std::ops::Range<usize> = 6..12;

/// Row LT, step 6 of the judge (after the audit): when `checks` is not empty,
/// `length_probe::probe_lengths` on the last board, in `<run dir>/lengths`, with the
/// run's own KiCad settings, then `length_probe::check`. `None` when nothing is
/// checked. A probe error fails the judge.
pub(crate) async fn judge_lengths(
    ctx: &JudgeContext,
    checks: &crate::length_probe::LengthChecks,
    config: Option<&Path>,
    board: &Path,
    run_dir: &Path,
) -> Result<Option<crate::length_probe::LengthMeasures>> {
    let _ = (ctx, checks, config, board, run_dir);
    todo!()
}
```

## Tests, each guard with a negative control

| Guard | Test | Negative control |
|---|---|---|
| schema stays compatible | a record without the new fields reads and writes back byte for byte; one with them round-trips | an unknown key inside a group is refused (`deny_unknown_fields`) |
| validation (L1a) | each rule refuses with its path: a net outside the interface, a net in two groups, a skew group of one net, tolerance ≤ 0, target ≤ tolerance, a pair split across groups or half in one, `max_skew_mm` ≤ 0 | the same record with the rule met is accepted |
| names (L4) | each case refuses before routing, naming group, net and fix, and the search folder holds no run: an unresolved or ambiguous name; a resolved name with `#`, `*`, `?`, a quote, a backslash or white space; a DSN name that is not a board net; a checked `/Clk` beside a board net `/CLK`; a skew on a pair DP left out; an unusable intent whose text holds `"length_groups"` | the same intent with resolvable names plans and routes; an unusable intent without those keys routes with D23's note |
| DSN projection (L19) | a board whose `.kicad_dru` has a conditioned `length` rule and a conditioned `skew` rule exports, and its DSN equals the DSN of the same board without them; `apply_specctra_ses` accepts its session | a rule mixing `length` with `clearance`, and a conditioned `clearance` rule, are still refused with today's messages |
| tune file (L5) | skew line `tolerance=T/2`; target line `target=L+T/2 tolerance=T`; trailing pair groups after every group, with half of `max_skew_mm` or 0.1; no intent text; parsed back by a copy of fastroute's `parse_tune_file` rules (keys, `#`, white space) | with `FASTROUTE_WINDOW_SHARE` set to 1 the lines change (the test pins the share) |
| pairs file (L13) | `skew=` is half of `max_skew_mm`, after `gap=`; a pair without one has no `skew=` | DP's pairs tests without skews are unchanged |
| impossible target (L12) | a two-pad net with `target + tolerance` below the bound refuses | a target exactly at the centre distance is accepted (the bound is below it); a net that owns a zone is never refused |
| layer heights (L7) | sublayers summed; mask, silk and paste skipped; KiCad's default stackup formula on a board without one; DSN order; `use_height_for_length_calcs: false` sends none; every via from the top layer equals `kicad_layer_distance_mm`, every via between two lower layers is short by the upper layer's copper | a DSN layer missing from the stackup sends none, with a note; a DSN whose file order is not its index order is refused by `dsn_layer_names` |
| routing passes unchanged (L6, L7) | with a tune file in the call, Full, Fallback and Polish get no `--tune`; with a resolved pair they get `--layer-heights` after `--pairs` | a call with groups but no pair, and a call with neither, get exactly DP's routing command lines (DP's own tests unchanged) |
| Tune pass arguments | optimizer off, every class ignored, `--tune`, `--initial-session`, never `--pairs` or a neck width, each setting once | the Polish pass's line from `pass_arguments` differs from it in exactly those settings |
| Tune pass keep rule | fake binaries: one that tunes only group nets (kept); one that also moves another net's wire (discarded, the net named); one that opens a connection (discarded); one that fails (failed, the input kept); one that hangs (killed, the run ends) | the first fake without `--tune` support fails with its own message |
| `changed_outside` | identical sessions: none; a group net's wire moved: none; another net's wire or via moved, added or removed: named; one trace split into two collinear pieces: equal; letter case of keywords | a wire moved by 0.4 µm compares equal (rounding is the rule) and one moved by 2 µm differs |
| probe files (L8) | one rule per net, numbered, `severity error`, `min 2000mm`; every listed severity set to `ignore` except `length_out_of_range`; arguments: no refill, no save, `--units mm` | a project without a `rule_severities` object still gives a probe that measures (KiCad's defaults, only slower) |
| `actual_mm` and `parse_probe` | a report captured from a real `kicad-cli` run on a fixture with probe rules (stored as a test file); the same description with translated words; a decimal comma; the net cross-checked through `[net]` items; a missing net is unmeasured | a description holding only the probe's own number gives `None`; two entries for one net, or an entry naming another net, is an error |
| check and G4 (L9) | skew window, target window, pair skew: each passes on the bound (within half a printed digit) and fails one step past it; misses summed | changing one length by 0.01 mm across a bound flips its line |
| G2 and G4 split | a baseline with 3 `length_out_of_range` and a run with 2: G2 passes, G4 fails | before the change, the same run passed G2 (the test names that) |
| rank (L10) | G4 after G3; among G4 failures the smaller miss first, then fewer KiCad violations; existing rank tests read the new positions | without groups and KiCad rules, positions 3 to 5 are 0 for every run and today's order is unchanged; the placement key reads `KEY_QUALITY` |
| status (L11) | G4 failing gives `complete_with_gate_failures`; D21's judged status needs all four gates | a run passing all four keeps today's status |
| no change without groups (L18) | no `tune.txt`, no Tune pass row, no probe folder, no `length_matching` on a board without `.kicad_dru` length rules | the same board with one group gets all four |
| Freerouting (L14) | the note when the intent has a group | none without one |
| `asset_references` and `schema_parameter_usage` (konnect crate) | the skill's new lines name only real fields and tools | run early (memory: they cost a red gate on #63) |
| end to end (ignored test: real fastroute and kicad-cli) | a repository fixture with a 4-net bus at Fast: the winner passes G4, and KiCad's DRC with the suggested rule reports 0 skew violations | the same with an impossible target: G4 fails on every run, status `complete_with_gate_failures` |

Scoped `cargo llvm-cov` on the three new modules and the changed functions. Targeted
`cargo mutants` on the guards: `validate_lengths`, `resolve`, `declares_lengths`,
`impossible_target`, `is_dsn_neutral` and `routing_bounds`, `changed_outside` and
`copper_by_net`, `tune_held`'s keep rule, `actual_mm`, `parse_probe`, `check`, `g4_lines`,
`counts_in_g2` and `key_part`. Both run in an isolated checkout under the lane locks, as
`AGENTS.md` says.

## Verification plan

Public KiCad demos only, scratch copies, with `KICAD_CONFIG_HOME` set to a scratch folder.
Each call runs through this checkout's `target/release/konnect` by pipe, inside one comparable
lane hold. As in Row BN, every call records wall time, runs launched and judged, the winner's
gates and the protected files.

**Step 0, before any body is written** (scripts on the real fastroute x18 and kicad-cli, a
public fixture, nothing in the product). These three checks gate the build. A failing one
brings its decision back to the design.

1. **The Tune pass routes nothing.** fastroute runs from a complete held session with every
   class ignored, the optimizer off, and a group no net needs to change: every net's copper is
   equal under the L6 geometry comparison. If it is not, L6's guard is redesigned.
2. **The Tune pass tunes only its nets.** The same pass with a real group changes only that
   group's nets.
3. **The probe reads KiCad's numbers.** A probe rule of `min 2000mm` on the fixture gives
   one entry per net, and its actual length is read. The captured report becomes the unit
   tests' fixture.

**Board A, `video`** (4 layers, 35 µm copper, 0.48 mm dielectrics, via heights counted):

- **The copy.** The tracks and vias of the 32 nets `/DQ0`…`/DQ31` are deleted, and every
  other track and via is locked (the DSN export refuses unlocked routing).
- **Intent.** Interface `ram` with the 32 nets, and two groups:
  - `dq_low`: `/DQ0`…`/DQ15`, skew mode, tolerance 0.5 mm;
  - `dq_high`: `/DQ16`…`/DQ31`, target mode, tolerance 0.5 mm. The target is the longest of
    these nets in the control run's winner plus 5 mm, rounded up to 1 mm.
- **Calls.** Effort M with the intent, and the control: effort M without groups.

**Board B, `cm5_minima`** (6 layers):

- **The copy.** D-PHY lane 0: its 4 data pairs and its clock pair, 10 nets, unrouted.
  Everything else is locked.
- **Intent.** Interface `dphy0` with the 5 pairs, each `max_skew_mm` 0.05, and one skew group
  of the 10 nets, tolerance 0.15 mm.
- **Calls.** Effort M, and a control without the group.

**The measure of success.** Every item must hold:

1. **All gates.** On both boards the winner passes G1 to G4.
2. **KiCad agrees independently.** A copy of each winner's judged board gets the suggested
   KiCad rules (L16) in its `.kicad_dru`. `kicad-cli pcb drc` on it then reports 0
   `length_out_of_range` and 0 `skew_out_of_range`.
3. **Tuning touched only its nets.** No Tune pass was discarded for another net, counted over
   every run, and the winner's Tune pass was kept.
4. **The meanders add no DRC trouble.** The winner's own held session (before its Tune pass)
   is judged as well, by the same judge. No DRC type is worse after the Tune pass than before
   it. This compares one run with itself, so routing chance plays no part.

**Recorded for the owner** (not pass/fail):

- **Spread.** `dq_low`'s KiCad max − min, control against tuned.
- **The fastroute-to-KiCad gap per net** (owner decision 3):
  - fastroute's view: the winner's session, wire lengths plus heights;
  - KiCad's view: the probe;
  - reported as median and largest, per board.
- **Right angles** added by the meanders.
- **Probe and Tune pass seconds** per judged run, and the runs judged at M with and without
  groups.

**Negative controls:**

- **Too long a target.** `dq_high` with a target of 500 mm: every run fails G4, the status is
  `complete_with_gate_failures`, and the result names each short net and its miss.
- **A misspelt net.** The call is refused before routing, and the search folder holds no run.
- **A half-listed pair.** `set_design_intent` refuses it.

**Decision rule.**

- If item 1 fails only because the winner's misses match the pad-edge gap, owner decision 3
  comes back with the numbers.
- If item 2 disagrees with the probe, the probe is wrong, and the row stops.

**Not covered:**

- Windows and Linux.
- The owner's own boards. They come after his word, as Row BN's did.
- KiCad's time-domain (delay) tuning.

## Verification of this document

**Independent review** (Sonnet 5.5, high, 2026-11-20). It checked the claims against
fastroute's, KiCad's and Konnect's source, and reported 11 findings. Each was confirmed
against the source and is fixed in this text:

- **Two must-fix.**
  - A board with a KiCad length rule could not be exported at all: `routing_bounds` refuses
    it. L19 was added, and L18 and L16 now depend on it.
  - The probe's `min 999999mm` overflows KiCad's 32-bit nanometre constraint. It is now
    2000 mm (L8), and the parser's tests use a captured report.
- **Three should-fix.**
  - fastroute gets half of each tolerance and skew as margin (L5, L13).
  - An unusable intent that declares lengths now refuses (L4).
  - The Tune pass's guard compares geometry, and step 0 proves it on the real binary (L6).
- **Six notes**, also fixed:
  - KiCad's `==` ignores letter case (L4);
  - zone nets are left out of L12;
  - DRC is compared within the same run;
  - heights go on routing passes only with a pair (L7);
  - three test rows gained negative controls;
  - the rule name is now one form.

Confirmed correct by the review: L5's mapping as first written, KiCad's via distance and L7's
height math, the default stackup, KiCad's skew semantics, the rank-key indices, the `[net]`
item naming, and that x18's `--help` lists `--tune`, `--layer-heights` and `skew=`.

**Skeleton compile** (2026-11-20). The blocks above were pasted into a scratch checkout of
DP's `5448e46d` by a script that reads them out of this file:
- the three new modules, with empty test files;
- the `specctra.rs`, `custom_rules.rs`, `fastroute.rs` and `route_judge.rs` additions,
  appended;
- the `design_intent.rs` items, with the `length_groups` field added to `Interface`.

Then `cargo check -p konnect-core --tests` ran through `chunk.sh`
(`target/creation-research/placement-search/chunk.sh`, both lane locks held, 12:09 to 12:10).

- **Result:** exit 0, `Finished` in 48 s, 0 errors.
- **Warnings:** 86, every one `never used` or `never constructed`, each on an item of these
  skeletons. Nothing calls them yet.
- **Same text:** all 8 blocks of this section were found verbatim in the compiled files after
  the run.

The scratch checkout and its target folder were removed afterwards. The commit holds this
document and the handoff line only.

## What this design does not do

- **From-to lengths** (KiCad's `fromTo`): fastroute measures whole nets. A `fromTo` rule is
  still checked by KiCad's DRC (G4(c)).
- **Time-domain (delay) tuning:** KiCad 10's tuning profiles.
- **Coupled meanders for a pair, and 45° or rounded meander corners:** fastroute work, under
  owner decision 3.
- **Shortening a net:** only another run's routing can do it, and best-of-N picks it.
- **Translating `.kicad_dru` rules** (owner decision 1), or writing KiCad rules into the
  project (L16: suggested, not written).
- **Placement for length:** Row PS does not know about groups. Its candidates route through
  this call and are judged by it.
