#!/bin/bash
# Slurm batch script, submitted by prepare.sh with all resource flags on the command line.
# Runs the plan inside the cluster's Apptainer image and releases the run lock at the end.
set -u
RUN=${1:?usage: job.sbatch RUN_DIR}
# shellcheck source=/dev/null
. "$RUN/meta.env"
ROOT=$(cd "$RUN/../.." && pwd)
LOCK="$ROOT/run.lock"
release() { [ "$(cat "$LOCK/owner" 2>/dev/null)" = "$RUNID" ] && rm -rf "$LOCK"; }
trap release EXIT

echo "job ${SLURM_JOB_ID:-?} run $RUNID on $(hostname) at $(date '+%F %T %z'), ${SLURM_CPUS_PER_TASK:-?} CPUs"
"${APPTAINER:-/usr/bin/apptainer}" exec "$SIF" bash "$RUN/bin/run-plan.sh" "$RUN"
status=$?
echo "$status" >"$RUN/exit_code"
echo "job ended at $(date '+%F %T %z') with exit $status"
exit "$status"
