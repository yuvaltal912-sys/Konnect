# Simulation handoff

The current state of the simulation work. Anyone picking this up, human or agent, starts here. Update this file in the same commit that changes the state.

**Last updated:** 2026-10-31, with PR 6 built and under review.

## Start here

1. Read `AGENTS.md` for the rules.
2. Check out the branch under "Now".
3. Read the "Now" list, then the sections of `docs/SIMULATION_PLAN.md` it names.
4. Run the gate before changing anything, so you know the baseline.

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
| 6 | Not opened | `feat/simulation-run` | **In progress** |
| 7 to 11 | Not opened | | Not started |

## Now: PR 6, an isolated operating point

Branch `feat/simulation-run`, from `feat/simulation-raw`. Plan sections: 2.2 (Worker, Limits, Classification, Reading results), 2.3 (`run_simulation`) and 4.

- [x] Worker run mode in `crates/konnect/src/sim_worker.rs`, with process rights dropped before the deck loads
- [x] Classification and supervision in `simulation/worker_client.rs`
- [x] Run store in `simulation/runs.rs`, with retention
- [x] `run_simulation`, `operating_point` arm only. Each PR declares only the parameters its handler reads
- [x] `ToolErrorKind::SimulationFailed` and the outcome envelope
- [x] Starter skill `kicad-simulation`
- [x] Live tests: a real operating point, a worker crash, process rights, and open questions Q18, Q19 and Q20
- [ ] Negative controls, independent review, gate, PR

**Done when:** a real operating point runs, and a worker crash leaves the server alive with a typed error.

## After PR 6

| PR | Outcome |
|---|---|
| 7 | Transient, partial results, measurements, envelopes. The first slice that answers inrush questions |
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
- `.probe alli` is a fatal error next to any digital or event node. `run_simulation` leaves it out of those circuits and warns `device_currents_not_saved`. PR 7 needs the same rule.
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

- The hosted CI has never run on this fork. It needs the owner's click in the Actions tab.
- Q17 is an accepted limit: `filesource`, `d_source`, `d_state` and table models read a numeric data file named in the deck.
- Local scratch scripts for mutation runs and dogfooding were never committed. The recipes above replace them.
