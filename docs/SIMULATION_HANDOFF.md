# Simulation handoff

The current state of the simulation work. Anyone picking this up, human or agent, starts here. Update this file in the same commit that changes the state.

**Last updated:** 2026-10-31, with PR 1 to 6 open and PR 7 handed over.

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

Each PR is based on the one before it. None is merged.

| Plan PR | GitHub | Branch | State |
|---|---|---|---|
| 1 to 3 | [#1](https://github.com/yuvaltal912-sys/Konnect/pull/1) | `feat/simulation` | Open |
| 4 | [#2](https://github.com/yuvaltal912-sys/Konnect/pull/2) | `feat/simulation-preflight` | Open |
| 5 | [#3](https://github.com/yuvaltal912-sys/Konnect/pull/3) | `feat/simulation-raw` | Open |
| 6 | [#4](https://github.com/yuvaltal912-sys/Konnect/pull/4) | `feat/simulation-run` | Open |
| 7 | Not opened | `feat/simulation-transient` | **Next** |
| 8 to 11 | Not opened | | Not started |

## Now: PR 7, transient analysis and measurements

The owner has approved this PR. It is the first slice that answers inrush and load-step questions.

**Branch:** create `feat/simulation-transient` from `feat/simulation-run`. Open the PR against `feat/simulation-run`, titled `feat(simulation): transient analysis and measurements`.

**Plan sections to read first:** 2.2 (Limits, Classification, Reading results, Signals, Measurements), 2.3 (`run_simulation` schema, transient output, failure matrix) and 6.

**Done when:** a real transient runs through `run_simulation` and returns correct inrush and load-step measurements, and a run that stops early reads as `partial` with its data kept.

### Scope

- [ ] `transient` arm in `analysis`: `stop_time_seconds`, `max_step_seconds`, `start_save_seconds`, `use_initial_conditions`. `operating_point` stays the last `oneOf` arm
- [ ] New `run_simulation` parameters: `signals`, `measurements`, `envelope_points`, `max_raw_bytes`
- [ ] Signal grammar: `v(NET)`, `v(NET1,NET2)`, `i(REF)`, `i(REF:PIN)`, `p(REF)`, resolved in Rust
- [ ] New `simulation/measure.rs` with the transient measurements below
- [ ] `signal_summaries`, with min, max, final and an optional envelope
- [ ] Size limit: the worker halts softly at `max_raw_bytes`
- [ ] Docs: plan, `kicad-simulation` skill, `tool-directory.md` and this file
- [ ] Unit tests, live tests, negative controls, an independent review, gate, PR

**Measurements in this PR:** `min`, `max`, `peak_to_peak`, `peak_abs`, `average`, `rms`, `integral`, `i2t`, `value_at`, `crossing`, `rise_time`, `fall_time`, `overshoot_percent`, `settling_time`.

**Not in this PR:** the AC measurements and every `_hz` and `_sweep` field (PR 9), protection stand-ins (PR 8), and stored-run tools (PR 10). A tool declares only the parameters its handler reads.

### Where PR 6 is hard-wired to an operating point

| Place | Change |
|---|---|
| `run_analysis` in `tools/simulation.rs` | Refuses every kind but `operating_point` |
| `prepare_run` | Picks the deck's own lines, and needs the analysis line |
| `RunEnd::report` | Passes `operating_point_covered` to `classify`, and always builds rows |
| `RunEnd::result_body` | `coverage_ratio` is 0 or 1, and `operating_point` is always present |
| `HaltReason` in `protocol.rs` | Needs `SizeLimit` |
| `classify` in `worker_client.rs` | Reads `let HaltReason::Timeout = reason`, and must name both reasons |
| `RunRequest` in `protocol.rs` | Needs the size limit. Bump `PROTOCOL_VERSION`; the fake-worker JSON in `tools/simulation.rs` hard-codes `"protocol":2` |
| `MAX_RAW_BYTES` in `worker_client.rs` | A fixed 1 GiB read limit, which must stay above `max_raw_bytes` |

### Decided

- **Measurements run in Rust, time-weighted.** Adaptive steps cluster on edges: naive RMS read 5.30 where the true value is 2.69 (E28). Never use `.meas`.
- **Signals never enter the deck.** `render_with` lines carry no caller text. Only numbers, written by Konnect, go into the `.tran` line. Reject non-finite and non-positive values before the deck is built.
- **No summaries unless `signals` is given.** This keeps the default result small.
- **A value that does not exist is `null`, never 0,** with a `finding` such as `level_not_reached`. That is an engineering result, not a failed check.
- **Outcome counts.** `requested` is 2 plus the measurement count. On a partial run each value carries `window_truncated: true` and counts as failed. A signal missing after the run is partial, with `signal_not_found`.
- **Coverage.** A transient is proven when the last time reaches the stop time within tolerance. `coverage_ratio` becomes a real ratio.
- **Non-ASCII nets.** Accept the name KiCad shows, and map it with `preflight::engine_node` before lookup. If two nets share an engine name, say so.

### Check these first, because nothing here has proven them

- **Reading vector length during a run.** The plan has the worker poll it with `ngGet_Vec_Info`. No run has tested that from the main thread while `bg_run` is going. If it is unsafe, read `SendStat` or another signal, and record what you find as E90.
- **The `.tran` step argument.** The plan does not fix it. `.tran 1n 10` is fatal: it died with exit 139 in E10 and ended in `malloc: can't allocate` in E88. `.tran 5m 10 0 1n` survives (E10). Keep the print step a fixed fraction of the stop time, and test the extremes.
- **`p(REF)`.** E30 shows `m1:power`, but which devices report it is not settled. KiCad adds one `.probe p(NAME)` per item and skips XSPICE parts (`eeschema/netlist_exporters/netlist_exporter_spice.cpp` near line 724). Do the same only for the references asked for.
- **`.probe alli` during a transient.** It multiplies vectors, so check memory against the default 256 MiB limit. It stays out of circuits with `event_nodes`.
- **Halting.** `bg_halt` was proven on a transient (E57 and the live tests), but only with the budget. Prove the size limit the same way.

### Tests the plan asks for

- **Unit, against values worked out independently of the code:** RC charge, RLC overshoot and settling, I²t against the analytic value, and the naive-against-weighted RMS regression. This settles Q12.
- **Classification:** every row, including `size_limit`, with zero points and with some.
- **Live:** soft-start inrush and a load step, compared with E31 (16.56 A peak, I²t 0.179, 96 mV droop), plus a soft halt on the budget and on the size limit. The E31 deck was not kept, so build a new one and record it in a fixture README.

## After PR 7

| PR | Outcome |
|---|---|
| 8 | Protection stand-ins in the skill: short, I²t fuse, eFuse limit, UVLO, reversed battery |
| 9 | AC and DC sweeps, `no_ac_stimulus`, `sweep_target_not_found` |
| 10 | `list_simulation_runs`, `measure_simulation_run`, CSV |
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
| `crates/konnect-core/src/simulation/worker_client.rs`, `protocol.rs` | Worker supervision and its contract |
| `crates/konnect/src/sim_worker.rs` | The only FFI. `konnect-core` stays free of `unsafe` |
| `crates/konnect/tests/simulation_live.rs` | Ignored tests that need KiCad |
| `crates/konnect-core/tests/fixtures/simulation/` | Real KiCad and ngspice fixtures, with provenance |

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
- Tests that touch security use nonexistent paths. Engine tests use harmless `echo` and marker files only.

## Known open items

- A value computed from a model parameter can still appear in an engine message, because nothing can recognise it. Worth knowing, not worth more code.
- A real segfault inside libngspice has not been reproduced on 44.2. The crash path is proven with a fake worker and with `SIGABRT` on the real one.
- `.op` has no pause test, so `bg_halt` cannot stop one. It ends at the worker's own deadline.
- `class-d`, `laser_driver` and `v_i_sources` report `engine_messages`. `laser_driver` and `v_i_sources` are true positives. The `class-d` lines were not rechecked after progress notes were filtered out.
- The hosted CI has never run on this fork. It needs the owner's click in the Actions tab.
- Q17 is an accepted limit: `filesource`, `d_source`, `d_state` and table models read a numeric data file named in the deck.
- Local scratch scripts for mutation runs and dogfooding were never committed. The recipes above replace them.
