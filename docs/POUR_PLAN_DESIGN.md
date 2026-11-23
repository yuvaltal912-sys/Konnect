# The pours a board should have: design of stage ZR4 (`plan_pours`)

Status: a design down to function skeletons, 2026-11-23. Nothing is built: every body is
`todo!()`. Stage ZR4 of Row ZR ([ZONE_ROUTING_DESIGN.md](ZONE_ROUTING_DESIGN.md), "Defining
zones" and the stage table). Designed in a Claude Fable 5.2 session on branch
`claude/zr4-pour-plan-79cc54`, from `fork/main` `c4c34369`. **Not yet reviewed** by an agent
that did not write it: the review runs before the plan is pushed (see "Verification of this
document"). An agent on Claude Opus 5.5 (high) then builds it from this file and the
skeleton. It touches no engine file.

## What the owner decided (2026-11-23)

| # | Question | His answer |
|---|---|---|
| 3 | Where do a board's pours come from? | "אני לא רוצה שהברירת מחדל תהיה אדמה, אני רוצה שהוא יתן את החלוקה האופטימלית ביותר. אלא אם אני מבקש מפורשות משהו אחר". The default is the best division of the copper among the board's nets, not ground only. An explicit request of his wins |
| 4 | How is the best division chosen? | "כללים + מילוי KiCad עכשיו (Recommended)". Stackup rules now, and every plan filled by KiCad on a scratch copy before it is returned. The tool only proposes; zones enter the board through `add_zone` on his OK. Ranking candidate divisions by routing each one comes with ZR5 |

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
3. **`add_zone` gains `thermal_gap`, `thermal_bridge_width` and `island_removal`**, so a
   proposal can say everything about its zone, and the zone KiCad checked is the zone that
   is added.

## What the tool does

```text
plan_pours(board, board_source?)
  1. the board's text        live from KiCad when it is open there, else the saved file
  2. the files beside it     <stem>.kicad_pro, .konnect/design_intent.json
  3. the plan (pure)         supply nets, the division, existing zones, settings, outline
  4. KiCad's fill            the board with and without the proposals, refilled and
                             checked by kicad-cli on scratch copies, then compared
  5. the result              proposals as add_zone arguments, what was left out and why,
                             the check
```

Steps 1 and 2 read. Step 3 is a pure function of what they read. Step 4 writes only below
Konnect's state folder (`<state>/pour-plans/<stamp>-<sha8>/`). The board, the project file
and the record are never written.

## Decisions (the code cites them as PP1 to PP14)

| # | Decision | Reason |
|---|---|---|
| PP1 | A read-only planner. A zone enters a board only through `add_zone`, called by the AI on the owner's OK | The owner's answer to decision 4. P5 of the row ("no hidden write"), Q1 ("a second opinion, never the author") and Z8 (zones are defined before the export, by an explicit tool) |
| PP2 | The default is a division of the copper among the ground and power nets. The record's `pours` are explicit requests and win | The owner's answer to decision 3, in his words above |
| PP3 | The division is one, made by rules. Candidate divisions ranked by routing come with ZR5 | The owner's answer to decision 4, and "Why routing cannot choose the division yet" |
| PP4 | A net's role is the layout audit's: a design-intent rail (0 V is ground), then a netclass whose name reads as power, then the net's name. The result gives each role's source, and a role that comes from a name is named in `next_steps` for the user to confirm | One definition of ground and power for the audit, the ground-via prune (its P3) and the planner. The owner asked for a default that works without a request. A name is a weaker fact than a rail, so it is shown, never hidden |
| PP5 | Whole-layer pours only, one net a layer | A local pour needs a shape, and no shape can be judged before routing can rank a division (PP3). A second whole-layer pour on a layer gets only what the first leaves |
| PP6 | The rule. 2 layers: the ground net with the most pads on both. 4 layers: that net on `F.Cu`, `In1.Cu` and `B.Cu`, the power net with the most pads on `In2.Cu` (ground there when no power net is known). Any other layer count: nothing by rule, only what the record asks for | The owner's answer to decision 4 names exactly this. "Most pads" is a fact of the board and is what a plane serves. A 6-layer stack has no single standard, and a guess there is worse than a request. The layer table's types are reported and not read: KiCad's default type is `signal`, so it says nothing on most boards |
| PP7 | A request with layers gives its net exactly those layers, and the rule gives that net no other layer. A request without layers puts its net first among the nets of its kind, so the rule's layer of that kind goes to it. A request that cannot be met is reported, with `asked: true`, and the status says so | "Unless I explicitly ask for something else": a request must be exact where it is exact, and visible when it fails. A net with no role that the record asks for is treated as power |
| PP8 | Existing zones are never changed. A layer that already has a pour of the net is left out. A layer where another net's pour sits at priority 0 is left out too, and the reason says to raise that zone's priority | A whole-layer pour must sit below every other pour of its layer. KiCad breaks a tie of equal priorities by UUID (`ZONE::HigherPriority`, `zone.cpp:436`), which is arbitrary, and has no priority below 0. Two pours of one net at one priority whose outlines meet are a `zones_intersect` error (`drc_test_provider_copper_clearance.cpp:1290`) |
| PP9 | Settings. Clearance: the largest of the net's class clearances (the Default class included), the project's `min_clearance` and the fab target's. Minimum width: the largest of 0.25 mm (KiCad's default), the project's `min_track_width` and the fab target's minimum track. Thermal pads, gap 0.5 mm, bridge the larger of 0.5 mm and the minimum width. Islands always removed. Priority 0 | Clearance: KiCad fills at the larger of the zone's own clearance and the rules' anyway, so a larger number only removes copper, and the probe below shows what that costs: on B5 the fill is clean at the class clearance and has starved thermals above it. Minimum width: copper narrower than the narrowest track the fab makes is not made reliably. The rest are KiCad's defaults and what both of the owner's boards use |
| PP10 | The outline is the board's own, as the Specctra export reads it: one closed loop, arcs as chords. A board the export refuses gets no plan, with the export's reason | KiCad clips a fill to the board edge and to every rule area, so isolation barriers and antenna keepouts hold. Pours are defined before the export (Z8): a board that cannot be exported cannot be routed, and a second outline reader could disagree with the first |
| PP11 | Every plan is filled by KiCad before it is returned: the board with the proposals and the board without them, refilled and checked by kicad-cli on scratch copies, then compared. `clean` means no DRC rule of severity error is reported more often with the proposals. Warnings that rise are listed. A proposal KiCad fills nothing of is withdrawn | The owner's answer to decision 4, and P1 of the row: KiCad's refill is the truth. Errors only, because a pour that touches no pad yet is `isolated_copper`, a warning, until the routing reaches it (the probe: 1 on `pours`). The scratch project reports isolated copper and starved thermals at least as warnings, so a project that ignores them cannot hide them |
| PP12 | No check, no plan. When kicad-cli cannot fill the plan, the tool returns an error that names the step and proposes nothing | He was told that every proposal is filled by KiCad before it is returned. An unchecked plan that looks like a checked one would break that |
| PP13 | The tool is in the `pcb_board` toolset, beside `add_zone`, `list_zones` and `delete_zone`. It reads the board through the board-source seam (`board_source`), like every other reader | One toolset holds the plan and the tool that applies it. The seam already decides when a saved file may stand in for a live board |
| PP14 | A proposal carries every `add_zone` argument, the defaults included, and the scratch zones are written by `add_zone`'s own file writer | What KiCad checked is what is added. A default that changes later cannot change an old plan's meaning |

### Thermal relief (Z12, and the owner's rule for ZR3)

- A proposal sets the zone's own thermal values only. A pad's own connection, gap and bridge
  stay the pad's; Konnect resolves them per pad when it writes `zones.txt` (Z12), unchanged.
- `add_zone` refuses a thermal bridge below the minimum width: KiCad makes no spoke narrower
  than the minimum width (`zone_filler.cpp:3181`), so every thermal pad of the net would stay
  unconnected. KiCad's own dialog refuses the same (`panel_zone_properties.cpp:387`).
- The check counts `starved_thermal` and lists it when it rises. It is an error in KiCad's
  defaults and in both of the owner's projects, so a plan that starves a pad is not `clean`.
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
  "check": {"clean": true, "open": [{"net": "GND", "before": 40, "after": 0}],
            "risen": [], "starved_thermals": 0, "folder": "…/pour-plans/…"},
  "next_steps": ["…"],
  "sources": {}, "source_evidence": {}
}
```

- **`status`**: `planned`; `planned_with_findings` when the check is not clean or a request
  could not be met; `nothing_to_propose` when no zone is proposed (then there is no `check`).
- **`not_proposed`** names every supply net and layer that was considered and left out, with
  the reason. `net` is `null` for a reason about the board (a stack with no rule, no ground
  net known).

## `add_zone`: three settings

`add_zone` and its alias `add_copper_pour` share one schema and one implementation; both
gain the same arguments.

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
  - a clearance or thermal gap below 0, and a minimum width of 0 or less, are refused.
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
  empty), so no existing revision changes, and `schema_version` stays 1. A Konnect older than
  this refuses a record that has `pours`, because the record is closed (`deny_unknown_fields`).
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
| `tools/pcb_board.rs` | `ZoneSettings` (new: `from_args`, `check`, `to_args`) read once by `add_zone_impl`; `zone_schema` gains the four arguments; `zone_sexp` (new, `pub(crate)`) replaces `format_zone_polygon`; the result echoes the settings; `tools()` adds `pour_plan_tool::tools()` |
| `crates/konnect-ipc/src/builders.rs` | `ZoneIslands` (new); `ZoneSpec` gains `thermal_gap_mm`, `thermal_bridge_mm` and `islands`; `build_zone` sends them |
| `tools/drc.rs:1325`, `crates/konnect-ipc/tests/live_kicad_test.rs:912` | The other two users of `ZoneSpec` pass the defaults (0.5, 0.5, `Always`) |
| `tools/design_intent.rs` | `PourIntent` (new); `DesignIntent.pours` (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`); `SECTIONS` gains `pours`; `replace_section`; `validate` appends `validate_pours` (new); the two tools' descriptions and the `section` enum |
| `tools/layout_quality.rs` | `SupplyRoles` and `supply_roles` (new, `pub(crate)`): `intent_roles` with given texts and the audit's defaults. No behaviour change |
| `specctra.rs` | `board_copper_stack` and `board_outline_um` (new, `pub(crate)`): `copper_stack` and `simple_closed_outline` as they are. No behaviour change |
| `tools/mod.rs`, `lib.rs` | The new modules; `supply_roles` and `SupplyRoles` re-exported |
| `router/registry.rs` | `pcb_board`: 17 to 18 tools; its description names the planner |

Not touched: `route_judge.rs`, `route_search.rs`, `pour_prune.rs`, `tools/route_fastroute.rs`,
`fastroute.rs`, `zone_fill.rs`, `length_probe.rs`, `scratch_board.rs` (the last three are
read: `pour_fills`, `fill_settings`, `force_prune_severities`, `copy_project`), the fastroute
pin, any engine file.

## The skeleton

The skeleton is code on this branch, not text in this file: the three new modules and the
new items in the existing files, each with its doc comment and a `todo!()` body. The doc
comments are the specification; this file gives the reasons.

| File | Skeleton items |
|---|---|
| `pour_plan.rs` | `SupplyRole`, `SupplyNet`, `CopperLayer`, `ExistingPour`, `BoardFacts`, `board_facts`, `ProjectRules`, `project_rules`, `supply_nets`, `Slot`, `NotProposed`, `Division`, `division`, `against_existing`, `settings_for`, `ProposedZone`, `PourPlan`, `plan`, `add_zone_arguments`, `with_zones` |
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
| Both paths write one zone | one settings value: the file text read by `fill_settings` equals what `build_zone` sends | a different value on one path: they differ |
| A bridge below the minimum width is refused before anything is written | `min_width` 0.5 and a bridge of 0.3: an error, the file's bytes unchanged | the check removed: the zone is written |
| A raised minimum width still gets spokes | `min_width` 0.8 and no bridge: a bridge of 0.8 | the default left at 0.5: the test fails |
| `island_min_area` is read or refused | given with `always`: refused; `below_area` without it: refused | accepted silently: the test fails |
| What `add_zone` writes is what the engine is told | `zone_sexp`, parsed, through `zone_fill::fill_settings`: the same values, and no refusal | a hatch fill written: `fill_settings` refuses it |

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
| A request without layers heads its kind | 4 layers, `+5V` asked, `+3V3` has more pads: `In2.Cu` is `+5V` | the request ignored: `+3V3` |
| A request that cannot be met is reported | a net not on the board; a layer not in the stack; a power net asked on 2 layers with no layer: `asked: true` rows | dropped silently: no row |
| Existing zones are never changed | the net already poured on `B.Cu`: only the front is proposed | the check removed: both |
| A whole-layer pour sits below every other | another net's pour at priority 0 on the layer: left out, the reason names it; at priority 1: proposed | the comparison removed: proposed at 0 |
| The clearance is the largest floor | a net in two classes; a Default above them; a project minimum above; a fab minimum above | each term dropped: its case fails |
| The minimum width and the bridge follow the fab | a fab minimum track of 0.4: width 0.4, bridge 0.5; of 0.8: both 0.8 | the bridge left at 0.5: `ZoneSettings::check` refuses it |
| A project file that does not read is refused | `project_rules(Some("{"))` is an error; `None` gives KiCad's values | `unwrap_or_default`: no error |
| The outline is the board's, in KiCad's axes | a rectangle: its 4 corners, Y down, not repeated; two loops: the export's error | the Y sign kept: the corners are mirrored |
| The roles are the audit's | a 0 V rail named `REF` is ground; a net with a name only has the source `name`; equal to the audit's `net_roles` on the same inputs | the name step removed: the net has no role |
| The arguments read back | `ZoneSettings::from_args(add_zone_arguments(…))` equals the proposal's settings | a key left out of `to_args`: they differ |
| The scratch zone is `add_zone`'s | `with_zones` adds exactly `zone_sexp` of each proposal before the root's close, and changes nothing else | another writer: the texts differ |

### The check

| Guard | Test | Negative control |
|---|---|---|
| An error that rises is not clean, a warning is | a scripted pair of reports: `starved_thermal` (error) 0 to 1: not clean; `isolated_copper` (warning) 0 to 1: clean, and listed | the severity not read: the warning case is not clean |
| Only a count that rises is listed | a rule at 3 and 3 is not in `risen`; 3 and 4 is | `>=` for `>`: the first is listed |
| A fill row is its own proposal's | the net has an older zone on another layer with 3 pieces: the proposal's row has its own count | matched by net only: the wrong count |
| No unconnected category is an error, not zero | a report without it: an error | `unwrap_or(0)`: a verdict |
| The owner's files are never written | the board's, the project's and the record's bytes before and after `check`, with a scripted refill that rewrites its scratch board | the scratch made beside the board: the folder's listing changes |
| The scratch project cannot hide a pour's faults | a project with `starved_thermal: ignore`: the scratch copy has `warning` | the forcing left out: `ignore` stays |
| Two calls never share a folder | `run_folder` twice with one stamp and text: the second is an error | `create_dir_all`: no error |
| kicad-cli runs on private settings | `KicadCli::refill` with a script in kicad-cli's place that records `KICAD_CONFIG_HOME` | `None` passed: the variable is not set |

### The tool

| Guard | Test | Negative control |
|---|---|---|
| The planner writes nothing | the bytes of the board, the project file and the record before and after `plan_pours` | (the check's own guard above; a write would have to be added) |
| No check, no plan | a kicad-cli that fails: an error result with no `proposals` | an unchecked plan returned: the test fails |
| A proposal KiCad fills nothing of is withdrawn | a check row with 0 pieces: the proposal is in `not_proposed`, with the reason | left in `proposals`: the test fails |
| The status tells the truth | clean and no unmet request: `planned`; an error that rose, or an unmet request: `planned_with_findings`; no proposal: `nothing_to_propose` | a condition inverted: the wrong status |
| The next steps name real tools | every tool and argument in `next_steps` is in the registry | a misspelt tool name: the test fails |
| A record that cannot be used is refused | a broken `design_intent.json`: `invalid_configuration`, no plan | read as absent: a plan is returned |
| The tool is where the documents say | `plan_pours` is in `pcb_board`; the registry's count is 18; `doc_tool_counts`, `asset_references` and `schema_parameter_usage` pass | the count left at 17: `registry_tool_counts_match_reality` fails |
| The real thing fills clean (ignored, needs kicad-cli) | the public `pours` fixture with its pours removed: `planned`, `clean`, two proposals with fill | the clearance raised to 0.5 on a fixture that then starves: the test fails |

Run `asset_references` and `schema_parameter_usage` (crate `konnect`) early: the
`konnect-core` suites do not cover them.

## Measurement: criteria fixed before any run

**Boards:** the five of "The x19 baseline" (dash, B5, `pic`, `ecc`, `pours`), each a scratch
copy with its copper pours removed (rule areas stay). **Build:** the branch's release binary
by pipe, the fastroute that `packaging/1-kopper/fastroute.json` pins on the day, effort M.
One call a board, each in its own routing hold, under the lane locks.

| Step | What | Criterion |
|---|---|---|
| M1 | `plan_pours` on the pour-less board (saved file, no KiCad open) | C1: the status is `planned` and `check.clean` is true |
| M2 | `add_zone` once per proposal with the returned arguments; then KiCad's own refill and DRC of that board | C2: every rule's count and every proposal's piece count equal the plan's own check |
| M3 | The DSN of the re-poured board, exported from its saved file as ZR0's fixtures were; one `route_specctra_dsn` call | C3: the winner passes G1 |
| M4 | SHA-256 of the board and the project file before and after M1 | C4: equal |

- **Done when C1 to C4 hold on all five boards** (the stage table's "the proposals refill
  clean and the route passes G1").
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
- `docs/API_MIGRATIONS.md`: one entry. The new tool; `add_zone`'s arguments and the two
  changes for an old call; the record's `pours` section and what an older Konnect does with it.
- `tool-directory.md`, `README.md`, `DEV.md`: the `plan_pours` row, `add_zone`'s row, the
  counts (`cargo xtask fix-doc-counts`).
- The `kicad-pcb` skill: `add_zone`'s new arguments and a short `plan_pours` entry in "Copper
  Pour". **The order of work does not change here.** "Pours last" changes with ZR5, on the
  owner's word (decision 5).
- `docs/ZONE_ROUTING_DESIGN.md` and `docs/CREATION_HANDOFF.md`: where the stage stands.

## What this design does not do

- **Local pours and split planes.** Whole layers only (PP5). They are candidates for ZR5,
  where routing can rank them.
- **A choice among divisions.** One division by rule (PP3). ZR5 adds candidates.
- **Stacks other than 2 and 4 layers** by rule (PP6). Requests work on any stack.
- **Settings in the record.** `pours` names nets and layers. A setting is changed in the
  proposal's arguments.
- **A long outline made short.** A board with arcs gives one corner per chord (about 12 a
  rounded corner of 2 mm). The arguments are exact and long.
- **Which inner layer is nearer the parts.** On 4 layers ground is always `In1.Cu`.
- **The project's own zone defaults** (`defaults.zones` of the `.kicad_pro`). They are the
  last values a zone dialog used, not a rule of the project.
- **Cleaning its folders.** Each call keeps `<state>/pour-plans/<stamp>-<sha8>/` (two board
  copies and two reports) as its record.
- **Windows and Linux:** macOS first, as the rest of the row.

## For ZR5

- The plan's division is one candidate. ZR5 adds others (ground on one side only, a power
  net on a layer, local pours) and routes each on a scratch copy, as
  `plan_convergent_placement` routes its three.
- `pour_plan::with_zones`, `ProposedZone` and `pour_check`'s scratch projects are what that
  needs of this stage.
- Clearance is a candidate dimension there too: PP9's floor is what the fill supports before
  routing; routing may say more.

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

**Skeleton compile** (2026-11-23, in a light lane slot): `cargo check --locked -p konnect-ipc
-p konnect-core` on this branch exits 0 with 0 errors. `konnect-core` gives 126 warnings, every
one an unused variable, import or item of the skeleton: nothing calls it yet. 32 bodies are
`todo!()`. `cargo fmt --all -- --check` is clean. Tests were not compiled or run.

**Independent review: not done.** A review by an agent that did not write this (Claude Opus
5.5, high, read only) was started and stopped unfinished when the owner paused all work on
the Mac (2026-11-23 15:31). It runs again before the plan is pushed, and its must-fix items
are taken into this text.

**Not verified.**

- The rule's front pour on `pic` and `ecc`, whose own boards have none (the build's M1).
- That KiCad's API applies the thermal values and the island mode of a created zone as
  `ZONE::Deserialize` reads them (the build's live test).
- The 4-layer rule on a real 4-layer board: none of the five is one. Unit tests only.
