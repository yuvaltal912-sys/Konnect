# Pours in the rank: design of stage ZR5 (the pour key)

Status: a design down to function skeletons, 2026-11-23. Nothing is built: every body is
`todo!()`. Stage ZR5 of Row ZR ([ZONE_ROUTING_DESIGN.md](ZONE_ROUTING_DESIGN.md), the stage
table and Z10). Designed in a Claude Fable 5.2 session on branch `claude/zr5-pour-rank-plan`,
from `fork/main` `c4c34369`. **Not yet reviewed** by an agent that did not write it (see
"Verification of this document"). It needs nothing of stage ZR4's code and touches no engine
file.

## What the owner said, and what is still his

- He started this design on 2026-11-23, in the session that designed ZR4: "לאחר שהוא יסיים
  וידחף את התוכנית שלו הוא ימשיך לתכנון של ZR5 באותה מתכונת" (relayed verbatim by the
  supervisor session).
- The row's stage table gives ZR5 as "Pour measures in the rank key; the skill's order of
  work", done on "the owner's word, on ZR3's numbers". Z10: "The key reproduces a measured
  order. Changing it needs its own numbers."
- **Three decisions are his and are not taken here** ("Decisions for the owner" below): when
  the new key is switched on; when the skill stops saying "pours last" (the row's decision
  5); and whether ranking candidate pour divisions by routing is its own stage.

## The short answer

1. **Today's key is not touched.** A second key, the **pour key**, is computed beside it for
   every judged run: today's thirteen entries in today's order, with four entries added that
   say what the routing did to the pours.
2. **Pour damage is measured against KiCad's own fill of the board before it was routed.**
   Three numbers: fills that vanished, how much of each pour net's best plane is still one
   piece, and pieces that were not there before.
3. **One setting says which key ranks**: `routing.rank_key` in the user config, `routing`
   (today's key) or `pours`. **The default is `routing`**, so nothing changes until the
   owner says so.
4. **Both keys are recorded on every run whatever the setting**, and every call says which
   run the other key would have chosen. His own boards then show what the switch would
   change, before it is made.

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
matrices: ZR1's (fastroute x20) and an earlier ZR2 matrix (engine build `2d559bc2`, not the
final one). It reads files only. Record:
`target/creation-research/zone-routing/zr5/design-replay/` in the main checkout.

- **The replay computes today's key exactly:** all 3,086 recorded keys of ZR1's matrix and
  all 2,801 of the earlier ZR2 matrix, all thirteen entries of each. It is the second, independent
  computation the measurement below uses.
- **A pour term changes winners, and where it sits decides how often.** With ZR0's recorded
  measures (starved thermals, pour layers with no fill, open before stitching, pieces, the
  largest piece's share) placed as a block:

  | Where the block sits | ZR1 matrix: boards whose winner changes | Earlier ZR2 matrix |
  |---|---|---|
  | Before the lengths | 3 of 5 | 2 of 5 |
  | After the right angles, before the vias | 2 of 5 | 1 of 5 |
  | Last, before the seconds | 0 of 5 | 0 of 5 |

- **A raw piece count is not a safe measure.** In the earlier ZR2 matrix, on one of the
  owner's boards, the runs that route ground as track (pours omitted) have 10 to 13 fill
  pieces and the runs that route through the pour have 13 to 16. Ranked on pieces first,
  the winner there becomes a run with the ground routed as track. Which shape cuts a pour
  less is an engine question that ZR3 works on. The key must measure the cut itself and
  must not prefer a shape.
- **On the two small public fixtures the pours are never cut** (`ecc`: 1 piece in all
  1,366 judged runs of the earlier ZR2 matrix; `pours`: 2 to 5). There the only difference a
  pour makes is the ground track: 0 to 12 mm through the pour, 88 to 143 mm with the pours
  omitted, and today's key cannot see it.
- **What the recorded runs cannot show:** the three damage numbers of this design. They
  need fill areas per layer, before and after routing, which no run recorded. Recording
  them is the first thing the build does.

## Decisions (the code cites them as PK1 to PK16)

| # | Decision | Reason |
|---|---|---|
| PK1 | Today's key is not touched: `Key`, `KEY_LEN`, `KEY_QUALITY`, `rank` and `rank_measures` stay byte for byte. The pour key is a second key beside it | Z10. "Off" is then today's ranking exactly, and every existing test of the key keeps guarding it |
| PK2 | The pour key is today's thirteen entries in today's order, with four added: pour faults and the plane-loss step before the lengths; the extra pieces and the pour-track step after the vias. No existing entry changes its meaning | One standard for a board with pours and a board without: with no pour net the four are 0, so the order is today's. The existing entries are a measured order (D12), and inserting leaves their order among themselves intact |
| PK3 | Pour damage is measured against KiCad's own fill of the board before it was routed (the judge's baseline, refilled by kicad-cli) | The unrouted fill is what the routing had to keep. A zone that is two pieces by design costs nothing. KiCad's refill is the truth of the row (P1) |
| PK4 | **Plane loss.** For each pour net and each of its layers: the largest fill piece after routing over the largest before, at most 1. The net's intact share is its best layer's. The loss is 1 minus that, summed over the pour nets, in steps of 5 % | The checklist's "keep one side largely continuous": one whole reference plane is what a return path needs, so the best layer counts. One number sees a split (the largest piece shrinks to a part) and a slot or lost island (it loses area). Coarse steps, so that planes that are nearly equal tie and the lengths decide |
| PK5 | **Pour faults** are KiCad's `starved_thermal` violations on the judged board plus the pour layers that had fill before routing and have none after. One entry, a count, before the plane loss | Each is a spot where a pour stopped doing its job. KiCad calls a starved thermal an error by default. Gate G2 only holds it to the unrouted board's count |
| PK6 | **Extra pieces** are, per pour layer, the pieces after routing beyond the pieces before, never below 0, summed. The entry sits after the vias | The guard ZR3 gives the engine's search is "never more pieces" (Z17), so the judge rewards what the engine protects. Late, because a fragment of the component-side fill matters less than a length step or a via, and the replay shows a raw piece count misleads when it leads |
| PK7 | **The pour-track step** is the track length on pour nets in units of 2 % of the shortest signal length of the pool (`LENGTH_STEP`). It sits after the extra pieces, before the copper quality. Vias on pour nets are not in the key | A ground track that the pour already connects is copper the board does not need, as the owner's own rule says of ground vias (ZR0b). Under a pour it harms nothing electrically, so it breaks ties and never outranks a signal measure. The unit is the signal length because a board's own scale is the fair one and stubs of a few mm then tie. Ground vias are the prune's: it removes the redundant ones from the winner, so a count before the prune would rank vias that are not delivered |
| PK8 | A judge that took no pour record gives 0 in entries 7, 8 and 13: a call with no pour net, and the prune's light judge. A run whose pour record was tried and failed, or has no damage, gets the largest number there | The first keeps today's order on a board without pours. The prune's light judge (`judge_light`) takes no pour record for any of the three boards it compares, so 0 on all three ranks nothing there; its loop guards the pours with its own clauses (no rule's count higher, no pour layer's fill lower), and its final comparison is of two full judges. The second is the key's own rule ("a measure the audit did not give sorts last"): an unmeasured run must not outrank a measured one. It is still not a gate: a failed pour measure never fails a run (ZR0) |
| PK9 | One setting says which key ranks: the user config's `routing.rank_key`, `routing` or `pours`, read once per call. The default is `routing`. There is no tool argument | It is the owner's standing choice, like `routing.effort`, not something an AI picks per call. A measurement sets it in a scratch config |
| PK10 | Both keys are computed and recorded for every judged run, whatever the setting. Every call's result says which key ranked and which run the other key puts first among the same judged runs | Z10 asks for numbers. This gives them on every real call at no cost: the judge already has the measures. The owner sees on his own boards what the switch would change before he makes it |
| PK11 | The engine's input does not depend on the setting. The uncounted file (`--x.gs_uncounted`, Z20) stays as it is | The pour track is a tie-break in the pour key, not a cost the engine should buy with signal length. With one engine input both keys rank the same runs, so PK10's comparison is exact. Z20's "the uncounted set follows the key" is left for a stage that has numbers for it |
| PK12 | Everything that ranks goes through one function with the setting: the search's winner and the runs it releases, the choice among a run's tuned candidates, the prune's "no entry higher" clause and its final comparison, and the result's listing | One standard with the search (the prune's own reason, P5). A site left on the other key would keep or drop what the winner's key does not |
| PK13 | The placement key is not changed: it keeps copying today's quality entries | In the order of work placement comes before the pours, so a new board's candidates are routed without any, and the four entries would be 0. A board that is placed again with its pours in gets no pour term in the placement rank: a limit, written down |
| PK14 | The build ships with the setting at `routing`. Switching the default is its own one-line change, on the owner's word | The stage's done criterion is his word on ZR3's numbers. The build can be merged, and can record, long before that |
| PK15 | The skill's order of work is designed here and applied on the owner's word (the row's decision 5): pours are planned and added before the routing export | It names `plan_pours`, so it follows ZR4's merge; and it is only right once the router connects through a pour (ZR2) |
| PK16 | Ranking candidate pour divisions by routing each one is not in this stage | It can only rank with the pour key, which is off until the owner's word. The key is built so that it can: damage is measured against each candidate's own unrouted fill, so a division with more pours is not punished for having them |

## The pour key

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
| **13** | **Extra pieces** | new (PK6) |
| **14** | **Pour-track step**: `round(pour_track_mm / shortest signal / 0.02)` | new (PK7) |
| 15 | Minus copper quality | today 11 |
| 16 | Route seconds | today 12 |

- **The pool is today's.** The shortest signal length of entry 14 is the one today's entry 7
  steps against: among the runs in the pool that pass G1, else the pool, else every run.
  When no run of it has signal track, entry 14 is 0 for all.
- **The step and the place of entry 8 are the two values the measurement is there to
  confirm.** They are fixed here so that a run cannot tune them; if the owner's boards show
  them wrong, the design is changed first and the measurement run again.

## Pour damage

```text
before   the judge's baseline: the saved board, refilled by kicad-cli, before any routing
after    the judged board: routed, refilled, stitched (the board ZR0's measures read)

per pour net and copper layer, over every zone of the net on that layer:
    pieces, the largest piece's area, the total area

lost_fills    layers with fill before and none after
plane_loss    sum over pour nets of  1 - max over layers( min(1, largest after / largest before) )
              a layer with no fill before is not a layer of the net here;
              a net with no fill before on any layer adds nothing
extra_pieces  sum over layers of  max(0, pieces after - pieces before)
```

- **An undamaged board scores 0, 0.0 and 0**, whatever its zones look like.
- **Where it lives:** `PourMeasures::damage` (new, optional), beside ZR0's measures, so a
  recorded run of before this stage still reads. `Baseline::pour_fills` (new, optional)
  holds the "before" rows.
- **Recorded only, as ZR0's measures are:** a fill that cannot be read never fails the
  baseline or a run. The damage is then absent, and PK8 says what the key does.
- **Per net for the reader:** the damage also names each net's best layer and intact share,
  so a result says "GND: 93 % of the back plane is one piece", not only a step.

## The setting, and what every call reports

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
  the other key puts first among the judged runs (a released run included; `null` when no
  run was judged).
- Every listed run and every row of `search.json` has `key` (today's, as now) and
  `pour_key` (new). The winner's measures carry `pours.damage`.
- `routing.rank_key` is read where `routing.effort` is (`tools::route_fastroute::
  routing_config_at`). A value that is not one of the two names refuses the call and names
  both, as a bad `routing.effort` does. `save_user_config` writes any key path, so no list
  of known keys changes.
- **What the setting changes:** which run wins, which judged runs the search keeps whole,
  which tuned candidate a run keeps, and the key the prune's clause (c) reads. Nothing else.

## Modules

New, in `crates/konnect-core/src/`:

| File | What |
|---|---|
| `pour_rank.rs` | Pure: the layer fills, the damage, the pour key, the setting, the ranking every site calls, the report |
| `pour_rank_tests.rs` | Its tests |

Changes to existing items, all made by the build (the skeleton changes none of them):

| Where | Change |
|---|---|
| `route_judge.rs` | `Baseline` gains `pour_fills: Option<Vec<LayerFill>>`, read from the baseline's refilled scratch board. `JudgeContext` gains `rank: RankMode`. `judge_candidates` ranks through `pour_rank::ranking_of`. The pool and the step of `rank_measures` become two private helpers that the pour key calls too; `rank_measures` returns what it returned |
| `zone_fill.rs` | `PourMeasures` gains `damage: Option<PourDamage>` (`#[serde(default, skip_serializing_if = "Option::is_none")]`). `pour_measures` and `pour_record` take the baseline's rows and fill it from the fills they already read |
| `route_search.rs` | `SearchPlan` carries the setting. `release_beyond_the_best` and `finish` rank through `pour_rank`. `SearchOutcome` gains `pour_keys` beside `keys` |
| `pour_prune.rs` | Clause (c), the full judge's final comparison and `keys_with` read the ranking key of the setting. The position in a `key:<n>` refusal is the ranking key's |
| `tools/route_fastroute.rs` | `routing_config_at` reads `routing.rank_key`. `listed_runs` sorts by the ranking key. The result gains `search.rank` and each row's `pour_key` |
| `lib.rs` | The new module |

Not touched: `place_judge.rs` (PK13), `fastroute.rs` (PK11), `length_probe.rs`,
`scratch_board.rs`, the fastroute pin, any engine file, anything of stage ZR4.

**Shared with stages ZR2 and ZR3** (another session): `route_judge.rs`, `route_search.rs`,
`pour_prune.rs`, `tools/route_fastroute.rs` and `zone_fill.rs`. The build starts from the
main of its day and is coordinated with that session; whoever merges second keeps both
sides.

## The skeleton

Code on this branch, in `pour_rank.rs`; the doc comments are the specification.

| Item | What |
|---|---|
| `PLANE_STEP`, `POUR_KEY_LEN`, `PourKey`, `POUR_FAULTS`, `PLANE_LOSS`, `EXTRA_PIECES`, `POUR_TRACK` | The step, the key's type, the four new positions |
| `LayerFill`, `layer_fills` | One pour net's fill on one layer, from the judge's own `pour_fills` rows |
| `NetPlane`, `PourDamage`, `pour_damage` | The damage of "before" and "after" |
| `plane_step`, `pour_track_step` | The two steps |
| `pour_keys` | The pour key of every run at once |
| `RankMode`, `RankSource`, `resolve_rank_mode` | The setting |
| `ranking`, `ranking_of`, `entries_above` | What every ranking site calls |
| `rank_report` | `search.rank` |

## Tests, each guard with a negative control

A control breaks the guard in the source, sees its test fail, and restores the file.

### The damage

| Guard | Test | Negative control |
|---|---|---|
| An undamaged board scores nothing | "after" equal to "before", with a zone that is two pieces by design: 0, 0.0, 0 | the total area as the denominator: a loss appears |
| The best layer counts | front cut to half, back whole: loss 0. Both cut, to 60 % and 80 %: loss 0.2 | the worst layer: 0.5 and 0.4 |
| The nets are summed | two nets at 0.1 and 0.2: 0.3 | the largest only: 0.2 |
| A piece that grew is not a gain | the largest piece larger after than before: loss 0, not below | the clamp removed: a negative loss |
| A fill that vanished is a fault | fill before, none after, on the net's only layer: 1 lost fill and a loss of 1.0. With a second layer that is whole: 1 lost fill, loss 0 | read as intact: no fault |
| Pieces are counted per layer and never negative | 2 before and 5 after: 3. 3 before and 1 after: 0. Both layers together | a signed sum: the second cancels the first |
| Several zones of a net on a layer are one fill | two zones: their areas add, the largest piece is the largest of both | per zone: two rows |
| A net with no fill before adds nothing | no fill before on any layer: loss 0, no lost fill | counted as wholly lost: 1.0 |

### The key

| Guard | Test | Negative control |
|---|---|---|
| Today's key is today's | `ranking(Routing, runs)` equals `route_judge::rank_measures(runs)` entry for entry, on runs that differ in every measure | an entry swapped in that arm |
| Today's entries keep their order in the pour key | each of the thirteen at its place: 0 to 6, then 9 to 12, 15, 16 | a place shifted by one |
| No pour record, today's order | runs with no pour measure and no pour error (a call with no pour net; the prune's light judge): entries 7, 8 and 13 are 0. With no track on a pour net either, the two keys sort the runs alike | the largest number written there: the entries differ |
| An unmeasured run does not outrank a measured one | two runs alike, one with `pours_error`: it sorts after | 0 written for it: a tie |
| Faults, then the plane, then the lengths | pairs that differ in one entry each; and one starved thermal more against two length steps fewer: the starved run loses | two places exchanged |
| The plane step rounds as the lengths do | 0.024 is 0; 0.025 is 1; 0.074 is 1; 0.076 is 2 | rounded down: 0.025 is 0 |
| The pour track is in signal units | the shortest signal 1000 mm: 9 mm is 0, 10 mm is 1, 400 mm is 20. No signal track in the pool: 0 for all | the shortest pour track as the unit: 9 mm is 50 |
| The pool is today's | a run outside the pool, and one that fails G1, do not set the unit | every run sets it |

### The setting and the sites

| Guard | Test | Negative control |
|---|---|---|
| The default is today's key | no `routing.rank_key`: `Routing`, from the default | `Pours` as the default |
| A value that is not a name is refused | `"pour"`, `3`, `""`: an error that names `routing.rank_key` and both names; `null` is "not set" | a silent fall to the default |
| Each site follows the setting | a scripted pool on which the two keys disagree: the search's winner, the runs it releases, a run's tuned choice, the prune's final comparison, and the listing each follow the key of the setting, under both settings | per site: left on `route_judge::rank`, its case fails |
| The prune's loop under `pours` | three light measures of a call with pours, the candidate's pour track longer by a step: clause (c) refuses it as `key:14`; under `routing` it does not. Entries 7, 8 and 13 are 0 on all three | clause (c) left on today's key under `pours`: the candidate is kept |
| Both keys are recorded whatever the setting | `search.json` rows and the result's runs have `key` and `pour_key` under both settings | `pour_key` only under `pours` |
| The report names the other key's choice | `other_winner` is the least of the other key, ties to the lower index, a released run included; `same_winner` says whether it is the winner | released runs left out |
| The engine's input is the same | the uncounted file's text under both settings | emptied under `pours` |

### The recording

| Guard | Test | Negative control |
|---|---|---|
| "Before" is the unrouted board's fill | the baseline of a fixture with a pour: its rows are the fills of its refilled scratch board | read before the refill: no rows |
| A fill that cannot be read fails nothing | an unreadable baseline board: `pour_fills` is absent, the baseline stands, runs are judged, their damage is absent | the error passed up: the judge fails |
| An older record still reads | measures without `damage`, and a baseline without `pour_fills`, deserialize | the `serde(default)` removed |
| The tool guards hold | no new tool and no new parameter: `doc_tool_counts` and `schema_parameter_usage` pass unchanged; `asset_references` passes with the skill's new words | (they are the repo's own guards) |

## Measurement: criteria fixed before any run

**Boards:** the five of "The x19 baseline" (dash, B5, `pic`, `ecc`, `pours`), each as it is,
with its own pours. **Build:** the branch's release binary by pipe, the fastroute that
`packaging/1-kopper/fastroute.json` pins on the day, effort M. Each routing call in its own
routing hold, under the lane locks.

| Step | What | Criterion |
|---|---|---|
| M1 | One `route_specctra_dsn` call a board, the setting left at its default | R1: `search.rank.ranked_by` is `routing`, and the winner is the least of the recorded `key`s |
| M2 | The same call's records | R2: every judged run has a `pour_key` with no "unmeasured" entry (7, 8, 13). A run that has one is listed with its `pours_error` |
| M3 | The offline replay, a second computation of both keys from each run's recorded measures and the baseline's rows | R3: every recorded `key` and `pour_key` equals the replay's |
| M4 | A second call a board with `routing.rank_key = "pours"` in the scratch config | R4: `ranked_by` is `pours`, the winner is the least of the recorded `pour_key`s, and it passes the gates M1's winner passes |
| M5 | The table below, from M1 | R5: it exists for all five boards. It is evidence, not a pass |

- **The table (M5), one row a board:** the winner by each key among M1's judged runs. For
  each: its shape, the four gates, pour faults, each pour net's intact share and best layer,
  extra pieces, the pour-track step, the signal step, the vias off the pour nets. And
  whether the two keys choose the same run.
- **Done for the build when** R1 to R4 hold on all five boards, the table exists and the
  gate is green. The key is still off.
- **Not a criterion of the build: switching on.** What the owner is shown for that is the
  same table on ZR3's engine, with one line a board: the pour key's winner has no more pour
  faults, no higher plane-loss step and no more extra pieces than today's winner, and its
  signal step is at most one above. Where a board does not hold that, the table says which
  entry and by how much. The word is his.
- A criterion that fails is reported with its cause. No step and no place is tuned to make
  a board pass. Results of the owner's boards stay out of tracked files, apart from counts.
  Records go to `target/creation-research/zone-routing/zr5/` in the main checkout.

## The skill's order of work (PK15; applied only on the owner's word)

Today the `kicad-pcb` skill's "Layout Order" routes at step 5 and adds pours at step 6, and
says "Do NOT add copper pours before routing is complete". The change, when he says so:

- Step 5 becomes the pours: `plan_pours`, the plan shown to the user, `add_zone` once per
  proposal on their OK, `refill_zones`, `save_project`.
- Step 6 becomes the routing: the export, then `route_specctra_dsn`.
- The "do not" sentence goes. In its place: pours go in before the routing export, the
  router connects through them, and the judge ranks what the routing does to them; a board
  that is routed by hand with `route_trace` may still pour last.
- Rule 10 ("Copper pour last") is rewritten the same way.

It waits for three things: stage ZR4 merged (the tool it names), stage ZR2 merged (a router
that connects through a pour), and his word.

## Documents

In the build's PR:

- This file gains "As built" and "Measured".
- `docs/API_MIGRATIONS.md`: one entry. `search.rank`, each run's `pour_key`, the measures'
  `pours.damage`, the baseline's `pour_fills`, and the user config's `routing.rank_key`.
  Nothing is removed or renamed.
- `docs/ROUTE_SEARCH_DESIGN.md`: a line under D12 that points here.
- `route_specctra_dsn`'s description and the skill's "Best-of-N routing": one sentence on
  `search.rank`. The words the skill then quotes go into `NOT_TOOLS` of
  `crates/konnect/tests/asset_references.rs`.
- `tool-directory.md`, `README.md`, `DEV.md`: only where they describe the key or the user
  config. No count changes: there is no new tool.
- `docs/ZONE_ROUTING_DESIGN.md` and `docs/CREATION_HANDOFF.md`: where the stage stands.

## What this design does not do

- **Switch the key on.** PK14.
- **Change the engine's cost.** PK11. A pour net's length and vias stay out of the group
  search's cost.
- **Rank ground vias.** PK7. The prune removes the redundant ones from the winner.
- **The placement key.** PK13.
- **Candidate pour divisions.** PK16.
- **A slot that neither splits the plane nor takes much area.** A long thin slot under a
  fast signal costs little plane area and stays below a 5 % step. The audit's check 3.1
  (a reference plane under critical routes) reads that, inside the copper quality.
- **A weight.** The key stays an order of whole numbers; nothing is traded at a rate.
- **Windows and Linux:** macOS first, as the rest of the row.

## Decisions for the owner

1. **When is the pour key switched on?** Recommended: the build ships with it off and
   records both keys on every call. It is switched on by a one-line change after ZR3's
   table, on his word.
2. **When does the skill stop saying "pours last"** (the row's decision 5)? Recommended:
   when ZR4 and ZR2 are merged and ZR2's table shows the as-exported shape at 0 open
   before stitching, on his word.
3. **Candidate pour divisions ranked by routing.** The ZR4 design said this "comes with
   ZR5". Recommended: its own stage, after the key is on, because it can only rank with
   that key.

## Verification of this document

**Read against the source** (`c4c34369`): `route_judge.rs` (`Measures`, `Baseline`,
`baseline`, `judge_one`, `judge_candidates`, `measure`, `gates`, `rank`, `rank_measures`,
`LENGTH_STEP`), `zone_fill.rs` (`ZoneLayerFill`, `PourMeasures`, `pour_fills`,
`pour_measures`, `record_pours`), `route_search.rs` (`release_beyond_the_best`, `finish`),
`pour_prune.rs` (`refusals`, the full judge's comparison, `keys_with`), `place_judge.rs`
(`rank`), `tools/route_fastroute.rs` (`routing_config_at`, `listed_runs`, `winner_json`),
`fastroute.rs` (`uncounted_file_text`), the `kicad-pcb` skill's "Layout Order".

**Measured:** the replay above, on recorded runs. No routing, no build.

**Skeleton compile** (2026-11-23, in a light lane slot): `cargo check --locked -p konnect-ipc
-p konnect-core` on this branch exits 0 with 0 errors and 43 warnings, every one an unused
variable or item of `pour_rank.rs`: nothing calls it yet. 13 bodies are `todo!()`. `cargo fmt
--all -- --check` is clean. Tests were not compiled or run. Record:
`target/creation-research/zone-routing/zr5/plan-check/` in the main checkout.

**Independent review:** not done yet. It runs before the plan is pushed.

**Not verified.**

- The three damage numbers on a real run: no run has recorded fill areas per layer.
- That 5 % is the right step, and that before the lengths is the right place, for the
  plane loss. The measurement is there for both.
- What ZR3's engine does to the pours. Its numbers are the ones the owner decides on.
