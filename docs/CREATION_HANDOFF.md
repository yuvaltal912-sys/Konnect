# Creation handoff

**2026-11-21.** Row CL Limit 2 is open for review. Its section below records the public fixture and B6 routing results. [PR #97](https://github.com/yuvaltal912-sys/Konnect/pull/97)

**S11 resumed 2026-11-12; implemented, reviewed and gated 2026-11-13.** The owner resumed PRs #63, #64 and #65 under one supervisor session (decision row "Claude resume of three PRs"); the 2026-11-10 pause no longer applies to #64. S11 runs as Claude (`claude-opus-5-5`, `xhigh`). Current state: "Row S11: suspected zone-fill defects".

The current state of the creation-side work: schematic, PCB and library tools. Anyone picking this up, human or agent, starts here. Update this file in the same commit that changes the state. The simulation track has its own file, [`SIMULATION_HANDOFF.md`](SIMULATION_HANDOFF.md).

**Last updated:** 2026-11-23 (Row ZR, stage ZR2, the model as the routing truth and the closing stage, merged as [#115](https://github.com/yuvaltal912-sys/Konnect/pull/115), main `ac8b5e01`, with fastroute x21 pinned; see "Row ZR"). Before that 2026-11-23 (Row ZR, stage ZR1, the engine's fill model and the pad comparison, merged as [#113](https://github.com/yuvaltal912-sys/Konnect/pull/113), main `900e8fe1`, with fastroute x20 pinned; stage ZR2 in progress (Claude); see "Row ZR"). Before that 2026-11-23 (Row ZR, stage ZR0, the judge's pour measures, merged as [#110](https://github.com/yuvaltal912-sys/Konnect/pull/110), main `13c71913`; stage ZR1 in progress (Claude); see "Row ZR"). Before that 2026-11-22 (Row GS, the smart-router switches on fastroute x19, merged as [#108](https://github.com/yuvaltal912-sys/Konnect/pull/108), main `6b31bf90`; x18 is now refused, and the installed package and candidate are still x18 until the owner has them rebuilt; see "Row GS"). Before that 2026-11-22 (Row ZR: the addendum "The design on x19" in [ZONE_ROUTING_DESIGN.md](ZONE_ROUTING_DESIGN.md), docs only, merged as [#106](https://github.com/yuvaltal912-sys/Konnect/pull/106), main `b78371d2`; the row still waits for the owner; see "Row ZR"). Before that 2026-11-22 (Row CT, family C as a placement tool, `plan_convergent_placement`, merged as [#104](https://github.com/yuvaltal912-sys/Konnect/pull/104), main `6abf6333`; see "Row CT"). Before that 2026-11-22 (Row ZR's research and design, zones in the router, merged as [#102](https://github.com/yuvaltal912-sys/Konnect/pull/102); no build work starts yet, on the owner's word; see "Row ZR"). Before that 2026-11-22 (Row CP merged as [#98](https://github.com/yuvaltal912-sys/Konnect/pull/98); see "Row CP"). Before that 2026-11-21 (Row CP: measured on two of the owner's boards, the review's fix round done, `fork/main` merged in with #96 and #97). Before that 2026-11-21 (Row CL's Limit 2, custom pads in the Specctra export, merged as [#97](https://github.com/yuvaltal912-sys/Konnect/pull/97) by Codex; see "Row CL"). Before that 2026-11-21 (F1: the route judge takes a live-exported board by its tree at each run's import too; branch `claude/judge-live-hash`, not merged; see "Row RJ"). Before that 2026-11-20 (Row CP, the convergent placement search, started on the owner's word). Before that 2026-11-20 (Row PS's changes since #92, the schematic-neighbour measure and generators 7 and 8, merged as [#94](https://github.com/yuvaltal912-sys/Konnect/pull/94); Row DP, differential pairs to fastroute, merged as [#93](https://github.com/yuvaltal912-sys/Konnect/pull/93); see "Row PS" and "Row DP"). Before that 2026-11-19 (the bench's owner flow merged as [#92](https://github.com/yuvaltal912-sys/Konnect/pull/92); see "Row PS"). Before that 2026-11-19 (Row PS, the placement-search bench, merged as [#90](https://github.com/yuvaltal912-sys/Konnect/pull/90); see its section). Before that 2026-11-18 (Row AH, the switching loops and pin types, merged as [#88](https://github.com/yuvaltal912-sys/Konnect/pull/88); see its section). Before that 2026-11-18 (Row AP, the decoupling check and pin types, merged as [#86](https://github.com/yuvaltal912-sys/Konnect/pull/86); see its section). Before that 2026-11-18 (Row RJ, the route judge and live boards, merged as [#85](https://github.com/yuvaltal912-sys/Konnect/pull/85); see its section). Before that 2026-11-18 (Row KE, the `kopper` entry skill, merged as [#83](https://github.com/yuvaltal912-sys/Konnect/pull/83); see its section). Before that 2026-11-18 (Row KP, the kopper1 plugin, merged as [#81](https://github.com/yuvaltal912-sys/Konnect/pull/81); see its section). Before that 2026-11-17 (Row JS, the judged route status, merged as [#79](https://github.com/yuvaltal912-sys/Konnect/pull/79); see its section). Before that 2026-11-17 (Row K4, 1 Kopper: its user guide, [`1-KOPPER.md`](../1-KOPPER.md), merged as [#77](https://github.com/yuvaltal912-sys/Konnect/pull/77); the 1 Kopper series is done; see its section). Before that 2026-11-17: Row K3, the package smoke test, merged as [#76](https://github.com/yuvaltal912-sys/Konnect/pull/76); Row K2, one install command, merged as [#75](https://github.com/yuvaltal912-sys/Konnect/pull/75); and Row K1, simulation off by default, merged as [#74](https://github.com/yuvaltal912-sys/Konnect/pull/74). Before that 2026-11-17 (Row BN merged as [#72](https://github.com/yuvaltal912-sys/Konnect/pull/72), fastroute x18 in [#73](https://github.com/yuvaltal912-sys/Konnect/pull/73); see its section). Before that 2026-11-13 (Rows S7 and S11). On 2026-11-09 ten PRs merged into main (`a8df6ba`), in this order: [#57](https://github.com/yuvaltal912-sys/Konnect/pull/57), [#58](https://github.com/yuvaltal912-sys/Konnect/pull/58), [#56](https://github.com/yuvaltal912-sys/Konnect/pull/56), [#51](https://github.com/yuvaltal912-sys/Konnect/pull/51), [#55](https://github.com/yuvaltal912-sys/Konnect/pull/55), [#53](https://github.com/yuvaltal912-sys/Konnect/pull/53) (simulation), [#49](https://github.com/yuvaltal912-sys/Konnect/pull/49), [#48](https://github.com/yuvaltal912-sys/Konnect/pull/48), [#50](https://github.com/yuvaltal912-sys/Konnect/pull/50), [#52](https://github.com/yuvaltal912-sys/Konnect/pull/52). Each first had an independent review, with every verified finding fixed and tested in its own PR, and all ten were merged through one gated trial integration (4,153 tests, `simulation_live` 95/95, release build); every main tree after a merge matched its trial step. Freerouting now starts without telemetry, outside the home folder and off the network. Open: [#54](https://github.com/yuvaltal912-sys/Konnect/pull/54) (simulation, see `SIMULATION_HANDOFF.md`). The owner's direction: routing quality on boards like his, measured against [`LAYOUT_CHECKLIST.md`](LAYOUT_CHECKLIST.md). Catalogue on main: 270 registered tools plus 7 meta-tools; `library`, `pcb_components` and `sch_wiring` are at the router's 20-tool cap. Row S5 (routing measurement on the reference board) merged on 2026-11-10 as [#60](https://github.com/yuvaltal912-sys/Konnect/pull/60); its section below has the numbers and the preset. S6's first half merged as `f246e48` on 2026-11-10 in [#62](https://github.com/yuvaltal912-sys/Konnect/pull/62), from `codex/creation-s6-pour-widths-preset` and main `7e9a174`; the owner authorized its publication and merge on 2026-11-10. Row S7, the read-only route-refinement planner, is implemented, reviewed and gated on `codex/creation-s7-route-refine` (2026-11-13); publication and merge go through [PR #63](https://github.com/yuvaltal912-sys/Konnect/pull/63), and GitHub is the source of truth for its state ("Row S7" below). "Now" gives the fresh-session step, and "Row S6, first half" records the evidence. Before that 2026-11-22 (main `dc3c3e60` merged in: F1 as [#96](https://github.com/yuvaltal912-sys/Konnect/pull/96), the custom-pad export as [#97](https://github.com/yuvaltal912-sys/Konnect/pull/97), and Row CP, the convergent placement search, as [#98](https://github.com/yuvaltal912-sys/Konnect/pull/98); Row LT is built and verified at M on `claude/length-tuning` (local commits, not pushed): the judge chooses among Tune candidates and fails a net whose copper is not a clean path, L22 to L27, plus the via-stub fix; the PR waits for the owner's word; see "Now" and the row "Row LT close"). Before that 2026-11-21 (Row LT's verification failed; the design addendum for findings F2 to F6, L20 to L27, is on branch `claude/length-tuning-design-674041`, its owner decisions 4 to 6 approved, not built; see "Now"). Before that 2026-11-20 (Row LT, length tuning: the build started on the owner's word, on branch `claude/length-tuning`, the design branch with main `0a3fc504` merged in; see "Now"). Before that 2026-11-20 (Row PS's changes since #92, the schematic-neighbour measure and generators 7 and 8, merged as [#94](https://github.com/yuvaltal912-sys/Konnect/pull/94); Row DP, differential pairs to fastroute, merged as [#93](https://github.com/yuvaltal912-sys/Konnect/pull/93); see "Row PS" and "Row DP"). Before that 2026-11-19 (the bench's owner flow merged as [#92](https://github.com/yuvaltal912-sys/Konnect/pull/92); see "Row PS"). Before that 2026-11-19 (Row PS, the placement-search bench, merged as [#90](https://github.com/yuvaltal912-sys/Konnect/pull/90); see its section). Before that 2026-11-18 (Row AH, the switching loops and pin types, merged as [#88](https://github.com/yuvaltal912-sys/Konnect/pull/88); see its section). Before that 2026-11-18 (Row AP, the decoupling check and pin types, merged as [#86](https://github.com/yuvaltal912-sys/Konnect/pull/86); see its section). Before that 2026-11-18 (Row RJ, the route judge and live boards, merged as [#85](https://github.com/yuvaltal912-sys/Konnect/pull/85); see its section). Before that 2026-11-18 (Row KE, the `kopper` entry skill, merged as [#83](https://github.com/yuvaltal912-sys/Konnect/pull/83); see its section). Before that 2026-11-18 (Row KP, the kopper1 plugin, merged as [#81](https://github.com/yuvaltal912-sys/Konnect/pull/81); see its section). Before that 2026-11-17 (Row JS, the judged route status, merged as [#79](https://github.com/yuvaltal912-sys/Konnect/pull/79); see its section). Before that 2026-11-17 (Row K4, 1 Kopper: its user guide, [`1-KOPPER.md`](../1-KOPPER.md), merged as [#77](https://github.com/yuvaltal912-sys/Konnect/pull/77); the 1 Kopper series is done; see its section). Before that 2026-11-17: Row K3, the package smoke test, merged as [#76](https://github.com/yuvaltal912-sys/Konnect/pull/76); Row K2, one install command, merged as [#75](https://github.com/yuvaltal912-sys/Konnect/pull/75); and Row K1, simulation off by default, merged as [#74](https://github.com/yuvaltal912-sys/Konnect/pull/74). Before that 2026-11-17 (Row BN merged as [#72](https://github.com/yuvaltal912-sys/Konnect/pull/72), fastroute x18 in [#73](https://github.com/yuvaltal912-sys/Konnect/pull/73); see its section). Before that 2026-11-13 (Rows S7 and S11). On 2026-11-09 ten PRs merged into main (`a8df6ba`), in this order: [#57](https://github.com/yuvaltal912-sys/Konnect/pull/57), [#58](https://github.com/yuvaltal912-sys/Konnect/pull/58), [#56](https://github.com/yuvaltal912-sys/Konnect/pull/56), [#51](https://github.com/yuvaltal912-sys/Konnect/pull/51), [#55](https://github.com/yuvaltal912-sys/Konnect/pull/55), [#53](https://github.com/yuvaltal912-sys/Konnect/pull/53) (simulation), [#49](https://github.com/yuvaltal912-sys/Konnect/pull/49), [#48](https://github.com/yuvaltal912-sys/Konnect/pull/48), [#50](https://github.com/yuvaltal912-sys/Konnect/pull/50), [#52](https://github.com/yuvaltal912-sys/Konnect/pull/52). Each first had an independent review, with every verified finding fixed and tested in its own PR, and all ten were merged through one gated trial integration (4,153 tests, `simulation_live` 95/95, release build); every main tree after a merge matched its trial step. Freerouting now starts without telemetry, outside the home folder and off the network. Open: [#54](https://github.com/yuvaltal912-sys/Konnect/pull/54) (simulation, see `SIMULATION_HANDOFF.md`). The owner's direction: routing quality on boards like his, measured against [`LAYOUT_CHECKLIST.md`](LAYOUT_CHECKLIST.md). Catalogue on main: 270 registered tools plus 7 meta-tools; `library`, `pcb_components` and `sch_wiring` are at the router's 20-tool cap. Row S5 (routing measurement on the reference board) merged on 2026-11-10 as [#60](https://github.com/yuvaltal912-sys/Konnect/pull/60); its section below has the numbers and the preset. S6's first half merged as `f246e48` on 2026-11-10 in [#62](https://github.com/yuvaltal912-sys/Konnect/pull/62), from `codex/creation-s6-pour-widths-preset` and main `7e9a174`; the owner authorized its publication and merge on 2026-11-10. Row S7, the read-only route-refinement planner, is implemented, reviewed and gated on `codex/creation-s7-route-refine` (2026-11-13); publication and merge go through [PR #63](https://github.com/yuvaltal912-sys/Konnect/pull/63), and GitHub is the source of truth for its state ("Row S7" below). "Now" gives the fresh-session step, and "Row S6, first half" records the evidence.

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
| Cross-sheet review, B4 (2026-11-10) | Owner requested another independent creation session; the supervisor selected Step 13 item 4, the #55 follow-up, and confirmed the narrow review/diagnostics/check integration. Reason: its source ownership is separate from S7/S11, and the plan needs no extra owner design decision. Heavy checks are serialized. Owner subsequently delegated future approvals to supervisor judgment; the supervisor reviews this exact claim text. No independent merge, CI/Actions or auto-merge is authorized |
| S6 first half (2026-11-10) | Owner approved keeping pour-owning nets at their effective class width and shipping S5’s measured router preset, as one PR. Reason: S5 supports these two changes; per-zone policy and stitching stay deferred. Run checks end to end, then show exact title/body before any push or PR |
| S6 publication and merge (2026-11-10) | After seeing the exact title/body, the owner asked to merge this S6 work before switching sessions. This authorizes publishing and merging PR #62 with the shown text, using the noreply merge command; it does not start another row or authorize CI/auto-merge |
| S11 parallel preparation (2026-11-10) | Owner authorized independent creation work alongside S7; supervisor selected S11 and later added separate cross-sheet review. Separate worktrees/source ownership; heavy slots are S7, S11, then cross-sheet. Exact claim publication text still needs approval; no merge, simulation implementation, CI or auto-merge is authorized |
| S11 claim publication (2026-11-10) | Owner explicitly approved pushing `a423ec8` to `fork` and opening draft #64 with the title/body retained in `S11_PR_DRAFT.md` (body hash `8af4bf53ae9df21b7f84893a812c837ba40db19ca6d698825325f96cb29a6a07`). Readback matched. Implementation may proceed; changed public text and later pushes still need approval, with no merge/CI/auto-merge authorization |
| Delegated creation supervision (2026-11-10) | Owner authorized the supervisor to approve creation actions by judgment; the later current-PR-only boundary below supersedes continued row reassignment. This supersedes owner-only per-row/exact-text approvals for creation; technical gates, independent review, negative controls, noreply commits, fork-only pushes and isolation stay mandatory. Supervisor controls serial integration; S11 does not independently merge or enable CI/auto-merge. S7 owns the canonical AGENTS.md edit |
| Current-PR-only boundary (2026-11-10, latest) | Owner reduced this Astra chat to High reasoning and limited it to completing PR #64, including required checks, review and supervised publication/integration, then stopping. No next row or new feature assignment after completion. Prior keep-three-active direction is superseded; no checks are waived. New helpers must stay at High, not Ultra. S7 owns AGENTS.md |
| Finish S11 only (2026-11-10, latest owner direction) | Let S11 complete PR #64 through remaining validation, exact source/binary-bound replay, served MCP checks, full gates and supervised publication/integration, then stop. S7 #63 and cross-sheet #65 were directed to checkpoint and pause; S11 does not depend on their completion or import unfinished code. Astra/High and all isolation/provenance rules remain |
| STOP NOW (2026-11-10, latest owner direction) | Document and stop immediately, superseding the instruction to finish PR #64. PRs #63/#64/#65 are paused. No new work, validation, review, publication or integration without explicit owner resume. Preserve current dirty source and failed gates; remain Astra/High. AGENTS.md stays with S7 |
| Claude resume of three PRs (2026-11-12) | The owner resumed #63, #64 and #65 under one supervisor session, one Claude worker per existing worktree; the old Codex sessions stay paused. The supervisor grants native, gate and instrumentation lanes, launches independent reviews, approves exact publication text and merges serially. Work stops after these three PRs |
| Next creation rows (2026-11-13) | Owner instructions to the creation supervisor, lifting the three-PR boundary for these items only. Start row S6's second half after #63 and redesign S9 ("תפעיל אותו אחרי s7 ותתכנן מחדש את s9"); build S9 after its redesign ("אחרי שסיימת לתכנן את s9 תפעיל גם אותו"); open a fix session for S5 if a defect is recorded ("אם יש צורך בתיקונים בs5 תפעיל גם שם סשן תיקונים"); an acute-junction resolver for the corner shapes #63 leaves, as its own row S7b after #63, on the supervisor's recommendation ("מה שאתה חושב שעדיף"); a small fix of the audit's netclass pattern matcher ("תפתח"). The supervisor delegation of #63–#65 (exact publication text, serial merge after a non-author review and green gates) extends to S6's second half, S9 and an S5 fix session ("כן אותו דבר"), and to S7b as part of the accepted recommendation; for the pattern-matcher fix the exact PR text and the merge go to the owner. S9 was redesigned on 2026-11-13 as width-first routing inside one routing call; the owner accepted its three design decisions ("כן לפי ההמלצות"). S10 is not instructed. New worker sessions run on Opus 5.5 (high, or xhigh for hard tasks) or Sonnet 5.5 where that saves, by his word the same day |
| S7b split (2026-11-14) | The owner's word on the supervisor's proposal: "Yes, split". Row S7b ships as two PRs: PR #68 carries the audit's corrected acute-corner reading only; the acute-junction resolver gets its own PR, claimed on its own branch after #68 merges. Reason: the independent review of candidate A asked for changes in the resolver (four must-fix findings) while the audit's fixes were ready first |
| Process-lookup sandbox profile (2026-11-12) | The macOS sandbox refuses any exec of a setuid binary (`forbidden-exec-sugid`), so `/bin/ps` failed under `protected-native.sb` and the unchanged bench test `a_resumed_run_stops_the_freerouting_its_folder_was_left_with` failed. The supervisor approved `protected-native-ps.sb`: the same profile plus `(allow process-exec (with no-sandbox) (literal "/bin/ps"))`; every write denial still holds for every other process. Use it for cargo and native parents; keep the original for negative controls |
| Lane locks (2026-11-12) | `target/creation-research/supervision/lanes/README.md` in the main checkout: `instr.lock` for every multi-run batch (mutants, llvm-cov, guard controls); a full gate, a release/native check or anything starting real KiCad takes `native.lock` then `instr.lock`; while another worker holds `native.lock`, start no cargo run at all; a gate first waits for 20 s with no foreign cargo/rustc. Reason: gates went red on timing-sensitive tests when other cargo work overlapped them (see "Row S11", baseline observations) |
| B4 completion boundary (2026-11-10) | Owner narrowed this session to completing only PR #65, then stopping; no new creation row or feature assignment follows it. Astra reasoning is High, superseding the earlier Ultra/keep-three-active direction. Required validation, independent review, protected-resource rules and supervisor approval still apply. S7 owns AGENTS.md |
| B4 pause (2026-11-10, newer) | Owner explicitly paused PR #65 before completion. Preserve the current dirty implementation and evidence; start no code, test/build/native/instrumentation/review work until the owner explicitly resumes. This supersedes finishing PR #65 before stopping. Astra remains High; S7 owns AGENTS.md |
| B4 resume (2026-11-12) | Owner resumed PR #65 through a Claude supervisor session that directs one implementation worker in the cross-sheet worktree. This supersedes the pause row above. Scope is unchanged: finish PR #65 only, then stop; no new row starts from that branch. |
| B4 same-name sheet pins (2026-11-12) | Owner approved on 2026-11-12 ("כן, להשאיר": yes, keep it). Several pins of one name on one sheet symbol tie their parent nets together through the child's hierarchical label. The review keeps that join, stays complete, and reports it as a note (`connectivity.notes`; `circuit.connectivity_notes` in `run_design_review`), with no gap and no diagnostic. Reason: KiCad joins them, so nothing is unknown, and a diagnostic would turn a known result into INCOMPLETE. KiCad 10.0.1, `eeschema/connection_graph.cpp`, `CONNECTION_GRAPH::propagateToNeighbors`, lines 2866 to 2948: each hierarchical label looks up every parent subgraph that holds a sheet pin of its name on the same sheet instance. This replaces the "named gap" wording of verification-plan row H04. |
| S9 width-first design (2026-11-13) | Owner, to the three open decisions of the S9 redesign ("כן לפי ההמלצות": yes, as recommended). (1) When a board has recorded currents, holding the intent width is the default (`width_policy: "hold"`). (2) A connection that cannot be routed at its intent width is finished narrower and named in the result (`width_exceptions`); the row is not held open for it, and widening in place is raised later as a planner option. (3) Critical nets and differential pairs are not part of this row. The default itself changes only on his word, after he has seen the row's measurement |
| S9 pass list (2026-11-13) | Creation supervisor, on the row's step-0 measurement (19 isolated Freerouting jobs on the reference board, no product code). The power-first pass of the redesign is not built. The list is: Full (retry off, optimizer on); Fallback only if connections stay open (retry at the floor width, optimizer off); Polish only after a fallback (retry off, optimizer on). Reason: with the retry off, the single run completed in 3 of 3 with every power net at its intent width; power first also passed but is not the simplest list that does, and it lowered reference coverage and added acute corners. Two bars were not met on the pours-omitted shape (power length, worst decoupling loop) and return to the owner with the row's measurement |
| S6 second half design (2026-11-13) | Creation supervisor, under the owner's delegation of 2026-11-13. (1) Per-zone `pours` values are `fixed` and `omit` only: `fixed` already writes a Specctra plane, which Freerouting 2.5.0 reads as a conduction area that other nets may cross, so "fixed copper" and "plane" are one DSN form; the public name `fixed` stays, and the docs now say what the router receives. (2) Stitching is a next-step instruction in the routing result (`completion.next_steps`) plus the same sequence in the `kicad-pcb` skill, not a step inside the tool: `route_specctra_dsn` works on files, while apply, refill and `route_batch` need the live board, and no hidden write path is allowed. Every tool and parameter the steps name is checked against the registry by a test. (3) `status` and `completion` are additive; `success` keeps its meaning. (4) A `pours` value that is not a string or an object is refused by the handler too (the schema already refused it at dispatch) |
| S7 effort control (2026-11-10) | Owner explicitly requested configurable compute quota and a shipped-agent question about effort. Supervisor approved named quick/standard/deep levels plus a bounded custom budget with conflicting selectors refused; geometry/safety and response limits do not relax. Ask once if no effort choice/delegation exists, reuse it, ask before increasing. Backend defaults remain usable. Final numeric tiers require measured rationale; published timing guidance uses public fixtures/demos, private timings remain ignored. This does not add approval prompts to authorized validation runs |
| S7 inherited clearance (2026-11-12) | Owner approved ("כן, בתנאי הזה") the planner rule that a corner repair may keep the saved clearance of the leg it shortens instead of the extra 0.01 mm margin: only against an exactly modelled obstacle, never closer than the replaced leg, never below the nominal clearance. Reason: autorouted copper already sits at exactly the nominal clearance, so the extra margin blocked most corner repairs. **Condition:** the native matrix shows no new DRC entry on any of the 11 cases. If one case shows one, the rule is not approved as it stands: stop and report, do not tune it to pass |
| S7 inherited clearance, outcome (2026-11-13) | Owner kept the rule ("כן לפי ההמלצות שלך"), although his condition was not met as worded. What he was shown: 5 new DRC identities on 3 of the 11 cases; none involves copper the plan created or changed; no clearance, short, hole or edge entry on any case in any round; 3 are an `unconnected_items` relabel that KiCad also produces with no planner on byte-identical boards; 2 are a one-for-one exchange of a `solder_mask_bridge` entry, whose trigger was not reproduced; DRC totals on those cases fell from 167 to 152–153. No source change follows |
| S7 unmet targets (2026-11-13) | Owner approved finishing PR #63 with two original targets documented as limits ("כן לפי ההמלצות שלך"). Acute corners: 8, 5, 5, 5, 3, 3, 3 left on the seven retained boards (from 11, 5, 5, 5, 5, 5, 5); the rest are shapes outside the three proposal kinds, and on six boards one of them is a T-junction within 0.06 degrees of square. The owner was first shown 2–8; the count was corrected after his answer, reported to him the same day, and he approved 3–8 as the documented limit on 2026-11-13 ("כן, מאושר"): an earlier measurement counted one more removal per board on four boards; that was a null edit (a track end moved under 0.1 µm, the angle did not change), found and removed on 2026-11-13. Cut-off ground pads: 95–96 % of ground pads in the main group on the three plane cases, from 75 %, and 100 % elsewhere, at the default radius. The search is finished after 4 rounds, so these are limits of the tool, not a question of more rounds. No new proposal kind, no change to the default radius, no tuning |
| Rust validation (2026-11-10) | Follow the owner's current local AGENTS guidance: scoped llvm-cov where useful and targeted cargo-mutants for validation/safety guards, in isolated worktrees and outputs; preserve the gate and source-mutation negative controls. Skip these tools for this docs-only claim, apply them to S7 implementation |
| Row E1 (2026-11-16) | The owner, on three export refusals the router research found on his boards: "fix them"; for shared References, "clearer refusal but with an option to run it anyway". Reason: KiCad opens, checks and fills all three boards; a shared Reference stays refused by default because two pads would get one DSN pin name |
| Row BN (2026-11-16) | The owner chose best-of-N routing with the fastroute candidate as the default `route_specctra_dsn` engine, by effort level: Fast = 2 runs at once, M = 240 s, H = 720 s, XH = 2400 s per board. On the run cap he chose "הזמן קובע, בלי תקרה" (time decides, no cap): every budgeted level runs its whole time. He asked for XH to be measured on 2 boards ("שנה ל2"); the session chose pic and pours. Freerouting stays available as `engine: "freerouting"`. Reason: result quality is his top value, and run time is information, not a decision for him. The fork is for his personal use only, so the GPL is not a design constraint |
| Row PS (2026-11-18) | The owner approved the placement-search design's three decisions as recommended ("Yes, as recommended"). (1) No winning candidate is applied now. If the bench says the search is worth it, a later read-only planner returns the winner as `set_component_placements` batches, one functional group at a time, for the AI to compare and adopt through the normal tools (Q1); the planner never writes. (2) The AI reference A is one Sonnet 5.5 (high) session per board with the kopper1 skills, served by the fork's release binary by pipe, isolated from KiCad and from the owner's placement, placing S0 in closed-board mode. (3) Every connector and every mounting hole and fiducial is held; the owner confirms or edits each board's list once, from the counts, before the run. Reason: the design's recommendations. He started the build on 2026-11-18 ("Yes, start the build") |
| Row PS flow (2026-11-19) | The owner's flow: he draws the board outline, imports the parts from the schematic and dumps them anywhere, then places and locks in KiCad only the parts he wants fixed, and the best-of-N search places everything else around the locked parts. So: the bench's held set is his KiCad locks (plus `held.json`'s list and the parts with no pad on a net), not the connector and mounting-hole rules, and not a list he confirms from counts; P (edits of his own placement) is off by default, because it has no meaning in this flow, while O and A stay as references. This reverses decision 1 of Row PS for the later tool stage: a tool may place the winner on his board (not built yet). The order is measure first, then the tool. Reason: the bench must measure what he will actually do. Options: `bench.json` `"hold": "locks" \| "rules"` (default `locks`) and `"families"` (default `["force", "schematic"]`) |
| Row LT (2026-11-20) | The owner approved the length-tuning design's three decisions as recommended ("Yes, as recommended"). (1) Only the design intent says what is tuned: `interfaces[].length_groups` and `differential_pairs[].max_skew_mm`. KiCad's own `length` and `skew` rules are checked by KiCad's DRC on every judged run (gate G4) but never translated, and the AI mirrors each into an intent group. (2) A differential pair inside a length group is allowed: fastroute lengthens each of its nets on its own, a trailing pair group restores its skew, and the judge checks both; the result marks it "tuned net by net". (3) Row LT is built on fastroute x18 unchanged; a fastroute row (lengths cut at pad edges, 45° meander corners) starts only if the verification shows a tolerance missed on a board where it fits, or on his word. Reason: the design's recommendations (`LENGTH_TUNING_DESIGN.md`). The build waits for DP's merge and his word |
| Row LT addendum (2026-11-21) | The owner approved the addendum's decisions 4 to 6 as recommended ("Yes, as recommended"). (4) A fastroute row FT starts now: each meander is checked against its own net's copper and never laid on an earlier meander, and the pair router either reaches every pad or reports the connection unrouted; Row LT keeps its own guards (L21, L22) whatever FT does. (5) Two Tune candidates per run: tuning without limits first, then a pour-safe one (`--router.plane_as_obstacle=true`) when the first cuts a pour or misses a window, both judged against the run's untuned board (L23, L24). (6) The measure of success judges tuning within each run, by counts: no more unconnected items and no more violations of any DRC type than the run's own untuned board; the winner passes G4; KiCad agrees with L25's rules; no Tune pass discarded for another net or for opening connections; on fixture F the winner also passes G1 to G4. Reason: the verification of 2026-11-21 showed fastroute's meanders crossing their own copper, pour cuts, and an untuned control that fails G1 at M (`LENGTH_TUNING_DESIGN.md`, addendum) |
| Row LT close (2026-11-22) | The owner approved decision 7 as recommended ("Yes, as recommended"): Row LT's PR goes ahead with the via-stub fix in `changed_outside` and the x18 limits written into the PR and this file; the PR's exact title and body go to the owner for his yes before anything is pushed (AGENTS.md). For fastroute x18, decision 6's items 2 and 4 read: every G4 line and every discarded Tune pass has a named cause in the result (`untuned_nets`, fastroute's "no room", a G4(d) line), and none is a Konnect defect. Once FT lands, board B is verified again at M with decision 6 as written. Reason: M4 (`target/creation-research/length-tuning/verify/M4.md`) showed every guard working and every failure traced to fastroute x18 or to room on the board; holding the PR adds no correctness (`LENGTH_TUNING_DESIGN.md`, "M4: diagnosis and the PR verdict") |
| Row CT (2026-11-22) | The owner gave the tool to a Claude session ("המשימה שלך היא לבנות כלי") and chose both open decisions as recommended. (1) A read-only planner: `plan_convergent_placement` returns the winner as `set_component_placements` arguments and writes nothing; the board changes only through that tool, on his OK. (2) Family C's best three candidates are routed with fastroute (effort `m` by default) and the winner is named by the bench's judge, not by C's cost. Reason: no new whole-board write path (Q1); on his board the C candidate of the lowest cost did not route completely in both bench runs (Row CL, limit 3), and run time is information, not a decision for him. The rest of the design (T3 to T11) is the session's, recorded with reasons in `CONVERGENT_PLACEMENT_TOOL_DESIGN.md` |
| Scope | macOS first. Windows and Linux are validated at the end of the series |

## PR sequence

Each worker owns one creation PR on its own branch. The owner's boundary (2026-11-12, decision row "Claude resume of three PRs"): creation work is PRs #63, #64 and #65 and then stops; no new row starts without his word. The creation supervisor approves publication text and merges serially. On 2026-11-13 he started further rows (decision row "Next creation rows").

| PR | Delivers | Depends on | State |
|---|---|---|---|
| 1 | Netlist read and intent compare: `get_schematic_netlist`, `compare_netlist` | none | Merged: [#19](https://github.com/yuvaltal912-sys/Konnect/pull/19) |
| 2 | Route safety: port upstream #901, stubs (#899) and `add_wire` T-dots (#900), report `nets_joined`; PR 1 proves no net changed | 1 | Merged: [#20](https://github.com/yuvaltal912-sys/Konnect/pull/20). Review fixes: merged, [#22](https://github.com/yuvaltal912-sys/Konnect/pull/22) |
| 2b | Freerouting on real boards: DSN export accepts rounded and arc outlines and roundrect pads and keeps zones and pours as fixed copper; install path or setup guidance | 2 | Merged: [#26](https://github.com/yuvaltal912-sys/Konnect/pull/26) |
| 3 | Response budget for ERC, DRC and list tools | none | Merged: [#21](https://github.com/yuvaltal912-sys/Konnect/pull/21). Leftovers (C24 `zone_fill_state`, A30's other long results) merged: [#49](https://github.com/yuvaltal912-sys/Konnect/pull/49) |
| 4 | Inline renders as image blocks; region and side options | none | Merged: [#28](https://github.com/yuvaltal912-sys/Konnect/pull/28). Follow-ups merged: inline layer and library-item renders and the `inline_diff` difference image ([#50](https://github.com/yuvaltal912-sys/Konnect/pull/50), row R25) |
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
| 15 | ERC/DRC baselines | 3 | Merged: [#43](https://github.com/yuvaltal912-sys/Konnect/pull/43). Follow-up: `run_design_review` reads a DRC baseline, merged [#47](https://github.com/yuvaltal912-sys/Konnect/pull/47) |
| 16 | Widen the Specctra DSN profile so Freerouting fits real boards: back-side parts, more than 2 copper layers, then a Freerouting version policy and a benchmark on KiCad's demo boards (rank 32: D7, B34, D24). PR 2b already did rounded/arc outlines, roundrect pads and zones as fixed copper | 2b, 13 | Merged: [#44](https://github.com/yuvaltal912-sys/Konnect/pull/44). Part 2 (version policy, benchmark) merged: [#51](https://github.com/yuvaltal912-sys/Konnect/pull/51) |
| 16b | Route at the project's rules: no Freerouting neck-down, the importer enforces class widths, copper-to-edge clearance in the DSN, power widths from design-intent currents (IPC-2221) | 16, Q3, 11 | Merged: [#58](https://github.com/yuvaltal912-sys/Konnect/pull/58) |
| 29 | Circuit-aware review against the owner's layout checklist (sections 2, 6, 8, 9) and the power tree from the design-intent rails (rank 29: B22, B23 partly) | Q3, 11 | Merged: [#55](https://github.com/yuvaltal912-sys/Konnect/pull/55). Follow-ups: derating (B23), datasheet-driven values, hierarchy-joined nets |
| R21 | Board construction (rank 21: A13, B25): footprint lock/unlock, rule areas (keepouts) that survive the DSN export, zone delete, polygon board outlines | 2b | Merged: [#48](https://github.com/yuvaltal912-sys/Konnect/pull/48) |
| R25 | Inline layer and library-item renders (rank 25: C6, A25, B9, B7): board layers, a symbol and a footprint returned as MCP image blocks, plus a generated difference image for `inline_diff` (PR 4 follow-ups) | 4 | Merged: [#50](https://github.com/yuvaltal912-sys/Konnect/pull/50). Not done: B9's further 3D camera options (left/right/front/back, pan, rotate, multi-view batch) |
| R33 | Local part import (rank 33: B20, rescoped 2026-11-08): copy a KiCad-format symbol, footprint and optional 3D model the owner exported or downloaded (EasyEDA/LCSC export, SnapEDA, Ultra Librarian) into a project-local library, preview then apply by revision, never overwrite, register the libraries, run the pin-pad check. No network access: EasyEDA's terms (2026-09-21) and robots.txt forbid automated fetches | none | Merged: [#52](https://github.com/yuvaltal912-sys/Konnect/pull/52). The tool is in `integration`: `library` is at the 20-tool cap |
| LQ | Layout quality audit `audit_layout_quality`: return paths, decoupling loops, bottlenecks and one comparable score, against the owner's checklist | 11 | Merged: [#56](https://github.com/yuvaltal912-sys/Konnect/pull/56) |
| FR | Freerouting started without telemetry, outside the home folder and off the network | 16 | Merged: [#57](https://github.com/yuvaltal912-sys/Konnect/pull/57) (MCP spawn), [#51](https://github.com/yuvaltal912-sys/Konnect/pull/51) (`--version` probe, bench) |

## Next: what to pick

**Now (2026-11-22, Row GS).** The smart-router switches (S1, F1 and the group search) are in `route_specctra_dsn`, merged as [#108](https://github.com/yuvaltal912-sys/Konnect/pull/108) (main `6b31bf90`, a noreply merge whose tree equals the PR head `3fc339a9`). **Nothing to pick here:** the row is done. What is left is the owner's: a 1 Kopper package with x19 and its install (the installed Konnect and the fastroute at the default place are still x18, which this main refuses), and whether the effort budgets grow. See "Row GS".

**Now (2026-11-23, Row ZR).** Zones in the router: [`ZONE_ROUTING_DESIGN.md`](ZONE_ROUTING_DESIGN.md), merged as [#102](https://github.com/yuvaltal912-sys/Konnect/pull/102). On 2026-11-22 the owner lifted "no build work yet" ("תן לסשן השני להתחיל לכתוב את הקוד של הzones", relayed by the supervisor session): decision 1 as recommended, ZR0 then ZR1. Stage ZR0 merged as [#110](https://github.com/yuvaltal912-sys/Konnect/pull/110) (main `13c71913`). Stage ZR1 merged as [#113](https://github.com/yuvaltal912-sys/Konnect/pull/113) (main `900e8fe1`), with fastroute x20 pinned (engine branch `zr1-fill-model`, `c13a28f`): no false connection in 1,545 compared runs. Routing with this main needs x20 installed; the supervisor session installed the 1 Kopper package with x20 and the ground-via prune on 2026-11-23. Stage ZR2 merged as [#115](https://github.com/yuvaltal912-sys/Konnect/pull/115) (main `ac8b5e01`), with fastroute x21 pinned (engine branch `zr2-truth`, `20d7c96`): the model's groups as connectivity and the closing stage. Routing with this main needs x21 installed; the installed 1 Kopper package has x20, and a package with x21 needs the owner's word. ZR3 waits for his word; its design must add thermal relief to the cost, to the "never more pour damage" rule and to the done criterion (his note, relayed 2026-11-23). ZR0b, the ground-via pruning, is the supervisor session's. See "Row ZR". Its addendum "The design on x19", merged as [#106](https://github.com/yuvaltal912-sys/Konnect/pull/106), brings it in line with fastroute x19 (Row GS); docs only. See "Row ZR".

**Now (2026-11-22, Row CP).** Row CP, the convergent placement search (family C in the placement bench), merged as [#98](https://github.com/yuvaltal912-sys/Konnect/pull/98) (main `dc3c3e60`). On copies of two of the owner's boards, C's best placement ranks first and beats his own. Three limits it found are open as "Row CL" (limits 1, 3 and 4; limit 2 merged as #97), each on the owner's word. Row CT, C as a placement tool (`plan_convergent_placement`), was built on the owner's word before limits 4 and 1, which it inherits and reports, and merged as [#104](https://github.com/yuvaltal912-sys/Konnect/pull/104) (main `6abf6333`; see "Row CT").

**Now (2026-11-22, Row LT).** Row DP merged as [#93](https://github.com/yuvaltal912-sys/Konnect/pull/93) (main `d758fbab`, a noreply merge whose tree equals the PR head `bd8a009a`). Row PS's changes since #92 (the schematic-neighbour measure D23, generators 7 and 8, the routing record's pair fields) merged as [#94](https://github.com/yuvaltal912-sys/Konnect/pull/94) (main `a68a0fa7`, a noreply merge whose tree equals the PR head `503ba63f`). Row LT (length tuning) is being built on branch `claude/length-tuning` (the design branch `claude/length-tuning-design-674041` with main `0a3fc504` merged in): the owner approved its three decisions on 2026-11-20 and started the build the same day. [`LENGTH_TUNING_DESIGN.md`](LENGTH_TUNING_DESIGN.md) holds the design, the skeletons, the tests and the verification plan. See "Row PS" and "Row DP". **2026-11-21:** the build (`claude/length-tuning` `735ce4ae`, not pushed) failed its verification at M (`target/creation-research/length-tuning/verify/RESULTS.md`). The root causes were found and designed for in the design's addendum (built in stages 5a and 5b, see the end of this paragraph): fastroute's meanders cross their own net's copper (F5), the Tune pass's autorouter stage removes held copper (F6), meanders cut other nets' pours (F3), and KiCad checks one rule per constraint type (F4). The owner approved its decisions 4 to 6 as recommended on 2026-11-21 (decision row "Row LT addendum"). The FT design (fastroute's tuner and pair router) is starting in its own session, in fastroute's worktree. Measurements M1 to M3 ran on 2026-11-21 (`target/creation-research/length-tuning/verify/ADDENDUM_MEASURES.md`). M1 passed as step 0b, M2 confirmed L25, and M3 confirmed L24's purpose; the design owner's verdict is in the addendum's "Step 0b results": no design change, and the LT bodies of L20 to L27 were built as designed. M1b runs at effort M. F1 (the judge refusing live-exported boards) is its own PR from main. Still open: Row RJ's first live run. **2026-11-21, stage 5a of the LT addendum (built, local commits on `claude/length-tuning`, not pushed):** the Tune pass runs with the autorouter off (L20); `tune_attempt` runs up to 3 rounds and drops the nets whose new copper touches itself, with their pair partners and pair group (L21, exact repeats of one piece are one piece); `tune_held` runs a `Free` and a `PourSafe` candidate (L24) and returns a `TuneOutcome`; `held.untuned` and `held.tune` carry the result. **2026-11-22, stage 5b (built, local commits `45db7178` and `a46d7a80` on `claude/length-tuning`, not pushed):** L22, G4(d): a checked net whose judged session crosses or overlaps itself (exact repeats are one piece; touches are not gated), or that KiCad's DRC names in a `track_dangling` or `unconnected_items` entry, fails G4, and its miss is its window's lower bound (`LengthMeasures.paths`). L23: `judge_candidates` judges the untuned session (`run-<i>-untuned`) and each Tune candidate (`run-<i>`, `run-<i>-pour-safe`), `tune_harm` rejects a candidate worse than the untuned board in unconnected items, DRC violations (not the two length keys), G3 lines or tracks below their floor, the best rank of the rest is the run's board, else the untuned one; the pour-safe candidate is judged only when the free one is rejected or fails G4. `tune_held` no longer replaces `held.ses`; the scheduler rewrites `Routed.held` from the choice (`TuneChoice::apply`), so the written SES, `kept_pass` and `result_pass` are the judged board's. L25: a skew group that holds a gated pair is suggested as a `length` rule pinned to the winner's longest net. L26: a reduced run keeps the `fastroute.log` and `tune.ses` of each discarded or failed Tune pass, and every run but the winner loses its `run-<i>-untuned` and `run-<i>-pour-safe` judge folders. L15: `length_matching` gains `untuned_nets` and `tune_candidates`. L27: the design's measure of success now judges tuning within each run. Checks on `a46d7a80`: the whole konnect-core lib 4559 passed, 0 failed (a first run had one failure, stage 5a's timing test `a_round_is_not_started_when_the_deadline_leaves_less_than_twice_the_last_pass`, under machine load; it passed in the rerun); the `konnect` guards `asset_references` 12 and `schema_parameter_usage` 5 passed; `cargo fmt --all --check` and `clippy -D warnings` are clean; 55 revert checks, every one caught (`stage5b/revert-checks.md`); one narrow mutants batch of 49, 46 caught, 0 missed, 3 unviable (`stage5b/mutants-summary.md`). Not verified: any real routing or KiCad run (the judge's tests use a fake kicad-cli), and the owner's M4 measure. Record: `target/creation-research/length-tuning/stage5b/`. **Build review, 2026-11-22** (`target/creation-research/length-tuning/review2.md`; verdicts in the design's "Build review" section; fixes in `5bceebe6` and the commit after it): the untuned board joins the rank and wins an exact tie (S1); `tune_harm` compares numbers, adding undersized nets, narrow paths, routed share and audit completion and dropping the G3 line count (S2); in a call with checks the judge's scratch projects, the baseline's too, set `track_dangling` to at least `warning` and `unconnected_items` to at least `error` (S3); `untuned_nets` lists every tune-file net the written session leaves untuned (S5); a faulted pair-only net adds its pair's longer length as its miss (S6); `apply` is an error for a variant the run does not hold, which fails the judge (N1); `tune.rows` say `candidate`, `kept` or `not_chosen` (N2); `raw_copper` skips a `qarc` (N3); the self-contact scan runs in `spawn_blocking` (N4); the pinned `min` is at least 0 (N5); discard reasons say "(no round left)" or "(the deadline leaves no room)" (N7); `tune_attempt` takes a clock, so its timing test no longer depends on load (S4). Checks: the whole konnect-core lib 4573 passed, 0 failed (one run in between had two other stage-5a timing tests, `a_pass_killed_by_the_deadline_ends_the_run_even_after_full_succeeded` and `a_tune_pass_killed_by_the_deadline_ends_the_run`, fail under load; both passed in the rerun on the same tree); `asset_references` 12 and `schema_parameter_usage` 5 passed; fmt and `clippy -D warnings` clean; 35 new or changed guards each revert-checked and caught (`stage5b/revert-checks-review.md`). No mutants batch. M4 ran at effort M (`target/creation-research/length-tuning/verify/M4.md`; diagnosis in `LENGTH_TUNING_DESIGN.md`, "M4: diagnosis and the PR verdict"): no chosen board is worse than its own untuned board, KiCad agrees with G4 under L25's rules (9 `length_out_of_range` plus 1 `skew_out_of_range` on B, 19 on A, 0 on F), board F passes G1 to G4, and the fastroute-to-KiCad gap fell from 38 mm to at most 0.72 mm. **The x18 limits:** every G4 line on A and B has a named cause that Row LT cannot remove. On A (19 lines at x18, 31 with the FT build) the cause is nets the tuner drops for copper that touches itself (17 nets, each in `untuned_nets`) and then no room for meanders on the inner layers. On B (11 lines) it is the same drops plus dangling held copper from the pair router on `C_N`, `D0_N` and `D1_N` (G4(d) lines); FT fixes the first and the dangling copper, and board A may never pass G4. **The via-stub fix** (`changed_outside` ignores a missing held piece of a net outside the tune file when both ends lie within the copper disc of one of that net's own vias; `specctra::via_disc_radii` reads the disc from the call's DSN) removes the one false discard M4 found: round 2 of B's free candidate named `C_N` for a 20 µm stub that fastroute drops at load. The pour-safe candidate's "opened 2" is fastroute's own count and stays. Checks on the fix: the whole konnect-core lib 4599 passed, 0 failed; `asset_references` 12 and `schema_parameter_usage` 5 passed; fmt and `clippy -D warnings` clean; 17 revert checks caught (`stage5b/revert-checks-stub.md` and `-stub2.md`), and a test that a real export's via padstacks each have a disc at half their diameter; the one line in `route()` that passes the radii into the Tune pass has no revert check and is exercised end to end by the re-run on B. **Next:** the PR on the owner's word (its title and body shown to him first); after FT lands, board B is verified again at M with decision 6 as written. **2026-11-22:** main `74dccf2a` is merged into this branch: F1, the judge taking a live-exported board by its tree at each run's import, merged as [#96](https://github.com/yuvaltal912-sys/Konnect/pull/96) (main `dddcd769`) and installed with `cargo xtask kopper-install` the same day (smoke 13 of 13); the custom-pad export merged as [#97](https://github.com/yuvaltal912-sys/Konnect/pull/97). Row RJ's first live run is still to run.

**Merged meanwhile (2026-11-22).** Row CP, the convergent placement search (family C in the placement bench), merged as [#98](https://github.com/yuvaltal912-sys/Konnect/pull/98) (main `dc3c3e60`); see "Row CP" for what is verified and what is still open, and "Row CL" for the four limits it handed to Codex. This branch has that main merged in.

**Before (2026-11-19).** Row PS, the placement-search bench, merged as [#90](https://github.com/yuvaltal912-sys/Konnect/pull/90); its owner-flow change (the bench holds only his KiCad locks and runs F and S, not P) merged as [#92](https://github.com/yuvaltal912-sys/Konnect/pull/92) (main `8795de82`, a noreply merge whose tree equals the PR head `0fc4ef63`). The owner has locked his parts on the copies in `target/creation-research/placement-search/lock-copies/`: B4 6, B5 10, B6 6 footprints (he left the B4 and B6 connectors unlocked on purpose). B4 to B6 have not run; the first run was his new board "dash" (see "Row PS").

**The 1 Kopper series** is done: Row K4 merged as [#77](https://github.com/yuvaltal912-sys/Konnect/pull/77) (main `25ea6fe1`). The fastroute source is backed up and the merged worktrees are removed (owner's yes, 2026-11-17).

**Row K3** merged on 2026-11-17 as [#76](https://github.com/yuvaltal912-sys/Konnect/pull/76) (main `f5c9c8ec`).

**Row K2** merged on 2026-11-17 as [#75](https://github.com/yuvaltal912-sys/Konnect/pull/75) (main `bb4255ff`).

**Row K1** merged on 2026-11-17 as [#74](https://github.com/yuvaltal912-sys/Konnect/pull/74) (main `3d585574`).

**Now (2026-11-17, Row BN).** Best-of-N routing with fastroute merged as [#72](https://github.com/yuvaltal912-sys/Konnect/pull/72) (`1c11922a`); the candidate is fastroute x18 since 2026-11-17 ([#73](https://github.com/yuvaltal912-sys/Konnect/pull/73), `6570e3d4`). See "Row BN".

**Now (2026-11-13, Row S7).** Implemented, reviewed and gated on `codex/creation-s7-route-refine`, base `f246e48dab3994de9a0a6fa97176ab8238ce1db9`, with main (`e166d97`, PRs #64 and #65) merged in. Publication and merge go through [PR #63](https://github.com/yuvaltal912-sys/Konnect/pull/63); GitHub is the source of truth for its state and merge commit.

- **Boundary:** creation work is PRs #63, #64 and #65 and then stops (`AGENTS.md`). The creation supervisor approves publication text, grants the native and instrumentation lanes, and merges. A worker never pushes, edits the PR or merges on its own.
- **State:** see "Row S7" below for what was built, the decisions, the evidence and the limits.
- **Next step:** none on this branch once #63 is merged. The rows the owner started on 2026-11-13 (S6 second half, S7b, S9, the pattern-matcher fix) each begin on their own branch from the merged main, under the creation supervisor.

**Current S11 state (2026-11-13):** see "Row S11: suspected zone-fill defects"
below. Implemented, reviewed and gated on `codex/creation-s11-zone-fill-defects`
(2026-11-13); publication and merge go through PR #64, and GitHub is the source
of truth for its state and merge commit. The following S6 instructions are
historical.

**Current B4 state: implemented, reviewed and gated on `codex/creation-cross-sheet-review` (2026-11-13).** This branch holds only cross-sheet circuit review. Its base is `fork/main` at `f246e48dab3994de9a0a6fa97176ab8238ce1db9` (merged #62). Publication and merge go through [PR #65](https://github.com/yuvaltal912-sys/Konnect/pull/65); GitHub is the source of truth for its state and merge commit. S7 and S11 are separately supervised; their states live in their own handoffs. The inherited S6 text below is historical. See "Backlog B4" for scope, evidence and the next step.

### S6 reconciliation retained for future agents

PR #62 is merged at `f246e48dab3994de9a0a6fa97176ab8238ce1db9`. Fresh `git fetch fork main`, `gh pr view 62 --repo yuvaltal912-sys/Konnect` and `gh pr list --repo yuvaltal912-sys/Konnect --state open` confirmed the merge and only simulation #54 open. The merge has two parents, the required noreply author, and the same tree as PR head `aeac8d9c2d094e2f6eb6c5e5cd7a60a818ab3905`. Local main and its unrelated guidance edits/untracked `.serena/` were preserved.

The supervisor owns S6's completed worktree `/Users/yuvaltal/.codex/worktrees/konnect-s6-preset/Konnect` and its local docs-only closeout `56e0188`; it was read, not changed or cherry-picked. Last full S6 gated runtime revision: `ea5734885e4693c6ee814f4634d3735d0e352acc`. S6's source/hash reconciliation is retained under `target/creation-research/s6/implementation/resume-reconciliation/results.json` in the main checkout. S6's runtime and private measurement history remains below and under `target/creation-research/s6/`; no routing sweep was repeated. The old [S6_RESUME_PROMPT.md](S6_RESUME_PROMPT.md) and approved [S6_PR_DRAFT.md](S6_PR_DRAFT.md) are historical S6 records, not instructions to recreate the PR.

**Claim rule.** Pick a "Not started" row whose dependencies are all merged. Before writing code, push a first commit on your branch that changes that row's state to "In progress (branch `<name>`)" and open a draft PR, so no other agent takes it. For a partially integrated row, claim its continuation on a new branch and draft PR before implementation. If another row is "In progress", don't touch the code files it names. The push and draft PR wait for the creation supervisor's approval of the exact title/body under the owner's 2026-11-10 delegation (`AGENTS.md`); until then the claim commit stays local.

**Owner direction (2026-11-08):** routing many traces is easy; routing them optimally is what he wants, on boards like his (power and mixed-signal, roughly 30–150 parts, 2–4 layers, pours, high-current nets). [`LAYOUT_CHECKLIST.md`](LAYOUT_CHECKLIST.md) is the standard: return paths, decoupling loops, protection and termination placement, widths for current. Completion is only a gate; he wants to give the currents of the main traces himself, through `set_design_intent`.

**Reference board:** a private, git-ignored copy of one owner-chosen board (100 parts, 2 layers) and its scripts are in `target/creation-research/reference-boards/` (`apply_ses.py` writes an SES into a board copy with KiCad's `pcbnew`, then refill and `kicad-cli pcb drc`). Keep its results out of tracked docs apart from counts.

**Next rows**, from the owner's resolution plan (`target/creation-research/RESOLUTION_PLAN.md`, local and git-ignored). Each starts only on the owner's OK. Inside a row he has started, measurement and check runs, Freerouting included, go end to end with no per-run OK (his instruction, 2026-11-10, which replaced an OK for every Freerouting run): one stop at the end, with the results and the exact PR title and body.

| Row | Delivers | Depends on | Files it owns | State |
|---|---|---|---|---|
| S5 | Measure first: a settings and DSN-variant sweep on the reference board through #51's harness, each run scored with `audit_layout_quality` plus length by net category (ground, power, signals); picks one preset. No product code | merged | `xtask/src/freerouting_bench/` (`mod.rs`, `report.rs`, new `board_score.rs`), new `xtask/freerouting-bench/score_board.py` and `stage_merge.py`, `DEV.md` | Merged: [#60](https://github.com/yuvaltal912-sys/Konnect/pull/60). 116 isolated jobs, preset chosen (section "Row S5" below) |
| S6 | Ground through the pour: no intent widening for pour nets, a per-zone `pours` policy in the DSN, stitching after refill | S5's result | `specctra.rs`, `specctra_ses.rs`, `tools/pcb_export.rs`, `freerouting_mcp.rs`, `tools/integration.rs`, `specctra_pours_tests.rs`; later pour fixtures | First half complete in [#62](https://github.com/yuvaltal912-sys/Konnect/pull/62); owner authorized merge on 2026-11-10 (verify PR state). Second half (per-zone policy, stitching after refill), started by the owner on 2026-11-13: implemented, reviewed and gated on branch `claude/creation-s6b-pour-policy` (2026-11-14); publication and merge through [#66](https://github.com/yuvaltal912-sys/Konnect/pull/66); GitHub is the source of truth for its state. Three targets unmet: 0 unconnected (on C1 and C2), power-net width shares no worse than C0 (on C1, C2 and C3), and at least 40 ground vias (on C2); see "Targets, by shape" under "Row S6, second half" |
| S7 | Stitching-via and corner clean-up planner, read-only (Q1: a second opinion, never the author) | merged | new `tools/route_refine.rs` with tests/fixtures; narrow `layout_quality*` reuse seams; routing registration/counts; `kicad-pcb` skill and API docs | Implemented, reviewed and gated on branch `codex/creation-s7-route-refine` (2026-11-13); publication and merge through [#63](https://github.com/yuvaltal912-sys/Konnect/pull/63), GitHub is the source of truth for its state. See "Row S7" |
| S7b | The audit's corrected acute-corner reading (owner decision 2026-11-13), one reading for the acid-trap check, the 5.3 scan, the planner's corner rows and the routing bench; then an acute-junction resolver, a fourth proposal kind | S7 | Reading: `tools/dfm.rs`, `tools/layout_quality_routing.rs`, the `corner_reading` module in `tools/mod.rs`, their tests, two of PR #63's tests in `tools/route_refine_tests.rs`, `xtask/src/freerouting_bench/quality.rs`, the acid-trap line of `tools/design_review.rs` and `tool-directory.md`, the migration note, the `kicad-review` skill, `DEV.md`'s bench line. Resolver: `tools/route_refine*.rs` | **Reading:** implemented, reviewed and gated on branch `claude/creation-s7b-junction-resolver` (2026-11-14); publication and merge through [#68](https://github.com/yuvaltal912-sys/Konnect/pull/68); GitHub is the source of truth for its state. **Resolver:** authorized by the owner on 2026-11-13, split out on his word of 2026-11-14 (decision row "S7b split"); reserved for the creation supervisor's worker, to be claimed on its own branch after #68 merges; its first build is kept on a local branch of the worker's worktree. Not in #68 because the review of candidate A found four must-fix findings in it. See "Row S7b" |
| A1 | Replace the audit's recursive netclass pattern matcher with a linear one | S7 | `tools/layout_quality.rs` | Implemented, reviewed and gated on branch `claude/creation-audit-pattern-matcher` (2026-11-13); publication and merge through that branch's PR; GitHub is the source of truth for its state. See "Row A1" |
| S9 | Width-first routing inside one `route_specctra_dsn` call: a net widened for its recorded current keeps that width (one full run with Freerouting's retry off; a fallback pass and a polish pass only when connections stay open), and every connection finished below its intent width is named in the result | S5, S6 (both halves), S7 | `freerouting_mcp.rs`, `tools/integration.rs`, new `specctra_seed.rs` and `intent_width_report.rs` with their tests and fixtures; own entries in `API_MIGRATIONS.md`, `tool-directory.md` and the `kicad-pcb` skill | Implemented on branch `claude/creation-s9-width-first` (2026-11-14), opt-in: the default does not change. Redesigned 2026-11-13; see the decision rows "S9 width-first design" and "S9 pass list". Review, measurement and the gate are recorded in [#69](https://github.com/yuvaltal912-sys/Konnect/pull/69); publication and merge go through it, and GitHub is the source of truth for its state. See "Row S9" |
| S10 | Only if signals stay above x1.03 after S6 and S9: re-route the worst nets | S9 | an evaluation script under `xtask/freerouting-bench/` first | Not started |
| S11 | Report zone/pad or zone/via reported-zero/short candidates; distinguish newly qualifying pairs after a valid refill comparison without establishing engine cause or routing correctness | merged | new `tools/zone_fill_defects.rs` and its tests, a few lines in `tools/drc.rs`, `TROUBLESHOOTING.md` | Implemented, reviewed and gated on branch `codex/creation-s11-zone-fill-defects` (2026-11-13); publication and merge through [#64](https://github.com/yuvaltal912-sys/Konnect/pull/64), GitHub is the source of truth for its state. See "Row S11" |
| B4 | Circuit review across sheets (#55/rank-29 follow-up): join supported hierarchy connectivity, retain instance identity and incomplete results for unresolved hierarchy | #55 and integration merged | `tools/circuit_model.rs`, `tools/circuit_review_tests.rs`, new hierarchy fixtures; narrow `tools/circuit_review.rs`, `tools/design_review.rs`, `tools/circuit_checks.rs`, `tools/design_review_circuit_tests.rs`; own shared-doc entries | Implemented on `codex/creation-cross-sheet-review` (2026-11-13): three review rounds, gate green. [#65](https://github.com/yuvaltal912-sys/Konnect/pull/65); GitHub is the source of truth for its merge state |
| E1 | Three export refusals of boards KiCad accepts (found on the owner's boards, 2026-11-16): outline ends within KiCad's 0.01 mm chaining distance, intent net names with outer spaces, and shared References (a clearer refusal, plus `allow_duplicate_references` to export them under unique DSN names); and teardrop zones, which were exported as pours | merged rows | `specctra.rs`, `specctra_ses.rs` (tests), `tools/pcb_export.rs`, `tools/design_intent.rs` and its tests, `API_MIGRATIONS.md`, `tool-directory.md` | Implemented, reviewed and gated on branch `claude/export-board-quirks` (2026-11-16); open as [#71](https://github.com/yuvaltal912-sys/Konnect/pull/71); GitHub is the source of truth for its merge state. See "Row E1" |
| BN | Best-of-N routing with fastroute as the default `route_specctra_dsn` engine: seed variants and both pour shapes, run in parallel within the effort level's budget, each run judged by Konnect on a scratch copy (apply, refill, DRC, stitching, layout audit), the winner chosen by absolute gates and a rank key; Freerouting stays available | E1, S9, S6 | new `fastroute.rs`, `route_search.rs`, `route_judge.rs`, `scratch_board.rs`, `tools/route_fastroute.rs` and their tests; `tools/integration.rs`, `tools/cli.rs`; `ROUTE_SEARCH_DESIGN.md`; own entries in `API_MIGRATIONS.md`, `tool-directory.md` and the `kicad-pcb` skill | Merged: [#72](https://github.com/yuvaltal912-sys/Konnect/pull/72) (`1c11922a`, 2026-11-17). fastroute x18 and the pic rerun on it: branch `claude/route-best-of-n-x18`. See "Row BN" |
| R2 | Rest of rank 32: custom and trapezoid pads, as a convex hull per layer | S6, S9 | `specctra.rs`, `specctra_ses.rs`, new `specctra_pads_tests.rs` | Not started |
| R3b | Footprint rule areas (antenna keepouts), cut-outs and Edge.Cuts slots as real DSN keepouts, replacing today's refusal | S6, S9 | `specctra.rs`, `specctra_ses.rs`, new `specctra_geometry_tests.rs` | Not started. Goes first if a board with an RF module arrives |
| R3a | Local clearance overrides, conservatively | S6, S9 | `specctra.rs`, `specctra_ses.rs`, new `specctra_clearance_tests.rs` | Not started |
| R4 | Copper text as KiCad's own glyph shapes over IPC | S6, S9 | `konnect-ipc/src/client.rs`, `tools/pcb_export.rs`, new `specctra_text_tests.rs` | Not started |

The R rows all edit `specctra.rs` and `specctra_ses.rs`, so they go one at a time; a new manifest field must land in both, because the importer refuses unknown fields. Blind and buried vias stay parked.

**Current boundary:** creation work is PRs #63, #64 and #65 plus the rows the owner started on 2026-11-13 (decision row "Next creation rows"): S6's second half, S7b, S9 and the pattern-matcher fix. No other row starts without his word; S10 is not authorized. S5's measurements change what each should be (section "Row S5", "What it means for the next rows"): S6's first half (stop widening the net that owns the pours, and send the measured settings) is supported as it stands; its second half (pours as planes) leaves ground connections open that S7's stitching proposals address only in part ("Row S7", native matrix); S9's design (power first, fixed) lost completion and did not help the widths, so it needs rethinking before it is built.

**Backlog after these**, best first: the audit sees ground and the skills say when to run it; current capacity (IPC-2152 derating, voltage drop, pour and via capacity) inside `audit_trace_current`; net classes from the design intent, with a check that intent net names exist (rank 38); circuit review across sheets (#55's follow-up); footprint text placement on the board (rank 20); footprint drag with track stretch (rank 36); an impedance estimator and stackup writer (D18); a placement planner as a second opinion (Q1, D6); planned-copper clearance and a free-space finder (C15, C18).

### Backlog B4: cross-sheet circuit review (PR #65), implemented

**State, 2026-11-13.** Worktree `/Users/yuvaltal/.codex/worktrees/konnect-cross-sheet-review/Konnect`, branch `codex/creation-cross-sheet-review`. The implementation is the commit that carries this text, on top of the claim commit `ff66f25`, based on `fork/main` `f246e48`. The supervisor approves the exact title and body before the push and merges [PR #65](https://github.com/yuvaltal912-sys/Konnect/pull/65) with the noreply command under "Known traps"; GitHub is the source of truth for the push and merge state. The owner paused this work on 2026-11-10 and resumed it on 2026-11-12 through a Claude supervisor session; scope is PR #65 only, then stop. The state at the pause is preserved in `target/creation-research/cross-sheet/pause-2026-11-10/`.

**What changed.** `audit_circuit_design` and the circuit part of `run_design_review` now judge one circuit joined across the hierarchy instead of each sheet alone.

- Nets join through global labels, global power symbols and matching parent sheet pins and child hierarchical labels. Local labels and local power stay inside their sheet use. A sheet used several times keeps each use's own designators and units.
- What cannot be read or joined is a named gap, never a pass: a missing or unreadable child, a sheet symbol with no UUID, a root with no UUID, an unresolved symbol, bus and variable names, hidden power pins. One cause is one gap. Every gap is counted in `coverage.unevaluated_subjects` and makes `run_design_review` INCOMPLETE.
- A conclusion that an unseen connection could change is withheld as `unproven`. A conflict that is already proven stays a finding. For bus termination that means a bank below 114 ohm is reported even when the bus is incomplete or the end assignment is unknown.
- Thresholds and the set of components a search may cross are unchanged.
- `docs/API_MIGRATIONS.md` lists every output change.

**Files.** `crates/konnect-core/src/tools/circuit_model.rs`, `circuit_review.rs`, `circuit_checks.rs`, `design_review.rs` (the review seam only), their tests `circuit_review_tests.rs` and `design_review_circuit_tests.rs`, the fixtures under `crates/konnect-core/tests/fixtures/circuit_hierarchy/`, and this branch's entries in this file, `docs/API_MIGRATIONS.md` and `tool-directory.md`. Not touched: `sch_connectivity.rs`, `sch_hierarchy.rs`, `konnect-sexp`, `xtask/`, `AGENTS.md`.

**Owner decision to carry forward: same-name sheet pins are a note.** Several pins of one name on one sheet symbol tie their parent nets together through the child's label. The review keeps that join and reports it in `connectivity.notes`, with no gap and no diagnostic. KiCad joins them: KiCad 10.0.1, `eeschema/connection_graph.cpp`, `CONNECTION_GRAPH::propagateToNeighbors`, lines 2866 to 2948. The owner approved this on 2026-11-12. It replaces the "named gap" wording of verification-plan row H04; the plan carries an appended amendment (`target/creation-research/cross-sheet/preparation/VERIFICATION_PLAN.md`). The decision row is in the table near the top of this file.

**Verified, 2026-11-12, on the source of this commit (review freeze `review-C`, tree `551c6ceddc4bc90c5ce747185cb116de262f6822`; the commit adds only this handoff text to that tree).** Every cargo parent ran under `/usr/bin/sandbox-exec -f /Users/yuvaltal/ClaudeCode/Konnect/target/creation-research/supervision/native-isolation/protected-native-ps.sb`, with a scratch `KICAD_CONFIG_HOME` and a signature of the installed Konnect and the KiCad preferences taken before and after.

| Check | Command or record | Result |
|---|---|---|
| Focused tests | `cargo test -p konnect-core --locked --lib -- tools::circuit_review::tests tools::design_review::circuit_tests` | 202 passed, 0 failed |
| Full gate | the nine commands under "Verify" in `docs/SIMULATION_HANDOFF.md`, under both lane locks with no other cargo or rustc process running; `resume-2026-11-12/native4b-review-C/` | green: fmt clean; workspace tests 4308 passed, 0 failed, 163 ignored; doctests 9 passed, 2 ignored; `simulation_live` 95 passed; Clippy `-D warnings` clean; licenses ok; `fix-doc-counts` no change; release build ok |
| Earlier red gate, same source | `resume-2026-11-12/native4-review-C/` | red: workspace tests 4302 passed, 6 failed, 163 ignored, while another agent's cargo tests ran outside the locks; the other eight commands exited 0. The six tests are named below and passed alone before the rerun |
| Guard controls | one guard removed or inverted per run, one named test each; `resume-2026-11-12/controls-C.json` | 100 controls in the final list, each killed by its named test in a recorded batch: 25 on the `review-C` source (the ones the last change touched), 75 on the `review-B` source. Not rerun as one batch on the final source |
| Mutation | `cargo mutants --in-place --in-diff <patch>`, tests limited to the two modules | first freeze, whole diff: 364 mutants, 260 caught, 12 unviable, 92 not caught; of those 92, 55 are caught by tests added later, 31 got a new test with a killed control, 2 are equivalent, 1 never terminates, 1 sat in code since removed, 2 sat on a line since rewritten and were caught in the next pass. Change to the second freeze: 79 mutants, 74 caught, 4 unviable, 1 missed (the 114 ohm bound; a test and two controls were added). Change to the third freeze: 27 mutants, 25 caught, 2 unviable, 0 missed |
| Coverage | `cargo llvm-cov` on the two modules; line and region, not branch coverage | every line changed since the first freeze executes. Across the whole PR 12 changed lines never execute (6 in `circuit_model.rs`, 2 in `circuit_review.rs`, 3 in `circuit_checks.rs`, 1 in `design_review.rs`): defensive branches and one scoped-audit line these two modules do not reach |
| Reference replay | baseline `f246e48` and this candidate, each an identified release binary, on one frozen private snapshot; counts only | unproven subjects 12 to 0; all 9 earlier findings kept, 3 new; 2 unevaluated subjects kept; none unexplained |
| KiCad fixtures | three synthetic hierarchies (14, 22 and 22 physical pins) exported by KiCad 10.0.8 on 2026-11-10; the model's pin partitions are compared with the exports in the focused tests | match |
| Independent review | three rounds by an agent that did not write the code; reports under the main checkout's `target/creation-research/supervision/reviews/pr65-review-{A,B,C}.md` | round C: 0 blocker, 0 major, 0 minor |

The checks that can only run on the commit itself (the gate on the committed source, and the release build's `get_installation_info` naming this fork's executable and the commit) are recorded in `implementation/reports/checkpoint-C.md` and in the PR body.

Evidence is under `target/creation-research/cross-sheet/` in this worktree (ignored by git, do not delete): `implementation/PROGRESS.md` is the ledger, `implementation/reports/checkpoint-{A,B,C}.md` the reports, `implementation/resume-2026-11-12/` the run records. The frozen review candidates are in `/Users/yuvaltal/.codex/worktrees/konnect-pr65-claude-freeze/`.

**Not verified.** Windows and Linux. A run in KiCad's GUI. Hosted CI. Branch coverage (the tooling gives line and region coverage only). The mutation runs cover the changed code of the four production files against the two test modules; they are not a sweep of the crate. No real-project result is in any tracked file, apart from the counts above.

**Baseline observation: six timing-sensitive tests fail under concurrent load.** They are in files this PR does not touch, and no fix belongs in this PR.

- `tools::integration::freerouting_setup_tests`: `a_banner_followed_by_a_startup_crash_is_not_available`, `a_java_that_cannot_start_the_jar_does_not_make_it_available`, `a_java_that_meets_the_jar_is_chosen_over_one_that_does_not`, `a_version_printed_before_a_nonzero_exit_counts_and_the_probe_is_quiet` (a stand-in `java` does not answer in time).
- `tools::pcb_sync::tests`: `completion_unnetted_copper_is_not_complete_connectivity`, `completion_whole_net_reassignment_preserves_copper_and_counts_readback` (the mock KiCad IPC receive times out).
- Red run: the first gate on this source, with another agent's cargo tests running at the same time. Green rerun: the six alone (6 passed), then the whole gate on a machine with no other cargo or rustc process, same source. A red gate on these tests is a reason to check for other cargo runs before reading it as a defect.

**What another agent needs to know before running anything here.**

- Lane locks: `/Users/yuvaltal/ClaudeCode/Konnect/target/creation-research/supervision/lanes/README.md`. A full gate takes `native.lock`, then `instr.lock`, waits for 20 s with no foreign cargo or rustc process, and runs alone; a mutation, coverage or guard-control batch takes `instr.lock`; no cargo run starts while another worker holds `native.lock`.
- The sandbox profile is `protected-native-ps.sb` (SHA-256 `6e2798f115fe9233a71ef892af3965462af3fcfc44241820fb7a563c02917bac`). The older `protected-native.sb` refuses macOS's setuid `/bin/ps`, which one benchmark test needs.
- `crates/konnect-core/build.rs` watches a git ref path that does not exist in a worktree, so the build script reruns on every build here and debug artifacts are not byte-identical between builds. Identify a binary from Cargo's JSON messages for that invocation and keep a copy of any binary used as evidence.
- The retained baseline binary `implementation/baseline-bin/konnect` (SHA-256 `f451b2e15802546060989e309be91c0edbcab6db9a7721b2d841cb2fc1c3dd5c`) embeds `f246e48`. Product metadata does not record working-tree state, so a build from a dirty tree is identified by its recorded source hashes, not by HEAD.
- The installed Konnect must stay at SHA-256 `3de188bb775ab4ce5748ea44b94cb098c86ed5d32135e70f4c2c3817c3a4ac65`. Do not restore the owner's KiCad preferences, even if they look changed: on 2026-11-10 the original baseline run saw two of his settings files saved during its interval, writer unproven, and they were left as found.

**Next concrete step.** None on this branch once [PR #65](https://github.com/yuvaltal912-sys/Konnect/pull/65) is merged; check GitHub. Until then: the supervisor approves the exact PR title and body, the worker pushes with `git push fork HEAD:refs/heads/codex/creation-cross-sheet-review` and updates the PR text, and the supervisor merges with the noreply command under "Known traps". No new row starts from this branch; the next creation row waits for the owner's word.


PR 13's rule editing, deletion and static expression workflow is complete within
its documented limits. Wider custom-rule conversion into DSN remains part of the
later routing-profile expansion; unsupported rule semantics refuse export.

**Checks still owed** (not blocking, but say so if your PR depends on them):

- `apply_specctra_ses` of a Freerouting session into a live KiCad, then refill and DRC (PR 16 proved export, routing and the import plan; the live apply is not run).
- PR 9 against a live KiCad: `crates/konnect/tests/live_kicad_tools.rs`, on a disposable board.
- PR 14 against a live KiCad: checkpoint, edit in the GUI, restore refused while open, restore after closing, on a disposable project.
- PR 12 against a live KiCad: `route_batch`, `modify_trace` and `delete_via` read back an exact identity match, and one undo reverts.
- #48 live: Freerouting routes around a rule area that `add_rule_area` wrote.
- #52 live: a real vendor export (EasyEDA/LCSC, SnapEDA or Ultra Librarian) imported and placed in a live KiCad.
- Windows and Linux for the whole series, at the end.

**Known traps.**

- `every_simulation_demo_flattens_or_names_its_export_failure` is flaky when another test run is using `~/Library/Caches/KiCad` at the same time (IBIS demo, "Cannot find U1.cache"). Rerun it alone.
- Use the noreply identity from `AGENTS.md`, including merge commits. Since the owner turned on email privacy (2026-11-08), GitHub merges work with `gh pr merge N --repo yuvaltal912-sys/Konnect --merge --match-head-commit <full sha> --author-email 274043415+yuvaltal912-sys@users.noreply.github.com` (first used for #42); check the merge commit's `%ae`. If GitHub refuses, stop and ask: never substitute a personal email or fast-forward `main`. Pushes are refused when the branch tip carries a personal email, so put a noreply commit on top.
- Two sessions edited `CREATION_HANDOFF.md`, `tool-directory.md` and `API_MIGRATIONS.md` at once and conflicted each time. Keep every PR's rows and entries when you resolve.
- Three `freerouting_setup_tests` (fake Java probes) can fail under a full parallel `cargo test --workspace` and pass alone. Rerun them alone.
- Under heavy machine load (several sessions running tests at once, load average above 30) the IPC mock tests in `pcb_sync` and `custom_rule_tests` time out (`GetOpenDocuments … Timed out`). They pass alone; rerun them before calling a gate red.
- Freerouting 2.5.0 needs Java 25 (class file 69); Java 21 fails with `UnsupportedClassVersionError`.
- Freerouting 2.x turns telemetry and analytics on by default and keeps its settings and log under the user's home (2.5: `~/Library/Application Support/freerouting`, `~/Library/Logs/freerouting`; 2.1: `java.io.tmpdir/freerouting`). 2.5.0 also sends a version check to `api.github.com/repos/freerouting/freerouting/releases/latest` on every start, with no switch to turn it off. Every place Konnect starts the JAR is isolated and offline: the MCP spawn in `freerouting_mcp.rs` (`route_specctra_dsn` and the `check_freerouting` MCP probe, #57), the `--version` probe in `tools/integration.rs` and the bench (#51), and `xtask/freerouting-bench/quiet-java.sh`. Each passes `--profile.allow_telemetry=false`, `--usage_and_diagnostic_data.disable_analytics=true`, `-Djava.awt.headless=true`, a scratch `-Duser.home` and `-Djava.io.tmpdir`, and sends every HTTP and HTTPS request to a closed loopback proxy (`127.0.0.1:9`; loopback stays direct through `http.nonProxyHosts`). The MCP spawn and the `--version` probe also drop every inherited `FREEROUTING__*` variable and point `XDG_*_HOME`, `APPDATA` and `LOCALAPPDATA` into the scratch folder, because 2.5.0 reads those ahead of `user.home`; the MCP spawn also passes `--user_data_path`, and removes its scratch folder only after the child has exited. Checked live on 2.5.0 (2026-11-09, isolated home): the scratch log says "Analytics are disabled" and `Version check skipped or timed out: java.net.ConnectException`, and no connection leaves loopback. The JSON Freerouting writes still shows `allow_telemetry: true`; the log line is the evidence. The MCP spawn seeds no `freerouting.json` (the flags make it unnecessary); the bench seeds one with a `profile.id` (next item). A new place that starts the JAR must reuse `jvm_arguments`, `server_arguments`, `user_data_path_argument` and `isolate_environment` in `freerouting_mcp.rs`.
- Freerouting 2.3 and later stop with a `NullPointerException` when their `freerouting.json` has no `profile.id`.
- Freerouting 2.5.0's job API (MCP `update_job_settings`) drops unknown keys without an error. It reads the snake_case names its settings serialize under (`neck_width_um`, `copper_to_edge_clearance_um`, `job_timeout`, `autorouter.max_passes`); camelCase was silently ignored until PR 16b's follow-up. Read the settings back with `get_effective_settings`, as `route_specctra_dsn` now does.
- A `freerouting.json` in the real home (`~/Library/Application Support/freerouting/`) fills every setting a run leaves out, and Freerouting writes settings back to it. Any run without an isolated home inherits hidden settings, which is how the dropped keys stayed hidden. Benchmark and route only with an isolated home (#51, #57).
- `optimizer.improvement_threshold` is a percentage, but a value between 0 and 0.1 is read as a fraction and multiplied by 100. On the command line the fraction rule is wider: `--router.optimizer.improvement_threshold` above 0 and below 1 is multiplied by 100 as it is parsed (`CliSettings`), so `0.25` runs as 25 % and the optimizer stops after its first pass (seen in S5's sweep; its log said "below the threshold (25.00%)"). Give 0.25 % as `0.0025`.
- KiCad 10.0.8's iterative zone refill can pour a lower-priority zone that overlaps a higher-priority zone of the same net over other nets' pads (seen at 0 mm on two connector pads; the copper at those pads was identical in a clean run). `ZoneFillIterativeRefill=0` in `~/Library/Preferences/kicad/10.0/kicad_advanced` avoids it for the GUI, IPC and kicad-cli; KiCad's standalone Python (`pcbnew` scripts) ignores that file. Run DRC after every refill.
- A scratch `KICAD_CONFIG_HOME` needs copies of the global `fp-lib-table` and `sym-lib-table`, or KiCad's library checks change and a DRC baseline no longer matches.
- Freerouting 2.5.0's fanout stage (on by default) necks a track down at the pin even with `automatic_neckdown` off; the importer then refuses the SES (seen as 0.15 mm wires in a 0.2 mm class). The owner's real-home `freerouting.json` turns fanout off, so every run that inherited it looked clean; the first isolated-home run did not. `route_specctra_dsn` sends `fanout.enabled: false`.
- Freerouting 2.5.0 treats a `copper_to_edge_clearance_um` within 1e-9 of 250 as its own default (`HeadlessBoardManager.applyCopperToEdgeClearanceOverride`): it skips the override, or clamps it to the outline's minimum pin gap, logging at debug only, while `get_effective_settings` still reports 250. A project asking for exactly 0.25 mm would route closer to the edge than asked. `route_specctra_dsn` sends 250.001.
- Freerouting 2.5.0's retry width (`neck_width_um`) is one value for every net, applied to any failed connection (`AutorouteConnectionRouter.retryConnectionNecked`, no per-net filter). Send it only when no net's floor is above it.
- Konnect's Freerouting JAR search does not look in `~/.local/share/freerouting/<version>`; pass `jar_path`.
- On macOS a KiCad binary (`kicad-cli`, KiCad's Python) finds its settings folder without reading `HOME`, and so does anything that starts one, the `konnect` binary's start-up probe included: with `HOME` pointed at `target/dogfood-home` and nothing else, the real `~/Library/Preferences/kicad/10.0/kicad_common.json` was re-saved once (seen 2026-11-10; content kept). Set `KICAD_CONFIG_HOME` to a scratch folder for every such child, as the bench's `--score` does.
- Freerouting 2.5.0's autorouter stops by itself when its score stalls ("has not improved by more than 0.5 points since pass #10. Stopping the auto-router after 20 passes"), whatever `autorouter.max_passes` allows. More passes do not finish a board it has given up on.
- Its optimizer runs on many threads and is not repeatable: one config gave the same copper in 12 of 16 runs on the reference board and three other routings in the other 4, one of which ran until the job budget. Compare configs only over several runs, with the spread beside the comparison.
- `optimizer_scoring.excess_*_weight` settings switch the optimizer off in effect: its score reads 0 and it keeps nothing (two cells of S5's sweep, 6 runs).
- A hand-written `(autoroute_settings (layer_rule …))` scope in a DSN made Freerouting 2.5.0 read the board with no pins and one connection, route nothing and log no warning. There is no tested way to set one layer's trace costs.
- With pours given as planes, Freerouting reports 0 unrouted while KiCad, after the refill, finds ground connections open (16 to 29 on the reference board): the router counts a pad inside a plane's outline as connected, and the real pour is cut by the new tracks. Judge completion by `kicad-cli` on the refilled board, never by Freerouting's count.
- Wiring given as fixed (`(type fix)`) cannot be moved by the router: with the power nets routed first and fixed, it left connections unrouted that it finishes when it routes everything at once.
- `export_specctra_dsn` needs a live KiCad. For a file-based DSN of a scratch board, run the ignored test `freerouting_routes_a_folder` with `KONNECT_PROBE_KEEP=<dir>` and `FREEROUTING_JAVA=/usr/bin/true`: the export is written and nothing starts Freerouting.
- Which settings a Freerouting run really used is in its file log, not on the console and not in the `freerouting.json` it saves back (that holds no router settings). 2.3 and later log "Applied CLI router setting: router.<path> = <value>" at debug level under the run's home, and after routing a "Routing Parameters" block with the via costs and each layer's trace costs. A setting that did not take is only a warning ("Failed to apply CLI router setting"), and Freerouting routes on with its default. Left alone, Freerouting also sets each layer's against-direction trace cost from the board's shape, so a sweep must give that lever itself. `-mt` is `router.max_threads` in 2.5.0; the optimizer's threads are `router.optimizer.max_threads` (default: cores less one). The bench reads all of this for every run and fails a run whose given setting was not applied as given.
- The plain workspace test run (`cargo test --workspace`, the gate's second line) re-saves the owner's real `~/Library/Preferences/kicad/10.0/kicad_common.json`: only `system.working_dir` changes, to a temp path. It was first seen right after the `protocol_stdio` test binary starts, and no test sets `KICAD_CONFIG_HOME`. It is on main, so it is a baseline finding, not any branch's. A fix was proposed to the owner as its own small PR; he has not answered and it is not started. Whether exporting `KICAD_CONFIG_HOME` for the whole gate avoids it is not tested.
- The repository setting "Allow auto-merge" is on since 2026-11-10 (the owner's choice). No PR has auto-merge set, and setting it on a PR still needs his word. The fork has no CI, so auto-merge would merge a ready PR at once, and the author email of such a merge commit has not been checked.
- `target/` in the main checkout is more than a build folder: `target/creation-research/` holds the owner's plan, the private reference board, the review records and S5's raw outputs, and two git worktrees sit under `target/` as well. None of it is in git, so `cargo clean` there would delete it for good. To build elsewhere, set `CARGO_TARGET_DIR`.
- Java 25 for Freerouting is at `/opt/homebrew/opt/openjdk@25/bin/java`; the system `java` is 11. The 2.5.0 JAR is at `~/.local/share/freerouting/2.5.0/freerouting-2.5.0.jar`. Both are on this Mac only.
- The MCP server named `konnect` in an agent's tool list, Claude's or Codex's, runs the installed binary (`~/.local/share/konnect/bin/konnect`). A call to it never tests this checkout: use `target/release/konnect` by pipe ("Verify" in `docs/SIMULATION_HANDOFF.md`).

## Row K1, 1 Kopper: simulation off by default

**What 1 Kopper is.** The owner's one name, since 2026-11-16, for this project: Konnect's creation work plus fastroute as the default best-of-N router, without the simulation branch. It is for his personal use. Its scope was set by the owner on 2026-11-17:
- Draft PR #70 (S7b resolver) and PR #54 (simulation) stay out.
- Simulation already merged into main stays in the code and is hidden, not removed.
- Only worktrees whose branch is merged get cleaned up, and only after he sees the list.
- It is built in its own worktrees, from main after #72.

The plan is four PRs:
- **K1** hides simulation.
- **K2** is one install for Konnect and fastroute: it also decides which skills are installed, so the `kicad-simulation` skill is K2's.
- **K3** is a smoke test of the package.
- **K4** is the docs.

**State, 2026-11-17.** Merged on the owner's word as [#74](https://github.com/yuvaltal912-sys/Konnect/pull/74): main `3d585574`, a noreply merge whose tree equals the PR head `ed24beaa` (branch `claude/1kopper-k1-hide-simulation`, base `1c11922a`, with main `6570e3d4` (#73) merged in).

**What K1 changes.** A config key `simulation_enabled`, default `false` (`crates/konnect/src/config.rs`). When it is off, `main.rs` calls `McpHandler::hide_toolset("simulation", …)`, which calls `ToolRouter::hide`. Then:
- `list_toolboxes` leaves the toolset out, and `all_toolsets` is the filtered list.
- `load_toolset("simulation")` answers that it is turned off and names the key.
- `load_all` and eager startup skip it.
- Its tools are unknown, even with `auto_load_toolsets`.
- No simulation runtime is made, so `get_installation_info` reports `ngspice.probe_status` as `not_available`.

The library default is unchanged: `ToolRouter::new` and `McpHandler::new` still serve every toolset, so embedding callers and the core tests see no change. The live simulation tests turn the key on in every server they start.

**Evidence.** Recorded in the PR description: the targeted tests, one negative control per guard, the independent review and the full gate.

## Row CP, the convergent placement search (family C)

**State, 2026-11-22.** Merged as [#98](https://github.com/yuvaltal912-sys/Konnect/pull/98) on the owner's word (main `dc3c3e60`, a noreply merge whose tree equals the PR head `e90bf484`). Built on `claude/convergent-placement-design` from `fork/main` `0a3fc504`, with `fork/main` `74dccf2a` (#96, #97) merged in; reviewed twice, measured on copies of two of the owner's boards with routing, mutants and the full gate below. Design: [`CONVERGENT_PLACEMENT_DESIGN.md`](CONVERGENT_PLACEMENT_DESIGN.md). The owner approved it with its three decisions as recommended on 2026-11-20 ("כן, כמומלץ") and started the build the same day ("תתחיל בבניית ההצבה"). He asked for Opus 5.5 (high) agents to write the bodies from the design's skeletons; the supervising session runs on Claude Fable 5.2.

**What it is.** A fourth family, C (`converge`), in the placement bench of Row PS. It has one cost per placement, a search that refines the candidates of F and S while the cost falls, and three stop rules that read the cost and nothing else: for a run, for the starts and for the effort. It adds no tool, no apply path and no registry change.

**Stages.** One agent at a time in the worktree; each stage is verified and committed before the next starts.
1. The seams. Goldens of F and S boards recorded before the change; `Legality` and `GapRule` pulled out of `legalize_turned`; the new types (`Family::Converge`, `ConvergeRecipe`, `BoardModel::hoods`, `Candidate::trace`, `Matrix::converge`); a geometry entry in `tools/placement/objectives.rs`; `place_converge.rs` as the design's skeleton. Done when the existing `place_` tests and the goldens pass unchanged.
2. The cost model: `CostModel::build`, `state`, `terms`, `legal`, `delta`, `apply` and `state_from`, with their tests.
3. The search: the moves, making room, the first temperature, annealing, descent, `converge`, `starts_stop` and `effort_stop`, with their tests.
4. The bench: `converge_all`, the records and `rerun`, the selection, `search.json` and `table.md`, the `families` option and the bench README.
5. An independent review of the diff and its fixes; scoped coverage and mutants on the guards; the gate.
6. The measurement of the design's "Measurement" section, steps 1 to 4.

**Verified.**
- **Baseline gate** on `ff01d66e` (the design commit), 2026-11-21, green: workspace tests 5183 passed, 0 failed, 170 ignored; sandboxed fastroute 38; doc tests 9; `simulation_live` 95 of 95; fmt, `clippy -D warnings`, `deny`, `fix-doc-counts` changing nothing, the release build, protected files unchanged, the installed binary's SHA-256 starting `3de188bb775ab4ce`. Record: `target/creation-research/convergent-placement/gate/baseline`.
- **Stage 1** (an Opus 5.5 agent, 2026-11-21): fmt and `clippy -p konnect-core --all-targets -D warnings` clean; `cargo test -p konnect-core --lib place_generators` 79 passed, 0 failed, 1 ignored (the goldens' recorder), 70 before the stage. 80 boards of F and S (seeds 1 to 8, every parameter set, the demo board as built and with a 0.7 mm part gap) equal SHA-256 goldens recorded before the change. 12 negative controls, each seen to fail its test: the gap rule in both arms, the part sides, the edge clearance, the copper obstacles, the start's family, C off unless named, the recipe's tag, the version, `Record::group`, `rerun`'s refusal. On the stage's commit `910e3007`: `--lib place_` 258 passed, 0 failed, 2 ignored (247 and 1 before the stage); `--lib tools::placement` (the placement tools' goldens) 122 passed, 0 failed, 1 ignored.
- **Stage 2** (an Opus 5.5 agent, 2026-11-21): the cost model. fmt and clippy clean; `--lib place_converge` 26 passed, 0 failed, 1 ignored (a timing test); `--lib place_generators` 79 passed; `--lib objective` 19 passed; `--lib tools::placement` 122 passed, 0 failed, 1 ignored, so the score tool's output is unchanged. The model's airwire length and crossings equal `evaluate`'s on the fixtures and after 1,000 random legal moves; after 10,000 applied moves the caches equal a full evaluation. 30 negative controls, each seen to fail its test. One `delta` takes 1.1 µs on the demo board and 4.0 µs on a synthetic board of 96 parts (release). Calls the agent made, kept: a pad is placed through the anchor as written (rounded to 1 µm), so the model's pad sites are the written board's bit for bit; `tools/placement.rs` opens `mod objectives`, `CONNECTOR_EDGE_LIMIT_MM` and `ref_prefix` to the crate (visibility only); a held part named in a move makes the move illegal. Not covered by a test: labels of real KiCad footprints (knockout, `justify`, several lines, a render cache); the speed on a dense real board.
- **Stage 3** (an Opus 5.5 agent, 2026-11-21): the search. fmt and clippy clean with the temporary allow line gone and no `todo!()` left; `--lib place_converge` 48 passed, 0 failed, 3 ignored (timing tests); `--lib place_generators` 79 passed. 31 negative controls: 28 failed their test, and 3 could not, each for a reason that is now recorded (below). Release runs with the measured parameters, every one ended by its rules (annealing `Quiet`, descent `FixedPoint`): the demo board 1.9 s at level 0 (about 260,000 evaluations) and 3.8 to 4.6 s at level 1; a synthetic board of 96 parts 8.3 s at level 0 (1.56 million evaluations) and 15.4 s at level 1.
  - **Findings to carry.** (1) On the demo board, level 1 was no better than level 0: runs end in one of two cost groups (about 60 to 78, or about 102 to 107), and which one is seed noise; the effort rule reads one run per level, so on such a board a bad level-1 run ends the ladder at once. On the 96-part board level 1 was 3.4 % better. (2) The design's control for the swap ("without the test of a swap's two parts against each other, an overlap appears") cannot fail: a swap keeps the two parts' offset, so it cannot make them overlap. The line is guarded by the model-level test of stage 2, and it matters for the compound move. (3) Making room is shown to work and is not shown to be needed: on the test board a whole run without it still ends with the pair in range, because annealing frees the cell while it is hot. (4) The control "the end is the board as written" passed on the fixtures, whose anchors are on the grid: a fixture with off-grid anchor offsets is still to add.
  - On the stage's commit `23d5a9a4`: `--lib place_` 306 passed, 0 failed, 5 ignored; `--lib tools::placement` 122 passed, 0 failed, 1 ignored.
- **Stage 4** (an Opus 5.5 agent, 2026-11-21): the bench wiring. fmt and clippy clean; `--lib place_` 319 passed, 0 failed, 6 ignored (run before a last rename of one JSON key in the ignored harness); `--lib place_converge` 48 passed; `--lib place_generators` 79 passed. 24 negative controls, each seen to fail its test, the design's named ones among them. `bench.json` gets `"converge"` in `families` (refused without `force` or `schematic`) and `"converge_terms": "on" | "off"`; the new ignored test `converge_check` runs phases 1 and 1b with no routing call and writes `converge-check.json`.
  - **The dry run on the public demo board** (release, 15 threads, the measured parameters, no routing): 160 starts available; 6 starts taken, the rule for starts stopped by its rule; the best start ran again at level 1, which did not pay, so the rule for effort stopped by its rule after one level; all 7 runs ended by their rules (annealing quiet; descent at a fixed point in 5 and quiet in 2), none by a guard. Airwire length fell from 1143 to 1194 mm at the starts to 642 to 729 mm, crossings from 33 to 39 to 5 to 9, and no run ended with a decoupling or connector violation. Phase 1b took 241 s.
  - **Findings to carry.** (1) A level-0 run takes 63 to 105 s on this board, 0.3 to 0.5 ms for each costed move, against 5 to 8 µs on the test boards: the cause is not yet profiled. (2) Level 1 ran 1.34 times the stages of level 0 on the same start and ended higher (919.9 against 901.6): one reading. (3) Label conflicts rose in 5 of 7 runs with the label term on.
  - On the stage's commit `60c9be73`: `--lib place_` 319 passed, 0 failed, 6 ignored; `--lib tools::placement` 122 passed, 0 failed, 1 ignored.
- **Two independent reviews** of `ff01d66e..60c9be73` (Sonnet 5.5 agents that did not write the code, read-only, no test run): one on the cost model and the search, one on the legalizer's pull-out and the bench wiring. Neither found a wrong result, an illegal board, a dependence on the thread count, a hang, a panic, private data in an output, or a change to what F, S and P produce. One point the design leaves open, kept as built: C's routing calls rotate through the round-robin with F, S and P, so a time limit that cuts calls costs every family alike. Their findings are missing tests and small things, taken in the fix round (below).
- **The performance pass** (an Opus 5.5 agent, 2026-11-21). The profile: 98.6 % of a run on the demo bench board was the attraction's search for the nearest legal cell (997 full legality tests per search, most of each in `Outline::overhang`). The search now skips the cells a nearby part blocks, by rows and columns found with the same `clear_of`, and tests the rest in the same order (8 tests per search); `Legality::legal_at` runs its four pure tests cheapest first. A level-0 run on that board went from 43 to 49 s to 0.84 to 0.95 s, and phase 1b from 238.8 s to 6.4 s. **No result changed:** 17 whole runs are pinned by goldens recorded before the change (the board's SHA-256, stages, sweeps, evaluations, both stops, and the bits of the first temperature and of every best cost), on four boards, copper obstacles and a rounded outline with a cut-out among them; and the harness's 12 runs on the demo bench board are identical before and after, apart from seconds. `--lib place_converge` 49 passed, 0 failed, 4 ignored; `--lib place_generators` 79 passed; `--lib objective` 19 passed; fmt and clippy clean. Two negative controls on the goldens (the mask inverted, the outline test disabled), each seen to fail. Found on the way: stage 1's goldens do not guard the outline test (their boards are rectangles); the new goldens' rounded board does.
- **The fix round** (an Opus 5.5 agent, 2026-11-21): the reviews' missing tests and small things, with no search result changed (both golden tests pass untouched). New tests, each with a control seen to fail: a movable connector's cost in the caches, the level inside annealing, the stop rule's definitions (the gain, what a proposal is, a zero-change move), a whole run by a curved outline and a cut-out, descent's guard, the fallback first temperature, the attraction itself, the end of the trace as written on off-grid anchors, the nearest-legal-cell search against a cell-by-cell search, `converge-check.json` holding counts only, the search with family C and no readable schematic. Changes: `search.json` has no `converge` key when family C is off; each C row of `table.md` says its start and level; the starts run at level 0 whatever the parameters hold; one ring order shared by `legalize` and the search. `--lib place_` 339 passed, 0 failed, 7 ignored; `--lib place_converge` 60 passed, 0 failed, 4 ignored; `--lib place_generators` 79 passed; `--lib objective` 19 passed; fmt and clippy clean. Three of the first controls did not fail (the quiet gain, the proposal count, the outline test in a whole run): each test was strengthened until its control failed.
- **Measurement step 1, first run** (commit `d4ef576e`, release, 15 threads, the copy of the owner's board, no routing): phase 1 took 15.3 s (160 starts) and phase 1b 28.7 s. 8 starts were taken and the rule for starts stopped by its rule; the best start's level 1 did not pay, so the rule for effort stopped by its rule after one level; all 9 runs ended by their rules, none by a guard. **The port's check failed:** costs 1208.5, 1210.7, 1259.8, 1342.5 and 4009.2 (one pair out of range), a median of 1259.8 against the 1245 fixed in advance. One of the search's own 9 runs also ended with a pair out of range, so "does making room work" fails on this run too.
- **The diagnosis** (an Opus 5.5 agent, scratch edits only, on the same board copy; 30 to 100 runs per question): two causes, both now amendments in the design (A1, A2). The attraction's target was on a 0.5 mm grid where the experiment's was on 0.25 mm, which made most attraction draws name no move; and the first temperature's estimate was a coin toss between a cold group and a hot one, with the cold runs the ones that leave a pair out of range. The legality test, the cost, the shifts, the turns and the swaps agreed with the experiment.
- **The two amendments, built** (an Opus 5.5 agent, 2026-11-21, commit `d413bf8c`): the attraction's target on the 0.25 mm grid, and the first temperature from 4,000 trial shifts tried from the start and not made, read on the soft cost alone. The 17 run goldens were recorded again, once, for this change; the 80 board goldens of F and S are untouched. `--lib place_converge` 65 passed, 0 failed, 4 ignored; `--lib place_` 344 passed, 0 failed, 7 ignored; fmt and clippy clean.
- **Measurement step 1, second run** (`d413bf8c`; the same start, seeds and threshold): **the port's check passes**, costs 1136.2, 1148.2, 1206.1, 1226.3 and 1235.6, a median of 1206.1 against 1245; no pair out of range in any of the 5; first temperatures 204 to 229. The search itself: 5 starts taken and the rule for starts stopped by its rule; effort levels 0, 1 and 2 cost 1360.2, 1297.4 (pays) and 1366.0 (does not), stopped by its rule; all 7 runs ended by their rules with no pair out of range; phase 1 took 15.1 s and phase 1b 69.7 s.
- **Measurement steps 2 and 3, the bench on the copy of the owner's board** (`d413bf8c`, release test binary with SHA-256 starting `bf5d71eca3e5d9ee`, effort M, 100-minute limit, families F, S and C; under `gate.lock`, `native.lock` and `instr.lock` on a quiet machine): step 2 took 4606 s and step 3, the same run with both new terms off, 4652 s; each routed 15 candidates (O 3, F 4, S 4, C 4).
  - **Step 2: C's best ranks 1, beats O and passes every gate**, with 1336.7 mm of routed signal against O's 1738.0 (0.77 times), 69 vias off pours against 82, copper quality 57.6 against 59.7, no decoupling and no connector violation, and 172 silk entries against 109. F's best ranks 7 and S's best 8, each with 2 decoupling violations. Two more C candidates pass every gate and rank 5 and 6, below O, by 1.0 and 3.0 acute corners. C's outer loops: 5 starts, levels 0 to 2, both by their rules, 0 of 7 runs ended by a guard, 66 s.
  - **Step 3 (terms off): C's best ranks 1 again** (1359.2 mm, 71 vias off pours, 59.5, 154 silk entries), and two more C candidates rank 2 and 3, above O. 5 starts, levels 0 to 1, both by their rules, 0 of 6 by a guard, 42 s.
  - **In each run the C candidate with the shortest routed signal failed G1** (25 and 26 unconnected items; a different start each time) and ranks 11: the cost does not see whether a placement routes completely.
  - **The question about the new terms is not settled.** Read on each run's best candidate, silk fails (172 against 154) while parts pulled away (3 against 5) and length pass. That compares two different placements; on the two starts both runs routed at level 0, the terms on gave fewer silk entries both times (161 against 176; 172 against 189). The design now reads the question in pairs, by a rule fixed before any paired run (its amendment A3).
  - The filled table is in the design, under "The answers on the owner's board".
- **The paired reading (A3)** with `KONNECT_CONVERGE_SILK=1` (`521c1ac9`; the switch reproduces the bench's silk entries on the owner's board exactly: 161, 172, 161, 158): the neighbour term passes on three boards of three (parts pulled away 4.8 against 6.8, 1.83 against 2.17, 34.8 against 62.8) and the label term on two of two (silk 161.4 against 164.4, and 212.5 against 231.2; B6 cannot be read). **Both weights stay**, so step 2 is the measurement of the final weights. `--lib place_` on `521c1ac9`: 346 passed, 0 failed, 7 ignored.
- **Step 4, B5** (`521c1ac9`, effort M, 4309 s, under the measurement locks; with the A reference, one Sonnet 5.5 session of 16 minutes through `a-kit`): 18 routed (O 3, A 3, F 4, S 4, C 4), all passing every gate. **C ranks 1 to 4 and beats O and A**: 778.6 mm of routed signal against 1026.3 and 1021.6, 42 vias off pours against 39 and 40, copper quality 58.9 against 66.9 and 66.5, 168 silk entries against 0 and 91. F's best ranks 11, S's best 14. C's loops: 8 starts, levels 0 to 1, both by their rules, 0 of 9 by a guard, 38 s.
- **B6**: family C runs and stops by its rules, but the Specctra export refuses the board (a pad of shape `custom`), for the owner's placement too, so nothing is routed. **B4**: family C generates nothing ("no room for 1 part" under its gap rule). Both are in "Row CL".
- **The A arm ran for the first time.** `a-kit` (private records): a neutral start, tool calls by pipe on the fork's binary, and a hand-in in the form `admit_ai` accepts; proved on the public demo board (A admitted and routed; a moved held part refused).

**Records.** `target/creation-research/convergent-placement/`: the lane driver `chunk.sh` (every cargo command of this row goes through it), the gate, `PROGRESS.md` and the agents' reports. The experiment behind the design is in `target/creation-research/convergence-study/`.

**Owner decision, 2026-11-21:** the bench of step 4 runs on two boards, B5 and B6, not three ("אנחנו מריצים על 2 לוחות עכשיו לא על 3"; he chose the two). B4 gets no bench run in this row.

- **The independent review of `60c9be73..HEAD`** (a Sonnet 5.5 agent that wrote none of it, read-only, nothing built or run; private record `reports/review-final.md`): **no finding of the first rank** (no wrong result, hang, panic or private data in an output). It confirms that the cell skipping of the performance pass gives the cell a full ring-order test gives, that `legal_at`'s reordering changes no verdict, that A1 and A2 are built as written with nothing of the old estimate left, and that nothing depends on the thread count, hash order or time. Its findings, fixed in the round below:
  - B1: the silk switch calls `place_route::unrouted_check`, which does not require the DRC report's unconnected-items category as the bench's own baseline (`route_judge::baseline`) does; a degraded report could be recorded as 0 silk entries with no reason.
  - B2: `GENERATOR_VERSION` stays 9 although family C's output for one recipe changed with A1 and A2; an old C record fails `rerun` at the board hash, not at the version.
  - Tests: the nearest-legal-cell equivalence test builds its precondition into itself; making room has no cell-by-cell comparison; the run goldens were recorded again on the new code, so they freeze behaviour and do not prove it; the goldens' bits depend on `exp`, `ln` and `powf` and may differ on Linux or Windows; no test of `silk_check` (it needs a real kicad-cli) or of the NaN path in `legal_at`.
  - Small: a silk check that fails for every run still ends as a passing test; a failure in `isolated_context` loses the search result; `first_legal_cell` wants an assert that its span is at most 64; README wording.

- **The fix round for that review** (a Sonnet 5.5 agent on the owner's word, "כל האייג׳נטים על sonnet5.5"; then "תמשיך בסבב התיקונים"): B1, the unrouted check now refuses a DRC report without the unconnected-items category, through the judge's own requirement (`place_route::unrouted_counts` over `route_judge::unconnected_total` and `violations_by_rule`), so the silk switch records `null` with a fixed reason and never 0. This also makes the bench's input check refuse such a report up front (`input_check_failed`), where before every candidate failed later in the judge for the same cause. B2, `GENERATOR_VERSION` is 10; neither golden fixture holds the version, so both are byte-identical. New tests: the nearest-legal-cell search against a plain ring-order search over 240 random legal states of four fixture boards; making room's guarantees on random states of 7 boards; a centre or size that is not finite is never legal. Small: a compile-time bound on the cell search's span; the silk harness fails when it reads none of its runs; a failure to start the silk check is a fixed reason instead of a lost result; the goldens say their bits are recorded on macOS (Apple Silicon). The search is bit-identical (both golden tests pass unchanged). Checked by the supervisor on the heavy lane: fmt and `clippy -D warnings` clean; `--lib place_` 352 passed, 0 failed, 7 ignored; the one control the agent could not run (the unrouted check's wiring undone) seen to fail its test and pass again when restored. Every other new test's control was seen to fail by the agent (private record `reports/fixes2.md`); two first controls did not fail and are explained there (one an equivalent mutant).

- **Mutants on the guards** (cargo-mutants 27.1.0 on the verify worktree at `76601327`, a Sonnet 5.5 agent; trimmed on the owner's word, "תקצר את מה שאפשר מבחינת המוטציות"; private record `reports/mutants.md`). Ran: `Legality::legal_at` (90 mutants: 76 caught, 9 timed out, 5 missed), its helpers `clear_of`, `gap`, `gap_between`, `copper_obstacles`, `copper_sides`, `keepout_box` (52: 46 caught, 2 timed out, 2 missed, 2 unviable), and one chunk over `first_legal_cell`, `first_temperature`, `quiet_enough`, `starts_stop`, `effort_stop`, `Trace::converged` and `OuterRule::pays` (148: 98 caught, 9 timed out, 41 missed). Every timeout is a mutant that makes the search run to its limit (caught by a hang, not by an assertion). Of the missed: 40 in `first_legal_cell` change only how many cells it tests, not which cell it returns (its pruning only saves calls of `legal`); 5 in `legal_at` are ties against a bound plus 1 µm that floating point hardly reaches. **Two real gaps, now pinned by tests** written by the supervisor, each seen to fail under its mutant: `two_boxes_exactly_the_gap_apart_are_clear_on_either_axis` (`clear_of`'s strict `>` on x and on y; two parts exactly the gap apart are clear) and `a_blocker_far_above_the_base_does_not_hide_the_one_legal_cell_far_below` (the rows' arithmetic in `first_legal_cell`). **Not run, by decision:** `CostModel::legal`, `make_room`, `delta`, `anneal`, `descend`, `gain_of`, `repair`, `routing_order`, `silk_entries`, `converge_pool`, `start_cost` (they are pinned by the run goldens, the random-state tests and the test that the caches equal a full evaluation, not by mutants).

- **The full gate on `ec10fd20`** (the row with `fork/main` at `74dccf2a` merged in; 2026-11-22, under `gate.lock`, `native.lock` and `instr.lock`): fmt clean; workspace tests 5311 passed, 0 failed, 177 ignored (the `konnect` crate's `asset_references` and `schema_parameter_usage` among them); sandboxed fastroute 38 passed; doc tests 9 passed, 2 ignored; `simulation_live` 94 of 95, the one failure the known IBIS demo flake (`every_simulation_demo_flattens_or_names_its_export_failure`, "Cannot find U1.cache"), which passed on its isolated retry, as it did in Row CL's gate; `clippy -D warnings`, `deny` and the release build clean; `fix-doc-counts` changed nothing; protected files unchanged; the installed binary's SHA-256 starts `3de188bb775ab4ce`. Record: private `gate/final/results.json`.

**Merged, 2026-11-22.** The owner resumed the row ("תמשיך בסבב התיקונים", then "כן, להמשיך"), approved the exact title and body ("כן, לדחוף ולפתוח") and the merge ("תמזג את ה-PR"). [PR #98](https://github.com/yuvaltal912-sys/Konnect/pull/98) merged as main `dc3c3e60`, a noreply merge whose tree equals the PR head `e90bf484`. The PR's body is the summary: what C is, the two boards measured, what failed first (the port check) and the amendments, the five limits, the verification and what was not verified.

**Not done.** A bench run on B6 (routable since #97) and B4 (no C placement); Row CL's limits 1, 3 and 4. Each on the owner's word.

## Row CL, four limits found by Row CP (handed to Codex, 2026-11-21)

**State, 2026-11-22.** Limit 2 merged as [#97](https://github.com/yuvaltal912-sys/Konnect/pull/97); limits 1, 3 and 4 not started. The owner asked on 2026-11-21 for a handoff so that Codex fixes the first three limits below ("תכין handoff לקודקס שיתקן את שלוש המגבלות שהתגלו היום"), and added the fourth the same day ("כן, תוסיף אותה ל-handoff של קודקס"). Row CP is merged as [#98](https://github.com/yuvaltal912-sys/Konnect/pull/98), so limits 1, 3 and 4 start from `fork/main`, in your own worktree and branch, each on the owner's word.

**Where the evidence is.** Private records, outside git, on this Mac: `CP=/Users/yuvaltal/ClaudeCode/Konnect/target/creation-research/convergent-placement` (`PROGRESS.md` is the dated log; `reports/`; `logs/`). The boards are the owner's private designs: copies only, and no net name, reference or coordinate of them in code, tests, fixtures or documents. Lane scripts for every cargo or KiCad command: `CP/light.sh` and `CP/heavy.sh` (set `CP_W=<your worktree>`; read `CP/AGENT_RULES.md` rule 4 and `target/creation-research/supervision/lanes/README.md`). Prebuilt release test binaries: `CP/bin/konnect_core-521c1ac9` (Row CP's code); drivers `CP/run_bench.sh <bench folder> <binary>` (bench with routing) and `CP/run_check.sh <bench folder> <binary> [silk]` (family C, no routing). The knowledge graph (graphify) was refreshed at `95ec0d05`: `/Users/yuvaltal/.codex/worktrees/konnect-main-graph/Konnect/graphify-out/`.

**Limit 1. A board too dense for family C's gap rule: C generates nothing.**
- Seen on board B4 (118 parts, 12 held). Every C run is `generate_failed` with `no room for 1 part(s): <one reference>` (`place_search.rs`, `describe`, `GenerateFailure::NoRoom`), from each of the 4 starts taken; F and S generate all 160 candidates there. The rule for starts then stops by its rule with no result.
- Cause as far as known: C legalizes its start under `GapRule::Everywhere` (every two courtyards 0.3 mm apart, 0.5 mm inside the outline; `place_generators.rs`, `legalize_with`, `GapRule`; design decisions C2 and C3 in `docs/CONVERGENT_PLACEMENT_DESIGN.md`), which is stricter than what F and S use.
- **Not known:** which kind of part has no cell; whether the owner's own placement of that board is legal under that rule (if it is not, the rule is stricter than the board needs). Find both first.
- Reproduce: `CP/scratch/diag-B4` (a bench folder with a one-minute limit; move its `search.json`, `candidates/`, `table.md`, `isolation/` and `input-check/` away, then `CP/heavy.sh <tag> CP/run_bench.sh CP/scratch/diag-B4 CP/bin/konnect_core-521c1ac9`), or `CP/run_check.sh` on a fresh copy of `CP/bench-B4`. Log: `PROGRESS.md`, entry "diag-b4-01".
- Done when: on a copy of B4 family C ends at least one run with a legal placement, or refuses the board up front with a reason that names the cause; a public or synthetic fixture reproduces the case in a test with a negative control; F's and S's 80 board goldens and C's 17 run goldens are unchanged, or a changed C golden is explained.
- This changes a design decision (C3). **The owner approves the design before it is built** (his standing rule: an algorithm's design is written and approved first).

**Limit 2. A footprint with a custom-shaped pad: the export to the router refuses the board, so nothing is routed.**
- **Done by Codex, merged as [#97](https://github.com/yuvaltal912-sys/Konnect/pull/97) on 2026-11-21** (a convex custom pad's copper is exported; its record follows this section). On a copy of B6 the bench now routes the owner's placement: KiCad's unconnected count fell from 136 to 2, and the two open items are on nets with no custom pad. The text below is the limit as it was handed over.
- Seen on board B6 (127 parts). The owner's own placement and every candidate are `export_refused`: `unsupported routing profile: pad <label> has shape 'custom'` (`crates/konnect-core/src/specctra.rs`, about line 3112). The bench then stops; the unrouted DRC check and family C's placement work on that board.
- This is the Specctra export, not Row CP: it is on `fork/main` too, and it can be fixed on a branch from `fork/main` with no dependence on Row CP.
- Reproduce: `CP/scratch/diag-B6` (seven-minute limit; same clean-up and command as above). Log: `PROGRESS.md`, entries "diag-b6-01" and "diag-b6-02". Row E1 ("board quirks KiCad accepts") is the nearest earlier work.
- Done when: a board whose footprint has a custom-shaped pad exports and routes through `route_with_fastroute` and the bench, with the pad's copper correct in the DSN (a public fixture, a test, a negative control); on a copy of B6 the bench routes the owner's placement.

**Limit 3. C's cost does not see whether a placement routes completely.**
- Seen on the copy of the owner's board, in both bench runs: the C candidate with the shortest routed signal failed gate G1 with 25 and 26 unconnected items (a different start each time) and ranks 11 of 15; with the terms on it is also the candidate of the lowest cost. The other three routed C candidates of each run pass every gate, and C's best still ranks 1.
- Records: `CP/bench-dash/dash/` (candidate `c0168`) and `CP/bench-dash-off/dash/` (candidate `c0167`): `search.json`, `table.md`, each candidate's `route.search/`. The filled answers are in `docs/CONVERGENT_PLACEMENT_DESIGN.md`, "The answers on the owner's board".
- **Not known:** why those two placements do not route completely (a pour cut into islands, a blocked pin escape, local congestion). All of a call's runs ended incomplete, so it is the placement and not one unlucky run: find the cause before choosing a fix.
- The cost today: airwire length, 4 times the crossings, the decoupling and connector requirements, and two weak terms (`place_converge.rs`, `CostModel::terms`). The rule for effort also reads one run per level.
- Done when: the cause is recorded with its evidence; a design for it is approved by the owner (a term in the cost, or a rule on which C candidates are routed, or both); then on the same board copy no C candidate that the search prefers fails G1 where a C candidate of a higher cost passes it.

**Limit 4. Where the tools find no decoupling pair, family C does not keep the capacitors by their ICs.**
- Seen on board B5 (94 parts) in the bench with routing: C's best ranks 1 and beats the owner's placement and the AI's, yet its copper quality is 58.9 against 66.9 and 66.5. The audit (`audit_layout_quality`, run by pipe on scratch copies of the six winning routed boards) puts the whole gap in one check, 4.1 `decoupling_loop` (category `decoupling_and_current`): 0.27 for the owner's board, 0.272 for the AI's, 0.0 for all four C boards. With 4 supply pins and a 10 mm threshold: the owner's median loop is 14.2 mm (worst 25.2, no pin without a capacitor); C's best 37.0 mm (worst 42.0, 1 pin without a capacitor); C's fourth 44.7 mm (2 pins without). Return path and routing do not explain the gap (C's best: 60.3 against 60.6, and 100 against 100).
- Cause on the placement side: `score_placement` reports no decoupling pair on that board (`decoupling_associations` empty; `crates/konnect-core/src/tools/placement/evaluate.rs`), so C's requirement term has nothing to hold (`place_converge.rs`, the pair requirement in `CostModel::terms`; design decision C2), and the bench's `decoupling` column reads 0 for every candidate. An AI session on the same board reported that the tool listed 9 capacitors as unproven; why is **not known**.
- Records: `CP/bench-B5/B5/` (`search.json`, `table.md`), the six audits in `CP/scratch/cq/audit-r<rank>.json` (rank 5 is the owner's, 8 the AI's, 1 to 4 are C's), and `PROGRESS.md`, the entry on copper quality.
- Done when: the reason the pairs are not found on that board is recorded; then either the detection finds them, or family C gets the pairs another way (the audit's own supply-pin and capacitor reading is one candidate), with a public fixture, a test and a negative control; and on a copy of B5 C's best keeps every supply pin's capacitor within the audit's reach, with its check 4.1 no worse than the owner's.
- A change to how C reads its requirements is a design change: the owner approves it before it is built.

**Order.** Limit 2 first (done, #97): it is self-contained, it starts from `fork/main`, and it cannot collide with Row CP. Limits 1, 3 and 4 change family C, so they start after Row CP is merged, or from its branch head with the owner's word; each begins with the diagnosis and a short design for the owner. Limit 4's first half, why the pairs are not found, is in the placement tools and can be diagnosed from `fork/main`. One PR per limit, each by this file's usual way (baseline gate, tests with negative controls, an independent review, the gate, a noreply commit, the exact PR title and body to the owner).

### Row CL, Limit 2 — custom pads in the Specctra export (Codex's record, merged as #97)

**Implemented and published, 2026-11-21.** Commit `6e9a58f0` implements the owner’s export fix on `codex/export-custom-pad`, based on `fork/main` at `0a3fc504`. Row CP's local handoff at `af59d99a` records the original refusal. Local claim `6d3c90cb` records the owner’s requested worktree at `~/.codex/worktrees/konnect-cp-limits/Konnect`. Its scratch boards and verification records stay there. Limits 1 and 3 belong to Row CP's placement work.

The Rust exporter merges the anchor and supported copper primitives on KiCad's nanometre grid, checks for one convex region without holes, and projects it to the 0.1 µm DSN grid. Each pin keeps its own offset, orientation and copper layers. The complete polygon participates in padstack identity. The public fixture derives entirely from the two-resistor test board and was written with KiCad 10.0.8. The tool directory and API migration entry describe the supported shapes.

**Verified on macOS.** Local evidence is under `target/row-cl/` in this worktree.

- Base verification at `0a3fc504` used the creation gate's shared locks and protected configuration. Workspace tests passed (5,183 plus 38 sandboxed), as did nine doc tests, fmt, Clippy, licenses, doc counts and the release build. The first live simulation run passed 94 of 95. The IBIS demo reported `Cannot find U1.cache` and passed on an isolated retry. The installed binary and protected settings matched their original hashes. See `gate/baseline-verified/results.json`.
- The five original regression tests all failed on the pre-change exporter. The feature build passed all 17 focused tests with `cargo test -p konnect-core --locked --lib specctra::custom_pad_tests`. The opt-in native fixture test ran separately and passed through `route_with_fastroute`, scratch KiCad refill/DRC and strict SES import with real track geometry.
- The placement bench routed the public fixture on all three reference calls and one generated placement with zero unconnected items and all three gates passing. The reference still has two library-footprint findings and one silkscreen-over-copper finding. Both custom copper polygons agreed with KiCad's merged outlines to within 4.94 µm. The 6 µm comparison allowance covers 5 µm curve chording, DSN rounding and the independent native contour. See `logs/public-verified-result.json` and `logs/public-verified-copper.json`.
- The B6 diagnostic with effort M and a seven-minute bench limit returned a judged SES for the owner's byte-identical input placement (35 runs judged in the first reference call). KiCad's unconnected count fell from 136 to 2. The two open items involve four endpoints on nets with no custom pads. G2 and G3 passed. The time limit allowed one of the three reference calls. All 32 custom pad boundaries met the same native comparison threshold. The original diagnostic had refused all three owner calls before routing. See `logs/B6-result.json`, `logs/B6-copper.json` and `logs/B6-open-counts.json`.
- The independent review prompted fixes and regression controls for nanometre gaps, swept circular line caps, odd-sized anchors, plated drill containment, both surface offset forms, layer-specific drilling, repeated primitive lists and native bridge policy. The final source review accepted those changes. The local AST graph includes the new exporter and its tests.

- A second B6 bench with effort H and a 15-minute limit judged 113 trials. It selected a byte-identical SES. The saved, refilled board has 547 track segments and 33 vias. A check by footprint UUID found all 127 positions, rotations and sides equal to the input. Its two opens, the G1 result, and the passing G2 and G3 checks matched effort M. All four open endpoints mapped to the judged board, with zero belonging to custom pads and zero affected nets containing a custom pad. See `logs/B6-high-result.json` and `logs/B6-high-open-counts.json`.
- The focused coverage run (`cargo llvm-cov --locked -p konnect-core --lib --json --summary-only -- specctra::custom_pad_tests`) covered 186 of 193 executable lines in `specctra_custom_pad.rs`.

- Targeted `cargo mutants` caught all five selected polygon and drill mutations. Separately, each of 16 source mutations failed its named regression test. The controls cover lost copper, Y reflection, premature rounding, islands, holes, concavity, odd anchor sizes, stroke width, drill containment, surface offsets, layer rules and native-bridge handling. The results are in `mutants/mutants.out/outcomes.json` and `controls/results.json`.
- The staged macOS gate passed 5,200 workspace tests plus 38 sandboxed router tests, nine doc tests and all 95 live simulation tests. Formatting, workspace Clippy with warnings denied, dependency licenses, doc counts, the release build and the protected-file checks passed. The installed binary SHA-256 begins with `3de188bb775ab4ce`. The standard commands and split for the nested sandbox are recorded in `gate/candidate/results.json`. The post-commit gate records the exact commit in `gate/final/results.json` and the release server's public-fixture routing response in `gate/final/served/route.json`.

**Reproduce the public route.** Set `KONNECT_FASTROUTE` to the router binary and use scratch `KICAD_CONFIG_HOME` and `KONNECT_STATE_DIR`. Run `cargo test --release -p konnect-core --locked --lib specctra::custom_pad_tests::fastroute_connects_the_custom_pad_fixture_after_kicad_refill -- --ignored --exact`. To repeat the placement bench, prepare a fresh folder with the saved input and routing rules as described in [the bench README](../xtask/placement-bench/README.md).

**Post-commit verification.** One IPC handshake test failed in the first workspace pass on `6e9a58f0`. It passed on its isolated retry and in a full repeat of that phase (5,200 passed). The 38 separately sandboxed router tests and all 95 live simulation tests passed. The release server returned `complete` for the public fixture with zero unconnected items. See `gate/final/workspace-repeat.json` and `gate/final/served/served.json`.

**Publication.** The owner approved the exact title and body for PR #97. GitHub's readback matched the approved text byte for byte and confirmed the published implementation head `6e9a58f0`. When the PR opened, `main` had advanced to `dddcd769` with the Row RJ fix in #96. Both branches had edited the handoff header and the same progress paragraph. Row CL now has its own status entry. [PR #97](https://github.com/yuvaltal912-sys/Konnect/pull/97)

**Next step.** The owner reviews PR #97 for integration. The two open B6 connections are recorded for routing work. This export change was verified on macOS.

## Row CT, family C as a placement tool (merged as #104, 2026-11-22)

**State, 2026-11-22.** Merged as [#104](https://github.com/yuvaltal912-sys/Konnect/pull/104) on the owner's word (main `6abf6333`, a noreply merge whose tree equals the PR head `87c3d474`). Built on branch `claude/converge-place-tool` from `fork/main` `1506938e`; the owner approved the exact PR title and body ("כן, לדחוף ולפתוח"), and GitHub's readback matched them, then the merge ("כן, למזג"). The owner gave the row to a Claude session the same day ("יש כבר קוד כתוב להנחת רכיבים מתכנסת, המשימה שלך היא לבנות כלי"), after it had been proposed for Codex. He chose both open decisions as recommended: a planner that writes nothing, and routing the best three candidates before a winner is named. Design: [`CONVERGENT_PLACEMENT_TOOL_DESIGN.md`](CONVERGENT_PLACEMENT_TOOL_DESIGN.md) (decisions T1 to T11).

**What it is.** A new MCP tool, `plan_convergent_placement`, in the `placement` toolset (now 7 tools). For a board open in KiCad (and the only board open) it reads the live text and KiCad's effective routing rules, makes a scratch project in `<state dir>/placement-plans/<ms>-<sha12>/`, runs the bench's search there (F and S starts, family C, its measured parameters and stop rules, the owner's KiCad locks held), routes family C's first three candidates with fastroute (effort `m` by default) and names the rank-1 routed one by the bench's judge. The response's `apply_with` holds the winner as exact `set_component_placements` arguments; nothing is applied. `planned_from` names the board state the plan is for; notes cover no decoupling pair found, an unreadable schematic, and old routing on the board.

**What changed.** `tools/placement_converge.rs` and its tests (new); `place_search.rs`: `Plan::routed` (`Routed::Every` for the bench, `ConvergeOnly` for the tool), `Plan::schematic`, `Outcome::model` and `schematic_reason`, `rebuild`; `place_schematic.rs`: `read_root`; the registry count and the doc counts, the `kicad-pcb` skill, `tool-directory.md`, `API_MIGRATIONS.md`, and the lock sentence of `set_footprint_lock`, the skill and `creation_guidance.rs` (the new planner holds locked parts too). The bench's behaviour and its `search.json` are unchanged (`every_routes_the_other_families_too_and_its_file_is_unchanged`).

**Verified (macOS).** Records in `target/creation-research/converge-tool/` (`PROGRESS.md`, `gate/`, `controls/`, `logs/`).
- Baseline gate on `1506938e`: green (5,575 tests + 63 sandboxed, 9 doc tests, 95 live simulation tests, clippy, licenses, doc counts, release build, installed hash `3de188bb775ab4ce`).
- New tests: 18 in `tools::placement_converge` (one ignored, end to end) and 3 in `place_search::tests`, all passing; the whole `place_` and `tools::placement` suites pass (490).
- Negative controls: 10 of 10 caught in round 1 (`controls/results.json`); in round 2, after the review's fixes, 6 of 7 (`controls/results2.json`): the false-P1 guard slipped because the fixture's extra C records were duplicates, which carry their own reason, so a test that builds the case was added and catches it (`controls/results3.json`). Each restored tree passed.
- End to end, `the_whole_plan_on_the_public_placement_fixture` (ignored; real fastroute, kicad-cli and judge, effort m, rules from the fixture project's Default class): `planned`, 3 routing calls, all complete with every gate passing, 5 of 8 parts moved, the user's folder byte-identical, 729 s. Run again on the final code: the same winner and numbers (730 s), and the notes name the fixture's 3 routing items before the apply.
- Independent review (Opus 5.5, high, read-only): 1 must-fix (no one-board guard before the net-class read), 5 should-fix (old routing not mentioned, a false P1 reason, `routing_calls` overcounted, a cancelled call kept routing, a second read of the schematic) and 3 nits; all fixed, each with a test and a negative control except the IPC guard. Its second look confirmed all nine fixes and found one more nit (teardrop zones, which the scratch copy strips, were not counted as old routing), fixed with a test.
- `cargo mutants` (27.1.0), one narrow batch on the tool's guard functions (`verdict_of`, `was_called`, `was_sent`, `gate_failures`, `stable_read`, `read_arguments`, `Planned::of`, `tool_plan`, `run_folder`) with `tools::placement_converge`'s tests: 24 mutants, 19 caught, 5 unviable, 0 missed, 7 minutes (`mutants/run/mutants.out/`). `cargo llvm-cov` was skipped: the 18 source-mutation controls and this batch already show each guard's test failing when the guard breaks.
- The konnect crate's guards: `asset_references` (the tool's five response words added to `NOT_TOOLS`), `schema_parameter_usage`, `creation_guidance` and `doc_tool_counts` pass; `cargo xtask fix-doc-counts` updated the counts (270 tools, 277 with the meta-tools).

- Full gate on the implementation commit `5c921a43`: green (5,595 tests + 63 sandboxed, 9 doc tests, 95 live simulation tests, clippy, licenses, doc counts unchanged, release build, protected files unchanged, installed hash `3de188bb775ab4ce`; `gate/final/results.json`).

**Not verified.** The live IPC read (no test starts KiCad; the one-board guard and the two-read check run only against a real KiCad). No run on any of the owner's boards. Windows and Linux.

**Limits.** Inherited from family C (Row CL): limit 1 (a board too dense for C's gap rule gives `no_candidate`), limit 4 (no decoupling pair found: capacitors not kept by their ICs; the response notes it), limit 3 (why the tool routes; three calls may still all fail a gate, then `planned_with_gate_failures`). `no_candidate` is tested only for a search with no start.

**Next step.** Each on the owner's word: a first run on a copy of one of his boards open in KiCad (his step: open it), at effort m, served by the fork's release binary; and a new 1 Kopper package build, without which the installed Konnect (SHA-256 `3de188bb775ab4ce`, unchanged) does not have the tool. No row starts without his word.

## Row GS, the smart-router switches (merged as #108, 2026-11-22)

**State, 2026-11-22.** Merged on the owner's word as [#108](https://github.com/yuvaltal912-sys/Konnect/pull/108): main `6b31bf90`, a noreply merge whose tree equals the PR head `3fc339a9`. The branch was `claude/router-group-search`, from `fork/main` `5a8273ea` with main `b78371d2` (#106) merged in: the build, the x19 pin and `engine.switches`, the review round, the by-pipe record, the measurement and the gate record. Nothing was installed: the installed Konnect (SHA-256 starting `3de188bb775ab4ce`) and the fastroute at the default place are unchanged, and that fastroute is x18. The x19 binary and the engine source (a git bundle of the whole fastroute repository, branch `gr`) are backed up in the owner's iCloud `Claude/1-Kopper/` folder. Built to the skeleton `target/creation-research/smart-router-research/skeleton/konnect_uptake.md` (decisions G1 to G6, recorded here as D25 in [`ROUTE_SEARCH_DESIGN.md`](ROUTE_SEARCH_DESIGN.md)). The engine build it pins, fastroute x19 (engine commit `b3ea91773361c2963ec440eaef405f3a8d832372`, SHA-256 `7616807b713cebb9c425b6ecee40566bd87dbf4cb5094a78c6c4e36d5fd3bb35`, in `packaging/1-kopper/fastroute.json`), is final; the final gate and the end-to-end checks below have not run. x19 is x18 plus the engine's row FT (the tuner keeps off its own net, and pair nets end clean; built and tested in the engine, not measured through Konnect before this row) plus S1, F1 and the group search, all three off unless Konnect sends their switches. The engine commit is on the branch `gr` of the local fastroute clone (`~/ClaudeCode/fastroute-s1f1`) and is not published anywhere.

**What it does.** `route_specctra_dsn` (fastroute) sends `SEARCH_SWITCHES` (`--x.oct_bound`, `--x.opt_follow_rips=2`, `--x.group_search`) on every Full, Fallback and Polish pass of every run, after `CANDIDATE_SWITCHES`, and never on a Tune pass (G1). The probe sends them before the sentinel, so a build without them (x18) is refused before any run, with `--x.oct_bound` named (G2). The manifest's pour nets, the judge's `pour_nets`, go to `<search folder>/uncounted.txt` and to every routing pass as `--x.gs_uncounted=<file>` (G3). No budget or hang-cap change (G4). The winner's result has `search.winner.group_search` (`ended_by`, `passes`, `kept_moves`, `largest_group`, `start` and `end` with `length_mm`, `vias`, `cost_mm`) when its kept routing pass reported one, and each judged run's `run.json` keeps the same summary (G5). The candidate's name is `C** + BR1 + W8P + GS`.

**What changed.** `fastroute.rs`: `SEARCH_SWITCHES`, `CANDIDATE_NAME`, the probe's step 2, `CallFiles::uncounted`, `pass_arguments`, `uncounted_file_text`, `PassOutcome::group_search` and `Held::group_search` (the kept routing pass's; a kept Tune pass leaves it), and the Tune pass's files never carry the uncounted file. `tools/route_fastroute.rs`: `uncounted.txt`, the runner's `uncounted`, `group_search_summary`, the winner's JSON and `run.json`. `tools/integration.rs`: the `fastroute_path` schema text names the switches. The result's `engine.switches` lists what a routing pass ran: `CANDIDATE_SWITCHES`, then `SEARCH_SWITCHES`. `packaging/1-kopper/fastroute.json` pins x19, and `1-KOPPER.md` names it. The probe also sends `--x.gs_uncounted` (an empty file), with the sentinel last; it proves that a build knows the switches, not which build it is (the SHA-256 does). The uncounted file leaves out a pour net that another, counted net of the manifest repeats in another letter case.

**Other callers.** `plan_convergent_placement` (`tools/placement_converge.rs`) calls `fastroute::locate` and routes its candidates through `route_with_fastroute` (`place_route::route_candidate`), as does the placement bench (`place_search`): both now refuse x18, and their candidates run the group search. Their measured parameters and timings (Rows PS, CP and CT) were measured on x18 and are not measured again here. `fastroute::route_held` has no caller but the production runner, and `pass_arguments` none but `run_pass`.

**The engine's row FT.** x19 holds FT, so the Tune pass and the pair passes change from the engine itself, whatever switches Konnect sends. Checked by pipe on x19 (2026-11-22, release `konnect` of `e1c3fdf5`, public fixture `specctra_pours_routing`; records in `target/creation-research/smart-router-research/gs-build/pipe/`). Row LT's check (the prepared fixture F, group `g` = S1, S2, tolerance 0.5, effort M), against M4's x18 record: status `complete` on both; launched / judged 151 / 148 against 130 / 123 (one call each, so this is load as much as the engine); the winner kept the Tune pass, one round, 0 nets dropped for self contact, 0 untuned nets, the free candidate chosen and the pour-safe one not judged, on both; G1 to G4 with no line on both; KiCad lengths S1 51.808 and S2 51.706 mm (spread 0.101) against 52.278 and 52.173 (0.104). The Tune pass's own log shows no search switch and no uncounted net (`oct_bound: false`, `opt_follow_rips: 0`, `group_search: false`, `gs_uncounted: []`), while Full and Fallback show all four. Row DP's check (S1/S2 a pair, gap 0.3, effort fast), against its x18 record: minimum edge gap 0.3 mm on both; S1 within 0.5 mm of S2 for 40.17 mm against 40.2; lengths S1 55.24 and S2 52.13 mm against 53.4 and 53.35; every gate passes, status `complete`. `API_MIGRATIONS.md`: the entry. Tests in `fastroute_tests.rs` (the fake fastroute learns `lacks_search`, `lacks`, `echo_uncounted`, `group_search.<pass>` and copies the uncounted file it reads), `tools/route_fastroute_tests.rs` and `route_judge_tests.rs`.

**Measured.** One `route_specctra_dsn` call at effort `m` for each board and side, by pipe, with scratch settings and state, each call in its own comparable routing hold. **Before:** the fork's release build of `e90bf484` (Row CP's head, before Row LT) with fastroute x18, not main's build; for a board with no length intent its routing command line is the same as main's (the commits between touch the Tune pass and the judge's G4), which is inferred from the log, not measured. **After:** the release build of `e1c3fdf5` with x19. Runs are paired by pour shape and seed, seeds 1 to 8 (16 runs a board), over the pairs that pass every gate on both sides. "Key" is the judge's rank order taken from the measures with one common length step for both sides (acute corners, signal length in 2 % steps, power length, right angles, vias off pour, copper quality), seconds left out. Records: `target/creation-research/smart-router-research/final/` (not in git).

| Board | Pairs | Signal length, median paired change | Shorter / longer | Vias, median | Key better / equal / worse | Runs passing every gate, of the 16 | Runs judged in the call |
|---|---|---|---|---|---|---|---|
| pic | 2 | -0.17 % | 1 / 1 | 4 -> 2.5 | 2 / 0 / 0 | 2 -> 2 | 228 -> 185 |
| ecc | 16 | -0.28 % | 9 / 2 | 0 -> 0 | 6 / 6 / 4 | 16 -> 16 | 1443 -> 1400 |
| pours | 8 | -3.47 % | 8 / 0 | 6 -> 6 | 8 / 0 / 0 | 9 -> 11 | 1418 -> 1428 |

No paired run on any board has more vias than its pair. The call's own winner:

| Board | Before | After |
|---|---|---|
| pic | 1387.9 mm, 1 via | 1384.3 mm, 1 via (same seed and shape) |
| ecc | 159.1 mm, 0 vias | 159.3 mm, 0 vias |
| pours | 206.9 mm, 6 vias, pours kept | 198.5 mm, 8 vias, pours omitted |

On `ecc` the four worse keys are one point of copper quality (two runs) and one 2 % length step on a 160 mm board (two runs, +0.93 %); its winner is 0.2 mm longer. **The stop:** every judged run on x19 has a group search summary, and every one ended `converged` (pic 185, ecc 1400, pours 1428). **Repeat:** one call repeated gave a byte-identical winner session and identical measures and gates in every judged run (12 of 12). **The owner's boards** (copies of two; counts only): 12 paired runs passed every gate on both sides; 12 of 12 were shorter, none had more vias, and the key was better in 12 of 12; every judged run converged (58 of 58). A run takes longer with the search, so a call at `m` judged fewer runs: 48 -> 12 on the larger board and 183 -> 46 on the smaller, and on the larger one 4 of the 16 wanted runs were not reached in the budget.

**Verified (macOS).** Records in `target/creation-research/smart-router-research/gs-build/` (`PROGRESS.md`, `gate/`, `controls/`, `mutants/`, `logs/`).
- Baseline gate on `5a8273ea` (the untouched branch, `gate/baseline/results.json`): green. Workspace tests 5,595 passed, 0 failed, 181 ignored; fastroute's sandboxed tests 63 passed; doc tests 9; `simulation_live` 95 of 95; fmt, clippy, licenses, `fix-doc-counts` changing nothing, the release build; protected files unchanged, installed hash `3de188bb775ab4ce`.
- `cargo test -p konnect --locked --test asset_references --test schema_parameter_usage`: 12 and 5 passed, before and after the change (no `NOT_TOOLS` entry was needed).
- `cargo test -p konnect-core --lib -- fastroute route_judge tools::route_fastroute route_search place_route`: 392 passed, 0 failed, 6 ignored. 16 new tests (7 command-line and file tests and 4 sandboxed ones in `fastroute_tests.rs`, 4 in `tools/route_fastroute_tests.rs`, 1 in `route_judge_tests.rs`), and 5 existing ones extended (the documented order, each setting once, the Polish-against-Tune line, the runner's Tune test, the candidate's name).
- Negative controls (a guard broken in the source, its tests run, the file restored from git; `controls/controls.py`, `controls/results.json`): 23 of 23 caught. The probe's search switches; the routing line's search switches; the Tune line without them and without the uncounted file; `--x.gs_uncounted` when given; the four name filters of the uncounted file and its `None`; the call writing `uncounted.txt`, giving it to the runner and the runner to every run; a pass reading `group_search`, the kept Fallback's and Polish's, `Held` carrying it; the winner's JSON, `run.json`, judged runs only; the summary's sum, its largest group and a pass without `kept`; a Tune choice keeping the routing pass's object.
- `cargo fmt --all -- --check` clean; `cargo clippy -p konnect-core --locked --all-targets -- -D warnings` clean.
- `cargo mutants` (27.1.0), one narrow batch, `--in-diff` on the changed non-test files (`fastroute.rs`, `tools/route_fastroute.rs`, `tools/integration.rs`) with the `fastroute`, `tools::route_fastroute`, `route_judge` and `tools::integration` tests: 33 mutants, 23 caught, 10 unviable, 0 missed, 0 timeouts, 18 minutes (`mutants/run/mutants.out/`). With no miss there was no rerun. `cargo llvm-cov` was skipped: the 23 controls and the batch already show each guard's test failing when the guard breaks.

- Review round (an independent review of `1ed17096`; fixes in `e1c3fdf5`): the focused set 413 passed, 0 failed, 7 ignored (with `tools::placement_converge`); `asset_references` 12 and `schema_parameter_usage` 5; `cargo test -p xtask` 128; fmt clean; `clippy -p konnect-core -p xtask --all-targets -D warnings` clean; negative controls C24 to C31, 8 of 8 caught (`engine.switches`, the sentinel moved before the search switches as the reviewer did, the probe's uncounted switch and its file, the letter-case rule); one `cargo mutants --in-diff` batch over the round's non-test lines: 15 mutants, 14 caught, 1 unviable, 0 missed (`mutants/run2/`).
- Release `konnect` of `e1c3fdf5`: `target/release/konnect`, SHA-256 `ce28f42da9cf4bd72037fe426b63d946a608eddf22457c14494cd02723efe76e` (a copy in `gs-build/pipe/`).
- By pipe through that binary with x19 (`fastroute_path`), each call in its own routing hold (`gate.lock`, `native.lock`, `instr.lock`; `pipe/hold.py`, `pipe/holds/`), scratch `HOME`, `TMPDIR`, `KONNECT_STATE_DIR` and `KICAD_CONFIG_HOME` with the KiCad API server off, the protected files and both fastroute binaries unchanged (`pipe/<case>/record.json`):
  - x18 at the default place, named as `fastroute_path`: refused at probe step 2 with `--x.oct_bound` named, "Nothing was routed", no search folder left.
  - The public pours fixture at `fast`: 2 launched, 2 judged, status `complete`, every gate passes; `search.winner.group_search` present (`converged`, 6 passes, 4 moves kept, largest group 1, length 216.49 to 209.58 mm, 6 vias); `engine.switches` the twelve; `uncounted.txt` holds `GND`; the Full and Fallback logs show `oct_bound: true`, `opt_follow_rips: 2`, `group_search: true`, `gs_uncounted: ["GND"]`.
  - Row LT's and Row DP's checks: as under "The engine's row FT" above.
- `cargo xtask kopper-package --prefix gs-build/package/1-kopper --konnect <the release> --fastroute <x19>`: accepted the x19 pin and wrote the package (`bin/konnect`, `bin/fastroute`, `1-kopper.json` with both SHA-256s); nothing installed, no MCP client changed, `~/.local/share` untouched.
- Final gate on `98d389da` (the branch head before this record, 2026-11-22, `gate/final/results.json`): every step green. fmt; workspace tests 5,610 passed, 0 failed, 181 ignored; fastroute's sandboxed tests 68 passed, 1 ignored; doc tests 9; `simulation_live` 95 of 95; clippy `-D warnings`, licenses, `fix-doc-counts` changing nothing; the release build (SHA-256 `3a3e896ec638cac61487b630a5dc463c1d2523afaa35e18d830f529fac2d156c`); protected files unchanged, the installed hash `3de188bb775ab4ce`. No foreign cargo at its start or end. The record commit after it changes only this file.

**Not verified, and limits.** Windows and Linux.
- **Fewer runs a call.** The budget of an effort level is unchanged (G4), so a call judges fewer runs; on a large board at `m` it may reach only a handful of seeds. Whether the budgets should grow is the owner's decision and is not part of this row.
- **Skew inside a differential pair.** In Row DP's check on the public pours fixture (pair S1/S2, effort `fast`, seed 1, pours omitted) the pair stays coupled (minimum gap 0.3 mm, 40.17 mm coupled, as before), but the two nets' lengths move apart: 53.40 and 53.35 mm before, 55.24 and 52.13 mm after. The same call with x19 and no search switch (through the older Konnect build) gives 53.40 and 53.35 mm, and the group search kept no move in the winner (`kept_moves` 0), so the change comes from S1 and F1 in the optimizer, on the pair's uncoupled ends (records: `gs-build/pipe-old-konnect/dp/record.json` beside `gs-build/pipe/dp/record.json`). A pair that needs matched lengths needs a length group (Row LT); this row does not keep the skew a pair had.

**Next step.** None on this row. On the owner's word only: `cargo xtask kopper-package --fastroute <the x19 binary>` and the install (the probe refuses a bundled or a default-place x18, so a Konnect built from this main routes with fastroute only once x19 is where it looks); a decision on the effort budgets; and an engine follow-up for the skew inside a differential pair, if he wants one.

## Row ZR, zones in the router (researched and designed, merged as #102, 2026-11-22)

**State, 2026-11-23.** ZR0 merged as [#110](https://github.com/yuvaltal912-sys/Konnect/pull/110) on the owner's word (main `13c71913`, a noreply merge whose tree equals the PR head `92a33e1d`; branch `claude/zr0-pour-measures`, from `fork/main` `652f491a`). ZR1 in progress (Claude), on its own branch from fastroute `gr` `b3ea917`. The owner lifted "no build work yet" on 2026-11-22 ("תן לסשן השני להתחיל לכתוב את הקוד של הzones", relayed verbatim by the supervisor session): read as his yes to decision 1 as recommended (ZR0, then ZR1), not as an answer to decisions 3, 4 and 5, which each come back to him when a stage needs one.

**Stage ZR0, the judge's pour measures (merged as [#110](https://github.com/yuvaltal912-sys/Konnect/pull/110), 2026-11-23).** The owner approved the exact title and body ("תפרסם") and the merge ("למזג"). `route_specctra_dsn`'s judge records `Measures.pours` for every judged run when the manifest has pours: KiCad's unconnected items on the pour nets after the first refill (before stitching) and on the last board, the fill pieces of every pour zone layer of the last board (one `filled_polygon` is one piece), the lowest largest-piece share, the unfilled pour zone layers, and the `starved_thermal` violations; `false_connected` and `false_open` stay `null` until ZR1. A failed pour measure is recorded in `Measures.pours_error` and never fails the run. Nothing about the routing, the gates or the rank key changes (Z10). New `crates/konnect-core/src/zone_fill.rs` and its tests; `route_judge.rs` (`judge_one`); `docs/API_MIGRATIONS.md`; the design's "ZR0 as built". Verified: baseline gate on `652f491a` green (fmt; workspace tests 5,610 passed, 0 failed, 181 ignored; sandboxed fastroute 68; doc tests 9; `simulation_live` 95 of 95; clippy, licenses, `fix-doc-counts`, the release build; protected files unchanged); `cargo test -p konnect-core --lib -- zone_fill route_judge` 180 passed; the wider set (`fastroute route_search place_ tools::route_fastroute route_judge zone_fill`) 820 passed; `asset_references` 12 and `schema_parameter_usage` 5; clippy `-D warnings` and fmt clean; negative controls 14 of 14 caught (each guard broken in the source, its test failed, the file restored). An independent review (Opus 5.5, high) found no must-fix item; its 3 should-fix items (a failed measure must not fail the run, the judge test must prove the fill is read from the last board, six missing unit cases) and the doc notes are taken. **The x19 baseline, 2026-11-23** (effort M, one call per board in its own routing hold, the branch's release build; the table is the design's "The x19 baseline"): on dash and B5 every as-exported run left pour pads open (median 25 and 11) and none passed the gates, `pic` 2, `ecc` 0, `pours` 0 in 575 of 707; the five winners are pours omitted, the same seeds as Row GS, with judged counts within Row GS's spread, and no `pours_error` in 5,804 judged runs. **Found:** the judge's stitching rounds (Z9) never ran on these five boards; the stitching planner refused every time ("unsupported local zone_connect override" on dash, B5 and `pic`, the research's change 6; "saved project required for planning rules" on `pours`). Not fixed here (a feature PR reports a red baseline); it is a follow-up on the owner's word. **The probe** (planes replaced by KiCad's fill of the unrouted board): B5's as-exported runs went from 0 of 28 passing to 4 of 4, `pic` stayed at 2 open, and the engine judged about a fifth as many runs. **Final gate on `98bd4734`, 2026-11-23: green** (fmt; workspace tests 5,629 passed, 0 failed, 181 ignored, 19 more than the baseline; sandboxed fastroute 68; doc tests 9; `simulation_live` 95 of 95; clippy `-D warnings`; licenses; `fix-doc-counts` changed nothing; release build `34657817`; the installed Konnect `3de188bb775ab4ce` and the protected files unchanged). After it the branch gained this line only. Records: `.claude/worktrees/creation-netlist-compare-06a8b0/target/creation-research/zone-routing/zr0/` (scripts, controls, measurement) and `target/creation-research/zone-routing/zr0/` in the main checkout (gate, logs).

**Stage ZR0b, the winner's redundant ground vias are pruned (built, 2026-11-23).** The owner's rule, 2026-11-23: "צריך להסיר ויות שמטרתן הייתה לחבר פד לאדמה אחרי שהמילוי בולע את הניתוב בין הפד לויה". He accepted ("סבבה") a clean-up step after the session is applied and the zones are filled, with KiCad's own refill and DRC as the judge, and had the supervisor session build it ("תבנה") while the zones session went on to ZR1. Branch `claude/zr0b-pour-prune`, from ZR0's head `92a33e1d` (main `13c71913`, the merge of #110, has the same tree); Claude; local, not pushed. After the search, `route_specctra_dsn` (fastroute) tries every via of a ground pour net of the WINNER, first with the wires that end at it and then alone. A candidate session is read back by the importer, written to a scratch board and refilled, and is kept only when nothing gets worse: KiCad's unconnected count and every rule's count, every entry of the judge's rank key, every pour zone layer's fill, and the audit's unstitched layer changes (3.2), decoupling loops (4.1) and scored checks. The pruned session is judged in full before it replaces the winner; `search.pour_prune` and the winner's `run.json` say what happened. Only a winner that passed every gate with no stitching round is pruned (so Z9 is never touched), only ground pours, and no other ground wire; the placement tools' routing calls do not prune (an internal switch, no schema argument). New `crates/konnect-core/src/pour_prune.rs` and its two test files; `route_judge.rs` (`judge_one` split into `routed_scratch` and `audit` with no change of behaviour; `judge_light`, `judge_session`); `length_probe.rs` (the forced severities); `route_search.rs`; `tools/route_fastroute.rs`; `tools/integration.rs`; `place_route.rs`; `docs/API_MIGRATIONS.md`; the design's "ZR0b" (decisions P1 to P14, the measurement, the limits). Verified: the baseline is ZR0's final gate on `98bd4734` (the same code as this branch's base; run by the zones session, green); the design was reviewed twice and the diff once by agents that did not write them (the diff: no must-fix item, its 6 should-fix items taken); `cargo test -p konnect-core --lib -- pour_prune route_judge` 161 passed and 3 ignored, `-- fastroute` 225 passed, `-- place_route` 14; `asset_references` 12 and `schema_parameter_usage` 5; clippy `-D warnings` and fmt clean; negative controls 85 of 85 caught; one narrow `cargo mutants` batch on the guards (cargo-mutants 27.1.0: 88 mutants, 80 caught, 3 unviable, 5 missed, of which 4 were tolerance edges that now have a test and 1 is equivalent; the rerun of the misses caught all); two ignored tests run the prune with the real kicad-cli on the public pours fixture (one recorded ground via: nothing removed, it is the back pour's only link; two: one removed with its stub). **Measured** (the branch's release build, x19, effort M, one call a board in its own routing hold, every comparison inside one call): dash 18 to 12 ground vias and B5 21 to 10 (all vias 86 to 80 and 53 to 42), with 11 and 22 wires, in 41 KiCad checks and 85 and 77 s; the winner's key unchanged, every other net byte-equal in the session, and every pour zone layer's fill, the 3.2 count and every 4.1 loop unchanged (an independent reading of the judged boards); the delivered session is `pruned.ses` byte for byte, and KiCad's own refill and DRC on a fresh copy of the delivered board gives 0 unconnected and the judged counts; B5's repeat delivers the same bytes; `pic`, `ecc` and `pours` have no ground via in the winner and say `skipped`. The vias that stayed: a pad would be left unconnected (8 and 6), a decoupling loop would grow (4 and 3), a layer change would lose its ground via (1 and 2). **Not verified, and limits.** Windows and Linux. The other ground wires stay: removing them is a follow-up on the owner's word (the prototype: about 90 more KiCad checks on the larger board, and a third of those wires lie partly outside the fill). A via whose stub is split into wire blocks that meet at a bare point cannot go with its whole stub (none on the two measured boards). A call dropped mid-prune leaves the prune's scratch files inside its own search folder. The prune adds about 80 s to a call on these boards, outside the effort's budget. Records: `target/creation-research/smart-router-research/zr0b/` (the two design reviews, the prototype, the review of the diff, the controls, the mutants batch, the measurement) and `skeleton/pour_prune.md`. **Final gate on `05c532cb`, 2026-11-23: green** (fmt; workspace tests 5,694 passed, 0 failed, 183 ignored, 65 more than ZR0's gate; sandboxed fastroute 71; doc tests 9; `simulation_live` 95 of 95; clippy `-D warnings`; licenses; `fix-doc-counts` changed nothing; release build `b84d4fd0`; the installed Konnect `3de188bb775ab4ce` and the protected files unchanged; records `target/creation-research/smart-router-research/zr0b/gate/final/`). That build ran B5 once more and delivered a session byte-identical to the measured one. After the gate the branch gains this line only. **Next step:** the owner's yes to the exact PR title and body; then nothing on this stage.

**Engine coordination (supervisor, 2026-11-23).** ZR1 adds new files, a loader and the report and leaves routing unchanged, so it may be built on its own branch from `gr` beside the supervisor's row RS. ZR2 and ZR3 change `group_search.rs`, `connectivity.rs`, `maze.rs` and the pipeline: they start from the engine head that holds RS, which the supervisor names. Update, 2026-11-23 (supervisor): row RS was measured and closed without shipping, so there is no such head; ZR2 starts from ZR1's own engine head, `c13a28f`, the commit x20 pins.

**Stage ZR1, the engine's fill model and the pad comparison (merged as [#113](https://github.com/yuvaltal912-sys/Konnect/pull/113), 2026-11-23).** The owner approved the exact title and body ("Yes, publish") and the merge ("Yes, merge"): main `900e8fe1`, a noreply merge whose tree equals the PR head `1ebed231`. Branch `claude/zr1-zone-plan` (Konnect, from main `13c71913`, with main `d3cd82db` merged in) and fastroute branch `zr1-fill-model` (`c13a28f`, from `gr` `b3ea917`; local, the repository has no remote). Nothing about the routing, the gates or the rank key changes. The engine (build **x20**) reads `--x.zones=<file>`, draws a raster fill model of each modelled pour once on the routed board, and writes the report's `planes` object (every pin of a modelled net as connected or open, and the starved thermal pins). Konnect writes `zones.txt` from the saved board (each pour's fill settings, the pads' own thermal values, the keepouts; every pour the model cannot cover is sent as today, with its reason), sends it on every routing pass of the as-exported shape and never on the Tune pass, and compares the engine's pins with KiCad's first refill in the judge: `Measures.pours.false_connected` and `false_open`. The probe requires `--x.zones`, so x19 is refused; `packaging/1-kopper/fastroute.json` pins x20. The exact formats: `target/creation-research/zone-routing/zr1/CONTRACT.md` in the worktree `.claude/worktrees/zr1-konnect`; the design's "ZR1 as built" has what differs from the skeletons and the measurement. Verified: independent reviews of both sides (Opus 5.5, high; engine: 5 must-fix, all closed and re-checked with the reviewer's own probes; Konnect: 1 must-fix, the probe, and 5 should-fix, all closed), each fix with a test and a negative control (engine 26 + 11 + 1, raster 27, Konnect 59 and 3 more); fastroute `cargo test --workspace` 440 passed; Konnect's focused suites 811 passed. **The measurement** (five boards, effort M, x20): 0 false connections in 1,545 compared runs; false opens at most 2.2 % of the modelled pads; the model takes at most 0.84 s; sessions byte-identical with and without `--x.zones` on all five boards. Records: `.claude/worktrees/zr1-konnect/target/creation-research/zone-routing/zr1/` (contract, notes, controls, `matrix.sh`, `analyze.py`, `result.json`, `identical.sh`, the runs); the engine's notes in `~/ClaudeCode/fastroute-zr1/target/zr1-notes.md`; the x20 binary at `target/creation-research/zone-routing/zr1/engine-d14d29c7/fastroute` in the main checkout (SHA-256 `d14d29c7`). **Final gate on `750220e5`, 2026-11-23: green** (fmt; workspace tests 5,736 passed, 0 failed, 184 ignored, 42 more than ZR0b's gate on main's code; sandboxed fastroute 74; doc tests 9; `simulation_live` 95 of 95; clippy `-D warnings`; licenses; `fix-doc-counts` changed nothing; release build `808c260d`; the installed Konnect `3de188bb775ab4ce` and the protected files unchanged; record `target/creation-research/zone-routing/zr1/gate/final/` in the main checkout). After it the branch gained this line only. The x20 binary and the engine source are backed up in the owner's iCloud `Claude/1-Kopper/` (`x20/`, `fastroute-src-2026-11-23-x20.bundle`).

**Stage ZR2, the model as the routing truth and the closing stage (merged as [#115](https://github.com/yuvaltal912-sys/Konnect/pull/115), 2026-11-23).** The owner approved the exact title and body ("Yes, publish") and the merge ("Yes, merge"): main `ac8b5e01`, a noreply merge whose tree equals the PR head `3b35c950`. On the owner's "כן, להתחיל" (relayed by the supervisor session): ZR2 only, from ZR1's engine head `c13a28f`. fastroute branch `zr2-truth` (local, the repository has no remote), build **x21** from `20d7c96` (SHA-256 `46d79d56`; the head `be366d8` adds one doc comment; release builds are not byte-reproducible, so the pin is the measured binary itself); Konnect branch `claude/zr2-engine-truth` from main `c4c34369`: the pin, the candidate name `C** + BR1 + W8P + GS + ZR2`, and the result's `closings[].left_open` cut to `left_open_count`. Konnect sends nothing new: `--x.zones` turns ZR2 on. What the engine does, and the measurement: the design's "ZR2 as built". Verified: four independent reviews of the engine (Opus 5.5; the first's must-fix and the second's, a pour only vias reach was wiped, closed; the third and fourth found none, and their should-fix items are taken or recorded as limits), each fix with a test and a negative control; fastroute `cargo test --workspace` 476 passed; Konnect's focused suites (`zone_fill`, `fastroute`, xtask `kopper`) 384 passed, and the new `left_open` cut has a negative control. **The measurement** (with ZR1's Konnect release `808c260d`, so the cut is covered by its unit test only) (five boards, effort M, x21): KiCad's open before stitching is 0 on 1,380 of 1,382 as-exported runs and no stitching via is needed; 0 false connections and 0 false opens; G1 and G2 pass shares at or above ZR0's on every board; B5 and `pic` now win as exported. Records: `target/creation-research/zone-routing/zr2/` in the main checkout (`matrix.sh`, `hold.sh` with its hard timeout, `analyze.py`, `result-46d79d56.json`, `identical.sh`, the runs; the earlier builds' runs kept as `runs-2d559bc2` and `runs-1426caf6-partial`, not to be quoted); the engine's notes in `~/ClaudeCode/fastroute-zr2/target/zr2-notes.md`; the x21 binary at `target/creation-research/zone-routing/zr2/engine-46d79d56/fastroute`, backed up in the owner's iCloud `Claude/1-Kopper/` (`x21/`, `fastroute-src-2026-11-23-x21.bundle`). **Final gate on `1e1c0224`, 2026-11-23: every step green** (fmt; workspace tests 5,737 passed, 0 failed, 184 ignored, against ZR1's 5,736 on main's code; sandboxed fastroute 74; doc tests 9; `simulation_live` 95 of 95; clippy `-D warnings`; licenses; `fix-doc-counts` changed nothing; release build `9a9a4720`; the installed Konnect `3de188bb775ab4ce` unchanged; record `target/creation-research/zone-routing/zr2/gate/final/` in the main checkout). The protected-file check flagged the owner's `kicad.json` and `kicad_common.json`, rewritten at 18:05 during the test step. The gate is not the writer: its sandbox denies every write under `~/Library/Preferences/kicad`, and both files had also changed at 17:35 and 17:40, before the gate. Something outside the gate writes them; not found. After the gate the branch gained this line only.

**Before 2026-11-23.** Waiting for the owner (no stage, ZR0 and ZR4 included, started before his word on the five decisions). Research and a design down to function skeletons: [ZONE_ROUTING_DESIGN.md](ZONE_ROUTING_DESIGN.md), merged as [#102](https://github.com/yuvaltal912-sys/Konnect/pull/102) on the owner's word (main `7bd3f77b`, a noreply merge whose tree equals the PR head `1fb90cd2`; written on branch `claude/zone-router-research`, from `fork/main` `517eebf0`). The owner approved the exact title and body ("כן, לפרסם") and the merge ("כן, למזג"), and said the same day that no build work starts yet ("אל תתחיל עדיין עבודה של בניה"). No code, no measurement. Reviewed by an agent that did not write it (22 facts checked, 13 design findings, all taken into the text), and its 9 skeleton blocks compile against fastroute `61d6e45` and Konnect `517eebf0` (`cargo check`, 0 errors; the record is the design's "Verification of this document"). It needs the owner's word on its five decisions before any stage starts.

**Addendum for x19, 2026-11-22**, merged as [#106](https://github.com/yuvaltal912-sys/Konnect/pull/106) on the owner's word (main `b78371d2`, a noreply merge whose tree equals the PR head `04602f80`; written on branch `claude/zr-x19-addendum-8185b5` from `fork/main` `5a8273ea`; docs only). The owner approved the exact title and body ("Yes, publish") and the merge ("Yes, merge"). The section "The design on x19" brings the design in line with fastroute x19 (branch `gr`, `b3ea917`, local), the build Row GS pins: S1, F1 and the group search on top of the engine the design read. Its decisions Z14 to Z21: ZR1 to ZR3 are built on `gr` once Row GS has merged, and decision 2 is rewritten to say so (Z14); a move of the group search redraws the model on its own board copy after each member that changed a modelled layer, so a move that strands a fill piece reads open (Z15); a model is set through `set_planes` with a process-wide unique generation, and the three caches that depend on it miss after a redraw: the contact cache, the open-count memo for modelled nets, and `StatsCache` (Z16); the search's cut cost is a rule, never more fill pieces or starved pins, not a term of its value (Z17); the order is closing after the routing passes, the optimizer, closing, the group search, W8 and the pair stages, then a redraw and a closing only when it finds work (Z18); the fixed-state setter of x19 stays safe if nothing in the model reads the fixed state and the stitching marking stays at load (Z19); the uncounted pour nets keep their content and stay out of the cost until ZR5 (Z20); the switch is `--x.zones`, so the probe can see it (Z21). Lines of the design it made wrong are corrected in place and say so. Verified by reading only (the engine at `b3ea917` against `61d6e45`, Konnect's `claude/router-group-search` `e1c3fdf5`); nothing compiled, routed or measured, and its skeletons were not compiled. Reviewed by an agent that did not write it (Opus 5.5, high: 3 must-fix, 8 should-fix, 13 notes, each checked against the code and fixed in the text; the record is the addendum's "Verification of this addendum").

**Why.** The owner, 2026-11-22: the router does not consider zone fills and does not define zones. Both hold. The export gives the router each pour's outline and none of its fill settings; fastroute counts a pad whose centre is inside the outline as connected and lets other nets cross a pour at no cost; nothing creates a zone, and the `kicad-pcb` skill tells agents to add pours after routing. Row S6's matrix has the cost in counts: 19 ground connections open that the router calls closed with pours as planes, and seven times the hand routing's ground track with pours omitted.

**The proposal.** A fill model inside fastroute (a raster, kept on the safe side), with KiCad's refill still the judge. Six stages, each its own PR and each measured: ZR0 baseline measures and a probe (Konnect, no routing change); ZR1 the model and its report (engine, no routing change); ZR2 truth (live fill pieces, a closing stage); ZR3 a cost for cutting a pour; ZR4 a read-only `plan_pours` tool and the intent's `pours`; ZR5 pour measures in the rank key.

**Dependencies.** ZR1 to ZR3 are fastroute work, built on x19 (`gr`) once Row GS has merged (the rewritten owner decision 2, Z14). ZR0 touches no engine file; its baseline is x19's, and Row GS is merged (#108). ZR0b and ZR4 touch no engine file.

**Files it names.** Konnect: new `zone_fill.rs`, `pour_plan.rs`, `tools/pour_plan_tool.rs`; changes in `specctra.rs`, `fastroute.rs`, `route_judge.rs`, `tools/route_fastroute.rs`, `tools/design_intent.rs`, `tools/pcb_board.rs`, `packaging/1-kopper/fastroute.json`. fastroute: new `crates/fr-engine/src/planes/`; on x19 also `pipeline/group_search.rs`, `pipeline/autorouter.rs`, `board/routing_board.rs` (`set_planes`) and `pipeline/stats.rs`.

**Done when** the owner has answered the design's decisions; then each stage as every row: baseline gate, tests with a negative control for each guard, a review by an agent that did not write it, the gate again, and publication on his yes to the exact PR text.

## Row PS, the best-of-N placement search (bench)

**State, 2026-11-19.** Merged on the owner's word as [#90](https://github.com/yuvaltal912-sys/Konnect/pull/90): main `df6c4e7e`, a noreply merge whose tree equals the PR head `2632c073` (branch `claude/placement-search-design`, base `4a638a1d`). Design commits are `894fa4a1` and `c79e1421`, from `fork/main` `d5588205`; the build commits run from stage A to `8465b08a`, and then `fork/main` `4a638a1d` was merged in. The owner started the build on 2026-11-18 ("Yes, start the build"). The bodies were written by Sonnet 5.5 (high) agents from the design's skeletons: [`PLACEMENT_SEARCH_DESIGN.md`](PLACEMENT_SEARCH_DESIGN.md). It adds no tool and has no apply path (Q1). No owner board has run.

**Owner's flow, 2026-11-19 (merged as [#92](https://github.com/yuvaltal912-sys/Konnect/pull/92), main `8795de82`; built on `claude/ps-locks-only` from `fork/main` `a80d4eea`).** `bench.json` gets two per-board options. `"hold"` is `locks` (default: KiCad's locks, `held.json`'s list and the parts with no pad on a net, through `HoldRules::locks_only()`) or `rules` (the earlier rules). `"families"` defaults to `["force", "schematic"]`; `"perturb"` brings P back. An unknown value or an empty list is refused. The search records the hold rules in `search.json`'s plan. Verified: `cargo test -p konnect-core --lib place_` 209 passed, 0 failed, 1 ignored (the bench); the new tests have negative controls (the `rules` mode holds the unlocked connector, naming `perturb` brings the 128 P recipes back). After the independent review, three tests prove the wiring (the search uses `Plan::hold`, `search.json` records `hold_rules`, `run_board`'s `board_plan` passes the board's hold rules and matrix); each fails with its line reverted. `table.md` has no row and no verdict for a family the matrix leaves out, and names the families that ran. The bench and the gate were not run on this branch.

**Schematic-neighbour measure, 2026-11-19 (merged as [#94](https://github.com/yuvaltal912-sys/Konnect/pull/94); built on `claude/ps-schematic-neighbours` from `fork/main` `8795de82`).** The owner asked for it: a circuit tied to a board interface should sit near it (the "12 V present" LED and its resistor next to the 12 V connector), and nothing measured that, because both nets are board-wide power nets. The schematic encodes the intent, so the new module `place_schematic.rs` reads the project's schematic from the bench's `input/` (the root named like the board, hierarchical sheets followed, a sheet file used twice being two instances), fixes once each part's 3 nearest schematic neighbours on the same sheet instance, and measures every candidate against them (mean distance between courtyard centres, the ratio to the input's, and the parts pulled away: more than 2x and more than 10 mm above the input's). It is **reported only**: `search.json` has `schematic_neighbours` per candidate (`null` with a reason when the schematic is missing), `table.md` two columns, and the rank key and the verdicts are unchanged. Design: D23 in [`PLACEMENT_SEARCH_DESIGN.md`](PLACEMENT_SEARCH_DESIGN.md). Its tests have negative controls (every guard's line was reverted and its test failed). It ran on the owner's board "dash" on 2026-11-20 (below).

**Built since #92, merged as [#94](https://github.com/yuvaltal912-sys/Konnect/pull/94) on 2026-11-20** (main `a68a0fa7`, a noreply merge whose tree equals the PR head `503ba63f`; branch `claude/ps-schematic-neighbours`):
- The schematic-neighbour measure (D23), reported only.
- Largest-first `legalize` (generator 7): the moved parts settle by decreasing courtyard-box area, ties by reference, after the held and unmoved parts. Reason: generation failed for a 19 × 50.8 mm module on a 100.5 × 60.5 mm board, 160 of 160 recipes.
- Decoupling caps after their IC, searching from its pad (generator 8): a moved cap settles right after its IC, or first among the moved parts when the IC is held or unmoved, and its spiral starts at the IC pad. Reason: with largest-first alone the best routed F candidates had 5 to 7 decoupling violations, and the owner's own placement has 0.
- The routing record's pair fields: each candidate's `route` record in `search.json` carries the call's `differential_pairs`, `differential_pairs_unresolved` and, when present, `differential_pairs_note`, so a run shows whether a pair was passed.
- Main with Row DP ([#93](https://github.com/yuvaltal912-sys/Konnect/pull/93), `d758fbab`) merged in (see "Row DP").

**Verified, the owner's new board "dash", 2026-11-20** (101 parts; 4 locked, then 12: 4 mounting holes plus 8 connectors; GND pours on both layers; effort m, 100 min; families F and S):
- Generation failed 160 of 160 before largest-first, and 0 after.
- With 4 locks, the hand placement ranked 1 to 3 and the best F ranked 4 (decoupling 4, connector 5).
- With 12 locks, the best F ranked 4 (decoupling 5, connector 0).
- With generator 8 and the CAN pair, the best F ranked 4 and the best S ranked 5, both passing every gate, with decoupling 2 against 0 for the hand placement.
- With the pair passed, the hand placement's CAN pair routed at equal lengths (42.3 / 42.3 mm, against 36.7 / 41.1 without) and 13.6 mm coupled within 0.5 mm (against 0.7).
- Unit tests on this branch: `cargo test -p konnect-core --lib place_` 247 passed, 0 failed, 1 ignored (the bench); `route_pairs` 29 passed; the konnect crate's `asset_references` 12 and `schema_parameter_usage` 5 passed; fmt and `clippy --workspace --all-targets -D warnings` clean. `cargo mutants` on the new `legalize` code: every miss killed by a test or named; the survivors are the old outline-containment and equal-gap comparisons.
- The gate on `c715c774` (main `d758fbab` merged in), run twice, 2026-11-20, records in `target/creation-research/placement-search/gate/ps-dp-merged` and `ps-dp-merged2`. Each run was red on one test from main's code that this branch does not touch, and each test passed on its retry; the other run of each suite was clean. Run 1: `freerouting_setup_tests::a_session_unreadable_for_widths_is_reported_and_not_complete` failed once (its fake Java was not found; 4252 passed, 1 failed). Run 2: workspace tests 5183 passed, 0 failed, 170 ignored; `simulation_live` 94 of 95, with `every_simulation_demo_flattens_or_names_its_export_failure` failing on the IBIS demo (`Cannot find U1.cache.`, the known flake while KiCad is open; it was open); run 1 had 95 of 95. Both runs: sandboxed fastroute 38 passed, doc tests, fmt, `clippy -D warnings`, `deny`, `fix-doc-counts` changing nothing, the release build, protected files unchanged, the installed binary's SHA-256 starting `3de188bb775ab4ce`.

**Not verified:** the A arm, and B4 to B6.

**Next:** none in progress. Open, each on the owner's word: why two decoupling caps on dash stay out of range; the A arm; B4 to B6.

**What it is.** The bench is an ignored test, `place_bench`, in `konnect-core`. It runs on board copies named in a bench folder's `bench.json`. On each board it does the following:
- It makes 288 candidates with all three families, 160 by default since 2026-11-19: F, the force core from a seeded scatter; S, schematic clusters shelf-packed; P, seeded edits of the owner's placement (off unless `bench.json` names `"perturb"`).
- It screens them.
- It routes the owner's placement (O) and the AI's (A) 3 times each, and 4 candidates per family once each. Each routing is one best-of-N fastroute call.
- It ranks everything under one key and writes `search.json` (every candidate, rebuildable) and `table.md` (counts only).

The code is in these places:
- the modules `place_held`, `place_generators`, `place_judge`, `place_route`, `place_search` and `place_bench_tests`;
- `tools/placement/force_core.rs` and `evaluate.rs`, taken out of `tools/placement.rs`. 27 golden fixtures prove the existing placement tools unchanged;
- `pcb_components::placements_applied_to` and `route_judge::rank_measures`;
- `xtask/placement-bench/net_rules.py` and the bench `README.md`.

**Choices made during the build**, each with its reason:
- **Unmoved parts.** `legalize` leaves a part that did not move as it is. Reason: an input's own intended overlap is not a candidate's fault.
- **The S grid.** It is shelf-packed, each part at its own width. Reason: the square grid overflowed the demo board.
- **Gaps.** A moved part keeps the largest rule clearance plus 0.1 mm (solder mask) from any box that is not a real courtyard. It also keeps the copper-to-edge clearance inside the outline. Reason: smoke 3's P2 failures were `clearance`, `copper_edge_clearance` and `solder_mask_bridge`.
- **Copper obstacles.** A moved part keeps that gap from the board's fixed copper: text, graphics, and rule areas that keep parts out. A part with through-hole pads occupies both sides. Copper text without a render cache is estimated:
  - width = characters × size × 1.3 + thickness;
  - height = lines × size × 1.7 + thickness.

  KiCad's own boxes for the demo's 19 copper texts were at most 1.22 and 1.70. Reason: in smoke 4, every routed F and S candidate failed P2. They had 12–24 `clearance` and 11–20 `solder_mask_bridge` each, all pads on copper text, plus one through-hole pad on a bottom-side pad. Known limits, which P2 still catches: copper inside footprints, text with `${…}`, and text overflowing a text box.
- **The smoke board.** The smoke uses the routing row's prepared copy of the public demo board, and holds P1 (terminal block) and P3 (ZIF socket) as connectors under decision 3. Since 2026-11-19 its `bench.json` entry says `"hold": "rules"` and all three families, so it keeps this coverage. Reason: the J-prefix rule misses this demo's P-named connectors. Without the hold, P3 (34 × 82.5 mm) fits nowhere once copper text is an obstacle.

**Verified, 2026-11-18 to 2026-11-19.**
- **Baseline gate** on `c79e1421`, which passed. Tests: 4923 passed, sandboxed 33, doc tests 9, `simulation_live` 95/95. Clippy, `deny`, `fix-doc-counts`, the release build and the installed hash all passed.
- **Unit tests.** `cargo test -p konnect-core --lib place_` on `8465b08a`: 201 passed, 0 failed, 1 ignored (the bench).
- **`cargo mutants`**, 4 rounds on the new and changed code, caught/missed:
  - 175/45;
  - 248/33;
  - 134/4;
  - 98/2.

  Every miss is killed by a test or named equivalent. The survivors are 2 equivalent mutants in the obstacle readers, and 2 at the exact 2 µm gap in `hits_part`, which no float position reaches.
- **Two independent reviews** (Sonnet 5.5). The first had 10 findings, fixed in `3d552232`. The second had 6, on the obstacle code, fixed in `4b27ce6c` to `8465b08a`.
- **Smoke** on the public demo board, at effort fast with a 20-minute limit, in 6 runs. The last run (generator 6, `8465b08a`) gave:
  - 291 candidates in 154 s, with no generation failure;
  - 15 routed (O 3, 4 per family), and every one passes P2;
  - best ranks: P 1, S 7, F 8. One S candidate passes every gate. P beats O: its best ranks above all 3 of O's calls. F and S are below O.
- **After merging `fork/main` `4a638a1d`:**
  - The first gate, on `5c05000d`, failed one test: `an_exported_candidate_passes_the_judge_preflight`. Since Row RJ, the judge accepts a board whose canonical SHA-256 matches. The test's control changed only a space, so it was the same board. `73a44b1d` makes it expect that and refuse a real change instead.
  - Gate on `73a44b1d`: tests 5103, sandboxed 33, doc tests 9, `simulation_live` 95/95. Fmt, clippy, `deny`, `fix-doc-counts` and the release build are green, and the installed hash and the protected files are unchanged.
  - The smoke on `73a44b1d` gave the same counts and verdicts as before the merge: 291 candidates in 153 s, 15 routed, all passing P2, and P beats O.

**Incident.** In stage A, one agent started a focused cargo run while another session's gate held `native.lock`. Since then, every cargo command of this row goes through `chunk.sh`, which waits for the locks. That other gate's result was not checked by this row.

**Not verified.**
- Any run on B4, B5 or B6.
- The A arm: no AI session has run.
- Effort M; the smoke used fast.
- The bench time, about 5.5 hours, which is still an estimate.
- The text estimate on fonts other than the demo's.
- The schematic-neighbour measure (D23) on a real board, and its thresholds (3 neighbours, 2x, 10 mm) on real placements.

**Board counts for the preparation** (read-only copies, counts only):
- copper text: 0 on each board;
- copper graphics: B4 0, B5 0, B6 19 filled 4.6 × 4.7 mm rectangles, each inside one J connector;
- connector-like footprints without a J reference: B6 has 4. Under the owner's flow they are held only if he locks them.

**Next:**
1. The owner supplies one new board, locked in KiCad on a copy. The preparation (the bench README) makes its stripped `input/` with the project's `.kicad_sch` files, `rules.json` and `held.json` as `[]`. The first run is on that board alone. B4, B5 and B6 (locks on the copies in `lock-copies/`: 6, 10 and 6) come later.
2. One A session per board (decision 2).
3. The run at effort M, with the defaults: `"hold": "locks"` and F and S. Read `sch. nbr mm` and `pulled away` beside the verdicts (D23): they do not rank.

Done when each board's `table.md` gives a verdict per family against O and A.

## Row DP (differential pairs to fastroute)

**State, 2026-11-20.** Merged as [#93](https://github.com/yuvaltal912-sys/Konnect/pull/93) on the owner's yes (main `d758fbab`, a noreply merge whose tree equals the PR head `bd8a009a`; branch `claude/fastroute-pairs`, base `8795de82`). The owner asked for CAN_H/CAN_L on his board to be routed coupled. Decision D23 in [`ROUTE_SEARCH_DESIGN.md`](ROUTE_SEARCH_DESIGN.md) has the design and the reasons.

**What it does.** `route_specctra_dsn` (fastroute engine) reads `interfaces[].differential_pairs` from `<board folder>/.konnect/design_intent.json` (the board is the manifest's `board_path`), resolves each name against the DSN's `network` (exact name, else the one net ending with `/` and the name, else the pair is left out and reported), takes `gap=` from the positive net's netclass `diff_pair_gap`, writes `<search folder>/pairs.txt` and sends `--pairs=<file>` on every pass of every run. No intent, or no pairs: the command line is as before. The result has `differential_pairs` and `differential_pairs_unresolved` (empty lists without pairs), and `differential_pairs_note` when the intent cannot be used. `fastroute::CallFiles` carries the amps and pairs files; `--pairs` is not in `CANDIDATE_SWITCHES`. `scratch_board::copy_project` copies `.konnect/design_intent.json` and nothing else of `.konnect`, so the placement bench routes with the same pairs. Code: `tools/route_pairs.rs` (new), `fastroute.rs`, `tools/route_fastroute.rs`, `specctra.rs` (`dsn_net_names`), `scratch_board.rs`.

**Judgment calls to confirm.**
- The gap follows KiCad's class resolution (priority order, then `Default`), so a class that leaves it unset takes the Default's gap. A value of 0 counts as not set and falls through to the next class, then Default, then none (reviewer's fix, 2026-11-20). The brief said "the netclass"; KiCad's own rule and `get_netclasses` both inherit from Default.
- A net is in one pair only; a later pair that reuses one is left out with a reason. A DSN name with white space cannot go in a pairs file; that pair is left out.

**Verified, 2026-11-20 (macOS; commands through the lane driver `chunk.sh`).**
- `cargo test -p konnect-core --lib -- route_pairs fastroute scratch_board` after the review fixes: 165 passed, 0 failed, 1 ignored (before them, with `place_route specctra layout_quality design_intent` added: 394 passed, 0 failed, 9 ignored). 29 new tests in `route_pairs_tests.rs`, plus new ones in `fastroute_tests.rs` (pairs switch only when given, every pass of the hold, every run of a search, the sandboxed run can read the file), `route_fastroute_tests.rs` (result fields, empty lists, no-winner forms) and `scratch_board_tests.rs`.
- `cargo test -p konnect --locked --test asset_references --test schema_parameter_usage`: 12 and 5 passed. `asset_references` needed `differential_pairs` and `differential_pairs_unresolved` in its `NOT_TOOLS` list.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean. `cargo fmt --all` applied.
- Revert checks (a line reverted, its tests failed, file restored from git): 6 of the guards (`--pairs` in `pass_arguments`; the pairs file on the Full, Fallback and Polish pass; the runner handing it to each run; the exact-name rule), and after the review 7 more: in `search_and_write` the plan read from the board's folder and not the DSN's, `write_file`, the runner's `pairs` field, the plan given to `conclude`; in `conclude` the plan given to `result_json` and to `no_winner_result`; and the gap of 0 falling through. The other guards rest on mutants.
- The review's gap in coverage (nothing tested `search_and_write`'s pairs wiring) is closed: the end of the search is `conclude`, tested in both endings, and `a_call_gives_every_run_the_boards_pairs_and_reports_them` drives `route_located` with a fake fastroute that echoes its `--pairs` argument and a fake kicad-cli: the pair comes from the board's own folder (a decoy intent beside the DSN is not read), `pairs.txt` has the exact line, both runs were given its path, the result lists the pair.
- `cargo mutants --in-diff` on the changed non-test files: 57 mutants, 46 caught, 9 unviable, 2 missed; both fixed (a test where class name order and priority disagree; an equivalent empty-parent guard removed). Rerun on `route_pairs.rs`: 37 mutants, 36 caught, 1 unviable, 0 missed.
- End to end, by pipe through the fork's debug `konnect`, on a scratch copy of the public `specctra_pours_routing` fixture (fastroute only under `sandbox-exec`, never `--live`, scratch `KICAD_CONFIG_HOME` and `KONNECT_STATE_DIR`), effort `fast`: with S1/S2 declared as a pair and a class `diff_pair_gap` 0.3 on S1, `pairs.txt` was `pair S1 S2 gap=0.3`, the winner passed every gate with status `complete`, and the session has the pair coupled: minimum edge gap 0.3 mm, 40.2 mm of S1 within 0.5 mm of S2, lengths 53.4 and 53.35 mm. The control without an intent: no pairs file, empty lists, gap 1.53 mm, 0 mm coupled, lengths 52.3 and 34.4 mm. With a pair naming a net the DSN lacks: the pair is in `differential_pairs_unresolved` with its reason and the board routes as before. Records in `target/creation-research/placement-search/dp-check/`.

- The gate on `bd8a009a`, 2026-11-20 (record `target/creation-research/placement-search/gate/dp-final`): every step green; workspace tests 5145 passed, 0 failed; sandboxed fastroute 38 passed; `simulation_live` 95 of 95; fmt, doc tests, `clippy -D warnings`, `deny`, `fix-doc-counts` changing nothing, the release build; protected files unchanged, the installed binary's SHA-256 starting `3de188bb775ab4ce`.
- The owner's board "dash", a copy: with CAN_H/CAN_L declared in its design intent, both nets resolved and were routed as a pair (see "Row PS").

**Not verified.** Windows and Linux. A live KiCad. A sheet-path prefix is tested on unit text only. How fastroute treats a pair whose nets also carry an `--initial-session` was seen working on the fake and on the fixture's `full` pass only.

**Next.** None: the row's done criterion (a route of his board with CAN_H/CAN_L declared reads both nets in `differential_pairs`) was met on a copy on 2026-11-20. Length tuning is Row LT.

## Row AH, the switching loops and pin types

**State, 2026-11-18.** Merged on the owner's word as [#88](https://github.com/yuvaltal912-sys/Konnect/pull/88): main `d8f91867`, a noreply merge whose tree equals the PR head `3f619e3f` (branch `claude/audit-hot-loop-pin-type`, base `3c7858ea`). The same day, `cargo xtask kopper-install` from main installed it on the owner's machine (smoke 13 of 13).

**Why.** Row AP's follow-up. Checks 8.1 and 8.2 find a converter's input pad by name (`VIN`, `PVIN`, `VOUT`, `IN`, `OUT`) and its ground pad by `PGND`, and failing a name took the nearest pad on the net. That could be an enable tied to VIN or a mode input tied to ground, which carry none of the loop's current.

**The fix.** After the name, the nearest pad on the net that `is_signal_pin` does not mark (`input`, `no_connect`); then, as before, the nearest pad on the net. Discrete stages (MOSFETs, diodes) are unchanged.

**Verified, 2026-11-18.**
- `cargo test -p konnect-core --lib -- layout_quality`: 75 passed, 0 failed. The new test: on the buck fixture with U1's pin names removed, the loop closes at the real input and ground pins; it still does with an enable and a mode input placed nearer the switch pad; with only inputs to choose from, it falls back to them; and a named `VIN` or `PGND` typed `input` is still found by its name.
- Negative controls: 5 of 5 caught, run on the final code (ground and power each ignore the type; the name match checks the type; each has no fallback), and the source was restored byte for byte.
- The owner's board has no switch node (8.1 and 8.2 do not run there), so it does not exercise this change.
- Independent review: sound, with three notes, all applied (a name-over-type test case, the one case that can read worse in `API_MIGRATIONS.md`, a variable name).
- Gate on `4d222f26` (before the review notes): tests 4932, sandboxed 33, `simulation_live` 95/95, every step green. The final gate, on `3f619e3f`: tests 4932, sandboxed 33, `simulation_live` 95/95, every main step green; the owner's `kicad_common.json` changed during it, written by the KiCad the owner had open (the gate's sandbox denies writes there).

## Row AP, the decoupling check and pin types

**State, 2026-11-18.** Merged on the owner's word as [#86](https://github.com/yuvaltal912-sys/Konnect/pull/86): main `e49bc3c3`, a noreply merge whose tree equals the PR head `e122ee14` (branch `claude/audit-supply-pin-type`, base `df707dbf`). The same day, `cargo xtask kopper-install` from main installed it on the owner's machine (smoke 13 of 13).

**Why.** On a real board, `audit_layout_quality` reported an IC's reset input, tied to its supply rail, as a decoupling-loop error. Check 4.1 took every IC pad on a power net as a supply pin. It also closed loops at the nearest ground pad even when that was an input tied low (an unused or enable input), so real loops read short. The owner's other session had reported this as the audit measuring U1 with C11 instead of C2. That did not reproduce: on the board after the import and after the refill, the audit measured that IC with its nearest capacitor.

**The fix.**
- `PadGeom` reads KiCad's `pintype`.
- A pad typed `input`, or any type with `no_connect`, on a power net is not a supply pin. The audit names it in `signal_pins_on_power_nets`.
- The loop closes at the nearest ground pad of another type when the IC has one, else at any ground pad, as before.
- Every other type is measured: libraries type supply pins `passive`, and some `bidirectional`. A first version that also skipped `bidirectional` and `output` pins was narrowed for that reason.
- An input left out whose pin function reads like a supply (`V…`, `AV…`, `DV…`, or containing `VCC` or `VDD`) is listed in `needs_human`, so a supply pin a library mistyped as `input` is not dropped silently.
- **Follow-up, not in this row:** the switching hot loops (8.1, 8.2) still close at the nearest ground pad, and pick a power pad, without reading the pin type. Taken up in Row AH.

**On a scratch copy of the owner's board** (counts only): 1 pin left out of 12, with no `needs_human` question. Two loops now close at the real ground pin instead of an input tied low. Both read longer, as the copper is.

**Verified, 2026-11-18.**
- `cargo test -p konnect-core --lib -- layout_quality`: 74 passed, 0 failed.
- Negative controls: 9 of 9 caught, run on the final code, and the source was restored byte for byte. The nine breaks:
  - no pin is a signal pin;
  - a combined type is not split;
  - `bidirectional` counts as a signal;
  - the ground pad ignores the type;
  - no fallback to an input-typed ground;
  - `pintype` is not read;
  - a supply-named input is not asked about;
  - every input is asked about;
  - skipped pins are not reported.
- Independent review: sound, with five notes, all applied (the `needs_human` question, the controls rerun, the 8.x follow-up, generic wording, a bare `no_connect` case).
- Gate on `dd983806` (before the review fixes): tests 4931, sandboxed 33, `simulation_live` 95/95, every step green. The final gate, on `e122ee14`: tests 4931, sandboxed 33, `simulation_live` 95/95, every step green.

## Row RJ, the route judge and live boards

**State, 2026-11-18.** Merged on the owner's word as [#85](https://github.com/yuvaltal912-sys/Konnect/pull/85): main `df707dbf`, a noreply merge whose tree equals the PR head `07e4fb8c` (branch `claude/route-judge-board-identity`, base `d5588205`). Installed on the owner's machine with #86 (main `e49bc3c3`). Its live check, the first `route_specctra_dsn` on a live board from a fresh export, is still to run. **2026-11-21:** that run could not have passed: the importer half of D22 was missing (F1 below). Its fix merged on the owner's word as [#96](https://github.com/yuvaltal912-sys/Konnect/pull/96) (main `dddcd769`, a noreply merge whose tree equals the PR head `0918061c`) and was installed with `cargo xtask kopper-install` on 2026-11-21 (smoke 13 of 13); the live check is next.

**The defect (found on the owner's board, 2026-11-18).** `export_specctra_dsn` hashes KiCad's in-memory text of the board (IPC) into `source_sha256`. The best-of-N judge compared that with the saved file's bytes. KiCad saves the board in another layout and without each footprint's `version`, `generator` and `generator_version`, so the two never matched, and `route_specctra_dsn` refused every board exported from live KiCad, however often it was saved. The fixtures never showed it: their text and their file are the same bytes. The live SES import (`apply_specctra_ses`) compares against the IPC text, so it was not affected. The judge's own import-plan call reads the saved file, so it was (F1 below).

**The fix (D22 in `ROUTE_SEARCH_DESIGN.md`).**
- The export also writes `source_canonical_sha256`: the board as a parsed tree, with that footprint metadata dropped, hashed with lengths and atom/string tags (`board_identity.rs`).
- The judge accepts the saved board when its bytes match `source_sha256` or its tree matches `source_canonical_sha256`. Any other change still refuses. An older manifest gets the byte check only.
- Closes the importer half (F1, Row LT's verification found it on a live board: 6 runs `judge_failed`): the preflight accepted the tree, but the run's own import-plan call still compared bytes. Both now use one rule, `board_identity::matches_export`; the judge's call is `specctra_ses::parse_import_plan_with(.., SourceCheck::BytesOrTree)`. `parse_import_plan`, which the live import (`apply_specctra_ses`) and every other caller use, keeps the byte check.
- DRC's live-to-saved check and the judge now share one normalization rule.

**Verified, 2026-11-18.**
- `cargo test -p konnect-core --lib -- board_identity route_judge specctra drc::`: 262 passed, 0 failed.
- Negative controls: 8 of 8 caught, and the source was restored byte for byte. The eight breaks:
  - the judge checks bytes only;
  - the judge trusts the field without comparing;
  - metadata is dropped from every list, not just footprints;
  - metadata is not dropped;
  - no length framing;
  - an atom and a string share a tag;
  - the export omits the field;
  - the importer lacks the field.
- Independent review: sound, with two notes. This record is one. The other is the next bullet.
- Gate on `7b618aa2`: tests 4929, sandboxed 33, `simulation_live` 95/95; fmt, doc tests, clippy, `deny`, `fix-doc-counts` and the release build green; installed hash and KiCad settings unchanged.
- **Not verified:** a run on a real live board. The tests model KiCad's IPC text (one line, footprint metadata added); they did not capture it. DRC's live-to-saved check rests on the same rule. The first `route_specctra_dsn` after the next install, from a fresh export of a live board, is the check.
- F1 fix, 2026-11-21 (branch `claude/judge-live-hash`): `cargo test -p konnect-core --lib -- specctra route_judge board_identity pcb_routing place_route route_search fastroute`: 461 passed, 0 failed; `asset_references` 12/12, `schema_parameter_usage` 5/5; fmt and `clippy --workspace --all-targets -D warnings` clean. 9 of 9 guard breaks caught. `cargo mutants` on the changed functions: no missed mutant in the new lines. Not run: a real `route_specctra_dsn` on a live board (the run needs the fix installed first).

## Row KE, the `kopper` entry skill

**State, 2026-11-18.** Merged on the owner's word as [#83](https://github.com/yuvaltal912-sys/Konnect/pull/83): main `9f92ab39`, a noreply merge whose tree equals the PR head `1b6b3c0c` (branch `claude/kopper1-entry-skill`, base `a0281a9a`). The same day, `cargo xtask kopper-install` from main installed it on the owner's machine. The work happened in the worktree `/Users/yuvaltal/ClaudeCode/Kopper1`, renamed from `Konnect-plugin` on the owner's word.

**Why.** In Codex, `@kopper1` calls the whole plugin. Claude has no command that loads a plugin at once, so the owner asked for one entry point there.

**What it changes.**
- A new bundled skill, `kopper` (`/kopper1:kopper`): load `konnect` first, then each workflow skill the task touches, from a table with one row per other bundled skill. It adds no rules of its own.
- It is not named `kopper1`. A skill of that name shares the plugin folder's path, `~/.claude/skills/kopper1`, so removing its loose copy would remove the plugin. A test now refuses any skill named like the plugin.
- A test holds the table to one row per other bundled skill and to bundled names only, and holds `konnect` as the first load.

**Verified, 2026-11-18.**
- `cargo test -p konnect --locked --bin konnect`: 105 passed, 0 failed.
- Negative controls: 5 of 5 caught. The five breaks were a dropped table row, a skill that is not bundled, the skill named `kopper1`, `konnect` not loaded first, and a `|` block description. The source was restored byte for byte.
- Independent review: sound, with two notes, both fixed in `e30ddae6` (README count, this record).
- Gate on `e30ddae6`: tests 4923 (the known IPC handshake flake failed once and passed alone); sandboxed 33; `simulation_live` 95/95; fmt, doc tests, clippy, `deny`, `fix-doc-counts` and the release build green; installed hash and KiCad settings unchanged.
- `konnect init` and `init --client codex` on a scratch home holding a copy of the owner's plugins: `kopper` lands in both plugins and in Codex's installed copy, a file the owner put in the plugin folder stays, and `status` reads current for both.

## Row KP, the kopper1 plugin

**State, 2026-11-18.** Merged on the owner's word as [#81](https://github.com/yuvaltal912-sys/Konnect/pull/81): main `cdb00e7c`, a noreply merge whose tree equals the PR head `9c272ef8` (branch `claude/kopper1-plugin`, base `748faeb9`). The same day, `cargo xtask kopper-install` from main installed it on the owner's machine.

**Why.** The owner saw only `konnect` in the `/` menus of Claude and Codex, and asked for all of 1 Kopper's skills in one plugin named `kopper1`. The five `kicad-*` skills had a multi-line `description: |`, and the menus left them out.

**What it changes.**
- `konnect init` (Claude) installs the skills as the plugin `~/.claude/skills/kopper1/`: `.claude-plugin/plugin.json` (name, this build's version, `"skills": ["./skills"]`) and `skills/<name>/`.
- It removes the loose `~/.claude/skills/<name>` copies of Konnect's names, a link as a link. Any other skill stays.
- `init --uninstall` removes the plugin's manifest and skills, and the folder only when empty.
- `status` and the server's guidance check cover the manifest.
- Codex: when the owner's Codex plugin `kopper1` exists (`~/.agents/plugins/kopper1/plugin.json`) and is enabled as `kopper1@<marketplace>` in `~/.codex/config.toml`, the skills go into it and into its installed copies under `~/.codex/plugins/cache/<enabled marketplace>/kopper1/<version>/` whose `plugin.json` is named `kopper1`. Loose copies in `~/.agents/skills` are removed, and its manifests are never touched. Without it, Codex stays loose.
  - How Codex refreshes those copies is not verified, and `status` checks the source only.
  - A plugin disabled after an install keeps the skills it had, and the next `init` writes loose copies beside it.
- On install, the loose copies are removed before the plugin is written; uninstall removes only the `plugin.json` Konnect wrote, then empty folders.
- Every bundled skill's `description` is one quoted line.

## Row JS, the judged route status

**State, 2026-11-17.** Merged on the owner's word as [#79](https://github.com/yuvaltal912-sys/Konnect/pull/79): main `edf31590`, a noreply merge whose tree equals the PR head `55427690` (branch `claude/route-judged-status`, base `0244ce30`).

**The defect (found in Row K3).** When the winner passed every judge gate, `judged_status` kept the router's own status. fastroute counts a connection a pour makes as one it left, so on the public `specctra_pours_routing` board 4 of the 6 recorded routes, and both 1 Kopper smoke runs, read `incomplete` while the judge found 0 unconnected. A winner given a plane read `unverified` for that alone.

**The fix (D21 in `ROUTE_SEARCH_DESIGN.md`, amending D15).** With every gate passed and no stitching via added by the judge, `status` is the judge's:
- `complete`;
- `complete_with_width_exceptions` or `unverified` from the winner's width report;
- `stopped` when the run stopped.

When the judge needed stitching vias, those vias are not in the session, so the router's status stands; `completion.judged.stitching_vias` (new) gives the count and the reason says why. A failed gate decides it as before. The router's count stays in `completion.router_unrouted`, and `completion.reason` adds why the status differs (`JUDGED_STATUS_NOTE`). fastroute's `complete_when` no longer says the status cannot show completion. `cargo xtask kopper-smoke` now also checks that `status` is `complete`: on its fixture the judge added no stitching via and the session had no width exception in both real smoke runs on this branch (`stitching_vias` 0, 13 of 13 checks), so anything else is a regression. `1-KOPPER.md` drops the "Known" line, and `API_MIGRATIONS.md` says what changed.

## Row K4, 1 Kopper: the user guide

**State, 2026-11-17.** Merged on the owner's word as [#77](https://github.com/yuvaltal912-sys/Konnect/pull/77): main `25ea6fe1`, a noreply merge whose tree equals the PR head `58f8f5b2` (branch `claude/1kopper-k4-docs`, base `f5c9c8ec`). Docs only.

**What K4 adds.** [`1-KOPPER.md`](../1-KOPPER.md) at the repository root is the owner's one page for 1 Kopper:
- what it is;
- install (`cargo xtask kopper-install`), check (`cargo xtask kopper-smoke`) and go back (`--rollback`);
- update;
- what it needs and what is where;
- what it never touches;
- how simulation stays off;
- how to change the pinned fastroute build;
- the known `status` reading.

`DEV.md` links to it from the 1 Kopper install entry.

**After the series.**
- Done on the owner's yes, 2026-11-17 (`target/creation-research/1-kopper/PLAN.md` has the details):
  - the fastroute source is backed up as a git bundle in his iCloud `Claude/1-Kopper/` folder. A GitHub copy would need his yes to the exact repository name;
  - the 15 merged worktrees with nothing uncommitted are removed. Their branches are kept.
- The `status` defect from Row K3 is fixed in Row JS ([#79](https://github.com/yuvaltal912-sys/Konnect/pull/79)).

## Row K3, 1 Kopper: the package smoke test

**State, 2026-11-17.** Merged on the owner's word as [#76](https://github.com/yuvaltal912-sys/Konnect/pull/76): main `f5c9c8ec`, a noreply merge whose tree equals the PR head `4cdbf358` (branch `claude/1kopper-k3-smoke`, base `bb4255ff`).

**What K3 adds.** `cargo xtask kopper-smoke [--prefix <dir>] [--kicad-cli <file>]` proves an installed package works. Run it after `kopper-install`.
- It checks that the package is one and that both binaries match its manifest.
- It starts the package's `konnect` by pipe in a fresh folder under `target/kopper-smoke/`, which is also its `HOME`, `TMPDIR`, state and KiCad settings folder. The environment is otherwise empty, and a `konnect.toml` names only `kicad-cli`.
- It checks:
  - there is no simulation toolbox, tool or runtime;
  - the server is the package's binary;
  - `route_specctra_dsn` at `fast`, with no `fastroute_path`, routes the public `specctra_pours_routing` fixture;
  - the route used the package's own fastroute (`bundled`, at its path, with its hash), wrote a session and passed every judge gate;
  - the installed Konnect and the owner's KiCad settings are unchanged.
- **The export is tracked.** `export_specctra_dsn` needs a live KiCad, so the fixture's export is tracked as `specctra_pours_routing.dsn` and `.dsn.konnect.json`, with `board_path` reduced to the file name; the smoke points it at its own copy. The export is the Rust exporter with the tool's options (widths from intent, the project's edge and hole clearances), run on the saved file under the project's Default class. It is not a live KiCad's text or rules, and not KiCad's native exporter. `the_kopper_smoke_export_matches_a_fresh_export` fails when a fresh export differs, and `KONNECT_REWRITE_KOPPER_SMOKE=1` rewrites the pair.

**Found while running it (not K3's).** On the fixture, the winner passed every judge gate, with 0 unconnected after the refill. Even so, the result's `status` was `incomplete`. The router counts the GND connection the pour makes as unrouted, and `judged_status` keeps the router's status whenever the gates pass. It is recorded for its own fix (fixed in Row JS, D21).

## Row K2, 1 Kopper: one install command

**State, 2026-11-17.** Merged on the owner's word as [#75](https://github.com/yuvaltal912-sys/Konnect/pull/75): main `bb4255ff`, a noreply merge whose tree equals the PR head `7d8ac7ca` (branch `claude/1kopper-k2-package`, base `3d585574`).

**What K2 adds.**
- **`cargo xtask kopper-install`**, the one command the owner asked for (2026-11-17, "תארוז את כולם בתוך פקודה אחת"). It runs these steps:
  0. Before it writes anything, it refuses:
     - a Claude entry that is not a stdio entry with a `command`;
     - a Codex `konnect` table whose `command` is not one plain quoted string alone on its line;
     - `CLAUDE_CONFIG_DIR` set;
     - `claude` not on `PATH`;
     - a way-back record that does not read.
  1. It packages, as `kopper-package` below does.
  2. It reads the entries again, then records the first original of each client: Claude's user entry in `~/.claude.json`, and Codex's `[mcp_servers.konnect]` command in `$CODEX_HOME/config.toml` (default `~/.codex`). The record is `<prefix>.rollback.json`, mode 0600, and an original is never overwritten. An entry that runs a 1 Kopper package, even one since removed, is never taken for an original.
  3. It runs the package's `konnect init` for Claude, and for Codex when Codex has an entry.
  4. It points both entries at `<prefix>/bin/konnect`:
     - Claude goes through `claude mcp remove` and `claude mcp add-json`, keeping the entry's other fields. A failed add puts the earlier entry back.
     - For Codex it rewrites that one `command` line in place. It keeps the line ending, the mode and any link to the file, and keeps the file as it was before the first switch as `config.toml.before-1-kopper`.
- **`cargo xtask kopper-install --rollback [--prefix <dir>]`** puts every recorded entry back first, then runs each recorded `konnect` binary's `init`. A failure does not stop the rest, and it keeps the record for another try. A rollback that fully succeeds sets the record aside as `.rollback.json.used`. A new client session picks up either change.
- **`cargo xtask kopper-package`** builds the release, then copies it and the fastroute build pinned in `packaging/1-kopper/fastroute.json` (x19 since Row GS, engine commit `b3ea91773361c2963ec440eaef405f3a8d832372` on the local, unpublished engine branch `gr`, SHA-256 `7616807b713cebb9c425b6ecee40566bd87dbf4cb5094a78c6c4e36d5fd3bb35`; x18 before; the SHA-256 is checked) to `~/.local/share/1-kopper/bin/`. It writes `1-kopper.json` with both hashes. A package already there is kept as `1-kopper.previous`. Before it writes anything, it refuses:
  - any prefix in, over or holding the installed Konnect, under `HOME` and under the account's own home;
  - a relative prefix, or one with `..`;
  - a folder that is not an earlier package, at the prefix or at `.previous`;
  - a `.new` or `.old` left from an interrupted run.
  It never runs fastroute.
- **Konnect** takes a `fastroute` next to its resolved executable only when the executable is inside a package (`1-kopper.json` one folder up). This source comes after the user config and before the default place. A broken bundled build is refused, never passed over for the default one.
- **`konnect init` and `status`** read `simulation_enabled` the way the server does: `--config`, else the usual search. When simulation is off:
  - `init` skips the `kicad-simulation` skill and removes a copy that an earlier install left;
  - the server's guidance check leaves the skill out;
  - `uninstall` still removes it.
  `status` warns on a configuration that does not load and checks every skill, while `init` refuses one.

**Evidence.** Recorded in the PR description: the targeted tests, a negative control per guard, the independent review and the full gate.

## Row BN, best-of-N routing with fastroute

**State, 2026-11-17.** Merged on the owner's word as [#72](https://github.com/yuvaltal912-sys/Konnect/pull/72): main `1c11922a`, a noreply merge whose tree equals the PR head `d52026e7`. The branch was `claude/route-best-of-n`, from `fork/main` `40517d4c` with `6224b579` (#71) merged in: `ff7d9e3e` (feature), `ee146254` (merge), `9f70e67f` (first review round, fastroute x15), `bb904343` (no run cap, fastroute x17), `fe81a441` (second review round), `d52026e7` (gate record). The follow-up branch `claude/route-best-of-n-x18` names fastroute x18 and records the pic rerun on it (below). The design and every decision (D1 to D20) are in [`ROUTE_SEARCH_DESIGN.md`](ROUTE_SEARCH_DESIGN.md).

- **What it does:** `route_specctra_dsn` routes with fastroute by default. One call runs many fastroute runs (seed variants of the placement order, and both pour shapes when the DSN has planes) in parallel within the effort level's budget (`effort`: `fast` 2 runs, `m` 240 s, `h` 720 s, `xh` 2400 s). Konnect judges each run itself on a scratch copy of the board (apply the session, refill, DRC, stitching rounds, layout audit) and picks the winner by three absolute gates (G1 complete, G2 no DRC rule worse than the unrouted board, G3 widths) and then a rank key. `engine: "freerouting"` keeps the old path.
- **fastroute:** the candidate build x18 since 2026-11-17 (engine commit `1e94751`, SHA-256 `ea0334db…`; x17 was `5e7c0348`, `a3cf7852…`). Konnect pins no build: it finds the binary at `~/.local/share/fastroute/candidate/fastroute` unless `fastroute_path`, `KONNECT_FASTROUTE` or `routing.fastroute_path` names another, or (since Row K2) the executable is inside a 1 Kopper package, whose own `bin/fastroute` it takes. A probe proves it is the candidate before routing. It runs only as a separate process under `sandbox-exec` (no network, scratch home), never with `--live`.
- **Safety:** the user's board and project are never written during the search; the judge's kicad-cli runs on a private copy of the KiCad settings, one per judge (D17); a hung run is killed at the hang cap and never wins; runs outside the 8 best are released during the search, and every run but the winner is reduced to its record at the end (D14, D19); the result lists the 20 best runs and counts every outcome, and `search.json` lists every run (D20).

**Verified (2026-11-16).**
- Baseline gate on `40517d4c`: all steps green, 4643 passed, 0 failed, 166 ignored.
- Two independent reviews (Sonnet 5.5, high), one per round. Round 1 (on `ff7d9e3e`) found that the judge stop latched for good, parallel judges shared one KiCad settings folder, a link in the settings aborted the call, and smaller findings; all were fixed in `9f70e67f`. Round 2 (on `bb904343`) found no correctness bug. It led to these fixes: a release no longer follows a linked run or pass folder (`is_real_folder`; test `a_release_follows_no_linked_run_or_pass_folder`, with real folders as the control); and the disk wording of D9 and D19, a stale comment and a stale test-table row were corrected.
- Targeted `cargo mutants` on the guards (`route_search`, `route_judge`, `route_fastroute`, `fastroute`, `scratch_board`):
  - Round 1: 104 mutants, 91 caught, 5 missed; 4 of those were then killed by new tests.
  - Round 2: 97 mutants, 84 caught, 7 missed.
  - After the new tests (`the_runner_releases_a_run_in_its_own_search_folder`, `remove_tree_raises_a_path_it_cannot_read_or_remove`, `a_row_with_no_routed_length_and_no_share_passes_g3`): 50 mutants, 48 caught, 1 missed.
  - Left, each equivalent or unreachable: `abs() > 1e-9` against `>=` in G1; the trait's default `release` (already a no-op); the fixed-count check in `launch_ready` (with no budget, `may_launch` is false anyway); `copy_project`'s empty-parent guard (a canonical board path always has a parent).
- `fastroute::tests::sandboxed::a_pass_past_its_deadline_is_killed_and_waited_for` failed once right after a fresh build (the fake had not written its pid within its 2 s deadline) and passed 5 of 5 on its own. It failed the same way in the follow-up's first gate (2026-11-17, under the full suite's load) and passed on its retry; its deadline is now 6 s on `claude/route-best-of-n-x18`.
- Follow-up gate on `ca2f377f` (2026-11-17): every step green. Tests 4864 passed, 0 failed, 168 ignored; the 31 sandboxed tests passed; `simulation_live` 95 passed; clippy, deny and fix-doc-counts clean; release build; protected files and the installed hash unchanged. The run before it had one unrelated failure under load, `tools::pcb_sync::tests::completion_copper_geometry_changes_invalidate_the_reviewed_revision` (neither branch touches `pcb_sync`); it passed alone and in the rerun. An independent review (Sonnet 5.5, high) checked every figure against the records; its three corrections are in.
- Measurement on the public fixtures (`pic`, `ecc`, `pours`): release binary of `bb904343`, each call in its own comparable lane hold. Every winner passed all three gates, no call ran past its budget, and the protected files and the installed Konnect were unchanged.

| board | effort | wall s | runs judged | other outcomes |
|---|---|---|---|---|
| pic | fast / m / h / xh | 8.6 / 232 / 711 / 2393 | 2 / 216 / 631 / 2284 | judge_failed 0 / 1 / 2 / 6 |
| ecc | fast / m / h | 1.9 / 239 / 720 | 2 / 580 / 1795 | none |
| pours | fast / m / h / xh | 1.9 / 240 / 720 / 2402 | 2 / 542 / 1749 / 5706 | cut_at_budget 0 / 1 / 0 / 20 |

- Largest search folder at its peak: 200 MB (pic, XH). Largest response: 23 KB.
- **Rerun on fastroute x18 (2026-11-17), pic only:** release binary of `64e89fd5` (branch `claude/route-best-of-n-x18`), same lane holds. x18 (W1) leaves out a wire that collapses to one point. No run failed its judge at any level. The six seeds above are now judged (at XH): four fail G1 (1 connection open), one (466) fails G2 (`starved_thermal` 1, the unrouted board 0), and one (699) passes all three gates but ranks below the winner. The winner is the same run as on x17 at M, H and XH (seed 71, pours omitted). Every winner passed all three gates, no call ran past its budget, and the protected files and the installed Konnect were unchanged.

| effort | wall s | runs judged | other outcomes | winner (shape, seed) |
|---|---|---|---|---|
| fast | 8.4 | 2 | none | omitted, 1 |
| m | 235 | 187 | none | omitted, 71 |
| h | 712 | 598 | none | omitted, 71 |
| xh | 2394 | 2127 | none | omitted, 71 |

  Fewer runs judged than on x17 (187 against 216 at M, 598 against 631 at H, 2127 against 2284 at XH): the median route time was 6 to 16 % longer (8.0, 7.65 and 7.3 s against 6.9, 7.2 and 6.8 s), and the host's 1-minute load at the start was higher on x18 at every level (3.5, 7.3 and 5.1 against 1.1, 5.1 and 3.9). Load is the likely cause; x18 itself is not ruled out. Largest search folder at its peak: 184 MB (XH).
- The `judge_failed` runs on pic were a fastroute defect, fixed in x18 (below). On some seeds (106, 280, 403, 466, 699, 1062, pours omitted) x17 left a trace shorter than 0.1 µm whose corners round onto one point; the session then held a one-point wire, and the importer refuses any path of fewer than 2 points. The search went on, and such a run could never win.
- On pours at M, H and XH, the winner keeps the pours. The judge reads 0 unconnected and every gate passes, but `status` is fastroute's own plane-mode value, `incomplete` (D15).
- Final gate on `fe81a441` (2026-11-17): every step green. Tests 4864 passed, 0 failed, 168 ignored; doc tests 9 passed; `simulation_live` 95 passed; clippy, deny and fix-doc-counts clean; release build; protected files and the installed hash unchanged. fastroute's 31 sandboxed tests ran as a step of their own (31 passed) with the cargo parent outside the gate's write-protection profile: each test starts its own `sandbox-exec` with the fastroute profile, and macOS refuses a sandbox inside a sandbox (`sandbox_apply: Operation not permitted`), which is how the first gate run failed those 30 tests and nothing else. No served check was run.

**Not covered.** Only the three public fixtures were measured, and only on macOS. No served test drives a real fastroute end to end: the router tests use a fake binary under the real sandbox.

**Next:** the follow-up PR from `claude/route-best-of-n-x18` (x18 in the docs, this record), on the owner's yes to its exact title and body; the merge only on his word, with the noreply merge command under "Known traps".

## Row E1, board quirks KiCad accepts

**State, 2026-11-16.** Branch `claude/export-board-quirks`, in the worktree `/Users/yuvaltal/ClaudeCode/Konnect-export-quirks`, based on `fork/main` `40517d4c`. The first three fixes are implemented, reviewed, gated and open as [PR #71](https://github.com/yuvaltal912-sys/Konnect/pull/71) on the owner's yes (2026-11-16). The teardrop fix is added to the same branch as its own commit; the PR's updated title and body wait for the owner's yes. The router research found these on the owner's three boards (counts only; the boards are private). Boards that exported before give the same DSN, unless two outline ends were already within 0.01 mm or the board has teardrops.

- **Outline:** `chain_edges` first runs `snap_outline_ends`. KiCad joins Edge.Cuts ends within 10 um, its chaining distance (`DEFAULT_CHAINING_EPSILON_MM`, `pcbnew/board.h:88` in KiCad 10.0.1, Euclidean, nearest end). Konnect does the same in two passes: the corners are fixed in board order, then each end goes to the nearest one. Ends are already on the DSN's 1 um grid, so the limit holds to within 1 um. A line shorter than the distance is dropped; such an arc is refused. Tests: `outline_ends_within_kicads_chaining_distance_are_one_corner` and `outline_ends_join_the_nearest_corner`.
- **Intent nets:** `net_ok` replaces `name_ok` for nets only (rails, currents, interfaces). Outer spaces are kept; an empty or all-space net is refused. Test: `a_net_name_with_outer_spaces_is_kept_as_written`. Control: a rail name with an outer space is still refused.
- **Teardrops** (added on the owner's word, 2026-11-16: "fix the teardrop export too"): `zones()` skips a zone with `(attr (teardrop))` (`is_teardrop`), so it is neither a plane nor a keepout and the importer's `zone_counts` leaves it out; the intent-width pour nets follow. `teardrops_skipped` is in the bundle and the response. KiCad's native exporter writes teardrops as planes, so `rust_only_bridge` keeps the Rust export for teardrops and for renamed references, and `adopt_native_dsn` refuses both as a backstop. Found on an owner board where 312 of 314 zones were teardrops and the DSN had 38 plane nets. Tests: `a_teardrop_zone_is_not_a_pour` (both teardrop types beside a real pour; control: an `attr` with no teardrop is a pour), `features_only_the_rust_exporter_handles_keep_it`.
- **Bench, teardrops** (the owner's yes, 2026-11-16): `score_board.py measure` marks each zone `teardrop` (`IsTeardropArea()`), and `Measure::from_json` leaves teardrops out of `pour_nets`. KiCad's refill rebuilds teardrops from the tracks, so before this every teardrop net counted as a pour net: its lengths went to the pour category and the bench's width checks skipped it (on the owner's board every signal and power length landed in the pour category). Test: `a_measurement_is_read_with_its_pour_nets` (a net with a real pour and a teardrop stays a pour; control: a zone marked `teardrop: false` is a pour). Live check, 2026-11-16: `HOME=<scratch> KICAD_CONFIG_HOME=<scratch> <KiCad.app>/Contents/Frameworks/Python.framework/Versions/Current/bin/python3 -I -B xtask/freerouting-bench/score_board.py measure <scratch>/board.kicad_pcb <scratch>/measure.json`, on `specctra_rounded_board.kicad_pcb` with a `padvia` teardrop zone added; result `zones: [GND B.Cu teardrop false, GND F.Cu teardrop true]`.
- **Audit, teardrops** (found by the bench fix's review): `audit_layout_quality` read a teardrop's fill as a pour of its net (`has_pour` in `layout_quality_routing.rs`), so the net lost its detour and via-per-connection judging. `Fill.teardrop` (set by `specctra::is_teardrop` in `read_fills`) keeps the fill as copper but out of `has_pour`, out of both `fill_at` plane lookups (reference continuity), and out of the unfilled-zone list. Tests: `a_teardrop_does_not_make_its_net_a_pour` (control: the same zone without the attribute is a pour) and `a_teardrop_fill_still_joins_what_it_touches` (a teardrop fill completes a net; control: without it the net is open; an unfilled teardrop leaves no net unjudged). The `fill_at` skip has no test of its own: no fixture has a teardrop under a reference-continuity probe.
- **Shared References:** `dsn_component_names` lists every shared Reference in one refusal. `allow_duplicate_references` names each later repeat `<reference>_DUP<n>` (the first unused `n` from 2), reports it in `renamed_references`, and keeps the Rust export (`rust_only_bridge`: `require` is refused, `prefer` notes it; `adopt_native_dsn` also refuses). Tests: `shared_references_are_refused_or_exported_under_unique_names` (the renamed session imports), `repeated_references_get_unused_names_in_board_order`, `the_export_schema_offers_duplicate_references_off_by_default`.

**Verified (2026-11-16).**
- Baseline gate on `40517d4c`: all nine "Verify" steps green, 4643 passed.
- First three fixes (`30612911`): gate green, 4650 passed, 0 failed, 166 ignored; 11 source mutations, each caught; independent review (Sonnet 5.5, high), nine findings addressed and a second pass on the fixes. An earlier gate run had one `simulation_live` IBIS failure, with KiCad's IBIS cache folder emptied during the run; it passed on the rerun, and this change does not touch simulation.
- Teardrops: independent review (Sonnet 5.5, high) found no code defect and six gaps (tests, docs, a backstop), all addressed. Gate on the branch with it and the bench and audit teardrop fixes: all nine steps green, 4653 passed, 0 failed, 166 ignored; 17 source mutations (the 11 above, the export's teardrop skip, the two `adopt_native_dsn` backstops, the bench's teardrop skip, and the audit's `has_pour` and unfilled-zone skips), each caught by its test. The bench and audit fixes were reviewed by an agent that did not write them, twice.
- The installed binary's SHA-256 starts `3de188bb775ab4ce`.
- Not covered: no served test drives `export_specctra_dsn` end to end with shared References or teardrops, because the IPC mock's board has neither. The handler's choice is covered by `rust_only_bridge`'s test and by `adopt_native_dsn` refusing both.

**Next:** the owner's review and merge of #71 (noreply merge command under "Known traps").

## Row S7b, the audit's corrected acute-corner reading (PR #68)

**State.** The audit reading is implemented, reviewed and gated on branch `claude/creation-s7b-junction-resolver` (2026-11-14), base `fork/main` `61b96dc` (merged #63), with `fork/main` `89780cd` (#66, #67) merged in. Built by a Claude worker (`claude-opus-5-5`) under the creation supervisor. Publication and merge go through [PR #68](https://github.com/yuvaltal912-sys/Konnect/pull/68); GitHub is the source of truth for its state, and the full gate's result is in the PR description. **Next step:** the acute-junction resolver, on its own branch from `fork/main` after #68 merges: authorized by the owner on 2026-11-13, split out on his word of 2026-11-14, reserved for the creation supervisor's worker. It is not in this PR because the independent review of candidate A found four must-fix findings in it. Its first build is not published: it is kept on a local branch in the creation worker's worktree, `/Users/yuvaltal/.codex/worktrees/konnect-s7b-junction-resolver/Konnect`, branch `claude/creation-s7b-junction-resolver` at `7b68dec` (the fork's branch of that name carries only this reading), with its records in that worktree's `target/creation-research/s7b/` (ignored). It is to be rebuilt with the review's fixes and published on its own branch after #68 merges.

**Owner decision (2026-11-13, "תתקן את הביקורת").** A census of the 32 acute corners #63 left on its seven retained boards found 24 readings of tracks shorter than 3 um, 7 junctions within 0.03 degrees of square and 1 real 45-degree V. The owner chose to correct the audit's reading, not to move copper so the audit reads differently. On 2026-11-14 the owner split the row ("Yes, split"): this reading ships in #68 and the resolver in its own PR.

**The corrected reading, one function** (`corner_reading::read_junctions` in `tools/dfm.rs`, with `is_acute`, `is_right_angle`, `is_short_segment`, `is_short_path` and `cluster_is_small`). A track shorter than 10 um, decided in whole nanometres, belongs to the junction it touches and gives no direction. Ends meet when they round to the same micrometre (a key); keys joined by short tracks are one junction while the box around them has a diagonal of at most 20 um (`CLUSTER_SPAN_UM`). A run beyond that (a curve or a leg drawn in pieces) is real copper and is read piece by piece, every piece a leg: the independent review showed that without the bound a 45-degree corner whose leg is drawn in 9-um pieces went unreported and a segment-drawn fillet read as a right angle. The bound is twice the longest piece in the router trial (9.6 um) and the widest run on the seven measured boards (9 um, at most two pieces), well inside a 0.1-mm track's end cap (50 um) and far below the run a wedge needs to open (about 130 um). A corner is acute below 89 degrees, the lower edge of the audit's right-angle band (89 to 91). A junction counts as inside its net's pad or via copper only when every point where its legs end is. Arcs are tracks, read from their end pieces. Short tracks stay visible: `checks.acid_trap.short_tracks`, and the 5.3 metrics `short_tracks` and `junctions_with_short_tracks`.

**Callers and what changed for each.**

| Caller | Change |
|---|---|
| `audit_manufacturing` | fewer acid traps where short tracks or near-square junctions were read as corners; `checks.acid_trap.checked` counts junctions of two or more legs; new `checks.acid_trap.short_tracks`; the issue text shows whole degrees rounded down; the description says below 89 degrees |
| `run_design_review` | its manufacturing part reads the same check; no code change |
| `audit_layout_quality` 5.3 | acute corners are the corrected check; bends, right angles and `acute_found_by_this_scan` read the same junctions; new metrics `short_tracks` and `junctions_with_short_tracks`; acute findings state 89 degrees |
| `plan_route_refinement` | `before.corners` is the corrected 5.3; `t_junction` lists only junctions the corrected check counts (a branch 0.17 degrees off square is no longer repaired; #63's 0.06-degree guard can no longer be reached); `chamfer` reads its rows from the corrected scan without the right angles that hold a short track, which it could only block (they are counted in `before.corners`, neither proposed nor in `blocked`); `coverage.blocked_total` is lower, because the corners only the old reading counted make no rows |
| routing bench (`xtask/src/freerouting_bench/quality.rs`) | its acute and right-angle counts on the SES use the same function; before, it read every piece's direction, counted each gap of a junction and used 89.5 and 90 +/- 0.5 degrees, so older bench reports are not comparable (`DEV.md`) |

**Where the short tracks come from** (router trial, 2026-11-14). Freerouting makes them while it routes; the SES rounding and the import add none (equal counts in the SES file and on the board in 18 runs, and still there with the optimizer off). Its 45-degree corner smoothing stops one internal unit (0.1 um) short of the next corner, and it snaps wire ends to via and pad centres. Of 33 such tracks in three runs, 15 end at a via, 7 are one-unit jogs at a 135-degree bend, 8 are other pieces inside a wire and 3 sit where two wire ends meet. Diagonal pieces reach 9.6 um, 0.4 um inside the 10 um line; the pieces scale with the DSN resolution, so a coarser one makes them longer. Above the line a jog reads as a right angle plus a 135-degree bend, never as acute (tested).

**Measured** (counts only). On the seven boards #63 was measured on: acute corners after #63 8, 5, 5, 5, 3, 3, 3 read as 1, 0, 0, 0, 0, 0, 0; right angles 6, 6, 6, 6, 2, 1, 2 read as 6, 6, 6, 6, 1, 1, 1 (none rises); before #63 acute 11, 5, 5, 5, 5, 5, 5 read as 4, 0, 0, 0, 2, 2, 2. **One real acute corner remains:** under the corrected reading one real acute corner is left on one retained board after PR #63's repairs (the native control run took that board from 4 to 1). This PR does not remove it; that is the resolver's job. Short tracks 15, 14, 14, 15, 13, 12, 13 per board in 12, 6, 6, 7, 9, 8, 9 junctions. Each rule alone after #63: the short-track rule leaves 2, 1, 1, 1, 1, 1, 1, the 89-degree line 7, 4, 4, 4, 2, 2, 2. No run of short pieces on these boards spans more than 9 um, so the 20-um bound changes none of these counts. An independent Python counter with the same rules gives the same numbers on all 14 boards. The native control run (PR #63's three kinds under the corrected reading, KiCad applying, refilling and checking DRC on the 11 staged cases) ran on the build reviewed as candidate A, before the review fixes; the fixes change no count on any of its 105 boards (recounted). It proposed the same repairs as #63's own native matrix: round by round on 10 of 11 cases, and on the eleventh the same 70 `chamfer` proposals paced one round apart by the 48 KiB response limit (the control's scratch board path was 5 characters longer and is repeated in every proposal; `before.corners` carries the two new metrics); after four rounds that board was two listed stitching proposals behind. How many proposals one response lists depends on the length of the board path (a #63 property).

**Review.** An independent review of this part asked for changes: the run bound (M1), the stated threshold (S1), the tool description (S2), the 10-um line in nanometres (S3), one reading for every caller including the bench (S4), `chamfer`'s rows (S5), the control explanation (S6) and this text (S7), plus seven nits. All are in this PR except two nits, with red tests first and a control per new rule. The two left: two long legs side by side, joined by a short piece, still read as a 0-degree acid trap (the copper overlaps and has no wedge; the error flags a trap that is not there and never hides one, and every acute corner on the 14 measured boards is 45 degrees); and the angle search is quadratic in one junction's legs (the 20-um bound keeps them to a few). A second, delta review (B) found the code clean and asked for text fixes only, all made. One limit it names: three or more router pieces at one point can span more than 20 um; that junction is then read piece by piece, as before S7b, so a false acute reading can return there. On the measured boards a run holds at most two pieces.

**Verified** (macOS, scratch config and state, protected-native sandbox; records in `target/creation-research/s7b/`, ignored). On the audit branch after the merge: `cargo test -p konnect-core --locked --lib -- dfm layout_quality route_refine design_review specctra` 414 passed, 0 failed, 9 ignored; `cargo test -p konnect --locked --tests` 195 passed; `cargo test -p xtask --locked` 92 passed; Clippy `-D warnings` clean on `konnect-core` and `xtask`. 25 source-mutation controls, one per rule of the reading, each caught by its named tests. Line coverage (`cargo llvm-cov`): `dfm.rs` 98.3 %, `layout_quality_routing.rs` 96.5 %; the two uncovered lines in the diff got a test. Scoped `cargo mutants` on the changed functions (the junction reader and acid-trap check, the 5.3 scan, the bench's `measure`): 202 caught, 2 timeouts, 28 missed. 21 survivors got a test, and a rerun on the final head kills every one of them; 7 are equivalent or on lines this row did not change, each with a written reason. The full gate and the served check ran on the branch head before publication; their results are in the PR description.

**Not verified.** Windows and Linux; boards beyond PR #63's 11 staged cases and the synthetic fixtures; KiCad's own reading of acid traps (it has none to compare with).

## Row A1, linear netclass pattern matcher

**State.** Implemented on branch `claude/creation-audit-pattern-matcher` (2026-11-13), base `fork/main` `61b96dc` (merged #63). Publication and merge go through that branch's PR; GitHub is the source of truth for its state. **Next step:** none on this branch once merged.

**Changed.** `glob` in `tools/layout_quality.rs`, which `netclasses_from_source` calls for every netclass pattern against every net, is now a two-row table over the name's characters: time proportional to pattern length times name length, no recursion, no backtracking. It is the algorithm of the private `glob` in `route_refine_geometry.rs`; each module keeps its own copy, so keep the two in step. The matching result is unchanged (`*` any run, `?` one character, the rest literal, whole name, case counts). It was exponential in the number of `*` before. No cap, refusal or argument was added; the call site is unchanged.

**Verified (macOS, scratch config and state, protected-native sandbox).**
- Differential test against the old recursive matcher, kept in the test module as the oracle: every pattern up to 5 characters against every name up to 4 over an alphabet with both wildcards, a non-ASCII character and a literal `*`, plus 20,000 seeded longer pairs; a direct test that case counts. A cost test and a call-site test fail on the old matcher.
- `cargo test -p konnect-core --locked --lib -- layout_quality`: 59 passed, 0 failed, 1 ignored (base 55 passed). `-- route_refine`: 116 passed, 0 failed, 1 ignored. `cargo test -p konnect --locked --tests`: all ok. fmt and `cargo clippy --workspace --locked --all-targets -- -D warnings`: exit 0.
- Negative controls, each caught: four breaks of the new matcher fail the differential test, a lowercasing matcher fails the case test, the recursive matcher put back fails the cost and call-site tests.
- `cargo mutants` on `glob`: 17 mutants, 17 caught.
- Release binary over stdio on a scratch copy of a retained board (90 nets) with 90,000 copies of `*?*?*?*Q~` in the project file: plan preparation 135.4 s before, 2.2 s after; `audit_layout_quality` 150.8 s before, 3.1 s after; audit outputs byte-identical. Synthetic boards with the same patterns, after only: 300 nets 5.2 s, 1,000 nets 17.1 s.

**Unfinished.** Preparation still costs patterns times nets (see "Known limits of the planner"); reducing the constant would change the call site and is not done. The full gate ran on the branch head before publication; its result is in the PR description. Not verified: Windows and Linux; KiCad's regular-expression reading of a pattern (not evaluated, as since #63).

## Row S7, read-only route-refinement planner (PR #63)

**State.** Implemented, reviewed and gated on branch `codex/creation-s7-route-refine` (2026-11-13), base `f246e48dab3994de9a0a6fa97176ab8238ce1db9` (merged #62), with main (`e166d97`, PRs #64 and #65) merged in. Publication and merge go through [PR #63](https://github.com/yuvaltal912-sys/Konnect/pull/63); GitHub is the source of truth for this PR's state and merge commit. Plan: [S7_PLAN.md](S7_PLAN.md). [S7_PR_DRAFT.md](S7_PR_DRAFT.md) is the text of the claim draft, kept as a record. **Next step:** none on this branch once #63 is merged; the rows the owner started on 2026-11-13 are in the decision row "Next creation rows".

**History.** Claimed on 2026-11-10 (claim commit `7060528`, the draft PR's first content). The owner stopped the work the same day with the implementation unfinished and resumed it on 2026-11-12 (decision row "Claude resume of three PRs"); it was finished on 2026-11-13 by a Claude worker (`claude-fable-5-2`) under the creation supervisor.

**Contract.** One new `plan_route_refinement` tool in `pcb_routing` (18 to 19 after implementation), returning a versioned saved-source plan with `mutated: false`. Three proposal families, conservative clearance/geometry checks, full terminal-preservation checks, cumulative conflict checks and explicit blocked coverage. Narrow layout-audit model/detector extraction is included in S7 scope; display findings are capped and cannot be the candidate inventory. Existing create/modify tools handle selective adoption; no automatic apply engine or single-undo claim. Independent contract review accepted this bounded approach and identified the midsegment acute-angle blind spot, through-via intermediate-layer clearance and coupled annular-width constraints as required negative controls.

**What the implementation adds to that contract**, each with tests:

- Search cost. Large pour rings get one cached edge tree and point index; distance checks take a cap and stay exact below it; the plane analysis charges the caller's budget through a small `Probe` seam in `layout_quality_planes.rs` (the audit's own path and results are unchanged). On the staged boards a whole plan now needs a few million work units; before, every retained board exhausted its budget.
- Complete reporting. `coverage.phases` (state and counters per kind, `not_started` for a kind a stopped search never reached), `candidates_rejected` by guard, `proposals_by_kind`, `effort.used`, and an `exhaustion` reason for every limit, including the response size, the 200-row unresolved list and the 200-object plan bound. The unresolved list no longer ends the search; one row per junction; refused stitching positions are counted, not listed. Budget exhaustion returns only fully checked proposals with `status: "partial"`.
- New guards: no stitching via on a pad of its own net; the copper of an unplated-hole pad is an obstacle; members of a locked group are locked.
- Inherited clearance for corner repairs, by owner decision and on its condition (decision table above; `Scene::safe`, one place to revert).
- A project that saves `null` class assignments or patterns is read as having none. Before, every project in that common state was refused.
- Track-width floor: the widest class of the net, never below the board minimum. A net's classes are those that name it exactly and those whose pattern matches it under KiCad's wildcard reading (`*`, `?`, everything else literal, whole name, case-sensitive); a class without a width uses Default's and a net no class names uses Default. KiCad's second, regular-expression reading of a pattern and its bus-name fallback are not evaluated: where only one of those puts a net in a wider class, a listed connector can be refused by `route_batch` at adoption, which then writes nothing.
- Effort tiers, calibrated: quick 20,000 candidates / 200,000 pair checks / 20,000,000 work / 5,000 ms; standard 200,000 / 2,000,000 / 200,000,000 / 30,000 ms; deep 2,000,000 / 20,000,000 / 2,000,000,000 / 120,000 ms; a custom budget may go up to deep. One decade apart on the deterministic counters; a search ends on the work limit or the candidate limit, whichever it reaches first. Rationale and measurements: `target/creation-research/s7/resume/calibration.md` (ignored).
- Review fixes: a board with a padstack the model cannot represent is refused; a via's hole is checked against other-net copper on every layer it crosses; nested locked groups and groups that share an id stay locked; copper on the Margin layer is refused; a T-junction whose branch already meets either trunk leg squarely is left alone.

**Effort controls.** `effort: quick|standard|deep` (backend default standard) or a complete, mutually exclusive `budget` object. Geometry, identity, connectivity, response and mutation bounds do not relax with effort. The time limit bounds the search; preparation and the final consistency checks can add time. The shipped skill asks about effort once when no choice or delegation exists, reuses the answer and asks before raising it. That guidance was checked with fresh-context synthetic next-action probes (five without the rule used the default without asking; five with it asked first); these are behaviour samples, not live-client guarantees.

**Verified (2026-11-13; command, result).** On the final source:

- `cargo fmt --all -- --check`: clean. `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean.
- `cargo test -p konnect-core --locked --lib route_refine -- --test-threads=1`: 116 passed, 0 failed, 1 ignored. The same command for `layout_quality`: 55 passed, 0 failed, 1 ignored. For `registry`: 1 passed.
- Source-mutation negative controls, one per guard: 64 of 64 detected over the review rounds (8 of them only after new tests were added), and 21 of 21 on the code the last round changed.
- `cargo mutants`, in place in an isolated validation checkout. Planner scope: 368 mutants, 300 caught, 60 missed, 1 timeout, 7 unviable; after the kill tests 31 were left, each with a written disposition. Class width and lock resolution (`Rules::read`, `Rules::width`, `glob`, `resolve_locks`): 41 mutants, 40 caught, 0 missed, 1 unviable. Corner repair (`Planner::corners`): 156 mutants, 148 caught, 8 alive, 0 timeout, 0 unviable; each survivor has a written disposition (3 equivalent, 1 unreachable, 4 exact-tie boundaries where both outcomes are correct).
- Scoped `cargo llvm-cov`, measured on the first reviewed candidate: lines 94.7 % of `route_refine.rs`, 94.2 % of `route_refine_geometry.rs`, 99.2 % of `route_refine_distance.rs`, 100 % of `route_refine_effort.rs`.
- Independent non-author review of the whole diff, four rounds on frozen candidates. Findings by round (blocker / major / minor / nit): 2 / 3 / 6 / 5, then 0 / 0 / 3 / 4, 0 / 0 / 2 / 4 and 0 / 0 / 0 / 2. Every finding was fixed or is documented as a limit; the last two nits are documentation.
- The native matrix below, on the final source.
- Identified release builds (Cargo artifact message, executable hash, source manifest), and the planner over stdio on 11 stock KiCad demo boards: 0 of 11 inside the conservative profile, each refused whole with its reason.
- The full gate on the branch with main merged in, the release built from that head and the served check over stdio are recorded in the PR text and under `target/creation-research/s7/closeout/`.
- The installed Konnect's SHA-256 is unchanged: `3de188bb775ab4ce5748ea44b94cb098c86ed5d32135e70f4c2c3817c3a4ac65`.

Every run is wrapped in the supervisor's `protected-native-ps.sb` profile, with a scratch `KICAD_CONFIG_HOME` and `KONNECT_STATE_DIR`; a record with command, environment and source hashes sits beside each log under `target/creation-research/s7/resume/` (ignored). The frozen review candidates are beside the worktree.

**Known limits of the planner.** The audit's acid-trap check does not examine a track end that lands mid-segment on another track, so a corner repair is judged there by the planner's own geometry, not by the audit. Outside the three proposal kinds and therefore left unresolved: an acute corner between two tracks, an acute T with no straight trunk, and a right angle that is not a pad-exit dogleg. A response lists about 27 proposals, so a board is refined in rounds: plan, adopt, refill, DRC, plan again. None of 11 stock KiCad demo boards is inside the conservative profile (slotted drills, copper text, local clearance overrides, more than four copper layers, copper drawn inside a footprint); the planner refuses those boards whole and says why. The time limit does not bound preparation. Preparation builds the audit model, whose netclass pattern matcher is linear since row A1 but costs patterns times nets. With 90,000 crafted patterns that is about 5 s on a synthetic 300-net board and 17 s on a 1,000-net one, and `audit_layout_quality` shares it (`preparation_ms` shows it).

**Native matrix, 2026-11-13 (KiCad 10.0.1 on scratch copies; 11 staged cases, up to 4 plan/adopt/refill/DRC rounds, final board against the original).** The 4 synthetic cases meet every target. On the 7 retained boards: right angles fall from 59–76 to 1–6 (limit 8, the existing board's 3 plus 5); every board gets shorter; no island, no width loss, no severity or location change. **Unmet:** acute corners stay at 8, 5, 5, 5, 3, 3, 3 on the seven boards (shapes outside the three proposal kinds; on six boards one of them is a T-junction within 0.06 degrees of square, which the audit counts because the angle is below 90). An earlier measurement counted one more removal per board on four boards; that was a null edit (a track end moved under 0.1 µm, the angle did not change), found and removed on 2026-11-13. Also unmet: on the 3 plane cases the share of ground pads in the main group rises from 0.75 to 0.96 but not to 1, and 1–2 new DRC identities appear. Those identities are relabelled older findings (one `unconnected_items` entry per case, while the unconnected count falls from 19 to 4–5; a `solder_mask_bridge` entry exchanged one for one); none involves copper the plan created or changed. The condition on the inherited-clearance rule counts entries, so it was **not met as worded**. A second run (2026-11-13) settled what the numbers mean: with zero proposals applied and byte-identical boards, KiCad produced the same `unconnected_items` relabel in 2 of 9 runs; the mask-bridge exchange did not reproduce in 12 no-op runs, so its trigger is not shown; and in a run allowed 8 rounds every retained case finished after 4 (the fifth round listed nothing), with the acute count flat after round 1. The owner then kept the rule and accepted both unmet targets as documented limits (decision table above). The matrix was repeated on the final source after the last review round: every target number is the same, and only the work counters differ. Records: `target/creation-research/s7/reports/checkpoint-B.md` and `checkpoint-C.md`.

**Not verified.** Live adoption and undo of a plan in a running KiCad GUI. Windows and Linux. `route_batch`'s refusal of a too-narrow connector by a live run: it is established by reading the code (its preflight runs before the commit, and it is the first operation of a proposal group). Boards inside KiCad's regular-expression reading of a netclass pattern or its bus-name fallback (not evaluated; a documented limit). A real board inside the conservative profile other than the staged cases: none of the 11 stock demos is inside. Coverage after the later review fixes (measured once, on the first reviewed candidate). Direct readback of the KiCad refill setting (next paragraph).

**Native setting provenance.** Scratch configuration is versioned at `native-batch/config/10.0/kicad_advanced`, with `ZoneFillIterativeRefill=0`. KiCad created its scratch settings under that versioned path. The installed release did not emit a direct advanced-setting boolean trace, even with the requested trace environment. One paired 0-versus-1 scratch control produced a toggle-associated saved-fill/DRC change consistent with the setting; it is not direct internal readback or a repeatability claim. Keep that limitation. It is recorded as a limit; no KiCad rebuild or change and no S6 routing sweep was performed.

**Baseline command/results (macOS, at base commit above).** Environment: Homebrew first on PATH, pinned Rust 1.96.0, `KICAD_DEMOS=/Users/yuvaltal/ClaudeCode/kicad-10.0.1/demos`, own worktree `target/` for `CARGO_TARGET_DIR`, and scratch `KICAD_CONFIG_HOME` with copied library tables. The gate driver is `target/creation-research/s7/gate.py` in this worktree; the immutable baseline environment and raw logs are beside its `baseline/results.json`.

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo test --workspace --locked --lib --tests --no-fail-fast` | 4,201 passed, 0 failed, 163 ignored; exit 0 |
| `cargo test --workspace --locked --doc` | 9 passed, 0 failed, 2 ignored; exit 0 |
| `cargo test -p konnect --locked --test simulation_live -- --ignored` | Initial: 94 passed, 1 failed, exit 101; known IBIS `Cannot find U5.cache`. Full retry: 95 passed, 0 failed, exit 0 |
| `cargo test -p konnect --locked --test simulation_live every_simulation_demo_flattens_or_names_its_export_failure -- --ignored --exact` | Isolated retry before full retry: 1 passed, exit 0 |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | exit 0 |
| `cargo deny check licenses` | exit 0 |
| `cargo xtask fix-doc-counts` | exit 0, no changes |
| `cargo build --release --locked -p konnect` | exit 0 |
| `shasum -a 256 ~/.local/share/konnect/bin/konnect` | `3de188bb775ab4ce5748ea44b94cb098c86ed5d32135e70f4c2c3817c3a4ac65` |

The first gate is retained as initially red, with both successful retries; no simulation code was changed. Protected installed-binary/KiCad/Freerouting-setting hashes and mtimes are unchanged. The heavy processes finished before the slot was released to S11. Copies of the logs, independent review and private input inventory are retained under `target/creation-research/s7/preparation/` in the main checkout, apart from the worktree originals. Graphify query used a copied S6 graph, then decisive claims were checked against source; no source edit or graph rebuild was needed for this claim.

## Row S11: suspected zone-fill defects (implemented; PR #64)

Implemented, reviewed and gated on branch `codex/creation-s11-zone-fill-defects`
(2026-11-13): base `f246e48` (merged S6 #62), claim `a423ec8`, then the
implementation commit (read it with
`git log -1 --format=%H -- crates/konnect-core/src/tools/zone_fill_defects.rs`).
Publication and merge go through
[PR #64](https://github.com/yuvaltal912-sys/Konnect/pull/64); GitHub is the
source of truth for its state and merge commit. Worktree
`/Users/yuvaltal/.codex/worktrees/konnect-s11-zone-fill/Konnect`. Evidence is
git-ignored under `target/creation-research/s11/implementation/` in that
worktree: one folder per phase, `reports/checkpoint-*.md` for the checkpoint
reports, and `PROGRESS.md` for the running log. Reviews are in the main
checkout's `target/creation-research/supervision/reviews/pr64-review-*.md`.

**Built.** `zone_fill_check.suspected_fill_defects` on `run_drc` and
`get_drc_violations`, carried into DRC baselines at
`source_evidence.zone_fill.suspected_fill_defects`. It flags resolved
different-net zone/pad and zone/via pairs that KiCad reports at an actual zero
under `clearance` or `hole_clearance` (supported plated holes), or as
`shorting_items`. It counts newly qualifying pairs after refill only from a
complete private-refill comparison: stable board, project and rules snapshots,
matching copied input, unchanged non-fill design data and a changed fill in
the implicated zone. Otherwise the count is null with a named
`attribution_reason`. Original DRC results, severities, counts, the KiCad call
count, tool arguments and the registry are unchanged. Contract
`S11_DESIGN.md`; fields and every reason literal `API_MIGRATIONS.md`; user
guidance `TROUBLESHOOTING.md`.

**What the owner must know (limits).**
- The diagnostic reports KiCad's formatted measurement; it does not
  independently prove physical overlap or an engine bug.
- Taken from the KiCad 10.0.1 source, not from a native run:
  - KiCad stops a rule after 499 `clearance` or 199 `hole_clearance` or
    `shorting_items` entries, excluded markers included. At that limit the
    report is `partial`.
  - A project that sets a supported rule to `ignore` means KiCad never tests
    that rule. The report is `partial` (`kicad_rule_ignored`).
  - Project DRC exclusions use up the limit, and a pair they name can resurface
    after refill. So any exclusion naming either item of any newly reported
    pair withholds the whole newly reported count.
- Not detected: a per-rule `(severity ignore)` in `.kicad_dru`. Not supported:
  copper zones inside footprints (attribution refused), teardrop entries (no
  UUID, counted unresolved), `start_end_only` and other partial-layer padstacks.
- The native refill associations are outcomes observed with KiCad's
  iterative-refill setting on and off, not a readback of the setting.
- Not verified: the KiCad GUI, Windows, Linux, other KiCad versions, and any
  upstream engine cause. No branch coverage was measured.

**Verified on macOS.**
- Full gate on the frozen candidate `review-freeze-C`, run once under both lane
  locks and the 20 s quiet rule, in `final-gate-4/`. Every step exited 0:

  | Step | Result |
  |---|---|
  | `cargo fmt --all -- --check` | passed |
  | `cargo test --workspace --locked --lib --tests --no-fail-fast` | 4,248 passed, 0 failed, 165 ignored |
  | `cargo test --workspace --locked --doc` | 9 passed, 2 ignored |
  | `cargo test -p konnect --locked --test simulation_live -- --ignored` | 95 passed |
  | `cargo clippy --workspace --locked --all-targets -- -D warnings` | passed |
  | `cargo deny check licenses` | passed |
  | `cargo xtask fix-doc-counts` | no change |
  | `cargo build --release --locked -p konnect` | passed |
  | installed Konnect SHA-256 | unchanged |

  Protected files were unchanged. Freeze D adds three tests only; the
  post-commit gate covers it (`final-gate-5/`, recorded in the PR text).
- Focused: `cargo test -p konnect-core --locked --lib --
  zone_fill served_drc_synchronizes_exact_board_and_waits_before_save
  --test-threads=1`. Result: 63 passed, 3 ignored opt-in replays.
  - The second filter adds one served test,
    `tools::drc::tests::served_drc_synchronizes_exact_board_and_waits_before_save`,
    which checks the `saved_ipc_refill` provenance label.
  - `zone_fill` alone gives 62 passed, 3 ignored.
- Negative controls: every guard removal failed its test, and the restored
  suite passed again. The latest rounds were `review-C/controls` (38) and
  `review-D/controls` (3).
- Instrumentation on freeze C (`instr-3/`, `--jobs 1`, line and function
  coverage only):
  - Detector coverage: 830 of 844 lines, 79 of 82 functions. Changed `drc.rs`
    production lines: 32 of 33. The one left is the `saved_ipc_refill` label;
    its served test runs outside the coverage filter.
  - Mutants: 123 selected, 114 caught, 3 unviable, 0 timeout, 6 missed.
  - Of the 6 missed:
    - 2 are equivalent (`Index::layers` `<`→`<=`; a `diagnostic` match guard);
    - 1 is the known qualified-equivalent digit counter in `reported_zero`;
    - 1 is an exact-6,000-byte boundary in `diagnostic`;
    - the other 2 were real test gaps in `Index::layers`. Freeze D added tests
      for them, and a rerun caught both (`review-D/controls/results.json`).
- Native scratch refills of a private board (counts only):
  - with iterative refill on, 4 entries in 2 pairs in 3 of 3 runs;
  - with it off, 0 in 3 of 3 runs;
  - one matched refill of a candidate-free input gave 4 newly qualifying
    entries.
- `kicad-cli` 10.0.8 creates `board.kicad_prl` but no `.kicad_pro`, in both
  primary and `--refill-zones --save-board` modes (`review-B/kicad-pro-probe/`).
- Independent reviews: four rounds by one non-author agent
  (`pr64-review-A.md` to `pr64-review-D.md`). A on freeze A found 1 major,
  7 minor and 6 nit issues; B on freeze B found 2 minor and 3 nit; C on freeze
  C found 1 nit (`project_exclusions_unreadable` covers the whole project
  file, now documented); D on freeze D (three tests only) found none. All are
  fixed or documented.

**Harness facts for the next agent.**
- Run every cargo or native parent under
  `target/creation-research/supervision/native-isolation/protected-native-ps.sb`
  in the main checkout, after checking its SHA-256 `6e2798f1…7bac`. The
  wrapper is `implementation/harness-solution/guard-shared.sh`.
- Use `/opt/homebrew/bin/python3.14 -I` and a scratch `KICAD_CONFIG_HOME`.
  Never pipe `kicad-cli` into `head`.
- `konnect-core`'s `build.rs` reruns on every build in a worktree. Identify
  test and release executables per invocation from Cargo's JSON artifact
  messages, never by path or mtime.
- Lane locks: see the decision row "Lane locks". The S11 drivers that obey them
  are `implementation/lane.sh` (owner-checked release) and
  `implementation/lanes_s11.py`, which implements rules 4 and 5.

**Baseline observations (not S11 items).** These timing-sensitive tests in
unchanged code failed only while other cargo work overlapped a gate, and pass
when run alone.
- `final-gate-3/` ran during a concurrent mutation batch. 15 tests failed:
  12 in `tools::pcb_sync::tests` and 3 in
  `tools::pcb_routing::custom_rule_tests`. They were mock-IPC
  `NNG recv failed after send: Timed out` failures and their follow-ons.
  Alone, they passed 15 of 15, twice (`retry-15/`, `retry-15-C/`).
- Cross-sheet's gate was overlapped by focused runs. 4 fake-java tests in
  `freerouting_setup_tests` and 2 NNG timeouts in `pcb_sync` failed (reported
  by the supervisor).
- `every_simulation_demo_flattens_or_names_its_export_failure` failed once, in
  the cancelled `final-gate-4-freezeB-cancelled/` run. The IBIS demo's deck
  vetting refused with `model_library_missing`, "Cannot find U1.cache". It
  passed in gates 2, 3 and 4. This is a possible intermittent in baseline
  simulation code.

**Post-commit checks.** The final branch head is the merge of the reviewed
source commit `c76970f764d8c26729b8b958550bee1f1a43af1e` with `fork/main` at
`ee64c5e22a6052702a2c84bb11b9c10fe7f87c84` (PR #65 merged). These checks ran
once on that merge:
- one full gate;
- `final_replay.py`, which asserts 4 newly qualifying entries through the
  committed test binary;
- a fresh release build, identified from Cargo's JSON;
- the served MCP checks over stdio;
- `graphify update .` and the installed-hash check.

Their counts are in PR #64's body, and the evidence folders are
`final-gate-5/`, `final-replay/` and `served-mcp/`.

**Next.** None on this branch once #64 is merged; the next creation row waits
for the owner's word.

## Row S9: width-first routing (PR #69)

**State (2026-11-15).** Implemented on branch `claude/creation-s9-width-first`. The branch started from `fork/main` `89780cd` (the merge of #66) and has `fork/main` `6705cd2` (the merge of #68, row S7b's corrected corner reading) merged into it. Publication and merge go through [PR #69](https://github.com/yuvaltal912-sys/Konnect/pull/69); GitHub is the source of truth for its state, and the PR description holds the review, the measurement and the gate. The decisions are in the decision table ("S9 width-first design", "S9 pass list").

**First review (2026-11-14).** One must-fix and six should-fix findings, all fixed on the branch before the second review.

- Must-fix: seeding a later pass panicked on any character outside ASCII in a bare DSN token (a board file, net or pin named in Hebrew, or with `µ`), which would have ended the stdio server. The scan is now over bytes and every cut of input text is checked.
- Should-fix: the width report skipped wire forms it did not measure; a Fallback that closed nothing was kept; the plane fallback's routing step dropped the width policy; two texts promised more than `hold` gives; a failed `hold` call blocked the same call; this handoff was stale. Each is described as it now behaves under "What it adds".
- Owner's words on the scope (2026-11-14): first "תתקן בינתיים רק את מה שחובה ומה שבגדר ״כדאי״ תתעד" (fix only the must-fix for now and document the rest), then, on hearing that the other fixes were already written and tested, "Yes, include them".
- Left from that review, with the reason for each, under "Known limits".

**Second review (2026-11-15).** No must-fix; two should-fix findings and nine nits. The two findings and four of the nits were wrong or incomplete texts, corrected with no change to what the code does: when `width_policy` appears in a result; that "the Full pass narrows nothing" needs `allow_neckdown` off; what happens to a failed call's files; which statuses an unmeasured wire leaves; the count of the first review's findings. Two nits are reading limits, listed under "Known limits". Three concerned untracked records.

**Next step.** The default: `width_policy` stays `"retry"` until the owner has seen this row's measurement and says to change it. Making `"hold"` the default for boards with recorded currents is then one line in `width_arguments` plus its test and docs. Raised for later, not built: widening a narrowed stretch in place, as a planner option.

**What it adds to `route_specctra_dsn`.**

- **A width report, always on.** After routing, the session is read against the manifest's `intent_widths`.
  - `intent_widths`: per net with a recorded width, the routed length, the length and share below that width, the longest such stretch.
  - `width_exceptions`: each stretch of track below its net's intent width (wires that share an end point), longest first, at most 100; `width_exceptions_total` counts all.
  - A net is judged when the export widened it or found it wide enough. A net that owns a pour is not judged.
  - With no such net the session is not read, and the result only gains the three fields, empty.
  - Never silent: keywords are read in any letter case, and a wire is measured as a `path`, a `polyline_path` or a `qarc`. A wire of a judged net in any other form is listed in `width_unmeasured`, and the status is then never a `complete` one: `unverified`, or `incomplete` with open connections, or `stopped` when the time ran out.
  - A wire up to and including half a micrometre below the intent width is at width, compared in whole nanometres.
  - Code: `intent_width_report.rs`; `width_report` in `tools/integration.rs`.
- **Two statuses.** `complete_with_width_exceptions`: the router left nothing, no plane, and some track is below its intent width. `stopped`: the time ran out before a later pass could start or while it ran. S6's rule stays: with a plane no "complete" status is returned. Both are added in `routing_status` and in `completion_report`, where S6's handoff said. A session that cannot be read for widths (`width_report_error`), or that holds a wire that cannot be measured (`width_unmeasured`), is never `complete` of either kind.
- **`width_policy: "hold"`, opt-in.** The default `"retry"` is the single run as before.
  - Full: every net, the retry off, the optimizer on.
  - Fallback, only if Full leaves connections open: started from Full's session, the retry at the one floor width, the optimizer off. `width_fallback: "none"` skips it. It is kept only if it ends with fewer open connections than it started with.
  - What `hold` guarantees: with `allow_neckdown` off (the default), the Full pass narrows nothing. A Fallback may narrow any track, also track the Full pass laid at width; nothing checks which. `width_exceptions` is the authority on what ended narrow.
  - Polish, only if the Fallback completes: started from its session, the retry off, the optimizer on.
  - The retry is never sent with the optimizer on (`check_pass_rule`): the optimizer lays narrowed retries of its own on every net it touches.
  - A later pass starts from a scratch copy of the DSN that holds the earlier session as ordinary wiring, never fixed (`specctra_seed.rs`; the DSN's own bytes are kept and the items are inserted into its `wiring` scope).
  - A later pass's session becomes the result only if its log shows it started with exactly the unrouted count the pass before ended with, it holds every seeded net and no typed item, and it ended as it must (the Fallback with fewer open connections, the Polish with none). Otherwise the earlier session stays, and the pass's row in `passes` says why.
  - `passes`, `result_pass` and `artifacts.passes_folder` appear only when `hold` was asked for. `width_policy` appears then, and also when `width_fallback` was given without `hold`, where it says the argument was not used. The passes' files stay in `<output>.passes`.
  - A call that returns an error leaves no `<output>.passes`, so the same call can be made again: an empty folder is removed, one with files is moved to `<output>.passes.failed-N` (N from 1 to 99). When all 99 names are taken or the move fails, the folder is removed with its files, and the error says so. A call that is killed or crashes leaves the folder.
  - The plane fallback's routing step in `completion` repeats the caller's `width_policy` and `width_fallback`.
  - Code: `hold_widths`, `route_held` and `width_arguments` in `tools/integration.rs`; `router_unrouted_at_start` and the typed `OverallTimeout` in `freerouting_mcp.rs`.
- **Docs.** `docs/API_MIGRATIONS.md` ("width-first routing"), `tool-directory.md`, and the `kicad-pcb` skill (step 3 reads `width_exceptions` and reports every row).

**Choices made in the build, with reasons.**

- The power-first pass of the redesign is not built (decision row "S9 pass list"). `ignore_net_classes` is not sent.
- A later pass that fails, is discarded or runs out of time leaves the earlier full-board session as the result. The redesign said "no session". Reason: that session is whole and is already the honest answer.
- Freerouting's "Necked retry" log lines are not read. They carry no job tag, so it is not known that the job log holds them. The exceptions come from the session's geometry only.
- `route_local` already checks every DSN it is given, so each scratch DSN passes `check_dsn_input` there.
- `specctra_ses.rs` was not touched: both new modules read sessions through `konnect_sexp`.
- Both new modules scan text as bytes and check every cut, so a board, net or pin named in any script is read, and a file they cannot read is an error, never a panic (the first review found a panic here).
- Seeding refuses what it cannot write back exactly: a wire that is not plain paths, a width of zero or less, a quoted name that holds a quote, a backslash or a control character.
- `konnect-ipc/src/builders.rs` and `tools/pcb_routing.rs`, named in the old row, are not needed: nothing is written to the live board inside the call.

**Verified (macOS).** Every cargo, KiCad, Freerouting and `konnect` parent ran under `protected-native-ps.sb` with a scratch `KICAD_CONFIG_HOME` and `KONNECT_STATE_DIR`. Records are under `target/creation-research/s9/` in the main checkout (ignored).

- **Baseline:** not rerun. The base tree `e27bfb2` is the tree #66's final gate passed on 2026-11-14 (4,515 workspace tests passed, 0 failed, 166 ignored; `simulation_live` 95 of 95).
- **Tests:** a failing run first for each behaviour (`build/runs/`), then:
  - `cargo test -p konnect-core --locked --lib`: 3769 passed, 0 failed.
  - `cargo test -p konnect --locked --tests`: 195 passed, 0 failed, in 16 binaries.
  - `cargo clippy --workspace --locked --all-targets -- -D warnings`: clean.
  - These are the focused runs on the branch after the first review's fixes; the full gate's totals are in the PR description.
- **Negative controls, scoped mutants, independent review, measurement, the full gate:** recorded in the PR description.

**Known limits.**

- The Fallback narrows whole connections, as Freerouting's retry does. The Polish does not widen them again. What ends narrow is listed, not repaired.
- The Fallback needs one floor width for every net. When the nets' floors differ it is skipped, as the single run's retry is, and open connections stay open.
- A stretch is a geometric group of wires in the session, not a pad-to-pad connection. Wires join only at their end points: a narrow wire that ends in the middle of another one is a stretch of its own, so the longest stretch reported can be shorter than the narrow copper that hangs together there.
- Copper the board already held locked is not in a session and is not judged.
- `hold` takes up to three routing jobs; `overall_timeout_seconds` covers all of them.
- Left from the first review:
  - A narrow path of no length is listed as an exception of 0 mm. Reason: naming too much is the safe side.
  - A DSN that already holds movable (untyped) wiring gets the seed on top of it. Reason: Konnect's export writes only fixed wiring, and a seed that Freerouting reads differently is caught by the start-count check.
  - The DSN's bare `(string_quote ")` token is recognised in lower case only. Reason: in upper case the DSN does not read and the pass fails with an error; nothing is guessed.
  - Net and layer names are compared as written, not folded to one case. Reason: KiCad's names are case-sensitive.
  - `konnect_sexp::parse_sexp` has no nesting limit, so a file nested hundreds of thousands deep ends the process. Reason: not new (the caller's DSN went through it before this row) and the fix belongs in `konnect-sexp`, outside this row's files: a follow-up.
  - A failure of the last step of a `hold` call (writing the output) goes through the same clean-up as a failed job, but only the failed-job case is run in a test.
- Found by the second review, not changed:
  - A session that holds a second `routes` or `resolution` scope is read by the first of each, with no error. Reason: Freerouting writes one of each and the SES importer refuses a second, so this needs a malformed file; refusing it is a logic change, left for a follow-up.
  - In a session with coarse units (`mm`, `mil` or `inch` at a low resolution), a wire's ends are rounded to a whole session unit before wires are joined, so narrow wires that end within one unit of each other can be reported as one longer stretch. Reason: it errs toward a longer stretch, and Freerouting's own sessions (`um 10`) hold whole units and are not affected.

## Row S6, second half: per-zone pours and stitching after refill (PR #66)

**State (2026-11-14).** Implemented, reviewed and gated on branch `claude/creation-s6b-pour-policy`. Base: `fork/main` `61b96dc`, with `fork/main` `720d86b` (#67, row A1) merged in. Publication and merge go through [PR #66](https://github.com/yuvaltal912-sys/Konnect/pull/66); GitHub is the source of truth for its state, and the final gate's result is in the PR description. The design rulings are in the decision table ("S6 second half design").

**Next step.** None on this branch once #66 is merged. The three unmet targets under "Targets, by shape" go to the owner, with the measurement for each shape; no shape is recommended here. Row S9 (width-first routing) starts after this merge, and is measured on these DSN shapes.

**What it adds.**

- **Per-zone `pours` in `export_specctra_dsn`.**
  - Besides `"fixed"` and `"omit"`, `pours` takes an object: an optional `default`, and a `zones` list of rules. Each rule has `uuid`, `name`, `net` and `layer` selectors, all of which must match, plus a `policy` of `fixed` or `omit`.
  - The unit is one pour on one copper layer, so `layer` splits a zone that spans two layers.
  - Refused: unknown keys and values, a rule with no selector or no policy, an empty selector, more than 256 rules, a non-copper layer, a rule that picks no pour (a rule area is named), and rules that disagree about one pour layer.
  - The manifest lists each pour layer with its policy and the rule that chose it (`pours`). A non-string, non-object `pours` is now refused by the handler too; the schema already refused it at dispatch.
  - Code: `PourPolicy`, `pour_decisions` and the manifest rows in `specctra.rs`; the schema and argument reading in `tools/pcb_export.rs`.
- **Honest routing results in `route_specctra_dsn`.**
  - New `status`: `complete` only when Freerouting reports 0 unrouted and the DSN gave it no conduction area. `incomplete` when it left connections; `unverified` otherwise.
  - A conduction area is a `plane`, or a `wiring` wire with an area shape and a net, matched in any letter case.
  - New `completion`: Freerouting's own count, the planes with their zones, and what decides (KiCad DRC after a refill). It also gives the exact `next_steps`, and, with planes, `if_still_open`.
  - **With planes, the steps start by keeping the exported board** (`save_project`, `create_checkpoint`). The fallback above 10 open connections puts that board back first: close KiCad, `restore_checkpoint` (preview, then apply), `launch_kicad_ui`, open the board. Only then does it export again with `pours: "omit"`, because the exporter refuses a board that holds unlocked routing.
  - `dsn_path` is checked before anything reads it: a regular `.dsn` file of at most 64 MiB.
- **Docs.** The `kicad-pcb` skill teaches the same sequence (Freerouting round trip, steps 3 and 7), saves the project before the export (step 2), and loads the `project` toolset the checkpoint tools are in. It, `tool-directory.md`, `DEV.md` and `docs/API_MIGRATIONS.md` say what `fixed` really writes.
- **Fixtures.** `specctra_pours_policy.kicad_pcb` (five pour layers and a rule area) and `specctra_pours_routing.kicad_pcb` (synthetic, with its generator beside it).

**Measurement (batches S6B-N1 and S6B-N2, 2026-11-13 and 14).**

- **Board and versions:** the private reference board (counts only), S5's test currents, Freerouting 2.5.0.
- **Path:** the product path, a frozen release `konnect` over stdio calling `route_specctra_dsn`.
- **Each run:** the SES is applied and refilled. Then up to 4 stitching rounds: `plan_route_refinement` with the stitching kind over stdio, a `pcbnew` stand-in adopting every listed via, then refill and DRC. Scoring is S5's bench (`--rescore --score`).
- 3 routings per cell.

| Cell: DSN shape | Status (router unrouted) | Open on GND after refill | Open after stitching, R = 3 mm | GND vias at the end | Track on the GND pour net (existing 81.7 mm) | Signals | Power net A below its intent width | Power net B below its intent width | Starved thermals |
|---|---|---|---|---|---|---|---|---|---|
| C0: every pour omitted (S5's preset shape) | `complete` (0), 3 of 3 | 0, 0, 0 | no stitching | 25 to 28 | 583 to 585 mm | x1.003 to x1.005 | 14 % | 0 % | 1 |
| C1: top full pour omitted, region and bottom pours planes | `unverified` (0) | 17, 19, 19 | **5, 3, 3** (R = 6 mm: the same) | 58 to 65 (R = 6: 65 to 74) | 9.9 mm | x1.010 to x1.012 | 16 % | 0 % | 2 |
| C2: every pour fixed (today's default) | `unverified` (0) | 19, 19, 19 | **6, 9, 8** | 36 to 47 | 0 mm | x1.000 to x1.009 | 28 to 36 % | 82 to 84 % | 2 to 3 |
| C3: only the bottom pour a plane, the other two omitted | `unverified` (0) | 1, 1, 1 | **0, 0, 0** | 82 | 71.6 mm | x0.992 | 34 % | 82 % | 1 |
| F1 (synthetic): top pour omitted, bottom pour a plane | `unverified` (0) | 0, 0, 0 | no round needed | 12 | n/a | n/a | n/a | n/a | 0 |
| F1 (synthetic): every pour omitted | `complete` (0), 3 of 3 | 0, 0, 0 | no stitching | 0 | n/a | n/a | n/a | n/a | 0 |

Power net B, the net 82 % below its width in C3, is the same net that is 82 to 84 % below it in C2 (today's default). It is not narrowed at all in C0 or C1. Power net A is the 14 % net of C0. One further power net was 9 % below its width in one C0 run and 0 % in every other run.

- **Targets, by shape:**
  - 0 unconnected on every run: met by C3 only. **Unmet on C1 (5, 3, 3) and C2 (6, 9, 8).** None of them is above 10, so the fallback is not triggered and the open connections are reported as open.
  - At least 40 vias on the ground pour net: met by C1 and C3; unmet by C2 in 2 of 3 runs.
  - Ground track near the existing routing's (at most +20 mm): met by C1, C2 and C3; unmet by C0, by design.
  - Signals at most x1.03: met by every cell.
  - No new DRC category against C0 and the existing routing: met by every cell. The starved-thermal count is 1 on C0 and C3, 2 on C1, and 2 to 3 on C2.
  - Power-net width shares no worse than C0: **unmet by C1, C2 and C3.** On power net A, C1 is 2 points worse than C0 (16 % against 14 %), C2 reaches 28 to 36 % and C3 34 %. Power net B is 0 % in C0 and C1, but 82 to 84 % in C2 and 82 % in C3. The narrowing comes from Freerouting's single retry width, which is row S9's subject.
- **This change does not make C0 worse.** Its DSN is byte-identical to S6's product export, and its numbers match S5's preset: 1 starved thermal, 14 %.
- **What stays open on C1.** All of it is pieces of the F.Cu region pour cut off by tracks: 4 piece-to-piece links and 1 region-to-bottom link in run 1, and 2 and 1 in runs 2 and 3.
  - Each open piece holds exactly one SMD GND pad, with no via and no track.
  - In 4 of the 5 pieces, no position fits a 0.6 mm via wholly inside both the piece and the bottom ground fill. In the fifth, every position that fits lies on the pad.
  - The planner's only refusal is "via-in-pad overlap" (25 or 26 positions), and the 6 mm radius closes nothing more.
  - C3, which leaves that pour out of the router's view, has 0 open.

**For S9, which edits the same two files next.** What this row changed there:

- **`freerouting_mcp.rs`.**
  - `log_lines` was pulled out of `optimizer_report`. It turns the job log into message lines.
  - New `router_unrouted(log)` returns the count of the last summary line. A summary is an "Auto-routing stage completed:" line, an "Optimizer pass #k … was completed" line, or "Skipping optimization stage: the board has N unrouted". If the last summary cannot be read, the count is `None`. Pass lines and the line that starts a stage are ignored.
  - New field `RouteEvidence.router_unrouted`. `run_state_machine` fetches `get_job_logs` once, for both readers.
  - New `check_dsn_input` and `read_dsn`, with the 64 MiB `MAX_DSN_BYTES`; `validate_inputs` uses the check.
  - Job settings, read-back and the isolation of the JAR are unchanged.
- **`tools/integration.rs`.**
  - New `routing_status(planes, router_unrouted)` and `completion_report(dsn, ses, planes, router_unrouted)`, which returns `status` and `completion`.
  - The constants `PLANE_FALLBACK_OPEN` (10), `STITCH_ROUNDS` (4), `STITCH_RADIUS_MM` (3.0, the planner's default) and `MAX_REASON_DETAIL` (160).
  - In `handle_route_specctra_dsn`: `dsn_path` is checked after the JAR is resolved and before Java is looked for. The planes are read before routing. `status` and `completion` are added after `success`.
  - Tests: `route_rule_tests` (`assert_calls_are_real`, `assert_prose_names_are_real`). In `freerouting_setup_tests`, an end-to-end handler test with a fake Freerouting checks the status wiring.
- **Where S9's statuses go.**
  - Add `complete_with_width_exceptions` and `stopped` in two places in `completion_report`: the `status` match (where `incomplete` already wins over unreadable planes), and the `reason` match. Also add them in `routing_status`.
  - One rule must stay: with a conduction area in the DSN, no "complete" status is ever returned.
  - Pass the last pass's unrouted count. Per-pass detail belongs in S9's own `passes` field. `next_steps` stays the KiCad-side sequence after the call.

**Verified (macOS).** Every cargo, KiCad, Freerouting and `konnect` parent ran under `protected-native-ps.sb`, with a scratch `KICAD_CONFIG_HOME` and `KONNECT_STATE_DIR`, except one `cargo fmt --all` (formatting only, no KiCad), run outside it on 2026-11-14. Records are under `target/creation-research/s6b/` (ignored).

- **Tests:**
  - `cargo test -p konnect-core --locked --lib -- specctra:: freerouting_mcp::tests tools::integration::route_rule_tests tools::integration::freerouting_setup_tests tools::pcb_export::new_export_format_tests layout_quality`: 202 passed, 0 failed.
  - `cargo test -p konnect --locked --tests`: 195 passed, 0 failed, in 16 binaries.
  - The whole konnect-core lib on candidate B: 3633 passed, 0 failed.
- **Negative controls:** one guard broken per run, its named test failing, the file restored byte for byte. Batches K1 (38 controls on the first candidate), K2 (the 12 surviving mutants of the first candidate), K3 (67 controls: every guard on the final source, 17 of them new for the review's findings) K4 (the 7 killable survivors of the delta mutants) and K5 (7 controls for the wording review C asked for: the save, checkpoint and restore steps, and the skill's round trip). Every control is killed, and each file is restored. In K3 one control survived at first: the fixed parse-error message. Its test read only the top context of the error; it now reads the whole chain, as `completion.reason` shows it, and the control is killed (K3b).
- **`cargo mutants`** on the changed lines, in isolated copies:
  - First candidate: 122 mutants, 101 caught, 12 missed, 9 unviable. Each of the 12 now has a kill test, shown by re-applying it.
  - On the review fixes (the delta from B): 51 mutants, 38 caught, 8 missed, 5 unviable. 7 of the 8 now have kill tests, shown by re-applying them (K4). The eighth turns the bounded read's `take(MAX + 1)` into `take(MAX)`. It differs only for a file that grows while it is read, a race no test can produce, so it is recorded as equivalent.
  - None on the changes for review C: they change strings and tests only.
- **Independent review:** two rounds by an agent that did not write the code. Review B returned CHANGES REQUIRED; its 2 must-fix and 5 should-fix findings were fixed with tests, and 10 of its 11 nits (nit 7 left: KiCad's DRC is the judge, and a violation count would widen the contract). Review C, of the delta, returned CLEAN WITH NITS; its should-fix (this row's count of unmet targets) and its 10 nits were taken: nit 9 by dropping two kinds of board detail, and nit 4 in part, since a project file that does not exist is left to `create_checkpoint`, which refuses it, and the step now says to stop. Nit 1, below under "Known limits", is handled in the wording of the steps and the skill.
- **The measurement above.**
- **The full gate on the merged head:** recorded in the PR description.

**Known limits.**

- A pour form that blocks other nets has no per-zone switch in Freerouting 2.5.0: `plane_as_obstacle` is board-wide, and a DSN plane is read with `isObstacle = false`. Evidence: bytecode notes in the main checkout, `target/creation-research/resolution-evidence/notes/rank32-checks-backlog.md` §3 and `notes/routing-quality.md` (ignored files).
- The steps cannot run inside the routing call: apply, refill and `route_batch` need the live board.
- The fallback needs KiCad closed for `restore_checkpoint`, and reopening the board in the PCB editor is a manual step; no tool closes or opens a board.
- The import binds the project file by hash, and the `save_project` step can make KiCad rewrite it when KiCad had not saved it itself. The import then reports a stale revision, and the recovery is a new export and routing run. The step says so, and the skill saves the project before the export so that this save has nothing new to write. Not run against a live KiCad.
- The planner's stitching closes only the open pieces it finds a safe via position for. On C1, what stays open after the stitching rounds (5, 3 and 3 connections in the three runs, the same at a 6 mm radius) is all cut-off pieces of the F.Cu region pour.
  - Each piece holds one SMD ground pad, with no via and no track.
  - In 4 of the 5 pieces, no 0.6 mm via fits wholly inside both the piece and the bottom ground fill.
  - In the fifth, every position that fits lies on the pad, and the planner refuses a via in a pad ("via-in-pad overlap", 25 or 26 positions).

**Not verified.**

- Live apply, `route_batch`, checkpoint and restore in a running KiCad GUI: the measurement used `pcbnew` stand-ins, as S7 did.
- The fallback branch: no run had more than 10 open.
- Windows and Linux.
- Another real board.
- The owner's real currents.
- `cargo llvm-cov`: not run. The diff-scoped mutants and the negative controls exercise every changed branch, and an instrumented build would hold a lane on a machine shared with other workers for little more evidence.
- That Freerouting 2.5.0 reads an area-shaped wiring wire as a conduction area: this was not checked against 2.5.0's reader. Counting such wires as planes is the conservative reading.

## Row S6, first half: pour widths and measured preset (PR #62)

2026-11-10. Branch `codex/creation-s6-pour-widths-preset`, base `7e9a174`, local claim `dace50b`. Implementation commit: `803ab9431cdfeeafb26c0b05b29a963175e83e8a`; the following docs-only commit records this revision. The owner explicitly authorized this row, including the pour-width default and all measurement runs; the owner subsequently approved publication and merge of [PR #62](https://github.com/yuvaltal912-sys/Konnect/pull/62), with the exact shown title/body.

**Changed.** `widen_for_intent` uses the existing copper-zone classifier and resolved net identity to retain a pour-owning net's effective class width, even when pours are omitted. Rule areas and non-copper zones do not qualify. `intent_widths` records `not_widened`, the retained width and a reason that explicitly does not claim verified pour capacity. Other current-bearing nets still widen; missing class widths remain refused. No new argument or plane policy.

`route_specctra_dsn` sends S5's preset: 20 passes and optimizer enabled by default (existing overrides retained), via cost 10, undesired-direction scalar 1.0, both optimizer consecutive-failure limits 1,000,000 and improvement threshold 0.25 percent. The job API receives `0.25`; the matching CLI receives `0.0025`. Every sent leaf is read back before routing. `API_MIGRATIONS.md` and tool schema text describe the changed defaults. The ignored file-export probe now saves the real reverse manifest beside its DSN for acceptance runs.

**Verified on macOS.** Commands, raw outputs, matrix decisions, hashes and the review are private/ignored under `target/creation-research/s6/` in the main checkout (implementation and measurement subfolders); copies also remain in the S6 worktrees.

- Baseline gate: 4,194 workspace tests, 9 doc tests, 95 live simulation tests; all passed. Final gate: 4,201 workspace tests, 9 doc tests, 95 live simulation tests; all passed, 163 workspace and 2 doc tests ignored. Commands: `cargo fmt --all -- --check`; `cargo test --workspace --locked --lib --tests --no-fail-fast`; `cargo test --workspace --locked --doc`; `cargo test -p konnect --locked --test simulation_live -- --ignored`; `cargo clippy --workspace --locked --all-targets -- -D warnings`; `cargo deny check licenses`; `cargo xtask fix-doc-counts` (no change); `cargo build --release --locked -p konnect`. Homebrew first on PATH, `KICAD_DEMOS` set, and scratch `KICAD_CONFIG_HOME` throughout. Full command/exit records: `implementation/baseline/results.json` and `implementation/final-gate/results.json`; `implementation/post-commit-gate/results.json` holds the same gate rerun after the local commits.
- The first final-gate live suite hit the known IBIS cache flake (`Cannot find U1.cache`): 94 passed, 1 failed. `cargo test -p konnect --locked --test simulation_live every_simulation_demo_flattens_or_names_its_export_failure -- --ignored --exact` passed alone, then the full 95-test command above passed. No simulation change was made; initial and retry logs are retained.
- Seven new tests, plus prior width/floor regressions retained on a fixture without pours. `cargo test -p konnect-core --locked --lib`: 3,333 passed, 43 ignored. Nine source-mutation negative controls were each caught, then restored: pour exemption, pass default, optimizer default, via cost, direction cost, threshold units, both optimizer early-stop limits, and the effective-settings guard. Restored targeted runs: all seven passed.
- Direct baseline/candidate file-export probes (`freerouting_routes_a_folder`, ignored, `FREEROUTING_JAVA=/usr/bin/true`, no JAR execution) on scratch stripped copies of all 19 demo boards: the same nine exported in each pour mode; all 18 DSNs byte-identical. Original source hashes unchanged. The rest remained unsupported.
- Real product binary by stdio: installation path and build identity checked; `route_specctra_dsn` called with no preset overrides. Three product-MCP and three CLI runs used the same fresh product DSN/manifest and S5 test-intent inputs. All six completed, all six had zero KiCad unconnected after refill, and all six passed the signal-length acceptance band. All four total/category median comparisons passed the declared copper-parity band; full spreads are retained privately. Every product run reported all 12 sent settings in effect. Captured actual router logs, not just scalar settings, agreed on both layers' costs and the optimizer thread count, and showed telemetry disabled and the closed-proxy version check. Known thermal and width gate failures remain visible in the private score records.
- Matrix/threshold choice: three serial runs per transport provide a median and spread without changing the measured preset. The acceptance bands are those under the prior "Now": zero unconnected per run, signals at most x1.03, and median copper agreement within 1 percent for total and each category. CLI thread settings match the product's configured defaults on this Mac; actual autorouting is single-threaded. No heavy build ran during routing. Both transports used the existing scoring pipeline on scratch boards with iterative refill disabled, then KiCad DRC.
- Independent diff review in another worktree found no actionable introduced defect; a second static settings pass agreed. Reviewed patch SHA-256 and the review text are retained. Nine negative controls left the reviewed patch byte-identical.
- `graphify update .` completed (AST-only): 14,819 nodes, 45,896 edges; diagnostic found zero dangling endpoints, self-loops or duplicate edges. Six non-code/config inputs yielded no AST nodes; documentation semantics were not re-extracted.
- Installed binary SHA-256 remains `3de188bb775ab4ce5748ea44b94cb098c86ed5d32135e70f4c2c3817c3a4ac65`; final-gate protected-file hashes also check the owner's KiCad/Freerouting settings.

**Not verified / unfinished.** Live `apply_specctra_ses` into the KiCad GUI; actual owner current values or pour capacity; another private reference board; other Freerouting versions; Windows and Linux. S6's second half remains unstarted. The known S5 thermal-relief and power-width failures are not fixed by this change. The owner subsequently selected S7 and independent parallel creation work, recorded above; that later authorization does not change S6's runtime or acceptance evidence.

## Row S5, routing measurement on the reference board (merged, #60)

Merged as [#60](https://github.com/yuvaltal912-sys/Konnect/pull/60) from branch `claude/creation-s5-routing-measure-46323d`, from main `c9ba0ce`; the merge commit is `5449cea`, and the first commit only claimed the row. No product code: everything is in `xtask/` and docs. Usage is in `DEV.md`, "Freerouting benchmark (opt-in)". The raw outputs, the decisions and a running log are local and git-ignored, in that worktree's `target/creation-research/s5/` (`DECISIONS.md`, `PROGRESS.md`, `RESULTS.md`, `matrix/`). A complete copy is in the main checkout, at `~/ClaudeCode/Konnect/target/creation-research/s5/`; read that one, because the worktree may be removed with its session.

**Built.** `cargo xtask freerouting-bench` now ranks router settings and DSN variants, not only JAR versions:

- `--config NAME=ARGS` (repeatable): a named set of router settings, routed and ranked on its own. Only `--router.<path>=<value>` and the router's short flags are let through, each setting once, so a config cannot turn telemetry on or move a run's files. A threshold that the command line would run as a hundred times what it says is refused.
- `--score <spec.json>`: every run, and the board's existing routing as the baseline row, goes through one pipeline on scratch copies: the session into a copy of the project (`score_board.py`, KiCad's Python), refill and DRC by `kicad-cli` under a scratch `KICAD_CONFIG_HOME` with iterative refill off, `audit_layout_quality` through this checkout's own `konnect` by pipe, then track length by net category (pour nets, power nets, signals) and the share of each power net below its intent width.
- Three gates per run (G1 completion, G2 no DRC type above the baseline's count, G3 widths) and the quality bands of the owner's plan, all thresholds named in the spec. Candidates (a config on a DSN, quoted at its worst run, with its spread over its runs beside it) rank by the gates, then acute corners, signal length, power length, right angles, vias, copper quality, runtime.
- Each run's inputs are on record before it starts (`inputs.json`: the JAR's and DSN's SHA-256, every argument, the bench's commit, the load), and its effective settings are read from Freerouting's own log afterwards (the applied settings and the routing parameters); a setting that was not applied as given fails the run, and so does a run that was not offline.
- `--max-jobs` with a `jobs.log` ledger, `--resume` (refused for a config or a DSN that is not what its saved runs used; what a stopped or replaced run left is kept beside the new one), and `--rescore` from the saved score files without running anything.
- `stage_merge.py` (experiment only) edits an exported DSN: a net out of its widened class, a pour dropped, and a first run's session merged in as fixed wiring for staged routing.

**The sweep (2026-11-10, owner's go-ahead to run it through).** 116 Freerouting 2.5.0 jobs, one at a time, each in its own home with telemetry off and the network closed: 109 in the final results (35 cells) and 7 in a first attempt that was set aside whole when its second cell showed a mis-scaled threshold. Of the 109: none failed to run, none was not offline, every given router setting was applied as given, all were scored on the board, 2 were stopped by the 10 minute job budget with a complete board. 100 are routings of the whole board; 6 are the first stage of the two staged variants (the power nets only, by design) and 3 routed nothing (the last line under the variants). Currents: the six test values worked back from the existing routing's widths, not the owner's own figures. Thresholds: the plan's. Three or more runs per cell, five for the two reference cells.

All ratios below are against the existing routing, over the nets that have track on both boards. The existing routing: 6 unconnected, 152 vias off the pour net and 53 on it, 0 acute and 3 right-angle corners, copper quality 61.3.

- **Gates over the 109 runs:** G1 passes in 70, G2 in 21, G3 in none. No candidate passes all three, so the bench itself chooses none.
  - G3 fails everywhere. In each of the 106 runs that routed anything, at least one power net is below its intent width (worst share of one net's length: 10 % to 91 %): Freerouting has one retry width for the whole board, and it takes over whole connections. The 3 runs that routed nothing fail it because the audit had no track to measure.
  - G2 fails in 88 runs. 82 of them have `starved_thermal` errors, 1 to 4 a run, where the existing routing has none: a pad whose thermal relief reaches its pour by fewer spokes than the zone asks. Of those 120 errors, 60 are one surface-mount pad, which is starved before anything is routed (the runs that routed nothing have it); the other 60 are on 8 through-hole pads. The three runs of the fanout-on cell also have 6, 6 and 8 `track_width` errors. The other 6 runs have no starved thermal and fail by one `track_dangling` each: via cost 25 with Freerouting's own direction cost, at 20 and at 100 passes, 3 runs each.
  - G1 fails in 39 runs: 12 where the router gave up on 1 or 2 connections (four settings cells), 12 with pours as planes, the 6 second stages of the staged variants, and the 9 that are not a routing of the whole board.
- **Settings, on the DSN the product exports today (20 cells, 64 runs).** Signals x1.00 to x1.14, power x1.08 to x1.25, track on the pour net x7.2 to x9.2 in every cell: no setting stops ground being routed as track.

  | Cell | Runs | G1 / G2 / G3 | Signals | Power | All track | Vias off the pour net | Acute | Right angles | Copper quality |
  |---|---|---|---|---|---|---|---|---|---|
  | What the product sends today | 3 | 3 / 0 / 0 | x1.08 | x1.20 | x1.28 | 98 | 9 | 78 | 56.3 |
  | The settings of the 2026-11-08 runs | 5 | 5 / 5 / 0 | x1.11 | x1.20 | x1.32 | 106 | 13 | 81 | 60.1 |
  | Via cost 25 (at those settings) | 3 | 3 / 0 / 0 | x1.09 | x1.12 | x1.25 | 120 | 5 | 77 | 58.7 to 58.8 |
  | **Via cost 10 and Freerouting's own direction cost** | 3 | 3 / 0 / 0 | x1.00 to x1.01 | x1.09 to x1.11 | x1.18 | 114 to 118 | 3 to 5 | 70 to 74 | 57.9 to 59.2 |
  | Today's product with via cost 10 | 3 | 3 / 0 / 0 | x1.00 to x1.02 | x1.08 to x1.12 | x1.18 to x1.19 | 133 to 159 | 4 to 6 | 74 to 77 | 57.1 to 58.4 |

  - Two levers move the lengths: a lower via cost, and not raising the against-direction trace cost (the 2026-11-08 runs had inherited a raised one). Together they bring the signals to the existing routing's length. Via cost 5 is no shorter than 10 (signals x1.01 to x1.02) and worse on vias (148 to 152), corners (10 to 13 acute) and copper quality (55.2 to 55.5), though its worst share of a power net below width is lower (38 %). Via cost 25 completes at the raised direction cost of the 2026-11-08 runs (3 of 3) and leaves 1 connection open in every run with Freerouting's own direction cost (6 of 6); via cost 100, at the raised cost, leaves 2 in every run.
  - The other levers of the plan change nothing or harm: the optimizer's early stop, its threshold and the bend cost leave the routing as it was; pull-tight accuracy 100 leaves a connection unrouted; 100 passes with a higher rip-up cost is longer on every length; the optimizer strategy flags cannot be confirmed and show no effect; fanout on necks tracks below the class width.
- **DSN variants, with the best settings** (via cost 10, Freerouting's own direction cost):

  | Variant | Runs | G1 / G2 / G3 | Signals | Power | All track | Track on the pour net | Open after refill | Copper quality | Worst share of a power net below width |
  |---|---|---|---|---|---|---|---|---|---|
  | Today's export (ground widened) | 3 | 3 / 0 / 0 | x1.00 to x1.01 | x1.09 to x1.11 | x1.18 | x7.3 to x7.4 | 0 | 57.9 to 59.2 | 77 to 79 % |
  | **Ground not widened** | 6 | 6 / 0 / 0 | x1.00 to x1.01 | x1.07 | x1.17 | x7.1 to x7.2 | 0 | 61.0 to 63.0 | 10 to 14 % |
  | Bottom pour and region pour as planes | 3 | 0 / 0 / 0 | x1.01 to x1.02 | x1.02 to x1.03 | x0.99 to x1.00 | x0.1 | 17 to 20 | 53.6 to 55.7 | 16 to 81 % |
  | Staged (power first, fixed), ground not widened | 3 | 0 / 0 / 0 | x1.01 | x1.02 | x1.16 | x7.1 | 3 | 60.6 | 23 % |
  | Staged, pours as planes | 3 | 0 / 0 / 0 | x1.03 | x1.04 | x1.01 | x0.1 | 16 | 56.8 | 45 % |

  - The variant with the bottom layer's trace costs raised was not measured: Freerouting read the edited DSN as an empty board (see "Known traps"). Its 3 runs exited 0 with nothing routed and 143 connections unconnected, and fail every gate; they are among the 109.
- **Spread.** Freerouting's optimizer is not repeatable (see "Known traps"). Over 16 runs of one process the signal ratio spans 0.019 and the power ratio 0.049, so the ranking compares length ratios in steps of 0.02, and a power difference under 0.05 between two cells is not a difference. The chosen cell's six runs are six different routings within 0.003 on signals and 0.001 on power.

**Preset chosen: via cost 10 with Freerouting's own direction cost, on a DSN whose pour net is not widened.**

- Settings: `scoring.via_costs` 10; the against-direction trace cost at Freerouting's own value (the cell sent `scoring.default_undesired_direction_trace_cost` 1.0; a run that sends none gets the same layer costs); the optimizer's early stop off (`optimizer.max_consecutive_failures_pass1` and `optimizer.max_consecutive_failures` at 1000000) with an improvement threshold of 0.25 % (`0.0025` on the command line); 20 autorouter passes; and what `route_specctra_dsn` already sends (fanout and neck-down off, the retry width and the edge clearance from the project). DSN: pours left out, power nets widened from the design intent, the net that owns the pours left at its class width.
- Measured over 6 runs: complete in all (0 unconnected, against 6 on the existing routing); signals x1.003 to x1.006 (inside the x1.03 band); power x1.066 to x1.067 (band x1.05: outside); 97 to 106 vias off the pour net (existing 152); 4 to 5 acute corners (existing 0) and 60 to 65 right angles (existing 3); copper quality 61.0 to 63.0 (existing 61.3); decoupling loops, reference-plane coverage and stitching at layer changes all at least as good as the existing routing's; 88 to 173 s. It fails G2 by 1 or 2 starved thermals in every run and G3 by 10 to 14 % of one power net below its width.
- In the plan's order: no candidate passes the gates; on this DSN the four settings tried fail the same two gates, and this one then leads on acute corners and signal length.
- Not chosen: the settings of 2026-11-08 on today's export, which the bench's own list puts first because they fail only G3. Their clean DRC is luck (the same process has a starved thermal in 4 of 16 runs, and in every run once the ground is not widened), and they are 10 % longer on signals.
- Second: today's product settings with only the via cost set to 10, on the same DSN. The same lengths in 31 to 33 s and nearly the same routing every time (2 distinct in 6 runs), copper quality 62.9, with one more acute corner and two starved thermals in every run. It is the choice if runtime or repeatability should count for more.

**What it means for the next rows.**

- **S6.** Not widening the net that owns the pours is supported: with it the routing is complete, every quality measure improves, and the power nets hold their width far more often (worst share below width 10 to 14 %, from 77 to 79 %). The measured settings belong with it: sending a via cost of 10 and no against-direction cost is what closes the signal gap, on either DSN. Ground is then still thin track (x7.1 to x7.2 of the existing routing's ground track), beside the pours. Pours as planes remove that track and bring all track to the existing routing's length, but leave 17 to 29 ground connections open in every run and settings tried, more than the "about 10" at which the plan says to fall back to thin ground track; planes need S7's stitching first. S6's acceptance line as written (ground track near the existing routing's, at least 40 ground vias) is reachable only that way: the plane runs have 5 vias on the pour net, the existing routing 53.
- **S7.** What no setting or variant removes: starved thermals (82 of 109 runs), right-angle corners (49 to 84 against 3), acute corners (1 to 16 against 0), and the open ground connections of the plane variants.
- **S9.** Routing the power classes first and fixing them did not do what the plan hoped: the rest could no longer be completed (3 connections open with ground not widened, 16 with planes), and the widths were no better than routing everything at once on the same DSN (one net 23 % below its width, against 10 to 14 %). Even alone on an empty board, one or two power nets fall back to the retry width for a whole connection. The limit is the single retry width, not the order.

**Verified** (2026-11-10, macOS, KiCad 10.0.8, Freerouting 2.5.0 with the published digest, Java 25).

- 87 xtask unit tests without Java, KiCad or Konnect (46 before), among them config parsing, the settings evidence over the lines of a real 2.5.0 log, the run-inputs record, the DRC-baseline diff, the category split, the width share and its stretches, the gates, the bands, the ranking and its spread, the report and its JSON, the resume rules, and `--rescore` over saved score files. 52 negative controls: each guard broken in the source, its test failed, the file restored.
- A rehearsal of the run loop with a stand-in for Java that replays a recorded session (Freerouting never starts): scoring inside the loop, the settings evidence, the job cap across invocations, a killed bench resumed (the process it left is stopped first), and `--rescore` leaving `results.json` byte for byte.
- An independent review of the diff found nine things, all addressed: the settings evidence read a file that holds no router settings on a real Freerouting (it now reads the log), a resumed sweep could merge runs made with other settings or another DSN, and seven smaller ones.
- Without routing: the pipeline over the seven sessions recorded on 2026-11-08 gives the counts recorded then (the #58 result: 0 unconnected, 125 vias, 11 acute and 76 right-angle corners, track x1.31 overall).
- On a real Freerouting: the first job reproduced the recorded #58 result (total track within 0.4 %, the same gate outcome); its log showed every given setting applied, "Analytics are disabled", and the version check refused by the closed proxy. The evidence check caught a mis-scaled threshold on its first run.
- The gate, on `a717879` (only this document has changed since): fmt; 4,194 lib and integration tests passed, 0 failed, 163 ignored; 9 doc tests passed, 0 failed; clippy `-D warnings`; `cargo deny check licenses`; `fix-doc-counts`, no change; release build; `simulation_live` skipped, not touched. One earlier run had the known IPC-mock flake (9 `pcb_sync` tests and 1 `custom_rule` test timed out); they passed alone and in every other full run. Installed binary SHA-256 still `3de188bb775ab4ce`.

**Not verified.** The owner's real currents (the widths, and so G3, are judged against test values). A second board. The variant with a per-layer trace cost. The optimizer strategy flags (they leave no line to check). Whether the preset's settings behave the same through Freerouting's MCP server, which is how the product routes (the sweep used the command line; subsequently checked in S6's first half above). Windows and Linux.

## PR 16b, route at the project's rules (merged, #58)

Merged as [#58](https://github.com/yuvaltal912-sys/Konnect/pull/58) from `feat/creation-route-at-project-rules`. The claim-only commit and draft preceded implementation, from main 535bf2e. Found by routing the owner's reference board (100 parts, 2 layers, the class of board he builds) with Freerouting 2.5.0; its private copy and scripts are in git-ignored `target/creation-research/reference-boards/`.

- **Neck-down off:** Freerouting's automatic neck-down put 0.15 mm tracks into a 0.2 mm class. `route_specctra_dsn` sends `automatic_neckdown: false` (`allow_neckdown` to override) and the project's `copper_to_edge_clearance_um` from the manifest beside the DSN.
- **Importer enforces class widths:** an SES with a wire narrower than its net's class is refused (it used to check only custom-rule minima). Class widths travel in the manifest's `classes`.
- **Power widths from intent:** `widths_from_intent` (default on) widens each net with a design-intent current to its IPC-2221 width on the worst signal layer (`widen_for_intent`, reusing `audit_trace_current`'s formula and copper sources). The owner wants to specify currents on the main traces; the record is where they go.
- **Wide nets and small pads:** one width per net made a 1.5 mm GND unable to reach small pads (31 unconnected). Freerouting's automatic neck-down would reach them but below the project minimum; its retry width (`neck_width_um`) instead routes such a connection whole at a given width. The manifest records each widened net's base width as its floor (`min_track_width_um`), and the importer holds every net to its floor. `route_specctra_dsn` passes the retry width only when every net's floor is the same (see the review follow-up below): Freerouting applies it to any failed connection of any net.

- **Settings Freerouting really reads (follow-up, 2026-11-09):** the job settings had camelCase names (`maxPasses`, `jobTimeoutString` since before this PR; `neckWidthUm`, `copperToEdgeClearanceUm` from it), which Freerouting 2.5.0 drops without an error. Only neck-down off arrived, through a declared alternate name. A `freerouting.json` in the owner's home hid it. `job_settings` now sends `autorouter.max_passes`, `job_timeout`, `neck_width_um`, `copper_to_edge_clearance_um`, `automatic_neckdown`, `fanout.enabled` and `optimizer.enabled`; `route_specctra_dsn` reads them back with `get_effective_settings` and refuses the run, routing nothing, when any is not in effect. Fanout is always off: in 2.5.0 it necks tracks at pins whatever `automatic_neckdown` says. The result reports `effective_settings` and the optimizer's outcome (`optimizer`, from `get_job_logs`, with `stop_reason` when the log gives one).
- **Hole clearance:** unplated holes are image keepouts with no rule, so the router kept only the 0.2 mm copper clearance from them while the project asks 0.25 mm (one run put a track 0.21 mm from a connector peg). Each such keepout now grows by `min_hole_clearance` less the smallest class clearance (`supported_profile.hole_keepout_growth_um`). Freerouting's own `hole_clearance_um` covers only circular keepouts, not slots, so it is not used.
- **Fail closed:** a footprint's own rule area or zone, or its Edge.Cuts graphics (a slot or cut-out), was silently left out of the DSN, so the router could cross an antenna keepout or a slot. Such a board is now refused, naming the footprint, until rank 32 models them.
- **Review follow-up (2026-11-09), six findings fixed:**
  - Pad-less footprints: the refusal and the copper keepouts now run for every footprint, before the pad-less skip, so an antenna-keepout or slot footprint with no pads is refused (named by Reference, else UUID) and a copper logo is a keepout. A pad-less footprint is not placed; its copper graphics and text are written at board level, turned and moved with the part. `footprints_without_pads` still counts every footprint with no pads.
  - One global retry width: Freerouting 2.5.0 retries any failed connection at `neck_width_um`, narrowing each layer's trace to it whatever the net. It is now sent only when every net's floor is the same; otherwise the retry is off: `neck_width_um` 0 is sent (so a stored setting cannot turn it back on), `rules_applied.retry_width_um` is `null` and `retry_off_reason` says why. A wide net that cannot reach a small pad is then reported unrouted, not narrowed. The importer names the single retry width when a refused wire is exactly the narrowest floor on the board.
  - Edge clearance of exactly 0.25 mm: 2.5.0 reads 250 um as its own default (see "Known traps"), so it is sent as 250.001 um and the read-back is compared with that.
  - A net with a design-intent current but no track width is no longer widened from zero (which let it through as a 0 width); it stays incomplete and is refused.
  - The optimizer's `stop_reason` also reads the stage timeout, the item limit and a stage that completed with timeout.

Acceptance on the reference board (counts only; pours omitted, currents back-calculated from the existing routing's widths for the test): KiCad DRC after applying the SES with `pcbnew` and refilling finds 0 unconnected (the existing routing: 6), 125 vias (existing: 152 by the layout audit's count, which leaves out the ground net's 53), and no track below 0.2 mm. The power widths are only half met: two of the five widened power nets run about half their length at the 0.2 mm retry width, because Freerouting's retry width is one global value that narrows the whole connection. Open: total track length is about 30% above the existing routing, most of it the ground net routed as wide track (see "Recommended next"). The refill that ran the GND pour up to two through-hole pads of one connector in one run is a KiCad 10.0.8 defect (see "Known traps"): turning iterative refill off removed all four violations in 3 of 3 runs, and the clean run had identical copper at those pads.

Verified on macOS (2026-11-08): the gate (fmt; 3779 lib and integration tests, 0 failures; doc tests; clippy `-D warnings`; `cargo deny check`; `fix-doc-counts`; release build); new tests for the job settings, the neck-down default, the importer's width floor (class and widened-net base), intent widening (never narrowed, unusable record refuses, power layers ignored) and the manifest's routing hints; 4 negative controls (neck-down default, width check, base-width floor, intent widening), each caught.

Follow-up verified on macOS (2026-11-09): the gate (fmt; 3796 lib and integration tests passed, and one `konnect-ipc` handshake-timing test, which this PR does not touch, failed once and passed 5 of 5 alone; doc tests; clippy `-D warnings`; `cargo deny check licenses`; `fix-doc-counts`, no change; release build; installed binary SHA-256 still `3de188bb775ab4ce`); new tests for the snake_case names, the read-back refusal (a dropped value, a missing key, fanout left on), the optimizer report (2.5.0's own log lines, a threshold stop, a skipped stage), a stand-in Freerouting MCP server (a setting not applied routes nothing and writes no SES; applied settings and the optimizer reach the result), fanout and neck-down off by default, hole-keepout growth, and the footprint zone and Edge.Cuts refusals; 8 negative controls (camelCase name back, read-back off, hole growth off, footprint zone accepted, footprint Edge.Cuts accepted, fanout not sent by `route_specctra_dsn`, fanout dropped from the job settings, stop reason not parsed), each caught. Live, on a scratch copy of the reference board with #57 merged in (isolated home, telemetry off): the real `route_specctra_dsn` handler routed through Freerouting 2.5.0's MCP server in 38 s; `effective_settings` showed every sent value in effect; the owner's real-home `freerouting.json` was unchanged (same hash and time). The first live run, before fanout was turned off, left 4 wires at 0.15 mm, and the strict importer refused that SES. With fanout off it accepted the SES (889 tracks, 62 vias). Applied with `pcbnew` and refilled with iterative refill off, KiCad DRC found 0 clearance, 0 hole-clearance and 0 track-width violations, 4 starved thermals, and 28 unconnected items: 1 signal connection the router could not finish (so the optimizer was skipped), and 27 on the ground net, where the pours were kept (the default) and Freerouting relied on them, but KiCad's refill split them into islands around the new tracks. Ground through the pour is the next item (see "Recommended next"). Not verified: a live KiCad, Windows and Linux.

Review follow-up verified on macOS (2026-11-09): fmt; 3797 lib and integration tests, 0 failures (an earlier run had the known `konnect-ipc` handshake-timing flake, which this change does not touch); 9 doc tests; clippy `-D warnings`; `cargo deny check licenses`; `fix-doc-counts`, no change; installed binary SHA-256 still `3de188bb775ab4ce`. New tests: pad-less footprint with a zone, with Edge.Cuts, named by Reference and by UUID; pad-less copper keepout in board space; retry width off for different floors and on for a widened net; the importer's retry-width message; no widening without a track width; the 250 um edge clearance nudge and its read-back; `neck_width_um` 0 sent when the retry is off; the optimizer's timeout and item-limit stop reasons. 8 negative controls (pad-less refusal, pad-less keepout, retry off, retry width 0 not sent, importer message, edge nudge, widening from zero, timeout stop reason), each caught. Not run: Freerouting, a live KiCad, the release build, `simulation_live`.

## Layout quality audit (merged, #56)

`audit_layout_quality(board, …)`: the measuring stick for "optimal" routing, in the `design_review` toolset (8 tools and 260 + 7 meta at the time; 9 on main). Read-only, from the saved board file (zone fills are read, so refill and save first). Code in `tools/layout_quality.rs` (tool, thresholds, net roles, scoring) with `_grid` (a raster of one net's copper: connectivity, area, shortest and widest paths), `_planes` (§3), `_power` (§4, §8) and `_routing` (§5, completion); tests in `layout_quality_tests.rs` over six boards `pcbnew` wrote (`tests/fixtures/layout_quality/`, `make_fixtures.py` regenerates them with KiCad 10.0.8's own Python).

**Measured**, each as a finding that names the checklist item, the measured value, the threshold and UUIDs:

| Item | Measure |
|---|---|
| 3.1 | Critical track length over its reference plane (the saved ground or power fill on the adjacent layer); stretches over voids, slots and other-net copper (splits) |
| 3.2 | Signal vias that change reference plane, and the nearest ground via spanning both planes (or a capacitor between two different planes) |
| 3.3 | Ground pad groups (cut-off pads, from the real shapes) and floating islands (copper that touches no pad) |
| 3.4 / 4.3 | Narrowest section on the best copper path from each rail's source to its loads, and on the ground return, against the IPC-2221 width for the recorded current |
| 4.1 | Per IC supply pin: capacitor → pin → ground → capacitor copper path length and plan-view area, ground side included. A pad typed `input` or `no_connect` (KiCad's `pintype`) is tied to the rail for its logic level and is not a supply pin; the loop closes at a ground pad of another type when the IC has one (Row AP) |
| 4.2 | `audit_trace_current` reused as is; its findings are re-emitted under 4.2 |
| 5.1 | Routed length over the pin-to-pin spanning tree (HPWL also reported; a tree under 1 mm is judged against 1 mm), vias per connection on signal nets, dangling track ends; nets carried by a pour have no ratio, and ground, power and pour nets no via ratio; those nets are named |
| 5.2 | Same-layer parallel runs of unrelated signals (tighter around critical nets and switch nodes) |
| 5.3 | Acute corners (PR 11's acid-trap check, reused) and 90° bends |
| 5.4 | Differential pairs: coupled fraction, gap spread, via count, reference plane, dangling ends; length mismatch as information |
| 8.1 | A buck's input loop or boost's output loop, IC-integrated or discrete (topology from the record's rail `source_ref`, else the `switching_regulators` argument, else `needs_human`) |
| 8.2 | Switch-node copper area and the clearance of feedback routes from it (pins named FB/VFB/ADJ/SENSE/COMP, or `feedback_nets`) |

**`needs_human`** (listed in the result): impedance from the stackup, rise-time and termination questions, capacitor DC-bias and ESR, intentional cuts, isolation and antenna keepouts, gate-drive, Kelvin and recirculation paths, length matching, and everything in §11.

**Roles.** Ground, power and critical nets come from the design-intent record (rails at 0 V are ground; interfaces and differential pairs are critical), netclasses (every assigned class and every matching pattern joins, as in KiCad's effective netclass) and the arguments, then net names; `net_roles` gives the source of each. With nothing recorded, every signal net over 5 mm is judged and `critical_basis` says `fallback_all_signals`.

**Score.** Every check scores 0..1 against its stated threshold (the loop checks full marks up to half the limit, 0.5 at the limit, 0 at twice it); categories are weighted means (return path 0.25, decoupling and current 0.25, switching loops 0.15, routing 0.35); `quality.copper_quality` is the weighted mean of the categories that measured something, `quality.weight_measured` says how much of the weight that is, and `quality.score` is `copper_quality` times the share of multi-pad nets that are routed, so a board with nets left open cannot out-score one that routed them. `metrics` holds the flat numbers a loop compares (`3.1.reference_coverage`, `4.1.worst_loop_mm`, `5.1.wire_to_hpwl`, …). Thresholds are arguments (defaults in `thresholds`, with `source`); compare runs only with the same thresholds, net roles and completion.

**Limits, said in the result.** Raster lengths and widths are good to about one cell (0.1 mm, coarser on very large boards); loop areas are plan-view, not 3-D inductance; a neck-down search judges the best single path and does not sum parallel copper; broadside coupling between layers is not measured; a net with a zone that is not saved filled is not judged by 3.1, 3.3 or the completion gate (each says so, and the gate leaves it out of `routed_share`); a topology the record cannot decide is not guessed; a power-role net behind an inductor is a filter, never a switch node, unless `switching_regulators` names it.

**Verified on macOS (2026-11-08).** The gate (fmt; 3796 lib and integration tests, 0 failures; doc tests; clippy `-D warnings`; `cargo deny check licenses`; `fix-doc-counts`; release build; installed binary SHA-256 still `3de188bb775ab4ce`; `simulation_live` skipped, not touched). A stdio run of the release binary through real dispatch (`load_toolset` design_review, then the tool, then an unknown argument refused). 31 unit tests over the six fixtures: a track over a plane split and a slot (64% over ground, 14 mm over the 3V3 region, the slot found), a missing stitching via (2 of 4 layer changes covered), a 3.8 mm against a 44 mm decoupling loop with the ground leg 23 mm of the excess, a 0.2 mm neck on a 3 A rail found at x 25–28 within one raster step, a buck whose far input capacitor makes a 31 mm hot loop against 5.5 mm, a feedback route 0.4 mm from its switch node, detours, stubs, parallel runs, acute and right-angle corners, a drifting differential pair, ground cut off from its pads, zones without a fill, nets left unrouted, refusals, and the raster itself. Negative controls (break, see a test fail, restore): any fill counted as ground, stitching always satisfied, the ground side left out of the loop, completion not weighed, the acid-trap check not reused, dangling ends never found. Cross-check against KiCad's own DRC on a real 2-layer project (5 states of one board): the audit's open ground nets ordered the four routed states as KiCad's unconnected-item counts did (0, 6, 9, 20), and the unrouted copy scores 0. Run over the 16 KiCad demo boards: 15 audited in 0.0–7.8 s each, one demo file (`RoyalBlue54L-Feather`) yields no copper-layer table to the parser and is reported `not_run`; stripping one demo's routing (61 nets open) moves its score from 64 to 15.

**Review follow-up (2026-11-09).** A two-vote review of the draft found twelve things; eleven are fixed and one was refuted by both votes. Each fix has a test that fails without it.

- A power-role net behind an inductor is a filter, whatever gave it the role, unless `switching_regulators` names it (a buck feeding an MCU used to turn `+3V3` into a switch node and drop its supply pins from 4.1). 4.1 counts the pins left to 8.1 and 8.2.
- 5.1 counts vias per connection on signal nets only; ground, power and pour nets carry stitching vias. A tree under 1 mm is judged against 1 mm. Nets with no ratio are named in the metrics (a bounded list plus the count), and a net with neither ratio is left out of the score.
- 5.2 files each segment over its box grown by the farthest a partner can be, so a pair across a 3 mm cell line is compared.
- 3.3: a via alone no longer anchors copper; a floating island is copper that touches no pad. Ground pad groups, and the completion gate, now come from the real shapes (touching within 0.01 mm on a shared layer), not the grown raster, so a track 0.15 mm short of its pad is open.
- 4.2 counts nets, not findings.
- 3.1 and 3.2: the reference plane on a layer is the ground or power net with most samples under the net. A solid power plane is a reference, another net's copper a split. Result fields renamed: `over_reference_mm`, `runs_off_reference`, `length_off_reference_mm`, `longest_run_off_reference_mm`; `reference_kind` says ground or power.
- A net with a zone that has no saved fill: 3.3 is `not_run` or `partial` with the reason, and the gate lists the net under `not_judged_listed`, leaves it out of `routed_share`, and sets `complete` to `null` (never "unrouted").
- 4.3 and 3.4: loads are spread from the nearest to the farthest instead of the nearest 12, the load limit is shared across nets, and loads or paths left out (per-net limit, the 80-path cap, ground returns with no ground pad) are named; the check is `partial` with the count, and a `not_run` reason keeps them too.
- Netclasses: every assigned class and every matching pattern joins, as in KiCad 10's `NET_SETTINGS::GetEffectiveNetClass` (an explicit assignment neither overrides nor yields to a pattern). A net is power if any of its classes is.
- 3.2 `worst_nearest_mm` is `null` when no stitching candidate exists (counted apart), not 1000000. UUIDs are now filled on acute corners, cut-off ground pads and the pair-length note.

**Verified after the follow-up (2026-11-09, macOS).** fmt; 3823 lib and integration tests, 0 failures; 9 doc tests; clippy `-D warnings`; `cargo deny check licenses`; `fix-doc-counts` left no diff; installed binary SHA-256 still `3de188bb775ab4ce`. 51 layout-quality tests, 20 of them new; they run on boards the test writes as text (`Bd` in `layout_quality_tests.rs`), since the six `pcbnew` fixtures could not isolate these cases. 27 negative controls (break, see the test fail, restore, compare the file): every one failed its test. Run over the demo boards: 19 files, 1 `not_run` (no copper layer table) and 1 with nothing to score. Whole-audit time on the demo boards in a debug build: one board about 4x slower (about 9 s to 34 s) from grouping pads by real shapes, the slowest board unchanged within noise.

**Not verified.** A live KiCad (the audit reads the file); a zone filled on only some of its layers (it counts as filled); a board with recorded currents and rails other than the fixtures; `simulation_live` (not touched); Windows and Linux. No datasheet, simulation or bench input is used, by design.

**Follow-ups.** Shipping skill guidance (`kicad-review`, `kicad-pcb`) that says when to call it; a routing loop that runs it on each Freerouting result and keeps the best; IPC-2152 derating and pour capacity in 4.2; a 3-D loop inductance estimate from the stackup; hatched-fill and keepout-aware reference checks.

## Rank 32 part 2, Freerouting version policy and benchmark (merged, #51)

B34 and D24, from main 966a89d, merged with main after #44 to #47. Files: `xtask/src/freerouting_bench/` (new), `xtask/freerouting-bench/` (corpus script, quiet Java wrapper), `freerouting_setup.rs`, the `check_freerouting` handler in `integration.rs`. Usage is in `DEV.md`, "Freerouting benchmark (opt-in)".

- **`cargo xtask freerouting-bench`**, opt-in, never in CI: any folder of DSN files, one or more JARs, two runs each. Completion is a gate (0 unrouted, 0 clearance violations, within Freerouting's own job budget); boards that pass get quality: wiring over an any-angle lower bound (half the pins' convex hull) and over HPWL, vias and vias per net, layer changes, acute and 90° corners away from pads, tracks, segments below their net class width, runtime, determinism (same copper in any order). Ranking: gate failures, then defects, wire/bound, vias, determinism, runtime, over the boards every competing version passed. Each run has its own `user.home` and temp folder with telemetry off. `--rescore` re-measures a finished run without Java. Results stay under `target/freerouting-bench/`.
- **`check_freerouting`** reports `version_policy`: recommended 2.5.0 (needs Java 25), its URL, SHA-256, the measured reason, and a warning when the installed JAR is another version or not the published file. Its `--version` probe now runs headless with telemetry off in a scratch home: 2.1.0 used to open its window and time out, so a working install read unavailable.
- **Measured (2026-11-08, Java 25, 120 s budget, all four releases):** on 10 boards exported by Konnect with pours omitted (5 KiCad demos of 15 to 114 parts, 5 fixtures), 2.5.0 passed the gate on 10, 2.1.0 on 7, 2.3.0 and 2.4.1 on 5 each. 2.5.0's two runs matched on all 10, 2.1.0's on 2 of 7. On the 5 boards all passed, 2.5.0 and 2.4.1 tied for shortest wiring (1.195 x the bound) and fewest vias. A second corpus from KiCad's own exporter (pours kept) also ranked 2.5.0 first, with no release completing 4 of its 12 boards. Caveat: 2.5.0 necks traces down near pads (202 segments below class width on 4 boards; 2.1.0 had none, but 13 acute corners).

Verified on macOS: the gate after merging main (fmt; 3,813 lib and integration tests, one of them first red on this PR's own skill wording and green after the fix; doc tests; clippy `-D warnings`; `cargo deny check licenses`; `fix-doc-counts` unchanged; release build; `simulation_live` skipped, not touched), a stdio run of the release binary (`check_freerouting` on the 2.1.0 KiCad plugin JAR: available, warned with the measured reason; on 2.5.0: matches, digest verified, no warning), 38 xtask unit tests without Java (DSN and SES readers, pin placement incl. back side, hull bound, corners, layer changes, widths, gate, ranking, report and JSON), version-policy tests over fake JARs and Java scripts, the ignored check of the pinned digest and Java 25 against the real 2.5.0 JAR, and the real benchmark runs above. Not verified: the owner's own boards (the corpus still comes from KiCad's demos), the boards' real net-class widths in the Konnect-exported corpus (PR 16's probe writes a flat 0.2 mm), Windows and Linux.

Follow-ups: the owner's board corpus; a setting sweep (passes, optimizer) on it with the same metrics (row S5). Starting Freerouting quietly in `route_specctra_dsn` and the MCP probe is done (#57).

**Review follow-up (2026-11-09).** Seven verified findings fixed, each with a test that fails without it:
- The `--version` probe fails closed: no scratch folder means the JAR is not started (`available` and `version_checked` false, `note` names the error). It also removes inherited `FREEROUTING__*` variables and pins `XDG_*`, `APPDATA` and `LOCALAPPDATA` into the scratch folder. `probe_local` in `freerouting_mcp.rs` is untouched here; its isolation comes with #57.
- A version line followed by a Java exception trace or a Freerouting ERROR line (not the expected "no input/output file" complaint of a board-less headless run) is a start-up crash, not `available`.
- The bench trusts 2.3+'s own `finished with state:` line over the wall clock for the job budget; the wall clock stays the test for 2.1.
- Each `--jar` gets a unique folder label (`-2`, `-3` on a clash, `unknown` too), saved as `dir` in `results.json`; `--rescore` reads it (old files fall back to the file-name label), and totals and ranking are per folder with the version shown.
- A run that does not report its clearance-violation count fails the gate.
- Versions compare by zero-padded numbers (`2.5` is `2.5.0`); `-SNAPSHOT` and other pre-release suffixes are kept and never the release.
- The bench seeds `home/.config/freerouting` (where 2.5 reads on Linux) and pins the `XDG_*`, `APPDATA`, `LOCALAPPDATA` folders into the run folder.

Verified (review follow-up): fmt; 3,838 lib and integration tests passed, 0 failed (the 46 xtask tests included); 9 doc tests; clippy `-D warnings`; `cargo deny check licenses`; `fix-doc-counts` unchanged; 19 negative controls, each guard broken in source, its test failed, restored byte for byte; one real 2.5.0 probe run the way `check_freerouting` starts it (headless, scratch home, folders pinned): banner, two WARN lines and the expected no-board complaint, so a healthy start, and every file it wrote stayed in the scratch home (its output is now a test); `simulation_live` and the release build skipped. Not verified: the bench against a real JAR (its helpers are unit-tested), Windows and Linux paths.

## Rank 29, circuit-aware review (merged, #55)

`audit_circuit_design(schematic, board?)` in the `design_review` toolset (8 tools and 260 + 7 at the time; 9 on main), run by `run_design_review`. Code: `circuit_model.rs` (parts, pins, nets of one use of a sheet; board pad distances), `circuit_checks.rs` (the checks), `circuit_review.rs` (rule table, accounting, power tree, feedback), role classifiers in `component_class.rs`. The standard is the owner's layout checklist (`target/creation-research/layout-checklist.md`, git-ignored). Written from the checklist and pin names; nothing is ported from kicad-happy, so `THIRD_PARTY.md` is unchanged.

- **Honesty.** Every finding has `checklist` and `rule`. `checks` gives each of the 36 rules a status: `ran`, `needs_human` (every subject needed a datasheet or other human input), `no_subjects` (nothing to examine, or only subjects outside the rule's remit, with the skip count and reasons) or `not_run` (`no_board`, `no_design_intent`), with examined/passed/findings/skipped/needs_human/unevaluated counts. `unevaluated` (a part missing from the board, an unrouted sense net, an unreadable termination value, a sheet use that was not audited) is a gap, with or without a board: `run_design_review` adds the diagnostic `circuit_checks_unevaluated`, so the verdict is INCOMPLETE. Subjects that need a human (a capacitor outside the 100n/1u/10u families, a regulator missing from the reference table, an ESD part of unknown capacitance, an I2C rail of unknown voltage, an unrecognised divider) are listed under `needs_human` with their subject and counted in `coverage.needs_human_subjects`; they do not make the review INCOMPLETE. `unproven` holds absences withheld because the net carries a global or hierarchical label. `needs_human` lists 20 checklist items no tool here judges. The verdict text is unchanged; LOOKS GOOD does not speak for `needs_human`.
- **Automated, placement (needs a board; pad-anchor distance, the metric `score_placement` uses, limits from its `decoupling_limit_mm`):** decoupling supply and ground legs; bulk near regulator and gate-driver supply; TVS at the connector and ahead of the circuit it protects; CAN/RS-485 transceiver near its connector; termination near the connector or transceiver; series termination at the proven driver; gate driver near its MOSFET and gate resistor at the gate; bootstrap cap at the driver pins; sense amplifier near the shunt plus a copper-graph Kelvin check; crystal load caps and crystal near the IC; ADC reference cap.
- **Automated, schematic:** connector signals protected, protection not behind a series part; CAN termination (direct, split, jumper-selectable) is 120 ohm and the bus has no star; RS-485 termination and fail-safe bias; I2C pull-up rail against device supply (voltages from the record, else the net name) and total value, with module pull-ups named as not counted; UART TX-TX and RX-RX; USB ESD present and low-capacitance (family tables; unknown parts are left to a human) and the recorded 90 ohm pair; bootstrap cap present; gate-drive return net equals the MOSFET source net; crystal load caps present with the computed CL (3 pF stray assumed, target left to a human).
- **Power tree (needs the record):** rail nets exist, name agrees with `voltage_v`, `source_ref` exists and is on the rail (directly or one inductor/resistor away), its input rail, and regulator feedback dividers against the recorded voltage using a built-in table of 15 reference voltages (confirm against the datasheet); an unknown regulator reports its implied reference.
- **Thresholds are heuristics, named in `circuit_checks.rs`** (ESD 10 mm, transceiver 25 mm, gate resistor 5 mm, gate driver 15 mm, sense amp 15 mm, crystal cap 5 mm, crystal 10 mm, bootstrap 5 mm). The owner should tune them.
- **Limits.** One circuit per use of a schematic file: nets do not join across sheets except by the `leaves_sheet` marker. A sheet used twice is audited once per use, with each symbol's designator read from its `(instances)` record for that sheet path; a use whose designators cannot be read that way, or repeat another use's, is listed under `unaudited_sheet_uses` and makes the review INCOMPLETE. Parts marked `dnp` or `on_board no` are left out of every rule and listed under `unfitted_parts`. A rail recorded as `/Sheet/NET` matches only a net local to that sheet. A CAN bus that leaves the sheet or passes a multi-pin choke is `unproven`, never a pass. Pad distances, not trace order or routed length. Kelvin check needs tracks; a zone-joined or unrouted sense net is `unevaluated`. Connector exposure is not known (`J1` internal header looks like an external connector), so unprotected signals are `info` unless the net is in a recorded interface. I2C missing pull-ups stay with `audit_connections`. B23 derating is not started. Section 3 (return paths) belongs to the board-level audit; section 7 and the rest of 1, 4, 5, 10, 11 are `needs_human`.

Verified on macOS (2026-11-08): 57 unit tests (`circuit_review_tests.rs`, one or more per rule: fires, passes, reports not-run or not-evaluated) and 5 review-integration tests (`design_review_circuit_tests.rs`) over scratch schematics and boards; clippy `-D warnings`; 4 negative controls each caught by a test (ground leg ignored, cross-sheet guard removed, unevaluated gaps not counted, Kelvin branch detection off). Not verified: a real KiCad board (no owner project was used), a live kicad-cli review with a board, Windows and Linux.

Review follow-up (2026-11-09), from 12 verified review findings on #55:

- Test points (`Connector:TestPoint*`, `TP` + digit) are no longer connectors, so they no longer raise ESD-distance, unprotected-signal or CAN-star findings.
- Decoupling judges each supply pin by its highest-frequency family (100n, then 1u, then 10u) against that family's own limit; a nearer 10 uF cap no longer excuses a far 100 nF one. A pin with only a 10 uF cap gets an `info` finding saying it has no 100 nF or 1 uF cap, never a silent pass. `placement.rs` is unchanged.
- Subjects that need a human are listed under `needs_human` with their subject and counted in `coverage.needs_human_subjects`; a rule whose subjects were all of that kind reads `needs_human`, and one with only out-of-remit skips says so instead of "nothing to examine". They do not make `run_design_review` INCOMPLETE; its `circuit` block gives the count.
- `dnp` and `on_board no` parts are left out of every rule, are never "missing from the board", and are listed under `unfitted_parts`.
- A sheet used more than once is audited once per use, each symbol's designator read from its `(instances)` record; a use that cannot be read that way, or repeats another use's designators, is listed under `unaudited_sheet_uses` and makes the review INCOMPLETE.
- CAN: no connector reached, a bus that leaves the sheet, or a multi-pin choke on the bus is `unevaluated` or `unproven`, never a pass. A two-pin header or `JP` jumper can select the termination and is not counted as a bus branch.
- The recorded USB pair impedance is judged once per design, with the design-intent record as its source.
- A source reaches its rail through a resistor only from a feedback pin, and through an inductor or ferrite only from a switch or output pin (any pin of a non-regulator). Gate drivers are not regulators.
- A rail recorded as `/Sheet/NET` matches only a net local to that sheet; bare and `/NET` names behave as before.
- The rule count is 36 (a test pins it), feedback pin names compare in normalised form, and the docs say any unevaluated subject makes the review INCOMPLETE, with or without a board.

Verified on macOS (2026-11-09): 33 new tests (83 in `circuit_review_tests.rs`, 12 in `design_review_circuit_tests.rs`) and 5 existing tests updated; 30 negative controls, each caught by a test (two first passed and got a test of their own); fmt, clippy `-D warnings`, the workspace tests and `cargo deny check licenses` clean. Not verified: a real KiCad board, Windows and Linux.

## Rank 21, board construction (merged, #48)

Findings A13 and B25. Branch `feat/creation-board-construction`, claimed from main 966a89d before any code. Four new tools in the `pcb_board` toolset (13 to 17), in `tools/board_construction.rs` (declared from `pcb_board.rs`, tests in `board_construction_tests.rs`), plus a polygon mode on `add_board_outline`:

- `set_footprint_lock`: without `locked`, reads every footprint's lock state (or those in `references`) from the saved file. With `locked`, previews, then applies with the preview's `revision`. Writes `(locked yes)` after the library id as KiCad 10 does; reads and removes KiCad 6's bare `locked` token and `(locked no)`. Unknown references are refused; a reference carried by two footprints is `ambiguous_target`.
- `add_rule_area`: rectangle or simple polygon on one or more copper layers (`*.Cu` expands to the board's copper layers, in board order), the five keepout flags (default, as KiCad 10's own new rule area in `pcbnew/zone_settings.cpp`: tracks, vias and pads forbidden; copper pours and footprints allowed). Written net-less, as KiCad 10 writes a rule area, with `(layer …)` for one layer and `(layers …)` for several. Names must be unique among zones. A rule area that forbids neither tracks nor vias is accepted, and the preview says the DSN export leaves it out. One that forbids nothing is refused.
- `list_zones`: top-level copper zones and rule areas from the saved file (UUID, name, kind, net, layers, keepout flags, priority, corners, bounding box). Read-only.
- `delete_zone`: by UUID or unique name; a name matching two zones is `ambiguous_target`, and so is a target whose UUID two zones carry. Zones inside footprints are not touched. Copper fills of the remaining zones are left as saved.
- `add_board_outline` takes `points` instead of x1..y2: corners in order, each optionally with `arc_mid` to make the next edge an arc. It goes through the same IPC-or-file gate as the rectangle. The loop must be simple: each arc is intersected exactly with every other edge (segment with circle, circle with circle, limited to the sweep), because chords miss an arc that swings across another edge. The reported `bbox` includes each arc's bulge. The file path writes conditionally, reads back, and reports Edge.Cuts shapes that were already there. It still appends, like the rectangle.

The three writers have no IPC path. Every write call refuses while KiCad holds the board open or its lock file is present (`refuse_if_board_open_in_kicad`), previews by default, binds `revision` to the operation, board path, saved bytes and normalised request (a preview never authorises a different request, and a `stale_target` refusal carries no revision, so the caller must preview again), writes with `write_atomic_if_unchanged`, and reads the file back (`mutation_outcome_uncertain` if the readback does not prove the result). Rule areas: the DSN export (`specctra::export_dsn`, not edited here) turns each one that forbids tracks or vias into one keepout per copper layer (`keepout`, `wire_keepout` or `via_keepout`).

Verified on macOS (2026-11-08): 22 unit tests over scratch copies of the KiCad-authored `specctra_rounded_board` fixture: lock and unlock round-trip the file byte for byte, the KiCad 6 token, `(locked no)`, no-op apply, unknown/duplicate references, stale revision after a file edit and after a request change, KiCad lock file, a mock KiCad holding the board (all three writers refused, file untouched); rule area survives `export_dsn` as keepouts with the right kind, layers and corners; nine malformed rule areas refused; `list_zones` on the fixture; delete by name removes the keepout from the DSN while the pour stays a plane; delete by UUID; unknown, ambiguous and empty delete targets; an L-shaped outline with an arc edge exported by `export_dsn` as one boundary; six malformed outlines refused; arc sampling and the ring check. The ignored `live_pcbnew_loads_the_written_rule_area_and_lock` passed with KiCad 10.0.8's bundled `pcbnew`: it loads the written rule area with the requested five flags, three corners and two copper layers, and reports R2 locked. Four negative controls (revision check, polygon check, arc sampling, open-in-KiCad gate on `delete_zone`) each failed a test and were restored. Gate: fmt, clippy `-D warnings`, doc tests, `cargo deny check licenses`, `fix-doc-counts` (263 + 7 meta at the time) and the release build pass. The full `cargo test --workspace --lib --tests` run had 3,736 passed and 19 failed: one was this PR (`arc_mid` in the skill text tripped the asset-reference check; reworded, the suite now passes), and 18 were mock-IPC receive timeouts in `pcb_sync::tests` (16) and `pcb_routing::custom_rule_tests` (2), which pass alone (89 and 8). After merging main (#45, #46, #47), the whole run was repeated on the merged tree: 3,767 passed, 0 failed, with fmt, clippy and doc tests clean. `simulation_live` was skipped (not touched). A stdio run of the release binary through real dispatch on a scratch copy of the fixture without its outline: polygon outline with an arc edge, rule area preview then apply, lock preview then apply, lock read, `list_zones`, a stale `delete_zone` revision refused, an unknown argument refused. `kicad-cli` 10.0.8 DRC on that board reports no outline errors (only library-lookup and unrouted items, expected for the scratch fixture). The installed binary is unchanged (`3de188bb775ab4ce`).

Not verified: a live KiCad GUI (no IPC path exists to test; the refusal is tested against a mock), the IPC path of the polygon outline (shared with the rounded rectangle, untested here), Freerouting actually routing around a new keepout (the DSN content is tested, the owner-deferred live run is not), `get_component_list` still has no `locked` field (it lives in `pcb_components.rs`, outside this PR's files; `set_footprint_lock` reads it instead), Windows and Linux. PR 16 (#44) edits `specctra.rs`; if it changes `export_dsn`'s signature, these tests need the new call.

Review follow-up (2026-11-09), from the verified review of #48:

- A `stale_target` refusal no longer names the current revision (reason `revision_mismatch`, as `restore_checkpoint`), so a wrong or guessed revision cannot be turned into an unpreviewed write.
- `delete_zone` refuses, before any write, a UUID that more than one zone carries, naming the count; a zone picked by name whose UUID another zone carries is refused the same way.
- The `set_footprint_lock` description and the `kicad-pcb` skill say that only `auto_place_from_schematic` and `refine_placement_force_directed` hold a locked footprint, that `update_pcb_from_schematic` keeps a locked orphan, and that the explicit move tools do not check the lock. Their behaviour is unchanged.
- The polygon outline check intersects arcs exactly instead of sampling them every 5°; the reviewer's spike, 0.065 mm past a radius-145 arc, is refused.
- The polygon outline's `bbox`, and `list_zones`' for a zone with an arc edge, include each arc's axis extremes.
- `add_rule_area`'s defaults now match KiCad 10.0.1: tracks, vias and pads forbidden; copper pours and footprints allowed.
- New tests: a lock file between preview and apply for all three writers, `commit`'s write-time lock and conflict refusals, the byte-exact delete by UUID, and the stale refusal for all three writers.

Verified on macOS (2026-11-09): 30 board-construction unit tests pass (one more is ignored), plus a new `creation_guidance` test for the skill text. 14 negative controls each failed their test and were restored byte for byte. Removing only the `add_rule_area` preflight still refuses an apply after a lock appears, because `commit` catches it; removing both fails the test. Gate: fmt, clippy `-D warnings`, doc tests (9 passed), `cargo deny check licenses`, and `fix-doc-counts` (no change, 263 + 7 = 270). The workspace `--lib --tests` run had 3,783 passed and 0 failed (the `konnect-ipc` handshake-timing test failed once under load and passed 5 of 5 alone). The arc check's touch tolerance is 1e-9 mm, below KiCad's 1 nm grid; tightening it to KiCad's resolution is left for later. Not re-run: the ignored `live_pcbnew_loads_the_written_rule_area_and_lock` (it now passes all five flags, the same ones as its earlier pass), the release build and the stdio run. The installed binary is unchanged (`3de188bb775ab4ce`).

## PR 16, Specctra DSN for real boards (merged, #44)

Rank 32 (D7), first part. Merged as [#44](https://github.com/yuvaltal912-sys/Konnect/pull/44) from `feat/creation-dsn-widening`. The claim-only commit and draft preceded implementation, from main 966a89d. Code in `specctra.rs` and `specctra_ses.rs`; tests in `specctra_layers_tests.rs` over a fixture KiCad 10.0.8's own `pcbnew` wrote (`specctra_four_layer_back.kicad_pcb`: 4 layers with a `power` layer, a back-side part at 90°, a through-hole part, a locked In1.Cu track, a locked via, an In2.Cu pour).

- **Layers:** 2 to 32 copper layers in stack order; KiCad `power` layers become Specctra `power` layers. Through-hole and via padstacks cover the stack.
- **Back side:** a back part is written as a front placement with its pads at their true board positions on B.Cu (KiCad stores flipped pads already mirrored, and the same rotation maps them; checked against `pcbnew`'s `PAD::GetPosition`), so the router never mirrors. KiCad's native exporter is not adopted for these boards.
- **Three pre-existing bugs fixed:** pin `rotate` was the board-space pad angle, so parts at 90°/270° had non-square pads turned twice; drill offsets were ignored (looked for on the pad, not in the drill), putting offset copper on its hole; and placements kept KiCad's negative angles, which Freerouting echoes normalised (`-90` → `270`), so the importer refused every such board as moved. Placements are now in [0, 360) and the importer compares directions.
- **`pours: "omit"`** (new argument, default `fixed`): leaves copper pours out of the DSN so the router can cross their area, the usual route-then-refill workflow; rule areas stay, the manifest still counts the pours. Without it a board-wide pour, exported from its outline as fixed copper since PR 2b, blocks most routing (complex_hierarchy: 86 unrouted with it, 0 without).
- **SES repeats:** Freerouting 2.5.0 wrote the same segment twice on two demo boards; an exact repeat (same net, layer, width and geometry) is now applied once and counted (`duplicates_skipped`), instead of refusing the whole SES.
- **Copper text keepouts** are rectangles at the text's own angle and justification (1.1 em per glyph), not a square for every rotation: that square was 67 mm wide on one demo board.
- **Pads and copper the router never saw:** `connect` pads, SMD pads on either outer layer, plated slots, repeated and empty pad numbers (`MP@1`, `@1`), unplated and ringless unconnected holes as keepouts, copperless pads skipped, padless footprints not placed (see the review follow-up for their copper), copper graphics and text as covering keepouts (they were invisible to the router before), board names with spaces.

Real-board check: KiCad 10.0.1's demo boards, scratch copies with routing stripped by a line-based script (it could not parse one board's formatting, so 17 were probed), through the ignored probe `probe_dsn_export_over_a_folder`. Before this PR 2 of 17 exported; now 9 do, including two 4-layer boards and boards with 14, 81 and 103 back-side parts. The 8 refused stop on what is still unsupported: custom pad shapes (2), local clearance overrides (2), open outlines (2), a `jumper` layer (1), duplicate footprint references (1).

Live routing (2026-11-08): Freerouting 2.5.0 (it needs Java 25: `/opt/homebrew/opt/openjdk@25`, installed with the owner's OK) routed all 9 exporting boards with `pours: "omit"`, and the strict import planner accepted every SES, through the ignored test `freerouting_routes_a_folder`. Six reported 0 unrouted (complex_hierarchy, both ecc83, interf_u, multichannel_mixer, sonde xilinx); the 4-layer coldfire had 3 unrouted (3011 tracks, 248 vias, 6 min) and the 4-layer video 8 of 588 nets (8370 tracks, 901 vias, 24 min); one log did not report a count. Every pin of six boards (3,922) matched the board model within 1 µm (`probe_pins_match_the_board_model`). Not run: `apply_specctra_ses` into a live KiCad and DRC after a refill.

Not done (the rest of rank 32 and follow-ups): the Freerouting version policy and benchmark (B34, D24) are in their own PR; custom pad shapes, local clearance overrides, blind/buried vias, copper text as exact outlines, and the exporter's own option to keep some pours (per net) while omitting others.

Verified on macOS (2026-11-08): the gate (fmt, 3759 lib and integration tests with 0 failures, doc tests, clippy `-D warnings`, `cargo deny check licenses`, `fix-doc-counts`, release build); 22 new tests in `specctra_layers_tests.rs` (plus three ignored probes), with back-side pad positions and orientations checked against KiCad 10.0.8's `pcbnew`; existing 2-layer DSN output unchanged; 9 negative controls (relative pad rotation, drill offset, native back-side refusal, copper-graphic keepouts, through-via span, SES layer count, unique pin ids, design-name token, placement-angle comparison), each caught by a test. Not verified: a live KiCad (the export reads the IPC snapshot; tests feed it the saved file), Windows and Linux.

## Rank 35, call log isolation (merged, #46)

Findings C1 and C2. Test traffic was about 94% of the owner's real call log, and 1,028 of its lines held records glued together by concurrent writers.

- **Path.** `observability::calls_log_path()` replaces the HOME-only default in `McpHandler::new`. `KONNECT_CALL_LOG=off` disables the file, an absolute path replaces it, anything else disables it with a warning. Otherwise a set `KONNECT_STATE_DIR` keeps the log at `logs/calls.jsonl` inside it, else the old default. Real users set neither, so their path is unchanged.
- **Tests.** konnect-core's own tests always get `None` (`cfg(test)`). `.cargo/config.toml` `[env]` sets `KONNECT_CALL_LOG=off` for everything cargo runs, `cargo run` included (set an absolute path to keep a log there). The stdio test helper sets it again for the servers it spawns.
- **Write.** Each record is serialised with its newline into one buffer and written with a single `write_all` on the append-mode handle. `writeln!` on a `File` had issued two writes.

Verified on macOS (2026-11-08): the gate (fmt; 3739 lib and integration tests, 0 failures; doc tests; clippy `-D warnings`; `cargo deny check licenses`; `fix-doc-counts`; release build). `simulation_live` was skipped: this PR does not touch simulation. New tests: override resolution; the `cfg(test)` guard with no env at all; a handler built in a child test process with HOME at a temp folder and every override removed writes nothing there; four processes appending 1,600 records to one file give only whole lines; a test process does not resolve the real path; a spawned server with HOME and `KONNECT_STATE_DIR` at a temp folder logs only into the state folder, and with the log off writes none. A stdio run of the release binary with that isolation logged 2 whole records in its state folder. Three negative controls each caught by a test: `writeln!` back (3 of 3 runs), the `cfg(test)` guard removed, the cargo `[env]` removed. The owner's existing log was not read, changed or cleaned. Not done: the `server_version`, `pid` and `session_id` record fields that C1 also proposed, rotation, and Windows and Linux.

## PR 15 follow-up, the review reads a DRC baseline (merged, #47)

`run_design_review` takes `baseline`, the name of a DRC baseline of the same board (`save_check_baseline` with `board`). It is loaded before anything runs with `check_baselines::load_for_compare`, now shared with `compare_check_baseline`, so a missing name (the stored names are listed), a damaged file, a symlinked store, a baseline of another board, an invalid name and `baseline` without `board` are all refusals and run nothing. The review's DRC run is normalized and diffed with PR 15's `normalize_drc` and `diff`; no key logic is repeated.

- Counted against the verdict: new violations, severity increases (`baseline_severity` given) and violations in a category the baseline did not report (`not_compared`, since nothing vouches for them). Each finding carries `baseline_change` and `baseline_key`. Unchanged and lowered ones are not findings.
- `design_review.baseline` names what was used: `name`, `path`, `revision` (SHA-256 of the stored file), `created_utc`, `source`, then the counts (new, raised, lowered, unchanged, resolved, not compared), per-rule tallies, up to 20 resolved violations, `keying`, `source_changed_since_baseline` and `warnings` (`options_differ` when the baseline ran with refill or live sync: the review runs neither). `null` without the argument.
- The verdict ends with the comparison, e.g. `NOT READY — … (DRC counted against baseline 'pre-route': 1 new, 1 unchanged, 1 resolved)`. `drc` keeps the run's absolute counts.
- DRC that cannot run: INCOMPLETE as before, and `baseline.status` is `not_compared` with `reason: drc_unavailable`.
- ERC: the review runs none, so an ERC baseline does not apply; the schema says so. The `kicad-review` skill now says to report the unchanged count and not to call such a board clean.

Verified on macOS: 6 unit tests (`design_review_baseline_tests.rs`) over a scratch project with a stand-in `kicad-cli`: new/unchanged/resolved in the verdict with the re-measured and moved violation matched; an unchanged board LOOKS GOOD against its baseline and NOT READY without it; severity raised counts and lowered does not; a category missing from the baseline counts; every refusal runs no DRC and leaves a damaged file as it was; DRC unavailable keeps the review INCOMPLETE. The 23 PR 15 tests pass after the loader refactor. 4 negative controls, each caught: counting unchanged findings, dropping severity raises, ignoring a missing baseline, dropping not-compared categories. The gate (2026-11-08): fmt; 3747 lib and integration tests passed, 1 failed and then fixed (`asset_references` caught a backticked field in the new skill line; rerun 12/12, and the `konnect` crate 191/191 afterwards); doc tests; clippy `-D warnings`; `cargo deny check licenses`; `fix-doc-counts` (no change, 259 + 7); release build; installed binary SHA-256 still `3de188bb775ab4ce`. `simulation_live` skipped (not touched). Not verified: a live kicad-cli run (the PR 15 live test covers the keys), Windows and Linux.

## Rank 33, local part import (merged, #52)

Finding B20, rescoped by the owner on 2026-11-08. One tool, `import_library_part`, in `tools/library_part_import.rs`. It was a `library` tool; with #50's `render_library_item` that toolset would hold 21, over the router's 20-tool cap, so it is registered in `integration` (10 tools), next to the JLCPCB part search and datasheet tools (approved by the owner on 2026-11-09). The tests are in `library_part_import_tests.rs`, and the fixtures in `tests/fixtures/part_import/`.

**Why local files, not an LCSC fetch.** Every peer importer reads EasyEDA's internal `easyeda.com/api/…` endpoints. EasyEDA's terms (effective 2026-09-21, section VII) restrict automated collection of their content and forbid modifying it or making derivative works. Their robots.txt disallows `/api/` for every agent, and disallows the whole site for `Claude-User`, `ClaudeBot` and `anthropic-ai`. So the tool makes no network requests. The owner exports or downloads the part's KiCad files (EasyEDA/LCSC export, SnapEDA, Ultra Librarian, a vendor's own), and the tool copies them into the project. It converts no format: the inputs must already be KiCad 6+ files (a `(module …)` footprint is refused with the `kicad-cli fp upgrade` hint). No code was taken from easyeda2kicad (AGPL-3.0) or JLC2KiCad_lib (MIT), so `THIRD_PARTY.md` is unchanged.

- **Targets.** `<project>/<library>.kicad_sym` (created or appended), `<library>.pretty/<footprint>.kicad_mod`, and with `model_file` `<library>.3dshapes/<model>`. Both libraries are registered as `${KIPRJMOD}/…` in the project tables. The default `library` is `imported_parts`. The project folder must hold a `.kicad_pro`.
- **Preview by default.** It shows every write with its action, the symbol's pins (number, name, type), the footprint's pads (number, type, shape, position, size, drill), the model action, the registrations, warnings, `blocked_by` and a `revision` (`sha256:` over each write's path, before and after images, the reused model and footprint, and the blockers). Pin and pad lists are capped at 300; counts are complete.
- **Apply** needs that revision (otherwise `stale_target`), refuses on any blocker (`plan_blocked`), then commits everything in one `commit_file_transaction` and reads every file back. The pin-pad check runs again on the saved files.
- **Never overwrites.** An identical footprint or model already at its target is reused (`reuse_identical`), so parts sharing a footprint go into one library. Blockers: `symbol_exists`, `footprint_exists` and `model_exists` (different bytes), `nickname_taken` (the project table binds the nickname to another library), `library_already_registered` (another nickname's row resolves to the same file), `library_disabled`, `symbol_library_unusable`, `lib_table_unusable`, `target_is_a_symlink` (also a linked lib table or footprint file), `target_has_wrong_type`, `target_case_mismatch` (an existing entry differs only in case). A nickname also in the global table, or a `(hidden)` row, is a warning.
- **Fields.** Footprint becomes `<library>:<footprint>` (the old value is reported as `footprint_field_before`). Optional `lcsc` (validated `C<digits>`), `mpn` and `manufacturer` go into the LCSC, MPN and Manufacturer fields, through PR 10's field editor.
- **Model.** `model_file` (`.step`/`.stp`/`.wrl`, UTF-8 text, format sniffed past a leading BOM, copied byte for byte, 64 MiB cap) replaces every model reference with one `${KIPRJMOD}` reference and keeps the first one's offset, scale, rotation, `hide` and opacity; embedded models that only the replaced references used are removed. A non-UTF-8 model is refused with a hint to re-export it as an ISO 10303-21 conforming (ASCII) STEP. Without `model_file`, existing references are kept and each is reported as `found`, `missing`, `uses_a_path_variable`, `relative_to_the_source_file_only`, `embedded` or `embedded_file_absent`.
- **Pin-pad check** is PR 10's `check_symbol_footprint` comparison, factored into `pin_pad_check` and run on the candidate. A mismatch is a warning, never a blocker (exposed or mounting pads are common).
- **Format version.** Appending a newer-format symbol raises the library's `(version …)` and warns. Derived (`extends`) symbols are refused, with a hint to flatten them in KiCad.

**Verified on macOS (2026-11-08):**
- 10 unit tests over scratch projects. They use synthetic fixtures (an LDO symbol, an SOT-23 footprint with a transformed vendor model reference, a stub STEP file) and KiCad 10's real `Device.kicad_sym` and `C_0603_1608Metric.kicad_mod`. They cover: a preview that writes nothing, apply with readback, the model transform kept, the tables written, `check_symbol_footprint` agreeing on the saved files, missing, wrong and stale revisions, appending a second part with a version raise and unchanged tables, every overwrite blocker, the nickname guard, a reported pin-pad mismatch, and 12 refused inputs that write nothing.
- 4 negative controls, each caught by a test: the symbol-exists blocker, the revision check, the model transform, and the nickname equivalence.
- The existing library tests pass unchanged.
- A stdio run of the release binary through real dispatch on a scratch project: preview, then apply with readback (5 files written, saved check `pass`), and an unknown argument refused.
- kicad-cli 10.0.8 read the imported library and footprint on a scratch copy: `sym export svg` and `fp export svg` both plotted.
- The full gate: fmt; 3739 lib and integration tests; doc tests; clippy `-D warnings`; `cargo deny check licenses`; `fix-doc-counts`; release build. Three `pcb_sync` mock-IPC tests timed out under the parallel run and pass alone (89/89). `simulation_live` was skipped because nothing in simulation changed.

**Not verified:** a real EasyEDA/LCSC export, which was never downloaded (downloads need the owner's OK, and EasyEDA forbids automated fetches); placing the imported part in a schematic or board in a live KiCad; a project open in KiCad during the import (the tables are written, and the result asks to reopen the project); Windows and Linux.

**Review follow-up (2026-11-09).** Nine verified review findings, all fixed:
1. A `project` path relative to the server's working folder is made absolute first. Before, every apply failed as "outcome uncertain".
2. A commit refused before anything was written is now `plan_blocked` with a `transaction_refused:` reason, not "files may be written". Only active journals count as pending: abandoned evidence files no longer do, and a project that cannot be inspected does. A linked lib table or footprint file blocks in the preview, and a folder that cannot be created is refused with nothing written.
3. An identical footprint is reused (`reuse_identical`, bound into the revision like the model), so parts sharing a footprint go into one library. A different one still blocks (owner decision).
4. A row under another nickname whose URI resolves to the same file blocks (`library_already_registered`), also before the file exists (a `${KIPRJMOD}` or absolute URI is then compared with `.` and `..` applied), and so does an existing entry that differs only in case (`target_case_mismatch`). No second row is added.
5. A `(disabled)` row blocks (`library_disabled`) instead of reporting `unchanged`; a `(hidden)` row warns.
6. Model replacement keeps the first model's `hide` (bare, `(hide)`, `yes` or `true`, as KiCad reads it) and opacity.
7. A `kicad-embed://` reference reports `embedded` (no warning) or `embedded_file_absent`. With `model_file`, embedded models that only the replaced references used are removed and listed.
8. The STEP/VRML sniff skips a leading UTF-8 BOM, and the bytes are copied as they are. A non-UTF-8 model is still refused, now with a hint to re-export it as an ISO 10303-21 conforming (ASCII) STEP (owner decision).
9. Tests now reach the same-file branch of the nickname check, every blocker, a relative project path and a refused commit. Two of them found that a `.pretty` that is a file or a `.kicad_sym` that is a folder ended in an internal I/O error instead of `target_has_wrong_type`; both now block.

The earlier "nickname equivalence" control covered only an identical URI; the same-file branch now has its own test and control.

Verified on macOS (2026-11-09): 25 unit tests in `library_part_import_tests.rs` (15 new, 1 updated); 19 negative controls, one per new guard, each caught by its test, and the source matched the fix again after each. The gate: fmt; 3778 lib and integration tests passed, 0 failed; 9 doc tests; clippy `-D warnings`; `cargo deny check licenses`; `fix-doc-counts` (no change, 260 + 7 at the time). The release build and `simulation_live` were skipped. Not verified: a real KiCad 9+ footprint with an embedded model (the tests use synthetic embedded data), KiCad opening a model with a BOM, and the case check on a case-sensitive disk.

## PR 15, ERC/DRC baselines (merged, #43)

Three tools in the `verification` toolset (15 to 18), in `tools/check_baselines.rs`. Finding C4: agents diffed ERC/DRC reports by hand in Bash 18 times to prove an edit broke nothing.

- `save_check_baseline` runs ERC (`schematic`) or DRC (`board`, with `run_drc`'s `sync_live_board` and `refill_zones`) and stores every violation at `<project_dir>/.konnect/baselines/<erc|drc>/<name>.json`, beside the visual baselines. An existing name is refused before the check runs (a synced DRC saves the board) unless `overwrite`. Create-only or revision-conditional write, then read back.
- `compare_check_baseline` loads the baseline first (a wrong name runs nothing and lists the stored names), runs the same check and lists `new`, `severity_changed` and `resolved`, paged through the PR 3 helper, with `unchanged_count`, per-rule tallies and `verdict` (`regressed` when anything is new or more severe). `output` writes everything, unchanged included. Warns when the DRC options or the ERC's KiCad version differ from the baseline's.
- `list_check_baselines`: both kinds, newest first, `source_state` (`unchanged`, `changed`, `missing`); unreadable files listed apart.

Stable keys: a hash of check, DRC category, rule, ERC sheet and the sorted item identities. Severity, prose and positions are not in it. An item is its UUID only when the UUID is in the saved design (DRC: ownership enrichment resolved it on the board; ERC: it occurs in a project `.kicad_sch`). Otherwise it is its description with measured values (`0.15 mm`, `45°`) masked, which covers KiCad 8 boards whose pads have no saved UUID. Repeats are numbered `~2`, `~3`. A DRC category that only one run reported is listed in `categories_not_compared`, never as resolved. `run_erc` and the baselines now share `sch_export::erc_report`, so both refuse a sub-sheet the same way.

Verified on macOS: 23 unit tests over scratch projects with a stand-in `kicad-cli` (new/resolved/unchanged for ERC and DRC; a re-run reordered, moved, with items swapped and DRC prose re-measured matches exactly; made-up UUIDs; repeats; severity changes; missing categories; refusals before any run; damaged and renamed baseline files; a symlinked `.konnect`; 1,500 changes under the client cap with paging covering each once), 12 negative controls each caught by a test, and the ignored live test `live_rerun_of_an_unchanged_project_matches_its_baseline` with kicad-cli 10.0.8 on a scratch copy of the KiCad 10 `pic_programmer` demo (5 ERC and 5 DRC violations, every item keyed by UUID, re-run 0 new and 0 resolved). `cargo test -p konnect-core -p konnect` and clippy pass. Not verified: a live edit between save and compare, `sync_live_board` against a live KiCad (it reuses `drc::run`, already covered), a KiCad 8 board, Windows and Linux.

Follow-ups, not in this PR: read baselines in `run_design_review` (merged, #47, section above); a delete tool (overwrite covers replacement today); the documented edit loop in the skills and agents (rank C29).

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

## Rank 25, inline layer and library-item renders (merged, #50)

Claim commit and draft PR came before code. Findings C6, A25, B9 and B7.

- **The text risk did not block.** KiCad 10.0.8 `pcb export svg`, `sym export svg`
  and `fp export svg` draw every visible string as stroke or outline paths and add
  an invisible `opacity="0"` copy, exactly like schematic exports. A board text in a
  TrueType font (Arial) was also plotted as outline paths. So `konnect-render`'s
  refusal of visible `<text>` stays unchanged, with no allowance; a future export
  that does emit visible text is refused, not rendered with a guessed font.
- **`render_board_layers`** (`pcb_export`, beside `export_svg`): named layers of
  the saved board as a PNG image block. Without `region` the page is the
  board-edge bounding box (`--page-size-mode 2`); with `region` it plots on the
  drawing sheet (`--page-size-mode 1`), where page mm equal board mm, and crops.
  A region off the sheet is refused with a hint to drop `region`. Undeclared
  layers are refused by name. Colours are KiCad's default plot colours on its
  default dark board background (`#001023`), because pale silkscreen and white
  drill marks vanish on white; `black_and_white` plots on white.
- **`render_library_item`** (`library`): one tool, kind chosen by the path. A
  `.kicad_sym` renders symbol `name` (`unit`, `body_style`; metadata lists units
  and De Morgan); a `.pretty` renders footprint `name`; one `.kicad_mod` (what
  `create_footprint` writes) is staged as a private `.pretty`, since kicad-cli
  reads only libraries. Options for the other kind are refused. kicad-cli exits 0
  without writing for an unknown footprint, so the file is the evidence. One tool
  instead of two because `library` sits at the router's 20-tool cap with it; a
  future library tool (for example rank 33's local import) must split the toolset.
- **`compare_visual_baseline` `inline_diff`** now returns a generated difference
  image (`konnect_render::diff_image_png`): added green, removed red, recoloured
  orange, unchanged drawing faded, a blue box around the change; legend in
  `diff_image`. Same canvas, threshold and background rule as the drift numbers.
- New renderer entry points: `svg_page_to_png` (whole page with the region path's
  8192 px height and 16 Mi pixel caps; `svg_to_png` has no height cap and stays
  for baselines), `svg_region_to_png_on` (background colour), `page_size_mm`.
  Renders write nothing beside the design unless `output` is given; the SVG goes
  to a private temporary directory.
- Verified: 25 new or rewritten tests (renderer 7, handlers 16 with a stand-in kicad-cli
  serving SVGs the real kicad-cli 10.0.8 exported from fixture parts written for
  this, schematic 1 rewritten and 1 new); 5 negative controls (visible-text guard,
  height cap, undeclared layer, foreign options, diff image vs current render),
  each failed its test and was restored. Release binary by stdio pipe: an image
  block arrived for a demo board (full, black and white, region), a 74LS00 De
  Morgan unit, a SOIC-8 and a single `.kicad_mod`, and for a real difference
  image; four refusals came back as structured errors. Gate under "Verify":
  all green except 21 IPC-mock tests in `pcb_sync`/`pcb_routing` that timed out
  under a load average near 40 from parallel sessions and passed when rerun alone.
  `simulation_live` was skipped (out of scope).
- Not verified: display in an actual MCP client, Windows and Linux, kicad-cli
  other than 10.0.8, boards placed off the drawing sheet with a region.
- **Review follow-up (2026-11-09), nine findings fixed:**
  - The difference image is an extra. When it would exceed 16 Mi pixels or the
    inline budget, `compare_visual_baseline` still returns status and metrics,
    with `diff_image` null and a `diff_image_skipped` warning that names a
    baseline `width_px` that fits.
  - Colours are pinned: `pcb` and `fp export svg` get `--theme _builtin_default`
    unless `black_and_white`. That is KiCad 10's built-in default theme
    (`COLOR_BUILTIN_DEFAULT`), whose board background is `#001023`. Before, the
    plot followed each machine's editor theme.
  - Footprint `layers` are checked before kicad-cli runs, against KiCad's
    canonical names and its default user names (`LSET::Name` and `LayerName` in
    KiCad 10.0.1). kicad-cli only reports an unknown name and plots the rest.
  - Board layers may be named by their user name too. Konnect passes kicad-cli
    the canonical name and reports what was asked. A name that matches two
    layers, or a canonical name another layer carries as its user name (kicad-cli
    reads user names first), is refused.
  - An option of the other kind at null or at its documented default is accepted.
  - A footprint name that is absolute, holds a path separator or `..` is refused
    before any path is joined. Symbol names never reach a path in Konnect;
    kicad-cli replaces separators in its own file names.
  - One inline budget for the inline images of `render_board_layers`,
    `render_library_item`, `render_schematic_png` and the difference image: at most
    8000 px a side and 3,750,000 PNG bytes, the Claude API's per-image limits.
    Over it, the call is refused with the remedies, and a file already written is
    named. File-only output keeps the 8192 px cap, so baselines do not change.
  - Whole-page renders get page wording and a `width_px` that fits; an inline
    page is held to the 8000 px side first, so that width also fits inline.
  - The stand-in kicad-cli knows a one-unit symbol, named `<name>_unit1.svg` as
    in KiCad 10.0.1's `doSymExportSvg`.
  - Seen in source, not fixed: KiCad 10 names a symbol with custom body-style
    names `<name>_unit<n>_<style>.svg`. Konnect does not parse that suffix, so
    such a symbol is refused with "wrote no SVG", not misdrawn.
  - Not in the budget: `get_board_2d_view` (clamped to 4000 px a side, no byte
    check).
- Review follow-up verified on macOS (2026-11-09): fmt; 3783 lib and integration
  tests, 0 failures; 9 doc tests; clippy `-D warnings`; `cargo deny check
  licenses`; `fix-doc-counts`, no change (261 + 7 at the time); installed binary SHA-256
  still `3de188bb775ab4ce`. 14 new tests and 5 extended ones, all with the
  stand-in kicad-cli or in-process renders. 19 negative controls, each failed
  its test and was restored. Not run: kicad-cli or Freerouting on a real
  board, an MCP client, the release build, the ignored `simulation_live` tests.

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

A caller that read every violation in one `get_drc_violations` or `run_erc` call must now follow `next_offset` or pass `output`; `docs/API_MIGRATIONS.md` says so.

**Rank 3 leftovers (merged, [#49](https://github.com/yuvaltal912-sys/Konnect/pull/49)).**

- **C24, `zone_fill_state`** on `run_drc` and `get_drc_violations` (`drc::zone_fill_check`): `fresh` when the zones were refilled for the run, when the board has no copper zones (rule areas don't count), or when a refill reproduces the saved fills; `stale` when it changes any; `unknown` when not checked or the check failed (the DRC result still comes back). The check is a second DRC with `--refill-zones --save-board` on a private copy of the board, `.kicad_pro` and `.kicad_dru` (`cli::refill_drc_on_private_copy`); fills are compared per zone, paired by `uuid` or `tstamp` and then by net, layers and outline, as sorted `filled_polygon` point sets, not bytes, because KiCad reorders other items on save. `zone_fill_check` counts the violations and unconnected items a refill clears or adds, with 10 examples each way. On by default (`check_zone_fills: false` skips it), so DRC on a board with zones takes two runs. Real KiCad 10.0.8 refills the same board identically twice, which is what makes the comparison sound.
- **A30, the other long results.** One helper extension, `response_budget::fit_lists` (several lists share one budget, in priority order; `truncated_lists` and `hint` appear only when something is cut), plus `Page::used` and `Page::hint`. Batch edits can't be paged by calling again, so `batch_place_components` (compact entries by default, `detail: "full"` for the old ones), `batch_connect_to_net`, `batch_connect_pins`, `batch_add_wire` and `batch_add_no_connect` are cut to fit with their counts whole. Read-only results page with `limit`/`offset`: `update_pcb_from_schematic` (`changes`, the other lists fitted into half the budget), `validate_for_manufacturing` (`issues`, with `issue_count` and `issues_by_severity`), `get_component_list` (`list_response`) and `query_traces` (`offset`, size cap).
- **Proof:** tests per tool (a long result pages or is cut under 50,000 characters with whole counts, pages cover every item once, a short result is unchanged, a bad argument is refused before any work), 12 zone-fill cases against a stand-in `kicad-cli`, and one real-KiCad test (`real_kicad_finds_old_fills_stale_and_refilled_ones_fresh`, ignored, needs `KICAD_CLI`) on a scratch copy of the pic_programmer demo: its old fills read stale, then fresh after a refill, and the board was never written. Four guards were broken one at a time (fill comparison, `query_traces` offset, the no-connect fit, the sync `change_count`); each failed a test.
- **Review follow-up (2026-11-09).** Nine verified findings, all fixed:
  1. `batch_place_components`: the outcome's retry list came after the cut, so thousands of failures passed the cap. `outcome::retry_failed_items` now lists at most 1,000 indexes (`item_indexes_total`, `item_indexes_truncated`), and the lists are cut with its size taken off the budget.
  2. Zones pair by `uuid` or `tstamp`, then by net, layers and outline. A zone nothing pairs is `unknown` (`zones_unpaired`), never `stale`. Fill points compare as numbers to KiCad's 1 nm, without `island` marks. `differing_zones` names each zone's `id`.
  3. The refill copy has no library tables, so `lib_footprint_issues` and `lib_footprint_mismatch` are left out of the refill counts (`rules_not_compared`). Copying the tables would not be exact: `${KIPRJMOD}` would point at the scratch folder.
  4. Copper zones inside footprints are read. kicad-cli's refill fills board zones only (KiCad 10 `ZONE_FILLER_TOOL::FillAllZones`), so they make the state `unknown` (`footprint_zones`; `not_comparable`, with no second DRC, when they are the only ones).
  5. An offset past the end adds `offset_past_end` and a hint with the total, in `Page::describe`, so every paged tool has it; an applied sync points to a new dry run instead.
  6. `validate_for_manufacturing` sorts its issues (severity, then text) and puts `issues_fingerprint` on every page.
  7. tool-directory rows, kicad-pcb `SKILL.md` step 5 (follow `next_offset`) and `API_MIGRATIONS.md`.
  8. Tests for the `batch_connect_to_net` and `batch_add_wire` fit, `update_pcb_from_schematic` refusing a bad window before the netlist export, and sync diagnostics cut through `truncated_lists`.
  9. `save_check_baseline` and `compare_check_baseline` forward `check_zone_fills`; an ERC baseline refuses `false`.
  Verified: `cargo fmt --check`, workspace lib and integration tests (3,788 passed, 0 failed, 155 ignored), doc tests (9 passed), clippy `-D warnings`, `cargo deny check licenses` and `cargo xtask fix-doc-counts` (no diff). 18 new guards were broken one at a time and each failed a test. The ignored real-KiCad test passed on KiCad 10.0.8 against a scratch copy of the pic_programmer demo, also with its zone ids rewritten as `tstamp`. Not run: a live client, a real KiCad 6/7 or KiCad 5 board, a board with copper zones inside footprints.
- **Limits left:** `validate_for_manufacturing`'s `outline` lists are not cut; `batch_add_junction`, `batch_add_bus` and the batch delete tools return counts or short lists and were left alone; the zone check doubles DRC time on boards with zones (also inside DRC baselines, whose `source_evidence` now carries `zone_fill`; `check_zone_fills: false` skips it there too); zones inside footprints can't be checked from kicad-cli; no live client run.

Proof: `response_budget_tests.rs` runs the real ERC handler against a stand-in `kicad-cli` that serves a 700-violation report (under the cap, counts of all 700, pages cover each violation once, `output` holds all of them), the DRC pager on 900 violations, and the list tools. Six guards were broken one at a time (the byte cap, the whole-list tally, truncation, paging order of `output`, argument checking, the summary) and each failed a test. Limits left: the cap counts characters of listed items only, so a response with very large non-list fields could still be large; the stand-in is not KiCad's own large report; the live KiCad was not asked.

## PR 2b, Freerouting on real boards (merged, #26)

`export_specctra_dsn` refused any board with an arc in its outline, a rounded or oval pad, or a copper zone, so a real routing run was blocked and the AI hand-rolled a router. What changed (`specctra.rs`, `specctra_ses.rs`, `freerouting_setup.rs`, `check_freerouting`):

- **Outline:** one closed loop of lines, arcs, a rectangle, a circle or a polygon. Specctra has no arc here, so arcs and circles are chords whose middle strays at most 5 um (`ARC_TOLERANCE_MM`); the manifest says so. Curves, old-form arcs, two loops and zero area are refused.
- **Pads:** `roundrect` as its bounding rectangle (never smaller than the copper), `oval` as a stadium path, equal-sided oval as a circle. Trapezoid and custom are still refused.
- **Zones:** a pour becomes a `plane` of its net per copper layer, a rule area that forbids tracks or vias becomes a keepout, both from the zone outline, as KiCad's own DSN does (the fixture carries KiCad's export for comparison). Thermal reliefs are not modelled: run DRC after a refill. The SES import still adds tracks and vias only; it now checks the board's zone inventory against the manifest and refuses a mismatch.
- **Setup:** `check_freerouting` asks the JAR which Java it needs (the class-file version of its main class), looks for one (`java_path`, `JAVA_HOME`, `PATH`, the usual JDK folders) and returns `setup` with the exact steps. `available` is false when the JAR cannot start. `route_specctra_dsn` refuses before starting anything when no Java fits. **Approach chosen: guidance, not a downloader.** Fetching and running a 60 MB program from the network is the owner's decision; the steps are the commands they would run.
- **Native bridge:** KiCad writes rounded pads as polygons, which the differential check does not model, so `native_bridge_mode: prefer` falls back to the Rust DSN on such boards (a test pins this).

**Freerouting could not be run here.** When this was built, the machine had only Java 11, and Freerouting 2.1.0 needs Java 21 (`UnsupportedClassVersionError`, class file 65). Java 21 was installed afterwards, on 2026-11-07, but the live run was not done. So the DSN was not run through the real engine. What was run instead: Konnect's own DSN parser round trip, the planes and keepouts compared with KiCad 10.0.8's own export of the same board, and the import planner on a Freerouting-authored SES (from another board, adapted; see the fixture README). The real-engine test is `freerouting_routes_the_rounded_board` (ignored): with a Java 21 and `FREEROUTING_JAR` set it exports, routes and plans the import. That check is superseded: PR 16's `freerouting_routes_a_folder` (2026-11-08) routed nine real demo boards through the same export and import path with Freerouting 2.5.0 (section above).

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
