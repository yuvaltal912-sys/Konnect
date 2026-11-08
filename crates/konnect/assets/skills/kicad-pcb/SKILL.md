---
name: kicad-pcb
description: |
  Workflow skill for KiCAD PCB layout and routing via MCP tools. Triggers on: "layout the board",
  "route traces", "PCB", "place footprints", "copper pour", "board outline", "differential pair",
  "board setup", "track width", "via", "zone", "design rules", "stackup", "silkscreen".
argument-hint: "[layout task]"
---

# KiCAD PCB Layout Workflow

This skill guides Claude to perform PCB layout using Konnect MCP tools.
ALL modifications go through MCP tools — never edit .kicad_pcb files directly.

---

## Prerequisites

Most PCB layout operations require KiCAD to be running with the board file open. The IPC
connection communicates with the running KiCAD instance in real-time.

Some board-construction and component tools have guarded closed-board paths. IPC-first
tools fall back to the file only when the transport is unreachable and the target board
has not been observed live during this server session. These paths use revision-aware
atomic writes: placement preserves pads, graphics, attributes, and models; moves
preserve the existing angle; rotations update the footprint and its child angles; the
closed-board flip fallback mirrors supported geometry and swaps front/back layers,
refusing any 3D model whose offset/rotation it cannot transform. On KiCad 10.0.6+,
`flip_component` prefers KiCad's own native FlipItems IPC command instead, which
handles that 3D-model transform correctly — the file fallback only applies when no
live KiCad holds the board. A reachable KiCad that predates 10.0.6 returns
the structured error **unsupported_capability**; every reachable rejection stays closed instead of racing
the editor with a file edit.

`unsafe_file_fallback` is a stop condition. It means Konnect reached this board live
earlier in the current server session but IPC is now unreachable, so the saved file may
be older than lost editor state. Pause mutation work, tell the user that Konnect left
the file unchanged, and ask them to reopen/recover, reconcile, and save the board in
KiCad. A read-only tool taking `board_source` can return the same kind, where nothing
was going to be written: reporting the saved file as current would be the unsafe act.
There, `board_source: "saved"` inspects that snapshot deliberately and says what it
excludes — offer it instead of retrying the default. Continue through live IPC afterward. Preserve the guard: do not retry-loop,
restart Konnect automatically, or edit `.kicad_pcb` directly. If the user confirms a
clean close and an authoritative saved file, they may restart Konnect to deliberately
begin a new closed-board session.

If connection fails:
- Tell the user to open KiCAD and load the project
- The board (.kicad_pcb) must be open in the PCB editor
- KiCAD's IPC API must be enabled (default in KiCAD 8+)

---

## Toolset Loading

Before any PCB work, load the required toolsets:

```
load_toolset('pcb_board')        # board outline, layers, setup, stackup
load_toolset('pcb_components')   # place, refresh, move, rotate, align footprints
load_toolset('pcb_routing')      # traces, vias, differential pairs
load_toolset('sch_export')       # update PCB from the saved schematic hierarchy
load_toolset('project')          # save the open PCB with save_project
```

Zones (`pcb_board`: add_zone; `pcb_routing`: add_copper_pour), component/net queries (`pcb_components`: find_component, get_component_list; `pcb_board`: get_board_info), and bulk placement (`pcb_components`: place_component_array, align_components, duplicate_component) are already covered by the toolsets loaded above.

Load additional toolsets as needed:

```
load_toolset('config')           # design rule storage: add_design_rule, list_design_rules
load_toolset('verification')     # run_drc, rules/sizes, check_clearance (explicit anchor/courtyard mode)
load_toolset('placement')        # score_placement and read-only query_board_region
load_toolset('pcb_export')       # export_specctra_dsn and refill_zones
load_toolset('integration')      # check_freerouting and local route_specctra_dsn
```

Always call `get_active_toolsets()` first to see what is already loaded.

### References by decision

- Read [`references/layer-reference.md`](references/layer-reference.md) when
  selecting a copper, fabrication, user, or mechanical layer or deciding which
  side owns an item.
- Read [`references/trace-width-table.md`](references/trace-width-table.md) when
  sizing a current-carrying trace, via, or controlled-impedance route. It defines
  the required calculation inputs and acceptance record; it is not a lookup table.
  Take the net's current, temperature rise and copper weight from
  `get_design_intent` when they are recorded, and say which inputs were not.
  After routing, `audit_trace_current(board)` checks every segment and via of
  those nets against IPC-2221; it covers the thermal limit only, not voltage
  drop.
- Read [`references/design-rules.md`](references/design-rules.md) when creating
  netclasses, configuring project constraints, or adjudicating DRC results.

---

## Layout Order

Follow this sequence for a clean PCB workflow:

1. **Board outline** — `set_board_size` or draw Edge.Cuts geometry. Both outline tools
   append, so resize with `delete_graphics(layer='Edge.Cuts')` first — a second call
   without it leaves two overlapping outlines and a DRC failure.
2. **Update from schematic** — call `update_pcb_from_schematic` first with
   `dry_run: true`. Review `status`, `coverage`, `diagnostics`, and staged positions.
   Apply only with `dry_run: false` and the exact returned
   `expected_plan_revision` value. The saved schematic hierarchy must be closed in the
   schematic editor, and the target board must be open in KiCad. A conflict is
   non-mutating; resolve it and rerun the dry run. A diagnostic about a library
   footprint names that footprint and every part that needs it: when the footprint
   cannot be placed (custom-shape pads are not supported), assign those parts a
   footprint without them; when the schematic connects a pad the footprint does not
   have, fix the symbol or the footprint choice. A successful apply is one KiCad
   undo entry, so Ctrl-Z reverses the whole update. The sync also copies each
   symbol's Datasheet, Description and user fields to its footprint (each change is
   listed in the dry run). A footprint whose symbol is gone is listed in the dry run
   and left alone; pass `delete_orphan_footprints: true` to the
   dry run and the apply to delete it. One with tracks on its pads is refused unless
   `delete_orphans_with_tracks: true` is also set, and then the tracks stay. A
   changed footprint ID still needs KiCad's Update PCB (Replace footprints).
3. **Refresh changed libraries** — when a linked footprint library changed, use
   `update_footprints_from_library`, the MCP equivalent of KiCad **Tools → Update
   Footprints from Library**. This is distinct from `update_pcb_from_schematic`:
   it refreshes supported library-owned pads, graphics, attributes, metadata, and
   3D models without changing references, placement, side, rotation, KIID, symbol
   metadata, instance overrides, or pad nets. Always call it first with
   `dry_run: true`; apply only with `dry_run: false` and the exact returned
   `expected_plan_revision`. The requested board must be open in live KiCad, one
   apply is one undo entry, and unsupported or stale content returns a non-mutating
   conflict instead of silently dropping it.
4. **Place components** — position all footprints
5. **Route traces** — use the supported Freerouting round trip below; verify all nets
6. **Copper pour** — add ground/power zones last
7. **Refill, save, and DRC** — synchronize the live board once as described below
8. **Report** — state the result, remaining unrouted nets, and blockers

Do NOT add copper pours before routing is complete — they interfere with interactive routing.
Preserve existing zones: the DSN exporter supports their outlines, and removing them
changes the design. Refill after routing and check the resulting copper.

---

## Placement

### Strategy

- Group components by functional block (power, digital, analog, connectors)
- Place ICs first, then their associated passives
- Decoupling caps: within 2mm of their IC power pins, on same layer
- Cable/EMI filter caps: on the connector's own pins, and judged against that
  connector rather than the nearest IC
- Connectors: at board edges, accessible for cables
- High-frequency components: minimize trace lengths between them
- Thermal considerations: power components away from sensitive analog

### Placement Tools

| Tool                      | Use Case                                    |
|---------------------------|---------------------------------------------|
| `place_component`         | Position one footprint via IPC or safe file fallback |
| `update_footprints_from_library` | Refresh placed definitions from linked libraries |
| `move_component`          | Relocate a footprint via IPC or safe file fallback |
| `rotate_component`        | Rotate a footprint via IPC or safe file fallback |
| `set_component_placements` | Set explicit positions and rotations in one atomic batch |
| `flip_component`          | Set F.Cu/B.Cu via native IPC (KiCad 10.0.6+) or safe file fallback |
| `set_placed_footprint_models` | Inspect or edit exact indexed 3D-model entries on a live placed footprint |
| `align_components`        | Align multiple components (top/bottom/left/right/center) |
| `place_component_array`   | Grid placement for repeated elements        |
| `get_board_2d_view`       | Image of the saved board; `side` top/bottom, `zoom` above 1 for a close-up |

### Bounded AI-directed placement loop

Load `load_toolset('sch_analysis')`, `load_toolset('placement')`, and
`load_toolset('verification')`. Complete placement through this ordered loop; a
proposed coordinate or a successful tool call is not evidence that the requested
board now has the intended placement:

1. **Recover intent and name the scope.** Read the schematic, connectivity, and
   current board. Identify the functional reason for each move and list the exact
   footprint references that may move. Do not infer a capacitor-to-IC relationship
   from GND alone; use **score_placement.decoupling_associations** as supporting
   evidence and treat **unproven_decoupling_caps** as unresolved intent.
2. **Build the held set.** Include every caller-identified intentional
   placement. A diagnostic `auto_place_from_schematic` dry-run reports saved-board
   lock records in its **held** list; accept those only when that saved file is
   known current. `get_component_list` does not expose lock state, so if neither
   current saved evidence nor the caller establishes the KiCad-locked references,
   report `BLOCKED` and ask the caller to confirm them. Never send a held
   reference to any placement, rotation, or flip call, including a batch.
3. **Plan a small explicit batch.** Prefer one functional group and the fewest
   references needed to test the improvement. `auto_place_from_schematic` is a
   deprecated diagnostic planner only: its plan is always blocked from apply.
   `refine_placement_force_directed` is also deprecated; do not weaken its gates
   or apply a score tie. Make a justified score-neutral change with explicit
   moves instead.
4. **Apply only the named moves.** Prefer
   `set_component_placements(board, placements=[{reference, x, y, rotation}, ...])`
   for a small explicit batch. Every entry requires absolute X/Y in mm and an
   absolute rotation in degrees; it is one live IPC update and one undo step, or
   one guarded closed-board write. Single moves may use `move_component` and
   `rotate_component`. Use `flip_component(board, reference, layer='B.Cu')` to
   change side; a layer rename does not mirror footprint geometry. Do not turn
   a diagnostic whole-board plan into an autonomous bulk mutation.
5. **Read back the exact requested live board.** After every batch, call
   `get_component_list` for that board and verify each requested reference's
   observed position, rotation, and layer. Call `get_board_info` and require
   `source: "ipc"` before treating this as live-board proof (`score_placement`
   names the same live authority **source: "live_ipc"**). A saved-file or CLI
   observation may support a separate check, but report it as saved/CLI evidence
   and do not present it as live IPC readback.
6. **Validate the observed result.** Re-run `score_placement` and KiCad DRC.
   Check the score's hard failures, outline status, deductions, associations,
   and evidence source—not only its numeric score. Continue with another small
   batch only when the observed result justifies it.
7. **Finish with evidence or `BLOCKED`.** Report the exact moved and held
   references, observed live positions, source for each check, score/DRC result,
   and remaining findings. Report `BLOCKED` when live source authority,
   functional intent, containment, or another required fact cannot be proved;
   never fill an evidence gap with a guessed coordinate or a request value.

Completion requires live readback of every applied move plus placement and DRC
results from the resulting board. A diagnostic plan, a saved-file snapshot, or
an unproved outline is not completion evidence.

### Placement diagnostics and planners

- `score_placement` reports a 0-100 score with named deductions. Hard failures
  (courtyard overlaps, parts outside a proven outline) decide the verdict
  regardless of the number, and a board with no outline can never pass.
  Read **score_incomplete**, **routing_metrics** and **decoupling_coverage**
  before comparing numeric scores. Routing metrics estimate fanout-filtered
  straight MST wirelength and projected crossings; they do not inspect routed
  copper, layers, shorts or DRC. `max_net_references` defaults to 8 and accepts
  2–256; retain the same policy when comparing placements. Missing data is
  unavailable evidence, not zero cost. Samples may truncate without truncating
  the aggregate calculation.
  **decoupling_associations** reports matching pad numbers/positions and
  non-ground net evidence. A truly pad-adjacent cap can qualify on a larger
  rail; distant associations still need bounded fanout. These are planar
  proximity heuristics, not datasheet or current-loop proof.
  **unproven_decoupling_caps** identifies caps the scorer did not guess about.
  `interface_filter_caps` requires nearby accessible pad pairs for every named
  capacitor terminal net, using the worst nearest-pair distance. A connector
  courtyard or one close terminal is insufficient; do not drag proven
  cable-filtering parts toward an IC. Mechanical row planners are unchanged
  and may now return a blocked plan under the improved judge.
- Outside-outline and connector-edge evidence is judged against the true
  Edge.Cuts outline, rounded corners and cut-outs included, and every
  `outside_outline` failure carries its `overhang_mm`. An outline that does not
  close, or has an unreadable graphic, is `outline_unproven`: treat it like
  `outline_missing`, not like `pass`, and report `BLOCKED` for the unavailable
  containment proof. Pass `edge_mount_references` for parts that are meant to
  sit on the edge. `run_design_review`, `audit_manufacturing` and
  `validate_for_manufacturing` run the same containment check on courtyards,
  pads and silkscreen text.
- `place_decoupling_caps`, `auto_place_from_schematic` and
  `refine_placement_force_directed` still refuse any outline that is not a
  provable rectangle; on a rounded or notched board place parts with small
  explicit moves and confirm with `score_placement`.
- `query_board_region` (placement toolset) answers "what is near this pad":
  name a `bbox`, a footprint or pad to look `around`, or a `point` and radius.
  Use it, and `query_traces` with `touching_pad`, instead of a script that
  parses the board file.
- `auto_place_from_schematic` returns a deterministic net-clustered starting
  plan for diagnosis. It never writes: `dry_run: false` returns structured
  **plan_blocked**.
- `refine_placement_force_directed` is deprecated. Its global-net spring model
  can pull a part toward every footprint sharing a board-wide rail. Dry-run may
  explain its plan, but blocked, non-improving, and score-tied plans do not
  apply; use bounded explicit moves instead.
- `place_decoupling_caps` plans a row beside an IC from exact caller-given
  `capacitor_references` (never net-inferred). It refuses an out-of-bounds,
  non-improving, or containment-unproven plan.
- `plan_bga_fanout` detects pitch from the pad grid; apply executes as one
  KiCad undo commit over live IPC.

### Placement Tips

- Use mm coordinates (KiCAD default for PCB)
- Standard grid: 0.5mm for placement, 0.25mm for fine adjustment
- Check component courtyard overlaps after placement
- Reference designator text: F.SilkS layer, 1mm height default

---

## Routing

Before choosing trace approach points, call `get_component_pads` for the
participating footprints. Use its returned board-space position, effective
rotation, shape, size, drill, and per-copper-layer geometry; do not estimate
copper extent from package family or a different pad in the footprint. A null
geometry field is unavailable evidence, not a zero-size pad.

For nearby geometry, use the read-only `query_board_region` from `placement`:

```
query_board_region(board, around={reference:'U1', pad:'1', margin_mm:2}, limit=50)
```

Use exactly one of `around`, `bbox`, or `point`. Read `sources`, `source_evidence`, `counts`,
`total_matches`, and `truncated`; narrow the region or filters if it is cut.
Zone results describe authored outlines, not filled copper, and footprint
graphics/text are excluded. Use `query_traces(board, touching_pad={reference, pad}, ...)`
when only segments touching the pad's copper matter. Do not replace these reads with a
script that parses `.kicad_pcb`.

### Freerouting round trip

For a supported board, use the local Freerouting path. Load `pcb_export`,
`integration`, `pcb_routing`, and `verification` before starting:

1. Run `check_freerouting(jar_path?, java_path?)`. Inspect `available`,
   `native_mcp_available`, and `bridge_available`, plus `note` or `setup` when
   unavailable. The tool locates the JAR and a compatible Java; it does not
   install either. If unavailable, stop and report the returned setup steps;
   continue only after the installation or capability blocker is resolved.
2. With the exact named board open over IPC and placement/rules settled, call
   `export_specctra_dsn(board, output='new-job.dsn')`. Keep the returned DSN and
   reverse manifest together. Rust export is the default
   (`native_bridge_mode: 'disable'`); no native plugin is required. Existing
   output files are refused. Do not change the board while routing this revision.
3. Call `route_specctra_dsn(dsn_path, ses_output_path='new-result.ses',
   jar_path?, java_path?)`. It runs the local JAR's native headless MCP server
   and writes a new SES; creating that file does not prove every net was routed.
4. Review `plan_specctra_ses_import(board, ses_path, manifest_path)`. It is
   read-only and checks the SES against the revision-bound manifest and live
   board. A stale revision needs a new export and routing job.
5. Apply with `apply_specctra_ses(board, ses_path, manifest_path,
   candidate_output_path='new-candidate.kicad_pcb')`. It imports tracks/vias in
   one undo transaction, checks IPC readback, and writes/checks a new candidate
   without overwriting the source. Inspect its `drc` result: a successful apply
   can still report violations or unconnected items.
6. Finish with the synchronized refill/DRC procedure below. The import's
   candidate check does not establish that existing zone fills are current.

**Current limits:** two copper layers and front-side SMD or through-hole
footprints; circle, rectangle, rounded-rectangle, and oval pads; one closed
outline made from supported lines/arcs, rectangle, circle, or polygon. Outline
arcs/circles become chords within 5 um, and rounded pads use their bounding
rectangles. Existing pours become fixed planes and track/via-blocking rule areas
become keepouts from zone outlines, not their fills; thermal reliefs are not
modelled. Locked straight tracks/vias are preserved. Unlocked routing, routed
arcs, other pad shapes, curves, custom DRC rules, incomplete routing rules,
back-side footprints, and more copper layers are refused. `flip_component`
supports back-side placement, but that does not widen this DSN profile.

If DSN export, Freerouting, or SES import fails, stop and report the exact
blocker. Do not hand-roll a router or direct-edit the board as a fallback. If
the tool cannot handle the requested board, explain the unsupported feature and
have the user finish that routing in KiCad PCB Editor, refill zones, and run its
Design Rules Checker. State every known unrouted net and unavailable check;
report routing as `INCOMPLETE` while a blocker, unrouted connection, or missing
routing-completion evidence remains. Never describe partial routing as finished.

### Routing Tools

| Tool                      | Use Case                                    |
|---------------------------|---------------------------------------------|
| `route_pad_to_pad`        | Direct connection, auto L-bend routing      |
| `route_trace`             | Manual segment-by-segment routing           |
| `route_differential_pair` | Matched-length USB/LVDS/Ethernet pairs      |
| `add_via`                 | Layer transition                            |
| `create_netclass`         | Define width/clearance rules for net groups |

### route_pad_to_pad

For an explicitly planned individual connection. Looks up both pad positions on
the board and lays an L-shaped trace between them; it is not an obstacle-aware
replacement for Freerouting.

```
route_pad_to_pad(board, net_name, ref1, pad1, ref2, pad2, layer?, width?)
```

- Emits one segment when the pads already share an X or Y, two otherwise
- Specify the width in mm from the accepted project netclass or sizing record.
- Routes entirely on `layer` (default `F.Cu`) — it does not add a via. To
  change layer mid-route, place the via yourself with `add_via` and route each
  side separately

### route_trace

One straight segment between two explicit points, for when you want to control
the path yourself.

```
route_trace(board, net_name, layer, x1, y1, x2, y2, width?)
```

- Use for an explicitly planned segment or a targeted correction, then verify it
- There is no waypoint list: call it once per segment to build a polyline
- Coordinates are board-space mm

### route_differential_pair

For differential signals (USB, HDMI, Ethernet, LVDS).

```
route_differential_pair(board, net_pos, net_neg, x1, y1, x2, y2, gap?, layer?, width?)
```

- Lays two straight traces parallel to the given line, offset `(gap + width)/2`
  either side, so spacing is constant along the segment
- Not a length-matching router: it adds no serpentine tuning, and equal length
  only follows from the two traces being parallel segments. Skew introduced
  before or after this call is yours to correct
- Common pairs: USB_D+/USB_D-, LVDS_P/LVDS_N

### Netclasses

Define routing rules for groups of nets:

```
create_netclass(board, name, trace_width?, clearance?, via_drill?, via_diameter?)
```

The class is written to the project's `.kicad_pro` file, which is where KiCad
has kept netclasses since v7 — the board file is not modified.

Before creating or updating a class, read `get_netclasses` and the applicable
design-rule/trace-sizing references. Derive width, clearance, gap, drill, and
diameter from the selected fabrication contract, stackup, and electrical
calculation. Read the classes back after the write and confirm every special net
resolves through the intended class. Missing inputs make the rule `INCOMPLETE`.

### Pre-defined sizes

Netclass width is the default. The Track/Via dropdowns are a separate palette
in the sibling `.kicad_pro`. Fill them with `set_predefined_sizes` so `W` /
`Shift+W` can step through extra widths without changing netclasses:

The values below show call syntax only; they are not engineering recommendations.
Replace every value with one from the accepted project sizing record, derived
from the current fabrication contract, stackup, and electrical requirements. If
that evidence is unavailable, report the sizing task as `INCOMPLETE` instead of
reusing these illustrative values.

```
set_predefined_sizes(board, track_widths=[0.2, 0.5, 0.8],
    via_dimensions=[{diameter:0.6, drill:0.3}, {diameter:0.8, drill:0.4}])
```

A leading 0 mm / 0,0 via is always kept as “use netclass values”. These sizes
are not DRC limits. KiCad reads the list on next project open.

---

## Copper Pour

Zone tools live in the `pcb_board` toolset.

### add_zone

Creates a copper pour area (polygon fill).

```
add_zone(board, net_name, layer, points, clearance?, min_width?,
         name?, priority?, pad_connection?)
```

- Almost always GND net on both F.Cu and B.Cu
- `points` is the outline polygon; define it slightly inside the board edge
  (0.5mm inset)
- `priority` defaults to 0; the higher priority wins where two pours overlap
- `pad_connection` is `solid` | `thermal` | `none`, defaulting to `thermal`
  as KiCad does
- With KiCad running on this board the zone is created over IPC and refilled
  for you, so it appears at once and is in KiCad's undo stack. Without a live
  KiCad it goes into the file instead, and the result says so (`source: file`)
  and carries a `warning` describing the process-local evidence and cold-start
  limitation. A board observed live earlier in this server session fails with
  `unsafe_file_fallback` instead of writing the file.

### refill_zones

Refill after any change that affects copper pour:
- After adding/moving components
- After routing new traces
- After modifying zone outlines
- After changing design rules

Zones do not auto-update — stale fills cause DRC errors.
Use `refill_zones(board)` for a standalone live refill, or combine refill, save,
and DRC with the procedure below. Do not refill twice for the same unchanged board.

### Zone Tips

- GND pour on both layers is standard practice
- Leave spoke thermal reliefs for through-hole pads (easier soldering)
- Use keepout zones to prevent copper in sensitive areas
- Zone clearance typically 0.3-0.5mm from traces

---

## Layer Reference

| Layer    | Name     | Purpose                              |
|----------|----------|--------------------------------------|
| F.Cu     | Front Copper   | Top copper traces and pads     |
| B.Cu     | Back Copper    | Bottom copper traces and pads  |
| F.SilkS  | Front Silk     | Top silkscreen (text, outlines)|
| B.SilkS  | Back Silk      | Bottom silkscreen              |
| F.Mask   | Front Mask     | Top solder mask openings       |
| B.Mask   | Back Mask      | Bottom solder mask openings    |
| Edge.Cuts| Board Outline  | Physical board boundary        |
| F.Fab    | Front Fab      | Top fabrication drawing        |
| B.Fab    | Back Fab       | Bottom fabrication drawing     |
| F.CrtYd  | Front Courtyard| Top component clearance area   |
| B.CrtYd  | Back Courtyard | Bottom component clearance area|
| In1.Cu   | Inner 1        | Internal copper layer 1        |
| In2.Cu   | Inner 2        | Internal copper layer 2        |

### Layer Usage Guidelines

- Route signals on F.Cu and B.Cu (2-layer) or add inner layers for complex boards
- Board outline MUST be on Edge.Cuts (closed polygon or rectangle)
- Silkscreen for reference designators and polarity marks
- Courtyard defines minimum spacing between components
- Use F.Fab/B.Fab for assembly drawings and component outlines

---

## Design Rule Check

After finishing mutations and when ready to save the live board, refill and
synchronize it once before checking:

```
run_drc(board, sync_live_board=true, refill_zones=true, output='full-drc.json')
```

This refills over IPC, saves the named board, and checks its persisted snapshot.
Verify `live_board_synced`, `zones_refilled`, `zone_refill_source`, and `source`.
For an intentionally saved-only check, omit synchronization and identify it as
saved-file evidence; it excludes unsaved live edits.

`errors`, `warnings`, `by_rule`, and `unconnected_items` cover more than the shown
page. Read every page using `next_offset` while `truncated` is true, or inspect
the full `output` report. For follow-up pages keep the same board and filters,
leave `sync_live_board` and `refill_zones` off, and make no edits. If the board
changes, restart the evidence from the new revision. `categories_not_reported`
and null counts mean unavailable checks, not zero findings. Completion needs
resolved DRC findings and proven routing completion; report every outstanding
unconnected item or the reason it could not be checked.

Common DRC errors and fixes:
- **Clearance violation**: move trace or component further apart
- **Unconnected net**: route missing connection
- **Track too close to edge**: move inward from board outline
- **Courtyard overlap**: increase spacing between components
- **Zone fill error**: run `refill_zones`

### Read `owner` before deciding on a board-edge violation

Every violation item carries `owner` and `ownership_status`. Read them before
choosing a fix — `"Circle of J1 on Edge.Cuts"` reads identically whether that
geometry is the board outline or a cutout the footprint carries itself.

- `owner.kind: "board"` — the item is the board's own geometry. Move the
  offending copper inward, or change the outline.
- `owner.kind: "footprint"` — the geometry belongs to that footprint
  (`owner.reference` names it), typically a connector's locking-peg cutout. It
  is still real fabrication geometry and the violation is still real, but the
  pad and the cutout move together, so **repositioning the component cannot fix
  it**. Review the footprint definition or the rule instead.
- `ownership_status` other than `"resolved"` (`"uuid_missing"`,
  `"not_found"`) — ownership is unknown, and `owner` is `null`. Do not assume
  the board owns it; check with `list_board_footprint_graphics` before advising
  a move.

---

## Rules

1. **Never edit .kicad_pcb directly** — all changes go through MCP tools
2. **Always verify placement after moves** — components may snap to unexpected positions
3. **Board outline first** — define the physical boundary before placing anything
4. **Refill zones after changes** — stale zone fills cause phantom DRC errors
5. **Check DRC before finishing** — use the synchronized procedure above, read
   the full result, and report unresolved findings or unrouted nets
6. **Use netclasses for consistency** — define track widths per net type, not per trace
7. **KiCAD normally must be running** — use guarded closed-board paths only when a
   tool explicitly offers them. Treat `unsafe_file_fallback` as a human recovery
   boundary; other PCB edits still require the live IPC connection.
8. **Save frequently** — call `save_project` after major operations
9. **Load toolsets first** — check `get_active_toolsets()` and load what you need
10. **Copper pour last** — add zones only after routing is substantially complete
11. **Stop on routing failure** — report the DSN/Freerouting/import blocker;
    never substitute a handwritten router or hide remaining unrouted nets


### Explicit routing batches

Use `route_batch` for an explicit, reviewed set of polyline segments and through
vias. It is not an autorouter and does not replace the Freerouting failure rule.
Inspect readback IDs and the outcome before continuing. Live netclasses supply
omitted widths and via dimensions; narrower widths and ambiguous pad routes
refuse. Use `modify_trace` for partial in-place edits and `query_vias` /
`delete_via` for observed via identities. A successful edit still needs the
relevant clearance/connectivity checks; an uncertain result needs live-board
inspection before retrying.

### Saved custom rules and routing

Use `create_custom_drc_rule`, `edit_custom_drc_rule` and `delete_custom_drc_rule`
to preview saved custom-rule changes before applying the returned revision.
Close the board and its root schematic before writing rules. General KiCad rule
statements are compiled on scratch copies; a successful edit does not establish
that the board passes DRC.

Custom-rule DSN export supports unconditional numeric clearance and track-width
minima. Other custom-rule forms return a blocker. Re-export after any project,
rule or live netclass change. Keep exactly one PCB open for export/import.
`plan_specctra_ses_import` does not run DRC. Application verifies the committed
copper against private copies of the captured settings; inspect the result and
its source/coverage. If restoration is uncertain, inspect the live board before
retrying. No save, refill or schematic-parity check is implicit.
