# Developer Guide — Konnect

Internal reference for developing and maintaining the Rust port.

New to the codebase? Start with the map in
[docs/DEVELOPER_OVERVIEW.md](docs/DEVELOPER_OVERVIEW.md), then return here for
the detailed implementation reference and current statistics.

Repository-wide naming, public API, branch, and pull-request rules live in
[docs/NAMING_CONVENTIONS.md](docs/NAMING_CONVENTIONS.md).

## Quick Start

For tool implementation or review, read
[the reliability contract](docs/RELIABILITY_CONTRACT.md) before choosing argument,
mutation, result, or recovery handling. Reuse the existing structured errors,
document-binding, transaction and readback helpers; document remaining gaps.
The contract also describes the evidence required for a changed behavior and
which catalogue-wide checks are still planned.

```bash
# protoc is required for protobuf code generation. If PROTOC is unset, the
# build falls back to `protoc` on PATH (see Build Requirements below).
set PROTOC=C:\path\to\protoc.exe   # or install via `choco install protoc`

cargo check                          # verify everything compiles (~15s)
cargo test --workspace --lib --tests # all tests
cargo build --release -p konnect # build the MCP server binary

# Build the schematic viewer (separate crate)
cd crates/schematic-viewer
cargo build --release
```

Schematic-viewer build notes (Windows):

- If `cargo` is not recognized in a fresh shell, add it to the session PATH first:
  `set PATH=%PATH%;%USERPROFILE%\.cargo\bin`
- Close any running viewer window before rebuilding — Windows locks a running
  `.exe`, so the link step fails while the app is open.

### Nix

A `flake.nix` provides a reproducible build and dev shell on Linux
(`x86_64`/`aarch64`), contributed in #198:

```bash
nix build .#konnect   # build the server binary
nix run  .#konnect    # build and run it
nix develop           # shell with the pinned toolchain, protoc, kicad-small
```

The flake pins its own toolchain from `rust-toolchain.toml` and reads the
version from the workspace `Cargo.toml`, so those stay in step automatically.
What it does *not* track automatically is the build itself: `cargoBuildFlags`
names `-p konnect --bin konnect`, and `preCheck` exports `KONNECT_STATE_DIR`
for the tests that need a writable state dir. **Renaming the binary, adding a
workspace member the server depends on, or introducing a test that needs
another environment variable will break the Nix build without breaking any
other job** — the `Nix flake` CI job exists to catch exactly that, so treat a
failure there as a real break rather than a Nix-user problem.

## Architecture

```
Konnect/
├── crates/
│   ├── konnect/              # Main binary + cdylib entry points
│   │   └── src/
│   │       ├── main.rs              # CLI: --config, subcommands
│   │       ├── lib.rs               # cdylib re-exports ffi
│   │       ├── ffi.rs               # C ABI: kicad_plugin_init/version/shutdown
│   │       ├── config.rs            # TOML + JSON config, socket path auto-detection
│   │       └── transport/
│   │           ├── stdio.rs         # Line-by-line JSON-RPC over stdin/stdout (default)
│   │           └── http.rs          # Streamable HTTP: POST + GET (SSE) on /mcp (transport = "http" / "both")
│   │
│   ├── konnect-core/          # All tool logic (22 toolsets)
│   │   └── src/
│   │       ├── mcp/
│   │       │   ├── protocol.rs      # MCP JSON-RPC 2.0 types
│   │       │   ├── handler.rs       # Dispatch: initialize, tools/list (all tools static), tools/call
│   │       │   └── server.rs        # Session state machine
│   │       ├── router/
│   │       │   ├── mod.rs           # ToolRouter: load/unload toolsets
│   │       │   ├── registry.rs      # Static toolset metadata + tools_for() dispatcher
│   │       │   └── meta_tools.rs    # 7 baseline meta-tools + Unix stdio reload
│   │       └── tools/
│   │           ├── mod.rs            # ToolDef, ToolContext, tool! macro, helpers, kicad_config_dir()
│   │           ├── board_source.rs   # board_source selector: live-vs-saved board reads + provenance
│   │           ├── live_board.rs     # what KiCad's answer proves about one board, shared by read and write gates
│   │           ├── cli.rs            # kicad-cli v10 subprocess wrapper (verified against actual binary)
│   │           ├── svg_import.rs     # SVG parsing + Bezier flattening for import_svg_logo (usvg-backed)
│   │           ├── project.rs        # 7 tools (incl. open_schematic_viewer)
│   │           ├── sch_components.rs # 18 tools (component placement with lib_symbols embedding)
│   │           ├── sch_wiring.rs     # 20 tools (incl. connect_pins, power symbol embedding)
│   │           ├── sch_analysis.rs   # 15 tools (union-find net graph, connectivity)
│   │           ├── sch_batch.rs      # 12 tools (single-read/single-write atomic operations)
│   │           ├── sch_export.rs     # SVG/PDF/netlist/ERC/PCB sync
│   │           ├── sch_netlist.rs    # hierarchy-wide netlist read + intent compare
│   │           ├── sch_bus.rs        # 4 tools (buses, bus entries, pin fan-out)
│   │           ├── pcb_sync.rs       # update_pcb_from_schematic: pure planner + one-commit IPC apply
│   │           ├── sch_hierarchy.rs  # 14 tools (typed Sheet model, sheet-file creation and page size, sheet CRUD + hierarchy/page queries + pin lifecycle)
│   │           ├── pcb_board.rs      # 11 tools (S-expr file editing, IPC fallback, SVG logo import)
│   │           ├── pcb_components.rs # 20 tools (IPC real-time + safe headless single-placement fallback)
│   │           ├── pcb_footprint_update.rs # library refresh planner + one-commit IPC apply
│   │           ├── pcb_routing.rs    # 15 tools (traces, vias, nets, netclasses, SES import)
│   │           ├── pcb_export.rs     # 14 tools (Gerber, PDF, 3D, Specctra DSN, DRC, DXF/GenCAD/IPC-2581/ODB++)
│   │           ├── library.rs        # 17 tools (symbol/footprint library management)
│   │           ├── footprint_graphics.rs # footprint primitive validation, inspection, and atomic edits
│   │           ├── footprint_metadata.rs # footprint description, tags, and attribute edits
│   │           ├── footprint_models.rs # footprint 3D model validation and atomic edits
│   │           ├── integration.rs    # 9 tools (JLCPCB SQLite, Freerouting MCP, datasheets)
│   │           ├── verification.rs   # 10 tools (DRC, design rules, KiCAD UI)
│   │           ├── config.rs         # 7 tools (user/project config, design rules)
│   │           ├── design_review.rs  # 6 tools (decoupling/connection/power/DFM audits); trace_current.rs adds audit_trace_current; layout_quality.rs (+ _grid, _planes, _power, _routing) adds audit_layout_quality; circuit_review.rs, circuit_checks.rs and circuit_model.rs add audit_circuit_design
│   │           ├── templates.rs      # 4 tools (6 built-in reference circuit templates)
│   │           └── manufacturing.rs  # 3 tools (export package, validate, cost estimate)
│   │
│   ├── konnect-sexp/                  # S-expression engine (no KiCAD dependency)
│   │   └── src/
│   │       ├── parser.rs             # nom-based parser (handles empty strings)
│   │       ├── writer.rs             # SexpEdit + apply_edits + write_atomic
│   │       ├── schematic.rs          # SymbolInstance, LibPin, extract_*, pin_endpoint
│   │       ├── geometry.rs           # PinTransform, transform_pin (CANONICAL pin math)
│   │       ├── net.rs                # NetRef — KiCad-10 vs legacy net node forms
│   │       ├── layers.rs             # Canonical layer names, copper stack queries
│   │       ├── command.rs            # Reversible edit commands
│   │       ├── transaction.rs        # Write-ahead journal for multi-file writes
│   │       └── error.rs              # SexpError (incl. Conflict)
│   │
│   ├── konnect-schematic-editor/      # Typed schematic model (parse → mutate → write)
│   │   └── src/
│   │       ├── schematic/            # Schematic, symbol, wire, label, sheet, misc
│   │       ├── sexp/                 # parser + writer (indent-preserving, see #210)
│   │       ├── library.rs            # Library symbol lookup
│   │       └── types.rs              # Shared value types (fmt_f64, …)
│   │
│   ├── konnect-ipc/                   # KiCAD 10 IPC API client
│   │   ├── proto/                    # Protobuf definitions (copied from KiCAD v10 source)
│   │   ├── build.rs                  # prost-build protobuf code generation
│   │   └── src/
│   │       ├── gen.rs                # Generated protobuf Rust types
│   │       ├── client.rs             # NNG req/rep client, all methods implemented
│   │       ├── builders.rs           # Protobuf message construction helpers (mm→nm conversion)
│   │       ├── transform.rs          # Rigid-body child transform — KiCAD 10 stores footprint
│   │       │                         # children in ABSOLUTE board coords (#23)
│   │       └── types.rs              # Public types (IpcFootprint, IpcTrack, etc.)
│   │
│   └── schematic-viewer/            # Tauri desktop app (separate from workspace)
│       ├── tauri.conf.json
│       ├── capabilities/default.json # Tauri 2 ACL grant (core:default) — without it event.listen() is silently denied
│       ├── src/main.rs               # Multi-sheet watcher + snapshot-isolated incremental kicad-cli SVG rendering + Tauri commands, 20 unit tests
│       └── frontend/index.html       # Pan/zoom SVG viewer, sheet selector, auto-refresh
│
├── plugin/                           # Python thin launcher (runs inside KiCAD)
│   ├── __init__.py                   # pcbnew.ActionPlugin — settings dialog (PCB Editor only)
│   ├── settings_dialog.py            # wxPython settings UI (paths, server control)
│   ├── native_bridge.py              # authenticated KiCad 10 native Specctra export bridge
│   └── plugin.json                   # KiCAD 10 IPC plugin manifest
│
├── packaging/
│   ├── build-pcm.ps1                 # Build the PCM zip (Windows)
│   ├── build-pcm.sh                  # Build the PCM zip (macOS/Linux)
│   ├── metadata.json                 # KiCAD PCM package manifest
│   ├── validate-pcm.py               # Validate metadata.json against the PCM schema
│   ├── schema/                       # PCM packages.v1 JSON schema
│   └── resources/                    # PCM package resources (icon.png)
│
└── .github/workflows/
    ├── ci.yml                        # 7 jobs: check+test (3-OS matrix), clippy and
    │                                 # fmt (ubuntu only), schematic viewer, Python
    │                                 # plugin, Nix flake, PCM packaging validation.
    │                                 # The last four cover code `cargo --workspace`
    │                                 # never sees. 9 check runs per PR — the matrix
    │                                 # counts as three.
    ├── e2e-kicad.yml                 # Real KiCAD 10.0.5: end-to-end suite, a
    │                                 # conformance pass over KiCAD's demo corpus, and
    │                                 # PCM assembly + schema validation. Weekly cron,
    │                                 # manual dispatch, reusable release gate, and an
    │                                 # opt-in label-triggered PR run.
    └── release.yml                   # Real-KiCad gate, build (4 targets), PCM package
                                      # (3 platforms, macOS universal via lipo), release
```

## KiCAD 10 Integration

### IPC API (PCB Editor — real-time)
- Transport: **NNG** (nanomsg-next-gen) over IPC sockets (Windows named pipes)
- Protocol: **Protocol Buffers** (protobuf3) with ApiRequest/ApiResponse envelope
- Socket path: from `KICAD_API_SOCKET` environment variable (set by KiCAD when launching plugins)
- Scope: **PCB editor only** — full CRUD on all board items, layer management, design rules
- Schematic editor IPC: export-only (SVG, PDF, BOM, netlist) — NO item CRUD

### S-Expression File Editing (Schematic — offline)
- Direct read/write of `.kicad_sch` files
- Symbol definitions auto-embedded from KiCAD 10's `.kicad_symdir` format
- Power symbols (VCC, GND) embedded from `power.kicad_symdir`
- Existing-file edits use revision-checked atomic replacement: read the exact
  source, acquire a cooperative lock, reject any intervening KiCad or Konnect
  change, write a unique sibling scratch file, fsync, and rename.
- Cooperative lock files live under `KONNECT_STATE_DIR/locks` when that
  absolute override is set, otherwise under the platform local-data directory
  (`konnect/locks`). Reads never create files in the KiCad project.
- Schematic writes also refuse while KiCad's sibling `~<name>.kicad_sch.lck`
  exists. KiCad records only a username and hostname, so Konnect cannot prove
  that a same-host or remote lock is stale; valid, foreign, empty, and malformed
  locks all fail closed. The check runs before a transaction journal is created
  and again at the final target-write boundary.
- Multi-file schematic changes use project-local
  `.konnect-transaction-*.json` write-ahead journals. These journals contain
  complete before/after images and must be treated as sensitive project data.
  On Unix a journal is created `0600` whatever the umask says; elsewhere it
  inherits whatever the platform gives a new file, so keep the project
  directory itself private there.
- A newly created design file or exported artifact takes the mode any other
  new file would get — `0666 & ~umask` on Unix. A replacement keeps the mode
  the destination already had, so a project created by an earlier Konnect
  stays at `0600` until its owner widens it with `chmod`.

`konnect_schematic_editor::Schematic` deliberately distinguishes creation from
replacement:

- `save(new_path)` is create-only and refuses to replace an existing path.
- `save(loaded_path)` and `overwrite()` replace only when the file still
  exactly matches the source loaded into the model. KiCad autosave therefore
  produces a conflict that callers must resolve by reloading and reapplying.
- Callers that intentionally replace an existing file must use the explicit
  revision-aware writer/command APIs; they must not delete the destination or
  weaken `save()` into an unconditional overwrite.

For journal diagnosis and recovery, use `konnect transaction status`,
`konnect transaction recover`, and the explicit force-gated `konnect
transaction abandon` escape hatch documented in
[Troubleshooting](docs/TROUBLESHOOTING.md#transaction-recovery-is-blocked-by-divergent-content).

### Which board a read answers about

A read-only PCB tool that can be answered by either KiCad or the saved file
asks `tools/board_source.rs` rather than choosing for itself. It advertises a
`board_source` argument — `auto` (default), `live`, `saved` — and reports
observed provenance beside the answer: a `sources` object naming the origin of
each domain (`ipc`, `saved_board`, `project_file`, `derived`, `unavailable`)
and a `source_evidence` object carrying `board_state`,
`excludes_unsaved_editor_state`, a machine-readable `reason` and its prose
`detail`.

Both it and the write gates in `pcb_board.rs` read KiCad's answer through
`tools/live_board.rs`, which owns the classification itself — including the
rule that a board this process watched KiCad hold is never treated as safely
absent afterwards, however KiCad stops answering (#240). The policies differ on
purpose (a stale read is disclosable, a stale write destroys work); the reading
of the evidence does not.

The rule the seam exists to hold: once KiCad has positively identified the
requested board as live, a failed query is **returned as a failure**. It never
becomes a successful answer read off a file that may be older than the editor's
unsaved state. A board this process watched KiCad hold, whose IPC endpoint then
disappears, is refused under `auto` for the same reason. An explicit `saved`
request may still inspect that snapshot, and says so in its `reason`.

`get_layer_list` and `get_netclasses` are the first consumers (#542). The
broader per-tool inventory is tracked in #574.

### kicad-cli v10 (Subprocess)
- Verified commands: `sch erc`, `sch export svg/pdf/bom/netlist`, `pcb drc`, `pcb export gerbers/drill/pdf/svg/step/vrml/pos/ipcd356`, `pcb render`
- Removed in v10: `sch annotate` (reimplemented in Rust), `pcb sync`, `pcb export/import specctra`
- Version format: `20250610`

### Plugin Installation
- **PCM zip** is the correct install method
- KiCad installs to: `C:\Users\<YOU>\Documents\KiCad\10.0\3rdparty\plugins\com_github_mixelpixx_konnect\`
- Both `__init__.py` (SWIG ActionPlugin for PCB editor settings dialog) and `plugin.json` (IPC exec plugin) are included
- `native_bridge.py` is a KiCad-10-only, opt-in compatibility bridge. It exposes
  only authenticated status and native Specctra export over an ephemeral
  loopback port. The caller cannot choose an output path; the plugin owns and
  removes the temporary artifact. Do not grow it into a general Python RPC
  surface or use it as the KiCad 11 architecture. The Rust exporter remains
  the default; callers must explicitly choose `prefer` or `require` to use this
  compatibility path.

## Structured Errors

Tool-call failures are typed via the `ToolErrorKind` enum in `crates/konnect-core/src/mcp/error.rs`. MCP's `CallToolResult` spec has no top-level `data` field, so structured errors ride inside the text content as JSON:

```json
{
  "message": "Tool 'place_component' is in toolset 'pcb_components' — call load_toolset('pcb_components') first, then retry.",
  "error": {
    "kind": "toolset_not_loaded",
    "toolset": "pcb_components",
    "tool": "place_component"
  }
}
```

`is_error: true` on the result; plain clients show the `message` field, structured clients match on `kind`. The observer's `error_kind` column is populated via `extract_error_kind()` so JSONL logs use the same vocabulary regardless of where the error originated.

### Current kinds

| `kind` | When |
|--------|------|
| `toolset_not_loaded` | Tool exists but its toolset isn't loaded yet |
| `unknown_tool` | Tool name doesn't exist in any toolset |
| `invalid_argument` | Required argument missing/malformed |
| `file_not_found` | Referenced file or project-discovery directory does not exist or cannot be read |
| `conflict` | The file changed, a write would replace existing paths, or schematic project ownership cannot be proven uniquely — carries the affected paths; ownership conflicts include the schematic directory and all candidate roots |
| `wrong_document` | The requested PCB is not among KiCad's positively identified open documents — carries `requested` and `open_documents`; path-bearing IPC operations never substitute another open board |
| `ambiguous_target` | More than one observed UUID, reference/unit, field, instance, or requested PCB document identity matches the requested target, or component instance records conflict across projects, units, or hierarchy paths — carries `target` and the stable `candidates`; the caller must choose rather than retrying blindly |
| `stale_target` | A previously bound PCB document is no longer uniquely open, saved symbol instance metadata disagrees with the proven hierarchy, or prospective component state lacks the expected document, UUID/reference, unit, library, hierarchy, position, rotation, or property value — carries `target` and `reason`; the edit, annotation, and grouping preflight refuses before writing |
| `mutation_outcome_uncertain` | A schematic edit committed, but immediate readback could not prove its requested result — carries `operation`, `path`, and `reason`; reload and inspect the named file before retrying because it may have changed |
| `simulation_failed` | A simulation stopped before giving a result: the engine is missing or crashed, the export failed, preflight refused the deck, or the run left no data — carries `stage`, a fixed `reason`, `log_path`, and `items`; branch on `reason` and fix what `items` names |
| `ambiguous_open_board` | KiCad answered, and its open-document list could not be read as a complete set of comparable board identities — carries `path`; neither the live nor the file path may run |
| `invalid_configuration` | A Konnect preferences file (`config.json`, `.konnect/project.json`) exists but cannot be used — carries `path` and a `reason` starting `malformed_json:` or `unreadable:`; defaults are never substituted, because the next save would persist them over the user's settings |
| `handler_error` | Catch-all for unmigrated `anyhow::Error` returns |

### Producing structured errors in a handler

```rust
if !path.exists() {
    return Ok(CallToolResult::error_kind(
        ToolErrorKind::FileNotFound { path: path.display().to_string() },
        format!("Project file not found: {}", path.display()),
    ));
}
```

Adding a new kind: edit `mcp/error.rs`, add the variant, add the match arm in `short_code()`, use it from the handler. The `short_code_matches_serialized_kind_field` test will fail loudly if they drift.

The dispatch-level errors (not-loaded/unknown/handler-panic) are fully structured, and so is **every input-schema error**. Three mechanisms produce them, in the order a call meets them:

1. **The schema gate.** Every advertised Draft 2020-12 schema is compiled when its tool definition is built, cached with that definition, and reused by `handler.rs::dispatch_tool`. Domain and meta-tool calls are refused before their handlers run when required fields are absent, present values have the wrong type, integer values are fractional, declared bounds fail, or a closed object contains an unknown key. Errors are structured `invalid_argument` values and name nested fields in caller notation such as `components[0].unit`. The legacy presence check remains first so a missing required field keeps its established schema order and message. An explicit `null` counts as absent there.

2. **The typed helpers**, in `tools/mod.rs`: `require_str`, `require_f64`, `require_array`, `require_u64`, checked optional helpers such as `opt_u32` and `opt_positive_f64`, and domain-specific validators. Each returns a `CallToolResult` carrying `InvalidArgument { field, reason }`. They preserve the same refusal when a handler is called directly — as most tests do — instead of relying only on MCP dispatch.

3. **`get_path`**, which returns `anyhow::Result<PathBuf>` so its 171 call sites can use `?`. Changing that signature was not worth it, so it attaches a `MissingArgument` marker to the error and `dispatch_tool` downcasts for it. Classify by downcasting, never by matching message text — the same rule `konnect_ipc::TransportUnreachable` follows.

A path that is *present but unusable* — absent on disk, wrong extension — is deliberately **not** an argument error. That is the handler trying and failing, and it stays a `handler_error` or `FileNotFound`. Collapsing the two would make "you forgot an argument" and "that file is not there" indistinguishable.

Why the gate exists: #218 found 25 sites across 18 tools that reported success on substituted values, and #546/#547 showed that wrong-typed options, fractional integers, declared bounds, and closed nested objects still passed. The compiled gate is the floor beneath handlers; checked helpers keep direct internal calls honest. Catalogue tests compile every schema and exercise real tools through dispatch, including a KiCad footprint fixture that must remain byte-identical on refusal.

Most in-handler errors still use `CallToolResult::error("free text")` or bubble `anyhow::Error`; migrating them is incremental. `project.rs::handle_get_project_info` demonstrates the structured `FileNotFound` pattern.

### One value KiCAD does not reject: an unrepresentable layer

`builders::layer_from_name` maps a layer name to a `BoardLayer`, and anything it does not know becomes `BL_UNDEFINED`. KiCAD 10.0.5 does **not** validate that field on an incoming item — it indexes its layer bitset with whatever arrives — so `BL_UNDEFINED` is answered with an access violation that terminates the process and takes the user's unsaved board with it (#237). Konnect sees only an NNG receive timeout.

So on any path that puts a layer into a message bound for KiCAD, use **`builders::try_layer_from_name`**, which refuses instead. `client.rs::build_footprint_item` validates the root layer, every pad layer and every graphic layer before it builds a single child; the `*.Cu`/`*.Mask`/`*.Paste` wildcards KiCAD itself writes are expanded rather than mapped, so they are skipped.

The infallible `layer_from_name` stays for read paths that already filter `BL_UNDEFINED` out. If you add a write path, the fallible one is the default — this is the class of bug where "the tool returned no error" and "the editor is still alive" are different questions.

## Observability

Every `tools/call` flows through `McpHandler::execute_tool`, which wraps the dispatch with:
- A **ring buffer** of the last 100 `CallRecord`s (surfaced via `get_recent_calls` meta-tool).
- **Per-tool counters** for totals, errors, cumulative duration, last-status, last-error (surfaced via `server_stats`).
- **JSONL append** to `<konnect dir>/logs/calls.jsonl` (one line per call). Paths:
  - Windows: `%APPDATA%\konnect\logs\calls.jsonl`
  - macOS: `~/Library/Application Support/konnect/logs/calls.jsonl`
  - Linux: `~/.konnect/logs/calls.jsonl`

  `KONNECT_CALL_LOG` overrides that: `off` disables the file, an absolute path replaces it, anything else disables it with a warning. Without it, a set `KONNECT_STATE_DIR` keeps the log at `$KONNECT_STATE_DIR/logs/calls.jsonl`. Each record is serialised with its newline into one buffer and written with a single `write_all` on an append-mode handle, so processes logging at once never share a line.

  **Tests never write the real log.** Under `cfg(test)` `calls_log_path()` is `None` for konnect-core's own tests, and `.cargo/config.toml` sets `KONNECT_CALL_LOG=off` for everything cargo runs (`cargo test`, and `cargo run` too: set an absolute path there to keep a log). The stdio test helper sets it again for the servers it spawns. A test that needs a log points `KONNECT_CALL_LOG` or `KONNECT_STATE_DIR` at a temporary folder.
- **Structured `tracing` events** (`tool_call_start` + `tool_call_end`) carrying `call_id`, `tool`, `toolset`, `status`, `dur_ms` — greppable in the stderr log.

Each `CallRecord` includes: `call_id`, `ts` (unix ms), `tool`, `toolset` (optional — `None` for meta-tools), `dur_ms`, `status` (`ok` / `error` / `not_found`), `error_kind`, `args_bytes`, `result_bytes`.

The observer is constructed once by `McpHandler::new` and stashed on both the handler and `ToolContext` so meta-tools can reach it. IO failures on the JSONL file never fail the tool call — they `tracing::warn!` and are silently dropped. Tests construct an in-memory-only observer via `ToolContext::new(...)` (no `log_path`).

Source: [`crates/konnect-core/src/observability.rs`](crates/konnect-core/src/observability.rs).

## Tool Routing (Starter Kit + On-Demand Loading)

The server does NOT expose all 268 tools (275 total with the 7 meta-tools) in `tools/list` by default — that would cost ~23K tokens of context on every listing. Instead:

- **Startup**: only `STARTER_KIT` toolsets are pre-loaded (see `router/registry.rs::STARTER_KIT`). Currently: `project`, `config`. Combined with the 7 meta-tools, baseline `tools/list` is 21 tools ≈ 2K tokens.
- **On demand**: the LLM reads `list_toolboxes` → calls `load_toolset(name)` to expose a toolset's tools in subsequent `tools/list` responses. `unload_toolset(name)` prunes them when the task shifts.
- **`tools/list_changed` notification**: sent on every load/unload so MCP clients refresh their local tool cache.
- **Error recovery**: if the LLM calls an unloaded tool, `handler.rs` returns an actionable error naming the toolset that owns it (so the LLM can load it and retry in one hop — no extra `list_toolboxes` round-trip).
- **`auto_load_toolsets` (config key, default `false`)**: when set, a miss in `dispatch_tool` loads the owning toolset and executes the call in the same hop instead of returning `toolset_not_loaded` -- fewer round trips, at the cost of toolsets accumulating monotonically for the rest of the session (`unload_toolset` still prunes, but a tool call reloads its toolset right back). Off by default because the router's whole point is keeping `tools/list` small; turn it on only if your client would rather eat the context growth than handle one recoverable error per miss. Set via `konnect.toml`/`settings.json` (`auto_load_toolsets = true`) or the equivalent `ServerConfig` field when embedding.

- **`eager_toolsets` (config key, default `false`)**: pre-loads every toolset at startup via `ToolRouter::load_all`, so the *first* `tools/list` is the full catalogue. This is for MCP clients that cache the initial tool list and never act on `notifications/tools/list_changed` — for those, a tool absent from the first listing can never be called at all, because `load_toolset` reports the names it loaded but returns no schemas and the client has nothing to invoke (#134, #169). Note `auto_load_toolsets` does **not** cover this case: it fires on a tool *call*, so it only helps a caller that already knows the tool name. Costs ~25K tokens per listing against the ~2K baseline, which is the router's entire reason for existing — hence off by default.

The router is defined in `crates/konnect-core/src/router/mod.rs`.

## Build Requirements

- Rust toolchain pinned by [`rust-toolchain.toml`](rust-toolchain.toml) (currently 1.96.0) —
  rustup picks it up automatically, and CI compiles with the same version. The pinned
  version IS the MSRV: bump it deliberately, in its own commit, after running the full
  local gate on the new version.
- `protoc` binary (for protobuf code generation in konnect-ipc crate)
  - Set `PROTOC` to the binary, or leave it unset and `konnect-ipc/build.rs` resolves
    `protoc` from PATH
  - Well-known-type includes (`google/protobuf/any.proto`) are derived from
    `<protoc>/../../include` after the binary is resolved to an absolute path. This
    covers both the upstream release layout (`bin/protoc` beside `include/`) and system
    packages (`/usr/bin/protoc` → `/usr/include`). Set `PROTOC_INCLUDE` to override when
    the binary and its protos live in unrelated prefixes — notably Chocolatey and scoop,
    whose shared shim directory is not the package prefix.
  - Some distributions ship the well-known `.proto` files separately from the compiler.
    Debian/Ubuntu need `protobuf-compiler` **and** `libprotobuf-dev`; Fedora needs
    `protobuf-compiler` and `protobuf-devel`. Missing them fails the build with
    `google/protobuf/any.proto: File not found`.
  - Download: https://github.com/protocolbuffers/protobuf/releases
- For schematic-viewer (built separately from the workspace — see Quick Start):
  - Rust toolchain on PATH (Windows: `set PATH=%PATH%;%USERPROFILE%\.cargo\bin` if `cargo`
    isn't recognized in the shell)
  - Tauri 2 prerequisites: WebView2 runtime on Windows (usually pre-installed on Win 10/11)
  - At runtime it discovers `kicad-cli` from the standard KiCAD install paths, then PATH;
    override with `--kicad-cli <path>`
  - Rebuilds fail while a viewer window is open (Windows locks the running `.exe`) — close
    the app before `cargo build`

## Test Suite

Run all: `PROTOC=<path> cargo test --workspace --lib --tests`

| Location | What |
|----------|------|
| `konnect-sexp` unit tests | Parser, writer, geometry transforms |
| `konnect-core` unit tests | Router load/unload, starter-kit, registry invariants, observability, error taxonomy, arg helpers |
| `konnect-core` integration tests | Fixture files: parse, edit, write, observability, structured errors |
| `konnect-schematic-editor` tests | Typed schematic model + round-tripping |
| `konnect-ipc` unit tests | Protobuf builders, client message construction, rigid-body child transform |
| `crates/konnect/tests/` | Protocol over stdio, doc/asset count guards, transaction CLI, `#[ignore]`d live-KiCAD tests |

`schematic-viewer` is **excluded from the workspace** (`Cargo.toml`'s `[workspace] exclude`) since
it's a Tauri app built separately, so `cargo test --workspace` never touches it. **CI does**: the
dedicated `viewer` job in `.github/workflows/ci.yml` runs `cargo check` and `cargo test` against
`crates/schematic-viewer/Cargo.toml`, because without it a viewer break ships silently. Run its
tests locally with `cd crates/schematic-viewer && cargo test`. Its 20 unit tests cover the pure sheet-tree-walking,
watch-directory, render-snapshot, event-debounce, and incremental-render-selection logic
(`walk_sheet_tree`, `compute_watch_dirs`, `snapshot_tree`, `drain_until_quiet`,
`files_needing_render`, `render_all`'s error handling) — the actual `kicad-cli` subprocess call
and Tauri command/event plumbing stay thin and untested, matching this codebase's existing
convention for other `kicad-cli`-calling code.

## Freerouting benchmark (opt-in)

`cargo xtask freerouting-bench` routes a folder of Specctra DSN files with one or
more Freerouting JARs and reports, per board and version, whether the routing is
complete and how good it is. It also ranks named router settings (`--config`)
and DSN variants of one board, and can judge every run on the real board
against its existing routing (`--score`). It needs Java and the JARs, takes
minutes to hours, and is **never part of CI or `cargo test`**. Only its parsing,
measuring and ranking are unit tests (`cargo test -p xtask`, no Java, KiCad or
Konnect).

```bash
cargo xtask freerouting-bench \
  --jar ~/.local/share/freerouting/2.5.0/freerouting-2.5.0.jar \
  --jar /path/to/freerouting-2.1.0.jar \
  --dsn-dir target/freerouting-bench/moderate --job-timeout-secs 120
```

- **Corpus.** The default is the `.dsn` fixtures in `crates/konnect-core/tests/fixtures`.
  For real boards, export KiCad's demos with KiCad's own exporter, using KiCad's
  Python: `…/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3 -I
  xtask/freerouting-bench/export_kicad_dsn.py ~/ClaudeCode/kicad-10.0.1/demos target/freerouting-bench/corpus`.
  It copies each board into a scratch folder, strips tracks, arcs and vias, and
  exports `<board>.dsn`. Source boards are never written. KiCad's exporter
  writes every pour as fixed copper, which blocks routing on boards with a
  board-wide pour, so prefer Konnect's own exporter with pours omitted (PR 16):
  write stripped boards with `export_kicad_dsn.py --stripped-boards <demos> <dir>`,
  then run PR 16's ignored test only for its DSNs:
  `KONNECT_DSN_PROBE_DIR=<dir> KONNECT_PROBE_KEEP=<out> KONNECT_PROBE_OMIT_POURS=1
  FREEROUTING_JAR=<jar> FREEROUTING_JAVA=xtask/freerouting-bench/quiet-java.sh
  FREEROUTING_SECONDS=2 cargo test -p konnect-core --lib freerouting_routes_a_folder
  -- --ignored`. The wrapper starts Freerouting in a throwaway home with
  telemetry off; delete the partial `.ses` files it leaves in `<out>`. Its
  routing rules are a flat 0.2 mm for every net, not the boards' own classes.
  Pick boards like the ones being designed (about 15 to 150 parts, 2 to 4
  layers, pours): the huge demos (`video`, `kit-dev-coldfire`, `jetson`,
  `vme-wren`) take the whole budget and say little about quality.
- **Java.** One JVM runs every JAR, so versions are compared on the same Java.
  The default is the newest of Homebrew's `openjdk@25` and `openjdk@21`.
  Freerouting 2.5.0 needs Java 25 (class file 69); 2.1 to 2.4 need 21.
- **Isolation.** Each run gets its own folder under `--out` (default
  `target/freerouting-bench/<unix time>`) with a copy of the DSN and its own
  `user.home` and `java.io.tmpdir`, with the `XDG_*`, `APPDATA` and
  `LOCALAPPDATA` folders pointed into it and any inherited `FREEROUTING__*`
  variable removed. So a run neither reads nor changes the settings of a
  Freerouting you use, and 2.5's `~/Library` settings and log stay in the run
  folder. Telemetry, contact and analytics are off in the seeded
  `freerouting.json`, on the command line and in the environment. Freerouting
  2.x turns them on by default. The seeded settings also carry an all-zero
  `profile.id`: 2.3 and later stop with a `NullPointerException` without one.
  Every HTTP and HTTPS request goes to a closed loopback port, so 2.5.0's
  update check (a GET to GitHub on every start, with no switch) never leaves
  the machine.
- **Budget.** `--job-timeout-secs` (default 60) is Freerouting's own
  `router.job_timeout`: a job that hits it stops and writes what it routed.
  `--timeout-secs` (default 600) kills the process. 2.3 and later log their own
  outcome for each job, and that decides whether the budget was hit; for 2.1,
  which logs none, a run that lasted the whole budget counts. 2.1.0 sometimes
  ignores its budget or exits without an SES; both count as a failed run.
- **Same version twice.** Each `--jar` gets its own folder under `--out`, named
  by the version in its file name, with `-2`, `-3` added when that is taken (and
  `unknown` for a file that names none). Totals and the ranking are per folder,
  with the reported version after it when it differs, so a rebuild stays a row of
  its own. `results.json` saves the folder as `dir` for `--rescore`.
- **Gate.** Completion is a gate, not a score. A board passes when every run
  wrote a readable SES with 0 unrouted connections (Freerouting's own count: 2.1's
  statistics block, the last `unrouted and N violation(s)` or `incomplete
  connections` line in 2.3 to 2.5), 0 clearance violations (a run that does not
  report the count fails), within the budget.
- **Quality**, measured from the DSN (pin positions, nets, net classes) and the
  first run's SES, on boards that pass:
  - wire/bound: wiring of the routed nets over half the perimeter of their
    pins' convex hull, a lower bound no wiring at any angle beats (a walk around
    any tree joining the pins encloses that hull). wire/HPWL is the usual
    reference, over the half-perimeter of the pins' bounding box; it bounds only
    90° routing, so Freerouting's 45° wiring can go below 1.00. Nets that also
    have a plane or pour, or wiring already in the DSN (Freerouting keeps it but
    does not write it to the SES), are left out. Pin positions mirror x for
    back-side parts, then turn counter-clockwise.
  - vias, vias per routed net, and layer changes (vias where wires on two layers
    meet; a via that only drops into a pour is not one).
  - acute corners (under 89.5°, acid traps) and right-angle corners (90 ± 0.5°),
    counted away from pads and vias.
  - tracks (unique segments) and narrow segments (below their net class's
    width; per class in `results.json`). Freerouting necks traces down near pads.
  - runtime (wall clock per run, JVM start included, median) and determinism:
    two runs are the same routing when they laid the same segments and vias in
    any order (2.1 sometimes writes the same copper in another order). Runs
    stopped by the budget are not compared.
- **Ranking.** Fewest boards failing the gate. Then, on the boards every
  competing version passed (a version that passed none competes on the gate
  alone): fewest defects (acute corners plus narrow segments), wire/bound in 1 %
  steps, fewest vias, fewest boards whose runs differ, least runtime. The top
  entry is printed as `Recommended:`.
- **Settings sweep.** `--config NAME=ARGS` (repeatable) routes and ranks named
  router settings, not only JARs. Each config with each JAR is a row of its own,
  in the folder `<NAME>@<jar label>`. ARGS are split at spaces and may only be
  `--router.<path>=<value>` or the router's short flags (`-mp -mt -is -us -oit
  -inc`, each with a value), so a config cannot turn telemetry on or move a
  run's files. Each setting may appear once, and the job budget stays
  `--job-timeout-secs` for all. A config that sets the passes or the router's
  threads (`--router.max_threads`, which is what 2.5 reads `-mt` as) replaces
  `--max-passes` or `--threads` for its runs. Give every lever a sweep compares
  in every config: Freerouting fills what is left out with its own defaults,
  and changes some of them by itself for each board (the per-layer trace
  costs, from the board's shape).
- **Effective settings.** Each run records what is known of the settings it
  really used (`settings` in `results.json`), from Freerouting's own log: 2.3
  and later write one under the run's home at debug level, whatever the console
  shows. (The `freerouting.json` they save back holds no router settings.)
  - A line in which Freerouting says a setting did not take ("Failed to apply
    CLI router setting", an unknown argument or property) fails the run:
    Freerouting only warns and routes on with its default.
  - Each `--router.` setting given is compared with the "Applied CLI router
    setting" lines. Another value, or no such line in a log that has them,
    fails the run. A log with no such lines at all (2.1) leaves the settings
    listed as unconfirmed.
  - The via costs and per-layer trace costs Freerouting printed for the
    routing are kept as `routing_parameters`. The CLI-only optimizer
    strategies `-is` and `-us` leave no line, so they are never confirmed.
  - A run whose log shows an answer to Freerouting's version check was not
    offline, and fails.
- **Inputs on record.** Before a run starts, its folder gets `inputs.json`:
  the JAR's and the DSN's SHA-256 (the DSN itself is copied in as `board.dsn`),
  every argument after `java`, the Java version, the commit this bench was
  built from and whether `xtask/` had uncommitted changes, the start time and
  the machine's load averages. A run whose inputs cannot be recorded is not
  started. Runs go one at a time, so a run's time is its own; check the load
  it started under before comparing runtimes.
- **DSN variants.** Every `.dsn` in the corpus is routed with every config, so
  variants of one board are candidates like any other.
  `xtask/freerouting-bench/stage_merge.py` (plain Python, **experiment only**)
  makes them by editing a DSN that Konnect exported: `unwiden` moves a net
  into the narrowest class, `drop-plane` removes pours by the index `planes`
  lists, `layer-cost` sets one layer's trace costs, and `merge` puts a first
  run's session into the DSN as fixed wiring for staged routing. Freerouting
  does not write fixed wiring back, so `merge` also writes `<name>.fixed.ses`
  beside `<name>.dsn`; the bench carries it into each run and scores both
  sessions together. An edited DSN has no reverse manifest: score it, never
  import it.
- **Board score** (`--score <spec.json>`). The measures above come from the
  session file alone: they cannot see a pour, a refill, KiCad's DRC or the
  layout audit. With a spec, every run and the board's existing routing (the
  baseline row) go through one pipeline, on scratch copies only: the session's
  wires and vias are put into a copy of the project
  (`xtask/freerouting-bench/score_board.py apply`, KiCad's own Python with
  `-I -B`), `kicad-cli pcb drc --refill-zones --save-board` refills and checks
  it, `score_board.py measure` reads the tracks, vias and zones back, and
  `audit_layout_quality` runs through this checkout's own `target/release/konnect`
  by pipe (state in `target/dogfood-home`; the installed Konnect is refused).
  The spec is a JSON file kept under `target/`, since it names a private board:

  ```json
  {
    "project": "project",
    "board": "board.kicad_pcb",
    "baseline": "routed.kicad_pcb",
    "intent_dsn": "V0.dsn",
    "kicad_config_home": "kicad-config",
    "currents": "the owner's, entered 2026-11-10",
    "thresholds": { "signal_match": 1.03 }
  }
  ```

  - `project` is a folder holding the unrouted board, its `.kicad_pro`, its
    libraries and `.konnect/design_intent.json`; it is copied whole for every
    run. `board` is the unrouted board's file name in it, and `baseline` a
    routed copy of the same board with the same placement. Paths are relative
    to the spec file.
  - `kicad_config_home` is a scratch `KICAD_CONFIG_HOME`, so no run reads or
    writes your KiCad settings: put `ZoneFillIterativeRefill=0` in
    `<it>/10.0/kicad_advanced` (KiCad 10.0.8's iterative refill defect, see
    `docs/CREATION_HANDOFF.md`) and copies of your global `fp-lib-table` and
    `sym-lib-table` beside it, or the library checks and so the DRC baseline
    change.
  - `currents` says where the design-intent currents came from (the owner, or
    test values); it is quoted with every result. `intent_dsn` is the export
    whose class widths are the intent widths (without it, each run's own DSN).
    `kicad_python`, `kicad_cli` and `konnect` default to the macOS paths and
    this checkout's release build: run `cargo build --release --locked -p konnect`
    first.
  - **Categories.** A net that owns a pour is a pour net (on these boards, the
    ground nets); a net with a recorded current and no pour is a power net; the
    rest are signals.
  - **Gates**, each over every run of a candidate. G1 completion: 0 unconnected
    in KiCad and a routed share of 1 in the audit, on the baseline's placement.
    G2 DRC: no violation type above the baseline's count. G3 width: no track
    below the narrowest class width, 0 `track_width` violations, each power net
    below its intent width on at most 5 % of its length and for no stretch over
    2 mm, and no more undersized nets or bottlenecks in the audit (4.2, 4.3)
    than the baseline.
  - **Quality**, against the baseline, each measure as beats, matches or below:
    signal and power track over the baseline's on the nets that have track on
    both boards (match up to x1.03 and x1.05), track on pour nets (+20 mm),
    vias off the pour nets, acute corners (none), right-angle corners (+5), the
    audit's `copper_quality` and category scores (-1), decoupling loops (+5 %),
    reference-plane coverage, unstitched layer changes and floating ground
    islands. `thresholds` in the spec replaces any limit by name
    (`below_intent_share`, `below_intent_section_mm`, `signal_match`,
    `power_match`, `pour_track_match_mm`, `right_angle_match`, `score_match`,
    `loop_match`, and `ratio_step` for the ranking); an unknown name is refused.
  - **Candidates.** Each config on each DSN is a candidate, quoted at its worst
    run. Ranking: the gates (a failed or unscored run last, then completion
    before DRC before width), acute corners, signal length and power length in
    steps of `ratio_step` (a threshold, 0.01 by default), right-angle corners,
    vias off the pour nets, copper quality, runtime. `Chosen:` names the first
    candidate only when it passes every gate in every run.
  - **Spread.** Freerouting varies from run to run, so the report lists each
    candidate's least and greatest value of every measure over its runs. A
    difference between two candidates that is no larger than that is not a
    difference: give a cell enough runs to see its spread, and set
    `ratio_step` no finer than it before reading the ranking.
- **Budget and resume.** Every Freerouting start is a line in `<out>/jobs.log`.
  `--max-jobs N` refuses an invocation whose runs, with those already there,
  would exceed N. `--resume` adds to the results already in `--out`: saved runs
  are kept, only the missing ones are routed, and runs that have a session but
  no board score are scored. A config whose settings, or a DSN whose content,
  is not what its saved runs used is refused, since the two would merge into
  one candidate: give it another name. A saved run that failed stays as it is
  unless `--retry-failed` is given, which routes it and the runs after it
  again (and lets a config all of whose runs failed change). `results.json` and
  `summary.md` are rewritten after every run, each written beside and moved
  into place, so a stopped sweep loses at most the run in progress. That
  run is routed again in an empty folder, and what it left is kept beside it
  as `run<N>.superseded-<unix time>`; if the bench was killed and its
  Freerouting lived on, the resumed run stops it first (each run notes its
  process id in `java.pid`), so it cannot write into the new run. Without
  `--resume`, an `--out` that already holds results is refused.
- **Rescore.** `cargo xtask freerouting-bench --rescore <results folder>`
  re-measures a finished run from its saved `board.dsn`, `board.ses`, score
  files and `results.json`, without Java, KiCad or Konnect, and rewrites its
  report. Use it after changing a metric or a threshold instead of routing
  again. With `--score <spec.json>` it also scores the runs that have no board
  score yet (KiCad and Konnect run; Java does not), which adds a board score to
  an earlier bench. Results that were scored are not rewritten without their
  scores: if the spec they name is gone, `--rescore` stops and says so.
- **Output.** `summary.md` (per-board table and ranking) and `results.json`,
  plus each run's `stdout.log`, `stderr.log`, `board.ses` and Freerouting's own log.
  Everything stays under `target/`. Quote only counts in tracked docs.
- **Updating the recommendation.** `check_freerouting` reports `version_policy`
  from `RECOMMENDED` in `crates/konnect-core/src/freerouting_setup.rs`: the
  version, the release URL and SHA-256 from GitHub's release API, and the
  measured reason. Change it only from a fresh run, with the counts in the
  reason and the date, JARs, Java and corpus size in `measured`.

## Adding a New Tool

Fixed input records are closed by `ToolDef::new` before the schema is advertised
and compiled. Declare all accepted fields in `properties`, including fixed
nested records. Intentional caller-keyed maps must explicitly set
`additionalProperties: true` or a value schema; register a new exception in
`fixed_records_are_closed_and_only_reviewed_maps_are_extensible` with a
compatibility test. A typed map's fixed value records are still closed. Untyped
caller data such as configuration `value` remains unrestricted. The same policy
applies to meta-tools. Use the served dispatch for typo/no-write regressions;
direct handler calls still require checked readers for their domain rules.

1. Add the `tool!(...)` definition to the appropriate toolset's `tools()` vec
2. Write the `async fn handle_*()` handler below the tools vec
3. Update `tool_count` in `router/registry.rs::ALL_TOOLSETS` — this is the declared count shown in `list_toolboxes`
4. If the new tool belongs in the default-available set, add its toolset to `registry.rs::STARTER_KIT`
5. Run `cargo check` and re-run the tool-directory extraction (see `tool-directory.md` header) to keep the docs in sync

## Current Stats

- **22 toolsets, 268 tools** + 7 meta-tools (4 routing + 2 observability + 1 runtime diagnostic — see `tool-directory.md`)
- The standalone Unix executable adds one conditional stdio maintenance meta-tool,
  `reload_server`; it is not registered for an embedded server, HTTP, mixed
  transport, or Windows.
- Baseline `tools/list`: 21 tools / ~2K tokens (starter kit + meta-tools)
- Full-catalog `tools/list` (all loaded): 275 tools (268 registered + 7 meta) / ~25K tokens
- **0 IPC stubs** (all protobuf methods implemented)
- **0 unimplemented tools**
- **Specctra DSN/SES are PCB-editor operations**, not `kicad-cli` commands.
  `export_specctra_dsn` creates a revision-bound routing job from the live
  editor, `route_specctra_dsn` delegates the route to Freerouting's local native
  MCP server, and `plan_specctra_ses_import` / `apply_specctra_ses` validate and
  return the result through one KiCad undo transaction.
