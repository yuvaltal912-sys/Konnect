# Creation handoff

**S11 resumed 2026-11-12; implemented, reviewed and gated 2026-11-13.** The owner resumed PRs #63, #64 and #65 under one supervisor session (decision row "Claude resume of three PRs"); the 2026-11-10 pause no longer applies to #64. S11 runs as Claude (`claude-opus-5-5`, `xhigh`). Current state: "Row S11: suspected zone-fill defects".

The current state of the creation-side work: schematic, PCB and library tools. Anyone picking this up, human or agent, starts here. Update this file in the same commit that changes the state. The simulation track has its own file, [`SIMULATION_HANDOFF.md`](SIMULATION_HANDOFF.md).

**Last updated:** 2026-11-13 (Rows S7 and S11). On 2026-11-09 ten PRs merged into main (`a8df6ba`), in this order: [#57](https://github.com/yuvaltal912-sys/Konnect/pull/57), [#58](https://github.com/yuvaltal912-sys/Konnect/pull/58), [#56](https://github.com/yuvaltal912-sys/Konnect/pull/56), [#51](https://github.com/yuvaltal912-sys/Konnect/pull/51), [#55](https://github.com/yuvaltal912-sys/Konnect/pull/55), [#53](https://github.com/yuvaltal912-sys/Konnect/pull/53) (simulation), [#49](https://github.com/yuvaltal912-sys/Konnect/pull/49), [#48](https://github.com/yuvaltal912-sys/Konnect/pull/48), [#50](https://github.com/yuvaltal912-sys/Konnect/pull/50), [#52](https://github.com/yuvaltal912-sys/Konnect/pull/52). Each first had an independent review, with every verified finding fixed and tested in its own PR, and all ten were merged through one gated trial integration (4,153 tests, `simulation_live` 95/95, release build); every main tree after a merge matched its trial step. Freerouting now starts without telemetry, outside the home folder and off the network. Open: [#54](https://github.com/yuvaltal912-sys/Konnect/pull/54) (simulation, see `SIMULATION_HANDOFF.md`). The owner's direction: routing quality on boards like his, measured against [`LAYOUT_CHECKLIST.md`](LAYOUT_CHECKLIST.md). Catalogue on main: 269 registered tools plus 7 meta-tools; `library`, `pcb_components` and `sch_wiring` are at the router's 20-tool cap. Row S5 (routing measurement on the reference board) merged on 2026-11-10 as [#60](https://github.com/yuvaltal912-sys/Konnect/pull/60); its section below has the numbers and the preset. S6's first half merged as `f246e48` on 2026-11-10 in [#62](https://github.com/yuvaltal912-sys/Konnect/pull/62), from `codex/creation-s6-pour-widths-preset` and main `7e9a174`; the owner authorized its publication and merge on 2026-11-10. Row S7, the read-only route-refinement planner, is implemented, reviewed and gated on `codex/creation-s7-route-refine` (2026-11-13); publication and merge go through [PR #63](https://github.com/yuvaltal912-sys/Konnect/pull/63), and GitHub is the source of truth for its state ("Row S7" below). "Now" gives the fresh-session step, and "Row S6, first half" records the evidence.

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
| Process-lookup sandbox profile (2026-11-12) | The macOS sandbox refuses any exec of a setuid binary (`forbidden-exec-sugid`), so `/bin/ps` failed under `protected-native.sb` and the unchanged bench test `a_resumed_run_stops_the_freerouting_its_folder_was_left_with` failed. The supervisor approved `protected-native-ps.sb`: the same profile plus `(allow process-exec (with no-sandbox) (literal "/bin/ps"))`; every write denial still holds for every other process. Use it for cargo and native parents; keep the original for negative controls |
| Lane locks (2026-11-12) | `target/creation-research/supervision/lanes/README.md` in the main checkout: `instr.lock` for every multi-run batch (mutants, llvm-cov, guard controls); a full gate, a release/native check or anything starting real KiCad takes `native.lock` then `instr.lock`; while another worker holds `native.lock`, start no cargo run at all; a gate first waits for 20 s with no foreign cargo/rustc. Reason: gates went red on timing-sensitive tests when other cargo work overlapped them (see "Row S11", baseline observations) |
| B4 completion boundary (2026-11-10) | Owner narrowed this session to completing only PR #65, then stopping; no new creation row or feature assignment follows it. Astra reasoning is High, superseding the earlier Ultra/keep-three-active direction. Required validation, independent review, protected-resource rules and supervisor approval still apply. S7 owns AGENTS.md |
| B4 pause (2026-11-10, newer) | Owner explicitly paused PR #65 before completion. Preserve the current dirty implementation and evidence; start no code, test/build/native/instrumentation/review work until the owner explicitly resumes. This supersedes finishing PR #65 before stopping. Astra remains High; S7 owns AGENTS.md |
| B4 resume (2026-11-12) | Owner resumed PR #65 through a Claude supervisor session that directs one implementation worker in the cross-sheet worktree. This supersedes the pause row above. Scope is unchanged: finish PR #65 only, then stop; no new row starts from that branch. |
| B4 same-name sheet pins (2026-11-12) | Owner approved on 2026-11-12 ("כן, להשאיר": yes, keep it). Several pins of one name on one sheet symbol tie their parent nets together through the child's hierarchical label. The review keeps that join, stays complete, and reports it as a note (`connectivity.notes`; `circuit.connectivity_notes` in `run_design_review`), with no gap and no diagnostic. Reason: KiCad joins them, so nothing is unknown, and a diagnostic would turn a known result into INCOMPLETE. KiCad 10.0.1, `eeschema/connection_graph.cpp`, `CONNECTION_GRAPH::propagateToNeighbors`, lines 2866 to 2948: each hierarchical label looks up every parent subgraph that holds a sheet pin of its name on the same sheet instance. This replaces the "named gap" wording of verification-plan row H04. |
| S6 second half design (2026-11-13) | Creation supervisor, under the owner's delegation of 2026-11-13. (1) Per-zone `pours` values are `fixed` and `omit` only: `fixed` already writes a Specctra plane, which Freerouting 2.5.0 reads as a conduction area that other nets may cross, so "fixed copper" and "plane" are one DSN form; the public name `fixed` stays, and the docs now say what the router receives. (2) Stitching is a next-step instruction in the routing result (`completion.next_steps`) plus the same sequence in the `kicad-pcb` skill, not a step inside the tool: `route_specctra_dsn` works on files, while apply, refill and `route_batch` need the live board, and no hidden write path is allowed. Every tool and parameter the steps name is checked against the registry by a test. (3) `status` and `completion` are additive; `success` keeps its meaning. (4) A `pours` value that is not a string or an object is refused by the handler too (the schema already refused it at dispatch) |
| S7 effort control (2026-11-10) | Owner explicitly requested configurable compute quota and a shipped-agent question about effort. Supervisor approved named quick/standard/deep levels plus a bounded custom budget with conflicting selectors refused; geometry/safety and response limits do not relax. Ask once if no effort choice/delegation exists, reuse it, ask before increasing. Backend defaults remain usable. Final numeric tiers require measured rationale; published timing guidance uses public fixtures/demos, private timings remain ignored. This does not add approval prompts to authorized validation runs |
| S7 inherited clearance (2026-11-12) | Owner approved ("כן, בתנאי הזה") the planner rule that a corner repair may keep the saved clearance of the leg it shortens instead of the extra 0.01 mm margin: only against an exactly modelled obstacle, never closer than the replaced leg, never below the nominal clearance. Reason: autorouted copper already sits at exactly the nominal clearance, so the extra margin blocked most corner repairs. **Condition:** the native matrix shows no new DRC entry on any of the 11 cases. If one case shows one, the rule is not approved as it stands: stop and report, do not tune it to pass |
| S7 inherited clearance, outcome (2026-11-13) | Owner kept the rule ("כן לפי ההמלצות שלך"), although his condition was not met as worded. What he was shown: 5 new DRC identities on 3 of the 11 cases; none involves copper the plan created or changed; no clearance, short, hole or edge entry on any case in any round; 3 are an `unconnected_items` relabel that KiCad also produces with no planner on byte-identical boards; 2 are a one-for-one exchange of a `solder_mask_bridge` entry, whose trigger was not reproduced; DRC totals on those cases fell from 167 to 152–153. No source change follows |
| S7 unmet targets (2026-11-13) | Owner approved finishing PR #63 with two original targets documented as limits ("כן לפי ההמלצות שלך"). Acute corners: 8, 5, 5, 5, 3, 3, 3 left on the seven retained boards (from 11, 5, 5, 5, 5, 5, 5); the rest are shapes outside the three proposal kinds, and on six boards one of them is a T-junction within 0.06 degrees of square. The owner was first shown 2–8; the count was corrected after his answer, reported to him the same day, and he approved 3–8 as the documented limit on 2026-11-13 ("כן, מאושר"): an earlier measurement counted one more removal per board on four boards; that was a null edit (a track end moved under 0.1 µm, the angle did not change), found and removed on 2026-11-13. Cut-off ground pads: 95–96 % of ground pads in the main group on the three plane cases, from 75 %, and 100 % elsewhere, at the default radius. The search is finished after 4 rounds, so these are limits of the tool, not a question of more rounds. No new proposal kind, no change to the default radius, no tuning |
| Rust validation (2026-11-10) | Follow the owner's current local AGENTS guidance: scoped llvm-cov where useful and targeted cargo-mutants for validation/safety guards, in isolated worktrees and outputs; preserve the gate and source-mutation negative controls. Skip these tools for this docs-only claim, apply them to S7 implementation |
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
| S6 | Ground through the pour: no intent widening for pour nets, a per-zone `pours` policy in the DSN, stitching after refill | S5's result | `specctra.rs`, `specctra_ses.rs`, `tools/pcb_export.rs`, `freerouting_mcp.rs`, `tools/integration.rs`, `specctra_pours_tests.rs`; later pour fixtures | First half complete in [#62](https://github.com/yuvaltal912-sys/Konnect/pull/62); owner authorized merge on 2026-11-10 (verify PR state). Second half (per-zone policy, stitching after refill), started by the owner on 2026-11-13: in progress on branch `claude/creation-s6b-pour-policy` (claimed 2026-11-13 by the creation supervisor's worker); GitHub is the source of truth for its PR's state |
| S7 | Stitching-via and corner clean-up planner, read-only (Q1: a second opinion, never the author) | merged | new `tools/route_refine.rs` with tests/fixtures; narrow `layout_quality*` reuse seams; routing registration/counts; `kicad-pcb` skill and API docs | Implemented, reviewed and gated on branch `codex/creation-s7-route-refine` (2026-11-13); publication and merge through [#63](https://github.com/yuvaltal912-sys/Konnect/pull/63), GitHub is the source of truth for its state. See "Row S7" |
| S7b | Acute-junction resolver: a further proposal kind for acute corners outside S7's three kinds | S7 | `tools/route_refine*.rs` | Authorized 2026-11-13; reserved for the creation supervisor's worker, not yet claimed |
| A1 | Replace the audit's recursive netclass pattern matcher with a linear one | S7 | `tools/layout_quality.rs` | Implemented, reviewed and gated on branch `claude/creation-audit-pattern-matcher` (2026-11-13); publication and merge through that branch's PR; GitHub is the source of truth for its state. See "Row A1" |
| S9 | Power and critical nets first, applied locked, then the rest | S5, S6, S7, the live apply check | `konnect-ipc/src/builders.rs`, `tools/pcb_routing.rs`, `freerouting_mcp.rs`, `tools/integration.rs` | Redesigned 2026-11-13 as width-first routing (pass engine in `route_specctra_dsn`, widths reported as exceptions); build authorized after S6's second half merges; reserved for the creation supervisor's worker |
| S10 | Only if signals stay above x1.03 after S6 and S9: re-route the worst nets | S9 | an evaluation script under `xtask/freerouting-bench/` first | Not started |
| S11 | Report zone/pad or zone/via reported-zero/short candidates; distinguish newly qualifying pairs after a valid refill comparison without establishing engine cause or routing correctness | merged | new `tools/zone_fill_defects.rs` and its tests, a few lines in `tools/drc.rs`, `TROUBLESHOOTING.md` | Implemented, reviewed and gated on branch `codex/creation-s11-zone-fill-defects` (2026-11-13); publication and merge through [#64](https://github.com/yuvaltal912-sys/Konnect/pull/64), GitHub is the source of truth for its state. See "Row S11" |
| B4 | Circuit review across sheets (#55/rank-29 follow-up): join supported hierarchy connectivity, retain instance identity and incomplete results for unresolved hierarchy | #55 and integration merged | `tools/circuit_model.rs`, `tools/circuit_review_tests.rs`, new hierarchy fixtures; narrow `tools/circuit_review.rs`, `tools/design_review.rs`, `tools/circuit_checks.rs`, `tools/design_review_circuit_tests.rs`; own shared-doc entries | Implemented on `codex/creation-cross-sheet-review` (2026-11-13): three review rounds, gate green. [#65](https://github.com/yuvaltal912-sys/Konnect/pull/65); GitHub is the source of truth for its merge state |
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

## Row S6, second half: per-zone pours and stitching after refill (PR #66)

**State.** Implemented on branch `claude/creation-s6b-pour-policy`, base `fork/main` `61b96dc`. The claim was published as draft [PR #66](https://github.com/yuvaltal912-sys/Konnect/pull/66). GitHub is the source of truth for this PR's state. The design rulings are in the decision table ("S6 second half design").

**What it adds.**

- **Per-zone `pours` in `export_specctra_dsn`.**
  - Besides `"fixed"` and `"omit"`, `pours` takes an object: an optional `default`, and a `zones` list of rules. Each rule has `uuid`, `name`, `net` and `layer` selectors, all of which must match, plus a `policy` of `fixed` or `omit`.
  - The unit is one pour on one copper layer, which is what the DSN writes, so `layer` can split a two-layer zone.
  - Refused: unknown keys and values, a rule with no selector or no policy, an empty selector, more than 256 rules, a non-copper layer, a rule that picks no pour (a rule area is named), and rules that disagree about one pour layer.
  - The manifest lists every pour layer with its policy and the rule that chose it (`pours`). The importer accepts that list and older manifests without it.
  - Code: `PourPolicy`, `pour_decisions` and the manifest rows, all in `specctra.rs`; the schema and argument reading in `tools/pcb_export.rs`.
- **Honest routing results in `route_specctra_dsn`.**
  - `freerouting_mcp.rs` reads Freerouting's own unrouted count from the job log (`router_unrouted`, the last autorouter or optimizer summary).
  - `tools/integration.rs` reads the planes from the DSN itself (`specctra::dsn_planes`) and names their zones from the manifest beside it.
  - New `status`: `complete` only with 0 unrouted and no plane; `incomplete` when the router left connections; `unverified` otherwise.
  - New `completion`: what decides (KiCad DRC after a refill), the exact `next_steps`, and, with planes, `if_still_open`. 1 to 10 open connections are reported as open; above 10, the steps fall back to `pours: "omit"`.
  - Code: `routing_status` and `completion_report` in `tools/integration.rs`.
- **Docs.** The `kicad-pcb` skill teaches the same sequence (Freerouting round trip, step 7). It, `tool-directory.md`, `DEV.md` and `docs/API_MIGRATIONS.md` now say what `fixed` really writes.

**For S9, which edits the same two files next.** What this row changed there:

- **`freerouting_mcp.rs`.**
  - `log_lines` was pulled out of `optimizer_report`. It turns the job log into message lines.
  - New `router_unrouted(log)` returns the last "(N unrouted" of an "Auto-routing stage completed" line or of an "Optimizer pass #k … was completed" line, or the N of "Skipping optimization stage: the board has N unrouted". It ignores pass lines and the line that starts a stage. It returns `None` when the log holds none of these.
  - New field `RouteEvidence.router_unrouted`. `run_state_machine` fetches `get_job_logs` once, and feeds both `optimizer_report` and `router_unrouted` from it.
  - Job settings, read-back and the isolation of the JAR are unchanged.
- **`tools/integration.rs`.**
  - New `routing_status(planes, router_unrouted)` and `completion_report(dsn, ses, planes, router_unrouted)`, which returns `status` and `completion`.
  - The constants `PLANE_FALLBACK_OPEN` (10), `STITCH_ROUNDS` (4) and `STITCH_RADIUS_MM` (3.0, the planner's default).
  - In `handle_route_specctra_dsn`: the planes are read from the DSN before routing (`specctra::dsn_planes`). `status` and `completion` are added after `success`. Every other field is unchanged.
  - Tests: `route_rule_tests`, including `assert_calls_are_real`. That helper checks every tool and parameter in `next_steps` and `fallback` against the registry and the tool's own input schema.
- **Where S9's statuses go.** Add `complete_with_width_exceptions` and `stopped` as arms of `routing_status`, ahead of `complete`.
  - One rule must stay: with a plane in the DSN, no "complete" status is ever returned, because Freerouting counts a pad inside a plane as connected.
  - Pass the last pass's unrouted count to `completion_report`. Per-pass detail belongs in S9's own `passes` field.
  - `next_steps` stays the KiCad-side sequence after the call. S9 should not add router passes to it.

**Known limits.**

- A pour form that blocks other nets has no per-zone switch in Freerouting 2.5.0: `plane_as_obstacle` is board-wide, and a DSN plane is read with `isObstacle = false`. Evidence: bytecode notes in the main checkout, `target/creation-research/resolution-evidence/notes/rank32-checks-backlog.md` §3 and `notes/routing-quality.md` (ignored files).
- The steps cannot run inside the routing call. Apply, refill and `route_batch` need the live board.
- The planner's stitching closes only the open connections it finds a safe via position for. On S5's plane boards, S7 measured 19 open falling to 4 or 5 at the default 3 mm radius.

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
| 4.1 | Per IC supply pin: capacitor → pin → ground → capacitor copper path length and plan-view area, ground side included |
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
