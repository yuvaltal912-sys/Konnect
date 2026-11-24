# Row CT: family C as a placement tool

**State, 2026-11-22.** Designed and built on the owner's word ("יש כבר קוד כתוב להנחת רכיבים
מתכנסת, המשימה שלך היא לבנות כלי"). He chose both open decisions as recommended: a planner that
writes nothing, and routing the best three candidates before a winner is named. Branch
`claude/converge-place-tool` from `fork/main` `1506938e`; merged as
[#104](https://github.com/yuvaltal912-sys/Konnect/pull/104) (main `6abf6333`).

## What it is

One MCP tool, `plan_convergent_placement`, in the `placement` toolset. It runs the placement
search of Row PS and Row CP on a scratch copy of the board that is open in KiCad: families F and
S make the starts, family C refines them until its stop rules end it, and the best three C
candidates are routed with fastroute and judged by the bench's own gates and rank key. The
result is the winning placement as the exact arguments of `set_component_placements`, with the
numbers behind it. The tool writes nothing to the board; the AI shows the owner the plan and
applies it through `set_component_placements` only on his OK (research decision Q1; Row PS
decision 1).

Nothing in the search changes: the generators, family C, its parameters and stop rules, the
judge, the gates and the rank key are the bench's, with the bench's values. The tool adds the
input side (the live board, KiCad's rules, a scratch project) and the output side (the winner,
the response).

## Decisions

The owner's two (2026-11-22, as recommended):

| ID | Decision | Reason |
|---|---|---|
| T1 | A read-only planner. It returns the winner as `set_component_placements` arguments; the board changes only through that existing tool, on the owner's OK, with its own readback. The arguments are one batch, not one batch per functional group (Row PS decision 1's earlier shape). | No new write path for a whole board; the planner is a second opinion (Q1). A converged placement is legal only as a whole: half of it applied can put a moved part on a part not yet moved. One batch is one KiCad undo step. |
| T2 | Route C's best three candidates (fastroute, effort `m` by default) and name the winner by the bench's judge, not by C's cost. | On the owner's board the C candidate of the lowest cost failed to route completely (25 and 26 open connections) in both bench runs (Row CL, limit 3). Run time is information, not a decision (owner, 2026-11-13). |

Calls made in this design (a sensible builder's, recorded with the reason):

| ID | Call | Reason |
|---|---|---|
| T3 | The board must be open in KiCad, and the only board open (`BoardAccess::LiveOnly`). The tool reads the live board's text and KiCad's effective routing rules in one IPC session, twice, and refuses when either changed in between (as `export_specctra_dsn` does). | The routing export needs KiCad's own net-class resolution; the bench gets the same from `net_rules.py`. The live text holds the owner's locks even before he saves. KiCad resolves net classes without naming a document (`GetNetClassForNets`), so a second open board could lend its rules; `export_specctra_dsn` has the same guard. |
| T3a | The response names the board state it was planned from (`planned_from`: the live text's SHA-256 and its count of tracks, arcs, vias and teardrop zones). A note says to call again if a part was moved, added or locked since; when the board holds routing, a note and the apply step say to remove it first. | `set_component_placements` over IPC checks no revision, and applying moves no copper: old tracks would be left off the moved pads. |
| T4 | Held = the owner's KiCad locks only (`HoldRules::locks_only()`, no caller list), plus the parts the held set always holds (no pad on a net). | The owner's flow (Row PS, 2026-11-19): he locks what is fixed, the search places the rest. |
| T5 | The search runs in a new run folder, `<state dir>/placement-plans/<milliseconds since 1970>-<12 hex of the input SHA-256>/`: `probe/` (fastroute's probe), `project/` (the files the judge's own `scratch_board::copy_project` copies, and the live text with its routing stripped by `place_route::strip_routing`, named like the board), `kicad-config/` (a scratch `KICAD_CONFIG_HOME` for the unrouted checks), `isolation/`, `candidates/`, `search.json`, `table.md`. The folder is kept and its path returned. | The bench's `input/` layout, made by the copy the judge already trusts, so the export and the judge read the same files; the owner's KiCad settings are never re-saved; the records stay for review. |
| T6 | The schematic neighbourhoods are read from the owner's own root schematic (beside the board), not from a copy: a new `Plan::schematic` field names the root (`None` in the bench: the board's own, as before). | `read_sheets` does plain reads and follows only files inside the root's folder; copying the sheet files would need their list, which the reader does not return. |
| T7 | The matrix is the bench's: `Matrix::bench().only(&[Force, Schematic])` with family C at `ConvergeParams::measured(0)` and `OuterRule::measured()`; generation threads = the host's cores. | Row CP measured these; no tuning. |
| T8 | Routing: no reference calls (O, A), only family C's candidates, the first three in C's own order (`routing_order`'s C sequence: requirements, then the converged cost; none drawn). A new `Plan::routed` field (`Routed::Every` in the bench, `Routed::ConvergeOnly` in the tool) drops every other id from the order. No time limit beyond each call's own budget. | Three calls at `m` are the plan's whole routing cost (times on the bench's boards in `target/creation-research/convergent-placement/`); the order and the judge are the bench's, unchanged. |
| T9 | The winner is the routed C candidate of rank 1. Its placements are generated again from its recipe on the search's own model (`Outcome::model`, never a second read of the files), and the tool refuses to return them unless the rebuilt board's SHA-256 is the judged one (`place_search::rebuild`). | The response must carry exactly the board that was judged; a schematic saved during the run must not change the rebuild. |
| T10 | `status`: `planned` (every gate passes), `planned_with_gate_failures` (the winner fails a gate; each failing gate is named), `no_candidate` (C made no candidate; C's reasons are given, Row CL limit 1), `not_routed` (no C candidate routed; each call's reason is given). Only the first two carry placements. | Cost alone can pick a bad winner (T2), so an unrouted plan is not offered. |
| T11 | The response holds counts and numbers only, as `search.json` does: the winner's cost terms and screen numbers, its route numbers and gates; one line per routed candidate; the search's counts and stop words; the held and movable counts; the run folder; `notes` for the known limits (no decoupling pair found: Row CL limit 4); `next_steps`. | The response budget, and what the AI needs to explain the plan to the owner. |

## Arguments

| Argument | Default | What |
|---|---|---|
| `board` | required | the `.kicad_pcb` that is open in KiCad |
| `effort` | `m` | the routing effort of each of the three calls (`fast`, `m`, `h`, `xh`) |
| `fastroute_path` | as `route_specctra_dsn` | the fastroute binary (argument, `KONNECT_FASTROUTE`, user config, the packaged binary, the home default) |

## Function skeletons

`crates/konnect-core/src/tools/placement_converge.rs` (a top-level tools file, so the konnect crate's
`schema_parameter_usage` guard reads its handler):

- `tools() -> Vec<ToolDef>`: the definition (schema closed, `BoardAccess::LiveOnly`).
- `handle_plan_convergent_placement(args, ctx)`: `read_arguments`, the live read (T3), the run
  folder (T5), fastroute (`locate_fastroute`, as `route_specctra_dsn`), `scratch_project`,
  `tool_plan`, then `plan_and_judge` with `PlaceRouteCall`.
- `live_input(ctx, board)` and `stable_read(before, after, rules, rules_after)`: T3.
- `run_folder(state, text, stamp)`, `scratch_project(folder, board, text)`: T5.
- `tool_plan(folder, inputs, schematic, threads) -> Plan`: T4, T6, T7, T8.
- `plan_and_judge(plan, routing, board, planned)`: the search awaited on the handler's task
  (phases 1 and 1b run inline, as in the bench; a cancelled call drops the routing calls, whose
  `JoinSet` and `kill_on_drop` stop the router), `verdict_of` on `Outcome::model`, `result_json`.
- `Planned::of(text)`: the live text's SHA-256 and its tracks, arcs, vias and teardrop zones, what `strip_routing` leaves out (T3a).
- `verdict_of(model, outcome) -> Verdict`: T9, T10. `result_json(...)`, `notes`, `next_steps`:
  T11.

`place_search.rs`: `Plan::routed` (`Routed::Every`, `Routed::ConvergeOnly`), `Plan::schematic`,
the one `retain` in `search` (T6, T8), and `rebuild` (`rerun` with the candidate, T9);
`search.json` names `routed` and `schematic` only when the tool set them, so the bench's file
is what it was. `place_schematic.rs`: `SchematicBaseline::read_root`.

## Tests (each guard with its negative control)

| Guard | Test | Negative control |
|---|---|---|
| T8 only C is routed | `converge_only_routes_family_c_alone_and_screens_every_other_record_out`: the search tests' fake router sees only family C ids, at most three | drop the `retain`: F and S ids reach the fake |
| T8 the bench is unchanged | `every_routes_the_other_families_too_and_its_file_is_unchanged`, and the bench's existing order tests | — |
| T6 the schematic root | `a_named_schematic_root_is_the_one_family_c_reads` | read beside the board whatever the field says |
| T9 the rebuilt board | `a_winner_whose_board_is_not_the_judged_one_is_refused` | skip the SHA-256 check |
| T9 the winner | `a_plan_routes_only_family_c_and_returns_the_rank_one_winner_as_apply_arguments`: the returned arguments applied to the input give the judged board of rank 1 | take the worst rank |
| T10 statuses | `a_winner_that_fails_a_gate_is_planned_with_each_failure_named`, `no_routed_candidate_gives_no_placements_and_the_reasons`, `no_start_is_no_candidate_and_nothing_is_routed` | a failing winner reads `planned`; an export refusal offers a plan |
| T3 stable read | `a_board_or_rules_that_change_while_read_are_refused` | compare nothing |
| T4 the locks | `the_plan_is_the_bench_search_with_the_locks_held_and_only_c_routed` | the bench's hold rules |
| T5 the scratch project | `the_scratch_project_is_stripped_named_like_the_board_and_beside_its_project_files`: the original folder's bytes are unchanged | the routing is not stripped |
| T11 the notes | `a_board_without_decoupling_pairs_gets_the_note_and_one_with_pairs_does_not` | the decoupling note always; the routing note on the wrong condition |
| T3a the old routing | `a_routed_board_is_counted_and_its_old_routing_named_before_the_apply` | vias not counted; the apply step never names the old routing |
| T10 calls | `an_input_check_that_fails_routes_nothing_and_says_so`: no call, `routing_calls` 0, the stop named | count records handed over instead of calls made |
| T10 reasons | `a_not_routed_verdict_gives_the_called_records_reasons_only`: a C record that was only not among the first three lends no reason | every C record explains the verdict |
| arguments | `the_arguments_default_to_effort_m_and_refuse_what_they_cannot_use`; the closed schema (`the_tool_is_registered_live_only_in_the_placement_toolset_with_a_closed_schema`) | — |

The guard tests of the konnect crate (`asset_references`, `schema_parameter_usage`,
`creation_guidance`) and the doc counts cover the registry entry, the skill and the tool
directory. `every_tool_the_next_steps_name_is_registered` checks the tools the response names.
The live read itself (IPC to a running KiCad, the one-board guard included) is not exercised by a
test: no test here starts KiCad. `no_candidate` is tested only for a search with no start, not for
one whose C records all fail P1.

## Limits (inherited, stated in the response)

- Row CL limit 1: on a board too dense for C's gap rule, C makes no candidate (`no_candidate`).
- Row CL limit 4: where `score_placement` finds no decoupling pair, C does not keep capacitors by
  their ICs; the response notes it when the board's pair count is 0.
- Row CL limit 3 is why the tool routes (T2); three calls may still all fail a gate.
- The routing at `m` ranks the placements; the final routing of the applied board is a separate
  `route_specctra_dsn` call at the owner's effort.
