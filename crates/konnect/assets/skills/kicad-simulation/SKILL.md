---
name: kicad-simulation
description: |
  SPICE simulation workflow for KiCAD schematics via MCP tools. Triggers on: "simulate", "SPICE",
  "ngspice", "operating point", "bias point", "what voltage is on this net", "does this circuit work",
  "check the DC levels", "run a simulation", "transient", "inrush", "load step", "droop", "overshoot",
  "settling time", "rise time", "soft start", "short circuit", "fuse", "eFuse", "current limit",
  "undervoltage lockout", "UVLO", "reverse polarity", "reversed battery", "protection",
  "AC analysis", "frequency response", "bode", "bandwidth", "phase margin", "gain margin",
  "stability", "loop gain", "DC sweep", "transfer curve", "sweep".
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
   - `{ "kind": "ac", "start_hz": 10, "stop_hz": 1e6 }` for gain, bandwidth and stability margins.
   - `{ "kind": "dc_sweep", "sweeps": [{ "target": "V1", "start_value": 0, "stop_value": 12, "step_value": 0.1 }] }` for a transfer curve or a polarity sweep.
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
| `phase_unreliable` | AC only: the sweep is too coarse to follow the phase past `phase_unreliable_from_hz`. Raise `points` |
| `starts_on_level` | It was already on the level when the window began, and never crossed it |
| `no_transition` | The signal is flat to within rounding, or the step runs the other way |
| `final_value_needed` | The window's last value cannot stand in for the final value |
| `not_settled` | It is outside the band in the last tenth of the window |
| `outside_data` | The window or time holds no data |
| `non_finite_data` | The data, or the result, is not a finite number |

**Initial conditions.** With `use_initial_conditions`, ngspice saves no point at time zero, so `analysis.first_point_seconds` is above 0 and `no_point_at_start` warns that a peak at that instant reads low and an integral misses the first step. Device probes would also lose a voltage set on a net by `.ic` or `.nodeset`, so with either card only node voltages and source currents are saved, `i(R1)` and `p(R1)` read `signal_not_found`, as does `i()` of a current source, and a `.probe` card of your own is refused as `probe_loses_initial_conditions`. To keep device currents, use `IC=` on the device, which probes do not disturb.

## AC Sweeps

A small-signal sweep around the operating point. It needs a source with an `AC` value: a KiCad source with `ac=1` in its `Sim.Params` has one, and a testbench source can say `VIN in 0 DC 0 AC 1`. With none, every signal would read zero, so `run_simulation` refuses it as `no_ac_stimulus`.

```json
{
  "schematic": "filter.kicad_sch",
  "analysis": { "kind": "ac", "start_hz": 10, "stop_hz": 1e6, "scale": "decade", "points": 100 },
  "signals": ["v(/OUT)"],
  "measurements": [
    { "name": "edge", "kind": "bandwidth_3db", "signal": "v(/OUT)" },
    { "name": "gain_1k", "kind": "value_at", "signal": "v(/OUT)", "at_hz": 1000 }
  ]
}
```

`points` is per decade or octave, or in all on a `linear` scale (at least 3). Frequencies run from 1e-9 to 1e18. A margin follows the phase from sample to sample, and a sweep too coarse to follow a sharp resonance would read an unstable loop as stable, so it gives `phase_unreliable` instead: use 100 points a decade or more for margins. An octave sweep, and some decade ones, end on their last grid point at or below `stop_hz`, up to a step short of it: read `simulated_to_hz`, and `sweep_ends_before_stop` says so. Gain is 20 log10 of the magnitude, so it is the gain only if the AC source is 1. Say "dB re 1 V" otherwise. `p()` has no value here.

| Measurement | Returns |
|---|---|
| `value_at` with `at_hz` | Gain in dB, and `phase_degrees` unwrapped from the lowest frequency |
| `bandwidth_3db` | The first frequency 3.01 dB below the gain at the lowest frequency, shown as `reference_gain_db`. With `reference_level: "peak"` it measures from the peak and gives `lower_edge_hz`, `upper_edge_hz` and their difference as `value` |
| `phase_margin` | 180 degrees plus the phase where the gain first falls through 0 dB, with `crossover_hz` |
| `gain_margin` | The negative of the gain in dB where the phase first falls through -180 degrees, with `phase_crossover_hz` |

**Stability margins need a loop gain, not a closed-loop response.** Break the loop at one point, drive it with an AC source, and take the loop gain between two nodes with `reference_signal`: `{ "kind": "phase_margin", "signal": "v(/RETURN)", "reference_signal": "v(/INJECT)" }`. The signal must be non-inverted: the result shows `start_phase_degrees`, and a loop that starts near 0 or at -90 is plausible while one that starts at 180 has the wrong sign. A margin is a first crossing: if the gain crosses 0 dB more than once, `from_hz` and `to_hz` pick which points are examined. A `null` value with `level_not_reached` means the curve never crossed, which for a gain margin means the phase never reaches -180.

## DC Sweeps

Varies up to two things and solves the circuit at each step: a voltage source, a current source, a resistor, or `TEMP` in degrees C. Give a part's reference (`BT1` finds `VBT1`), a testbench element's own name, or `TEMP`. A name the circuit does not have is `sweep_target_not_found`.

```json
{
  "schematic": "power.kicad_sch",
  "analysis": { "kind": "dc_sweep", "sweeps": [
    { "target": "BT1", "start_value": -12, "stop_value": 12, "step_value": 0.05 }
  ] },
  "signals": ["v(/VOUT)"],
  "measurements": [
    { "name": "out_at_12", "kind": "value_at", "signal": "v(/VOUT)", "at_sweep": 12 },
    { "name": "turn_on", "kind": "crossing", "signal": "v(/VOUT)", "level": 5, "direction": "rising" }
  ]
}
```

A negative `step_value` sweeps down, and its sign must point from the start toward the stop. Measurements and windows follow the order of the sweep: `from_sweep` 12 and `to_sweep` 0 on a sweep that falls from 12. One sweep takes `min`, `max`, `peak_to_peak`, `peak_abs`, `value_at` (`at_sweep`) and `crossing`, and a place on the sweep comes back in the sweep's unit. An average over a sweep has no meaning, so it is refused. With two sweeps the first varies fastest, `signals` cover the whole grid (`at_start` and `at_stop` are its first and last point), and measurements are refused: the data is in `artifacts.raw_path`.

## Protection Scenarios

Parts that protect a rail rarely have a usable model, so paste a stand-in into `testbench.spice_lines`, one array item per line. A stand-in is an approximation with the behaviour its parameters give and nothing else. It is not a model of a specific part and has no thermal behaviour: no heating, no thermal shutdown, no drift with temperature. The result reads `simulated_circuit: "modified"`. Say both things when you report it.

Each block below ran through `run_simulation` and matched values worked out without SPICE. The first lines of a block (supply, load, and for the fuse its path resistor) are a bench harness that your schematic supplies instead. Swap the node names for yours, using the `spice_name` values from readiness, such as `/rect_out`. For a bench check, exclude everything with `exclude_references: ["*"]` and paste the block as it stands.

Rules for all five:

- **Keep transitions smooth, as written.** A stand-in that flips back and forth stops the run with `no_convergence`: a lockout whose hysteresis is narrower than the supply sag does. Set `max_step_seconds` as each block says.
- **Add no `.probe`, `.ic`, `.nodeset`, `K` coupling or `.options interp`.** Konnect refuses some of them and they change the circuit in the rest. The capacitor in the fuse uses `IC=` on the device, which is safe, and every block also runs with `use_initial_conditions`.
- **Rename what collides.** Nodes and elements carry the prefixes `sh_`, `fz_`, `ef_`, `uv_`, `rb_` and `RSHORT`, `VFZS` and so on. Change one that a part of your schematic already uses.
- **Use `**` for powers.** `exp()` in a behavioural expression turns linear above an argument of 14, so `exp(ln(1G)*x)` quietly reads 9.3 Mohm for 1 Gohm.

### Output short

A resistor across the output that falls from 1 G to 5 mohm at `sh_t0`. The fuse and eFuse blocks include it as their fault. `max_step_seconds` 1e-6 is plenty.

```
* Output short. Your circuit supplies the first three elements; the rest is the fault.
VBAT vbat 0 DC 12
RSRC vbat out 20m
RLOAD out 0 12
* Fault: a resistor across the output that falls from 1 G to 5 mohm around 5 ms.
.param sh_t0=5m sh_tau=5u sh_ron=5m sh_roff=1G
.func sh_s() {0.5*(1+tanh((time-sh_t0)/sh_tau))}
RSHORT out 0 R='sh_roff**(1-sh_s())*sh_ron**sh_s()'
```

This harness gives 480.04 A and 2.399 V at the output, as 12 V over 20 mohm plus 5 mohm in parallel with 12 ohm.

### I²t fuse

Opens for good once the integral of the current squared reaches `fz_rated`, in A²s. `v(fz_q)` is that integral. The fuse must sit in series, with `VFZS` in the path to read the current. The block includes the short, so it opens at 5 ms.

```
* I2t fuse into a short. Your circuit supplies the supply, RSRC and the load.
VBAT vbat 0 DC 12
RSRC vbat fz_in 20m
RLOAD out 0 12
* Fault: the output short.
.param sh_t0=5m sh_tau=5u sh_ron=5m sh_roff=1G
.func sh_s() {0.5*(1+tanh((time-sh_t0)/sh_tau))}
RSHORT out 0 R='sh_roff**(1-sh_s())*sh_ron**sh_s()'
* Stand-in: a latching switch that opens once the integral of i^2 passes the rating.
.param fz_rated=2
VFZS fz_in fz_a 0
BFZQ 0 fz_q I='time>0 ? 1u*i(VFZS)*i(VFZS) : 0'
CFZQ fz_q 0 1u IC=0
RFZQ fz_q 0 1e12
BFZC fz_ctl 0 V='fz_rated-v(fz_q)'
SFZ fz_a out fz_ctl 0 SWFZ
.model SWFZ SW(Vt=0 Vh={fz_rated/1000} Ron=5m Roff=1Meg)
```

The step cannot land on the rating, so the integral overshoots the trip by up to the prospective current squared times `max_step_seconds`: 0.016 A²s at 400 A and 0.1 µs, and 0.004 A²s was seen. For a 1 percent error, use `max_step_seconds` of `0.01 * fz_rated / I²`, and narrow the run to the moments around the fault. The fuse trips 0.1 percent above `fz_rated`, switching from 5 mohm to 1 Mohm (edit them in `.model SWFZ`), and never closes again or cools: a long overload below the rating still builds the integral. It opens in one step and has no arc. With inductance ahead of it the opening makes a voltage spike and the current rings afterwards: 1 µH gave 21.9 kV and -5 mA at 9.9 ms instead of 12 µA, and the run still read `complete`. Do not trust voltages across an opening fuse in an inductive circuit.

### eFuse current limit

Behaves as `ef_ron` below `ef_ilim` and holds `ef_ilim` above it, for as long as the fault lasts. There is no timer, no foldback and no thermal shutdown, so it never trips. Its resistance rises as the current nears the limit: from the formula, 1.1 times `ef_ron` at half the limit and 1.9 times at 95 percent. It also conducts backwards, so pair it with the reversed battery block.

```
* eFuse current limit into a short. Your circuit supplies the supply and the load.
VBAT vbat 0 DC 12
RLOAD out 0 12
* Fault: the output short.
.param sh_t0=5m sh_tau=5u sh_ron=5m sh_roff=1G
.func sh_s() {0.5*(1+tanh((time-sh_t0)/sh_tau))}
RSHORT out 0 R='sh_roff**(1-sh_s())*sh_ron**sh_s()'
* Stand-in: a pass element that is ef_ron below the limit and holds ef_ilim above it.
.param ef_ilim=5 ef_ron=20m
BEFZ vbat out I='ef_ilim*tanh(v(vbat,out)/(ef_ilim*ef_ron))'
```

### Undervoltage lockout

A switch that closes when its own input rises above `uv_on` and opens when it falls below `uv_off`. Keep `uv_on` above `uv_off`: swapped values run with no warning. Feed it a slow ramp, because the switch changes in one step. The thresholds are right to within the ramp rate times `max_step_seconds`: 10 mV for 1 V per ms and 10 µs. The hysteresis must be wider than the sag when the load connects: with a 3 ohm source ahead of it, the 12 V load pulled the input from 10 V to 8 V, the stand-in chattered and the run stopped in `no_convergence`.

```
* Undervoltage lockout on a slow ramp. Your circuit supplies the input and the load.
VIN vin 0 PWL(0 0 14m 14 28m 0)
RLOAD out 0 12
* Stand-in: a switch that closes above uv_on and opens below uv_off, on its own input.
.param uv_on=10 uv_off=9
SUV vin out vin 0 SWUV
.model SWUV SW(Vt={(uv_on+uv_off)/2} Vh={(uv_on-uv_off)/2} Ron=10m Roff=1G)
```

### Reversed battery

The stand-in conducts forward through `rb_ron` and leaks backward through `rb_roff`. Sweep it upward with `dc_sweep` on `VBAT` from -12 to 12 V in 0.05 V steps: the sweep replaces the `PWL` value, so the block runs as it stands. The ramp in the block gives the same curve as a transient on 24 ms with `max_step_seconds` 5e-7, within 0.2 percent at ten places from -12 to 12 V (a part in 10^9 from 6 V up), and a ramp is the way to go with a capacitor on the output (ramp slower than its time constant). A coarser ramp step reads the knee badly: 10 µs read 25 percent off at -50 mV. `i(VBAT)` reads positive when leakage flows back into the battery.

```
* Reversed battery on a slow ramp from -12 V to 12 V. Your circuit supplies the battery and the load.
VBAT vbat 0 PWL(0 -12 24m 12)
RLOAD out 0 12
* Stand-in: a pass element that conducts forward at rb_ron and leaks backward through rb_roff.
.param rb_ron=30m rb_roff=12Meg rb_knee=10m
BRB vbat out I='v(vbat,out)*(1/rb_roff+(1/rb_ron-1/rb_roff)*0.5*(1+tanh(v(vbat,out)/rb_knee)))'
```

At -12 V this reads 1.0 µA of leakage and -12 µV at the output. **Sweep it upward.** Between about -0.56 V and 0 the stand-in has two solutions, so a sweep that runs downward through zero latches into conduction (-0.55 V out, 45 mA) where an upward one blocks, and the lowest output of an upward sweep depends on its step (-1.1 mV at 0.05 V, -40 mV at 0.01 V) because the knee leaks most within 50 mV of zero. Take the leakage from the ends. At +12 V the output is 11.970 V. The knee `rb_knee` of 10 mV limits both ends. `rb_ron` is the resistance only where its drop is well above about 30 mV: 5 mohm at 1 A dropped 6.4 mV and not 5.0. Blocking reaches `rb_roff` only beyond about 100 mV in reverse. Near zero volts it leaks, from the formula 45 mA at 5 mV, 75 µA at 50 mV and 83 nA at 1 V.

### In your own schematic

This eFuse feeds the `/rect_out` net of the rectifier demo, with its own R2 and C1 as the load. Exclude V1, D1 and R1.

```
* eFuse into a net of the schematic. Exclude V1, D1 and R1 of the rectifier demo, and R2 and C1 are the load.
VBAT vbat GND DC 12
.param ef_ilim=5 ef_ron=20m
BEFZ vbat /rect_out I='ef_ilim*tanh(v(vbat,/rect_out)/(ef_ilim*ef_ron))'
```

## Fixing an Unmodelled Symbol

| `suggested_fix` | What to do |
|---|---|
| `copy_library_sim_fields` | Copy the `Sim.*` fields from the library symbol with `edit_schematic_component` |
| `exclude` | The part has no series role. Add it to `testbench.exclude_references` |
| `replace_series_part` | Removing it would break the path. Add a stand-in through `testbench.spice_lines`, such as the protection blocks above |

Nothing in `testbench` touches the schematic. Use the `spice_name` values from readiness in `spice_lines`.

## Reading a Result

- **`outcome.status`:** `complete` means the requested range was covered and every measurement found its signal. `partial` means the data is real but stops early or a signal was missing, and `stopped.reason` says why it stopped: `timeout`, `size_limit`, `no_convergence` or `incomplete_coverage`.
- **`analysis.coverage_ratio`:** how much of the requested time, frequency range or sweep ran. On a partial run every measurement and summary carries `window_truncated: true`, because it describes only the part that ran. Say so, and never report such a peak as the run's peak. On a complete run the same flag means that measurement's window reached past the saved data, such as before `start_save_seconds`, so its number leaves out that stretch.
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
| `floating_subcircuit` | Nodes have no DC path to ground, so their voltages are arbitrary. Refused for an operating point and a DC sweep | Add the missing path, or a large resistor through `spice_lines` |
| `no_ac_stimulus` | No source has an `AC` value, so an AC sweep would read zero | Set `ac=1` on a source, or add `VIN in 0 DC 0 AC 1` to `spice_lines` |
| `sweep_target_not_found` | The DC sweep names something the circuit does not have | Use a V, I or R reference, a testbench element's name, or `TEMP` |
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
