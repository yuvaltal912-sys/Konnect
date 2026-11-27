# Shared settings for the Mac-side HPC scripts (hpc-submit, hpc-status, hpc-fetch).
# Sourced, not run. Bash 3.2 compatible (macOS /bin/bash). See docs/HPC_OFFLOAD.md.
#
# Every value can be overridden from the environment.

# ssh alias from ~/.ssh/config (docs/HPC_OFFLOAD.md, "One-time sign-in setup").
HPC_HOST=${HPC_HOST:-bgu-hpc}
# BatchMode: the scripts never prompt for a password or code. Without a key or an
# open ControlMaster connection they stop and say how to sign in.
HPC_SSH=${HPC_SSH:-ssh -o BatchMode=yes -o ConnectTimeout=20}
# Work folder on the cluster (under the cluster home from the owner's notes).
HPC_REMOTE_ROOT=${HPC_REMOTE_ROOT:-/truenas/home/yuvalaj/konnect-hpc}

# Slurm (owner's notes, 2026-11-27): partition course, 1 job per user, 4 h, one node of 80 CPUs.
HPC_PARTITION=${HPC_PARTITION:-course}
HPC_ACCOUNT=${HPC_ACCOUNT:-course_students}
HPC_QOS=${HPC_QOS:-course_interactive}
HPC_SIF=${HPC_SIF:-/truenas/sif_images/pytorch_cuda12.1_ngc.sif}
HPC_MAX_CPUS=80
HPC_MAX_MINUTES=240

hpc_die() { echo "hpc: $*" >&2; exit 1; }

# Records go to the MAIN checkout's target/hpc-runs (git-ignored, outlives a worktree).
hpc_local_runs() {
    if [ -n "${HPC_LOCAL_RUNS:-}" ]; then echo "$HPC_LOCAL_RUNS"; return; fi
    local common
    common=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) ||
        hpc_die "run this inside the Konnect repository"
    echo "$(dirname "$common")/target/hpc-runs"
}

# hpc_remote CMD...: run one command line on the cluster's login node.
hpc_remote() {
    # shellcheck disable=SC2086  # HPC_SSH is a command plus options, split on purpose.
    $HPC_SSH "$HPC_HOST" "$@"
}

hpc_check_connection() {
    hpc_remote true 2>/dev/null && return 0
    cat >&2 <<EOF
hpc: cannot reach '$HPC_HOST' without a password prompt.
     Connect the BGU VPN, then sign in once yourself (docs/HPC_OFFLOAD.md):
         ssh -fN $HPC_HOST
     The scripts never type a password or a one-time code.
EOF
    exit 1
}

# Prints the user's jobs on the cluster (running or queued), one per line; empty if none.
hpc_user_jobs() {
    hpc_remote 'squeue -h -u "$USER" -o "%i %j %T %M %R"'
}

# The run submitted last (newest job id file); dry runs have none and are skipped.
hpc_latest_run() {
    local runs newest
    runs=$(hpc_local_runs)
    # shellcheck disable=SC2012  # run ids are [0-9a-z-] only.
    newest=$(ls -1t "$runs"/*/jobid 2>/dev/null | head -n 1)
    [ -n "$newest" ] || hpc_die "no submitted run recorded in $runs"
    basename "$(dirname "$newest")"
}

hpc_valid_run_id() {
    case "$1" in
        *[!0-9a-z-]* | '') hpc_die "not a run id: '$1'" ;;
    esac
}
