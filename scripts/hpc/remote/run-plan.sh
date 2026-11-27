#!/bin/bash
# Runs inside the container on the compute node. Executes plan.txt step by step in the
# fixed checkout, offline, with the pinned toolchain, and writes results.tsv as it goes.
#
# Verdicts (docs/HPC_OFFLOAD.md, "Plans"): ok, FAILED, NO-TESTS, MISSED, INVALID, NO-BASELINE,
# WRONG-TEST, PATCH-FAILED, recorded, recorded(exit N); +DIRTY when a step left changes.
set -uo pipefail
RUN=${1:?usage: run-plan.sh RUN_DIR}
# shellcheck source=/dev/null
. "$RUN/meta.env"
ROOT=$(cd "$RUN/../.." && pwd)
SRC="$ROOT/src"
OUT="$RUN/out"
mkdir -p "$OUT/steps" "$RUN/scratch/kicad-config" "$RUN/scratch/state"

# shellcheck source=/dev/null
. "$HOME/.cargo/env"
export CARGO_TARGET_DIR="$ROOT/target"
export CARGO_NET_OFFLINE=true RUSTUP_AUTO_INSTALL=0 CARGO_TERM_COLOR=never
export CARGO_BUILD_JOBS="${SLURM_CPUS_PER_TASK:-$CPUS}"
# Same isolation the Mac gate uses: never a real KiCad settings or state folder.
export KICAD_CONFIG_HOME="$RUN/scratch/kicad-config" KONNECT_STATE_DIR="$RUN/scratch/state"
export TMPDIR="${TMPDIR:-/tmp}/konnect-$RUNID"
export RUN_OUT="$OUT" SHA RUNID
mkdir -p "$TMPDIR"
if [ -x "$ROOT/tools/protoc/bin/protoc" ]; then
    export PROTOC="$ROOT/tools/protoc/bin/protoc" PROTOC_INCLUDE="$ROOT/tools/protoc/include"
fi

cd "$SRC" || exit 4
head=$(git rev-parse HEAD)
want=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
{
    echo "commit $head"
    echo "expected $SHA"
    echo "host $(hostname)"
    echo "slurm_job ${SLURM_JOB_ID:-} cpus ${SLURM_CPUS_PER_TASK:-} mem_per_node ${SLURM_MEM_PER_NODE:-}"
    echo "nproc $(nproc)"
    echo "kernel $(uname -srm)"
    echo "rustc $(rustc -V 2>&1)"
    echo "cargo $(cargo -V 2>&1)"
    echo "cc $(cc --version 2>&1 | head -n 1)"
    echo "cmake $(cmake --version 2>&1 | head -n 1)"
    echo "protoc $(${PROTOC:-protoc} --version 2>&1)"
    echo "cargo-mutants $(cargo mutants --version 2>&1 | head -n 1)"
} >"$OUT/env.txt"
[ "$head" = "$SHA" ] || { echo "checkout is $head, expected $SHA" >>"$OUT/env.txt"; exit 4; }
rustc -V | grep -q "^rustc $want " || { echo "toolchain is not $want" >>"$OUT/env.txt"; exit 4; }

# `cargo mutants` never sees the shared CARGO_TARGET_DIR. It copies the tree once per job, and
# with --jobs above 1 every copy built into the one shared target and every mutant read
# unviable (job 20899: 81 of 81); without it each copy builds in its own target (job 20920).
# Ordinary steps keep the shared target, the warm cache. Wrappers early on PATH unset it for a
# `cargo [+toolchain] [options] mutants ...` or `cargo-mutants ...` process, whatever the step's
# command looks like, and say so in the step's log.
SHIM="$RUN/scratch/bin"
mkdir -p "$SHIM"
real_cargo=$(command -v cargo)
real_mutants=$(command -v cargo-mutants || true)
cat >"$SHIM/cargo" <<EOF
#!/bin/bash
for a in "\$@"; do
    case "\$a" in
        +* | -*) ;;
        mutants) [ -n "\${CARGO_TARGET_DIR:-}" ] && echo "hpc: cargo mutants runs without the shared CARGO_TARGET_DIR" >&2
                 unset CARGO_TARGET_DIR; break ;;
        *) break ;;
    esac
done
exec $(printf %q "$real_cargo") "\$@"
EOF
if [ -n "$real_mutants" ]; then
    cat >"$SHIM/cargo-mutants" <<EOF
#!/bin/bash
[ -n "\${CARGO_TARGET_DIR:-}" ] && echo "hpc: cargo mutants runs without the shared CARGO_TARGET_DIR" >&2
unset CARGO_TARGET_DIR
exec $(printf %q "$real_mutants") "\$@"
EOF
fi
chmod +x "$SHIM"/*
export PATH="$SHIM:$PATH"

# Sum the libtest summaries ("test result: ok. 12 passed; 0 failed; 3 ignored; ...").
counts() {
    awk '/^test result:/ { r++; for (i = 2; i <= NF; i++) {
            if ($i ~ /^passed;?$/) p += $(i-1); if ($i ~ /^failed;?$/) f += $(i-1);
            if ($i ~ /^ignored;?$/) g += $(i-1) } }
         END { printf "%d\t%d\t%d\t%d", p, f, g, r }' "$1"
}
# Names of failing tests ("test some::path ... FAILED"), comma-separated.
failing_tests() {
    sed -n 's/^test \(.*\) \.\.\. FAILED$/\1/p' "$1" | sort -u | paste -sd, -
}

printf 'n\tname\texpect\texit\tseconds\tpassed\tfailed\tignored\tverdict\tfailing\n' >"$OUT/results.tsv"
: >"$OUT/.ok-pass-commands"
overall=0
n=0
exec 3<"$RUN/plan.txt"
while IFS= read -r line <&3 || [ -n "$line" ]; do
    read -r expect name cmd <<<"$line"
    n=$((n + 1))
    log=$(printf '%s/steps/%02d-%s.log' "$OUT" "$n" "$name")
    patch='' want_test=''
    case "$expect" in
        fail:*)
            spec=${expect#fail:}
            case "$spec" in *@*) want_test=${spec#*@}; spec=${spec%%@*} ;; esac
            patch="$RUN/$spec" expect=fail ;;
    esac
    echo "step $n $name ($expect) at $(date '+%T'): $cmd"
    touch "$OUT/heartbeat"
    start=$(date +%s)
    if [ -n "$patch" ] && ! git apply "$patch" >"$log" 2>&1; then
        status=99 verdict=PATCH-FAILED
    else
        echo "\$ $cmd" >>"$log"
        bash -c "$cmd" </dev/null >>"$log" 2>&1
        status=$?
        verdict=''
    fi
    secs=$(($(date +%s) - start))
    IFS=$'\t' read -r passed failed ignored summaries <<<"$(counts "$log")"
    failing=$(failing_tests "$log")
    if [ -z "$verdict" ]; then
        case "$expect" in
            pass)
                if [ "$status" -ne 0 ]; then verdict=FAILED
                elif [ "$summaries" -gt 0 ] && [ "$passed" -eq 0 ]; then verdict=NO-TESTS # filter matched nothing
                else verdict=ok; fi ;;
            any) [ "$status" -eq 0 ] && verdict=recorded || verdict="recorded(exit $status)" ;;
            fail)
                if [ "$status" -eq 0 ]; then verdict=MISSED
                elif [ "$failed" -eq 0 ]; then verdict=INVALID # non-zero exit, no failing test: a build error proves nothing
                elif ! grep -Fxq -- "$cmd" "$OUT/.ok-pass-commands"; then verdict=NO-BASELINE
                elif [ -n "$want_test" ] && ! echo ",$failing," | grep -Fq -- "$want_test"; then verdict=WRONG-TEST
                else verdict=ok; fi ;;
        esac
    fi
    [ "$expect" = pass ] && [ "$verdict" = ok ] && printf '%s\n' "$cmd" >>"$OUT/.ok-pass-commands"
    # What the step left behind, apart from its own control patch.
    left=$(git status --porcelain --untracked-files=all | cut -c4-)
    if [ -n "$patch" ] && [ -n "$left" ]; then
        left=$(printf '%s\n' "$left" | grep -Fxv -f <(git apply --numstat "$patch" 2>/dev/null | cut -f3) || true)
    fi
    moved=0
    [ "$(git rev-parse HEAD)" = "$SHA" ] || moved=1
    # Restore the exact commit after every step.
    git checkout -q --force --detach "$SHA" && git checkout -q --force -- . && git clean -q -fd
    if [ "$moved" = 1 ] || [ -n "$left" ] || [ -n "$(git status --porcelain)" ] || [ "$(git rev-parse HEAD)" != "$SHA" ]; then
        verdict="$verdict+DIRTY"
        { echo "left behind:"; printf '%s\n' "$left"; [ "$moved" = 1 ] && echo "HEAD moved"; } >>"$log"
    fi
    case "$verdict" in ok | recorded | 'recorded(exit '*')') ;; *) overall=1 ;; esac
    printf '%d\t%s\t%s\t%d\t%d\t%d\t%d\t%d\t%s\t%s\n' "$n" "$name" "$expect" "$status" "$secs" \
        "$passed" "$failed" "$ignored" "$verdict" "${failing:--}" >>"$OUT/results.tsv"
    echo "step $n $name: exit $status, ${secs}s, $passed passed, $failed failed: $verdict"
done
exec 3<&-
echo "$overall" >"$OUT/overall"
exit "$overall"
