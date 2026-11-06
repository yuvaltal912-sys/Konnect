# board_geometry/cases.kicad_pcb

Hand-written KiCad 10 board (format 20260206) for the board-geometry layer. `kicad-cli pcb drc`
loads it and reports only `copper_edge_clearance`, `track_dangling` and `lib_footprint_issues` (the
parts are not library parts). No real project's data is in it.

**Outline.** 40 × 30 mm, corners rounded with 5 mm radius `gr_arc`s (the way `add_board_outline`
draws them), plus a `gr_circle` cut-out of radius 3 mm centred on (20, 15). The arc centres are
(5, 5), (35, 5), (35, 25), (5, 25).

| Part | At | What it is | Hand-computed expectation |
|---|---|---|---|
| U1 | (12, 8) | inside | no finding |
| Q1 | (30, 22), 90° | inside; pad 1 local (-1.5, 0) lands at (30, 23.5) | pad box x 29.2..30.8, y 23.1..23.9 |
| J1 | (40, 15) | courtyard x 37..43 straddles the right edge; pad 2 spans x 39.5..41.5 | courtyard 3.0 mm over, pad 1.5 mm over |
| R9 | (60, 15) | wholly outside; courtyard x 59..61 | 21.0 mm (far corner to the edge at x = 40) |
| C1 | (3, 3) | in a rounded corner; courtyard 1..5 square; pad 3 spans 0.5..1.5 | courtyard corner (1, 1) is 5.657 − 5 = 0.657 mm outside the arc; pad corner (0.5, 0.5) is 6.364 − 5 = 1.364 mm outside |
| C2 | (22.5, 15) | courtyard x 20.5..24.5, y 13..17 over the cut-out; pad 1 inside it, pad 2 at x 23.5 | courtyard left edge midpoint (20.5, 15) is 3 − 0.5 = 2.5 mm inside the cut-out |
| Q2 | (20, 18.5), 90° | courtyard over the cut-out's edge (y = 18); pad 1 local (-0.7, 0) lands at (20, 19.2), 0.8 mm below it | courtyard only; the pad is on the board (wrong rotation sign puts it across the edge) |
| TP1 | (20, 27) | body inside; reference text centred on the top edge (y = 30) | text only |

Routing for the region queries: VCC `F.Cu` (10, 8)–(10, 12), GND `F.Cu` (14, 8)–(14, 12), VCC `B.Cu`
(10, 12)–(14, 12), a VCC through via at (10, 12), a GND zone outline (33, 24)–(37, 27) on `F.Cu`,
and a silkscreen line (8, 12)–(16, 12).

KiCad's own DRC flags exactly three pads as `copper_edge_clearance` (clearance 0.5 mm): J1 pad 2
(actual 0.0), C1 pad 3 (actual 0.0, on the rounded corner) and C2 pad 2 (actual 0.2, from the
cut-out). Q2 pad 1 is not flagged, which pins the rotation sign. The ignored test
`board_geometry::tests::kicad_drc_agrees_with_the_outline_about_which_pads_are_at_the_edge` checks
that against the live KiCad.
