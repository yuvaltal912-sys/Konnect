# SPICE simulation for Konnect

Planning document, kept current as the PR series lands. PRs 1 to 3 are implemented.

Evidence base: KiCad 10.0.1 on macOS arm64 with its bundled ngspice-44.2; Konnect `main` at `c7e87cd8a`; KiCad source at tag `10.0.1`, commit `2db9e5a72b7c`; ngspice source at master, 44.2 and 42. Every cited KiCad file was diffed between that tag and the `10.0` branch [verified: run E71].

Review status: the first draft went through one adversarial review by four independent reviewers. Later revisions were checked by experiment, not by a second review.

Tags: `[verified: run]` was executed here. `[verified: source]` was read in source but not executed. `[assumed]` is unconfirmed and listed in section 5. E numbers point to Appendix B.

## 1. Summary

### Problem

An assistant driving KiCad through Konnect can draw a circuit but cannot check how it behaves. KiCad has a simulator, but it is reachable only from the GUI. `kicad-cli sch` offers `erc`, `export` and `upgrade`, and KiCad's IPC API has no simulation commands [verified: run].

### Proposal

Add an on-demand `simulation` toolset that gives an assistant what KiCad's simulator does, in four stages:

1. **Snapshot and export.** Konnect copies the saved schematic into a private run folder and runs KiCad's own exporter there. Nothing touches the user's project, and there is no second netlister.
2. **Preflight.** Pure Rust inlines everything the simulator will read into one vetted deck, then refuses any deck that cannot give a trustworthy result.
3. **Run.** A hidden `konnect sim-worker` subprocess, one per run, loads the libngspice that KiCad already installed. It is the same engine KiCad's simulator uses.
4. **Report.** Pure Rust parses the raw results file, computes time-weighted measurements and returns compact JSON. Full data stays on disk and is returned by path.

### Why this shape

| Finding | Consequence |
|---|---|
| Exporting a schematic with unmodelled parts exits 0 with empty stderr and writes lines like `Q2 __Q2` and `J1 __J1`, which SPICE reads by first letter [verified: run] | A successful export proves nothing, so preflight is required |
| `ngSpice_Circ` and `run` both return 0 when the circuit failed to parse or the analysis aborted [verified: run] | Return codes are not evidence. Use `sim_status`, point counts and coverage |
| A circuit with no node 0 returns believable wrong voltages, nets differing only by case merge silently, and an AC run with no AC source returns all zeros with success [verified: run] | Preflight checks ground, net names and stimulus |
| `.tran 1n 10` segfaults the host, a missing include leaves the library unable to recover, and a hard-edged behavioural model ran over 2 minutes at 500 MB [verified: run] | The engine runs in a separate single-use process with limits |
| Decks can run commands while loading, before any `run`: from `.control` blocks, `*#` comment-style lines, a `.title *ng_script` line, and files included through forms like `.incx`. An ordinary `.model` line can make the engine load a shared library. KiCad's exporter passes such lines through from schematic text [verified: run] | The engine reads only one flattened, vetted deck, and the two code models able to start programs or load libraries are refused |
| KiCad installs the library, not the `ngspice` program, on nearly every platform [verified: run on macOS; verified: source elsewhere] | Loading the library avoids an extra install where it is reachable |

### What v1 covers

Operating point, transient, AC and DC sweep; KiCad's measurements plus power and stability ones; protection scenarios through behavioural stand-ins; stored runs; SVG plots; a workflow skill.

v1 works on schematics only: no PCB data, layout parasitics or board tools. Digital code models are on, except the two that can start programs or load libraries.

It does not cover noise, pole-zero, transfer function, sensitivity, FFT, S-parameters, tuners or corners. Those follow as separate PRs.

### Install needs

| Platform | Needs | Status |
|---|---|---|
| macOS | Nothing beyond KiCad | [verified: run] |
| Windows | Nothing beyond KiCad, if `ngspice.dll` and code models ship as the build scripts suggest | [assumed] |
| Fedora, Arch | Nothing beyond KiCad | [verified: source] |
| Debian, Ubuntu | The `ngspice` package, for code models. `libngspice0` alone runs plain SPICE decks | [verified: source] |
| Flatpak, AppImage | A host `libngspice` set through config, because the library is inside the sandbox or image. Host loading of the Flatpak copy is untested | [assumed] |

### Rejected alternatives

| Alternative | Why not |
|---|---|
| `ngspice` CLI as the primary engine | Almost no KiCad install ships it, so most users need an extra install [verified: source]. It has real strengths: batch mode streams rows to the raw file, has `-n` to skip `.spiceinit`, and reports failure by exit status [verified: source]. The deck and raw-file contract keeps it open as a later fallback for hosts where the library cannot be reached |
| Loading libngspice inside the MCP server | A crash would take the server down [verified: run]. KiCad needs crash handlers for this [verified: source] |
| Linking or bundling ngspice | Adds redistribution duties for LGPL, MPL and GPL parts [verified: source] |
| ngspice `.meas` for measurements | Text output only, and a failed measure leaves `run` returning 0 [verified: run] |
| Running `kicad-cli` on the live project | `kicad-cli` takes a project lock on every run, and a kill strands it in the project [verified: run]. Konnect's viewer also records, on Windows, a nonzero exit after a good export with a foreign lock, and KiCad saves failing during export [verified: source]. On macOS every lock state tested still exited 0 [verified: run] |
| Keeping `.include` lines and scanning files separately | More paths to resolve, and ngspice's own lookup differs from any scanner [verified: source]. A single flat deck gives one artifact to check |

### Acceptance criteria

- `load_toolset("simulation")` exposes the tools, and the starter kit is unchanged.
- On a stock macOS KiCad 10 install, simulation works with nothing else installed. Windows has the same target, pending confirmation.
- A worker crash leaves the MCP server running and returns a typed error.
- A run that failed, aborted or stopped early never reads as complete.
- A `.control` block never reaches the engine. The worker cannot start a process on macOS or Windows.
- Any result from a circuit with excluded or added parts says so in its body.
- Short circuit, I²t fuse, eFuse limit, undervoltage lockout and reversed battery scenarios have working stand-ins and live tests.
- A typical result is under 2 KB of JSON, with full data on disk.

### Architectural fit

This extends an existing KiCad capability, the `kicad-cli` netlist exporter, and orchestrates an external engine through a bounded contract, the route tracker #590 describes for Freerouting. Whether ngspice qualifies as an accepted engine would need maintainer review [assumed].

It does reimplement some KiCad behaviour, each piece for a stated reason:

| Duplicated behaviour | Why | Retired when |
|---|---|---|
| Net-name conversion | The SPICE export has no pin-to-net map, so the KiCad-format netlist must be mapped into SPICE names | KiCad exports that map |
| Recognising `REF __REF` lines | KiCad reports no error for unmodelled symbols | KiCad reports them |
| Engine setup sequence | KiCad's setup is GUI-side and absent from exported netlists | `kicad-cli` gains a simulate command |

### Where this lands

`mixelpixx/Konnect` is archived and read-only, so no issue or PR can go there. Its successor, Konnect-EDA, is commercial and closed source, so it can take a feature request but not this code [verified: run]. This work lands in an AGPL-3.0 fork at `c7e87cd8a`, and this summary is written for that fork's tracker.

## 2. Toolset

**Name:** `simulation`. **Category:** `simulation` (new). On-demand, never in `STARTER_KIT`.

### 2.1 Coverage of KiCad's simulator

| KiCad simulator feature | Konnect coverage | Arrives |
|---|---|---|
| DC operating point | `run_simulation`, kind `operating_point` | PR 6 |
| Transient | kind `transient` | PR 7 |
| AC small-signal: decade, octave, linear | kind `ac` with `scale` | PR 9 |
| DC sweep: V, I, R, TEMP, optional second source | kind `dc_sweep`, up to two sweeps | PR 9 |
| Probing signals | `signals` | PR 7 |
| Measurements: min, max, average, RMS, peak-to-peak, time or frequency of min and max, integral | `measurements`. `min` and `max` return their location | PR 7 |
| Show netlist | `artifacts.deck_path` | PR 6 |
| Export data as CSV | `artifacts.csv_path` | PR 10 |
| Export plot | `plot_simulation_run`: SVG in v1, PNG later | PR 11 |
| User-defined signals | `v(a,b)` and `p(ref)` in v1; general expressions later | PR 7, later |
| Assigning `Sim.*` models | Existing `edit_schematic_component`, plus readiness hints | existing |
| Noise, pole-zero, FFT, S-parameters | One PR per family after v1. Noise, pole-zero and FFT ran on ngspice-44.2 [verified: run E35]. S-parameters [assumed] | later |
| Transfer function, sensitivity, distortion | KiCad reaches these only through its Custom page [verified: source E34]. First two ran on ngspice-44.2 [verified: run E35]; add on demand. Distortion untested | later |
| Per-signal Fourier | With FFT | later |
| Tuners, single and multi-run | `value_overrides`, then sweeps and corners | later |
| Workbook (`.wbk`) | Read-only import of stored analyses | later |
| Custom page | Free-form analysis commands are **not** accepted. `spice_lines` takes elements and models only | none |
| Digital models | On, except `d_process` and `d_cosim`. Event-data output comes later. See D2 | PR 6, later |
| Operating-point annotation on the schematic, zoom, cursors, paste plot | Out of scope: GUI only | none |

### 2.2 Pipeline rules

**Snapshot and export.** `cli::export_spice_netlist` copies the root sheet, its sub-sheets and the `.kicad_pro` into `<run_dir>/src/` by plain read and write, porting the viewer's `snapshot_tree` [verified: source E48]. Project-relative `Sim.Library` values are rewritten in the copy to live absolute paths, so KiCad's resolver finds them. Across KiCad's 18 simulation demos this gave the same netlist lines as a full project copy, byte for byte on 17 and reordered on one. Without the rewrite, 15 of the 18 exit 2. No `sym-lib-table` is needed [verified: run E65]. A sheet outside the project is refused. The KiCad-format netlist is exported first and the SPICE netlist second, with stdin null and `kill_on_drop`, and flattening follows at once, because IBIS models include a cache file in KiCad's per-user folder that a later SPICE export from any project can wipe [verified: run E66]. Readiness exports are serialised within one server for the same reason. Another SPICE export, from `generate_netlist` or a second Konnect process, can still wipe the cache first; that fails closed as `model_library_missing`, and retrying clears it. Any nonzero exit is then a real failure, a timeout may kill the child, both exports read one state, and no lock or file handle touches the project: a kill strands its lock only inside the run directory [verified: run E67]. The artifacts must be non-empty, and exit 2 still writes a file, so a file alone proves nothing [verified: run E65].

**Run directory.** `<state root>/simulation/<project hash>/<run_id>/`. `state root` is `KONNECT_STATE_DIR`, else `dirs::data_local_dir()/konnect`, exposed through one shared helper [verified: source E60]. It sits outside the project because raw files can reach hundreds of MB and projects may be in synced folders. `project hash` is SHA-256 of the canonical path, truncated to 12 hex. `run_id` is `<project hash>-<yyyymmddThhmmss>-<4 hex>`, so lookup needs no scan. Schemas enforce the pattern and the handler re-checks it, as `validate_transaction_id` does [verified: source E60]. The newest 20 runs per project are kept, from the first PR that creates runs.

**Flattening.** Konnect reads every file the deck references and inlines it into one `deck.sp`. The engine opens no model file. That removes path quoting, nested-include lookup, and every difference between Konnect's path logic and ngspice's. Lines are matched after trimming whitespace, ignoring case, by prefix, because that is how ngspice matches them [verified: run E19, E52].

| Line | From netlist or schematic text | From a model file | From `spice_lines` |
|---|---|---|---|
| `.control` to `.endc` block | Removed, reported in `dropped_directives` | Refused | Refused |
| Comment: a line starting `*`, `$`, `#`, `;` or `//`, including `*#` and `*ng_script`, or starting one of `=[]?()&%"!:,`, as a vendor banner does | Removed | Removed | Removed |
| `.title` | Removed | Removed | Refused |
| Analysis directive: `.ac`, `.dc`, `.tran`, `.op`, `.noise`, `.pz`, `.tf`, `.sens`, `.sens2`, `.disto`, `.sp`, `.pss`, `.hb`, `.four*` | Removed, reported | Removed, reported | Refused |
| `.end` | Refused unless it is the last card, because ngspice would lose the rest | Removed | Refused |
| `.end` glued to other text, such as `.end,x` | Refused | Refused | Refused |
| A dot card ngspice does not act on | Removed, reported as `unknown directive` | Removed, reported | Refused |
| A line ending in `\\` | Refused | Refused | Refused |
| `.inc*`, `.lib*` | Inlined | Inlined | Inlined |

Why these choices:

- Schematic text keeps working: 5 of KiCad's 18 simulation demos have a `.control` block, mostly housekeeping, sharing a text box with `.options` lines those circuits need [verified: source E58]. Removing the block is as safe as refusing it, because the engine never sees it. No person placed such a block in a downloaded model file, so there it is refused.
- `*#` lines execute as commands [verified: run E20], but `*####` banners are common in real libraries [verified: run E20], so they are removed rather than refused.
- Konnect writes one fixed `.title` as line 1, because `.title *ng_script` switches the whole deck into command mode [verified: run E51], and KiCad passes that line through from schematic text [verified: run E68].
- A `.end` in schematic text lands before the elements, so left in place it would cut the circuit off [verified: run E68].
- KiCad passes a whole text box if any line in it qualifies, so prose beside a directive reaches the deck and fails at parse. A `*#` line or `.measure` alone in a box does not pass [verified: run E68].

A final pass runs on the exact text the engine receives, and states what a line must look like instead of listing bad ones, because ngspice rewrites lines as it reads them [verified: run E82]. Every line from line 2 must start with a letter, `.` or `+`, must not have `#` as its second character, must hold no control character but a tab, and must not end in `\\`. Any other first character can be turned into `*` by ngspice, and `*#` then runs as a command; ngspice can also overwrite the first character of an ordinary line [verified: run E82]. `@` stops reading [verified: source E80]. It refuses a first token starting `.control`, `.endc`, `.inc`, `.lib` or `.title`, any `.end` before the last line, and any dot card not on the known list. A token ends at whitespace, one of `=(),+-*/^"'`, or a comment, as ngspice reads it, so `.end,x` and `.end-x` are an `.end` [verified: run E82, E83]. `.options` keys must be on a list of solver and log settings, because the rest set interpreter variables, `measoutfile` writes to any path, and `interp` thins the saved samples until a 2 µs pulse reads 0 V [verified: run E83, E95]. It refuses any line outside a comment holding the word `d_process` or `d_cosim`, ignoring case and quotes, because either name can reach the engine on a `.model` line, a continuation line or through a string `.param` [verified: run E77, E79]. Last, it joins `+` lines after removing each line's comment, as ngspice does, and requires every `.model*` and `.cmodel` card to have a name that is not an expression and a type that is a plain word and not the name of a `.func`. The type is the first word after the name that is not `key=value`, because ngspice deletes pairs such as `mfg=` first; after ` ako:`, it comes after the copied model's name. `$` starts a comment only outside PSpice mode, so a card must pass both readings. CIDER device models are refused, because they can write files [verified: run E84]. That closes the string-parameter question without proving anything about parameter handling: every route that reached `dlopen` used the type field [verified: run E79, E82]. This invariant has its own negative-control tests.

**Include resolution.** Relative paths search as ngspice searches: the project, because KiCad runs with it as the working folder, then the including file's folder or, at the top level, the first include's folder, then the most recent include's folder [verified: run E83]. `${KIPRJMOD}` resolves to the project, because KiCad leaves it unexpanded in schematic text [verified: run E81]. Every other variable, `~/` and network paths are refused. Paths must fall under the project, KiCad's library directories, KiCad's IBIS cache, or a configured `ngspice_model_dirs` entry, so an `.include` cannot pull a private file into the run folder or error text. A root that holds the user's home folder is never used, and a configured folder that is unusable is reported in `ignored_model_dirs`. Unresolved paths and cycles fail closed. `model_mode` is an input: under KiCad's default `lt` and `ps` modes, `.lib file section` includes the whole file [verified: run E52, source E5]; in other modes, every `.lib` line is refused in v1 as `unsupported_library_form`. `.incpslt` is always refused, because inlining changes which compatibility passes that file gets [verified: run E84], and so is a `.lib <section>` header that names no file. KiCad's own netlists use `.include` [verified: run E81]. Refusal text repeats only a filtered file name, never free text from a model file. Size is checked before a file is read: 64 MiB of includes and a million deck lines.

Flattened and included decks gave identical results on 13 KiCad demos, inlining 24 model files, and on a real library split into nested includes [verified: run E76].

**Deck assembly.** Line 1 is the fixed `.title`, then the flattened netlist minus excluded references, testbench lines, `.save all`, `.probe alli`, a `.probe p(NAME)` for each part a signal asks about, the requested analysis, `.end`. Those last lines are Konnect's own and carry no caller text: the analysis is numbers it formats, and a probe takes its name from the deck. `.probe` splices hidden sources into the circuit, so it runs only for parts asked about, and not on a coupling, where it stops the run. Under `use_initial_conditions` a device reads its start from its own pins, which a probe has moved to a hidden node, so with a `.ic` or `.nodeset` card every probe of Konnect's stays out, and a `.probe` card from the caller is refused as `probe_loses_initial_conditions`. A device's own `IC=` is not affected [verified: run E92, E94, E95]. Stop time is limited to 1e-15 to 1e6 s, and `max_step_seconds` to the stop time: ngspice's smallest step is a fixed share of its largest, so past those it stops for no reason in the circuit [verified: run E94]. Transient `tstep` is derived as stop time divided by 2000, which keeps the crash ratio out of reach, because the crash follows `tstop/tstep` rather than `tmax` [verified: run E10, E91]. It also caps the step at that size unless `max_step_seconds` says otherwise, and the `.tran` line states the cap whenever `start_save_seconds` is set, because ngspice would otherwise size it from the saved stretch alone [verified: run E94], and a run needs a step near a hundredth of its fastest edge to place a peak in time [verified: run E93].

**Engine hardening.**

| Measure | Why | Evidence |
|---|---|---|
| Load `digital.cm`, refuse `d_process` and `d_cosim` | D2 chose digital models on. It is the only code model importing `fork`, `execv` and `dlopen`, and only those two models call them. The deck scan refuses both | [verified: run E50, E77] |
| Worker sets `RLIMIT_NPROC` to 0 after loading, on macOS | Second barrier: `shell` fails and `bg_run` still works. `d_process` also needs `fork`. `d_cosim` needs only `dlopen`, so this does not stop it | [verified: run E53] |
| Windows job object with one active process; Linux needs a seccomp filter because threads count against the limit | Same barrier | [assumed] |
| Working directory is the run directory, holding an empty `.spiceinit` | `ngSpice_nospiceinit` is unusable, and the cwd file takes priority over the home one | [verified: run E70, source E23] |
| Cleared environment, except what loading needs | `SPICE_*` variables must not change results | [verified: source E11] |
| Point `SPICE_SCRIPTS` at the empty run directory, and set `ngCM_Input_Path` to the project | A `spinit` must not change results or load digital models past the off switch. `ngSpice_nospinit` and `ngSpice_nospiceinit` must not be called: both segfault before init on ngspice-44.2. Relative data files for code models | [verified: run E78, source E11] |

This is hardening, not a sandbox. See R4.

**Worker.** `konnect sim-worker --protocol 1 --run-dir <dir>`, or `--probe`, hidden from help. Parent side: `kill_on_drop`, stdin null, stdout and stderr to files in the run directory, because a child must not hold the MCP pipes [verified: source E60]. Sequence: read `request.json`; `dlopen`; init, with no `nospinit` calls; load code models with `codemodel` [verified: run E34]; send `set ngbehavior=...` before the deck, from the project's `model_mode` (0 none, 1 none, 2 `psa`, 3 `lta`, 4 `ltpsa`, 5 `hsa`), default `ltpsa` when absent or when no workbook is saved, as KiCad's simulator does [verified: source E5, E82; run E32]; drop process rights; `ngSpice_Circ`; `bg_run`; wait for the exit flag; print `$sim_status` through its own `echo` marker, because the library has no getter [verified: run E15, source E11]; `write results.raw`; write `status.json` last, with the explicit `halted_reason`. Single use, then exit. The worker also exits by itself at timeout plus grace, so a dead parent cannot leave it stuck.

`--probe` loads the library, initialises it and prints version and code-model count as JSON. It is the only source of `engine.version` and `available`, so a library that exists but cannot load is never reported as available.

**How konnect-core finds the worker.** A `OnceLock` on `ToolContext`, set through a `McpHandler` setter called only from `main.rs`, carries the worker path and ngspice keys [verified: source E60]. Embedded and test contexts refuse by absence, and tests inject a fake worker.

**Limits.** `timeout_seconds` is a budget for the whole call, export included. Default 40, because clients on the TypeScript SDK time out at 60 s and Konnect sends no progress [verified: source E59]; maximum 600. The worker runs `bg_halt` at the budget, keeps partial data, and the parent kills 15 s later, because `bg_halt` is cooperative [verified: source E11]. `max_raw_bytes` default 256 MiB, maximum 1 GiB: ngspice's data callback counts the points on the engine's own thread, and the worker halts softly at the limit, since memory is about 8 bytes per point per vector at about 700,000 points per second [verified: run E25, E90]. The callback costs nothing measurable. Polling `ngGet_Vec_Info` from the main thread does not work [verified: run E90]. A halt lands just past the limit, so the server's read limit stands 25 percent above the maximum. Soft halt rescued a real runaway: 4 s, 173,841 points, 5.18 of 20 ms, data intact [verified: run E57].

**Preflight findings,** run on the assembled deck, after exclusions and testbench lines.

| Finding | Rule | Severity | Evidence |
|---|---|---|---|
| Child sheet | Root schematic only, as `run_erc` does | Refused | [verified: source E39] |
| `unmodelled_symbols` | `REF __REF` with no nodes and no `.model` | Refused | [verified: run E2, source E4] |
| `invalid_component_value` | A value ngspice would not read as written, judged the way it reads: a number, then scale letters, then it stops. Refused: a word; `3V3`; `10k-1%`, `10k(1%)` and `100_nF`, where the token ends early; a rating such as `16V` on a capacitor or `100n 50V`, where the last number wins; `ic=1` with no value; a waveform number of those shapes; `DC 12V DC`; `K1 L1 L2 90%`. Passing: suitable units, tags ngspice ignores such as `47R;1W` or `10k@0402`, expressions, `.param` names and models. `4k7` passes only in `lt` modes, because elsewhere ngspice silently reads 4k | Refused | [verified: run E3, E85] |
| `no_ground_reference` | Nothing touches node `0` or `GND`, at top level, inside a called subcircuit, through an analog code model, or through a nonzero `rshunt`. Potentiometers, digital models and `gshunt` add no reference. List ground-like nets | Refused | [verified: run E16, E85] |
| `net_name_collision` | Distinct KiCad nets holding live parts that map to one SPICE name, as ngspice reads it: lowercase, `gnd` as `0`, and every non-ASCII byte as `_`, so two Hebrew labels of one length merge. Also `A(1)` and `A[1]`, and a net holding `=`, which ngspice cuts in two | Refused | [verified: run E17, E69, E85, E86] |
| `floating_subcircuit` | A group of nodes with no DC path to node 0. Capacitors, current sources, MOS gates, controlled-source inputs and digital nodes are not DC paths. A subcircuit is read pin by pin through its body, including nested calls and `.global` nodes, and a body with a line that cannot be read counts as joining and grounding every pin | Warning here, because series capacitors and isolated windings give correct AC and transient results. `run_simulation` should refuse it for an operating point | [verified: run E16, E85] |
| `unknown_testbench_node` | A testbench node, or `.ic` or `.nodeset` target, that is not a schematic net or `0`, and that no other card names. Lines from a file a testbench includes count. A typo otherwise makes a new silent node: the run reports success with zero volts everywhere | Refused | [verified: run E63, E85] |
| `no_ac_stimulus` | Kind `ac` with no source carrying an AC value | Refused | [verified: run E54] |
| `sweep_target_not_found` | `dc_sweep` target absent from the deck. Caught early, for a typed reason instead of a fatal engine message | Refused | [verified: run E63] |
| `encrypted_model` | Model line starting `$CDNENCSTART` | Refused | [verified: run E32] |
| `unreadable_model` | NUL bytes or invalid UTF-8 outside comments | Refused | [verified: run E19] |
| `model_library_missing` | Unresolved include, or `kicad-cli` reported a missing library | Refused | [verified: run E12] |
| `unsafe_deck_content` | Refused forms above, or a path outside the allowed roots, with file and line | Refused | [verified: run E18 to E20, E50 to E52] |
| `unbalanced_deck_block` | A `.if` or `.subckt` left open, or a stray `.endif` or `.ends`, matched by prefix as ngspice does. An open block swallows the analysis line Konnect adds, and the engine then reports a failure that hides the cause | Refused | [verified: run E89] |
| `device_currents_not_saved` | Digital or event-driven parts are present, or a `.ic` or `.nodeset` card meets `use_initial_conditions`, so `.probe alli` is left out and only analog node voltages and source currents are saved | Warning, in the result | [verified: run E88, E94] |
| `probe_loses_initial_conditions` | A `.probe` card meets `.ic` or `.nodeset` under `use_initial_conditions` | Refused | [verified: run E95] |
| `no_point_at_start` | ngspice saved no point at time zero, as under `use_initial_conditions`, so an event at that instant is cut short. `analysis.first_point_seconds` says where the data starts | Warning, in the result | [verified: run E94] |
| `engine_messages` | What the engine ignored or changed in a run that finished, such as an unknown model parameter or a resistor raised to its minimum | Warning, in the result | [verified: run E89] |
| `code_models_missing` | No `.cm` files found. Decks needing them will fail to parse | Warning | [verified: run E34] |
| `unpowered_net` | A group with no independent source. Dependent sources do not count, and a subcircuit counts only if its body holds a source | Warning | [verified: run E63, E85] |
| `inferred_from_value` | An R, L, C, V or I element whose `lib_id` is not a plain R, L, C or source symbol, such as an NTC named `RT1` | Warning, disclosed in the result | [verified: source E4] |
| `generic_default_model` | A varistor or potentiometer still at library defaults, such as a varistor's 1 kV threshold or a dual potentiometer's 1 ohm | Warning, disclosed in the result | [verified: source E62, run E85] |
| `digital_models_disabled` | An element instantiates a digital code model while `digital.cm` is off. Otherwise the engine gives only a parse error | Refused | [verified: run E72, E85] |
| `ground_like_local_net` | A net such as `/gnd` or `/0`. KiCad 10.0.1 keeps these as ordinary nodes; later 10.0 builds turn them into ground | Warning | [verified: run E69, E71] |
| `net_map_mismatch` | A node in the SPICE netlist that the name port cannot explain. Readiness then returns `spice_net: null` for it, never a guess | Warning | [verified: run E69] |
| `pulse_period_defaulted` | `PULSE` with fewer than 7 values, because the default changed at ngspice 46 | Warning | [verified: source E46] |
| `duplicate_element` | Two top-level elements with one name, such as a netlist part repeated in `spice_lines` | Refused | [verified: run E85] |
| `missing_referenced_element` | A `K`, `F` or `H` element whose partner is absent, which excluding a part can cause | Refused | [verified: run E85] |
| `ambiguous_scale_letter` | A capital `M` that is not `Meg`, such as `1MOhm` or `1MHz`, in any R, L, C or source field or `.param`; `1F` on C; `5A` on I. ngspice reads these as milli, femto and atto | Warning | [verified: run E85] |
| `implausible_value` | A capacitance or inductance of 1 F or 1 H or more, a bare decimal of 1 mF or more on a capacitor, or a source of 10 kV or more. KiCad turns `22P` into peta, and a battery value `18650` is 18.65 kV | Warning | [verified: run E85] |
| `source_without_value` | A V or I source with no value, which reads as zero | Warning | [verified: run E85] |
| `gnd_pin_in_subcircuit` | A subcircuit pin named `gnd` or `0`, which ngspice reads as global ground, so the net wired to it is not connected | Warning | [verified: run E85] |
| `exclusion_matched_nothing` | An `exclude_references` entry that selects no symbol | Warning | |
| `ground_only_inside_subcircuit` | No top-level element touches node 0, so the only reference is inside a subcircuit, and a ground net named `GNDA` floats | Warning | [verified: run E85] |
| `decimal_comma` | `1,5V` or `0,5us` in a source, or any `1,5` in a waveform other than `PWL`, where every later field moves along by one | Warning | [verified: run E85] |
| `duplicate_definition` | One model or subcircuit name with two different bodies, where ngspice uses the first for both | Warning | [verified: run E85] |
| `unknown_initial_node` | A schematic `.ic` or `.nodeset` on no net, which ngspice drops without saying so | Warning | [verified: run E85] |

The element parser needs a node count per device letter. A device it cannot parse, or any `.if` block, stops the checks that need the whole circuit and reports `connectivity_not_checked`, `ground_not_checked` or `testbench_nodes_not_checked`, so an unrun check never reads as a pass. `no_ac_stimulus` and `sweep_target_not_found` need an analysis, so they arrive with those kinds in PR 9.

**Classification.** Log text explains; it never decides.

| Signals | Result |
|---|---|
| Worker died, or no `status.json` | error `engine_crashed`, or `timed_out` if the server killed it or it left at its own deadline |
| Exit callback fired at any stage. It can fire during a run, leaving `sim_status=0` and an empty plot | error `engine_failed` [verified: run E73] |
| `$sim_status` undefined after `run` | error `circuit_not_parsed` [verified: run E15] |
| `sim_status=1`, zero points | error `no_convergence` [verified: run E13] |
| `sim_status=1`, some points | partial, reason `no_convergence`, with point and node [verified: run E14] |
| `halted_reason` set, zero points | error `timed_out` or `size_limit` |
| `halted_reason` set, some points | partial, reason `timeout` or `size_limit`. A halted run reads `sim_status=0`, so the worker's own record decides [verified: run E15, E57] |
| `sim_status=0`, coverage proven | complete, with any warnings |
| `sim_status=0`, coverage not proven | partial, reason `incomplete_coverage` |

Coverage proof per kind:

| Kind | Proof |
|---|---|
| `operating_point` | One point, non-empty vectors |
| `transient` | Last time reaches stop time within tolerance |
| `ac` | Last frequency reaches `stop_hz` within tolerance |
| `dc_sweep` | Expected point count |

**Reading results.** `simulation/raw.rs` reads what `write` produces, in text and binary, real and complex. A file cut short keeps every whole point and reads as truncated; a damaged line is an error, never a value [verified: run E87]. The file alone cannot prove success, so PR 6 must not treat it as proof:

| Trap | Rule for the worker and classification | Evidence |
|---|---|---|
| A halted run writes a complete-looking file with fewer points | `halted_reason` from the worker decides | [verified: run E87] |
| `write` after a failed analysis falls back to the `constants` plot | The parser refuses it as `NoResults`, which reads as `results_unreadable` | [verified: run E87] |
| Batch `run file` leaves a placeholder point count and garbage AC imaginary parts | The worker uses `write`, never batch output | [verified: run E87] |
| `write` keeps only the last plot, so noise has no spectrum | The noise PR must write each plot | [verified: run E87] |
| `v(pole(1))` and `v(r1:r)` have kind `voltage` | Units come from the signal the user asked for, not from the kind | [verified: run E87] |
| Two variables can share a name. An inductor or behavioural source reports its current twice under one name, differing by solver rounding, and a two-port `dac_bridge` reports two different currents | Data that agrees to a millionth of its peak is one signal. Different data returns nothing rather than guess: `signal_not_found` | [verified: run E87, E92, E94] |
| Non-ASCII labels arrive as underscores | Map a label through the engine's rule before looking it up | [verified: run E86] |

**Signals.** Grammar: `v(NET)`, `v(NET1,NET2)`, `i(REF)`, `i(REF:PIN)`, `p(REF)`. Net names are given as KiCad shows them. References map to element names through the element list, so `i(BT1)` finds `i(vbt1)`. Raw names are lowercase, for example `v(/vout)`, `i(r1)`, `i(m1:d)`, `m1:power` [verified: run E30]. A multi-pin part has no plain current, only `i(m1:d)` and the like, so `i(M1)` returns `signal_not_found` with the real `pins`. Subcircuits follow the same pattern, and every device reports power but code models, which ngspice leaves out without a word [verified: run E92]. A testbench element has no reference, so its own name stands, even over an excluded part it replaces. Each name passes through the engine's rule, so a non-ASCII reference is found. `v(NET1,NET2)` is worked out in Rust, and ground reads zero. A whole name that is a net wins over splitting at a comma. Current is positive into the device's first pin, so a delivering source reads negative [verified: run E27]. If `signals` is omitted, the result has no summaries, and with no `measurements` there are none either, which keeps the default small. Two nets the engine reads as one node give the shared values and a `nets_share_engine_node` warning.

**Measurements** run in Rust on parsed vectors, because they need exact definitions, and adaptive steps cluster on edges: naive RMS read 5.30 where the time-weighted value is 2.69 [verified: run E28].

| Kind | Definition | Unit |
|---|---|---|
| `min`, `max` | Extreme value and its location | Signal |
| `peak_to_peak` | `max` minus `min` | Signal |
| `peak_abs` | Value of largest magnitude, signed, with location | Signal |
| `average`, `rms` | Time-weighted over the window [verified: run E28]. RMS uses the exact integral of the straight line squared: a trapezoid of the squares reads high on every slope, 3.2 percent on a triangle [verified: run E94] | Signal |
| `integral` | Trapezoid integral | Signal × s |
| `i2t` | The exact integral of the straight line squared, for fuses | A²s |
| `value_at` | Linear interpolation. For AC returns gain in dB and phase | Signal |
| `crossing` | First time the signal reaches `level` from the other side, in `direction`, returned on the analysis axis. A signal that starts on the level has not crossed it | Axis |
| `rise_time`, `fall_time` | 10 to 90 percent between start and final level. Defaults: window start and end values, or `final_value`. From the first time it reaches 90 percent, back to the last time it left 10 percent, so ringing before the edge does not count. See below for what stops a default final value | s |
| `overshoot_percent` | How far past final the signal goes, as a share of `|final - start|`, downward for a falling step. Zero if it never passes | % |
| `settling_time` | Time from window start until the signal stays within `band_percent` of final. The band is a share of `|final|` or `|final - start|`, whichever is larger: a rail has a large final and a small step, a discharge the reverse. If both are under a thousandth of the swing, as an inrush that dies away, the band is a share of the swing. With no `final_value`, the last tenth of the window must also stay in the band, because the last value is always inside a band around itself | s |
| `bandwidth_3db` | First point 3.01 dB below the reference. Default reference is the lowest frequency; `reference_level: "peak"` returns lower and upper edges | Hz |
| `phase_margin` | `180 + phase` of `signal / reference_signal` at unity gain, phase unwrapped from the lowest frequency. Returns the crossover frequency too. `signal` must be a non-inverted loop gain | degrees |
| `gain_margin` | Negative gain in dB at phase -180 | dB |

A signal is taken as straight lines between its samples, so a window starts and ends between two of them. Each measurement names only the fields it uses, and a stray one is refused rather than ignored. Margin and `bandwidth_3db` calculations matched an analytic three-pole system [verified: run E29]. The rise, settling, overshoot and I²t definitions matched closed forms, in unit tests and through the engine, and a soft-start and load step matched a reference worked out with no SPICE engine [verified: run E93].

With no `final_value`, the time-weighted mean of the window's last tenth stands in for it, because a last sample moves with the ripple phase: `settling_time` on `buck_conv` swung from -16 to +75 percent with the window's end [verified: run E96]. That holds only if the signal has stopped: the last tenth of the window stays within 5 percent of the step, its two halves average within 0.1 percent of the step, so ripple does not count and a slow tail does, and the signal stays within three steps of its start. For settling, the two halves may differ by the larger of that and 5 percent of the band. Otherwise a ramp, a sine or a pulse that came back gives a confident number measured against whatever value the window ended on: an RC cut off at 3 time constants read 17 percent short [verified: run E94, E95]. Each such result returns the `start_value` and `final_value` it used, because no test can see every window that ends on the wrong level: one ending just past a crest, within about 0.7 percent of its length, still passes, and a sag that stays down reads as overshoot [verified: run E96]. A step below a billionth of the signal's size, or below 1e-12, is rounding noise.

A value that does not exist is `null`, never 0, and carries a `finding`: `level_not_reached`, `starts_on_level`, `no_transition`, `final_value_needed`, `not_settled`, `outside_data` or `non_finite_data`. That is an engineering finding, not a failed check. On a partial run, each value is flagged `window_truncated: true` and counted as failed in `outcome`. A window reaching past the data, such as before `start_save_seconds`, is flagged the same way but not counted, because the rest of the run is sound. A summary carries the same flag on a partial run. A signal the run does not hold gives `signal_not_found` and a partial outcome. A summary signal has no outcome unit of its own, so a missing one fails the "range covered" unit.

**Protection scenarios.** Ship as tested stand-ins in the skill, pasted into `spice_lines`. The E56 decks were not kept, so PR 8 rebuilt them as fixtures in `tests/fixtures/simulation/protection/`. The skill shows those files line for line, and a test fails if the two differ. Each ran through `run_simulation` against `reference.py`, which works the expected values out with no SPICE engine [verified: run E97]:

| Scenario | How | Result |
|---|---|---|
| Output short | A behavioural resistor across the output, falling from 1 G to 5 mΩ around a set time | 480.04 A and 2.399 V, as the circuit equation gives |
| I²t fuse | A capacitor integrating the current squared, and a switch with hysteresis that opens on it and stays open | Opened at 5.020535 ms against 5.020498 ms from the reference, at a 0.1 µs step. Its integral stopped at 2.0059 A²s against a trip at 2.002, and at 2.0174 with a 0.2 µs step |
| eFuse current limit | A behavioural current source, `ilim * tanh(v / (ilim * ron))` | 5.000 A into a short, 11.97976 V into a 12 Ω load against 11.97976 V |
| Undervoltage lockout | A switch with hysteresis driven by its own input, on a slow ramp | On at 10.003 V and off at 9.001 V against 10 and 9, at a 10 µs step |
| Reversed battery | A behavioural pass element, on a slow transient ramp from -12 to 12 V | 1.0 µA of leakage at -12 V, 11.970 V at 12 V |

**Reversed battery uses a ramp for now.** The sweep form, `dc_sweep` from -12 to 12 V, does not exist until PR 9, so PR 8 proves the scenario with a transient ramp through `run_simulation`. The stand-in has no reactive part, so a ramp gives the same curve as a sweep. PR 9 re-runs the scenario as `dc_sweep` and compares the two.

**Where PR 8 differs from the first plan.** The short is a behavioural resistor and not a switch, so the reference can reproduce its edge exactly. The fuse needs a latch: without one it settles half open, because the current falls as the resistance rises and the integral stops short of the rating [verified: run E97]. A behavioural expression turns `exp()` linear above an argument of 14, so every power is written with `**` [verified: run E98]. A hard-edged control was not found to run away, and a lockout whose hysteresis is narrower than the supply sag stops with `no_convergence` [verified: run E98].

These are behavioural approximations, not models of a specific part. Thermal behaviour is not modelled. A result reads `simulated_circuit: "modified"`.

**Source disclosure.** Every result carries `sources` and `source_evidence`, following `provenance()` in `tools/board_source.rs` [verified: source E39]: saved files, `excludes_unsaved_editor_state: true`, and locks seen. A lock is disclosed, not refused, because a stale read is disclosable. `inputs_hash` covers the snapshot sheets, the `.kicad_pro` and every inlined model file. `design_state_hash` is the wrong set, because it includes the board and omits models [verified: source E60].

### 2.3 Tools

Shared schema fragments are Rust helpers, and each tool embeds its own `$defs`, because a dangling `$ref` panics at catalogue build [verified: source E60]. `operating_point` is the last `oneOf` arm, because the validator reports the first arm on a tie [verified: source E60]. Top-level arguments are read by literal name in `tools/simulation.rs`, because `every_declared_parameter_is_read_by_its_registered_handler` scans only that level [verified: source E60]. Each PR declares only the parameters its handler reads.

`schematic` keeps the bare noun used by `run_erc` and `generate_netlist`, which `NAMING_CONVENTIONS.md` tolerates. Time and phase follow existing `_seconds` and `_degrees`. Electrical units have no listed precedent, so this adds `_hz`, `_db`, `_percent`.

#### `check_simulation_readiness`

Reports whether a schematic can be simulated, and why not, without running a simulation.

```json
{
  "type": "object",
  "required": ["schematic"],
  "properties": {
    "schematic": { "type": "string", "description": "Root .kicad_sch path" },
    "testbench": { "$ref": "#/$defs/testbench" },
    "net_filter": { "type": "string", "description": "Substring filter on net names" },
    "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 25 }
  },
  "$defs": { "testbench": "embedded by the shared helper" }
}
```

Output:

```json
{
  "ready": false,
  "checks": [
    { "name": "engine", "ran": true, "passed": true },
    { "name": "netlist_export", "ran": true, "passed": true },
    { "name": "deck_vetting", "ran": true, "passed": true },
    { "name": "model_coverage", "ran": true, "passed": false, "code": "unmodelled_symbols", "total": 9, "shown": 5, "truncated": true, "items": ["D1"] },
    { "name": "component_values", "ran": true, "passed": true, "total": 0, "shown": 0, "truncated": false, "items": [] },
    { "name": "ground", "ran": true, "passed": false, "code": "no_ground_reference", "total": 1, "shown": 1, "truncated": false, "items": ["PGND"] },
    { "name": "connectivity", "ran": false, "code": "connectivity_not_checked", "reason": "Unmodelled symbols have no nodes in the deck yet.", "total": 0, "shown": 0, "truncated": false, "items": [] }
  ],
  "engine": { "available": true, "version": "44.2", "library_path": "...", "code_model_count": 5, "compatibility": "ltpsa", "probe_status": "ok" },
  "nets": { "total": 9, "shown": 5, "truncated": true, "items": [ { "kicad_name": "/VBAT", "spice_name": "/VBAT", "in_deck": true, "pin_count": 4 } ] },
  "elements": { "total": 13, "shown": 5, "truncated": true, "items": [ { "reference": "BT1", "element_name": "VBT1", "modelled": true, "excluded": false } ] },
  "deck": { "line_count": 40, "inlined_file_count": 2, "dropped_directives": [".tran"] },
  "unmodelled_symbols": {
    "total": 9, "shown": 5, "truncated": true,
    "items": [ {
      "reference": "F1", "value": "10A", "lib_id": "Device:Fuse", "pin_total": 2,
      "pins": [ { "pin": "1", "pin_name": "~", "pin_type": "passive", "net": "/VBAT", "spice_net": "/VBAT" } ],
      "suggested_fix": "replace_series_part"
    } ]
  },
  "warnings": [ { "code": "floating_subcircuit", "detail": "...", "total": 1, "shown": 1, "truncated": false, "items": ["x, y"] } ],
  "source_evidence": { "excludes_unsaved_editor_state": true, "locks_present": [] }
}
```

`ready` is `true` only if every check ran and passed, `false` if one failed, and `null` if a check could not run, with the missing evidence named. The `deck_vetting` check carries a `refusal` with `code`, `origin` and `line`, and a passing deck adds `deck` with its line count, inlined file count and `dropped_directives`. An `unmodelled_symbols` item suggests `copy_library_sim_fields` when the library symbol has `Sim.*` fields and the placed one does not, `replace_series_part` when two or more pins on signal nets touch other parts, and `exclude` otherwise. `spice_net` is null for a net the deck does not name.

| Case | Result |
|---|---|
| Findings, including a missing engine | Normal output, as `check_freerouting` does. `engine.available: false` with `searched_locations` |
| Export failed | Normal output, `netlist_export` with `passed: false`, `ready: false` |
| Bad argument, child sheet | `invalid_argument` |
| Schematic missing | `file_not_found` |
| Ownership conflict | `conflict` or `stale_target`, as `run_erc` maps them |

#### `run_simulation`

Runs one analysis on a schematic and returns compact results.

```json
{
  "type": "object",
  "required": ["schematic", "analysis"],
  "properties": {
    "schematic": { "type": "string" },
    "analysis": { "oneOf": [
      { "type": "object", "required": ["kind", "stop_time_seconds"], "properties": {
          "kind": { "const": "transient" },
          "stop_time_seconds": { "type": "number", "exclusiveMinimum": 0 },
          "max_step_seconds": { "type": "number", "exclusiveMinimum": 0 },
          "start_save_seconds": { "type": "number", "minimum": 0 },
          "use_initial_conditions": { "type": "boolean", "default": false } } },
      { "type": "object", "required": ["kind", "start_hz", "stop_hz"], "properties": {
          "kind": { "const": "ac" },
          "scale": { "enum": ["decade", "octave", "linear"], "default": "decade" },
          "points": { "type": "integer", "minimum": 1, "maximum": 10000, "default": 50, "description": "Per decade or octave, or total if linear" },
          "start_hz": { "type": "number", "exclusiveMinimum": 0 },
          "stop_hz": { "type": "number", "exclusiveMinimum": 0 } } },
      { "type": "object", "required": ["kind", "sweeps"], "properties": {
          "kind": { "const": "dc_sweep" },
          "sweeps": { "type": "array", "minItems": 1, "maxItems": 2, "items": { "type": "object",
            "required": ["target", "start_value", "stop_value", "step_value"], "properties": {
              "target": { "type": "string", "description": "Source or resistor reference, or TEMP. Values use its unit" },
              "start_value": { "type": "number" },
              "stop_value": { "type": "number" },
              "step_value": { "type": "number", "not": { "const": 0 }, "description": "Negative sweeps down" } } } } } },
      { "type": "object", "required": ["kind"], "properties": { "kind": { "const": "operating_point" } } }
    ] },
    "testbench": { "$ref": "#/$defs/testbench" },
    "signals": { "type": "array", "maxItems": 16, "items": { "type": "string" } },
    "measurements": { "type": "array", "maxItems": 32, "items": { "$ref": "#/$defs/measurement" } },
    "envelope_points": { "type": "integer", "minimum": 0, "maximum": 64, "default": 0 },
    "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 40 },
    "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 600, "default": 40 },
    "max_raw_bytes": { "type": "integer", "minimum": 1048576, "maximum": 1073741824, "default": 268435456 }
  },
  "$defs": {
    "testbench": { "type": "object", "properties": {
      "exclude_references": { "type": "array", "items": { "type": "string" }, "description": "References or prefix wildcards such as J*" },
      "spice_lines": { "type": "array", "maxItems": 200, "items": { "type": "string" }, "description": "Elements, models and includes. Use spice_name values from readiness. Give every PULSE field" } } },
    "measurement": { "type": "object", "required": ["kind", "signal"], "properties": {
      "name": { "type": "string" },
      "kind": { "enum": ["min", "max", "average", "rms", "peak_to_peak", "peak_abs", "integral", "i2t", "value_at", "crossing", "rise_time", "fall_time", "overshoot_percent", "settling_time", "bandwidth_3db", "phase_margin", "gain_margin"] },
      "signal": { "type": "string" },
      "reference_signal": { "type": "string" },
      "from_seconds": { "type": "number" }, "to_seconds": { "type": "number" }, "at_seconds": { "type": "number" },
      "from_hz": { "type": "number" }, "to_hz": { "type": "number" }, "at_hz": { "type": "number" },
      "from_sweep": { "type": "number" }, "to_sweep": { "type": "number" }, "at_sweep": { "type": "number" },
      "level": { "type": "number" },
      "direction": { "enum": ["rising", "falling", "either"], "default": "either" },
      "final_value": { "type": "number" },
      "band_percent": { "type": "number", "exclusiveMinimum": 0, "default": 2 },
      "reference_level": { "enum": ["low_frequency", "peak"], "default": "low_frequency" } } }
  }
}
```

`stop_hz` must exceed `start_hz`, checked in the handler. A negative `step_value` sweeps down [verified: run E55]. Later analysis kinds add `oneOf` arms, so they add no tools. The schema is about 3.5 KB before descriptions, paid on every listing by clients that load the whole catalogue [verified: source E60].

Output, transient example:

```json
{
  "run_id": "3fa9c1d27b04-20261030T133512-7f3a",
  "simulated_circuit": "modified",
  "circuit": {
    "excluded_references": ["J1", "J2"],
    "added_line_count": 4,
    "dropped_directives": [ ".control block", ".tran" ],
    "approximations": [ { "reference": "RT1", "code": "inferred_from_value" } ]
  },
  "analysis": { "kind": "transient", "stop_time_seconds": 0.14, "simulated_to_seconds": 0.14, "coverage_ratio": 1.0, "points": 7053 },
  "engine": { "name": "ngspice", "version": "44.2", "compatibility": "ltpsa", "code_models_loaded": 6 },
  "sources": ["saved_schematic_snapshot"],
  "source_evidence": { "excludes_unsaved_editor_state": true, "locks_present": [], "inputs_hash": "..." },
  "measurements": [ { "name": "inrush_peak", "kind": "peak_abs", "signal": "i(vbat)", "value": -16.56, "unit": "A", "at_seconds": 0.01012, "window_truncated": false } ],
  "signal_summaries": [ { "signal": "v(/vout)", "unit": "V", "min": 0.0, "max": 11.95, "final": 11.85, "envelope": { "x": [], "min": [], "max": [] } } ],
  "warnings": [],
  "artifacts": { "dir": "...", "deck_path": "...", "raw_path": "...", "log_path": "...", "raw_bytes": 225696 },
  "outcome": { "status": "complete", "target": "...", "source": "saved_schematic_snapshot", "requested": 3, "completed": 3, "failed": 0, "retry": null }
}
```

`simulated_circuit` is `as_drawn` when nothing was excluded or added, and otherwise `modified`. It is mandatory, and the skill must relay it. A modified run is `complete` for the deck that ran, because that is what was requested, but it is never presented as a result for the unmodified schematic. `envelope` appears only when `envelope_points` is above 0, and holds evenly spaced buckets keeping each bucket's min and max.

Per kind:

- `operating_point`: `operating_point: { total, shown, truncated, rows: [ { signal, value, unit } ] }`, counts computed before the cut, as `run_drc` does. The full table sits beside the raw file.
- `ac`: `signal_summaries` carries `peak_gain_db`, `peak_at_hz`, `low_frequency_gain_db`, `high_frequency_gain_db`.
- `dc_sweep`: `signal_summaries` carries `min`, `max`, `at_start`, `at_stop`.

`outcome` counting: `requested` is 2 plus the measurement count. The two fixed units are "engine ran" and "requested range covered", which keeps partial expressible with no measurements (2, 1, 1), as the `outcome.rs` assertion requires [verified: source E42]. Partial uses both nonzero. Refusals carry `failed` with `retry_whole_request()`.

Failure matrix. Decision: **one new error kind plus the existing envelope.** Add `ToolErrorKind::SimulationFailed { stage, reason, log_path, items }`, with `reason` a closed enum. `OutcomeStatus` has `Failed`, which adopted tools attach to refusals [verified: source E42], so every row below also carries `outcome`. Partial data is not an error.

| Case | `simulation_failed` reason, or result |
|---|---|
| Bad argument, child sheet | `invalid_argument` |
| Schematic missing | `file_not_found` |
| Ownership conflict | `conflict` or `stale_target` |
| Simulator missing | `engine_not_found`, with searched locations |
| Library will not load, wrong protocol, no worker in this mode | `engine_load_failed` |
| Netlist export fails or times out | `netlist_export_failed` or `netlist_export_timed_out`, with reporter text |
| Sheet outside the project | `sheet_outside_project` |
| Preflight refusal | The finding's code from section 2.2 |
| Circuit does not parse | `circuit_not_parsed`. A log line is echoed only if none of its words start a word found only in model files, and hidden lines are counted. A value computed from a model parameter cannot be recognised, so it can still show [verified: run E89] |
| Library fatal error at any stage | `engine_failed` |
| Run folder cannot be created or written | `run_store_failed` |
| No convergence, no data | `no_convergence`, node and point when known |
| Timeout, no data | `timed_out` |
| Worker crash | `engine_crashed`, signal or exit code |
| Raw file unreadable | `results_unreadable` |
| Requested signal absent after the run | Partial; that measurement counts as failed with `signal_not_found` |
| Measurement does not fit the analysis, or `signals` on an operating point | `invalid_argument` before the run |
| No convergence, timeout or size limit, some data | `outcome.status: partial`, `stopped: { reason, at_point, node }`. `node` appears when the engine named one |
| Finished with warnings | `outcome.status: complete`, `warnings` |
| Level never reached | Complete, `value: null`, `finding` |

Diagnostic text is capped and treated as untrusted data.

#### `list_simulation_runs`

Lists stored runs for a schematic, newest first, so a lost response does not lose a run.

```json
{
  "type": "object",
  "required": ["schematic"],
  "properties": {
    "schematic": { "type": "string" },
    "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 10 }
  }
}
```

Output: `{ total, shown, truncated, runs: [ { run_id, created_at, analysis_kind, run_status, simulated_circuit, coverage_ratio, inputs_changed_since_run } ] }`. No runs is a valid empty list. Errors: `file_not_found`, `invalid_argument`.

#### `measure_simulation_run`

Computes more measurements on a stored run without simulating again.

```json
{
  "type": "object",
  "required": ["run_id", "measurements"],
  "properties": {
    "run_id": { "type": "string", "pattern": "^[0-9a-f]{12}-[0-9]{8}T[0-9]{6}-[0-9a-f]{4}$" },
    "signals": { "type": "array", "maxItems": 16, "items": { "type": "string" } },
    "measurements": { "type": "array", "minItems": 1, "maxItems": 32, "items": { "$ref": "#/$defs/measurement" } },
    "envelope_points": { "type": "integer", "minimum": 0, "maximum": 64, "default": 0 }
  },
  "$defs": { "measurement": "embedded by the shared helper" }
}
```

Output: `run_id`, `run_status`, `simulated_circuit`, `coverage_ratio`, `measurements`, `signal_summaries`, `inputs_changed_since_run`, `outcome`. `requested` is the measurement count. `outcome` is `partial` if the stored run was partial. Adopted alongside `run_simulation`.

| Case | Result |
|---|---|
| Bad id, measurement not valid for the stored analysis | `invalid_argument` |
| Run gone, raw file unreadable | `simulation_failed`, `run_not_found` or `results_unreadable` |
| Signal not in the raw file | That measurement fails with `signal_not_found`; others proceed |

#### `plot_simulation_run`

Renders stored signals to an SVG file and returns its path.

```json
{
  "type": "object",
  "required": ["run_id", "signals"],
  "properties": {
    "run_id": { "type": "string", "pattern": "^[0-9a-f]{12}-[0-9]{8}T[0-9]{6}-[0-9a-f]{4}$" },
    "signals": { "type": "array", "minItems": 1, "maxItems": 8, "items": { "type": "string" } },
    "width_px": { "type": "integer", "minimum": 200, "maximum": 1600, "default": 800 }
  }
}
```

Output: `{ path, width_px, height_px, bytes, signals_plotted, run_status, coverage_ratio, simulated_circuit, inputs_changed_since_run }`. The SVG carries a visible banner if the run was partial or modified, and envelopes are downsampled so file size stays bounded. Same errors as `measure_simulation_run`, and an absent signal is `invalid_argument`.

**Rendering decision.** v1 writes SVG with a small Rust module and adds no plotting crate. `konnect_render::svg_to_png` refuses visible `<text>` [verified: source E41], so a PNG needs labels as stroke paths, which comes later with a small vector font [assumed: licence to be checked]. Inline images come later too, only on explicit request, capped at 800 px, with the JSON block first because `outcome::attach` reads the first block [verified: source E42].

### 2.4 Later analyses

One PR per family after v1, each adding a `oneOf` arm, an output shape and its failure cases.

| Family | Arm | Output | Notes |
|---|---|---|---|
| Noise | `noise`, output and input source, scale, range | Integrated output and input noise, spectrum on disk | Plot differs from AC |
| Pole-zero | `pole_zero`, input and output nodes, transfer type | Lists of poles and zeros | KiCad reads these with `print all` [verified: run E35] |
| FFT | `fft` on a stored transient, signals, window | Top harmonics, THD, spectrum on disk | Needs `linearize`; the run stays single-use because it re-reads the raw file in Rust [assumed] |
| TF, sensitivity | Small arms | Scalar tables | KiCad Custom page only |
| S-parameters | `s_parameters` | Touchstone on disk | [assumed] |
| Tuners, sweeps, corners | `value_overrides`, then sweeps | One row per variant | High value: tolerance on inrush |
| Expressions | Safe subset | `signal_summaries` | No free-form `let` |
| Workbook import | Read stored analyses from `.wbk` | Echoed analyses | GUI-only format |

## 3. Files to add or change

No new workspace member, because that breaks the Nix build unless the flake changes too [verified: source E43]. `konnect-core` stays free of `unsafe` [verified: source E44].

**Add**

| File | Purpose | Pattern followed |
|---|---|---|
| `crates/konnect-core/src/tools/simulation.rs` | `tools()`, `tool!` registrations, `handle_*`, schema helpers | `tools/sch_export.rs`; `point_schema()` in `footprint_graphics.rs` |
| `crates/konnect-core/src/simulation/mod.rs` | Module root | Existing modules |
| `crates/konnect-core/src/simulation/protocol.rs` | `request.json`, `status.json`, protocol version; shared with the worker | New; both crates depend on it |
| `crates/konnect-core/src/simulation/snapshot.rs` | Snapshot, `Sim.Library` rewrite, non-bailing hierarchy walk, lock disclosure including `.kicad_pro` | Port of `snapshot_tree`; the viewer is outside the workspace |
| `crates/konnect-core/src/simulation/flatten.rs` | Flatten, vet, invariant | Pure parsers such as `specctra.rs` |
| `crates/konnect-core/src/simulation/preflight.rs`, `net_names.rs`, `kicad_netlist.rs`, `element.rs` | Checks, name port, KiCad and SPICE netlist parsing, element parser | Pure parsers |
| `crates/konnect-core/src/simulation/raw.rs` | Raw file parser | `specctra_ses.rs` |
| `crates/konnect-core/src/simulation/measure.rs` | Measurements | Pure functions, tests beside the code |
| `crates/konnect-core/src/simulation/worker_client.rs` | Spawn, supervise, classify | `freerouting_mcp.rs` |
| `crates/konnect-core/src/simulation/runs.rs` | Run store, retention, id validation | `validate_transaction_id` |
| `crates/konnect-core/src/simulation/plot.rs` | SVG plot | New |
| `crates/konnect/src/sim_worker.rs` | Hidden worker: `libloading`, FFI, callbacks, rights drop | `transaction_cli.rs`, `ffi.rs` |
| `crates/konnect/tests/simulation_live.rs` | Ignored live tests | `tests/e2e_kicad.rs` |
| `crates/konnect-core/tests/fixtures/simulation/` | Real KiCad and ngspice fixtures with `<name>.README.md` | Fixture convention |
| `crates/konnect/assets/skills/kicad-simulation/SKILL.md` | Workflow skill and stand-ins | Existing skills |

**Change**

| File | Change | Pattern |
|---|---|---|
| `crates/konnect-core/src/lib.rs`, `src/tools/mod.rs` | Module declarations, `OnceLock` on `ToolContext` | Guidance store |
| `crates/konnect-core/src/mcp/handler.rs` | Setter for the simulation runtime | Guidance setter |
| `crates/konnect/src/main.rs` | `sim-worker` in `SUBCOMMANDS`, dispatch, call the setter | `transaction` |
| `crates/konnect/src/config.rs` | `ngspice_library`, `ngspice_code_models_dir`, `ngspice_model_dirs`, `ngspice_allow_digital_models` (default true) | `kicad_cli`; blank means default as `jlcpcb_db_path` does |
| `crates/konnect-core/src/router/registry.rs` | `ToolsetMeta { name: "simulation", category: "simulation", tool_count, .. }`, `build_tools_for` arm | `sch_export`. A missing arm panics |
| `crates/konnect-core/src/tools/cli.rs` | `export_spice_netlist`, `.stdin(Stdio::null())` in `run_cli_captured` | `verify_nonempty_file` |
| `crates/konnect-core/src/tools/sch_export.rs` | `owning_project_root` to `pub(crate)` | Existing helper |
| `crates/konnect-core/src/kicad_install.rs` | Library and code-model discovery | `find_cli`; order: config key, sibling of the resolved `kicad-cli`, KiCad install locations, system loader. On macOS map `Contents/MacOS` to `Contents/PlugIns/sim` |
| `crates/konnect-core/src/runtime_info.rs` | `ngspice` block from `--probe` | `kicad` block; same resolver as the tools |
| `crates/konnect-core/src/mcp/error.rs` | `SimulationFailed`, `short_code()` arm | Existing kinds, plus the `DEV.md` table |
| `crates/konnect-core/src/outcome.rs`, `docs/reliability-legacy-inventory.json` | Adopt `run_simulation`, `measure_simulation_run` | Must change together |
| `Cargo.toml`, `crates/konnect/Cargo.toml`, `Cargo.lock` | `libloading` (ISC) in `konnect` only; `libc` on Unix for `setrlimit`; `windows-sys` on Windows for the job object. Both of the latter are already in `Cargo.lock` but are not direct dependencies of `konnect` [verified: source E64] | `[workspace.dependencies]`; `cfg` target tables as `konnect-core` uses for `winreg` |
| `tool-directory.md` | New section, heading, rows; hand-edit the category count at line 15 | Existing rows; xtask cannot create these |
| `crates/konnect/assets/skills/konnect/SKILL.md`, `crates/konnect/src/manifest.rs`, `crates/konnect/src/install.rs`, `README.md` | Toolset row, `SKILLS` entry, install-count test, skill count | Existing entries |
| `plugin/settings_dialog.py`, `docs/KICAD_INTEGRATION.md`, `docs/TROUBLESHOOTING.md` | New keys, client timeout advice | `kicad_cli` entries |
| `.github/workflows/e2e-kicad.yml`, `docs/TESTING_AND_RELEASE.md` | New ignored-test step, coverage map | Existing steps |

Run `cargo xtask fix-doc-counts` in every PR that registers a tool, not once at the end. It updates totals in the README, `DEV.md`, `tool-directory.md`, packaging metadata and others, but it does not set `tool_count` or add rows.

## 4. PR series

Each PR has one reviewable outcome.

| # | Title | Outcome | Done when |
|---|---|---|---|
| 1 | `feat(simulation): report whether a simulator is available` | Discovery, config keys, `libloading`, `sim-worker --probe`, `get_installation_info` block, toolset with the readiness `engine` check | Readiness reports a probed version, or the locations searched |
| 2 | `feat(simulation): export a netlist from a snapshot` | Snapshot, both exports, net map, name port | Readiness returns `nets` and `elements` on real fixtures, and the project folder is untouched |
| 3 | `feat(simulation): flatten and vet decks` | Security boundary alone | Every form in section 6 is refused or removed, and the invariant test has a negative control |
| 4 | `feat(simulation): preflight checks` | All findings | Readiness is complete on real fixtures |
| 5 | `feat(simulation): parse ngspice raw files` | Parser with real fixtures, no tool surface | Binary, ASCII, real, complex, partial all parse |
| 6 | `feat(simulation): run an isolated operating point` | Worker run mode, classification, `run_simulation`, error kind, envelope, run store with retention, starter skill | **Smallest useful slice.** Real operating point; a crash leaves the server alive |
| 7 | `feat(simulation): transient analysis and measurements` | Transient, partial results, measurements, envelopes | **First slice that answers inrush and load-step questions** |
| 8 | `feat(simulation): protection scenarios` | Tested stand-ins in the skill, no new tool surface | Short, I²t fuse, eFuse limit, UVLO, reversed battery pass live. Reversed battery runs as a transient ramp: the sweep form waits for PR 9 |
| 9 | `feat(simulation): AC and DC sweeps` | Both analyses, AC measurements, `no_ac_stimulus` | Margins match an analytic fixture. The reversed battery stand-in also runs as a `dc_sweep` from -12 to 12 V and matches its ramp |
| 10 | `feat(simulation): stored runs` | `list_simulation_runs`, `measure_simulation_run`, CSV | New measurement needs no new simulation |
| 11 | `feat(simulation): SVG plots` | `plot_simulation_run` | SVG on disk, with a banner on partial or modified runs |
| Any | `fix(schematic): copy Sim fields on placement` | `add_schematic_component` copies `Sim.*` as hidden instance fields. Needed: KiCad reads instance fields only, so a placed diode and source export as `REF __REF` [verified: run E74] | A placed diode simulates as it does when placed in KiCad |

**v1 is PRs 1 to 11, plus the placement fix.**

**Scope ranking** for low-voltage power distribution and analog work:

| Analysis | Value | Cost | Why |
|---|---|---|---|
| Transient | Highest | Medium | Inrush, load steps, protection timing |
| Operating point | High | Low | Proves the pipeline, checks rails and bias |
| AC | High for analog | Low once complex data parses | Bandwidth, stability margins |
| Parameter sweeps, corners | High | Medium | Tolerance on inrush; first after v1 |
| DC sweep | Medium | Low | Transfer curves, reverse polarity |
| Noise, TF, pole-zero, sensitivity, FFT | Medium | Low to medium | New result shapes |
| S-parameters | Low here | Medium | Needs port setup |

**`Sim.*` tool.** v1 reports unmodelled symbols and adds no assignment tool. `edit_schematic_component` already writes arbitrary fields, including `Sim.*`, and `get_schematic_component` reads them [verified: source E45]. `testbench` handles boundary parts without touching the schematic. Readiness returns the library's own values when an instance lacks them.

### Rules for every PR

Recorded here so every implementation session follows them.

- **Toolchain, installed at PR 1 and not before.** Nothing in this document needs Rust. `rust-toolchain.toml` pins 1.96.0 with clippy and rustfmt; `protoc` is required by `konnect-ipc/build.rs`, and `cmake` by `nng-sys` and `aws-lc-sys`. When PR 1 starts:

  ```bash
  HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALLED_DEPENDENTS_CHECK=1 brew install rustup cargo-deny protobuf cmake
  brew link --force rustup
  rustup toolchain install 1.96.0 --profile minimal -c clippy -c rustfmt
  rustup default 1.96.0
  ```

  The two flags stop Homebrew from upgrading Python and Node as a side effect. `rustup` is keg-only, so the link is what puts `cargo` on PATH without shell-profile edits; it is untested on this Mac, and `brew unlink rustup` undoes it. The `default` line makes `cargo` work outside the checkout too, where `rust-toolchain.toml` does not apply. Budget about 10 GB for the toolchain and `target/`.
- **Baseline first.** Before any code change, run on `main` what `ci.yml` runs: `cargo test --workspace --locked --lib --tests`, `cargo test --workspace --locked --doc`, `cargo clippy --workspace --locked --all-targets -- -D warnings`, `cargo deny check licenses`, `cargo fmt --all -- --check`. Nothing yet shows `main` is green on this Mac. A red baseline is reported, not fixed inside a feature PR.
- **Docs land with the tool.** Every PR that registers a tool also lands its `tool-directory.md` row, the toolset's `tool_count`, and the output of `cargo xtask fix-doc-counts`. `tool_directory_lists_every_registered_tool` and `registry_tool_counts_match_reality` fail at the first registration, so this cannot wait for PR 11.
- **The installed Konnect is never touched.** It is `~/.local/share/konnect/bin/konnect`, version 0.13.0, SHA-256 starting `3de188bb775ab4ce`, registered in `~/.claude.json`. Every PR's verification re-checks the hash and the entry. Never call `reload_server`: `exec_reload` in `main.rs` re-execs the validated installed binary, so it can never serve the fork, and the restart drops the live toolsets.
- **Dogfood the fork by pipe.** After committing, build with `cargo build --release -p konnect`, then run `target/release/konnect` with `HOME=$PWD/target/dogfood-home` and feed it `initialize` and a `tools/call` of `get_installation_info` on stdin. Trust a result only when `runtime.executable_path` is the fork binary and `build.commit` is the fork's HEAD; a local build records it from `.git` through `build.rs`.

## 5. Risks and open questions

### Decisions

| ID | Decision | Recommendation |
|---|---|---|
| **D1** | Where this lands, since the AGPL repo is archived and its successor is closed source | **Decided:** your own AGPL-3.0 fork at `c7e87cd8a`. Keep `deny.toml` unchanged so the code stays portable. Optionally file the summary as a feature request on the Konnect-EDA tracker. Distributing a fork, or offering it to outside users over a network, carries AGPL duties |
| **D2** | Digital XSPICE models | **Decided: on.** `Class-D`, the `boost` supply and `up-down-counter` then simulate [verified: run E72]. `d_process` and `d_cosim` are refused by the deck scan, because they can start programs or load libraries. `ngspice_allow_digital_models` can turn all digital models off, and no tool can change it. Remaining risk: R4 and Q16 |
| **D3** | Fidelity on real boards. On a real power distribution board, 102 of 116 elements had no model, mostly connectors, fuses and jumpers [verified: run E75], and many vendor models are encrypted. Results then depend on excluded boundary parts and behavioural stand-ins | **Decided:** accept for v1, with `simulated_circuit: "modified"` on every such result. Typed fault helpers can come later if free text proves limiting |

### Risks

| ID | Risk | Mitigation |
|---|---|---|
| R1 | A fork drifts with no upstream | Keep `simulation/` free of Konnect internals, so it can move to a standalone server, which would also work beside Konnect-EDA |
| R2 | Models unavailable or encrypted [verified: run E32, source E46] | Detect and say so. Never guess |
| R3 | ngspice varies by platform, from 42 to 47, with behaviour changes [verified: source E8, E46] | Report the runtime version in every result; `pulse_period_defaulted` |
| R4 | Vetting and hardening are not a sandbox. Three review rounds found nine routes past the flattener between them [verified: run E82, E83], so a further route is likely. An unknown route would run with the user's rights, and Linux lacks the second barrier in v1. With digital models on, only the deck scan stops `d_cosim`, which can load any library it can name. The scan now has two layers: the two names anywhere, and a literal `.model` type | Test the known list, keep the flat-deck invariant, consider a full OS sandbox later |
| R5 | stdio is serial with no cancellation, so a long run blocks every other call [verified: source E47] | Default budget 40 s. A job-and-poll design has no precedent here and is out of scope |
| R6 | Client deadlines differ, and a lost response hides `run_id` [verified: source E59] | Budget below 60 s, `list_simulation_runs`, troubleshooting note |
| R7 | Library-only engine has no route on some Linux setups [verified: source E8] | Clear `engine_not_found` and a config key. CLI backend is the planned fallback |
| R8 | `REF __REF` is a KiCad internal, not a contract [verified: source E4] | Fixtures per KiCad version, plus `net_map_mismatch` at run time, because exporter rules change within 10.0 [verified: run E71]. An unknown form still fails at parse |
| R9 | Results use saved files only, and schematic IPC has no save command [verified: source E39] | Disclosed in `source_evidence` |
| R10 | Stand-ins read as more accurate than they are | `simulated_circuit`, approximations list, and the skill must relay them |
| R11 | A library crash loses in-memory data, because `write` runs after the analysis [verified: source E11] | Timeouts are soft-halted, so this affects crashes only. Reported as `engine_crashed` |

### Open questions, each [assumed] until settled

| ID | Question | How it gets settled |
|---|---|---|
| Q1 | Does Windows KiCad provide `ngspice.dll` and `lib\ngspice\*.cm`, and does `LoadLibrary` find dependencies? If not, does the official DLL archive work? | Windows e2e, PR 1 |
| Q2 | Minimum ngspice version. `sim_status` exists at 42 [verified: source E15], but 42 lacks some exports [verified: source E11] | Live test against Ubuntu's 42 |
| Q3 | Do S-parameters and distortion run through this path? | Their PRs |
| Q4 | Does a hardened Konnect build still load KiCad's library? | Only if releases become hardened |
| Q5 | How do embedded simulation libraries behave in a snapshot? | Scratch project |
| Q6 | Can the host load a Flatpak `libngspice`? | Linux test |
| Q7 | Does a Windows job object block `shell`? What should Linux use, given threads count against `RLIMIT_NPROC`? | PR 6 |
| Q8 | Which real vendor models need sections or other unsupported forms? | Sample vendor libraries in PR 3 |
| Q9 | Actual tool-call deadlines in Claude Desktop, Claude Code and others | Observation |
| Q10 | Is ngspice acceptable as an engine under tracker #590's review idea? | Moot: D1 chose a fork |
| Q11 | Licence terms of a stroke font for PNG output | Before any PNG PR |
| Q12 | Settled. Rise and overshoot match closed forms. Settling needed one change: a share of the final value alone is no band when the final value is zero, so the band is a share of the final value or the step, whichever is larger [verified: run E93] | Done |
| Q14 | Does `kicad-cli` on Windows behave as it does on macOS inside a snapshot? macOS exited 0 in every lock state tested [verified: run E67]; the viewer's Windows notes differ [verified: source E48] | Windows e2e, PR 2 |
| Q15 | Do real vendor sets with their own nested includes flatten correctly? Tested only on a real library split into nested files [verified: run E76] | Sample vendor libraries in PR 3 |
| Q16 | Settled, by removing the dependency. String expressions can only reach a `.model` type or name field, and both must now be literal. Tested forms in E79, with a negative control for each | Closed in PR 3 |
| Q17 | Decided for v1, as an accepted limit: `filesource`, `d_source`, `d_state`, `table2d` and `table3d` read a data file named in the deck. They run no code, and refusing them would break legal XSPICE stimulus. A deck can therefore read a numeric file it can name [verified: run E83]. Revisit with an OS sandbox | Accepted, R4 |
| Q20 | Settled. The worker runs with a cleared environment, in the C locale. Every non-ASCII byte in a name becomes `_`, so `/\u05e9` reads back as `v(/__)` [verified: run E86, E88]. `run_simulation` maps a node back to the KiCad name when exactly one net makes it | Done |
| Q18 | Settled. An auto-bridge file in the project folder is not read: the worker's working folder is the run folder, and `sourcepath` is refused as an option. The same file in the run folder is read, which proves the test can see the difference [verified: run E88] | Done |
| Q19 | Settled. `ngCM_Input_Path` set to the project lets code models find relative data files, and without it they stop with a fatal error [verified: run E88] | Done |
| Q13 | Does FFT computed from a stored transient match KiCad's `linearize` and `fft` result? | FFT PR |

## 6. Test plan

**Hosted CI: no KiCad, no ngspice.** Runs in `cargo test --workspace --locked --lib --tests` on all three OSes.

- **Raw parser:** binary and ASCII, real and complex, op, tran, AC, DC, partial, truncated, garbage. Fixtures from real ngspice with provenance READMEs.
- **Preflight:** real `kicad-cli` netlists. One test per finding, each with a negative control: neuter the guard and watch the test fail, per `GOVERNANCE.md`.
- **Flatten and vetting:** tab, form feed, vertical tab, mixed case, `.controlx`, trailing text, `.if` and `.subckt` bodies, `.incx`, `.libx`, `.lib` sections per mode, nested includes, cycles, unresolved includes, `$VAR`, `~/`, paths outside allowed roots, `*#` in each position, `.title *ng_script`, `.end` mid-file, Latin-1 comments, a `$CDNENCSTART` sample shaped like the public library in E32, and `d_process` and `d_cosim` in any case, on a continuation line and through a string `.param`. Plus the final invariant.
- **Net names:** every `ConvertToSpiceMarkup` rule at tag 10.0.1, including the overbar, subscript, bus and collision cases from E69.
- **Measurements:** expected values computed independently of the code: RC charge, RLC overshoot and settling, a three-pole loop, I²t against the analytic value, and the naive-versus-weighted RMS regression.
- **Classification:** canned `status.json` and logs for every row, per analysis kind.
- **Worker client:** a fake worker injected through the runtime, covering crash by signal, hang until the hard kill, no status file, garbage output.
- **Export:** an argv-scanning fake, as in `pcb_sync.rs`. Asserts null stdin, verified artifacts, and that nothing is written to the project, using a read-only project folder.
- **Run store:** id validation, traversal attempts, retention.
- **Served dispatch:** partial and error results through `McpHandler`, as `RELIABILITY_CONTRACT.md` requires. One test asserts `analysis.stop_time_seconds` is named for a transient with no stop time.
- **Catalogue gates** apply automatically: `registry_tool_counts_match_reality`, `tool_directory_lists_every_registered_tool`, `every_declared_parameter_is_read_by_its_registered_handler`, `calling_any_tool_with_no_arguments_names_its_first_required_one`, `doc_tool_counts`.

**Ignored live tests,** `crates/konnect/tests/simulation_live.rs`, `#[ignore = "needs KiCad 10 with libngspice"]`, through `env!("CARGO_BIN_EXE_konnect")`:

- Operating point, transient and AC on KiCad's `demos/simulation`, including `rectifier` and `amplifier-ac`. `sallen_key` checks that its `.control` block is dropped and reported. `Q17ng` must return `netlist_export_failed`, because KiCad's own library parser rejects it at 10.0.1 [verified: run E65].
- Flattening: flattened and included decks match on the KiCad demos, as in E76. `v_i_sources` is left out because it uses random sources and differs from itself between runs.
- Worker crash: drive `sim-worker` directly with a `.tran 1n 10` deck, then confirm the server answers its next call.
- Non-convergence with partial data, using the behavioural-source deck from E14.
- Runaway: the hard-edged stand-in from E57 halts softly and keeps data; a stuck worker is hard-killed and leaves no process.
- Rights drop: a hand-built deck bypassing Konnect's vetting cannot start a process on macOS.
- Power scenarios: soft-start inrush and load step against E31, and the protection scenarios against `protection/reference.py`, which stands in for E56 [verified: run E97].

**`e2e-kicad.yml`.** Windows, KiCad 10.0.5 through Chocolatey. If Q1 holds, no ngspice install is needed, and the change is one more `cargo test -p konnect --test simulation_live -- --ignored` step. This job gates releases, so a failure blocks publication [verified: source E49]. If Q1 fails, the Chocolatey `ngspice` package is not enough, because it appears to carry the program and not the DLL [verified: source E8]. The fallback is a pinned download of the official DLL archive [assumed]. Linux is not covered by this job. A manual workflow installing Ubuntu's `libngspice0` and `ngspice` gives a version 42 lower bound for Q2.

**Nix.** Stays hermetic: no new required environment variable, live tests stay ignored [verified: source E43].

**Unobserved environments,** named as validation debt: Linux distributions, Flatpak, AppImage, Intel macOS, ngspice 45 to 47.

## Appendix A. Settled questions

**1. Netlist.** `--format spice` emits `.title`, includes, models, directives, elements, `.end`. The CLI passes only `OPTION_SIM_COMMAND`, so it adds no `.save` or `.probe` lines [verified: source E4]. Analysis lines in schematic text reach the file [verified: run E68]. The workbook is never read, and neither is `model_mode` [verified: source E5]. `Sim.Library` paths resolve through `${KIPRJMOD}` to absolute paths; raw `lib=` values stay unresolved; a missing library gives exit 2 but still writes a file. Library includes appear as quoted absolute paths, including KiCad's own `Simulation_SPICE.sp` [verified: run E65, source E6]. Symbols without a model produce `REF __REF` silently [verified: run E2]. Only two-pin R, L, C, V, I parts are inferred from their value [verified: source E4]. There is no native simulation command [verified: run E1].

**2. Engine.** Worker process plus runtime-loaded libngspice. Extra install: none on macOS, likely none on Windows, see the install table. Single binary: unchanged, because the library is loaded at run time from the user's KiCad; the release build is one self-contained executable, though not strictly static on Linux [verified: source E60]. Isolation: full, by process. Discovery: config key, sibling of the resolved `kicad-cli`, install locations, system loader; blank means default, as `jlcpcb_db_path` does. Licensing: `libloading` is ISC and on the `deny.toml` allow list [verified: run E61, source E37], and nothing from ngspice ships, so its redistribution terms are not triggered [verified: source E36]. Engineering reading, not legal advice.

**3. Scope.** See section 4. Smallest useful slice is PR 6; first slice for the first user is PR 7; protection is PR 8. No `Sim.*` tool in v1.

**4. Results.** Compact JSON, measurements in Rust, full data on disk. Plots are SVG files by path in v1. Inline images later, on request only. `konnect-render` rasterises SVG but refuses visible text and has no plotting code [verified: source E41].

**5. Failures.** One new error kind, plus `outcome` on every result. See the matrix in section 2.3.

**6. Testing.** See section 6.

## Appendix B. Evidence

| ID | How | Result |
|---|---|---|
| E1 | Ran `kicad-cli` help; read KiCad `api/proto` | `sch {erc,export,upgrade}`. No sim in the schematic proto |
| E2 | Ran `sch export netlist --format spice` on a real schematic | rc 0, empty stderr, `Q2 __Q2`, `M1 __M1`, `J1 __J1`, `VBT1 Net-_BT1-+_ GND DC Battery` |
| E3 | Ran those lines through libngspice | `could not find a valid modelname`, `unknown parameter (battery)`, `Error: circuit not parsed.`, rc 0 from both calls |
| E4 | Read `netlist_exporter_spice.cpp`, `spice_generator.cpp`, `sim_model.cpp`, `eeschema_jobs_handler.cpp` | Options, directive filter, `__REF` origin, inference rules, exit semantics |
| E5 | Read `simulator_frame_ui.cpp`, `spice_settings.*`, `inpcompat.c`, `inpcom.c` | `.wbk` is JSON and GUI only. `model_mode` enum 0 to 5, default LT_PSPICE. In `lt` or `ps`, `.lib` becomes `.inc` |
| E6 | Read `sim_lib_mgr.cpp`, `filename_resolver.cpp` | Library resolution order |
| E7 | Ran `find`, `otool`, `codesign`, `nm`; loaded the library | `PlugIns/sim/libngspice.0.dylib`, six `.cm` files, no executable, no spinit, banner ngspice-44.2 |
| E8 | Read build scripts and package indexes, not executed | Library-only on most platforms; versions 42 to 47; `libngspice0` lacks `.cm`; Chocolatey package appears to lack the DLL |
| E9 | Loaded the library from an ad-hoc-signed host; `codesign` on the installed Konnect | Same signing state: ad-hoc, no hardened runtime |
| E10 | Ran decks under `bg_run` | `.tran 1n 10` and `1n 2.2` exit 139; `1n 1` and `.tran 5m 10 0 1n` survive; blocking `run` hits the exit callback |
| E11 | Read `sharedspice.c/h`, `ngspice.cpp`, `outitf.c` | `bg_halt` cooperative; raw `exit` and `abort` remain; KiCad traps SIGSEGV; no variable getter; some exports absent at 42; KiCad calls `ngCM_Input_Path` |
| E12 | Ran a missing `.include` | Exit callback status 1, rc 1, `cannot recover and awaits to be reset or detached` |
| E13 | Ran a voltage loop `.op` | Plot exists with zero-length vectors; `run simulation(s) aborted`; `sim_status=1` |
| E14 | Ran a mid-run failure | 1028 points, reached 1 ms of 3 ms, `trouble with node "x"`, `sim_status=1` |
| E15 | Ran each state; read `runcoms.c` at tag 42 | Undefined if never parsed, 0 ok, 1 failed, 0 when halted; present at 42 |
| E16 | Ran a PGND-only circuit | rc 0, a=3.003, pgnd=-8.997; only singular-matrix warnings |
| E17 | Ran case-differing nets; read `ConvertToSpiceMarkup` | Merged. `% ( ) , [ ] < > ~` and space become `_`; sub and superscripts flatten; `//` rewritten. The `/0` and `/gnd` rewrites exist only after tag 10.0.1 |
| E18 | Ran `.control` with `shell` | Executes during `ngSpice_Circ`; `/bin/sh` invoked |
| E19 | Ran variants; read `inp.c` | Runs from `.include`, `.lib` section, `.if`, indented, tab, FF, VT, mixed case, trailing text, `.controlx`. First-line `*ng_script` runs. NBSP gives a fatal UTF-8 error |
| E20 | Ran `*#` forms; GitHub code search; read `inp.c:778` | Runs at top level, indented, in `.subckt`, in includes. `*#####` harmless. 387 public `.lib` hits for `*####` |
| E21 | Ran `* #`, BOM, NBSP, `;` forms | Not executed |
| E22 | Ran `alias` then `unalias` | Alias blocks; `unalias` restores `shell` |
| E23 | Read `sharedspice.c:960-1023` | Working-directory `.spiceinit`, else passwd home; `no_spiceinit` honoured only in the non-POSIX branch |
| E24 | Ran `bg_run`, `bg_halt`, `write`, `bg_resume` | Partial vectors kept; write works; resume works; `tran: 54.7%` from SendStat |
| E25 | Ran 2M points | 8.1 bytes per point per vector; about 700k points per second |
| E26 | Ran `.meas` | Stdout text, 6 to 7 digits, no vectors; failure on stderr; rc 0 |
| E27 | Ran `.meas MAX i(V1)` on an inrush | Returned 0 on a roughly 240 A event; delivering source reads negative |
| E28 | Ran an inrush; computed by hand | Naive RMS 5.30, weighted 2.69; I²t 1.444 against analytic 1.439; `.meas RMS` matched trapezoid on a second deck |
| E29 | Ran a three-pole AC deck | fc 30145.3 Hz, PM 1.77 degrees, matching analytic; f-3dB 100.0 Hz |
| E30 | Ran `write` in ASCII and binary | Header format; `v(/vout)`, `i(r1)`, `i(m1:d)`, `m1:power` |
| E31 | Ran a MOSFET soft-start and load step | 7053 points; 16.56 A peak; I²t 0.179; 96 mV droop; about 620 KB as JSON. Quoted Hebrew path include works, unquoted fails |
| E32 | Ran `set ngbehavior=ltpsa`; ran a public TI library | `Compatibility modes selected: ps lt a`. `$CDNENCSTART` file: `Missing token`, exit callback |
| E33 | Accidentally: `kicad-cli | head`; read `lockfile.h`, `settings_manager.cpp` | SIGPIPE left `~<project>.kicad_pro.lck`; destructor removes it on clean exit |
| E34 | Read `sim_types.h`, `dialog_sim_command.cpp`, `simulator_frame_ui.cpp`, `simulator_control.cpp`; ran XSPICE with and without `.cm` | 11 analysis types, 8 measurement actions, tuners, exports. XSPICE fails without code models, works after `codemodel` |
| E35 | Ran `.noise`, `.tf`, `.pz`, `.sens`, `linearize` plus `fft`, `alter` | All completed with `sim_status=0` or rc 0 |
| E36 | Read ngspice `COPYING` | Modified BSD plus LGPL, MPL and GPL parts (`table.cm`) |
| E37 | Read crates.io and repo metadata | `libloading` ISC, 0.9.0, maintained. Existing bindings stale |
| E38 | GitHub API; read README | `archived: true`; Konnect-EDA commercial, closed source |
| E39 | Read `sch_export.rs`, `tools/mod.rs`, `pcb_sync.rs`, `board_source.rs` | Ownership check, lock handling, source evidence |
| E40 | Read `tools/mod.rs`, `router/mod.rs`, `footprint_graphics.rs` | Schema closure, Draft 2020-12, `oneOf` precedent |
| E41 | Read `konnect-render/src/lib.rs` | `svg_to_png` refuses visible text; no plotting crate |
| E42 | Read `outcome.rs`, `mcp/error.rs`, `sch_components.rs` | Four statuses; `Failed` attached to refusals; count assertion; no process or timeout kind |
| E43 | Read `DEV.md:55-61`, `xtask`, `doc_tool_counts.rs` | Workspace-member and env-var hazard; doc gates |
| E44 | Searched the tree | No `unsafe` in `konnect-core`; no lints forbid it |
| E45 | Read `sch_components.rs`, `sch_batch.rs`, `sim_lib_mgr.cpp` | Arbitrary field writes; five properties on placement; KiCad reads instance fields |
| E46 | Read forum, vendor and commit sources | Encrypted models unreadable; `PULSE` default change at 46 |
| E47 | Read `transport/stdio.rs`, `handler.rs` | Serial loop; `notifications/cancelled` unhandled |
| E48 | Read `crates/schematic-viewer/src/main.rs` | Snapshot pattern; exit -1 after a good export with a foreign lock; Windows sharing violation |
| E49 | Read workflows | Windows-only e2e; release gate |
| E50 | Ran `nm -u` on each `.cm`; ran `d_cosim` and `d_process` decks with nonexistent paths; read `cfunc.mod` | Only `digital.cm` imports `fork`, `execv`, `dlopen`. `d_cosim` called `dlopen` at run. Without `digital.cm`: `Unknown model type d_process`. `d_process` exec reached by source and symbol only: the test deck did not get past port syntax |
| E51 | Ran `.title *ng_script` then a bare `echo`; read `inpcom.c` | The `echo` ran as a command. No effect from inside an included file |
| E52 | Ran `.incx`, and `.libx` under `ltpsa`, with a harmless `echo` block; read `inpcom.c` | Both included the file and ran the block |
| E53 | Ran with `RLIMIT_NPROC` set to 0 | `shell` failed with `Unable to execute`; `bg_run` and analysis still worked |
| E54 | Ran `.ac` with a DC-only source | `sim_status=0`, 11 points, maximum magnitude 0.0 |
| E55 | Ran `.dc V1 2 0 -0.5` | Falling sweep works |
| E56 | Ran stand-ins | Fuse: 343 A peak, opened at 5.0275 ms for a 5 ms short, 2.06 A²s. eFuse: 5.0 A. UVLO: 10.005 and 9.001 V. Reverse: -1 µA, 11.59 V at 12 V |
| E57 | Ran a hard-edged stand-in | Blocking `run` exceeded 2 minutes at 500 MB; `bg_halt` at 4 s kept 173,841 points to 5.18 of 20 ms, `sim_status=0` |
| E58 | Searched `demos/simulation` at tag 10.0.1 | `.control` in `sallen_key`, `Class-D`, `smps-com`, `royer1`, `Q17ng`: 5 of 18 |
| E59 | Review read the MCP TypeScript SDK | `DEFAULT_REQUEST_TIMEOUT_MSEC = 60_000`; no progress in Konnect |
| E60 | Review read Konnect source | Guidance `OnceLock` pattern, `validate_transaction_id`, state-root logic, dangling `$ref` panic path, oneOf error selection, parameter-usage scan depth, `design_hash` set, stdio rule for children, schema catalogue cost, gnu Linux target |
| E61 | Read `deny.toml` | Allow list includes ISC; no GPL or LGPL |
| E62 | Review read `Device.kicad_symdir`, `Simulation_SPICE.sp` | Varistor default threshold 1 kV; Fuse and TVS have no `Sim.` fields |
| E63 | Ran a mistyped source node and a `.dc` on an absent source | Mistyped node: `sim_status=0`, no warnings, `v(out)` 0.0. Absent target: `no such device`, `sim_status=1` |
| E77 | Ran `d_cosim` decks with a nonexistent library path, and a review of model-type routes in ngspice 44.2 source and runs | Reached `dlopen`: literal, uppercase, continuation line, string `.param`. Blocked: bare `.param`, `{a}{b}`, subcircuit parameter, `.func`, split tokens, `\\` continuation. Type names match whole tokens by `strcasecmp`; PSpice U-device translation emits fixed types only. The parameter-string review did not complete |
| E78 | Ran `ngSpice_nospinit` and `ngSpice_nospiceinit` before init from Python and from the worker; ran `SPICE_SCRIPTS` pointing at folders with and without a `spinit` | Both calls exit 139 on ngspice-44.2. `SPICE_SCRIPTS` decides which `spinit` loads |
| E79 | Ran `d_cosim` decks with a nonexistent library path through every form of `.model` field; read `inpdomod.c` | Reached `dlopen`: literal, `.model m {t}(...)` with a string `.param`, `.model m 't'(...)`, `.modelx`, and `\\` continuation, which corrects E77. Blocked by ngspice: `{ t }` spaced, `{t}` as the name, `.model m={t}`, `.model m(d_cosim ...)`. The type is the token after the name, read by `INPgetTok`. Vetting refuses every reached form |
| E80 | Read `inpcom.c` at 44.2; ran a deck with a bare `@` line | `@` first on a line ends reading: the run gave `sim_status=1` and no data. `readline` drops every `\r`. Lines first in `#`, `$` or `*` are skipped by stitching. `+` and `\\` both join with a space, so continuation cannot rebuild a split word |
| E81 | Ran `check_simulation_readiness` over all 18 demos at tag 10.0.1; a real export with text-box includes | 17 pass vetting and `Q17ng` stops at its known export failure. `ibis` inlines KiCad's per-user cache, which `kicad-cli` writes under the real home even with `HOME` changed. A text `.include` stays relative and `${KIPRJMOD}` stays literal in the netlist |
| E82 | Independent review by three reviewers of the first flattener, each form run through KiCad's libngspice 44.2 with harmless markers and a nonexistent library path; read `inpcom.c`, `inpcompat.c`, `inpgtok.c`, `inp2dot.c`, `simulator_frame.cpp` | Bypassed it: `r#...` in an unused subcircuit under `ltpsa` (`comment_out_unused_subckt_models` writes `*` over the first character, after `pspice_compat` has already rewritten `*#`); `;#`, `=#`, `(#` and others in modes without `ps` (`inp_check_syntax`); a no-break space before `#` or `$`; `.model m;x D` then `+ {t}(...)`, where the comment hides the type; `"d_co""sim"` joining into a string. Wrong results: `.end,x` stopped the circuit silently; `.tran,1u,1m` and `.op;x` ran. False refusals on real libraries: `ako:` cards and `.model D9_Empty D;`. KiCad forces `LT_PSPICE` while the workbook name is empty. All are fixed or named as limits, each with a test |
| E83 | Rounds two and three of the same review, each form run through libngspice 44.2 | Reached `d_cosim`: `.cmodel m {t}(...)`, `.model m mfg=x {t}(...)` in `lt` and `ps` modes, and `.model m g()(...)` with `.func g() {d_co sim}`. `.options measoutfile=<path>` wrote a file. Silent circuit differences: a `.end` before the last card, a `;` line inside a continuation, a first netlist line that ngspice reads as the title, include search order, and `\\` around comments, includes and dot-leading values. `\\` appears on 4 lines in 527 real files, so it is refused rather than imitated. Accepted as limits: Q17 and Q18 |
| E84 | Round four: raw against flattened through libngspice 44.2 in modes 1, 4 and 5, on 4,120 generated decks, 363 ngspice example decks and 439 model and subcircuit instance decks | Two differences, both fixed: a form feed before a comment or `+` line in a model file, and `.incpslt` in `ltpsa`, where the PSpice pass runs twice. The reviewer for side effects stopped early on a provider safety filter, so the final version has had no complete independent side-effect review. A source search for `fopen` then found that CIDER models (`numd`, `numd2`, `numos`, `nbjt`, `nbjt2`) write state files to `rootfile`. A `numd` deck wrote them to the path it named, and vetting refuses that deck. Repeated on the final code: 3,669 generated cases and 793 example cases matched. The 17 example differences are all explained: Monte Carlo decks that also differ between two raw runs, one `.meas` log line number, a same-valued listing order, and `table3d` data files that fail loudly from an empty folder (Q19) |
| E85 | Five independent review rounds of preflight, each case run through libngspice 44.2 or a real `kicad-cli` export on a scratch copy; `4k7`, `R47` and `4n7` run by mode | Wrong results it now catches: `100n 50V` read as 50 F, `3V3` as 3 V, `1MOhm` and `1MHz` as milli, `22P` exported as 2.2e+16, labels holding `=`, `.if(` read as live. Wrong refusals removed: series capacitors and isolated windings, which give correct AC and transient results; `.param` names as values; `47R`, `100n_X7R`, `5VDC`; `DC(5)` and `(1k)`; `R = (expr)`; `tc=a,b`. Also a panic on `\u00b0` before a bracket, and a cubic slowdown. A potentiometer counts as no reference. KiCad's 17 exportable demos all report ready. Known gaps left as they are: a body line in a text `.subckt` can match a symbol's name, a call with the wrong pin count fails in the engine with no typed refusal, and values that fail loudly in the engine, such as `DC 9V Battery`, are left to it |
| E86 | Ran Hebrew node names through libngspice 44.2 under the C locale and under UTF-8; read `inp_casefix` | C: `/\u05e9` and `/\u05ea` both became `/__`, one node. UTF-8: only bytes 0x80 to 0x9F changed, so `/\u05d0` and `/\u05d1` still merged. `inp_casefix` replaces any byte `isprint` rejects |
| E87 | Generated six decks through libngspice 44.2 in text and binary, and compared the parser with the engine's in-memory vectors; ran the parser against noise, pz, sens, tf, disto, sp, `.save` and event decks; cut files at every byte and fuzzed them | Every value matched. 328,884 cuts and 600,000 fuzz cases: no panic. A 2 GB binary file parsed in 1.1 s at a 4 GB peak. Traps listed under Reading results |
| E88 | Ran the worker on libngspice 44.2 and `run_simulation` on 13 KiCad demos | 9 ran on the first pass. `class-d` and `up-down-counter` stopped: `.probe alli` is fatal next to any event node (`Evtcheck_nodes`), so those circuits now save voltages and source currents only, and both run. `ibis` needs KiCad's real cache folder and `q17` does not export. A `SIGSEGV` sent from outside is swallowed by Rust's handler and the run carries on, so the live crash test uses `SIGABRT`. `.tran 1n 10` now ends in the exit callback, not a segfault |
| E89 | Two independent review rounds of the run path, each finding reproduced through the worker or the server | Fixed: model text in errors, engine warnings missing from complete results, a panic on a stray folder name, retention failures refusing good runs, the budget not covering the probe and exports, a halt lost before the engine thread starts, and paths passed through engine commands, which ngspice expands. `SPICE_SCRIPTS` over about 500 bytes overran a buffer, so it is now `.`. 26 odd folder names pass. `.ENDSUBCKT` and `.ends,f` close a block, as KiCad's own QA library needs |
| E90 | Polled `ngGet_Vec_Info` from the main thread during `bg_run`, 15 runs and over 24 million calls; read `sharedspice.c` and `outitf.c`; timed `SendData` on a 1,000,006-point run | No crash, but every run printed `Error: vector time not found!` until the plot existed, which would read as an engine warning, and 1 run in 12 missed the vector on all 2,000,001 polls. `vec_get` walks lists the engine's thread is building. `SendData` runs on that thread after each point and reports vector count and point index. It needs `SendInitData` set too. Time 2.38 s with it, 2.40 s without. Halts at 1 and 16 MiB left 1.12 and 1.0036 times the limit, data intact |
| E91 | Ran `.tran stop/2000 stop` at 14 stop times from 1e-15 to 1e12 s, with `max_step` down to 1e-12 and with `tstart` and `uic` | No crash at any. Runs past 1e6 s and tiny steps ran into the budget and halted softly. A `tstart` near the stop time leaves one point. `max_step` above the print step gives a coarse run: 30 points for 1 ms |
| E92 | Ran a mixed-device deck with `.probe alli` and `.probe p()`; a three-pin subcircuit; a `gain` code model; a digital circuit; every KiCad demo as a 1 ms transient | Names: `i(m1:d)`, `i(q1:b)`, `i(x2:a)`, `i(r1)`, `i(vbat)`, `m1:power`, `x1:power`. `i(l1)` and `i(b1)` appear twice with equal data. `.probe p(A1)` is dropped silently, and `.probe p(R1)` works beside digital parts. `.probe` adds hidden elements such as `bprobe_int_r1power`. `.probe alli` gives about 4 vectors per device. The largest demo has 76, which at 256 MiB still holds 441,505 points. `laser_driver` and `v_i_sources` reached the limit for real and stopped softly at 441,507 and 610,720 points. 11 of 14 entries ran; `ibis` needs its cache folder configured, `q17` does not export, and `power_supplies` is a folder of projects |
| E93 | Ran `soft_start.sp` through `run_simulation` at four step sizes, against an RK4 integration of the same equations with no SPICE engine; an RC and an RLC against closed forms | At 0.5 µs: -16.9039 A at 10.8565 ms, I²t 0.24465, rail 11.98087 V, low 11.88536 V, against 16.9040 A at 10.8566 ms, 0.24465, 11.98087, 11.88536. At the default 70 µs every value stays within 1 percent, but the peak lands at 10.21 ms on the flat top of the inrush. A 2 µs step took 1,019,783 points where 0.5 µs took 281,020, so a smaller step is not always a longer run. E31's deck was not kept; this one reads 16.90 A, 0.2447 A²s and 95.5 mV against E31's 16.56 A, 0.179 and 96 mV |
| E94 | Independent review of PR 7, each case run through `run_simulation`, then repeated by the author on the boundary cases; read `inpc_probe.c`, `dctran.c`, `traninit.c` | Fixed: `.probe` moves pin 2 of a two-pin part to a hidden node, so `.ic` under `uic` read 0 V on `C1 0 a` and 1.84 V on `C1 a 0`, and adding `p(C101)` to `buck_conv` moved `v(/VOUT)` from 5.89 to 0.042 V. A device `IC=`, and `.ic` without `uic`, are not affected. Inductor currents failed bit equality, on `amplifier-ac` and `royer`, differing by 4.8e-11 of peak. `uic` saves no point at t=0: a 12 kA inrush read 8 kA, 34 A²s for 72, with no flag. A flat rail with 7e-15 V of noise read a 458 ns rise and 33 percent overshoot. A ramp read settled at 0.98 s, and a pulse ending 1 mV high read a 1.6 ns rise and 499900 percent. RMS of a triangle read 0.5957 for 0.57735. `value_at` at the stop time read null. `p(K1)` stopped the run. A save start past 96 percent of the stop time collapsed the step. Stop times from 1e9 s and a large `max_step` read as `no_convergence`. Left as they are, because each fails loudly: a net named `time` or `frequency`, and `/out#branch` |
| E95 | Second independent review of PR 7, each case run through `run_simulation`; read `cktic.c` | Fixed: `.nodeset` seeds a start through the same vector as `.ic`, so it lost its voltage under `uic` too, and a caller's own `.probe` beat the guard. An RC cut off at 3 time constants read a 16.6 percent short rise and a 31.6 percent short settling time with no flag. `.options interp` read 0 V for a 5 V pulse. `start_save_seconds: 0` silenced `no_point_at_start`. A window from before a late `start_save_seconds` lost its flag, hiding a 12 A peak. A ramp on a rail read settled at 0.74 s. A pulse most of the way back gave a number. Vectors of bare rounding read as noise steps and as two signals. Still accepted: a window ending on a wrong level that the tests cannot see, which `start_value` and `final_value` expose. A `.ic` inside a `.subckt` is refused by preflight, though ngspice applies it. A `.ic` in a dead `.if` drops currents, which fails loudly. A negative control caught no test for an uppercase `.IC`, which now has one |
| E96 | Third independent review of PR 7, each case run through `run_simulation`, on `buck_conv`, `LM317` and `boost` among others | Fixed: the last sample stood in for the final level, so `settling_time` on `buck_conv` read 1.145, 1.365 or 2.390 ms by where the window ended. An inrush back to zero had no band and read `no_transition`. No route to a lost start voltage reproduced, and removing `interp` breaks no demo or shipped library. Accepted: a window ending just past a crest passes the checks, a sag that stays down reads as overshoot, the drift check refuses some finished signals at some window lengths, every `.probe` is refused under `uic` though `v()` and `i()` of some parts hold, and a 500 fA step reads as noise |
| E97 | Rebuilt the five E56 stand-ins as decks, ran each through `run_simulation` at 0.1 to 10 µs steps and under `uic`, and compared with `protection/reference.py`: closed forms, bisection for the nonlinear loads, RK4 at 1 ns for the fuse | Short 480.03998667 A and 2.39920027 V, to every digit. Fuse opens at 5.020535 ms against 5.020498 ms from the reference, passes 399.5497 A at its peak against 399.5503 A, and carries 12 µA after. eFuse 11.97976185 V against 11.97976188 V, 5.000 A into the short. UVLO on at 10.003 V and off at 9.001 V. Reverse -1.0 µA and 11.97000081 V. E56 differed where its decks did: 343 A, 2.06 A²s and 11.59 V came from other resistances and a coarser step. A fuse with no latch settled half open at 1.07 A, because the current falls as the resistance rises. The integral overshoots the trip by up to the current squared times the step: 2.0059 A²s at 0.1 µs and 2.0174 at 0.2 µs, against a trip at 2.002 and a bound of 0.016 and 0.032 above it. All five also run under `uic`, keeping device currents. Net names like `/rect_out` work inside expressions |
| E98 | Compared the engine with the reference on a 1 GΩ off state, then probed `exp()`, `**` and `pow()` in a B source. Rebuilt hard-edged shorts: a vertical PWL edge, a 1 ns edge, a zero-rise pulse and a behavioural step, with and without 1 µH and 100 µF. Ran a lockout behind 0.5 and 3 Ω of source | `exp(x)` is exact to 14 and linear above it: `exp(15)` reads 2.4052e6, twice `exp(14)`, so `exp(ln(1G))` read 9.288e6 and the off state leaked 1.3 µA more than the reference. `**` and `pow()` are exact. Every hard-edged short completed at the default step and at 0.1 µs, so E57's runaway was not reproduced with these. The 0.5 Ω lockout ran and shifted its off point to 18.63 ms. The 3 Ω lockout sagged from 10 V to 8 V, chattered and stopped with `no_convergence` at 10.00000003 ms |
| E64 | Read `Cargo.toml`, `Cargo.lock`, and the `konnect` and `konnect-core` manifests | `dirs` and `sha2` already in `konnect-core`. `libc` 0.2.189 and `windows-sys` 0.61.2 are in the lock, not direct dependencies of `konnect` |
| E65 | Ran both netlist formats on all 18 demos at tag 10.0.1: full copy, snapshot with `Sim.Library` rewrite, snapshot without | Snapshot matched byte for byte on 17; `laser_driver` had the same lines in a different order. Without the rewrite 15 exit 2 and still write a file, with `Simulation model library not found`. `Q17ng` exits 2 either way: KiCad's parser rejects `all_devices.lib` |
| E66 | Ran the `ibis` demo repeatedly and after other projects' exports; read the cache cleanup in `netlist_exporter_spice.cpp` at the tag | Includes `~/Library/Caches/KiCad/10.0/ibis/U1.cache`. Present after its own exports; missing after later exports from other projects |
| E67 | Ran locks on scratch copies: clean run, same-user, foreign, unparseable, sheet lock, SIGKILL | Clean run removes its lock. All lock states exit 0 with a good file; unparseable also prints an error. SIGKILL strands the lock |
| E68 | Added text boxes to a scratch copy of `rectifier` | `.title *ng_script`, `.control`, a `*#` line and prose inside a qualifying box, and a relative `.include` all reach the netlist. `.end` lands before the elements. `*#` alone, `.inc`, `.measure`, `.option` and an indented `.tran` do not pass |
| E69 | Ran a name port against 18 demos and a real 3-sheet board, then 15 stress labels | All 544 node tokens matched. Stress: 13 of 15, covering overbar, subscript, bus and spaces. `0` and `gnd` stay `/0` and `/gnd` at 10.0.1. `A(1)` and `A[1]` merged into `/A_1_` |
| E70 | Started the library in folders with a `.spiceinit`, an empty one, and none | A file with `echo` ran; empty and absent ran nothing. Home-file precedence rests on source E23 |
| E71 | Diffed 18 cited KiCad files between tag 10.0.1 and branch 10.0 | 8 identical. The other 10 were read: cited behaviour holds at the tag except the `/0` and `/gnd` rewrites and model-name uniquifying, both added after it |
| E72 | Ran all demos with and without `digital.cm` | `Class-D`, `smps-com` and `up-down-counter` fail with `MIF-ERROR - unable to find definition of model`. The rest are unaffected |
| E73 | Ran `up-down-counter` `.op` with digital models loaded | `Error: ngspice.dll cannot recover` during the run, `sim_status=0`, plot `op1` with no vectors |
| E74 | Removed instance `Sim.*` fields from a scratch copy of `rectifier`, keeping the library symbol's | `D1 __D1`, `V1 __V1`, rc 0. R and C still inferred |
| E75 | Exported a scratch copy of a real 3-sheet power distribution board | 116 elements, 102 unmodelled: 37 J, 31 F, 12 JP, 8 D, 7 TP, 7 others |
| E76 | Ran a flattening prototype against included decks | 13 demos with no difference across all vectors, 24 model files inlined, 5 `.control` blocks removed. A real library split into nested files also matched. `hv_converter` timed out both ways. `v_i_sources` differs from itself: random sources |
