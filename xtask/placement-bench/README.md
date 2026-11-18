# Placement bench: folder layout and preparation

The bench (`place_bench` in `crates/konnect-core/src/place_bench_tests.rs`, a later stage)
runs the best-of-N placement search on copies of boards. Design:
`docs/PLACEMENT_SEARCH_DESIGN.md`. Nothing here touches an original board, a live KiCad or
the installed Konnect. Documents get counts only: the bench folder is git-ignored.

## Folder layout

One folder per board under `target/creation-research/placement-search/bench/<board>/`:

| Path | What |
|---|---|
| `orig/` | the original project copy, read-only; never opened again |
| `input/` | the project copy with its routing stripped; the search's input |
| `rules.json` | each net's effective routing rules (`net_rules.py`) |
| `held.json` | the held list the owner confirmed (design, owner decision 3) |
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
5. Write `held.json`: the rule's list (connectors, mounting holes, fiducials, KiCad locks);
   the owner confirms or edits it once, from the counts.
6. Make S0 (`neutral_start`), then A: one AI session on a copy of S0, isolated as design D20
   says, saved as `ai/board.kicad_pcb`.

The original boards are private: no net, part or board name goes into a document.
