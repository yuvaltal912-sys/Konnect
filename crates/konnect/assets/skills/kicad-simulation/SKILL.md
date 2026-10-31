---
name: kicad-simulation
description: |
  SPICE simulation workflow for KiCAD schematics via MCP tools. Triggers on: "simulate", "SPICE",
  "ngspice", "operating point", "bias point", "what voltage is on this net", "does this circuit work",
  "check the DC levels", "run a simulation", "transient", "inrush", "load step", "droop", "overshoot",
  "settling time", "rise time", "soft start".
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
4. Run `run_simulation` with one analysis:
   - `{ "kind": "operating_point" }` for DC levels.
   - `{ "kind": "transient", "stop_time_seconds": 0.14 }` for inrush, load steps and anything over time.
5. Report the results, and state `simulated_circuit` if it is `modified`.

## Transient Runs

Add `signals` for summaries and `measurements` for numbers. With neither you get no values back, only the proof that the run covered the range. Caller text never reaches the deck.

```json
{
  "schematic": "power.kicad_sch",
  "analysis": { "kind": "transient", "stop_time_seconds": 0.14, "max_step_seconds": 1e-6 },
  "signals": ["v(/VOUT)"],
  "envelope_points": 16,
  "measurements": [
    { "name": "inrush_peak", "kind": "peak_abs", "signal": "i(BT1)" },
    { "name": "inrush_i2t", "kind": "i2t", "signal": "i(BT1)", "to_seconds": 0.06 },
    { "name": "rail_low", "kind": "min", "signal": "v(/VOUT)", "from_seconds": 0.06 }
  ]
}
```

| Signal | Meaning |
|---|---|
| `v(NET)`, `v(NET1,NET2)` | A net by the name KiCad shows, or the difference of two |
| `i(REF)` | The current of a two-pin part or source, by reference |
| `i(REF:PIN)` | A pin of a larger part, such as `i(M1:d)`. A wrong guess lists the real `pins` |
| `p(REF)` | Power in a part, counting the pins it shows. A subcircuit with a hidden ground or supply reads short. Code models and `K` couplings report none |

A testbench element has no reference, so use the name it has in `spice_lines`.

Set `max_step_seconds` to about a hundredth of the fastest edge you care about. The default step is `stop_time_seconds / 2000`: on a 0.14 s run that is 70 µs, which got a 750 µs inrush within 1% but put its peak 0.6 ms early. A smaller step costs points, so narrow the run first.

| Measurement | Needs | Returns |
|---|---|---|
| `min`, `max`, `peak_abs` | | The value and `at_seconds`. `peak_abs` keeps its sign |
| `peak_to_peak`, `average`, `rms`, `integral` | | Time-weighted over the window |
| `i2t` | An `i(...)` signal | A²s, for fuses |
| `value_at` | `at_seconds` | The value at that time |
| `crossing` | `level`, optional `direction` | The first time the signal reaches the level |
| `rise_time`, `fall_time` | | 10 to 90 percent |
| `overshoot_percent` | | How far the signal goes past the final value, in the step's direction, as a share of the step |
| `settling_time` | Optional `band_percent`, default 2 | Time from the window start until it stays in the band. The band is a share of the final value or the step, whichever is larger |

All but `value_at` take `from_seconds` and `to_seconds`. The last four take their step from the window: its first value, and the mean of its last tenth as the final value. So give a window that holds just one edge and let it run on until the signal has stopped. They return `start_value` and `final_value`: check them, because a window ending just past a crest gives a wrong number and no test catches every case. A signal still moving at the end, or an edge in the last tenth of the window, gives `final_value_needed`: narrow or extend the window, or give `final_value`. So does a rise, fall or overshoot whose signal strays more than three steps from its start, such as an inrush current, where `peak_abs` is the measurement you want. A sag that stays down reads as overshoot: use `min` for a droop.

A `value` of `null` with a `finding` is an answer, not a failure.

| Finding | Meaning |
|---|---|
| `level_not_reached` | The signal never got there |
| `starts_on_level` | It was already on the level when the window began, and never crossed it |
| `no_transition` | The signal is flat to within rounding, or the step runs the other way |
| `final_value_needed` | The window's last value cannot stand in for the final value |
| `not_settled` | It is outside the band in the last tenth of the window |
| `outside_data` | The window or time holds no data |
| `non_finite_data` | The data, or the result, is not a finite number |

**Initial conditions.** With `use_initial_conditions`, ngspice saves no point at time zero, so `analysis.first_point_seconds` is above 0 and `no_point_at_start` warns that a peak at that instant reads low and an integral misses the first step. Device probes would also lose a voltage set on a net by `.ic` or `.nodeset`, so with either card only node voltages and source currents are saved, `i(R1)` and `p(R1)` read `signal_not_found`, as does `i()` of a current source, and a `.probe` card of your own is refused as `probe_loses_initial_conditions`. To keep device currents, use `IC=` on the device, which probes do not disturb.

## Fixing an Unmodelled Symbol

| `suggested_fix` | What to do |
|---|---|
| `copy_library_sim_fields` | Copy the `Sim.*` fields from the library symbol with `edit_schematic_component` |
| `exclude` | The part has no series role. Add it to `testbench.exclude_references` |
| `replace_series_part` | Removing it would break the path. Add a stand-in through `testbench.spice_lines` |

Nothing in `testbench` touches the schematic. Use the `spice_name` values from readiness in `spice_lines`.

## Reading a Result

- **`outcome.status`:** `complete` means the requested range was covered and every measurement found its signal. `partial` means the data is real but stops early or a signal was missing, and `stopped.reason` says why it stopped: `timeout`, `size_limit`, `no_convergence` or `incomplete_coverage`.
- **`analysis.coverage_ratio`:** how much of the requested time ran. On a partial run every measurement and summary carries `window_truncated: true`, because it describes only the part that ran. Say so, and never report such a peak as the run's peak. On a complete run the same flag means that measurement's window reached past the saved data, such as before `start_save_seconds`, so its number leaves out that stretch.
- **`finding: "signal_not_found"`:** the run does not hold that signal. Only voltage sources keep their currents in circuits with digital parts, or with `.ic` or `.nodeset` under `use_initial_conditions`, and `device_currents_not_saved` says which. Digital nets have no `v()`.
- **`simulated_circuit`:** `modified` means parts were excluded or added. Never present such a result as describing the schematic as drawn.
- **`circuit.approximations`:** parts whose behaviour rests on an assumption. Say so when they matter.
- **`warnings`:** read them. `engine_messages` lists what ngspice ignored or changed, which can alter the circuit. `device_currents_not_saved` means only node voltages and voltage source currents were saved.
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
| `timed_out` | The budget ran out before any data | Raise `timeout_seconds`, up to the client's own limit |
| `size_limit` | The size limit left nothing readable | Raise `max_raw_bytes`, or shorten the run |
| `probe_loses_initial_conditions` | A `.probe` card meets `.ic` or `.nodeset` under `use_initial_conditions` | Remove the card, or use `IC=` on the device |
| `engine_failed` | The simulator stopped on a fatal error | Read `error.items` and the log |
| `engine_crashed` | The simulator process died | The server is unaffected. Read the log |
| `unbalanced_deck_block` | A `.if` or `.subckt` is left open | Close it. Check `spice_lines` first |
| `engine_not_found` | No libngspice was found | Set `ngspice_library`, using the listed locations |

A run that stops early with data is not an error. For `stopped.reason: "size_limit"`, raise `max_raw_bytes`, shorten the run or raise `max_step_seconds`. For `timeout`, do the same with `timeout_seconds`. For `no_convergence`, `stopped.node` names where it failed.

MCP clients often stop waiting at 60 seconds, which is why the default budget is 40.
