# Simulation handoff

The current state of the simulation work. Anyone picking this up, human or agent, starts here. Update this file in the same commit that changes the state.

**Last updated:** 2026-11-10. Simulation v1, its follow-up fixes, the reliability repairs (#15), M1 (#16) and M2 (#18) are merged. The ngspice version-pin fix is merged as #23. M3's first part (op-amp, comparator, LDO, series reference) is merged as [#53](https://github.com/yuvaltal912-sys/Konnect/pull/53) (`f4dde06`, 2026-11-09), after its review follow-up. M3's second part (load switch, eFuse, ideal diode) is open in [#54](https://github.com/yuvaltal912-sys/Konnect/pull/54) on branch `feat/simulation-datasheet-models-m3-switches`: an independent review found four verified findings, not fixed yet. It waits, because the owner gives creation work priority (2026-11-09).

## Start here

1. Read `AGENTS.md` for the rules.
2. Create a branch from `main` for the work, unless "Now" names an open one to continue.
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
| D4 | (2026-11-05) Models fitted to datasheet values live in the project's `simulation_models/` folder and attach through `Sim.*` fields. Their results read `as_drawn`, and disclose the parts as `fitted_model` in `circuit.approximations`, the plot banner and the listing. D3 still governs testbench stand-ins and exclusions |
| Scope | The first version simulates schematics only |
| Platforms | macOS first. Windows and Linux are validated at the end of the series |

## PR stack

PRs 1 to 11, the three fixes after them, the reliability repairs, M1, M2 and M3's first part are merged to `main` on the `fork` remote, each as a merge commit.

| Plan PR | GitHub | Branch | State |
|---|---|---|---|
| 1 to 3 | [#1](https://github.com/yuvaltal912-sys/Konnect/pull/1) | `feat/simulation` | Merged |
| 4 | [#2](https://github.com/yuvaltal912-sys/Konnect/pull/2) | `feat/simulation-preflight` | Merged |
| 5 | [#3](https://github.com/yuvaltal912-sys/Konnect/pull/3) | `feat/simulation-raw` | Merged |
| 6 | [#4](https://github.com/yuvaltal912-sys/Konnect/pull/4) | `feat/simulation-run` | Merged |
| 7 | [#5](https://github.com/yuvaltal912-sys/Konnect/pull/5) | `feat/simulation-transient` | Merged |
| 8 | [#6](https://github.com/yuvaltal912-sys/Konnect/pull/6) | `feat/simulation-protection` | Merged |
| 9 | [#8](https://github.com/yuvaltal912-sys/Konnect/pull/8) | `feat/simulation-sweeps` | Merged as `2503cad` |
| 10 | [#9](https://github.com/yuvaltal912-sys/Konnect/pull/9) | `feat/simulation-stored-runs` | Merged as `d6b5ea8` |
| 11 | [#10](https://github.com/yuvaltal912-sys/Konnect/pull/10) | `feat/simulation-plots` | Merged as `7bd17c1` |
| Fix | [#11](https://github.com/yuvaltal912-sys/Konnect/pull/11) | `fix/simulation-false-refusals` | Merged as `d6f0d51` |
| Fix | [#12](https://github.com/yuvaltal912-sys/Konnect/pull/12) | `fix/schematic-sim-fields` | Merged |
| Fix | [#13](https://github.com/yuvaltal912-sys/Konnect/pull/13) | `fix/replace-component-sim-fields` | Merged as `5d5eeb5` |
| Repairs | [#15](https://github.com/yuvaltal912-sys/Konnect/pull/15) | `fix/simulation-validation-and-results` | Merged as `15f7137` |
| M1 | [#16](https://github.com/yuvaltal912-sys/Konnect/pull/16) | `feat/simulation-datasheet-models` | Merged as `d10dbe1` |
| M2 | [#18](https://github.com/yuvaltal912-sys/Konnect/pull/18) | `feat/simulation-datasheet-models-m2` | Merged as `d228d5f` |
| Version check | [#23](https://github.com/yuvaltal912-sys/Konnect/pull/23) | `fix/live-test-ngspice-version` | Merged as `bdafc67` |
| M3 (first part) | [#53](https://github.com/yuvaltal912-sys/Konnect/pull/53) | `feat/simulation-datasheet-models-m3` | Merged as `f4dde06` |
| M3 (second part) | [#54](https://github.com/yuvaltal912-sys/Konnect/pull/54) | `feat/simulation-datasheet-models-m3-switches` | Open: four verified review findings to fix first |

## Datasheet models: M1 and M2 merged

The owner asked for a design and its parts' datasheets to be enough to simulate. On a real power board 102 of 116 parts had no model (E75). The series is in the session plan. M1 and M2 are merged as PRs #16 and #18, and M3's first part as #53:

| PR | Delivers |
|---|---|
| M1 | Readiness groups the parts that need models. `create_simulation_model` fits diodes, Schottky diodes, LEDs and Zeners, plus resistances, capacitors and inductors. `assign_simulation_model` attaches a fitted model to a batch of parts. Runs disclose them |
| M2 | MOSFET (VDMOS, as a subcircuit, with a gate-charge check), TVS, BJT. **All done** (TVS: `tvs`, `tvs_bidirectional`; MOSFET: `n_mosfet`, `p_mosfet`; BJT: `npn`, `pnp`); merged as [#18](https://github.com/yuvaltal912-sys/Konnect/pull/18) |
| M3 | Behavioural ICs. **Merged in #53:** `op_amp`, `comparator`, `ldo`, `voltage_reference`. **Open in #54:** load switch, eFuse, ideal diode (its review findings first; see "M3 progress") |
| M4 | A standalone SPICE netlist in place of a schematic |
| M5 | Vendor library import and attachment, with raw-ngspice safety vetting |

What M1 built, and what later work must keep:

- **Konnect never reads a datasheet.** The caller gives `characteristics`: `{quantity, value, conditions, statistic}` in SI base units, so no scale letter can enter. Each kind has a closed list of quantities. A curve with more points than its fit needs holds its middle point out as an independent check.
- **The fits** (`simulation/models/`) are the diode law by least squares on the voltage, the junction capacitance law, ngspice's breakdown for a Zener (a forward and a breakdown diode, each with its own series resistance), and exact closed forms for the passives. Two traps were found on the engine: the solver's default tolerances stop a breakdown 30 mV short at 30 V, so a bench solves at `reltol=1e-6`; and with a Schottky's large IS ngspice's current at BV falls short by `IS (BV / Vt - 1)`, which `breakdown_ibv` adds back. A golden-section search that ends at zero series resistance returns a sliver, and a sliver makes ngspice's matrix singular, so a negligible one is written as none.
- **Nothing is written unless the engine reproduces every fitted and held-out value within `tolerance_percent`.** A bench is staged in a private folder, which is its only include root, and runs at `.temp` equal to the datasheet temperature with `TNOM` written to match. A blocked engine or a failed row writes nothing. A `check_only` row, such as a typical reverse current, is reported and never stops the write; a datasheet limit there passes on its own side.
- **A fitted file holds no caller text.** KiCad's GUI includes a model file straight into ngspice, where a `*#` line runs as a command. So the file is one marker line, with a hash of the body, then cards in a narrow grammar of numbers Konnect formats; `text::check_fitted` accepts that grammar and nothing else. The values and the datasheet reference go in a `.konnect.json` sidecar. Publishing is one `commit_file_transaction`, and a file is replaced only if it still passes the check and keeps its pins.
- **`assign_simulation_model` takes only Konnect-fitted files.** It writes `Sim.Library` as `${KIPRJMOD}/simulation_models/<name>.lib` (KiCad tries a bare relative path against its working folder first), `Sim.Name`, `Sim.Device` and `Sim.Pins`, through `plan_sim_field_swap`. Before any write, a private copy of the project is exported with `kicad-cli`, and each part must come out as an element of the model with every model pin on its mapped pin's net. It refuses: a lock on any sheet (Eeschema saves every sheet it holds); a connected pin left unmapped, unless it is stacked on a mapped pin's net; a pin map that contradicts the symbol's A/K names; an open jumper; a coupled inductor; and an excluded or legacy-enabled part. All sheets go in one transaction, then each part is read back.
- **Disclosure.** `flatten` records Konnect's marker on each consumed file (`FlatDeck.fitted_inputs`), with the names the file defines, read loosely so an edited file still names its cards; a marker below an added line still marks the file, as edited. Preflight names the parts on those models as `fitted_model`, including a hand-written subcircuit that calls one. It names those that KiCad's dialog changed (a regenerated `REF.name` card, or a diode's `m` or `area` on its element line) as `fitted_model_overridden`, and those on an edited or name-sharing file as `fitted_model_unverified`. All three list part references and are approximations. Plots draw a FITTED MODELS banner line that counts the parts and how many may no longer match their check, and listings carry the codes.
- **A fitted passive is a subcircuit** with a 0 V source, `VKONNECT`, at port 1. `i(REF)` reads `i(xref)` where probes ran, and `i(v.xref.vkonnect)` where they did not, as under `use_initial_conditions` with a `.ic`.
- **Readiness** adds `parts_needing_models`, one entry per kind of part: references, sheet files, datasheet (a URL, a plain file inside the project, or raw text), allowlisted part-number fields, library pins with bare names, and the library's `Sim.Pins`. It covers parts KiCad runs on a default model too (`kicad_default_model`).
- **Review repairs.** Three adversarial reviews of M1 confirmed 26 findings, and all are fixed. An inductor is checked with a current, because a voltage source across it is a loop at DC, and a low-Q inductor with a winding capacitance fits on either side of its reading. A capacitor near its resonance fits below it. A zero bias is accepted where a datasheet prints one, such as CJO at 0 V. An identical rewrite writes nothing and reports `unchanged`. Assign scans KiCad's locks again just before its commit. After a failed commit each file is read back: this call's text, or a journal that can still finish the write, reports `uncertain`; another writer's change is a conflict with a safe retry; otherwise nothing was written. A Konnect sidecar left without its model is replaced, and a foreign one never is. `unpowered_net` ignores a fitted passive's 0 V sense source.
- **Tests:** units in `simulation/models/*`, `simulation/preflight.rs`, `tools/simulation/parts.rs`, `tools/simulation_models.rs` (refusals that write nothing), `signals.rs`, and six live tests at the end of `simulation_live.rs`.

Limits left in M1: one temperature per model, and no temperature check; no reverse recovery (no transient bench yet); a fuse never opens; capacitance and inductance are linear at their stated bias; a Zener's very soft knee is held at the largest `NBV` that keeps ngspice's knee above 30% of BV, and the knee then reads low; reused sheets are refused, not assigned. Disclosure rests on the marker, so a file whose marker line was deleted reads as an ordinary model. A commit that stops while recovering an earlier journal naming the same file answers `uncertain`, the cautious side, though this call wrote nothing. The KiCad dialog round trip and a run in KiCad's GUI simulator have not been checked: they are owner checks. The installed KiCad is 10.0.8 with ngspice 45.2, while the KiCad facts come from the 10.0.1 source; one live test still pins ngspice 44.2 and fails for that reason alone, a red baseline reported and not fixed here.

Gate on macOS, 2026-11-06: formatting, workspace tests (3,197 passed, 136 ignored), doctests, the live simulation tests (89 passed; the one failure is the ngspice 44.2 pin), Clippy, licence checks, documentation counts and the release build. The installed binary hash is unchanged. The repairs PR #15 and then M1, PR #16, merged on 2026-11-06.

### M2 progress: TVS (done)

- **Kinds `tvs` and `tvs_bidirectional`** (`simulation/models/tvs.rs`). Inputs: `breakdown_voltage` at its test current, `clamping_voltage` at `peak_pulse_current_amps` (a new quantity and condition), optional `reverse_current` (checked, not fitted). A unidirectional part also takes `forward_voltage` and `junction_capacitance`; a bidirectional one takes neither.
- **The fit.** Two voltages fix the slope between them, not the knee, so `NBV = 1` and the series resistance carries the rest of the rise. If the clamp sits below the junction law, the resistance is zero and `NBV` takes the rise. Both cases are listed in `fit.assumed`.
- **Bidirectional** is two Zener-style cells in series through an internal node `M` (`DF A M`, `DB A M`, `DG K M`, `DH K M`). Each datasheet voltage is the breakdown cell's plus the other cell's forward drop, which is a plain silicon junction and is listed as assumed.
- **Shared code.** `diode.rs` now holds `breakdown_card`, which `zener` and `tvs` both call, and `check_voltage_at_current`. The grammar checker accepts `DG`/`DH`, and a marker `kind` of up to 24 characters.
- **Tests:** four in `tvs.rs`, one live test (`both_fitted_tvs_kinds_are_read_by_kicad_and_clamp_on_the_engine`) at the end of `simulation_live.rs`. Negative control: zeroing the series resistance in the card fails the live test with the clamp reading 15.25 V against 24 V.
- **Limits:** the breakdown voltage does not follow temperature; no pulse-width or thermal behaviour, so a long pulse clamps as a short one; a bidirectional part's capacitance is not modelled.

### M2 progress: MOSFET (done)

- **Kinds `n_mosfet`, `p_mosfet`** (`simulation/models/mosfet.rs`). Every value is a magnitude. Inputs: `gate_threshold_voltage` at a drain current, `on_resistance` at a gate voltage and current, two or more `drain_current` values (`gate_source_voltage_volts`, `drain_source_voltage_volts`), optional `forward_voltage` (the body diode) and `gate_resistance`.
- **The model** is a three-pin subcircuit `D G S` holding one `M1` and one `.model ... VDMOS` card, so KiCad's export never adds thermal nodes. The channel type is a bare `NCHAN` or `PCHAN` continuation line.
- **The law was checked against the engine, not assumed.** A scratch run of libngspice 45.2 matched `vgst = ksub ln(1 + exp((Vgs - VTO) / ksub))`, `beta = KP / (1 + THETA max(Vgs - VTO, 0))`, triode `beta Vds (vgst - Vds / 2)`, saturation `beta vgst^2 / 2`, to six digits, with `RD` in series. So the fit runs on the law (`Law` in `mosfet.rs`) and needs no engine round trip. The memory note's `(1 + lambda Vds)` term and `RS` are not used: lambda is 0 and all series resistance is `RD`.
- **A P-channel card needs a negative `VTO`.** With `PCHAN` and a positive `VTO` ngspice reads 393 A where the mirror of the N-channel part reads 78 A. The fit writes `-VTO` for `p_mosfet`, and the bench uses sources of the opposite sign.
- **The fit.** `fit::least_squares` (Levenberg-Marquardt, numeric Jacobian) fits `VTO`, `KP` and `THETA` to the log of the current at the threshold point and every transfer point; `RD` then follows from the on-resistance by inverting the law; three rounds. With more than three transfer values the middle one is held out. An on-resistance below the channel alone gives `RD = 0` and an `assumed` note; a current the channel cannot carry at the on-resistance's gate voltage is refused on that field.
- **Grammar.** `text.rs` accepts `M1 D G S <name>_m` and a `.model <name>_m VDMOS` card whose parameters come from `MOSFET_PARAMETERS`.
- **Tests:** eight in `mosfet.rs` (law against engine values, inversion, recovery, hold-out, P-channel mirror, grammar, two refusals), one in `fit.rs`, and `both_fitted_mosfet_kinds_are_checked_on_the_engine_and_attach_to_a_three_pin_part` in `simulation_live.rs`, which fits both kinds and assigns the N-channel one to `Q2` of the `class-d` demo. Negative controls: dropping the P-channel `VTO` sign, and not writing `RD`, each fail the live test (the second with 60.5 percent worst error).
- **Capacitances** (optional, but all or nothing for the curve): three or more `reverse_transfer_capacitance` values at drain voltages fit ngspice's gate-drain law (`GateDrain`: `s atan(a vgd) + y`, `CGDMAX`, `CGDMIN`, `A`) by least squares on the log, holding out the middle one when there are four or more. `input_capacitance` less the fitted Cgd gives `CGS`; `output_capacitance` less it gives the body diode's `CJO`, `VJ`, `M` through `fit_capacitance`. Ciss and Coss without Crss are refused. All three are measured on the bench at 1 MHz with Vgs = 0: Ciss as a series capacitance (so `RG` does not read it low), Crss through the gate's source, Coss through the drain's. Without a curve the result says in `fit.assumed` that ngspice's default capacitances stand.
- **Gate charge is checked, never fitted.** `gate_charge` (to a gate voltage) and `gate_drain_charge` run on a transient bench, `Probe::Charge` in `bench.rs`: a current source `Id` from the supply into the drain, a diode back to the supply, and a constant gate current sized to deliver the charge in 250 ns. Qg is the current times the time the gate reaches the stated `Vgs`; Qgd is the current times the time the drain takes to fall from 90 to 10 percent of `Vds`. Rows are `check_only`. Three traps, each hit and fixed: the bench's `reltol=1e-6` stalls a transient at the gate current's first edge (timestep 1e-21 s), so the fixture relaxes it (`.options reltol=1e-3 ...`); an unclamped gate charges to hundreds of volts and the body diode's solve then fails, so a diode clamps it two volts above the stated `Vgs`; and a 1 Tohm gate shunt beside that clamp diode (leakage `gmin`, also 1 Tohm) halves the gate's DC level, so the shunt is 1 Mohm. A hand integral of the fitted Cgd over the drain swing agreed with the engine's Qgd (21 nC).
- **Tests:** 14 in `mosfet.rs`, 2 in `bench.rs`, 1 in `fit.rs`, and the live test above, which now also fits the full capacitance curve and both charges for both polarities and asserts the two charge rows. Negative controls, each failing the live test: dropping the P-channel `VTO` sign, not writing `RD`, halving `CGS` (the Ciss rows fail), and halving the Qg probe (-49.9 percent).
- **Limits:** no avalanche (`BV`, `IBV`, `NBV` are in the grammar but never written, so a drain can rise above any rating), no temperature behaviour (one temperature, `TNOM` written), no reverse recovery (body diode `TT` is 0), `lambda` is 0 so the saturation current does not rise with drain voltage, and a value read at a different gate voltage than 0 V for the capacitances is not supported. Quasi-saturation (`RQ`, `VQ`) and `RS` are unused.
- **Left for M2:** nothing; see the BJT section below.

### M2 progress: BJT (done)

- **Kinds `npn`, `pnp`** (`simulation/models/bjt.rs`), every value a magnitude, a three-pin subcircuit `C B E` around one `Q1` and one Gummel-Poon card (`.model <name>_q NPN|PNP`; the grammar takes `BJT_PARAMETERS`).
- **The law was checked against the engine** (libngspice 45.2, scratch harness): with no Early voltage, `RB = RE = 0` and `NR = 1`, `Ic = (IF - IR)/qb - IR/BR` and `Ib = IF/BF + ISE(e^(Vbe/NE Vt) - 1) + IR/BR`, `qb = (1 + sqrt(1 + 4 IF/IKF))/2`, matched to 0.05 percent (the gap was a rounded Vt). So the fit runs on the law.
- **Inputs.** Three or more `current_gain` values (a curve; with four or more the middle one is held out) and one or more `base_emitter_voltage` at a collector current and voltage give `IS`, `NF`, `BF`, `IKF`, `ISE` by `fit::least_squares` (`NE` is fixed at 1.5; with one Vbe value `NF` is 1 and the result says so). Optional: `collector_emitter_saturation_voltage` at a collector and base current gives `BR` by bisection, or `RC` when it is above what `BR = 0.05` gives, or `BR = 200` and a note when it is below what the model reaches; `collector_base_capacitance` and `emitter_base_capacitance` at reverse voltages give `CJC/VJC/MJC` and `CJE/VJE/MJE` by `fit_capacitance`; `transition_frequency` gives `TF` from `1/(2 pi fT)` less the depletion capacitances over `gm` (ngspice's forward-bias linearisation at `FC = 0.5`).
- **Bench.** Gain: a base current of `Ic/hFE` is forced and `Ic/Ib` read, so it is the gain at that base current. Vbe: the base is grounded and the emitter current forced, so the emitter sits at `-Vbe`. Saturation: both currents forced, the collector voltage read. Cobo and Cibo: a reverse bias with an AC drive, the far pin open through 1 Gohm, `Probe::Capacitance`. fT: a unit AC base current at the biased point and `Probe::CurrentMagnitude` times the test frequency (a third of fT unless the caller gives one).
- **Tests:** eight in `bjt.rs` (the law against engine values, inverses, recovery, hold-out, saturation, grammar for both polarities, capacitances and fT, a duplicate), and `both_fitted_bipolar_kinds_are_checked_on_the_engine_and_attach_to_a_three_pin_part` in `simulation_live.rs`, which fits both polarities and assigns the NPN to `Q2` of the `amplifier-ac` demo. Negative controls, each failing the live test in the expected row: `TF` not written (fT), `BR` not written (saturation), the PNP junction bias unmirrored (all three capacitances).
- **Limits:** no Early voltage (the collector current does not rise with collector voltage), no base or emitter resistance, no temperature behaviour, the gain is checked at the base current the datasheet gain implies and not at exactly the datasheet collector current, fT is a small-signal estimate that ignores `RB`, and `ITF`/`XTF` are unused.

### M3 progress: behavioural ICs (op-amp, comparator, LDO, reference; merged, #53)

- **Kinds** `op_amp` and `comparator` (`simulation/models/amplifier.rs`, five pins `INP INN VCC VEE OUT`), and `ldo` and `voltage_reference` (`simulation/models/regulator.rs`, three pins `IN OUT COM`). Each is one subcircuit of behavioural lines in the fitted grammar; nothing refers to ground inside.
- **Grammar** (`text.rs`). A subcircuit may now hold linear `E` and `G` sources, `H` sources that sense one of the body's own `V` sources, `V` and `I` sources with a plain number, any `R`/`C`/`L`/`D` named by Konnect, and `B` lines whose value is `V=` or `I=` of an expression in a closed grammar: numbers as Konnect formats them, `v(node)` or `v(node,node)`, `+ - * /`, parentheses, and `min max sqrt tanh exp ln` with their right argument counts (`check_expression`). No word, string, brace, `$`, `#`, `;`, `i(...)` or `pow` can pass, and an expression is at most 4 KiB. Clamp diodes use the `<name>_c` model suffix. A port named `gnd` is refused (see the trap below).
- **Op-amp law.** A mid-supply node from `EMID mid VEE VCC VEE 0.5`. The input stage `BG` is `gm (Vd - Vos)` limited to `imax`, into `RA = 1 Mohm` and `CA`: `A0 = gm RA`, `GBW = gm / (2 pi CA)`, `SR = imax / CA`. Diodes clamp the gain node to each rail less the swing (`EH`/`VH`, `EL`/`VL`, referred to `mid` so the clamp current never reaches a supply pin). `EO` buffers it through `RO` and a sense source `VS`; `HS` and `BP` draw a sourcing output's current from `VCC`, so a sunk current returns through `VEE` (checked on the engine by hand: 2 mA sourced reads 2.5 mA at `VCC` and 0.5 mA at `VEE`, and the reverse when sinking). The clamp offsets are solved so each saturated output, with its datasheet load to mid-supply, reads its swing, the clamp diode's drop at the stage's current included.
- **Op-amp bench.** Open-loop gain on AC through a DC servo (1 Mohm from the output to `INN`, which a 1 GF capacitor grounds at every frequency checked, so the offset cannot saturate the output): the DC gain at a hundredth of the pole, the product as `|A| f` at `GBW / 10`, Bode values at their own frequencies (held out against the single pole), each on a fixture at its own supply and with its own load to mid-supply (a DC gain stated at a load fits the core's gain as `A (RL + Rout) / RL`), and the output resistance as the voltage an AC current into `OUT` makes. Offset, bias and supply current on followers, swings with the output driven to a rail, and the slew rate on a follower stepped across most of its swing, timed between 10 percent and the lesser of 70 percent and the point where the input stage leaves its limit.
- **Comparator law.** The same core with a DC gain of 1e6 and a stage `imax x / ((|x| + e)^(1-p) (|x| + v)^p)`, written with `exp` and `ln` whose arguments stay under 14. Hysteresis adds `(Vh / 2) tanh(v(a, mid) / 0.1)` to the input, so the trip points sit `Vh / 2` either side of the offset at any supply. The delay is `CA` times the integral of `da / I(a)` from the low clamp to the middle of the swing; one value fixes `CA` with `v` at its overdrive, two fit `v` too, three or more fit the sharpness `p` (0.25 to 1), and with four or more the middle one is held out. The trip points are read on a slow PWL triangle, a second transient run (`Bench.slow`), whose rate is set from the fitted law so the output lags its trip by under a thousandth of the hysteresis.
- **Regulator law.** `BO s COM` is the smooth lesser of `V0 + k (Vin - Vin0)` (less a stiff current-limit term) and `Vin - Rdo I`, each corner `(a + b - sqrt((a - b)^2 + e^2)) / 2`. `I` is the output current through `VS`, made a voltage by `HS`. `ROUT` after the source is the load regulation. `BI` draws the output current from `IN`, and `BQ` the quiescent current, `Iq Vin / sqrt(Vin^2 + 0.01)`. An LDO sinks only 1 uA a volt; a reference sinks through `ROUT`, to `COM`. `output_voltage` values fit `V0`, `k` and `ROUT` by least squares on the terms the values fix, holding out one when there are more; `dropout_voltage` gives `Rdo`; the current-limit setting is solved on the law so the engine reads the datasheet's current at the held output. Output checks run with `COM` lifted 1 V off ground, so a pin the engine tied to ground reads wrong.
- **Refused, with the field:** an op-amp without exactly one DC gain, or one of 160 or less (it reads as dB); a slew rate that leaves the input stage linear over under 1 mV against the product (a value in V/us); a swing beyond half the supply, or one the stage's current cannot reach through `RA`; two delays at one overdrive; `output_voltage` values that change input voltage and load current together, so line and load regulation cannot be told apart (the value that separates them is never the one held out); an output value with its input below it, read in dropout, or within 10 percent of the current limit; a load line that rises; a dropout no larger than the load line's drop; a current limit the dropout resistance could never pass, or read with the output at or above its setting.
- **Three traps found on the engine:** a hard `min`/`max` clamp on an output leaves Newton a flat region, and libngspice 45.2 failed every operating-point method on it (diode clamps and the smooth forms solve at once); ngspice reads a node named `gnd` as ground, even a subcircuit's port, which tied the first LDO's third pin to ground in the LM317 run while every bench passed, so the pin is `COM` and the grammar refuses `gnd`; and a `PULSE` with no width never falls, so a triangle is a `PWL`. A fourth cost time: a clamp diode goes on pushing current into the gain node while it turns off, which made every fitted delay read 1 to 2 percent short until the delay integral included it.
- **Tests:** 7 in `amplifier.rs`, 6 in `regulator.rs`, one in `fit.rs` for the rank test, a grammar test of 12 accepted and 34 refused behavioural lines in `text.rs`, and two live tests: `behavioural_amplifiers_are_checked_on_the_engine_and_a_fitted_op_amp_runs_a_filter` (fits both kinds, assigns the op-amp to `U1` of `sallen_key`, and the AC run meets the filter's formula: the -3 dB edge within 1e-3 and -6.02 dB at f0) and `behavioural_regulators_are_checked_on_the_engine_and_a_fitted_ldo_regulates_a_supply` (fits both kinds, assigns an LM317-valued `ldo` to `U1` of `LM317_power_supply` with `COM` on ADJ, and a 0.3 s transient holds OUT 1.25 V above ADJ and 9.14 V at the output). Both require a worst engine error under 1 percent; the runs read about 0.03 percent. Negative controls, each failing a live test: the `COM` pin named `GND` with the grammar's refusal off, the clamp current left out of the delay integral, the old fixed-period trip ramp, the hysteresis term left out of the model, and the slew read into the settling tail.
- **Limits:** a single pole (90-degree phase margin) and no second-stage dynamics, so loop stability needs the vendor model; no common-mode or supply rejection, input range, noise, offset current or temperature behaviour; the comparator output is push-pull only, its delay is checked on the rising edge with no load, and its DC gain is fixed; the regulators have no loop dynamics (a load step is answered at once), no PSRR, thermal shutdown, enable pin or reverse conduction, and the current limit is flat to a short (no foldback). The supply-pin current split of an op-amp under load is checked by hand only.
- **Left for M3, in #54 (open):** load switch, eFuse and ideal diode. They fit the regulator core: a pass element `Vin - Ron I` with a limit term (eFuse), an enable threshold and a soft-start ramp (load switch, which needs a capacitor in the law), and a reverse-current cut-off (ideal diode).

### M3 gate (macOS, 2026-11-08, first part)

Formatting, workspace tests (3,743 passed, 156 ignored), doctests (9 passed, 2 ignored), the live simulation tests (95 passed, none failed: no red baseline), Clippy with warnings denied, `cargo deny check licenses`, documentation counts (unchanged: no tool was added) and the release build. The installed binary hash is unchanged (`3de188bb775ab4ce...`). Dogfood by pipe, on a scratch copy of `demos/simulation`, with `get_installation_info` naming the fork binary and HEAD: an `op_amp` and an `ldo` written within 0.01 percent, attached to `sallen_key` `U1` and `LM317_power_supply` `U1`; the AC run's -3 dB edge read 1024.3 Hz (the formula gives 1024.3), the transient held the output at 9.134 V (9.138 V predicted), both read `as_drawn` with `fitted_model` for `U1`, and an output value in dropout was refused on its field. By hand on libngspice 45.2: the fitted LDO's input carries the load current plus the quiescent current, which leaves through `COM`. Not run: hosted CI, Windows and Linux, the KiCad dialog round trip and a run in KiCad's GUI simulator, which stay owner checks.

### M3 review follow-up (2026-11-09)

Three verified review findings, fixed in one change:

1. A regulator or reference fit whose `output_voltage` values change input voltage and load current together left three terms to two equations, and `fit::solve` returned an arbitrary exact answer. Such values are now refused on `characteristics`, with a message naming the values that would separate the two. The hold-out never takes the value that separates them, and `fit_output` also refuses a normal matrix that is not full rank on a relative test (`fit::independent`, unit diagonal, pivot under 1e-12). `fit::solve` is unchanged for its other callers.
2. An op-amp DC gain of 160 or less reads as dB and is refused, since there is no unit field to say otherwise. A slew rate that leaves the input stage linear over under 1 mV against the product is refused as a value in V/us.
3. An `open_loop_gain` stated at a load or a supply is now fitted and checked there: the core's gain is raised by the output resistance's share, and each gain value runs on a fixture at its own supply with its load to mid-supply. The product's own supply is honoured the same way. The live op-amp test now states its DC gain at a load.

Verified on macOS: formatting, workspace tests (3,748 passed, 156 ignored), doctests (9 passed, 2 ignored), Clippy with warnings denied, `cargo deny check licenses`, documentation counts (unchanged), and the live simulation tests (95 passed, none failed). Nine negative controls, one per new guard, each failed its test and was restored. Not run: the release build, a dogfood run by pipe, hosted CI, Windows and Linux.

### M2 gate (macOS, 2026-11-06)

Formatting, workspace tests (3,226 passed, 139 ignored), doctests (9 passed, 2 ignored), the live simulation tests (92 passed; the one failure is the ngspice 44.2 pin, a red baseline reported and not fixed), Clippy with warnings denied, `cargo deny check licenses`, documentation counts and the release build. The installed binary hash is unchanged. Not run: the dogfood run by pipe, hosted CI, Windows and Linux, and the KiCad dialog round trip, which stays an owner check.

**The ngspice 44.2 pin is fixed (merged as [#23](https://github.com/yuvaltal912-sys/Konnect/pull/23)).** `a_real_operating_point_runs_and_leaves_the_project_untouched` pinned the engine version to `44.2`, but KiCad ships its own libngspice, and 10.0.8 ships 45.2. The test now checks that a `MAJOR.MINOR` version of 44 or later is reported. With this fix the live suite has no red baseline.

## Simulation validation and stored results (merged as PR #15)

The owner requested reproduction and repair of the known simulation issues. Work was on `fix/simulation-validation-and-results`, merged as PR #15 on 2026-11-06; it is not installed. The original macOS baseline passed formatting, workspace tests, doctests, 79 live simulation tests, Clippy, license checks and the release build.

- **AC stimulus:** follow reachable subcircuit calls, with cycle protection. Unused definitions no longer satisfy `no_ac_stimulus`; the last AC magnitude assignment wins. Parameters and conditional branches still need engine evaluation.
- **Legacy model replacement:** remove and read back KiCad's nine recognized legacy simulation fields as well as modern `Sim.*`, returning removed values in `sim_fields_dropped`. Same-symbol replacement and lookalike custom properties stay unchanged. A real baseline reproduction using `Spice_*` and an external diode model returned about 0.396828 V after a diode-to-resistor replacement; the corrected divider is 2.5 V.
- **Stored results:** `run_status` remains the historical outcome. `result_files_present`, `result_files_error` and `without_results` disclose current status/raw-file availability without parsing every large raw file during listing.
- **IBIS provenance:** new run records compare original `.ibs` inputs rather than disposable KiCad-generated models, with before/after-export checks. Unprovable origins and legacy IBIS records report unknown (`null`), not a fabricated unchanged result. A changed original model still reports a change.
- **Numerical interpretation:** successful numeric AC margins carry `sampling_validation: "unverified"`, `crossing_selection: "first"` and an actionable warning. Subcircuit power identifies `power_scope: "exposed_subcircuit_pins"` and `total_device_power_verified: false`; summaries, measurements, operating-point rows and plots disclose that boundary. These safeguards preserve the engine's values; they do not solve sampling aliasing or recover hidden power.

Regression tests are in `simulation/element.rs`, `simulation/ac.rs`, `simulation/signals.rs`, `simulation/runs/inputs.rs`, `tools/simulation/stored.rs`, `tools/sim_field_placement_tests.rs`, and `konnect/tests/simulation_live.rs`. The tests include baseline failures, independent circuit expectations, and negative controls. Final macOS validation passed: formatting, the full workspace library/integration suite, doctests, all 84 ignored live simulation tests, Clippy with warnings denied, license checks, documentation-count checks, and the release build. All 16 targeted negative controls failed as intended and were restored. The installed binary hash remains `3de188bb775ab4ce5748ea44b94cb098c86ed5d32135e70f4c2c3817c3a4ac65`. CodeRabbit could not authenticate; the owner chose local validation only. Hosted CI and Windows/Linux were not run.

### Local code-review repairs

The owner requested all six review fixes and both improvements on the same local branch, preserving the work above. On 2026-11-05 the owner had them committed on that branch, so the datasheet-model work can build on them. It merged as PR #15 on 2026-11-06 and is not installed.

- **Unit replacement:** edit actual `unit` nodes with the existing string-aware scanner. Quoted custom-property names and values containing `(unit ...)` remain unchanged; both the placed symbol and its hierarchical instance receive the new unit.
- **Run retention:** young folders without `run.json` are potentially active and are excluded from deletion candidates. They may temporarily exceed the 20-run limit. Completed runs and abandoned record-less folders older than one hour remain eligible, with abandoned runs removed first.
- **Model provenance:** retain the logical model path and its resolved target, so retargeting a file or parent-directory symlink is a change. Ordinary-model fingerprints come from the exact bytes consumed by flattening, including the source-evidence hash. IBIS originals still require before/after-export evidence. Older provenance that retained only canonical paths cannot prove unchanged inputs and reads unknown.
- **Listing cost:** reuse successful bounded model fingerprints within one listing request. Each record still enforces its own byte limit, and a later request reads fresh content.
- **AC reachability:** identify nested subcircuits by lexical scope, so a source in an unused parent's local definition cannot excite another parent's same-named definition. The separate connectivity summary remains conservative; parameter expressions and conditional branches still need engine evaluation.
- **HTTP origins:** parse the URI authority so IPv6 loopback origins are accepted alongside IPv4 and `localhost`. Foreign hosts, credentials and non-origin URI forms remain refused.
- **Protocol tests:** default subprocesses own temporary configuration, home and state directories. Explicit configuration fixtures remain supported, and inherited host settings cannot change the default tool-loading assertions.

The review baseline had two `protocol_stdio` failures caused by inherited user configuration; both passed when the same test binary ran in an isolated environment. After the repairs, the ordinary macOS gate passed: formatting; workspace library/integration tests (3,145 passed, 130 ignored); doctests (9 passed, 2 ignored); all 84 ignored live simulation tests; Clippy with warnings denied; license checks; and the release build. The protocol tests also passed under deliberately hostile external configuration and state settings. Focused regressions were observed failing against the old or disabled fixes, including model provenance/cache controls and a bounded-reader file-growth control. The nested-scope AC cases were also checked through the rebuilt fork against ngspice 44.2 on scratch circuits. The installed binary hash remains `3de188bb775ab4ce5748ea44b94cb098c86ed5d32135e70f4c2c3817c3a4ac65`. Hosted CI, Windows/Linux, the separate viewer and Python plugin were not run.

## Now: the plan's v1 is merged

PR 1 to 11 and the placement fix are merged, which finishes the plan's v1 on macOS. The `replace_component` fix is merged too. Ask the owner before starting any follow-up. The "After PR 11" table lists what is left.

**Next in simulation:** fix #54's four verified review findings, then merge it; this waits while creation work has priority. After M3, the next PR waits for the owner's choice. Recommended: M5 (vendor library import, with raw-ngspice safety vetting) before M4 (a standalone SPICE netlist), because vendor models are what fitted models cannot replace.

### What the placement fix changed that later work needs to know

- **One helper does it.** `SymbolMetadata.sim_fields` (in `library.rs`) reads the embedded library symbol's direct `Sim.*` properties in library order, first of a repeated name. `PlacementFields.sim_fields` carries them, and `sch_components::push_sim_fields` writes them. `place_one_component` is the single writer for `add_schematic_component`, `batch_place_components` and `apply_template`; `add_power_symbol` builds its own symbol and calls the same helper. A new placer must call it too.
- **The copy is exact and has no argument.** `ki_*` fields stay in the library. A caller changes a `Sim.*` value afterwards with `edit_schematic_component`.
- **The post-write readback checks the copied fields** (they are in `ComponentTargetUnit.fields`), so a placement that loses one is reported, not passed.
- **`replace_component` is the other half, and has its own section below.** It is not a placement, so this fix left it alone; it keeps no old fields any more (see "What the `replace_component` fix changed"). Parts placed before the placement fix still read `unmodelled_symbols` with `copy_library_sim_fields`.
- **Limits left:** fields are read from the schematic's embedded library copy (as Footprint is), so a stale embedded copy places stale fields until `update_symbols_from_library`. Creation PR 6 (#30), consolidated in the creation integration, adds inherited library inclusion attributes including `exclude_from_sim`. Legacy simulation fields still remain outside this modern `Sim.*` placement copy. `Sim_Library`/`Sim_Name` are not aliases recognized by KiCad 10.0.1. The readback compares against the same list the writer used, so it proves the write and not the extraction; the extraction is tested on its own in `library.rs`.
- **Tests:** `tools/sim_field_placement_tests.rs` (one test per path, with a quoted `Sim.Params` value), a template test in `templates.rs`, and two ignored live tests at the end of `simulation_live.rs`: a diode placed by `add_schematic_component` and another by `batch_place_components` simulate, and the same circuit with the instance `Sim.*` fields cut out is refused as `unmodelled_symbols`. Evidence is E110 and E111 in the plan. There is no "X4" in the plan (its open questions are Q1 to Q17); E45 and E74 are the evidence behind this row.

### What the `replace_component` fix changed

- **`replace_component` swaps the instance's `Sim.*` fields with its `lib_id`.** After the new definition is embedded, `plan_sim_field_swap` (in `sch_components.rs`) takes `symbol_metadata(...).sim_fields` of the new symbol and, in every placed unit, deletes each `Sim.*` property and inserts the new ones, hidden, after the unit's last property. The text edits are applied before the junction pass and the write, like the `lib_id` edits.
- **Decision: the old part's `Sim.*` go, hand-edited ones included.** A tuned `Sim.Params` describes the part that is leaving. The result lists `sim_fields_dropped` and `sim_fields_added` (`[{name, value}]`) so a caller can restore one. A field the new symbol repeats exactly is in neither list. Modern `Sim.*` and the nine KiCad-recognized legacy simulation names count. `Simulation`, `sim.device`, `Spice_Notes`, `Sim_Library` and `Sim_Name` remain unrelated properties.
- **Replacing a symbol with itself changes no Sim field.** Nothing is being swapped, so a hand edit survives. A unit that already carries exactly the new fields, in any layout, is left as it is.
- **The readback checks both ways.** The bound intent (`ComponentTargetUnit.fields`) drops the old `Sim.*` and holds the new, so a missing new field is reported; `unit_with_stale_sim_field` reports an old one that survived.
- **It does not rename the part, and it does not touch Value or Footprint.** A diode `D1` replaced by `Device:R` has no Sim fields (the library's `R` has none) and still reads `D1`, so KiCad cannot infer a model and readiness says `unmodelled_symbols`. That is loud, where it used to run as a diode. Renaming it to `R2` with `edit_schematic_component` makes KiCad infer a resistor from the prefix and the value.
- **Limits left:** the new fields come from the schematic's embedded library copy, as in the placement fix. The current follow-up also removes recognized `Spice_*` and KiCad 7-development `Sim_*` aliases; unsupported lookalikes are preserved. The reproduced failure was an old `Spice_*` model resurrecting when the new implicit part has no modern assignment. `Sim.Enable` is a `Sim.*` field, so a part switched off in the simulation is switched back on by a replacement (it appears in `sim_fields_dropped`). The insert indent is whitespace only; a block written on one line gets the default.
- **Tests:** 16 in `tools/sim_field_placement_tests.rs` (both directions, replacement rather than merge, a hand edit, the same symbol, CRLF and tabs, two units that disagree, lookalike names, a one-line block, identical fields in another layout, the stale-field check) and three ignored live tests at the end of `simulation_live.rs`. Evidence is E112 and E113 in the plan.

### What the false-refusal fix changed

- **`.ic` and `.nodeset` inside a `.subckt` or `.macro` are judged against that body, not against the circuit's nets.** `read_definitions` in `element.rs` records initial-condition nodes at depth 0 only. `read` records the others per subcircuit, and `Circuit::unknown_subcircuit_initial_nodes` accepts a node that is a pin, a body element's node, ground, a `.global`, or `xN.name`, and does not judge a body holding a line the reader could not place. A typo inside a subcircuit is still refused (testbench) or warned `unknown_initial_node` (schematic). Before, every such card was flagged, the correct ones too.
- **A file the testbench includes now counts through the nodes of its top-level elements.** `read` adds each such element's nodes to `word_cards`. A value, a model name, a parameter, an element name, a model line, a subcircuit body, a library the schematic includes, and an element inside an `.if` block do not count. A typo inside the included file is still refused.
- **Left as limits:** the testbench's own lines still count every word on a card (as before), and an included element inside an `.if` is never counted, so a circuit that needs one stays refused. The same file included twice passes `testbench_nodes`, and `element_names` reports the duplicate.
- **Left as limits, in the subcircuit `.ic` check** (every such card was flagged before, so these are no worse than before, but they are gaps): an element in a dead `.if` inside the body counts as a body node; two definitions of one subcircuit name are merged; and any `xN.name` is accepted without checking that the body has that instance, as at the top level.
- `flatten.rs` is unchanged. The flat deck is the same, so this is only a change in what preflight judges.
- **Run against the engine by the PR's reviewer, not by the tests:** libngspice 44.2 applies a `.ic` written inside a subcircuit (`v(x1.int)` read 1 with it, 0 without) and logs "IC on non-existent node" for a mistyped one.

### What PR 11 changed that later work needs to know

- **`plot_simulation_run` draws a stored run to `plot-<12 hex>.svg` in the run's folder.** `simulation/plot.rs` is the renderer (no plotting crate, no `unsafe`, nothing about runs), `tools/simulation/plots.rs` gathers each analysis's data and writes the file. A new analysis kind must add its arm there, as it must in `Analysis::record`, `from_record` and `Evaluation`.
- **`stored::open` and `stored::read_plot` are the way to a stored run.** They were cut out of `measure_simulation_run`, which now uses them too, in the same order of checks. Reach a run only through them (`runs::locate`, then `runs::read_record`).
- **What is drawn:** a panel for each unit for a transient or one DC sweep; a Bode plot (gain in dB, unwrapped phase) on a log frequency axis for AC; for two sweeps a panel for each signal (four at most) with a curve for each step of the second, eight at most, thinned evenly with the first and last kept. An operating point is refused. A partial run is drawn only as far as it got, because stretching the axis to the requested range made a run that covered 1 percent a blank plot.
- **The banner is drawn first** and has a line each for `PARTIAL RUN`, `MODIFIED CIRCUIT` and `SCHEMATIC CHANGED` (`inputs_changed_since_run: true`). The last was not in the plan.
- **Size is bounded by pixels, not by points.** At most two points a pixel column a trace, keeping the lowest and highest. 281,020 points on two signals is 25,710 bytes. A trace of 40 points or fewer also gets a mark at each sample.
- **Text is untrusted.** Net names and the file name are escaped, cut to 64 characters and stripped of controls and reordering characters before they reach a text node. Never put one in an attribute. The tests parse the file with a strict XML parser (`usvg::roxmltree`, already a dependency).
- **Files:** the same request replaces its file, at most 16 plots stay in a run folder, and only a plain file named exactly `plot-<12 hex>.svg` is ever replaced or removed. A link where the plot goes is `run_store_failed`.
- **`signals_plotted` is a list** of the signals as written. The plan did not say whether it was a count.
- **The SVG is not a PNG.** `konnect_render::svg_to_png` still refuses `<text>`, and the skill says the file is for a browser or an editor.
- **Phase on the plot is the phase margins use,** so the same aliasing limits apply, and a `phase_unreliable` warning and a note in the picture say where it stops being trustworthy.

### What PR 10 changed that the next PR needs to know

- **A finished run keeps `run.json`.** It is what `list_simulation_runs` and `measure_simulation_run` read: the schematic path, the analysis as run (`Analysis::record`, with a DC sweep's element and unit), `run_status`, `stopped`, `simulated_circuit`, `coverage_ratio`, `inputs_hash`, the inlined model files, and the net and element tables that turn `v(/NET)` and `i(REF)` into the engine's names. Only runs with data are recorded. A failed run, and any run from PR 6 to 9, has none: it is counted as `without_record` and not listed, and `measure_simulation_run` says `run_not_found`. PR 11 should read the same record, through `runs::read_record`.
- **A stored measurement is the live one.** `Evaluation` (in `tools/simulation.rs`) holds the signal lookup and the per-analysis gathering that `RunEnd` used to hold, and both a live run and a stored one go through it. `Wanted::for_analysis` is the argument check for a known analysis. A new analysis kind must add its arm in `Analysis::record`, `from_record` and `Evaluation`, or stored runs will not read it.
- **`runs::locate` takes an id and nothing else.** It checks the exact shape first, builds `<state>/simulation/<first 12>/<id>` and refuses a link (`symlink_metadata`). `list` and `read_record` never follow a link either. PR 11 must reach a run only through `locate`.
- **`inputs_changed_since_run`** is the hash of the saved sheets and `.kicad_pro` taken now (`CurrentSources`, a snapshot and `hash_sources`, no export) with the run's inlined model files, against the run's `inputs_hash`. `null` when the sheets cannot be copied. A model file that has gone is a change.
- **`results.csv` is written for every run** (`export.rs`, 10 million values at most, then `csv_path: null` with a `csv_not_written` warning). The scale of a DC sweep is renamed for what it varies, and a second sweep gets a column of its own, rebuilt from the point number. Names that a spreadsheet would run as a formula get a leading `'`.
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

## After PR 11

The plan's v1 is PR 1 to 11 and the placement fix, all merged. What is left after them:

| Work | Outcome |
|---|---|
| Two-sweep slices | An `outer_step` selection on `measure_simulation_run`, if the owner wants it |
| PNG and inline images | Need a small vector font for labels, with its licence checked, and `outcome::attach` reading the JSON block first |
| Margin runs at double density | `run_simulation` doing the second AC run itself |

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
| `crates/konnect-core/src/simulation/plot.rs` | The SVG renderer: layout, ticks, envelopes, escaping. Knows nothing of runs |
| `crates/konnect-core/src/tools/simulation/plots.rs` | `plot_simulation_run`: what each analysis draws, the banner, the file |
| `crates/konnect-core/src/simulation/signals.rs`, `measure.rs`, `ac.rs` | Signal grammar and lookup (real and complex), measurements on any axis, and AC gain, phase and margins |
| `crates/konnect-core/src/tools/simulation/stored.rs` | `list_simulation_runs` and `measure_simulation_run`, and `open` and `read_plot`, which every reader of a stored run goes through |
| `crates/konnect-core/src/tools/simulation/sweeps.rs` | The `ac` and `dc_sweep` analyses: arguments, deck line, target lookup, coverage |
| `crates/konnect-core/src/simulation/worker_client.rs`, `protocol.rs` | Worker supervision and its contract |
| `crates/konnect/src/sim_worker.rs` | The only FFI. `konnect-core` stays free of `unsafe` |
| `crates/konnect/tests/simulation_live.rs` | Ignored tests that need KiCad |
| `crates/konnect-core/tests/fixtures/simulation/` | Real KiCad and ngspice fixtures, with provenance. `soft_start.sp` and its reference script stand in for E31, `protection/` and its `reference.py` for E56, and `ac_loop.sp` and its reference script for E29 |
| `crates/konnect/assets/skills/kicad-simulation/SKILL.md` | The skill, including the protection stand-ins and modelling parts from datasheets |
| `crates/konnect-core/src/simulation/models/` | Fits to datasheet values (`diode.rs`, `passive.rs`, `tvs.rs`, `mosfet.rs`, `bjt.rs`, and the behavioural ICs in `amplifier.rs` and `regulator.rs`), the fitted-file grammar and its strict check (`text.rs`), and bench fixtures and probes (`bench.rs`) |
| `crates/konnect-core/src/tools/simulation_models.rs` | `create_simulation_model` and `assign_simulation_model` |
| `crates/konnect-core/src/tools/simulation/bench.rs`, `assignment.rs`, `parts.rs` | The engine bench, the export check before an assignment, and readiness's `parts_needing_models` |

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
- ngspice reads a node named `gnd` as ground, a subcircuit's port included. A bench whose ground pin sits at ground cannot see it; lift the pin.
- A `PULSE` with zero width never falls on libngspice 45.2. Write a triangle as a `PWL`.
- A hard `min`/`max` clamp in a `B` source can fail every operating-point method. Clamp with diodes, or a smooth form.

## Known open items

- **Include-search provenance:** stored checks compare the logical paths selected by the run. They do not replay the include search order, so adding a formerly absent, higher-priority include candidate can change a future run without changing the selected file recorded by the old run. A later fix needs to retain and recheck the candidate prefix from `Flattener::search_path`.
- A phase margin or gain margin is read from the first crossing only, and a loop gain with a low-frequency phase of exactly -180 degrees depends on the sign of rounding in the imaginary part. The result shows `start_phase_degrees` so the reader can see it. A gain margin from a start within 0.001 degree of -180 reads `starts_on_level`.
- AC stimulus reachability is repaired in the current follow-up. Parameter expressions and `.if` branches are not evaluated by this structural guard, so they can still require engine diagnosis.
- **The undersampling tests are a best effort.** Four review rounds each found another edge of them. Left as limits: a deep notch below crossover is refused at sensible densities (a 50 dB notch at 300 Hz needed 800 points a decade), and an all-pass or long delay aliases unseen. The dependable control is two runs at double the density. If a later PR touches this, consider making `run_simulation` do that second run itself when a margin is asked for.
- **Stored runs, limits left in PR 10:** new records in the current follow-up track proven original IBIS inputs; older IBIS records cannot establish model provenance and read `null`. Runs from PR 6 to 9 have no record, so they are counted as `without_record` and cannot be measured. A stored measurement on a partial run counts as failed, as a live one does, though its number is exact. The prune-race branch of `measure_simulation_run` (`or_not_found`) is tested through the function and not through a real race. Listings now disclose status/raw-file presence; they do not parse the whole raw payload, and a later read can still fail after a concurrent deletion.
- **PR 11's review:** four rounds by independent agents, the last finding nothing serious, and 130 source breaks of which 123 failed the tests (E108, E109). The round 4 fixes were not reviewed again. Layout is checked by a geometry test that holds every text off every other text, off the banner's edge and out of the plot frames at widths from 200 to 1600, using an estimate of 0.65 em a character, not by pixels.
- **Plots, limits left in PR 11:** text is placed by an estimate (0.6 em a character), so an unusually wide font could run a label past its margin, and nothing was checked in a viewer other than Chromium's. A linear AC sweep is drawn on a log axis and bunches at the high end. A two-sweep run whose first sweep has one point draws a mark for each step and no curve. A run's plots are not listed anywhere: `path` is the only way to find one. The picture was read by eye on one run of each analysis (E107) and by test for structure; no test compares pixels.
- A DC sweep of one point (start equals stop), or an AC sweep that holds one point, reads `complete` with `outside_data` for every window measurement. An AC sweep with under two points is refused; a DC one is not.
- With two sweeps `signals` summarise the whole grid and measurements are refused, live and stored. A slice per outer value needs a result shape of its own, and PR 10 left it out (see above). `results.csv` has the grid.
- A `bandwidth_3db` measured from the lowest frequency on a band-pass reads the first fall below that level, which can be far above the band. The result shows `reference_gain_db`, and `reference_level: "peak"` gives the band's own edges.
- `bg_halt` stops an AC sweep at the size limit and at the budget: a 1 MB limit halted a 90,005-point sweep at 34,407 points. `.op` still has no pause test.
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
