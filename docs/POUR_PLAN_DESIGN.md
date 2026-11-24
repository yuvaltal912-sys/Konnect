# The pours a board should have: design of stage ZR4 (`plan_pours`)

Status: designed down to function skeletons on 2026-11-23, and built on 2026-11-24 on
branch `claude/zr4-pour-plan-build` (see "As built" and "Measured"). Stage ZR4 of Row ZR ([ZONE_ROUTING_DESIGN.md](ZONE_ROUTING_DESIGN.md), "Defining
zones" and the stage table). Designed in a Claude Fable 5.2 session on branch
`claude/zr4-pour-plan-79cc54`, from `fork/main` `c4c34369`. **Reviewed** by an agent that
did not write it (Claude Opus 5.5, high, read only); its two must-fix items and its other
findings are in this text (see "Verification of this document"). An agent on Claude Opus
5.5 (high) builds it from this file and the skeleton. It touches no engine file.

## What the owner decided (2026-11-23)

| # | Question | His answer |
|---|---|---|
| 3 | Where do a board's pours come from? | "אני לא רוצה שהברירת מחדל תהיה אדמה, אני רוצה שהוא יתן את החלוקה האופטימלית ביותר. אלא אם אני מבקש מפורשות משהו אחר". The default is the best division of the copper among the board's nets, not ground only. An explicit request of his wins |
| 4 | How is the best division chosen? | "כללים + מילוי KiCad עכשיו (Recommended)". Stackup rules now, and every plan filled by KiCad on a scratch copy before it is returned. The tool only proposes; zones enter the board through `add_zone` on his OK. Ranking candidate divisions by routing each one comes later: since the owner's decision 3 for ZR5 (2026-11-23) it is its own stage, after the pour key is on |

Decision 4 as the design first asked it ("ground on both sides of a 2-layer board?") is
answered inside this one: yes, by rule. Both of his measured boards already pour ground on
both layers.

**Why routing cannot choose the division yet.** The judge's rank key leaves pour copper out:
`pour_track_mm` is recorded and not ranked, and `vias_off_pour` counts only the other nets
(`route_judge.rs`, `rank_measures`). A division that pours more nets would rank better only
because its copper stops being counted. ZR5 puts pour measures into the key; until then
routing has nothing sound to say about a division.

## The short answer

1. **`plan_pours`** is a read-only tool. It divides the board's copper layers among its
   ground and power nets, has KiCad fill that plan on a scratch copy, and returns each zone
   as the exact arguments of `add_zone`. It writes nothing to the board.
2. **The design intent gains `pours`**: the owner's explicit requests. Each one wins over
   the rules.
3. **`add_zone` gains three settings** (the thermal gap, the thermal bridge width and what
   happens to islands; four arguments, since the island rule can carry an area), so a
   proposal can say everything about its zone, and the zone KiCad checked is the zone that
   is added.

## What the tool does

```text
plan_pours(board, board_source?)
  1. the board's text        live from KiCad when it is open there, else the saved file
  2. the files beside it     <stem>.kicad_pro, .konnect/design_intent.json
  3. the plan (pure)         supply nets, the division, existing zones, settings, outline,
                             the rule areas a pour would fill
  4. KiCad's fill            the board with and without the proposals, refilled and
                             checked by kicad-cli on scratch copies, then compared
  5. the result              proposals as add_zone arguments, what was left out and why,
                             the check
```

Steps 1 and 2 read. Step 3 is a pure function of what they read. Step 4 writes only below
Konnect's state folder (`<state>/pour-plans/<stamp>-<sha8>/`). The board, the project file
and the record are never written.

## Decisions (the code cites them as PP1 to PP15)

| # | Decision | Reason |
|---|---|---|
| PP1 | A read-only planner. A zone enters a board only through `add_zone`, called by the AI on the owner's OK | The owner's answer to decision 4. P5 of the row ("no hidden write"), Q1 ("a second opinion, never the author") and Z8 (zones are defined before the export, by an explicit tool) |
| PP2 | The default is a division of the copper among the ground and power nets. The record's `pours` are explicit requests and win | The owner's answer to decision 3, in his words above |
| PP3 | The division is one, made by rules. Candidate divisions ranked by routing come later, as a stage of their own after the pour key is on (the owner's decision 3 for ZR5, 2026-11-23: "כן, כמו שהמלצת בשלוש ההחלטות") | The owner's answer to decision 4, and "Why routing cannot choose the division yet" |
| PP4 | A net's role is the layout audit's: a design-intent rail (0 V is ground), then a netclass whose name reads as power, then the net's name. The result gives each role's source, and a role that comes from a name is named in `next_steps` for the user to confirm | One definition of ground and power for the audit, the ground-via prune (its P3) and the planner. The owner asked for a default that works without a request. A name is a weaker fact than a rail, so it is shown, never hidden |
| PP5 | Whole-layer pours only, one net a layer | A local pour needs a shape, and no shape can be judged before routing can rank a division (PP3). A second whole-layer pour on a layer gets only what the first leaves |
| PP6 | The rule. 2 layers: the ground net with the most pads on both. 4 layers: that net on `F.Cu`, `In1.Cu` and `B.Cu`, the power net with the most pads on `In2.Cu` (ground there when no power net is known). Any other layer count: nothing by rule, only what the record asks for | The owner's answer to decision 4 names exactly this. "Most pads" is a fact of the board and is what a plane serves. A 6-layer stack has no single standard, and a guess there is worse than a request. The layer table's types are reported and not read: KiCad's default type is `signal`, so it says nothing on most boards |
| PP7 | A request with layers gives its net exactly those layers. The rule never places that net, and never puts another net of its kind in its place: the layers the rule would have given it stay free, each named in `not_proposed`. A request without layers puts its net first among the nets of its kind, so the rule's layer of that kind goes to it. A request that cannot be met (a net with no pad on the board, a layer the stack does not have, a net that gets no layer) is reported, with `asked: true`, and the status says so. A net the board does not have never takes a layer from one it has | "Unless I explicitly ask for something else": a request must be exact where it is exact, and visible when it fails. With `GND` asked on `B.Cu` only and a second ground net `AGND` on the board, a rule that moved on to the next ground net would pour `AGND` over the whole front, which nobody asked for (the review's finding). A net with no role that the record asks for is treated as power |
| PP8 | Existing zones are never changed. A layer that already has a pour of the net is left out. A layer where another net's pour sits at priority 0 is left out too, and the reason says to raise that zone's priority | A whole-layer pour must sit below every other pour of its layer. KiCad breaks a tie of equal priorities by UUID (`ZONE::HigherPriority`, `zone.cpp:436`), which is arbitrary, and has no priority below 0. Two pours of one net at one priority whose outlines meet are a `zones_intersect` error (`drc_test_provider_copper_clearance.cpp:1290`) |
| PP9 | Settings. Clearance: the largest of the net's class clearances (the Default class included), the project's `min_clearance` and the fab target's. Minimum width: the largest of 0.25 mm (KiCad's default), the project's `min_track_width` and the fab target's minimum track. Thermal pads, gap 0.5 mm, bridge the larger of 0.5 mm and the minimum width. Islands always removed. Priority 0 | Clearance: KiCad fills at the larger of the zone's own clearance and the rules' anyway, so a larger number only removes copper, and the probe below shows what that costs: on B5 the fill is clean at the class clearance and has starved thermals above it. Minimum width: copper narrower than the narrowest track the fab makes is not made reliably. The rest are KiCad's defaults and what both of the owner's boards use |
| PP10 | The outline is the board's own, as the Specctra export reads it: one closed loop, arcs as chords. A board that the export's readers of the stack, the outline, the footprints or the zones refuse gets no plan, with the reader's reason | KiCad clips a fill to the board edge with the edge clearance (`zone_filler.cpp:2176`), and keeps it out of every rule area that forbids copper pours (`zone_filler.cpp:2306`); for the other rule areas see PP15. Pours are defined before the export (Z8): a board that cannot be exported cannot be routed, and a second outline reader could disagree with the first. The export's other refusals (locked routing it cannot write, copper graphics) are the export's to report when it runs |
| PP11 | Every plan is filled by KiCad before it is returned: the board with the proposals and the board without them, refilled and checked by kicad-cli on scratch copies, then compared. `clean` means that no DRC rule of severity error is reported more often with the proposals, and that no more thermal pads are starved, whatever severity the project gives that rule. Warnings that rise are listed. A proposal KiCad fills nothing of is withdrawn | The owner's answer to decision 4, and P1 of the row: KiCad's refill is the truth. Errors only, because a pour that touches no pad yet is `isolated_copper`, a warning, until the routing reaches it (the probe: 1 on `pours`). A starved thermal is different: it is a pad of the pour's own net with too few spokes, and a project that reports it as a warning has the same weak joint. In the scratch project isolated copper and starved thermals are at least warnings and unconnected items are errors, so a project that ignores them cannot hide them, and `open` counts real items |
| PP12 | No check, no plan. When kicad-cli cannot fill the plan, the tool returns an error that names the step and proposes nothing | He was told that every proposal is filled by KiCad before it is returned. An unchecked plan that looks like a checked one would break that |
| PP13 | The tool is in the `pcb_board` toolset, beside `add_zone`, `list_zones` and `delete_zone`. It reads the board through the board-source seam (`board_source`), like every other reader | One toolset holds the plan and the tool that applies it. The seam already decides when a saved file may stand in for a live board |
| PP14 | A proposal carries every `add_zone` argument, the defaults included, and the scratch zones are written by `add_zone`'s own file writer | What KiCad checked is what is added. A default that changes later cannot change an old plan's meaning. **One limit, on the live path:** over IPC KiCad makes a zone from the board's default zone settings and the API then sets every value of the proposal; what the API has no field for (corner smoothing, the lock) stays as the last zone dialog of that KiCad session left it (`zone.cpp:89`, `zone_settings.cpp:209`, `edit_zone_helpers.cpp:84`). A zone that comes out with rounded corners fills a little differently, and the engine's model does not cover it; the route's `pours.legacy` names it. The file path has no such inheritance |
| PP15 | A rule area that keeps tracks, vias or pads out and lets copper pours in is a finding when a proposal covers its layer: it is listed in `rule_areas_filled`, `next_steps` says how to keep copper out of it, and the status is `planned_with_findings`. The plan still proposes the whole layer | KiCad keeps a fill out of a rule area only when the area forbids pours (`zone_filler.cpp:2306`). `add_rule_area`'s own default, which is KiCad's, forbids tracks, vias and pads and allows pours, so an antenna keepout made with the defaults gets copper under the antenna, and DRC has no rule against it. The plan cannot know whether the area is an antenna or only a no-routing strip, so it says what will happen and leaves the choice (the checklist: "keep intentional isolation barriers and antenna keepouts when adding copper"). Found by the review; none of the five measured boards has a rule area |

### Thermal relief (Z12, and the owner's rule for ZR3)

- A proposal sets the zone's own thermal values only. A pad's own connection, gap and bridge
  stay the pad's; Konnect resolves them per pad when it writes `zones.txt` (Z12), unchanged.
- `add_zone` refuses a thermal bridge below the minimum width: KiCad makes no spoke narrower
  than the minimum width (`zone_filler.cpp:3181`), so every thermal pad of the net would stay
  unconnected. KiCad's own dialog refuses the same (`panel_zone_properties.cpp:387`).
- The check counts `starved_thermal` without and with the proposals. A plan under which
  the count rises is not `clean`, whatever severity the project gives the rule (it is an
  error in KiCad's defaults and in both of the owner's projects).
- A zone that `add_zone` writes reads back through `zone_fill::fill_settings` with the same
  values, and as a zone the engine's model covers (solid fill, square corners). A test holds
  both.

## The probe behind PP9 and PP11 (2026-11-23)

Before the design was fixed: each of the five boards of "The x19 baseline", unrouted, on
scratch copies, with its pours removed and with its own pours at three clearances. kicad-cli
refill and DRC, 20 runs in 21 s under the lane locks. Counts only. "Open on ground" is
KiCad's unconnected items that name a ground net.

| Board | No pours: open on ground | 0.2 mm | 0.3 mm | 0.5 mm |
|---|---|---|---|---|
| dash | 73 | 4 | 4 | 4 |
| B5 | 40 | 0 | 0, and 1 starved thermal | 4, the front fill in 5 pieces, 2 starved thermals |
| `pic` | 39 | 0 | 0 | 0 |
| `ecc` | 6 | 0 | 0 | 0 |
| `pours` | 11 | 0 | 0 | 0 |

- **The class clearance (0.2 mm on all five) is the only one of the three that is clean on
  every board.** Hence PP9. The owner's own zones are at 0.3 mm on both of his boards; that
  difference goes to him with the PR.
- **`pours` reports 1 `isolated_copper` warning with its pours at every clearance** and none
  without them. Hence "errors only" in PP11.
- **No other rule's count changed on any board.**
- **One refill and DRC takes about a second** on these boards, so the check costs seconds.
- This used each board's own pour outlines, not the planner's proposals. `pic` and `ecc` have
  ground on the back only; what the rule's front pour does there is measured in the build.

Record: `target/creation-research/zone-routing/zr4/design-probe/` in the main checkout
(`probe.py`, `driver.sh`, `probe.log`).

## The result

```json
{
  "status": "planned",
  "board": "/path/board.kicad_pcb",
  "board_sha256": "…",
  "stack": [{"name": "F.Cu", "kind": "signal"}, {"name": "B.Cu", "kind": "signal"}],
  "nets": [{"net": "GND", "role": "ground", "source": "name", "pads": 61}],
  "proposals": [{
    "net": "GND", "layer": "F.Cu", "asked": false,
    "reason": "…",
    "add_zone": {"board": "…", "net_name": "GND", "layer": "F.Cu", "points": [{"x": 0, "y": 0}],
                 "clearance": 0.2, "min_width": 0.25, "priority": 0, "pad_connection": "thermal",
                 "thermal_gap": 0.5, "thermal_bridge_width": 0.5, "island_removal": "always"},
    "fill": {"pieces": 1, "area_mm2": 4012.5, "largest_piece_share": 1.0}
  }],
  "not_proposed": [{"net": "+5V", "layer": null, "asked": false, "reason": "…"}],
  "rule_areas_filled": [],
  "check": {"clean": true, "open": [{"net": "GND", "before": 40, "after": 0}],
            "risen": [], "starved_thermals_before": 0, "starved_thermals": 0,
            "folder": "…/pour-plans/…"},
  "next_steps": ["…"],
  "sources": {}, "source_evidence": {}
}
```

- **`status`**, decided in this order:
  - `nothing_to_propose` when no zone is proposed: none was made, or KiCad's fill withdrew
    every one. `check` is there only in the second case. A request that could not be met
    is still in `not_proposed` and in `next_steps`.
  - `planned_with_findings` when the check is not clean, a request could not be met, or a
    proposal fills inside a rule area (PP15).
  - `planned` otherwise.
- **`not_proposed`** names every supply net and layer that was considered and left out, with
  the reason. `net` is `null` for a reason about the board (a stack with no rule, no ground
  net known, a layer the rule leaves free).
- **`rule_areas_filled`**: `[{layer, name, uuid, forbids}]`, one row per rule area and
  proposed layer (PP15); `forbids` is what the area keeps out, of `tracks`, `vias`, `pads`.

## `add_zone`: three settings, four arguments

`add_zone` and its alias `add_copper_pour` share one schema and one implementation; both
gain the same arguments. The fourth argument, `island_min_area`, belongs to
`island_removal`.

| Argument | Values | Default |
|---|---|---|
| `thermal_gap` | mm, 0 or more | 0.5 |
| `thermal_bridge_width` | mm, above 0, not below `min_width` | the larger of 0.5 and `min_width` |
| `island_removal` | `always`, `never`, `below_area` | `always` |
| `island_min_area` | mm², above 0, at most 2000; only with `below_area`, and required there | none |

- **Both paths write them.** IPC: `ZoneSpec` gains the three settings and `build_zone` sends
  them (`thermal_spokes.width` and `gap`, `island_mode`, `min_island_area` in nm², as
  `ZONE::Serialize` does). File: `(thermal_gap …)`, `(thermal_bridge_width …)`, and, only for
  a mode other than `always`, `(island_removal_mode N)` with `(island_area_min A)` for
  `below_area`.
- **A default call writes the zone it wrote before**, byte for byte apart from the UUID.
- **What changes for an old call:**
  - a call that only raises `min_width` above 0.5 now gets a bridge of that width, where it
    got 0.5 and so no spoke at all;
  - a clearance or thermal gap below 0, and a minimum width of 0 or less, are refused;
  - a net name or a layer that holds `"` or `\` is written escaped, as the zone's name
    already was (`NetRef::zone_net_nodes` and `zone_layer_node` in `konnect-sexp` wrote it
    raw, which broke the board's text). Every other name is written as before. The planner
    needs this: it writes names it read off the board.
- **A call that is refused has touched nothing:** the settings are read and checked before
  the IPC attempt and before the file is opened.
- **The result** echoes the four values.

## The design intent: `pours`

```json
"pours": [
  {"net": "+5V", "layers": ["F.Cu"], "notes": "the supply side of the board"},
  {"net": "AGND"}
]
```

- A fifth section of the record, set with `set_design_intent(section: "pours", …)`.
- **Validated like the others:** a net is listed once; a layer is a copper layer name
  (`F.Cu`, `B.Cu`, `In1.Cu` to `In30.Cu`), listed once in its entry; no layer is asked for by
  two entries. Whether the net or the layer is on the board is `plan_pours`'s to report (the
  record never reads the board).
- **A record without `pours` is written exactly as before** (the field is left out when
  empty), so no existing revision changes, and `schema_version` stays 1.
- **A Konnect older than this refuses a record that has `pours`,** the whole record and for
  every tool that reads it, not only for pours, because the record is closed
  (`deny_unknown_fields`). So the package with this stage is installed before `pours` is
  written into a project. The PR text and the migration entry say so.
- **`missing_sections` does not list it:** without requests the planner has its rule.
- **Nets and layers only.** A setting is changed in a proposal's arguments before `add_zone`.

## Modules

New, in `crates/konnect-core/src/`:

| File | What |
|---|---|
| `pour_plan.rs` | The plan, pure: the board's facts, the supply nets, the division, existing zones, settings, the `add_zone` arguments, the scratch board's text |
| `pour_check.rs` | KiCad's fill of a plan: the scratch projects, the two refills, the verdict |
| `tools/pour_plan_tool.rs` | `plan_pours`: the board and the sidecars in, the response out |
| `pour_plan_tests.rs`, `pour_check_tests.rs`, `tools/pour_plan_tool_tests.rs` | Their tests |

Changes to existing items:

| Where | Change |
|---|---|
| `tools/pcb_board.rs` | `ZoneSettings` (new: `from_args`, `check`, `to_args`) read once by `add_zone_impl`, before the IPC attempt; `zone_schema` gains the four arguments; `zone_sexp` (new, `pub(crate)`) replaces `format_zone_polygon`; the result echoes the settings; `tools()` adds `pour_plan_tool::tools()` |
| `crates/konnect-ipc/src/builders.rs` | `ZoneIslands` (new); `ZoneSpec` gains `thermal_gap_mm`, `thermal_bridge_mm` and `islands`; `build_zone` sends them |
| `tools/drc.rs:1326`, `crates/konnect-ipc/tests/live_kicad_test.rs:912`, `builders.rs`'s own tests (`:794`, `:867`, `:890`) | The other users of `ZoneSpec` pass the defaults (0.5, 0.5, `Always`) |
| `crates/konnect-sexp/src/net.rs` | `NetRef::zone_net_nodes` and `zone_layer_node` escape `\` and `"` in the name they write. No change for any other name |
| `tools/design_intent.rs` | `PourIntent` (new); `DesignIntent.pours` (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`); `SECTIONS` gains `pours`; `replace_section`; `validate` appends `validate_pours` (new); the two tools' descriptions and the `section` enum |
| `tools/layout_quality.rs` | `SupplyRoles` and `supply_roles` (new, `pub(crate)`): `intent_roles` with given texts and the audit's defaults. No behaviour change |
| `specctra.rs` | `board_copper_stack` and `board_outline_um` (new, `pub(crate)`): `copper_stack` and `simple_closed_outline` as they are. No behaviour change |
| `tools/mod.rs`, `lib.rs` | The new modules; `supply_roles` and `SupplyRoles` re-exported |
| `router/registry.rs` | `pcb_board`: 17 to 18 tools; its description names the planner |

Not touched: `route_judge.rs`, `route_search.rs`, `pour_prune.rs`, `tools/route_fastroute.rs`,
`fastroute.rs`, `zone_fill.rs`, `length_probe.rs`, `scratch_board.rs` (the last three are
read: `pour_fills`, `fill_settings`, `force_prune_severities`, `force_check_severities`,
`copy_project`), `tools/placement_converge.rs`, the fastroute pin, any engine file.

## The skeleton

The skeleton is code on this branch, not text in this file: the three new modules and the
new items in the existing files, each with its doc comment and a `todo!()` body. The doc
comments are the specification; this file gives the reasons.

| File | Skeleton items |
|---|---|
| `pour_plan.rs` | `SupplyRole`, `SupplyNet`, `CopperLayer`, `ExistingPour`, `OpenRuleArea`, `BoardFacts`, `board_facts`, `ProjectRules`, `project_rules`, `supply_nets`, `Slot`, `NotProposed`, `Division`, `division`, `against_existing`, `settings_for`, `ProposedZone`, `PourPlan`, `plan`, `add_zone_arguments`, `with_zones` |
| `pour_check.rs` | `Refill`, `KicadCli`, `ProposalFill`, `NetOpen`, `Risen`, `FillCheck`, `verdict`, `run_folder`, `scratch`, `check` |
| `tools/pour_plan_tool.rs` | `tools` (the tool's name, description and schema are final), `handle_plan_pours`, `BoardText`, `board_text`, `Sidecars`, `sidecars`, `withdraw_unfilled`, `status`, `result_json`, `next_steps` |
| `tools/pcb_board.rs` | `MAX_ISLAND_AREA_MM2`, `ZoneSettings` and its three methods, `zone_sexp` |
| `crates/konnect-ipc/src/builders.rs` | `ZoneIslands` and its three methods |
| `tools/design_intent.rs` | `PourIntent`, `validate_pours` |
| `tools/layout_quality.rs` | `SupplyRoles`, `supply_roles` |
| `specctra.rs` | `board_copper_stack`, `board_outline_um` |

The tool is not registered in the skeleton: registering it changes the registry's counts,
which the builder does together with the documents that quote them.

## Tests, each guard with a negative control

A control breaks the guard in the source, sees its test fail, and restores the file.

### `add_zone`

| Guard | Test | Negative control |
|---|---|---|
| A default call writes the zone it wrote before | the file text of a call with no new argument: gap and bridge 0.5, no island node | a default changed: the text differs |
| The settings reach the file | gap 0.3, bridge 0.4, `never`: the three nodes | a setting dropped in `zone_sexp`: its node is missing |
| The settings reach KiCad | `build_zone`: spoke width and gap, `island_mode`, `min_island_area` in nm² | the old constant left in `build_zone`: the values differ |
| Both paths write one zone | one settings value in which every fill setting differs from KiCad's default and from `add_zone`'s (`fill_settings` reads a missing node as KiCad's default, so a default value would hide a dropped node): the file text read by `fill_settings` equals what `build_zone` sends | a node dropped in `zone_sexp`, or a different value on one path: they differ |
| A bridge below the minimum width is refused before anything is written | `min_width` 0.5 and a bridge of 0.3: an error; the file's bytes are unchanged, and with the tests' mock KiCad no request reached it | the check removed, or moved after the IPC attempt: the zone is written, or a request is recorded |
| Every value `ZoneSettings::check` refuses | a clearance and a gap below 0; a minimum width of 0 and below 0; a bridge of 0; an island area of 0 and above 2000 mm²; an unknown `island_removal` and `pad_connection`; a string where a number is wanted: each an error that names its argument | each check removed in turn: its case is accepted |
| A raised minimum width still gets spokes | `min_width` 0.8 and no bridge: a bridge of 0.8 | the default left at 0.5: the test fails |
| `island_min_area` is read or refused | given with `always`: refused; `below_area` without it: refused | accepted silently: the test fails |
| What `add_zone` writes is what the engine is told | `zone_sexp`, parsed, through `zone_fill::fill_settings`: the same values, and no refusal | a hatch fill written: `fill_settings` refuses it |
| A name with a quote or a backslash comes back out whole | `zone_sexp` for a net named `A"B\C`: the text parses and the parsed name is the input; `GND`, `/Sheet/+3V3` and `Net-(U1-Pad3)` are written byte for byte as before | the escape removed in `zone_net_nodes`: the text does not parse, or the name differs |
| The result says what was written | the four values in `add_zone`'s result, on the file path and on the mock IPC path | a key left out: the test fails |

### The design intent

| Guard | Test | Negative control |
|---|---|---|
| A record without `pours` is written as before | set another section on a record: the text has no `pours` key | `skip_serializing_if` removed: the text gains `"pours": []` |
| A request is exact or refused | a net twice; a layer twice in an entry; a layer in two entries; `F.SilkS`, `In0.Cu`, `In31.Cu`; an empty net | each check removed: the record is accepted |
| `pours` is a section like the others | set, read back, clear with `[]`; an object where a list is wanted is refused | the section left out of `replace_section`: "not one of" |

### The plan (pure)

| Guard | Test | Negative control |
|---|---|---|
| 2 layers: ground on both | a 2-layer board, one ground net: two proposals | the front layer left out of the rule: one |
| 4 layers: ground on three, power on `In2.Cu` | two power nets: the one with more pads on `In2.Cu`; no power net: ground there | the inner layers swapped: the test fails |
| No rule for another stack | 6 layers: no proposal by rule and one `not_proposed` row; a request with a layer is still met | the 4-layer rule applied to 6: proposals appear |
| Most pads first, then the name | two ground nets, 40 and 6 pads; then equal pads | the order reversed: the other net |
| A request with layers is exact | `GND` asked on `B.Cu` only, 2 layers: one proposal, and a row for the free layer | the rule still giving `GND` the front: two |
| The rule never fills a placed net's layers with another net | `GND` (40 pads) asked on `B.Cu` only, `AGND` (6 pads) on the board, 2 layers: one proposal, `F.Cu` free with its row, `AGND` not proposed. 4 layers, `+5V` (the most pads) asked on `In1.Cu`: `In2.Cu` stays free, `+3V3` is not put there | the rule moving on to the next net of the kind: `AGND` on `F.Cu`, `+3V3` on `In2.Cu` |
| A request without layers heads its kind | 4 layers, `+5V` asked, `+3V3` has more pads: `In2.Cu` is `+5V` | the request ignored: `+3V3` |
| A request that cannot be met is reported | a net not on the board; a layer not in the stack; a power net asked on 2 layers with no layer: `asked: true` rows | dropped silently: no row |
| A net the board does not have takes no layer | 4 layers, `+12V` asked without layers and not on the board: `In2.Cu` goes to the real power net with the most pads, and one `asked: true` row names `+12V`. A ground rail of the record with no pad on the board is not in `nets` | the pad condition dropped in `supply_nets`: `In2.Cu` is `+12V`, or the padless rail heads the ground nets |
| Existing zones are never changed | the net already poured on `B.Cu`: only the front is proposed | the check removed: both |
| Only a pour is an existing pour | a rule area and a teardrop zone of the net on the layer, and no pour: the layer is proposed | either counted as a pour: the layer is left out |
| A rule area that lets pours in is a finding | tracks and vias forbidden, pours allowed, on `F.Cu`: a row in `open_rule_areas`, and in the plan's `filled_rule_areas` when `F.Cu` has a proposal. With `copperpour not_allowed`, or with footprints only forbidden: no row. On a layer with no proposal: not in `filled_rule_areas` | the reader dropped: no row; the pour flag not read: the closed area is listed too |
| A whole-layer pour sits below every other | another net's pour at priority 0 on the layer: left out, the reason names it; at priority 1: proposed | the comparison removed: proposed at 0 |
| The clearance is the largest floor | a net in two classes; a Default above them; a project minimum above; a fab minimum above | each term dropped: its case fails |
| The minimum width and the bridge follow the fab | a fab minimum track of 0.4: width 0.4, bridge 0.5; of 0.8: both 0.8 | the bridge left at 0.5: `ZoneSettings::check` refuses it |
| A project file that does not read is refused | `project_rules(Some("{"))` is an error; so is a `min_clearance` of -1, and one that is a string; `None` gives KiCad's values | `unwrap_or_default`: no error |
| The outline is the board's, in KiCad's axes | a rectangle: its 4 corners, Y down, not repeated; two loops: the export's error | the Y sign kept: the corners are mirrored |
| The roles are the audit's | a 0 V rail named `REF` is ground; a net with a name only has the source `name`; equal to the audit's `net_roles` on the same inputs | the name step removed: the net has no role |
| The arguments read back | `ZoneSettings::from_args(add_zone_arguments(…))` equals the proposal's settings | a key left out of `to_args`: they differ |
| The scratch zone is `add_zone`'s | `with_zones` adds exactly `zone_sexp` of each proposal before the root's close, and changes nothing else | another writer: the texts differ |
| `with_zones` refuses what it cannot write | a board of the older format that does not declare the net: an error that names the net; a text whose root is not `kicad_pcb`: an error | the refusal removed: net 0 is written, or the text is taken |

### The check

| Guard | Test | Negative control |
|---|---|---|
| An error that rises is not clean, a warning is | a scripted pair of reports: `clearance` (error) 0 to 1: not clean; `isolated_copper` (warning) 0 to 1: clean, and listed | the severity not read: the warning case is not clean |
| A starved thermal that rises is not clean at any severity | `starved_thermal` reported as a warning, 0 to 1: not clean, and both counts are in the verdict; 1 and 1: clean | only errors read: the first case is clean |
| Only a count that rises is listed | a rule at 3 and 3 is not in `risen`; 3 and 4 is | `>=` for `>`: the first is listed |
| A fill row is its own proposal's | the net has an older zone on another layer with 3 pieces: the proposal's row has its own count | matched by net only: the wrong count |
| A proposal with no fill row is an error | a refilled text that lacks the proposal's zone: an error, no verdict | a missing row read as 0 pieces: a verdict |
| No unconnected category is an error, not zero | a report without it: an error | `unwrap_or(0)`: a verdict |
| The owner's files are never written | the board's, the project's and the record's bytes before and after `check`, with a scripted refill that rewrites its scratch board | the scratch made beside the board: the folder's listing changes |
| The scratch project cannot hide a pour's faults | a project with `starved_thermal: ignore` and `unconnected_items: ignore`: the scratch copy has `warning` and `error` | either forcing left out: its `ignore` stays |
| Two calls never share a folder | `run_folder` twice with one stamp and text: the second is an error | `create_dir_all`: no error |
| kicad-cli runs on private settings | `KicadCli::refill` with a script in kicad-cli's place that records `KICAD_CONFIG_HOME` | `None` passed: the variable is not set |

### The tool

| Guard | Test | Negative control |
|---|---|---|
| The planner writes nothing | the bytes of the board, the project file and the record before and after `plan_pours` | (the check's own guard above; a write would have to be added) |
| No check, no plan | a kicad-cli that fails: an error result with no `proposals` | an unchecked plan returned: the test fails |
| A proposal KiCad fills nothing of is withdrawn | a check row with 0 pieces: the proposal is in `not_proposed`, with the reason, and a rule area of that layer leaves `rule_areas_filled` | left in `proposals`: the test fails |
| The status tells the truth | clean and no unmet request: `planned`; an error that rose, an unmet request, or a filled rule area: `planned_with_findings`; no proposal: `nothing_to_propose` | a condition inverted: the wrong status |
| No proposal comes first | every proposal withdrawn and one unmet request: `nothing_to_propose`, the `check` is in the result, the request is in `not_proposed` and in `next_steps` | the order changed: `planned_with_findings` with no proposal |
| A filled rule area is said out loud | a rule area that lets pours in, on a proposed layer: its row in `rule_areas_filled`, and a `next_steps` line that names `delete_zone` and `add_rule_area` | the row dropped from the result: the test fails |
| The next steps name real tools | every tool and argument in `next_steps` is in the registry | a misspelt tool name: the test fails |
| A record that cannot be used is refused | a broken `design_intent.json`: `invalid_configuration`, no plan | read as absent: a plan is returned |
| A project file that cannot be read is refused | a `.kicad_pro` that exists and is not readable text: an error result, no plan | read as absent: a plan on KiCad's defaults |
| The result is the documented one | the keys of a result are those of "The result", with `sources` and `source_evidence` for the saved read | a key renamed: the test fails |
| The settings copy is not kept | after a call, and after a call whose check failed, `<folder>/kicad-config` is gone | the removal dropped: the folder is there |
| The tool is where the documents say | `plan_pours` is in `pcb_board`; the registry's count is 18; `doc_tool_counts`, `asset_references` and `schema_parameter_usage` pass | the count left at 17: `registry_tool_counts_match_reality` fails |
| The real thing fills clean (ignored, needs kicad-cli) | the public `pours` fixture with its pours removed: `planned`, `clean`, two proposals with fill | the proposals' minimum width and bridge raised above the smallest ground pad's short side, so that KiCad makes no spoke (`zone_filler.cpp:3178`): thermals starve and the test fails |
| KiCad fills inside a rule area that lets pours in (ignored, needs kicad-cli) | the same fixture with a rule area over part of the board: tracks and vias forbidden, the fill's area is the area without it; `copperpour` forbidden too, the fill is smaller | (a fact of KiCad that PP15 rests on; the test is its record) |

Run `asset_references` and `schema_parameter_usage` (crate `konnect`) early: the
`konnect-core` suites do not cover them.

## Measurement: criteria fixed before any run

**Boards:** the five of "The x19 baseline" (dash, B5, `pic`, `ecc`, `pours`), each a scratch
copy with its copper pours removed (rule areas stay). **Build:** the branch's release binary
by pipe, the fastroute that `packaging/1-kopper/fastroute.json` pins on the day, effort M.
One call a board, each in its own routing hold, under the lane locks.

| Step | What | Criterion |
|---|---|---|
| M1 | `plan_pours` on the pour-less board (saved file, no KiCad open) | C1: the status is `planned` and `check.clean` is true. C5: the plan is the rule's: exactly two proposals (all five boards have 2 layers), the ground net with the most pads on `F.Cu` and on `B.Cu`, neither `asked` |
| M2 | On a scratch copy of the pour-less board: `add_zone` once per proposal with the returned arguments (the file path, no KiCad open). Then the refill and DRC the check itself runs: kicad-cli, the scratch project with the same forced severities, private settings | C2: every rule's count equals the count in the check's own report (`<folder>/with/<board>.drc.json`), and every proposal's piece count equals its `fill.pieces` |
| M3 | The DSN of the re-poured board, exported from its saved file as ZR0's fixtures were; one `route_specctra_dsn` call | C3: the winner passes G1 |
| M4 | SHA-256 before and after M1 of the board, the project file, the design-intent record when the project has one, and every file of the KiCad settings folder the binary was given (`KICAD_CONFIG_HOME`) | C4: all equal |
| M5 | `plan_pours` on each board as it is, with its own pours | C6: no layer that already has the net's pour is proposed. dash, B5 and `pours` pour both layers: `nothing_to_propose`. `pic` and `ecc` pour the back only: one proposal, on `F.Cu` |

- **Done when C1 to C6 hold on all five boards.** C1 with C3 is the stage table's criterion
  ("the proposals refill clean and the route passes G1"); C2, C4, C5 and C6 are this
  design's own, so that the four cannot hold while the planner divides wrongly, proposes a
  second pour over an existing one, or writes a file of the owner's.
- **Not measured, and said so in the PR:** a request of the design intent (`pours`), a
  board with a rule area, a 4-layer board, and `add_zone`'s live path. Unit tests cover
  the first three; the live path has its ignored KiCad test.
- **Recorded beside them, not criteria:** the winner's gates and rank key, its pour measures
  and ground vias, the tool's time, and the same call on each board with its own pours, in
  the same run, as the reference.
- **A criterion that fails is reported with its cause.** No rule or threshold is tuned to
  make a board pass.
- Results of the owner's boards stay out of tracked files, apart from counts. Records go to
  `target/creation-research/zone-routing/zr4/` in the main checkout.

## Documents

In the build's PR:

- This file gains "As built" and "Measured".
- `docs/API_MIGRATIONS.md`: one entry. The new tool; `add_zone`'s arguments and the
  changes for an old call (the bridge default, the refusals, the escaped names); the
  record's `pours` section, and that an older Konnect refuses the whole record once it has
  `pours`.
- `tool-directory.md`, `README.md`, `DEV.md`: the `plan_pours` row, `add_zone`'s row, the
  counts. `cargo xtask fix-doc-counts` rewrites the counts there and in
  `packaging/metadata.json` and `plugin/plugin.json` (the registered count goes up by one, from 270 to 271).
- The `kicad-pcb` skill:
  - `add_zone`'s new arguments and a short `plan_pours` entry in "Copper Pour".
  - **The order of work does not change here.** "Pours last" stays the skill's order until
    ZR5, on the owner's word (decision 5). The `plan_pours` entry says what the tool itself
    says: its zones go in before the export or after the routed session is imported, never
    between the two. So the skill and the tool give one instruction, not two.
  - The `add_zone` bullet that tells the caller to inset `points` 0.5 mm is corrected: KiCad
    keeps a fill off the board edge by the edge clearance, and the plan uses the board's
    own outline.
- `crates/konnect/tests/asset_references.rs`, `NOT_TOOLS`: every backticked word of the new
  skill text that reads like a tool and is none. At least `planned_with_findings`,
  `nothing_to_propose`, `not_proposed`, `rule_areas_filled` and `below_area`, and whichever
  of `starved_thermal`, `isolated_copper`, `largest_piece_share` and `area_mm2` the text
  quotes.
- `docs/ZONE_ROUTING_DESIGN.md` and `docs/CREATION_HANDOFF.md`: where the stage stands.
- The PR text, under what the owner needs to know before merging: the proposals use the
  netclass clearance where his own zones use 0.3 mm (the probe); the package is installed
  before `pours` is written into a record; and PP14's limit on the live path.

## What this design does not do

- **Local pours and split planes.** Whole layers only (PP5). They are candidates for the candidate-divisions stage (after the pour key is on),
  where routing can rank them.
- **A choice among divisions.** One division by rule (PP3). The candidate-divisions stage, after the pour key is on, adds candidates.
- **Stacks other than 2 and 4 layers** by rule (PP6). Requests work on any stack.
- **Settings in the record.** `pours` names nets and layers. A setting is changed in the
  proposal's arguments.
- **A long outline made short.** A board with arcs gives one corner per chord (about 12 a
  rounded corner of 2 mm). The arguments are exact and long.
- **Which inner layer is nearer the parts.** On 4 layers ground is always `In1.Cu`.
- **The project's own zone defaults** (`defaults.zones` of the `.kicad_pro`). They are the
  last values a zone dialog used, not a rule of the project.
- **A board whose footprint has a rule area or a zone of its own.** The export's footprint
  reader refuses it today, and the plan reads the board through that reader (PP10), so
  such a board gets no plan, with that reason. For the same reason a zone inside a
  footprint is never met as an existing pour.
- **A zone exactly as asked on the live path** (PP14's limit): corner smoothing and the
  lock are KiCad's, from its last zone dialog.
- **Cleaning its folders.** Each call keeps `<state>/pour-plans/<stamp>-<sha8>/` as its
  record: two scratch projects (the board, the project file, the rules file, the library
  tables, the record and the project's own `*.pretty` libraries, each twice) and two
  reports. The copy of KiCad's settings is not kept.
- **Windows and Linux:** macOS first, as the rest of the row.

## For the candidate divisions (after ZR5's pour key)

ZR5 puts pour measures into the rank key. Candidate divisions are their own stage, after
that key is on (the owner's decision 3 for ZR5, 2026-11-23).

- The plan's division is one candidate. That stage adds others (ground on one side only, a power
  net on a layer, local pours) and routes each on a scratch copy, as
  `plan_convergent_placement` routes its three.
- `pour_plan::with_zones`, `ProposedZone` and `pour_check`'s scratch projects are what that
  needs of this stage.
- Clearance is a candidate dimension there too: PP9's floor is what the fill supports before
  routing; routing may say more.

## As built (2026-11-24)

Built by Claude Opus 5.5 (high) from this file and the skeleton, on branch
`claude/zr4-pour-plan-build`. The build was in two phases. Phase A was the bodies, the
tests, the negative controls and a mutants batch. An agent that did not write the diff
then reviewed it. Phase B was the review's items, the merge of `fork/main` `e07ff839`,
the gate, the measurement and these documents. No decision changed. What differs from
the skeleton:

- `ZoneSettings::to_args` takes `self`, not `&self`: clippy refuses `to_*(&self)` on a
  `Copy` type.
- `ZoneSettings::spec` is new. It maps the settings to `ZoneSpec` once for the IPC path,
  so the two paths cannot drift apart.
- The handler's steps 2 to 7 are in a private `plan_pours_in`, which takes the state
  folder. The tests give their own state folder, so no test writes below the user's
  real one.
- `board_facts` refuses a pour whose `(priority …)` is not a whole number. The skeleton
  said only "0 when absent".
- A slot is `asked` when a request put the net there. That covers a request without
  layers whose net took the rule's layer of its kind.
- **Review item 1.** A slot that `against_existing` refuses because its own net already
  pours that layer is not an unmet request (`asked: false`): the board already meets
  it. A request that another net's pour blocks stays unmet.
- **Review item 2.** When a placed net's requested layers are not on the board, the
  reason of each free layer says so.
- **A fact of KiCad found by the control of "The real thing fills clean".** KiCad
  reports a thermal pad that gets no spoke at all as unconnected, not as a starved
  thermal (`drc_test_provider_zone_connections.cpp:176`). Raising the minimum width
  above the pads' short side made a plan that KiCad filled, that connected no pad
  (GND open 11 before, 11 after), and that `check.clean` still called clean. The test
  now also asserts that the pours join every GND pad (`open` after is 0), and the
  control fails it. `clean` itself is unchanged, as PP11 defines it. A plan whose
  pours connect nothing is visible only in `check.open`. The decision is the owner's.
- Tests: one or more for every row of the tables above. Negative controls: 82, every
  one caught (`target/creation-research/zone-routing/zr4/controls/RECORD.md`).
- Mutants: one narrow batch on the new guards, 122 mutants. 108 were caught, 6 were
  unviable and 8 were missed. Three tests were added, and the 8 were run once more: 7
  were caught. One survives, in `supply_nets` (`||` to `&&`). No valid record can
  reach the case it changes (`zr4/mutants/RECORD.md`).

## Measured (2026-11-24)

The design's "Measurement" was run once on the branch's release build (`f497868c`, the
gate's copy, from `6f8d205a`). Calls went by pipe, with no KiCad open, and with fastroute
x21 (`46d79d56`, the pin) at effort M. Each routing call ran in its own routing hold under
the lane locks. The boards are the five of "The x19 baseline", as scratch copies with
their copper pours removed. None of them has a rule area. Counts only. Record:
`target/creation-research/zone-routing/zr4/measure/` (`result.json`,
`plan-record.json`, the runs).

| Board | C1 | C2 | C3 | C4 | C5 | C6 |
|---|---|---|---|---|---|---|
| dash | holds | holds | holds | holds | holds | holds |
| B5 | **fails** | holds | holds | holds | holds | holds |
| `pic` | holds | holds | holds | holds | holds | holds |
| `ecc` | holds | holds | holds | holds | holds | holds |
| `pours` | holds | holds | holds | holds | holds | holds |

- **C1 fails on B5.** The status is `planned_with_findings`: `starved_thermal`
  (error) rose from 0 to 2. Two fine-pitch IC pads got 1 spoke where the zone asks
  for 2.
  - The cause is PP9's thermal values: a gap and a spoke width of 0.5 mm. B5's own
    pours use a gap of 0.3 mm and a spoke width of 0.4 mm.
  - The same scratch board, refilled with those two values and nothing else changed,
    has 0 starved thermals (`measure/b5_probe.sh`).
  - So PP9's reason ("what both of the owner's boards use") holds for dash and not for
    B5. Nothing was tuned; the choice is the owner's.
- **C1 holds on the other four.** KiCad's unconnected items on the ground net go to 0
  on every board: from 69 (dash), 39 (`pic`), 6 (`ecc`) and 11 (`pours`). On B5 too,
  from 40. On `pours`, 1 `isolated_copper` warning rose, as the probe predicted.
- **C2:** on every board, `add_zone` called with the returned arguments gives the same
  count for every DRC rule as the check's own report. It also gives the same piece
  count for every proposal: 1 a layer, and 5 on each layer of `pic`.
- **C3:** every re-poured board routes `complete`, and its winner passes G1. It
  passes all four gates on all five boards.
- **C4:** `plan_pours` changes none of these bytes: the board, the project file, the
  design-intent record (dash has one), and every file of the given KiCad settings
  folder (8 to 10 files).
- **C5:** every board gets two proposals: the ground net with the most pads, on F.Cu
  and B.Cu, neither `asked`.
- **C6:** dash, B5 and `pours` give `nothing_to_propose` as they are. `pic` and `ecc`
  give one proposal, on F.Cu.
- **Recorded, not criteria.** The planner takes 1.5 to 3.0 s a board. The reference is
  the same route call on each board with its own pours. Against it, the winner with the
  plan's pours had:
  - fill pieces: 14 against 34 (dash), 11 against 13 (B5), 14 against 2 (`pic`),
    2 against 1 (`ecc`), 2 against 2 (`pours`);
  - starved thermals: 0 on every board (the reference on B5: 1);
  - `vias_off_pour`: equal on four boards; 0 against 1 on `pic`.
- **Not measured,** as the design said: a request of the design intent, a board with a
  rule area, a 4-layer board, and `add_zone`'s live path. Unit tests cover the first
  three.
- PP15 was checked on a real KiCad fill: the ignored test
  `kicad_fills_inside_a_rule_area_that_lets_pours_in` ran on the public fixture with a
  rule area added, and passed. The ignored test `the_real_thing_fills_clean` passed too.

## Verification of this document

**Read against the source** (`c4c34369`): `tools/pcb_board.rs` (`add_zone_impl`,
`format_zone_polygon`, `zone_schema`), `konnect-ipc/src/builders.rs` (`ZoneSpec`,
`build_zone`), `tools/design_intent.rs`, `tools/layout_quality.rs` (`intent_roles`),
`specctra.rs` (`copper_stack`, `simple_closed_outline`, `pour_fill_rows`, `pad_pins`),
`zone_fill.rs` (`fill_settings`, `pour_fills`), `route_judge.rs` (`rank_measures`,
`private_kicad_config`), `tools/cli.rs` (`refill_drc_on_private_copy_with_config`),
`scratch_board.rs`, `length_probe.rs` (`force_prune_severities`), `tools/board_source.rs`,
`tools/placement_converge.rs`, the registry and the three guard tests of crate `konnect`.
KiCad 10.0.1: `zone.cpp` (`HigherPriority`, `Serialize`), `zone_filler.cpp` (the spoke
width), `drc_test_provider_copper_clearance.cpp` (`testZonesToZones`),
`pcb_io_kicad_sexpr.cpp` and its parser (the fill's nodes, `island_area_min`),
`panel_zone_properties.cpp`.

**Measured:** the probe above. Nothing else: no routing, no build of the tool.

**Skeleton compile** (2026-11-23, in a light lane slot, and again after the review's
changes): `cargo check --locked -p konnect-ipc -p konnect-core` on this branch exits 0 with
0 errors. `konnect-core` gives 127 warnings, every one an unused variable, import or item of
the skeleton: nothing calls it yet. 32 bodies are `todo!()`. `cargo fmt --all -- --check` is
clean. Tests were not compiled or run. Record of the second run:
`target/creation-research/zone-routing/zr4/review-check/` in the main checkout.

**Independent review: done, 2026-11-23.** An agent that did not write this (Claude Opus
5.5, high, read only: no build, no KiCad) read the design, the skeleton, Konnect's source
and KiCad 10.0.1's. Its verdict: ready with the must-fix items. The facts about the
existing code and about KiCad held, with one exception (the first row). Each finding was
checked against the source before it was taken.

| # | Finding | Taken as |
|---|---|---|
| 1, must-fix | KiCad keeps a fill out of a rule area only when the area forbids pours; PP10 said "every rule area" | PP10 corrected, PP15 added, `OpenRuleArea`, `rule_areas_filled`, tests |
| 2, must-fix | `division` moved on to the next ground net when the first was asked with layers, and poured it over a layer the owner left clear | PP7 and `division` rewritten, a two-net test |
| 3 | A net the record asks for and the board lacks could take a layer | `supply_nets` drops every net with no pad; a test |
| 4 | `clean` stayed true when thermals starved on a project that reports them as warnings; unconnected items could be ignored | `clean` reads starved thermals at any severity; the scratch project forces unconnected items too |
| 5 | The zone writer wrote a net name raw | `zone_net_nodes` and `zone_layer_node` escape; a test |
| 6 | Over IPC a new zone keeps what the API cannot set | PP14's limit, "Not verified", the PR text |
| 7 | The tool told the caller to add pours before the export; the skill says pours last | the tool names both orders and the one thing that is refused; the skill's entry says the same |
| 8 | The skill's new words would fail `asset_references`; two more files quote the count | named under "Documents" |
| 9, 10 | Three tests that would pass with their guard gone, and guards with no test | the tests tables: 8 rows changed, 16 added |
| 11 | C2 could not be measured as written, and C1 to C4 could hold on a broken planner | C2 rewritten, C4 widened, C5 and C6 added |
| 12 to 17 | `board_facts` claimed all of the export's refusals; footprint zones; the status when every proposal is withdrawn; the folder's stamp and what it keeps; "three" against four arguments; older builds and `pours` | each in its place in this text and in the doc comments |

**Not verified.**

- The rule's front pour on `pic` and `ecc`, whose own boards have none (the build's M1).
- That KiCad's API applies the thermal values and the island mode of a created zone as
  `ZONE::Deserialize` reads them: the build's live test makes a zone over IPC and compares
  every setting with KiCad's own text of the board.
- What a zone made over IPC inherits from the editor (PP14's limit). It takes a zone dialog
  in a running KiCad to set those defaults, which no test here drives; it is read from
  KiCad's source only.
- The 4-layer rule on a real 4-layer board: none of the five is one. Unit tests only.
- PP15 on a real board: none of the five has a rule area. Unit tests, and one ignored test
  that has KiCad fill a board with a rule area.
- A request of the design intent on a real board: unit tests only.
