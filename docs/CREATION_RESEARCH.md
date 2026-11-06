# Creation-side research: schematic, PCB and library tools

**Date:** 2026-11-06. **Scope:** Konnect's creation side (schematic, PCB layout and libraries). Simulation is a separate track and is out of scope. **Method:** four read-only research tracks run in parallel (A codebase, B landscape, C real use, D methods), producing 136 findings. Every theme was then checked twice by independent agents, once for "does the evidence hold?" and once for "does Konnect already cover this?". A critic pass looked for gaps and privacy leaks. Nothing was built or run, and no Konnect tool was called. The research brief is [`CREATION_RESEARCH_PROMPT.md`](CREATION_RESEARCH_PROMPT.md).

Item IDs (A3, C19, …) refer to the findings files in `target/creation-research/`, which are git-ignored and stay on this machine.

## 1. Summary

The ten changes that matter most:

1. **Prove the drawn circuit is the intended circuit.** Add a hierarchy-wide netlist read plus a compare against an intended `{net: [ref.pin]}` (A3, B14, D11). Nothing does this today, yet both agents are told to check it.
2. **Stop schematic wiring from silently merging nets.** `connect_pins` and its siblings draw a blind L-route and drop a junction on every pin they cross (A2). Upstream PR #901 fixes most of it.
3. **Keep ERC/DRC results inside the client's limit.** Eight real ERC/DRC results were rejected for size (51–71k characters), which breaks the main verification loop (C3, A30, C26).
4. **Make the inline schematic render visible to the model.** It returns base64 inside JSON text instead of an image block (C5). The fix is small: two handlers plus a shared image helper.
5. **Fix the shipped guidance.** The PCB skill never names the Freerouting route path or atomic `set_component_placements`. The schematic skill calls `save_project`, which saves only the PCB. No skill names any hierarchy or bus tool, or loads those toolsets (A7–A10, A33).
6. **Add a spatial board query.** Real sessions made 106 read-only Python runs on board files because no tool answers "what is near this pad" (C19).
7. **Check that footprints sit inside the real board outline.** The review checks test only whether the string `Edge.Cuts` exists. `score_placement` checks courtyard boxes, but only against a provably rectangular outline, so the rounded outlines Konnect itself draws come back unproven (C17, A14).
8. **Give placement a real objective.** `score_placement` has no wirelength term. The existing net model is dominated by power nets, so any new metric needs fanout filtering (A14, D2, D1).
9. **Let schematic-to-board sync finish the job.** Today a person must run KiCad's Update PCB by hand to replace footprints, delete orphan footprints and copy fields (C7).
10. **Keep pins on their wires after a symbol swap.** `replace_component` doesn't check that pins still meet their wires, and mirror can be set only when a symbol is added (C8). Mirroring after placement is a small PR; a pin-aligning replace is the real fix.

**Recommended first PR:** item 1, the netlist read and intent compare. It is small (the parser and the kicad-cli wrapper exist), it closes a real gap on its own, and it becomes the acceptance check for every later schematic PR, including the route-safety fix in item 2.

## 2. Where the creation side stands

**Catalogue** (from `registry.rs` and `tool-directory.md`): the registry held 22 toolsets and 235 tool entries plus 7 meta-tools when this was written, before PR 1 added two. The creation side was 16 of those toolsets, holding 177 of those tools:

| Area | Tools | With a direct test* | Named in shipped skills/agents |
|---|---|---|---|
| Schematic (7 of them) | 93 | 79 | 25 |
| PCB (5 of them) | 67 | 53 | 38 |
| Library | 17 | 12 | 14 |
| Review side (templates, design_review, verification) | 20 | 14 | 13 |
| **Total** | **177** | **144 (81%)** | **77 (44%)** |

\*A heuristic: a handler or tool name referenced in a test. Indirect tests are not counted. The `integration` (9 tools, which includes Freerouting), `project` (7) and `manufacturing` (3) toolsets are not in this table.

- None of the 14 `sch_hierarchy` tools or the 4 `sch_bus` tools is named in any skill or agent.
- The reliability contract's `outcome` envelope covers 4 of the 177 creation tools.
- There are 0 `TODO`/`FIXME` markers. Unfinished paths show up as explicit refusals instead, such as `move_connected`.
- Upstream (archived) has 201 open issues and 63 open, unmerged PRs. Upstream `main` differs from the fork point only in its README.

**Real use** (call log and 15 transcripts, 2026-08-27 to 2026-11-06, installed binary):

- About 94% of the 28,864 call-log records are unit-test traffic, because in-crate tests write to the real log (C1). That leaves 1,725 real-use records, and their non-ok rate is 2.6%.
- The transcripts hold 693 Konnect calls against 813 Bash calls. That includes about 190 Python runs on schematic and board files: about 151 read-only and 39 that write. These were classified by regex, so the exact count depends on the classifier.
- The client rejected 8 ERC/DRC results and 2 inline schematic renders for size.
- Most of the board-side evidence comes from rework of one existing board. Requirements, part selection, stackup, first placement and hierarchy creation have little real-use signal, so the lack of a track-C item there does not mean there is no pain.

## 3. Ranked backlog

Impact means impact on the finished design. Evidence from real use (C) weighs more than landscape (B) or papers (D). Impact, effort and confidence reflect the verification pass, not the original estimates. Merged duplicates share one row.

| Rank | IDs | Title | Kind | Area | Impact | Effort | Confidence | Evidence |
|---|---|---|---|---|---|---|---|---|
| 1 | A3, B14, D11, C20 (netlist half) | Hierarchy-wide netlist read plus intent compare (`get_schematic_netlist`, `compare_netlist`) | new_tool | schematic | high | S–M | confirmed | `sch_export.rs:113-143` writes a file only; `pcb_sync.rs:1070` parser and `simulation/snapshot.rs` export are reusable; 9 real netlist-diff Bash calls |
| 2 | A2, D10 (core) | Refuse or re-route schematic wiring that would merge nets; report `nets_joined` | upgrade_tool | schematic | high | M | confirmed | `sch_wiring.rs:522-562`; upstream #887 repro, PR #901 |
| 3 | C3, A30, C26, C24 | Shared response budget (summary by rule, limit/offset, byte cap, full report to file) for ERC, DRC, `get_drc_violations`, sync and list tools; stale-fill hint (C24, partly covered by `refill_zones`) | upgrade_tool | cross | high | S (ERC/DRC), then M (list tools) | confirmed | 8 ERC/DRC results rejected at 51–71k characters; `sch_export.rs:146-167`; reuse the `listing()` helper in `simulation.rs:2397` |
| 4 | C5 | Inline schematic render (and `compare_visual_baseline` diff) returned as an MCP image | upgrade_tool | schematic | high | S | confirmed | `sch_export.rs:307-312`; 2 renders rejected (118k and 325k characters) |
| 5 | A10, A7, A8, A9, A33, A20, C29, D25 | Guidance fixes: Freerouting path, `set_component_placements`, no `save_project` in the schematic flow, hierarchy/bus section, build-agent toolsets and power rule, the stale back-side placement message (A20; `flip_component` already covers the job), the verify loop (C29) as tools land | workflow_or_skill | cross | high | S | confirmed | `kicad-pcb/SKILL.md:89-121`; `project.rs:60-68`; skills match `crates/konnect/assets`; D25 is mostly covered by the existing `AuditFinding` |
| 6 | C19, then C18, C23, C15 | Spatial board query (`query_board_region`) plus `touching_pad`/`limit` filters on `query_traces`. Follow-ups on the same geometry layer: free-space finder (C18, M), copper-island report (C23, low, since DRC `unconnected_items` partly covers it), and planned-copper clearance check (C15, L) | new_tool | pcb | high | M | confirmed | 106 read-only Python runs on board files; `query_traces` up to 70 KB |
| 7 | C17, A14 (containment) | True polygon/arc outline containment in `score_placement` and the design review | upgrade_tool | pcb | high | S–M | confirmed | `design_review.rs:521`, `manufacturing.rs:1021` test only for the `Edge.Cuts` string; a part outside the outline was found by eye |
| 8 | A14, D2, D1 | Wirelength/ratsnest and crossing terms in `score_placement`, with fanout-filtered nets | upgrade_tool | pcb | high | M | confirmed | no wirelength term in `placement.rs`; in the deprecated refiner's spring model, power nets carry about 85–89% of the weight on the demo fixture. Make the fanout threshold configurable; the fixed 8 in `placement.rs:2037` gives way to pin adjacency for decoupling (rank 22) |
| 9 | C7 | `update_pcb_from_schematic`: copy all fields and delete orphans (cheap), then in-place footprint replace and routed-pad reassignment (large) | upgrade_tool | pcb | high | S then L | confirmed | `pcb_sync.rs:797`, `:841`, `:878`; the owner ran KiCad's Update PCB by hand |
| 10 | C8 | `mirror` on `rotate_schematic_component`; later, pin-aligning `replace_component` | upgrade_tool | schematic | medium | S (M for replace) | confirmed | `sch_components.rs:56` (add-time only), `:9392` (#450) |
| 11 | A16, B21 | Symbol-pin vs footprint-pad check before layout | new_tool | library | high | S | confirmed | today the mismatch surfaces only at sync; both readers exist |
| 12 | D17, C16 | IPC-2221/2152 trace current audit per net | new_tool | pcb | high | M | confirmed | a 16 KB private current-audit script, edited 10 times |
| 13 | D16, A32, B38 (rules) | Validated `.kicad_dru` custom-rule writer with a DRC dry run. Plus the `.kicad_pro` fields: edge, silk and annular minima in `set_design_rules`, netclass diff-pair fields, and `delete_netclass` (ports of #965/#966, #866; S) | new_tool | pcb | high | M | confirmed | only per-layer width/clearance is writable; `add_design_rule` stores English; the DSN export refuses any `.kicad_dru` (`pcb_export.rs:751`) |
| 14 | D15, D20 | Make the review honest: `audit_manufacturing` outline and width checks can never fire on KiCad 6+ boards; pin-type classification | upgrade_tool | cross | medium | S | confirmed | `design_review.rs:521` (outline check), `:513` (fab values unused); checks go by name prefix |
| 15 | A11, A12, C10, C11 (in-place modify) | Routing with readback KIIDs, pad-net and netclass-width checks, polyline/batch tracks and vias in one commit, via query/delete, in-place `modify_trace` | upgrade_tool | pcb | medium | M | confirmed | runs of 42 `route_trace` calls; `pcb_routing.rs:1279` delete-and-re-add |
| 16 | A15 | `create_symbol` fields (Footprint, Description, keywords), `edit_symbol`, delete on tab-indented libraries, refusal of duplicate names | new_tool | library | high | M | confirmed | `library.rs:223-291`; upstream PRs #909, #949 |
| 17 | C4 | ERC/DRC baseline save and compare (new, resolved, unchanged) | new_tool | cross | medium | M | confirmed | 18 hand-diff Bash calls |
| 18 | C21 | Restorable project checkpoints (`snapshot_project` only writes PDFs) | new_tool | cross | medium | M | confirmed | 51 backup/zip Bash calls; "undo what you did" |
| 19 | B35, A34 | DNP / in-BOM / on-board flags, library flags copied on placement, and title blocks | new_tool | schematic | medium | S–M | confirmed | grep finds 0 DNP setters; upstream PRs #816, #943/#944 |
| 20 | A24, C9 | Footprint field-text placement on the board (Reference/Value position, layer, visibility) | new_tool | pcb | medium | M | confirmed | silk-vs-edge warnings left for the owner; upstream #829 |
| 21 | A13, B25 | Board construction: footprint lock, rule areas, zone delete, polygon outlines | new_tool | pcb | high | M | confirmed | `pcb_board.rs:1059-1176`; upstream PR #910; rule areas gated on the #722 Specctra round trip |
| 22 | D3, D22 | Pin-level decoupling distance (not the IC bounding box) | upgrade_tool | pcb | high | M | likely | `placement.rs:2037-2128` |
| 23 | C14 (reduced), C13 | Edit-readiness report: which editor owns which file, and lock presence. Fail fast once per session when KiCad is closed (C13; partly a guidance question) | new_tool | cross | medium | M | likely | 7 owner state reports, 10 assistant requests, 39 lock or editor-state checks. Drop stale-lock clearing: KiCad locks have no PID |
| 24 | A18 | Search by description, keywords and tags, ranked, with `truncated` and skipped-library reporting; `search_footprints` ignores project libraries | upgrade_tool | library | medium | S | confirmed | `library.rs:4707-4795` |
| 25 | C6, A25, B9, B7 | Render region and zoom (schematic); side, zoom and layers (board); library-item renders | upgrade_tool | cross | medium | M | confirmed | real sessions bypassed the tools for crops, back-side and close-up views (the count depends on the corpus); risk: `konnect-render` refuses visible text |
| 26 | A5 | Power symbols by pin and in batch, `#FLG` numbering | upgrade_tool | schematic | medium | S | confirmed | upstream PRs #782, #857 |
| 27 | A4 | `apply_connections`: many nets in one atomic call | new_tool | schematic | medium | M | likely | build agent `maxTurns: 40`; `batch_connect_pins` already covers wire-style multi-net wiring, so the gap is label-style nets |
| 28 | A28, D12 | Address pins by name when the name is unique | upgrade_tool | schematic | low–medium | S | likely | `sch_wiring.rs:369-382` |
| 29 | B22, B23 | Circuit-aware review: power tree, feedback dividers, crystal load caps, ESD per connector, then derating | upgrade_tool | schematic | high | M–L | likely | none in `design_review.rs`; MIT precedents (kicad-happy) |
| 30 | A26, A6 (block move) | Overlap checks that include labels and field text; rigid block move with wires | upgrade_tool | schematic | medium | M | confirmed | upstream PRs #815, #934; a true wire-stretching drag was declined upstream |
| 31 | A35, A29 | Hierarchy-wide annotation; net queries on netlist connectivity with `not_found` and label UUIDs | upgrade_tool | schematic | medium | M | confirmed | upstream #463, #860, #746 |
| 32 | D7, then B34, D24 | Widen the Specctra DSN profile: zones, back-side parts, more than 2 layers, arc outlines. Then a Freerouting version policy (B34) and a benchmark corpus built on KiCad's demo boards (D24) | upgrade_tool | pcb | high | L | confirmed | `pcb_export.rs:257-260`; without it, item 5's Freerouting advice fits few real boards |
| 33 | B20 | Import symbol, footprint and 3D model by LCSC number | new_tool | library | high | L | likely | four peer servers do it; easyeda2kicad (AGPL-3.0) and JLC2KiCad_lib (MIT) |
| 34 | A22, C30 | Tests for 21 untested primitives; extend the kicad-cli load test to every schematic writer | infrastructure | cross | medium | M | confirmed | the coverage script; `e2e_kicad.rs` |
| 35 | C1, C2 | Keep tests out of the real call log; one `write_all` per record | infrastructure | cross | medium | S | confirmed | `handler.rs:89`; 1,028 corrupt lines |
| 36 | C11 (drag) | Move a footprint and stretch its attached tracks | new_tool | pcb | medium–high | L | confirmed | 26 tracks re-attached one by one. Needs the owner decision in §5 |
| 37 | B26, D19 | Return-path and reference-plane audit | new_tool | pcb | medium | M–L | likely | none today; antonmadto (MIT) rules |
| 38 | B24 | Derive net classes and routing rules from the schematic | new_tool | pcb | medium | M | likely | heuristic; needs the design-intent store (§5) |
| 39 | C12 | Batch 3D-model assignment | upgrade_tool | pcb | low–medium | S–M | likely | 126 calls for 46 footprints; partly covered by library update |
| 40 | A27, B15, D13, B16 | Schematic free-space finder, anchor placement, readability score and lint | new_tool | schematic | medium | M–L | likely | inferred from guidance; no real overlap failures seen |
| 41 | A31 | Move batch creation tools onto the shared `outcome` envelope | infrastructure | cross | medium–low | M per family | confirmed | `reliability-legacy-inventory.json` |
| 42 | A19 | Delete the dead `SchematicBuilder` and fix the skill text (S); move splice writers onto the typed model (L) | infrastructure | schematic | low–medium | S / L | confirmed | `schematic_builder.rs` has no callers |
| 43 | C22, B8 | Read legacy footprint stroke syntax; `kicad-cli fp upgrade` path | upgrade_tool | library | low–medium | S | likely | `footprint_graphics.rs:330-347` |
| 44 | D14, B13 | Netlist-to-schematic layout planner (dry run only) | new_tool | schematic | medium | L | likely | needs the owner decision in §5 |
| 45 | B4, B5, B6, B3, B1 | KiCad 10 variants, jobsets, board stats, proto refresh, DRC-marker injection (B1, mostly covered by `mutate_editor_selection`) | new_tool | cross | low | S–L | confirmed | parked by `ROADMAP.md:112-114` |
| 46 | A21 | Templates: fix the connection maps (upstream PR #867) and wire them through rank 27, or describe them honestly; add a per-template netlist test using rank 1 | upgrade_tool | schematic | medium | M | confirmed (track A only, not panel-checked) | `templates.rs:243-245` vs `:569-571`; non-existent pin names in one template; 3 tests |

**Process item: A1, harvest upstream.** About 20 of the 63 open upstream PRs map onto rows above (#901, #782/#857, #816, #909/#949, #910, #943/#944, #934, #815, #965/#966, #866, #867). Port them one at a time with provenance. They are the cheapest route to rows 2, 13, 16, 19, 21, 26, 30 and 46. Licensing is not a blocker: code submitted to the AGPL repository stays AGPL, and #947 concerns only upstream's own closed-source relicensing. Upstream will never merge these PRs, so the fork owns them. Expect conflicts on `tool-directory.md`, the registry and plugin metadata every time, and five candidates already need a manual rebase.

**Dropped after verification:**
- A23 (alias cleanup): mostly done already.
- B2 (IPC connectivity queries): already covered by the live board seam and DRC `unconnected_items`.
- B10 (KiCad 11 native DSN): the premise was wrong, since the default exporter is Rust.
- D5 (legalization): it would serve only retired planners.
- B36 (text-edit fallback): replace it with a one-line rule to report the tool gap and give the GUI steps (carried in PR 5).
- C25 (wire-chain analysis): `get_connected_items` already traces each pin.

**Not ranked (parked, below the cut, or watch items):**
- Second-opinion planners (allowed by §5 question 1; they start after PRs 1, 7 and 8): D8, B32 and D9 (PCB routing planners), D10 beyond rank 2 (schematic A* router), D4 and B29 (annealing and planning placers), D6 (trial-route check, also gated on rank 32).
- Gated on A21 (rank 46) being fixed first: D21 and D26 (reference topologies), because the templates are wrong today. B31 (design blocks as templates) goes with them.
- Below the cut, low or unverified: A17 and B18 (IPC-7351 generation; KiCad's stock generated footprints cover most cases), A36 (copy-routing and diff-pair), B17 (min-cut connectivity repair), B19 (KLC checks, mostly covered inside `create_footprint`), B27 (stitching vias), B28 (teardrops and fiducials), B30 (replicate repeated channels), B37 (datasheet library), B39 (MCP annotations), B40 (embedded files), B41 (pin and gate swap), B42 (KiKit panelization), D18 (impedance estimator), D23 (length tuning).
- Quick wins that can slot in anywhere: C27 (path-parameter aliases), C28 (symbol index cache).
- Watch items: B11 and B12 (KiCad 11 DRC and schematic IPC), B33 and D27 (other routers, Topola).
- D28 records the methods cut as unshippable (GPU training and similar).

## 4. Candidate PR sequence

One PR at a time, each on its own branch from `main`. Each early PR unblocks later ones.

| # | PR | Covers | Done when | Depends on |
|---|---|---|---|---|
| 1 | Netlist read and intent compare | rank 1 (A3, B14, D11, netlist half of C20) | `get_schematic_netlist` returns `{net: [ref.pin]}` across the hierarchy, unnamed nets included; `compare_netlist` reports missing, extra, merged and split nets by pin partition; fixture tests include a hierarchical sheet | — |
| 2 | Route safety (port #901) | rank 2 (A2, D10 core) | wiring that would join a foreign net re-routes or refuses; `nets_joined` is reported; PR 1 proves no net changed on fixtures. The connect_to_net stubs (#899) and add_wire T-dots (#900) are included because #901 touches the same path (one verifier suggested a follow-up instead) | 1 |
| 3 | Response budget | rank 3 (C3, A30, C26, C24) | shared helper; ERC/DRC return a summary plus top N under a byte cap, with the full report on disk; then the list tools and a names-only `load_toolset` | — |
| 4 | Render as image | ranks 4, 25 (C5; C6, A25, B9, B7) | inline renders arrive as image blocks; region crop for schematics; side and zoom for boards | — |
| 5 | Guidance fixes | rank 5 (A10, A7, A8, A9, A33, A20) | shipped skills and agents match the code; the Freerouting path is documented with its limits; the build agent ends with `compare_netlist`; the rule "report the tool gap and give the GUI steps" is added (it replaces B36) | 1, 2 |
| 6 | Small schematic edits | ranks 10, 19 (C8, B35, A34) | mirror after placement; DNP and BOM/board flags; title block (ports #816, #943/#944) | — |
| 7 | Board geometry layer | ranks 6, 7 (C19, C17, A14 containment) | `query_board_region`; polygon/arc outline containment in `score_placement` and the review | — |
| 8 | Placement objective | ranks 8, 22 (A14, D2, D1, D3, D22) | wirelength and crossing terms; fanout-filtered nets; pin-level decoupling distance | 7 |
| 9 | Sync completion | rank 9 (C7) | field copy and orphan delete first, previewed in the dry run; footprint replace in a follow-up PR | — |
| 10 | Library correctness | ranks 11, 16, 24 (A16, B21, A15, A18) | pin-pad check; symbol fields and `edit_symbol` (ports #909, #949); ranked search | — |
| 11 | Honest review and current audit | ranks 14, 12 (D15, D20, D17, C16) | `audit_manufacturing` checks fire on KiCad 6+ boards; per-net current audit | 7; §5 question 3 (design-intent record), or a per-call currents argument as an interim |
| 12 | Routing batch and readback | rank 15 (A11, A12, C10, in-place half of C11) | polyline/batch apply in one commit, KIIDs back, net and width checks, in-place modify | — |
| 13 | DRC rule writer | rank 13 (D16, A32, B38 rules) | validated `.kicad_dru` writer; the DSN export refusal resolved; `.kicad_pro` minima and diff-pair fields (ports #965/#966, #866) | — |
| 14 | Checkpoints and readiness | ranks 18, 23 (C21, C14, C13) | checkpoint, list and restore (refused while an editor owns a file); readiness table | — |
| 15 | ERC/DRC baselines | rank 17 (C4) | save and compare with stable keys | 3 |

After these come the larger items (ranks 21, 29, 32, 33) and anything that waits on §5. Rank 35 (call-log hygiene) is a quick win that can slot in anywhere.

## 5. Open questions for the owner

1. **Konnect-owned placement and routing algorithms.** *Decided 2026-11-06: allowed back as a second opinion, never as the author.* Upstream #590 and #754 retired the autonomous placers ("not a second router"). The owner's rule is that the AI does its own wiring, placement or routing. A Konnect planner (D8, D10, B32, D9, C11, D4, B29, D14) runs alongside and returns a plan only, never writing a file. At the end, the AI compares the plan with its own result, adopts the good ideas through the normal editing tools, and ignores the plan where it is worse. This needs three things:
   - every planner returns a shared, read-only plan format;
   - a compare step that scores both results with the same metrics: rank 1 netlist equivalence, rank 8 wirelength and crossings, the C15 clearance check, and the D13 readability score for schematics;
   - a skill section that describes the loop.

   The metrics come first, so planners start after PRs 1, 7 and 8. Ranks 36 and 44 and the planners parked in §3 are unblocked on these terms.
2. **Freerouting as the route path.** *Decided 2026-11-06: yes, after the core PRs (1–15).* Widen the DSN profile (rank 32) so Freerouting works on real boards with pours and back-side parts. Until then, PR 5 documents Freerouting with its limits.
3. **A design-intent record.** *Decided 2026-11-06: yes, one record per project.* It holds rails, currents, interfaces and the fab target, and ranks 12, 29 and 38 read from it instead of each taking their own inputs. It gets its own PR before PR 11; until it lands, PR 11 can use a per-call currents argument as an interim.
4. **KiCad 10 variants and jobsets.** *Decided 2026-11-06: keep parked* (`ROADMAP.md:112-114`). Revisit only if board variants or repeatable fab-output jobs become needed.

## 6. Limits of this research

- **Gaps no track covered** (from the critic): a stackup and copper-weight writer for board setup; part selection with the JLCPCB toolset; bulk footprint assignment (the CvPcb step); the zone lifecycle (list, edit, refill one zone, delete); DXF outline import and enclosure fit; fab dimensions and notes. Each needs its own look before it is ranked.
- **Real-use weighting:** the evidence comes mostly from rework of one board. Track D's raw call-log counts are dominated by test traffic. For example, the 98/98 `route_specctra_dsn` "errors" are a missing-file dispatch test, so they were not used for ranking.
- **Not reached:** about 175 of the 201 upstream issue bodies (titles only); paywalled papers (Kraftwerk2, Wybrow et al., Schulze et al.); JITX source; DeepPCB API docs; the Quilter scorecard; Flux docs beyond snippets; a KiCad 11 release date. Firecrawl credits ran out during verification, so some peer-README quotes in B7, B9 and B14 are unchecked.
- **Unverified risk:** `konnect-render` refuses SVGs with visible text, which may block the layer and library renders in rank 25.
- **Instructions found in data and ignored:** two peer READMEs addressed AI agents, and the arXiv tool appended rendering instructions to every result.
- **Privacy:** this document holds counts only. The git-ignored `real_use.md` findings file still quotes some board-specific details from transcripts, so delete it once it's no longer needed.

## 7. Sources

**Track A, codebase:** `crates/konnect-core/src/router/registry.rs`, `crates/konnect-core/src/tools/*.rs` and `tools/placement/`, `crates/konnect-schematic-editor`, `crates/konnect-sexp`, `crates/konnect-render`, `tool-directory.md`, `ROADMAP.md`, `docs/RELIABILITY_CONTRACT.md`, `docs/reliability-legacy-inventory.json`, `docs/API_MIGRATIONS.md`, `docs/DEVELOPING_TOOLS.md`, `docs/KICAD_INTEGRATION.md`, the shipped skills and agents in `crates/konnect/assets`, and upstream issues and PRs at github.com/mixelpixx/Konnect.

**Track B, landscape:** KiCad 9.0, 10.0 and 10.0.8 release notes (kicad.org); the KiCad 10 CLI reference (docs.kicad.org/10.0/en/cli); KiCad IPC protos (10.0 and master) and kicad-python; peer servers and skill packs on GitHub: lamaalrajih/kicad-mcp, mixelpixx/KiCAD-MCP-Server, oaslananka/kicad-mcp-pro, Seeed-Studio/kicad-mcp-server, NiRuLabs/kicad-mcp-server, ProductOfAmerica/mcp-server-kicad, ShamanAndrey/kicad-mcp-layer, Finerestaurant/kicad-mcp-python, blwfish/kicad-mcp, antonmadto/kicad-mcp, Draganito/kicad-mcp, aklofas/kicad-happy, Diode663/kicad-claude-skills and jasiek/kicad-llm-plugin; code-first tools diodeinc/pcb, atopile, SKiDL, tscircuit and tscircuit-autorouter; easyeda2kicad and TousstNicolas/JLC2KiCad_lib; yaqwsx/KiKit; Freerouting; kicad-library-utils; instadeepai/deeppcb-kicad-plugin; vendor pages for DeepPCB, Quilter, Flux Copilot and JITX.

**Track C, real use:** the Konnect call log (`~/Library/Application Support/konnect/logs/calls.jsonl`, counts only); the 15 Claude Code transcripts that contain Konnect calls; the project memory notes.

**Track D, methods:** arXiv 2611.08175 (PCBRouteAgent), 2607.05915 (PCBWorld), 2605.30345 (SchGen) and 2602.00510 (PCBSchemaGen), and abstracts of 2607.03835 (Weave), 2210.14259, 2502.14012, 2602.23540, 2411.13899, 2601.04505, 2603.18102, 2609.33356, 2606.23704, 2608.04434, 1705.04984, 2506.10577 and 2305.01540; Cypress (ISPD 2025); Wybrow et al., orthogonal connector routing; Schulze et al., layered graphs with port constraints (JVLC 2014); the KiCad custom-rules manual, the PCB calculator manual, and the IPC-2152 and return-path doxygen sources; topola; kicad-sch-api orthogonal routing notes; licenses from GitHub for NVlabs/Cypress, SA-PCB, PCBWorld, rl_pcb and OpenROAD.
