# Offloading test runs to the BGU HPC cluster

Status 2026-11-27: working. Real jobs ran on the cluster on 2026-11-27: 20894, 20895, 20896 and
20898 on `hpc-node5`, then the mutants jobs 20899, 20915 and 20920 (one PR's batch, on
`hpc-node5` and `hpc-node7`) and 20921 (the shared-target fix); the measured values below come
from them. Their records are
in the main checkout's `target/hpc-runs/` (20896, the one-off `cargo-mutants` install, has no
record folder; its times come from `sacct`). Values still
marked *unmeasured* are not settled by those runs. `scripts/hpc/selftest.sh` also runs the
scripts against a fake cluster on the Mac (no cargo, no network), and each guard was broken once
to check that the self-test catches it.

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
| Login node | `ece-hpc`, with `sbatch`, `squeue` and `sacct` in `/opt/slurm/bin`; no `apptainer` there (only on the compute nodes, as expected) |
| This Mac | set up 2026-11-27 as below: key `~/.ssh/id_ed25519_bgu_hpc`, the `bgu-hpc` block; host key checked from the Mac and from the server |

BGU's public HPC docs (hpc.bgu.ac.il) cover the central cluster (`bhn20`), not the ECE one. They
say the login node is reachable only from the campus network or the VPN, and they use key-based
sign-in. Without the VPN this Mac was not tested.

### One-time sign-in setup (done 2026-11-27; the owner does it, with his passphrase and his web-shell session)

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
   ssh bgu-hpc 'hostname -f; command -v sbatch squeue sacct'
   ```

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

Measured on the cluster, 2026-11-27 (16 CPUs, 16 GB, `hpc-node5`, x86_64):

| Cost | Value |
|---|---|
| Upload | a 12 MB git bundle (the whole history), seconds over the VPN |
| Preparation on the login node | about 5 minutes the first time (installing Rust 1.96.0 and fetching every crate); 4 seconds after that (run folder created to `sacct`'s submit time) |
| Queue wait | none: `sacct` gives the same submit and start time for all four jobs (one afternoon; a busy week may differ) |
| `konnect-sexp` tests (`--lib`) | 168 s cold, including its build; 32 s warm. 227 passed, 2 ignored |
| `konnect-core` cold build plus the `place_converge` tests | 245 s for the step, of which the tests took 77 s |
| `cargo mutants` on `konnect-sexp/src/net.rs`, `--jobs 4`, before the fix (job 20898, shared target) | 42 mutants in 6 minutes: 19 caught, 22 unviable, 1 missed |
| The same, through the fix: `mutants-probe.plan`'s three steps word for word, in a six-step proof plan (job 20921, `hpc-node5`) | 42 mutants in 5 minutes: 40 caught, 1 unviable, 1 missed |
| One PR's mutants batch: `konnect-core/src/tools/netclass_live.rs`, `--jobs 6` on 32 CPUs (job 20920, `hpc-node7`) | 81 mutants in 19.6 minutes: 75 caught, 3 missed, 3 unviable. The Mac's run of the same batch: 22 minutes, the same 3 missed |
| `target/` after those runs | 2.3 GB (`du`, read over ssh after job 20898) |
| Disk | the home folder is on a shared 166 TB file system with 112 TB free; `quota` reports no user quota. The node's `/tmp` has 59 GB free |
| Network from the compute node | reaches crates.io |

*Unmeasured:* the whole `cargo test --workspace --lib --tests` on Linux, the queue at busy times,
other `--jobs` values, and the two example plans as files of their own (`first-probe.plan` with
its `--skip` has not run; `mutants-probe.plan`'s steps ran inside job 20921's proof plan, each as
expected, while that plan's own extra shard step found 0 mutants, because its file was not on
that commit yet, and read FAILED). The wrapper's reading of option values (`-C DIR mutants` and
the like) is checked on the fake cluster only; on the cluster it ran for plain `cargo mutants`.

The gain: one mutants batch on the cluster is about as fast as on the Mac (19.6 against 22
minutes for the same 81 mutants), so the gain is not speed. It is the second lane: the batch
leaves the Mac's `instr.lock`, 20 to 60 minutes per batch that gates no longer wait behind. With
the 1-job quota, the four sessions share one cluster job at a time, so the cluster is a second
lane, not four.

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
- **Known Linux skip.** `place_converge::tests::whole_runs_are_the_recorded_goldens` compares
  bit-exact costs recorded on the Mac (aarch64). On the cluster (x86_64) one intermediate cost
  differs by 8 ulp; the end cost and the board are identical. It is a platform difference, not a
  bug: Linux plans skip it (`-- --skip place_converge::tests::whole_runs_are_the_recorded_goldens`),
  and the macOS gate still runs it. The skip itself has not run on the cluster yet.
- **Build needs.** `konnect-core` builds nng (cmake and a C compiler; the container has gcc 11.3
  and cmake 3.24.1) and the IPC protos. The container's `protoc` is 3.12.4, which rejects the
  protos' proto3 `optional` fields. So `prepare.sh` installs protoc 36.2 under
  `konnect-hpc/tools/protoc/` when it is missing or another version: it downloads the release zip
  from GitHub, checks its SHA-256 against the digest GitHub publishes for that asset (pinned in
  `scripts/hpc/common.sh`), and refuses a mismatch. `run-plan.sh` puts it on `PROTOC`. The
  protoc 36.2 the jobs used was installed by hand the same way; this install step was not part of
  those jobs. Its pieces were checked on the login node instead (`curl` reaches GitHub, the
  digest matches, `unzip` is there, the zip has `bin/protoc` and `include/`, and it reports
  `libprotoc 36.2`), and the self-test covers the step itself. An installed `protoc` that reports
  36.2 is kept as it is: its version is checked, not its digest.

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
any                mutants  cargo mutants -p konnect-core --file crates/konnect-core/src/place_converge.rs --jobs 4 --timeout 120 --output "$RUN_OUT"
```

Two example plans are in `scripts/hpc/plans/`: `first-probe.plan` (tools, disk and network on the
node, then two focused test runs) and `mutants-probe.plan` (one file's mutants). An `any` step
reads `recorded(exit N)` for every non-zero exit, so a mutants plan ends with a `pass` step that
checks that at least one mutant was really tested (caught, missed or timed out); without it a
red baseline, or a batch that is all unviable, would look green.

`hpc-fetch` ends `evidence.txt` with one line: `ALL STEPS AS EXPECTED`, `NOT AS EXPECTED`,
`INCOMPLETE` (steps missing: wall time or a crash), `NOT CLEAN` (every step as expected but Slurm
reports a state other than `COMPLETED`), or `INVALID` (the cluster checked out another commit).

Mutants on the cluster use `cargo-mutants` 27.1.0, the Mac's version. It was installed once, on
2026-11-27, in `~/.cargo/bin` by a separate one-off Slurm job on a compute node (job 20896, 8 CPUs,
1.5 minutes), with `cargo install --locked cargo-mutants --version 27.1.0` and the home folder's
default toolchain (1.100), not the pinned one. Repeat that the same way if it is ever missing;
`env.txt` records the version each job sees.

**`cargo mutants` never sees the shared `CARGO_TARGET_DIR`.** cargo-mutants copies the tree once
per job. With `--jobs` above 1 and the shared target every copy built into that one folder, and
every mutant read unviable: job 20899 read 81 of 81 unviable on one PR's batch at `--jobs 6`.
`--jobs 1` worked with and without the shared target (job 20915: 3 of 3 caught both ways), and
`env -u CARGO_TARGET_DIR` at `--jobs 6` gave the Mac's result (job 20920, in the table above).
So `run-plan.sh` puts two wrappers first on `PATH`: one for `cargo` that unsets the variable when
the subcommand is `mutants`, and one for `cargo-mutants`. The subcommand is the first word that
is not an option, a `+toolchain` or the value of a global option that takes one (`-C`, `-Z`,
`--config`, `--color`, `--explain`), so `cargo test mutants` and
`cargo run -- mutants` keep the shared target. The job stops before any step if `cargo` on `PATH`
is already a wrapper, which would call itself. A plan author needs no `env -u`, and a step's log says
`hpc: cargo mutants runs without the shared CARGO_TARGET_DIR`. Every other cargo command keeps
the shared target, the warm cache, even in the same step. Each mutants copy builds cold, so a
batch spends time on the baseline build: 180 s for `konnect-core` (job 20899's log), 19 s for
`konnect-sexp` (job 20921's).

The net.rs probe shows the fix on the cluster. Before it (job 20898, `--jobs 4`, 16 CPUs): 19
caught, 22 unviable, 1 missed. Through the wrappers, the same 42 mutants (job 20921,
`mutants-probe.plan`'s steps word for word): 40 caught, 1 unviable, 1 missed, and the step's log carries
the wrapper's line. So 21 of the 22 unviable came from the shared target. The missed mutant
(`names_nets_in_place` in `konnect-sexp/src/net.rs` replaced with `true`) is a gap that is already
on `main`, not something these scripts caused.

**`cargo mutants` keeps its copies off the node's `/tmp`.** Each copy (a tree and its own target,
several GB for `konnect-core`) goes under `TMPDIR`. On the node that was `/tmp`, local and about
59 GB: at `--jobs 8` one PR's batch filled it, and 261 of 273 mutants read unviable with
`No space left on device` (job 20938, 40 CPUs). The same two wrappers now set `TMPDIR` for a
mutants process to `scratch/mutants-tmp` in the run's folder on the network home, and the step's
log says `hpc: cargo mutants copies its trees under <folder>, not the node's /tmp`. Other cargo
commands keep the node's `/tmp`. If the folder cannot be made, the step stops rather than fall back
to `/tmp`. cargo-mutants removes its copies when it ends cleanly. `run-plan.sh` removes the folder
when the job ends, and `prepare.sh` removes any that a killed run left before the next job.

Two more lessons from the same day:
- **Filter the tests.** Without a filter after `--`, the unmutated baseline runs the whole lib
  suite, and on Linux it fails: 122 tests in job 20937, 107 of them in `tools::simulation` (no
  simulator here), 7 in `simulation::worker_client` and 4 in `jobs::tests` (an open Linux finding
  against #126). Give each batch the mutated modules' tests, as `-- --lib -- <filter>`.
- **Ask for 32 to 40 CPUs.** An 80-CPU request (job 20932) stayed pending about 20 minutes and was
  cancelled; 40 CPUs started at once.

### Commands

```bash
scripts/hpc/hpc-submit --cpus 16 --mem 16G --time 0:30:00 --label "PR D controls" PLAN_FILE
scripts/hpc/hpc-submit --cmd 'cargo test -p konnect-sexp --locked --lib'
scripts/hpc/hpc-status            # quota, then the last run's sacct line and progress
scripts/hpc/hpc-status --quota
scripts/hpc/hpc-fetch             # after it ends: logs and evidence.txt
```

`--dry-run` builds the run folder and sends nothing. Limits: 1 to 80 CPUs, at most 4:00:00.
A label takes letters, digits, spaces and `._:#/-` only (no commas).
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
  account. On 2026-11-27 the owner gave a standing OK to install on the cluster whatever the
  pipeline needs, without asking each time, on three conditions: official sources only, checksums
  verified, and builds on compute nodes. A submit downloads three things: the pinned Rust
  toolchain from rustup when it is missing (once), protoc 36.2 from GitHub when it is missing
  (once, digest checked), and the crates in `Cargo.lock` from crates.io (`cargo fetch`, cached
  after the first run). Which jobs to send, and when, stays with the PR's own rules.
- **Real-project files.** Plans run tests from the repository only. Never put the owner's boards
  or the reference board in a plan.

## 5. Open items

- A first full `cargo test --workspace --lib --tests` on Linux, with the list of macOS-only
  failures to `--skip`.
- Other `--jobs` values on a PR-sized batch (only `--jobs 6` on 32 CPUs is measured).
- The lane README line in section 4, for the supervisor or the owner to add.
