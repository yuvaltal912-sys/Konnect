# Simulation handoff

The current state of the simulation work. Anyone picking this up, human or agent, starts here. Update this file in the same commit that changes the state.

**Last updated:** 2026-10-31, with PR 1 to 9 merged to `main` (PR 9 as `2503cad`) and PR 10 in progress.

## Start here

1. Read `AGENTS.md` for the rules.
2. Create the branch under "Now".
3. Read the "Now" list, then the sections of `docs/SIMULATION_PLAN.md` it names.
4. Run the gate before changing anything, so you know the baseline.
5. If a step here cannot run in your environment, such as `gh`, a push or the KiCad library, say so. Never skip it silently.

## The goal

A Konnect toolset that gives an assistant what KiCad's simulator does: run a schematic through ngspice and return compact, trustworthy results. The first user is low-voltage power distribution boards (inrush, load steps, protection) and analog circuits.

The pipeline: snapshot the saved sheets, export with `kicad-cli`, flatten and vet one deck in Rust, run preflight, load KiCad's libngspice in a hidden single-use `konnect sim-worker` process, then parse the raw file in Rust.

Decisions already made by the owner:

| ID | Decision |
|---|---|
| D1 | This lives in the owner's AGPL-3.0 fork, `yuvaltal912-sys/Konnect` |
| D2 | Digital XSPICE models are on. Vetting refuses `d_process` and `d_cosim` |
| D3 | Marked behavioural stand-ins are accepted. Such a result always reads `simulated_circuit: "modified"` |
| Scope | The first version simulates schematics only |
| Platforms | macOS first. Windows and Linux are validated at the end of the series |

## PR stack

PRs 1 to 9 are merged to `main` on the `fork` remote, each as a merge commit. Later PRs are based on `main`.

| Plan PR | GitHub | Branch | State |
|---|---|---|---|
| 1 to 3 | [#1](https://github.com/yuvaltal912-sys/Konnect/pull/1) | `feat/simulation` | Merged |
| 4 | [#2](https://github.com/yuvaltal912-sys/Konnect/pull/2) | `feat/simulation-preflight` | Merged |
| 5 | [#3](https://github.com/yuvaltal912-sys/Konnect/pull/3) | `feat/simulation-raw` | Merged |
| 6 | [#4](https://github.com/yuvaltal912-sys/Konnect/pull/4) | `feat/simulation-run` | Merged |
| 7 | [#5](https://github.com/yuvaltal912-sys/Konnect/pull/5) | `feat/simulation-transient` | Merged |
| 8 | [#6](https://github.com/yuvaltal912-sys/Konnect/pull/6) | `feat/simulation-protection` | Merged |
| 9 | [#8](https://github.com/yuvaltal912-sys/Konnect/pull/8) | `feat/simulation-sweeps` | Merged as `2503cad` |
| 10 | Not opened | `feat/simulation-stored-runs` | In progress |
| 11 | Not opened | | Not started |

## Now: PR 10 (stored runs)

PR 9 is merged. It added the `ac` and `dc_sweep` arms of `run_simulation`, with `no_ac_stimulus`, `sweep_target_not_found`, bandwidth, phase margin, gain margin, per-kind summaries and coverage proof. PR 10 adds `list_simulation_runs`, `measure_simulation_run` and CSV export. Ask the owner before starting PR 11.

### What PR 10 changed that the next PR needs to know

- **A finished run keeps `run.json`.** It is what `list_simulation_runs` and `measure_simulation_run` read: the schematic path, the analysis as run (`Analysis::record`, with a DC sweep's element and unit), `run_status`, `stopped`, `simulated_circuit`, `coverage_ratio`, `inputs_hash`, the inlined model files, and the net and element tables that turn `v(/NET)` and `i(REF)` into the engine's names. Only runs with data are recorded. A failed run, and any run from PR 6 to 9, has none: it is counted as `without_record` and not listed, and `measure_simulation_run` says `run_not_found`. PR 11 should read the same record, through `runs::read_record`.
- **A stored measurement is the live one.** `Evaluation` (in `tools/simulation.rs`) holds the signal lookup and the per-analysis gathering that `RunEnd` used to hold, and both a live run and a stored one go through it. `Wanted::for_analysis` is the argument check for a known analysis. A new analysis kind must add its arm in `Analysis::record`, `from_record` and `Evaluation`, or stored runs will not read it.
- **`runs::locate` takes an id and nothing else.** It checks the exact shape first, builds `<state>/simulation/<first 12>/<id>` and refuses a link (`symlink_metadata`). `list` and `read_record` never follow a link either. PR 11 must reach a run only through `locate`.
- **`inputs_changed_since_run`** is the hash of the saved sheets and `.kicad_pro` taken now (`CurrentSources`, a snapshot and `hash_sources`, no export) with the run's inlined model files, against the run's `inputs_hash`. `null` when the sheets cannot be copied. A model file that has gone is a change.
- **`results.csv` is written for every run** (`export.rs`, 20 million values at most, then `csv_path: null` with a `csv_not_written` warning). The scale of a DC sweep is renamed for what it varies, and a second sweep gets a column of its own, rebuilt from the point number. Names that a spreadsheet would run as a formula get a leading `'`.
- **Two sweeps: left out on purpose.** A slice per outer value would need a result shape of its own (one measurement list per outer step, an `outcome` that counts them, and a bound on how many a call may return), and PR 10's job was to make a stored run measurable. So a two-sweep run takes `signals` and no measurements, as before, and its whole grid is in `results.csv` with both sweeps' values, which a script can slice. If the owner wants slices, it is a later PR that adds an `outer_step` selection to `measure_simulation_run`.
- **Runs made in one second** sort by their random end, so "newest first" is only exact to the second.
- **`measure_simulation_run` needs no engine.** It works in a process with no worker, and with the engine gone.

### What PR 9 changed that the next PR needs to know

- **Protocol is 4.** `RunRequest.complex_data` tells the worker an AC sweep saves 16 bytes a value, and `data_bytes` counts by it. Without it an AC run reached twice `max_raw_bytes` (E99). A fake worker's `status.json` must say `PROTOCOL_VERSION`, not a number.
- **The engine's plots have quirks that Konnect works around.** A DC plot holds one scale vector, named `v-sweep`, `i-sweep`, `res-sweep` or `temp-sweep` whatever the target, and with two sweeps the first varies fastest and the second is not saved: its value is rebuilt from the point number. An AC plot's scale is `frequency`, complex. PR 10's stored runs and CSV must handle both.
- **The AC grid ends short.** ngspice ends an octave sweep, and some decade ones, on the last grid point at or below `stop_hz`, up to a step short (`dec 10 1 5` ends at 3.98). `Ac::covered` accepts that and refuses a run cut where the next point is the stop. The plan's "last frequency reaches `stop_hz`" would have made those runs partial.
- **A step that points away from the stop writes no data and reports success.** `Sweep::from_args` refuses it. So does a step lost in rounding, and `lin 2` (one point) and frequencies outside 1e-9 to 1e18.
- **The engine adds its DC step each time,** so a sweep can end one point short (`TEMP` by 0.1 stops at 99.9). The count is the plan's, give or take one, and a short end warns `sweep_ends_before_stop`, by count: a missing outer row shows as `TEMP ran 1000 of 1001 steps`.
- **Phase is doubtful where a sample turns it over 120 degrees, or the gain jumps over 10 dB by more than the step before it** (the first step is not judged, so a pole at the start of a linear sweep passes; scaling the limit by the step let an unstable loop through, E102). Margins there are null with `phase_unreliable`. **An all-pass or a long delay aliases unseen** and reads a stable margin at 50 and even 100 points a decade: this cannot be fixed from the samples, so the skill says to run twice at double the density. `bandwidth_3db` is null with `sweep_too_coarse` when its edge lies in a step over 10 dB, or a peak-referenced one has its peak between samples (a parabola through the highest sample and its neighbours rises over 0.15 dB above it).
- **Measurements are per analysis.** `measure::parse` takes an `AxisSpec`; `Kind::fits` says what each analysis takes, and a field of another analysis is refused. A falling sweep is measured on a negated axis and put back (`Measurement::sign`). Two sweeps take `signals` and no measurements. PR 10's `measure_simulation_run` should reuse `parse` and `Kind::fits`.
- **AC margins read a first crossing,** and the result shows `start_phase_degrees`, `crossover_hz` and `phase_crossover_hz`, because a loop gain taken with the wrong sign reads 180 degrees off and only the start phase shows it. `ac.rs` is Rust-only and shares nothing with the engine.
- **`floating_subcircuit` now refuses a DC sweep as well as an operating point,** since both solve at DC. The plan said operating point only.
- **The E29 deck was not kept.** `ac_loop.sp` and `ac_loop_reference.py` replace it, as `protection/` replaced E56. The plan's old numbers (fc 30145 Hz, PM 1.77) belong to a deck nobody has.
- **Reversed battery:** the sweep and a ramp at 0.5 µs steps agree at ten places from -12 to 12 V: to a part in 10^9 from 6 V up, in 10^6 at 1 V, and within 0.19 percent at ±0.05 and ±0.1 V. A ramp at 10 µs read 25 percent off at -50 mV by interpolation alone (the knee is exponential). The stand-in has a second, conducting DC solution between -0.56 V and 0, and which one the engine finds depends on direction and step (down from 0 it latches; up at 0.01 V it also latched, at -45 mV). So only the ends and the 0.05 V upward sweep are results.

### What PR 8 changed that later PRs need to know

- **The skill's decks are the tested decks.** They live in `crates/konnect-core/tests/fixtures/simulation/protection/*.sp`, and `crates/konnect/tests/protection_skill.rs` fails if a line is missing from the skill. Change a deck and the skill together.
- **`exp()` in a behavioural expression is linear above 14.** `exp(15)` reads twice `exp(14)`. Write powers with `**` (E98).
- **A latch is needed for a fuse.** Without one the fuse settles half open (E97). The integral overshoots its trip by up to the current squared times the step.
- **Done in PR 9:** `reverse.sp` runs unchanged as a `dc_sweep` from -12 to 12 V and agrees with the ramp.

### What PR 7 changed that later PRs need to know

- **`max_raw_bytes` counts in `SendData`,** on the engine's own thread. Polling `ngGet_Vec_Info` from the main thread does not work (E90). `data_bytes` takes the bytes per value, and PR 9 sends 16 for an AC sweep.
- **`measure.rs` is Rust-only and time-weighted.** A signal is straight lines between samples, and RMS and I²t use the exact integral of that line squared, not the trapezoid of the squares (E94).
- **A step measurement with no `final_value` uses the mean of the window's last tenth,** behind stillness checks, and shows the `start_value` and `final_value` it used. The rules are in the plan under Measurements. They will never catch every window that ends on the wrong level: E96 names the known misses.
- **`.probe` changes the circuit.** Under `uic`, it loses any voltage set by `.ic` or `.nodeset`, so Konnect leaves out its own probes and refuses a caller's. It also stops the run on a `K` coupling (E92, E94, E95). PR 8's stand-ins are added through `spice_lines`, so they must not add `.probe`.
- **`i(l1)` appears twice** in the raw file, differing by rounding. `signals::vector` treats that as one signal, and real disagreement as none.
- **`.options interp` is refused,** because it thins the saved samples (E95).

## After PR 10

| PR | Outcome |
|---|---|
| 11 | `plot_simulation_run`, SVG |
| Any | `fix(schematic): copy Sim fields on placement` |

Windows and Linux follow all of these: `.github/workflows/e2e-kicad.yml` gets its live-test step then, and Q1, Q2, Q6 and Q7 are settled then.

## Where things are

| Path | Holds |
|---|---|
| `docs/SIMULATION_PLAN.md` | Design, evidence (E numbers) and open questions (Q numbers) |
| `crates/konnect-core/src/tools/simulation.rs` | Tool definitions and handlers |
| `crates/konnect-core/src/simulation/snapshot.rs` | Private copy of the saved sheets |
| `crates/konnect-core/src/simulation/kicad_netlist.rs`, `net_names.rs` | Pin-to-net map and SPICE names |
| `crates/konnect-core/src/simulation/flatten.rs` | One vetted deck. This is the security boundary |
| `crates/konnect-core/src/simulation/element.rs`, `preflight.rs` | Deck reader and checks |
| `crates/konnect-core/src/simulation/raw.rs` | Raw file parser |
| `crates/konnect-core/src/simulation/runs.rs` | Run folders: create, prune, `locate` by id, `list`, and the `run.json` record |
| `crates/konnect-core/src/simulation/export.rs` | The plot as CSV |
| `crates/konnect-core/src/simulation/signals.rs`, `measure.rs`, `ac.rs` | Signal grammar and lookup (real and complex), measurements on any axis, and AC gain, phase and margins |
| `crates/konnect-core/src/tools/simulation/stored.rs` | `list_simulation_runs` and `measure_simulation_run` |
| `crates/konnect-core/src/tools/simulation/sweeps.rs` | The `ac` and `dc_sweep` analyses: arguments, deck line, target lookup, coverage |
| `crates/konnect-core/src/simulation/worker_client.rs`, `protocol.rs` | Worker supervision and its contract |
| `crates/konnect/src/sim_worker.rs` | The only FFI. `konnect-core` stays free of `unsafe` |
| `crates/konnect/tests/simulation_live.rs` | Ignored tests that need KiCad |
| `crates/konnect-core/tests/fixtures/simulation/` | Real KiCad and ngspice fixtures, with provenance. `soft_start.sp` and its reference script stand in for E31, `protection/` and its `reference.py` for E56, and `ac_loop.sp` and its reference script for E29 |
| `crates/konnect/assets/skills/kicad-simulation/SKILL.md` | The skill, including the protection stand-ins |

## Graph

A knowledge graph of the code and docs lives in `graphify-out/`, on this Mac only. Git ignores it, along with `.codex/` and the local skill copy, so a fresh clone has none of them. `AGENTS.md` says how to use it. The git hooks here keep it current after each commit, with no LLM cost. `.graphifyignore` leaves out app icons and test data.

`graphify query` is noisy on a graph this size. When you know a symbol, `graphify explain` and `graphify path` work better.

To build it elsewhere, run `graphify extract . --backend claude-cli`, then `graphify cluster-only . --no-viz --backend=claude-cli`.

## Verify

Run this on every PR, after the last commit:

```bash
export PATH="$(brew --prefix)/bin:$PATH" KICAD_DEMOS="$HOME/ClaudeCode/kicad-10.0.1/demos"
cargo fmt --all -- --check
cargo test --workspace --locked --lib --tests --no-fail-fast
cargo test --workspace --locked --doc
cargo test -p konnect --locked --test simulation_live -- --ignored
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo deny check licenses
cargo xtask fix-doc-counts
cargo build --release --locked -p konnect
shasum -a 256 ~/.local/share/konnect/bin/konnect
```

**Dogfood the fork by pipe.** Run `target/release/konnect` with `HOME` and `KONNECT_STATE_DIR` set to `$PWD/target/dogfood-home`. On stdin send one JSON-RPC line each for `initialize`, then `tools/call` of `load_toolset` with `{"name":"simulation"}`, then the tool. Trust the result only when `get_installation_info` shows `runtime.executable_path` as the fork binary and `build.commit` as HEAD. Use scratch copies of `~/ClaudeCode/kicad-10.0.1/demos/simulation`.

**Negative controls.** For each guard, break it in the source, run the tests, confirm one fails, and restore it. `GOVERNANCE.md` asks for this. PR 3 to 5 were held to it.

**Source references.** KiCad 10.0.1 is at `~/ClaudeCode/kicad-10.0.1`. The ngspice 44.2 source is not kept on disk, so fetch it into a new empty folder when needed.

## Traps that already cost time

- `ngSpice_Circ` and `ngSpice_Command` return 0 on failure. `$sim_status`, point counts and coverage are the evidence.
- A halted run writes a complete-looking raw file. Only the worker's own record says it stopped.
- `write` after a failed analysis falls back to the `constants` plot, and the parser refuses that.
- The worker runs in the C locale, so ngspice turns every non-ASCII byte in a node name into `_`. Look signals up by the engine's name, such as `v(/__)`.
- `.probe alli` is a fatal error next to any digital or event node. `run_simulation` leaves it out of those circuits and warns `device_currents_not_saved`.
- A `SIGSEGV` sent from outside does not kill the worker: Rust's handler swallows it. Test crashes with `SIGABRT`.
- ngspice expands `$`, `{` and backticks in commands, and copies `SPICE_SCRIPTS` into a fixed buffer. So `write` takes a relative name and `SPICE_SCRIPTS` is `.`. Never put a path in a command.
- A `bg_halt` sent before the engine's thread starts marks the engine as stopped, and every later one is refused. The worker waits for the thread's own signal before halting.
- ngspice rewrites deck lines in its messages, so matching whole lines cannot keep model text out of errors. `engine_complaints` compares words.
- `ngSpice_nospinit` and `ngSpice_nospiceinit` crash on 44.2 if called before init. The worker shadows those files instead.
- `ngSpice_Circ` stops at the first `.end`, and the first line sent is the title.
- A dangling symlink passes path checks, so test an interpreter by running it.
- Editing Rust with a script breaks after `cargo fmt` reshapes the text. Check the file before rerunning a partly applied edit.
- The default transient step is `stop / 2000`. On a 0.14 s run that is 70 µs: the values stay within 1 percent, but a peak on a flat top lands 0.6 ms early. A smaller step is not always a longer run: 2 µs took 1,019,783 points where 0.5 µs took 281,020.
- `exp()` in a B source or behavioural `R=` is exact only up to an argument of 14, and linear above it, so `exp(ln(1G))` reads 9.3 million. Use `**` or `pow()`.
- A hard-edged control did not run away in any rebuilt short (E98). E57's deck is gone, so do not claim the cause. The failure seen is a lockout chattering, which ends in `no_convergence`.
- Under `uic`, ngspice saves no point at time zero, so `first_point_seconds` is above 0.
- `.dc` writes no data and reports success if its step points away from the stop. Konnect refuses that first. A capacitor or inductor target is `Fatal error`, so only V, I, R and `TEMP` are accepted.
- The sweep scale in a DC plot is named `v-sweep` and its kin, not after the target, and a second sweep's value is not saved.
- An AC plot's grid does not always reach `stop_hz`: an octave sweep ends on its last point at or below it, and a decade one may. `lin` with 1 point is the start alone.
- `p()` means nothing in AC: `r1:power` is a complex apparent power. Konnect refuses it there.
- The Mac has no `timeout` command. A probe script needs its own time limit.
- `cargo fmt` reshapes a Rust file, so a script that edits one by exact text breaks after a format. Match by pattern, or edit by hand.
- A reviewer's scratch `HOME` moves KiCad's IBIS cache path, so the `ibis` demo only runs from the real home.
- Tests that touch security use nonexistent paths. Engine tests use harmless `echo` and marker files only.

## Known open items

- A phase margin or gain margin is read from the first crossing only, and a loop gain with a low-frequency phase of exactly -180 degrees depends on the sign of rounding in the imaginary part. The result shows `start_phase_degrees` so the reader can see it. A gain margin from a start within 0.001 degree of -180 reads `starts_on_level`.
- `no_ac_stimulus` counts any V or I source with a non-zero `AC` value, including one inside a subcircuit that nothing calls, so a circuit whose only AC source is in an uncalled subcircuit passes and reads zero.
- **The undersampling tests are a best effort.** Four review rounds each found another edge of them. Left as limits: a deep notch below crossover is refused at sensible densities (a 50 dB notch at 300 Hz needed 800 points a decade), and an all-pass or long delay aliases unseen. The dependable control is two runs at double the density. If a later PR touches this, consider making `run_simulation` do that second run itself when a margin is asked for.
- A top-level element in a file that a testbench `.include`s is refused as `unknown_testbench_node`, though the plan says lines from such a file count. Found in PR 9's review, in PR 4's code, and not fixed.
- A DC sweep of one point (start equals stop), or an AC sweep that holds one point, reads `complete` with `outside_data` for every window measurement. An AC sweep with under two points is refused; a DC one is not.
- With two sweeps `signals` summarise the whole grid and measurements are refused, live and stored. A slice per outer value needs a result shape of its own, and PR 10 left it out (see above). `results.csv` has the grid.
- A `bandwidth_3db` measured from the lowest frequency on a band-pass reads the first fall below that level, which can be far above the band. The result shows `reference_gain_db`, and `reference_level: "peak"` gives the band's own edges.
- `bg_halt` stops an AC sweep at the size limit and at the budget: a 1 MB limit halted a 90,005-point sweep at 34,407 points. `.op` still has no pause test.
- `.ic` inside a `.subckt` is refused by preflight as `unknown_testbench_node`, or warned as dropped, though ngspice applies it. A false refusal, from PR 4.
- A `.ic` in a dead `.if` still drops device currents under `uic`. It fails loudly.
- `p(REF)` on a subcircuit counts only the pins it shows, so a hidden ground or supply reads short.
- A net named `time` or `frequency`, and `/out#branch`, read `signal_not_found`. No wrong number comes out.
- A part with `Sim.Enable` off reads `as_drawn`. The first PR 7 review reported this from before PR 7, and it has not been rechecked.
- On a partial run, a measurement wholly inside the data reads `window_truncated: true` and counts as failed, as the plan says, though its number is exact.
- Code-model `ic=` under probes, and `bg_halt` when one step takes over a second, have not been tested.
- A value computed from a model parameter can still appear in an engine message, because nothing can recognise it. Worth knowing, not worth more code.
- A real segfault inside libngspice has not been reproduced on 44.2. The crash path is proven with a fake worker and with `SIGABRT` on the real one.
- `.op` has no pause test, so `bg_halt` cannot stop one. It ends at the worker's own deadline.
- `class-d`, `laser_driver` and `v_i_sources` report `engine_messages`. `laser_driver` and `v_i_sources` are true positives. The `class-d` lines were not rechecked after progress notes were filtered out.
- The protection stand-ins are approximations with known edges, all in the skill: the fuse opens in one step with no arc, so inductance ahead of it rings (E97); the reverse stand-in leaks near zero volts because of its 10 mV knee; the eFuse resistance rises near its limit. The independent review of PR 8 found these, not wrong numbers.
- The hosted CI has never run on this fork. It needs the owner's click in the Actions tab.
- Q17 is an accepted limit: `filesource`, `d_source`, `d_state` and table models read a numeric data file named in the deck.
- Local scratch scripts for mutation runs and dogfooding were never committed. The recipes above replace them.
