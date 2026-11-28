# Track widths in the full flow (Row TW): design

**State, 2026-11-28.** Designed, built, reviewed twice and measured on branch
`claude/trace-widths-e218f8` from `fork/main` `4b73f974`. The record of what was verified
is in `docs/CREATION_HANDOFF.md`, "Row TW". Real-board results stay
out of this note, apart from counts.

## Why

On 2026-11-28 `run_full_flow` routed the owner's board at effort `xh`, and every track came
out 0.2 mm, its supplies included. The board has only KiCad's Default
netclass, and its design intent has no `rails` and no `currents`. Konnect never invents a
rule, so every net was routed at Default. His words: "they can't all be the same width".

## The short answer

A width plan, made before routing and shown at gate A beside the pour plan:

1. **Roles.** Supplies are the pour plan's supply nets (`pour_roles`, confirmed at gate R).
   Differential pairs come from the design intent's interfaces. Every other net is a
   signal, at its class width.
2. **Currents.** A supply's current comes from the design intent (`currents`, or its rail's
   `max_current_a`). A supply with none, and no pour, gets a stated default of **1 A**,
   shown at gate A and asked, never written silently.
3. **Widths.** IPC-2221 at a 10 °C rise (the net's own rise when recorded), copper from the
   design intent's fab target, else the stackup, else 1 oz outer and 0.5 oz inner, on the
   worst layer the router may use. This is exactly what the export's existing intent
   widening (Row S9) and `audit_trace_current` compute: one function serves all three.
4. **Vias.** When one via of the net's class cannot carry its current, the net gets the
   power via, 0.8/0.4 mm, and the plan says how many a layer change needs in parallel.
5. **Pads.** Pads narrower than a planned width are listed: see "Neck-down".
6. **The write.** On the yes, one new live step, L5 "Track widths recorded", writes the
   confirmed currents to the design intent through `set_design_intent`, with the flow's
   checkpoint and verified save. The export (R1) then gives those nets their width and via,
   and F4 runs `audit_trace_current` and reports it.

## Decision W1: where the widths are written

The request named `create_netclass` and `assign_net_to_class`. The second cannot work in
this flow:

- The flow writes the board while KiCad holds it open (every live step runs over IPC).
- `assign_net_to_class` refuses an open board. KiCad keeps netclass patterns in memory and
  writes them to `.kicad_pro` on every board save (`pcbnew/files.cpp:1049`), and KiCad 10's
  IPC API has no command for patterns: `project_commands.proto` has `GetNetClasses` and
  `SetNetClasses` (class definitions only), `board_commands.proto` only reads a net's class
  (`GetNetClassForNets`). Row RW recorded the same limit.
- A Power class with no nets assigned changes nothing.

| Option | For | Against |
|---|---|---|
| **A. The currents in the design intent; the export widens (recommended)** | Works with the board open. The export already turns a recorded current into a per-net DSN class with its own width and via (Row S9), and `audit_trace_current` reads the same record, so the route and the check cannot disagree. The current is the design fact; the width follows from it | KiCad's own netclasses stay at Default: KiCad's interactive router uses 0.2 mm for later hand edits, and Board Setup shows no Power class. Every Konnect export keeps the widths, since the record stays |
| B. A custom DRC rule per supply (`A.NetName == '12V'`, `track_width`) | KiCad's DRC and interactive router see it | The DSN export refuses conditional custom rules today; a new translation path; DRC would flag every intentional narrow stretch |
| C. Netclasses and patterns | What KiCad users expect | Impossible with the board open (above); the flow would need the user to assign the nets in Board Setup by hand, the manual step Row FF removed |

**Recommended: A.** The PR says plainly that KiCad's netclasses are unchanged and why.

## The plan, in full

`width_plan::plan` is pure: board text, the live effective rules, the design intent, the
resolved supply roles, the poured nets.

- **Poured nets.** A net with a pour on the board, or one that P4 proposes, is "poured": it
  gets no default current, and a recorded one does not widen its tracks (Row S9's rule: the
  pour carries the current; its capacity is not verified). Ground is poured on every board
  the pour plan covers. P4 also plans the board without its proposed zones
  (`if_pours_skipped` in the gate body): when the user answers `pours: skip`, L5 writes
  that plan's currents, since those nets then carry their current on tracks. Gate A's
  `track_widths` question names both.
- **Recorded currents win.** A net whose current the design intent records keeps it; an
  answer that names it is refused with "edit it with set_design_intent", as `pour_roles`
  refuses a role that contradicts a rail. L5 reads the record again before it writes: a
  proposal the record now covers is dropped, and a given current it now holds stops L5.
- **Rises.** A new current for a rail net is sized at its rail's own `max_temp_rise_c`
  when it has one, as the export and the audit will read it; else at 10 °C.
- **Rail siblings.** A rail's current reaches every net of the rail in the export, so every
  one of them with a pad is sized and shown, whatever its role.
- **Locked vias.** A net with locked vias keeps its class via in the plan, as in the export.
- **Rail nets.** A rail net with no current gets the default on its rail's `max_current_a`
  (one value for all the rail's nets). Two nets of one rail given two values is refused.
- **Classes shown.** Nets are grouped by planned width and via: `Power` (or `Power 1`,
  `Power 2` … when several), `Signal` for the rest at their class width, and the
  differential pairs at their class's pair width and gap (Row DP routes them).

### Neck-down

fastroute has no neck-down at a pad (no such code in `fr-engine`; the flow sends
`automatic_neckdown=false`). What it has is Row S9's fallback: a connection that cannot be
made at the widened width is routed again at the net's floor, its class width, and the width
report (`intent_width_report`) lists each stretch below the intent width. So:

- The plan lists, per sized net, the pads narrower than its width (`neck_down_pads`).
- A track wider than a pad still ends on it; the risk is clearance to the next pad.
- A stretch the fallback narrowed is an `audit_trace_current` error, reported, never hidden.
- A real neck-down (only the last part of the track narrowed at the pad) is a fastroute
  change, outside this row (another session holds fastroute's engine).

## Changes

| File | Change |
|---|---|
| `width_plan.rs` (new, pure) | `plan`, `needed_width_mm`, `via_for_current`, `records` (the design-intent sections to write), `answer_refusal`; tests in `width_plan_tests.rs` |
| `specctra.rs` | `widen_for_intent` uses `width_plan::needed_width_mm` and `via_for_current`: a net with a recorded current also gets the power via when its class via cannot carry it (not on a net with locked vias: the export requires a locked via to equal its rule). `pour_nets` factored out. `PadPin` gains `min_size_um` |
| `tools/trace_current.rs` | `via_capacity_a` factored out (the audit's own sum, unchanged) |
| `full_flow.rs` | `Step::Widths` ("L5"); `Approvals.track_widths` (`"as_proposed"`, `"skip"`, or `{net: amps}`); `PlanFacts.widths_to_record`; its need, skip reason, refusals, counts and summary line |
| `tools/full_flow_tool.rs` | P4 also writes `p4-widths.json`; gate A shows it; L5's calls (`get_design_intent`, then `set_design_intent` for `rails` and/or `currents` with the revision); `already_done` for L5; F4 also runs `audit_trace_current` |
| `tools/pcb_export.rs` | the `widths_from_intent` description names the via |
| docs | this note, `FULL_FLOW_DESIGN.md` (L5, the key), `API_MIGRATIONS.md`, `CREATION_HANDOFF.md` |

## Answers at gate A

`approve.track_widths`:

- `"as_proposed"`: record every proposed default (recommended).
- `"skip"`: record nothing; nets with a recorded current are still widened by the export.
- `{ "<net>": <amps> }`: those nets get that current (0 < amps ≤ 100); every other proposed
  default is recorded as proposed. A net may be any board net with a pad that has no
  recorded current, so a signal that carries current (a motor phase, a heater) can be named.

It is asked only when routing runs and the plan has a current to record. It may be given at
the start. The way back for L5 is the call file: `get_design_intent`'s result before the
write holds the sections as they were (checkpoints do not hold `.konnect/`).

## What is checked, and how

- Unit tests of the plan (roles, defaults, recorded currents, rails, pours, vias, pads,
  refusals) with a negative control per guard.
- The export: a widened net's class has its width and the power via in the DSN; a net with
  locked vias keeps its via; the SES import reads both vias.
- The flow: L5's order, need, skip reasons, refusals, `already_done`, the gate body, F4's
  audit, with the fake board.
- Measurement at effort M: the public boards and a scratch copy of the owner's board, routed
  with and without the plan; per net width, vias, and `audit_trace_current` (counts only).
