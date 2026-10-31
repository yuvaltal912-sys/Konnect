# Simulation netlist fixtures

Real `kicad-cli sch export netlist` output from KiCad 10.0.1 on macOS, in both
`--format spice` (`.cir`) and `--format kicadsexpr` (`.net`). Only absolute
scratch paths and the export date were normalised.

| Fixture | Source project at KiCad tag 10.0.1 | Edit before export |
|---|---|---|
| `rectifier_unmodelled` | `demos/simulation/rectifier` | Instance `Sim.*` fields removed, keeping the library symbols' |
| `net_collision` | `demos/simulation/rectifier` | Labels renamed to `A(1)` and `A[1]`, so two nets share a SPICE name |
| `subsheets` | `demos/simulation/subsheets` | None |

`raw/` holds real results from the libngspice 44.2 in KiCad 10.0.1 on macOS arm64.
Each `.sp` deck ran once. `set filetype=ascii` and `set filetype=binary` then wrote
that run as `<name>.ascii.raw` and `<name>.binary.raw`, and `<name>.expected.json`
holds every vector read through `ngGet_Vec_Info`, a route that does not use the raw
file. `halted` ran with `stop after 12` first. Binary files are little-endian.

`soft_start.sp` is a hand-written deck, not a KiCad export: a 12 V rail where a
MOSFET ramps 1000 uF up, then a 5 A load steps on at 60 ms. The deck behind E31 was
not kept, so this one replaces it, tuned to the same scale: E31 read 16.56 A, 0.179
A²s and 96 mV, this reads 16.90 A, 0.2447 A²s and 95.5 mV.
`soft_start_reference.py` works those numbers out with no SPICE engine, by RK4 at
0.1 µs on the same equations, and the live test compares `run_simulation` with what
it prints. With `max_step_seconds` at 0.5 µs the two agree to every digit it prints.

`protection/` holds the five protection stand-ins the `kicad-simulation` skill shows, as
`.sp` decks, and `spliced.sp`, the eFuse joined to the rectifier demo's `/rect_out`.
They are hand-written, not KiCad exports. The E56 decks were not kept, so these replace
them. `protection/reference.py` works the expected values out with no SPICE engine:
closed forms, bisection for the nonlinear loads, and RK4 at 1 ns for the fuse's
integral. The live tests compare `run_simulation` with what it prints, and a test fails
if a line of a deck is missing from the skill.

`ac_loop.sp` is a hand-written testbench: a loop gain of three real poles, at 100 Hz,
1 kHz and 10 kHz, with a DC gain of 100. The deck behind E29 was not kept, so this
replaces it. `ac_loop_reference.py` solves its formulas by bisection with no SPICE
engine, and the live test compares `run_simulation` with what it prints: a crossover
at 3013.7 Hz, a phase margin of 3.486 degrees and a gain margin of 1.742 dB.

KiCad-owned formats: do not hand-edit these. Regenerate them with the commands
above if a KiCad version changes the output.
