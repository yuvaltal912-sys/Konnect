# One call from placement to routing (Row FF): design

**State, 2026-11-27.** Built on branch `claude/full-flow-design-1a7eef`, from `fork/main`
`01386bcf` (B to E merged), with `fork/main` `24c2b04a` (#131) merged in. Local, not pushed,
no PR. The owner approved the four decisions at the end of this note as recommended ("Yes,
all four") and the build ("כן, לבנות את ו' אחרי שהם ימוזגו"), both relayed by the
supervisor session. "The build" below says what changed from this design and what was
verified. Real-board results stay out of this note, apart from counts.

## What the owner asked for

Fix F of the six ranked fixes from his run of 2026-11-25: one mode that takes a board from
placement through pours to routing, in one call or one tracked job.

That run needed about 15 manual tool steps for "full routing":

- `plan_convergent_placement`, then `set_component_placements`;
- `plan_pours`, which asked him to confirm net roles and gave no easy way to do it;
- two `add_zone` calls, then `refill_zones`;
- the old routing deleted with 62 single calls;
- a netclass width that needed the board closed;
- `export_specctra_dsn`, `route_specctra_dsn`, then the import;
- skew rules written one at a time.

The IPC link to KiCad dropped three times along the way. Pours were never part of the
plan, and nothing said whether they had been tried.

## The short answer

One new tool, `run_full_flow`. It is a tracked job on D's job module:

- A **start call** returns at once with a `run_id`.
- **Report calls** (`run_id`, `wait_s`) show progress and, at the end, the whole result.
- The job stops at a **gate** only where the user has not approved in advance.
- An **answer call** (`run_id`, `answer`, `expected_plan_revision`) gives every approval of
  a gate in one argument.

The flow plans everything on a scratch copy first:

1. the pours on the current placement;
2. the placement search, whose candidates are routed with those pours (built, then left
   off by its measurement: see "The build", FF4);
3. the pours again, on the winning placement;
4. an export check of the board the flow is about to build.

Then it shows one plan: the parts that move, the old routing that goes, the zones that are
added, and the pour roles that come only from net names. The user answers once. The job
then changes the live board one step at a time:

- a checkpoint before each step, and a verified save after it;
- export, then a route with fastroute at the user's effort;
- the session applied when every judge gate passes (or a question when not);
- stitching, the length rules in one batch, then a refill, DRC and save.

The final report lists every step as done, skipped or not checked, in the Row HS style.

Every write goes through the public tool's own handler, taken from the registry with the
same checks the MCP dispatcher runs. So B's checkpoint guard, C's revision check and E's
live rule writes apply exactly as they do when the AI calls those tools by hand. F adds no
new way to write a board.

## What already exists, and what F takes from each

| Fix | Branch or PR | What F uses | Which part of F depends on it |
|---|---|---|---|
| A, honest summaries | merged, #124 | the route `summary` (`lengths`, `pours`, `completion.not_checked`); the placement winner's `moves` and `change` | the step summaries and the final report |
| B, board safety | `claude/board-safety-ffb8c0` | `checkpoints::ensure_checkpoint`; `board_safety::guard`, applied by the registry's catalogue to `set_component_placements` and `apply_specctra_ses`; `route_preflight`; the plain "board not open" sentence; `board_safety`'s `liveness` and `Outcome::of`, made `pub(crate)` so F does not copy them; the `live_project.restore` form | the checkpoint before each live step, recovery, the pre-flight |
| C, `delete_all_routing` | `claude/delete-all-routing-5ed290` | the dry run (`will_delete`, `kept_locked` with `export_will_refuse`, `plan_revision`, `status: noop`); the apply with `expected_plan_revision` | P1, gate A's count, L2 and its resume check |
| D, async placement | `claude/async-placement-runs` | `jobs::{JobKind, Jobs, claim_board, board_busy, run_folder_of, Progress, Report, MAX_WAIT, valid_run_id}`, with `write_json` made `pub(crate)`; `place_search::{SearchProgress, search_observed}`; `placement_converge::{verdict_of, result_json, Planned, tool_plan, scratch_project}`, with `live_input`, `LiveInput` and `locate_fastroute` made `pub(crate)`. The run setup the handler builds inline (`RouteInputs`, `isolated_context`, `PlaceRouteCall`) is factored into `prepare_search`, which both use | the job shape, the run time and the MCP timeout, P3 |
| E, rules with the board open | `claude/rules-with-board-open` | `create_custom_drc_rule` with `rules=[...]` and `replace_existing` (one preview, one apply). `create_netclass` over IPC is not called by F; it removes the "close the board" stop from the user's own rule edits before the flow | F3. Its next step is prose today, so F's PR gives it a structured form (see "Modules") |
| main | — | `live_board::observe`, `board_identity::canonical_sha256`, `place_search::{RouteCall, rebuild}`, `place_route::{strip_routing, export_candidate}`, `pour_plan::{with_zones, add_zone_arguments, ProposedZone}`, `route_judge`'s `find_tool`/`call_tool` (the in-process call precedent) | throughout |

Their interfaces may still change in review. F's code touches them only in the adapters
listed under "Modules". Each adapter has a test that pins the call it makes.

## The tool's shape

**Recommended: a job on D's module, split into legs between gates, with live writes
through the registry's own handlers (decision FF1).**

| Option | For | Against |
|---|---|---|
| One synchronous tool | One call | Cannot finish under the MCP client's limit. The routing alone takes 4 to 40 minutes, and a placement run on one of the owner's boards has outlasted the client's 30 minutes (Row AP2). |
| A planner that returns one ordered batch of steps | Follows Q1, Row PS decision 1, Row CT and Row ZR decision 4 to the letter: the planner writes nothing | The AI still makes 10 or more live calls. Each one can hit an IPC drop. Nothing records where the flow stopped, and the approvals stay spread over many turns. That is the pain itself. |
| **A tracked job, legs between gates** | One start call. One answer for the approvals. It survives IPC drops and server restarts through its record. It reuses D's module. The same handlers mean the same guards. | It writes the live board itself, at the approved points. That amends "the planner never writes" for this one tool: owner decision 1. |

### Calls

A call has exactly one of these forms. A mix of two forms is refused, as D refuses one.

| Form | Arguments | Answer |
|---|---|---|
| start | `board`, and optionally `effort`, `placement_effort`, `fastroute_path`, `steps`, `approve` | `status: started` with `run_id`; or `awaiting_answer` at gate R (pour roles); or a refusal from the pre-flight |
| report | `run_id`, `wait_s` (0 to 600, default 0) | `running` with `phase` and progress; `awaiting_answer` with the gate; `stopped`; `done`; `abandoned` |
| answer | `run_id`, `answer` (approvals), `expected_plan_revision` | `started` (the next leg), or a refusal that names the stale or missing key |
| resume | `run_id`, `resume: true` | `started` from the stopped step, or a refusal that says why the board is not where the step left it |
| abandon | `run_id`, `abandon: true` | `abandoned`. Refused while a leg runs, because D has no per-run stop: wait for its gate or stop. Nothing on the board is undone; the result names the checkpoint to restore |

- `effort` is the final routing's effort. Default: the user config `routing.effort`, else `m`.
- `placement_effort` is the effort at which the placement candidates are routed. Default
  `m`, Row CT's default.
- `steps` defaults to every step: `{"placement": "run", "pours": "plan", "routing": "run"}`.
  Each step can be `"skip"` instead.

## The order of the steps

Where each step runs:

- **scratch** is a copy in the flow's folder. The user's project is never written there.
- **live** is the board open in KiCad, changed over IPC through the named tool.

Each attempt at a scratch or route step gets its own numbered folder,
`<flow>/attempts/<n>/`. `export_specctra_dsn`, `route_specctra_dsn` and `scratch_project`
never replace an output, so a resume must not reuse one (FF14).

| # | Step | Where | How | Gate | Checkpoint first | Skipped when |
|---|---|---|---|---|---|---|
| P0 | Pre-flight | live read | see "P0 in full" below | R, if a role comes only from a net name and `approve.pour_roles` is missing | no | never |
| P1 | Old routing counted | live read | `delete_all_routing` dry run, with `include_locked: true` when the user chose `delete_including_locked` | shown at A | no | no routing on the board |
| P2 | Pours on the current placement | scratch | `plan_pours`'s planner and KiCad fill check (`plan_text`) on the stripped live text, with the confirmed roles | no | no | `steps.pours: skip` |
| P3 | Placement search | scratch | Row CT's search (`prepare_search`, `plan_with_outcome`). Each candidate is routed, and the unrouted yardstick measured, on a board with P2's zones added (`WithPours`, FF4) | no | no | `steps.placement: skip` |
| P4 | Pours on the winner, and the export check | scratch | P2 again, on the winner's board from `place_search::rebuild`. Then `place_route::export_candidate` on that board with P4's zones (FF17) | no | no | pours skipped; the export check runs whenever routing will run |
| A | **Gate A, the plan** | — | see "What gate A shows" below | **yes**, unless every key is approved in advance | — | nothing to approve |
| L1 | The board unchanged since the plan | live read | the live board's canonical SHA-256 equals `planned_from`, and the live board equals its saved file | no | — | never |
| L2 | Old routing deleted | live | `delete_all_routing`: a fresh dry run whose counts must equal the approved counts (FF9), then the apply with its `plan_revision` | approved at A | yes (the start checkpoint) | nothing to delete, or `delete_routing: keep` |
| L3 | Placement applied | live | `set_component_placements` with `apply_with` (B's guard checkpoints too) | approved at A | yes | `change: none`, or `placement: keep_current` |
| L4 | Zones added | live | `add_zone` once per P4 proposal, then `refill_zones` | approved at A | yes | no proposal, or `pours: skip` |
| R1 | Export | live read | `export_specctra_dsn(board, output=<attempt>/board.dsn)` with the default `pours` (`fixed`) | no | no | routing skipped |
| R2 | Route | scratch | `route_specctra_dsn` (fastroute, `effort`; B's pre-flight runs inside it). A result with no winner stops the flow at once (see "When a step fails") | no | no | as R1 |
| R3 | Import check | live read | `plan_specctra_ses_import` | no | no | as R1 |
| B | **Gate B, the routing** | — | the route `summary`, `status`, `completion.judged.gates` | only when `routing: ask`, or `apply_if_complete` and `completion.judged.all_gates_pass` is false (FF16) | — | otherwise |
| F1 | Session applied | live | `apply_specctra_ses(..., candidate_output_path=<attempt>/candidate.kicad_pcb)` (B's guard checkpoints) | approved at A or B | yes | not approved |
| F2a | Plane stitching | live | only when `completion.next_steps` holds the plane-stitching steps: at most 4 rounds of the planner (its arguments as the step gives them, `search_radius_mm` included), `route_batch` with every stitching proposal (split at 200 objects a call), then `run_drc` with refill. Stops when no proposal is left or 0 items on those nets are unconnected (the skill's rule). A save before each round, because the planner reads the saved board | covered by the routing approval | yes, each round | no such step |
| F2b | Reference vias | live | only when `completion.next_steps` holds `reference_steps`: one pass of the same three calls. A planner refusal (it refuses local pad overrides, for one) is not a failed write: it is "left for you", with the step's own text, and the flow goes on | covered by the routing approval | yes | no such step |
| F3 | Length rules | project file | only when the route result has the structured rules step: E's one `create_custom_drc_rule` batch with `replace_existing: true`, a dry run then the apply with its `plan_revision` | covered by the routing approval | yes | no such step |
| F4 | Refill, DRC, save | live | `run_drc(board, sync_live_board: true, refill_zones: true)`, then a verified save | no | no | never, once anything was written |
| F5 | Report | — | the final summary | — | — | never |

Every live step ends with a verified save. That is step 6 of "Each live step", and is not
repeated in the table.

### P0 in full

The pre-flight runs in the start call itself and takes seconds:

- the arguments;
- no other flow and no placement run on the board (both job kinds' claims, FF12);
- Row CT's stable read (`live_input`): one board open, the text and the rules read twice
  and equal;
- the live board equals its saved file, else refused (FF6);
- `<board>.kicad_pro` beside the board;
- fastroute located once, into a probe folder that is removed on a refusal; its path is
  kept in `flow.json`;
- the design intent read;
- the pour roles resolved.

`planned_from` is `canonical_sha256` of the live text. Row CT's `Planned` (a byte hash)
stays in the placement body only, for what it means there.

### What gate A shows

- **The placement:** `moves`, `change`, and the P3 winner's routed verdict. The verdict is
  the evidence that a routing exists for the new placement.
- **The routing that goes:** counts by kind, and `kept_locked`.
- **The zones:** net, layer, and the fill check.
- **The roles,** with their sources.
- **The export check.**
- **The plain warning:** "the old routing is deleted before the new routing exists; the
  way back is the start checkpoint".

### Why this order

- **Roles before the long search (gate R).** The pours change what the router can do, so a
  wrong role would mislead the whole placement search. The question comes within seconds of
  the start, while the user is still there.
- **Pours before and after placement (P2, P4).** The candidates are judged with the pours
  they will have (FF4). The zones that are added come from a fill check on the winner.
  Their outline is the board outline, so it does not depend on the placement. Their thermal
  values (PP16) and the fill check do.
- **Nothing destructive before the route is shown possible (P3, P4, FF17).** L2 deletes
  routing the user may want back. So the plan must show, before gate A, that the new
  placement routes (P3's routed winner) and that the board F will build exports (P4's
  check). `export_specctra_dsn` refuses many things besides unlocked routing: pad shapes,
  clearance overrides, custom rules.
- **Deletion before placement (L2 before L3).** Row CT's note: "remove the old routing
  before the placements are applied". Old routing is not moved with its pads.
- **Zones before the export (L4 before R1).** `plan_pours`'s rule: "before
  export_specctra_dsn or after the routed session is imported, never between the two". The
  router's pour key needs the pours in the DSN. The search tries "as exported" and "pours
  omitted" itself, and the route `summary.pours` says which way won.
- **The final route runs on the live board after L4, never on the scratch winner (FF3).**
  The import is bound to the exact exported revision (`plan_specctra_ses_import`), and
  KiCad's save does not give back the scratch text byte for byte. The final route also runs
  at the user's `effort`, which is usually higher than the candidates' `m`.

## Approvals and gates

### The `approve` object

`approve` is the same object at the start and in an answer:

```json
{
  "placement":      "apply" | "keep_current",
  "delete_routing": "delete" | "delete_including_locked" | "keep",
  "pours":          "add" | "skip",
  "pour_roles":     "as_named" | { "<net>": "ground" | "power" | "not_a_supply" },
  "routing":        "apply_if_complete" | "apply_best" | "ask" | "skip"
}
```

- At the start, any key may be given in advance. A key given there is not asked again.
  Leaving out `approve` means "ask me".
- At a gate, the result lists `needs`, the keys still missing, each with the plan's
  `recommended` value. It also gives `answer_with`, the exact next call with every needed
  key at its recommended value. The AI shows the user the plan. On a plain yes it sends
  `answer_with` unchanged. Otherwise it changes the keys the user named.
- An answer may set only keys that are not set yet, with one exception: `pour_roles` may
  be answered again at gate A, which plans the pours again. Any other key set twice with
  two values is refused, naming it.
- The recommended values:
  - **placement:** `apply` when `change` is `placement`; `keep_current` when it is `small`
    (Row HS: "applying it is optional"). With `none` there is no question, and the step is
    skipped.
  - **delete_routing:** `delete` whenever placement or routing runs.
    `delete_including_locked` when P1's `kept_locked.export_will_refuse` says the export
    would refuse the kept locked routing, or when locked routing touches a part that moves.
  - **pours:** `add` when the fill check is clean. With findings there is no
    recommendation: the user decides, as `plan_pours`'s own next steps say.
  - **pour_roles:** `as_named`.
  - **routing:** `apply_if_complete`.
- `expected_plan_revision` binds the answer to what was shown. It is a SHA-256 over the
  flow id, the gate, `planned_from` and the gate's plan body. An answer to an older plan is
  refused and names the current revision.

### Combinations that are refused

Each refusal is checked when the answer arrives, before anything is written. Each names the
key to change. "Routing runs" means `steps.routing` is `run` and `approve.routing` is not
`skip`.

- `placement: apply` with `delete_routing: keep` while the board has unlocked routing: the
  old routing would stay off the moved pads (Row CT).
- Routing runs with `delete_routing: keep` while the board has unlocked routing:
  `export_specctra_dsn` refuses such a board. The way out is `routing: skip`.
- Routing runs while P4's export check failed: the refusal quotes the export's reason.
- Routing runs with `delete_routing: delete` while `kept_locked.export_will_refuse` is set:
  the way out is `delete_including_locked`.
- A `pour_roles` entry that contradicts a design-intent rail: the intent wins
  (`supply_roles`'s order). The refusal names the rail and says to edit it with
  `set_design_intent`.
- A `pour_roles` net that the board does not have.

### Roles in one argument (gate R, and `plan_pours`)

`plan_pours` today says "Confirm them with the user, or record the rails with
set_design_intent", and a rail needs a voltage the user may not know. F, and `plan_pours`
itself, get one argument: `pour_roles`.

- `"as_named"` confirms every role that comes from a name.
- A map overrides the roles of the nets it names. Their source becomes `"user"`, and the
  planner's `nets[].source` says so.
- Nothing is saved. The run keeps the roles in `flow.json`, and the final report suggests
  `set_design_intent` (section `rails`) for the nets with a known voltage (FF5, owner
  decision 3).
- The rails that a role may not contradict are read from the resolved roles' sources: a
  source that names a design-intent rail (`rails_of`).

When an answer at gate A changes the roles, F plans the pours again on the winner, and runs
the export check again, before anything is written. The new plan goes back to gate A only
if the proposals changed. The placement search is not run again: it was judged with the old
pours, and the result says so in `not_checked`.

### What one yes covers

By default, one yes at gate A (owner decision 2) covers:

- the old routing deleted;
- the placement applied;
- the zones added;
- the routing applied when every judge gate passes (`completion.judged.all_gates_pass`),
  together with the stitching that the judge added and the session lacks, and its length
  rules.

`status` alone cannot decide this. `judged_status` keeps the router's status whenever the
judge needed stitching the session lacks, so a board that needs F2 never reads `complete`
before F2 has run (FF16). F4's DRC is then the check.

The flow stops again only when a judge gate failed (gate B). `apply_best` applies the
winner whatever its gates. `ask` always stops at gate B.

## Checkpoints and recovery

### Each live step

L2 to L4 and F1 to F4 all run the same protocol:

1. **Intent first.** `flow.json` records the step as `calling`, with its arguments, before
   any call. A crash after this point leaves a record that says what may have happened.
2. **Liveness.** B's `liveness`: the board is open, and the live board equals its saved
   file. A failed read is tried 3 times in all, 5 s apart (FF7). When it still fails, the
   flow stops with `why: kicad_unreachable` and can be resumed.
3. **Where the board stands.** The saved file's canonical SHA-256 must equal the saved
   `after` of the step before (`planned_from` for the first step). Anything else is
   `board_changed`: the flow stops, and resuming is refused until the user decides.
4. **Checkpoint.** `ensure_checkpoint(project, "full flow <run_id>: before <step>")`, on
   the blocking pool as B's guard runs it. The first one is recorded as `way_back.start`.
   It is reused when nothing changed since the last one, as B reuses one.
5. **The step.** The tool's handler from the registry, through `RegistryCaller`, with
   exactly the arguments shown at the gate. The whole result is written to
   `<flow>/steps/<n>-<tool>.json` and classified:
   - `Succeeded`.
   - `Failed`: refused, nothing changed.
   - `MayHaveChanged`: the error kind is `mutation_outcome_uncertain` or
     `ipc_outcome_unknown`, or the handler returned an `Err` from a writing tool. An
     `Err` is not evidence that nothing changed.
6. **Verified save.** `save_project`, then the live text and the saved file are both read,
   and their canonical SHA-256 must be equal. `save_project` takes no arguments and says
   only "Board saved successfully."; P0's one-board guard, checked again in step 2, makes
   it this board. The saved file's hash becomes the step's `after`. The chain runs on saved
   files because `board_identity` promises live-equals-file only within one KiCad session.

### When a step fails

On resume, each writing step first asks whether it is already done, from the board itself
(`already_done`, FF15). A record lost between the call and `flow.json` then costs nothing:

- **L2:** a fresh dry run says `noop`.
- **L3:** every footprint of `apply_with` is at its target.
- **L4:** a zone of that net, layer and outline exists. Only the missing ones are added.
- **F1:** the board holds the session's tracks and vias.
- **F2a, F2b:** each round records its own `after`, so a resume continues at the next
  round.
- **F3:** E's dry run reports every rule as `unchanged`.

| Outcome | Flow state | What the result says | Resume |
|---|---|---|---|
| A refusal before any live step (pre-flight, scratch work, a gate) | `stopped` | why; the board is unchanged | allowed once the cause is fixed; the leg starts again in a new attempt folder |
| `Failed` at a live step | `stopped` | `done_so_far` (the steps before, saved); `way_back.start` and `way_back.before_step`, each as `restore_checkpoint`'s preview then its apply with `expected_revision`, with the board closed in KiCad first | `already_done`, else the step runs again when the board is at its `before` |
| `MayHaveChanged`, or the server stopped during a call (D: `failed` with `STOPPED`, or `interrupted`; a blocking IPC write may still have landed) | `stopped`, `board_state: uncertain` | as above, plus "inspect the board in KiCad first" | `already_done`, else the step runs again only when the board is at its `before`, else refused: restore or abandon |
| The step succeeded, and the save failed or did not verify | `stopped`, `board_state: unsaved` | the step's result, and that KiCad holds the change unsaved | the save alone is tried again |
| R2 has no winner, or the route or judge errs | `stopped` | the route's own error. The board holds L2 to L4 and no routing. The result says so plainly, with `way_back.start` | allowed; starts again from R1 in a new attempt folder |
| `board_changed` | `stopped` | which `after` was expected, and what was found | refused; the user restores or abandons |

- **No write is retried on its own.** An IPC drop after a write was sent cannot say whether
  KiCad applied it. `resume` decides from the board (FF7, FF15).
- **The flow never restores a checkpoint.** `restore_checkpoint` needs KiCad closed and
  puts back every design file. It is the user's call, and the result gives the exact
  arguments.
- **Each live step is one KiCad undo entry** (or one per zone). Ctrl-Z works while the
  board is open. The checkpoints matter once KiCad has saved.
- **About 8 checkpoints per flow.** They are reused when nothing changed. Nothing deletes
  them, as today; the final report counts them.
- **A board with unsaved changes is refused at the start** (FF6). Otherwise the first
  checkpoint could not hold the board the user is looking at.
- **One flow per board.**
  - Between legs, `flow.json` is the record. `<state>/full-flow/.boards/<16 hex>.json`
    names the open flow. It is made with `create_new`, so two starts cannot both make it.
  - A second start on that board is refused and names the open flow.
  - The board is also refused while a `plan_convergent_placement` run holds it, and
    `plan_convergent_placement` gets the reverse check: one line in its start (FF12).
- **The leg handoff.** A leg writes `awaiting_answer` to `flow.json` a moment before D
  marks the leg done and frees its claim. So an answer first waits for that leg's `Report`
  to end, up to 10 s, before it claims the board for the next leg.

## Summaries

Each step leaves one record in the Row HS style:

```json
{ "step": "L2", "title": "Old routing deleted", "state": "done",
  "why": null, "counts": { "tracks": 0, "arcs": 0, "vias": 0, "kept_locked": 0 },
  "checkpoint": "<id>", "result_file": "steps/02-delete_all_routing.json",
  "not_checked": [] }
```

- `state` is one of `done`, `skipped`, `failed`, `uncertain`, `not_run`. `why` is set for
  every state but `done`.
- `counts` holds only what the step's own result reports. F derives no number of its own.
- `not_checked` comes from the step's own result:
  - `completion.not_checked` of the route;
  - the placement notes that say a check could not run (no decoupling pair found, an
    unreadable schematic);
  - a pour fill check that is not clean;
  - "placement judged with the earlier pour roles", when the roles changed at gate A;
  - "the search ran without the locked routing", when the board has locked routing that
    stays;
  - "left for you: <tool>", for every next step F does not run.

The final result (`status: done`) carries:

- `summary.lines`: at most 8 short sentences, in the order of the steps, one per step that
  did or skipped something. Example: "Placement: 7 parts moved." "Pours: 2 zones added;
  the fill check is clean." "Routing: every judge gate passed; the pours went as exported
  and won." The route's `summary.lengths.line` and `summary.pours.line` are quoted as they
  are.
- `summary.done`, `summary.skipped` (each with `why`), and `summary.not_checked` (the
  union, each item with its step).
- `placement`: the winner's `moves` and `change`, as Row HS gives them.
- `routing`: the route's `status`, `summary` and `completion.judged.gates`, as they are.
- `drc`: F4's unconnected items and violations.
- `way_back`: the start checkpoint, with `restore_checkpoint`'s preview and apply
  arguments.
- `records`: the flow folder.

The response stays under the response budget (PR 3). The full step results are files in
the flow folder, and the response names them.

## Run time and the MCP timeout

| Part | Typical time | Source |
|---|---|---|
| Pre-flight, gate R | seconds | two live reads |
| P2 and P4, each | under a minute to a few minutes | KiCad's refill and DRC with and without the proposals, up to two fills (PP16) |
| P3, the placement search | 10 to over 30 minutes | Row CT: 729 s on the public fixture at `m`; over 30 minutes on one of the owner's boards at `h` (Row AP2) |
| L2 to L4 | about a minute | IPC writes, refills, verified saves |
| R1 to R3 | the effort's budget plus the judge: `m` 240 s, `h` 720 s, `xh` 2,400 s, each with several minutes of judging and pruning | Row BN |
| F1 to F4 | a few minutes | the apply's candidate DRC, refills, stitching rounds |

- **Every call stays within D's limits.** A start or answer call returns once its leg has
  started. A report waits at most `wait_s`, which is capped at 600 s (`MAX_WAIT`).
- **Each report carries** `phase`, `elapsed_s` and the current step's progress:
  - P3: D's `candidates` counts;
  - R2: the route search's run count;
  - otherwise: the step name.
- **A report suggests `wait_s: 300`**, as D's does, because a stdio server answers one call
  at a time.
- **The legs run on D's own runtime.** A server that ends stops them (D's
  `stop_background_runs`), and their router processes are killed. `flow.json` keeps the
  step, and `resume` continues from it through `already_done`.

## What stays a separate tool

Everything that exists stays, unchanged in what it does. F calls the tools; it does not
replace them.

- **The step-by-step tools** stay, for that path and for partial work:
  `plan_convergent_placement`, `plan_pours`, `delete_all_routing`, `export_specctra_dsn`,
  `route_specctra_dsn`, `plan_specctra_ses_import`, `apply_specctra_ses`,
  `plan_route_refinement`, `route_batch` and `create_custom_drc_rule`.
- **Rule decisions stay the user's, before the flow.** F reads these rules and routes at
  them. It never invents a width or a group.
  - Netclass widths: `create_netclass`, live since E.
  - Net-to-class assignment: `assign_net_to_class`, which still refuses while KiCad holds
    the board (E).
  - Length groups and pair skews: `set_design_intent`.
- **Board setup and recovery** stay separate too: the board outline, locks
  (`set_footprint_lock`; the search holds locked parts), keepouts, the schematic sync, and
  `restore_checkpoint`.
- **Freerouting.** `engine: "freerouting"` stays on `route_specctra_dsn` only. F routes
  with fastroute, the default engine.

## Decisions taken in this design (with the reason)

| # | Decision | Reason |
|---|---|---|
| FF1 | A tracked job on D's module, one D run per leg between gates. The flow's own record is `flow.json` | The run time is beyond any MCP limit, and D's module already gives runs that report, stop with the server, and read back after a restart. Legs, not one run that waits: a run that waited hours for an answer would hold the board claim, and a restart would lose it. A leg ends at a gate, and `flow.json` survives |
| FF2 | Every write goes through the registry's handler of the public tool (`RegistryCaller`), with the arguments shown at the gate. Before the handler it runs what `dispatch_tool` runs: the required-argument check and the input validator. It records the call in the observer | One write path: B's guard, C's revision check, E's live writes and every validation apply as they do by hand. `route_judge`'s `call_tool` is the precedent for an in-process registry call with an `Arc<ToolContext>` from a background future. A test fake replaces the caller. F's own code never edits board text that reaches the user's project |
| FF3 | The final route runs on the live board after L4, at the user's `effort`. The placement candidates' routes stay inside the search | The import binds the exported revision. The user's effort is the quality they chose |
| FF4 | The candidates are routed, and the unrouted yardstick measured, on boards with P2's zones added. `WithPours` adds them to every candidate's text and delegates to a second `PlaceRouteCall` whose `inputs.board` is a zoned copy of the scratch board. The search's model, its input SHA-256 and `rebuild` keep the board without them | The candidates are judged by the routing they will really get. The pour key (the default `routing.rank_key`) can rank only pours the DSN has. `PlaceRouteCall::unrouted` checks that its text is its input board on disk, so the yardstick must be the zoned copy, compared like with like. Generation and the model stay Row CT's, so the standalone tool is unchanged. Measured in the build, both ways (owner decision 4) |
| FF5 | `pour_roles` in one argument, on F and on `plan_pours`, not saved | A role is not a rail: a rail needs a voltage the user may not know. One argument gives the confirmation in one place. Saving stays `set_design_intent`'s job |
| FF6 | A live board with unsaved changes is refused at the start | B's checkpoint holds saved files. Saving the user's unsaved work for them would hide it inside "before the flow" |
| FF7 | Reads are tried 3 times in all, 5 s apart. Writes are never retried | An IPC drop after a write cannot say whether KiCad applied it. The board can (FF15) |
| FF8 | A save after every live step, verified by comparing the live text with the saved file | It makes each checkpoint (saved files) the board after the step before, so every way back is exact. `save_project` reports nothing to rely on |
| FF9 | Old routing is counted at P1 and deleted at L2 only when a fresh dry run gives the same counts | The user approved a number. A different number means the board changed |
| FF10 | Plane stitching (F2a) adopts every stitching proposal of the registry planner, for at most 4 rounds. Reference vias (F2b) get one pass, and a refusal there is left to the user. Both are covered by the routing approval | F2a is the skill's procedure for the plane-stitching vias the judge added and the session lacks; it uses the same strict planner the steps name. The judge's reference round used a lenient profile (`plan_reference_stitching`) that the registry tool does not offer, so F2b can only try, and must not fail a flow whose session is already applied |
| FF11 | No automatic restore, and no automatic retry of the whole flow | Both are destructive or costly. The result gives the exact arguments |
| FF12 | P0 checks both job kinds' claims, and `plan_convergent_placement` checks F's | Two runs that write one board would race. D's claim covers one kind only |
| FF13 | The tool lives in the `integration` toolset, registered with an `Arc<ToolContext>` handler. `arc_tool!` moves from `integration.rs` to `tools/mod.rs`; its name keeps the `tool` ending that `schema_parameter_usage` looks for | Its longest leg is the route. It calls the other toolsets' tools through the catalogue, so the user does not need to load them. Its legs need the `Arc` |
| FF14 | One numbered attempt folder per attempt at a scratch or route step. The located fastroute path is kept in `flow.json` | The export, the route and `scratch_project` never replace an output, and `locate_fastroute` makes its own probe folder |
| FF15 | Resume asks each writing step whether it is already done, from the live board (`already_done`). It does not compare a hash it may never have written | A crash between a call and the record leaves no `after` to compare. The board itself says whether the deletion, the placements, the zones, the session or the rules are there |
| FF16 | Gate B reads `completion.judged.all_gates_pass`, not `status` | `judged_status` keeps the router's status whenever the judge added stitching the session lacks, so `apply_if_complete` on `status` would stop exactly where F2 should run |
| FF17 | P4 ends with `place_route::export_candidate` on the board F will build (the winner with P4's zones), whenever routing will run | Nothing destructive happens until the export is known to accept the new board. The export refuses more than unlocked routing |

## Decisions for the owner

Answered 2026-11-27: all four as recommended ("Yes, all four").

1. **The tool's shape.** One tracked job that writes the board itself, only at the points
   you approved, through the same tools the AI uses by hand. *Recommended: yes.*
   - This amends "a planner never writes" for this one tool. That rule is Q1, Row PS
     decision 1, Row CT, and Row ZR decision 4 ("the tool only proposes").
   - The alternative, a planner that returns a list of steps, keeps those decisions as
     they are. It leaves the many calls and the IPC drops.
2. **What one yes covers.** *Recommended: yes.*
   - One stop after the plan: the placement, the old routing, the pours.
   - The routing applied without asking when every judge gate passes, with its stitching
     and length rules.
   - The flow stops again only when a judge gate failed.
3. **Pour roles.** A `pour_roles` argument, asked in the first seconds before the long
   search, also added to `plan_pours`, and not saved in the design intent.
   *Recommended: yes.*
4. **Pours in the placement search.** The placement candidates are judged with the planned
   pours on the board. The build measures both ways on public fixtures. If "with pours" is
   not at least as good, it ships "without" and tells you. *Recommended: yes.*

## Modules

| File | What changes |
|---|---|
| `crates/konnect-core/src/full_flow.rs` (new) | The flow's model, with no input or output: steps, approvals, gates, `flow.json`, revisions, the protocol's decisions, summaries |
| `crates/konnect-core/src/full_flow_tests.rs` (new) | Its tests |
| `crates/konnect-core/src/tools/full_flow_tool.rs` (new) | The tool: calls, legs, `RegistryCaller`, `WithPours`, the adapters to B to E |
| `crates/konnect-core/src/tools/full_flow_tool_tests.rs` (new) | Its tests, with a fake `FlowCaller` and a fake `RouteCall` |
| `crates/konnect-core/src/tools/pour_plan_tool.rs` | the `pour_roles` argument; `plan_pours_in` split so that a text can be planned (`plan_text`), returning the final plan beside the result |
| `crates/konnect-core/src/pour_plan.rs` | `with_roles`, `rails_of` |
| `crates/konnect-core/src/tools/placement_converge.rs` (D's version) | `prepare_search` (the run setup the handler builds inline); `plan_with_outcome` (`plan_and_judge_observed` returning the `Outcome` beside the body); `live_input`, `LiveInput` and `locate_fastroute` made `pub(crate)`; the busy check of F's kind at the start |
| `crates/konnect-core/src/jobs.rs` (D's) | `write_json` made `pub(crate)` |
| `crates/konnect-core/src/tools/board_safety.rs` (B's) | `liveness`, `Liveness` and `Outcome::of` made `pub(crate)` |
| `crates/konnect-core/src/tools/route_fastroute.rs` (E's version) | the length rules step carries `tool: "create_custom_drc_rule"` and `arguments: {board, rules, replace_existing: true}` beside its prose line. This is additive, an `API_MIGRATIONS.md` entry |
| `crates/konnect-core/src/mcp/handler.rs` | `first_missing_required` and the observer helpers (`call_status`, `new_call_id`, `unix_ms`) made `pub(crate)` for `RegistryCaller` |
| `crates/konnect-core/src/tools/mod.rs`, `tools/integration.rs`, `router/registry.rs`, `lib.rs` | `arc_tool!` moved; registration; the tool count |
| `crates/konnect/assets/skills/kicad-pcb/SKILL.md`, `tool-directory.md`, `docs/API_MIGRATIONS.md`, `docs/CREATION_HANDOFF.md` | the flow and its gates; the new tool and argument; Row FF |
| `crates/konnect/tests/asset_references.rs` | the response's new words in `NOT_TOOLS` (run `asset_references` and `schema_parameter_usage` early) |

## Skeletons

Bodies say what they must do. `Result` is `anyhow::Result` unless named.

### `full_flow.rs`

```rust
//! Row FF: one call from placement through pours to routing. The flow's model, with no
//! input or output, so that every decision is tested without KiCad, fastroute or a clock.
//! The tool (`tools::full_flow_tool`) does the work and asks this module what comes next.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Every step of the flow, in order. `id` is the design table's letter and number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum Step {
    Preflight,        // P0
    CountRouting,     // P1
    PoursBefore,      // P2
    Placement,        // P3
    PoursAfter,       // P4 (with the export check)
    Unchanged,        // L1
    DeleteRouting,    // L2
    ApplyPlacement,   // L3
    AddZones,         // L4
    Export,           // R1
    Route,            // R2
    ImportCheck,      // R3
    ApplySession,     // F1
    PlaneStitch,      // F2a
    ReferenceVias,    // F2b
    LengthRules,      // F3
    Finish,           // F4
}

impl Step {
    /// The design's id ("P0" … "F4") and a title for the summary.
    pub(crate) fn id(self) -> &'static str;
    pub(crate) fn title(self) -> &'static str;
    /// Whether the step changes the live board or the project files: L2–L4, F1–F4. Each
    /// of these runs the live-step protocol.
    pub(crate) fn writes(self) -> bool;
}

/// Where the flow can stop for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Gate {
    /// Gate R: a role that comes only from a net's name, before the long search.
    Roles,
    /// Gate A: the plan (placement, old routing, pours, the export check).
    Plan,
    /// Gate B: the routing, when its approval asks for it.
    Routing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlacementAnswer { Apply, KeepCurrent }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeleteAnswer { Delete, DeleteIncludingLocked, Keep }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PoursAnswer { Add, Skip }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RoutingAnswer { ApplyIfComplete, ApplyBest, Ask, Skip }

/// A role the user gives one net.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GivenRole { Ground, Power, NotASupply }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RolesAnswer {
    /// `"as_named"`: every role that comes from a name is confirmed.
    AsNamed(AsNamed),
    /// Net to role; the nets not named keep their resolved role.
    Map(BTreeMap<String, GivenRole>),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AsNamed { AsNamed }

/// `approve`: every key optional; a missing key is asked at its gate.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Approvals {
    pub placement: Option<PlacementAnswer>,
    pub delete_routing: Option<DeleteAnswer>,
    pub pours: Option<PoursAnswer>,
    pub pour_roles: Option<RolesAnswer>,
    pub routing: Option<RoutingAnswer>,
}

impl Approvals {
    /// `self` with every key of `answer` that is set. A key set in both with two values is
    /// an error naming it, except `pour_roles` at gate A (`gate == Gate::Plan`), which the
    /// answer replaces: the caller then plans the pours again (`roles_changed`).
    pub(crate) fn merged(&self, answer: &Approvals, gate: Gate) -> Result<Approvals, String>;
    /// Whether `answer` replaces the roles `self` holds with different ones.
    pub(crate) fn roles_changed(&self, answer: &Approvals) -> bool;
    /// "Routing runs": `steps.routing` is `Run` and `routing` is not `Skip`.
    pub(crate) fn routing_runs(&self, steps: &Steps) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunOrSkip { Run, Skip }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlanOrSkip { Plan, Skip }

/// `steps`, as the call gives them (`"run"`, `"plan"`, `"skip"`); every field defaults to
/// running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Steps { pub placement: RunOrSkip, pub pours: PlanOrSkip, pub routing: RunOrSkip }

/// What the plan holds when gate A is decided: the facts the refusals and the
/// recommendations read. Built from the scratch legs' results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PlanFacts {
    /// Row HS's `change` of the winner: `none`, `small`, `placement`; `None` when the
    /// placement step was skipped or found no candidate.
    pub change: Option<String>,
    /// Unlocked tracks, arcs and vias (P1's `will_delete.total` without `include_locked`).
    pub unlocked_routing: usize,
    /// P1's `kept_locked.export_will_refuse` is set.
    pub locked_blocks_export: bool,
    /// Locked routing ends on a pad of a part the winner moves.
    pub locked_on_moved: bool,
    /// P4's zone count, and whether its fill check is clean.
    pub zones: usize,
    pub fill_clean: bool,
    /// P4's export check: `None` passed (or not run), `Some(reason)` refused.
    pub export_refused: Option<String>,
    /// Nets whose role comes from the name only, after the given roles.
    pub named_only: Vec<String>,
}

/// One key a gate still needs: its name, the recommended value (`None` when the plan
/// recommends nothing: a fill check that is not clean), and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Need { pub key: &'static str, pub recommended: Option<Value>, pub why: String }

/// The keys gate A needs, given `facts`, `steps` and what is approved. Placement is
/// needed when `change` is `small` or `placement` (none when `none`: the step is
/// skipped). Delete_routing is needed when any routing is on the board and placement or
/// routing will run. Pours are needed when `zones > 0`, routing when `steps.routing` is
/// `Run`. Recommendations as in "The approve object".
pub(crate) fn needs_at_plan(facts: &PlanFacts, steps: &Steps, approved: &Approvals) -> Vec<Need>;

/// Gate R's need: `pour_roles` when `named_only` is not empty and it is not approved.
pub(crate) fn needs_roles(named_only: &[String], approved: &Approvals) -> Option<Need>;

/// Whether gate B stops the flow. `Ask` always stops; `ApplyIfComplete` stops when
/// `all_gates_pass` (the route's `completion.judged.all_gates_pass`) is false or absent;
/// `ApplyBest` never stops; `Skip` never reaches the route (FF16).
pub(crate) fn stops_for_routing(approval: RoutingAnswer, all_gates_pass: Option<bool>) -> bool;

/// The refusals of "Combinations that are refused", each naming the key to change:
/// - placement applied with unlocked routing kept;
/// - routing runs (`Approvals::routing_runs`) with unlocked routing kept;
/// - routing runs with a refused export check;
/// - routing runs with `Delete` while `locked_blocks_export`;
/// - a role against a design-intent rail (`rails`: net to `"ground"` or `"power"`,
///   from `pour_plan::rails_of`);
/// - a role for a net the board does not have (`nets`).
///
/// Checked before any write.
pub(crate) fn refused_combination(
    approved: &Approvals,
    facts: &PlanFacts,
    steps: &Steps,
    rails: &BTreeMap<String, &'static str>,
    nets: &BTreeSet<String>,
) -> Option<String>;

/// `expected_plan_revision`: SHA-256 (hex) over the flow id, the gate, `planned_from`
/// and the canonical JSON of the gate's plan body.
pub(crate) fn plan_revision(flow_id: &str, gate: Gate, planned_from: &str, body: &Value) -> String;

/// Where the flow stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FlowState { Running, AwaitingAnswer, Stopped, Done, Abandoned }

/// What the board is known to hold when the flow stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BoardState { Unchanged, AsRecorded, Uncertain, Unsaved }

/// One step's record, in the Row HS style.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StepRecord {
    pub step: Step,
    pub state: StepState,
    /// Set for every state but `done`.
    pub why: Option<String>,
    /// Only what the step's own result reports (`counts_of`).
    pub counts: Value,
    pub checkpoint: Option<String>,
    /// The saved file's canonical SHA-256 before and after a writing step (after the
    /// verified save). `after` is `None` until the save is verified.
    pub before: Option<String>,
    pub after: Option<String>,
    /// For F2a and F2b: the `after` of each round done.
    pub rounds: Vec<String>,
    pub result_file: Option<String>,
    pub not_checked: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StepState { Calling, Done, Skipped, Failed, Uncertain, Unsaved, NotRun }

/// `flow.json`: the whole record of one flow, rewritten (D's `jobs::write_json`, durable)
/// before every call (the intent, `Calling`), at every step's end and at every gate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FlowRecord {
    pub flow_id: String,
    pub board: String,
    pub project: String,
    pub state: FlowState,
    pub board_state: BoardState,
    pub steps_asked: Steps,
    pub approvals: Approvals,
    pub effort: String,
    pub placement_effort: String,
    /// The fastroute binary P0 located (path, version, source).
    pub fastroute: Value,
    /// `canonical_sha256` of the live text when the plan was read.
    pub planned_from: String,
    pub gate: Option<Gate>,
    pub gate_body: Option<Value>,
    pub plan_revision: Option<String>,
    pub facts: Option<PlanFacts>,
    /// The step running or stopped at, and the attempt folder number it uses.
    pub at: Option<Step>,
    pub attempt: u32,
    pub records: Vec<StepRecord>,
    /// D's run ids of the legs, in order.
    pub legs: Vec<String>,
    /// The first checkpoint, the way back to before the flow.
    pub start_checkpoint: Option<String>,
    pub stop_reason: Option<String>,
}

/// What a writing step does before it runs (step 3 of "Each live step").
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LiveStart {
    /// The saved board is where the step before left it: checkpoint and run.
    Run,
    /// The board is neither there nor already past this step (`already_done` said no):
    /// stop with `board_changed`, naming the expected SHA-256.
    Changed { expected: String, found: String },
}

/// Step 3's decision: `found` is the saved file's canonical SHA-256 now, `expected` the
/// `after` of the step before (or `planned_from`).
pub(crate) fn plan_live_step(found: &str, expected: &str) -> LiveStart;

/// How a step's tool result reads, as B classifies it (`board_safety`'s `Outcome::of`),
/// with one addition: a handler `Err` from a writing tool is `MayHaveChanged`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome { Succeeded, Failed, MayHaveChanged }

/// Whether `resume` may run the stopped writing step again, given whether it is already
/// done (`done`, from `already_done` on the live board) and the saved file's SHA-256 now.
/// - Done: record it as done (with the board as found as its `after`) and go on.
/// - Not done, and the board is at `before`: run it again.
/// - A step `Unsaved`: only the save is tried again.
/// - Otherwise: the reason, naming the checkpoint to restore.
pub(crate) fn may_resume(record: &StepRecord, done: bool, found: &str) -> Result<Resume, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resume { RecordDone, RunAgain, SaveAgain }

/// The step after `done`, given what is asked, approved and found, or `End`. Skips by
/// the "Skipped when" column; a step whose approval is still missing yields the gate
/// instead (`Next::Gate`).
pub(crate) fn next(record: &FlowRecord, all_gates_pass: Option<bool>) -> Next;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Next { Step(Step), Gate(Gate), End }

/// The final summary. `lines`: at most 8, in the steps' order, one per step that did or
/// skipped something, with the route's `summary.lengths.line` and `summary.pours.line`
/// quoted as they are. Then `done`, `skipped` with `why`, `not_checked` with each item's
/// step, the checkpoint count, and `way_back`. Pure: reads only `record` and the step
/// results it is given.
pub(crate) fn summary(record: &FlowRecord, route_summary: Option<&Value>) -> Value;

/// One step's `counts`, from its own tool result only:
/// - L2: `removed`;
/// - L3: the placements sent;
/// - L4: the zones added;
/// - F1: the session's items as the apply reports them;
/// - F2a and F2b: rounds and vias adopted;
/// - F3: `rules` by action;
/// - F4: the DRC counts.
pub(crate) fn counts_of(step: Step, result: &Value) -> Value;
```

### `tools/full_flow_tool.rs`

```rust
//! `run_full_flow` (Row FF): the tool, its legs on `crate::jobs`, and the adapters to the
//! tools it calls. Every write is a public tool's own handler from the registry
//! (`RegistryCaller`), so B's checkpoint guard, C's revision check and E's live rule
//! writes apply exactly as by hand.

use crate::full_flow::*;
use crate::jobs::{JobKind, Jobs, Progress};
use crate::mcp::protocol::CallToolResult;
use crate::tools::{ToolContext, ToolDef};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The flows' own folder below the state folder. `<state>/full-flow/<flow_id>/` holds
/// `flow.json`, `steps/` and `attempts/<n>/` (each with `pours/`, `placement/`, `zoned/`,
/// `export-check/` and `route/` as the attempt needs them).
/// `<state>/full-flow/.boards/<16 hex>.json` names a board's open flow.
const FLOWS_FOLDER: &str = "full-flow";

/// The legs, as `crate::jobs` keeps them: one D run per leg, its folder
/// `run_folder_of(state, &FLOW_LEGS, leg_id)`, made empty by `start_leg`.
pub(crate) const FLOW_LEGS: JobKind = JobKind { folder: "full-flow-legs", tool: "run_full_flow" };

/// The `wait_s` a report suggests (D's value, for the same reason).
const SUGGESTED_WAIT_S: u64 = 300;

/// Reads of the live board: tries in all, and the pause between them (FF7).
const READ_TRIES: u32 = 3;
const READ_PAUSE: std::time::Duration = std::time::Duration::from_secs(5);

/// How long an answer waits for the leg before it to end (the leg handoff).
const HANDOFF_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// The most plane-stitching rounds (the skill's rule), and `route_batch`'s cap.
const STITCH_ROUNDS: usize = 4;
const BATCH_CAP: usize = 200;

/// Registered with `arc_tool!` (moved to `tools/mod.rs`), in the `integration` toolset.
pub(crate) fn tools() -> Vec<ToolDef>;

/// The call's form, as "Calls" lists them; any mix is refused, naming the argument of the
/// other form (as `placement_converge::read_request` does).
#[derive(Debug, Clone, PartialEq)]
enum Request {
    Start(StartCall),
    Report { run_id: String, wait: std::time::Duration },
    Answer { run_id: String, answer: Approvals, expected_plan_revision: String },
    Resume { run_id: String },
    Abandon { run_id: String },
}

#[derive(Debug, Clone, PartialEq)]
struct StartCall {
    board: PathBuf,
    effort: Option<String>,
    placement_effort: Option<String>,
    fastroute_path: Option<PathBuf>,
    steps: Steps,
    approve: Approvals,
}

fn read_request(args: &Value) -> Result<Request, CallToolResult>;

async fn handle_run_full_flow(args: &Value, ctx: Arc<ToolContext>) -> anyhow::Result<CallToolResult>;

/// Writes on the user's project go through here. A fake replaces it in the tests.
pub(crate) trait FlowCaller: Send + Sync + 'static {
    fn call(&self, tool: &'static str, args: Value) -> crate::route_search::BoxFuture<anyhow::Result<CallToolResult>>;
}

/// What `McpHandler::dispatch_tool` does, minus the transport. In order:
/// 1. the `ToolDef` from the catalogue (`route_judge`'s `find_tool`; the catalogue applies
///    B's `guard`);
/// 2. `first_missing_required`, then the definition's input validator (a refusal is a
///    `Failed` result, nothing sent);
/// 3. the handler with the server's `Arc<ToolContext>`;
/// 4. the call recorded in the observer as a nested call of `run_full_flow`.
///
/// A handler `Err` comes back as `Err`, and `outcome_of_call` reads it.
pub(crate) struct RegistryCaller { pub ctx: Arc<ToolContext> }
impl FlowCaller for RegistryCaller { /* as above */ }

/// P0, in the start call itself (seconds), as "P0 in full" lists it. Two details:
/// - Fastroute is located into `<state>/full-flow/.probe-<stamp>`, which is removed on a
///   refusal and moved into the flow folder otherwise.
/// - The roles are `supply_roles` over the board's nets, then `pour_plan::with_roles`
///   with `approve.pour_roles`.
///
/// Returns the record's start, or the refusal. On a refusal nothing is left behind: no
/// flow folder, no marker, no claim.
async fn preflight(ctx: &Arc<ToolContext>, call: &StartCall) -> anyhow::Result<Result<Preflight, CallToolResult>>;

struct Preflight {
    live: crate::tools::placement_converge::LiveInput,
    planned_from: String,
    project: PathBuf,
    fastroute: Value,
    named_only: Vec<String>,
    roles: crate::tools::SupplyRoles,
    rails: std::collections::BTreeMap<String, &'static str>,
}

/// The board's open-flow marker. `mark_open` makes it with `create_new`, and fails when
/// another flow holds it. `clear_open` removes it when the flow ends (done or abandoned),
/// never while it is stopped.
fn open_flow(state: &Path, board: &Path) -> anyhow::Result<Option<String>>;
fn mark_open(state: &Path, board: &Path, flow_id: &str) -> anyhow::Result<Result<(), String>>;
fn clear_open(state: &Path, board: &Path, flow_id: &str) -> anyhow::Result<()>;

/// Start the next leg as a D run: `claim_board(FLOW_LEGS)`, the leg's empty run folder,
/// and `Jobs::start` with `run_leg`. The leg's id is appended to `flow.json` before the
/// start returns.
fn start_leg(jobs: &Jobs, state: &Path, flow: FlowRecord, ctx: Arc<ToolContext>) -> anyhow::Result<Value>;

/// One leg: the steps from `flow.at` until a gate, a stop or the end, each through
/// `run_step`, with `flow.json` rewritten as the protocol says. Its D result is the flow's
/// view at the leg's end, which is what a report returns.
async fn run_leg(flow: FlowRecord, caller: Arc<dyn FlowCaller>, ctx: Arc<ToolContext>, progress: Arc<FlowProgress>) -> anyhow::Result<Value>;

/// A leg's progress: the step, and the step's own counts while it runs (P3: D's
/// `SearchProgress`; R2: the route search's runs so far).
pub(crate) struct FlowProgress { /* the step and an inner progress */ }
impl Progress for FlowProgress { fn snapshot(&self) -> Value; }

/// One step. A reading or scratch step runs as its table row says, in the attempt
/// folder. A writing step runs `live_step`. Returns its record.
async fn run_step(step: Step, flow: &mut FlowRecord, caller: &dyn FlowCaller, ctx: &Arc<ToolContext>, progress: &FlowProgress) -> anyhow::Result<StepRecord>;

/// The calls of one writing step. A fixed list for L2 to L4, F1, F3 and F4. A round
/// closure for F2a and F2b, whose calls depend on the planner's answer.
enum StepCalls {
    Fixed(Vec<(&'static str, Value)>),
    Rounds { max: usize, round: Box<dyn Fn(usize, &Value) -> Option<Vec<(&'static str, Value)>> + Send + Sync> },
}

/// The live-step protocol of "Each live step", steps 1 to 6:
/// 1. the intent in `flow.json`;
/// 2. `liveness` with `READ_TRIES`;
/// 3. `plan_live_step`;
/// 4. `ensure_checkpoint` on the blocking pool;
/// 5. the calls through `caller`, each result written and classified (`outcome_of_call`),
///    the first that is not `Succeeded` ending the step;
/// 6. `verified_save`, giving `after` (for rounds, after each round).
async fn live_step(step: Step, calls: StepCalls, flow: &mut FlowRecord, caller: &dyn FlowCaller, ctx: &Arc<ToolContext>) -> anyhow::Result<StepRecord>;

/// `save_project`, then the live text (IPC) and the saved file. Both are read and their
/// `canonical_sha256` compared. `Ok(hash)` when equal; else the step is `Unsaved`.
async fn verified_save(board: &Path, caller: &dyn FlowCaller, ctx: &Arc<ToolContext>) -> anyhow::Result<Result<String, String>>;

/// B's `Outcome::of` for a result. An `Err` from the handler of a writing tool is
/// `MayHaveChanged`; an `Err` before the handler (the validator) is `Failed`.
fn outcome_of_call(call: &anyhow::Result<CallToolResult>, reached_handler: bool) -> Outcome;

/// FF15: whether a writing step is already done, from the live board and the step's
/// arguments. L2: a fresh dry run says `noop`. L3: every footprint of `apply_with` is at
/// its target position, side and turn. L4: a zone of each proposal's net, layer and
/// outline exists (and which are missing). F1: the board holds the session's tracks and
/// vias. F3: E's dry run reports every rule `unchanged`. F2a and F2b answer through their
/// `rounds`.
async fn already_done(step: Step, flow: &FlowRecord, caller: &dyn FlowCaller, ctx: &Arc<ToolContext>) -> anyhow::Result<bool>;

// ─── The scratch steps ───────────────────────────────────────────────────────

/// P2 and P4: `pour_plan_tool::plan_text` on `text` (P2: the stripped live text; P4: the
/// winner's text from `place_search::rebuild`), with `roles`, in
/// `<attempt>/pours/<p2|p4>/`. Returns the plan's result body (as `plan_pours` returns it)
/// and its final proposals (after PP16's thermal step and the withdrawal of unfilled
/// ones).
async fn plan_pours_on(ctx: &Arc<ToolContext>, board: &Path, text: &str, roles: &crate::tools::SupplyRoles, folder: &Path) -> anyhow::Result<PoursOn>;

struct PoursOn { body: Value, zones: Vec<crate::pour_plan::ProposedZone> }

/// P3, in `<attempt>/placement/`. Row CT's search through `prepare_search` and
/// `plan_with_outcome`. When P2 proposed zones, the routing is `WithPours` (FF4).
/// Returns `plan_convergent_placement`'s result body and the winner's board text
/// (`place_search::rebuild` of the outcome's winner record, without P2's zones). `None`
/// when there is no winner.
async fn search_placement(ctx: &Arc<ToolContext>, flow: &FlowRecord, live: &str, zones: &[crate::pour_plan::ProposedZone], progress: &FlowProgress) -> anyhow::Result<Placed>;

struct Placed { body: Value, winner_text: Option<String> }

/// FF4. `route`: the candidate's text with `zones` added (`pour_plan::with_zones`), handed
/// to `zoned`. `unrouted`: checks that the text it is given is the model's input board
/// (`plain_text`), then hands `zoned_text` to `zoned`. `zoned` is a `PlaceRouteCall` whose
/// `inputs.board` is `<attempt>/zoned/<board>.kicad_pcb` holding `zoned_text`, so
/// `PlaceRouteCall`'s own on-disk check holds. The search's model, its input SHA-256 and
/// `rebuild` never see the zones.
pub(crate) struct WithPours {
    pub zoned: Arc<dyn crate::place_search::RouteCall>,
    pub zones: Vec<crate::pour_plan::ProposedZone>,
    pub plain_text: String,
    pub zoned_text: String,
}
impl crate::place_search::RouteCall for WithPours { /* as above */ }

/// FF17: `place_route::export_candidate` on `text` (the winner with P4's zones; the
/// locked routing the flow keeps is not in it, and `not_checked` says so), in
/// `<attempt>/export-check/`. `Ok(())` or the export's reason.
async fn export_check(ctx: &Arc<ToolContext>, flow: &FlowRecord, text: &str) -> anyhow::Result<Result<(), String>>;

// ─── The adapters to B to E: one place each, pinned by a test ────────────────

/// C: `delete_all_routing`'s dry run (P1, L2's fresh one, and `already_done`), the apply,
/// and its counts. `include_locked` follows `DeleteIncludingLocked`.
fn delete_preview_args(board: &Path, include_locked: bool) -> Value;
fn delete_apply_args(board: &Path, include_locked: bool, plan_revision: &str) -> Value;
/// The approved counts against a fresh dry run's: `will_delete.total`, `.tracks`,
/// `.arcs`, `.vias` and `kept_locked` equal, or the reason (FF9).
fn same_deletion(approved: &Value, fresh: &Value) -> Result<(), String>;

/// D/CT: the winner's `apply_with` (the exact `set_component_placements` arguments).
fn placement_args(placement_body: &Value) -> Option<Value>;

/// ZR4: `add_zone` once per proposal (only the missing ones on a resume), from
/// `pour_plan::add_zone_arguments`, then `refill_zones`.
fn zone_calls(board: &Path, zones: &[crate::pour_plan::ProposedZone]) -> Vec<(&'static str, Value)>;

/// R1 to R3 and F1, in `<attempt>/route/`:
/// - `export_specctra_dsn(board, output=board.dsn)`;
/// - `route_specctra_dsn(dsn_path, ses_output_path=routed.ses, effort, fastroute_path)`,
///   with `fastroute_path` from `flow.json`;
/// - `plan_specctra_ses_import(board, ses_path, manifest_path)`;
/// - `apply_specctra_ses(..., candidate_output_path=candidate.kicad_pcb)`.
fn export_args(board: &Path, route: &Path) -> Value;
fn route_args(route: &Path, effort: &str, fastroute: &Path) -> Value;
fn import_check_args(board: &Path, route: &Path) -> Value;
fn apply_session_args(board: &Path, route: &Path) -> Value;

/// F2a, F2b and F3: the steps of the route's `completion.next_steps` this flow runs.
/// - Plane stitching: the `plan_route_refinement`, `route_batch` and `run_drc` steps of
///   the plane rounds, taken with their arguments as given.
/// - Reference vias: the `reference_steps` group, recognised by its `nets` and
///   `search_radius_mm`.
/// - The rules: the structured length-rules step (`tool: "create_custom_drc_rule"`).
///
/// Any other step is not run, and is listed in `not_checked` as "left for you: <its
/// text>".
fn route_follow_ups(route_result: &Value) -> FollowUps;

struct FollowUps { plane: Option<Vec<Value>>, reference: Option<Vec<Value>>, rules: Option<Value>, left: Vec<String> }

/// The round closures of F2a and F2b. A round is the planner's call; then, when it
/// proposes vias, `route_batch` calls of at most `BATCH_CAP` objects, and `run_drc` with
/// refill. F2a stops when the planner proposes nothing or the DRC finds 0 unconnected
/// items on its nets, after at most `STITCH_ROUNDS`. F2b runs one round, and a planner
/// refusal ends it as "left for you" (FF10).
fn plane_rounds(steps: &[Value]) -> StepCalls;
fn reference_round(steps: &[Value]) -> StepCalls;

// ─── Reports, answers, resume ────────────────────────────────────────────────

/// A report: `flow.json`, and the running leg's `Jobs::report` with `wait` (at most
/// `jobs::MAX_WAIT`). The view: `state`, `phase` (the step's title), `elapsed_s`, the
/// progress, the gate with `needs` and `answer_with` when awaiting an answer, the summary
/// when done, and `next_steps`.
async fn report(state: &Path, run_id: &str, wait: std::time::Duration) -> CallToolResult;

/// An answer, in order:
/// 1. the flow awaits one, and its last leg has ended (`HANDOFF_WAIT`);
/// 2. `expected_plan_revision` is the current one;
/// 3. `Approvals::merged`, then `refused_combination`;
/// 4. when the roles changed at gate A, P4 again with its export check, and gate A again
///    if the proposals changed; else the next leg.
async fn answer(state: &Path, run_id: &str, approvals: Approvals, revision: &str, ctx: Arc<ToolContext>) -> anyhow::Result<CallToolResult>;

/// A resume. The flow must be stopped and no leg running. For a writing step:
/// `already_done`, the saved file's SHA-256 (with `READ_TRIES`), then `may_resume`. For a
/// reading or scratch step: the leg again, in a new attempt folder.
async fn resume(state: &Path, run_id: &str, ctx: Arc<ToolContext>) -> anyhow::Result<CallToolResult>;

/// An abandon. Refused while a leg runs, because D has no per-run stop. Otherwise
/// `flow.json` says `abandoned`, the open marker is cleared, and the result names the
/// start checkpoint with `restore_checkpoint`'s preview and apply. Nothing on the board
/// is undone.
fn abandon(state: &Path, run_id: &str) -> anyhow::Result<CallToolResult>;

/// The gate's view:
/// - the plan body, as "What gate A shows" lists it;
/// - `needs` and `plan_revision`;
/// - `answer_with`: the answer call with every needed key at its recommended value. A key
///   with no recommendation is left out, and `needs` says the user decides.
fn gate_view(flow: &FlowRecord, gate: Gate, needs: &[Need]) -> Value;
```

### `tools/placement_converge.rs` (on D's version)

```rust
/// What the handler builds inline today before `start_run`: the run folder's scratch
/// project (`scratch_project`), the located fastroute, the private KiCad settings folder,
/// `RouteInputs`, `tool_plan`, `isolated_context` and the `PlaceRouteCall`. Shared by the
/// tool and F, so that F's search is Row CT's search. `text` is the stripped board text
/// the search plans; `fastroute` is a binary already located.
pub(crate) fn prepare_search(
    ctx: &ToolContext,
    folder: &Path,
    board: &Path,
    live: &LiveInput,
    effort: Effort,
    fastroute: &Path,
) -> anyhow::Result<PreparedSearch>;

pub(crate) struct PreparedSearch {
    pub plan: Plan,
    pub inputs: RouteInputs,
    pub routing: Arc<dyn RouteCall>,
}

/// `plan_and_judge_observed`, also returning the search's `Outcome`, so a caller can
/// `place_search::rebuild` the winner's board text. The tool keeps calling
/// `plan_and_judge_observed`, which becomes this with the outcome dropped.
pub(crate) async fn plan_with_outcome(
    plan: &Plan,
    routing: Arc<dyn RouteCall>,
    board: &Path,
    planned: &Planned,
    progress: &SearchProgress,
) -> anyhow::Result<(Value, Outcome)>;
```

### `pour_plan.rs` and `tools/pour_plan_tool.rs`

```rust
/// The roles of `roles`, with each net of `given` set to its role and its source `"user"`.
/// `NotASupply` removes the net from both lists. A net of `given` that is a design-intent
/// rail of the other role is an error naming the rail: the intent wins.
pub(crate) fn with_roles(
    roles: &crate::tools::SupplyRoles,
    given: &crate::full_flow::RolesAnswer,
    rails: &BTreeMap<String, &'static str>,
) -> Result<crate::tools::SupplyRoles, String>;

/// The nets whose role comes from a design-intent rail (their source in `roles` says
/// so), with that role.
pub(crate) fn rails_of(roles: &crate::tools::SupplyRoles) -> BTreeMap<String, &'static str>;

/// `plan_pours_in`'s steps 3 to 7 on a given board text (`text`, named `board` in the
/// arguments it returns), with `roles` in place of the resolved ones when given. The tool
/// calls it with the text it read, F with a scratch text. The result is the tool's,
/// unchanged, beside the final plan: the proposals after PP16's thermal step and
/// `withdraw_unfilled`, the ones whose `add_zone_arguments` the result carries. `None`
/// when nothing was proposed or the check failed.
pub(crate) async fn plan_text(
    ctx: &ToolContext,
    board: &Path,
    text: &str,
    roles: Option<&crate::tools::SupplyRoles>,
    state: Option<&Path>,
) -> anyhow::Result<(CallToolResult, Option<PourPlan>)>;
```

`plan_pours` gets `pour_roles` with the same schema as F's. The next step "Confirm them
with the user" then names it: "…or call again with `pour_roles`".

## Tests, each guard with a negative control

Every guard below gets a test and a negative control: the guard broken by hand, its test
run, and the test must fail.

- **Calls:** each form alone is accepted. Each mix of two forms is refused, naming the
  argument. `wait_s` above 600 is refused. `steps` takes only its strings. `abandon` is
  refused while a leg runs.
- **P0 refusals:** a second board open; unsaved changes; no `.kicad_pro`; a flow open on
  the board; a placement run on the board; fastroute missing. Each refusal leaves nothing
  behind: no flow folder, no probe folder, no marker, no claim. Two starts at once: only
  one makes the marker.
- **Gate R:** a role from a name with no `pour_roles` stops before any leg starts.
  `"as_named"` and a map each go on. A role against a rail is refused.
- **Gate A:**
  - `needs_at_plan` for each `change` value, with and without routing, with a clean and
    an unclean fill check, and with locked routing that blocks the export;
  - every pre-approved key is not asked again;
  - `answer_with` sent back unchanged is accepted;
  - an older revision is refused;
  - a second value for a key is refused, except `pour_roles`, which plans the pours again.
- **Refused combinations:** each of the six, keyed on `routing_runs`, and none of them
  makes a `FlowCaller` call (the fake records none).
- **Order:** with the fake caller and every key approved at the start, the calls come in
  exactly this order: L2, L3, L4, R1 to R3, F1, F2a, F2b, F3, F4, with a verified save
  after each writing step and before each stitching round. Every skip condition of the
  table holds.
- **The live-step protocol:**
  - the intent is written before the call;
  - `plan_live_step`'s two answers;
  - the checkpoint before each writing step;
  - the first non-`Succeeded` result ends the step;
  - a handler `Err` gives `uncertain`;
  - a save that does not verify gives `unsaved`;
  - a failed read is tried 3 times in all, and never a write.
- **FF9:** a fresh dry run with other counts stops L2 before the apply.
- **Resume:**
  - `already_done` for each writing step, from a board where the step landed and its
    record did not;
  - `RunAgain` only at `before`;
  - `SaveAgain` for `unsaved`;
  - `board_changed` refuses;
  - a resume of R2 uses a new attempt folder.
- **`WithPours`:**
  - every board `route` hands on holds the zones;
  - `unrouted` hands on the zoned copy, and refuses a text that is not the model's input;
  - the model's text and `rebuild`'s winner text hold no zone.
- **FF17:** a winner whose export is refused blocks routing at gate A with the export's
  reason.
- **Gate B:** `stops_for_routing` for each value, with `all_gates_pass` true, false and
  absent. A route whose judge needed stitching (so its `status` stays the router's) does
  not stop under `apply_if_complete` when every gate passed.
- **Follow-ups:**
  - `route_follow_ups` takes only the three groups, and lists every other step as left,
    never called;
  - F2b's planner refusal does not fail the flow;
  - `route_batch` calls never exceed 200 objects.
- **Summary:** the 8-line cap; every skipped step has a `why`; `not_checked` carries the
  route's `completion.not_checked`, the "earlier pour roles" note when the roles changed at
  gate A, and the locked-routing note.
- **Adapters:** one test per adapter pins its arguments against the B to E tools'
  current schemas (the registry's validator), so a change in their review fails here
  first. The structured length-rules step has its own test in `route_fastroute`.
- **The konnect crate:** `asset_references`, `schema_parameter_usage`,
  `creation_guidance` and `doc_tool_counts`.

`cargo mutants` runs once, narrowly, on these guards. Only the misses are run again.

- In `full_flow.rs`: `needs_at_plan`, `refused_combination`, `plan_live_step`,
  `may_resume`, `stops_for_routing`, `Approvals::merged` and `routing_runs`.
- In the tool: `same_deletion`, `outcome_of_call`, `already_done` and `verified_save`.

## Measurement (public fixtures only)

1. **End to end without KiCad.** One ignored test runs the whole flow on Row CT's public
   placement fixture at effort `m`, with the real fastroute, kicad-cli and judge. Its fake
   `FlowCaller` applies the writes to a scratch copy in file mode. It records the time of
   each step and the final summary.
2. **A live smoke on a public fixture.** The same fixture opened in KiCad, served by the
   fork's release binary by pipe. It covers the live-only tools the fake leaves out:
   `delete_all_routing`, `export_specctra_dsn`, `plan_specctra_ses_import`,
   `apply_specctra_ses` and the verified save. Opening KiCad is the owner's step.
3. **FF4, owner decision 4.** The placement search with and without `WithPours`, on the
   same fixtures, at effort `m`. The candidates are compared by the route judge's pour key,
   and so are the final routes after P4. "With pours" ships only if it is at least as good.
   Both ways go in the record, under `target/creation-research/full-flow/`.
4. **A live run on one of the owner's boards.** On a copy, open in KiCad, only on his
   word. Counts only in any document.

## What this design does not do

- **It changes nothing in the tools it calls**, with three exceptions:
  - `plan_pours`'s `pour_roles` argument;
  - the structured length-rules step;
  - the refactors listed under "Modules", which change no behaviour.

  Row CT's search, `plan_pours`'s rules, the route search and the judge are untouched.
- **It does not decide rules.** Netclass widths, net-to-class assignment, length groups and
  pair skews stay the user's, before the flow.
- **It does not route with Freerouting.** It does not restore a checkpoint itself, and it
  does not retry a write.
- **It does not handle more than one open board** (Row CT's limit and
  `export_specctra_dsn`'s).
- **Known, left as it is:**
  - When the roles change at gate A, the placement was judged with the earlier pours, and
    the result says so.
  - The search and the export check run without the board's locked routing
    (`strip_routing` takes it out), while `delete_routing: delete` keeps it on the live
    board; the result says so, and gate A recommends `delete_including_locked` when it
    touches a moved part.
  - A placement run gives `no_candidate` on a board too dense for family C's gap rule (Row
    CL limit 1). The flow then offers `keep_current` and goes on.
  - Checkpoints are not pruned.
- **Not designed here:**
  - Windows and Linux (macOS first, AGENTS.md).
  - Adding `delete_all_routing` and `add_zone` to B's `BULK_LIVE_EDITS`. F takes its own
    checkpoint before them; adding them is a small follow-up once B and C have merged.

## The build (2026-11-27)

### What changed from the design

- **The interfaces as merged.** C's `kept_locked.export_will_refuse` is a list, not a
  flag. F takes the import, apply and stitching arguments verbatim from the route's own
  `completion.next_steps`, with a new `candidate_output_path` for every F1 attempt (the
  apply never replaces a candidate). `new_call_id` and `unix_ms` were public already.
  #131 added `delete_all_routing` to B's checkpoint list, and its noop to B's outcomes:
  F's checkpoint before L2 and the guard's are one checkpoint (`ensure_checkpoint` reuses
  identical files; a test pins it), and a noop is a success.
- **`FlowIo`.** The live reads, the checkpoint and the registry calls are one trait, so
  the whole order and protocol from gate A on is tested with a fake board. Production is
  `LiveIo` (`registry_call`, what `dispatch_tool` runs).
- **FF4, owner decision 4: measured, and shipped without.** On the public placement
  fixture at effort m, the search was run both ways, each winner given its own P4 pours,
  and the two final routes compared by the pour key. Both routed completely, every gate
  passed. "With pours" lost by one step of the ground-track entry (30 against 29), with a
  shorter signal length step (50 against 51) and higher copper quality (93.9 against
  88.9). "With" was not at least as good, so P2 is skipped and the candidates are routed
  as they are (`SEARCH_WITH_POURS = false`); `WithPours` stays, tested, for a re-measure.
  One public fixture only; the record is `target/creation-research/full-flow/ff4-2026-11-27.json`.
- **F2a's stop.** "0 unconnected items" is read from `run_drc`'s `by_rule` tally for the
  whole board, not per plane net; the 4-round cap still bounds it.
- **From the reviews.** A refused combination at L1 reopens gate A with the reason, where
  any key may change (not a stop). A resume that plans again keeps only the start's
  approvals, so gate A asks again. P4 runs again when the roles or the placement to apply
  change at gate A, and the pours are asked again when the zones differ. A refusal after
  an earlier write of the same step is `uncertain`. Every live read requires one board
  open, also before each save. Stitching rounds take a checkpoint each and resume from
  the last round. The start writes `flow.json` before its marker.

### Verified

| Check | Result |
|---|---|
| Baseline gate, merge commit `e52f43ca`, in a clean worktree | green, but one timing test, `a_tune_pass_killed_by_the_deadline_ends_the_run`, failed while another session's tests ran, and passed alone: tests 6143 passed, 1 failed, 188 ignored; doc, `simulation_live` (95), Clippy, licenses, doc counts and the release build green; installed Konnect `3de188bb775ab4ce` |
| Unit and protocol tests | 92 in `full_flow` and `full_flow_tool`, 4 new in `pour_plan`/`pour_plan_tool`, 2 in `route_lengths`/`route_fastroute` |
| Negative controls, one per guard | 28 of 28 caught (three controls were rewritten once: their first break was not a break) |
| Independent review, Sonnet 5.5 high, read-only | round 1: 0 blockers, 6 majors, 6 minors; round 2 on the fixes: 0, 1, 5. All fixed, each with a test |
| FF4 measurement | as above, 32 minutes at effort m |
| Final gate, `0fd73b4b` (gated as `bc73dfc8`, the same tree) | green: tests 6261 passed, 0 failed, 189 ignored; doctests 9; `simulation_live` 95; Clippy, licenses, doc counts, release build; installed Konnect unchanged |
| Mutants, on the Mac | 278 mutants: 227 caught, 43 missed, 8 unviable. After tests for the misses, the 43 again: 32 caught. Three more tests with hand controls; 8 left, equivalent or KiCad-only (the handoff's Row FF lists them) |

### Not verified

- **No live run.** Nothing here opened KiCad. The live smoke on a public fixture (measurement
  2) and a run on one of the owner's boards (4) wait for the owner: KiCad opened by him.
- **No end-to-end run without KiCad.** The live half (`delete_all_routing`,
  `apply_specctra_ses`, the verified save) has no file mode, so measurement 1 is the FF4
  run for the scratch half and the fake-board tests for the live half.
- **The progress of R2.** A report shows the step only, not the route search's run count.

## Review

One round, by an agent that did not write the note: Sonnet 5.5, high, read-only, against
main and the four branches. It found 2 blockers, 12 majors and 7 minors. Each is taken into
the text above:

- **Blocker: the yardstick.** `PlaceRouteCall::unrouted` checks its text against its input
  board on disk. `WithPours` therefore delegates to a zoned copy (FF4).
- **Blocker: the length rules.** The route's length-rules step is prose today. F's PR gives
  it a structured form with `replace_existing: true` (F3, Modules).
- **The winner's text.** It is not in the placement body, so `plan_with_outcome` returns
  the `Outcome`, and `prepare_search` shares the setup.
- **Gate B.** It read `status`, which stays the router's when stitching is needed. It now
  reads `all_gates_pass` (FF16).
- **No proof before deleting.** P3's routed winner, P4's export check, and the
  `export_will_refuse` handling now come before anything is deleted (FF17).
- **Resume.** A hash that may never have been written could not drive it. It now runs from
  the board through `already_done`, with the intent recorded first (FF15).
- **The save.** It is now verified; the chain runs on the saved file's hash; an `unsaved`
  state was added (FF8).
- **`planned_from`.** It is now `canonical_sha256`.
- **`RegistryCaller`.** It now runs what `dispatch_tool` runs. A handler `Err` is
  `MayHaveChanged`. `arc_tool!` moves (FF2, FF13).
- **Stitching.** It is split into plane rounds and one reference pass; a planner refusal is
  left to the user; arguments are taken verbatim; a save comes before each round; batches
  are capped at 200; rounds are recorded (FF10).
- **Paths.** Numbered attempt folders, and the fastroute path kept (FF14).
- **The leg handoff race.** An answer now waits for the leg before it.
- **The structured plan.** `plan_text` returns it beside the result, and `rails_of` gives
  the rails.
- **The approval logic.** `pour_roles` may be answered again at gate A; the refusals are
  keyed on `routing_runs`; `steps` takes strings.
- **The minors.** The locked-routing note. `liveness` and `Outcome::of` made `pub(crate)`.
  `live_board::observe` placed on main. The marker made with `create_new`. `abandon`
  refused while a leg runs. `ensure_checkpoint` on the blocking pool, and the checkpoint
  count. A live smoke. Restore as preview then apply. Decision 1's full list of the rules
  it amends. A board's run time given as "over 30 minutes", not its exact figure.
