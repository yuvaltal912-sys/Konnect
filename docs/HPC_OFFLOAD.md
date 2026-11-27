# Offloading test runs to the BGU HPC cluster

Status 2026-11-27: design and scripts only. `scripts/hpc/selftest.sh` runs them against a fake
cluster on the Mac (fake ssh, Slurm, container and toolchain; no cargo, no network) and passes;
each guard was broken once and the self-test caught it. **No real cluster job has run through
them yet**, and `~/.ssh` is unchanged. The first job, and everything below marked *unmeasured*,
waits for the owner's yes.

## Recommendation

Move **`cargo mutants` batches and negative-control batches** to the cluster first. They are the
longest jobs that hold a Mac lane lock (`instr.lock`, which every full gate also needs), and they
are platform-independent for any guard that is not macOS-specific. Focused `cargo test` runs come
second: worth sending only while the Mac is blocked by another session's `native.lock`. The full
gate stays on the Mac.

## 1. Terminal access

Checked from this Mac on 2026-11-27, with the BGU VPN up (the route to the cluster goes through the
VPN interface):

| Fact | Value |
|---|---|
| Host | `ece-hpc.bgu.ac.il` (132.72.105.245), port 22 open over the VPN |
| Server | OpenSSH 8.7 |
| Sign-in methods offered | `publickey`, `gssapi-keyex`, `gssapi-with-mic`, `password`. No `keyboard-interactive`, so no one-time code is asked for |
| Host key (ED25519) | `SHA256:qh8Jj3ykfstzY9arZBGKfnYtTIeBlWAuxHLlO/gJVqw` |
| This Mac | no `~/.ssh/config`, no key or `known_hosts` entry for this host |

BGU's public HPC docs (hpc.bgu.ac.il) cover the central cluster (`bhn20`), not the ECE one. They
say the login node is reachable only from the campus network or the VPN, and they use key-based
sign-in. Without the VPN this Mac was not tested.

Not yet verified: that `ece-hpc.bgu.ac.il` itself has `sbatch`. The web shell may land on another
login node. Step 4 below checks it.

### One-time sign-in setup (the owner does this; it needs his passphrase and his web-shell session)

1. Make a key on the Mac and keep its passphrase in the macOS keychain:

   ```bash
   ssh-keygen -t ed25519 -f ~/.ssh/id_ed25519_bgu_hpc -C konnect-hpc
   ssh-add --apple-use-keychain ~/.ssh/id_ed25519_bgu_hpc
   pbcopy < ~/.ssh/id_ed25519_bgu_hpc.pub
   ```

2. In the web shell (ece-hpc dashboard → Clusters → ECE HPC Cluster Shell Access), paste the key
   between the quotes, and print the host's fingerprint and name:

   ```bash
   mkdir -p ~/.ssh && chmod 700 ~/.ssh && echo 'PASTE-THE-KEY-HERE' >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys
   ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub; hostname -f
   ```

3. Add this block to `~/.ssh/config` on the Mac. The ControlMaster lines make one sign-in serve
   every script for 8 hours:

   ```
   Host bgu-hpc
       HostName ece-hpc.bgu.ac.il
       User yuvalaj
       IdentityFile ~/.ssh/id_ed25519_bgu_hpc
       IdentitiesOnly yes
       AddKeysToAgent yes
       UseKeychain yes
       ControlMaster auto
       ControlPath ~/.ssh/cm-%C
       ControlPersist 8h
       ServerAliveInterval 60
   ```

4. Test, once. The first time, ssh asks to trust the host key: accept only if it matches the
   fingerprint above and the one step 2 printed.

   ```bash
   ssh bgu-hpc 'hostname -f; command -v sbatch squeue sacct apptainer'
   ```

   If `sbatch` is missing there, set `HostName` to the name step 2 printed.

Without a key, the password still works: `ssh -fN bgu-hpc`, typed by the owner, opens the master
connection, and the scripts reuse it until it expires. The scripts always run ssh with
`BatchMode=yes`, so they never prompt for a password and stop with these instructions instead.

## 2. Where the Mac waits, and what is worth moving

Measured on this Mac (15 cores: 5 performance and 10 efficiency; 24 GB):

| Work | Mac time | Lock it holds | Source |
|---|---|---|---|
| Full gate | 12 to 13 min (tests step 5 min, sandboxed fastroute tests 1.5 to 2 min, release build 2.5 to 3 min) | `gate`, `native`, `instr` | two gate baselines of 2026-11-27 |
| Waiting for the gate lock | 10 min in one of those two | | same |
| `cargo mutants` batch | 23, 41 and 56 min of summed build and test time (25, 136 and 182 mutants) | `instr` | three recorded batches |
| Negative-control batch | one per PR, several focused runs | `instr` or a light slot | lane README |

Rule 4 of the lane README forbids every cargo run, even one focused test, while another session
holds `native.lock`. Two gates per PR in each of four parallel sessions put the other three
sessions on hold for about 25 minutes per PR.

| Candidate | Move? | Why |
|---|---|---|
| macOS gate | **stays** | macOS comes first (AGENTS.md); it is the evidence |
| Anything that starts KiCad, `kicad-cli`, KiCad IPC, Java or Freerouting | **stays** | none of these is on the cluster; real-project files stay on the Mac |
| fastroute and routing measurements | **stays** | comparable only on the same machine |
| Controls for macOS-only guards (sandbox profile, `fastroute::tests::sandboxed`) | **stays** | the guard does not exist on Linux |
| `cargo mutants` | **move first** | longest `instr.lock` holder; 80 CPUs on a node against 15 here |
| Negative-control batches | **move** | frees `instr.lock`; a plan step can apply each control's patch, and the job checks a test really failed |
| Focused `cargo test` | **move while the Mac is gated** | small gain otherwise: a warm Mac run takes a minute or two |
| Linux validation at the end of the series | **move** | owed anyway; this Mac has no Linux |

Costs per run: the upload is a 12 MB git bundle (the whole history; measured), seconds over the
VPN. Preparation on the login node (checkout, `cargo fetch`) takes seconds once crates are cached.
*Unmeasured:* the queue wait in the `course` partition, the first cold build (expect 10 to 15
minutes on 16 CPUs), warm rebuilds on the cluster's network file system, and the home quota against
a `target/` folder of several GB. The first job measures all of these.

Expected gain once it works (*unmeasured*): a mutants batch leaves the Mac's lane altogether, which
is 20 to 60 minutes of `instr.lock` per batch that gates no longer wait behind. With the 1-job quota,
the four sessions share one cluster job at a time, so the cluster is a second lane, not four.

## 3. How a run works

```
Mac: hpc-submit                         login node: prepare.sh               compute node: job.sbatch → run-plan.sh
  git bundle of the exact commit  ──tar over ssh──▶  quota check, run.lock      apptainer exec <image>
  plan.txt (+ control patches)                       fetch bundle → repo.git      offline build, pinned toolchain
  no cargo on the Mac                                checkout src/ at the SHA     each step: log, exit, test counts, verdict
                                                     rustup 1.96.0 if missing     restore the exact commit after each step
                                                     cargo fetch --locked         results.tsv, env.txt
                                                     sbatch ──────────────────▶   releases run.lock at the end
Mac: hpc-status (squeue, sacct, progress)     Mac: hpc-fetch (logs back, evidence.txt)
```

- **The exact commit.** `hpc-submit --commit REV` bundles that commit alone, pushed or not.
  Uncommitted changes are not sent, and the job refuses to run if the checkout is not that SHA.
  Nothing is pushed to any git remote.
- **Toolchain.** The checkout's `rust-toolchain.toml` selects the toolchain through rustup. If
  1.96.0 is missing, `prepare.sh` installs it (download only, on the login node). The job sets
  `RUSTUP_AUTO_INSTALL=0` and refuses to run unless `rustc -V` reports 1.96.0, so the 1.100
  toolchain in the home folder is never used by mistake.
- **Cargo cache.** One fixed checkout (`src/`) and one `target/` folder under the cluster work
  folder (`/truenas/home/yuvalaj/konnect-hpc`), kept between jobs. The job builds offline
  (`CARGO_NET_OFFLINE=true`) from the crates `prepare.sh` fetched.
- **Isolation.** As on the Mac gate: `KICAD_CONFIG_HOME` and `KONNECT_STATE_DIR` point into the
  run's scratch folder.
- **Quota.** One job per user, running or queued. `hpc-submit` checks `squeue -u "$USER"` before
  sending anything, `prepare.sh` checks again, and a `run.lock` folder on the cluster guards
  `src/` and `target/`. A lock is stale when its job has left the queue, its preparing process is
  gone, or its heartbeat is over an hour old; it is then broken with an atomic rename. A refused
  or failed preparation deletes its own upload. If the connection drops right after `sbatch`,
  `hpc-submit` finds the job by name and records it.
- **Logs and exit status.** `hpc-status` shows `squeue` and `sacct` (state, exit code, elapsed,
  allocated CPUs and TRES) and the job's last lines. `hpc-fetch` refuses while the job is pending
  or running, then copies the run folder back and writes `evidence.txt`.
- **Tests that need KiCad.** The KiCad tests are `#[ignore]` (`e2e_kicad`, `live_kicad_tools`,
  `simulation_live`) and do not run under `--lib --tests`; upstream CI ran that set on Ubuntu
  without KiCad. The fork's later tests have never run on Linux, so the first full Linux run will
  show which ones assume macOS (40 `target_os = "macos"` sites; `fastroute::tests::sandboxed`
  uses the macOS sandbox). List those in the plan's commands with `--skip`, not by deleting tests.
- **Build needs.** `konnect-core` builds nng (cmake and a C compiler) and the IPC protos (`protoc`;
  CI pins 3.23.4). If the container lacks `protoc`, put a release under
  `konnect-hpc/tools/protoc/` (bin and include); `run-plan.sh` picks it up. That download needs the
  owner's yes.

### Plans

One step per line: `EXPECT NAME COMMAND`. `#` starts a comment.

| EXPECT | Means | Verdict |
|---|---|---|
| `pass` | exit 0; if libtest ran, at least one test passed | `ok`, `FAILED`, `NO-TESTS` (the filter matched nothing) |
| `fail:PATCH[@TEST]` | negative control: apply PATCH (relative to the plan file); at least one test must fail, TEST (part of a test name) among them if given | `ok` (caught), `MISSED` (tests passed), `INVALID` (non-zero exit but no failing test, such as a build error), `NO-BASELINE`, `WRONG-TEST`, `PATCH-FAILED` |
| `any` | record only (a probe, a mutants run) | `recorded`, or `recorded(exit N)`, which `hpc-fetch` notes |

Every control needs a baseline: a `pass` step with the same command earlier in the plan.
`hpc-submit` refuses a plan without one, and a control whose baseline was not `ok` reads
`NO-BASELINE`, so a test that already fails on the commit cannot count as "caught". Each row also
lists the failing tests' names. Those names come from libtest's `test NAME ... FAILED` lines, so
do not pass `-q` or `--format terse` to a control's test run: with no names, `@TEST` reads
`WRONG-TEST`.

`+DIRTY` means the step moved HEAD or left changed or new files (a control's own patch aside). The
next step still starts from the exact commit. Steps see `SHA`, `RUNID` and `RUN_OUT` (a folder that
is fetched).

```
pass               lib      cargo test -p konnect-core --locked --lib place_converge
fail:nc/1.patch@caps  guard-1  cargo test -p konnect-core --locked --lib place_converge
any                mutants  env -u CARGO_TARGET_DIR cargo mutants --no-shuffle -j 8 -f crates/konnect-core/src/place_converge.rs --output "$RUN_OUT" -- --lib
```

`hpc-fetch` ends `evidence.txt` with one line: `ALL STEPS AS EXPECTED`, `NOT AS EXPECTED`,
`INCOMPLETE` (steps missing: wall time or a crash), `NOT CLEAN` (every step as expected but Slurm
reports a state other than `COMPLETED`), or `INVALID` (the cluster checked out another commit).

Mutants on the cluster need `cargo-mutants` 27.1.0 (the Mac's version) installed there first
(`cargo install cargo-mutants --locked --version 27.1.0`, from a job if the node has network, or on
the login node). Each mutants copy builds on its own, so unset `CARGO_TARGET_DIR` for that step.
The job count is to be tuned on the first mutants run.

### Commands

```bash
scripts/hpc/hpc-submit --cpus 16 --mem 16G --time 0:30:00 --label "PR D controls" PLAN_FILE
scripts/hpc/hpc-submit --cmd 'cargo test -p konnect-sexp --locked --lib'
scripts/hpc/hpc-status            # quota, then the last run's sacct line and progress
scripts/hpc/hpc-status --quota
scripts/hpc/hpc-fetch             # after it ends: logs and evidence.txt
```

`--dry-run` builds the run folder and sends nothing. Limits: 1 to 80 CPUs, at most 4:00:00.
After editing the scripts, run `scripts/hpc/selftest.sh` (about 30 seconds, no network).
Records go to the main checkout's `target/hpc-runs/<run id>/` (git-ignored, and it outlives a
worktree). Settings (host alias, work folder, partition, account, QoS, image) are at the top of
`scripts/hpc/common.sh`.

## 4. How it fits the project's rules

- **Lane locks.** A cluster job uses no Mac CPU, so it takes no lane lock. `hpc-submit`,
  `hpc-status` and `hpc-fetch` run git, tar and ssh only, never cargo, so rule 4 (no cargo while
  someone holds `native.lock`) does not stop them. Proposed line for the lane README, for the
  supervisor or the owner to add: *"A cluster job (scripts/hpc) takes no lane lock; its scripts
  run no cargo on the Mac."*
- **Evidence.** `evidence.txt` records the run id, the commit (sent and checked out), the job id,
  the requested resources and `sacct`'s allocation (`AllocTRES`, node, start, end), the toolchain,
  and each step's exit code, time, test counts and verdict. It is **Linux evidence**: a PR says
  "Linux, BGU cluster, job N" beside the result and never offers it as the macOS gate.
  A run that stopped early reads `INCOMPLETE`.
- **Outward actions.** A submit sends the owner's code to the university cluster under his
  account: each job needs his yes, as do the first sign-in setup and any other download onto the
  cluster. A submit itself downloads two things, which that yes covers: the pinned Rust toolchain
  from rustup when it is missing (once), and the crates in `Cargo.lock` from crates.io
  (`cargo fetch`, cached after the first run).
- **Real-project files.** Plans run tests from the repository only. Never put the owner's boards
  or the reference board in a plan.

## 5. Open items

- First real job (proposed: `scripts/hpc/plans/first-probe.plan`, 16 CPUs, 16 GB, 30 minutes):
  tools on the node, disk and quota, network from the node, then two focused test runs.
- Then: the queue wait, cold and warm build times, the home quota against `target/`, whether the
  node can install `cargo-mutants`, and a first full `cargo test --workspace --lib --tests` on Linux
  with the list of macOS-only failures.
- After the first good job: a short pointer in `AGENTS.md` (Tools) and the creation handoff.
