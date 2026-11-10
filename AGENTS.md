# Agent notes

Two tracks, two state files. Start with the one for your task; each says where the work stands, what comes next, and how to verify it.

- Simulation: `docs/SIMULATION_HANDOFF.md`. `docs/SIMULATION_PLAN.md` holds the full design and its evidence.
- Creation (schematic, PCB, library): `docs/CREATION_HANDOFF.md`. `docs/CREATION_RESEARCH.md` holds the ranked backlog and the 15-PR sequence.

## Rules

- Reply in the language the owner last used. Keep it short, with one clear recommendation.
- Update the track's handoff file (`docs/SIMULATION_HANDOFF.md` or `docs/CREATION_HANDOFF.md`) in the same commit that changes where the work stands.
- Work on one PR at a time, and ask before starting the next one.
- Commit only on a feature branch, and push only to the `fork` remote. `origin` is the archived upstream and is read-only.
- Do not merge a PR, enable CI or Actions, or turn on auto-merge unless the owner asks. When he asks for a merge, use the noreply merge command under "Known traps" in `docs/CREATION_HANDOFF.md`; never fast-forward `main`.
- Never send a message under the owner's name without showing the exact text and getting a yes. A push, a PR's title and body, and a PR comment all count. A general go-ahead approves the direction, not the text.
- Each PR goes the same way: the baseline gate first; the change with tests and a negative control for each guard; a review of the diff by an agent that did not write it; the gate again; a noreply commit; then the exact PR title and body to the owner.
- Inside a row the owner has started, measurement and check runs go end to end with no per-run OK (his instruction, 2026-11-10). Choose the matrix and thresholds yourself and record each choice with its reason. Stop once at the end, with the results and the exact PR title and body.
- macOS comes first. Windows and Linux validation waits until the end of the series.
- Report what was and was not verified. A red baseline is reported, not fixed inside a feature PR.
- Real-project results stay out of documents, apart from counts.

## Never touch

- The installed Konnect at `~/.local/share/konnect/bin/konnect`. Its SHA-256 starts `3de188bb775ab4ce`. Check that at the end of every PR.
- `reload_server`. It re-executes the installed binary, so it can never serve this fork.
- `/Applications/KiCad` and `/Applications/Claude.app`.
- A live KiCad project. Run `kicad-cli` on scratch copies only, and never pipe it into `head`: the killed process leaves a lock behind.
- The reference board under `target/creation-research/reference-boards/`. It is private and read-only: work on scratch copies.
- `cargo clean` in the main checkout. `target/` is git-ignored, and `target/creation-research/` holds the owner's plan, the reference board and the review and measurement records. None of it is in git.
- The Freerouting JAR outside the isolation Konnect and the bench give it (scratch home, telemetry off, network closed). Start it only through them; "Known traps" in `docs/CREATION_HANDOFF.md` has the details.

## Tools

- System `python3` is the wrong interpreter. Use `/opt/homebrew/bin/python3.14 -I`.
- The toolchain comes from `rust-toolchain.toml`. Put Homebrew first on `PATH` so `cargo` resolves.
- Commit with the noreply address, never a personal one:

  ```bash
  git -c user.email=274043415+yuvaltal912-sys@users.noreply.github.com commit
  ```

- The MCP server named `konnect` in an agent's tool list (Claude or Codex) is the installed binary, not this checkout. Exercise a change with `target/release/konnect` by pipe: "Dogfood the fork by pipe" under "Verify" in `docs/SIMULATION_HANDOFF.md`.
- `gh` has no default repository here, and `origin` is the upstream. Pass `--repo yuvaltal912-sys/Konnect` to every `gh` command.
- Set `KICAD_CONFIG_HOME` to a scratch folder for everything that starts KiCad (`kicad-cli`, KiCad's Python, the `konnect` binary). Without it the owner's real KiCad settings get re-saved.

## Working across agents

More than one coding agent (Claude, Codex) works on this project, one after another. The files in the repo are the only memory they share: no chat history, agent memory or agent-side hook carries over.

- **When you start:** read your track's handoff file before changing anything, and check its claims against `git status`, `git log` and `gh pr list --repo yuvaltal912-sys/Konnect`. A "tested" with no recorded command and result counts as unverified.
- **Before you stop**, or when the owner says he is switching agents, update the handoff file: date, branch and commit, what changed, what was verified (command, result), what is unfinished or broken, and the next concrete step with its done criterion.
- **Durable decisions** go where they survive a rewrite: the owner-decision table in the handoff file, with the reason, and a rule here when every agent must follow it.
- **Keep this file current and short.** When build or test commands, structure or conventions change, update it in the same change. Replace stale lines instead of appending.
- **Claude-only instructions** live in `CLAUDE.md`, below its `@AGENTS.md` first line. Keep that import, and don't copy Claude-only content here.
- **One agent per checkout at a time.** Agents working at the same time each use their own worktree and branch, and follow the claim rule in `docs/CREATION_HANDOFF.md`.
- **No secrets** in this file or in a handoff file. Name the variable and where its value comes from.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
