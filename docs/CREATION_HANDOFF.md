# Creation handoff

The current state of the creation-side work: schematic, PCB and library tools. Anyone picking this up, human or agent, starts here. Update this file in the same commit that changes the state. The simulation track has its own file, [`SIMULATION_HANDOFF.md`](SIMULATION_HANDOFF.md).

**Last updated:** 2026-11-08. All 15 core creation PRs are merged. PR 11 (honest review and current audit) merged as [#42](https://github.com/yuvaltal912-sys/Konnect/pull/42) (`06a992c`), the first GitHub merge authored with the noreply address. PR 15 (ERC/DRC baselines) merged as [#43](https://github.com/yuvaltal912-sys/Konnect/pull/43). Catalogue: 259 registered tools plus 7 meta-tools (266). Next pickable: PR 16, the Specctra DSN profile widening (rank 32, owner decision Q2). The installed binary remains protected.

## Start here

1. Read `AGENTS.md` for the rules.
2. Create a branch from `main` for the work, unless "Now" names an open one to continue.
3. Read the PR row below, then the sections of `docs/CREATION_RESEARCH.md` it names (§3 has the evidence, §4 the sequence, §5 the owner's decisions).
4. Run the gate before changing anything, so you know the baseline. It is under "Verify" in `docs/SIMULATION_HANDOFF.md`.
5. If a step cannot run in your environment, such as `gh`, a push or `kicad-cli`, say so. Never skip it silently.

## The goal

Make Konnect's creation tools do what its agents are already told to do. The build and review agents are told to check that connectivity matches intent, and until PR 1 no tool could. The research ([`CREATION_RESEARCH.md`](CREATION_RESEARCH.md), from [`CREATION_RESEARCH_PROMPT.md`](CREATION_RESEARCH_PROMPT.md)) ranks 46 changes and orders the first 15 into PRs. Counts in it are from before PR 1; the registry is the source for current ones.

Owner decisions (2026-11-06, research §5):

| ID | Decision |
|---|---|
| Q1 | Konnect-owned placement and routing planners are allowed as a second opinion, never as the author. A planner returns a read-only plan; the AI compares it with its own result and adopts what is good through the normal editing tools. Planners start only after PRs 1, 7 and 8 provide the metrics |
| Q2 | Freerouting is the route path, after the core PRs (1 to 15). Widen the DSN profile (rank 32) first. Until then PR 5 documents it with its limits. **Amended 2026-11-06:** a focused part of rank 32 moves up as PR 2b, between PR 2 and PR 3, after a routing run on a real board went badly: the DSN export refused a rounded outline, Freerouting was not installed, and the AI hand-rolled its own router and left ground connections unrouted. PR 2b makes the DSN export accept rounded and arc outlines and roundrect pads, keep existing zones and pours as fixed copper, and gives an install path or clear setup guidance for Freerouting. More layers and back-side parts stay in rank 32 for later |
| Q3 | One design-intent record per project (rails, currents, interfaces, fab target). Ranks 12, 29 and 38 read from it. It gets its own PR before PR 11; PR 11 may take a per-call currents argument as an interim |
| Q4 | KiCad 10 variants and jobsets stay parked (`ROADMAP.md`) |
| Scope | macOS first. Windows and Linux are validated at the end of the series |

## PR sequence

One PR at a time, each on its own branch from `main`. Ask the owner before starting the next. The owner has sometimes approved two PRs in parallel when their code files do not overlap; that is their call, case by case.

| PR | Delivers | Depends on | State |
|---|---|---|---|
| 1 | Netlist read and intent compare: `get_schematic_netlist`, `compare_netlist` | none | Merged: [#19](https://github.com/yuvaltal912-sys/Konnect/pull/19) |
| 2 | Route safety: port upstream #901, stubs (#899) and `add_wire` T-dots (#900), report `nets_joined`; PR 1 proves no net changed | 1 | Merged: [#20](https://github.com/yuvaltal912-sys/Konnect/pull/20). Review fixes: merged, [#22](https://github.com/yuvaltal912-sys/Konnect/pull/22) |
| 2b | Freerouting on real boards: DSN export accepts rounded and arc outlines and roundrect pads and keeps zones and pours as fixed copper; install path or setup guidance | 2 | Merged: [#26](https://github.com/yuvaltal912-sys/Konnect/pull/26) |
| 3 | Response budget for ERC, DRC and list tools | none | Merged: [#21](https://github.com/yuvaltal912-sys/Konnect/pull/21). Leftovers (C24 stale-fill field, A30's other long results) are open under rank 3 |
| 4 | Inline renders as image blocks; region and side options | none | Merged: [#28](https://github.com/yuvaltal912-sys/Konnect/pull/28). Follow-ups: inline layer and library-item renders (rank 25), a generated difference image for `inline_diff` |
| 5 | Guidance fixes in the shipped skills and agents; Freerouting path documented. Also (2026-11-06): if the DSN export or Freerouting fails, stop and report the blocker instead of hand-rolling a router, and never finish with unrouted nets without saying so | 1, 2, 2b | Merged: [#29](https://github.com/yuvaltal912-sys/Konnect/pull/29) |
| 6 | Small schematic edits: mirror after placement, DNP and BOM flags, title block | none | Integrated from [#30](https://github.com/yuvaltal912-sys/Konnect/pull/30) in the creation alignment PR |
| 7 | Board geometry layer: `query_board_region`, true outline containment | none | Merged: [#25](https://github.com/yuvaltal912-sys/Konnect/pull/25). Follow-ups on the layer: free-space finder, copper islands, planned-copper clearance (C18, C23, C15) |
| 8 | Placement objective: wirelength, crossings, pin-level decoupling | 7 | Integrated from [#31](https://github.com/yuvaltal912-sys/Konnect/pull/31) in the creation alignment PR |
| 9 | Sync completion: field copy and orphan delete in `update_pcb_from_schematic` | none | Merged, first half: field copy and orphan delete: [#24](https://github.com/yuvaltal912-sys/Konnect/pull/24) |
| 9b | Sync completion, second half: in-place footprint replace and routed-pad net reassignment | 9 | Integrated from [#33](https://github.com/yuvaltal912-sys/Konnect/pull/33) in the creation alignment PR |
| Q3 | Design-intent record (rails, currents, interfaces, fab target), read by ranks 12, 29 and 38 | none | Merged: [#37](https://github.com/yuvaltal912-sys/Konnect/pull/37) (`ef4b1be`). PR 11 is unblocked |
| 10 | Library correctness: pin-pad check, symbol fields, ranked search | none | Integrated from [#32](https://github.com/yuvaltal912-sys/Konnect/pull/32) in the creation alignment PR |
| 11 | Honest review and current audit | 7, Q3 | Merged: [#42](https://github.com/yuvaltal912-sys/Konnect/pull/42) (`06a992c`). Follow-ups: IPC-2152 derating, voltage drop, current split, circuit-aware review (rank 29) |
| 12 | Routing batch and readback | none | Integrated from [#34](https://github.com/yuvaltal912-sys/Konnect/pull/34) in the creation alignment PR |
| 13 | DRC rule writer | none | Merged: initial slice [#35](https://github.com/yuvaltal912-sys/Konnect/pull/35), continuation [#38](https://github.com/yuvaltal912-sys/Konnect/pull/38). DSN accepts the explicit unconditional numeric clearance/width profile; general rule authoring is broader |
| 14 | Checkpoints and readiness report | none | Merged: [#39](https://github.com/yuvaltal912-sys/Konnect/pull/39) (`1e33c5f`) |
| 15 | ERC/DRC baselines | 3 | Merged: [#43](https://github.com/yuvaltal912-sys/Konnect/pull/43). Follow-up: read baselines in `run_design_review` (now unblocked) |
| 16 | Widen the Specctra DSN profile so Freerouting fits real boards: back-side parts, more than 2 copper layers, then a Freerouting version policy and a benchmark on KiCad's demo boards (rank 32: D7, B34, D24). PR 2b already did rounded/arc outlines, roundrect pads and zones as fixed copper | 2b, 13 | Not started |

## Next: what to pick

**Claim rule.** Pick a "Not started" row whose dependencies are all merged. Before writing code, push a first commit on your branch that changes that row's state to "In progress (branch `<name>`)" and open a draft PR, so no other agent takes it. For a partially integrated row, claim its continuation on a new branch and draft PR before implementation. If another row is "In progress", don't touch the code files it names.

**Rank 3 leftovers: In progress (branch `feat/creation-rank3-leftovers`).** C24 (`zone_fill_state` on DRC results) and A30's other long results (`batch_place_components`, `update_pcb_from_schematic`, `validate_for_manufacturing`, the batch wiring tools, `get_component_list`, `query_traces`).

**Recommended order**, after the owner approves the next implementation:

1. **PR 16**: widen the Specctra DSN profile (rank 32), the route path the owner chose for after the core PRs (Q2). Start with back-side parts and more than two copper layers on KiCad's demo boards; the live Freerouting run (below) is its acceptance check, so ask the owner before running it.
2. Smaller follow-ups that can run beside it, since their files don't overlap `pcb_export.rs`: read ERC/DRC baselines in `run_design_review` (PR 15 follow-up); keep tests out of the real call log (rank 35); the `cargo deny` advisories (its own PR).
3. Then the larger items: circuit-aware review (rank 29, reads the Q3 rails), board construction (rank 21), LCSC import (rank 33).

PR 13's rule editing, deletion and static expression workflow is complete within
its documented limits. Wider custom-rule conversion into DSN remains part of the
later routing-profile expansion; unsupported rule semantics refuse export.

**Checks still owed** (not blocking, but say so if your PR depends on them):

- The real Freerouting run, `freerouting_routes_the_rounded_board` (ignored). Java 21 is now installed at `/opt/homebrew/opt/openjdk@21` (the system `java` is still 11). The owner deferred the live run; ask before running it.
- PR 9 against a live KiCad: `crates/konnect/tests/live_kicad_tools.rs`, on a disposable board.
- PR 14 against a live KiCad: checkpoint, edit in the GUI, restore refused while open, restore after closing, on a disposable project.
- Windows and Linux for the whole series, at the end.

**Known traps.**

- `every_simulation_demo_flattens_or_names_its_export_failure` is flaky when another test run is using `~/Library/Caches/KiCad` at the same time (IBIS demo, "Cannot find U1.cache"). Rerun it alone.
- Use the noreply identity from `AGENTS.md`, including merge commits. Since the owner turned on email privacy (2026-11-08), GitHub merges work with `gh pr merge N --merge --match-head-commit <full sha> --author-email 274043415+yuvaltal912-sys@users.noreply.github.com` (first used for #42); check the merge commit's `%ae`. If GitHub refuses, stop and ask: never substitute a personal email or fast-forward `main`. Pushes are refused when the branch tip carries a personal email, so put a noreply commit on top.
- Two sessions edited `CREATION_HANDOFF.md`, `tool-directory.md` and `API_MIGRATIONS.md` at once and conflicted each time. Keep every PR's rows and entries when you resolve.
- `cargo deny check` is red on main since 2026-11-08: new RustSec advisories for `h2`, `rustls`, `webpki` and `pin-utils`, with no lockfile change. It needs its own PR; report it, don't fix it inside a feature PR.
- Three `freerouting_setup_tests` (fake Java probes) can fail under a full parallel `cargo test --workspace` and pass alone. Rerun them alone.

## PR 15, ERC/DRC baselines (merged, #43)

Three tools in the `verification` toolset (15 to 18), in `tools/check_baselines.rs`. Finding C4: agents diffed ERC/DRC reports by hand in Bash 18 times to prove an edit broke nothing.

- `save_check_baseline` runs ERC (`schematic`) or DRC (`board`, with `run_drc`'s `sync_live_board` and `refill_zones`) and stores every violation at `<project_dir>/.konnect/baselines/<erc|drc>/<name>.json`, beside the visual baselines. An existing name is refused before the check runs (a synced DRC saves the board) unless `overwrite`. Create-only or revision-conditional write, then read back.
- `compare_check_baseline` loads the baseline first (a wrong name runs nothing and lists the stored names), runs the same check and lists `new`, `severity_changed` and `resolved`, paged through the PR 3 helper, with `unchanged_count`, per-rule tallies and `verdict` (`regressed` when anything is new or more severe). `output` writes everything, unchanged included. Warns when the DRC options or the ERC's KiCad version differ from the baseline's.
- `list_check_baselines`: both kinds, newest first, `source_state` (`unchanged`, `changed`, `missing`); unreadable files listed apart.

Stable keys: a hash of check, DRC category, rule, ERC sheet and the sorted item identities. Severity, prose and positions are not in it. An item is its UUID only when the UUID is in the saved design (DRC: ownership enrichment resolved it on the board; ERC: it occurs in a project `.kicad_sch`). Otherwise it is its description with measured values (`0.15 mm`, `45°`) masked, which covers KiCad 8 boards whose pads have no saved UUID. Repeats are numbered `~2`, `~3`. A DRC category that only one run reported is listed in `categories_not_compared`, never as resolved. `run_erc` and the baselines now share `sch_export::erc_report`, so both refuse a sub-sheet the same way.

Verified on macOS: 23 unit tests over scratch projects with a stand-in `kicad-cli` (new/resolved/unchanged for ERC and DRC; a re-run reordered, moved, with items swapped and DRC prose re-measured matches exactly; made-up UUIDs; repeats; severity changes; missing categories; refusals before any run; damaged and renamed baseline files; a symlinked `.konnect`; 1,500 changes under the client cap with paging covering each once), 12 negative controls each caught by a test, and the ignored live test `live_rerun_of_an_unchanged_project_matches_its_baseline` with kicad-cli 10.0.8 on a scratch copy of the KiCad 10 `pic_programmer` demo (5 ERC and 5 DRC violations, every item keyed by UUID, re-run 0 new and 0 resolved). `cargo test -p konnect-core -p konnect` and clippy pass. Not verified: a live edit between save and compare, `sync_live_board` against a live KiCad (it reuses `drc::run`, already covered), a KiCad 8 board, Windows and Linux.

Follow-ups, not in this PR: read baselines in `run_design_review` (unblocked now that PR 11 is merged); a delete tool (overwrite covers replacement today); the documented edit loop in the skills and agents (rank C29).

## PR 11, honest review and current audit (merged, #42)

Ranks 14 and 12 (D15, D20, D17, C16). Merged as [#42](https://github.com/yuvaltal912-sys/Konnect/pull/42) (`06a992c`) from `feat/creation-honest-review-current-audit`. The claim-only commit and draft preceded implementation, from main 97b7059.

- **`audit_manufacturing` checks what it says.** The outline half was already fixed by PR 7. Gone: the "possible silkscreen overlap" warning (footprint anchors under 1 mm apart) and the trace-width check that read a `setup` key KiCad 6+ never writes, so it could never fire. New, in `tools/dfm.rs` over `BoardGeometry`: track widths, via drill/diameter/annular ring, through-hole drills and plated pad rings, via-in-pad on SMD pads (warning on its own net, error on another), track-to-track acid traps under 90° (junctions inside a same-net pad or via skipped and counted), and the `.kicad_pro` DRC minimums against the fab. Findings are grouped (per net and layer, per via size, per footprint) with example UUIDs in `location`. `checks` gives each check's coverage, `delegated_to_drc` what `run_drc` proves instead, `not_checked` what nothing covers. Limits: `fab_house`, else the design-intent fab target's house, else `jlcpcb`; the record's minimums win for that house, and `fab_limits` names each value's source. `PadGeom` gained `kind`, `size` and `drill`.
- **Classification by type** (`tools/component_class.rs`, D20). Power pins: KiCad `power_in`; names only on `passive`/`unspecified` pins; ground nets and power symbols skipped. Capacitors: library symbol, else `C<digit>` reference (`CR1` no longer counts). One SI parser (`100n`, `4u7`, `0.1µF`, `10uF/25V`, `2k2`, `4R7`), shared with `score_placement`'s decoupling families. The old bulk matcher's `µ` arms never matched; they do now. Findings carry `classified_by`.
- **`audit_trace_current`** (`tools/trace_current.rs`, `design_review` toolset, now 7 tools; 256 + 7 meta at the time). Reads `currents` and rail `max_current_a` from the Q3 record, checks every segment and via of those nets against IPC-2221 (k = 0.048 outer, 0.024 inner), copper from the record, else the board stackup, else 1 oz / 0.5 oz. It assumes every item carries the full net current and says so; a zone of the net on the same layer turns an error into a warning. Net names are matched against the board (`nets_not_on_board`), which Q3 left open. No record or no currents is a status, not an error; an unusable record is `invalid_configuration`. The per-call currents interim from Q3 was not needed.
- **`run_design_review`** runs it when a board is given; `design_review.trace_current` says whether it ran. A missing record does not make the review INCOMPLETE; an unusable one does.

Not done: IPC-2152 derating, current split across parallel paths, pour capacity, voltage drop, acid traps at track-to-pad edges or mid-segment T-junctions, solder-mask slivers, filled/capped via status (not in the board file).

Verified on macOS (2026-11-08): the gate (fmt, 3719 lib and integration tests with 0 failures, doc tests, clippy `-D warnings`, `cargo deny check licenses`, `fix-doc-counts`, release build); 33 new tests over scratch fixtures (SI parser, classifiers, each DFM check, fab-limit sources, IPC-2221 against the published chart, every trace-audit status) plus the review integration; a stdio run of the release binary through real dispatch (`audit_trace_current` and `audit_manufacturing` on a scratch project, unknown argument refused); 8 negative controls (ground skip, power-symbol skip, bulk tolerance, intent house match, zone softening, pad-junction skip, via annular check, RKM fraction), each caught by a test. Existing review fixtures pass unchanged. Not verified: a real KiCad board with recorded currents, a live KiCad, the `simulation_live` suite (not touched), Windows and Linux.

## PR 14, checkpoints and readiness report (merged, #39)

Four tools in the `project` toolset (starter kit, so no `load_toolset`), in `tools/checkpoints.rs`:

- `create_checkpoint` copies the saved design files to `<project_dir>/.konnect/checkpoints/<id>/` with a SHA-256 manifest. It builds in a hidden `.partial-*` folder, reads every copy back, re-reads the project and refuses (`stale_target`) if a save landed during the copy, then renames into place. Open KiCad documents are named in `locks_present`: their unsaved changes are not captured.
- `list_checkpoints`: newest first by millisecond time, each with changed/missing/added counts against the saved files. Malformed checkpoints are listed under `unreadable`.
- `restore_checkpoint` previews by default and applies only with the preview's revision, which binds the checkpoint, the subset and every target's current digest. It refuses while a target has a KiCad lock or is open over IPC, while the project lock is present, while a journal is pending, or for a board seen live this session. It verifies the stored copies, takes a `pre_restore` checkpoint (`undo_checkpoint`), writes through `commit_file_transaction`, and reads every file back. It never deletes files added since.
- `get_edit_readiness`: per file, lock state, open in KiCad, seen live, pending journals, and `edit_route` (`file`, `live`, `blocked`). It says once when KiCad does not answer over IPC (C13). It never clears a lock: KiCad locks carry no PID.

The shipped `konnect` skill now tells agents to check readiness before writing and to checkpoint risky work instead of zipping files by hand. `snapshot_project` no longer calls its PDFs a checkpoint.

Verified on macOS: 32 unit tests over scratch fixtures, including mock KiCads holding the board and schematic open or answering without documents, and a stdio run of the release binary on a scratch copy of the `pic_programmer` demo (checkpoint, preview, apply with readback, undo listed, locked restore refused), and 9 negative controls (project lock, revision, copy integrity, live route, IPC-open board, symlinked parent folder, root-sheet lock covering sub-sheets, AppleDouble `._*` files, journal verdict), each caught by a test. An independent review found five defects, all fixed here: a lock or conflict mid-commit was reported as "nothing written" (now claimed only when every target reads as before and no journal is left, otherwise `mutation_outcome_uncertain`); a symlinked folder inside the project let restore read or write outside it; a sub-sheet of an open schematic read as editable (KiCad locks only the root sheet); `._*` files broke checkpoints; and a pending journal could leave the verdict `ready`. Not verified: a live KiCad GUI, the mid-commit lock path and a save racing the copy (both guarded, neither has a test), restore with IPC timing out, Windows and Linux. KiCad 10's own local history (`.history`) is excluded, not read. So is the Q3 design-intent record (`.konnect/design_intent.json`): checkpoints hold KiCad design files only.

## Q3, the design-intent record (merged, #37)

One record per project at `<project_dir>/.konnect/design_intent.json`, in `tools/design_intent.rs`, served by two `config` tools (starter kit, so they are listed without `load_toolset`):

- `get_design_intent` returns the record, `source` (`file` or `absent`), a content `revision` (`sha256:` of the file text) and `missing_sections`, so a dependent result can name what was never recorded. Net names are stored as given and are not checked against the design (`net_names_checked: false`).
- `set_design_intent` replaces one section (`rails`, `currents`, `interfaces`, `fab_target`) and keeps the others. An existing record needs `expected_revision`; a missing or wrong one is `stale_target` and writes nothing. `dry_run` validates and returns the `candidate_revision`. Writes are conditional on the text that was read, then read back. The directory must hold a `.kicad_pro`, so a mistyped path leaves no stray folder.

The record is closed and typed (`schema_version` 1, `deny_unknown_fields`, sections checked for shape before serde, which would otherwise read a struct from a positional array). Refused: duplicate names, a net in two rails or in both a rail and `currents`, non-positive currents, impedances and fab sizes, tolerance outside (0, 100], a layer count other than 1 or an even 2 to 32, inner copper on a board of two layers or fewer, a via diameter not larger than its drill, differential pairs naming nets outside the interface, and a differential impedance with no pairs. A file that cannot be used is `invalid_configuration` for both tools and is never replaced.

The input schema leaves `value` open (`additionalProperties: true`) on purpose. Dispatch closes object schemas by default, which refused every `fab_target` object; unit calls bypass dispatch, so only the release-binary probe caught it. `the_dispatch_schema_admits_every_section_shape` now covers it.

Readers: rank 12 (current audit) takes rail and net currents, `max_temp_rise_c` and copper weights; rank 29 (circuit-aware review) takes rails with `source_ref`; rank 38 (net classes from the schematic) takes interfaces, pairs and impedances. `read_design_intent` is the crate-level entry point for them. The shipped review and PCB skills now point at `get_design_intent`. Not done here: checking net names against the schematic or board, and any reader.

## PR 6, small schematic edits (integrated, #30)

The claim-only commit and draft preceded implementation, from merged `main`.

- `rotate_schematic_component` accepts `rotation` and/or absolute `mirror`
  (`x`, `y`, `none`). Omitted transforms are preserved across every resolved
  placed unit. Field text and no-connects follow the transform, junctions are
  reconciled, and prospective and saved state are checked. Connected wires do
  not move; pin-aligning replacement and wire stretching remain separate work.
- `edit_schematic_component` accepts DNP/BOM/board/simulation inclusion booleans
  for all resolved units. Component mutation/get/list observations expose saved
  nested `flags`. Unrequested legacy simulation-token absence is preserved.
  New components copy resolved library defaults, including inheritance; an
  additional unit keeps consistent package choices. Missing and explicit-false
  simulation exclusion are equivalent for that check, while untouched units
  keep their existing token representation.
- New `set_schematic_title_block` in `sch_hierarchy` partially updates one saved
  sheet's title/date/revision/company/comments1–9. Targeted edits preserve other
  bytes, omitted values, unknown list children, indentation and CRLF. Invalid or
  ambiguous metadata, malformed child shapes, unsupported comment lines and
  implicit/multiple roots are refused. A revision/lock-protected write is
  followed by independent saved verification; failure then reports uncertainty.
- API migrations, tool directory and schematic guidance describe the changes.
  `cargo xtask fix-doc-counts` updates generated counts for the one added tool.
- Focused verification: 13 new component tests, 16 normal title-block tests and
  the ignored real KiCad SVG oracle passed, as did the existing library suite.
  All 57 removal controls caused actual failures and were restored: 25 title,
  32 component/library/schema, including one explicitly structural assertion of
  the redundant published mirror-type metadata. These are not model-behavior
  tests. The first full gate exposed the separate power-symbol constructor's
  missing flag copy and two exact served-response expectations; these were
  fixed, with a served power regression and its removal control. The corrected
  final workspace gate and release/stdio checks are recorded on #30.
- No installed-binary update, live-project edit, variant support or Claude
  runtime was used. Client GUI behavior, Windows/Linux and hosted CI remain
  unverified. CodeRabbit's prior-PR review is still blocked by authentication.

## PR 8, placement objectives (integrated, #31)

The claim-only commit and draft preceded implementation. This PR is independent
of PR 6 and adds no new tools; it upgrades the existing scorer on PR 7's layer.

- Routing proxies use deterministic per-net Euclidean pad-site MST length and
  strict interior crossings between different nets. `max_net_references`
  (2–256, default8) bounds distinct-reference fanout. Counts, raw values,
  exclusions, fixed work limits and bounded diagnostic samples are explicit.
  These are planar straight-airwire estimates, not actual routed/unrouted
  copper, shorts, DRC or a layer-aware routing plan.
- New advisory deductions use mean MST edge length / board diagonal, scaled
  by20 and rounded up (cap20), and crossings times2 (cap20). Geometry/outline
  hard verdicts retain precedence. Missing data/normalization, ambiguous
  identities, exceeded work budgets or unproven associations set
  `score_incomplete`; unavailable routing aggregates are null, not zero.
- Decoupling now uses matching non-ground pad anchors. Pad-adjacent caps may
  qualify on larger rails; farther associations need bounded fanout. Return
  values identify the pads/positions and basis. Connector waivers check every
  named terminal net using worst nearest-pair distance and existing mounting-
  side/through-stack evidence. This remains a proximity heuristic, not a proof
  of pin role, datasheet intent, arbitrary pad layers or routed current loops.
- New witness arrays share byte/item budgets while exact counts/distances
  remain evaluated. Scalar identifiers and legacy result lists are not a
  whole-response size guarantee; wider response budgeting remains rank3 work.
- Legacy row/refinement planners retain their existing geometry and retirement
  limits. A bbox-oriented row may correctly stop improving under the better
  judge. Incomplete before/after scores block application without hiding the
  diagnostic plan; applied replies disclose post-write score completeness too.
- Focused verification passed111 placement tests, including48 new tests and
  hand-calculated native-fixture expectations. All75 removal controls failed
  their intended tests and sources were restored:32 objective,28 decoupling,
  15 handler/schema/integration. QA identified and closed the new witness-array
  byte-budget gap. API migrations, tool directory and PCB guidance are updated.
  The final gate and release/scratch checks are recorded on #31.
- No live-project or installed-binary edit, real Freerouting job, autonomous
  placement/routing or Claude runtime was used. Client GUI behavior,
  Windows/Linux and hosted CI remain unverified.

## PR 4, render images and focused views (merged, #28)

The claim commit landed before code changes and the draft was opened against `main`.

- `render_schematic_png` with `inline: true` and `compare_visual_baseline` with
  `inline_diff: true` return JSON metadata followed by a real MCP PNG image block.
  The JSON base64 fields are removed; `docs/API_MIGRATIONS.md` gives the replacement.
  `inline_diff` still shows the current full sheet, not a generated difference image.
- Schematic `region: {x, y, width, height}` selects a rectangle in mm from the
  page's top-left. It is rasterized directly at `width_px` for a detailed close-up.
  Invalid or out-of-page rectangles are refused; region height is at most 8192 px
  and area at most 16,777,216 pixels. Explicit positive SVG page dimensions in mm
  are required, matching KiCad's exports. The full-sheet rasterization function
  and visual-baseline comparison path are unchanged.
- `get_board_2d_view` adds `side: "top" | "bottom"` and positive finite `zoom`.
  Width/height defaults and clamping are unchanged, and it still returns one image
  block. A private temporary output preserves any neighbouring `.render.png`.
  It reads the saved board through KiCad's orthographic 3D renderer. Selected-layer
  views remain in `export_svg`; inline layer and library-item renders from rank 25
  remain follow-ups.
- Regression coverage adds 23 tests across the renderer, schematic handlers, MCP
  helper and board handler. The existing real KiCad SVG fixture exercises the
  crop, and stand-in CLI tests exercise the handlers. All 29 negative controls
  (9 renderer, 8 board, 12 schematic/MCP) failed their intended tests, and all
  mutations were restored. The final macOS gate is listed on #28. No live editor IPC is needed for these tools;
  Windows, Linux and display in an actual MCP client remain unverified.

## PR 5, shipped creation guidance (merged, #29)

The claim commit and draft preceded implementation. This branch is rebased on
`main` after PR 4 merged; the schematic and PCB skills name its `region` crop and board `side`/`zoom`; no runtime tools, schemas or API response shapes
change, so `API_MIGRATIONS.md` and `tool-directory.md` need no new entries here.

- Schematic guidance loads library/hierarchy/bus tools, distinguishes placed IC
  power units from library-wide pin metadata, and keeps repeated-sheet local
  rails separate. Schematic tools save their edits; `save_project` is PCB-only.
  Final acceptance uses independently prepared intent with `compare_netlist`
  `scope: "complete"`, inspecting `result`, `passed` and mismatch counts.
- PCB guidance names atomic `set_component_placements`, `flip_component` and
  `query_board_region`. It gives the supported check/export/route/plan/apply
  Freerouting path, revision/manifest checks, current profile limits and a hard
  stop on routing failure. Unsupported work goes back to concrete KiCad GUI
  steps; blockers, unrouted connections and missing completion evidence remain
  `INCOMPLETE`.
- Review/manufacturing guidance follows all ERC/DRC pages or the full report,
  keeps the source stable, and avoids repeating save/refill for page reads.
  It reports actual saved/live provenance, preserves audit limitations and now
  describes PR 7's true outline/containment checks correctly.
- Six new tests check compiled skill bytes, registry ownership and key workflow
  contracts; existing asset checks still cover all tool names and example
  parameters. Three independent Codex dry runs covered blocked review/routing/
  schematic cases and supported continuations. These simulations do not prove
  live client behavior. All 13 guard-removal checks failed their intended tests,
  all mutations were restored and both asset suites passed. The final gate is on #29.
- Both agent prompt linters passed. The generic skill validator could not start
  under the required isolated Python because PyYAML is absent; native repository
  and compiled-asset checks are the available verification. No client-side agent
  run, real Freerouting job, Windows, Linux or hosted CI was performed. The
  owner's deferred live Freerouting run remains deferred.

## PR 3, response budget (merged, #21)

The client refuses a tool result near 50,000 characters. Real use had 8 ERC/DRC results refused at 51 to 71 thousand, which sent the assistant to run `kicad-cli` itself. One helper, `tools/response_budget.rs`, now does the cutting:

- `window(args, default)` reads `limit` (1 to 10,000) and `offset`; a present value it cannot read is refused, not replaced by the default.
- `page(items, offset, limit, budget)` stops at `limit` or at 40,000 characters of items, whichever comes first, and always returns one item when any remain. `describe` adds `shown`, `offset`, `limit`, `truncated` and, when truncated, `next_offset`, `byte_capped` and a `hint`. `tally` counts every item by rule and severity, not just the page. `list_response` is the same for plain lists.
- Applied to `run_erc` (new `limit` default 100, `offset`, `by_rule`), `run_drc` and `get_drc_violations` (shared `drc::violations_page`; the latter had no limit at all), `list_schematic_components`, `list_symbol_libraries` and `list_footprint_libraries`. `output` still writes the full, unpaged report, which is the "full report on disk" part.
- `load_toolset` returns each tool's first sentence (110 characters at most) by default; `detail: "full"` returns the old text. `tools/list` is unchanged. This was 20% of the bytes returned in real use.

Not done, and still open under rank 3: C24 (a `zone_fill_state` field on DRC, which depends on this), and A30's other long results (`batch_place_components`, `update_pcb_from_schematic`, `validate_for_manufacturing`, the batch wiring tools), `get_component_list` and `query_traces`. They can reuse the helper. A caller that read every violation in one `get_drc_violations` or `run_erc` call must now follow `next_offset` or pass `output`; `docs/API_MIGRATIONS.md` says so.

Proof: `response_budget_tests.rs` runs the real ERC handler against a stand-in `kicad-cli` that serves a 700-violation report (under the cap, counts of all 700, pages cover each violation once, `output` holds all of them), the DRC pager on 900 violations, and the list tools. Six guards were broken one at a time (the byte cap, the whole-list tally, truncation, paging order of `output`, argument checking, the summary) and each failed a test. Limits left: the cap counts characters of listed items only, so a response with very large non-list fields could still be large; the stand-in is not KiCad's own large report; the live KiCad was not asked.

## PR 2b, Freerouting on real boards (merged, #26)

`export_specctra_dsn` refused any board with an arc in its outline, a rounded or oval pad, or a copper zone, so a real routing run was blocked and the AI hand-rolled a router. What changed (`specctra.rs`, `specctra_ses.rs`, `freerouting_setup.rs`, `check_freerouting`):

- **Outline:** one closed loop of lines, arcs, a rectangle, a circle or a polygon. Specctra has no arc here, so arcs and circles are chords whose middle strays at most 5 um (`ARC_TOLERANCE_MM`); the manifest says so. Curves, old-form arcs, two loops and zero area are refused.
- **Pads:** `roundrect` as its bounding rectangle (never smaller than the copper), `oval` as a stadium path, equal-sided oval as a circle. Trapezoid and custom are still refused.
- **Zones:** a pour becomes a `plane` of its net per copper layer, a rule area that forbids tracks or vias becomes a keepout, both from the zone outline, as KiCad's own DSN does (the fixture carries KiCad's export for comparison). Thermal reliefs are not modelled: run DRC after a refill. The SES import still adds tracks and vias only; it now checks the board's zone inventory against the manifest and refuses a mismatch.
- **Setup:** `check_freerouting` asks the JAR which Java it needs (the class-file version of its main class), looks for one (`java_path`, `JAVA_HOME`, `PATH`, the usual JDK folders) and returns `setup` with the exact steps. `available` is false when the JAR cannot start. `route_specctra_dsn` refuses before starting anything when no Java fits. **Approach chosen: guidance, not a downloader.** Fetching and running a 60 MB program from the network is the owner's decision; the steps are the commands they would run.
- **Native bridge:** KiCad writes rounded pads as polygons, which the differential check does not model, so `native_bridge_mode: prefer` falls back to the Rust DSN on such boards (a test pins this).

**Freerouting could not be run here.** When this was built, the machine had only Java 11, and Freerouting 2.1.0 needs Java 21 (`UnsupportedClassVersionError`, class file 65). Java 21 was installed afterwards, on 2026-11-07, but the live run was not done. So the DSN was not run through the real engine. What was run instead: Konnect's own DSN parser round trip, the planes and keepouts compared with KiCad 10.0.8's own export of the same board, and the import planner on a Freerouting-authored SES (from another board, adapted; see the fixture README). The real-engine test is `freerouting_routes_the_rounded_board` (ignored): with a Java 21 and `FREEROUTING_JAR` set it exports, routes and plans the import. That is the one check still owed.

Limits left: more than two copper layers and back-side parts (rank 32); the router sees a pour's outline, not its fill; a rounded pad's corners are unavailable to the router; the rounded-pad polygon KiCad writes is not adopted.

## PR 2 fixes (merged)

Review fixes for PR 2, now merged. In brief:

A four-lens review of PR 20, each finding checked by two verifiers, confirmed ten defects in the merged code. All are fixed on `fix/route-safety-review`:

- **A pin is not a tap.** `RouteGuard` allowed the net of any wire an endpoint lay on, so a pin sitting mid-span on a foreign wire with no dot made `connect_pins` and `connect_to_net` join that net silently (the route's own T dots it). Only a free coordinate (`Endpoints::Points`) taps now; pins (`Endpoints::Pins`) do not, and `connect_to_net` with `pin_x`/`pin_y` that are a placed pin's end counts as that pin. `batch_connect_to_net` is Pins in every mode, and `connect_pins_to_bus` now checks each stub, bus entry and label (`check_additions`).
- **A label alone is checked.** `stub_length` 0 skipped the guard, but a label attaches to any wire it lies on. With no stub the only way a label joins a foreign net is a wire it lies inside with no junction (`label_under_undotted_wire`), so that cheap geometric screen runs first and only the few pins that fail it get the full check; `batch_connect_to_net`'s default path reads the wires once and runs the full check on none of the ordinary pins (a test counts the checks). When the sheet's pins cannot all be read, a label is judged without them (`label_join_without_pins`): it goes on unless the wire is another net, does not carry the name, and the point is not a free coordinate.
- **`nets_joined` in every mode.** The default `batch_connect_to_net` returned no such key. It reports always now, and `batch_connect_to_net` sets `isError` when every pin was skipped.
- **Aliases.** The allow-by-name rule matched only a net's winning name, so a stub to a net that carried the requested name as a lower-ranked alias was refused. It matches any name on the net.
- **A negative `stub_length`** is refused (`invalid_argument`, schema minimum 0, and in the handler).
- **An unreadable symbol** is refused as "cannot check what the wire would join", not as a join, and the docs say so.
- **Second review of this PR** (independent, same day) found the partial fixes folded in above plus a performance regression in the default batch and an over-broad label fallback; both are fixed, with a test that fails without each. Known gap, not fixed: the alias rule ignores what kind of name it matches (a local label and a power symbol of one name merge in the model; whether that matches KiCad is a separate question).
- **Tests** for each of the above, the unreadable-symbol branch and `nets_joined: null`, and for the names a label-made merge reports. The ignored KiCad test also checks that the refused pin cases leave the circuit unchanged.

Seven guards were broken one at a time and each failed a test. Limits as before; the check still sees only pins, labels and sheet pins.

## PR 9, sync completion, first half (merged, #24)

Branch `feat/creation-sync-completion`; all in `tools/pcb_sync.rs`, plus the two new arguments in the schema in `sch_export.rs`.

- **Field copy.** The netlist's `(fields …)` is read into `DesignComponent.fields` (everything but Reference, Value, Footprint and Component Class, as KiCad's updater skips them). A matched footprint's Datasheet, Description and user fields are compared by name; each difference is a `field_changes` entry (`name`, `from`, `to`) on the `update` change, and an added footprint carries `fields`. The write goes through `apply_footprint_fields`, the same path as reference and value. A user field the footprint lacks is created hidden on `F.Fab`/`B.Fab` at the footprint origin, as KiCad does; one the symbol leaves empty is not created. A footprint field the symbol lacks is left alone (KiCad removes it only under its "remove extra fields" option; Konnect has no such option yet). A board that was `noop` can now be `ready`.
- **Orphans.** A footprint is an orphan when it has a schematic identity, is not board-only, matches no symbol, and its symbol is not merely excluded from the BOM (`ExportedDesign.bom_excluded` is new, because the flag reader used to drop those symbols without a trace, which would have orphaned their footprints). Always listed under `orphan_footprints`. `delete_orphan_footprints` plans a `delete` change for each and the apply deletes exactly those KIIDs, first in the commit. A locked orphan is kept (`kept_locked`). A conflicted plan deletes nothing (`kept_plan_conflict`).
- **Attached copper.** Tracks, arcs and vias are now read with their geometry (`BoardState.copper`). Copper is attached to an orphan when it is on one of its pads' nets and an end lies inside the circle that holds that pad (half the diagonal of the largest layer size, plus half the track width; a via counts as 1 mm wide; any layer). It is deliberately wide: it can name copper that merely passes close and cannot miss connected copper. With `delete_orphan_footprints` alone, such an orphan makes the plan a conflict (`orphan_footprint_has_tracks`) that lists the copper. `delete_orphans_with_tracks` allows it; the copper stays. That flag without the first is `invalid_argument`.
- **Readback.** `orphans_deleted.applied`, `deleted_footprints` and `fields_updated.applied` come from reading the footprints back after the commit. A footprint KiCad still lists is `orphan_footprint_still_present`; a field that did not land is `field_readback_differs`. The plan revision now covers footprint fields, locked and board-only flags and the copper KIIDs, and the deletions are in `changes`, so a plan reviewed without the flag cannot be applied with it.
- **Proof.** Unit tests on the planner and on the served `tools/call` path against the protobuf mock board (changed field, idempotent second run, field already right, added footprint, orphan kept by default, unrouted orphan deleted, orphan with copper refused and then allowed, copper that does not touch, locked/unlinked/board-only/BOM-excluded never deleted, stale revision, a KiCad that ignores deletes or updates, flag misuse, conflicted plan). Sixteen guards were broken one at a time and each failed a test: copper never found, copper net or distance not compared, locked, board-only, unlinked and BOM-excluded footprints deleted, deletion on by default, a conflict that still deletes, deletion counted from the request, fields never compared, field readback skipped, the second flag accepted alone, new field visible, deletion never sent, Footprint field leaking into the field list.
- **Not verified.** Nothing ran against a live KiCad: the field create/update and `DeleteItems` of a footprint are exercised only against the protobuf mock, shaped from KiCad 10.0.1's `footprint.cpp`/`pcb_field.cpp`. The live test is `crates/konnect/tests/live_kicad_tools.rs` (needs a disposable board). Windows and Linux were not run.
- **Known limit.** Replacing a symbol under the same reference (delete R5, add a new R5) is still a `reference_identity_conflict`, and a conflict deletes nothing, so the old R5 has to go first (`delete_component`, or KiCad). PR 9b retains this refusal: a different schematic identity still needs explicit repair, rather than an inferred reference-only replacement.
- **PR 9b** is implemented in draft #33: in-place replacement and whole-net copper reassignment, preserving instance state and refusing routed-pad geometry changes. See its section below.

## PR 2, route safety (merged, #20)

Two commits:

- **#901 ported as it stands** (author Paulius Zaleckas, cherry-picked with `-x`): `connect_pins`, `batch_connect_pins` and `add_schematic_connection` try the other L-bend and refuse with `plan_blocked` when both would join another net. One conflict, in `docs/API_MIGRATIONS.md`, kept both entries.
- **#899 and #900, which #901 touches the same path as.** A stub from `connect_to_net` or `batch_connect_to_net` is checked the same way (`pick_stub_direction`): `auto` tries the other directions, a named direction that would join is refused or, in the batch, skipped into `errors`. Reaching what already carries the stub's own net name is not a join, because the label joins it anyway (an existing test needs this). `add_wire`, `batch_add_wire` and the stub wires dot only the T's the new wire forms (`t_junctions_on`). `add_wire` takes coordinates, so it draws what it is given and only reports.
- **`nets_joined`** (`sch_wiring::nets_joined`) on all seven wiring tools: `[{nets: [...]}]` for each net that now holds two or more nets the sheet had before; `[]` for none; `null` when pins cannot be resolved. A net of bare wires is not listed.

Proof: `route_safety_tests.rs`. The fast tests read the file. One ignored test asks the real KiCad, through `compare_netlist`, that the result connects what was meant on the #901 fixture (a far wire changes nothing, a refused route changes nothing, the avoided pin stays alone, the stub leaves the tee alone):

```bash
cargo test -p konnect-core --lib route_safety -- --ignored
```

Guards were broken one at a time (the join check, the T filter, the stub check, the report); each failed fast tests and the live one. Limits left: the check sees pins, labels and sheet pins, so two bare-wire nets with nothing on them can still touch unreported; labels or power symbols that merge nets by name are out of its sight until `compare_netlist` runs; `add_power_symbol`, `add_junction`, the bus tools and the `move_*` tools were not looked at, and `add_wire` / `batch_add_wire` only report.

## PR 1, the netlist read and compare (merged, #19)

Two tools in the `sch_export` toolset (`tools/sch_netlist.rs`, tests in `tools/sch_netlist_tests.rs`).

- **`get_schematic_netlist(schematic, limit, offset, name_contains)`** returns `{net: ["REF.PIN", ...]}` for the whole hierarchy. `schematic` is the root sheet or the `.kicad_pro`; a sub-sheet is refused with the root named, because KiCad would export it as if it were the design. The saved sheets are copied into a private folder (the simulation snapshot, `simulation/snapshot.rs`) and `kicad-cli sch export netlist --format kicadsexpr` runs on the copy (`cli::export_kicad_netlist_text`), then `simulation::kicad_netlist::parse` reads it. That parser keeps every node, which `pcb_sync::parse_exported_netlist` does not (it drops footprint-less parts and bails on a pad in two nets), so the schematic check uses the simulation one. Nets KiCad names itself (`Net-(R1-Pad2)`, `unconnected-(R2-Pad2)`) are included. Pages stop at 40,000 characters of net data or `limit` nets, whichever comes first, and say `truncated` and `next_offset`. One net is cut to 300 pins with its true size in `pins_truncated`.
- **`compare_netlist(schematic, intended | reference_schematic | reference_netlist, scope, limit)`** compares pin sets, never net names. `missing` is an intended pin the design does not have. `split` is one intended net spread over several design nets. `merged` is one design net holding pins of several intended nets. `extra` is a pin joined to a net the intent does not give it. The verdict is `pass`, `fail`, or `incomplete`. With `intended`, `scope: listed_pins` (default) looks only at the pins listed, so a partial intent works; `complete` also reports every other pin joined to them. A second design or a `.net` file is always compared completely.
- **`incomplete` is deliberate.** A duplicated reference (two `R1`) puts one pin id on two nets, so it cannot be told apart. Nothing wrong found plus that ambiguity reads `incomplete`, never `pass`. Unannotated references (`R?`) are warned about.
- **Saved files only.** The result never includes unsaved editor changes, and says so (`source.saved_files_only`, plus `editor_locks` and a warning when KiCad holds a lock). Callers save first.
- **Do not build on** the per-file `NetGraph` or design_review's per-sheet scope: neither merges nets across sheets.

Evidence: the committed `tests/fixtures/single_pin_nets.net` is KiCad's own export of a two-sheet hierarchy (README beside it). Unit tests run in the normal gate, including the handlers against a stand-in `kicad-cli`. One ignored test runs the real KiCad on edited copies and covers a hierarchical net, an unnamed net, a split and a merged case:

```bash
cargo test -p konnect-core --lib sch_netlist -- --ignored
```

Seven guards were broken one at a time and each broke a test (split, merged and extra detection, the size cap, the incomplete verdict, the missing-pin check, and the snapshot copy). The tools were also run through the built binary over stdio (`load_toolset` then both tools, a refusal included) on a scratch copy of the fixture. Limits left: the sub-sheet refusal needs a `.kicad_pro` beside the sheets, as `run_erc`'s does; a loose child sheet with no project file is read as its own root, and the response's `design_components` and `source.schematic` show what was read. It reads a design KiCad can export, so a project that does not load has no answer; pin ids are `REF.PIN` with the first dot as the separator; there is no `outcome` envelope yet (this track's rank 41); Windows and Linux were not run.

PR 2 should call `compare_netlist` with `reference_schematic` on a copy of the project taken before the wiring change. Edits to `connect_pins` and its siblings are in `tools/sch_wiring.rs`.

## PR 7, the board geometry layer (merged, #25)

Research ranks 6 and 7 (C19, C17, A14's containment part). One model of the board, one outline, and four tools that read it.

- **The layer** (`konnect-sexp`: `planar.rs`, `outline.rs`, `board_geometry.rs`). `Outline` joins every `Edge.Cuts` line, arc, circle, rectangle, polygon and Bézier (and the same shapes inside a footprint, such as a mounting slot) into closed rings; a ring inside a ring is a cut-out (even-odd). Arcs and circles are tessellated within 0.005 mm of the true curve and say so (`tolerance_mm`, `curved`). `status` is `closed`, `open` (a gap), `unreadable` (a graphic that could not be read, including the pre-KiCad-7 arc form) or `missing`; only `closed` judges anything. `BoardGeometry` holds footprints (oriented courtyard box, pads as real polygons, Reference/Value text), tracks, arc tracks, vias, zone outlines and graphics. A pad's angle is board-space and its centre goes through the footprint's rigid transform, the convention `bounds::pad_bbox` was checked against KiCad with. Anything unreadable is counted in `skipped`, never zero-filled.
- **`query_board_region`** (placement toolset, read-only, `tools/board_geometry.rs`). One of `bbox`, `around {reference, pad?, margin_mm}` or `point {x, y, radius_mm}`; `kinds`, `layer`, `net`, `limit` (default 50, max 200). Nearest first, with nets, layers and UUIDs, `total_matches`, `counts` and `truncated`. It reads the live board through the board-source seam (`board_source`), so unsaved edits count, and the answer names which it used.
- **`query_traces`** gains `limit` (default 200) and `touching_pad`. This changes its default: a board with more than 200 segments now truncates (and says so) unless `limit` is raised.
- **Containment against the true outline.** `score_placement` no longer needs a rectangle: `outline_shape` is `rectangular`, `closed`, `unproven` (open or unreadable) or `missing`, each `outside_outline` failure carries `overhang_mm`, and `edge_mount_references` lists parts that may sit on the edge. `audit_manufacturing`, `validate_for_manufacturing` and `run_design_review` replaced their `contains("Edge.Cuts")` test, which every KiCad 6+ board passes through its layer table, with the same check: a missing or open outline, and each footprint whose courtyard, pads or silkscreen text cross or sit outside it, with the overhang in mm and the same allowlist. Pads over the edge and parts wholly outside are errors; a courtyard alone and text are warnings; allowlisted parts are `info`, not dropped.
- **Evidence.** `tests/fixtures/board_geometry/cases.kicad_pcb` (README beside it) is a 40 × 30 mm board with 5 mm arc corners and a 3 mm cut-out, with a part inside, straddling the edge, fully outside, in the rounded corner, over the cut-out, rotated, and with text over the edge. Every expected number in the tests is hand-computed from its coordinates. The independent oracle is KiCad's own DRC on a scratch copy (`board_geometry::tests::kicad_drc_agrees_with_the_outline_about_which_pads_are_at_the_edge`, ignored): its `copper_edge_clearance` names the same three pads this geometry does, at distances within 0.01 mm, including the pad in the rounded corner that a bounding-box test passes and the rotated pad that is on the board only under the right rotation sign.
- **Negative controls.** 18 guards were broken one at a time and each failed a test: arcs in the outline, cut-outs, the outline vertex inside a part, the fully-outside fast path, the pad rotation sign, the allowlist, the open-outline refusal, the truncation flag, the layer filter, `score_placement`'s use of the true outline and its allowlist, the old string test, the manufacturing issues and the `info`-does-not-block-READY rule, `touching_pad`, the trace limit, and the live-board read.
- **Not done, on purpose.** The three planners (`place_decoupling_caps`, `auto_place_from_schematic`, `refine_placement_force_directed`) still refuse any outline that is not a provable rectangle; moving them to the true outline belongs with PR 8, and until then `place_decoupling_caps`'s `verdict_after_plan` reads `hard_fail` rather than `outline_unproven` on such a board. Free-space finder, copper-island report and planned-copper clearance (C18, C23, C15) are the follow-ups on this layer.
- **Limits.** Field text boxes are estimated from the font size (KiCad's glyph metrics are not available), so text findings are warnings and say `estimated`. `score_placement` still uses the axis-aligned courtyard box for overlaps and containment (exact at 0°, 90°, 180°, 270°; a superset at other angles), while the review checks use the oriented box. Custom-shape pads are the box of their primitives (`approximate_shape`). Zones are their authored outline, not the fill. Footprint graphics, dimensions and tables are not in the query (`not_included` says so). Overlapping outline rings are read by the even-odd rule, which treats an overlap as a hole. `query_traces` lists straight segments only, as before. macOS only; Windows and Linux were not run.
- **Verify.** The gate in `docs/SIMULATION_HANDOFF.md`, plus the oracle: `KICAD_CLI=/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli cargo test -p konnect-core --lib board_geometry -- --ignored`.


## PR 10, library correctness (integrated, #32)

The claim commit and draft preceded implementation. Adds `edit_symbol` and
`check_symbol_footprint` in the library toolset, with implementation and tests
in `tools/library_correctness.rs`.

- Partial edits cover Value, Reference prefix, Footprint, Description, Datasheet
  and keywords. Omitted fields and existing geometry/field positioning remain
  intact; writes check the original revision and independently read back saved
  metadata. `create_symbol` gains footprint, description and keywords, refuses
  duplicate names, and verifies saved fields. `delete_symbol` uses structural
  spans and refuses deletion of an inherited base.
- Pin/pad comparison uses effective inherited pin identifier sets, ignoring
  unnumbered mechanical pads and collapsing repeated identifiers. Missing/extra
  pads fail; unresolved chains or unreadable/absent identifiers are incomplete.
  It does not prove a physical pin map or datasheet correspondence.
- Searches rank exact names, prefixes, substrings and metadata, require all query
  words, merge global/project registrations with project precedence, then page
  the complete ranked result. Failures of discovered entries are disclosed.
  Missing/invalid library tables and nested table-expansion diagnostics remain
  outside that completeness claim.
- Verification: the baseline gate was green. 178 focused library tests (13 new) and 20 isolated
  guard-break controls exercise metadata, saved verification, structural edits,
  inheritance, identifier mismatch and ranked/paged project search. The full
  after-commit gate and release-binary scratch dogfood are required at closeout;
  their results are reported in the PR. No live project or installed binary is
  used for mutation. Windows/Linux, GUI library refresh and physical pin-map
  validation remain unverified.

## PR 9b, sync replacement and routed nets (integrated, #33)

The claim commit and draft preceded implementation. No new tools or arguments.
`pcb_sync_completion.rs` holds the new preparation, revision and readback helpers;
`pcb_sync.rs` integrates them, and `pcb_footprint_update.rs` exposes a narrow
adapter to its existing instance-preserving rebuild.

- Changed footprint IDs are previewed as updates with `replace_with`. Library
  preparation and symbol-field application happen before mutation. KIID,
  position, angle, side, lock, identity and overrides survive. Connected pads
  cannot disappear; routed-pad geometry cannot change, so existing copper stays
  attached rather than being moved by an implicit router. With unresolved copper,
  every pad's geometry is held fixed, including pads without assigned nets.
- Whole-net reassignment requires unanimous saved intent for all old terminals
  and complete known copper coverage. Tracks, arcs and vias retain geometry and
  KIIDs. Splits, skipped/board-only terminals, zones, unnetted copper and
  ambiguous/malformed live pad data refuse safely.
- Preview revisions cover raw live objects and exact prepared library bytes.
  Completion counters come from readback. Differences after commit are
  `applied_with_differences`; an unavailable verification is an explicit uncertain
  outcome, not a preflight refusal. Inspect the live board before retrying.
- Verification includes KiCad-authored library fixtures, front/back and rotated
  instance preservation, served MCP preview/apply/no-op, unsafe-pad refusal,
  whole-net changes, stale revisions, ignored writes and post-commit failures.
  All 35 isolated guard-break controls caused actual test failures and were
  restored. The focused suite has 94 passing tests (16 new), plus the ignored
  release stdio probe. The full after-commit gate is required at closeout;
  exact results are recorded in the PR. The release stdio dogfood uses mock IPC
  and a netlist-export stand-in, not a live GUI.
- Still unverified: real KiCad IPC replacement/reassignment, GUI/undo behavior,
  Windows/Linux and physical DRC/connectivity after a real-board operation.
  Arbitrary partial net splits, routed-pad relocation and reference reuse under
  a different schematic identity remain explicit follow-ups. Saving the board
  and refilling zones are not part of this sync.

## PR 12, verified routing batches (integrated, #34)

The claim commit and draft preceded implementation. A shared live routing path
in `pcb_routing_batch.rs` serves three new tools and four upgraded tools.

- `route_batch` expands explicit polylines and through vias, max 200 objects,
  preflights the complete request and writes one undo commit. `route_trace`,
  `route_pad_to_pad` and `add_via` use the same creation/readback checks.
- Nets and effective rules come from live IPC; enabled copper layers and live
  pad net/layer membership are checked. Repeated pad anchors refuse. Preferred
  netclass widths are Konnect's lower routing-policy bound, not KiCad DRC or
  current-capacity proof. Widths/drills/diameters round to nanometers before
  checks. Missing live evidence refuses rather than using saved defaults.
- `modify_trace` updates an unlocked observed straight trace in place, retaining
  KIID and omitted fields. `query_vias` is paged/byte-bounded; `delete_via` checks
  type/lock and post-write absence. All use exact requested-board binding.
- Creation receipts and readback must agree on identities, nets, geometry and
  inventory. Commit or post-write verification failures are explicit uncertain
  outcomes; inspect before retrying. These seven tools adopt the shared outcome
  envelope, counting one request separately from copper-item counts.
- Verification covers served MCP, live-vs-saved pad anchors, mixed batches,
  rounding, class/layer/net failures, stale/type/lock refusals, modified/deleted
  postconditions and faulty receipts/readback. The full after-commit gate and
  release stdio probe are required at closeout; exact results are in the PR.
  All 45 guard-break controls caused actual test failures, then source was
  restored. The focused routing suite has 68 passing tests; the old live-pad
  regressions are retained in the shared served harness.
- Not claimed: path planning, obstacle/short/clearance checks, DRC, zone refill
  or save. Real KiCad GUI IPC/undo and Windows/Linux remain unverified. The
  release probe uses a private IPC bridge to a protobuf mock and scratch files.
  Differential-pair routing, delete_trace and Specctra keep their own contracts.

## PR 13, initial rule-writer slice (merged, #35; historical scope)

In #35, `create_custom_drc_rule` was a deliberately narrow first step: it previews or
creates one unconditional clearance or track-width minimum, and refuses an
existing rules file unless every rule is in its canonical single-line format.
This preserved unsupported files unchanged; #38 below adds general rule editing.
The preview revision binds board path, saved board and project bytes, existing
rule-file bytes and the candidate.
Apply rechecks the canonical board lock, writes atomically and verifies saved
bytes. Lock inspection failures refuse rather than imply absence.

Validation runs KiCad DRC on a private copy of a KiCad-authored board, with the
candidate and a temporary probe clearance. A violation naming the probe proves
that the scratch rules loaded; the command's exit status alone does not. It also runs DRC on a separate scratch copy of the saved target board and
project, reporting counts and disclosing missing external libraries, parity and
zone refill. It never runs DRC directly on the original. Real KiCad scratch tests cover preview,
apply, an observed target-rule violation and malformed-rule rejection. PR 13
now also adds three project minima and independent preview/apply tools for differential-pair settings and deletion of netclasses unreferenced in saved project data. Project writes reject duplicate keys and linked files, check editor locks and verify conditional writes. Schematic connectivity and live netclass validation are not inspected. These were the limits of #35. Continuation #38 below closes the rule edit/delete and expression gaps and enables the explicitly supported custom-rule DSN/SES profile with revision binding, native scratch validation and verified recovery.

## PR 13 continuation (merged, #38)

The claim-only commit and draft preceded implementation, from main d0880ac.
Work was isolated in `Konnect-pr13-rules` on `codex/pr13-rules-completion`;
Q3's separate config/design-intent work is outside these files.

The continuation adds full-statement rule creation, named edit/delete, strict
whole-file framing and semantic inventory preservation, native compilation of
conditions and assertions, and project/schematic-lock/revision checks. A trailing
replacement comment cannot hide another rule, and stale no-op edits refuse.

DSN export now translates the explicit unconditional numeric clearance/width
minimum subset conservatively, preserving larger live preferences. Other rule
semantics refuse export. The manifest seals saved project/rule bytes and live
classes; SES planning/apply recheck them. Legacy manifests with saved sidecars
require re-export. Native require mode refuses custom files; prefer uses Rust.

SES verification runs DRC only in a private copy with the captured sidecars.
Custom-rule candidates require complete, clean PCB/unconnected-item evidence, all severities
and enabled required checks. Failed verification compensates only verified new
identities, disjoint from all original board UUIDs. Source, IPC and serialized
candidate routing identities must agree. Exact restored serialization proves
rollback; otherwise the outcome is uncertain. No save, refill or schematic-parity check is requested. Coverage,
ignored checks and source are explicit. All 88 guard-removal controls produced actual test failures and source was
restored. The final gate and release probes are recorded on #38 after the
implementation commit.

Real KiCad GUI/undo, the owner-deferred Freerouting run, hosted CI and Windows/
Linux are not established by the mock IPC tests or native scratch CLI tests.

The implementation head ebab52c passed 3,645 workspace tests, 9 doctests and all
93 live simulations, plus formatting, Clippy, licenses, catalogue counts and the
release build. Both standalone-fork probes asserted executable path and exact
commit; six real KiCad scratch tests passed explicitly. No real GUI/undo or
Freerouting engine result is implied. The merge uses the owner's explicitly
authorized ordinary fast-forward, preserving noreply commits without a force
push. Installed SHA-256 remains
`3de188bb775ab4ce5748ea44b94cb098c86ed5d32135e70f4c2c3817c3a4ac65`.
