---
name: kicad-simulation
description: |
  SPICE simulation workflow for KiCAD schematics via MCP tools. Triggers on: "simulate", "SPICE",
  "ngspice", "operating point", "bias point", "what voltage is on this net", "does this circuit work",
  "check the DC levels", "run a simulation".
argument-hint: "[circuit or question]"
---

# KiCAD Simulation Workflow

Simulation runs on the saved schematic, on a private copy, in a separate process. It never writes to the project.

---

## Toolset Loading

```
load_toolset('simulation')    # check_simulation_readiness, run_simulation
```

## Workflow

1. Save the schematic. Unsaved editor changes are not seen.
2. Run `check_simulation_readiness`. `ready: true` means it can run. `ready: false` names what stops it. `ready: null` means a check could not run.
3. Fix what it names, using its `suggested_fix` on each unmodelled symbol.
4. Run `run_simulation` with `analysis: { "kind": "operating_point" }`.
5. Report the results, and state `simulated_circuit` if it is `modified`.

## Fixing an Unmodelled Symbol

| `suggested_fix` | What to do |
|---|---|
| `copy_library_sim_fields` | Copy the `Sim.*` fields from the library symbol with `edit_schematic_component` |
| `exclude` | The part has no series role. Add it to `testbench.exclude_references` |
| `replace_series_part` | Removing it would break the path. Add a stand-in through `testbench.spice_lines` |

Nothing in `testbench` touches the schematic. Use the `spice_name` values from readiness in `spice_lines`.

## Reading a Result

- **`outcome.status`:** `complete` means the requested range was covered. `partial` means the data is real but stops early, and `stopped.reason` says why.
- **`simulated_circuit`:** `modified` means parts were excluded or added. Never present such a result as describing the schematic as drawn.
- **`circuit.approximations`:** parts whose behaviour rests on an assumption. Say so when they matter.
- **`warnings`:** read them. `engine_messages` lists what ngspice ignored or changed, which can alter the circuit. `device_currents_not_saved` means a digital circuit reports only voltages and source currents.
- **`operating_point.rows`:** currents read positive into a device's first pin, so a delivering source reads negative.
- **Full data:** `artifacts.table_path` holds every row, and `artifacts.deck_path` holds the deck that ran.

## When It Fails

Branch on `error.reason`, and show `error.items`.

| Reason | Meaning | Next step |
|---|---|---|
| `unmodelled_symbols` | Parts have no simulation model | Fix as above |
| `no_ground_reference` | No element reaches node 0 | Add a ground to the schematic |
| `floating_subcircuit` | Nodes have no DC path to ground, so their voltages are arbitrary | Add the missing path, or a large resistor through `spice_lines` |
| `net_name_collision` | Two nets become one node in the engine | Rename one. Non-ASCII labels of equal length collide |
| `circuit_not_parsed` | ngspice rejected the deck | Read `error.items` and the log |
| `no_convergence` | The analysis did not settle | Check for voltage loops and floating gates |
| `timed_out` | The budget ran out | Raise `timeout_seconds`, up to the client's own limit |
| `engine_failed` | The simulator stopped on a fatal error | Read `error.items` and the log |
| `engine_crashed` | The simulator process died | The server is unaffected. Read the log |
| `unbalanced_deck_block` | A `.if` or `.subckt` is left open | Close it. Check `spice_lines` first |
| `engine_not_found` | No libngspice was found | Set `ngspice_library`, using the listed locations |

MCP clients often stop waiting at 60 seconds, which is why the default budget is 40.
