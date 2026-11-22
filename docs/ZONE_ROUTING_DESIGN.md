# Zones in the router: research and design (Row ZR)

Status: research, and a design down to function skeletons, 2026-11-22. Nothing is built. Nothing
new was measured: every number below comes from a recorded run and names its record. Designed
in a Claude Fable 5.2 session; reviewed by an agent that did not write it, and corrected from
its findings (see "Verification of this document"). Branch `claude/zone-router-research`, from
`fork/main` `517eebf0`. Engine facts are from fastroute `61d6e45` (branch `ft-design`: the
pinned x18, `1e94751`, plus row FT). KiCad facts are from the KiCad 10.0.1 source.

## What the owner asked for

- 2026-11-22: "כיום המנתב שלנו לא מתחשב בZones fill בכלל ולא מגדיר אותם." A research task, toward
  adding zones to the router. Rust only down to function skeletons and the general logic.
- His standing values apply: result quality over run time, measurement first, one clear
  recommendation per decision.

## The short answer

1. **He is right on both counts.** The router is handed each pour, but only as its drawn
   outline. It never sees the copper KiCad will really fill, and it never creates a zone.
2. **The root cause is one rule in the engine:** a pad whose centre is inside a pour's outline
   counts as connected, and other nets may cross the pour for free. KiCad fills something else.
3. **The recommendation:** give the engine a model of the fill, price every cut of a pour while
   routing, and keep KiCad's refill as the judge. Six stages; each one is measured and can be
   the last.
4. **The first stage changes no routing.** It measures how wrong the outline reading is with
   fastroute, so every later stage has a number to beat. No such number exists yet.

## What the code does today

### Konnect

| What | Where | Fact |
|---|---|---|
| Export of a pour | `specctra.rs:2208` (`zones`), `:2339` | One Specctra `(plane NET (polygon …))` per pour and copper layer, from the zone's **outline**. Its own comment: "The zone's outline is used, not its fill, so thermal reliefs and clearance gaps are not modelled" |
| What the export reads of a zone | `specctra.rs:2208-2294` | Net, layers, outline, keepout flags, UUID, name. It reads none of: clearance, `min_thickness`, `connect_pads`, thermal gap and bridge, priority, island removal, hatch |
| The choice per pour | `specctra.rs:537` (`PourMode`) | `fixed` (a plane) or `omit` (left out, its net routed as track). No third form |
| The search | `route_search.rs:133` (`Shape`) | Two shapes of every DSN: as exported, and every plane stripped |
| The judge | `route_judge.rs:26-28`, `:780-885` | KiCad refill and DRC on a scratch copy, then at most 4 stitching rounds (vias only, radius 3 mm) for the nets given as planes |
| Creating a zone | `tools/pcb_board.rs:2766` | Only `add_zone` and its alias `add_copper_pour`, called by hand. No tool decides a zone's net, layer, outline or settings |
| Guidance to agents | `kicad-pcb/SKILL.md:141-145` | "add ground/power zones last"; "Do NOT add copper pours before routing is complete". An agent that follows it routes a board with no zone at all |

### fastroute

| What | Where | Fact |
|---|---|---|
| A plane on load | `fr-io/src/structure.rs:726` | A conduction area with its polygon and holes, `is_obstacle: false` |
| When a pad touches a plane | `fr-engine/src/board/connectivity.rs:204` | Same net, same layer, and the pad's or via's **centre** inside the polygon; for a trace, an end point inside it |
| When a plane net is done | `autoroute/router.rs:466`, and `:227` | The item's connected set holds a conduction area: `ConnectedToPlane`. Any conduction area, anywhere |
| Other nets on a pour | `board/item.rs` (`is_obstacle`), `autoroute/maze.rs:669` | On a signal layer they cross it freely, and the maze's step cost has no pour term. The only pour cost is the pour net's own via cost (`plane_via_costs`, default 5). (On a power-type layer no track is laid, and a via through another net's plane there is refused, `maze.rs:378`) |
| The one switch | `board/routing_board.rs:1072` | `--router.plane_as_obstacle`: every pour on a signal layer blocks every other net. Board-wide, all or nothing |
| Its stand-in for a zone | `board/basic_board.rs:341`, `connectivity.rs:312` | Stitching vias: the fixed vias of a net that touch nothing count as joined to each other, "by their zone", with no zone on the board |
| The report | `fastroute/src/report.rs` | No field about pours |
| Its own KiCad plugin | `integrations/kicad/plugins/core.py:1145`, `:1095` | It strips the pours on signal layers, for the reason of this document: "pads inside count as connected even if the refilled zone cuts them off" (`docs/IMPROVEMENTS.md`, "Zone handling"). For nets named as obstacles it replaces the outline by KiCad's **real fill**. So the engine already loads a fill as planes |

### KiCad's fill

`ZONE_FILLER::fillCopperZone` (`pcbnew/zone_filler.cpp:2568`) builds a pour in this order:

1. The smoothed outline (a zone may round or chamfer its corners).
2. Minus a gap around each pad of its own net that connects by thermal relief.
3. Minus the clearance around every other net's pad, track, via and hole, the board edge and
   rule areas. The clearance is the worst of the zone's own and the rules', plus 0.0005 mm; a
   hole takes the worse of the clearance and the hole clearance.
4. Plus the thermal spokes, each kept only if its far end lands on copper that survives step 5.
5. Minus everything narrower than the zone's minimum width (shrink by half, drop small blobs,
   grow back).
6. Minus higher-priority zones of other nets with their clearance, and the minimum width
   again; then higher-priority zones of its own net.
7. Then, in `Fill` (`:871-957`): pieces that touch nothing of their net are removed, by the
   zone's island rule. If every piece is such an island, none is removed.

About the spokes (`:3073-3435`, `padstack.cpp:1114`):

- Four per pad, from the pad shape's own position: a `+` for rectangular and oval pads, an `X`
  for round ones, unless the pad sets its own angle.
- The width is the zone's bridge width, cut down to the pad's smaller side. A spoke narrower
  than the zone's minimum width is not made at all.
- A via gets no thermal relief: a pour joins a via solidly.
- A pad can override its zone's connection, gap and bridge, and can inherit them from its
  footprint.

So a pad inside the outline is connected only when copper survives all of this next to it, and
the piece it sits on reaches the rest of the net. DRC adds one more rule: a thermal pad with at
least one spoke and fewer than the minimum (2 by default) is a `starved_thermal`
(`drc/drc_test_provider_zone_connections.cpp:120`). A track from inside the pad onto the fill
counts as a spoke (`:230-242`).

## The gap in numbers

From the recorded matrix of row S6, second half (`CREATION_HANDOFF.md`, "Row S6, second half";
the reference board, 3 routings per cell). **It was measured with Freerouting 2.5.0**, before
fastroute. fastroute reads a plane by the same rule, so the shape of the result is expected to
hold; the numbers are not fastroute's. Stage ZR0 makes the first fastroute baseline.

| What the router is given | Router's own open count | Open on ground after KiCad's refill | After 4 stitching rounds | Track on the ground net |
|---|---|---|---|---|
| Every pour as a plane (today's default) | 0 | 19, 19, 19 | 6, 9, 8 | 0 mm |
| Every pour omitted | 0 | 0, 0, 0 | none needed | 583 to 585 mm (hand routing: 81.7 mm) |
| Only the bottom pour as a plane | 0 | 1, 1, 1 | 0, 0, 0 | 71.6 mm |

- **Row 1:** the router calls 19 open connections closed. Stitching repairs most, not all.
- **Row 2:** safe, and it lays seven times the ground track of the hand routing.
- **Row 3:** one pour that happens to stay whole brings the ground net close to the hand
  routing. (Its power nets were still far below their width; that is row S9's subject, not this
  row's.) Getting row 3 on purpose instead of by luck is what this row is after.
- **With fastroute, on the public fixtures (row BN):** on `pic` the pours-omitted shape wins at
  M, H and XH; on the pours fixture the winner keeps its pours. So neither shape is always
  right, and the search pays for both on every call.

## Options considered

| | Option | For | Against |
|---|---|---|---|
| A | Stay blind: route with pours omitted, fill afterwards | Works today, no new code | Keeps row 2: ground eats the routing channels, and nothing protects the return path |
| B | KiCad in the loop: refill between engine runs and feed the real fill as planes | Exact fill, no model to maintain; the plugin proves the engine loads it | Truth arrives only between engine runs (2 or 3 a route). The damage is done inside a run. The engine still needs changes |
| **C** | **A fill model inside the engine, KiCad as the judge** | The engine can price a cut at the moment it makes it, and close what it broke before it exits | A model can be wrong. It is kept on the safe side and is checked against KiCad on every run |

**Recommended: C.** B is not thrown away: it is the judge that already exists, and stage ZR0
uses it as a cheap probe before any engine work.

## The design

### Principles

- **P1. KiCad's refill is the truth.** The model predicts; the judge's refill and DRC decide.
- **P2. The model errs to the safe side.** It may miss copper that KiCad fills. It must never
  call a pad connected that KiCad leaves open. Stage ZR1 measures exactly that.
- **P3. Off by default.** Without `--zones` the engine takes today's code paths, byte for byte.
- **P4. Nothing is guessed.** Where the model does not cover a board, the pour keeps today's
  reading, and the result names it and the reason:
  - one zone: a hatch fill, rounded or chamfered corners, a setting that cannot be read;
  - every zone of the board: a custom rule that can change a zone's clearance or connection; a
    pad or via with unused-layer removal, a zone-layer override or a backdrill; a custom pad
    with its own spoke shapes on a pour net; no manifest, or a manifest and DSN that disagree
    on the planes.
- **P5. No hidden write.** Konnect proposes zones; a person or an agent adds them with
  `add_zone`. The route call still writes nothing to the board.

### The fill model

One raster per copper layer that has a modelled zone. Cell size 0.05 mm (`cell_mm` in the zones
file): a fifth of KiCad's default minimum width.

For each zone, highest priority first:

1. **Inside:** the cells whose centre is inside the outline and outside its holes, less one
   cell along the border.
2. **Knockouts:** stamp every blocking shape, grown by its clearance plus one cell:
   - another net's pad, via and track; its hole, by the larger of the clearance and the hole
     clearance;
   - every keepout the DSN carries (unplated holes and copper text are exported as keepouts);
   - the board edge and every area that keeps pours out;
   - the fill of a higher-priority zone of another net;
   - where the board has teardrops: the hull of each other-net pad or via and the start of each
     track that enters it;
   - a pad of the zone's own net that connects by thermal relief, grown by its gap;
   - a pad of its own net set to "no connection".
3. **Spokes:** for each thermal pad that overlaps the fill of step 1 and 2: four spokes at the
   pad's angle, of its bridge width cut down to the pad's smaller side. None when that is
   narrower than the minimum width. A spoke is kept when its far end lies on fill that
   survives step 4 and no other net's knockout cuts it.
4. **Minimum width:** an opening (erode, then dilate) by half the minimum width.
5. **Pieces:** label the 4-connected pieces. Diagonal contact is not copper.
6. **Islands:** a piece that nothing of its net sits on is removed, by the zone's rule.

That is the **snapshot**: it depends on the pads and on other nets' copper, and it is redrawn
at fixed points (below). What sits on which piece is asked of the board as it is at that moment:

7. **Attachments:** a via or a solid pad by its centre cell; a thermal pad by the pieces its
   kept spokes reach; a track by every piece its centre line crosses.
8. **Groups:** per net, items and pieces joined through the attachments and through direct
   contact. The **main group** has the most pins. A piece of the main group is **live**.

Every rounding in steps 1 to 4 is toward less copper. Where KiCad makes more copper than the
model (it keeps a spoke that only meets another spoke, it bridges pieces that nearly touch, it
keeps every piece when all are islands), the model is short, which P2 allows.

Why a raster and not polygons: the engine has no polygon clipper. A raster is exact enough to
count pieces and price cuts, and cannot fail on a degenerate shape. Its cost grows with the
board's area (4 million cells per layer for 100 × 100 mm) and is measured in ZR1; the cell size
is a parameter. KiCad keeps the exact polygons.

### What the engine does with it

1. **Truth.** For a net that has a modelled pour, the model's groups are the connectivity:
   - a piece joins everything that sits on it;
   - only the main group reaches the pour, so only its items are `ConnectedToPlane`. Both
     early returns of the router read the same set, so both follow;
   - a pad the fill never reaches, or one on a cut-off piece, is open, and the autorouter
     routes it;
   - the stitching-via stand-in is off for such a net: the model says where a via sits;
   - the engine's own count then matches what KiCad will find.
   A new via onto a pour must land on a live cell; the maze takes its landing point from the
   model.
2. **Cost.** A step of another net across live fill costs extra, per length. A via of another
   net through live fill costs extra, once. So a pour stays whole where the board allows it, and
   is cut where it must be. This is the middle ground `plane_as_obstacle` does not have. The
   two can be on together (row LT's pour-safe candidate): the pours then block as well, and the
   model still decides what is connected.
3. **Closing.** After the routing passes, and again after the optimizer, the snapshot is
   redrawn from the board as routed. Every stranded group is routed to the main group, with
   tracks, vias and rip-up. A starved pad gets a short track onto the fill, which KiCad counts
   as a spoke. Rounds repeat until nothing is open or a round closes nothing.

**When the snapshot is redrawn:** before every autoroute pass and every optimizer pass, when
the pipeline swaps the board for a saved one, before each closing round, and before the report.
Between those points a cut made by another net is not yet in it; the closing stage catches the
last ones. Groups are never stale: they are computed from the snapshot and the board's current
items, and cached by the board's own change counter, as the engine's contact cache is. The
parallel pass routes on board copies; each copy shares the snapshot and has its own groups.

**Known limit of the cost:** the optimizer compares routes by open connections, vias and
length. It can shorten a track across a pour. The closing stage after it repairs the
connection; whether the cost must also enter the optimizer's comparison is decided on ZR3's
numbers.

### What Konnect sends and reads

- **Sends** `--zones=<file>`, like `--pairs` and `--tune`. One `zone` line per modelled plane,
  keyed by the plane's place in the DSN:

  ```text
  zones 1
  cell_mm 0.05
  min_spokes 2
  edge_clearance_mm 0.3
  hole_clearance_mm 0.25
  teardrop_mm 0
  zone 0 clearance_mm=0.2 min_width_mm=0.25 connect=thermal gap_mm=0.5 bridge_mm=0.5 priority=0 islands=remove
  pad U3 5 connect=thermal gap_mm=0.5 bridge_mm=0.5 angle_deg=45
  keepout B.Cu 1000 -2000 3000 -2000 3000 -4000
  ```

  - A `pad` line carries a pad's resolved connection, gap, bridge and angle, and is written
    when any of them differs from its zone's values at angle 0. So the engine holds no KiCad
    default: Konnect resolves pad, footprint and zone, and the round pads' 45 degrees.
  - A plane with no `zone` line keeps today's reading. The pours-omitted shape gets no file.
- **Reads** a `planes` section of the engine's report: the pads the model leaves open, the
  starved pads, what the closing stage routed, the piece count.
- **Compares** it with KiCad's refill in the judge, pad by pad. KiCad's side is the layout
  audit's own grouping over the refilled board's saved fills (`layout_quality_planes`, which
  already reports the pads cut off from a ground net's main group). A pad the model put in the
  main group and that grouping leaves outside is a **false connection**: the one number P2 is
  about.

### Defining zones

A read-only tool, `plan_pours`, shaped like `plan_route_refinement`:

- **Which nets:** the design intent's new `pours` list. Without it, the ground rail only (a rail
  of 0 V is a stated fact, not a guess from a name). No intent: no proposal, and it says so.
- **Which layers:** by the stackup. 2 copper layers: ground on the bottom, and on the top as
  well. 4 layers: ground on the first inner layer, the main power rail on the second.
- **Outline:** the board outline. KiCad clips it to the edge clearance and to every rule area,
  so isolation barriers and antenna keepouts hold.
- **Settings:** clearance and minimum width from the netclass and the fab target, KiCad's
  defaults for the rest.
- **Existing zones are never changed.** A net that already has a pour on a layer is skipped.
- **Result:** one ready `add_zone` argument set per proposal. Nothing is written.

The order of work then becomes: plan pours, add them, export, route, refill. The skill's "pours
last" rule changes only after stage ZR2 measures better than today.

## Stages

Each stage is its own PR, measured on the public fixtures and the owner's boards at effort M.

| Stage | Where | What | Done when |
|---|---|---|---|
| **ZR0** Measure | Konnect | The judge records, per run: open on pour nets before stitching, stitching vias, fill pieces, the largest piece's share, starved thermals, the winner's shape. Plus one probe in the bench: planes from KiCad's real fill of the unrouted board | A baseline table per board and shape, with fastroute. Sessions byte-identical to before |
| **ZR1** Model | engine, Konnect | `--zones`, the model, the report. Routing unchanged | Zero false connections on every run. Sessions byte-identical with and without `--zones`. The redraw's time recorded |
| **ZR2** Truth | engine | The model's groups as connectivity, landing points, the closing stage | Open before stitching is 0 and no stitching via is needed, on at least 9 of 10 judged as-exported runs. G1 and G2 pass shares not below ZR0 |
| **ZR3** Cost | engine | The cut cost, with its weight chosen from a small matrix | Fewer pieces and a larger main piece than ZR2 at the same gate shares; ground track of the winner near the hand routing's |
| **ZR4** Define | Konnect | `plan_pours`, the intent's `pours`, `add_zone`'s missing settings | On each board with its pours removed, the proposals refill clean and the route passes G1 |
| **ZR5** Rank | Konnect | Pour measures in the rank key; the skill's order of work | The owner's word, on ZR3's numbers |

ZR4 touches no engine file and may run beside ZR1 to ZR3.

## Decisions taken in this design (with the reason)

| # | Decision | Reason |
|---|---|---|
| Z1 | The model lives in the engine, not in Konnect | Only the engine can price a cut when it makes it. Konnect already has the exact answer afterwards, from KiCad |
| Z2 | A raster, 0.05 mm cells | No clipper in the engine; pieces and cuts need counting, not exact edges; it cannot fail on a bad polygon |
| Z3 | Every rounding is toward less copper: the outline loses a cell, every blocking shape gains one | P2: a raster rounds, and it must never round a neck or a pad into copper |
| Z4 | For a modelled net the model's groups replace the engine's own walk. A piece joins what sits on it; only the main group reaches the pour | The engine's walk treats a pour as one node, so two cut-off pieces would join. Replacing the set in one place makes every caller right: the two early returns, the item list, the open count |
| Z5 | A cost, not an obstacle | An obstacle on a 2-layer board with both sides poured leaves nothing to route on (row LT met this with the pour-safe candidate) |
| Z6 | Parameters travel in a side file keyed by plane index, and the loader records which item each plane became | The DSN has no place for them; `--pairs` and `--tune` set the pattern; an index needs no net name. The loader can skip a plane and adds areas of its own, so the index is recorded, not counted |
| Z7 | The saved board is the source of the parameters, at route time | The route call already reads it (the judge's preflight). The manifest stays as it is; its `pours` rows are in the DSN's plane order |
| Z8 | Zones are defined before the export, by an explicit tool | The importer checks the board against the export's snapshot. A zone added after the export would be refused. It also keeps Q1 ("a second opinion, never the author") |
| Z9 | The judge's stitching rounds stay | They are the fallback, and their count is the measure of how much the engine still misses |
| Z10 | Pour terms enter the rank key last (ZR5) | The key reproduces a measured order. Changing it needs its own numbers (the reasoning of D10) |
| Z11 | The raster is a shared snapshot; groups are computed per board and cached by its change counter | A snapshot can be shared by the parallel pass's board copies. Groups must follow every insert and rip-up, and a cache keyed by the counter cannot go stale |
| Z12 | Konnect resolves every pad's thermal values; the engine does geometry only | KiCad's defaults (a round pad's 45 degrees, footprint inheritance) stay in one place, beside the code that reads KiCad files |
| Z13 | KiCad's side of the comparison is the audit's grouping over the saved fills | KiCad's DRC reports open pairs between groups, not open pads. The audit already turns a refilled board into "pads outside the main group" |

## Decisions for the owner

1. **Go ahead with ZR0 and ZR1?** They change no routing: a fastroute baseline, then the model
   beside KiCad's truth. Recommended: yes. The model's error is the one unknown the rest
   depends on.
2. **The engine row waits for row FT.** FT is built in fastroute and not yet measured or
   pinned. Recommended: pin FT first, then build ZR1 on top, so each build change is measured
   alone.
3. **Where pours come from (ZR4).** Recommended: a `pours` list in the design intent, ground
   rail by default, and a read-only planner. He adds the zones, or tells the agent to.
4. **Ground on both sides of a 2-layer board?** Recommended: yes, bottom as the main plane and
   top as fill. Both are in the checklist's spirit; the model handles the top fill's islands.
5. **When the skill stops saying "pours last".** Recommended: only after ZR2's table shows the
   as-exported shape at 0 open before stitching.

## Modules

fastroute, new files in `crates/fr-engine/src/planes/`:

| File | What |
|---|---|
| `zones_file.rs` | the `--zones` file |
| `raster.rs` | the grid, bitmaps, stamping, erode and dilate, labelling |
| `model.rs` | the snapshot, and what the engine asks of it |
| `graph.rs` | groups of a net, the main group, live pieces |
| `cost.rs` | the maze's surcharges |
| `closing.rs` | the closing stage |

fastroute, changes to existing items:

| Where | Change |
|---|---|
| `fr-io/src/structure.rs` | records, per `(plane …)` of the DSN in order, the item it became or that it was skipped; the board keeps the list |
| `board/routing_board.rs` | `planes: Option<Arc<PlaneModel>>`, shared by board copies |
| `board/connectivity.rs` | `connected_set` for a net with a modelled pour returns the model's group, with the pour's item only for the main group; `conduction_area_normal_contacts` and its pad, via and trace side use `PlaneModel::live_contact`; the stitching-via joins are skipped for such a net; the contact cache's key gains the model's generation |
| `pipeline/stats.rs` | the open-count memo's key gains the model's generation |
| `autoroute/maze.rs` | `expand_to_door_section` adds `cost::trace_surcharge`; the layer change adds `cost::via_surcharge`; a target door on a modelled pour is taken only where `PlaneModel::landing` finds a point |
| `autoroute/locator.rs` | a trace end into a modelled pour is placed at the landing point |
| `pipeline/mod.rs`, `pipeline/optimizer.rs` | the redraw points; `closing::close_planes` after the passes and after the optimizer |
| `pipeline/fanout.rs` | a fanout via into a modelled pour takes its landing point too (Konnect's preset has fanout off) |
| `fastroute/src/main.rs` | `--zones=FILE`; `--router.scoring.plane_cut_cost=`, `plane_via_cut_cost=`; `mark_stitching_vias` leaves a modelled net's vias unmarked |
| `fastroute/src/report.rs` | the `planes` section |

Konnect, new files in `crates/konnect-core/src/`:

| File | What |
|---|---|
| `zone_fill.rs` | each pour's fill settings from the saved board; the zones file; the report's `planes`; the pour measures |
| `pour_plan.rs` | the pours a board should have (ZR4) |
| `tools/pour_plan_tool.rs` | the `plan_pours` tool |

Konnect, changes to existing items:

| Where | Change |
|---|---|
| `specctra.rs` | `pour_fill_rows`: the pours in export order, each with its `(zone …)` node, for `zone_fill` to read |
| `fastroute.rs` | `CallFiles.zones: Option<&Path>`, sent by `pass_arguments` as `--zones=` after `--layer-heights`, only for a variant with planes |
| `route_judge.rs` | `Measures.pours: Option<PourMeasures>`, filled after the first refill and again after the last |
| `tools/route_fastroute.rs` | builds the zone plan after the pairs plan, writes `zones.txt`, adds `pours` to the result |
| `tools/design_intent.rs` | `DesignIntent.pours: Vec<PourIntent>`, validated like the other sections |
| `tools/pcb_board.rs` | `add_zone` gains `thermal_gap`, `thermal_bridge_width` and `island_removal` |
| `packaging/1-kopper/fastroute.json` | the build that knows `--zones`, in its own PR |

## Skeletons

Bodies are `todo!()`. Compiled as described under "Verification of this document".

### fastroute: `planes/zones_file.rs`

```rust
//! Row ZR: the `--zones` file. One `zone` line per modelled plane of the DSN, `pad` lines for
//! pads whose thermal values differ from their zone's, `keepout` lines for areas no zone may
//! fill.

/// How a pour joins a pad of its own net.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadConnect {
    /// Copper straight onto the pad.
    Solid,
    /// A gap around the pad, crossed by spokes.
    Thermal,
    /// Kept clear of the pad, like another net's copper.
    None,
}

/// What happens to a fill piece that nothing of its net sits on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IslandRule {
    Remove,
    Keep,
    RemoveBelow { area_mm2: f64 },
}

/// One `zone` line: the fill settings of one `(plane …)` of the DSN.
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneParams {
    /// The plane's place among the `(plane …)` entries of the DSN's structure, from 0.
    pub plane_index: usize,
    pub clearance_mm: f64,
    pub min_width_mm: f64,
    /// For every pad of the net that has no `pad` line.
    pub connect: PadConnect,
    pub thermal_gap_mm: f64,
    pub thermal_bridge_mm: f64,
    /// Higher wins where zones overlap.
    pub priority: u32,
    pub islands: IslandRule,
}

/// One `pad` line: a pad's own resolved values, on every layer of the pad.
#[derive(Debug, Clone, PartialEq)]
pub struct PadParams {
    pub component: String,
    pub pin: String,
    pub connect: PadConnect,
    pub thermal_gap_mm: f64,
    pub thermal_bridge_mm: f64,
    /// The spokes' angle from the pad's own axes: 0 is a `+`, 45 an `X`.
    pub angle_deg: f64,
}

/// One `keepout` line: an area no zone may fill, in the DSN's own unit and axes.
#[derive(Debug, Clone, PartialEq)]
pub struct FillKeepout {
    pub layer: String,
    pub corners: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ZonesFile {
    pub cell_mm: f64,
    /// A thermal pad with at least one spoke and fewer than this is starved.
    pub min_spokes: u32,
    pub edge_clearance_mm: f64,
    pub hole_clearance_mm: f64,
    /// The longest teardrop a pad or via of the board allows; 0 when the board has none.
    pub teardrop_mm: f64,
    pub zones: Vec<ZoneParams>,
    pub pads: Vec<PadParams>,
    pub keepouts: Vec<FillKeepout>,
}

/// Reads the file. The first line must be `zones 1`. Then, in any order: `cell_mm`,
/// `min_spokes`, `edge_clearance_mm`, `hole_clearance_mm`, `teardrop_mm` (each exactly once),
/// `zone <index> key=value…`, `pad <component> <pin> key=value…`,
/// `keepout <layer> x y x y…`. Blank lines and lines that start with `#` are skipped.
///
/// Refuses, naming the line: an unknown first word or key, a key given twice or missing, a
/// number that is not finite or is below zero, a `cell_mm` outside 0.01 to 0.2, a plane index
/// or a pad given twice, a keepout with fewer than three corners. Nothing is defaulted.
pub fn parse(text: &str) -> Result<ZonesFile, String> {
    todo!()
}
```

### fastroute: `planes/raster.rs`

```rust
//! Row ZR: the raster the fill model is drawn on.

use fr_geom::{Area, FloatPoint, IntBox, TileShape};

/// A square grid over the board, in board units. Cell `(x, y)` has the index
/// `y * width + x`.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub min_x: f64,
    pub min_y: f64,
    pub cell: f64,
    pub width: usize,
    pub height: usize,
}

impl Grid {
    /// The grid that covers `bounds` with cells of `cell` board units, one spare cell on
    /// every side.
    pub fn covering(bounds: &IntBox, cell: f64) -> Grid {
        todo!()
    }

    /// The cell that holds `point`, or `None` outside the grid.
    pub fn cell_at(&self, point: &FloatPoint) -> Option<usize> {
        todo!()
    }

    pub fn centre(&self, cell: usize) -> FloatPoint {
        todo!()
    }
}

/// One bit per cell of a grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Bitmap {
    pub words: Vec<u64>,
    pub cells: usize,
}

impl Bitmap {
    pub fn new(cells: usize) -> Bitmap {
        todo!()
    }

    pub fn get(&self, cell: usize) -> bool {
        todo!()
    }

    pub fn set(&mut self, cell: usize, value: bool) {
        todo!()
    }

    pub fn count(&self) -> usize {
        todo!()
    }
}

/// Sets every cell whose centre is inside `area` and outside its holes (scanline, even-odd).
pub fn fill_area(grid: &Grid, area: &Area, into: &mut Bitmap) {
    todo!()
}

/// Sets every cell whose centre is nearer than `grow` to `shape` (inside counts as 0).
/// Only the cells of the shape's box, grown by `grow`, are visited.
pub fn stamp_shape(grid: &Grid, shape: &TileShape, grow: f64, into: &mut Bitmap) {
    todo!()
}

/// Sets every cell whose centre is nearer than `radius` to the segment `a`-`b`: a track of
/// that half width plus its clearance.
pub fn stamp_segment(grid: &Grid, a: &FloatPoint, b: &FloatPoint, radius: f64, into: &mut Bitmap) {
    todo!()
}

/// The cells of `fill` that are at least `radius` from every cell outside it (the grid's
/// border counts as outside). An exact Euclidean distance transform (Felzenszwalb and
/// Huttenlocher), so the result does not depend on scan order.
pub fn erode(grid: &Grid, fill: &Bitmap, radius: f64) -> Bitmap {
    todo!()
}

/// The cells nearer than `radius` to a cell of `fill`. The same transform.
pub fn dilate(grid: &Grid, fill: &Bitmap, radius: f64) -> Bitmap {
    todo!()
}

/// The 4-connected pieces of a bitmap. `of_cell` is 0 outside the fill, else the piece's
/// number from 1, numbered in scan order (so the same bitmap always gives the same numbers).
/// `sizes[n]` is the cell count of piece `n`; `sizes[0]` is unused.
#[derive(Debug, Clone, PartialEq)]
pub struct Labels {
    pub of_cell: Vec<u32>,
    pub sizes: Vec<u32>,
}

pub fn label(grid: &Grid, fill: &Bitmap) -> Labels {
    todo!()
}

/// The length of the segment `a`-`b` that lies over set cells, in board units: a walk along
/// the segment in steps of half a cell.
pub fn length_inside(grid: &Grid, fill: &Bitmap, a: &FloatPoint, b: &FloatPoint) -> f64 {
    todo!()
}
```

### fastroute: `planes/graph.rs`

```rust
//! Row ZR: what the fill really connects. Per net, items and fill pieces joined into groups.

use crate::board::{ItemKey, RoutingBoard};
use crate::ids::NetNo;

use super::model::{Attachment, ZoneFill};

/// The groups of one net. A group is a set of items and fill pieces that are one piece of
/// copper.
#[derive(Debug, Clone)]
pub struct NetGroups {
    /// Each item of the net and its group, in item id order.
    pub item_group: Vec<(ItemKey, usize)>,
    /// Each fill piece `(zone index, piece number)` that has an attachment, and its group.
    pub piece_group: Vec<((usize, u32), usize)>,
    /// Pins per group.
    pub pins: Vec<usize>,
    /// The group with the most pins; among equals the one with the most fill cells, then
    /// the one that holds the item with the lowest id.
    pub main: usize,
}

/// The groups of `net` on the board as it is: a union-find over the net's items and the
/// pieces of its modelled zones.
/// 1. Two items are joined when they touch (`normal_contacts`). The conduction areas of
///    modelled planes are left out of this step, and so are the stitching-via joins. A plane
///    of the net that is not modelled stays an item and joins what the outline rule says.
/// 2. An item and a piece are joined by each attachment.
/// 3. Pieces of two zones of the net on one layer are joined where their cells touch.
pub fn net_groups(board: &RoutingBoard, net: NetNo, zones: &[ZoneFill], attachments: &[Attachment]) -> NetGroups {
    todo!()
}

impl NetGroups {
    /// The group of an item of the net.
    pub fn group_of(&self, item: ItemKey) -> Option<usize> {
        todo!()
    }

    /// The items of one group, in item id order.
    pub fn items_of(&self, group: usize) -> Vec<ItemKey> {
        todo!()
    }

    /// Whether a fill piece belongs to the main group.
    pub fn is_live(&self, zone: usize, piece: u32) -> bool {
        todo!()
    }

    /// Every group but the main one, by its first item.
    pub fn stranded(&self) -> Vec<usize> {
        todo!()
    }
}
```

### fastroute: `planes/model.rs`

```rust
//! Row ZR: the fill model. A snapshot drawn from the board and the zones file, and the
//! questions the connectivity, the maze and the closing stage ask of it. It never changes the
//! board, and nothing changes it after `build`.

use crate::board::{ItemKey, RoutingBoard};
use crate::ids::{LayerNo, NetNo};
use fr_geom::{FloatPoint, TileShape};

use super::graph::NetGroups;
use super::raster::{Bitmap, Grid, Labels};
use super::zones_file::{ZoneParams, ZonesFile};

/// The predicted fill of one modelled plane.
#[derive(Debug, Clone)]
pub struct ZoneFill {
    /// The plane's conduction area item.
    pub area: ItemKey,
    pub net: NetNo,
    pub layer: LayerNo,
    pub params: ZoneParams,
    pub fill: Bitmap,
    pub labels: Labels,
}

/// The kept spokes of one thermal pin on one zone: how many reach each piece.
#[derive(Debug, Clone, PartialEq)]
pub struct PinSpokes {
    pub pin: ItemKey,
    /// Index into [`PlaneModel::zones`].
    pub zone: usize,
    /// `(piece number, spokes that reach it)`.
    pub reach: Vec<(u32, u32)>,
}

/// One item of a zone's net and a fill piece it sits on.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub item: ItemKey,
    pub zone: usize,
    pub piece: u32,
}

/// Why an item of a modelled net is not done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenKind {
    /// Not in its net's main group.
    Stranded,
    /// A thermal pin of the main group with at least one spoke and fewer than `min_spokes`.
    Starved { spokes: u32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenItem {
    pub item: ItemKey,
    pub net: NetNo,
    pub kind: OpenKind,
}

#[derive(Debug, Clone)]
pub struct PlaneModel {
    pub grid: Grid,
    pub zones: Vec<ZoneFill>,
    /// Planes of the DSN with no `zone` line: they keep the outline reading.
    pub legacy: Vec<ItemKey>,
    /// Pins are fixed, so their spokes belong to the snapshot.
    pub spokes: Vec<PinSpokes>,
    pub min_spokes: u32,
    /// A new number on every build: part of the key of every cache that holds a contact or a
    /// count.
    pub generation: u64,
}

impl PlaneModel {
    /// Draws the snapshot from the board as it is now.
    /// 1. The planes: the board's record of which item each `(plane …)` of the DSN became.
    ///    A `zone` line whose plane was skipped at load, or has no entry, is an error.
    /// 2. The grid: the board's bounding box at `file.cell_mm`.
    /// 3. Per layer, the zones by priority, highest first: [`inside`], minus [`knockouts`],
    ///    plus [`spokes`], then `raster::erode` and `raster::dilate` by half the minimum
    ///    width, then `raster::label`.
    /// 4. The island rule: a piece that no pin, via or trace of the net sits on
    ///    ([`PlaneModel::attachments`]) is cleared, and the zone labelled again.
    pub fn build(board: &RoutingBoard, file: &ZonesFile, generation: u64) -> Result<PlaneModel, String> {
        todo!()
    }

    /// Whether `net` has a modelled plane.
    pub fn models(&self, net: NetNo) -> bool {
        todo!()
    }

    /// What sits on which piece, for the items `net` has on the board now: a via or a solid
    /// pin by its centre cell; a thermal pin by `self.spokes`; a trace by every piece its
    /// centre line crosses. In item id order.
    pub fn attachments(&self, board: &RoutingBoard, net: NetNo) -> Vec<Attachment> {
        todo!()
    }

    /// The groups of `net` on the board now (`graph::net_groups` over [`Self::attachments`]).
    /// The caller caches the result by this model's generation and the board's item epoch.
    pub fn groups(&self, board: &RoutingBoard, net: NetNo) -> NetGroups {
        todo!()
    }

    /// The contact rule of a modelled plane: whether `item` sits on a live piece of `area`.
    /// `None` when `area` is not a modelled plane; the caller then keeps the outline rule.
    pub fn live_contact(&self, board: &RoutingBoard, area: ItemKey, item: ItemKey) -> Option<bool> {
        todo!()
    }

    /// A point inside `within` where a track end or via of half width `half_width` lands on
    /// a live piece of `area`, every cell under it being fill: the candidate nearest to
    /// `within`'s centre of gravity. `None` when there is none.
    pub fn landing(&self, board: &RoutingBoard, area: ItemKey, within: &TileShape, half_width: f64) -> Option<FloatPoint> {
        todo!()
    }

    /// Every pin and via of a modelled net that is stranded or starved, in item id order.
    pub fn open_items(&self, board: &RoutingBoard) -> Vec<OpenItem> {
        todo!()
    }

    /// The length of `a`-`b` on `layer` that lies over a live piece of a net other than
    /// `net`.
    pub fn foreign_length(&self, board: &RoutingBoard, layer: LayerNo, net: NetNo, a: &FloatPoint, b: &FloatPoint) -> f64 {
        todo!()
    }

    /// Whether a disc of `radius` at `at` on `layer` touches a live piece of a net other
    /// than `net`.
    pub fn foreign_disc(&self, board: &RoutingBoard, layer: LayerNo, net: NetNo, at: &FloatPoint, radius: f64) -> bool {
        todo!()
    }
}

/// The cells inside a plane's outline and outside its holes (`raster::fill_area` on the
/// conduction area's shape), less one cell along the border (`raster::erode` by one cell).
fn inside(board: &RoutingBoard, grid: &Grid, area: ItemKey) -> Bitmap {
    todo!()
}

/// The cells a zone may not fill. Every blocking shape is stamped grown by its clearance
/// plus one cell:
/// - a pin, via or trace of another net on the layer: the larger of the zone's clearance and
///   the engine's clearance between the two items' classes;
/// - a drill hole of another net: the larger of that and `file.hole_clearance_mm`;
/// - every obstacle area and keepout of the board on the layer: the zone's clearance;
/// - the board's outline: `file.edge_clearance_mm`; and every cell outside the board;
/// - each `file.keepouts` area of the layer, grown by nothing;
/// - the fill of each zone of another net already drawn on the layer (`above`): the zone's
///   clearance;
/// - when `file.teardrop_mm` is above 0: the convex hull of each other-net pin or via and
///   the first `teardrop_mm` of each trace that ends in it, grown as a trace is;
/// - a pin of the zone's own net that connects by thermal relief (its `pad` line, else the
///   zone's rule): its gap;
/// - a pin of its own net set to no connection: the clearance.
fn knockouts(
    board: &RoutingBoard,
    grid: &Grid,
    net: NetNo,
    layer: LayerNo,
    params: &ZoneParams,
    file: &ZonesFile,
    above: &[ZoneFill],
) -> Bitmap {
    todo!()
}

/// The spokes of the zone's thermal pins. Only a pin whose shape overlaps `raw` (the fill
/// before spokes) gets any. The width is the pin's bridge width cut down to the pin's
/// smaller side; a width below the zone's minimum width gives no spoke. Four rectangles of
/// that width from the pin shape's own position, at the pin's angle from its axes, to the
/// pin's box grown by the gap and half the minimum width. A spoke is kept when its far end
/// lies in `body` (`raw` opened by half the minimum width) and no cell of it is in `blocked`
/// (the knockouts of other nets). Returns the kept spokes' cells and, per pin, each kept
/// spoke's far end.
fn spokes(
    board: &RoutingBoard,
    grid: &Grid,
    net: NetNo,
    layer: LayerNo,
    params: &ZoneParams,
    file: &ZonesFile,
    raw: &Bitmap,
    body: &Bitmap,
    blocked: &Bitmap,
) -> (Bitmap, Vec<(ItemKey, Vec<FloatPoint>)>) {
    todo!()
}
```

### fastroute: `planes/cost.rs`

```rust
//! Row ZR: what a cut of a pour costs the maze.

use crate::board::RoutingBoard;
use crate::ids::{LayerNo, NetNo};
use fr_geom::FloatPoint;

use super::model::PlaneModel;

/// The weights, from `--router.scoring.plane_cut_cost` and `plane_via_cut_cost`. Both 0: no
/// surcharge, and the maze's values are today's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CutCost {
    /// Extra cost of a length over another net's live fill, as a share of the layer's own
    /// trace cost (1.0 doubles it).
    pub per_length: f64,
    /// Extra cost of a via through another net's live fill, per pour layer it crosses.
    pub per_via: f64,
}

/// The surcharge of a maze step from `from` to `to` on `layer` for `net`:
/// `per_length * layer_trace_cost * PlaneModel::foreign_length`. Never negative, so the
/// maze's distance estimate stays a lower bound. The step is the straight line between two
/// door middles, not the final trace: the price is an estimate.
pub fn trace_surcharge(
    board: &RoutingBoard,
    model: &PlaneModel,
    cost: &CutCost,
    layer: LayerNo,
    net: NetNo,
    from: &FloatPoint,
    to: &FloatPoint,
    layer_trace_cost: f64,
) -> f64 {
    todo!()
}

/// The surcharge of a via of `net` at `at` from `first_layer` to `last_layer`: `per_via` for
/// each layer between them where `PlaneModel::foreign_disc` holds for the via's radius plus
/// its clearance.
pub fn via_surcharge(
    board: &RoutingBoard,
    model: &PlaneModel,
    cost: &CutCost,
    net: NetNo,
    at: &FloatPoint,
    first_layer: LayerNo,
    last_layer: LayerNo,
    knockout_radius: f64,
) -> f64 {
    todo!()
}
```

### fastroute: `planes/closing.rs`

```rust
//! Row ZR: the closing stage. After the routing passes, and after the optimizer, it joins
//! what the pours no longer connect.

use crate::board::{ItemKey, RoutingBoard};
use crate::datastructures::StopToken;
use crate::ids::NetNo;
use fr_settings::RouterSettings;

use super::model::{OpenItem, PlaneModel};
use super::zones_file::ZonesFile;

#[derive(Debug, Clone, PartialEq)]
pub struct ClosingRound {
    pub open_before: usize,
    pub routed: usize,
    pub spoke_tracks: usize,
    pub open_after: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Closing {
    pub rounds: Vec<ClosingRound>,
    pub left_open: Vec<OpenItem>,
    /// Why the stage ended early, when it did (a failed build).
    pub stopped: Option<String>,
}

/// The most rounds one call makes.
pub const MAX_ROUNDS: usize = 8;

/// Rounds until nothing is open, a round changes nothing, `MAX_ROUNDS`, or `stop`:
/// 1. `PlaneModel::build` from the board as it is, set on the board.
/// 2. `PlaneModel::open_items`. None: done.
/// 3. [`route_group`] for each stranded group of each modelled net, in the order of
///    `NetGroups::stranded`.
/// 4. [`spoke_track`] for each starved pin.
/// A failed build ends the stage, leaves the board as it was, and is named in `stopped`.
pub fn close_planes(board: &mut RoutingBoard, file: &ZonesFile, settings: &RouterSettings, stop: &StopToken) -> Closing {
    todo!()
}

/// One connection from a stranded group to the main group of `net`: the search of
/// `autoroute::router::route_connection` with the two sets given (start: `group`;
/// destination: `main` and the net's modelled planes), rip-up allowed, the pour net's via
/// cost. Kept only if it adds no clearance violation (`clearance_violation_count` before and
/// after) and the group is in the main group afterwards. Returns whether it was kept.
fn route_group(
    board: &mut RoutingBoard,
    settings: &RouterSettings,
    net: NetNo,
    group: &[ItemKey],
    main: &[ItemKey],
    stop: &StopToken,
) -> bool {
    todo!()
}

/// One short track for a starved pin: from the pin's centre, straight out, to the nearest
/// point on live fill of its zone that lies between two kept spokes. Tried along the free
/// spoke directions first, then the diagonals between them. Kept only if it adds no
/// clearance violation. Returns whether one was kept.
fn spoke_track(board: &mut RoutingBoard, model: &PlaneModel, pin: ItemKey, zone: usize) -> bool {
    todo!()
}
```

### Konnect: `zone_fill.rs`

```rust
//! Row ZR: what Konnect tells fastroute about each pour's fill, and what it reads back
//! (docs/ZONE_ROUTING_DESIGN.md).

use crate::tools::cli::DrcReport;
use anyhow::Result;
use konnect_sexp::SexpNode;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

/// The zones file's name in the call's folder.
pub(crate) const ZONES_FILE: &str = "zones.txt";

/// The model's cell, mm (Z2).
pub(crate) const CELL_MM: f64 = 0.05;

/// How a pour joins the pads of its own net (`connect_pads`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PadConnect {
    Solid,
    Thermal,
    /// Thermal for a pad with a hole, solid for the rest (`thru_hole_only`). Resolved per pad
    /// before the file is written: the engine never sees it.
    ThermalThroughHole,
    None,
}

/// What KiCad does with a fill piece that touches nothing of its net.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum IslandRule {
    Remove,
    Keep,
    RemoveBelow { area_mm2: f64 },
}

/// One pour's fill settings, as its `(zone …)` gives them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct FillSettings {
    pub clearance_mm: f64,
    pub min_width_mm: f64,
    pub connect: PadConnect,
    pub thermal_gap_mm: f64,
    pub thermal_bridge_mm: f64,
    pub priority: u32,
    pub islands: IslandRule,
}

/// Reads one `(zone …)`: `connect_pads` and its `clearance`, `min_thickness`, `priority`,
/// and from `fill`: `thermal_gap`, `thermal_bridge_width`, `island_removal_mode`,
/// `island_area_min`. A value KiCad leaves out is KiCad's default for it.
///
/// `Err(reason)` for a zone the model does not cover (P4): a hatch fill, a corner smoothing
/// (`smoothing` with a radius above 0), a value that is not a finite number of 0 or more, a
/// minimum width of 0.
pub(crate) fn fill_settings(zone: &SexpNode) -> std::result::Result<FillSettings, String> {
    todo!()
}

/// A pour the engine models.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ZoneRow {
    /// Its place among the DSN's `(plane …)` entries.
    pub plane_index: usize,
    pub net: String,
    pub layer: String,
    pub uuid: Option<String>,
    pub settings: FillSettings,
}

/// A pour sent as today, with the reason.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct LegacyZone {
    pub net: String,
    pub layer: String,
    pub uuid: Option<String>,
    pub reason: String,
}

/// A pad of a pour net, with its resolved values (pad, then footprint, then zone).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct PadRow {
    pub component: String,
    pub pin: String,
    /// Never `ThermalThroughHole`: resolved by whether the pad has a hole.
    pub connect: PadConnect,
    pub thermal_gap_mm: f64,
    pub thermal_bridge_mm: f64,
    /// The pad's own spoke angle, else KiCad's default for its shape (45 for a round pad,
    /// 0 for the rest).
    pub angle_deg: f64,
}

/// A rule area that keeps pours out, on one copper layer, in DSN micrometres.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct KeepoutRow {
    pub layer: String,
    pub corners: Vec<(i64, i64)>,
}

/// What one call tells fastroute about the pours.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub(crate) struct ZonePlan {
    pub zones: Vec<ZoneRow>,
    pub legacy: Vec<LegacyZone>,
    pub pads: Vec<PadRow>,
    pub keepouts: Vec<KeepoutRow>,
    pub min_spokes: u32,
    pub edge_clearance_mm: f64,
    pub hole_clearance_mm: f64,
    pub teardrop_mm: f64,
}

/// The call's zone plan.
/// 1. The pours in export order (`specctra::pour_fill_rows` on the saved board), paired with
///    the manifest's `pours` rows. A row the manifest gives `omit` has no plane. The others
///    take plane indices in order; the count must equal the DSN's `(plane …)` count, else
///    every pour is `legacy` with that reason (never a shifted index). The same when
///    `manifest` is `None`.
/// 2. Each pour: [`fill_settings`], or a `legacy` row with its reason.
/// 3. Every pour becomes `legacy`, with the reason, when the board has (P4): a custom rule
///    that can change a zone's clearance or connection (`custom_rules`); a pad or via with
///    unused-layer removal, a zone-layer override or a backdrill; a custom pad with spoke
///    shapes on a pour net.
/// 4. `pads`: for each pad of a modelled pour's net, its resolved connection, gap, bridge
///    and angle, kept only when one of them differs from its zone's values at angle 0.
/// 5. `keepouts`: each rule area with `copperpour not_allowed`, per copper layer.
/// 6. `min_spokes`, the edge clearance and the hole clearance from the project file, else
///    KiCad's defaults. `teardrop_mm`: the largest `max_length` of a pad or via that has
///    teardrops on, else 0.
pub(crate) fn zone_plan(
    board_text: &str,
    manifest: Option<&Value>,
    dsn_text: &str,
    project_text: Option<&str>,
) -> Result<ZonePlan> {
    todo!()
}

/// The zones file of a plan (the format in docs/ZONE_ROUTING_DESIGN.md, "What Konnect sends
/// and reads"), or `None` when the plan models no pour: the call then sends no `--zones`.
/// Numbers are written with the shortest text that reads back to the same value.
pub(crate) fn file_text(plan: &ZonePlan) -> Option<String> {
    todo!()
}

/// What the engine's model said, from the report's `planes` section.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct EnginePlanes {
    /// Pads the model leaves open, as `<component>-<pin>`.
    pub open_pads: BTreeSet<String>,
    pub starved_pads: BTreeSet<String>,
    pub closing_routed: usize,
    pub pieces: usize,
    /// Seconds one redraw of the snapshot took, the longest of the run.
    pub redraw_s: f64,
}

/// `None` when the report has no `planes` section (an older build, or a run with no file).
pub(crate) fn engine_planes(report: &Value) -> Option<EnginePlanes> {
    todo!()
}

/// KiCad's side of the comparison (Z13): the pads of each pour net outside the net's main
/// copper group on a refilled board, as `<component>-<pin>`, by the layout audit's own
/// grouping over the board's saved fills (`tools::layout_quality_planes`).
pub(crate) fn kicad_open_pads(refilled_board_text: &str, pour_nets: &BTreeSet<String>) -> Result<BTreeSet<String>> {
    todo!()
}

/// What the judge records about the pours of one run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct PourMeasures {
    /// KiCad's unconnected items on pour nets after the first refill.
    pub open_before_stitching: usize,
    /// Fill pieces over all pour layers, after the last refill.
    pub pieces: usize,
    /// The largest piece's area as a share of all fill of its zone and layer; the lowest
    /// such share over the zones.
    pub largest_piece_share: f64,
    pub starved_thermals: usize,
    /// Pads the engine's model put in the main group and [`kicad_open_pads`] of the first
    /// refill leaves outside. `None` without an engine section, and when the two sides
    /// cannot be compared: KiCad reports 0 unconnected on the pour nets while
    /// `kicad_open_pads` is not empty (the audit's grouping then disagrees with KiCad's).
    pub false_connected: Option<usize>,
    /// Pads the model left open and KiCad connects.
    pub false_open: Option<usize>,
}

/// The measures of one judged run: `first_board_text` and `first_refill` are the board and
/// its DRC right after the session was applied and refilled; `last_board_text` is the last
/// board the judge wrote.
pub(crate) fn pour_measures(
    first_board_text: &str,
    first_refill: &DrcReport,
    last_board_text: &str,
    pour_nets: &BTreeSet<String>,
    engine: Option<&EnginePlanes>,
) -> Result<PourMeasures> {
    todo!()
}
```

### Konnect: `pour_plan.rs`

```rust
//! Row ZR, stage ZR4: the pours a board should have. Proposed, never written
//! (docs/ZONE_ROUTING_DESIGN.md, "Defining zones").

use crate::tools::design_intent::DesignIntent;
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

/// One zone to add.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ProposedZone {
    pub net: String,
    pub layer: String,
    pub points_mm: Vec<(f64, f64)>,
    pub clearance_mm: f64,
    pub min_width_mm: f64,
    pub priority: u32,
    /// `solid`, `thermal` or `none`, as `add_zone` takes it.
    pub pad_connection: &'static str,
    /// Why this net on this layer, in one sentence.
    pub reason: String,
}

/// A pour that was asked for and is not proposed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Skipped {
    pub net: String,
    pub layer: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct PourPlan {
    pub proposals: Vec<ProposedZone>,
    pub skipped: Vec<Skipped>,
    /// The saved board the plan was made from.
    pub board_sha256: String,
}

/// The nets to pour, in order: the intent's `pours`, else the nets of each rail of 0 V.
/// `Err` when neither gives a net, or a named net is not on the board; the message says
/// which section to fill.
pub(crate) fn pour_nets(intent: &DesignIntent, board_nets: &BTreeSet<String>) -> std::result::Result<Vec<String>, String> {
    todo!()
}

/// `(layer, net)` for a board's copper layers, outermost first in `copper_layers`:
/// - 2 layers: the first net on the bottom, then on the top;
/// - 4 layers: the first net on the first inner layer, the second net (when there is one)
///   on the second inner layer;
/// - any other count: nothing, and the caller reports it.
/// A net the intent gives layers for takes those layers instead.
pub(crate) fn layers_for(copper_layers: &[String], nets: &[String], intent: &DesignIntent) -> Vec<(String, String)> {
    todo!()
}

/// The plan for a saved board.
/// 1. [`pour_nets`], [`layers_for`].
/// 2. A `(layer, net)` that already has a pour of that net is skipped ("already poured").
/// 3. The outline: the board's own (`specctra`'s closed outline), as drawn.
/// 4. Clearance: the larger of the net's class clearance and the fab target's minimum.
///    Minimum width: the larger of 0.25 mm and the fab target's minimum track.
/// 5. Priority 0 for the first net, 1 for the next.
pub(crate) fn plan(board_text: &str, intent: &DesignIntent, project_text: Option<&str>) -> Result<PourPlan> {
    todo!()
}

/// The `add_zone` arguments that add one proposal to `board`.
pub(crate) fn add_zone_arguments(board: &Path, zone: &ProposedZone) -> Value {
    todo!()
}
```

### Konnect: `tools/design_intent.rs` (added item)

```rust
/// Row ZR: a net the owner wants poured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PourIntent {
    pub net: String,
    /// Copper layers by name. Empty: the planner chooses by the stackup.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}
```

## Tests, each guard with a negative control

| Guard | Test | Negative control |
|---|---|---|
| The zones file refuses what it does not know | each refusal of `parse`, by line | the same file without the bad line parses |
| Knockouts round toward less copper | a track at exactly clearance from a pad: the gap cell is clear | the extra cell removed: the test must fail |
| The outline rounds toward less copper | a neck of the outline one cell above the minimum width: pruned | the border cell kept: the neck survives |
| The opening drops a neck below the minimum width | two blocks joined by a 0.2 mm neck, minimum 0.25: two pieces | a 0.3 mm neck: one piece |
| A spoke counts only when its far end is on fill | a pad boxed in on three sides: 1 spoke, starved | the box removed: 4 spokes |
| A spoke narrower than the minimum width is not made | a pad 0.2 mm wide, minimum 0.25: no spoke, the pad open | minimum 0.15: spokes |
| A round pad's spokes are an `X` | a round pad with tracks on its four axes: 4 spokes | angle 0 on the same pad: 0 spokes |
| An exported keepout is a knockout | an unplated hole in a pour: a hole in the fill | the keepout removed: solid fill |
| Only the main group reaches the pour | two pads on two pieces: the smaller side is not `ConnectedToPlane` | the cutting track removed: both are |
| A piece joins what sits on it | two pads on one stranded piece: one group, one connection routed | the piece cut between them: two groups |
| A stitching via does not join a modelled net | a bare fixed via off the fill: not in the main group | the net without a `zone` line: joined, as today |
| A contact is never served stale | a rebuild that strands a pad: the next `connected_set` leaves it out | the generation left out of the cache key: the test must fail |
| A skipped plane cannot shift an index | a DSN whose first plane names an unknown net: `build` refuses the second plane's line | both planes loaded: rows in order |
| A landing is on fill | `landing` inside a knocked-out area gives `None` | the knockout removed: a point |
| The surcharge never lowers a cost | random segments: the surcharge is 0 or more | weights at 0: exactly today's `expansion_value` |
| Off by default | a route without `--zones`: the session equals the pinned build's, byte for byte | with `--zones` and ZR2 on, a board with a cut pour gives another session |
| The closing stage adds no violation | a connection that would break a clearance is dropped | the check off: the violation count rises |
| A manifest and DSN that disagree model nothing | they disagree on the plane count: every pour `legacy`, with the reason | they agree: rows in order |
| An uncovered zone is not modelled | a hatch zone, and a zone with rounded corners, are `legacy` rows | the same zones solid and square: `zones` rows |
| An uncovered board is not modelled | a via with unused-layer removal: every pour `legacy` | the via plain: `zones` rows |
| A pad's own values reach the engine | a round thermal pad: a `pad` line with angle 45 | a rectangular pad with the zone's values: no line |
| The planner writes nothing | the board's bytes before and after `plan_pours` | (the registry's read-only guard tests) |
| A false connection is counted | a recorded report and refilled board with one such pad: 1 | the pad open in both: 0 |
| Sides that cannot be compared are not compared | KiCad at 0 unconnected and the audit's grouping not empty: `None` | both empty: `Some(0)` |

## Verification plan

1. **ZR0 probe, before any engine work.** On the reference board's scratch copy: refill the
   unrouted board, write each fill piece as a plane (the plugin's method), route at effort M,
   and compare "open before stitching" with the outline planes. It tells how much of the gap is
   the outline itself and how much is cuts made while routing.
2. **ZR1 agreement.** Every as-exported run of the bench: false connections must be 0. False
   opens are recorded; above 1 pad in 20 the model is too timid and the reason is found first.
   The redraw's time is recorded per board.
3. **ZR2 and ZR3.** The stage table's criteria, at effort M, with the same seeds as ZR0. ZR3
   also counts the connections the closing stage makes after the optimizer: that number says
   whether the optimizer undoes the cost.
4. **Each engine build** goes through the pin change of `packaging/1-kopper/fastroute.json` in
   its own PR, as x18 did.
5. **Gate and review** as every row: the baseline gate, tests with controls, a review by an
   agent that did not write the code, the gate again.

## Verification of this document

**Independent review** (Claude Opus 5.5, high, 2026-11-22). It read the first draft against
Konnect's, fastroute's and KiCad's source. Every finding below was then checked against the
source by the author before it was accepted, and is fixed in this text.

- **Facts: 22 claims checked.** 12 correct, 8 imprecise or incomplete, 2 wrong.
  - Wrong: the gap table's numbers are Freerouting's, not fastroute's. Now said, and ZR0 is
    named as the first fastroute baseline.
  - Wrong: "the pours-omitted shape wins on the public fixtures". It wins on `pic`; on the
    pours fixture the winner keeps its pours. Now said.
  - Made precise: the second `ConnectedToPlane` site; the power-layer exception; the
    stitching-via stand-in; what the plugin's real fill is for; KiCad's step order with
    iterative refill; the all-islands rule; the starved-thermal rule; what row 3 does not
    show.
- **Design: 4 must-fix.**
  - The spokes were not KiCad's: an `X` on round pads, the width cut to the pad, none below the
    minimum width, per-pad and footprint values. Now the fill model's step 3, the `pad` line
    and Z12.
  - The knockouts missed the DSN's keepouts (unplated holes, copper text) and the hole
    clearance. Now in step 2 and the file.
  - Unused-layer removal and backdrills change what a pour joins. Now P4, for the whole board.
  - The engine's contact and count caches would have served contacts of an older model. Now
    the model's generation is in their keys.
- **Design: 6 should-fix.**
  - The outline rounded toward more copper. Now it loses a cell (Z3).
  - The groups inherited the stitching-via joins. Now left out for a modelled net.
  - A stranded piece's pads were routed one by one, and the closing stage was not aimed at the
    main group. Now a piece joins what sits on it (Z4), `route_group` takes its two sets, and
    `spoke_track` exists.
  - The plane index could shift (the loader can skip a plane and adds areas of its own). Now
    recorded at load (Z6).
  - The optimizer can undo the cost. Now a stated limit with its own measure in ZR3.
  - The parallel pass would have lost model updates. Now a shared snapshot and per-board
    groups (Z11).
- **Design: 3 notes,** taken: teardrops and rounded corners (step 2, P4); the cases where the
  model is short of KiCad, which P2 allows; the surcharge is sound and is an estimate.
- **Gaps: 7,** taken: the fanout stage; a swapped board; what "a false connection" is measured
  against (Z13); the raster's cost, now measured in ZR1 and no longer claimed; the two
  switches together; a DSN with no manifest; the baseline's engine.
- **Not taken:** one line reference. The reviewer gave `:218-232` for the track that counts as
  a spoke; the lines read here are `:230-242`.

**Skeleton compile** (2026-11-22, after the review's fixes). A script read the 9 blocks above
out of this file into two scratch clones: fastroute at `61d6e45` (the six `planes/` files, a
`mod.rs`, and `pub mod planes;`) and Konnect at `517eebf0` (the two new modules, and
`PourIntent` appended to `tools/design_intent.rs`). Then `cargo check -p fr-engine` and
`cargo check -p konnect-core` ran under the lane locks (`chunk.sh`, 13:57).

- **Result:** both exit 0, 0 errors.
- **Warnings:** fastroute 122, of which 121 on the six new files (unused variable, dead code)
  and 1 that was there before (`trace_tightener.rs`). Konnect 49, all on the two new files.
  Nothing calls the skeletons yet.
- **Same text:** all 9 blocks were found verbatim in the compiled files. 47 bodies are
  `todo!()`.
- The scratch clones were removed afterwards. The first draft's blocks compiled the same way
  before the review (13:40).

**Not verified.**

- Nothing was routed, refilled or measured for this document. How well the model matches
  KiCad is unknown until ZR1.
- The "changes to existing items" tables are a list, not compiled code.
- Engine line numbers are of `61d6e45`; KiCad's are of 10.0.1.

## What this design does not do

- **Hatched fills, rounded zone corners, zones inside footprints:** sent as today (P4), or
  refused by the export as today.
- **KiCad's custom rules on zones:** not evaluated. A project that has one keeps today's
  reading for every pour.
- **Boards with unused-layer removal or backdrills:** today's reading for every pour.
- **Local power pours for high-current nets:** the planner proposes whole-layer pours only.
- **Pour current capacity** (IPC-2152): the audit's subject, not the router's.
- **Return paths under critical nets:** the cost keeps pours whole in general. It does not yet
  know which nets are critical.
- **The tuner and the pair stages:** not priced. The tuner already keeps off other nets' pours
  (row FT).
- **Windows and Linux:** as the rest of fastroute, macOS first.
- **Freerouting:** unchanged. It keeps the outline reading and says so.
