#!/bin/bash
# Runs on the cluster's LOGIN node, started by hpc-submit over ssh. Light work only:
# check the quota, take the run lock, check out the exact commit, make sure the pinned
# toolchain and protoc are installed, download crates (`cargo fetch`, no compiling), then sbatch.
# The last line it prints on success is JOBID=<n>.
set -euo pipefail
RUN=${1:?usage: prepare.sh RUN_DIR}
# shellcheck source=/dev/null
. "$RUN/meta.env"
ROOT=$(cd "$RUN/../.." && pwd)
LOCK="$ROOT/run.lock"
SRC="$ROOT/src"
log() { echo "prepare: $*"; }
[ "$(basename "$(dirname "$RUN")")" = runs ] || { echo "prepare: $RUN is not a run folder" >&2; exit 4; }

# On any failure before sbatch: drop this run's upload (it counts against the home quota)
# and release the lock if it is ours. hpc-submit keeps the log on the Mac.
submitted=0
release() { if [ "$(cat "$LOCK/owner" 2>/dev/null)" = "$RUNID" ]; then rm -rf "$LOCK"; fi; }
cleanup() {
    [ "$submitted" = 0 ] || return 0
    # A hangup right after sbatch: the job exists, so keep what it needs.
    if [ -n "$(squeue -h -u "$USER" -n "konnect-$RUNID" -o %i 2>/dev/null)" ]; then return 0; fi
    rm -rf "$RUN"
    release
}
trap cleanup EXIT

# 1. Quota: 1 job per user, running or queued.
jobs=$(squeue -h -u "$USER" -o "%i %j %T")
if [ -n "$jobs" ]; then
    echo "prepare: the 1-job quota is in use; not submitting:" >&2
    echo "$jobs" >&2
    exit 3
fi

# 2. Run lock (mkdir is atomic). It guards src/ and target/ from a second run.
#    A lock is stale when its job has left the queue, when its preparing process is gone
#    (same login node), or when its heartbeat is over an hour old.
if [ -d "$LOCK" ]; then
    owner=$(cat "$LOCK/owner" 2>/dev/null || true)
    jobid=$(cat "$LOCK/jobid" 2>/dev/null || true)
    pid='' host=''
    [ -f "$LOCK/pid" ] && read -r pid host <"$LOCK/pid"
    beat=$(($(date +%s) - $(stat -c %Y "$LOCK/owner" 2>/dev/null || stat -c %Y "$LOCK")))
    if [ -n "$jobid" ]; then
        if [ -n "$(squeue -h -j "$jobid" 2>/dev/null)" ]; then
            echo "prepare: run $owner (job $jobid) still holds the lock" >&2; exit 3
        fi
    elif [ "$host" = "$(hostname)" ] && [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
        echo "prepare: run $owner is being prepared (pid $pid)" >&2; exit 3
    elif [ "$host" != "$(hostname)" ] && [ "$beat" -lt 3600 ]; then
        echo "prepare: run $owner is being prepared on ${host:-another node} (heartbeat ${beat}s ago)" >&2; exit 3
    fi
    # Break it atomically: only one process can rename it away.
    # Break it atomically: only one process can rename it away. If the lock renamed is not
    # the one judged stale (another preparer broke it and took a new one meanwhile), put it back.
    broken="$LOCK.stale.$$"
    mv "$LOCK" "$broken" 2>/dev/null || { echo "prepare: lost the race for a stale lock" >&2; exit 3; }
    if [ "$(cat "$broken/owner" 2>/dev/null)" != "$owner" ] || [ "$(cat "$broken/jobid" 2>/dev/null || true)" != "$jobid" ]; then
        mv "$broken" "$LOCK" 2>/dev/null || true
        echo "prepare: another run took the lock meanwhile" >&2; exit 3
    fi
    log "removed stale lock of run $owner (job ${jobid:-none}, heartbeat ${beat}s ago)"
    rm -rf "$broken"
fi
mkdir "$LOCK" || { echo "prepare: lost the race for the run lock" >&2; exit 3; }
echo "$RUNID" >"$LOCK/owner"
echo "$$ $(hostname)" >"$LOCK/pid"
heartbeat() { touch "$LOCK/owner"; }

# 3. The exact commit, in a fixed checkout path so the cargo cache stays valid.
[ -d "$ROOT/repo.git" ] || git init -q --bare "$ROOT/repo.git"
git -C "$ROOT/repo.git" fetch -q "$RUN/commit.bundle" "+refs/heads/run:refs/runs/$RUNID"
got=$(git -C "$ROOT/repo.git" rev-parse "refs/runs/$RUNID^{commit}")
[ "$got" = "$SHA" ] || { echo "prepare: bundle holds $got, expected $SHA" >&2; exit 4; }
rm -f "$RUN/commit.bundle"
heartbeat
if [ ! -e "$SRC/.git" ]; then
    git -C "$ROOT/repo.git" worktree add -q --detach "$SRC" "$SHA"
else
    git -C "$SRC" checkout -q --detach --force "$SHA"
    git -C "$SRC" clean -q -fdx
fi
[ "$(git -C "$SRC" rev-parse HEAD)" = "$SHA" ] || { echo "prepare: checkout is not $SHA" >&2; exit 4; }
log "checked out $SHA"

# 4. Toolchain pinned by rust-toolchain.toml; install it once if missing (download only).
# shellcheck source=/dev/null
. "$HOME/.cargo/env"
want=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$SRC/rust-toolchain.toml")
[ -n "$want" ] || { echo "prepare: no channel in rust-toolchain.toml" >&2; exit 4; }
if ! rustup toolchain list | grep -q "^$want"; then
    log "installing Rust $want (first run only)"
    heartbeat
    rustup toolchain install "$want" --profile minimal --component clippy,rustfmt
fi
(cd "$SRC" && rustc -V | grep -q "^rustc $want ") ||
    { echo "prepare: rustup does not select $want in $SRC" >&2; exit 4; }
log "toolchain $(cd "$SRC" && rustc -V)"

# 4b. protoc pinned by meta.env; install it once if missing or another version (download
#     only). The zip is used only if its SHA-256 matches the pinned digest.
PROTOC_DIR="$ROOT/tools/protoc"
want_protoc="libprotoc $PROTOC_VERSION"
if [ "$("$PROTOC_DIR/bin/protoc" --version 2>/dev/null || true)" != "$want_protoc" ]; then
    log "installing protoc $PROTOC_VERSION (first run only)"
    heartbeat
    dl="$RUN/protoc-download" # inside the run folder: a failure's cleanup removes it
    mkdir -p "$dl"
    curl -fsSL --retry 2 -o "$dl/protoc.zip" \
        "https://github.com/protocolbuffers/protobuf/releases/download/v$PROTOC_VERSION/protoc-$PROTOC_VERSION-linux-x86_64.zip"
    got_sha=$(sha256sum "$dl/protoc.zip" | awk '{ print $1 }')
    [ "$got_sha" = "$PROTOC_SHA256" ] ||
        { echo "prepare: protoc zip digest is $got_sha, expected $PROTOC_SHA256; not installed" >&2; exit 4; }
    unzip -q "$dl/protoc.zip" -d "$dl/protoc"
    [ "$("$dl/protoc/bin/protoc" --version 2>/dev/null || true)" = "$want_protoc" ] ||
        { echo "prepare: the downloaded protoc does not report $want_protoc" >&2; exit 4; }
    mkdir -p "$ROOT/tools"
    rm -rf "$PROTOC_DIR.old"
    if [ -e "$PROTOC_DIR" ]; then mv "$PROTOC_DIR" "$PROTOC_DIR.old"; fi
    mv "$dl/protoc" "$PROTOC_DIR"
    rm -rf "$dl" "$PROTOC_DIR.old"
fi
log "protoc $PROTOC_VERSION"

# 5. Crates for Cargo.lock, so the job builds offline.
heartbeat
(cd "$SRC" && cargo fetch --locked -q)
log "crates fetched"

# 6. Submit. The job owns the lock from here and releases it when it ends.
jobid=$(sbatch --parsable \
    --job-name="konnect-$RUNID" \
    --partition="$PARTITION" --account="$ACCOUNT" --qos="$QOS" \
    --nodes=1 --ntasks=1 --cpus-per-task="$CPUS" --mem="$MEM" --time="$TIME" \
    --output="$RUN/slurm-%j.out" \
    "$RUN/bin/job.sbatch" "$RUN")
jobid=${jobid%%;*}
submitted=1
echo "$jobid" >"$RUN/jobid"
if [ "$(cat "$LOCK/owner" 2>/dev/null)" = "$RUNID" ]; then echo "$jobid" >"$LOCK/jobid"; fi
echo "JOBID=$jobid"
