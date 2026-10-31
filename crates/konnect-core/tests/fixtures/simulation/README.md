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

KiCad-owned formats: do not hand-edit these. Regenerate them with the commands
above if a KiCad version changes the output.
