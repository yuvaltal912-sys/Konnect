# Zones in the router: research and design (Row ZR)

Status: research, and a design down to function skeletons, 2026-11-22. Nothing is built. Nothing
new was measured: every number below comes from a recorded run and names its record. Designed
in a Claude Fable 5.2 session; reviewed by an agent that did not write it, and corrected from
its findings (see "Verification of this document"). Merged as #102 (main `7bd3f77b`); written
on branch `claude/zone-router-research`, from `fork/main` `517eebf0`. No stage has started: each
waits for the owner's word. Engine facts are from fastroute `61d6e45` (branch `ft-design`: the
pinned x18, `1e94751`, plus row FT). KiCad facts are from the KiCad 10.0.1 source. The section
"The design on x19" (2026-11-22) brings the design in line with fastroute x19 (branch `gr`,
`b3ea917`), the build Row GS pins (merged as #108); a line it made wrong says so where it stands.

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
- **P3. Off by default.** Without `--x.zones` the engine takes today's code paths, byte for byte.
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
   as a spoke. Rounds repeat until nothing is open or a round closes nothing. On x19 the group
   search runs between the second closing and W8, and a third closing runs after W8 only when
   W8 opened something (corrected for x19: see "The design on x19", Z18).

**When the snapshot is redrawn:** before every autoroute pass and every optimizer pass, when
the pipeline swaps the board for a saved one, before each closing round, and before the report.
Between those points a cut made by another net is not yet in it; the closing stage catches the
last ones. Groups are never stale: they are computed from the snapshot and the board's current
items, and cached by the board's own change counter, as the engine's contact cache is. The
parallel pass routes on board copies; each copy shares the snapshot and has its own groups.
On x19 a group-search move routes on a copy that shares the snapshot too, but redraws its own
after each member that changed a modelled layer, and W8 is followed by a redraw (corrected for x19: see "The design on x19", Z15
and Z18).

**Known limit of the cost:** the optimizer compares routes by open connections, vias and
length. It can shorten a track across a pour. The closing stage after it repairs the
connection; whether the cost must also enter the optimizer's comparison is decided on ZR3's
numbers. On x19 the group search has the same blind spot, and in its open count as well; it gets
a move-local redraw and a "never more pour damage" rule (corrected for x19: see "The design on x19", Z15 and Z17).

### What Konnect sends and reads

- **Sends** `--x.zones=<file>` (named `--zones` before the x19 addendum, Z21), like `--pairs`
  and `--tune`. One `zone` line per modelled plane, keyed by the plane's place in the DSN:

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
| **ZR0** Measure | Konnect | The judge records, per run: open on pour nets before stitching, stitching vias, fill pieces, the largest piece's share, starved thermals, the winner's shape. Plus one probe in the bench: planes from KiCad's real fill of the unrouted board | A baseline table per board and shape, with fastroute x19 and the smart-router switches on, from the main that holds Row GS (corrected for x19, Z14). Sessions byte-identical to before |
| **ZR0b** Prune (added 2026-11-23, the owner's rule) | Konnect | After the search, the winner's ground vias that the filled pours make redundant leave the delivered session, each with the wires that end at it; KiCad's refill and DRC, the pours' fill and the judge's measures are the test ("ZR0b" below) | Fewer ground vias on the owner's boards with no gate, key entry, fill or audit measure worse, and the delivered session is the judged board |
| **ZR1** Model | engine, Konnect | `--x.zones`, the model, the report. Routing unchanged | Zero false connections on every run. Sessions byte-identical with and without `--x.zones`, with count time limits (`--no-time-limits`; x19 note: the redraws change timing under wall-clock limits). The redraw's time recorded |
| **ZR2** Truth | engine | The model's groups as connectivity, landing points, the closing stage; on x19 also the group search's move-local redraw and the closing order (Z15, Z18) | Open before stitching is 0 and no stitching via is needed, on at least 9 of 10 judged as-exported runs. G1 and G2 pass shares not below ZR0 |
| **ZR3** Cost | engine | The cut cost, with its weight chosen from a small matrix; on x19 also the group search's "never more pour damage" rule (Z17) | Fewer pieces and a larger main piece than ZR2 at the same gate shares; ground track of the winner near the hand routing's; the search's `pour_worse` and `cut` ends counted |
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
   depends on. ZR0 starts only after Row GS has merged: its baseline is x19's (Z14).
2. **The engine row is built on x19** (rewritten for x19, Z14). FT is no longer pinned alone:
   x19 carries it with S1, F1 and the group search, and Row GS measures them as one build.
   Recommended: build ZR1 to ZR3 on `gr` once Row GS has merged, and measure ZR against the
   pinned x19, each ZR stage with its own pin PR.
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
| `zones_file.rs` | the `--x.zones` file |
| `raster.rs` | the grid, bitmaps, stamping, erode and dilate, labelling |
| `model.rs` | the snapshot, and what the engine asks of it |
| `graph.rs` | groups of a net, the main group, live pieces |
| `cost.rs` | the maze's surcharges |
| `closing.rs` | the closing stage |

fastroute, changes to existing items:

| Where | Change |
|---|---|
| `fr-io/src/structure.rs` | records, per `(plane …)` of the DSN in order, the item it became or that it was skipped; the board keeps the list |
| `board/routing_board.rs` | `planes: Option<Arc<PlaneModel>>`, shared by board copies; `set_planes`, which draws the generation and renews the repository epoch (added for x19, Z16) |
| `board/connectivity.rs` | `connected_set` for a net with a modelled pour returns the model's group, with the pour's item only for the main group; `conduction_area_normal_contacts` and its pad, via and trace side use `PlaneModel::live_contact`; the stitching-via joins are skipped for such a net; the contact cache misses after every redraw because `set_planes` renews the repository epoch (corrected for x19, Z16) |
| `pipeline/stats.rs` | the open-count memo's key gains the model's generation for a modelled net, and `StatsCache`'s key becomes `(board_hash, generation)` (corrected for x19, Z16) |
| `autoroute/maze.rs` | `expand_to_door_section` adds `cost::trace_surcharge`; the layer change adds `cost::via_surcharge`; a target door on a modelled pour is taken only where `PlaneModel::landing` finds a point |
| `autoroute/locator.rs` | a trace end into a modelled pour is placed at the landing point |
| `pipeline/mod.rs`, `pipeline/autorouter.rs`, `pipeline/optimizer.rs` | the redraw points, in the pass loops; `closing::close_planes` after the passes and after the optimizer (also when the optimizer is off and the search runs), and after the last copper stage when its redraw finds work; the group search between the second closing and W8; checkpoints read their open count from a redraw (corrected for x19, Z16, Z18) |
| `pipeline/group_search.rs` | (x19) the move-local redraw in `route_member`, `Value.pour` and its rule, the ends `cut` and `pour_worse` (Z15, Z17) |
| `pipeline/fanout.rs` | a fanout via into a modelled pour takes its landing point too (Konnect's preset has fanout off) |
| `fastroute/src/main.rs` | `--x.zones=FILE`, an `--x.` switch (Z21), dropped with `--parity` and `--no-enhancements` as x19's `exp_for_mode` drops the search; the redraw and closing after the pair stages; `--router.scoring.plane_cut_cost=`, `plane_via_cut_cost=`; `mark_stitching_vias` leaves a modelled net's vias unmarked, at load only (x19: Z19) |
| `fastroute/src/report.rs` | the `planes` section, beside x19's `group_search`, which gains the pour state at its start and end (Z17) |

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
| `fastroute.rs` | `CallFiles.zones: Option<&Path>` beside Row GS's `uncounted`, sent by `pass_arguments` as `--x.zones=` after `--layer-heights` and before `--initial-session`, only for a variant with planes; never by `tune_pass_arguments`; the probe sends it (x19: see "Konnect's command line"). `PassOutcome` and `Held` keep the report's `planes` beside `group_search` |
| `route_judge.rs` | `Measures.pours: Option<PourMeasures>`, filled after the first refill and again after the last |
| `tools/route_fastroute.rs` | builds the zone plan after the pairs plan, writes `zones.txt`, adds `pours` to the result; the engine's `planes` beside `group_search` in `run.json` and `search.winner` (x19) |
| `tools/design_intent.rs` | `DesignIntent.pours: Vec<PourIntent>`, validated like the other sections |
| `tools/pcb_board.rs` | `add_zone` gains `thermal_gap`, `thermal_bridge_width` and `island_removal` |
| `packaging/1-kopper/fastroute.json` | the build that knows `--x.zones`, in its own PR |

## Skeletons

Bodies are `todo!()`. Compiled as described under "Verification of this document".

### fastroute: `planes/zones_file.rs`

```rust
//! Row ZR: the `--x.zones` file. One `zone` line per modelled plane of the DSN, `pad` lines for
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
/// 1. `RoutingBoard::set_planes` from the board as it is (x19 addendum, Z16).
/// 2. `PlaneModel::open_items`. None: done.
/// 3. [`route_group`] for each stranded group of each modelled net, in the order of
///    `NetGroups::stranded`.
/// 4. [`spoke_track`] for each starved pin.
/// A failed build ends the stage, leaves the board as it was, and is named in `stopped`.
/// Ends with a redraw, so the next stage starts from a current snapshot (x19 addendum, Z18).
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
/// and reads"), or `None` when the plan models no pour: the call then sends no `--x.zones`.
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
| A contact is never served stale | a rebuild that strands a pad: the next `connected_set` leaves it out | `set_planes` without renewing the repository epoch: the test must fail (corrected for x19, Z16) |
| A skipped plane cannot shift an index | a DSN whose first plane names an unknown net: `build` refuses the second plane's line | both planes loaded: rows in order |
| A landing is on fill | `landing` inside a knocked-out area gives `None` | the knockout removed: a point |
| The surcharge never lowers a cost | random segments: the surcharge is 0 or more | weights at 0: exactly today's `expansion_value` |
| Off by default | a route without `--x.zones`: the session equals the pinned build's (x19, switches on), byte for byte | with `--x.zones` and ZR2 on, a board with a cut pour gives another session |
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
- Engine line numbers are of `61d6e45`; KiCad's are of 10.0.1. On x19 (`b3ea917`), of the
  files cited here, only `autoroute/router.rs` from line 389 on and `autoroute/maze.rs` from
  line 1178 on moved (`router.rs:466` is `:471`); the addendum's own are of `b3ea917`.

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

## The design on x19 (addendum, 2026-11-22)

Status: a document only, merged as #106 (main `b78371d2`). Nothing was built, compiled, routed
or measured for it. It brings the
design above in line with fastroute **x19**, the build Row GS pins in Konnect (merged as #108): branch
`gr` of the local fastroute clone, commit `b3ea917`, not published. x19 is `61d6e45` (the engine
this design read) plus ten commits, all of them behind switches that are off by default:

| Row | Switch | What it does | Where |
|---|---|---|---|
| S1 | `--x.oct_bound` | The optimizer's length bound in the 45-degree metric | `scoring/bounds.rs`, `optimizer.rs` |
| F1 | `--x.opt_follow_rips=N` | An optimizer candidate routes up to N nets its re-route ripped | `optimizer.rs` (`follow_rips`) |
| GR | `--x.group_search`, `--x.gs_*` | The group search, a stage after the optimizer and before W8 | `pipeline/group_search.rs`, `pipeline/mod.rs:251-258` |

Row FT was already in `61d6e45`. Konnect's side of Row GS (branch `claude/router-group-search`,
`e1c3fdf5`, local, not merged) sends `--x.oct_bound`, `--x.opt_follow_rips=2` and
`--x.group_search` on every routing pass and never on the Tune pass, sends the pour nets as
`--x.gs_uncounted=<file>`, and pins x19. **Until Row GS merges, x19 is planned, not main.** Where
this addendum changed a line above, that line says so and points here.

Decisions Z14 to Z21 below extend the design's own table.

### 1. The base of the engine stages

**Z14. ZR1 to ZR3 are built on `gr` (x19), after Row GS has merged.** Their baseline is the
main that pins x19, with the smart-router switches on as Konnect sends them.

- **Why.** Decision 2 asked for row FT to be measured and pinned alone first, so that each build
  change is measured alone. That can no longer happen: x19 carries FT together with S1, F1 and
  the group search, and Row GS measures them as one build. Building ZR on `ft-design` would mean
  a zone build that Konnect never runs. What stays true of decision 2 is its purpose: ZR is
  measured alone against the pinned x19, and every ZR stage still has its own pin PR.
- **Owner decision 2 is rewritten** to say this (below).

**What each file the modules table changes now has to fit around.** `git diff
ft-design..gr --stat`, read file by file:

| File | x19's change | What the zone change must fit around |
|---|---|---|
| `pipeline/stats.rs` | `incomplete_count` split; `incomplete_counts_by_net` added (F1 and the search read the per-net counts) | Both read the same memo, so the model's generation in a modelled net's memo key covers both; `StatsCache`'s key gains it too (Z16). The per-net counts come from the model for a modelled net once Z4 is in `connected_set` |
| `pipeline/mod.rs` | The group search between the optimizer and W8; `PipelineContext.group_search_never_rip`; `PipelineOutcome.group_search` | The closing stage and the redraw points move (point 3, Z18). `PipelineOutcome` gains the closing's record beside `group_search` |
| `pipeline/optimizer.rs` | S1's bound; F1's `follow_rips` (reads `incomplete_counts_by_net` before and after each round) | F1 has the optimizer's blind spot (below, "The optimizer"). No new change beyond the design's redraw points |
| `autoroute/maze.rs` | `check_ripup` refuses to rip an item of a net in `ctrl.protected_nets` (`:1178-1184`) | The cut surcharges go into the expansion (`:630` on), not into `check_ripup`. Protection stops rips, not cuts: a member may still lay copper across a protected pour net's fill (point 2, `MoveEnd::Cut`) |
| `pipeline/autorouter.rs` | `protected_nets`, `neck_retry`, `pass_panicked`, `tail_nets` (all for the search's members) | The redraw before every pass goes into its pass loop, not into `autoroute_passes_for_optimizing_item` (Z18) |
| `autoroute/router.rs`, `autoroute/control.rs` | `protected_nets` and `neck_retry` passed through to the maze | A landing point (`PlaneModel::landing`) applies inside the search's moves too: the retry is off there, so a landing that needs the neck width fails the member |
| `fastroute/src/main.rs` | `exp_for_mode` drops S1, F1 and the search under `--parity` and `--no-enhancements`; the pair nets go to `group_search_never_rip`; the pipeline's second run after the pairs are released (it was there before x19; x19 keeps its `group_search` report) | `--x.zones` is dropped the same way (P3: today's paths, byte for byte, in those modes). The second pipeline run gets the same closing and redraw points. `mark_stitching_vias` (`:808`) stays a load-time step (Z19) |
| `fastroute/src/report.rs` | The `group_search` object, written before `unrouted` | The `planes` section goes beside it, and `group_search` gains the pour state at its start and end (Z17) |
| `pipeline/group_search.rs` (new in x19, not in the modules table) | The search | Its move redraws the model on its board copy (Z15), its value gains the pour rule (Z17), and a new end `cut` |
| `board/item_list.rs` (new setter, not in the modules table) | `set_fixed_state_keeping_version` | Stays safe under the conditions of Z19 |

Untouched by x19: `fr-io/src/structure.rs`, `board/routing_board.rs`, `board/connectivity.rs`,
`autoroute/locator.rs`, `pipeline/fanout.rs`. Line numbers above them are still `61d6e45`'s,
except `autoroute/router.rs` from line 389 on (by 4 to 6 lines) and `autoroute/maze.rs` from line
1178 on (by 7). The lines this design cites in them did not move, but `router.rs:466` is now
`:471`.

**The fixed-state setter.** `set_fixed_state_keeping_version` (`item_list.rs:222-234`) changes an
item's fixed state and renews neither the item's version nor the repository's epoch. The group
search uses it on a move's board copy only: `freeze` makes the copper of the nets no move may
rip user-fixed, `hold_old_tails` does the same to other nets' tails, and `thaw` gives both back
before the move's value is read (`try_move_impl`, step 4). What keys on those numbers today:

- the open-count memo: per net, the global epoch and each item's key and version
  (`stats.rs:124-140`);
- the contact cache: the repository epoch and the search tree's epoch (`connectivity.rs:127`);
- the parallel pass's commit check: item versions (`parallel_pass.rs:93`, `:122`). The setter's
  own comment says that check never sees a search copy; that was read, not re-proved.
- `StatsCache` keys on `board_hash`, which does hash the fixed state, but the search never
  reads it: a move reads `incomplete_count` and the violations directly.

None of the keyed caches reads the fixed state, so the setter is safe today. One routing step
does read it and is meant to: the cycle removal skips a user-fixed trace through a conduction
area (`trace_ops.rs:343`), so a frozen trace is never removed. With Z4 that walk reads
`live_contact`, which stays free of the fixed state (condition 1).

**Z19. With zones, the setter stays safe, under four conditions.** The design adds the model's
snapshot, the groups cached by the change counter (Z11) and the cut measures. Each of them must:

1. **Read no fixed state.** `PlaneModel::build`, `knockouts`, `spokes`, `attachments`,
   `net_groups`, `live_contact`, `landing`, `open_items` and the `foreign_*` queries depend on
   geometry and nets only. A user-fixed track of another net is a knockout like any other. The
   module doc of `planes/` says so, and a test proves it (below).
2. **Leave the stitching marking at load.** `mark_stitching_vias` reads the fixed state: a via
   counts as stitching when it is `ShoveFixed` or more and touches nothing
   (`basic_board.rs:348-362`). It runs once, at load (`main.rs:808`), and its set is kept by id.
   If a redraw re-ran it, `hold_old_tails` would turn a dangling tail via, made `UserFixed` for
   the move, into a stitching via joined to the rest of its net. So the design's change ("leaves
   a modelled net's vias unmarked") goes into that one load-time call and nowhere else.
3. **Use a process-wide unique number for the model.** Two moves on one thread start from the
   same baseline, and the pour net's items have the same keys and versions in both. A per-board
   counter would give their two redraws the same number, and the memo would hand the second move
   the count of the board before it. Z16 settles this.
4. **Count all copper in the pour measures.** The search's `counted_copper` skips user-fixed
   items, which is right for its length (no move can change locked copper). The pour state of
   Z17 must not copy that filter: it counts what is on the board. It is read after `thaw` in any
   case.

**Z16. A model is set on a board through one method, and every cache that a model changes
keys on its generation.**

```rust
impl RoutingBoard {
    /// Row ZR: puts a snapshot on this board (a board copy keeps its parent's `Arc` until it
    /// calls this). The generation is `board::epoch::next_epoch()`, unique in the process, so
    /// two board copies that redraw on two threads, or one after the other on one thread, never
    /// share one. Then `PlaneModel::build(self, file, generation)`, or `redraw` of the current
    /// model when `dirty` is given, then `self.items.touch()`, so the contact cache (keyed by
    /// the repository epoch) misses, and the new `Arc`.
    pub fn set_planes(&mut self, file: &ZonesFile, dirty: Option<&[DirtyWindow]>) -> Result<(), String> {
        todo!()
    }
}
```

Three caches depend on the model, and each must miss after a redraw:

| Cache | Key today | With a model |
|---|---|---|
| The contact cache (`connectivity.rs:127`) | the repository and tree epochs | `set_planes` renews the repository epoch |
| The open-count memo (`stats.rs:124-140`) | per net: the global epoch and each item's key and version | a modelled net's entry also holds the model's generation; other nets' entries are unchanged, so they still hit |
| `StatsCache` (`stats.rs:20-53`): the full statistics, open count included, read at every pass boundary of the autorouter, the optimizer and multi-start | `board_hash` (`history.rs:31`): the items' content and fixed state, no epoch | the key is `(board_hash, generation)`. Without it, statistics read under a stale snapshot would be served again after the redraw, on the same content |

- **Why not renew the global epoch instead:** it would make every net's memo entry miss, and
  `route_member` counts every net before and after each member. Each redraw would then cost a
  full recount.
- **History:** `BoardHistory` keeps each saved board's score as of the snapshot it had. A
  restore redraws (as designed), and the closing after the passes reads the truth.
- **Checkpoints:** a checkpoint's key holds the open count, and `better_than` compares it first
  (`mod.rs:117-120`), so a key read under a stale snapshot could keep a board with a hidden cut
  in a killed run's session file. With a model, `PipelineContext::checkpoint` takes the count
  from a redraw of the board it saves (on a copy), and each closing writes a checkpoint at the
  stage of the pass it follows.
- **Lines corrected above:** the modules table's rows for `board/connectivity.rs` and
  `pipeline/stats.rs`, and the negative control of "A contact is never served stale".

### 2. The group search and the pours

**What the search reads, verified in `group_search.rs`:**

- **The value** (`value`, `counted_copper`): open connections from `stats::incomplete_count`,
  clearance violations, track length and vias. The length and the vias skip user-fixed copper
  and every net in `uncounted_nets`.
- **The rule** (`Value::improves`): never more open connections, violations or vias. Fewer open
  connections, or as many and fewer violations, win at any cost; otherwise the cost must drop by
  at least `min_gain_mm`. The vias it counts are counted vias only, so a move may add
  vias of a pour net and the rule does not see them.
- **The uncounted nets** (`--x.gs_uncounted`, Konnect's pour nets): out of the cost, but they
  can be ripped and re-routed as members.
- **The group** (`route_member`): a net joins when a member leaves it with more open
  connections than before (`incomplete_counts_by_net` before and after the member).

**The blind spot, a third time.** Two parts of it are plain from the code: the value has no
pour term, and pour nets cost nothing. The open count itself is the third. With Z4 in
`connected_set`, `value` reads the model's count with no change to the search: `incomplete_count` → `NetIncompletes` →
`connected_set`. But the snapshot is the one drawn before the search, and a move routes on a
copy of it. A member's track across a pour is a knockout the snapshot does not hold. The pour
net's own items do not change, so its memo entry (same keys, versions and epoch) is served
again. The move then reads complete while it has stranded a piece. Fixing the value alone does
not help; the snapshot must follow the move.

**Z15. A move redraws the model on its own board copy, after each member that changed copper
on a modelled layer.**

- **Where:** in `route_member`, between `autoroute_passes_for_optimizing_item` and the `after`
  count. Then `opened` holds a pour net that the member cut. That net joins the group as an
  ordinary member: its open connections are its stranded groups (Z4). They are routed to the
  main group by the member's ordinary autoroute, not by `closing::route_group`. It is counted
  against the level's `max_nets`.
- **Once more at the end of the move:** after the tail removal of step 3
  (`nets_that_lost_items`) and before step 4 reads the value. So the move's value, and the
  board a kept move brings with it, have a snapshot equal to what `build` would draw.
- **The copy's model only:** `set_planes` replaces the copy's `Arc`. The pass's baseline and
  the other thread's copy keep theirs.
- **A window, not the whole board:** the windows are measured against the item index the
  model itself was drawn from, not against the member's start. So the target's rip
  (`rip_target`, before the first member) and the tail removal are in them too. The redraw
  covers the boxes of the copper added or removed on modelled layers since that index, grown by
  the largest knockout and the minimum width. It then labels each touched zone again in full,
  because pieces are global. When nothing changed on a modelled layer since the model was drawn,
  there is no redraw.
- **Cost:** each redraw labels every touched zone again, a full `Labels` per zone (about 16 MB
  at 100 × 100 mm and 0.05 mm cells). ZR2 records the redraw count and time per move.

```rust
/// Row ZR: where a board's copper changed on a modelled layer.
#[derive(Debug, Clone, PartialEq)]
pub struct DirtyWindow {
    pub layer: LayerNo,
    pub bounds: IntBox,
}

/// The windows of the traces, vias and pins on modelled layers whose id is new on `board`, or
/// whose id is gone or version differs from `model.drawn_from` (item id -> (key, version) of
/// the board the model was drawn from, as `parallel_pass` takes it), each grown by `grow` board
/// units. Empty: no redraw.
pub fn dirty_windows(board: &RoutingBoard, model: &PlaneModel, grow: f64) -> Vec<DirtyWindow> {
    todo!()
}

// `PlaneModel` gains `drawn_from: HashMap<i32, (ItemKey, u64)>`, set by `build` and `redraw`,
// and, per zone, the piece count before the island rule (for `PourState::pieces`).

impl PlaneModel {
    /// The snapshot `build` would draw from `board` now, computed from `self` again only inside
    /// `dirty`: knockouts and spokes of the cells in a window, the opening in each window grown
    /// by the minimum width, then a full `raster::label` and the island rule of every zone a
    /// window touches. With `FASTROUTE_VERIFY_CACHES=1` it is compared with a full `build`
    /// and must be equal.
    pub fn redraw(&self, board: &RoutingBoard, file: &ZonesFile, dirty: &[DirtyWindow], generation: u64) -> Result<PlaneModel, String> {
        todo!()
    }
}
```

A new end of a move is needed. Protection stops rips (`check_ripup`), not cuts, so a later
member may cut a pour net that was routed earlier in the move. Today the search would report
that as `Reopened`, whose count "must be 0" as a guard. To keep that guard meaningful, a cut gets
an end of its own. Like `Reopened`, it ends the move, which is given up; the routed net is not
queued again. The check for it runs just before the check for `Reopened` (`try_move_impl`,
step 3):

```rust
pub enum MoveEnd {
    // … as today …
    /// Row ZR: a member cut the fill of a modelled net that was already routed in this move.
    /// Expected with pours, unlike `Reopened`, which stays a guard at 0.
    Cut(NetNo),
    /// Row ZR: complete and cheaper, refused because the pour state got worse (Z17).
    PourWorse,
}
```

**Z17. The search's cut cost is a rule, "never more pour damage", not a term of the value.**

```rust
/// Row ZR: what the pours of a board look like to the search, from its current snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PourState {
    /// Fill pieces over every modelled zone after the opening and before the island rule, so a
    /// cut that splits off a piece counts even when KiCad then removes it as an island.
    pub pieces: u32,
    /// Thermal pins of a main group with at least one kept spoke and fewer than `min_spokes`.
    pub starved: u32,
}

impl PlaneModel {
    pub fn pour_state(&self) -> PourState {
        todo!()
    }
}
```

- **`Value` gains `pour: PourState`** (all zero on a board with no model). `Value::improves`
  gains one more refusal beside its "never more vias": more `pieces`, or more `starved`, than
  the incumbent. A move refused for that alone ends `PourWorse`, so the report counts what the
  rule cost.
- **The open count** is the model's through Z4 and Z15, so a stranded group is already "more
  open" and refused. `starved` is a separate count because a starved pin is in its main group
  and so is not open.
- **Why a rule and not a weight:**
  1. The judge's key has no pour term until ZR5 (Z10). A weight would turn pour damage into mm
     at a rate nothing checks. "Never more vias" exists for the same reason.
  2. The routes a move builds are already priced by ZR3's surcharge in the maze. The search
     needs a guard that it never accepts a worse pour, not a second price.
  3. Both counts are whole numbers and can only stay or fall. The search's proof of its end
     stands as it is: the pair (open, violations) never rises, and while it stands still a kept
     move lowers the cost by `min_gain_mm`.
- **What it does not catch:** a neck that narrows without splitting, or fill area lost without a
  new piece. That is the job of the maze's per-length surcharge. ZR3 records the largest piece's
  share at the search's start and end, so the numbers show whether the rule is enough.
- **Not in ZR3:** a move that heals a pour (fewer pieces at a higher cost) is not kept. Taking it
  would put pieces into the order of the value and change the end proof. It is decided on ZR3's
  numbers, with ZR5.

**Z20. The uncounted set keeps its content and its meaning for the cost.** It stays the judge's
`pour_nets`, from the same source (Row GS's G3), less the names `uncounted_file_text` leaves out
on the safe side. A pour net's length and vias stay out of the cost while the judge's key
leaves them out, which is until ZR5. What changes is that a modelled pour
net is no longer free in what matters to it: the model's open count (Z15) and the pour rule
(Z17) read its copper and the copper that crosses it.

Still free, as today:

- its length, its vias and its corners;
- every pour the model does not cover: a `legacy` row (P4), and the pours-omitted shape, which
  sends no zones file.

When ZR5 adds pour terms to the key, the uncounted set follows the key and shrinks.

**The optimizer.** F1's `follow_rips` and the optimizer's own comparison read the same per-net
counts, from the snapshot drawn before the optimizer pass. A candidate that cuts a pour reads
complete until the next pass's redraw, as "Known limit of the cost" says. Nothing changes here:
the closing after the optimizer repairs it before the search starts (point 3), and whether the
candidates need Z15's redraw too is decided on ZR3's numbers. It would be the same function.

### 3. Where the closing stage and the redraws go

**Z18. One pipeline run on x19 with zones, in order:**

| Step | Snapshot | Closing |
|---|---|---|
| Load | `set_planes` after `mark_stitching_vias` (Z19) | |
| Routing passes | redrawn before each pass, as designed | after the last pass (closing 1) |
| Optimizer | redrawn before each pass, as designed | after it (closing 2) |
| Group search | Starts from the board's snapshot. Each move redraws its own copy (Z15). A kept move is applied by a board swap and brings its copy's snapshot, so a swap needs no redraw. A retried move (the pass's step 4) starts from the board just applied | none |
| W8 (parallel vias) | | |
| The pair stages, on a pass with `--pairs` (`couple_pairs`, `close_pair_nets`, `match_pair_lengths`, `main.rs:682-704`, after the pipeline) | | |
| After the last copper stage | redrawn | closing 3, only when the redraw finds an open or starved item (`after_last_stage` in the report) |
| Report | redrawn | |

- **The closing always ends with a redraw**, so the stage after it starts from a current
  snapshot. That is a postcondition of `close_planes`.
- **The redraw "before every pass" is hooked into the pass loops** of
  `BatchAutorouter::run_batch_loop` (`pipeline/autorouter.rs`) and
  `BatchOptimizer::run_batch_loop`. It is not in `autoroute_passes_for_optimizing_item`, which
  optimizer candidates and the search's members call: they route on the snapshot they were given.
- **Closing 2 runs whenever the search runs,** with the optimizer off too: the search runs
  without the optimizer (`mod.rs:251-253`).
- **Why closing 2 comes before the search, not after it.** The search keeps only moves that open
  nothing in the model's count and make the pours no worse (Z15, Z17), so it leaves no work for
  a closing. And it never tries a connection that was open when it started (`skip_items`): a
  group the optimizer stranded and nobody closed would stay stranded for the whole search.
- **Why W8 stays after the search and the closing.** W8 sizes the vias of the final layer
  changes, and the closing adds layer changes. But W8 adds vias of supply nets, each one a
  knockout in another net's pour, and the pair stages lay copper after the pipeline. So the last
  redraw is a check: on most boards it finds nothing, and closing 3 does not run. When it does,
  its vias get no parallel vias from W8.
- **The second pipeline run** (`main.rs`: after the pairs are released, with connections still
  open) runs the same steps again. The report keeps the last run's closing and `planes`, as it
  keeps the last run's `group_search`.

**Can a move's board copy share the snapshot, as the parallel pass's copies do?**

- **To route its members, yes.** The baseline's `Arc` is read only, as in the parallel pass.
- **To judge the move, no.** Its value must come from its own redraw (Z15), with its own epoch
  (Z16). Each thread then holds at most one extra model, plus the one of the best kept board so
  far.
- **Determinism holds:** the redraw reads no clock and draws no random number, so the search's
  "the same result on 1, 2 and 4 threads" still holds with `--x.zones`, and gets a test.

### ZR0 on x19

- **The baseline is x19's.** ZR0 is measured from the main that holds Row GS, with x19 and the
  smart-router switches on as Konnect sends them, at effort M. A baseline on x18 would describe an
  engine Konnect no longer runs.
- **Where its pour measures sit.**
  - `Measures.pours` goes in `route_judge.rs`, so it is in every judged run's
    `run.json` under `judged.measures` and in `search.winner.measures`.
  - Row GS puts the search's summary beside them: `group_search` at the top of `run.json` and
    `search.winner.group_search` (`tools/route_fastroute.rs`, `reduce_run_folders` and
    `winner_json`). ZR0 reads the two side by side. A run whose search kept moves can then be
    compared on its pours with one whose search kept nothing, which is the first measure of the
    blind spot of point 2.
  - From ZR1, the engine's `planes` section takes the same path as `group_search`: read where
    the report's `group_search` is read (`fastroute.rs`, `PassOutcome` and `Held`), and written
    beside it in `run.json` and in `search.winner`, as `engine_planes`. `run.json` already has
    a `planes` key, the variant's plane nets (`reduce_run_folders`).
  - Row GS's `group_search_summary` keeps `ended_by`, the pass count, `kept_moves`,
    `largest_group`, and `start` and `end` (length, vias, cost). ZR3 extends it with the ends
    summed over the passes (`cut` and `pour_worse` among them) and the pour state at the start
    and the end. Like Row GS's, it is kept for judged runs only.

### Konnect's command line

- **Routing passes.** With Row GS the order is `CANDIDATE_SWITCHES`, `SEARCH_SWITCHES`,
  `--x.par_via_amps=`, `--x.gs_uncounted=`, `--pairs=`, `--layer-heights=`,
  `--initial-session=`. `CallFiles.zones` sits beside `CallFiles.uncounted`, and `--x.zones=` goes
  after `--layer-heights=` and before `--initial-session=`, as the design had it, for a variant
  with planes only.
- **The Tune pass** gets neither search switch nor `--x.gs_uncounted` (Row GS's G1). It gets no
  `--x.zones` either: its autorouter and optimizer are off and it runs no closing, so a model would
  only cost a redraw. Row FT keeps the tuner off other nets' pours, and the judge's refill checks
  the result. "What this design does not do" already leaves the tuner unpriced.
- **The probe.** ZR1's pin PR adds `--x.zones=<scratch>/probe-zones.txt` (a valid file with no
  `zone` line) before the sentinel, so a build without it is refused before any run, as Row GS's
  probe does for its switches.

**Z21. The switch is `--x.zones=<file>`, not `--zones=`.** fastroute checks only `--x.`
arguments for names it does not know (`main.rs:144`, `exp::parse_arg`). Any other unknown
argument goes to the settings parser, which only warns (`fr-settings/src/cli.rs`). A build
without `--zones` would pass the probe. As an `--x.` switch, the file is read while the
arguments are parsed, as `--x.gs_uncounted` is, and `exp_for_mode` drops it with `--parity` and
`--no-enhancements`. Every `--zones` above now reads `--x.zones`, the two skeleton doc comments
included.

### The corner limit Row GS records

Row GS records that the search re-routes pour nets without looking at corners, while the judge
ranks acute corners and right angles on pour copper too. Verified: `rank_measures` puts acute
corners at position 6 of the key (from 0), above signal length, and right angles at 9. Both are
the audit's metrics `5.3.*` over all copper. `Value` has no corner term.

ZR neither explains it nor fixes it. The limit comes from the pour nets being uncounted, and
Z20 keeps them uncounted. With ZR2, the closing adds pour-net tracks where today a pad counts as
connected by its centre, so the search has more of that copper to move. ZR only makes that new
copper short and straight: a landing point, and a spoke track straight out from its pin.

The fix belongs to Row GS: a "never more corners" rule in the search's value, over the move's
nets, uncounted ones included, counted as the audit counts them. It has the shape of Z17 and is
not part of this row.

### Changes this forces in the text above

- **The status lines** say that this section exists.
- **"Closing", "When the snapshot is redrawn", "Known limit of the cost":** a sentence each,
  pointing here (Z15, Z18).
- **Stages:**
  - ZR0's baseline is x19's (Z14).
  - ZR2 adds the move-local redraw (Z15) and the closing order (Z18).
  - ZR3 adds the pour rule (Z17); its done criterion adds the counts of `pour_worse` and `cut`
    ends.
- **The switch** is `--x.zones` everywhere (Z21).
- **Owner decision 2** is rewritten (Z14). Decision 1 now waits for Row GS's merge, because
  ZR0's baseline is x19's. The other three decisions are unchanged.
- **Modules:**
  - Rows changed: `board/connectivity.rs` and `pipeline/stats.rs` (Z16), `pipeline/mod.rs`,
    `autorouter.rs` and `optimizer.rs` (Z16, Z18), `fastroute/src/main.rs` (`--x.zones`
    dropped with the other enhancements), `fastroute/src/report.rs`, Konnect's `fastroute.rs`
    and `tools/route_fastroute.rs`.
  - Rows added: `board/routing_board.rs` gains `set_planes`, and `pipeline/group_search.rs` is
    new.
- **Tests:** the rows below are added, and the control of "A contact is never served stale" is
  `set_planes` without renewing the repository epoch.

| Guard | Test | Negative control |
|---|---|---|
| A move that strands a piece reads open | a member's track across a modelled pour strands a pad: the move is not kept, and the pour net is in `opened` | no redraw in `route_member`: the move is kept, the test fails |
| Two moves of one baseline never share a model | two moves on one thread, one of them cutting the pour: two different counts | the generation left out of the memo key: the cutting move reads the baseline's count |
| Statistics follow the model | a redraw that strands a pad on an unchanged board: the next `StatsCache::score` counts it | the generation left out of the `StatsCache` key: the old count |
| The move's last redraw sees its tail removal | a move whose tail removal frees fill: the kept board's model equals a full `build` | the redraw after step 3 left out: they differ |
| The pour rule refuses more damage | a cheaper move that splits a piece and opens nothing ends `pour_worse` | the rule off: kept |
| A cut is not a reopening | a later member cuts an earlier pour member's fill: `cut`, and `reopened` stays 0 | the end folded into `Reopened`: its count is 1 |
| Frozen copper does not change the model | `freeze` then `thaw` around `set_planes`: the same fill and groups | a knockout that skips user-fixed items: the fill differs |
| The stitching marking stays at load | a held tail via with no contact: never joined to the net's other vias | the marking re-run at a redraw: it is joined |
| A windowed redraw equals a full one | random moves on a fixture: `redraw` equals `build`, cell for cell | a window not grown by the minimum width: they differ |
| The search stays deterministic with zones | 1, 2 and 4 threads with `--x.zones`: the same session | (as the search's own test) |
| `--x.zones` is an enhancement | `--parity` with `--x.zones`: today's session, and the drop logged | without `--parity`: the zones file is read |
| The Tune pass gets no zones | `tune_pass_arguments` holds no `--x.zones=` | the argument added: the test fails |

### Verification of this addendum

- **Read, read-only:**
  - fastroute: `gr` `b3ea917`, against `ft-design` `61d6e45`, the merge base. `git diff
    ft-design..gr --stat` lists 30 files, of which 14 are tests and test data. Read in full:
    `pipeline/group_search.rs`. Read in the parts above: `pipeline/mod.rs`, `optimizer.rs`,
    `autorouter.rs`, `stats.rs`, `autoroute/maze.rs`, `router.rs`, `control.rs`,
    `board/item_list.rs`, `epoch.rs`, `connectivity.rs`, `basic_board.rs`, `parallel_pass.rs`,
    `exp.rs`, `fastroute/src/main.rs`, `report.rs`, and `docs/IMPROVEMENTS.md`'s rows S1, F1
    and GR.
  - Konnect: `claude/router-group-search` `e1c3fdf5` (`fastroute.rs`,
    `tools/route_fastroute.rs`, and the docs it changes) and its design file
    (`target/creation-research/smart-router-research/skeleton/konnect_uptake.md`), plus
    `route_judge.rs` on main `5a8273ea`.
- **Inferred, not checked by a run:**
  - the cost of a windowed redraw per member;
  - how many moves the pour rule refuses;
  - that W8's redraw rarely finds work;
  - that the parallel pass never sees a search copy (the setter's own comment says so).
- **Independent review** (Claude Opus 5.5, high, 2026-11-22), read-only against the same
  sources, of the first draft. It confirmed the cited lines, that Z17 keeps the search's end
  proof and that the setter is safe, and found 3 must-fix items, 8 should-fix and 13 notes. Each was checked against the code before it was taken; all
  are fixed in this text:
  - **Must-fix:** `StatsCache` keys on `board_hash`, which no epoch reaches (now in Z16); the
    redraw windows missed the target's rip and the last tail removal (now measured from the
    model's own index, with a redraw at the end of the move, Z15); a non-`--x.` switch cannot be
    probed (Z21).
  - **Should-fix:** the pair stages after the pipeline; the run summary and the `planes` key in
    `run.json`; the memo cost of renewing the global epoch (Z16 now keys modelled nets only);
    what `Cut` does; a negative control's wording; checkpoints; where the redraw before a pass
    is hooked; closing 2 with the optimizer off.
- **The two skeleton doc comments** that named `--zones` (`zones_file.rs`, `zone_fill.rs`) and
  `close_planes`'s step 1 were changed after the design's compile record; that record is of the
  text before.
- **Not done:** the skeletons in this section were not compiled (no build was allowed while Row
  GS's measurement held the machine). The next stage's PR compiles them, as the design's own
  were.

## ZR0 as built (2026-11-22)

Stage ZR0's measures are built in `crates/konnect-core/src/zone_fill.rs` and filled by the
judge (`route_judge.rs`, `judge_one`). They differ from the `zone_fill.rs` skeleton above as
follows, each on purpose:

- **`pour_measures(first_refill, last_board_text, last_refill, pour_nets)`.** It takes no
  first board text and no engine section: ZR0 has no engine model, and the first board's
  folder is removed once a stitching round writes the next one, so what ZR0 needs of the
  first refill (its open count) is taken from its report.
- **Fields added:** `open_after_stitching` (the same count on the last board) and
  `unfilled` (pour zone layers with no saved fill, which would otherwise read as whole).
- **`largest_piece_share` is `Option<f64>`:** `None` when no pour layer has fill.
- **A piece is one `filled_polygon`.** KiCad 10 saves each piece of a fill as one fractured
  ring (`pcb_io_kicad_sexpr.cpp`), so its shoelace area is the piece less its holes. No raster.
- **A failed pour measure is recorded, not raised** (`Measures.pours_error`): the measures
  feed no gate and no rank key (Z10), so they must not fail a run or reject a Tune
  candidate.
- **`kicad_open_pads`, `false_connected` and `false_open` wait for ZR1.**

### The x19 baseline (2026-11-23)

One `route_specctra_dsn` call per board at effort M (240 s), fastroute x19, the branch's release
build, each in its own routing hold. Five boards: two of the owner's (dash, B5) and the public
fixtures `pic`, `ecc` and `pours`. Then the probe: each manifest's pour planes replaced by
KiCad's real fill of the unrouted board, and one more call on it. Counts are per judged run;
"open" is KiCad's unconnected items on the pour nets after the first refill.

| Board | Pours as planes (as exported): runs, gates passed, open (median, max) | Pours omitted: runs, gates passed, open |
|---|---|---|
| dash | 7, 0, 25 and 28 | 5, 5, 0 |
| B5 | 28, 0, 11 and 13 | 26, 26, 0 |
| `pic` | 92, 0, 2 and 5 | 91, 27, 0 |
| `ecc` | 710, 710, 0 and 0 | 709, 709, 0 |
| `pours` | 707, 435, 0 and 1 (575 runs at 0) | 707, 602, 0 |

- **The stitching rounds never ran** (Z9). On every as-exported run that left a pour net open,
  the stitching planner refused: "unsupported local zone_connect override" on dash, B5 and `pic`
  (the research's change 6, outside this row), and "saved project required for planning rules"
  on `pours`. So `open_after_stitching` equals `open_before_stitching` in all 5,804 judged runs,
  and the count Z9 meant to measure how much the engine misses is the open count itself.
- **The five winners are pours omitted**, the same seeds as Row GS's x19 calls, and the judged
  counts are within Row GS's spread (dash 12 and 12, B5 54 and 46, `pic` 183 and 185, `ecc` 1,419
  and 1,400, `pours` 1,414 and 1,428): the measures cost no measurable judge time. No run
  recorded a `pours_error`.
- **The fill on the winners:** dash 32 pieces with a lowest largest-piece share of 0.71; B5 13
  and 0.95; `pic` 3 and 0.92; `ecc` and `pours` whole. No winner has a starved thermal or an
  unfilled pour layer.
- **The probe.** The unrouted fill is one piece per plane on all five boards, so the probe
  changes the planes' shape only (clearance voids, the board edge), not their count. On B5 the
  as-exported runs went from 0 of 28 passing (open 11) to 4 of 4 passing (open 0). On `pic` they
  stayed at open 2 (0 of 5 passing); `ecc` and `pours` did not change; on dash no as-exported
  run was judged in the budget. The fill-shaped planes slow the engine: 11 runs judged on B5
  instead of 54, 16 on `pic` instead of 183, 2 on dash instead of 12. One board, four runs: it
  points ZR2's truth at the planes' shape, and it is not yet evidence.

## ZR0b: the winner's redundant ground vias are pruned (2026-11-23)

The owner, 2026-11-23: "צריך להסיר ויות שמטרתן הייתה לחבר פד לאדמה אחרי שהמילוי בולע את
הניתוב בין הפד לויה". With the pours left out, the router draws ground as a tree of tracks and
vias. KiCad's fill then connects most of those pads by itself, and a via that tied a pad to the
other layer is left with nothing to do. Until the engine models the fill (ZR1 to ZR3),
`route_specctra_dsn` removes such vias from the session it delivers. Konnect only: no engine
file, no new pin. Code: `crates/konnect-core/src/pour_prune.rs`; the judge's steps it reuses
are in `route_judge.rs` (`routed_scratch`, `judge_light`, `judge_session`).

### What it does

After the search, on the winner only:

1. Every via of a ground pour net is tried, first together with the wires that end at it,
   then alone, in the order of the session text.
2. A candidate session is the winner's text with those blocks cut out. It is read back by the
   importer's own parser, written to a scratch copy of the board and refilled by KiCad, so the
   board that is checked is the board the delivered session gives.
3. The candidate replaces the state accepted so far only when every clause of P5 below holds.
4. The pruned session is then judged in full, like any run, and replaces the winner only when
   it passes every gate with no stitching round and no entry of its rank key is higher than
   the router's session's. Otherwise the router's session is delivered as before.
5. `search.pour_prune` says what happened: the outcome, the vias and wires removed, and for
   each via that stayed the clauses that kept it.

### Decisions (the code cites them as P1 to P14)

| # | Decision | Reason |
|---|---|---|
| P1 | Once a call, on the winner, after the search and before the result is written | A prune costs about a minute of KiCad and audit runs; on every judged run it would take the judge lanes for most of the budget |
| P2 | Scope: the vias of ground pour nets, each with the wires that end at it. No other wire goes | The owner's words are about vias and the routing between the pad and the via. Another ground wire is other copper: where the fill does not cover it, removing it is an electrical choice he did not ask for |
| P3 | Only pour nets the audit of the router's session lists as ground (`net_roles.ground`) | On a power pour, vias in parallel carry current, which no check here sees. The audit's role puts the design intent and the netclass before the name |
| P4 | The unit is a `(via …)` or `(wire …)` block of the session text; blocks the importer would apply as one item are one unit | The session is what the call delivers, and every candidate goes through the importer |
| P5 | A candidate is kept only when ALL hold. Against the state accepted so far: (a) KiCad's unconnected count is not higher; (b) no KiCad rule's violation count is higher; (c) no entry of the judge's rank key is higher (seconds left out); (e) the audit's unstitched layer changes (3.2) are not more; (g) no audit check that scored is unscored. Against the router's session, with a tolerance: (d) no pour zone layer's filled area is lower; (f) the decoupling loops (4.1) are not fewer measured, the worst and the median not longer, the mean area not larger, and no listed pin's loop longer or larger | (a) is his rule. (b): a pad whose track was its second spoke becomes `starved_thermal`, a stub becomes `track_dangling`. (c): one standard with the search. (d): a fill island whose only link is the via would vanish with no DRC item. (e) and (f): the two checklist items a DRC-only prune made worse on both measured boards. (g): a check that stops scoring leaves the mean and can raise it. The clauses with a tolerance compare with the router's session, so a tolerance cannot add up over many vias |
| P6 | Each via with the wires that end at it; if refused, alone. A via whose alone test fails (a) or (d) is final; the others are tried again in a later sweep while a sweep removed something, at most 3 | A via removed alone can leave a stub, which (b) refuses; with its wires it does not. Connectivity and fill islands only fall as copper goes |
| P7 | The audit runs only after (a) and (b) pass | Most refusals are the DRC's |
| P8 | Only a winner that passed every gate and whose judge wrote no stitching round is pruned | Then the copper and the fill the loop checks are the judged board's. The judge's stitching (Z9) is never touched |
| P9 | The pruned session is judged in full (`judge_one`) and replaces the winner only when every gate passes, no stitching round was written and no key entry is higher, full judge against full judge | The full judge is the last word for every run. The loop's own gates and key are never reported: its scratch project has forced severities (P11) |
| P10 | A candidate text the importer refuses is a refused candidate. Any other error ends the loop with the state accepted so far, which P9 then judges | Every accepted state was checked whole. When a candidate is refused the router's session is parsed again; if that fails too, the board or the manifest changed, and that is an error |
| P11 | The prune's scratch project reports `track_dangling`, `via_dangling`, `isolated_copper` and `starved_thermal` at least as warnings | (b) can refuse only what KiCad reports |
| P12 | No schema argument. An internal switch: on for `route_specctra_dsn`, off for the placement tools' routing calls | The placement tools deliver placements, not sessions; a prune there costs minutes a candidate and would change what their key ranks |
| P13 | The time is reported (`pour_prune.seconds`), outside the effort's budget. Sequential checks; a cap of 400 checks | The budget is the draw's (D6). The loop is finite: at most 2 checks a via a sweep |
| P14 | The router's session and its judged board stay in the search folder; the pruned session is `pruned.ses` in the winner's run folder and its judged board is under `judge/run-<i>-pruned` | The record shows before and after |

Constants: `MAX_CHECKS` 400, `VIA_SWEEPS` 3, `FILL_TOLERANCE_MM2` 0.01, `LOOP_TOLERANCE_MM`
0.05, `LOOP_AREA_TOLERANCE_MM2` 0.1.

### Why the test is wider than DRC (measured before the build)

On scratch copies of two of the owner's judged x19 winners, with KiCad's unconnected count and
violation counts as the only test, 10 of 18 and 14 of 21 ground vias went. The same boards then
scored lower in Konnect's own audit: one more critical-net layer change had no ground via near
it (3.2) on both, the worst decoupling loop on one grew from 19 to 47 mm (4.1), two right
angles appeared where a T junction lost a branch, copper quality fell 4.0 and 0.5 points, and
two small fill pieces of one board vanished with no DRC item. Removing every other ground wire
as well took about 90 more KiCad checks on the larger board, and a third of the wires it
removed lay partly outside the fill. Hence P2 and P5.

### As built

- The design was reviewed twice before the build and the diff once after it, each time by an
  agent that did not write it. The review of the diff found no must-fix item.
- `judge_one` is split into `routed_scratch` and `audit`, with no change of behaviour: its
  kicad-cli calls, folders, error wording and `Judged` are as before. One difference on an
  error path: its settings copy is now made before the importer step, so a session the
  importer refuses makes and removes that copy.
- `prune_winner` never fails the call: its steps run in a task of their own, and a panic or an
  error leaves the router's session as the winner with the reason in the record.
- After the keys are taken again, a winner that would no longer rank first is not replaced.
  P9 makes that impossible today; the guard enforces it.
- `checks` and `audits` count the full judge of the pruned session too.

### Measured (2026-11-23)

The branch's release build, fastroute x19, effort M, one call a board, each in its own routing
hold. Every comparison is inside one call: the router's own winning session against the
delivered one.

| | dash | B5 | `pic`, `ecc`, `pours` |
|---|---|---|---|
| Outcome | pruned | pruned | skipped: the winner has no via on a ground pour net |
| Ground vias | 18 to 12 | 21 to 10 | 0 |
| Wires that went with them | 11 | 22 | |
| All vias | 86 to 80 | 53 to 42 | |
| KiCad checks, audits, seconds | 41, 25, 85 | 41, 27, 77 | 1, 1, about 1 |

- **Nothing got worse.** The winner's rank key is the same before and after on both boards.
  Every net but the ground net is byte-equal in the session. An independent before-and-after
  reading of the two judged boards found every pour zone layer's fill area, the 3.2 count,
  every 4.1 loop, the corners and copper quality unchanged.
- **The delivered session is the judged board.** It is `pruned.ses` byte for byte, and KiCad's
  own refill and DRC on a fresh copy of its judged board gives 0 unconnected and the judged
  violation counts.
- **Why the others stayed** (a via counts under every clause that refused it): a pad would be
  left unconnected, 8 and 6; a decoupling loop would grow, 4 and 3; a layer change would lose
  its ground via, 1 and 2.
- **The same every time:** B5's repeat delivers the same session bytes.
- **Cost:** the prune adds about 80 s to a call of about 240 s on these boards.

### Limits

- The other ground wires stay (P2). They are harmless copper inside the pour; removing them is
  a follow-up on the owner's word, with the numbers above.
- A via whose stub is split into several wire blocks that meet at a bare point cannot go with
  its whole stub: the group is the via and the wires that end at it. The record counts vias
  kept by a with-wires `rule:track_dangling`; on the two measured boards there was none.
- A winner the judge stitched, and a power pour, are left alone (P8, P3).
- A call that is dropped mid-prune leaves the prune's scratch files inside its own search
  folder.
- The public fixtures' winners hold no ground via, so the public evidence of a real prune is
  two ignored tests that run the real kicad-cli on the public pours fixture
  (`route_judge::tests::prunes_`): with one recorded ground via the prune removes nothing
  (the via is the back pour's only link, and KiCad's `isolated_copper` count would rise); with
  two it removes one, with its stub, and keeps the other for the same reason. The numbers
  above are from the owner's boards, as counts.

## ZR1 as built (2026-11-23)

Stage ZR1 is built in two parts. The engine part is fastroute build **x20**: branch `zr1-fill-model`, commit `c13a28f`, which is x19's `gr` `b3ea917` plus the `planes/` model, the plane record, `--x.zones` and the report's `planes` object. Konnect pins it in `packaging/1-kopper/fastroute.json`. The Konnect part is `zone_fill.rs`'s zone plan, the zones file, and the judge's pad-by-pad comparison. ZR1 changes no routing, no gate and no rank key.

### The engine

- **The model is drawn once,** on the board as routed, after the last copper stage of the last pipeline run, and only with `--x.zones`. Nothing writes to the board. The report's `planes` object lists every pin of a modelled net as connected or open, and the starved thermal pins.
- **Safe-side choices** beyond the text above, each so the model never calls a pin connected that KiCad leaves open:
  1. **A strict majority.** A pin counts as connected only when its net's main group holds more than half the net's pins; otherwise every pin of that net is open. Being in the main group is not monotone in copper, and only a majority is sure to be KiCad's main group too. Konnect's side of the comparison uses the same rule.
  2. **Other zones.** An other-net zone that may pour first knocks the zone out by its whole outline, grown by the larger of the two zones' clearances and the class clearance. A same-net zone with no `zone` line takes its outline out of a lower modelled zone.
  3. **Keepouts.** Every DSN keepout, and every `keepout` line of the file, is grown by a cell.
  4. **Holes.** A pad's hole is bounded by the pad itself. A via's drill is the larger of its padstack name's drill and the scaled one.
  5. **Thermal pads.** Every thermal pad of the zone's net on the layer gets its gap knocked out, even one that only grazes the outline.
  6. **Spokes.** A spoke is dropped when another net blocks any cell of it, when it only meets another spoke, or when its far end is off the opened fill.
  7. **Teardrops.** A teardrop is measured from where the track leaves the pad, and continues as a disc where the track ends first.
  8. **Islands.** `remove` clears every unattached piece.
  9. **A size cap.** Cells times modelled zones are capped at 40 M, and a model past the cap is a build error in `planes.error`.
- **Known limit:** an other-net zone with no `zone` line has no clearance the engine knows. Konnect raises each modelled zone's clearance to that of every other-net legacy zone on its layer instead.

### Konnect

- **`zone_plan`** reads the saved `.kicad_pro` and `.kicad_dru` beside the board.
- **Each zone's clearance** is raised to the board's `min_clearance`, and to that of every other-net legacy zone on its layer.
- **The pad lines** carry each pad's resolved connection, gap, bridge and angle (the angle modulo 90, relative to the pad).
- **P4 refusals, added to those in "Principles":**
  - a board-wide reason, making every pour legacy: a custom rule that sets a clearance, connection, spoke or `disallow` value; per-layer padstack values (a clearance on any net; connection or thermal on a pour net); `start_end_only` vias; a convex-hull custom pad of any net on a modelled layer; track-to-track teardrops; an unreadable project, teardrop or keepout value; DSN pin names that differ from the board's;
  - a net's zones made legacy: a pad whose roundness the DSN and KiCad's filler disagree on, a pad that needs a line under two zones with different values, a pad that would need an empty name.
- **The comparison** (`compare_pads`) is taken on the first refilled board, before any stitching round:
  - KiCad's side is the layout audit's own grouping over the saved fills (`pads_off_main_group`, which check 3.3 now shares), with the same strict-majority rule.
  - A failed side leaves both counts `None`, with its reason in `pours_error`.
  - A Tune candidate's board is not compared.
  - A pruned winner (ZR0b) keeps the router session's two counts: the engine modelled that session, never the pruned one.
- **The probe** sends `--x.zones=<scratch>/probe-zones.txt`, a valid file with no `zone` line, so x19 is refused before any run.

### The measurement (2026-11-23)

One `route_specctra_dsn` call per board at effort M, with x20 and this branch's release build, each in its own routing hold, on the five boards of "The x19 baseline". Every as-exported judged run was compared.

| Board | Runs compared | False connections | False opens (total, runs with any) | Model build, longest |
|---|---|---|---|---|
| dash | 6 | 0 | 10, 3 | 0.40 s |
| B5 | 26 | 0 | 42, 25 | 0.34 s |
| `pic` | 83 | 0 | 0, 0 | 0.84 s |
| `ecc` | 721 | 0 | 0, 0 | 0.07 s |
| `pours` | 709 | 0 | 0, 0 | 0.15 s |

- **Zero false connections** in 1,545 compared runs, and no engine error or `pours_error`.
- **False opens** are at most 2.2 % of the modelled pads (B5, 73 a run), under the 1 in 20 of "Verification plan".
- **Sessions are byte-identical** with and without `--x.zones` on all five boards: x20 driven directly with a routing pass's switches and the call's own files, `--no-time-limits`. The reports differ only by `planes` and the run time.
- **The winners** are pours omitted on all five boards, as in ZR0.
- **Judged run counts:** dash 11, B5 51, `pic` 165, `ecc` 1,442, `pours` 1,417. `pic` is about 10 % below ZR0's 183 and Row GS's 185, which is the cost of drawing the model on every as-exported run there.
- **The ground-via prune** (ZR0b): dash 41 checks in 85 s, B5 41 checks in 77 s. The other three boards have no via on a ground pour net.

**Not verified:** a model drawn more than once per run (ZR2), and boards whose pads have no UUID (older than KiCad 7). On those the comparison is recorded as a `pours_error`, never as a count.

## ZR2 as built (2026-11-23)

Stage ZR2 is engine work: fastroute build **x21**, branch `zr2-truth`, commit `20d7c96` (the branch's head `be366d8` adds one doc comment after it). That is x20's `c13a28f` plus the model as the routing's connectivity, the landing points, the closing stage and the redraw points. Konnect pins the measured binary (SHA-256 `46d79d56`) in `packaging/1-kopper/fastroute.json`. fastroute's release builds are not byte-reproducible (a rebuild of the same source gave another hash), so the pinned file is that binary, kept with the records and in the owner's iCloud. Konnect also renames the candidate `C** + BR1 + W8P + GS + ZR2`, and cuts the closings' open-pad lists to counts in the result. Konnect sends nothing new: `--x.zones` turns ZR2 on as it turned ZR1 on. The gates and the rank key do not change.

### The engine

- **The model is the truth for a modelled net.** `connected_set` of such a net is the model's group. Two pads join through a pour only where the model's piece holds both. Contacts with a modelled plane need the item on the zone's fill, so the cycle rules through a plane are piece-aware.
- **Landing points.** The maze takes a target door on a modelled pour only at a landing point. That is a cell whose disc lies under the fill, centred on live fill, nearest to where the route enters. The locator places the track end there, nearest to the next via or door. The via optimizer moves a plane via only onto live fill.
- **The closing stage.** It runs after the routing passes, after the optimizer, and after the last stage when a redraw finds work. Each round routes every stranded group to its net's main group, with rip-up. It adds a spoke track for each starved thermal, per zone it is starved on. A connection is kept only when the group then joins the main group, no violation is added, and its own net's open count goes down. A round that does not lower the open count ends the stage. Each closing ends with a redraw.
- **Redraws.** The model is drawn again at the top of every autorouter and optimizer pass, on every swap to a saved board, at checkpoints, in the closings, and after each group-search member (Z15). It is drawn only when the copper changed. A two-slot cache reuses a model drawn before for the same copper.
- **The main group, two readings.** The router's main group (`NetGroups::main`) is usually the group with the most pins. There are two exceptions. A net with nothing on its pours takes the largest piece as a pin-less main group. Where the most-pins group holds no pour cell while another group does, the main group is the group holding the largest piece, so the pour stays live once one pad has landed on it. The report's pad split keeps ZR1's rule: the most pins, with a strict majority. So the closing's `left_open`, which is the router's view, can list a pad that the split calls connected.
- **Islands.** A zone whose every piece is an island keeps them, as KiCad does: a pour that only vias will reach is not wiped.
- **Fanout.** `--x.zones` with the fanout on is refused. Konnect's passes send it off.

### The measurement (2026-11-23)

One `route_specctra_dsn` call per board at effort M, with x21 and the ZR1 gate's release build of Konnect (`808c260d`), each in its own routing hold with a hard timeout. The boards are those of "The x19 baseline". Every as-exported judged run was compared.

| Board | As-exported runs | KiCad open before stitching 0 | Stitching vias needed | G1 passing (ZR0 → ZR2) | G2 passing (ZR0 → ZR2) | False connections, false opens |
|---|---|---|---|---|---|---|
| dash | 1 | 1 | 0 | 0/7 → 1/1 | 7/7 → 1/1 | 0, 0 |
| B5 | 6 | 6 | 0 | 0/28 → 6/6 | 7/28 → 6/6 | 0, 0 |
| `pic` | 37 | 37 | 0 | 1/92 → 25/37 | 0/92 → 35/37 | 0, 0 |
| `ecc` | 663 | 663 | 0 | 710/710 → 663/663 | 710/710 → 663/663 | 0, 0 |
| `pours` | 675 | 673 | 0 | 575/707 → 673/675 | 537/707 → 572/675 | 0, 0 |

- **The done criterion holds on every board.** KiCad's open count before stitching is 0 and no stitching via is needed on at least 9 of 10 as-exported runs (the lowest share is `pours`, 673 of 675). The G1 and G2 pass shares are not below ZR0's.
- **Zero false connections and zero false opens** in 1,382 compared runs. ZR1 had 52 false opens on the owner's two boards. No engine error and no `pours_error`.
- **The winners:** B5 and `pic` are now as exported, with their pours as planes. dash, `ecc` and `pours` are still pours omitted. In ZR0 and ZR1 every winner was pours omitted.
- **Fewer runs are judged** in a call of about the same length: dash 3, B5 16, `pic` 73, `ecc` 1,325, `pours` 1,349 (ZR1: 11, 51, 165, 1,442, 1,417). An as-exported run now routes the closings and draws the model many times.
- **Model draws:** the longest was 0.34 s (`pic`). The median was at most 0.074 s.
- **Sessions are byte-identical to x20's without `--x.zones`** on all five boards (x20 and x21 driven directly with a routing pass's switches and the call's own files, `--no-time-limits`).

### Limits

- **The closing's partial-group path has no board test.** A stranded group is joined only in part when a rip splits it mid-round. The helper that picks what is left (`still_to_route`) is tested on its own.
- **The last redraw of a group-search move** has no control that bites (C10). The member's own tail removal already runs before it.
- **No windowed redraw.** A redraw reuses the fixed layers and draws the rest in full.
- **A unit-level difference from KiCad:** the model decides islands per plane, KiCad per zone across its layers. Both err on the safe side for connections.
