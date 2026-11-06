# IBIS input provenance fixture

`ibis.net` was exported by the installed KiCad 10.0.1 `kicad-cli sch export netlist --format kicadsexpr` from a scratch copy of the KiCad 10.0.1 `demos/simulation/ibis` project on 2026-11-04. The scratch-directory prefix in source metadata was removed; component fields are unchanged. `ibis_v1_1.ibs` and `ibis.kicad_sch` are unchanged copies of the demo's original model and schematic.

The exported fields bind `U1` through `U6` to `Sim.Device=IBIS` and `Sim.Library=ibis_v1_1.ibs`. KiCad's SPICE export generated six `KiCad/10.0/ibis/<reference>.cache` files. A subsequent SPICE export of a scratch rectifier demo, with the same isolated `KICAD_CACHE_HOME`, deleted all six. Both exports exited zero. Tests use this independent KiCad output to prove that stored input provenance follows the original IBIS file rather than the disposable generated includes.

These are upstream reference-demo fixtures, not a user's design or measured circuit.
