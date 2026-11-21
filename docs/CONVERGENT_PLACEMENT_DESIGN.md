# Convergent placement search: design

Status: approved, and being built as Row CP (see [`CREATION_HANDOFF.md`](CREATION_HANDOFF.md) for
where the build stands). Written on 2026-11-20 on branch `claude/convergent-placement-design` from
`fork/main` `0a3fc504`, by the session the owner asked ("כן, תכתוב"), which runs on Claude
Fable 5.2. The standing rule gives algorithm design to Opus 5.5; here the owner asked this session
directly. The owner approved its three decisions as recommended the same day ("כן, כמומלץ"), and
then started the build ("תתחיל בבניית ההצבה") with Opus 5.5 (high) agents writing the bodies, as
he asked. The text below is the design as it was approved, with two decisions amended after the
first measurement (C4's attraction grid and C5's first temperature: see "Amendments after the
first measurement"), one measurement question given a paired reading (A3, in the same section)
and three rows of the tests table corrected. The answers measured so far are under "The answers
on the owner's board". No board of his was written.

## The design on one page

- **The problem.** Today's placement search makes a fixed list of candidates and routes the
  best few. No step reads a result and improves on it, and nothing says when to stop except the
  list's end and a time limit.
- **The answer.** One number, the cost of a placement, which is cheap to compute. A search
  takes the candidates the generators already make, moves parts while the cost falls, and stops
  when it no longer falls. Only the result is routed.
- **Three stop rules, all read the cost and nothing else.** A run stops when its best cost has
  stopped falling (C6). The search stops taking new starts when new starts no longer find a
  better cost, and stops doubling its effort when a doubling no longer pays (C7). There is no
  time limit and no run count; guards exist against a search that never settles, and a result
  they end is marked as not converged.
- **Where it goes.** A fourth family, C, in the placement bench that exists (Row PS). It is
  judged like F and S, against the owner's own placement, by the same routing calls. No tool, no
  write to a board of his: measure first, as owner decision 4 of
  [`PLACEMENT_SEARCH_DESIGN.md`](PLACEMENT_SEARCH_DESIGN.md) says.
- **What was measured before this design.** A throwaway experiment on a copy of the owner's
  board: the converged placements routed 22 to 24 % shorter than his own placement, passed
  every gate, and ranked first in the bench (tables below).
- **What it does not give.** One global best. The experiment showed that the search refines a
  start and does not find a good placement from any start: where a run ends depends on where it
  began. So the search starts from the best candidates the generators make, and the rules say
  when more starts and more effort stop paying. They do not prove an optimum.

The owner approved the three decisions as recommended on 2026-11-20: see "Decisions for the owner".

## What the owner asked for

On 2026-11-20: the routing search and the placement search work "stupidly"; propose an
algorithm that converges to a value, so that we know when to stop, and not by time or by a number
of routings. He then approved a throwaway experiment, and after its result this document.

This document covers placement. Routing has its own design.

## What was measured

Counts only: the board is the owner's (101 parts, 12 of them locked, 89 movable; 63 % of the
board area is courtyards). All of it is on copies, in `target/creation-research/`, which git
ignores.

**From the records of the search as it is** (26 routing calls and 480 generated candidates of
this board):

| Finding | Number |
|---|---|
| Generated candidates that reach the owner's airwire length | 0 of 480 (best 1.17x, median 1.7x to 2.4x) |
| How well the cheap airwire length predicts the routed signal length | rank correlation 0.92 to 0.97 |
| One placement evaluation against one routing call | about 3 ms against 240 s |
| Routing runs of one placement that pass every gate differ in length by | 1 to 3.5 % |

So the cheap number tells which placement routes shorter, it is 80,000 times cheaper than a
routing call, and the generators never look at it while they work.

**The experiment** (a Python model of the cost, checked against `score_placement`: the same
airwire length and crossings on 9 boards, to 4 decimals). Cost = airwire length + 4 mm per
crossing + the decoupling pairs. Hard rules: locked parts do not move, courtyard boxes keep
0.3 mm from each other and 0.5 mm from the outline. Every run below ended by its stop rule.

| Start | Search | Runs | Airwire, mm | Crossings | Broken decoupling pairs |
|---|---|---|---|---|---|
| the owner's placement itself | none | | 1220 | 128 | 0 |
| the owner's placement | descent only, 2 to 4 s | 1 | 1035 | 67 | 0 |
| the owner's placement | annealing, then descent | 2 seeds | 889 to 900 | 28 to 30 | 0 |
| the generated candidate with the best start | annealing, then descent | 3 seeds | 943 to 995 | 27 to 34 | 0 |
| 7 other generated candidates (F and S) | annealing, then descent | 9 | 1020 to 1468 | 41 to 163 | 0 in 8 runs, 2 in 1 run |
| 8 random legal starts | annealing, then descent | 8 | 986 to 1470 | 29 to 131 | 0 |

A run at the base effort took 115 to 193 stages and 0.5 to 0.97 million evaluations, 9 to 19 minutes in Python.
Two of the results were written to a board with Konnect's own tools and scored by the real
`score_placement`: verdict pass, no hard failure, the same airwire length and crossings as the
model said.

**Routed, in the bench that exists** (effort M, the same judge for all):

| Placement | Calls | Every gate | Signal length, mm | Vias off pours | Copper quality | Silk entries | Rank |
|---|---|---|---|---|---|---|---|
| the owner's | 3, identical | pass | 1738.0 | 82 | 59.7 | 109 | 2 to 4 |
| converged, from the best generated start | 1 | pass | 1326.3 (0.76x) | 81 | 61.4 | 160 | 1 |
| converged, from the owner's placement | 1 | pass | 1348.6 (0.78x) | 64 | 60.7 | 175 | 1 |

The airwire ratio of the placement from the generated start was 0.77 and its routed ratio came
out 0.76.

**What the experiment showed about the search itself:**

- **The start decides much of the end.** From 8 random starts the cost at the end ran from
  1228 to 2074 (69 % apart). From the 8 generated candidates that were at hand, taken in the
  order of their start cost, the first gave the best end (1195) and the others 1255 to 2201 (3260 for the
  run that ended with two broken pairs); the rank correlation between start cost and end cost was 0.62. Between seeds of one start the end
  differed by 1.5 to 6.6 %.
- **More effort still pays, and it has not yet been seen to stop paying.** Three starts were
  run at effort levels 0, 1 and 2 (each level cools twice as slowly), one seed each. Cost at
  the end:

  | Start | Level 0 | Level 1 (2x) | Level 2 (4x) |
  |---|---|---|---|
  | the best generated start | 2212 (1138 without its one broken pair) | 1208 | 1124 |
  | a random start | 1241 | 1139 | 1109 |
  | another random start | 2046 | 1994 | 1919 |

  Each doubling lowered the cost by 2.5 to 8 %, with no flattening within 4 times the effort
  (0.7, 1.4 and 3.0 million evaluations). Two cautions. Seeds of one start at level 0 already
  differ by 6.6 %, and the best of three (1121) equals the level-2 run of the same start, so
  one run per level is a noisy reading. And a bad start stays bad: at 4 times the effort the
  worse random start was still 70 % above the other two. A first ladder was wrong at its
  slowest level (the move radius grew with the stage count) and is not counted.
- **A requirement can stay broken.** Two of the 25 runs at the base effort, from two starts,
  ended with decoupling capacitors out of range of their IC (2 of them and 1), though their
  airwire length was among the best: the cells near the IC were taken, and no single move
  frees one.
- **Silk got worse:** 109 entries became 160 and 175 (labels over a neighbour's pads: 52 became
  85 and 97; labels over other silk: 51 became 69 and 70). The cost did not see labels.
- **4 or 5 parts moved away from their schematic neighbours** (the measure of D23), though the
  mean distance stayed the same (0.997x and 1.011x). A part that sits only on power nets has
  nothing in the cost that pulls it anywhere.
- One board, and one routing call for each converged placement. Not yet a general result.

## The flow of one search

```
phase 1b of place_search, after the candidates of F and S exist     (family C; the rest is unchanged)
 1. the cost model, built once: the score's nets, the decoupling pairs, the movable
    connectors, the schematic neighbourhoods, the silk labels, the legality rule
 2. the starts: the candidates of F and S that pass P1, the lowest start cost first
 3. start by start, each a run at effort level 0, on its own thread:
      make the start legal under C's gap rule
      anneal     stages at a falling temperature, until the inner rule says stop
      repair     make room for a decoupling capacitor that is out of range
      descend    sweeps, until no single move lowers the cost
      -> a candidate, its cost term by term, its trace; a record like any candidate's
    the rule for starts, read in start order: stop when 3 starts in a row did not pay
 4. the best start again at effort level 1, 2, ... (each level cools twice as slowly);
    the rule for effort: stop at the first level that did not pay
 5. the bench goes on as today: screen, selection (C's best 3 and 1 drawn), routing calls,
    the judge, search.json and table.md
```

Nothing is written outside the bench folder. Family C never sees where the owner put a movable
part: its starts are F's and S's candidates, which know what S0 knows.

## Decisions taken in this design (with the reason)

| # | Decision | Reason |
|---|---|---|
| C1 | Family C is built into the bench of Row PS: a fourth family beside F, S and P, named `converge` in `bench.json`'s `families` (the default stays F and S). It needs at least one of F and S named with it, and is refused without. C's records come after every record of F, S and P (their ids and their draws in the selection do not change), are checked for duplicates like any candidate's, and get the schematic measure in the same pass. No tool, no registry change, no apply path | Owner decision 4 of the placement-search design: measure first, then the tool. The same references, routing calls and judge make C's result comparable with everything measured so far. C's starts are F's and S's candidates (C8), so a search without them has nothing to refine |
| C2 | **One cost, in mm; less is better.** `cost = L + 4 X + P + Q + 0.1 N + 0.2 S`. **L**: the airwire length the score measures (the Euclidean spanning tree over the pad sites of each net with 2 to 8 distinct references), plus 0.3 times the same length of each net above 8 references that has no pour on the board; a rail with a pour counts nothing. **X**: the score's proper crossings between the trees of different eligible nets. **P**: for each decoupling pair, the distance d between its two pads, plus R + 50 (d - limit) when d is past the pair's limit. **Q**: for each connector (reference prefix `J`, the score's own rule) whose box centre is farther than 10 mm from the outline, R + 50 times the excess. **R**, what a broken requirement costs, is the larger of 1000 mm and the soft cost (everything but P's and Q's R terms) of the run's own start. **N**: for each part that has schematic neighbours (D23's neighbourhoods, at most 3 parts), its mean centre distance to them, summed. **S**: the number of label conflicts between parts that share a side: a visible silk text of one part whose box lies over another part's courtyard box, or over a silk text of another part | L, X, P are the experiment's cost, unchanged, and L and X are the score's own numbers, so the search and the screen cannot disagree about a placement. 4 mm per crossing and 0.3 for rails are the values the experiment ran with; they were not tuned. The rank key puts decoupling and connector violations before every routing measure (D18, fields 6 and 7), and the cost must order the same way: R is at least the whole soft cost the run starts with, so no placement better than the start can pay for a broken requirement with length. The experiment ran with a fixed 1000 mm, which was below its board's airwire length (1220 mm and more); on that board the runs repaired the pairs all the same, and R is the rule that does not depend on the board's size. A pair that cannot be measured (a pad missing, or on its part twice) counts as broken in every state, as the screen counts it, and changes no move. Q mirrors the `connector_edge` deduction and counts what it counts, held connectors included (theirs is a constant); the experiment's board never met it (no unlocked connector far from an edge) and other boards will. N and S are new, for the two faults the experiment showed. Their weights make them tie-breakers: N pulls a part with a fifth of the force of one two-pin net and with the same order as a rail, so it decides only for a part that nets do not place (the indicator LED on power nets, D23's own example); S is a stand-in for KiCad's three silk checks, which the search cannot run: a neighbour's pads and silk outline lie inside its courtyard box, so a label over that box is a label over its pads or its outline, and a label over a label is the third case. It costs 0.2 mm, so 50 label conflicts weigh 1 % of the airwire length, less than one routed length step, which is where silk stands in the rank key (field 14, after every routed measure) |
| C3 | **Hard rules, not costs.** A move is made only if every part it moves is legal after it, by the test that legalizes F and S today (`legalize_turned`'s `legal_at`, pulled out as `Legality`): inside the true outline by `edge_mm`, clear of every part that shares a side with it, clear of the board's copper obstacles. One difference: family C keeps the larger of 2 µm and `part_gap_mm` between **any** two boxes; `legalize` lets two real courtyards touch at 2 µm. Held parts never move. No flips (D5 stays) | Legal by construction means every placement the search visits, and so its result, passes P1; a cost for overlaps would have to be tuned against the lengths and could end on an overlap. The experiment ran exactly this (0.3 mm between all boxes is that board's `part_gap_mm`) and both routed results passed P2. Courtyards that touch leave no room for a label, and silk was a fault the experiment showed. `legalize` itself is unchanged, so F and S boards stay byte for byte (a golden test) |
| C4 | **The moves** of one movable part, drawn by the seed: a shift within a radius, on a 0.25 mm grid (55 in 100); a turn by 1, 2 or 3 quarter turns about the box centre (12); a swap of centres with another movable part (15); the attraction, which puts the part at the nearest legal cell to the mean of the pads its nets reach (18): the mean is snapped to the 0.25 mm grid of the shifts, and the cells tried are that point and the points around it in steps of 0.5 mm (amendment A1). **Any movable part may turn** (owner decision 1, one switch: `turn_parts`); a part that was read at an angle that is not a multiple of 90° never turns. One more move is not drawn but made on purpose, **making room**: for a decoupling pair past its limit, the capacitor goes to the cell nearest the IC's pad whatever movable parts stand there, and each part it displaces goes to its own nearest legal cell; the whole is one move, made when it lowers the cost. It is tried for every broken pair at the end of each stage and before descent | The first four are the experiment's moves and shares, unchanged. The shift explores, the swap moves a part through a crowd, the attraction is the directed step that the generators lack, and the turn is what a designer does to shorten a connection: in the two routed results 58 and 63 of 89 parts ended turned. Making room is new and was not in the experiment: it is the answer to the runs that ended with capacitors out of range because no single move could free a cell by the IC. It is what generator 8 does when it settles a capacitor right after its IC (D4), as a move |
| C5 | **The temperature.** A move that lowers the cost is made; one that raises it by c is made with probability exp(-c / T). The first T is read from the board, and the run starts melted (amendment A2): 4,000 trial shifts of up to 10 mm are tried from the start and none is made; over the legal ones whose soft cost rises (the cost without the requirement terms), the mean rise m gives T = m / ln(1 / 0.95), so an average uphill shift is accepted 19 times in 20. A stage is 200 proposals per movable part at one T. At effort level 0, T is multiplied by 0.95 after each stage; level k cools 2^k times as slowly (0.95 to the power 1 / 2^k). The shift radius follows the temperature, not the stage count: half the shorter side of the outline's bbox at the first T, times (T / first T)^0.594, never below 1 mm | Simulated annealing as in the experiment. Accepting some uphill moves early is what lets a part cross a barrier; descent alone stops at the first dip (1035 mm against 889 to 900 mm from the same start). The first T is read from the board, not set: a constant would be hot for one board and cold for another. At level 0 the radius rule is the experiment's (0.97 per stage is 0.95^0.594). Tying it to the temperature is a correction: the experiment first shrank the radius by a factor per stage, which at a slower cooling let the radius grow and ended those runs early and badly. The experiment set the first T from the mean uphill change of 400 trial shifts over ln 2, requirement jumps included, and this design first copied it; the first measurement showed that estimate to be a coin toss, and it was amended (A2, below) |
| C6 | **The inner stop rule.** After each stage, the gain is the fall of the best cost over the stage, as a share of the best cost at its start. At level 0, annealing ends when 5 stages in a row gained less than 0.1 % and the last stage accepted fewer than 1 % of its proposals as uphill moves (a stage's proposals are the draws that named a move, legal or not). At level k the rule reads the same fall of the temperature: 5 x 2^k stages, each gaining less than 0.1 % / 2^k. The best placement seen is restored, and descent runs: sweeps over every movable part that take the first move that lowers the cost, until a whole sweep makes no move (then no single move of the set lowers the cost) or 3 sweeps in a row gain less than 0.1 %. Guards: 2000 x 2^k stages, 200 sweeps; a run they end is recorded `converged: false` with which guard | This is the owner's request: the stop reads the value. Both conditions are needed: a stage can gain nothing while the search is still hot and wandering. It must end: T falls every stage, so uphill moves die out, and what is left is a descent on a finite grid inside a bounded outline, where the cost can fall only so many times. Every run of the experiment ended by the rule, after 115 to 193 stages at level 0 (294 to 376 at level 1, 622 to 759 at level 2) and 2 to 4 sweeps, so the guards are 10 times away. The rule is scaled with the level because a stage of a slower cooling is a smaller step: unscaled, it would call a slow run quiet while it is still hot |
| C7 | **The outer stop rules.** "Pays" means: lowers the best cost so far by at least 1 %. **Starts:** runs at level 0 go start by start in the order of C8; the search stops taking starts once 3 starts in a row did not pay (guard: 64 starts). **Effort:** the start whose run is the best is run again at level 1, then 2, and so on, each a new run from the same start; the search stops at the first level that did not pay (guard: level 5). Runs go on all threads at once, but both rules are read in order and nothing past a stop is recorded, so the result does not depend on the number of cores | Both are the same rule: go on while going on pays. 1 % is the least gain worth more work: routing runs of one placement already differ by 1 to 3.5 % in length, so a smaller gain in airwire length is below what a routing call can show. Three starts: replayed on the 8 generated candidates of the experiment in start order, the rule stops after 4 starts and keeps the best of all 8; one start in a row would stop on one unlucky run, since seeds of one start differ by up to 6.6 %. Effort is doubled only on the best start, because a level costs as much as all the levels below it together. In the experiment each doubling still paid 2.5 to 8 % up to level 2, so where the rule for effort stops is not yet known: the first measurement reads it, in Rust, where a level costs seconds and not an hour. The guard at level 5 is 32 times the base effort |
| C8 | **The starts** are the candidates of families F and S that pass P1, each made legal under C's gap rule first (a part that already keeps the gap stays where it is), in the order of their start cost: the screen's airwire length plus 4 mm per crossing, lowest first, ties to the lower id. The requirement terms are left out of the order | The experiment: a random legal start ended between 1228 and 2074, the best generated start at 1121 to 1195, and a better start gave, on the whole, a better end. So the search is a refiner, and the generators that exist are its global step: F's force relax is what a random start lacks. The requirement terms are left out because every run repairs them, so a start with one broken pair fewer and twice the airwire length is not a better start. F and S candidates take nothing from the owner's placement, which keeps D7's fairness |
| C9 | **Same start and seed, same board.** A run's randomness is `SplitMix64` seeded by its start recipe's seed and its level. No clock, thread id or `HashMap` order reaches a run. At the end of each stage every cache is rebuilt from nothing, so rounding of the running sums does not add up | D3 of the placement-search design. Known limit: the acceptance test calls `exp`, which is the platform's; on another platform one bit may differ and a run may take another path. `rerun` is for the machine that made the record, and macOS comes first |
| C10 | **The cost's numbers are the score's own.** The net filter and the spanning tree of `tools/placement/objectives.rs` are used, not written again, so ties in a tree resolve the same way; a test compares L and X of the model with `evaluate`'s `routing_metrics` after thousands of random moves | D10's reason: two copies of one measure drift apart. Crossings depend on which of two equal edges a tree takes |
| C11 | **Records.** Each run is a record of family C. Its recipe holds its start's recipe, its level and every parameter, so `rerun` rebuilds it (it generates the start again, then runs) from the record and a model built as the search builds it: the clearances and the schematic neighbourhoods set. The trace records the SHA-256 of the neighbourhoods the run read, and `rerun` refuses a model whose neighbourhoods differ. The record has `converged` and the run's trace: the cost term by term at the start and the end (the end's from the board as written, anchors rounded to 1 µm, so its airwire length, crossings and violation counts are the screen's own), the first temperature, stages, sweeps, both stop reasons, repairs, evaluations, and the best cost after every stage and sweep. `search.json` also gets the outer trace: the best cost after each start and after each level, and why each rule stopped. `table.md` gets C's row and one line of counts | D21: a record must say what ran and be rebuilt from it. The curves are the evidence that a stop was a convergence |
| C12 | **Selection for routing.** C's candidates that pass P1 are ordered by the screen's requirement fields (score incomplete, decoupling violations, connector violations) and then by their cost; the best 3 and 1 drawn are routed, as for F and S | The cost is what the family converged on; the screen key orders crossings before length, which is another order. The requirement fields first keeps D18's order, and puts a run that ended with a broken pair behind every run that did not |
| C13 | **No time limit inside C.** The board's bench limit (D15) still bounds the routing calls, and C's own wall time does not count against it: the launch rule reads the time since the search began less the time phase 1b took. C's wall time is recorded | The owner's request. The limit was sized for the routing calls (D15), and a search that takes its own time must not cost O or a family a call. The expected time is seconds to a few minutes per board: the experiment needed 0.5 to 0.97 million evaluations per run at level 0, and twice as many for each level above it, and an evaluation in Rust is expected to take tens of microseconds; the first measurement says what it is |
| C14 | The generator version becomes 9 | D21: a record names the build that can rebuild it |

## Decisions for the owner

Approved as recommended by the owner on 2026-11-20 ("כן, כמומלץ"): parts may turn, both
terms are in the cost, and B4, B5 and B6 are prepared in the same row after the two prepared
boards. **Changed by the owner on 2026-11-21** ("אנחנו מריצים על 2 לוחות עכשיו לא על 3"): step 4
runs on two boards, B5 and B6, his choice of the two. B4 gets no bench run in this row; family
C failed to generate on it in the preparation's load check, which is diagnosed all the same.

1. **May the search turn any part you did not lock, by quarter turns?** F and S turn nothing
   today (D5). Recommendation: yes. Your flow says a locked part is fixed and the rest is the
   search's; in the experiment 58 and 63 of the 89 movable parts ended turned, and those are the
   placements that routed 22 to 24 % shorter. If a connector or a switch must keep its facing,
   lock it. A no is one switch (`turn_parts`) and nothing else in the design changes; what it
   costs in length was not measured.
2. **Schematic neighbours and silk labels as weak terms of the cost?** Recommendation: yes,
   both, as tie-breakers (C2). The judge does not change: the schematic measure stays reported
   and not ranked (D23), and silk stays where it is in the rank key. The terms only make the
   search prefer, between two placements of about the same airwire length, the one that keeps an
   LED by its connector and a label off a pad. A no is two weights set to 0.
3. **Which boards?** The bench is prepared for one public board and for the board of the
   experiment. Recommendation: build and measure on those two first, then prepare B4, B5 and
   B6 from your lock copies inside the same row (inputs, rules, and one AI placement each for
   the A reference, on Sonnet 5.5 as you decided for Row PS). One board is not a general result.
   It costs about 5 more hours of machine time and three AI sessions.

## Amendments after the first measurement (2026-11-21)

The first run of the measurement's step 1 **failed the port's check**: a median cost of 1259.8
against the 1245 fixed in advance. Its five costs were 1208.5, 1210.7, 1259.8, 1342.5 and
4009.2, the last from a run that ended with one pair out of range. A diagnosis on the same board
copy found two causes. Each is corrected here by a design decision with its reason. Nothing else
changed, and the check is run again with the same start, seeds and threshold; both results are
reported.

| # | What was wrong | Evidence (the same board copy; counts and numbers only) | The amendment |
|---|---|---|---|
| A1 (C4) | The skeleton said that the attraction looks for its cell "on the 0.5 mm grid". The experiment snapped the target to 0.25 mm and stepped 0.5 mm from there. On one fixed 0.5 mm lattice, a part that an attraction placed before gets its own cell back whenever that cell is the nearest legal one, and the draw names no move | The share of attraction draws that name a move, at stage 50: 0.745 in the experiment, 0.294 as built. Under the experiment's own conditions (R 1000, its first temperatures) the build ended at a median of 1213 over 30 runs, against 1163 over 11 runs of the experiment; with the target on the 0.25 mm grid, 1173, and every stage-by-stage figure (moves accepted, uphill moves, evaluations) within 0.01 of the experiment's. Shifts, turns, swaps, the legality test and the cost agreed from the start | The target is snapped to the 0.25 mm grid; the cells tried are that point and the rings around it in steps of 0.5 mm. This is what C4 meant ("the experiment's moves, unchanged"): the skeleton's words were wrong |
| A2 (C5) | The first temperature was the mean uphill change of 400 chained trial shifts over ln 2, requirement jumps included. About 25 of the 400 trials are legal and uphill, and each one that pushes a pair out of range adds about 150 to the estimate | Over 100 seeds from one start the estimate ran from 11.9 to 538, in two groups: 25 seeds drew no such trial and got 12 to 22. Runs from a first temperature of 15 to 30 ended with a pair out of range in 5 of 16; from 60 upward, in 0 of 40. With A1 in, the cost of the runs that end with every pair in range does not depend on the first temperature (medians of 1172 to 1204 from 15 to 960). The experiment's own model has the same coin toss (10 cold seeds in 100); its 11 runs happened to draw none (68.7 to 256.1). More trials do not settle the estimate while the jumps are in it; without them it is stable (15.8 to 16.9 at 40,000 trials) | The first temperature reads the soft cost alone, from 4,000 trial shifts tried from the start and not made, and is set so that an average uphill shift is accepted 19 times in 20: T = mean rise / ln(1 / 0.95). On the measured board that is about 220: inside the range where no run left a pair out of range, and where the experiment in fact ran (its first temperatures accepted an average uphill shift 85 to 96 times in 100). Whether a broken requirement gets repaired is no longer left to a temperature that a few trials set |

Limits. Both amendments rest on one board. 0.95 is a chosen value, the common practice of
starting an annealing melted, and not a measured optimum: on that board the result is flat from
a first temperature of 120 to one of 960. A 5-seed median on that board moves by about 30 mm
between sets of seeds, which is the size of the first failure's margin; the check stays as it
was fixed, and the wider sweeps are in the row's records.

Also learned on the way, with no change to the design: on the public demo board and on the
owner's, the run at effort level 1 did not pay against level 0, so the rule for effort stopped
after one level; it reads one run per level, which is a noisy reading (C7's own caution).

### A3: the question about the new terms is read in pairs

Fixed on 2026-11-21, after the two bench runs on the owner's board (steps 2 and 3) and before
any paired run existed.

"Do the new terms earn their place?" did not say which candidates it compares. Read as the bench
reads a family, by its best routed candidate, the silk half **fails** on the owner's board: 172
silk entries with the terms on against 154 with them off. The other two halves pass: 3 parts
pulled away against 5, and 1336.7 mm of routed signal against 1359.2.

That reading compares two different placements, from two different starts, and the spread
between starts is wider than the difference it found: the four routed C candidates have 158 to
172 silk entries with the terms on, and 154 to 189 with them off. On the two starts that both
runs routed at level 0, the terms on gave fewer entries both times (161 against 176, and 172
against 189). So that reading settles the question in neither direction, and no weight is
changed on it.

From here on the question is read in pairs. It needs no routing call, because silk entries and
parts pulled away are measures of the placement: the bench reads both on a candidate's board
before it is routed.

- **A pair** is one start that the search takes at level 0 both with the terms on and with them
  off: the two ends, with their silk entries and their parts pulled away.
- **On a board, a term passes** when its mean over the pairs is lower with the terms on: silk
  entries for the label term, parts pulled away for the neighbour term. The neighbour term can
  be read only on a board whose input holds its schematic.
- **The boards** are the owner's board, B5 and B6. (Written as "and B4, B5 and B6"; on
  2026-11-21, before any paired run on them, the owner cut step 4 to two boards and chose B5
  and B6.)
- **A term that does not pass on more than half of the boards where it can be read is set to
  0**, the document says so, and the bench runs are repeated on the final weights.
- The length half stays as written and as measured on the owner's board, where it passed.

Limits. The two terms are switched together, so one term's effect is not separated from the
other's. And this reading was chosen after the first one had been seen, which is why it is
decided on runs that did not exist when it was fixed.

## Modules

New files in `crates/konnect-core/src/`:

| File | What |
|---|---|
| `place_converge.rs` | the cost model, a state and its caches, the moves, annealing, repair, descent, one run (`converge`), the two outer rules (`starts_stop`, `effort_stop`) |
| `place_converge_tests.rs` | its tests |

Changed:

| File | Change |
|---|---|
| `place_generators.rs` | `Family::Converge`, `Params::Converge(ConvergeRecipe)`; `BoardModel::hoods` (the schematic neighbourhoods, set by the search like `input_failures` and `clearances`); `Candidate::trace`; `Matrix::converge` (the parameters and the outer rule, `None` when the family is off); `generate` gets the arm that generates the start and calls `converge`; `Legality` and `GapRule`, with `legalize_turned` calling `Legality::legal_at` in place of its closure, and `legalize_with` taking the rule; `turn_vector`, `text_box` and the tail of `generate` (as `placements_from`) become `pub(crate)`; `GENERATOR_VERSION` 9 |
| `tools/placement/objectives.rs` | the net filter and `mst` become reachable from `place_converge` (a geometry-only entry beside the one that builds the JSON samples); the tool's output does not change |
| `place_search.rs` | `Record::converge` (the trace); `Record::group` gives `C`; `converge_all` (phase 1b, the two outer loops) and `OuterTrace`; `Outcome::converge`; `search` sets `model.hoods` and calls `converge_all` when the matrix has the family; `routing_order` takes `C` and its order (C12); `search_json` and `table` print C |
| `place_bench_tests.rs` | `families` accepts `converge`, with F or S |
| `lib.rs` | the new module |
| `docs/CREATION_HANDOFF.md`, `xtask/placement-bench/README.md` | the row, the option |

Not touched: the rank key and every gate (`place_judge.rs`), the routing call (`place_route.rs`,
`route_search.rs`, `fastroute.rs`), the schematic measure (`place_schematic.rs`, read as it is),
`place_held.rs`, both placement tools and `score_placement`'s output. No new tool, so the
registry count is unchanged.

## Skeletons

`place_converge.rs` and the `Legality` addition below were pasted into this branch and checked
with `cargo check -p konnect-core --tests` (see the end of this section), then removed: the
branch carries the design only. The changes to existing types are given as snippets and were
not compiled.

### `place_converge.rs`

```rust
//! Family C, the convergent placement search (`docs/CONVERGENT_PLACEMENT_DESIGN.md`): one
//! scalar cost of a placement, a search that moves parts while the cost falls, and two
//! stop rules that read the cost and nothing else (no clock, no run count).
//!
//! One run refines one start (a candidate of family F or S): annealing stage by stage
//! until the best cost stops falling, then descent to a placement that no single move
//! improves. The same model, start, seed and parameters give the same placement on any
//! thread. The search around the runs (which starts, how much effort, when to stop) is
//! `place_search::converge_all`, by the rules of [`OuterRule`].

// Only the bench reaches this module (`place_search`, design C1), as with `place_search`.
#![allow(dead_code)]

use crate::place_generators::{BoardModel, Candidate, GenerateFailure, Legality};
use konnect_ipc::types::IpcFootprintPlacement;
use crate::place_schematic::Neighbourhoods;
use crate::route_search::SplitMix64;
use crate::tools::placement::force_core::Rect;
use serde::Serialize;

/// The parameters of one run. They are part of the recipe, so a record says exactly what
/// ran, and `rerun` rebuilds the board from it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct ConvergeParams {
    /// The cost of one proper crossing between the airwire trees of two nets, mm.
    pub crossing_mm: f64,
    /// A net with more than this many distinct references is a rail (the score's limit).
    pub fanout_limit: usize,
    /// The weight of a rail's airwire length when the board has no pour on that net. A
    /// rail with a pour weighs 0: the pour connects it wherever the part sits.
    pub rail_weight: f64,
    /// The least a broken requirement costs before its excess, mm (a decoupling pair
    /// past its limit, a connector too far from the edge). A run uses the larger of
    /// this and the soft cost of its own start (`Trace::requirement_mm`), so one broken
    /// requirement always costs more than everything else the run can still gain.
    pub requirement_mm: f64,
    /// The cost of each mm a broken requirement is past its limit.
    pub requirement_slope: f64,
    /// The weight of the schematic-neighbour term, per mm; 0 turns the term off.
    pub neighbour_weight: f64,
    /// The cost of one label conflict, mm; 0 turns the term off.
    pub label_mm: f64,
    /// Whether a part may turn by quarter turns (owner decision 1). Off: no turn is
    /// proposed and every part keeps the rotation read.
    pub turn_parts: bool,
    /// The effort level k, 0 upward. Level k cools 2^k times as slowly as level 0 and
    /// scales the inner stop rule with it, so that the rule reads the same fall of the
    /// temperature at every level (design C5, C6): see `cooling_at_level`,
    /// `quiet_stages_at_level` and `quiet_gain_at_level`.
    pub level: u32,
    /// Proposals in one annealing stage, per movable part.
    pub stage_per_part: usize,
    /// Level 0's factor of the temperature from one stage to the next.
    pub cooling: f64,
    /// The shift radius is `radius_start x (T / first T)^radius_exponent`, never below
    /// 1 mm: it follows the temperature, not the stage count, so every level shrinks it
    /// over the same range of temperatures.
    pub radius_exponent: f64,
    /// At level 0, a stage that lowers the best cost by less than this share is quiet;
    /// a sweep of descent too, at every level.
    pub quiet_gain: f64,
    /// At level 0, annealing ends after this many quiet stages in a row ...
    pub quiet_stages: usize,
    /// ... once a stage accepts fewer uphill moves than this share of its proposals.
    pub uphill_floor: f64,
    /// Descent also ends after this many quiet sweeps in a row.
    pub quiet_sweeps: usize,
    /// Guards against a run that never settles. Never the expected stop: a run they end
    /// is recorded `converged: false`.
    pub max_stages: usize,
    pub max_sweeps: usize,
}

impl ConvergeParams {
    /// The values the experiment of 2026-11-20 ran with (design C2, C5, C6), at
    /// `level`: parts may turn, crossing 4 mm, fanout 8, rail weight 0.3, requirement
    /// 1000 mm and slope 50, 200 proposals per part and stage, cooling 0.95, radius exponent 0.594, quiet
    /// gain 0.001, 5 quiet stages, uphill floor 0.01, 3 quiet sweeps; and the two terms
    /// the experiment did not have, neighbour weight 0.1 and label 0.2 mm; guards 2000
    /// stages (at level 0; `max_stages_at_level` scales it) and 200 sweeps.
    pub(crate) fn measured(level: u32) -> Self {
        todo!()
    }

    /// `cooling` to the power 1 / 2^level: level 1 takes two stages for the fall of the
    /// temperature that level 0 makes in one.
    pub(crate) fn cooling_at_level(&self) -> f64 {
        todo!()
    }

    /// `quiet_stages` x 2^level: the same fall of the temperature as at level 0.
    pub(crate) fn quiet_stages_at_level(&self) -> usize {
        todo!()
    }

    /// `quiet_gain` / 2^level: a stage of level k is 1 / 2^k of a stage of level 0.
    pub(crate) fn quiet_gain_at_level(&self) -> f64 {
        todo!()
    }

    /// `max_stages` x 2^level.
    pub(crate) fn max_stages_at_level(&self) -> usize {
        todo!()
    }

    /// Refuses a value that is not finite, a weight below 0, a cooling outside (0, 1), a
    /// quiet gain outside (0, 1), a radius exponent that is not above 0, a level above
    /// 16, and a count of 0.
    pub(crate) fn validate(&self) -> Result<(), GenerateFailure> {
        todo!()
    }
}

/// The outer rules: when to stop taking new starts, and when to stop doubling the effort
/// (design C7).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct OuterRule {
    /// What counts as paying: a best cost below (1 - `gain`) times the best cost before.
    pub gain: f64,
    /// Starts: the search stops taking starts at the first count n >= `window` + 1 at
    /// which the last `window` starts did not pay.
    pub window: usize,
    /// Guards, recorded as a guard's stop: the most starts, and the highest effort level.
    pub max_starts: usize,
    pub max_level: u32,
}

impl OuterRule {
    /// The measured rule (design C7): gain 0.01, window 3, at most 64 starts, levels up
    /// to 5.
    pub(crate) fn measured() -> Self {
        todo!()
    }

    /// Whether a best cost of `after` pays against `before`: `after` < (1 - `gain`) x
    /// `before`. An infinite `before` is paid by any finite `after`.
    pub(crate) fn pays(&self, before: f64, after: f64) -> bool {
        todo!()
    }
}

/// Why annealing ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnnealStop {
    /// `quiet_stages` quiet stages in a row and uphill moves below `uphill_floor`.
    Quiet,
    /// `max_stages` reached.
    Guard,
}

/// Why descent ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DescentStop {
    /// A whole sweep made no move: no single move of the set lowers the cost.
    FixedPoint,
    /// `quiet_sweeps` quiet sweeps in a row.
    Quiet,
    /// `max_sweeps` reached.
    Guard,
}

/// The cost of a placement, term by term. `cost` is their weighted sum (design C2).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct Terms {
    pub cost: f64,
    /// The score's own airwire measure: the eligible nets only, weight 1.
    pub airwire_mm: f64,
    /// The rails that count (no pour), unweighted.
    pub rail_airwire_mm: f64,
    /// The score's own proper-crossing count.
    pub crossings: usize,
    /// Decoupling pairs past their limit, and pairs that cannot be measured (the
    /// screen's own count, `place_judge::pair_violations`).
    pub pair_violations: usize,
    /// The sum of the pairs' pad-to-pad distances.
    pub pair_mm: f64,
    /// Connectors farther from the outline than the score's limit, held ones included
    /// (the screen's own count).
    pub connector_violations: usize,
    /// The sum over the parts that have schematic neighbours of their mean centre
    /// distance to them; `None` when the board has no neighbourhoods.
    pub neighbour_mm: Option<f64>,
    /// Label conflicts between parts that share a side: a silk label of one part over
    /// another part's courtyard box, and a label of one part over a label of another
    /// (each pair of labels once).
    pub label_conflicts: usize,
    /// The soft cost: `cost` without the requirement terms of the pairs and connectors.
    pub soft: f64,
}

/// What one run did, for its record.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Trace {
    /// The effort level the run was made at.
    pub level: u32,
    /// What a broken requirement cost in this run: the larger of
    /// `ConvergeParams::requirement_mm` and the start's soft cost.
    pub requirement_mm: f64,
    /// SHA-256 of the schematic neighbourhoods the run read (their JSON, keys sorted);
    /// `None` when it had none. `rerun` refuses a model whose neighbourhoods differ.
    pub hoods_sha256: Option<String>,
    /// The start as given, before it was made legal under family C's gap rule.
    pub start: Terms,
    /// The result as written: the terms of the state rebuilt from the placements with
    /// their anchors rounded to 1 µm, so the airwire length, the crossings and the two
    /// violation counts are the screen's own of the written board.
    pub end: Terms,
    /// Compound moves that made room for a broken requirement (design C4), tried and made.
    pub repairs_tried: u64,
    pub repairs_made: u64,
    pub first_temperature: f64,
    pub stages: usize,
    pub anneal_stop: AnnealStop,
    pub sweeps: usize,
    pub descent_stop: DescentStop,
    /// Legal moves whose cost change was computed.
    pub evaluations: u64,
    /// The best cost at the end of each stage, then of each sweep: the curve.
    pub best_by_step: Vec<f64>,
}

impl Trace {
    /// Both stops are a rule's, not a guard's.
    pub(crate) fn converged(&self) -> bool {
        todo!()
    }
}

/// A net as the cost reads it.
#[derive(Debug, Clone, PartialEq)]
struct Net {
    /// 1 for an eligible net, `rail_weight` for a rail that counts.
    weight: f64,
    /// Counted by the score's measure, and in the crossings.
    eligible: bool,
    /// Its pad sites: (part, index into the part's `pads`).
    sites: Vec<(usize, usize)>,
}

/// A decoupling pair as the cost reads it: (part, pad index) of the capacitor and of the
/// IC, and the limit.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pair {
    cap: (usize, usize),
    ic: (usize, usize),
    limit_mm: f64,
}

/// Everything that does not change while parts move, built once per board and run.
/// Parts are the model's, by its index. A part's position is its courtyard box centre and
/// its quarter turns from the rotation read; every offset below is from that centre, as
/// read, and is turned with the part (`place_generators::turn_vector`).
pub(crate) struct CostModel<'a> {
    model: &'a BoardModel,
    params: ConvergeParams,
    legality: Legality<'a>,
    /// Movable parts, in index order.
    movable: Vec<usize>,
    /// A part read at a multiple of 90 degrees may turn; another keeps its rotation.
    may_turn: Vec<bool>,
    /// Courtyard box size as read.
    sizes: Vec<(f64, f64)>,
    /// Each pad site's offset from the centre, in the order of the connectivity index.
    pads: Vec<Vec<(f64, f64)>>,
    /// Each visible silk label's box, relative to the centre.
    labels: Vec<Vec<Rect>>,
    nets: Vec<Net>,
    /// The nets each part is on.
    part_nets: Vec<Vec<usize>>,
    pairs: Vec<Pair>,
    part_pairs: Vec<Vec<usize>>,
    /// Movable parts the score's connector rule reads (reference prefix `J`).
    connectors: Vec<usize>,
    /// Each part's schematic neighbours, by part index; empty without a neighbourhood.
    hoods: Vec<Vec<usize>>,
    /// The parts that list each part as a neighbour.
    hood_of: Vec<Vec<usize>>,
}

/// A placement and the caches that make a move's cost change cheap.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct State {
    pub centres: Vec<(f64, f64)>,
    /// Quarter turns from the rotation read, 0 to 3.
    pub turns: Vec<u8>,
    /// Each net's airwire length and tree edges (x0, y0, x1, y1).
    net_mm: Vec<f64>,
    net_edges: Vec<Vec<(f64, f64, f64, f64)>>,
    crossings: usize,
    pair_cost: Vec<f64>,
    connector_cost: Vec<f64>,
    neighbour_mm: Vec<f64>,
    label_conflicts: usize,
    cost: f64,
}

impl State {
    pub(crate) fn cost(&self) -> f64 {
        self.cost
    }
}

/// One proposed move: one part's new centre and turns, or two parts' (a swap).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Move {
    pub parts: Vec<(usize, (f64, f64), u8)>,
}

/// The cost change of a legal move and what `apply` needs to make it without computing
/// it again.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Delta {
    pub cost: f64,
    nets: Vec<(usize, f64, Vec<(f64, f64, f64, f64)>)>,
    crossings: i64,
    pairs: Vec<(usize, f64)>,
    connectors: Vec<(usize, f64)>,
    neighbours: Vec<(usize, f64)>,
    labels: i64,
}

impl<'a> CostModel<'a> {
    /// Reads what the cost needs from `model` (its text is parsed once more for the pad
    /// sites, the pour nets and the silk labels) and from `hoods`:
    /// - nets: the score's own filter (`objectives`: at least 2 pad sites, 2 to
    ///   `fanout_limit` distinct references) at weight 1; a net above the limit at
    ///   `rail_weight` unless a zone on the board carries its name, or the weight is 0;
    /// - pairs: `model.intent.pairs`; a pair whose two pads cannot each be found once is
    ///   not measured: it counts as a violation that no move changes, as the screen
    ///   counts it, and adds the requirement cost to every state alike;
    /// - connectors: every part whose reference prefix is `J` (the score's rule); a held
    ///   one's term is a constant;
    /// - labels: every visible text of a footprint on a silkscreen layer (`F.SilkS` or
    ///   `B.SilkS`): its `property` entries and its `fp_text` entries, a text with
    ///   `(hide yes)`, a bare `hide`, or `hide` in its `effects` left out. A label's
    ///   centre is its `(at x y)` taken through the footprint's transform
    ///   (`konnect_sexp::geometry::rotate_about`, as a pad's is); its box is the
    ///   stroke-font estimate `place_generators::text_box` makes for copper text (the
    ///   reference's own length for `${REFERENCE}`), laid along the text's stored angle,
    ///   which is absolute, and its hull at an angle that is not a quarter turn. The box
    ///   is kept relative to the part's centre and turns with the part;
    /// - hoods: `hoods.neighbours` by part index; a name that is not on the board is
    ///   `Invalid`.
    ///
    /// Errs with `Invalid` when no part may move or a parameter is refused.
    /// The requirement cost is `params.requirement_mm` until `with_requirement` sets it.
    pub(crate) fn build(
        model: &'a BoardModel,
        hoods: Option<&Neighbourhoods>,
        params: &ConvergeParams,
    ) -> Result<Self, GenerateFailure> {
        todo!()
    }

    /// The same model with `mm` as the cost of a broken requirement (`converge` gives
    /// the larger of the parameter and the start's soft cost).
    pub(crate) fn with_requirement(self, mm: f64) -> Self {
        todo!()
    }

    /// The state of a placement, every cache computed from nothing. `apply` keeps the
    /// caches equal to this (the test `a_states_caches_equal_a_full_evaluation`).
    pub(crate) fn state(&self, centres: Vec<(f64, f64)>, turns: Vec<u8>) -> State {
        todo!()
    }

    /// The terms of `state`, for the trace and the record.
    pub(crate) fn terms(&self, state: &State) -> Terms {
        todo!()
    }

    /// Whether `mv` is legal: every part it names is legal at its new centre and turns
    /// against every other part where it would be (`Legality::legal_at`, with
    /// `GapRule::Everywhere`), and the two parts of a swap against each other.
    pub(crate) fn legal(&self, state: &State, mv: &Move) -> bool {
        todo!()
    }

    /// The cost change of the legal move `mv`: the airwire trees of the nets of its parts
    /// rebuilt, their crossings counted again against every other eligible net and among
    /// themselves, the pairs and connectors of its parts, the neighbour distances of its
    /// parts and of the parts that list them, and the label conflicts its parts take
    /// part in. Reads `state`, changes nothing.
    pub(crate) fn delta(&self, state: &State, mv: &Move) -> Delta {
        todo!()
    }

    /// Makes the move.
    pub(crate) fn apply(&self, state: &mut State, mv: &Move, delta: Delta) {
        todo!()
    }

    /// A random proposal for the movable part `part` (design C4), or `None` when the kind
    /// drawn has nothing to propose: a shift by up to `radius_mm` on each axis, snapped to
    /// the 0.25 mm grid (55 in 100); a turn by 1 to 3 quarter turns about the centre (12;
    /// nothing when the part may not turn); a swap of centres with another movable part
    /// drawn uniformly (15; nothing when it draws the part itself); or the attraction
    /// with a jitter of `radius_mm` / 4 (18). A proposal that would leave the part where
    /// and as it is counts as nothing. A stage's proposals are the draws that named a
    /// move, legal or not; the uphill share is the uphill moves made over that count.
    fn propose(&self, state: &State, part: usize, radius_mm: f64, rng: &mut SplitMix64) -> Option<Move> {
        todo!()
    }

    /// Making room (design C4), for a decoupling pair past its limit: the capacitor at the
    /// cell nearest the IC's pad (0.5 mm grid, within the pair's limit) that is legal
    /// against the outline, the copper obstacles, the held parts and the IC, whatever
    /// movable parts stand there; each movable part it then lies over is moved to its own
    /// nearest legal cell, largest first, against everything as it then stands. `None`
    /// when no such cell exists or a displaced part finds none within 25 rings. The move
    /// names the capacitor and every displaced part.
    fn make_room(&self, state: &State, pair: usize) -> Option<Move> {
        todo!()
    }

    /// The attraction of `part`: the mean of the pads its nets reach on other parts (a
    /// pair's pad counts three times), jittered by up to `jitter_mm` and snapped to the
    /// 0.25 mm grid of the shifts; then the nearest legal cell among that point and the
    /// points around it in steps of 0.5 mm, within 25 rings (amendment A1). `None` when
    /// the part reaches no other part or no cell is legal.
    fn attraction(&self, state: &State, part: usize, jitter_mm: f64, rng: &mut SplitMix64) -> Option<Move> {
        todo!()
    }
}

/// The state a run starts from (design C8): `start`, the placements of a candidate of
/// family F or S (a part it does not list is where and as the model read it), made legal
/// under `GapRule::Everywhere` by `place_generators::legalize_with` (a part that already
/// keeps the gap stays; one that does not moves to its nearest legal cell). `NoRoom` when
/// a part finds none; `Invalid` when a placement names a part that is not movable or a
/// rotation that is not the part's own plus quarter turns.
fn state_from(cost: &CostModel<'_>, start: &[IpcFootprintPlacement]) -> Result<State, GenerateFailure> {
    todo!()
}

/// The first temperature (amendment A2): 4,000 trial shifts of up to 10 mm on each axis
/// (0.25 mm grid) of parts drawn uniformly, each tried from `state` itself and none
/// made: `state` is not changed. Over the legal trials whose soft cost rises (the cost
/// change without the requirement terms of the pairs and connectors: no R and no
/// excess), the mean rise over ln(1 / 0.95), so that an average uphill shift is
/// accepted 19 times in 20 (50 when no trial was uphill).
fn first_temperature(cost: &CostModel<'_>, state: &State, rng: &mut SplitMix64) -> f64 {
    todo!()
}

/// Whether annealing ends after a stage (design C6): `quiet_run` stages in a row were
/// quiet, at least `quiet_stages_at_level`, and the stage's uphill share is below
/// `uphill_floor`. Both are needed.
pub(crate) fn quiet_enough(quiet_run: usize, uphill_share: f64, params: &ConvergeParams) -> bool {
    todo!()
}

/// Annealing (design C5, C6). A stage is `stage_per_part` proposals per movable part at
/// one temperature; a proposal that is legal is made when it lowers the cost, and
/// otherwise with probability exp(-change / temperature). After each stage: the gain is
/// the fall of the best cost over the stage as a share of the best cost at its start; a
/// stage that gains less than `quiet_gain_at_level` is quiet. Annealing ends when
/// `quiet_enough` says so, or at `max_stages_at_level`.
/// Between stages the temperature is multiplied by `cooling_at_level`, and the shift
/// radius is set from the temperature: `r0 x (T / first T)^radius_exponent`, never below
/// 1 mm, `r0` half the shorter side of the outline's bbox. At each stage's end `repair`
/// runs and the caches are rebuilt from nothing, so rounding does not add up. `state`
/// ends as the best placement seen. Returns the stages run, why it ended, and the best
/// cost after each stage.
fn anneal(
    cost: &CostModel<'_>,
    state: &mut State,
    temperature: f64,
    rng: &mut SplitMix64,
    evaluations: &mut u64,
) -> (usize, AnnealStop, Vec<f64>) {
    todo!()
}

/// Descent (design C6). A sweep visits every movable part once, in an order drawn by
/// `rng`; for a part it tries, in this fixed order, shifts of 0.25, 0.5, 1, 2 and 4 mm in
/// the 8 compass directions, the 3 turns, the attraction without jitter, and swaps with
/// its 6 nearest movable parts, makes the first move that lowers the cost by more than
/// 1e-9, and tries again from the top until none does. `repair` runs before the first
/// sweep. Descent ends when a sweep made no move (`FixedPoint`), after `quiet_sweeps`
/// quiet sweeps in a row, or at `max_sweeps`.
fn descend(
    cost: &CostModel<'_>,
    state: &mut State,
    rng: &mut SplitMix64,
    evaluations: &mut u64,
) -> (usize, DescentStop, Vec<f64>) {
    todo!()
}

/// The repair of broken requirements: for every decoupling pair past its limit, in pair
/// order, `CostModel::make_room`, made when it lowers the cost. Returns how many were
/// tried and how many made.
fn repair(cost: &CostModel<'_>, state: &mut State, evaluations: &mut u64) -> (u64, u64) {
    todo!()
}

/// One run of family C from `start`: `CostModel::build`, `state_from`,
/// `with_requirement` (the larger of the parameter and the start's soft cost),
/// `first_temperature`, `anneal`, `descend`; then the placements of every movable part
/// that is not exactly where and as it was read (`place_generators::placements_from`,
/// the tail of `generate`). The random stream is `SplitMix64(seed)` advanced by
/// `params.level` draws, so the levels of one start do not repeat each other.
/// `Candidate::converged` is `Trace::converged`, `iterations_run` the stages.
/// Pure: the same model, hoods, start, seed and parameters give the same result.
pub(crate) fn converge(
    model: &BoardModel,
    hoods: Option<&Neighbourhoods>,
    start: &[IpcFootprintPlacement],
    seed: u64,
    params: &ConvergeParams,
) -> Result<(Candidate, Trace), GenerateFailure> {
    todo!()
}

/// The rule for starts, on the best costs of the runs of starts 1, 2, 3, ... in start
/// order: the number of starts after which the search stops taking starts, or `None`
/// when it has not stopped within `costs`. It stops at the first n >= `window` + 1 at
/// which min(costs[..n]) does not pay against min(costs[..n - window]) (`OuterRule::pays`),
/// and at `max_starts` whatever the costs say. A start whose run failed is
/// `f64::INFINITY`.
pub(crate) fn starts_stop(costs: &[f64], rule: &OuterRule) -> Option<usize> {
    todo!()
}

/// The rule for effort, on the cost of the best start's run at levels 0, 1, 2, ...: the
/// number of levels after which the search stops doubling, or `None` when it has not
/// stopped within `costs`. It stops after the first level k >= 1 whose cost does not pay
/// against the best of the levels below it, and after `max_level` whatever the costs say.
pub(crate) fn effort_stop(costs: &[f64], rule: &OuterRule) -> Option<usize> {
    todo!()
}
```

### Added to `place_generators.rs`

```rust
// ─── The legality of one moved part ──────────────────────────────────────────

/// Which gap a moved part keeps from another part's box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GapRule {
    /// `legalize` as it is: 2 µm between two real courtyards, and the larger of 2 µm and
    /// `part_gap_mm` when either box is not one (`Clearances::gap_between`).
    CourtyardsMayTouch,
    /// Family C: the larger of 2 µm and `part_gap_mm` between any two boxes.
    Everywhere,
}

/// The legality test of one moved part: `legalize_turned`'s `legal_at`, pulled out so that
/// family C tests a move by the same rule that legalizes F and S (the outline, the edge
/// clearance, the part sides, the copper obstacles), with its own gap rule.
pub(crate) struct Legality<'a> {
    model: &'a BoardModel,
    rule: GapRule,
    /// `Outline::containment_threshold_mm`, read once.
    threshold_mm: f64,
}

impl<'a> Legality<'a> {
    pub(crate) fn new(model: &'a BoardModel, rule: GapRule) -> Self {
        todo!()
    }

    /// Whether part `i`, its box of `size` centred at `at`, is legal there: the box grown
    /// by `edge_mm` on every side lies inside the true outline (first its bbox, then
    /// `Outline::overhang` against the containment threshold); it is clear of every part
    /// of `others` whose sides intersect its own (`clear_of`), each at `centres[j]` with
    /// `sizes[j]`; and it keeps the larger of 2 µm and `part_gap_mm` from every copper
    /// obstacle on its sides. With `GapRule::CourtyardsMayTouch` this is the closure
    /// `legalize_turned` has today, term for term.
    pub(crate) fn legal_at(
        &self,
        i: usize,
        at: (f64, f64),
        size: (f64, f64),
        others: impl IntoIterator<Item = usize>,
        centres: &[(f64, f64)],
        sizes: &[(f64, f64)],
    ) -> bool {
        todo!()
    }

    /// Whether parts `i` and `j`, at the given centres and sizes, keep the rule's gap, or
    /// share no side. The two parts of a swap are tested against each other with it.
    pub(crate) fn clear_of(
        &self,
        i: usize,
        at_i: (f64, f64),
        size_i: (f64, f64),
        j: usize,
        at_j: (f64, f64),
        size_j: (f64, f64),
    ) -> bool {
        todo!()
    }
}
```

`legalize_turned` keeps its settling order and its ring search; only its closure becomes
`Legality::new(model, GapRule::CourtyardsMayTouch).legal_at(i, at, sizes[i], settled, centres, sizes)`.
`legalize_with(model, sizes, turns, centres, rule)` is `legalize_turned` with the rule given;
`legalize_turned` calls it with `CourtyardsMayTouch`, and family C's `state_from` with
`Everywhere`.

### Changed types (snippets, not compiled)

```rust
// place_generators.rs
pub(crate) enum Family {
    Force,
    Schematic,
    Perturb,
    /// C: the convergent search (`place_converge`), one run per start and level.
    Converge,
}

pub(crate) enum Params {
    Force(ForceParams),
    Schematic(SchematicParams),
    Perturb(PerturbParams),
    Converge(ConvergeRecipe),
}

/// A run of family C, enough to rebuild it: the recipe of the candidate it starts from
/// (family F or S, never C) and its parameters, the level among them. The run's seed is
/// the start recipe's.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ConvergeRecipe {
    pub start: Box<Recipe>,
    pub params: ConvergeParams,
}

pub(crate) struct BoardModel {
    // ... as today, and:
    /// The schematic neighbourhoods (`place_schematic::Neighbourhoods`). `None` from
    /// `board_model`; `place_search::search` sets it from its `SchematicBaseline`. `None`
    /// when the schematic cannot be read: family C then runs without the term N, and
    /// its trace has `neighbour_mm: null`.
    pub hoods: Option<Neighbourhoods>,
}

pub(crate) struct Candidate {
    // ... as today, and:
    /// Family C's run; `None` for the other families.
    pub trace: Option<Trace>,
}

pub(crate) struct Matrix {
    // ... as today, and:
    /// Family C: its level-0 parameters and its outer rules. `None`: the family is off.
    /// C has no fixed list of recipes, so `recipes` does not list it; `converge_all`
    /// makes them.
    pub converge: Option<(ConvergeParams, OuterRule)>,
}

// In `generate`:
//   (Family::Converge, Params::Converge(run)) => {
//       let start = generate(model, &run.start)?;          // family F or S, else `Invalid`
//       let (mut candidate, trace) =
//           converge(model, model.hoods.as_ref(), &start.placements, recipe.seed, &run.params)?;
//       candidate.trace = Some(trace);
//       return Ok(candidate);
//   }

/// The absolute anchor and rotation of every movable part that is not exactly where and
/// as it was read: the tail of `generate`, for `converge` too. A part that stayed is not
/// listed (D11).
pub(crate) fn placements_from(
    model: &BoardModel,
    centres: &[(f64, f64)],
    turns: &[i32],
) -> Vec<IpcFootprintPlacement>;

// place_search.rs
pub(crate) struct Record {
    // ... as today, and:
    /// Family C's trace (design C11). `None` for every other origin.
    pub converge: Option<Trace>,
}

/// The outer loops' own record, `search.json`'s `converge`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct OuterTrace {
    pub params: ConvergeParams,
    pub rule: OuterRule,
    /// The ids of the starts taken, in start order, and the best cost after each;
    /// `None` while every run so far failed.
    pub starts: Vec<usize>,
    pub best_by_start: Vec<Option<f64>>,
    /// `rule`, `guard`, or `no_more_starts` (every start was taken).
    pub starts_stop: &'static str,
    /// The id of the start the effort was doubled on, and its cost at levels 0, 1, ...
    pub best_start: Option<usize>,
    pub cost_by_level: Vec<f64>,
    /// `rule` or `guard`; `None` when no run of a start succeeded.
    pub effort_stop: Option<&'static str>,
    /// Wall time of the whole phase.
    pub seconds: f64,
}

/// Phase 1b, family C. `starts` are the records of families F and S as phase 1 left
/// them. The pool is those with outcome `generated` that pass P1 (`screen_key[0] == 0`),
/// ordered by start cost (the screen's `wirelength_mm` + `params.crossing_mm` x
/// `crossings`; a record without both sorts last), ties to the lower id.
///
/// Starts. The pool's starts are handed in that order to `threads` scoped threads from
/// one atomic counter. Each job is `generated_record` with the recipe
/// `Recipe { family: Converge, seed: <the start recipe's seed>, params: Params::Converge(
/// ConvergeRecipe { start, params: *params }) }` (generate, write, held check, evaluate,
/// screen, as for every family) and files the candidate's trace on its record. After
/// each job the costs of the longest finished prefix are read by
/// `place_converge::starts_stop`; once it gives n, no new start is handed out, the
/// running jobs end, and exactly the records of the first n starts are kept.
///
/// Effort. The start whose record has the lowest cost (ties to the earlier start) is
/// run again with `params.level` 1, 2, ..., one level at a time, each a record like the
/// others; after each, `place_converge::effort_stop` reads the costs of levels 0 up, and
/// the loop ends when it gives a count. Every level's record is kept, the one that did
/// not pay too.
///
/// Ids are `first_id` upward: the starts in start order, then the levels. A panic is a
/// `generate_failed` record and costs infinity. The result is the same for any number of
/// threads.
pub(crate) fn converge_all(
    model: &BoardModel,
    starts: &[Record],
    params: &ConvergeParams,
    rule: &OuterRule,
    threads: usize,
    first_id: usize,
) -> (Vec<Record>, OuterTrace);
```

**Compile check, 2026-11-20.** `place_converge.rs` and the `Legality` addition were pasted into this branch, with the `mod` line in `lib.rs`. `cargo check -p konnect-core --tests` (rustc 1.96.0) finished with no error. Its 73 warnings are all unused variables, fields and items of the `todo!()` bodies, in the two pasted pieces (56 and 17). The files were then removed: this branch carries the design only.

## Tests, each guard with a negative control

| Guard | Test (`place_converge_tests.rs` unless named) | Negative control |
|---|---|---|
| The cost's length and crossings are the score's | `the_models_airwire_and_crossings_are_the_scores_own`: on the rectangle, rounded and demo fixtures, `Terms::airwire_mm` and `crossings` equal `evaluate`'s `routing_metrics` as read (the length to 1e-6 mm, the crossings exactly), and again after 1,000 random legal moves: the board is written with `placements_applied_to`, and the state rebuilt from that board's own anchors is compared with its evaluation by the same bounds | Site order reversed in the tree on the fixture with two equal edges changes the crossings; counting a rail in `airwire_mm` changes the length |
| The caches are a full evaluation | `a_states_caches_equal_a_full_evaluation`: after 10,000 applied moves of every kind, making room included, the state equals `CostModel::state` of its centres and turns (counts exactly, sums to 1e-6) | Leaving out the crossings between two nets of the moved part fails it |
| The pull-out changes nothing | `legality_is_the_closure_legalize_had` (`place_generators_tests.rs`): every existing `legalize` test unchanged, and the boards of F and S recipes 1 to 8 on the demo fixture equal goldens recorded before the change | `GapRule::Everywhere` inside `legalize_turned` changes a golden |
| C keeps its gap, from the start on | `family_c_keeps_the_part_gap_between_real_courtyards`; `a_start_whose_courtyards_touch_is_made_legal_first` | With `CourtyardsMayTouch` two courtyards end 2 µm apart |
| Every result passes P1 | `a_converged_board_has_no_new_hard_failure`: the board of a run on the crowded fixture, evaluated: no `courtyard_overlap` or `outside_outline` that names a movable part | With `legal` testing no other part, an overlap appears. (As first written, the control was the test of a swap's two parts against each other; that cannot fail here, because a swap keeps the offset between its two parts. That line is guarded at the model's level, by `the_parts_of_a_swap_are_tested_against_each_other`, and matters for the compound move) |
| Held parts never move | `family_c_never_lists_a_held_part`, every hold reason; the board passes `check_held` | — |
| No flip, and turns only where allowed | `no_part_changes_side`; `a_part_read_off_the_quarter_turns_is_never_turned` | A part read at 0° does turn in the same run |
| Same start and seed, same board | `the_same_run_gives_the_same_board_bytes`: twice, and from pools of 1 and 8 threads | Another seed differs; level 1 differs from level 0 |
| A run stops by its rules | `a_run_stops_by_its_rules_on_the_fixtures`: `AnnealStop::Quiet`, and `DescentStop::FixedPoint` or `Quiet`, on three fixtures, at levels 0 and 1 | `max_stages: 3` ends on the guard and `Trace::converged` is false |
| The inner rule needs both conditions | `quiet_enough_needs_the_quiet_stages_and_the_cold`: a table of quiet runs and uphill shares | Without the uphill condition, 5 quiet stages at an uphill share of 3 % end the run |
| The inner rule reads the same fall of the temperature at every level | `a_level_scales_the_cooling_the_patience_and_the_quiet_gain`: `quiet_stages_at_level` x ln(`cooling_at_level`) is the same at levels 0 to 3; `quiet_gain_at_level` x `quiet_stages_at_level` is the same; so is the radius at one temperature | An unscaled patience gives a fourth of the fall at level 2 |
| The first temperature (A2) | `the_first_temperature_trials_do_not_move_the_start`; `the_first_temperature_reads_the_soft_cost`: the same value whatever a broken requirement costs, on a fixture where trials push a pair out of range; `the_first_temperature_accepts_an_average_uphill_shift_19_times_in_20`: the value is the mean soft rise of the trials, each from the start, over ln(1 / 0.95) | With the whole cost change, the value follows R; with the trials chained, it differs from the replay; with ln 2, it is 13.5 times lower |
| The attraction's grid (A1) | `the_attraction_searches_from_the_quarter_millimetre_grid`: a part on the 0.5 mm lattice whose target lies a quarter millimetre off it is moved there when the cell is legal | With the target snapped to 0.5 mm the same draw names no move |
| The requirement cost follows the start | `a_broken_requirement_costs_at_least_the_starts_soft_cost`: a start whose soft cost is above 1000 mm | A fixed 1000 mm lets a move trade a pair for length on that fixture |
| Turns can be switched off | `with_turn_parts_off_every_part_keeps_its_rotation` | — |
| Descent ends at a fixed point | `after_descent_no_single_move_lowers_the_cost`: when the stop is `FixedPoint`, every move of descent's set, tried on the result, lowers the cost by no more than 1e-9 | The state after annealing alone has such a move on the fixture |
| Making room | `a_capacitor_boxed_out_of_range_gets_a_cell_by_its_ic`: a fixture with every cell within the limit taken by movable parts; after `repair` the pair is within its limit and nothing overlaps; `making_room_never_moves_a_held_part`; `making_room_is_refused_when_a_displaced_part_finds_no_cell` | Without `repair`, descent ends at a fixed point with the pair broken. (As first written: "the same run ends with the pair broken". On the fixture a whole run frees the cell while it is hot, so the control is at descent's level) |
| The connector term is the score's rule | `the_connector_term_counts_what_the_score_deducts`: an unlocked `J` in the middle, one at the edge and a locked one in the middle; `Terms::connector_violations` equals the screen's on the start and on the result; `a_held_connector_changes_no_moves_cost` | Counting movable connectors only gives one less than the screen |
| A pair that cannot be measured | `an_unmeasured_pair_is_a_constant_violation_and_the_run_still_ends`: a pair whose IC pad is missing; `Terms::pair_violations` equals the screen's | Refusing the model fails family C on a board where F and S run |
| A pour connects its rail | `a_rail_with_a_pour_weighs_nothing_and_one_without_weighs_the_rail_weight` | — |
| The neighbour term is the measure's sum | `the_neighbour_term_is_the_schematic_measures_sum`: `Terms::neighbour_mm` equals the sum of `place_schematic::part_distances`; `a_part_on_rails_only_ends_by_its_schematic_neighbours` | With `neighbour_weight: 0` the same part ends farther away; with no schematic the run still ends and `neighbour_mm` is `None` |
| Label conflicts | `a_label_over_another_part_is_a_conflict_and_over_its_own_is_not`; `two_parts_labels_over_each_other_are_one_conflict`; hidden texts (all three spellings) and the other side of the board count nothing; a label is placed through its footprint's transform and turns with its part | With `label_mm: 0` the count does not enter the cost |
| The rule for starts | `starts_stop_reads_the_costs_in_start_order`: a table of cost lists and the n each gives (the experiment's 8 costs give 4); a failed start is infinity; the guard | A window of 1 stops after 2 on the same list |
| The rule for effort | `effort_stop_ends_at_the_first_level_that_does_not_pay`: a table; the guard at `max_level` | A gain of 0 never stops before the guard |
| The starts and their order | `the_starts_are_f_and_s_candidates_that_pass_p1_by_start_cost` (`place_search_tests.rs`): a P1 failure, a duplicate and a failed record are no start; the order ignores the decoupling count | Ordered by the screen key, a candidate with one broken pair fewer and twice the airwire length comes first |
| The outer result does not depend on the pool | `the_recorded_runs_do_not_depend_on_the_threads` (`place_search_tests.rs`): pools of 1 and 8 give the same records | Recording every finished start gives more records from 8 threads |
| Parameters | `bad_parameters_are_refused`: a negative weight, cooling 1, a count of 0, a value that is not finite, a level above 16 | `ConvergeParams::measured(0)` is accepted |
| Selection | `family_c_is_ordered_by_requirements_then_cost` (`place_search_tests.rs`) | Ordered by the screen key, a candidate with fewer crossings and a higher cost comes first |
| Rerun | `rerun_rebuilds_a_converged_board` | Another generator version is refused; a start recipe of family C is refused; a model with other schematic neighbourhoods is refused |
| Counts only | `the_table_names_no_net_and_no_part`, with family C in the search | — |
| Generator version | `the_generator_version_is_nine` | — |
| The bench's option | `converge_needs_f_or_s_and_the_default_stays_f_and_s` (`place_bench_tests.rs`) | `["converge"]` alone is refused with its word |

Scoped `cargo llvm-cov` on `place_converge.rs`, and targeted `cargo mutants` on the guards
(`Legality::legal_at`, `Legality::clear_of`, `CostModel::legal`, `CostModel::delta`,
`CostModel::make_room`, the two inner stop conditions, `Trace::converged`, `OuterRule::pays`,
`starts_stop`, `effort_stop`, the starts' order and C's order in `routing_order`), in an
isolated checkout, as `AGENTS.md` says.

## Measurement

Every parameter is fixed here, before the first run (`ConvergeParams::measured`,
`OuterRule::measured`). Nothing is tuned after seeing a result; a change is a new design
decision with its reason.

1. **Without routing, seconds to minutes.** Family C on the public demo board and on the
   experiment's copy of the owner's board. Read: every run's stop reasons, both outer stops, the
   wall time and the time of one evaluation. **The port's check:** from the experiment's best
   start (the F recipe of seed 24 with rail weight 0, generator 8's board), 5 runs at level 0
   with seeds 1 to 5 and both new terms off, as the experiment ran (R is this design's: on
   this start it is the start's soft cost, above the experiment's 1000 mm). The experiment's
   three seeds cost 1121, 1163 and 1195 (943 to 995 mm of airwire, 27 to 34
   crossings). The port is right when the median of its 5 costs is at most 1245: the
   experiment's median plus its seed spread of 7 %. Lower is welcome. The experiment's model is
   `target/creation-research/convergence-study/spike1/anneal.py` (with `board.py`), for the
   builder to read beside the skeletons; where the two differ, this document decides.
   **First run, 2026-11-21: failed, at a median of 1259.8** (see "Amendments after the first
   measurement"). The check is run again after the two amendments, unchanged.
2. **The bench, the owner's board**, effort M, 100-minute limit, families F, S and C: O 3
   calls, 4 calls per family.
3. **The same run with both new terms off** (`neighbour_weight` 0, `label_mm` 0): what the terms
   buy and cost.
4. **B5 and B6** (owner decision 3 as he changed it on 2026-11-21; written for B4, B5 and
   B6), after steps 1 to 3 on the two prepared boards: the
   bench preparation from his lock copies, one AI placement each for the A reference, then the
   same run.

**The answers, fixed in advance:**

| Question | Passes when |
|---|---|
| Is the port the experiment's search? | The check of step 1: a median cost of at most 1245 |
| Does it stop by convergence? | Every run and both outer loops end by their rules, on every board; no guard |
| Is the result legal and complete? | C's best candidate passes P1, P2 and G1 to G3 on every board, and has no decoupling or connector violation on a board where O has none |
| Is it better than the generators? | C's best ranks above the best of F and of S on every board |
| Is it as good as a hand placement? | On the owner's board C's verdict against O is "beats", as the experiment's placements did (reported per board for B4 to B6) |
| Do the new terms earn their place? | With them on: fewer silk entries and fewer parts pulled away than with them off, and a routed signal length no more than one step longer. A term that fails this is set to 0 and the document says so |
| Does making room work? | No C run ends with a broken pair on a board where another C run of the same search has none |

A question that fails is reported as it is. Expected cost: minutes for C itself; 100 minutes of
routing calls per bench run.

### The answers on the owner's board (2026-11-21)

Commit `d413bf8c`, release, 15 threads, effort M. Step 2 took 4606 s and step 3 4652 s; each
routed 15 candidates (O 3, F 4, S 4, C 4). Counts and numbers only. One routing call per
candidate, and one board: not yet a general result.

| Question | Answer | What was measured |
|---|---|---|
| Is the port the experiment's search? | Failed, then passes | First run: a median of 1259.8. After A1 and A2, the same start, seeds and threshold: 1136.2, 1148.2, 1206.1, 1226.3 and 1235.6, a median of 1206.1 against 1245 |
| Does it stop by convergence? | Passes | Every run ended by its rules (annealing quiet, descent at a fixed point) and both outer rules stopped by their rule: 7 runs in step 1's second run, 7 in step 2, 6 in step 3. No guard. On the public demo board the dry run of commit `60c9be73` (before the performance pass and the amendments) ended its 7 runs by their rules too; it is still to be repeated on the final commit |
| Is the result legal and complete? | Passes | C's best passes P1, P2 and G1 to G3 with no decoupling and no connector violation; O has none either |
| Is it better than the generators? | Passes | C's best ranks 1; F's best ranks 7 and S's best 8, each with 2 decoupling violations |
| Is it as good as a hand placement? | Passes: "beats" | C's best against O: 1336.7 mm of routed signal against 1738.0 (0.77 times), 69 vias off pours against 82, copper quality 57.6 against 59.7, 172 silk entries against 109 |
| Do the new terms earn their place? | Not settled | As written, on each run's best: silk fails (172 against 154), parts pulled away pass (3 against 5), length passes (1336.7 mm against 1359.2). Read in pairs from here on: see A3 |
| Does making room work? | Passes | No C run ended with a pair out of range: 0 of 12 in step 1's second run, 0 of 7 in step 2, 0 of 6 in step 3 |

What these runs also showed:

- **The cost does not see whether a placement routes completely.** In each bench run the C
  candidate with the shortest routed signal failed G1 (25 and 26 unconnected items; a different
  start each time), and ranks 11. With the terms on it is also the candidate of the lowest
  cost. The other three routed C candidates of each run pass every gate.
- **Silk is worse than the hand placement's in every C candidate**: 154 to 189 entries against
  109, with the terms on or off.
- **With the terms on, two of the three complete C candidates rank below O** (5 and 6), by 1.0
  and 3.0 acute corners of their routing; with the terms off the three rank 1 to 3. One call
  each.
- **The rule for effort**: with the terms on, level 1 paid (1360.2 to 1297.4) and level 2 did
  not (1366.0); with them off, level 1 did not pay (1186.2 to 1185.9). C's search took 66 s and
  42 s.
- **The same five starts at level 0, terms on against off** (means): airwire 998 mm against
  977, crossings 51.8 against 46.6, label conflicts 115 against 159, parts pulled away 4.8
  against 6.8.

Still open here: B4, B5 and B6 (step 4), and the paired reading of A3 on the four boards.

## What this design does not do

- **A global optimum.** A run ends at a placement no single move improves; another start can end
  lower. The outer rules say when more starts and more effort stop paying, no more.
- **Search from a random start.** Measured: far worse than from a generated candidate.
- **Double the effort on more than the best start.** A start that is second at level 0 could be
  first at level 2. The first measurement shows how often the order of starts changes with the
  level; if it does, this is the first change to design.
- **Start from the owner's own placement.** It gave the lowest cost in the experiment (889 to
  900 mm against 943 to 995 mm) and the same routed length within one routing call's noise. In
  his flow there is no placement to start from, and family P is off for the same reason.
- **Move an IC with its capacitors as one body.** An IC whose capacitors are in range moves only
  as far as they stay in range.
- **A loop between placement and routing** (move the parts that the routed result shows are in
  the way).
- **Apply a result, or offer one through a tool.** After the bench, on the owner's word.
- Flips; turns that are not quarter turns; moving a label (the term only keeps parts from
  landing under one); thermal spacing, analog and switching separation, room to probe and
  rework (no measure exists).
- Windows and Linux; a live KiCad.

## Records

The experiment: `target/creation-research/convergence-study/` (the evidence scripts), its
`spike1/` (the model, the runs, the boards, the three bench folders) and its `design-skeleton/`
(the skeleton files as compiled). Private and outside git.
