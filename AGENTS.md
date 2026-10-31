# Agent notes

Start with `docs/SIMULATION_HANDOFF.md`. It says where the work stands, what comes next, and how to verify it. `docs/SIMULATION_PLAN.md` holds the full design and its evidence.

## Rules

- Reply to the owner in Hebrew. Keep it short, with one clear recommendation.
- Update `docs/SIMULATION_HANDOFF.md` in the same commit that changes where the work stands.
- Work on one PR at a time, and ask before starting the next one.
- Commit only on a feature branch, and push only to the `fork` remote. `origin` is the archived upstream and is read-only.
- Do not merge a PR, enable CI or Actions, or turn on auto-merge unless the owner asks.
- Never send a message under the owner's name without showing the exact text and getting a yes.
- macOS comes first. Windows and Linux validation waits until the end of the series.
- Report what was and was not verified. A red baseline is reported, not fixed inside a feature PR.
- Real-project results stay out of documents, apart from counts.

## Never touch

- The installed Konnect at `~/.local/share/konnect/bin/konnect`. Its SHA-256 starts `3de188bb775ab4ce`. Check that at the end of every PR.
- `reload_server`. It re-executes the installed binary, so it can never serve this fork.
- `/Applications/KiCad` and `/Applications/Claude.app`.
- A live KiCad project. Run `kicad-cli` on scratch copies only, and never pipe it into `head`: the killed process leaves a lock behind.

## Tools

- System `python3` is the wrong interpreter. Use `/opt/homebrew/bin/python3.14 -I`.
- The toolchain comes from `rust-toolchain.toml`. Put Homebrew first on `PATH` so `cargo` resolves.
- Commit with the noreply address, never a personal one:

  ```bash
  git -c user.email=274043415+yuvaltal912-sys@users.noreply.github.com commit
  ```
