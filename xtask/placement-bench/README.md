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
| `input/` | the project copy with its routing stripped; the search's input |
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
   kept). Use stage 7's design-intent record in `input/.konnect/`.
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
   references to hold. Nothing else is held (see `hold` below), so the list is his locks.
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
| `time_limit_minutes` | the whole board's limit, generation included | 100 |
| `effort` | `fast`, `m`, `h` or `xh` | `m` |
| `hold` | which parts the search holds: `locks` or `rules` (below) | `locks` |
| `families` | the generator families, any of `force`, `schematic`, `perturb` | `["force", "schematic"]` |

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
