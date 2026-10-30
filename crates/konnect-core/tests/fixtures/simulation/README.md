# Simulation netlist fixtures

Real `kicad-cli sch export netlist` output from KiCad 10.0.1 on macOS, in both
`--format spice` (`.cir`) and `--format kicadsexpr` (`.net`). Only absolute
scratch paths and the export date were normalised.

| Fixture | Source project at KiCad tag 10.0.1 | Edit before export |
|---|---|---|
| `rectifier_unmodelled` | `demos/simulation/rectifier` | Instance `Sim.*` fields removed, keeping the library symbols' |
| `net_collision` | `demos/simulation/rectifier` | Labels renamed to `A(1)` and `A[1]`, so two nets share a SPICE name |
| `subsheets` | `demos/simulation/subsheets` | None |

KiCad-owned formats: do not hand-edit these. Regenerate them with the commands
above if a KiCad version changes the output.
