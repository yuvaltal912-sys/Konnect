# Pours in the rank: design of stage ZR5 (the pour record, then the pour key)

Status: a design down to function skeletons, 2026-11-23. Nothing is built: every body is
`todo!()`. Stage ZR5 of Row ZR ([ZONE_ROUTING_DESIGN.md](ZONE_ROUTING_DESIGN.md), the stage
table and Z10). Designed in a Claude Fable 5.2 session on branch `claude/zr5-pour-rank-plan`,
from `fork/main` `c4c34369`. **Reviewed** by an agent that did not write it (Claude Opus 5.5,
high, read only); its two must-fix items and its other findings are in this text (see
"Verification of this document"). It needs nothing of stage ZR4's code and touches no engine
file.

## What the owner said, and what is still his

- He started this design on 2026-11-23, in the session that designed ZR4: "לאחר שהוא יסיים
  וידחף את התוכנית שלו הוא ימשיך לתכנון של ZR5 באותה מתכונת" (relayed verbatim by the
  supervisor session).
- **The order, 2026-11-23:** "כן, ZR5 קודם" (relayed verbatim by the supervisor session).
  The question was whether ZR5's recording, which changes no rank, merges before ZR3's
  measurement, so that ZR3's runs carry the new pour measures. It sets the order only: each
  push, each PR text and each merge still needs his yes.
- The row's stage table gives ZR5 as "Pour measures in the rank key; the skill's order of
  work", done on "the owner's word, on ZR3's numbers". Z10: "Pour terms enter the rank key
  last (ZR5). The key reproduces a measured order. Changing it needs its own numbers."
  "Last" there is the last stage, not the last place in the key.
- **Three decisions, 2026-11-23:** "כן, כמו שהמלצת בשלוש ההחלטות", to the three under
  "Decisions for the owner" below, each as recommended: the pour key is switched on after
  ZR3's table, on his word; the skill stops saying "pours last" once ZR4 and ZR2 are
  merged; ranking candidate pour divisions by routing is its own stage.

## The short answer

The stage is two PRs, in his order.

1. **The pour record (first, before ZR3's measurement).** Every judged run records what the
   routing did to the pours, measured against KiCad's own fill of the board before it was
   routed: fills that vanished, how much of the planes is still one piece, and how much pour
   copper was lost. No rank changes: today's key is today's, byte for byte.
2. **The pour key and its setting (second, after ZR3's numbers).** A second key beside
   today's: today's thirteen entries in today's order, with four entries added from the
   record. One setting says which key ranks, `routing.rank_key` in the user config; the
   default is today's key. Both keys are written for every judged run, and every call says
   which run the other key puts first.
3. **Switching the pour key on is neither PR.** It is a one-line change on his word.

## Why the key needs it

- The judge's rank key has no pour term. `pour_track_mm` is recorded and not ranked, and
  ZR0's pour measures are "recorded only" (`route_judge.rs`, `Measures::pours`; Z10).
- So two runs that pass every gate, one with the ground plane whole and one with it cut in
  two, rank by track length and vias alone. Stages ZR2 and ZR3 teach the engine to connect
  through a pour and to cut it less. Without a pour term the search cannot prefer the runs
  where that worked.
- The owner's checklist, section 3: "on 2 layers keep one side largely continuous ground
  and inspect every cut", and "signals must not cross plane splits, slots, voids or narrow
  ground bottlenecks". Its first line puts the return path beside the signal path.

## What the recorded runs show (2026-11-23, no new run)

An offline replay of today's key from the recorded measures (`run.json`), on two finished
matrices: ZR1's (fastroute x20) and ZR2's final one (engine build `46d79d56`, run by the
session that builds ZR2). It reads files only. An earlier ZR2 matrix (`2d559bc2`) is from
before ZR2's two fix rounds and is not quoted. Record:
`target/creation-research/zone-routing/zr5/design-replay/` in the main checkout.

- **The replay computes today's key exactly:** all 3,086 recorded keys of ZR1's matrix and
  all 2,766 of ZR2's, all thirteen entries of each. It is the second, independent
  computation the measurements below use.
- **With ZR2 the as-exported shape has runs that pass every gate on all five boards** (on
  ZR1's matrix, on two). So runs that connect ground through the pour and runs that route
  it as track now meet in one ranking, and today's key cannot tell what either did to the
  pours.
- **A pour term changes winners, and where it sits decides how often.** With ZR0's recorded
  measures (starved thermals, pour layers with no fill, open before stitching, pieces, the
  largest piece's share) placed as a block:

  | Where the block sits | ZR1's matrix: boards whose winner changes | ZR2's matrix |
  |---|---|---|
  | Before the lengths | 3 of 5 | 3 of 5 |
  | After the right angles, before the vias | 2 of 5 | 2 of 5 |
  | Last, before the seconds | 0 of 5 | 0 of 5 |

- **A raw piece count is not a safe measure.** In ZR2's matrix, on one of the owner's
  boards, the runs that route ground as track (pours omitted) have 10 to 13 fill pieces and
  the runs that route through the pour have 13 to 16. Ranked on pieces first, the winner
  there becomes a run with the ground routed as track. The review found why a piece count
  can even reward damage (PK6).
- **Today's winner can carry a pour fault that other runs do not.** On the same board the
  winner has 1 starved thermal; runs of both shapes have none. Gate G2 lets it pass because
  the unrouted board has one too.
- **On the two small public fixtures the pours are never cut** (`ecc`: 1 piece in all 1,325
  judged runs of ZR2's matrix; `pours`: 2 to 5). There the only difference a pour makes is
  the ground track: 0 to 8 mm through the pour, 88 to 143 mm with the pours omitted, and
  today's key cannot see it.
- **What the recorded runs cannot show:** the numbers of this design. They need fill areas
  per layer, before and after routing, which no run recorded. That is why the record comes
  first.

## The two parts

| | Part 1: the pour record | Part 2: the pour key |
|---|---|---|
| What | The "before" rows in the baseline, the "after" rows and the damage in every judged run | The second key, the setting, the report of the other key's choice |
| Changes a rank | No | Only when the setting is `pours`; the default is `routing` |
| When | Now, so that it is merged before ZR3's measurement (his order) | After ZR3's numbers have confirmed the key's four open values |
| Skeleton | `pour_damage.rs` | `pour_rank.rs` |
| Open values | None | The two steps and the two places ("The pour key" below) |

Part 1 fixes what is measured. Part 2's steps and places are fixed here as the candidate,
and the replay computes that candidate, and any other, from part 1's rows on ZR3's runs
before a line of part 2 is built.

## Decisions (the code cites them as PK1 to PK18)

| # | Decision | Reason |
|---|---|---|
| PK1 | Today's key is not changed: `Key`, `KEY_LEN`, `KEY_QUALITY`, and every value `rank` and `rank_measures` return. The pour key is a second key beside it | Z10. "Off" is then today's ranking exactly, every existing test of the key keeps guarding it, and the replay checks it on every recorded run |
| PK2 | The pour key is today's thirteen entries in today's order, with four added: pour faults and the plane-loss step before the lengths; the area-loss step and the pour-track step after the vias. No existing entry changes its meaning | One standard for a board with pours and a board without: with no pour net the four are 0, so the order is today's. The existing entries are a measured order (D12), and inserting leaves their order among themselves intact |
| PK3 | Pour damage is measured against KiCad's own fill of the board before it was routed (the judge's baseline, refilled by kicad-cli). Only copper that is connected counts: a piece KiCad saved as an island is not fill | The fill before routing is what the routing had to keep. A zone that is two pieces by design costs nothing. KiCad's refill is the truth of the row (P1). An island that a zone's own rule keeps (`(island yes)`) touches nothing of its net, so it is no plane |
| PK4 | **Plane loss.** For each pour net: its plane before routing is its largest piece on any one layer; what is kept is its largest piece on any one layer after routing (on a layer, never more than that layer's largest piece before). The loss is the area no longer kept over the area of all the nets' planes, a share from 0 to 1. In the key it is stepped at 5 % | The checklist's "keep one side largely continuous": one whole reference plane is what a return path needs, so the best single layer counts. One number sees a split (the largest piece shrinks to a part) and a slot or a lost island (it loses area). Weighted by area, because with equal weights a 30 mm² local pour cut by one track would count like a fifth of the ground plane (the review). By area and not by each layer's ratio, so that a small layer that stayed whole cannot hide a plane that was destroyed |
| PK5 | **Pour faults** are KiCad's `starved_thermal` violations on the judged board plus the pour layers that had fill before routing and have none after. One entry, a count, before the plane loss | Each is a spot where a pour stopped doing its job. KiCad calls a starved thermal an error by default. Gate G2 only holds it to the unrouted board's count, and ZR2's matrix has a winner with one more than other runs |
| PK6 | **Area loss** is the pour copper that is gone: 1 minus the fill area after routing over the fill area before, over every pour layer. In the key it is stepped at 1 %, after the vias. The pieces stay recorded (ZR0) and are not ranked | KiCad deletes a cut-off piece unless something of its net sits on it (`zone_filler.cpp:944`). So of two runs that cut the plane alike, the one whose cut-off piece is kept alive by a ground pad or via has one piece more and more copper. A piece count would rank it worse; the area ranks it better, which is also what the prune holds (P5(d): losing a via-fed island is harm). The first design ranked extra pieces; the review found this |
| PK7 | **The pour-track step** is the track length on pour nets in units of 2 % of the shortest signal length of the pool (`LENGTH_STEP`). It is the last of the four, before the copper quality. Vias on pour nets are not in the key | A ground track that the pour already connects is copper the board does not need, as the owner's own rule says of ground vias (ZR0b). Under a pour it harms nothing electrically, so it breaks ties and never outranks a signal measure. The unit is the signal length because a board's own scale is the fair one and stubs of a few mm then tie. Ground vias are the prune's: it removes the redundant ones from the winner, so a count before the prune would rank vias that are not delivered |
| PK8 | A judge that took no pour record gives 0 in the three damage entries: a call with no pour net, and the prune's light judge. A run whose pour record was tried and failed, or has no damage, gets the largest number there | The first keeps today's order on a board without pours. The prune's light judge (`judge_light`) takes no pour record for any of the three boards it compares, so 0 on all three ranks nothing there; its loop guards the pours with its own clauses (no rule's count higher, no pour layer's fill lower), and its final comparison is of two full judges. The second is the key's own rule ("a measure the audit did not give sorts last"). It is still not a gate: a failed pour measure never fails a run (ZR0) |
| PK9 | One setting says which key ranks: the user config's `routing.rank_key`, `routing` or `pours`. The default is `routing`. There is no tool argument. `route_specctra_dsn` reads it on every fastroute call, whatever arguments the call has | It is the owner's standing choice, like `routing.effort`, not something an AI picks per call. Today the user config is read only when a call leaves the effort to it (`prepare_call`), so a setting read "where the effort is read" would be ignored whenever `effort` is passed (the review) |
| PK10 | Both keys are computed and written for every judged run, whatever the setting. Every call says which key ranked and which run the other key puts first among the boards this call judged | Z10 asks for numbers, and this gives them on every real call. It is a report on this call's boards, not a second search: each run's tuned board was chosen, and the winner pruned, under the key that ranked |
| PK11 | The engine's input does not depend on the setting. The uncounted file (`--x.gs_uncounted`, Z20) stays as it is | The pour track is a tie-break in the pour key, not a cost the engine should buy with signal length. And with pour nets in the engine's cost every ground and stitching via would cost the search, which could work against the prune and against ZR2's closing vias (the ZR2 session's caution). Z20's "the uncounted set follows the key" is left for a stage that has numbers for it |
| PK12 | Everything that ranks goes through one function with the setting: the search's winner and the runs it releases, the choice among a run's tuned candidates, the prune's "no entry higher" clause, its final comparison and the keys it takes again, and the result's listing | One standard with the search (the prune's own reason, P5). A site left on the other key would keep or drop what the winner's key does not |
| PK13 | Placement is not changed: its key keeps copying today's quality entries, and the routing calls of the placement tools rank by today's key whatever the setting, through a constant, as they already switch the prune off (`place_route::CANDIDATE_PRUNE`) | Their key ranks placements with today's entries; a candidate's routing winner chosen by another key would be judged by a standard its ranking does not read. In the order of work placement comes before the pours, so a new board's candidates have none. A board placed again with its pours in gets no pour term: a limit, written down |
| PK14 | Part 2 ships with the setting at `routing`. Switching the default is its own one-line change, on the owner's word | The stage's done criterion is his word on ZR3's numbers |
| PK15 | The skill's order of work is designed here and applied once stages ZR4 and ZR2 are merged (the row's decision 5, answered by the owner on 2026-11-23): pours are planned and added before the routing export | It names `plan_pours`, so it follows ZR4's merge; and it is only right once the router connects through a pour (ZR2) |
| PK16 | Ranking candidate pour divisions by routing each one is not in this stage | It can only rank with the pour key, which is off until the owner's word. The measures are built so that it can: damage is against each candidate's own fill before routing, and the plane loss is a share of plane area, so a pour that is added and stays whole lowers it |
| PK17 | Two PRs: the record first, the key after ZR3's numbers | The owner's order. It also lets numbers confirm the key's open values before they are code |
| PK18 | The record carries its rows: the "before" rows in the baseline and the "after" rows in each run, beside the numbers made from them | So that a second computation can redo every number, and so that another step, place or sum can be tried on recorded runs without a new measurement (the review: totals alone cannot be checked) |

**One measure in two places is a weighting, not a double count.** Starved thermals are also
held by gate G2 and by the prune's clause (b); the audit's checks 3.1 to 3.4 read planes
inside the copper quality (entry 15). In a key that is an order, a later entry only breaks
ties of the earlier ones.

## Pour damage (part 1)

```text
before   the judge's baseline: the saved board, refilled by kicad-cli, before this routing
after    the judged board: routed, refilled, stitched (the board ZR0's measures read)

a layer fill: one pour net on one copper layer, over every zone of the net on that layer,
              of the pieces KiCad did not save as islands:
              pieces, the largest piece's area, the total area; and the islands' count

lost_fills   layer fills with a piece before and none after
plane_loss   for each net:  plane = max over layers( largest before )
                            kept  = max over layers( min(largest after, largest before) )
             sum over nets( plane - kept ) / sum over nets( plane );  0 when no net has fill
area_loss    1 - sum over layer fills( min(area after, area before) ) / sum( area before )
             over the layer fills with area before;  0 when there is none
```

- **An undamaged board scores 0, 0.0 and 0.0**, whatever its zones look like.
- **A saved board that already has routing** gives the damage of this routing: the baseline
  is that board, refilled.
- **Read on the judged board,** after the judge's stitching rounds, as ZR0's measures are.
  The stitching vias are not in the delivered session (D21): a limit, as for ZR0's.
- **Zones of one net that overlap on a layer are not merged:** the largest piece is the
  largest saved piece of any of them. A limit.

### What a run records

```json
"baseline": {"ok": true, "violations": {}, "unconnected": 61,
  "pour_fills": [{"net": "GND", "layer": "B.Cu", "pieces": 1, "largest_mm2": 4000.0,
                  "area_mm2": 4000.0, "islands": 0}]},

"measures": {"pours": {"pieces": 3, "starved_thermals": 0,
  "damage": {
    "lost_fills": 0, "plane_loss": 0.07, "area_loss": 0.11,
    "nets": [{"net": "GND", "plane_layer": "B.Cu", "plane_mm2": 4000.0,
              "kept_layer": "B.Cu", "kept_mm2": 3720.0, "kept_share": 0.93}],
    "layers": [{"net": "GND", "layer": "B.Cu", "pieces": 3, "largest_mm2": 3720.0,
                "area_mm2": 3560.0, "islands": 0}]}}}
```

(The numbers are made up.) `baseline.pour_fills` is in the result and in `search.json`;
`measures.pours.damage` is in every judged run's `run.json` and in the winner's measures.

- **Recorded only, as ZR0's measures are.** A fill that cannot be read never fails the
  baseline or a run. `pour_fills` is then absent with `pour_fills_error`, and the runs'
  `damage` is absent.
- **An older record still reads:** `damage` is optional in `PourMeasures`. Nothing reads a
  baseline back, so `Baseline` stays write-only.

### Part 1: changes to existing items

| Where | Change |
|---|---|
| `zone_fill.rs` | `ZoneLayerFill` gains `islands: Vec<bool>`, one per piece in the order of `pieces_mm2`: KiCad saved the piece with `(island yes)`. `pour_fills` fills it; its present readers read `pieces_mm2` as they do. `PourMeasures` gains `damage: Option<PourDamage>` (`#[serde(default, skip_serializing_if = "Option::is_none")]`). `pour_measures` keeps its signature and its results; its body from the fills on becomes a function of the fills, which `pour_record` calls too. `pour_record` takes the baseline's rows and fills `damage` from the fills it has already read: the judged board is parsed once, as now |
| `route_judge.rs` | `Baseline` gains `pour_fills: Option<Vec<LayerFill>>` and `pour_fills_error: Option<String>`, read from the baseline's refilled scratch board when the call has pour nets. `judge_one` passes the rows to `pour_record` |
| `tools/route_fastroute.rs` | `baseline_json` writes `pour_fills` and `pour_fills_error` (its fields are listed one by one) |
| The tests that build a `Baseline`, a `PourMeasures` or a `ZoneLayerFill` by hand | The new field, with nothing in it |
| `lib.rs` | The new module |

Not touched in part 1: `rank`, `rank_measures`, `route_search.rs`, `pour_prune.rs`,
`place_judge.rs`, `fastroute.rs`, the fastroute pin, any engine file, anything of stage ZR4.

### Part 1: the skeleton (`pour_damage.rs`)

| Item | What |
|---|---|
| `LayerFill`, `layer_fills` | One pour net's connected fill on one layer, from the judge's own `pour_fills` rows |
| `NetPlane`, `PourDamage`, `pour_damage` | The damage of "before" and "after", with its rows |

### Part 1: tests, each guard with a negative control

A control breaks the guard in the source, sees its test fail, and restores the file.

| Guard | Test | Negative control |
|---|---|---|
| An undamaged board scores nothing | "after" equal to "before", with a zone that is two pieces by design: 0, 0.0, 0.0 | the total area as the plane: a loss appears |
| The best single layer counts, by area | front and back planes of 4000 mm² each; the front cut to 2000, the back whole: loss 0. Both cut, to 2400 and 3200: loss 0.2 | the worst layer: 0.5 and 0.4 |
| A small whole layer does not hide a destroyed plane | a net with a 4000 mm² plane cut to 1000 and a 20 mm² zone on another layer that stayed whole: loss 0.75 | the best ratio instead of the best area: loss 0 |
| The nets are weighted by area | a 4000 mm² plane that lost 400 and a 30 mm² pour that lost 6: loss 406 / 4030 | equal weights: (0.1 + 0.2) / 2 |
| A piece that grew is not a gain | the largest piece larger after than before on a layer: that layer keeps at most its own "before" | the clamp removed: a negative loss |
| A fill that vanished is a fault | fill before, none after, on the net's only layer: 1 lost fill, plane loss 1.0, area loss 1.0. With a second layer that is whole: 1 lost fill, plane loss 0 | read as intact: no fault |
| A cut-off piece that a via keeps alive is not worse than one that is gone | the same cut twice: once the cut-off piece is in "after", once it is not. The first has the lower area loss and the same plane loss | pieces ranked instead of area: the first is worse |
| An island KiCad kept is not fill | a piece saved with `(island yes)`: not in pieces, largest or area; counted in `islands` | the flag not read: the island is the largest piece |
| Several zones of a net on a layer are one fill | two zones: their areas add, the largest piece is the largest of both | per zone: two rows |
| A layer fill with no row after is 0 after | a net and layer in "before" and not in "after": lost when it had a piece. A row only in "after" is not read | the missing row read as unchanged: no loss |
| A net with no fill before adds nothing | no fill before on any layer: not in the sums, and its row says so | counted as wholly lost: 1.0 |
| The rows are in the record | `damage.layers` are the "after" rows and `damage.nets` one row per net of "before", with the layers the two areas were taken from; ties go to the first layer in "before"'s order | a row dropped: the test fails |
| "Before" is the board's fill before routing (needs kicad-cli, ignored) | a fixture saved with no fill: the baseline's rows are the fills of its refilled scratch board | read before the refill: no rows |
| A fill that cannot be read fails nothing | an unreadable baseline board: `pour_fills` is absent with its error, the baseline stands, runs are judged, their `damage` is absent | the error passed up: the judge fails |
| An older record still reads | measures without `damage` deserialize | the `serde(default)` removed |
| The record changes no rank | two runs alike but for their damage: `rank_measures` gives one key | the damage read by `rank_measures`: two keys |
| The fills' present readers read what they read | `pour_measures` on its existing fixtures: the same `PourMeasures` but for `damage`; the prune's fill clause on a fixture with an island: the same refusals | `pieces_mm2` emptied of islands: ZR0's pieces change |

### Part 1: measurement, criteria fixed before any run

**Boards:** the five of "The x19 baseline" (dash, B5, `pic`, `ecc`, `pours`), each as it is,
with its own pours. **Build:** the branch's release binary by pipe, the fastroute that
`packaging/1-kopper/fastroute.json` pins on the day, effort M. One `route_specctra_dsn` call
a board, each in its own routing hold, under the lane locks.

| Step | What | Criterion |
|---|---|---|
| M1 | The call's records | R1: the baseline has a row for every pour net and layer of the board, and every judged run has a `damage` with an "after" row for each of them. A run without one is listed with its reason |
| M2 | A second computation: a script with its own reader of `filled_polygon` reads the kept baseline board (`judge/baseline`) and the winner's judged board | R2: it gives the winner's `lost_fills` exactly, its two losses within 1e-6 and every row's areas within 0.001 mm² |
| M3 | The same records | R3: on every board at least one judged run has an area loss above 0, and no run has a loss below 0 or above 1 |
| M4 | The replay of today's key from each run's recorded measures | R4: every recorded `key` equals the replay's, and the winner is the least of them: the record changed no rank |
| M5 | The judge's seconds a run, beside ZR2's matrix on the same boards | recorded, not a criterion |

- **Done for part 1 when** R1 to R4 hold on all five boards and the gate is green.
- A criterion that fails is reported with its cause. Results of the owner's boards stay out
  of tracked files, apart from counts. Records go to
  `target/creation-research/zone-routing/zr5/` in the main checkout.

### Part 1: documents

- This file gains "Part 1 as built" and "Part 1 measured".
- `docs/API_MIGRATIONS.md`: one entry. `baseline.pour_fills` and the measures'
  `pours.damage`. Nothing is removed or renamed.
- `route_specctra_dsn`'s description and the skill's "Best-of-N routing": one sentence on
  what `pours.damage` says. The words the skill then quotes go into `NOT_TOOLS` of
  `crates/konnect/tests/asset_references.rs`.
- `docs/ZONE_ROUTING_DESIGN.md` and `docs/CREATION_HANDOFF.md`: where the stage stands.

## The pour key (part 2: the candidate)

`[i64; 17]`; the least is the best. Entries marked "today" are today's entry, unchanged.

| # | Entry | From |
|---|---|---|
| 0 to 3 | G1, G2, G3, G4 failed | today 0 to 3 |
| 4 | The intent's length miss | today 4 |
| 5 | KiCad's own length and skew violations | today 5 |
| 6 | Acute corners | today 6 |
| **7** | **Pour faults**: starved thermals plus lost fills | new (PK5) |
| **8** | **Plane-loss step**: `round(plane_loss / 0.05)` | new (PK4) |
| 9 | Signal length step | today 7 |
| 10 | Power length step | today 8 |
| 11 | Right angles | today 9 |
| 12 | Vias off the pour nets | today 10 |
| **13** | **Area-loss step**: `round(area_loss / 0.01)` | new (PK6) |
| **14** | **Pour-track step**: `round(pour_track_mm / shortest signal / 0.02)` | new (PK7) |
| 15 | Minus copper quality | today 11 |
| 16 | Route seconds | today 12 |

- **The pool is today's.** The shortest signal length of entry 14 is the one today's entry 7
  steps against: among the runs in the pool that pass G1, else the pool, else every run.
  When no run of it has signal track, entry 14 is 0 for all.
- **Four open values:** the two steps (5 % and 1 %) and the two places (before the lengths,
  after the vias). They are the candidate. Part 2 is built now with these four, the switch
  off: asked "לבנות כבר עכשיו את ZR5 חלק 2, עם המתג כבוי?", the owner said "כן"
  (2026-11-23, relayed verbatim by the supervisor session). After ZR3's runs, the replay
  computes the candidate key from part 1's rows on them, and the table below goes to the
  owner. If his boards show a value wrong, only the values change.

### The setting, and what every call reports

```json
"rank": {
  "ranked_by": "routing",
  "source": "default",
  "other_key": "pours",
  "other_winner": 12,
  "same_winner": false
}
```

- `search.rank` of `route_specctra_dsn`'s result. `other_winner` is the index of the run
  the other key puts first among the boards this call judged (a released run included;
  `null` when no run was judged). Each of those boards is the one the ranking key kept: a
  run's tuned choice was made by it, and the winner's keys are taken after the prune. So it
  says what the other key thinks of this call's boards, not what a search under the other
  key would have delivered.
- `key` (today's) and `pour_key` (new) are written where `key` is written today: every
  judged run's `run.json`, the winner in the result, and the prune's `before`.
- **What the setting changes:** which run wins, which judged runs the search keeps whole,
  which tuned candidate a run keeps, and the key the prune reads. Nothing else.
- **A user config that does not parse refuses every fastroute call of the route tool.**
  Today a call that passes both `effort` and `fastroute_path` does not read the config and
  routes. A setting that is there and cannot be read is not guessed past.

### Part 2: changes to existing items

| Where | Change |
|---|---|
| `route_judge.rs` | `JudgeContext` gains `rank: RankMode`, the one place a call's mode is kept. `judge_candidates` ranks through `pour_rank::ranking_of`. The pool rule and the step of `rank_measures` become two `pub(crate)` helpers that the pour key calls too; `rank_measures` returns what it returned |
| `route_search.rs` | `release_beyond_the_best` and `finish` rank through `pour_rank`, with the mode of the plan's judge context. `SearchOutcome` gains `pour_keys` beside `keys` |
| `pour_prune.rs` | Clause (c), the full judge's final comparison, `keys_with` and `replace_winner` read the ranking key of the mode and take both keys again. The position in a `key:<n>` refusal is the ranking key's. `before` gains `pour_key` |
| `tools/route_fastroute.rs` | `route_with_fastroute` takes where the mode comes from: the user config (the route tool; read on every call) or a fixed mode (a caller that pins it). `listed_runs` sorts by the ranking key. `run.json` and `winner_json` gain `pour_key`; the result gains `search.rank` |
| `place_route.rs` | `CANDIDATE_RANK: RankMode = RankMode::Routing`, passed by `route_candidate` beside `CANDIDATE_PRUNE` (PK13) |
| `lib.rs` | The new module |

Not touched: `place_judge.rs` (PK13), `fastroute.rs` (PK11), any engine file.

### Part 2: the skeleton (`pour_rank.rs`)

| Item | What |
|---|---|
| `PLANE_STEP`, `AREA_STEP`, `POUR_KEY_LEN`, `PourKey`, `POUR_FAULTS`, `PLANE_LOSS`, `AREA_LOSS`, `POUR_TRACK` | The steps, the key's type, the four new positions |
| `plane_step`, `area_step`, `pour_track_step` | The three steps |
| `pour_keys` | The pour key of every run at once |
| `RankMode`, `RankSource`, `RankChoice`, `resolve_rank_mode` | The setting |
| `ranking`, `ranking_of`, `entries_above` | What every ranking site calls |
| `rank_report` | `search.rank` |

### Part 2: tests, each guard with a negative control

| Guard | Test | Negative control |
|---|---|---|
| Today's key is today's | `ranking(Routing, runs)` equals `route_judge::rank_measures(runs)` entry for entry, on runs that differ in every measure | an entry swapped in that arm |
| Today's entries keep their order in the pour key | each of the thirteen at its place: 0 to 6, then 9 to 12, 15, 16 | a place shifted by one |
| No pour record, today's order | runs with no pour measure and no pour error (a call with no pour net; the prune's light judge): entries 7, 8 and 13 are 0. With no track on a pour net either, the two keys sort the runs alike, ties included | the largest number written there: the entries differ |
| An unmeasured run does not outrank a measured one | two runs alike, the first with `pours_error`: it sorts after the second, and its entries 7, 8 and 13 are the largest number. The same for pour measures with no damage | 0 written for it: the first sorts first |
| Faults, then the plane, then the lengths | pairs that differ in one entry each; and one starved thermal more against two length steps fewer: the starved run loses | two places exchanged |
| The steps round as the lengths do | plane: 0.024 is 0, 0.025 is 1, 0.074 is 1, 0.076 is 2. Area: 0.004 is 0, 0.005 is 1. A loss that is not a finite number of 0 or more is the largest number | rounded down: 0.025 is 0 |
| The pour track is in signal units | the shortest signal 1000 mm: 9 mm is 0, 10 mm is 1, 400 mm is 20. No signal track in the pool: 0 for all | the shortest pour track as the unit: 9 mm is 50 |
| The pool is today's | a run outside the pool, and one that fails G1, do not set the unit | every run sets it |
| The default is today's key | no `routing.rank_key`: `Routing`, from the default; `"pours"`: `Pours`, from the user config | `Pours` as the default |
| A value that is not a name is refused | `"pour"`, `3`, `""`: an error that names `routing.rank_key` and both names; `null` is "not set" | a silent fall to the default |
| The setting is read whatever the arguments | a call with `effort` and `fastroute_path` given and `routing.rank_key: "pours"` in the config: ranked by `pours` | read only when `effort` is absent: ranked by `routing` |
| Placement's routing calls rank by today's key | `route_candidate` with `pours` in the config: `ranked_by` is `routing`, and the config is not read | the constant ignored: `pours` |
| Each site follows the mode | a scripted pool on which the two keys disagree: the search's winner, the runs it releases, a run's tuned choice (an exact tie still keeps the untuned board), the prune's final comparison and the keys it takes again, and the listing each follow the key of the mode, under both modes | per site: left on `route_judge::rank`, its case fails |
| The prune's refusals name the ranking key's places | a candidate with a longer signal step: `key:9` under `pours`, `key:7` under `routing` | clause (c) left on today's key under `pours`: `key:7` |
| Both keys are written whatever the mode | `run.json`, the winner and the prune's `before` have `key` and `pour_key` under both modes | `pour_key` only under `pours` |
| The report names the other key's choice | `other_winner` is the least of the other key, ties to the lower index, a released run included; `null` with no judged run; `same_winner` says whether it is the winner | released runs left out |
| The engine's input is the same | the call writes the same uncounted file under both modes | emptied under `pours` |
| Two keys of one call have one length | `entries_above` on keys of different lengths is a bug and panics in a test build | a silent compare of the shorter |

### Part 2: measurement and the switch

- **After ZR3's runs** (part 2 is built before them, switch off, on the owner's "כן" of
  2026-11-23; see "Four open values"): on ZR3's runs, which carry part 1's record, the
  replay computes the candidate key of every judged run. The table, one row a board: the
  winner by each key, and for each its shape, the four gates, pour faults, each net's kept
  share and
  layer, the area loss, the pour-track step, the signal step, the vias off the pour nets;
  and whether the two keys choose the same run.
- **The line the owner is shown for each board:** the pour key's winner has no more pour
  faults, no higher plane-loss step and no higher area-loss step than today's winner, and
  its signal step is at most one above. Where a board does not hold that, the table says
  which entry and by how much. The word is his.
- **Part 2's own criteria, fixed now:**
  - with the default mode a call's winner is the least of the recorded `key`s;
  - with `pours` in a scratch user config a call's winner is the least of the recorded
    `pour_key`s, and it passes every gate that the same call's `other_winner` passes (one
    call, so the two are compared on the same boards);
  - every recorded `pour_key` equals the replay's from the recorded measures.

  A scratch user config on macOS means a scratch `HOME`, which also moves fastroute's
  default place, so the call names the engine's path.
- No step and no place is tuned to make a board pass.

### Part 2 as built (2026-11-24)

Branch `claude/zr5-pour-key-build`, which holds part 1's branch (and through it
`fork/main` `e07ff839`). Built by Claude Opus 5.5; reviewed by an agent that did not write
it (no must-fix; its items are taken below). The four values are the candidate's: 5 %,
1 %, before the lengths, after the vias. The setting ships at `routing`.

What differs from the skeleton and the tables above, each with its reason:

- **The search's mode** comes from a required `Runner::rank()`, which the production
  runner reads from its judge context. `SearchPlan` holds no judge context, and a
  required method cannot fall to a default. `SearchOutcome` also keeps `rank`, which the
  listing sorts by.
- **The call's mode** is settled by `prepare_ranked_call`: `call_rank_at` of the user
  config, then `prepare_call`. A refusal comes before the search folder is made.
  `prepare_call` alone gives today's key, fixed (the tests' calls).
- **The rank read refuses only what keeps it from reading `routing.rank_key`:** a file
  that does not parse, a `routing` that is not an object, a value that names no mode.
  A bad value of another key refuses only the calls that read that key (the review).
  The refusal is `invalid_configuration` and names the file.
- **`route_judge::tuned_choice`** is the tuned-board choice on its own, so that it has a
  unit test. `pour_rank::TODAY_AT` holds today's thirteen places in the pour key.
- **The prune's functions** (`refusals`, `prune_loop`, `full_judge_refusal`,
  `keys_with`, `replace_winner`) take the mode as an argument; `prune_winner` passes
  `ctx.rank`.
- **`search.rank`** is also in `search.json` and in the no-winner result.
  `same_winner` is `false` when the other key chose no run.

Limits, written down:

- That `judge_candidates` passes the call's mode to `tuned_choice` has no test of its
  own: the judge's test harness has no pour record that splits the two keys.
  `tuned_choice` itself is tested under both modes.
- That `route_candidate` passes `CANDIDATE_RANK` is not tested through a routing call:
  the user config's path comes from `HOME`. The constant's value is tested, as for
  `CANDIDATE_PRUNE`.

## The skill's order of work (PK15; applied once ZR4 and ZR2 are merged)

Today the `kicad-pcb` skill's "Layout Order" routes at step 5 and adds pours at step 6, and
says "Do NOT add copper pours before routing is complete". The change:

- Step 5 becomes the pours: `plan_pours`, the plan shown to the user, `add_zone` once per
  proposal on their OK, `refill_zones`, `save_project`.
- Step 6 becomes the routing: the export, then `route_specctra_dsn`.
- The "do not" sentence goes. In its place: pours go in before the routing export, the
  router connects through them, and the judge records what the routing does to them; a
  board that is routed by hand with `route_trace` may still pour last.
- Rule 10 ("Copper pour last") is rewritten the same way.

It waits for two things: stage ZR4 merged (the tool it names) and stage ZR2 merged (a
router that connects through a pour). The owner answered on 2026-11-23 that it changes
then. It is a PR of its own, and its text goes to him like any other.

## What this design does not do

- **Switch the key on.** PK14.
- **Change the engine's cost.** PK11.
- **Rank ground vias, or pieces.** PK7, PK6.
- **Placement.** PK13.
- **Candidate pour divisions.** PK16.
- **A slot that neither splits the plane nor takes much area.** A long thin slot under a
  fast signal costs little plane area and stays below a 5 % step. The audit's check 3.1
  (a reference plane under critical routes) reads that, inside the copper quality.
- **Pieces before KiCad's island rule,** which is what the engine's guard counts (Z17). It
  would take one more refill a run with the rule off. The area loss sees the same cut from
  the other side.
- **A weight.** The key stays an order of whole numbers; nothing is traded at a rate.
- **Windows and Linux:** macOS first, as the rest of the row.

## Decisions for the owner

1. **The order of the two parts.** Answered 2026-11-23: "כן, ZR5 קודם". The record merges
   before ZR3's measurement.
2. **When is the pour key switched on?** Recommended: part 2 is built after ZR3's table,
   ships off, and is switched on by a one-line change on his word. (The build's timing
   was changed by him the same day: built now, switch off. See "For 2" below.)
3. **When does the skill stop saying "pours last"** (the row's decision 5)? Recommended:
   when ZR4 and ZR2 are merged and ZR2's table shows the as-exported shape at 0 open
   before stitching.
4. **Candidate pour divisions ranked by routing.** The ZR4 design said this "comes with
   ZR5". Recommended: its own stage, after the key is on, because it can only rank with
   that key.

**Answered 2026-11-23, all three as recommended:** "כן, כמו שהמלצת בשלוש ההחלטות". He was
shown them as: the pour key after ZR3's table, on his word; the skill's order once ZR4 and
ZR2 are merged; candidate divisions as their own stage after the key is on.

- For 2: part 2's build still starts on his word, and so does the switch after it. His
  word on the build came the same day, before ZR3's table: asked "לבנות כבר עכשיו את ZR5
  חלק 2, עם המתג כבוי?", he said "כן" (relayed verbatim by the supervisor session). So part
  2 is built now with the four candidate values and ships with the switch at `routing`;
  ZR3's table confirms the values or changes them, and the switch stays his word.
- For 3: ZR2's final matrix already has the as-exported shape at 0 open before stitching on
  every run that passes the gates, on all five boards (the replay of its records). So the
  skill's change waits for the two merges only. It is a PR of its own; its text goes to him
  like any other.
- For 4: stage ZR4's own design, which says the ranking "comes with ZR5", is corrected in
  ZR4's PR.

## Verification of this document

**Read against the source** (`c4c34369`): `route_judge.rs` (`Measures`, `Baseline`,
`baseline`, `judge_one`, `judge_light`, `judge_session`, `judge_candidates`, `measure`,
`gates`, `rank`, `rank_measures`, `LENGTH_STEP`), `zone_fill.rs` (`ZoneLayerFill`,
`PourMeasures`, `pour_fills`, `pour_measures`, `pour_record`, `record_pours`),
`route_search.rs` (`RunRecord`, `release_beyond_the_best`, `finish`), `pour_prune.rs`
(`Checked`, `refusals`, the checker, the full judge's comparison, `keys_with`,
`replace_winner`), `place_judge.rs` (`rank`), `place_route.rs` (`route_candidate`,
`CANDIDATE_PRUNE`), `tools/route_fastroute.rs` (`prepare_call`, `routing_config_at`,
`listed_runs`, `winner_json`, `baseline_json`, the `run.json` writer), `fastroute.rs`
(`uncounted_file_text`), the `kicad-pcb` skill's "Layout Order". KiCad 10.0.1:
`zone_filler.cpp` (the island rule), `pcb_io_kicad_sexpr.cpp` (how a fill piece and its
island flag are saved).

**Measured:** the replay above, on recorded runs. No routing, no build.

**Asked of the session that builds ZR2 and ZR3** (2026-11-23): ZR2 as it will merge changes
none of `Measures`, `PourMeasures`, `Baseline`, `rank_measures`, `uncounted_file_text` or a
ranking site; its one shared file with part 1 is `zone_fill.rs` (`engine_planes_record`, a
different function). ZR3 as designed is the engine's, with a new pin on Konnect's side.
With no uncounted file the engine counts every trace and via that is not user-fixed. The
build starts from the main of its day; whoever merges second keeps both sides, in
`zone_fill.rs`, `lib.rs`, `tools/route_fastroute_tests.rs` and the shared documents.

**Independent review: done, 2026-11-23.** An agent that did not write this (Claude Opus
5.5, high, read only: no build, no KiCad) read the first design (`92d72313`), the skeleton,
Konnect's source and KiCad's. Its verdict: ready with the must-fix items; the structure
holds (a second key beside today's, off by default, and no ranking site mixes a run with a
pour record and one without). Each finding was checked against the source before it was
taken.

| # | Finding | Taken as |
|---|---|---|
| 1, must-fix | KiCad deletes a cut-off island unless something of its net sits on it, so counting extra pieces ranked the run that kept more copper as worse | PK6: the area loss in its place; a test; pieces recorded, not ranked |
| 2, must-fix | "Both keys rank the same runs, so the comparison is exact" was false: the mode decides each run's tuned board and what is pruned | PK10 and PK11 reworded; `other_winner` defined on this call's boards |
| 3 | `search.json` rows have no key today; the plan missed `run.json`, the winner, the prune's `before`, `baseline_json` and `replace_winner` | the two change tables name each |
| 4 | Read "where the effort is read", the setting would be ignored whenever `effort` is passed | PK9: read on every call; a test |
| 5 | The placement tools' routing calls were not addressed | PK13: pinned to today's key by a constant; a test |
| 6 | Equal weights let a small pour count like the ground plane, and a division with more pours was punished | PK4: weighted by area, the best layer by area; PK16 corrected |
| 7 | Totals alone cannot be recomputed, so the criteria could hold with the damage broken | PK18: the rows are recorded; criteria R2 and R3 |
| 8 to 11 | A test asked `Baseline` to deserialize; two tests passed with their guard gone; one criterion compared two separate searches | each corrected in its table |
| Notes | `rank_measures`' helpers need `pub(crate)`; one home for the mode; keys of one length; the island flag; the stitched board; overlapping zones; `1-KOPPER.md`; Z10's "last" | each in its place in this text |

**After the review, on the owner's order:** the stage became two PRs (PK17). The review read
the design as one build; what it found in the damage is part 1's, what it found in the
sites and the setting is part 2's.

**Skeleton compile, part 1** (2026-11-23, in a light lane slot): `cargo check --locked -p
konnect-ipc -p konnect-core` exits 0 with 0 errors and 8 warnings, every one an unused
variable or item of `pour_damage.rs`: nothing calls it yet. 2 bodies are `todo!()`. `cargo fmt
--all -- --check` is clean. Tests were not compiled or run. Record:
`target/creation-research/zone-routing/zr5/plan-check/` in the main checkout.

**Skeleton compile, part 2** (the same day, the commit after part 1's): with `pour_rank.rs`
added the same command exits 0 with 0 errors and 47 warnings, 39 of them unused variables
and items of `pour_rank.rs` and the 8 of part 1. 12 bodies of `pour_rank.rs` are `todo!()`.
`cargo fmt --all -- --check` is clean. Part 1's builder starts from the commit before this
one, which does not have the file.

**Not verified.**

- The damage numbers on a real run: no run has recorded fill areas per layer. Part 1's
  measurement is the first.
- The key's four open values. ZR3's runs and the replay are there for them.
- What ZR3's engine does to the pours. Its numbers are the ones the owner decides on.
- Part 2's user documents (`1-KOPPER.md`, where a user-config setting belongs, and the
  rest) are listed when part 2 is planned in detail, after the numbers.
