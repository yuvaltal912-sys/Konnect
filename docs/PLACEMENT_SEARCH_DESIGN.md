# Best-of-N placement search: design

Status: design only. Claude Opus 5.5 designed it down to the skeletons on 2026-11-18, on branch
`claude/placement-search-design` from `fork/main` `d5588205`. The owner approved its three
decisions as recommended the same day ("Yes, as recommended"). Sonnet 5.5 agents write the bodies
when he starts the build. Nothing is built, and no board was written.

**Owner's flow, 2026-11-19 (decision 4 below).** He draws the outline, imports the parts and
dumps them anywhere, then places and locks in KiCad only the parts he wants fixed. The search
places everything else around the locked parts. So the bench's held set is KiCad's locks (plus
`held.json` and the parts with no pad on a net), not the rules of D6, and P (edits of his own
placement) is off by default. The rules and P stay as bench options.

## What the owner asked for

- A parallel best-of-N search for footprint placement, built like best-of-N routing
  ([`ROUTE_SEARCH_DESIGN.md`](ROUTE_SEARCH_DESIGN.md)): N candidates of one board are made side by side, each
  is judged the same way, and they are ranked.
- Measurement first. The first thing built is a bench that runs on copies of boards and changes
  nothing in the product. It answers one question: can generated candidates match or beat the
  AI's own placement and the owner's hand placement on his boards (B4 VCU, B5 BSPD, B6
  Fuseboard)?
- Q1 stands: a Konnect planner is a second opinion, never the author. Nothing in this design
  applies a candidate to a board.

## What the code says today

Checked in `fork/main` `d5588205`; board counts from read-only copies of the owner's three boards.

- **Both placers are deterministic** (no random number, no clock). Tests
  `force_directed_is_deterministic_and_respects_locks` and
  `auto_place_plans_a_legal_deterministic_layout` assert it.
- **`auto_place_from_schematic` never writes.** Its plan is always blocked.
- **`refine_placement_force_directed` can still write a real board.** If a plan passes every
  gate, `dry_run: false` writes through `update_closed_board_footprints`. The gates are: a
  `max_displacement_mm`, convergence, a better and complete score, no hard failure, every target
  inside the outline, and the board not open in KiCad. Test:
  `force_directed_applicable_plan_still_applies`. The search never calls the handler, so this path
  stays as it is and goes unused.
- **Both legacy placers refuse any outline that is not a provable rectangle.** Two of the owner's
  three boards have rounded corners (4 lines and 4 arcs), so both placers refuse them.
  `score_placement` already judges the true outline.
- **The force-directed placer** works only on courtyard-box centres, not on pads, and never
  rotates a part. Its repulsion ring uses part widths only. Its core (springs, cooling loop,
  collision pass) sits inside the tool handler, between the argument parsing and the score and
  apply gates.
- **The schematic placer** joins parts over every net, so one shared ground net makes the whole
  board a single cluster. It reads the board's nets, not the schematic.
- **The placers' score** runs through `score_of_content`, which writes a temp file and goes
  through `handle_score_placement`. That handler first asks KiCad's IPC whether the file is open.
- **The routing call needs live KiCad.** `export_specctra_dsn` reads each net's routing rules
  from the live editor over IPC. The file-based DSN probe gives every net a flat 0.2 mm
  instead (the router research's harness defect 3).
- **The routing result has what the judge needs.** The `route_specctra_dsn` result carries
  `search.winner.gates`, `search.winner.measures` and `search.baseline`. The baseline is the
  routed board's own unrouted DRC, by type.
- **The owner's boards:**

  | | B4 | B5 | B6 |
  |---|---|---|---|
  | footprints | 118 | 94 | 127 |
  | KiCad-locked footprints | 0 | 0 | 0 |
  | `J` references | 13 | 5 | 32 |
  | schematic sheets | 2 | 2 | 3 |
  | outline | rounded rectangle | rectangle | rounded rectangle |
  | back-side parts | 0 | 2 | 0 |

## The flow of one search

```
place_search(plan)        one board; everything is written below plan.folder
 1. held set (`plan.hold` rules) and intent from the input's saved text (refuse what cannot
    be proved);
    the input's unrouted check (P2's yardstick)
 2. the model: courtyards, connectivity, the true outline and its bbox; S0
 3. references: O (the input) and A (the AI's board, admitted only if it differs from
    S0 in nothing but where movable parts sit and how they turn), each held-checked,
    evaluated and screened like any candidate
 4. phase 1, on a bounded pool of threads: the matrix's recipes (288 with all three
    families, 160 with F and S) -> generate -> legalize
    -> board text -> held check -> evaluate -> decoupling pairs -> screen;
    then boards that repeat an earlier one are marked duplicates
 5. phase 2, selection: O 3 times, A 3 times, then per family its 3 best by the
    screen and 1 drawn (only candidates that pass P1), the families interleaved (best,
    drawn, second, third), round r starting at family r mod 3
 6. phase 3, 18 routing calls (14 without P), one at a time while the launch rule allows:
    candidate project -> file-based DSN -> route_specctra_dsn (fastroute, best-of-N,
    effort M) -> its winner's gates and measures, and its unrouted baseline
 7. judge: P1, P2, G1 to G3, the rank key
 8. search.json (every candidate) and table.md (counts only)
```

Nothing is written to the input board or any project of the owner. Every board the search
makes is a scratch copy below the plan's folder. Every routing call runs fastroute the way the
product does (sandboxed, never `--live`). The search applies nothing.

## Decisions taken in this design (with the reason)

| # | Decision | Reason |
|---|---|---|
| D1 | The first build is the bench, an ignored test in `konnect-core` (`place_bench_tests.rs`) that runs the search on copies. It adds no tool, changes no registry and has no apply path. Product files change only by three extractions that keep their outputs (the force core, the score evaluation, the footprint writer) and by two additions in `route_judge.rs` (`Deserialize` on `Measures` and `Gates`, and `rank_measures`) | The prompt and Q1. The bench needs crate-internal functions, so it lives inside the crate, as the DSN probe `freerouting_routes_a_folder` does |
| D2 | Candidates are two references and three generator families. O: the input board, as the owner placed it. A: the AI's placement from S0. F: the force core from a seeded scatter. S: the schematic placer's clusters with a fanout filter, raw or polished by the force core. P: seeded edits of O | F and S answer "from scratch, can a generator match A and O". P answers "can search improve O". These are the prompt's four sources |
| D3 | Randomness is `route_search::SplitMix64`, one stream per recipe, from its seed. It picks F's scatter, S's cluster order and P's edits. No clock, thread id or `HashMap` order reaches a candidate | The two placers are deterministic, and their tests say so. A seed is the only new input, and SplitMix64 already gives the same sequence on every machine and build (route design D2) |
| D4 | Generators accept any closed outline. The force core runs in the outline's bbox; then `legalize` moves every part that the recipe moved inside the true outline (`Outline::overhang`, the score's own containment check) and off every settled part that shares a side with it (a through-hole part is on both sides), held or not, by 2 µm between two real courtyards and by `part_gap_mm` when either box is not a real courtyard (a footprint without one is bounded by its pads, and 2 µm leaves pads of different nets touching). A moved part's box also lies `edge_mm` inside the outline, and keeps the larger of 2 µm and `part_gap_mm` from every copper obstacle on its sides (below). Both values (`Clearances` on the model) are set by the search from the routing inputs: `part_gap_mm` is the largest `clearance_mm` of the board's rules plus 0.1 mm of solder-mask margin, `edge_mm` the project's copper-to-edge clearance; both are 0 where nobody set them, and both are recorded in `search.json`'s plan. A part at exactly its input position and rotation is legal as it is: its containment and its overlaps with other unmoved parts are not checked, because the input may hold them on purpose (a socket over its neighbours, a part at the edge), and P1 is relative to the input's own hard failures (D17). The same holds for the obstacles: an unmoved part is not tested against them. The two legacy tools still refuse an outline that is not a provable rectangle. **Order.** The moved parts settle in decreasing area of their courtyard box (width times height), equal areas by reference, after the held and the unmoved parts, which settle first as before. Reference order left a big part for last, when the small parts were scattered and no free strip was left: on a board of 100.5 x 60.5 mm, a 19 x 50.8 mm module found no room in 160 of 160 F and S recipes, and a hand placement of the same parts fits it. Largest first is the standard order of greedy legalization. **Decoupling caps.** Every moved decoupling cap is taken out of that list and settles right after its IC, as a designer places it: before the other moved parts when the IC is held or unmoved, the caps of one IC in `intent.pairs` order, a cap in several pairs by its first pair; a cap whose pair cannot be measured (the IC is not on the board, or a pad is missing or on the part twice) keeps its largest-first place. Its search does not start at its own centre: it starts at the IC pad (the IC's final centre plus the pad's offset, turned with the IC), and within a ring takes the cell that puts the cap's own pad nearest the IC pad. What is legal is unchanged. The reason is the owner's board (101 parts, 12 locked): with the largest-first order alone the best routed F candidates had 5 to 7 decoupling violations and the owner's own placement has 0, because a 0603 capacitor settled almost last, after the cells near its IC pin were taken, and landed in the nearest free cell, out of range. **Copper obstacles.** The model lists the boxes of the board's fixed copper, from its top-level items: `gr_line`, `gr_rect`, `gr_circle`, `gr_arc`, `gr_poly`, `gr_curve`, `gr_text_box` and `gr_text` on a copper layer, and rule areas whose keepout says `(footprints not_allowed)` or `(pads not_allowed)`. A keepout that forbids only tracks, vias or pours is no obstacle, and nothing on silkscreen, fabrication or `Edge.Cuts` is. Each box is a superset of the copper. A graphic is its geometry grown by half the stroke; an arc is its exact extent (`konnect_sexp::geometry::arc_bbox`, the bulge included, the box of the three points when they are collinear), so a near-straight arc is a thin box and not the box of its huge circle. An `(arc …)` inside the `pts` of a polygon, a rule area or a text box is boxed the same way. A `gr_text` with a `render_cache` is the box of the cache's polygons; if any one of them is unreadable the whole text takes the estimate, never a partial box. Without a cache it is the stroke-font estimate (a text with no readable size is 1.524 mm, KiCad's default; `(line_spacing s)` multiplies the line pitch by max(1, s); knockout text, `knockout` on its layer, grows by max(thickness, size_y / 4) on every side, where KiCad's own margin is at most max(thickness / 2, size_y / 9)): width = longest line's characters x `size_x` x 1.3 + thickness, height = lines x `size_y` x 1.7 + thickness, centred on `at`. A `justify left` or `right` makes the box span the anchor by the whole width on x, `top` or `bottom` the same on y: a superset whichever way the text is mirrored. The box is then turned by `at`'s angle about the anchor, exactly for a cardinal angle and as the axis-aligned hull otherwise. Each obstacle has its sides: `F.Cu` the top, `B.Cu` the bottom, any other copper layer (an inner one, `*.Cu`, `F&B.Cu`) both. **Part sides.** A footprint with any `thru_hole` or `np_thru_hole` pad occupies both sides, any other its own `layer_side`; two parts, or a part and an obstacle, meet only when their sides intersect. **Known limits, which the P2 gate still catches:** a text that contains `${` (a variable, whose value is not known here) is boxed by its source characters; a `gr_text_box` whose text overflows its box is boxed by the corners, not by the text; and copper inside a footprint (a copper logo `fp_poly` in a pad-less held footprint) is not an obstacle. The bench's three owner board copies have no footprint with copper graphics. The estimate's numbers come from KiCad's own boxes of the 19 copper texts of the demo board: width less thickness over characters x `size_x` was at most 1.22, height over lines x `size_y` was 1.70 with the thickness in, so 1.3 and 1.7 plus the thickness are ceilings. The cache is KiCad's own outline of the glyphs, so it needs no margin | Two of the three boards have rounded corners, and both legacy placers refuse those. A candidate is judged by the same containment check that legalized it. Repairing what the input itself does would fail every recipe that leaves such a part alone (the smoke run's 128 P recipes). Without the two clearances every routed F and S candidate failed P2 in the third smoke run: new `clearance`, `copper_edge_clearance`, `solder_mask_bridge`, `shorting_items` and `pth_inside_courtyard` entries on boards the input has none on. With them, every routed F and S candidate of the fourth smoke run still failed P2: 12 to 24 `clearance` and 11 to 20 `solder_mask_bridge` entries per candidate, every one a through-hole pad against a board-level copper text (a product title, the `1=>>` markers), which `legalize` did not know; and one `shorting_items` with a `pth_inside_courtyard`, a through-hole part on top of a bottom-side SMD pad, because `legalize` compared only parts with the same `layer_side` and a through-hole pad is on every copper layer. The generator version is 8: 5 for the obstacles and the part sides, 6 for the exact boxes (arcs, text estimate), 7 for the largest-first order, 8 for the decoupling caps |
| D5 | No flips. F and S keep every part's rotation as read. P may turn a part with exactly 2 pads by 90° or 180° | A flip changes the assembly side and mirrors the footprint, and no generator here models that. The force core has no torque, so rotation can only be a discrete edit. Turning 2-pad passives is the cheap case of the checklist's "rotate parts for direct connections". S0 keeps O's rotations, a disclosed leak |
| D6 | The held set, in the mode `rules` (bench option `hold`, since 2026-11-19; see D6a for the default): KiCad locks, plus the caller's list, plus every connector (prefix `J`), at the edge or not, plus mechanical parts (no pad on a net, a prefix `H`, `MH` or `FID`, or a footprint ID naming a mounting hole or fiducial). It is read from the saved text only, and refused when it cannot be proved (`held_set`'s list). A reference shared only by footprints that have no pad (the logo footprints, which share one reference on each of the owner's boards) is allowed: each is held (mechanical) and keyed `<reference>#<n>`, n its order among those footprints in file order, and `check_held` proves them unchanged by that key. A reference shared by any footprint with a pad stays refused, and the refusal says to give those parts unique references in the copy (the bench's preparation does that) The owner's boards have no locked footprint, so there the held set was to be the rules plus his edits (owner decision 3, superseded for the bench by decision 4) | This is the placement loop's own rule: saved evidence, or `BLOCKED`. Connectors and mounting holes come first in the checklist ("outline, mounting holes, connectors"). A cable, a mating board or the enclosure fixes a connector's place and facing (checklist 10, "connector orientation"), not the router. A mounting hole may have a netted pad, so a pad rule alone would miss it |
| D6a | The bench's default held set is `locks` (owner decision 4, 2026-11-19): KiCad's locks (`HoldReason::KicadLock`), the caller's list (`held.json`) and every footprint with no pad on a net, that last one because such a part (a logo, a test-point artwork) plays no part in routing and its `<reference>#<n>` keys never reach a generator. The connector prefixes, the mechanical prefixes and the footprint-word rules are off (`HoldRules::locks_only()`), so a connector that the owner did not lock moves like any part. `"hold": "rules"` in `bench.json` keeps D6 as written. The mode reaches the search as `Plan::hold` and is recorded in `search.json`'s plan (`hold_rules`) | The owner's flow: he places and locks only what he wants fixed and expects the search to place the rest, connectors included. A rule that holds a part he did not lock would measure something he will not do. The held list of a board is then his own act on a copy (`lock-copies`), not a rule confirmed from counts |
| D7 | S0 keeps the held parts as read and puts every movable part in rows outside the outline, with sides and rotations as read. A starts from S0. F and S start from their own seeded layouts and know no more than S0 does. F, S and A get the same brief: the held list, the groups and the decoupling pairs. P starts from O | A fair comparison gives every from-scratch arm the same facts, and A never sees O |
| D8 | Decoupling pairs come from `score_placement`'s `decoupling_associations` on the input board (every entry, whatever its status), never from a shared net. In F and in S's polish they are pad-to-pad springs of weight 10. Each candidate is measured against that same fixed list (`pair_violations`: a pair whose two pads are farther apart than its limit, or cannot be found, is a violation), and in the rank the violations are an ordering term right after the gates. The candidate's own associations are not used: a capacitor moved away from every IC drops out of them, which would read as no violation. Groups (the caller's, KiCad groups, schematic sheet instances) are centre springs of weight 0.5; their spread is recorded, not ranked | Hard constraints would make legalize fail on a dense board. Ranking this requirement before every routing preference keeps the checklist's "beside its supply pin" first without hiding a candidate that misses it. Weight 10 is twice the strongest legacy spring (5, differential pairs); 0.5 is half a signal net, a nudge. Whether grouping helps beyond what the router rewards is for the bench to show, from the recorded spread. Taking pairs from the input board uses the owner's placement as evidence of which capacitor serves which IC. This is disclosed, and every arm gets the same list |
| D9 | The force core boundary. `force_core.rs` holds the constants, the spring graph, the cooling loop (`relax`) and the collision pass (`resolve_collisions`). The handler keeps the arguments, the board read, the rectangle check and its refusal, the part list and its movable rule, the legacy weights (`net_weight`), anchors and displacements, the score before and after, the applicability gates and the apply path. Its output stays byte-identical (golden test) | Everything the search varies (start, weights, extra springs, bounds) is an input of the core. Everything the tool must keep (its refusals, gates and apply path) stays in the tool |
| D10 | `evaluate.rs` holds what `handle_score_placement` does after parsing. The handler reads the board (live or saved), calls it and adds `source` | A candidate must be scored on a thread, with no async runtime, no temp file and no question to KiCad. Tool and search share one evaluation, so they cannot drift apart |
| D11 | `pcb_components::placements_applied_to` is the loop of `update_closed_board_footprints` without the file. A generator lists only the parts it moved or turned: a part that stayed is not listed, because a set re-spells its text and pad angles even by 0 degrees. `placements_moved_applied_to` is the same loop that only moves a footprint whose rotation is already the placement's, as the move tool does, and sets a turned one: `admit_ai` rebuilds A with it | A candidate's board is written by the same transform as a real move (the reason `apply_placements_to_content` exists), and in memory, so the pool needs no temp files. A candidate must equal the input in everything it did not change, and an AI that moves parts one by one with the move tool leaves the angles of the parts it did not turn as spelled; a set of every part would not reproduce its board |
| D12 | Phase 1 (generate, write, check the held set, evaluate, screen) runs on a bounded pool of scoped threads, one per core, before any routing. Routing calls then run one at a time, and each uses the routing search's own lanes on the whole machine. Both phases fall under the board's one time limit (D15) | Generation is CPU-only and takes seconds. The routing search's lanes (route design D5) are tuned for one search per machine. Row BN's rerun saw host load change routing throughput by 6 to 16 %, so two searches at once would make candidates' routing incomparable |
| D13 | Every routed candidate is routed at effort M. Each reference (O, A) is routed 3 times; each selected generated candidate once | Equal routing effort is what makes the routed measures a placement comparison. Fast (2 runs) is too noisy: on B4 with pours fixed, stage 7 left 2 to 10 unconnected across 7 seeds. H or XH would make the bench 3 to 10 times longer. A family gets the best of 4 calls; a reference's own 3 calls show the routing noise its best must clear (D18) |
| D14 | The matrix is 288 recipes with all three families; the bench's `families` option (default F and S, 160 recipes; P is off because editing the owner's own placement has no meaning in his flow, owner decision 4) keeps only the named ones, and O and A stay as references either way. F: seeds 1 to 32 x rail weight {0, 0.5, 3} on nets with more than 8 references. S: seeds 1 to 32 x {raw, polished}. P: seeds 1 to 64 x {4 edits within 2 mm, 12 edits within 5 mm}. A board that repeats an earlier one is a duplicate and is never routed. Per family, the screen's 3 best and 1 drawn candidate among those that pass P1 are routed | Generating and screening cost seconds; routing costs minutes. So many candidates are made and few are routed. The drawn one shows whether the screen picks well: if drawn candidates beat the screen's best, the screen is wrong. Fixed slots give every family an answer. 8 is `score_placement`'s own fanout limit; 3 is the legacy weight of a power name. Small edits and screen ties can repeat a board, which would waste a call. S's shelves take the first of the margins 2, 1, 0.5 and 0.25 mm that fit the bounds (0.25 mm, clamped into the bounds, if none does). Each part has its own width: cells of a cluster's largest member (a ZIF socket) overflowed the board far beyond `legalize`'s reach, 28 to 35 parts failed in the smoke runs |
| D15 | The time limit is 100 minutes per board (18 calls of about 5 minutes, plus phase 1). A routing call starts only when the median wall time of the calls that routed a candidate so far still fits before the limit; before any such call has ended, the estimate is the budget plus 60 s, a margin this design chooses. A started call is never cut; its own budget ends it. The rounds go F, S, P; S, P, F; P, F, S; and so on (round r starts at family r mod 3) | Route design D6's launch rule, applied to whole calls. Route design D7 lets a call that has no judged run yet wait past its budget without a bound, so a call that judged no run (no winner) says nothing about how long the next one takes and stays out of the median, which replaces the first estimate as soon as a call routes one. The drawn candidates come second in each family's turn, so a short limit does not cut the screen's check first. The family order rotates so that the same family (P, last in a fixed order) is not the one that loses its last call whenever the limit cuts a round |
| D16 | Every stage failure becomes a record, and the search goes on. A generator panic is caught. Three routing calls in a row that gave no routed candidate (a refused export, a failed call, or a call with no winner), with none routed yet, stop the routing phase; a routed candidate resets the count, and after the first one the phase does not stop this way. `table.md` names a stop by a fixed word (`time_limit`, `input_check_failed`, `calls_failed_in_a_row`); the full reason, which can hold a path or a tool's output, is in `search.json` only | Route design D16 and D18: a broken export or binary fails every call the same way, and a call costs minutes. A call with no winner after the budget is the same signal when nothing has routed yet. The table is a document that leaves the folder, so it carries counts and codes, not text of errors |
| D17 | The gates are P1 (no hard failure of the placement score that names a movable part and is not among the input board's own, compared by kind and sorted references, and the outline is closed), P2 (no physical DRC type worse than on the input board, both unrouted), and G1 to G3 of the routing call's winner. The score runs with every held part as `edge_mount_references`. Decoupling, connectors, the routing tail, silk and the score only order | P1 and P2 are buildability, judged on what a candidate changed: a held connector that crosses the edge on purpose, an overlap between two held parts, or a fault the owner's board has between movable parts, is the same on every candidate, O included, and must not fail them all (O never fails P1 on its own faults, and a candidate that does not mend one is not punished for it). G1 to G3 are the routing judge's absolute gates (route design D10). They are relative to the candidate's own unrouted board, and P2 ties that board to the input. Silk is a label fix, not a fault |
| D18 | The key (`place_judge::rank`) puts the unconnected count before any length. Length steps come only from candidates that pass P1, P2 and G1. Ties go to the lower id: O, then A, then recipe order. A family's verdict against a reference: it beats the reference when its best ranks above all of the reference's judged calls, matches it when its best ranks among them, and is below it otherwise (`verdict`). A reference's judged calls are its routed calls and its calls with no winner (both carry a key): a no-winner call is the reference's worst rank, and the table's legend says so | Stage 7's B6: when every run failed every gate, the order fell to length, and the run that routed least came first. A candidate with an illegal placement must not set the steps either. A family's best is a best of 4 calls, so a "beats" must clear the reference's own routing noise, not one lucky call |
| D19 | Each net's routing rules are read once per board from KiCad's own resolution: headless KiCad Python with a scratch `KICAD_CONFIG_HOME`, as the router research's `net_rules.py` did. Every candidate is exported with `specctra::export_dsn_with_context` | `export_specctra_dsn` reads the rules from the live editor, and the bench never uses one. Net classes follow net names and patterns, not placement. The probe's flat 0.2 mm was harness defect 3 |
| D20 | Isolation. Every tool call runs with an `ipc_address` inside the bench folder, where nothing listens. `KICAD_CONFIG_HOME` and `KONNECT_STATE_DIR` must be scratch folders. Input boards must lie inside the bench folder, links followed, and a board's folder, its `candidates` and its `isolation` must not be links. An unknown home folder is refused (the real KiCad and Konnect folders, `~/Library/Preferences/kicad`, `~/.config/kicad`, `~/.local/share/konnect` and `~/Library/Application Support/konnect`, cannot then be excluded). fastroute runs only through `route_with_fastroute` (`sandbox-exec`, never `--live`). Each board's run holds the lane locks (native, then instr), because it starts kicad-cli. The A session runs with `KICAD_API_SOCKET` set to a dead path inside its own folder, scratch `KICAD_CONFIG_HOME` and `KONNECT_STATE_DIR`, and a working folder that holds only its S0 copy and its brief. A is admitted only if it differs from S0 in nothing but where movable parts sit and how they turn (`admit_ai`) | This is the never-touch list. The judge calls tools that ask IPC first, and a running KiCad of the owner must never be asked. An empty `ipc_address` falls back to `KICAD_API_SOCKET`, and the release binary finds a live socket on its own, so the A session needs the dead path too. Without the admission check, an AI session could change a net, a zone, the outline or a rule, or flip a part, and still be compared as a placement |
| D21 | Every candidate records its origin, generator version, recipe (family, seed, parameters), the SHA-256s of the input, the held list, the intent and its own board, its timings, and its routing call (folder, fastroute SHA-256, effort, winner shape and seed, elapsed, overrun, host load at start). `rerun` rebuilds a generated board byte for byte from its record | Route design D20. A routing call is a timed search, so its winner can differ on a rerun. Its search folder keeps the winner's variant and session, so the routed board can be rebuilt without routing again |
| D22 | Documents report counts only. The tables and `search.json` stay in `target/creation-research/placement-search/`, which git ignores | `AGENTS.md`: real-project results stay out of documents, apart from counts |
| D23 | **Schematic neighbours, reported and not ranked** (owner request, 2026-11-19). A circuit tied to a board interface should sit near it: the "12 V present" LED and its resistor next to the 12 V connector. Both nets are board-wide power nets, so no trace length pulls the LED to the connector, and nothing measured it. The schematic holds the intent: designers draw such parts next to each other. `place_schematic.rs` reads the board's own project (the `.kicad_sch` with the board's file stem in the input folder, copied there by the bench preparation), follows each `(sheet …)` through its `uuid` and its `Sheetfile` property (so a file used twice is two sheet instances), and fixes once, from the input board, the neighbourhood of every measured part. **Mapping.** A footprint's `(path "/<sheet uuids>/<symbol uuid>")` names its sheet instance (the sheet uuids below the root) and its symbol. If no symbol of that instance has that uuid, or the path names no known instance, the part is matched by its reference on the instance (the symbol's `(instances … (path "/<root uuid><sheets>" (reference …)))` entry, else its `Reference` property): on the instance the path names, else on the only instance that has the reference; a reference on two instances is not guessed. A part's schematic points are the `(at x y)` of every unit it has on the instance. A footprint that has at least one pad and a matched symbol is a measured part; one with a pad and no symbol is counted as unmatched; a pad-less footprint is skipped and not counted. **Neighbourhood.** A measured part's neighbours are its 3 nearest measured parts on the same sheet instance (least distance between unit points, ties by reference; fewer when the sheet has fewer; none, and the part is left out of the mean and counted as alone). A held part is a measured part and a neighbour like any other: the connector may be locked and the LED must still sit near it. **Measure.** The board distance is between courtyard-box centres (`FootprintCourtyard::bbox`, the centres the placement model uses) on the candidate's own text. `own(i)` is the mean board distance from part i to its neighbours; `mean_mm` is the mean of `own(i)`. Each candidate's `search.json` record gets `schematic_neighbours`: `mean_mm`; `ratio_to_input` (`mean_mm` over the input board's, `null` when that is 0); `parts_pulled_away` (parts whose `own(i)` is more than twice the input's and more than 10 mm above it, both strictly); `parts_measured`, `parts_unmatched` and `parts_alone`. A missing or unreadable schematic, or a candidate that lacks a measured part, gives `schematic_neighbours: null` with a `schematic_neighbours_reason`, and the search goes on. **Not in the rank**: the key, the rank and the verdicts do not read it. `table.md` has two columns in the routed-candidates table, `sch. nbr mm` and `pulled away`, and one line that says so | The owner's request, in substance: a part tied to an interface should stay near it, relative to his own placement. Trace length cannot say this for a power net, but the schematic can. It is reported only because nothing yet shows that a candidate which moves a part away is worse, and the rank key is the judged result (D18): a new term would change every verdict without evidence. 3 neighbours, 2x and 10 mm are this design's choices, to read from the first run on a board of his: 2x alone would flag a 1 mm to 3 mm move, and 10 mm alone a 12 mm part that was already 11 mm away. The centres are the model's own, so the measure and the generators agree on where a part is. The schematic is read with plain reads, not `read_schematic`, which takes document locks in Konnect's state folder: the search writes only below its own folder. A sheet file is refused when it lies outside the root's folder or includes itself |

## Decisions for the owner

Approved as recommended by the owner on 2026-11-18 ("Yes, as recommended").

1. **May a winning candidate ever be applied, and how?** *(Reversed by decision 4 for the later
   tool stage; the text below is the 2026-11-18 recommendation.)* Q1 says a planner never writes, and this
   design applies nothing. Recommendation: not before the bench answers. If it answers yes, a later
   read-only planner returns the winner as explicit `set_component_placements` batches, one
   functional group at a time. The AI compares each batch with its own placement and adopts it
   through the normal tools, with readback, as the route-refinement planner works (Q1's
   second-opinion shape). The planner itself never gets a write path.
2. **Who makes A, the AI reference?** Recommendation: one Claude session per board on Sonnet 5.5
   (high), the standing default for new agents (Opus needs your yes). It uses the kopper1 skills,
   served by this checkout's release binary by pipe, not the installed Konnect. It places a
   scratch copy of S0 in closed-board mode, before any generator result exists, and is isolated
   as D20 says (no KiCad socket, scratch settings, only S0 and the brief in its folder). Its brief
   is the held list, the groups and the decoupling pairs, and its time is recorded.
3. **The held list of each board.** None of the three boards has a KiCad-locked footprint.
   Recommendation: the rule (every connector, every mounting hole and fiducial) makes each
   board's list, and you confirm or edit it once, from the counts, before the run. On B6 that
   holds at least 32 connectors of 127 footprints; release any that may move.
   *Superseded for the bench by decision 4 (2026-11-19): the held set is his KiCad locks.*

4. **The owner's flow (2026-11-19).** He draws the board outline, imports the parts from the
   schematic and dumps them anywhere, then places and locks in KiCad only the parts he wants
   fixed; the search places everything else around them. Consequences, all decided:
   - The held set is his KiCad locks (D6a), not the rules and not a list he confirms from counts:
     this replaces decision 3 for the bench. He locks the parts on copies of B4, B5 and B6.
   - P is off by default (`families` defaults to F and S); O and A remain as references.
   - Decision 1 is reversed for the later tool stage: a tool may place the winner on his board.
     It is still not built here. The order is measure first, then the tool.

## Modules

New files in `crates/konnect-core/src/`:

| File | What |
|---|---|
| `tools/placement/force_core.rs` | the force-directed core, pulled out of `handle_force_directed` (D9) |
| `tools/placement/evaluate.rs` | `score_placement`'s evaluation, pulled out of its handler (D10) |
| `place_held.rs` | the held set, its proof, functional groups, decoupling pairs |
| `place_generators.rs` | the board model, the three families, S0, legalize, the matrix |
| `place_judge.rs` | the screen, the placement gates, the routing result, the rank key |
| `place_route.rs` | the stripped input, routing rules, a candidate's project and DSN, the routing call, the isolated context |
| `place_schematic.rs` | the schematic-neighbour measure (D23): the sheets read, the neighbourhoods fixed once, the measure of a candidate |
| `place_search.rs` | records, the generation pool, selection, the launch rule, the search, rerun, `search.json`, `table.md` |
| `place_bench_tests.rs` | the bench (ignored) and its environment guards |
| `place_held_tests.rs`, `place_generators_tests.rs`, `place_judge_tests.rs`, `place_route_tests.rs`, `place_schematic_tests.rs`, `place_search_tests.rs`, `tools/placement/force_core_tests.rs` | tests |

Changed: `lib.rs` (the new modules), `tools/placement.rs` (the two handlers call the extracted
code; `FD_*` constants move to `force_core.rs`), `tools/pcb_components.rs` (`placements_applied_to`),
`route_judge.rs` (`Deserialize` on `Measures` and `Gates`; `rank_measures`, which `rank` calls),
`docs/CREATION_HANDOFF.md` (a row). No new tool, so the registry count is unchanged. Not touched:
`route_search.rs`, `fastroute.rs`, `tools/route_fastroute.rs`, `specctra.rs`, `scratch_board.rs`
(all used as they are), the kicad-pcb skill, `API_MIGRATIONS.md`, `tool-directory.md`.

## Skeletons

The skeletons compile when pasted in, apart from the `todo!()`s. They were checked by pasting them
into this branch and running `cargo check -p konnect-core --tests` (see the end of this section).
`Result` is `anyhow::Result` unless named.

### `tools/placement/force_core.rs`

```rust
//! The force-directed placer's arithmetic, pulled out of `handle_force_directed`
//! (`refine_placement_force_directed`) so that a search can run it on threads, from
//! other starts and with other parameters. Pure: no I/O, no clock, no randomness of
//! its own.
//!
//! With [`FdParams::legacy`], the legacy spring weights, an empty [`FdExtras`] and the
//! parts' courtyard centres as the start, [`relax`] and then [`resolve_collisions`]
//! perform the same floating-point operations, in the same order, as the handler did
//! before the pull-out. `force_directed_output_is_unchanged_by_the_pull_out` holds them
//! to it.

use konnect_sexp::board::{PcbConnectivityIndex, Side};
use std::collections::BTreeMap;

/// An axis-aligned rectangle `(x0, y0, x1, y1)` in board mm, `x0 < x1`, `y0 < y1`.
pub(crate) type Rect = (f64, f64, f64, f64);

/// The spring embedder's constants. Every field is finite; all but
/// `temperature_floor_mm` are above 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FdParams {
    pub k_spring: f64,
    pub k_repel: f64,
    pub k_wall: f64,
    pub damping: f64,
    pub min_dist_mm: f64,
    pub grid_mm: f64,
    /// Rounds the snapped layout must stay unchanged to count as converged.
    pub patience: usize,
    /// The iteration ceiling (the tool's `iterations`, at most 10 000).
    pub iterations: usize,
    /// The first temperature as a share of the bounds' shorter side (legacy 0.05).
    pub step_share: f64,
    /// Added to the falling temperature every round (legacy 0.1 mm).
    pub temperature_floor_mm: f64,
    /// The ring beyond the two half-widths inside which repulsion is boosted
    /// (legacy 1.0 mm). Like the legacy, only the widths (`size.0`) enter it.
    pub clear_ring_mm: f64,
    /// The collision pass searches rings `1..ring_end` (end exclusive) before it leaves
    /// a part where it snapped. Legacy 200: the handler's loop is `for ring in 1..200`,
    /// so 199 rings.
    pub ring_end: usize,
}

impl FdParams {
    /// The tool's constants (`FD_K_SPRING`, `FD_K_REPEL`, `FD_K_WALL`, `FD_DAMPING`,
    /// `FD_MIN_DIST`, `FD_GRID_MM`, `FD_PATIENCE` in `placement.rs`, which move here),
    /// the legacy literals above, and `iterations` from the tool's argument.
    pub(crate) fn legacy(iterations: usize) -> Self {
        todo!()
    }
}

/// One part as the core sees it, in the caller's order (the handler's: sorted by
/// reference).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FdPart {
    /// Courtyard bbox width and height, mm.
    pub size: (f64, f64),
    pub side: Side,
    /// False for a KiCad-locked, caller-locked or unselected part: it exerts force and
    /// is an obstacle, but never moves.
    pub movable: bool,
}

/// Accumulated spring weight per part pair `(a, b)`, `a < b`, every weight above 0.
/// A `BTreeMap`, so iteration is in pair order, as the handler's per-round `collect`
/// into a `BTreeMap` made it.
pub(crate) type Springs = BTreeMap<(usize, usize), f64>;

/// The handler's spring graph, generalized only in where the weight comes from: for
/// every net of `index`, in its sorted order, with `weight(net) > 0`, every pair of
/// distinct parts of `references` with a pad on that net gains `weight(net)`, added in
/// the handler's order. A pad whose reference is not in `references` is ignored.
/// `references` is sorted and unique; a reference's position is its part index. The
/// handler passes `net_weight`.
pub(crate) fn springs(
    index: &PcbConnectivityIndex,
    references: &[&str],
    weight: &dyn Fn(&str) -> f64,
) -> Springs {
    todo!()
}

/// A spring between two pads instead of two centres: it pulls the pad of part `a` at
/// `a_offset` (from `a`'s centre) toward the pad of part `b` at `b_offset`, with force
/// `weight * k_spring * distance`, on both parts. Decoupling pairs use it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PadSpring {
    pub a: usize,
    pub b: usize,
    pub a_offset: (f64, f64),
    pub b_offset: (f64, f64),
    pub weight: f64,
}

/// A spring from every member of a group toward the members' mean centre, the mean
/// taken afresh every round, with force `weight * k_spring * distance`. Functional
/// groups use it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GroupSpring {
    pub members: Vec<usize>,
    pub weight: f64,
}

/// Forces the tool never had. Empty (the default) adds no operation at all, so the
/// legacy arithmetic is untouched; otherwise they are added after the walls.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct FdExtras {
    pub pads: Vec<PadSpring>,
    pub groups: Vec<GroupSpring>,
}

/// What the cooling loop ends with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Relaxed {
    /// Unsnapped centres, one per part; an immovable part's is its start.
    pub centres: Vec<(f64, f64)>,
    pub iterations_run: usize,
    /// The layout snapped to `grid_mm` stayed unchanged for `patience` rounds.
    pub converged: bool,
}

/// The cooling loop of `handle_force_directed`, unchanged. Per round: the temperature
/// `shorter side of bounds * step_share * (1 - round / iterations) + floor`; pairwise
/// repulsion, boosted inside the clearance ring; the springs in pair order; the walls
/// of `bounds`; then `extras`. Velocities are damped and clamped to the temperature;
/// each centre is clamped so its half-size stays inside `bounds`. Converged when the
/// layout snapped to `grid_mm` is unchanged for `patience` rounds. Immovable parts keep
/// velocity 0 and their start.
///
/// Requires `parts.len() == start.len()`, every index in `springs` and `extras` below
/// `parts.len()`, and `bounds` at least as large as every part. It panics otherwise:
/// that is a caller bug, not an input condition, since every caller builds all four
/// from one courtyard scan.
pub(crate) fn relax(
    parts: &[FdPart],
    springs: &Springs,
    extras: &FdExtras,
    bounds: Rect,
    start: &[(f64, f64)],
    params: &FdParams,
) -> Relaxed {
    todo!()
}

/// The handler's collision pass, unchanged. Immovable parts are settled first; then
/// each movable part, in index order, is snapped to `grid_mm` and, while its bbox
/// overlaps a settled part on its own side, spirals outward ring by ring (8 directions
/// per ring, each clamped into `bounds`) to the first free cell, over the rings
/// `1..ring_end`. A part with no free cell stays where it snapped: the
/// placement score's hard failure then says so. Mutates `centres` in place.
pub(crate) fn resolve_collisions(
    parts: &[FdPart],
    bounds: Rect,
    centres: &mut [(f64, f64)],
    params: &FdParams,
) {
    todo!()
}
```

`handle_force_directed` after the extraction (a sketch, not a skeleton):

```rust
// … arguments, board read, rectangle check, `parts` sorted by reference: unchanged …
let references: Vec<&str> = parts.iter().map(|c| c.reference.as_deref().expect("filtered")).collect();
let core: Vec<FdPart> = /* size from the courtyard bbox, side, movable as today's closure */;
let springs = force_core::springs(&index, &references, &net_weight);
let start: Vec<(f64, f64)> = parts.iter().map(|c| bbox_center(c.bbox)).collect();
let params = FdParams::legacy(iterations);
let bounds = (ox0, oy0, ox1, oy1);
let relaxed = force_core::relax(&core, &springs, &FdExtras::default(), bounds, &start, &params);
let mut centres = relaxed.centres;
force_core::resolve_collisions(&core, bounds, &mut centres, &params);
// … from "Final positions" on: unchanged, with `relaxed.iterations_run` and `relaxed.converged` …
```

### `tools/placement/evaluate.rs`

```rust
//! `score_placement`'s judgement of a parsed board, pulled out of
//! `handle_score_placement` so that a search can score thousands of candidates on
//! threads with no file, no async runtime and no call to KiCad. The handler reads the
//! board (live or saved) as before, calls [`evaluate`] and adds `source`; its response
//! is unchanged (`score_placement_output_is_unchanged_by_the_pull_out`).

use konnect_sexp::board_geometry::Allowlist;
use konnect_sexp::parser::SexpNode;

/// Every field of `score_placement`'s response except `source`, computed from `tree`
/// alone, by the code that follows `parse_sexp` in today's handler: `verdict`, `score`,
/// `score_incomplete`, `routing_metrics`, `decoupling_coverage`, the outline fields,
/// `edge_mount_listed`, `hard_failures`, `deductions`, `connector_edges`,
/// `uncoupled_caps`, `unproven_decoupling_caps`, `decoupling_associations`,
/// `interface_filter_caps`, `footprints_scored` and `footprints_skipped`.
/// `max_net_references` is already validated (2..=256). Errs only where the handler's
/// `?` did (a coverage record that does not serialize). No I/O.
pub(crate) fn evaluate(
    tree: &SexpNode,
    edge_mount: &Allowlist,
    max_net_references: usize,
) -> anyhow::Result<serde_json::Value> {
    todo!()
}
```

`handle_score_placement` becomes: arguments, the live-or-saved read (unchanged), `parse_sexp`,
`let mut result = evaluate(&tree, &edge_mount, max_net_references)?;`,
`result["source"] = json!(source);`, `Ok(CallToolResult::json(&result))`.

### Added to `tools/pcb_components.rs` and `route_judge.rs`

```rust
// Compile check only: the two functions the design adds to existing files.
use crate::route_judge::{Gates, Key, Measures};
use konnect_ipc::types::IpcFootprintPlacement;

// ── in tools/pcb_components.rs ──

/// `placements` applied to `content` in order, each exactly as
/// `update_closed_board_footprints` applies it (`prepare_closed_board_footprint_update`
/// with `FootprintPlacementUpdate::Set`), with no file: the same transform, so a
/// candidate's board and a real apply cannot disagree. Errs on the first placement that
/// does not apply. `update_closed_board_footprints` becomes: read, this, persist.
pub(crate) fn placements_applied_to(
    content: &str,
    placements: &[IpcFootprintPlacement],
) -> Result<String, crate::tools::pcb_components::ClosedBoardError> {
    todo!()
}

// ── in route_judge.rs ──

/// `rank`'s key for each run, with the length steps taken against the shortest of
/// the runs that are in the pool (the `bool`) and pass G1; against all runs in the pool
/// when none of them passes; against every run when none is in the pool. `rank(judged)`
/// becomes this with every run in the pool, so its keys do not change.
pub(crate) fn rank_measures(runs: &[(&Measures, &Gates, bool)]) -> Vec<Key> {
    todo!()
}
```

`route_judge::Measures` and `route_judge::Gates` gain `serde::Deserialize` beside `Serialize`, so
`place_route::read_route_result` reads a call's winner back into the judge's own types.

### `place_held.rs`

```rust
//! Which footprints a placement search may move, and the intent that steers it.
//!
//! Read from the saved board text only (in the bench, a copy), never from a live
//! editor: the text is the evidence, and what it cannot prove is refused, as the
//! `kicad-pcb` skill's bounded placement loop reports `BLOCKED` instead of guessing.

use konnect_sexp::board::Side;
use konnect_sexp::parser::SexpNode;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Why a footprint never moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HoldReason {
    /// KiCad's own lock: a direct `(locked yes)` (`FootprintCourtyard::locked`).
    KicadLock,
    /// Named by the caller (the bench's per-board list).
    Caller,
    /// A connector ([`HoldRules::connector_prefixes`]), at the edge or not: its place
    /// and its facing serve a cable, a mating board or the enclosure.
    Connector,
    /// A mounting hole, fiducial or logo: no pad on any net, or a mechanical prefix or
    /// footprint name ([`HoldRules`]).
    Mechanical,
}

/// A held footprint, as the saved text has it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HeldPart {
    pub reasons: BTreeSet<HoldReason>,
    /// The root `(at x y rot)` and the side, exactly as read.
    pub at: (f64, f64),
    pub rotation_deg: f64,
    pub side: Side,
}

/// The rules that hold a part besides KiCad's lock and the caller's list.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HoldRules {
    /// Reference prefixes (the letters before the number) that name connectors.
    /// Default `["J"]`, as `score_placement`'s connector check reads them.
    pub connector_prefixes: Vec<String>,
    /// Reference prefixes of mechanical parts. Default `["H", "MH", "FID"]`.
    pub mechanical_prefixes: Vec<String>,
    /// Words in a footprint's library ID that mark it mechanical, matched without
    /// case. Default `["MountingHole", "Fiducial"]`.
    pub mechanical_footprint_words: Vec<String>,
}

impl Default for HoldRules {
    /// The defaults above.
    fn default() -> Self {
        todo!()
    }
}

/// The parts that stay and the parts that may move.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HeldSet {
    pub held: BTreeMap<String, HeldPart>,
    /// Every other footprint: the parts a generator may move.
    pub movable: BTreeSet<String>,
    /// SHA-256 of the board text the set was read from.
    pub board_sha256: String,
    /// SHA-256 of the held list as the records write it: sorted references, each with
    /// its `at`, rotation, side and reasons.
    pub held_sha256: String,
}

/// Why the held set cannot be proved, with the step that fixes it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct HeldRefusal {
    pub reason: String,
    pub next_step: String,
}

/// The held set of the saved board `text`. Refuses, never guesses, when:
/// - the text does not parse as a board;
/// - the outline is not closed (`Outline::status` is not `Closed`): no candidate's
///   containment could be judged or repaired;
/// - a reference in `caller` names no footprint, or more than one;
/// - a footprint has no reference, or a reference is shared by footprints of which any
///   has a pad (the rule of `score_placement`'s `ambiguous_pad_references`; the
///   refusal's next step says to give those parts unique references in the copy). A
///   reference shared only by footprints with no pad at all (logos) is allowed: each
///   is held (`Mechanical`) under the key `<reference>#<n>`, n its order among the
///   footprints with that reference in file order, from 1. `check_held` finds them by
///   that key, and the board model indexes them by it;
/// - the courtyard scan skipped a footprint (unreadable geometry): where it is cannot
///   be proved;
/// - nothing is left to move.
///
/// The text is the only evidence. That is the placement loop's rule that a lock record
/// counts only from a saved file known to be current, and a copy is current by
/// construction. A product caller, if one is ever allowed (owner decision 1), must
/// first refuse a board open in KiCad (`refuse_if_board_open_in_kicad`), as the
/// deprecated planner's apply does.
pub(crate) fn held_set(
    text: &str,
    caller: &BTreeSet<String>,
    rules: &HoldRules,
) -> Result<HeldSet, HeldRefusal> {
    todo!()
}

/// Proves that `candidate` keeps every held part exactly (`at`, rotation and side
/// compared as read, with no tolerance) and has the same footprint references as the
/// set's board, each once. Each difference is one line.
pub(crate) fn check_held(set: &HeldSet, candidate: &SexpNode) -> Result<(), Vec<String>> {
    todo!()
}

/// The root anchors and rotations of `references` as `tree` has them, in reference
/// order. Errs naming a reference that is not on the board exactly once.
pub(crate) fn placements_of(
    tree: &SexpNode,
    references: &BTreeSet<String>,
) -> anyhow::Result<Vec<konnect_ipc::types::IpcFootprintPlacement>> {
    todo!()
}

/// Whether two boards are the same board: equal parsed trees, with an atom that
/// reads as a number on both sides compared as a number to within 1e-6 (mm or
/// degrees) and every other atom compared exactly. Formatting and whitespace do not
/// count. Errs with the path of the first difference.
pub(crate) fn same_board(a: &SexpNode, b: &SexpNode) -> Result<(), String> {
    todo!()
}

/// Where a functional group came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GroupSource {
    Caller,
    /// A KiCad `(group …)` on the board that holds footprints.
    KicadGroup,
    /// One schematic sheet instance: the footprints whose `(path …)` share every
    /// element but the last.
    Sheet,
}

/// Parts that belong together (checklist 2: group by function).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Group {
    pub name: String,
    pub source: GroupSource,
    pub members: BTreeSet<String>,
}

/// A capacitor and the IC pad it decouples, as `score_placement` associated them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DecouplingPair {
    pub cap: String,
    pub cap_pad: String,
    pub ic: String,
    pub ic_pad: String,
    pub net: String,
    pub limit_mm: f64,
    /// `association_basis`, as the score gave it.
    pub basis: String,
}

/// What steers the generators beyond the netlist.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(crate) struct Intent {
    pub groups: Vec<Group>,
    pub pairs: Vec<DecouplingPair>,
    /// SHA-256 of this record as `search.json` writes it.
    pub sha256: String,
}

/// One pair per entry of `evaluated["decoupling_associations"]` (an
/// `evaluate::evaluate` result for the input board), whatever the entry's status;
/// nothing else. A pair is never inferred from a shared net, ground or not. Errs on an
/// entry that lacks a field.
pub(crate) fn decoupling_pairs(evaluated: &serde_json::Value) -> anyhow::Result<Vec<DecouplingPair>> {
    todo!()
}

/// The groups: the caller's first, then each KiCad `(group …)` that holds footprints,
/// then one per schematic sheet instance when the board has at least 2. Held members
/// are dropped (they do not move), and a group left with fewer than 2 members is
/// dropped. A part may be in several groups.
pub(crate) fn groups(tree: &SexpNode, held: &HeldSet, caller: &[Group]) -> Vec<Group> {
    todo!()
}
```

### `place_generators.rs`

```rust
//! Placement candidates. Each generator is a pure function of the board model and a
//! recipe (family, seed, parameters): the same pair gives the same candidate on any
//! thread, in any order, in any run of the same build. Randomness is
//! `route_search::SplitMix64`, seeded by the recipe alone.

use crate::place_held::{HeldSet, Intent};
use crate::route_search::SplitMix64;
use crate::tools::placement::force_core::{FdExtras, FdPart, Rect, Relaxed, Springs};
use konnect_ipc::types::IpcFootprintPlacement;
use konnect_sexp::board::{FootprintCourtyard, PcbConnectivityIndex};
use konnect_sexp::outline::Outline;
use serde::Serialize;

/// Bumped whenever any generator's arithmetic changes, so that a record says whether
/// this build can reproduce it.
pub(crate) const GENERATOR_VERSION: u32 = 8;

/// The grid every generator snaps centres to, and legalize searches on (the force
/// core's legacy grid).
pub(crate) const GRID_MM: f64 = 0.5;

/// Rings legalize searches before it gives up on a part.
pub(crate) const LEGALIZE_RINGS: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Family {
    /// F: the force core from a seeded scatter. Like S, it takes from O only what S0
    /// keeps: the held parts, every side and rotation, and the brief.
    Force,
    /// S: the schematic placer's clusters as shelf-packed rows, optionally polished by the core.
    Schematic,
    /// P: seeded perturbations of the input placement (start O).
    Perturb,
}

/// One candidate's full description: with the model's input, enough to rebuild it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Recipe {
    pub family: Family,
    pub seed: u64,
    pub params: Params,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Params {
    Force(ForceParams),
    Schematic(SchematicParams),
    Perturb(PerturbParams),
}

/// The force core's spring graph and run for one recipe.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct ForceParams {
    /// The weight of a net with more than `fanout_limit` distinct references (a
    /// board-wide rail). Other nets keep the legacy `net_weight`. With
    /// `fanout_limit` = `usize::MAX` no net is a rail, so any `rail_weight` gives the
    /// legacy graph; 3.0 is the legacy weight of a power name.
    pub rail_weight: f64,
    pub fanout_limit: usize,
    /// Weight of each decoupling pair's pad spring (`PadSpring`).
    pub pair_weight: f64,
    /// Weight of each functional group's centre spring (`GroupSpring`).
    pub group_weight: f64,
    pub iterations: usize,
}

/// The schematic placer, with the fanout filter the legacy lacks.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct SchematicParams {
    /// Union-find joins parts only over nets with at most this many distinct
    /// references. The legacy joined over every net, so one ground net made the whole
    /// board one cluster.
    pub fanout_limit: usize,
    /// When set, the force core runs from the grid with these parameters.
    pub polish: Option<ForceParams>,
}

/// Seeded edits of the input placement.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct PerturbParams {
    pub moves: usize,
    /// A shift moves a part's centre by up to this far.
    pub radius_mm: f64,
    /// Swaps of two movable parts with the same footprint ID are allowed.
    pub swaps: bool,
    /// Quarter and half turns of a part with exactly 2 pads are allowed.
    pub turns: bool,
}

/// Everything a generator reads, built once per board from its saved text.
#[derive(Debug, Clone)]
pub(crate) struct BoardModel {
    pub text: String,
    /// Every footprint's courtyard, sorted by reference (a part's index everywhere).
    pub parts: Vec<FootprintCourtyard>,
    pub references: Vec<String>,
    pub index: PcbConnectivityIndex,
    pub outline: Outline,
    /// The outline's bbox: the force core's bounds.
    pub bounds: Rect,
    pub movable: Vec<bool>,
    /// Distinct references per net, for the fanout filters.
    pub net_fanout: std::collections::BTreeMap<String, usize>,
    pub held: HeldSet,
    pub intent: Intent,
    /// The copper sides each part occupies: a through-hole part both, any other its own.
    pub part_sides: Vec<Sides>,
    /// The boxes of the board's fixed copper (text, graphics, rule areas that keep
    /// footprints or pads out), each with its sides. A moved part stays clear of them.
    pub copper: Vec<CopperObstacle>,
}

/// Builds the model. Refuses a text whose SHA-256 is not `held.board_sha256`, a scan
/// that skipped a footprint, and an outline that is not closed (both are refused by
/// `held_set` too; checked again because the model may be built from another call).
pub(crate) fn board_model(text: &str, held: HeldSet, intent: Intent) -> anyhow::Result<BoardModel> {
    todo!()
}

/// One generated placement: an absolute anchor and rotation for every movable part, in
/// reference order. Held parts are never listed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Candidate {
    pub placements: Vec<IpcFootprintPlacement>,
    /// The force core's, when the recipe ran it.
    pub converged: Option<bool>,
    pub iterations_run: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub(crate) enum GenerateFailure {
    /// Legalize found no legal cell for these parts.
    NoRoom(Vec<String>),
    /// The recipe does not fit the model: a parameter out of range, say.
    Invalid(String),
    /// The generator panicked; its message. Set by the pool, never by a generator.
    Panicked(String),
}

/// One candidate for `recipe`:
/// - Force: `scatter`, then `force`, then `legalize`.
/// - Schematic: `schematic_grid`, then `force` when `polish` is set, then `legalize`.
/// - Perturb: `perturb` of the input, then `legalize`.
///
/// Centres become anchors by each part's own offset (anchor minus courtyard centre,
/// turned with the part), rounded to 1 µm as the legacy handler rounds them. A part that
/// neither moved nor turned is not listed. Pure.
pub(crate) fn generate(model: &BoardModel, recipe: &Recipe) -> Result<Candidate, GenerateFailure> {
    todo!()
}

/// S0, the AI arm's start: every movable part in rows to the right of `bounds`, outside
/// the outline, in reference order, 1 mm apart with no overlap; held parts, every side
/// and every rotation as read. F and S start from their own seeded layouts but know no
/// more than S0 does.
pub(crate) fn neutral_start(model: &BoardModel) -> Vec<IpcFootprintPlacement> {
    todo!()
}

/// Each movable centre drawn uniformly in `bounds` shrunk by its half-size; held
/// parts at their own centres.
fn scatter(model: &BoardModel, rng: &mut SplitMix64) -> Vec<(f64, f64)> {
    todo!()
}

/// The schematic placer with the fanout filter: union-find over the nets with at most
/// `fanout_limit` distinct references, clusters in a seeded order, each cluster's
/// members by courtyard area descending (ties by reference, as the legacy), packed in
/// shelves: parts go left to right inside `bounds`, each at its own width plus a
/// margin, a new shelf opens below (as tall as its tallest part plus the margin) when
/// the next part does not fit the row, and a cluster may continue onto the next shelf.
/// Clusters follow each other in order with one more margin between them. The margin is
/// the first of 2, 1, 0.5 and 0.25 mm whose shelves fit inside `bounds`; when none does,
/// the 0.25 mm layout with every centre clamped into `bounds` (inset by half the part's
/// size), so that `legalize` starts on the board. The margin is a function of the board
/// and the recipe and is not recorded. Held parts at their own centres.
fn schematic_grid(model: &BoardModel, params: &SchematicParams, rng: &mut SplitMix64) -> Vec<(f64, f64)> {
    todo!()
}

/// `params.moves` seeded edits of the input placement, each one of: a shift by up to
/// `radius_mm`; a swap of two movable parts with the same footprint ID (`swaps`); a
/// quarter or half turn of a 2-pad part (`turns`). Returns centres and rotations.
fn perturb(model: &BoardModel, params: &PerturbParams, rng: &mut SplitMix64) -> (Vec<(f64, f64)>, Vec<f64>) {
    todo!()
}

/// The force core from `start` with `spring_graph`, the pairs' pad springs and the
/// groups' centre springs, on `model.bounds`; then its collision pass.
fn force(model: &BoardModel, params: &ForceParams, start: &[(f64, f64)]) -> Relaxed {
    todo!()
}

/// The spring graph of `params`: `force_core::springs` with the weight
/// `rail_weight` for a net above `fanout_limit` distinct references, else `net_weight`.
fn spring_graph(model: &BoardModel, params: &ForceParams) -> Springs {
    todo!()
}

/// The pairs' pad springs and the groups' centre springs, with the recipe's weights.
fn extras(model: &BoardModel, params: &ForceParams) -> FdExtras {
    todo!()
}

/// The core's view of every part: size (after any turn), side, movable.
fn core_parts(model: &BoardModel, sizes: &[(f64, f64)]) -> Vec<FdPart> {
    todo!()
}

/// Containment and overlap repair against the true outline. Held parts settle first,
/// and so does every movable part at exactly its input centre and size: it is legal as
/// it is, whatever it overlaps or overhangs (the input may hold that on purpose, and P1
/// is relative to the input's own hard failures). Then each part the recipe moved,
/// the largest courtyard box first (`sizes[i]`, width times height; equal areas by
/// reference), except that a decoupling cap settles right after its IC (first of the
/// moved parts when the IC is held or unmoved). A cap searches from the IC pad and takes
/// the cell of each ring that puts its pad nearest the IC pad. When its courtyard bbox (`sizes[i]` at `centres[i]`) overhangs the
/// outline (`Outline::overhang` deeper than `containment_threshold_mm`) once the box is
/// grown by `model.clearances.edge_mm`, or comes within 2 µm (`part_gap_mm` when either
/// box is not a real courtyard) of a settled part that shares a side with it (moved or
/// not, held or not), or the larger of 2 µm and `part_gap_mm` of a copper obstacle on its
/// sides (unmoved parts are not tested against obstacles), it moves to the
/// nearest legal cell, spiralling on `GRID_MM` for up to `LEGALIZE_RINGS` rings. Every
/// part is placed, or the result is `NoRoom` naming the parts that were not.
pub(crate) fn legalize(
    model: &BoardModel,
    sizes: &[(f64, f64)],
    centres: &mut [(f64, f64)],
) -> Result<(), GenerateFailure> {
    todo!()
}

/// A courtyard bbox turned by `quarter_turns` x 90° about `anchor`, in KiCad's
/// Y-down, counter-clockwise-on-screen sense (`konnect_sexp::geometry`'s). Exact for a
/// bbox the scan made at a cardinal angle; a superset otherwise, as the scan's own is.
pub(crate) fn turned_bbox(bbox: Rect, anchor: (f64, f64), quarter_turns: i32) -> Rect {
    todo!()
}

/// A seeded value in `[0, 1)`: the top 53 bits of the next draw.
pub(crate) fn unit(rng: &mut SplitMix64) -> f64 {
    todo!()
}

/// The fixed recipe matrix of one board.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Matrix {
    pub force_seeds: Vec<u64>,
    pub force: Vec<ForceParams>,
    pub schematic_seeds: Vec<u64>,
    pub schematic: Vec<SchematicParams>,
    pub perturb_seeds: Vec<u64>,
    pub perturb: Vec<PerturbParams>,
}

impl Matrix {
    /// The bench's matrix (design decision D14): F seeds 1..=32 x rail weight
    /// {0, 0.5, 3} on nets above 8 references (pair weight 10, group weight 0.5, 600
    /// iterations);
    /// S seeds 1..=32 x {raw, polished with rail weight 0.5}; P seeds 1..=64 x
    /// {4 edits within 2 mm, 12 edits within 5 mm}, swaps and turns on. 288 recipes.
    pub(crate) fn bench() -> Self {
        todo!()
    }
}

/// Every recipe of `matrix`, in a fixed order: F, then S, then P; within a family,
/// seed-major, then the parameter sets in the order given.
pub(crate) fn recipes(matrix: &Matrix) -> Vec<Recipe> {
    todo!()
}
```

### `place_judge.rs`

```rust
//! One judge for every candidate, references included: the placement's own checks,
//! then what the product's best-of-N routing call makes of it, then one rank key.

use crate::place_held::{DecouplingPair, HeldSet};
use crate::route_judge::{Gates, Measures};
use konnect_sexp::parser::SexpNode;
use serde::Serialize;
use std::collections::BTreeMap;

/// The cheap measures of one candidate, from its `evaluate::evaluate` result (run
/// with every held part as `edge_mount_references`, so a held part that sits over the
/// edge on purpose is listed, not failed) and its own board.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Screen {
    pub verdict: String,
    pub score: i64,
    pub score_incomplete: bool,
    pub hard_failures: usize,
    /// Hard failures that name at least one movable part. A failure among held parts
    /// alone is the same on every candidate, O included, and is no candidate's fault.
    pub movable_hard_failures: usize,
    /// Those of them that are not among the input board's own (kind and sorted
    /// references). P1 reads this count, so O never fails P1 on its own faults.
    pub new_movable_hard_failures: usize,
    /// `outline_missing` or `outline_unproven`.
    pub outline_unproven: bool,
    /// The intent's decoupling pairs (taken from O) that this board breaks
    /// (`pair_violations`). Measured on the fixed pair list, never on the candidate's
    /// own associations, so a capacitor moved away from every IC counts.
    pub decoupling_violations: usize,
    /// References of the score's `connector_edge` deduction (`deductions`).
    pub connector_violations: usize,
    /// `routing_metrics`: straight-airwire MST length and crossings; `None` when the
    /// score could not establish them.
    pub wirelength_mm: Option<f64>,
    pub crossings: Option<usize>,
    /// Mean member distance to its group's centre over the board diagonal, averaged
    /// over the groups. Recorded, not ranked (decision D8).
    pub group_spread: Option<f64>,
}

/// A hard failure as P1 compares it with the input board's: its kind and its sorted
/// references.
pub(crate) type FailureKey = (String, Vec<String>);

/// The hard failures of `evaluated` that name at least one movable part, as keys.
pub(crate) fn movable_failure_keys(
    evaluated: &serde_json::Value,
    held: &HeldSet,
) -> anyhow::Result<Vec<FailureKey>> {
    todo!()
}

/// The screen of one candidate: `evaluated` read, its hard failures split by `held` and
/// by `baseline` (the input board's `movable_failure_keys`; empty judges every failure
/// as new), `decoupling_violations` as given. Errs on a field the score always gives
/// that is missing.
pub(crate) fn screen(
    evaluated: &serde_json::Value,
    held: &HeldSet,
    baseline: &[FailureKey],
    decoupling_violations: usize,
    group_spread: Option<f64>,
) -> anyhow::Result<Screen> {
    todo!()
}

/// The pairs of `pairs` whose capacitor pad and IC pad lie farther apart on `board`
/// than the pair's `limit_mm` (straight pad-anchor distance, the score's own metric),
/// plus every pair whose pads cannot both be found once on `board`: a pair is never
/// left unmeasured.
pub(crate) fn pair_violations(board: &SexpNode, pairs: &[DecouplingPair]) -> usize {
    todo!()
}

/// Least is best: P1 failed (`new_movable_hard_failures` above 0 or the outline unproven),
/// score incomplete, decoupling violations, connector violations, crossings,
/// wirelength in 0.1 mm steps. A measure the score did not give sorts last
/// (`i64::MAX`).
pub(crate) type ScreenKey = [i64; 6];

pub(crate) fn screen_key(screen: &Screen) -> ScreenKey {
    todo!()
}

/// DRC types P2 does not gate: an unrouted board has every connection open; silk and
/// text move with their footprints and are ordered, not gated (`SILK`); library
/// mismatches do not depend on placement; starved thermals and isolated copper on an
/// unrouted board are the pours' state before routing. Every other type is physical,
/// a type KiCad adds later included.
pub(crate) const NOT_PHYSICAL: &[&str] = &[
    "unconnected_items",
    "silk_overlap",
    "silk_over_copper",
    "silk_edge_clearance",
    "text_height",
    "text_thickness",
    "lib_footprint_issues",
    "lib_footprint_mismatch",
    "starved_thermal",
    "isolated_copper",
];

/// The silk types the rank key counts.
pub(crate) const SILK: &[&str] = &["silk_overlap", "silk_over_copper", "silk_edge_clearance"];

/// Each gate passes when its list is empty; each entry is one readable line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct PlacementGates {
    /// P1: no hard failure of the placement score (a courtyard overlap, a part
    /// outside the true outline) names a movable part unless the input board has the
    /// same one (kind and references), and the outline is closed.
    pub p1: Vec<String>,
    /// P2: no physical DRC type worse than on the input board, both unrouted.
    pub p2: Vec<String>,
}

impl PlacementGates {
    pub(crate) fn pass(&self) -> bool {
        todo!()
    }
}

/// P1 from `screen`. P2: every DRC type of `unrouted` (the candidate's unrouted board,
/// refilled and checked as the routing judge's baseline does) that is not in
/// `NOT_PHYSICAL`, at most `input_unrouted`'s count of that type. `unrouted` = `None`
/// (no check came back) is a P2 line, never a pass.
pub(crate) fn placement_gates(
    screen: &Screen,
    unrouted: Option<&BTreeMap<String, usize>>,
    input_unrouted: &BTreeMap<String, usize>,
) -> PlacementGates {
    todo!()
}

/// What a candidate's routing call gave.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RouteResult {
    /// The call's winner (`search.winner`: its gates and measures), the candidate's
    /// unrouted board check (`search.baseline`), the call's `status`, and the call's
    /// summary as recorded (engine, effort, lanes, outcome counts, elapsed, winner shape
    /// and seed, the search folder).
    Routed {
        gates: Gates,
        measures: Measures,
        unrouted: BTreeMap<String, usize>,
        status: String,
        call: serde_json::Value,
    },
    /// The call ran but judged no run; its baseline when it had one.
    NoWinner {
        unrouted: Option<BTreeMap<String, usize>>,
        reason: String,
    },
    /// A stage failed: `export` (the DSN export refused) or `call` (the call refused or
    /// erred).
    Failed { stage: &'static str, reason: String },
    /// Not routed: `screened_out`, `time_limit` or `stopped`.
    NotRouted { why: &'static str },
}

/// Everything the key reads about one candidate.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Judged {
    /// The candidate's id: O's routing calls first, then A's, then the generated
    /// candidates in recipe order.
    pub id: usize,
    pub screen: Screen,
    pub placement: PlacementGates,
    pub route: RouteResult,
    /// Silk DRC entries (`SILK`) on the candidate's unrouted board; `None` without one.
    pub silk: Option<usize>,
}

/// A candidate's rank key; the least is the best.
pub(crate) type Key = [i64; 16];

/// The key of every judged candidate at once (the length steps need the pool):
/// 0. P1 or P2 failed;
/// 1. not routed (`Routed` is 0; every other result 1);
/// 2. G1 failed;
/// 3. KiCad's unconnected count after the refill and stitching (`i64::MAX` when not
///    routed): a candidate that routes less can never win by being shorter;
/// 4. G2 failed; 5. G3 failed;
/// 6. decoupling violations; 7. connector violations (requirements before preferences);
/// 8 to 13. the routing key's acute corners, signal and power length steps, right
///    angles, vias off the pour nets and minus copper quality, from
///    `route_judge::rank_measures`, the steps taken against the shortest routed
///    candidate that passes P1, P2 and G1 (so an illegal placement cannot set them);
/// 14. silk entries; 15. minus the placement score.
/// Missing values sort last. Ties go to the lower id, so O, then A, then the
/// generated candidates in recipe order: a generated candidate must be strictly
/// better to rank above a reference.
pub(crate) fn rank(judged: &[&Judged]) -> Vec<Key> {
    todo!()
}
```

### `place_route.rs`

```rust
//! A candidate's way to the product's router: its own project copy, a file-based DSN
//! export, one `route_specctra_dsn` best-of-N call, and the call's result read back.
//! Nothing here writes outside the folder it is given, and nothing talks to a running
//! KiCad.

use crate::mcp::protocol::CallToolResult;
use crate::place_judge::RouteResult;
use crate::route_search::Effort;
use crate::specctra::ExportOptions;
use crate::tools::ToolContext;
use konnect_ipc::IpcEffectiveRoutingRules;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What every candidate of one board is routed with. None of it depends on placement.
#[derive(Debug, Clone)]
pub(crate) struct RouteInputs {
    /// The input board inside its stripped project copy (no tracks, arcs, vias or
    /// teardrop zones; `strip_routing`), in the bench folder.
    pub board: PathBuf,
    /// Each net's effective routing rules as KiCad resolved them, once, on the input
    /// board. Net classes follow net names and patterns, not placement.
    pub rules: IpcEffectiveRoutingRules,
    /// As `export_specctra_dsn` builds them (`export_options`).
    pub options: ExportOptions,
    pub effort: Effort,
    /// The fastroute binary, named in every call, so no call reads the user config
    /// for it.
    pub fastroute: PathBuf,
    /// The kicad-cli the unrouted checks run.
    pub kicad_cli: String,
    /// The bench's scratch `KICAD_CONFIG_HOME`.
    pub kicad_config_home: PathBuf,
}

/// Rules from KiCad's own resolution written as JSON: net name to `class_name`,
/// `constituents`, `track_width_mm`, `clearance_mm`, `via_diameter_mm`,
/// `via_drill_mm` (the router research's `net_rules.py` writes this form, from
/// `GetEffectiveNetClass`). Refuses a net with a pad on `board_text` that has no entry.
pub(crate) fn rules_from_json(text: &str, board_text: &str) -> anyhow::Result<IpcEffectiveRoutingRules> {
    todo!()
}

/// The options `export_specctra_dsn` builds for `board` with its defaults: pours
/// `fixed`, `min_copper_edge_clearance` and `min_hole_clearance` from the
/// `.kicad_pro` beside it, widths from intent on, duplicate references refused.
pub(crate) fn export_options(board: &Path) -> ExportOptions {
    todo!()
}

/// The input board with every top-level `(segment …)`, `(arc …)` and `(via …)` and
/// every teardrop zone removed, each with the whitespace before it; every other byte
/// kept. Quote-aware block scanning. Errs on text that does not parse as a board.
pub(crate) fn strip_routing(text: &str) -> anyhow::Result<String> {
    todo!()
}

/// The candidate's own project in `folder` (which must not exist yet):
/// `scratch_board::copy_project` of `inputs.board`, the candidate's text written over
/// the copy's board (same file name), then `<stem>.dsn` and `<stem>.dsn.konnect.json`
/// from `specctra::export_dsn_with_context` with `inputs.rules`, the copy's
/// `routing_context::Snapshot` and `inputs.options`. The manifest names the copy as
/// `board_path` and the text's SHA-256 as `source_sha256`, so `route_judge::prepare`
/// accepts it. Returns the DSN's path; an export refusal is the error.
pub(crate) fn export_candidate(inputs: &RouteInputs, folder: &Path, board_text: &str) -> anyhow::Result<PathBuf> {
    todo!()
}

/// One best-of-N routing call: `route_fastroute::route_with_fastroute` with
/// `{dsn_path, ses_output_path: <folder>/route.ses, engine: "fastroute",
/// effort: inputs.effort.name(), fastroute_path: inputs.fastroute}` (the effort as the
/// lower-case name `resolve_effort` reads), read by `read_route_result`. An `Err` from
/// the call is `Failed { stage: "call" }`. A call that succeeds keeps its search folder
/// (`<folder>/route.search`); one that does not has it set aside by the call
/// (`<folder>/route.search.failed-<n>`), and the result's `search_folder` names where,
/// which the record keeps.
pub(crate) async fn route_candidate(
    ctx: Arc<ToolContext>,
    inputs: &RouteInputs,
    dsn: &Path,
    folder: &Path,
) -> RouteResult {
    todo!()
}

/// The call's result read back, its `search` object first:
/// - a result (error or not) whose `search.winner` is null is `NoWinner`, with
///   `search.baseline.violations` when `search.baseline.ok` is true. This is how the
///   call reports "no run was routed and judged": an error result that still carries
///   its `search`;
/// - an error result with no `search` is `Failed { stage: "call" }` with its text (a
///   refusal before the search: the binary, the preflight, an argument);
/// - otherwise `Routed`, with the winner's `gates` and `measures` (deserialized into
///   `route_judge`'s own types), `search.baseline.violations`, `status` and the
///   summary.
pub(crate) fn read_route_result(result: &CallToolResult) -> RouteResult {
    todo!()
}

/// The input board's unrouted check: its text in a scratch project under `folder`,
/// then `tools::cli::refill_drc_on_private_copy_with_config` with
/// `inputs.kicad_config_home`, violations counted by type. P2's yardstick.
pub(crate) async fn unrouted_check(inputs: &RouteInputs, folder: &Path) -> anyhow::Result<BTreeMap<String, usize>> {
    todo!()
}

/// The tool context every bench call runs with: the default server config with
/// `kicad_cli`, and `ipc_address` set to a socket path inside `folder` that nothing
/// listens on, so every IPC attempt of every tool the judge calls is `Unreachable` and
/// takes its saved-file path. Refuses a `folder` that is not empty or not absolute.
pub(crate) fn isolated_context(folder: &Path, kicad_cli: &str) -> anyhow::Result<Arc<ToolContext>> {
    todo!()
}
```

### `place_search.rs`

```rust
//! The placement search of one board: generate many candidates on threads, screen
//! them, route the references and the best of each family one call at a time, rank
//! everything under one judge, and record every candidate so that it can be rebuilt.
//! It writes only below its own folder, and never the input board.

use crate::place_generators::{BoardModel, Matrix, Recipe};
use crate::place_judge::{Judged, Key, RouteResult, Screen, ScreenKey};
use crate::place_route::RouteInputs;
use crate::route_search::{BoxFuture, Effort};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Where a candidate came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Origin {
    /// O: the board as the owner placed it, routed `Plan::reference_calls` times;
    /// `call` counts from 1. Ids `0..reference_calls`.
    Owner { call: usize },
    /// A: the AI's placement from S0, read from the bench folder and admitted by
    /// `admit_ai`, routed the same number of times. The ids after O's.
    Ai { board: PathBuf, call: usize },
    Generated { recipe: Recipe, generator_version: u32 },
}

/// Wall times of one candidate's stages.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(crate) struct Timings {
    pub generate_ms: Option<f64>,
    pub write_ms: Option<f64>,
    pub evaluate_ms: Option<f64>,
    pub export_s: Option<f64>,
    pub route_s: Option<f64>,
}

/// One candidate, as `search.json` lists it.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Record {
    pub id: usize,
    pub origin: Origin,
    /// SHA-256 of the candidate's board text; `None` when no board was made.
    pub board_sha256: Option<String>,
    pub input_sha256: String,
    pub held_sha256: String,
    pub intent_sha256: String,
    /// `generated`, `generate_failed`, `held_moved`, `write_failed`, `screen_failed`,
    /// `duplicate` (its board equals an earlier candidate's, named in `reason`),
    /// `ai_refused`, `screened_out`, `routed`, `no_winner`, `export_refused`,
    /// `route_failed`, `not_routed_time_limit` or `not_routed_stopped`.
    pub outcome: &'static str,
    pub reason: Option<String>,
    pub converged: Option<bool>,
    pub screen: Option<Screen>,
    pub screen_key: Option<ScreenKey>,
    pub key: Option<Key>,
    /// 1 is the best, over the judged candidates of this board.
    pub rank: Option<usize>,
    pub timings: Timings,
    /// The routing call as recorded: its folder, engine (path, SHA-256, version),
    /// effort, lanes, outcome counts, elapsed and overrun, the winner's shape and seed,
    /// the host's 1-minute load at its start.
    pub route: Option<serde_json::Value>,
    #[serde(skip)]
    pub board_text: Option<String>,
    #[serde(skip)]
    pub judged: Option<Judged>,
}

/// One board's search.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    /// The board's bench name (`B4`, `B5`, `B6`, or a public fixture's).
    pub name: String,
    /// The board's bench folder; everything is written below it.
    pub folder: PathBuf,
    pub matrix: Matrix,
    /// Generation threads: the machine's cores.
    pub threads: usize,
    /// Routing calls for each reference, O and A (3): the spread of a reference's own
    /// calls is the routing noise a family's best must clear.
    pub reference_calls: usize,
    /// Per family, this many best by the screen key are routed ...
    pub best_per_family: usize,
    /// ... and this many drawn, with `draw_seed`, from the family's rest that passes P1.
    pub drawn_per_family: usize,
    pub draw_seed: u64,
    /// The board's whole time limit, generation included.
    pub time_limit: Duration,
    pub route: RouteInputs,
    /// A's board, when the bench has one.
    pub ai_board: Option<PathBuf>,
}

/// How one board's search ended.
#[derive(Debug, Clone)]
pub(crate) struct Outcome {
    pub records: Vec<Record>,
    pub elapsed: Duration,
    /// Why the routing phase stopped early, when it did, in full: `search.json` only.
    pub stopped_routing: Option<String>,
    /// The same as one fixed word, the only form `table.md` prints: `time_limit`,
    /// `input_check_failed` or `calls_failed_in_a_row`.
    pub stopped_code: Option<&'static str>,
    /// P2's yardstick: the input board's unrouted check.
    pub input_unrouted: BTreeMap<String, usize>,
}

/// The routing phase's two jobs, so that the scheduler is tested with fakes on a
/// paused clock. The production one wraps `place_route` with an isolated context.
pub(crate) trait RouteCall: Send + Sync + 'static {
    /// Export and route one candidate in its own new `folder`.
    fn route(&self, id: usize, board_text: String, folder: PathBuf) -> BoxFuture<(RouteResult, Timings)>;
    /// The input board's unrouted check (`place_route::unrouted_check`).
    fn unrouted(&self, board_text: String, folder: PathBuf) -> BoxFuture<anyhow::Result<BTreeMap<String, usize>>>;
}

/// Phase 1, generation. `threads` scoped threads pull the next recipe index from one
/// atomic counter; each job generates (`place_generators::generate`), writes the board
/// text (`pcb_components::placements_applied_to`), proves the held set
/// (`place_held::check_held`), evaluates (`evaluate::evaluate`, every held part as
/// `edge_mount_references`), measures the intent's pairs (`place_judge::pair_violations`)
/// and screens. A panic in a job is caught (`std::panic::catch_unwind`) and recorded
/// `generate_failed` with its message; no job's failure stops another. Each result
/// lands in its recipe's slot, so the order in which jobs finish changes nothing. Ids
/// are `first_id + slot`. After the pool, a record whose board equals a lower id's
/// (O and A included) becomes `duplicate`.
pub(crate) fn generate_all(model: &BoardModel, recipes: &[Recipe], threads: usize, first_id: usize) -> Vec<Record> {
    todo!()
}

/// Phase 2, selection, as candidate ids in routing order: O's calls, then A's; then,
/// per family, among the candidates that pass P1 (one that fails it is never routed),
/// its `best` best by the screen key (ties to the lower id) and `drawn` drawn uniformly
/// with `SplitMix64(draw_seed)` from its other candidates that pass P1.
/// The families are interleaved round by round: each family's best, then each
/// family's drawn one, then the second best, then the rest, and round r starts at
/// family r mod 3 (F1, S1, P1; Sd, Pd, Fd; P2, F2, S2; …), so a time limit cuts every
/// family alike over the rounds and the screen's check is not the first thing cut.
/// A candidate whose board equals an earlier one's
/// (`board_sha256`) was recorded `duplicate` in phase 1 and is never selected; nor is a
/// record that failed before the screen.
pub(crate) fn routing_order(records: &[Record], best: usize, drawn: usize, draw_seed: u64) -> Vec<usize> {
    todo!()
}

/// What a routing call is expected to take before any has finished: the effort's
/// budget plus 60 s, a margin this design chooses for the judge's tail after the
/// budget (route design D7 lets a call that has no judged run yet wait past its
/// budget without a bound, so the median takes over as soon as one call ends). Fast
/// has no budget: the hang cap.
pub(crate) fn first_call_estimate(effort: Effort) -> Duration {
    todo!()
}

/// The launch rule: a routing call started at `elapsed` is expected to end by
/// `limit`: `elapsed + expected <= limit`, `expected` the median wall time of the calls
/// that routed a candidate so far (a call with no winner is not one), or
/// `first_call_estimate` before any. A call is never cut once
/// started: its own budget ends it (route design D7).
pub(crate) fn may_route(elapsed: Duration, limit: Duration, call_walls: &[Duration], effort: Effort) -> bool {
    todo!()
}

/// Calls in a row that gave no routed candidate (`export_refused`, `route_failed` and
/// `no_winner`), with no routed candidate yet, that stop the routing phase. A routed
/// candidate resets the count, and after the first one the phase never stops this way.
pub(crate) const ROUTE_FAILURES_BEFORE_STOP: usize = 3;

/// The whole search of one board (flow steps 1 to 8): the held set and the intent,
/// the model, the input's unrouted check, the references and phase 1, selection, then
/// the routing phase one call at a time in `routing_order` while `may_route` allows,
/// each candidate's P gates and route result, the keys, and `search.json` and
/// `table.md` in `plan.folder`. Errs only before phase 1 (a held set that cannot be
/// proved, an input that does not parse); every later failure is a record.
pub(crate) async fn search(plan: &Plan, routing: Arc<dyn RouteCall>) -> anyhow::Result<Outcome> {
    todo!()
}

/// A's board admitted as a reference: the anchors and rotations of the movable parts
/// read from `ai_text` (`place_held::placements_of`), applied to S0's text with
/// `pcb_components::placements_moved_applied_to` (a part whose rotation is S0's is
/// moved, as the move tool moves it; only a turned part is set), and the result must be
/// the same board as `ai_text` (`place_held::same_board`). So A differs from S0 only in where movable
/// parts are and how they turn: a flip, a changed held part, net, zone, outline or rule
/// is refused, and the refusal (`ai_refused`, with the first difference) is a record.
/// Returns the rebuilt text, which is what is judged.
pub(crate) fn admit_ai(s0_text: &str, ai_text: &str, movable: &std::collections::BTreeSet<String>) -> Result<String, String> {
    todo!()
}

/// Rebuilds a generated record's board text and proves it is the recorded one. Errs
/// when the record is not a generated one, when its `generator_version`, input,
/// held or intent SHA-256 differ from this build's model, or when the rebuilt text's
/// SHA-256 is not `board_sha256`.
pub(crate) fn rerun(model: &BoardModel, record: &Record) -> anyhow::Result<String> {
    todo!()
}

/// `search.json`: the plan (matrix, limits, effort, fastroute, threads, host cores),
/// every record, the input's unrouted check, and how the search ended.
pub(crate) fn search_json(plan: &Plan, outcome: &Outcome) -> serde_json::Value {
    todo!()
}

/// `table.md` of one board, counts only, no net or part names: one row per routed
/// candidate in rank order (rank, origin, family and seed, P1, P2, G1, unconnected,
/// G2, G3, decoupling, connector, acute, signal and power mm, vias off the pours,
/// copper quality, silk, placement score), then one row per family and reference (best
/// rank, median rank of its routed candidates, how many pass every gate, how many were
/// generated, failed, duplicates and screened out), then each family's verdict against
/// O and against A (`verdict`; a family's best is its best routed candidate, a
/// reference's calls are its routed calls and its calls with no winner, and a no-winner
/// call is the reference's worst rank: the table's legend says so). A stop is named by
/// its fixed code only, never by the reason's text.
pub(crate) fn table(plan: &Plan, outcome: &Outcome) -> String {
    todo!()
}

/// A family's verdict against one reference, by rank (1 is best).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Verdict {
    /// The family's best ranks above every one of the reference's calls.
    Beats,
    /// It ranks between the reference's best and worst call: inside its routing noise.
    Matches,
    /// It ranks below every one of the reference's calls.
    Below,
    /// The family or the reference has no routed candidate.
    Unknown,
}

/// `Verdict` from the family's best rank and the ranks of the reference's calls.
pub(crate) fn verdict(family_best: Option<usize>, reference_calls: &[usize]) -> Verdict {
    todo!()
}
```

### `place_bench_tests.rs`

```rust
//! The placement bench, an ignored test: it runs the placement search on copies of
//! boards and changes nothing in the product. One run per board named in the bench
//! folder's `bench.json`:
//!
//! ```text
//! KONNECT_PLACE_BENCH=<bench folder> KONNECT_FASTROUTE=<candidate fastroute>
//! KICAD_CONFIG_HOME=<scratch> KONNECT_STATE_DIR=<scratch>
//! cargo test -p konnect-core --release place_bench -- --ignored --nocapture
//! ```
//!
//! `bench.json` names, per board: its folder below the bench folder (holding
//! `input/` with the stripped project copy, `rules.json`, `held.json` and optionally
//! `ai/board.kicad_pcb`), its time limit, the effort, `hold` (`locks` or `rules`) and
//! `families`. The bench writes only below the bench folder.

use std::path::PathBuf;

/// The checked environment of one bench run.
#[derive(Debug, Clone)]
pub(crate) struct BenchEnv {
    pub folder: PathBuf,
    pub fastroute: PathBuf,
    pub kicad_cli: String,
    pub kicad_config_home: PathBuf,
    pub state_dir: PathBuf,
}

/// The bench's guards, before anything runs. Refuses when: `KONNECT_PLACE_BENCH` is
/// unset or not an absolute folder; `KICAD_CONFIG_HOME` or `KONNECT_STATE_DIR` is
/// unset, or is KiCad's or Konnect's real folder (`~/Library/Preferences/kicad`,
/// `~/.local/share/konnect`), or is not below a scratch root (the bench folder or the
/// system temp folder); `KONNECT_FASTROUTE` is not an executable file; any input board
/// named in `bench.json` resolves (links followed) to a path outside the bench
/// folder; kicad-cli is missing.
pub(crate) fn bench_environment() -> anyhow::Result<BenchEnv> {
    todo!()
}

/// Every board of `bench.json`: the plan (`Matrix::bench`, 3 calls per reference, 3
/// best and 1 drawn per family, draw seed 1, the board's time limit, effort M), the
/// production `RouteCall`
/// with `place_route::isolated_context`, then `place_search::search`. A board that
/// errs is reported and the next one runs. Prints one line per board: its table's
/// path and its counts.
#[test]
#[ignore = "needs KONNECT_PLACE_BENCH, KONNECT_FASTROUTE, KICAD_CONFIG_HOME and KONNECT_STATE_DIR"]
fn place_bench() {
    todo!()
}
```

**Compile check, 2026-11-18.** After the independent review's fixes, the nine skeleton files above were pasted into this branch, with their `mod` lines in `lib.rs` and `tools/placement.rs`. `cargo check -p konnect-core --tests` (rustc 1.96.0) finished with no error. Its 218 warnings are all unused variables and unused items of the `todo!()` bodies, all in the new files. The files were then removed: this branch carries the design only.

## Tests, each guard with a negative control

| Guard | Test (file) | Negative control |
|---|---|---|
| The force core's extraction changes nothing | `force_directed_output_is_unchanged_by_the_pull_out` (`tools/placement.rs`): the tool's JSON for six cases (default; `locked: ["U1"]`; `references: ["C1"]` with `max_displacement_mm`; `iterations: 50`; the far-cap board; a KiCad-locked footprint) equals a golden file recorded from `d5588205` before the change by a one-off ignored recorder | `FD_DAMPING` 0.85 → 0.86 inside the core, or the walls moved before the springs, fails it |
| The score's extraction changes nothing | `score_placement_output_is_unchanged_by_the_pull_out` (`tools/placement/scoring_tests.rs`): goldens for the rectangle, rounded, cut-out, missing, open, overlap, outside, decoupling and interface-filter fixtures | Leaving the connector-edge deduction out of `evaluate` fails it |
| The in-memory writer is the file writer | `placements_applied_to_equals_the_closed_board_write` (`tools/pcb_components.rs`): set, move and turn cases give the same text both ways | Dropping the rotation from the in-memory path fails it |
| `rank` keeps its keys | `rank_is_rank_measures_with_every_run_in_the_pool` (`route_judge_tests.rs`), and the existing rank tests unchanged | Taking the pool from the runs not marked changes the keys |
| The judge's types read back | `measures_and_gates_survive_a_round_trip` (`route_judge_tests.rs`) | — |
| Each hold reason holds | `lock_caller_connector_and_mechanical_parts_are_held` (`place_held_tests.rs`): an interior `J`, an `H` with a netted pad, a `MountingHole` footprint with another prefix, a part with no netted pad | The same footprints renamed `R…` with a netted pad become movable |
| `locks` holds only locks and pad-less parts | `locks_only_holds_the_locked_parts_and_frees_the_connector`, `locks_only_still_holds_parts_with_no_pad_on_a_net` (`place_held_tests.rs`): an unlocked `J`, `H` and fiducial move; a locked `J` and `L` are held; a pad-less part and the `<reference>#<n>` logos are held | The same board under `HoldRules::default()` holds the `J`, `H` and fiducial |
| The bench's options | `hold_defaults_to_locks_and_rules_keeps_the_old_rules`, `families_default_to_force_and_schematic_and_perturb_brings_p_back`, `an_unknown_hold_or_family_or_an_empty_list_is_refused` (`place_bench_tests.rs`): default `locks`, default F and S (160 recipes, the same F and S recipes as in the 288), an unknown value or an empty list refused with its word | `"hold": "rules"` and `"perturb"` bring back the old rules and the 288 |
| Boards compare as boards | `same_board_ignores_formatting_not_values`: re-spaced text and `1.5` against `1.50` are the same; a 0.01 mm shift is not | — |
| Refuse what cannot be proved | `held_set_refuses_what_it_cannot_prove`: open outline, unknown caller reference, shared reference, a skipped courtyard, a missing reference, nothing movable, each with its next step | The clean fixture is accepted |
| Held parts never move | `a_candidate_that_moves_a_held_part_is_refused`: a 1 µm shift, a turn, a flip, a dropped footprint, an added one | The unchanged text passes |
| Pairs never come from ground | `pairs_come_from_associations_never_from_ground`: a capacitor sharing only GND with an IC gives no pair | Sharing a non-ground rail gives the pair |
| Groups | `groups_drop_held_members_and_groups_of_one`; `sheet_groups_follow_the_footprint_paths` | — |
| Same recipe, same board | `the_same_recipe_gives_the_same_board_bytes` (`place_generators_tests.rs`): twice, and in pools of 1 and 8 threads | Seed 2 differs from seed 1, which proves the seed reaches the generator |
| Generators leave held parts alone | `generators_never_list_a_held_part`, every family, every hold reason | — |
| Containment in the true outline | `legalize_puts_every_part_inside_the_true_outline`: a rounded-corner fixture with parts scattered into the corners | Without legalize at least one part overhangs an arc (the fixture tests the arcs) |
| No guess when there is no room | `legalize_says_no_room_instead_of_guessing`: an over-full board gives `NoRoom` naming the parts | — |
| Unmoved parts are legal as they are | `legalize_leaves_unmoved_parts_alone_even_where_the_input_overlaps_or_overhangs`: two unmoved parts that overlap and one over the edge stay, with no `NoRoom`; a moved part still lands clear of them and inside | Repairing unmoved parts moves them, or fails with `NoRoom` |
| Clearances beyond the courtyards | `parts_without_a_courtyard_are_legalized_apart_by_the_part_gap`; `two_real_courtyards_may_still_touch_closely`; `a_moved_part_keeps_the_edge_clearance_inside_a_rounded_outline` (a straight edge and an arc); `the_clearances_come_from_the_rules_and_the_export_options` | No part gap leaves pads-only parts 0.5 mm apart; applying the gap to courtyards too moves parts that may touch; no edge clearance leaves a part 0.3 mm from the edge |
| Copper obstacles | `a_moved_part_is_moved_off_copper_text_and_not_off_silkscreen_text`; `a_moved_part_keeps_the_part_gap_from_copper`; `a_copper_text_box_follows_its_size_lines_thickness_and_justification`; `a_turned_copper_text_box_is_turned_about_its_anchor`; `a_copper_text_with_a_render_cache_is_boxed_by_the_cache`; `copper_graphics_are_boxed_by_their_geometry_grown_by_half_the_stroke`; `an_arc_box_covers_the_bulge`; `a_near_straight_arc_is_a_thin_box_and_does_not_block_a_part_beside_it`; `a_polygon_box_includes_the_extent_of_its_arcs`; `a_text_estimate_follows_line_spacing_a_default_size_and_knockout`; `copper_items_take_the_sides_of_their_layers`; `a_part_is_moved_off_copper_graphics_on_its_side` | The same text on `F.SilkS` does not move the part; a text turned 90° or justified left has the box of the turned or the anchored estimate; an arc over the top reaches y = 30 although none of its points does, and stays above y = 40; a 20 mm arc 0.01 mm high is a thin box, and a part 1 mm from it stays |
| Rule areas | `a_rule_area_that_keeps_parts_out_blocks_a_moved_part` (footprints, pads, the legacy `layer`, the back); `a_rule_area_that_keeps_out_only_tracks_vias_or_pours_is_no_obstacle` (and a pour); `a_rule_area_with_an_arc_is_boxed_by_the_arc_extent` | A tracks-only, vias-only or pour-only keepout and a plain pour leave the part where it is |
| Part sides | `a_through_hole_part_occupies_both_sides_and_another_its_own` (`thru_hole` and `np_thru_hole`); `a_through_hole_part_and_a_bottom_part_that_overlap_are_not_both_legal`; `a_part_meets_copper_text_only_on_a_side_it_occupies` | Two SMD parts on opposite sides may overlap; a top part over `B.Cu` text stays, a through-hole one does not; an inner-layer text blocks both |
| Unmoved parts and obstacles | `an_unmoved_part_on_copper_text_stays_with_no_room_trouble` | A part on copper text in the input stays, with no `NoRoom`; a moved one lands clear of the text and the part |
| Largest first | `a_big_part_fits_because_it_settles_before_the_small_one`; `parts_settle_by_decreasing_area_and_equal_areas_by_reference`; `held_and_unmoved_parts_still_settle_first_whatever_their_size` | A 38 x 38 mm part on a 60 x 40 mm board gets `NoRoom` when a 2 x 1 mm part with the smaller reference settles first; equal areas keep the reference order; a larger moved part never moves a held or unmoved one |
| Decoupling caps | `an_ics_caps_settle_right_after_it_and_land_within_their_limit`; `a_caps_search_starts_at_the_ic_pad_not_at_its_own_centre`; `a_cap_without_a_measured_pair_keeps_its_place`; `the_caps_of_a_moved_ic_land_by_its_final_pad`; `unmoved_and_held_parts_stay_where_a_cap_search_runs` (also: the same twice); `a_turned_cap_measures_its_pad_from_the_turned_offset`; `the_settling_order_puts_a_cap_after_its_ic_and_the_caps_of_a_fixed_ic_first` | Without the pairs a crowd of larger parts pushes a cap 7 mm from its pin; a cap far away and legal stays there; a pair that cannot be measured changes nothing |
| Generator version | `the_generator_version_is_eight` | — |
| The diagnosis is in search.json only | `records_carry_the_gate_lines_and_the_unrouted_counts_in_search_json_only`: `placement_gates`, `unrouted` and `route_gates` on each judged record, none in `table.md` | — |
| The shelves fit the board | `the_schematic_shelves_take_a_smaller_margin_when_the_wide_one_overflows`: 40 parts whose 2 mm shelves are 24 mm on a 20 mm board get the 1 mm shelves; `a_cluster_with_one_large_part_fits_a_board_the_cell_grid_overflowed`: a socket and 30 small parts in one cluster, no `NoRoom`; `shelves_keep_a_cluster_together_and_clamp_when_nothing_fits` | A single 2 mm margin leaves parts outside the bounds; cells of the cluster's largest size overflow the second fixture |
| The fanout filter | `the_schematic_grid_does_not_join_over_a_rail`: GND on every part; fanout 8 gives several clusters | Fanout `usize::MAX` gives one cluster, as the legacy does |
| The legacy graph is one setting | `the_legacy_graph_is_rail_weight_three_without_a_limit` | — |
| Turns | `turned_bbox_matches_the_scan_of_a_turned_footprint`: turn a fixture footprint with the writer, scan again | The opposite sense of rotation fails it |
| Edits | `perturb_swaps_only_same_footprints_and_turns_only_two_pad_parts` | — |
| S0 | `neutral_start_is_outside_the_outline_and_overlap_free`, held parts unchanged | — |
| The matrix | `the_bench_matrix_has_288_recipes_in_a_fixed_order` | — |
| P2 gates physical types only | `p2_gates_physical_types_only` (`place_judge_tests.rs`): one more `courtyards_overlap` than the input fails; five more `silk_overlap` pass; one more of an unknown type fails | The input's own counts pass |
| No check is no pass | `p2_without_an_unrouted_check_fails` | — |
| P1 judges what a candidate changed | `a_held_part_over_the_edge_or_on_another_held_part_fails_no_candidate`; `p1_fails_only_on_a_hard_failure_the_input_board_does_not_have`: the input's own movable faults fail neither O nor a candidate that keeps them, a new one fails it | The same overhang on a movable part fails P1; counting every movable failure as new fails the second |
| Decoupling is measured on the fixed list | `moving_a_capacitor_away_is_a_decoupling_violation`: a pair's capacitor moved far from every IC counts, though the candidate's own associations drop it | The capacitor at its pad's side counts nothing |
| A pair is never unmeasured | `a_pair_whose_pads_are_missing_is_a_violation` | — |
| Routing less cannot win by being shorter | `a_shorter_candidate_with_an_open_connection_ranks_below_a_complete_one` | Swapping key fields 3 and 9 (length before unconnected) fails it |
| An illegal placement sets no steps | `an_illegal_placement_does_not_set_the_length_steps`: a very short candidate that fails P1 leaves the others' keys unchanged | Putting it in the pool changes them |
| Requirement before preference | `a_decoupling_violation_ranks_below_any_longer_route` | — |
| Ties | `ties_go_to_the_lower_id` | — |
| The screen's order | `screen_key_orders_verdict_completeness_requirements_then_airwires` | — |
| The strip | `strip_routing_removes_tracks_vias_and_teardrops_only` (`place_route_tests.rs`), every other byte kept | A zone that is not a teardrop stays |
| A candidate's DSN is the judge's | `an_exported_candidate_passes_the_judge_preflight`: `export_candidate`, then `route_judge::prepare` accepts it | One byte of the copy's board changed after the export: `prepare` refuses |
| Rules | `rules_from_json_refuses_a_net_it_lacks` | The complete file is accepted |
| The result reader | `read_route_result_reads_the_winner_the_baseline_and_failures`, from results recorded from `route_fastroute`'s fake-runner tests: a winner; the "no run was routed and judged" error result, which carries `search`; a refusal before the search | The no-winner error result is `NoWinner` with its baseline, never `Failed`; the refusal is `Failed` |
| The effort is named as the call reads it | `the_routing_call_names_the_effort_in_lower_case`: the arguments hold `"m"` | — |
| No KiCad is asked | `the_isolated_context_reaches_no_kicad`: `score_placement` through it on a fixture reports `source: saved_file`, and the IPC address lies inside the folder | — |
| Writes stay in the folder | `export_candidate_writes_only_inside_its_folder`: the parent folder's listing and hashes are unchanged | — |
| A panic is a record | `a_panicking_generator_is_a_record_not_the_end` (`place_search_tests.rs`): a test-only recipe that panics is `generate_failed` with its message; the others are generated | — |
| The pool is bounded | `the_pool_never_runs_more_jobs_than_threads` (a counter) | — |
| Routing order | `routing_order_takes_the_references_then_the_families_interleaved`: O 3 times, A 3 times, then best, drawn, second and third per family, round r starting at family r mod 3 (`the_round_robin_starts_each_round_at_another_family`); `a_candidate_that_fails_p1_is_never_the_best_of_its_family` | Without the rotation P's drawn candidate is always last |
| Duplicates | `a_repeated_board_is_a_duplicate_and_never_routed` | Two boards that differ by 1 µm are both kept |
| A is a placement only | `admit_ai_refuses_any_edit_but_placement`: a flip, a moved held part, a changed net, zone, outline or rule | S0 with movable parts moved (by the move writer) and turned (by a set) is admitted |
| A moved part is not re-spelled | `a_board_written_by_the_move_writer_is_admitted` (`place_search_tests.rs`): a part at 180 degrees with texts at 100 and 180 degrees, moved by the move tool's own writer; and `placements_moved_applied_to_moves_what_is_not_turned_and_sets_what_is` (`tools/pcb_components.rs`) | Rebuilding with a set of every part refuses a board the AI wrote correctly |
| A part that stays stays | `a_part_that_did_not_move_keeps_its_footprint_block_byte_for_byte` (`place_generators_tests.rs`); `a_perturbation_that_ends_where_it_began_is_a_duplicate_of_the_input` | Listing the unmoved parts re-spells the text of the one at 180 degrees |
| Clearance survives rounding | `legalized_parts_do_not_overlap_once_their_anchors_are_rounded_and_written`: two parts 0.1 to 1 µm from touching, anchors off the micron grid, written and scanned again | A clearance of 1 nm leaves a 0.3 µm overlap in the written board |
| The verdict clears the noise | `verdict_needs_to_clear_every_reference_call`: beats, matches, below, unknown | A family best between two reference calls is `Matches`, not `Beats` |
| The draw | `the_draw_is_fixed_and_never_draws_a_p1_failure` | — |
| The time limit | `no_call_starts_that_cannot_end_by_the_limit`: `may_route` table, and a fake router with fixed wall times on a paused clock | Without the check, calls run past the limit |
| Failures are records | `routing_failures_are_records_and_three_in_a_row_stop`; `calls_with_no_winner_count_toward_the_stop_like_failures` | A routed candidate between the failures keeps the phase going |
| A call with no winner is no timing | `a_call_with_no_winner_does_not_set_the_launch_rules_median` | The median of every call ends the search at the first slow no-winner call |
| A no-winner reference call ranks last | `a_reference_call_with_no_winner_is_the_references_worst_rank` | Counting routed calls only reads "matches" as "below" |
| Only routed boards keep their text | `only_the_boards_that_are_routed_keep_their_text` | Keeping every text holds 288 copies of the board |
| Rerun | `rerun_rebuilds_the_recorded_board` | A changed generator version or input SHA-256 is refused |
| Counts only | `the_table_names_no_net_and_no_part`: no reference or net name of the fixture appears in `table.md`; `the_table_prints_a_code_for_a_stop_never_the_reason`: a failure's path and tool output stay in `search.json` | — |
| Bench environment | `bench_environment_refuses_real_settings_and_foreign_inputs` (`place_bench_tests.rs`, not ignored): the real KiCad and Konnect folders (`Library/Application Support/konnect` too), an unknown home folder, an input outside the bench folder, an input reached through a link, a board folder, `candidates` or `isolation` that is a link | A scratch setup passes |
| End to end | `place_bench` (ignored): the bench itself | — |

Scoped `cargo llvm-cov` on the new modules, and targeted `cargo mutants` on the guards
(`held_set`, `check_held`, `same_board`, `admit_ai`, `legalize`, `pair_violations`,
`placement_gates`, `rank`, `verdict`, `routing_order`, `may_route`, the stop rule and
`bench_environment`), in an isolated checkout, as `AGENTS.md` says.

## Measurement: the bench

**Boards.** First a public fixture (`pic_programmer`, KiCad's demo) as the smoke run: the whole
pipeline at effort Fast with 2 routed per family, and no owner data. Then B4, B5 and B6, the same
three boards as the routing head-to-head (router research, stage 7).

**Preparation, once per board, in `target/creation-research/placement-search/bench/<board>/`:**

1. Copy the original into `orig/` and make it read-only; never open the original again.
2. `input/`: the project copy, routing stripped (`strip_routing`). Use stage 7's design-intent
   record, whose currents were worked back from the routed widths, and the board quirks KiCad
   accepts (Row E1, now in the product).
3. `rules.json`: each net's effective rules from headless KiCad Python, with a scratch
   `KICAD_CONFIG_HOME` (D19).
4. The owner locks, in KiCad and on a copy of each board, the parts he wants fixed (owner
   decision 4); the lock copy becomes `input/`'s board once its routing is stripped.
   `held.json` is then an optional extra list (a JSON array, `[]` when there is none).
5. S0 (`neutral_start`), then A by one AI session (owner decision 2), saved as `ai/board.kicad_pcb`
   with its session time.

**The run.** Per board: 160 recipes (F and S; 288 with `"perturb"` named) and 14 routing calls
at effort M (O and A 3 times each, and 4 per family; 18 with P), with a 100-minute limit, one board at a time, each holding the lane locks (native,
then instr).
The bench names its own fastroute (the candidate x18) and runs with scratch `KICAD_CONFIG_HOME` and
`KONNECT_STATE_DIR`. Recorded per board: `search.json` (every candidate) and `table.md`.

**The answer, per board:** the ranks of O's and A's calls and of each family's best; whether that
candidate passes every gate; and each family's verdict against O and against A: beats (above all
3 of the reference's calls), matches (among them) or below (D18). Across the three boards: each
family's best and median rank, how many of its candidates pass every gate, and whether its drawn
candidate ever beat its screened ones (the screen's check).

**Expected cost:** about 5.5 hours of machine time (3 x 100 minutes plus the smoke run), and three
AI sessions for A. If the routing calls run long, that is reported as information.

## What this design does not do

- Apply a candidate, or offer one through a tool (owner decision 1, reversed on 2026-11-19 for
  the later tool stage: measure first, then the tool).
- Flip parts; turn anything but a 2-pad part; place silk or text.
- A second round: editing the winners and routing them again. That comes after the bench, if the
  bench says the families are worth it.
- Tune weights, seeds or thresholds after seeing a result: the matrix is fixed before the first
  run.
- Measure the checklist's placement items that have no measure yet: switching parts away from
  analog, room to mate, probe and rework, thermal spacing (checklist 2), return paths (3).
- Remove routing noise: a reference gets 3 calls, so a verdict must clear its spread, but each
  generated candidate gets one, and two close candidates can swap places on a rerun.
- Windows and Linux; a live KiCad.
