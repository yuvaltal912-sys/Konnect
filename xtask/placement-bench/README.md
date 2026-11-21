# Placement bench: folder layout and preparation

The bench (`place_bench` in `crates/konnect-core/src/place_bench_tests.rs`, a later stage)
runs the best-of-N placement search on copies of boards (an ignored test). Design:
`docs/PLACEMENT_SEARCH_DESIGN.md`. Nothing here touches an original board, a live KiCad or
the installed Konnect. Documents get counts only: the bench folder is git-ignored.

## Folder layout

One folder per board under `target/creation-research/placement-search/bench/<board>/`:

| Path | What |
|---|---|
| `orig/` | the original project copy, read-only; never opened again |
| `input/` | the project copy with its routing stripped; the search's input. It holds the project's `.kicad_sch` files too (the root named like the board, `board.kicad_sch` for `board.kicad_pcb`, and every sheet file it names), which the schematic-neighbour measure reads (design D23) |
| `rules.json` | each net's effective routing rules (`net_rules.py`) |
| `held.json` | extra references to hold, besides the owner's KiCad locks (a JSON array, `[]` for none) |
| `ai/board.kicad_pcb` | A, the AI's placement, with its session time |
| `candidates/`, `search.json`, `table.md` | written by the run |

Run with scratch `KICAD_CONFIG_HOME` and `KONNECT_STATE_DIR` (never the real ones), and
hold the lane locks (native, then instr) while a board runs.

## Preparation, once per board

1. Copy the original project into `orig/` and make it read-only (`chmod -R a-w`). Work on
   copies only.
2. Make `input/` from `orig/` and strip its routing with the bench's strip,
   `place_route::strip_routing` (tracks, arcs, vias and teardrop zones; every other byte
   kept). Copy the project's `.kicad_sch` files into `input/` unchanged, in the same relative layout, with the root schematic named like the board. Without the root, `search.json` records the measure as `null` with its reason, and the search runs as before. Use stage 7's design-intent record in `input/.konnect/`.
3. Where the held set refuses a shared reference (parts with pads that share one), give
   those parts unique references in the `input/` copy (for example `C7` shared by four
   parts becomes `C7_1` to `C7_4`). Pad-less footprints (logos) may share a reference and
   need nothing: each is held as `<reference>#<n>`.
4. Make `rules.json` with KiCad's bundled Python and a scratch `KICAD_CONFIG_HOME`:
   `net_rules.py input/<board>.kicad_pcb rules.json`. It prints counts only.
5. The owner's locking step (his flow, 2026-11-19; design, owner decision 4). He locks in
   KiCad, on a copy, only the parts he wants fixed (outline drawn, parts imported and dumped
   anywhere): `target/creation-research/placement-search/lock-copies/<board>/board.kicad_pcb`.
   Make `orig/` and `input/` from that copy. Write `held.json` as `[]`, unless he names more
   references to hold. The held set is then his locks plus the parts with no pad on a net
   (see `hold` below); nothing else is held.
6. Make S0 (`neutral_start`), then A: one AI session on a copy of S0, isolated as design D20
   says, saved as `ai/board.kicad_pcb`.

The original boards are private: no net, part or board name goes into a document.

## Running the bench

`bench.json` sits in the bench folder (`target/creation-research/placement-search/bench/`):

```json
{ "boards": [
  { "name": "B4", "folder": "B4" },
  { "name": "smoke", "folder": "smoke", "board": "pic.kicad_pcb",
    "time_limit_minutes": 20, "effort": "fast",
    "hold": "rules", "families": ["force", "schematic", "perturb"] }
] }
```

| Key | Meaning | Default |
|---|---|---|
| `name` | the board's bench name; titles its table, must be unique | required |
| `folder` | its folder below the bench folder (a plain relative path) | required |
| `board` | the board file in `<folder>/input/` | the only `.kicad_pcb` there |
| `time_limit_minutes` | the whole board's limit, generation included, except family C's phase 1b, whose time does not count against it (design C13) | 100 |
| `effort` | `fast`, `m`, `h` or `xh` | `m` |
| `hold` | which parts the search holds: `locks` or `rules` (below) | `locks` |
| `families` | the generator families, any of `force`, `schematic`, `perturb`, `converge` | `["force", "schematic"]` |
| `converge_terms` | family C's two new cost terms (schematic neighbours, silk labels): `on` or `off` | `on` |

`hold`:
- `locks`: held are the footprints KiCad locks, `held.json`'s references, and the footprints
  with no pad on any net (logos and other artwork, including the `<reference>#<n>` keys of a
  shared reference), which play no part in routing. A connector, a mounting hole or a fiducial
  that the owner did not lock is not held: it moves like any part.
- `rules`: as before, the above plus every connector (prefix `J`), every mechanical part
  (prefix `H`, `MH` or `FID`, or a footprint ID naming a mounting hole or fiducial). The public
  smoke board uses it (its connectors are named `P1` and `P3`, listed in its `held.json`).

`families` keeps only the named families of the 288-recipe matrix: F and S give 160 recipes, and
naming `perturb` brings the 128 P recipes back. P edits the owner's own placement, which has no
meaning in his flow, so it is off by default. O and A are routed as references whatever is
named. An unknown `hold` value, an unknown family or an empty `families` list is refused before
anything runs. The hold rules and the matrix are recorded in `search.json`'s plan.

`converge` adds family C, the convergent search (`docs/CONVERGENT_PLACEMENT_DESIGN.md`). It
refines F's and S's candidates, so it needs `force` or `schematic` named with it and is refused
alone. It runs after every F, S and P candidate exists (phase 1b), at the design's measured
parameters and outer rules; its wall time does not count against `time_limit_minutes`.
`converge_terms: "off"` sets the neighbour weight and the label cost to 0 for that board (the
design's measurement step 3); any other value than `on` or `off` is refused. With C,
`search.json` also holds C's parameters and rule (`plan.matrix.converge`), each C record's
trace (`converge`: the cost term by term at the start and the end, both stop reasons, stages,
sweeps, evaluations, the best cost after every stage and sweep), and the outer trace (top-level
`converge`: the starts taken and the best cost after each, the cost by effort level, why each
rule stopped, seconds). `table.md` gets C's row and verdict, and one line of counts on the
outer loops.

Per board the folder holds `input/`, `rules.json`, `held.json` (a JSON array of the
references to hold besides the mode's own: `["J1", "H3"]`, or `[]`) and optionally `ai/board.kicad_pcb`.
A folder that already holds a `search.json` or `candidates/` is refused: move them away
first.

```text
KONNECT_PLACE_BENCH=<bench folder> KONNECT_FASTROUTE=<candidate fastroute> \
KICAD_CONFIG_HOME=<bench folder>/scratch/kicad-config \
KONNECT_STATE_DIR=<bench folder>/scratch/state \
cargo test -p konnect-core --release place_bench -- --ignored --nocapture
```

Before anything runs the bench refuses: an unset or relative `KONNECT_PLACE_BENCH`;
`KICAD_CONFIG_HOME` or `KONNECT_STATE_DIR` unset, at or around KiCad's or Konnect's real
folder, or not below the bench folder or the system temp folder; a `KONNECT_FASTROUTE`
that is not an executable file; an input or AI board that resolves (links followed)
outside the bench folder; no kicad-cli (`KONNECT_KICAD_CLI`, else the usual search). It
writes `candidates/`, `search.json`, `table.md` and an empty `isolation/` below each
board's folder, and prints one line per board.

The bench does not take the lane locks (native, then instr): the driver that starts it
holds them for each board's run.

## Family C without routing

`converge_check`, an ignored test beside `place_bench`, runs family C on every board that names
`converge`, with no routing call (the design's measurement step 1). It takes the same
environment and guards as `place_bench`, starts no router, starts no KiCad unless
`KONNECT_CONVERGE_SILK=1` is set (below), and writes only `<board folder>/converge-check.json`
(refused when the file exists): phase 1's counts, the outer trace, and per C run its recipe
(start family, seed, parameter-set index, level), outcome, cost, both stop reasons, stages,
sweeps, evaluations, the start and end terms, and seconds. `KONNECT_CONVERGE_PORT_CHECK=1` adds
the port's check: 5 runs at level 0, seeds 1 to 5, both new terms off, from F seed 24 with rail
weight 0, and their median cost. Counts and numbers only. It repeats phase 1 itself and reads
no AI board, so an F or S candidate whose board equals A's is a start here and a `duplicate` in
the bench: its pool can then differ from the bench's. Its run ids start after phase 1's records
with no O or A record before them, so they are not the bench's ids: match runs by their recipe.

`KONNECT_CONVERGE_SILK=1` adds design A3's two measures, with no routing call, by the bench's
own code, and **starts kicad-cli** (a refill and DRC per board, one at a time, with the scratch
`KICAD_CONFIG_HOME`; a real `kicad-cli` is needed, so `KONNECT_KICAD_CLI` must not name a
stand-in). It runs the bench's input check on the input board (`PlaceRouteCall::unrouted`, that
is `place_route::unrouted_check`), writes each C run's board as the bench writes a candidate
before its routing call (`export_candidate`), runs the same check on that copy, and adds:

- once per board, `input_silk`: the input's `silk_overlap`, `silk_over_copper`,
  `silk_edge_clearance` and their `sum` (the bench's `input_unrouted`), and `input_silk_reason`;
- per C run, `silk` (the same four numbers of the run's board; `sum` is the table's `silk`
  column) and `silk_reason`, and `schematic_neighbours` (as `search.json` records it: `mean_mm`,
  `ratio_to_input`, `parts_pulled_away`, `parts_measured`, `parts_unmatched`, `parts_alone`) and
  `schematic_neighbours_reason`.

A reason is `null` beside a value, and beside a `null` one fixed phrase (`no board`, `the export
failed`, `the unrouted check failed`, `the input's schematic gives no neighbourhoods`, `the board
could not be measured`); a failure's own text goes to the log only. The scratch boards stay in
`<board folder>/converge-check-silk/` (`isolation/`, `input-check/`, `candidates/c<id>/`, the
bench's layout) for inspection, and a run is refused while that folder exists. Without the
variable the file is what it was before the variable existed.

```text
KONNECT_PLACE_BENCH=<bench folder> KONNECT_FASTROUTE=<any executable fastroute> \
KICAD_CONFIG_HOME=<bench folder>/scratch/kicad-config \
KONNECT_STATE_DIR=<bench folder>/scratch/state [KONNECT_CONVERGE_PORT_CHECK=1] \
[KONNECT_CONVERGE_SILK=1] \
cargo test -p konnect-core --release --lib place_bench_tests::converge_check -- --exact --ignored --nocapture
```
