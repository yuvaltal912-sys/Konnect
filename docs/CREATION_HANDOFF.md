# Creation handoff

The current state of the creation-side work: schematic, PCB and library tools. Anyone picking this up, human or agent, starts here. Update this file in the same commit that changes the state. The simulation track has its own file, [`SIMULATION_HANDOFF.md`](SIMULATION_HANDOFF.md).

**Last updated:** 2026-11-06. The research is done. PR 1 (#19, 243d9c7) and PR 2 (#20, 65b6fc2) are merged. A review of PR 2 found ten defects; the fix PR is open and not merged. PR 3 (#21) is open and waits behind it and PR 2b. Ask the owner before starting anything else.

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

One PR at a time, each on its own branch from `main`. Ask the owner before starting the next.

| PR | Delivers | Depends on | State |
|---|---|---|---|
| 1 | Netlist read and intent compare: `get_schematic_netlist`, `compare_netlist` | none | Merged: [#19](https://github.com/yuvaltal912-sys/Konnect/pull/19) |
| 2 | Route safety: port upstream #901, stubs (#899) and `add_wire` T-dots (#900), report `nets_joined`; PR 1 proves no net changed | 1 | Merged: [#20](https://github.com/yuvaltal912-sys/Konnect/pull/20). Review fixes: **open**, [#22](https://github.com/yuvaltal912-sys/Konnect/pull/22) |
| 2b | Freerouting on real boards: DSN export accepts rounded and arc outlines and roundrect pads and keeps zones and pours as fixed copper; install path or setup guidance | 2 | Not started (decided 2026-11-06, goes before PR 3) |
| 3 | Response budget for ERC, DRC and list tools | none | **Open**: [#21](https://github.com/yuvaltal912-sys/Konnect/pull/21), waits behind the PR 2 fixes and PR 2b |
| 4 | Inline renders as image blocks; region and side options | none | Not started |
| 5 | Guidance fixes in the shipped skills and agents; Freerouting path documented. Also (2026-11-06): if the DSN export or Freerouting fails, stop and report the blocker instead of hand-rolling a router, and never finish with unrouted nets without saying so | 1, 2, 2b | Not started |
| 6 | Small schematic edits: mirror after placement, DNP and BOM flags, title block | none | Not started |
| 7 | Board geometry layer: `query_board_region`, true outline containment | none | Not started |
| 8 | Placement objective: wirelength, crossings, pin-level decoupling | 7 | Not started |
| 9 | Sync completion: field copy and orphan delete in `update_pcb_from_schematic` | none | **Open**, first half: field copy and orphan delete (PR link below). Footprint replace and routed-pad reassignment are the follow-up, PR 9b, not started |
| 10 | Library correctness: pin-pad check, symbol fields, ranked search | none | Not started |
| 11 | Honest review and current audit | 7, Q3 | Not started |
| 12 | Routing batch and readback | none | Not started |
| 13 | DRC rule writer | none | Not started |
| 14 | Checkpoints and readiness report | none | Not started |
| 15 | ERC/DRC baselines | 3 | Not started |

## Now: fixes from the review of PR 2

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

## PR 9, sync completion, first half (open)

Branch `feat/creation-sync-completion`; all in `tools/pcb_sync.rs`, plus the two new arguments in the schema in `sch_export.rs`.

- **Field copy.** The netlist's `(fields …)` is read into `DesignComponent.fields` (everything but Reference, Value, Footprint and Component Class, as KiCad's updater skips them). A matched footprint's Datasheet, Description and user fields are compared by name; each difference is a `field_changes` entry (`name`, `from`, `to`) on the `update` change, and an added footprint carries `fields`. The write goes through `apply_footprint_fields`, the same path as reference and value. A user field the footprint lacks is created hidden on `F.Fab`/`B.Fab` at the footprint origin, as KiCad does; one the symbol leaves empty is not created. A footprint field the symbol lacks is left alone (KiCad removes it only under its "remove extra fields" option; Konnect has no such option yet). A board that was `noop` can now be `ready`.
- **Orphans.** A footprint is an orphan when it has a schematic identity, is not board-only, matches no symbol, and its symbol is not merely excluded from the BOM (`ExportedDesign.bom_excluded` is new, because the flag reader used to drop those symbols without a trace, which would have orphaned their footprints). Always listed under `orphan_footprints`. `delete_orphan_footprints` plans a `delete` change for each and the apply deletes exactly those KIIDs, first in the commit. A locked orphan is kept (`kept_locked`). A conflicted plan deletes nothing (`kept_plan_conflict`).
- **Attached copper.** Tracks, arcs and vias are now read with their geometry (`BoardState.copper`). Copper is attached to an orphan when it is on one of its pads' nets and an end lies inside the circle that holds that pad (half the diagonal of the largest layer size, plus half the track width; a via counts as 1 mm wide; any layer). It is deliberately wide: it can name copper that merely passes close and cannot miss connected copper. With `delete_orphan_footprints` alone, such an orphan makes the plan a conflict (`orphan_footprint_has_tracks`) that lists the copper. `delete_orphans_with_tracks` allows it; the copper stays. That flag without the first is `invalid_argument`.
- **Readback.** `orphans_deleted.applied`, `deleted_footprints` and `fields_updated.applied` come from reading the footprints back after the commit. A footprint KiCad still lists is `orphan_footprint_still_present`; a field that did not land is `field_readback_differs`. The plan revision now covers footprint fields, locked and board-only flags and the copper KIIDs, and the deletions are in `changes`, so a plan reviewed without the flag cannot be applied with it.
- **Proof.** Unit tests on the planner and on the served `tools/call` path against the protobuf mock board (changed field, idempotent second run, field already right, added footprint, orphan kept by default, unrouted orphan deleted, orphan with copper refused and then allowed, copper that does not touch, locked/unlinked/board-only/BOM-excluded never deleted, stale revision, a KiCad that ignores deletes or updates, flag misuse, conflicted plan). Sixteen guards were broken one at a time and each failed a test: copper never found, copper net or distance not compared, locked, board-only, unlinked and BOM-excluded footprints deleted, deletion on by default, a conflict that still deletes, deletion counted from the request, fields never compared, field readback skipped, the second flag accepted alone, new field visible, deletion never sent, Footprint field leaking into the field list.
- **Not verified.** Nothing ran against a live KiCad: the field create/update and `DeleteItems` of a footprint are exercised only against the protobuf mock, shaped from KiCad 10.0.1's `footprint.cpp`/`pcb_field.cpp`. The live test is `crates/konnect/tests/live_kicad_tools.rs` (needs a disposable board). Windows and Linux were not run.
- **Known limit.** Replacing a symbol under the same reference (delete R5, add a new R5) is still a `reference_identity_conflict`, and a conflict deletes nothing, so the old R5 has to go first (`delete_component`, or KiCad). Resolving that inside one plan belongs with PR 9b.
- **PR 9b, next.** In-place footprint replacement (`footprint_id_changed`, now a conflict) and routed-pad net reassignment (`routed_pad_net_change`, now a conflict) need to keep position, rotation, side, locked state, pad nets and the tracks attached to the old pads. Ask the owner before starting.

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
