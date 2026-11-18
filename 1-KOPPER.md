# 1 Kopper

1 Kopper is this fork of Konnect, packaged for its owner's own use. It is
Konnect's creation tools (schematic, PCB, library) with **fastroute** as the
default router, run as a timed best-of-N search. It comes without the
simulation branch.

## Install

From a checkout of `main`:

```bash
cargo xtask kopper-install
```

Then start a new Claude or Codex session, and check the package:

```bash
cargo xtask kopper-smoke
```

That is all. `kopper-install` does these steps, in this order:

1. **Checks** what the clients need, and changes nothing if a check fails:
   - Claude's `konnect` entry, if any, is a stdio entry with a command;
   - Codex's `konnect` table, if any, has one plain quoted `command`;
   - `claude` is on `PATH`, and `CLAUDE_CONFIG_DIR` is not set;
   - the way-back record reads.
2. **Builds** the Konnect release, which takes minutes. It then copies Konnect and
   the pinned fastroute build into `~/.local/share/1-kopper/`. A fastroute file
   that is not the pinned build is refused only now, after the build. Nothing is
   written then.
3. **Records the way back** in `~/.local/share/1-kopper.rollback.json`: the
   `konnect` entries Claude and Codex had before. The first original of each is
   kept and never overwritten. An entry that already runs a package is not
   recorded.
4. **Installs the guidance** (skills, agents, hooks) of this build, without the
   simulation skill. Claude gets the skills as the plugin `kopper1`; Codex gets
   them in its own plugin `kopper1` when it has one, else loose.
5. **Switches** Claude's and Codex's `konnect` entry to
   `~/.local/share/1-kopper/bin/konnect`.

`kopper-smoke` starts the packaged `konnect` in a scratch folder under the
checkout's `target/kopper-smoke/` and checks it end to end:
- the package matches its manifest;
- simulation is off;
- a public board routes with the package's own fastroute, passes the judge and
  reads `status: complete`;
- the installed Konnect and your KiCad settings are unchanged. The check fails if
  the installed Konnect is missing.

## Go back

```bash
cargo xtask kopper-install --rollback
```

It puts the recorded entries back, then runs each one's own `konnect init`.
- If all of that worked, it renames the record to `1-kopper.rollback.json.used`.
- If anything failed, it keeps the record. Fix the problem and run it again.

The package folder stays where it is. Then start a new session.

## Update

Pull `main` and run `cargo xtask kopper-install` again.
- The package that was there is kept as `~/.local/share/1-kopper.previous`. That
  is one step back only: each update replaces the older `.previous`.
- The way-back record still holds the first original entries.

## What it needs

- macOS: fastroute runs only under `sandbox-exec`.
- The Rust toolchain from `rust-toolchain.toml`, with Homebrew first on `PATH`.
- KiCad at `/Applications/KiCad`: `kicad-cli` judges every route.
- The fastroute build named in `packaging/1-kopper/fastroute.json` (today x18) at
  `~/.local/share/fastroute/candidate/fastroute`, or named with `--fastroute <file>`.
  The install refuses a file whose SHA-256 is not the pinned one.
- `claude` on `PATH`.
- Codex is switched only if `$CODEX_HOME/config.toml` (default `~/.codex`)
  already has a `[mcp_servers.konnect]` with a plain quoted `command`. Otherwise
  Codex is left alone.

## What is where

| Path | What it is |
|---|---|
| `~/.local/share/1-kopper/bin/konnect` | the server the clients run |
| `~/.local/share/1-kopper/bin/fastroute` | the pinned fastroute build. Konnect takes it because `1-kopper.json` is one folder up, unless `KONNECT_FASTROUTE` or `routing.fastroute_path` names another |
| `~/.local/share/1-kopper/1-kopper.json` | versions, commit and SHA-256 of both |
| `~/.local/share/1-kopper.previous/` | the package before the last install |
| `~/.local/share/1-kopper.rollback.json` | the original `konnect` entries (mode 0600) |
| `$CODEX_HOME/config.toml.before-1-kopper` | Codex's config before the first switch |
| `~/.claude/skills/kopper1/` | Claude's KiCad skills, as the plugin `kopper1` (`/kopper1:kicad-pcb` and the rest in the `/` menu; `/kopper1:kopper` loads the ones a task needs) |
| `~/.agents/plugins/kopper1/` | Codex's KiCad skills, when Codex has the plugin `kopper1` (else loose in `~/.agents/skills/`) |

## What it never touches

- The installed Konnect at `~/.local/share/konnect`. The install refuses to
  package into it, or over it. A rollback runs it for `init`, but does not
  change it.
- Your KiCad settings. The smoke checks every file under them before and after.
- Any client entry other than `konnect`.

## Simulation

The simulation code is still in the source and hidden. The server serves it only
when the configuration it finds sets `simulation_enabled = true`. It looks in
this order:
1. `--config <file>`;
2. `konnect.toml` or `settings.json` in the current folder;
3. the settings next to the binary;
4. the user config.

A project folder with such a `konnect.toml` turns simulation on there. Without
that key, `konnect init` installs no `kicad-simulation` skill, and removes one
that an earlier install left.

## Change the fastroute build

Edit `packaging/1-kopper/fastroute.json` (`build`, `engine_commit`, `sha256`) in
its own PR, put that build at the default place, then run
`cargo xtask kopper-install`.

## Known

- On a board where the judge had to add stitching vias to connect a pour,
  `status` stays fastroute's own. Follow `completion.next_steps`; they include
  the stitching.
- Windows and Linux are not supported yet.

Background and every decision: `docs/CREATION_HANDOFF.md`, rows K1 to K4.
