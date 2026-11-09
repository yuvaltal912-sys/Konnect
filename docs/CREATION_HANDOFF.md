# Creation handoff

The current state of the creation-side work: schematic, PCB and library tools. Anyone picking this up, human or agent, starts here. Update this file in the same commit that changes the state. The simulation track has its own file, [`SIMULATION_HANDOFF.md`](SIMULATION_HANDOFF.md).

**Last updated:** 2026-11-09. #58 (PR 16b) gained a follow-up: job settings under the names Freerouting 2.5.0 reads, checked with `get_effective_settings`; fanout always off; hole clearance for unplated holes; footprint rule areas and slots refused (section below). An independent review of every open PR followed (step 3 of the resolution plan); #58 then fixed its six verified findings: pad-less footprints, the single retry width, the 250 um edge sentinel, widening from no width, and optimizer timeouts. The owner approved the library-cap fix for #52 (`import_library_part` registered in `integration`), to land with #52's merge. main is 535bf2e: the 15 core creation PRs, PR 16 (#44), #45, #46 and #47 are merged. Open, all awaiting the owner's review: #48–#58 (list below). The owner's direction: routing quality on boards like his, measured against [`LAYOUT_CHECKLIST.md`](LAYOUT_CHECKLIST.md). Catalogue on main: 265 registered tools plus 7 meta-tools. Work stopped here at the owner's request; nothing is running.

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
| 3 | Response budget for ERC, DRC and list tools | none | Merged: [#21](https://github.com/yuvaltal912-sys/Konnect/pull/21). Leftovers (C24 `zone_fill_state`, A30's other long results) done in draft [#49](https://github.com/yuvaltal912-sys/Konnect/pull/49), waiting for review |
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
| 15 | ERC/DRC baselines | 3 | Merged: [#43](https://github.com/yuvaltal912-sys/Konnect/pull/43). Follow-up: `run_design_review` reads a DRC baseline, merged [#47](https://github.com/yuvaltal912-sys/Konnect/pull/47) |
| 16 | Widen the Specctra DSN profile so Freerouting fits real boards: back-side parts, more than 2 copper layers, then a Freerouting version policy and a benchmark on KiCad's demo boards (rank 32: D7, B34, D24). PR 2b already did rounded/arc outlines, roundrect pads and zones as fixed copper | 2b, 13 | Merged: [#44](https://github.com/yuvaltal912-sys/Konnect/pull/44). Part 2 (version policy, benchmark) in [#51](https://github.com/yuvaltal912-sys/Konnect/pull/51) |
| 16b | Route at the project's rules: no Freerouting neck-down, the importer enforces class widths, copper-to-edge clearance in the DSN, power widths from design-intent currents (IPC-2221) | 16, Q3, 11 | In progress (branch `feat/creation-route-at-project-rules`) |
| 29 | Circuit-aware review against the owner's layout checklist (sections 2, 6, 8, 9) and the power tree from the design-intent rails (rank 29: B22, B23 partly) | Q3, 11 | Draft [#55](https://github.com/yuvaltal912-sys/Konnect/pull/55), branch `feat/creation-circuit-aware-review`. Follow-ups: derating (B23), datasheet-driven values, hierarchy-joined nets |
| R21 | Board construction (rank 21: A13, B25): footprint lock/unlock, rule areas (keepouts) that survive the DSN export, zone delete, polygon board outlines | 2b | Implemented, draft [#48](https://github.com/yuvaltal912-sys/Konnect/pull/48) (branch `feat/creation-board-construction`); waiting for review and merge |

## Next: what to pick

**Rank 32 part 2 (B34, D24): ready for review, draft [#51](https://github.com/yuvaltal912-sys/Konnect/pull/51) (branch `feat/creation-freerouting-benchmark`).** `cargo xtask freerouting-bench` and the recommended Freerouting release in `check_freerouting`; see its section below.

**Claim rule.** Pick a "Not started" row whose dependencies are all merged. Before writing code, push a first commit on your branch that changes that row's state to "In progress (branch `<name>`)" and open a draft PR, so no other agent takes it. For a partially integrated row, claim its continuation on a new branch and draft PR before implementation. If another row is "In progress", don't touch the code files it names.

**Rank 3 leftovers: done in draft [#49](https://github.com/yuvaltal912-sys/Konnect/pull/49) (branch `feat/creation-rank3-leftovers`), waiting for review and merge.** C24 (`zone_fill_state` on DRC results) and A30's other long results. See "Rank 3 leftovers" under PR 3.

**Open PRs** (owner review; each PR's section or body names the files it owns, so don't touch them until it merges):

| PR | What | State |
|---|---|---|
| [#48](https://github.com/yuvaltal912-sys/Konnect/pull/48) | Board construction tools: footprint lock, rule areas, zone delete, polygon outlines (rank 21) | draft, gate green |
| [#49](https://github.com/yuvaltal912-sys/Konnect/pull/49) | Rank 3 leftovers: DRC `zone_fill_state`, paged long results | draft, merged with main, gate green |
| [#50](https://github.com/yuvaltal912-sys/Konnect/pull/50) | Inline layer and library renders, `inline_diff` difference image (rank 25) | draft, gate green |
| [#51](https://github.com/yuvaltal912-sys/Konnect/pull/51) | Freerouting version policy and quality benchmark (rank 32 part 2); recommends 2.5.0 on Java 25 | draft, gate green |
| [#52](https://github.com/yuvaltal912-sys/Konnect/pull/52) | Part import from local KiCad files (rank 33; no network: EasyEDA's terms) | ready, merged with main |
| [#53](https://github.com/yuvaltal912-sys/Konnect/pull/53), then [#54](https://github.com/yuvaltal912-sys/Konnect/pull/54) | Simulation M3: op-amp, comparator, LDO, reference; then load switch, eFuse, ideal diode (stacked: merge #53 first) | #53 ready, #54 draft; live suite 96/96 |
| [#55](https://github.com/yuvaltal912-sys/Konnect/pull/55) | Circuit-aware review against the layout checklist (rank 29) | draft |
| [#56](https://github.com/yuvaltal912-sys/Konnect/pull/56) | Layout quality audit `audit_layout_quality`: return paths, decoupling loops, bottlenecks, one comparable score | draft |
| [#57](https://github.com/yuvaltal912-sys/Konnect/pull/57) | Freerouting MCP started without telemetry and outside the home folder | draft |
| [#58](https://github.com/yuvaltal912-sys/Konnect/pull/58) | PR 16b, route at the project's rules (section below) | draft, gate green |

Several of these edit the same shared docs (`CREATION_HANDOFF.md`, `tool-directory.md`, `API_MIGRATIONS.md`) and the tool counts: whoever merges second merges main in, keeps every PR's entries, and reruns `cargo xtask fix-doc-counts`. #51, #57 and #58 all touch `freerouting_mcp.rs`/`integration.rs` in different functions.

**Owner direction (2026-11-08):** routing many traces is easy; routing them optimally is what he wants, on boards like his (power and mixed-signal, roughly 30–150 parts, 2–4 layers, pours, high-current nets). [`LAYOUT_CHECKLIST.md`](LAYOUT_CHECKLIST.md) is the standard: return paths, decoupling loops, protection and termination placement, widths for current. Completion is only a gate; he wants to give the currents of the main traces himself, through `set_design_intent`.

**Reference board:** a private, git-ignored copy of one owner-chosen board (100 parts, 2 layers) and its scripts are in `target/creation-research/reference-boards/` (`apply_ses.py` writes an SES into a board copy with KiCad's `pcbnew`, then refill and `kicad-cli pcb drc`). Keep its results out of tracked docs apart from counts.

**Layout quality audit: In progress (branch `feat/layout-quality-audit`, draft [#56](https://github.com/yuvaltal912-sys/Konnect/pull/56)).** Implemented and verified (see its section below); waiting for owner review. Files it owns: `tools/layout_quality*.rs`, `tests/fixtures/layout_quality/`, the registration line in `design_review::tools()` and its `tool_count` in `router/registry.rs`; `trace_current.rs` only gained `pub(crate)` on its sizing helpers.

**Recommended next**, after the owner approves:

1. Close the length gap, measuring first. With #58 and pours omitted from the DSN, the reference board routes clean but about 30% longer than its existing routing; with pours kept (the default), the ground pours come out split after refill (see PR 16b's follow-up). Most of the excess is the ground net: with pours omitted from the DSN and ground widened from the design intent, Freerouting routes it as wide track where the existing routing drops it into its pours with vias. Steps, each scored with `audit_layout_quality` (#56) plus length by net category: a settings sweep through #51's harness (`optimizer.*`, `optimizer_scoring.*`, `scoring.*`, with the optimizer's early stop off); then ground through the pour (no intent widening for pour nets, a per-zone pour choice in the DSN, stitching vias after refill); then power and critical nets first, locked, then the rest.
2. A detector for KiCad's fill defect (below, "Known traps"): flag zone copper a refill adds over another net's pad as a KiCad defect, not a routing error. Explained, no GUI check needed.
3. Rest of rank 32: custom pad shapes, footprint rule areas and Edge.Cuts slots (refused since PR 16b's follow-up, until modelled), local clearance overrides, copper text as exact outlines, per-net choice of which pours to keep; blind/buried vias parked.

PR 13's rule editing, deletion and static expression workflow is complete within
its documented limits. Wider custom-rule conversion into DSN remains part of the
later routing-profile expansion; unsupported rule semantics refuse export.

**Checks still owed** (not blocking, but say so if your PR depends on them):

- `apply_specctra_ses` of a Freerouting session into a live KiCad, then refill and DRC (PR 16 proved export, routing and the import plan; the live apply is not run).
- PR 9 against a live KiCad: `crates/konnect/tests/live_kicad_tools.rs`, on a disposable board.
- PR 14 against a live KiCad: checkpoint, edit in the GUI, restore refused while open, restore after closing, on a disposable project.
- Windows and Linux for the whole series, at the end.

**Known traps.**

- `every_simulation_demo_flattens_or_names_its_export_failure` is flaky when another test run is using `~/Library/Caches/KiCad` at the same time (IBIS demo, "Cannot find U1.cache"). Rerun it alone.
- Use the noreply identity from `AGENTS.md`, including merge commits. Since the owner turned on email privacy (2026-11-08), GitHub merges work with `gh pr merge N --merge --match-head-commit <full sha> --author-email 274043415+yuvaltal912-sys@users.noreply.github.com` (first used for #42); check the merge commit's `%ae`. If GitHub refuses, stop and ask: never substitute a personal email or fast-forward `main`. Pushes are refused when the branch tip carries a personal email, so put a noreply commit on top.
- Two sessions edited `CREATION_HANDOFF.md`, `tool-directory.md` and `API_MIGRATIONS.md` at once and conflicted each time. Keep every PR's rows and entries when you resolve.
- Three `freerouting_setup_tests` (fake Java probes) can fail under a full parallel `cargo test --workspace` and pass alone. Rerun them alone.
- Under heavy machine load (several sessions running tests at once, load average above 30) the IPC mock tests in `pcb_sync` and `custom_rule_tests` time out (`GetOpenDocuments … Timed out`). They pass alone; rerun them before calling a gate red.
- Freerouting 2.5.0 needs Java 25 (class file 69); Java 21 fails with `UnsupportedClassVersionError`.
- Freerouting 2.x turns telemetry and analytics on by default and writes its settings and log under the user's home (2.5: `~/Library/Application Support/freerouting`, `~/Library/Logs/freerouting`; 2.1: `java.io.tmpdir/freerouting`). The MCP spawn in `freerouting_mcp.rs` (used by `route_specctra_dsn` and by the `check_freerouting` MCP probe) now passes `--profile.allow_telemetry=false`, `--usage_and_diagnostic_data.disable_analytics=true`, `-Djava.awt.headless=true`, a scratch `-Duser.home` and `-Djava.io.tmpdir`, and `--user_data_path` pointing at the scratch home. The scratch folder lives until the child has exited, on `close` and on drop. The child also loses every inherited `FREEROUTING__*` variable, and `XDG_*_HOME`, `APPDATA` and `LOCALAPPDATA` point into the scratch folder, because 2.5.0 reads those ahead of `user.home`. `check_freerouting` also runs `java -jar <jar> --version` (`tools/integration.rs`). 2.5.0 has no such flag, so that run starts fully in the real home with telemetry on until it is killed. It is not isolated until #51 lands. 2.5.0 also starts a version check on every start, with no switch to turn it off: one GET to `api.github.com/repos/freerouting/freerouting/releases/latest` through Java's default proxy selector. The spawn blocks it with `OFFLINE_JVM_ARGUMENTS`: every HTTP and HTTPS request goes to a closed loopback proxy (`127.0.0.1:9`), and loopback stays direct (`http.nonProxyHosts`). Checked live on 2.5.0 (2026-11-09, isolated home, telemetry off): the MCP handshake and tool list answer, no connection leaves loopback, and the scratch log says `Version check skipped or timed out: java.net.ConnectException`. Checked live on 2.5.0: the log in the scratch home says "Analytics are disabled" (absent without the flags), and the settings file and log land in the scratch home. That live check predates `--user_data_path` and the environment clean-up, which were read from the 2.5.0 bytecode and covered by fake-Java tests only. Do not seed a `freerouting.json` there: 2.3+ crash with a NullPointerException unless it has a `profile.id` UUID, and the flags make a seed unnecessary. The JSON Freerouting writes still shows `allow_telemetry: true`; the log line is the evidence, not that file. A new place that starts the JAR must reuse `jvm_arguments`, `server_arguments`, `user_data_path_argument` and `isolate_environment` in `freerouting_mcp.rs`.
- Freerouting 2.x turns telemetry and analytics on by default and keeps its settings and log in the user's home (2.5 under `~/Library/Application Support/freerouting` and `~/Library/Logs/freerouting`). Pass `--profile.allow_telemetry=false --usage_and_diagnostic_data.disable_analytics=true` and a scratch `-Duser.home`; `xtask/freerouting-bench/quiet-java.sh` does both. 2.5.0 also sends a version check to `api.github.com` on every start, with no switch; the `--version` probe, the bench and `quiet-java.sh` send all HTTP and HTTPS to a closed loopback proxy (`127.0.0.1:9`), so it fails at once and loopback stays direct. `route_specctra_dsn` and the MCP probe in `check_freerouting` (`freerouting_mcp.rs`) do the same since #57 (the item above).
- Freerouting 2.3 and later stop with a `NullPointerException` when their `freerouting.json` has no `profile.id`.
- Freerouting 2.5.0's job API (MCP `update_job_settings`) drops unknown keys without an error. It reads the snake_case names its settings serialize under (`neck_width_um`, `copper_to_edge_clearance_um`, `job_timeout`, `autorouter.max_passes`); camelCase was silently ignored until PR 16b's follow-up. Read the settings back with `get_effective_settings`, as `route_specctra_dsn` now does.
- A `freerouting.json` in the real home (`~/Library/Application Support/freerouting/`) fills every setting a run leaves out, and Freerouting writes settings back to it. Any run without an isolated home inherits hidden settings, which is how the dropped keys stayed hidden. Benchmark and route only with an isolated home (#51, #57).
- `optimizer.improvement_threshold` is a percentage, but a value between 0 and 0.1 is read as a fraction and multiplied by 100.
- KiCad 10.0.8's iterative zone refill can pour a lower-priority zone that overlaps a higher-priority zone of the same net over other nets' pads (seen at 0 mm on two connector pads; the copper at those pads was identical in a clean run). `ZoneFillIterativeRefill=0` in `~/Library/Preferences/kicad/10.0/kicad_advanced` avoids it for the GUI, IPC and kicad-cli; KiCad's standalone Python (`pcbnew` scripts) ignores that file. Run DRC after every refill.
- A scratch `KICAD_CONFIG_HOME` needs copies of the global `fp-lib-table` and `sym-lib-table`, or KiCad's library checks change and a DRC baseline no longer matches.
- Freerouting 2.5.0's fanout stage (on by default) necks a track down at the pin even with `automatic_neckdown` off; the importer then refuses the SES (seen as 0.15 mm wires in a 0.2 mm class). The owner's real-home `freerouting.json` turns fanout off, so every run that inherited it looked clean; the first isolated-home run did not. `route_specctra_dsn` sends `fanout.enabled: false`.
- Freerouting 2.5.0 treats a `copper_to_edge_clearance_um` within 1e-9 of 250 as its own default (`HeadlessBoardManager.applyCopperToEdgeClearanceOverride`): it skips the override, or clamps it to the outline's minimum pin gap, logging at debug only, while `get_effective_settings` still reports 250. A project asking for exactly 0.25 mm would route closer to the edge than asked. `route_specctra_dsn` sends 250.001.
- Freerouting 2.5.0's retry width (`neck_width_um`) is one value for every net, applied to any failed connection (`AutorouteConnectionRouter.retryConnectionNecked`, no per-net filter). Send it only when no net's floor is above it.
- Konnect's Freerouting JAR search does not look in `~/.local/share/freerouting/<version>`; pass `jar_path`.

## PR 16b, route at the project's rules (draft, #58)

Branch `feat/creation-route-at-project-rules`, draft [#58](https://github.com/yuvaltal912-sys/Konnect/pull/58). The claim-only commit and draft preceded implementation, from main 535bf2e. Found by routing the owner's reference board (100 parts, 2 layers, the class of board he builds) with Freerouting 2.5.0; its private copy and scripts are in git-ignored `target/creation-research/reference-boards/`.

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

## Layout quality audit (draft, #56)

`audit_layout_quality(board, …)`: the measuring stick for "optimal" routing, in the `design_review` toolset (8 tools now; 260 + 7 meta). Read-only, from the saved board file (zone fills are read, so refill and save first). Code in `tools/layout_quality.rs` (tool, thresholds, net roles, scoring) with `_grid` (a raster of one net's copper: connectivity, area, shortest and widest paths), `_planes` (§3), `_power` (§4, §8) and `_routing` (§5, completion); tests in `layout_quality_tests.rs` over six boards `pcbnew` wrote (`tests/fixtures/layout_quality/`, `make_fixtures.py` regenerates them with KiCad 10.0.8's own Python).

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

## Rank 32 part 2, Freerouting version policy and benchmark (draft, #51)

B34 and D24, from main 966a89d, merged with main after #44 to #47. Files: `xtask/src/freerouting_bench/` (new), `xtask/freerouting-bench/` (corpus script, quiet Java wrapper), `freerouting_setup.rs`, the `check_freerouting` handler in `integration.rs`. Usage is in `DEV.md`, "Freerouting benchmark (opt-in)".

- **`cargo xtask freerouting-bench`**, opt-in, never in CI: any folder of DSN files, one or more JARs, two runs each. Completion is a gate (0 unrouted, 0 clearance violations, within Freerouting's own job budget); boards that pass get quality: wiring over an any-angle lower bound (half the pins' convex hull) and over HPWL, vias and vias per net, layer changes, acute and 90° corners away from pads, tracks, segments below their net class width, runtime, determinism (same copper in any order). Ranking: gate failures, then defects, wire/bound, vias, determinism, runtime, over the boards every competing version passed. Each run has its own `user.home` and temp folder with telemetry off. `--rescore` re-measures a finished run without Java. Results stay under `target/freerouting-bench/`.
- **`check_freerouting`** reports `version_policy`: recommended 2.5.0 (needs Java 25), its URL, SHA-256, the measured reason, and a warning when the installed JAR is another version or not the published file. Its `--version` probe now runs headless with telemetry off in a scratch home: 2.1.0 used to open its window and time out, so a working install read unavailable.
- **Measured (2026-11-08, Java 25, 120 s budget, all four releases):** on 10 boards exported by Konnect with pours omitted (5 KiCad demos of 15 to 114 parts, 5 fixtures), 2.5.0 passed the gate on 10, 2.1.0 on 7, 2.3.0 and 2.4.1 on 5 each. 2.5.0's two runs matched on all 10, 2.1.0's on 2 of 7. On the 5 boards all passed, 2.5.0 and 2.4.1 tied for shortest wiring (1.195 x the bound) and fewest vias. A second corpus from KiCad's own exporter (pours kept) also ranked 2.5.0 first, with no release completing 4 of its 12 boards. Caveat: 2.5.0 necks traces down near pads (202 segments below class width on 4 boards; 2.1.0 had none, but 13 acute corners).

Verified on macOS: the gate after merging main (fmt; 3,813 lib and integration tests, one of them first red on this PR's own skill wording and green after the fix; doc tests; clippy `-D warnings`; `cargo deny check licenses`; `fix-doc-counts` unchanged; release build; `simulation_live` skipped, not touched), a stdio run of the release binary (`check_freerouting` on the 2.1.0 KiCad plugin JAR: available, warned with the measured reason; on 2.5.0: matches, digest verified, no warning), 38 xtask unit tests without Java (DSN and SES readers, pin placement incl. back side, hull bound, corners, layer changes, widths, gate, ranking, report and JSON), version-policy tests over fake JARs and Java scripts, the ignored check of the pinned digest and Java 25 against the real 2.5.0 JAR, and the real benchmark runs above. Not verified: the owner's own boards (the corpus still comes from KiCad's demos), the boards' real net-class widths in the Konnect-exported corpus (PR 16's probe writes a flat 0.2 mm), Windows and Linux.

Follow-ups: start Freerouting quietly in `route_specctra_dsn` and the MCP probe (`freerouting_mcp.rs`); the owner's board corpus; a setting sweep (passes, optimizer) on it with the same metrics.

**Review follow-up (2026-11-09).** Seven verified findings fixed, each with a test that fails without it:
- The `--version` probe fails closed: no scratch folder means the JAR is not started (`available` and `version_checked` false, `note` names the error). It also removes inherited `FREEROUTING__*` variables and pins `XDG_*`, `APPDATA` and `LOCALAPPDATA` into the scratch folder. `probe_local` in `freerouting_mcp.rs` is untouched here; its isolation comes with #57.
- A version line followed by a Java exception trace or a Freerouting ERROR line (not the expected "no input/output file" complaint of a board-less headless run) is a start-up crash, not `available`.
- The bench trusts 2.3+'s own `finished with state:` line over the wall clock for the job budget; the wall clock stays the test for 2.1.
- Each `--jar` gets a unique folder label (`-2`, `-3` on a clash, `unknown` too), saved as `dir` in `results.json`; `--rescore` reads it (old files fall back to the file-name label), and totals and ranking are per folder with the version shown.
- A run that does not report its clearance-violation count fails the gate.
- Versions compare by zero-padded numbers (`2.5` is `2.5.0`); `-SNAPSHOT` and other pre-release suffixes are kept and never the release.
- The bench seeds `home/.config/freerouting` (where 2.5 reads on Linux) and pins the `XDG_*`, `APPDATA`, `LOCALAPPDATA` folders into the run folder.

Verified (review follow-up): fmt; 3,838 lib and integration tests passed, 0 failed (the 46 xtask tests included); 9 doc tests; clippy `-D warnings`; `cargo deny check licenses`; `fix-doc-counts` unchanged; 19 negative controls, each guard broken in source, its test failed, restored byte for byte; one real 2.5.0 probe run the way `check_freerouting` starts it (headless, scratch home, folders pinned): banner, two WARN lines and the expected no-board complaint, so a healthy start, and every file it wrote stayed in the scratch home (its output is now a test); `simulation_live` and the release build skipped. Not verified: the bench against a real JAR (its helpers are unit-tested), Windows and Linux paths.

## Rank 29, circuit-aware review (draft #55)

`audit_circuit_design(schematic, board?)` in the `design_review` toolset (8 tools; 260 + 7 registered), run by `run_design_review`. Code: `circuit_model.rs` (parts, pins, nets of one use of a sheet; board pad distances), `circuit_checks.rs` (the checks), `circuit_review.rs` (rule table, accounting, power tree, feedback), role classifiers in `component_class.rs`. The standard is the owner's layout checklist (`target/creation-research/layout-checklist.md`, git-ignored). Written from the checklist and pin names; nothing is ported from kicad-happy, so `THIRD_PARTY.md` is unchanged.

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

## Rank 21, board construction (draft, #48)

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

## PR 15, ERC/DRC baselines (merged, #43)

Three tools in the `verification` toolset (15 to 18), in `tools/check_baselines.rs`. Finding C4: agents diffed ERC/DRC reports by hand in Bash 18 times to prove an edit broke nothing.

- `save_check_baseline` runs ERC (`schematic`) or DRC (`board`, with `run_drc`'s `sync_live_board` and `refill_zones`) and stores every violation at `<project_dir>/.konnect/baselines/<erc|drc>/<name>.json`, beside the visual baselines. An existing name is refused before the check runs (a synced DRC saves the board) unless `overwrite`. Create-only or revision-conditional write, then read back.
- `compare_check_baseline` loads the baseline first (a wrong name runs nothing and lists the stored names), runs the same check and lists `new`, `severity_changed` and `resolved`, paged through the PR 3 helper, with `unchanged_count`, per-rule tallies and `verdict` (`regressed` when anything is new or more severe). `output` writes everything, unchanged included. Warns when the DRC options or the ERC's KiCad version differ from the baseline's.
- `list_check_baselines`: both kinds, newest first, `source_state` (`unchanged`, `changed`, `missing`); unreadable files listed apart.

Stable keys: a hash of check, DRC category, rule, ERC sheet and the sorted item identities. Severity, prose and positions are not in it. An item is its UUID only when the UUID is in the saved design (DRC: ownership enrichment resolved it on the board; ERC: it occurs in a project `.kicad_sch`). Otherwise it is its description with measured values (`0.15 mm`, `45°`) masked, which covers KiCad 8 boards whose pads have no saved UUID. Repeats are numbered `~2`, `~3`. A DRC category that only one run reported is listed in `categories_not_compared`, never as resolved. `run_erc` and the baselines now share `sch_export::erc_report`, so both refuse a sub-sheet the same way.

Verified on macOS: 23 unit tests over scratch projects with a stand-in `kicad-cli` (new/resolved/unchanged for ERC and DRC; a re-run reordered, moved, with items swapped and DRC prose re-measured matches exactly; made-up UUIDs; repeats; severity changes; missing categories; refusals before any run; damaged and renamed baseline files; a symlinked `.konnect`; 1,500 changes under the client cap with paging covering each once), 12 negative controls each caught by a test, and the ignored live test `live_rerun_of_an_unchanged_project_matches_its_baseline` with kicad-cli 10.0.8 on a scratch copy of the KiCad 10 `pic_programmer` demo (5 ERC and 5 DRC violations, every item keyed by UUID, re-run 0 new and 0 resolved). `cargo test -p konnect-core -p konnect` and clippy pass. Not verified: a live edit between save and compare, `sync_live_board` against a live KiCad (it reuses `drc::run`, already covered), a KiCad 8 board, Windows and Linux.

Follow-ups, not in this PR: read baselines in `run_design_review` (draft #47, section above); a delete tool (overwrite covers replacement today); the documented edit loop in the skills and agents (rank C29).

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

A caller that read every violation in one `get_drc_violations` or `run_erc` call must now follow `next_offset` or pass `output`; `docs/API_MIGRATIONS.md` says so.

**Rank 3 leftovers ([#49](https://github.com/yuvaltal912-sys/Konnect/pull/49), draft).**

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
