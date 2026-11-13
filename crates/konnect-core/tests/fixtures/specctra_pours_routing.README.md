# Specctra pour-routing fixture

`specctra_pours_routing.kicad_pcb` (with `specctra_pours_routing.kicad_pro`) is
a synthetic board for routing with pours as planes and stitching after refill.
It needs no private board.

- 2 copper layers, a 50 x 40 mm rectangular outline.
- Twelve SMD 0805 resistors (`R1` to `R12`) in a 4 x 3 grid, alternately at
  0 and 90 degrees.
- Pad 2 of every resistor is `GND`. Pad 1 of `Rk` and `R(13-k)` share signal
  `Sk` (k = 1 to 6), so every signal crosses the board.
- A `GND` pour on each side: `GND_TOP` (F.Cu) and `GND_BOTTOM` (B.Cu), each 1 mm
  inside the outline. The router's signals cut both.
- The project sets one `Default` class: 0.25 mm track, 0.2 mm clearance, and a
  0.6 / 0.3 mm via.

The pours have outlines and no saved fill. Refill with
`kicad-cli pcb drc --refill-zones --save-board` on a scratch copy before
planning on it.

## Provenance

Written on 2026-11-13 by `specctra_pours_routing.make.py`, beside this file:
plain Python with no KiCad, run as
`/opt/homebrew/bin/python3.14 -I specctra_pours_routing.make.py <out-dir>`.
UUIDs are uuid5 values, so the script writes the same bytes every time.
`specctra_pours_tests.rs` uses the fixture. So did row S6's second-half
measurement natively, in cells F1omit and F1plane.
